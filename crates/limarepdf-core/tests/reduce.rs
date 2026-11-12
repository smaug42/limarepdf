//! Reducing pictures (ENH-19): Word's Compress Pictures choices, measured where each picture
//! is drawn largest, masks following their pictures, shared pictures left, kinds that can't
//! be redone left, and nothing else on the page changed.

mod common;

use limarepdf_core::{render, KeptWhy, Pdf, PictureQuality, ReduceOptions, Reduced};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

const W: f32 = 612.0;
const H: f32 = 792.0;

/// Where the tests draw a 1,200 × 800 picture: 4 by 2⅔ inches, 300 pixels per inch.
const AT_300: &str = "q 288 0 0 192 72 400 cm /Im1 Do Q";

/// A photo-like RGB picture: smooth, with more than 256 colours, Flate-compressed.
fn photo(w: i64, h: i64) -> Stream {
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            data.extend([
                (x * 255 / w) as u8,
                (y * 255 / h) as u8,
                (((x / 4) * (y / 4)) % 200) as u8,
            ]);
        }
    }
    picture(w, h, "DeviceRGB", data)
}

/// An 8-bit picture of `data`, Flate-compressed.
fn picture(w: i64, h: i64, space: &str, data: Vec<u8>) -> Stream {
    let mut s = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => w, "Height" => h,
            "ColorSpace" => space, "BitsPerComponent" => 8,
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

/// Page `page` draws picture `im` as `/Im1`, at [`AT_300`].
fn at_300(p: &mut Pdf, page: usize, im: ObjectId) {
    draw(p, page, AT_300, dictionary! {"Im1" => im});
}

fn reduce(p: &mut Pdf, pages: &[usize], quality: PictureQuality) -> Reduced {
    let opts = ReduceOptions {
        quality,
        jpeg_quality: None,
    };
    p.reduce_pictures(pages, &opts).unwrap()
}

/// The picture page `page` draws as `name`.
fn picture_of(p: &Pdf, page: usize, name: &str) -> ObjectId {
    let id = p.page_ids()[page];
    let res = p.doc.get_dictionary(id).unwrap().get(b"Resources").unwrap();
    let res = p.doc.dereference(res).unwrap().1.as_dict().unwrap();
    let xo = p.doc.dereference(res.get(b"XObject").unwrap()).unwrap().1;
    let xo = xo.as_dict().unwrap();
    xo.get(name.as_bytes()).unwrap().as_reference().unwrap()
}

fn stream(p: &Pdf, id: ObjectId) -> &Stream {
    p.doc.get_object(id).unwrap().as_stream().unwrap()
}

fn size_of(p: &Pdf, id: ObjectId) -> (i64, i64) {
    let d = &stream(p, id).dict;
    let n = |k: &[u8]| d.get(k).unwrap().as_i64().unwrap();
    (n(b"Width"), n(b"Height"))
}

fn filter_of(p: &Pdf, id: ObjectId) -> Vec<u8> {
    let d = &stream(p, id).dict;
    d.get(b"Filter").unwrap().as_name().unwrap().to_vec()
}

/// The mean difference of two drawings of page `page` at 72 dpi, per channel.
fn drift(a: &mut Pdf, b: &mut Pdf, page: usize) -> f64 {
    let (x, y) = (
        render::load(a.to_bytes().unwrap(), None).unwrap(),
        render::load(b.to_bytes().unwrap(), None).unwrap(),
    );
    let (x, y) = (
        render::Renderer::new(&x).render(page, 1.0).unwrap(),
        render::Renderer::new(&y).render(page, 1.0).unwrap(),
    );
    let sum: u64 = x
        .rgba
        .iter()
        .zip(&y.rgba)
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    sum as f64 / x.rgba.len() as f64
}

#[test]
fn a_photo_above_the_resolution_is_scaled_down_and_saved_as_jpeg() {
    let mut p = pages(1);
    let im = p.doc.add_object(photo(1200, 800));
    at_300(&mut p, 0, im);
    let mut was = p.clone();
    let content_before = p.doc.get_page_content(p.page_ids()[0]);
    let r = reduce(&mut p, &[0], PictureQuality::Print);
    assert_eq!(r.pictures, 1, "{r:?}");
    assert!(r.after < r.before);
    assert!(r.kept.is_empty(), "{r:?}");
    assert_eq!(picture_of(&p, 0, "Im1"), im, "replaced in place");
    // 1,200 × 220 / 300 = 880, 800 × 220 / 300 ≈ 587.
    assert_eq!(size_of(&p, im), (880, 587));
    assert_eq!(filter_of(&p, im), b"DCTDecode");
    let most = r.most.unwrap();
    assert_eq!((most.page, most.picture), (0, im));
    // The page's content and text are as they were.
    assert_eq!(p.doc.get_page_content(p.page_ids()[0]), content_before);
    assert_eq!(p.page_text(0).trim(), "Words on the page");
    assert!(drift(&mut was, &mut p, 0) < 3.0);
}

#[test]
fn each_quality_gives_its_resolution_and_high_fidelity_changes_nothing() {
    for (q, w) in [
        (PictureQuality::Hd, 1200),
        (PictureQuality::Print, 880),
        (PictureQuality::Web, 600),
        (PictureQuality::Email, 384),
    ] {
        let mut p = pages(1);
        let im = p.doc.add_object(photo(1200, 800));
        at_300(&mut p, 0, im);
        let r = reduce(&mut p, &[0], q);
        assert_eq!(size_of(&p, im).0, w, "{q:?}");
        // HD's 330 is above the picture's 300: kept.
        if q == PictureQuality::Hd {
            assert_eq!(r.kept.get(&KeptWhy::Resolution), Some(&1));
        }
    }
    let mut p = pages(1);
    let im = p.doc.add_object(photo(1200, 800));
    at_300(&mut p, 0, im);
    let before = p.doc.objects.clone();
    let r = reduce(&mut p, &[0], PictureQuality::HighFidelity);
    assert_eq!(r, Default::default());
    assert!(p.doc.objects == before);
}

/// A drawing (form XObject) of the unit square drawing `/P` with `matrix`.
fn drawing(p: &mut Pdf, matrix: [f32; 6], im: ObjectId) -> ObjectId {
    let m: Vec<Object> = matrix.iter().map(|&v| v.into()).collect();
    p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "Matrix" => m,
            "Resources" => dictionary! {"XObject" => dictionary! {"P" => im}},
        },
        b"/P Do".to_vec(),
    ))
}

