//! Content editing state and interaction: selecting, moving and resizing objects, inserting text, images
//! and shapes, and the inspector panel.

use crate::fields::FormUi;
use crate::selection::{ObjKind, Selection, WordAt};
use crate::theme;
use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use egui_phosphor::regular as icon;
use mpe_core::{
    words_text, Annot, Family, FieldValue, FontChoice, Link, LinkTarget, ObjectKind, PageObject,
    PageWord, PathStyle, Shape, TextEdit,
};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    /// The default: select words (visible or recognised) to copy, follow links, fill in
    /// forms. Never changes page content.
    Read,
    Select,
    Text,
    Image,
    Rect,
    Ellipse,
    Line,
}

/// A change requested by the interface, applied by the tab after the frame is drawn.
pub enum Action {
    SetBox(usize, usize, [f32; 4]),
    Delete(usize, usize),
    AddShape(usize, Shape, [f32; 4]),
    AddText(usize, (f32, f32), String),
    AddImage(usize, [f32; 4]),
    ReplaceImage(usize, usize, Vec<u8>),
    EditText(usize, usize, TextEdit),
    PathStyle(usize, usize, PathStyle),
    /// Set the named form field, drawn on these pages.
    SetField(Vec<usize>, String, FieldValue),
    /// Change an annotation on a page: its colour, opacity and comment.
    SetAnnot(usize, (u32, u16), AnnotEdit),
    DeleteAnnot(usize, (u32, u16)),
}

/// An annotation's properties as the inspector edits them.
#[derive(Clone, PartialEq)]
pub struct AnnotEdit {
    pub color: Option<[f32; 3]>,
    pub opacity: f32,
    pub contents: String,
}

#[derive(Clone, Copy)]
enum Mode {
    Move,
    Handle(usize),
}

struct Drag {
    page: usize,
    id: usize,
    mode: Mode,
    start: Pos2,
    orig: [f32; 4],
    cur: [f32; 4],
}

struct Draw {
    page: usize,
    start: Pos2,
    cur: Pos2,
}

/// Working copy of the selected object's properties shown in the inspector.
struct Insp {
    key: (usize, usize),
    text: String,
    font: FontChoice,
    size: f32,
    color: [f32; 3],
    fill_on: bool,
    fill: [f32; 3],
    stroke_on: bool,
    stroke: [f32; 3],
    width: f32,
    geom: [f32; 4],
    orig_font: FontChoice,
    orig_size: f32,
    orig_color: [f32; 3],
}

pub struct Editor {
    pub tool: Tool,
    /// Pages, an object or words; see [`Selection`].
    pub selection: Selection,
    objs: HashMap<usize, Vec<PageObject>>,
    unavailable: HashMap<usize, String>,
    drag: Option<Drag>,
    draw: Option<Draw>,
    pub new_text: Option<(usize, (f32, f32))>,
    text_buf: String,
    focus_text: bool,
    pub tfont: FontChoice,
    pub tsize: f32,
    pub tcolor: [f32; 3],
    pub fill_on: bool,
    pub fill: [f32; 3],
    pub stroke_on: bool,
    pub stroke: [f32; 3],
    pub width: f32,
    pub pending_image: Option<(Vec<u8>, f32)>,
    insp: Option<Insp>,
    pub actions: Vec<Action>,
    /// Outline invisible (OCR) text and let it be selected and corrected.
    pub show_hidden: bool,
    words: HashMap<usize, Vec<PageWord>>,
    /// A word selection is being dragged out.
    text_drag: bool,
    links: HashMap<usize, Vec<Link>>,
    /// A link was clicked; the tab follows it.
    pub follow: Option<LinkTarget>,
    /// Filling in form fields.
    pub form: FormUi,
    annots: HashMap<usize, Vec<Annot>>,
    /// The selected annotation's properties being edited in the inspector, with its id.
    annot_insp: Option<((u32, u16), AnnotEdit)>,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            tool: Tool::Read,
            selection: Selection::None,
            objs: HashMap::new(),
            unavailable: HashMap::new(),
            drag: None,
            draw: None,
            new_text: None,
            text_buf: String::new(),
            focus_text: false,
            tfont: FontChoice::default(),
            tsize: 14.0,
            tcolor: [0.0; 3],
            fill_on: true,
            fill: [0.95, 0.75, 0.2],
            stroke_on: true,
            stroke: [0.1, 0.1, 0.1],
            width: 1.5,
            pending_image: None,
            insp: None,
            actions: Vec::new(),
            show_hidden: false,
            words: HashMap::new(),
            text_drag: false,
            links: HashMap::new(),
            follow: None,
            form: FormUi::default(),
            annots: HashMap::new(),
            annot_insp: None,
        }
    }
}

/// Hover text for an annotation: its kind, author and date, then its comment.
pub fn annot_tip(a: &Annot) -> String {
    let mut tip = a.kind.label();
    for extra in [&a.author, &a.modified] {
        if !extra.is_empty() {
            tip += &format!(" · {extra}");
        }
    }
    if a.hidden() {
        tip += " · hidden";
    }
    if !a.contents.is_empty() {
        tip += &format!("\n\n{}", a.contents);
    }
    tip
}

fn norm(r: [f32; 4]) -> [f32; 4] {
    [
        r[0].min(r[2]),
        r[1].min(r[3]),
        r[0].max(r[2]),
        r[1].max(r[3]),
    ]
}

/// The smallest box holding both.
fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn handle_pos(b: [f32; 4], h: usize) -> (f32, f32) {
    let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
    match h {
        0 => (b[0], b[1]),
        1 => (cx, b[1]),
        2 => (b[2], b[1]),
        3 => (b[2], cy),
        4 => (b[2], b[3]),
        5 => (cx, b[3]),
        6 => (b[0], b[3]),
        _ => (b[0], cy),
    }
}

fn handle_cursor(h: usize) -> CursorIcon {
    match h {
        0 | 4 => CursorIcon::ResizeNwSe,
        2 | 6 => CursorIcon::ResizeNeSw,
        1 | 5 => CursorIcon::ResizeVertical,
        _ => CursorIcon::ResizeHorizontal,
    }
}

fn resize(orig: [f32; 4], h: usize, dx: f32, dy: f32, keep_aspect: bool) -> [f32; 4] {
    let mut b = orig;
    if matches!(h, 0 | 6 | 7) {
        b[0] += dx;
    }
    if matches!(h, 2..=4) {
        b[2] += dx;
    }
    if matches!(h, 0..=2) {
        b[1] += dy;
    }
    if matches!(h, 4..=6) {
        b[3] += dy;
    }
    if keep_aspect && matches!(h, 0 | 2 | 4 | 6) {
        let (ow, oh) = ((orig[2] - orig[0]).max(0.01), (orig[3] - orig[1]).max(0.01));
        let (w, hh) = ((b[2] - b[0]).abs(), (b[3] - b[1]).abs());
        let s = (w / ow).max(hh / oh);
        let (nw, nh) = (ow * s, oh * s);
        match h {
            0 => {
                b[0] = orig[2] - nw;
                b[1] = orig[3] - nh;
            }
            2 => {
                b[2] = orig[0] + nw;
                b[1] = orig[3] - nh;
            }
            4 => {
                b[2] = orig[0] + nw;
                b[3] = orig[1] + nh;
            }
            _ => {
                b[0] = orig[2] - nw;
                b[3] = orig[1] + nh;
            }
        }
    }
    norm(b)
}

