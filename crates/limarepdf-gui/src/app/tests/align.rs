//! Align, distribute and snap (NEW-08, batch 46's stage 5): Edit > Arrange's Align menu
//! lines the selected objects up by each other, one alone by the page, and spaces three or
//! more out, each one Undo; objects moved or resized snap to the page's and the other
//! objects' edges and middles with a guide drawn, Alt for a free drag, Snap to objects off
//! for none.

use super::*;
use crate::editor::{Align, Axis};
use limarepdf_core::render::{self, Bitmap, ImageFormat};
use limarepdf_core::{ObjectKind, PathStyle, Shape};

/// The photo (page points, y down).
const PHOTO: [f32; 4] = [100.0, 100.0, 300.0, 250.0];
/// Three grey boxes, for spacing out.
const SHAPES: [[f32; 4]; 3] = [
    [60.0, 400.0, 100.0, 440.0],
    [130.0, 460.0, 150.0, 480.0],
    [400.0, 420.0, 480.0, 460.0],
];

/// The fixture's objects: their indices on the page.
struct Ids {
    photo: usize,
    caption: usize,
    shapes: [usize; 3],
}

/// A tab of a Letter page with a photo (a green picture) at [`PHOTO`], its caption under it
/// ("A walrus on the shore", 12-point Helvetica from 150 on baseline 270) and the grey boxes
/// [`SHAPES`], with the arrow.
fn photo_tab() -> (egui::Context, App, Ids) {
    let (ctx, mut app) = super::paragraphs::new_app();
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    let grey = PathStyle {
        fill: Some([0.6, 0.6, 0.6]),
        stroke: None,
        line_width: 1.0,
    };
    for b in SHAPES {
        pdf.add_shape(0, Shape::Rect, b, &grey).unwrap();
    }
    let green = Bitmap {
        width: 2,
        height: 2,
        rgba: [0, 160, 0, 255].repeat(4),
    };
    let png = render::encode(&green, ImageFormat::Png, 72.0).unwrap();
    pdf.add_image(0, PHOTO, &png).unwrap();
    let (caption, font) = ("A walrus on the shore", FontChoice::default());
    pdf.add_text(0, (150.0, 270.0), caption, font, 12.0, [0.0; 3])
        .unwrap();
    let objs = pdf.page_objects(0).unwrap();
    let find = |f: &dyn Fn(&limarepdf_core::PageObject) -> bool| {
        objs.iter().position(f).expect("the object")
    };
    let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(&b).all(|(p, q)| (p - q).abs() < 1.5);
    let ids = Ids {
        photo: find(&|o| o.kind == ObjectKind::Image),
        caption: find(&|o| o.kind == ObjectKind::Text),
        shapes: SHAPES.map(|s| find(&|o| o.kind == ObjectKind::Path && near(o.bbox, s))),
    };
    assert!(app.add_tab(&ctx, pdf, None, None, false));
    app.act(&ctx, Act::Tool(Tool::Select));
    settle(&mut app, &ctx);
    (ctx, app, ids)
}

/// The middle of box `b` across and down.
fn mid(b: [f32; 4]) -> (f32, f32) {
    ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0)
}

/// Whether two boxes are the same, within `tol` points.
fn same(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    a.iter().zip(&b).all(|(p, q)| (p - q).abs() < tol)
}

/// Whether a control called `name` is in `tree` (a menu item's name ends in its empty key).
fn listed(tree: &egui::accesskit::TreeUpdate, name: &str) -> bool {
    (tree.nodes.iter()).any(|(_, n)| n.label().map(str::trim_end) == Some(name))
}

/// Click the menu item called `name`, as a user does, then let the window carry it out.
fn click_item(app: &mut App, ctx: &egui::Context, name: &str) {
    let tree = settled_at(app, ctx, 1400.0);
    let item = (tree.nodes.iter())
        .find(|(_, n)| n.label().map(str::trim_end) == Some(name))
        .and_then(|(_, n)| n.bounds())
        .unwrap_or_else(|| panic!("no {name}"));
    let (x, y) = ((item.x0 + item.x1) / 2.0, (item.y0 + item.y1) / 2.0);
    window(app, ctx, 1400.0, click_at(egui::pos2(x as f32, y as f32)));
    settle(app, ctx);
}

/// Run command `act`, then let the window carry it out.
fn run(app: &mut App, ctx: &egui::Context, act: Act) {
    app.act(ctx, act);
    settle(app, ctx);
}

