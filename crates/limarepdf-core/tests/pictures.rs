//! Saving pictures (NEW-09's stage 2): a JPEG upright on the page written byte for byte, any
//! other picture, or one turned or flipped on the page, a lossless PNG as it shows; and every
//! picture of a document to a folder, named by page and number, one drawn on several pages
//! once.

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, GenericImageView, ImageFormat};
use limarepdf_core::{ObjectKind, Pdf, PictureFormat};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

const W: f32 = 612.0;
const H: f32 = 792.0;

/// A `w` × `h` picture's RGB samples: a different colour in each corner, so a turn or a flip
/// shows.
fn corners(w: u32, h: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let (right, low) = (x >= w / 2, y >= h / 2);
            data.extend(match (right, low) {
                (false, false) => [220, 30, 30],
                (true, false) => [30, 200, 30],
                (false, true) => [30, 30, 220],
                (true, true) => [240, 220, 20],
            });
        }
    }
    data
}

/// An RGB JPEG of [`corners`].
fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 90)
        .encode(&corners(w, h), w, h, image::ExtendedColorType::Rgb8)
        .unwrap();
    out
}

fn jpeg_picture(w: u32, h: u32) -> Stream {
    Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64,
            "Height" => h as i64, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        },
        jpeg(w, h),
    )
    .with_compression(false)
}

/// An 8-bit picture of `data` in `space`, Flate-compressed.
fn flate_picture(w: u32, h: u32, space: Object, data: Vec<u8>) -> Stream {
    let mut s = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64,
            "Height" => h as i64, "ColorSpace" => space, "BitsPerComponent" => 8,
        },
        data,
    );
    s.compress().unwrap();
    s
}

/// A blank document of `n` pages.
fn pages(n: usize) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..n {
        p.insert_blank(i, W, H).unwrap();
    }
    p
}

/// Page `page` draws `content` with `xobjects` (and a line of text in Helvetica).
fn draw(p: &mut Pdf, page: usize, content: &str, xobjects: Dictionary) {
    let id = p.page_ids()[page];
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let content = format!("BT /F1 12 Tf 72 740 Td (Words on the page) Tj ET {content}");
    let c = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.into_bytes()));
    let d = p.doc.get_dictionary_mut(id).unwrap();
    d.set("Contents", c);
    d.set(
        "Resources",
        dictionary! {"Font" => dictionary! {"F1" => font}, "XObject" => xobjects},
    );
}

/// A one-page document drawing `picture` as `/Im1` with `cm`.
fn one(picture: Stream, cm: &str) -> (Pdf, ObjectId) {
    let mut p = pages(1);
    let im = p.doc.add_object(picture);
    draw(
        &mut p,
        0,
        &format!("q {cm} cm /Im1 Do Q"),
        dictionary! {"Im1" => im},
    );
    (p, im)
}

/// The id of the only picture among page `page`'s objects.
fn picture_id(p: &Pdf, page: usize) -> usize {
    let objs = p.page_objects(page).unwrap();
    let mut ims = objs.iter().filter(|o| o.kind == ObjectKind::Image);
    let id = ims.next().expect("a picture on the page").id;
    assert!(ims.next().is_none());
    id
}

fn saved(p: &Pdf, page: usize) -> (PictureFormat, Vec<u8>) {
    let id = picture_id(p, page);
    let out = p.picture_file(page, id).unwrap();
    assert_eq!(p.picture_format(page, id).unwrap(), out.0);
    out
}

fn png(bytes: &[u8]) -> DynamicImage {
    assert_eq!(image::guess_format(bytes).unwrap(), ImageFormat::Png);
    image::load_from_memory_with_format(bytes, ImageFormat::Png).unwrap()
}

/// Pixel (`x`, `y`)'s colour, near enough to `want` (JPEG blurs a little).
fn near(img: &DynamicImage, x: u32, y: u32, want: [u8; 3]) -> bool {
    let p = img.get_pixel(x, y).0;
    (0..3).all(|i| p[i].abs_diff(want[i]) <= 24)
}

const RED: [u8; 3] = [220, 30, 30];
const GREEN: [u8; 3] = [30, 200, 30];
const BLUE: [u8; 3] = [30, 30, 220];
const YELLOW: [u8; 3] = [240, 220, 20];

