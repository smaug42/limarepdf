//! The Comments view (ENH-40): the document's comments and markup as cards under their
//! pages' names, replies under what they answer, as Word's and Acrobat's comment lists show
//! them. Only the cards in view are drawn, so a long list costs no more than a short one. A
//! count, a filter by type, author and page, and a search of the comments' words narrow the
//! list, per tab and never in the file. A card clicked selects its comment and shows its
//! mark on the page; a mark selected on the page selects its card and brings it into view.
//! The list is one stop for Tab and takes the keys as the thumbnails do; Previous and Next
//! comment go through the same cards.

use super::organise::take_keyboard;
use super::{thread_order, Tab};
use crate::editor::annot_tip;
use crate::icon;
use crate::selection::Selection;
use crate::theme;
use eframe::egui::{self, Key, Rect, Sense, Vec2};
use limarepdf_core::{display_line, words_text, Annot, AnnotKind, PageWord, Pdf};
use std::collections::{BTreeSet, HashMap, HashSet};

/// The room inside a card, between cards, before a reply for each step it is deeper, and
/// the height of the selected card's Reply button.
const PAD: f32 = 8.0;
const GAP: f32 = 6.0;
const INDENT: f32 = 14.0;
const REPLY: f32 = 22.0;
/// The room under a card's second line, and kept clear of the scroll bar on the right.
const SUB: f32 = 3.0;
const BAR: f32 = 10.0;
/// A comment's first lines shown on its card, at most.
const LINES: usize = 2;

/// A tab's Comments view: what its filter and search let through, and where the list is.
/// Never written into the file.
#[derive(Default)]
pub struct Review {
    /// The kinds (by their labels) and authors whose comments the filter hides.
    pub hide_kinds: BTreeSet<String>,
    pub hide_authors: BTreeSet<String>,
    /// Only the comments on the page being read.
    pub this_page: bool,
    /// Words to find in the comments, their replies and the words they mark.
    pub query: String,
    /// The cards the filter and search let through, for what they were worked out from.
    order: Option<(Shown, Vec<(usize, usize)>)>,
    /// The words each text markup without a comment marks, read as its card is first drawn
    /// or searched, and the pages' words they were read from, for revision `read_at`.
    marked: HashMap<(u32, u16), String>,
    words: HashMap<usize, Vec<PageWord>>,
    read_at: u64,
    /// How many lines each comment's first lines take on its card, for the width
    /// `measured` and revision `read_at`.
    lines: HashMap<(u32, u16), usize>,
    measured: f32,
    /// The comment last shown selected: another one selected brings its card into view.
    shown: Option<(u32, u16)>,
    /// The keys moved the selection: its card is ringed while the list has the keyboard.
    keyed: bool,
    /// Bring the selected card into view when next drawn.
    seek: bool,
    /// The part of the list in view last frame, in its own points.
    view: Option<Rect>,
    /// The Filter button's width when last drawn: the search box beside it takes the rest.
    filter_width: Option<f32>,
}

/// What the cards shown were worked out from: the document's revision and the filter and
/// search, with the page being read when only its comments are shown.
#[derive(Clone, PartialEq)]
struct Shown {
    revision: u64,
    kinds: BTreeSet<String>,
    authors: BTreeSet<String>,
    page: Option<usize>,
    query: String,
}

/// A row of the list: a page's name, or a comment's card (its place in the list of
/// annotations and how deep a reply it is).
#[derive(Clone, Copy)]
enum Row {
    Page(usize),
    Card(usize, usize),
}

/// What the card list's keys asked for.
enum CardKey {
    To(usize),
    Reply,
    Leave,
}

impl Tab {
    /// The Comments view's keyboard stop: the list of cards.
    pub fn comments_id(&self) -> egui::Id {
        egui::Id::unique(("comment-list", self.id))
    }