impl Editor {
    /// Forget cached object lists (after any change to the document).
    pub fn invalidate(&mut self) {
        self.objs.clear();
        self.unavailable.clear();
        self.drag = None;
        self.draw = None;
        self.insp = None;
        self.words.clear();
        self.selection.clear_words();
        self.text_drag = false;
        self.links.clear();
        self.form.invalidate();
        self.annots.clear();
        self.annot_insp = None;
    }

    /// The annotations on `page`, read once until the document changes.
    pub fn annots(&mut self, pdf: &mpe_core::Pdf, page: usize) -> &[Annot] {
        self.annots
            .entry(page)
            .or_insert_with(|| pdf.page_annotations(page).unwrap_or_default())
    }

    /// The topmost shown annotation under `p` (page points): over one of its lines for
    /// markup, else inside its box.
    fn annot_at(&mut self, pdf: &mpe_core::Pdf, page: usize, p: Pos2) -> Option<&Annot> {
        let inside = |b: &[f32; 4]| p.x >= b[0] && p.x <= b[2] && p.y >= b[1] && p.y <= b[3];
        self.annots(pdf, page).iter().rev().find(|a| {
            !a.hidden()
                && if a.quads.is_empty() {
                    inside(&a.bbox)
                } else {
                    a.quads.iter().any(inside)
                }
        })
    }

    /// The selected annotation, once its page's annotations have been read.
    pub fn selected_annot(&self) -> Option<&Annot> {
        let (page, id) = self.selection.annot()?;
        self.annots.get(&page)?.iter().find(|a| a.id == id)
    }

    pub fn set_tool(&mut self, t: Tool) {
        self.tool = t;
        self.drag = None;
        self.draw = None;
        if t == Tool::Read {
            self.selection.clear_object();
            self.insp = None;
        } else {
            self.selection.clear_words();
            self.text_drag = false;
        }
        if t != Tool::Text {
            self.new_text = None;
        }
    }

    pub fn set_tool_keep_sel(&mut self, t: Tool) {
        let sel = self.selection.clone();
        self.set_tool(t);
        self.selection = sel;
    }

    /// Select object `id` on `page`; nothing, if the page has no such object.
    pub fn select_object(&mut self, page: usize, id: usize) {
        let obj = self.objs.get(&page).and_then(|v| v.get(id));
        self.selection = match obj.map(ObjKind::of) {
            Some(kind) => Selection::Object { page, id, kind },
            None => Selection::None,
        };
    }

    /// The selected object's page and index.
    pub fn sel(&self) -> Option<(usize, usize)> {
        self.selection.object()
    }

    pub fn objects(&mut self, pdf: &mpe_core::Pdf, page: usize) -> &[PageObject] {
        if !self.objs.contains_key(&page) {
            match pdf.page_objects(page) {
                Ok(v) => {
                    self.objs.insert(page, v);
                }
                Err(e) => {
                    self.unavailable.insert(page, e.to_string());
                    self.objs.insert(page, Vec::new());
                }
            }
        }
        &self.objs[&page]
    }

    fn words(&mut self, pdf: &mpe_core::Pdf, page: usize) -> &[PageWord] {
        self.words
            .entry(page)
            .or_insert_with(|| pdf.page_words(page).unwrap_or_default())
    }

    /// The selected words as text, if any are selected: pages in order, separated by a
    /// line break.
    pub fn selected_text(&self) -> Option<String> {
        let ((first, _), (last, _)) = self.selection.words()?;
        let parts: Vec<String> = (first..=last)
            .filter_map(|p| {
                let words = self.words.get(&p)?;
                let range = self.selection.words_on(p, words.len())?;
                Some(words_text(&words[range]))
            })
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n"))
    }

    /// For each page with selected words, the rectangle around them with a small margin
    /// (displayed-page points), for printing or saving the selection as cropped pages.
    pub fn selected_rects(&self) -> Vec<(usize, [f32; 4])> {
        const PAD: f32 = 4.0;
        let Some(((first, _), (last, _))) = self.selection.words() else {
            return Vec::new();
        };
        (first..=last)
            .filter_map(|p| {
                let words = self.words.get(&p)?;
                let range = self.selection.words_on(p, words.len())?;
                let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                for b in words[range].iter().map(|w| w.bbox) {
                    (r[0], r[1]) = (r[0].min(b[0]), r[1].min(b[1]));
                    (r[2], r[3]) = (r[2].max(b[2]), r[3].max(b[3]));
                }
                (r[0] < r[2]).then_some((p, [r[0] - PAD, r[1] - PAD, r[2] + PAD, r[3] + PAD]))
            })
            .collect()
    }

    /// For each page with selected words, the box around each selected line's words
    /// (displayed-page points), for marking them.
    pub fn selected_lines(&self) -> Vec<(usize, Vec<[f32; 4]>)> {
        let Some(((first, _), (last, _))) = self.selection.words() else {
            return Vec::new();
        };
        (first..=last)
            .filter_map(|p| {
                let words = self.words.get(&p)?;
                let range = self.selection.words_on(p, words.len())?;
                Some((p, Self::line_boxes(&words[range])))
            })
            .collect()
    }

    /// Move the moving end of the word selection to `end`, or start one there. The words
    /// of the pages in between are read, for copying.
    fn extend_words(&mut self, pdf: &mpe_core::Pdf, end: WordAt) {
        let anchor = match self.selection {
            Selection::Words { anchor, .. } => anchor,
            _ => end,
        };
        for p in anchor.0.min(end.0)..=anchor.0.max(end.0) {
            self.words(pdf, p);
        }
        self.selection = Selection::Words { anchor, end };
    }

    /// The first and last index of the words on word `i`'s line.
    fn line_of(words: &[PageWord], i: usize) -> Option<(usize, usize)> {
        let line = words.get(i)?.line;
        let first = words.iter().position(|w| w.line == line)?;
        let last = words.iter().rposition(|w| w.line == line)?;
        Some((first, last))
    }

