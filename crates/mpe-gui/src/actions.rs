//! Every user-facing command, defined once. The menu, the top bar, the command palette
//! (Ctrl+K) and the keyboard shortcuts are all built from [`ACTIONS`]; what a command
//! does is in `App::act`.

use crate::editor::Tool;
use crate::selection::{ObjKind, Selection};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, ThemePreference};
use egui_phosphor::regular as icon;

/// A command. Running one goes through `App::act`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    Open,
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
    ToggleInspector,
    ToggleHidden,
    Theme(ThemePreference),
    Palette,
    Tool(Tool),
    RotR,
    RotL,
    Delete,
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
    /// Words are selected with the text tool.
    pub words: bool,
    /// An object is selected with the arrow tool.
    pub object: bool,
    /// What the selected object is.
    pub obj_kind: Option<ObjKind>,
    pub tool: Option<Tool>,
    pub side: bool,
    pub inspector: bool,
    pub hidden_text: bool,
    pub theme: ThemePreference,
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
fn object(c: &Ctx) -> bool {
    c.object
}
fn text_object(c: &Ctx) -> bool {
    c.object && matches!(c.obj_kind, Some(ObjKind::Text | ObjKind::HiddenText))
}
fn image_object(c: &Ctx) -> bool {
    c.object && c.obj_kind == Some(ObjKind::Image)
}
fn text_tool(c: &Ctx) -> bool {
    c.tool == Some(Tool::SelectText)
}
fn editing(c: &Ctx) -> bool {
    c.doc && !matches!(c.tool, None | Some(Tool::View))
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
        label: "New from images…",
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
        keys: &[k(C, Key::A)],
        enabled: text_tool,
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
        act: Act::DeleteObject,
        group: Group::Edit,
        label: "Delete the selected object",
        icon: icon::TRASH,
        keys: &[k(N, Key::Delete), k(N, Key::Backspace)],
        enabled: object,
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
        enabled: editing,
        menu: false,
        palette: false,
        ..BASE
    },
    Action {
        act: Act::Nudge(-1, 0),
        group: Group::Edit,
        label: "Move the selected object left",
        keys: &[k(N, Key::ArrowLeft)],
        enabled: object,
        menu: false,
        palette: false,
        ..BASE
    },
    Action {
        act: Act::Nudge(1, 0),
        group: Group::Edit,
        label: "Move the selected object right",
        keys: &[k(N, Key::ArrowRight)],
        enabled: object,
        menu: false,
        palette: false,
        ..BASE
    },
    Action {
        act: Act::Nudge(0, -1),
        group: Group::Edit,
        label: "Move the selected object up",
        keys: &[k(N, Key::ArrowUp)],
        enabled: object,
        menu: false,
        palette: false,
        ..BASE
    },
    Action {
        act: Act::Nudge(0, 1),
        group: Group::Edit,
        label: "Move the selected object down",
        keys: &[k(N, Key::ArrowDown)],
        enabled: object,
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
        act: Act::Tool(Tool::View),
        group: Group::Tools,
        label: "Read only (no editing)",
        icon: icon::HAND,
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
        act: Act::Tool(Tool::SelectText),
        group: Group::Tools,
        label: "Select and copy text",
        icon: icon::CURSOR_TEXT,
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
        act: Act::Tool(Tool::Rect),
        group: Group::Tools,
        label: "Add rectangle",
        icon: icon::SQUARE,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Ellipse),
        group: Group::Tools,
        label: "Add ellipse",
        icon: icon::CIRCLE,
        menu: false,
        ..BASE
    },
    Action {
        act: Act::Tool(Tool::Line),
        group: Group::Tools,
        label: "Add line",
        icon: icon::LINE_SEGMENT,
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
        act: Act::Delete,
        group: Group::Document,
        label: "Delete pages",
        icon: icon::TRASH,
        hint: "The selected pages, or the current page. Undo brings them back.",
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
        Selection::Words { .. } => &[&[Act::Copy, Act::SelectAllText]],
        Selection::Object { .. } => &[
            &[Act::EditObjectText, Act::ReplaceImage, Act::CopyObjectText],
            &[Act::DeleteObject],
        ],
        Selection::None | Selection::Pages(_) => &[
            &[Act::RotR, Act::RotL],
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
        for a in items {
            menu_item(ui, a, c, acts);
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
        let words = Selection::Words {
            page: 0,
            first: 0,
            last: 0,
        };
        for sel in [Selection::None, object, words] {
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
