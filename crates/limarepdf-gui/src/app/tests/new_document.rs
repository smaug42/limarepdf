//! A new blank document (NEW-09's stage 1): Insert > Blank document, ☰ > New blank document,
//! the start screen, the command search and Ctrl+N (⌘N) open an "Untitled" tab of one blank
//! page on the region's paper. Untouched, it closes without asking, as Word's does; changed,
//! closing asks, and Save saves it as a new file.

use super::*;

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

/// The tab in front is a new blank document: untitled, one page of the region's paper,
/// nothing to save.
fn is_blank_document(app: &App) {
    let t = &app.tabs[app.active];
    assert_eq!(t.title(), "Untitled");
    assert!(t.path.is_none() && !t.unsaved() && !t.can_undo());
    let paper = limarepdf_core::print::new_document_size(paper_locale().as_deref());
    assert_eq!(t.sizes, [paper]);
    let a4 = (595.28, 841.89);
    let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 0.05 && (a.1 - b.1).abs() < 0.05;
    assert!(paper == (612.0, 792.0) || near(paper, a4), "{paper:?}");
}

#[test]
fn new_blank_document_is_in_the_menu_the_band_and_the_command_search() {
    let a = actions::get(Act::NewBlank);
    assert_eq!(a.label, "New blank document");
    assert_eq!(menu::path(Act::NewBlank).unwrap(), ["New blank document"]);
    assert!(a.palette);
    let n = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::N);
    assert_eq!(a.keys, [n]);
    assert!((a.enabled)(&Ctx::default()), "greyed with no document");
    assert!(Band::Insert.acts().contains(&Act::NewBlank));
    assert!(!Act::NewBlank.uses_document());
}

/// Ctrl+N opens one with nothing open, and another beside a document, in front.
#[test]
fn ctrl_n_opens_an_untitled_blank_page() {
    let ctx = test_ctx();
    let mut app = empty_app();
    settle(&mut app, &ctx);
    let ctrl_n = key_with(egui::Key::N, egui::Modifiers::COMMAND);
    window(&mut app, &ctx, 1400.0, vec![ctrl_n.clone()]);
    settle(&mut app, &ctx);
    assert_eq!(app.tabs.len(), 1);
    is_blank_document(&app);
    pages_tab(&mut app, &ctx, 3);
    window(&mut app, &ctx, 1400.0, vec![ctrl_n]);
    settle(&mut app, &ctx);
    assert_eq!((app.tabs.len(), app.active), (3, 2));
    is_blank_document(&app);
}

/// Insert's Blank document and the start screen's button open one too.
#[test]
fn the_band_and_the_start_screen_open_one() {
    let ctx = test_ctx();
    let mut app = empty_app();
    click_named(&mut app, &ctx, "New blank document");
    settle(&mut app, &ctx);
    assert_eq!(app.tabs.len(), 1, "the start screen's button");
    is_blank_document(&app);
    app.band = Band::Insert;
    click_named(&mut app, &ctx, "Blank document");
    settle(&mut app, &ctx);
    assert_eq!((app.tabs.len(), app.active), (2, 1), "the band's button");
    is_blank_document(&app);
}

/// Untouched, it closes without asking, as in Word; changed, closing asks, and Save saves it
/// as a new file through the save chooser.
#[test]
fn an_untouched_one_closes_without_asking() {
    let ctx = test_ctx();
    let mut app = empty_app();
    app.act(&ctx, Act::NewBlank);
    close_by_button(&mut app, &ctx, "Untitled");
    assert!(app.tabs.is_empty(), "not closed");
    assert!(matches!(app.dialog, Dialog::None), "it asked");

    let dir = test_dir("new-blank");
    app.act(&ctx, Act::NewBlank);
    assert!(app.tabs[0].edit(&ctx, |p| p.insert_blank(1, 612.0, 792.0)));
    assert_eq!(app.tabs[0].title(), "Untitled *");
    app.close_asking(0);
    let asked = matches!(app.dialog, Dialog::ConfirmClose(..));
    assert!(asked, "it didn't ask");
    SAVE_AS.with(|s| s.set(Some(dir.join("new"))));
    click(&mut app, &ctx, "Save as…");
    saves_done(&mut app, &ctx);
    assert_eq!(pages_in(&dir.join("new.pdf")), 2, "not saved as a file");
    assert!(app.tabs.is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}
