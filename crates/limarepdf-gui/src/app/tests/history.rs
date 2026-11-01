//! Batch 43 (PF-42): undo steps keep only what each change changed, so more of them fit,
//! and Undo puts the document back in place rather than reading a copy of it again, with
//! app.rs's helpers.

use super::*;
use crate::tab::Stamp;
use limarepdf_core::{Info, TextEdit};
use lopdf::{dictionary, Dictionary, Object, Stream};

fn test_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    ctx
}

/// The tab's document as it would be written now, with the highest object number the
/// highest in use: Undo doesn't lower it (no number is given to another object), and puts
/// back everything else.
fn snap(t: &Tab) -> Vec<u8> {
    let mut copy = t.pdf.clone();
    copy.doc.max_id = copy.doc.objects.keys().map(|id| id.0).max().unwrap_or(0);
    copy.snapshot().unwrap()
}

/// One page holding a grey picture of noise `side` pixels square, stored as it is: it
/// doesn't get smaller written out.
fn picture_doc(side: u32) -> Pdf {
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    let mut seed = 0x2545_f491_u32;
    let pixels: Vec<u8> = (0..side * side)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        })
        .collect();
    let image = pdf.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => side as i64,
            "Height" => side as i64, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        pixels,
    ));
    let draw = b"q 400 0 0 400 100 200 cm /Im0 Do Q".to_vec();
    let content = pdf.doc.add_object(Stream::new(Dictionary::new(), draw));
    let page = pdf.page_ids()[0];
    let d = pdf.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", content);
    d.set(
        "Resources",
        dictionary! { "XObject" => dictionary! { "Im0" => image } },
    );
    pdf
}

/// Pages holding `lines`, one line of text a page, at `path` (not written yet).
fn survey(app: &mut App, ctx: &egui::Context, lines: &[&str], path: Option<PathBuf>) {
    let mut pdf = Pdf::new();
    for (i, line) in lines.iter().enumerate() {
        pdf.insert_blank(i, 612.0, 792.0).unwrap();
        let font = FontChoice::default();
        pdf.add_text(i, (72.0, 100.0), line, font, 20.0, [0.0; 3])
            .unwrap();
    }
    assert!(app.add_tab(ctx, pdf, path, None, false));
}

/// A sticky note on page `p` at `at`, as a change.
fn note(t: &mut Tab, ctx: &egui::Context, p: usize, at: (f32, f32), text: &str) -> bool {
    t.edit(ctx, |pdf| {
        pdf.add_note(p, at, [1.0, 1.0, 0.0], text, "Pat Example")
            .map(drop)
    })
}

/// Whether any stream of the file at `path`, unpacked, holds `word`.
fn file_holds(path: &Path, word: &str) -> bool {
    let pdf = Pdf::open(path, None).unwrap();
    pdf.doc.objects.values().any(|o| match o {
        Object::Stream(s) => {
            let bytes = s
                .decompressed_content()
                .unwrap_or_else(|_| s.content.clone());
            bytes.windows(word.len()).any(|w| w == word.as_bytes())
        }
        _ => false,
    })
}

/// What a reader sees of a document: each page's size and text, and its comments (page,
/// kind and words).
#[derive(PartialEq)]
struct Reading {
    sizes: Vec<(f32, f32)>,
    text: Vec<String>,
    comments: Vec<(usize, String, String)>,
}

fn reading(pdf: &Pdf) -> Reading {
    Reading {
        sizes: pdf.page_sizes(),
        text: (0..pdf.page_count()).map(|p| pdf.page_text(p)).collect(),
        comments: (pdf.annotations().into_iter())
            .map(|a| (a.page, format!("{:?}", a.kind), a.contents))
            .collect(),
    }
}

