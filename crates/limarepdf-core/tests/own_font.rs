//! Which code writes a character in the font a line of text is already in (ENH-31, stage 1):
//! a code that reads as the character, whose glyph the embedded program has and draws, whose
//! width the PDF gives as the program does, and that neither the encoding nor the program's
//! own names contradict. Fonts that can't take new text say why, and so does each character
//! that can't be written. A Type 0 TrueType font with no ToUnicode map reads through its
//! program's cmap (PB-26).
//!
//! An edited line written in that font (stage 2): changed words through those codes, the
//! letters left as they were with their own codes and the spacing numbers between them, a
//! word with a character the font can't write sent alone to another font, a line in one part
//! rewritten in place as one `TJ`, and a line that wouldn't read back as its words written
//! again in another font. Opt-in: `cargo test -p limarepdf-core --test own_font -- --ignored
//! cross_check` has Poppler read and draw edited lines.

use limarepdf_core::CharRefusal::{Ambiguous, Contradicted, Encoding, NoCode, NoGlyph, Width};
use limarepdf_core::{
    bundled_face, CharRefusal, FontChoice, FontRefusal, LineFont, ObjectKind, PageObject, PartFont,
    Pdf, TextEdit, TextPart,
};
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};
use std::fmt::Write;

const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/fonts/");

/// A test font's file, from tests/data/fonts.
fn font_file(name: &str) -> Vec<u8> {
    std::fs::read(format!("{FONTS}{name}")).unwrap()
}

/// Liberation Sans Regular, as core bundles it (TrueType, 2048 units to the em).
fn sans() -> &'static [u8] {
    bundled_face("Liberation Sans").unwrap()
}

/// Source Serif 4 Regular (OpenType with a CFF program, named glyphs, 1000 units to the em).
fn serif() -> Vec<u8> {
    font_file("SourceSerif4-Regular.otf")
}

/// The glyph `program`'s Unicode cmap gives `c`.
fn glyph(program: &[u8], c: char) -> u16 {
    let f = FontRef::new(program).unwrap();
    let g = f
        .charmap()
        .map(c)
        .unwrap_or_else(|| panic!("no glyph for {c:?}"));
    g.to_u32() as u16
}

/// The glyph named `name` in `program`.
fn named_glyph(program: &[u8], name: &str) -> u16 {
    let f = FontRef::new(program).unwrap();
    let found = f.glyph_names().iter().find(|(_, n)| n.as_str() == name);
    let (g, _) = found.unwrap_or_else(|| panic!("no glyph named {name}"));
    g.to_u32() as u16
}

/// Glyph `g`'s advance in `program`, in thousandths of an em, as PDF widths are given.
fn advance(program: &[u8], g: u16) -> i64 {
    let f = FontRef::new(program).unwrap();
    let upem = f
        .metrics(Size::unscaled(), LocationRef::default())
        .units_per_em as f32;
    let w = f
        .glyph_metrics(Size::unscaled(), LocationRef::default())
        .advance_width(GlyphId::new(g.into()))
        .unwrap();
    (w * 1000.0 / upem).round() as i64
}

