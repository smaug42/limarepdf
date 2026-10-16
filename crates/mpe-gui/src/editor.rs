//! Content editing state and interaction: selecting, moving and resizing objects, inserting text, images
//! and shapes, and the inspector panel.

use crate::fields::FormUi;
use crate::selection::{ObjKind, Selection, WordAt};
use crate::theme;
use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use egui_phosphor::regular as icon;
use mpe_core::{
    words_text, Annot, AnnotKind, Family, FieldValue, FontChoice, Link, LinkTarget, ObjectKind,
    PageObject, PageWord, PathStyle, Shape, ShapeLook, StampColor, StampKind, StampPlace, TextEdit,
    TextStyle,
};
use std::collections::HashMap;
use std::path::PathBuf;

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
    /// Click to put a sticky note on the page, then type its comment.
    Note,
    /// Drag a box, then type text straight onto the page.
    TextBox,
    /// Draw freehand; strokes made in quick succession make one drawing.
    Pen,
    /// Click to put down a stamp, or drag a box to size it.
    Stamp,
}

impl Tool {
    /// The tools behind the toolbar's Annotate button.
    pub const ANNOTATE: [Tool; 7] = [
        Tool::Note,
        Tool::TextBox,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Line,
        Tool::Pen,
        Tool::Stamp,
    ];
}

/// The stamp the stamp tool puts down.
#[derive(Clone, PartialEq, Debug)]
pub enum StampPick {
    Text(StampKind),
    /// Your own wording, already in capitals.
    Custom(String, StampColor),
    Picture(PathBuf),
}

/// A pen stroke strictly sooner than this (seconds) after the last one joins its drawing.
const INK_PAUSE: f64 = 1.5;

/// A pen stroke being drawn: its points in page points, and the drawing it joins.
struct PenStroke {
    page: usize,
    points: Vec<Pos2>,
    join: Option<(u32, u16)>,
}

/// A note or text box being typed: a new one at `rect` (displayed-page points), or the
/// existing one `id`.
pub struct Writing {
    pub page: usize,
    pub id: Option<(u32, u16)>,
    pub note: bool,
    pub rect: [f32; 4],
    pub text: String,
    /// The text it had, for an existing one.
    original: String,
    /// A text box's text size in points.
    size: f32,
    /// Takes the keyboard on the next frame.
    fresh: bool,
}

impl Writing {
    /// A new note (with its icon's box) or text box, empty.
    fn new(page: usize, note: bool, rect: [f32; 4], size: f32) -> Self {
        Self {
            page,
            id: None,
            note,
            rect,
            text: String::new(),
            original: String::new(),
            size,
            fresh: true,
        }
    }
}

/// A note's icon on screen, in pixels, whatever the zoom, as other readers show notes.
const NOTE_PX: f32 = 24.0;

/// The box an annotation takes up on screen, in page points at `zoom` pixels per point:
/// a note's icon grows from its top left corner to [`NOTE_PX`].
fn shown_box(note: bool, b: [f32; 4], zoom: f32) -> [f32; 4] {
    if !note {
        return b;
    }
    let s = NOTE_PX / zoom.max(0.01);
    [b[0], b[1], b[2].max(b[0] + s), b[3].max(b[1] + s)]
}

/// Whether a double-click opens it for typing: a note or a text box that isn't locked.
fn writable(a: &Annot) -> bool {
    !a.locked() && matches!(a.kind, AnnotKind::Note | AnnotKind::TextBox)
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
    /// Replace an annotation's comment, or a text box's text.
    AnnotText(usize, (u32, u16), String),
    /// Give an annotation a new box (displayed-page points).
    MoveAnnot(usize, (u32, u16), [f32; 4]),
    /// Move a line's ends (x1, y1, x2, y2 in displayed-page points).
    LineEnds(usize, (u32, u16), [f32; 4]),
    /// Turn a drawing by this many degrees, clockwise.
    RotateInk(usize, (u32, u16), f32),
    DeleteAnnot(usize, (u32, u16)),
    /// A new note centred on a point, with its comment.
    AddNote(usize, (f32, f32), String),
    /// A new text box and its text.
    AddTextBox(usize, [f32; 4], String),
    /// A rectangle, ellipse or line as an annotation, styled as the tool is.
    AddShapeAnnot(usize, Shape, [f32; 4]),
    /// A new drawing with one pen stroke (page points).
    AddInk(usize, Vec<(f32, f32)>),
    /// Another stroke for a drawing.
    AddInkStroke(usize, (u32, u16), Vec<(f32, f32)>),
    /// The stamp tool's stamp.
    AddStamp(usize, StampPlace),
    /// Answer an annotation.
    Reply(usize, (u32, u16), String),
}

/// An annotation's properties as the inspector edits them.
#[derive(Clone, PartialEq)]
pub struct AnnotEdit {
    pub color: Option<[f32; 3]>,
    pub opacity: f32,
    pub contents: String,
    /// A text box's text size, colour, fill and border.
    pub text: Option<TextStyle>,
    /// A shape's, line's or drawing's fill, width and arrowhead.
    pub shape: Option<ShapeLook>,
}

/// Whether the annotation can be moved with the arrow tool: not locked, and not text
/// markup, which stays on its text.
pub fn movable(a: &Annot) -> bool {
    !a.locked() && a.quads.is_empty()
}

/// Whether it can also be resized by its box: a note's icon keeps its size, and a line
/// has handles on its ends instead.
pub fn resizable(a: &Annot) -> bool {
    movable(a) && !matches!(a.kind, AnnotKind::Note | AnnotKind::Line)
}

#[derive(Clone, Copy)]
enum Mode {
    Move,
    Handle(usize),
    /// One end of a line (0 is its start); `Drag::orig` and `cur` hold both ends.
    End(usize),
    /// Turning a drawing about the middle of its box (`orig`); `cur[0]` holds the angle
    /// in degrees, clockwise.
    Rotate,
}

/// How far above a drawing's box its rotate handle sits, in pixels.
const ROTATE_GAP: f32 = 22.0;

