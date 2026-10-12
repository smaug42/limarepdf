//! Content editing state and interaction: selecting, moving and resizing objects, inserting text, images
//! and shapes, and the inspector panel.

use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use egui_phosphor::regular as icon;
use mpe_core::{Family, FontChoice, ObjectKind, PageObject, PathStyle, Shape, TextEdit};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    /// Reading only: page clicks do nothing.
    View,
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
    pub sel: Option<(usize, usize)>,
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
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            tool: Tool::View,
            sel: None,
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
        }
    }
}

fn norm(r: [f32; 4]) -> [f32; 4] {
    [
        r[0].min(r[2]),
        r[1].min(r[3]),
        r[0].max(r[2]),
        r[1].max(r[3]),
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
    if matches!(h, 2 | 3 | 4) {
        b[2] += dx;
    }
    if matches!(h, 0 | 1 | 2) {
        b[1] += dy;
    }
    if matches!(h, 4 | 5 | 6) {
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
    }

    pub fn set_tool(&mut self, t: Tool) {
        self.tool = t;
        self.drag = None;
        self.draw = None;
        if t != Tool::Text {
            self.new_text = None;
        }
        if t == Tool::View {
            self.sel = None;
        }
    }

    pub fn set_tool_keep_sel(&mut self, t: Tool) {
        let sel = self.sel;
        self.set_tool(t);
        self.sel = sel;
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

    pub fn selected_object(&self) -> Option<&PageObject> {
        let (p, id) = self.sel?;
        self.objs.get(&p)?.get(id)
    }

    /// After an insertion or replacement the new object is the last one on the page.
    pub fn select_last(&mut self, pdf: &mpe_core::Pdf, page: usize) {
        let n = self.objects(pdf, page).len();
        self.sel = n.checked_sub(1).map(|i| (page, i));
        self.insp = None;
    }

    fn hit(&self, page: usize, p: Pos2, tol: f32) -> Option<usize> {
        let objs = self.objs.get(&page)?;
        objs.iter()
            .enumerate()
            .filter(|(_, o)| {
                p.x >= o.bbox[0] - tol
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

    /// Handle pointer interaction and draw overlays for one visible page. `r` is the page's screen rectangle
    /// and `k` the screen size of one PDF point.
    pub fn page_ui(
        &mut self,
        ui: &mut egui::Ui,
        pdf: &mpe_core::Pdf,
        tab_id: u64,
        page: usize,
        r: Rect,
        k: f32,
    ) {
        if self.tool == Tool::View {
            return;
        }
        self.objects(pdf, page);
        let painter = ui.painter().clone();
        let to_screen = |x: f32, y: f32| Pos2::new(r.min.x + x * k, r.min.y + y * k);
        let to_pt = |p: Pos2| Pos2::new((p.x - r.min.x) / k, (p.y - r.min.y) / k);
        let screen_rect =
            |b: [f32; 4]| Rect::from_min_max(to_screen(b[0], b[1]), to_screen(b[2], b[3]));
        let resp = ui.interact(
            r,
            ui.id().with(("page-edit", tab_id, page)),
            Sense::click_and_drag(),
        );
        let accent = ui.visuals().selection.bg_fill;
        let accent_strong = ui.visuals().selection.stroke.color;
        let hover_pt = resp.hover_pos().map(to_pt);
        let tol = 3.0 / k;
        let htol = 6.0 / k;
        let shift = ui.input(|i| i.modifiers.shift);

        // current selection box on this page
        let sel_here = self.sel.filter(|s| s.0 == page);
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
                            self.sel = Some((page, i));
                            self.start_drag(page, i, Mode::Move, p);
                        } else {
                            self.sel = None;
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
                            self.sel = self.hit(page, p, tol).map(|i| (page, i));
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
            Tool::View => {}
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

    /// Keyboard shortcuts for the selection (only called when no text field has focus).
    pub fn keys(&mut self, ctx: &egui::Context) {
        if self.tool == Tool::View {
            return;
        }
        let (del, esc, left, right, up, down, shift) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.modifiers.shift,
            )
        });
        if esc {
            self.new_text = None;
            if self.tool != Tool::Select {
                self.tool = Tool::Select;
            } else {
                self.sel = None;
            }
        }
        let Some((p, id)) = self.sel else { return };
        if del {
            self.actions.push(Action::Delete(p, id));
        }
        let step = if shift { 10.0 } else { 1.0 };
        let (dx, dy) = (
            (right as i32 - left as i32) as f32 * step,
            (down as i32 - up as i32) as f32 * step,
        );
        if dx != 0.0 || dy != 0.0 {
            if let Some(o) = self.objs.get(&p).and_then(|v| v.get(id)) {
                let b = o.bbox;
                self.actions.push(Action::SetBox(
                    p,
                    id,
                    [b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy],
                ));
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
        let Some(key) = self.sel else {
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

    /// The right-hand inspector: tool options, new-text entry and properties of the selected object.
    pub fn inspector(&mut self, ui: &mut egui::Ui) {
        self.sync_insp();
        ui.heading("Edit");
        ui.separator();
        let page_err = self.sel.and_then(|s| self.unavailable.get(&s.0)).cloned();
        if let Some(e) = page_err {
            ui.colored_label(
                Color32::LIGHT_RED,
                format!("Editing is unavailable on this page: {e}"),
            );
        }
        match self.tool {
            Tool::View => {
                ui.label("Pick a tool in the toolbar to edit page content.");
                return;
            }
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
                if !obj.font_reusable {
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
                    .button(format!("{}  Replace image…", icon::IMAGE))
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
        if ui.button(format!("{}  Delete", icon::TRASH)).clicked() {
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
