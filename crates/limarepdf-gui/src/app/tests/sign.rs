//! Batch 42 (ENH-09): Fill and sign as a few clear tasks, its pane and menu, and D2's
//! flow of a flat form signed, finished, saved and reopened, with app.rs's helpers.

use super::*;
use crate::signing::{Entry, Saved};
use egui::accesskit::{Node, NodeId, Role, Toggled, TreeUpdate};
use limarepdf_core::{AnnotKind, ObjectKind, SignRole, SignStyle};
use std::collections::{BTreeSet, HashMap};

/// Why Finish signing… is greyed with nothing put down.
const NOTHING_PLACED: &str = "Put a signature, the date or a mark on the page first.";
/// The pin that keeps a placing tool on after something is put down.
const PIN: &str = "Keep tool active";

fn test_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    ctx
}

/// A signature typed as `text`, in the first style.
fn typed(text: &str) -> Saved {
    Saved::Typed {
        text: text.into(),
        style: SignStyle::ALL[0],
    }
}

/// Save typed signatures `texts` in the app's kit, for this session only. Each goes first
/// in the list, as one made with Create does: the last is "Signature 1".
fn save_signatures(app: &mut App, texts: &[&str]) {
    for text in texts {
        let e = Entry {
            saved: typed(text),
            keep: false,
        };
        assert_eq!(app.settings.sign.add(SignRole::Signature, e), None);
    }
}

/// Save initials typed as `text` in the app's kit, for this session only.
fn save_initials(app: &mut App, text: &str) {
    let e = Entry {
        saved: typed(text),
        keep: false,
    };
    assert_eq!(app.settings.sign.add(SignRole::Initials, e), None);
}

/// Click the control named `name`, as people click: the clock moves on, so two clicks are
/// a double-click only when meant.
fn click_on(app: &mut App, ty: &Typist, name: &str) {
    let tree = ty.pause(app);
    let p = middle(&tree, name);
    ty.frame(app, vec![egui::Event::PointerMoved(p)]);
    ty.frame(app, vec![press(p, true)]);
    ty.frame(app, vec![press(p, false)]);
    ty.pause(app);
}

/// Click at page point `at` (x, y down) on the first page, as people click.
fn click_page_at(app: &mut App, ty: &Typist, at: (f32, f32)) {
    click_page_as(app, ty, at, egui::Modifiers::NONE);
}

/// [`click_page_at`] with `held` keys down (Ctrl, say), let go after.
fn click_page_as(app: &mut App, ty: &Typist, at: (f32, f32), held: egui::Modifiers) {
    let (min, k) = ty.page(app);
    let p = min + egui::vec2(at.0, at.1) * k;
    let with = |e: egui::Event| vec![egui::Event::ModifiersChanged(held), e];
    ty.frame(app, with(egui::Event::PointerMoved(p)));
    ty.frame(app, with(press(p, true)));
    ty.frame(app, with(press(p, false)));
    let none = egui::Event::ModifiersChanged(egui::Modifiers::NONE);
    ty.frame(app, vec![none]);
    ty.pause(app);
}

/// Drag from page point `from` to `to` on the first page, in steps, as people drag.
fn drag_page_to(app: &mut App, ty: &Typist, from: (f32, f32), to: (f32, f32)) {
    let (min, k) = ty.page(app);
    let at = |(x, y): (f32, f32)| min + egui::vec2(x, y) * k;
    let (a, b) = (at(from), at(to));
    ty.frame(app, vec![egui::Event::PointerMoved(a)]);
    ty.frame(app, vec![press(a, true)]);
    for i in 1..=6 {
        let p = a + (b - a) * (i as f32 / 6.0);
        ty.frame(app, vec![egui::Event::PointerMoved(p)]);
    }
    ty.frame(app, vec![press(b, false)]);
    ty.pause(app);
}

