//! Every user-facing command, defined once. The menu, the top bar, the command palette
//! (Ctrl+K) and the keyboard shortcuts are all built from [`ACTIONS`]; what a command
//! does is in `App::act`.

use crate::editor::Tool;
use crate::markup::{self, Mark};
use crate::selection::{ObjKind, Selection};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, ThemePreference};
use egui_phosphor::regular as icon;

/// A command. Running one goes through `App::act`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    Open,
    /// Open the nth file in the recent-files list (of those that still exist). Built
    /// from the list at run time, so not in [`ACTIONS`].
    OpenRecent(usize),
    /// Empty the recent-files list. Shown with the list, so not in [`ACTIONS`].
    ClearRecent,
    NewFromImages,
    Save,
    SaveAs,
    ExportImages,
    ExportText,
    Print,
    CloseTab,
    Quit,
    Undo,
    Redo,
    Copy,
    SelectAllText,
    /// Print the selected words, as cropped pages or as plain text.
    PrintSelection,
    /// Save the selected words as a PDF (cropped pages or plain text) or a text file.
    SaveSelection,
    /// Mark the selected words, in the colour last chosen for the kind.
    Mark(Mark),
    /// Choose the colour for highlights, or (`line`) for the other marks: an index into
    /// [`crate::markup::HIGHLIGHTS`] or [`crate::markup::LINES`]. Shown in the markup
    /// menu, so not in [`ACTIONS`].
    MarkColor {
        line: bool,
        index: usize,
    },
    /// Set the name written into comments and markup.
    CommentName,
    /// Delete the selected object, or the selected pages.
    DeleteSelection,
    /// Copy the text of the selected text object.
    CopyObjectText,
    /// Put the keyboard in the inspector's text box for the selected text object.
    EditObjectText,
    /// Replace the selected image with one from a file.
    ReplaceImage,
    DeleteObject,
    /// Move the selected object by this many points (×10 with Shift).
    Nudge(i8, i8),
    /// Cancel what is in progress: new text, a drawing tool, or the selection.
    Escape,
    Find,
    ZoomIn,
    ZoomOut,
    FitWidth,
    Actual,
    NextPage,
    PrevPage,
    FirstPage,
    LastPage,
    ToggleSide,
    /// Show the side panel's Bookmarks tab.
    Bookmarks,
    Comments,
    FlattenAnnots,
    ToggleInspector,
    ToggleHidden,
    /// Tint every form field, and outline the required ones.
    ToggleFields,
    NextField,
    PrevField,
    /// Empty the form field with the keyboard.
    ClearField,
    ResetForm,
    FlattenForm,
    Theme(ThemePreference),
    Palette,
    Tool(Tool),
    RotR,
    RotL,
    Crop,
    Delete,
    RemoveBlank,
    Extract,
    Blank,
    InsertImages,
    Scans,
    Ocr,
    Merge,
    Split,
    HeaderFooter,
    Watermark,
    Props,
    Security,
    About,
}

/// Where a command is listed in the menu and the palette.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    File,
    Edit,
    View,
    Tools,
    Document,
    Help,
}

impl Group {
    pub const MENUS: [Group; 5] = [
        Group::File,
        Group::Edit,
        Group::View,
        Group::Document,
        Group::Help,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Group::File => "File",
            Group::Edit => "Edit",
            Group::View => "View",
            Group::Tools => "Tools",
            Group::Document => "Document",
            Group::Help => "Help",
        }
    }
}

/// What the commands' conditions are checked against, taken once per frame.
#[derive(Clone, Copy, Default)]
pub struct Ctx {
    /// A document is open.
    pub doc: bool,
    /// A background job (OCR, optimising, exporting, printing) is running.
    pub busy: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Words are selected with the read tool.
    pub words: bool,
    /// An object is selected with the arrow tool.
    pub object: bool,
    /// What the selected object is.
    pub obj_kind: Option<ObjKind>,
    /// Pages are selected in the thumbnails.
    pub pages: bool,
    pub tool: Option<Tool>,
    /// The Annotate button's tool, the one last used.
    pub annot_tool: Option<Tool>,
    pub side: bool,
    pub inspector: bool,
    /// The active tab's zoom follows the width of the view.
    pub fit: bool,
    pub hidden_text: bool,
    /// The document has form fields.
    pub form: bool,
    /// A form field has the keyboard.
    pub field: bool,
    pub fields_shown: bool,
    pub theme: ThemePreference,
    /// An annotation is selected, and whether it may be changed.
    pub annot: bool,
    pub annot_locked: bool,
    /// The arrow tool can move the selected annotation.
    pub annot_moves: bool,
    /// The markup colours last chosen; see [`Act::MarkColor`].
    pub highlight: usize,
    pub line: usize,
}

