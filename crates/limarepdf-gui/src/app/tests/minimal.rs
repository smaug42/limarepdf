//! ENH-45's stage 2: the Minimal interface, one quiet row of commands in place of the
//! commands row and the band, chosen by the switch at the row's right end, View's Minimal
//! interface, Ctrl+F1 (Option+Cmd+R on a Mac) or Settings; one setting, kept, and a switch
//! that changes nothing but what is shown. Stage 3: Tools, the inspector's Font group while
//! text is the target, and More for a row without room.

use super::paragraphs::{double_click_word, new_app, prose_tab, status_says};
use super::*;
use egui::accesskit::{Role, Toggled};

/// The Minimal interface's key, the registry's, as egui-winit sends it: Ctrl+F1, with Ctrl
/// as Linux and Windows have it; Option+Cmd+R on a Mac, with its Cmd.
fn minimal_key() -> (Key, egui::Modifiers) {
    let k = actions::get(Act::ToggleMinimal).keys[0];
    let mut m = k.modifiers;
    if m.command {
        m.ctrl = !cfg!(target_os = "macos");
        m.mac_cmd = cfg!(target_os = "macos");
    }
    (k.logical_key, m)
}

/// The Minimal interface's key pressed.
fn minimal_key_event() -> egui::Event {
    let (key, m) = minimal_key();
    key_with(key, m)
}

/// The switch's name in each interface: what it does.
const TO_MINIMAL: &str = SWITCH_TO_MINIMAL;
const TO_FULL: &str = SWITCH_TO_FULL;

/// Click the interface's switch, as it is named in the interface in use.
fn click_switch(app: &mut App, ctx: &egui::Context) {
    let name = if app.settings.minimal {
        TO_FULL
    } else {
        TO_MINIMAL
    };
    click_named(app, ctx, name);
}

/// Which interface the switch says it goes to: Some(true) for Minimal.
fn goes_to_minimal(tree: &egui::accesskit::TreeUpdate) -> Option<bool> {
    match (node(tree, TO_MINIMAL), node(tree, TO_FULL)) {
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        _ => None,
    }
}

/// An app with no document, its settings kept in `file`.
fn kept_app(file: &Path) -> App {
    App::with(
        Settings::from_file(Some(file.to_path_buf())),
        Recent::from_file(None),
    )
}

/// The whole window `size` with `events`, then settled; its AccessKit tree.
fn drawn(
    app: &mut App,
    ctx: &egui::Context,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::accesskit::TreeUpdate {
    let mut events = Some(events);
    let mut tree = None;
    for _ in 0..6 {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events: events.take().unwrap_or_default(),
            ..Default::default()
        };
        let mut frame = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut frame));
        out.textures_delta.clear();
        tree = out.platform_output.accesskit_update;
    }
    tree.expect("the window's widgets")
}

/// Whether the control `name` is ticked, as a screen reader is told.
fn ticked(tree: &egui::accesskit::TreeUpdate, name: &str) -> Option<bool> {
    (tree.nodes.iter())
        .filter(|(_, n)| n.label() == Some(name))
        .find_map(|(_, n)| n.toggled())
        .map(|t| t == Toggled::True)
}

/// A node's bounds as a rect.
fn rect(b: egui::accesskit::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(b.x0 as f32, b.y0 as f32),
        egui::pos2(b.x1 as f32, b.y1 as f32),
    )
}

/// The controls in the row with the File menu, left to right, with their names.
fn row(tree: &egui::accesskit::TreeUpdate) -> Vec<(String, egui::Rect)> {
    let (file, _) = node(tree, "File").expect("the File menu");
    let mut out: Vec<(String, egui::Rect)> = (tree.nodes.iter())
        .filter(|(_, n)| n.supports_action(egui::accesskit::Action::Click))
        .filter_map(|(_, n)| Some((n.label()?.to_string(), rect(n.bounds()?))))
        .filter(|(_, r)| (r.center().y - file.center().y).abs() < 4.0)
        .collect();
    out.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
    out
}

fn names(row: &[(String, egui::Rect)]) -> Vec<&str> {
    row.iter().map(|(n, _)| n.as_str()).collect()
}

/// Whether one of the band's names is shown.
fn band_shown(tree: &egui::accesskit::TreeUpdate) -> bool {
    (tree.nodes.iter()).any(|(_, n)| n.role() == Role::Tab && n.label() == Some("Insert"))
}

/// Full unless chosen; the switch, View's choice and Ctrl+F1 are one setting, kept in the
/// settings file both ways and restored at start before any document opens.
#[test]
fn the_interface_is_one_setting_kept_both_ways() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let file = test_dir("minimal-kept").join("settings.txt");
    let mut app = kept_app(&file);
    let tree = settle(&mut app, &ctx);
    assert_eq!(goes_to_minimal(&tree), Some(true), "Full first");
    assert!(node(&tree, "Open PDF…").is_some(), "the start screen");
    let view = |app: &App, ctx: &egui::Context| {
        let a = actions::get(Act::ToggleMinimal);
        (a.checked.unwrap())(&app.state(ctx))
    };
    assert!(!view(&app, &ctx));
    assert_eq!(
        menu::path(Act::ToggleMinimal).unwrap(),
        ["View", "Show", "Minimal interface"]
    );
    let a = actions::get(Act::ToggleMinimal);
    assert!(a.palette, "the command search");

    // The switch, on the start screen, in the tab strip above the commands.
    let (switch, _) = node(&tree, TO_MINIMAL).unwrap();
    let (file_menu, _) = node(&tree, "File").unwrap();
    assert!(switch.bottom() <= file_menu.top(), "{switch:?} not above");
    click_switch(&mut app, &ctx);
    let tree = settle(&mut app, &ctx);
    assert!(app.settings.minimal && view(&app, &ctx));
    assert_eq!(goes_to_minimal(&tree), Some(false));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("interface = minimal\n"), "{text}");

    // Restored at start, before any document opens.
    let mut again = kept_app(&file);
    let tree = settle(&mut again, &ctx);
    assert_eq!(goes_to_minimal(&tree), Some(false), "not restored");
    assert!(node(&tree, "Navigation panel").is_some(), "Minimal's row");

    // Ctrl+F1, as Word folds its ribbon away, back to Full; kept so too.
    window(&mut again, &ctx, 1400.0, vec![minimal_key_event()]);
    let tree = settle(&mut again, &ctx);
    assert!(!again.settings.minimal, "Ctrl+F1");
    assert_eq!(goes_to_minimal(&tree), Some(true));
    let mut third = kept_app(&file);
    let tree = settle(&mut third, &ctx);
    assert_eq!(goes_to_minimal(&tree), Some(true), "Full restored");
    // And View's choice, as the menus and the command search run it.
    third.act(&ctx, Act::ToggleMinimal);
    assert!(third.settings.minimal && view(&third, &ctx));
}