#[test]
fn the_resolution_is_measured_where_the_picture_is_drawn_largest_through_drawings() {
    let mut p = pages(2);
    let im = p.doc.add_object(photo(1200, 800));
    // Page 1 draws it small (1 inch wide: 1,200 ppi) inside a drawing scaled by half;
    // page 2 at 4 inches (300 ppi) through a drawing turned a quarter.
    let small = drawing(&mut p, [0.5, 0.0, 0.0, 0.5, 0.0, 0.0], im);
    let turned = drawing(&mut p, [0.0, 1.0, -1.0, 0.0, 0.0, 0.0], im);
    let page1 = "q 144 0 0 96 72 400 cm /F Do Q";
    draw(&mut p, 0, page1, dictionary! {"F" => small});
    let page2 = "q 192 0 0 288 300 300 cm /F Do Q";
    draw(&mut p, 1, page2, dictionary! {"F" => turned});
    let r = reduce(&mut p, &[0, 1], PictureQuality::Web);
    assert_eq!(r.pictures, 1, "{r:?}");
    // Measured at page 2's 300 ppi: 1,200 × 150 / 300.
    assert_eq!(size_of(&p, im), (600, 400));
}

/// A smaller copy beside the original the other page still needs would make the file
/// larger, so the picture is left (pdfbox's `2990` grew by half when copies were made).
#[test]
fn a_picture_shared_with_a_page_not_chosen_is_left() {
    let mut p = pages(2);
    let im = p.doc.add_object(photo(1200, 800));
    at_300(&mut p, 0, im);
    at_300(&mut p, 1, im);
    let before = p.doc.objects.clone();
    let r = reduce(&mut p, &[0], PictureQuality::Email);
    assert_eq!(r.pictures, 0, "{r:?}");
    assert_eq!(r.kept.get(&KeptWhy::Shared), Some(&1), "{r:?}");
    assert!(p.doc.objects == before);
    let r = reduce(&mut p, &[0, 1], PictureQuality::Email);
    assert_eq!(r.pictures, 1, "{r:?}");
    assert_eq!(picture_of(&p, 1, "Im1"), im);
    assert_eq!(size_of(&p, im).0, 384);
}

