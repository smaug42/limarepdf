//! Content editing state and interaction: selecting, moving and resizing objects, inserting text, images
//! and shapes, and the inspector panel.

use crate::dialogs;
use crate::fields::FormUi;
use crate::page_text::{self, PageText};
use crate::selection::{ObjKind, Selection, WordAt};
use crate::signing::{SignKit, SignPane, SignPick};
use crate::theme;
use crate::typing_fonts::{self, Wanted};
use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use egui_phosphor::regular as icon;
use limarepdf_core::page_colours::PageColours;
use limarepdf_core::{
    words_text, Annot, AnnotKind, Family, FieldValue, FontChoice, Link, LinkTarget, ObjectKind,
    PageObject, PageWord, PathStyle, Shape, ShapeLook, SignPlace, StampColor, StampKind,
    StampPlace, TextEdit, TextStyle, MIN_SIGNATURE_BOX,
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
    /// drawing takes several strokes.
    pub const PLACING: [Tool; 10] = [
        Tool::Text,
        Tool::Image,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Line,
        Tool::Note,
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
            Tool::Note => "note",
            Tool::TextBox => "text_box",
            Tool::Stamp => "stamp",
            Tool::Redact => "redact",
            Tool::Sign => "fill_and_sign",
            Tool::Read | Tool::Select | Tool::Pen | Tool::DigitalSign => "",
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
    SetBox(usize, usize, [f32; 4]),
    Delete(usize, usize),
    AddShape(usize, Shape, [f32; 4]),
    /// New text, its lines separated by `\n`, from the first line's baseline at the point
    /// given, set as typed.
    AddText(usize, (f32, f32), String, page_text::Look),
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

impl Insp {
    /// The properties of object `o`, at `key`, as they are.
    fn of(key: (usize, usize), o: &PageObject) -> Self {
        let font = FontChoice::from_name(o.font_name.as_deref().unwrap_or(""));
        let size = o.font_size.unwrap_or(12.0);
        let color = if o.kind == ObjectKind::Text {
            o.fill.unwrap_or([0.0; 3])
        } else {
            [0.0; 3]
        };
        Insp {
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
        }
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
    let id = ui.id().with("size-box");
    // What was typed, kept only once something has been.
    let typed = ui.data(|d| d.get_temp::<String>(id));
    let mut text = typed.clone().unwrap_or_else(|| size_text(*size));
    let r = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .desired_width(52.0),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Font size"));
    let edited = typed.is_some() || r.changed();
    let mut chosen = false;
    if r.lost_focus() && edited {
        if let Some(v) = parse_size(&text, &range).filter(|v| *v != *size) {
            chosen = true;
            *size = v;
        }
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
                let on = v == *size;
                if ui.selectable_label(on, size_text(v)).clicked() {
                    chosen |= !on;
                    *size = v;
                    ui.close();
                }
            }
        })
        .0
        .on_hover_text("Font sizes")
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Font sizes"));
    ui.label("pt");
    chosen
}

/// Whether two colours are the same to the eye. The colour buttons store a colour back
/// slightly changed by converting it, so exact comparison would see a change never made.
fn same_color(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1.0 / 512.0)
}

/// Whether a change made in the inspector is finished: no list or colour picker is open,
/// no button is held and nothing is being typed. Changes are kept then, one undo step each.
fn settled(ctx: &egui::Context) -> bool {
    !egui::Popup::is_any_open(ctx)
        && !ctx.input(|i| i.pointer.any_down())
        && !ctx.egui_wants_keyboard_input()
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
    refused: page_text::Refused,
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

/// The change the inspector's working copy makes to its line of text, as core is asked it.
fn text_change(insp: &Insp) -> TextEdit {
    TextEdit {
        text: insp.text.replace(['\n', '\r'], " "),
        font: (insp.font != insp.orig_font).then_some(insp.font),
        size: (insp.size != insp.orig_size).then_some(insp.size),
        color: (!same_color(insp.color, insp.orig_color)).then_some(insp.color),
    }
}

/// The font line `o` changed by `te` is written in, as core decides it: its own when it
/// can be kept, else a standard font (the one chosen, or the nearest to its own).
fn line_font<'a>(o: &'a PageObject, te: &TextEdit) -> Wanted<'a> {
    let name = o.font_name.as_deref().unwrap_or("");
    if te.keeps_font(o.font_reusable) {
        Wanted::Kept(name)
    } else {
        Wanted::Standard(te.font.unwrap_or_else(|| FontChoice::from_name(name)))
    }
}

