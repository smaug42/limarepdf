//! Simple embedded fonts without a ToUnicode map read through the encoding of their own
//! program where the PDF gives no base encoding (PB-23), as poppler reads them: TeX's fonts
//! put ” at 0x22 and – at 0x7B, where the standard encoding has " and {.

use limarepdf_core::Pdf;
use lopdf::{dictionary, Dictionary, Object, Stream};

const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/fonts/");

/// The test fonts' codes: ff, fi, ”, A, “, – and —.
const SHOWN: &str = "<0B0C22415C7B7C>";
const READ: &str = "fffi”A“–—";

/// A page showing `content` in `/F1`, a font of `subtype` with `encoding` (if any) and its
/// program `file` (if any) under `key` of its descriptor, with `extra` in the program's
/// dictionary. `flags` are the descriptor's.
fn page(
    subtype: &str,
    file: Option<(&str, &str, Dictionary)>,
    encoding: Option<Object>,
    flags: i64,
    content: &str,
) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let mut desc = dictionary! {
        "Type" => "FontDescriptor", "FontName" => "LimareOwnEncoding", "Flags" => flags,
        "FontBBox" => vec![0.into(), 0.into(), 500.into(), 700.into()], "ItalicAngle" => 0,
        "Ascent" => 700, "Descent" => 0, "CapHeight" => 700, "StemV" => 80,
    };
    if let Some((name, key, extra)) = file {
        let data = std::fs::read(format!("{FONTS}{name}")).unwrap();
        let program = p.doc.add_object(Stream::new(extra, data));
        desc.set(key, program);
    }
    let desc = p.doc.add_object(desc);
    let mut font = dictionary! {
        "Type" => "Font", "Subtype" => subtype, "BaseFont" => "LimareOwnEncoding",
        "FirstChar" => 0, "LastChar" => 255,
        "Widths" => vec![Object::Integer(500); 256], "FontDescriptor" => desc,
    };
    if let Some(e) = encoding {
        font.set("Encoding", e);
    }
    let font = p.doc.add_object(font);
    let page = p.page_ids()[0];
    let s = p.doc.add_object(Stream::new(
        Dictionary::new(),
        format!("BT /F1 20 Tf 10 100 Td {content} Tj ET").into_bytes(),
    ));
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", s);
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    p
}

fn type1(encoding: Option<Object>, content: &str) -> Pdf {
    let len = std::fs::read(format!("{FONTS}LimareOwnEncoding.pfa"))
        .unwrap()
        .len() as i64;
    let lengths = dictionary! { "Length1" => len, "Length2" => 0, "Length3" => 0 };
    let file = ("LimareOwnEncoding.pfa", "FontFile", lengths);
    page("Type1", Some(file), encoding, 4, content)
}

fn text(p: &Pdf) -> String {
    p.page_text(0).trim().to_string()
}

#[test]
fn a_type1_program_s_own_encoding_is_read_without_a_pdf_encoding() {
    assert_eq!(text(&type1(None, SHOWN)), READ);
}

#[test]
fn differences_without_a_base_apply_to_the_program_s_own_encoding() {
    let enc = dictionary! { "Differences" => vec![65.into(), "B".into()] };
    assert_eq!(text(&type1(Some(enc.into()), SHOWN)), "fffi”B“–—");
}

#[test]
fn a_named_encoding_still_wins_over_the_program_s() {
    let p = type1(Some("WinAnsiEncoding".into()), "<22415C7B>");
    assert_eq!(text(&p), "\"A\\{");
    let enc = dictionary! {
        "BaseEncoding" => "WinAnsiEncoding", "Differences" => vec![66.into(), "C".into()],
    };
    assert_eq!(text(&type1(Some(enc.into()), "<224142>")), "\"AC");
    // A name no list has (`g12`) leaves the others in place (lopdf drops them all).
    let enc = dictionary! {
        "Type" => "Encoding", "BaseEncoding" => "WinAnsiEncoding",
        "Differences" => vec![65.into(), "g12".into(), "C".into()],
    };
    assert_eq!(text(&type1(Some(enc.into()), "<2242>")), "\"C");
}

#[test]
fn a_cff_program_s_own_encoding_is_read_without_a_pdf_encoding() {
    let file = (
        "LimareOwnEncoding.cff",
        "FontFile3",
        dictionary! { "Subtype" => "Type1C" },
    );
    assert_eq!(text(&page("Type1", Some(file), None, 4, SHOWN)), READ);
}

#[test]
fn a_symbolic_truetype_program_s_glyph_names_are_read_without_a_pdf_encoding() {
    let file = ("LimareSymbolic.ttf", "FontFile2", Dictionary::new());
    assert_eq!(text(&page("TrueType", Some(file), None, 4, SHOWN)), READ);
}

/// A page showing `content` in `/F1`, the standard font `base` without a program, with
/// `encoding` if any.
fn standard(base: &str, encoding: Option<Object>, content: &str) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let mut font = dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base };
    if let Some(e) = encoding {
        font.set("Encoding", e);
    }
    let font = p.doc.add_object(font);
    let page = p.page_ids()[0];
    let s = p.doc.add_object(Stream::new(
        Dictionary::new(),
        format!("BT /F1 20 Tf 10 100 Td {content} Tj ET").into_bytes(),
    ));
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", s);
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    p
}

/// Symbol and ZapfDingbats have encodings of their own, which a PDF without an `/Encoding`
/// for them means (PB-25): their Greek letters, maths and dingbats read as such, not as the
/// Latin letters at those codes in the standard encoding.
#[test]
fn symbol_and_zapfdingbats_read_by_their_own_encodings() {
    assert_eq!(text(&standard("Symbol", None, "(abg \\042)")), "αβγ ∀");
    assert_eq!(text(&standard("ZapfDingbats", None, "(4n)")), "✔■");
    // Differences over it, without a base.
    let enc = dictionary! { "Differences" => vec![97.into(), "Psi".into()] };
    assert_eq!(text(&standard("Symbol", Some(enc.into()), "(ab)")), "Ψβ");
    // A named encoding is still the PDF's.
    let p = standard("Symbol", Some("WinAnsiEncoding".into()), "(ab)");
    assert_eq!(text(&p), "ab");
}

#[test]
fn a_font_without_a_program_reads_by_the_standard_encoding_as_before() {
    assert_eq!(text(&page("Type1", None, None, 32, "<2241>")), "\"A");
}
