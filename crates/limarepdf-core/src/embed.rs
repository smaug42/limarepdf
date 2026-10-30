//! Fonts embedded in PDFs, for text the 12 standard fonts can't write (the Unicode text
//! design, section 4): Polish, Czech, Vietnamese, Greek, Russian and the rest.
//!
//! A face is written once per document as a Type 0 font with a CIDFontType2 descendant and
//! `Identity-H` encoding: two bytes a glyph, each a code of the font's own, which a
//! `/CIDToGIDMap` stream maps to the glyph in the embedded font. A code stands for one glyph
//! written for one text, so a ligature's glyph maps to all its letters and one glyph two
//! characters share (Latin A and Cyrillic А, in many fonts) maps back to each. `/W` gives
//! the widths as the face has them, so every reader spaces the text alike, and a ToUnicode
//! map the text, so search, copy and LimarePDF's own text reading get it back. The font file
//! is the face cut down to the glyphs used by the `subsetter` crate, which numbers the
//! glyphs it keeps in the order it is given them. What is written is shaped first
//! (`layout.rs`, [`Face::shape`]): kerning, ligatures and marks as the font places them.
//!
//! When more text is set in the face, the same font object is rewritten with the old codes
//! given first, so they keep their numbers and text written before stays valid. What that
//! needs is kept in the font dictionary under `/LimarePDF` (readers ignore keys they don't
//! know): the face, each code's glyph and text, and a checksum of the font file. A font
//! object whose font file has changed since (another app rewrote it), or that was made from
//! another version of the face, isn't grown: a new one is started.
//!
//! Installed fonts (`typefaces.rs`) are written the same way, always embedded. An OpenType
//! font with CFF outlines is written as a CIDFontType0 font with a `/FontFile3 /OpenType`
//! font file, which takes no map: each code is its glyph's number in the font file, as in
//! the fonts LimarePDF 0.11 wrote, which go on growing so. In those a glyph is written for
//! one text only; text that needs it for another goes to another font object of the face. A
//! variable font is set at the chosen weight as it is cut down, which gives a TrueType font
//! whatever its outlines.

use crate::{Family, FontChoice, Pdf, Result};
use anyhow::{anyhow, bail};
use harfrust::{
    BufferClusterLevel, BufferFlags, Direction, Feature, ShapeOptions, ShapePlan, ShaperData,
    ShaperInstance, UnicodeBuffer, Variation,
};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};
use sha2::{Digest, Sha256};
use skrifa::instance::Location;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{FontRef, LocationRef, MetadataProvider, Size};
use skrifa::{GlyphId, Tag};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use subsetter::GlyphRemapper;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

/// The bundled Liberation faces (see `assets/fonts/README.md`): sans, serif and mono, each
/// regular, bold, italic and bold italic, by name. They have the widths of Helvetica, Times
/// and Courier. Their files are given by [`bundled_face`], which the app, drawing typed text
/// in them, uses too, so there is one copy.
pub static LIBERATION: [[&str; 4]; 3] = [
    [
        "Liberation Sans",
        "Liberation Sans Bold",
        "Liberation Sans Italic",
        "Liberation Sans Bold Italic",
    ],
    [
        "Liberation Serif",
        "Liberation Serif Bold",
        "Liberation Serif Italic",
        "Liberation Serif Bold Italic",
    ],
    [
        "Liberation Mono",
        "Liberation Mono Bold",
        "Liberation Mono Italic",
        "Liberation Mono Bold Italic",
    ],
];

/// A bundled face's file name, and the file as stored: packed with zlib by `build.rs`.
macro_rules! packed {
    ($file:literal) => {
        (
            $file,
            include_bytes!(concat!(env!("OUT_DIR"), "/", $file, ".z")),
        )
    };
}

/// The files of the faces in [`LIBERATION`], in its order, as stored.
static PACKED: [(&str, &[u8]); 12] = [
    packed!("LiberationSans-Regular.ttf"),
    packed!("LiberationSans-Bold.ttf"),
    packed!("LiberationSans-Italic.ttf"),
    packed!("LiberationSans-BoldItalic.ttf"),
    packed!("LiberationSerif-Regular.ttf"),
    packed!("LiberationSerif-Bold.ttf"),
    packed!("LiberationSerif-Italic.ttf"),
    packed!("LiberationSerif-BoldItalic.ttf"),
    packed!("LiberationMono-Regular.ttf"),
    packed!("LiberationMono-Bold.ttf"),
    packed!("LiberationMono-Italic.ttf"),
    packed!("LiberationMono-BoldItalic.ttf"),
];

/// The bundled faces' files, each unpacked the first time it is used.
static UNPACKED: [OnceLock<Vec<u8>>; 12] = [const { OnceLock::new() }; 12];

/// The file of the bundled face `name` (one of [`LIBERATION`]), unpacked the first time it is
/// asked for and kept for the rest of the run; `None` for other names.
pub fn bundled_face(name: &str) -> Option<&'static [u8]> {
    let n = LIBERATION.iter().flatten().position(|l| *l == name)?;
    Some(unpacked(n))
}

/// The file of the `n`th bundled face, unpacked: byte for byte the file in `assets/fonts`.
fn unpacked(n: usize) -> &'static [u8] {
    UNPACKED[n].get_or_init(|| {
        let mut data = Vec::new();
        flate2::read::ZlibDecoder::new(PACKED[n].1)
            .read_to_end(&mut data)
            .expect("a bundled face unpacks");
        data
    })
}

/// The bundled face for a standard font, by name: Helvetica is Liberation Sans, and so on.
/// For an installed family, its standard stand-in's ([`FontChoice::standard`]).
pub fn liberation(f: FontChoice) -> &'static str {
    let (family, style) = place(f);
    LIBERATION[family][style]
}

/// Where the face of the standard font nearest `f` is in [`LIBERATION`]: its family, and its
/// style in that.
fn place(f: FontChoice) -> (usize, usize) {
    let f = f.standard();
    let family = match f.family {
        Family::Times => 1,
        Family::Courier => 2,
        Family::Helvetica | Family::Installed(_) => 0,
    };
    (family, f.bold as usize + 2 * f.italic as usize)
}

/// Key of LimarePDF's own entry in the fonts it writes.
pub(crate) const KEY: &[u8] = b"LimarePDF";
/// Most glyphs one embedded font holds: a glyph number is two bytes, and the subsetter's
/// numbering stops short of 65,536.
const MAX_GLYPHS: usize = 0xFFF0;

/// A face that can be embedded.
pub(crate) struct Face {
    /// Its PostScript name ("LiberationSans-Bold"), as PDF names it.
    ps: String,
    data: &'static [u8],
    /// Its index in the font file (a collection holds several).
    index: u32,
    /// The variation settings of a variable font set at a weight; empty for others.
    coords: Vec<([u8; 4], f32)>,
    /// Where in the font's design space it is set (the default for a static font).
    location: Location,
    /// Whether it is written as an OpenType CFF font (`/FontFile3`): one with CFF outlines.
    /// TrueType, and variable fonts (made TrueType when set at a weight), are `/FontFile2`.
    cff: bool,
    /// The first bytes of the face file's checksum (with its index and settings, when it
    /// has any), so a font object made from another version or weight of the face is told
    /// apart.
    source: [u8; 8],
    /// Each glyph's width, in thousandths of an em, as the face has it.
    advances: Vec<f32>,
    /// Each glyph's width, in thousandths of an em, rounded as `/W` gives it in the fonts
    /// LimarePDF 0.11 wrote.
    widths: Vec<i64>,
    /// Font units per em.
    upem: f32,
    /// Whether each glyph has an outline ([`Face::outlined`]), found the first time asked.
    outlines: Box<[OnceLock<bool>]>,
    /// What shaping needs of the face, made the first time it is shaped.
    shaper: OnceLock<ShaperData>,
    /// Where a variable font is set, for shaping; `None` for others.
    instance: Option<ShaperInstance>,
    /// The shaping plans made, by script, language and whether only kerning was asked.
    plans: Mutex<Vec<(PlanKey, Arc<ShapePlan>)>>,
}

/// What a shaping plan is made for: a script, a language and whether only kerning is wanted.
type PlanKey = (harfrust::Script, Option<harfrust::Language>, bool);