/// Minimal's one row: the File menu, the navigation panel, Open, Save, Undo and Redo, then
/// Find, Tools, the inspector and Print, with no band's names and no band; Full's row is as it
/// was. The switch is at the tab strip's right end in both, above Print, labelled with what
/// it does. The status bar has the page layout and Fit width beside the zoom in Minimal, and
/// no Show planned features.
#[test]
fn minimal_has_one_row_and_full_keeps_its_band() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let band = band_shown(&tree) && node(&tree, "Edit text").is_some();
    assert!(band, "Full's band");
    assert!(node(&tree, "Show planned features").is_some());
    let layout = node(&tree, "Page layout");
    assert!(layout.is_none(), "Full's status bar as approved");
    let full = row(&tree);
    assert_eq!(names(&full).last(), Some(&"Compact commands"), "{full:?}");
    let in_strip = |tree: &egui::accesskit::TreeUpdate, name: &str| {
        let (switch, _) = node(tree, name).expect(name);
        let (tab, _) = node(tree, "Untitled, unsaved").expect("the tab");
        let (print, _) = node(tree, "Print").expect("Print");
        let level = (switch.center().y - tab.center().y).abs() < 6.0;
        assert!(level && switch.right() > 1350.0, "{name}: {switch:?}");
        assert!(switch.bottom() <= print.top(), "{name}: not above Print");
        assert!(tab.right() <= switch.left(), "{name}: over the tab");
    };
    in_strip(&tree, TO_MINIMAL);

    app.act(&ctx, Act::ToggleMinimal);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(!band_shown(&tree), "the band's names");
    for gone in ["Edit text", "Paste", "Compact commands"] {
        assert!(node(&tree, gone).is_none(), "{gone} shown in Minimal");
    }
    let planned = node(&tree, "Show planned features");
    assert!(planned.is_none(), "the status bar's Show planned features");
    let minimal = row(&tree);
    let want = [
        "File",
        "Navigation panel",
        "Open",
        "Save",
        "Undo",
        "Redo",
        "Find",
        "Tools",
        "Inspector",
        "Print",
    ];
    assert_eq!(names(&minimal), want);
    for pair in minimal.windows(2) {
        assert!(pair[0].1.right() <= pair[1].1.left(), "{pair:?} overlap");
    }
    in_strip(&tree, TO_FULL);
    // The panels' buttons say whether their panel shows.
    assert_eq!(ticked(&tree, "Navigation panel"), Some(true));
    assert_eq!(ticked(&tree, "Inspector"), Some(true));
    let regions = tree.nodes.iter().filter(|(_, n)| n.role() == Role::Toolbar);
    let commands = regions.filter_map(|(_, n)| n.label()).collect::<Vec<_>>();
    assert!(commands.contains(&"Commands"), "{commands:?}");

    // The status bar: the page layout's menu and Fit width run View's commands.
    app.tabs[0].fit = false;
    click_named(&mut app, &ctx, "Fit width");
    settled_at(&mut app, &ctx, 1400.0);
    assert!(app.tabs[0].fit, "Fit width");
    click_named(&mut app, &ctx, "Page layout");
    // A menu item's name is its label, then its key's text.
    let facing = actions::get(Act::Layout(Layout::Facing)).label;
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let item = (tree.nodes.iter())
        .filter(|(_, n)| n.label().is_some_and(|l| l.starts_with(facing)))
        .find_map(|(_, n)| n.bounds())
        .expect("the page layout's menu");
    window(&mut app, &ctx, 1400.0, click_at(rect(item).center()));
    settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(app.tabs[0].layout, Layout::Facing, "the page layout");

    // Back to Full: its row and status bar as they were.
    click_switch(&mut app, &ctx);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(band_shown(&tree) && node(&tree, "Show planned features").is_some());
    let back = row(&tree);
    assert_eq!(names(&back), names(&full));
    in_strip(&tree, TO_MINIMAL);
}

/// Compact commands is the Full interface's: greyed in Minimal, saying why, on the start
/// screen too, and its setting kept for Full through switches.
#[test]
fn compact_commands_is_greyed_in_minimal_and_kept_for_full() {
    let a = actions::get(Act::ToggleCompact);
    for doc in [false, true] {
        let c = Ctx {
            doc,
            minimal: true,
            ..Ctx::default()
        };
        assert!(!(a.enabled)(&c), "enabled in Minimal");
        assert_eq!(actions::why_not(&c, a), actions::FULL_ONLY);
        let full = Ctx {
            minimal: false,
            ..c
        };
        assert!((a.enabled)(&full), "greyed in Full");
    }
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::ToggleCompact);
    app.act(&ctx, Act::ToggleMinimal);
    settled_at(&mut app, &ctx, 1400.0);
    assert!(app.settings.compact_commands, "lost in Minimal");
    app.act(&ctx, Act::ToggleMinimal);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(app.settings.compact_commands, "lost after");
    assert_eq!(ticked(&tree, "Compact commands"), Some(true));
}

/// What each tab is, as a switch must leave it: its id, whether unsaved, its history, zoom,
/// place, page, layout and selection.
fn tabs_now(app: &App) -> Vec<String> {
    (app.tabs.iter())
        .map(|t| {
            format!(
                "{} {} {:?} {} {} {:?} {} {:?} {:?}",
                t.id,
                t.unsaved(),
                t.history(),
                t.zoom,
                t.fit,
                t.place(),
                t.current,
                t.layout,
                t.ed.selection
            )
        })
        .collect()
}

