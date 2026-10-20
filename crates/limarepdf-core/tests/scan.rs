//! Optimising scanned pages.

use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage};
use limarepdf_core::{render, ImageImport, Pdf, ScanMode, ScanOptions, ScanResult};
use lopdf::{dictionary, Dictionary, Object, Stream};
use std::io::Cursor;

const LINES: [&str; 8] = [
    "The quick brown fox jumps over the lazy dog.",
    "Pack my box with five dozen liquor jugs.",
    "How vexingly quick daft zebras jump!",
    "Sphinx of black quartz, judge my vow.",
    "The five boxing wizards jump quickly.",
    "Jackdaws love my big sphinx of quartz.",
    "Bright vixens jump; dozy fowl quack.",
    "Quick zephyrs blow, vexing daft Jim.",
];

fn opts() -> ScanOptions {
    ScanOptions {
        deskew: false,
        despeckle: false,
        ..Default::default()
    }
}

fn png(img: DynamicImage) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A 4 x 3 inch page of text, tilted `deg` degrees anticlockwise, rendered as a
/// greyscale scan at 300 dpi.
fn text_scan(deg: f32) -> GrayImage {
    let mut p = Pdf::new();
    p.insert_blank(0, 288.0, 216.0).unwrap();
    let (s, c) = deg.to_radians().sin_cos();
    // Rotate about the page centre.
    let (cx, cy) = (144.0, 108.0);
    let mut content = format!(
        "q {c} {s} {} {c} {} {} cm BT /F1 13 Tf 16 190 Td 16 TL\n",
        -s,
        cx - c * cx + s * cy,
        cy - s * cx - c * cy
    );
    for l in LINES {
        content.push_str(&format!("({l}) ' "));
    }
    content.push_str("ET Q");
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
    let hp = render::load(p.snapshot().unwrap(), None).unwrap();
    let b = render::Renderer::new(&hp).render(0, 300.0 / 72.0).unwrap();
    GrayImage::from_fn(b.width as u32, b.height as u32, |x, y| {
        Luma([b.rgba[(y as usize * b.width + x as usize) * 4]])
    })
}

fn scanned(img: DynamicImage, dpi: f32) -> Pdf {
    let opts = ImageImport {
        dpi: Some(dpi),
        ..Default::default()
    };
    let mut p = Pdf::from_images(&[png(img)], &opts).unwrap();
    // Store the image Flate-compressed, as most PDFs do.
    p.doc.compress();
    p
}

fn optimize(p: &mut Pdf, o: &ScanOptions) -> ScanResult {
    let mut r = p.optimize_scans(&[0], o).unwrap();
    assert_eq!(r.len(), 1);
    r.remove(0).1
}

/// The image XObject on page 1.
fn image_dict(p: &Pdf) -> Dictionary {
    let page = p.doc.get_dictionary(p.page_ids()[0]).unwrap();
    let res = page.get(b"Resources").unwrap().as_dict().unwrap();
    let xo = res.get(b"XObject").unwrap().as_dict().unwrap();
    let id = xo.get(b"Im0").unwrap().as_reference().unwrap();
    p.doc
        .get_object(id)
        .unwrap()
        .as_stream()
        .unwrap()
        .dict
        .clone()
}

fn name(d: &Dictionary, key: &[u8]) -> String {
    String::from_utf8(d.get(key).unwrap().as_name().unwrap().to_vec()).unwrap()
}

fn gray_render(p: &mut Pdf, scale: f32) -> GrayImage {
    let hp = render::load(p.snapshot().unwrap(), None).unwrap();
    let b = render::Renderer::new(&hp).render(0, scale).unwrap();
    GrayImage::from_fn(b.width as u32, b.height as u32, |x, y| {
        Luma([b.rgba[(y as usize * b.width + x as usize) * 4]])
    })
}

