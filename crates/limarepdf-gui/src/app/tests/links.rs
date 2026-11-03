//! Links on the page (ENH-17): what hovering over one says, and what a click does; making
//! them over selected words or with the Link tool, and changing, moving and removing them.

use super::tools::line_at;
use super::*;
use crate::dialogs::{LinkAsk, LinkTo};
use limarepdf_core::{Link, LinkTarget};
use lopdf::{dictionary, Object, StringFormat};

/// Where the links are, in displayed points on the first page.
const WEB: [f32; 4] = [100.0, 100.0, 250.0, 120.0];
const PROGRAM: [f32; 4] = [100.0, 200.0, 250.0, 220.0];

/// A two-page tab whose first page has a web link whose address hides a right-to-left
/// override (U+202E, which would show "exe.fdp" backwards as "pdf.exe"), and a link
/// that runs a program.
fn links_tab(ctx: &egui::Context) -> App {
    let mut app = app_with(ctx, 1);
    let t = app.tabs.last_mut().unwrap();
    assert!(t.edit(ctx, |p| {
        let page = p.page_ids()[0];
        // 792 points high: y measured up from the bottom.
        let at = |b: [f32; 4]| -> Object {
            let r = [b[0], 792.0 - b[3], b[2], 792.0 - b[1]];
            r.map(Object::Real).to_vec().into()
        };
        let mut uri = vec![0xEF, 0xBB, 0xBF];
        uri.extend("https://example.com/\u{202E}exe.fdp".as_bytes());
        let uri = Object::String(uri, StringFormat::Literal);
        let web = dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => at(WEB),
        "A" => dictionary! { "S" => "URI", "URI" => uri } };
        let run = dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => at(PROGRAM),
        "A" => dictionary! { "S" => "Launch", "F" => Object::string_literal("run.exe") } };
        let refs: Vec<Object> = [web, run]
            .into_iter()
            .map(|d| p.doc.add_object(d).into())
            .collect();
        p.doc.get_dictionary_mut(page)?.set("Annots", refs);
        Ok(())
    }));
    app
}

/// The middle of box `b` on the first page, on screen.
fn middle_of(ty: &Typist, app: &mut App, b: [f32; 4]) -> egui::Pos2 {
    let (min, k) = ty.page(app);
    min + egui::vec2((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0) * k
}

/// The tooltip shown with the pointer held over `at`, if it starts with `start`.
fn tip_over(ty: &Typist, app: &mut App, at: egui::Pos2, start: &str) -> Option<String> {
    ty.frame(app, vec![egui::Event::PointerMoved(at)]);
    let tree = ty.pause(app);
    line_at(&tree, start).map(|(text, _)| text)
}

/// A web link's address is shown on hover without the characters that could make it look
/// like another (the user guide's promise; the hover showed it as the file had it), and a
/// click asks before opening it.
#[test]
fn a_web_links_address_is_shown_cleaned_and_asked_about() {
    let (ctx, _) = paragraphs::new_app();
    let ty = Typist::new(&ctx);
    let mut app = links_tab(&ctx);
    let at = middle_of(&ty, &mut app, WEB);
    let tip = tip_over(&ty, &mut app, at, "https://example.com/").expect("no address shown");
    assert!(!tip.contains('\u{202E}'), "{tip:?}");
    ty.click(&mut app, at);
    assert!(matches!(&app.dialog, Dialog::OpenLink(u) if u.starts_with("https://example.com/")));
}

/// A link that runs a program says so on hover, and a click there doesn't offer to run it.
#[test]
fn a_link_that_runs_a_program_says_so_and_does_nothing() {
    let (ctx, _) = paragraphs::new_app();
    let ty = Typist::new(&ctx);
    let mut app = links_tab(&ctx);
    let at = middle_of(&ty, &mut app, PROGRAM);
    let tip = tip_over(&ty, &mut app, at, "Runs a program");
    assert_eq!(
        tip.as_deref(),
        Some("Runs a program or opens a file: run.exe. LimarePDF doesn't run them.")
    );
    ty.click(&mut app, at);
    assert!(matches!(app.dialog, Dialog::None));
    assert!(app.tabs.iter().all(|t| t.open_link.is_none()));
}

// Making and changing links (ENH-17's stage 2).

/// A key with Ctrl (⌘ on a Mac), then the window settled.
fn ctrl(app: &mut App, ctx: &egui::Context, key: Key, shift: bool) {
    let m = if shift { CTRL_SHIFT } else { CTRL };
    window(app, ctx, 1400.0, vec![key_with(key, m)]);
    settle(app, ctx);
}

/// A key alone, then the window settled.
fn tap(app: &mut App, ctx: &egui::Context, key: Key) {
    window(app, ctx, 1400.0, vec![key_with(key, egui::Modifiers::NONE)]);
    settle(app, ctx);
}

/// The link window open, to fill in.
fn asked(app: &mut App) -> &mut LinkAsk {
    match &mut app.dialog {
        Dialog::Link(_, ask) => ask,
        _ => panic!("no link window"),
    }
}

/// The active tab's links on its first page.
fn links(app: &App) -> Vec<Link> {
    app.tabs[app.active].pdf.page_links(0).unwrap()
}

/// A link already on the first page of the active tab: page 2 over `BOX`, not underlined.
const BOX: [f32; 4] = [100.0, 100.0, 250.0, 140.0];

fn with_link(ctx: &egui::Context) -> App {
    let mut app = app_with(ctx, 1);
    let t = &mut app.tabs[0];
    let to = LinkTarget::Page(1);
    assert!(t.edit(ctx, |p| p.add_link(0, &[BOX], &to, false).map(|_| ())));
    app
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.6)
}