#[test]
fn a_picture_in_a_drawing_shared_with_a_page_not_chosen_is_left() {
    let mut p = pages(2);
    let im = p.doc.add_object(photo(1200, 800));
    let form = drawing(&mut p, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], im);
    for page in 0..2 {
        let content = "q 288 0 0 192 72 400 cm /F Do Q";
        draw(&mut p, page, content, dictionary! {"F" => form});
    }
    let before = p.doc.objects.clone();
    let r = reduce(&mut p, &[0], PictureQuality::Email);
    assert_eq!(r.pictures, 0);
    assert_eq!(r.kept.get(&KeptWhy::Shared), Some(&1), "{r:?}");
    assert!(p.doc.objects == before);
    // Both pages chosen: reduced in place.
    let r = reduce(&mut p, &[0, 1], PictureQuality::Email);
    assert_eq!(r.pictures, 1);
    assert_eq!(size_of(&p, im).0, 384);
}

#[test]
fn a_picture_also_in_a_comments_drawing_is_left() {
    let mut p = pages(1);
    let im = p.doc.add_object(photo(1200, 800));
    at_300(&mut p, 0, im);
    let ap = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
            "Resources" => dictionary! {"XObject" => dictionary! {"P" => im}},
        },
        b"q 100 0 0 100 0 0 cm /P Do Q".to_vec(),
    ));
    let annot = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Stamp",
        "Rect" => vec![300.into(), 100.into(), 400.into(), 200.into()],
        "AP" => dictionary! {"N" => ap},
    });
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Annots", vec![Object::Reference(annot)]);
    let r = reduce(&mut p, &[0], PictureQuality::Email);
    assert_eq!(r.pictures, 0, "{r:?}");
    assert_eq!(r.kept.get(&KeptWhy::Shared), Some(&1), "{r:?}");
    assert_eq!(picture_of(&p, 0, "Im1"), im);
    assert_eq!(size_of(&p, im), (1200, 800));
}

#[test]
fn a_soft_mask_is_scaled_with_its_picture() {
    let mut p = pages(1);
    let mask: Vec<u8> = (0..1200 * 800).map(|i| (i % 1200 / 5) as u8).collect();
    let mask = p.doc.add_object(picture(1200, 800, "DeviceGray", mask));
    let mut pic = photo(1200, 800);
    pic.dict.set("SMask", mask);
    let im = p.doc.add_object(pic);
    at_300(&mut p, 0, im);
    let r = reduce(&mut p, &[0], PictureQuality::Web);
    assert_eq!(r.pictures, 1, "{r:?}");
    let d = &stream(&p, im).dict;
    let m = d.get(b"SMask").unwrap().as_reference().unwrap();
    assert_ne!(m, mask);
    assert_eq!(size_of(&p, m), (600, 400));
    assert_eq!(filter_of(&p, m), b"FlateDecode");
}

#[test]
fn a_drawing_of_few_colours_stays_lossless() {
    let mut p = pages(1);
    // Four flat colours, as a chart.
    let data: Vec<u8> = (0..1200 * 800)
        .flat_map(|i| match (i % 1200) / 300 {
            0 => [255, 0, 0],
            1 => [0, 128, 0],
            2 => [0, 0, 255],
            _ => [255, 255, 255],
        })
        .collect();
    let im = p.doc.add_object(picture(1200, 800, "DeviceRGB", data));
    at_300(&mut p, 0, im);
    let r = reduce(&mut p, &[0], PictureQuality::Email);
    assert_eq!(r.pictures, 1, "{r:?}");
    assert_eq!(filter_of(&p, im), b"FlateDecode");
}