/// Fraction of pixels that differ by more than half the range.
fn differ(a: &GrayImage, b: &GrayImage) -> f32 {
    assert_eq!(a.dimensions(), b.dimensions());
    let n = a
        .pixels()
        .zip(b.pixels())
        .filter(|(p, q)| (p.0[0] as i32 - q.0[0] as i32).abs() > 128)
        .count();
    n as f32 / a.pixels().len() as f32
}

fn optimized(r: &ScanResult) -> (ScanMode, usize, usize, Option<f32>) {
    match r {
        ScanResult::Optimized {
            mode,
            before,
            after,
            deskewed,
            ..
        } => (*mode, *before, *after, *deskewed),
        r => panic!("not optimised: {r:?}"),
    }
}

#[test]
fn text_scan_becomes_black_and_white() {
    let mut p = scanned(DynamicImage::ImageLuma8(text_scan(0.0)), 300.0);
    let looks = gray_render(&mut p, 1.0);
    let (mode, before, after, _) = optimized(&optimize(&mut p, &opts()));
    assert_eq!(mode, ScanMode::BlackWhite);
    assert!(after * 4 < before, "{before} -> {after} bytes");
    let d = image_dict(&p);
    assert_eq!(name(&d, b"Filter"), "CCITTFaxDecode");
    assert_eq!(d.get(b"Width").unwrap().as_i64().unwrap(), 1200);
    let now = gray_render(&mut p, 1.0);
    let f = differ(&looks, &now);
    assert!(f < 0.01, "{:.2}% of pixels changed", f * 100.0);
}

#[test]
fn photo_scan_stays_grey_and_is_downsampled() {
    // A gradient with noise, like a scanned photograph. (A clean gradient compresses
    // better with Flate than with JPEG and would be left alone.)
    let mut seed = 1u32;
    let g = GrayImage::from_fn(600, 400, |x, y| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let noise = (seed >> 16) % 24;
        Luma([((x + y) / 5 + noise) as u8])
    });
    let mut p = scanned(DynamicImage::ImageLuma8(g), 300.0);
    let (mode, ..) = optimized(&optimize(&mut p, &opts()));
    assert_eq!(mode, ScanMode::Gray);
    let d = image_dict(&p);
    assert_eq!(name(&d, b"Filter"), "DCTDecode");
    assert_eq!(name(&d, b"ColorSpace"), "DeviceGray");
    assert_eq!(d.get(b"Width").unwrap().as_i64().unwrap(), 400);
    assert_eq!(d.get(b"Height").unwrap().as_i64().unwrap(), 267);
    // Same place on the page.
    assert_eq!(p.page_size(0).unwrap(), (144.0, 96.0));
}

#[test]
fn colour_scan_stays_colour() {
    let c = RgbImage::from_fn(300, 300, |x, y| Rgb([x as u8, y as u8, 128]));
    let mut p = scanned(DynamicImage::ImageRgb8(c), 150.0);
    let (mode, ..) = optimized(&optimize(&mut p, &opts()));
    assert_eq!(mode, ScanMode::Color);
    assert_eq!(name(&image_dict(&p), b"ColorSpace"), "DeviceRGB");
}

#[test]
fn mode_can_be_forced() {
    let mut p = scanned(DynamicImage::ImageLuma8(text_scan(0.0)), 300.0);
    let o = ScanOptions {
        mode: ScanMode::Gray,
        ..opts()
    };
    assert_eq!(optimized(&optimize(&mut p, &o)).0, ScanMode::Gray);
}

#[test]
fn tilted_scan_is_straightened() {
    let mut p = scanned(DynamicImage::ImageLuma8(text_scan(2.0)), 300.0);
    let o = ScanOptions {
        deskew: true,
        ..opts()
    };
    let (.., deskewed) = optimized(&optimize(&mut p, &o));
    // Anticlockwise tilt: lines run up to the right.
    let a = deskewed.expect("no tilt found");
    assert!((a + 2.0).abs() < 0.15, "corrected {a} degrees");
    // Nothing left to straighten (or to shrink).
    let again = optimize(&mut p, &o);
    assert!(matches!(again, ScanResult::Skipped(_)), "{again:?}");
}

