//! ENH-16: bookmarks moved in the Bookmarks view, as Word's outline and Navigation pane
//! move headings: dragged before, after or into another, or from the keyboard, the list
//! one stop for Tab with a tree's keys.

use super::*;
use limarepdf_core::BookmarkPlace;

const ALT_SHIFT: egui::Modifiers = egui::Modifiers::ALT.plus(egui::Modifiers::SHIFT);
const NONE: egui::Modifiers = egui::Modifiers::NONE;

/// One tab of four pages with Walrus (Calves and Tusks inside), Otter, and Seal (Pups
/// inside), going to pages 1, 2, 3, 2, 3 and 4; the Bookmarks view shown.
fn bookmarks_tab(ctx: &egui::Context) -> App {
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let mut pdf = Pdf::new();
    for i in 0..4 {
        pdf.insert_blank(i, 400.0, 600.0).unwrap();
    }
    use BookmarkPlace::{End, Under};
    for (title, page, place) in [
        ("Walrus", 0, End),
        ("Calves", 1, Under(0)),
        ("Tusks", 2, Under(0)),
        ("Otter", 1, End),
        ("Seal", 2, End),
        ("Pups", 3, Under(4)),
    ] {
        pdf.add_bookmark(title, page, place).unwrap();
    }
    assert!(app.add_tab(ctx, pdf, None, None, false));
    app.act(ctx, Act::Bookmarks);
    app
}

fn test_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    ctx
}

/// The outline on one line: each title after a dot for each level it is in.
fn tree_of(app: &App) -> String {
    let items = app.tabs[0].pdf.outline();
    let shown = items
        .iter()
        .map(|i| format!("{}{}", ".".repeat(i.depth), i.title));
    shown.collect::<Vec<_>>().join(" ")
}

/// The tab's undo steps.
fn steps(app: &App) -> usize {
    app.tabs[0].history().1
}

/// The title of the bookmark selected in the list.
fn selected(app: &App) -> Option<String> {
    let t = &app.tabs[0];
    let id = t.outline_selected()?;
    let mut nodes = t.pdf.outline_nodes().into_iter();
    nodes.find(|(n, _)| *n == id).map(|(_, i)| i.title)
}

/// Click the bookmark or control called `name`, as people click.
fn click_on(app: &mut App, ty: &Typist, name: &str) -> egui::accesskit::TreeUpdate {
    let tree = ty.pause(app);
    let p = middle(&tree, name);
    ty.frame(app, vec![egui::Event::PointerMoved(p)]);
    ty.frame(app, vec![press(p, true)]);
    ty.frame(app, vec![press(p, false)]);
    ty.pause(app)
}

/// Press `key` with `held` keys down, as a keyboard sends them, and let the window settle.
fn key(app: &mut App, ty: &Typist, key: Key, held: egui::Modifiers) -> egui::accesskit::TreeUpdate {
    let down = egui::Event::ModifiersChanged(held);
    ty.frame(app, vec![down, key_with(key, held)]);
    ty.frame(app, vec![egui::Event::ModifiersChanged(NONE)]);
    ty.pause(app)
}

/// Drag from `from` to `to` with the mouse, in small steps, and let go.
fn drag_to(app: &mut App, ty: &Typist, from: egui::Pos2, to: egui::Pos2) {
    ty.frame(app, vec![egui::Event::PointerMoved(from)]);
    ty.frame(app, vec![press(from, true)]);
    for i in 1..=8 {
        let p = from.lerp(to, i as f32 / 8.0);
        ty.frame(app, vec![egui::Event::PointerMoved(p)]);
    }
    ty.frame(app, vec![egui::Event::PointerMoved(to)]);
    ty.frame(app, vec![press(to, false)]);
    ty.pause(app);
}

/// The row of the bookmark called `name` on screen.
fn row(tree: &egui::accesskit::TreeUpdate, name: &str) -> egui::Rect {
    node(tree, name).unwrap_or_else(|| panic!("no {name}")).0
}