/// With four tabs, one changed, the one in front zoomed and its pages selected, switching
/// both ways keeps their order, the one in front, each unsaved mark, history, zoom, place
/// and selection; Undo and Close afterwards act on the tab in front.
#[test]
fn a_switch_keeps_every_tab_as_it_was() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    named_tabs(&mut app, &ctx);
    unsaved_tab(&mut app, &ctx, Some(PathBuf::from("/nonexistent/d.pdf")));
    app.active = 3;
    let t = &mut app.tabs[3];
    t.fit = false;
    t.zoom = 1.5;
    settled_at(&mut app, &ctx, 1400.0);
    app.act(&ctx, Act::SelectAllPages);
    settled_at(&mut app, &ctx, 1400.0);
    let before = tabs_now(&app);
    assert!(app.tabs[3].unsaved() && !app.tabs[0].unsaved());
    for (i, way) in ["to Minimal", "back to Full"].into_iter().enumerate() {
        click_switch(&mut app, &ctx);
        settled_at(&mut app, &ctx, 1400.0);
        assert_eq!(app.settings.minimal, i == 0, "{way}");
        assert_eq!(app.active, 3, "{way}: the tab in front");
        assert_eq!(tabs_now(&app), before, "{way}");
    }
    app.act(&ctx, Act::ToggleMinimal);
    settled_at(&mut app, &ctx, 1400.0);
    let others: Vec<_> = app.tabs[..3].iter().map(|t| t.history()).collect();
    let undo = app.tabs[3].history().1;
    app.act(&ctx, Act::Undo);
    settled_at(&mut app, &ctx, 1400.0);
    let now = app.tabs[3].history().1;
    assert_eq!(now, undo - 1, "Undo on the tab in front");
    let now: Vec<_> = app.tabs[..3].iter().map(|t| t.history()).collect();
    assert_eq!(now, others, "Undo reached another tab");
    // Closing the tab in front: it has its change again, so it asks first.
    app.act(&ctx, Act::Redo);
    app.act(&ctx, Act::CloseTab);
    settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(app.tabs.len(), 4, "closed without asking");
    assert!(!matches!(app.dialog, Dialog::None), "no question");
}

/// The tab strip is Full's own in Minimal: one, three and four ordinary names exactly where
/// Full has them at 1000×660 and 1280×800, above the one row; many have All open documents.
#[test]
fn minimal_has_full_s_tab_strip() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let tabs = |tree: &egui::accesskit::TreeUpdate| -> Vec<(String, egui::Rect)> {
        let pdf = |n: &egui::accesskit::Node| n.label().is_some_and(|l| l.contains(".pdf"));
        let mut v: Vec<_> = (tree.nodes.iter())
            .filter(|(_, n)| n.role() == Role::Tab && pdf(n))
            .filter_map(|(_, n)| Some((n.label()?.to_string(), rect(n.bounds()?))))
            .collect();
        v.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
        v
    };
    let files = [
        "Lease agreement.pdf",
        "Harbour survey.pdf",
        "Invoice 2026-114.pdf",
        "Walrus field notes.pdf",
    ];
    for n in [1, 3, 4] {
        for size in [egui::vec2(1000.0, 660.0), egui::vec2(1280.0, 800.0)] {
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            for name in &files[..n] {
                pages_tab(&mut app, &ctx, 1);
                let path = PathBuf::from("/nonexistent").join(name);
                app.tabs.last_mut().unwrap().path = Some(path);
            }
            let full = tabs(&drawn(&mut app, &ctx, size, vec![]));
            assert_eq!(full.len(), n);
            app.act(&ctx, Act::ToggleMinimal);
            let tree = drawn(&mut app, &ctx, size, vec![]);
            let minimal = tabs(&tree);
            assert_eq!(minimal, full, "{n} tabs at {size:?}");
            let (file, _) = node(&tree, "File").unwrap();
            let above = minimal.iter().all(|(_, r)| r.bottom() <= file.top());
            assert!(above, "the tabs above");
        }
    }
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    for i in 0..14 {
        pages_tab(&mut app, &ctx, 1);
        let name = format!("/nonexistent/Quarterly report {i}.pdf");
        app.tabs.last_mut().unwrap().path = Some(PathBuf::from(name));
    }
    let size = egui::vec2(1000.0, 660.0);
    let full = tabs(&drawn(&mut app, &ctx, size, vec![]));
    app.act(&ctx, Act::ToggleMinimal);
    let tree = drawn(&mut app, &ctx, size, vec![]);
    assert!(node(&tree, "All open documents").is_some(), "many tabs");
    // The switch is as wide saying Full as Minimal: the tabs don't move.
    assert_eq!(tabs(&tree), full, "the tabs moved with the switch");
}

/// The line and the paragraph being edited stay open with the keyboard through a switch by
/// the switch and by Ctrl+F1, both ways: nothing kept, no Undo step, the typing pending.
#[test]
fn a_switch_leaves_the_typing_open_with_the_keyboard() {
    for paragraph in [false, true] {
        let (ctx, mut app) = new_app();
        prose_tab(&mut app, &ctx);
        double_click_word(&mut app, &ctx, "fishing");
        let said = match paragraph {
            false => "Editing one line",
            true => {
                app.act(&ctx, Act::EditParagraph);
                "Editing a paragraph"
            }
        };
        let tree = settled_at(&mut app, &ctx, 1400.0);
        assert!(status_says(&tree, said), "{said}: not open");
        let typed = vec![egui::Event::Text(" very".into())];
        window(&mut app, &ctx, 1400.0, typed);
        settled_at(&mut app, &ctx, 1400.0);
        let steps = app.tabs[0].history();
        for how in ["the switch", "Ctrl+F1", "the switch back", "Ctrl+F1 back"] {
            match how.starts_with("Ctrl") {
                true => {
                    window(&mut app, &ctx, 1400.0, vec![minimal_key_event()]);
                }
                false => click_switch(&mut app, &ctx),
            }
            let tree = settled_at(&mut app, &ctx, 1400.0);
            let case = format!("{said}, {how}");
            assert!(status_says(&tree, said), "{case}: ended");
            assert!(app.tabs[0].ed.pending(), "{case}: kept");
            assert_eq!(app.tabs[0].history(), steps, "{case}: a change");
            let typing = app.tabs[0].ed.typing_id();
            assert_eq!(ctx.memory(|m| m.focused()), typing, "{case}: the keyboard");
        }
        assert!(!app.settings.minimal);
        // Typing goes on in it.
        window(&mut app, &ctx, 1400.0, vec![egui::Event::Text("!".into())]);
        settled_at(&mut app, &ctx, 1400.0);
        assert!(app.tabs[0].ed.pending());
    }
}

