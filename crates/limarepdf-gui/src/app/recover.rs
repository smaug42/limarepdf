//! The app's side of crash recovery (`limarepdf-recovery-design.md`, stage R2): starting
//! this run's session, writing each tab's copy when it is due, withdrawing a tab's copies
//! before a redaction, a removal of hidden information or a password takes effect, the
//! restore window at start, the setting, and ending the session at a normal exit. The store
//! and its thread are `crate::recovery`.

use super::{App, Dialog};
use crate::banners::{self, Kind as BannerKind};
use crate::dialogs::{self as dlg, Offer, RestoreOut};
use crate::recovery::schedule::{Next, INTERVAL};
use crate::recovery::{
    self, Clock, Crashed, DocInfo, Found, Fs, NotConfirmed, Outcome, Queue, RealFs, Store, TabId,
    Withdrawal,
};
use crate::tab::{Recovered, Tab};
use eframe::egui;
use limarepdf_core::{display_line, local_date, local_time, Pdf};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// A session found at start that ended without cleaning up, while its copies are offered
/// or restored. Its folder stays locked meanwhile.
pub(super) struct Source {
    crashed: Crashed,
    /// Copies restored into tabs not yet saved or closed; deleted when they are.
    restored: Vec<TabId>,
    /// Copies that turned out not to open, with why.
    broken: HashMap<TabId, String>,
}

#[derive(Default)]
pub(super) struct Rescue {
    /// The recovery thread; `None` when recovery is off for this run.
    queue: Option<Queue>,
    /// Sessions with copies offered or restored.
    found: Vec<Source>,
    /// The restore window is open.
    asking: bool,
    /// Files from the command line, opened once the restore window closes.
    startup: Option<(Vec<PathBuf>, Option<usize>)>,
    /// When files withdrawals left were last tried again.
    retried: Option<Instant>,
}

/// The notice when recovery can't start (section 6).
fn off_notice(why: &str) -> Dialog {
    Dialog::Message(
        "Recovery copies are off".into(),
        format!(
            "LimarePDF can't keep recovery copies in this session: {}. Everything else works \
             as usual, but if LimarePDF stops without closing normally, unsaved changes can't \
             be offered back. Save your work as you go.",
            display_line(why, 300)
        ),
    )
}

/// What a tab's copy says about its document.
fn doc_info(t: &Tab) -> DocInfo {
    DocInfo {
        title: t.name(),
        original: t
            .path
            .clone()
            .or_else(|| t.recovered.as_ref().and_then(|r| r.original.clone())),
        pages: t.page_count(),
        repaired: t.repaired,
        signed: t.signed,
        scrubbed: t.scrubbed,
    }
}

/// Files a withdrawal invalidated but couldn't delete: the tab's notice names them until
/// they are gone.
fn left(t: &mut Tab, files: Vec<PathBuf>) {
    for f in files {
        // A withdrawal tried again names the files it still couldn't delete.
        if !t.copy.leftovers.contains(&f) {
            t.copy.leftovers.push(f);
        }
    }
    if let Some(f) = t.copy.leftovers.first() {
        t.show_banner(banners::leftover_copy(f));
    }
}

/// The dialog when a command stops because an earlier copy could still be offered
/// (section 6); `done` is what didn't happen, as "applied the redactions". `ours` when the
/// copy is this session's, which turning recovery copies off also withdraws (a restored
/// tab's copy in a crashed session isn't).
fn stopped(n: NotConfirmed, done: &str, ours: bool) -> Dialog {
    let title = "Recovery copy not removed".to_string();
    let or_off = if ours {
        ", or turn off recovery copies (File > Keep recovery copies of unsaved changes)"
    } else {
        ""
    };
    match n {
        NotConfirmed::Kept { file, why } => Dialog::RecoveryFile {
            title,
            text: format!(
                "LimarePDF couldn't remove an earlier recovery copy of this document ({}), so \
                 it hasn't {done}. Otherwise that copy, made before the change, could be offered \
                 after a crash.\n\nThe copy is at {}. Delete it{or_off}, then try again.",
                display_line(&why, 300),
                file.display()
            ),
            file,
        },
        NotConfirmed::SyncFailed { folder, why } => Dialog::RecoveryFile {
            title,
            text: format!(
                "LimarePDF removed the earlier recovery copies of this document, but the disk \
                 can't confirm the change ({}), so it hasn't {done}: after a power cut, a copy \
                 made before the change could come back and be offered after a crash.\n\nThe \
                 recovery folder is {}, and its disk is failing to confirm changes. Save the \
                 document to another drive, and check the disk.",
                display_line(&why, 300),
                folder.display()
            ),
            file: folder,
        },
    }
}

/// How the restore window shows a copy.
fn offer(d: &Found, broken: Option<&String>, keep_until: SystemTime) -> Offer {
    let keep = format!("Kept until {}.", local_date(keep_until));
    match (&d.state, broken) {
        (Ok(desc), None) => {
            let from = desc.info.original.as_ref().map_or("Untitled".into(), |p| {
                display_line(&p.to_string_lossy(), 200)
            });
            let pages = match desc.info.pages {
                1 => "1 page".to_string(),
                n => format!("{n} pages"),
            };
            Offer {
                title: Some(display_line(&desc.info.title, 120)),
                detail: format!(
                    "{from}\nCopy made {}, {pages}. {keep}",
                    local_time(desc.time)
                ),
            }
        }
        (Err(why), _) | (_, Some(why)) => Offer {
            title: None,
            detail: format!("It can't be restored: {why}. {keep}"),
        },
    }
}

/// Shows `path` in the system's file manager: a folder itself; for a file, its folder on
/// Linux and the file selected on macOS and Windows.
fn show_file(path: &Path) -> std::io::Result<()> {
    use std::process::Command;
    let folder = path.is_dir();
    let mut cmd;
    if cfg!(target_os = "macos") {
        cmd = Command::new("open");
        if !folder {
            cmd.arg("-R");
        }
        cmd.arg(path);
    } else if cfg!(windows) {
        cmd = Command::new("explorer");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            if folder {
                cmd.arg(path);
            } else {
                // Explorer reads its own quoting, which `arg` doesn't give.
                cmd.raw_arg(format!("/select,\"{}\"", path.display()));
            }
        }
    } else {
        cmd = Command::new("xdg-open");
        cmd.arg(if folder {
            path
        } else {
            path.parent().unwrap_or(path)
        });
    }
    let mut child = cmd.spawn()?;
    // Collected when it ends, so it isn't left behind as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}

impl App {
    /// At start: finds copies left by a LimarePDF that stopped without closing normally and
    /// offers them before opening `files` from the command line, and starts this run's
    /// session. Recovery that can't start is off for the run, with one notice.
    pub(super) fn start_recovery(
        &mut self,
        ctx: &egui::Context,
        files: Vec<PathBuf>,
        page: Option<usize>,
    ) {
        match recovery::recovery_root() {
            Some(root) => {
                self.start_recovery_in(ctx, Arc::new(RealFs), &root, recovery::system_clock())
            }
            None if self.settings.recovery => {
                self.dialog = off_notice("there is no folder for them (HOME isn't set)")
            }
            None => {}
        }
        if self.rescue.asking {
            self.rescue.startup = Some((files, page));
        } else {
            self.open_startup(ctx, files, page);
        }
    }

