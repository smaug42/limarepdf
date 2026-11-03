//! The File menu (☰), every command by kind (ENH-49): File's own commands first, in Word's
//! and LibreOffice's order, then a submenu for each of the band's names, in the band's
//! order and each laid out as its band's groups (the user: "mirror the band. That way if
//! it's odd to a user, it's at least consistent"), Help, Settings, and Close tab and Quit
//! last. The commands are the registry's ([`crate::actions`]); this is only where they go.

use crate::actions::{self, Act, Ctx, PageMove};
use crate::editor::{Align, Axis, TextAlign, Tool};
use crate::markup::Mark;
use crate::selection::Toward;
use crate::settings::{Accent, Layout, PageLook};
use eframe::egui::{self, ThemePreference};
use limarepdf_core::page_colours::PageColours;
use limarepdf_core::SignRole;
use std::path::{Path, PathBuf};

/// A place in the menu.
pub enum Entry {
    Cmd(Act),
    /// A submenu of these, by its name.
    Sub(&'static str, &'static [Entry]),
    /// A line between groups.
    Sep,
    /// Open recent's submenu: the recent files that still exist, then Clear recent files.
    Recent,
}

use Entry::{Cmd, Recent, Sep, Sub};

/// The menu's submenus after File's own commands: the band's names, then Help.
const KINDS: [&str; 8] = [
    "Edit",
    "Insert",
    "Review",
    "Pages",
    "Forms and sign",
    "Protect",
    "View",
    "Help",
];

pub static MENU: &[Entry] = &[
    Cmd(Act::NewBlank),
    Cmd(Act::NewFromImages),
    Cmd(Act::Open),
    Recent,
    Sep,
    Cmd(Act::Save),
    Cmd(Act::SaveAs),
    Cmd(Act::Reload),
    Sep,
    Sub(
        "Export",
        &[
            Cmd(Act::ExportImages),
            Cmd(Act::ExportText),
            Cmd(Act::SaveSelection),
        ],
    ),
    Cmd(Act::ReduceSize),
    Sep,
    Cmd(Act::PrintPreview),
    Cmd(Act::Print),
    Cmd(Act::PrintSelection),
    Sep,
    Cmd(Act::Props),
    Sep,
    Sub("Edit", EDIT),
    Sub("Insert", INSERT),
    Sub("Review", REVIEW),
    Sub("Pages", PAGES),
    Sub("Forms and sign", FORMS),
    Sub("Protect", PROTECT),
    Sub("View", VIEW),
    Sub("Help", HELP),
    Sep,
    Cmd(Act::Settings),
    Sep,
    Cmd(Act::CloseTab),
    Cmd(Act::Quit),
];

static EDIT: &[Entry] = &[
    Cmd(Act::Undo),
    Cmd(Act::Redo),
    Sep,
    // Clipboard, as menus order it.
    Cmd(Act::Cut),
    Cmd(Act::Copy),
    Cmd(Act::Paste),
    Cmd(Act::DeleteSelection),
    Sep,
    // Font and Alignment.
    Cmd(Act::Bold),
    Cmd(Act::Italic),
    Cmd(Act::Underline),
    Cmd(Act::Strikethrough),
    Sub(
        "Align text",
        &[
            Cmd(Act::AlignText(TextAlign::Left)),
            Cmd(Act::AlignText(TextAlign::Centre)),
            Cmd(Act::AlignText(TextAlign::Right)),
            Cmd(Act::AlignText(TextAlign::Justified)),
        ],
    ),
    Sep,
    // Editing.
    Cmd(Act::EditObjectText),
    Cmd(Act::Find),
    Cmd(Act::FindNext),
    Cmd(Act::FindPrevious),
    Cmd(Act::Replace),
    Cmd(Act::ReplaceMatch),
    Cmd(Act::ReplaceAll),
    Sub(
        "Select",
        &[
            Cmd(Act::Tool(Tool::Read)),
            Cmd(Act::Tool(Tool::Select)),
            Sep,
            Cmd(Act::SelectAllText),
            Cmd(Act::SelectAllObjects),
            Cmd(Act::SelectAllPages),
        ],
    ),
    Sep,
    // Text flow, and what acts on the text or picture selected.
    Cmd(Act::EditParagraph),
    Cmd(Act::MoveText),
    Cmd(Act::CopyObjectText),
    Cmd(Act::ReplaceImage),
    Sep,
    // Arrange.
    Sub(
        "Align objects",
        &[
            Cmd(Act::Align(Align::Left)),
            Cmd(Act::Align(Align::Centre)),
            Cmd(Act::Align(Align::Right)),
            Cmd(Act::Align(Align::Top)),
            Cmd(Act::Align(Align::Middle)),
            Cmd(Act::Align(Align::Bottom)),
            Sep,
            Cmd(Act::Distribute(Axis::Across)),
            Cmd(Act::Distribute(Axis::Down)),
            Sep,
            Cmd(Act::ToggleSnap),
        ],
    ),
];