/// The character the base encoding `name` gives `code`, as lopdf (and so LimarePDF) reads it.
fn base_char(name: &str, code: u8) -> Option<char> {
    let doc = Document::new();
    let font = dictionary! { "Type" => "Font", "Subtype" => "Type1", "Encoding" => name };
    let s = font
        .get_font_encoding(&doc)
        .ok()?
        .bytes_to_string(&[code])
        .ok()?;
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// The widths of codes `first..=last` read by base encoding `encoding`, from `program`'s
/// glyph for each character by its Unicode cmap; 0 where it has none.
fn widths_by_cmap(program: &[u8], encoding: &str, first: u8, last: u8) -> Vec<i64> {
    let f = FontRef::new(program).unwrap();
    let cmap = f.charmap();
    (first..=last)
        .map(|code| {
            let g = base_char(encoding, code).and_then(|c| cmap.map(c));
            g.map_or(0, |g| advance(program, g.to_u32() as u16))
        })
        .collect()
}

/// Two-byte codes as a hex string's digits.
fn hex2(codes: &[u16]) -> String {
    codes.iter().map(|c| format!("{c:04X}")).collect()
}

/// A code of a Type 0 font as the bytes of a show string.
fn two(code: u16) -> Vec<u8> {
    code.to_be_bytes().to_vec()
}

/// A ToUnicode map from codes of `bytes` bytes each to their text.
fn to_unicode(bytes: usize, map: &[(u32, String)]) -> Stream {
    let (lo, hi) = if bytes == 1 {
        ("00", "FF")
    } else {
        ("0000", "FFFF")
    };
    let mut s = format!(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<{lo}> <{hi}>\nendcodespacerange\n"
    );
    for chunk in map.chunks(100) {
        let _ = writeln!(s, "{} beginbfchar", chunk.len());
        for (code, text) in chunk {
            let _ = write!(s, "<{code:0w$X}> <", w = 2 * bytes);
            for u in text.encode_utf16() {
                let _ = write!(s, "{u:04X}");
            }
            s.push_str(">\n");
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    Stream::new(Dictionary::new(), s.into_bytes())
}

/// A font a fixture adds to a document.
trait AddFont {
    /// Adds the font and the objects it refers to, giving the font's id.
    fn add(&self, doc: &mut Document) -> ObjectId;
}

/// A font descriptor for `name` with `flags` (32 for Latin text, 4 for symbolic), holding
/// `program` (if any) under its key, with the program's own dictionary.
fn descriptor(
    doc: &mut Document,
    name: &str,
    flags: i64,
    program: &Option<(&'static str, Dictionary, Vec<u8>)>,
) -> ObjectId {
    let mut d = dictionary! {
        "Type" => "FontDescriptor", "FontName" => name, "Flags" => flags,
        "FontBBox" => vec![0.into(), (-200).into(), 1000.into(), 900.into()],
        "ItalicAngle" => 0, "Ascent" => 900, "Descent" => -200, "CapHeight" => 700,
        "StemV" => 80,
    };
    if let Some((key, dict, data)) = program {
        let file = doc.add_object(Stream::new(dict.clone(), data.clone()));
        d.set(*key, file);
    }
    doc.add_object(d)
}

/// A simple font (one byte a code).
#[derive(Clone)]
struct Simple {
    /// Its `/Subtype`: Type1 or TrueType.
    subtype: &'static str,
    base: &'static str,
    /// Its program, under this key of its descriptor, with the program's dictionary. Without
    /// one the font has no descriptor.
    program: Option<(&'static str, Dictionary, Vec<u8>)>,
    flags: i64,
    encoding: Option<Object>,
    /// `/FirstChar`, and the widths from it; none when empty.
    first: i64,
    widths: Vec<i64>,
    /// The ToUnicode map's entries; no map when empty.
    to_unicode: Vec<(u32, String)>,
}

impl AddFont for Simple {
    fn add(&self, doc: &mut Document) -> ObjectId {
        let mut font = dictionary! {
            "Type" => "Font", "Subtype" => self.subtype, "BaseFont" => self.base,
        };
        if self.program.is_some() {
            let d = descriptor(doc, self.base, self.flags, &self.program);
            font.set("FontDescriptor", d);
        }
        if let Some(e) = &self.encoding {
            font.set("Encoding", e.clone());
        }
        if !self.widths.is_empty() {
            font.set("FirstChar", self.first);
            font.set("LastChar", self.first + self.widths.len() as i64 - 1);
            let widths: Vec<Object> = self.widths.iter().map(|&w| Object::Integer(w)).collect();
            font.set("Widths", widths);
        }
        if !self.to_unicode.is_empty() {
            let map = doc.add_object(to_unicode(1, &self.to_unicode));
            font.set("ToUnicode", map);
        }
        doc.add_object(font)
    }
}

/// How a CIDFont finds a CID's glyph.
#[derive(Clone)]
enum CidToGid {
    Identity,
    /// The glyph of each CID from 0.
    Map(Vec<u16>),
}

/// A Type 0 font (a CIDFont under a CMap).
#[derive(Clone)]
struct Type0 {
    base: &'static str,
    /// Its CMap's name: Identity-H and the like.
    cmap: &'static str,
    /// The CIDFont's subtype: CIDFontType2 (TrueType) or CIDFontType0 (CFF).
    cid_font: &'static str,
    /// Its program, under this key of its descriptor, with the program's dictionary.
    program: (&'static str, Dictionary, Vec<u8>),
    /// CIDs' widths, for `/W`; the others are `/DW`'s 1000.
    widths: Vec<(u16, i64)>,
    /// The ToUnicode map's entries; no map when empty.
    to_unicode: Vec<(u32, String)>,
    cid_to_gid: Option<CidToGid>,
}

impl AddFont for Type0 {
    fn add(&self, doc: &mut Document) -> ObjectId {
        let d = descriptor(doc, self.base, 4, &Some(self.program.clone()));
        let mut w = Vec::new();
        for &(cid, width) in &self.widths {
            w.push(Object::Integer(cid.into()));
            w.push(Object::Array(vec![width.into()]));
        }
        let mut cid_font = dictionary! {
            "Type" => "Font", "Subtype" => self.cid_font, "BaseFont" => self.base,
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal("Identity"),
                "Supplement" => 0,
            },
            "FontDescriptor" => d, "DW" => 1000, "W" => w,
        };
        match &self.cid_to_gid {
            Some(CidToGid::Identity) => cid_font.set("CIDToGIDMap", "Identity"),
            Some(CidToGid::Map(gids)) => {
                let bytes: Vec<u8> = gids.iter().flat_map(|g| g.to_be_bytes()).collect();
                let map = doc.add_object(Stream::new(Dictionary::new(), bytes));
                cid_font.set("CIDToGIDMap", map);
            }
            None => {}
        }
        let cid_font = doc.add_object(cid_font);
        let mut font = dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => self.base,
            "Encoding" => self.cmap, "DescendantFonts" => vec![cid_font.into()],
        };
        if !self.to_unicode.is_empty() {
            let map = doc.add_object(to_unicode(2, &self.to_unicode));
            font.set("ToUnicode", map);
        }
        doc.add_object(font)
    }
}

/// A Type 3 font whose one glyph, `a` at 0x61, is a box drawn by the PDF itself.
struct Type3;

impl AddFont for Type3 {
    fn add(&self, doc: &mut Document) -> ObjectId {
        let a = doc.add_object(Stream::new(
            Dictionary::new(),
            b"1000 0 0 0 800 800 d1 100 0 600 700 re f".to_vec(),
        ));
        doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type3",
            "FontBBox" => vec![0.into(), 0.into(), 800.into(), 800.into()],
            "FontMatrix" => vec![0.001.into(), 0.into(), 0.into(), 0.001.into(), 0.into(), 0.into()],
            "CharProcs" => dictionary! { "a" => a },
            "Encoding" => dictionary! {
                "Type" => "Encoding", "Differences" => vec![97.into(), "a".into()],
            },
            "FirstChar" => 97, "LastChar" => 97, "Widths" => vec![1000.into()],
        })
    }
}

/// A 300 × 200 page drawing `content`, with `fonts` (each a resource name and the font).
fn page(fonts: &[(&str, &dyn AddFont)], content: &str) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let mut res = Dictionary::new();
    for (name, font) in fonts {
        let id = font.add(&mut p.doc);
        res.set(*name, id);
    }
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", s);
    d.set("Resources", dictionary! { "Font" => res });
    p
}

/// A line showing the string `hex` (its bytes in hex) in font resource `font`, `y` points
/// up the page.
fn line(font: &str, y: i32, hex: &str) -> String {
    format!("BT /{font} 20 Tf 10 {y} Td <{hex}> Tj ET\n")
}

/// A page with one line, showing `hex` in `font`.
fn one(font: &dyn AddFont, hex: &str) -> Pdf {
    page(&[("F1", font)], &line("F1", 100, hex))
}

/// The page's lines of text, as read.
fn texts(p: &Pdf) -> Vec<String> {
    let objs = p.page_objects(0).unwrap();
    let text = objs.into_iter().filter(|o| o.kind == ObjectKind::Text);
    text.map(|o| o.text.unwrap_or_default()).collect()
}

/// The fonts of the page's line reading `text`, one for each part.
fn fonts_of(p: &Pdf, text: &str) -> Vec<LineFont> {
    let objs = p.page_objects(0).unwrap();
    let o = objs
        .iter()
        .find(|o| o.kind == ObjectKind::Text && o.text.as_deref() == Some(text))
        .unwrap_or_else(|| panic!("no line {text:?} in {:?}", texts(p)));
    p.line_fonts(0, o.id).unwrap()
}

/// The font of the page's line reading `text`, in one part.
fn font_of(p: &Pdf, text: &str) -> LineFont {
    let mut f = fonts_of(p, text);
    assert_eq!(f.len(), 1, "{f:?}");
    f.remove(0)
}

/// The fonts of the page's only line of text, one for each part.
fn fonts(p: &Pdf) -> Vec<LineFont> {
    let objs = p.page_objects(0).unwrap();
    let text: Vec<_> = objs.iter().filter(|o| o.kind == ObjectKind::Text).collect();
    assert_eq!(text.len(), 1, "{:?}", texts(p));
    p.line_fonts(0, text[0].id).unwrap()
}

/// The font of the page's only line of text, in one part.
fn only_font(p: &Pdf) -> LineFont {
    let mut f = fonts(p);
    assert_eq!(f.len(), 1, "{f:?}");
    f.remove(0)
}

fn text(p: &Pdf) -> String {
    p.page_text(0).trim().to_string()
}

/// Liberation Sans as a subset TrueType font with WinAnsi, codes 32 to 255, its widths
/// from the program.
fn sans_simple() -> Simple {
    let len = sans().len() as i64;
    Simple {
        subtype: "TrueType",
        base: "ABCDEF+LiberationSans",
        program: Some((
            "FontFile2",
            dictionary! { "Length1" => len },
            sans().to_vec(),
        )),
        flags: 32,
        encoding: Some("WinAnsiEncoding".into()),
        first: 32,
        widths: widths_by_cmap(sans(), "WinAnsiEncoding", 32, 255),
        to_unicode: Vec::new(),
    }
}

/// "Hello" in WinAnsi (and ASCII).
const HELLO: &str = "48656C6C6F";

/// A Limare test font (tools/make_encoding_test_fonts.py) as a simple font with no
/// `/Encoding`, its program under `key`: every glyph is a box 500 wide, and the program's
/// own encoding is TeX's OT1 (ff and fi at 0x0B and 0x0C, ” at 0x22, A to C, “ at 0x5C, – at
/// 0x7B, — at 0x7C). Named as a subset, as embedded fonts mostly are.
fn limare(subtype: &'static str, key: &'static str, dict: Dictionary, file: &str) -> Simple {
    Simple {
        subtype,
        base: "ABCDEF+LimareOwnEncoding",
        program: Some((key, dict, font_file(file))),
        flags: 4,
        encoding: None,
        first: 0,
        widths: vec![500; 256],
        to_unicode: Vec::new(),
    }
}

/// The Limare test font as a bare CFF program (`/FontFile3` `/Type1C`).
fn limare_cff() -> Simple {
    let dict = dictionary! { "Subtype" => "Type1C" };
    limare("Type1", "FontFile3", dict, "LimareOwnEncoding.cff")
}

/// The Limare test font as a Type 1 program (`/FontFile`).
fn limare_type1() -> Simple {
    let len = font_file("LimareOwnEncoding.pfa").len() as i64;
    let dict = dictionary! { "Length1" => len, "Length2" => 0, "Length3" => 0 };
    limare("Type1", "FontFile", dict, "LimareOwnEncoding.pfa")
}

/// The Limare test font as a symbolic TrueType program: (3,0) and (1,0) cmaps, glyph names
/// in `post`.
fn limare_symbolic() -> Simple {
    let mut f = limare(
        "TrueType",
        "FontFile2",
        Dictionary::new(),
        "LimareSymbolic.ttf",
    );
    f.base = "ABCDEF+LimareSymbolic";
    f
}

/// What the Limare test fonts show: ff, fi, ”, A, “, – and —.
const LIMARE_SHOWN: &str = "0B0C22415C7B7C";
const LIMARE_READ: &str = "fffi”A“–—";

/// Source Serif 4's bare CFF program (named glyphs; the standard encoding its own) as a
/// simple font, with `differences` (code, glyph name) over that encoding; codes 32 to 126,
/// their widths from the program.
fn serif_cff(differences: &[(u8, &str)]) -> Simple {
    let otf = serif();
    let f = FontRef::new(&otf).unwrap();
    let cff = f.table_data(Tag::new(b"CFF ")).unwrap().as_bytes().to_vec();
    let mut widths = widths_by_cmap(&otf, "StandardEncoding", 32, 126);
    let mut diffs = Vec::new();
    for &(code, name) in differences {
        widths[code as usize - 32] = advance(&otf, named_glyph(&otf, name));
        diffs.push(Object::Integer(code.into()));
        diffs.push(Object::Name(name.as_bytes().to_vec()));
    }
    let encoding = (!differences.is_empty())
        .then(|| dictionary! { "Type" => "Encoding", "Differences" => diffs }.into());
    Simple {
        subtype: "Type1",
        base: "ABCDEF+SourceSerif4-Regular",
        program: Some(("FontFile3", dictionary! { "Subtype" => "Type1C" }, cff)),
        flags: 32,
        encoding,
        first: 32,
        widths,
        to_unicode: Vec::new(),
    }
}

/// Liberation Sans as a Type 0 font with Identity-H, each code a glyph number
/// (`/CIDToGIDMap /Identity`), with widths for the glyphs of `chars` and, if `mapped`, a
/// ToUnicode map for them.
fn sans_type0(chars: &str, mapped: bool) -> Type0 {
    let len = sans().len() as i64;
    let gids: Vec<u16> = chars.chars().map(|c| glyph(sans(), c)).collect();
    Type0 {
        base: "ABCDEF+LiberationSans",
        cmap: "Identity-H",
        cid_font: "CIDFontType2",
        program: (
            "FontFile2",
            dictionary! { "Length1" => len },
            sans().to_vec(),
        ),
        widths: gids.iter().map(|&g| (g, advance(sans(), g))).collect(),
        to_unicode: if mapped {
            let pairs = gids.iter().zip(chars.chars());
            pairs.map(|(&g, c)| (g.into(), c.to_string())).collect()
        } else {
            Vec::new()
        },
        cid_to_gid: Some(CidToGid::Identity),
    }
}

/// The codes of `text` in [`sans_type0`]: Liberation Sans's glyph numbers.
fn sans_codes(text: &str) -> Vec<u16> {
    text.chars().map(|c| glyph(sans(), c)).collect()
}

/// Source Serif 4 cut down by `subsetter` to the glyphs of `chars` (glyph 1 first, in their
/// order), and each character's glyph there. Its CFF program is CID-keyed, each glyph's CID
/// its number, and it has no cmap.
fn serif_subset(chars: &str) -> (Vec<u8>, Vec<(char, u16)>) {
    let otf = serif();
    let mut mapper = subsetter::GlyphRemapper::new();
    let mut cids = Vec::new();
    for c in chars.chars() {
        cids.push((c, mapper.remap(glyph(&otf, c))));
    }
    (subsetter::subset(&otf, 0, &mapper).unwrap(), cids)
}

/// [`serif_subset`] of `chars` as a Type 0 font with Identity-H and a CIDFontType0: the bare
/// CFF program (`/FontFile3` `/CIDFontType0C`), or the whole OpenType font (`/OpenType`),
/// with widths and a ToUnicode map for its glyphs. Each CID with its character.
fn serif_cid(chars: &str, bare: bool) -> (Type0, Vec<(char, u16)>) {
    let otf = serif();
    let (sub, cids) = serif_subset(chars);
    let program = if bare {
        let f = FontRef::new(&sub).unwrap();
        let cff = f.table_data(Tag::new(b"CFF ")).unwrap().as_bytes().to_vec();
        (
            "FontFile3",
            dictionary! { "Subtype" => "CIDFontType0C" },
            cff,
        )
    } else {
        ("FontFile3", dictionary! { "Subtype" => "OpenType" }, sub)
    };
    let widths = cids
        .iter()
        .map(|&(c, cid)| (cid, advance(&otf, glyph(&otf, c))));
    let to_unicode = cids.iter().map(|&(c, cid)| (cid.into(), c.to_string()));
    let font = Type0 {
        base: "ABCDEF+SourceSerif4-Regular",
        cmap: "Identity-H",
        cid_font: "CIDFontType0",
        program,
        widths: widths.collect(),
        to_unicode: to_unicode.collect(),
        cid_to_gid: None,
    };
    (font, cids)
}

/// The CID of `c` in [`serif_cid`]'s list.
fn cid_of(cids: &[(char, u16)], c: char) -> u16 {
    cids.iter().find(|&&(d, _)| d == c).unwrap().1
}

// Simple TrueType.

#[test]
fn a_truetype_subset_writes_its_letters_by_its_encoding() {
    let p = one(&sans_simple(), HELLO);
    assert_eq!(text(&p), "Hello");
    let f = only_font(&p);
    assert_eq!(f.refused(), None);
    assert_eq!(f.code('e'), Ok(vec![b'e']));
    assert_eq!(f.code('H'), Ok(vec![b'H']));
    // Letters not on the line, WinAnsi's beyond ASCII among them, and the space (which
    // draws nothing).
    assert_eq!(f.code('z'), Ok(vec![b'z']));
    assert_eq!(f.code('é'), Ok(vec![0xE9]));
    assert_eq!(f.code('€'), Ok(vec![0x80]));
    assert_eq!(f.code(' '), Ok(vec![0x20]));
    // The program has ŋ, but no code of WinAnsi reads as it.
    assert_eq!(f.code('ŋ'), Err(NoCode));
}

#[test]
fn a_width_off_by_more_than_two_percent_is_refused() {
    let e = (b'e' - 32) as usize;
    let mut font = sans_simple();
    let right = font.widths[e];
    font.widths[e] = (right as f64 * 1.10).round() as i64;
    let f = font_of(&one(&font, HELLO), "Hello");
    assert_eq!(f.code('e'), Err(Width));
    assert_eq!(f.code('H'), Ok(vec![b'H']));
    // 1% off is within what writers' rounding gives.
    font.widths[e] = (right as f64 * 1.01).round() as i64;
    assert_eq!(
        font_of(&one(&font, HELLO), "Hello").code('e'),
        Ok(vec![b'e'])
    );
    // A code past `/LastChar` has no width in the PDF.
    let mut font = sans_simple();
    font.widths.truncate(127 - 32);
    let f = font_of(&one(&font, HELLO), "Hello");
    assert_eq!(f.code('é'), Err(Width));
    assert_eq!(f.code('o'), Ok(vec![b'o']));
}

#[test]
fn a_tounicode_entry_the_encoding_disagrees_with_is_refused() {
    let mut font = sans_simple();
    font.to_unicode = vec![(0x61, "ŋ".into()), (0x65, "e".into())];
    let p = one(&font, "6165");
    assert_eq!(text(&p), "ŋe");
    let f = font_of(&p, "ŋe");
    // WinAnsi names a at 0x61.
    assert_eq!(f.code('ŋ'), Err(Encoding));
    // 0x61 doesn't read as a any more, and no other code does.
    assert_eq!(f.code('a'), Err(NoCode));
    // An entry that agrees changes nothing.
    assert_eq!(f.code('e'), Ok(vec![b'e']));
}

#[test]
fn a_symbolic_truetype_font_writes_by_its_own_glyph_names() {
    let p = one(&limare_symbolic(), LIMARE_SHOWN);
    assert_eq!(text(&p), LIMARE_READ);
    let f = only_font(&p);
    assert_eq!(f.refused(), None);
    assert_eq!(f.code('”'), Ok(vec![0x22]));
    assert_eq!(f.code('–'), Ok(vec![0x7B]));
    assert_eq!(f.code('A'), Ok(vec![0x41]));
    // B is in the program but not on the line.
    assert_eq!(f.code('B'), Ok(vec![0x42]));
}

#[test]
fn a_tounicode_entry_the_glyph_s_name_contradicts_is_refused() {
    // The glyph at 0x41 is named A in `post`; the map says Ω.
    let mut font = limare_symbolic();
    font.to_unicode = vec![(0x41, "Ω".into())];
    let p = one(&font, "414222");
    assert_eq!(text(&p), "ΩB”");
    let f = only_font(&p);
    assert_eq!(f.code('Ω'), Err(Contradicted));
    assert_eq!(f.code('A'), Err(NoCode));
    assert_eq!(f.code('B'), Ok(vec![0x42]));
}

// Simple CFF, Type 1 and OpenType.

/// What a Limare test font in a simple font, by its own encoding, can and can't write.
fn writes_by_its_own_encoding(f: &LineFont) {
    assert_eq!(f.refused(), None);
    assert_eq!(f.code('”'), Ok(vec![0x22]));
    assert_eq!(f.code('“'), Ok(vec![0x5C]));
    assert_eq!(f.code('–'), Ok(vec![0x7B]));
    assert_eq!(f.code('—'), Ok(vec![0x7C]));
    assert_eq!(f.code('A'), Ok(vec![0x41]));
    assert_eq!(f.code('C'), Ok(vec![0x43]));
    // The ligatures read as their letters, so no code reads as ﬁ or ﬀ, and they are never
    // used for f or i: 0x66 reads as f by the standard encoding, but the program has no
    // glyph there.
    assert_eq!(f.code('ﬁ'), Err(NoCode));
    assert_eq!(f.code('ﬀ'), Err(NoCode));
    assert_eq!(f.code('f'), Err(NoGlyph));
    assert_eq!(f.code('i'), Err(NoGlyph));
    assert_eq!(f.code('€'), Err(NoCode));
}

#[test]
fn a_cff_program_writes_by_its_own_encoding_but_not_its_ligatures() {
    let p = one(&limare_cff(), LIMARE_SHOWN);
    assert_eq!(text(&p), LIMARE_READ);
    writes_by_its_own_encoding(&only_font(&p));
}

#[test]
fn a_type1_program_writes_by_its_own_encoding_but_not_its_ligatures() {
    let p = one(&limare_type1(), LIMARE_SHOWN);
    assert_eq!(text(&p), LIMARE_READ);
    writes_by_its_own_encoding(&only_font(&p));
}

#[test]
fn a_whole_embedded_font_writes_no_ascii_its_program_lacks() {
    // Not named as a subset, with no encoding of the PDF's own: before its program was
    // read, plain ASCII was written in such a font as itself, and its f drew nothing.
    let mut font = limare_cff();
    font.base = "LimareOwnEncoding";
    let f = only_font(&one(&font, LIMARE_SHOWN));
    assert_eq!(f.code('f'), Err(NoGlyph));
    assert_eq!(f.code('A'), Ok(vec![0x41]));
}

#[test]
fn an_opentype_program_in_a_simple_font_writes_by_the_pdf_s_encoding() {
    let otf = serif();
    let font = Simple {
        subtype: "Type1",
        base: "ABCDEF+SourceSerif4-Regular",
        program: Some((
            "FontFile3",
            dictionary! { "Subtype" => "OpenType" },
            otf.clone(),
        )),
        flags: 32,
        encoding: Some("WinAnsiEncoding".into()),
        first: 32,
        widths: widths_by_cmap(&otf, "WinAnsiEncoding", 32, 255),
        to_unicode: Vec::new(),
    };
    let p = one(&font, HELLO);
    assert_eq!(text(&p), "Hello");
    let f = font_of(&p, "Hello");
    assert_eq!(f.refused(), None);
    assert_eq!(f.code('a'), Ok(vec![b'a']));
    assert_eq!(f.code('l'), Ok(vec![b'l']));
}

// Type 0.

#[test]
fn an_identity_truetype_font_writes_each_glyph_by_its_number() {
    let p = one(&sans_type0("Helo", true), &hex2(&sans_codes("Hello")));
    assert_eq!(text(&p), "Hello");
    let f = only_font(&p);
    assert_eq!(f.refused(), None);
    // Liberation Sans's l is glyph 79 and H glyph 43.
    assert_eq!(f.code('l'), Ok(two(glyph(sans(), 'l'))));
    assert_eq!(f.code('H'), Ok(two(glyph(sans(), 'H'))));
}

#[test]
fn a_cid_whose_glyph_the_program_reads_otherwise_is_contradicted() {
    // The map says A's glyph is ა (a letter Liberation Sans hasn't, so no other code can
    // read as it); its cmap says A. Its width is right, so only the reading is wrong.
    let a = glyph(sans(), 'A');
    let mut font = sans_type0("Helo", true);
    font.to_unicode.push((a.into(), "ა".into()));
    font.widths.push((a, advance(sans(), a)));
    let f = font_of(&one(&font, &hex2(&sans_codes("Hello"))), "Hello");
    assert_eq!(f.code('ა'), Err(Contradicted));
    assert_eq!(f.code('e'), Ok(two(glyph(sans(), 'e'))));
}

#[test]
fn a_cid_width_well_off_the_program_s_is_refused() {
    let o = glyph(sans(), 'o');
    let mut font = sans_type0("Helo", true);
    for (cid, w) in &mut font.widths {
        if *cid == o {
            *w = (*w as f64 * 1.10).round() as i64;
        }
    }
    let f = font_of(&one(&font, &hex2(&sans_codes("Hello"))), "Hello");
    assert_eq!(f.code('o'), Err(Width));
    assert_eq!(f.code('l'), Ok(two(glyph(sans(), 'l'))));
}

#[test]
fn a_cid_past_the_program_s_glyphs_has_no_glyph() {
    // Liberation Sans has 2620 glyphs; ☃ isn't one of its characters.
    let mut font = sans_type0("Helo", true);
    font.to_unicode.push((3000, "☃".into()));
    let f = font_of(&one(&font, &hex2(&sans_codes("Hello"))), "Hello");
    assert_eq!(f.code('☃'), Err(NoGlyph));
}

#[test]
fn a_cid_keyed_cff_font_writes_each_glyph_by_its_cid() {
    for bare in [true, false] {
        let (font, cids) = serif_cid("Helo", bare);
        let shown: Vec<u16> = "Hello".chars().map(|c| cid_of(&cids, c)).collect();
        let p = one(&font, &hex2(&shown));
        assert_eq!(text(&p), "Hello", "bare CFF: {bare}");
        let f = only_font(&p);
        assert_eq!(f.refused(), None, "bare CFF: {bare}");
        assert_eq!(f.code('l'), Ok(two(cid_of(&cids, 'l'))), "bare CFF: {bare}");
        assert_eq!(f.code('H'), Ok(two(cid_of(&cids, 'H'))), "bare CFF: {bare}");
        // The program has no cmap, and the map reads no code as x.
        assert_eq!(f.code('x'), Err(NoCode), "bare CFF: {bare}");
    }
}

// Several codes for one character.

#[test]
fn codes_drawing_different_glyphs_for_one_character_are_ambiguous() {
    // 0x31 draws a.sups (a superior a), which reads as a, as 0x61's a does.
    let font = serif_cff(&[(0x31, "a.sups")]);
    let p = one(&font, "4243");
    assert_eq!(text(&p), "BC");
    assert_eq!(font_of(&p, "BC").code('a'), Err(Ambiguous));
    // Other letters are one code each.
    assert_eq!(font_of(&p, "BC").code('b'), Ok(vec![b'b']));
}

#[test]
fn the_code_the_line_uses_settles_which_glyph() {
    let font = serif_cff(&[(0x31, "a.sups")]);
    let p = one(&font, "4231");
    assert_eq!(font_of(&p, "Ba").code('a'), Ok(vec![0x31]));
    let p = one(&font, "4261");
    assert_eq!(font_of(&p, "Ba").code('a'), Ok(vec![0x61]));
}

#[test]
fn a_code_used_elsewhere_on_the_page_in_the_font_settles_which_glyph() {
    let font = serif_cff(&[(0x31, "a.sups")]);
    let p = page(
        &[("F1", &font)],
        &(line("F1", 100, "4243") + &line("F1", 50, "31")),
    );
    assert_eq!(font_of(&p, "BC").code('a'), Ok(vec![0x31]));
    // A line in another font object, even a copy of this one, doesn't count.
    let p = page(
        &[("F1", &font), ("F2", &font)],
        &(line("F1", 100, "4243") + &line("F2", 50, "31")),
    );
    assert_eq!(font_of(&p, "BC").code('a'), Err(Ambiguous));
}

#[test]
fn codes_drawing_one_glyph_give_the_line_s_or_else_the_lowest() {
    // 0x30 and 0x61 both draw a.
    let font = serif_cff(&[(0x30, "a")]);
    let p = one(&font, "4243");
    assert_eq!(font_of(&p, "BC").code('a'), Ok(vec![0x30]));
    let p = one(&font, "4261");
    assert_eq!(font_of(&p, "Ba").code('a'), Ok(vec![0x61]));
}

// Fonts that can't take new text.

#[test]
fn a_type3_font_is_refused() {
    let p = one(&Type3, "61");
    let f = only_font(&p);
    assert_eq!(f.refused(), Some(FontRefusal::Type3));
    assert_eq!(f.code('a'), Err(CharRefusal::Font(FontRefusal::Type3)));
}

/// An unembedded simple font `base` with `encoding`, if any.
fn unembedded(base: &'static str, encoding: Option<&str>) -> Simple {
    Simple {
        subtype: "Type1",
        base,
        program: None,
        flags: 32,
        encoding: encoding.map(Object::from),
        first: 0,
        widths: Vec::new(),
        to_unicode: Vec::new(),
    }
}

#[test]
fn unembedded_fonts_write_ascii_only_when_reusable() {
    let p = one(&unembedded("Helvetica", Some("WinAnsiEncoding")), HELLO);
    let f = only_font(&p);
    assert_eq!(f.refused(), Some(FontRefusal::NotEmbedded));
    assert_eq!(f.code('a'), Ok(vec![b'a']));
    assert_eq!(f.code('~'), Ok(vec![b'~']));
    assert_eq!(f.code(' '), Ok(vec![b' ']));
    // WinAnsi has é, but only ASCII is written in a font with no program.
    let not_embedded = Err(CharRefusal::Font(FontRefusal::NotEmbedded));
    assert_eq!(f.code('é'), not_embedded);
    // A subset's name, or Symbol, can't be reused at all.
    let p = one(
        &unembedded("ABCDEF+Helvetica", Some("WinAnsiEncoding")),
        HELLO,
    );
    let f = only_font(&p);
    assert_eq!(f.refused(), Some(FontRefusal::NotEmbedded));
    assert_eq!(f.code('a'), not_embedded);
    let p = one(&unembedded("Symbol", None), "616263");
    assert_eq!(only_font(&p).code('a'), not_embedded);
}

#[test]
fn type0_fonts_with_other_cmaps_are_refused() {
    let mut font = sans_type0("A", false);
    font.cmap = "UniJIS-UCS2-H";
    let f = font_of(&one(&font, "0041"), "A");
    assert_eq!(f.refused(), Some(FontRefusal::Cmap));
    assert_eq!(f.code('A'), Err(CharRefusal::Font(FontRefusal::Cmap)));
}

#[test]
fn vertical_writing_is_refused() {
    let mut font = sans_type0("Hi", true);
    font.cmap = "Identity-V";
    let f = only_font(&one(&font, &hex2(&sans_codes("Hi"))));
    assert_eq!(f.refused(), Some(FontRefusal::Vertical));
    assert_eq!(f.code('H'), Err(CharRefusal::Font(FontRefusal::Vertical)));
}

#[test]
fn a_program_limarepdf_can_t_read_is_refused() {
    let mut font = sans_simple();
    font.program = Some((
        "FontFile2",
        Dictionary::new(),
        b"Not a font program at all. ".repeat(40),
    ));
    let f = only_font(&one(&font, HELLO));
    assert_eq!(f.refused(), Some(FontRefusal::Unreadable));
    assert_eq!(f.code('e'), Err(CharRefusal::Font(FontRefusal::Unreadable)));
}

#[test]
fn limarepdf_s_own_embedded_fonts_are_refused() {
    // Helvetica's WinAnsi can't write ż, ó, ł or ć, so this embeds a font of LimarePDF's.
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    p.add_text(
        0,
        (20.0, 60.0),
        "Zażółć",
        FontChoice::default(),
        14.0,
        [0.0; 3],
    )
    .unwrap();
    let fonts = fonts_of(&p, "Zażółć");
    assert!(!fonts.is_empty());
    for f in fonts {
        assert_eq!(f.refused(), Some(FontRefusal::Own));
        assert_eq!(f.code('a'), Err(CharRefusal::Font(FontRefusal::Own)));
    }
}

// Lines in several fonts.

#[test]
fn each_part_of_a_line_has_its_own_font() {
    let p = page(
        &[
            ("F1", &sans_simple()),
            ("F2", &unembedded("Helvetica", Some("WinAnsiEncoding"))),
        ],
        &format!("BT /F1 20 Tf 10 100 Td <{HELLO}> Tj /F2 20 Tf ( world) Tj ET\n"),
    );
    let f = fonts_of(&p, "Hello world");
    assert_eq!(f.len(), 2, "{f:?}");
    assert_eq!(f[0].refused(), None);
    assert_eq!(f[0].code('é'), Ok(vec![0xE9]));
    assert_eq!(f[1].refused(), Some(FontRefusal::NotEmbedded));
    assert_eq!(f[1].code('w'), Ok(vec![b'w']));
    assert_eq!(
        f[1].code('é'),
        Err(CharRefusal::Font(FontRefusal::NotEmbedded))
    );
}

// PB-26: a Type 0 TrueType font with no ToUnicode map.

#[test]
fn an_identity_truetype_font_without_tounicode_reads_by_its_program() {
    // The codes are Liberation Sans's glyph numbers, not ASCII: H is 0x002B.
    let p = one(
        &sans_type0("Helo wrd", false),
        &hex2(&sans_codes("Hello world")),
    );
    assert_eq!(text(&p), "Hello world");
    // And so it can be written in.
    let f = only_font(&p);
    assert_eq!(f.refused(), None);
    assert_eq!(f.code('w'), Ok(two(glyph(sans(), 'w'))));
}

#[test]
fn a_cid_to_gid_map_is_followed_to_the_program_s_glyphs() {
    // CIDs 1 to 8 for the characters of "Helo wrd", in turn.
    let chars: Vec<char> = "Helo wrd".chars().collect();
    let gids: Vec<u16> = chars.iter().map(|&c| glyph(sans(), c)).collect();
    let cid = |c: char| chars.iter().position(|&d| d == c).unwrap() as u16 + 1;
    let mut font = sans_type0("", false);
    font.cid_to_gid = Some(CidToGid::Map([0].into_iter().chain(gids.clone()).collect()));
    font.widths = (gids.iter().enumerate())
        .map(|(i, &g)| (i as u16 + 1, advance(sans(), g)))
        .collect();
    let shown: Vec<u16> = "Hello world".chars().map(cid).collect();
    let p = one(&font, &hex2(&shown));
    assert_eq!(text(&p), "Hello world");
    let f = only_font(&p);
    assert_eq!(f.code('w'), Ok(two(cid('w'))));
    assert_eq!(f.code('H'), Ok(two(1)));
}

#[test]
fn a_tounicode_naming_identity_reads_each_code_as_its_unicode_value() {
    // As pdf.js's bug951051 and bug1650302 are written: `/ToUnicode /Identity-H` in place
    // of a map, each code a character's UTF-16 value. The PDF says so, whatever glyphs the
    // codes draw (here Liberation Sans's of those numbers, which its cmap reads otherwise).
    let shown: Vec<u16> = "Výbava ș".encode_utf16().collect();
    let mut p = one(&sans_type0("", false), &hex2(&shown));
    let type0 = |o: &Object| {
        let subtype = o.as_dict().and_then(|d| d.get(b"Subtype"));
        subtype
            .and_then(Object::as_name)
            .is_ok_and(|n| n == b"Type0")
    };
    let id = (p.doc.objects.iter())
        .find(|(_, o)| type0(o))
        .map(|(id, _)| *id)
        .unwrap();
    let font = p.doc.get_dictionary_mut(id).unwrap();
    font.set("ToUnicode", Object::Name(b"Identity-H".to_vec()));
    assert_eq!(text(&p), "Výbava ș");
}

// Stage 2: edited lines written in their own font.

/// The page's text line reading `text`.
fn line_of(p: &Pdf, text: &str) -> PageObject {
    let objs = p.page_objects(0).unwrap().into_iter();
    let mut found = objs.filter(|o| o.kind == ObjectKind::Text && o.text.as_deref() == Some(text));
    found
        .next()
        .unwrap_or_else(|| panic!("no line {text:?} in {:?}", texts(p)))
}

/// `n` characters of a line as read, from its character `from` on, unchanged where they were.
fn keep(from: usize, n: usize) -> Vec<Option<usize>> {
    (from..from + n).map(Some).collect()
}

/// `n` characters typed or moved.
fn none(n: usize) -> Vec<Option<usize>> {
    vec![None; n]
}

/// Part `i` of line `o` holding `text`, in the part's own style, as the app sends an edit:
/// each character the line's own where `origins` says it was (`None` for one typed or
/// moved).
fn at(o: &PageObject, i: usize, text: &str, origins: Vec<Option<usize>>) -> TextPart {
    assert_eq!(origins.len(), text.chars().count(), "{text:?}");
    let s = &o.spans[i];
    TextPart {
        text: text.into(),
        font: PartFont::Span(i),
        size: s.font_size,
        color: s.fill.unwrap_or([0.0; 3]),
        rise: s.rise,
        from: None,
        origins,
    }
}

/// Part `i` of line `o` changed to `new`: the characters it shares with the part's text at
/// its start and end are the line's own there, and those between are typed, as core reads
/// a change of words alone.
fn typed(o: &PageObject, i: usize, new: &str) -> TextPart {
    let line: Vec<char> = o.text.as_deref().unwrap().chars().collect();
    let (a, b) = o.spans[i].chars;
    let (old, text): (&[char], Vec<char>) = (&line[a..b], new.chars().collect());
    let head = old.iter().zip(&text).take_while(|(x, y)| x == y).count();
    let room = old.len().min(text.len()) - head;
    let tail = (old.iter().rev().zip(text.iter().rev()))
        .take_while(|(x, y)| x == y)
        .count()
        .min(room);
    let typed = none(text.len() - head - tail);
    let origins = [keep(a, head), typed, keep(b - tail, tail)];
    at(o, i, new, origins.concat())
}

/// A change of a line's words alone.
fn words(text: &str) -> TextEdit {
    TextEdit {
        text: text.into(),
        font: None,
        size: None,
        color: None,
        parts: Vec::new(),
    }
}

/// Whether `part` of line `o` of page 1 is written all in the font of the line's part it
/// names ([`PageObject::keeps_font`]).
fn kept(p: &Pdf, o: &PageObject, part: &TextPart) -> bool {
    o.keeps_font(part, &p.line_fonts(0, o.id).unwrap())
}

/// Whether a change of line `o`'s words alone to `text` keeps its font
/// ([`TextEdit::keeps_font`]).
fn words_kept(p: &Pdf, o: &PageObject, text: &str) -> bool {
    words(text).keeps_font(o, &p.line_fonts(0, o.id).unwrap())
}

/// A piece of a part as [`PageObject::kept_pieces`] says it is written: its characters,
/// whether it keeps the font, and the characters that send it to another.
type Predicted = ((usize, usize), bool, Vec<char>);

/// How `part` of line `o` of page 1 is written ([`PageObject::kept_pieces`]).
fn predicted(p: &Pdf, o: &PageObject, part: &TextPart) -> Vec<Predicted> {
    let fonts = p.line_fonts(0, o.id).unwrap();
    let pieces = o.kept_pieces(part, &fonts).into_iter();
    pieces.map(|k| (k.chars, k.kept, k.missing)).collect()
}

/// Line `o` of page 1 written as `parts`: the ids [`Pdf::edit_texts`] says were written in
/// another font after all.
fn write(p: &mut Pdf, o: &PageObject, parts: Vec<TextPart>) -> Vec<usize> {
    p.edit_texts(0, &[(o.id, TextEdit::in_parts(parts))])
        .unwrap()
}

/// The page's operators, as parsed.
fn ops(p: &Pdf) -> Vec<Operation> {
    let page = p.page_ids()[0];
    Content::decode(&p.doc.get_page_content(page))
        .unwrap()
        .operations
}

/// An operator as its name and operands, to compare.
fn op_of(o: &Operation) -> (String, Vec<Object>) {
    (o.operator.clone(), o.operands.clone())
}

/// The page's operators, to compare ([`op_of`]).
fn op_list(p: &Pdf) -> Vec<(String, Vec<Object>)> {
    ops(p).iter().map(op_of).collect()
}

/// The operators' names.
fn names(ops: &[Operation]) -> Vec<&str> {
    ops.iter().map(|o| o.operator.as_str()).collect()
}

/// What a show draws, as a `TJ` array holds it ([`shown`]).
#[derive(Clone, Debug, PartialEq)]
enum Item {
    /// Codes: the bytes of the strings next to each other, together.
    Codes(Vec<u8>),
    /// A spacing number, to hundredths.
    Move(f32),
}

/// Codes `b` as a show draws them ([`Item::Codes`]).
fn codes(b: &[u8]) -> Item {
    Item::Codes(b.to_vec())
}

/// What show `o` (`Tj`, `TJ`, `'` or `"`) draws.
fn shown(o: &Operation) -> Vec<Item> {
    let items = match o.operator.as_str() {
        "TJ" => o.operands[0].as_array().unwrap().clone(),
        "Tj" | "'" => o.operands.clone(),
        "\"" => o.operands[2..].to_vec(),
        other => panic!("{other} shows nothing"),
    };
    let mut out = Vec::new();
    for it in items {
        match (it, out.last_mut()) {
            (Object::String(b, _), Some(Item::Codes(codes))) => codes.extend(b),
            (Object::String(b, _), _) => out.push(Item::Codes(b)),
            (n, _) => out.push(Item::Move((n.as_float().unwrap() * 100.0).round() / 100.0)),
        }
    }
    out
}

/// The page's shows in order, each with the name of the font it is drawn in.
fn shows(p: &Pdf) -> Vec<(String, Vec<Item>)> {
    let mut font = String::new();
    let mut out = Vec::new();
    for o in ops(p) {
        match o.operator.as_str() {
            "Tf" => font = String::from_utf8_lossy(o.operands[0].as_name().unwrap()).into(),
            "Tj" | "TJ" | "'" | "\"" => out.push((font.clone(), shown(&o))),
            _ => {}
        }
    }
    out
}

/// The object `o` is, or refers to.
fn resolve<'a>(p: &'a Pdf, o: &'a Object) -> &'a Object {
    p.doc.dereference(o).unwrap().1
}

/// The names of the page's fonts, each with the object it names.
fn font_resources(p: &Pdf) -> Vec<(String, Object)> {
    let page = p.doc.get_dictionary(p.page_ids()[0]).unwrap();
    let res = resolve(p, page.get(b"Resources").unwrap());
    let fonts = resolve(p, res.as_dict().unwrap().get(b"Font").unwrap());
    let fonts = fonts.as_dict().unwrap();
    let mut out: Vec<(String, Object)> = (fonts.iter())
        .map(|(k, v)| (String::from_utf8_lossy(k).into(), v.clone()))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The names of the page's fonts.
fn font_names(p: &Pdf) -> Vec<String> {
    font_resources(p).into_iter().map(|(n, _)| n).collect()
}

/// The embedded programs of the page's fonts named `names`, as their streams decode, each
/// with its font's name.
fn programs_of(p: &Pdf, names: &[String]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for (name, font) in font_resources(p) {
        if !names.contains(&name) {
            continue;
        }
        let font = resolve(p, &font);
        let font = match font.as_dict().unwrap().get(b"DescendantFonts") {
            Ok(d) => resolve(p, &resolve(p, d).as_array().unwrap()[0]),
            Err(_) => font,
        };
        let Ok(d) = font.as_dict().unwrap().get(b"FontDescriptor") else {
            continue;
        };
        let d = resolve(p, d).as_dict().unwrap();
        for key in ["FontFile", "FontFile2", "FontFile3"] {
            if let Ok(f) = d.get(key.as_bytes()) {
                let s = resolve(p, f).as_stream().unwrap();
                out.push((name.clone(), s.get_plain_content().unwrap()));
            }
        }
    }
    out
}

/// The embedded programs of all the page's fonts ([`programs_of`]).
fn programs(p: &Pdf) -> Vec<(String, Vec<u8>)> {
    programs_of(p, &font_names(p))
}

/// The page's words, as read.
fn word_list(p: &Pdf) -> Vec<String> {
    let words = p.page_words(0).unwrap().into_iter();
    words.map(|w| w.text).collect()
}

/// Each part of line `o`: its font and its characters.
fn styles(o: &PageObject) -> Vec<(String, (usize, usize))> {
    (o.spans.iter())
        .map(|s| (s.font_name.clone(), s.chars))
        .collect()
}

/// Where `part` (part 0 of line `o`) first changes the line: its first character that isn't
/// the line's own there.
fn change_in(o: &PageObject, part: &TextPart) -> usize {
    let line: Vec<char> = o.text.as_deref().unwrap().chars().collect();
    let text: Vec<char> = part.text.chars().collect();
    let own = |j: usize| part.origins[j] == Some(j) && line.get(j) == Some(&text[j]);
    (0..text.len()).find(|&j| !own(j)).unwrap_or(text.len())
}

/// Where each of `part`'s characters was in the line ([`TextPart::origins`]).
fn origins_of(part: &TextPart) -> Vec<Option<usize>> {
    part.origins.clone()
}

/// Whether positions `a` and `b` are the same within 0.05 pt.
fn close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.05)
}