#[test]
fn a_jpeg_is_scaled_down_at_no_more_than_its_own_quality() {
    let mut p = pages(1);
    let img = image::RgbImage::from_fn(1200, 800, |x, y| {
        image::Rgb([(x / 5) as u8, (y / 4) as u8, ((x ^ y) % 200) as u8])
    });
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 60)
        .encode_image(&img)
        .unwrap();
    let im = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 1200, "Height" => 800,
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode",
        },
        jpeg.clone(),
    ));
    at_300(&mut p, 0, im);
    let r = reduce(&mut p, &[0], PictureQuality::Hd);
    // HD's 330 ppi is above its 300: kept as it was, byte for byte.
    assert_eq!(r.kept.get(&KeptWhy::Resolution), Some(&1));
    let r = reduce(&mut p, &[0], PictureQuality::Web);
    assert_eq!(r.pictures, 1, "{r:?}");
    assert_eq!(filter_of(&p, im), b"DCTDecode");
    let data = &stream(&p, im).content;
    let small = image::load_from_memory(data).unwrap();
    assert_eq!((small.width(), small.height()), (600, 400));
    // Saved at 60 (its own), not Web's 80: a quarter the pixels, well under half the bytes.
    assert!(
        data.len() * 2 < jpeg.len(),
        "{} of {}",
        data.len(),
        jpeg.len()
    );
}

#[test]
fn kinds_that_cant_be_redone_are_left_and_counted() {
    let mut p = pages(1);
    let big = |cs: Object, bpc: i64, n: usize, extra: Dictionary| {
        let mut d = dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 1200, "Height" => 800,
            "ColorSpace" => cs, "BitsPerComponent" => bpc,
        };
        for (k, v) in extra {
            d.set(k, v);
        }
        let row = (1200 * n * bpc as usize).div_ceil(8);
        let mut s = Stream::new(d, (0..row * 800).map(|i| (i % 7) as u8).collect());
        s.compress().unwrap();
        s
    };
    let none = Dictionary::new;
    let cmyk = p.doc.add_object(big("DeviceCMYK".into(), 8, 4, none()));
    let bits = p.doc.add_object(big("DeviceGray".into(), 1, 1, none()));
    let palette = Object::Array(vec![
        "Indexed".into(),
        "DeviceRGB".into(),
        1.into(),
        Object::String(vec![0; 6], lopdf::StringFormat::Hexadecimal),
    ]);
    let indexed = p.doc.add_object(big(palette, 8, 1, none()));
    let inverted = dictionary! {"Decode" => vec![1.into(), 0.into()]};
    let decode = p.doc.add_object(big("DeviceGray".into(), 8, 1, inverted));
    let mut stencil = big("DeviceGray".into(), 1, 1, none());
    stencil.dict.remove(b"ColorSpace");
    stencil.dict.set("ImageMask", true);
    let stencil = p.doc.add_object(stencil);
    let mut jpx = picture(1200, 800, "DeviceRGB", vec![0; 100]);
    jpx.dict.set("Filter", "JPXDecode");
    let jpx = p.doc.add_object(jpx);
    let names = ["A", "B", "C", "D", "E", "F"];
    let ids = [cmyk, bits, indexed, decode, stencil, jpx];
    let mut xo = Dictionary::new();
    let mut content = String::new();
    for (n, id) in names.iter().zip(ids) {
        xo.set(*n, id);
        content.push_str(&format!("q 288 0 0 192 72 400 cm /{n} Do Q "));
    }
    content.push_str("BI /W 2 /H 1 /CS /G /BPC 8 ID \u{1}\u{2} EI");
    draw(&mut p, 0, &content, xo);
    let before = p.doc.objects.clone();
    let r = reduce(&mut p, &[0], PictureQuality::Email);
    assert_eq!(r.pictures, 0, "{r:?}");
    assert_eq!(r.kept.get(&KeptWhy::Kind), Some(&6), "{r:?}");
    assert_eq!(r.kept.get(&KeptWhy::Inline), Some(&1), "{r:?}");
    assert!(p.doc.objects == before);
}