pub struct Action {
    pub act: Act,
    pub group: Group,
    pub label: &'static str,
    /// Phosphor glyph, or "" for none.
    pub icon: &'static str,
    /// Shortcuts; the first is the one shown.
    pub keys: &'static [KeyboardShortcut],
    pub enabled: fn(&Ctx) -> bool,
    /// For on/off commands: whether it is on.
    pub checked: Option<fn(&Ctx) -> bool>,
    /// Longer explanation, shown on hover.
    pub hint: &'static str,
    /// Listed in the menu (otherwise only in the palette, the top bar or as a key).
    pub menu: bool,
    /// Listed in the command palette.
    pub palette: bool,
    /// Draw a separator above it in the menu.
    pub sep: bool,
    /// Submenu of its group's menu, or "".
    pub submenu: &'static str,
}

fn always(_: &Ctx) -> bool {
    true
}
fn doc(c: &Ctx) -> bool {
    c.doc
}
fn idle(c: &Ctx) -> bool {
    c.doc && !c.busy
}
fn undo(c: &Ctx) -> bool {
    c.can_undo
}
fn redo(c: &Ctx) -> bool {
    c.can_redo
}
fn words(c: &Ctx) -> bool {
    c.words
}
fn words_idle(c: &Ctx) -> bool {
    c.words && !c.busy
}
fn object(c: &Ctx) -> bool {
    c.object
}
/// An object, or an annotation the arrow can move.
fn nudgeable(c: &Ctx) -> bool {
    c.object || c.annot_moves
}
fn text_object(c: &Ctx) -> bool {
    c.object && matches!(c.obj_kind, Some(ObjKind::Text | ObjKind::HiddenText))
}
fn deletable(c: &Ctx) -> bool {
    c.object || c.pages || (c.annot && !c.annot_locked)
}
fn image_object(c: &Ctx) -> bool {
    c.object && c.obj_kind == Some(ObjKind::Image)
}
fn text_tool(c: &Ctx) -> bool {
    c.tool == Some(Tool::Read)
}
fn any_tool(c: &Ctx) -> bool {
    c.doc && c.tool.is_some()
}

const C: Modifiers = Modifiers::COMMAND;
const CS: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);
const N: Modifiers = Modifiers::NONE;
const fn k(m: Modifiers, key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(m, key)
}

/// A command with the usual settings: listed in the menu and the palette, no
/// shortcut, icon, hint or separator. Fields are overridden with struct update syntax.
const BASE: Action = Action {
    act: Act::About,
    group: Group::Help,
    label: "",
    icon: "",
    keys: &[],
    enabled: doc,
    checked: None,
    hint: "",
    menu: true,
    palette: true,
    sep: false,
    submenu: "",
};

