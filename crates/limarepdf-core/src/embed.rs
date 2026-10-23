//! Fonts embedded in PDFs, for text the 12 standard fonts can't write (the Unicode text
//! design, section 4): Polish, Czech, Vietnamese, Greek, Russian and the rest.
//!
//! A face is written once per document as a Type 0 font with a CIDFontType2 descendant and
//! `Identity-H` encoding: each character is its glyph's number in the embedded font, two
//! bytes. `/W` gives the widths, so every reader spaces the text alike, and a ToUnicode map
//! the characters, so search, copy and LimarePDF's own text reading get them back. The font
//! file is the face cut down to the glyphs used by the `subsetter` crate, which numbers the
//! glyphs it keeps in the order it is given them.
//!
//! When more text is set in the face, the same font object is rewritten with the old glyphs
//! given first, so they keep their numbers and text written before stays valid. What that
//! needs is kept in the font dictionary under `/LimarePDF` (readers ignore keys they don't
//! know): the face, the glyphs in order with the character each was written for, and a
//! checksum of the font file. A font object whose font file has changed since (another app
//! rewrote it), or that was made from another version of the face, isn't grown: a new one
//! is started.

use crate::fonts::{winansi_byte, winansi_bytes};
use crate::{FontChoice, Pdf, Result};
use anyhow::{anyhow, bail};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};
use sha2::{Digest, Sha256};
use skrifa::prelude::{FontRef, LocationRef, MetadataProvider, Size};
use skrifa::GlyphId;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::OnceLock;
use subsetter::GlyphRemapper;
use unicode_normalization::UnicodeNormalization;

/// The bundled Liberation faces (see `assets/fonts/README.md`): sans, serif and mono, each
/// regular, bold, italic and bold italic, by name. They have the widths of Helvetica, Times
/// and Courier. A static, so the app, which draws typed text in them, uses this one copy.
pub static LIBERATION: [[(&str, &[u8]); 4]; 3] = [
    [
        (
            "Liberation Sans",
            include_bytes!("../assets/fonts/LiberationSans-Regular.ttf"),
        ),
        (
            "Liberation Sans Bold",
            include_bytes!("../assets/fonts/LiberationSans-Bold.ttf"),
        ),
        (
            "Liberation Sans Italic",
            include_bytes!("../assets/fonts/LiberationSans-Italic.ttf"),
        ),
        (
            "Liberation Sans Bold Italic",
            include_bytes!("../assets/fonts/LiberationSans-BoldItalic.ttf"),
        ),
    ],
    [
        (
            "Liberation Serif",
            include_bytes!("../assets/fonts/LiberationSerif-Regular.ttf"),
        ),
        (
            "Liberation Serif Bold",
            include_bytes!("../assets/fonts/LiberationSerif-Bold.ttf"),
        ),
        (
            "Liberation Serif Italic",
            include_bytes!("../assets/fonts/LiberationSerif-Italic.ttf"),
        ),
        (
            "Liberation Serif Bold Italic",
            include_bytes!("../assets/fonts/LiberationSerif-BoldItalic.ttf"),
        ),
    ],
    [
        (
            "Liberation Mono",
            include_bytes!("../assets/fonts/LiberationMono-Regular.ttf"),
        ),
        (
            "Liberation Mono Bold",
            include_bytes!("../assets/fonts/LiberationMono-Bold.ttf"),
        ),
        (
            "Liberation Mono Italic",
            include_bytes!("../assets/fonts/LiberationMono-Italic.ttf"),
        ),
        (
            "Liberation Mono Bold Italic",
            include_bytes!("../assets/fonts/LiberationMono-BoldItalic.ttf"),
        ),
    ],
];

/// The bundled face for a standard font, by name and file: Helvetica is Liberation Sans,
/// and so on.
pub fn liberation(f: FontChoice) -> (&'static str, &'static [u8]) {
    let (family, style) = place(f);
    LIBERATION[family][style]
}

/// Where the face of `f` is in [`LIBERATION`]: its family, and its style in that.
fn place(f: FontChoice) -> (usize, usize) {
    let family = match f.family {
        crate::Family::Helvetica => 0,
        crate::Family::Times => 1,
        crate::Family::Courier => 2,
    };
    (family, f.bold as usize + 2 * f.italic as usize)
}

/// Key of LimarePDF's own entry in the fonts it writes.
const KEY: &[u8] = b"LimarePDF";
/// Most glyphs one embedded font holds: a glyph number is two bytes, and the subsetter's
/// numbering stops short of 65,536.
const MAX_GLYPHS: usize = 0xFFF0;

/// A face that can be embedded.
pub(crate) struct Face {
    /// Its PostScript name ("LiberationSans-Bold"), as PDF names it.
    ps: String,
    data: &'static [u8],
    /// The first bytes of the face file's checksum, so a font object made from another
    /// version of the face is told apart.
    source: [u8; 8],
    /// Each glyph's width, in thousandths of an em, rounded as `/W` gives it.
    widths: Vec<i64>,
}

/// Largest embedded font file read back to grow it, once decompressed.
const MAX_FILE: usize = 32 << 20;

/// The bundled faces, each read the first time it is needed.
static FACES: [OnceLock<Face>; 12] = [const { OnceLock::new() }; 12];

