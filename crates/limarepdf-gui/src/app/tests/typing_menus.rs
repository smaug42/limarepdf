//! The user's finds from batch 43's second stage: a right-click in text being typed on the
//! page keeps what is selected in it and opens a menu for it (Cut, Copy and Paste, Bold and
//! Italic), and Paste is on the floating bar while a line is edited and in the menu and bar of
//! words selected while reading, where it types the clipboard over them as Ctrl+V does.

use super::*;
use egui::accesskit::{Node, NodeId, Role, TreeUpdate};
use std::collections::HashMap;

fn test_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    ctx
}

/// A frame of the window, as [`Typist::frame`] draws it, and what it asked of the system.
fn asked(app: &mut App, ty: &Typist, events: Vec<egui::Event>) -> Vec<egui::ViewportCommand> {
    ty.time.set(ty.time.get() + 0.05);
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, TEST_HEIGHT),
        )),
        time: Some(ty.time.get()),
        events,
        ..Default::default()
    };
    let mut f = eframe::Frame::_new_kittest();
    let mut out = ty.ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut f));
    out.textures_delta.clear();
    let root = out.viewport_output.remove(&egui::ViewportId::ROOT);
    root.map(|v| v.commands).unwrap_or_default()
}

/// A click at `p` and the frames after it: how many times the window asked for `want`.
fn click_asking(app: &mut App, ty: &Typist, p: egui::Pos2, want: &egui::ViewportCommand) -> usize {
    let mut said = asked(app, ty, vec![egui::Event::PointerMoved(p), press(p, true)]);
    said.extend(asked(app, ty, vec![press(p, false)]));
    for _ in 0..20 {
        said.extend(asked(app, ty, vec![]));
    }
    said.iter().filter(|c| *c == want).count()
}

/// A right-click at `p`, then a pause.
fn right_click(app: &mut App, ty: &Typist, p: egui::Pos2) -> TreeUpdate {
    let button = |pressed| egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: Default::default(),
    };
    ty.frame(app, vec![egui::Event::PointerMoved(p)]);
    ty.frame(app, vec![button(true)]);
    ty.frame(app, vec![button(false)]);
    ty.pause(app)
}

/// The named items of the menus open, with their middles.
fn menu_items(tree: &TreeUpdate) -> Vec<(String, egui::Pos2)> {
    let nodes: HashMap<NodeId, &Node> = tree.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let menus = tree.nodes.iter().filter(|(_, n)| n.role() == Role::Menu);
    let mut next: Vec<NodeId> = menus.map(|(id, _)| *id).collect();
    let mut out = Vec::new();
    while let Some(id) = next.pop() {
        let Some(n) = nodes.get(&id) else { continue };
        next.extend(n.children().iter().copied());
        if let (Some(name), Some(b), false) = (n.label(), n.bounds(), n.role() == Role::Menu) {
            let mid = |a: f64, b: f64| ((a + b) / 2.0) as f32;
            out.push((
                name.to_string(),
                egui::pos2(mid(b.x0, b.x1), mid(b.y0, b.y1)),
            ));
        }
    }
    out
}

/// The middle of the menu item whose name starts with `name` (egui adds the shortcut).
fn item(items: &[(String, egui::Pos2)], name: &str) -> Option<egui::Pos2> {
    let found = items.iter().find(|(n, _)| n.starts_with(name));
    found.map(|(_, p)| *p)
}

/// Whether the floating bar, all of it (words have more buttons than [`bar_buttons`] looks
/// along), has a button whose name starts with `name`: one in its Dock button's row, before
/// it.
fn on_bar(tree: &TreeUpdate, name: &str) -> bool {
    let Some((dock, _)) = node(tree, "Dock controls on the right") else {
        return false;
    };
    tree.nodes.iter().any(|(_, n)| {
        let named = n.role() == Role::Button && n.label().is_some_and(|l| l.starts_with(name));
        let place = n
            .bounds()
            .map(|b| egui::pos2(b.x1 as f32, ((b.y0 + b.y1) / 2.0) as f32));
        named && place.is_some_and(|p| (p.y - dock.center().y).abs() < 2.0 && p.x <= dock.min.x)
    })
}

