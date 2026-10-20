//! Background page rendering. One parsed document is shared by several threads;
//! each thread owns its own (non-Send) render cache. Newest requests are served first
//! so scrolling stays responsive.

use limarepdf_core::page_colours::PageColours;
use limarepdf_core::render::{self, Bitmap, Missing, Region, Renderer};
use std::collections::BTreeSet;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

/// (page index, quantised scale, tile column and row; `None` for the whole page)
pub type Key = (usize, u32, Option<(u32, u32)>);

/// The side of a tile, in bitmap pixels.
pub const TILE: u32 = 512;

/// How pages are coloured on screen.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Colours {
    pub mode: PageColours,
    /// Pictures are recoloured too; otherwise they keep their own colours.
    pub pictures_too: bool,
}

pub struct Response {
    pub key: Key,
    /// The colours it was drawn in; a response drawn before a change is dropped.
    pub colours: Colours,
    pub bitmap: Result<Bitmap, String>,
    /// What couldn't be drawn. Each thing is reported once per thread, so it may have
    /// been on an earlier page.
    pub missing: BTreeSet<Missing>,
}

struct Queue {
    items: Vec<(Key, f32)>,
    /// The jobs being drawn and their colours, one entry per thread.
    running: Vec<Job>,
    /// Running jobs no longer wanted: a whole page stops at its next band and sends
    /// nothing, as its key has also left the tab's pending set.
    dropped: Vec<Job>,
    colours: Colours,
    closed: bool,
}

/// A job being drawn: its key and the colours it is drawn in.
type Job = (Key, Colours);

impl Queue {
    /// Whether the running `job` should stop.
    fn stopped(&self, job: &Job) -> bool {
        self.closed || self.dropped.contains(job)
    }

    /// Stop the running jobs for which `stop` is true.
    fn drop_running(&mut self, stop: impl Fn(&Job) -> bool) {
        for job in &self.running {
            if stop(job) && !self.dropped.contains(job) {
                self.dropped.push(*job);
            }
        }
    }

    /// `job` is no longer running.
    fn finished(&mut self, job: &Job) {
        if let Some(at) = self.running.iter().position(|j| j == job) {
            self.running.swap_remove(at);
        }
        if !self.running.contains(job) {
            self.dropped.retain(|j| j != job);
        }
    }
}

/// Lock the queue. Nothing panics while holding it, but if something ever did, the queue
/// is still usable, and stopping the worker (in `Drop`) must never panic in turn.
fn queue(lock: &Mutex<Queue>) -> MutexGuard<'_, Queue> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct Worker {
    shared: Arc<(Mutex<Queue>, Condvar)>,
    pub rx: Receiver<Response>,
}