/// A form field being filled in stays open with the keyboard through a switch.
#[test]
fn a_switch_leaves_a_field_open() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let ty = Typist::new(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let path = test_dir("minimal-field").join("form.pdf");
    form_file(&path, "");
    open_and_wait(&mut app, &ctx, &path, 1);
    ty.click_field(&mut app, "Name", 0);
    ty.text(&mut app, "Ann");
    let field = app.tabs[0].ed.form.typing_in();
    assert_eq!(field, Some(("Name".to_string(), 0)), "the field isn't open");
    let steps = app.tabs[0].history();
    let (key, m) = minimal_key();
    ty.key(&mut app, key, m);
    let tree = ty.pause(&mut app);
    assert!(app.settings.minimal);
    let open = app.tabs[0].ed.form.typing_in();
    assert_eq!(open, field, "the field was closed");
    assert_eq!(focused_text(&tree).as_deref(), Some("Ann"), "the keyboard");
    assert_eq!(app.tabs[0].history(), steps, "a change");
}

/// New text being typed stays open with the keyboard through a switch, by Ctrl+F1 and by
/// the switch, whose click takes the keyboard from it.
#[test]
fn a_switch_leaves_new_text_open() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::Tool(Tool::Text));
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let (page, _) = node(&tree, "Page 1").expect("the page");
    let p = page.min + egui::vec2(300.0, 300.0) * (page.width() / 612.0);
    for events in [vec![egui::Event::PointerMoved(p)], vec![press(p, true)]] {
        window(&mut app, &ctx, 1400.0, events);
    }
    window(&mut app, &ctx, 1400.0, vec![press(p, false)]);
    settled_at(&mut app, &ctx, 1400.0);
    let typed = vec![egui::Event::Text("Walrus".into())];
    window(&mut app, &ctx, 1400.0, typed);
    settled_at(&mut app, &ctx, 1400.0);
    let typing = app.tabs[0].ed.typing_id();
    let adding = app.tabs[0].ed.adding.is_some();
    assert!(adding && typing.is_some(), "no new text");
    let steps = app.tabs[0].history();
    for how in ["Ctrl+F1", "the switch"] {
        match how {
            "Ctrl+F1" => {
                window(&mut app, &ctx, 1400.0, vec![minimal_key_event()]);
            }
            _ => click_switch(&mut app, &ctx),
        }
        settled_at(&mut app, &ctx, 1400.0);
        let adding = app.tabs[0].ed.adding.is_some();
        assert!(adding, "{how}: the new text was ended");
        assert_eq!(app.tabs[0].history(), steps, "{how}: a change");
        assert_eq!(ctx.memory(|m| m.focused()), typing, "{how}: the keyboard");
    }
    assert!(!app.settings.minimal);
}

/// A placing tool chosen stays chosen through a switch, pinned or not, and the pins stay.
#[test]
fn a_switch_keeps_the_tool_and_its_pins() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    for pinned in [false, true] {
        let mut app = app_with(&ctx, 1);
        if pinned {
            app.settings.keep_tools = vec![Tool::Note];
        }
        app.act(&ctx, Act::Tool(Tool::Note));
        settled_at(&mut app, &ctx, 1400.0);
        for _ in 0..2 {
            click_switch(&mut app, &ctx);
            settled_at(&mut app, &ctx, 1400.0);
            assert_eq!(app.tabs[0].ed.tool, Tool::Note, "pinned {pinned}");
            assert_eq!(app.settings.keep_tools.contains(&Tool::Note), pinned);
        }
    }
}

/// The keyboard on a control the other interface leaves out goes back to the typing open
/// on the page, else to the switch; the switch itself keeps it through a switch.
#[test]
fn the_keyboard_never_stays_on_a_control_gone() {
    // The band's font size box, while a line is edited: back to the line.
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    double_click_word(&mut app, &ctx, "fishing");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let (size, _) = node(&tree, "Font size").expect("the band's font size");
    window(&mut app, &ctx, 1400.0, click_at(size.center()));
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Font size"));
    window(&mut app, &ctx, 1400.0, vec![minimal_key_event()]);
    settled_at(&mut app, &ctx, 1400.0);
    assert!(app.settings.minimal);
    assert!(app.tabs[0].ed.editing_line(), "the line was ended");
    let typing = app.tabs[0].ed.typing_id();
    assert_eq!(ctx.memory(|m| m.focused()), typing, "not back in the line");

    // A band's name, with no typing: to the switch, which keeps it as it is pressed.
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    settled_at(&mut app, &ctx, 1400.0);
    let file = (0..8).any(|_| f6(&mut app, &ctx, false).as_deref() == Some("File"));
    assert!(file, "F6 never reached the commands");
    let tab = key_with(Key::Tab, egui::Modifiers::NONE);
    for _ in 0..4 {
        window(&mut app, &ctx, 1400.0, vec![tab.clone()]);
    }
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Edit"));
    window(&mut app, &ctx, 1400.0, vec![minimal_key_event()]);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(app.settings.minimal);
    let on = focused_name(&ctx, &tree);
    assert_eq!(on.as_deref(), Some(TO_FULL), "not on the switch");
    let space = key_with(Key::Space, egui::Modifiers::NONE);
    window(&mut app, &ctx, 1400.0, vec![space]);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(!app.settings.minimal, "Space on the switch");
    let on = focused_name(&ctx, &tree);
    assert_eq!(on.as_deref(), Some(TO_MINIMAL), "the switch let go");
}