    pub(super) fn start_recovery_in(
        &mut self,
        ctx: &egui::Context,
        fs: Arc<dyn Fs>,
        root: &Path,
        clock: Clock,
    ) {
        self.rescue.found = recovery::find_crashed(&fs, root, clock())
            .into_iter()
            .map(|crashed| Source {
                crashed,
                restored: Vec::new(),
                broken: HashMap::new(),
            })
            .collect();
        self.rescue.asking = !self.rescue.found.is_empty();
        match Store::start(fs, root, clock) {
            Ok(store) => {
                if !self.settings.recovery {
                    store.turn_off();
                }
                let ctx = ctx.clone();
                let wake = Box::new(move || ctx.request_repaint());
                self.rescue.queue = Some(Queue::new(Arc::new(store), wake));
            }
            Err(e) if self.settings.recovery => self.dialog = off_notice(&e),
            Err(_) => {}
        }
    }

    /// Every frame: takes the recovery thread's reports and queues a copy of each tab whose
    /// copy is due (section 4).
    pub(super) fn recovery_tick(&mut self, ctx: &egui::Context) {
        self.recovery_tick_at(ctx, Instant::now());
    }

    fn recovery_tick_at(&mut self, ctx: &egui::Context, now: Instant) {
        let Some(q) = &self.rescue.queue else {
            return;
        };
        for r in q.reports() {
            let Some(t) = self.tabs.iter_mut().find(|t| t.id == r.tab.0) else {
                continue;
            };
            match r.outcome {
                Outcome::Written(revision) => {
                    t.copy.written(revision);
                    t.drop_banner(BannerKind::NoCopies);
                }
                Outcome::Refused => t.copy.not_written(),
                Outcome::Failed(why) => {
                    t.copy.not_written();
                    // Told once, not at every try.
                    if !t.copy.told {
                        t.copy.told = true;
                        t.show_banner(banners::no_copies(&why));
                    }
                }
                Outcome::Removed => t.copy.removed(),
                Outcome::RemoveFailed(why) => {
                    self.status = format!("Couldn't delete a recovery copy: {why}")
                }
            }
        }

        // Files withdrawals left: tried again each minute, and their notices taken down
        // once they are gone.
        let minute = self.rescue.retried.is_none_or(|r| now >= r + INTERVAL);
        if minute && self.tabs.iter().any(|t| !t.copy.leftovers.is_empty()) {
            for s in &mut self.rescue.found {
                s.crashed.retry_leftovers();
            }
            for t in &mut self.tabs {
                t.copy.leftovers.retain(|p| q.store().exists(p));
                if t.copy.leftovers.is_empty() {
                    t.drop_banner(BannerKind::LeftoverCopy);
                }
            }
            q.retry();
            self.rescue.retried = Some(now);
        }

        if !q.store().is_on() {
            return;
        }
        let mut wait: Option<Duration> = None;
        for t in &mut self.tabs {
            // A broken tab's state can't be trusted; its last copy stays.
            if t.broken.is_some() {
                continue;
            }
            t.copy.protected |= t.encryption.is_some() || t.pdf.encrypted();
            match t.copy.next(now, t.revision, t.dirty) {
                Next::Idle => {}
                Next::After(d) => wait = Some(wait.map_or(d, |w| w.min(d))),
                Next::Now => {
                    // No ticket: recovery is blocked for the tab until a withdrawal is
                    // confirmed, or for the session (section 6).
                    let Some(ticket) = q.store().ticket(TabId(t.id), t.revision) else {
                        t.copy.tried(now);
                        continue;
                    };
                    // Taken here, on the UI thread, so that no clone falls between a
                    // withdrawal and the change it is for (section 4). Pruned, so that
                    // nothing a redaction or a removal of hidden information took out is
                    // left in it.
                    match crate::guard(|| Ok(t.pdf.clone())) {
                        Ok(mut pdf) => {
                            q.write(ticket, doc_info(t), Box::new(move || pdf.pruned_snapshot()));
                            t.copy.queued(now);
                        }
                        Err(e) => {
                            t.copy.tried(now);
                            if !t.copy.told {
                                t.copy.told = true;
                                t.show_banner(banners::no_copies(&format!("{e:#}")));
                            }
                        }
                    }
                }
            }
        }
        if let Some(d) = wait {
            ctx.request_repaint_after(d);
        }
    }

    /// Withdraws tab `i`'s recovery copies before a redaction, a removal of hidden
    /// information (or a redo of either) or a password takes effect; `done` says what, for
    /// the message. `Err`
    /// is the dialog to show instead of running the command: an earlier copy could still
    /// be offered (section 6). A copy invalidated but left on disk gets the tab's notice,
    /// and the command goes ahead.
    pub(super) fn withdraw_copies(&mut self, i: usize, done: &str) -> Result<(), Dialog> {
        let Some(t) = self.tabs.get_mut(i) else {
            return Ok(());
        };
        // A restored tab's copy in the crashed session was made before the change too,
        // and is withdrawn whether or not this session keeps copies.
        if let Some(r) = &t.recovered {
            let src = self.rescue.found.iter_mut();
            if let Some(src) = src.into_iter().find(|s| s.crashed.dir == r.session) {
                match src.crashed.withdraw(r.tab) {
                    Withdrawal::Confirmed => {}
                    Withdrawal::Leftover(files) => left(t, files),
                    Withdrawal::NotConfirmed(n) => return Err(stopped(n, done, false)),
                }
            }
        }
        let Some(q) = &self.rescue.queue else {
            return Ok(());
        };
        t.copy.withdrawn();
        // A tab never written has nothing to withdraw; a protected one had its copies
        // withdrawn when it became protected.
        if !t.copy.asked || t.copy.protected {
            return Ok(());
        }
        match q.store().withdraw(TabId(t.id)) {
            Withdrawal::Confirmed => Ok(()),
            Withdrawal::Leftover(files) => {
                left(t, files);
                Ok(())
            }
            Withdrawal::NotConfirmed(n) => Err(stopped(n, done, true)),
        }
    }

    /// File > Keep recovery copies of unsaved changes. Turning it off always takes effect
    /// for new copies; copies it can't withdraw are reported (section 6).
    pub(super) fn toggle_recovery(&mut self) {
        let on = !self.settings.recovery;
        self.settings.recovery = on;
        self.settings.save();
        let Some(q) = &self.rescue.queue else {
            return;
        };
        if on {
            q.store().turn_on();
            return;
        }
        let mut kept = Vec::new();
        for (tab, w) in q.store().turn_off() {
            let t = self.tabs.iter_mut().find(|t| t.id == tab.0);
            match (w, t) {
                (Withdrawal::Leftover(files), Some(t)) => left(t, files),
                (Withdrawal::NotConfirmed(NotConfirmed::Kept { file, .. }), _)
                | (Withdrawal::NotConfirmed(NotConfirmed::SyncFailed { folder: file, .. }), _) => {
                    kept.push(file)
                }
                _ => {}
            }
        }
        for t in &mut self.tabs {
            t.copy.withdrawn();
            t.copy.told = false;
            t.drop_banner(BannerKind::NoCopies);
        }
        kept.dedup();
        if let Some(first) = kept.first().cloned() {
            let list: Vec<String> = kept.iter().map(|p| p.display().to_string()).collect();
            self.dialog = Dialog::RecoveryFile {
                title: "Recovery copy not removed".into(),
                text: format!(
                    "Recovery copies are off, and no new ones will be made. But LimarePDF \
                     couldn't remove all the earlier ones, so they could still be offered after \
                     a crash:\n\n{}\n\nDelete them yourself, or LimarePDF tries again the next \
                     time it starts.",
                    list.join("\n")
                ),
                file: first,
            };
        }
    }