    /// Each line's box: the union of its words' boxes, so gaps between words count.
    fn line_boxes(words: &[PageWord]) -> Vec<[f32; 4]> {
        // Words come in reading order, so a line's words are together.
        let mut boxes: Vec<(usize, [f32; 4])> = Vec::new();
        for w in words {
            match boxes.last_mut() {
                Some((l, b)) if *l == w.line => *b = union(*b, w.bbox),
                _ => boxes.push((w.line, w.bbox)),
            }
        }
        boxes.into_iter().map(|(_, b)| b).collect()
    }

    /// Index of the word nearest to `p` (page points), if the page has words.
    fn nearest_word(words: &[PageWord], p: Pos2) -> Option<usize> {
        let dist = |b: [f32; 4]| {
            let dx = (b[0] - p.x).max(p.x - b[2]).max(0.0);
            let dy = (b[1] - p.y).max(p.y - b[3]).max(0.0);
            // Rows matter more than columns when choosing among lines.
            dx + 3.0 * dy
        };
        (0..words.len()).min_by(|&i, &j| dist(words[i].bbox).total_cmp(&dist(words[j].bbox)))
    }

    pub fn selected_object(&self) -> Option<&PageObject> {
        let (p, id) = self.sel()?;
        self.objs.get(&p)?.get(id)
    }

    /// After an insertion or replacement the new object is the last one on the page.
    pub fn select_last(&mut self, pdf: &mpe_core::Pdf, page: usize) {
        let n = self.objects(pdf, page).len();
        match n.checked_sub(1) {
            Some(i) => self.select_object(page, i),
            None => self.selection.clear_object(),
        }
        self.insp = None;
    }

    fn hit(&self, page: usize, p: Pos2, tol: f32) -> Option<usize> {
        let objs = self.objs.get(&page)?;
        objs.iter()
            .enumerate()
            .filter(|(_, o)| {
                (self.show_hidden || !o.hidden)
                    && p.x >= o.bbox[0] - tol
                    && p.x <= o.bbox[2] + tol
                    && p.y >= o.bbox[1] - tol
                    && p.y <= o.bbox[3] + tol
            })
            .min_by(|(ia, a), (ib, b)| {
                let (aa, ab) = (
                    (a.bbox[2] - a.bbox[0]) * (a.bbox[3] - a.bbox[1]),
                    (b.bbox[2] - b.bbox[0]) * (b.bbox[3] - b.bbox[1]),
                );
                aa.partial_cmp(&ab)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(ib.cmp(ia))
            })
            .map(|(i, _)| i)
    }

    fn handle_hit(&self, b: [f32; 4], p: Pos2, tol: f32) -> Option<usize> {
        (0..8).find(|&h| {
            let (x, y) = handle_pos(b, h);
            (p.x - x).abs() <= tol && (p.y - y).abs() <= tol
        })
    }

    fn default_style(&self) -> PathStyle {
        PathStyle {
            fill: self.fill_on.then_some(self.fill),
            stroke: self.stroke_on.then_some(self.stroke),
            line_width: self.width,
        }
    }

    pub fn style_for_new(&self) -> PathStyle {
        self.default_style()
    }