/// Menu order is table order within each group.
pub static ACTIONS: &[Action] = &[
    // ---------------------------------------------------------------- File
    Action {
        act: Act::Open,
        group: Group::File,
        label: "Open…",
        icon: icon::FOLDER_OPEN,
        keys: &[k(C, Key::O)],
        enabled: always,
        ..BASE
    },
    Action {
        act: Act::NewFromImages,
        group: Group::File,
        label: "Make a PDF from images…",
        icon: icon::IMAGES,
        enabled: always,
        ..BASE
    },
    Action {
        act: Act::Save,
        group: Group::File,
        label: "Save",
        icon: icon::FLOPPY_DISK,
        keys: &[k(C, Key::S)],
        ..BASE
    },
    Action {
        act: Act::SaveAs,
        group: Group::File,
        label: "Save As…",
        keys: &[k(CS, Key::S)],
        ..BASE
    },
    Action {
        act: Act::ExportImages,
        group: Group::File,
        label: "Export pages as images…",
        icon: icon::FILE_IMAGE,
        enabled: idle,
        sep: true,
        ..BASE
    },
    Action {
        act: Act::ExportText,
        group: Group::File,
        label: "Export text…",
        icon: icon::FILE_TEXT,
        ..BASE
    },
    Action {
        act: Act::Print,
        group: Group::File,
        label: "Print…",
        icon: icon::PRINTER,
        keys: &[k(C, Key::P)],
        enabled: idle,
        ..BASE
    },
    Action {
        act: Act::CloseTab,
        group: Group::File,
        label: "Close tab",
        icon: icon::X,
        keys: &[k(C, Key::W)],
        sep: true,
        ..BASE
    },
    Action {
        act: Act::Quit,
        group: Group::File,
        label: "Quit",
        icon: icon::SIGN_OUT,
        enabled: always,
        sep: true,
        ..BASE
    },
    // ---------------------------------------------------------------- Edit
    Action {
        act: Act::Undo,
        group: Group::Edit,
        label: "Undo",
        icon: icon::ARROW_COUNTER_CLOCKWISE,
        keys: &[k(C, Key::Z)],
        enabled: undo,
        ..BASE
    },
    Action {
        act: Act::Redo,
        group: Group::Edit,
        label: "Redo",
        icon: icon::ARROW_CLOCKWISE,
        keys: &[k(C, Key::Y), k(CS, Key::Z)],
        enabled: redo,
        ..BASE
    },
    Action {
        act: Act::Copy,
        group: Group::Edit,
        label: "Copy text",
        icon: icon::COPY,
        keys: &[k(C, Key::C)],
        enabled: words,
        sep: true,
        ..BASE
    },
    Action {
        act: Act::SelectAllText,
        group: Group::Edit,
        label: "Select all text on the page",
        icon: icon::SELECTION_ALL,
        keys: &[k(C, Key::A)],
        enabled: text_tool,
        ..BASE
    },
    Action {
        act: Act::PrintSelection,
        group: Group::Edit,
        label: "Print selected text…",
        icon: icon::PRINTER,
        enabled: words_idle,
        ..BASE
    },
    Action {
        act: Act::SaveSelection,
        group: Group::Edit,
        label: "Save selected text…",
        icon: icon::FLOPPY_DISK,
        enabled: words_idle,
        ..BASE
    },
    Action {
        act: Act::Mark(Mark::Highlight),
        group: Group::Edit,
        label: "Highlight",
        icon: icon::HIGHLIGHTER,
        keys: &[k(CS, Key::H)],
        enabled: words_idle,
        sep: true,
        hint: "Highlight the selected text, in the colour last chosen.",
        ..BASE
    },
    Action {
        act: Act::Mark(Mark::Underline),
        group: Group::Edit,
        label: "Underline",
        icon: icon::TEXT_UNDERLINE,
        enabled: words_idle,
        ..BASE
    },
    Action {
        act: Act::Mark(Mark::StrikeOut),
        group: Group::Edit,
        label: "Strike out",
        icon: icon::TEXT_STRIKETHROUGH,
        enabled: words_idle,
        ..BASE
    },
    Action {
        act: Act::Mark(Mark::Squiggly),
        group: Group::Edit,
        label: "Squiggly underline",
        icon: icon::WAVE_SINE,
        enabled: words_idle,
        ..BASE
    },
    Action {
        act: Act::CommentName,
        group: Group::Edit,
        label: "Your name for comments…",
        icon: icon::USER,
        enabled: always,
        hint: "The name saved in comments and markup you add, so others see who made them.",
        ..BASE
    },
    Action {
        act: Act::EditObjectText,
        group: Group::Edit,
        label: "Edit text",
        icon: icon::PENCIL_SIMPLE,
        enabled: text_object,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::ReplaceImage,
        group: Group::Edit,
        label: "Replace image…",
        icon: icon::IMAGE,
        enabled: image_object,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::CopyObjectText,
        group: Group::Edit,
        label: "Copy the object's text",
        icon: icon::COPY,
        enabled: text_object,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::DeleteSelection,
        group: Group::Edit,
        label: "Delete the selection",
        icon: icon::TRASH,
        keys: &[k(N, Key::Delete), k(N, Key::Backspace)],
        enabled: deletable,
        hint: "The selected object, comment or markup, or the pages selected in the \
               thumbnails. Undo brings them back.",
        ..BASE
    },
    Action {
        act: Act::DeleteObject,
        group: Group::Edit,
        label: "Delete the object",
        icon: icon::TRASH,
        enabled: object,
        menu: false,
        palette: false,
        ..BASE
    },
    Action {
        act: Act::Find,
        group: Group::Edit,
        label: "Find…",
        icon: icon::MAGNIFYING_GLASS,
        keys: &[k(C, Key::F)],
        sep: true,
        ..BASE
    },
    Action {
        act: Act::Escape,
        group: Group::Edit,
        label: "Cancel or deselect",
        keys: &[k(N, Key::Escape)],
        enabled: any_tool,
        menu: false,
        palette: false,
        ..BASE
    },
    Action {
        act: Act::Nudge(-1, 0),
        group: Group::Edit,
        label: "Move the selected object left",
        keys: &[k(N, Key::ArrowLeft)],
        enabled: nudgeable,
        menu: false,
        palette: false,
        ..BASE
    },
    Action {
        act: Act::Nudge(1, 0),
        group: Group::Edit,
        label: "Move the selected object right",
        keys: &[k(N, Key::ArrowRight)],
        enabled: nudgeable,
        menu: false,
        palette: false,
        ..BASE
    },
    Action {
        act: Act::Nudge(0, -1),
        group: Group::Edit,
        label: "Move the selected object up",
        keys: &[k(N, Key::ArrowUp)],
        enabled: nudgeable,
        menu: false,
        palette: false,
        ..BASE
    },
    Action {
        act: Act::Nudge(0, 1),
        group: Group::Edit,
        label: "Move the selected object down",
        keys: &[k(N, Key::ArrowDown)],
        enabled: nudgeable,
        menu: false,
        palette: false,
        ..BASE
    },
    // ---------------------------------------------------------------- View
    Action {
        act: Act::ZoomIn,
        group: Group::View,
        label: "Zoom in",
        icon: icon::MAGNIFYING_GLASS_PLUS,
        keys: &[k(C, Key::Equals)],
        ..BASE
    },
    Action {
        act: Act::ZoomOut,
        group: Group::View,
        label: "Zoom out",
        icon: icon::MAGNIFYING_GLASS_MINUS,
        keys: &[k(C, Key::Minus)],
        ..BASE
    },
    Action {
        act: Act::FitWidth,
        group: Group::View,
        label: "Fit width",
        icon: icon::ARROWS_OUT,
        keys: &[k(C, Key::Num0)],
        checked: Some(|c| c.fit),
        hint: "Keep the pages as wide as the space between the side panels, as the window or \
               panels change. Zooming turns it off; this turns it back on.",
        ..BASE
    },
    Action {
        act: Act::Actual,
        group: Group::View,
        label: "Actual size",
        keys: &[k(C, Key::Num1)],
        ..BASE
    },
    Action {
        act: Act::NextPage,
        group: Group::View,
        label: "Next page",
        keys: &[k(N, Key::PageDown)],
        menu: false,
        ..BASE
    },
    Action {
        act: Act::PrevPage,
        group: Group::View,
        label: "Previous page",
        keys: &[k(N, Key::PageUp)],
        menu: false,
        ..BASE
    },
    Action {
        act: Act::FirstPage,
        group: Group::View,
        label: "First page",
        keys: &[k(N, Key::Home)],
        menu: false,
        ..BASE
    },
    Action {
        act: Act::LastPage,
        group: Group::View,
        label: "Last page",
        keys: &[k(N, Key::End)],
        menu: false,
        ..BASE
    },
    Action {
        act: Act::ToggleSide,
        group: Group::View,
        label: "Side panel",
        icon: icon::SIDEBAR_SIMPLE,
        enabled: always,
        checked: Some(|c| c.side),
        sep: true,
        ..BASE
    },
    Action {
        act: Act::Bookmarks,
        group: Group::View,
        label: "Bookmarks",
        icon: icon::BOOKMARK_SIMPLE,
        hint: "Show the document's outline in the side panel.",
        ..BASE
    },
    Action {
        act: Act::Comments,
        group: Group::View,
        label: "Comments",
        icon: icon::CHAT_TEXT,
        hint: "List the document's comments, notes and markup in the side panel.",
        ..BASE
    },
    Action {
        act: Act::ToggleInspector,
        group: Group::View,
        label: "Inspector",
        icon: icon::SLIDERS_HORIZONTAL,
        enabled: always,
        checked: Some(|c| c.inspector),
        hint: "The panel on the right: options for what is selected, or for the \
               document when nothing is.",
        ..BASE
    },
    Action {
        act: Act::ToggleHidden,
        group: Group::View,
        label: "Show recognised text",
        checked: Some(|c| c.hidden_text),
        hint: "Outline the invisible text added by OCR. Hover to read it; select it with \
               the arrow tool to correct it.",
        ..BASE
    },
    Action {
        act: Act::ToggleFields,
        group: Group::View,
        label: "Highlight fields",
        enabled: |c| c.form,
        checked: Some(|c| c.fields_shown),
        hint: "Tint the form's fields, and outline the required ones.",
        ..BASE
    },
    Action {
        act: Act::NextField,
        group: Group::View,
        label: "Next field",
        enabled: |c| c.form,
        hint: "Tab also moves to the next field.",
        menu: false,
        ..BASE
    },
    Action {
        act: Act::PrevField,
        group: Group::View,
        label: "Previous field",
        enabled: |c| c.form,
        hint: "Shift+Tab also moves to the previous field.",
        menu: false,
        ..BASE
    },
    Action {
        act: Act::ClearField,
        group: Group::Edit,
        label: "Clear field",
        enabled: |c| c.field,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::Theme(ThemePreference::System),
        group: Group::View,
        label: "System",
        enabled: always,
        checked: Some(|c| c.theme == ThemePreference::System),
        submenu: "Theme",
        ..BASE
    },
    Action {
        act: Act::Theme(ThemePreference::Light),
        group: Group::View,
        label: "Light",
        enabled: always,
        checked: Some(|c| c.theme == ThemePreference::Light),
        submenu: "Theme",
        ..BASE
    },
    Action {
        act: Act::Theme(ThemePreference::Dark),
        group: Group::View,
        label: "Dark",
        enabled: always,
        checked: Some(|c| c.theme == ThemePreference::Dark),
        submenu: "Theme",
        ..BASE
    },
    Action {
        act: Act::Palette,
        group: Group::View,
        label: "Command palette…",
        icon: icon::COMMAND,
        keys: &[k(C, Key::K)],
        enabled: always,
        palette: false,
        sep: true,
        ..BASE
    },
    // --------------------------------------------------------------- Tools
    Action {
        act: Act::Tool(Tool::Read),
        group: Group::Tools,
        label: "Read and select text",
        icon: icon::CURSOR_TEXT,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Select),
        group: Group::Tools,
        label: "Select, move and resize objects",
        icon: icon::CURSOR,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Text),
        group: Group::Tools,
        label: "Add text",
        icon: icon::TEXT_T,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Image),
        group: Group::Tools,
        label: "Add image…",
        icon: icon::IMAGE,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Note),
        group: Group::Tools,
        label: "Add a note",
        icon: icon::NOTE,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::TextBox),
        group: Group::Tools,
        label: "Add a text box",
        icon: icon::TEXTBOX,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Rect),
        group: Group::Tools,
        label: "Add a rectangle",
        icon: icon::SQUARE,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Ellipse),
        group: Group::Tools,
        label: "Add an ellipse",
        icon: icon::CIRCLE,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Line),
        group: Group::Tools,
        label: "Add a line or arrow",
        icon: icon::LINE_SEGMENT,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Pen),
        group: Group::Tools,
        label: "Draw with the pen",
        icon: icon::SCRIBBLE,
        menu: false,
        ..BASE
    },
    // ------------------------------------------------------------ Document
    Action {
        act: Act::RotR,
        group: Group::Document,
        label: "Rotate clockwise",
        icon: icon::ARROW_CLOCKWISE,
        ..BASE
    },
    Action {
        act: Act::RotL,
        group: Group::Document,
        label: "Rotate counter-clockwise",
        icon: icon::ARROW_COUNTER_CLOCKWISE,
        ..BASE
    },
    Action {
        act: Act::Crop,
        group: Group::Document,
        label: "Crop pages…",
        icon: icon::CROP,
        hint: "Trim the margins of the selected pages, or all pages.",
        ..BASE
    },
    Action {
        act: Act::Delete,
        group: Group::Document,
        label: "Delete pages",
        icon: icon::TRASH,
        hint: "The selected pages, or the current page. Undo brings them back.",
        ..BASE
    },
    Action {
        act: Act::RemoveBlank,
        group: Group::Document,
        label: "Remove blank pages",
        icon: icon::BROOM,
        hint: "Pages of one even colour, give or take specks or a lone page number. Undo brings them back.",
        enabled: idle,
        ..BASE
    },
    Action {
        act: Act::Extract,
        group: Group::Document,
        label: "Extract pages to a new tab",
        icon: icon::ARROW_SQUARE_OUT,
        hint: "The selected pages, or the current page.",
        ..BASE
    },
    Action {
        act: Act::Blank,
        group: Group::Document,
        label: "Insert blank page",
        icon: icon::FILE_PLUS,
        hint: "After the selected pages, or the current page, at its size.",
        ..BASE
    },
    Action {
        act: Act::InsertImages,
        group: Group::Document,
        label: "Insert images…",
        icon: icon::IMAGES,
        hint: "After the selected pages, or the current page.",
        ..BASE
    },
    Action {
        act: Act::Scans,
        group: Group::Document,
        label: "Optimise scanned pages…",
        enabled: idle,
        ..BASE
    },
    Action {
        act: Act::Ocr,
        group: Group::Document,
        label: "Recognise text (OCR)…",
        enabled: idle,
        ..BASE
    },
    Action {
        act: Act::Merge,
        group: Group::Document,
        label: "Append PDFs…",
        icon: icon::STACK_PLUS,
        ..BASE
    },
    Action {
        act: Act::Split,
        group: Group::Document,
        label: "Split into files…",
        ..BASE
    },
    Action {
        act: Act::HeaderFooter,
        group: Group::Document,
        label: "Headers, footers, Bates…",
        sep: true,
        ..BASE
    },
    Action {
        act: Act::Watermark,
        group: Group::Document,
        label: "Watermark…",
        ..BASE
    },
    Action {
        act: Act::Props,
        group: Group::Document,
        label: "Properties…",
        sep: true,
        ..BASE
    },
    Action {
        act: Act::ResetForm,
        group: Group::Document,
        label: "Reset form",
        enabled: |c| c.form && !c.busy,
        hint: "Put every field back to its default value. Can be undone.",
        ..BASE
    },
    Action {
        act: Act::FlattenForm,
        group: Group::Document,
        label: "Flatten form…",
        enabled: |c| c.form && !c.busy,
        hint: "Make the fields part of the pages, as they look now, so they can't be \
               changed. Can be undone.",
        ..BASE
    },
    Action {
        act: Act::FlattenAnnots,
        group: Group::Document,
        label: "Flatten annotations…",
        enabled: |c| c.doc && !c.busy,
        hint: "Make comments, notes, highlights and drawings part of the pages, as they \
               look now, so they can't be changed or removed. Can be undone.",
        ..BASE
    },
    Action {
        act: Act::Security,
        group: Group::Document,
        label: "Password security…",
        ..BASE
    },
    // ---------------------------------------------------------------- Help
    Action {
        act: Act::About,
        group: Group::Help,
        label: "About",
        enabled: always,
        ..BASE
    },
];

