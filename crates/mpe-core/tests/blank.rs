//! Finding and removing blank pages.

use image::{DynamicImage, GrayImage, Luma};
use mpe_core::{FontChoice, ImageImport, Pdf, BLANK_TOLERANCE};

fn png(img: GrayImage) -> Vec<u8> {
    let mut out = Vec::new();
    DynamicImage::ImageLuma8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A scanned sheet: grey paper, faint show-through from the other side, a few specks, and optionally a block of
/// dark "text".
fn scan(text: bool) -> Vec<u8> {
    // Letter size at 150 dpi, with about 20 specks of 3 by 3 pixels.
    png(GrayImage::from_fn(1275, 1650, |x, y| {
        let speck = ((x / 3) * 7 + (y / 3) * 13) % 9973 == 0;
        let show_through = (100..700).contains(&x) && (100..130).contains(&y);
        let block = text && (100..700).contains(&x) && (300..340).contains(&y);
        Luma([if speck || block {
            20
        } else if show_through {
            185
        } else {
            215
        }])
    }))
}

/// Pages: 0 text, 1 empty, 2 a page number only, 3 a blank scan, 4 a scan with text.
fn mixed() -> Pdf {
    let mut p = Pdf::new();
    for i in 0..3 {
        p.insert_blank(i, 612.0, 792.0).unwrap();
    }
    let font = FontChoice::from_name("Helvetica");
    p.add_text(
        0,
        (72.0, 100.0),
        "A line of ordinary text on the page.",
        font,
        12.0,
        [0.0; 3],
    )
    .unwrap();
    p.add_text(2, (300.0, 760.0), "7", font, 10.0, [0.0; 3])
        .unwrap();
    let scans = Pdf::from_images(&[scan(false), scan(true)], &ImageImport::default()).unwrap();
    p.append(vec![scans]).unwrap();
    p
}

#[test]
fn finds_empty_pages_specks_and_lone_page_numbers() {
    let mut p = mixed();
    assert_eq!(p.page_count(), 5);
    assert_eq!(p.blank_pages(BLANK_TOLERANCE).unwrap(), vec![1, 2, 3]);
    // With no tolerance, only the page with nothing on it is blank.
    assert_eq!(p.blank_pages(0.0).unwrap(), vec![1]);
}

#[test]
fn removes_blank_pages() {
    let mut p = mixed();
    assert_eq!(
        p.remove_blank_pages(BLANK_TOLERANCE).unwrap(),
        vec![1, 2, 3]
    );
    assert_eq!(p.page_count(), 2);
    assert!(p.blank_pages(BLANK_TOLERANCE).unwrap().is_empty());
    assert!(p.remove_blank_pages(BLANK_TOLERANCE).unwrap().is_empty());
}

#[test]
fn keeps_a_document_that_is_all_blank() {
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    p.insert_blank(1, 612.0, 792.0).unwrap();
    assert!(p.remove_blank_pages(BLANK_TOLERANCE).is_err());
    assert_eq!(p.page_count(), 2);
}