/// Drag from page point `from` to `to` with `keys` held: the guides drawn just before the
/// release.
fn drag_with(
    app: &mut App,
    ctx: &egui::Context,
    from: (f32, f32),
    to: (f32, f32),
    keys: egui::Modifiers,
) -> Vec<(bool, f32)> {
    let (min, k) = page_on_screen(app, ctx);
    let at = |(x, y): (f32, f32)| min + egui::vec2(x, y) * k;
    let (a, b) = (at(from), at(to));
    window_held(app, ctx, vec![egui::Event::PointerMoved(a)], keys);
    window_held(app, ctx, vec![press(a, true)], keys);
    for i in 1..=4 {
        let p = a + (b - a) * (i as f32 / 4.0);
        window_held(app, ctx, vec![egui::Event::PointerMoved(p)], keys);
    }
    let guides = app.tabs[0].ed.guides();
    window_held(app, ctx, vec![press(b, false)], keys);
    window_held(app, ctx, vec![], egui::Modifiers::NONE);
    settle(app, ctx);
    guides
}

/// A photo and its caption, both selected, line up by each other: centred, the caption
/// moves under the photo's middle and the photo, the wider, stays; aligned left, the
/// caption starts where the photo does. Each is one Undo, both stay selected, and the
/// caption keeps its size.
#[test]
fn a_photo_and_its_caption_line_up_with_one_undo() {
    let (ctx, mut app, ids) = photo_tab();
    let (photo, caption) = (ids.photo, ids.caption);
    select(&mut app, &ctx, &[photo, caption]);
    let before = boxes(&app, 0);
    let steps = app.tabs[0].history().1;
    run(&mut app, &ctx, Act::Align(Align::Centre));
    let after = boxes(&app, 0);
    assert_eq!(app.tabs[0].history().1, steps + 1, "one undo step");
    assert!(same(after[photo], before[photo], 0.05), "the photo moved");
    let (was, now) = (before[caption], after[caption]);
    assert!((mid(now).0 - 200.0).abs() < 0.05, "{now:?}");
    assert!((now[1] - was[1]).abs() < 0.05, "it moved down: {now:?}");
    let width = |b: [f32; 4]| b[2] - b[0];
    assert!((width(now) - width(was)).abs() < 0.05, "resized");
    let both = Some((0, vec![photo, caption]));
    assert_eq!(held(&app), both, "still selected");

    run(&mut app, &ctx, Act::Align(Align::Left));
    assert!((boxes(&app, 0)[caption][0] - 100.0).abs() < 0.05);
    assert_eq!(app.tabs[0].history().1, steps + 2);
    // Nothing else on the page moved.
    let now = boxes(&app, 0);
    for s in ids.shapes {
        assert!(same(now[s], before[s], 0.05), "box {s}");
    }
    run(&mut app, &ctx, Act::Undo);
    assert!((mid(boxes(&app, 0)[caption]).0 - 200.0).abs() < 0.05);
    run(&mut app, &ctx, Act::Undo);
    assert!(same(boxes(&app, 0)[caption], was, 0.05), "Undo put it back");
}

/// One object alone lines up by the page: its right edge at the page's, centred on it, its
/// foot at the page's.
#[test]
fn one_object_alone_lines_up_by_the_page() {
    let (ctx, mut app, ids) = photo_tab();
    select(&mut app, &ctx, &[ids.photo]);
    run(&mut app, &ctx, Act::Align(Align::Right));
    let b = boxes(&app, 0)[ids.photo];
    assert!(same(b, [412.0, 100.0, 612.0, 250.0], 0.05), "{b:?}");
    run(&mut app, &ctx, Act::Align(Align::Centre));
    assert!((mid(boxes(&app, 0)[ids.photo]).0 - 306.0).abs() < 0.05);
    run(&mut app, &ctx, Act::Align(Align::Bottom));
    let b = boxes(&app, 0)[ids.photo];
    assert!(same(b, [206.0, 642.0, 406.0, 792.0], 0.05), "{b:?}");
    // Already there: no change, no step.
    let steps = app.tabs[0].history().1;
    run(&mut app, &ctx, Act::Align(Align::Bottom));
    assert_eq!(app.tabs[0].history().1, steps);
}

