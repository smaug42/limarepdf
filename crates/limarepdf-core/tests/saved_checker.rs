//! The saved-output checker itself (`tests/common/saved.rs`): a marker written into page
//! content in any way a reader still reads it is found.

mod common;

use common::saved::{tools, Saved};
use limarepdf_core::Pdf;
use lopdf::{dictionary, Dictionary, Object, Stream, StringFormat};

/// A one-page 300×200 pt document drawing `content` with `/F1` (Helvetica), saved.
fn saved(content: &str) -> Saved {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let page = p.page_ids()[0];
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", Object::Reference(s));
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    Saved::of(&mut p)
}

fn show(text: &str) -> String {
    format!("BT /F1 12 Tf 20 100 Td {text} ET")
}

/// The ways of writing SECRET (and, for the escaped delimiters, S(E)CRET) that the raw bytes
/// don't hold as such.
const HIDDEN: [(&str, &str, &str); 7] = [
    ("octal escape", "SECRET", r"(SE\103RET) Tj"),
    ("all octal", "SECRET", r"(\123\105\103\122\105\124) Tj"),
    ("escaped delimiters", "S(E)CRET", r"(S\(E\)CRET) Tj"),
    ("line continuation", "SECRET", "(SEC\\\nRET) Tj"),
    ("hex with spaces", "SECRET", "<53 45 43 52 45 54> Tj"),
    ("split in TJ", "SECRET", "[(SEC) -0 (RET)] TJ"),
    ("split in TJ, hex", "SECRET", "[<534543> 120 (R\\105T)] TJ"),
];

/// The forms the checker found before it read content.
const PLAIN: [(&str, &str, &str); 4] = [
    ("plain", "SECRET", "(SECRET) Tj"),
    ("hex", "SECRET", "<534543524554> Tj"),
    ("lower-case hex", "SECRET", "<736563726574> Tj"),
    ("UTF-16 hex", "SECRET", "<FEFF005300450043005200450054> Tj"),
];

#[test]
fn strings_in_content_are_found_however_written() {
    let missed: Vec<&str> = HIDDEN
        .iter()
        .filter(|(_, marker, text)| saved(&show(text)).places(marker).is_empty())
        .map(|(what, ..)| *what)
        .collect();
    assert!(missed.is_empty(), "not found: {missed:?}");
}

#[test]
fn plain_hex_and_utf16_strings_are_still_found() {
    for (what, marker, text) in PLAIN {
        let marker = if what == "lower-case hex" {
            "secret"
        } else {
            marker
        };
        let s = saved(&show(text));
        assert!(
            !s.places(marker).is_empty(),
            "{what}: {marker} not found in {text}"
        );
    }
}

#[test]
fn text_that_is_not_there_is_not_found() {
    // The parts of a split marker, in two text objects, and strings that only share letters.
    let s = saved(&format!(
        "{} {}",
        show("[(SEC) -0 (ret)] TJ"),
        show("(RET) Tj (SE\\103) Tj")
    ));
    assert!(s.places("SECRET").is_empty());
    // A segment that isn't content is still searched as bytes.
    let s = saved("not ( content SECRET");
    assert!(!s.places("SECRET").is_empty());
}

/// `text` as a PDF text string in UTF-16BE with its byte order mark, as ActualText and Alt
/// often are.
fn utf16(text: &str) -> Vec<u8> {
    let mut out = vec![0xFE, 0xFF];
    out.extend(text.encode_utf16().flat_map(|u| u.to_be_bytes()));
    out
}

/// A saved one-page document whose page dictionary also holds `key` set to `value`.
fn saved_with(key: &str, value: Object) -> Saved {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let page = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set(key.as_bytes().to_vec(), value);
    Saved::of(&mut p)
}

#[test]
fn utf16_text_strings_in_objects_are_found() {
    for format in [StringFormat::Literal, StringFormat::Hexadecimal] {
        let s = saved_with("Alt", Object::String(utf16("SECRET"), format));
        assert!(!s.places("SECRET").is_empty(), "{format:?}: not found");
    }
}

#[test]
fn utf16_actual_text_in_content_is_found() {
    let hex: String = utf16("SECRET").iter().map(|b| format!("{b:02X}")).collect();
    let s = saved(&format!(
        "/Span << /ActualText <{hex}> >> BDC {} EMC",
        show("(xxxxxx) Tj")
    ));
    assert!(!s.places("SECRET").is_empty());
}

#[test]
fn names_are_found_written_plainly_or_escaped() {
    // lopdf writes a space in a name as `#20`, as other writers may write any character.
    for name in ["SECRET", "SEC RET"] {
        let s = saved_with("Tag", Object::Name(name.as_bytes().to_vec()));
        assert!(!s.places(name).is_empty(), "the name {name:?} not found");
        let s = saved_with(name, Object::Integer(1));
        assert!(!s.places(name).is_empty(), "the key {name:?} not found");
    }
}

/// Every hidden form is one another reader finds, so the checker must too.
#[test]
#[ignore = "needs pdftotext; run with -- --ignored cross_check"]
fn cross_check_readers_find_every_form() {
    for (what, marker, text) in HIDDEN {
        let s = saved(&show(text));
        let read = tools::pdftotext(&s.bytes);
        assert!(read.contains(marker), "{what}: pdftotext reads {read:?}");
    }
}
