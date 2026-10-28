//! Organise (NEW-14): the document's pages as a grid of cards, shown in place of the pages,
//! to select, move, turn, take out and delete pages, as a presentation's slide sorter. Each
//! card is the page's picture with a checkbox and its name under it. Only the rows in view
//! are drawn, with their pictures, so a long document's grid is as quick as a short one's.

use super::{edge_scroll, Tab, BASE};
use crate::actions::Ctx;
use crate::icon;
use crate::selection::Selection;
use crate::settings::{ORGANISE_MAX, ORGANISE_MIN};
use crate::theme;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Vec2};
use std::collections::BTreeSet;

/// The room round a card's picture, inside the card, and between cards (the workspace's
/// mockup), the grid's margin, and the height of the row under a picture with its
/// checkbox and name.
const PAD: f32 = 12.0;
const BETWEEN: f32 = 14.0;
const MARGIN: f32 = 16.0;
const LABEL: f32 = 26.0;
/// Rows beyond those in view drawn ahead, and the rows whose pictures are kept.
const AHEAD: usize = 1;
const KEEP: usize = 2;

/// A tab's Organise state, while its pages show as the grid.
pub struct Organise {
    /// The page at the top of the page view when the grid opened, and how far down it the
    /// view was (points): the page view comes back there.
    top: Option<(u32, u16)>,
    offset: f32,
    /// A drag across the grid's background, selecting the cards it touches: where it
    /// began, in the grid's own points, and the pages selected before it, which it adds
    /// to with Ctrl or Shift held.
    sweep: Option<(Pos2, BTreeSet<usize>)>,
    /// Bring this card into view when next drawn.
    seek: Option<usize>,
    /// The first card in view last drawn, which stays at the top when the zoom changes.
    first: usize,
    rezoom: bool,
    /// The part of the grid in view last drawn, in its own points.
    view: Option<Rect>,
    /// The size of most pages, in points, and the revision it was found at: the cards'
    /// size.
    typical: (u64, Vec2),
}

/// The grid's layout: its cards' size, how many to a row, and where the first is.
#[derive(Clone, Copy)]
struct Cards {
    card: Vec2,
    cols: usize,
    left: f32,
}

impl Cards {
    /// The top left of card `i`, in the grid's own points.
    fn at(&self, i: usize) -> Vec2 {
        let (row, col) = (i / self.cols, i % self.cols);
        Vec2::new(
            self.left + col as f32 * (self.card.x + BETWEEN),
            MARGIN + row as f32 * (self.card.y + BETWEEN),
        )
    }

    /// The rows from `top` to `bottom` (the grid's own points), whole.
    fn rows(&self, top: f32, bottom: f32, rows: usize) -> (usize, usize) {
        let step = self.card.y + BETWEEN;
        let first = ((top - MARGIN) / step).floor().max(0.0) as usize;
        let last = ((bottom - MARGIN) / step).ceil().max(0.0) as usize;
        (first.min(rows), last.min(rows))
    }

    /// The cards a rectangle in the grid's own points touches, of `n`.
    fn touched(&self, r: Rect, n: usize) -> BTreeSet<usize> {
        let rows = n.div_ceil(self.cols);
        let (r0, r1) = self.rows(r.min.y, r.max.y, rows);
        let mut out = BTreeSet::new();
        for row in r0..r1 {
            for col in 0..self.cols {
                let i = row * self.cols + col;
                if i < n && Rect::from_min_size(Pos2::ZERO + self.at(i), self.card).intersects(r) {
                    out.insert(i);
                }
            }
        }
        out
    }
}

impl Tab {
    /// Whether the tab shows its pages as Organise's grid.
    pub fn organising(&self) -> bool {
        self.organise.is_some()
    }

