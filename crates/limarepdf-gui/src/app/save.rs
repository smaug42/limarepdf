//! Saving on a thread of its own (ENH-35). The window's thread keeps the typing, asks the
//! save's questions and makes its checks, as before; then it hands a copy of the document
//! (or, for a signed one, the bytes with the edits added) to a thread, which writes the
//! file and reads it back. The window keeps drawing meanwhile, with a busy sign in the
//! status bar, and the document can still be changed: the file gets it as it was when
//! saved, and a tab changed since stays unsaved. Saving the tab again, closing it and
//! quitting wait for the save to end; Reload and signing are off until then.

use super::{not_confirmed, App, Dialog, Loaded, NOT_CONFIRMED};
use crate::banners::Kind as BannerKind;
use crate::signatures::Check;
use crate::tab::State;
use crate::timing;
use eframe::egui;
use limarepdf_core::{disk, Pdf};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

/// What a save's thread writes.
pub(super) enum Write {
    /// The whole document, encrypted as it is set to be.
    Whole(Box<Pdf>),
    /// A signed document's file with the edits added after what was signed.
    Appended(Vec<u8>),
}

/// A save whose thread is writing.
pub(super) struct Running {
    tab: u64,
    n: u64,
    path: PathBuf,
    /// The tab when it was handed over: changed since, it stays unsaved.
    state: State,
    appended: bool,
    /// The password to check the saved file's signatures with, when they are checked.
    check: Option<Option<String>>,
    /// Save (false) or Save as (true) chosen again while it ran, done once it ends.
    again: Option<bool>,
    thread: Option<JoinHandle<()>>,
}

/// What a save's thread sends back.
pub(super) struct Saved {
    tab: u64,
    n: u64,
    path: PathBuf,
    written: limarepdf_core::Result<()>,
    /// The file read back, with its version; `None` when nothing was written.
    read: Option<std::io::Result<(Vec<u8>, disk::Version)>>,
}

/// What waits for a save to end before it goes on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum AfterSave {
    /// Save tab `.0` again (`.1`: as a new file), chosen while its save ran.
    Save(u64, bool),
    /// Close tab `.0`, asking as usual if it is still unsaved.
    Close(u64),
    /// Quit, asking as usual about anything still unsaved, once every save has ended.
    Quit,
}

impl App {
    /// Whether tab `tab`'s save is being written.
    pub(super) fn saving(&self, tab: u64) -> bool {
        self.saves.iter().any(|r| r.tab == tab)
    }

    /// Writes `write` to `path` for the active tab on a thread of its own; the result
    /// arrives as [`Loaded::Saved`]. `check`: the password to check the saved file's
    /// signatures with, when they are checked.
    pub(super) fn start_save(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        write: Write,
        check: Option<Option<String>>,
    ) {
        let t = &self.tabs[self.active];
        self.saves_made += 1;
        let (tab, n) = (t.id, self.saves_made);
        let appended = matches!(write, Write::Appended(_));
        let (tx, repaint) = (self.tx.clone(), ctx.clone());
        #[cfg(test)]
        let gate = self.save_gate.take();
        let to = path.clone();
        let thread = std::thread::spawn(move || {
            #[cfg(test)]
            if let Some(gate) = gate {
                let _ = gate.recv();
            }
            timing::mark("save start", || to.display().to_string());
            let written = crate::guard(|| match write {
                Write::Whole(mut pdf) => pdf.save(&to),
                Write::Appended(bytes) => limarepdf_core::write_file(&to, &bytes),
            });
            timing::mark("save end", || super::saved(&written).into());
            // Not confirmed on disk is written all the same.
            let wrote = written.as_ref().map_or_else(not_confirmed, |()| true);
            let read = wrote.then(|| disk::read(&to));
            let _ = tx.send(Loaded::Saved(Box::new(Saved {
                tab,
                n,
                path: to,
                written,
                read,
            })));
            repaint.request_repaint();
        });
        self.saves.push(Running {
            tab,
            n,
            path,
            state: t.state(),
            appended,
            check,
            again: None,
            thread: Some(thread),
        });
        // The busy sign says it; an older message would read as this save's.
        self.status.clear();
        // The next frame shows the sign.
        ctx.request_repaint();
    }

    /// The busy sign's text while saves are written, with the newest one's number.
    pub(super) fn busy_sign(&self) -> Option<(u64, String)> {
        let newest = self.saves.last()?;
        let text = match self.saves.len() {
            1 => {
                let name = newest.path.file_name().unwrap_or(newest.path.as_os_str());
                format!("Saving {}…", name.to_string_lossy())
            }
            n => format!("Saving {n} documents…"),
        };
        Some((newest.n, text))
    }

