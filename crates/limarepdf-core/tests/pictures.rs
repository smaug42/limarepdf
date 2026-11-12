//! Saving pictures (NEW-09's stage 2): a JPEG upright on the page written byte for byte, any
//! other picture, or one turned or flipped on the page, a lossless PNG as it shows; and every
//! picture of a document to a folder, named by page and number, one drawn on several pages
//! once. A picture's pixels as it shows, for the system's clipboard (NEW-09's stage 3), and
//! pixels pasted from it put on a page as a lossless picture.

mod common;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, GenericImageView, ImageFormat};
use limarepdf_core::render::Bitmap;
use limarepdf_core::{png_file, ObjectKind, Pdf, PictureFormat};
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

#[test]
fn a_picture_copied_out_is_its_pixels_as_it_shows() {
    // Lossless: exactly its pixels, opaque.
    let data = corners(60, 30);
    let (p, _) = one(
        flate_picture(60, 30, "DeviceRGB".into(), data.clone()),
        "120 0 0 60 72 400",
    );
    let b = p.picture_pixels(0, picture_id(&p, 0)).unwrap();
    assert_eq!((b.width, b.height), (60, 30));
    let rgb: Vec<u8> = (b.rgba.chunks(4))
        .flat_map(|c| [c[0], c[1], c[2]])
        .collect();
    assert_eq!(rgb, data);
    assert!(b.rgba.chunks(4).all(|c| c[3] == 255));
    // A JPEG turned a quarter anticlockwise on the page: turned as it shows.
    let (p, _) = one(jpeg_picture(80, 40), "0 160 -80 0 300 300");
    let b = p.picture_pixels(0, picture_id(&p, 0)).unwrap();
    assert_eq!((b.width, b.height), (40, 80));
    let at = |x: usize, y: usize| {
        let i = (y * b.width + x) * 4;
        [b.rgba[i], b.rgba[i + 1], b.rgba[i + 2]]
    };
    let close = |got: [u8; 3], want: [u8; 3]| (0..3).all(|i| got[i].abs_diff(want[i]) <= 24);
    assert!(close(at(5, 75), RED), "{:?}", at(5, 75));
    assert!(close(at(5, 5), GREEN), "{:?}", at(5, 5));
    let text = (p.page_objects(0).unwrap().into_iter())
        .find(|o| o.kind == ObjectKind::Text)
        .unwrap();
    assert!(p.picture_pixels(0, text.id).is_err());
}

#[test]
fn pixels_pasted_onto_a_page_keep_their_colours_and_transparency() {
    // A 6 × 4 picture: the left half opaque red and blue rows, the right half half clear.
    let rgba: Vec<u8> = (0..24)
        .flat_map(|i| {
            let (x, y) = (i % 6, i / 6);
            let c = if y % 2 == 0 {
                [200, 20, 20]
            } else {
                [20, 20, 200]
            };
            [c[0], c[1], c[2], if x < 3 { 255 } else { 128 }]
        })
        .collect();
    let b = Bitmap {
        width: 6,
        height: 4,
        rgba: rgba.clone(),
    };
    let file = png_file(&b).unwrap();
    let img = png(&file);
    assert!(img.color().has_alpha());
    assert_eq!(img.to_rgba8().into_raw(), rgba);
    // Put on a page and copied out again: the same pixels.
    let mut p = pages(1);
    p.add_image(0, [100.0, 100.0, 106.0, 104.0], &file).unwrap();
    let back = p.picture_pixels(0, picture_id(&p, 0)).unwrap();
    assert_eq!((back.width, back.height), (6, 4));
    for (got, want) in back.rgba.chunks(4).zip(rgba.chunks(4)) {
        assert!(
            (0..4).all(|i| got[i].abs_diff(want[i]) <= 2),
            "{got:?} for {want:?}"
        );
    }
    // Opaque pixels make a picture without alpha.
    let opaque = Bitmap {
        width: 2,
        height: 1,
        rgba: vec![10, 20, 30, 255, 40, 50, 60, 255],
    };
    assert!(!png(&png_file(&opaque).unwrap()).color().has_alpha());
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
    let scratch = common::tool::Scratch::new("pictures");
    let dir = scratch.join("pictures");
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
}

// ------------------------------------------ colour profiles and calibration (C-02)

/// RGB with sRGB's primaries and straight-line tone curves: 128 shows as about 188 (see
/// `data/images/README.md`).
const LINEAR_RGB: &[u8] = include_bytes!("data/images/linear-rgb.icc");

/// CalRGB with sRGB's primaries and white and straight-line tone curves, as
/// [`LINEAR_RGB`].
fn cal_rgb() -> Object {
    let r = |v: &[f32]| Object::Array(v.iter().map(|&x| x.into()).collect());
    Object::Array(vec![
        "CalRGB".into(),
        Object::Dictionary(dictionary! {
            "WhitePoint" => r(&[0.9505, 1.0, 1.089]),
            "Gamma" => r(&[1.0, 1.0, 1.0]),
            "Matrix" => r(&[0.4124, 0.2126, 0.0193, 0.3576, 0.7152, 0.1192, 0.1805, 0.0722, 0.9505]),
        }),
    ])
}

