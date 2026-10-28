//! The pages drawn with the stamp being set put on (ENH-23), as a printer's proof shows
//! them. While the inspector's Watermark or Headers, footers, Bates tool is open and its
//! stamp isn't added yet, once its settings settle (nothing dragged, typed or picked), a
//! copy of the document has the stamp put on by core's own code, off the window's thread,
//! and a page worker of its own draws the pages and thumbnails in view from it. Those
//! drawings are shown in place of the pages' own, so a stamp set behind the content shows
//! behind it, as adding it makes the pages. A drawing for settings since changed is never
//! shown: while the settings are being changed, and until the drawing for them comes, the
//! pages show the drawn previews (`previews.rs`). The document, its undo, its revision and
//! whether it has unsaved changes never change.

use super::{stamp_onto, StampForm, Tab, BASE, THUMB_W};
use crate::inspector;
use crate::timing;
use crate::worker::{Colours, Key, Worker, TILE};
use anyhow::Context as _;
use eframe::egui::{self, Color32, Pos2, Rect, TextureHandle, TextureOptions, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, TryRecvError};

/// All of a texture.
const WHOLE: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

/// What a proof is drawn from: the document at a revision, in the colours pages are drawn
/// in, with each stamp put on its pages.
#[derive(Clone, PartialEq)]
struct Ask {
    revision: u64,
    colours: Colours,
    stamps: Vec<(StampForm, Vec<usize>)>,
}

/// The pages drawn with the stamps put on, for one [`Ask`].
struct Proof {
    ask: Ask,
    /// `None` when the copy couldn't be made: the pages keep the drawn previews.
    worker: Option<Worker>,
    /// Each page's drawing and its quantised scale, as `Tab::pages`.
    pages: HashMap<usize, (u32, TextureHandle)>,
    /// At high zoom, the parts of the pages in view, as `Tab::tiles`.
    tiles: HashMap<Key, TextureHandle>,
    thumbs: HashMap<usize, TextureHandle>,
    pending: HashSet<Key>,
    /// What couldn't be drawn: not asked for again, and the drawn preview stays.
    failed: HashSet<Key>,
    /// What was shown or asked for since the worker's queue was last trimmed.
    seen: HashSet<Key>,
}

/// A tab's proof, and the copy being made for the next one.
#[derive(Default)]
pub(super) struct Proofs {
    making: Option<(Ask, Receiver<Result<Worker, String>>)>,
    made: Option<Proof>,
}

/// The watermark picture's file for `form`, if it has one: read again for each copy, as
/// `Tab::put_stamp` reads it.
fn picture(form: &StampForm) -> limarepdf_core::Result<Option<Vec<u8>>> {
    let StampForm::Wm(w) = form else {
        return Ok(None);
    };
    if !w.picture {
        return Ok(None);
    }
    let path = w.image.as_ref().context("choose a picture first")?;
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(bytes))
}

impl Proof {
    fn new(ask: Ask, worker: Option<Worker>) -> Self {
        Self {
            ask,
            worker,
            pages: HashMap::new(),
            tiles: HashMap::new(),
            thumbs: HashMap::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            seen: HashSet::new(),
        }
    }

    /// Whether page `page` has a stamp put on.
    fn stamps(&self, page: usize) -> bool {
        let stamps = &self.ask.stamps;
        stamps.iter().any(|(_, pages)| pages.contains(&page))
    }

    /// Ask for `key`, drawn at `scale`, unless it is drawn, being drawn or couldn't be.
    fn want(&mut self, key: Key, scale: f32) {
        self.seen.insert(key);
        let drawn = match key {
            (page, 0, None) => self.thumbs.contains_key(&page),
            (page, q, None) => self.pages.get(&page).is_some_and(|p| p.0 == q),
            tile => self.tiles.contains_key(&tile),
        };
        if drawn || self.failed.contains(&key) || !self.pending.insert(key) {
            return;
        }
        if let Some(w) = &self.worker {
            w.request(key, scale);
        }
    }