/// A right-click on the word selected in a line being edited keeps it selected and opens a
/// menu for it: Cut, Copy and Paste, then Bold and Italic. Copy there copies it, the line
/// still open with the word selected.
#[test]
fn a_right_click_in_a_line_being_edited_keeps_the_words_and_offers_their_commands() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let note = typing_doc(&mut app, &ctx);
    start_typing(&mut app, &ty, "line on the page", note);
    let id = app.tabs[0].ed.typing_id().expect("the line isn't open");
    let sel = page_text::selection(&ctx, id).expect("nothing selected");
    assert!(sel.0 != sel.1, "the word isn't selected: {sel:?}");
    let (min, k) = ty.page(&mut app);
    let tree = right_click(&mut app, &ty, min + egui::vec2(470.0, 295.0) * k);
    let now = page_text::selection(&ctx, id);
    assert_eq!(now, Some(sel), "the right-click let go of the word");
    let items = menu_items(&tree);
    for name in ["Cut", "Copy", "Paste", "Bold", "Italic"] {
        assert!(item(&items, name).is_some(), "no {name} in {items:?}");
    }
    let copy = item(&items, "Copy").unwrap();
    let copied = click_asking(&mut app, &ty, copy, &egui::ViewportCommand::RequestCopy);
    assert_eq!(copied, 1, "the word wasn't copied");
    assert_eq!(app.tabs[0].ed.typing_id(), Some(id), "the line closed");
    let now = page_text::selection(&ctx, id);
    assert_eq!(now, Some(sel), "the word isn't selected after Copy");
}

/// New text keeps what is selected in it through a right-click there too, and has the same
/// menu.
#[test]
fn a_right_click_in_new_text_keeps_the_selection_and_offers_its_commands() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let note = typing_doc(&mut app, &ctx);
    start_typing(&mut app, &ty, "new text", note);
    ty.text(&mut app, "Seal Lion");
    let id = app.tabs[0].ed.typing_id().expect("not typing");
    page_text::select(&ctx, id, (0, 4));
    let tree = ty.pause(&mut app);
    let (text, _) = node(&tree, page_text::NAME).expect("the new text");
    // On "Seal", a quarter of the way along.
    let p = egui::pos2(text.min.x + text.width() * 0.25, text.center().y);
    let tree = right_click(&mut app, &ty, p);
    let now = page_text::selection(&ctx, id);
    assert_eq!(now, Some((0, 4)), "the right-click let go of the words");
    let items = menu_items(&tree);
    for name in ["Cut", "Copy", "Paste"] {
        assert!(item(&items, name).is_some(), "no {name} in {items:?}");
    }
}

/// The floating bar offers Paste beside Cut and Copy while a line is edited.
#[test]
fn the_floating_bar_offers_paste_while_a_line_is_edited() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let note = typing_doc(&mut app, &ctx);
    start_typing(&mut app, &ty, "line on the page", note);
    let tree = ty.pause(&mut app);
    let bar: Vec<String> = bar_buttons(&tree).into_iter().map(|(n, _)| n).collect();
    for name in ["Cut", "Copy", "Paste"] {
        let on = bar.iter().any(|n| n.starts_with(name));
        assert!(on, "no {name} on the bar: {bar:?}");
    }
}

/// Words selected while reading have Paste on the floating bar and in their right-click
/// menu: it types what the clipboard holds over them, as Ctrl+V does, their line opening
/// with it in their place.
#[test]
fn paste_types_the_clipboard_over_words_selected_while_reading() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    lines_tab(&mut app, &ctx, 400.0, &paragraph(72.0, 100.0, 3, 1, false));
    let words = app.tabs[0].pdf.page_words(0).unwrap();
    let i = words.iter().position(|w| w.text == "otter").expect("otter");
    let b = words[i].bbox;
    app.tabs[0].ed.selection = Selection::word((0, i));
    let tree = ty.pause(&mut app);
    assert!(on_bar(&tree, "Cut"), "no bar");
    assert!(on_bar(&tree, "Paste"), "no Paste on the bar");
    let (min, k) = ty.page(&mut app);
    let p = min + egui::vec2((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0) * k;
    let tree = right_click(&mut app, &ty, p);
    let items = menu_items(&tree);
    let paste = item(&items, "Paste").unwrap_or_else(|| panic!("no Paste in {items:?}"));
    let asked = click_asking(&mut app, &ty, paste, &egui::ViewportCommand::RequestPaste);
    assert_eq!(asked, 1, "the clipboard wasn't asked for");
    // The system's answer: the clipboard's text.
    ty.frame(&mut app, vec![egui::Event::Paste("beaver".into())]);
    let tree = ty.pause(&mut app);
    let editing = |n: &Node| n.label().is_some_and(|l| l.starts_with("Editing text:"));
    let line = tree.nodes.iter().find(|(_, n)| editing(n));
    let typed = line.and_then(|(_, n)| n.value());
    assert_eq!(typed, Some("The beaver ate"), "not typed over the word");
}