/// `[/ICCBased …]` for RGB profile `data`, added to `p`.
fn icc(p: &mut Pdf, data: &[u8]) -> Object {
    let mut s = Stream::new(dictionary! { "N" => 3 }, data.to_vec());
    s.compress().unwrap();
    Object::Array(vec!["ICCBased".into(), p.doc.add_object(s).into()])
}

/// A colour space, made in the document given (where it needs an object of its own).
type MakeSpace = fn(&mut Pdf) -> Object;

/// Colour spaces in which numbers don't show as they would in sRGB: [`LINEAR_RGB`] and
/// CalRGB.
const NOT_SRGB: [(&str, MakeSpace); 2] = [
    ("profile", |p| icc(p, LINEAR_RGB)),
    ("CalRGB", |_| cal_rgb()),
];

/// An upright 80 × 40 JPEG of [`corners`] drawn with `cm`, in colour space `cs` (given the
/// document).
fn jpeg_in(cm: &str, cs: impl FnOnce(&mut Pdf) -> Object) -> (Pdf, ObjectId) {
    let (mut p, im) = one(jpeg_picture(80, 40), cm);
    let cs = cs(&mut p);
    let s = p.doc.get_object_mut(im).unwrap().as_stream_mut().unwrap();
    s.dict.set("ColorSpace", cs);
    common::seeds::pdf(&p);
    (p, im)
}

/// The mean difference between `a` and `b`, RGB samples, over every pixel and channel (0 to
/// 255).
fn mean_delta<'a>(a: &[u8], b: impl IntoIterator<Item = &'a u8>) -> f32 {
    let sum: u64 = a.iter().zip(b).map(|(&x, &y)| x.abs_diff(y) as u64).sum();
    sum as f32 / a.len() as f32
}

/// The mean difference between the picture on page 1 as it shows and as saved.
fn saved_unlike_shown(p: &Pdf) -> (PictureFormat, f32) {
    let (format, bytes) = saved(p, 0);
    let file = image::load_from_memory(&bytes).unwrap().to_rgb8();
    let shown = p.picture_pixels(0, picture_id(p, 0)).unwrap();
    assert_eq!(file.dimensions(), (shown.width as u32, shown.height as u32));
    let rgb = shown.rgba.as_chunks::<4>().0.iter().flat_map(|px| &px[..3]);
    (format, mean_delta(file.as_raw(), rgb))
}

#[test]
fn a_jpeg_in_a_colour_profile_other_than_srgb_is_saved_as_it_shows() {
    for (what, cs) in NOT_SRGB {
        let (p, _) = jpeg_in("160 0 0 80 72 400", cs);
        let (format, delta) = saved_unlike_shown(&p);
        // A JPEG holds no colour space of the PDF's: other apps show its numbers as sRGB.
        assert!(
            format == PictureFormat::Png && delta < 2.0,
            "{what}: saved as {format:?}, mean difference {delta:.1}"
        );
    }
}

#[test]
fn a_jpeg_in_an_srgb_profile_is_written_byte_for_byte() {
    // The profile's header names sRGB as its device: shown as plain RGB, as other apps show
    // the JPEG.
    let mut srgb = LINEAR_RGB.to_vec();
    srgb[52..56].copy_from_slice(b"sRGB");
    let (p, im) = jpeg_in("160 0 0 80 72 400", |p| icc(p, &srgb));
    let (format, delta) = saved_unlike_shown(&p);
    assert_eq!(format, PictureFormat::Jpeg);
    assert!(delta < 2.0, "mean difference {delta:.1}");
    let stored = &p.doc.get_object(im).unwrap().as_stream().unwrap().content;
    assert_eq!(&saved(&p, 0).1, stored);
}

/// Poppler shows a JPEG in a colour profile or CalRGB as it is saved.
#[test]
#[ignore = "opt-in: needs poppler"]
fn poppler_shows_a_jpeg_in_a_colour_profile_as_it_is_saved() {
    for (what, cs) in NOT_SRGB {
        // At its own size, a pixel to a point: 72 dots an inch shows it pixel for pixel,
        // from 72 points in and 352 down.
        let (mut p, _) = jpeg_in("80 0 0 40 72 400", cs);
        let file = image::load_from_memory(&saved(&p, 0).1).unwrap().to_rgb8();
        let page = common::saved::tools::pdftoppm(&p.to_bytes().unwrap());
        let rows = page.rgb.chunks_exact(page.width * 3).skip(352).take(40);
        let shown = rows.flat_map(|row| &row[72 * 3..152 * 3]);
        let delta = mean_delta(file.as_raw(), shown);
        assert!(delta < 4.0, "{what}: mean difference {delta:.1}");
    }
}
