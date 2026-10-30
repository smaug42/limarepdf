//! The document tabs with many documents open: one row of one height, each tab shrinking to
//! a readable width as more open and the row scrolling past that, by the wheel or the arrows
//! at its right, with a menu of every open document; the tab in front always in view.

use super::*;
use egui::accesskit::{Node, NodeId, Role, Toggled, TreeUpdate};

fn test_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    ctx
}

/// An app with nothing open, reading and writing none of the user's settings.
fn empty_app() -> App {
    App::with(Settings::from_file(None), Recent::from_file(None))
}

/// The arrows scrolling the tabs and the button listing every open document.
const LEFT: &str = "Scroll tabs left";
const RIGHT: &str = "Scroll tabs right";
const ALL: &str = "All open documents";

/// The tab strip's margin at each side (`theme::tabs_frame`).
const MARGIN: f32 = 10.0;

/// Adds `n` tabs of one page with long file names, nothing unsaved, the last one in front;
/// their names.
fn long_tabs(app: &mut App, ctx: &egui::Context, n: usize) -> Vec<String> {
    (0..n)
        .map(|i| {
            pages_tab(app, ctx, 1);
            let name = format!("Quarterly report for the northern region {:02}.pdf", i + 1);
            let path = PathBuf::from(format!("/nonexistent/{name}"));
            app.tabs.last_mut().unwrap().path = Some(path);
            name
        })
        .collect()
}

/// The window `width` points wide once settled: a narrow window's panels fold over many
/// frames.
fn drawn(app: &mut App, ctx: &egui::Context, width: f32) -> TreeUpdate {
    for _ in 0..40 {
        window(app, ctx, width, vec![]);
    }
    window(app, ctx, width, vec![])
}

fn bounds(n: &Node) -> egui::Rect {
    let b = n.bounds().expect("a place");
    egui::Rect::from_min_max(
        egui::pos2(b.x0 as f32, b.y0 as f32),
        egui::pos2(b.x1 as f32, b.y1 as f32),
    )
}

/// How far down the tab strip reaches: from the window's top to the commands under it
/// (their File menu's row).
fn strip(tree: &TreeUpdate) -> egui::Rangef {
    let (file, _) = node(tree, "File").expect("the commands");
    egui::Rangef::new(0.0, file.top())
}

/// One document tab: its name, where it is and whether it is the one in front.
struct Shown {
    name: String,
    rect: egui::Rect,
    front: bool,
}

/// The document tabs, left to right (the band's names, tabs too, are below the strip).
fn tabs(tree: &TreeUpdate) -> Vec<Shown> {
    let row = strip(tree);
    let mut out: Vec<Shown> = (tree.nodes.iter())
        .filter(|(_, n)| n.role() == Role::Tab && n.bounds().is_some())
        .map(|(_, n)| Shown {
            name: n.label().unwrap_or_default().to_string(),
            rect: bounds(n),
            front: n.is_selected() == Some(true),
        })
        .filter(|t| row.contains(t.rect.center().y))
        .collect();
    out.sort_by(|a, b| a.rect.left().total_cmp(&b.rect.left()));
    out
}

fn front(tree: &TreeUpdate) -> Shown {
    let mut all = tabs(tree).into_iter();
    all.find(|t| t.front).expect("the tab in front")
}

/// Across the part of the strip the tabs are seen in, in a window `width` points wide: left
/// of the arrows and the documents button when they are shown.
fn view(tree: &TreeUpdate, width: f32) -> egui::Rangef {
    let mut right = width - MARGIN;
    for name in [LEFT, RIGHT, ALL] {
        if let Some((b, _)) = node(tree, name) {
            right = right.min(b.left());
        }
    }
    egui::Rangef::new(MARGIN, right)
}

/// Whether `r` is all in the part of the strip the tabs are seen in.
fn in_view(tree: &TreeUpdate, width: f32, r: egui::Rect) -> bool {
    let v = view(tree, width);
    r.left() >= v.min - 0.5 && r.right() <= v.max + 0.5
}

fn is_disabled(tree: &TreeUpdate, name: &str) -> bool {
    let found = tree.nodes.iter().find(|(_, n)| n.label() == Some(name));
    found.expect(name).1.is_disabled()
}

/// Clicks the middle of `r`, in a window `width` points wide, and settles.
fn click_in(app: &mut App, ctx: &egui::Context, width: f32, r: egui::Rect) -> TreeUpdate {
    window(app, ctx, width, click_at(r.center()));
    drawn(app, ctx, width)
}