/// F6 and Shift+F6 go round Minimal's regions as Full's: the inspector, the status bar, the
/// tabs, the commands, the navigation panel and the page.
#[test]
fn f6_goes_round_minimal_s_regions() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::ToggleMinimal);
    settled_at(&mut app, &ctx, 1400.0);
    let round = [
        Some("Collapse the inspector"),
        Some("Page view"),
        Some("Untitled, unsaved"),
        Some("File"),
        Some("Pages"),
        None,
        Some("Collapse the inspector"),
    ];
    let mut reached = Vec::new();
    for _ in 0..round.len() {
        reached.push(f6(&mut app, &ctx, false));
    }
    let want: Vec<Option<String>> = round.iter().map(|s| s.map(str::to_string)).collect();
    assert_eq!(reached, want);
    let mut back = Vec::new();
    for _ in 0..round.len() - 1 {
        back.push(f6(&mut app, &ctx, true));
    }
    let mut want_back = want[..round.len() - 1].to_vec();
    want_back.reverse();
    assert_eq!(back, want_back);
}

/// Minimal's row at 1000, 760 and 520 points wide, and under 600 high: nothing overlaps or
/// leaves the window, and the File menu is there, the switch at the tab strip's end, clear
/// of the tab; under 560 points Find and Print give their labels up for their icons, Tools
/// keeping its; every command has room, so no More.
#[test]
fn minimal_s_row_fits_narrow_and_short_windows() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::ToggleMinimal);
    for size in [
        egui::vec2(1000.0, 660.0),
        egui::vec2(760.0, 560.0),
        egui::vec2(520.0, 560.0),
    ] {
        let tree = drawn(&mut app, &ctx, size, vec![]);
        let r = row(&tree);
        let n = names(&r);
        assert_eq!(n.first(), Some(&"File"), "{n:?} at {size:?}");
        assert_eq!(n.len(), 10, "{n:?} at {size:?}");
        assert!(!n.contains(&"More"), "{n:?} at {size:?}");
        let (tools, _) = node(&tree, "Tools").unwrap();
        assert!(tools.width() > 50.0, "Tools labelled at {size:?}");
        for pair in r.windows(2) {
            let apart = pair[0].1.right() <= pair[1].1.left();
            assert!(apart, "{pair:?} at {size:?}");
        }
        let inside = |b: &egui::Rect| b.left() >= 0.0 && b.right() <= size.x;
        assert!(r.iter().all(|(_, b)| inside(b)), "{r:?} at {size:?}");
        let (switch, _) = node(&tree, TO_FULL).expect("the switch");
        let (tab, _) = node(&tree, "Untitled, unsaved").expect("the tab");
        let clear = inside(&switch) && tab.right() <= switch.left();
        assert!(clear, "{switch:?} at {size:?}");
        let (find, _) = node(&tree, "Find").unwrap();
        let label = find.width() > 40.0;
        assert_eq!(label, size.x >= 560.0, "Find's label at {size:?}");
    }
}

/// Settings has Minimal interface beside Compact commands, which it greys; ticked there it
/// switches as the switch does.
#[test]
fn settings_has_the_interface() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::Settings);
    let size = egui::vec2(1000.0, 800.0);
    let tree = dialog_frame(&mut app, &ctx, size, 1.0, vec![]);
    assert_eq!(ticked(&tree, "Minimal interface"), Some(false));
    let Dialog::Settings(p) = &mut app.dialog else {
        panic!("Settings isn't open");
    };
    p.minimal = true;
    let tree = dialog_frame(&mut app, &ctx, size, 1.0, vec![]);
    assert!(app.settings.minimal, "not switched");
    let (_, compact) = (tree.nodes.iter())
        .find(|(_, n)| n.label() == Some("Compact commands"))
        .expect("Compact commands");
    assert!(compact.is_disabled(), "Compact commands not greyed");
}

/// An unwritable settings file is said, and the choice still holds for the session.
#[test]
fn an_unsaved_choice_is_said_and_still_holds() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let dir = test_dir("minimal-unwritable");
    // A file where the settings folder would be.
    std::fs::write(dir.join("blocker"), "a file, not a folder").unwrap();
    let mut app = kept_app(&dir.join("blocker").join("settings.txt"));
    click_switch(&mut app, &ctx);
    settle(&mut app, &ctx);
    let (title, _) = message(&app);
    assert_eq!(title, "Settings not saved");
    assert!(app.settings.minimal);
    app.dialog = Dialog::None;
    let tree = settle(&mut app, &ctx);
    assert_eq!(goes_to_minimal(&tree), Some(false), "not Minimal now");
}

// ENH-45's stage 3: Tools, the inspector's Font group, More.

/// Where the control `name` is in a window `size`: of that name exactly, else the first
/// whose name starts with it (a menu item's name is its label, then its key's text).
pub(super) fn place(tree: &egui::accesskit::TreeUpdate, name: &str) -> Option<egui::Rect> {
    let exact = node(tree, name).map(|(r, _)| r);
    exact.or_else(|| {
        (tree.nodes.iter())
            .filter(|(_, n)| n.label().is_some_and(|l| l.starts_with(name)))
            .find_map(|(_, n)| n.bounds().map(rect))
    })
}

/// Click the control `name` in a window `size`, then settle.
pub(super) fn click_in(app: &mut App, ctx: &egui::Context, size: egui::Vec2, name: &str) {
    let tree = drawn(app, ctx, size, vec![]);
    let at = place(&tree, name).unwrap_or_else(|| panic!("no {name}"));
    drawn(app, ctx, size, click_at(at.center()));
}

/// Where the text `name` shown as a label (not a button) is: a label's text is its value.
fn label_at(tree: &egui::accesskit::TreeUpdate, name: &str) -> Option<egui::Rect> {
    (tree.nodes.iter())
        .filter(|(_, n)| n.role() == Role::Label && n.value() == Some(name))
        .find_map(|(_, n)| n.bounds().map(rect))
}

