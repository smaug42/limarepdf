//! Text edited over a picture, a gradient or coloured boxes shows what is behind it (ENH-21):
//! the old text of a line or a paragraph being edited is hidden by the page drawn without
//! it, which comes from a thread of its own, once for the text edited, and again after a
//! zoom or a change of page colours; the colour around it hides it until then.

use super::paragraphs::{double_click_word, new_app};
use super::*;
use crate::settings::PageLook;
use limarepdf_core::{PathStyle, Shape};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Textures set in the frames drawn, by id.
type Seen = HashMap<egui::TextureId, egui::ColorImage>;

/// A tab of a Letter page with a heading and, below it, four lines of 12-point Helvetica 15
/// points apart, all over two boxes, red on the left and blue on the right, which meet
/// under the lines' middle; with the read tool.
fn lines_on_boxes(app: &mut App, ctx: &egui::Context) {
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    let style = |fill| PathStyle {
        fill: Some(fill),
        stroke: None,
        line_width: 1.0,
    };
    let (red, blue) = ([0.9, 0.1, 0.1], [0.1, 0.2, 0.9]);
    pdf.add_shape(0, Shape::Rect, [50.0, 50.0, 240.0, 220.0], &style(red))
        .unwrap();
    pdf.add_shape(0, Shape::Rect, [240.0, 50.0, 440.0, 220.0], &style(blue))
        .unwrap();
    let lines = [
        "Harbour notes",
        "The harbour was quiet that morning, and the fishing boats",
        "sat low in the water while gulls argued over the nets",
        "drying along the wall. Nobody on the quay was in a hurry;",
        "the tide would turn when it turned.",
    ];
    // The heading at 70, the paragraph's lines from 100, 15 apart.
    for (i, text) in lines.into_iter().enumerate() {
        let y = if i == 0 { 70.0 } else { 85.0 + 15.0 * i as f32 };
        pdf.add_text(0, (72.0, y), text, FontChoice::default(), 12.0, [0.0; 3])
            .unwrap();
    }
    assert!(app.add_tab(ctx, pdf, None, None, false));
    app.act(ctx, Act::Tool(Tool::Read));
}

/// A frame of the window 1400 points wide with `events`, the textures it sets kept in
/// `seen`: what it draws.
fn frame(
    app: &mut App,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    seen: &mut Seen,
) -> Vec<egui::epaint::ClippedShape> {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, TEST_HEIGHT),
        )),
        events,
        ..Default::default()
    };
    let mut f = eframe::Frame::_new_kittest();
    let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut f));
    for (id, deltas) in std::mem::take(&mut out.textures_delta.set) {
        for delta in deltas {
            if let (egui::ImageData::Color(img), None) = (&delta.image, delta.pos) {
                seen.insert(id, (**img).clone());
            }
        }
    }
    out.textures_delta.clear();
    out.shapes
}

