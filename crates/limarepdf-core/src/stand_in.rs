//! What LimarePDF's pages draw a font in that the PDF names but doesn't hold (ENH-25), so
//! that text typed in such a font shows in the face it will have once kept.
//!
//! The renderer is hayro 0.8's, and this follows its choice (its `font/standard_font.rs`,
//! `type1.rs`, `true_type.rs` and `cid.rs`). A font is drawn in the standard font it is by
//! name; else in one of the family a word in its name gives ("helvetica", then "arial" or
//! "sans", "courier", "mono", "times", "serif", in that order), bold where its descriptor's
//! weight is 700 or more or the name says "bold" or "demi", italic where its italic angle
//! isn't 0 or the name says "italic" or "oblique". A simple font without such a word is
//! drawn in Times where its descriptor's flags say serif, else Helvetica, bold where its
//! PostScript name says "Bold" and italic where the flags or the name ("Italic") say so (its
//! fixed-pitch flag and weight aren't read). A CID font without such a word goes to an
//! installed font (`render.rs`, [`InstalledFonts::find`]), else to the standard font by
//! the same rule, with no descriptor read (hayro looks for one on the Type 0 font itself).

use crate::fonts::{Family, FontChoice};
use crate::installed_fonts::{Found, InstalledFonts};
use hayro::hayro_interpret::font::FallbackFontQuery;
use hayro::hayro_interpret::hayro_cmap::{CharacterCollection, CidFamily};
use lopdf::{Dictionary, Document, Object};
use std::sync::Arc;

/// What a font the PDF doesn't hold is drawn in ([`crate::LineFont::stand_in`]).
#[derive(Clone, Debug, PartialEq)]
pub enum StandIn {
    /// One of the standard fonts in Helvetica, Times or Courier (drawn in its Liberation
    /// face while typing).
    Standard(FontChoice),
    /// A CID font: the installed font found for it, else the standard font.
    Installed(InstalledQuery, FontChoice),
}

/// A question for an installed font, as the renderer asks it for a CID font it doesn't hold.
#[derive(Clone, Debug)]
pub struct InstalledQuery(Arc<FallbackFontQuery>);

impl InstalledQuery {
    /// What tells this question from another: the names and collection it asks for.
    pub fn key(&self) -> String {
        let q = &self.0;
        let collection = q.character_collection.as_ref().map(|c| &c.family);
        format!(
            "{:?} {:?} {:?} {} {}",
            q.post_script_name, q.font_name, collection, q.is_bold, q.is_italic
        )
    }

    /// The installed font in `fonts` that answers it, as the renderer finds it: a stand-in
    /// too.
    pub fn find(&self, fonts: &InstalledFonts) -> Option<Found> {
        fonts.find(&self.0)
    }
}

