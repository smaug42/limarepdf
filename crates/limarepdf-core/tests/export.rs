//! Exporting pages as images and text.

use limarepdf_core::render::{image_names, ImageFormat};
use limarepdf_core::{ImageImport, Pdf};
use lopdf::{dictionary, Dictionary, Stream};
use std::path::PathBuf;

/// A fresh, empty directory under Cargo's temporary directory for integration tests.
fn out_dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("export-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Two pages: Letter with a black 100 pt square at the bottom left, then 4 x 3 inches.
fn two_pages() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    p.insert_blank(1, 288.0, 216.0).unwrap();
    let page = p.page_ids()[0];
    let cs = p.doc.add_object(Stream::new(
        Dictionary::new(),
        b"0 0 0 rg 0 0 100 100 re f".to_vec(),
    ));
    p.doc.get_dictionary_mut(page).unwrap().set("Contents", cs);
    p
}

fn round_trip(format: ImageFormat) {
    let mut p = two_pages();
    let dir = out_dir(format.extension());
    let files = p.export_images(&[0, 1], 100.0, format, &dir, "doc");
    let ext = format.extension();
    let want = ["doc-001", "doc-002"].map(|n| dir.join(format!("{n}.{ext}")));
    assert_eq!(files.unwrap(), want);

    let img = image::open(&want[0]).unwrap().to_rgb8();
    // 612 x 792 pt at 100 dpi.
    assert_eq!(img.dimensions(), (850, 1100));
    // The square is 100 pt = 139 px, at the bottom left; the rest of the page is white.
    let (square, blank) = (img.get_pixel(50, 1050), img.get_pixel(400, 400));
    assert!(square[0] < 40 && blank[0] > 215, "{square:?} {blank:?}");

    // The files record their resolution, so they import back at the pages' size.
    let bytes: Vec<Vec<u8>> = want.iter().map(|f| std::fs::read(f).unwrap()).collect();
    let back = Pdf::from_images(&bytes, &ImageImport::default()).unwrap();
    for (i, (w, h)) in [(612.0, 792.0), (288.0, 216.0)].into_iter().enumerate() {
        let (bw, bh) = back.page_size(i).unwrap();
        let close = (bw - w).abs() < 1.0 && (bh - h).abs() < 1.0;
        assert!(close, "page {i}: {bw} x {bh}");
    }
}

#[test]
fn exports_png_pages_that_import_back_at_size() {
    round_trip(ImageFormat::Png);
}

#[test]
fn exports_jpeg_pages_that_import_back_at_size() {
    round_trip(ImageFormat::Jpeg(90));
}

#[test]
fn image_names_sort_in_page_order() {
    let png = ImageFormat::Png;
    assert_eq!(image_names(&[0, 9], "a", png), ["a-001.png", "a-010.png"]);
    let jpg = ImageFormat::Jpeg(80);
    let long = image_names(&[4, 1499], "b", jpg);
    assert_eq!(long, ["b-0005.jpg", "b-1500.jpg"]);
}

#[test]
fn rejects_pages_that_do_not_exist() {
    let mut p = two_pages();
    let dir = out_dir("missing");
    assert!(p
        .export_images(&[0, 5], 72.0, ImageFormat::Png, &dir, "x")
        .is_err());
}

#[test]
fn pages_text_separates_pages_with_form_feeds() {
    let mut p = Pdf::new();
    for (i, word) in ["Alpha", "Beta"].into_iter().enumerate() {
        p.insert_blank(i, 300.0, 300.0).unwrap();
        let page = p.page_ids()[i];
        let font = p.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let content = format!("BT /F1 12 Tf 20 200 Td ({word}) Tj ET");
        let cs = p
            .doc
            .add_object(Stream::new(Dictionary::new(), content.into_bytes()));
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", cs);
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
    }
    let text = p.pages_text(&[1, 0]);
    let parts: Vec<&str> = text.split('\u{c}').map(str::trim).collect();
    assert_eq!(parts, ["Beta", "Alpha"]);
}