/// Whether any text in the window says `s`: a label, or a tooltip shown.
fn says(tree: &TreeUpdate, s: &str) -> bool {
    tree.nodes
        .iter()
        .any(|(_, n)| n.value().is_some_and(|v| v.contains(s)))
}

/// Whether the biggest control named `name` is greyed.
fn greyed(tree: &TreeUpdate, name: &str) -> bool {
    let named = tree.nodes.iter().filter(|(_, n)| n.label() == Some(name));
    let area = |n: &Node| n.bounds().map_or(0.0, |b| b.width() * b.height());
    let n = named.max_by(|(_, a), (_, b)| area(a).total_cmp(&area(b)));
    n.unwrap_or_else(|| panic!("no {name}")).1.is_disabled()
}

/// A node's place on screen.
fn place(n: &Node) -> Option<egui::Rect> {
    let b = n.bounds()?;
    let min = egui::pos2(b.x0 as f32, b.y0 as f32);
    let max = egui::pos2(b.x1 as f32, b.y1 as f32);
    Some(egui::Rect::from_min_max(min, max))
}

/// The nodes inside the nodes `is_root` picks, each root's in turn.
fn inside(tree: &TreeUpdate, is_root: impl Fn(&Node) -> bool) -> Vec<Vec<&Node>> {
    let nodes: HashMap<NodeId, &Node> = tree.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let roots = tree.nodes.iter().filter(|(_, n)| is_root(n));
    roots
        .map(|(root, _)| {
            let (mut out, mut next) = (Vec::new(), vec![*root]);
            while let Some(id) = next.pop() {
                if let Some(n) = nodes.get(&id) {
                    out.push(*n);
                    next.extend(n.children().iter().copied());
                }
            }
            out
        })
        .collect()
}

/// The place of the first node in the inspector (the region holding the most, as
/// [`inspector_says`] reads it) that `is` picks.
fn in_inspector(tree: &TreeUpdate, is: impl Fn(&Node) -> bool) -> Option<egui::Rect> {
    let regions = inside(tree, |n| {
        n.role() == Role::Complementary && n.label() == Some("Inspector")
    });
    let region = regions.into_iter().max_by_key(Vec::len)?;
    region.into_iter().filter(|&n| is(n)).find_map(place)
}

/// The named items of the menus open, by name, with their place and whether greyed.
fn menu_items(tree: &TreeUpdate) -> Vec<(String, egui::Rect, bool)> {
    let menus = inside(tree, |n| n.role() == Role::Menu);
    let items = menus.into_iter().flatten();
    items
        .filter(|n| n.role() != Role::Menu)
        .filter_map(|n| Some((n.label()?.to_string(), place(n)?, n.is_disabled())))
        .collect()
}

/// The menu item whose name starts with `name` (egui adds the shortcut to a menu item's).
fn menu_item(tree: &TreeUpdate, name: &str) -> Option<(egui::Rect, bool)> {
    let items = menu_items(tree);
    let item = items.into_iter().find(|(n, ..)| n.starts_with(name))?;
    Some((item.1, item.2))
}

