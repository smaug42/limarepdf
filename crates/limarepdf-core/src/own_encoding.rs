//! The text of a simple font's codes where the PDF's `/Encoding` is a dictionary or leaves the
//! base to the font program (PB-23). An embedded program has an encoding of its own, which
//! the standard makes the base when the PDF names none: TeX's fonts put ” at 0x22 and – at
//! 0x7B, where the standard encoding has " and {. Glyph names give the text, by the Adobe
//! Glyph List (`quotedblright`), `uniXXXX` or `uXXXX` names, and their parts joined by `_`.
//! lopdf reads the other encodings. Its own reading of an `/Encoding` dictionary passes over
//! one without its optional `/Type`, and over all its `/Differences` when one name isn't in
//! its glyph list (`g12`, `uni2019`); this reads both.

use lopdf::{dictionary, Dictionary, Document, Encoding, Object};
use skrifa::raw::ps::cff::CffFontRef;
use skrifa::raw::ps::type1::Type1Font;
use skrifa::raw::types::Tag;
use skrifa::raw::{FontRef, TableProvider};
use skrifa::{GlyphId, MetadataProvider};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock, Mutex};

/// The text of each one-byte code; `None` for a code with none.
pub(crate) type Codes = Vec<Option<String>>;

/// Largest font program read for its encoding.
const MAX_PROGRAM: usize = 32 << 20;
/// Most programs, and glyph names, whose text is kept between pages.
const KEPT: usize = 256;
const KEPT_NAMES: usize = 1 << 16;

/// The text of each code of simple font `font`, when its `/Encoding` is a dictionary, or is
/// missing and the font has a program with an encoding of its own (symbolic or without a
/// Unicode cmap, for TrueType). `None` leaves the encoding to lopdf: a named one, or none for
/// a font without a program.
pub(crate) fn simple_codes(doc: &Document, font: &Dictionary) -> Option<Codes> {
    let deref = |o| deref(doc, o);
    let encoding = font.get(b"Encoding").ok().and_then(deref);
    let dict = match encoding {
        Some(Object::Dictionary(d)) => Some(d),
        None => None,
        // A name, or something that isn't an encoding: lopdf's.
        Some(_) => return None,
    };
    let base = dict
        .and_then(|d| d.get(b"BaseEncoding").ok())
        .and_then(deref)
        .and_then(|o| o.as_name().ok())
        .and_then(named);
    let mut codes: Codes = match base {
        Some(table) => table,
        None => match own_codes(doc, font) {
            Some(codes) => codes.to_vec(),
            None if dict.is_some() => named(b"StandardEncoding")?,
            None => return None,
        },
    };
    let differences = dict
        .and_then(|d| d.get(b"Differences").ok())
        .and_then(deref)
        .and_then(|o| o.as_array().ok());
    let mut code = 0usize;
    for o in differences.into_iter().flatten() {
        match deref(o) {
            Some(Object::Integer(n)) => code = usize::try_from(*n).unwrap_or(256),
            Some(Object::Name(n)) => {
                if let Some(slot) = codes.get_mut(code) {
                    *slot = std::str::from_utf8(n).ok().and_then(glyph_text);
                }
                code += 1;
            }
            _ => {}
        }
    }
    Some(codes)
}

fn deref<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Object> {
    doc.dereference(o).ok().map(|(_, o)| o)
}

/// The text of each code of a base encoding, by name, as lopdf reads it (its tables are its
/// own).
fn named(name: &[u8]) -> Option<Codes> {
    if !matches!(
        name,
        b"WinAnsiEncoding" | b"MacRomanEncoding" | b"MacExpertEncoding" | b"StandardEncoding"
    ) {
        return None;
    }
    let doc = Document::new();
    let font = dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "Encoding" => Object::Name(name.to_vec()),
    };
    let enc = font.get_font_encoding(&doc).ok()?;
    Some((0..=255u8).map(|c| text_of(&enc, c)).collect())
}

/// What `enc` reads code `c` as, if anything.
fn text_of(enc: &Encoding, c: u8) -> Option<String> {
    enc.bytes_to_string(&[c]).ok().filter(|s| !s.is_empty())
}