#[test]
fn an_upright_jpeg_is_written_byte_for_byte() {
    let (p, im) = one(jpeg_picture(80, 40), "160 0 0 80 72 400");
    let (format, bytes) = saved(&p, 0);
    assert_eq!(format, PictureFormat::Jpeg);
    assert_eq!(format.extension(), "jpg");
    let stored = &p.doc.get_object(im).unwrap().as_stream().unwrap().content;
    assert_eq!(&bytes, stored);
}

#[test]
fn a_stretched_upright_jpeg_is_still_written_as_it_is() {
    // Shown twice as wide as its pixels: still upright, so the file is the JPEG itself.
    let (p, im) = one(jpeg_picture(80, 40), "320 0 0 40 72 400");
    let (format, bytes) = saved(&p, 0);
    assert_eq!(format, PictureFormat::Jpeg);
    let stored = &p.doc.get_object(im).unwrap().as_stream().unwrap().content;
    assert_eq!(&bytes, stored);
}

#[test]
fn a_lossless_picture_is_a_png_of_exactly_its_pixels() {
    let data = corners(60, 30);
    let (p, _) = one(
        flate_picture(60, 30, "DeviceRGB".into(), data.clone()),
        "120 0 0 60 72 400",
    );
    let (format, bytes) = saved(&p, 0);
    assert_eq!(format, PictureFormat::Png);
    assert_eq!(format.extension(), "png");
    let img = png(&bytes);
    assert_eq!(img.dimensions(), (60, 30));
    // Opaque: saved without an alpha channel.
    assert!(!img.color().has_alpha());
    assert_eq!(img.to_rgb8().into_raw(), data);
}

#[test]
fn a_grey_picture_keeps_its_shades() {
    let data: Vec<u8> = (0..40u32 * 20).map(|i| (i % 40 * 6) as u8).collect();
    let (p, _) = one(
        flate_picture(40, 20, "DeviceGray".into(), data.clone()),
        "80 0 0 40 72 400",
    );
    let (format, bytes) = saved(&p, 0);
    assert_eq!(format, PictureFormat::Png);
    let img = png(&bytes);
    assert_eq!(img.dimensions(), (40, 20));
    assert_eq!(img.to_luma8().into_raw(), data);
}

#[test]
fn a_jpeg_turned_on_the_page_is_a_png_as_it_shows() {
    // Turned a quarter anticlockwise: the picture's top edge runs up its left side.
    let (p, _) = one(jpeg_picture(80, 40), "0 160 -80 0 300 300");
    let (format, bytes) = saved(&p, 0);
    assert_eq!(format, PictureFormat::Png);
    let img = png(&bytes);
    assert_eq!(img.dimensions(), (40, 80));
    // The top left corner (red) is now at the bottom left, the top right (green) at the
    // top left.
    assert!(near(&img, 5, 75, RED), "{:?}", img.get_pixel(5, 75));
    assert!(near(&img, 5, 5, GREEN), "{:?}", img.get_pixel(5, 5));
    assert!(near(&img, 35, 75, BLUE), "{:?}", img.get_pixel(35, 75));
    assert!(near(&img, 35, 5, YELLOW), "{:?}", img.get_pixel(35, 5));
}

#[test]
fn a_jpeg_flipped_on_the_page_is_a_png_as_it_shows() {
    // Mirrored left to right.
    let (p, _) = one(jpeg_picture(80, 40), "-160 0 0 80 300 400");
    let (format, bytes) = saved(&p, 0);
    assert_eq!(format, PictureFormat::Png);
    let img = png(&bytes);
    assert_eq!(img.dimensions(), (80, 40));
    assert!(near(&img, 75, 5, RED), "{:?}", img.get_pixel(75, 5));
    assert!(near(&img, 5, 5, GREEN), "{:?}", img.get_pixel(5, 5));
}