/// The window drawn as [`Typist::frame`] draws it, and the characters drawn in it that
/// have no glyph in their font (egui draws a box for them), each with the text it is in.
fn missing_glyphs(app: &mut App, ty: &Typist) -> BTreeSet<String> {
    ty.time.set(ty.time.get() + 0.05);
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, TEST_HEIGHT),
        )),
        time: Some(ty.time.get()),
        ..Default::default()
    };
    let (ctx, mut frame) = (ty.ctx, eframe::Frame::_new_kittest());
    let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut frame));
    out.textures_delta.clear();
    // Each section of each text drawn: its font and its text.
    fn walk(s: &egui::Shape, out: &mut Vec<(egui::FontId, String)>) {
        match s {
            egui::Shape::Text(t) => {
                let job = &t.galley.job;
                for section in &job.sections {
                    let r = &section.byte_range;
                    let text = job.text[r.start.0..r.end.0].to_string();
                    out.push((section.format.font_id.clone(), text));
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
    let mut missing = BTreeSet::new();
    for (font, text) in texts {
        for c in text.chars().filter(|c| !c.is_whitespace()) {
            if !ctx.fonts_mut(|f| f.has_glyph(&font, c)) {
                missing.insert(format!("{c} (U+{:04X}) in {text:?}", c as u32));
            }
        }
    }
    missing
}

/// Fill and sign's pane offers one way to make a signature and one to make initials, with
/// none saved and with some: one "Create signature…" button and one "Create initials…",
/// and no "New…" button.
#[test]
fn fill_and_sign_offers_one_way_to_create_each() {
    let ctx = test_ctx();
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::Tool(Tool::Sign));
    let check = |app: &mut App, what: &str| {
        let names = focusable(app, &ctx, 1400.0);
        let count = |name: &str| names.iter().filter(|n| n.as_deref() == Some(name)).count();
        for name in ["Create signature…", "Create initials…"] {
            assert_eq!(count(name), 1, "{what}: how many {name}");
        }
        assert_eq!(count("New…"), 0, "{what}: a New… button");
    };
    check(&mut app, "none saved");
    save_signatures(&mut app, &["Sam Lee", "S. Lee"]);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(node(&tree, "Signature 2").is_some(), "none shown");
    check(&mut app, "two saved");
}

/// Each saved signature has a remove button beside it, shown, not only a right-click menu;
/// clicking "Remove signature 2" takes that one out of the list, and the other stays. The
/// initials have theirs too.
#[test]
fn a_saved_signature_has_a_visible_remove_button() {
    let ctx = test_ctx();
    let mut app = app_with(&ctx, 1);
    // "S. Lee" is Signature 1, "Sam Lee" Signature 2.
    save_signatures(&mut app, &["Sam Lee", "S. Lee"]);
    save_initials(&mut app, "SL");
    app.act(&ctx, Act::Tool(Tool::Sign));
    let tree = settled_at(&mut app, &ctx, 1400.0);
    for i in [1, 2] {
        let name = format!("Remove signature {i}");
        let (r, _) = node(&tree, &name).unwrap_or_else(|| panic!("no {name}"));
        let shown = r.width() > 4.0 && r.height() > 4.0;
        assert!(shown, "{name} isn't shown: {r:?}");
        let (sig, _) = node(&tree, &format!("Signature {i}")).expect("the signature");
        let beside = r.top() < sig.bottom() && r.bottom() > sig.top();
        assert!(beside, "{name} at {r:?}, not beside {sig:?}");
    }
    click_named(&mut app, &ctx, "Remove signature 2");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let left: Vec<Saved> = (app.settings.sign.signatures.iter())
        .map(|e| e.saved.clone())
        .collect();
    assert_eq!(left, [typed("S. Lee")], "not the second taken out");
    assert!(node(&tree, "Signature 1").is_some(), "both went");
    for gone in ["Signature 2", "Remove signature 2"] {
        assert!(node(&tree, gone).is_none(), "{gone} still shown");
    }
    assert_eq!(app.settings.sign.initials.len(), 1, "the initials went");
    click_named(&mut app, &ctx, "Remove initials 1");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(app.settings.sign.initials.is_empty(), "the initials stayed");
    assert!(node(&tree, "Initials 1").is_none(), "still shown");
    assert_eq!(app.settings.sign.signatures.len(), 1, "a signature went");
}

/// Every character drawn in the window with Fill and sign's pane (and its name field's
/// tooltip), and with its choices menu open, has a glyph in its font: no missing-glyph
/// boxes where the tick and cross were (Inter has neither ✔ nor ✖).
#[test]
fn no_character_in_fill_and_sign_lacks_a_glyph() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = app_with(&ctx, 1);
    save_signatures(&mut app, &["Sam Lee"]);
    app.act(&ctx, Act::Tool(Tool::Sign));
    let tree = ty.pause(&mut app);
    assert!(node(&tree, "Tick").is_some(), "no pane");
    let mut pane = missing_glyphs(&mut app, &ty);
    // With the tooltip of the name's field shown too.
    let (r, _) = node(&tree, "Your name").expect("the name's field");
    ty.frame(&mut app, vec![egui::Event::PointerMoved(r.center())]);
    let tree = ty.pause(&mut app);
    assert!(says(&tree, "Also signs your comments"), "no tooltip");
    pane.extend(missing_glyphs(&mut app, &ty));
    ty.frame(&mut app, vec![egui::Event::PointerGone]);
    app.band = Band::Forms;
    click_on(&mut app, &ty, "Fill and sign choices");
    assert!(egui::Popup::is_any_open(&ctx), "the menu isn't open");
    let tree = ty.pause(&mut app);
    assert!(menu_item(&tree, "Tick").is_some(), "no Tick in the menu");
    let menu = missing_glyphs(&mut app, &ty);
    assert!(
        pane.is_empty() && menu.is_empty(),
        "no glyph for, with the pane: {pane:?}; with the menu open: {menu:?}"
    );
}

/// The "Keep tool active" pin is in the inspector's heading row, beside its collapse
/// button, not on a row of its own below it; clicking it still turns keeping the tool on
/// and off.
#[test]
fn the_keep_tool_pin_is_in_the_inspector_heading_row() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::Tool(Tool::Sign));
    let tree = ty.pause(&mut app);
    // The inspector's own heading: the band and the status bar say "Fill and sign" too.
    let heading = in_inspector(&tree, |n| n.value() == Some("Fill and sign"));
    let heading = heading.expect("no heading in the inspector");
    let pin = in_inspector(&tree, |n| n.label() == Some(PIN));
    let pin = pin.expect("no pin in the inspector");
    let collapse = in_inspector(&tree, |n| n.label() == Some("Collapse the inspector"));
    let collapse = collapse.expect("no collapse button");
    let (y, at) = (heading.center().y, pin.center().y);
    assert!(
        (at - y).abs() <= 6.0,
        "the pin at {pin:?} is not in the heading's row at {heading:?}"
    );
    let c = collapse.center().y;
    assert!((at - c).abs() <= 6.0, "not beside {collapse:?}");
    assert!(pin.left() > heading.right(), "the pin before the heading");
    let toggled = |tree: &TreeUpdate| {
        let pin = tree.nodes.iter().find(|(_, n)| n.label() == Some(PIN));
        pin.and_then(|(_, n)| n.toggled())
    };
    assert_eq!(toggled(&tree), Some(Toggled::False));
    click_on(&mut app, &ty, PIN);
    let tree = ty.pause(&mut app);
    assert_eq!(toggled(&tree), Some(Toggled::True), "not turned on");
    assert!(app.settings.keep_tools.contains(&Tool::Sign), "not kept");
    click_on(&mut app, &ty, PIN);
    let tree = ty.pause(&mut app);
    assert_eq!(toggled(&tree), Some(Toggled::False), "not turned off");
    assert!(!app.settings.keep_tools.contains(&Tool::Sign), "still kept");
}