impl Face {
    /// The bundled face of the standard font `f`.
    pub(crate) fn liberation(f: FontChoice) -> &'static Face {
        let (family, style) = place(f);
        FACES[4 * family + style]
            .get_or_init(|| Face::read(LIBERATION[family][style].1).expect("a bundled face reads"))
    }

    fn read(data: &'static [u8]) -> Result<Self> {
        let font = FontRef::new(data).map_err(|e| anyhow!("can't read the font: {e}"))?;
        let ps = font
            .localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME)
            .english_or_first()
            .map(|s| s.to_string())
            .unwrap_or_default();
        // A PDF name of the usual kind: letters, digits and hyphens.
        let ps: String = ps
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if ps.is_empty() {
            bail!("the font has no PostScript name");
        }
        let m = font.metrics(Size::unscaled(), LocationRef::default());
        let em = 1000.0 / m.units_per_em.max(1) as f32;
        let gm = font.glyph_metrics(Size::unscaled(), LocationRef::default());
        let widths = (0..m.glyph_count as u32)
            .map(|g| (gm.advance_width(GlyphId::new(g)).unwrap_or(0.0) * em).round() as i64)
            .collect();
        let mut source = [0; 8];
        source.copy_from_slice(&Sha256::digest(data)[..8]);
        Ok(Self {
            ps,
            data,
            source,
            widths,
        })
    }

    fn font(&self) -> FontRef<'static> {
        FontRef::new(self.data).expect("read when the face was")
    }

    /// The face's glyph for `c`; `None` for characters it lacks, control characters and
    /// those of right-to-left scripts.
    fn glyph(&self, c: char) -> Option<u16> {
        if c.is_control() || right_to_left(c) {
            return None;
        }
        let g = self.font().charmap().map(c)?.to_u32();
        (g != 0).then_some(g as u16)
    }

    /// Whether the face has a glyph for `c` that LimarePDF can write (control characters
    /// and right-to-left scripts never).
    pub(crate) fn covers(&self, c: char) -> bool {
        self.glyph(c).is_some()
    }

    /// The characters `text` is written with in the face: put together (NFC) where the face
    /// has the combined letter, else apart where it has the parts; and those it can't write,
    /// each once.
    fn spelled(&self, text: &str) -> (Vec<char>, String) {
        let (mut out, mut missing) = (Vec::new(), String::new());
        for c in text.nfc() {
            if self.covers(c) {
                out.push(c);
                continue;
            }
            let parts: Vec<char> = c.to_string().nfd().collect();
            if parts.len() > 1 && parts.iter().all(|&p| self.covers(p)) {
                out.extend(parts);
            } else if !missing.contains(c) {
                missing.push(c);
            }
        }
        (out, missing)
    }

    /// The characters `text` is written with in the face ([`Face::spelled`]), or those it
    /// can't write, when there are any.
    fn spell(&self, text: &str) -> std::result::Result<Vec<char>, String> {
        match self.spelled(text) {
            (out, missing) if missing.is_empty() => Ok(out),
            (_, missing) => Err(missing),
        }
    }

    /// The width of `text` set in the face at `size`, in points, as readers space it (from
    /// the widths `/W` gives). Characters the face can't write count as nothing.
    pub(crate) fn width(&self, text: &str, size: f32) -> f32 {
        let units: i64 = self
            .spelled(text)
            .0
            .into_iter()
            .filter_map(|c| self.glyph(c))
            .map(|g| self.widths[g as usize])
            .sum();
        units as f32 * size / 1000.0
    }
}

/// Whether LimarePDF can write `c` in the standard font `f`: in the font itself, or in its
/// embedded face. Control characters never.
pub fn writable(f: FontChoice, c: char) -> bool {
    winansi_byte(c).is_some() || Face::liberation(f).covers(c)
}

/// The message refusing text with the characters `missing`.
pub(crate) fn cant_write(missing: &str) -> String {
    format!(
        "LimarePDF can't write \"{missing}\" yet; Latin, Greek and Cyrillic characters are \
         supported"
    )
}

/// The characters of `text` LimarePDF can't write in the standard font `f` or its embedded
/// face, each once (as [`Pdf::pen`] writes it; control characters included).
pub(crate) fn unwritable(f: FontChoice, text: &str) -> String {
    if text.chars().all(|c| winansi_byte(c).is_some()) {
        return String::new();
    }
    Face::liberation(f).spelled(text).1
}

/// Text set in an embedded font: the font object to name in the resources, and the bytes
/// to show with `Tj`.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct Set {
    pub font: ObjectId,
    pub bytes: Vec<u8>,
}

/// An embedded font grown for some text: the font object, and the code of each glyph it
/// holds, to write that text or any part of it.
#[derive(Clone)]
pub(crate) struct Codes {
    pub font: ObjectId,
    face: &'static Face,
    /// The code of each glyph of the face the font holds.
    cids: HashMap<u16, u16>,
}

impl Codes {
    /// The bytes that show `text`, two for each character. Characters the font doesn't
    /// hold are left out.
    pub(crate) fn bytes(&self, text: &str) -> Vec<u8> {
        self.face
            .spelled(text)
            .0
            .into_iter()
            .filter_map(|c| self.cids.get(&self.face.glyph(c)?))
            .flat_map(|n| n.to_be_bytes())
            .collect()
    }

    /// The width of `text` at `size`, in points.
    pub(crate) fn width(&self, text: &str, size: f32) -> f32 {
        self.face.width(text, size)
    }
}

/// The font a writer sets text in: the standard font, with nothing embedded, while the
/// text fits WinAnsi, else its embedded face (the Unicode text design, question 2).
#[derive(Clone)]
pub(crate) enum Pen {
    Standard(FontChoice),
    Embedded(Codes),
}

impl Pen {
    /// The font to name in the resources: a standard font's dictionary, or a reference to
    /// the embedded font.
    pub(crate) fn font(&self) -> Object {
        match self {
            Pen::Standard(f) => dictionary! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => f.base_font(),
                "Encoding" => "WinAnsiEncoding",
            }
            .into(),
            Pen::Embedded(c) => Object::Reference(c.font),
        }
    }

    /// The bytes that show `text` (part of the text the pen was made for).
    pub(crate) fn bytes(&self, text: &str) -> Vec<u8> {
        match self {
            Pen::Standard(_) => winansi_bytes(text),
            Pen::Embedded(c) => c.bytes(text),
        }
    }

    /// `text` as a string object to show with `Tj`.
    pub(crate) fn string(&self, text: &str) -> Object {
        let format = match self {
            Pen::Standard(_) => lopdf::StringFormat::Literal,
            Pen::Embedded(_) => lopdf::StringFormat::Hexadecimal,
        };
        Object::String(self.bytes(text), format)
    }

    /// `text` as a string written in a content stream: `(…)` in a standard font, `<…>` in
    /// an embedded one.
    pub(crate) fn token(&self, text: &str) -> String {
        match self {
            Pen::Standard(_) => format!("({})", crate::overlay::escape(text)),
            Pen::Embedded(c) => {
                let hex: String = c.bytes(text).iter().map(|b| format!("{b:02X}")).collect();
                format!("<{hex}>")
            }
        }
    }

    /// The width of `text` at `size`, in points.
    pub(crate) fn width(&self, text: &str, size: f32) -> f32 {
        match self {
            Pen::Standard(f) => {
                let units: f32 = text
                    .chars()
                    .map(|c| crate::std14::width(f.base_font(), c).unwrap_or(0.0))
                    .sum();
                units * size / 1000.0
            }
            Pen::Embedded(c) => c.width(text, size),
        }
    }
}

