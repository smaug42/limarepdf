//! Background page rendering. One parsed document is shared by several threads;
//! each thread owns its own (non-Send) render cache. Newest requests are served first
//! so scrolling stays responsive.

use limarepdf_core::page_colours::PageColours;
use limarepdf_core::render::{self, Bitmap, Missing, Renderer};
use std::collections::BTreeSet;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

/// (page index, quantised scale)
pub type Key = (usize, u32);

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
    colours: Colours,
    closed: bool,
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
                        let mut q = lock.lock().unwrap();
                        loop {
                            if q.closed {
                                return;
                            }
                            if let Some(j) = q.items.pop() {
                                break (j, q.colours);
                            }
                            q = cv.wait(q).unwrap();
                        }
                    };
                    let (key, scale) = job;
                    let bitmap = match (&renderer, &*parsed) {
                        (Some(r), _) => {
                            // A malformed page must not take the whole viewer down.
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                r.render_in(key.0, scale, colours.mode, !colours.pictures_too)
                            }))
                            .map_err(|_| "renderer panicked on this page".to_string())
                            .and_then(|r| r.map_err(|e| e.to_string()))
                        }
                        (None, Err(e)) => Err(e.clone()),
                        _ => Err("renderer unavailable".into()),
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
        lock.lock().unwrap().items.push((key, scale));
        cv.notify_one();
    }

    /// Draw pages from now on in `colours`, dropping the queued jobs.
    pub fn set_colours(&self, colours: Colours) {
        let mut q = self.shared.0.lock().unwrap();
        q.colours = colours;
        q.items.clear();
    }

    /// Drop queued jobs that are no longer wanted.
    pub fn retain(&self, keep: impl Fn(&Key) -> bool) {
        self.shared.0.lock().unwrap().items.retain(|(k, _)| keep(k));
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let (lock, cv) = &*self.shared;
        lock.lock().unwrap().closed = true;
        cv.notify_all();
    }
}