/// A page drawing a photo whose colour space is `/A`, a name its page's `/ColorSpace`
/// resources give as `aliases` (each name with what it stands for).
fn aliased(aliases: &[(&str, &str)]) -> (Pdf, ObjectId) {
    let mut p = pages(1);
    let mut im = photo(1200, 800);
    im.dict.set("ColorSpace", Object::Name(b"A".to_vec()));
    let im = p.doc.add_object(im);
    at_300(&mut p, 0, im);
    let mut spaces = Dictionary::new();
    for (name, means) in aliases {
        spaces.set(*name, Object::Name(means.as_bytes().to_vec()));
    }
    let id = p.page_ids()[0];
    let page = p.doc.get_dictionary_mut(id).unwrap();
    let res = page.get_mut(b"Resources").unwrap().as_dict_mut().unwrap();
    res.set("ColorSpace", spaces);
    (p, im)
}

/// The child process of the next test: reduces pictures whose colour space names itself,
/// directly and through another name, and previews the first, each on a line.
#[test]
#[ignore = "started by colour_spaces_naming_themselves_are_left"]
fn colour_spaces_naming_themselves_child() {
    if std::env::var("ALIAS_CHILD").is_err() {
        return;
    }
    let cycles: [&[(&str, &str)]; 2] = [&[("A", "A")], &[("A", "B"), ("B", "A")]];
    for (case, aliases) in ["itself", "each other"].into_iter().zip(cycles) {
        let (mut p, im) = aliased(aliases);
        let was = stream(&p, im).content.clone();
        let r = reduce(&mut p, &[0], PictureQuality::Email);
        let kind = r.kept.get(&KeptWhy::Kind).copied().unwrap_or(0);
        let same = stream(&p, im).content == was;
        println!("alias|{case}|{}|{kind}|{same}", r.pictures);
    }
    let (p, _) = aliased(&[("A", "A")]);
    let opts = ReduceOptions {
        quality: PictureQuality::Email,
        jpeg_quality: None,
    };
    let r = p.reduce_preview(&[0], &opts).unwrap().report;
    let kind = r.kept.get(&KeptWhy::Kind).copied().unwrap_or(0);
    println!("alias|preview|{}|{kind}", r.pictures);
}

/// A picture whose colour space is a name standing for itself, or for a name standing for
/// it (M-01), is left as a kind that can't be redone, at once: it was followed for ever,
/// until the stack ran out and the app stopped.
#[test]
fn colour_spaces_naming_themselves_are_left() {
    use common::apart::{self, Ran};
    let env = [("ALIAS_CHILD", "1".to_string())];
    let limit = std::time::Duration::from_secs(30);
    let ran = apart::run("colour_spaces_naming_themselves_child", &env, limit);
    let Ran::Ended(text, status) = ran else {
        panic!("not done within 30 s");
    };
    let said = apart::reports(&text, "alias|");
    assert_eq!(
        said,
        ["itself|0|1|true", "each other|0|1|true", "preview|0|1"],
        "{}",
        apart::why_stopped(status)
    );
}

/// A chain of six names ending in RGB is followed: the picture is still redone.
#[test]
fn a_chain_of_colour_space_names_is_followed() {
    // A to DeviceRGB through `n` names: A, C1, C2, … (not letters alone: G stands for
    // DeviceGray).
    let chain = |n: usize| -> Vec<(String, String)> {
        let name = |i: usize| match i {
            0 => "A".to_string(),
            i if i == n => "DeviceRGB".to_string(),
            i => format!("C{i}"),
        };
        (0..n).map(|i| (name(i), name(i + 1))).collect()
    };
    fn pairs(c: &[(String, String)]) -> Vec<(&str, &str)> {
        c.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect()
    }
    let (mut p, im) = aliased(&pairs(&chain(8)));
    let r = reduce(&mut p, &[0], PictureQuality::Email);
    assert_eq!(r.pictures, 1, "{r:?}");
    assert_eq!(size_of(&p, im), (384, 256));
    // One name more than followed: kept as it is.
    let (mut p, im) = aliased(&pairs(&chain(9)));
    let r = reduce(&mut p, &[0], PictureQuality::Email);
    assert_eq!(r.pictures, 0, "{r:?}");
    assert_eq!(r.kept.get(&KeptWhy::Kind), Some(&1), "{r:?}");
    assert_eq!(size_of(&p, im), (1200, 800));
}

