//! Organise (NEW-14): the document's pages as a grid of cards, shown in place of the pages,
//! to select, move, turn, take out and delete pages, as a presentation's slide sorter. Each
//! card is the page's picture with a checkbox and its name under it. Only the rows in view
//! are drawn, with their pictures, so a long document's grid is as quick as a short one's.
//! Cards are dragged to move them; the keyboard moves among them as in a file manager, and
//! Alt+Shift with an arrow moves the pages selected, as Word moves paragraphs. Pages cut
//! are shown faded until pasted, which moves them; a click between cards puts the
//! insertion bar there, where pages inserted or pasted go.

use super::{edge_scroll, moved_order, moving_label, Tab, BASE};
use crate::actions::{Ctx, PageMove};
use crate::icon;
use crate::selection::Selection;
use crate::settings::{ORGANISE_MAX, ORGANISE_MIN};
use crate::theme;
use eframe::egui::{self, Color32, Key, Pos2, Rect, Sense, Vec2};
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
/// How faint a cut page's card is, until it is pasted.
const FADED: f32 = 0.35;

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
    /// The card the keyboard is at, and whether to ring it: the keys moved it, or Tab
    /// brought the keyboard to the grid.
    pub(super) focus: usize,
    keyed: bool,
    /// The insertion bar, where pages inserted or pasted go: before page `.0`, drawn
    /// after the card before it when `.1` (a click past a row's last card).
    pub(super) bar: Option<(usize, bool)>,
    /// Cards are being dragged, and where they would land now (before that page; the
    /// page count for the end), when over the grid.
    dragging: bool,
    drop: Option<usize>,
    /// The cards to a row, and the rows in view, last drawn: the keys' steps.
    cols: usize,
    screen: usize,
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

    /// The gap between cards nearest point `p` (the grid's own points), of `n` cards: the
    /// place pages go there (before that page; `n` for the end), and whether it is past
    /// the last card of its row. Over a card, the gap on the side of its half. With
    /// `beside`, only beside the cards of a row, not between rows or below them.
    fn gap(&self, p: Pos2, n: usize, beside: bool) -> Option<(usize, bool)> {
        let rows = n.div_ceil(self.cols).max(1);
        let step = self.card.y + BETWEEN;
        let row = if beside {
            let row = ((p.y - MARGIN) / step).floor();
            let into = p.y - MARGIN - row * step;
            if row < 0.0 || row as usize >= rows || into > self.card.y {
                return None;
            }
            row as usize
        } else {
            let row = ((p.y - MARGIN + BETWEEN / 2.0) / step).floor();
            row.clamp(0.0, (rows - 1) as f32) as usize
        };
        let in_row = (n - row * self.cols).min(self.cols);
        let col = ((p.x - self.left + BETWEEN / 2.0) / (self.card.x + BETWEEN)).round();
        let col = col.clamp(0.0, in_row as f32) as usize;
        Some((row * self.cols + col, col == in_row && col > 0))
    }

    /// Where the insertion bar is drawn for place `at` of `n` (see [`Cards::gap`]): in the
    /// gap before that card, or after the card before it.
    fn bar(&self, at: usize, after: bool, n: usize) -> Rect {
        let (row, col) = match (after || at >= n) && at > 0 {
            true => ((at - 1) / self.cols, (at - 1) % self.cols + 1),
            false => (at / self.cols, at % self.cols),
        };
        let x = self.left + col as f32 * (self.card.x + BETWEEN) - BETWEEN / 2.0;
        let top = MARGIN + row as f32 * (self.card.y + BETWEEN);
        Rect::from_min_max(
            Pos2::new(x - 1.5, top),
            Pos2::new(x + 1.5, top + self.card.y),
        )
    }
}

