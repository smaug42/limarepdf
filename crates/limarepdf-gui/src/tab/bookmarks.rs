//! The Bookmarks view: the outline indented by level, with a caret to show or hide the
//! bookmarks inside one. A click goes to a bookmark's page; a double-click or F2 renames
//! it. As Word's outline and Navigation pane move headings (ENH-16), a bookmark is dragged
//! before, after or into another, or moved with Alt+Shift and an arrow or from its
//! right-click menu, each move one undo step, the bookmarks inside it going with it. The
//! list is one stop for Tab and takes a tree's keys, as the thumbnails and the comment
//! cards take theirs. Which bookmarks are open in the list is the list's, kept by bookmark
//! across edits, never written into the file.

use super::organise::take_keyboard;
use super::Tab;
use crate::icon;
use crate::theme;
use eframe::egui::{self, Key, Pos2, Rect, Sense, Vec2};
use limarepdf_core::OutlineItem;
use limarepdf_core::{bookmark_moves, bookmark_step, display_line, BookmarkPlace, BookmarkStep};
use std::collections::HashMap;

/// A bookmark's object: it keeps it when moved, renamed or opened, and through Undo.
type Obj = (u32, u16);

/// How far in each level is, the caret's room before a title, the space between rows,
/// and the room kept clear of the scroll bar on the right.
const INDENT: f32 = 12.0;
const CARET: f32 = 16.0;
const GAP: f32 = 2.0;
const BAR: f32 = 10.0;

/// Word's outline keys: Alt+Shift with an arrow.
const MOVE: egui::Modifiers = egui::Modifiers::ALT.plus(egui::Modifiers::SHIFT);
const STEPS: [(Key, BookmarkStep); 4] = [
    (Key::ArrowUp, BookmarkStep::Up),
    (Key::ArrowDown, BookmarkStep::Down),
    (Key::ArrowLeft, BookmarkStep::Promote),
    (Key::ArrowRight, BookmarkStep::Demote),
];

/// A tab's Bookmarks view.
#[derive(Default)]
pub struct Outline {
    /// The bookmarks, read when first shown and again after the document changes.
    read: Option<Read>,
    /// Bookmarks opened or closed in the list; the others as the file has them.
    open: HashMap<Obj, bool>,
    /// The bookmark selected, wherever it moves; a new one goes after it.
    sel: Option<Obj>,
    /// A bookmark being renamed, and its title so far; `focus` until the box has been
    /// given the keyboard.
    rename: Option<(usize, String)>,
    focus: bool,
    /// The bookmark being dragged.
    drag: Option<Obj>,
    /// The keys moved the selection: its row is ringed while the list has the keyboard.
    keyed: bool,
    /// Bring the selected bookmark into view when next drawn, opening the ones it is in.
    seek: bool,
    /// The part of the list in view last frame, in its own points.
    view: Option<Rect>,
}

/// The bookmarks as read at `rev`: each one's object and what it is.
struct Read {
    rev: u64,
    ids: Vec<Obj>,
    items: Vec<OutlineItem>,
}

impl Read {
    fn is_open(&self, open: &HashMap<Obj, bool>, i: usize) -> bool {
        let set = open.get(&self.ids[i]).copied();
        set.unwrap_or(self.items[i].open)
    }

    /// The positions of the bookmarks shown: those not inside a closed one.
    fn rows(&self, open: &HashMap<Obj, bool>) -> Vec<usize> {
        let mut rows = Vec::new();
        let mut closed_at: Option<usize> = None;
        for (i, item) in self.items.iter().enumerate() {
            match closed_at {
                Some(d) if item.depth > d => continue,
                _ => closed_at = None,
            }
            rows.push(i);
            if item.has_children && !self.is_open(open, i) {
                closed_at = Some(item.depth);
            }
        }
        rows
    }