static INSERT: &[Entry] = &[
    Cmd(Act::Tool(Tool::Text)),
    Cmd(Act::Tool(Tool::Image)),
    Sep,
    Cmd(Act::HeaderFooter),
    Cmd(Act::Watermark),
    Sep,
    Cmd(Act::Link),
    Sub(
        "Selected link",
        &[
            Cmd(Act::EditLink),
            Cmd(Act::OpenLink),
            Cmd(Act::CopyLinkAddress),
            Cmd(Act::RemoveLink),
        ],
    ),
];

static REVIEW: &[Entry] = &[
    Cmd(Act::Tool(Tool::Note)),
    Cmd(Act::Comments),
    // Right after Comments (batch 42's Q2).
    Cmd(Act::PrevComment),
    Cmd(Act::NextComment),
    Cmd(Act::CommentName),
    Sep,
    Cmd(Act::Mark(Mark::Highlight)),
    Cmd(Act::Mark(Mark::Underline)),
    Cmd(Act::Mark(Mark::StrikeOut)),
    Cmd(Act::Mark(Mark::Squiggly)),
    Cmd(Act::Tool(Tool::TextBox)),
    Sub(
        "Draw",
        &[
            Cmd(Act::Tool(Tool::Pen)),
            Cmd(Act::Tool(Tool::Rect)),
            Cmd(Act::Tool(Tool::Ellipse)),
            Cmd(Act::Tool(Tool::Line)),
        ],
    ),
    Cmd(Act::Tool(Tool::Stamp)),
    Sep,
    Cmd(Act::Spelling),
    Cmd(Act::SpellAsYouType),
    Cmd(Act::AddDictionary),
    Sep,
    Cmd(Act::FlattenAnnots),
];

static PAGES: &[Entry] = &[
    Cmd(Act::RotR),
    Cmd(Act::RotL),
    Cmd(Act::Extract),
    Cmd(Act::Delete),
    Cmd(Act::RemoveBlank),
    Sep,
    Cmd(Act::Blank),
    Cmd(Act::InsertImages),
    Cmd(Act::InsertPdf),
    Cmd(Act::Merge),
    Sub(
        "Move pages",
        &[
            Cmd(Act::MovePages(PageMove::Earlier)),
            Cmd(Act::MovePages(PageMove::Later)),
            Cmd(Act::MovePages(PageMove::Up)),
            Cmd(Act::MovePages(PageMove::Down)),
            Sep,
            Cmd(Act::MovePages(PageMove::Start)),
            Cmd(Act::MovePages(PageMove::End)),
        ],
    ),
    Cmd(Act::Split),
    Cmd(Act::Organise),
    Sep,
    Cmd(Act::Crop),
    Cmd(Act::PageLabels),
    Sep,
    Cmd(Act::Ocr),
    Cmd(Act::Scans),
];

