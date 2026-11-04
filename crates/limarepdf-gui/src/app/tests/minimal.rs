//! ENH-45's stage 2: the Minimal interface, one quiet row of commands in place of the
//! commands row and the band, chosen by the switch at the row's right end, View's Minimal
//! interface, Ctrl+F1 or Settings; one setting, kept, and a switch that changes nothing but
//! what is shown.

use super::paragraphs::{double_click_word, new_app, prose_tab, status_says};
use super::*;
use egui::accesskit::{Role, Toggled};

/// Linux's Ctrl, as egui-winit sends it.
const CTRL: egui::Modifiers = egui::Modifiers {
    alt: false,
    ctrl: true,
    shift: false,
    mac_cmd: false,
    command: true,
};

const SWITCH: &str = "Minimal interface";

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
    assert_eq!(ticked(&tree, SWITCH), Some(false), "Full first");
    assert!(node(&tree, "Open PDF…").is_some(), "the start screen");
    let view = |app: &App, ctx: &egui::Context| {
        let a = actions::get(Act::ToggleMinimal);
        (a.checked.unwrap())(&app.state(ctx))
    };
    assert!(!view(&app, &ctx));
    assert_eq!(
        menu::path(Act::ToggleMinimal).unwrap(),
        ["View", "Show", SWITCH]
    );
    let a = actions::get(Act::ToggleMinimal);
    assert!(a.palette, "the command search");

    // The switch, on the start screen.
    click_named(&mut app, &ctx, SWITCH);
    let tree = settle(&mut app, &ctx);
    assert!(app.settings.minimal && view(&app, &ctx));
    assert_eq!(ticked(&tree, SWITCH), Some(true));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("interface = minimal\n"), "{text}");

    // Restored at start, before any document opens.
    let mut again = kept_app(&file);
    let tree = settle(&mut again, &ctx);
    assert_eq!(ticked(&tree, SWITCH), Some(true), "not restored");
    assert!(node(&tree, "Navigation panel").is_some(), "Minimal's row");

    // Ctrl+F1, as Word folds its ribbon away, back to Full; kept so too.
    window(&mut again, &ctx, 1400.0, vec![key_with(Key::F1, CTRL)]);
    let tree = settle(&mut again, &ctx);
    assert!(!again.settings.minimal, "Ctrl+F1");
    assert_eq!(ticked(&tree, SWITCH), Some(false));
    let mut third = kept_app(&file);
    let tree = settle(&mut third, &ctx);
    assert_eq!(ticked(&tree, SWITCH), Some(false), "Full restored");
    // And View's choice, as the menus and the command search run it.
    third.act(&ctx, Act::ToggleMinimal);
    assert!(third.settings.minimal && view(&third, &ctx));
}

/// Minimal's one row: the File menu, the navigation panel, Open, Save, Undo and Redo, then
/// Find, the inspector, Print and the switch at the right end, with no band's names and no
/// band; Full's row is as it was, the switch at its end. The status bar has the page
/// layout and Fit width beside the zoom in Minimal, and no Show planned features.
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
    let full = row(&settled_at(&mut app, &ctx, 1400.0));
    assert_eq!(names(&full).last(), Some(&SWITCH), "{:?}", names(&full));

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
        "Inspector",
        "Print",
        SWITCH,
    ];
    assert_eq!(names(&minimal), want);
    for pair in minimal.windows(2) {
        assert!(pair[0].1.right() <= pair[1].1.left(), "{pair:?} overlap");
    }
    let end = minimal.last().unwrap().1.right();
    assert!(end > 1350.0, "the switch at the end");
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
    click_named(&mut app, &ctx, SWITCH);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(band_shown(&tree) && node(&tree, "Show planned features").is_some());
    let back = row(&tree);
    assert_eq!(names(&back), names(&full));
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
        click_named(&mut app, &ctx, SWITCH);
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
    app.act(&ctx, Act::ToggleMinimal);
    let tree = drawn(&mut app, &ctx, egui::vec2(1000.0, 660.0), vec![]);
    assert!(node(&tree, "All open documents").is_some(), "many tabs");
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
                    window(&mut app, &ctx, 1400.0, vec![key_with(Key::F1, CTRL)]);
                }
                false => click_named(&mut app, &ctx, SWITCH),
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
    ty.key(&mut app, Key::F1, CTRL);
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
                window(&mut app, &ctx, 1400.0, vec![key_with(Key::F1, CTRL)]);
            }
            _ => click_named(&mut app, &ctx, SWITCH),
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
            click_named(&mut app, &ctx, SWITCH);
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
    window(&mut app, &ctx, 1400.0, vec![key_with(Key::F1, CTRL)]);
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
    window(&mut app, &ctx, 1400.0, vec![key_with(Key::F1, CTRL)]);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(app.settings.minimal);
    let on = focused_name(&ctx, &tree);
    assert_eq!(on.as_deref(), Some(SWITCH), "not on the switch");
    let space = key_with(Key::Space, egui::Modifiers::NONE);
    window(&mut app, &ctx, 1400.0, vec![space]);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(!app.settings.minimal, "Space on the switch");
    let on = focused_name(&ctx, &tree);
    assert_eq!(on.as_deref(), Some(SWITCH), "the switch let go");
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
/// leaves the window, and the File menu and the switch are there; under 560 points Find and
/// Print give their labels up for their icons.
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
        let both = n.contains(&"File") && n.contains(&SWITCH);
        assert!(both, "{n:?} at {size:?}");
        assert_eq!(n.len(), 10, "{n:?} at {size:?}");
        for pair in r.windows(2) {
            let apart = pair[0].1.right() <= pair[1].1.left();
            assert!(apart, "{pair:?} at {size:?}");
        }
        let inside = |b: &egui::Rect| b.left() >= 0.0 && b.right() <= size.x;
        assert!(r.iter().all(|(_, b)| inside(b)), "{r:?} at {size:?}");
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
    assert_eq!(ticked(&tree, SWITCH), Some(false));
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
    click_named(&mut app, &ctx, SWITCH);
    let tree = settle(&mut app, &ctx);
    let (title, _) = message(&app);
    assert_eq!(title, "Settings not saved");
    assert!(app.settings.minimal);
    assert_eq!(ticked(&tree, SWITCH), Some(true));
}
