//! Drawing part of a page (`Renderer::render_region`), as the app does in tiles at high
//! zoom: a region must match the same pixels of the whole page drawn at that scale.

#[allow(dead_code)]
#[path = "common/tool.rs"]
mod tool;

#[path = "common/seeds.rs"]
mod seeds;

use limarepdf_core::page_colours::PageColours;
use limarepdf_core::render::{self, Bitmap, Region};
use limarepdf_core::Pdf;
use lopdf::{dictionary, Object, Stream};

/// A 300 × 200 page: a red box at (100, 50)–(200, 150), a diagonal blue line, and text.
fn vector_page() -> Vec<u8> {
    vector_page_at(1.0, false)
}

/// The vector page `k` times the size, all it draws too; `masked`, drawn through two soft
/// masks that hide nothing, so that its passes are weighed.
fn vector_page_at(k: f32, masked: bool) -> Vec<u8> {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0 * k, 200.0 * k).unwrap();
    let (m0, m1) = if masked {
        ("/M0 gs ", "/M1 gs ")
    } else {
        ("", "")
    };
    let content = format!(
        "{k} 0 0 {k} 0 0 cm {m0}1 0 0 rg 100 50 100 100 re f \
         {m1}0 0 1 RG 2 w 10 10 m 290 190 l S \
         BT /F1 18 Tf 0 g 20 170 Td (Tiles) Tj ET"
    );
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let s = p
        .doc
        .add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
    let mut resources = dictionary! { "Font" => dictionary! { "F1" => Object::Reference(font) } };
    if masked {
        let mut states = lopdf::Dictionary::new();
        for (name, white) in [("M0", "0 0 300 200"), ("M1", "-1 -1 302 202")] {
            let white = p.doc.add_object(Stream::new(
                dictionary! {
                    "Type" => "XObject",
                    "Subtype" => "Form",
                    "BBox" => vec![(-1).into(), (-1).into(), 301.into(), 201.into()],
                    "Group" => dictionary! { "S" => "Transparency", "CS" => "DeviceGray" },
                },
                format!("1 g {white} re f").into_bytes(),
            ));
            let mask = p.doc.add_object(dictionary! {
                "SMask" => dictionary! { "S" => "Luminosity", "G" => Object::Reference(white) },
            });
            states.set(name, Object::Reference(mask));
        }
        resources.set("ExtGState", states);
    }
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", Object::Reference(s));
    d.set("Resources", resources);
    p.snapshot().unwrap()
}

fn px(b: &Bitmap, x: usize, y: usize) -> [u8; 3] {
    let i = (y * b.width + x) * 4;
    [b.rgba[i], b.rgba[i + 1], b.rgba[i + 2]]
}

fn region(x: u32, y: u32, width: u32, height: u32) -> Region {
    Region {
        x,
        y,
        width,
        height,
    }
}

#[test]
fn tiles_match_the_whole_page() {
    let hp = render::load(vector_page(), None).unwrap();
    let r = render::Renderer::new(&hp);
    let scale = 3.0;
    let whole = r.render(0, scale).unwrap();
    assert_eq!((whole.width, whole.height), (900, 600));
    let tile = 256;
    for ty in (0..600).step_by(tile) {
        for tx in (0..900).step_by(tile) {
            let reg = region(tx as u32, ty as u32, tile as u32, tile as u32);
            let t = r.render_region(0, scale, reg).unwrap();
            // The last row and column are cut to the page.
            assert_eq!(t.width, tile.min(900 - tx));
            assert_eq!(t.height, tile.min(600 - ty));
            for y in 0..t.height {
                for x in 0..t.width {
                    assert_eq!(
                        px(&t, x, y),
                        px(&whole, tx + x, ty + y),
                        "pixel {}, {}",
                        tx + x,
                        ty + y
                    );
                }
            }
        }
    }
}

/// The masked vector page twice the size, 2400 × 1600 pixels at the scale given, and the
/// side of the pieces it's drawn in on this thread, the memory of a pass held to a little
/// over 1024 pixels a side.
fn pieced_page() -> (Vec<u8>, f32, usize) {
    // A pixel takes the masks, and in a piece the piece's bitmap: 7.7 MB in one pass, 6.3 MB
    // in pieces of 1024 and 9.8 MB in pieces of 1280.
    render::pass_seam::hold_to(6.5 * 1024.0 * 1024.0);
    (vector_page_at(2.0, true), 4.0, 1024)
}

