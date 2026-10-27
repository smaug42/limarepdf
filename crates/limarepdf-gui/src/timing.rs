//! The optional timing log, for measuring the app (REL-07, docs/developer/performance.md).
//! With `LIMAREPDF_TIMING` set to a file's path, the app writes there when it starts, takes
//! input, finishes building a frame, first draws a page at the size it wants, and starts and
//! ends its longer work. Unset, each point costs one check and nothing is written.
//!
//! One line per event, tab-separated: microseconds since the Unix epoch (to match with times
//! taken outside the app), microseconds since the log started, the event and its detail. A
//! frame is "built" once the app has laid it out, before egui paints it and the window shows
//! it (which waits for the display, up to a refresh interval with vsync): the app can't see
//! later than that.

use eframe::egui;
use std::collections::HashSet;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// The environment variable naming the log's file.
pub const VAR: &str = "LIMAREPDF_TIMING";

static LOG: OnceLock<Log> = OnceLock::new();

pub struct Log {
    began: Instant,
    out: Mutex<Out>,
}

struct Out {
    to: Box<dyn Write + Send>,
    /// Events written by [`once`], not written again.
    seen: HashSet<String>,
    /// Writing failed (said once on stderr): nothing more is written.
    failed: bool,
}

impl Log {
    fn new(to: Box<dyn Write + Send>) -> Self {
        Self {
            began: Instant::now(),
            out: Mutex::new(Out {
                to,
                seen: HashSet::new(),
                failed: false,
            }),
        }
    }

    fn write(&self, event: &str, detail: &str, once: bool) {
        let since = self.began.elapsed().as_micros();
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_micros());
        let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        if out.failed || (once && !out.seen.insert(format!("{event}\t{detail}"))) {
            return;
        }
        let line = format!("{epoch}\t{since}\t{event}\t{detail}\n");
        if let Err(e) = out.to.write_all(line.as_bytes()) {
            out.fail(&e);
        }
    }

    fn flush(&self) {
        let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        if !out.failed {
            if let Err(e) = out.to.flush() {
                out.fail(&e);
            }
        }
    }
}

impl Out {
    fn fail(&mut self, e: &std::io::Error) {
        self.failed = true;
        eprintln!("limarepdf: the timing log stopped: {e}");
    }
}

/// Starts the log when `LIMAREPDF_TIMING` names a file. A file that can't be made is said on
/// stderr, and the app runs without the log.
pub fn start() {
    let Some(path) = std::env::var_os(VAR).filter(|p| !p.is_empty()) else {
        return;
    };
    match std::fs::File::create(&path) {
        Ok(f) => {
            let log = Log::new(Box::new(BufWriter::new(f)));
            log.write("start", "", false);
            let _ = LOG.set(log);
        }
        Err(e) => eprintln!(
            "limarepdf: no timing log, as {} can't be written: {e}",
            Path::new(&path).display()
        ),
    }
}

/// Whether the log is on.
pub fn on() -> bool {
    LOG.get().is_some()
}

/// Writes `event` with its detail, made only when the log is on.
pub fn mark(event: &str, detail: impl FnOnce() -> String) {
    if let Some(log) = LOG.get() {
        log.write(event, &detail(), false);
    }
}

/// Writes `event` the first time it comes with this detail.
pub fn once(event: &str, detail: impl FnOnce() -> String) {
    if let Some(log) = LOG.get() {
        log.write(event, &detail(), true);
    }
}

/// Writes out what is held; once a frame and when the app closes.
pub fn flush() {
    if let Some(log) = LOG.get() {
        log.flush();
    }
}

/// The keys the log names: those that type nothing.
const NAMED: [egui::Key; 14] = {
    use egui::Key::*;
    [
        Enter, Escape, Tab, Backspace, Delete, Insert, Home, End, PageUp, PageDown, ArrowDown,
        ArrowLeft, ArrowRight, ArrowUp,
    ]
};