/// One glyph as [`Face::shape`] gives it: the face's glyph, the characters of the text it
/// draws (from and to; none for a glyph after the first that draws a character, a mark
/// placed apart, say), and how far it moves the line on and where it is placed from there,
/// in thousandths of an em.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shaped {
    pub glyph: u16,
    pub chars: (usize, usize),
    pub advance: f32,
    pub x: f32,
    pub y: f32,
}

/// The features turned off for text written in a standard font itself, which shows each
/// character's own glyph: only kerning is kept.
const NO_LIGATURES: [&[u8; 4]; 3] = [b"liga", b"clig", b"calt"];

/// Largest embedded font file read back to grow it, once decompressed.
const MAX_FILE: usize = 32 << 20;
/// Largest installed font file read (collections of Chinese, Japanese and Korean fonts run to
/// tens of megabytes).
const MAX_INSTALLED: u64 = 256 << 20;

/// The bundled faces, each read the first time it is needed.
static FACES: [OnceLock<Face>; 12] = [const { OnceLock::new() }; 12];

/// An installed face's font file, index in it and PostScript name.
type FaceKey = (PathBuf, u32, String);

/// The installed faces read: each is read once and kept for the rest of the run, as the
/// bundled ones are.
static INSTALLED: Mutex<Vec<(FaceKey, &'static Face)>> = Mutex::new(Vec::new());

impl Face {
    /// The bundled face of the standard font `f` (for an installed family, its standard
    /// stand-in's).
    pub(crate) fn liberation(f: FontChoice) -> &'static Face {
        let (family, style) = place(f);
        FACES[4 * family + style].get_or_init(|| {
            Face::read(unpacked(4 * family + style), 0, Vec::new(), None)
                .expect("a bundled face reads")
        })
    }

    /// The face text in `f` is written in when it is embedded: a standard font's bundled
    /// face, or the installed family's face, read from its file the first time.
    pub(crate) fn of(f: FontChoice) -> Result<&'static Face> {
        let Family::Installed(n) = f.family else {
            return Ok(Face::liberation(f));
        };
        let t = crate::typeface(n).ok_or_else(|| anyhow!("that font isn't available"))?;
        let file = t.face(f.bold, f.italic).ok_or_else(|| {
            let style = match (f.bold, f.italic) {
                (true, true) => "bold italic",
                (true, false) => "bold",
                (false, true) => "italic",
                (false, false) => "regular",
            };
            anyhow!("{} has no {style} face", t.label())
        })?;
        let key = (file.path.clone(), file.index, file.ps.clone());
        let mut read = INSTALLED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, face)) = read.iter().find(|(k, _)| *k == key) {
            return Ok(face);
        }
        let cant = |e: &dyn std::fmt::Display| anyhow!("can't read the font {}: {e}", t.label());
        let size = std::fs::metadata(&file.path).map_err(|e| cant(&e))?.len();
        if size > MAX_INSTALLED {
            bail!("the font {} is too large to embed", t.label());
        }
        let data: &'static [u8] = std::fs::read(&file.path).map_err(|e| cant(&e))?.leak();
        let face = Face::read(data, file.index, file.coords.clone(), Some(&file.ps))
            .map_err(|e| cant(&e))?;
        let face: &'static Face = Box::leak(Box::new(face));
        read.push((key, face));
        Ok(face)
    }

    /// The face at `index` in the font file `data`, set at `coords`, named `ps` (else by
    /// the font's own PostScript name).
    fn read(
        data: &'static [u8],
        index: u32,
        coords: Vec<([u8; 4], f32)>,
        ps: Option<&str>,
    ) -> Result<Self> {
        let font =
            FontRef::from_index(data, index).map_err(|e| anyhow!("can't read the font: {e}"))?;
        let ps = match ps {
            Some(ps) => ps.to_string(),
            None => font
                .localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME)
                .english_or_first()
                .map(|s| s.to_string())
                .unwrap_or_default(),
        };
        // A PDF name of the usual kind: letters, digits and hyphens.
        let ps: String = ps
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if ps.is_empty() {
            bail!("the font has no PostScript name");
        }
        let has = |t: &[u8; 4]| font.table_data(Tag::new(t)).is_some();
        let cff = has(b"CFF ") && !has(b"glyf");
        if !cff && !has(b"glyf") && !has(b"CFF2") {
            bail!("the font has no outlines");
        }
        let location = font
            .axes()
            .location(coords.iter().map(|(t, v)| (Tag::new(t), *v)));
        let at = LocationRef::from(&location);
        let m = font.metrics(Size::unscaled(), at);
        let em = 1000.0 / m.units_per_em.max(1) as f32;
        let gm = font.glyph_metrics(Size::unscaled(), at);
        let advances: Vec<f32> = (0..m.glyph_count as u32)
            .map(|g| gm.advance_width(GlyphId::new(g)).unwrap_or(0.0) * em)
            .collect();
        let widths = advances.iter().map(|w| w.round() as i64).collect();
        let outlines = (0..m.glyph_count).map(|_| OnceLock::new()).collect();
        let instance = (!coords.is_empty()).then(|| {
            let at = coords.iter().map(|(t, v)| Variation {
                tag: Tag::new(t),
                value: *v,
            });
            ShaperInstance::from_variations(&font, at)
        });
        let mut sum = Sha256::new();
        sum.update(data);
        if index != 0 || !coords.is_empty() {
            sum.update(index.to_be_bytes());
            for (t, v) in &coords {
                sum.update(t);
                sum.update(v.to_be_bytes());
            }
        }
        let mut source = [0; 8];
        source.copy_from_slice(&sum.finalize()[..8]);
        Ok(Self {
            ps,
            data,
            index,
            coords,
            location,
            cff,
            source,
            advances,
            widths,
            upem: m.units_per_em.max(1) as f32,
            outlines,
            shaper: OnceLock::new(),
            instance,
            plans: Mutex::new(Vec::new()),
        })
    }

    /// Whether glyph `g` is in the face.
    pub(crate) fn has_glyph(&self, g: u16) -> bool {
        (g as usize) < self.advances.len()
    }

    /// Glyph `g`'s width as the face has it, in thousandths of an em.
    pub(crate) fn advance(&self, g: u16) -> f32 {
        self.advances.get(g as usize).copied().unwrap_or(0.0)
    }

    /// Glyph `g`'s width as `/W` gives it: to a thousandth of a unit when `exact`, else
    /// rounded, as LimarePDF 0.11 wrote it.
    pub(crate) fn width_of(&self, g: u16, exact: bool) -> f32 {
        match exact {
            true => (self.advance(g) * 1000.0).round() / 1000.0,
            false => self.widths.get(g as usize).copied().unwrap_or(0) as f32,
        }
    }

    /// `text` shaped in the face, left to right in `script` (and `language`, when known),
    /// with the font's own features (kerning, ligatures, contextual forms, marks placed),
    /// or with `kerning_only` one glyph for each character, spaced by the font's kerning.
    /// Each character is a cluster of its own (a ligature's letters and a glyph's parts
    /// aside), so a mark keeps its own glyph's text.
    pub(crate) fn shape(
        &self,
        text: &[char],
        script: harfrust::Script,
        language: Option<&harfrust::Language>,
        kerning_only: bool,
    ) -> Vec<Shaped> {
        if text.is_empty() {
            return Vec::new();
        }
        let font = self.font();
        let data = self.shaper.get_or_init(|| ShaperData::new(&font));
        let shaper = data.shaper(&font).instance(self.instance.as_ref()).build();
        let features: Vec<Feature> = match kerning_only {
            true => (NO_LIGATURES.iter())
                .map(|t| Feature::new(Tag::new(t), 0, ..))
                .collect(),
            false => Vec::new(),
        };
        let key: PlanKey = (script, language.cloned(), kerning_only);
        let plan = {
            let mut plans = self.plans.lock().unwrap_or_else(|e| e.into_inner());
            match plans.iter().find(|(k, _)| *k == key) {
                Some((_, p)) => Arc::clone(p),
                None => {
                    let p = Arc::new(ShapePlan::new(
                        &shaper,
                        Direction::LeftToRight,
                        Some(script),
                        language,
                        &features,
                    ));
                    plans.push((key, Arc::clone(&p)));
                    p
                }
            }
        };
        let s: String = text.iter().collect();
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(&s);
        buffer.set_direction(Direction::LeftToRight);
        buffer.set_script(script);
        if let Some(l) = language {
            buffer.set_language(l.clone());
        }
        buffer.set_cluster_level(BufferClusterLevel::MonotoneCharacters);
        buffer.set_flags(BufferFlags::BEGINNING_OF_TEXT | BufferFlags::END_OF_TEXT);
        let options = ShapeOptions::new().plan(Some(&plan)).features(&features);
        let out = shaper.shape(buffer, options);
        // Clusters are where their characters start in `s`, in bytes.
        let starts: Vec<usize> = s.char_indices().map(|(b, _)| b).collect();
        let at = |cluster: u32| starts.partition_point(|&b| b < cluster as usize);
        let (infos, places) = (out.glyph_infos(), out.glyph_positions());
        let k = 1000.0 / self.upem;
        let mut glyphs = Vec::with_capacity(infos.len());
        for (i, (info, place)) in infos.iter().zip(places).enumerate() {
            let from = at(info.cluster);
            let first = i == 0 || infos[i - 1].cluster != info.cluster;
            let to = (infos[i + 1..].iter())
                .find(|g| g.cluster != info.cluster)
                .map_or(text.len(), |g| at(g.cluster));
            glyphs.push(Shaped {
                glyph: info.glyph_id as u16,
                chars: if first { (from, to) } else { (from, from) },
                advance: place.x_advance as f32 * k,
                x: place.x_offset as f32 * k,
                y: place.y_offset as f32 * k,
            });
        }
        glyphs
    }

    fn font(&self) -> FontRef<'static> {
        FontRef::from_index(self.data, self.index).expect("read when the face was")
    }

    /// Its weight: the weight it is set at, else the font's own.
    fn weight(&self) -> f32 {
        self.coords
            .iter()
            .find(|(t, _)| t == b"wght")
            .map_or_else(|| self.font().attributes().weight.value(), |(_, v)| *v)
    }

    /// The face's glyph for `c`; `None` for characters it lacks, control characters, those
    /// of right-to-left and shaped scripts and the parts of emoji sequences
    /// ([`emoji_part`]), and for a character meant to show something, a glyph with no
    /// outline: a colour emoji's, say, drawn by colour layers or pictures a PDF doesn't
    /// have, so it would show nothing.
    fn glyph(&self, c: char) -> Option<u16> {
        if c.is_control() || right_to_left(c) || shaped(c) || emoji_part(c) {
            return None;
        }
        let g = self.font().charmap().map(c)?.to_u32() as u16;
        (g != 0 && (blank(c) || self.outlined(g))).then_some(g)
    }

    /// The face's glyph for `c` as its character map gives it, whatever it is.
    pub(crate) fn glyph_of(&self, c: char) -> Option<u16> {
        self.font()
            .charmap()
            .map(c)
            .map(|g| g.to_u32() as u16)
            .filter(|&g| g != 0)
    }

    /// Whether glyph `g` draws anything, set as the face is.
    fn outlined(&self, g: u16) -> bool {
        let Some(known) = self.outlines.get(g as usize) else {
            return false;
        };
        *known.get_or_init(|| {
            let mut pen = Strokes(0);
            let settings =
                DrawSettings::unhinted(Size::unscaled(), LocationRef::from(&self.location));
            self.font()
                .outline_glyphs()
                .get(GlyphId::new(g.into()))
                .is_some_and(|o| o.draw(settings, &mut pen).is_ok() && pen.0 > 0)
        })
    }

    /// Whether the face has a glyph for `c` that LimarePDF can write (control characters,
    /// right-to-left and shaped scripts and the parts of emoji sequences never).
    pub(crate) fn covers(&self, c: char) -> bool {
        self.glyph(c).is_some()
    }

    /// The characters `text` is written with in the face: put together (NFC) where the face
    /// has the combined letter, else apart where it has the parts; and those it can't write,
    /// each once, an emoji sequence whole ([`emoji_sequence`]).
    pub(crate) fn spelled(&self, text: &str) -> (Vec<char>, String) {
        let (mut out, mut missing) = (Vec::new(), String::new());
        let text: String = text.nfc().collect();
        for cluster in text.graphemes(true) {
            if emoji_sequence(cluster) {
                if !missing.contains(cluster) {
                    missing.push_str(cluster);
                }
                continue;
            }
            for c in cluster.chars() {
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
        }
        (out, missing)
    }

    /// The characters `text` is written with in the face ([`Face::spelled`]), or those it
    /// can't write, when there are any.
    #[cfg(test)]
    fn spell(&self, text: &str) -> std::result::Result<Vec<char>, String> {
        match self.spelled(text) {
            (out, missing) if missing.is_empty() => Ok(out),
            (_, missing) => Err(missing),
        }
    }
}