#[test]
fn pieces_match_one_pass() {
    let (page, scale, side) = pieced_page();
    let hp = render::load(page, None).unwrap();
    let r = render::Renderer::new(&hp);
    let pieced = r.render(0, scale).unwrap();
    assert_eq!((pieced.width, pieced.height), (2400, 1600));
    assert_eq!(render::pass_seam::last_side(), Some(side as u32));
    // Tiles drawn in one pass each, none starting on an edge between the pieces, so every
    // edge runs through some tile. Drawn from another origin, an edge pixel can come out a
    // level apart, as the app's tiles can; no more.
    let tile = 512;
    assert!((side - tile / 2) % tile != 0);
    for ty in (tile / 2..1600).step_by(tile) {
        for tx in (tile / 2..2400).step_by(tile) {
            let reg = region(tx as u32, ty as u32, tile as u32, tile as u32);
            let one = r.render_region(0, scale, reg).unwrap();
            for y in 0..one.height {
                for x in 0..one.width {
                    let (gx, gy) = (tx + x, ty + y);
                    let (a, b) = (px(&one, x, y), px(&pieced, gx, gy));
                    let apart = (0..3).all(|i| a[i].abs_diff(b[i]) <= 1);
                    assert!(apart, "pixel {gx}, {gy}: {a:?}, {b:?}");
                }
            }
        }
    }
}

#[test]
fn a_page_without_masks_or_patterns_is_drawn_in_one_pass() {
    // Held to a byte, the masked page is drawn in the smallest pieces, the other in one.
    render::pass_seam::hold_to(1.0);
    for (masked, side) in [(true, 256), (false, 2400)] {
        let hp = render::load(vector_page_at(2.0, masked), None).unwrap();
        render::Renderer::new(&hp).render(0, 4.0).unwrap();
        assert_eq!(render::pass_seam::last_side(), Some(side), "{masked}");
    }
}

#[test]
fn drawing_stops_between_pieces() {
    let (page, scale, side) = pieced_page();
    let pieces = 2400usize.div_ceil(side) * 1600usize.div_ceil(side);
    let hp = render::load(page, None).unwrap();
    let r = render::Renderer::new(&hp);
    // Asked before each piece.
    let asked = std::cell::Cell::new(0);
    let b = r
        .render_unless(0, scale, &|| {
            asked.set(asked.get() + 1);
            false
        })
        .unwrap();
    assert!(b.is_some());
    assert_eq!(asked.get(), pieces);
    // Told to stop before the second piece.
    asked.set(0);
    let b = r
        .render_unless(0, scale, &|| {
            asked.set(asked.get() + 1);
            asked.get() > 1
        })
        .unwrap();
    assert!(b.is_none());
    assert_eq!(asked.get(), 2);
    // A page that fits in one piece is asked once.
    asked.set(0);
    let b = r
        .render_unless(0, 1.0, &|| {
            asked.set(asked.get() + 1);
            false
        })
        .unwrap()
        .unwrap();
    assert_eq!((b.width, b.height), (600, 400));
    assert_eq!(asked.get(), 1);
}

#[test]
fn a_region_outside_the_page_is_an_error() {
    let hp = render::load(vector_page(), None).unwrap();
    let r = render::Renderer::new(&hp);
    assert!(r.render_region(0, 1.0, region(300, 0, 10, 10)).is_err());
    assert!(r.render_region(0, 1.0, region(0, 0, 0, 10)).is_err());
    assert!(r.render_region(1, 1.0, region(0, 0, 10, 10)).is_err());
}

#[test]
fn edges_land_in_place_at_the_highest_scale() {
    let hp = render::load(vector_page(), None).unwrap();
    let r = render::Renderer::new(&hp);
    let s = render::MAX_REGION_SCALE;
    // The red box's left edge, 100 pt in, at 25600 px; its bottom (y = 50 pt up from the
    // bottom of a 200 pt page) at 150 pt down.
    let (edge, mid) = ((100.0 * s) as u32, (100.0 * s) as u32);
    let t = r.render_region(0, s, region(edge - 8, mid, 16, 4)).unwrap();
    assert_eq!(px(&t, 2, 1), [255, 255, 255]);
    assert_eq!(px(&t, 12, 1), [255, 0, 0]);
    let bottom = (150.0 * s) as u32;
    let t = r
        .render_region(0, s, region(150 * s as u32, bottom - 8, 4, 16))
        .unwrap();
    assert_eq!(px(&t, 1, 2), [255, 0, 0]);
    assert_eq!(px(&t, 1, 12), [255, 255, 255]);
}