impl Worker {
    pub fn spawn(ctx: eframe::egui::Context, bytes: Vec<u8>, colours: Colours) -> Self {
        let shared = Arc::new((
            Mutex::new(Queue {
                items: Vec::new(),
                running: Vec::new(),
                dropped: Vec::new(),
                colours,
                closed: false,
            }),
            Condvar::new(),
        ));
        let (tx, rx) = channel::<Response>();
        let parsed = Arc::new(render::load(bytes, None).map_err(|e| e.to_string()));
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .saturating_sub(1)
            .clamp(1, 6);
        for _ in 0..threads {
            let shared = shared.clone();
            let tx: Sender<Response> = tx.clone();
            let parsed = parsed.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let renderer = match &*parsed {
                    Ok(pdf) => Some(Renderer::new(pdf)),
                    Err(_) => None,
                };
                loop {
                    let (job, colours) = {
                        let (lock, cv) = &*shared;
                        let mut q = queue(lock);
                        loop {
                            if q.closed {
                                return;
                            }
                            if let Some(j) = q.items.pop() {
                                let colours = q.colours;
                                q.running.push((j.0, colours));
                                break (j, colours);
                            }
                            q = cv.wait(q).unwrap_or_else(PoisonError::into_inner);
                        }
                    };
                    let (key, scale) = job;
                    let stop = || queue(&shared.0).stopped(&(key, colours));
                    let bitmap = match (&renderer, &*parsed) {
                        (Some(r), _) => {
                            // A malformed page must not take the whole viewer down.
                            let (mode, keep) = (colours.mode, !colours.pictures_too);
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match key.2 {
                                None => r.render_in(key.0, scale, mode, keep, &stop),
                                Some((c, row)) => {
                                    let region = Region {
                                        x: c * TILE,
                                        y: row * TILE,
                                        width: TILE,
                                        height: TILE,
                                    };
                                    r.render_region_in(key.0, scale, region, mode, keep)
                                        .map(Some)
                                }
                            }))
                            .map_err(|_| "renderer panicked on this page".to_string())
                            .and_then(|r| r.map_err(|e| e.to_string()))
                        }
                        (None, Err(e)) => Err(e.clone()),
                        _ => Err("renderer unavailable".into()),
                    };
                    queue(&shared.0).finished(&(key, colours));
                    // Stopped: what couldn't be drawn is reported with the next job.
                    let bitmap = match bitmap.transpose() {
                        Some(b) => b,
                        None => continue,
                    };
                    let missing = renderer.as_ref().map(|r| r.take_missing());
                    let r = Response {
                        key,
                        colours,
                        bitmap,
                        missing: missing.unwrap_or_default(),
                    };
                    if tx.send(r).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            });
        }
        Self { shared, rx }
    }

    pub fn request(&self, key: Key, scale: f32) {
        let (lock, cv) = &*self.shared;
        let mut q = queue(lock);
        // Wanted again: a drawing of it under way in these colours finishes after all.
        let job = (key, q.colours);
        q.dropped.retain(|j| *j != job);
        q.items.push((key, scale));
        cv.notify_one();
    }

    /// Draw pages from now on in `colours`, dropping the queued jobs and stopping the
    /// running ones.
    pub fn set_colours(&self, colours: Colours) {
        let mut q = queue(&self.shared.0);
        q.colours = colours;
        q.items.clear();
        q.drop_running(|_| true);
    }

    /// Drop the jobs that are no longer wanted, queued or running.
    pub fn retain(&self, keep: impl Fn(&Key) -> bool) {
        let mut q = queue(&self.shared.0);
        q.items.retain(|(k, _)| keep(k));
        q.drop_running(|(k, _)| !keep(k));
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let (lock, cv) = &*self.shared;
        queue(lock).closed = true;
        cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_with_a_poisoned_queue() {
        let bytes = limarepdf_core::Pdf::new().app_snapshot().unwrap();
        let w = Worker::spawn(eframe::egui::Context::default(), bytes, Colours::default());
        let shared = w.shared.clone();
        let _ = std::thread::spawn(move || {
            let _q = shared.0.lock().unwrap();
            panic!("poison the queue");
        })
        .join();
        assert!(w.shared.0.is_poisoned());
        w.request((0, 100, None), 1.0);
        drop(w);
    }

    #[test]
    fn running_jobs_stop_when_no_longer_wanted() {
        let mut q = Queue {
            items: Vec::new(),
            running: Vec::new(),
            dropped: Vec::new(),
            colours: Colours::default(),
            closed: false,
        };
        let dark = Colours {
            mode: PageColours::Dark,
            pictures_too: false,
        };
        let (a, b) = (((0, 20, None), dark), ((0, 30, None), dark));
        q.running.extend([a, b, b]);
        // Dropped once however often it is asked.
        q.drop_running(|j| j.0 .1 == 30);
        q.drop_running(|j| j.0 .1 == 30);
        assert_eq!(q.dropped, [b]);
        assert!(q.stopped(&b) && !q.stopped(&a));
        // The same key in other colours is another job.
        assert!(!q.stopped(&(b.0, Colours::default())));
        // Stopped until the last thread drawing it is done.
        q.finished(&b);
        assert!(q.stopped(&b));
        q.finished(&b);
        assert!(q.dropped.is_empty() && q.running == [a]);
        // Closing stops everything.
        q.closed = true;
        assert!(q.stopped(&a));
    }

    #[test]
    fn requests_and_colours_decide_what_keeps_running() {
        let w = Worker::spawn(
            eframe::egui::Context::default(),
            limarepdf_core::Pdf::new().app_snapshot().unwrap(),
            Colours::default(),
        );
        // Stand in for a thread drawing a page, without drawing it.
        let key = (0, 20, None);
        let job = (key, Colours::default());
        queue(&w.shared.0).running.push(job);
        w.retain(|k| *k != key);
        assert!(queue(&w.shared.0).stopped(&job));
        // Wanted again in the same colours: it finishes after all.
        w.request(key, 1.0);
        assert!(!queue(&w.shared.0).stopped(&job));
        // New colours stop it, and asking for the key again doesn't bring it back.
        let dark = Colours {
            mode: PageColours::Dark,
            pictures_too: false,
        };
        w.set_colours(dark);
        w.request(key, 1.0);
        assert!(queue(&w.shared.0).stopped(&job));
        queue(&w.shared.0).finished(&job);
    }
}