/// A circular arrow in place of the pointer, over the rotate handle and while turning:
/// the system cursors have no rotate shape. Dark on a light disc, so it shows on any page.
fn rotate_cursor(ctx: &egui::Context) {
    let Some(at) = ctx.pointer_latest_pos() else {
        return;
    };
    ctx.set_cursor_icon(CursorIcon::None);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("rotate-cursor"),
    ));
    painter.circle(
        at,
        10.0,
        Color32::WHITE.gamma_multiply(0.9),
        Stroke::new(1.0, Color32::from_gray(60)),
    );
    painter.text(
        at,
        egui::Align2::CENTER_CENTER,
        icon::ARROW_CLOCKWISE,
        egui::FontId::new(15.0, theme::icons()),
        Color32::from_gray(20),
    );
}

struct Drag {
    page: usize,
    id: usize,
    /// Moving an annotation, not object `id`.
    annot: Option<(u32, u16)>,
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
    /// The Annotate button's tool: the one last used.
    pub annot_tool: Tool,
    /// A note or text box being typed.
    pub writing: Option<Writing>,
    pub note_color: [f32; 3],
    /// How new text boxes are set.
    pub box_style: TextStyle,
    /// The name new annotations are signed with (from the settings).
    pub author: String,
    /// Pixels per page point, as the pages are drawn now.
    zoom: f32,
    /// The shape tools draw into the page's content instead of adding annotations.
    pub draw_permanent: bool,
    /// New lines get an arrowhead.
    pub arrow: bool,
    pub pen_color: [f32; 3],
    pub pen_width: f32,
    pen_stroke: Option<PenStroke>,
    /// The drawing the pen last added to, and its page.
    pub ink: Option<(usize, (u32, u16))>,
    /// When (`egui` time) the pen's last stroke ended.
    ink_at: f64,
    /// A reply being typed in the inspector.
    reply: String,
    pub stamp: StampPick,
    /// Text stamps get a line with the author's name and the date (from the settings).
    pub stamp_name_date: bool,
    /// Pictures used as stamps, the latest first (from the settings).
    pub stamp_pictures: Vec<PathBuf>,
    /// Stamp wordings of your own, the latest first (from the settings).
    pub stamp_wordings: Vec<(String, StampColor)>,
    /// A stamp wording being typed in the inspector, and its colour.
    stamp_draft: String,
    stamp_draft_color: StampColor,
    /// Put the keyboard in the reply box (and show the inspector).
    pub reply_focus: bool,
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
            fill_on: false,
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
            annot_tool: Tool::Note,
            writing: None,
            note_color: [1.0, 0.85, 0.2],
            box_style: TextStyle {
                border: 1.0,
                ..TextStyle::default()
            },
            author: String::new(),
            zoom: 1.0,
            draw_permanent: false,
            arrow: false,
            pen_color: [0.1, 0.25, 0.8],
            pen_width: 2.0,
            pen_stroke: None,
            ink: None,
            ink_at: f64::NEG_INFINITY,
            reply: String::new(),
            stamp: StampPick::Text(StampKind::Approved),
            stamp_name_date: true,
            stamp_pictures: Vec::new(),
            stamp_wordings: Vec::new(),
            stamp_draft: String::new(),
            stamp_draft_color: StampColor::Red,
            reply_focus: false,
        }
    }
}

