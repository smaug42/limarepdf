//! Text strings as other programs write them: PDFDocEncoding, without a byte order mark.
//! The file is built from raw bytes, not through LimarePDF's own writer, then read through
//! every place that shows such text (properties, bookmarks, form names, values and
//! descriptions, comments and page labels), edited and saved.

use limarepdf_core::{FieldValue, Info, Pdf};
use lopdf::{dictionary, Object, StringFormat};

/// `Budget • €100 — “approved”` in PDFDocEncoding.
const BUDGET: &[u8] = b"Budget \x80 \xa0100 \x84 \x8dapproved\x8e";
const BUDGET_TEXT: &str = "Budget • €100 — “approved”";

fn raw(b: &[u8]) -> Object {
    Object::String(b.to_vec(), StringFormat::Literal)
}

/// A one-page file with PDFDocEncoding text in each place, saved and opened again.
fn file() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let page = p.page_ids()[0];
    let info = p.doc.add_object(dictionary! {
        "Title" => raw(BUDGET),
        "Author" => raw(b"Ren\xe9e \x95ukasz"),
    });
    p.doc.trailer.set("Info", info);

    let outline = p.doc.new_object_id();
    let item = p.doc.add_object(dictionary! {
        "Title" => raw(BUDGET),
        "Parent" => outline,
        "Dest" => vec![page.into(), "Fit".into()],
    });
    p.doc.objects.insert(
        outline,
        dictionary! { "Type" => "Outlines", "First" => item, "Last" => item, "Count" => 1 }.into(),
    );

    let field = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
        "T" => raw(b"Caf\xe9 \x85 total"),
        "TU" => raw(b"\x8dAmount\x8e \x81"),
        "V" => raw(BUDGET),
        "Rect" => vec![10.into(), 10.into(), 200.into(), 30.into()],
        "P" => page,
    });
    let note = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Text",
        "Contents" => raw(BUDGET),
        "T" => raw(b"\x95ukasz"),
        "Rect" => vec![50.into(), 150.into(), 70.into(), 170.into()],
    });
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", vec![field.into(), note.into()]);
    let fields: Vec<Object> = vec![field.into()];
    let form = p.doc.add_object(dictionary! { "Fields" => fields });

    let labels = dictionary! {
        "Nums" => vec![0.into(), dictionary! { "S" => "D", "P" => raw(b"Annex \x84 ") }.into()],
    };
    let cat = p.doc.catalog_mut().unwrap();
    cat.set("Outlines", outline);
    cat.set("AcroForm", form);
    cat.set("PageLabels", labels);
    reopened(&mut p)
}

fn reopened(p: &mut Pdf) -> Pdf {
    Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap()
}

/// The raw bytes of the document's title.
fn title_bytes(p: &Pdf) -> Vec<u8> {
    let info = p.doc.trailer.get(b"Info").unwrap();
    let (_, info) = p.doc.dereference(info).unwrap();
    let title = info.as_dict().unwrap().get(b"Title").unwrap();
    let (_, title) = p.doc.dereference(title).unwrap();
    title.as_str().unwrap().to_vec()
}

#[test]
fn every_place_reads_pdfdocencoding() {
    let p = file();
    let info = p.info();
    assert_eq!(info.title.as_deref(), Some(BUDGET_TEXT));
    assert_eq!(info.author.as_deref(), Some("Renée Łukasz"));
    assert_eq!(p.outline()[0].title, BUDGET_TEXT);
    let fields = p.form_fields();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "Café – total");
    assert_eq!(fields[0].value, FieldValue::Text(BUDGET_TEXT.into()));
    assert_eq!(fields[0].tooltip.as_deref(), Some("“Amount” †"));
    let note = p
        .annotations()
        .into_iter()
        .find(|a| a.author == "Łukasz")
        .expect("the comment, by its author");
    assert_eq!(note.contents, BUDGET_TEXT);
    assert_eq!(p.page_labels().unwrap(), ["Annex — 1"]);
}

#[test]
fn edits_and_saves_keep_the_characters() {
    let mut p = file();
    // The field is found by the name as read, and the new value reads back.
    let paid = FieldValue::Text("€5 • paid “in full”".into());
    p.set_field("Café – total", paid.clone()).unwrap();
    let note = p.annotations().into_iter().find(|a| a.author == "Łukasz");
    let id = note.unwrap().id;
    p.set_annotation(id, None, 1.0, "Note — €20").unwrap();
    p.rename_bookmark(0, "Budget • €200").unwrap();
    let mut p = reopened(&mut p);
    let f = &p.form_fields()[0];
    assert_eq!(f.name, "Café – total");
    assert_eq!(f.value, paid);
    let note = p.annotations().into_iter().find(|a| a.id == id).unwrap();
    assert_eq!(note.contents, "Note — €20");
    assert_eq!(p.outline()[0].title, "Budget • €200");
    // Text nobody changed is written back byte for byte.
    assert_eq!(title_bytes(&p), BUDGET);
    assert_eq!(p.info().title.as_deref(), Some(BUDGET_TEXT));

    let info = Info {
        title: Some("Budget • €300 — “final”".into()),
        ..p.info()
    };
    p.set_info(&info).unwrap();
    let p = reopened(&mut p);
    assert_eq!(p.info().title.as_deref(), Some("Budget • €300 — “final”"));
    assert_eq!(p.info().author.as_deref(), Some("Renée Łukasz"));
}

/// Poppler reads the made-up file's title the same way. Opt-in: needs `pdfinfo`.
#[test]
#[ignore]
fn cross_check_pdfdocencoding_with_poppler() {
    let dir = std::env::temp_dir().join(format!("limarepdf-pdfdoc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pdfdoc.pdf");
    std::fs::write(&path, file().to_bytes().unwrap()).unwrap();
    let out = std::process::Command::new("pdfinfo")
        .args(["-enc", "UTF-8"])
        .arg(&path)
        .output()
        .expect("pdfinfo");
    let out = String::from_utf8(out.stdout).unwrap();
    let title = out
        .lines()
        .find_map(|l| l.strip_prefix("Title:"))
        .expect("a title line")
        .trim();
    assert_eq!(title, file().info().title.unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}