    /// Shows the file the active tab's leftover notice names.
    pub(super) fn show_leftover(&mut self) {
        let file = self
            .tabs
            .get(self.active)
            .and_then(|t| t.copy.leftovers.first().cloned());
        if let Some(f) = file {
            self.show_file(&f);
        }
    }

    pub(super) fn show_file(&mut self, path: &Path) {
        if let Err(e) = show_file(path) {
            self.status = format!("Couldn't show {}: {e}", path.display());
        }
    }

    /// After tab `i` was saved: its recovery copy goes, and a restored tab's copy in the
    /// crashed session too (section 7).
    pub(super) fn saved_copy(&mut self, i: usize) {
        let Some(t) = self.tabs.get_mut(i) else {
            return;
        };
        if let Some(q) = &self.rescue.queue {
            if t.copy.asked {
                q.remove(TabId(t.id));
            }
        }
        t.copy.told = false;
        t.drop_banner(BannerKind::NoCopies);
        if let Some(r) = t.recovered.take() {
            t.drop_banner(BannerKind::Recovered);
            self.release(&r);
        }
    }

    /// After a tab was closed (or replaced by its signed version): the same.
    pub(super) fn closed_copy(&mut self, t: &Tab) {
        if let Some(q) = &self.rescue.queue {
            if t.copy.asked {
                q.remove(TabId(t.id));
            }
        }
        if let Some(r) = &t.recovered {
            self.release(r);
        }
    }

    /// Deletes the copy a restored tab came from, and lets its session go when nothing in
    /// it is restored or offered any more.
    fn release(&mut self, r: &Recovered) {
        let Some(s) = self
            .rescue
            .found
            .iter()
            .position(|s| s.crashed.dir == r.session)
        else {
            return;
        };
        let src = &mut self.rescue.found[s];
        src.restored.retain(|t| *t != r.tab);
        if let Some(i) = src.crashed.docs.iter().position(|d| d.tab == r.tab) {
            if let Err(e) = src.crashed.discard(i) {
                self.status = format!("Couldn't delete a recovery copy: {e}");
            }
        }
        if !self.rescue.asking && src.restored.is_empty() {
            self.rescue.found.remove(s);
        }
    }

    /// The restore window, while open (section 7).
    pub(super) fn restore_window(&mut self, ctx: &egui::Context) {
        if !self.rescue.asking {
            return;
        }
        let mut rows = Vec::new();
        let mut offers = Vec::new();
        for (s, src) in self.rescue.found.iter().enumerate() {
            for d in &src.crashed.docs {
                if !src.restored.contains(&d.tab) {
                    rows.push((s, d.tab));
                    offers.push(offer(d, src.broken.get(&d.tab), src.crashed.keep_until));
                }
            }
        }
        if offers.is_empty() {
            return self.close_restore(ctx);
        }
        let (show, i) = match dlg::restore(ctx, &offers) {
            RestoreOut::Open => return,
            RestoreOut::NotNow => return self.close_restore(ctx),
            RestoreOut::Restore(i) => return self.restore_copy(ctx, rows[i].0, rows[i].1),
            RestoreOut::ShowFile(i) => (true, i),
            RestoreOut::Discard(i) => (false, i),
        };
        let (s, tab) = rows[i];
        let crashed = &mut self.rescue.found[s].crashed;
        let Some(i) = crashed.docs.iter().position(|d| d.tab == tab) else {
            return;
        };
        if show {
            if let Some(f) = crashed.file(i) {
                self.show_file(&f);
            }
        } else if let Err(e) = crashed.discard(i) {
            self.status = format!("Couldn't delete a recovery copy: {e}");
        }
    }

    /// Opens copy `tab` of session `s` as a new, unsaved tab (section 7).
    fn restore_copy(&mut self, ctx: &egui::Context, s: usize, tab: TabId) {
        let src = &mut self.rescue.found[s];
        let Some(i) = src.crashed.docs.iter().position(|d| d.tab == tab) else {
            return;
        };
        let Ok(desc) = src.crashed.docs[i].state.clone() else {
            return;
        };
        let opened = src.crashed.read(i).and_then(|bytes| {
            crate::guard(|| Pdf::from_bytes(&bytes, None))
                .map_err(|e| format!("it doesn't open ({e:#})"))
        });
        let pdf = match opened {
            Ok(p) => p,
            Err(why) => {
                src.broken.insert(tab, why);
                return;
            }
        };
        let session = src.crashed.dir.clone();
        if !self.add_tab(ctx, pdf, None, None, true) {
            return;
        }
        self.rescue.found[s].restored.push(tab);
        let t = &mut self.tabs[self.active];
        let name = desc
            .info
            .original
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| desc.info.title.clone());
        let name = name.strip_suffix(" (recovered)").unwrap_or(&name);
        t.label = Some(format!("{} (recovered)", display_line(name, 120)));
        // Its copy is the one it was restored from, which stays until it is saved or
        // closed.
        t.copy.copied = Some(t.revision);
        let signed = t.pdf.signature_count() > 0;
        t.show_banner(banners::recovered(&local_time(desc.time), signed));
        t.recovered = Some(Recovered {
            session,
            tab,
            original: desc.info.original,
        });
    }

    /// The restore window closed: copies not restored stay for the next start, and the
    /// files from the command line open now.
    fn close_restore(&mut self, ctx: &egui::Context) {
        self.rescue.asking = false;
        self.rescue.found.retain(|s| !s.restored.is_empty());
        if let Some((files, page)) = self.rescue.startup.take() {
            self.open_startup(ctx, files, page);
        }
    }

    /// At a normal exit: this session's copies and folder go, and so do the copies restored
    /// tabs came from (quitting closes them). Copies only offered stay for the next start.
    pub(super) fn end_recovery(&mut self) {
        self.rescue.asking = false;
        let restored: Vec<Recovered> = self
            .tabs
            .iter()
            .filter_map(|t| t.recovered.clone())
            .collect();
        for r in &restored {
            self.release(r);
        }
        self.rescue.found.clear();
        if let Some(q) = self.rescue.queue.take() {
            if let Err(e) = q.finish().end() {
                eprintln!("limarepdf: {e}");
            }
        }
    }
}

/// The app's recovery tests (the design's section 9, stage R2), on the store's model disk.
#[cfg(test)]
mod tests {
    use super::super::RECOVERED;
    use super::*;
    use crate::actions::Act;
    use crate::editor::Tool;
    use crate::inspector::{Apply, Sec};
    use crate::recent::Recent;
    use crate::recovery::model::{Faulty, Kind, Model};
    use crate::settings::Settings;
    use crate::signatures;
    use limarepdf_core::{Cipher, Encryption, Hidden, Restrictions};
    use std::time::UNIX_EPOCH;

    const T0: u64 = 1_800_000_000;

    fn root() -> PathBuf {
        PathBuf::from("/state/limarepdf/recovery")
    }

    fn clock() -> Clock {
        Arc::new(|| UNIX_EPOCH + Duration::from_secs(T0))
    }

    fn page() -> Pdf {
        let mut p = Pdf::new();
        p.insert_blank(0, 612.0, 792.0).unwrap();
        p
    }