/// Distance from `p` to the segment from `a` to `b`.
fn seg_dist(p: Pos2, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (a, b) = (Pos2::new(a.0, a.1), Pos2::new(b.0, b.1));
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Where a shape drag ends with Shift held: squares and circles, and lines at a multiple
/// of 45°.
fn constrain(tool: Tool, a: Pos2, b: Pos2) -> Pos2 {
    let d = b - a;
    match tool {
        Tool::Line => {
            let step = std::f32::consts::FRAC_PI_4;
            let angle = (d.y.atan2(d.x) / step).round() * step;
            a + Vec2::angled(angle) * d.length()
        }
        Tool::Rect | Tool::Ellipse => {
            let m = d.x.abs().max(d.y.abs());
            a + Vec2::new(m * d.x.signum(), m * d.y.signum())
        }
        _ => b,
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
        self.ink = None;
    }

    /// The annotations on `page`, read once until the document changes.
    pub fn annots(&mut self, pdf: &mpe_core::Pdf, page: usize) -> &[Annot] {
        self.annots
            .entry(page)
            .or_insert_with(|| pdf.page_annotations(page).unwrap_or_default())
    }

    /// The topmost shown annotation under `p` (page points): over one of its lines for
    /// markup, near its strokes for lines and drawings, else inside its box.
    fn annot_at(&mut self, pdf: &mpe_core::Pdf, page: usize, p: Pos2) -> Option<&Annot> {
        let inside = |b: &[f32; 4]| p.x >= b[0] && p.x <= b[2] && p.y >= b[1] && p.y <= b[3];
        let zoom = self.zoom;
        self.annots(pdf, page).iter().rev().find(|a| {
            let reach = 4.0 / zoom + a.shape.map_or(0.0, |s| s.width / 2.0);
            let near = |s: &Vec<(f32, f32)>| match s[..] {
                [q] => p.distance(Pos2::new(q.0, q.1)) <= reach,
                _ => s.windows(2).any(|w| seg_dist(p, w[0], w[1]) <= reach),
            };
            a.on_page()
                && if !a.paths.is_empty() {
                    a.paths.iter().filter(|s| !s.is_empty()).any(near)
                } else if a.quads.is_empty() {
                    inside(&shown_box(a.kind == AnnotKind::Note, a.bbox, zoom))
                } else {
                    a.quads.iter().any(inside)
                }
        })
    }

    /// Start typing a note or text box, finishing one already open first.
    fn start_writing(&mut self, w: Writing) {
        self.finish_writing();
        self.selection.clear_annot();
        self.writing = Some(w);
    }

    /// Open the selected note or text box for typing.
    pub fn write_selected(&mut self) {
        let Some(a) = self.selected_annot().filter(|a| writable(a)).cloned() else {
            return;
        };
        self.finish_writing();
        self.writing = Some(Writing {
            id: Some(a.id),
            text: a.contents.clone(),
            original: a.contents,
            ..Writing::new(
                a.page,
                a.kind == AnnotKind::Note,
                a.bbox,
                a.text_style.map_or(12.0, |s| s.size),
            )
        });
    }

    /// Keep what was typed: a new note or text box with some text, or changed text.
    pub fn finish_writing(&mut self) {
        let Some(w) = self.writing.take() else { return };
        match w.id {
            Some(id) if w.text != w.original => {
                self.actions.push(Action::AnnotText(w.page, id, w.text));
            }
            None if !w.text.trim().is_empty() => {
                let r = w.rect;
                self.actions.push(if w.note {
                    Action::AddNote(w.page, ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0), w.text)
                } else {
                    Action::AddTextBox(w.page, r, w.text)
                });
            }
            _ => {}
        }
    }

    /// The note or text box being typed on `page` (drawn at `r`, `k` pixels per point):
    /// a comment box beside a note's icon, or the text box's own area. Returns true when
    /// the page should not take the pointer this frame: it is over the text box, or the
    /// typing just ended.
    fn writing_ui(&mut self, ui: &mut egui::Ui, page: usize, r: Rect, k: f32) -> bool {
        let Some(w) = self.writing.as_mut().filter(|w| w.page == page) else {
            return false;
        };
        let to_screen = |x: f32, y: f32| Pos2::new(r.min.x + x * k, r.min.y + y * k);
        let b = shown_box(w.note, w.rect, k);
        let area = Rect::from_min_max(to_screen(b[0], b[1]), to_screen(b[2], b[3]));
        // Light colours in either theme, as for form fields: the text sits on the paper.
        let theme = ui.visuals().clone();
        let (out, done, cancel) = if w.note {
            let [cr, cg, cb] = self.note_color.map(|v| (v * 255.0).round() as u8);
            if w.id.is_none() {
                ui.painter()
                    .rect_filled(area, 2.0, Color32::from_rgb(cr, cg, cb));
            }
            let (mut done, mut cancel) = (false, false);
            let inner = egui::Area::new(ui.id().with(("note-writing", page)))
                .fixed_pos(area.right_top() + Vec2::new(6.0, 0.0))
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style())
                        .show(ui, |ui| {
                            ui.set_width(240.0);
                            let out = ui.add(
                                egui::TextEdit::multiline(&mut w.text)
                                    .id(egui::Id::new(("note-text", page)))
                                    .hint_text("Comment")
                                    .desired_rows(4)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.horizontal(|ui| {
                                done = ui.button("Done").clicked();
                                cancel = ui.button("Cancel").clicked();
                            });
                            out
                        })
                        .inner
                });
            (inner.inner, done, cancel)
        } else {
            let te = egui::TextEdit::multiline(&mut w.text)
                .id(egui::Id::new(("box-text", page)))
                .font(egui::FontId::proportional((w.size * k).clamp(6.0, 96.0)))
                .desired_rows(1)
                .min_size(area.size())
                .desired_width(area.width())
                .margin(egui::Margin::same(2));
            *ui.visuals_mut() = egui::Visuals::light();
            let out = ui.put(area, te);
            *ui.visuals_mut() = theme;
            (out, false, false)
        };
        if w.fresh {
            out.request_focus();
            w.fresh = false;
            return false;
        }
        // Ctrl+Enter finishes; Esc drops what was typed. The focus is asked first: asking
        // inside `input_mut` would wait on the lock it holds.
        let focused = out.has_focus();
        let (finish, esc) = ui.input_mut(|i| {
            (
                focused && i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if cancel || (esc && (out.lost_focus() || out.has_focus())) {
            self.writing = None;
            return true;
        }
        // A note stays open until Done; a text box ends when it loses the keyboard.
        if done || finish || (!w.note && out.lost_focus()) {
            self.finish_writing();
            return true;
        }
        !w.note && out.hovered()
    }

    /// The selected annotation, once its page's annotations have been read.
    pub fn selected_annot(&self) -> Option<&Annot> {
        let (page, id) = self.selection.annot()?;
        self.annots.get(&page)?.iter().find(|a| a.id == id)
    }

    pub fn set_tool(&mut self, t: Tool) {
        self.finish_writing();
        if Tool::ANNOTATE.contains(&t) {
            self.annot_tool = t;
        }
        self.tool = t;
        self.drag = None;
        self.draw = None;
        self.pen_stroke = None;
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
        let click_pt = |resp: &egui::Response| resp.interact_pointer_pos().map(to_pt);
        self.zoom = k;
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

        // current selection box on this page: an object's, or an annotation's that the
        // arrow can move
        self.annots(pdf, page);
        let sel_here = self.sel().filter(|s| s.0 == page);
        let sel_annot = self
            .selected_annot()
            .filter(|a| a.page == page && self.tool == Tool::Select && movable(a))
            .map(|a| (a.id, a.bbox, resizable(a)));
        // A selected line's two ends, which have handles instead of its box.
        let sel_line = self
            .selected_annot()
            .filter(|a| a.page == page && self.tool == Tool::Select && movable(a))
            .filter(|a| a.kind == AnnotKind::Line)
            .and_then(|a| match a.paths.first()?.as_slice() {
                [p, q, ..] => Some((a.id, [p.0, p.1, q.0, q.1])),
                _ => None,
            });
        let end_hit = |p: Pos2| {
            let (_, e) = sel_line?;
            (0..2).find(|&i| p.distance(Pos2::new(e[2 * i], e[2 * i + 1])) <= htol)
        };
        // A selected drawing's box and strokes; it has a rotate handle above its box.
        let sel_ink = self
            .selected_annot()
            .filter(|a| a.page == page && self.tool == Tool::Select && movable(a))
            .filter(|a| a.kind == AnnotKind::Ink)
            .map(|a| (a.id, a.bbox, a.paths.clone()));
        let rotate_at = |b: [f32; 4]| Pos2::new((b[0] + b[2]) / 2.0, b[1] - ROTATE_GAP / k);
        let rotate_hit = |p: Pos2| {
            sel_ink
                .as_ref()
                .is_some_and(|(_, b, _)| p.distance(rotate_at(*b)) <= htol)
        };
        let sel_note = self
            .selected_annot()
            .is_some_and(|a| a.kind == AnnotKind::Note);
        let sel_box = sel_here
            .and_then(|(_, id)| {
                if let Some(d) = &self.drag {
                    if d.page == page && d.id == id && d.annot.is_none() {
                        return Some(d.cur);
                    }
                }
                self.objs.get(&page)?.get(id).map(|o| o.bbox)
            })
            .or_else(|| {
                let (id, b, _) = sel_annot?;
                let d = self.drag.as_ref().filter(|d| d.annot == Some(id));
                let d = d.filter(|d| !matches!(d.mode, Mode::Rotate));
                Some(shown_box(sel_note, d.map_or(b, |d| d.cur), k))
            });
        // The box that has handles: not a note's.
        let sized_box = sel_box.filter(|_| sel_annot.is_none_or(|(_, _, r)| r));
        let annot_drag = |a: &Annot, mode: Mode, start: Pos2| Drag {
            page,
            id: 0,
            annot: Some(a.id),
            mode,
            start,
            orig: a.bbox,
            cur: a.bbox,
        };
        // Typing a note or text box: its box takes the keyboard and the pointer.
        if self.writing_ui(ui, page, r, k) {
            return resp;
        }
        let ctrl_annot = click_pt(&resp)
            .filter(|_| ctrl && resp.clicked())
            .filter(|_| Tool::ANNOTATE.contains(&self.tool))
            .and_then(|p| self.annot_at(pdf, page, p))
            .map(|a| a.id);

        match self.tool {
            // Ctrl+click on an annotation selects it in the annotation tools.
            _ if ctrl_annot.is_some() => {
                if let Some(id) = ctrl_annot {
                    self.selection = Selection::Annot { page, id };
                }
            }
            Tool::Select => {
                if let Some(p) = hover_pt {
                    let over_handle = sized_box.and_then(|b| self.handle_hit(b, p, htol));
                    let over_annot = self.annot_at(pdf, page, p).cloned();
                    if rotate_hit(p) && !self.dragging() {
                        rotate_cursor(ui.ctx());
                    } else if end_hit(p).is_some() {
                        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                    } else if let Some(h) = over_handle {
                        ui.ctx().set_cursor_icon(handle_cursor(h));
                    } else if let Some(a) = over_annot {
                        if movable(&a) {
                            ui.ctx().set_cursor_icon(CursorIcon::Grab);
                        }
                        if !self.dragging() {
                            resp = resp.on_hover_text_at_pointer(annot_tip(&a));
                        }
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
                        let h = sized_box.and_then(|b| self.handle_hit(b, p, htol));
                        let selected = self.selected_annot().filter(|_| sel_annot.is_some());
                        if let Some((id, b, _)) = sel_ink.as_ref().filter(|_| rotate_hit(p)) {
                            self.drag = Some(Drag {
                                page,
                                id: 0,
                                annot: Some(*id),
                                mode: Mode::Rotate,
                                start: p,
                                orig: *b,
                                cur: [0.0; 4],
                            });
                        } else if let (Some(i), Some((id, ends))) = (end_hit(p), sel_line) {
                            self.drag = Some(Drag {
                                page,
                                id: 0,
                                annot: Some(id),
                                mode: Mode::End(i),
                                start: p,
                                orig: ends,
                                cur: ends,
                            });
                        } else if let (Some(h), Some(a)) = (h, selected) {
                            self.drag = Some(annot_drag(a, Mode::Handle(h), p));
                        } else if let (Some(h), Some((_, id))) = (h, sel_here) {
                            self.start_drag(page, id, Mode::Handle(h), p);
                        } else if let Some(a) = self.annot_at(pdf, page, p).cloned() {
                            self.selection = Selection::Annot { page, id: a.id };
                            if movable(&a) {
                                self.drag = Some(annot_drag(&a, Mode::Move, p));
                            }
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
                                // Shift keeps the line at a multiple of 45° from its
                                // other end.
                                Mode::End(i) => {
                                    let o = 1 - i;
                                    let other = Pos2::new(d.orig[2 * o], d.orig[2 * o + 1]);
                                    let q = if shift {
                                        constrain(Tool::Line, other, p)
                                    } else {
                                        p
                                    };
                                    let mut e = d.orig;
                                    (e[2 * i], e[2 * i + 1]) = (q.x, q.y);
                                    e
                                }
                                // The angle swept about the box's middle since the press;
                                // Shift keeps to steps of 15°.
                                Mode::Rotate => {
                                    let o = d.orig;
                                    let c = Pos2::new((o[0] + o[2]) / 2.0, (o[1] + o[3]) / 2.0);
                                    let angle = |q: Pos2| (q.y - c.y).atan2(q.x - c.x);
                                    let mut deg = (angle(p) - angle(d.start)).to_degrees();
                                    deg = (deg + 180.0).rem_euclid(360.0) - 180.0;
                                    if shift {
                                        deg = (deg / 15.0).round() * 15.0;
                                    }
                                    [deg, 0.0, 0.0, 0.0]
                                }
                            };
                            match d.mode {
                                Mode::Move => ui.ctx().set_cursor_icon(CursorIcon::Grabbing),
                                Mode::Handle(h) => ui.ctx().set_cursor_icon(handle_cursor(h)),
                                Mode::End(_) => ui.ctx().set_cursor_icon(CursorIcon::Crosshair),
                                Mode::Rotate => rotate_cursor(ui.ctx()),
                            }
                        }
                    }
                }
                if resp.drag_stopped() {
                    if let Some(d) = self.drag.take() {
                        let moved = match d.mode {
                            Mode::Rotate => d.cur[0].abs() > 0.1,
                            _ => d.cur.iter().zip(&d.orig).any(|(a, b)| (a - b).abs() > 0.5),
                        };
                        if d.page == page && moved {
                            self.actions.push(match (d.annot, d.mode) {
                                (Some(id), Mode::End(_)) => Action::LineEnds(d.page, id, d.cur),
                                (Some(id), Mode::Rotate) => Action::RotateInk(d.page, id, d.cur[0]),
                                (Some(id), _) => Action::MoveAnnot(d.page, id, d.cur),
                                (None, _) => Action::SetBox(d.page, d.id, d.cur),
                            });
                        }
                    }
                }
                if resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos().map(to_pt) {
                        let h = sized_box.and_then(|b| self.handle_hit(b, p, htol));
                        if h.is_none() {
                            let annot = self.annot_at(pdf, page, p).map(|a| a.id);
                            match (annot, self.hit(page, p, tol)) {
                                (Some(id), _) => self.selection = Selection::Annot { page, id },
                                (None, Some(i)) => self.select_object(page, i),
                                (None, None) => {
                                    self.selection.clear_object();
                                    self.selection.clear_annot();
                                }
                            }
                        }
                    }
                }
                if resp.double_clicked() && self.selection.annot().is_some() {
                    self.write_selected();
                } else if resp.double_clicked() {
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
                    // On a note or text box, open it for typing; else select a word.
                    let annot = click_at().and_then(|p| self.annot_at(pdf, page, p));
                    if let Some(id) = annot.filter(|a| writable(a)).map(|a| a.id) {
                        self.selection = Selection::Annot { page, id };
                        self.write_selected();
                    } else if let Some(i) = nearest(click_at()) {
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
            Tool::Note => {
                ui.ctx().set_cursor_icon(CursorIcon::Cell);
                if let Some(p) = click_pt(&resp).filter(|_| resp.clicked()) {
                    // The icon, drawn at a fixed size on screen from the note's top left
                    // corner, is centred on the click. The note itself is 20 points.
                    let (x, y) = (p.x - NOTE_PX / 2.0 / k, p.y - NOTE_PX / 2.0 / k);
                    let rect = [x, y, x + 20.0, y + 20.0];
                    self.start_writing(Writing::new(page, true, rect, 0.0));
                }
                if let Some(a) = hover_pt.and_then(|p| self.annot_at(pdf, page, p)) {
                    resp = resp.on_hover_text_at_pointer(annot_tip(a));
                }
            }
            Tool::Image | Tool::Rect | Tool::Ellipse | Tool::Line | Tool::TextBox | Tool::Stamp => {
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
                            if shift {
                                d.cur = constrain(self.tool, d.start, d.cur);
                            }
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
                } else if resp.clicked() && self.tool == Tool::Stamp {
                    if let Some(p) = resp.interact_pointer_pos().map(to_pt) {
                        let at = StampPlace::Centre(p.x, p.y);
                        self.actions.push(Action::AddStamp(page, at));
                    }
                } else if resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos().map(to_pt) {
                        let (w, h) = match (self.tool, &self.pending_image) {
                            (Tool::Image, Some((_, aspect))) => (200.0, 200.0 / aspect.max(0.01)),
                            (Tool::Line, _) => (120.0, 0.0),
                            (Tool::TextBox, _) => (200.0, (self.box_style.size * 2.5).max(20.0)),
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
                        let shape = |s: Shape, b: [f32; 4]| {
                            if self.draw_permanent {
                                Action::AddShape(page, s, b)
                            } else {
                                Action::AddShapeAnnot(page, s, b)
                            }
                        };
                        match self.tool {
                            Tool::Image => self.actions.push(Action::AddImage(page, n)),
                            Tool::Rect => self.actions.push(shape(Shape::Rect, n)),
                            Tool::Ellipse => self.actions.push(shape(Shape::Ellipse, n)),
                            Tool::Line => self.actions.push(shape(Shape::Line, rect)),
                            Tool::Stamp => self
                                .actions
                                .push(Action::AddStamp(page, StampPlace::Fill(n))),
                            Tool::TextBox => {
                                let size = self.box_style.size;
                                self.start_writing(Writing::new(page, false, n, size));
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
            Tool::Pen => {
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                let now = ui.input(|i| i.time);
                // A stroke soon after the last one, on the same page, joins its drawing.
                let join = self
                    .ink
                    .filter(|(p, _)| *p == page && now - self.ink_at < INK_PAUSE)
                    .map(|(_, id)| id);
                if resp.drag_started() {
                    if let Some(p) = ui.input(|i| i.pointer.press_origin()).map(to_pt) {
                        self.pen_stroke = Some(PenStroke {
                            page,
                            points: vec![p],
                            join,
                        });
                    }
                }
                if resp.dragged() {
                    let s = self.pen_stroke.as_mut().filter(|s| s.page == page);
                    if let (Some(s), Some(pos)) = (s, resp.interact_pointer_pos()) {
                        let p = to_pt(pos);
                        // A point for each screen pixel moved, at most.
                        if s.points.last().is_none_or(|q| q.distance(p) * k >= 1.0) {
                            s.points.push(p);
                        }
                    }
                }
                let done = if resp.drag_stopped() {
                    self.pen_stroke.take().filter(|s| s.page == page)
                } else if resp.clicked() {
                    // A click makes a dot.
                    click_pt(&resp).map(|p| PenStroke {
                        page,
                        points: vec![p],
                        join,
                    })
                } else {
                    None
                };
                if let Some(s) = done {
                    let pts = s.points.iter().map(|p| (p.x, p.y)).collect();
                    self.actions.push(match s.join {
                        Some(id) => Action::AddInkStroke(page, id, pts),
                        None => Action::AddInk(page, pts),
                    });
                    self.ink_at = now;
                }
                if let Some(s) = self.pen_stroke.as_ref().filter(|s| s.page == page) {
                    let [cr, cg, cb] = self.pen_color.map(|v| (v * 255.0).round() as u8);
                    let line: Vec<Pos2> = s.points.iter().map(|p| to_screen(p.x, p.y)).collect();
                    painter.add(egui::Shape::line(
                        line,
                        Stroke::new((self.pen_width * k).max(1.0), Color32::from_rgb(cr, cg, cb)),
                    ));
                }
            }
        }

        // Selection frame and handles; a line has round handles on its ends instead, and
        // shows where it goes while dragged.
        if let Some((id, e)) = sel_line {
            let d = self.drag.as_ref().filter(|d| d.annot == Some(id));
            let e = match d {
                Some(d) if matches!(d.mode, Mode::End(_)) => d.cur,
                Some(d) => {
                    let (dx, dy) = (d.cur[0] - d.orig[0], d.cur[1] - d.orig[1]);
                    [e[0] + dx, e[1] + dy, e[2] + dx, e[3] + dy]
                }
                None => e,
            };
            let ends = [to_screen(e[0], e[1]), to_screen(e[2], e[3])];
            let guide = if d.is_some() {
                Stroke::new(1.5, accent_strong)
            } else {
                Stroke::new(1.0, accent_strong.gamma_multiply(0.6))
            };
            painter.line_segment(ends, guide);
            for c in ends {
                painter.circle_filled(c, 4.5, Color32::WHITE);
                painter.circle_stroke(c, 4.5, Stroke::new(1.5, accent_strong));
            }
        } else if let Some((_, b, paths)) = sel_ink.as_ref().filter(|_| {
            self.drag
                .as_ref()
                .is_some_and(|d| d.page == page && matches!(d.mode, Mode::Rotate))
        }) {
            // Turning a drawing: its strokes as they will be, in place of its box.
            let deg = self.drag.as_ref().map_or(0.0, |d| d.cur[0]);
            let c = Pos2::new((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
            let rot = egui::emath::Rot2::from_angle(deg.to_radians());
            let st = Stroke::new(1.5, accent_strong);
            for s in paths {
                let pts: Vec<Pos2> = s
                    .iter()
                    .map(|&(x, y)| {
                        let q = c + rot * (Pos2::new(x, y) - c);
                        to_screen(q.x, q.y)
                    })
                    .collect();
                painter.add(egui::Shape::line(pts, st));
            }
            painter.text(
                to_screen(c.x, c.y),
                egui::Align2::CENTER_CENTER,
                format!("{deg:.0}°"),
                egui::FontId::proportional(13.0),
                accent_strong,
            );
        } else if self.tool == Tool::Select {
            if let Some((_, b, _)) = &sel_ink {
                let (top, knob) = (
                    to_screen((b[0] + b[2]) / 2.0, b[1]),
                    to_screen(rotate_at(*b).x, rotate_at(*b).y),
                );
                painter.line_segment([top, knob], Stroke::new(1.0, accent_strong));
                painter.circle_filled(knob, 5.0, Color32::WHITE);
                painter.circle_stroke(knob, 5.0, Stroke::new(1.5, accent_strong));
            }
            if let Some(b) = sel_box {
                let sr = screen_rect(b);
                painter.rect_stroke(
                    sr,
                    0.0,
                    Stroke::new(1.5, accent_strong),
                    StrokeKind::Outside,
                );
                let handles = if sized_box.is_some() { 0..8 } else { 0..0 };
                for h in handles {
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
        // Notes at a fixed size on screen, over the page's small drawing of them.
        for a in self.annots.get(&page).into_iter().flatten() {
            if a.kind != AnnotKind::Note || !a.on_page() {
                continue;
            }
            let moving = self.drag.as_ref().filter(|d| d.annot == Some(a.id));
            let b = shown_box(true, moving.map_or(a.bbox, |d| d.cur), k);
            let sr = Rect::from_min_size(to_screen(b[0], b[1]), Vec2::splat(NOTE_PX));
            let [cr, cg, cb] = a
                .color
                .unwrap_or([1.0, 0.85, 0.2])
                .map(|v| (v * 255.0) as u8);
            let fill = Color32::from_rgb(cr, cg, cb).gamma_multiply(a.opacity.max(0.3));
            painter.rect_filled(sr, 3.0, fill);
            painter.rect_stroke(
                sr,
                3.0,
                Stroke::new(1.0, Color32::from_gray(60)),
                StrokeKind::Inside,
            );
            painter.text(
                sr.center(),
                egui::Align2::CENTER_CENTER,
                icon::NOTE,
                egui::FontId::new(NOTE_PX * 0.7, theme::icons()),
                Color32::from_gray(30),
            );
        }
        // The selected annotation: each marked line, or its box (unless the arrow has
        // framed it above).
        if sel_annot.is_none() && self.selection.annot().is_some_and(|(p, _)| p == page) {
            self.annots(pdf, page);
            if let Some(a) = self.selected_annot() {
                let shown = [shown_box(a.kind == AnnotKind::Note, a.bbox, k)];
                let boxes = if a.quads.is_empty() {
                    &shown[..]
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
        let annotating = self.tool == Tool::Select || Tool::ANNOTATE.contains(&self.tool);
        match (self.tool, p) {
            (_, Some(p)) if annotating && self.annot_at(pdf, page, p).is_some() => {
                if let Some(id) = self.annot_at(pdf, page, p).map(|a| a.id) {
                    self.selection = Selection::Annot { page, id };
                }
                return;
            }
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
                annot: None,
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
            Tool::Text
                | Tool::Image
                | Tool::Rect
                | Tool::Ellipse
                | Tool::Line
                | Tool::Note
                | Tool::TextBox
                | Tool::Pen
                | Tool::Stamp
        ) || self.sel().is_some()
            || self.selection.words().is_some()
            || self.selection.annot().is_some()
    }

    /// The page and box (page points) of the selected object or words, for placing the
    /// floating toolbar. For words, the part on the page where the selection's moving end
    /// is. A drawing's box includes its rotate handle, so the toolbar doesn't cover it.
    pub fn selection_box(&self) -> Option<(usize, [f32; 4])> {
        if let Some(o) = self.selected_object() {
            return Some((self.sel()?.0, o.bbox));
        }
        if let Some(a) = self.selected_annot() {
            let mut b = a.bbox;
            if a.kind == AnnotKind::Ink && self.tool == Tool::Select && movable(a) {
                b[1] -= (ROTATE_GAP + 8.0) / self.zoom.max(0.01);
            }
            return Some((a.page, b));
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

    /// Move the selected object, or annotation (arrow tool), by (dx, dy) points.
    pub fn nudge(&mut self, dx: f32, dy: f32) {
        if self.tool == Tool::Select {
            if let Some(a) = self.selected_annot().filter(|a| movable(a)) {
                let b = a.bbox;
                let to = [b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy];
                self.actions.push(Action::MoveAnnot(a.page, a.id, to));
                return;
            }
        }
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
        if self.writing.take().is_some() {
            return;
        }
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
            text: a.text_style,
            shape: a.shape,
        };
        if self.annot_insp.as_ref().is_none_or(|(id, _)| *id != a.id) {
            if self.annot_insp.is_some() {
                self.reply.clear();
            }
            self.annot_insp = Some((a.id, now.clone()));
        }
        let replies: Vec<Annot> = self
            .annots
            .get(&a.page)
            .into_iter()
            .flatten()
            .filter(|r| r.reply_to == Some(a.id))
            .cloned()
            .collect();
        let Some((_, e)) = self.annot_insp.as_mut() else {
            return;
        };
        ui.strong(if a.reply_to.is_some() {
            "Reply".into()
        } else {
            a.kind.label()
        });
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
            match e.text.as_mut() {
                // A text box's colour is its fill, set with its text.
                Some(s) => text_style_ui(ui, s),
                // A stamp's colours are part of its drawing.
                None if a.kind == AnnotKind::Stamp => {}
                None => {
                    ui.horizontal(|ui| {
                        let mut on = e.color.is_some();
                        let label = match a.kind {
                            AnnotKind::Square | AnnotKind::Circle => "Outline",
                            AnnotKind::Line | AnnotKind::Ink => "Line",
                            _ => "Colour",
                        };
                        if ui.checkbox(&mut on, label).changed() {
                            e.color = on.then_some(a.color.unwrap_or([0.0; 3]));
                        }
                        if let Some(c) = e.color.as_mut() {
                            ui.color_edit_button_rgb(c);
                        }
                        if let Some(s) = e.shape.as_mut() {
                            ui.add(
                                egui::DragValue::new(&mut s.width)
                                    .range(0.0..=40.0)
                                    .speed(0.1)
                                    .suffix(" pt"),
                            );
                        }
                    });
                }
            }
            if let Some(s) = e.shape.as_mut() {
                match a.kind {
                    AnnotKind::Square | AnnotKind::Circle => {
                        ui.horizontal(|ui| {
                            let mut on = s.fill.is_some();
                            if ui.checkbox(&mut on, "Fill").changed() {
                                s.fill = on.then_some([1.0, 1.0, 0.6]);
                            }
                            if let Some(c) = s.fill.as_mut() {
                                ui.color_edit_button_rgb(c);
                            }
                        });
                    }
                    AnnotKind::Line => {
                        ui.checkbox(&mut s.arrow, "Arrowhead");
                    }
                    _ => {}
                }
            }
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
            ui.label(if e.text.is_some() { "Text" } else { "Comment" });
            ui.add(
                egui::TextEdit::multiline(&mut e.contents)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY),
            );
            if writable(&a) {
                ui.weak("Double-click it on the page to type there.");
            }
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
        // Replies, then a box to add one. Answering doesn't change the annotation, so
        // locked ones can be answered too.
        ui.separator();
        for r in &replies {
            let who: Vec<&str> = [r.author.as_str(), r.modified.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect();
            ui.weak(if who.is_empty() {
                "Reply".to_string()
            } else {
                who.join(" · ")
            });
            ui.label(&r.contents);
            ui.add_space(4.0);
        }
        let out = ui.add(
            egui::TextEdit::multiline(&mut self.reply)
                .hint_text("Reply")
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        if self.reply_focus {
            out.request_focus();
            self.reply_focus = false;
        }
        let ready = !self.reply.trim().is_empty();
        if ui
            .add_enabled(
                ready,
                egui::Button::new((theme::icon(icon::ARROW_BEND_UP_LEFT), "Add reply")),
            )
            .clicked()
        {
            let text = std::mem::take(&mut self.reply);
            self.actions.push(Action::Reply(a.page, a.id, text));
        }
    }

    /// The right-hand inspector: tool options, new-text entry and properties of the selected object.
    /// The stamp tool's options: the ready-made stamps, your own wordings, the
    /// name-and-date switch and the pictures used before.
    fn stamp_ui(&mut self, ui: &mut egui::Ui) {
        let look = |word: &str, color: StampColor| {
            let [r, g, b] = color.rgb().map(|c| (c * 255.0) as u8);
            egui::RichText::new(word)
                .strong()
                .color(Color32::from_rgb(r, g, b))
        };
        ui.strong("New stamp");
        for k in StampKind::ALL {
            let pick = StampPick::Text(k);
            let text = look(&k.label().to_uppercase(), k.color());
            if ui.selectable_label(self.stamp == pick, text).clicked() {
                self.stamp = pick;
            }
        }
        let mut remove = None;
        for (i, (word, color)) in self.stamp_wordings.iter().enumerate() {
            let pick = StampPick::Custom(word.clone(), *color);
            let r = ui.selectable_label(self.stamp == pick, look(word, *color));
            if r.clicked() {
                self.stamp = pick;
            }
            r.context_menu(|ui| {
                if ui.button("Remove from the list").clicked() {
                    remove = Some(i);
                    ui.close();
                }
            });
        }
        if let Some(i) = remove {
            let (word, color) = self.stamp_wordings.remove(i);
            if self.stamp == StampPick::Custom(word, color) {
                self.stamp = StampPick::Text(StampKind::Approved);
            }
        }
        ui.label("Your own wording");
        // A colour applies to what is typed next and to the picked wording of your own.
        ui.horizontal(|ui| {
            for c in StampColor::ALL {
                let name = format!("{}{}", c.name()[..1].to_uppercase(), &c.name()[1..]);
                if ui
                    .selectable_label(self.stamp_draft_color == c, look(&name, c))
                    .clicked()
                {
                    self.stamp_draft_color = c;
                    if let StampPick::Custom(word, color) = &mut self.stamp {
                        *color = c;
                        for w in self.stamp_wordings.iter_mut().filter(|w| w.0 == *word) {
                            w.1 = c;
                        }
                    }
                }
            }
        });
        let entry = ui.add(
            egui::TextEdit::singleline(&mut self.stamp_draft)
                .hint_text("Type, then click on the page")
                .desired_width(f32::INFINITY),
        );
        // What is typed is the stamp to put down straight away; it joins the list when
        // the box is left (Enter, or a click on the page or elsewhere).
        let wording = mpe_core::stamp_wording(&self.stamp_draft);
        if let (true, Ok(word)) = (entry.changed(), &wording) {
            self.stamp = StampPick::Custom(word.clone(), self.stamp_draft_color);
        }
        match wording {
            Ok(word) if entry.lost_focus() => {
                let pick = (word, self.stamp_draft_color);
                self.stamp_wordings.retain(|w| w.0 != pick.0);
                self.stamp_wordings.insert(0, pick.clone());
                self.stamp_wordings
                    .truncate(crate::settings::STAMP_WORDINGS);
                self.stamp = StampPick::Custom(pick.0, pick.1);
                self.stamp_draft.clear();
            }
            Err(e) if !self.stamp_draft.trim().is_empty() => {
                let e = e.to_string();
                let e = format!("{}{}", e[..1].to_uppercase(), &e[1..]);
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            _ => {}
        }
        ui.checkbox(&mut self.stamp_name_date, "Add my name and the date");
        ui.add_enabled_ui(self.stamp_name_date, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut self.author).hint_text("No name"))
                    .on_hover_text("Also signs your comments (Edit > Your name for comments)");
            });
        });
        ui.separator();
        ui.label("Pictures");
        let mut remove = None;
        for (i, path) in self.stamp_pictures.iter().enumerate() {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into(),
            );
            let pick = StampPick::Picture(path.clone());
            let r = ui
                .selectable_label(self.stamp == pick, name)
                .on_hover_text(path.display().to_string());
            if r.clicked() {
                self.stamp = pick;
            }
            r.context_menu(|ui| {
                if ui.button("Remove from the list").clicked() {
                    remove = Some(i);
                    ui.close();
                }
            });
        }
        if let Some(i) = remove {
            let path = self.stamp_pictures.remove(i);
            if self.stamp == StampPick::Picture(path) {
                self.stamp = StampPick::Text(StampKind::Approved);
            }
        }
        if ui.button("Choose a picture…").clicked() {
            if let Some(path) = pick_image_path() {
                self.stamp_pictures.retain(|p| *p != path);
                self.stamp_pictures.insert(0, path.clone());
                self.stamp_pictures
                    .truncate(crate::settings::STAMP_PICTURES);
                self.stamp = StampPick::Picture(path);
            }
        }
        ui.separator();
        ui.label(
            "Click on the page to put the stamp down, or drag a box to size it. The arrow \
             moves and resizes it afterwards. Ctrl+click selects an annotation.",
        );
    }

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
                if self.tool == Tool::Line && !self.draw_permanent {
                    ui.checkbox(&mut self.arrow, "Arrowhead");
                }
                ui.checkbox(&mut self.draw_permanent, "Draw on page (permanent)")
                    .on_hover_text(
                        "Draw into the page itself instead of adding an annotation. It can \
                         still be moved with the arrow, but it is no longer a comment that \
                         other apps can list, hide or remove.",
                    );
                ui.label(if self.draw_permanent {
                    "Drag on the page, or click for a default size. Shift keeps squares, \
                     circles and 45° lines."
                } else {
                    "Drag on the page, or click for a default size. Shift keeps squares, \
                     circles and 45° lines. Ctrl+click selects an annotation."
                });
                ui.separator();
            }
            Tool::Pen => {
                ui.strong("Pen");
                ui.horizontal(|ui| {
                    ui.color_edit_button_rgb(&mut self.pen_color);
                    ui.add(
                        egui::DragValue::new(&mut self.pen_width)
                            .range(0.25..=40.0)
                            .speed(0.1)
                            .suffix(" pt"),
                    );
                });
                ui.label(
                    "Drag to draw. Strokes made one soon after another become one drawing, \
                     which the arrow tool moves as a whole and Undo takes back a stroke at a \
                     time. Ctrl+click selects an annotation.",
                );
                return;
            }
            Tool::Note => {
                ui.strong("New note");
                ui.horizontal(|ui| {
                    ui.label("Colour");
                    ui.color_edit_button_rgb(&mut self.note_color);
                });
                ui.label(
                    "Click on the page to place a note, type its comment, then press Done \
                     (or Ctrl+Enter). Ctrl+click selects an annotation.",
                );
                return;
            }
            Tool::Stamp => {
                self.stamp_ui(ui);
                return;
            }
            Tool::TextBox => {
                ui.strong("New text box");
                text_style_ui(ui, &mut self.box_style);
                ui.label(
                    "Drag a box on the page, or click for a default size, then type. Click \
                     outside the box or press Ctrl+Enter to finish; Esc drops it. Ctrl+click \
                     selects an annotation.",
                );
                return;
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

/// A text box's text size and colour, fill and border.
fn text_style_ui(ui: &mut egui::Ui, s: &mut TextStyle) {
    ui.horizontal(|ui| {
        ui.label("Text");
        ui.add(
            egui::DragValue::new(&mut s.size)
                .range(4.0..=200.0)
                .suffix(" pt"),
        );
        ui.color_edit_button_rgb(&mut s.color);
    });
    ui.horizontal(|ui| {
        let mut on = s.fill.is_some();
        if ui.checkbox(&mut on, "Background").changed() {
            s.fill = on.then_some([1.0, 1.0, 0.85]);
        }
        if let Some(c) = s.fill.as_mut() {
            ui.color_edit_button_rgb(c);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Border");
        ui.add(
            egui::DragValue::new(&mut s.border)
                .range(0.0..=20.0)
                .speed(0.1)
                .suffix(" pt"),
        );
    });
}

/// Ask for an image file and return its bytes if it can be decoded.
pub fn pick_image() -> Option<Vec<u8>> {
    std::fs::read(pick_image_path()?).ok()
}

pub fn pick_image_path() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Images", &["png", "jpg", "jpeg"])
        .pick_file()
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