/// Thirty notes added beside a large picture keep thirty steps in a few kilobytes each,
/// not a copy of the picture each.
#[test]
fn many_small_changes_beside_a_large_picture_keep_a_small_history() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    assert!(app.add_tab(&ctx, picture_doc(1600), None, None, false));
    let t = &mut app.tabs[0];
    assert!(snap(t).len() > 2_500_000);
    for i in 0..30 {
        let at = (60.0 + 15.0 * i as f32, 60.0);
        assert!(note(t, &ctx, 0, at, &format!("Note {i}")));
    }
    assert_eq!(t.history().1, 30, "steps kept");
    let held = t.undo_held();
    assert!(held < 1_000_000, "the steps hold {held} bytes");
    // Every step still goes back.
    for _ in 0..30 {
        t.undo(&ctx);
    }
    assert!(t.pdf.annotations().is_empty());
}

/// A hundred steps are kept, as Word and LibreOffice keep: after 120 changes, Undo goes
/// back to the document as it was after the 20th, and Redo forward to after the last.
#[test]
fn a_hundred_steps_are_kept() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    pages_tab(&mut app, &ctx, 1);
    let t = &mut app.tabs[0];
    let mut kept = Vec::new();
    for i in 1..=120 {
        let info = Info {
            title: Some(format!("Walrus survey {i}")),
            ..Default::default()
        };
        assert!(t.edit_setting(&ctx, None, |p| p.set_info(&info)));
        if i == 20 || i == 120 {
            kept.push(snap(t));
        }
    }
    assert_eq!(t.history().1, 100, "steps kept");
    for _ in 0..100 {
        t.undo(&ctx);
    }
    assert!(!t.can_undo());
    assert_eq!(t.pdf.info().title.as_deref(), Some("Walrus survey 20"));
    assert!(snap(t) == kept[0], "not as after the 20th change");
    for _ in 0..100 {
        t.redo(&ctx);
    }
    assert!(!t.can_redo());
    assert!(snap(t) == kept[1], "not as after the last change");
}

/// A password-protected document stays known as protected through Undo and Redo: signing
/// it is still refused, as before the change. Undo used to read a copy of the document
/// again, which forgot that the file was encrypted.
#[test]
fn undo_keeps_a_password_protected_document_protected() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("history-locked");
    let path = dir.join("locked.pdf");
    pages_file(&path, 300.0, 2, Some("walrus"));
    app.spawn_open(&ctx, path.clone(), None, None);
    wait_for(&mut app, &ctx, |a| a.opening.is_empty());
    assert_eq!(asked_password(&app), Some(path.clone()));
    answer(&mut app, &ctx, "walrus", "Open");
    wait_for(&mut app, &ctx, |a| a.tabs.len() == 1);
    assert!(app.tabs[0].pdf.encrypted());
    let refused = |app: &mut App, when: &str| {
        app.act(&ctx, Act::SignDigital);
        let (title, text) = message(app);
        assert_eq!(title, "Sign with digital ID", "{when}");
        assert!(text.contains("can't sign encrypted"), "{when}: {text}");
        app.dialog = Dialog::None;
    };
    refused(&mut app, "as opened");
    assert!(note(&mut app.tabs[0], &ctx, 0, (60.0, 60.0), "Private"));
    app.act(&ctx, Act::Undo);
    assert!(app.tabs[0].pdf.annotations().is_empty());
    assert!(app.tabs[0].pdf.encrypted(), "Undo forgot the password");
    refused(&mut app, "after Undo");
    app.act(&ctx, Act::Redo);
    assert_eq!(app.tabs[0].pdf.annotations().len(), 1);
    assert!(app.tabs[0].pdf.encrypted(), "Redo forgot the password");
    refused(&mut app, "after Redo");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Several kinds of change undone and redone go through each state as it first was; a