/// Whether line `o` is edited drawn as it will be written; turned lines and invisible
/// (OCR) ones are edited in a plain box.
fn drawn_in_place(o: &PageObject) -> bool {
    o.upright && !o.hidden && o.baseline.is_some()
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

/// The characters (from, to) of the word at character `i` of `s`; empty between words.
fn word_around(s: &str, i: usize) -> (usize, usize) {
    let chars: Vec<char> = s.chars().collect();
    let i = i.min(chars.len());
    let word = |c: &char| !c.is_whitespace();
    let mut a = i;
    while a > 0 && word(&chars[a - 1]) {
        a -= 1;
    }
    let mut b = i;
    while b < chars.len() && word(&chars[b]) {
        b += 1;
    }
    (a, b)
}

/// The characters (from, to) of each word of `s`, in order.
fn word_spans(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in s.chars().enumerate() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(a)) => {
                out.push((a, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(a) = start {
        out.push((a, s.chars().count()));
    }
    out
}

pub struct Editor {
    pub tool: Tool,
    /// Pages, an object or words; see [`Selection`].
    pub selection: Selection,
    objs: HashMap<usize, Vec<PageObject>>,
    unavailable: HashMap<usize, String>,
    drag: Option<Drag>,
    draw: Option<Draw>,
    /// New text being typed on the page (Add text).
    pub adding: Option<PageText>,
    /// Put the keyboard in the inspector's text box for the selected text object.
    focus_text: bool,
    /// The note about a character that box refused.
    text_refused: page_text::Refused,
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
    /// A line of page text being edited on the page (read tool).
    line_edit: Option<LineEdit>,
    /// Page text just added, which the read tool lets move and resize until something
    /// else is selected; other page text moves only with the arrow.
    pub loose: Option<(usize, usize)>,
    /// The placing tools kept active after putting something down, each pinned on its own
    /// (from the settings).
    pub keep: Vec<Tool>,
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
            adding: None,
            focus_text: false,
            text_refused: page_text::Refused::default(),
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
            line_edit: None,
            loose: None,
            keep: Vec::new(),
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
            reply: String::new(),
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
        let Some(w) = self.writing.take().filter(Writing::changed) else {
            return;
        };
        let r = w.rect;
        self.actions.push(match w.id {
            Some(id) => Action::AnnotText(w.page, id, w.text),
            None if w.note => {
                Action::AddNote(w.page, ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0), w.text)
            }
            None => Action::AddTextBox(w.page, r, w.text),
        });
    }

    /// Keep the new text being typed, if any: added as one undo step.
    pub fn finish_adding(&mut self) {
        let Some(a) = self.adding.take().filter(PageText::has_text) else {
            return;
        };
        let look = self.new_look();
        self.actions
            .push(Action::AddText(a.page, a.at, a.text, look));
    }

    /// How new text is set, from the inspector.
    fn new_look(&self) -> page_text::Look {
        page_text::Look {
            font: self.tfont,
            size: self.tsize,
            color: self.tcolor,
        }
    }

    /// Whether something typed isn't in the document yet: new text, a note or text box
    /// being typed on the page, a form field being typed in, the selected comment's
    /// changes in the inspector, or a reply to it not yet added. Saving keeps it first,
    /// and closing asks, as for unsaved changes.
    pub fn pending(&self) -> bool {
        self.adding.as_ref().is_some_and(PageText::has_text)
            || self.writing.as_ref().is_some_and(Writing::changed)
            || self.form.typing()
            || self.annot_changes().is_some()
            || self.reply_to().is_some()
            || self.insp_change().is_some()
    }

    /// Queue the changes that keep what [`Self::pending`] finds, as clicking away and Add
    /// reply would.
    pub fn keep_pending(&mut self) {
        self.finish_adding();
        self.finish_writing();
        self.commit_insp();
        self.form.commit_typing(&mut self.actions);
        if let Some(a) = self.annot_changes() {
            self.actions.push(a);
        }
        if let Some(a) = self.reply_to() {
            self.reply.clear();
            self.actions.push(a);
        }
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
                let te = text_change(insp);
                let same = obj.text.as_deref() == Some(te.text.as_str())
                    && te.font.is_none()
                    && te.size.is_none()
                    && te.color.is_none();
                (!same).then_some(Action::EditText(page, id, te))
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
                (!same).then_some(Action::PathStyle(page, id, st))
            }
            _ => None,
        }
    }

    /// Keep the selected object's changes (see [`Self::insp_change`]) as one undo step. The
    /// working copy starts again from the object.
    fn commit_insp(&mut self) {
        if let Some(a) = self.insp_change() {
            self.actions.push(a);
            self.insp = None;
            self.line_edit = None;
        }
    }

    /// Edit a line of page text on the page, with `pick` selected.
    fn start_line_edit(&mut self, page: usize, id: usize, pick: Pick) {
        self.select_object(page, id);
        self.sync_insp();
        self.cover = None;
        self.line_edit = Some(LineEdit {
            page,
            id,
            pick,
            fresh: true,
            refused: page_text::Refused::default(),
        });
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
        let under = on.iter().position(|w| p.x >= w.bbox[0] && p.x <= w.bbox[2]);
        match under.filter(|_| on.len() == spans.len()) {
            Some(j) => Pick::Chars(spans[j].0, spans[j].1),
            None => Pick::At(p, true),
        }
    }

    /// Whether a line of page text is being edited, or new text typed, on the page.
    pub fn typing_on_page(&self) -> bool {
        self.line_edit.is_some() || self.adding.is_some()
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
        let mut look = self.new_look();
        let Some(a) = self.adding.as_mut().filter(|a| a.page == page) else {
            return false;
        };
        let shown = a.ui(ui, r, k, &mut look, self.page_colours, accent, settled);
        // Ctrl+B and Ctrl+I change the inspector's choice, as its buttons do.
        self.tfont = look.font;
        if shown.done {
            self.finish_adding();
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
        let (Some(le), Some(insp)) = (self.line_edit.as_mut(), self.insp.as_mut()) else {
            return false;
        };
        let ctx = ui.ctx().clone();
        let to_screen = |x: f32, y: f32| Pos2::new(r.min.x + x * k, r.min.y + y * k);
        let id = egui::Id::new(("line-edit", page));
        let drawn = drawn_in_place(&o);
        if ctx.memory(|m| m.has_focus(id)) {
            // Before the editor sees the keys: what can't be written isn't typed, and
            // Ctrl+B and Ctrl+I switch the whole line, as the inspector's buttons do.
            let t = page_text::take_typing(ui, true, !o.hidden);
            insp.font.bold ^= t.bold;
            insp.font.italic ^= t.italic;
            if t.refused {
                le.refused.mark(&ctx);
            }
        }
        let b = o.bbox;
        let (out, area) = if drawn {
            let family = typing_fonts::family(&ctx, line_font(&o, &text_change(insp)));
            let style = page_text::Style {
                font: egui::FontId::new((insp.size * k).max(1.0), family),
                color: page_text::shown(self.page_colours, insp.color),
                mode: self.page_colours,
                accent: ui.visuals().selection.stroke.color,
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
                &mut insp.text,
                to_screen(x, y),
                &style,
                false,
                cover,
                false,
            );
            (f.out, f.outline)
        } else {
            let area = Rect::from_min_max(to_screen(b[0], b[1]), to_screen(b[2], b[3])).expand(2.0);
            // Light colours in either theme, as for text boxes: the text sits on the paper.
            let font = egui::FontId::proportional((area.height() * 0.75).clamp(6.0, 96.0));
            let out = egui::Area::new(id.with("area"))
                .fixed_pos(area.min)
                .order(egui::Order::Foreground)
                .show(&ctx, |ui| {
                    *ui.visuals_mut() = egui::Visuals::light();
                    egui::TextEdit::singleline(&mut insp.text)
                        .id(id)
                        .font(font)
                        .min_size(area.size())
                        .desired_width(area.width())
                        .clip_text(false)
                        .margin(egui::Margin::same(2))
                        .show(ui)
                })
                .inner;
            (out, area)
        };
        let edit = &out.response.response;
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
                        let n = insp.text.chars().count();
                        let at = ((p.x - b[0]) / (b[2] - b[0]).max(0.01)).clamp(0.0, 1.0);
                        (at * n as f32).round() as usize
                    };
                    if word {
                        word_around(&insp.text, i)
                    } else {
                        (i, i)
                    }
                }
            };
            if let Some(mut st) = egui::TextEdit::load_state(&ctx, id) {
                let range = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(from),
                    egui::text::CCursor::new(to),
                );
                st.cursor.set_char_range(Some(range));
                st.clear_undoer();
                st.store(&ctx, id);
            }
            edit.request_focus();
            le.fresh = false;
            return true;
        }
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if esc && (edit.lost_focus() || edit.has_focus()) {
            // Esc keeps what was typed, as a click away does; Ctrl+Z takes it back.
            self.line_edit = None;
            self.commit_insp();
            return true;
        }
        // Scrolled out of view, or the keyboard went elsewhere: keep what was typed. A click
        // that ended it still selects what it was on.
        if !edit.has_focus() || !ui.clip_rect().intersects(area) {
            self.line_edit = None;
            self.commit_insp();
            return false;
        }
        edit.hovered()
    }

    /// The reply typed to the selected comment, as the change that adds it.
    fn reply_to(&self) -> Option<Action> {
        let a = self.selected_annot()?;
        let typed = !self.reply.trim().is_empty();
        typed.then(|| Action::Reply(a.page, a.id, self.reply.clone()))
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
        self.pen_stroke = None;
        self.text_drag = false;
        self.adding = None;
        self.writing = None;
        self.pending_image = None;
        self.insp = None;
        self.line_edit = None;
        self.loose = None;
        self.annot_insp = None;
        self.digital_place = None;
        self.digital_error = None;
        self.actions.clear();
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
        self.loose = self.sel();
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

    pub fn objects(&mut self, pdf: &limarepdf_core::Pdf, page: usize) -> &[PageObject] {
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

    /// Shade the selected words of `page` (`words`) in `accent`.
    fn paint_words(
        &self,
        painter: &egui::Painter,
        page: usize,
        words: &[PageWord],
        screen_rect: impl Fn([f32; 4]) -> Rect,
        accent: Color32,
    ) {
        if let Some(range) = self.selection.words_on(page, words.len()) {
            let fill = accent.gamma_multiply(0.45);
            for w in &words[range] {
                painter.rect_filled(screen_rect(w.bbox), 1.0, fill);
            }
        }
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
    pub fn select_last(&mut self, pdf: &limarepdf_core::Pdf, page: usize) {
        let n = self.objects(pdf, page).len();
        match n.checked_sub(1) {
            Some(i) => self.select_object(page, i),
            None => self.selection.clear_object(),
        }
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

    /// Whether the selected object `key` can be moved and resized now: anything with the
    /// arrow; with the read tool, all but page text, except text just added.
    fn arrangeable(&self, key: (usize, usize)) -> bool {
        let text = matches!(
            self.selection,
            Selection::Object {
                kind: ObjKind::Text | ObjKind::HiddenText,
                ..
            }
        );
        match self.tool {
            Tool::Select => true,
            Tool::Read => !text || self.loose == Some(key),
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
        if self.loose.is_some() && self.loose != self.sel() {
            self.loose = None;
        }
        let screen_rect =
            |b: [f32; 4]| Rect::from_min_max(to_screen(b[0], b[1]), to_screen(b[2], b[3]));
        let id = ui.id().with(("page-edit", tab_id, page));
        let mut resp = ui.interact(r, id, Sense::click_and_drag());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, true, format!("Page {}", page + 1))
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
        // The box that has handles: not a note's, nor page text's with the read tool.
        let sized_box = sel_box.filter(|_| match sel_annot {
            Some((_, _, r)) => r,
            None => sel_here.is_some_and(|s| self.arrangeable(s)),
        });
        // With the read tool, a press on the selected item (or its handles) moves it.
        let grabs = |ed: &mut Self, p: Pos2| {
            let on_handle = sized_box.is_some_and(|b| ed.handle_hit(b, p, htol).is_some());
            let on_annot = sel_annot
                .is_some_and(|(id, _, _)| ed.annot_at(pdf, page, p).is_some_and(|a| a.id == id));
            let on_object = sel_here.filter(|&s| ed.arrangeable(s)).is_some_and(|_| {
                sel_box.is_some_and(|b| {
                    p.x >= b[0] - tol && p.x <= b[2] + tol && p.y >= b[1] - tol && p.y <= b[3] + tol
                })
            });
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
        };
        // A press at `p` starts moving, resizing or turning: by the selected item's
        // handles; else with the arrow whatever is under it, which is selected, and with
        // the read tool the selected item itself (see `grabs`).
        let start_arrange = |ed: &mut Self, p: Pos2, read: bool| {
            let h = sized_box.and_then(|b| ed.handle_hit(b, p, htol));
            let selected = ed.selected_annot().filter(|_| sel_annot.is_some()).cloned();
            if let Some((id, b, _)) = sel_ink.as_ref().filter(|_| rotate_hit(p)) {
                ed.drag = Some(Drag {
                    page,
                    id: 0,
                    annot: Some(*id),
                    mode: Mode::Rotate,
                    start: p,
                    orig: *b,
                    cur: [0.0; 4],
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
                });
            } else if let (Some(h), Some(a)) = (h, &selected) {
                ed.drag = Some(annot_drag(a, Mode::Handle(h), p));
            } else if let (Some(h), Some((_, id))) = (h, sel_here) {
                ed.start_drag(page, id, Mode::Handle(h), p);
            } else if read {
                match (&selected, sel_here) {
                    (Some(a), _) => ed.drag = Some(annot_drag(a, Mode::Move, p)),
                    (None, Some((_, id))) => ed.start_drag(page, id, Mode::Move, p),
                    (None, None) => {}
                }
            } else if let Some(a) = ed.annot_at(pdf, page, p).cloned() {
                ed.selection = Selection::Annot { page, id: a.id };
                if movable(&a) {
                    ed.drag = Some(annot_drag(&a, Mode::Move, p));
                }
            } else if let Some(i) = ed.hit(page, p, tol) {
                ed.select_object(page, i);
                ed.start_drag(page, i, Mode::Move, p);
            } else {
                ed.selection.clear_object();
            }
        };
        // Typing new text, a note or text box, or a line of page text: it takes the
        // keyboard and the pointer.
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
                    if let Some(p) = ui.input(|i| i.pointer.press_origin()).map(to_pt) {
                        start_arrange(self, p, false);
                    }
                }
                if resp.dragged() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        self.drag_to(ui.ctx(), page, to_pt(pos), shift);
                    }
                }
                if resp.drag_stopped() {
                    self.drag_done(page);
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
                let on_text = |p: Pos2| lines.iter().any(|&b| inside(p, b));
                // Pictures and shapes a click selects; not ones covering most of the page.
                let area = (r.width() / k) * (r.height() / k);
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
                        object_at(self, p).filter(|&i| sel_here.map(|s| s.1) != Some(i))
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
                // starts on the selected picture, shape or comment moves it instead. A
                // click clears or selects a picture, shape or comment, Shift+click
                // extends, a double-click edits a line of page text (or selects a word
                // where there is none) and a triple-click selects a line.
                if resp.drag_started() {
                    let p = ui.input(|i| i.pointer.press_origin()).map(to_pt);
                    if let Some(p) = p.filter(|&p| grabs(self, p)) {
                        start_arrange(self, p, true);
                    } else {
                        self.start_word_drag(page, p, &words);
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
                if resp.triple_clicked() {
                    let line = nearest(click_at()).and_then(|i| Self::line_of(&words, i));
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
                    } else if let Some(i) = nearest(click_at()) {
                        self.selection = Selection::word((page, i));
                    }
                } else if resp.clicked() {
                    // A click on a comment or markup selects it, as one off the text on a
                    // picture or shape does; a drag still selects the text under them.
                    let annot = click_at()
                        .filter(|_| !shift)
                        .and_then(|p| self.annot_at(pdf, page, p))
                        .map(|a| a.id);
                    let object = click_at()
                        .filter(|_| !shift)
                        .and_then(|p| object_at(self, p));
                    match (annot, object, nearest(click_at())) {
                        (Some(id), _, _) => self.selection = Selection::Annot { page, id },
                        (None, Some(i), _) => self.select_object(page, i),
                        (None, None, Some(i)) if shift && self.selection.words().is_some() => {
                            self.extend_words(pdf, (page, i));
                        }
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
                self.paint_words(&painter, page, &words, screen_rect, accent);
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
                self.paint_words(&painter, page, &words, screen_rect, accent);
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
                        self.adding = Some(PageText::new(page, (p.x, p.y)));
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
                // A picture or shape, as a click selects them.
                self.objects(pdf, page);
                if let Some(i) = self.read_hit(page, p, tol, area) {
                    if self.sel() != Some((page, i)) {
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
            });
        }
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
            self.actions.push(match (d.annot, d.mode) {
                (Some(id), Mode::End(_)) => Action::LineEnds(d.page, id, d.cur),
                (Some(id), Mode::Rotate) => Action::RotateInk(d.page, id, d.cur[0]),
                (Some(id), _) => Action::MoveAnnot(d.page, id, d.cur),
                (None, _) => Action::SetBox(d.page, d.id, d.cur),
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
                | Tool::Redact
                | Tool::Sign
                | Tool::DigitalSign
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

    /// Whether an object is selected, which can be deleted.
    pub fn has_object(&self) -> bool {
        self.sel().is_some()
    }

    /// Whether the selected object can be moved now (see [`Self::arrangeable`]).
    pub fn object_moves(&self) -> bool {
        self.sel().is_some_and(|s| self.arrangeable(s))
    }

    /// Whether the selected annotation can be moved now.
    pub fn annot_moves(&self) -> bool {
        self.arranging() && self.selected_annot().is_some_and(movable)
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
        if self.annot_moves() {
            if let Some(a) = self.selected_annot() {
                let b = a.bbox;
                let to = [b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy];
                self.actions.push(Action::MoveAnnot(a.page, a.id, to));
                return;
            }
        }
        let Some((p, id)) = self.sel().filter(|_| self.object_moves()) else {
            return;
        };
        if let Some(o) = self.objs.get(&p).and_then(|v| v.get(id)) {
            let b = o.bbox;
            self.actions.push(Action::SetBox(
                p,
                id,
                [b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy],
            ));
        }
    }

    /// Esc: keep the new text being typed, or drop a note, text box or line being typed on
    /// the page, or clear a selected annotation, or the word or object selection (read
    /// tool); otherwise leave a drawing tool for the arrow, or with the arrow clear the
    /// selection.
    pub fn escape(&mut self) {
        if self.adding.is_some() {
            self.finish_adding();
            self.adding = None;
            return;
        }
        if self.writing.take().is_some() {
            return;
        }
        if self.line_edit.take().is_some() {
            // What was typed is kept, as Esc in the editor does.
            self.commit_insp();
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
            Tool::Select if self.sel().is_some() => {
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
        if let Selection::Object { page, id, .. } = new {
            self.loose = Some((page, id));
        }
        self.selection = new;
    }

    /// The pin at the top of a placing tool's options: on, the tool stays active after
    /// putting something down, until Esc or another tool.
    fn keep_ui(&mut self, ui: &mut egui::Ui) {
        let tool = self.tool;
        if !Tool::PLACING.contains(&tool) {
            return;
        }
        let on = self.keep.contains(&tool);
        // One row, the pin at its right.
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        let right = egui::Layout::right_to_left(egui::Align::Center);
        ui.allocate_ui_with_layout(row, right, |ui| {
            let r = ui
                .add(egui::Button::new(theme::icon(icon::PUSH_PIN).size(16.0)).selected(on))
                .on_hover_text(if on {
                    "Keep tool active: on. It stays after putting something down, until Esc \
                     or another tool."
                } else {
                    "Keep tool active"
                });
            r.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, "Keep tool active")
            });
            if r.clicked() {
                if on {
                    self.keep.retain(|t| *t != tool);
                } else {
                    self.keep.push(tool);
                }
            }
        });
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

    /// The selected annotation: what and whose it is, and its colour, opacity and comment,
    /// each finished change kept at once as one undo step.
    fn annot_inspector(&mut self, ui: &mut egui::Ui) {
        let Some(a) = self.selected_annot().cloned() else {
            ui.weak("Nothing selected.");
            return;
        };
        let now = AnnotEdit::of(&a);
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
            a.label()
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
            // Changes are kept as they are made, each finished one a step to undo; the
            // comment when it is left.
            if ui.button((theme::icon(icon::TRASH), "Delete")).clicked() {
                self.actions.push(Action::DeleteAnnot(a.page, a.id));
                done = true;
            } else if !e.same_as(&now) && settled(ui.ctx()) {
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
            ui.weak(if who.is_empty() {
                "Reply".to_string()
            } else {
                who.join(" · ")
            });
            ui.label(limarepdf_core::display_text(&r.contents, 2000));
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
        self.keep_ui(ui);
        match self.tool {
            Tool::Text => {
                let was = self.new_look();
                ui.strong("New text");
                Self::font_ui(ui, &mut self.tfont);
                ui.horizontal(|ui| {
                    ui.label("Size");
                    size_ui(ui, &mut self.tsize, 4.0..=200.0);
                    ui.color_edit_button_rgb(&mut self.tcolor);
                });
                // Changes apply to the text being typed, which then takes the keyboard back.
                if self.new_look() != was {
                    if let Some(a) = self.adding.as_mut() {
                        a.refocus = true;
                    }
                }
                ui.label(if self.adding.is_some() {
                    "Type on the page. Enter starts a new line; Esc or a click elsewhere \
                     finishes."
                } else {
                    "Click on the page where the text should start, and type."
                });
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
            Tool::Sign => {
                let out = self
                    .sign
                    .ui(ui, &mut self.sign_kit, &mut self.author, self.sign_items);
                self.finish_signing |= out.finish;
                if let Some(e) = &self.sign_error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                return;
            }
            Tool::DigitalSign => {
                ui.strong(if self.digital_certify {
                    "Certify with a digital ID"
                } else {
                    "Sign with a digital ID"
                });
                ui.label(
                    "Click an empty signature field, or drag a box on the page where the \
                     signature goes. You choose your digital ID next.",
                );
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
                ui.add_space(6.0);
                if ui.button("Cancel").clicked() {
                    self.set_tool(Tool::Read);
                }
                return;
            }
            Tool::Redact => {
                ui.strong("Mark for redaction");
                ui.label(
                    "Drag a box over anything to remove: text, pictures, drawings. Each box \
                     becomes a red-outlined mark; nothing is removed yet. Check the marks, then \
                     choose Document > Apply redactions to remove what is under them for good. \
                     Selected text and search results can be marked too.",
                );
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
                ui.strong("New text box");
                text_style_ui(ui, &mut self.box_style);
                ui.label(
                    "Drag a box on the page, or click for a default size, then type. Click \
                     outside the box or press Ctrl+Enter to finish; Esc drops it. Ctrl+click \
                     selects an annotation.",
                );
                return;
            }
            // A selected object shows its properties below.
            Tool::Select | Tool::Read if self.sel().is_some() => {}
            Tool::Select => {}
            Tool::Read => {
                ui.strong("Selected text");
                ui.label(
                    "Drag over words to select them, across pages if you like, Shift+click to \
                     extend, or press Ctrl+A for the whole page. Ctrl+C copies; right-click to \
                     print or save the selection. Double-click a line of page text to edit it, \
                     and click a picture, shape or comment to select, move or resize it.",
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

        // Changes are kept as they are made: each finished one is a step to undo.
        match obj.kind {
            ObjectKind::Text => {
                ui.add_space(4.0);
                if self.line_edit.is_some() {
                    ui.weak("Editing on the page. Ctrl+Z takes a change back.");
                } else {
                    let id = egui::Id::new(("text-object", page, id));
                    // What can't be written isn't typed, as on the page.
                    if ui.ctx().memory(|m| m.has_focus(id)) {
                        let t = page_text::take_typing(ui, true, false);
                        if t.refused {
                            self.text_refused.mark(ui.ctx());
                        }
                    }
                    let out = egui::TextEdit::singleline(&mut insp.text)
                        .id(id)
                        .desired_width(f32::INFINITY)
                        .show(ui);
                    let r = &out.response.response;
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Text")
                    });
                    self.text_refused.show(ui.ctx(), &out, id);
                    if self.focus_text {
                        r.request_focus();
                        self.focus_text = false;
                    }
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
                        size_ui(ui, &mut insp.size, 1.0..=400.0);
                        ui.color_edit_button_rgb(&mut insp.color);
                    });
                }
                if !obj.font_reusable && !obj.hidden {
                    ui.weak(format!(
                        "Font \"{}\" is embedded or non-standard. Edited text is set in the nearest standard font.",
                        obj.font_name.as_deref().unwrap_or("?")
                    ));
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
            // What was changed goes with it.
            self.insp = None;
            self.actions.push(Action::Delete(page, id));
        } else if settled(ui.ctx()) {
            self.commit_insp();
        }
    }
}

/// A text box's text size and colour, fill and border.
fn text_style_ui(ui: &mut egui::Ui, s: &mut TextStyle) {
    ui.horizontal(|ui| {
        ui.label("Text");
        size_ui(ui, &mut s.size, 4.0..=200.0);
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
        ed.reply = "Walrus reply".into();
        assert!(ed.pending());
        if let Some((_, e)) = ed.annot_insp.as_mut() {
            e.contents = "Walrus comment".into();
        }
        ed.keep_pending();
        assert!(matches!(
            ed.actions.as_slice(),
            [Action::SetAnnot(0, i, e), Action::Reply(0, j, r)]
                if *i == id && e.contents == "Walrus comment" && *j == id && r == "Walrus reply"
        ));
        assert!(ed.reply.is_empty());
        // A reply with nothing selected to answer isn't one.
        ed.actions.clear();
        ed.reply = "Walrus reply".into();
        ed.selection = Selection::None;
        assert!(!ed.pending());
    }

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
        let id = egui::Id::new("size test");
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
                ui.data_mut(|d| d.insert_temp(id, ui.id().with("size-box")));
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
        ed.select_object(0, 2);
        ed.sync_insp();
        let insp = ed.insp.as_mut().unwrap();
        insp.orig_size = 10.126;
        insp.size = 10.126;
        assert!(!ed.pending());
        // A change smaller than a hundredth is still a change.
        ed.insp.as_mut().unwrap().size = 10.13;
        ed.keep_pending();
        assert!(matches!(
            ed.actions.as_slice(),
            [Action::EditText(0, 2, te)] if te.size == Some(10.13)
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
        ed.loose = Some((0, 2));
        assert!(ed.object_moves());
        ed.loose = None;
        ed.set_tool_keep_sel(Tool::Select);
        assert!(ed.object_moves());
    }

    #[test]
    fn inspector_changes_are_kept_on_clicking_away() {
        let (_, mut ed) = page_with_objects();
        ed.select_object(0, 2);
        ed.sync_insp();
        assert!(!ed.pending());
        // A colour stored back a hair off by the colour button isn't a change.
        ed.insp.as_mut().unwrap().color[0] += 1e-6;
        assert!(!ed.pending());
        // Typed words: kept as one edit of the line when another object is selected.
        ed.insp.as_mut().unwrap().text = "Walruses".into();
        assert!(ed.pending());
        ed.select_object(0, 1);
        ed.sync_insp();
        assert!(matches!(
            ed.actions.as_slice(),
            [Action::EditText(0, 2, te)]
                if te.text == "Walruses" && te.font.is_none() && te.size.is_none()
        ));
        // A shape's new outline, kept when nothing is selected any more.
        ed.actions.clear();
        ed.insp.as_mut().unwrap().stroke_on = true;
        ed.selection = Selection::None;
        ed.sync_insp();
        assert!(matches!(
            ed.actions.as_slice(),
            [Action::PathStyle(0, 1, st)] if st.stroke.is_some()
        ));
        assert!(ed.insp.is_none() && !ed.pending());
        // A size picked for the line: a change of size only.
        ed.actions.clear();
        ed.select_object(0, 2);
        ed.sync_insp();
        ed.insp.as_mut().unwrap().size = 18.0;
        ed.keep_pending();
        assert!(matches!(
            ed.actions.as_slice(),
            [Action::EditText(0, 2, te)]
                if te.text == "Walrus like cold water" && te.size == Some(18.0) && te.color.is_none()
        ));
    }

    #[test]
    fn escape_keeps_what_was_typed_on_the_page() {
        let (_, mut ed) = page_with_objects();
        ed.start_line_edit(0, 2, Pick::Chars(0, 6));
        assert!(ed.typing_on_page());
        ed.insp.as_mut().unwrap().text = "Seal".into();
        ed.escape();
        assert!(!ed.typing_on_page() && !ed.pending());
        assert!(matches!(
            ed.actions.as_slice(),
            [Action::EditText(0, 2, te)] if te.text == "Seal"
        ));
        // The line stays selected; Esc again clears it.
        assert_eq!(ed.sel(), Some((0, 2)));
        ed.escape();
        assert_eq!(ed.sel(), None);
        // Nothing typed: Esc only closes the editor.
        ed.actions.clear();
        ed.start_line_edit(0, 2, Pick::Chars(0, 6));
        ed.escape();
        assert!(!ed.typing_on_page() && ed.actions.is_empty());
    }

    /// The editor draws a line in the font core will write it in.
    #[test]
    fn a_line_is_edited_in_the_font_it_will_be_written_in() {
        let (_, mut ed) = page_with_objects();
        ed.select_object(0, 2);
        ed.sync_insp();
        let o = ed.objs[&0][2].clone();
        let font = |ed: &Editor, o: &PageObject| {
            let te = text_change(ed.insp.as_ref().unwrap());
            format!("{:?}", line_font(o, &te))
        };
        // Its own font, when core can keep it: plain ASCII and no style changed.
        let kept = PageObject {
            font_name: Some("Arial,Bold".into()),
            font_reusable: true,
            ..o.clone()
        };
        assert_eq!(font(&ed, &kept), "Kept(\"Arial,Bold\")");
        ed.insp.as_mut().unwrap().text = "Walrus 42".into();
        assert_eq!(font(&ed, &kept), "Kept(\"Arial,Bold\")");
        // A character beyond ASCII, a style change, or a font that can't be kept: the
        // standard font nearest it, or the one chosen.
        let arial_bold = format!("Standard({:?})", FontChoice::from_name("Arial,Bold"));
        ed.insp.as_mut().unwrap().text = "Café".into();
        assert_eq!(font(&ed, &kept), arial_bold);
        ed.insp.as_mut().unwrap().text = "Walrus".into();
        ed.insp.as_mut().unwrap().size = 20.0;
        assert_eq!(font(&ed, &kept), arial_bold);
        ed.insp.as_mut().unwrap().size = ed.insp.as_ref().unwrap().orig_size;
        let not_kept = PageObject {
            font_reusable: false,
            ..kept.clone()
        };
        assert_eq!(font(&ed, &not_kept), arial_bold);
        ed.insp.as_mut().unwrap().font.italic = true;
        let courier = FontChoice {
            family: Family::Courier,
            ..ed.insp.as_ref().unwrap().font
        };
        ed.insp.as_mut().unwrap().font = courier;
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