/// The registry entry for `act`.
pub fn get(act: Act) -> &'static Action {
    ACTIONS
        .iter()
        .find(|a| a.act == act)
        .unwrap_or_else(|| panic!("{act:?} is not in ACTIONS"))
}

/// Shortcuts in the order to test them: those with Shift or Alt first, since egui
/// ignores extra Shift and Alt when matching (Ctrl+Shift+S would also match Ctrl+S).
pub fn shortcuts() -> Vec<(&'static Action, KeyboardShortcut)> {
    let mut v: Vec<_> = ACTIONS
        .iter()
        .flat_map(|a| a.keys.iter().map(move |k| (a, *k)))
        .collect();
    v.sort_by_key(|(_, k)| !(k.modifiers.shift || k.modifiers.alt));
    v
}

/// Whether every word of `query` appears in the command's label or group name.
pub fn matches(a: &Action, query: &str) -> bool {
    let hay = format!("{} {}", a.label, a.group.name()).to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|w| hay.contains(w))
}

/// What a right-click menu offers, in groups separated by a line, for what is selected
/// after the click (right-clicking selects what is under the pointer first).
pub fn context_items(sel: &Selection) -> &'static [&'static [Act]] {
    match sel {
        Selection::Words { .. } => &[
            &[Act::Copy, Act::SelectAllText],
            &[Act::Mark(Mark::Highlight)],
            &[Act::PrintSelection, Act::SaveSelection],
        ],
        Selection::Annot { .. } => &[&[Act::DeleteSelection]],
        Selection::Object { .. } => &[
            &[Act::EditObjectText, Act::ReplaceImage, Act::CopyObjectText],
            &[Act::DeleteObject],
        ],
        Selection::None | Selection::Pages(_) => &[
            &[Act::RotR, Act::RotL, Act::Crop],
            &[Act::Blank, Act::InsertImages, Act::Extract, Act::Delete],
        ],
    }
}

