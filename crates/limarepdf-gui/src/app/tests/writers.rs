//! Writers finish and report (S1-04). A save or a signed copy doesn't replace a file
//! another app changed while it was being written, and says so as before writing (PB-99).
//! A job whose tab was closed meanwhile still says what it wrote or what failed (PB-87).
//! Quitting waits for saves, jobs writing files and signing, and asks before stopping a job
//! that only changes a document (PB-88). A failed save says the Save chosen while it ran
//! wasn't done either (PB-89).

use super::*;
use limarepdf_core::commit_seam;

const PASSWORD: &str = "correct horse battery";

/// The bytes another app saves: a form whose Name is `name`, written in `scratch`.
fn other_apps_form(scratch: &Path, name: &str) -> Vec<u8> {
    let path = scratch.join("other-app.pdf");
    form_file(&path, name);
    std::fs::read(&path).unwrap()
}

/// A signing window for tab `tab`'s first page, with an ID of its own kept in `ids` and
/// its password typed.
fn sign_window(app: &App, tab: usize, ids: &Path) -> SignWindow {
    let store = limarepdf_core::IdStore::new(ids.to_path_buf());
    let new = limarepdf_core::NewId {
        name: "Jane Example".into(),
        ..Default::default()
    };
    store.create(&new, PASSWORD).unwrap();
    let t = &app.tabs[tab];
    let place = SignPlace::NewBox {
        page: 0,
        rect: [72.0, 72.0, 272.0, 132.0],
    };
    let mut w = SignWindow::new(t.id, t.revision, place, "a box".into(), false, false);
    w.test_store(store, PASSWORD);
    w
}

/// Signs as Sign… does, into `out`: the signing window is the app's, signing.
fn start_signing_into(app: &mut App, ctx: &egui::Context, mut w: SignWindow, out: &Path) {
    SAVE_AS.with(|s| s.set(Some(out.to_path_buf())));
    app.start_signing(ctx, &mut w);
    assert!(w.busy(), "not signing: {:?}", w.error);
    app.sign_window = Some(w);
}

/// The window drawn until signing has ended, for two minutes at most.
fn signing_ended(app: &mut App, ctx: &egui::Context) {
    ctx.enable_accesskit();
    for _ in 0..12_000 {
        window(app, ctx, 1400.0, vec![]);
        if !app.sign_window.as_ref().is_some_and(SignWindow::busy) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("signing never ended");
}

/// Waits for the jobs running to end, and takes in their results.
fn jobs_done(app: &mut App, ctx: &egui::Context) {
    wait_for(app, ctx, |a| a.jobs.is_empty());
}

/// The next job started is held before it begins until the sender sends or is dropped.
fn hold_next_job(app: &mut App) -> Sender<()> {
    let (go, gate) = channel();
    app.job_gate = Some(gate);
    go
}

/// The sentence a closed tab's job ends with (PB-87).
fn for_closed(name: &str) -> String {
    format!("(This was for \"{name}\", which was closed while it ran.)")
}

/// PB-99: another app saves the file just as Save is about to replace it. It isn't
/// replaced; Save asks, as when the change is found before saving, and the tab stays
/// unsaved, with Reload above the pages. The Save chosen meanwhile is answered by that
/// question. Replace then puts this document there.
#[test]
fn a_file_changed_while_it_was_saved_is_asked_about_not_replaced() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("writers-save");
    let path = dir.join("form.pdf");
    form_file(&path, "AlphaUnique");
    let theirs = other_apps_form(&test_dir("writers-save-scratch"), "Other app");
    open_and_wait(&mut app, &ctx, &path, 1);
    fill(&mut app, &ctx, "Notes", "Ours");
    let hold = commit_seam::hold(&dir);
    app.act(&ctx, Act::Save);
    assert_eq!(hold.arrived(), std::fs::canonicalize(&path).unwrap());
    app.act(&ctx, Act::Save);
    std::fs::write(&path, &theirs).unwrap();
    hold.release();
    saves_done(&mut app, &ctx);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        theirs,
        "their file was replaced"
    );
    assert!(
        matches!(app.dialog, Dialog::ConfirmStale(..)),
        "not asked about the change"
    );
    let t = &app.tabs[0];
    assert!(t.dirty, "marked saved");
    assert!(t
        .banners
        .iter()
        .any(|b| b.kind == BannerKind::ChangedOnDisk));
    assert!(
        app.after_saves.is_empty(),
        "the Save chosen meanwhile is run too"
    );
    drop(hold);

    click(&mut app, &ctx, "Replace");
    saves_done(&mut app, &ctx);
    assert_eq!(field_in(&path, "Notes"), "Ours");
    assert_eq!(field_in(&path, "Name"), "AlphaUnique");
    assert!(!app.tabs[0].dirty);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// PB-99 for a signed document, saved by adding the edits after what was signed: the
