//! Every user-facing command, defined once. The menu, the top bar, the command palette
//! (Ctrl+K) and the keyboard shortcuts are all built from [`ACTIONS`]; what a command
//! does is in `App::act`.

use crate::editor::Tool;
use crate::icon;
use crate::markup::{self, Mark};
use crate::selection::{ObjKind, Selection};
use crate::settings::{Accent, Layout, PageLook};
use crate::signing::SignPick;
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, ThemePreference};
use limarepdf_core::page_colours::PageColours;
use limarepdf_core::SignRole;

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
    /// Read the document's file again, as it now is on disk, after asking when there are
    /// unsaved changes.
    Reload,
    ExportImages,
    ExportText,
    Print,
    CloseTab,
    /// Keep recovery copies of documents with unsaved changes, or stop.
    ToggleRecovery,
    /// Show, in the file manager, the recovery file the active tab's notice names. Offered
    /// by the notice, so not in [`ACTIONS`].
    ShowRecoveryFile,
    Quit,
    Undo,
    Redo,
    /// Copy: what is selected in the text being typed, else the selected words, else the
    /// selected lines of page text.
    Copy,
    /// Paste into the text being typed on the page, at its caret.
    Paste,
    SelectAllText,
    /// Select every object on the page in front, with the arrow.
    SelectAllObjects,
    /// Print the selected words, as cropped pages or as plain text.
    PrintSelection,
    /// Save the selected words as a PDF (cropped pages or plain text) or a text file.
    SaveSelection,
    /// Bold on or off for the selected words, or the lines selected with the arrow, by
    /// Word's rule: all bold becomes not bold, anything else all bold.
    Bold,
    /// Italic on or off, as [`Act::Bold`].
    Italic,
    /// Mark the selected words, in the colour last chosen for the kind.
    Mark(Mark),
    /// Choose the colour for highlights, or (`line`) for the other marks: an index into
    /// [`crate::markup::HIGHLIGHTS`] or [`crate::markup::LINES`]. Shown in the markup
    /// menu, so not in [`ACTIONS`].
    MarkColor {
        line: bool,
        index: usize,
    },
    /// Mark the selected words for redaction.
    MarkRedact,
    /// Mark every result of the last search for redaction.
    MarkResultsRedact,
    /// Remove everything under the redaction marks, after asking.
    ApplyRedactions,
    /// Draw the fill-and-sign items into the pages, after asking.
    FinishSigning,
    /// Choose kinds of hidden information (metadata, scripts…) to remove.
    RemoveHidden,
    /// Set the name written into comments and markup.
    CommentName,
    /// Delete the selected objects, or the selected pages.
    DeleteSelection,
    /// Copy the text of the selected lines of page text, in reading order.
    CopyObjectText,
    /// Put the keyboard in the inspector's text box for the selected text object.
    EditObjectText,
    /// Move or resize the selected line of page text: the arrow tool, keeping it selected.
    MoveText,
    /// The Fill and sign tool, putting this down next (its button's menu).
    SignWith(SignPick),
    /// The Fill and sign tool, with the Create window open for a new signature or
    /// initials.
    CreateSign(SignRole),
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
    /// The page layout; Facing pages again, when on, goes back to continuous pages.
    Layout(Layout),
    /// In the facing layout, show the first page alone, or pair it with the second.
    ToggleCover,
    ToggleSide,
    /// Show the side panel's page thumbnails.
    PagesView,
    /// Show the side panel's search results.
    SearchView,
    /// Show the side panel's Bookmarks view.
    Bookmarks,
    Comments,
    /// Show the side panel's Signatures tab.
    Signatures,
    /// The details of the active document's nth digital signature. Opened from the
    /// Signatures tab or a signed field, so not in [`ACTIONS`].
    SignatureDetails(usize),
    /// Open the version of the document the nth signature signed, in a new tab. Not in
    /// [`ACTIONS`].
    SignedVersion(usize),
    /// The signers and certificate authorities trusted in LimarePDF.
    TrustedCertificates,
    /// Create, import, export and remove digital IDs.
    DigitalIds,
    /// Sign with a digital ID: choose the place, then the ID.
    SignDigital,
    /// Certify the document with a digital ID.
    Certify,
    /// The digital signature tool has its place: open the signing window. Not in
    /// [`ACTIONS`].
    DigitalPlaced,
    FlattenAnnots,
    ToggleInspector,
    /// Put the selection's controls in the inspector, or back in a bar by the selection.
    ToggleDock,
    ToggleHidden,
    /// Tint every form field, and outline the required ones.
    ToggleFields,
    /// Call pages by the document's page labels, or by their numbers.
    TogglePageLabels,
    /// Show the features planned but not built where they will be, greyed (ENH-41).
    TogglePlanned,
    /// The band's commands on one row, as icons, or full.
    ToggleCompact,
    /// The accent colour: the desktop's, or LimarePDF's.
    Accent(Accent),
    NextField,
    PrevField,
    /// Empty the form field with the keyboard.
    ClearField,
    ResetForm,
    FlattenForm,
    Theme(ThemePreference),
    /// How pages are coloured on screen (display only).
    PageColours(PageLook),
    /// Page colour modes recolour pictures too.
    TogglePicturesToo,
    Palette,
    /// Move the keyboard to the next region of the window (F6), or the one before
    /// (Shift+F6): see [`crate::regions`].
    NextRegion,
    PrevRegion,
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
    /// Open the inspector's page labels for the selected pages, or the current page.
    PageLabels,
    HeaderFooter,
    Watermark,
    Props,
    Security,
    About,
}