/// Three boxes spaced out evenly across, then down: the outer two stay, the one between
/// has equal gaps either side, one Undo each. Two selected, Distribute is greyed and says
/// why; with nothing the tool can move selected, Align says why.
#[test]
fn distribute_spaces_three_out_the_outer_two_staying() {
    let (ctx, mut app, ids) = photo_tab();
    let [a, b, c] = ids.shapes;
    let across = actions::get(Act::Distribute(Axis::Across));
    select(&mut app, &ctx, &[a, b]);
    let state = app.state(&ctx);
    assert!(!(across.enabled)(&state), "two selected");
    assert_eq!(
        actions::why_not(&state, across),
        "Select three or more objects first."
    );
    select(&mut app, &ctx, &[c, a, b]);
    let steps = app.tabs[0].history().1;
    run(&mut app, &ctx, Act::Distribute(Axis::Across));
    let now = boxes(&app, 0);
    assert!(same(now[a], SHAPES[0], 0.05) && same(now[c], SHAPES[2], 0.05));
    // 420 across, 140 of it the boxes: two gaps of 140.
    let between = [240.0, 460.0, 260.0, 480.0];
    assert!(same(now[b], between, 0.05), "{:?}", now[b]);
    assert_eq!(app.tabs[0].history().1, steps + 1);
    // Down: the first at the top and the last at the bottom stay.
    run(&mut app, &ctx, Act::Distribute(Axis::Down));
    let now = boxes(&app, 0);
    let between = [400.0, 430.0, 480.0, 470.0];
    assert!(same(now[c], between, 0.05), "{:?}", now[c]);
    assert!(same(now[a], SHAPES[0], 0.05) && (now[b][3] - 480.0).abs() < 0.05);

    // With the read tool, page text selected doesn't move: Align is greyed, and says so.
    app.act(&ctx, Act::Tool(Tool::Read));
    select(&mut app, &ctx, &[ids.caption]);
    let state = app.state(&ctx);
    let left = actions::get(Act::Align(Align::Left));
    assert!(!(left.enabled)(&state));
    assert_eq!(
        actions::why_not(&state, left),
        "Select something the tool in use can move first."
    );
}

/// The caption dragged to two points right of the photo's left edge snaps onto it, a guide
/// drawn along both while dragging; with Alt held it goes where it is dragged, no guide;
/// with Snap to objects off, the same, and that is remembered as the app's setting.
#[test]
fn a_moved_object_snaps_to_another_unless_alt_is_held() {
    let (ctx, mut app, ids) = photo_tab();
    let caption = ids.caption;
    select(&mut app, &ctx, &[caption]);
    let was = boxes(&app, 0)[caption];
    let (x, y) = mid(was);
    // Its left edge to 102 across, as dragged.
    let to = (x - (was[0] - 102.0), y);
    let none = egui::Modifiers::NONE;
    let guides = drag_with(&mut app, &ctx, (x, y), to, none);
    let now = boxes(&app, 0)[caption];
    assert!((now[0] - 100.0).abs() < 0.05, "not snapped: {now:?}");
    assert!((now[1] - was[1]).abs() < 0.05, "moved down: {now:?}");
    assert!(guides.contains(&(true, 100.0)), "{guides:?}");
    assert!(app.tabs[0].ed.guides().is_empty(), "the guide stayed");
    run(&mut app, &ctx, Act::Undo);

    let guides = drag_with(&mut app, &ctx, (x, y), to, egui::Modifiers::ALT);
    let now = boxes(&app, 0)[caption];
    assert!((now[0] - 102.0).abs() < 0.6, "snapped with Alt: {now:?}");
    assert!(guides.is_empty(), "{guides:?}");
    run(&mut app, &ctx, Act::Undo);

    run(&mut app, &ctx, Act::ToggleSnap);
    assert!(!app.settings.snap && !app.tabs[0].ed.snap);
    let ticked = actions::get(Act::ToggleSnap).checked.unwrap();
    assert!(!ticked(&app.state(&ctx)));
    let guides = drag_with(&mut app, &ctx, (x, y), to, none);
    let now = boxes(&app, 0)[caption];
    assert!((now[0] - 102.0).abs() < 0.6, "snapped when off: {now:?}");
    assert!(guides.is_empty(), "{guides:?}");
    run(&mut app, &ctx, Act::ToggleSnap);
    assert!(app.settings.snap);
}

