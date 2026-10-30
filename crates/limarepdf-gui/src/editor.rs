//! Content editing state and interaction: selecting, moving and resizing objects, inserting text, images
//! and shapes, and the inspector panel.

use crate::chosen::{Chosen, Left, Of};
use crate::dialogs;
use crate::fields::FormUi;
use crate::icon;
use crate::inspector::{facts, note, section, section_name};
use crate::page_text::{self, selection, PageText};
use crate::selection::{ObjKind, Selected, Selection, Toward, WordAt};
use crate::signing::{SignKit, SignPane, SignPick};
use crate::styled::{History, InLine, Restyle, Styled};
use crate::theme;
use crate::typing_fonts::Wanted;
use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use limarepdf_core::page_colours::PageColours;
use limarepdf_core::{
    words_text, Annot, AnnotKind, BoxSetting, FieldValue, FontChoice, FontRefusal, LineFont, Link,
    LinkTarget, ObjectKind, PageObject, PageWord, PartFont, PathStyle, Shape, ShapeLook, SignPlace,
    StampColor, StampKind, StampPlace, TextEdit, TextPart, TextStyle, MIN_SIGNATURE_BOX,
};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use unicode_segmentation::UnicodeSegmentation;

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
    /// Drag boxes to mark areas for redaction; nothing is removed until applied.
    Redact,
    /// Fill and sign: put down your signature, initials, the date, your name, text or a
    /// tick, cross or dot.
    Sign,
    /// Choose where a digital signature goes: click an empty signature field or drag a
    /// box. The signing window opens next.
    DigitalSign,
}

impl Tool {
    /// The tools behind the toolbar's Comment button.
    pub const COMMENTING: [Tool; 7] = [
        Tool::Note,
        Tool::TextBox,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Line,
        Tool::Pen,
        Tool::Stamp,
    ];

    /// The tools that add annotations: the commenting tools and redaction marks.
    pub const ANNOTATE: [Tool; 8] = [
        Tool::Note,
        Tool::TextBox,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Line,
        Tool::Pen,
        Tool::Stamp,
        Tool::Redact,
    ];

    /// The tools that put one thing down and go back to the read tool, unless kept active
    /// (see [`Editor::keep`]). The pen stays in hand until Esc or another tool, as a
    /// drawing takes several strokes. The Note tool always goes back: kept active, every
    /// click put down a note, so none could be opened or deleted.
    pub const PLACING: [Tool; 9] = [
        Tool::Text,
        Tool::Image,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Line,
        Tool::TextBox,
        Tool::Stamp,
        Tool::Redact,
        Tool::Sign,
    ];

    /// The tool's name in the settings file, for those in [`Tool::PLACING`].
    pub fn key(self) -> &'static str {
        match self {
            Tool::Text => "text",
            Tool::Image => "image",
            Tool::Rect => "rectangle",
            Tool::Ellipse => "ellipse",
            Tool::Line => "line",
            Tool::TextBox => "text_box",
            Tool::Stamp => "stamp",
            Tool::Redact => "redact",
            Tool::Sign => "fill_and_sign",
            Tool::Read | Tool::Select | Tool::Pen | Tool::Note | Tool::DigitalSign => "",
        }
    }

    /// The placing tool named `key` in the settings file.
    pub fn from_key(key: &str) -> Option<Tool> {
        Tool::PLACING.into_iter().find(|t| t.key() == key)
    }
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

/// A kind of typing that isn't in the document yet (see [`Editor::pending`]), in the
/// order they are kept before a command ([`Editor::keep`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Typed {
    /// New text on a page.
    Text,
    /// A note or text box.
    Writing,
    /// The selected object's changes, in the inspector or typed over its line.
    Object,
    /// A form field.
    Field,
    /// The selected comment's changes in the inspector.
    Comment,
    /// A reply to a comment: every draft reply, one at a time (see [`Editor::draft`]).
    Reply,
    /// The look of words selected with the read tool, or of lines among several objects
    /// selected with the arrow, changed in the inspector.
    Words,
    /// The look of the shapes among several objects selected, changed in the inspector.
    Objects,
}

impl Typed {
    pub const ALL: [Typed; 8] = [
        Typed::Text,
        Typed::Writing,
        Typed::Object,
        Typed::Field,
        Typed::Comment,
        Typed::Reply,
        Typed::Words,
        Typed::Objects,
    ];

    /// What Undo and Redo keep first: all but draft replies, which stay drafts, as Word and
    /// Google Docs keep them. Added first, Undo would take one back instead of the change
    /// before.
    pub const BUT_DRAFTS: [Typed; 7] = [
        Typed::Text,
        Typed::Writing,
        Typed::Object,
        Typed::Field,
        Typed::Comment,
        Typed::Words,
        Typed::Objects,
    ];
}

/// A reply typed to a comment and not yet added: kept with its comment until it is, as
/// Word and Google Docs keep a draft (batch 42's Q1).
#[derive(Clone, Debug)]
struct Draft {
    /// The comment's page, or `None` while the comment isn't in the document (Undo took it
    /// away; Redo brings both back).
    page: Option<usize>,
    text: String,
}

impl Draft {
    /// It can be added as a reply: there is something to add, and its comment to add it to.
    fn ready(&self) -> bool {
        self.page.is_some() && !self.text.trim().is_empty()
    }
}

/// What [`Editor::keep`] took from the editor to keep, to put back if keeping it fails.
pub struct Held(Kept);

enum Kept {
    Text(Box<PageText>),
    Writing(Writing),
    Object(Box<(Insp, Option<LineEdit>)>),
    /// The value the field had before.
    Field(String),
    Comment,
    /// The draft and the comment it answers.
    Reply((u32, u16), Draft),
    Words(Box<Chosen>),
    Objects(Box<Group>),
}

/// Typing put back after its change failed ([`Editor::settle_kept`]), as it was then: new
/// text, a note's or text box's text, or the selected object's line and look.
enum NotKept {
    Text(Styled),
    Writing(String),
    Object((usize, usize), Styled, Look),
}

impl NotKept {
    fn of(held: &Held) -> Option<Self> {
        match &held.0 {
            Kept::Text(a) => Some(Self::Text(a.text.clone())),
            Kept::Writing(w) => Some(Self::Writing(w.text.clone())),
            Kept::Object(o) => Some(Self::Object(o.0.key, o.0.text.clone(), o.0.look())),
            _ => None,
        }
    }
}

/// What had the keyboard on the page when F6 took it to another region of the window
/// (ENH-10), given it back when F6 comes back to the page.
#[derive(Clone, Debug, PartialEq)]
pub enum Parked {
    /// New text, a line being edited, or a note or text box, still open: its text box.
    /// A click anywhere ends the wait, and the typing then goes on as after any click.
    Typing(egui::Id),
    /// A form field's text box, whose value was kept as it was left: the field's name
    /// and widget.
    Field(String, usize),
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
    /// A text box's border's width in points.
    border: f32,
    /// Takes the keyboard on the next frame.
    fresh: bool,
    /// Takes the keyboard back once a change in the inspector is finished.
    refocus: bool,
    /// A list or colour picker was open last frame: the click that closes it is not one
    /// outside a new note.
    picking: bool,
    /// The note about a character a text box can't write.
    refused: page_text::Refused,
    /// What an input method is composing in it, from and to ([`page_text::composition`]).
    composing: Option<(usize, usize)>,
}

impl Writing {
    /// Take what an input method was still composing as committed, as when the box loses
    /// the keyboard: in a text box, set in Helvetica, what can't be written is left out
    /// ([`page_text::commit_left`]); a note's comment takes any character. Returns the
    /// first character left out.
    fn commit_composition(&mut self) -> Option<String> {
        let r = self.composing.take().filter(|_| !self.note)?;
        let helvetica = FontChoice::default();
        let (text, left) = page_text::commit_left(&self.text, r, false, helvetica);
        self.text = text;
        left
    }

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
            border: 0.0,
            fresh: true,
            refocus: false,
            picking: false,
            refused: page_text::Refused::default(),
            composing: None,
        }
    }

    /// Whether finishing it changes the document: a new one with some text, or changed
    /// text.
    fn changed(&self) -> bool {
        match self.id {
            Some(_) => self.text != self.original,
            None => !self.text.trim().is_empty(),
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
    /// Objects of a page moved or resized: each to the box given.
    SetBox(usize, Vec<(usize, [f32; 4])>),
    /// Objects of a page deleted.
    Delete(usize, Vec<usize>),
    AddShape(usize, Shape, [f32; 4]),
    /// New text, its lines separated by `\n`, from the first line's baseline at the point
    /// given, in the styles it was typed in.
    AddText(usize, (f32, f32), Styled),
    AddImage(usize, [f32; 4]),
    ReplaceImage(usize, usize, Vec<u8>),
    /// A line's change; true when only its words changed, not their styles, so it can join
    /// a recent change of the same line.
    EditText(usize, usize, TextEdit, bool),
    /// Words selected with the read tool restyled: each page's changed lines, by object, and
    /// the lines with selected words left as they were. The words stay selected.
    Restyle(Vec<(usize, Vec<(usize, TextEdit)>)>, Left),
    /// Shapes of a page restyled.
    PathStyle(usize, Vec<(usize, PathStyle)>),
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
    /// Mark an area (displayed-page points) for redaction.
    AddRedactMark(usize, [f32; 4]),
    /// The Sign tool's picked item.
    AddSign(usize, StampPlace),
    /// Sign in an empty signature field: its page and box. Switches to the Sign tool.
    SignHere(usize, [f32; 4]),
    /// A signed signature field was clicked: show its digital signature's details (the
    /// field's full name).
    SignatureDetails(String),
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

impl AnnotEdit {
    /// The annotation's properties as they are.
    fn of(a: &Annot) -> Self {
        Self {
            color: a.color,
            opacity: a.opacity,
            contents: a.contents.clone(),
            text: a.text_style,
            shape: a.shape,
        }
    }

    /// Whether it changes nothing in `was`, colours compared as [`same_color`] does.
    fn same_as(&self, was: &AnnotEdit) -> bool {
        let col = |a: Option<[f32; 3]>, b: Option<[f32; 3]>| match (a, b) {
            (Some(a), Some(b)) => same_color(a, b),
            (a, b) => a.is_none() && b.is_none(),
        };
        let text = match (self.text, was.text) {
            (Some(a), Some(b)) => {
                same_color(a.color, b.color)
                    && col(a.fill, b.fill)
                    && (a.size, a.border) == (b.size, b.border)
            }
            (a, b) => a.is_none() && b.is_none(),
        };
        let shape = match (self.shape, was.shape) {
            (Some(a), Some(b)) => col(a.fill, b.fill) && (a.width, a.arrow) == (b.width, b.arrow),
            (a, b) => a.is_none() && b.is_none(),
        };
        col(self.color, was.color)
            && self.opacity == was.opacity
            && self.contents == was.contents
            && text
            && shape
    }
}

/// Whether the annotation can be moved: not locked, and not text markup, which stays on
/// its text. A redaction mark moves, all its areas together (PF-37).
pub fn movable(a: &Annot) -> bool {
    !a.locked() && (a.quads.is_empty() || a.kind == AnnotKind::Redact)
}

/// Whether it can also be resized by its box: a note's icon keeps its size, a line has
/// handles on its ends instead, and a redaction mark's areas over several lines follow
/// their text.
pub fn resizable(a: &Annot) -> bool {
    movable(a) && !matches!(a.kind, AnnotKind::Note | AnnotKind::Line) && a.quads.len() <= 1
}

/// Whether `p` (page points) is in a redaction mark's box, which its outline shows when it
/// is selected: a press there moves it, between the lines it marks too.
fn in_mark_box(a: &Annot, p: Pos2) -> bool {
    let b = a.bbox;
    a.kind == AnnotKind::Redact && p.x >= b[0] && p.x <= b[2] && p.y >= b[1] && p.y <= b[3]
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
        egui::Id::unique("rotate-cursor"),
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
    /// Objects moved together (`Mode::Move`), with their boxes as they were: `orig` and
    /// `cur` are the box round them all.
    group: Vec<(usize, [f32; 4])>,
}

struct Draw {
    page: usize,
    start: Pos2,
    cur: Pos2,
}

struct Marquee {
    page: usize,
    start: Pos2,
    cur: Pos2,
    /// Shift was held: what it selects is added to the objects selected.
    add: bool,
}

impl Marquee {
    /// The rectangle (page points).
    fn rect(&self) -> [f32; 4] {
        let (a, b) = (self.start, self.cur);
        [a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)]
    }
}

/// Whether box `b` is wholly inside box `r`.
fn inside(b: [f32; 4], r: [f32; 4]) -> bool {
    b[0] >= r[0] && b[1] >= r[1] && b[2] <= r[2] && b[3] <= r[3]
}

/// Box `b` moved by `(dx, dy)`.
fn shifted(b: [f32; 4], dx: f32, dy: f32) -> [f32; 4] {
    [b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy]
}

/// Working copy of the selected object's properties shown in the inspector.
struct Insp {
    key: (usize, usize),
    /// A line's text, a style for each character.
    text: Styled,
    fill_on: bool,
    fill: [f32; 3],
    stroke_on: bool,
    stroke: [f32; 3],
    width: f32,
    geom: [f32; 4],
    /// The line as it is in the document.
    orig: Styled,
    /// Undo and redo inside the line, until the change is kept.
    history: History,
    /// A line's fonts, which say what its words can be written in
    /// ([`limarepdf_core::Pdf::line_fonts`]): read by the page it is on, once.
    fonts: Option<Vec<LineFont>>,
}

impl Insp {
    /// The properties of object `o`, at `key`, as they are.
    fn of(key: (usize, usize), o: &PageObject) -> Self {
        let text = if o.kind == ObjectKind::Text {
            Styled::of(o, font_of)
        } else {
            Styled::default()
        };
        Insp {
            key,
            orig: text.clone(),
            history: History::new(&text),
            text,
            fill_on: o.has_fill,
            fill: o.fill.unwrap_or([0.8, 0.8, 0.8]),
            stroke_on: o.has_stroke,
            stroke: o.stroke.unwrap_or([0.0; 3]),
            width: o.line_width.unwrap_or(1.0),
            geom: o.bbox,
            fonts: None,
        }
    }

    /// Its shape's look, as changed.
    fn look(&self) -> Look {
        Look {
            fill_on: self.fill_on,
            fill: self.fill,
            stroke_on: self.stroke_on,
            stroke: self.stroke,
            width: self.width,
        }
    }
}

/// A shape's look, as the inspector shows and changes it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Look {
    fill_on: bool,
    fill: [f32; 3],
    stroke_on: bool,
    stroke: [f32; 3],
    width: f32,
}

impl Look {
    fn of(o: &PageObject) -> Self {
        Look {
            fill_on: o.has_fill,
            fill: o.fill.unwrap_or([0.8, 0.8, 0.8]),
            stroke_on: o.has_stroke,
            stroke: o.stroke.unwrap_or([0.0; 3]),
            width: o.line_width.unwrap_or(1.0),
        }
    }

    /// Whether it looks as `was` does, colours compared as [`same_color`] does.
    fn same_as(&self, was: &Look) -> bool {
        (self.fill_on, self.stroke_on) == (was.fill_on, was.stroke_on)
            && same_color(self.fill, was.fill)
            && same_color(self.stroke, was.stroke)
            && (self.width - was.width).abs() < 0.005
    }

    fn style(&self) -> PathStyle {
        PathStyle {
            fill: self.fill_on.then_some(self.fill),
            stroke: self.stroke_on.then_some(self.stroke),
            line_width: self.width,
        }
    }
}

/// Working copy of several selected objects' properties shown in the inspector: the look
/// of the shapes among them, changed together.
struct Group {
    page: usize,
    /// The objects, as selected.
    ids: Vec<usize>,
    /// Each shape among them: its index, its look in the document and as changed.
    shapes: Vec<(usize, Look, Look)>,
    /// Where the box round them all starts (page points), as the inspector moves it.
    at: [f32; 2],
}

impl Group {
    /// Objects `ids` of a page whose objects are `objs`, as they are.
    fn of(page: usize, ids: Vec<usize>, objs: &[PageObject]) -> Self {
        let held: Vec<(usize, &PageObject)> = ids
            .iter()
            .filter_map(|&id| Some((id, objs.get(id)?)))
            .collect();
        let shapes = (held.iter())
            .filter(|(_, o)| o.kind == ObjectKind::Path)
            .map(|&(id, o)| (id, Look::of(o), Look::of(o)))
            .collect();
        let b = held.iter().map(|(_, o)| o.bbox).reduce(union);
        let at = b.map_or([0.0; 2], |b| [b[0], b[1]]);
        Group {
            page,
            ids,
            shapes,
            at,
        }
    }

    /// The shapes' changes not yet in the document, as the change that makes them.
    fn change(&self) -> Option<Action> {
        let styles: Vec<(usize, PathStyle)> = (self.shapes.iter())
            .filter(|(_, was, now)| !now.same_as(was))
            .map(|(id, _, now)| (*id, now.style()))
            .collect();
        (!styles.is_empty()).then_some(Action::PathStyle(self.page, styles))
    }

    /// Change every shape's look with `f`.
    fn set(&mut self, f: impl Fn(&mut Look)) {
        self.shapes.iter_mut().for_each(|(.., now)| f(now));
    }

    /// The value `f` gives every shape, when they all have the same one.
    fn all<T: PartialEq>(&self, f: impl Fn(&Look) -> T) -> Option<T> {
        let mut values = self.shapes.iter().map(|(.., now)| f(now));
        let first = values.next()?;
        values.all(|v| v == first).then_some(first)
    }

    /// The colours `f` gives the shapes, each once, in the order of the shapes.
    fn colors(&self, f: impl Fn(&Look) -> [f32; 3]) -> Vec<[f32; 3]> {
        let mut out: Vec<[f32; 3]> = Vec::new();
        for (.., now) in &self.shapes {
            let c = f(now);
            if !out.iter().any(|o| same_color(*o, c)) {
                out.push(c);
            }
        }
        out
    }
}

/// The font sizes offered in the size list, as word processors offer them.
const SIZES: [f32; 16] = [
    6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 36.0, 48.0, 72.0,
];

/// A size as the size box shows it: the exact value, as short as it can be written and still
/// read back the same, so an unusual size such as 10.126 isn't shown rounded.
fn size_text(v: f32) -> String {
    format!("{v}")
}

/// A size typed in the size box, kept within `range`; `None` if it isn't a number.
fn parse_size(s: &str, range: &std::ops::RangeInclusive<f32>) -> Option<f32> {
    let v: f32 = s.trim().trim_end_matches("pt").trim().parse().ok()?;
    v.is_finite().then(|| v.clamp(*range.start(), *range.end()))
}

/// The font size: a box showing the exact size, which can be typed in (Enter or leaving it
/// keeps it), and a list of the usual sizes beside it. Returns true when a size was chosen.
/// Going into the box and out again without typing changes nothing; text that isn't a
/// number leaves the size as it was.
fn size_ui(ui: &mut egui::Ui, size: &mut f32, range: std::ops::RangeInclusive<f32>) -> bool {
    let chosen = size_box(ui, Some(*size), range);
    ui.label("pt");
    if let Some(v) = chosen {
        *size = v;
    }
    chosen.is_some()
}

/// The font size as [`size_ui`] shows it, for `size`, or empty for text in several sizes,
/// where any size chosen counts. Returns the size chosen.
fn size_box(
    ui: &mut egui::Ui,
    size: Option<f32>,
    range: std::ops::RangeInclusive<f32>,
) -> Option<f32> {
    let id = ui.scope_id().with("size-box");
    // What was typed, kept only once something has been.
    let typed = ui.data(|d| d.get_temp::<String>(id));
    let mut text = typed
        .clone()
        .unwrap_or_else(|| size.map(size_text).unwrap_or_default());
    let r = ui
        .add(
            egui::TextEdit::singleline(&mut text)
                .id(id)
                .desired_width(44.0),
        )
        .on_hover_text("Font size");
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Font size"));
    let edited = typed.is_some() || r.changed();
    let mut chosen = None;
    // Esc drops what was typed.
    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if r.lost_focus() && edited && !esc {
        chosen = parse_size(&text, &range).filter(|v| Some(*v) != size);
    }
    // While typing, the box keeps what is typed; otherwise it shows the size.
    if r.has_focus() && edited {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    let list =
        egui::Button::new(theme::icon(icon::CARET_DOWN).size(10.0)).frame_when_inactive(false);
    egui::containers::menu::MenuButton::from_button(list)
        .ui(ui, |ui| {
            for v in SIZES.into_iter().filter(|v| range.contains(v)) {
                let on = Some(v) == size;
                let r = ui.selectable_label(on, size_text(v));
                dialogs::set_name(&r, &format!("{} pt", size_text(v)));
                if r.clicked() {
                    if !on {
                        chosen = Some(v);
                    }
                    ui.close();
                }
            }
        })
        .0
        .on_hover_text("Font sizes")
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Font sizes"));
    chosen
}

/// A colour button for PDF colour `c`, named for screen readers (egui leaves it unnamed).
pub fn color_button(ui: &mut egui::Ui, c: &mut [f32; 3], name: &str) -> egui::Response {
    // The value egui's picker last gave for `c`: converting back and forth isn't exact,
    // and egui keeps a grey's hue by the value it gave (BUG-38).
    let key = ui.auto_id_with("pdf-colour");
    let mut shown = ui
        .data(|d| d.get_temp::<([f32; 3], [f32; 3])>(key))
        .filter(|(pdf, _)| pdf == c)
        .map_or_else(|| to_linear(*c), |(_, shown)| shown);
    let r = ui.color_edit_button_rgb(&mut shown);
    // Only a change made in the picker is taken back: a colour only looked at stays
    // exactly as it was.
    if r.changed() {
        *c = from_linear(shown);
        ui.data_mut(|d| d.insert_temp(key, (*c, shown)));
    }
    dialogs::set_name(&r, name);
    r
}

/// PDF colour `c` as egui's colour pickers take it. A PDF's RGB values are what the page
/// shows (gamma-encoded, as sRGB), while egui's pickers work in linear light; given the
/// values as they are, a mid grey looked lighter in the picker than on the page (BUG-38).
fn to_linear(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| egui::ecolor::linear_from_gamma(v.clamp(0.0, 1.0)))
}

/// The PDF colour of a colour egui's pickers give ([`to_linear`]).
fn from_linear(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| egui::ecolor::gamma_from_linear(v).clamp(0.0, 1.0))
}

/// The colour button for text in `colors` (each once, the first the picker starts from),
/// drawn as word processors draw it: a letter A over a bar of the colour, outlined so black
/// shows on the dark theme, and a ▾ for the picker. With more than one colour, the bar
/// shows them side by side and the button says the colour is mixed, and its picker offers
/// each to make it all that colour. Returns the colour picked: anything done in the picker
/// is a pick, even of the colour it started from, so picking the first colour of mixed text
/// makes it all that colour; opening and closing it is not.
fn text_color_button(ui: &mut egui::Ui, colors: &[[f32; 3]]) -> Option<[f32; 3]> {
    colors_button(ui, colors, TEXT_SWATCH)
}

/// What a colour button for one colour or several shows and is called
/// ([`colors_button`]).
#[derive(Clone, Copy)]
struct Swatch {
    /// What it colours, as its tooltip and screen readers say: "Text colour".
    name: &'static str,
    /// A letter A over a bar of the colour, as for text; else a patch of it.
    letter: bool,
    /// Where the colours its picker offers are: "in the text".
    within: &'static str,
    /// The heading over them: "In the text".
    offered: &'static str,
    /// What picking one of them does: "Make it all this colour".
    all: &'static str,
}

const TEXT_SWATCH: Swatch = Swatch {
    name: "Text colour",
    letter: true,
    within: "in the text",
    offered: "In the text",
    all: "Make it all this colour",
};

/// The colour button for things in `colors` (each once, the first the picker starts
/// from), as [`text_color_button`] is for text: a patch of the colour (or an A over a bar,
/// `swatch.letter`) and a ▾ for the picker, the colours side by side when they are mixed,
/// and the picker offering each.
fn colors_button(ui: &mut egui::Ui, colors: &[[f32; 3]], swatch: Swatch) -> Option<[f32; 3]> {
    use egui::widgets::color_picker::{color_picker_hsva_2d, Alpha};
    let was = colors.first().copied().unwrap_or([0.0; 3]);
    let mixed = colors.len() > 1;
    let popup = ui.auto_id_with("text-colour");
    let open = egui::Popup::is_id_open(ui.ctx(), popup);
    let size = egui::vec2(36.0, ui.spacing().interact_size.y.max(22.0));
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    r.widget_info(|| egui::WidgetInfo::new(egui::Role::ColorWell));
    let r = r.on_hover_text(swatch.name);
    match mixed {
        true => dialogs::set_name(&r, &format!("{} (mixed)", swatch.name)),
        false => dialogs::set_name(&r, swatch.name),
    }
    if ui.is_rect_visible(rect) {
        let look = if open {
            &ui.visuals().widgets.open
        } else {
            ui.style().interact(&r)
        };
        let rect = rect.expand(look.expansion);
        let painter = ui.painter();
        painter.rect(
            rect,
            look.corner_radius,
            look.weak_bg_fill,
            look.bg_stroke,
            StrokeKind::Inside,
        );
        let ink = look.text_color();
        // The A and its bar (or the patch) on the left, the ▾ on the right.
        let left = Rect::from_min_max(rect.min, Pos2::new(rect.max.x - 11.0, rect.max.y));
        let bar = match swatch.letter {
            true => Rect::from_center_size(
                Pos2::new(left.center().x, rect.max.y - 5.0),
                egui::vec2(14.0, 3.0),
            ),
            false => Rect::from_center_size(left.center(), egui::vec2(14.0, 12.0)),
        };
        if swatch.letter {
            let font = egui::FontId::proportional((bar.top() - rect.top() - 2.0).min(13.0));
            let a = Pos2::new(left.center().x, bar.top() - 1.0);
            painter.text(a, egui::Align2::CENTER_BOTTOM, "A", font, ink);
        }
        let w = bar.width() / colors.len().max(1) as f32;
        for (i, &c) in colors.iter().enumerate() {
            let x = bar.min.x + w * i as f32;
            let band = Rect::from_min_max(Pos2::new(x, bar.min.y), Pos2::new(x + w, bar.max.y));
            painter.rect_filled(band, 0.0, rgb32(c));
        }
        let outline = Stroke::new(1.0, ink.gamma_multiply(0.6));
        painter.rect_stroke(bar.expand(1.0), 1.0, outline, StrokeKind::Inside);
        let tip = Pos2::new(rect.max.x - 7.0, rect.center().y + 2.0);
        let caret = vec![
            tip + egui::vec2(-3.0, -3.5),
            tip + egui::vec2(3.0, -3.5),
            tip,
        ];
        painter.add(egui::Shape::convex_polygon(caret, ink, Stroke::NONE));
    }
    // The colours there were when the picker opened, offered while it stays open.
    let offered = popup.with("offered");
    let hsva_id = popup.with("hsva");
    if !open {
        ui.data_mut(|d| {
            d.remove::<Vec<[f32; 3]>>(offered);
            d.remove::<egui::ecolor::Hsva>(hsva_id);
        });
    }
    let mut chosen = None;
    egui::Popup::menu(&r)
        .id(popup)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let offer = ui.data_mut(|d| d.get_temp_mut_or(offered, colors.to_vec()).clone());
            // The picker keeps its own hue and saturation where the colour has none
            // (black, greys), as egui's own colour button does.
            let [r, g, b] = to_linear(was);
            let start = egui::Rgba::from_rgb(r, g, b);
            let mut hsva = ui
                .data(|d| d.get_temp::<egui::ecolor::Hsva>(hsva_id))
                .filter(|h| same_color(from_linear(h.to_rgb()), was))
                .unwrap_or_else(|| egui::ecolor::Hsva::from(start));
            ui.spacing_mut().slider_width = PICKER_WIDTH;
            let picker = ui.vertical(|ui| color_picker_hsva_2d(ui, &mut hsva, Alpha::Opaque));
            ui.data_mut(|d| d.insert_temp(hsva_id, hsva));
            // egui's picker ends with its colour area and hue slider: a press there picks
            // the colour under the pointer, even the one the picker started from. A
            // change made only by converting colours back and forth is no pick.
            let s = ui.spacing();
            let slid = PICKER_WIDTH + s.item_spacing.y + s.interact_size.y;
            let r = picker.response.rect;
            let sliders = Rect::from_min_max(Pos2::new(r.left(), r.bottom() - slid), r.max);
            let pressed = ui.input(|i| {
                let at = i.pointer.press_origin();
                (i.pointer.any_down() || i.pointer.any_released())
                    && at.is_some_and(|p| sliders.contains(p))
            });
            let c = from_linear(hsva.to_rgb());
            if pressed || (picker.inner && !same_color(c, was)) {
                // A colour the text has, to the eye, is that colour exactly.
                let had = offer.iter().copied().find(|o| same_color(*o, c));
                chosen = Some(had.unwrap_or(c));
            }
            if offer.len() > 1 {
                ui.add_space(4.0);
                ui.label(swatch.offered);
                ui.horizontal(|ui| {
                    for (i, &c) in offer.iter().enumerate() {
                        let size = ui.spacing().interact_size;
                        let (rect, r) = ui.allocate_exact_size(size, Sense::click());
                        let (n, within) = (offer.len(), swatch.within);
                        let name = format!("Colour {} of {n} {within}", i + 1);
                        r.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::Button, true, &name)
                        });
                        dialogs::set_name(&r, &name);
                        let look = ui.style().interact(&r);
                        ui.painter().rect_filled(rect, 2.0, rgb32(c));
                        let border = (look.fg_stroke.width, look.bg_stroke.color);
                        ui.painter()
                            .rect_stroke(rect, 2.0, border, StrokeKind::Inside);
                        if r.on_hover_text(swatch.all).clicked() {
                            chosen = Some(c);
                            ui.close();
                        }
                    }
                });
            }
        });
    chosen
}