/// The pin is in the heading row of the other tools that put things down too (Stamp
/// here), and gone while a comment is selected with such a tool: the inspector shows the
/// comment then, which the pin has nothing to do with.
#[test]
fn the_pin_shows_for_placing_tools_but_not_for_a_comment() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = app_with(&ctx, 1);
    let (at, yellow) = ((300.0, 200.0), [1.0, 1.0, 0.0]);
    let note = |p: &mut Pdf| p.add_note(0, at, yellow, "Walrus", "").map(drop);
    assert!(app.tabs[0].edit(&ctx, note));
    app.act(&ctx, Act::Tool(Tool::Stamp));
    let tree = ty.pause(&mut app);
    let heading = in_inspector(&tree, |n| n.value() == Some("New stamp"));
    let heading = heading.expect("no heading in the inspector");
    let pin = in_inspector(&tree, |n| n.label() == Some(PIN));
    let pin = pin.expect("no pin for the Stamp tool");
    let (y, at) = (heading.center().y, pin.center().y);
    assert!((at - y).abs() <= 6.0, "pin {pin:?}, heading {heading:?}");
    // The note selected with Ctrl+click, the Stamp tool still in hand.
    let b = app.tabs[0].pdf.annotations()[0].bbox;
    click_page_as(&mut app, &ty, (b[0] + 4.0, b[1] + 4.0), CTRL);
    assert_eq!(app.tabs[0].ed.tool, Tool::Stamp);
    let picked = app.tabs[0].ed.selection.annot();
    assert!(picked.is_some(), "no comment selected");
    let tree = ty.pause(&mut app);
    assert!(node(&tree, PIN).is_none(), "a pin with a comment selected");
}