/// file another app saved meanwhile isn't added to or replaced.
#[test]
fn a_signed_file_changed_while_it_was_saved_is_not_replaced() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("writers-signed");
    let ids = test_dir("writers-signed-ids");
    let path = dir.join("signed.pdf");
    let store = limarepdf_core::IdStore::new(ids.join("ids"));
    let new = limarepdf_core::NewId {
        name: "Jane Example".into(),
        ..Default::default()
    };
    let id = store.create(&new, PASSWORD).unwrap().id;
    let unlocked = store.unlock(&id, PASSWORD).unwrap();
    let mut doc = Pdf::from_text("Hello", (612.0, 792.0)).unwrap();
    let request = limarepdf_core::SignRequest {
        id: &unlocked,
        place: SignPlace::NewBox {
            page: 0,
            rect: [72.0, 72.0, 272.0, 132.0],
        },
        reason: None,
        location: None,
        certify: None,
        art: None,
        accept_disallowed: false,
    };
    let signed = limarepdf_core::sign_document(&doc.to_bytes().unwrap(), None, &request).unwrap();
    std::fs::write(&path, &signed.bytes).unwrap();
    open_and_wait(&mut app, &ctx, &path, 1);
    let t = &mut app.tabs[0];
    assert!(t.signed, "not opened as signed");
    let note = |p: &mut Pdf| p.add_note(0, (40.0, 40.0), [1.0, 0.8, 0.0], "A comment", "Jane");
    assert!(t.edit(&ctx, |p| note(p).map(|_| ())));
    let theirs = b"%PDF-1.7\n% another app's version of the file\n".to_vec();
    let hold = commit_seam::hold(&dir);
    app.act(&ctx, Act::Save);
    hold.arrived();
    std::fs::write(&path, &theirs).unwrap();
    hold.release();
    saves_done(&mut app, &ctx);
    drop(hold);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        theirs,
        "their file was replaced"
    );
    assert!(matches!(app.dialog, Dialog::ConfirmStale(..)), "not asked");
    assert!(app.tabs[0].dirty, "marked saved");
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&ids).unwrap();
}

/// PB-99 for signing over the tab's own file: another app saving it meanwhile, the signed
/// copy isn't put in its place, and the signing window says why.
#[test]
fn a_file_changed_while_it_was_signed_over_is_not_replaced() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("writers-sign");
    let ids = test_dir("writers-sign-ids");
    let path = dir.join("form.pdf");
    form_file(&path, "AlphaUnique");
    let theirs = other_apps_form(&ids, "Other app");
    open_and_wait(&mut app, &ctx, &path, 1);
    let w = sign_window(&app, 0, &ids.join("ids"));
    let hold = commit_seam::hold(&dir);
    start_signing_into(&mut app, &ctx, w, &path);
    hold.arrived();
    std::fs::write(&path, &theirs).unwrap();
    hold.release();
    signing_ended(&mut app, &ctx);
    drop(hold);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        theirs,
        "their file was replaced"
    );
    let w = app.sign_window.as_ref().expect("the signing window closed");
    let error = w.error.clone().unwrap_or_default();
    let said = "Another app or tab changed this file while it was being signed";
    assert!(error.starts_with(said), "{error}");
    assert_eq!(app.tabs.len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&ids).unwrap();
}

