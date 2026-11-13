//! Pictures whose stated size their data can't come near filling (PB-81, found by fuzzing).
//! hayro sizes its buffers from a picture's stated width and height, so a few bytes claiming
//! 255,555,555 × 2 pixels make it ask for gigabytes. The copies pages are drawn from leave
//! such a picture out, so it is reported like any picture that can't be decoded; the
//! document itself keeps it. Large pictures whose data is there, and small ones a little
//! short, are drawn as they are.

// lopdf's page list, of files made here (clippy.toml).
#![allow(clippy::disallowed_methods)]

use limarepdf_core::print::{Orientation, PageSetup, Paper, PerSheet, Scale};
use limarepdf_core::render::{self, Missing};
use limarepdf_core::Pdf;
use lopdf::{dictionary, Dictionary, Document, Object, Stream};

/// The fuzz input (`draw` target, 2026-11-04): an inline image stating 255,555,555 × 2
/// grey pixels with about 45 bytes of data.
const FUZZED: &[u8] = b"BI /W 255555555 /H 2 /CS /G /BPC 8 ID \x00\x40\x80\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xffminus\xff\xff\xff\xff\xff\xff\xff\x00\xff\xff\xff\xff EI";

/// A one-page document drawing `content`, with picture `im` as `/Im1` if given.
fn page(content: &[u8], im: Option<Stream>) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 200.0, 200.0).unwrap();
    let pid = p.page_ids()[0];
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.to_vec()));
    let mut res = Dictionary::new();
    if let Some(im) = im {
        let id = p.doc.add_object(im);
        res.set("XObject", dictionary! { "Im1" => id });
    }
    let d = p.doc.get_dictionary_mut(pid).unwrap();
    d.set("Contents", Object::Reference(s));
    d.set("Resources", res);
    p
}

const DRAW: &[u8] = b"q 100 0 0 100 50 50 cm /Im1 Do Q";

/// A grey picture `w` × `h` with `data`, compressed.
fn grey(w: i64, h: i64, data: Vec<u8>) -> Stream {
    let mut s = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => w,
            "Height" => h,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
        },
        data,
    );
    s.compress().unwrap();
    s
}

/// The first page's content in `bytes`, decoded.
fn content_of(bytes: &[u8]) -> Vec<u8> {
    let doc = Document::load_mem(bytes).unwrap();
    let pid = *doc.get_pages().values().next().unwrap();
    doc.get_page_content(pid)
}

/// `/Im1`'s stated width in `bytes`.
fn width_of_im1(bytes: &[u8]) -> i64 {
    let doc = Document::load_mem(bytes).unwrap();
    let pid = *doc.get_pages().values().next().unwrap();
    let page = doc.get_dictionary(pid).unwrap();
    let res = doc.dereference(page.get(b"Resources").unwrap()).unwrap().1;
    let xo = res.as_dict().unwrap().get(b"XObject").unwrap();
    let xo = doc.dereference(xo).unwrap().1.as_dict().unwrap();
    let im = doc.dereference(xo.get(b"Im1").unwrap()).unwrap().1;
    im.as_stream()
        .unwrap()
        .dict
        .get(b"Width")
        .unwrap()
        .as_i64()
        .unwrap()
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// A stated size far past its data, as a picture object: 255,555,555 × 2 with 45 bytes.
fn unfillable_object() -> Pdf {
    page(DRAW, Some(grey(255_555_555, 2, vec![0x80; 45])))
}

/// A fax picture stating 100,000 rows with 20 bytes of data, which can hold at most 160
/// (each row takes at least a bit).
fn unfillable_fax() -> Pdf {
    let s = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 100_000,
            "Height" => 100_000,
            "ImageMask" => true,
            "Filter" => "CCITTFaxDecode",
            "DecodeParms" => dictionary! { "K" => -1, "Columns" => 100_000, "Rows" => 100_000 },
        },
        vec![0; 20],
    );
    page(DRAW, Some(s))
}