/// The width of the text colour picker's colour area and hue slider, as egui's own.
const PICKER_WIDTH: f32 = 275.0;

/// A colour, its parts from 0 to 1, as egui draws it.
fn rgb32(c: [f32; 3]) -> Color32 {
    let [r, g, b] = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    Color32::from_rgb(r, g, b)
}

/// Whether two colours are the same to the eye. The colour buttons store a colour back
/// slightly changed by converting it, so exact comparison would see a change never made.
pub fn same_color(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1.0 / 512.0)
}

/// Whether a change made in the inspector is finished: no list or colour picker is open,
/// no button is held and nothing is being typed. Changes are kept then, one undo step each.
fn settled(ctx: &egui::Context) -> bool {
    !egui::Popup::is_any_open(ctx)
        && !ctx.input(|i| i.pointer.any_down())
        && !ctx.egui_wants_keyboard_input()
}

/// Whether the pointer was pressed or let go this frame in the inspector (`insp`) or in the
/// band's groups that change the text being typed (`band`): a click there leaves the
/// typing open, to take the keyboard back once the change is made.
fn keeping(ctx: &egui::Context, insp: Option<Rect>, band: &[Rect]) -> bool {
    ctx.input(|i| {
        let clicked = i.pointer.any_pressed() || i.pointer.any_released();
        let at = i.pointer.press_origin().or(i.pointer.latest_pos());
        let inside =
            |p: Pos2| insp.is_some_and(|r| r.contains(p)) || band.iter().any(|r| r.contains(p));
        clicked && at.is_some_and(inside)
    })
}

/// Where the inspector or the floating bar is, from where and in which pass it was drawn
/// last ([`Editor::inspector_drawn`]): there if drawn this frame or the last (the pop-out
/// and the bar are drawn after the pages), else nowhere, so a click where a hidden one was
/// ends typing as any click away does.
fn drawn_at(ctx: &egui::Context, (rect, pass): (Rect, u64)) -> Rect {
    if ctx.cumulative_pass_nr() <= pass + 1 {
        rect
    } else {
        Rect::NOTHING
    }
}

/// The text box of the line being edited on `page`, in the tab's `scope`
/// ([`Editor::scope`]).
fn line_edit_id(scope: egui::Id, page: usize) -> egui::Id {
    scope.with(("line-edit", page))
}

/// A line of page text being edited in place: its page and object index. The text being
/// typed is the inspector's working copy.
struct LineEdit {
    page: usize,
    id: usize,
    /// What is selected when the editor opens.
    pick: Pick,
    /// Takes the keyboard on the next frame.
    fresh: bool,
    /// Takes the keyboard back once a change in the inspector is finished.
    refocus: bool,
    refused: page_text::Refused,
    /// Where it was drawn last on the screen, as typed: the floating bar keeps clear of it.
    area: Rect,
    /// What was typed or pasted before the line had the keyboard (with words selected with
    /// the read tool, which it types over), given to it once it has it, before the keys
    /// since.
    typed: Vec<egui::Event>,
    /// What an input method is composing in it, from and to ([`page_text::composition`]).
    composing: Option<(usize, usize)>,
}

/// Take what an input method was still composing in the line being edited, `le`, whose
/// text `insp` has, as committed ([`page_text::commit_left`]), as when it loses the
/// keyboard: in Helvetica for `hidden` (OCR) text, as that is written. Returns the first
/// character left out.
fn commit_line_composition(le: &mut LineEdit, insp: &mut Insp, hidden: bool) -> Option<String> {
    let r = le.composing.take()?;
    let font = match hidden {
        true => FontChoice::default(),
        false => insp.text.typing_font(r),
    };
    page_text::commit_left_styled(&mut insp.text, r, true, font)
}

/// What a line editor selects when it opens.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pick {
    /// These characters (from, to): the word double-clicked, found among the page's words.
    Chars(usize, usize),
    /// The character nearest this point (page points) as the line is drawn; with `word`,
    /// the word around it.
    At(Pos2, bool),
}

/// The change `text` makes to line `o`, as core is asked it: in parts, each in its style;
/// invisible (OCR) text, which has no styles, and an emptied line, in one.
fn text_change(text: &Styled, o: &PageObject) -> TextEdit {
    let flat = |s: &str| s.replace(['\n', '\r'], " ");
    if o.hidden || text.text().is_empty() {
        return TextEdit {
            text: flat(text.text()),
            font: None,
            size: None,
            color: None,
            parts: Vec::new(),
        };
    }
    let mut parts = text.parts();
    for p in &mut parts {
        p.text = flat(&p.text);
    }
    TextEdit::in_parts(parts)
}

/// Edit's Font group (ENH-38) for what a style command changes in `text`: the font list
/// and size (within `sizes`) in a row, then Bold, Italic, `planned` (the planned styles)
/// and the colour below them, or beside them when `compact`; each shows blank where that
/// mixes values. `in_pdf` names the PDF fonts of text already in the PDF. Returns whether
/// one was used.
fn styles_ui(
    ui: &mut egui::Ui,
    text: &mut dyn Restyle,
    sizes: std::ops::RangeInclusive<f32>,
    in_pdf: &[String],
    planned: &dyn Fn(&mut egui::Ui),
    compact: bool,
) -> bool {
    let shown = text.shown();
    let first = text.first();
    let was = FontChoice {
        bold: shown.bold,
        italic: shown.italic,
        ..shown.font.unwrap_or(first.font)
    };
    let mut font = was;
    let mut used = false;
    let mut picked = false;
    ui.with_layout(look_layout(compact), |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        ui.horizontal(|ui| {
            picked = font_field(ui, |ui| {
                crate::font_list::ui(ui, &mut font, shown.font.is_none(), in_pdf)
            });
            if let Some(size) = size_box(ui, shown.size, sizes) {
                text.set_size(size);
                used = true;
            }
        });
        ui.horizontal(|ui| {
            crate::font_list::styles(ui, &mut font);
            planned(ui);
            if let Some(c) = text_color_button(ui, &text.colors()) {
                text.set_color(c);
                used = true;
            }
        });
    });
    if picked {
        // Each character takes the family's face nearest its own.
        if shown.font.is_none() || font.family != was.family {
            text.set_family(font.family);
            used = true;
        }
    } else {
        if font.bold != was.bold {
            text.bold();
            used = true;
        }
        if font.italic != was.italic {
            text.italic();
            used = true;
        }
    }
    used
}

/// The Font group's two rows: one above the other, or side by side when `compact`.
fn look_layout(compact: bool) -> egui::Layout {
    if compact {
        egui::Layout::left_to_right(egui::Align::Center)
    } else {
        egui::Layout::top_down(egui::Align::Min)
    }
}

/// The font list, as wide as the Font group's first row leaves room for.
fn font_field(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
    let size = egui::vec2(136.0, ui.spacing().interact_size.y);
    ui.allocate_ui(size, add).inner
}

/// What the inspector says of the look of `text`: its font and size, "Mixed" where they
/// differ.
fn look_facts(text: &dyn Restyle) -> [(&'static str, String); 2] {
    let shown = text.shown();
    let font = shown.font.map_or("Mixed".into(), |f| f.family_name());
    let size = shown
        .size
        .map_or("Mixed".into(), |s| format!("{} pt", size_text(s)));
    [("Font", font), ("Size", size)]
}

/// The PDF's names for the fonts of the characters of line `o` a style command with `sel`
/// selected changes: those selected, or at a caret the one before it (the first at the
/// start), each once.
fn pdf_fonts(o: &PageObject, sel: (usize, usize)) -> Vec<String> {
    let (from, to) = (sel.0.min(sel.1), sel.0.max(sel.1));
    let (from, to) = if from == to {
        (from.saturating_sub(1), from.max(1))
    } else {
        (from, to)
    };
    let mut out: Vec<String> = Vec::new();
    for s in o
        .spans
        .iter()
        .filter(|s| s.chars.0 < to && s.chars.1 > from)
    {
        if !out.contains(&s.font_name) {
            out.push(s.font_name.clone());
        }
    }
    out
}

/// Line `o`'s text as `text` has it, in the pieces it is written in, each with the font it
/// is written in, as core decides it, `fonts` being the line's ([`PageObject::kept_pieces`]):
/// a part's own font where it is kept, else the installed family that font is from or the
/// nearest standard font, or the font chosen.
fn drawn_pieces<'a>(
    o: &'a PageObject,
    fonts: &'a [LineFont],
    text: &Styled,
) -> Vec<(TextPart, Wanted<'a>)> {
    let mut out = Vec::new();
    for part in text.drawn_parts() {
        let other = Wanted::Standard(text.font_of(&part));
        let PartFont::Span(i) = part.font else {
            out.push((part, other));
            continue;
        };
        let name = o.spans.get(i).map_or("", |s| s.font_name.as_str());
        let kept = Wanted::Kept(name, fonts.get(i));
        let font = |k: bool| if k { kept } else { other };
        if part.text.is_empty() {
            let keeps = o.keeps_font(&part, fonts);
            out.push((part, font(keeps)));
            continue;
        }
        let chars: Vec<char> = part.text.chars().collect();
        for piece in o.kept_pieces(&part, fonts) {
            let (a, b) = piece.chars;
            let sub = TextPart {
                text: chars[a..b].iter().collect(),
                from: None,
                origins: Vec::new(),
                ..part.clone()
            };
            out.push((sub, font(piece.kept)));
        }
    }
    out
}

/// Where the font line `o` is written in comes from, as `text` has it now, `fonts` being
/// the line's: the PDF's own kept ("Preserved"), else what replaces it, the installed
/// family it is from or the nearest standard font; "Partly preserved" when only some of it
/// keeps it. Parts in a font chosen in the band are written in that font, so they count
/// only when nothing else is left.
fn source_font(o: &PageObject, fonts: &[LineFont], text: &Styled) -> &'static str {
    let mut found: Vec<&str> = Vec::new();
    for (part, font) in drawn_pieces(o, fonts, text) {
        let source = match font {
            _ if !matches!(part.font, PartFont::Span(_)) => continue,
            Wanted::Kept(..) => "Preserved",
            Wanted::Standard(f) if !f.is_standard() => "Installed family",
            Wanted::Standard(_) => "Nearest standard",
        };
        if !found.contains(&source) {
            found.push(source);
        }
    }
    match found[..] {
        [] => "Font chosen",
        [one] => one,
        _ if found.contains(&"Preserved") => "Partly preserved",
        _ => "Mixed",
    }
}

/// What the inspector says about the font line `o` is written in, as `text` has it, with
/// `sel` selected, `fonts` being the line's, and why on hover: the characters typed that
/// its font has no code for, whose words go to another font; else, when the font of what
/// is selected can't take new text at all, that edits use another. `None` when there is
/// nothing to say.
fn font_note(
    o: &PageObject,
    fonts: &[LineFont],
    text: &Styled,
    sel: (usize, usize),
) -> Option<(String, String)> {
    let (mut missing, mut used, mut names) = (Vec::new(), None, Vec::new());
    for part in text.parts() {
        let PartFont::Span(i) = part.font else {
            continue;
        };
        for piece in o.kept_pieces(&part, fonts) {
            if piece.missing.is_empty() {
                continue;
            }
            used.get_or_insert(text.font_of(&part).family_name());
            let name = o.spans.get(i).map(|s| untagged(&s.font_name).to_string());
            names.extend(name.filter(|n| !names.contains(n)));
            for c in piece.missing {
                if !missing.contains(&c) {
                    missing.push(c);
                }
            }
        }
    }
    if let Some(used) = used {
        let chars: Vec<String> = missing.iter().map(char::to_string).collect();
        let chars = chars.join(" ");
        let note = format!("Not in this PDF's font: {chars}. Those words use {used}.");
        return Some((note, names.join(", ")));
    }
    // What is selected, or at a caret the character before it (the first at the start).
    let (from, to) = (sel.0.min(sel.1), sel.0.max(sel.1));
    let (from, to) = match from == to {
        true => (from.saturating_sub(1), from.max(1)),
        false => (from, to),
    };
    let (i, s) = (o.spans.iter().enumerate())
        .filter(|(_, s)| s.chars.0 < to && s.chars.1 > from)
        .find(|(i, _)| fonts.get(*i).is_some_and(|f| !f.takes_new_text()))?;
    let why = match fonts[i].refused()? {
        FontRefusal::Own => return None,
        FontRefusal::Type3 => "Type 3 font",
        FontRefusal::NotEmbedded => "Not embedded",
        FontRefusal::Vertical => "Vertical writing",
        FontRefusal::Cmap | FontRefusal::Unreadable => "Its characters can't be read",
    };
    let used = text.span_font(i).family_name();
    let note = format!("This PDF's font can't take new text. Edits use {used}.");
    Some((note, format!("{}: {why}", untagged(&s.font_name))))
}

/// A PDF font's name without a subset's tag: "ABCDEF+Calibri" is Calibri.
fn untagged(name: &str) -> &str {
    match name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => name,
    }
}

/// The font a line set in the PDF font `name` is shown as in the inspector, and written in
/// when it can't keep its own: the installed family it is from ([`FontChoice::of_font`]),
/// once the installed fonts have been read (at start, on another thread), else the nearest
/// standard font.
pub(crate) fn font_of(name: &str) -> FontChoice {
    limarepdf_core::typefaces_ready()
        .then(|| FontChoice::installed_font(name, &limarepdf_core::typefaces()))
        .flatten()
        .unwrap_or_else(|| FontChoice::from_name(name))
}

/// Whether line `o` is edited drawn as it will be written; turned lines and invisible
/// (OCR) ones are edited in a plain box.
fn drawn_in_place(o: &PageObject) -> bool {
    o.upright && !o.hidden && o.baseline.is_some()
}

/// The place between characters of line `o` nearest `p` (page points), for a line edited
/// in a plain box: measured along the line, whichever way it runs on the page, by where
/// its `n` characters are, or spread evenly along its box when that isn't known.
fn caret_along(o: &PageObject, p: Pos2, n: usize) -> usize {
    let b = o.bbox;
    let (dx, dy) = o.dir.unwrap_or((1.0, 0.0));
    let (x, y) = o.baseline.unwrap_or((b[0], b[3]));
    let along = |px: f32, py: f32| (px - x) * dx + (py - y) * dy;
    let at = along(p.x, p.y);
    if o.stops.len() == n + 1 {
        let off = |i: usize| (o.stops[i] - at).abs();
        return (0..=n)
            .min_by(|&i, &j| off(i).total_cmp(&off(j)))
            .unwrap_or(0);
    }
    let ends = [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])].map(|(x, y)| along(x, y));
    let from = ends.iter().copied().fold(f32::MAX, f32::min);
    let to = ends.iter().copied().fold(f32::MIN, f32::max);
    let f = ((at - from) / (to - from).max(0.01)).clamp(0.0, 1.0);
    (f * n as f32).round() as usize
}

/// What hides line `o` while it is edited: its box, with room for accents above and
/// descenders below, in page points.
fn cover_box(o: &PageObject) -> [f32; 4] {
    let b = o.bbox;
    let h = b[3] - b[1];
    [
        b[0] - 0.05 * h,
        b[1] - 0.12 * h,
        b[2] + 0.05 * h,
        b[3] + 0.06 * h,
    ]
}

/// The characters (from, to) of the word at character `i` of `s`; empty between words. At
/// the end of one word and the start of another, the one with letters or digits, else the
/// one starting there: after "Walrus" in "Walrus," it is "Walrus".
pub fn word_around(s: &str, i: usize) -> (usize, usize) {
    let i = i.min(s.chars().count());
    let near: Vec<((usize, usize), bool)> = (unicode_pieces(s).into_iter())
        .filter(|&((a, b), word)| word.is_some() && a <= i && i <= b && a < b)
        .map(|(r, word)| (r, word == Some(true)))
        .collect();
    // With letters first, then the one starting at `i`.
    let rank = |((a, _), letters): &&((usize, usize), bool)| (!*letters, *a != i);
    near.iter().min_by_key(rank).map_or((i, i), |(r, _)| *r)
}

/// The characters (from, to) of each word of `s`, in order.
fn word_spans(s: &str) -> Vec<(usize, usize)> {
    (unicode_pieces(s).into_iter())
        .filter(|(_, word)| word.is_some())
        .map(|(r, _)| r)
        .collect()
}

/// `s` in its pieces by Unicode's word rule (UAX #29, as Ctrl+arrows go in the typing box):
/// each piece's characters (from, to), and whether it is a word, `Some(true)` when it has
/// letters or digits; spaces aren't words. Each Chinese or Japanese ideograph is a word, as
/// is each punctuation mark.
fn unicode_pieces(s: &str) -> Vec<((usize, usize), Option<bool>)> {
    let mut at = 0;
    (s.split_word_bounds())
        .map(|piece| {
            let n = piece.chars().count();
            let word = match piece.chars().all(char::is_whitespace) {
                true => None,
                false => Some(piece.chars().any(char::is_alphanumeric)),
            };
            at += n;
            ((at - n, at), word)
        })
        .collect()
}

/// A search match on a page: the number of its hit, and its boxes there (displayed-page
/// points).
pub type Match = (usize, Vec<[f32; 4]>);