/// The photo's right handle dragged to two points past a box's left edge snaps onto it,
/// the photo's other edges staying, a guide along the edge moved.
#[test]
fn a_resized_object_snaps_the_edge_it_moves() {
    let (ctx, mut app, ids) = photo_tab();
    select(&mut app, &ctx, &[ids.photo]);
    let guides = drag_with(
        &mut app,
        &ctx,
        (300.0, 175.0),
        (402.0, 175.0),
        egui::Modifiers::NONE,
    );
    let b = boxes(&app, 0)[ids.photo];
    assert!(same(b, [100.0, 100.0, 400.0, 250.0], 0.05), "{b:?}");
    assert_eq!(guides, [(true, 400.0)]);
}

/// Edit > Arrange's Align opens Word's Align menu, Snap to objects in it ticked; the
/// selected objects' right-click menu has Align with the commands that apply; the command
/// search finds them.
#[test]
fn align_is_in_the_band_the_right_click_menu_and_the_search() {
    let (ctx, mut app, ids) = photo_tab();
    select(&mut app, &ctx, &[ids.photo, ids.caption]);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(node(&tree, "Align — Planned").is_none(), "still planned");
    click_named(&mut app, &ctx, "Align");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    for name in [
        "Align left",
        "Align centre",
        "Align right",
        "Align top",
        "Align middle",
        "Align bottom",
        "Distribute horizontally",
        "Distribute vertically",
        "Snap to objects",
    ] {
        assert!(listed(&tree, name), "no {name}");
    }
    let snap = (tree.nodes.iter()).find(|(_, n)| n.label() == Some("Snap to objects"));
    let ticked = snap.and_then(|(_, n)| n.toggled());
    assert_eq!(ticked, Some(egui::accesskit::Toggled::True));
    // A click on one runs it, as a menu item does, and closes the menu.
    click_item(&mut app, &ctx, "Align left");
    assert!(!egui::Popup::is_any_open(&ctx), "the menu stayed open");
    assert!((boxes(&app, 0)[ids.caption][0] - 100.0).abs() < 0.05);

    // Right-clicked, the two selected: Align, with all but Distribute.
    let sel = Selection::Objects {
        page: 0,
        held: Vec::new(),
        loose: false,
    };
    let groups = actions::context_items(&sel);
    assert!(groups.contains(&&actions::ALIGNS[..]), "not in the menu");
    let (min, k) = page_on_screen(&mut app, &ctx);
    let p = min + egui::vec2(200.0, 175.0) * k;
    let button = |pressed| egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: Default::default(),
    };
    let right = vec![egui::Event::PointerMoved(p), button(true), button(false)];
    window(&mut app, &ctx, 1400.0, right);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(held(&app), Some((0, vec![ids.photo, ids.caption])));
    // The menu's Align, below the band's.
    let at = (tree.nodes.iter())
        .filter(|(_, n)| n.label() == Some("Align"))
        .filter_map(|(_, n)| n.bounds())
        .max_by(|a, b| a.y0.total_cmp(&b.y0))
        .filter(|b| b.y0 as f32 > min.y)
        .expect("no Align in the menu");
    let (x, y) = ((at.x0 + at.x1) / 2.0, (at.y0 + at.y1) / 2.0);
    let at = egui::pos2(x as f32, y as f32);
    window(&mut app, &ctx, 1400.0, vec![egui::Event::PointerMoved(at)]);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(listed(&tree, "Align right"), "the submenu didn't open");
    assert!(!listed(&tree, "Distribute horizontally"), "two selected");

    // The floating bar keeps to its own few buttons, as Word's: no Align there.
    select(&mut app, &ctx, &[ids.photo, ids.caption, ids.shapes[0]]);
    let bar = actions::toolbar_items(&app.tabs[0].ed.selection, &app.state(&ctx));
    let arranged = |a: &&&Action| matches!(a.act, Act::Align(_) | Act::Distribute(_));
    let on_bar: Vec<_> = (bar.iter().flatten()).filter(arranged).collect();
    assert!(on_bar.is_empty(), "on the floating bar: {}", on_bar.len());

    let found: Vec<Act> = (actions::ACTIONS.iter())
        .filter(|a| a.palette && actions::matches(a, "align centre"))
        .map(|a| a.act)
        .collect();
    assert_eq!(found, [Act::Align(Align::Centre)]);
    let spread = |q| (actions::ACTIONS.iter()).any(|a| a.palette && actions::matches(a, q));
    assert!(spread("distribute") && spread("snap"));
}
