//! Files the system asks LimarePDF to open outside its command line: on macOS, Finder's Open
//! With, a double-click and a file dropped on the Dock icon (macos.rs; BUG-28). They can come
//! before the window exists or while the app runs, so they wait here for the app's next frame.

use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

/// Files waiting to be opened, and the window to wake when more come.
#[derive(Clone, Default)]
pub struct Incoming(Arc<Mutex<Waiting>>);

#[derive(Default)]
struct Waiting {
    files: Vec<PathBuf>,
    /// Files asked for whose place on disk couldn't be read.
    unread: usize,
    window: Option<egui::Context>,
}

impl Incoming {
    /// The one the system hands files to; the app's tests make their own.
    pub fn system() -> &'static Incoming {
        static SYSTEM: OnceLock<Incoming> = OnceLock::new();
        SYSTEM.get_or_init(Incoming::default)
    }

    /// Files to open, and how many more were asked for that can't be found. Elsewhere files
    /// come only on the command line.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn hand_in(&self, files: Vec<PathBuf>, unread: usize) {
        let mut w = self.lock();
        w.files.extend(files);
        w.unread += unread;
        if let Some(ctx) = &w.window {
            ctx.request_repaint();
        }
    }

    /// Wakes `ctx` whenever files are handed in from now on.
    pub fn wake(&self, ctx: &egui::Context) {
        self.lock().window = Some(ctx.clone());
    }

    /// The files waiting, in the order asked for, and the count of those that can't be found.
    pub fn take(&self) -> (Vec<PathBuf>, usize) {
        let mut w = self.lock();
        (std::mem::take(&mut w.files), std::mem::take(&mut w.unread))
    }

    fn lock(&self) -> MutexGuard<'_, Waiting> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Files wait in the order sent, from before the window exists, and each new one wakes
    /// the window once it does.
    #[test]
    fn files_wait_in_order_and_wake_the_window() {
        let ctx = egui::Context::default();
        let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = wakes.clone();
        ctx.set_request_repaint_callback(move |_| {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        let woken = || wakes.load(std::sync::atomic::Ordering::SeqCst);
        let incoming = Incoming::default();
        incoming.hand_in(vec!["a.pdf".into()], 1);
        incoming.wake(&ctx);
        assert_eq!(woken(), 0);
        incoming.hand_in(vec!["b.pdf".into(), "c.pdf".into()], 0);
        assert_eq!(woken(), 1);
        let files: Vec<PathBuf> = ["a.pdf", "b.pdf", "c.pdf"].map(PathBuf::from).into();
        assert_eq!(incoming.take(), (files, 1));
        assert_eq!(incoming.take(), (Vec::new(), 0));
    }
}