/// Fill a right-click menu: only the commands that are available now.
pub fn context_menu(ui: &mut egui::Ui, sel: &Selection, c: &Ctx, acts: &mut Vec<Act>) {
    let mut first = true;
    for group in context_items(sel) {
        let items: Vec<&Action> = group
            .iter()
            .map(|&a| get(a))
            .filter(|a| (a.enabled)(c))
            .collect();
        if items.is_empty() {
            continue;
        }
        if !first {
            ui.separator();
        }
        first = false;
        for a in &items {
            menu_item(ui, a, c, acts);
        }
        if items.iter().any(|a| a.act == Act::Mark(Mark::Highlight)) {
            ui.menu_button("Underline, strike out, colour", |ui| {
                markup::menu(ui, c, acts)
            });
        }
    }
}

/// Tooltip for a command: its label and first shortcut.
pub fn tooltip(ctx: &egui::Context, a: &Action) -> String {
    match a.keys.first() {
        Some(k) => format!("{} ({})", a.label, ctx.format_shortcut(k)),
        None => a.label.to_string(),
    }
}

/// The floating toolbar's buttons: the right-click menu's commands that are
/// available now and have an icon, in groups.
pub fn toolbar_items(sel: &Selection, c: &Ctx) -> Vec<Vec<&'static Action>> {
    context_items(sel)
        .iter()
        .map(|g| {
            let usable = |a: &&Action| (a.enabled)(c) && !a.icon.is_empty();
            g.iter().map(|&a| get(a)).filter(usable).collect::<Vec<_>>()
        })
        .filter(|g| !g.is_empty())
        .collect()
}