/// Clicks the control called `name`, in a window `width` points wide, and settles.
fn click_on(app: &mut App, ctx: &egui::Context, width: f32, name: &str) -> TreeUpdate {
    let tree = drawn(app, ctx, width);
    let (r, _) = node(&tree, name).unwrap_or_else(|| panic!("no {name}"));
    click_in(app, ctx, width, r)
}

/// The texts drawn in the window `width` points wide, settled.
fn texts_drawn(app: &mut App, ctx: &egui::Context, width: f32) -> Vec<String> {
    drawn(app, ctx, width);
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width, TEST_HEIGHT),
        )),
        ..Default::default()
    };
    let mut frame = eframe::Frame::_new_kittest();
    let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut frame));
    out.textures_delta.clear();
    fn walk(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for c in &out.shapes {
        walk(&c.shape, &mut texts);
    }
    texts
}

/// The bug the user found: with several documents open the tab strip wrapped the last
/// tab's name letter by letter and grew down over the window. Many long-named tabs in a
/// narrow window keep one row, as high as one tab's, each tab one line high and the one in
/// front in view.
#[test]
fn many_long_named_tabs_keep_one_row_in_a_narrow_window() {
    let ctx = test_ctx();
    let mut one = empty_app();
    long_tabs(&mut one, &ctx, 1);
    let tree = drawn(&mut one, &ctx, 700.0);
    let (row, tab) = (strip(&tree), front(&tree).rect);
    for width in [700.0, 1000.0, 1400.0] {
        let ctx = test_ctx();
        let mut app = empty_app();
        let names = long_tabs(&mut app, &ctx, 12);
        let tree = drawn(&mut app, &ctx, width);
        let at = strip(&tree);
        let high = (at.span() - row.span()).abs() < 0.5;
        assert!(high, "at {width}: the strip is {at:?}, one tab's {row:?}");
        let shown = tabs(&tree);
        let shown_names: Vec<&str> = shown.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(shown_names, names, "at {width}");
        for t in &shown {
            let (name, r) = (&t.name, t.rect);
            let level = (r.top() - tab.top()).abs() < 0.5;
            let one_line = level && (r.height() - tab.height()).abs() < 0.5;
            assert!(one_line, "at {width}: {name} is {r:?}, one tab {tab:?}");
        }
        let f = front(&tree);
        let seen = in_view(&tree, width, f.rect);
        assert!(seen, "at {width}: {:?} not in view", f.rect);
    }
}

/// The tabs shrink as more open, sharing the room, and stop at a width that still shows the
/// start of each name; past that the row scrolls. A few fit at their own widths.
#[test]
fn tabs_shrink_to_share_the_room_then_scroll() {
    let ctx = test_ctx();
    let mut app = empty_app();
    long_tabs(&mut app, &ctx, 3);
    let tree = drawn(&mut app, &ctx, 1400.0);
    let wide: Vec<f32> = tabs(&tree).iter().map(|t| t.rect.width()).collect();
    assert!(node(&tree, ALL).is_none() && node(&tree, RIGHT).is_none());
    assert!(wide.iter().all(|w| (w - wide[0]).abs() < 0.5), "{wide:?}");
    assert!(wide[0] <= 240.5, "{wide:?}: wider than a tab takes");
    // Five fill 1000 points between them, narrower than three.
    let mut app = empty_app();
    long_tabs(&mut app, &ctx, 5);
    let tree = drawn(&mut app, &ctx, 1000.0);
    let shared: Vec<f32> = tabs(&tree).iter().map(|t| t.rect.width()).collect();
    assert!(shared[0] < wide[0] - 1.0, "{shared:?} beside {wide:?}");
    assert!(node(&tree, ALL).is_none(), "five fit 1000 points");
    let all_seen = tabs(&tree).iter().all(|t| in_view(&tree, 1000.0, t.rect));
    assert!(all_seen, "{shared:?}");
    // Twelve in 700 points don't: each is as narrow as a tab goes, and the row scrolls.
    let mut app = empty_app();
    long_tabs(&mut app, &ctx, 12);
    let tree = drawn(&mut app, &ctx, 700.0);
    let least: Vec<f32> = tabs(&tree).iter().map(|t| t.rect.width()).collect();
    let narrowest = least.iter().all(|w| (150.0..=170.0).contains(w));
    assert!(narrowest, "{least:?}");
    assert!(node(&tree, ALL).is_some() && node(&tree, LEFT).is_some());
    let seen = (tabs(&tree).iter())
        .filter(|t| in_view(&tree, 700.0, t.rect))
        .count();
    assert!((1..12).contains(&seen), "{seen} in view");
}

