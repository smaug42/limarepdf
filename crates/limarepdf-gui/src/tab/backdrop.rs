//! The page drawn without the text being edited (ENH-21), shown under the typing so that
//! what is behind the old text (a photo, a gradient, a coloured box) shows through, not
//! one colour. Core makes the page alone without the old text's objects
//! ([`limarepdf_core::Pdf::page_without`]) once for each text edited; a thread of its own
//! draws the part the editor hides at the scale the page is shown at and in its colours,
//! and draws it again from the same page after a zoom or a change of colours, never as
//! the text is typed. Until the drawing comes, and should it fail, the editor hides the old
//! text in the colour around it (`Tab::sync_cover`); after a zoom the last drawing is
//! shown stretched until the new one comes.

use super::{Tab, BASE};
use crate::timing;
use crate::worker::Colours;
use eframe::egui::{self, TextureHandle, TextureOptions};
use limarepdf_core::render::{self, Bitmap, Region, Renderer};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

/// The longest side of a drawing, in pixels: a larger part of the page at a high zoom is
/// drawn at a lower scale and stretched.
const MOST: f32 = 4096.0;

/// What the editor hides: the page, the objects it is without, the box drawn (page
/// points), and the document's revision they are of.
type Hidden = (usize, Vec<usize>, [f32; 4], u64);

/// A drawing asked for: its scale (pixels a point) and colours.
type Ask = (f32, Colours);

/// A drawing's bitmap, or why it couldn't be drawn.
type Drawn = Result<Bitmap, String>;

pub(super) struct Backdrop {
    hidden: Hidden,
    /// The page alone without the objects; `None` when it couldn't be made.
    page: Option<Arc<Vec<u8>>>,
    /// The drawing under way: what it was asked at, the region drawn, and its answer.
    drawing: Option<(Ask, Region, Receiver<Drawn>)>,
    /// The last drawing that came: what it was asked at, its picture, and where it goes
    /// on the page (points).
    shown: Option<(Ask, TextureHandle, [f32; 4])>,
    /// The last drawing that failed, not asked for again.
    failed: Option<Ask>,
}

impl Tab {
    /// Ask for the page drawn without the text the editor hides, as it changes, and give
    /// the editor the latest drawing in the colours shown.
    pub(super) fn sync_backdrop(&mut self, ctx: &egui::Context) {
        let wanted = (self.ed.backdrop_wanted()).map(|(p, ids, b)| (p, ids, b, self.revision));
        if self.backdrop.as_ref().map(|b| &b.hidden) != wanted.as_ref() {
            self.backdrop = wanted.map(|hidden| {
                let page = (self.pdf.page_without(hidden.0, &hidden.1))
                    .inspect_err(|e| timing::mark("backdrop failed", || format!("{e:#}")))
                    .ok()
                    .map(Arc::new);
                Backdrop {
                    hidden,
                    page,
                    drawing: None,
                    shown: None,
                    failed: None,
                }
            });
        }
        let Some((page, b)) = self.backdrop.as_ref().map(|bd| (bd.hidden.0, bd.hidden.2)) else {
            self.ed.backdrop = None;
            return;
        };
        // At the scale the page is drawn at on the screen: its tiles' when it is in tiles.
        let ppp = ctx.pixels_per_point();
        let (_, whole, tiles) = self.page_key(ctx, page, self.zoom * BASE * ppp);
        let scale = tiles.map_or(whole, |t| t as f32 / 20.0);
        let side = (b[2] - b[0]).max(b[3] - b[1]).max(1.0);
        let ask = (scale.min(MOST / side), self.colours);
        let Some(bd) = self.backdrop.as_mut() else {
            return;
        };
        let answer =
            (bd.drawing.as_ref()).map(|(asked, region, rx)| (*asked, *region, rx.try_recv()));
        match answer {
            Some((asked, region, Ok(Ok(bitmap)))) => {
                let img = egui::ColorImage::from_rgba_premultiplied(
                    [bitmap.width, bitmap.height],
                    &bitmap.rgba,
                );
                let name = format!("backdrop{}", self.id);
                let tex = ctx.load_texture(name, img, TextureOptions::LINEAR);
                let s = asked.0;
                let (x, y) = (region.x as f32 / s, region.y as f32 / s);
                let (w, h) = (bitmap.width as f32 / s, bitmap.height as f32 / s);
                bd.shown = Some((asked, tex, [x, y, x + w, y + h]));
                bd.drawing = None;
            }
            Some((asked, _, Ok(Err(e)))) => {
                timing::mark("backdrop failed", || e);
                bd.failed = Some(asked);
                bd.drawing = None;
            }
            Some((asked, _, Err(TryRecvError::Disconnected))) => {
                bd.failed = Some(asked);
                bd.drawing = None;
            }
            Some((_, _, Err(TryRecvError::Empty))) | None => {}
        }
        let current = bd.shown.as_ref().is_some_and(|s| s.0 == ask);
        if let (None, false, Some(bytes)) = (&bd.drawing, current, &bd.page) {
            if bd.failed != Some(ask) {
                let s = ask.0;
                let px = |v: f32| (v * s).max(0.0);
                let (x, y) = (px(b[0]).floor() as u32, px(b[1]).floor() as u32);
                let region = Region {
                    x,
                    y,
                    width: (px(b[2]).ceil() as u32).saturating_sub(x).max(1),
                    height: (px(b[3]).ceil() as u32).saturating_sub(y).max(1),
                };
                let rx = draw(ctx, bytes.clone(), s, region, ask.1);
                bd.drawing = Some((ask, region, rx));
            }
        }
        // The last drawing in the colours shown, at whatever scale: stretched after a zoom
        // until the new one comes.
        let shown = bd.shown.as_ref().filter(|s| s.0 .1 == self.colours);
        self.ed.backdrop = shown.map(|s| (s.1.id(), s.2));
    }
}

/// Draw `region` of the one page of `bytes` at `scale` in `colours`, on a thread of its
/// own; the bitmap, or why it couldn't be drawn, comes on the receiver.
fn draw(
    ctx: &egui::Context,
    bytes: Arc<Vec<u8>>,
    scale: f32,
    region: Region,
    colours: Colours,
) -> Receiver<Drawn> {
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let (mode, keep) = (colours.mode, !colours.pictures_too);
        let drawn = crate::guard(|| {
            let pdf = render::load(bytes.to_vec(), None)?;
            Renderer::new(&pdf).render_region_in(0, scale, region, mode, keep)
        });
        if tx.send(drawn.map_err(|e| format!("{e:#}"))).is_ok() {
            ctx.request_repaint();
        }
    });
    rx
}