/// One menu entry: a checkbox for on/off commands, else a button with its shortcut.
/// A click queues the command in `acts`.
pub fn menu_item(ui: &mut egui::Ui, a: &Action, c: &Ctx, acts: &mut Vec<Act>) {
    let enabled = (a.enabled)(c);
    let r = match a.checked {
        Some(on) => {
            let mut v = on(c);
            ui.add_enabled(enabled, egui::Checkbox::new(&mut v, a.label))
        }
        None => {
            let key = a.keys.first().map(|k| ui.ctx().format_shortcut(k));
            let b = egui::Button::new(a.label).shortcut_text(key.unwrap_or_default());
            ui.add_enabled(enabled, b)
        }
    };
    let r = if a.hint.is_empty() {
        r
    } else {
        r.on_hover_text(a.hint)
    };
    if r.clicked() {
        acts.push(a.act);
        if a.checked.is_none() {
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_act_is_listed_once() {
        for (i, a) in ACTIONS.iter().enumerate() {
            let dup = ACTIONS[i + 1..].iter().any(|b| b.act == a.act);
            assert!(!dup, "{:?} is listed twice", a.act);
        }
    }

    #[test]
    fn shortcuts_are_unique() {
        let all = shortcuts();
        for (i, (a, k)) in all.iter().enumerate() {
            for (b, j) in &all[i + 1..] {
                assert!(k != j, "{:?} and {:?} share a shortcut", a.act, b.act);
            }
        }
    }

    #[test]
    fn shift_shortcuts_are_tried_first() {
        let all = shortcuts();
        let save_as = all.iter().position(|(a, _)| a.act == Act::SaveAs).unwrap();
        let save = all.iter().position(|(a, _)| a.act == Act::Save).unwrap();
        assert!(save_as < save);
    }

    #[test]
    fn context_menus_list_registered_commands() {
        let object = Selection::Object {
            page: 0,
            id: 0,
            kind: ObjKind::Path,
        };
        let words = Selection::word((0, 0));
        let annot = Selection::Annot {
            page: 0,
            id: (1, 0),
        };
        for sel in [Selection::None, object, words, annot] {
            for group in context_items(&sel) {
                for &a in *group {
                    get(a);
                }
            }
        }
    }

    #[test]
    fn palette_search_matches_every_word() {
        let print = get(Act::Print);
        assert!(matches(print, "print"));
        assert!(matches(print, "FILE pri"));
        assert!(!matches(print, "print watermark"));
    }
}