impl Act {
    /// Whether the command uses the document as it is: changes, saves, prints, exports or
    /// signs it, or asks a question about it. What is being typed goes into the document
    /// first (BUG-01). Commands that only show something, open a section of the inspector
    /// or choose a tool leave the typing as it is; closing asks about it.
    pub fn uses_document(self) -> bool {
        match self {
            Act::Save
            | Act::SaveAs
            | Act::Reload
            | Act::ExportText
            | Act::Print
            | Act::PrintSelection
            | Act::SaveSelection
            | Act::Bold
            | Act::Italic
            | Act::Undo
            | Act::Redo
            | Act::Mark(_)
            | Act::MarkRedact
            | Act::MarkResultsRedact
            | Act::ApplyRedactions
            | Act::FinishSigning
            | Act::RemoveHidden
            | Act::DeleteSelection
            | Act::MoveText
            | Act::ReplaceImage
            | Act::DeleteObject
            | Act::Nudge(..)
            | Act::SignDigital
            | Act::Certify
            | Act::DigitalPlaced
            | Act::FlattenAnnots
            | Act::ResetForm
            | Act::FlattenForm
            | Act::RotR
            | Act::RotL
            | Act::Delete
            | Act::RemoveBlank
            | Act::Extract
            | Act::Blank
            | Act::InsertImages
            | Act::Merge => true,
            Act::Open
            | Act::OpenRecent(_)
            | Act::ClearRecent
            | Act::NewFromImages
            | Act::ExportImages
            | Act::CloseTab
            | Act::ToggleRecovery
            | Act::ShowRecoveryFile
            | Act::Quit
            | Act::Copy
            | Act::Paste
            | Act::SelectAllText
            | Act::SelectAllObjects
            | Act::MarkColor { .. }
            | Act::CommentName
            | Act::CopyObjectText
            | Act::EditObjectText
            | Act::SignWith(_)
            | Act::CreateSign(_)
            | Act::Escape
            | Act::Find
            | Act::ZoomIn
            | Act::ZoomOut
            | Act::FitWidth
            | Act::Actual
            | Act::NextPage
            | Act::PrevPage
            | Act::FirstPage
            | Act::LastPage
            | Act::Layout(_)
            | Act::ToggleCover
            | Act::ToggleSide
            | Act::PagesView
            | Act::SearchView
            | Act::Bookmarks
            | Act::Comments
            | Act::Signatures
            | Act::SignatureDetails(_)
            | Act::SignedVersion(_)
            | Act::TrustedCertificates
            | Act::DigitalIds
            | Act::ToggleInspector
            | Act::ToggleDock
            | Act::ToggleHidden
            | Act::ToggleFields
            | Act::TogglePageLabels
            | Act::TogglePlanned
            | Act::ToggleCompact
            | Act::Accent(_)
            | Act::NextField
            | Act::PrevField
            | Act::ClearField
            | Act::Theme(_)
            | Act::PageColours(_)
            | Act::TogglePicturesToo
            | Act::Palette
            | Act::NextRegion
            | Act::PrevRegion
            | Act::Tool(_)
            | Act::Crop
            | Act::Split
            | Act::PageLabels
            | Act::HeaderFooter
            | Act::Watermark
            | Act::Props
            | Act::Security
            | Act::Scans
            | Act::Ocr
            | Act::About => false,
        }
    }
}

/// Where a command is listed in the menu and the palette.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    File,
    Edit,
    View,
    Tools,
    Pages,
    Document,
    Help,
}