    /// Show the pages as Organise's grid, or the pages again. What is being typed on a
    /// page goes into the document first; when it can't, the grid doesn't open (the error
    /// is said). Back from the grid, the page view is where it was, at the page then at
    /// its top wherever it went, or at the page being read.
    pub fn set_organise(&mut self, ctx: &egui::Context, on: bool) -> bool {
        if on == self.organising() {
            return true;
        }
        if !on {
            let Some(o) = self.organise.take() else {
                return true;
            };
            let top = o.top.and_then(|id| self.ids.iter().position(|&p| p == id));
            match top {
                Some(p) => {
                    self.goto = None;
                    self.rezoom = Some((p, o.offset));
                }
                None => self.go_to(self.current),
            }
            return true;
        }
        if !self.keep_page_typing(ctx) {
            return false;
        }
        // Words and objects are the page view's to select.
        if self.ed.selection.pages().is_none() {
            self.ed.selection = Selection::None;
        }
        let (top, offset) = self.view;
        self.organise = Some(Organise {
            top: self.ids.get(top).copied(),
            offset,
            sweep: None,
            seek: Some(self.current),
            first: 0,
            rezoom: false,
            view: None,
            typical: (u64::MAX, Vec2::ZERO),
        });
        true
    }

    /// Back to the page view, at page `p`: a card opened.
    pub fn open_page(&mut self, ctx: &egui::Context, p: usize) {
        self.set_organise(ctx, false);
        self.go_to(p);
    }

    /// Change the grid's zoom, keeping the first card in view at the top.
    pub fn zoom_grid(&mut self, z: f32) {
        let z = z.clamp(ORGANISE_MIN, ORGANISE_MAX);
        if (z - self.grid_zoom).abs() < 1e-4 {
            return;
        }
        self.grid_zoom = z;
        if let Some(o) = self.organise.as_mut() {
            o.rezoom = true;
        }
    }

    /// The size of most pages, in points: the median width and height, so that one large
    /// page among many doesn't make every card large. Each card is a page of this size,
    /// other pages fitted in it.
    fn typical_size(&mut self) -> Vec2 {
        let rev = self.revision;
        let Some(o) = self.organise.as_mut() else {
            return Vec2::new(612.0, 792.0);
        };
        if o.typical.0 != rev {
            let median = |side: fn(&(f32, f32)) -> f32| {
                let mut all: Vec<f32> = self.sizes.iter().map(side).collect();
                all.sort_by(f32::total_cmp);
                all.get(all.len() / 2).copied().unwrap_or(792.0).max(1.0)
            };
            o.typical = (rev, Vec2::new(median(|s| s.0), median(|s| s.1)));
        }
        o.typical.1
    }