#[test]
fn snapshot_leaves_out_an_inline_image_its_data_cant_fill() {
    let mut p = page(FUZZED, None);
    for bytes in [p.render_snapshot().unwrap(), p.app_snapshot().unwrap()] {
        assert!(
            !contains(&content_of(&bytes), b"255555555"),
            "the picture stating 255,555,555 pixels across is still drawn"
        );
    }
    let own = p.doc.get_page_content(p.page_ids()[0]);
    assert!(
        contains(&own, b"255555555"),
        "the document itself was changed"
    );
}

#[test]
fn snapshot_leaves_out_a_picture_object_its_data_cant_fill() {
    let mut p = unfillable_object();
    assert!(width_of_im1(&p.render_snapshot().unwrap()) <= 1);
    assert!(width_of_im1(&p.uncropped_app_snapshot().unwrap()) <= 1);
    assert_eq!(width_of_im1(&p.snapshot().unwrap()), 255_555_555);
}

#[test]
fn snapshot_leaves_out_a_fax_picture_stating_more_rows_than_its_data_holds() {
    let mut p = unfillable_fax();
    assert!(width_of_im1(&p.render_snapshot().unwrap()) <= 1);
}

/// A fax picture `columns` wide and `rows` tall, with data enough for its rows (each takes
/// at least a bit): hayro sizes a row from its width alone.
fn wide_fax(columns: i64, rows: i64) -> Pdf {
    let s = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => columns,
            "Height" => rows,
            "ImageMask" => true,
            "Filter" => "CCITTFaxDecode",
            "DecodeParms" => dictionary! { "K" => -1, "Columns" => columns, "Rows" => rows },
        },
        vec![0; 200],
    );
    page(DRAW, Some(s))
}

#[test]
fn snapshot_leaves_out_a_fax_picture_too_wide_to_draw() {
    let mut p = wide_fax(1 << 30, 16);
    assert!(width_of_im1(&p.render_snapshot().unwrap()) <= 1);
    assert_eq!(width_of_im1(&p.snapshot().unwrap()), 1 << 30);
    // 2^17 columns is as wide as a fax picture is drawn, at 200 rows past the size
    // pictures are checked from.
    let mut p = wide_fax(1 << 17, 200);
    assert_eq!(width_of_im1(&p.render_snapshot().unwrap()), 1 << 17);
    let mut p = wide_fax((1 << 17) + 1, 200);
    assert!(width_of_im1(&p.render_snapshot().unwrap()) <= 1);
}

#[test]
fn an_inline_image_stating_a_size_past_counting_is_read_by_its_end() {
    // Its length, height × width × components, overflows: before, a panic.
    for content in [
        &b"q BI /W 18446744073709551615 /H 2 /BPC 8 /CS /RGB ID xyz EI Q"[..],
        b"q BI /W 8 /H 18446744073709551615 /BPC 1 /CS /G ID xyz EI Q",
    ] {
        let mut p = page(content, None);
        let objects = p.page_objects(0).unwrap();
        assert_eq!(objects.len(), 1, "{}", String::from_utf8_lossy(content));
        let drawn = p.drawing_snapshot().unwrap();
        assert!(contains(&content_of(&p.snapshot().unwrap()), b"ID xyz EI"));
        render::load(drawn, None).unwrap();
    }
}

#[test]
fn snapshot_keeps_a_large_picture_whose_data_is_there() {
    // 20 million grey pixels, all there (compressed to a few kilobytes).
    let mut p = page(DRAW, Some(grey(5000, 4000, vec![0; 5000 * 4000])));
    assert_eq!(width_of_im1(&p.render_snapshot().unwrap()), 5000);
}

#[test]
fn snapshot_keeps_a_large_picture_half_there() {
    let mut p = page(DRAW, Some(grey(5000, 4000, vec![0x40; 5000 * 2000])));
    assert_eq!(width_of_im1(&p.render_snapshot().unwrap()), 5000);
}