/// change after Undo starts a new branch; and what is saved reads as the tab does, the
/// words redacted (undone and redone first) nowhere in the file.
#[test]
fn changes_undone_redone_and_saved_agree() {
    let ctx = test_ctx();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("history-sequence");
    let path = dir.join("survey.pdf");
    let lines = ["Walrus survey one", "Seal count two", "Otter notes three"];
    survey(&mut app, &ctx, &lines, Some(path.clone()));
    let t = &mut app.tabs[0];
    let mut states = vec![snap(t)];
    // A note, a page turned, a page deleted, words redacted, a watermark.
    assert!(note(t, &ctx, 0, (300.0, 300.0), "Check the count"));
    states.push(snap(t));
    assert!(t.arrange(&ctx, &[1], |p| p.rotate_pages(&[1], 90)));
    states.push(snap(t));
    assert!(t.arrange(&ctx, &[2], |p| p.delete_pages(&[2])));
    states.push(snap(t));
    let walrus = t.pdf.find_text_boxes(0, "Walrus").unwrap()[0];
    assert!(t.edit_removing(&ctx, |p| p.redact(&[(0, walrus)]).map(drop)));
    assert!(!t.pdf.page_text(0).contains("Walrus"));
    assert!(t.scrubbed);
    states.push(snap(t));
    t.forms.wm.text = "DRAFT".into();
    t.forms.wm.range = "all".into();
    t.stamp_from_form(&ctx, Stamp::Watermark, false);
    assert!(t.error.is_none(), "{:?}", t.error);
    assert!(t.pdf.page_text(1).contains("DRAFT"));
    states.push(snap(t));
    assert_eq!(t.history().1, 5);
    // Back through each state, and forward again.
    for (n, state) in states.iter().enumerate().rev().skip(1) {
        t.undo(&ctx);
        assert!(snap(t) == *state, "Undo to state {n}");
    }
    assert!(!t.can_undo());
    assert!(!t.scrubbed, "the redaction undone");
    assert!(t.pdf.page_text(0).contains("Walrus"));
    for (n, state) in states.iter().enumerate().skip(1) {
        t.redo(&ctx);
        assert!(snap(t) == *state, "Redo to state {n}");
    }
    assert!(!t.can_redo());
    assert!(t.scrubbed, "the redaction redone");
    // Saved, the file reads as the tab, and holds nothing of the words redacted.
    app.act(&ctx, Act::Save);
    saves_done(&mut app, &ctx);
    assert!(matches!(app.dialog, Dialog::None));
    assert!(!app.tabs[0].dirty, "not saved");
    let saved = Pdf::open(&path, None).unwrap();
    assert!(
        reading(&saved) == reading(&app.tabs[0].pdf),
        "saved differently"
    );
    assert!(
        !file_holds(&path, "Walrus"),
        "the redacted words are in the file"
    );
    assert!(file_holds(&path, "Seal"));
    // Two undone, then a change: a new branch, with nothing to redo.
    let t = &mut app.tabs[0];
    t.undo(&ctx);
    t.undo(&ctx);
    assert!(snap(t) == states[3]);
    assert!(note(t, &ctx, 1, (100.0, 400.0), "A second look"));
    assert!(!t.can_redo(), "the undone changes are still there to redo");
    let branch = snap(t);
    assert!(branch != states[4] && branch != states[5]);
    t.undo(&ctx);
    assert!(snap(t) == states[3]);
    t.redo(&ctx);
    assert!(snap(t) == branch);
    // Closed by its button, the change since the save discarded.
    let title = app.tabs[0].title();
    close_by_button(&mut app, &ctx, &title);
    click(&mut app, &ctx, "Discard");
    settle(&mut app, &ctx);
    assert!(app.tabs.is_empty(), "the tab didn't close");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A watermark updated for other pages leaves the pages it no longer covers as they were
/// before it, and Undo and Redo step through each update's pages.
#[test]
fn a_watermark_updated_for_other_pages_leaves_the_rest_as_before() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let lines = ["Walrus one", "Walrus two", "Walrus three"];
    survey(&mut app, &ctx, &lines, None);
    let t = &mut app.tabs[0];
    let blank = snap(t);
    let marked = |t: &Tab| -> Vec<bool> {
        (0..3)
            .map(|p| t.pdf.page_text(p).contains("DRAFT"))
            .collect()
    };
    t.forms.wm.text = "DRAFT".into();
    t.forms.wm.range = "1-3".into();
    t.stamp_from_form(&ctx, Stamp::Watermark, false);
    assert_eq!(marked(t), [true, true, true]);
    t.forms.wm.range = "1".into();
    t.stamp_from_form(&ctx, Stamp::Watermark, true);
    assert_eq!(marked(t), [true, false, false]);
    t.forms.wm.range = "2".into();
    t.stamp_from_form(&ctx, Stamp::Watermark, true);
    assert_eq!(marked(t), [false, true, false]);
    assert!(t.linked(Stamp::Watermark).is_some());
    assert_eq!(t.history().1, 3);
    // Pages 1 and 3 are as they were before any watermark.
    let without = Pdf::from_bytes(&blank, None).unwrap();
    for p in [0, 2] {
        assert_eq!(t.pdf.page_text(p), without.page_text(p), "page {}", p + 1);
    }
    // Back through each update, the form following, and forward again.
    t.undo(&ctx);
    assert_eq!(marked(t), [true, false, false]);
    assert_eq!(t.forms.wm.range, "1");
    t.undo(&ctx);
    assert_eq!(marked(t), [true, true, true]);
    assert_eq!(t.forms.wm.range, "1-3");
    t.undo(&ctx);
    assert_eq!(marked(t), [false, false, false]);
    assert!(snap(t) == blank);
    t.redo(&ctx);
    assert_eq!(marked(t), [true, true, true]);
    t.redo(&ctx);
    assert_eq!(marked(t), [true, false, false]);
    t.redo(&ctx);
    assert_eq!(marked(t), [false, true, false]);
    assert!(t.linked(Stamp::Watermark).is_some());
    // Updated once more, for pages 1 and 3, after Undo and Redo.
    t.forms.wm.range = "1,3".into();
    t.stamp_from_form(&ctx, Stamp::Watermark, true);
    assert_eq!(marked(t), [true, false, true]);
}