/// The tab brought to the front is scrolled into view, whatever brings it there: choosing
/// it, a document opened, the one in front closed, or the window narrowed.
#[test]
fn the_tab_in_front_is_always_scrolled_into_view() {
    let ctx = test_ctx();
    let mut app = empty_app();
    let names = long_tabs(&mut app, &ctx, 12);
    for front_at in [11, 0, 6, 11, 3] {
        app.active = front_at;
        let tree = drawn(&mut app, &ctx, 700.0);
        let f = front(&tree);
        assert_eq!(f.name, names[front_at]);
        assert!(in_view(&tree, 700.0, f.rect), "{}: {:?}", f.name, f.rect);
    }
    // A document opened comes in front at the end, in view.
    app.active = 0;
    drawn(&mut app, &ctx, 700.0);
    pages_tab(&mut app, &ctx, 1);
    let tree = drawn(&mut app, &ctx, 700.0);
    let f = front(&tree);
    assert_eq!(f.name, "Untitled");
    assert!(in_view(&tree, 700.0, f.rect), "the new tab at {:?}", f.rect);
    // Closing the one in front brings the one before it, in view.
    let tree = click_on(&mut app, &ctx, 700.0, "Close Untitled");
    let f = front(&tree);
    assert_eq!(f.name, names[11]);
    assert!(in_view(&tree, 700.0, f.rect), "after closing: {:?}", f.rect);
    // In front in a wide window, then the window narrowed.
    app.active = 7;
    let tree = drawn(&mut app, &ctx, 2600.0);
    assert!(in_view(&tree, 2600.0, front(&tree).rect), "wide");
    let tree = drawn(&mut app, &ctx, 600.0);
    let f = front(&tree);
    assert_eq!(f.name, names[7]);
    assert!(in_view(&tree, 600.0, f.rect), "narrowed: {:?}", f.rect);
}

/// A tab half out of view is brought into it by clicking it.
#[test]
fn a_tab_half_out_of_view_comes_into_view_when_chosen() {
    let ctx = test_ctx();
    let mut app = empty_app();
    let names = long_tabs(&mut app, &ctx, 12);
    app.active = 0;
    let tree = drawn(&mut app, &ctx, 700.0);
    let v = view(&tree, 700.0);
    let all = tabs(&tree);
    let half = all.iter().position(|t| t.rect.right() > v.max + 1.0);
    let i = half.expect("a tab half out of view");
    let r = all[i].rect;
    let p = egui::pos2(v.max - 30.0, r.center().y);
    assert!(r.left() < p.x, "{r:?} not under {p:?}");
    window(&mut app, &ctx, 700.0, click_at(p));
    let tree = drawn(&mut app, &ctx, 700.0);
    let f = front(&tree);
    assert_eq!(f.name, names[i]);
    assert!(in_view(&tree, 700.0, f.rect), "{:?}", f.rect);
}

fn rect_of(tree: &TreeUpdate, id: NodeId) -> egui::Rect {
    let (_, n) = tree.nodes.iter().find(|(i, _)| *i == id).expect("the node");
    bounds(n)
}

