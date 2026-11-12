//! Finding and removing blank pages.

use image::{DynamicImage, GrayImage, Luma};
use limarepdf_core::{FontChoice, ImageImport, Pdf, BLANK_TOLERANCE};
use lopdf::{dictionary, Dictionary, ObjectId, Stream};

#[path = "common/seeds.rs"]
mod seeds;

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

/// A picture that can't be decoded: three bytes marked as a JPEG.
fn broken_picture(p: &mut Pdf) -> ObjectId {
    p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 2,
            "Height" => 1,
            "BitsPerComponent" => 8,
            "ColorSpace" => "DeviceRGB",
            "Filter" => "DCTDecode",
        },
        vec![0, 1, 2],
    ))
}

/// Gives page `page` the content `content` and the resources `resources`.
fn draw(p: &mut Pdf, page: usize, content: &str, resources: Dictionary) {
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let id = p.page_ids()[page];
    let d = p.doc.get_dictionary_mut(id).unwrap();
    d.set("Contents", s);
    d.set("Resources", resources);
}

/// Shows picture `im` on page `page`.
fn show(p: &mut Pdf, page: usize, im: ObjectId) {
    draw(
        p,
        page,
        "q 150 0 0 100 0 0 cm /Im Do Q",
        dictionary! { "XObject" => dictionary! { "Im" => im } },
    );
}

#[test]
fn keeps_a_page_whose_picture_cant_be_read() {
    // The audit's pages (A-04): a picture that can't be decoded, drawn as nothing, and a black square.
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    p.insert_blank(1, 300.0, 200.0).unwrap();
    let im = broken_picture(&mut p);
    show(&mut p, 0, im);
    draw(&mut p, 1, "0 0 0 rg 30 30 100 100 re f", Dictionary::new());
    seeds::pdf(&p);
    assert!(p.blank_pages(BLANK_TOLERANCE).unwrap().is_empty());
    assert!(p.remove_blank_pages(BLANK_TOLERANCE).unwrap().is_empty());
    let q = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert_eq!(q.page_count(), 2);
    let pictures = q.doc.get_page_images(q.page_ids()[0]).unwrap();
    assert_eq!(pictures.len(), 1);
    assert_eq!(
        pictures[0].content,
        [0, 1, 2],
        "the picture's bytes are kept"
    );
}

#[test]
fn a_picture_that_cant_be_read_among_blank_pages() {
    // Pages: 0 text, 1 empty, 2 a page number only, 3 a blank scan, 4 a scan with text, 5 a picture that can't be
    // decoded.
    let mut p = mixed();
    p.insert_blank(5, 612.0, 792.0).unwrap();
    let im = broken_picture(&mut p);
    show(&mut p, 5, im);
    assert_eq!(p.blank_pages(BLANK_TOLERANCE).unwrap(), vec![1, 2, 3]);
    assert_eq!(p.blank_pages(0.0).unwrap(), vec![1]);
    assert_eq!(
        p.remove_blank_pages(BLANK_TOLERANCE).unwrap(),
        vec![1, 2, 3]
    );
    assert_eq!(p.page_count(), 3);
}

#[test]
fn every_page_showing_a_picture_that_cant_be_read_is_kept() {
    // Pages 0 and 1 show the same picture, which can't be decoded; page 2 is empty.
    let mut p = Pdf::new();
    for i in 0..3 {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    let im = broken_picture(&mut p);
    show(&mut p, 0, im);
    show(&mut p, 1, im);
    assert_eq!(p.blank_pages(BLANK_TOLERANCE).unwrap(), vec![2]);
}