pub(super) const WIDE: egui::Vec2 = egui::vec2(1400.0, 800.0);

/// Tools: under the band's names, in the band's order down two columns, the commands a task
/// starts from, labelled, with no planned feature, all inside a 1000 × 660 window; a
/// command picked runs as the band's button does and closes Tools; one in a command's own
/// menu too (Draw's shapes).
#[test]
fn tools_lists_the_tasks_by_kind_and_runs_them() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::ToggleMinimal);
    let laptop = egui::vec2(1000.0, 660.0);
    click_in(&mut app, &ctx, laptop, "Tools");
    let tree = drawn(&mut app, &ctx, laptop, vec![]);
    let at: Vec<egui::Rect> = (band::TOOLS.iter())
        .map(|(b, _)| label_at(&tree, b.name()).unwrap_or_else(|| panic!("no {}", b.name())))
        .collect();
    let down = |c: &[egui::Rect]| c.windows(2).all(|w| w[0].top() < w[1].top());
    assert!(down(&at[..3]) && down(&at[3..]), "{at:?}");
    assert!(at[2].right() < at[3].left(), "not two columns: {at:?}");
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, laptop);
    for name in ["Stamp", "Passwords", "All commands"] {
        let r = place(&tree, name).unwrap_or_else(|| panic!("no {name}"));
        assert!(window.contains_rect(r), "{name} outside the window: {r:?}");
    }
    for name in [
        "Edit text",
        "Paragraph",
        "Select",
        "Align objects",
        "Text",
        "Image",
        "Note",
        "Comments",
        "Highlight",
        "Text box",
        "Draw",
        "Stamp",
        "Organise",
        "Insert pages",
        "Rotate",
        "Move",
        "Fill fields",
        "Fill and sign",
        "Digital signature",
        "Mark redaction",
        "Apply redactions…",
        "Passwords",
        "All commands",
    ] {
        assert!(node(&tree, name).is_some(), "no {name} in Tools");
    }
    for p in band::all_planned() {
        assert!(node(&tree, &p.name()).is_none(), "{} in Tools", p.label);
    }
    // Picked, as the band's Note: the tool on, Tools closed.
    click_in(&mut app, &ctx, WIDE, "Note");
    assert_eq!(app.tabs[0].ed.tool, Tool::Note);
    let tree = drawn(&mut app, &ctx, WIDE, vec![]);
    assert!(node(&tree, "Passwords").is_none(), "Tools still open");
    // A shape from Draw's menu in it.
    click_in(&mut app, &ctx, WIDE, "Tools");
    click_in(&mut app, &ctx, WIDE, "Draw options");
    let shape = actions::get(Act::Tool(Tool::Rect)).label;
    click_in(&mut app, &ctx, WIDE, shape);
    assert_eq!(app.tabs[0].ed.tool, Tool::Rect);
    let tree = drawn(&mut app, &ctx, WIDE, vec![]);
    let open = node(&tree, "Passwords").is_some();
    assert!(!open, "Tools open after a shape");
}

/// Tools' All commands opens the File menu, where its groups always are.
#[test]
fn all_commands_opens_the_file_menu() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::ToggleMinimal);
    let tree = drawn(&mut app, &ctx, WIDE, vec![]);
    assert!(node(&tree, "Export").is_none(), "the File menu open first");
    click_in(&mut app, &ctx, WIDE, "Tools");
    click_in(&mut app, &ctx, WIDE, "All commands");
    let tree = drawn(&mut app, &ctx, WIDE, vec![]);
    assert!(node(&tree, "Passwords").is_none(), "Tools still open");
    let (file, _) = node(&tree, "File").unwrap();
    let export = place(&tree, "Export").expect("the File menu");
    let under = export.top() >= file.bottom() - 1.0;
    assert!(under, "{export:?} not under the File menu");
}

/// Where the inspector's left edge is: its resize handle's.
fn inspector_left(tree: &egui::accesskit::TreeUpdate) -> f32 {
    let (edge, _) = node(tree, "Inspector width").expect("the inspector's edge");
    edge.center().x
}

/// Where the Font size box is in the inspector (the window's right), if it shows.
fn inspector_font(tree: &egui::accesskit::TreeUpdate) -> Option<egui::Rect> {
    (tree.nodes.iter())
        .filter(|(_, n)| n.label() == Some("Font size"))
        .filter_map(|(_, n)| n.bounds().map(rect))
        .find(|r| r.left() > 1100.0)
}

/// Minimal has no band: while text is the target (a line or paragraph being edited, the
/// Text tool's new text) its inspector heads with Edit's Font group and the alignments;
/// not with nothing selected, and not in Full, whose band has them. A style changed there
/// leaves the line open with the keyboard back in it.
#[test]
fn minimal_s_inspector_has_the_font_group_while_text_is_the_target() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    app.act(&ctx, Act::ToggleMinimal);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(inspector_font(&tree).is_none(), "nothing selected");
    let panel = inspector_left(&tree);
    double_click_word(&mut app, &ctx, "fishing");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing one line"));
    let at = inspector_font(&tree).expect("the Font group, a line edited");
    // In the panel's own width, give or take its last point: the page doesn't move.
    let now = inspector_left(&tree);
    assert!((now - panel).abs() <= 1.0, "widened: {panel} to {now}");
    let (justify, _) = node(&tree, "Justify").expect("the alignments");
    let below = justify.left() > 1100.0 && justify.top() > at.top();
    assert!(below, "{justify:?}");
    // Bold in the inspector, below its Font size.
    let bold = (tree.nodes.iter())
        .filter(|(_, n)| n.label() == Some("Bold"))
        .filter_map(|(_, n)| n.bounds().map(rect))
        .find(|r| r.left() > 1100.0)
        .expect("Bold in the inspector");
    window(&mut app, &ctx, 1400.0, click_at(bold.center()));
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing one line"), "Bold ended it");
    assert!(app.tabs[0].ed.pending(), "the typing lost");
    let typing = app.tabs[0].ed.typing_id();
    assert_eq!(ctx.memory(|m| m.focused()), typing, "the keyboard");

    app.act(&ctx, Act::EditParagraph);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing a paragraph"));
    assert!(inspector_font(&tree).is_some(), "a paragraph edited");

    // Full: the band's, not the inspector's.
    app.act(&ctx, Act::ToggleMinimal);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(inspector_font(&tree).is_none(), "in Full's inspector");
    let sizes = (tree.nodes.iter()).filter(|(_, n)| n.label() == Some("Font size"));
    assert_eq!(sizes.count(), 1, "the band's alone");

    // The Text tool: the next new text's look.
    app.act(&ctx, Act::ToggleMinimal);
    app.act(&ctx, Act::Tool(Tool::Text));
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(inspector_font(&tree).is_some(), "the Text tool");
}

