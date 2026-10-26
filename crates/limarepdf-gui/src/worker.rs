//! Background page rendering. One parsed document is shared by several threads;
//! each thread owns its own (non-Send) render cache. Newest requests are served first
//! so scrolling stays responsive.

use crate::timing;
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

/// The colour around a part of a page, asked for with [`Worker::sample`].
pub struct Sampled {
    pub ask: Sample,
    pub colours: Colours,
    /// The most common colour in the ring; `None` when the page couldn't be drawn there.
    pub colour: Option<[u8; 3]>,
}

/// A part of a page whose surrounding colour is wanted: `region` of page `page` drawn at
/// `scale`, and the ring of it outside `inner` (pixels from the region's top left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub page: usize,
    pub scale: f32,
    pub region: Region,
    pub inner: [u32; 4],
}

impl Sample {
    /// The ring `pad` points wide around `bbox` (displayed-page points) on `page`.
    pub fn around(page: usize, bbox: [f32; 4], pad: f32) -> Self {
        // Enough pixels to see thin rules; small enough to draw at once.
        const SCALE: f32 = 2.0;
        let px = |v: f32| (v * SCALE).max(0.0).round() as u32;
        let (x, y) = (px(bbox[0] - pad), px(bbox[1] - pad));
        let (x1, y1) = (px(bbox[2] + pad), px(bbox[3] + pad));
        let gap = |v: f32| v.max(0.0).round() as u32;
        let inner = [
            gap((bbox[0] * SCALE) - x as f32),
            gap((bbox[1] * SCALE) - y as f32),
            gap((bbox[2] * SCALE) - x as f32),
            gap((bbox[3] * SCALE) - y as f32),
        ];
        Sample {
            page,
            scale: SCALE,
            region: Region {
                x,
                y,
                width: x1.saturating_sub(x).max(1),
                height: y1.saturating_sub(y).max(1),
            },
            inner,
        }
    }
}

/// The most common colour of `b` outside `inner` (left, top, right, bottom in pixels),
/// colours within a few steps of each other counted as one.
pub fn ring_colour(b: &Bitmap, inner: [u32; 4]) -> Option<[u8; 3]> {
    let mut counts: std::collections::HashMap<[u8; 3], (u32, [u32; 3])> = Default::default();
    for y in 0..b.height {
        for x in 0..b.width {
            let (xi, yi) = (x as u32, y as u32);
            if xi >= inner[0] && xi < inner[2] && yi >= inner[1] && yi < inner[3] {
                continue;
            }
            let i = (y * b.width + x) * 4;
            let a = b.rgba[i + 3] as u32;
            if a == 0 {
                continue;
            }
            // The bitmap is premultiplied; pages are opaque, but stay correct if not.
            let px = [0, 1, 2].map(|c| (b.rgba[i + c] as u32 * 255 / a).min(255));
            let e = counts.entry(px.map(|c| (c >> 3) as u8)).or_default();
            e.0 += 1;
            for (sum, c) in e.1.iter_mut().zip(px) {
                *sum += c;
            }
        }
    }
    let (n, sum) = counts.into_values().max_by_key(|(n, _)| *n)?;
    Some(sum.map(|s| (s / n) as u8))
}

struct Queue {
    items: Vec<(Key, f32)>,
    /// Colours to find, served before pages: a line being edited waits on one.
    samples: Vec<Sample>,
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
    pub samples: Receiver<Sampled>,
    /// The last colour asked of this worker; a new worker (the document changed) has none.
    pub asked: Option<(Sample, Colours)>,
}