/// A smooth grey 1,200 × 800 picture at [`AT_300`] in `/G`, which the page's colour spaces
/// name as Lab (PB-107). `G` is also the abbreviation inline pictures use for DeviceGray;
/// one component a pixel is a third of what Lab needs, so a reader taking the name shows
/// little of it. Smooth, so drawings by different readers differ only by colour.
fn g_also_lab() -> Pdf {
    let mut p = pages(1);
    let (w, h) = (1200, 800);
    let data = (0..w * h)
        .map(|i| ((i % w) * 200 / w + (i / w) * 55 / h) as u8)
        .collect();
    let im = p.doc.add_object(picture(w, h, "G", data));
    at_300(&mut p, 0, im);
    let lab = Object::Array(vec![
        "Lab".into(),
        Object::Dictionary(dictionary! {
            "WhitePoint" => vec![0.9505.into(), 1.0.into(), 1.089.into()],
        }),
    ]);
    let id = p.page_ids()[0];
    let page = p.doc.get_dictionary_mut(id).unwrap();
    let res = page.get_mut(b"Resources").unwrap().as_dict_mut().unwrap();
    res.set("ColorSpace", dictionary! { "G" => lab });
    p
}

/// Page 1 as LimarePDF draws it at 72 dpi.
fn drawn(p: &mut Pdf) -> common::compare::Image {
    let hp = render::load(p.to_bytes().unwrap(), None).unwrap();
    let b = render::Renderer::new(&hp).render(0, 1.0).unwrap();
    common::compare::Image::from_bitmap(&b)
}

/// The mean difference of two drawings of a page at 72 dpi where [`AT_300`] draws, per
/// channel.
fn picture_drift(a: &common::compare::Image, b: &common::compare::Image) -> f64 {
    // 72 to 360 points across; 400 to 592 up the page, 200 to 392 down.
    let (mut sum, mut n) = (0u64, 0u64);
    for y in 200..392 {
        for x in 72..360 {
            let (p, q) = (a.px(x, y), b.px(x, y));
            sum += (0..3).map(|k| u64::from(p[k].abs_diff(q[k]))).sum::<u64>();
            n += 3;
        }
    }
    sum as f64 / n as f64
}

/// A picture XObject in `/G` shows as DeviceGray whatever the page names `G` (hayro and
/// poppler alike, which read the abbreviations first), so redone as grey it looks the same.
#[test]
fn a_picture_in_a_colour_space_named_g_is_redone_as_it_shows() {
    let mut p = g_also_lab();
    let mut was = p.clone();
    let before = drawn(&mut p);
    let grey = (200..392).all(|y| {
        (72..360).all(|x| {
            let [r, g, b] = before.px(x, y);
            r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1
        })
    });
    assert!(grey, "the picture isn't drawn in grey");
    let r = reduce(&mut p, &[0], PictureQuality::Email);
    assert_eq!(r.pictures, 1, "{r:?}");
    let d = drift(&mut was, &mut p, 0);
    assert!(d < 3.0, "mean difference {d:.2}");
}

/// Poppler shows the picture in `/G` as LimarePDF does: as DeviceGray.
#[test]
#[ignore = "opt-in: needs poppler"]
fn poppler_shows_a_picture_in_a_colour_space_named_g_as_grey() {
    let mut p = g_also_lab();
    let theirs = common::saved::tools::pdftoppm(&p.to_bytes().unwrap());
    let d = picture_drift(&drawn(&mut p), &theirs);
    assert!(d < 3.0, "mean difference {d:.2}");
}