/// Tools > Paragraph while a line is edited switches it to a paragraph, as the band's
/// Paragraph does: the click on Tools leaves the line open, and no change is made.
#[test]
fn tools_paragraph_switches_the_line_being_edited() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    app.act(&ctx, Act::ToggleMinimal);
    double_click_word(&mut app, &ctx, "fishing");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing one line"));
    let steps = app.tabs[0].history();
    click_in(&mut app, &ctx, WIDE, "Tools");
    let tree = drawn(&mut app, &ctx, WIDE, vec![]);
    assert!(status_says(&tree, "Editing one line"), "Tools ended it");
    click_in(&mut app, &ctx, WIDE, "Paragraph");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing a paragraph"), "not switched");
    assert!(app.settings.paragraphs, "Text flow's mode");
    assert_eq!(app.tabs[0].history(), steps, "a change");
}

/// Narrower than its commands, Minimal's row puts the least needed under More: the File
/// menu, Tools and More show, nothing overlaps or leaves the window, Open (the last to go)
/// stays; each of the row's commands is either in the row or in More's menu, which runs
/// it.
#[test]
fn more_holds_the_commands_without_room() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::ToggleMinimal);
    let size = egui::vec2(300.0, 560.0);
    let tree = drawn(&mut app, &ctx, size, vec![]);
    let r = row(&tree);
    let n = names(&r);
    for name in ["File", "Open", "Tools", "More"] {
        assert!(n.contains(&name), "no {name}: {n:?}");
    }
    assert!(!n.contains(&"Navigation panel"), "{n:?}");
    for pair in r.windows(2) {
        assert!(pair[0].1.right() <= pair[1].1.left(), "{pair:?}");
    }
    let inside = |b: &egui::Rect| b.left() >= 0.0 && b.right() <= size.x;
    assert!(r.iter().all(|(_, b)| inside(b)), "{r:?}");
    let in_row = |a: Act| n.contains(&actions::get(a).label.trim_end_matches('…'));
    let folded: Vec<Act> = (ROW_ORDER.into_iter()).filter(|&a| !in_row(a)).collect();
    assert!(folded.contains(&Act::Find), "{n:?}");
    click_in(&mut app, &ctx, size, "More");
    let tree = drawn(&mut app, &ctx, size, vec![]);
    for a in &folded {
        let label = actions::get(*a).label;
        assert!(place(&tree, label).is_some(), "{label} not in More");
    }
    click_in(&mut app, &ctx, size, actions::get(Act::Find).label);
    assert_eq!(app.tabs[0].side, Side::Search, "Find from More");
}

// ENH-45's stage 4: the acceptance's remaining checks.

/// Closing tabs in Minimal is Full's: the tab in front closed, a neighbour comes to the
/// front; an unsaved one asks first; the last closed leaves the start screen, the switch
/// still at the strip's end.
#[test]
fn closing_tabs_in_minimal_is_as_in_full() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    app.settings.minimal = true;
    let ids = named_tabs(&mut app, &ctx);
    app.active = 1;
    close_by_button(&mut app, &ctx, "b.pdf");
    let left: Vec<u64> = app.tabs.iter().map(|t| t.id).collect();
    assert_eq!(left, [ids[0], ids[2]]);
    assert!(app.active < 2, "no tab in front");
    app.tabs[0].dirty = true;
    close_by_button(&mut app, &ctx, "a.pdf *");
    assert!(matches!(app.dialog, Dialog::ConfirmClose(..)), "didn't ask");
    click(&mut app, &ctx, "Discard");
    assert_eq!(app.tabs.len(), 1);
    close_by_button(&mut app, &ctx, "c.pdf");
    assert!(app.tabs.is_empty());
    let tree = settle(&mut app, &ctx);
    assert!(node(&tree, "Open PDF…").is_some(), "no start screen");
    assert_eq!(goes_to_minimal(&tree), Some(false), "the switch");
    assert!(app.settings.minimal);
}

/// A new text box being typed stays open with the keyboard through a switch, by the key and
/// by the switch; Esc then ends it as usual, what was typed kept as the text box.
#[test]
fn a_switch_leaves_a_new_text_box_open() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::Tool(Tool::TextBox));
    settled_at(&mut app, &ctx, 1400.0);
    click_page(&mut app, &ctx, (100.0, 100.0), false);
    let typed = vec![egui::Event::Text("Walrus".into())];
    window(&mut app, &ctx, 1400.0, typed);
    settled_at(&mut app, &ctx, 1400.0);
    assert!(app.tabs[0].ed.writing.is_some(), "no text box");
    let typing = app.tabs[0].ed.typing_id();
    let steps = app.tabs[0].history();
    let boxes = app.tabs[0].pdf.annotations().len();
    for how in ["the key", "the switch"] {
        match how {
            "the key" => {
                window(&mut app, &ctx, 1400.0, vec![minimal_key_event()]);
            }
            _ => click_switch(&mut app, &ctx),
        }
        settled_at(&mut app, &ctx, 1400.0);
        let open = app.tabs[0].ed.writing.is_some();
        assert!(open, "{how}: the text box was ended");
        assert_eq!(app.tabs[0].history(), steps, "{how}: a change");
        assert_eq!(ctx.memory(|m| m.focused()), typing, "{how}: the keyboard");
    }
    let esc = key_with(Key::Escape, egui::Modifiers::NONE);
    window(&mut app, &ctx, 1400.0, vec![esc]);
    settled_at(&mut app, &ctx, 1400.0);
    assert!(app.tabs[0].ed.writing.is_none(), "Esc didn't end it");
    let now = app.tabs[0].pdf.annotations().len();
    assert_eq!(now, boxes + 1, "the text box wasn't kept");
}