/// The documents button shows only while the tabs don't all fit; its menu lists every open
/// document in tab order, the one in front chosen and the unsaved ones saying so, and
/// choosing one brings it to the front, in view.
#[test]
fn the_documents_button_lists_every_document_and_brings_one_to_the_front() {
    let ctx = test_ctx();
    let mut few = empty_app();
    named_tabs(&mut few, &ctx);
    let tree = drawn(&mut few, &ctx, 1400.0);
    assert!(node(&tree, ALL).is_none(), "shown with three short tabs");
    let mut app = empty_app();
    let mut names = long_tabs(&mut app, &ctx, 11);
    let path = PathBuf::from("/nonexistent/Notes on the northern region budget.pdf");
    unsaved_tab(&mut app, &ctx, Some(path));
    names.push("Notes on the northern region budget.pdf".into());
    app.active = 4;
    let tree = drawn(&mut app, &ctx, 700.0);
    let (r, _) = node(&tree, ALL).expect("the documents button");
    assert!(r.right() <= 700.0 - MARGIN + 0.5, "{r:?} past the strip");
    let tree = click_in(&mut app, &ctx, 700.0, r);
    // The menu's items, top to bottom.
    let document = |label: &str| names.iter().any(|n| label.starts_with(n.as_str()));
    let mut items: Vec<(f32, String, Option<Toggled>)> = (tree.nodes.iter())
        .filter(|(_, n)| n.role() == Role::RadioButton && n.bounds().is_some())
        .map(|(_, n)| {
            let label = n.label().unwrap_or_default().to_string();
            (bounds(n).top(), label, n.toggled())
        })
        .filter(|(_, label, _)| document(label))
        .collect();
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    let items: Vec<_> = items.into_iter().map(|(_, l, on)| (l, on)).collect();
    let mut want: Vec<(String, Option<Toggled>)> = (names.iter())
        .map(|n| (n.clone(), Some(Toggled::False)))
        .collect();
    want[4].1 = Some(Toggled::True);
    want[11].0.push_str(", unsaved");
    assert_eq!(items, want);
    // Choosing the tenth brings it to the front, in view, and closes the menu.
    let (pick, _) = (tree.nodes.iter())
        .find(|(_, n)| n.role() == Role::RadioButton && n.label() == Some(names[9].as_str()))
        .expect("the tenth");
    let tree = click_in(&mut app, &ctx, 700.0, rect_of(&tree, *pick));
    assert_eq!(app.active, 9);
    assert!(!egui::Popup::is_any_open(&ctx), "the menu stayed open");
    let f = front(&tree);
    assert_eq!(f.name, names[9]);
    assert!(in_view(&tree, 700.0, f.rect), "{:?}", f.rect);
}

/// A wheel step over the tabs scrolls them sideways (an upright wheel too, as in web
/// browsers), and the arrows scroll them about a tab at a time, each greyed at its end.
#[test]
fn the_wheel_and_the_arrows_scroll_the_tabs_greyed_at_their_ends() {
    let ctx = test_ctx();
    let mut app = empty_app();
    let names = long_tabs(&mut app, &ctx, 12);
    app.active = 0;
    let tree = drawn(&mut app, &ctx, 700.0);
    let first = |tree: &TreeUpdate| tabs(tree)[0].rect.left();
    let start = first(&tree);
    assert!(is_disabled(&tree, LEFT), "scrolls left at the start");
    assert!(!is_disabled(&tree, RIGHT));
    // The wheel turned down over the tabs.
    let over = front(&tree).rect.center();
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -120.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers::NONE,
    };
    window(&mut app, &ctx, 700.0, vec![egui::Event::PointerMoved(over)]);
    window(&mut app, &ctx, 700.0, vec![wheel]);
    let tree = drawn(&mut app, &ctx, 700.0);
    let moved = start - first(&tree);
    assert!(moved > 60.0, "the wheel moved the tabs {moved} points");
    assert!(!is_disabled(&tree, LEFT), "greyed after scrolling");
    // The arrow right, to the end: a tab or so each time, then greyed with the last tab in
    // view.
    let mut tree = tree;
    let mut clicks = 0;
    while !is_disabled(&tree, RIGHT) {
        let before = first(&tree);
        tree = click_on(&mut app, &ctx, 700.0, RIGHT);
        let step = before - first(&tree);
        assert!(step > 20.0 && step < 300.0, "a step of {step}");
        clicks += 1;
        assert!(clicks < 20, "never reached the end");
    }
    let last = tabs(&tree).pop().unwrap();
    assert_eq!(last.name, names[11]);
    let r = last.rect;
    assert!(in_view(&tree, 700.0, r), "the last tab: {r:?}");
    assert!(!is_disabled(&tree, LEFT));
    // Scrolling leaves the tab in front as it was.
    assert_eq!(app.active, 0);
    // And back to the start.
    while !is_disabled(&tree, LEFT) {
        tree = click_on(&mut app, &ctx, 700.0, LEFT);
        clicks += 1;
        assert!(clicks < 40, "never got back");
    }
    assert!((first(&tree) - start).abs() < 0.5, "back at the start");
}