/// What a key does in a list of pages: the keyboard goes to a page, selecting it alone,
/// the pages from the anchor to it, or nothing; or the page it is at is added or taken
/// out, or opened.
#[derive(Clone, Copy, PartialEq, Debug)]
enum ListKey {
    To(usize, How),
    Toggle,
    Open,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum How {
    Select,
    Extend,
    Focus,
}

/// Takes the first press of `key` with exactly Shift and Ctrl (Cmd on macOS) as asked,
/// and never Alt, which moves the pages selected instead.
fn take(i: &mut egui::InputState, key: Key, shift: bool, command: bool) -> bool {
    let at = i.events.iter().position(|e| {
        matches!(e, egui::Event::Key { key: k, pressed: true, modifiers: m, .. }
            if *k == key && !m.alt && m.shift == shift && m.command == command)
    });
    at.map(|at| i.events.remove(at)).is_some()
}

/// The next key for a list of `n` pages, `cols` to a row and `screen` rows in view, the
/// keyboard at page `at`, taken from the input: the arrows go a page or a row, Page Up and
/// Page Down a screen of rows, Home and End to the first and last page; with Shift they
/// select from the anchor, with Ctrl or Cmd they move the keyboard alone. Space (with Ctrl
/// or Cmd too) adds or takes out the page, Enter opens it.
fn list_key(
    ctx: &egui::Context,
    at: usize,
    cols: usize,
    screen: usize,
    n: usize,
) -> Option<ListKey> {
    let last = n.saturating_sub(1);
    let page = screen.max(1) * cols;
    let down = |by: usize| match at + by {
        to if to <= last => to,
        // A row below that is short of this column: its last page.
        _ if at / cols < last / cols => last,
        _ => at,
    };
    let steps = [
        (Key::ArrowLeft, at.saturating_sub(1)),
        (Key::ArrowRight, (at + 1).min(last)),
        (Key::ArrowUp, at.checked_sub(cols).unwrap_or(at)),
        (Key::ArrowDown, down(cols)),
        (Key::PageUp, at.checked_sub(page).unwrap_or(at % cols)),
        (Key::PageDown, (at + page).min(last)),
        (Key::Home, 0),
        (Key::End, last),
    ];
    ctx.input_mut(|i| {
        for (key, to) in steps {
            for (shift, command, how) in [
                (false, false, How::Select),
                (true, false, How::Extend),
                (false, true, How::Focus),
            ] {
                if take(i, key, shift, command) {
                    return Some(ListKey::To(to, how));
                }
            }
        }
        if take(i, Key::Space, false, false) || take(i, Key::Space, false, true) {
            return Some(ListKey::Toggle);
        }
        take(i, Key::Enter, false, false).then_some(ListKey::Open)
    })
}

/// The keyboard to the list `id`, which took a key. An arrow pressed with nothing holding
/// the keyboard was already taken by egui to move it from one control to the next at the
/// end of the frame, which would take it straight on from the list: not now.
fn take_keyboard(ctx: &egui::Context, id: egui::Id) {
    ctx.memory_mut(|m| {
        m.request_focus(id);
        m.move_focus(egui::FocusDirection::None);
    });
}

/// The order after moving the pages `moving` of `n` by `by` places (earlier when
/// negative), each as far as it can go without passing the start, the end or another of
/// them, so that pages apart stay apart: for each new position, the old page.
fn shifted_order(n: usize, moving: &BTreeSet<usize>, by: isize) -> Vec<usize> {
    let pages: Vec<usize> = moving.iter().copied().filter(|&p| p < n).collect();
    let mut to = vec![0; pages.len()];
    if by < 0 {
        let mut next = 0;
        for (j, &p) in pages.iter().enumerate() {
            to[j] = p.saturating_sub(by.unsigned_abs()).max(next);
            next = to[j] + 1;
        }
    } else {
        let mut next = n;
        for (j, &p) in pages.iter().enumerate().rev() {
            to[j] = (p + by as usize).min(next - 1);
            next = to[j];
        }
    }
    let mut slots: Vec<Option<usize>> = vec![None; n];
    for (&p, &t) in pages.iter().zip(&to) {
        slots[t] = Some(p);
    }
    let mut rest = (0..n).filter(|p| !moving.contains(p));
    slots
        .into_iter()
        .map(|s| s.or_else(|| rest.next()).unwrap_or(0))
        .collect()
}

impl Tab {
    /// Whether the tab shows its pages as Organise's grid.
    pub fn organising(&self) -> bool {
        self.organise.is_some()
    }