    /// Overlays drawn whatever the tool: search matches (`current` is outlined) and, when
    /// switched on, outlines of recognised (invisible) text, with the text of the line under
    /// the pointer shown above it. `r` and `k` as for [`Editor::page_ui`].
    #[allow(clippy::too_many_arguments)]
    pub fn overlay(
        &mut self,
        ui: &egui::Ui,
        pdf: &mpe_core::Pdf,
        page: usize,
        r: Rect,
        k: f32,
        matches: &[Vec<[f32; 4]>],
        current: Option<usize>,
    ) {
        if !self.show_hidden && self.selected_object().is_some_and(|o| o.hidden) {
            self.selection.clear_object();
            self.insp = None;
        }
        let painter = ui.painter();
        let screen_rect = |b: [f32; 4]| {
            Rect::from_min_max(
                Pos2::new(r.min.x + b[0] * k, r.min.y + b[1] * k),
                Pos2::new(r.min.x + b[2] * k, r.min.y + b[3] * k),
            )
        };
        // A match broken across lines has a box on each.
        for (i, boxes) in matches.iter().enumerate() {
            for b in boxes {
                let sr = screen_rect(*b).expand(1.0);
                painter.rect_filled(sr, 1.0, Color32::from_rgba_unmultiplied(255, 210, 0, 90));
                if current == Some(i) {
                    let orange = Stroke::new(2.0, Color32::from_rgb(230, 120, 0));
                    painter.rect_stroke(sr, 1.0, orange, StrokeKind::Outside);
                }
            }
        }
        if !self.show_hidden {
            return;
        }
        let lines: Vec<(Rect, String)> = self
            .objects(pdf, page)
            .iter()
            .filter(|o| o.hidden)
            .map(|o| (screen_rect(o.bbox), o.text.clone().unwrap_or_default()))
            .collect();
        let blue = Color32::from_rgb(40, 110, 230);
        let hover = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|p| r.contains(*p));
        let mut tip = None;
        for (sr, text) in &lines {
            painter.rect_filled(*sr, 0.0, blue.gamma_multiply(0.08));
            let edge = Stroke::new(1.0, blue.gamma_multiply(0.8));
            painter.rect_stroke(*sr, 0.0, edge, StrokeKind::Outside);
            if hover.is_some_and(|p| sr.contains(p)) {
                tip = Some((*sr, text.clone()));
            }
        }
        if let Some((sr, text)) = tip {
            let font = egui::FontId::proportional(14.0);
            let galley = painter.layout_no_wrap(text, font, ui.visuals().strong_text_color());
            let pos = sr.left_top() - Vec2::new(0.0, galley.size().y + 6.0);
            let bg = Rect::from_min_size(pos, galley.size()).expand(3.0);
            painter.rect_filled(bg, 3.0, ui.visuals().extreme_bg_color);
            painter.rect_stroke(bg, 3.0, Stroke::new(1.0, blue), StrokeKind::Inside);
            painter.galley(pos, galley, Color32::PLACEHOLDER);
        }
    }

    /// Handle pointer interaction and draw overlays for one visible page. `r` is the page's screen rectangle
    /// and `k` the screen size of one PDF point. Returns the page's response, for its
    /// right-click menu.
    pub fn page_ui(
        &mut self,
        ui: &mut egui::Ui,
        pdf: &mpe_core::Pdf,
        tab_id: u64,
        page: usize,
        r: Rect,
        k: f32,
    ) -> egui::Response {
        let to_screen = |x: f32, y: f32| Pos2::new(r.min.x + x * k, r.min.y + y * k);
        let to_pt = |p: Pos2| Pos2::new((p.x - r.min.x) / k, (p.y - r.min.y) / k);
        let screen_rect =
            |b: [f32; 4]| Rect::from_min_max(to_screen(b[0], b[1]), to_screen(b[2], b[3]));
        let id = ui.id().with(("page-edit", tab_id, page));
        let mut resp = ui.interact(r, id, Sense::click_and_drag());
        if resp.secondary_clicked() {
            let p = resp.interact_pointer_pos().map(to_pt);
            self.select_for_menu(pdf, page, p, 3.0 / k);
        }
        // Links are followed with a click in the read tool, and with Ctrl+click in the
        // editing tools, so they don't get in the way of editing. A drag that starts on a
        // link still selects text.
        let selecting = self.tool == Tool::Read;
        let mut over_link = false;
        let ctrl = ui.input(|i| i.modifiers.command);
        // Form fields are filled in the same way, and sit above links.
        let filling = (selecting || ctrl) && !self.dragging();
        let over_field = self
            .form
            .page_ui(ui, pdf, page, r, k, &resp, filling, &mut self.actions);
        if over_field {
            if !selecting || resp.clicked() {
                return resp;
            }
            over_link = true;
        } else if filling {
            let link = resp.hover_pos().map(to_pt).and_then(|p| {
                let links = self
                    .links
                    .entry(page)
                    .or_insert_with(|| pdf.page_links(page).unwrap_or_default());
                links.iter().find(|l| {
                    let b = l.bbox;
                    (b[0]..=b[2]).contains(&p.x) && (b[1]..=b[3]).contains(&p.y)
                })
            });
            if let Some(link) = link {
                let target = link.target.clone();
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                let tip = match &target {
                    LinkTarget::Page(n) => format!("Go to page {}", n + 1),
                    LinkTarget::Uri(u) => u.clone(),
                };
                over_link = true;
                resp = resp.on_hover_text_at_pointer(tip);
                if resp.clicked() {
                    self.follow = Some(target);
                    return resp;
                }
                // Holding Ctrl over a link sets an editing tool aside.
                if !selecting {
                    return resp;
                }
            }
        }
        self.objects(pdf, page);
        let painter = ui.painter().clone();
        let accent = ui.visuals().selection.bg_fill;
        let accent_strong = ui.visuals().selection.stroke.color;
        let hover_pt = resp.hover_pos().map(to_pt);
        let tol = 3.0 / k;
        let htol = 6.0 / k;
        let shift = ui.input(|i| i.modifiers.shift);

        // current selection box on this page
        let sel_here = self.sel().filter(|s| s.0 == page);
        let sel_box = sel_here.and_then(|(_, id)| {
            if let Some(d) = &self.drag {
                if d.page == page && d.id == id {
                    return Some(d.cur);
                }
            }
            self.objs.get(&page)?.get(id).map(|o| o.bbox)
        });

        match self.tool {
            Tool::Select => {
                if let Some(p) = hover_pt {
                    let over_handle = sel_box.and_then(|b| self.handle_hit(b, p, htol));
                    if let Some(h) = over_handle {
                        ui.ctx().set_cursor_icon(handle_cursor(h));
                    } else if let Some(i) = self.hit(page, p, tol) {
                        ui.ctx().set_cursor_icon(CursorIcon::Grab);
                        if sel_here.map(|s| s.1) != Some(i) {
                            if let Some(o) = self.objs.get(&page).and_then(|v| v.get(i)) {
                                painter.rect_stroke(
                                    screen_rect(o.bbox),
                                    0.0,
                                    Stroke::new(1.0, accent_strong.gamma_multiply(0.7)),
                                    StrokeKind::Outside,
                                );
                            }
                        }
                    }
                }
                if resp.drag_started() {
                    let p = ui.input(|i| i.pointer.press_origin()).map(to_pt);
                    if let Some(p) = p {
                        let h = sel_box.and_then(|b| self.handle_hit(b, p, htol));
                        if let (Some(h), Some((_, id))) = (h, sel_here) {
                            self.start_drag(page, id, Mode::Handle(h), p);
                        } else if let Some(i) = self.hit(page, p, tol) {
                            self.select_object(page, i);
                            self.start_drag(page, i, Mode::Move, p);
                        } else {
                            self.selection.clear_object();
                        }
                    }
                }
                if resp.dragged() {
                    if let (Some(d), Some(pos)) = (self.drag.as_mut(), resp.interact_pointer_pos())
                    {
                        if d.page == page {
                            let p = to_pt(pos);
                            let (dx, dy) = (p.x - d.start.x, p.y - d.start.y);
                            d.cur = match d.mode {
                                Mode::Move => [
                                    d.orig[0] + dx,
                                    d.orig[1] + dy,
                                    d.orig[2] + dx,
                                    d.orig[3] + dy,
                                ],
                                Mode::Handle(h) => resize(d.orig, h, dx, dy, shift),
                            };
                            ui.ctx().set_cursor_icon(match d.mode {
                                Mode::Move => CursorIcon::Grabbing,
                                Mode::Handle(h) => handle_cursor(h),
                            });
                        }
                    }
                }
                if resp.drag_stopped() {
                    if let Some(d) = self.drag.take() {
                        let moved = d.cur.iter().zip(&d.orig).any(|(a, b)| (a - b).abs() > 0.5);
                        if d.page == page && moved {
                            self.actions.push(Action::SetBox(d.page, d.id, d.cur));
                        }
                    }
                }
                if resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos().map(to_pt) {
                        let h = sel_box.and_then(|b| self.handle_hit(b, p, htol));
                        if h.is_none() {
                            match self.hit(page, p, tol) {
                                Some(i) => self.select_object(page, i),
                                None => self.selection.clear_object(),
                            }
                        }
                    }
                }
                if resp.double_clicked() {
                    if let Some(o) = self.selected_object() {
                        if o.kind == ObjectKind::Text {
                            self.focus_text = true;
                        }
                    }
                }
            }
            Tool::Read => {
                let words = self.words(pdf, page).to_vec();
                let inside =
                    |p: Pos2, b: [f32; 4]| p.x >= b[0] && p.x <= b[2] && p.y >= b[1] && p.y <= b[3];
                let lines = Self::line_boxes(&words);
                if !over_link && hover_pt.is_some_and(|p| lines.iter().any(|&b| inside(p, b))) {
                    ui.ctx().set_cursor_icon(CursorIcon::Text);
                }
                let nearest = |p: Option<Pos2>| p.and_then(|p| Self::nearest_word(&words, p));
                // A drag selects the words between the press and the pointer, in reading
                // order, across pages: whichever visible page the pointer is over (held
                // inside the view, which scrolls at its edges) moves the end. A click
                // clears, Shift+click extends, a double-click selects a word and a
                // triple-click its line.
                if resp.drag_started() {
                    let p = ui.input(|i| i.pointer.press_origin()).map(to_pt);
                    if let Some(i) = nearest(p) {
                        self.selection = Selection::word((page, i));
                        self.text_drag = true;
                    }
                }
                if self.text_drag {
                    let view = ui.clip_rect();
                    let at = ui.input(|i| i.pointer.latest_pos()).map(|p| view.clamp(p));
                    if let Some(i) = nearest(at.filter(|p| r.contains(*p)).map(to_pt)) {
                        self.extend_words(pdf, (page, i));
                    }
                }
                if resp.drag_stopped() {
                    self.text_drag = false;
                }
                let click_at = || resp.interact_pointer_pos().map(to_pt);
                if resp.triple_clicked() {
                    let line = nearest(click_at()).and_then(|i| Self::line_of(&words, i));
                    if let Some((a, b)) = line {
                        self.selection = Selection::Words {
                            anchor: (page, a),
                            end: (page, b),
                        };
                    }
                } else if resp.double_clicked() {
                    if let Some(i) = nearest(click_at()) {
                        self.selection = Selection::word((page, i));
                    }
                } else if resp.clicked() {
                    // A click on a comment or markup selects it; a drag still selects
                    // the text under it.
                    let annot = click_at()
                        .filter(|_| !shift)
                        .and_then(|p| self.annot_at(pdf, page, p))
                        .map(|a| a.id);
                    match (annot, nearest(click_at())) {
                        (Some(id), _) => self.selection = Selection::Annot { page, id },
                        (None, Some(i)) if shift && self.selection.words().is_some() => {
                            self.extend_words(pdf, (page, i));
                        }
                        _ => {
                            self.selection.clear_words();
                            self.selection.clear_annot();
                        }
                    }
                }
                if !over_link && !self.text_drag {
                    let tip = hover_pt
                        .and_then(|p| self.annot_at(pdf, page, p))
                        .map(annot_tip);
                    if let Some(tip) = tip {
                        resp = resp.on_hover_text_at_pointer(tip);
                    }
                }
                if let Some(range) = self.selection.words_on(page, words.len()) {
                    let fill = accent.gamma_multiply(0.45);
                    for w in &words[range] {
                        painter.rect_filled(screen_rect(w.bbox), 1.0, fill);
                    }
                }
            }
            Tool::Text => {
                ui.ctx().set_cursor_icon(CursorIcon::Text);
                if resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos().map(to_pt) {
                        self.new_text = Some((page, (p.x, p.y)));
                        self.text_buf.clear();
                        self.focus_text = true;
                    }
                }
            }
            Tool::Image | Tool::Rect | Tool::Ellipse | Tool::Line => {
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                if resp.drag_started() {
                    if let Some(p) = ui.input(|i| i.pointer.press_origin()).map(to_pt) {
                        self.draw = Some(Draw {
                            page,
                            start: p,
                            cur: p,
                        });
                    }
                }
                if resp.dragged() {
                    if let (Some(d), Some(pos)) = (self.draw.as_mut(), resp.interact_pointer_pos())
                    {
                        if d.page == page {
                            d.cur = to_pt(pos);
                        }
                    }
                }
                let mut finish: Option<(Pos2, Pos2)> = None;
                if resp.drag_stopped() {
                    if let Some(d) = self.draw.take() {
                        if d.page == page {
                            finish = Some((d.start, d.cur));
                        }
                    }
                } else if resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos().map(to_pt) {
                        let (w, h) = match (self.tool, &self.pending_image) {
                            (Tool::Image, Some((_, aspect))) => (200.0, 200.0 / aspect.max(0.01)),
                            (Tool::Line, _) => (120.0, 0.0),
                            _ => (120.0, 80.0),
                        };
                        finish = Some((p, Pos2::new(p.x + w, p.y + h)));
                    }
                }
                if let Some((a, b)) = finish {
                    let rect = [a.x, a.y, b.x, b.y];
                    let n = norm(rect);
                    let big =
                        self.tool == Tool::Line || ((n[2] - n[0]) > 2.0 && (n[3] - n[1]) > 2.0);
                    if big {
                        match self.tool {
                            Tool::Image => self.actions.push(Action::AddImage(page, n)),
                            Tool::Rect => self.actions.push(Action::AddShape(page, Shape::Rect, n)),
                            Tool::Ellipse => {
                                self.actions.push(Action::AddShape(page, Shape::Ellipse, n))
                            }
                            Tool::Line => {
                                self.actions.push(Action::AddShape(page, Shape::Line, rect))
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(d) = &self.draw {
                    if d.page == page {
                        let (a, b) = (to_screen(d.start.x, d.start.y), to_screen(d.cur.x, d.cur.y));
                        let st = Stroke::new(1.5, accent_strong);
                        if self.tool == Tool::Line {
                            painter.line_segment([a, b], st);
                        } else {
                            let rr = Rect::from_two_pos(a, b);
                            painter.rect_filled(rr, 0.0, accent.gamma_multiply(0.3));
                            painter.rect_stroke(rr, 0.0, st, StrokeKind::Inside);
                        }
                    }
                }
            }
        }

        // Selection frame and handles.
        if self.tool == Tool::Select {
            if let Some(b) = sel_box {
                let sr = screen_rect(b);
                painter.rect_stroke(
                    sr,
                    0.0,
                    Stroke::new(1.5, accent_strong),
                    StrokeKind::Outside,
                );
                for h in 0..8 {
                    let (x, y) = handle_pos(b, h);
                    let c = to_screen(x, y);
                    let hr = Rect::from_center_size(c, Vec2::splat(7.0));
                    painter.rect_filled(hr, 1.0, Color32::WHITE);
                    painter.rect_stroke(
                        hr,
                        1.0,
                        Stroke::new(1.0, accent_strong),
                        StrokeKind::Inside,
                    );
                }
            }
        }
        // The selected annotation: each marked line, or its box.
        if self.selection.annot().is_some_and(|(p, _)| p == page) {
            self.annots(pdf, page);
            if let Some(a) = self.selected_annot() {
                let boxes = if a.quads.is_empty() {
                    std::slice::from_ref(&a.bbox)
                } else {
                    &a.quads[..]
                };
                for &b in boxes {
                    painter.rect_stroke(
                        screen_rect(b).expand(2.0),
                        2.0,
                        Stroke::new(2.0, accent_strong),
                        StrokeKind::Outside,
                    );
                }
            }
        }
        if let Some((p, (x, y))) = self.new_text {
            if p == page {
                let c = to_screen(x, y);
                painter.line_segment(
                    [c - Vec2::new(0.0, 14.0), c + Vec2::new(0.0, 4.0)],
                    Stroke::new(2.0, accent_strong),
                );
                painter.line_segment(
                    [c - Vec2::new(8.0, 0.0), c + Vec2::new(24.0, 0.0)],
                    Stroke::new(1.0, accent_strong),
                );
            }
        }
        resp
    }

    /// Before a right-click menu opens: select what is under the pointer `p` (page
    /// points), as most apps do. With the arrow, an object; with the read tool, a word
    /// (keeping a word selection the click falls inside); otherwise, or over nothing,
    /// the page (keeping a page selection that includes it).
    fn select_for_menu(&mut self, pdf: &mpe_core::Pdf, page: usize, p: Option<Pos2>, tol: f32) {
        match (self.tool, p) {
            (Tool::Select, Some(p)) => {
                self.objects(pdf, page);
                if let Some(i) = self.hit(page, p, tol) {
                    if self.sel() != Some((page, i)) {
                        self.select_object(page, i);
                    }
                    return;
                }
            }
            (Tool::Read, Some(p)) => {
                let words = self.words(pdf, page);
                let n = words.len();
                let under = words.iter().position(|w| {
                    let b = w.bbox;
                    p.x >= b[0] && p.x <= b[2] && p.y >= b[1] && p.y <= b[3]
                });
                // Selected words keep the menu for them, even over markup.
                let on = self.selection.words_on(page, n);
                if under.is_some_and(|i| on.is_some_and(|r| r.contains(&i))) {
                    return;
                }
                if let Some(id) = self.annot_at(pdf, page, p).map(|a| a.id) {
                    self.selection = Selection::Annot { page, id };
                    return;
                }
                if let Some(i) = under {
                    self.selection = Selection::word((page, i));
                    return;
                }
            }
            _ => {}
        }
        if !self.selection.pages().is_some_and(|s| s.contains(&page)) {
            self.selection = Selection::pages_from([page].into());
        }
    }

    fn start_drag(&mut self, page: usize, id: usize, mode: Mode, start: Pos2) {
        if let Some(o) = self.objs.get(&page).and_then(|v| v.get(id)) {
            self.drag = Some(Drag {
                page,
                id,
                mode,
                start,
                orig: o.bbox,
                cur: o.bbox,
            });
        }
    }

    /// Select every word on page `page` (read tool).
    pub fn select_all_words(&mut self, pdf: &mpe_core::Pdf, page: usize) {
        let n = self.words(pdf, page).len();
        match n.checked_sub(1) {
            Some(last) => {
                self.selection = Selection::Words {
                    anchor: (page, 0),
                    end: (page, last),
                }
            }
            None => self.selection.clear_words(),
        }
    }

    /// Whether the inspector shows the editor's section: a selected object or words, or
    /// a tool that adds content (its options). Otherwise it shows pages or the document.
    pub fn owns_panel(&self) -> bool {
        matches!(
            self.tool,
            Tool::Text | Tool::Image | Tool::Rect | Tool::Ellipse | Tool::Line
        ) || self.sel().is_some()
            || self.selection.words().is_some()
            || self.selection.annot().is_some()
    }

    /// The page and box (page points) of the selected object or words, for placing the
    /// floating toolbar. For words, the part on the page where the selection's moving end
    /// is.
    pub fn selection_box(&self) -> Option<(usize, [f32; 4])> {
        if let Some(o) = self.selected_object() {
            return Some((self.sel()?.0, o.bbox));
        }
        if let Some(a) = self.selected_annot() {
            return Some((a.page, a.bbox));
        }
        let Selection::Words { end: (page, _), .. } = self.selection else {
            return None;
        };
        let words = self.words.get(&page)?;
        let run = &words[self.selection.words_on(page, words.len())?];
        let bbox = run.iter().map(|w| w.bbox).reduce(union)?;
        Some((page, bbox))
    }

    /// Whether words are being selected by dragging. The drag ends when the button is
    /// released, even if the page it started on has scrolled out of view.
    pub fn selecting_words(&mut self, ctx: &egui::Context) -> bool {
        if self.text_drag && !ctx.input(|i| i.pointer.primary_down()) {
            self.text_drag = false;
        }
        self.text_drag
    }

    /// The pointer is moving, resizing, drawing or selecting something.
    pub fn dragging(&self) -> bool {
        self.drag.is_some() || self.draw.is_some() || self.text_drag
    }

    /// Put the keyboard in the inspector's text box for the selected text object.
    pub fn focus_text_field(&mut self) {
        self.focus_text = true;
    }

    /// Whether an object is selected and can be deleted or moved.
    pub fn has_object(&self) -> bool {
        self.sel().is_some() && self.tool != Tool::Read
    }

    /// Delete the selected object, or the selected annotation unless it is locked.
    pub fn delete_selected(&mut self) {
        if let Some((p, id)) = self.sel() {
            self.actions.push(Action::Delete(p, id));
        } else if let Some(a) = self.selected_annot().filter(|a| !a.locked()) {
            self.actions.push(Action::DeleteAnnot(a.page, a.id));
        }
    }

    /// Move the selected object by (dx, dy) points.
    pub fn nudge(&mut self, dx: f32, dy: f32) {
        let Some((p, id)) = self.sel() else { return };
        if let Some(o) = self.objs.get(&p).and_then(|v| v.get(id)) {
            let b = o.bbox;
            self.actions.push(Action::SetBox(
                p,
                id,
                [b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy],
            ));
        }
    }

    /// Esc: clear a selected annotation, or the word selection (read tool); otherwise drop
    /// unfinished text, leave a drawing tool for the arrow, or with the arrow clear the
    /// selection.
    pub fn escape(&mut self) {
        if self.selection.annot().is_some() {
            self.selection.clear_annot();
            return;
        }
        match self.tool {
            Tool::Read => self.selection.clear_words(),
            Tool::Select => {
                self.new_text = None;
                self.selection.clear_object();
            }
            _ => {
                self.new_text = None;
                self.tool = Tool::Select;
            }
        }
    }

    // ------------------------------------------------------------ inspector

    fn font_ui(ui: &mut egui::Ui, f: &mut FontChoice) {
        egui::ComboBox::from_id_salt(ui.id().with("fam"))
            .selected_text(match f.family {
                Family::Helvetica => "Helvetica",
                Family::Times => "Times",
                Family::Courier => "Courier",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut f.family, Family::Helvetica, "Helvetica");
                ui.selectable_value(&mut f.family, Family::Times, "Times");
                ui.selectable_value(&mut f.family, Family::Courier, "Courier");
            });
        ui.horizontal(|ui| {
            ui.toggle_value(&mut f.bold, egui::RichText::new("B").strong());
            ui.toggle_value(&mut f.italic, egui::RichText::new("I").italics());
        });
    }

    fn sync_insp(&mut self) {
        let Some(key) = self.sel() else {
            self.insp = None;
            return;
        };
        if self.insp.as_ref().map(|i| i.key) == Some(key) {
            return;
        }
        let Some(o) = self.objs.get(&key.0).and_then(|v| v.get(key.1)) else {
            self.insp = None;
            return;
        };
        let font = FontChoice::from_name(o.font_name.as_deref().unwrap_or(""));
        let size = o.font_size.unwrap_or(12.0);
        let color = if o.kind == ObjectKind::Text {
            o.fill.unwrap_or([0.0; 3])
        } else {
            [0.0; 3]
        };
        self.insp = Some(Insp {
            key,
            text: o.text.clone().unwrap_or_default(),
            font,
            size,
            color,
            fill_on: o.has_fill,
            fill: o.fill.unwrap_or([0.8, 0.8, 0.8]),
            stroke_on: o.has_stroke,
            stroke: o.stroke.unwrap_or([0.0; 3]),
            width: o.line_width.unwrap_or(1.0),
            geom: o.bbox,
            orig_font: font,
            orig_size: size,
            orig_color: color,
        });
    }

    /// The selected annotation: what and whose it is, and its colour, opacity and comment,
    /// changed with Apply as one undo step.
    fn annot_inspector(&mut self, ui: &mut egui::Ui) {
        let Some(a) = self.selected_annot().cloned() else {
            ui.weak("Nothing selected.");
            return;
        };
        let now = AnnotEdit {
            color: a.color,
            opacity: a.opacity,
            contents: a.contents.clone(),
        };
        if self.annot_insp.as_ref().is_none_or(|(id, _)| *id != a.id) {
            self.annot_insp = Some((a.id, now.clone()));
        }
        let Some((_, e)) = self.annot_insp.as_mut() else {
            return;
        };
        ui.strong(a.kind.label());
        let mut about = vec![format!("Page {}", a.page + 1)];
        about.extend(
            [&a.author, &a.modified]
                .into_iter()
                .filter(|s| !s.is_empty())
                .cloned(),
        );
        ui.weak(about.join(" · "));
        if a.locked() {
            ui.weak("Locked in this document: it can't be changed or deleted here.");
        }
        ui.separator();
        ui.add_enabled_ui(!a.locked(), |ui| {
            ui.horizontal(|ui| {
                let mut on = e.color.is_some();
                if ui.checkbox(&mut on, "Colour").changed() {
                    e.color = on.then_some(a.color.unwrap_or([0.0; 3]));
                }
                if let Some(c) = e.color.as_mut() {
                    ui.color_edit_button_rgb(c);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Opacity");
                let mut pct = e.opacity * 100.0;
                if ui
                    .add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix(" %"))
                    .changed()
                {
                    e.opacity = pct / 100.0;
                }
            });
            ui.label("Comment");
            ui.add(
                egui::TextEdit::multiline(&mut e.contents)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(*e != now, egui::Button::new("Apply changes"))
                    .clicked()
                {
                    self.actions.push(Action::SetAnnot(a.page, a.id, e.clone()));
                }
                if ui.button((theme::icon(icon::TRASH), "Delete")).clicked() {
                    self.actions.push(Action::DeleteAnnot(a.page, a.id));
                }
            });
        });
    }

    /// The right-hand inspector: tool options, new-text entry and properties of the selected object.
    pub fn inspector(&mut self, ui: &mut egui::Ui) {
        if self.selection.annot().is_some() {
            self.annot_inspector(ui);
            return;
        }
        self.sync_insp();
        let page_err = self.sel().and_then(|s| self.unavailable.get(&s.0)).cloned();
        if let Some(e) = page_err {
            ui.colored_label(
                Color32::LIGHT_RED,
                format!("Editing is unavailable on this page: {e}"),
            );
        }
        match self.tool {
            Tool::Text => {
                ui.strong("New text");
                Self::font_ui(ui, &mut self.tfont);
                ui.horizontal(|ui| {
                    ui.label("Size");
                    ui.add(
                        egui::DragValue::new(&mut self.tsize)
                            .range(4.0..=200.0)
                            .suffix(" pt"),
                    );
                    ui.color_edit_button_rgb(&mut self.tcolor);
                });
                if let Some((page, pos)) = self.new_text {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.text_buf)
                            .hint_text("Type, then Enter"),
                    );
                    if self.focus_text {
                        r.request_focus();
                        self.focus_text = false;
                    }
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        if (ui.button("Add").clicked() || enter) && !self.text_buf.trim().is_empty()
                        {
                            self.actions.push(Action::AddText(
                                page,
                                pos,
                                std::mem::take(&mut self.text_buf),
                            ));
                            self.new_text = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.new_text = None;
                        }
                    });
                } else {
                    ui.label("Click on the page where the text should start.");
                }
                ui.separator();
            }
            Tool::Image => {
                ui.strong("Insert image");
                ui.label(if self.pending_image.is_some() {
                    "Drag a box on the page, or click to place it at a default width."
                } else {
                    "No image chosen."
                });
                ui.separator();
            }
            Tool::Rect | Tool::Ellipse | Tool::Line => {
                ui.strong(match self.tool {
                    Tool::Rect => "New rectangle",
                    Tool::Ellipse => "New ellipse",
                    _ => "New line",
                });
                if self.tool != Tool::Line {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.fill_on, "Fill");
                        ui.color_edit_button_rgb(&mut self.fill);
                    });
                }
                ui.horizontal(|ui| {
                    if self.tool == Tool::Line {
                        self.stroke_on = true;
                    } else {
                        ui.checkbox(&mut self.stroke_on, "Outline");
                    }
                    ui.color_edit_button_rgb(&mut self.stroke);
                    ui.add(
                        egui::DragValue::new(&mut self.width)
                            .range(0.25..=40.0)
                            .speed(0.1)
                            .suffix(" pt"),
                    );
                });
                ui.label("Drag on the page, or click for a default size.");
                ui.separator();
            }
            Tool::Select => {}
            Tool::Read => {
                ui.strong("Selected text");
                ui.label(
                    "Drag over words to select them, across pages if you like, double-click a \
                     word, triple-click a line, Shift+click to extend, or press Ctrl+A for the \
                     whole page. Ctrl+C copies; right-click to print or save the selection.",
                );
                ui.separator();
                match self.selected_text() {
                    Some(text) => {
                        let mut shown = text.clone();
                        ui.add(
                            egui::TextEdit::multiline(&mut shown)
                                .desired_rows(6)
                                .desired_width(f32::INFINITY)
                                .interactive(false),
                        );
                        if ui.button("Copy").clicked() {
                            ui.ctx().copy_text(text);
                        }
                    }
                    None => {
                        ui.weak("Nothing selected.");
                    }
                }
                return;
            }
        }

        let Some(insp) = self.insp.as_mut() else {
            if self.tool == Tool::Select {
                ui.label("Click an object on the page to select it. Drag to move, drag the handles to resize (Shift keeps proportions). Arrow keys nudge, Delete removes.");
            }
            return;
        };
        let (page, id) = insp.key;
        let Some(obj) = self.objs.get(&page).and_then(|v| v.get(id)).cloned() else {
            return;
        };
        ui.strong(match obj.kind {
            ObjectKind::Text if obj.hidden => "Recognised text (invisible)",
            ObjectKind::Text => "Text",
            ObjectKind::Image => "Image",
            ObjectKind::Form => "Group (form XObject)",
            ObjectKind::Path => "Shape",
        });

        // Geometry
        let mut geom_done = false;
        let (mut x, mut y, mut w, mut h) = (
            insp.geom[0],
            insp.geom[1],
            insp.geom[2] - insp.geom[0],
            insp.geom[3] - insp.geom[1],
        );
        egui::Grid::new("geom").num_columns(4).show(ui, |ui| {
            for (label, v, lo) in [("X", &mut x, -5000.0), ("Y", &mut y, -5000.0)] {
                ui.label(label);
                let r = ui.add(egui::DragValue::new(v).range(lo..=20000.0).speed(0.5));
                geom_done |= r.drag_stopped() || r.lost_focus();
            }
            ui.end_row();
            for (label, v) in [("W", &mut w), ("H", &mut h)] {
                ui.label(label);
                let r = ui.add(egui::DragValue::new(v).range(1.0..=20000.0).speed(0.5));
                geom_done |= r.drag_stopped() || r.lost_focus();
            }
            ui.end_row();
        });
        insp.geom = [x, y, x + w, y + h];
        if geom_done
            && insp
                .geom
                .iter()
                .zip(&obj.bbox)
                .any(|(a, b)| (a - b).abs() > 0.05)
        {
            self.actions.push(Action::SetBox(page, id, insp.geom));
        }

        match obj.kind {
            ObjectKind::Text => {
                ui.add_space(4.0);
                let r = ui.add(
                    egui::TextEdit::multiline(&mut insp.text)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                if self.focus_text {
                    r.request_focus();
                    self.focus_text = false;
                }
                if obj.hidden {
                    ui.weak(
                        "Text found by OCR. It is not drawn, but it is what search, select \
                         and copy use. Correct misread words here; the line stays invisible \
                         and in place. Clear it to remove the line.",
                    );
                } else {
                    Self::font_ui(ui, &mut insp.font);
                    ui.horizontal(|ui| {
                        ui.label("Size");
                        ui.add(
                            egui::DragValue::new(&mut insp.size)
                                .range(1.0..=400.0)
                                .suffix(" pt"),
                        );
                        ui.color_edit_button_rgb(&mut insp.color);
                    });
                }
                if !obj.font_reusable && !obj.hidden {
                    ui.weak(format!(
                        "Font \"{}\" is embedded or non-standard. Edited text is set in the nearest standard font.",
                        obj.font_name.as_deref().unwrap_or("?")
                    ));
                }
                if insp.text.contains('\n') {
                    ui.weak("Only one line per text object; line breaks are removed.");
                }
                if ui.button("Apply text changes").clicked() {
                    let text = insp.text.replace(['\n', '\r'], " ");
                    let unchanged = Some(text.as_str()) == obj.text.as_deref()
                        && insp.font == insp.orig_font
                        && (insp.size - insp.orig_size).abs() <= 0.01
                        && insp.color == insp.orig_color;
                    if !unchanged {
                        self.actions.push(Action::EditText(
                            page,
                            id,
                            TextEdit {
                                text,
                                font: (insp.font != insp.orig_font).then_some(insp.font),
                                size: ((insp.size - insp.orig_size).abs() > 0.01)
                                    .then_some(insp.size),
                                color: (insp.color != insp.orig_color).then_some(insp.color),
                            },
                        ));
                    }
                }
            }
            ObjectKind::Path => {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut insp.fill_on, "Fill");
                    ui.color_edit_button_rgb(&mut insp.fill);
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut insp.stroke_on, "Outline");
                    ui.color_edit_button_rgb(&mut insp.stroke);
                    ui.add(
                        egui::DragValue::new(&mut insp.width)
                            .range(0.25..=100.0)
                            .speed(0.1)
                            .suffix(" pt"),
                    );
                });
                if ui.button("Apply style").clicked() {
                    self.actions.push(Action::PathStyle(
                        page,
                        id,
                        PathStyle {
                            fill: insp.fill_on.then_some(insp.fill),
                            stroke: insp.stroke_on.then_some(insp.stroke),
                            line_width: insp.width,
                        },
                    ));
                }
            }
            ObjectKind::Image => {
                ui.add_space(4.0);
                if obj.inline {
                    ui.weak("Inline image: it can be moved, resized or deleted, but not replaced.");
                } else if ui
                    .button((theme::icon(icon::IMAGE), "Replace image…"))
                    .clicked()
                {
                    if let Some(bytes) = pick_image() {
                        self.actions.push(Action::ReplaceImage(page, id, bytes));
                    }
                }
            }
            ObjectKind::Form => {
                ui.weak(
                    "A reusable group of content. It can be moved, resized or deleted as a whole.",
                );
            }
        }
        ui.add_space(6.0);
        if ui.button((theme::icon(icon::TRASH), "Delete")).clicked() {
            self.actions.push(Action::Delete(page, id));
        }
    }
}