    /// The bookmark bookmark `i` is in.
    fn parent(&self, i: usize) -> Option<usize> {
        let depth = self.items[i].depth;
        (0..i).rev().find(|&j| self.items[j].depth < depth)
    }

    /// Where a bookmark dragged from `from` goes when dropped `y` points down the list
    /// (`rows` as shown, `pitch` apart, each `high`): onto the top third of a row before
    /// it, the middle inside it, the bottom third after it (before the first inside it when
    /// those are shown), below the rows last of all. `None` where it can't go or would stay.
    fn drop_place(
        &self,
        open: &HashMap<Obj, bool>,
        rows: &[usize],
        from: usize,
        y: f32,
        (pitch, high): (f32, f32),
    ) -> Option<BookmarkPlace> {
        if y < 0.0 {
            return None;
        }
        let k = (y / pitch) as usize;
        let place = match rows.get(k) {
            None => BookmarkPlace::End,
            Some(&i) => {
                let part = (y - k as f32 * pitch) / high;
                let inside = self.items[i].has_children && self.is_open(open, i);
                match part {
                    p if p < 1.0 / 3.0 => BookmarkPlace::Before(i),
                    p if p <= 2.0 / 3.0 => BookmarkPlace::Under(i),
                    _ if inside => BookmarkPlace::Before(i + 1),
                    _ => BookmarkPlace::After(i),
                }
            }
        };
        bookmark_moves(&self.items, from, place)
            .is_ok_and(|moves| moves)
            .then_some(place)
    }
}

/// Why a step can't be taken: said when its key is pressed, and on its greyed menu item.
fn cant(step: BookmarkStep) -> &'static str {
    match step {
        BookmarkStep::Up => "Already first at its level",
        BookmarkStep::Down => "Already last at its level",
        BookmarkStep::Promote => "Already at the top level",
        BookmarkStep::Demote => "No bookmark above it at its level to go into",
    }
}

/// What the list was asked to do this frame, done once it is drawn.
enum Asked {
    GoTo(usize),
    Toggle(usize),
    Rename(usize),
    Renamed(usize, String),
    StopRenaming,
    Delete(usize),
    Add(BookmarkPlace),
    Step(usize, BookmarkStep),
    Move(usize, BookmarkPlace),
}

impl Tab {
    /// The Bookmarks view's keyboard stop: the list.
    pub fn outline_id(&self) -> egui::Id {
        egui::Id::unique(("bookmark-list", self.id))
    }

    /// The bookmark selected in the Bookmarks view, by its object.
    #[cfg(test)]
    pub fn outline_selected(&self) -> Option<Obj> {
        self.outline.sel
    }

    /// Read the bookmarks again after the document changed. A selected one no longer
    /// there is let go.
    fn read_outline(&mut self) {
        let rev = self.revision;
        if self.outline.read.as_ref().is_some_and(|r| r.rev == rev) {
            return;
        }
        let (ids, items): (Vec<Obj>, Vec<OutlineItem>) =
            self.pdf.outline_nodes().into_iter().unzip();
        let o = &mut self.outline;
        if o.sel.is_some_and(|s| !ids.contains(&s)) {
            o.sel = None;
        }
        if o.rename.as_ref().is_some_and(|(i, _)| *i >= ids.len()) {
            o.rename = None;
        }
        o.read = Some(Read { rev, ids, items });
    }

    /// Open the bookmarks the selected one is in, so it shows.
    fn open_to_selected(&mut self) {
        let o = &mut self.outline;
        let Some(read) = &o.read else {
            return;
        };
        let Some(mut i) = o.sel.and_then(|s| read.ids.iter().position(|&id| id == s)) else {
            return;
        };
        while let Some(p) = read.parent(i) {
            o.open.insert(read.ids[p], true);
            i = p;
        }
    }