#[test]
fn a_jpeg_on_a_turned_page_is_a_png_as_the_page_shows_it() {
    let (mut p, _) = one(jpeg_picture(80, 40), "160 0 0 80 72 400");
    let page = p.page_ids()[0];
    p.doc.get_dictionary_mut(page).unwrap().set("Rotate", 90);
    let (format, bytes) = saved(&p, 0);
    assert_eq!(format, PictureFormat::Png);
    let img = png(&bytes);
    // A quarter clockwise: the top left corner (red) is at the top right.
    assert_eq!(img.dimensions(), (40, 80));
    assert!(near(&img, 35, 5, RED), "{:?}", img.get_pixel(35, 5));
    assert!(near(&img, 5, 5, BLUE), "{:?}", img.get_pixel(5, 5));
}

#[test]
fn a_picture_with_a_soft_mask_keeps_its_transparency() {
    let mut p = pages(1);
    let mask: Vec<u8> = (0..20u32 * 10)
        .map(|i| if i % 20 < 10 { 255 } else { 0 })
        .collect();
    let mask = p
        .doc
        .add_object(flate_picture(20, 10, "DeviceGray".into(), mask));
    let mut pic = flate_picture(20, 10, "DeviceRGB".into(), corners(20, 10));
    pic.dict.set("SMask", mask);
    let im = p.doc.add_object(pic);
    draw(
        &mut p,
        0,
        "q 40 0 0 20 72 400 cm /Im1 Do Q",
        dictionary! {"Im1" => im},
    );
    let (format, bytes) = saved(&p, 0);
    assert_eq!(format, PictureFormat::Png);
    let img = png(&bytes);
    assert!(img.color().has_alpha());
    let img = img.to_rgba8();
    assert_eq!(img.dimensions(), (20, 10));
    // The left half shows, the right half is clear.
    assert_eq!(img.get_pixel(2, 2).0, [220, 30, 30, 255]);
    assert_eq!(img.get_pixel(17, 2).0[3], 0);
}

#[test]
fn a_jpeg_with_a_soft_mask_is_a_png_keeping_its_transparency() {
    let mut p = pages(1);
    let mask = p
        .doc
        .add_object(flate_picture(80, 40, "DeviceGray".into(), vec![0; 80 * 40]));
    let mut pic = jpeg_picture(80, 40);
    pic.dict.set("SMask", mask);
    let im = p.doc.add_object(pic);
    draw(
        &mut p,
        0,
        "q 160 0 0 80 72 400 cm /Im1 Do Q",
        dictionary! {"Im1" => im},
    );
    let (format, bytes) = saved(&p, 0);
    assert_eq!(format, PictureFormat::Png);
    assert_eq!(png(&bytes).to_rgba8().get_pixel(10, 10).0[3], 0);
}

#[test]
fn a_cmyk_picture_is_a_png_in_colour() {
    // Pure cyan, then pure magenta, in CMYK.
    let mut data = Vec::new();
    for i in 0..20 * 10 {
        data.extend(if i % 20 < 10 {
            [255, 0, 0, 0]
        } else {
            [0, 255, 0, 0]
        });
    }
    let (p, _) = one(
        flate_picture(20, 10, "DeviceCMYK".into(), data),
        "40 0 0 20 72 400",
    );
    let (format, bytes) = saved(&p, 0);
    assert_eq!(format, PictureFormat::Png);
    let img = png(&bytes).to_rgb8();
    let (c, m) = (img.get_pixel(2, 2).0, img.get_pixel(17, 2).0);
    // Cyan: little red, much green and blue. Magenta: much red and blue, little green.
    assert!(c[0] < 100 && c[1] > 120 && c[2] > 180, "{c:?}");
    assert!(m[0] > 180 && m[1] < 100 && m[2] > 100, "{m:?}");
}

#[test]
fn a_palette_picture_is_a_png_of_its_colours() {
    let palette = Object::String(vec![255, 0, 0, 0, 0, 255], lopdf::StringFormat::Hexadecimal);
    let space = Object::Array(vec![
        "Indexed".into(),
        "DeviceRGB".into(),
        1.into(),
        palette,
    ]);
    let data: Vec<u8> = (0..20u32 * 10).map(|i| u8::from(i % 20 >= 10)).collect();
    let (p, _) = one(flate_picture(20, 10, space, data), "40 0 0 20 72 400");
    let img = png(&saved(&p, 0).1).to_rgb8();
    assert_eq!(img.get_pixel(2, 2).0, [255, 0, 0]);
    assert_eq!(img.get_pixel(17, 2).0, [0, 0, 255]);
}