/// Saved signatures and initials, each with its remove button, fit the inspector: it keeps
/// the width Fill and sign's pane has with none saved, frame after frame, at its usual
/// width and when made narrow (a width remembered from an earlier run). A row sized from
/// the room left but wider than it widened the panel a little every frame, until the
/// picture reached its largest size.
#[test]
fn saved_signatures_leave_the_inspector_its_width() {
    let mut widened = Vec::new();
    for narrow in [None, Some(150.0)] {
        let ctx = test_ctx();
        let ty = Typist::new(&ctx);
        let mut settings = Settings::from_file(None);
        if let Some(w) = narrow {
            settings.panel_widths.insert("inspector".into(), w);
        }
        let mut app = App::with(settings, Recent::from_file(None));
        let mut pdf = Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        assert!(app.add_tab(&ctx, pdf, None, None, false));
        let width = |tree: &TreeUpdate| {
            let (edge, _) = node(tree, "Inspector width").expect("the edge");
            1400.0 - edge.center().x
        };
        app.act(&ctx, Act::Tool(Tool::Sign));
        let empty = width(&ty.pause(&mut app));
        // Two signatures (the longer one's picture as wide as the room lets it be) and
        // initials, shown in the pane already open.
        save_signatures(&mut app, &["Sam Lee", "S. Lee"]);
        save_initials(&mut app, "SL");
        let mut widths = Vec::new();
        for _ in 0..60 {
            widths.push(width(&ty.frame(&mut app, vec![])));
        }
        let tree = ty.frame(&mut app, vec![]);
        let shown = node(&tree, "Remove initials 1").is_some();
        assert!(shown, "{narrow:?}: no pane");
        widths.dedup();
        if widths != [empty] {
            let was = format!("{narrow:?}: {empty} with none saved");
            widened.push(format!("{was}, then {widths:?}"));
        }
    }
    assert_eq!(widened, [""; 0], "the inspector widened");
}

/// The Forms and sign band's Fill and sign menu, with a signature saved, ends with Finish
/// signing…: greyed while nothing is put down, saying why on hover, and on once something
/// is.
#[test]
fn finish_signing_is_in_the_fill_and_sign_menu() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = app_with(&ctx, 1);
    save_signatures(&mut app, &["Sam Lee"]);
    app.band = Band::Forms;
    click_on(&mut app, &ty, "Fill and sign choices");
    let tree = ty.pause(&mut app);
    let (r, off) = menu_item(&tree, "Finish signing…").expect("no Finish signing… in the menu");
    assert!(off, "on with nothing put down");
    let items = menu_items(&tree);
    let tops = items.iter().map(|(_, r, _)| r.top());
    let last = tops.fold(f32::MIN, f32::max);
    assert!(r.top() >= last, "not at the menu's end: {items:?}");
    ty.frame(&mut app, vec![egui::Event::PointerMoved(r.center())]);
    let tree = ty.pause(&mut app);
    assert!(says(&tree, NOTHING_PLACED), "no reason on hover");
    ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
    ty.pause(&mut app);
    assert!(!egui::Popup::is_any_open(&ctx), "the menu didn't close");

    // The saved signature put down with the tool.
    app.act(&ctx, Act::Tool(Tool::Sign));
    click_page_at(&mut app, &ty, (200.0, 150.0));
    assert_eq!(app.tabs[0].pdf.sign_items(), 1, "nothing put down");
    click_on(&mut app, &ty, "Fill and sign choices");
    let tree = ty.pause(&mut app);
    let (_, off) = menu_item(&tree, "Finish signing…").expect("no Finish signing… in the menu");
    assert!(!off, "greyed with a signature put down");
}