/// Insert > Link is a command now, no longer Planned. With nothing selected it turns the
/// Link tool on, pressed, and clicked again (or Ctrl+K, as in Word, or Esc) back to the
/// read tool; the command palette moves to Ctrl+Shift+P (ENH-17's Q1).
#[test]
fn link_turns_the_link_tool_on_and_off_and_the_palette_moves() {
    let (ctx, _) = paragraphs::new_app();
    let mut app = app_with(&ctx, 1);
    app.band = Band::Insert;
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(node(&tree, "Link — Planned").is_none(), "still Planned");
    let button = (tree.nodes.iter()).find(|(_, n)| n.label() == Some("Link"));
    let usable = button.is_some_and(|(_, n)| !n.is_disabled());
    assert!(usable, "no Link button");
    click_named(&mut app, &ctx, "Link");
    settle(&mut app, &ctx);
    assert_eq!(app.tabs[0].ed.tool, Tool::Link);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let pressed = (tree.nodes.iter())
        .filter(|(_, n)| n.label() == Some("Link"))
        .any(|(_, n)| n.toggled() == Some(egui::accesskit::Toggled::True));
    assert!(pressed, "Link not shown pressed");
    click_named(&mut app, &ctx, "Link");
    settle(&mut app, &ctx);
    assert_eq!(app.tabs[0].ed.tool, Tool::Read);
    ctrl(&mut app, &ctx, Key::K, false);
    assert_eq!(app.tabs[0].ed.tool, Tool::Link, "Ctrl+K");
    tap(&mut app, &ctx, Key::Escape);
    assert_eq!(app.tabs[0].ed.tool, Tool::Read, "Esc");
    assert!(app.palette.is_none());
    ctrl(&mut app, &ctx, Key::P, true);
    assert!(app.palette.is_some(), "Ctrl+Shift+P");
}

/// Words selected, then Ctrl+K: the Insert link window, Show as a link ticked; Insert link
/// links the words' line, underlined, to the address as written, one undo step, and lets
/// the words go, as Word does.
#[test]
fn selected_words_are_linked_underlined_in_one_step() {
    let (ctx, mut app) = paragraphs::new_app();
    text_tab(&mut app, &ctx, &["Visit the walrus pages today"]);
    settled_at(&mut app, &ctx, 1400.0);
    let words = app.tabs[0].pdf.page_words(0).unwrap();
    let at = |s: &str| words.iter().position(|w| w.text == s).expect(s);
    app.tabs[0].ed.selection = Selection::Words {
        anchor: (0, at("walrus")),
        end: (0, at("pages")),
    };
    settle(&mut app, &ctx);
    ctrl(&mut app, &ctx, Key::K, false);
    let ask = asked(&mut app);
    assert!(ask.form.underline && ask.editing.is_none());
    assert_eq!(ask.areas.len(), 1);
    ask.form.web = "example.com/walrus".into();
    click_named(&mut app, &ctx, "Insert link");
    settle(&mut app, &ctx);
    assert!(matches!(app.dialog, Dialog::None), "still open");
    let l = links(&app);
    assert_eq!(l.len(), 1, "{l:?}");
    let to = LinkTarget::Uri("https://example.com/walrus".into());
    assert_eq!((&l[0].target, l[0].underlined), (&to, true));
    let (a, b) = (words[at("walrus")].bbox, words[at("pages")].bbox);
    let over = l[0].bbox[0] <= a[0] + 0.5 && l[0].bbox[2] >= b[2] - 0.5;
    assert!(over, "{:?} not over {a:?} to {b:?}", l[0].bbox);
    let kept = app.tabs[0].ed.selection.words().is_some();
    assert!(!kept, "the words stayed selected");
    app.act(&ctx, Act::Undo);
    settle(&mut app, &ctx);
    assert!(links(&app).is_empty(), "Undo left the link");
}