/// The list is one stop for Tab; its rows take clicks, not the keyboard. Up and Down
/// select the bookmark before or after as they are shown and go to its page, as a click
/// does; Home and End the first and last; Right opens a closed bookmark and then goes into
/// it, Left closes an open one and then goes to the one it is in; Esc gives the keyboard
/// back.
#[test]
fn the_list_is_one_tab_stop_whose_keys_walk_the_tree() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = bookmarks_tab(&ctx);
    let tree = ty.pause(&mut app);
    assert!(node(&tree, "Bookmark list").is_some(), "no list");
    let focusable = |name: &str| {
        let found = tree.nodes.iter().find(|(_, n)| n.label() == Some(name));
        found.is_some_and(|(_, n)| n.supports_action(egui::accesskit::Action::Focus))
    };
    assert!(focusable("Bookmark list"), "the list is no stop for Tab");
    for name in ["Walrus", "Otter", "Pups", "Collapse Walrus"] {
        assert!(!focusable(name), "{name} is a stop for Tab");
    }
    click_on(&mut app, &ty, "Otter");
    assert_eq!(selected(&app).as_deref(), Some("Otter"));
    assert_eq!(app.tabs[0].current, 1);
    let tree = ty.pause(&mut app);
    assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Bookmark list"));
    let walk = [
        (Key::ArrowDown, "Seal", 2),
        (Key::ArrowDown, "Pups", 3),
        (Key::ArrowDown, "Pups", 3),
        (Key::Home, "Walrus", 0),
        (Key::ArrowDown, "Calves", 1),
        (Key::End, "Pups", 3),
        (Key::ArrowUp, "Seal", 2),
        (Key::ArrowUp, "Otter", 1),
        (Key::ArrowUp, "Tusks", 2),
    ];
    for (k, title, page) in walk {
        key(&mut app, &ty, k, NONE);
        assert_eq!(selected(&app).as_deref(), Some(title), "{k:?}");
        assert_eq!(app.tabs[0].current, page, "{k:?} to {title}");
    }
    key(&mut app, &ty, Key::ArrowLeft, NONE);
    assert_eq!(selected(&app).as_deref(), Some("Walrus"));
    let tree = key(&mut app, &ty, Key::ArrowLeft, NONE);
    assert!(node(&tree, "Calves").is_none(), "Walrus didn't close");
    assert_eq!(selected(&app).as_deref(), Some("Walrus"));
    let tree = key(&mut app, &ty, Key::ArrowRight, NONE);
    assert!(node(&tree, "Calves").is_some(), "Walrus didn't open");
    key(&mut app, &ty, Key::ArrowRight, NONE);
    assert_eq!(selected(&app).as_deref(), Some("Calves"));
    // Opening and closing changes nothing in the document.
    assert!(!app.tabs[0].dirty && steps(&app) == 0);
    let tree = key(&mut app, &ty, Key::Escape, NONE);
    assert_ne!(focused_name(&ctx, &tree).as_deref(), Some("Bookmark list"));
    assert_eq!(selected(&app).as_deref(), Some("Calves"), "Esc let it go");
}

/// Alt+Shift with Up or Down moves the selected bookmark among those at its level, with
/// Left out of the one it is in and with Right into the one above, as Word's outline keys
/// do; each move one undo step, the bookmark staying selected, through Undo too. Where it
/// can't go, it says why and nothing changes.
#[test]
fn alt_shift_arrows_move_promote_and_demote_the_selected_bookmark() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = bookmarks_tab(&ctx);
    click_on(&mut app, &ty, "Otter");
    key(&mut app, &ty, Key::ArrowUp, ALT_SHIFT);
    assert_eq!(tree_of(&app), "Otter Walrus .Calves .Tusks Seal .Pups");
    assert_eq!(selected(&app).as_deref(), Some("Otter"));
    let nothing_above = "No bookmark above it at its level to go into";
    for (k, says) in [
        (Key::ArrowUp, "Already first at its level"),
        (Key::ArrowRight, nothing_above),
        (Key::ArrowLeft, "Already at the top level"),
    ] {
        let tree = key(&mut app, &ty, k, ALT_SHIFT);
        assert!(shows(&tree, says), "{k:?}: no {says:?}");
        assert_eq!(tree_of(&app), "Otter Walrus .Calves .Tusks Seal .Pups");
    }
    assert_eq!(steps(&app), 1);
    key(&mut app, &ty, Key::ArrowDown, ALT_SHIFT);
    assert_eq!(tree_of(&app), "Walrus .Calves .Tusks Otter Seal .Pups");
    key(&mut app, &ty, Key::ArrowRight, ALT_SHIFT);
    assert_eq!(tree_of(&app), "Walrus .Calves .Tusks .Otter Seal .Pups");
    key(&mut app, &ty, Key::ArrowUp, ALT_SHIFT);
    assert_eq!(tree_of(&app), "Walrus .Calves .Otter .Tusks Seal .Pups");
    let tree = key(&mut app, &ty, Key::ArrowRight, ALT_SHIFT);
    assert_eq!(tree_of(&app), "Walrus .Calves ..Otter .Tusks Seal .Pups");
    assert!(node(&tree, "Otter").is_some(), "Calves didn't open");
    key(&mut app, &ty, Key::ArrowLeft, ALT_SHIFT);
    assert_eq!(tree_of(&app), "Walrus .Calves .Otter .Tusks Seal .Pups");
    assert_eq!(steps(&app), 6);
    assert_eq!(selected(&app).as_deref(), Some("Otter"));
    // Not the pages: they stay where they were.
    let ids = app.tabs[0].pdf.page_ids();
    app.act(&ctx, Act::Undo);
    ty.pause(&mut app);
    assert_eq!(tree_of(&app), "Walrus .Calves ..Otter .Tusks Seal .Pups");
    assert_eq!(selected(&app).as_deref(), Some("Otter"), "Undo lost it");
    app.act(&ctx, Act::Redo);
    ty.pause(&mut app);
    assert_eq!(tree_of(&app), "Walrus .Calves .Otter .Tusks Seal .Pups");
    assert_eq!(app.tabs[0].pdf.page_ids(), ids);
    // What each goes to is as it was.
    let pages: Vec<_> = app.tabs[0].pdf.outline().iter().map(|i| i.page).collect();
    assert_eq!(pages, [0, 1, 1, 2, 2, 3].map(Some));
}

