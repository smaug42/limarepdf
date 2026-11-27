//! Operations that go through many pages find each page in the page tree once, not by its
//! number each time, which reads the tree again up to that page (PB-29): on a long document
//! that is the page count squared. Each operation is counted ([`pages_seam`]) on a made-up
//! long document and may read the tree a few times over, not once a page.
//!
//! The seam counts entries read on any thread, so the operations are counted in a child
//! process, where no other test runs, one operation at a time.

use std::time::Duration;

use crate::common;

use common::apart::{self, Ran};
use common::tool::Scratch;
use limarepdf_core::print::{Orientation, PageSetup, Paper, PerSheet, Scale};
use limarepdf_core::{
    pages_seam, sign_document, HeaderFooter, Hidden, IdStore, ImageWatermark, LinkId, Ocr, Pdf,
    ReduceOptions, ScanOptions, SignPlace, SignRequest, Watermark,
};
use lopdf::{dictionary, Document, Object, Stream};

/// The made-up document's pages.
const PAGES: usize = 2_000;

/// The links written inside the last page's `/Annots`.
const LINKS: usize = 100;

/// How many times over an operation may read the page tree: a few whole walks (applying
/// redactions takes 15, each step finding the pages once). Finding each page by its number
/// reads about half the tree a page, here a thousand times over.
const WALKS: u64 = 20;

/// The child's time to count every operation, which takes 10 s in a dev build.
const LIMIT: Duration = Duration::from_secs(120);

/// `PAGES` pages, each with a line of text ending "Secret", a redaction mark over it and a
/// text field of its own; the last page has `LINKS` links written inside its `/Annots`.
fn long_document() -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let tree = doc.new_object_id();
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let mut fields = Vec::new();
    let kids: Vec<Object> = (0..PAGES)
        .map(|i| {
            let page = doc.new_object_id();
            let text = format!(
                "BT /F1 12 Tf 30 100 Td (Page {} keeps this line Secret) Tj ET",
                i + 1
            );
            let content = doc.add_object(Stream::new(dictionary! {}, text.into_bytes()));
            let mark = doc.add_object(dictionary! {
                "Type" => "Annot", "Subtype" => "Redact", "P" => page,
                "Rect" => vec![25.into(), 95.into(), 300.into(), 115.into()],
            });
            let field = doc.add_object(dictionary! {
                "Type" => "Annot", "Subtype" => "Widget", "P" => page, "F" => 4,
                "FT" => "Tx", "T" => Object::string_literal(format!("f{i}")),
                "V" => Object::string_literal(format!("value {i}")),
                "Rect" => vec![300.into(), 700.into(), 500.into(), 720.into()],
            });
            fields.push(Object::Reference(field));
            let mut annots: Vec<Object> = vec![mark.into(), field.into()];
            if i == PAGES - 1 {
                annots.extend((0..LINKS).map(|n| {
                    let y = 200 + 4 * n as i64;
                    Object::Dictionary(dictionary! {
                        "Type" => "Annot", "Subtype" => "Link",
                        "Rect" => vec![30.into(), y.into(), 200.into(), (y + 3).into()],
                        "A" => dictionary! {
                            "S" => "URI",
                            "URI" => Object::string_literal(format!("https://example.com/{n}")),
                        },
                    })
                }));
            }
            doc.objects.insert(
                page,
                dictionary! {
                    "Type" => "Page", "Parent" => tree, "Contents" => content,
                    "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                    "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                    "Annots" => annots,
                }
                .into(),
            );
            page.into()
        })
        .collect();
    doc.objects.insert(
        tree,
        dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => PAGES as i64 }.into(),
    );
    let form = doc.add_object(dictionary! {
        "Fields" => fields, "DA" => Object::string_literal("/Helv 10 Tf 0 g"),
    });
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => tree, "AcroForm" => form,
    });
    doc.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

/// Letter paper with no margins: a sheet is its page as it is.
fn as_is() -> PageSetup {
    PageSetup {
        paper: Paper {
            id: "letter".into(),
            label: "Letter".into(),
            size: (612.0, 792.0),
            margins: Some([0.0; 4]),
        },
        orientation: Orientation::Automatic,
        scale: Scale::ShrinkOversized,
        per_sheet: PerSheet::default(),
        booklet: None,
    }
}

#[test]
fn operations_on_every_page_read_the_page_tree_a_few_times_not_once_a_page() {
    let env = [("PAGE_WALKS_CHILD", "1".to_string())];
    let ran = apart::run(module_path!(), "page_walks_child", &env, LIMIT);
    let Ran::Ended(text, status) = ran else {
        panic!("not done within {} s", LIMIT.as_secs());
    };
    let said = apart::reports(&text, "walks|");
    let Some((&"done", counted)) = said.split_last() else {
        panic!("{}", apart::why_stopped(status));
    };
    let mut over = Vec::new();
    for l in counted {
        let (what, read) = l.rsplit_once('|').unwrap();
        let read: u64 = read.parse().unwrap();
        let times = read as f64 / PAGES as f64;
        println!("{what}: {read} entries, {times:.1} times the pages");
        if read > WALKS * PAGES as u64 {
            over.push(format!("{what}: {read}"));
        }
    }
    assert!(
        over.is_empty(),
        "read the page tree more than {WALKS} times over ({} entries): {over:?}",
        WALKS * PAGES as u64
    );
}