/// Part of a show as a test writes it: characters, or a spacing number.
#[derive(Clone, Copy, Debug)]
enum Piece {
    C(&'static str),
    N(f32),
}
use Piece::{C, N};

/// What the pieces of a show read as: their characters, a spacing number under -150 a space.
fn read(pieces: &[Piece]) -> String {
    let each = pieces.iter().map(|p| match *p {
        C(s) => s,
        N(x) if x < -150.0 => " ",
        N(_) => "",
    });
    each.collect()
}

/// A kind of font the writer is checked in: the font, and the code it shows each character
/// with.
struct Kind {
    name: &'static str,
    font: Box<dyn AddFont>,
    code: Box<dyn Fn(char) -> Vec<u8>>,
}

impl Kind {
    fn new(
        name: &'static str,
        font: impl AddFont + 'static,
        code: impl Fn(char) -> Vec<u8> + 'static,
    ) -> Self {
        let (font, code) = (Box::new(font), Box::new(code));
        Self { name, font, code }
    }

    /// The codes of `text`.
    fn codes(&self, text: &str) -> Vec<u8> {
        text.chars().flat_map(|c| (self.code)(c)).collect()
    }

    /// A show of `pieces`, as a content stream holds it.
    fn tj(&self, pieces: &[Piece]) -> String {
        let each: Vec<String> = (pieces.iter())
            .map(|p| match *p {
                C(s) => format!("<{}>", hex(&self.codes(s))),
                N(x) => format!("{x}"),
            })
            .collect();
        format!("[{}] TJ", each.join(" "))
    }