    /// Take in the drawings that came.
    fn collect(&mut self, ctx: &egui::Context, tab: u64) {
        let Some(w) = &self.worker else {
            return;
        };
        while let Ok(r) = w.rx.try_recv() {
            self.pending.remove(&r.key);
            // What couldn't be drawn (`r.missing`) is the document's, and its own drawings
            // report it.
            let Ok(b) = r.bitmap else {
                self.failed.insert(r.key);
                continue;
            };
            let img = egui::ColorImage::from_rgba_premultiplied([b.width, b.height], &b.rgba);
            let (page, q, tile) = r.key;
            let name = format!("proof{tab}-{page}-{q}-{tile:?}");
            let tex = ctx.load_texture(name, img, TextureOptions::LINEAR);
            match tile {
                Some(_) => {
                    self.tiles.insert(r.key, tex);
                }
                None if q == 0 => {
                    self.thumbs.insert(page, tex);
                }
                None => {
                    self.pages.insert(page, (q, tex));
                }
            }
        }
    }

    /// Drop the jobs and drawings no longer wanted: those neither shown nor asked for
    /// since the last time, but for the thumbnails and the pages the tab keeps its own
    /// drawing of (`kept`).
    fn trim(&mut self, kept: impl Fn(usize) -> bool) {
        let seen = std::mem::take(&mut self.seen);
        if let Some(w) = &self.worker {
            w.retain(|k| seen.contains(k));
        }
        self.pending.retain(|k| seen.contains(k));
        self.tiles.retain(|k, _| seen.contains(k));
        let shown = |p: usize| seen.iter().any(|k| k.0 == p && k.1 != 0 && k.2.is_none());
        self.pages.retain(|&p, _| kept(p) || shown(p));
    }

    /// Draw the tiles of page `page` (`w` × `h` points, shown at `r`) at quantised scale
    /// `q` in the view of `painter`, over its drawing, asking for those missing: as
    /// `Tab::draw_tiles`, without the ring around the view or the tiles of an earlier zoom.
    fn draw_tiles(
        &mut self,
        painter: &egui::Painter,
        page: usize,
        (w, h): (f32, f32),
        r: Rect,
        q: u32,
    ) {
        let vis = painter.clip_rect().intersect(r);
        if !vis.is_positive() {
            return;
        }
        let s = q as f32 / 20.0;
        // Screen points per tile pixel, and the tiles across and down the page.
        let k = r.width() / (w * s).max(1.0);
        let cols = ((w * s) as u32).div_ceil(TILE).max(1);
        let rows = ((h * s) as u32).div_ceil(TILE).max(1);
        let span = |a: f32, b: f32, n: u32| {
            let t = TILE as f32 * k;
            let lo = ((a / t).floor().max(0.0) as u32).min(n - 1);
            let hi = (((b / t).ceil() as u32).max(1) - 1).min(n - 1);
            lo..=hi
        };
        let across = span(vis.min.x - r.min.x, vis.max.x - r.min.x, cols);
        for row in span(vis.min.y - r.min.y, vis.max.y - r.min.y, rows) {
            for c in across.clone() {
                let key = (page, q, Some((c, row)));
                self.want(key, s);
                if let Some(tex) = self.tiles.get(&key) {
                    let [tw, th] = tex.size();
                    let at = r.min + Vec2::new((c * TILE) as f32, (row * TILE) as f32) * k;
                    let size = Vec2::new(tw as f32, th as f32) * k;
                    let at = Rect::from_min_size(at, size);
                    painter.image(tex.id(), at, WHOLE, Color32::WHITE);
                }
            }
        }
    }
}

impl Tab {
    /// What a proof of `stamps` is drawn from now.
    fn proof_of(&self, stamps: Vec<(StampForm, Vec<usize>)>) -> Ask {
        Ask {
            revision: self.revision,
            colours: self.colours,
            stamps,
        }
    }