    /// Organise's grid, in place of the pages.
    pub fn show_grid(&mut self, ui: &mut egui::Ui, base: &Ctx) {
        let ctx = ui.ctx().clone();
        self.collect(&ctx);
        self.poll_search(&ctx);
        self.banners_ui(ui);
        let zd = ui.input(|i| i.zoom_delta());
        if zd != 1.0 && ui.ui_contains_pointer() {
            self.zoom_grid(self.grid_zoom * zd);
        }
        let n = self.sizes.len();
        let dark = ui.visuals().dark_mode;
        self.grid_heading(ui, n);

        let frame = self.grid_zoom * BASE * self.typical_size();
        let card = frame + Vec2::new(2.0 * PAD, 2.0 * PAD + LABEL);
        let width = ui.available_width();
        // From the left, as a slide sorter's: a card moves only when the cards in a row
        // change, not as the panels beside the grid change width a little.
        let fit = ((width - 2.0 * MARGIN + BETWEEN) / (card.x + BETWEEN)).floor();
        let cols = fit.max(1.0) as usize;
        let lay = Cards {
            card,
            cols,
            left: MARGIN,
        };
        let rows = n.div_ceil(cols);
        let height = 2.0 * MARGIN + rows as f32 * card.y + rows.saturating_sub(1) as f32 * BETWEEN;

        ui.style_mut().spacing.scroll.fade.strength = 0.0;
        let mut area = egui::ScrollArea::vertical()
            .id_salt(("grid", self.id))
            .auto_shrink([false, false]);
        // A page gone to (from the bookmarks or search, say) comes into view, scrolling
        // only as far as it must; a new zoom keeps the first card in view at the top.
        let o = self.organise.as_mut().expect("organising");
        let seek = o.seek.take().or(self.goto.take());
        if let Some(p) = seek.filter(|&p| p < n) {
            let top = lay.at(p).y;
            let bottom = top + card.y;
            let to = match o.view {
                Some(v) if top < v.min.y => Some(top - MARGIN),
                Some(v) if bottom > v.max.y => Some(bottom + MARGIN - v.height()),
                Some(_) => None,
                None => Some(top - MARGIN),
            };
            if let Some(to) = to {
                area = area.vertical_scroll_offset(to.max(0.0));
            }
        } else if std::mem::take(&mut o.rezoom) {
            let first = o.first.min(n.saturating_sub(1));
            area = area.vertical_scroll_offset((lay.at(first).y - MARGIN).max(0.0));
        }

        let ppp = ctx.pixels_per_point();
        let paper = self.paper();
        let proof = self.proof_shown(&ctx);
        let mut wanted = Vec::new();
        let (mut click, mut open, mut toggle) = (None, None, None);
        let mut kept = (0, 0);
        area.show_viewport(ui, |ui, view| {
            let origin = ui.max_rect().min;
            let room = Vec2::new(width, height.max(view.height()));
            ui.set_min_size(room);
            let to_grid = |p: Pos2| Pos2::ZERO + (p - origin);
            // The background first, so the cards over it take their own clicks.
            let bg = ui.interact(
                Rect::from_min_size(origin, room),
                ui.scope_id().with("grid-background"),
                Sense::click_and_drag(),
            );
            bg.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ListBox, true, "Page grid"));
            let (r0, r1) = lay.rows(view.min.y, view.max.y, rows);
            let (lo, hi) = (r0.saturating_sub(AHEAD), (r1 + AHEAD).min(rows));
            kept = (r0.saturating_sub(KEEP), (r1 + KEEP).min(rows));
            let o = self.organise.as_mut().expect("organising");
            o.view = Some(view);
            o.first = (r0 * cols).min(n.saturating_sub(1));
            let chosen = self.ed.selection.pages().cloned().unwrap_or_default();
            for i in lo * cols..(hi * cols).min(n) {
                let rect = Rect::from_min_size(origin + lay.at(i), card);
                // The page's picture, fitted in the card's frame, at its own size when
                // smaller.
                let (w, h) = self.sizes[i];
                let fit = (frame.x / w.max(1.0)).min(frame.y / h.max(1.0));
                let k = (self.grid_zoom * BASE).min(fit);
                let (key, scale, _) = self.page_key(&ctx, i, k * ppp);
                let have = self.pages.get(&i).map(|(q, _)| *q);
                let old = have != Some(key.1) || self.stale.contains(&i);
                if old && !self.failed.contains_key(&i) {
                    wanted.push((key, scale));
                }
                // The card and its checkbox take clicks whether or not they are drawn this
                // pass: a click egui is following on a widget left out of a pass (one laid
                // out again, as when the heading's count changes) is dropped. Neither is a
                // stop for Tab: the grid takes the keys for its cards.
                let name = format!("Page {}", self.page_name(i));
                let id = ui.scope_id().with(("card", self.ids.get(i).copied()));
                let resp = ui.interact(rect, id, Sense::CLICK | Sense::DRAG);
                let sel = chosen.contains(&i);
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::Button, true, sel, &name)
                });
                let holder = Rect::from_min_size(rect.min + Vec2::splat(PAD), frame);
                let row = Rect::from_min_size(
                    Pos2::new(rect.min.x + PAD, holder.max.y + 4.0),
                    Vec2::new(frame.x, LABEL - 4.0),
                );
                let check = Rect::from_center_size(
                    Pos2::new(row.min.x + 8.0, row.center().y),
                    Vec2::splat(16.0),
                );
                let cb = ui.interact(check, id.with("check"), Sense::CLICK);
                let what = format!("Select page {}", self.page_name(i));
                cb.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::CheckBox, true, sel, &what)
                });
                if cb.clicked() {
                    toggle = Some(i);
                }
                if resp.double_clicked() {
                    open = Some(i);
                } else if resp.clicked() {
                    click = Some((i, ui.input(|s| s.modifiers)));
                }
                // Right-click acts on this page, or on the selected pages if it is one.
                if resp.secondary_clicked() && !sel {
                    self.ed.selection = Selection::pages_from([i].into());
                    self.anchor = i;
                }
                self.context_menu(&resp, base);
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let (fill, edge) = match sel {
                    true => (theme::accent_bg(dark), theme::accent(dark)),
                    false => (theme::surface(dark), theme::border(dark)),
                };
                let p = ui.painter();
                p.rect_filled(rect, 5.0, fill);
                let line = egui::Stroke::new(1.0, edge);
                p.rect_stroke(rect, 5.0, line, egui::StrokeKind::Inside);
                let img = Rect::from_center_size(holder.center(), Vec2::new(w * k, h * k));
                p.rect_filled(img, 0.0, paper);
                let outline = egui::Stroke::new(1.0, theme::border(dark));
                p.rect_stroke(img, 0.0, outline, egui::StrokeKind::Outside);
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                match self.pages.get(&i) {
                    Some((_, t)) => {
                        p.image(t.id(), img, uv, Color32::WHITE);
                    }
                    None if self.failed.contains_key(&i) => {
                        let glyph = egui::FontId::new(24.0, theme::icons());
                        let (at, broken) = (egui::Align2::CENTER_CENTER, icon::IMAGE_BROKEN);
                        p.text(img.center(), at, broken, glyph, Color32::GRAY);
                    }
                    None => {}
                }
                if proof {
                    self.proof_thumb(ui.painter(), i, img);
                }
                // The checkbox, ticked white on the accent, and the page's name beside it.
                let look = ui.style().interact(&cb);
                let accent = theme::accent(dark);
                let (fill, edge) = match sel {
                    true => (accent, egui::Stroke::new(1.0, accent)),
                    false => (look.bg_fill, look.bg_stroke),
                };
                let corner = egui::CornerRadius::same(3);
                let inside = egui::StrokeKind::Inside;
                ui.painter()
                    .rect(check.shrink(1.0), corner, fill, edge, inside);
                if sel {
                    let r = check.shrink(4.5);
                    let tick = vec![
                        Pos2::new(r.left(), r.center().y),
                        Pos2::new(r.center().x - 1.0, r.bottom()),
                        Pos2::new(r.right(), r.top()),
                    ];
                    let white = egui::Stroke::new(1.8, Color32::WHITE);
                    ui.painter().line(tick, white);
                }
                ui.painter().text(
                    Pos2::new(check.max.x + 5.0, row.center().y),
                    egui::Align2::LEFT_CENTER,
                    &name,
                    egui::FontId::proportional(12.0),
                    ui.visuals().text_color(),
                );
            }

            // A drag across the background selects the cards it touches; with Ctrl or
            // Shift held, as well as those selected before.
            let o = self.organise.as_mut().expect("organising");
            if bg.drag_started() {
                let from = ui.input(|i| i.pointer.press_origin()).unwrap_or(origin);
                let m = ui.input(|i| i.modifiers);
                let before = match m.command || m.shift {
                    true => chosen.clone(),
                    false => BTreeSet::new(),
                };
                o.sweep = Some((to_grid(from), before));
            }
            let pointer = ui.input(|i| i.pointer.latest_pos());
            if let (Some((from, before)), Some(p)) = (&o.sweep, pointer) {
                let swept = Rect::from_two_pos(*from, to_grid(p));
                let mut set = before.clone();
                set.extend(lay.touched(swept, n));
                let on_screen = swept.translate(origin - Pos2::ZERO);
                let accent = theme::accent(dark);
                let (p, fill) = (ui.painter(), accent.gamma_multiply(0.15));
                p.rect_filled(on_screen, 0.0, fill);
                let line = egui::Stroke::new(1.0, accent);
                p.rect_stroke(on_screen, 0.0, line, egui::StrokeKind::Inside);
                self.ed.selection = Selection::pages_from(set);
                edge_scroll(ui);
            }
            let o = self.organise.as_mut().expect("organising");
            if bg.drag_stopped() || !ui.input(|i| i.pointer.any_down()) {
                o.sweep = None;
            }
            if bg.clicked() {
                self.ed.selection = Selection::None;
            }
        });

        if let Some(i) = open {
            self.open_page(&ctx, i);
            return;
        }
        if let Some(i) = toggle {
            self.toggle_page(i);
        }
        if let Some((i, m)) = click {
            self.click_page(i, m);
        }
        // Pictures for the rows in view and the next, the nearest first; those far from
        // the view let go, and not drawn.
        for (key, scale) in wanted {
            if self.pending.insert(key) {
                self.worker.request(key, scale);
            }
        }
        let (lo, hi) = (kept.0 * cols, (kept.1 * cols).min(n));
        self.pages.retain(|&p, _| p >= lo && p < hi);
        let keep = |k: &super::Key| k.2.is_none() && (k.1 == 0 || (k.0 >= lo && k.0 < hi));
        self.worker.retain(keep);
        self.pending.retain(keep);
    }

    /// The grid's heading: its name, and how many pages there are and are selected.
    fn grid_heading(&self, ui: &mut egui::Ui, n: usize) {
        let dark = ui.visuals().dark_mode;
        let chosen = self.ed.selection.pages().map_or(0, BTreeSet::len);
        let pages = match n {
            1 => "1 page".to_string(),
            n => format!("{n} pages"),
        };
        let count = match chosen {
            0 => pages,
            m => format!("{pages} · {m} selected"),
        };
        ui.add_space(10.0);
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(MARGIN as i8, 0))
            .show(ui, |ui| {
                egui::Sides::new().show(
                    ui,
                    |ui| {
                        let name = egui::RichText::new("Organise pages")
                            .size(15.0)
                            .family(theme::medium());
                        ui.label(name);
                    },
                    |ui| {
                        let muted = theme::muted(dark);
                        ui.label(egui::RichText::new(count).size(12.0).color(muted));
                    },
                );
            });
    }

    /// A click on card `i`: it alone selected; with Shift, the pages from the last one
    /// clicked to it; with Ctrl or Cmd, it added or taken out.
    fn click_page(&mut self, i: usize, m: egui::Modifiers) {
        let mut set = self.ed.selection.pages().cloned().unwrap_or_default();
        if m.shift {
            let (a, b) = (self.anchor.min(i), self.anchor.max(i));
            set = (a..=b).collect();
        } else if m.command {
            if !set.remove(&i) {
                set.insert(i);
            }
            self.anchor = i;
        } else {
            set = BTreeSet::from([i]);
            self.anchor = i;
        }
        self.ed.selection = Selection::pages_from(set);
    }

    /// Card `i`'s checkbox: the page added to the pages selected, or taken out.
    fn toggle_page(&mut self, i: usize) {
        let mut set = self.ed.selection.pages().cloned().unwrap_or_default();
        if !set.remove(&i) {
            set.insert(i);
        }
        self.anchor = i;
        self.ed.selection = Selection::pages_from(set);
    }

    /// Every page selected.
    pub fn select_all_pages(&mut self) {
        let n = self.page_count();
        self.ed.selection = Selection::pages_from((0..n).collect());
    }

    /// The grid's pictures, by page, in page order: what each was drawn as.
    #[cfg(test)]
    pub fn grid_pictures(&self) -> Vec<egui::TextureId> {
        (0..self.page_count())
            .filter_map(|i| self.pages.get(&i).map(|(_, t)| t.id()))
            .collect()
    }

    /// How many of the grid's pictures are still being drawn.
    #[cfg(test)]
    pub fn grid_waiting(&self) -> usize {
        self.pending
            .iter()
            .filter(|k| k.2.is_none() && k.1 != 0)
            .count()
    }

    /// Whether page `i`'s picture is kept.
    #[cfg(test)]
    pub fn grid_drawn(&self, i: usize) -> bool {
        self.pages.contains_key(&i)
    }
}
