//! Drawing part of a page (`Renderer::render_region`), as the app does in tiles at high
//! zoom: a region must match the same pixels of the whole page drawn at that scale.

use limarepdf_core::page_colours::PageColours;
use limarepdf_core::render::{self, Bitmap, Region};
use limarepdf_core::Pdf;
use lopdf::{dictionary, Object, Stream};

/// A 300 × 200 page: a red box at (100, 50)–(200, 150), a diagonal blue line, and text.
fn vector_page() -> Vec<u8> {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let content = "1 0 0 rg 100 50 100 100 re f \
                   0 0 1 RG 2 w 10 10 m 290 190 l S \
                   BT /F1 18 Tf 0 g 20 170 Td (Tiles) Tj ET";
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let s = p
        .doc
        .add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", Object::Reference(s));
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => Object::Reference(font) } },
    );
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
    let whole = r.render_in(0, scale, PageColours::Dark, true).unwrap();
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
    let dir = std::env::temp_dir().join(format!("limarepdf-huge-{}", std::process::id()));
    let e = p
        .export_images(&[0], 72.0, render::ImageFormat::Png, &dir, "page")
        .expect_err("too large");
    assert!(e.to_string().contains("too large"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}