/// The child process of the test above: each operation's page-tree entries read, on a line.
#[test]
#[ignore = "started by operations_on_every_page_read_the_page_tree_a_few_times_not_once_a_page"]
fn page_walks_child() {
    if std::env::var("PAGE_WALKS_CHILD").is_err() {
        return;
    }
    let bytes = long_document();
    let open = || Pdf::from_bytes(&bytes, None).unwrap();
    let all: Vec<usize> = (0..PAGES).collect();
    let every_other: Vec<usize> = (0..PAGES).step_by(2).collect();
    let last = PAGES - 1;
    let picture = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/images/cmyk-cyan-red.jpg"),
    )
    .unwrap();
    let dir = Scratch::new("page-walks");
    let store = IdStore::new(dir.join("ids"));
    let p12 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/signatures/ids/modern.p12");
    let imported = store
        .import(&std::fs::read(p12).unwrap(), "mpe-test-password", None)
        .unwrap();
    let id = store.unlock(&imported.id, "mpe-test-password").unwrap();
    let ocr = Ocr::bundled().unwrap();

    // Each operation on a document of its own, opened before counting.
    let check = |what: &str, op: &dyn Fn(&mut Pdf)| {
        let mut p = open();
        let ((), read) = pages_seam::count(|| op(&mut p));
        println!("walks|{what}|{read}");
    };
    check("finding redaction marks", &|p| {
        assert_eq!(p.find_redaction_marks("Secret").unwrap().len(), PAGES);
    });
    check("redacting every page", &|p| {
        let marks: Vec<_> = (0..PAGES)
            .map(|i| (i, [25.0, 677.0, 300.0, 697.0]))
            .collect();
        p.redact(&marks).unwrap();
    });
    check("applying the redaction marks", &|p| {
        p.apply_redactions().unwrap();
    });
    check("removing hidden information", &|p| {
        p.remove_hidden(&Hidden::ALL).unwrap();
    });
    check("listing the form's fields", &|p| {
        assert_eq!(p.form_fields().len(), PAGES);
    });
    check("flattening the form", &|p| {
        p.flatten_form(None).unwrap();
    });
    check("listing the comments", &|p| {
        p.annotations();
    });
    check("searching", &|p| {
        assert_eq!(p.search("Secret").unwrap().len(), PAGES);
    });
    check("extracting the text", &|p| {
        p.pages_text(&all);
    });
    check("reading the document's words", &|p| {
        p.document_words();
    });
    check("recognising text", &|p| {
        p.ocr_pages(&all, &ocr).unwrap();
    });
    check("optimising scans", &|p| {
        p.optimize_scans(&all, &ScanOptions::default()).unwrap();
    });
    check("reducing pictures", &|p| {
        p.reduce_pictures(&all, &ReduceOptions::default()).unwrap();
    });
    check("previewing reduced pictures", &|p| {
        p.reduce_preview(&all, &ReduceOptions::default()).unwrap();
    });
    check("flattening the comments", &|p| {
        p.flatten_annotations(None).unwrap();
    });
    check("finding fields that won't print or show", &|p| {
        p.unprintable_fields(&all);
        p.unshown_fields(&all);
    });
    check("turning every page", &|p| {
        p.rotate_pages(&all, 90).unwrap();
    });
    check("extracting every other page", &|p| {
        p.extract(&every_other).unwrap();
    });
    check("deleting every other page", &|p| {
        p.delete_pages(&every_other).unwrap();
    });
    check("adding a watermark", &|p| {
        p.add_watermark(&all, &Watermark::default()).unwrap();
    });
    check("adding a picture watermark", &|p| {
        p.add_image_watermark(&all, &ImageWatermark::new(picture.clone()))
            .unwrap();
    });
    check("adding page numbers", &|p| {
        let hf = HeaderFooter {
            footer: [String::new(), "{page} of {pages}".into(), String::new()],
            ..HeaderFooter::default()
        };
        p.add_header_footer(&all, &hf).unwrap();
    });
    check("printing every page", &|p| {
        p.print_sheets(&all, &as_is()).unwrap();
    });
    check("removing the last page's links", &|p| {
        let links: Vec<LinkId> = p.page_links(last).unwrap().iter().map(|l| l.id).collect();
        assert_eq!(links.len(), LINKS);
        p.remove_links(last, &links).unwrap();
    });
    check("signing on the last page", &|_| {
        let req = SignRequest {
            id: &id,
            place: SignPlace::NewBox {
                page: last,
                rect: [20.0, 500.0, 220.0, 550.0],
            },
            reason: None,
            location: None,
            certify: None,
            art: None,
            accept_disallowed: false,
        };
        sign_document(&bytes, None, &req).unwrap();
    });
    println!("walks|done");
}