#[test]
fn scan_with_invisible_text_is_not_straightened() {
    // An OCR layer is invisible text; straightening would move the image under it.
    let mut p = scanned(DynamicImage::ImageLuma8(text_scan(2.0)), 300.0);
    let page = p.page_ids()[0];
    let old = p
        .doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Contents")
        .unwrap()
        .clone();
    let layer = p.doc.add_object(Stream::new(
        Dictionary::new(),
        b"BT 3 Tr /F1 12 Tf 10 10 Td (word) Tj ET".to_vec(),
    ));
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Contents", vec![old, Object::Reference(layer)]);
    let o = ScanOptions {
        deskew: true,
        ..opts()
    };
    assert_eq!(optimized(&optimize(&mut p, &o)).3, None);
}

#[test]
fn level_scan_is_not_rotated() {
    let mut p = scanned(DynamicImage::ImageLuma8(text_scan(0.0)), 300.0);
    let o = ScanOptions {
        deskew: true,
        ..opts()
    };
    assert_eq!(optimized(&optimize(&mut p, &o)).3, None);
}

/// White 600 x 600 with a 100 x 100 black block at (100, 100) and single black
/// pixels along y = 400.
fn specks() -> GrayImage {
    GrayImage::from_fn(600, 600, |x, y| {
        let block = (100..200).contains(&x) && (100..200).contains(&y);
        let speck = y == 400 && x % 50 == 25;
        Luma([if block || speck { 0 } else { 255 }])
    })
}

fn check_specks(p: &mut Pdf, gone: bool) {
    // 600 px on a 144 pt page: render at 300 dpi to get one pixel per image pixel.
    let r = gray_render(p, 300.0 / 72.0);
    assert!(r.get_pixel(150, 150).0[0] < 64, "block removed");
    let speck = r.get_pixel(25, 400).0[0];
    assert_eq!(speck > 192, gone, "speck value {speck}");
}

#[test]
fn specks_are_removed() {
    let mut p = scanned(DynamicImage::ImageLuma8(specks()), 300.0);
    let o = ScanOptions {
        despeckle: true,
        ..opts()
    };
    optimized(&optimize(&mut p, &o));
    check_specks(&mut p, true);
}

#[test]
fn group4_images_are_decoded() {
    let mut p = scanned(DynamicImage::ImageLuma8(specks()), 300.0);
    optimized(&optimize(&mut p, &opts()));
    assert_eq!(name(&image_dict(&p), b"Filter"), "CCITTFaxDecode");
    check_specks(&mut p, false);
    // Second pass reads the G4 image back.
    let o = ScanOptions {
        despeckle: true,
        ..opts()
    };
    optimized(&optimize(&mut p, &o));
    check_specks(&mut p, true);
}

#[test]
fn huge_images_are_skipped_before_decoding() {
    // 8 KB standing for billions of pixels: a Group 4 fax picture (each 1 bit is a white
    // row) and a 1-bit one. Each would take gigabytes once decoded.
    let huge = [
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 65_535, "Height" => 65_535,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 1,
            "Filter" => "CCITTFaxDecode",
            "DecodeParms" => dictionary! { "K" => -1, "Columns" => 65_535 },
        },
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 80_000, "Height" => 80_000,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 1,
        },
        // Past u32: must not wrap to a small size.
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => (1i64 << 32) + 10,
            "Height" => 10, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
    ];
    for dict in huge {
        let mut p = scanned(DynamicImage::ImageLuma8(GrayImage::new(40, 30)), 10.0);
        let page = p.doc.get_dictionary(p.page_ids()[0]).unwrap();
        let res = page.get(b"Resources").unwrap().as_dict().unwrap();
        let xo = res.get(b"XObject").unwrap().as_dict().unwrap();
        let id = xo.get(b"Im0").unwrap().as_reference().unwrap();
        p.doc
            .objects
            .insert(id, Object::Stream(Stream::new(dict, vec![0xFF; 8192])));
        match optimize(&mut p, &opts()) {
            ScanResult::Skipped(why) => assert!(why.contains("too large"), "{why}"),
            r => panic!("{r:?}"),
        }
    }
}