/// The character the Adobe Glyph List gives glyph name `name`, as lopdf has the list (its own):
/// a font whose one difference is that name reads code 0 as it, and lopdf reads a font with a
/// name it doesn't know through the standard encoding instead.
fn listed(name: &str) -> Option<String> {
    let doc = Document::new();
    let enc = dictionary! {
        "Type" => "Encoding", "Differences" => vec![0.into(), Object::Name(name.into())],
    };
    let font = dictionary! { "Type" => "Font", "Subtype" => "Type1", "Encoding" => enc };
    match font.get_font_encoding(&doc).ok()? {
        e @ Encoding::Differences(_) => text_of(&e, 0),
        _ => None,
    }
}

/// The text a glyph name stands for: by the Adobe Glyph List, `uniXXXX` (several characters
/// in a row allowed) or `uXXXX` to `uXXXXXX`, with what follows a `.` left out (`a.sc`) and
/// parts joined by `_` (`f_f_i`) each read. `None` for `.notdef` and names none of these read.
pub(crate) fn glyph_text(name: &str) -> Option<String> {
    static KEEP: LazyLock<Mutex<HashMap<Box<str>, Option<String>>>> =
        LazyLock::new(Default::default);
    if let Some(t) = KEEP.lock().unwrap_or_else(|e| e.into_inner()).get(name) {
        return t.clone();
    }
    let text = read_name(name);
    let mut keep = KEEP.lock().unwrap_or_else(|e| e.into_inner());
    if keep.len() >= KEPT_NAMES {
        keep.clear();
    }
    keep.insert(name.into(), text.clone());
    text
}

/// [`glyph_text`], read.
fn read_name(name: &str) -> Option<String> {
    let name = match name.find('.') {
        Some(0) | None => name,
        Some(i) => &name[..i],
    };
    if name.is_empty() || name == ".notdef" {
        return None;
    }
    let mut out = String::new();
    for part in name.split('_') {
        let hex = |s: &str| {
            let v = u32::from_str_radix(s, 16).ok()?;
            char::from_u32(v)
        };
        if let Some(t) = listed(part) {
            out.push_str(&t);
        } else if let Some(h) = part.strip_prefix("uni").filter(|h| {
            !h.is_empty() && h.len() % 4 == 0 && h.bytes().all(|b| b.is_ascii_hexdigit())
        }) {
            for i in (0..h.len()).step_by(4) {
                out.push(hex(&h[i..i + 4])?);
            }
        } else {
            let h = part.strip_prefix('u').filter(|h| {
                (4..=6).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_hexdigit())
            })?;
            out.push(hex(h)?);
        }
    }
    Some(out)
}

/// What kind of program a font descriptor's key holds.
#[derive(Clone, Copy, Hash)]
enum Kind {
    Type1,
    Cff,
    /// TrueType or OpenType. `symbolic` from the descriptor's flags.
    Sfnt {
        symbolic: bool,
    },
}

/// The text of each code in the encoding of `font`'s own program, by its glyph names; kept
/// between calls for the same program.
fn own_codes(doc: &Document, font: &Dictionary) -> Option<Arc<Codes>> {
    static KEEP: LazyLock<Mutex<HashMap<u64, Option<Arc<Codes>>>>> =
        LazyLock::new(Default::default);
    let deref = |o| deref(doc, o);
    let desc = font
        .get(b"FontDescriptor")
        .ok()
        .and_then(deref)
        .and_then(|o| o.as_dict().ok())?;
    let symbolic = desc
        .get(b"Flags")
        .ok()
        .and_then(deref)
        .and_then(|o| o.as_i64().ok())
        .is_some_and(|f| f & 4 != 0);
    let (kind, stream) = [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
        .into_iter()
        .find_map(|k| {
            let s = desc.get(k).ok().and_then(deref)?.as_stream().ok()?;
            let kind = match k {
                b"FontFile" => Kind::Type1,
                b"FontFile2" => Kind::Sfnt { symbolic },
                _ => match s.dict.get(b"Subtype").and_then(|o| o.as_name()) {
                    Ok(b"Type1C") => Kind::Cff,
                    Ok(b"OpenType") => Kind::Sfnt { symbolic },
                    _ => return None,
                },
            };
            Some((kind, s))
        })?;
    let mut h = DefaultHasher::new();
    (kind, &stream.content).hash(&mut h);
    let key = h.finish();
    if let Some(codes) = KEEP.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return codes.clone();
    }
    let data = stream.get_plain_content_with_limit(MAX_PROGRAM).ok();
    let codes = data.and_then(|d| program_names(kind, &d)).map(|names| {
        let text = |n: &Option<Box<str>>| n.as_deref().and_then(glyph_text);
        Arc::new(names.iter().map(text).collect::<Codes>())
    });
    let mut keep = KEEP.lock().unwrap_or_else(|e| e.into_inner());
    if keep.len() >= KEPT {
        keep.clear();
    }
    keep.insert(key, codes.clone());
    codes
}