/// Docked controls stay docked through switches and narrow windows, in the panel or its
/// strip; floating, the bar shows with a selection only. The panels' choices are one set,
/// unchanged by switching or by a narrow window folding them.
#[test]
fn docking_and_panels_hold_through_switches_and_narrow_windows() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    with_notes(&mut app, &ctx);
    let id = app.tabs[0].pdf.annotations()[0].id;
    let note = Selection::Annot { page: 0, id };
    app.tabs[0].ed.selection = note.clone();
    let dock = Some("Dock controls on the right".to_string());
    let float = Some("Float controls near selection".to_string());
    // Floating: the bar with the note selected, in either interface; none without it.
    for minimal in [false, true] {
        app.settings.minimal = minimal;
        let names = focusable(&mut app, &ctx, 1400.0);
        assert!(names.contains(&dock), "no floating bar, Minimal {minimal}");
    }
    app.tabs[0].ed.selection = Selection::default();
    let names = focusable(&mut app, &ctx, 1400.0);
    assert!(!names.contains(&dock), "a bar with nothing selected");
    // Docked: kept through switches and resizes.
    app.tabs[0].ed.selection = note;
    app.act(&ctx, Act::ToggleDock);
    let ways = [
        (true, 1400.0),
        (true, 700.0),
        (false, 700.0),
        (true, 1400.0),
    ];
    for (minimal, width) in ways {
        app.settings.minimal = minimal;
        let names = focusable(&mut app, &ctx, width);
        let docked = names.contains(&float) && !names.contains(&dock);
        assert!(docked, "Minimal {minimal} at {width}");
    }
    assert!(app.settings.controls_docked);
    // The navigation panel closed by its button stays closed; the inspector stays open.
    app.act(&ctx, Act::ToggleSide);
    let kept = |app: &App| (app.settings.navigation_panel, app.settings.inspector_panel);
    let chosen = kept(&app);
    assert_eq!(chosen, (false, true));
    for (minimal, width) in [(false, 1400.0), (true, 700.0), (true, 1400.0)] {
        app.settings.minimal = minimal;
        settled_at(&mut app, &ctx, width);
        assert_eq!(kept(&app), chosen, "Minimal {minimal} at {width}");
    }
    assert!(!app.side.shown() && app.inspector.shown());
}

/// The whole window `size` at `scale` pixels a point, settled; its AccessKit tree.
fn drawn_scaled(
    app: &mut App,
    ctx: &egui::Context,
    size: egui::Vec2,
    scale: f32,
    events: Vec<egui::Event>,
) -> egui::accesskit::TreeUpdate {
    let mut events = Some(events);
    let mut tree = None;
    for _ in 0..6 {
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events: events.take().unwrap_or_default(),
            ..Default::default()
        };
        let viewport = raw.viewports.entry(egui::ViewportId::ROOT).or_default();
        viewport.native_pixels_per_point = Some(scale);
        let mut frame = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut frame));
        out.textures_delta.clear();
        tree = out.platform_output.accesskit_update;
    }
    tree.expect("the window's widgets")
}

/// Minimal at scales 1 and 2, in the light, dark and System themes with each accent: the
/// row and Tools inside the window, nothing overlapping, every control clearly named, and
/// the row's buttons as big as Full's.
#[test]
fn minimal_holds_at_each_scale_theme_and_accent() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    let size = egui::vec2(1000.0, 660.0);
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let full = drawn_scaled(&mut app, &ctx, size, 1.0, vec![]);
    let (open_full, _) = node(&full, "Open").unwrap();
    app.act(&ctx, Act::ToggleMinimal);
    let themes = [
        egui::ThemePreference::Light,
        egui::ThemePreference::Dark,
        egui::ThemePreference::System,
    ];
    for scale in [1.0, 2.0] {
        for theme in themes {
            for accent in [Accent::Desktop, Accent::LimarePdf] {
                let at = format!("scale {scale}, {theme:?}, {accent:?}");
                app.act(&ctx, Act::Theme(theme));
                app.act(&ctx, Act::Accent(accent));
                let tree = drawn_scaled(&mut app, &ctx, size, scale, vec![]);
                let r = row(&tree);
                assert_eq!(r.len(), 10, "{at}: {:?}", names(&r));
                for pair in r.windows(2) {
                    assert!(pair[0].1.right() <= pair[1].1.left(), "{at}: {pair:?}");
                }
                assert!(r.iter().all(|(_, b)| window.contains_rect(*b)), "{at}");
                assert_eq!(dlg::unclear_names(&tree), [""; 0], "{at}");
                let (open, _) = node(&tree, "Open").unwrap();
                let big = open.height() >= open_full.height() - 0.5
                    && open.width() >= open_full.width() - 0.5;
                assert!(big, "{at}: {open:?}");
            }
        }
        // Tools open at this scale, inside the window, its foot too.
        let tree = drawn_scaled(&mut app, &ctx, size, scale, vec![]);
        let tools = place(&tree, "Tools").unwrap();
        drawn_scaled(&mut app, &ctx, size, scale, click_at(tools.center()));
        let tree = drawn_scaled(&mut app, &ctx, size, scale, vec![]);
        let foot = place(&tree, "All commands").expect("Tools open");
        assert!(window.contains_rect(foot), "scale {scale}: {foot:?}");
        assert_eq!(dlg::unclear_names(&tree), [""; 0], "Tools at scale {scale}");
        let esc = key_with(Key::Escape, egui::Modifiers::NONE);
        drawn_scaled(&mut app, &ctx, size, scale, vec![esc]);
    }
}