/// What LimarePDF keeps in a font it wrote, to grow it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Record {
    /// The glyphs of the face it holds, in their order in the font file (glyph 1 first),
    /// each with the character it was written for.
    glyphs: Vec<(u16, char)>,
}

/// The objects of an embedded font other than its font dictionary: kept when it grows.
#[derive(Clone, Copy, Default)]
struct Parts {
    descendant: Option<ObjectId>,
    descriptor: Option<ObjectId>,
    file: Option<ObjectId>,
    to_unicode: Option<ObjectId>,
}

impl Pdf {
    /// `text` set in the embedded face of `f` ([`Pdf::embed_codes`]), and the bytes that
    /// show it.
    #[cfg(test)]
    pub(crate) fn embed_text(&mut self, f: FontChoice, text: &str) -> Result<Set> {
        let codes = self.embed_codes(f, text)?;
        Ok(Set {
            font: codes.font,
            bytes: codes.bytes(text),
        })
    }

    /// The pen to write `text`, and any part of it, in the standard font `f`: the font
    /// itself while every character fits WinAnsi, else its embedded face, grown to hold
    /// them ([`Pdf::embed_codes`]). Control characters don't count: line breaks, for one,
    /// as lines are shown one by one.
    pub(crate) fn pen(&mut self, f: FontChoice, text: &str) -> Result<Pen> {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if text.chars().all(|c| winansi_byte(c).is_some()) {
            return Ok(Pen::Standard(f));
        }
        let missing = unwritable(f, &text);
        if !missing.is_empty() {
            bail!("{}", cant_write(&missing));
        }
        Ok(Pen::Embedded(self.embed_codes(f, &text)?))
    }

    /// The font object a pen writes with: a new standard font object, or the embedded font.
    pub(crate) fn pen_font(&mut self, pen: &Pen) -> ObjectId {
        match pen {
            Pen::Standard(_) => self.doc.add_object(pen.font()),
            Pen::Embedded(c) => c.font,
        }
    }

    /// The embedded face of the standard font `f`: the face's font object in this
    /// document, grown to hold the glyphs `text` needs (made the first time), and the codes
    /// to write that text. Accents typed as separate marks are put together first (NFC)
    /// where the face has the combined letter, and a letter it has only in parts is written
    /// in them. Refused, with nothing changed, when the face lacks a character.
    pub(crate) fn embed_codes(&mut self, f: FontChoice, text: &str) -> Result<Codes> {
        let face = Face::liberation(f);
        let (font, cids) = self.embed_in(face, text)?;
        Ok(Codes { font, face, cids })
    }

    fn embed_in(&mut self, face: &Face, text: &str) -> Result<(ObjectId, HashMap<u16, u16>)> {
        let chars = face
            .spell(text)
            .map_err(|missing| anyhow!("{} has no characters for \"{missing}\"", face.ps))?;
        let found = self.embedded(face);
        let (mut record, at) = match found {
            Some((id, record, parts)) => (record, Some((id, parts))),
            None => (Record::default(), None),
        };
        let before = record.glyphs.len();
        let mut cid: HashMap<u16, u16> = record
            .glyphs
            .iter()
            .enumerate()
            .map(|(i, &(g, _))| (g, i as u16 + 1))
            .collect();
        for c in chars {
            let g = face.glyph(c).expect("checked above");
            if let std::collections::hash_map::Entry::Vacant(e) = cid.entry(g) {
                if record.glyphs.len() >= MAX_GLYPHS {
                    bail!("too many different characters in one font");
                }
                record.glyphs.push((g, c));
                e.insert(record.glyphs.len() as u16);
            }
        }
        let font = match at {
            Some((id, _)) if record.glyphs.len() == before => id,
            Some((id, parts)) => self.write_font(face, &record, Some(id), parts)?,
            None => self.write_font(face, &record, None, Parts::default())?,
        };
        self.made_fonts.insert(font);
        Ok((font, cid))
    }

    /// This document's font object for `face` that LimarePDF wrote and can grow, with what
    /// it holds and its other objects. In a signed document, only one made since it was
    /// opened: growing rewrites objects, and in one a signed revision holds, checking the
    /// signature would see a change.
    fn embedded(&self, face: &Face) -> Option<(ObjectId, Record, Parts)> {
        let signed = self.holds_signature();
        self.doc
            .objects
            .iter()
            .filter(|(id, _)| !signed || self.made_fonts.contains(id))
            .filter_map(|(&id, o)| Some((id, o.as_dict().ok()?)))
            .find_map(|(id, d)| {
                let (record, parts) = self.grows(face, d)?;
                Some((id, record, parts))
            })
    }

    /// Whether the document holds a signature dictionary (one with a `/ByteRange`), signed
    /// or being signed.
    fn holds_signature(&self) -> bool {
        self.doc
            .objects
            .values()
            .any(|o| o.as_dict().is_ok_and(|d| d.has(b"ByteRange")))
    }