/// The pane's Finish signing… is greyed while nothing is put down and says why on hover,
/// as the command does wherever it is.
#[test]
fn a_greyed_finish_signing_says_why() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::Tool(Tool::Sign));
    let tree = ty.pause(&mut app);
    let finish = "Finish signing…";
    let (r, _) = node(&tree, finish).expect("the pane's Finish signing…");
    assert!(greyed(&tree, finish), "on with nothing put down");
    ty.frame(&mut app, vec![egui::Event::PointerMoved(r.center())]);
    let tree = ty.pause(&mut app);
    assert!(says(&tree, NOTHING_PLACED), "no reason on hover");
    // The command, in the menus and the command search.
    let a = actions::get(Act::FinishSigning);
    let c = app.state(&ctx);
    assert!(!(a.enabled)(&c), "the command is on");
    assert_eq!(actions::why_not(&c, a), NOTHING_PLACED);
}

/// The Finish signing question says the items were put down with Fill and sign, the name
/// people see, not the Sign tool.
#[test]
fn the_finish_signing_question_names_fill_and_sign() {
    let ctx = test_ctx();
    let mut app = app_with(&ctx, 1);
    let ink = limarepdf_core::InkColor::Black;
    for y in [100.0, 140.0] {
        let put = |p: &mut Pdf| p.add_sign_text(0, (200.0, y), "Sam Lee", ink, "").map(drop);
        assert!(app.tabs[0].edit(&ctx, put));
    }
    assert_eq!(app.tabs[0].pdf.sign_items(), 2);
    app.act(&ctx, Act::FinishSigning);
    let tree = settle(&mut app, &ctx);
    assert!(shows(&tree, "Finish signing?"), "no question");
    let texts: Vec<&str> = tree.nodes.iter().filter_map(|(_, n)| n.value()).collect();
    let has = |s: &str| texts.iter().any(|t| t.contains(s));
    let question: Vec<_> = texts.iter().filter(|t| t.contains("put down")).collect();
    assert!(has("put down with Fill and sign"), "{question:?}");
    assert!(!has("Sign tool"), "names the Sign tool: {question:?}");
}

/// A page of a flat form: lines to fill in, "Name", "Date" and "I agree", and no form
/// fields, as a printed form scanned or saved from another app is. As if just opened.
fn flat_form(ctx: &egui::Context) -> App {
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    for (y, line) in [
        (100.0, "Name  ______________________"),
        (140.0, "Date  ______________________"),
        (180.0, "I agree to the terms  ____"),
    ] {
        let font = FontChoice::default();
        pdf.add_text(0, (72.0, y), line, font, 14.0, [0.0; 3])
            .unwrap();
    }
    assert!(app.add_tab(ctx, pdf, None, None, false));
    app
}