    /// The Bookmarks side tab.
    pub fn show_outline(&mut self, ui: &mut egui::Ui) {
        self.read_outline();
        let ctx = ui.ctx().clone();
        if self.outline.seek {
            self.open_to_selected();
        }
        let Some(read) = self.outline.read.take() else {
            return;
        };
        let sel = (self.outline.sel).and_then(|s| read.ids.iter().position(|&id| id == s));
        let mut asked = Vec::new();
        let add = theme::glyph_button(ui, icon::PLUS, "Add bookmark")
            .on_hover_text(match sel {
                Some(_) => "Bookmark the current page, after the selected bookmark",
                None => "Bookmark the current page",
            })
            .clicked();
        if add {
            asked.push(Asked::Add(match sel {
                Some(s) => BookmarkPlace::After(s),
                None => BookmarkPlace::End,
            }));
        }
        if read.items.is_empty() {
            ui.weak("This document has no bookmarks.");
        }

        // The list is one stop for Tab, which takes the arrows and Esc for the bookmarks;
        // the rows take clicks and drags.
        let list_id = self.outline_id();
        let stop = ui.interact(ui.available_rect_before_wrap(), list_id, Sense::FOCUSABLE);
        stop.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Tree, true, "Bookmark list"));
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
            self.outline.keyed = true;
        }
        let ring = stop.has_focus() && self.outline.keyed;

        let rows = read.rows(&self.outline.open);
        let high = ui.spacing().interact_size.y;
        let pitch = high + GAP;
        let mut area = egui::ScrollArea::vertical()
            .id_salt(("bookmarks", self.id))
            .auto_shrink([false, false]);
        if std::mem::take(&mut self.outline.seek) {
            let at = sel.and_then(|s| rows.iter().position(|&r| r == s));
            if let Some(k) = at {
                let (top, bottom) = (k as f32 * pitch, k as f32 * pitch + high);
                match self.outline.view {
                    Some(v) if top < v.min.y => area = area.vertical_scroll_offset(top),
                    Some(v) if bottom > v.max.y => {
                        let to = (bottom - v.height()).max(0.0);
                        area = area.vertical_scroll_offset(to);
                    }
                    Some(_) => {}
                    None => area = area.vertical_scroll_offset(top),
                }
            }
        }
        let dark = ui.visuals().dark_mode;
        let mut editing = self.outline.rename.take();
        let mut focus = std::mem::take(&mut self.outline.focus);
        let dragging = self.outline.drag;
        let mut drag = dragging;
        area.show_viewport(ui, |ui, view| {
            ui.set_min_height(rows.len() as f32 * pitch);
            self.outline.view = Some(view);
            let (origin, w) = (ui.min_rect().min, ui.available_width());
            let first = (view.min.y / pitch).max(0.0) as usize;
            let last = ((view.max.y / pitch) as usize + 1).min(rows.len());
            for (k, &i) in rows.iter().enumerate().take(last).skip(first) {
                let (obj, item) = (read.ids[i], &read.items[i]);
                let top = origin.y + k as f32 * pitch;
                let left = origin.x + item.depth as f32 * INDENT;
                let caret = Rect::from_min_size(Pos2::new(left, top), Vec2::new(CARET, high));
                let right = (origin.x + w - BAR).max(caret.max.x + 20.0);
                let rect = Rect::from_min_max(caret.right_top(), Pos2::new(right, top + high));
                if item.has_children {
                    let open = read.is_open(&self.outline.open, i);
                    let id = egui::Id::unique(("bookmark caret", self.id, obj));
                    let r = ui.interact(caret, id, Sense::CLICK);
                    let what = if open { "Collapse" } else { "Expand" };
                    let name = format!("{what} {}", item.title);
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
                    let glyph = if open {
                        icon::CARET_DOWN
                    } else {
                        icon::CARET_RIGHT
                    };
                    let colour = match r.hovered() {
                        true => ui.visuals().strong_text_color(),
                        false => ui.visuals().text_color(),
                    };
                    let font = egui::FontId::new(10.0, theme::icons());
                    let (p, middle) = (ui.painter(), egui::Align2::CENTER_CENTER);
                    p.text(caret.center(), middle, glyph, font, colour);
                    if r.clicked() {
                        asked.push(Asked::Toggle(i));
                    }
                }
                if let Some((at, title)) = editing.as_mut().filter(|(at, _)| *at == i) {
                    let typing = egui::TextEdit::singleline(title).desired_width(rect.width());
                    let r = ui.put(rect, typing);
                    if std::mem::take(&mut focus) {
                        r.request_focus();
                    }
                    let (esc, enter) =
                        ui.input(|i| (i.key_pressed(Key::Escape), i.key_pressed(Key::Enter)));
                    if esc {
                        asked.push(Asked::StopRenaming);
                    } else if r.lost_focus() {
                        let title = title.trim().to_string();
                        if !title.is_empty() && title != item.title {
                            asked.push(Asked::Renamed(*at, title));
                        }
                        asked.push(Asked::StopRenaming);
                    }
                    // From the keyboard, the keyboard goes back to the list.
                    if esc || enter {
                        ctx.memory_mut(|m| m.request_focus(list_id));
                    }
                    continue;
                }
                // Clicks and drags only: the list is the Tab stop, not each row.
                let id = egui::Id::unique(("bookmark", self.id, obj));
                let resp = ui.interact(rect, id, Sense::CLICK | Sense::DRAG);
                let shown = display_line(&item.title, 300);
                let selected = sel == Some(i);
                resp.widget_info(|| {
                    let mut info = egui::WidgetInfo::labeled(egui::Role::TreeItem, true, &shown);
                    info.selected = Some(selected);
                    info
                });
                let p = ui.painter();
                let fill = match selected {
                    true => theme::accent_bg(dark),
                    false if resp.hovered() && dragging.is_none() => {
                        ui.visuals().widgets.hovered.weak_bg_fill
                    }
                    false => egui::Color32::TRANSPARENT,
                };
                p.rect_filled(rect, 3.0, fill);
                if ring && selected {
                    let ring = egui::Stroke::new(2.0, theme::accent(dark));
                    p.rect_stroke(rect, 3.0, ring, egui::StrokeKind::Inside);
                }
                let mut text = egui::RichText::new(&shown);
                if item.page.is_none() {
                    text = text.weak();
                }
                let galley = egui::WidgetText::from(text).into_galley(
                    ui,
                    Some(egui::TextWrapMode::Truncate),
                    rect.width() - 8.0,
                    egui::TextStyle::Button,
                );
                let at = Pos2::new(rect.min.x + 4.0, rect.center().y - galley.size().y / 2.0);
                ui.painter().galley(at, galley, ui.visuals().text_color());
                if resp.drag_started() {
                    drag = Some(obj);
                    self.outline.sel = Some(obj);
                }
                if resp.secondary_clicked() {
                    self.outline.sel = Some(obj);
                }
                if resp.clicked() {
                    asked.push(Asked::GoTo(i));
                }
                if resp.double_clicked() {
                    asked.push(Asked::Rename(i));
                }
                let menu = |ui: &mut egui::Ui, asked: &mut Vec<Asked>| {
                    if theme::glyph_button(ui, icon::PENCIL_SIMPLE, "Rename").clicked() {
                        asked.push(Asked::Rename(i));
                    }
                    ui.separator();
                    let moves = [
                        (icon::ARROW_UP, "Move up"),
                        (icon::ARROW_DOWN, "Move down"),
                        (icon::TEXT_OUTDENT, "Promote"),
                        (icon::TEXT_INDENT, "Demote"),
                    ];
                    for ((glyph, label), (key, step)) in moves.into_iter().zip(STEPS) {
                        let shortcut = egui::KeyboardShortcut::new(MOVE, key);
                        let keys = crate::actions::key_text(ui.ctx(), &shortcut);
                        let can = bookmark_step(&read.items, i, step).is_some();
                        let b = egui::Button::new((theme::icon(glyph), label)).shortcut_text(&keys);
                        let r = ui.add_enabled(can, b).on_disabled_hover_text(cant(step));
                        crate::dialogs::set_name(&r, &format!("{label} {keys}"));
                        if r.clicked() {
                            asked.push(Asked::Step(i, step));
                        }
                    }
                    ui.separator();
                    let under = "Add bookmark under this";
                    if theme::glyph_button(ui, icon::PLUS, under).clicked() {
                        asked.push(Asked::Add(BookmarkPlace::Under(i)));
                    }
                    let label = match item.has_children {
                        true => "Delete, with its children",
                        false => "Delete",
                    };
                    if theme::glyph_button(ui, icon::TRASH, label).clicked() {
                        asked.push(Asked::Delete(i));
                    }
                };
                let resp = match dragging {
                    None => resp.on_hover_text(match item.page {
                        Some(p) => format!("{shown}\nPage {}", self.page_name(p)),
                        None => format!("{shown}\nGoes outside this document"),
                    }),
                    Some(_) => resp,
                };
                resp.context_menu(|ui| menu(ui, &mut asked));
            }

            // A drag: where it would go shown as a line between rows, or the row it would
            // go into outlined; let go there, it goes.
            let Some(obj) = dragging else {
                return;
            };
            let from = read.ids.iter().position(|&id| id == obj);
            let (pointer, down, released) = ctx.input(|i| {
                let p = &i.pointer;
                (p.interact_pos(), p.any_down(), p.any_released())
            });
            let clip = ui.clip_rect();
            let place = match (from, pointer) {
                (Some(from), Some(p)) if clip.contains(p) => {
                    let open = &self.outline.open;
                    read.drop_place(open, &rows, from, p.y - origin.y, (pitch, high))
                }
                _ => None,
            };
            let row_of = |i: usize| rows.iter().position(|&r| r == i);
            let accent = theme::accent(dark);
            let line = |y: f32, depth: usize| {
                let x = origin.x + depth as f32 * INDENT + CARET;
                let span = egui::Rangef::new(x, origin.x + w - BAR);
                ui.painter().hline(span, y, egui::Stroke::new(2.0, accent));
            };
            let row_top = |k: usize| origin.y + k as f32 * pitch;
            match place {
                Some(BookmarkPlace::Before(i)) => {
                    let k = row_of(i).unwrap_or(0);
                    line(row_top(k) - GAP / 2.0, read.items[i].depth);
                }
                Some(BookmarkPlace::After(i)) => {
                    let k = row_of(i).unwrap_or(0);
                    line(row_top(k) + high + GAP / 2.0, read.items[i].depth);
                }
                Some(BookmarkPlace::Under(i)) => {
                    let k = row_of(i).unwrap_or(0);
                    let left = origin.x + read.items[i].depth as f32 * INDENT + CARET;
                    let top = Pos2::new(left, row_top(k));
                    let r = Rect::from_min_max(top, Pos2::new(origin.x + w - BAR, top.y + high));
                    let (edge, inside) = (egui::Stroke::new(2.0, accent), egui::StrokeKind::Inside);
                    ui.painter().rect_stroke(r, 3.0, edge, inside);
                }
                Some(BookmarkPlace::End) => line(row_top(rows.len()) - GAP / 2.0, 0),
                None => {}
            }
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            if let Some(p) = pointer {
                if p.y < clip.top() + 24.0 {
                    ui.scroll_with_delta(Vec2::new(0.0, 6.0));
                } else if p.y > clip.bottom() - 24.0 {
                    ui.scroll_with_delta(Vec2::new(0.0, -6.0));
                }
            }
            ctx.request_repaint();
            if released || !down {
                drag = None;
                if let (Some(from), Some(place)) = (from, place.filter(|_| released)) {
                    asked.push(Asked::Move(from, place));
                }
            }
        });
        self.outline.drag = drag;
        self.outline.rename = editing;
        self.outline.focus = focus;
        let ids = read.ids.clone();
        let titles = |i: usize| read.items[i].title.clone();
        let pages: Vec<Option<usize>> = asked
            .iter()
            .map(|a| match a {
                Asked::GoTo(i) => read.items[*i].page,
                _ => None,
            })
            .collect();
        let renames: Vec<Option<String>> = asked
            .iter()
            .map(|a| match a {
                Asked::Rename(i) => Some(titles(*i)),
                _ => None,
            })
            .collect();
        self.outline.read = Some(read);
        for ((a, page), title) in asked.into_iter().zip(pages).zip(renames) {
            match a {
                Asked::GoTo(i) => {
                    self.outline.sel = Some(ids[i]);
                    self.outline.keyed = false;
                    ctx.memory_mut(|m| m.request_focus(list_id));
                    if let Some(p) = page {
                        self.go_to(p);
                    }
                }
                Asked::Toggle(i) => {
                    let open = (self.outline.read.as_ref())
                        .is_some_and(|r| r.is_open(&self.outline.open, i));
                    self.outline.open.insert(ids[i], !open);
                }
                Asked::Rename(i) => {
                    self.outline.rename = title.map(|t| (i, t));
                    self.outline.focus = true;
                }
                Asked::Renamed(i, title) => {
                    self.edit(&ctx, |p| p.rename_bookmark(i, &title));
                }
                Asked::StopRenaming => self.outline.rename = None,
                Asked::Delete(i) => self.delete_bookmark(&ctx, i),
                Asked::Add(place) => self.add_bookmark(&ctx, place),
                Asked::Step(i, step) => {
                    self.outline.sel = Some(ids[i]);
                    self.step_bookmark(&ctx, i, step);
                }
                Asked::Move(i, place) => self.move_bookmark(&ctx, i, place),
            }
        }
    }

    /// Bookmark the page being read at `place`, its title open for typing.
    fn add_bookmark(&mut self, ctx: &egui::Context, place: BookmarkPlace) {
        let page = self.current;
        let title = format!("Page {}", self.page_name(page));
        let mut added = None;
        let ok = self.edit(ctx, |p| {
            added = Some(p.add_bookmark(&title, page, place)?);
            Ok(())
        });
        if let Some(i) = added.filter(|_| ok) {
            self.outline.sel = self.pdf.outline_nodes().get(i).map(|(id, _)| *id);
            self.outline.rename = Some((i, title));
            (self.outline.focus, self.outline.seek) = (true, true);
        }
    }

    /// Delete bookmark `i`, with the bookmarks inside it.
    fn delete_bookmark(&mut self, ctx: &egui::Context, i: usize) {
        if self.edit(ctx, |p| p.remove_bookmark(i)) {
            self.outline.sel = None;
            self.notice = Some("Deleted the bookmark".into());
        }
    }

    /// Move bookmark `i` to `place` in one undo step, it staying selected and in view:
    /// nothing when it is there already, and why not when it can't go there. Bookmarks
    /// aren't drawn on the pages, so nothing is redrawn.
    fn move_bookmark(&mut self, ctx: &egui::Context, i: usize, place: BookmarkPlace) {
        let Some(read) = &self.outline.read else {
            return;
        };
        match bookmark_moves(&read.items, i, place) {
            Err(e) => {
                self.info = Some(format!("Can't move it there: {e}"));
                return;
            }
            Ok(false) => return,
            Ok(true) => {}
        }
        self.outline.sel = Some(read.ids[i]);
        let moved = |p: &mut limarepdf_core::Pdf| p.move_bookmark(i, place).map(drop);
        if self.keep_page_typing(ctx) && self.edit_setting(ctx, None, moved) {
            self.outline.seek = true;
        }
    }

    /// Move bookmark `i` a step, as Word's outline keys do, or say why it can't go.
    fn step_bookmark(&mut self, ctx: &egui::Context, i: usize, step: BookmarkStep) {
        let place = (self.outline.read.as_ref()).and_then(|r| bookmark_step(&r.items, i, step));
        match place {
            Some(place) => self.move_bookmark(ctx, i, place),
            None => self.info = Some(cant(step).into()),
        }
    }

    /// The list's keys, while it has the keyboard: Up and Down to the bookmark before or
    /// after as shown, Home and End to the first and last, each selected and its page
    /// shown; Right opens a closed bookmark, then goes into it; Left closes an open one,
    /// then goes to the one it is in; Enter shows its page again; F2 renames it and Delete
    /// deletes it; Alt+Shift with an arrow moves it; Esc gives the keyboard back to the
    /// page, it staying selected.
    pub(super) fn outline_keys(&mut self, ctx: &egui::Context) {
        let id = self.outline_id();
        let step = ctx.input_mut(|i| {
            let mut steps = STEPS.into_iter();
            steps.find(|&(k, _)| i.consume_key(MOVE, k)).map(|(_, s)| s)
        });
        const KEYS: [Key; 10] = [
            Key::ArrowUp,
            Key::ArrowDown,
            Key::ArrowLeft,
            Key::ArrowRight,
            Key::Home,
            Key::End,
            Key::Enter,
            Key::F2,
            Key::Delete,
            Key::Escape,
        ];
        let none = egui::Modifiers::NONE;
        let key = match step {
            Some(_) => None,
            None => ctx.input_mut(|i| KEYS.into_iter().find(|&k| i.consume_key(none, k))),
        };
        if step.is_none() && key.is_none() {
            return;
        }
        self.read_outline();
        let Some(read) = &self.outline.read else {
            return;
        };
        let open = &self.outline.open;
        let sel = (self.outline.sel).and_then(|s| read.ids.iter().position(|&id| id == s));
        take_keyboard(ctx, id);
        self.outline.keyed = true;
        if let Some(step) = step {
            if let Some(i) = sel {
                self.step_bookmark(ctx, i, step);
            }
            return;
        }
        let rows = read.rows(open);
        let at = sel.and_then(|s| rows.iter().position(|&r| r == s));
        let last = rows.len().saturating_sub(1);
        let shut = |i: usize| (read.items[i].has_children, read.is_open(open, i));
        let to = match (key, sel) {
            (Some(Key::Escape), _) => {
                ctx.memory_mut(|m| m.surrender_focus(id));
                return;
            }
            (Some(Key::F2), Some(s)) => {
                self.outline.rename = Some((s, read.items[s].title.clone()));
                self.outline.focus = true;
                return;
            }
            (Some(Key::Delete), Some(s)) => {
                self.delete_bookmark(ctx, s);
                return;
            }
            _ if rows.is_empty() => return,
            (Some(Key::ArrowRight), Some(s)) => match shut(s) {
                (true, true) => s + 1,
                (true, false) => {
                    self.outline.open.insert(read.ids[s], true);
                    return;
                }
                (false, _) => return,
            },
            (Some(Key::ArrowLeft), Some(s)) => match shut(s) {
                (true, true) => {
                    self.outline.open.insert(read.ids[s], false);
                    return;
                }
                _ => match read.parent(s) {
                    Some(p) => p,
                    None => return,
                },
            },
            (Some(Key::Enter), Some(s)) => s,
            (Some(Key::ArrowUp), _) => rows[at.map_or(0, |k| k.saturating_sub(1))],
            (Some(Key::ArrowDown), _) => rows[at.map_or(0, |k| (k + 1).min(last))],
            (Some(Key::Home), _) => rows[0],
            (Some(Key::End), _) => rows[last],
            _ => return,
        };
        let page = read.items[to].page;
        self.outline.sel = Some(read.ids[to]);
        self.outline.seek = true;
        if let Some(p) = page {
            self.go_to(p);
        }
    }
}