static FORMS: &[Entry] = &[
    Cmd(Act::NextField),
    Cmd(Act::PrevField),
    Cmd(Act::ToggleFields),
    Cmd(Act::ClearField),
    Cmd(Act::ResetForm),
    Cmd(Act::FlattenForm),
    Sep,
    Cmd(Act::Tool(Tool::Sign)),
    Cmd(Act::CreateSign(SignRole::Signature)),
    Cmd(Act::CreateSign(SignRole::Initials)),
    Cmd(Act::FinishSigning),
    Sep,
    Sub(
        "Digital signatures",
        &[
            Cmd(Act::Signatures),
            Sep,
            Cmd(Act::SignDigital),
            Cmd(Act::Certify),
            Sep,
            Cmd(Act::DigitalIds),
            Cmd(Act::TrustedCertificates),
        ],
    ),
];

static PROTECT: &[Entry] = &[
    Cmd(Act::Tool(Tool::Redact)),
    Cmd(Act::MarkRedact),
    Cmd(Act::MarkResultsRedact),
    Cmd(Act::ApplyRedactions),
    Sep,
    Cmd(Act::Security),
    Cmd(Act::RemoveHidden),
];

static VIEW: &[Entry] = &[
    Cmd(Act::ZoomIn),
    Cmd(Act::ZoomOut),
    Cmd(Act::FitWidth),
    Cmd(Act::Actual),
    Sub(
        "Page layout",
        &[
            Cmd(Act::Layout(Layout::Continuous)),
            Cmd(Act::Layout(Layout::Facing)),
            Cmd(Act::Layout(Layout::Single)),
            Sep,
            Cmd(Act::ToggleCover),
        ],
    ),
    Sep,
    Sub(
        "Go to page",
        &[
            Cmd(Act::NextPage),
            Cmd(Act::PrevPage),
            Cmd(Act::FirstPage),
            Cmd(Act::LastPage),
        ],
    ),
    Sub(
        "Go to document",
        &[Cmd(Act::NextDocument), Cmd(Act::PrevDocument)],
    ),
    Sep,
    Cmd(Act::ToggleSide),
    Cmd(Act::PagesView),
    Cmd(Act::Bookmarks),
    Cmd(Act::SearchView),
    Cmd(Act::ToggleInspector),
    Cmd(Act::ToggleDock),
    Sep,
    Sub(
        "Theme",
        &[
            Cmd(Act::Theme(ThemePreference::System)),
            Cmd(Act::Theme(ThemePreference::Light)),
            Cmd(Act::Theme(ThemePreference::Dark)),
        ],
    ),
    Sub(
        "Accent colour",
        &[
            Cmd(Act::Accent(Accent::Desktop)),
            Cmd(Act::Accent(Accent::LimarePdf)),
        ],
    ),
    Sub(
        "Page colours",
        &[
            Cmd(Act::PageColours(PageLook::Fixed(PageColours::Normal))),
            Cmd(Act::PageColours(PageLook::Automatic)),
            Cmd(Act::PageColours(PageLook::Fixed(PageColours::Dark))),
            Cmd(Act::PageColours(PageLook::Fixed(PageColours::HighContrast))),
            Cmd(Act::PageColours(PageLook::Fixed(PageColours::Sepia))),
            Sep,
            Cmd(Act::TogglePicturesToo),
        ],
    ),
    Sub(
        "Show",
        &[
            Cmd(Act::ToggleHidden),
            Cmd(Act::TogglePageLabels),
            Cmd(Act::TogglePlanned),
            Cmd(Act::ToggleCompact),
        ],
    ),
];

static HELP: &[Entry] = &[
    Cmd(Act::Palette),
    Sep,
    Cmd(Act::UserGuide),
    Cmd(Act::ReportProblem),
    Sep,
    Cmd(Act::About),
];

