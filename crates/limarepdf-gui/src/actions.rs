//! Every user-facing command, defined once. The menu, the top bar, the command palette
//! (Ctrl+K) and the keyboard shortcuts are all built from [`ACTIONS`]; what a command
//! does is in `App::act`.

use crate::editor::Tool;
use crate::markup::{self, Mark};
use crate::selection::{ObjKind, Selection};
use crate::settings::{Layout, PageLook};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, ThemePreference};
use egui_phosphor::regular as icon;
use limarepdf_core::page_colours::PageColours;

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
    /// Keep recovery copies of documents with unsaved changes, or stop.
    ToggleRecovery,
    /// Show, in the file manager, the recovery file the active tab's notice names. Offered
    /// by the notice, so not in [`ACTIONS`].
    ShowRecoveryFile,
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
    /// How pages are coloured on screen (display only).
    PageColours(PageLook),
    /// Page colour modes recolour pictures too.
    TogglePicturesToo,
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
fn doc_idle(c: &Ctx) -> bool {
    c.doc && !c.busy
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
        enabled: text_object,
        sep: true,
        hint: "Change the words of the selected line of page text.",
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
        label: "Copy the object's text",
        icon: icon::COPY,
        enabled: text_object,
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
    // --------------------------------------------------------------- Tools
    Action {
        act: Act::Tool(Tool::Read),
        group: Group::Tools,
        label: "Read and select text",
        icon: icon::CURSOR_TEXT,
        hint: "Select and copy text, fill in forms and follow links.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Select),
        group: Group::Tools,
        label: "Select, move and resize objects",
        icon: icon::CURSOR,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Text),
        group: Group::Tools,
        label: "Add text",
        icon: icon::TEXT_T,
        sep: true,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Image),
        group: Group::Tools,
        label: "Add image…",
        icon: icon::IMAGE,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Redact),
        group: Group::Tools,
        label: "Mark areas for redaction",
        icon: icon::EYE_SLASH,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Sign),
        group: Group::Tools,
        label: "Fill and sign",
        icon: icon::SIGNATURE,
        hint: "Put your signature, initials, the date or a tick on the page. Not a digital \
               signature.",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Note),
        group: Group::Tools,
        label: "Add a note",
        icon: icon::NOTE,
        sep: true,
        submenu: "Commenting",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::TextBox),
        group: Group::Tools,
        label: "Add a text box",
        icon: icon::TEXTBOX,
        submenu: "Commenting",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Rect),
        group: Group::Tools,
        label: "Add a rectangle",
        icon: icon::SQUARE,
        submenu: "Commenting",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Ellipse),
        group: Group::Tools,
        label: "Add an ellipse",
        icon: icon::CIRCLE,
        submenu: "Commenting",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Line),
        group: Group::Tools,
        label: "Add a line or arrow",
        icon: icon::LINE_SEGMENT,
        submenu: "Commenting",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Pen),
        group: Group::Tools,
        label: "Draw with the pen",
        icon: icon::SCRIBBLE,
        submenu: "Commenting",
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Stamp),
        group: Group::Tools,
        label: "Add a stamp",
        icon: icon::STAMP,
        submenu: "Commenting",
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
        ..BASE
    },
    Action {
        act: Act::Split,
        group: Group::Pages,
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
               tool into the pages, so they can no longer be moved or removed.",
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
        enabled: doc_idle,
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
        enabled: doc_idle,
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
            &[Act::MarkRedact],
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
}