/// PB-87: a job whose tab was closed while it ran still says what it wrote, or what
/// failed, naming the document; one that changes the document says its result was
/// discarded.
#[test]
fn a_closed_tabs_job_still_says_how_it_ended() {
    let ctx = egui::Context::default();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("writers-closed");
    // A folder that is a file: nothing can be written in it.
    std::fs::write(dir.join("blocker"), "a file").unwrap();
    std::fs::create_dir(dir.join("parts")).unwrap();
    for (into, fails) in [(dir.join("parts"), false), (dir.join("blocker"), true)] {
        pages_tab(&mut app, &ctx, 2);
        let name = app.tabs[0].name();
        let go = hold_next_job(&mut app);
        let s = SplitFiles {
            by: SplitBy::Every(1),
            dir: into.clone(),
            stem: "doc".into(),
        };
        app.split_into(&ctx, s);
        assert_eq!(app.jobs.len(), 1, "the split didn't start");
        app.close_tab(0);
        go.send(()).unwrap();
        jobs_done(&mut app, &ctx);
        let (title, text) = message(&app);
        assert_eq!(title, "Split into files");
        assert!(text.ends_with(&for_closed(&name)), "{text}");
        let wrote = format!("Wrote 2 files to {}.", into.display());
        assert_eq!(text.starts_with(&wrote), !fails, "{text}");
        assert_eq!(into.join("doc-002.pdf").exists(), !fails, "{text}");
        app.dialog = Dialog::None;
    }

    text_tab(&mut app, &ctx, &["A walrus", ""]);
    let name = app.tabs[0].name();
    let go = hold_next_job(&mut app);
    app.act(&ctx, Act::RemoveBlank);
    assert_eq!(app.jobs.len(), 1, "Remove blank pages didn't start");
    app.close_tab(0);
    go.send(()).unwrap();
    jobs_done(&mut app, &ctx);
    let discarded = format!("\"{name}\" was closed while this ran, so its result was discarded.");
    assert_eq!(message(&app), ("Remove blank pages", discarded.as_str()));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// PB-88: quitting, by the menu or by closing the window, while a split writes its files
/// waits for it, saying so in the status bar; every part is written, the split's message
/// shows, and quitting goes on after it.
#[test]
fn quitting_waits_for_a_job_writing_files() {
    for by_window in [false, true] {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.enable_accesskit();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("writers-quit-split");
        pages_tab(&mut app, &ctx, 3);
        let go = hold_next_job(&mut app);
        let s = SplitFiles {
            by: SplitBy::Every(1),
            dir: dir.clone(),
            stem: "doc".into(),
        };
        app.split_into(&ctx, s);
        assert_eq!(app.jobs.len(), 1, "the split didn't start");
        if by_window {
            close_window(&mut app, &ctx);
        } else {
            app.act(&ctx, Act::Quit);
        }
        assert!(!app.quit_ok, "quit while the split was writing");
        let waiting = "Quitting when Split into files finishes…";
        assert_eq!(app.quit_sign().as_deref(), Some(waiting));
        assert!(shows(&settle(&mut app, &ctx), waiting), "not said");
        assert!(!app.quit_ok);
        go.send(()).unwrap();
        jobs_done(&mut app, &ctx);
        let parts = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(parts, 3, "parts missing");
        let (title, text) = message(&app);
        assert_eq!(title, "Split into files");
        assert!(text.starts_with("Wrote 3 files"), "{text}");
        assert!(!app.quit_ok, "quit before its message was read");
        click(&mut app, &ctx, "OK");
        assert!(app.quit_ok, "didn't go on quitting");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// PB-88: quitting while a signed copy is being written waits for it too.
#[test]
fn quitting_waits_for_signing() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("writers-quit-sign");
    let ids = test_dir("writers-quit-sign-ids");
    pages_tab(&mut app, &ctx, 1);
    let out = dir.join("signed.pdf");
    let w = sign_window(&app, 0, &ids.join("ids"));
    let hold = commit_seam::hold(&dir);
    start_signing_into(&mut app, &ctx, w, &out);
    hold.arrived();
    app.act(&ctx, Act::Quit);
    assert!(!app.quit_ok, "quit while signing");
    let waiting = "Quitting when signing finishes…";
    assert_eq!(app.quit_sign().as_deref(), Some(waiting));
    hold.release();
    signing_ended(&mut app, &ctx);
    drop(hold);
    assert!(out.exists(), "the signed copy wasn't written");
    assert_eq!(message(&app).0, "Signed");
    assert!(!app.quit_ok);
    click(&mut app, &ctx, "OK");
    assert!(app.quit_ok, "didn't go on quitting");
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&ids).unwrap();
}

/// PB-88: quitting while jobs that only change a document run asks before stopping
/// them. Cancel keeps the app open; Quit now goes on, asking about unsaved work.
#[test]
fn quitting_asks_before_stopping_a_job() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    text_tab(&mut app, &ctx, &["A walrus", ""]);
    let t = app.tabs.last_mut().unwrap();
    assert!(t.edit(&ctx, |p| p.insert_blank(2, 300.0, 500.0)));
    let name = t.name();
    let first = hold_next_job(&mut app);
    app.act(&ctx, Act::RemoveBlank);
    app.act(&ctx, Act::Quit);
    let one = format!("Remove blank pages is still running for \"{name}\". Quit now and stop it?");
    assert!(
        matches!(&app.dialog, Dialog::ConfirmStopTasks(text, false) if *text == one),
        "not asked"
    );
    click(&mut app, &ctx, "Cancel");
    assert!(!app.quit_ok && matches!(app.dialog, Dialog::None));
    assert_eq!(app.jobs.len(), 1, "the job was stopped");

    text_tab(&mut app, &ctx, &["A seal", ""]);
    let second = hold_next_job(&mut app);
    app.act(&ctx, Act::RemoveBlank);
    app.act(&ctx, Act::Quit);
    let two = "2 tasks are still running. Quit now and stop them?";
    assert!(
        matches!(&app.dialog, Dialog::ConfirmStopTasks(text, false) if text == two),
        "not asked"
    );
    click(&mut app, &ctx, "Quit now");
    assert!(
        !app.quit_ok,
        "quit without asking about the unsaved document"
    );
    assert!(matches!(app.dialog, Dialog::ConfirmClose(_, true)));
    click(&mut app, &ctx, "Discard");
    assert!(app.quit_ok, "didn't quit");
    drop((first, second));
}