    /// Save (or Save as, `as_new`) chosen for the active tab while its save runs: done
    /// once it ends. Save as stays chosen over Save.
    pub(super) fn save_again(&mut self, as_new: bool) {
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        if let Some(r) = self.saves.iter_mut().find(|r| r.tab == t.id) {
            r.again = Some(as_new || r.again == Some(true));
        }
    }

    /// A save's thread finished: the tab is saved as [`App::save`] always left it, unless
    /// it changed meanwhile, when it stays unsaved; the same messages are shown.
    pub(super) fn saved(&mut self, ctx: &egui::Context, s: Saved) {
        let Some(r) = self.saves.iter().position(|r| r.tab == s.tab && r.n == s.n) else {
            return;
        };
        let mut run = self.saves.remove(r);
        if let Some(thread) = run.thread.take() {
            let _ = thread.join();
        }
        let confirmed = match s.written {
            Ok(()) => true,
            Err(e) if not_confirmed(&e) => false,
            Err(e) => {
                self.status.clear();
                self.show(Dialog::Message("Save failed".into(), format!("{e:#}")));
                return;
            }
        };
        self.status = if run.appended {
            format!(
                "Saved {}; the changes were added after what was signed, so the signatures stay",
                s.path.display()
            )
        } else {
            format!("Saved {}", s.path.display())
        };
        self.recent.add(&s.path);
        self.replace_ok = None;
        // Closing and quitting wait for the save, so the tab is there, unless the save
        // outlived it some other way: the file is saved all the same.
        let Some(i) = self.tabs.iter().position(|t| t.id == s.tab) else {
            return;
        };
        let t = &mut self.tabs[i];
        let before = t.state();
        let unchanged = before == run.state;
        t.dirty = !(confirmed && unchanged);
        if unchanged {
            t.scrubbed = false;
        }
        if !run.appended {
            // The saved file is the repaired copy, with nothing left to repair, and it reads
            // the same in every app.
            t.repaired = false;
            t.banners
                .retain(|b| !matches!(b.kind, BannerKind::Repaired | BannerKind::OtherApps));
            // The saved copy's signatures are already broken: no need to ask again.
            t.signed = false;
        }
        let (bytes, version) = match s.read {
            Some(Ok((bytes, v))) => (Ok(bytes), Some(v)),
            Some(Err(e)) => (Err(e.to_string()), None),
            None => (Err(String::new()), None),
        };
        t.wrote(s.path, version);
        // The Signatures tab describes the file, so check the saved one (a saved recovered
        // copy's too, which was never checked).
        match (run.check.take(), bytes) {
            (Some(password), Ok(bytes)) => {
                self.check_signatures(ctx, i, Arc::from(bytes), password);
            }
            (Some(_), Err(e)) => {
                let t = &mut self.tabs[i];
                if let Some(c) = t.sigs.as_mut() {
                    c.check =
                        Check::Failed(format!("LimarePDF can't read the saved file again: {e}"));
                    t.sig_banner();
                }
            }
            (None, _) => {}
        }
        self.saved_copy(i, confirmed && unchanged);
        // Questions asked about the tab while it saved still apply to it.
        let (id, now) = (s.tab, self.tabs[i].state());
        let open = std::iter::once(&mut self.dialog).chain(&mut self.waiting);
        for a in open.filter_map(Dialog::asked_mut) {
            if a.tab == id && *a.state == before {
                *a.state = now.clone();
            }
        }
        if let Some(as_new) = run.again {
            self.after_saves.push(AfterSave::Save(id, as_new));
        }
        if !confirmed {
            self.show(Dialog::Message(
                "Saved, not confirmed".into(),
                NOT_CONFIRMED.into(),
            ));
        }
    }

    /// Goes on with what waited for a save, once no question is open: the first whose
    /// save has ended (each may ask something).
    pub(super) fn after_save(&mut self, ctx: &egui::Context) {
        let ready = self.after_saves.iter().position(|a| match *a {
            AfterSave::Save(tab, _) | AfterSave::Close(tab) => !self.saving(tab),
            AfterSave::Quit => self.saves.is_empty(),
        });
        let Some(k) = ready else {
            return;
        };
        match self.after_saves.remove(k) {
            AfterSave::Save(tab, as_new) => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == tab) {
                    // The tab comes to the front for its save's questions.
                    self.active = i;
                    self.save(ctx, as_new);
                }
            }
            AfterSave::Close(tab) => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == tab) {
                    self.close_asking(i);
                }
            }
            AfterSave::Quit => self.quit(ctx),
        }
    }

    /// `then` once the saves it waits for have ended.
    pub(super) fn wait_for_saves(&mut self, then: AfterSave) {
        if !self.after_saves.contains(&then) {
            self.after_saves.push(then);
        }
    }

    /// At exit, the saves still being written are finished first.
    pub(super) fn finish_saves(&mut self) {
        for r in &mut self.saves {
            if let Some(thread) = r.thread.take() {
                let _ = thread.join();
            }
        }
    }
}