#[test]
fn no_gain_leaves_the_image() {
    let mut p = scanned(DynamicImage::ImageLuma8(text_scan(0.0)), 300.0);
    optimized(&optimize(&mut p, &opts()));
    let again = optimize(&mut p, &opts());
    match again {
        ScanResult::Skipped(why) => assert!(why.contains("smaller"), "{why}"),
        r => panic!("{r:?}"),
    }
}

#[test]
fn non_scan_pages_are_skipped() {
    let mut p = Pdf::new();
    p.insert_blank(0, 100.0, 100.0).unwrap();
    p.insert_blank(1, 100.0, 100.0).unwrap();
    let page = p.page_ids()[1];
    let cs = p.doc.add_object(Stream::new(
        Dictionary::new(),
        b"0 0 1 rg 10 10 50 50 re f".to_vec(),
    ));
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Contents", Object::Reference(cs));
    let r = p.optimize_scans(&[0, 1], &opts()).unwrap();
    assert!(matches!(&r[0].1, ScanResult::Skipped(w) if w.contains("single image")));
    assert!(matches!(&r[1].1, ScanResult::Skipped(w) if w.contains("other than an image")));
    assert!(p.optimize_scans(&[2], &opts()).is_err());
}

/// A noisy grey gradient, like a scanned photograph, `w` x `h` pixels.
fn photo(w: u32, h: u32) -> GrayImage {
    let mut seed = 1u32;
    GrayImage::from_fn(w, h, |x, y| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let noise = (seed >> 16) % 24;
        Luma([((x + y) / 5 + noise) as u8])
    })
}

/// `img` scanned at `dpi` on page 1, and a page 2 drawing the same image object over
/// the whole of a page `scale` times as large.
fn shared(img: DynamicImage, dpi: f32, scale: f32) -> Pdf {
    let mut p = scanned(img, dpi);
    let (w, h) = p.page_size(0).unwrap();
    let id = image_id(&p, 0);
    p.insert_blank(1, w * scale, h * scale).unwrap();
    let page = p.page_ids()[1];
    let cs = p.doc.add_object(Stream::new(
        Dictionary::new(),
        format!("q {} 0 0 {} 0 0 cm /Im0 Do Q", w * scale, h * scale).into_bytes(),
    ));
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", cs);
    d.set(
        "Resources",
        dictionary! { "XObject" => dictionary! { "Im0" => id } },
    );
    p
}

/// The image object drawn on page `i`, found through its own resources.
fn image_id(p: &Pdf, i: usize) -> lopdf::ObjectId {
    let page = p.doc.get_dictionary(p.page_ids()[i]).unwrap();
    let res = page.get(b"Resources").unwrap();
    let res = p.doc.dereference(res).unwrap().1.as_dict().unwrap();
    let xo = res.get(b"XObject").unwrap();
    let xo = p.doc.dereference(xo).unwrap().1.as_dict().unwrap();
    xo.get(b"Im0").unwrap().as_reference().unwrap()
}

/// The image drawn on page `i`: its dictionary and its encoded bytes.
fn image_on(p: &Pdf, i: usize) -> (Dictionary, Vec<u8>) {
    let s = p
        .doc
        .get_object(image_id(p, i))
        .unwrap()
        .as_stream()
        .unwrap();
    (s.dict.clone(), s.content.clone())
}

/// The width in pixels of the image drawn on page `i`.
fn width_on(p: &Pdf, i: usize) -> i64 {
    image_on(p, i).0.get(b"Width").unwrap().as_i64().unwrap()
}

fn render_page(p: &mut Pdf, i: usize, scale: f32) -> GrayImage {
    let hp = render::load(p.snapshot().unwrap(), None).unwrap();
    let b = render::Renderer::new(&hp).render(i, scale).unwrap();
    GrayImage::from_fn(b.width as u32, b.height as u32, |x, y| {
        Luma([b.rgba[(y as usize * b.width + x as usize) * 4]])
    })
}