/// The commands left out of the menu, with the kind the command search files each under:
/// what keys alone do (moving and extending the selection, Esc, F6), as word processors
/// leave it out of their menus; the hyphens' commands and Delete the object, which the
/// right-click menus offer where they act; and Keep recovery copies, which is in Settings.
pub const LEFT_OUT: &[(Act, &str)] = &[
    (Act::Escape, "Edit"),
    (Act::Nudge(-1, 0), "Edit"),
    (Act::Nudge(1, 0), "Edit"),
    (Act::Nudge(0, -1), "Edit"),
    (Act::Nudge(0, 1), "Edit"),
    (Act::Extend(Toward::Left), "Edit"),
    (Act::Extend(Toward::Right), "Edit"),
    (Act::Extend(Toward::Up), "Edit"),
    (Act::Extend(Toward::Down), "Edit"),
    (Act::Extend(Toward::LineStart), "Edit"),
    (Act::Extend(Toward::LineEnd), "Edit"),
    (Act::JoinHyphen, "Edit"),
    (Act::KeepHyphen, "Edit"),
    (Act::DeleteObject, "Edit"),
    (Act::NextRegion, "View"),
    (Act::PrevRegion, "View"),
    (Act::ToggleRecovery, "File"),
];

/// The submenus' names on the way to `act` in `entries`, and the entry before it there.
fn find<'a>(
    entries: &'a [Entry],
    act: Act,
    way: &mut Vec<&'static str>,
) -> Option<Option<&'a Entry>> {
    for (i, e) in entries.iter().enumerate() {
        match e {
            Cmd(a) if *a == act => return Some(i.checked_sub(1).map(|j| &entries[j])),
            Sub(name, inner) => {
                way.push(name);
                if let Some(before) = find(inner, act, way) {
                    return Some(before);
                }
                way.pop();
            }
            _ => {}
        }
    }
    None
}

/// The way to command `act` in the menu: the submenus' names, then its label; `None` when
/// it isn't there.
pub fn path(act: Act) -> Option<Vec<&'static str>> {
    let mut way = Vec::new();
    find(MENU, act, &mut way)?;
    way.push(actions::get(act).label);
    Some(way)
}

/// The way to `act` for messages, as menus are named in them: "Protect > Password
/// security…", "File > Properties…"; its label alone when it isn't in the menu.
pub fn way(act: Act) -> String {
    match path(act) {
        Some(p) if KINDS.contains(&p[0]) => p.join(" > "),
        Some(p) => format!("File > {}", p.join(" > ")),
        None => actions::get(act).label.to_string(),
    }
}

/// The kind the command search files `act` under: the band's name or Help that it is
/// under in the menu, else File.
pub fn category(act: Act) -> &'static str {
    if let Some(&(_, kind)) = LEFT_OUT.iter().find(|(a, _)| *a == act) {
        return kind;
    }
    match path(act) {
        Some(p) if p.len() > 1 && KINDS.contains(&p[0]) => p[0],
        _ => "File",
    }
}

/// The submenu `act` is in below its kind ("Theme" for Dark, "Export" for Export text…),
/// if any.
pub fn submenu(act: Act) -> Option<&'static str> {
    let p = path(act)?;
    let inner = p.len().checked_sub(2).map(|i| p[i])?;
    (inner != category(act)).then_some(inner)
}

/// Whether a line comes before `act` in its submenu, as in the band's menus and the
/// right-click menus, which take their lines from here.
pub fn line_before(act: Act) -> bool {
    matches!(find(MENU, act, &mut Vec::new()), Some(Some(Sep)))
}

/// The menu's submenus, in order, after File's own (Export).
#[cfg(test)]
pub fn submenus() -> Vec<&'static str> {
    (MENU.iter())
        .filter_map(|e| match e {
            Sub(name, _) if KINDS.contains(name) => Some(*name),
            _ => None,
        })
        .collect()
}