#[test]
fn snapshot_keeps_a_small_picture_a_little_short() {
    let mut p = page(DRAW, Some(grey(10, 10, vec![0x40; 5])));
    assert_eq!(width_of_im1(&p.render_snapshot().unwrap()), 10);
}

#[test]
fn snapshot_checks_again_what_changed_since_it_was_found_fine() {
    let mut p = page(DRAW, Some(grey(5000, 4000, vec![0; 5000 * 4000])));
    assert_eq!(width_of_im1(&p.render_snapshot().unwrap()), 5000);
    // The same picture and the same content, made unfillable under the same numbers.
    let pid = p.page_ids()[0];
    let res = p
        .doc
        .get_dictionary(pid)
        .unwrap()
        .get(b"Resources")
        .unwrap();
    let xo = res
        .as_dict()
        .unwrap()
        .get(b"XObject")
        .unwrap()
        .as_dict()
        .unwrap();
    let im = xo.get(b"Im1").unwrap().as_reference().unwrap();
    let bad = grey(255_555_555, 2, vec![0x80; 45]);
    p.doc.objects.insert(im, Object::Stream(bad));
    assert!(width_of_im1(&p.render_snapshot().unwrap()) <= 1);

    let mut p = page(DRAW, None);
    p.render_snapshot().unwrap();
    let content = p.doc.get_page_contents(p.page_ids()[0])[0];
    let fuzzed = Stream::new(Dictionary::new(), FUZZED.to_vec());
    p.doc.objects.insert(content, Object::Stream(fuzzed));
    let bytes = p.app_snapshot().unwrap();
    assert!(!contains(&content_of(&bytes), b"255555555"));
}

#[test]
fn snapshot_keeps_reading_the_picture_as_an_object() {
    let p = page(FUZZED, None);
    let objects = p.page_objects(0).unwrap();
    assert!(
        objects.iter().any(|o| o.inline),
        "the inline image isn't listed as an object"
    );
}

/// Drawn as the app draws the page: the drawing finishes and the picture is reported as
/// one that couldn't be shown.
fn drawn_without_the_picture(p: &mut Pdf) {
    let hp = render::load(p.app_snapshot().unwrap(), None).unwrap();
    let r = render::Renderer::new(&hp);
    r.render(0, 1.0).unwrap();
    assert!(
        r.take_missing().contains(&Missing::Picture),
        "the left-out picture isn't reported"
    );
}

#[test]
fn drawing_leaves_out_an_inline_image_its_data_cant_fill() {
    drawn_without_the_picture(&mut page(FUZZED, None));
}

#[test]
fn drawing_leaves_out_a_picture_object_its_data_cant_fill() {
    drawn_without_the_picture(&mut unfillable_object());
}

#[test]
fn drawing_leaves_out_a_fax_picture_its_data_cant_fill() {
    drawn_without_the_picture(&mut unfillable_fax());
}

#[test]
fn drawing_leaves_it_out_of_the_page_under_text_being_edited() {
    let bytes = unfillable_object().page_without(0, &[]).unwrap();
    assert!(width_of_im1(&bytes) <= 1);
    let p = page(FUZZED, None);
    let bytes = p.page_without(0, &[]).unwrap();
    assert!(!contains(&content_of(&bytes), b"255555555"));
}

#[test]
fn drawing_leaves_it_out_of_pages_printed_as_pictures() {
    let paper = Paper {
        id: "page".into(),
        label: "page".into(),
        size: (200.0, 200.0),
        margins: Some([0.0; 4]),
    };
    let setup = PageSetup {
        paper,
        orientation: Orientation::Automatic,
        scale: Scale::ShrinkOversized,
        per_sheet: PerSheet::default(),
        booklet: None,
    };
    for p in [unfillable_object(), page(FUZZED, None)] {
        assert!(!p.print_data(&[0], &setup, true).unwrap().is_empty());
    }
}