/// A bookmark dragged onto the top of another goes before it, onto the bottom after it,
/// onto the middle inside it, last; below the list, last of all. Dropped inside itself or
/// where it is, nothing changes.
#[test]
fn bookmarks_dragged_go_before_after_or_inside_the_one_dropped_on() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = bookmarks_tab(&ctx);
    let tree = ty.pause(&mut app);
    let (seal, walrus) = (row(&tree, "Seal"), row(&tree, "Walrus"));
    let top = |r: egui::Rect| egui::pos2(r.center().x, r.top() + r.height() * 0.15);
    let bottom = |r: egui::Rect| egui::pos2(r.center().x, r.bottom() - r.height() * 0.15);
    drag_to(&mut app, &ty, seal.center(), top(walrus));
    assert_eq!(tree_of(&app), "Seal .Pups Walrus .Calves .Tusks Otter");
    assert_eq!(selected(&app).as_deref(), Some("Seal"));

    let tree = ty.pause(&mut app);
    let (otter, calves) = (row(&tree, "Otter"), row(&tree, "Calves"));
    drag_to(&mut app, &ty, otter.center(), calves.center());
    assert_eq!(tree_of(&app), "Seal .Pups Walrus .Calves ..Otter .Tusks");

    let tree = ty.pause(&mut app);
    let (pups, tusks) = (row(&tree, "Pups"), row(&tree, "Tusks"));
    drag_to(&mut app, &ty, pups.center(), bottom(tusks));
    assert_eq!(tree_of(&app), "Seal Walrus .Calves ..Otter .Tusks .Pups");

    let tree = ty.pause(&mut app);
    let seal = row(&tree, "Seal");
    let list = row(&tree, "Bookmark list");
    let below = egui::pos2(seal.center().x, list.bottom() - 10.0);
    drag_to(&mut app, &ty, seal.center(), below);
    assert_eq!(tree_of(&app), "Walrus .Calves ..Otter .Tusks .Pups Seal");
    assert_eq!(steps(&app), 4);

    // Into itself, or onto its own place: nothing.
    let tree = ty.pause(&mut app);
    let [walrus, otter, seal] = ["Walrus", "Otter", "Seal"].map(|n| row(&tree, n));
    drag_to(&mut app, &ty, walrus.center(), otter.center());
    drag_to(&mut app, &ty, seal.center(), top(seal));
    assert_eq!(tree_of(&app), "Walrus .Calves ..Otter .Tusks .Pups Seal");
    assert_eq!(steps(&app), 4, "a move that moved nothing");
}