fn png(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(c));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn dark_tiles_keep_pictures_where_the_page_does() {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    p.add_image(
        0,
        [200.0, 100.0, 260.0, 150.0],
        &png(8, 8, [0, 200, 0, 255]),
    )
    .unwrap();
    let hp = render::load(p.snapshot().unwrap(), None).unwrap();
    let r = render::Renderer::new(&hp);
    let scale = 4.0;
    let whole = r
        .render_in(0, scale, PageColours::Dark, true, &|| false)
        .unwrap()
        .unwrap();
    // A tile over the picture's left edge (200 pt = 800 px) and middle row (125 pt down =
    // 500 px).
    let t = r
        .render_region_in(0, scale, region(760, 480, 80, 40), PageColours::Dark, true)
        .unwrap();
    for y in 0..t.height {
        for x in 0..t.width {
            assert_eq!(px(&t, x, y), px(&whole, 760 + x, 480 + y), "pixel {x}, {y}");
        }
    }
    assert_eq!(px(&t, 10, 20), PageColours::Dark.paper());
    let green = px(&t, 60, 20);
    assert!(green[1] > 150 && green[0] < 50, "{green:?}");
}

#[test]
fn a_huge_page_is_an_error_not_a_huge_bitmap() {
    // A page 10 million points a side: even at the smallest scale, the bitmap would be
    // 65,535 pixels a side (17 GB).
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let page = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("MediaBox", vec![0.into(), 0.into(), 1e7.into(), 1e7.into()]);
    let hp = render::load(p.snapshot().unwrap(), None).unwrap();
    let r = render::Renderer::new(&hp);
    for scale in [0.0, 1.0, 16.0] {
        let e = r.render(0, scale).err().expect("too large");
        assert!(e.to_string().contains("too large"), "{e}");
    }
    let whole = region(0, 0, u32::MAX, u32::MAX);
    assert!(r.render_region(0, 1.0, whole).is_err());
    // A tile of it still draws, as the app shows it at high zoom.
    let t = r.render_region(0, 1.0, region(0, 0, 512, 512)).unwrap();
    assert_eq!((t.width, t.height), (512, 512));
    // Exporting it fails cleanly too.
    let (_scratch, dir) = export_dir("huge");
    let e = p
        .export_images(
            &[0],
            72.0,
            render::ImageFormat::Png,
            &dir,
            "page",
            &mut Default::default(),
        )
        .expect_err("too large");
    assert!(e.to_string().contains("too large"), "{e}");
}

/// A document of pages `sizes` (points), each with a red box 10 pt square in its far
/// corner: top right, `rotate` degrees turned.
fn red_far_off(sizes: &[(f32, f32)], rotate: i64) -> Pdf {
    let mut p = Pdf::new();
    for (k, &(w, h)) in sizes.iter().enumerate() {
        p.insert_blank(k, 300.0, 200.0).unwrap();
        let content = format!("1 0 0 rg {} {} 10 10 re f", w - 10.0, h - 10.0);
        let s = p
            .doc
            .add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page = p.page_ids()[k];
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("MediaBox", vec![0.into(), 0.into(), w.into(), h.into()]);
        d.set("Contents", Object::Reference(s));
        d.set("Rotate", rotate);
    }
    p
}

/// A folder of its own for one test's export, not made yet, in a scratch folder kept as long
/// as the first.
fn export_dir(name: &str) -> (tool::Scratch, std::path::PathBuf) {
    let scratch = tool::Scratch::new(name);
    let dir = scratch.join("export");
    (scratch, dir)
}

/// The files in `dir`, none when it isn't there.
fn files_in(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |d| d.count())
}