    /// What a show of `pieces` draws ([`shown`]).
    fn items(&self, pieces: &[Piece]) -> Vec<Item> {
        let mut out = Vec::new();
        for p in pieces {
            match (*p, out.last_mut()) {
                (C(s), Some(Item::Codes(codes))) => codes.extend(self.codes(s)),
                (C(s), _) => out.push(Item::Codes(self.codes(s))),
                (N(x), _) => out.push(Item::Move(x)),
            }
        }
        out
    }
}

/// Bytes as a hex string's digits.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// A character as its byte, for fonts whose codes are ASCII's.
fn ascii(c: char) -> Vec<u8> {
    vec![c as u8]
}

/// The characters the Type 0 fixtures of [`latin`] have.
const LATIN: &str = "The quickfox";

/// Source Serif 4 whole, an OpenType program with CFF outlines, in a simple font with
/// WinAnsi, codes 32 to 255, their widths from the program.
fn serif_opentype() -> Simple {
    let otf = serif();
    Simple {
        subtype: "Type1",
        base: "ABCDEF+SourceSerif4-Regular",
        program: Some((
            "FontFile3",
            dictionary! { "Subtype" => "OpenType" },
            otf.clone(),
        )),
        flags: 32,
        encoding: Some("WinAnsiEncoding".into()),
        first: 32,
        widths: widths_by_cmap(&otf, "WinAnsiEncoding", 32, 255),
        to_unicode: Vec::new(),
    }
}

fn simple_truetype() -> Kind {
    Kind::new("simple TrueType", sans_simple(), ascii)
}

fn simple_cff() -> Kind {
    Kind::new("simple CFF", serif_cff(&[]), ascii)
}

fn simple_opentype() -> Kind {
    Kind::new("OpenType CFF in a simple font", serif_opentype(), ascii)
}

fn type0_truetype(mapped: bool) -> Kind {
    let name = match mapped {
        true => "Type 0 TrueType",
        false => "Type 0 TrueType without ToUnicode",
    };
    Kind::new(name, sans_type0(LATIN, mapped), |c| two(glyph(sans(), c)))
}

fn type0_cff(bare: bool) -> Kind {
    let name = match bare {
        true => "Type 0 CFF",
        false => "Type 0 OpenType CFF",
    };
    let (font, cids) = serif_cid(LATIN, bare);
    Kind::new(name, font, move |c| two(cid_of(&cids, c)))
}

fn own_encoding_cff() -> Kind {
    Kind::new("CFF by its own encoding", limare_cff(), ascii)
}

fn own_encoding_type1() -> Kind {
    Kind::new("Type 1 by its own encoding", limare_type1(), ascii)
}

fn symbolic_truetype() -> Kind {
    Kind::new("symbolic TrueType", limare_symbolic(), ascii)
}

/// A line as one `TJ` shows it, another line of the page, and edits of the line as the app
/// sends them, each with the show it should be written as.
struct Script {
    line: Vec<Piece>,
    other: &'static str,
    edits: Vec<Change>,
}

/// An edit of a [`Script`]'s line.
struct Change {
    what: &'static str,
    text: &'static str,
    origins: Vec<Option<usize>>,
    shown: Vec<Piece>,
}

/// "The quck fox", kerned between letters: a letter put in (the number between the letters
/// it goes between dropped), a word deleted, and a word moved (its own kerning lost).
fn latin() -> Script {
    let change = |what, text, origins: &[Vec<Option<usize>>], shown: &[Piece]| Change {
        what,
        text,
        origins: origins.concat(),
        shown: shown.to_vec(),
    };
    // "Th" and "ox" are kerned, and every edit keeps them.
    let kerned = |middle: &[Piece], after: &[Piece]| {
        [&[C("Th"), N(25.0)], middle, &[N(12.0), C("ox")], after].concat()
    };
    Script {
        line: kerned(&[C("e qu"), N(10.0), C("ck f")], &[]),
        other: "fox",
        edits: vec![
            change(
                "a letter put in",
                "The quick fox",
                &[keep(0, 6), none(1), keep(6, 6)],
                &kerned(&[C("e quick f")], &[]),
            ),
            change(
                "a word deleted",
                "The fox",
                &[keep(0, 4), keep(9, 3)],
                &kerned(&[C("e f")], &[]),
            ),
            change(
                "a word moved",
                "The fox quck",
                &[keep(0, 4), keep(9, 3), none(5)],
                &kerned(&[C("e f")], &[C(" quck")]),
            ),
        ],
    }
}

/// "ABC CAB BA" in a Limare test font, which has only A to C of the letters and no space:
/// its spaces are moves. A letter put in, a word deleted, and a word moved (the space
/// before it, which has no code, written as the line's own word gap).
fn limare_line() -> Script {
    let change = |what, text, origins: &[Vec<Option<usize>>], shown: &[Piece]| Change {
        what,
        text,
        origins: origins.concat(),
        shown: shown.to_vec(),
    };
    let gap = N(-300.0);
    Script {
        line: vec![C("A"), N(40.0), C("BC"), gap, C("CAB"), gap, C("BA")],
        other: "CAB",
        edits: vec![
            change(
                "a letter put in",
                "ABBC CAB BA",
                &[keep(0, 2), none(1), keep(2, 8)],
                &[C("A"), N(40.0), C("BBC"), gap, C("CAB"), gap, C("BA")],
            ),
            change(
                "a word deleted",
                "ABC BA",
                &[keep(0, 4), keep(8, 2)],
                &[C("A"), N(40.0), C("BC"), gap, C("BA")],
            ),
            change(
                "a word moved",
                "ABC BA CAB",
                &[keep(0, 4), keep(8, 2), none(4)],
                &[C("A"), N(40.0), C("BC"), gap, C("BA"), gap, C("CAB")],
            ),
        ],
    }
}

/// A 300 × 200 page with the script's line in the kind's font at (10, 100), and its other
/// line in that font below.
fn script_page(kind: &Kind, s: &Script) -> Pdf {
    let other = hex(&kind.codes(s.other));
    let content = format!(
        "BT /F1 20 Tf 10 100 Td {} ET\nBT /F1 20 Tf 10 50 Td <{other}> Tj ET\n",
        kind.tj(&s.line)
    );
    page(&[("F1", kind.font.as_ref())], &content)
}

/// Each of the script's edits in the kind's font, as the app sends it: written in place as
/// the one show it should be, in the line's own font and program with nothing added, read as
/// its words, the characters before the change where they were and the rest of the page as
/// it was; the same as the change of words alone writes; then saved, opened again and edited
/// again, still in that font and program.
fn edits_keep_the_font(kind: &Kind, s: &Script) {
    let was = read(&s.line);
    for e in &s.edits {
        let why = format!("{}, {}", kind.name, e.what);
        let mut p = script_page(kind, s);
        let o = line_of(&p, &was);
        let other = line_of(&p, s.other);
        let (before, fonts, progs) = (ops(&p), font_resources(&p), programs(&p));
        assert_eq!(progs.len(), 1, "{why}");
        let part = at(&o, 0, e.text, e.origins.clone());
        let (n, k) = (e.text.chars().count(), change_in(&o, &part));
        assert_eq!(predicted(&p, &o, &part), [((0, n), true, vec![])], "{why}");
        assert!(kept(&p, &o, &part), "{why}");
        assert_eq!(write(&mut p, &o, vec![part]), Vec::<usize>::new(), "{why}");

        // In place: the page's operators as they were but the line's show, which is now
        // the one wanted.
        let after = ops(&p);
        assert_eq!(after.len(), before.len(), "{why}");
        let show = (before.iter()).position(|o| o.operator == "TJ").unwrap();
        for (i, (a, b)) in before.iter().zip(&after).enumerate() {
            if i != show {
                assert_eq!(op_of(a), op_of(b), "{why}: operator {i}");
            }
        }
        assert_eq!(after[show].operator, "TJ", "{why}");
        assert_eq!(shown(&after[show]), kind.items(&e.shown), "{why}");
        // The line's own font object and its program, nothing added.
        assert_eq!(font_resources(&p), fonts, "{why}");
        assert_eq!(programs(&p), progs, "{why}");

        // Read as its words, in its font; the characters before the change where they were,
        // and the other line too.
        let now = line_of(&p, e.text);
        let font = o.spans[0].font_name.clone();
        assert_eq!(styles(&now), [(font.clone(), (0, n))], "{why}");
        let mut want: Vec<&str> = e.text.split_whitespace().collect();
        want.push(s.other);
        assert_eq!(word_list(&p), want, "{why}");
        assert!(
            close(&now.stops[..k], &o.stops[..k]),
            "{why}: {:?} then {:?}",
            o.stops,
            now.stops
        );
        let (b0, b1) = (o.baseline.unwrap(), now.baseline.unwrap());
        assert!(close(&[b0.0, b0.1], &[b1.0, b1.1]), "{why}");
        assert!(close(&line_of(&p, s.other).bbox, &other.bbox), "{why}");

        // A change of words alone whose characters are kept at its ends is written alike.
        if origins_of(&typed(&o, 0, e.text)) == e.origins {
            let mut q = script_page(kind, s);
            assert!(words_kept(&q, &o, e.text), "{why}");
            q.edit_text(0, o.id, &words(e.text)).unwrap();
            assert_eq!(op_list(&q), op_list(&p), "{why}");
        }

        // Saved, opened again and edited again: still the line's own font and program.
        let mut q = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
        assert_eq!(programs(&q), progs, "{why}: saved");
        let again = line_of(&q, e.text);
        assert_eq!(styles(&again), [(font.clone(), (0, n))], "{why}");
        let shorter: String = e.text.chars().take(n - 1).collect();
        q.edit_text(0, again.id, &words(&shorter)).unwrap();
        let now = line_of(&q, &shorter);
        assert_eq!(styles(&now), [(font, (0, n - 1))], "{why}: edited again");
        assert_eq!(font_names(&q), ["F1"], "{why}: edited again");
        assert_eq!(programs(&q), progs, "{why}: edited again");
        let mut want: Vec<&str> = shorter.split_whitespace().collect();
        want.push(s.other);
        assert_eq!(word_list(&q), want, "{why}: edited again");
    }
}

#[test]
fn a_truetype_subset_keeps_its_font_and_kerning_through_typos_deleted_and_moved_words() {
    edits_keep_the_font(&simple_truetype(), &latin());
}

#[test]
fn a_cff_font_keeps_its_font_and_kerning_through_typos_deleted_and_moved_words() {
    edits_keep_the_font(&simple_cff(), &latin());
}

#[test]
fn an_opentype_cff_simple_font_keeps_its_font_and_kerning_through_edits() {
    edits_keep_the_font(&simple_opentype(), &latin());
}

#[test]
fn an_identity_truetype_font_keeps_its_font_and_kerning_through_edits() {
    edits_keep_the_font(&type0_truetype(true), &latin());
}

#[test]
fn an_identity_truetype_font_without_tounicode_keeps_its_font_through_edits() {
    edits_keep_the_font(&type0_truetype(false), &latin());
}

#[test]
fn an_identity_cff_subset_keeps_its_font_and_kerning_through_edits() {
    edits_keep_the_font(&type0_cff(true), &latin());
}

#[test]
fn an_identity_opentype_cff_subset_keeps_its_font_and_kerning_through_edits() {
    edits_keep_the_font(&type0_cff(false), &latin());
}

#[test]
fn a_cff_font_by_its_own_encoding_keeps_its_font_through_edits() {
    edits_keep_the_font(&own_encoding_cff(), &limare_line());
}

#[test]
fn a_type1_font_by_its_own_encoding_keeps_its_font_through_edits() {
    edits_keep_the_font(&own_encoding_type1(), &limare_line());
}

#[test]
fn a_symbolic_truetype_font_keeps_its_font_through_edits() {
    edits_keep_the_font(&symbolic_truetype(), &limare_line());
}

/// Whether each of line `o`'s characters but spaces is in font `own`.
fn letters_in(o: &PageObject, own: &str) -> Vec<(char, bool)> {
    let text: Vec<char> = o.text.as_deref().unwrap().chars().collect();
    let font = |j: usize| {
        let s = o.spans.iter().find(|s| s.chars.0 <= j && j < s.chars.1);
        s.unwrap().font_name == own
    };
    (0..text.len())
        .filter(|&j| !text[j].is_whitespace())
        .map(|j| (text[j], font(j)))
        .collect()
}

/// Whether each of `text`'s characters but spaces keeps the font, as `pieces` say.
fn letters_kept(text: &str, pieces: &[Predicted]) -> Vec<(char, bool)> {
    let text: Vec<char> = text.chars().collect();
    let kept = |j: usize| pieces.iter().find(|p| p.0 .0 <= j && j < p.0 .1).unwrap().1;
    (0..text.len())
        .filter(|&j| !text[j].is_whitespace())
        .map(|j| (text[j], kept(j)))
        .collect()
}

/// Line `o` of page 1 edited as `part` (its part 0), which `want` says is written in pieces:
/// said so beforehand ([`PageObject::kept_pieces`]) and written so, each piece in the line's
/// font or another, reading as the words; the line's font and program kept, and a font
/// added only for the words that went to another; the characters before the change where
/// they were. With `exact`, the parts of the line as read are the pieces (spaces the font
/// has codes for); else only its letters are compared, as a space written as a move goes
/// with the show before it, in whichever font.
fn falls_back(p: &mut Pdf, o: &PageObject, part: TextPart, want: &[Predicted], exact: bool) {
    let why = part.text.clone();
    let own = o.spans[0].font_name.clone();
    let (fonts, names) = (font_resources(p), font_names(p));
    let progs = programs(p);
    assert_eq!(predicted(p, o, &part), want, "{why}");
    assert_eq!(kept(p, o, &part), want.iter().all(|w| w.1), "{why}");
    let k = change_in(o, &part);
    assert!(write(p, o, vec![part.clone()]).is_empty(), "{why}");
    let now = line_of(p, &part.text);
    let (got, said) = (letters_in(&now, &own), letters_kept(&why, want));
    assert_eq!(got, said, "{why}: {:?}", styles(&now));
    if exact {
        let mut parts: Vec<((usize, usize), bool)> = Vec::new();
        for (font, chars) in styles(&now) {
            match parts.last_mut() {
                Some((c, k)) if *k == (font == own) => c.1 = chars.1,
                _ => parts.push((chars, font == own)),
            }
        }
        let pieces: Vec<_> = want.iter().map(|w| (w.0, w.1)).collect();
        assert_eq!(parts, pieces, "{why}: {:?}", styles(&now));
    }
    let after = font_resources(p);
    assert!(fonts.iter().all(|f| after.contains(f)), "{why}: {after:?}");
    let added = after.iter().filter(|f| !fonts.contains(f));
    let elsewhere = want.iter().any(|w| !w.1);
    assert_eq!(added.count(), usize::from(elsewhere), "{why}: {after:?}");
    assert_eq!(programs_of(p, &names), progs, "{why}");
    let want_words: Vec<&str> = part.text.split_whitespace().collect();
    let read: Vec<String> = word_list(p).into_iter().take(want_words.len()).collect();
    assert_eq!(read, want_words, "{why}");
    let (was, is) = (&o.stops[..k], &now.stops[..k]);
    assert!(close(is, was), "{why}: {was:?} then {is:?}");
}

/// A word holding a character the font's subset has no code for goes whole to another font
/// (plan Q2), with the space between it and the next such word; the rest of the line keeps
/// the font, its kerning too, as `kept_pieces` says beforehand.
#[test]
fn a_word_the_subset_can_t_write_goes_alone_to_another_font() {
    for kind in [type0_cff(true), type0_cff(false), type0_truetype(true)] {
        let s = latin();
        let mut p = script_page(&kind, &s);
        let o = line_of(&p, "The quck fox");
        // b, r, w and n aren't in the subset; e and o are.
        let part = typed(&o, 0, "The quick brown wren fox");
        let want = [
            ((0, 10), true, vec![]),
            ((10, 20), false, vec!['b', 'r', 'w', 'n']),
            ((20, 24), true, vec![]),
        ];
        falls_back(&mut p, &o, part, &want, true);
        let in_f1: Vec<Vec<Item>> = (shows(&p).into_iter())
            .filter(|(f, _)| f == "F1")
            .map(|(_, items)| items)
            .collect();
        let other = kind.items(&[C("fox")]);
        let kerned = [
            other,
            kind.items(&[C("Th"), N(25.0), C("e quick ")]),
            kind.items(&[C(" f"), N(12.0), C("ox")]),
        ];
        assert_eq!(in_f1, kerned, "{}", kind.name);
    }
}

/// PB-53: a right-to-left or shaped character is never written in a line's own font, one code
/// a character, unshaped and in typed order, though the font has a code for it (the whole of
/// Liberation Sans has Hebrew): the edit is refused, and says why.
#[test]
fn scripts_not_written_yet_are_refused_in_a_line_s_own_font_too() {
    // A font with Hebrew in its map, as a document with Hebrew elsewhere has.
    let font = sans_type0("Shalomשלום", true);
    for typed in ["Shalom שלום", "Shalom ש"] {
        let mut p = one(&font, &hex2(&sans_codes("Shalom")));
        let c = typed.chars().last().unwrap();
        let code = only_font(&p).code(c);
        assert!(code.is_ok(), "the font has a code for {c:?}");
        let before = format!("{:?}", p.doc.objects);
        let o = line_of(&p, "Shalom");
        let e = p.edit_text(0, o.id, &words(typed)).unwrap_err().to_string();
        assert!(e.contains("right-to-left"), "{typed}: {e}");
        assert_eq!(format!("{:?}", p.doc.objects), before, "nothing changed");
    }
}

/// A font that can't take new text (Type 3, a subset that isn't embedded) keeps the words
/// left as they were, each letter with its own code; the changed words go to another font.
#[test]
fn a_font_that_can_t_take_new_text_keeps_the_words_left_as_they_were() {
    let subset = unembedded("ABCDEF+Helvetica", Some("WinAnsiEncoding"));
    let src = "BT /F1 20 Tf 10 100 Td (Hello there world) Tj ET\n";
    let typo = vec![
        ((0, 6), true, vec![]),
        ((6, 11), false, vec!['i', 'r']),
        ((11, 17), true, vec![]),
    ];
    let deleted = vec![((0, 11), true, vec![])];
    let moved = vec![
        ((0, 12), true, vec![]),
        ((12, 17), false, vec!['t', 'h', 'e', 'r']),
    ];
    // The word moved: cut, and put at the end with a space before it.
    let moved_origins = [keep(0, 6), keep(12, 5), none(6)].concat();
    let cases = [
        ("Hello their world", None, typo),
        ("Hello world", None, deleted),
        ("Hello world there", Some(moved_origins), moved),
    ];
    for (text, origins, want) in cases {
        let mut p = page(&[("F1", &subset)], src);
        let o = line_of(&p, "Hello there world");
        let part = match origins {
            Some(origins) => at(&o, 0, text, origins),
            None => typed(&o, 0, text),
        };
        let all = want.len() == 1;
        falls_back(&mut p, &o, part, &want, true);
        // Written all in its own font, the line is changed in place.
        if all {
            let one = vec![codes(b"Hello world")];
            assert_eq!(shows(&p), [("F1".into(), one)]);
        }
    }
    // A Type 3 font's one glyph draws a; the b typed sends its word elsewhere. Its a is
    // twice the usual width, so its em is two units of text space: the gap a word's.
    let src = "BT /F1 20 Tf 10 100 Td [(aa) -600 (a)] TJ ET\n";
    let mut p = page(&[("F1", &Type3)], src);
    let o = line_of(&p, "aa a");
    let want = [((0, 3), true, vec![]), ((3, 5), false, vec!['b'])];
    falls_back(&mut p, &o, typed(&o, 0, "aa ab"), &want, true);
    let aa = vec![codes(b"aa"), Item::Move(-600.0)];
    assert_eq!(shows(&p)[0], ("F1".into(), aa));
}

/// A letter left as it was keeps the code it was drawn with even where a new one wouldn't be
/// given it: here the PDF's width for e is 10% off the program's.
#[test]
fn letters_left_as_they_were_keep_codes_new_letters_would_not_be_given() {
    let e = (b'e' - 32) as usize;
    let mut font = sans_simple();
    font.widths[e] = (font.widths[e] as f64 * 1.10).round() as i64;
    let mut p = one(&font, HELLO);
    let o = line_of(&p, "Hello");
    let want = [((0, 6), true, vec![]), ((6, 11), false, vec!['e'])];
    falls_back(&mut p, &o, typed(&o, 0, "Hello there"), &want, true);
}

/// A space the font has no code for is written as a move: the nearest gap of the line read
/// as a space, else a quarter of an em. It reads back as a space.
#[test]
fn a_space_the_font_has_no_code_for_is_the_line_s_word_gap() {
    let kind = own_encoding_cff();
    let gap = N(-300.0);
    let check = |line: &[Piece], text: &str, want: &[Piece]| {
        let content = format!("BT /F1 20 Tf 10 100 Td {} ET\n", kind.tj(line));
        let mut p = page(&[("F1", kind.font.as_ref())], &content);
        let o = line_of(&p, &read(line));
        let part = typed(&o, 0, text);
        let n = part.text.chars().count();
        assert_eq!(predicted(&p, &o, &part), [((0, n), true, vec![])]);
        assert!(write(&mut p, &o, vec![part]).is_empty());
        assert_eq!(shows(&p), [("F1".into(), kind.items(want))], "{text}");
        assert_eq!(word_list(&p), text.split(' ').collect::<Vec<_>>());
    };
    // The line's own gap, and none to be had.
    let want = [C("ABC"), gap, C("CAB"), gap, C("BA")];
    check(&[C("ABC"), gap, C("CAB")], "ABC CAB BA", &want);
    check(&[C("ABC")], "ABC CAB", &[C("ABC"), N(-250.0), C("CAB")]);
}

/// A line in a Limare test font whose first code draws the ff ligature, which reads as "ff".
const LIGATURE: &str = "BT /F1 20 Tf 10 100 Td [<0B41> -300 <4243>] TJ ET\n";

/// A ligature's code is written again only with all its letters unchanged and together;
/// else each letter goes through the font's codes, and the program has no f of its own, so
/// the word goes to another font.
#[test]
fn a_ligature_is_kept_only_with_all_its_letters_unchanged_and_together() {
    let mut p = page(&[("F1", &limare_cff())], LIGATURE);
    let o = line_of(&p, "ffA BC");
    let part = typed(&o, 0, "ffAB BC");
    assert!(kept(&p, &o, &part));
    assert!(write(&mut p, &o, vec![part]).is_empty());
    let want = vec![codes(&[0x0B, 0x41, 0x42]), Item::Move(-300.0), codes(b"BC")];
    assert_eq!(shows(&p), [("F1".into(), want)]);
    assert_eq!(word_list(&p), ["ffAB", "BC"]);
    // One f deleted.
    let mut p = page(&[("F1", &limare_cff())], LIGATURE);
    let o = line_of(&p, "ffA BC");
    let want = [((0, 2), false, vec!['f']), ((2, 5), true, vec![])];
    falls_back(&mut p, &o, typed(&o, 0, "fA BC"), &want, false);
}

/// A ligature character typed (ﬁ, U+FB01) has no code in a font whose ligature glyphs read
/// as their letters, so its word goes to another font, and only that word: LimarePDF reads
/// the character back as its letters ("fi"), which is still the word typed.
#[test]
fn a_typed_ligature_character_sends_only_its_word_to_another_font() {
    let mut p = page(&[("F1", &limare_cff())], LIGATURE);
    let o = line_of(&p, "ffA BC");
    let part = typed(&o, 0, "ﬁA BC");
    let want = [((0, 2), false, vec!['ﬁ']), ((2, 5), true, vec![])];
    assert_eq!(predicted(&p, &o, &part), want);
    assert_eq!(write(&mut p, &o, vec![part]), Vec::<usize>::new());
    let now = line_of(&p, "fiA BC");
    let own = o.spans[0].font_name.as_str();
    let kept = [false, false, false, true, true];
    let letters: Vec<(char, bool)> = "fiABC".chars().zip(kept).collect();
    assert_eq!(letters_in(&now, own), letters, "{:?}", styles(&now));
}

/// Plain ASCII typed into an embedded font goes through its program too: a whole font with
/// no encoding of the PDF's own has no f, so the word with the f goes to another font (it
/// was written in it, drawing nothing). An unembedded standard font still takes plain
/// ASCII as itself, in place.
#[test]
fn ascii_goes_through_the_program_and_an_unembedded_font_still_takes_it() {
    let mut font = limare_cff();
    font.base = "LimareOwnEncoding";
    let src = "BT /F1 20 Tf 10 100 Td [(AB) -300 (C)] TJ ET\n";
    let mut p = page(&[("F1", &font)], src);
    let o = line_of(&p, "AB C");
    let want = [((0, 3), false, vec!['f']), ((3, 5), true, vec![])];
    falls_back(&mut p, &o, typed(&o, 0, "AfB C"), &want, false);
    let f1: Vec<_> = shows(&p).into_iter().filter(|(f, _)| f == "F1").collect();
    assert_eq!(f1, [("F1".into(), vec![codes(b"C")])]);

    let helvetica = unembedded("Helvetica", Some("WinAnsiEncoding"));
    let src = "BT /F1 20 Tf 10 100 Td [(He) 20 (llo)] TJ ET\n";
    let mut p = page(&[("F1", &helvetica)], src);
    let o = line_of(&p, "Hello");
    let before = ops(&p);
    assert!(write(&mut p, &o, vec![typed(&o, 0, "Help")]).is_empty());
    assert_eq!(ops(&p).len(), before.len());
    let want = vec![codes(b"He"), Item::Move(20.0), codes(b"lp")];
    assert_eq!(shows(&p), [("F1".into(), want)]);
    assert_eq!(font_names(&p), ["F1"]);
    // Only plain ASCII, as before: the word with é is written anew (in Helvetica again,
    // which LimarePDF writes it in), the line's show taken out.
    let src = "BT /F1 20 Tf 10 100 Td (Hello) Tj ET\n";
    let mut p = page(&[("F1", &helvetica)], src);
    let o = line_of(&p, "Hello");
    let part = typed(&o, 0, "Héllo");
    assert_eq!(predicted(&p, &o, &part), [((0, 5), false, vec!['é'])]);
    assert!(write(&mut p, &o, vec![part]).is_empty());
    let all = shows(&p);
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(all[0].0.starts_with("LimareEd"), "{all:?}");
    assert_eq!(texts(&p), ["Héllo"]);
}

/// A line in one part drawn by several shows is written in place of the first as one `TJ`:
/// everything before it as it was, its other shows gone, a `'` or `"` show's line move kept,
/// and the show that carries on after it put back where it was.
#[test]
fn a_line_of_several_shows_is_rewritten_in_place_as_one_tj() {
    let src = "BT /F1 20 Tf 10 100 Td (The ) Tj [(qu) 10 (ck)] TJ ( fox) Tj [-3000 (Far)] TJ ET\n\
               BT /F1 20 Tf 10 50 Td (fox) Tj ET\n";
    let mut p = page(&[("F1", &sans_simple())], src);
    let o = line_of(&p, "The quck fox");
    let far = line_of(&p, "Far");
    let before = ops(&p);
    assert!(write(&mut p, &o, vec![typed(&o, 0, "The quick fox")]).is_empty());
    let after = ops(&p);
    let line = ["BT", "Tf", "Td", "TJ", "Tm", "TJ", "ET"];
    let other = ["BT", "Tf", "Td", "Tj", "ET"];
    assert_eq!(names(&after), [&line[..], &other].concat());
    for i in 0..3 {
        assert_eq!(op_of(&after[i]), op_of(&before[i]));
    }
    assert_eq!(shown(&after[3]), [codes(b"The quick fox")]);
    for (a, b) in after[5..].iter().zip(&before[6..]) {
        assert_eq!(op_of(a), op_of(b));
    }
    assert!(close(&line_of(&p, "Far").bbox, &far.bbox));
    assert_eq!(word_list(&p), ["The", "quick", "fox", "Far", "fox"]);

    // Shown by ' or ", whose line move stays.
    let cases = [
        ("(The quck fox) '", vec!["T*"]),
        ("0 0 (The quck fox) \"", vec!["Tw", "Tc", "T*"]),
    ];
    for (show, moves) in cases {
        let src = format!("BT /F1 20 Tf 24 TL 10 124 Td {show} ET\n");
        let mut p = page(&[("F1", &sans_simple())], &src);
        let o = line_of(&p, "The quck fox");
        assert!(write(&mut p, &o, vec![typed(&o, 0, "The quick fox")]).is_empty());
        let want = [vec!["BT", "Tf", "TL", "Td"], moves, vec!["TJ", "ET"]].concat();
        assert_eq!(names(&ops(&p)), want, "{show}");
        let now = line_of(&p, "The quick fox");
        assert_eq!(now.baseline, o.baseline, "{show}");
    }
}

/// A letter shown alone, then a `Tm` back to the left and a `TJ` whose first number moves on
/// again to just after it, as in pdf.js `issue19182.pdf`: the move back and the number
/// forward are written as one spacing number, which moves nothing, so the line reads as it
/// did and keeps its font (written side by side, the reader took the number forward alone
/// as a word gap, and the line went to another font).
#[test]
fn spacing_numbers_side_by_side_are_written_as_one() {
    let font = sans_simple();
    let c = font.widths[(b'C' - 32) as usize];
    let src = format!(
        "BT /F1 20 Tf 1 0 0 1 10 100 Tm (C) Tj 1 0 0 1 0 100 Tm [{} (ompany Secretary)] TJ ET\n",
        -(500 + c)
    );
    let mut p = page(&[("F1", &font)], &src);
    let o = line_of(&p, "Company Secretary");
    let part = typed(&o, 0, "Company Sceretary");
    assert!(kept(&p, &o, &part));
    assert!(write(&mut p, &o, vec![part]).is_empty());
    assert_eq!(font_names(&p), ["F1"]);
    assert_eq!(word_list(&p), ["Company", "Sceretary"]);
    for (font, items) in shows(&p) {
        let numbers = items.windows(2);
        let side_by_side = numbers.filter(|w| matches!(w, [Item::Move(_), Item::Move(_)]));
        assert_eq!(side_by_side.count(), 0, "{font}: {items:?}");
    }
    // The letters stand where they did. The line as read began its box at the second `Tm`,
    // the room its first number moved over included; written again, it begins at the C.
    let (now, was) = (line_of(&p, "Company Sceretary").bbox, o.bbox);
    assert!(close(&now[1..], &was[1..]), "{now:?} {was:?}");
    assert!((now[0] - 10.0).abs() < 0.05, "{now:?}");
}

/// The fonts of all a page's lines, read together with the page read once, are each line's
/// own: the same refusals, the same code for each character, the same preview.
#[test]
fn a_page_s_line_fonts_read_together_are_each_line_s() {
    let src = format!(
        "BT /F1 20 Tf 10 150 Td <{HELLO}> Tj ET\n\
         BT /F2 20 Tf 10 100 Td (fox ) Tj /F1 20 Tf <{HELLO}> Tj ET\n\
         10 10 50 50 re f\n"
    );
    let helvetica = unembedded("Helvetica", Some("WinAnsiEncoding"));
    let p = page(&[("F1", &sans_simple()), ("F2", &helvetica)], &src);
    let objs = p.page_objects(0).unwrap();
    let all = p.page_line_fonts(0).unwrap();
    assert_eq!(all.len(), objs.len());
    assert_eq!(texts(&p), ["Hello", "fox Hello"]);
    for o in &objs {
        let Some(together) = &all[o.id] else {
            assert_ne!(o.kind, ObjectKind::Text);
            continue;
        };
        let alone = p.line_fonts(0, o.id).unwrap();
        assert_eq!(together.len(), alone.len());
        for (a, b) in together.iter().zip(&alone) {
            assert_eq!(a.refused(), b.refused());
            assert_eq!(a.preview_key(), b.preview_key());
            for c in "Helo fxXé€ß".chars() {
                assert_eq!(a.code(c), b.code(c), "{c:?}");
            }
        }
    }
}

/// A line in several parts is written on top of the page, its old shows taken out and the
/// rest of the page as it was; each part in its own font, the changed one through its codes.
#[test]
fn a_line_in_several_parts_is_written_on_top_without_its_old_shows() {
    let src = format!(
        "BT /F1 20 Tf 10 100 Td <{HELLO}> Tj /F2 20 Tf ( world) Tj ET\n\
         BT /F1 20 Tf 10 50 Td (fox) Tj ET\n"
    );
    let helvetica = unembedded("Helvetica", Some("WinAnsiEncoding"));
    let mut p = page(&[("F1", &sans_simple()), ("F2", &helvetica)], &src);
    let o = line_of(&p, "Hello world");
    let (before, fonts, progs) = (ops(&p), font_resources(&p), programs(&p));
    let parts = vec![typed(&o, 0, "Hallo"), typed(&o, 1, " world")];
    assert!(parts.iter().all(|t| kept(&p, &o, t)));
    assert!(write(&mut p, &o, parts).is_empty());
    let now = line_of(&p, "Hallo world");
    let want = [
        ("ABCDEF+LiberationSans".to_string(), (0, 5)),
        ("Helvetica".to_string(), (5, 11)),
    ];
    assert_eq!(styles(&now), want);
    assert_eq!(font_resources(&p), fonts);
    assert_eq!(programs(&p), progs);
    assert!(close(&now.stops[..1], &o.stops[..1]));
    assert_eq!(word_list(&p), ["Hallo", "world", "fox"]);
    // The page as it was without the line's two shows, then the line on top.
    let after = ops(&p);
    let top = after.iter().rposition(|o| o.operator == "q").unwrap();
    let mut page_ops = &after[..top];
    if page_ops.first().map(|o| o.operator.as_str()) == Some("q") {
        page_ops = &page_ops[1..page_ops.len() - 1];
    }
    let shows_at = [3, 5];
    let left: Vec<_> = (before.iter().enumerate())
        .filter(|(i, _)| !shows_at.contains(i))
        .map(|(_, o)| op_of(o))
        .collect();
    assert_eq!(page_ops.iter().map(op_of).collect::<Vec<_>>(), left);
    let on_top = names(&after[top..]);
    assert_eq!(on_top[..2], ["q", "BT"]);
    assert_eq!(on_top[on_top.len() - 2..], ["ET", "Q"]);
    let line = [
        ("F1".to_string(), vec![codes(b"Hallo")]),
        ("F2".to_string(), vec![codes(b" world")]),
    ];
    assert_eq!(shows(&p)[1..], line);
}

/// A line that wouldn't read back as its words, written in its own font, is written again in
/// another font, and `edit_texts` says so: here written in place, it would end where the next
/// show starts and run into it ("ABJJJJFar"). The next show stays where it was.
#[test]
fn a_line_that_would_read_as_other_words_is_written_in_another_font_and_said() {
    // "Far" starts 40 pt after "AB", which "JJJJ" fills exactly (500 thousandths an em each,
    // at 20 pt).
    let src = "BT /F1 20 Tf 10 100 Td (AB) Tj [-2000 (Far)] TJ ET\n";
    let mut p = page(&[("F1", &sans_simple())], src);
    let o = line_of(&p, "AB");
    let far = line_of(&p, "Far");
    let part = typed(&o, 0, "ABJJJJ");
    assert!(kept(&p, &o, &part));
    assert_eq!(write(&mut p, &o, vec![part]), [o.id]);
    let now = line_of(&p, "ABJJJJ");
    assert_ne!(now.spans[0].font_name, o.spans[0].font_name);
    assert_eq!(word_list(&p), ["ABJJJJ", "Far"]);
    assert!(close(&line_of(&p, "Far").bbox, &far.bbox));
    assert!(close(&now.stops[..2], &o.stops[..2]));
    let fonts = font_names(&p);
    assert_eq!(fonts.len(), 2, "{fonts:?}");
    assert!(fonts[1].starts_with("LimareEd"), "{fonts:?}");
}

/// The folder for one run of a Poppler tool, as the cross-checks run side by side.
fn run_dir() -> std::path::PathBuf {
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = format!("limarepdf-own-font-{}-{run}", std::process::id());
    let dir = std::env::temp_dir().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The text Poppler's `pdftotext` reads in the PDF `bytes`, its words one space apart.
fn poppler_text(bytes: &[u8]) -> String {
    let dir = run_dir();
    let pdf = dir.join("in.pdf");
    std::fs::write(&pdf, bytes).unwrap();
    let out = std::process::Command::new("pdftotext")
        .arg(&pdf)
        .arg("-")
        .output()
        .expect("pdftotext installed");
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(out.status.success(), "pdftotext failed");
    spaced(&String::from_utf8_lossy(&out.stdout))
}

/// `text`'s words one space apart.
fn spaced(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first page of the PDF `bytes` as Poppler's `pdftoppm` draws it in grey at 2 pixels a
/// point: its width, and a byte a pixel.
fn poppler_page(bytes: &[u8]) -> (usize, Vec<u8>) {
    let dir = run_dir();
    let pdf = dir.join("in.pdf");
    std::fs::write(&pdf, bytes).unwrap();
    let ok = std::process::Command::new("pdftoppm")
        .args(["-r", "144", "-gray", "-singlefile"])
        .arg(&pdf)
        .arg(dir.join("out"))
        .status()
        .expect("pdftoppm installed")
        .success();
    assert!(ok, "pdftoppm failed");
    let pgm = std::fs::read(dir.join("out.pgm")).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    // A binary PGM: "P5", width, height and the largest value, a space, then a byte a pixel.
    let mut fields = Vec::new();
    let mut at = 0;
    while fields.len() < 4 {
        while pgm[at].is_ascii_whitespace() {
            at += 1;
        }
        let from = at;
        while !pgm[at].is_ascii_whitespace() {
            at += 1;
        }
        fields.push(String::from_utf8_lossy(&pgm[from..at]).into_owned());
    }
    (fields[1].parse().unwrap(), pgm[at + 1..].to_vec())
}

/// Poppler reads an edited line as its words (where it reads the line as it was), and draws
/// the page as it was but for the line from its first changed character to its end, the
/// letters there drawing something (no blank glyphs where letters should be); with a
/// `reference`, the page written by hand as the edit should write it, exactly as that is
/// drawn (so no `.notdef` box either, which some fonts draw as a box like any other).
fn poppler_sees(why: &str, mut p: Pdf, o: &PageObject, part: TextPart, reference: Option<Pdf>) {
    let was_text = poppler_text(&p.to_bytes().unwrap());
    let (w, was) = poppler_page(&p.to_bytes().unwrap());
    let k = change_in(o, &part);
    let text = part.text.clone();
    assert!(write(&mut p, o, vec![part]).is_empty(), "{why}");
    let now = line_of(&p, &text);
    let bytes = p.to_bytes().unwrap();
    if was_text.contains(&spaced(o.text.as_deref().unwrap())) {
        let (read, want) = (poppler_text(&bytes), spaced(&text));
        assert!(read.contains(&want), "{why}: Poppler reads {read:?}");
    } else {
        eprintln!("{why}: Poppler reads {was_text:?} before the edit: text not compared");
    }
    let (_, is) = poppler_page(&bytes);
    assert_eq!(was.len(), is.len(), "{why}");
    // The changed area, in pixels, with a few to spare for edges drawn soft.
    let (bx, _) = o.baseline.unwrap();
    let x0 = 2.0 * (bx + o.stops[k]) - 4.0;
    let x1 = 2.0 * o.bbox[2].max(now.bbox[2]) + 4.0;
    let y0 = 2.0 * o.bbox[1].min(now.bbox[1]) - 4.0;
    let y1 = 2.0 * o.bbox[3].max(now.bbox[3]) + 4.0;
    let inside = |x: usize, y: usize| {
        let (x, y) = (x as f32, y as f32);
        x0 <= x && x <= x1 && y0 <= y && y <= y1
    };
    let outside = (0..was.len())
        .filter(|&i| was[i] != is[i] && !inside(i % w, i / w))
        .count();
    assert_eq!(outside, 0, "{why}: changed outside the line's change");
    let chars: Vec<char> = text.chars().collect();
    for j in (k..chars.len()).filter(|&j| !chars[j].is_whitespace()) {
        let (a, b) = (2.0 * (bx + now.stops[j]), 2.0 * (bx + now.stops[j + 1]));
        let ink = (0..is.len())
            .filter(|&i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                a <= x && x < b && y0 <= y && y <= y1 && is[i] < 128
            })
            .count();
        assert!(ink > 0, "{why}: {:?} draws nothing", chars[j]);
    }
    if let Some(mut r) = reference {
        let (_, want) = poppler_page(&r.to_bytes().unwrap());
        let off = (0..is.len()).filter(|&i| is[i] != want[i]).count();
        assert_eq!(off, 0, "{why}: not drawn as written by hand");
    }
}

/// Poppler reads and draws a typo fixed in each kind of font, and a word sent to another
/// font. Opt-in: `cargo test -p limarepdf-core --test own_font -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_edited_lines_with_poppler() {
    let have = |tool: &str| std::process::Command::new(tool).arg("-v").output().is_ok();
    if !have("pdftotext") || !have("pdftoppm") {
        eprintln!("pdftotext or pdftoppm (Poppler) isn't installed: nothing checked");
        return;
    }
    let latin_kinds = [
        simple_truetype(),
        simple_cff(),
        simple_opentype(),
        type0_truetype(true),
        type0_truetype(false),
        type0_cff(true),
        type0_cff(false),
    ];
    let own_kinds = [
        own_encoding_cff(),
        own_encoding_type1(),
        symbolic_truetype(),
    ];
    let scripts = (latin_kinds.iter().map(|k| (k, latin())))
        .chain(own_kinds.iter().map(|k| (k, limare_line())));
    for (kind, s) in scripts {
        for e in &s.edits {
            let p = script_page(kind, &s);
            let o = line_of(&p, &read(&s.line));
            let part = at(&o, 0, e.text, e.origins.clone());
            let by_hand = Script {
                line: e.shown.clone(),
                other: s.other,
                edits: Vec::new(),
            };
            let reference = Some(script_page(kind, &by_hand));
            let why = format!("{}, {}", kind.name, e.what);
            poppler_sees(&why, p, &o, part, reference);
        }
    }
    for kind in [type0_cff(true), type0_truetype(true)] {
        let p = script_page(&kind, &latin());
        let o = line_of(&p, "The quck fox");
        let part = typed(&o, 0, "The quick brown wren fox");
        let why = format!("{}, a word sent to another font", kind.name);
        poppler_sees(&why, p, &o, part, None);
    }
}

// ---- A paragraph edited across its lines (NEW-01, batch 45's 2b-2): each letter keeps its
// own code in the paragraph's font, typed ones that font's code, a word it can't write
// alone going to another font.

const PARAGRAPH: [&str; 4] = [
    "the harbour was quiet that morning and",
    "the fishing boats sat low in the water",
    "while gulls argued over the nets that",
    "dried along the wall of the old quay.",
];

/// A page with [`PARAGRAPH`] in `font` at 10 points, each line in a text object of its own,
/// its text as `hex` writes it.
fn paragraph_page(font: &dyn AddFont, hex: impl Fn(&str) -> String) -> Pdf {
    let line = |k: usize, l: &str| {
        let y = 150 - 12 * k as i32;
        format!("BT /F1 10 Tf 10 {y} Td <{}> Tj ET\n", hex(l))
    };
    let content: String = (PARAGRAPH.iter().enumerate())
        .map(|(k, l)| line(k, l))
        .collect();
    page(&[("F1", font)], &content)
}

/// Page 0's only block edited as a paragraph to `new`: the characters it starts and ends
/// with unchanged kept where they were, those typed in the style before them.
fn edit_paragraph_to(p: &mut Pdf, new: &str) -> Vec<usize> {
    let blocks = p.page_blocks(0).unwrap();
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    let para = p.paragraph(0, &blocks[0], &p.document_words()).unwrap();
    let setting = p.paragraph_setting(&para).unwrap();
    let (old, new): (Vec<char>, Vec<char>) = (para.text.chars().collect(), new.chars().collect());
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - head;
    let tail = (old.iter().rev().zip(new.iter().rev()))
        .take_while(|(a, b)| a == b)
        .count()
        .min(room);
    let mut parts: Vec<TextPart> = Vec::new();
    let mut span = 0;
    for (j, &c) in new.iter().enumerate() {
        let origin = match j {
            _ if j < head => Some(j),
            _ if j >= new.len() - tail => Some(old.len() + j - new.len()),
            _ => None,
        };
        if let Some(k) = origin {
            span = para.span_at(k).unwrap();
        }
        let s = &para.spans[span];
        match parts.last_mut() {
            Some(p) if p.font == PartFont::Span(span) => {
                p.text.push(c);
                p.origins.push(origin);
            }
            _ => parts.push(TextPart {
                text: c.to_string(),
                font: PartFont::Span(span),
                size: s.font_size,
                color: s.fill.unwrap_or([0.0; 3]),
                rise: s.rise,
                from: None,
                origins: vec![origin],
            }),
        }
    }
    p.edit_paragraph(&setting, &parts).unwrap()
}

/// The bytes page 0 shows in font resource `font`, one string after another.
fn shown_in(p: &Pdf, font: &str) -> Vec<u8> {
    let page = p.page_ids()[0];
    let content = Content::decode(&p.doc.get_page_content(page)).unwrap();
    let mut out = Vec::new();
    let mut now: Vec<u8> = Vec::new();
    for o in content.operations {
        match o.operator.as_str() {
            "Tf" => now = o.operands[0].as_name().unwrap().to_vec(),
            "Tj" | "TJ" if now == font.as_bytes() => {
                let items = match o.operator.as_str() {
                    "TJ" => o.operands[0].as_array().unwrap().clone(),
                    _ => o.operands.clone(),
                };
                for it in items {
                    if let Object::String(b, _) = it {
                        out.extend(b);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[test]
fn a_paragraph_in_a_subset_truetype_font_keeps_its_codes_across_its_lines() {
    let ascii = |s: &str| s.bytes().map(|b| format!("{b:02X}")).collect::<String>();
    let mut p = paragraph_page(&sans_simple(), ascii);
    let joined = PARAGRAPH.join(" ");
    let new = joined.replacen("quiet", "very quiet and still", 1);
    let lost = edit_paragraph_to(&mut p, &new);
    assert!(lost.is_empty(), "{lost:?}");
    // Every letter in the paragraph's own font, nothing added to the page's fonts.
    assert_eq!(font_names(&p), ["F1"]);
    // (The spaces lines end with hang, unwritten.)
    let shown: String = shown_in(&p, "F1").iter().map(|&b| b as char).collect();
    assert_eq!(shown.replace(' ', ""), new.replace(' ', ""));
    assert_eq!(texts(&p).join(" "), new);
}

#[test]
fn a_paragraph_in_a_cid_font_keeps_its_codes_and_sends_a_word_it_cant_write_away() {
    // Liberation Sans as Identity-H, mapping only the paragraph's own letters.
    let letters: String = {
        let mut l: Vec<char> = PARAGRAPH.concat().chars().collect();
        l.sort_unstable();
        l.dedup();
        l.into_iter().collect()
    };
    let font = sans_type0(&letters, true);
    let mut p = paragraph_page(&font, |s| hex2(&sans_codes(s)));
    // "tall" is written in its letters' codes; "Zebra" has a "Z" the font lacks.
    let joined = PARAGRAPH.join(" ");
    let new = joined.replacen("boats", "tall boats near Zebra", 1);
    let lost = edit_paragraph_to(&mut p, &new);
    assert!(lost.is_empty(), "{lost:?}");
    let read = texts(&p).join(" ");
    assert_eq!(read.split_whitespace().collect::<Vec<_>>().join(" "), new);
    let shown = shown_in(&p, "F1");
    let tall: Vec<u8> = sans_codes("tall").into_iter().flat_map(two).collect();
    let has = |codes: &[u8]| shown.windows(codes.len()).any(|w| w == codes);
    assert!(has(&tall), "\"tall\" not in the font's codes");
    // "Zebra" went to another font, added to the page; nothing else did.
    let names = font_names(&p);
    assert_eq!(names.len(), 2, "{names:?}");
    let zebra: Vec<u8> = sans_codes("ebra").into_iter().flat_map(two).collect();
    assert!(!has(&zebra), "\"Zebra\" kept in the font");
}
