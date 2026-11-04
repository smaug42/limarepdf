//! Settings (ENH-49): ☰ > Settings… (Ctrl+,) gathers what is set for every document in one
//! window; each change takes effect at once and is kept, with no document open too.

use super::*;
use crate::settings::PageLook;

/// An app with no document, its settings kept in a file of its own.
fn settings_app(name: &str) -> (App, PathBuf) {
    let dir = test_dir(name);
    let file = dir.join("settings.txt");
    let app = App::with(
        Settings::from_file(Some(file.clone())),
        Recent::from_file(None),
    );
    (app, file)
}

/// The window's choices.
fn prefs(app: &mut App) -> &mut dlg::Prefs {
    match &mut app.dialog {
        Dialog::Settings(p) => p,
        _ => panic!("Settings isn't open"),
    }
}

fn frame(app: &mut App, ctx: &egui::Context) -> egui::accesskit::TreeUpdate {
    dialog_frame(app, ctx, egui::vec2(1000.0, 800.0), 1.0, vec![])
}

#[test]
fn settings_is_in_the_file_menu_the_command_search_and_ctrl_comma() {
    let a = actions::get(Act::Settings);
    assert_eq!(menu::path(Act::Settings).unwrap(), ["Settings…"]);
    assert!(a.palette);
    let comma = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Comma);
    assert_eq!(a.keys, [comma]);
    assert!((a.enabled)(&Ctx::default()), "greyed with no document");
    let ctx = egui::Context::default();
    let (mut app, _) = settings_app("settings-open");
    app.act(&ctx, Act::Settings);
    assert!(matches!(app.dialog, Dialog::Settings(_)));
}

#[test]
fn each_change_takes_effect_at_once_and_is_kept() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let (mut app, file) = settings_app("settings-change");
    app.act(&ctx, Act::Settings);
    let tree = frame(&mut app, &ctx);
    for name in [
        "Your name for comments",
        "Choose the documents folder",
        "Keep recovery copies of unsaved changes",
        "Theme",
        "Accent colour",
        "Page colours",
        "Compact commands",
        "Page layout",
        "Snap to objects",
        "Check spelling as you type",
        "Spelling language",
        "Add a dictionary…",
        "Close",
    ] {
        assert!(node(&tree, name).is_some(), "{name}");
    }
    let p = prefs(&mut app);
    assert!(!p.compact && p.planned && p.snap && p.spelling && p.recovery);
    p.name = "Jane Doe".into();
    p.compact = true;
    p.planned = false;
    p.snap = false;
    p.spelling = false;
    p.fields = false;
    p.page_labels = false;
    p.stamp_name_date = false;
    p.layout = Layout::Facing;
    p.page_look = PageLook::Fixed(limarepdf_core::page_colours::PageColours::Sepia);
    p.theme = egui::ThemePreference::Dark;
    frame(&mut app, &ctx);
    // At once, with the window still open.
    assert!(matches!(app.dialog, Dialog::Settings(_)));
    let s = &app.settings;
    assert_eq!(s.author(), "Jane Doe");
    assert!(s.compact_commands && !s.show_planned && !s.snap && !s.spelling);
    assert!(!s.fields_shown && !s.page_labels && !s.stamp_name_date);
    assert_eq!(s.layout, Layout::Facing);
    assert_eq!(
        ctx.options(|o| o.theme_preference),
        egui::ThemePreference::Dark
    );
    // And kept.
    let kept = Settings::from_file(Some(file));
    assert_eq!(kept.author(), "Jane Doe");
    assert!(kept.compact_commands && !kept.show_planned && !kept.snap && !kept.spelling);
    assert_eq!(kept.layout, Layout::Facing);
    assert_eq!(kept.theme, egui::ThemePreference::Dark);
    let sepia = PageLook::Fixed(limarepdf_core::page_colours::PageColours::Sepia);
    assert_eq!(kept.page_look, sepia);
    click(&mut app, &ctx, "Close");
    assert!(matches!(app.dialog, Dialog::None));
    // Opened again, it shows them.
    app.act(&ctx, Act::Settings);
    let p = prefs(&mut app);
    assert_eq!(p.name, "Jane Doe");
    assert!(p.compact && !p.planned);
}

/// Open and Save As start in the documents folder set in Settings, as Word's default file
/// location; Save As starts beside the document's own file when it has one.
#[test]
fn open_and_save_as_start_in_the_documents_folder() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let (mut app, _) = settings_app("settings-folder");
    let docs = test_dir("settings-folder-docs");
    assert_eq!(App::start_folder(None, None), None);
    app.act(&ctx, Act::Settings);
    tests::FOLDER.with(|f| f.set(Some(docs.clone())));
    click(&mut app, &ctx, "Choose the documents folder");
    frame(&mut app, &ctx);
    assert_eq!(app.settings.folder.as_ref(), Some(&docs));
    let folder = app.settings.folder.as_deref();
    assert_eq!(App::start_folder(None, folder), Some(docs.clone()));
    // A new document's Save As starts there; one with a file, beside it.
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    assert!(app.add_tab(&ctx, pdf, None, None, false));
    let folder = app.settings.folder.as_deref();
    assert_eq!(
        App::start_folder(Some(&app.tabs[0]), folder),
        Some(docs.clone())
    );
    let elsewhere = test_dir("settings-folder-elsewhere");
    app.tabs[0].path = Some(elsewhere.join("a.pdf"));
    let folder = app.settings.folder.as_deref();
    assert_eq!(
        App::start_folder(Some(&app.tabs[0]), folder),
        Some(elsewhere)
    );
    // A folder that has gone isn't offered.
    let gone = docs.join("gone");
    assert_eq!(App::start_folder(None, Some(&gone)), None);
    // Clear goes back to the folder last used.
    app.dialog = Dialog::None;
    app.act(&ctx, Act::Settings);
    click(&mut app, &ctx, "Clear the documents folder");
    frame(&mut app, &ctx);
    assert_eq!(app.settings.folder, None);
}

#[test]
fn help_opens_the_guide_and_the_issues_page() {
    for (act, label) in [
        (Act::UserGuide, "User guide"),
        (Act::ReportProblem, "Report a problem"),
    ] {
        assert_eq!(menu::path(act).unwrap(), ["Help", label]);
        assert!((actions::get(act).enabled)(&Ctx::default()));
    }
    let ctx = egui::Context::default();
    let (mut app, _) = settings_app("settings-help");
    let opened = |app: &mut App, act| {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.act(ui.ctx(), act));
        out.textures_delta.clear();
        let url = out.platform_output.commands.iter().find_map(|c| match c {
            egui::OutputCommand::OpenUrl(u) => Some(u.url.clone()),
            _ => None,
        });
        url.expect("nothing opened")
    };
    assert!(opened(&mut app, Act::UserGuide).ends_with("/docs/README.md"));
    assert!(opened(&mut app, Act::ReportProblem).ends_with("/issues"));
}