/// The right-click menu moves a bookmark too, its keys shown, and greys what can't be done
/// there, saying why.
#[test]
fn the_right_click_menu_moves_bookmarks() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = bookmarks_tab(&ctx);
    let tree = ty.pause(&mut app);
    let at = middle(&tree, "Walrus");
    ty.frame(&mut app, vec![egui::Event::PointerMoved(at)]);
    let right = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: Default::default(),
    };
    ty.frame(&mut app, vec![right(true)]);
    ty.frame(&mut app, vec![right(false)]);
    let tree = ty.pause(&mut app);
    let item = |name: &str| {
        let found = tree.nodes.iter().find(|(_, n)| {
            n.label().is_some_and(|l| l.starts_with(name))
                && n.role() == egui::accesskit::Role::Button
        });
        found.unwrap_or_else(|| panic!("no {name}")).1.clone()
    };
    for (name, on) in [
        ("Move up", false),
        ("Move down", true),
        ("Promote", false),
        ("Demote", false),
    ] {
        assert_eq!(!item(name).is_disabled(), on, "{name}");
    }
    let name = item("Move down").label().unwrap().to_string();
    let key = ctx.format_shortcut(&egui::KeyboardShortcut::new(ALT_SHIFT, Key::ArrowDown));
    assert!(name.contains(&key), "{name:?} without its key");
    click_on(&mut app, &ty, &name);
    assert_eq!(tree_of(&app), "Otter Walrus .Calves .Tusks Seal .Pups");
    assert_eq!(selected(&app).as_deref(), Some("Walrus"));
}

/// Bookmarks opened or closed in the list stay so after an edit; before, every edit
/// opened and closed them again as the file had them.
#[test]
fn opened_and_closed_bookmarks_stay_so_after_an_edit() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = bookmarks_tab(&ctx);
    click_on(&mut app, &ty, "Collapse Walrus");
    let tree = ty.pause(&mut app);
    assert!(node(&tree, "Calves").is_none(), "Walrus didn't close");
    assert!(app.tabs[0].edit(&ctx, |p| p.rename_bookmark(3, "Sea otter")));
    let tree = ty.pause(&mut app);
    assert!(node(&tree, "Sea otter").is_some(), "not renamed");
    assert!(node(&tree, "Calves").is_none(), "Walrus opened again");
    assert!(node(&tree, "Pups").is_some(), "Seal closed");
}

/// F2 names the selected bookmark anew and Delete deletes it with those inside it, from
/// the list's keyboard.
#[test]
fn f2_renames_and_delete_deletes_from_the_list() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = bookmarks_tab(&ctx);
    click_on(&mut app, &ty, "Seal");
    let tree = key(&mut app, &ty, Key::F2, NONE);
    let typing = tree.nodes.iter().find(|(id, _)| *id == tree.focus);
    let value = typing.and_then(|(_, n)| n.value().map(str::to_string));
    assert_eq!(value.as_deref(), Some("Seal"), "not renaming");
    key(&mut app, &ty, Key::Escape, NONE);
    assert_eq!(tree_of(&app), "Walrus .Calves .Tusks Otter Seal .Pups");
    click_on(&mut app, &ty, "Seal");
    let tree = key(&mut app, &ty, Key::Delete, NONE);
    assert_eq!(tree_of(&app), "Walrus .Calves .Tusks Otter");
    assert_eq!(toast(&app).as_deref(), Some("Deleted the bookmark"));
    assert!(node(&tree, "Pups").is_none());
}

/// ENH-18: Append PDFs… brings the files' bookmarks after the document's, shown in the
/// view, and Undo takes them away with the pages.
#[test]
fn appended_files_bring_their_bookmarks_and_undo_takes_them_away() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = bookmarks_tab(&ctx);
    let mut added = Pdf::new();
    added.insert_blank(0, 400.0, 600.0).unwrap();
    added
        .add_bookmark("Walruses abroad", 0, BookmarkPlace::End)
        .unwrap();
    let a = Appending {
        tab: app.tabs[0].id,
        to: PagesGo::End,
        files: vec![PathBuf::from("abroad.pdf")],
        read: vec![Some(added)],
    };
    app.append_read(&ctx, a);
    let all = "Walrus .Calves .Tusks Otter Seal .Pups";
    assert_eq!(tree_of(&app), format!("{all} Walruses abroad"));
    assert_eq!(app.tabs[0].pdf.outline()[6].page, Some(4));
    let tree = ty.pause(&mut app);
    assert!(node(&tree, "Walruses abroad").is_some(), "not in the view");
    app.act(&ctx, Act::Undo);
    assert_eq!(tree_of(&app), all);
    let tree = ty.pause(&mut app);
    assert!(node(&tree, "Walruses abroad").is_none(), "kept");
}
