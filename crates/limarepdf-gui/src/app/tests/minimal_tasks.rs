//! ENH-45's stage 4: everyday tasks done in the Minimal interface by its own routes (its row,
//! Tools, the floating bar and the inspector), without the band: find a word, correct a typo,
//! fill a field, add a signature, highlight, save selected text, insert an image, open Print,
//! and mark then apply a redaction, its confirmation kept.

use super::minimal::{click_in, WIDE};
use super::paragraphs::{new_app, prose_tab, status_says};
use super::*;
use crate::signing::{Entry, Saved};
use limarepdf_core::{FieldValue, ObjectKind, SignRole, SignStyle};

/// `k` pressed alone.
fn press_key(k: Key) -> egui::Event {
    key_with(k, egui::Modifiers::NONE)
}

/// Where `word` is on the active tab's first page, in page points: its box's middle.
fn word_at(app: &App, word: &str) -> (f32, f32) {
    let words = app.tabs[app.active].pdf.page_words(0).unwrap();
    let w = words.iter().find(|w| w.text == word).expect(word).bbox;
    ((w[0] + w[2]) / 2.0, (w[1] + w[3]) / 2.0)
}

/// Whether the active tab's first page still has `word`.
fn has_word(app: &App, word: &str) -> bool {
    let words = app.tabs[app.active].pdf.page_words(0).unwrap();
    words.iter().any(|w| w.text.contains(word))
}

/// Drag across `word` with the read tool: the words selected, the floating bar by them.
fn select_word(app: &mut App, ctx: &egui::Context, word: &str) {
    let words = app.tabs[app.active].pdf.page_words(0).unwrap();
    let w = words.iter().find(|w| w.text == word).expect(word).bbox;
    let y = (w[1] + w[3]) / 2.0;
    drag_page(app, ctx, (w[0] + 0.5, y), (w[2] - 0.5, y), false);
}

/// Prose in Minimal, settled.
fn minimal_prose() -> (egui::Context, App) {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    app.act(&ctx, Act::ToggleMinimal);
    settled_at(&mut app, &ctx, 1400.0);
    (ctx, app)
}

/// Find a word from the row's Find: the search field takes the keyboard and finds it.
/// Correct a typo from Tools > Edit text: a click on the line, typing, Esc, one step kept.
#[test]
fn find_a_word_and_correct_a_typo() {
    let (ctx, mut app) = minimal_prose();
    click_named(&mut app, &ctx, "Find");
    settle(&mut app, &ctx);
    let typed = vec![egui::Event::Text("fishing".into())];
    window(&mut app, &ctx, 1400.0, typed);
    window(&mut app, &ctx, 1400.0, vec![press_key(Key::Enter)]);
    searched(&mut app, &ctx);
    assert_eq!(app.tabs[0].query, "fishing", "the search field");
    assert!(!app.tabs[0].hits.is_empty(), "nothing found");

    let steps = app.tabs[0].history();
    click_in(&mut app, &ctx, WIDE, "Tools");
    click_in(&mut app, &ctx, WIDE, "Edit text");
    let at = word_at(&app, "fishing");
    click_page(&mut app, &ctx, at, false);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing one line"), "not editing it");
    window(&mut app, &ctx, 1400.0, vec![egui::Event::Text("Q".into())]);
    settled_at(&mut app, &ctx, 1400.0);
    window(&mut app, &ctx, 1400.0, vec![press_key(Key::Escape)]);
    settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(app.tabs[0].history().1, steps.1 + 1, "not one step");
    assert!(has_word(&app, "Q"), "the typing wasn't kept");
}

