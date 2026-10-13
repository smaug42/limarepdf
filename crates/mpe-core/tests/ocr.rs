//! OCR text layers, with the bundled recognition model.

use image::{DynamicImage, GrayImage, Luma};
use lopdf::{dictionary, Dictionary, Stream};
use mpe_core::{render, ImageImport, ObjectKind, Ocr, OcrResult, Pdf};
use std::io::Cursor;

const LINES: [&str; 6] = [
    "Invoice 2026-0417, issued on 12 October 2026.",
    "Please pay the balance within 30 days.",
    "The committee met on Tuesday morning.",
    "Contact accounts@example.org for details.",
    "Shipping weight 14.7 kg, reference ZX-88.",
    "She quickly jumped over the fence.",
];

/// A 4 x 3 inch page of text rendered as a greyscale image at 300 dpi.
fn text_image() -> GrayImage {
    let mut p = Pdf::new();
    p.insert_blank(0, 288.0, 216.0).unwrap();
    let mut content = String::from("BT /F1 12 Tf 16 190 Td 18 TL\n");
    for l in LINES {
        content.push_str(&format!("({l}) '\n"));
    }
    content.push_str("ET");
    let page = p.page_ids()[0];
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let cs = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.into_bytes()));
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", cs);
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    gray(&mut p, 300.0 / 72.0)
}

fn gray(p: &mut Pdf, scale: f32) -> GrayImage {
    let hp = render::load(p.snapshot().unwrap(), None).unwrap();
    let b = render::Renderer::new(&hp).render(0, scale).unwrap();
    GrayImage::from_fn(b.width as u32, b.height as u32, |x, y| {
        Luma([b.rgba[(y as usize * b.width + x as usize) * 4]])
    })
}

/// A scanned page: the image as the page's only content.
fn scanned(img: GrayImage) -> Pdf {
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageLuma8(img)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let opts = ImageImport {
        dpi: Some(300.0),
        ..Default::default()
    };
    Pdf::from_images(&[png.into_inner()], &opts).unwrap()
}

/// Share of the words in LINES found in `text`, ignoring end punctuation.
fn recall(text: &str) -> f32 {
    let strip = |w: &str| w.trim_matches(|c: char| ".,;:".contains(c)).to_string();
    let got: Vec<String> = text.split_whitespace().map(strip).collect();
    let want: Vec<String> = LINES
        .iter()
        .flat_map(|l| l.split_whitespace())
        .map(strip)
        .collect();
    let hits = want.iter().filter(|w| got.contains(w)).count();
    hits as f32 / want.len() as f32
}

#[test]
fn adds_an_invisible_searchable_layer() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    let before = gray(&mut p, 1.0);
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    assert!(matches!(r[0].1, OcrResult::Added { lines: 6, .. }), "{r:?}");
    let text = p.page_text(0);
    assert!(recall(&text) > 0.9, "recall {}: {text}", recall(&text));
    let lines = text.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(lines, 6, "{text}");
    assert!(!p.search("committee").unwrap().is_empty());
    // Invisible: the page looks the same.
    let after = gray(&mut p, 1.0);
    assert_eq!(before.as_raw(), after.as_raw());
    // Survives a save and reload.
    let again = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert!(recall(&again.page_text(0)) > 0.9);
}

/// Box of the topmost text line, in points from the top-left of the displayed page.
/// The layer is made visible first, since page_objects lists visible text only.
fn first_line_box(p: &mut Pdf) -> [f32; 4] {
    let ids: Vec<_> = p.doc.objects.keys().copied().collect();
    for id in ids {
        if let Ok(s) = p.doc.get_object_mut(id).and_then(|o| o.as_stream_mut()) {
            if s.content.windows(4).any(|w| w == b"3 Tr") {
                let text = String::from_utf8_lossy(&s.content).replace("3 Tr", "0 Tr");
                s.set_content(text.into_bytes());
            }
        }
    }
    p.page_objects(0)
        .unwrap()
        .into_iter()
        .filter(|o| o.kind == ObjectKind::Text)
        .min_by(|a, b| a.bbox[1].total_cmp(&b.bbox[1]))
        .expect("no text objects")
        .bbox
}

#[test]
fn words_sit_where_they_are_printed() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    p.ocr_pages(&[0], &ocr).unwrap();
    // The first line's baseline is at y = 190 - 18 = 172 pt (from the bottom), so its
    // text occupies roughly 32..46 pt from the top; it starts at x = 16 pt.
    let b = first_line_box(&mut p);
    assert!((b[0] - 16.0).abs() < 4.0, "{b:?}");
    assert!(b[1] > 26.0 && b[1] < 38.0, "{b:?}");
    assert!(b[3] > 40.0 && b[3] < 52.0, "{b:?}");
}

#[test]
fn rotated_page_is_read_upright() {
    let ocr = Ocr::bundled().unwrap();
    // Stored sideways, shown upright through /Rotate 90.
    let img = image::imageops::rotate270(&text_image());
    let mut p = scanned(img);
    let page = p.page_ids()[0];
    p.doc.get_dictionary_mut(page).unwrap().set("Rotate", 90);
    let (w, h) = p.page_size(0).unwrap();
    assert!((w - 288.0).abs() < 1.0, "{w} x {h}");
    assert!((h - 216.0).abs() < 1.0, "{w} x {h}");
    p.ocr_pages(&[0], &ocr).unwrap();
    let text = p.page_text(0);
    assert!(recall(&text) > 0.9, "{text}");
    let b = first_line_box(&mut p);
    assert!((b[0] - 16.0).abs() < 4.0, "{b:?}");
    assert!(b[1] > 26.0 && b[1] < 38.0, "{b:?}");
}

#[test]
fn pages_with_text_are_skipped() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(text_image());
    p.ocr_pages(&[0], &ocr).unwrap();
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    assert!(matches!(&r[0].1, OcrResult::Skipped(w) if w.contains("already has text")));
}

#[test]
fn blank_page_has_no_text() {
    let ocr = Ocr::bundled().unwrap();
    let mut p = scanned(GrayImage::from_pixel(600, 600, Luma([255])));
    let r = p.ocr_pages(&[0], &ocr).unwrap();
    let none = matches!(&r[0].1, OcrResult::Skipped(w) if w.contains("no text"));
    assert!(none, "{r:?}");
}