impl PartialEq for InstalledQuery {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

/// What the font `dict` is drawn in when the PDF doesn't hold its program; `None` when it
/// does, when it is a Type 3 font, and when it is drawn in Symbol or ZapfDingbats.
pub(crate) fn stand_in(doc: &Document, dict: &Dictionary) -> Option<StandIn> {
    let empty = Dictionary::new();
    match name(doc, dict, b"Subtype")? {
        b"Type1" | b"MMType1" | b"TrueType" | b"OpenType" => {
            let desc = sub_dict(doc, dict, b"FontDescriptor").unwrap_or(&empty);
            if embedded(doc, desc) {
                return None;
            }
            match by_name(doc, dict, desc) {
                Some(named) => named.map(StandIn::Standard),
                None => Some(StandIn::Standard(by_flags(&query(doc, dict)))),
            }
        }
        b"Type0" => {
            let first = (dict.get(b"DescendantFonts").ok())
                .and_then(|o| doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_array().ok())
                .and_then(|a| a.first())
                .and_then(|o| doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_dict().ok())?;
            let desc = sub_dict(doc, first, b"FontDescriptor").unwrap_or(&empty);
            if embedded(doc, desc) {
                return None;
            }
            if let Some(named) = by_name(doc, dict, desc) {
                return named.map(StandIn::Standard);
            }
            let mut q = query(doc, dict);
            q.character_collection = collection(doc, dict, first);
            let standard = by_flags(&q);
            Some(StandIn::Installed(InstalledQuery(Arc::new(q)), standard))
        }
        _ => None,
    }
}

/// The font's program is in the PDF, where the renderer reads one.
fn embedded(doc: &Document, desc: &Dictionary) -> bool {
    [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
        .iter()
        .any(|k| {
            let o = desc.get(k).ok().and_then(|o| doc.dereference(o).ok());
            o.is_some_and(|(_, o)| o.as_stream().is_ok())
        })
}

/// The standard font a font is drawn in by its name: the one it is, else one of the family
/// a word in it gives. `Some(None)` for Symbol and ZapfDingbats, which the bundled faces
/// don't draw.
fn by_name(doc: &Document, dict: &Dictionary, desc: &Dictionary) -> Option<Option<FontChoice>> {
    let base = name(doc, dict, b"BaseFont")?;
    let name = without_subset(std::str::from_utf8(base).ok()?);
    let choice = |family, bold, italic| {
        Some(Some(FontChoice {
            family,
            bold,
            italic,
        }))
    };
    use Family::{Courier, Helvetica, Times};
    match name {
        "Helvetica" => return choice(Helvetica, false, false),
        "Helvetica-Bold" => return choice(Helvetica, true, false),
        "Helvetica-Oblique" => return choice(Helvetica, false, true),
        "Helvetica-BoldOblique" => return choice(Helvetica, true, true),
        "Courier" => return choice(Courier, false, false),
        "Courier-Bold" => return choice(Courier, true, false),
        "Courier-Oblique" => return choice(Courier, false, true),
        "Courier-BoldOblique" => return choice(Courier, true, true),
        "Times-Roman" => return choice(Times, false, false),
        "Times-Bold" => return choice(Times, true, false),
        "Times-Italic" => return choice(Times, false, true),
        "Times-BoldItalic" => return choice(Times, true, true),
        "Symbol" | "ZapfDingbats" => return Some(None),
        _ => {}
    }
    let lower = name.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    let weight = whole(number(doc, desc, b"FontWeight"));
    let bold = weight.is_some_and(|w| w >= 700) || has(&["bold", "demi"]);
    let angle = number(doc, desc, b"ItalicAngle");
    let italic = angle.is_some_and(|a| a != 0.0) || has(&["italic", "oblique"]);
    let family = if has(&["helvetica", "arial", "sans"]) {
        Helvetica
    } else if has(&["courier", "mono"]) {
        Courier
    } else if has(&["times", "serif"]) {
        Times
    } else if has(&["dingbats"]) {
        return Some(None);
    } else {
        return None;
    };
    choice(family, bold, italic)
}

/// The standard font a font without a family word in its name is drawn in: by the serif
/// flag, bold and italic as the question has them.
fn by_flags(q: &FallbackFontQuery) -> FontChoice {
    FontChoice {
        family: if q.is_serif {
            Family::Times
        } else {
            Family::Helvetica
        },
        bold: q.is_bold,
        italic: q.is_italic,
    }
}

/// The renderer's question about the font `dict`, read as it reads it: the names without a
/// subset's tag, the family, weight and flags from a descriptor of the font's own (which a
/// Type 0 font hasn't), bold and italic from "Bold" and "Italic" in the PostScript name too.
fn query(doc: &Document, dict: &Dictionary) -> FallbackFontQuery {
    let text = |d: &Dictionary, key: &[u8]| {
        let n = name(doc, d, key)?;
        Some(without_subset(std::str::from_utf8(n).ok()?).to_string())
    };
    let mut q = FallbackFontQuery {
        post_script_name: text(dict, b"BaseFont"),
        ..Default::default()
    };
    if let Some(desc) = sub_dict(doc, dict, b"FontDescriptor") {
        q.font_name = text(dict, b"FontName");
        q.font_family = name(doc, desc, b"FontFamily")
            .and_then(|n| std::str::from_utf8(n).ok())
            .map(str::to_string);
        q.font_weight = whole(number(doc, desc, b"FontWeight")).unwrap_or(400);
        if let Some(flags) = whole(number(doc, desc, b"Flags")) {
            q.is_serif = flags & (1 << 1) != 0;
            q.is_italic = flags & (1 << 6) != 0;
            q.is_small_cap = flags & (1 << 17) != 0;
        }
    }
    let ps = q.post_script_name.as_deref().unwrap_or("");
    q.is_italic |= ps.contains("Italic");
    q.is_bold |= ps.contains("Bold");
    q
}

/// A CID font's character collection, as the renderer reads it: its CMap's, unless that is
/// Identity, else its descendant's `/CIDSystemInfo`.
fn collection(
    doc: &Document,
    dict: &Dictionary,
    first: &Dictionary,
) -> Option<CharacterCollection> {
    let cmap = name(doc, dict, b"Encoding").and_then(crate::fonts::predefined_collection);
    let cmap = cmap.filter(|c| c.family != CidFamily::AdobeIdentity);
    cmap.or_else(|| {
        let (_, info) = doc.dereference(first.get(b"CIDSystemInfo").ok()?).ok()?;
        let info = info.as_dict().ok()?;
        let s = |k: &[u8]| {
            let (_, o) = doc.dereference(info.get(k).ok()?).ok()?;
            o.as_str().ok().map(<[u8]>::to_vec)
        };
        let supplement = (info.get(b"Supplement").ok())
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_i64().ok())
            .unwrap_or(0);
        Some(CharacterCollection {
            family: CidFamily::from_registry_ordering(&s(b"Registry")?, &s(b"Ordering")?),
            supplement: supplement as i32,
        })
    })
}

/// A font's name without a subset's tag ("ABCDEF+Arial" is "Arial").
fn without_subset(name: &str) -> &str {
    match name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 => rest,
        _ => name,
    }
}

fn sub_dict<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a Dictionary> {
    let (_, o) = doc.dereference(d.get(key).ok()?).ok()?;
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn name<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
    let (_, o) = doc.dereference(d.get(key).ok()?).ok()?;
    o.as_name().ok()
}

fn number(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<f32> {
    doc.dereference(d.get(key).ok()?).ok()?.1.as_float().ok()
}

/// A number read as the renderer reads a whole one it wants: cut to its whole part, and
/// none when that is below zero.
fn whole(n: Option<f32>) -> Option<u32> {
    n.map(f32::trunc).filter(|n| *n >= 0.0).map(|n| n as u32)
}