/// Whether LimarePDF can write `c` in `f`, or in a font it falls back to for it
/// (`layout.rs`): a standard font itself or its embedded face, an installed family's face,
/// or another installed font that has it. Control characters never.
pub fn writable(f: FontChoice, c: char) -> bool {
    !c.is_control() && unwritable(f, c.encode_utf8(&mut [0; 4])).is_empty()
}

/// The message refusing text with the characters `missing`, which no font can write: its
/// emoji sequences ([`emoji_sequence`]), and the characters of scripts LimarePDF doesn't
/// write yet ([`unwritten_script`]), when it has any, as no font helps with those; else
/// that no installed font has them.
pub(crate) fn cant_write(missing: &str) -> String {
    let sequences = sequences(missing);
    if !sequences.is_empty() {
        return format!(
            "LimarePDF can't write \"{sequences}\" yet: flags, skin tones, keycaps and joined \
             emoji aren't supported"
        );
    }
    let script: String = missing.chars().filter(|&c| unwritten_script(c)).collect();
    if !script.is_empty() {
        return format!(
            "LimarePDF can't write \"{script}\" yet: right-to-left writing, and scripts whose \
             letters join, such as Arabic, Hebrew and Hindi, aren't supported"
        );
    }
    format!("No font on this computer has \"{missing}\"")
}

/// The emoji sequences ([`emoji_sequence`]) among the characters `missing`.
fn sequences(missing: &str) -> String {
    missing
        .graphemes(true)
        .filter(|g| emoji_sequence(g))
        .collect()
}

/// Why a text box with the characters `missing`, which no font can write, can't be drawn:
/// the end of a message that says which box (BUG-39).
pub(crate) fn box_refusal(missing: &str) -> String {
    let sequences = sequences(missing);
    if !sequences.is_empty() {
        return format!(
            "it has \"{sequences}\", and flags, skin tones, keycaps and joined emoji aren't \
             supported yet"
        );
    }
    let script: String = missing.chars().filter(|&c| unwritten_script(c)).collect();
    if !script.is_empty() {
        return format!(
            "it has \"{script}\", in a script LimarePDF can't write yet (right-to-left \
             writing, and scripts whose letters join, aren't supported)"
        );
    }
    format!("it has \"{missing}\", which no font on this computer has")
}

/// Whether LimarePDF could write `c` in an installed font that has it: not a control
/// character, nor of a right-to-left or shaped script, nor a part of an emoji sequence
/// ([`emoji_part`]), so choosing another font may help. A single emoji can be, in a font
/// that draws it with an outline (Symbola, say).
pub fn installed_could_write(c: char) -> bool {
    !c.is_control() && !unwritten_script(c) && !emoji_part(c)
}

/// Whether `c` is of a script LimarePDF doesn't write in any font yet: one written right
/// to left, or whose letters change shape or combine.
pub fn unwritten_script(c: char) -> bool {
    right_to_left(c) || shaped(c)
}

/// The message refusing text in `f` with the characters `missing`, which no font can write
/// ([`cant_write`]; `f` and every font it falls back to lack them).
pub fn refusal(_f: FontChoice, missing: &str) -> String {
    cant_write(missing)
}

/// Refuses `text`, with [`refusal`]'s message, when LimarePDF can't write all of it in
/// `f` and the fonts it falls back to, as [`Pdf::pen`] would (control characters, such as
/// line breaks, aside). Nothing is embedded.
pub(crate) fn check_writable(f: FontChoice, text: &str) -> Result<()> {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    let missing = unwritable(f, &text);
    if !missing.is_empty() {
        bail!("{}", refusal(f, &missing));
    }
    Ok(())
}

/// The characters of `text` LimarePDF can't write in `f` or any font it falls back to
/// (`layout.rs`), each once, an emoji sequence whole (control characters included).
pub(crate) fn unwritable(f: FontChoice, text: &str) -> String {
    crate::layout::Setter::new(f, text, None).missing(text)
}