    /// What the font dictionary `d` holds when it is one LimarePDF wrote for `face` and its
    /// font file is still the one it wrote.
    fn grows(&self, face: &Face, d: &Dictionary) -> Option<(Record, Parts)> {
        if d.get(b"Subtype").and_then(Object::as_name).ok()? != b"Type0" {
            return None;
        }
        let own = d.get(KEY).and_then(Object::as_dict).ok()?;
        let s = |k: &[u8]| own.get(k).and_then(Object::as_str).ok();
        if s(b"Face")? != face.ps.as_bytes() || s(b"Source")? != face.source {
            return None;
        }
        let list = own.get(b"Glyphs").and_then(Object::as_array).ok()?;
        if list.len() > 2 * MAX_GLYPHS {
            return None;
        }
        let mut glyphs = Vec::with_capacity(list.len() / 2);
        let mut seen = HashSet::new();
        for pair in list.chunks(2) {
            let [g, c] = pair else { return None };
            let g = u16::try_from(g.as_i64().ok()?).ok()?;
            let c = char::from_u32(u32::try_from(c.as_i64().ok()?).ok()?)?;
            // Each glyph once, for the character it was written for.
            if face.glyph(c) != Some(g) || !seen.insert(g) {
                return None;
            }
            glyphs.push((g, c));
        }
        let reference = |d: &Dictionary, k: &[u8]| d.get(k).and_then(Object::as_reference).ok();
        let descendants = d.get(b"DescendantFonts").and_then(Object::as_array).ok()?;
        let descendant = descendants.first()?.as_reference().ok()?;
        let cid_font = self.doc.get_dictionary(descendant).ok()?;
        let descriptor = reference(cid_font, b"FontDescriptor")?;
        let file = reference(self.doc.get_dictionary(descriptor).ok()?, b"FontFile2")?;
        let data = self.doc.get_object(file).ok()?.as_stream().ok()?;
        let data = data.get_plain_content_with_limit(MAX_FILE).ok()?;
        if Sha256::digest(&data)[..] != *s(b"Sum")? {
            return None;
        }
        let parts = Parts {
            descendant: Some(descendant),
            descriptor: Some(descriptor),
            file: Some(file),
            to_unicode: reference(d, b"ToUnicode"),
        };
        Some((Record { glyphs }, parts))
    }

    /// Writes the font object for `face` holding `record`'s glyphs: at `at` with its other
    /// objects in `parts` when it grows, else as new objects.
    fn write_font(
        &mut self,
        face: &Face,
        record: &Record,
        at: Option<ObjectId>,
        parts: Parts,
    ) -> Result<ObjectId> {
        let gids: Vec<u16> = record.glyphs.iter().map(|&(g, _)| g).collect();
        let mapper = GlyphRemapper::new_from_glyphs(&gids);
        // The subset's glyph numbers are the ones the text is written with.
        let numbered = gids
            .iter()
            .enumerate()
            .all(|(i, &g)| mapper.get(g) == Some(i as u16 + 1));
        if !numbered {
            bail!("the font's glyphs were numbered unexpectedly; nothing was changed");
        }
        let sub = subsetter::subset(face.data, 0, &mapper)
            .map_err(|e| anyhow!("can't cut down {}: {e}", face.ps))?;
        let sum = Sha256::digest(&sub);
        let base = format!("{}+{}", tag(&sum), face.ps);

        let font = face.font();
        let m = font.metrics(Size::unscaled(), LocationRef::default());
        let em = 1000.0 / m.units_per_em.max(1) as f32;
        let scaled = |v: f32| Object::Integer((v * em).round() as i64);
        let bbox = m
            .bounds
            .map_or([0.0; 4], |b| [b.x_min, b.y_min, b.x_max, b.y_max]);
        let weight = font.attributes().weight.value();
        let italic = m.italic_angle != 0.0;
        // Symbolic: the glyphs aren't only the standard Latin set.
        let flags = 4 | m.is_monospace as i64 | (64 * italic as i64) | (2 * serif(&face.ps) as i64);

        let mut file = Stream::new(dictionary! { "Length1" => sub.len() as i64 }, sub);
        file.allows_compression = true;
        let file = self.put(parts.file, file.into());
        let descriptor = self.put(
            parts.descriptor,
            dictionary! {
                "Type" => "FontDescriptor",
                "FontName" => Object::Name(base.clone().into_bytes()),
                "Flags" => flags,
                "FontBBox" => bbox.iter().map(|&v| scaled(v)).collect::<Vec<_>>(),
                "ItalicAngle" => m.italic_angle,
                "Ascent" => scaled(m.ascent),
                "Descent" => scaled(m.descent),
                "CapHeight" => scaled(m.cap_height.unwrap_or(m.ascent)),
                // Estimated from the weight, as PDF writers commonly do.
                "StemV" => (10.0 + 0.244 * (weight - 50.0)).round() as i64,
                "FontFile2" => file,
            }
            .into(),
        );
        let widths: Vec<Object> = std::iter::once(0)
            .chain(gids.iter().copied())
            .map(|g| Object::Integer(face.widths[g as usize]))
            .collect();
        let descendant = self.put(
            parts.descendant,
            dictionary! {
                "Type" => "Font",
                "Subtype" => "CIDFontType2",
                "BaseFont" => Object::Name(base.clone().into_bytes()),
                "CIDSystemInfo" => dictionary! {
                    "Registry" => Object::string_literal("Adobe"),
                    "Ordering" => Object::string_literal("Identity"),
                    "Supplement" => 0,
                },
                "FontDescriptor" => descriptor,
                "DW" => face.widths[0],
                "W" => vec![0.into(), widths.into()],
                "CIDToGIDMap" => "Identity",
            }
            .into(),
        );
        let to_unicode = self.put(
            parts.to_unicode,
            Stream::new(Dictionary::new(), to_unicode(&record.glyphs).into_bytes()).into(),
        );
        let list: Vec<Object> = record
            .glyphs
            .iter()
            .flat_map(|&(g, c)| [Object::Integer(g.into()), Object::Integer(c as i64)])
            .collect();
        let own = dictionary! {
            "Face" => Object::string_literal(face.ps.as_str()),
            "Source" => Object::String(face.source.to_vec(), lopdf::StringFormat::Hexadecimal),
            "Glyphs" => list,
            "Sum" => Object::String(sum.to_vec(), lopdf::StringFormat::Hexadecimal),
        };
        let mut dict = dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => Object::Name(base.into_bytes()),
            "Encoding" => "Identity-H",
            "DescendantFonts" => vec![descendant.into()],
            "ToUnicode" => to_unicode,
        };
        dict.set(KEY, own);
        Ok(self.put(at, dict.into()))
    }

    /// Puts `obj` at `at`, replacing what is there, or as a new object.
    fn put(&mut self, at: Option<ObjectId>, obj: Object) -> ObjectId {
        match at {
            Some(id) => {
                self.doc.objects.insert(id, obj);
                id
            }
            None => self.doc.add_object(obj),
        }
    }
}