impl Worker {
    pub fn spawn(ctx: eframe::egui::Context, bytes: Vec<u8>, colours: Colours) -> Self {
        let shared = Arc::new((
            Mutex::new(Queue {
                items: Vec::new(),
                samples: Vec::new(),
                running: Vec::new(),
                dropped: Vec::new(),
                colours,
                closed: false,
            }),
            Condvar::new(),
        ));
        let (tx, rx) = channel::<Response>();
        let (stx, samples) = channel::<Sampled>();
        let parsed = Arc::new(render::load(bytes, None).map_err(|e| e.to_string()));
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .saturating_sub(1)
            .clamp(1, 6);
        for _ in 0..threads {
            let shared = shared.clone();
            let tx: Sender<Response> = tx.clone();
            let stx: Sender<Sampled> = stx.clone();
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
                            if let Some(ask) = q.samples.pop() {
                                let colours = q.colours;
                                drop(q);
                                let (mode, keep) = (colours.mode, !colours.pictures_too);
                                let colour = renderer.as_ref().and_then(|r| {
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        r.render_region_in(
                                            ask.page, ask.scale, ask.region, mode, keep,
                                        )
                                    }))
                                    .ok()?
                                    .ok()
                                    .and_then(|b| ring_colour(&b, ask.inner))
                                });
                                if stx
                                    .send(Sampled {
                                        ask,
                                        colours,
                                        colour,
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                                ctx.request_repaint();
                                q = queue(lock);
                                continue;
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
                    // Page (from 1), quantised scale (0 for a thumbnail) and tile, if one.
                    let what = || match key.2 {
                        None => format!("page {} at {}", key.0 + 1, key.1),
                        Some((c, row)) => format!("page {} at {} tile {c},{row}", key.0 + 1, key.1),
                    };
                    timing::mark("render start", what);
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
                    timing::mark("render end", || match &bitmap {
                        Ok(Some(_)) => what(),
                        Ok(None) => format!("{} stopped", what()),
                        Err(_) => format!("{} failed", what()),
                    });
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
        Self {
            shared,
            rx,
            samples,
            asked: None,
        }
    }

    /// Find the colour around part of a page (see [`Sample`]); the answer comes on
    /// `samples`. Only the latest question is kept.
    pub fn sample(&mut self, ask: Sample) {
        let (lock, cv) = &*self.shared;
        let mut q = queue(lock);
        q.samples.clear();
        q.samples.push(ask);
        self.asked = Some((ask, q.colours));
        cv.notify_one();
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
            samples: Vec::new(),
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

    #[test]
    fn the_ring_colour_is_the_most_common_one_outside_the_box() {
        // 10 × 6 of blue, a white box inside, and a few red pixels in the ring.
        let (w, h) = (10, 6);
        let mut rgba = [20u8, 40, 200, 255].repeat(w * h);
        let mut put = |x: usize, y: usize, c: [u8; 4]| {
            rgba[(y * w + x) * 4..][..4].copy_from_slice(&c);
        };
        for y in 2..4 {
            for x in 3..7 {
                put(x, y, [255; 4]);
            }
        }
        for x in 0..5 {
            put(x, 0, [255, 0, 0, 255]);
        }
        let b = Bitmap {
            width: w,
            height: h,
            rgba,
        };
        assert_eq!(ring_colour(&b, [3, 2, 7, 4]), Some([20, 40, 200]));
        // Inside the box doesn't count, however much of it there is.
        assert_eq!(ring_colour(&b, [0, 0, 10, 6]), None);
    }

    #[test]
    fn a_sample_is_the_ring_around_a_box() {
        let s = Sample::around(2, [10.0, 20.0, 50.0, 30.0], 3.0);
        assert_eq!(
            (s.page, s.region),
            (
                2,
                Region {
                    x: 14,
                    y: 34,
                    width: 92,
                    height: 32
                }
            )
        );
        assert_eq!(s.inner, [6, 6, 86, 26]);
        // Near the page's corner the region starts at it.
        let s = Sample::around(0, [1.0, 1.0, 5.0, 5.0], 3.0);
        assert_eq!((s.region.x, s.region.y), (0, 0));
        assert_eq!(s.inner, [2, 2, 10, 10]);
    }

    #[test]
    fn the_colour_around_white_text_on_a_bar_is_the_bar() {
        let mut pdf = limarepdf_core::Pdf::new();
        pdf.insert_blank(0, 200.0, 100.0).unwrap();
        pdf.add_shape(
            0,
            limarepdf_core::Shape::Rect,
            [10.0, 10.0, 190.0, 60.0],
            &limarepdf_core::PathStyle {
                fill: Some([0.0, 0.0, 1.0]),
                stroke: None,
                line_width: 1.0,
            },
        )
        .unwrap();
        let font = limarepdf_core::FontChoice::default();
        pdf.add_text(0, (20.0, 40.0), "Walrus", font, 20.0, [1.0; 3])
            .unwrap();
        let line = pdf
            .page_objects(0)
            .unwrap()
            .into_iter()
            .find_map(|o| o.text.is_some().then_some(o.bbox));
        let ask = Sample::around(0, line.unwrap(), 3.0);
        for (mode, want) in [
            (PageColours::Normal, [0, 0, 255]),
            (PageColours::Sepia, PageColours::Sepia.pixel([0, 0, 255])),
        ] {
            let colours = Colours {
                mode,
                pictures_too: false,
            };
            let mut w = Worker::spawn(
                eframe::egui::Context::default(),
                pdf.app_snapshot().unwrap(),
                colours,
            );
            w.sample(ask);
            assert_eq!(w.asked, Some((ask, colours)));
            let got = w
                .samples
                .recv_timeout(std::time::Duration::from_secs(20))
                .unwrap();
            assert_eq!((got.ask, got.colours), (ask, colours));
            let c = got.colour.unwrap();
            assert!(
                (0..3).all(|i| c[i].abs_diff(want[i]) <= 3),
                "{mode:?}: {c:?}"
            );
        }
    }
}