/// Where each glyph of some text an embedded face writes is ([`Pdf::embed_glyphs`]): the
/// font object, the code that shows it, and its width as the font's `/W` gives it, in
/// thousandths of an em.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placed {
    pub font: ObjectId,
    pub code: u16,
    pub width: f32,
}

/// What LimarePDF keeps in a font it wrote, to grow it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Record {
    /// Each code's glyph of the face and the text it was written for (a ligature's
    /// letters; nothing for a glyph after the first that draws a character), code 1
    /// first.
    glyphs: Vec<(u16, String)>,
    /// Whether the codes are the font's own, mapped to its glyphs by a `/CIDToGIDMap`
    /// stream, so one glyph can be written for two texts. Otherwise each code is its
    /// glyph's number in the font file (`Identity`), and each glyph is written for one
    /// text: CFF fonts, which take no map, and the fonts LimarePDF 0.11 wrote.
    own: bool,
    /// Whether `/W` gives the widths to a thousandth of a unit, as the face has them,
    /// rather than rounded, as LimarePDF 0.11 wrote them.
    exact: bool,
}

/// The objects of an embedded font other than its font dictionary: kept when it grows.
#[derive(Clone, Copy, Default)]
struct Parts {
    descendant: Option<ObjectId>,
    descriptor: Option<ObjectId>,
    file: Option<ObjectId>,
    to_unicode: Option<ObjectId>,
    /// The `/CIDToGIDMap` stream of a font with codes of its own.
    map: Option<ObjectId>,
}

/// Text set in an embedded font: the font object to name in the resources, and the codes
/// that show it.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct Set {
    pub font: ObjectId,
    pub bytes: Vec<u8>,
}

impl Pdf {
    /// `text` set in the embedded face of `f` with nothing falling back, in one font
    /// object.
    #[cfg(test)]
    pub(crate) fn embed_text(&mut self, f: FontChoice, text: &str) -> Result<Set> {
        let face = Face::of(f)?;
        let chars = face
            .spell(text)
            .map_err(|missing| anyhow!("{} has no characters for \"{missing}\"", face.ps))?;
        // One glyph a character, as the character map gives it, so the tests of what is
        // embedded can match codes to characters; shaping has its own (`layout.rs`).
        let script = crate::layout::script_of(&chars);
        let glyphs = face.shape(&chars, script, None, true);
        let needs: Vec<(u16, String)> = (glyphs.iter())
            .map(|g| (g.glyph, chars[g.chars.0..g.chars.1].iter().collect()))
            .collect();
        let placed = self.embed_glyphs(face, &needs)?;
        let font = match placed.first() {
            Some(p) => p.font,
            None => self.embed_glyphs(face, &[(0, String::new())])?[0].font,
        };
        if placed.iter().any(|p| p.font != font) {
            bail!("the text went to two font objects");
        }
        Ok(Set {
            font,
            bytes: placed.iter().flat_map(|p| p.code.to_be_bytes()).collect(),
        })
    }

    /// The embedded face's font objects in this document, grown to hold each of `glyphs`
    /// (a glyph of the face, and the text it is written for), made the first time: where
    /// each is, in order. A glyph a font holds for another text, in a font whose codes are
    /// its glyphs' numbers, goes to another of the face's font objects, made when there is
    /// none.
    pub(crate) fn embed_glyphs(
        &mut self,
        face: &'static Face,
        glyphs: &[(u16, String)],
    ) -> Result<Vec<Placed>> {
        if let Some((g, _)) = glyphs.iter().find(|(g, _)| !face.has_glyph(*g)) {
            bail!("{} has no glyph {g}", face.ps);
        }
        let mut found = self.embedded(face).into_iter();
        // Where each goes: the round of fonts tried, and its code there.
        let mut out: Vec<Option<(usize, u16)>> = vec![None; glyphs.len()];
        let mut rounds: Vec<(ObjectId, bool)> = Vec::new();
        while out.iter().any(Option::is_none) {
            let (at, mut record) = match found.next() {
                Some((id, record, parts)) => (Some((id, parts)), record),
                None => (
                    None,
                    Record {
                        glyphs: Vec::new(),
                        own: !face.cff,
                        exact: true,
                    },
                ),
            };
            let before = record.glyphs.len();
            let mut codes: HashMap<(u16, String), u16> = (record.glyphs.iter().enumerate())
                .map(|(i, (g, t))| ((*g, t.clone()), i as u16 + 1))
                .collect();
            let mut held: HashSet<u16> = record.glyphs.iter().map(|(g, _)| *g).collect();
            let round = rounds.len();
            for (k, (g, t)) in glyphs.iter().enumerate() {
                if out[k].is_some() {
                    continue;
                }
                if let Some(&code) = codes.get(&(*g, t.clone())) {
                    out[k] = Some((round, code));
                    continue;
                }
                if (!record.own && held.contains(g)) || record.glyphs.len() >= MAX_GLYPHS {
                    continue;
                }
                record.glyphs.push((*g, t.clone()));
                let code = record.glyphs.len() as u16;
                codes.insert((*g, t.clone()), code);
                held.insert(*g);
                out[k] = Some((round, code));
            }
            let font = match at {
                Some((id, _)) if record.glyphs.len() == before => id,
                Some((id, parts)) => self.write_font(face, &record, Some(id), parts)?,
                None if record.glyphs.is_empty() => {
                    bail!("too many different characters in one font")
                }
                None => self.write_font(face, &record, None, Parts::default())?,
            };
            self.made_fonts.insert(font);
            rounds.push((font, record.exact));
        }
        Ok((out.into_iter().zip(glyphs))
            .map(|(at, (g, _))| {
                let (round, code) = at.expect("placed");
                let (font, exact) = rounds[round];
                Placed {
                    font,
                    code,
                    width: face.width_of(*g, exact),
                }
            })
            .collect())
    }