#[test]
fn an_inline_picture_is_saved_too() {
    let mut p = pages(1);
    let mut content = b"q 40 0 0 20 72 400 cm BI /W 4 /H 2 /CS /RGB /BPC 8 ID ".to_vec();
    let px: Vec<u8> = [[255u8, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]]
        .iter()
        .cycle()
        .take(8)
        .flatten()
        .copied()
        .collect();
    content.extend(&px);
    content.extend(b"\nEI Q");
    let c = p.doc.add_object(Stream::new(Dictionary::new(), content));
    let page = p.page_ids()[0];
    p.doc.get_dictionary_mut(page).unwrap().set("Contents", c);
    let (format, bytes) = saved(&p, 0);
    assert_eq!(format, PictureFormat::Png);
    let img = png(&bytes).to_rgb8();
    assert_eq!(img.dimensions(), (4, 2));
    assert_eq!(img.into_raw(), px);
}

#[test]
fn only_a_picture_can_be_saved_as_one() {
    let (p, _) = one(jpeg_picture(8, 8), "16 0 0 16 72 400");
    let objs = p.page_objects(0).unwrap();
    let text = objs.iter().find(|o| o.kind == ObjectKind::Text).unwrap();
    assert!(p.picture_file(0, text.id).is_err());
    assert!(p.picture_format(0, text.id).is_err());
    assert!(p.picture_file(0, 99).is_err());
    assert!(p.picture_file(3, 0).is_err());
}

/// Three pages: a logo (a JPEG) on the first and the third, a drawing holding a lossless
/// picture on the second, and on the third a photo (a JPEG) after the logo.
fn three_pages() -> Pdf {
    let mut p = pages(3);
    let logo = p.doc.add_object(jpeg_picture(30, 20));
    let photo = p.doc.add_object(jpeg_picture(80, 40));
    let flat = p
        .doc
        .add_object(flate_picture(10, 10, "DeviceRGB".into(), corners(10, 10)));
    let form = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "Resources" => dictionary! {"XObject" => dictionary! {"Pic" => flat}},
        },
        b"/Pic Do".to_vec(),
    ));
    draw(
        &mut p,
        0,
        "q 60 0 0 40 72 600 cm /L Do Q",
        dictionary! {"L" => logo},
    );
    draw(
        &mut p,
        1,
        "q 100 0 0 100 72 400 cm /F Do Q",
        dictionary! {"F" => form},
    );
    draw(
        &mut p,
        2,
        "q 60 0 0 40 72 600 cm /L Do Q q 160 0 0 80 72 300 cm /P Do Q",
        dictionary! {"L" => logo, "P" => photo},
    );
    p
}

#[test]
fn every_picture_is_named_by_page_and_number_once() {
    let p = three_pages();
    assert_eq!(
        p.picture_names("report"),
        [
            "report-page-001-image-1.jpg",
            "report-page-002-image-1.png",
            // The second picture on page 3: the first, the logo, is saved with page 1.
            "report-page-003-image-2.jpg",
        ]
    );
    assert!(pages(2).picture_names("empty").is_empty());
}

#[test]
fn every_picture_is_saved_to_a_folder() {
    let p = three_pages();
    let dir = std::env::temp_dir().join(format!("limarepdf-pictures-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let done = p.save_all_pictures(&dir, "report").unwrap();
    assert!(done.failed.is_empty(), "{:?}", done.failed);
    let names: Vec<String> = (done.written.iter())
        .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, p.picture_names("report"));
    // The logo and the photo as they are; the drawing's picture with its own pixels.
    assert_eq!(std::fs::read(&done.written[0]).unwrap(), jpeg(30, 20));
    assert_eq!(std::fs::read(&done.written[2]).unwrap(), jpeg(80, 40));
    let flat = png(&std::fs::read(&done.written[1]).unwrap());
    assert_eq!(flat.to_rgb8().into_raw(), corners(10, 10));
    std::fs::remove_dir_all(&dir).unwrap();
}