    /// The grid's keyboard stop: the list of cards.
    pub fn grid_id(&self) -> egui::Id {
        egui::Id::unique(("page-grid", self.id))
    }

    /// The side panel's thumbnails' keyboard stop.
    pub fn list_id(&self) -> egui::Id {
        egui::Id::unique(("page-list", self.id))
    }

    /// Whether the grid or the thumbnails have the keyboard: their keys are the pages',
    /// and the commands' plain keys (Delete, Esc) work there too.
    pub fn holds_keys(&self, ctx: &egui::Context) -> bool {
        let focused = ctx.memory(|m| m.focused());
        matches!(focused, Some(f) if f == self.grid_id() || f == self.list_id())
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
            focus: self.current,
            keyed: false,
            bar: None,
            dragging: false,
            drop: None,
            cols: 1,
            screen: 1,
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
        o.cols = cols;
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
        let cut = self.cut_pages();
        let grid = self.grid_id();
        let mut wanted = Vec::new();
        let (mut click, mut open, mut toggle, mut dropped) = (None, None, None, None);
        let mut kept = (0, 0);
        area.show_viewport(ui, |ui, view| {
            let origin = ui.max_rect().min;
            let room = Vec2::new(width, height.max(view.height()));
            ui.set_min_size(room);
            let to_grid = |p: Pos2| Pos2::ZERO + (p - origin);
            // The background first, so the cards over it take their own clicks. It is the
            // grid's one stop for Tab, which takes the arrows and Esc for the cards.
            let bg = ui.interact(
                Rect::from_min_size(origin, room),
                grid,
                Sense::click_and_drag(),
            );
            bg.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ListBox, true, "Page grid"));
            let pressed = ui.input(|i| i.pointer.any_pressed());
            let o = self.organise.as_mut().expect("organising");
            if bg.has_focus() {
                let keep = egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                    tab: false,
                };
                ui.memory_mut(|m| m.set_focus_lock_filter(grid, keep));
            }
            if bg.gained_focus() && !pressed {
                o.keyed = true;
            }
            let ring = bg.has_focus() && o.keyed;
            let (r0, r1) = lay.rows(view.min.y, view.max.y, rows);
            let (lo, hi) = (r0.saturating_sub(AHEAD), (r1 + AHEAD).min(rows));
            kept = (r0.saturating_sub(KEEP), (r1 + KEEP).min(rows));
            o.view = Some(view);
            o.first = (r0 * cols).min(n.saturating_sub(1));
            o.screen = ((view.height() + BETWEEN) / (card.y + BETWEEN))
                .floor()
                .max(1.0) as usize;
            let focus = o.focus;
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
                // Dragging a card moves it, or the pages selected if it is one of them.
                if resp.drag_started() {
                    if !sel {
                        self.ed.selection = Selection::pages_from([i].into());
                        self.anchor = i;
                    }
                    let o = self.organise.as_mut().expect("organising");
                    (o.dragging, o.focus, o.bar) = (true, i, None);
                    ui.memory_mut(|m| m.request_focus(grid));
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
                // A page cut is faint until it is pasted.
                let faint = |c: Color32| match cut.contains(&i) {
                    true => c.gamma_multiply(FADED),
                    false => c,
                };
                let (fill, edge) = match sel {
                    true => (theme::accent_bg(dark), theme::accent(dark)),
                    false => (theme::surface(dark), theme::border(dark)),
                };
                let p = ui.painter();
                p.rect_filled(rect, 5.0, fill);
                let line = egui::Stroke::new(1.0, edge);
                p.rect_stroke(rect, 5.0, line, egui::StrokeKind::Inside);
                // The card the keyboard is at.
                if ring && i == focus {
                    let ring = egui::Stroke::new(2.0, theme::accent(dark));
                    p.rect_stroke(rect.expand(3.0), 7.0, ring, egui::StrokeKind::Outside);
                }
                let img = Rect::from_center_size(holder.center(), Vec2::new(w * k, h * k));
                p.rect_filled(img, 0.0, faint(paper));
                let outline = egui::Stroke::new(1.0, faint(theme::border(dark)));
                p.rect_stroke(img, 0.0, outline, egui::StrokeKind::Outside);
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                match self.pages.get(&i) {
                    Some((_, t)) => {
                        p.image(t.id(), img, uv, faint(Color32::WHITE));
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
                // The checkbox, ticked white on the accent, and the page's name beside it;
                // not ticked, its edge stands out from the card.
                let look = ui.style().interact(&cb);
                let accent = theme::accent(dark);
                let (fill, edge) = match sel {
                    true => (accent, accent),
                    false => (look.bg_fill, theme::muted(dark)),
                };
                let edge = egui::Stroke::new(1.0, edge);
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
                    faint(ui.visuals().text_color()),
                );
            }

            let accent = theme::accent(dark);
            let pointer = ui.input(|i| i.pointer.latest_pos());
            let o = self.organise.as_mut().expect("organising");
            // Cards dragged: the bar where they would land, over the grid; let go there,
            // they move; outside it, or after Esc, nothing does.
            if o.dragging {
                let over = pointer.filter(|&p| ui.clip_rect().contains(p));
                let gap = over.and_then(|p| Some((p, lay.gap(to_grid(p), n, false)?)));
                o.drop = gap.map(|(_, (at, _))| at);
                if let Some((p, (at, after))) = gap {
                    let bar = lay.bar(at, after, n).translate(origin - Pos2::ZERO);
                    ui.painter().rect_filled(bar, 1.5, accent);
                    moving_label(ui, p, chosen.len().max(1));
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                edge_scroll(ui);
                if !ui.input(|i| i.pointer.any_down()) {
                    dropped = o.drop.take();
                    o.dragging = false;
                }
            } else if let Some((at, after)) = o.bar {
                let bar = lay.bar(at, after, n).translate(origin - Pos2::ZERO);
                ui.painter().rect_filled(bar, 1.5, accent);
            }

            // A drag across the background selects the cards it touches; with Ctrl or
            // Shift held, as well as those selected before.
            if bg.drag_started() {
                let from = ui.input(|i| i.pointer.press_origin()).unwrap_or(origin);
                let m = ui.input(|i| i.modifiers);
                let before = match m.command || m.shift {
                    true => chosen.clone(),
                    false => BTreeSet::new(),
                };
                o.sweep = Some((to_grid(from), before));
                o.bar = None;
            }
            if let (Some((from, before)), Some(p)) = (&o.sweep, pointer) {
                let swept = Rect::from_two_pos(*from, to_grid(p));
                let mut set = before.clone();
                set.extend(lay.touched(swept, n));
                let on_screen = swept.translate(origin - Pos2::ZERO);
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
            // A click between cards puts the insertion bar there, the pages selected let
            // go, as in a slide sorter; elsewhere it lets go of them.
            if bg.clicked() {
                let at = bg.interact_pointer_pos();
                o.bar = at.and_then(|p| lay.gap(to_grid(p), n, true));
                o.keyed = false;
                self.ed.selection = Selection::None;
                ui.memory_mut(|m| m.request_focus(grid));
            }
        });

        if let Some(i) = open {
            self.open_page(&ctx, i);
            return;
        }
        // A click on a card or its checkbox gives the grid the keyboard.
        if toggle.is_some() || click.is_some() {
            let o = self.organise.as_mut().expect("organising");
            (o.keyed, o.bar) = (false, None);
            ctx.memory_mut(|m| m.request_focus(grid));
        }
        if let Some(i) = toggle {
            self.toggle_page(i);
        }
        if let Some((i, m)) = click {
            self.click_page(i, m);
        }
        if let Some(to) = dropped {
            self.move_selected_pages(&ctx, to);
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
        if let Some(o) = self.organise.as_mut() {
            o.focus = i;
        }
    }

    /// Card `i`'s checkbox: the page added to the pages selected, or taken out.
    fn toggle_page(&mut self, i: usize) {
        let mut set = self.ed.selection.pages().cloned().unwrap_or_default();
        if !set.remove(&i) {
            set.insert(i);
        }
        self.anchor = i;
        self.ed.selection = Selection::pages_from(set);
        if let Some(o) = self.organise.as_mut() {
            o.focus = i;
        }
    }

    /// Every page selected.
    pub fn select_all_pages(&mut self) {
        let n = self.page_count();
        self.ed.selection = Selection::pages_from((0..n).collect());
    }

    /// The keyboard to page `p` of a list of pages: it alone selected, the pages from the
    /// anchor to it, or the selection left as it is.
    fn key_to(&mut self, p: usize, how: How) {
        match how {
            How::Select => {
                self.ed.selection = Selection::pages_from([p].into());
                self.anchor = p;
            }
            How::Extend => {
                let (a, b) = (self.anchor.min(p), self.anchor.max(p));
                self.ed.selection = Selection::pages_from((a..=b).collect());
            }
            How::Focus => {}
        }
    }

    /// The keys of the grid, when it has the keyboard or nothing has, or of the
    /// thumbnails, when they have it (see [`list_key`]). Run before the commands' keys,
    /// which then don't see them.
    pub fn list_keys(&mut self, ctx: &egui::Context) {
        let focused = ctx.memory(|m| m.focused());
        let n = self.page_count();
        if n == 0 {
            return;
        }
        if self.organising() {
            let grid = self.grid_id();
            if focused.is_some_and(|f| f != grid) {
                return;
            }
            loop {
                let o = self.organise.as_ref().expect("organising");
                let at = o.focus.min(n - 1);
                let Some(key) = list_key(ctx, at, o.cols.max(1), o.screen, n) else {
                    return;
                };
                take_keyboard(ctx, grid);
                let at = match key {
                    ListKey::Open => {
                        self.open_page(ctx, at);
                        return;
                    }
                    ListKey::Toggle => {
                        self.toggle_page(at);
                        at
                    }
                    ListKey::To(p, how) => {
                        self.key_to(p, how);
                        p
                    }
                };
                let o = self.organise.as_mut().expect("organising");
                (o.focus, o.keyed, o.seek, o.bar) = (at, true, Some(at), None);
            }
        }
        if focused != Some(self.list_id()) {
            return;
        }
        // The thumbnails: a column, the page view following the keyboard.
        let at = self.thumb_focus.min(n - 1);
        if let Some(key) = list_key(ctx, at, 1, self.thumb_screen, n) {
            take_keyboard(ctx, self.list_id());
            let p = match key {
                ListKey::Open => at,
                ListKey::Toggle => {
                    let mut set = self.ed.selection.pages().cloned().unwrap_or_default();
                    if !set.remove(&at) {
                        set.insert(at);
                    }
                    self.ed.selection = Selection::pages_from(set);
                    self.anchor = at;
                    at
                }
                ListKey::To(p, how) => {
                    self.key_to(p, how);
                    p
                }
            };
            if !matches!(key, ListKey::To(_, How::Focus) | ListKey::Toggle) {
                self.go_to(p);
            }
            (self.thumb_focus, self.thumb_keyed, self.thumb_seek) = (p, true, Some(p));
        }
    }

    /// Move the pages selected a place or a row earlier or later (a row is the grid's;
    /// in the thumbnails, a page), or to the start or the end, in one step. Pages apart
    /// stay apart, except to the start or the end, where they go together.
    pub fn move_pages(&mut self, ctx: &egui::Context, to: PageMove) {
        let Some(moving) = self.ed.selection.pages().cloned() else {
            return;
        };
        let n = self.page_count();
        let row = self.organise.as_ref().map_or(1, |o| o.cols.max(1)) as isize;
        let order = match to {
            PageMove::Earlier => shifted_order(n, &moving, -1),
            PageMove::Later => shifted_order(n, &moving, 1),
            PageMove::Up => shifted_order(n, &moving, -row),
            PageMove::Down => shifted_order(n, &moving, row),
            PageMove::Start => moved_order(n, &moving, 0),
            PageMove::End => moved_order(n, &moving, n),
        };
        self.reorder_pages(ctx, &order);
    }

    /// Put the pages in `order` (for each new position, the old page), in one step: the
    /// pages selected stay selected wherever they went, the first of them shown.
    pub(super) fn reorder_pages(&mut self, ctx: &egui::Context, order: &[usize]) {
        if order.iter().copied().eq(0..order.len()) {
            return;
        }
        let before = self.revision;
        self.arrange(ctx, &[], |p| p.reorder(order));
        if self.revision == before {
            return;
        }
        // The selection followed its pages (Tab::follow_pages).
        let first = self.ed.selection.pages().and_then(|s| s.first().copied());
        if let Some(p) = first {
            self.anchor = p;
            self.show_placed(p);
        }
    }

    /// Bring page `p`, just moved or added, into view: the grid's card, or in the page
    /// view the page.
    fn show_placed(&mut self, p: usize) {
        match self.organise.as_mut() {
            Some(o) => (o.seek, o.focus) = (Some(p), p),
            None => self.go_to(p),
        }
    }

    /// After `k` pages were added from page `at` on: in Organise they are selected, the
    /// insertion bar after them gone; in the page view, the first is gone to.
    pub fn placed(&mut self, at: usize, k: usize) {
        if k == 0 {
            return;
        }
        if self.organising() {
            self.ed.selection = Selection::pages_from((at..at + k).collect());
            self.anchor = at;
        }
        self.show_placed(at);
    }

    /// Where pages inserted or pasted go: at the insertion bar in Organise, else after the
    /// selected pages, else after the current page.
    pub fn insert_at(&self) -> usize {
        let n = self.page_count();
        let bar = self.organise.as_ref().and_then(|o| o.bar);
        match bar {
            Some((at, _)) => at.min(n),
            None => self.targets().last().map_or(n, |&p| (p + 1).min(n)),
        }
    }

    /// The page with object `id`, now.
    pub fn page_of(&self, id: (u32, u16)) -> Option<usize> {
        self.ids.iter().position(|&p| p == id)
    }

    /// Page `p`'s object.
    pub fn page_id(&self, p: usize) -> Option<(u32, u16)> {
        self.ids.get(p).copied()
    }

    /// Cut the pages selected: they show faint until pasted, which moves them, or Esc.
    /// How many were cut.
    pub fn cut_selected_pages(&mut self) -> usize {
        let chosen = self.ed.selection.pages().cloned().unwrap_or_default();
        self.cut = chosen.iter().filter_map(|&p| self.page_id(p)).collect();
        self.cut.len()
    }

    /// The pages cut and not yet pasted, where they are now.
    pub fn cut_pages(&self) -> BTreeSet<usize> {
        self.cut.iter().filter_map(|&id| self.page_of(id)).collect()
    }

    pub fn has_cut(&self) -> bool {
        !self.cut.is_empty()
    }

    /// Paste the pages cut: they move, in their order, to where pages go
    /// ([`Tab::insert_at`]), in one step, and are selected there. With nowhere else for
    /// them, nothing moves, and what to do is returned to say.
    pub fn paste_pages(&mut self, ctx: &egui::Context) -> Option<&'static str> {
        let moving = self.cut_pages();
        let (to, n) = (self.insert_at(), self.page_count());
        if moved_order(n, &moving, to).iter().copied().eq(0..n) {
            return Some(
                "The pages cut are already there. Click a page to put them after it, or \
                 between pages, then paste",
            );
        }
        self.ed.selection = Selection::pages_from(moving);
        let before = self.revision;
        self.move_selected_pages(ctx, to);
        // Moved: nothing is left to paste. (A move that failed says why, the pages still
        // cut.)
        if self.revision != before {
            self.cut.clear();
        }
        None
    }