    /// This document's font objects for `face` that LimarePDF wrote and can grow, with what
    /// each holds and its other objects, in the order they were made. In a signed document,
    /// only those made since it was opened: growing rewrites objects, and in one a signed
    /// revision holds, checking the signature would see a change.
    fn embedded(&self, face: &Face) -> Vec<(ObjectId, Record, Parts)> {
        let signed = self.holds_signature();
        self.doc
            .objects
            .iter()
            .filter(|(id, _)| !signed || self.made_fonts.contains(id))
            .filter_map(|(&id, o)| Some((id, o.as_dict().ok()?)))
            .filter_map(|(id, d)| {
                let (record, parts) = self.grows(face, d)?;
                Some((id, record, parts))
            })
            .collect()
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
        let name = |k: &[u8]| own.get(k).and_then(Object::as_name).ok();
        let mut record = Record {
            glyphs: Vec::new(),
            own: name(b"Codes") == Some(b"Own"),
            exact: name(b"Widths") == Some(b"Exact"),
        };
        let mut seen = HashSet::new();
        if let Ok(list) = own.get(b"Texts").and_then(Object::as_array) {
            if list.len() > 2 * MAX_GLYPHS || (record.own && face.cff) {
                return None;
            }
            for pair in list.chunks(2) {
                let [g, t] = pair else { return None };
                let g = u16::try_from(g.as_i64().ok()?).ok()?;
                let t = String::from_utf8(t.as_str().ok()?.to_vec()).ok()?;
                // Each glyph once where codes are glyph numbers; each glyph and text once
                // where they are the font's own.
                if !face.has_glyph(g) || !seen.insert((g, (record.own).then(|| t.clone()))) {
                    return None;
                }
                record.glyphs.push((g, t));
            }
        } else {
            // As LimarePDF 0.11 wrote it: each glyph its code, written for one character.
            let list = own.get(b"Glyphs").and_then(Object::as_array).ok()?;
            if list.len() > 2 * MAX_GLYPHS || record.own || record.exact {
                return None;
            }
            for pair in list.chunks(2) {
                let [g, c] = pair else { return None };
                let g = u16::try_from(g.as_i64().ok()?).ok()?;
                let c = char::from_u32(u32::try_from(c.as_i64().ok()?).ok()?)?;
                if face.glyph(c) != Some(g) || !seen.insert((g, None)) {
                    return None;
                }
                record.glyphs.push((g, c.to_string()));
            }
        }
        let reference = |d: &Dictionary, k: &[u8]| d.get(k).and_then(Object::as_reference).ok();
        let descendants = d.get(b"DescendantFonts").and_then(Object::as_array).ok()?;
        let descendant = descendants.first()?.as_reference().ok()?;
        let cid_font = self.doc.get_dictionary(descendant).ok()?;
        let map = reference(cid_font, b"CIDToGIDMap");
        if record.own != map.is_some() {
            return None;
        }
        let descriptor = reference(cid_font, b"FontDescriptor")?;
        let descriptor_dict = self.doc.get_dictionary(descriptor).ok()?;
        let file = reference(descriptor_dict, file_key(face).as_bytes())?;
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
            map,
        };
        Some((record, parts))
    }

    /// Writes the font object for `face` holding `record`'s codes: at `at` with its other
    /// objects in `parts` when it grows, else as new objects.
    fn write_font(
        &mut self,
        face: &Face,
        record: &Record,
        at: Option<ObjectId>,
        parts: Parts,
    ) -> Result<ObjectId> {
        // The glyphs the font file keeps, each once, in the order first written.
        let mut gids: Vec<u16> = Vec::with_capacity(record.glyphs.len());
        let mut kept = HashSet::new();
        for &(g, _) in &record.glyphs {
            if kept.insert(g) {
                gids.push(g);
            }
        }
        let mapper = GlyphRemapper::new_from_glyphs(&gids);
        // Where codes are glyph numbers, the subset's numbers are the codes the text is
        // written with.
        let numbered = (gids.iter().enumerate()).all(|(i, &g)| mapper.get(g) == Some(i as u16 + 1));
        if !record.own && (!numbered || gids.len() != record.glyphs.len()) {
            bail!("the font's glyphs were numbered unexpectedly; nothing was changed");
        }
        let coords: Vec<(subsetter::Tag, f32)> = face
            .coords
            .iter()
            .map(|(t, v)| (subsetter::Tag::new(t), *v))
            .collect();
        let sub = subsetter::subset_with_variations(face.data, face.index, &coords, &mapper)
            .map_err(|e| anyhow!("can't cut down {}: {e}", face.ps))?;
        let sum = Sha256::digest(&sub);
        let base = format!("{}+{}", tag(&sum), face.ps);

        let font = face.font();
        let m = font.metrics(Size::unscaled(), LocationRef::from(&face.location));
        let em = 1000.0 / m.units_per_em.max(1) as f32;
        let scaled = |v: f32| Object::Integer((v * em).round() as i64);
        let bbox = m
            .bounds
            .map_or([0.0; 4], |b| [b.x_min, b.y_min, b.x_max, b.y_max]);
        let weight = face.weight();
        let italic = m.italic_angle != 0.0;
        // Symbolic: the glyphs aren't only the standard Latin set.
        let flags = 4 | m.is_monospace as i64 | (64 * italic as i64) | (2 * serif(&face.ps) as i64);

        let mut file = if face.cff {
            Stream::new(dictionary! { "Subtype" => "OpenType" }, sub)
        } else {
            Stream::new(dictionary! { "Length1" => sub.len() as i64 }, sub)
        };
        file.allows_compression = true;
        let file = self.put(parts.file, file.into());
        let mut descriptor = dictionary! {
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
        };
        descriptor.set(file_key(face), file);
        let descriptor = self.put(parts.descriptor, descriptor.into());
        let width = |g: u16| match record.exact {
            true => Object::Real(face.width_of(g, true)),
            false => Object::Integer(face.widths[g as usize]),
        };
        let widths: Vec<Object> = std::iter::once(0)
            .chain(record.glyphs.iter().map(|&(g, _)| g))
            .map(width)
            .collect();
        let mut descendant = dictionary! {
            "Type" => "Font",
            "Subtype" => if face.cff { "CIDFontType0" } else { "CIDFontType2" },
            "BaseFont" => Object::Name(base.clone().into_bytes()),
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal("Identity"),
                "Supplement" => 0,
            },
            "FontDescriptor" => descriptor,
            "DW" => width(0),
            "W" => vec![0.into(), widths.into()],
        };
        // A CFF font's glyphs are found by CID through its own charset, which the subset
        // makes the glyph numbers; only TrueType fonts take a map: the codes' own, or
        // `Identity` where codes are glyph numbers.
        if record.own {
            let mut map = vec![0, 0];
            for &(g, _) in &record.glyphs {
                map.extend(mapper.get(g).unwrap_or(0).to_be_bytes());
            }
            let mut stream = Stream::new(Dictionary::new(), map);
            stream.allows_compression = true;
            descendant.set("CIDToGIDMap", self.put(parts.map, stream.into()));
        } else if !face.cff {
            descendant.set("CIDToGIDMap", "Identity");
        }
        let descendant = self.put(parts.descendant, descendant.into());
        let to_unicode = self.put(
            parts.to_unicode,
            Stream::new(Dictionary::new(), to_unicode(&record.glyphs).into_bytes()).into(),
        );
        let list: Vec<Object> = (record.glyphs.iter())
            .flat_map(|(g, t)| {
                let text = Object::String(t.as_bytes().to_vec(), lopdf::StringFormat::Hexadecimal);
                [Object::Integer((*g).into()), text]
            })
            .collect();
        let mut own = dictionary! {
            "Face" => Object::string_literal(face.ps.as_str()),
            "Source" => Object::String(face.source.to_vec(), lopdf::StringFormat::Hexadecimal),
            "Texts" => list,
            "Codes" => if record.own { "Own" } else { "Identity" },
            "Sum" => Object::String(sum.to_vec(), lopdf::StringFormat::Hexadecimal),
        };
        if record.exact {
            own.set("Widths", "Exact");
        }
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

/// Whether `c` is of a script whose letters change shape or combine around each other
/// (Devanagari and the other Indic scripts, Thai, Lao, Tibetan, Myanmar, Khmer, Mongolian,
/// conjoining Hangul jamo and the rest): written one glyph per character, without a shaping
/// engine, they would come out wrong (the Unicode text design, section 6). Installed fonts
/// may have them; Liberation doesn't.
fn shaped(c: char) -> bool {
    matches!(
        c,
        '\u{0900}'..='\u{0DFF}'
            | '\u{0E00}'..='\u{0FFF}'
            | '\u{1000}'..='\u{109F}'
            | '\u{1100}'..='\u{11FF}'
            | '\u{1780}'..='\u{18AF}'
            | '\u{1A00}'..='\u{1BFF}'
            | '\u{A800}'..='\u{AAFF}'
            | '\u{ABC0}'..='\u{ABFF}'
            | '\u{D7B0}'..='\u{D7FF}'
            | '\u{11000}'..='\u{11FFF}'
    )
}

/// Whether `cluster`, one character as readers see it (a grapheme cluster), is an emoji
/// made of several characters: with a skin tone, a flag (regional indicators, or tags), a
/// keycap, the emoji variation selector, or emoji joined by the zero-width joiner (families
/// and the like); or one of those parts alone. Written a character at a time it would show
/// something else, and LimarePDF can't write them as one yet (PF-14).
pub fn emoji_sequence(cluster: &str) -> bool {
    cluster.chars().any(emoji_part)
        || (cluster.contains('\u{200D}') && cluster.chars().any(pictograph))
}

/// Whether `c` only makes emoji sequences: the skin tones, the regional indicators (flags),
/// the keycap mark, tags (flags of regions) and the emoji variation selector. The
/// zero-width joiner isn't one: alone, it is an invisible mark some text has.
fn emoji_part(c: char) -> bool {
    matches!(
        c,
        '\u{1F3FB}'..='\u{1F3FF}'
            | '\u{1F1E6}'..='\u{1F1FF}'
            | '\u{20E3}'
            | '\u{FE0F}'
            | '\u{E0000}'..='\u{E007F}'
    )
}

/// Whether `c` is a picture an emoji sequence joins: the emoji blocks (faces, animals and
/// the rest), and the older symbols some sequences use (♀, ❤, ⚕).
fn pictograph(c: char) -> bool {
    matches!(
        c,
        '\u{2600}'..='\u{27BF}' | '\u{2B00}'..='\u{2BFF}' | '\u{1F000}'..='\u{1FAFF}'
    )
}