/// Ask for an image file and return its bytes if it can be decoded.
pub fn pick_image() -> Option<Vec<u8>> {
    let path = rfd::FileDialog::new()
        .add_filter("Images", &["png", "jpg", "jpeg"])
        .pick_file()?;
    std::fs::read(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(x: f32, y: f32, line: usize) -> PageWord {
        PageWord {
            text: "w".into(),
            bbox: [x, y, x + 20.0, y + 10.0],
            line,
        }
    }

    #[test]
    fn lines_join_their_words() {
        let words = [
            word(0.0, 0.0, 0),
            word(30.0, 0.0, 0),
            word(0.0, 20.0, 1),
            word(30.0, 20.0, 1),
            word(60.0, 20.0, 1),
        ];
        // The gap between words (20..30) is inside the line's box.
        assert_eq!(
            Editor::line_boxes(&words),
            [[0.0, 0.0, 50.0, 10.0], [0.0, 20.0, 80.0, 30.0]]
        );
        assert_eq!(Editor::line_of(&words, 0), Some((0, 1)));
        assert_eq!(Editor::line_of(&words, 3), Some((2, 4)));
        assert_eq!(Editor::line_of(&words, 9), None);
        assert!(Editor::line_boxes(&[]).is_empty());
    }

    #[test]
    fn a_press_in_a_margin_finds_the_nearest_word() {
        let words = [word(100.0, 100.0, 0), word(100.0, 200.0, 1)];
        // Left of the second line, and below everything.
        assert_eq!(
            Editor::nearest_word(&words, Pos2::new(10.0, 205.0)),
            Some(1)
        );
        assert_eq!(
            Editor::nearest_word(&words, Pos2::new(110.0, 900.0)),
            Some(1)
        );
        assert_eq!(Editor::nearest_word(&[], Pos2::ZERO), None);
    }
}
