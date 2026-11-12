//! Destinations reserved across tabs and jobs (S1-03: H-05, PB-86). A file a save is
//! writing is written by nothing else until that save ends: another tab's Save as, a
//! selection or the text saved there, a signed copy and a split's part are refused, by any
//! spelling of its name, and the save goes on as it would alone. However the save ends,
//! the name is free again (the open tab's own check aside). A split never writes over a
//! file open in another tab. The Digital IDs window's exports keep apart from the tabs'
//! files, and an ID's export from the files tabs save (PB-167).

use super::*;

/// The active tab saved as `path`, its save held before it writes until the sender sends
/// or is dropped.
fn held_save_as(app: &mut App, ctx: &egui::Context, path: &Path) -> Sender<()> {
    let (go, gate) = channel();
    app.save_gate = Some(gate);
    SAVE_AS.with(|s| s.set(Some(path.to_path_buf())));
    app.act(ctx, Act::SaveAs);
    go
}

/// The message shown says `name` is being saved from another tab.
fn refused_while_saved(app: &App, name: &str) {
    let (_, why) = message(app);
    let said = format!("\"{name}\" is being saved from another tab");
    assert!(why.contains(&said), "{why}");
}

/// Two unsaved tabs Save as one file while the first save is held: the second is refused
/// whatever the name's spelling, stays unsaved without a file, and the file is the first
/// tab's document alone once its save ends (H-05; the audit's gated repro).
#[test]
fn a_file_being_saved_is_refused_to_another_tab() {
    // The name said, and the path in the folder chosen.
    let spellings = [
        ("result.pdf", "result.pdf"),
        ("result.pdf", "sub/../result.pdf"),
        ("RESULT.pdf", "RESULT.pdf"),
    ];
    for (name, spelling) in spellings {
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("destination-race");
        std::fs::create_dir(dir.join("sub")).unwrap();
        let path = dir.join("result.pdf");
        unsaved_tab(&mut app, &ctx, None);
        let first = held_save_as(&mut app, &ctx, &path);
        assert_eq!(app.saves.len(), 1);
        let b = unsaved_tab(&mut app, &ctx, None);
        assert!(app.tabs[1].edit(&ctx, |p| p.insert_blank(2, 300.0, 500.0)));
        let second = held_save_as(&mut app, &ctx, &dir.join(spelling));
        assert_eq!(app.saves.len(), 1, "{name}: both saves were let through");
        refused_while_saved(&app, name);
        first.send(()).unwrap();
        saves_done(&mut app, &ctx);
        let written = pages_in(&path);
        assert_eq!(written, 2, "{name}: the other tab's document was written");
        assert!(!app.tabs[0].unsaved());
        let t = &app.tabs[1];
        let left = t.id == b && t.unsaved() && t.path.is_none();
        assert!(left, "{name}: marked saved");
        drop(second);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// Once a save has written its file the name is the tab's, refused by the open tab's
/// check; once that tab is closed, another tab saves there.
#[test]
fn a_file_is_free_again_once_its_save_ends() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("destination-written");
    let path = dir.join("result.pdf");
    unsaved_tab(&mut app, &ctx, None);
    held_save_as(&mut app, &ctx, &path).send(()).unwrap();
    saves_done(&mut app, &ctx);
    assert_eq!(app.tabs[0].path.as_deref(), Some(path.as_path()));
    unsaved_tab(&mut app, &ctx, None);
    assert!(app.tabs[1].edit(&ctx, |p| p.insert_blank(2, 300.0, 500.0)));
    SAVE_AS.with(|s| s.set(Some(path.clone())));
    app.act(&ctx, Act::SaveAs);
    let (_, why) = message(&app);
    let said = "\"result.pdf\" is open in another tab";
    assert!(why.contains(said), "{why}");
    app.dialog = Dialog::None;
    app.close_asking(0);
    assert_eq!(app.tabs.len(), 1, "a saved tab asked before closing");
    SAVE_AS.with(|s| s.set(Some(path.clone())));
    app.act(&ctx, Act::SaveAs);
    saves_done(&mut app, &ctx);
    assert_eq!(pages_in(&path), 3);
    assert!(!app.tabs[0].unsaved());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A save that fails, or isn't confirmed on disk, frees its file too.
#[test]
fn a_file_is_free_again_when_its_save_fails_or_is_not_confirmed() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("destination-failed");
    // A folder that is a file: nothing can be written in it.
    std::fs::write(dir.join("blocker"), "a file").unwrap();
    let blocked = dir.join("blocker/x.pdf");
    for _ in 0..2 {
        unsaved_tab(&mut app, &ctx, None);
        SAVE_AS.with(|s| s.set(Some(blocked.clone())));
        app.act(&ctx, Act::SaveAs);
        saves_done(&mut app, &ctx);
        assert_eq!(message(&app).0, "Save failed");
        app.dialog = Dialog::None;
    }

    let path = dir.join("result.pdf");
    limarepdf_core::sync_seam::fail_in(&dir);
    app.active = 0;
    SAVE_AS.with(|s| s.set(Some(path.clone())));
    app.act(&ctx, Act::SaveAs);
    saves_done(&mut app, &ctx);
    assert_eq!(message(&app).0, "Saved, not confirmed");
    app.dialog = Dialog::None;
    limarepdf_core::sync_seam::stop_failing_in(&dir);
    // Closed without saving again: the file stays, and no tab has it.
    app.close_asking(0);
    click(&mut app, &ctx, "Discard");
    assert_eq!(app.tabs.len(), 1);
    app.active = 0;
    assert!(app.tabs[0].edit(&ctx, |p| p.insert_blank(2, 300.0, 500.0)));
    SAVE_AS.with(|s| s.set(Some(path.clone())));
    app.act(&ctx, Act::SaveAs);
    saves_done(&mut app, &ctx);
    assert!(!matches!(app.dialog, Dialog::Message(..)), "refused");
    assert_eq!(pages_in(&path), 3);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// While a save is held, every other writer naming its file is refused, writing nothing:
/// the selection saved as a file, the text exported, a signed copy, and a split's part
/// (PB-86). The file is the saving tab's document once the save ends.
#[test]
fn every_writer_is_refused_a_file_being_saved() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("destination-writers");
    let path = dir.join("doc-001.pdf");
    unsaved_tab(&mut app, &ctx, None);
    let first = held_save_as(&mut app, &ctx, &path);
    text_tab(&mut app, &ctx, &["A walrus", "", ""]);
    let t = &mut app.tabs[1];
    t.ed.select_all_words(&t.pdf, 0);
    assert!(app.tabs[1].ed.selected_text().is_some());

    SAVE_AS.with(|s| s.set(Some(path.clone())));
    app.save_selection(dlg::Layout::TextFile);
    refused_while_saved(&app, "doc-001.pdf");
    app.dialog = Dialog::None;

    SAVE_AS.with(|s| s.set(Some(path.clone())));
    app.export_text();
    refused_while_saved(&app, "doc-001.pdf");
    app.dialog = Dialog::None;

    let t = &app.tabs[1];
    let place = SignPlace::NewBox {
        page: 0,
        rect: [72.0, 72.0, 272.0, 132.0],
    };
    let mut w = SignWindow::new(t.id, t.revision, place, "a box".into(), false, false);
    SAVE_AS.with(|s| s.set(Some(path.clone())));
    app.start_signing(&ctx, &mut w);
    let why = w.error.clone().unwrap_or_default();
    let said = "\"doc-001.pdf\" is being saved from another tab";
    assert!(why.contains(said), "{why}");
    assert!(!w.busy());

    let s = SplitFiles {
        by: SplitBy::Every(1),
        dir: dir.clone(),
        stem: "doc".into(),
    };
    app.split_into(&ctx, s);
    assert!(!app.busy, "the split started");
    refused_while_saved(&app, "doc-001.pdf");
    app.dialog = Dialog::None;

    first.send(()).unwrap();
    saves_done(&mut app, &ctx);
    assert_eq!(pages_in(&path), 2, "another writer's file");
    let other = dir.join("doc-002.pdf");
    assert!(!other.exists(), "the split wrote its other parts");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A split's parts are being written until its result arrives: a tab saving as one is
/// refused until then, and saves there after.
#[test]
fn a_jobs_files_are_free_again_once_it_ends() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("destination-job");
    let part = dir.join("doc-002.pdf");
    pages_tab(&mut app, &ctx, 3);
    let s = SplitFiles {
        by: SplitBy::Every(1),
        dir: dir.clone(),
        stem: "doc".into(),
    };
    app.split_into(&ctx, s);
    assert!(app.busy, "the split didn't start");
    // Its result isn't taken in until the next frame, written or not.
    unsaved_tab(&mut app, &ctx, None);
    SAVE_AS.with(|s| s.set(Some(dir.join("DOC-002.pdf"))));
    app.act(&ctx, Act::SaveAs);
    let (_, why) = message(&app);
    assert!(why.contains("\"DOC-002.pdf\" is being written"), "{why}");
    assert!(app.saves.is_empty());
    app.dialog = Dialog::None;
    wait_for(&mut app, &ctx, |a| !a.busy);
    assert_eq!(pages_in(&part), 1);
    SAVE_AS.with(|s| s.set(Some(part.clone())));
    app.act(&ctx, Act::SaveAs);
    saves_done(&mut app, &ctx);
    assert_eq!(pages_in(&part), 2, "refused once the split ended");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Signing's file is free again on the first frame the signing window isn't signing,
/// however it ended.
#[test]
fn signings_file_is_free_again_once_it_ends() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("destination-signing");
    let path = dir.join("signed.pdf");
    let tab = unsaved_tab(&mut app, &ctx, None);
    let claimed = [path.clone()];
    app.destinations.claim(&claimed, Owner::Signing);
    let (_, why) = app.conflict(&path, tab, true).expect("refused");
    assert!(why.contains("\"signed.pdf\" is being written"), "{why}");
    app.digital_windows(&ctx);
    assert!(app.conflict(&path, tab, true).is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The Digital IDs window's exports, from no tab, are refused a tab's file and a file being
/// saved; an ID's export is being written until it ends, so a tab saving as its file is
/// refused until then (PB-167).
#[test]
fn id_exports_and_the_tabs_files_keep_apart() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("destination-id-export");
    let open = dir.join("doc.pdf");
    form_file(&open, "Kept");
    open_and_wait(&mut app, &ctx, &open, 1);
    let why = app
        .export_conflict(&open)
        .expect("a tab's file let through");
    assert!(why.contains("\"doc.pdf\" is open in a tab"), "{why}");
    let saved = dir.join("saved.pdf");
    unsaved_tab(&mut app, &ctx, None);
    let go = held_save_as(&mut app, &ctx, &saved);
    let why = app
        .export_conflict(&saved)
        .expect("a file being saved let through");
    let said = "\"saved.pdf\" is being saved from a tab";
    assert!(why.contains(said), "{why}");
    drop(go);
    saves_done(&mut app, &ctx);

    let p12 = dir.join("jane.p12");
    let store = limarepdf_core::IdStore::new(dir.join("ids"));
    let (w, go) = IdsWindow::test_export(&ctx, store, p12.clone());
    app.ids_window = Some(w);
    ctx.enable_accesskit();
    window(&mut app, &ctx, 1400.0, vec![]);
    SAVE_AS.with(|s| s.set(Some(dir.join("JANE.p12"))));
    app.act(&ctx, Act::SaveAs);
    let (_, why) = message(&app);
    assert!(why.contains("\"JANE.p12\" is being written"), "{why}");
    assert!(app.saves.is_empty());
    app.dialog = Dialog::None;
    go.send(()).unwrap();
    for _ in 0..500 {
        window(&mut app, &ctx, 1400.0, vec![]);
        if !app.ids_window.as_ref().is_some_and(IdsWindow::busy) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let tab = app.tabs[app.active].id;
    let left = app.conflict(&p12, tab, false);
    assert!(left.is_none(), "claimed after the export");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A split whose part has the name of a file open in another tab is refused, before the
/// question about replacing files, and writes nothing (PB-86).
#[test]
fn a_split_never_writes_over_another_tabs_file() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("destination-split");
    let open = dir.join("doc-002.pdf");
    form_file(&open, "Kept");
    open_and_wait(&mut app, &ctx, &open, 1);
    pages_tab(&mut app, &ctx, 3);
    let s = SplitFiles {
        by: SplitBy::Every(1),
        dir: dir.clone(),
        stem: "doc".into(),
    };
    app.split_into(&ctx, s);
    assert!(!app.busy, "the split started");
    let (_, why) = message(&app);
    assert!(why.contains("\"doc-002.pdf\" is open in a tab"), "{why}");
    assert_eq!(field_in(&open, "Name"), "Kept");
    assert!(!dir.join("doc-001.pdf").exists(), "a part was written");
    std::fs::remove_dir_all(&dir).unwrap();
}