#[test]
fn undo_puts_every_picture_back() {
    let mut p = pages(2);
    let a = p.doc.add_object(photo(1200, 800));
    let b = p.doc.add_object(photo(900, 900));
    at_300(&mut p, 0, a);
    let square = "q 216 0 0 216 72 400 cm /Im1 Do Q";
    draw(&mut p, 1, square, dictionary! {"Im1" => b});
    let was = p.doc.objects.clone();
    let before = p.before();
    let r = reduce(&mut p, &[0, 1], PictureQuality::Web);
    assert_eq!(r.pictures, 2, "{r:?}");
    let undo = before.undo(&p);
    p.restore(undo);
    assert!(p.doc.objects == was);
}

/// A picture whose `/DecodeParms` is a reference (pdf.js's `issue17906` has them): its PNG
/// predictor is undone before it is scaled, so it keeps its colours.
#[test]
fn a_picture_whose_decode_parameters_are_a_reference_keeps_its_colours() {
    let mut p = pages(1);
    let (w, h) = (1200usize, 800usize);
    // Each row: PNG's "no filter" byte, then its pixels, red on the left, blue on the right.
    let mut row = vec![0u8];
    row.extend([220, 20, 20].repeat(w / 2));
    row.extend([20, 20, 220].repeat(w / 2));
    let rows = row.repeat(h);
    let parms = p.doc.add_object(dictionary! {
        "Predictor" => 15, "Colors" => 3, "BitsPerComponent" => 8, "Columns" => w as i64,
    });
    let mut s = picture(w as i64, h as i64, "DeviceRGB", rows);
    s.dict.set("DecodeParms", parms);
    let im = p.doc.add_object(s);
    at_300(&mut p, 0, im);
    let r = reduce(&mut p, &[0], PictureQuality::Email);
    assert_eq!(r.pictures, 1, "{r:?}");
    let (nw, nh) = size_of(&p, im);
    let (nw, nh) = (nw as usize, nh as usize);
    let s = stream(&p, im);
    assert!(!s.dict.has(b"DecodeParms"));
    let data = s.decompressed_content().unwrap();
    let px = |x: usize, y: usize| {
        let i = (y * nw + x) * 3;
        [data[i], data[i + 1], data[i + 2]]
    };
    for y in [0, nh / 2, nh - 3, nh - 2, nh - 1] {
        assert_eq!(px(5, y), [220, 20, 20], "row {y}");
        assert_eq!(px(nw - 5, y), [20, 20, 220], "row {y}");
    }
}

#[test]
fn the_preview_says_what_reducing_would_do_and_changes_nothing() {
    let mut p = pages(1);
    let im = p.doc.add_object(photo(1200, 800));
    at_300(&mut p, 0, im);
    let before = p.doc.objects.clone();
    let now = p.saved_size().unwrap();
    let opts = ReduceOptions {
        quality: PictureQuality::Email,
        jpeg_quality: None,
    };
    let preview = p.reduce_preview(&[0], &opts).unwrap();
    assert!(p.doc.objects == before, "the document is changed");
    assert_eq!(preview.report.pictures, 1);
    assert!(preview.after * 4 < now, "{} of {now}", preview.after);
    // The size it says is the size reducing and saving gives.
    let mut reduced = p.clone();
    reduced.reduce_pictures(&[0], &opts).unwrap();
    assert_eq!(reduced.to_bytes().unwrap().len(), preview.after);
    let (a, b) = preview.parts.unwrap();
    let side = limarepdf_core::PREVIEW_SIDE as usize;
    assert_eq!((a.width, a.height), (side, side));
    assert_eq!((b.width, b.height), (side, side));
    // The same part: alike on average, the after softer.
    let mean = |m: &render::Bitmap| {
        let sum: f64 = m.rgba.iter().map(|&v| v as f64).sum();
        sum / m.rgba.len() as f64
    };
    let (ma, mb) = (mean(&a), mean(&b));
    assert!((ma - mb).abs() < 4.0, "{ma} {mb}");
    assert!(a.rgba != b.rgba);
}

#[test]
fn a_page_past_the_end_is_refused() {
    let mut p = pages(1);
    let opts = ReduceOptions::default();
    assert!(p.reduce_pictures(&[1], &opts).is_err());
}