    /// Adds a tab of one blank page with an unsaved change (a second page); its index.
    fn add_changed(app: &mut App, ctx: &egui::Context) -> usize {
        assert!(app.add_tab(ctx, page(), None, None, false));
        let t = app.tabs.last_mut().unwrap();
        assert!(t.edit(ctx, |p| p.insert_blank(1, 612.0, 792.0)));
        app.tabs.len() - 1
    }

    /// An app keeping its recovery copies on `fs`, with one changed tab. Nothing is read
    /// from or written to the user's settings.
    fn app_on(ctx: &egui::Context, fs: Arc<dyn Fs>) -> App {
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        app.start_recovery_in(ctx, fs, &root(), clock());
        add_changed(&mut app, ctx);
        app
    }

    fn queue(app: &App) -> &Queue {
        app.rescue.queue.as_ref().expect("recovery started")
    }

    /// Runs the schedule at `at`, waits for the recovery thread and takes its reports.
    fn tick(app: &mut App, ctx: &egui::Context, at: Instant) {
        app.recovery_tick_at(ctx, at);
        while !queue(app).idle() {
            std::thread::sleep(Duration::from_millis(1));
        }
        app.recovery_tick_at(ctx, at);
    }

    /// Lets a minute pass from `t0`, so every changed tab is copied.
    fn copy_all(app: &mut App, ctx: &egui::Context, t0: Instant) {
        tick(app, ctx, t0);
        tick(app, ctx, t0 + INTERVAL);
    }

    /// This session's descriptions and copies, without its lock and temporary files (which
    /// are never offered; the model names them `.t1.txt.m5.tmp`, so a failure set for
    /// `t1.txt` leaves them too).
    fn files(m: &Model, app: &App) -> Vec<String> {
        let mut v: Vec<String> = m
            .names(queue(app).store().dir())
            .into_iter()
            .filter(|n| n != "lock" && !n.starts_with('.'))
            .collect();
        v.sort();
        v
    }

    fn password() -> Sec {
        Sec {
            enabled: true,
            user: "open sesame".into(),
            ..Sec::default()
        }
    }

    fn mark(app: &mut App, ctx: &egui::Context) {
        let t = &mut app.tabs[0];
        let area = [[72.0, 72.0, 300.0, 120.0]];
        assert!(t.edit(ctx, |p| p.add_redaction_mark(0, &area, "me").map(drop)));
    }

    #[test]
    fn a_changed_tab_is_copied_after_a_minute_and_its_copy_goes_when_saved_or_closed() {
        let ctx = egui::Context::default();
        let m = Model::new();
        let mut app = app_on(&ctx, Arc::new(m.clone()));
        let other = add_changed(&mut app, &ctx);
        let t0 = Instant::now();
        tick(&mut app, &ctx, t0);
        assert!(
            files(&m, &app).is_empty(),
            "nothing before the minute is up"
        );
        tick(&mut app, &ctx, t0 + INTERVAL);
        let f = files(&m, &app);
        assert_eq!(f.len(), 4, "{f:?}");
        assert!(f.contains(&"t1.txt".into()) && f.contains(&"t2.txt".into()));
        assert_eq!(app.tabs[0].copy.copied, Some(app.tabs[0].revision));

        app.close_tab(other);
        tick(&mut app, &ctx, t0 + INTERVAL);
        let f = files(&m, &app);
        assert!(f.iter().all(|n| n.starts_with("t1")), "{f:?}");

        let path = std::env::temp_dir().join(format!(
            "limarepdf-recover-{}-saved.pdf",
            std::process::id()
        ));
        app.active = 0;
        app.tabs[0].path = Some(path.clone());
        app.save_now(&ctx, false);
        let _ = std::fs::remove_file(&path);
        assert!(!app.tabs[0].dirty);
        tick(&mut app, &ctx, t0 + INTERVAL * 2);
        assert!(files(&m, &app).is_empty());
        assert_eq!(app.tabs[0].copy.copied, None);

        // A change after the save is copied again; a normal exit leaves nothing.
        assert!(app.tabs[0].edit(&ctx, |p| p.insert_blank(0, 612.0, 792.0)));
        copy_all(&mut app, &ctx, t0 + INTERVAL * 3);
        assert_eq!(files(&m, &app).len(), 2);
        app.end_recovery();
        assert!(m.names(&root()).is_empty());
    }

    #[test]
    fn a_password_withdraws_the_copy_first_and_protected_tabs_get_none() {
        let ctx = egui::Context::default();
        let m = Model::new();
        let mut app = app_on(&ctx, Arc::new(m.clone()));
        let t0 = Instant::now();
        copy_all(&mut app, &ctx, t0);
        assert!(!files(&m, &app).is_empty());
        app.tabs[0].forms.sec = password();
        app.apply_panel(&ctx, Apply::Security);
        assert!(app.tabs[0].encryption.is_some());
        assert!(files(&m, &app).is_empty());
        assert!(app.tabs[0].edit(&ctx, |p| p.insert_blank(0, 612.0, 792.0)));
        copy_all(&mut app, &ctx, t0 + INTERVAL * 2);
        assert!(files(&m, &app).is_empty());
        // Taking the password off again (undo) doesn't bring copies back.
        app.tabs[0].undo(&ctx);
        app.tabs[0].undo(&ctx);
        assert!(app.tabs[0].encryption.is_none());
        copy_all(&mut app, &ctx, t0 + INTERVAL * 4);
        assert!(files(&m, &app).is_empty());

        // A document opened with a password gets none either, nor pages taken from it.
        let mut p = page();
        p.set_encryption(Some(Encryption {
            user_password: "u".into(),
            owner_password: "o".into(),
            cipher: Cipher::Aes128,
            restrictions: Restrictions::default(),
        }));
        let opened = Pdf::from_bytes(&p.to_bytes().unwrap(), Some("u")).unwrap();
        assert!(app.add_tab(&ctx, opened, None, None, false));
        let i = app.tabs.len() - 1;
        assert!(app.tabs[i].edit(&ctx, |p| p.insert_blank(0, 612.0, 792.0)));
        app.act(&ctx, Act::Extract);
        assert_eq!(app.tabs.len(), i + 2);
        copy_all(&mut app, &ctx, t0 + INTERVAL * 6);
        assert!(files(&m, &app).is_empty());
        assert!(app.tabs[i..]
            .iter()
            .all(|t| t.copy.protected && !t.copy.asked));
    }