pub struct Editor {
    pub tool: Tool,
    /// Pages, an object or words; see [`Selection`].
    pub selection: Selection,
    objs: HashMap<usize, Vec<PageObject>>,
    unavailable: HashMap<usize, String>,
    /// The fonts of each line of a page, by object id, read together the first time one of
    /// its lines is selected ([`Editor::read_line_fonts`]), kept as `objs` is.
    line_fonts: HashMap<usize, Vec<Option<Vec<LineFont>>>>,
    drag: Option<Drag>,
    draw: Option<Draw>,
    /// A rectangle being dragged out with the arrow, selecting what is wholly inside it.
    marquee: Option<Marquee>,
    /// A press held still on a page (its page point): egui takes one held a while for a
    /// drag, but the arrow and the read tool start one only once the pointer moves further
    /// than a click can, and one let go before that is a click.
    still: Option<(usize, Pos2)>,
    /// New text being typed on the page (Add text).
    pub adding: Option<PageText>,
    /// The note about a character a box in the inspector refused.
    text_refused: page_text::Refused,
    /// The note at the end of the words selected with the read tool when what was typed
    /// couldn't go over them ([`Self::type_over`]).
    words_note: page_text::Refused,
    /// Where the inspector was drawn last, and in which pass: a click there doesn't end a
    /// line being edited ([`drawn_at`]).
    insp_rect: (Rect, u64),
    /// Where the floating bar was drawn last, and in which pass: a click there doesn't end
    /// a line being edited either.
    pub bar_rect: (Rect, u64),
    /// The box of the text being typed on the page (a line, new text, a note or a text box)
    /// as drawn this frame, for its right-click menu; the tab takes it each frame.
    pub typing_box: Option<egui::Response>,
    /// Where the band's groups that change the text being typed (its look, Copy and Paste)
    /// were drawn this frame: a click there doesn't end the typing ([`band::show`]).
    ///
    /// [`band::show`]: crate::band::show
    pub keep_typing: Vec<Rect>,
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
    /// Several objects selected, as the inspector changes them ([`Editor::sync_group`]).
    group: Option<Group>,
    /// The words selected with the read tool, or lines among several objects selected
    /// with the arrow, as the inspector and Bold and Italic restyle them
    /// ([`Editor::sync_chosen`]).
    chosen: Option<Chosen>,
    /// A line of page text being edited on the page (read tool).
    line_edit: Option<LineEdit>,
    /// The placing tools kept active after putting something down, each pinned on its own
    /// (from the settings).
    pub keep: Vec<Tool>,
    pub actions: Vec<Action>,
    /// What [`Editor::keep_later`] took, by its change's place in [`Editor::actions`]: put
    /// back to go on typing if the change fails ([`Editor::settle_kept`]).
    later: Vec<(usize, Held)>,
    /// Typing put back after its change failed, as it was then: ended again unchanged, it
    /// isn't tried again but left out.
    not_kept: Option<NotKept>,
    /// Typing was left out: its change failed, or the document changed before it could be
    /// put back. The tab says so.
    pub left_out: bool,
    /// What an input method was still composing when typing lost the keyboard was taken as
    /// committed, and this, the first character that can't be written, left out of it
    /// (PB-54). The tab says so ([`page_text::left_note`]).
    pub left_composing: Option<String>,
    /// The document's language (`/Lang`), as the pages were drawn last: text typed on the
    /// page is drawn in the fonts core sets it in for it ([`page_text::runs`]).
    lang: Option<String>,
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
    /// What had the keyboard on the page when F6 took it elsewhere, for when it comes back.
    pub parked: Option<Parked>,
    /// The pass in which a click outside a note closed it: that click puts down nothing
    /// else, whichever page it was on.
    note_closed: Option<u64>,
    pub note_color: [f32; 3],
    /// How new text boxes are set.
    pub box_style: TextStyle,
    /// The name new annotations are signed with (from the settings).
    pub author: String,
    /// The features planned but not built are shown (from the settings; ENH-41).
    pub planned: bool,
    /// How many areas are marked for redaction (counted by the tab).
    pub redact_marks: usize,
    /// The Redact tool's Apply button was pressed; the tab runs the command.
    pub apply_redactions: bool,
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
    /// The replies typed and not yet added, by the comment they answer: the inspector's
    /// reply box shows the selected comment's.
    drafts: BTreeMap<(u32, u16), Draft>,
    /// What the selected comment's page is called (its label), lent by the app.
    pub annot_page: Option<String>,
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
    /// Saved signatures and the ink and date choices (from the settings).
    pub sign_kit: SignKit,
    /// The Sign tool's pane.
    pub sign: SignPane,
    /// Fill-and-sign items still movable in the document, for the Sign tool's pane.
    pub sign_items: usize,
    /// The pane's Finish signing button was pressed.
    pub finish_signing: bool,
    /// Why the Sign tool couldn't put something down.
    sign_error: Option<String>,
    /// The digital signature tool places a certification rather than a signature.
    pub digital_certify: bool,
    /// The empty signature fields (full name, page), listed in the digital signature
    /// tool's pane; kept current by the app while the tool is on.
    pub digital_fields: Vec<(String, usize)>,
    /// Where the digital signature goes, once chosen: taken by the app, which opens the
    /// signing window.
    pub digital_place: Option<SignPlace>,
    /// Why the digital signature tool couldn't take a place.
    digital_error: Option<String>,
    /// How pages are coloured on screen (set by the tab), which text being typed matches.
    pub page_colours: PageColours,
    /// The colour around the line being edited (found by the tab), which hides the line
    /// under the editor; `None` until known.
    pub cover: Option<Color32>,
    /// What the ids of the tab's text boxes are made from, so that no two documents share
    /// one, nor their typing.
    scope: egui::Id,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            tool: Tool::Read,
            selection: Selection::None,
            objs: HashMap::new(),
            unavailable: HashMap::new(),
            line_fonts: HashMap::new(),
            drag: None,
            draw: None,
            marquee: None,
            still: None,
            adding: None,
            text_refused: page_text::Refused::default(),
            words_note: page_text::Refused::default(),
            insp_rect: (Rect::NOTHING, 0),
            bar_rect: (Rect::NOTHING, 0),
            typing_box: None,
            keep_typing: Vec::new(),
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
            group: None,
            chosen: None,
            line_edit: None,
            keep: Vec::new(),
            actions: Vec::new(),
            later: Vec::new(),
            not_kept: None,
            left_out: false,
            left_composing: None,
            lang: None,
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
            parked: None,
            note_closed: None,
            note_color: [1.0, 0.85, 0.2],
            box_style: TextStyle {
                border: 1.0,
                ..TextStyle::default()
            },
            author: String::new(),
            planned: false,
            redact_marks: 0,
            apply_redactions: false,
            zoom: 1.0,
            draw_permanent: false,
            arrow: false,
            pen_color: [0.1, 0.25, 0.8],
            pen_width: 2.0,
            pen_stroke: None,
            ink: None,
            ink_at: f64::NEG_INFINITY,
            drafts: BTreeMap::new(),
            annot_page: None,
            stamp: StampPick::Text(StampKind::Approved),
            stamp_name_date: true,
            stamp_pictures: Vec::new(),
            stamp_wordings: Vec::new(),
            stamp_draft: String::new(),
            stamp_draft_color: StampColor::Red,
            reply_focus: false,
            sign_kit: SignKit::from_file(None),
            sign: SignPane::default(),
            sign_items: 0,
            finish_signing: false,
            sign_error: None,
            digital_certify: false,
            digital_fields: Vec::new(),
            digital_place: None,
            digital_error: None,
            page_colours: PageColours::Normal,
            cover: None,
            scope: egui::Id::unique("editor"),
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
    let mut tip = a.label();
    for extra in [&a.author, &a.modified] {
        let extra = limarepdf_core::display_line(extra, 80);
        if !extra.is_empty() {
            tip += &format!(" · {extra}");
        }
    }
    if a.hidden() {
        tip += " · hidden";
    }
    let contents = limarepdf_core::display_text(&a.contents, 2000);
    if !contents.is_empty() {
        tip += &format!("\n\n{contents}");
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

/// The word of word `j`'s line among `words` whose middle is nearest `x` across (page
/// points): where Shift+Down or Shift+Up takes a selection's end.
fn across(words: &[PageWord], j: usize, x: f32) -> usize {
    let middle = |w: &PageWord| ((w.bbox[0] + w.bbox[2]) / 2.0 - x).abs();
    let line = words[j].line;
    (0..words.len())
        .filter(|&k| words[k].line == line)
        .min_by(|&a, &b| middle(&words[a]).total_cmp(&middle(&words[b])))
        .unwrap_or(j)
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
    /// The editor of tab `tab` (its [`crate::tab::Tab::id`]).
    pub fn for_tab(tab: u64) -> Self {
        let scope = egui::Id::unique(("tab", tab));
        let mut ed = Self {
            scope,
            ..Self::default()
        };
        ed.form.scope = scope;
        ed
    }

    /// What the ids of the tab's text boxes are made from: each is this with what the box
    /// edits.
    pub fn scope(&self) -> egui::Id {
        self.scope
    }

    /// The text box of the note or text box being typed.
    pub fn writing_id(&self) -> Option<egui::Id> {
        let w = self.writing.as_ref()?;
        let kind = if w.note { "note-text" } else { "box-text" };
        Some(self.scope.with((kind, w.page, w.id)))
    }

    /// The text box of the typing open on the page: new text, a line being edited, or a
    /// note or text box.
    pub fn typing_id(&self) -> Option<egui::Id> {
        if let Some(a) = &self.adding {
            return Some(a.id());
        }
        if let Some(le) = &self.line_edit {
            return Some(line_edit_id(self.scope, le.page));
        }
        self.writing_id()
    }

    /// Whether `id` is the box of a new note or text box being typed, which every new one
    /// on its page shares.
    pub fn new_box(&self, id: egui::Id) -> bool {
        self.writing.as_ref().is_some_and(|w| w.id.is_none()) && self.writing_id() == Some(id)
    }

    /// What Ctrl+Z and Ctrl+Y can take back and redo in the typing on the page whose box
    /// `id` has the keyboard: new text or a line being edited, which keep their own
    /// history. `None` for any other box.
    pub fn typing_history(&self, id: egui::Id) -> Option<(bool, bool)> {
        let history = match (&self.adding, &self.line_edit) {
            (Some(a), _) if a.id() == id => a.history(),
            (_, Some(le)) if line_edit_id(self.scope, le.page) == id => {
                &self.insp.as_ref()?.history
            }
            _ => return None,
        };
        Some((history.can_undo(), history.can_redo()))
    }

    /// The text being typed takes the keyboard back once the pointer is let go: Copy or
    /// Paste in the band, clicked while typing.
    pub fn refocus_typing(&mut self) {
        if let Some(a) = self.adding.as_mut() {
            a.refocus = true;
        }
        if let Some(le) = self.line_edit.as_mut() {
            le.refocus = true;
        }
        if let Some(w) = self.writing.as_mut() {
            w.refocus = true;
        }
    }

    /// Forget cached object lists (after any change to the document).
    pub fn invalidate(&mut self) {
        self.objs.clear();
        self.unavailable.clear();
        self.line_fonts.clear();
        self.drag = None;
        self.draw = None;
        self.marquee = None;
        self.insp = None;
        self.group = None;
        self.chosen = None;
        self.line_edit = None;
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
    pub fn annots(&mut self, pdf: &limarepdf_core::Pdf, page: usize) -> &[Annot] {
        self.annots
            .entry(page)
            .or_insert_with(|| pdf.page_annotations(page).unwrap_or_default())
    }

    /// The topmost shown annotation under `p` (page points): over one of its lines for
    /// markup, near its strokes for lines and drawings, else inside its box.
    fn annot_at(&mut self, pdf: &limarepdf_core::Pdf, page: usize, p: Pos2) -> Option<&Annot> {
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
            border: a.text_style.map_or(0.0, |s| s.border),
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
        self.keep_later(Typed::Writing);
    }

    /// Keep the new text being typed, if any: added as one undo step.
    pub fn finish_adding(&mut self) {
        self.keep_later(Typed::Text);
    }

    /// Queue the change that keeps `piece` and stop typing it, as clicking away does.
    /// Returns what was being typed, for [`Self::put_back`] if the change fails or
    /// [`Self::done`] once it is made; `None` when there was nothing to keep (a new note
    /// or text left empty goes).
    pub fn keep(&mut self, piece: Typed) -> Option<Held> {
        // What an input method was still composing is taken as committed (PB-54).
        let left = match piece {
            Typed::Text => self.adding.as_mut().and_then(PageText::commit_composition),
            Typed::Writing => self.writing.as_mut().and_then(Writing::commit_composition),
            Typed::Object => {
                let key = self.insp.as_ref().map(|i| i.key);
                let o = key.and_then(|(p, id)| self.objs.get(&p)?.get(id));
                let hidden = o.is_some_and(|o| o.hidden);
                match (self.line_edit.as_mut(), self.insp.as_mut()) {
                    (Some(le), Some(insp)) => commit_line_composition(le, insp, hidden),
                    _ => None,
                }
            }
            _ => None,
        };
        if self.left_composing.is_none() {
            self.left_composing = left;
        }
        let (action, held) = match piece {
            Typed::Text => {
                let a = self.adding.take().filter(PageText::has_text)?;
                let add = Action::AddText(a.page, a.at, a.text.clone());
                (add, Kept::Text(Box::new(a)))
            }
            Typed::Writing => {
                let w = self.writing.take().filter(Writing::changed)?;
                let (r, text) = (w.rect, w.text.clone());
                let a = match w.id {
                    Some(id) => Action::AnnotText(w.page, id, text),
                    None if w.note => {
                        Action::AddNote(w.page, ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0), text)
                    }
                    None => Action::AddTextBox(w.page, r, text),
                };
                (a, Kept::Writing(w))
            }
            Typed::Object => {
                let a = self.insp_change()?;
                let held = (self.insp.take()?, self.line_edit.take());
                (a, Kept::Object(Box::new(held)))
            }
            Typed::Field => {
                let mut acts = Vec::new();
                let original = self.form.commit_typing(&mut acts)?;
                (acts.pop()?, Kept::Field(original))
            }
            Typed::Comment => (self.annot_changes()?, Kept::Comment),
            // One draft at a time: one that can't be added is put back alone.
            Typed::Reply => {
                let id = *self.drafts.iter().find(|(_, d)| d.ready())?.0;
                let d = self.drafts.remove(&id)?;
                let a = Action::Reply(d.page?, id, d.text.clone());
                (a, Kept::Reply(id, d))
            }
            Typed::Words => {
                let c = self.chosen.take_if(|c| c.changed())?;
                let restyle = Action::Restyle(c.changes(), c.left);
                (restyle, Kept::Words(Box::new(c)))
            }
            Typed::Objects => {
                let g = self.group.take_if(|g| g.change().is_some())?;
                (g.change()?, Kept::Objects(Box::new(g)))
            }
        };
        self.actions.push(action);
        Some(Held(held))
    }

    /// Go on typing what [`Self::keep`] took, its change not made.
    pub fn put_back(&mut self, held: Held) {
        match held.0 {
            Kept::Text(a) => self.adding = Some(*a),
            Kept::Writing(w) => self.writing = Some(w),
            Kept::Object(o) => {
                let (insp, line) = *o;
                // Selected again, should a click have selected something else meanwhile.
                if self.sel() != Some(insp.key) {
                    self.select_object(insp.key.0, insp.key.1);
                }
                (self.insp, self.line_edit) = (Some(insp), line);
            }
            Kept::Field(original) => self.form.uncommit(original),
            Kept::Comment => {}
            Kept::Reply(id, d) => {
                self.drafts.insert(id, d);
            }
            Kept::Words(c) => self.chosen = Some(*c),
            Kept::Objects(g) => self.group = Some(*g),
        }
    }

    /// What [`Self::keep`] took is in the document now.
    pub fn done(&mut self, held: Held) {
        self.carry_on(&held);
    }

    /// The next new text carries on in the style this one ended in, as typing would.
    fn carry_on(&mut self, held: &Held) {
        if let Kept::Text(a) = &held.0 {
            let end = page_text::Look::of(a.text.end_style());
            (self.tfont, self.tsize, self.tcolor) = (end.font, end.size, end.color);
        }
    }

    /// [`Self::keep`], as clicking away keeps typing: what it took is held until its change
    /// is made ([`Self::settle_kept`]). Typing put back after its change failed, ended again
    /// unchanged, isn't tried again: it is left out, and the tab says so.
    fn keep_later(&mut self, piece: Typed) {
        if self.refused_again(piece) {
            match piece {
                Typed::Text => self.adding = None,
                Typed::Writing => self.writing = None,
                _ => (self.insp, self.line_edit) = (None, None),
            }
            self.not_kept = None;
            self.left_out = true;
            return;
        }
        if let Some(held) = self.keep(piece) {
            // New text started next, in this frame, carries on in this one's style.
            self.carry_on(&held);
            self.not_kept = None;
            self.later.push((self.actions.len() - 1, held));
        }
    }

    /// Whether `piece` is typing put back after its change failed, unchanged since.
    fn refused_again(&self, piece: Typed) -> bool {
        match (&self.not_kept, piece) {
            (Some(NotKept::Text(t)), Typed::Text) => {
                self.adding.as_ref().is_some_and(|a| a.text.same_as(t))
            }
            (Some(NotKept::Writing(t)), Typed::Writing) => {
                self.writing.as_ref().is_some_and(|w| w.text == *t)
            }
            (Some(NotKept::Object(key, t, look)), Typed::Object) => {
                let same = |i: &Insp| i.key == *key && i.text.same_as(t) && i.look().same_as(look);
                self.insp.as_ref().is_some_and(same)
            }
            _ => false,
        }
    }

    /// After the tab made the changes [`Self::keep_later`] queued: typing whose change was
    /// made is done with; typing whose change failed (`failed`: the change's place in the
    /// actions as queued, and whether a change after it was made) goes back to be typed on,
    /// given the keyboard, unless the document changed since, when its place may be
    /// another's: then it is left out.
    pub fn settle_kept(&mut self, failed: &[(usize, bool)]) {
        for (at, held) in std::mem::take(&mut self.later) {
            match failed.iter().find(|(f, _)| *f == at) {
                None => self.done(held),
                Some((_, false)) => {
                    self.not_kept = NotKept::of(&held);
                    self.put_back(held);
                    self.refocus_typing();
                }
                Some((_, true)) => self.left_out = true,
            }
        }
    }

    /// How new text is set, from the inspector.
    pub fn new_look(&self) -> page_text::Look {
        page_text::Look {
            font: self.tfont,
            size: self.tsize,
            color: self.tcolor,
        }
    }

    /// Whether something typed isn't in the document yet: new text, a note or text box
    /// being typed on the page, a form field being typed in, the selected comment's
    /// changes in the inspector, a draft reply to a comment, or the selected words' or
    /// objects' look changed in the inspector. Saving keeps it first, and closing asks, as
    /// for unsaved changes.
    pub fn pending(&self) -> bool {
        self.adding.as_ref().is_some_and(PageText::has_text)
            || self.writing.as_ref().is_some_and(Writing::changed)
            || self.form.typing()
            || self.annot_changes().is_some()
            || self.drafts.values().any(Draft::ready)
            || self.insp_change().is_some()
            || self.chosen.as_ref().is_some_and(Chosen::changed)
            || self.group.as_ref().is_some_and(|g| g.change().is_some())
    }

    /// The selected comment's changes in the inspector, as the change that applies them.
    fn annot_changes(&self) -> Option<Action> {
        let (id, e) = self.annot_insp.as_ref()?;
        let a = self
            .selected_annot()
            .filter(|a| a.id == *id && !a.locked())?;
        (!e.same_as(&AnnotEdit::of(a))).then(|| Action::SetAnnot(a.page, a.id, e.clone()))
    }

    /// The selected object's changes in the inspector, or typed over its line on the page,
    /// that aren't in the document yet, as the change that makes them.
    fn insp_change(&self) -> Option<Action> {
        let insp = self.insp.as_ref()?;
        let (page, id) = insp.key;
        let obj = self.objs.get(&page)?.get(id)?;
        match obj.kind {
            ObjectKind::Text => {
                let te = text_change(&insp.text, obj);
                let words_only = !insp.text.restyled();
                (!insp.text.same_as(&insp.orig))
                    .then_some(Action::EditText(page, id, te, words_only))
            }
            ObjectKind::Path => {
                let was = Insp::of(insp.key, obj);
                let same = (insp.fill_on, insp.stroke_on) == (was.fill_on, was.stroke_on)
                    && same_color(insp.fill, was.fill)
                    && same_color(insp.stroke, was.stroke)
                    && (insp.width - was.width).abs() < 0.005;
                let st = PathStyle {
                    fill: insp.fill_on.then_some(insp.fill),
                    stroke: insp.stroke_on.then_some(insp.stroke),
                    line_width: insp.width,
                };
                (!same).then_some(Action::PathStyle(page, vec![(id, st)]))
            }
            _ => None,
        }
    }

    /// Keep the selected object's changes (see [`Self::insp_change`]) as one undo step. The
    /// working copy starts again from the object.
    fn commit_insp(&mut self) {
        self.keep_later(Typed::Object);
    }

    /// Stop editing the line on the page, keeping what was typed. With the arrow, or once
    /// made movable (Move or resize text), the line stays selected; with another tool it
    /// isn't left selected, as a word processor leaves no selection behind, so the inspector
    /// goes back to what it showed before.
    fn end_line_edit(&mut self) {
        let Some(line) = self.line_edit.as_ref().map(|le| (le.page, le.id)) else {
            return;
        };
        // Kept with what was typed, to come back with it if the change fails.
        self.commit_insp();
        self.line_edit = None;
        let line = Some(line);
        if self.tool != Tool::Select && !self.selection.loose() && self.sel() == line {
            self.selection.clear_object();
        }
    }

    /// Edit a line of page text on the page, with `pick` selected.
    fn start_line_edit(&mut self, page: usize, id: usize, pick: Pick) {
        // One of several selected: it alone, as a word processor edits one at a time. A
        // line already selected alone stays as it is (movable with the read tool, say).
        if self.sel() != Some((page, id)) {
            self.select_object(page, id);
        }
        self.sync_insp();
        self.cover = None;
        self.line_edit = Some(LineEdit {
            page,
            id,
            pick,
            fresh: true,
            refocus: false,
            refused: page_text::Refused::default(),
            area: Rect::NOTHING,
            typed: Vec::new(),
            composing: None,
        });
    }

    /// The line of page text to edit for the words selected with the read tool, and what of
    /// it to select: the line where the selection's moving end is (else the last that can
    /// change), its selected characters selected; and whether all the words are in it.
    /// `None` when none of their lines can change (invisible text, a group, vertical
    /// writing).
    fn words_line(&mut self, pdf: &limarepdf_core::Pdf) -> Option<(usize, usize, Pick, bool)> {
        let Selection::Words { end, .. } = self.selection else {
            return None;
        };
        self.sync_chosen(pdf);
        let c = self.chosen.as_ref()?;
        let at = (self.words.get(&end.0))
            .and_then(|w| w.get(end.1))
            .and_then(|w| w.object);
        let (page, id) = (at.map(|id| (end.0, id)))
            .filter(|&(p, id)| c.selected_in(p, id).is_some())
            .or_else(|| c.line_ids().last())?;
        let (a, b) = c.selected_in(page, id)?;
        Some((page, id, Pick::Chars(a, b), c.one_line()))
    }

    /// Edit the line of the words selected with the read tool on the page, with them
    /// selected there (Edit text; across lines, the line where the selection ends, its part
    /// selected). Whether there was such a line.
    pub fn edit_words(&mut self, pdf: &limarepdf_core::Pdf) -> bool {
        let Some((page, id, pick, _)) = self.words_line(pdf) else {
            return false;
        };
        self.start_line_edit(page, id, pick);
        true
    }

    /// Text `typed` or pasted (its events, an input method's among them) with no text box
    /// having the keyboard: over the words selected with the read tool in one line, as a
    /// word processor types over a selection, their line opens on the page and it goes in
    /// their place, an input method's first composition too (the input method is on while
    /// words are selected: [`Self::input_method_at_words`]); into a line just opened, before
    /// it has the keyboard, it goes there. Across lines nothing is typed, and a note at the
    /// selection's end says why. Whether it was taken.
    pub fn type_over(
        &mut self,
        ctx: &egui::Context,
        pdf: &limarepdf_core::Pdf,
        typed: Vec<egui::Event>,
    ) -> bool {
        if let Some(le) = self.line_edit.as_mut().filter(|le| le.fresh) {
            le.typed.extend(typed);
            return true;
        }
        // An input method's empty composition (one ended, or the method turned on) types
        // nothing.
        let something = typed.iter().any(|e| match e {
            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. })
            | egui::Event::Ime(egui::ImeEvent::Commit(text)) => !text.is_empty(),
            _ => true,
        });
        let words =
            self.selection.words().is_some() && matches!(self.tool, Tool::Read | Tool::Text);
        if !words || !something {
            return false;
        }
        match self.words_line(pdf) {
            Some((page, id, pick, true)) => {
                self.start_line_edit(page, id, pick);
                if let Some(le) = self.line_edit.as_mut() {
                    le.typed = typed;
                }
            }
            Some((.., false)) => {
                let note = "Type over words within one line. Paragraph editing is planned.";
                self.words_note.mark(ctx, note.into());
            }
            None => {
                if let Some(said) = self.chosen.as_ref().and_then(|c| c.left.said()) {
                    self.words_note.mark(ctx, said);
                }
            }
        }
        true
    }