/// D2 (build plan 3.2): a flat form signed as people do. With Fill and sign kept on, the
/// saved signature, the date and a tick are put down by clicks; the tick is moved with
/// the read tool; Finish signing from the pane, the question answered, draws all three
/// into the page; Undo brings them back as items and Redo draws them in again; saved as a
/// new file and opened again, they are part of the page, the date in the kit's style.
#[test]
fn a_flat_form_is_signed_finished_saved_and_reopened() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = flat_form(&ctx);
    save_signatures(&mut app, &["Sam Lee"]);
    app.act(&ctx, Act::Tool(Tool::Sign));
    click_on(&mut app, &ty, PIN);
    assert!(app.settings.keep_tools.contains(&Tool::Sign), "not kept on");
    // The signature on the Name line, then the date and a tick.
    click_page_at(&mut app, &ty, (320.0, 95.0));
    click_on(&mut app, &ty, "Date");
    click_page_at(&mut app, &ty, (320.0, 135.0));
    click_on(&mut app, &ty, "Tick");
    click_page_at(&mut app, &ty, (300.0, 175.0));
    let labels = |app: &App| -> Vec<String> {
        let annots = app.tabs[0].pdf.annotations();
        annots.iter().map(|a| a.label()).collect()
    };
    assert_eq!(app.tabs[0].pdf.sign_items(), 3, "{:?}", labels(&app));
    assert_eq!(app.tabs[0].ed.tool, Tool::Sign, "the tool wasn't kept");

    // The tick moved with the read tool: clicked, then dragged 40 points right.
    app.act(&ctx, Act::Tool(Tool::Read));
    let tick = |app: &App| {
        let annots = app.tabs[0].pdf.annotations();
        let tick = annots.into_iter().find(|a| a.label() == "Tick");
        let tick = tick.unwrap_or_else(|| panic!("no tick: {:?}", labels(app)));
        tick.bbox
    };
    let was = tick(&app);
    let mid = ((was[0] + was[2]) / 2.0, (was[1] + was[3]) / 2.0);
    click_page_at(&mut app, &ty, mid);
    drag_page_to(&mut app, &ty, mid, (mid.0 + 40.0, mid.1));
    let now = tick(&app);
    let (dx, dy) = (now[0] - was[0], now[1] - was[1]);
    let wider = (now[2] - now[0]) - (was[2] - was[0]);
    let moved = (dx - 40.0).abs() < 0.6 && dy.abs() < 0.6 && wider.abs() < 0.1;
    assert!(moved, "{was:?} to {now:?}");
    assert_eq!(app.tabs[0].pdf.sign_items(), 3, "{:?}", labels(&app));

    // Finished from the pane, the question answered in the window.
    let forms = |app: &App| {
        let objs = app.tabs[0].pdf.page_objects(0).unwrap();
        objs.iter().filter(|o| o.kind == ObjectKind::Form).count()
    };
    assert_eq!(forms(&app), 0);
    app.act(&ctx, Act::Tool(Tool::Sign));
    click_on(&mut app, &ty, "Finish signing…");
    click_on(&mut app, &ty, "Finish signing");
    assert_eq!(app.tabs[0].pdf.sign_items(), 0, "{:?}", labels(&app));
    assert_eq!(forms(&app), 3, "not drawn into the page");
    app.act(&ctx, Act::Undo);
    assert_eq!(app.tabs[0].pdf.sign_items(), 3, "Undo");
    assert_eq!(forms(&app), 0, "Undo");
    app.act(&ctx, Act::Redo);
    assert_eq!(app.tabs[0].pdf.sign_items(), 0, "Redo");
    assert_eq!(forms(&app), 3, "Redo");

    let dir = test_dir("flat-form-signed");
    let path = dir.join("signed.pdf");
    SAVE_AS.with(|s| s.set(Some(path.clone())));
    app.act(&ctx, Act::SaveAs);
    saves_done(&mut app, &ctx);
    let back = Pdf::open(&path, None).expect("the saved file");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(back.sign_items(), 0);
    let kinds: Vec<AnnotKind> = back.annotations().into_iter().map(|a| a.kind).collect();
    assert!(kinds.is_empty(), "annotations left: {kinds:?}");
    let text = back.page_text(0);
    let date = app.settings.sign.date.today();
    assert!(text.contains(&date), "no {date} in {text:?}");
    assert!(text.contains("Name"), "{text:?}");
}