    /// Read the annotations again after the document changed. A selected one no longer
    /// there is let go.
    fn read_comments(&mut self) {
        let rev = self.revision;
        if self.comments.as_ref().is_none_or(|(r, _)| *r != rev) {
            let list = self.pdf.annotations();
            let gone = |(_, id)| !list.iter().any(|a| a.id == id);
            if self.ed.selection.annot().is_some_and(gone) {
                self.ed.selection.clear_annot();
            }
            self.comments = Some((rev, list));
        }
    }

    /// The cards shown, in the list's order with each one's depth: the threads whose
    /// comment the filter lets through and whose words hold the search's, page by page.
    fn shown_order(&mut self) -> Vec<(usize, usize)> {
        self.read_comments();
        let r = &mut self.review;
        let now = Shown {
            revision: self.revision,
            kinds: r.hide_kinds.clone(),
            authors: r.hide_authors.clone(),
            page: r.this_page.then_some(self.current),
            query: r.query.trim().to_lowercase(),
        };
        if let Some((was, order)) = &r.order {
            if *was == now {
                return order.clone();
            }
        }
        if r.read_at != self.revision {
            r.marked.clear();
            r.words.clear();
            r.lines.clear();
            r.read_at = self.revision;
        }
        let list = self.comments.as_ref().map_or(&[][..], |(_, l)| l);
        let all = thread_order(list);
        let mut order = Vec::with_capacity(all.len());
        let mut k = 0;
        while k < all.len() {
            // A thread: its comment, then the replies deeper than it.
            let end = (k + 1..all.len())
                .find(|&j| all[j].1 == 0)
                .unwrap_or(all.len());
            let a = &list[all[k].0];
            let passes = !now.kinds.contains(&a.label())
                && !now.authors.contains(&author_name(a))
                && now.page.is_none_or(|p| p == a.page);
            let found = now.query.is_empty()
                || all[k..end].iter().any(|&(i, _)| {
                    let b = &list[i];
                    b.contents.to_lowercase().contains(&now.query)
                        || (b.contents.trim().is_empty() && !b.quads.is_empty() && {
                            let words = marked(&self.pdf, &mut r.marked, &mut r.words, b);
                            words.to_lowercase().contains(&now.query)
                        })
                });
            if passes && found {
                order.extend_from_slice(&all[k..end]);
            }
            k = end;
        }
        r.order = Some((now, order.clone()));
        order
    }