    /// What to select of line `id`'s text when it is double-clicked at `p`: the word
    /// there, found among the page's `words` on the line, or around where `p` falls on the
    /// line when they don't match its text.
    fn word_pick(&self, page: usize, id: usize, p: Pos2, words: &[PageWord]) -> Pick {
        let Some(o) = self.objs.get(&page).and_then(|v| v.get(id)) else {
            return Pick::Chars(0, 0);
        };
        let text = o.text.as_deref().unwrap_or("");
        let b = o.bbox;
        let mut on: Vec<&PageWord> = words
            .iter()
            .filter(|w| {
                let (x, y) = ((w.bbox[0] + w.bbox[2]) / 2.0, (w.bbox[1] + w.bbox[3]) / 2.0);
                x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3]
            })
            .collect();
        on.sort_by(|v, w| v.bbox[0].total_cmp(&w.bbox[0]));
        let spans = word_spans(text);
        if !o.upright {
            // A turned line's words don't run left to right on the page: in the order they
            // run along the line, the word under `p`.
            let (dx, dy) = o.dir.unwrap_or((1.0, 0.0));
            let along = |w: &PageWord| {
                let b = w.bbox;
                (b[0] + b[2]) / 2.0 * dx + (b[1] + b[3]) / 2.0 * dy
            };
            on.sort_by(|v, w| along(v).total_cmp(&along(w)));
            let inside = |w: &&PageWord| {
                let b = w.bbox;
                p.x >= b[0] && p.x <= b[2] && p.y >= b[1] && p.y <= b[3]
            };
            let under = on.iter().position(inside);
            return match under.filter(|_| on.len() == spans.len()) {
                Some(j) => Pick::Chars(spans[j].0, spans[j].1),
                None => Pick::At(p, true),
            };
        }
        let under = on.iter().position(|w| p.x >= w.bbox[0] && p.x <= w.bbox[2]);
        match under.filter(|_| on.len() == spans.len()) {
            Some(j) => Pick::Chars(spans[j].0, spans[j].1),
            None => Pick::At(p, true),
        }
    }

    /// Whether a line of page text is being edited, or new text, a note or a text box
    /// typed, on the page.
    pub fn typing_on_page(&self) -> bool {
        self.line_edit.is_some() || self.adding.is_some() || self.writing.is_some()
    }

    /// The line being edited drawn in place, as its page and the box to hide (page
    /// points): the tab finds the colour around it for [`Self::cover`].
    pub fn cover_wanted(&self) -> Option<(usize, [f32; 4])> {
        let le = self.line_edit.as_ref()?;
        let o = self.objs.get(&le.page)?.get(le.id)?;
        drawn_in_place(o).then(|| (le.page, cover_box(o)))
    }

    /// The new text being typed on `page` (drawn at `r`, `k` pixels per point). Esc, Tab
    /// or scrolling it out of view keeps it. Returns true when the page should not take
    /// the pointer this frame: it is over the text, or the editor just opened or closed.
    fn adding_ui(&mut self, ui: &mut egui::Ui, page: usize, r: Rect, k: f32) -> bool {
        let accent = ui.visuals().selection.stroke.color;
        let settled = settled(ui.ctx());
        let Some(a) = self.adding.as_mut().filter(|a| a.page == page) else {
            return false;
        };
        let lang = self.lang.as_deref();
        let shown = a.ui(ui, r, k, self.page_colours, accent, settled, lang);
        self.typing_box = Some(shown.edit);
        if self.left_composing.is_none() {
            self.left_composing = shown.left;
        }
        if shown.done {
            // Under a dialog (a change's failure said, say) it waits, open, for it to close.
            if crate::dialogs::any_open(ui.ctx()) {
                a.refocus = true;
            } else {
                self.finish_adding();
            }
        }
        shown.over
    }

    /// The line of page text being edited on `page` (drawn at `r`, `k` pixels per point):
    /// drawn as it will be written, in the font core will write it in, over the line,
    /// which is hidden underneath; a turned or invisible line in a plain box. Esc, Enter,
    /// Tab or clicking away keeps the change. Returns true when the page should not take
    /// the pointer this frame: it is over the editor, or the editor just opened or closed.
    fn line_edit_ui(&mut self, ui: &mut egui::Ui, page: usize, r: Rect, k: f32) -> bool {
        if self.line_edit.as_ref().is_none_or(|l| l.page != page) {
            return false;
        }
        self.sync_insp();
        let Some(key) = self.line_edit.as_ref().map(|l| (l.page, l.id)) else {
            return false;
        };
        let o = self.objs.get(&page).and_then(|v| v.get(key.1)).cloned();
        let ok = self.sel() == Some(key) && self.insp.as_ref().is_some_and(|i| i.key == key);
        let (Some(o), true) = (o, ok) else {
            self.line_edit = None;
            return false;
        };
        let to_screen = |x: f32, y: f32| Pos2::new(r.min.x + x * k, r.min.y + y * k);
        let on_screen =
            |b: [f32; 4]| Rect::from_min_max(to_screen(b[0], b[1]), to_screen(b[2], b[3]));
        // The other lines of the page shown, which typing may run into.
        let others: Vec<Rect> = (self.objs.get(&page).into_iter().flatten().enumerate())
            .filter(|(i, l)| *i != key.1 && l.kind == ObjectKind::Text && !l.hidden)
            .map(|(_, l)| on_screen(l.bbox))
            .collect();
        let line = on_screen(o.bbox);
        // Past the page's edge, or typed on past the line's end into another line: the part
        // beyond its end, across the middle of its height, meets another line's box.
        let warn = |text: Rect| {
            page_text::past_edge(r, text).or_else(|| {
                let (mid, half) = (line.center().y, line.height() * 0.25);
                let x = (line.max.x + k).max(text.min.x)..=text.max.x;
                let beyond = Rect::from_x_y_ranges(x, mid - half..=mid + half);
                let over = beyond.is_positive() && others.iter().any(|l| l.intersects(beyond));
                over.then_some("Over other text")
            })
        };
        let (Some(le), Some(insp)) = (self.line_edit.as_mut(), self.insp.as_mut()) else {
            return false;
        };
        let ctx = ui.ctx().clone();
        let id = line_edit_id(self.scope, page);
        let drawn = drawn_in_place(&o);
        if ctx.memory(|m| m.has_focus(id)) && !le.typed.is_empty() {
            // Typed as it opened, over what it selected: before the keys since, as typed.
            let typed = std::mem::take(&mut le.typed);
            ctx.input_mut(|i| {
                i.events.splice(0..0, typed);
            });
        }
        if ctx.memory(|m| m.has_focus(id)) {
            // Before the editor sees the keys: what can't be written in the font it goes
            // into isn't typed, and Ctrl+B and Ctrl+I switch what is selected, as the
            // inspector's buttons do. Hidden (OCR) text is written in Helvetica.
            let sel = selection(&ctx, id).unwrap_or((0, 0));
            let font = if o.hidden {
                FontChoice::default()
            } else {
                insp.text.typing_font(sel)
            };
            let t = page_text::take_typing(ui, true, !o.hidden, font);
            page_text::undo_redo(&ctx, id, &t, &mut insp.history, &mut insp.text);
            page_text::switch_styles(&ctx, &t, &mut insp.text, sel, font, &mut le.refused);
            if let Some(c) = t.refused {
                le.refused.mark(&ctx, page_text::note(&c).to_string());
            }
        }
        let b = o.bbox;
        let mut typed = insp.text.text().to_string();
        let mode = self.page_colours;
        // The text as egui has it this frame, in parts at `k` pixels per point, coloured
        // by `color`.
        let fonts = insp.fonts.as_deref().unwrap_or(&[]);
        let lang = self.lang.as_deref();
        let runs = |t: &str, k: f32, color: &dyn Fn([f32; 3]) -> Color32| {
            let text = insp.text.preview(t);
            page_text::runs(&ctx, &drawn_pieces(&o, fonts, &text), k, color, lang)
        };
        let (out, area) = if drawn {
            let runs = |t: &str| runs(t, k, &|c| page_text::shown(mode, c));
            let style = page_text::Style {
                runs: &runs,
                mode,
                accent: ui.visuals().selection.stroke.color,
                warn: &warn,
            };
            let (x, y) = o.baseline.unwrap_or((b[0], b[3]));
            let c = cover_box(&o);
            let cover = self.cover.map(|fill| {
                let c = Rect::from_min_max(to_screen(c[0], c[1]), to_screen(c[2], c[3]));
                (c, fill)
            });
            let f = page_text::field(
                &ctx,
                id,
                &mut typed,
                to_screen(x, y),
                &style,
                false,
                cover,
                false,
            );
            (f.out, f.outline)
        } else {
            let area = Rect::from_min_max(to_screen(b[0], b[1]), to_screen(b[2], b[3])).expand(2.0);
            // A turned line shows its styles too, at its size on the page, in a box as long
            // as the line and one line high; invisible (OCR) text has no styles, and is
            // shown at its box's height.
            let parts = insp.text.drawn_parts();
            let largest = parts.iter().map(|p| p.size).fold(0.0, f32::max);
            let px = if o.hidden {
                area.height() * 0.75
            } else {
                largest * k
            };
            let font = egui::FontId::proportional(px.clamp(6.0, 96.0));
            let scale = font.size / largest.max(0.01);
            let size = if o.hidden {
                area.size()
            } else {
                egui::vec2(area.width().max(area.height()), font.size * 1.4)
            };
            let mut layouter = |_: &egui::Ui, t: &dyn egui::TextBuffer, _: f32| {
                let paper = |c| page_text::shown(PageColours::Normal, c);
                page_text::galley(&ctx, t.as_str(), &runs(t.as_str(), scale, &paper)).0
            };
            // At the line, moved left as far as it must to stay in view.
            let view = ui.clip_rect();
            let x = area.min.x.min(view.max.x - size.x).max(view.min.x);
            let held = egui::TextEdit::load_state(&ctx, id).and_then(|s| s.cursor.char_range());
            let mut out = egui::Area::new(id.with("area"))
                .fixed_pos(Pos2::new(x, area.min.y))
                .order(egui::Order::Foreground)
                .show(&ctx, |ui| {
                    // Light colours in either theme, as for text boxes: the text sits on
                    // the paper.
                    *ui.visuals_mut() = egui::Visuals::light();
                    let mut buffer = page_text::Clusters(&mut typed);
                    let te = egui::TextEdit::singleline(&mut buffer)
                        .id(id)
                        .font(font)
                        .min_size(size)
                        .desired_width(size.x)
                        .clip_text(false)
                        .margin(egui::Margin::same(2));
                    if o.hidden {
                        te.show(ui)
                    } else {
                        te.layouter(&mut layouter).show(ui)
                    }
                })
                .inner;
            page_text::keep_on_right_click(&ctx, id, &mut out, held);
            page_text::snap_drawn(&ctx, id, &mut out, &typed, held);
            (out, area)
        };
        // The styles follow what was typed.
        let sel = out
            .cursor_range
            .map(|c| (c.secondary.index.0, c.primary.index.0));
        if insp.text.sync(&typed, sel.map(|s| s.1)) {
            ctx.request_repaint();
        }
        if let Some(sel) = sel {
            insp.text.caret_at(sel);
        }
        let now = ctx.input(|i| i.time);
        insp.history.observe(&insp.text, sel, now, settled(&ctx));
        let edit = &out.response.response;
        // What an input method composes shows unchecked; left without the keyboard, it is
        // taken as committed (PB-54).
        le.composing = page_text::composition(&ctx, id, le.composing);
        if !edit.has_focus() {
            let left = commit_line_composition(le, insp, o.hidden);
            self.left_composing = self.left_composing.take().or(left);
        }
        self.typing_box = Some(edit.clone());
        le.area = area.union(edit.rect);
        let name = format!("Editing text: {}", o.text.as_deref().unwrap_or(""));
        dialogs::set_name(edit, &name);
        le.refused.show(&ctx, &out, id);
        if le.fresh {
            // The caret where clicked, or the word double-clicked selected, as in a word
            // processor: by the text as drawn, or along the plain box.
            let (from, to) = match le.pick {
                Pick::Chars(a, b) => (a, b),
                Pick::At(p, word) => {
                    let i = if drawn {
                        let at = to_screen(p.x, p.y) - out.galley_pos;
                        out.galley.cursor_from_pos(at).index.0
                    } else {
                        caret_along(&o, p, insp.text.text().chars().count())
                    };
                    if word {
                        word_around(insp.text.text(), i)
                    } else {
                        (i, i)
                    }
                }
            };
            page_text::select(&ctx, id, (from, to));
            insp.history
                .observe(&insp.text, Some((from, to)), now, true);
            edit.request_focus();
            le.fresh = false;
            return true;
        }
        // Under a dialog (a change's failure said, say) it waits, open, for it to close, and
        // takes the keyboard back then: its Esc and clicks are the dialog's.
        if crate::dialogs::any_open(&ctx) {
            le.refocus = true;
            return edit.hovered();
        }
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if esc && (edit.lost_focus() || edit.has_focus()) {
            // Esc keeps what was typed, as a click away does; Ctrl+Z takes it back.
            self.end_line_edit();
            return true;
        }
        // F6 took the keyboard to another region: the line waits, open, for it to come back.
        if !edit.has_focus() && self.parked == Some(Parked::Typing(id)) {
            return edit.hovered();
        }
        // The band's font list, size, colour and buttons (and the inspector's and the
        // floating bar's) change the line being edited, which takes the keyboard back once
        // the change is finished, as new text does.
        let bar = drawn_at(&ctx, self.bar_rect);
        let in_inspector = keeping(
            &ctx,
            Some(drawn_at(&ctx, self.insp_rect)),
            &[&self.keep_typing[..], &[bar]].concat(),
        );
        let picking = egui::Popup::is_any_open(&ctx);
        if !edit.has_focus() && (le.refocus || in_inspector || picking) {
            // Not in the click's own frames: a list it opens shows from the next one.
            if settled(&ctx) && !picking && !in_inspector {
                page_text::give_back(&ctx, id);
                le.refocus = false;
            } else {
                le.refocus = true;
            }
            return edit.hovered();
        }
        // Scrolled out of view, or the keyboard went elsewhere: keep what was typed. A click
        // that ended it still selects what it was on.
        if !edit.has_focus() || !ui.clip_rect().intersects(area) {
            self.end_line_edit();
            return false;
        }
        edit.hovered()
    }

    /// The reply typed to comment `id` and not yet added, if any: a draft, or what the reply
    /// box holds for the selected comment.
    pub fn draft(&self, id: (u32, u16)) -> Option<&str> {
        let d = self.drafts.get(&id)?;
        (!d.text.trim().is_empty()).then_some(d.text.as_str())
    }

    /// The comments with draft replies.
    pub fn drafted(&self) -> Vec<(u32, u16)> {
        self.drafts.keys().copied().collect()
    }

    /// Where the drafts' comments are now, after a change to the document: `pages` gives
    /// each of [`Self::drafted`]'s page, or `None` for one no longer in it.
    pub fn place_drafts(&mut self, pages: &[Option<usize>]) {
        for (d, &p) in self.drafts.values_mut().zip(pages) {
            d.page = p;
        }
    }

    /// The note or text box being typed on `page` (drawn at `r`, `k` pixels per point):
    /// a comment box beside a note's icon, or the text box's own area. Esc and Ctrl+Enter
    /// keep what was typed, as does a click outside a note's comment box or the text box
    /// losing the keyboard. For a new note only a click on the page counts as outside, so
    /// its colour can be changed in the inspector while it is typed. Returns true when the
    /// page should not take the pointer this frame: it is over the text box, or the typing
    /// just ended.
    fn writing_ui(&mut self, ui: &mut egui::Ui, page: usize, r: Rect, k: f32) -> bool {
        let (Some(id), scope) = (self.writing_id(), self.scope) else {
            return false;
        };
        let Some(w) = self.writing.as_mut().filter(|w| w.page == page) else {
            return false;
        };
        // A new text box shows the Text box tool's size and border, which it is kept with,
        // as the inspector changes them.
        if w.id.is_none() && !w.note {
            w.size = self.box_style.size;
            w.border = self.box_style.border;
        }
        let to_screen = |x: f32, y: f32| Pos2::new(r.min.x + x * k, r.min.y + y * k);
        let b = shown_box(w.note, w.rect, k);
        let area = Rect::from_min_max(to_screen(b[0], b[1]), to_screen(b[2], b[3]));
        // Light colours in either theme, as for form fields: the text sits on the paper.
        let theme = ui.visuals().clone();
        // `held`: a click on the page that only closed a list or colour picker.
        let (out, outside, held) = if w.note {
            let [cr, cg, cb] = self.note_color.map(|v| (v * 255.0).round() as u8);
            if w.id.is_none() {
                ui.painter()
                    .rect_filled(area, 2.0, Color32::from_rgb(cr, cg, cb));
            }
            let held = egui::TextEdit::load_state(ui.ctx(), id).and_then(|s| s.cursor.char_range());
            let inner = egui::Area::new(scope.with(("note-writing", page)))
                .fixed_pos(area.right_top() + Vec2::new(6.0, 0.0))
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style())
                        .show(ui, |ui| {
                            ui.set_width(240.0);
                            let mut buffer = page_text::Clusters(&mut w.text);
                            let mut out = egui::TextEdit::multiline(&mut buffer)
                                .id(id)
                                .hint_text("Comment")
                                .desired_rows(4)
                                .desired_width(f32::INFINITY)
                                .show(ui);
                            page_text::keep_on_right_click(ui.ctx(), id, &mut out, held);
                            page_text::snap_drawn(ui.ctx(), id, &mut out, &w.text, held);
                            out.response.response
                        })
                        .inner
                });
            // Only a click on the page closes it: one in the inspector changes it, its text
            // typed here alone (ENH-13). Panels are drawn on the background layer, which egui
            // doesn't list among the layers under the pointer.
            let click = ui.input(|i| i.pointer.any_click().then(|| i.pointer.interact_pos()));
            let on_page = click.flatten().is_some_and(|pos| {
                let layer = ui.ctx().layer_id_at(pos);
                !inner.response.rect.contains(pos)
                    && ui.clip_rect().contains(pos)
                    && layer.is_none_or(|l| l == ui.layer_id())
            });
            let open = egui::Popup::is_any_open(ui.ctx());
            let picking = std::mem::replace(&mut w.picking, open);
            (inner.inner, on_page && !picking, on_page && picking)
        } else {
            // Set in Helvetica, or its embedded face: what neither has isn't typed, as on
            // the page. A note's comment is a text string, which takes any character.
            if ui.ctx().memory(|m| m.has_focus(id)) {
                let helvetica = FontChoice::default();
                if let Some(c) = page_text::leave_out_untypable(ui, false, helvetica) {
                    let note = page_text::note(&c).to_string();
                    w.refused.mark(ui.ctx(), note);
                }
            }
            // In Helvetica, as the box is written, the words it lacks in the fonts core
            // writes them in, not the interface's font; on the lines core breaks it into,
            // where it writes them in the box.
            let set = BoxSetting::text_box(w.rect[2] - w.rect[0], w.border, w.size);
            let lang = self.lang.as_deref();
            let (helvetica, color) = (FontChoice::default(), egui::Visuals::light().text_color());
            let now = page_text::set_in_box(ui.ctx(), &set, &w.text, helvetica, k, color, lang);
            let mut layouter = |ui: &egui::Ui, t: &dyn egui::TextBuffer, _: f32| {
                if t.as_str() == now.galley.text() {
                    return now.galley.clone();
                }
                page_text::set_in_box(ui.ctx(), &set, t.as_str(), helvetica, k, color, lang).galley
            };
            let mut buffer = page_text::Clusters(&mut w.text);
            let te = egui::TextEdit::multiline(&mut buffer)
                .id(id)
                .font(egui::FontId::proportional(now.px))
                .desired_rows(1)
                .min_size(area.size())
                .desired_width(area.width())
                .margin(now.margin)
                .layouter(&mut layouter);
            *ui.visuals_mut() = egui::Visuals::light();
            // As `Ui::put` lays it out, keeping the output for the note at the caret.
            let layout = egui::Layout::centered_and_justified(egui::Direction::TopDown);
            let at = egui::UiBuilder::new().max_rect(area).layout(layout);
            let held = egui::TextEdit::load_state(ui.ctx(), id).and_then(|s| s.cursor.char_range());
            let mut out = ui.scope_builder(at, |ui| te.show(ui)).inner;
            page_text::keep_on_right_click(ui.ctx(), id, &mut out, held);
            page_text::snap_drawn(ui.ctx(), id, &mut out, &w.text, held);
            // What an input method composes shows unchecked; left without the keyboard, it
            // is taken as committed (PB-54).
            w.composing = page_text::composition(ui.ctx(), id, w.composing);
            if !out.response.response.has_focus() {
                let left = w.commit_composition();
                self.left_composing = self.left_composing.take().or(left);
            }
            *ui.visuals_mut() = theme;
            w.refused.show(ui.ctx(), &out, id);
            (out.response.response, false, false)
        };
        self.typing_box = Some(out.clone());
        if w.fresh {
            out.request_focus();
            w.fresh = false;
            return false;
        }
        // The focus is asked first: asking inside `input_mut` would wait on the lock it
        // holds.
        let focused = out.has_focus();
        let (finish, esc) = ui.input_mut(|i| {
            (
                focused && i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        let esc = esc && (out.lost_focus() || out.has_focus());
        // A note stays open when it loses the keyboard (to Tab, say) until a click outside
        // it; a text box ends then, unless F6 took the keyboard to another region, which
        // it waits for.
        let parked = self.parked == Some(Parked::Typing(out.id));
        // Copy and Paste in the band leave it open, and it takes the keyboard back; so does a
        // change in the inspector, which shows the note or text box (ENH-13), or the Text
        // box tool's look for a new one, as word processors keep a box open while its pane
        // is used.
        let insp = Some(drawn_at(ui.ctx(), self.insp_rect));
        if keeping(ui.ctx(), insp, &self.keep_typing) {
            w.refocus = true;
        }
        // Under a dialog (a change's failure said, say) it waits, open, for it to close.
        if crate::dialogs::any_open(ui.ctx()) {
            w.refocus = true;
            return held || (!w.note && out.hovered());
        }
        let kept = parked || w.refocus;
        if finish || esc || outside || (!w.note && out.lost_focus() && !kept) {
            if outside {
                self.note_closed = Some(ui.ctx().cumulative_pass_nr());
            }
            self.finish_writing();
            return true;
        }
        // After the keys, so the Esc that closed the colour picker doesn't also end it.
        if w.refocus && settled(ui.ctx()) && !out.has_focus() {
            page_text::hand_back(ui.ctx(), out.id);
            w.refocus = false;
        }
        held || (!w.note && out.hovered())
    }

    /// The selected annotation, once its page's annotations have been read.
    pub fn selected_annot(&self) -> Option<&Annot> {
        let (page, id) = self.selection.annot()?;
        self.annots.get(&page)?.iter().find(|a| a.id == id)
    }

    /// Put the Sign tool's picked item at `place` on `page`; with no signature saved yet,
    /// open the Create window first and put the new one there.
    pub fn place_sign(&mut self, page: usize, place: StampPlace) {
        self.sign_error = None;
        match self.sign.pick {
            _ if self.sign.ready(&self.sign_kit) => {
                self.actions.push(Action::AddSign(page, place));
            }
            SignPick::Saved(role, _) => {
                let author = self.author.clone();
                self.sign.start_create(role, &author, Some((page, place)));
            }
            _ => self.sign_error = Some("Type the text to put down first.".into()),
        }
    }

    pub fn set_tool(&mut self, t: Tool) {
        self.finish_adding();
        self.finish_writing();
        self.commit_insp();
        self.line_edit = None;
        if Tool::COMMENTING.contains(&t) {
            self.annot_tool = t;
        }
        self.tool = t;
        self.drag = None;
        self.draw = None;
        self.marquee = None;
        self.pen_stroke = None;
        if t == Tool::Read {
            self.selection.clear_object();
            self.insp = None;
        } else {
            self.selection.clear_words();
            self.text_drag = false;
        }
        // A tool that puts things down starts with nothing selected, so its options show.
        if !matches!(t, Tool::Read | Tool::Select) {
            self.selection.clear_object();
            self.selection.clear_annot();
        }
        if t != Tool::DigitalSign {
            self.digital_place = None;
            self.digital_error = None;
        }
        if t != Tool::Sign {
            self.sign.create = None;
        }
    }

    /// After a command failed part way: the Read tool, with nothing selected, dragged, being
    /// typed or waiting to be put down, and no changes queued.
    pub fn reset(&mut self) {
        self.tool = Tool::Read;
        self.selection = Selection::None;
        self.drag = None;
        self.draw = None;
        self.marquee = None;
        self.pen_stroke = None;
        self.text_drag = false;
        self.adding = None;
        self.writing = None;
        self.pending_image = None;
        self.insp = None;
        self.group = None;
        self.line_edit = None;
        self.annot_insp = None;
        self.digital_place = None;
        self.digital_error = None;
        self.actions.clear();
        self.later.clear();
        self.not_kept = None;
        self.apply_redactions = false;
    }

    pub fn set_tool_keep_sel(&mut self, t: Tool) {
        let sel = self.selection.clone();
        self.set_tool(t);
        self.selection = sel;
    }

    /// Let the selected line of page text be moved and resized with the read tool, as
    /// text just added is, until something else is selected (Move or resize text).
    pub fn move_text(&mut self) {
        // A line being edited on the page is finished first, what was typed kept.
        if self.line_edit.take().is_some() {
            self.commit_insp();
        }
        if self.tool != Tool::Read {
            self.set_tool_keep_sel(Tool::Read);
        }
        self.selection.set_loose(true);
    }

    /// Select object `id` on `page`, it alone; nothing, if the page has no such object.
    pub fn select_object(&mut self, page: usize, id: usize) {
        self.select_objects(page, [id]);
    }

    /// Select objects `ids` on `page`, those alone; the ones the page doesn't have are left
    /// out, and with none, nothing is selected.
    pub fn select_objects(&mut self, page: usize, ids: impl IntoIterator<Item = usize>) {
        let objs = self.objs.get(&page).map_or(&[][..], |v| v);
        let mut seen = std::collections::HashSet::new();
        let held: Vec<Selected> = (ids.into_iter())
            .filter(|&id| seen.insert(id))
            .filter_map(|id| Some(Selected::of(id, objs.get(id)?)))
            .collect();
        self.selection = Selection::objects_from(page, held);
    }

    /// Shift+click with the arrow: add object `id` on `page` to the objects selected there,
    /// or take it out if it is one of them. Another page's objects, or anything else
    /// selected, give way to it.
    pub fn toggle_object(&mut self, page: usize, id: usize) {
        if self.selection.holds(page, id) {
            self.selection.let_go(|h| h.id == id);
            return;
        }
        let o = self.objs.get(&page).and_then(|v| v.get(id));
        if let (Selection::Objects { page: on, held, .. }, Some(o)) = (&mut self.selection, o) {
            if *on == page {
                held.push(Selected::of(id, o));
                return;
            }
        }
        self.select_object(page, id);
    }

    /// Select every object on `page` that a click could select with the arrow: invisible
    /// text only while it is shown (Ctrl+A with the arrow).
    pub fn select_all_objects(&mut self, pdf: &limarepdf_core::Pdf, page: usize) {
        let shown = self.show_hidden;
        let ids: Vec<usize> = (self.objects(pdf, page).iter().enumerate())
            .filter(|(_, o)| shown || !o.hidden)
            .map(|(i, _)| i)
            .collect();
        self.select_objects(page, ids);
    }

    /// The selected objects on `page`, with their boxes as they are shown: moved along while
    /// they are being dragged.
    fn held_boxes(&self, page: usize) -> Vec<(usize, [f32; 4])> {
        let Some((on, held)) = self.selection.objects().filter(|(p, _)| *p == page) else {
            return Vec::new();
        };
        let objs = self.objs.get(&on).map_or(&[][..], |v| v);
        let drag = self
            .drag
            .as_ref()
            .filter(|d| d.page == page && d.annot.is_none());
        // A move takes them all (its group is what is selected), a resize the one.
        let moved = drag
            .filter(|d| matches!(d.mode, Mode::Move) && !d.group.is_empty())
            .map(|d| (d.cur[0] - d.orig[0], d.cur[1] - d.orig[1]));
        let resized = drag.filter(|d| d.group.is_empty()).map(|d| (d.id, d.cur));
        held.iter()
            .filter_map(|h| {
                let b = objs.get(h.id)?.bbox;
                Some((
                    h.id,
                    match (moved, resized) {
                        (Some((dx, dy)), _) => shifted(b, dx, dy),
                        (_, Some((id, cur))) if id == h.id => cur,
                        _ => b,
                    },
                ))
            })
            .collect()
    }

    /// The selected object's page and index.
    pub fn sel(&self) -> Option<(usize, usize)> {
        self.selection.object()
    }

    /// The selected line's fonts, when it is on `page` and they aren't read yet: from the
    /// fonts of all the page's lines, read together the first time one of them is selected
    /// and kept until the document changes, so that selecting the page's lines one after
    /// another reads the page once. A page that can't be read gives none: the edit then
    /// says what is wrong.
    fn read_line_fonts(&mut self, pdf: &limarepdf_core::Pdf, page: usize) {
        let line = |o: &PageObject| o.kind == ObjectKind::Text && !o.hidden;
        let Some(insp) = (self.insp.as_mut()).filter(|i| i.key.0 == page && i.fonts.is_none())
        else {
            return;
        };
        let o = self.objs.get(&page).and_then(|v| v.get(insp.key.1));
        if o.is_some_and(line) {
            let all = (self.line_fonts.entry(page))
                .or_insert_with(|| pdf.page_line_fonts(page).unwrap_or_default());
            insp.fonts = Some(all.get(insp.key.1).cloned().flatten().unwrap_or_default());
        }
    }

    /// The objects on `page`, read once until the document changes. Read again after a
    /// change, the objects selected there are found again in them ([`Selection::refind`]).
    pub fn objects(&mut self, pdf: &limarepdf_core::Pdf, page: usize) -> &[PageObject] {
        if !self.objs.contains_key(&page) {
            let objs = match pdf.page_objects(page) {
                Ok(v) => v,
                Err(e) => {
                    self.unavailable.insert(page, e.to_string());
                    Vec::new()
                }
            };
            self.selection.refind(page, &objs);
            self.objs.insert(page, objs);
        }
        &self.objs[&page]
    }

    fn words(&mut self, pdf: &limarepdf_core::Pdf, page: usize) -> &[PageWord] {
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
    fn extend_words(&mut self, pdf: &limarepdf_core::Pdf, end: WordAt) {
        let anchor = match self.selection {
            Selection::Words { anchor, .. } => anchor,
            _ => end,
        };
        for p in anchor.0.min(end.0)..=anchor.0.max(end.0) {
            self.words(pdf, p);
        }
        self.selection = Selection::Words { anchor, end };
    }

    /// Move the moving end of the words selected `toward` a word or line, as Shift with an
    /// arrow, Home or End moves a word processor's: a word left or right in reading order,
    /// to the word nearest across on the line above or below, each across pages, or to its
    /// line's first or last word.
    pub fn extend_by(&mut self, pdf: &limarepdf_core::Pdf, toward: Toward) {
        let Selection::Words { end: (page, i), .. } = self.selection else {
            return;
        };
        let words = self.words(pdf, page).to_vec();
        let Some(w) = words.get(i) else {
            return;
        };
        let (first, last) = Self::line_of(&words, i).unwrap_or((i, i));
        let x = (w.bbox[0] + w.bbox[2]) / 2.0;
        let to = match toward {
            Toward::LineStart => Some((page, first)),
            Toward::LineEnd => Some((page, last)),
            Toward::Right if i + 1 < words.len() => Some((page, i + 1)),
            Toward::Left if i > 0 => Some((page, i - 1)),
            Toward::Down if last + 1 < words.len() => Some((page, across(&words, last + 1, x))),
            Toward::Up if first > 0 => Some((page, across(&words, first - 1, x))),
            // Past the page's words: on to the next page with words, or back to the last.
            Toward::Right | Toward::Down => {
                let next = (page + 1..pdf.page_count()).find(|&p| !self.words(pdf, p).is_empty());
                next.map(|p| match toward {
                    Toward::Right => (p, 0),
                    _ => (p, across(&self.words[&p], 0, x)),
                })
            }
            Toward::Left | Toward::Up => {
                let back = (0..page).rev().find(|&p| !self.words(pdf, p).is_empty());
                back.map(|p| {
                    let n = self.words[&p].len();
                    match toward {
                        Toward::Left => (p, n - 1),
                        _ => (p, across(&self.words[&p], n - 1, x)),
                    }
                })
            }
        };
        if let Some(to) = to {
            self.extend_words(pdf, to);
        }
    }

    /// The moving end of the words selected: its page and box (page points).
    pub fn words_end(&self) -> Option<(usize, [f32; 4])> {
        let Selection::Words { end, .. } = self.selection else {
            return None;
        };
        Some((end.0, self.words.get(&end.0)?.get(end.1)?.bbox))
    }

    /// Start selecting words with a drag pressed at `p` (page points) on `page`, from the
    /// word nearest it among the page's `words`.
    fn start_word_drag(&mut self, page: usize, p: Option<Pos2>, words: &[PageWord]) {
        if let Some(i) = p.and_then(|p| Self::nearest_word(words, p)) {
            self.selection = Selection::word((page, i));
            self.text_drag = true;
        }
    }

    /// While words are selected by dragging, the selection's end follows the pointer:
    /// whichever visible page (`page` drawn at `r`, `k` pixels per point) it is over, held
    /// inside the view, which scrolls at its edges. The drag ends with `resp`'s.
    #[allow(clippy::too_many_arguments)]
    fn follow_word_drag(
        &mut self,
        ui: &egui::Ui,
        pdf: &limarepdf_core::Pdf,
        page: usize,
        r: Rect,
        k: f32,
        resp: &egui::Response,
        words: &[PageWord],
    ) {
        if self.text_drag {
            let view = ui.clip_rect();
            let at = ui.input(|i| i.pointer.latest_pos()).map(|p| view.clamp(p));
            let at = at.filter(|p| r.contains(*p));
            let at = at.map(|p| Pos2::new((p.x - r.min.x) / k, (p.y - r.min.y) / k));
            if let Some(i) = at.and_then(|p| Self::nearest_word(words, p)) {
                self.extend_words(pdf, (page, i));
            }
        }
        if resp.drag_stopped() {
            self.text_drag = false;
        }
    }

    /// Shade the selected words of `page` (`words`) in `fill` ([`theme::text_shading`]) as
    /// word processors shade a selection ([`Self::shaded`]).
    fn paint_words(
        &self,
        painter: &egui::Painter,
        page: usize,
        words: &[PageWord],
        screen_rect: impl Fn([f32; 4]) -> Rect,
        fill: Color32,
    ) {
        if let Some(range) = self.selection.words_on(page, words.len()) {
            for b in Self::shaded(&words[range]) {
                painter.rect_filled(screen_rect(b), 1.0, fill);
            }
        }
    }

    /// The note about what was typed with words selected ([`Self::type_over`]), under
    /// their moving end, where a word processor's caret is, when that is on `page` (whose
    /// `words` are drawn at `screen_rect`).
    fn words_note_at(
        &mut self,
        ctx: &egui::Context,
        page: usize,
        words: &[PageWord],
        screen_rect: impl Fn([f32; 4]) -> Rect,
    ) {
        let Selection::Words { anchor, end } = self.selection else {
            return;
        };
        let Some(w) = words.get(end.1).filter(|_| end.0 == page) else {
            return;
        };
        let r = screen_rect(w.bbox);
        let caret = if end >= anchor {
            r.right_bottom()
        } else {
            r.left_bottom()
        };
        let id = self.scope.with(("words-note", page));
        let below = caret + Vec2::new(0.0, 4.0);
        self.words_note.show_at(ctx, below, id);
        self.input_method_at_words(ctx, r, caret.x);
    }

    /// With words selected and nothing having the keyboard, the input method is on, as for
    /// a box with the keyboard, so its first composition types over them ([`Self::type_over`],
    /// PB-54); its candidate window at `x` on the screen, the selection's moving end, in the
    /// last word, drawn at `word`.
    fn input_method_at_words(&self, ctx: &egui::Context, word: Rect, x: f32) {
        if ctx.memory(|m| m.focused().is_some()) || !matches!(self.tool, Tool::Read | Tool::Text) {
            return;
        }
        let caret = Rect::from_x_y_ranges(x..=x + 1.0, word.y_range());
        ctx.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Normal,
                rect: word,
                cursor_rect: caret,
                should_interrupt_composition: false,
            });
        });
    }

    /// The bands that shade `words`: one a line, the spaces between its words too (what a
    /// mark made from the selection covers, [`Self::selected_lines`]), and the lines of a
    /// paragraph meeting halfway across the space between them, with no stripe between.
    fn shaded(words: &[PageWord]) -> Vec<[f32; 4]> {
        let mut bands = Self::line_boxes(words);
        for i in 1..bands.len() {
            let (above, below) = (bands[i - 1], bands[i]);
            let gap = below[1] - above[3];
            let height = (above[3] - above[1]).min(below[3] - below[1]);
            let across = above[0] < below[2] && below[0] < above[2];
            if across && gap > 0.0 && gap < 0.5 * height {
                let middle = above[3] + gap / 2.0;
                (bands[i - 1][3], bands[i][1]) = (middle, middle);
            }
        }
        bands
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

    /// The word a double-click or triple-click at `p` (page points) is on: the nearest of
    /// `words`, when `p` is on its line, within `tol` of the line's box. Off the text,
    /// none, as a word processor selects nothing there.
    fn word_on_line(words: &[PageWord], p: Pos2, tol: f32) -> Option<usize> {
        let i = Self::nearest_word(words, p)?;
        let on = words.iter().filter(|w| w.line == words[i].line);
        let line = on.fold(words[i].bbox, |u, w| union(u, w.bbox));
        let near = p.x >= line[0] - tol
            && p.x <= line[2] + tol
            && p.y >= line[1] - tol
            && p.y <= line[3] + tol;
        near.then_some(i)
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

    /// The selected objects, as the page has them now.
    fn selected_objects(&self) -> Vec<&PageObject> {
        let Some((page, held)) = self.selection.objects() else {
            return Vec::new();
        };
        let objs = self.objs.get(&page).map_or(&[][..], |v| v);
        held.iter().filter_map(|h| objs.get(h.id)).collect()
    }

    /// The text of the selected lines of page text (visible or not), in reading order: top
    /// to bottom, and left to right along a row.
    pub fn selected_line_text(&self) -> Vec<String> {
        let mut lines: Vec<&PageObject> = (self.selected_objects().into_iter())
            .filter(|o| o.kind == ObjectKind::Text)
            .collect();
        lines.sort_by(|a, b| a.bbox[1].total_cmp(&b.bbox[1]));
        // A line whose middle is within half a row's first line's height of that line's
        // middle is on its row.
        let mid = |o: &PageObject| (o.bbox[1] + o.bbox[3]) / 2.0;
        let mut rows: Vec<Vec<&PageObject>> = Vec::new();
        for o in lines {
            let first = rows.last().map(|r| r[0]);
            match first.filter(|f| (mid(o) - mid(f)).abs() < (f.bbox[3] - f.bbox[1]) / 2.0) {
                Some(_) => rows.last_mut().unwrap().push(o),
                None => rows.push(vec![o]),
            }
        }
        (rows.into_iter())
            .flat_map(|mut row| {
                row.sort_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
                row
            })
            .filter_map(|o| o.text.clone())
            .collect()
    }

    /// After an insertion: what was put down is at the end of the page's objects, from
    /// object `from` on (the lines of new text, say), and is selected, as one.
    pub fn select_new(&mut self, pdf: &limarepdf_core::Pdf, page: usize, from: usize) {
        let n = self.objects(pdf, page).len();
        self.select_objects(page, from..n);
        self.insp = None;
    }

    fn hit(&self, page: usize, p: Pos2, tol: f32) -> Option<usize> {
        self.hit_where(page, p, tol, |_| true)
    }

    /// The visible line of page text under `p`, for editing it.
    fn text_hit(&self, page: usize, p: Pos2, tol: f32) -> Option<usize> {
        self.hit_where(page, p, tol, |o| o.kind == ObjectKind::Text && !o.hidden)
    }

    /// The picture or shape under `p` that a click with the read tool selects: not page
    /// text or a group, and nothing covering most of the page (`area`, square points),
    /// such as a scanned page or a background, which the arrow still selects.
    fn read_hit(&self, page: usize, p: Pos2, tol: f32, area: f32) -> Option<usize> {
        self.hit_where(page, p, tol, |o| {
            let b = o.bbox;
            matches!(o.kind, ObjectKind::Image | ObjectKind::Path)
                && (b[2] - b[0]) * (b[3] - b[1]) < 0.8 * area
        })
    }

    /// Whether the tool moves and resizes what is selected: the arrow, and the read tool
    /// for all but page text (see [`Self::arrangeable`]).
    fn arranging(&self) -> bool {
        matches!(self.tool, Tool::Select | Tool::Read)
    }

    /// Whether the selected objects can be moved (and one resized) now: anything with the
    /// arrow; with the read tool, all but page text, except text just added (`loose`).
    fn arrangeable(&self) -> bool {
        let Some((_, held)) = self.selection.objects() else {
            return false;
        };
        let text = (held.iter()).any(|h| matches!(h.kind, ObjKind::Text | ObjKind::HiddenText));
        match self.tool {
            Tool::Select => true,
            Tool::Read => !text || self.selection.loose(),
            _ => false,
        }
    }

    /// The smallest object under `p` (within `tol`) that `keep` accepts; invisible text
    /// only while it is shown.
    fn hit_where(
        &self,
        page: usize,
        p: Pos2,
        tol: f32,
        keep: impl Fn(&PageObject) -> bool,
    ) -> Option<usize> {
        let objs = self.objs.get(&page)?;
        objs.iter()
            .enumerate()
            .filter(|(_, o)| {
                keep(o)
                    && (self.show_hidden || !o.hidden)
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
        pdf: &limarepdf_core::Pdf,
        page: usize,
        r: Rect,
        k: f32,
        matches: &[Match],
        current: Option<usize>,
    ) {
        // Invisible text hidden again is let go.
        let hidden = |h: &Selected| h.kind == ObjKind::HiddenText;
        let held = self.selection.objects().map_or(&[][..], |(_, held)| held);
        if !self.show_hidden && held.iter().any(hidden) {
            self.selection.let_go(hidden);
            self.insp = None;
        }
        let painter = ui.painter();
        let screen_rect = |b: [f32; 4]| {
            Rect::from_min_max(
                Pos2::new(r.min.x + b[0] * k, r.min.y + b[1] * k),
                Pos2::new(r.min.x + b[2] * k, r.min.y + b[3] * k),
            )
        };
        // A match broken across lines has a box on each; `current` is the current match's
        // hit, which may go on from the page before.
        for (hit, boxes) in matches {
            for b in boxes {
                let sr = screen_rect(*b).expand(1.0);
                painter.rect_filled(sr, 1.0, Color32::from_rgba_unmultiplied(255, 210, 0, 90));
                if current == Some(*hit) {
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
        pdf: &limarepdf_core::Pdf,
        tab_id: u64,
        page: usize,
        r: Rect,
        k: f32,
    ) -> egui::Response {
        let to_screen = |x: f32, y: f32| Pos2::new(r.min.x + x * k, r.min.y + y * k);
        let to_pt = |p: Pos2| Pos2::new((p.x - r.min.x) / k, (p.y - r.min.y) / k);
        let click_pt = |resp: &egui::Response| resp.interact_pointer_pos().map(to_pt);
        self.zoom = k;
        // Another object or nothing selected: the last one's changes are kept, even with
        // the inspector folded away.
        if self
            .insp
            .as_ref()
            .is_some_and(|i| Some(i.key) != self.sel())
        {
            self.sync_insp();
        }
        self.read_line_fonts(pdf, page);
        self.sync_group();
        let screen_rect =
            |b: [f32; 4]| Rect::from_min_max(to_screen(b[0], b[1]), to_screen(b[2], b[3]));
        let id = ui.scope_id().with(("page-edit", tab_id, page));
        let mut resp = ui.interact(r, id, Sense::click_and_drag());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Unknown, true, format!("Page {}", page + 1))
        });
        if resp.secondary_clicked() {
            let p = resp.interact_pointer_pos().map(to_pt);
            let area = (r.width() / k) * (r.height() / k);
            self.select_for_menu(pdf, page, p, 3.0 / k, area);
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
        let shading = theme::text_shading(ui.visuals().dark_mode);
        let accent_strong = ui.visuals().selection.stroke.color;
        let hover_pt = resp.hover_pos().map(to_pt);
        let tol = 3.0 / k;
        let htol = 6.0 / k;
        let shift = ui.input(|i| i.modifiers.shift);
        let area = (r.width() / k) * (r.height() / k);
        // Where a drag with the arrow or the read tool begins, the frame it does: once the
        // pointer has gone further from the press than a click can. One let go before that,
        // however long it was held, is a click there.
        let origin = ui.input(|i| i.pointer.press_origin());
        let far = {
            let click = ui.ctx().options(|o| o.input_options.max_click_dist);
            let at = ui.input(|i| i.pointer.latest_pos());
            origin.zip(at).is_some_and(|(a, b)| a.distance(b) > click)
        };
        if resp.drag_started() {
            self.still = origin.map(|p| (page, to_pt(p)));
        }
        let ours = |s: &mut (usize, Pos2)| s.0 == page;
        let begins = ((resp.drag_started() || resp.dragged()) && far)
            .then(|| self.still.take_if(ours).map(|s| s.1))
            .flatten();
        let still_click = (resp.drag_stopped())
            .then(|| self.still.take_if(ours).map(|s| s.1))
            .flatten();

        // current selection box on this page: an object's, or an annotation's that the
        // arrow can move
        self.annots(pdf, page);
        let sel_here = self.sel().filter(|s| s.0 == page);
        // The objects selected here, as shown, and the box round them all.
        let held_here = self.held_boxes(page);
        let held_box = held_here.iter().map(|h| h.1).reduce(union);
        let on_held = |p: Pos2| {
            held_here.iter().any(|(_, b)| {
                p.x >= b[0] - tol && p.x <= b[2] + tol && p.y >= b[1] - tol && p.y <= b[3] + tol
            })
        };
        let sel_annot = self
            .selected_annot()
            .filter(|a| a.page == page && self.arranging() && movable(a))
            .map(|a| (a.id, a.bbox, resizable(a)));
        // A selected line's two ends, which have handles instead of its box.
        let sel_line = self
            .selected_annot()
            .filter(|a| a.page == page && self.arranging() && movable(a))
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
            .filter(|a| a.page == page && self.arranging() && movable(a))
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
            .and_then(|(_, id)| held_here.iter().find(|h| h.0 == id).map(|h| h.1))
            .or_else(|| {
                let (id, b, _) = sel_annot?;
                let d = self.drag.as_ref().filter(|d| d.annot == Some(id));
                let d = d.filter(|d| !matches!(d.mode, Mode::Rotate));
                Some(shown_box(sel_note, d.map_or(b, |d| d.cur), k))
            });
        // The box that has handles: not a note's, nor page text's with the read tool, nor
        // several objects' (they move together, one resizes alone).
        let sized_box = sel_box.filter(|_| match sel_annot {
            Some((_, _, r)) => r,
            None => sel_here.is_some() && self.arrangeable(),
        });
        // With the read tool, a press on the selected item (or its handles) moves it.
        let grabs = |ed: &mut Self, p: Pos2| {
            let on_handle = sized_box.is_some_and(|b| ed.handle_hit(b, p, htol).is_some());
            let on_annot = sel_annot.is_some_and(|(id, _, _)| {
                ed.annot_at(pdf, page, p).is_some_and(|a| a.id == id)
                    || ed.selected_annot().is_some_and(|a| in_mark_box(a, p))
            });
            let on_object = ed.arrangeable() && on_held(p);
            on_handle || on_annot || on_object || rotate_hit(p) || end_hit(p).is_some()
        };
        let annot_drag = |a: &Annot, mode: Mode, start: Pos2| Drag {
            page,
            id: 0,
            annot: Some(a.id),
            mode,
            start,
            orig: a.bbox,
            cur: a.bbox,
            group: Vec::new(),
        };
        // A press at `p` starts moving, resizing or turning: by the selected item's
        // handles; else with the arrow whatever is under it, which is selected, and with
        // the read tool the selected item itself (see `grabs`). With the arrow, a press on
        // nothing, on a background not selected, or with Shift away from the objects
        // selected, starts a rectangle that selects what is wholly inside it.
        let start_arrange = |ed: &mut Self, p: Pos2, read: bool| {
            let h = sized_box.and_then(|b| ed.handle_hit(b, p, htol));
            let selected = ed.selected_annot().filter(|_| sel_annot.is_some()).cloned();
            let under = ed.hit(page, p, tol);
            if let Some((id, b, _)) = sel_ink.as_ref().filter(|_| rotate_hit(p)) {
                ed.drag = Some(Drag {
                    page,
                    id: 0,
                    annot: Some(*id),
                    mode: Mode::Rotate,
                    start: p,
                    orig: *b,
                    cur: [0.0; 4],
                    group: Vec::new(),
                });
            } else if let (Some(i), Some((id, ends))) = (end_hit(p), sel_line) {
                ed.drag = Some(Drag {
                    page,
                    id: 0,
                    annot: Some(id),
                    mode: Mode::End(i),
                    start: p,
                    orig: ends,
                    cur: ends,
                    group: Vec::new(),
                });
            } else if let (Some(h), Some(a)) = (h, &selected) {
                ed.drag = Some(annot_drag(a, Mode::Handle(h), p));
            } else if let (Some(h), Some((_, id))) = (h, sel_here) {
                ed.start_drag(page, id, Mode::Handle(h), p);
            } else if read {
                match &selected {
                    Some(a) => ed.drag = Some(annot_drag(a, Mode::Move, p)),
                    None => ed.start_move(page, p),
                }
            } else if shift {
                match under.filter(|&i| ed.selection.holds(page, i)) {
                    Some(_) => ed.start_move(page, p),
                    None => ed.start_marquee(page, p, true),
                }
            } else if let Some(a) = selected.as_ref().filter(|a| in_mark_box(a, p)) {
                ed.drag = Some(annot_drag(a, Mode::Move, p));
            } else if let Some(a) = ed.annot_at(pdf, page, p).cloned() {
                ed.selection = Selection::Annot { page, id: a.id };
                if movable(&a) {
                    ed.drag = Some(annot_drag(&a, Mode::Move, p));
                }
            } else if under.is_some_and(|i| ed.selection.holds(page, i)) {
                ed.start_move(page, p);
            } else if let Some(i) = under.filter(|&i| !ed.background(page, i, area)) {
                ed.select_object(page, i);
                ed.start_move(page, p);
            } else {
                ed.start_marquee(page, p, false);
            }
        };
        // Typing new text, a note or text box, or a line of page text: it takes the
        // keyboard and the pointer, drawn in the fonts core sets it in for the document's
        // language.
        if self.typing_on_page() {
            self.lang = pdf.language();
        }
        if self.adding_ui(ui, page, r, k)
            || self.writing_ui(ui, page, r, k)
            || self.line_edit_ui(ui, page, r, k)
        {
            return resp;
        }
        let ctrl_annot = click_pt(&resp)
            .filter(|_| ctrl && resp.clicked())
            .filter(|_| Tool::ANNOTATE.contains(&self.tool) || self.tool == Tool::Sign)
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
                    } else if let Some(i) = self.hit(page, p, tol).filter(|_| !self.dragging()) {
                        // A drag moves what is selected, or what isn't but for a
                        // background, which a drag draws a rectangle over instead.
                        let held = self.selection.holds(page, i);
                        if held || !self.background(page, i, area) {
                            ui.ctx().set_cursor_icon(CursorIcon::Grab);
                        }
                        if !held {
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
                if let Some(p) = begins {
                    start_arrange(self, p, false);
                }
                if resp.dragged() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        match self.marquee.as_mut().filter(|m| m.page == page) {
                            Some(m) => m.cur = to_pt(pos),
                            None => self.drag_to(ui.ctx(), page, to_pt(pos), shift),
                        }
                    }
                }
                if resp.drag_stopped() {
                    self.marquee_done(page);
                    self.drag_done(page);
                }
                // A click selects what is under it, alone; Shift+click adds an object to
                // those selected, or takes it out, and does nothing on a comment.
                let click = (resp.clicked())
                    .then(|| resp.interact_pointer_pos().map(to_pt))
                    .flatten()
                    .or(still_click);
                if let Some(p) = click {
                    let h = sized_box.and_then(|b| self.handle_hit(b, p, htol));
                    if h.is_none() {
                        let annot = self.annot_at(pdf, page, p).map(|a| a.id);
                        match (annot, self.hit(page, p, tol)) {
                            (Some(_), _) if shift => {}
                            (Some(id), _) => self.selection = Selection::Annot { page, id },
                            (None, Some(i)) if shift => self.toggle_object(page, i),
                            (None, Some(i)) => self.select_object(page, i),
                            (None, None) if shift => {}
                            (None, None) => {
                                self.selection.clear_object();
                                self.selection.clear_annot();
                            }
                        }
                    }
                }
                if resp.double_clicked() && self.selection.annot().is_some() {
                    self.write_selected();
                } else if resp.double_clicked() {
                    // On page text: edit its line there, the word selected, as the read tool
                    // does.
                    let text = self
                        .selected_object()
                        .is_some_and(|o| o.kind == ObjectKind::Text);
                    let at = resp.interact_pointer_pos().map(to_pt);
                    if let (true, Some((on, i)), Some(p)) = (text, self.sel(), at) {
                        let words = self.words(pdf, on).to_vec();
                        let pick = self.word_pick(on, i, p, &words);
                        self.start_line_edit(on, i, pick);
                    }
                }
            }
            Tool::Read => {
                let words = self.words(pdf, page).to_vec();
                let inside =
                    |p: Pos2, b: [f32; 4]| p.x >= b[0] && p.x <= b[2] && p.y >= b[1] && p.y <= b[3];
                let lines = Self::line_boxes(&words);
                let on_text = |p: Pos2| lines.iter().any(|&b| inside(p, b));
                // Pictures and shapes a click selects; not ones covering most of the page.
                let object_at = |ed: &Self, p: Pos2| {
                    let i = ed.read_hit(page, p, tol, area)?;
                    (!on_text(p)).then_some(i)
                };
                if let Some(p) = hover_pt.filter(|_| !over_link && !self.text_drag) {
                    let handle = sized_box.and_then(|b| self.handle_hit(b, p, htol));
                    if rotate_hit(p) && !self.dragging() {
                        rotate_cursor(ui.ctx());
                    } else if end_hit(p).is_some() {
                        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                    } else if let Some(h) = handle {
                        ui.ctx().set_cursor_icon(handle_cursor(h));
                    } else if grabs(self, p) {
                        ui.ctx().set_cursor_icon(CursorIcon::Grab);
                    } else if on_text(p) {
                        ui.ctx().set_cursor_icon(CursorIcon::Text);
                    } else if let Some(i) =
                        object_at(self, p).filter(|&i| !self.selection.holds(page, i))
                    {
                        // What a click would select is outlined.
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
                let nearest = |p: Option<Pos2>| p.and_then(|p| Self::nearest_word(&words, p));
                // A drag selects the words between the press and the pointer, in reading
                // order, across pages: whichever visible page the pointer is over (held
                // inside the view, which scrolls at its edges) moves the end. A drag that
                // starts on the selected pictures, shapes or comment moves them instead. A
                // click clears or selects a picture, shape or comment, Shift+click
                // extends the words or adds a picture or shape, a double-click edits a
                // line of page text (or selects a word of text that isn't one, such as
                // recognised text) and a triple-click selects a line; off the text,
                // neither selects anything.
                if let Some(p) = begins {
                    if grabs(self, p) {
                        start_arrange(self, p, true);
                    } else {
                        self.start_word_drag(page, Some(p), &words);
                    }
                }
                if self.drag.is_some() {
                    if resp.dragged() {
                        if let Some(pos) = resp.interact_pointer_pos() {
                            self.drag_to(ui.ctx(), page, to_pt(pos), shift);
                        }
                    }
                    if resp.drag_stopped() {
                        self.drag_done(page);
                    }
                }
                self.follow_word_drag(ui, pdf, page, r, k, &resp, &words);
                let click_at = || resp.interact_pointer_pos().map(to_pt);
                let on_line = |p: Option<Pos2>| p.and_then(|p| Self::word_on_line(&words, p, tol));
                if resp.triple_clicked() {
                    let line = on_line(click_at()).and_then(|i| Self::line_of(&words, i));
                    if let Some((a, b)) = line {
                        self.selection = Selection::Words {
                            anchor: (page, a),
                            end: (page, b),
                        };
                    }
                } else if resp.double_clicked() {
                    // On a note or text box, open it for typing; on page text, edit its
                    // line there; else select a word.
                    let line = click_at().and_then(|p| Some((p, self.text_hit(page, p, tol)?)));
                    let annot = click_at().and_then(|p| self.annot_at(pdf, page, p));
                    if let Some(id) = annot.filter(|a| writable(a)).map(|a| a.id) {
                        self.selection = Selection::Annot { page, id };
                        self.write_selected();
                    } else if let Some((p, i)) = line {
                        let pick = self.word_pick(page, i, p, &words);
                        self.start_line_edit(page, i, pick);
                    } else if let Some(i) = on_line(click_at()) {
                        self.selection = Selection::word((page, i));
                    }
                } else if let Some(p) = resp.clicked().then(click_at).flatten().or(still_click) {
                    // A click on a comment or markup selects it, as one off the text on a
                    // picture or shape does; a drag still selects the text under them.
                    // Shift+click extends the words selected; else, as with the arrow, it
                    // adds a picture or shape to those selected or takes it out, and
                    // changes nothing elsewhere.
                    let annot = (!shift)
                        .then(|| self.annot_at(pdf, page, p).map(|a| a.id))
                        .flatten();
                    let object = object_at(self, p);
                    let extend = shift && self.selection.words().is_some();
                    match (annot, object, nearest(Some(p))) {
                        (Some(id), _, _) => self.selection = Selection::Annot { page, id },
                        (None, _, Some(i)) if extend => self.extend_words(pdf, (page, i)),
                        (None, Some(i), _) if shift => self.toggle_object(page, i),
                        (None, Some(i), _) => self.select_object(page, i),
                        _ if shift && self.selection.objects().is_some() => {}
                        _ => {
                            self.selection.clear_words();
                            self.selection.clear_annot();
                            self.selection.clear_object();
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
                self.paint_words(&painter, page, &words, screen_rect, shading);
                self.words_note_at(ui.ctx(), page, &words, screen_rect);
            }
            Tool::Text => {
                ui.ctx().set_cursor_icon(CursorIcon::Text);
                // A drag selects text, as with the read tool, finishing new text first.
                let words = self.words(pdf, page).to_vec();
                if resp.drag_started() {
                    self.finish_adding();
                    let p = ui.input(|i| i.pointer.press_origin()).map(to_pt);
                    self.start_word_drag(page, p, &words);
                }
                self.follow_word_drag(ui, pdf, page, r, k, &resp, &words);
                self.paint_words(&painter, page, &words, screen_rect, shading);
                self.words_note_at(ui.ctx(), page, &words, screen_rect);
                if let Some(p) = click_pt(&resp).filter(|_| resp.clicked()) {
                    self.selection.clear_words();
                    if self.adding.as_ref().is_some_and(PageText::has_text) {
                        // A click elsewhere finishes the text being typed.
                        self.finish_adding();
                    } else if let Some(i) = self.text_hit(page, p, tol) {
                        // On page text: edit its line, the caret where clicked.
                        self.adding = None;
                        self.start_line_edit(page, i, Pick::At(p, false));
                    } else {
                        // New text there; with nothing typed yet, it moves there.
                        let look = self.new_look();
                        self.adding = Some(PageText::new(self.scope, page, (p.x, p.y), look));
                    }
                }
            }
            Tool::Note => {
                ui.ctx().set_cursor_icon(CursorIcon::Cell);
                if let Some(p) = click_pt(&resp).filter(|_| resp.clicked()) {
                    // A click away from a note being typed (on another page) only closes
                    // it, as a click on its own page does.
                    let closed = self.note_closed == Some(ui.ctx().cumulative_pass_nr());
                    let picking = egui::Popup::is_any_open(ui.ctx())
                        || self.writing.as_ref().is_some_and(|w| w.picking);
                    if self.writing.is_some() || closed {
                        // A click that only closes a list or colour picker keeps it open.
                        if !picking {
                            self.finish_writing();
                        }
                    } else {
                        // The icon, drawn at a fixed size on screen from the note's top
                        // left corner, is centred on the click. The note itself is 20
                        // points.
                        let (x, y) = (p.x - NOTE_PX / 2.0 / k, p.y - NOTE_PX / 2.0 / k);
                        let rect = [x, y, x + 20.0, y + 20.0];
                        self.start_writing(Writing::new(page, true, rect, 0.0));
                    }
                }
                if let Some(a) = hover_pt.and_then(|p| self.annot_at(pdf, page, p)) {
                    resp = resp.on_hover_text_at_pointer(annot_tip(a));
                }
            }
            Tool::Image
            | Tool::Rect
            | Tool::Ellipse
            | Tool::Line
            | Tool::TextBox
            | Tool::Stamp
            | Tool::Redact
            | Tool::Sign
            | Tool::DigitalSign => {
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
                } else if resp.clicked() && self.tool == Tool::Sign {
                    // In an empty signature field, the item fills the field.
                    if let Some(p) = resp.interact_pointer_pos().map(to_pt) {
                        let field = crate::signing::box_at(&pdf.empty_signature_boxes(page), p);
                        let place = match field {
                            Some(b) => StampPlace::Fill(b),
                            None => StampPlace::Centre(p.x, p.y),
                        };
                        self.place_sign(page, place);
                    }
                } else if resp.clicked() && self.tool == Tool::DigitalSign {
                    // A click picks an empty signature field; anywhere else needs a box.
                    if let Some(p) = resp.interact_pointer_pos().map(to_pt) {
                        let inside = |b: &[f32; 4]| {
                            let n = norm(*b);
                            p.x >= n[0] && p.x <= n[2] && p.y >= n[1] && p.y <= n[3]
                        };
                        let field = pdf.empty_signature_fields().into_iter().find(|f| {
                            f.widgets
                                .iter()
                                .any(|w| w.page == page && !w.hidden && inside(&w.bbox))
                        });
                        match field {
                            Some(f) => {
                                self.digital_error = None;
                                self.digital_place = Some(SignPlace::Field(f.name));
                            }
                            None => {
                                self.digital_error = Some(
                                    "Click inside an empty signature field, or drag a box where \
                                     the signature goes."
                                        .into(),
                                )
                            }
                        }
                    }
                } else if resp.clicked() && self.tool == Tool::Stamp {
                    if let Some(p) = resp.interact_pointer_pos().map(to_pt) {
                        let at = StampPlace::Centre(p.x, p.y);
                        self.actions.push(Action::AddStamp(page, at));
                    }
                } else if resp.clicked() && self.tool != Tool::Redact {
                    // A click makes a default size; a redaction mark has none.
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
                            Tool::Redact => self.actions.push(Action::AddRedactMark(page, n)),
                            Tool::Sign => self.place_sign(page, StampPlace::Fill(n)),
                            Tool::DigitalSign => {
                                if n[2] - n[0] >= MIN_SIGNATURE_BOX
                                    && n[3] - n[1] >= MIN_SIGNATURE_BOX
                                {
                                    self.digital_error = None;
                                    self.digital_place = Some(SignPlace::NewBox { page, rect: n });
                                } else {
                                    self.digital_error = Some(format!(
                                        "That box is too small: make it at least \
                                         {MIN_SIGNATURE_BOX} points wide and high."
                                    ));
                                }
                            }
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
        } else if self.arranging() {
            if let Some((_, b, _)) = &sel_ink {
                let (top, knob) = (
                    to_screen((b[0] + b[2]) / 2.0, b[1]),
                    to_screen(rotate_at(*b).x, rotate_at(*b).y),
                );
                painter.line_segment([top, knob], Stroke::new(1.0, accent_strong));
                painter.circle_filled(knob, 5.0, Color32::WHITE);
                painter.circle_stroke(knob, 5.0, Stroke::new(1.5, accent_strong));
            }
            if held_here.len() > 1 {
                // Several: each outlined, and a dashed box round them all, with no handles.
                for (_, b) in &held_here {
                    let sr = screen_rect(*b);
                    let st = Stroke::new(1.5, accent_strong);
                    painter.rect_stroke(sr, 0.0, st, StrokeKind::Outside);
                }
                if let Some(b) = held_box {
                    let sr = screen_rect(b).expand(4.0);
                    let corners = [sr.left_top(), sr.right_top(), sr.right_bottom()];
                    let ring = [&corners[..], &[sr.left_bottom(), sr.left_top()]].concat();
                    let st = Stroke::new(1.0, accent_strong);
                    painter.extend(egui::Shape::dashed_line(&ring, st, 5.0, 3.0));
                }
            } else if let Some(b) = sel_box {
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
        // The rectangle being drawn, and what it would select outlined as it grows.
        if let Some(rect) = self
            .marquee
            .as_ref()
            .filter(|m| m.page == page)
            .map(Marquee::rect)
        {
            let objs = self.objs.get(&page).map_or(&[][..], |v| v);
            for i in self.inside_rect(page, rect) {
                let st = Stroke::new(1.0, accent_strong.gamma_multiply(0.8));
                painter.rect_stroke(screen_rect(objs[i].bbox), 0.0, st, StrokeKind::Outside);
            }
            let st = Stroke::new(1.0, accent_strong);
            let fill = accent.gamma_multiply(0.25);
            painter.rect(screen_rect(rect), 0.0, fill, st, StrokeKind::Inside);
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
            let ink = Color32::from_gray(40);
            // The seven standard note icons, shaped as Acrobat shows them: the symbol
            // itself in the note's colour, outlined. Other names show Note.
            let glyph = match a.name.as_str() {
                "Comment" => icon::CHAT_TEXT,
                "Key" => icon::KEY,
                "Help" => icon::QUESTION,
                "Paragraph" => icon::PARAGRAPH,
                "Insert" => icon::CARET_UP,
                "NewParagraph" => {
                    // No Phosphor symbol: a triangle over the letters NP.
                    let at = |x: f32, y: f32| sr.min + Vec2::new(x, y) * NOTE_PX;
                    let tri = vec![at(0.5, 0.08), at(0.82, 0.48), at(0.18, 0.48)];
                    painter.add(egui::Shape::convex_polygon(
                        tri,
                        fill,
                        Stroke::new(1.0, ink),
                    ));
                    painter.text(
                        at(0.5, 0.74),
                        egui::Align2::CENTER_CENTER,
                        "NP",
                        egui::FontId::new(NOTE_PX * 0.42, theme::semibold()),
                        ink,
                    );
                    continue;
                }
                _ => icon::NOTE,
            };
            for (family, color) in [(theme::icons_fill(), fill), (theme::icons(), ink)] {
                painter.text(
                    sr.center(),
                    egui::Align2::CENTER_CENTER,
                    glyph,
                    egui::FontId::new(NOTE_PX, family),
                    color,
                );
            }
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
        resp
    }

    /// Before a right-click menu opens: select what is under the pointer `p` (page
    /// points), as most apps do. With the arrow, an object; with the read tool, a word
    /// (keeping a word selection the click falls inside); otherwise, or over nothing,
    /// the page (keeping a page selection that includes it).
    fn select_for_menu(
        &mut self,
        pdf: &limarepdf_core::Pdf,
        page: usize,
        p: Option<Pos2>,
        tol: f32,
        area: f32,
    ) {
        let annotating =
            matches!(self.tool, Tool::Select | Tool::Sign) || Tool::ANNOTATE.contains(&self.tool);
        match (self.tool, p) {
            (_, Some(p)) if annotating && self.annot_at(pdf, page, p).is_some() => {
                if let Some(id) = self.annot_at(pdf, page, p).map(|a| a.id) {
                    self.selection = Selection::Annot { page, id };
                }
                return;
            }
            (Tool::Select, Some(p)) => {
                self.objects(pdf, page);
                // One of the objects selected keeps them all, for the menu for them.
                if let Some(i) = self.hit(page, p, tol) {
                    if !self.selection.holds(page, i) {
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
                // A picture or shape, as a click selects them; one of those selected keeps
                // them all.
                self.objects(pdf, page);
                if let Some(i) = self.read_hit(page, p, tol, area) {
                    if !self.selection.holds(page, i) {
                        self.select_object(page, i);
                    }
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
                group: Vec::new(),
            });
        }
    }

    /// Start moving the objects selected on `page` together, from `start`.
    fn start_move(&mut self, page: usize, start: Pos2) {
        let group = self.held_boxes(page);
        let Some(b) = group.iter().map(|g| g.1).reduce(union) else {
            return;
        };
        self.drag = Some(Drag {
            page,
            id: group[0].0,
            annot: None,
            mode: Mode::Move,
            start,
            orig: b,
            cur: b,
            group,
        });
    }

    /// Start a rectangle on `page` at `start`, selecting what it wholly holds; with `add`,
    /// as well as the objects selected.
    fn start_marquee(&mut self, page: usize, start: Pos2, add: bool) {
        self.marquee = Some(Marquee {
            page,
            start,
            cur: start,
            add,
        });
    }

    /// The objects on `page` wholly inside `rect` (page points) that the arrow can select:
    /// invisible text only while it is shown.
    fn inside_rect(&self, page: usize, rect: [f32; 4]) -> Vec<usize> {
        let objs = self.objs.get(&page).map_or(&[][..], |v| v);
        (objs.iter().enumerate())
            .filter(|(_, o)| (self.show_hidden || !o.hidden) && inside(o.bbox, rect))
            .map(|(i, _)| i)
            .collect()
    }

    /// The rectangle on `page` was let go: what it wholly holds is selected, added to the
    /// objects selected when it was started with Shift. Holding nothing, it clears the
    /// selection, as a click on nothing does, or with Shift leaves it as it was.
    fn marquee_done(&mut self, page: usize) {
        let Some(m) = self.marquee.take().filter(|m| m.page == page) else {
            return;
        };
        let new = self.inside_rect(page, m.rect());
        if m.add {
            let mut ids = (self.selection.object_ids())
                .filter(|(p, _)| *p == page)
                .map_or_else(Vec::new, |(_, ids)| ids);
            let had: std::collections::HashSet<usize> = ids.iter().copied().collect();
            if new.iter().all(|i| had.contains(i)) {
                return;
            }
            ids.extend(new.into_iter().filter(|i| !had.contains(i)));
            let loose = self.selection.loose();
            self.select_objects(page, ids);
            self.selection.set_loose(loose);
        } else if new.is_empty() {
            self.selection.clear_object();
            self.selection.clear_annot();
        } else {
            self.select_objects(page, new);
        }
    }

    /// Whether object `id` on `page` covers most of the page (`area`, square points), as a
    /// scanned page's picture or a background does: a drag that starts on it while it isn't
    /// selected draws a rectangle, so it isn't dragged away by accident.
    fn background(&self, page: usize, id: usize, area: f32) -> bool {
        self.objs
            .get(&page)
            .and_then(|v| v.get(id))
            .is_some_and(|o| {
                let b = o.bbox;
                (b[2] - b[0]) * (b[3] - b[1]) >= 0.8 * area
            })
    }

    /// Follow the pointer, at `p` (page points) on `page`, with the move, resize or turn
    /// being dragged. Shift keeps proportions, lines at 45° and turns to steps of 15°.
    fn drag_to(&mut self, ctx: &egui::Context, page: usize, p: Pos2, shift: bool) {
        let Some(d) = self.drag.as_mut().filter(|d| d.page == page) else {
            return;
        };
        let (dx, dy) = (p.x - d.start.x, p.y - d.start.y);
        d.cur = match d.mode {
            Mode::Move => [
                d.orig[0] + dx,
                d.orig[1] + dy,
                d.orig[2] + dx,
                d.orig[3] + dy,
            ],
            Mode::Handle(h) => resize(d.orig, h, dx, dy, shift),
            // Shift keeps the line at a multiple of 45° from its other end.
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
            // The angle swept about the box's middle since the press.
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
            Mode::Move => ctx.set_cursor_icon(CursorIcon::Grabbing),
            Mode::Handle(h) => ctx.set_cursor_icon(handle_cursor(h)),
            Mode::End(_) => ctx.set_cursor_icon(CursorIcon::Crosshair),
            Mode::Rotate => rotate_cursor(ctx),
        }
    }

    /// The drag on `page` ended: queue the move, resize or turn it made, if any.
    fn drag_done(&mut self, page: usize) {
        let Some(d) = self.drag.take() else {
            return;
        };
        let moved = match d.mode {
            Mode::Rotate => d.cur[0].abs() > 0.1,
            _ => d.cur.iter().zip(&d.orig).any(|(a, b)| (a - b).abs() > 0.5),
        };
        if d.page == page && moved {
            let (dx, dy) = (d.cur[0] - d.orig[0], d.cur[1] - d.orig[1]);
            self.actions.push(match (d.annot, d.mode) {
                (Some(id), Mode::End(_)) => Action::LineEnds(d.page, id, d.cur),
                (Some(id), Mode::Rotate) => Action::RotateInk(d.page, id, d.cur[0]),
                (Some(id), _) => Action::MoveAnnot(d.page, id, d.cur),
                (None, Mode::Move) => Action::SetBox(
                    d.page,
                    (d.group.iter())
                        .map(|&(id, b)| (id, shifted(b, dx, dy)))
                        .collect(),
                ),
                (None, _) => Action::SetBox(d.page, vec![(d.id, d.cur)]),
            });
        }
    }

    /// Select every word on page `page` (read tool).
    pub fn select_all_words(&mut self, pdf: &limarepdf_core::Pdf, page: usize) {
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

    /// Whether the inspector shows the editor's section: a selected object or comment,
    /// selected words whose look can change (their font, Bold, Italic, size and colour), or
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
                | Tool::Redact
                | Tool::Sign
                | Tool::DigitalSign
        ) || self.has_object()
            || self.selection.annot().is_some()
            || self.words_panel()
    }

    /// The inspector's heading for the editor's section, as it shows them: the selected
    /// comment or words, else the tool's options, else the selected object or objects
    /// ("Comment", "Selected text", "New rectangle", "Image", "3 objects").
    pub fn heading(&self) -> String {
        if self.selection.annot().is_some() {
            return match self.selected_annot() {
                Some(a) if a.reply_to.is_some() => "Reply".into(),
                Some(a) => a.label(),
                None => "Comment".into(),
            };
        }
        if self.words_panel() {
            return "Selected text".into();
        }
        if let Some(tool) = self.tool_heading() {
            return tool.into();
        }
        match self.selection.objects() {
            Some((_, held)) if held.len() > 1 => format!("{} objects", held.len()),
            _ => self.object_heading().unwrap_or("Selection").into(),
        }
    }

    /// Whether the Text tool is adding text: not while it edits a line of the page, which
    /// the band and the inspector show as the other tools do.
    fn adding_text(&self) -> bool {
        self.tool == Tool::Text && self.line_edit.is_none()
    }

    /// The heading of a tool's options in the inspector.
    fn tool_heading(&self) -> Option<&'static str> {
        Some(match self.tool {
            Tool::Text if !self.adding_text() => return None,
            Tool::Text => "New text",
            Tool::Image => "Insert image",
            Tool::Rect => "New rectangle",
            Tool::Ellipse => "New ellipse",
            Tool::Line => "New line",
            Tool::Pen => "Pen",
            Tool::Note => "New note",
            Tool::Stamp => "New stamp",
            Tool::Sign => "Fill and sign",
            Tool::DigitalSign if self.digital_certify => "Certify with a digital ID",
            Tool::DigitalSign => "Sign with a digital ID",
            Tool::Redact => "Mark for redaction",
            Tool::TextBox => "New text box",
            Tool::Select | Tool::Read => return None,
        })
    }

    /// The heading of the selected object's properties in the inspector.
    fn object_heading(&self) -> Option<&'static str> {
        let (page, id) = self.sel()?;
        let o = self.objs.get(&page)?.get(id)?;
        Some(match o.kind {
            ObjectKind::Text if o.hidden => "Recognised text (invisible)",
            // As the workspace has it, and as for selected words.
            ObjectKind::Text => "Selected text",
            ObjectKind::Image => "Image",
            ObjectKind::Form => "Group",
            ObjectKind::Path => "Shape",
        })
    }

    /// The page and box (page points) of the selected objects (the box round them all) or
    /// words, for placing the floating toolbar. For words, the part on the page where the
    /// selection's moving end is. A drawing's box includes its rotate handle, so the toolbar
    /// doesn't cover it.
    pub fn selection_box(&self) -> Option<(usize, [f32; 4])> {
        if let Some((page, held)) = self.selection.objects() {
            let objs = self.objs.get(&page)?;
            let b = (held.iter()).filter_map(|h| objs.get(h.id).map(|o| o.bbox));
            return Some((page, b.reduce(union)?));
        }
        if let Some(a) = self.selected_annot() {
            let mut b = a.bbox;
            if a.kind == AnnotKind::Ink && self.arranging() && movable(a) {
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

    /// A line of page text is being edited on the page.
    pub fn editing_line(&self) -> bool {
        self.line_edit.is_some()
    }

    /// Where the line being edited on `page` was drawn last on the screen, as typed.
    pub fn line_area(&self, page: usize) -> Option<Rect> {
        let le = self.line_edit.as_ref().filter(|le| le.page == page)?;
        le.area.is_positive().then_some(le.area)
    }

    /// The boxes (page points) of the text on `page`, as far as it has been read: its lines
    /// and its words. The floating bar keeps clear of them.
    pub fn text_boxes(&self, page: usize) -> Vec<[f32; 4]> {
        let objs = self.objs.get(&page).into_iter().flatten();
        let lines = objs.filter(|o| o.kind == ObjectKind::Text).map(|o| o.bbox);
        let words = self.words.get(&page).into_iter().flatten().map(|w| w.bbox);
        lines.chain(words).collect()
    }

    /// Whether words are being selected by dragging. The drag ends when the button is
    /// released, even if the page it started on has scrolled out of view.
    pub fn selecting_words(&mut self, ctx: &egui::Context) -> bool {
        if self.text_drag && !ctx.input(|i| i.pointer.primary_down()) {
            self.text_drag = false;
        }
        self.text_drag
    }

    /// The page a move, resize, turn, shape, pen stroke or selection rectangle being dragged
    /// is on.
    pub fn gesture_page(&self) -> Option<usize> {
        let drag = self.drag.as_ref().map(|d| d.page);
        let draw = self.draw.as_ref().map(|d| d.page);
        let marquee = self.marquee.as_ref().map(|m| m.page);
        (drag.or(draw).or(marquee)).or(self.pen_stroke.as_ref().map(|s| s.page))
    }

    /// The pointer is moving, resizing, drawing or selecting something.
    pub fn dragging(&self) -> bool {
        self.drag.is_some() || self.draw.is_some() || self.marquee.is_some() || self.text_drag
    }

    /// One line of page text is selected, for Edit text to edit on the page.
    pub fn one_line_selected(&self) -> bool {
        self.sel()
            .and_then(|(page, id)| self.objs.get(&page)?.get(id))
            .is_some_and(|o| o.kind == ObjectKind::Text)
    }

    /// Edit the selected line of page text on the page, the caret at its end (Edit text).
    pub fn edit_selected_text(&mut self) {
        let Some((page, id)) = self.sel() else {
            return;
        };
        let Some(o) = self.objs.get(&page).and_then(|v| v.get(id)) else {
            return;
        };
        if o.kind == ObjectKind::Text {
            let end = o.text.as_deref().unwrap_or("").chars().count();
            self.start_line_edit(page, id, Pick::Chars(end, end));
        }
    }

    /// Whether objects are selected (one or several), which can be deleted.
    pub fn has_object(&self) -> bool {
        self.selection.objects().is_some()
    }

    /// Whether the selected objects can be moved now (see [`Self::arrangeable`]).
    pub fn object_moves(&self) -> bool {
        self.arrangeable()
    }

    /// Whether the selected annotation can be moved now.
    pub fn annot_moves(&self) -> bool {
        self.arranging() && self.selected_annot().is_some_and(movable)
    }

    /// Delete the selected objects, or the selected annotation unless it is locked.
    pub fn delete_selected(&mut self) {
        if let Some((p, ids)) = self.selection.object_ids() {
            self.actions.push(Action::Delete(p, ids));
        } else if let Some(a) = self.selected_annot().filter(|a| !a.locked()) {
            self.actions.push(Action::DeleteAnnot(a.page, a.id));
        }
    }

    /// Move the selected objects, or annotation (arrow tool), by (dx, dy) points.
    pub fn nudge(&mut self, dx: f32, dy: f32) {
        if self.annot_moves() {
            if let Some(a) = self.selected_annot() {
                let b = a.bbox;
                let to = [b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy];
                self.actions.push(Action::MoveAnnot(a.page, a.id, to));
                return;
            }
        }
        let Some((p, ids)) = self.selection.object_ids().filter(|_| self.object_moves()) else {
            return;
        };
        let objs = self.objs.get(&p).map_or(&[][..], |v| v);
        let boxes: Vec<(usize, [f32; 4])> = (ids.into_iter())
            .filter_map(|id| Some((id, shifted(objs.get(id)?.bbox, dx, dy))))
            .collect();
        if !boxes.is_empty() {
            self.actions.push(Action::SetBox(p, boxes));
        }
    }

    /// Esc: let go of a move, resize, turn, shape, pen stroke, selection rectangle or word
    /// selection being dragged, changing nothing (the tool stays); else keep the new text,
    /// note, text box or line being typed on the page, or clear a selected annotation, or the
    /// word or object selection (read tool); otherwise leave a drawing tool for the arrow, or
    /// with the arrow clear the selection.
    pub fn escape(&mut self) {
        if self.dragging() || self.pen_stroke.is_some() {
            self.drag = None;
            self.draw = None;
            self.marquee = None;
            self.pen_stroke = None;
            if std::mem::take(&mut self.text_drag) {
                self.selection.clear_words();
            }
            return;
        }
        // New text moved by its outline puts the move back itself ([`PageText::ui`]).
        if self.adding.as_ref().is_some_and(PageText::moving) {
            return;
        }
        if self.adding.is_some() {
            self.finish_adding();
            self.adding = None;
            return;
        }
        if self.writing.is_some() {
            self.finish_writing();
            return;
        }
        if self.line_edit.is_some() {
            // What was typed is kept, as Esc in the editor does.
            self.end_line_edit();
            return;
        }
        if self.selection.annot().is_some() {
            self.selection.clear_annot();
            return;
        }
        match self.tool {
            Tool::Read => {
                self.selection.clear_words();
                self.selection.clear_object();
            }
            // The arrow lets go of the selection first, then gives way to the read tool.
            Tool::Select if self.has_object() => {
                self.selection.clear_object();
            }
            // Any other tool goes back to the read tool; a pinned one stays pinned.
            _ => self.set_tool(Tool::Read),
        }
    }

    /// Something was put down with the current tool; `new` selects it. A tool kept
    /// active stays for the next one; otherwise the read tool comes back with the new item
    /// selected, to adjust (page text just added can be moved with it).
    pub fn placed(&mut self, new: Selection) {
        if self.keep.contains(&self.tool) {
            self.selection.clear_object();
            self.selection.clear_annot();
            return;
        }
        self.set_tool(Tool::Read);
        self.selection = new;
        self.selection.set_loose(true);
    }

    /// The pin at the top of a placing tool's options: on, the tool stays active after
    /// putting something down, until Esc or another tool.
    /// Whether the inspector's heading has the Keep tool active pin: a tool that puts
    /// things down is in use and its options are shown (no comment selected).
    pub fn keeps_pin(&self) -> bool {
        Tool::PLACING.contains(&self.tool) && self.selection.annot().is_none()
    }

    /// The Keep tool active pin, in the inspector's heading row beside its other buttons
    /// (ENH-09: no row of its own between the heading and the tool's options).
    pub fn keep_button(&mut self, ui: &mut egui::Ui) {
        let tool = self.tool;
        let on = self.keep.contains(&tool);
        let pin = egui::Button::new(theme::icon(icon::PUSH_PIN))
            .selected(on)
            .frame_when_inactive(false);
        let r = ui.add(pin).on_hover_text(if on {
            "Keep tool active: on. It stays after putting something down, until Esc or \
             another tool."
        } else {
            "Keep tool active"
        });
        r.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, "Keep tool active")
        });
        if r.clicked() {
            if on {
                self.keep.retain(|t| *t != tool);
            } else {
                self.keep.push(tool);
            }
        }
    }

    // ------------------------------------------------------------ inspector

    /// Keep the working copy on the selected object. When another object (or none) is
    /// selected, the last one's changes are kept first, as clicking away keeps them.
    fn sync_insp(&mut self) {
        let key = self.sel();
        if key.is_some() && self.insp.as_ref().map(|i| i.key) == key {
            return;
        }
        self.commit_insp();
        self.line_edit = None;
        let o = key.and_then(|key| Some((key, self.objs.get(&key.0)?.get(key.1)?)));
        self.insp = o.map(|(key, o)| Insp::of(key, o));
    }

    /// Keep the working copy on the several objects selected ([`Group`]). When the
    /// selection changes, the last one's changes are kept first, as clicking away keeps them.
    fn sync_group(&mut self) {
        let key = self.selection.object_ids().filter(|(_, ids)| ids.len() > 1);
        let same = |g: &Group| {
            key.as_ref()
                .is_some_and(|(p, ids)| g.page == *p && g.ids == *ids)
        };
        if self.group.as_ref().is_some_and(same) {
            return;
        }
        self.keep(Typed::Objects);
        self.group = key.and_then(|(page, ids)| Some(Group::of(page, ids, self.objs.get(&page)?)));
    }

    /// The inspector for several objects selected: what they are, the look of the lines of
    /// text among them and of the shapes, each changing only its own kind, where they are,
    /// and Delete. Changes are kept as they are made, each finished one a step to undo.
    fn group_ui(&mut self, ui: &mut egui::Ui) {
        self.sync_group();
        let Some((page, ids)) = self.selection.object_ids() else {
            return;
        };
        let objs = self.objs.get(&page).map_or(&[][..], |v| v);
        ui.label(kinds_of(ids.iter().filter_map(|&id| objs.get(id))));
        let boxes: Vec<(usize, [f32; 4])> = (ids.iter())
            .filter_map(|&id| Some((id, objs.get(id)?.bbox)))
            .collect();
        let settled = settled(ui.ctx());
        // Their text's look is changed in the band's Font group.
        if let Some(c) = self.chosen.as_ref().filter(|c| c.restylable()) {
            ui.add_space(4.0);
            facts(ui, "lines-look", &look_facts(&c.text));
        }
        if let Some(g) = self.group.as_mut().filter(|g| !g.shapes.is_empty()) {
            ui.add_space(4.0);
            shapes_ui(ui, g);
            if settled {
                self.keep(Typed::Objects);
            }
        }

        // Where they are: the box round them all, which moves them together. They are
        // resized one at a time.
        ui.add_space(4.0);
        let (Some(b), Some(g)) = (boxes.iter().map(|&(_, b)| b).reduce(union), &self.group) else {
            return;
        };
        let [mut x, mut y] = g.at;
        let done = section(ui, section_name("Position"))
            .id_salt("position-and-size")
            .show(ui, |ui| {
                let mut done = false;
                egui::Grid::new("geom").num_columns(2).show(ui, |ui| {
                    for (label, v) in [("X", &mut x), ("Y", &mut y)] {
                        ui.label(label);
                        let field = egui::DragValue::new(v).range(-5000.0..=20000.0);
                        let r = ui.add(field.clamp_existing_to_range(false).speed(0.5));
                        dialogs::set_name(&r, label);
                        done |= r.drag_stopped() || r.lost_focus();
                        ui.end_row();
                    }
                });
                done
            })
            .body_returned
            .unwrap_or(false);
        let (dx, dy) = (x - b[0], y - b[1]);
        if done && (dx.abs() > 0.05 || dy.abs() > 0.05) {
            let to = boxes.iter().map(|&(id, b)| (id, shifted(b, dx, dy)));
            self.actions.push(Action::SetBox(page, to.collect()));
        }
        // Once finished, the fields show where they are, moved or not (a move refused).
        if let Some(g) = self.group.as_mut() {
            g.at = if done { [b[0], b[1]] } else { [x, y] };
        }

        ui.add_space(6.0);
        if theme::glyph_button(ui, icon::TRASH, "Delete").clicked() {
            // What was changed goes with them.
            self.group = None;
            self.chosen = None;
            self.actions.push(Action::Delete(page, ids));
        }
    }

    /// What the selection's words or lines to restyle are: words selected with the read
    /// tool, or several objects selected with the arrow (one is the object inspector's).
    fn chosen_of(&self) -> Option<Of> {
        match &self.selection {
            Selection::Words { .. } => self.selection.words().map(Of::Words),
            Selection::Objects { page, held, .. } if held.len() > 1 => {
                Some(Of::Lines(*page, held.iter().map(|h| h.id).collect()))
            }
            _ => None,
        }
    }

    /// Keep the words selected with the read tool, or the lines among several objects
    /// selected with the arrow, ready to restyle ([`Chosen`]), read from `pdf` when the
    /// selection changes. The last selection's changes are kept first, as clicking away
    /// keeps them.
    pub fn sync_chosen(&mut self, pdf: &limarepdf_core::Pdf) {
        // Read first: after a change, the selected objects are found again as it reads.
        if let Some((page, _)) = self.selection.objects() {
            self.objects(pdf, page);
        }
        let of = self.chosen_of();
        if of.is_some() && self.chosen.as_ref().map(|c| &c.of) == of.as_ref() {
            return;
        }
        self.keep(Typed::Words);
        self.chosen = None;
        let Some(of) = of else {
            return;
        };
        let mut c = Chosen::new(of.clone());
        match of {
            Of::Words(words) => {
                for p in words.0 .0..=words.1 .0 {
                    let n = self.words(pdf, p).len();
                    self.objects(pdf, p);
                    if let Some(range) = self.selection.words_on(p, n) {
                        c.add_page(p, &self.words[&p][range], &self.objs[&p]);
                    }
                }
            }
            Of::Lines(page, ids) => c.add_lines(page, &ids, &self.objs[&page]),
        }
        self.chosen = Some(c);
    }

    /// The words or lines to restyle, when they are the selection's as it is now.
    fn chosen_now(&self) -> Option<&Chosen> {
        let of = self.chosen_of()?;
        self.chosen.as_ref().filter(|c| c.of == of)
    }

    /// Whether the inspector shows the selected words' look: some of them can be restyled.
    fn words_panel(&self) -> bool {
        self.selection.words().is_some() && self.chosen_now().is_some_and(Chosen::restylable)
    }

    /// Bold (with `bold` false, Italic) on or off by Word's rule, as one undo step: for the
    /// words selected with the read tool, or the lines of page text selected with the arrow.
    /// In the line being edited on the page, for what is selected there, or the word at the
    /// caret, as Ctrl+B there does: the line stays open and takes the keyboard back.
    /// Returns what to say when none of the selected words or lines can change.
    pub fn switch_style(
        &mut self,
        ctx: &egui::Context,
        pdf: &limarepdf_core::Pdf,
        bold: bool,
    ) -> Option<String> {
        if self.chosen_of().is_some() {
            self.sync_chosen(pdf);
            let c = self.chosen.as_mut()?;
            if !c.restylable() {
                return c.left.said();
            }
            match bold {
                true => c.text.bold(),
                false => c.text.italic(),
            }
            self.keep(Typed::Words);
            return None;
        }
        let (page, id) = self.sel()?;
        let visible = self.objs.get(&page)?.get(id)?;
        if visible.kind != ObjectKind::Text || visible.hidden {
            return None;
        }
        self.sync_insp();
        let insp = self.insp.as_mut()?;
        let scope = self.scope;
        if let Some(le) = (self.line_edit.as_mut()).filter(|le| (le.page, le.id) == (page, id)) {
            let sel = page_text::held(ctx, line_edit_id(scope, page)).unwrap_or((0, 0));
            match bold {
                true => insp.text.bold(sel),
                false => insp.text.italic(sel),
            }
            le.refocus = true;
            return None;
        }
        let all = (0, insp.text.text().chars().count());
        match bold {
            true => insp.text.bold(all),
            false => insp.text.italic(all),
        }
        self.commit_insp();
        None
    }

    /// Delete the words selected with the read tool from their lines, as one undo step
    /// across lines and pages, as Delete and Backspace take a selection out of a word
    /// processor's text ([`crate::styled::Across::delete`]); nothing is selected after.
    /// Returns what to say when none of them can change.
    pub fn delete_words(&mut self, pdf: &limarepdf_core::Pdf) -> Option<String> {
        self.selection.words()?;
        self.sync_chosen(pdf);
        let c = self.chosen.as_mut()?;
        if !c.restylable() {
            return c.left.said();
        }
        c.text.delete();
        let c = self.chosen.take()?;
        self.selection.clear_words();
        self.actions.push(Action::Restyle(c.changes(), c.left));
        None
    }

    /// After a change to the document, select again the words `sel` selected, as the page
    /// reads now, when they still read `was`; else nothing is selected, rather than other
    /// words.
    pub fn reselect(&mut self, pdf: &limarepdf_core::Pdf, sel: Selection, was: Option<String>) {
        let Selection::Words { anchor, end } = sel else {
            return;
        };
        self.extend_words(pdf, anchor);
        self.extend_words(pdf, end);
        self.selection = Selection::Words { anchor, end };
        if self.selected_text() != was {
            self.selection.clear_words();
        }
    }

    /// The look of the words selected with the read tool, across lines and pages, which
    /// the band's Font group changes ([`Self::font_band`]).
    fn words_ui(&mut self, ui: &mut egui::Ui) {
        let Some(c) = self.chosen.as_ref() else {
            return;
        };
        ui.add_space(4.0);
        let [font, size] = look_facts(&c.text);
        let editing = ("Editing", "Selected words".into());
        facts(ui, "words-look", &[editing, font, size]);
        self.planned_text_tools(ui);
    }

    /// The text tools planned (ENH-41), folded at the end of selected text's inspector as the
    /// workspace has them: marked Planned, saying what is planned. Not with the planned
    /// features hidden.
    fn planned_text_tools(&self, ui: &mut egui::Ui) {
        if !self.planned {
            return;
        }
        let text = |size, color| egui::TextFormat::simple(egui::FontId::proportional(size), color);
        let ink = ui.visuals().text_color();
        let muted = theme::muted(ui.visuals().dark_mode);
        let mut name = egui::text::LayoutJob::default();
        name.append("Future text tools", 0.0, text(12.0, ink));
        name.append("Planned", 6.0, text(11.0, muted));
        let r = section(ui, name)
            .id_salt("future-text-tools")
            .show(ui, |ui| {
                note(
                    ui,
                    "Paragraph reflow, underline and strike-through are planned. Your \
                     existing text tools keep working.",
                );
            });
        let said = "Future text tools — Planned";
        r.header_response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::DisclosureTriangle, true, said));
    }

    /// Edit's Font group (ENH-38): the font list, size, Bold, Italic, `planned` (the planned
    /// styles) and the colour, for the text they change: the new text being typed (what
    /// is selected in it) or, before it, the next new text's look; the words selected with
    /// the read tool; the line of page text selected or being edited (what is selected in
    /// it); the lines among several objects. With none of them, the next new text's look,
    /// greyed. Changes apply as they are made, each finished one an undo step
    /// ([`Self::settle_look`]); the typing stays open and takes the keyboard back. `compact`:
    /// on one row.
    pub fn font_band(&mut self, ui: &mut egui::Ui, planned: &dyn Fn(&mut egui::Ui), compact: bool) {
        if self.adding_text() {
            let Some(a) = self.adding.as_mut() else {
                self.next_text_look(ui, planned, compact);
                return;
            };
            let end = a.text.text().chars().count();
            let sel = page_text::held(ui.ctx(), a.id()).unwrap_or((end, end));
            let mut text = InLine {
                text: &mut a.text,
                sel,
            };
            if styles_ui(ui, &mut text, 4.0..=200.0, &[], planned, compact) {
                a.refocus = true;
            }
            return;
        }
        if self.chosen_now().is_some_and(Chosen::restylable) {
            if let Some(c) = self.chosen.as_mut() {
                styles_ui(ui, &mut c.text, 1.0..=400.0, &c.in_pdf, planned, compact);
                return;
            }
        }
        self.sync_insp();
        let line = self.insp.as_ref().and_then(|insp| {
            let (page, id) = insp.key;
            let o = self.objs.get(&page)?.get(id)?;
            (o.kind == ObjectKind::Text && !o.hidden && self.selection.object().is_some())
                .then(|| o.clone())
        });
        if let (Some(obj), Some(insp)) = (line, self.insp.as_mut()) {
            // What is selected in the line being edited on the page, else all of it.
            let sel = match self.line_edit.as_ref() {
                Some(le) => {
                    page_text::held(ui.ctx(), line_edit_id(self.scope, le.page)).unwrap_or((0, 0))
                }
                None => (0, insp.text.text().chars().count()),
            };
            let in_pdf = pdf_fonts(&obj, sel);
            let mut text = InLine {
                text: &mut insp.text,
                sel,
            };
            styles_ui(ui, &mut text, 1.0..=400.0, &in_pdf, planned, compact);
            return;
        }
        ui.add_enabled_ui(false, |ui| self.next_text_look(ui, planned, compact));
    }

    /// The look of the next new text, in the band's Font group.
    fn next_text_look(
        &mut self,
        ui: &mut egui::Ui,
        planned: &dyn Fn(&mut egui::Ui),
        compact: bool,
    ) {
        ui.with_layout(look_layout(compact), |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            ui.horizontal(|ui| {
                font_field(ui, |ui| {
                    crate::font_list::ui(ui, &mut self.tfont, false, &[])
                });
                if let Some(size) = size_box(ui, Some(self.tsize), 4.0..=200.0) {
                    self.tsize = size;
                }
            });
            ui.horizontal(|ui| {
                crate::font_list::styles(ui, &mut self.tfont);
                planned(ui);
                if let Some(c) = text_color_button(ui, &[self.tcolor]) {
                    self.tcolor = c;
                }
            });
        });
    }

    /// Keeps the changes the band's Font group made, once they are finished (the pointer
    /// let go, no list open), each an undo step: the selected words' or lines', or the
    /// selected line's when it isn't being edited (one being edited keeps them with its
    /// typing). Each frame, whether or not the inspector is shown.
    pub fn settle_look(&mut self, ctx: &egui::Context) {
        let settled = settled(ctx);
        let object = self.line_edit.is_none() && self.selection.object().is_some();
        if let Some(insp) = self.insp.as_mut().filter(|_| object) {
            // The line being edited on the page watches its own changes.
            let now = ctx.input(|i| i.time);
            insp.history.observe(&insp.text, None, now, settled);
        }
        if settled {
            self.keep(Typed::Words);
            if object {
                self.commit_insp();
            }
        }
    }

    /// The selected annotation: what and whose it is, and its colour, opacity and comment,
    /// each finished change kept at once as one undo step.
    fn annot_inspector(&mut self, ui: &mut egui::Ui) {
        let Some(a) = self.selected_annot().cloned() else {
            ui.weak("Nothing selected.");
            return;
        };
        let now = AnnotEdit::of(&a);
        if self.annot_insp.as_ref().is_none_or(|(id, _)| *id != a.id) {
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
        // What it is heads the panel (`Editor::heading`).
        let page = (self.annot_page.clone()).unwrap_or_else(|| (a.page + 1).to_string());
        let mut about = vec![format!("Page {page}")];
        about.extend(
            [&a.author, &a.modified]
                .into_iter()
                .filter(|s| !s.is_empty())
                .cloned(),
        );
        note(ui, about.join(" · "));
        if a.locked() {
            note(ui, "Locked: can't be changed or deleted.");
        }
        ui.separator();
        let mut done = false;
        ui.add_enabled_ui(!a.locked(), |ui| {
            match e.text.as_mut() {
                // A text box's colour is its fill, set with its text.
                Some(s) => text_style_ui(ui, s),
                // A stamp's colours are part of its drawing.
                None if a.kind == AnnotKind::Stamp => {}
                None => {
                    ui.horizontal(|ui| {
                        let mut on = e.color.is_some();
                        let (label, name) = match a.kind {
                            AnnotKind::Square | AnnotKind::Circle => ("Outline", "Outline colour"),
                            AnnotKind::Line | AnnotKind::Ink => ("Line", "Line colour"),
                            _ => ("Colour", "Colour"),
                        };
                        if ui.checkbox(&mut on, label).changed() {
                            e.color = on.then_some(a.color.unwrap_or([0.0; 3]));
                        }
                        if let Some(c) = e.color.as_mut() {
                            color_button(ui, c, name);
                        }
                        if let Some(s) = e.shape.as_mut() {
                            let r = ui.add(
                                egui::DragValue::new(&mut s.width)
                                    .range(0.0..=40.0)
                                    .clamp_existing_to_range(false)
                                    .speed(0.1)
                                    .suffix(" pt"),
                            );
                            dialogs::set_name(&r, "Line width");
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
                                color_button(ui, c, "Fill colour");
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
                let r = dialogs::slider(ui, &mut pct, 0.0..=100.0, " %", "Opacity");
                if r.changed() {
                    e.opacity = pct / 100.0;
                }
            });
            // Open on the page, its text is typed there: one editor for it (ENH-13).
            let typed_on_page = self.writing.as_ref().is_some_and(|w| w.id == Some(a.id));
            let mut left = false;
            if !typed_on_page {
                let label = if e.text.is_some() { "Text" } else { "Comment" };
                ui.label(label);
                // A text box's text is set in Helvetica, or its embedded face: what neither
                // has isn't typed, as on the page. A comment is a text string, which takes
                // any.
                let id = self.scope.with(("annot-text", a.id));
                if e.text.is_some() && ui.ctx().memory(|m| m.has_focus(id)) {
                    let helvetica = FontChoice::default();
                    if let Some(c) = page_text::leave_out_untypable(ui, false, helvetica) {
                        let note = page_text::note(&c).to_string();
                        self.text_refused.mark(ui.ctx(), note);
                    }
                }
                let held =
                    egui::TextEdit::load_state(ui.ctx(), id).and_then(|s| s.cursor.char_range());
                let mut out = egui::TextEdit::multiline(&mut page_text::Clusters(&mut e.contents))
                    .id(id)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .show(ui);
                page_text::snap_drawn(ui.ctx(), id, &mut out, &e.contents, held);
                self.text_refused.show(ui.ctx(), &out, id);
                dialogs::set_name(&out.response.response, label);
                // Changes are kept as they are made, each finished one a step to undo; the
                // comment when it is left, by Tab too, which puts the keyboard on the next
                // control (PB-20).
                left = out.response.response.lost_focus();
            }
            // A text box's words Helvetica lacks, which go to an installed font.
            if e.text.is_some() {
                let on_page = self.writing.as_ref().filter(|_| typed_on_page);
                let text = on_page.map_or(e.contents.as_str(), |w| w.text.as_str());
                let helvetica = FontChoice::default();
                if let Some((said, why)) = page_text::fallback_note(&[(helvetica, text)]) {
                    note(ui, &said).on_hover_text(why);
                }
            }
            // The keyboard back in its text on the page doesn't hold a change made here.
            let ctx = ui.ctx();
            let settled = settled(ctx)
                || (typed_on_page
                    && !egui::Popup::is_any_open(ctx)
                    && !ctx.input(|i| i.pointer.any_down()));
            if theme::glyph_button(ui, icon::TRASH, "Delete").clicked() {
                // What is typed on the page goes with it.
                if typed_on_page {
                    self.writing = None;
                }
                self.actions.push(Action::DeleteAnnot(a.page, a.id));
                done = true;
            } else if !e.same_as(&now) && (settled || left) {
                self.actions.push(Action::SetAnnot(a.page, a.id, e.clone()));
                done = true;
            }
        });
        // Read again from the annotation once the change is made.
        if done {
            self.annot_insp = None;
        }
        // Replies, then a box to add one. Answering doesn't change the annotation, so
        // locked ones can be answered too.
        ui.separator();
        for r in &replies {
            let who: Vec<String> = [&r.author, &r.modified]
                .into_iter()
                .map(|s| limarepdf_core::display_line(s, 80))
                .filter(|s| !s.is_empty())
                .collect();
            note(
                ui,
                if who.is_empty() {
                    "Reply".to_string()
                } else {
                    who.join(" · ")
                },
            );
            ui.label(limarepdf_core::display_text(&r.contents, 2000));
            ui.add_space(4.0);
        }
        // The comment's own draft, kept when another is selected (Q1); Ctrl+Enter adds it,
        // as the button does. Asked before the box takes the key as a line break.
        let id = self.scope.with(("reply", a.id));
        let ctrl_enter = ui.ctx().memory(|m| m.has_focus(id))
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
        let draft = self.drafts.entry(a.id).or_insert(Draft {
            page: Some(a.page),
            text: String::new(),
        });
        draft.page = Some(a.page);
        let out = ui.add(
            egui::TextEdit::multiline(&mut draft.text)
                .id(id)
                .hint_text("Reply")
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        dialogs::set_name(&out, "Reply");
        if self.reply_focus {
            out.request_focus();
            self.reply_focus = false;
        }
        let ready = !draft.text.trim().is_empty();
        let add = ui.add_enabled(
            ready,
            egui::Button::new((theme::icon(icon::ARROW_BEND_UP_LEFT), "Add reply")),
        );
        dialogs::set_name(&add, "Add reply");
        if ready && (add.clicked() || ctrl_enter) {
            let text = std::mem::take(&mut draft.text);
            self.actions.push(Action::Reply(a.page, a.id, text));
        }
        if self.drafts.get(&a.id).is_some_and(|d| d.text.is_empty()) {
            self.drafts.remove(&a.id);
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
        dialogs::set_name(&entry, "Your own stamp");
        // What is typed is the stamp to put down straight away; it joins the list when
        // the box is left (Enter, or a click on the page or elsewhere).
        let wording = limarepdf_core::stamp_wording(&self.stamp_draft);
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
                let r = ui
                    .add(egui::TextEdit::singleline(&mut self.author).hint_text("No name"))
                    .on_hover_text(
                        "Also signs your comments (Review > Comments > Your name for comments)",
                    );
                dialogs::set_name(&r, "Your name");
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
    }

    /// The Create signature window, while open (from the Fill and sign menu or pane, or a
    /// click with nothing saved yet); what it makes is put down where that click was.
    pub fn sign_window(&mut self, ctx: &egui::Context) {
        if let Some((page, place)) =
            self.sign
                .create_ui(ctx, &mut self.sign_kit, &mut self.sign_error)
        {
            self.actions.push(Action::AddSign(page, place));
        }
    }

    /// Part of the inspector was drawn at `rect` this frame: the panel, its heading with
    /// its Collapse button too, its strip or its pop-out. A click anywhere in them doesn't
    /// end a line being edited. (Not the clip rect of the inspector's scrolling contents,
    /// which in the pop-out runs the window's width.)
    pub fn inspector_drawn(&mut self, ctx: &egui::Context, rect: Rect) {
        let pass = ctx.cumulative_pass_nr();
        let (was, at) = self.insp_rect;
        let rect = if at == pass { was.union(rect) } else { rect };
        self.insp_rect = (rect, pass);
    }

    pub fn inspector(&mut self, ui: &mut egui::Ui) {
        if self.selection.annot().is_some() {
            self.annot_inspector(ui);
            return;
        }
        self.sync_insp();
        let page = self.selection.objects().map(|(p, _)| p);
        let page_err = page.and_then(|p| self.unavailable.get(&p)).cloned();
        if let Some(e) = page_err {
            ui.colored_label(
                Color32::LIGHT_RED,
                format!("Editing is unavailable on this page: {e}"),
            );
        }
        // A tool's options and the selected object's properties are headed by the panel's
        // heading (`Editor::heading`); the object's again below a tool's options.
        match self.tool {
            // The look of the text being typed, or of the next, is changed in the band's
            // Font group ([`Self::font_band`]). A line edited with the tool shows its own.
            Tool::Text if self.adding_text() => {
                let rows = match self.adding.as_mut() {
                    Some(a) => {
                        let end = a.text.text().chars().count();
                        let sel = page_text::held(ui.ctx(), a.id()).unwrap_or((end, end));
                        let text = InLine {
                            text: &mut a.text,
                            sel,
                        };
                        let [font, size] = look_facts(&text);
                        [("Editing", "New text".to_string()), font, size]
                    }
                    None => [
                        ("Editing", "Next new text".into()),
                        ("Font", self.tfont.family_name()),
                        ("Size", format!("{} pt", size_text(self.tsize))),
                    ],
                };
                facts(ui, "new-text-look", &rows);
                // Words the fonts chosen lack, which go to an installed font.
                if let Some(a) = &self.adding {
                    let parts = a.text.drawn_parts();
                    let pieces: Vec<(FontChoice, &str)> = (parts.iter())
                        .map(|p| (a.text.font_of(p), p.text.as_str()))
                        .collect();
                    if let Some((said, why)) = page_text::fallback_note(&pieces) {
                        note(ui, &said).on_hover_text(why);
                    }
                }
                ui.separator();
            }
            Tool::Image => {
                if self.pending_image.is_none() {
                    ui.label("No image chosen.");
                }
                ui.separator();
            }
            Tool::Rect | Tool::Ellipse | Tool::Line => {
                if self.tool != Tool::Line {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.fill_on, "Fill");
                        color_button(ui, &mut self.fill, "Fill colour");
                    });
                }
                ui.horizontal(|ui| {
                    let name = if self.tool == Tool::Line {
                        self.stroke_on = true;
                        "Line colour"
                    } else {
                        ui.checkbox(&mut self.stroke_on, "Outline");
                        "Outline colour"
                    };
                    color_button(ui, &mut self.stroke, name);
                    let r = ui.add(
                        egui::DragValue::new(&mut self.width)
                            .range(0.25..=40.0)
                            .speed(0.1)
                            .suffix(" pt"),
                    );
                    dialogs::set_name(&r, "Line width");
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
                ui.separator();
            }
            Tool::Pen => {
                ui.horizontal(|ui| {
                    color_button(ui, &mut self.pen_color, "Pen colour");
                    let r = ui.add(
                        egui::DragValue::new(&mut self.pen_width)
                            .range(0.25..=40.0)
                            .speed(0.1)
                            .suffix(" pt"),
                    );
                    dialogs::set_name(&r, "Pen width");
                });
                return;
            }
            Tool::Note => {
                let was = self.note_color;
                ui.horizontal(|ui| {
                    ui.label("Colour");
                    color_button(ui, &mut self.note_color, "Note colour");
                });
                // The colour applies to the note being typed, which then takes the
                // keyboard back.
                if !same_color(self.note_color, was) {
                    if let Some(w) = self.writing.as_mut().filter(|w| w.note && w.id.is_none()) {
                        w.refocus = true;
                    }
                }
                return;
            }
            Tool::Stamp => {
                self.stamp_ui(ui);
                return;
            }
            Tool::Sign => {
                let (kit, author) = (&mut self.sign_kit, &mut self.author);
                let out = self.sign.ui(ui, self.scope, kit, author, self.sign_items);
                self.finish_signing |= out.finish;
                if let Some(e) = &self.sign_error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                return;
            }
            Tool::DigitalSign => {
                ui.label("Click an empty signature field, or drag a box.");
                if !self.digital_fields.is_empty() {
                    ui.add_space(6.0);
                    ui.label("Empty signature fields:");
                    for (name, page) in &self.digital_fields {
                        let text = format!(
                            "{} (page {})",
                            limarepdf_core::display_line(name, 40),
                            page + 1
                        );
                        if ui.button(text).clicked() {
                            self.digital_error = None;
                            self.digital_place = Some(SignPlace::Field(name.clone()));
                        }
                    }
                }
                if let Some(e) = &self.digital_error {
                    ui.add_space(4.0);
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                return;
            }
            Tool::Redact => {
                ui.label("Nothing is removed until the marks are applied.");
                ui.add_space(6.0);
                let n = self.redact_marks;
                ui.label(match n {
                    0 => "No areas are marked yet.".to_string(),
                    1 => "1 area is marked.".to_string(),
                    n => format!("{n} areas are marked."),
                });
                if ui
                    .add_enabled(n > 0, egui::Button::new("Apply redactions…"))
                    .clicked()
                {
                    self.apply_redactions = true;
                }
                return;
            }
            Tool::TextBox => {
                text_style_ui(ui, &mut self.box_style);
                // Words Helvetica lacks in a new text box, which go to an installed font.
                if let Some(w) = self.writing.as_ref().filter(|w| !w.note && w.id.is_none()) {
                    let helvetica = FontChoice::default();
                    if let Some((said, why)) = page_text::fallback_note(&[(helvetica, &w.text)]) {
                        note(ui, &said).on_hover_text(why);
                    }
                }
                return;
            }
            // Selected words show their look, several objects theirs; a selected object its
            // properties below.
            Tool::Select | Tool::Read if self.words_panel() => {
                self.words_ui(ui);
                return;
            }
            Tool::Select | Tool::Read if self.selection.object().is_none() && self.has_object() => {
                self.group_ui(ui);
                return;
            }
            Tool::Select | Tool::Read | Tool::Text => {}
        }

        // Below a tool's options, the object's own heading.
        let heading = self.tool_heading().and_then(|_| self.object_heading());
        let Some(insp) = self.insp.as_mut() else {
            return;
        };
        let (page, id) = insp.key;
        let Some(obj) = self.objs.get(&page).and_then(|v| v.get(id)).cloned() else {
            return;
        };
        if let Some(heading) = heading {
            ui.strong(heading);
        }

        // Changes are kept as they are made: each finished one is a step to undo.
        match obj.kind {
            ObjectKind::Text => {
                ui.add_space(4.0);
                // The words are typed on the page (a double-click, or Edit text), as in a
                // word processor, and their look changed in the band's Font group: the
                // panel says what they are.
                if obj.hidden {
                    note(ui, "Used by search and copy.");
                } else {
                    // What is selected in the line being edited on the page, else all of it.
                    let sel = match self.line_edit.as_ref() {
                        Some(le) => page_text::held(ui.ctx(), line_edit_id(self.scope, le.page))
                            .unwrap_or((0, 0)),
                        None => (0, insp.text.text().chars().count()),
                    };
                    let text = InLine {
                        text: &mut insp.text,
                        sel,
                    };
                    let [font, size] = look_facts(&text);
                    let fonts = insp.fonts.as_deref().unwrap_or(&[]);
                    let source = source_font(&obj, fonts, &insp.text);
                    // The selection's fonts as the PDF names them, without a subset's tag.
                    let names = pdf_fonts(&obj, sel);
                    let named: Vec<&str> = names.iter().map(|n| untagged(n)).collect();
                    let rows = [
                        ("Editing", "One line".to_string()),
                        font,
                        size,
                        ("PDF font", named.join(", ")),
                        ("Source font", source.into()),
                    ];
                    facts(ui, "line-look", &rows);
                    // Words that go to another font, or a font that can't take new text.
                    if let Some((said, why)) = font_note(&obj, fonts, &insp.text, sel) {
                        note(ui, &said).on_hover_text(why);
                    }
                    // Words the font chosen lacks, which go to an installed font.
                    let pieces = drawn_pieces(&obj, fonts, &insp.text);
                    let chosen: Vec<(FontChoice, &str)> = (pieces.iter())
                        .filter_map(|(p, w)| match w {
                            Wanted::Standard(f) => Some((*f, p.text.as_str())),
                            Wanted::Kept(..) => None,
                        })
                        .collect();
                    if let Some((said, why)) = page_text::fallback_note(&chosen) {
                        note(ui, &said).on_hover_text(why);
                    }
                }
            }
            ObjectKind::Path => {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut insp.fill_on, "Fill");
                    color_button(ui, &mut insp.fill, "Fill colour");
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut insp.stroke_on, "Outline");
                    color_button(ui, &mut insp.stroke, "Outline colour");
                    // A width outside the range (a hairline) is shown as it is: raised to
                    // the range, it would be a change never made.
                    let r = ui.add(
                        egui::DragValue::new(&mut insp.width)
                            .range(0.25..=100.0)
                            .clamp_existing_to_range(false)
                            .speed(0.1)
                            .suffix(" pt"),
                    );
                    dialogs::set_name(&r, "Outline width");
                });
            }
            ObjectKind::Image => {
                ui.add_space(4.0);
                if obj.inline {
                    note(ui, "Inline image: can't be replaced.");
                } else if theme::glyph_button(ui, icon::IMAGE, "Replace image…").clicked() {
                    if let Some(bytes) = pick_image() {
                        self.actions.push(Action::ReplaceImage(page, id, bytes));
                    }
                }
            }
            ObjectKind::Form => {}
        }

        // Position and size, in a section that starts closed and stays as it was left from
        // one object to the next; a field a row, as the section's indent leaves no room for
        // two.
        ui.add_space(4.0);
        let (mut x, mut y, mut w, mut h) = (
            insp.geom[0],
            insp.geom[1],
            insp.geom[2] - insp.geom[0],
            insp.geom[3] - insp.geom[1],
        );
        let geom_done = section(ui, section_name("Position and size"))
            .id_salt("position-and-size")
            .show(ui, |ui| {
                let mut done = false;
                egui::Grid::new("geom").num_columns(2).show(ui, |ui| {
                    for (label, v, lo) in [
                        ("X", &mut x, -5000.0),
                        ("Y", &mut y, -5000.0),
                        ("Width", &mut w, 1.0),
                        ("Height", &mut h, 1.0),
                    ] {
                        ui.label(label);
                        // A rule's height under 1 pt is shown as it is: raised, it would
                        // be made with the next change.
                        let field = egui::DragValue::new(v).range(lo..=20000.0);
                        let r = ui.add(field.clamp_existing_to_range(false).speed(0.5));
                        dialogs::set_name(&r, label);
                        done |= r.drag_stopped() || r.lost_focus();
                        ui.end_row();
                    }
                });
                done
            })
            .body_returned
            .unwrap_or(false);
        insp.geom = [x, y, x + w, y + h];
        if geom_done
            && insp
                .geom
                .iter()
                .zip(&obj.bbox)
                .any(|(a, b)| (a - b).abs() > 0.05)
        {
            self.actions
                .push(Action::SetBox(page, vec![(id, insp.geom)]));
        }

        ui.add_space(6.0);
        if theme::glyph_button(ui, icon::TRASH, "Delete").clicked() {
            // What was changed goes with it.
            self.insp = None;
            self.actions.push(Action::Delete(page, vec![id]));
        } else if settled(ui.ctx()) && self.line_edit.is_none() {
            // A line being edited on the page shows the change as it is made, and keeps
            // it with the typing when the editing finishes.
            self.commit_insp();
        }
        if obj.kind == ObjectKind::Text && !obj.hidden {
            self.planned_text_tools(ui);
        }
    }
}

/// What several objects are, for the inspector: "2 lines of text, 1 shape".
fn kinds_of<'a>(objs: impl Iterator<Item = &'a PageObject>) -> String {
    let mut counts: Vec<(ObjKind, usize)> = Vec::new();
    for o in objs {
        let kind = ObjKind::of(o);
        match counts.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, n)) => *n += 1,
            None => counts.push((kind, 1)),
        }
    }
    counts.sort_by_key(|(k, _)| match k {
        ObjKind::Text => 0,
        ObjKind::HiddenText => 1,
        ObjKind::Image => 2,
        ObjKind::Path => 3,
        ObjKind::Form => 4,
    });
    let said: Vec<String> = (counts.into_iter())
        .map(|(kind, n)| {
            let (one, many) = match kind {
                ObjKind::Text => ("line of text", "lines of text"),
                ObjKind::HiddenText => ("line of invisible text", "lines of invisible text"),
                ObjKind::Image => ("image", "images"),
                ObjKind::Path => ("shape", "shapes"),
                ObjKind::Form => ("group", "groups"),
            };
            format!("{n} {}", if n == 1 { one } else { many })
        })
        .collect();
    said.join(", ")
}

/// The look of the shapes among several objects selected: Fill and Outline, each on or off
/// for all of them (shown as mixed when they differ), a colour picked making it that
/// colour for all (and on), and one outline width for all (blank when they differ).
fn shapes_ui(ui: &mut egui::Ui, g: &mut Group) {
    const FILL: Swatch = Swatch {
        name: "Fill colour",
        letter: false,
        within: "in the shapes",
        offered: "In the shapes",
        all: "Make them all this colour",
    };
    const OUTLINE: Swatch = Swatch {
        name: "Outline colour",
        ..FILL
    };
    ui.horizontal(|ui| {
        let all = g.all(|l| l.fill_on);
        let mut on = all == Some(true);
        let r = ui.add(egui::Checkbox::new(&mut on, "Fill").indeterminate(all.is_none()));
        if r.changed() {
            g.set(|l| l.fill_on = on);
        }
        if let Some(c) = colors_button(ui, &g.colors(|l| l.fill), FILL) {
            g.set(|l| (l.fill_on, l.fill) = (true, c));
        }
    });
    ui.horizontal(|ui| {
        let all = g.all(|l| l.stroke_on);
        let mut on = all == Some(true);
        let r = ui.add(egui::Checkbox::new(&mut on, "Outline").indeterminate(all.is_none()));
        if r.changed() {
            g.set(|l| l.stroke_on = on);
        }
        if let Some(c) = colors_button(ui, &g.colors(|l| l.stroke), OUTLINE) {
            g.set(|l| (l.stroke_on, l.stroke) = (true, c));
        }
        // Widths compared to a hundredth of a point, as a change is.
        let widths = g.all(|l| (l.width * 100.0).round() as i64);
        let mut w = g.shapes.first().map_or(1.0, |(.., l)| l.width);
        // A hairline is shown as it is, as for one shape.
        let mut value = egui::DragValue::new(&mut w)
            .range(0.25..=100.0)
            .clamp_existing_to_range(false)
            .speed(0.1);
        value = match widths {
            Some(_) => value.suffix(" pt"),
            None => value.custom_formatter(|_, _| String::new()),
        };
        let r = ui.add(value);
        dialogs::set_name(&r, "Outline width");
        if r.changed() {
            g.set(|l| l.width = w);
        }
    });
}

/// A text box's text size and colour, fill and border.
fn text_style_ui(ui: &mut egui::Ui, s: &mut TextStyle) {
    ui.horizontal(|ui| {
        ui.label("Text");
        size_ui(ui, &mut s.size, 4.0..=200.0);
        if let Some(c) = text_color_button(ui, &[s.color]) {
            s.color = c;
        }
    });
    ui.horizontal(|ui| {
        let mut on = s.fill.is_some();
        if ui.checkbox(&mut on, "Background").changed() {
            s.fill = on.then_some([1.0, 1.0, 0.85]);
        }
        if let Some(c) = s.fill.as_mut() {
            color_button(ui, c, "Background colour");
        }
    });
    ui.horizontal(|ui| {
        ui.label("Border");
        let r = ui.add(
            egui::DragValue::new(&mut s.border)
                .range(0.0..=20.0)
                .clamp_existing_to_range(false)
                .speed(0.1)
                .suffix(" pt"),
        );
        dialogs::set_name(&r, "Border width");
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
    use limarepdf_core::Family;

    /// BUG-38: egui's pickers show a PDF colour as the page does, and give back the
    /// value they show.
    #[test]
    fn pickers_show_pdf_colours_as_the_page_does() {
        let colours = [
            [0.5, 0.5, 0.5],
            [0.2, 0.6, 0.9],
            [0.0; 3],
            [1.0; 3],
            [1.0, 0.0, 0.0],
        ];
        for c in colours {
            let [r, g, b] = to_linear(c);
            let shown = Color32::from(egui::Rgba::from_rgb(r, g, b));
            let page = rgb32(c);
            let near = |a: u8, b: u8| a.abs_diff(b) <= 1;
            let same =
                near(shown.r(), page.r()) && near(shown.g(), page.g()) && near(shown.b(), page.b());
            assert!(same, "{c:?}: {shown:?}, {page:?}");
            assert!(same_color(from_linear(to_linear(c)), c), "{c:?}");
        }
    }

    /// BUG-38: a colour button opened and closed leaves its colour exactly as it was.
    #[test]
    fn a_colour_only_looked_at_stays_as_it_was() {
        let ctx = egui::Context::default();
        let was = [0.3, 0.6, 0.9];
        let mut c = was;
        // Draws the button with `events`; returns where it is.
        let frame = |events: Vec<egui::Event>, c: &mut [f32; 3]| {
            let raw = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut at = Pos2::ZERO;
            let mut out = ctx.run_ui(raw, |ui| {
                at = color_button(ui, c, "Colour").rect.center();
            });
            // Nothing draws the frame.
            out.textures_delta.clear();
            at
        };
        let at = frame(vec![], &mut c);
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(vec![egui::Event::PointerMoved(at)], &mut c);
        frame(vec![press(true)], &mut c);
        frame(vec![press(false)], &mut c);
        for _ in 0..3 {
            frame(vec![], &mut c);
        }
        assert!(egui::Popup::is_any_open(&ctx), "the picker didn't open");
        assert_eq!(c, was);
    }

    /// Queue the changes that keep what [`Editor::pending`] finds, as the tab does before a
    /// command, and stop typing them.
    fn keep_all(ed: &mut Editor) {
        for piece in Typed::ALL {
            if let Some(held) = ed.keep(piece) {
                ed.done(held);
            }
        }
    }

    /// BUG-41: new text, and a note or text box, whose change failed come back to go on
    /// typing; ended again unchanged, they are left out, and changed, tried again. Put back
    /// only while nothing changed the document since.
    #[test]
    fn typing_whose_change_failed_comes_back() {
        let look = page_text::Look {
            font: FontChoice::default(),
            size: 12.0,
            color: [0.0; 3],
        };
        for piece in [Typed::Text, Typed::Writing] {
            let mut ed = Editor::default();
            let typed = |ed: &mut Editor, text: &str| match piece {
                Typed::Text => {
                    let a = ed.adding.get_or_insert_with(|| {
                        PageText::new(egui::Id::NULL, 0, (100.0, 100.0), look)
                    });
                    a.text.sync(text, None);
                }
                _ => {
                    let w = ed.writing.get_or_insert_with(|| {
                        Writing::new(0, true, [100.0, 100.0, 120.0, 120.0], 12.0)
                    });
                    w.text = text.into();
                }
            };
            let end = |ed: &mut Editor| match piece {
                Typed::Text => ed.finish_adding(),
                _ => ed.finish_writing(),
            };
            // Its change failed: back to go on with.
            typed(&mut ed, "Seal");
            end(&mut ed);
            assert_eq!(ed.actions.len(), 1, "{piece:?}");
            assert!(!ed.pending(), "{piece:?}: still open");
            ed.actions.clear();
            ed.settle_kept(&[(0, false)]);
            assert!(ed.pending() && !ed.left_out, "{piece:?}: not back");
            // Ended again as it is: left out, and said, not tried again.
            end(&mut ed);
            assert!(
                ed.actions.is_empty() && !ed.pending(),
                "{piece:?}: tried again"
            );
            assert!(std::mem::take(&mut ed.left_out), "{piece:?}: not said");
            // Changed after coming back: tried again.
            typed(&mut ed, "Seal");
            end(&mut ed);
            ed.actions.clear();
            ed.settle_kept(&[(0, false)]);
            typed(&mut ed, "Seals");
            end(&mut ed);
            assert_eq!(ed.actions.len(), 1, "{piece:?}: changed, not tried");
            assert!(!ed.left_out, "{piece:?}");
            // A change after it made meanwhile: its place may be another's, so it is left
            // out, not put back.
            ed.actions.clear();
            ed.settle_kept(&[(0, true)]);
            assert!(!ed.pending() && ed.left_out, "{piece:?}: put back");
            // Made: done with.
            ed.left_out = false;
            typed(&mut ed, "Walrus");
            end(&mut ed);
            ed.actions.clear();
            ed.settle_kept(&[]);
            assert!(!ed.pending() && !ed.left_out, "{piece:?}");
        }
    }

    #[test]
    fn comment_changes_and_replies_not_yet_kept_are_pending() {
        let mut pdf = limarepdf_core::Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        let id = pdf
            .add_note(0, (100.0, 100.0), [1.0, 1.0, 0.0], "Old text", "Tester")
            .unwrap();
        let mut ed = Editor::default();
        let a = ed.annots(&pdf, 0)[0].clone();
        ed.selection = Selection::Annot { page: 0, id };
        // Selected and shown in the inspector, unchanged: nothing to keep, even with its
        // colour stored back a hair off by the colour button.
        ed.annot_insp = Some((id, AnnotEdit::of(&a)));
        assert!(!ed.pending());
        if let Some((_, e)) = ed.annot_insp.as_mut() {
            e.color = e.color.map(|c| c.map(|v| v - 1e-6));
        }
        assert!(!ed.pending());
        // A changed comment, and a reply typed.
        if let Some((_, e)) = ed.annot_insp.as_mut() {
            e.contents = "Walrus comment".into();
        }
        assert!(ed.pending());
        ed.annot_insp = Some((id, AnnotEdit::of(&a)));
        let draft = |text: &str| Draft {
            page: Some(0),
            text: text.into(),
        };
        ed.drafts.insert(id, draft("Walrus reply"));
        assert!(ed.pending());
        if let Some((_, e)) = ed.annot_insp.as_mut() {
            e.contents = "Walrus comment".into();
        }
        keep_all(&mut ed);
        assert!(matches!(
            ed.actions.as_slice(),
            [Action::SetAnnot(0, i, e), Action::Reply(0, j, r)]
                if *i == id && e.contents == "Walrus comment" && *j == id && r == "Walrus reply"
        ));
        assert!(ed.draft(id).is_none());
        // A draft is its comment's, selected or not; one whose comment is gone (Undo took
        // it away) can't be added, so isn't pending.
        ed.actions.clear();
        ed.drafts.insert(id, draft("Walrus reply"));
        ed.selection = Selection::None;
        assert!(ed.pending());
        ed.place_drafts(&[None]);
        assert!(!ed.pending());
        assert_eq!(ed.draft(id), Some("Walrus reply"), "the draft went");
    }

    fn word(x: f32, y: f32, line: usize) -> PageWord {
        PageWord {
            text: "w".into(),
            bbox: [x, y, x + 20.0, y + 10.0],
            line,
            object: None,
            chars: (0, 1),
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

    /// Lines of a paragraph are shaded meeting halfway across the space between them; a
    /// gap between paragraphs, and lines side by side, are left as they are.
    #[test]
    fn shaded_lines_of_a_paragraph_meet() {
        let words = [
            word(0.0, 0.0, 0),
            word(0.0, 12.0, 1),
            word(0.0, 40.0, 2),
            word(100.0, 0.0, 3),
        ];
        assert_eq!(
            Editor::shaded(&words),
            [
                [0.0, 0.0, 20.0, 11.0],
                [0.0, 11.0, 20.0, 22.0],
                [0.0, 40.0, 20.0, 50.0],
                [100.0, 0.0, 120.0, 10.0],
            ]
        );
    }

    #[test]
    fn sizes_show_exactly_and_typed_ones_stay_in_range() {
        assert_eq!(size_text(12.0), "12");
        assert_eq!(size_text(10.98), "10.98");
        assert_eq!(size_text(10.5), "10.5");
        // Not rounded, to a listed size or to two places.
        assert_eq!(size_text(11.333), "11.333");
        assert_eq!(size_text(10.126), "10.126");
        let range = 1.0..=400.0;
        assert_eq!(parse_size(" 13.5 ", &range), Some(13.5));
        assert_eq!(parse_size("9pt", &range), Some(9.0));
        assert_eq!(parse_size("9 pt", &range), Some(9.0));
        assert_eq!(parse_size("1000", &range), Some(400.0));
        assert_eq!(parse_size("0", &range), Some(1.0));
        assert_eq!(parse_size("big", &range), None);
        assert_eq!(parse_size("NaN", &range), None);
    }

    /// Draws the size box for a frame with `events`, after putting the keyboard in it
    /// (`Some(true)`) or taking it away (`Some(false)`). Returns whether a size was chosen.
    fn size_frame(
        ctx: &egui::Context,
        size: &mut f32,
        focus: Option<bool>,
        events: Vec<egui::Event>,
    ) -> bool {
        let id = egui::Id::unique("size test");
        let field = ctx.data(|d| d.get_temp::<egui::Id>(id));
        match (focus, field) {
            (Some(true), Some(f)) => ctx.memory_mut(|m| m.request_focus(f)),
            (Some(false), Some(f)) => ctx.memory_mut(|m| m.surrender_focus(f)),
            _ => {}
        }
        let mut chosen = false;
        let raw = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            ui.push_id(id, |ui| {
                ui.data_mut(|d| d.insert_temp(id, ui.scope_id().with("size-box")));
                chosen = size_ui(ui, size, 1.0..=400.0);
            });
        });
        out.textures_delta.clear();
        chosen
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn the_size_box_changes_nothing_until_a_size_is_typed() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut size = 10.126_f32;
        let none = egui::Modifiers::NONE;
        let mut chosen = false;
        // Into the box and out again, twice, and Enter in it, without typing.
        for _ in 0..2 {
            chosen |= size_frame(&ctx, &mut size, None, vec![]);
            chosen |= size_frame(&ctx, &mut size, Some(true), vec![]);
            chosen |= size_frame(&ctx, &mut size, None, vec![]);
            chosen |= size_frame(&ctx, &mut size, Some(false), vec![]);
            chosen |= size_frame(&ctx, &mut size, None, vec![]);
        }
        chosen |= size_frame(&ctx, &mut size, Some(true), vec![]);
        chosen |= size_frame(&ctx, &mut size, None, vec![key(egui::Key::Enter, none)]);
        assert_eq!(size, 10.126);
        assert!(!chosen);
        // Text that isn't a size leaves it as it was.
        size_frame(&ctx, &mut size, Some(true), vec![]);
        size_frame(&ctx, &mut size, None, vec![egui::Event::Text("x".into())]);
        let c = size_frame(&ctx, &mut size, None, vec![key(egui::Key::Enter, none)]);
        assert_eq!(size, 10.126);
        assert!(!c);
        // A size typed is chosen once.
        size_frame(&ctx, &mut size, Some(true), vec![]);
        let all = key(egui::Key::A, egui::Modifiers::COMMAND);
        size_frame(
            &ctx,
            &mut size,
            None,
            vec![all, egui::Event::Text("11.5".into())],
        );
        let c = size_frame(&ctx, &mut size, None, vec![key(egui::Key::Enter, none)]);
        assert_eq!(size, 11.5);
        assert!(c);
        assert!(!size_frame(&ctx, &mut size, None, vec![]));
    }

    #[test]
    fn an_unusual_size_is_kept_exactly() {
        let (_, mut ed) = page_with_objects();
        let o = &mut ed.objs.get_mut(&0).unwrap()[2];
        o.font_size = Some(10.126);
        o.spans[0].font_size = 10.126;
        ed.select_object(0, 2);
        ed.sync_insp();
        assert!(!ed.pending());
        // A change smaller than a hundredth is still a change.
        ed.insp.as_mut().unwrap().text.set_size((0, 22), 10.13);
        keep_all(&mut ed);
        assert!(matches!(
            ed.actions.as_slice(),
            [Action::EditText(0, 2, te, false)] if te.parts[0].size == 10.13
        ));
    }

    #[test]
    fn a_double_click_picks_the_word_under_it() {
        let s = "Walrus ate the fish";
        assert_eq!(word_around(s, 0), (0, 6));
        assert_eq!(word_around(s, 3), (0, 6));
        assert_eq!(word_around(s, 8), (7, 10));
        // At the end, and past it.
        assert_eq!(word_around(s, 19), (15, 19));
        assert_eq!(word_around(s, 99), (15, 19));
        // Between two spaces: no word.
        assert_eq!(word_around("a  b", 2), (2, 2));
        assert_eq!(word_around("", 0), (0, 0));
        assert_eq!(word_spans(" Walrus  ate "), [(1, 7), (9, 12)]);
        assert!(word_spans("  ").is_empty());
        // Words by Unicode's rule (UAX #29), as Ctrl+arrows in the box go: punctuation is
        // apart from the word it follows, an apostrophe within one keeps it whole, and each
        // Chinese ideograph is a word.
        assert_eq!(word_around("Walrus, ate", 3), (0, 6));
        assert_eq!(word_around("Walrus, ate", 6), (0, 6));
        assert_eq!(word_around("Walrus, ate", 6 + 1), (6, 7));
        assert_eq!(word_around("can't swim", 2), (0, 5));
        assert_eq!(word_around("海象吃鱼", 1), (1, 2));
        assert_eq!(word_spans("Walrus, ate"), [(0, 6), (6, 7), (8, 11)]);
        assert_eq!(word_spans("海象 fish"), [(0, 1), (1, 2), (3, 7)]);
    }

    /// A page with a background over the whole page, a small rectangle and a line of text.
    fn page_with_objects() -> (limarepdf_core::Pdf, Editor) {
        let mut pdf = limarepdf_core::Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        let st = PathStyle {
            fill: Some([0.9, 0.9, 0.9]),
            stroke: None,
            line_width: 1.0,
        };
        pdf.add_shape(0, Shape::Rect, [0.0, 0.0, 612.0, 792.0], &st)
            .unwrap();
        pdf.add_shape(0, Shape::Rect, [100.0, 100.0, 200.0, 150.0], &st)
            .unwrap();
        let text = "Walrus like cold water";
        pdf.add_text(
            0,
            (100.0, 300.0),
            text,
            FontChoice::default(),
            12.0,
            [0.0; 3],
        )
        .unwrap();
        let mut ed = Editor::default();
        assert_eq!(ed.objects(&pdf, 0).len(), 3);
        (pdf, ed)
    }

    /// BUG-25: the spaces after a label's last word don't reach over the next label: a
    /// click between them is on neither, and editing the first doesn't hide the second.
    #[test]
    fn spaces_after_a_line_are_not_part_of_it() {
        let mut pdf = limarepdf_core::Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        for (at, text) in [(100.0, "Anrede      "), (145.0, "Titel")] {
            let font = FontChoice::default();
            pdf.add_text(0, (at, 300.0), text, font, 12.0, [0.0; 3])
                .unwrap();
        }
        let mut ed = Editor::default();
        let objs = ed.objects(&pdf, 0).to_vec();
        let [label, next] = [0, 1].map(|i| objs[i].bbox);
        let y = (label[1] + label[3]) / 2.0;
        assert_eq!(ed.text_hit(0, Pos2::new(142.0, y), 0.5), None, "{label:?}");
        assert_eq!(ed.text_hit(0, Pos2::new(147.0, y), 0.5), Some(1));
        assert!(cover_box(&objs[0])[2] < next[0], "{label:?} hides {next:?}");
    }

    #[test]
    fn reading_selects_pictures_and_shapes_but_not_backgrounds_or_text() {
        let (pdf, mut ed) = page_with_objects();
        let area = 612.0 * 792.0;
        let text = ed.objs[&0][2].bbox;
        let in_text = Pos2::new((text[0] + text[2]) / 2.0, (text[1] + text[3]) / 2.0);
        // The small rectangle, over the background.
        assert_eq!(ed.read_hit(0, Pos2::new(150.0, 120.0), 1.0, area), Some(1));
        // Only the background is there: the arrow takes it, reading doesn't.
        assert_eq!(ed.read_hit(0, Pos2::new(500.0, 700.0), 1.0, area), None);
        assert_eq!(ed.hit(0, Pos2::new(500.0, 700.0), 1.0), Some(0));
        // Page text isn't selected by a click, but a double-click edits it.
        assert_eq!(ed.read_hit(0, in_text, 1.0, area), None);
        assert_eq!(ed.text_hit(0, in_text, 1.0), Some(2));
        // The word double-clicked is found by the page's words: "cold".
        let words = ed.words(&pdf, 0).to_vec();
        let cold = words.iter().find(|w| w.text == "cold").unwrap().bbox;
        let at = Pos2::new(cold[2] - 1.0, (cold[1] + cold[3]) / 2.0);
        assert_eq!(ed.word_pick(0, 2, at, &words), Pick::Chars(12, 16));
        // Without the words: around where the click falls on the line as it is drawn.
        assert_eq!(ed.word_pick(0, 2, at, &[]), Pick::At(at, true));
    }

    #[test]
    fn reading_moves_all_but_page_text_unless_just_added() {
        let (_, mut ed) = page_with_objects();
        ed.select_object(0, 1);
        assert!(ed.object_moves());
        ed.select_object(0, 2);
        assert!(!ed.object_moves() && ed.has_object());
        ed.selection.set_loose(true);
        assert!(ed.object_moves());
        ed.selection.set_loose(false);
        ed.set_tool_keep_sel(Tool::Select);
        assert!(ed.object_moves());
    }

    /// The editor's inspector drawn once settled, as its widgets.
    fn inspector_tree(ed: &mut Editor) -> egui::accesskit::TreeUpdate {
        drawn_tree(ed, |ed, ui| ed.inspector(ui))
    }

    /// The band's Font group for `ed`'s selection, drawn alone.
    fn band_tree(ed: &mut Editor) -> egui::accesskit::TreeUpdate {
        drawn_tree(ed, |ed, ui| ed.font_band(ui, &|_| {}, false))
    }

    /// What `draw` draws of `ed`, alone in a window, settled.
    fn drawn_tree(
        ed: &mut Editor,
        draw: impl Fn(&mut Editor, &mut egui::Ui),
    ) -> egui::accesskit::TreeUpdate {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut tree = None;
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| draw(ed, ui));
            });
            out.textures_delta.clear();
            tree = out.platform_output.accesskit_update;
        }
        tree.expect("the widgets")
    }

    /// ENH-06: several objects selected show, in the inspector, how many and what they are,
    /// and the look of the lines of text and of the shapes among them, mixed or blank where
    /// they differ; a change to the shapes there is one change for every shape, each keeping
    /// what wasn't changed.
    #[test]
    fn several_objects_show_their_mixed_look() {
        let mut pdf = limarepdf_core::Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        let style = |fill, stroke: Option<[f32; 3]>, line_width| PathStyle {
            fill: Some(fill),
            stroke,
            line_width,
        };
        let (red, blue) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        let shapes = [
            (
                [100.0, 100.0, 200.0, 150.0],
                style(red, Some([0.0; 3]), 1.0),
            ),
            ([300.0, 100.0, 400.0, 150.0], style(blue, None, 1.0)),
            (
                [100.0, 200.0, 200.0, 250.0],
                style(blue, Some([0.0; 3]), 3.0),
            ),
        ];
        for (b, st) in shapes {
            pdf.add_shape(0, Shape::Rect, b, &st).unwrap();
        }
        let font = FontChoice::default();
        for (text, y, size) in [("Walrus", 300.0, 12.0), ("Seal", 350.0, 20.0)] {
            pdf.add_text(0, (100.0, y), text, font, size, [0.0; 3])
                .unwrap();
        }
        let mut ed = Editor::default();
        ed.set_tool(Tool::Select);
        ed.objects(&pdf, 0);
        ed.select_objects(0, 0..5);
        ed.sync_chosen(&pdf);
        assert_eq!(ed.heading(), "5 objects");
        let tree = inspector_tree(&mut ed);
        let named = |name: &str| {
            let found = tree.nodes.iter().find(|(_, n)| n.label() == Some(name));
            found.map(|(_, n)| n.clone())
        };
        let said = |text: &str| tree.nodes.iter().any(|(_, n)| n.value() == Some(text));
        assert!(said("2 lines of text, 3 shapes"));
        assert!(named("Fill colour (mixed)").is_some(), "red and blue");
        assert!(named("Outline colour").is_some(), "one outline colour");
        let outline = named("Outline").expect("the Outline box");
        assert_eq!(outline.toggled(), Some(egui::accesskit::Toggled::Mixed));
        let fill = named("Fill").expect("the Fill box");
        assert_eq!(fill.toggled(), Some(egui::accesskit::Toggled::True));
        let width = named("Outline width").expect("the width");
        assert_eq!(
            width.value().unwrap_or_default().trim(),
            "",
            "blank: 1 pt and 3 pt"
        );
        // Their text's size is mixed, and blank in Edit's Font group.
        assert!(said("Mixed"), "12 and 20");
        let band = band_tree(&mut ed);
        let size = (band.nodes.iter()).find(|(_, n)| n.label() == Some("Font size"));
        let size = size.and_then(|(_, n)| n.value()).expect("the lines' size");
        assert_eq!(size, "", "blank: 12 and 20");

        // Every shape's width at once: one change, each keeping its own fill.
        ed.group.as_mut().unwrap().set(|l| l.width = 2.0);
        assert!(ed.pending());
        keep_all(&mut ed);
        let [Action::PathStyle(0, styles)] = ed.actions.as_slice() else {
            panic!("{} changes", ed.actions.len());
        };
        let fills: Vec<_> = (styles.iter())
            .map(|(id, s)| (*id, s.fill, s.stroke.is_some(), s.line_width))
            .collect();
        let want = [
            (0, Some(red), true, 2.0),
            (1, Some(blue), false, 2.0),
            (2, Some(blue), true, 2.0),
        ];
        assert_eq!(fills, want);
        assert!(!ed.pending());
    }

    /// ENH-06: the text of several lines selected, copied, is in reading order (top to
    /// bottom, left to right along a row), whatever order they were selected in; and the
    /// invisible lines among several selected are counted as not restyled.
    #[test]
    fn several_lines_copy_in_reading_order_and_say_what_is_left() {
        let mut pdf = limarepdf_core::Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        let font = FontChoice::default();
        for (text, at) in [
            ("right", (300.0, 100.0)),
            ("second", (100.0, 140.0)),
            ("left", (100.0, 101.0)),
        ] {
            pdf.add_text(0, at, text, font, 12.0, [0.0; 3]).unwrap();
        }
        let mut ed = Editor::default();
        ed.set_tool(Tool::Select);
        ed.objects(&pdf, 0);
        ed.select_objects(0, [1, 0, 2]);
        assert_eq!(ed.selected_line_text(), ["left", "right", "second"]);
        // Invisible text among them: not restyled, and said.
        ed.objs.get_mut(&0).unwrap()[1].hidden = true;
        ed.chosen = None;
        ed.sync_chosen(&pdf);
        assert_eq!(
            ed.switch_style(&egui::Context::default(), &pdf, true),
            None,
            "the visible lines can change"
        );
        let [Action::Restyle(changes, left)] = ed.actions.as_slice() else {
            panic!("{} changes", ed.actions.len());
        };
        assert_eq!(changes.len(), 1);
        let ids: Vec<usize> = changes[0].1.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [0, 2]);
        assert_eq!(
            left.said().as_deref(),
            Some("Not changed: 1 line of invisible recognised text")
        );
    }

    #[test]
    fn inspector_changes_are_kept_on_clicking_away() {
        let (_, mut ed) = page_with_objects();
        ed.select_object(0, 2);
        ed.sync_insp();
        assert!(!ed.pending());
        // A colour stored back a hair off by the colour button isn't a change.
        ed.insp
            .as_mut()
            .unwrap()
            .text
            .set_color((0, 22), [1e-6, 0.0, 0.0]);
        assert!(!ed.pending());
        // Typed words: kept as one edit of the line when another object is selected.
        ed.selection = Selection::None;
        ed.sync_insp();
        ed.select_object(0, 2);
        ed.sync_insp();
        ed.insp.as_mut().unwrap().text.sync("Walruses", None);
        assert!(ed.pending());
        ed.select_object(0, 1);
        ed.sync_insp();
        assert!(matches!(
            ed.actions.as_slice(),
            [Action::EditText(0, 2, te, true)]
                if te.text == "Walruses" && te.parts[0].font == PartFont::Span(0)
        ));
        // A shape's new outline, kept when nothing is selected any more.
        ed.actions.clear();
        ed.insp.as_mut().unwrap().stroke_on = true;
        ed.selection = Selection::None;
        ed.sync_insp();
        assert!(matches!(
            ed.actions.as_slice(),
            [Action::PathStyle(0, styles)] if matches!(styles[..], [(1, st)] if st.stroke.is_some())
        ));
        assert!(ed.insp.is_none() && !ed.pending());
        // A size picked for the line: a change of size only.
        ed.actions.clear();
        ed.select_object(0, 2);
        ed.sync_insp();
        ed.insp.as_mut().unwrap().text.set_size((0, 22), 18.0);
        keep_all(&mut ed);
        assert!(matches!(
            ed.actions.as_slice(),
            [Action::EditText(0, 2, te, false)]
                if te.text == "Walrus like cold water"
                    && te.parts.len() == 1
                    && te.parts[0].size == 18.0
                    && te.parts[0].color == [0.0; 3]
        ));
    }

    #[test]
    fn escape_keeps_what_was_typed_on_the_page() {
        let (_, mut ed) = page_with_objects();
        ed.start_line_edit(0, 2, Pick::Chars(0, 6));
        assert!(ed.typing_on_page());
        ed.insp.as_mut().unwrap().text.sync("Seal", None);
        ed.escape();
        assert!(!ed.typing_on_page() && !ed.pending());
        assert!(matches!(
            ed.actions.as_slice(),
            [Action::EditText(0, 2, te, true)] if te.text == "Seal"
        ));
        // With the read tool, the line isn't left selected.
        assert_eq!(ed.sel(), None);
        // With the arrow it stays selected; Esc again clears it.
        ed.set_tool(Tool::Select);
        ed.start_line_edit(0, 2, Pick::Chars(0, 6));
        ed.escape();
        assert_eq!(ed.sel(), Some((0, 2)));
        ed.escape();
        assert_eq!(ed.sel(), None);
        // Nothing typed: Esc only closes the editor.
        ed.actions.clear();
        ed.start_line_edit(0, 2, Pick::Chars(0, 6));
        ed.escape();
        assert!(!ed.typing_on_page() && ed.actions.is_empty());
    }

    /// A line set in an installed family (by LimarePDF or another app) shows as that family,
    /// once the installed fonts are read; a standard font stays one.
    #[test]
    fn a_line_in_an_installed_family_shows_as_that_family() {
        let serif = crate::font_list::test_family("Source Serif 4");
        limarepdf_core::typefaces();
        let f = font_of("QWERTY+SourceSerif4-Regular");
        assert_eq!((f.family, f.bold, f.italic), (serif, false, false));
        assert_eq!(font_of("Times-Bold"), FontChoice::from_name("Times-Bold"));
        assert_eq!(font_of("Arial,Bold"), FontChoice::from_name("Arial,Bold"));
    }

    /// The inspector says which characters typed the line's font has no code for, whose
    /// words go to another font; or that the font can't take new text at all, and why on
    /// hover (batch 31, Q5).
    #[test]
    fn the_inspector_says_what_the_line_s_font_can_t_write() {
        use lopdf::{dictionary, Dictionary, Object, Stream};
        let (pdf, ed) = page_with_objects();
        let o = ed.objs[&0][2].clone();
        let fonts = pdf.line_fonts(0, 2).unwrap();
        let mut text = Styled::of(&o, FontChoice::from_name);
        assert_eq!(font_note(&o, &fonts, &text, (0, 0)), None);
        text.sync("Walrus like größe Maße", None);
        let (said, why) = font_note(&o, &fonts, &text, (0, 0)).unwrap();
        let want = "Not in this PDF's font: ö ß. Those words use Helvetica.";
        assert_eq!(said, want);
        assert_eq!(why, "Helvetica");

        // A subset the PDF doesn't hold can't take any.
        let mut pdf = limarepdf_core::Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        let font = pdf.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "ABCDEF+Garamond-Italic",
            "Encoding" => "WinAnsiEncoding",
        });
        let content = b"BT /F1 20 Tf 72 700 Td (Walrus) Tj ET".to_vec();
        let s = pdf.doc.add_object(Stream::new(Dictionary::new(), content));
        let page = pdf.page_ids()[0];
        let d = pdf.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", Object::Reference(s));
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
        let o = pdf.page_objects(0).unwrap().remove(0);
        let fonts = pdf.line_fonts(0, o.id).unwrap();
        let text = Styled::of(&o, FontChoice::from_name);
        let used = FontChoice::from_name("ABCDEF+Garamond-Italic").family_name();
        let (said, why) = font_note(&o, &fonts, &text, (0, 0)).unwrap();
        let want = format!("This PDF's font can't take new text. Edits use {used}.");
        assert_eq!(said, want);
        assert_eq!(why, "Garamond-Italic: Not embedded");
        // Typed letters it lacks say which first.
        let mut typed = text.clone();
        typed.sync("Walrus ö", None);
        let (said, _) = font_note(&o, &fonts, &typed, (0, 0)).unwrap();
        assert!(said.starts_with("Not in this PDF's font: ö."), "{said}");
    }

    /// A line's fonts are read with all the page's lines the first time one of them is
    /// selected, and kept until the document changes: selecting another line of the page
    /// doesn't read the page again (on a page with heavy content, or fonts of two-byte
    /// codes, that took a tenth to a third of a second each time).
    #[test]
    fn a_page_s_line_fonts_are_read_once_until_the_document_changes() {
        let mut pdf = limarepdf_core::Pdf::new();
        pdf.insert_blank(0, 612.0, 792.0).unwrap();
        for (y, text) in [(300.0, "Walrus like it cold"), (200.0, "Otters too")] {
            pdf.add_text(0, (100.0, y), text, FontChoice::default(), 12.0, [0.0; 3])
                .unwrap();
        }
        let mut ed = Editor::default();
        assert_eq!(ed.objects(&pdf, 0).len(), 2);
        let fonts_of = |ed: &mut Editor, pdf: &limarepdf_core::Pdf, id: usize| {
            ed.select_object(0, id);
            ed.sync_insp();
            ed.read_line_fonts(pdf, 0);
            ed.insp.as_ref().unwrap().fonts.clone().unwrap()
        };
        let takes = |f: &[LineFont]| f.len() == 1 && f[0].takes_new_text();
        let first = fonts_of(&mut ed, &pdf, 0);
        assert!(takes(&first), "{first:?}");
        // The page's fonts taken away behind the editor's back: the next line's fonts are
        // those read with the first, not read from the page again.
        let page = pdf.doc.get_dictionary_mut(pdf.page_ids()[0]).unwrap();
        page.remove(b"Resources");
        assert!(pdf.line_fonts(0, 1).unwrap()[0].refused() == Some(FontRefusal::Unreadable));
        let second = fonts_of(&mut ed, &pdf, 1);
        assert!(takes(&second), "{second:?}");
        // A change to the document forgets them.
        ed.invalidate();
        assert!(ed.line_fonts.is_empty());
    }

    /// The editor draws a line in the font core will write it in.
    #[test]
    fn a_line_is_edited_in_the_font_it_will_be_written_in() {
        let (pdf, mut ed) = page_with_objects();
        let o = ed.objs[&0][2].clone();
        // Shown as Arial Bold, in the line's own font (an unembedded one, whose plain ASCII
        // is written as it is).
        let mut kept = o.clone();
        kept.font_name = Some("Arial,Bold".into());
        kept.spans[0].font_name = "Arial,Bold".into();
        ed.objs.get_mut(&0).unwrap()[2] = kept.clone();
        ed.select_object(0, 2);
        ed.sync_insp();
        let fonts = pdf.line_fonts(0, 2).unwrap();
        ed.insp.as_mut().unwrap().fonts = Some(fonts.clone());
        let font = |ed: &Editor, o: &PageObject| {
            let insp = ed.insp.as_ref().unwrap();
            let fonts = insp.fonts.as_deref().unwrap_or(&[]);
            match drawn_pieces(o, fonts, &insp.text)[0].1 {
                Wanted::Kept(name, _) => format!("Kept({name:?})"),
                Wanted::Standard(f) => format!("Standard({f:?})"),
            }
        };
        // Its own font, when core can keep it: its own text, or plain ASCII.
        assert_eq!(font(&ed, &kept), "Kept(\"Arial,Bold\")");
        fn text(ed: &mut Editor) -> &mut Styled {
            &mut ed.insp.as_mut().unwrap().text
        }
        text(&mut ed).sync("Walrus 42", None);
        assert_eq!(font(&ed, &kept), "Kept(\"Arial,Bold\")");
        // A size or colour changed: still its own, as each part has its size and colour.
        text(&mut ed).set_size((0, 9), 20.0);
        assert_eq!(font(&ed, &kept), "Kept(\"Arial,Bold\")");
        // A word with a character the font can't take, a font that can't be kept, or
        // another font chosen: the standard font nearest it, or the one chosen.
        let arial_bold = format!("Standard({:?})", FontChoice::from_name("Arial,Bold"));
        text(&mut ed).sync("Café", None);
        assert_eq!(font(&ed, &kept), arial_bold);
        text(&mut ed).sync("Walrus", None);
        ed.insp.as_mut().unwrap().fonts = Some(Vec::new());
        assert_eq!(font(&ed, &kept), arial_bold);
        ed.insp.as_mut().unwrap().fonts = Some(fonts);
        text(&mut ed).italic((0, 6));
        text(&mut ed).set_family((0, 6), Family::Courier);
        let courier = FontChoice {
            family: Family::Courier,
            bold: true,
            italic: true,
        };
        assert_eq!(font(&ed, &kept), format!("Standard({courier:?})"));
        // Turned and invisible lines are edited in a plain box, and nothing hides them.
        assert!(drawn_in_place(&o));
        for plain in [
            PageObject {
                upright: false,
                ..o.clone()
            },
            PageObject {
                hidden: true,
                ..o.clone()
            },
        ] {
            assert!(!drawn_in_place(&plain));
        }
        ed.start_line_edit(0, 2, Pick::Chars(0, 0));
        let (page, b) = ed.cover_wanted().expect("a box to hide");
        assert_eq!(page, 0);
        // The box with room for accents and descenders.
        let ob = o.bbox;
        assert!(
            b[0] < ob[0] && b[1] < ob[1] && b[2] > ob[2] && b[3] > ob[3],
            "{b:?}"
        );
        ed.objs.get_mut(&0).unwrap()[2].upright = false;
        assert_eq!(ed.cover_wanted(), None);
    }

    /// The text colour button reads as one, as in word processors: a letter A over a bar of
    /// the text's colour, the colours side by side where the text mixes them, in both
    /// themes (a black swatch alone was lost on the dark theme).
    #[test]
    fn the_text_colour_button_is_a_letter_over_its_colour() {
        let red = [1.0, 0.0, 0.0];
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            for colors in [vec![[0.0; 3]], vec![[0.0; 3], red]] {
                let ctx = egui::Context::default();
                crate::theme::install(&ctx);
                ctx.set_theme(theme);
                let mut button = Rect::NOTHING;
                let mut panel = Color32::TRANSPARENT;
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    panel = ui.visuals().panel_fill;
                    button = ui.scope(|ui| text_color_button(ui, &colors)).response.rect;
                });
                out.textures_delta.clear();
                // How far apart two colours are in lightness, 0 to 1.
                let light = |c: Color32| (c.r() as f32 + c.g() as f32 + c.b() as f32) / 765.0;
                let stands_out = |c: Color32| (light(c) - light(panel)).abs() > 0.3;
                let inside = |r: Rect| button.expand(1.0).contains_rect(r);
                let letter = out.shapes.iter().find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == "A" => Some(t),
                    _ => None,
                });
                let letter = letter.unwrap_or_else(|| panic!("{theme:?}: no A"));
                assert!(inside(letter.visual_bounding_rect()), "{theme:?}");
                let ink = letter.fallback_color;
                assert!(stands_out(ink), "{theme:?}: the A {ink:?} on {panel:?}");
                // A bar of each colour under the A, side by side.
                let bars: Vec<Rect> = colors
                    .iter()
                    .map(|&c| {
                        let bar = out.shapes.iter().find_map(|s| match &s.shape {
                            egui::Shape::Rect(r) if r.fill == rgb32(c) && inside(r.rect) => {
                                Some(r.rect)
                            }
                            _ => None,
                        });
                        bar.unwrap_or_else(|| panic!("{theme:?}: no bar of {c:?}"))
                    })
                    .collect();
                for bar in &bars {
                    assert!(
                        bar.height() <= 5.0,
                        "{theme:?}: a swatch, not a bar: {bar:?}"
                    );
                    assert!(bar.top() >= letter.visual_bounding_rect().center().y);
                }
                if let [a, b] = bars[..] {
                    assert!(a.right() <= b.left() + 0.5, "not side by side: {a:?} {b:?}");
                }
                // Outlined, so a black bar shows on the dark theme.
                let all = bars.iter().fold(Rect::NOTHING, |u, b| u.union(*b));
                let outlined = out.shapes.iter().any(|s| match &s.shape {
                    egui::Shape::Rect(r) => {
                        r.rect.expand(0.5).contains_rect(all)
                            && r.rect.height() < all.height() + 4.0
                            && r.stroke.width > 0.0
                            && stands_out(r.stroke.color)
                    }
                    _ => false,
                });
                assert!(outlined, "{theme:?}: the bar has no outline");
            }
        }
    }

    /// ENH-04: the font list names the PDF's fonts of the characters a style change would
    /// change: those selected, or the one before a caret.
    #[test]
    fn the_font_list_names_the_pdf_fonts_of_the_selected_characters() {
        use limarepdf_core::TextSpan;
        let (_, ed) = page_with_objects();
        let mut o = ed.objs[&0][2].clone();
        let first = o.spans[0].clone();
        o.spans = vec![
            TextSpan {
                chars: (0, 7),
                font_name: "ABCDEF+C059-Roman".into(),
                ..first.clone()
            },
            TextSpan {
                chars: (7, 10),
                font_name: "Helvetica".into(),
                ..first.clone()
            },
            TextSpan {
                chars: (10, 15),
                font_name: "ABCDEF+C059-Roman".into(),
                ..first
            },
        ];
        let c059 = "ABCDEF+C059-Roman";
        let both = [c059, "Helvetica"];
        assert_eq!(pdf_fonts(&o, (0, 15)), both);
        assert_eq!(pdf_fonts(&o, (8, 2)), both, "selected backwards");
        assert_eq!(pdf_fonts(&o, (7, 10)), ["Helvetica"]);
        // At a caret, the character before it; at the start, the first.
        assert_eq!(pdf_fonts(&o, (7, 7)), [c059]);
        assert_eq!(pdf_fonts(&o, (8, 8)), ["Helvetica"]);
        assert_eq!(pdf_fonts(&o, (0, 0)), [c059]);
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