impl Group {
    pub const MENUS: [Group; 7] = [
        Group::File,
        Group::Edit,
        Group::View,
        Group::Tools,
        Group::Pages,
        Group::Document,
        Group::Help,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Group::File => "File",
            Group::Edit => "Edit",
            Group::View => "View",
            Group::Tools => "Tools",
            Group::Pages => "Pages",
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
    /// The document was opened from or saved to a file.
    pub file: bool,
    /// A background job (OCR, optimising, exporting, printing) is running.
    pub busy: bool,
    /// The active tab's save is being written.
    pub saving: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Words are selected with the read tool.
    pub words: bool,
    /// Objects are selected, one or several.
    pub object: bool,
    /// The selected objects can be moved with the current tool: page text only with the
    /// arrow, or just after it was added.
    pub object_moves: bool,
    /// What the selected object is, when one is selected.
    pub obj_kind: Option<ObjKind>,
    /// Some of the selected objects are lines of page text, visible or not.
    pub obj_text: bool,
    /// Some of the selected objects are lines of visible page text.
    pub obj_lines: bool,
    /// Pages are selected in the thumbnails.
    pub pages: bool,
    pub tool: Option<Tool>,
    /// The Annotate button's tool, the one last used.
    pub annot_tool: Option<Tool>,
    pub side: bool,
    pub inspector: bool,
    /// The selection's controls are docked in the inspector.
    pub docked: bool,
    /// The active tab's zoom follows the width of the view.
    pub fit: bool,
    pub layout: Layout,
    /// See [`Act::ToggleCover`].
    pub cover: bool,
    pub hidden_text: bool,
    /// The document has form fields.
    pub form: bool,
    /// A form field has the keyboard.
    pub field: bool,
    pub fields_shown: bool,
    pub theme: ThemePreference,
    pub page_look: PageLook,
    pub pictures_too: bool,
    /// Recovery copies are kept.
    pub recovery: bool,
    /// See [`Act::TogglePageLabels`].
    pub page_labels: bool,
    /// See [`Act::TogglePlanned`].
    pub planned: bool,
    /// See [`Act::ToggleCompact`]: the setting, whatever a short window does.
    pub compact: bool,
    pub accent: Accent,
    /// Text is being typed on the page: new text, a line of page text, a note or a text
    /// box, open whether or not it has the keyboard just now.
    pub typing: bool,
    /// A line of page text is being edited on the page.
    pub line_edit: bool,
    /// An annotation is selected, and whether it may be changed.
    pub annot: bool,
    pub annot_locked: bool,
    /// The selected annotation can be moved with the current tool.
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
/// The document's file, which waits while it is saved.
fn file(c: &Ctx) -> bool {
    c.file && !c.busy && !c.saving
}
/// Signing, which replaces the tab with the signed file: not while the tab is saved.
fn signable(c: &Ctx) -> bool {
    c.doc && !c.busy && !c.saving
}
fn undo(c: &Ctx) -> bool {
    c.can_undo
}
fn redo(c: &Ctx) -> bool {
    c.can_redo
}
fn words_idle(c: &Ctx) -> bool {
    c.words && !c.busy
}
/// Something to copy: the text being typed (what is selected in it), words or lines.
fn copyable(c: &Ctx) -> bool {
    c.typing || c.words || lines_among(c)
}
fn typing(c: &Ctx) -> bool {
    c.typing
}
fn doc_idle(c: &Ctx) -> bool {
    c.doc && !c.busy
}
fn object(c: &Ctx) -> bool {
    c.object
}
/// An object or annotation that can be moved with the current tool.
fn nudgeable(c: &Ctx) -> bool {
    c.object_moves || c.annot_moves
}
/// Page text selected with the read tool, which moves only with the arrow.
fn fixed_text(c: &Ctx) -> bool {
    one_line(c) && !c.object_moves
}
/// One line of page text selected.
fn one_line(c: &Ctx) -> bool {
    c.object && matches!(c.obj_kind, Some(ObjKind::Text | ObjKind::HiddenText))
}
/// Lines of page text among the selected objects.
fn lines_among(c: &Ctx) -> bool {
    c.object && c.obj_text
}
/// Selected words, or lines of visible page text among the selected objects: what Bold and
/// Italic change.
fn styleable(c: &Ctx) -> bool {
    (c.words || (c.object && c.obj_lines)) && !c.busy
}
fn arrow_tool(c: &Ctx) -> bool {
    c.doc && c.tool == Some(Tool::Select)
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
        act: Act::Reload,
        group: Group::File,
        label: "Reload from disk",
        icon: icon::ARROWS_CLOCKWISE,
        enabled: file,
        hint: "Reads the file again as it now is, after asking when there are unsaved \
               changes, which reloading loses",
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
        act: Act::ToggleRecovery,
        group: Group::File,
        label: "Keep recovery copies of unsaved changes",
        icon: icon::LIFEBUOY,
        enabled: always,
        checked: Some(|c| c.recovery),
        hint: "Keeps a private copy of each document with unsaved changes, about a minute \
               behind, so that LimarePDF can offer it back if it stops without closing \
               normally. Your own files are never changed. Not for password-protected \
               documents.",
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
        enabled: copyable,
        sep: true,
        hint: "Copy what is selected in the text being typed, else the selected words or \
               lines of text.",
        ..BASE
    },
    Action {
        act: Act::Paste,
        group: Group::Edit,
        label: "Paste",
        icon: icon::CLIPBOARD_TEXT,
        enabled: typing,
        hint: "Paste into the text being typed, at its caret (Ctrl+V there does the same).",
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
        act: Act::SelectAllObjects,
        group: Group::Edit,
        label: "Select all on the page",
        icon: icon::SELECTION_ALL,
        keys: &[k(C, Key::A)],
        enabled: arrow_tool,
        hint: "Every picture, shape and line of text on the page, to move, restyle or \
               delete together.",
        ..BASE
    },
    Action {
        act: Act::Bold,
        group: Group::Edit,
        label: "Bold",
        icon: icon::TEXT_B,
        keys: &[k(C, Key::B)],
        enabled: styleable,
        sep: true,
        ..BASE
    },
    Action {
        act: Act::Italic,
        group: Group::Edit,
        label: "Italic",
        icon: icon::TEXT_ITALIC,
        keys: &[k(C, Key::I)],
        enabled: styleable,
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
        act: Act::MarkRedact,
        group: Group::Edit,
        label: "Mark for redaction",
        icon: icon::EYE_SLASH,
        enabled: words_idle,
        sep: true,
        hint: "Mark the selected text to be removed for good when the marks are applied.",
        ..BASE
    },
    Action {
        act: Act::MarkResultsRedact,
        group: Group::Edit,
        label: "Mark all search results for redaction",
        icon: icon::EYE_SLASH,
        enabled: doc_idle,
        hint: "Mark every match of the last search to be removed when the marks are applied.",
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
        enabled: doc_idle,
        sep: true,
        hint: "Change the words of the selected line of page text. With none selected, click \
               a line to change it, or somewhere empty to add text.",
        ..BASE
    },
    Action {
        act: Act::MoveText,
        group: Group::Edit,
        label: "Move or resize text",
        icon: icon::ARROWS_OUT_CARDINAL,
        enabled: fixed_text,
        hint: "Move or resize the selected line of page text: drag it or its handles, \
               until you select something else.",
        ..BASE
    },
    Action {
        act: Act::ReplaceImage,
        group: Group::Edit,
        label: "Replace image…",
        icon: icon::IMAGE,
        enabled: image_object,
        ..BASE
    },
    Action {
        act: Act::CopyObjectText,
        group: Group::Edit,
        label: "Copy the selected text lines",
        icon: icon::COPY,
        enabled: lines_among,
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
        palette: false,
        ..BASE
    },
    Action {
        act: Act::Nudge(-1, 0),
        group: Group::Edit,
        label: "Move left",
        keys: &[k(N, Key::ArrowLeft)],
        enabled: nudgeable,
        palette: false,
        hint: "One point; with Shift, ten.",
        submenu: "Move selected object",
        ..BASE
    },
    Action {
        act: Act::Nudge(1, 0),
        group: Group::Edit,
        label: "Move right",
        keys: &[k(N, Key::ArrowRight)],
        enabled: nudgeable,
        palette: false,
        hint: "One point; with Shift, ten.",
        submenu: "Move selected object",
        ..BASE
    },
    Action {
        act: Act::Nudge(0, -1),
        group: Group::Edit,
        label: "Move up",
        keys: &[k(N, Key::ArrowUp)],
        enabled: nudgeable,
        palette: false,
        hint: "One point; with Shift, ten.",
        submenu: "Move selected object",
        ..BASE
    },
    Action {
        act: Act::Nudge(0, 1),
        group: Group::Edit,
        label: "Move down",
        keys: &[k(N, Key::ArrowDown)],
        enabled: nudgeable,
        palette: false,
        hint: "One point; with Shift, ten.",
        submenu: "Move selected object",
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
        sep: true,
        submenu: "Go to page",
        ..BASE
    },
    Action {
        act: Act::PrevPage,
        group: Group::View,
        label: "Previous page",
        keys: &[k(N, Key::PageUp)],
        submenu: "Go to page",
        ..BASE
    },
    Action {
        act: Act::FirstPage,
        group: Group::View,
        label: "First page",
        keys: &[k(N, Key::Home)],
        submenu: "Go to page",
        ..BASE
    },
    Action {
        act: Act::LastPage,
        group: Group::View,
        label: "Last page",
        keys: &[k(N, Key::End)],
        submenu: "Go to page",
        ..BASE
    },
    Action {
        act: Act::Layout(Layout::Continuous),
        group: Group::View,
        label: "Continuous pages",
        icon: icon::ROWS,
        enabled: always,
        checked: Some(|c| c.layout == Layout::Continuous),
        hint: "One column of pages, scrolling through the document.",
        submenu: "Page layout",
        ..BASE
    },
    Action {
        act: Act::Layout(Layout::Facing),
        group: Group::View,
        label: "Facing pages",
        icon: icon::BOOK_OPEN,
        enabled: always,
        checked: Some(|c| c.layout == Layout::Facing),
        hint: "Two pages side by side, scrolling through the document.",
        submenu: "Page layout",
        ..BASE
    },
    Action {
        act: Act::Layout(Layout::Single),
        group: Group::View,
        label: "Single page",
        icon: icon::FILE,
        enabled: always,
        checked: Some(|c| c.layout == Layout::Single),
        hint: "One page at a time. Scrolling past its top or bottom turns the page.",
        submenu: "Page layout",
        ..BASE
    },
    Action {
        act: Act::ToggleCover,
        group: Group::View,
        label: "Show cover page alone",
        enabled: |c| c.layout == Layout::Facing,
        checked: Some(|c| c.cover),
        hint: "In facing pages, show the first page on its own, as in a book, so even \
               pages are on the left. Off pairs the first page with the second.",
        sep: true,
        submenu: "Page layout",
        ..BASE
    },
    Action {
        act: Act::ToggleSide,
        group: Group::View,
        label: "Navigation panel",
        icon: icon::SIDEBAR_SIMPLE,
        enabled: always,
        checked: Some(|c| c.side),
        sep: true,
        hint: "The panel on the left: pages, search results, bookmarks and comments.",
        ..BASE
    },
    Action {
        act: Act::PagesView,
        group: Group::View,
        label: "Pages",
        icon: icon::FILES,
        hint: "Show the page thumbnails in the navigation panel.",
        ..BASE
    },
    Action {
        act: Act::SearchView,
        group: Group::View,
        label: "Search results",
        icon: icon::LIST_MAGNIFYING_GLASS,
        hint: "Show the results of the last search in the navigation panel.",
        ..BASE
    },
    Action {
        act: Act::Bookmarks,
        group: Group::View,
        label: "Bookmarks",
        icon: icon::BOOKMARK_SIMPLE,
        hint: "Show the document's outline in the navigation panel.",
        ..BASE
    },
    Action {
        act: Act::Comments,
        group: Group::View,
        label: "Comments",
        icon: icon::CHAT_TEXT,
        hint: "List the document's comments, notes and markup in the navigation panel.",
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
        act: Act::ToggleDock,
        group: Group::View,
        label: "Selection controls on the right",
        icon: icon::SIDEBAR_SIMPLE,
        enabled: always,
        checked: Some(|c| c.docked),
        hint: "Show the buttons for selected text or objects at the top of the inspector \
               (or its strip) instead of in a bar by the selection.",
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
        act: Act::TogglePageLabels,
        group: Group::View,
        label: "Use the document's page labels",
        enabled: always,
        checked: Some(|c| c.page_labels),
        hint: "Call pages by the numbering the document gives them, such as iii or A-1, \
               in thumbnails, search results and bookmarks. Off numbers them 1, 2, 3 \
               from the first page. The status bar shows both.",
        ..BASE
    },
    Action {
        act: Act::TogglePlanned,
        group: Group::View,
        label: "Show planned features",
        icon: icon::EYE,
        enabled: always,
        checked: Some(|c| c.planned),
        hint: "Show features not built yet, greyed, where they will be.",
        ..BASE
    },
    Action {
        act: Act::ToggleCompact,
        group: Group::View,
        label: "Compact commands",
        icon: icon::CARET_UP,
        enabled: always,
        checked: Some(|c| c.compact),
        hint: "The commands on one row, as icons: their names show on hover. Short windows \
               show them so anyway.",
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
        submenu: "Form fields",
        ..BASE
    },
    Action {
        act: Act::PrevField,
        group: Group::View,
        label: "Previous field",
        enabled: |c| c.form,
        hint: "Shift+Tab also moves to the previous field.",
        submenu: "Form fields",
        ..BASE
    },
    Action {
        act: Act::ClearField,
        group: Group::Edit,
        label: "Clear field",
        enabled: |c| c.field,
        sep: true,
        hint: "Empty the form field being filled in.",
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
        act: Act::Accent(Accent::Desktop),
        group: Group::View,
        label: "Desktop",
        enabled: always,
        checked: Some(|c| c.accent == Accent::Desktop),
        hint: "The accent colour your desktop uses, where it says one; else LimarePDF's.",
        submenu: "Accent colour",
        ..BASE
    },
    Action {
        act: Act::Accent(Accent::LimarePdf),
        group: Group::View,
        label: "LimarePDF",
        enabled: always,
        checked: Some(|c| c.accent == Accent::LimarePdf),
        hint: "LimarePDF's own teal.",
        submenu: "Accent colour",
        ..BASE
    },
    Action {
        act: Act::PageColours(PageLook::Fixed(PageColours::Normal)),
        group: Group::View,
        label: "Normal",
        enabled: always,
        checked: Some(|c| c.page_look == PageLook::Fixed(PageColours::Normal)),
        hint: "Pages in their own colours.",
        submenu: "Page colours",
        ..BASE
    },
    Action {
        act: Act::PageColours(PageLook::Automatic),
        group: Group::View,
        label: "Automatic",
        enabled: always,
        checked: Some(|c| c.page_look == PageLook::Automatic),
        hint: "Dark pages while the app's theme is dark, normal pages while it is light.",
        submenu: "Page colours",
        ..BASE
    },
    Action {
        act: Act::PageColours(PageLook::Fixed(PageColours::Dark)),
        group: Group::View,
        label: "Dark",
        enabled: always,
        checked: Some(|c| c.page_look == PageLook::Fixed(PageColours::Dark)),
        hint: "Dark grey paper and light text, colours kept. Only how pages look on \
               screen: the file, printing and exports keep their colours.",
        submenu: "Page colours",
        ..BASE
    },
    Action {
        act: Act::PageColours(PageLook::Fixed(PageColours::HighContrast)),
        group: Group::View,
        label: "High contrast",
        enabled: always,
        checked: Some(|c| c.page_look == PageLook::Fixed(PageColours::HighContrast)),
        hint: "White text on black, in greys. Only how pages look on screen.",
        submenu: "Page colours",
        ..BASE
    },
    Action {
        act: Act::PageColours(PageLook::Fixed(PageColours::Sepia)),
        group: Group::View,
        label: "Sepia",
        enabled: always,
        checked: Some(|c| c.page_look == PageLook::Fixed(PageColours::Sepia)),
        hint: "Warm paper and dark brown text. Only how pages look on screen.",
        submenu: "Page colours",
        ..BASE
    },
    Action {
        act: Act::TogglePicturesToo,
        group: Group::View,
        label: "Recolour pictures too",
        enabled: always,
        checked: Some(|c| c.pictures_too),
        hint: "Off leaves photos and other pictures in their own colours. A scanned page \
               is one picture, so turn this on to recolour scans.",
        sep: true,
        submenu: "Page colours",
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
    Action {
        act: Act::NextRegion,
        group: Group::View,
        label: "Next region",
        keys: &[k(N, Key::F6)],
        enabled: always,
        palette: false,
        hint: "Move the keyboard to the next part of the window: the top bar, the tabs, the \
               notices, navigation, the page, the inspector or the status bar.",
        ..BASE
    },
    Action {
        act: Act::PrevRegion,
        group: Group::View,
        label: "Previous region",
        keys: &[k(Modifiers::SHIFT, Key::F6)],
        enabled: always,
        palette: false,
        hint: "Move the keyboard to the part of the window before.",
        ..BASE
    },
    // --------------------------------------------------------------- Tools
    Action {
        act: Act::Tool(Tool::Read),
        group: Group::Tools,
        label: "Read and select text",
        icon: icon::CURSOR_TEXT,
        hint: "Select and copy text, fill in forms and follow links; double-click page text to \
               edit it, and click pictures, shapes and comments to move or resize them.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Select),
        group: Group::Tools,
        label: "Select, move and resize objects",
        icon: icon::CURSOR,
        hint: "Click an object to select it. Drag to move, drag the handles to resize (Shift \
               keeps proportions); arrow keys nudge.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Text),
        group: Group::Tools,
        label: "Add text",
        icon: icon::TEXT_T,
        sep: true,
        hint: "Click on the page where the text should start, and type.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Image),
        group: Group::Tools,
        label: "Add image…",
        icon: icon::IMAGE,
        hint: "Choose a picture, then drag a box on the page or click to place it.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Redact),
        group: Group::Tools,
        label: "Mark areas for redaction",
        icon: icon::EYE_SLASH,
        hint: "Drag a box over anything to remove. Nothing is removed until the marks are \
               applied.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Sign),
        group: Group::Tools,
        label: "Fill and sign",
        icon: icon::SIGNATURE,
        hint: "Put your signature, initials, the date or a tick on the page: click to put it \
               down, or drag a box to size it. Not a digital signature.",
        ..BASE
    },
    Action {
        act: Act::CreateSign(SignRole::Signature),
        group: Group::Tools,
        label: "Create signature…",
        hint: "Draw, type or pick a picture of your signature, then put it on the page \
               with Fill and sign.",
        ..BASE
    },
    Action {
        act: Act::CreateSign(SignRole::Initials),
        group: Group::Tools,
        label: "Create initials…",
        hint: "Draw, type or pick a picture of your initials, then put them on the page \
               with Fill and sign.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Note),
        group: Group::Tools,
        label: "Add a note",
        icon: icon::NOTE,
        sep: true,
        submenu: "Commenting",
        hint: "Click on the page to place a note and type its comment.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::TextBox),
        group: Group::Tools,
        label: "Add a text box",
        icon: icon::TEXTBOX,
        submenu: "Commenting",
        hint: "Drag a box on the page, or click for a default size, then type.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Rect),
        group: Group::Tools,
        label: "Add a rectangle",
        icon: icon::SQUARE,
        submenu: "Commenting",
        hint: "Drag on the page, or click for a default size. Shift keeps squares.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Ellipse),
        group: Group::Tools,
        label: "Add an ellipse",
        icon: icon::CIRCLE,
        submenu: "Commenting",
        hint: "Drag on the page, or click for a default size. Shift keeps circles.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Line),
        group: Group::Tools,
        label: "Add a line or arrow",
        icon: icon::LINE_SEGMENT,
        submenu: "Commenting",
        hint: "Drag on the page, or click for a default length. Shift keeps 45° steps.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Pen),
        group: Group::Tools,
        label: "Draw with the pen",
        icon: icon::SCRIBBLE,
        submenu: "Commenting",
        hint: "Drag to draw. Strokes made close together become one drawing.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Stamp),
        group: Group::Tools,
        label: "Add a stamp",
        icon: icon::STAMP,
        submenu: "Commenting",
        hint: "Click on the page to put the stamp down, or drag a box to size it.",
        ..BASE
    },
    // ------------------------------------------------------------ Document
    Action {
        act: Act::RotR,
        group: Group::Pages,
        label: "Rotate clockwise",
        icon: icon::ARROW_CLOCKWISE,
        ..BASE
    },
    Action {
        act: Act::RotL,
        group: Group::Pages,
        label: "Rotate counter-clockwise",
        icon: icon::ARROW_COUNTER_CLOCKWISE,
        ..BASE
    },
    Action {
        act: Act::Crop,
        group: Group::Pages,
        label: "Crop pages…",
        icon: icon::CROP,
        hint: "Trim the margins of the selected pages, or all pages.",
        ..BASE
    },
    Action {
        act: Act::Delete,
        group: Group::Pages,
        label: "Delete pages",
        icon: icon::TRASH,
        hint: "The selected pages, or the current page. Undo brings them back.",
        ..BASE
    },
    Action {
        act: Act::RemoveBlank,
        group: Group::Pages,
        label: "Remove blank pages",
        icon: icon::BROOM,
        hint: "Pages of one even colour, give or take specks or a lone page number. Undo brings them back.",
        enabled: idle,
        ..BASE
    },
    Action {
        act: Act::Extract,
        group: Group::Pages,
        label: "Extract pages to a new tab",
        icon: icon::ARROW_SQUARE_OUT,
        hint: "The selected pages, or the current page.",
        ..BASE
    },
    Action {
        act: Act::Blank,
        group: Group::Pages,
        sep: true,
        label: "Insert blank page",
        icon: icon::FILE_PLUS,
        hint: "After the selected pages, or the current page, at its size.",
        ..BASE
    },
    Action {
        act: Act::InsertImages,
        group: Group::Pages,
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
        group: Group::Pages,
        sep: true,
        label: "Append PDFs…",
        icon: icon::STACK_PLUS,
        // Files are read in the background, one append at a time.
        enabled: idle,
        ..BASE
    },
    Action {
        act: Act::Split,
        group: Group::Pages,
        label: "Split into files…",
        ..BASE
    },
    Action {
        act: Act::PageLabels,
        group: Group::Pages,
        label: "Page labels…",
        sep: true,
        hint: "Number pages your own way, such as i, ii, iii then 1, 2, 3, from the selected \
               pages or the current one.",
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
               look now, so they are no longer comments. Can be undone.",
        ..BASE
    },
    Action {
        act: Act::ApplyRedactions,
        group: Group::Document,
        label: "Apply redactions…",
        icon: icon::EYE_SLASH,
        enabled: doc_idle,
        hint: "Remove the text, pictures and drawings under the redaction marks for good, \
               and paint the areas black.",
        ..BASE
    },
    Action {
        act: Act::FinishSigning,
        group: Group::Document,
        label: "Finish signing…",
        icon: icon::SIGNATURE,
        enabled: doc_idle,
        hint: "Draw the signatures, dates, ticks and other items put down with the Sign \
               tool into the pages, so they can no longer be moved or removed as items.",
        ..BASE
    },
    Action {
        act: Act::RemoveHidden,
        group: Group::Document,
        label: "Remove hidden information…",
        icon: icon::BROOM,
        enabled: doc_idle,
        sep: true,
        hint: "Remove metadata, scripts, attachments, hidden layers and other information that \
               travels with the file but isn't on its pages.",
        ..BASE
    },
    Action {
        act: Act::Security,
        group: Group::Document,
        label: "Password security…",
        ..BASE
    },
    Action {
        act: Act::Signatures,
        group: Group::Document,
        label: "Signatures in this document",
        icon: icon::SEAL_CHECK,
        sep: true,
        hint: "List the document's digital signatures and whether they are valid.",
        submenu: "Digital signatures",
        ..BASE
    },
    Action {
        act: Act::SignDigital,
        group: Group::Document,
        label: "Sign with digital ID…",
        icon: icon::SEAL_CHECK,
        enabled: signable,
        sep: true,
        hint: "Sign so that others can check who signed and whether anything changed \
               afterwards. The signed document is saved as a new file.",
        submenu: "Digital signatures",
        ..BASE
    },
    Action {
        act: Act::Certify,
        group: Group::Document,
        label: "Certify…",
        icon: icon::CERTIFICATE,
        enabled: signable,
        hint: "Sign as the document's author, choosing what others may change afterwards. \
               Only before anyone has signed it.",
        submenu: "Digital signatures",
        ..BASE
    },
    Action {
        act: Act::DigitalIds,
        group: Group::Document,
        label: "Digital IDs…",
        icon: icon::IDENTIFICATION_CARD,
        enabled: always,
        sep: true,
        hint: "Create, import, export or remove the digital IDs you sign documents with.",
        submenu: "Digital signatures",
        ..BASE
    },
    Action {
        act: Act::TrustedCertificates,
        group: Group::Document,
        label: "Trusted certificates…",
        icon: icon::CERTIFICATE,
        enabled: always,
        hint: "The signers and certificate authorities you trust when LimarePDF checks digital \
               signatures.",
        submenu: "Digital signatures",
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

/// The command's name for messages, without a trailing "…".
pub fn name(act: Act) -> &'static str {
    ACTIONS
        .iter()
        .find(|a| a.act == act)
        .map_or("The command", |a| a.label.trim_end_matches('…'))
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

/// The command's name in the palette for screen readers: its label after its submenu's
/// name, if it has one ("Theme: Dark", "Page colours: Dark"), so commands of the same
/// label tell apart. The palette shows it as "Theme › Dark".
pub fn spoken_title(a: &Action) -> String {
    match a.submenu {
        "" => a.label.to_string(),
        sub => format!("{sub}: {}", a.label),
    }
}

/// Whether every word of `query` appears in the command's label, submenu or group name.
pub fn matches(a: &Action, query: &str) -> bool {
    let hay = format!("{} {} {}", a.label, a.submenu, a.group.name()).to_lowercase();
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
            &[Act::Bold, Act::Italic],
            &[Act::Mark(Mark::Highlight)],
            &[Act::MarkRedact],
            &[Act::PrintSelection, Act::SaveSelection],
        ],
        Selection::Annot { .. } => &[&[Act::DeleteSelection]],
        Selection::Objects { .. } => &[
            &[
                Act::EditObjectText,
                Act::MoveText,
                Act::ReplaceImage,
                Act::CopyObjectText,
            ],
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

/// Tooltip for a command: its label and first shortcut, then its hint on a line of its own.
pub fn tooltip(ctx: &egui::Context, a: &Action) -> String {
    let name = match a.keys.first() {
        Some(k) => format!("{} ({})", a.label, ctx.format_shortcut(k)),
        None => a.label.to_string(),
    };
    if a.hint.is_empty() {
        name
    } else {
        format!("{name}\n{}", a.hint)
    }
}

/// The floating toolbar's buttons: the right-click menu's commands that are
/// available now and have an icon, in groups. While a line is edited on the page, Bold,
/// Italic and Copy for what is selected in it, as the keys do there (ENH-20).
pub fn toolbar_items(sel: &Selection, c: &Ctx) -> Vec<Vec<&'static Action>> {
    const LINE_EDIT: &[&[Act]] = &[&[Act::Bold, Act::Italic], &[Act::Copy]];
    let items = if c.line_edit {
        LINE_EDIT
    } else {
        context_items(sel)
    };
    items
        .iter()
        .map(|g| {
            let usable = |a: &&Action| (a.enabled)(c) && !a.icon.is_empty();
            g.iter().map(|&a| get(a)).filter(usable).collect::<Vec<_>>()
        })
        .filter(|g| !g.is_empty())
        .collect()
}

/// The selection's buttons from [`toolbar_items`], in the ui's direction, groups apart:
/// for the floating bar, the inspector's top and its strip. Highlight has a menu beside
/// it for the other marks and the colours.
pub fn context_buttons(
    ui: &mut egui::Ui,
    groups: &[Vec<&'static Action>],
    c: &Ctx,
    acts: &mut Vec<Act>,
) {
    for (n, group) in groups.iter().enumerate() {
        if n > 0 {
            ui.separator();
        }
        for a in group {
            let glyph = match a.act {
                // A bold B and a slanted I with serifs, as the inspector and word
                // processors show them; the icon is drawn inside some padding.
                Act::Bold => egui::RichText::new("B")
                    .family(crate::theme::semibold())
                    .size(16.0),
                Act::Italic => crate::theme::icon(a.icon).size(19.0),
                _ => crate::theme::icon(a.icon).size(16.0),
            };
            let b = egui::Button::new(glyph)
                .frame_when_inactive(false)
                .min_size(egui::vec2(26.0, 26.0));
            let r = ui.add(b);
            let name = a.label.trim_end_matches('…');
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
            if r.on_hover_text(tooltip(ui.ctx(), a)).clicked() {
                acts.push(a.act);
            }
            if a.act == Act::Mark(Mark::Highlight) {
                let more = egui::Button::new(crate::theme::icon(icon::CARET_DOWN).size(12.0))
                    .frame_when_inactive(false)
                    .min_size(egui::vec2(16.0, 26.0));
                egui::containers::menu::MenuButton::from_button(more)
                    .ui(ui, |ui| markup::menu(ui, c, acts))
                    .0
                    .on_hover_text("Underline, strike out, colours");
            }
        }
    }
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

    /// Each shortcut does one thing: two commands share one only when no tool has both
    /// (Ctrl+A selects all text with the read tool, all objects with the arrow).
    #[test]
    fn shortcuts_are_unique() {
        let tools = [Tool::Read, Tool::Select, Tool::Text, Tool::Rect, Tool::Note];
        let both = |a: &Action, b: &Action| {
            tools.iter().any(|&t| {
                let c = Ctx {
                    doc: true,
                    tool: Some(t),
                    ..Ctx::default()
                };
                (a.enabled)(&c) && (b.enabled)(&c)
            })
        };
        let all = shortcuts();
        for (i, (a, k)) in all.iter().enumerate() {
            for (b, j) in &all[i + 1..] {
                let apart = a.act == Act::SelectAllText && b.act == Act::SelectAllObjects;
                assert!(
                    k != j || apart,
                    "{:?} and {:?} share a shortcut",
                    a.act,
                    b.act
                );
                if k == j {
                    assert!(!both(a, b), "{:?} and {:?} are on together", a.act, b.act);
                }
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
        let object = Selection::Objects {
            page: 0,
            held: Vec::new(),
            loose: false,
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

    /// Every command has a route in the menu, so nothing is reachable only by an icon, a
    /// key or a right-click. Delete the object is Delete the selection, for objects.
    #[test]
    fn every_command_is_in_the_menu() {
        for a in ACTIONS {
            if a.act != Act::DeleteObject {
                assert!(a.menu, "{:?} is not in the menu", a.act);
                assert!(
                    Group::MENUS.contains(&a.group),
                    "{:?}'s menu isn't shown",
                    a.act
                );
            }
        }
    }

    /// A submenu is one run of entries, or its later entries would make a second
    /// submenu of the same name.
    #[test]
    fn submenus_are_unbroken() {
        for g in Group::MENUS {
            let items: Vec<_> = ACTIONS.iter().filter(|a| a.group == g && a.menu).collect();
            let mut seen: Vec<&str> = Vec::new();
            for (i, a) in items.iter().enumerate() {
                let starts = i == 0 || items[i - 1].submenu != a.submenu;
                if !a.submenu.is_empty() && starts {
                    assert!(!seen.contains(&a.submenu), "{} is split", a.submenu);
                    seen.push(a.submenu);
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

    /// BUG-34 (audit A18): a command in a submenu is found by the submenu's name too, so
    /// "theme" finds the theme's commands and "dark theme" only its Dark.
    #[test]
    fn palette_search_matches_the_submenu() {
        let found = |query: &str| -> Vec<Act> {
            ACTIONS
                .iter()
                .filter(|a| a.palette && matches(a, query))
                .map(|a| a.act)
                .collect()
        };
        let themes = [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ];
        assert_eq!(found("theme"), themes.map(Act::Theme));
        assert_eq!(found("dark theme"), [Act::Theme(ThemePreference::Dark)]);
        let dark = Act::PageColours(PageLook::Fixed(PageColours::Dark));
        assert_eq!(found("page colours dark"), [dark]);
        assert_eq!(found("page colours").len(), 6);
    }

    /// BUG-34: no two commands in the palette look or sound alike: each one's submenu and
    /// label, shown and named for screen readers, are its own.
    #[test]
    fn palette_titles_are_unique() {
        let listed: Vec<_> = ACTIONS.iter().filter(|a| a.palette).collect();
        for (i, a) in listed.iter().enumerate() {
            for b in &listed[i + 1..] {
                let (x, y) = (spoken_title(a), spoken_title(b));
                assert!(x != y, "{:?} and {:?} are both {x}", a.act, b.act);
            }
        }
        assert_eq!(spoken_title(get(Act::Print)), "Print…");
        let dark = get(Act::Theme(ThemePreference::Dark));
        assert_eq!(spoken_title(dark), "Theme: Dark");
    }
}
