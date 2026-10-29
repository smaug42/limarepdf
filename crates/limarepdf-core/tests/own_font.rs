//! Which code writes a character in the font a line of text is already in (ENH-31, stage 1):
//! a code that reads as the character, whose glyph the embedded program has and draws, whose
//! width the PDF gives as the program does, and that neither the encoding nor the program's
//! own names contradict. Fonts that can't take new text say why, and so does each character
//! that can't be written. A Type 0 TrueType font with no ToUnicode map reads through its
//! program's cmap (PB-26).

use limarepdf_core::CharRefusal::{Ambiguous, Contradicted, Encoding, NoCode, NoGlyph, Width};
use limarepdf_core::{
    bundled_face, CharRefusal, FontChoice, FontRefusal, LineFont, ObjectKind, Pdf,
};
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