/// Closing tabs until the rest fit hides the arrows and the documents button.
#[test]
fn closing_tabs_until_they_fit_hides_the_arrows_and_the_documents_button() {
    let ctx = test_ctx();
    let mut app = empty_app();
    long_tabs(&mut app, &ctx, 12);
    let mut tree = drawn(&mut app, &ctx, 1000.0);
    assert!(node(&tree, ALL).is_some() && node(&tree, RIGHT).is_some());
    let mut hidden = false;
    while app.tabs.len() > 2 {
        let f = front(&tree);
        tree = click_on(&mut app, &ctx, 1000.0, &format!("Close {}", f.name));
        let shown = [LEFT, RIGHT, ALL].map(|n| node(&tree, n).is_some());
        assert!(shown.iter().all(|s| *s == shown[0]), "{shown:?}");
        if hidden {
            assert!(!shown[0], "shown again with {} tabs", app.tabs.len());
        }
        hidden = !shown[0];
        let all_seen = tabs(&tree).iter().all(|t| in_view(&tree, 1000.0, t.rect));
        assert_eq!(all_seen, hidden, "with {} tabs", app.tabs.len());
    }
    assert!(hidden, "still shown with two tabs");
}

/// Each tab's full name is its name for screen readers ("unsaved" after it while it has
/// changes to save) and shows on hover, however short the tab.
#[test]
fn a_tab_keeps_its_full_name_for_screen_readers_and_on_hover() {
    let ctx = test_ctx();
    let mut app = empty_app();
    let mut names = long_tabs(&mut app, &ctx, 11);
    let path = PathBuf::from("/nonexistent/Notes on the northern region budget.pdf");
    unsaved_tab(&mut app, &ctx, Some(path));
    names.push("Notes on the northern region budget.pdf".into());
    app.active = 11;
    let tree = drawn(&mut app, &ctx, 700.0);
    let mut want = names.clone();
    want[11].push_str(", unsaved");
    let said: Vec<String> = tabs(&tree).into_iter().map(|t| t.name).collect();
    assert_eq!(said, want);
    // Shortened on screen.
    let texts = texts_drawn(&mut app, &ctx, 700.0);
    for n in &names {
        assert!(!texts.contains(n), "{n} drawn in full at 700 points");
    }
    let cut = |t: &String| t.starts_with("Notes on") && t.ends_with('…');
    assert!(texts.iter().any(cut), "no shortened name in {texts:?}");
    // The full name on hover, the pointer kept still on the tab's name.
    let time = std::cell::Cell::new(10.0);
    let frame = |app: &mut App, events: Vec<egui::Event>| {
        time.set(time.get() + 0.05);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(700.0, TEST_HEIGHT),
            )),
            time: Some(time.get()),
            events,
            ..Default::default()
        };
        let mut f = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut f));
        out.textures_delta.clear();
        out.platform_output.accesskit_update.expect("the window")
    };
    let f = front(&frame(&mut app, vec![]));
    let p = egui::pos2(f.rect.left() + 60.0, f.rect.center().y);
    frame(&mut app, vec![egui::Event::PointerMoved(p)]);
    let mut tree = frame(&mut app, vec![]);
    for _ in 0..30 {
        tree = frame(&mut app, vec![]);
    }
    let tip = (tree.nodes.iter()).any(|(_, n)| n.value().is_some_and(|v| v.contains(&names[11])));
    assert!(tip, "no tooltip naming {}", names[11]);
}

/// Names cut short in a narrow window show in full again once it is widened.
#[test]
fn a_window_widened_again_shows_the_names_in_full() {
    let ctx = test_ctx();
    let mut app = empty_app();
    let names: Vec<String> = (1..=6)
        .map(|i| {
            pages_tab(&mut app, &ctx, 1);
            let name = format!("Budget, part {i}.pdf");
            let path = PathBuf::from(format!("/nonexistent/{name}"));
            app.tabs.last_mut().unwrap().path = Some(path);
            name
        })
        .collect();
    let texts = texts_drawn(&mut app, &ctx, 700.0);
    for n in &names {
        assert!(!texts.contains(n), "{n} in full at 700 points");
    }
    let texts = texts_drawn(&mut app, &ctx, 1400.0);
    for n in &names {
        assert!(texts.contains(n), "{n} not in full in {texts:?}");
    }
}