/// A change that fails part way, or panics, leaves the document and its history as they
/// were, and says what went wrong.
#[test]
fn a_failed_change_leaves_the_document_and_its_history_as_they_were() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    pages_tab(&mut app, &ctx, 2);
    let t = &mut app.tabs[0];
    assert!(note(t, &ctx, 0, (60.0, 60.0), "Kept"));
    let (was, history, dirty) = (snap(t), t.history(), t.dirty);
    let failed = t.edit(&ctx, |p| {
        p.rotate_pages(&[0], 90)?;
        anyhow::bail!("the walrus refused")
    });
    assert!(!failed);
    assert!(snap(t) == was, "the failed change stayed");
    assert_eq!((t.history(), t.dirty), (history, dirty));
    let said = t.error.take().unwrap();
    assert!(said.contains("the walrus refused"), "{said}");
    let panicked = t.edit(&ctx, |p| -> limarepdf_core::Result<()> {
        p.insert_blank(1, 100.0, 100.0)?;
        panic!("the walrus panicked")
    });
    assert!(!panicked);
    assert!(snap(t) == was, "the change before the panic stayed");
    assert_eq!((t.history(), t.dirty), (history, dirty));
    assert!(t.error.take().is_some());
    // Undo still takes back the last change made.
    t.undo(&ctx);
    assert!(t.pdf.annotations().is_empty());
}

/// Words typed into a line again soon join one step: one Undo goes back to before the
/// first, and Redo forward to after the last.
#[test]
fn typing_joined_into_one_step_undoes_and_redoes_whole() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    survey(&mut app, &ctx, &["Walrus"], None);
    let t = &mut app.tabs[0];
    let before = snap(t);
    let line = |t: &Tab| {
        let objs = t.pdf.page_objects(0).unwrap();
        objs.into_iter().find(|o| o.text.is_some()).unwrap().id
    };
    for text in ["Walruses", "Walruses swim", "Walruses swim far"] {
        let id = line(t);
        t.ed.select_object(0, id);
        let te = TextEdit {
            text: text.into(),
            font: None,
            size: None,
            color: None,
            parts: Vec::new(),
        };
        t.ed.actions
            .push(EdAction::EditText(0, id, te, Vec::new(), true));
        t.apply_actions(&ctx);
    }
    assert_eq!(t.history().1, 1, "not one step");
    assert!(t.pdf.page_text(0).contains("Walruses swim far"));
    let after = snap(t);
    t.undo(&ctx);
    assert!(
        snap(t) == before,
        "one Undo didn't go back to before the first"
    );
    assert!(!t.can_undo());
    t.redo(&ctx);
    assert!(snap(t) == after, "Redo didn't go forward to after the last");
}