    #[test]
    fn merging_a_protected_document_withdraws_the_copy_and_protects_the_tab() {
        // Protected, but opening without a password: an empty user password.
        let mut p = page();
        p.set_encryption(Some(Encryption {
            user_password: String::new(),
            owner_password: "o".into(),
            cipher: Cipher::Aes128,
            restrictions: Restrictions::default(),
        }));
        let locked = std::env::temp_dir().join(format!(
            "limarepdf-recover-{}-locked.pdf",
            std::process::id()
        ));
        std::fs::write(&locked, p.to_bytes().unwrap()).unwrap();
        let plain = locked.with_file_name(format!(
            "limarepdf-recover-{}-plain.pdf",
            std::process::id()
        ));
        std::fs::write(&plain, page().to_bytes().unwrap()).unwrap();

        let ctx = egui::Context::default();
        let m = Model::new();
        let f = Faulty::new(Arc::new(m.clone()));
        let mut app = app_on(&ctx, f.clone());
        let t0 = Instant::now();
        copy_all(&mut app, &ctx, t0);
        // An unprotected document: copies go on.
        app.merge_files(&ctx, std::slice::from_ref(&plain));
        assert_eq!(app.tabs[0].page_count(), 3);
        assert!(!app.tabs[0].copy.protected);
        assert!(!files(&m, &app).is_empty());

        // A copy that could still be offered stops the merge.
        f.fail(Kind::Remove, "t1.txt");
        f.fail(Kind::Rename, "t1.txt");
        app.merge_files(&ctx, &[plain.clone(), locked.clone()]);
        assert_eq!(app.tabs[0].page_count(), 3);
        assert!(matches!(&app.dialog, Dialog::RecoveryFile { .. }));
        f.clear();
        app.dialog = Dialog::None;

        app.merge_files(&ctx, &[plain.clone(), locked.clone()]);
        let _ = std::fs::remove_file(&locked);
        let _ = std::fs::remove_file(&plain);
        assert_eq!(app.tabs[0].page_count(), 5);
        assert!(app.tabs[0].copy.protected);
        assert!(files(&m, &app).is_empty());
        assert!(app.tabs[0].edit(&ctx, |p| p.insert_blank(0, 612.0, 792.0)));
        copy_all(&mut app, &ctx, t0 + INTERVAL * 2);
        assert!(files(&m, &app).is_empty());
    }

    #[test]
    fn removing_content_withdraws_the_copy_and_writes_a_new_one_at_once() {
        for redact in [true, false] {
            let ctx = egui::Context::default();
            let m = Model::new();
            let mut app = app_on(&ctx, Arc::new(m.clone()));
            let t0 = Instant::now();
            if redact {
                mark(&mut app, &ctx);
            }
            copy_all(&mut app, &ctx, t0);
            let before = files(&m, &app);
            assert_eq!(before.len(), 2);
            if redact {
                app.apply_redactions(&ctx);
            } else {
                app.remove_hidden(&ctx, &[Hidden::Metadata]);
            }
            assert!(app.tabs[0].scrubbed);
            assert!(files(&m, &app).is_empty(), "withdrawn");
            // Written again at once, not a minute after the change.
            tick(&mut app, &ctx, t0 + INTERVAL + Duration::from_secs(1));
            let after = files(&m, &app);
            assert_eq!(after.len(), 2);
            assert_ne!(after, before);
            assert_eq!(app.tabs[0].copy.copied, Some(app.tabs[0].revision));
        }
    }

    /// A removal of hidden information that is refused (a hidden layer and a drawing that
    /// can't be read) changes nothing: not the document, its flags, revision or history,
    /// nor its recovery copies.
    #[test]
    fn a_refused_removal_changes_nothing() {
        use lopdf::{dictionary, Object, Stream};
        let ctx = egui::Context::default();
        let m = Model::new();
        let mut app = app_on(&ctx, Arc::new(m.clone()));
        let t = &mut app.tabs[0];
        assert!(t.edit(&ctx, |p| {
            let off = p.doc.add_object(
                dictionary! { "Type" => "OCG", "Name" => Object::string_literal("Off") },
            );
            let props = dictionary! {
                "OCGs" => vec![Object::Reference(off)],
                "D" => dictionary! { "OFF" => vec![Object::Reference(off)] },
            };
            p.doc.catalog_mut()?.set("OCProperties", props);
            let broken = p.doc.add_object(Stream::new(
                dictionary! { "Type" => "XObject", "Subtype" => "Form", "Filter" => "FlateDecode" },
                b"not compressed data".to_vec(),
            ));
            let page = p.page_ids()[0];
            let res = dictionary! { "XObject" => dictionary! { "Fm" => broken } };
            p.doc.get_dictionary_mut(page)?.set("Resources", res);
            p.set_info(&limarepdf_core::Info {
                title: Some("Walrus title".into()),
                ..Default::default()
            })
        }));
        let t0 = Instant::now();
        copy_all(&mut app, &ctx, t0);
        let copies = files(&m, &app);
        assert_eq!(copies.len(), 2);
        let t = &app.tabs[0];
        let (state, doc) = (t.state(), t.pdf.clone().snapshot().unwrap());

        app.remove_hidden(&ctx, &[Hidden::Metadata, Hidden::HiddenLayers]);
        assert!(
            matches!(&app.dialog, Dialog::Message(_, text) if text.contains("can't be read")),
            "a message saying why"
        );
        let t = &app.tabs[0];
        assert!(t.state() == state, "flags, revision and history unchanged");
        assert!(!t.scrubbed);
        assert_eq!(t.pdf.clone().snapshot().unwrap(), doc);
        assert_eq!(t.pdf.info().title.as_deref(), Some("Walrus title"));
        assert_eq!(files(&m, &app), copies, "the copies stay");
    }