    /// Esc for the pages: cards or thumbnails being dragged stay where they were, then
    /// pages cut are no longer cut, then the insertion bar goes, then in Organise the
    /// pages selected are let go. Whether there was something.
    pub fn escape_pages(&mut self) -> bool {
        if let Some(o) = self.organise.as_mut().filter(|o| o.dragging) {
            (o.dragging, o.drop) = (false, None);
            return true;
        }
        if self.thumb_drag.take().is_some() || !std::mem::take(&mut self.cut).is_empty() {
            return true;
        }
        let Some(o) = self.organise.as_mut() else {
            return false;
        };
        if o.bar.take().is_none() {
            self.ed.selection = Selection::None;
        }
        true
    }

    /// The card the keyboard is at.
    #[cfg(test)]
    pub fn grid_focus(&self) -> usize {
        self.organise.as_ref().map_or(0, |o| o.focus)
    }

    /// Where the insertion bar is: the place pages go.
    #[cfg(test)]
    pub fn grid_bar(&self) -> Option<usize> {
        self.organise.as_ref().and_then(|o| o.bar).map(|(at, _)| at)
    }

    /// Where the cards dragged would land now.
    #[cfg(test)]
    pub fn grid_drop(&self) -> Option<usize> {
        self.organise.as_ref().and_then(|o| o.drop)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn set(pages: &[usize]) -> BTreeSet<usize> {
        pages.iter().copied().collect()
    }

    /// Pages moved a place or a row go as far as they can, pages apart staying apart and
    /// in their order, none past the start or the end.
    #[test]
    fn shifting_pages_keeps_them_apart_and_in_order() {
        assert_eq!(shifted_order(5, &set(&[2]), -1), [0, 2, 1, 3, 4]);
        assert_eq!(shifted_order(7, &set(&[2, 5]), -1), [0, 2, 1, 3, 5, 4, 6]);
        assert_eq!(shifted_order(5, &set(&[0, 3]), -1), [0, 1, 3, 2, 4]);
        assert_eq!(shifted_order(5, &set(&[1, 2]), -4), [1, 2, 0, 3, 4]);
        assert_eq!(shifted_order(5, &set(&[2]), 1), [0, 1, 3, 2, 4]);
        assert_eq!(shifted_order(5, &set(&[3, 4]), 1), [0, 1, 2, 3, 4]);
        assert_eq!(shifted_order(9, &set(&[1]), 4), [0, 2, 3, 4, 5, 1, 6, 7, 8]);
    }

    /// A gap is found beside a row's cards, before the first and past the last; between
    /// rows only when asked (a drop), not for a click.
    #[test]
    fn the_gap_nearest_a_point() {
        let lay = Cards {
            card: Vec2::new(100.0, 140.0),
            cols: 3,
            left: MARGIN,
        };
        let mid = |row: usize| MARGIN + row as f32 * 154.0 + 70.0;
        // Between the first and second cards, and over the second's right half.
        assert_eq!(
            lay.gap(Pos2::new(MARGIN + 107.0, mid(0)), 8, true),
            Some((1, false))
        );
        assert_eq!(
            lay.gap(Pos2::new(MARGIN + 190.0, mid(0)), 8, false),
            Some((2, false))
        );
        assert_eq!(lay.gap(Pos2::new(2.0, mid(1)), 8, true), Some((3, false)));
        // Past the last card of a row; of the last row, the end.
        assert_eq!(lay.gap(Pos2::new(400.0, mid(0)), 8, true), Some((3, true)));
        assert_eq!(lay.gap(Pos2::new(400.0, mid(2)), 8, true), Some((8, true)));
        // Between rows, and below the last.
        assert_eq!(
            lay.gap(Pos2::new(MARGIN + 107.0, MARGIN + 147.0), 8, true),
            None
        );
        assert_eq!(
            lay.gap(Pos2::new(MARGIN + 107.0, 2000.0), 8, false),
            Some((7, false))
        );
    }
}