/// Fill a field by clicking it and typing, Enter keeping it; then add a saved signature
/// from Tools > Fill and sign with a click on the page.
#[test]
fn fill_a_field_and_add_a_signature() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let ty = Typist::new(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    app.settings.minimal = true;
    let path = test_dir("minimal-tasks-form").join("form.pdf");
    form_file(&path, "");
    open_and_wait(&mut app, &ctx, &path, 1);
    ty.click_field(&mut app, "Name", 0);
    ty.text(&mut app, "Ann");
    ty.key(&mut app, Key::Enter, egui::Modifiers::NONE);
    ty.pause(&mut app);
    let name = (app.tabs[0].pdf.form_fields().into_iter()).find(|f| f.name == "Name");
    let value = name.expect("the Name field").value;
    assert_eq!(value, FieldValue::Text("Ann".into()));

    let saved = Saved::Typed {
        text: "Sam Lee".into(),
        style: SignStyle::ALL[0],
    };
    let entry = Entry { saved, keep: false };
    assert_eq!(app.settings.sign.add(SignRole::Signature, entry), None);
    let click = |app: &mut App, name: &str| {
        let tree = ty.pause(app);
        let p = middle(&tree, name);
        ty.frame(app, vec![egui::Event::PointerMoved(p)]);
        ty.frame(app, vec![press(p, true)]);
        ty.frame(app, vec![press(p, false)]);
    };
    click(&mut app, "Tools");
    click(&mut app, "Fill and sign");
    assert_eq!(app.tabs[0].ed.tool, Tool::Sign, "Fill and sign");
    let (min, k) = ty.page(&mut app);
    let p = min + egui::vec2(300.0, 500.0) * k;
    ty.frame(&mut app, vec![egui::Event::PointerMoved(p)]);
    ty.frame(&mut app, vec![press(p, true)]);
    ty.frame(&mut app, vec![press(p, false)]);
    ty.pause(&mut app);
    assert_eq!(app.tabs[0].pdf.sign_items(), 1, "no signature placed");
}

/// Highlight words from the floating bar by them; save selected text from it too, which
/// asks how before choosing a file.
#[test]
fn highlight_and_save_selected_text() {
    let (ctx, mut app) = minimal_prose();
    let marks = |app: &App| app.tabs[0].pdf.annotations().len();
    select_word(&mut app, &ctx, "fishing");
    let selected = app.tabs[0].ed.selection.words().is_some();
    assert!(selected, "no words selected");
    click_named(&mut app, &ctx, "Highlight");
    settle(&mut app, &ctx);
    assert_eq!(marks(&app), 1, "not highlighted");

    select_word(&mut app, &ctx, "gulls");
    click_named(&mut app, &ctx, "Save selected text");
    settle(&mut app, &ctx);
    let asked = matches!(app.dialog, Dialog::SaveSelection(..));
    assert!(asked, "didn't ask");
    app.dialog = Dialog::None;
}

/// Insert an image: Tools > Image is the band's command, which asks the system for a file
/// (never reached from tests); the picture it gives is placed with a click. Then the row's
/// Print opens Print.
#[test]
fn insert_an_image_and_open_print() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::ToggleMinimal);
    settled_at(&mut app, &ctx, 1400.0);
    let images = |app: &App| {
        let objs = app.tabs[0].pdf.page_objects(0).unwrap();
        objs.iter().filter(|o| o.kind == ObjectKind::Image).count()
    };
    app.tabs[0].arm_image(turned_photo());
    click_page(&mut app, &ctx, (200.0, 300.0), false);
    settle(&mut app, &ctx);
    assert_eq!(images(&app), 1, "no picture placed");

    click_named(&mut app, &ctx, "Print");
    assert!(matches!(app.dialog, Dialog::Print(..)), "Print isn't open");
    app.dialog = Dialog::None;
}

/// Mark a redaction with Tools > Mark redaction over a word, then apply it from Tools >
/// Apply redactions…, which asks first: the word is gone from the page.
#[test]
fn mark_and_apply_a_redaction() {
    let (ctx, mut app) = minimal_prose();
    click_in(&mut app, &ctx, WIDE, "Tools");
    click_in(&mut app, &ctx, WIDE, "Mark redaction");
    assert_eq!(app.tabs[0].ed.tool, Tool::Redact);
    let words = app.tabs[0].pdf.page_words(0).unwrap();
    let w = words.iter().find(|w| w.text == "fishing").unwrap().bbox;
    let (from, to) = ((w[0] - 2.0, w[1] - 2.0), (w[2] + 2.0, w[3] + 2.0));
    drag_page(&mut app, &ctx, from, to, false);
    assert_eq!(app.tabs[0].pdf.annotations().len(), 1, "no mark");
    assert!(has_word(&app, "fishing"), "removed before applying");

    click_in(&mut app, &ctx, WIDE, "Tools");
    click_in(&mut app, &ctx, WIDE, "Apply redactions…");
    let asked = matches!(app.dialog, Dialog::ConfirmApplyRedactions(..));
    assert!(asked, "didn't ask first");
    click(&mut app, &ctx, "Apply redactions");
    settle(&mut app, &ctx);
    assert!(!has_word(&app, "fishing"), "the word is still there");
}