/// The Link tool: a box dragged on the page opens Insert link, Show as a link not
/// ticked; a page chosen makes a link to it there, not underlined, selected for moving,
/// the tool staying on.
#[test]
fn a_box_dragged_with_the_link_tool_links_to_a_page() {
    let (ctx, _) = paragraphs::new_app();
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::Link);
    drag_page(&mut app, &ctx, (100.0, 100.0), (250.0, 140.0), false);
    let ask = asked(&mut app);
    assert!(!ask.form.underline);
    let [(page, areas)] = &ask.areas[..] else {
        panic!("{:?}", ask.areas)
    };
    assert!(*page == 0 && near(areas[0], BOX), "{areas:?}");
    ask.form.to = LinkTo::Page;
    ask.form.page = 2;
    click_named(&mut app, &ctx, "Insert link");
    settle(&mut app, &ctx);
    let l = links(&app);
    assert_eq!(l.len(), 1);
    let made = (&l[0].target, l[0].underlined);
    assert_eq!(made, (&LinkTarget::Page(1), false));
    assert!(near(l[0].bbox, BOX), "{:?}", l[0].bbox);
    let t = &app.tabs[0];
    assert_eq!(t.ed.tool, Tool::Link);
    assert_eq!(t.ed.selection.link(), Some((0, l[0].id)));
    assert_eq!(t.ed.heading(), "Link");
}

/// With the Link tool a click selects a link and a drag moves it; Delete removes it, Undo
/// brings it back; a double-click opens Edit link, whose Save link points it elsewhere and
/// keeps it selected. Esc lets the link go, then ends the tool.
#[test]
fn a_link_is_selected_moved_changed_and_removed_with_the_link_tool() {
    let (ctx, _) = paragraphs::new_app();
    let mut app = with_link(&ctx);
    app.act(&ctx, Act::Link);
    click_page(&mut app, &ctx, (175.0, 120.0), false);
    let id = links(&app)[0].id;
    assert_eq!(app.tabs[0].ed.selection.link(), Some((0, id)));
    drag_page(&mut app, &ctx, (175.0, 120.0), (225.0, 170.0), false);
    let moved = links(&app)[0].bbox;
    assert!(near(moved, [150.0, 150.0, 300.0, 190.0]), "{moved:?}");
    tap(&mut app, &ctx, Key::Delete);
    assert!(links(&app).is_empty(), "Delete left it");
    app.act(&ctx, Act::Undo);
    settle(&mut app, &ctx);
    assert_eq!(links(&app).len(), 1, "Undo didn't bring it back");
    // Away from the clicks before, which would make this a triple-click.
    for _ in 0..30 {
        settle(&mut app, &ctx);
    }
    let (min, k) = page_on_screen(&mut app, &ctx);
    let p = min + egui::vec2(225.0, 170.0) * k;
    let click = || {
        vec![
            egui::Event::PointerMoved(p),
            press(p, true),
            press(p, false),
        ]
    };
    window(&mut app, &ctx, 1400.0, click());
    window(&mut app, &ctx, 1400.0, click());
    settle(&mut app, &ctx);
    let ask = asked(&mut app);
    assert!(ask.editing.is_some());
    assert_eq!((ask.form.to, ask.form.page), (LinkTo::Page, 2));
    ask.form.to = LinkTo::Email;
    ask.form.email = "walrus@example.com".into();
    click_named(&mut app, &ctx, "Save link");
    settle(&mut app, &ctx);
    let l = links(&app);
    let to = LinkTarget::Uri("mailto:walrus@example.com".into());
    assert_eq!((l.len(), &l[0].target), (1, &to));
    assert_eq!(app.tabs[0].ed.selection.link(), Some((0, l[0].id)));
    tap(&mut app, &ctx, Key::Escape);
    assert_eq!(app.tabs[0].ed.selection.link(), None);
    assert_eq!(app.tabs[0].ed.tool, Tool::Link);
    tap(&mut app, &ctx, Key::Escape);
    assert_eq!(app.tabs[0].ed.tool, Tool::Read);
}