/// Frames until the page drawn without the text being edited is shown, other than `not`:
/// its picture, where it goes on the page, and what the frame showing it drew.
fn backdrop_shown(
    app: &mut App,
    ctx: &egui::Context,
    seen: &mut Seen,
    not: Option<egui::TextureId>,
) -> (egui::TextureId, [f32; 4], Vec<egui::epaint::ClippedShape>) {
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        let shapes = frame(app, ctx, vec![], seen);
        if let Some((tex, at)) = app.tabs[app.active].ed.backdrop {
            if Some(tex) != not {
                return (tex, at, shapes);
            }
        }
        assert!(
            Instant::now() < until,
            "the page without the text never came"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Whether `shapes` draw the picture `tex`.
fn draws(shapes: &[egui::epaint::ClippedShape], tex: egui::TextureId) -> bool {
    fn has(s: &egui::Shape, tex: egui::TextureId) -> bool {
        match s {
            egui::Shape::Vec(v) => v.iter().any(|s| has(s, tex)),
            egui::Shape::Mesh(m) => m.texture_id == tex,
            _ => false,
        }
    }
    shapes.iter().any(|c| has(&c.shape, tex))
}

/// How many of `img`'s pixels are mostly red, mostly blue, and dark (text).
fn reds_blues_darks(img: &egui::ColorImage) -> (usize, usize, usize) {
    let mut n = (0, 0, 0);
    for p in &img.pixels {
        let [r, g, b, _] = p.to_array();
        if r > 150 && g < 80 && b < 80 {
            n.0 += 1;
        } else if b > 150 && r < 80 {
            n.1 += 1;
        } else if r < 90 && g < 90 && b < 90 {
            n.2 += 1;
        }
    }
    n
}

/// The size of picture `tex`, in pixels.
fn size(ctx: &egui::Context, tex: egui::TextureId) -> [usize; 2] {
    let meta = ctx.tex_manager().read().meta(tex).map(|m| m.size);
    meta.expect("a picture")
}

/// After a zoom, the page without the text being edited (other than `was`) drawn again at
/// the new scale, as its frames upload it: the picture, its pixels and where it goes.
fn zoomed(
    app: &mut App,
    ctx: &egui::Context,
    was: egui::TextureId,
) -> (egui::TextureId, egui::ColorImage, [f32; 4]) {
    let mut seen = Seen::new();
    let before = size(ctx, was);
    app.act(ctx, Act::ZoomIn);
    let (tex, at, shapes) = backdrop_shown(app, ctx, &mut seen, Some(was));
    assert!(draws(&shapes, tex), "drawn under the typing");
    let img = seen.remove(&tex).expect("uploaded in the frames seen");
    assert!(
        img.size[0] > before[0],
        "{:?} after a zoom, {before:?} before",
        img.size
    );
    (tex, img, at)
}

/// The box of the active tab's first page's lines below the heading, in page points.
fn block_box(app: &App) -> [f32; 4] {
    let objs = app.tabs[app.active].pdf.page_objects(0).unwrap();
    let lines = objs.iter().filter(|o| o.text.is_some() && o.bbox[1] > 80.0);
    let b = lines.map(|o| o.bbox).reduce(|a, b| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    });
    b.expect("the lines")
}

/// A paragraph edited over two boxes: its old lines are hidden by the page drawn without
/// them, both boxes and none of the old text, over the whole block; drawn once while it is
/// typed in, again after a zoom, at the new scale, and in other page colours; gone when
/// the editing ends.
#[test]
fn a_paragraph_edited_over_boxes_hides_its_old_lines_with_the_page_drawn_without_them() {
    let (ctx, mut app) = new_app();
    lines_on_boxes(&mut app, &ctx);
    app.set_paragraphs(true);
    double_click_word(&mut app, &ctx, "fishing");
    assert!(
        app.tabs[0].ed.para_text().is_some(),
        "a paragraph is edited"
    );
    let mut seen = Seen::new();
    let (tex, at, shapes) = backdrop_shown(&mut app, &ctx, &mut seen, None);
    assert!(draws(&shapes, tex), "drawn under the typing");
    let b = block_box(&app);
    assert!(
        at[0] <= b[0] && at[1] <= b[1] && at[2] >= b[2] && at[3] >= b[3],
        "{at:?} doesn't cover the block {b:?}"
    );

    // Typed in: the same drawing, not drawn again.
    window(
        &mut app,
        &ctx,
        1400.0,
        vec![egui::Event::Text(" very".into())],
    );
    for _ in 0..10 {
        frame(&mut app, &ctx, vec![], &mut seen);
    }
    assert_eq!(app.tabs[0].ed.backdrop.map(|b| b.0), Some(tex));

    // After a zoom, drawn again at the new scale for the same place: both boxes, and none
    // of the old text.
    let (tex, img, at2) = zoomed(&mut app, &ctx, tex);
    for (a, b) in at.iter().zip(at2) {
        assert!((a - b).abs() < 1.0, "{at:?} vs {at2:?}");
    }
    let (reds, blues, darks) = reds_blues_darks(&img);
    let all = img.pixels.len();
    assert!(
        reds > all / 4 && blues > all / 4,
        "{reds} red, {blues} blue of {all}"
    );
    assert_eq!(darks, 0, "the old text shows in it");

    // Other page colours: not shown in the old ones, drawn again in the new.
    let dark = PageLook::Fixed(limarepdf_core::page_colours::PageColours::Dark);
    app.act(&ctx, Act::PageColours(dark));
    frame(&mut app, &ctx, vec![], &mut seen);
    assert_ne!(app.tabs[0].ed.backdrop.map(|b| b.0), Some(tex));
    let (darker, ..) = backdrop_shown(&mut app, &ctx, &mut seen, Some(tex));
    let again = seen.get(&darker).expect("uploaded in the frames seen");
    assert_eq!(again.size, img.size);
    assert!(again.pixels != img.pixels, "not recoloured");

    // Kept: nothing hides anything.
    window(
        &mut app,
        &ctx,
        1400.0,
        vec![key_with(Key::Escape, egui::Modifiers::NONE)],
    );
    settled_at(&mut app, &ctx, 1400.0);
    assert!(app.tabs[0].ed.para_text().is_none());
    assert_eq!(app.tabs[0].ed.backdrop, None);
}

/// A line edited over two boxes, the same: hidden by the page drawn without it.
#[test]
fn a_line_edited_over_boxes_hides_its_old_text_with_the_page_drawn_without_it() {
    let (ctx, mut app) = new_app();
    lines_on_boxes(&mut app, &ctx);
    double_click_word(&mut app, &ctx, "gulls");
    let ed = &app.tabs[0].ed;
    assert!(ed.typing_on_page() && ed.para_text().is_none());
    let mut seen = Seen::new();
    let (tex, at, shapes) = backdrop_shown(&mut app, &ctx, &mut seen, None);
    assert!(draws(&shapes, tex));
    // The line's box: the one at 115.
    assert!(
        at[1] < 115.0 - 8.0 && at[3] > 115.0 && at[3] < 125.0,
        "{at:?}"
    );
    let (_, img, _) = zoomed(&mut app, &ctx, tex);
    let (reds, blues, darks) = reds_blues_darks(&img);
    assert!(reds > 0 && blues > 0, "{reds} red, {blues} blue");
    assert_eq!(darks, 0, "the old text shows in it");
}
