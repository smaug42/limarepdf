//! Importing PNG, JPEG and TIFF files as pages.

use image::codecs::jpeg::{JpegEncoder, PixelDensity};
use image::metadata::Orientation;
use image::{DynamicImage, GrayImage, Rgb, RgbImage, Rgba, RgbaImage};
use limarepdf_core::{render, ImageImport, Pdf};
use lopdf::Dictionary;
use std::io::Cursor;

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const YELLOW: [u8; 3] = [255, 255, 0];
const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

fn at_dpi(d: f32) -> ImageImport {
    ImageImport {
        dpi: Some(d),
        ..Default::default()
    }
}

fn import(files: &[Vec<u8>], opts: &ImageImport) -> Pdf {
    Pdf::from_images(files, opts).unwrap()
}

fn size(p: &Pdf, i: usize) -> (f32, f32) {
    p.page_size(i).unwrap()
}

fn assert_size(p: &Pdf, i: usize, want: (f32, f32)) {
    let got = size(p, i);
    assert!(
        (got.0 - want.0).abs() < 0.05 && (got.1 - want.1).abs() < 0.05,
        "page {i}: got {got:?}, want {want:?}"
    );
}

/// The image XObject placed on page `i`.
fn im0(p: &Pdf, i: usize) -> Dictionary {
    let page = p.doc.get_dictionary(p.page_ids()[i]).unwrap();
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

struct Shot(render::Bitmap, f32);

impl Shot {
    /// Pixel at (x, y) in points from the top-left of the displayed page.
    fn px(&self, x: f32, y: f32) -> [u8; 3] {
        let (px, py) = ((x * self.1) as usize, (y * self.1) as usize);
        let i = (py * self.0.width + px) * 4;
        [self.0.rgba[i], self.0.rgba[i + 1], self.0.rgba[i + 2]]
    }

    fn expect(&self, x: f32, y: f32, want: [u8; 3], tol: i32, what: &str) {
        let got = self.px(x, y);
        let ok = (0..3).all(|k| (got[k] as i32 - want[k] as i32).abs() <= tol);
        assert!(ok, "{what}: at ({x}, {y}) got {got:?}, want {want:?}");
    }
}

fn shot(p: &mut Pdf, page: usize, scale: f32) -> Shot {
    let hp = render::load(p.snapshot().unwrap(), None).unwrap();
    Shot(
        render::Renderer::new(&hp).render(page, scale).unwrap(),
        scale,
    )
}

/// A 40 x 20 image with four coloured quadrants: red, green / blue, yellow.
fn quadrants() -> RgbImage {
    RgbImage::from_fn(40, 20, |x, y| {
        Rgb(match (x < 20, y < 10) {
            (true, true) => RED,
            (false, true) => GREEN,
            (true, false) => BLUE,
            (false, false) => YELLOW,
        })
    })
}

fn encode_png(img: &DynamicImage) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in bytes {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// Insert a chunk right after IHDR.
fn with_chunk(png: Vec<u8>, kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    chunk.extend_from_slice(&body);
    chunk.extend_from_slice(&crc32(&body).to_be_bytes());
    let mut out = png[..33].to_vec();
    out.extend(chunk);
    out.extend_from_slice(&png[33..]);
    out
}

fn phys_dpi(dpi: f32) -> Vec<u8> {
    let ppm = (dpi / 0.0254).round() as u32;
    let mut d = ppm.to_be_bytes().to_vec();
    d.extend(ppm.to_be_bytes());
    d.push(1);
    d
}

/// A little-endian TIFF Exif block holding only an orientation tag.
fn exif(orientation: u8) -> Vec<u8> {
    let mut e = b"II*\0".to_vec();
    e.extend(8u32.to_le_bytes());
    e.extend(1u16.to_le_bytes());
    e.extend(0x0112u16.to_le_bytes());
    e.extend(3u16.to_le_bytes());
    e.extend(1u32.to_le_bytes());
    e.extend([orientation, 0, 0, 0]);
    e.extend(0u32.to_le_bytes());
    e
}

fn encode_jpeg(img: &DynamicImage, dpi: Option<u16>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = JpegEncoder::new_with_quality(&mut out, 95);
    if let Some(d) = dpi {
        enc.set_pixel_density(PixelDensity::dpi(d));
    }
    // Not `encode_image(&DynamicImage)`: that views every image as RGBA.
    enc.encode(
        img.as_bytes(),
        img.width(),
        img.height(),
        img.color().into(),
    )
    .unwrap();
    out
}

/// Insert an APP1 Exif segment right after SOI.
fn with_exif(jpeg: Vec<u8>, orientation: u8) -> Vec<u8> {
    let mut seg = b"Exif\0\0".to_vec();
    seg.extend(exif(orientation));
    let mut out = jpeg[..2].to_vec();
    out.extend([0xFF, 0xE1]);
    out.extend(((seg.len() + 2) as u16).to_be_bytes());
    out.extend(seg);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// A single-strip bilevel TIFF. `photometric` 0 = WhiteIsZero, 1 = BlackIsZero;
/// `compression` 1 = none, 4 = CCITT G4.
fn bilevel_tiff(w: u32, h: u32, photometric: u16, compression: u16, data: &[u8]) -> Vec<u8> {
    let entries: [(u16, u16, u32); 9] = [
        (256, 4, w),
        (257, 4, h),
        (258, 3, 1),
        (259, 3, compression as u32),
        (262, 3, photometric as u32),
        (273, 4, 8 + 2 + 9 * 12 + 4),
        (277, 3, 1),
        (278, 4, h),
        (279, 4, data.len() as u32),
    ];
    let mut t = b"II*\0".to_vec();
    t.extend(8u32.to_le_bytes());
    t.extend((entries.len() as u16).to_le_bytes());
    for (tag, kind, value) in entries {
        t.extend(tag.to_le_bytes());
        t.extend(kind.to_le_bytes());
        t.extend(1u32.to_le_bytes());
        if kind == 3 {
            t.extend((value as u16).to_le_bytes());
            t.extend([0, 0]);
        } else {
            t.extend(value.to_le_bytes());
        }
    }
    t.extend(0u32.to_le_bytes());
    t.extend_from_slice(data);
    t
}

/// 12 x 2 test pattern: row 0 is 4 black then 8 white, row 1 is all black.
fn pattern_black(x: usize, y: usize) -> bool {
    y == 1 || x < 4
}

fn check_pattern(p: &mut Pdf) {
    assert_size(p, 0, (12.0, 2.0));
    assert_eq!(
        im0(p, 0)
            .get(b"BitsPerComponent")
            .unwrap()
            .as_i64()
            .unwrap(),
        1
    );
    let s = shot(p, 0, 8.0);
    s.expect(1.5, 0.5, BLACK, 12, "row 0, black run");
    s.expect(8.5, 0.5, WHITE, 12, "row 0, white run");
    s.expect(10.5, 1.5, BLACK, 12, "row 1");
}

// ------------------------------------------------------------------ sizes

#[test]
fn png_page_size_follows_phys_resolution() {
    let png = encode_png(&DynamicImage::ImageRgb8(RgbImage::new(300, 150)));
    let p = import(
        &[with_chunk(png, b"pHYs", &phys_dpi(300.0))],
        &Default::default(),
    );
    assert_size(&p, 0, (72.0, 36.0));
}

#[test]
fn missing_resolution_uses_fallback() {
    let png = encode_png(&DynamicImage::ImageRgb8(RgbImage::new(300, 150)));
    let p = import(&[png], &Default::default());
    assert_size(&p, 0, (144.0, 72.0));
}

#[test]
fn dpi_option_overrides_the_file() {
    let png = encode_png(&DynamicImage::ImageRgb8(RgbImage::new(300, 150)));
    let p = import(&[with_chunk(png, b"pHYs", &phys_dpi(300.0))], &at_dpi(72.0));
    assert_size(&p, 0, (300.0, 150.0));
}

#[test]
fn jpeg_page_size_follows_jfif_density() {
    let jpg = encode_jpeg(&DynamicImage::ImageRgb8(RgbImage::new(400, 200)), Some(200));
    let p = import(&[jpg], &Default::default());
    assert_size(&p, 0, (144.0, 72.0));
}

// ------------------------------------------------------------ orientation

#[test]
fn png_exif_orientation_all_eight() {
    for code in 1..=8u8 {
        let o = Orientation::from_exif(code).unwrap();
        let src = DynamicImage::ImageRgb8(quadrants());
        let mut want = src.clone();
        want.apply_orientation(o);
        let want = want.to_rgb8();
        let png = with_chunk(encode_png(&src), b"eXIf", &exif(code));
        let mut p = import(&[png], &at_dpi(72.0));
        let (w, h) = (want.width() as f32, want.height() as f32);
        assert_size(&p, 0, (w, h));
        let s = shot(&mut p, 0, 2.0);
        for (fx, fy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
            let (x, y) = (w * fx, h * fy);
            let c = want.get_pixel(x as u32, y as u32).0;
            s.expect(x, y, c, 12, &format!("EXIF orientation {code}"));
        }
    }
}

#[test]
fn jpeg_exif_orientation() {
    // Orientation 6: rotate 90 degrees clockwise for display.
    let src = DynamicImage::ImageRgb8(quadrants());
    let jpg = with_exif(encode_jpeg(&src, None), 6);
    let mut p = import(&[jpg], &at_dpi(72.0));
    assert_size(&p, 0, (20.0, 40.0));
    let s = shot(&mut p, 0, 2.0);
    // Left column (red over blue) becomes the top row, reading right to left.
    s.expect(5.0, 10.0, BLUE, 40, "top left");
    s.expect(15.0, 10.0, RED, 40, "top right");
    s.expect(5.0, 30.0, YELLOW, 40, "bottom left");
    s.expect(15.0, 30.0, GREEN, 40, "bottom right");
}

// ------------------------------------------------------------ colour data

#[test]
fn jpeg_is_embedded_unchanged_with_its_colour_space() {
    let gray = encode_jpeg(&DynamicImage::ImageLuma8(GrayImage::new(16, 16)), None);
    let rgb = encode_jpeg(&DynamicImage::ImageRgb8(RgbImage::new(16, 16)), None);
    let p = import(&[gray.clone(), rgb], &Default::default());
    let (g, c) = (im0(&p, 0), im0(&p, 1));
    assert_eq!(name(&g, b"Filter"), "DCTDecode");
    assert_eq!(name(&g, b"ColorSpace"), "DeviceGray");
    assert_eq!(name(&c, b"ColorSpace"), "DeviceRGB");
    let page = p.doc.get_dictionary(p.page_ids()[0]).unwrap();
    let id = page
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"XObject")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Im0")
        .unwrap()
        .as_reference()
        .unwrap();
    let stream = p.doc.get_object(id).unwrap().as_stream().unwrap();
    assert_eq!(stream.content, gray, "JPEG bytes were changed");
}

#[test]
fn grey_png_stays_grey() {
    let png = encode_png(&DynamicImage::ImageLuma8(GrayImage::new(8, 8)));
    let p = import(&[png], &Default::default());
    assert_eq!(name(&im0(&p, 0), b"ColorSpace"), "DeviceGray");
}

#[test]
fn png_alpha_becomes_smask_only_when_translucent() {
    let opaque = RgbaImage::from_pixel(8, 8, Rgba([255, 0, 0, 255]));
    let clear = RgbaImage::from_pixel(8, 8, Rgba([255, 0, 0, 128]));
    let p = import(
        &[
            encode_png(&DynamicImage::ImageRgba8(opaque)),
            encode_png(&DynamicImage::ImageRgba8(clear)),
        ],
        &Default::default(),
    );
    assert!(im0(&p, 0).get(b"SMask").is_err());
    assert!(im0(&p, 1).get(b"SMask").is_ok());
}

// ------------------------------------------------------------------- TIFF

#[test]
fn multi_page_tiff_becomes_pages() {
    use tiff::encoder::{colortype, Rational, TiffEncoder};
    use tiff::tags::ResolutionUnit;
    let mut buf = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut buf).unwrap();
        let rgb: Vec<u8> = quadrants().into_raw();
        let mut a = enc.new_image::<colortype::RGB8>(40, 20).unwrap();
        a.resolution(ResolutionUnit::Inch, Rational { n: 100, d: 1 });
        a.write_data(&rgb).unwrap();
        let mut b = enc.new_image::<colortype::Gray8>(10, 10).unwrap();
        b.resolution(ResolutionUnit::Centimeter, Rational { n: 10, d: 1 });
        b.write_data(&[128u8; 100]).unwrap();
    }
    let mut p = import(&[buf.into_inner()], &Default::default());
    assert_eq!(p.page_count(), 2);
    assert_size(&p, 0, (40.0 * 0.72, 20.0 * 0.72));
    assert_size(&p, 1, (10.0 * 72.0 / 25.4, 10.0 * 72.0 / 25.4));
    assert_eq!(name(&im0(&p, 1), b"ColorSpace"), "DeviceGray");
    let s = shot(&mut p, 0, 4.0);
    s.expect(7.0, 3.5, RED, 12, "TIFF page 1 top left");
    s.expect(21.0, 10.5, YELLOW, 12, "TIFF page 1 bottom right");
}