    /// Bring the proof up to date for `stamps`, the stamps the pages show with their pages
    /// (none: the tool closed or the stamp added, and the proof is dropped): the copy made
    /// taken in if it is still for them, the drawings that came collected, the jobs no
    /// longer wanted dropped, and, once the settings settle, a copy made for them. Run
    /// once a frame, before the pages are drawn. Returns whether there is a proof for the
    /// stamps as they are; one for anything else is dropped, never shown.
    pub(super) fn sync_proofs(
        &mut self,
        ctx: &egui::Context,
        stamps: Vec<(StampForm, Vec<usize>)>,
    ) -> bool {
        if stamps.is_empty() {
            self.proofs = Proofs::default();
            return false;
        }
        let ask = self.proof_of(stamps);
        let p = &mut self.proofs;
        let came = p.making.as_ref().and_then(|(_, rx)| match rx.try_recv() {
            Ok(made) => Some(made),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the thread making it stopped".into())),
        });
        if let Some(made) = came {
            let made_for = p.making.take().map(|m| m.0);
            // A copy that couldn't be made isn't reported: the pages keep the drawn
            // previews, and adding the stamp says what is wrong.
            if let Err(why) = &made {
                timing::mark("stamp preview failed", || why.clone());
            }
            if made_for.as_ref() == Some(&ask) {
                p.made = Some(Proof::new(ask.clone(), made.ok()));
            }
        }
        if p.made.as_ref().is_some_and(|m| m.ask != ask) {
            p.made = None;
        }
        // Settled settings with no proof: a copy for them, once the one being made (for
        // settings passed through) is done.
        if p.made.is_none() && p.making.is_none() && !inspector::changing(ctx) {
            let stamps = ask.stamps.clone();
            let mut copy = self.pdf.clone();
            let rx = Worker::spawn_from(ctx.clone(), self.colours, move || {
                for (form, pages) in &stamps {
                    stamp_onto(&mut copy, form, pages, picture(form)?)?;
                }
                copy.app_snapshot()
            });
            p.making = Some((ask, rx));
        }
        let Some(m) = p.made.as_mut() else {
            return false;
        };
        m.collect(ctx, self.id);
        let own = &self.pages;
        m.trim(|page| own.contains_key(&page));
        true
    }

    /// Whether there is a proof for the stamps as the forms are now, for the thumbnails:
    /// they are drawn before the inspector and the pages, after the last frame's commands.
    pub(super) fn proof_shown(&self, ctx: &egui::Context) -> bool {
        let Some(m) = &self.proofs.made else {
            return false;
        };
        m.ask == self.proof_of(self.stamping(ctx.cumulative_pass_nr()))
    }

    /// Draw page `page` at `r` from the proof, over its own drawing and its tiles, asking
    /// for what the proof lacks. False when the proof has no drawing of the page yet, or
    /// can't have one: the drawn previews are shown instead.
    pub(super) fn proof_page(&mut self, painter: &egui::Painter, page: usize, r: Rect) -> bool {
        let ctx = painter.ctx();
        let scale = self.zoom * BASE * ctx.pixels_per_point();
        let (key, key_scale, tiles) = self.page_key(ctx, page, scale);
        let size = self.sizes.get(page).copied().unwrap_or((1.0, 1.0));
        let Some(m) = self.proofs.made.as_mut().filter(|m| m.stamps(page)) else {
            return false;
        };
        m.want(key, key_scale);
        let Some(tex) = m.pages.get(&page).map(|p| p.1.id()) else {
            return false;
        };
        painter.image(tex, r, WHOLE, Color32::WHITE);
        if let Some(q) = tiles {
            m.draw_tiles(painter, page, size, r, q);
        }
        true
    }

    /// Draw page `page`'s thumbnail at `r` from the proof, when the page has a stamp put
    /// on, asking for it until it comes. Only while [`Tab::proof_shown`].
    pub(super) fn proof_thumb(&mut self, painter: &egui::Painter, page: usize, r: Rect) {
        let ppp = painter.ctx().pixels_per_point();
        let w = self.sizes.get(page).map_or(1.0, |s| s.0);
        let Some(m) = self.proofs.made.as_mut().filter(|m| m.stamps(page)) else {
            return;
        };
        m.want((page, 0, None), THUMB_W * ppp / w.max(1.0));
        if let Some(t) = m.thumbs.get(&page) {
            painter.image(t.id(), r, WHOLE, Color32::WHITE);
        }
    }
}