    /// The Comments side view: a search box, the count and the filter, then the cards.
    pub fn show_comments(&mut self, ui: &mut egui::Ui) {
        let order = self.shown_order();
        let total = self.comments.as_ref().map_or(0, |(_, l)| l.len());
        if total == 0 {
            ui.weak("This document has no comments or markup.");
            return;
        }
        // The search box and Filter on one row, left to right so Tab reaches them in that
        // order: the box is what room Filter leaves, as wide as it was last drawn (the
        // frame drawn again when that changed).
        ui.horizontal(|ui| {
            let filter = self.review.filter_width.unwrap_or(80.0);
            let room = ui.available_width() - filter - ui.spacing().item_spacing.x;
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.review.query)
                    .hint_text("Search comments")
                    .desired_width(room.max(40.0)),
            );
            crate::dialogs::set_name(&r, "Search comments");
            let width = self.filter_menu(ui);
            if self.review.filter_width != Some(width) {
                self.review.filter_width = Some(width);
                ui.ctx().request_discard("filter measured");
            }
        });
        let count = match (order.len(), total) {
            (1, 1) => "1 comment".to_string(),
            (n, t) if n == t => format!("{t} comments"),
            (n, t) => format!("{n} of {t} comments"),
        };
        let muted = theme::muted(ui.visuals().dark_mode);
        ui.label(egui::RichText::new(count).small().color(muted));
        ui.add_space(2.0);
        self.cards(ui, &order);
    }

    /// The Filter menu: the kinds and authors to show, all pages or the one being read.
    /// Returns its button's width.
    fn filter_menu(&mut self, ui: &mut egui::Ui) -> f32 {
        let list = self.comments.as_ref().map_or(&[][..], |(_, l)| l);
        let roots = list.iter().filter(|a| a.reply_to.is_none());
        let kinds: BTreeSet<String> = roots.clone().map(Annot::label).collect();
        let authors: BTreeSet<String> = roots.map(author_name).collect();
        let r = &mut self.review;
        let on = !r.hide_kinds.is_empty() || !r.hide_authors.is_empty() || r.this_page;
        let caret = egui::Atom::from(theme::icon(icon::CARET_DOWN).size(10.0));
        let button = egui::Button::new((theme::icon(icon::FUNNEL_SIMPLE), "Filter", caret))
            .frame_when_inactive(on)
            .selected(on);
        let (menu, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
            let heading = |ui: &mut egui::Ui, text: &str| {
                let muted = theme::muted(ui.visuals().dark_mode);
                ui.label(egui::RichText::new(text).small().strong().color(muted));
            };
            heading(ui, "Type");
            for k in &kinds {
                let mut shown = !r.hide_kinds.contains(k);
                if ui.checkbox(&mut shown, k.as_str()).changed() {
                    toggle(&mut r.hide_kinds, k, shown);
                }
            }
            ui.separator();
            heading(ui, "Author");
            for a in &authors {
                let mut shown = !r.hide_authors.contains(a);
                if ui.checkbox(&mut shown, a.as_str()).changed() {
                    toggle(&mut r.hide_authors, a, shown);
                }
            }
            ui.separator();
            heading(ui, "Page");
            ui.radio_value(&mut r.this_page, false, "All pages");
            ui.radio_value(&mut r.this_page, true, "This page");
            ui.separator();
            let clear = ui.add_enabled(on, egui::Button::new("Clear filters"));
            if clear.clicked() {
                (r.hide_kinds, r.hide_authors) = Default::default();
                r.this_page = false;
                ui.close();
            }
        });
        menu.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Filter"));
        menu.rect.width()
    }

    /// The cards, under their pages' names; only those in view are drawn.
    fn cards(&mut self, ui: &mut egui::Ui, order: &[(usize, usize)]) {
        let ctx = ui.ctx().clone();
        let list_id = self.comments_id();
        // The list is one stop for Tab, which takes the arrows and Esc for the cards; the
        // cards take clicks.
        let stop = ui.interact(ui.available_rect_before_wrap(), list_id, Sense::FOCUSABLE);
        stop.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ListBox, true, "Comment list"));
        if stop.has_focus() {
            let keep = egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
                tab: false,
            };
            ctx.memory_mut(|m| m.set_focus_lock_filter(list_id, keep));
        }
        if stop.gained_focus() && !ctx.input(|i| i.pointer.any_pressed()) {
            self.review.keyed = true;
        }
        let ring = stop.has_focus() && self.review.keyed;
        if order.is_empty() {
            ui.weak("No comments match.");
            return;
        }
        let Some((_, list)) = &self.comments else {
            return;
        };
        let chosen_id = self.ed.selection.annot().map(|(_, id)| id);
        if chosen_id != self.review.shown {
            self.review.shown = chosen_id;
            self.review.seek = true;
        }

        // The rows and their heights: a card's first lines measured once for its width.
        let body = egui::TextStyle::Body.resolve(ui.style());
        let small = egui::TextStyle::Small.resolve(ui.style());
        let line = ui.text_style_height(&egui::TextStyle::Body);
        let small_h = ui.text_style_height(&egui::TextStyle::Small);
        let width = ui.available_width();
        let r = &mut self.review;
        if (r.measured - width).abs() > 0.5 {
            r.lines.clear();
            r.measured = width;
        }
        let measured = |&(i, _): &(usize, usize)| r.lines.contains_key(&list[i].id);
        if !order.iter().all(measured) {
            ctx.fonts_mut(|f| {
                for &(i, depth) in order {
                    let a = &list[i];
                    if r.lines.contains_key(&a.id) {
                        continue;
                    }
                    let room = width - BAR - depth as f32 * INDENT - 2.0 * PAD - 4.0;
                    let n = match first_line(a) {
                        Some(t) => {
                            let mut w = 0.0;
                            let over = t.chars().any(|c| {
                                w += f.glyph_width(&body, c);
                                w > room
                            });
                            if over || a.contents.trim().lines().count() > 1 {
                                LINES
                            } else {
                                1
                            }
                        }
                        // Text markup shows the words it marks, on one line.
                        None if !a.quads.is_empty() => 1,
                        None => 0,
                    };
                    r.lines.insert(a.id, n);
                }
            });
        }
        let mut rows = Vec::with_capacity(order.len() + 8);
        let mut names = HashMap::new();
        let mut page = usize::MAX;
        for &(i, d) in order {
            if list[i].page != page {
                page = list[i].page;
                rows.push(Row::Page(page));
                let name = match self.use_labels {
                    true => self.labels.as_ref().and_then(|l| l.get(page).cloned()),
                    false => None,
                };
                names.insert(page, name.unwrap_or_else(|| (page + 1).to_string()));
            }
            rows.push(Row::Card(i, d));
        }
        let head_h = line + 12.0;
        let heights: Vec<f32> = (rows.iter())
            .map(|row| match *row {
                Row::Page(_) => head_h,
                Row::Card(i, _) => {
                    let a = &list[i];
                    let n = r.lines.get(&a.id).copied().unwrap_or(1);
                    let mut h = 2.0 * PAD + line + n as f32 * line;
                    if !sub_line(a).is_empty() {
                        h += small_h + SUB;
                    }
                    if self.ed.draft(a.id).is_some() {
                        h += small_h + 2.0;
                    }
                    if chosen_id == Some(a.id) {
                        h += REPLY + 6.0;
                    }
                    h + GAP
                }
            })
            .collect();
        let n = rows.len();
        let (mut tops, mut ends) = (Vec::with_capacity(n), Vec::with_capacity(n));
        let mut y = 0.0;
        for h in &heights {
            tops.push(y);
            y += h;
            ends.push(y);
        }
        let total = y;
        let mut area = egui::ScrollArea::vertical()
            .id_salt(("comment cards", self.id))
            .auto_shrink([false, false]);
        if std::mem::take(&mut r.seek) {
            let at = rows
                .iter()
                .position(|row| matches!(*row, Row::Card(i, _) if Some(list[i].id) == chosen_id));
            if let (Some(k), Some(view)) = (at, r.view) {
                let (top, bottom) = (tops[k], tops[k] + heights[k]);
                // Only as far as it must, the page's name with it when it is just above.
                let top = match k.checked_sub(1).map(|j| &rows[j]) {
                    Some(Row::Page(_)) => tops[k - 1],
                    _ => top,
                };
                if top < view.min.y {
                    area = area.vertical_scroll_offset(top);
                } else if bottom > view.max.y {
                    area = area.vertical_scroll_offset((bottom - view.height()).max(0.0));
                }
            } else if let Some(k) = at {
                area = area.vertical_scroll_offset(tops[k]);
            }
        }

        let mut chosen = None;
        let mut delete = None;
        let mut reply = false;
        let dark = ui.visuals().dark_mode;
        area.show_viewport(ui, |ui, view| {
            ui.set_min_height(total);
            self.review.view = Some(view);
            let (origin, w) = (ui.min_rect().min, ui.available_width());
            let first = ends.partition_point(|&end| end < view.min.y);
            for (k, row) in rows.iter().enumerate().skip(first) {
                if tops[k] > view.max.y {
                    break;
                }
                let h = heights[k];
                let visuals = ui.visuals().clone();
                match *row {
                    Row::Page(p) => {
                        let name = format!("Page {}", names[&p]);
                        let rect =
                            Rect::from_min_size(origin + Vec2::new(0.0, tops[k]), Vec2::new(w, h));
                        let id = ui.scope_id().with(("comment page", p));
                        let resp = ui.interact(rect, id, Sense::hover());
                        let bold = egui::FontId::new(small.size, theme::semibold());
                        ui.painter().text(
                            rect.left_bottom() + Vec2::new(2.0, -6.0),
                            egui::Align2::LEFT_BOTTOM,
                            &name,
                            bold,
                            theme::muted(dark),
                        );
                        resp.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::Label, true, &name)
                        });
                    }
                    Row::Card(i, depth) => {
                        let a = &list[i];
                        let indent = depth as f32 * INDENT;
                        let rect = Rect::from_min_size(
                            origin + Vec2::new(indent, tops[k]),
                            Vec2::new(w - indent - BAR, h - GAP),
                        );
                        let selected = chosen_id == Some(a.id);
                        let id = ui.scope_id().with(("comment card", a.id));
                        // Clicks only: the list is the Tab stop, not each card.
                        let resp = ui.interact(rect, id, Sense::CLICK);
                        let p = ui.painter().clone();
                        let (fill, edge) = match selected {
                            true => (theme::accent_bg(dark), theme::accent(dark)),
                            false if resp.hovered() => {
                                (visuals.widgets.hovered.weak_bg_fill, theme::border(dark))
                            }
                            false => (theme::surface(dark), theme::border(dark)),
                        };
                        p.rect_filled(rect, 5.0, fill);
                        let width = if selected { 1.5 } else { 1.0 };
                        let stroke = egui::Stroke::new(width, edge);
                        p.rect_stroke(rect, 5.0, stroke, egui::StrokeKind::Inside);
                        if ring && selected {
                            let ring = egui::Stroke::new(2.0, theme::accent(dark));
                            p.rect_stroke(rect.expand(2.0), 7.0, ring, egui::StrokeKind::Outside);
                        }
                        let text = match a.hidden() {
                            true => visuals.weak_text_color(),
                            false => visuals.text_color(),
                        };
                        let muted = theme::muted(dark);
                        let inner = rect.shrink(PAD);
                        // As Word's cards: the kind's icon and who made it, then what it is
                        // and when, small.
                        let kind = kind_label(a);
                        let who = author_name(a);
                        let named = !a.author.trim().is_empty();
                        let date = display_line(&a.modified, 40);
                        let glyph = egui::FontId::new(body.size, theme::icons());
                        let bold = egui::FontId::new(body.size, theme::semibold());
                        let format = |font: &egui::FontId, color| egui::TextFormat {
                            font_id: font.clone(),
                            color,
                            valign: egui::Align::Center,
                            ..Default::default()
                        };
                        let one_row = |job: &mut egui::text::LayoutJob| {
                            job.wrap = egui::text::TextWrapping {
                                max_width: inner.width().max(1.0),
                                max_rows: 1,
                                break_anywhere: true,
                                overflow_character: Some('…'),
                            };
                        };
                        let mut job = egui::text::LayoutJob::default();
                        job.append(kind_icon(a), 0.0, format(&glyph, muted));
                        job.append(if named { &who } else { &kind }, 6.0, format(&bold, text));
                        one_row(&mut job);
                        p.galley(inner.min, p.layout_job(job), text);
                        let mut y = inner.min.y + line;
                        let sub = sub_line(a);
                        if !sub.is_empty() {
                            let mut job =
                                egui::text::LayoutJob::single_section(sub, format(&small, muted));
                            one_row(&mut job);
                            p.galley(egui::pos2(inner.min.x, y), p.layout_job(job), muted);
                            y += small_h + SUB;
                        }
                        // The comment's first lines, or the words a markup marks.
                        let summary = match first_line(a) {
                            Some(_) => a
                                .contents
                                .trim()
                                .lines()
                                .map(|l| display_line(l, 200))
                                .filter(|l| !l.is_empty())
                                .take(LINES)
                                .collect::<Vec<_>>()
                                .join(" "),
                            None if !a.quads.is_empty() => {
                                let r = &mut self.review;
                                let words = marked(&self.pdf, &mut r.marked, &mut r.words, a);
                                match display_line(&words, 200) {
                                    w if w.is_empty() => String::new(),
                                    w => format!("“{w}”"),
                                }
                            }
                            None => String::new(),
                        };
                        let rows_n = self.review.lines.get(&a.id).copied().unwrap_or(1);
                        if !summary.is_empty() {
                            let mut job = egui::text::LayoutJob::single_section(
                                summary.clone(),
                                format(&body, text),
                            );
                            job.wrap = egui::text::TextWrapping {
                                max_width: inner.width().max(1.0),
                                max_rows: rows_n.max(1),
                                break_anywhere: false,
                                overflow_character: Some('…'),
                            };
                            p.galley(egui::pos2(inner.min.x, y), p.layout_job(job), text);
                        }
                        y += rows_n as f32 * line;
                        let draft = self.ed.draft(a.id).is_some();
                        if draft {
                            let mut job = egui::text::LayoutJob::default();
                            let glyph = egui::FontId::new(small.size, theme::icons());
                            job.append(icon::PENCIL_SIMPLE_LINE, 0.0, format(&glyph, muted));
                            job.append("Draft reply", 4.0, format(&small, muted));
                            p.galley(egui::pos2(inner.min.x, y + 2.0), p.layout_job(job), muted);
                            y += small_h + 2.0;
                        }
                        let mut parts = vec![kind.clone()];
                        parts.extend(named.then(|| who.clone()));
                        parts.extend((!date.is_empty()).then(|| date.clone()));
                        let mut name = parts.join(" · ");
                        if !summary.is_empty() {
                            name += &format!(": {summary}");
                        }
                        if draft {
                            name += " · Draft reply";
                        }
                        resp.widget_info(|| {
                            egui::WidgetInfo::selected(egui::Role::Button, true, selected, &name)
                        });
                        let resp = resp.on_hover_text(annot_tip(a));
                        // The selected card's Reply puts the keyboard in the reply box.
                        if selected {
                            let at = Rect::from_min_size(
                                egui::pos2(inner.min.x, y + 6.0),
                                Vec2::new(REPLY * 4.0, REPLY).min(inner.size()),
                            );
                            let glyph = theme::icon(icon::ARROW_BEND_UP_LEFT);
                            let b = ui.put(at, egui::Button::new((glyph, "Reply")));
                            crate::dialogs::set_name(&b, "Reply to this comment");
                            if b.clicked() {
                                chosen = Some(i);
                                reply = true;
                            }
                        }
                        if resp.clicked() {
                            chosen = Some(i);
                        }
                        resp.context_menu(|ui| {
                            let answer = icon::ARROW_BEND_UP_LEFT;
                            if theme::glyph_button(ui, answer, "Reply…").clicked() {
                                chosen = Some(i);
                                reply = true;
                            }
                            let label = if list.iter().any(|b| b.reply_to == Some(a.id)) {
                                "Delete, with its replies"
                            } else {
                                "Delete"
                            };
                            let del = ui
                                .add_enabled(!a.locked(), egui::Button::new(label))
                                .on_disabled_hover_text("It is locked in this document.");
                            if del.clicked() {
                                delete = Some(a.id);
                            }
                        });
                    }
                }
            }
        });
        if let Some(i) = chosen {
            let a = &list[i];
            let (id, page, bbox) = (a.id, a.page, a.bbox);
            self.ed.selection = Selection::Annot { page, id };
            self.review.shown = Some(id);
            self.review.keyed = false;
            self.ed.reply_focus = reply;
            if !reply {
                ctx.memory_mut(|m| m.request_focus(list_id));
            }
            self.show_mark(page, bbox);
        }
        if let Some(id) = delete {
            if self.edit(&ctx, |p| p.remove_annotations(|a| a.id == id).map(|_| ())) {
                self.notice = Some("Deleted the comment".into());
            }
        }
    }

    /// Show a comment's mark on the page: on the page being read, scrolled only as far as
    /// it must be; on another, that page at the mark.
    fn show_mark(&mut self, page: usize, bbox: [f32; 4]) {
        if page == self.current {
            self.reveal = Some((page, bbox));
        } else {
            self.go_to(page);
            self.goto_y = Some(bbox[1]);
        }
    }

    /// The card list's keys, while it has the keyboard: Up and Down to the card before or
    /// after, Home and End to the first and last, each selected with its mark shown; Enter
    /// to the reply box; Esc back to the page, the comment staying selected. Delete is the
    /// command's.
    pub(super) fn comment_keys(&mut self, ctx: &egui::Context) {
        let id = self.comments_id();
        const KEYS: [Key; 6] = [
            Key::ArrowUp,
            Key::ArrowDown,
            Key::Home,
            Key::End,
            Key::Enter,
            Key::Escape,
        ];
        let none = egui::Modifiers::NONE;
        let key = ctx.input_mut(|i| KEYS.into_iter().find(|&k| i.consume_key(none, k)));
        let Some(key) = key else {
            return;
        };
        let order = self.shown_order();
        let list = self.comments.as_ref().map_or(&[][..], |(_, l)| l);
        let at = self
            .ed
            .selection
            .annot()
            .and_then(|(_, id)| order.iter().position(|&(i, _)| list[i].id == id));
        let last = order.len().saturating_sub(1);
        let to = match key {
            Key::Escape => CardKey::Leave,
            Key::Enter => CardKey::Reply,
            _ if order.is_empty() => return,
            Key::ArrowUp => CardKey::To(at.map_or(0, |k| k.saturating_sub(1))),
            Key::ArrowDown => CardKey::To(at.map_or(0, |k| (k + 1).min(last))),
            Key::Home => CardKey::To(0),
            _ => CardKey::To(last),
        };
        match to {
            CardKey::Leave => ctx.memory_mut(|m| m.surrender_focus(id)),
            CardKey::Reply => {
                if self.ed.selection.annot().is_some() {
                    self.ed.reply_focus = true;
                }
            }
            CardKey::To(k) => {
                take_keyboard(ctx, id);
                let a = &list[order[k].0];
                let (id, page, bbox) = (a.id, a.page, a.bbox);
                self.ed.selection = Selection::Annot { page, id };
                (self.review.keyed, self.review.seek) = (true, true);
                self.show_mark(page, bbox);
            }
        }
    }

    /// Previous or Next comment: the card before or after the selected one, as the Comments
    /// view lists them with its filter and search, selected with its mark shown; from
    /// nothing selected, the first on or after the page being read, or the last on or
    /// before it. What to say when there is none.
    pub fn step_comment(&mut self, forward: bool) -> Option<&'static str> {
        let order = self.shown_order();
        if order.is_empty() {
            return Some("No comments to go to");
        }
        let list = self.comments.as_ref().map_or(&[][..], |(_, l)| l);
        let shown: HashSet<usize> = order.iter().map(|&(i, _)| i).collect();
        let sel = self.ed.selection.annot().map(|(_, id)| id);
        let to = match sel.and_then(|id| list.iter().position(|a| a.id == id)) {
            // From the selected comment, shown or not, by its place among them all.
            Some(at) => {
                let all: Vec<usize> = thread_order(list).into_iter().map(|(i, _)| i).collect();
                let k = all.iter().position(|&i| i == at).unwrap_or(0);
                let next = match forward {
                    true => all[k + 1..].iter().find(|i| shown.contains(i)),
                    false => all[..k].iter().rev().find(|i| shown.contains(i)),
                };
                match (next, forward) {
                    (Some(&i), _) => i,
                    (None, true) => return Some("No more comments after this one"),
                    (None, false) => return Some("No more comments before this one"),
                }
            }
            None => {
                let page = self.current;
                let next = match forward {
                    true => order.iter().find(|&&(i, _)| list[i].page >= page),
                    false => order.iter().rev().find(|&&(i, _)| list[i].page <= page),
                };
                match (next, forward) {
                    (Some(&(i, _)), _) => i,
                    (None, true) => return Some("No more comments after this page"),
                    (None, false) => return Some("No more comments before this page"),
                }
            }
        };
        let a = &list[to];
        let (id, page, bbox) = (a.id, a.page, a.bbox);
        self.ed.selection = Selection::Annot { page, id };
        self.review.seek = true;
        self.show_mark(page, bbox);
        None
    }
}