/// Whether `c` is of a script written right to left (Hebrew, Arabic, Syriac, Thaana, N'Ko
/// and the rest): such lines must be reordered for display, which LimarePDF doesn't do yet
/// (the Unicode text design, section 6), so Liberation's Hebrew isn't offered.
fn right_to_left(c: char) -> bool {
    matches!(
        c,
        '\u{0590}'..='\u{08FF}'
            | '\u{FB1D}'..='\u{FDFF}'
            | '\u{FE70}'..='\u{FEFF}'
            | '\u{10800}'..='\u{10FFF}'
            | '\u{1E800}'..='\u{1EFFF}'
    )
}

/// Whether a face is a serif one, by its name (the Serif flag of its font descriptor).
fn serif(ps: &str) -> bool {
    ps.contains("Serif") && !ps.contains("Sans")
}

/// The six capital letters that start a subset font's name ("ABCDEF+"), from the font
/// file's checksum, so another subset gets another name.
fn tag(sum: &[u8]) -> String {
    sum.iter()
        .take(6)
        .map(|b| (b'A' + b % 26) as char)
        .collect()
}

/// A ToUnicode map from each glyph's number (glyph 1 first) to the character it was
/// written for.
fn to_unicode(glyphs: &[(u16, char)]) -> String {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    for (i, chunk) in glyphs.chunks(100).enumerate() {
        let _ = writeln!(s, "{} beginbfchar", chunk.len());
        for (j, &(_, c)) in chunk.iter().enumerate() {
            let _ = write!(s, "<{:04X}> <", 100 * i + j + 1);
            for u in c.encode_utf16(&mut [0; 2]) {
                let _ = write!(s, "{u:04X}");
            }
            s.push_str(">\n");
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::FontCtx;
    use crate::render::{self, Renderer};
    use crate::Family;
    use skrifa::outline::{DrawSettings, OutlinePen};

    fn choice(family: Family, bold: bool, italic: bool) -> FontChoice {
        FontChoice {
            family,
            bold,
            italic,
        }
    }

    const SANS: FontChoice = FontChoice {
        family: Family::Helvetica,
        bold: false,
        italic: false,
    };

    /// Every standard font, with its face's PostScript name.
    fn every_face() -> Vec<(FontChoice, String)> {
        let mut out = Vec::new();
        for (family, name) in [
            (Family::Helvetica, "Sans"),
            (Family::Times, "Serif"),
            (Family::Courier, "Mono"),
        ] {
            for (bold, italic, style) in [
                (false, false, ""),
                (true, false, "-Bold"),
                (false, true, "-Italic"),
                (true, true, "-BoldItalic"),
            ] {
                let ps = format!("Liberation{name}{style}");
                out.push((choice(family, bold, italic), ps));
            }
        }
        out
    }

    /// Words in the scripts of the first scope (section 6), each beyond WinAnsi.
    const WORDS: [&str; 8] = [
        "Zażółć gęślą jaźń",    // Polish
        "Příliš žluťoučký kůň", // Czech
        "Iğdır şehrinde ılık",  // Turkish
        "Știință și țară",      // Romanian
        "Tiếng Việt có dấu",    // Vietnamese
        "Ελληνικά γράμματα",    // Greek
        "Съешь же ещё этих",    // Russian
        "Їжак і ґанок",         // Ukrainian
    ];

    /// A blank page 400 by 300.
    fn blank() -> Pdf {
        let mut p = Pdf::new();
        p.insert_blank(0, 400.0, 300.0).unwrap();
        p
    }

    /// Shows each of `lines` on the first page in its font, 12 pt, one under the other from
    /// the top, as the page's only content.
    fn show(p: &mut Pdf, lines: &[(FontChoice, &str)]) {
        let page = p.page_ids()[0];
        let mut content = Vec::new();
        for (i, (f, text)) in lines.iter().enumerate() {
            let set = p.embed_text(*f, text).unwrap();
            let name = p
                .add_resource(page, "Font", "LimareEm", Object::Reference(set.font))
                .unwrap();
            let hex: String = set.bytes.iter().map(|b| format!("{b:02X}")).collect();
            let y = 270 - 24 * i;
            content.extend(format!("BT /{name} 12 Tf 20 {y} Td <{hex}> Tj ET\n").bytes());
        }
        let id = p.doc.add_object(Stream::new(Dictionary::new(), content));
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("Contents", Object::Reference(id));
    }

    /// The document saved and opened again.
    fn reopened(p: &mut Pdf) -> Pdf {
        Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap()
    }

    /// The font objects LimarePDF wrote in `p`.
    fn own_fonts(p: &Pdf) -> Vec<ObjectId> {
        let mut ids: Vec<ObjectId> = p
            .doc
            .objects
            .iter()
            .filter(|(_, o)| o.as_dict().is_ok_and(|d| d.has(KEY)))
            .map(|(&id, _)| id)
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn text_beyond_western_european_reads_back() {
        let mut p = blank();
        let faces = every_face();
        let lines: Vec<(FontChoice, &str)> = WORDS
            .iter()
            .enumerate()
            .map(|(i, w)| (faces[i % faces.len()].0, *w))
            .collect();
        show(&mut p, &lines);
        let back = reopened(&mut p);
        let text = back.page_text(0);
        for w in WORDS {
            assert!(text.contains(w), "{w:?} in {text:?}");
        }
    }

    #[test]
    fn each_face_is_named_and_described_as_itself() {
        for (f, ps) in every_face() {
            let mut p = blank();
            let set = p.embed_text(f, "Ωmega").unwrap();
            let font = p.doc.get_dictionary(set.font).unwrap();
            let base = font.get(b"BaseFont").unwrap().as_name().unwrap();
            let base = String::from_utf8(base.to_vec()).unwrap();
            let (tag, name) = base.split_once('+').unwrap();
            assert_eq!(name, ps);
            assert!(
                tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()),
                "{tag}"
            );
            assert_eq!(
                font.get(b"Encoding").unwrap().as_name().unwrap(),
                b"Identity-H"
            );

            let descendant = font.get(b"DescendantFonts").unwrap().as_array().unwrap()[0]
                .as_reference()
                .unwrap();
            let cid = p.doc.get_dictionary(descendant).unwrap();
            assert_eq!(
                cid.get(b"Subtype").unwrap().as_name().unwrap(),
                b"CIDFontType2"
            );
            let desc = cid.get(b"FontDescriptor").unwrap().as_reference().unwrap();
            let desc = p.doc.get_dictionary(desc).unwrap();
            let flags = desc.get(b"Flags").unwrap().as_i64().unwrap();
            assert_eq!(
                flags & 1 != 0,
                f.family == Family::Courier,
                "{ps} fixed pitch"
            );
            assert_eq!(flags & 2 != 0, f.family == Family::Times, "{ps} serif");
            assert_eq!(flags & 64 != 0, f.italic, "{ps} italic");
            let stem = desc.get(b"StemV").unwrap().as_i64().unwrap();
            assert!(
                if f.bold { stem > 150 } else { stem < 100 },
                "{ps} StemV {stem}"
            );
        }
    }

    #[test]
    fn the_font_grows_in_place_and_earlier_text_stays_valid() {
        let mut p = blank();
        let first = p.embed_text(SANS, "Zażółć").unwrap();
        let again = p.embed_text(SANS, "Zażółć").unwrap();
        assert_eq!((again.font, &again.bytes), (first.font, &first.bytes));

        let more = p.embed_text(SANS, "жёлтый Zebra").unwrap();
        assert_eq!(more.font, first.font, "the same font object grows");
        assert_eq!(own_fonts(&p), [first.font]);
        let third = p.embed_text(SANS, "Zażółć").unwrap();
        assert_eq!(
            third.bytes, first.bytes,
            "the first glyphs keep their numbers"
        );
        // "Z" was written first; "Zebra" uses its number.
        assert_eq!(more.bytes[14..16], first.bytes[..2]);

        // Other faces get fonts of their own.
        let bold = p
            .embed_text(choice(Family::Helvetica, true, false), "Ż")
            .unwrap();
        assert_ne!(bold.font, first.font);

        // And it still grows after the file is saved and opened again (once the page uses
        // the fonts: saving drops unused objects).
        let page = p.page_ids()[0];
        for font in [first.font, bold.font] {
            p.add_resource(page, "Font", "LimareEm", font.into())
                .unwrap();
        }
        let mut back = reopened(&mut p);
        let fonts = own_fonts(&back);
        assert_eq!(fonts.len(), 2);
        let grown = back.embed_text(SANS, "Zażółć ΑΒΓ").unwrap();
        assert!(fonts.contains(&grown.font));
        assert_eq!(grown.bytes[..first.bytes.len()], first.bytes[..]);
        assert_eq!(own_fonts(&back), fonts, "no new font objects");
    }

    #[test]
    fn earlier_text_draws_the_same_after_the_font_grows() {
        let mut p = blank();
        show(
            &mut p,
            &[(SANS, "Příliš žluťoučký kůň"), (SANS, "Ελληνικά")],
        );
        let draw = |p: &mut Pdf| {
            let h = render::load(p.to_bytes().unwrap(), None).unwrap();
            let r = Renderer::new(&h);
            let b = r.render(0, 2.0).unwrap();
            assert!(r.take_missing().is_empty());
            b
        };
        let before = draw(&mut p);
        let inked = before.rgba.chunks(4).filter(|px| px[0] < 128).count();
        assert!(inked > 500, "the text is drawn ({inked} dark pixels)");
        // Many more glyphs, none of them shown.
        p.embed_text(SANS, "Съешь же ещё этих мягких французских булок")
            .unwrap();
        assert!(before.rgba == draw(&mut p).rgba);
    }

    /// Records an outline's drawing commands.
    #[derive(Default, PartialEq, Debug)]
    struct Pen(Vec<(char, [f32; 6])>);

    impl OutlinePen for Pen {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.push(('M', [x, y, 0.0, 0.0, 0.0, 0.0]));
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.0.push(('L', [x, y, 0.0, 0.0, 0.0, 0.0]));
        }
        fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
            self.0.push(('Q', [cx0, cy0, x, y, 0.0, 0.0]));
        }
        fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
            self.0.push(('C', [cx0, cy0, cx1, cy1, x, y]));
        }
        fn close(&mut self) {
            self.0.push(('Z', [0.0; 6]));
        }
    }

    fn outline(font: &FontRef, g: u32) -> Pen {
        let mut pen = Pen::default();
        let glyph = font.outline_glyphs().get(GlyphId::new(g)).unwrap();
        let settings = DrawSettings::unhinted(Size::unscaled(), LocationRef::default());
        glyph.draw(settings, &mut pen).unwrap();
        pen
    }

    /// The embedded font file of the font object `id`.
    fn font_file(p: &Pdf, id: ObjectId) -> Vec<u8> {
        let font = p.doc.get_dictionary(id).unwrap();
        let cid = font.get(b"DescendantFonts").unwrap().as_array().unwrap()[0]
            .as_reference()
            .unwrap();
        let cid = p.doc.get_dictionary(cid).unwrap();
        let desc = cid.get(b"FontDescriptor").unwrap().as_reference().unwrap();
        let desc = p.doc.get_dictionary(desc).unwrap();
        let file = desc.get(b"FontFile2").unwrap().as_reference().unwrap();
        let file = p.doc.get_object(file).unwrap().as_stream().unwrap();
        let data = file.get_plain_content().unwrap();
        assert_eq!(
            file.dict.get(b"Length1").unwrap().as_i64().unwrap(),
            data.len() as i64
        );
        data
    }

    #[test]
    fn the_subset_draws_each_glyph_as_the_face_does() {
        // Accented letters are made of parts in Liberation: the parts come along.
        let text = "Ẫẩắ ǅǈ Ŵŷ ÀÉÎõü ΐΰ Ѐйё ½ ffi";
        for f in [SANS, choice(Family::Times, true, true)] {
            let mut p = blank();
            let set = p.embed_text(f, text).unwrap();
            let face = Face::liberation(f);
            let full = face.font();
            let data = font_file(&p, set.font);
            let sub = FontRef::new(&data).unwrap();
            let sub_widths = sub.glyph_metrics(Size::unscaled(), LocationRef::default());
            let full_widths = full.glyph_metrics(Size::unscaled(), LocationRef::default());
            for c in text.nfc() {
                let g = face.glyph(c).unwrap() as u32;
                let i = set.bytes.chunks(2).zip(text.nfc()).find(|(_, d)| *d == c);
                let cid = u16::from_be_bytes(i.unwrap().0.try_into().unwrap()) as u32;
                assert_eq!(outline(&sub, cid), outline(&full, g), "{c:?}");
                assert_eq!(
                    sub_widths.advance_width(GlyphId::new(cid)),
                    full_widths.advance_width(GlyphId::new(g)),
                    "{c:?}"
                );
            }
            // A few characters cost a few KB, not the face's 400.
            assert!(data.len() < 40_000, "{} bytes", data.len());
            assert!(face.data.len() > 300_000);
        }
    }

    #[test]
    fn readers_get_the_faces_widths() {
        let mut p = blank();
        let text = "Tiếng Việt, Ελληνικά, Кириллица!";
        let set = p.embed_text(SANS, text).unwrap();
        let font = p.doc.get_dictionary(set.font).unwrap();
        let ctx = FontCtx::new(&p.doc, font);
        let face = Face::liberation(SANS);
        for (code, c) in set.bytes.chunks(2).zip(text.nfc()) {
            let code = u16::from_be_bytes(code.try_into().unwrap()) as u32;
            let want = face.width(&c.to_string(), 1000.0);
            assert_eq!(ctx.width(code, 2), want, "{c:?}");
        }
        let total: f32 = text.chars().map(|c| face.width(&c.to_string(), 12.0)).sum();
        assert!((face.width(text, 12.0) - total).abs() < 1e-3);
    }

    #[test]
    fn the_faces_have_the_standard_fonts_widths() {
        let ascii: String = (' '..='~').collect();
        for (f, ps) in every_face() {
            let face = Face::liberation(f);
            for c in ascii.chars() {
                let want = crate::std14::width(f.base_font(), c).unwrap();
                assert_eq!(face.width(&c.to_string(), 1000.0), want, "{ps} {c:?}");
            }
        }
    }

    #[test]
    fn accents_typed_apart_are_put_together() {
        let mut p = blank();
        let apart = p.embed_text(SANS, "Cafe\u{301} ou\u{308}").unwrap();
        let together = p.embed_text(SANS, "Café oü").unwrap();
        assert_eq!(apart.bytes, together.bytes);
        show(&mut p, &[(SANS, "Cafe\u{301}")]);
        assert!(reopened(&mut p).page_text(0).contains("Café"));
    }

    #[test]
    fn letters_the_face_has_only_in_parts_are_written_in_them() {
        let face = Face::liberation(SANS);
        let parted: Vec<char> = ('\u{C0}'..='\u{2FFF}')
            .filter(|&c| !face.covers(c))
            .filter(|&c| {
                let parts: Vec<char> = c.to_string().nfd().collect();
                parts.len() > 1 && parts.iter().all(|&p| face.covers(p))
            })
            .collect();
        assert!(!parted.is_empty());
        let text: String = parted.iter().take(20).collect();
        let mut p = blank();
        show(&mut p, &[(SANS, &text)]);
        let back = reopened(&mut p).page_text(0);
        // Read back in parts; the same text once put together (U+212B ANGSTROM SIGN is
        // Å in NFC).
        let back: String = back.trim().nfc().collect();
        assert_eq!(back, text.nfc().collect::<String>());
        assert!(face.width(&text, 10.0) > 0.0);
    }

    #[test]
    fn password_protected_files_and_undo_copies_keep_the_font() {
        let mut p = blank();
        show(&mut p, &[(SANS, "Ελληνικά")]);
        let font = own_fonts(&p)[0];
        // Undo copies are saved as they are.
        let mut copy = Pdf::from_bytes(&p.snapshot().unwrap(), None).unwrap();
        assert_eq!(copy.embed_text(SANS, "λ").unwrap().font, font);

        p.set_encryption(Some(crate::Encryption {
            user_password: "user".into(),
            owner_password: "owner".into(),
            cipher: crate::Cipher::Aes256,
            restrictions: Default::default(),
        }));
        let locked = p.to_bytes().unwrap();
        // The text map is encrypted with the rest.
        assert!(!locked.windows(11).any(|w| w == b"beginbfchar"));
        let mut back = Pdf::from_bytes(&locked, Some("user")).unwrap();
        assert!(back.page_text(0).contains("Ελληνικά"));
        let fonts = own_fonts(&back);
        assert_eq!(fonts.len(), 1);
        assert_eq!(back.embed_text(SANS, "Ωω").unwrap().font, fonts[0]);
    }

    #[test]
    fn characters_the_face_lacks_are_refused_with_nothing_changed() {
        let mut p = blank();
        p.embed_text(SANS, "Ąą").unwrap();
        let before = p.doc.objects.clone();
        // Liberation has Hebrew, but it is written right to left.
        assert!(Face::liberation(SANS).font().charmap().map('ש').is_some());
        for text in ["Ąą 日本語", "שלום", "tab\there", "😀"] {
            let e = p.embed_text(SANS, text).unwrap_err().to_string();
            assert!(e.contains("LiberationSans has no characters for"), "{e}");
            assert!(p.doc.objects == before, "{text:?} changed the document");
        }
        // Each missing character named once.
        let e = p.embed_text(SANS, "a 日本 b 日").unwrap_err().to_string();
        assert!(e.ends_with("\"日本\""), "{e}");
    }

    #[test]
    fn fonts_changed_by_others_are_not_grown() {
        let made = |p: &mut Pdf| p.embed_text(SANS, "Łódź").unwrap().font;

        // A Liberation subset another app wrote (LibreOffice writes them): no record.
        let mut p = blank();
        let theirs = made(&mut p);
        p.doc.get_dictionary_mut(theirs).unwrap().remove(KEY);
        let ours = made(&mut p);
        assert_ne!(ours, theirs);
        assert_eq!(made(&mut p), ours);

        // Ours, with its font file rewritten since.
        let mut p = blank();
        let first = made(&mut p);
        let set = p.embed_text(SANS, "x").unwrap();
        assert_eq!(set.font, first);
        let file = {
            let font = p.doc.get_dictionary(first).unwrap();
            let cid = font.get(b"DescendantFonts").unwrap().as_array().unwrap()[0]
                .as_reference()
                .unwrap();
            let desc = p
                .doc
                .get_dictionary(cid)
                .unwrap()
                .get(b"FontDescriptor")
                .unwrap();
            let desc = p.doc.get_dictionary(desc.as_reference().unwrap()).unwrap();
            desc.get(b"FontFile2").unwrap().as_reference().unwrap()
        };
        let stream = p.doc.get_object_mut(file).unwrap().as_stream_mut().unwrap();
        let mut data = stream.content.clone();
        data.push(0);
        stream.set_content(data);
        let fresh = made(&mut p);
        assert_ne!(fresh, first);

        // Ours, from another version of the face.
        let mut p = blank();
        let first = made(&mut p);
        let own = p
            .doc
            .get_dictionary_mut(first)
            .unwrap()
            .get_mut(KEY)
            .unwrap();
        own.as_dict_mut().unwrap().set(
            "Source",
            Object::String(vec![0; 8], lopdf::StringFormat::Hexadecimal),
        );
        assert_ne!(made(&mut p), first);

        // Ours, with a record that doesn't match the face.
        let mut p = blank();
        let first = made(&mut p);
        let own = p
            .doc
            .get_dictionary_mut(first)
            .unwrap()
            .get_mut(KEY)
            .unwrap();
        let own = own.as_dict_mut().unwrap();
        let mut glyphs = own.get(b"Glyphs").unwrap().as_array().unwrap().clone();
        glyphs[1] = Object::Integer('?' as i64);
        own.set("Glyphs", glyphs);
        assert_ne!(made(&mut p), first);

        // Ours, with a glyph listed twice.
        let mut p = blank();
        let first = made(&mut p);
        let own = p
            .doc
            .get_dictionary_mut(first)
            .unwrap()
            .get_mut(KEY)
            .unwrap();
        let own = own.as_dict_mut().unwrap();
        let mut glyphs = own.get(b"Glyphs").unwrap().as_array().unwrap().clone();
        glyphs.extend(glyphs[..2].to_vec());
        own.set("Glyphs", glyphs);
        assert_ne!(made(&mut p), first);
    }

    #[test]
    fn the_text_map_spells_characters_beyond_the_first_plane() {
        let map = to_unicode(&[(5, 'A'), (9, '𝐀'), (3, 'ж')]);
        assert!(map.contains("<0002> <D835DC00>"), "{map}");
        let read = crate::tounicode::ToUnicode::parse(map.as_bytes()).unwrap();
        assert_eq!(read.get(1), Some("A"));
        assert_eq!(read.get(2), Some("𝐀"));
        assert_eq!(read.get(3), Some("ж"));
        // More than 100 glyphs: in blocks of 100, as the CMap language limits them.
        let many: Vec<(u16, char)> = ('a'..='z').cycle().take(250).map(|c| (1, c)).collect();
        let map = to_unicode(&many);
        assert_eq!(map.matches("100 beginbfchar").count(), 2);
        assert!(map.contains("50 beginbfchar"));
        let read = crate::tounicode::ToUnicode::parse(map.as_bytes()).unwrap();
        assert_eq!(read.get(250), Some("p"));
    }

    /// Runs an installed tool on `bytes`, for the opt-in cross-checks; `{}` in `args` is the
    /// file. Its standard output and error.
    fn tool(name: &str, args: &[&str], bytes: &[u8]) -> (String, String) {
        let dir = std::env::temp_dir().join(format!("limarepdf-embed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.pdf"));
        std::fs::write(&path, bytes).unwrap();
        let args: Vec<String> = args
            .iter()
            .map(|a| a.replace("{}", path.to_str().unwrap()))
            .collect();
        let out = std::process::Command::new(name)
            .args(&args)
            .output()
            .unwrap_or_else(|e| panic!("{name} installed: {e}"));
        std::fs::remove_file(&path).unwrap();
        assert!(out.status.success(), "{name} failed");
        let s = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        (s(&out.stdout), s(&out.stderr))
    }

    fn every_word() -> Vec<u8> {
        let mut p = blank();
        let faces = every_face();
        let lines: Vec<(FontChoice, &str)> = WORDS
            .iter()
            .enumerate()
            .map(|(i, w)| (faces[(3 * i) % faces.len()].0, *w))
            .collect();
        show(&mut p, &lines);
        p.to_bytes().unwrap()
    }

    /// Poppler and Ghostscript read the text, and poppler lists the fonts as embedded
    /// subsets with a text map: `cargo test -p limarepdf-core embed -- --ignored cross_check`.
    #[test]
    #[ignore]
    fn cross_check_with_poppler_and_ghostscript() {
        let bytes = every_word();
        let (text, _) = tool("pdftotext", &["{}", "-"], &bytes);
        let gs = [
            "-q",
            "-dNOPAUSE",
            "-dBATCH",
            "-dSAFER",
            "-sDEVICE=txtwrite",
            "-sOutputFile=-",
            "{}",
        ];
        let (gs_text, gs_errors) = tool("gs", &gs, &bytes);
        for w in WORDS {
            assert!(text.contains(w), "poppler: {w:?} in {text:?}");
            assert!(gs_text.contains(w), "Ghostscript: {w:?} in {gs_text:?}");
        }
        assert!(gs_errors.trim().is_empty(), "Ghostscript: {gs_errors}");

        let (fonts, errors) = tool("pdffonts", &["{}"], &bytes);
        assert!(errors.trim().is_empty(), "pdffonts: {errors}");
        // One font per face, however many lines use it.
        let rows: Vec<&str> = fonts.lines().skip(2).collect();
        assert_eq!(rows.len(), 4, "{fonts}");
        for row in rows {
            // name, type, encoding, then emb sub uni: all yes.
            assert!(
                row.contains("CID TrueType") && row.contains("Identity-H"),
                "{row}"
            );
            let flags: Vec<&str> = row.split_whitespace().rev().skip(2).take(3).collect();
            assert_eq!(flags, ["yes", "yes", "yes"], "{row}");
        }

        // Ghostscript draws it without complaint.
        let draw = [
            "-q",
            "-dNOPAUSE",
            "-dBATCH",
            "-dSAFER",
            "-sDEVICE=nullpage",
            "{}",
        ];
        let (out, errors) = tool("gs", &draw, &bytes);
        assert!(
            (out + &errors).trim().is_empty(),
            "Ghostscript drawing: {errors}"
        );
    }
}