/// A right-click on a web link with the read tool selects it and offers Word's four: Edit
/// link…, Open link, Copy link address and Remove link; Remove link takes it away. (A link
/// to a page has no address to copy, so the menu leaves Copy link address out.)
#[test]
fn a_right_click_on_a_link_offers_words_four() {
    let (ctx, _) = paragraphs::new_app();
    let mut app = with_link(&ctx);
    let id = links(&app)[0].id;
    let to = LinkTarget::Uri("https://example.com/".into());
    let t = &mut app.tabs[0];
    assert!(t.edit(&ctx, |p| p.set_link(0, id, &to, false).map(|_| ())));
    let (min, k) = page_on_screen(&mut app, &ctx);
    let p = min + egui::vec2(175.0, 120.0) * k;
    let button = |pressed| egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: Default::default(),
    };
    let right = vec![egui::Event::PointerMoved(p), button(true), button(false)];
    window(&mut app, &ctx, 1400.0, right);
    let tree = settle(&mut app, &ctx);
    assert!(app.tabs[0].ed.selection.link().is_some(), "not selected");
    let item = |name: &str| {
        (tree.nodes.iter())
            .find(|(_, n)| n.label().map(str::trim_end) == Some(name))
            .and_then(|(_, n)| n.bounds())
    };
    let four = [
        "Edit link…",
        "Open link",
        "Copy link address",
        "Remove link",
    ];
    for name in four {
        assert!(item(name).is_some(), "no {name} in the menu");
    }
    let b = item("Remove link").unwrap();
    let at = egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
    window(&mut app, &ctx, 1400.0, vec![egui::Event::PointerMoved(at)]);
    window(&mut app, &ctx, 1400.0, vec![press(at, true)]);
    window(&mut app, &ctx, 1400.0, vec![press(at, false)]);
    settle(&mut app, &ctx);
    assert!(links(&app).is_empty(), "Remove link left it");
}

/// The link window says under the address why one isn't taken, and Insert link is greyed
/// saying so; it fits a narrow window and names every control.
#[test]
fn the_link_window_says_why_an_address_isnt_taken() {
    let (ctx, _) = paragraphs::new_app();
    let mut app = app_with(&ctx, 1);
    let mut ask = LinkAsk {
        form: crate::dialogs::LinkForm::new(2, 0, true),
        areas: vec![(0, vec![BOX])],
        editing: None,
        was: None,
    };
    ask.form.web = "javascript:alert(1)".into();
    app.dialog = Dialog::Link(app.asked(), Box::new(ask));
    for width in [1400.0, 400.0] {
        let tree = settled_at(&mut app, &ctx, width);
        let why = "LimarePDF links to web pages and email addresses only.";
        let said = (tree.nodes.iter()).any(|(_, n)| n.value() == Some(why));
        assert!(said, "{width}: no reason shown");
        assert_eq!(greyed(&tree), Some(true), "{width}: Insert link");
        assert_eq!(dlg::unclear_names(&tree), [""; 0], "{width}");
    }
    asked(&mut app).form.web = "example.com".into();
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(greyed(&tree), Some(false), "Insert link");
}

/// Whether the link window's Insert link is greyed.
fn greyed(tree: &egui::accesskit::TreeUpdate) -> Option<bool> {
    let insert = (tree.nodes.iter()).find(|(_, n)| n.label() == Some("Insert link"));
    insert.map(|(_, n)| n.is_disabled())
}

/// The inspector of a link selected with the Link tool: Show as a link changes it at once,
/// one undo step; Link to and the page box are there with it.
#[test]
fn the_inspector_changes_a_selected_link() {
    let (ctx, _) = paragraphs::new_app();
    let mut app = with_link(&ctx);
    app.act(&ctx, Act::Link);
    click_page(&mut app, &ctx, (175.0, 120.0), false);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let shown = [
        "Page in this document",
        "Page",
        "Show as a link: blue underline",
    ];
    for name in shown {
        assert!(node(&tree, name).is_some(), "no {name} in the inspector");
    }
    click_named(&mut app, &ctx, "Show as a link: blue underline");
    settle(&mut app, &ctx);
    assert!(links(&app)[0].underlined, "not underlined");
    app.act(&ctx, Act::Undo);
    settle(&mut app, &ctx);
    assert!(!links(&app)[0].underlined, "Undo left it underlined");
}