#[test]
fn a_page_too_tall_or_wide_for_its_bitmap_is_refused_whole() {
    // At 576 dpi, 10 × 10,000 pt is 80 × 80,000 pixels and 10,000 × 10 pt the other way:
    // few pixels, but one side past what the rasteriser draws. Before, the tall page came out
    // 65,535 pixels tall without its bottom, and the wide one panicked.
    for (size, rotate) in [
        ((10.0, 10_000.0), 0),
        ((10_000.0, 10.0), 0),
        ((10_000.0, 10.0), 90),
        ((10.0, 10_000.0), 90),
    ] {
        let mut p = red_far_off(&[size], rotate);
        seeds::pdf(&p);
        let hp = render::load(p.snapshot().unwrap(), None).unwrap();
        let e = render::encode_pages(&hp, &[0], 576.0, render::ImageFormat::Png)
            .expect_err("too large");
        assert!(
            e.to_string().contains("too large"),
            "{size:?} {rotate}: {e}"
        );
        let r = render::Renderer::new(&hp);
        assert!(r.render(0, 8.0).is_err(), "{size:?} {rotate}");
        let (_scratch, dir) = export_dir("too-long");
        let e = p
            .export_images(
                &[0],
                576.0,
                render::ImageFormat::Png,
                &dir,
                "page",
                &mut Default::default(),
            )
            .expect_err("too large");
        assert!(
            e.to_string().contains("too large"),
            "{size:?} {rotate}: {e}"
        );
        assert_eq!(files_in(&dir), 0, "{size:?} {rotate}");
    }
}

#[test]
fn a_bitmap_side_of_65280_pixels_is_drawn_whole_and_65281_refused() {
    // The rasteriser's sides are 16-bit, and vello_cpu 0.3 panics on a bitmap more than
    // 65,408 pixels wide (its strips run past the edge): 65,280 is 255 of its 256-pixel
    // wide tiles. A green band runs far past the page's ends, a blue one is clipped short.
    let hp = |w: f32| {
        let mut p = red_far_off(&[(w, 10.0), (10.0, w)], 0);
        let past = format!(
            "q 0 1 0 RG 4 w -500 2 m 1000000 2 l S Q \
             q 5 0 {} 10 re W n 0 0 1 rg 0 0 {w} 10 re f Q 1 0 0 rg {} 0 10 10 re f",
            w - 20.0,
            w - 10.0
        );
        let s = (p.doc).add_object(Stream::new(dictionary! {}, past.into_bytes()));
        let page = p.page_ids()[0];
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", Object::Reference(s));
        render::load(p.snapshot().unwrap(), None).unwrap()
    };
    let most = hp(65_280.0);
    let r = render::Renderer::new(&most);
    let wide = r.render(0, 1.0).unwrap();
    assert_eq!((wide.width, wide.height), (65_280, 10));
    assert_eq!(px(&wide, 65_275, 4), [255, 0, 0], "the far end is drawn");
    assert_eq!(px(&wide, 65_000, 4), [0, 0, 255]);
    // The green band shows where the blue doesn't cover it.
    assert_eq!(px(&wide, 2, 8), [0, 255, 0]);
    assert_eq!(px(&wide, 65_265, 8), [0, 255, 0]);
    let tall = r.render(1, 1.0).unwrap();
    assert_eq!((tall.width, tall.height), (10, 65_280));
    assert_eq!(px(&tall, 5, 4), [255, 0, 0], "the top is drawn");
    assert_eq!(px(&tall, 5, 65_275), [255, 255, 255]);
    let files = render::encode_pages(&most, &[0, 1], 72.0, render::ImageFormat::Png).unwrap();
    for (f, size) in files.iter().zip([(65_280, 10), (10, 65_280)]) {
        let i = image::load_from_memory(f).unwrap();
        assert_eq!((i.width(), i.height()), size);
    }
    // A region wider than that is cut to it, as at the page's edge, not a panic.
    let far = hp(70_000.0);
    let r = render::Renderer::new(&far);
    let wider = r.render_region(0, 1.0, region(0, 0, 70_000, 10)).unwrap();
    assert_eq!((wider.width, wider.height), (65_280, 10));
    let over = hp(65_281.0);
    let r = render::Renderer::new(&over);
    for page in [0, 1] {
        let e = r.render(page, 1.0).err().expect("too large");
        assert!(e.to_string().contains("too large"), "{e}");
        let e = render::encode_pages(&over, &[page], 72.0, render::ImageFormat::Png)
            .expect_err("too large");
        assert!(e.to_string().contains("too large"), "{e}");
    }
}

#[test]
fn an_export_with_one_page_too_large_writes_no_pages() {
    let mut p = red_far_off(&[(300.0, 200.0), (300.0, 200.0), (10.0, 10_000.0)], 0);
    let (_scratch, dir) = export_dir("one-too-long");
    let e = p
        .export_images(
            &[0, 1, 2],
            576.0,
            render::ImageFormat::Png,
            &dir,
            "page",
            &mut Default::default(),
        )
        .expect_err("too large");
    assert!(e.to_string().contains("page 3"), "{e}");
    assert_eq!(files_in(&dir), 0);
}