#[test]
fn bilevel_tiff_black_is_zero() {
    // Black = 0. Rows padded to whole bytes.
    let data = [0x0F, 0xF0, 0x00, 0x00];
    let mut p = import(&[bilevel_tiff(12, 2, 1, 1, &data)], &at_dpi(72.0));
    check_pattern(&mut p);
}

#[test]
fn bilevel_tiff_white_is_zero() {
    let data = [0xF0, 0x00, 0xFF, 0xF0];
    let mut p = import(&[bilevel_tiff(12, 2, 0, 1, &data)], &at_dpi(72.0));
    check_pattern(&mut p);
}

#[test]
fn g4_tiff() {
    use fax::{encoder::Encoder, Color, VecWriter};
    let mut enc = Encoder::new(VecWriter::new());
    for y in 0..2 {
        let row = (0..12).map(|x| {
            if pattern_black(x, y) {
                Color::Black
            } else {
                Color::White
            }
        });
        enc.encode_line(row, 12).unwrap();
    }
    let g4 = enc.finish().unwrap().finish();
    let mut p = import(&[bilevel_tiff(12, 2, 0, 4, &g4)], &at_dpi(72.0));
    check_pattern(&mut p);
}

// -------------------------------------------------------------- placement

#[test]
fn inserts_at_position() {
    let mut p = Pdf::new();
    p.insert_blank(0, 100.0, 100.0).unwrap();
    p.insert_blank(1, 200.0, 200.0).unwrap();
    let png = encode_png(&DynamicImage::ImageRgb8(RgbImage::new(30, 30)));
    assert_eq!(p.insert_image_pages(1, &[png], &at_dpi(72.0)).unwrap(), 1);
    assert_eq!(p.page_count(), 3);
    assert_size(&p, 0, (100.0, 100.0));
    assert_size(&p, 1, (30.0, 30.0));
    assert_size(&p, 2, (200.0, 200.0));
}

#[test]
fn a_bad_file_changes_nothing() {
    let mut p = Pdf::new();
    p.insert_blank(0, 100.0, 100.0).unwrap();
    let objects = p.doc.objects.len();
    let png = encode_png(&DynamicImage::ImageRgb8(RgbImage::new(30, 30)));
    let err = p
        .insert_image_pages(1, &[png, b"not an image".to_vec()], &Default::default())
        .unwrap_err();
    assert!(err.to_string().contains("image 2"), "{err}");
    assert_eq!(p.page_count(), 1);
    assert_eq!(p.doc.objects.len(), objects);
}
