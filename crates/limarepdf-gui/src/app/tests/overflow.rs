//! Text typed on the page that runs over something says what, on a tag where it happens
//! (PB-35; batch 45's 3b-2): one rule for a line, a paragraph and new text, against every
//! visible thing on the page the text didn't already overlap (a page-wide background aside)
//! and the page's edge; new text isn't warned about a picture or shape it was started on;
//! the floating bar keeps clear of the tag.

use super::*;
use limarepdf_core::render::{self, Bitmap, ImageFormat};
use limarepdf_core::{PathStyle, Shape};

/// A tab of a Letter page with `lines` (12-point Helvetica, each from its baseline's left
/// end), grey boxes filling `boxes` and green pictures filling `pictures` (displayed-page
/// points, y down), with the read tool.
fn scene_tab(
    app: &mut App,
    ctx: &egui::Context,
    lines: &[(&str, (f32, f32))],
    boxes: &[[f32; 4]],
    pictures: &[[f32; 4]],
) {
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    let grey = PathStyle {
        fill: Some([0.6, 0.6, 0.6]),
        stroke: None,
        line_width: 1.0,
    };
    for &b in boxes {
        pdf.add_shape(0, Shape::Rect, b, &grey).unwrap();
    }
    let green = Bitmap {
        width: 2,
        height: 2,
        rgba: [0, 160, 0, 255].repeat(4),
    };
    let png = render::encode(&green, ImageFormat::Png, 72.0).unwrap();
    for &r in pictures {
        pdf.add_image(0, r, &png).unwrap();
    }
    for &(text, at) in lines {
        pdf.add_text(0, at, text, FontChoice::default(), 12.0, [0.0; 3])
            .unwrap();
    }
    assert!(app.add_tab(ctx, pdf, None, None, false));
    app.act(ctx, Act::Tool(Tool::Read));
}

/// Where the tag saying `why` is on the screen, if one does.
fn tag(tree: &egui::accesskit::TreeUpdate, why: &str) -> Option<egui::Rect> {
    let (_, n) = tree.nodes.iter().find(|(_, n)| n.value() == Some(why))?;
    let b = n.bounds()?;
    Some(egui::Rect::from_min_max(
        egui::pos2(b.x0 as f32, b.y0 as f32),
        egui::pos2(b.x1 as f32, b.y1 as f32),
    ))
}

/// Where page 1 is on the screen: its top left, and pixels a point.
fn page_at(tree: &egui::accesskit::TreeUpdate) -> (egui::Pos2, f32) {
    let (page, _) = node(tree, "Page 1").expect("the page");
    (page.min, page.width() / 612.0)
}

/// `text` typed, then the window settled.
fn typed(app: &mut App, ctx: &egui::Context, text: &str) -> egui::accesskit::TreeUpdate {
    window(app, ctx, 1400.0, vec![egui::Event::Text(text.into())]);
    settled_at(app, ctx, 1400.0)
}

fn esc(app: &mut App, ctx: &egui::Context) {
    let esc = key_with(Key::Escape, egui::Modifiers::NONE);
    window(app, ctx, 1400.0, vec![esc]);
    settled_at(app, ctx, 1400.0);
}

/// A line typed on past its end into a grey box beside it says "Over a shape", and into a
/// picture "Over a picture", on a tag just below the line where it runs into it, at the
/// box's left: not at the outline's end. As it was, nothing is said.
#[test]
fn a_line_typed_into_a_shape_or_a_picture_says_so_where_it_happens() {
    let (ctx, mut app) = super::paragraphs::new_app();
    let lines = [
        ("The otter ate", (72.0, 100.0)),
        ("The seal ate", (72.0, 200.0)),
    ];
    scene_tab(
        &mut app,
        &ctx,
        &lines,
        &[[180.0, 88.0, 240.0, 106.0]],
        &[[180.0, 188.0, 240.0, 206.0]],
    );
    let cases = [
        ("otter", "otter and the walrus", "Over a shape", 100.0),
        ("seal", "seal and the walrus", "Over a picture", 200.0),
    ];
    for (word, text, why, baseline) in cases {
        super::paragraphs::double_click_word(&mut app, &ctx, word);
        assert!(app.tabs[0].ed.editing_line(), "{word}");
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(tag(&tree, why), None, "{word}: as it was");
        let tree = typed(&mut app, &ctx, text);
        let (min, k) = page_at(&tree);
        let at = tag(&tree, why).unwrap_or_else(|| panic!("{word}: no {why:?}"));
        let left = min.x + 180.0 * k;
        assert!(
            (at.min.x - left).abs() < 3.0,
            "{word}: {at:?}, the box at {left}"
        );
        let base = min.y + baseline * k;
        assert!(
            at.min.y > base && at.min.y < base + 12.0 * k,
            "{word}: {at:?}"
        );
        esc(&mut app, &ctx);
    }
}

/// New text started on a coloured box isn't warned about it as it is typed (the user's
/// answer, Q3); new text typed into a box it wasn't started on says "Over a shape", and into
/// a line "Over other text".
#[test]
fn new_text_says_what_it_runs_into_but_not_the_box_it_was_started_on() {
    let (ctx, mut app) = super::paragraphs::new_app();
    let lines = [("Seal", (300.0, 440.0))];
    scene_tab(&mut app, &ctx, &lines, &[[100.0, 300.0, 400.0, 360.0]], &[]);
    let whys = ["Over a shape", "Over other text", "Over a picture"];
    let (shape, text) = (Some("Over a shape"), Some("Over other text"));
    let cases = [
        ((120.0, 340.0), "Walrus and the otter", None),
        ((40.0, 315.0), "Walrus and the otter", shape),
        ((180.0, 440.0), "Walrus and the otter ate", text),
    ];
    for (at, text, want) in cases {
        app.act(&ctx, Act::Tool(Tool::Text));
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let (min, k) = page_at(&tree);
        let p = min + egui::vec2(at.0, at.1) * k;
        window(&mut app, &ctx, 1400.0, click_at(p));
        settled_at(&mut app, &ctx, 1400.0);
        let tree = typed(&mut app, &ctx, text);
        let said = whys.into_iter().find(|w| tag(&tree, w).is_some());
        assert_eq!(said, want, "{text} from {at:?}");
        esc(&mut app, &ctx);
    }
}

/// With no room above a line being edited, the floating bar goes below it, and keeps clear
/// of the tag saying what the line runs over, which sits below the line too.
#[test]
fn the_floating_bar_below_a_line_keeps_clear_of_its_tag() {
    let (ctx, mut app) = super::paragraphs::new_app();
    let lines = [("The otter ate", (72.0, 14.0))];
    scene_tab(&mut app, &ctx, &lines, &[[160.0, 2.0, 230.0, 20.0]], &[]);
    for _ in 0..10 {
        window(&mut app, &ctx, 1400.0, vec![]);
    }
    app.tabs[0].go_to_at(0, 0.0);
    super::paragraphs::double_click_word(&mut app, &ctx, "otter");
    let tree = typed(&mut app, &ctx, "otter and the walrus");
    let at = tag(&tree, "Over a shape").expect("the tag");
    let buttons = bar_buttons(&tree);
    let bar = (buttons.iter().map(|(_, r)| *r))
        .reduce(|a, b| a.union(b))
        .expect("the bar");
    let (below, clear) = (bar.min.y > at.min.y, !bar.intersects(at));
    assert!(below && clear, "the bar {bar:?}, the tag {at:?}");
}