    /// A one-page document drawing "Walrus text" inside `mark`'s area, with a title, an
    /// XMP stream and an attachment, each saying "Walrus" too.
    fn secrets() -> Pdf {
        use lopdf::{dictionary, Dictionary, Object, Stream};
        let mut p = page();
        p.set_info(&limarepdf_core::Info {
            title: Some("Walrus title".into()),
            ..Default::default()
        })
        .unwrap();
        let font = p.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let text = b"BT /F1 12 Tf 100 690 Td (Walrus text) Tj ET".to_vec();
        let s = p.doc.add_object(Stream::new(Dictionary::new(), text));
        let xmp = p.doc.add_object(Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            b"<x:xmpmeta>Walrus XMP</x:xmpmeta>".to_vec(),
        ));
        let file = p.doc.add_object(Stream::new(
            dictionary! { "Type" => "EmbeddedFile" },
            b"Walrus attachment".to_vec(),
        ));
        let spec = p.doc.add_object(dictionary! {
            "Type" => "Filespec", "F" => Object::string_literal("notes.txt"),
            "EF" => dictionary! { "F" => file },
        });
        let id = p.page_ids()[0];
        let d = p.doc.get_dictionary_mut(id).unwrap();
        d.set("Contents", s);
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
        let names = dictionary! {
            "Names" => vec![Object::string_literal("notes.txt"), Object::Reference(spec)],
        };
        let c = p.doc.catalog_mut().unwrap();
        c.set("Metadata", xmp);
        c.set("Names", dictionary! { "EmbeddedFiles" => names });
        p
    }

    /// What the tab's copy on `m` says of "Walrus …"; see [`walruses_in`].
    fn walruses(m: &Model, app: &App) -> Vec<String> {
        let dir = queue(app).store().dir().to_path_buf();
        let pdf = files(m, app)
            .into_iter()
            .find(|n| n.ends_with(".pdf"))
            .expect("a copy");
        walruses_in(&m.read(&dir.join(pdf)).unwrap())
    }

    /// What a PDF says of "Walrus …", anywhere in it: every object's strings and
    /// (decompressed) streams.
    fn walruses_in(bytes: &[u8]) -> Vec<String> {
        use lopdf::Object;
        fn find(o: &Object, out: &mut Vec<String>) {
            let mut look = |b: &[u8]| {
                let s = String::from_utf8_lossy(b);
                for (i, _) in s.match_indices("Walrus ") {
                    let w = s[i..].split(['<', ')']).next().unwrap_or_default();
                    out.push(w.to_string());
                }
            };
            match o {
                Object::String(b, _) => look(b),
                Object::Array(a) => a.iter().for_each(|x| find(x, out)),
                Object::Dictionary(d) => d.iter().for_each(|(_, v)| find(v, out)),
                Object::Stream(s) => {
                    look(
                        &s.decompressed_content()
                            .unwrap_or_else(|_| s.content.clone()),
                    );
                    s.dict.iter().for_each(|(_, v)| find(v, out));
                }
                _ => {}
            }
        }
        let doc = lopdf::Document::load_mem(bytes).unwrap();
        let mut out = Vec::new();
        doc.objects.values().for_each(|o| find(o, &mut out));
        out.sort();
        out.dedup();
        out
    }

    /// Whether a start after a crash now, or after a power cut now in any of the ways it
    /// could leave the disk, could offer a copy saying `walrus`.
    fn could_offer(m: &Model, walrus: &str) -> bool {
        std::iter::once(m.crash()).chain(m.power_cuts()).any(|cut| {
            let fs: Arc<dyn Fs> = Arc::new(cut);
            recovery::find_crashed(&fs, &root(), clock()())
                .iter()
                .any(|c| {
                    (0..c.docs.len()).any(|i| {
                        c.read(i)
                            .is_ok_and(|b| walruses_in(&b).iter().any(|w| w == walrus))
                    })
                })
        })
    }

    /// An app keeping its copies on a disk that fails as told, with one tab of
    /// [`secrets`]: opened, or `restored` from a crashed session's copy (its tab 7).
    fn secrets_app(ctx: &egui::Context, restored: bool) -> (Model, Arc<Faulty>, App) {
        let m = if restored {
            crashed_with(&secrets().to_bytes().unwrap(), DocInfo::default())
        } else {
            Model::new()
        };
        let f = Faulty::new(Arc::new(m.clone()));
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        app.start_recovery_in(ctx, f.clone(), &root(), clock());
        if restored {
            app.restore_copy(ctx, 0, TabId(7));
            app.close_restore(ctx);
        } else {
            assert!(app.add_tab(ctx, secrets(), None, None, false));
        }
        (m, f, app)
    }

    /// Redoing a redaction or a removal of hidden information removes the content again,
    /// so the copy written after undoing it is withdrawn first: in an ordinary tab and a
    /// restored one, and for a second removal (the tab already marked as having had content
    /// removed) as for a first. No crash or power cut afterwards can offer the content, and
    /// a new copy without it is written at once.
    #[test]
    fn redoing_a_removal_withdraws_the_copy_made_after_undoing_it() {
        let kinds = [
            ("text", "Walrus text"),
            ("metadata", "Walrus XMP"),
            ("attachments", "Walrus attachment"),
        ];
        for (kind, gone) in kinds {
            for restored in [false, true] {
                for again in [false, true] {
                    let case = format!("{kind}, restored: {restored}, again: {again}");
                    let ctx = egui::Context::default();
                    let (m, _, mut app) = secrets_app(&ctx, restored);
                    if again {
                        // A removal of nothing secret first.
                        app.remove_hidden(&ctx, &[Hidden::Scripts]);
                        assert!(app.tabs[0].scrubbed, "{case}");
                    }
                    mark(&mut app, &ctx);
                    match kind {
                        "text" => app.apply_redactions(&ctx),
                        "metadata" => app.remove_hidden(&ctx, &[Hidden::Metadata]),
                        _ => app.remove_hidden(&ctx, &[Hidden::Attachments]),
                    }
                    assert!(app.tabs[0].scrubbed, "{case}");
                    app.dialog = Dialog::None;
                    let t0 = Instant::now();
                    tick(&mut app, &ctx, t0);
                    assert!(!walruses(&m, &app).contains(&gone.into()), "{case}");

                    // Undone: the content is back, and so is it in the next copy.
                    app.act(&ctx, Act::Undo);
                    copy_all(&mut app, &ctx, t0 + INTERVAL);
                    assert!(walruses(&m, &app).contains(&gone.into()), "{case}");
                    assert!(could_offer(&m, gone), "{case}");

                    // Redone: that copy goes first.
                    app.act(&ctx, Act::Redo);
                    assert!(matches!(app.dialog, Dialog::None), "{case}");
                    assert!(app.tabs[0].scrubbed, "{case}");
                    assert!(files(&m, &app).is_empty(), "{case}: not withdrawn");
                    assert!(!could_offer(&m, gone), "{case}");
                    tick(&mut app, &ctx, t0 + INTERVAL * 2 + Duration::from_secs(1));
                    let t = &app.tabs[0];
                    assert_eq!(
                        t.copy.copied,
                        Some(t.revision),
                        "{case}: not written at once"
                    );
                    assert!(!walruses(&m, &app).contains(&gone.into()), "{case}");
                    assert!(!could_offer(&m, gone), "{case}");
                }
            }
        }
    }

    /// A redo of a removal whose withdrawal can't be confirmed doesn't happen, at every
    /// try while the folder sync keeps failing: the document, its flags and both undo lists
    /// stay as they were. Once a sync succeeds, the redo goes ahead.
    #[test]
    fn a_redo_that_cant_withdraw_the_copy_changes_nothing() {
        for restored in [false, true] {
            let ctx = egui::Context::default();
            let (m, f, mut app) = secrets_app(&ctx, restored);
            mark(&mut app, &ctx);
            app.apply_redactions(&ctx);
            let t0 = Instant::now();
            tick(&mut app, &ctx, t0);
            app.act(&ctx, Act::Undo);
            copy_all(&mut app, &ctx, t0 + INTERVAL);
            f.fail(Kind::Sync, "session-");
            let state = app.tabs[0].state();
            let objects = app.tabs[0].pdf.doc.objects.clone();
            for _ in 0..3 {
                app.act(&ctx, Act::Redo);
                let t = &app.tabs[0];
                assert!(t.state() == state, "restored: {restored}: the tab changed");
                assert!(t.pdf.doc.objects == objects, "the content changed");
                assert!(t.redo_removes() && !t.scrubbed);
                assert_eq!(t.pdf.redaction_marks().len(), 1);
                match &app.dialog {
                    Dialog::RecoveryFile { text, .. } => {
                        assert!(text.contains("hasn't redone the change"), "{text}")
                    }
                    _ => panic!("no message"),
                }
                app.dialog = Dialog::None;
            }
            f.clear();
            app.act(&ctx, Act::Redo);
            let t = &app.tabs[0];
            assert!(t.scrubbed && t.pdf.redaction_marks().is_empty());
            assert!(!could_offer(&m, "Walrus text"));
        }
    }

    /// A restored tab's copy whose withdrawal couldn't be confirmed (the crashed session's
    /// folder sync failed) stops the command at every try, though the restore list no
    /// longer has it, until a sync succeeds; then no crash or power cut can bring it back.
    #[test]
    fn a_restored_copy_not_confirmed_withdrawn_stops_the_command_at_every_try() {
        let ctx = egui::Context::default();
        let (m, f, mut app) = secrets_app(&ctx, true);
        let session = app.rescue.found[0].crashed.dir.clone();
        mark(&mut app, &ctx);
        f.fail(Kind::Sync, &session.file_name().unwrap().to_string_lossy());
        let state = app.tabs[0].state();
        let objects = app.tabs[0].pdf.doc.objects.clone();
        for n in 0..3 {
            app.apply_redactions(&ctx);
            let t = &app.tabs[0];
            assert!(t.state() == state, "try {n}: the tab changed");
            assert!(t.pdf.doc.objects == objects, "try {n}: the content changed");
            match &app.dialog {
                Dialog::RecoveryFile { text, file, .. } => {
                    assert_eq!(*file, session);
                    assert!(text.contains("hasn't applied the redactions"), "{text}");
                }
                _ => panic!("try {n}: no message"),
            }
            app.dialog = Dialog::None;
            assert!(app.rescue.found[0].crashed.docs.is_empty());
            assert!(
                could_offer(&m, "Walrus text"),
                "a power cut could bring it back"
            );
        }
        f.clear();
        app.apply_redactions(&ctx);
        assert!(app.tabs[0].scrubbed);
        assert!(!m.exists(&session));
        assert!(!could_offer(&m, "Walrus text"));
    }

    #[test]
    fn the_copy_after_removing_content_has_none_of_it() {
        for redact in [true, false] {
            let ctx = egui::Context::default();
            let m = Model::new();
            let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
            app.start_recovery_in(&ctx, Arc::new(m.clone()), &root(), clock());
            assert!(app.add_tab(&ctx, secrets(), None, None, false));
            mark(&mut app, &ctx);
            let t0 = Instant::now();
            copy_all(&mut app, &ctx, t0);
            let all = [
                "Walrus XMP",
                "Walrus attachment",
                "Walrus text",
                "Walrus title",
            ];
            assert_eq!(walruses(&m, &app), all);
            let left: &[&str] = if redact {
                app.apply_redactions(&ctx);
                &["Walrus XMP", "Walrus attachment", "Walrus title"]
            } else {
                app.remove_hidden(&ctx, &Hidden::ALL);
                &["Walrus text"]
            };
            assert!(app.tabs[0].scrubbed);
            tick(&mut app, &ctx, t0 + INTERVAL + Duration::from_secs(1));
            assert_eq!(app.tabs[0].copy.copied, Some(app.tabs[0].revision));
            assert_eq!(walruses(&m, &app), left, "redact: {redact}");
        }
    }

    #[test]
    fn a_copy_that_could_still_be_offered_stops_the_command() {
        let cases: [(&str, bool); 4] = [
            ("applied the redactions", false),
            ("removed the hidden information", false),
            ("added the password", false),
            ("applied the redactions", true),
        ];
        for (n, (done, sync)) in cases.into_iter().enumerate() {
            let ctx = egui::Context::default();
            let m = Model::new();
            let f = Faulty::new(Arc::new(m.clone()));
            let mut app = app_on(&ctx, f.clone());
            mark(&mut app, &ctx);
            copy_all(&mut app, &ctx, Instant::now());
            if sync {
                // Removed, but the folder sync that makes it last fails.
                f.fail(Kind::Sync, "session-");
            } else {
                // The description can be neither removed nor replaced.
                f.fail(Kind::Remove, "t1.txt");
                f.fail(Kind::Rename, "t1.txt");
            }
            app.tabs[0].forms.sec = password();
            let state = app.tabs[0].state();
            let objects = app.tabs[0].pdf.doc.objects.clone();
            let run = |app: &mut App| match n {
                0 | 3 => app.apply_redactions(&ctx),
                1 => app.remove_hidden(&ctx, &[Hidden::Metadata]),
                _ => app.apply_panel(&ctx, Apply::Security),
            };
            run(&mut app);
            assert!(app.tabs[0].state() == state, "{done}: the document changed");
            assert!(
                app.tabs[0].pdf.doc.objects == objects,
                "{done}: the content changed"
            );
            assert_eq!(app.tabs[0].pdf.redaction_marks().len(), 1);
            match &app.dialog {
                Dialog::RecoveryFile { text, file, .. } => {
                    assert!(text.contains(&file.display().to_string()), "{text}");
                    assert!(text.contains(&format!("hasn't {done}")), "{text}");
                    let named = if sync { "session-" } else { "t1.txt" };
                    assert!(file.to_string_lossy().contains(named), "{}", file.display());
                }
                _ => panic!("{done}: no message naming the file"),
            }
            // Once the copy can go, the command runs.
            f.clear();
            app.dialog = Dialog::None;
            run(&mut app);
            let t = &app.tabs[0];
            assert!(
                if n == 2 {
                    t.encryption.is_some()
                } else {
                    t.scrubbed
                },
                "{done}"
            );
        }
    }

    #[test]
    fn a_copy_invalidated_but_left_on_disk_lets_the_command_go_ahead_with_a_notice() {
        let ctx = egui::Context::default();
        let m = Model::new();
        let f = Faulty::new(Arc::new(m.clone()));
        let mut app = app_on(&ctx, f.clone());
        let t0 = Instant::now();
        copy_all(&mut app, &ctx, t0);
        f.fail(Kind::Remove, "t1-");
        app.remove_hidden(&ctx, &[Hidden::Metadata]);
        assert!(app.tabs[0].scrubbed);
        assert!(
            matches!(&app.dialog, Dialog::Message(title, _) if title == "Hidden information removed")
        );
        let left = app.tabs[0].copy.leftovers.clone();
        assert_eq!(left.len(), 1);
        let notice = &app.tabs[0].banners[0];
        assert_eq!(notice.kind, BannerKind::LeftoverCopy);
        assert!(notice.text.contains(&left[0].display().to_string()));
        // No new copies for the rest of the session.
        tick(&mut app, &ctx, t0 + INTERVAL * 2);
        assert_eq!(
            files(&m, &app),
            [left[0].file_name().unwrap().to_string_lossy()]
        );
        // Deleted once it can be, at the next try; then the notice goes.
        f.clear();
        tick(&mut app, &ctx, t0 + INTERVAL * 3);
        tick(&mut app, &ctx, t0 + INTERVAL * 4);
        assert!(files(&m, &app).is_empty());
        assert!(app.tabs[0].copy.leftovers.is_empty());
        assert!(app.tabs[0]
            .banners
            .iter()
            .all(|b| b.kind != BannerKind::LeftoverCopy));
    }

    #[test]
    fn turning_recovery_off_always_takes_effect() {
        let ctx = egui::Context::default();
        let m = Model::new();
        let f = Faulty::new(Arc::new(m.clone()));
        let mut app = app_on(&ctx, f.clone());
        add_changed(&mut app, &ctx);
        let t0 = Instant::now();
        copy_all(&mut app, &ctx, t0);
        f.fail(Kind::Remove, "t1.txt");
        f.fail(Kind::Rename, "t1.txt");
        app.act(&ctx, Act::ToggleRecovery);
        assert!(!app.settings.recovery && !queue(&app).store().is_on());
        match &app.dialog {
            Dialog::RecoveryFile { file, .. } => assert!(file.ends_with("t1.txt")),
            _ => panic!("the copy left isn't reported"),
        }
        f.clear();
        assert!(app.tabs[1].edit(&ctx, |p| p.insert_blank(0, 612.0, 792.0)));
        copy_all(&mut app, &ctx, t0 + INTERVAL * 2);
        assert_eq!(files(&m, &app), ["t1.txt"]);

        // On again: copies resume, except for the tab whose withdrawal wasn't confirmed.
        app.act(&ctx, Act::ToggleRecovery);
        tick(&mut app, &ctx, t0 + INTERVAL * 4);
        let f = files(&m, &app);
        assert!(f.contains(&"t2.txt".into()) && f.len() == 3, "{f:?}");
    }

    #[test]
    fn recovery_that_cant_start_is_off_with_one_notice() {
        let ctx = egui::Context::default();
        let f = Faulty::new(Arc::new(Model::new()));
        f.fail(Kind::Sync, "/state");
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        app.start_recovery_in(&ctx, f.clone(), &root(), clock());
        assert!(app.rescue.queue.is_none());
        assert!(matches!(&app.dialog, Dialog::Message(t, _) if t == "Recovery copies are off"));
        // Not when the user turned recovery off.
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        app.settings.recovery = false;
        app.start_recovery_in(&ctx, f, &root(), clock());
        assert!(matches!(app.dialog, Dialog::None));
    }

    /// A disk left by a LimarePDF that stopped with a copy of `bytes` for its tab 7.
    fn crashed_with(bytes: &[u8], info: DocInfo) -> Model {
        let m = Model::new();
        let s = Store::start(Arc::new(m.clone()), &root(), clock()).unwrap();
        let t = s.ticket(TabId(7), 3).unwrap();
        s.publish(&t, &info, bytes).unwrap();
        m.crash()
    }

    #[test]
    fn a_restored_signed_copy_is_marked_and_its_signatures_never_checked() {
        let bytes = include_bytes!(
            "../../../limarepdf-core/tests/data/signatures/pyhanko/minimal-two-fields-signed-twice.pdf"
        );
        let info = DocInfo {
            title: "contract.pdf".into(),
            original: Some("/docs/contract.pdf".into()),
            pages: 1,
            signed: true,
            ..DocInfo::default()
        };
        let disk = crashed_with(bytes, info);
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        app.start_recovery_in(&ctx, Arc::new(disk.clone()), &root(), clock());
        assert!(app.rescue.asking);
        let offered = offer(
            &app.rescue.found[0].crashed.docs[0],
            None,
            SystemTime::now(),
        );
        assert_eq!(offered.title.as_deref(), Some("contract.pdf"));
        assert!(
            offered.detail.starts_with("/docs/contract.pdf\n"),
            "{}",
            offered.detail
        );

        app.restore_copy(&ctx, 0, TabId(7));
        let t = &app.tabs[0];
        assert_eq!(t.title(), "contract (recovered) *");
        assert!(t.path.is_none() && t.sigs.is_none(), "no signature check");
        assert_eq!(t.banners[0].kind, BannerKind::Recovered);
        assert!(t.banners[0].text.contains("can't be checked here"));
        let rows = signatures::recovered_rows(&t.pdf);
        assert!(!rows.is_empty() && rows.len() == t.pdf.signature_count());
        assert!(rows
            .iter()
            .all(|r| r.ends_with(": Not checked: recovered copy")));
        // Restoring didn't make another copy of it.
        tick(&mut app, &ctx, Instant::now() + INTERVAL * 2);
        assert!(!app.tabs[0].copy.asked);

        // Save and Sign both ask before breaking the signatures.
        app.save_signed(&ctx, false);
        assert!(matches!(&app.dialog, Dialog::ConfirmSaveSigned(false, why) if why == RECOVERED));
        app.dialog = Dialog::None;
        app.act(&ctx, Act::SignDigital);
        assert!(matches!(app.dialog, Dialog::ConfirmSignRecovered));
        assert!(app.tabs[0].ed.tool != Tool::DigitalSign);

        // The copy stays while its tab is open, and goes when it is closed.
        app.close_restore(&ctx);
        assert_eq!(app.rescue.found.len(), 1);
        let session = app.rescue.found[0].crashed.dir.clone();
        assert!(disk.exists(&session.join("t7.txt")));
        app.close_tab(0);
        assert!(app.rescue.found.is_empty());
        assert!(!disk.exists(&session));
    }

    #[test]
    fn removing_content_from_a_restored_tab_withdraws_the_copy_it_came_from() {
        let bytes = secrets().to_bytes().unwrap();
        let cases = [
            "applied the redactions",
            "removed the hidden information",
            "added the password",
        ];
        for (n, done) in cases.into_iter().enumerate() {
            for stuck in [true, false] {
                let disk = crashed_with(&bytes, DocInfo::default());
                let f = Faulty::new(Arc::new(disk.clone()));
                let ctx = egui::Context::default();
                let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
                app.start_recovery_in(&ctx, f.clone(), &root(), clock());
                app.restore_copy(&ctx, 0, TabId(7));
                app.close_restore(&ctx);
                let session = app.rescue.found[0].crashed.dir.clone();
                let desc = session.join("t7.txt");
                mark(&mut app, &ctx);
                app.tabs[0].forms.sec = password();
                if stuck {
                    f.fail(Kind::Remove, "t7.txt");
                    f.fail(Kind::Rename, "t7.txt");
                }
                let state = app.tabs[0].state();
                match n {
                    0 => app.apply_redactions(&ctx),
                    1 => app.remove_hidden(&ctx, &[Hidden::Metadata]),
                    _ => app.apply_panel(&ctx, Apply::Security),
                }
                if stuck {
                    assert!(app.tabs[0].state() == state, "{done}: the document changed");
                    match &app.dialog {
                        Dialog::RecoveryFile { text, file, .. } => {
                            assert_eq!(*file, desc, "{done}");
                            assert!(text.contains(&format!("hasn't {done}")), "{text}");
                            // Turning recovery off doesn't remove a crashed session's copy.
                            assert!(!text.contains("turn off"), "{text}");
                        }
                        _ => panic!("{done}: no message naming the file"),
                    }
                    assert!(disk.exists(&desc));
                    continue;
                }
                let t = &app.tabs[0];
                assert!(
                    if n == 2 {
                        t.encryption.is_some()
                    } else {
                        t.scrubbed
                    },
                    "{done}"
                );
                assert!(
                    !disk.exists(&session),
                    "{done}: the old copy is still there"
                );
                if n == 0 {
                    // The tab's new copy, in this session, is of the redacted document.
                    tick(&mut app, &ctx, Instant::now());
                    let w = walruses(&disk, &app);
                    assert!(!w.is_empty() && !w.contains(&"Walrus text".into()), "{w:?}");
                }
                app.close_tab(0);
                assert!(app.rescue.found.is_empty());
            }
        }
    }

    #[test]
    fn a_damaged_copy_is_offered_to_show_or_discard_never_restored() {
        let disk = crashed_with(b"%PDF-1.7 not much", DocInfo::default());
        let ctx = egui::Context::default();
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        app.start_recovery_in(&ctx, Arc::new(disk.clone()), &root(), clock());
        // It doesn't open: the window says why, and offers Show file and Discard.
        app.restore_copy(&ctx, 0, TabId(7));
        assert!(app.tabs.is_empty());
        let src = &app.rescue.found[0];
        let o = offer(
            &src.crashed.docs[0],
            src.broken.get(&TabId(7)),
            SystemTime::now(),
        );
        assert!(o.title.is_none());
        assert!(
            o.detail
                .starts_with("It can't be restored: it doesn't open"),
            "{}",
            o.detail
        );
        // "Not now" keeps it for the next start.
        app.close_restore(&ctx);
        assert!(app.rescue.found.is_empty());
        let again = recovery::find_crashed(&(Arc::new(disk) as Arc<dyn Fs>), &root(), clock()());
        assert_eq!(again.len(), 1);
    }
}