/// Every character drawn in the tab strip and the documents menu, the arrows' carets, the
/// "…" of a shortened name and the dot for "Unsaved" among them, has a glyph in its font.
#[test]
fn no_character_in_the_tab_strip_lacks_a_glyph() {
    let ctx = test_ctx();
    let mut app = empty_app();
    let mut names = long_tabs(&mut app, &ctx, 11);
    let path = PathBuf::from("/nonexistent/Notes on the northern region budget.pdf");
    unsaved_tab(&mut app, &ctx, Some(path));
    names.push("Notes on the northern region budget.pdf".into());
    app.active = 4;
    let tree = click_on(&mut app, &ctx, 700.0, ALL);
    assert!(egui::Popup::is_any_open(&ctx), "the menu isn't open");
    let below = strip(&tree).max;
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(700.0, TEST_HEIGHT),
        )),
        ..Default::default()
    };
    let mut frame = eframe::Frame::_new_kittest();
    let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(&mut app, ui, &mut frame));
    out.textures_delta.clear();
    // Each text drawn in the strip or naming a document: its font and its text.
    fn walk(s: &egui::Shape, out: &mut Vec<(egui::Pos2, egui::FontId, String)>) {
        match s {
            egui::Shape::Text(t) => {
                let job = &t.galley.job;
                for section in &job.sections {
                    let r = &section.byte_range;
                    let text = job.text[r.start.0..r.end.0].to_string();
                    out.push((t.pos, section.format.font_id.clone(), text));
                }
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for c in &out.shapes {
        walk(&c.shape, &mut texts);
    }
    let ours = |p: &egui::Pos2, text: &str| {
        let start = text.trim_end_matches('…');
        let named = names.iter().any(|n| n.starts_with(start));
        p.y < below || named || text == "Unsaved"
    };
    let texts: Vec<_> = texts.into_iter().filter(|(p, _, t)| ours(p, t)).collect();
    let shown: Vec<&str> = texts.iter().map(|(_, _, t)| t.as_str()).collect();
    let cut = shown.iter().any(|t| t.ends_with('…'));
    assert!(cut, "nothing shortened");
    assert!(shown.contains(&icon::CARET_LEFT), "no arrow");
    assert!(shown.contains(&icon::CIRCLE), "no unsaved dot");
    let mut missing = Vec::new();
    for (_, font, text) in &texts {
        for c in text.chars().filter(|c| !c.is_whitespace()) {
            if !ctx.fonts_mut(|f| f.has_glyph(font, c)) {
                missing.push(format!("{c} (U+{:04X}) in {text:?}", c as u32));
            }
        }
    }
    assert!(missing.is_empty(), "no glyph for {missing:?}");
}

/// An unsaved tab says so on screen while it has room for the word, and its name for screen
/// readers says it whatever its width.
#[test]
fn an_unsaved_tab_says_so() {
    let ctx = test_ctx();
    let mut app = empty_app();
    named_tabs(&mut app, &ctx);
    unsaved_tab(&mut app, &ctx, Some(PathBuf::from("/nonexistent/d.pdf")));
    let texts = texts_drawn(&mut app, &ctx, 1400.0);
    assert!(texts.iter().any(|t| t == "Unsaved"), "{texts:?}");
    let tree = drawn(&mut app, &ctx, 1400.0);
    let said: Vec<String> = tabs(&tree).into_iter().map(|t| t.name).collect();
    assert_eq!(said, ["a.pdf", "b.pdf", "c.pdf", "d.pdf, unsaved"]);
    long_tabs(&mut app, &ctx, 12);
    let tree = drawn(&mut app, &ctx, 600.0);
    let said: Vec<String> = tabs(&tree).into_iter().map(|t| t.name).collect();
    assert!(said.contains(&"d.pdf, unsaved".to_string()), "{said:?}");
}

/// The row's parts are measured: the strip and the commands under it keep their places from
/// frame to frame, many tabs or few, at every width (an egui row sized from the room left
/// can widen its panel each frame).
#[test]
fn the_tab_row_keeps_its_size_from_frame_to_frame() {
    let ctx = test_ctx();
    for n in [1, 3, 12, 30] {
        let mut app = empty_app();
        long_tabs(&mut app, &ctx, n);
        for width in [1400.0, 900.0, 600.0, 420.0] {
            drawn(&mut app, &ctx, width);
            let mut places = Vec::new();
            for _ in 0..6 {
                let tree = window(&mut app, &ctx, width, vec![]);
                let ends: Vec<egui::Rect> = [LEFT, RIGHT, ALL]
                    .iter()
                    .filter_map(|n| node(&tree, n).map(|(r, _)| r))
                    .collect();
                let f = front(&tree);
                let (closer, _) = node(&tree, &format!("Close {}", f.name)).expect("its close");
                places.push((strip(&tree), ends, f.rect, closer));
            }
            let first = places[0].clone();
            let same = places.iter().all(|p| *p == first);
            assert!(same, "{n} at {width}: {places:?}");
            let (_, ends, f, closer) = first;
            let edge = width - MARGIN + 0.5;
            let inside = ends.iter().all(|e| e.right() <= edge);
            assert!(inside, "{n} tabs at {width}: {ends:?}");
            let apart = ends.windows(2).all(|w| w[0].right() <= w[1].left() + 0.5);
            assert!(apart, "{n} tabs at {width}: {ends:?} overlap");
            assert!(f.right() <= edge, "{n} tabs at {width}: {f:?}");
            let held = f.contains_rect(closer);
            assert!(held, "{n} at {width}: {closer:?} out of {f:?}");
        }
    }
}

/// Tab goes through the tabs (each and its close button), then the arrows and the documents
/// button, then the commands; a tab reached by Tab comes into view.
#[test]
fn tab_goes_through_the_tabs_then_the_arrows_and_the_documents_button() {
    let ctx = test_ctx();
    let mut app = empty_app();
    let names = long_tabs(&mut app, &ctx, 12);
    app.active = 0;
    drawn(&mut app, &ctx, 700.0);
    let tab = key_with(Key::Tab, egui::Modifiers::NONE);
    let mut reached: Vec<String> = Vec::new();
    for _ in 0..60 {
        window(&mut app, &ctx, 700.0, vec![tab.clone()]);
        let tree = window(&mut app, &ctx, 700.0, vec![]);
        let Some(name) = focused_name(&ctx, &tree) else {
            continue;
        };
        if name == "File" {
            break;
        }
        // A tab with the keyboard is in view.
        if tabs(&tree).iter().any(|t| t.name == name) {
            let tree = drawn(&mut app, &ctx, 700.0);
            let t = tabs(&tree).into_iter().find(|t| t.name == name).unwrap();
            assert!(in_view(&tree, 700.0, t.rect), "{name} at {:?}", t.rect);
        }
        reached.push(name);
    }
    let mut want = Vec::new();
    for n in &names {
        want.push(n.clone());
        want.push(format!("Close {n}"));
    }
    let at = |name: &str| reached.iter().position(|r| r == name);
    let pos: Vec<Option<usize>> = want.iter().map(|w| at(w)).collect();
    assert!(pos.iter().all(Option::is_some), "{reached:?}");
    let ordered = pos.windows(2).all(|w| w[0] < w[1]);
    assert!(ordered, "out of order: {reached:?}");
    let last = pos.last().unwrap().unwrap();
    let docs = at(ALL).expect("the documents button by Tab");
    assert!(docs > last, "{reached:?}");
    let arrow = at(LEFT).or(at(RIGHT)).expect("an arrow by Tab");
    assert!(arrow > last && arrow < docs, "{reached:?}");
}

/// Ctrl as Linux and Windows send it, with Command; with Shift too.
fn ctrl(shift: bool) -> egui::Modifiers {
    egui::Modifiers {
        ctrl: true,
        command: true,
        shift,
        ..Default::default()
    }
}

/// ENH-43: Ctrl+Tab and Ctrl+F6 bring the next document to the front, with Shift the one
/// before, round from the last to the first and back, the tab coming into view; with the
/// keyboard in a text box too. With one document open they do nothing.
#[test]
fn ctrl_tab_and_ctrl_f6_go_through_the_documents_in_turn() {
    let ctx = test_ctx();
    let mut one = empty_app();
    long_tabs(&mut one, &ctx, 1);
    drawn(&mut one, &ctx, 700.0);
    window(&mut one, &ctx, 700.0, vec![key_with(Key::Tab, ctrl(false))]);
    assert_eq!(one.active, 0, "one document");

    let ctx = test_ctx();
    let mut app = empty_app();
    let names = long_tabs(&mut app, &ctx, 12);
    app.active = 0;
    drawn(&mut app, &ctx, 700.0);
    let steps = [
        (Key::Tab, false, 1),
        (Key::F6, false, 2),
        (Key::Tab, true, 1),
        (Key::F6, true, 0),
        (Key::Tab, true, 11),
        (Key::Tab, false, 0),
        (Key::F6, true, 11),
        (Key::F6, false, 0),
    ];
    for (key, shift, want) in steps {
        window(&mut app, &ctx, 700.0, vec![key_with(key, ctrl(shift))]);
        let tree = drawn(&mut app, &ctx, 700.0);
        assert_eq!(app.active, want, "{key:?} with Shift {shift}");
        let f = front(&tree);
        assert_eq!(f.name, names[want]);
        assert!(in_view(&tree, 700.0, f.rect), "{} at {:?}", f.name, f.rect);
    }

    // The keyboard in the search box.
    window(&mut app, &ctx, 700.0, vec![key_with(Key::F, ctrl(false))]);
    drawn(&mut app, &ctx, 700.0);
    assert!(ctx.text_edit_focused(), "the search box has the keyboard");
    window(&mut app, &ctx, 700.0, vec![key_with(Key::Tab, ctrl(false))]);
    assert_eq!(app.active, 1, "from the search box");
}

/// PB-49: Enter on an arrow with the keyboard scrolls the tabs; at the row's end, the arrow
/// greyed, the keyboard goes to the other arrow rather than nowhere, and so back.
#[test]
fn the_keyboard_stays_on_the_arrows_at_the_row_s_ends() {
    let ctx = test_ctx();
    let mut app = empty_app();
    long_tabs(&mut app, &ctx, 12);
    app.active = 0;
    drawn(&mut app, &ctx, 700.0);
    // Tab comes to the arrows after the last tab, which it brought into view: to the arrow
    // left, the arrow right being greyed then.
    let tab = key_with(Key::Tab, egui::Modifiers::NONE);
    let mut found = false;
    for _ in 0..60 {
        window(&mut app, &ctx, 700.0, vec![tab.clone()]);
        let tree = window(&mut app, &ctx, 700.0, vec![]);
        if focused_name(&ctx, &tree).as_deref() == Some(LEFT) {
            found = true;
            break;
        }
    }
    assert!(found, "Tab reaches the arrow left");
    let enter = key_with(Key::Enter, egui::Modifiers::NONE);
    for (from, to) in [(LEFT, RIGHT), (RIGHT, LEFT)] {
        let mut presses = 0;
        let tree = loop {
            window(&mut app, &ctx, 700.0, vec![enter.clone()]);
            let tree = drawn(&mut app, &ctx, 700.0);
            if is_disabled(&tree, from) {
                break tree;
            }
            assert_eq!(focused_name(&ctx, &tree).as_deref(), Some(from));
            presses += 1;
            assert!(presses < 20, "never reached the end");
        };
        assert!(presses > 0, "{from} scrolled");
        assert_eq!(focused_name(&ctx, &tree).as_deref(), Some(to), "at the end");
    }
    assert_eq!(app.active, 0, "the tab in front stays");
}

/// An arrow held down steps the tabs once at once, then again and again until it is let
/// go, which doesn't step again; one held only briefly steps once.
#[test]
fn holding_an_arrow_keeps_scrolling_the_tabs() {
    let ctx = test_ctx();
    let mut app = empty_app();
    long_tabs(&mut app, &ctx, 30);
    app.active = 0;
    let tree = drawn(&mut app, &ctx, 700.0);
    let first = |tree: &TreeUpdate| tabs(tree)[0].rect.left();
    let start = first(&tree);
    let (r, _) = node(&tree, RIGHT).expect("the arrow right");
    let p = r.center();
    window(&mut app, &ctx, 700.0, vec![egui::Event::PointerMoved(p)]);
    // Each frame here is 1/60 of a second on; the tabs are drawn as they were scrolled the
    // frame before.
    window(&mut app, &ctx, 700.0, vec![press(p, true)]);
    let tree = window(&mut app, &ctx, 700.0, vec![]);
    let once = start - first(&tree);
    assert!(once > 20.0 && once < 300.0, "a step of {once}");
    let mut tree = tree;
    for _ in 0..17 {
        tree = window(&mut app, &ctx, 700.0, vec![]);
    }
    let short = start - first(&tree);
    assert!(
        (short - once).abs() < 0.5,
        "a short hold steps once: {short}"
    );
    for _ in 0..42 {
        tree = window(&mut app, &ctx, 700.0, vec![]);
    }
    let long = start - first(&tree);
    assert!(
        long > once * 4.0,
        "a second's hold moved {long}, a step {once}"
    );
    let tree = window(&mut app, &ctx, 700.0, vec![press(p, false)]);
    let held = start - first(&tree);
    let tree = drawn(&mut app, &ctx, 700.0);
    let after = start - first(&tree);
    assert!(
        (after - held).abs() < 0.5,
        "letting go stepped: {after} after {held}"
    );
    assert_eq!(app.active, 0, "the tab in front stays");
}