/// Opt-in: every fetched sample's pictures reduced for email, saved, read again and its
/// listed pages drawn: no file larger than saved without reducing, none that fails to open
/// or draw where it drew before, no page changed more than softening explains.
/// `REDUCE_ONLY=name`, `REDUCE_EVERY=n`; run with `prlimit --data` (one process).
#[test]
#[ignore = "needs the fetched samples: sh tools/fetch-render-samples.sh"]
fn reduce_pictures_on_the_corpus() {
    let only = std::env::var("REDUCE_ONLY").unwrap_or_default();
    let every = common::corpus::every("REDUCE_EVERY");
    let samples: Vec<_> = common::corpus::samples(&only)
        .into_iter()
        .filter(|s| s.path().exists())
        .enumerate()
        .filter(|(i, _)| i % every == 0)
        .map(|(_, s)| s)
        .collect();
    assert!(
        !samples.is_empty(),
        "no samples: run sh tools/fetch-render-samples.sh"
    );
    let mut kept = std::collections::BTreeMap::new();
    let (mut files, mut pictures, mut was, mut now, mut worst) = (0, 0, 0usize, 0usize, 0.0f64);
    let mut problems = Vec::new();
    for s in &samples {
        // Named first, so a file that ends the process is known.
        eprintln!("{}", s.name);
        let password = s.password.as_deref();
        let bytes = std::fs::read(s.path()).unwrap();
        let Ok(mut p) = Pdf::from_bytes(&bytes, password) else {
            continue;
        };
        let pages: Vec<usize> = (0..p.page_count()).collect();
        let Ok(plain) = p.to_bytes() else {
            continue;
        };
        let opts = ReduceOptions {
            quality: PictureQuality::Email,
            jpeg_quality: None,
        };
        let r = match p.reduce_pictures(&pages, &opts) {
            Ok(r) => r,
            Err(e) => {
                problems.push(format!("{}: {e}", s.name));
                continue;
            }
        };
        files += 1;
        pictures += r.pictures;
        for (why, n) in &r.kept {
            *kept.entry(*why).or_insert(0) += n;
        }
        let reduced = match p.to_bytes() {
            Ok(b) => b,
            Err(e) => {
                problems.push(format!("{}: not written: {e}", s.name));
                continue;
            }
        };
        was += plain.len();
        now += reduced.len();
        if reduced.len() > plain.len() {
            let sizes = format!("{} → {} bytes", plain.len(), reduced.len());
            problems.push(format!("{}: {sizes}", s.name));
        }
        if let Err(e) = Pdf::from_bytes(&reduced, password) {
            problems.push(format!("{}: doesn't open again: {e}", s.name));
            continue;
        }
        if r.pictures == 0 {
            continue;
        }
        let (Ok(a), Ok(b)) = (
            render::load(plain, password),
            render::load(reduced, password),
        ) else {
            problems.push(format!("{}: hayro can't load one of them", s.name));
            continue;
        };
        let (ra, rb) = (render::Renderer::new(&a), render::Renderer::new(&b));
        for page in s.first..=s.last.min(ra.page_count()) {
            // At a width of their own: some samples have pages too large for hayro at 72 dpi.
            match (
                ra.render_to_width(page - 1, 400.0),
                rb.render_to_width(page - 1, 400.0),
            ) {
                (Ok(x), Ok(y)) if x.rgba.len() == y.rgba.len() => {
                    let sum: u64 = x
                        .rgba
                        .iter()
                        .zip(&y.rgba)
                        .map(|(a, b)| u64::from(a.abs_diff(*b)))
                        .sum();
                    let moved = sum as f64 / x.rgba.len() as f64;
                    worst = worst.max(moved);
                    // Email's 96 ppi softens a picture; more than this is another picture.
                    if moved > 8.0 {
                        problems.push(format!("{} p{page}: moved {moved:.1}", s.name));
                    }
                }
                (Err(_), _) => {}
                _ => problems.push(format!("{} p{page}: not drawn as before", s.name)),
            }
        }
    }
    println!(
        "{files} files, {pictures} pictures reduced: {was} → {now} bytes ({:.1}% smaller); \
         kept {kept:?}; the most a drawn page moved: {worst:.2} a channel on average",
        100.0 * (1.0 - now as f64 / was.max(1) as f64)
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