#[test]
fn an_image_shared_with_an_unselected_page_stays_on_that_page() {
    let mut p = shared(DynamicImage::ImageLuma8(photo(600, 400)), 300.0, 1.0);
    let (before, looks) = (image_on(&p, 1), render_page(&mut p, 1, 1.0));
    let r = p.optimize_scans(&[0], &opts()).unwrap();
    assert!(
        matches!(
            r[0].1,
            ScanResult::Optimized {
                original_kept: true,
                ..
            }
        ),
        "{:?}",
        r[0].1
    );
    // Page 1 has its smaller image; page 2 still has the original, as it looks.
    assert_eq!(name(&image_on(&p, 0).0, b"Filter"), "DCTDecode");
    assert_eq!(width_on(&p, 0), 400);
    assert!(image_on(&p, 1) == before, "page 2's image was changed");
    assert_eq!(differ(&looks, &render_page(&mut p, 1, 1.0)), 0.0);
    // Saved and read back, both pages draw their own image.
    let back = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert_eq!(width_on(&back, 0), 400);
    assert_eq!(image_on(&back, 1).1, before.1);
}

#[test]
fn pages_sharing_an_image_keep_the_resolution_each_needs() {
    // Page 1 shows the image at 300 dpi, page 2 (twice as large) at 150 dpi. Kept to at
    // most 200 dpi, page 1 may lose detail, page 2 none.
    let mut p = shared(DynamicImage::ImageLuma8(photo(600, 400)), 300.0, 2.0);
    let r = p.optimize_scans(&[0, 1], &opts()).unwrap();
    assert_eq!(r.len(), 2);
    // Page 1 is 2 inches wide, page 2 four.
    assert!(width_on(&p, 0) >= 400, "page 1: {} pixels", width_on(&p, 0));
    assert!(width_on(&p, 1) >= 600, "page 2: {} pixels", width_on(&p, 1));
}

#[test]
fn a_shared_image_under_text_is_not_straightened() {
    // Page 1 has an OCR layer over its tilted scan; page 2 draws the same scan alone.
    // Page 2 comes first in the selection, as the first use used to decide.
    let mut p = shared(DynamicImage::ImageLuma8(text_scan(2.0)), 300.0, 1.0);
    let page = p.page_ids()[0];
    let old = p
        .doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Contents")
        .unwrap()
        .clone();
    let layer = p.doc.add_object(Stream::new(
        Dictionary::new(),
        b"BT 3 Tr /F1 12 Tf 10 10 Td (word) Tj ET".to_vec(),
    ));
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Contents", vec![old, Object::Reference(layer)]);
    let looks = render_page(&mut p, 0, 1.0);
    let o = ScanOptions {
        deskew: true,
        ..opts()
    };
    p.optimize_scans(&[1, 0], &o).unwrap();
    let f = differ(&looks, &render_page(&mut p, 0, 1.0));
    assert!(f < 0.01, "{:.2}% of page 1's pixels changed", f * 100.0);
}

#[test]
fn pages_sharing_an_image_are_reported_as_optimised() {
    let mut p = shared(DynamicImage::ImageLuma8(photo(600, 400)), 300.0, 1.0);
    let r = p.optimize_scans(&[0, 1], &opts()).unwrap();
    assert!(
        matches!(
            r[0].1,
            ScanResult::Optimized {
                original_kept: false,
                ..
            }
        ),
        "{:?}",
        r[0].1
    );
    assert_eq!(
        r[1].1,
        ScanResult::SameImage(0),
        "page 2 changed with page 1 but was reported otherwise"
    );
    // Both pages draw the one new image; the original is gone.
    assert_eq!(image_id(&p, 0), image_id(&p, 1));
    let images = p
        .doc
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .filter(|s| s.dict.get(b"Subtype").and_then(|n| n.as_name()).ok() == Some(b"Image"))
        .count();
    assert_eq!(images, 1);
}