/// The menu's commands, in order.
#[cfg(test)]
pub fn order() -> Vec<Act> {
    fn walk(entries: &[Entry], out: &mut Vec<Act>) {
        for e in entries {
            match e {
                Cmd(a) => out.push(*a),
                Sub(_, inner) => walk(inner, out),
                Sep | Recent => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(MENU, &mut out);
    out
}

/// Draws the menu; `recent` is the recent files that still exist.
pub fn show(ui: &mut egui::Ui, c: &Ctx, recent: &[&PathBuf], acts: &mut Vec<Act>) {
    entries(ui, MENU, c, recent, acts);
}

fn entries(ui: &mut egui::Ui, list: &[Entry], c: &Ctx, recent: &[&PathBuf], acts: &mut Vec<Act>) {
    for e in list {
        match e {
            Cmd(a) => actions::menu_item(ui, actions::get(*a), c, acts),
            Sub(name, inner) => {
                ui.menu_button(*name, |ui| entries(ui, inner, c, recent, acts));
            }
            Sep => {
                ui.separator();
            }
            Recent => recent_menu(ui, recent, acts),
        }
    }
}

/// Open recent: the recent files that still exist, then Clear recent files.
fn recent_menu(ui: &mut egui::Ui, recent: &[&PathBuf], acts: &mut Vec<Act>) {
    ui.add_enabled_ui(!recent.is_empty(), |ui| {
        ui.menu_button("Open recent", |ui| {
            for (i, path) in recent.iter().enumerate() {
                let tip = path.display().to_string();
                let row = egui::Button::new(file_name(path)).truncate();
                if ui.add(row).on_hover_text(tip).clicked() {
                    acts.push(Act::OpenRecent(i));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Clear recent files").clicked() {
                acts.push(Act::ClearRecent);
                ui.close();
            }
        });
    });
}

/// A path's file name, or the whole path when it has none.
pub fn file_name(path: &Path) -> String {
    match path.file_name() {
        Some(n) => n.to_string_lossy().into(),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::ACTIONS;
    use crate::band::Band;

    /// ENH-49: the menu's submenus are the band's names in the band's order, then Help, so
    /// that a command is under the same name in both (the user: "if it's odd to a user, it's
    /// at least consistent").
    #[test]
    fn the_submenus_are_the_bands_names_in_order() {
        let mut want: Vec<&str> = Band::ALL.iter().map(|b| b.name()).collect();
        want.push("Help");
        assert_eq!(submenus(), want);
        assert_eq!(KINDS[..], want[..]);
    }

    /// ENH-49: each command a band holds is in the menu under that band's name (or another
    /// band's that holds it too), or among File's own.
    #[test]
    fn band_commands_are_under_their_bands_name() {
        let mut wrong = Vec::new();
        for band in Band::ALL {
            for act in band.acts() {
                let holders: Vec<&str> = (Band::ALL.iter())
                    .filter(|b| b.acts().contains(&act))
                    .map(|b| b.name())
                    .collect();
                match path(act) {
                    Some(p) if !KINDS.contains(&p[0]) || holders.contains(&p[0]) => {}
                    p => wrong.push(format!("{act:?} ({}): {p:?}", band.name())),
                }
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// ENH-49: a band's submenu has the band's groups in the band's order: the first of each
    /// group's commands there comes after the first of the group before.
    #[test]
    fn band_submenus_keep_the_bands_order() {
        let order = order();
        for band in Band::ALL {
            let mut last = None;
            for g in band.groups() {
                let here = (g.acts().iter())
                    .filter(|&&a| path(a).is_some_and(|p| p.len() > 1 && p[0] == band.name()))
                    .filter_map(|a| order.iter().position(|o| o == a))
                    .min();
                if let Some(at) = here {
                    assert!(
                        last.is_none_or(|l| l < at),
                        "{}'s {} is before the group above it",
                        band.name(),
                        g.name
                    );
                    last = Some(at);
                }
            }
        }
    }

    /// ENH-49: what keys alone do (moving and extending the selection, Esc, F6) isn't in the
    /// menu, as word processors leave it out of theirs; nor are the hyphens' commands, which
    /// the right-click menu offers where they act. Each still has its key or that menu.
    #[test]
    fn what_keys_do_is_left_out() {
        let keys_only = [
            Act::Escape,
            Act::Nudge(-1, 0),
            Act::Nudge(1, 0),
            Act::Nudge(0, -1),
            Act::Nudge(0, 1),
            Act::Extend(Toward::Left),
            Act::Extend(Toward::Right),
            Act::Extend(Toward::Up),
            Act::Extend(Toward::Down),
            Act::Extend(Toward::LineStart),
            Act::Extend(Toward::LineEnd),
            Act::NextRegion,
            Act::PrevRegion,
        ];
        for a in keys_only {
            assert_eq!(path(a), None, "{a:?}");
            assert!(!actions::get(a).keys.is_empty(), "{a:?} has no key");
            assert!(LEFT_OUT.iter().any(|(l, _)| *l == a), "{a:?}");
        }
        let typing = actions::typing_items();
        for a in [Act::JoinHyphen, Act::KeepHyphen] {
            assert_eq!(path(a), None, "{a:?}");
            assert!(typing.contains(&a), "{a:?} isn't in the right-click menu");
        }
    }

    /// ENH-49: every command is in the menu once, or left out on purpose, not both; each
    /// submenu name is used once.
    #[test]
    fn every_command_is_in_the_menu_once_or_left_out() {
        let order = order();
        for a in ACTIONS {
            let n = order.iter().filter(|&&o| o == a.act).count();
            let out = LEFT_OUT.iter().filter(|(l, _)| *l == a.act).count();
            assert_eq!((n + out), 1, "{:?}: {n} in the menu, {out} left out", a.act);
        }
        assert_eq!(order.len() + LEFT_OUT.len(), ACTIONS.len());
        fn names(entries: &[Entry], seen: &mut Vec<&'static str>) {
            for e in entries {
                if let Sub(name, inner) = e {
                    assert!(!seen.contains(name), "{name} twice");
                    seen.push(name);
                    names(inner, seen);
                }
            }
        }
        names(MENU, &mut Vec::new());
    }

    /// ENH-49: no line starts or ends a submenu, nor two follow each other.
    #[test]
    fn lines_only_divide() {
        fn check(name: &str, entries: &[Entry]) {
            let first = entries.first().is_some_and(|e| matches!(e, Sep));
            let last = entries.last().is_some_and(|e| matches!(e, Sep));
            assert!(!first && !last, "{name}");
            for w in entries.windows(2) {
                assert!(!matches!((&w[0], &w[1]), (Sep, Sep)), "{name}");
            }
            for e in entries {
                if let Sub(n, inner) = e {
                    check(n, inner);
                }
            }
        }
        check("File", MENU);
    }

    /// The ways messages give, and the kind and submenu the command search shows.
    #[test]
    fn ways_kinds_and_submenus() {
        assert_eq!(way(Act::Security), "Protect > Password security…");
        assert_eq!(way(Act::Props), "File > Properties…");
        assert_eq!(way(Act::ExportText), "File > Export > Export text…");
        assert_eq!(
            way(Act::DigitalIds),
            "Forms and sign > Digital signatures > Digital IDs…"
        );
        assert_eq!(category(Act::ExportText), "File");
        assert_eq!(submenu(Act::ExportText), Some("Export"));
        assert_eq!(category(Act::Theme(ThemePreference::Dark)), "View");
        assert_eq!(submenu(Act::Theme(ThemePreference::Dark)), Some("Theme"));
        assert_eq!(category(Act::Watermark), "Insert");
        assert_eq!(submenu(Act::Watermark), None);
        assert_eq!(category(Act::JoinHyphen), "Edit");
        assert!(line_before(Act::Distribute(Axis::Across)));
        assert!(!line_before(Act::Distribute(Axis::Down)));
        assert!(!line_before(Act::NewFromImages));
    }
}
