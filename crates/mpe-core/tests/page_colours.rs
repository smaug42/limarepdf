use mpe_core::page_colours::PageColours;
use mpe_core::{render, Pdf};

fn png(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(c));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn px(b: &render::Bitmap, x: usize, y: usize) -> [u8; 3] {
    let i = (y * b.width + x) * 4;
    [b.rgba[i], b.rgba[i + 1], b.rgba[i + 2]]
}

/// A 300 × 200 page with a green picture at (200, 100)–(260, 150).
fn page_with_picture() -> Vec<u8> {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    p.add_image(
        0,
        [200.0, 100.0, 260.0, 150.0],
        &png(8, 8, [0, 200, 0, 255]),
    )
    .unwrap();
    p.snapshot().unwrap()
}

#[test]
fn dark_pages_keep_pictures_unless_asked() {
    let hp = render::load(page_with_picture(), None).unwrap();
    let r = render::Renderer::new(&hp);
    let normal = r.render(0, 2.0).unwrap();
    let kept = r.render_in(0, 2.0, PageColours::Dark, true).unwrap();
    let all = r.render_in(0, 2.0, PageColours::Dark, false).unwrap();
    // Paper turns dark in both.
    assert_eq!(px(&kept, 20, 20), PageColours::Dark.paper());
    assert_eq!(px(&all, 20, 20), PageColours::Dark.paper());
    // The picture's middle (230, 125 pt) is untouched only when pictures are kept.
    let green = px(&normal, 460, 250);
    assert!(green[1] > 150 && green[0] < 50, "{green:?}");
    assert_eq!(px(&kept, 460, 250), green);
    assert_eq!(
        px(&all, 460, 250),
        PageColours::Dark.pixel(green),
        "pictures recoloured when asked"
    );
}

#[test]
fn normal_is_the_plain_render() {
    let hp = render::load(page_with_picture(), None).unwrap();
    let r = render::Renderer::new(&hp);
    let a = r.render(0, 1.0).unwrap();
    let b = r.render_in(0, 1.0, PageColours::Normal, true).unwrap();
    assert_eq!(a.rgba, b.rgba);
}