/// What an input event is, for the log; `None` for those not timed (the pointer moving,
/// modifiers alone, the window's own events). What is typed is never written, as it may be a
/// password: text by its length, and a key by its name only when it types nothing.
pub fn input(e: &egui::Event) -> Option<String> {
    let updown = |pressed: bool| if pressed { "down" } else { "up" };
    Some(match e {
        egui::Event::Key { key, pressed, .. } if NAMED.contains(key) => {
            format!("key {} {}", key.name(), updown(*pressed))
        }
        egui::Event::Key { pressed, .. } => format!("key {}", updown(*pressed)),
        egui::Event::Text(t) | egui::Event::Ime(egui::ImeEvent::Commit(t)) => {
            format!("text {}", t.chars().count())
        }
        egui::Event::Paste(_) => "paste".into(),
        egui::Event::Copy => "copy".into(),
        egui::Event::Cut => "cut".into(),
        egui::Event::PointerButton {
            button, pressed, ..
        } => format!("button {button:?} {}", updown(*pressed)),
        egui::Event::MouseWheel { .. } => "wheel".into(),
        egui::Event::Zoom(_) => "zoom".into(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A writer the test can read back.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn lines(s: &Shared) -> Vec<Vec<String>> {
        let text = String::from_utf8(s.0.lock().unwrap().clone()).unwrap();
        text.lines()
            .map(|l| l.split('\t').map(str::to_string).collect())
            .collect()
    }

    #[test]
    fn each_event_is_a_line_of_both_times_the_event_and_its_detail() {
        let s = Shared::default();
        let log = Log::new(Box::new(s.clone()));
        log.write("start", "", false);
        log.write("frame built", "1 pass 0", false);
        let l = lines(&s);
        assert_eq!(l.len(), 2);
        assert_eq!(l[1][2..], ["frame built", "1 pass 0"]);
        let epoch = |i: usize| l[i][0].parse::<u128>().unwrap();
        let since = |i: usize| l[i][1].parse::<u128>().unwrap();
        // Microseconds since 1970 (after 2020), and since the log began, in order.
        assert!(epoch(0) > 1_577_836_800_000_000 && epoch(1) >= epoch(0));
        assert!(since(1) >= since(0));
    }

    #[test]
    fn once_writes_an_event_only_the_first_time_with_its_detail() {
        let s = Shared::default();
        let log = Log::new(Box::new(s.clone()));
        for detail in ["tab 1 page 1", "tab 1 page 1", "tab 1 page 2"] {
            log.write("page drawn", detail, true);
        }
        log.write("page drawn", "tab 1 page 1", false);
        let details: Vec<_> = lines(&s).into_iter().map(|l| l[3].clone()).collect();
        assert_eq!(details, ["tab 1 page 1", "tab 1 page 2", "tab 1 page 1"]);
    }

    /// A writer that fails, counting the tries.
    struct Failing(Arc<Mutex<usize>>);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            *self.0.lock().unwrap() += 1;
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            *self.0.lock().unwrap() += 1;
            Err(std::io::Error::other("disk full"))
        }
    }

    #[test]
    fn a_failed_write_stops_the_log() {
        let tries = Arc::new(Mutex::new(0));
        let log = Log::new(Box::new(Failing(tries.clone())));
        log.write("start", "", false);
        log.write("frame built", "1 pass 0", false);
        log.flush();
        assert_eq!(*tries.lock().unwrap(), 1);
    }

    #[test]
    fn inputs_are_named_without_what_is_typed_and_moves_are_left_out() {
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        let named = |k| input(&key(k));
        assert_eq!(named(egui::Key::Enter).as_deref(), Some("key Enter down"));
        let left = named(egui::Key::ArrowLeft);
        assert_eq!(left.as_deref(), Some("key Left down"));
        use egui::Key::{Comma, Num7, Quote, Space, A};
        for typing in [A, Num7, Comma, Quote, Space] {
            assert_eq!(named(typing).as_deref(), Some("key down"));
        }
        let text = egui::Event::Text("pässwörd".into());
        assert_eq!(input(&text).as_deref(), Some("text 8"));
        let click = egui::Event::PointerButton {
            pos: egui::Pos2::ZERO,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        };
        assert_eq!(input(&click).as_deref(), Some("button Primary up"));
        assert_eq!(input(&egui::Event::PointerMoved(egui::Pos2::ZERO)), None);
    }
}