/// Show `item` (`shown`) or hide it, in the set of those hidden.
fn toggle(hidden: &mut BTreeSet<String>, item: &str, shown: bool) {
    if shown {
        hidden.remove(item);
    } else {
        hidden.insert(item.to_string());
    }
}

/// A comment's author as the list names them.
fn author_name(a: &Annot) -> String {
    match display_line(&a.author, 80) {
        n if n.is_empty() => "(No name)".into(),
        n => n,
    }
}

/// A card's second line: what the comment is, when its author heads the card, and when it
/// was last changed.
fn sub_line(a: &Annot) -> String {
    let mut parts = Vec::new();
    if !a.author.trim().is_empty() {
        parts.push(kind_label(a));
    }
    parts.extend(Some(display_line(&a.modified, 40)).filter(|d| !d.is_empty()));
    parts.join(" · ")
}

/// The kind a card names: a reply is a reply, whatever it is made as.
fn kind_label(a: &Annot) -> String {
    match a.reply_to {
        Some(_) => "Reply".into(),
        None => a.label(),
    }
}

/// The icon a card shows for its kind, as the tools that add them have.
fn kind_icon(a: &Annot) -> &'static str {
    if a.reply_to.is_some() {
        return icon::ARROW_BEND_UP_LEFT;
    }
    match a.kind {
        AnnotKind::Highlight => icon::HIGHLIGHTER,
        AnnotKind::Underline => icon::TEXT_UNDERLINE,
        AnnotKind::StrikeOut => icon::TEXT_STRIKETHROUGH,
        AnnotKind::Squiggly => icon::WAVE_SINE,
        AnnotKind::Note => icon::NOTE,
        AnnotKind::TextBox => icon::TEXTBOX,
        AnnotKind::Square => icon::SQUARE,
        AnnotKind::Circle => icon::CIRCLE,
        AnnotKind::Line => icon::LINE_SEGMENT,
        AnnotKind::Polygon | AnnotKind::PolyLine | AnnotKind::Ink => icon::PENCIL_SIMPLE,
        AnnotKind::Stamp if a.label() != a.kind.label() => icon::SIGNATURE,
        AnnotKind::Stamp => icon::STAMP,
        AnnotKind::Redact => icon::EYE_SLASH,
        AnnotKind::Other(_) => icon::CHAT_TEXT,
    }
}

/// A comment's first line with something on it, as shown.
fn first_line(a: &Annot) -> Option<String> {
    a.contents
        .lines()
        .map(|l| display_line(l, 200))
        .find(|l| !l.is_empty())
}

/// The words text markup `a` marks, read from its page's text once (`words`, by page) and
/// kept (`marked`, by the markup).
fn marked(
    pdf: &Pdf,
    marked: &mut HashMap<(u32, u16), String>,
    words: &mut HashMap<usize, Vec<PageWord>>,
    a: &Annot,
) -> String {
    if let Some(m) = marked.get(&a.id) {
        return m.clone();
    }
    let page = words
        .entry(a.page)
        .or_insert_with(|| pdf.page_words(a.page).unwrap_or_default());
    let under: Vec<PageWord> = (page.iter())
        .filter(|w| {
            let (x, y) = ((w.bbox[0] + w.bbox[2]) / 2.0, (w.bbox[1] + w.bbox[3]) / 2.0);
            a.quads
                .iter()
                .any(|q| x >= q[0] && x <= q[2] && y >= q[1] && y <= q[3])
        })
        .cloned()
        .collect();
    let text = words_text(&under);
    marked.insert(a.id, text.clone());
    text
}