/// The glyph name of each code in program `data`'s own encoding.
fn program_names(kind: Kind, data: &[u8]) -> Option<Vec<Option<Box<str>>>> {
    let name = |s: &str| Some(Box::<str>::from(s));
    match kind {
        Kind::Type1 => {
            let font = Type1Font::new(data).ok()?;
            let enc = font.encoding()?;
            Some(
                (0..=255)
                    .map(|c| enc.glyph_name(c).and_then(name))
                    .collect(),
            )
        }
        Kind::Cff => cff_names(data),
        Kind::Sfnt { symbolic } => {
            let font = FontRef::new(data).ok()?;
            if let Some(cff) = font.table_data(Tag::new(b"CFF ")) {
                return cff_names(cff.as_bytes());
            }
            truetype_names(&font, symbolic)
        }
    }
}

/// [`program_names`] of a CFF program: each code's glyph, by the program's encoding, and
/// that glyph's name, by its charset. CID-keyed programs have no encoding.
fn cff_names(data: &[u8]) -> Option<Vec<Option<Box<str>>>> {
    let font = CffFontRef::new_cff(data, 0, None).ok()?;
    if font.is_cid() {
        return None;
    }
    let enc = font.encoding()?;
    let charset = enc.charset().clone();
    let name = |c: u8| {
        let sid = charset.string_id(enc.map(c)?).ok()?;
        let s = std::str::from_utf8(font.string(sid)?).ok()?;
        Some(Box::<str>::from(s))
    };
    Some((0..=255).map(name).collect())
}

/// [`program_names`] of a TrueType program that is symbolic or has no Unicode cmap: each
/// code's glyph by its (3,0) cmap (the code, or the code at 0xF000, 0xF100 or 0xF200, as
/// the standard and other readers look) or else its (1,0) cmap, and that glyph's name in
/// `post`. Programs with a Unicode cmap that aren't symbolic are drawn through the PDF's
/// encoding: `None`.
fn truetype_names(font: &FontRef, symbolic: bool) -> Option<Vec<Option<Box<str>>>> {
    let cmap = font.cmap().ok()?;
    let sub = |platform: u16, encoding: u16| {
        let rec = cmap
            .encoding_records()
            .iter()
            .find(|r| r.platform_id() as u16 == platform && r.encoding_id() == encoding)?;
        rec.subtable(cmap.offset_data()).ok()
    };
    if !symbolic && sub(3, 1).is_some() {
        return None;
    }
    let (ms, mac) = (sub(3, 0), sub(1, 0));
    if ms.is_none() && mac.is_none() {
        return None;
    }
    let names = font.glyph_names();
    let glyph = |c: u32| -> Option<GlyphId> {
        if let Some(t) = &ms {
            for base in [0, 0xF000, 0xF100, 0xF200] {
                if let Some(g) = t.map_codepoint(base + c).filter(|g| g.to_u32() != 0) {
                    return Some(g);
                }
            }
        }
        mac.as_ref()?.map_codepoint(c).filter(|g| g.to_u32() != 0)
    };
    let name = |c: u32| {
        let n = names.get(glyph(c)?)?;
        (!n.is_synthesized()).then(|| Box::<str>::from(n.as_str()))
    };
    Some((0..=255).map(name).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_names_give_their_text() {
        let t = |n: &str| glyph_text(n);
        assert_eq!(t("quotedblright").as_deref(), Some("”"));
        assert_eq!(t("A").as_deref(), Some("A"));
        assert_eq!(t("a.sc").as_deref(), Some("a"));
        assert_eq!(t("f_f_i").as_deref(), Some("ffi"));
        assert_eq!(t("uni20AC").as_deref(), Some("€"));
        assert_eq!(t("uni00410042").as_deref(), Some("AB"));
        assert_eq!(t("u1F600").as_deref(), Some("😀"));
        // Surrogates, odd lengths and names no list has read as nothing.
        for none in [".notdef", "uniD800", "uni004", "u12", "g12", "", "A_g12"] {
            assert_eq!(t(none), None, "{none}");
        }
    }
}