/// Whether `c` is meant to show nothing: spaces, and invisible marks (Unicode's default
/// ignorable characters: joiners, variation selectors, the soft hyphen and the rest), and
/// the blank Braille pattern. A glyph with no outline is right for these.
fn blank(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '\u{AD}'
                | '\u{34F}'
                | '\u{61C}'
                | '\u{115F}'..='\u{1160}'
                | '\u{17B4}'..='\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{2800}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF0}'..='\u{FFF8}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0000}'..='\u{E0FFF}'
        )
}

/// Counts the lines and curves an outline draws.
struct Strokes(usize);

impl OutlinePen for Strokes {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {
        self.0 += 1;
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn close(&mut self) {}
}

/// The font descriptor's key for a face's font file: `/FontFile3` (with `/Subtype
/// /OpenType`) for a CFF font, `/FontFile2` for TrueType.
fn file_key(face: &Face) -> &'static str {
    if face.cff {
        "FontFile3"
    } else {
        "FontFile2"
    }
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

/// A ToUnicode map from each code (code 1 first) to the text it was written for: several
/// characters for a ligature. A code written for no text (a glyph after the first that
/// draws a character) has no entry.
fn to_unicode(glyphs: &[(u16, String)]) -> String {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<(usize, &str)> = (glyphs.iter().enumerate())
        .filter(|(_, (_, t))| !t.is_empty())
        .map(|(i, (_, t))| (i + 1, t.as_str()))
        .collect();
    for chunk in entries.chunks(100) {
        let _ = writeln!(s, "{} beginbfchar", chunk.len());
        for &(code, text) in chunk {
            let _ = write!(s, "<{code:04X}> <");
            for u in text.encode_utf16() {
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
    fn bundled_faces_unpack_byte_for_byte_as_released() {
        // Stored compressed, but the Reserved Font Name needs the faces unmodified: each
        // unpacks to the file with the checksum the fonts README gives it.
        let readme = include_str!("../assets/fonts/README.md");
        let (mut stored, mut whole) = (0, 0);
        for (n, (file, packed)) in PACKED.iter().enumerate() {
            let row = readme
                .lines()
                .find(|l| l.starts_with(&format!("| {file} |")))
                .unwrap_or_else(|| panic!("{file} isn't in the README"));
            let want = row.split('`').nth(1).unwrap();
            let data = bundled_face(LIBERATION[n / 4][n % 4]).unwrap();
            let mut got = String::new();
            for b in Sha256::digest(data) {
                write!(got, "{b:02x}").unwrap();
            }
            assert_eq!(got, want, "{file}");
            // Named as the file is: "Liberation Serif Bold Italic" is
            // LiberationSerif-BoldItalic.ttf.
            let mut words = LIBERATION[n / 4][n % 4].split(' ');
            let family: String = words.by_ref().take(2).collect();
            let style = match words.collect::<String>() {
                s if s.is_empty() => "Regular".to_string(),
                s => s,
            };
            assert_eq!(*file, format!("{family}-{style}.ttf"));
            stored += packed.len();
            whole += data.len();
        }
        assert!(bundled_face("Helvetica").is_none());
        // About half the size as stored.
        assert!(stored * 10 < whole * 6, "{stored} of {whole} bytes");
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
        let cff = cid.get(b"Subtype").unwrap().as_name().unwrap() == b"CIDFontType0";
        let key: &[u8] = if cff { b"FontFile3" } else { b"FontFile2" };
        let file = desc.get(key).unwrap().as_reference().unwrap();
        let file = p.doc.get_object(file).unwrap().as_stream().unwrap();
        let data = file.get_plain_content().unwrap();
        if cff {
            assert_eq!(
                file.dict.get(b"Subtype").unwrap().as_name().unwrap(),
                b"OpenType"
            );
            assert!(!cid.has(b"CIDToGIDMap"));
        } else {
            assert_eq!(
                file.dict.get(b"Length1").unwrap().as_i64().unwrap(),
                data.len() as i64
            );
        }
        data
    }

    /// A test font as an installed family's face, by the family's name: Source Serif 4
    /// (OpenType CFF), Cantarell (CFF2, variable, weights 100 to 800), Caveat (TrueType,
    /// variable, weights 400 to 700) or [`EMOJI`].
    fn installed(name: &str, bold: bool) -> FontChoice {
        static NUMBERS: OnceLock<Vec<u16>> = OnceLock::new();
        let numbers = NUMBERS.get_or_init(|| {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            let files = [
                dir.join("tests/data/fonts/SourceSerif4-Regular.otf"),
                dir.join("tests/data/fonts/Cantarell-VF.otf"),
                dir.join("assets/fonts/Caveat.ttf"),
                dir.join("tests/data/fonts/LimareEmojiTest.ttf"),
            ];
            crate::add_font_files(&files.iter().map(|p| p.as_path()).collect::<Vec<_>>())
        });
        let n = numbers
            .iter()
            .copied()
            .find(|&n| crate::typeface(n).unwrap().name == name)
            .unwrap();
        choice(Family::Installed(n), bold, false)
    }

    /// The descendant font's subtype and the font file's key of the font object `id`.
    fn kind(p: &Pdf, id: ObjectId) -> (String, String) {
        let font = p.doc.get_dictionary(id).unwrap();
        let cid = font.get(b"DescendantFonts").unwrap().as_array().unwrap()[0]
            .as_reference()
            .unwrap();
        let cid = p.doc.get_dictionary(cid).unwrap();
        let subtype = cid.get(b"Subtype").unwrap().as_name().unwrap();
        let desc = cid.get(b"FontDescriptor").unwrap().as_reference().unwrap();
        let desc = p.doc.get_dictionary(desc).unwrap();
        let file = ["FontFile2", "FontFile3"]
            .into_iter()
            .find(|k| desc.has(k.as_bytes()))
            .unwrap();
        (String::from_utf8_lossy(subtype).into(), file.into())
    }

    #[test]
    fn installed_fonts_are_embedded_read_back_and_drawn() {
        let serif = installed("Source Serif 4", false);
        let cantarell = installed("Cantarell", false);
        let heavy = installed("Cantarell", true);
        let caveat = installed("Caveat", true);
        let lines = [
            (serif, "Zażółć gęślą jaźń"),
            (serif, "Ελληνικά γράμματα"),
            (cantarell, "Příliš žluťoučký kůň"),
            (heavy, "Съешь же ещё этих"),
            (caveat, "Iğdır şehrinde ılık"),
            (serif, "Plain words"),
        ];
        let mut p = blank();
        show(&mut p, &lines);
        // One font per face: the static CFF font as one, the variable ones made TrueType.
        let fonts = own_fonts(&p);
        assert_eq!(fonts.len(), 4);
        let mut kinds: Vec<(String, String)> = fonts.iter().map(|&id| kind(&p, id)).collect();
        kinds.sort();
        let cff = ("CIDFontType0".to_string(), "FontFile3".to_string());
        let truetype = ("CIDFontType2".to_string(), "FontFile2".to_string());
        assert_eq!(kinds, [cff, truetype.clone(), truetype.clone(), truetype]);
        // Text in an installed font is embedded even when it fits WinAnsi.
        let set = p.setter(serif, "Plain words").own();
        assert_eq!(set, crate::layout::SetIn::Face(serif));

        let mut back = reopened(&mut p);
        let text = back.page_text(0);
        for (_, line) in lines {
            assert!(text.contains(line), "{line:?} in {text:?}");
        }
        let h = render::load(back.to_bytes().unwrap(), None).unwrap();
        let r = Renderer::new(&h);
        let b = r.render(0, 2.0).unwrap();
        assert!(r.take_missing().is_empty());
        let inked = b.rgba.chunks(4).filter(|px| px[0] < 128).count();
        assert!(inked > 2000, "the text is drawn ({inked} dark pixels)");
    }

    #[test]
    fn installed_subsets_draw_each_glyph_as_the_face_does() {
        let text = "Zażółć Ελληνικά";
        let serif = installed("Source Serif 4", false);
        let mut p = blank();
        let set = p.embed_text(serif, text).unwrap();
        let face = Face::of(serif).unwrap();
        let full = face.font();
        let data = font_file(&p, set.font);
        let sub = FontRef::new(&data).unwrap();
        for (code, c) in set.bytes.chunks(2).zip(text.nfc()) {
            let cid = u16::from_be_bytes(code.try_into().unwrap()) as u32;
            let g = face.glyph(c).unwrap() as u32;
            assert_eq!(outline(&sub, cid), outline(&full, g), "{c:?}");
        }
        assert!(data.len() < 40_000, "{} bytes", data.len());
    }

    #[test]
    fn variable_fonts_are_set_at_the_weight_chosen() {
        let text = "Zażółć жёлтый";
        for name in ["Cantarell", "Caveat"] {
            let (regular, bold) = (installed(name, false), installed(name, true));
            let mut p = blank();
            let thin = p.embed_text(regular, text).unwrap();
            let heavy = p.embed_text(bold, text).unwrap();
            assert_ne!(thin.font, heavy.font, "{name}: each weight its own font");
            let (thin_face, heavy_face) = (Face::of(regular).unwrap(), Face::of(bold).unwrap());
            assert_ne!(thin_face.source, heavy_face.source);
            assert_eq!(thin_face.weight(), 400.0);
            assert_eq!(heavy_face.weight(), 700.0);

            // The embedded font is the face at that weight: its widths, and its strokes
            // (the subset's outlines are wider where the letters are bolder).
            let mut wider = 0;
            for (set, face) in [(&thin, thin_face), (&heavy, heavy_face)] {
                let data = font_file(&p, set.font);
                let sub = FontRef::new(&data).unwrap();
                assert!(
                    sub.axes().is_empty(),
                    "{name}: set at a weight, not variable"
                );
                let sub_widths = sub.glyph_metrics(Size::unscaled(), LocationRef::default());
                let em = 1000.0
                    / sub
                        .metrics(Size::unscaled(), LocationRef::default())
                        .units_per_em as f32;
                for (code, c) in set.bytes.chunks(2).zip(text.nfc()) {
                    let cid = u16::from_be_bytes(code.try_into().unwrap()) as u32;
                    let w = sub_widths.advance_width(GlyphId::new(cid)).unwrap() * em;
                    let want = face.advance(face.glyph(c).unwrap());
                    assert!((w - want).abs() <= 0.5, "{name} {c:?}: {w} and {want}");
                }
            }
            let ink = |set: &Set| {
                let data = font_file(&p, set.font);
                let sub = FontRef::new(&data).unwrap();
                let first = u16::from_be_bytes(set.bytes[..2].try_into().unwrap()) as u32;
                let bounds = |pen: Pen| {
                    let xs: Vec<f32> = pen.0.iter().map(|(_, a)| a[0]).collect();
                    xs.iter().cloned().fold(f32::MIN, f32::max)
                        - xs.iter().cloned().fold(f32::MAX, f32::min)
                };
                bounds(outline(&sub, first))
            };
            if ink(&heavy) > ink(&thin) {
                wider += 1;
            }
            assert_eq!(wider, 1, "{name}: the bold Z is wider");
        }
    }

    #[test]
    fn installed_fonts_grow_in_place_after_reopening() {
        for name in ["Source Serif 4", "Cantarell"] {
            let f = installed(name, false);
            let mut p = blank();
            let first = p.embed_text(f, "Zażółć").unwrap();
            let page = p.page_ids()[0];
            p.add_resource(page, "Font", "LimareEm", first.font.into())
                .unwrap();
            let mut back = reopened(&mut p);
            let font = own_fonts(&back)[0];
            let more = back.embed_text(f, "Zażółć gęślą").unwrap();
            assert_eq!(more.font, font, "{name}: the font grows");
            assert_eq!(more.bytes[..first.bytes.len()], first.bytes[..], "{name}");
            assert_eq!(own_fonts(&back).len(), 1);
        }
    }

    #[test]
    fn chinese_japanese_and_korean_in_an_installed_font_that_has_them() {
        // Only meaningful where Noto Sans CJK is installed; passes trivially otherwise.
        let Some((n, _)) = crate::typefaces()
            .into_iter()
            .find(|(_, t)| t.name.starts_with("Noto Sans CJK"))
        else {
            return;
        };
        let f = choice(Family::Installed(n), false, false);
        let lines = ["日本語のテキスト", "中文字体 한국어"];
        let mut p = blank();
        show(&mut p, &lines.map(|l| (f, l)));
        let back = reopened(&mut p);
        let text = back.page_text(0);
        for line in lines {
            assert!(text.contains(line), "{line:?} in {text:?}");
        }
        let fonts = own_fonts(&back);
        assert_eq!(fonts.len(), 1);
        assert_eq!(kind(&back, fonts[0]).0, "CIDFontType0");
        // Not in the standard fonts' faces.
        assert!(!Face::liberation(SANS).covers('中'));
    }

    /// A character no font has: a noncharacter (some fonts map unassigned ones).
    const NONE: char = '\u{2FFFE}';

    #[test]
    fn installed_fonts_name_what_they_lack() {
        // Caveat has Latin and Cyrillic, but no Greek: Greek words go to another font.
        let caveat = installed("Caveat", false);
        let face = Face::of(caveat).unwrap();
        assert!(face.covers('ж') && face.covers('A') && !face.covers('Ω'));
        assert!(writable(caveat, 'Ω') && !writable(caveat, '\n'));
        let gone = crate::fallbacks(caveat, "Ωmega and Ψ");
        let words: Vec<&str> = gone.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(words, ["Ωmega", "Ψ"]);
        assert!(gone.iter().all(|(_, f)| *f != caveat));
        // What no font has is refused, with nothing changed.
        let mut p = blank();
        let before = p.doc.objects.len();
        let text = format!("Ωmega {NONE}");
        let err = p.pen(caveat, &[&text]).err().unwrap().to_string();
        assert_eq!(err, format!("No font on this computer has \"{NONE}\""));
        assert_eq!(p.doc.objects.len(), before, "nothing changed");
        assert_eq!(unwritable(caveat, &format!("Ωж{NONE}")), NONE.to_string());
        // Scripts LimarePDF can't write in any font are named so.
        assert_eq!(refusal(caveat, "क"), cant_write("क"));
        let rtl = refusal(caveat, "שΩ");
        assert!(rtl.contains("\"ש\""), "{rtl}");
        assert!(rtl.contains("right-to-left"), "{rtl}");
        for c in ['क', 'ก', 'ᄀ', 'ក', 'ꦄ', '𑀅'] {
            assert!(shaped(c), "{c:?}");
        }
        // Single emoji too: some installed fonts draw them with outlines.
        for c in ['A', 'Ω', 'ж', '中', 'あ', '한', 'ẫ', '😀', '🐋'] {
            assert!(!shaped(c), "{c:?}");
            assert!(installed_could_write(c), "{c:?}");
        }
        for c in ['\n', 'ש', 'ب', 'क', 'ก'] {
            assert!(!installed_could_write(c), "{c:?}");
        }
        // Nor the parts of emoji sequences.
        for c in ['🇬', '🏽', '\u{FE0F}', '\u{20E3}', '\u{E0067}'] {
            assert!(!installed_could_write(c), "{c:?}");
        }
        let whale = "No font on this computer has \"🐋\"";
        assert_eq!(refusal(caveat, "🐋"), whale);

        // A face the family lacks: nothing is written in it, or in a fallback for it.
        let bold_serif = installed("Source Serif 4", true);
        let err = p.pen(bold_serif, &["Bold"]).err().unwrap().to_string();
        assert_eq!(err, "Source Serif 4 has no bold face");
        assert!(!writable(bold_serif, 'A'));
        assert_eq!(p.doc.objects.len(), before);
    }

    /// The made-up font with emoji (`tools/make_emoji_test_font.py`).
    const EMOJI: &str = "Limare Emoji Test";

    /// Dark pixels on the first page of `p`, saved, at 2 pixels per point.
    fn inked(p: &mut Pdf) -> usize {
        let h = render::load(p.to_bytes().unwrap(), None).unwrap();
        let r = Renderer::new(&h);
        let b = r.render(0, 2.0).unwrap();
        assert!(r.take_missing().is_empty());
        b.rgba.chunks(4).filter(|px| px[0] < 128).count()
    }

    #[test]
    fn single_emoji_are_written_in_a_font_that_has_their_outlines() {
        let emoji = installed(EMOJI, false);
        assert!(writable(emoji, '😀') && writable(emoji, 'W'));
        let mut p = blank();
        show(&mut p, &[(emoji, "Walrus 😀")]);
        let mut back = reopened(&mut p);
        let text = back.page_text(0);
        assert!(text.contains("Walrus 😀"), "{text:?}");
        let mut words = blank();
        show(&mut words, &[(emoji, "Walrus")]);
        // The emoji is a box 0.9 em square: about 21 pixels at 12 pt.
        let (with, without) = (inked(&mut back), inked(&mut words));
        assert!(with > without + 300, "the emoji is drawn: {with} {without}");
    }

    #[test]
    fn glyphs_without_an_outline_are_refused() {
        let emoji = installed(EMOJI, false);
        // The font maps 😎 to a glyph drawn only by colour layers: written in a PDF, it
        // would show nothing.
        let face = Face::of(emoji).unwrap();
        assert!(face.font().charmap().map('😎').is_some());
        assert!(!face.covers('😎') && face.covers('😀'));
        // It goes to another installed font with an outline for it, when there is one.
        let gone = crate::fallbacks(emoji, "Cool 😎");
        let elsewhere = gone.iter().all(|(w, f)| w == "😎" && *f != emoji);
        assert!(elsewhere, "{gone:?}");
        // Characters meant to show nothing are written: spaces, and the invisible ones.
        for c in [' ', '\u{2009}', '\u{202F}', '\u{200B}', '\u{AD}'] {
            assert!(writable(SANS, c), "{c:?}");
        }
        assert!(writable(emoji, ' '));
    }

    #[test]
    fn emoji_sequences_are_refused_whole() {
        let emoji = installed(EMOJI, false);
        let mut p = blank();
        let before = p.doc.objects.len();
        // The font has a glyph for every part, but written one by one the parts would
        // say something else: a thumb and a colour swatch, two people, two letters.
        let sequences = [
            "👍🏽",
            "👨\u{200D}👩",
            "🇬🇧",
            "1\u{FE0F}\u{20E3}",
            "😀\u{FE0F}",
        ];
        for s in sequences {
            assert_eq!(unwritable(emoji, &format!("Seal {s} fish")), s, "{s:?}");
            let text = format!("Seal {s}");
            let err = p.pen(emoji, &[&text]).err().unwrap().to_string();
            let want = format!(
                "LimarePDF can't write \"{s}\" yet: flags, skin tones, keycaps and joined emoji \
                 aren't supported"
            );
            assert_eq!(err, want);
            assert_eq!(refusal(emoji, s), want);
            assert_eq!(cant_write(s), want);
        }
        assert_eq!(p.doc.objects.len(), before, "nothing changed");
        // The parts that only make sequences can't be written alone either; the others can.
        for c in ['🏽', '🇬', '\u{20E3}', '\u{FE0F}'] {
            assert!(!writable(emoji, c), "{c:?}");
        }
        assert!(writable(emoji, '👍') && writable(emoji, '👨'));
        // A joiner outside an emoji is only an invisible mark, written as it is.
        assert!(unwritable(SANS, "a\u{200D}b").is_empty());
        // In a standard font, named whole too, beside what else no font has.
        assert_eq!(
            unwritable(SANS, &format!("a 👍🏽 b {NONE}")),
            format!("👍🏽{NONE}")
        );
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
            // As the face has it, to a thousandth of a unit.
            let want = face.advance(face.glyph(c).unwrap());
            assert!((ctx.width(code, 2) - want).abs() < 1e-3, "{c:?}");
        }
    }

    #[test]
    fn the_faces_have_the_standard_fonts_widths() {
        let ascii: String = (' '..='~').collect();
        for (f, ps) in every_face() {
            let face = Face::liberation(f);
            for c in ascii.chars() {
                let want = crate::std14::width(f.base_font(), c).unwrap();
                let got = face.width_of(face.glyph(c).unwrap(), false);
                assert_eq!(got, want, "{ps} {c:?}");
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
        let setter = crate::layout::Setter::new(SANS, &text, None);
        assert!(setter.width(&text, 10.0) > 0.0);
        assert!(setter.missing(&text).is_empty());
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

        // Ours, with a record that doesn't match the face: a glyph it hasn't.
        let mut p = blank();
        let first = made(&mut p);
        let own = p
            .doc
            .get_dictionary_mut(first)
            .unwrap()
            .get_mut(KEY)
            .unwrap();
        let own = own.as_dict_mut().unwrap();
        let mut texts = own.get(b"Texts").unwrap().as_array().unwrap().clone();
        texts[0] = Object::Integer(65_000);
        own.set("Texts", texts);
        assert_ne!(made(&mut p), first);

        // Ours, with a glyph and its text listed twice.
        let mut p = blank();
        let first = made(&mut p);
        let own = p
            .doc
            .get_dictionary_mut(first)
            .unwrap()
            .get_mut(KEY)
            .unwrap();
        let own = own.as_dict_mut().unwrap();
        let mut texts = own.get(b"Texts").unwrap().as_array().unwrap().clone();
        texts.extend(texts[..2].to_vec());
        own.set("Texts", texts);
        assert_ne!(made(&mut p), first);
    }

    #[test]
    fn the_text_map_spells_characters_beyond_the_first_plane() {
        let texts = |list: &[(u16, &str)]| -> Vec<(u16, String)> {
            list.iter().map(|&(g, t)| (g, t.to_string())).collect()
        };
        let map = to_unicode(&texts(&[(5, "A"), (9, "𝐀"), (3, "ж"), (7, "fi"), (8, "")]));
        assert!(map.contains("<0002> <D835DC00>"), "{map}");
        // A ligature maps to its letters; a glyph written for no text has no entry.
        assert!(map.contains("<0004> <00660069>"), "{map}");
        assert!(!map.contains("<0005>"), "{map}");
        let read = crate::tounicode::ToUnicode::parse(map.as_bytes()).unwrap();
        assert_eq!(read.get(1), Some("A"));
        assert_eq!(read.get(2), Some("𝐀"));
        assert_eq!(read.get(3), Some("ж"));
        assert_eq!(read.get(4), Some("fi"));
        // More than 100 glyphs: in blocks of 100, as the CMap language limits them.
        let many: Vec<(u16, String)> = ('a'..='z')
            .cycle()
            .take(250)
            .map(|c| (1, c.to_string()))
            .collect();
        let map = to_unicode(&many);
        assert_eq!(map.matches("100 beginbfchar").count(), 2);
        assert!(map.contains("50 beginbfchar"));
        let read = crate::tounicode::ToUnicode::parse(map.as_bytes()).unwrap();
        assert_eq!(read.get(250), Some("p"));
    }

    /// Runs an installed tool on `bytes`, for the opt-in cross-checks; `{}` in `args` is the
    /// file. Its standard output and error.
    fn tool(name: &str, args: &[&str], bytes: &[u8]) -> (String, String) {
        // A file of its own for each run, as the cross-checks run side by side.
        static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("limarepdf-embed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}-{run}.pdf"));
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

    /// The same for installed fonts: the OpenType CFF one listed as a CID Type 0C font, the
    /// variable ones set at a weight as CID TrueType fonts:
    /// `cargo test -p limarepdf-core embed -- --ignored cross_check`.
    #[test]
    #[ignore]
    fn cross_check_installed_fonts_with_poppler_and_ghostscript() {
        let lines = [
            (installed("Source Serif 4", false), "Zażółć gęślą jaźń"),
            (installed("Source Serif 4", false), "Ελληνικά γράμματα"),
            (installed("Cantarell", false), "Příliš žluťoučký kůň"),
            (installed("Cantarell", true), "Съешь же ещё этих"),
            (installed("Caveat", true), "Iğdır şehrinde ılık"),
        ];
        let mut p = blank();
        show(&mut p, &lines);
        let bytes = p.to_bytes().unwrap();
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
        for (_, w) in lines {
            assert!(text.contains(w), "poppler: {w:?} in {text:?}");
            assert!(gs_text.contains(w), "Ghostscript: {w:?} in {gs_text:?}");
        }
        assert!(gs_errors.trim().is_empty(), "Ghostscript: {gs_errors}");

        let (fonts, errors) = tool("pdffonts", &["{}"], &bytes);
        assert!(errors.trim().is_empty(), "pdffonts: {errors}");
        let rows: Vec<&str> = fonts.lines().skip(2).collect();
        assert_eq!(rows.len(), 4, "{fonts}");
        for row in &rows {
            let kind = if row.contains("SourceSerif4") {
                "CID Type 0C"
            } else {
                "CID TrueType"
            };
            assert!(row.contains(kind) && row.contains("Identity-H"), "{row}");
            let flags: Vec<&str> = row.split_whitespace().rev().skip(2).take(3).collect();
            assert_eq!(flags, ["yes", "yes", "yes"], "{row}");
        }

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