/// PB-89: a save that fails says that the Save (or Save as) chosen while it ran wasn't
/// done either, and nothing more is saved.
#[test]
fn a_failed_save_says_the_save_chosen_meanwhile_was_not_done() {
    for (again, said) in [(Act::Save, "Save"), (Act::SaveAs, "Save as")] {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        let dir = test_dir("writers-again");
        std::fs::write(dir.join("blocker"), "a file").unwrap();
        unsaved_tab(&mut app, &ctx, Some(dir.join("blocker").join("x.pdf")));
        let (go, gate) = channel();
        app.save_gate = Some(gate);
        app.act(&ctx, Act::Save);
        assert_eq!(app.saves.len(), 1);
        app.act(&ctx, again);
        go.send(()).unwrap();
        saves_done(&mut app, &ctx);
        let (title, text) = message(&app);
        assert_eq!(title, "Save failed");
        let meanwhile = format!(
            "You chose {said} again while it was saving; that wasn't done either. Choose \
             Save again, or Save as to save somewhere else."
        );
        assert!(text.ends_with(&meanwhile), "{text}");
        click(&mut app, &ctx, "OK");
        assert!(
            app.saves.is_empty() && app.after_saves.is_empty(),
            "saved again"
        );
        assert!(app.tabs[0].dirty);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
