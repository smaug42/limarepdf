//! The command band (ENH-38, build plan 2.2): the names Edit (the mockup's Home, renamed by
//! the user, 2026-10-27), Insert, Review, Pages, Forms and sign, Protect and View, and below
//! them the chosen one's groups of labelled commands, divided (their names show only once
//! folded: the user, 2026-10-27, as the band was tall and the groups read as what they are).
//! Every button runs a command from the registry ([`crate::actions`]), which says whether it
//! is available and whether it is on; a caret beside a button opens related commands.
//! Features planned but not built show greyed where they will be, marked Planned, and run
//! nothing (ENH-41). Groups without room
//! fold, from the right, into a button with the group's name that opens them; groups of
//! planned features alone are left out first.

use crate::actions::{self, Act, Ctx};
use crate::editor::Tool;
use crate::icon;
use crate::markup::{self, Mark};
use crate::settings::{Accent, Layout, PageLook};
use crate::signing::SignMenu;
use crate::theme;
use eframe::egui::{self, Rect, RichText, ThemePreference};
use limarepdf_core::page_colours::PageColours;
use limarepdf_core::SignRole;

/// One of the band's names, chosen above it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Band {
    #[default]
    Edit,
    Insert,
    Review,
    Pages,
    Forms,
    Protect,
    View,
}

impl Band {
    pub const ALL: [Band; 7] = [
        Band::Edit,
        Band::Insert,
        Band::Review,
        Band::Pages,
        Band::Forms,
        Band::Protect,
        Band::View,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Band::Edit => "Edit",
            Band::Insert => "Insert",
            Band::Review => "Review",
            Band::Pages => "Pages",
            Band::Forms => "Forms and sign",
            Band::Protect => "Protect",
            Band::View => "View",
        }
    }

    pub fn groups(self) -> &'static [Group] {
        match self {
            Band::Edit => EDIT,
            Band::Insert => INSERT,
            Band::Review => REVIEW,
            Band::Pages => PAGES,
            Band::Forms => FORMS,
            Band::Protect => PROTECT,
            Band::View => VIEW,
        }
    }
}

/// A named group of commands in a band.
pub struct Group {
    pub name: &'static str,
    pub items: &'static [Item],
}

impl Group {
    /// Its controls change the text being typed, which they leave open: a click on them
    /// doesn't end the typing.
    fn keeps_typing(&self) -> bool {
        self.items.iter().any(|i| match i {
            Item::Font => true,
            Item::Button(b) => matches!(b.act, Some(Act::Paste | Act::Cut | Act::Copy)),
            _ => false,
        })
    }

    /// Something in it shows: with planned features hidden, a group of them alone doesn't.
    fn shown(&self, planned: bool) -> bool {
        planned || !self.all_planned()
    }

    /// It holds planned features only: it folds first, before groups of built commands.
    fn all_planned(&self) -> bool {
        self.items.iter().all(|i| matches!(i, Item::Planned(_)))
    }

    /// The name screen readers give its first control.
    #[cfg(test)]
    pub fn first_name(&self) -> String {
        match &self.items[0] {
            Item::Button(b) => b.label.into(),
            Item::Font => "Font".into(),
            Item::Sign => "Fill and sign".into(),
            Item::Marks => "Highlight".into(),
            Item::Planned(p) => p.name(),
        }
    }
}

pub enum Item {
    Button(Button),
    /// Edit's font list and size, Bold, Italic and the text colour, for the text selected
    /// or being typed (the editor's), with the planned Underline and Strike-through.
    Font,
    /// Fill and sign, its menu the saved signatures and initials, the date, name and marks.
    Sign,
    /// Highlight, its menu the other marks and their colours.
    Marks,
    Planned(&'static Planned),
}

/// A command's button: its label and icon here, its state from the registry. With `menu`,
/// a caret beside it opens related commands; with no `act`, the button itself opens them.
pub struct Button {
    pub act: Option<Act>,
    pub label: &'static str,
    pub icon: &'static str,
    pub menu: &'static [Act],
}

/// A feature planned but not built (ENH-41), shown greyed where it will be.
pub struct Planned {
    pub label: &'static str,
    pub icon: &'static str,
    /// The build plan's item that builds it (for the plan's records; not shown).
    #[cfg_attr(not(test), allow(dead_code))]
    pub item: &'static str,
    /// What it will do, said on hover and to screen readers.
    pub what: &'static str,
}

impl Planned {
    /// Its name for screen readers: what it is, and that it is planned.
    pub fn name(&self) -> String {
        format!("{} — Planned", self.label)
    }
}

const fn b(act: Act, label: &'static str, icon: &'static str) -> Item {
    Item::Button(Button {
        act: Some(act),
        label,
        icon,
        menu: &[],
    })
}

const fn split(act: Act, label: &'static str, icon: &'static str, menu: &'static [Act]) -> Item {
    Item::Button(Button {
        act: Some(act),
        label,
        icon,
        menu,
    })
}

const fn menu(label: &'static str, icon: &'static str, items: &'static [Act]) -> Item {
    Item::Button(Button {
        act: None,
        label,
        icon,
        menu: items,
    })
}

const fn group(name: &'static str, items: &'static [Item]) -> Group {
    Group { name, items }
}

// The planned features, each with the build plan's item that builds it.
pub const UNDERLINE: Planned = Planned {
    label: "Underline",
    icon: icon::TEXT_UNDERLINE,
    item: "NEW-07",
    what: "Underline or strike through the selected text.",
};
const REPLACE: Planned = Planned {
    label: "Replace",
    icon: icon::SWAP,
    item: "NEW-04",
    what: "Find text and replace it.",
};
const PARAGRAPH: Planned = Planned {
    label: "Paragraph",
    icon: icon::TEXT_ALIGN_LEFT,
    item: "NEW-01",
    what: "Edit a paragraph as one, its lines rewrapping as you type.",
};
const ALIGN: Planned = Planned {
    label: "Align",
    icon: icon::ALIGN_LEFT,
    item: "NEW-08",
    what: "Line up and space out the selected objects.",
};
const LINK: Planned = Planned {
    label: "Link",
    icon: icon::LINK,
    item: "ENH-17",
    what: "Add a link to a web page or a place in the document.",
};
const BLANK_DOCUMENT: Planned = Planned {
    label: "Blank document",
    icon: icon::FILE_PLUS,
    item: "NEW-09",
    what: "Start a new, empty document.",
};
const SPELLING: Planned = Planned {
    label: "Spelling",
    icon: icon::TEXT_AA,
    item: "NEW-06",
    what: "Check the spelling of the text you type.",
};
const EXPORT_COMMENTS: Planned = Planned {
    label: "Export comments",
    icon: icon::FILE_ARROW_UP,
    item: "PF-05",
    what: "Save the comments as a summary or for another app.",
};
const CREATE_FIELDS: Planned = Planned {
    label: "Create fields",
    icon: icon::LIST_PLUS,
    item: "PF-04",
    what: "Add form fields for others to fill in.",
};
const YOUR_AI: Planned = Planned {
    label: "Your AI",
    icon: icon::SPARKLE,
    item: "ENH-42",
    what: "Connect an AI of your own to ask about the document. Off unless you connect it.",
};
/// The planned features in the File menu.
pub const FILE_PLANNED: [&Planned; 2] = [
    &Planned {
        label: "Print preview",
        icon: icon::PRINTER,
        item: "ENH-14",
        what: "See the pages as they will print, with paper size, orientation and scaling.",
    },
    &Planned {
        label: "Reduce file size",
        icon: icon::FILE_ARROW_UP,
        item: "ENH-19",
        what: "Make the file smaller, with a preview of the result.",
    },
];

static EDIT: &[Group] = &[
    group(
        "Clipboard",
        &[
            b(Act::Paste, "Paste", icon::CLIPBOARD_TEXT),
            b(Act::Cut, "Cut", icon::SCISSORS),
            b(Act::Copy, "Copy", icon::COPY),
        ],
    ),
    group("Font", &[Item::Font]),
    group(
        "Editing",
        &[
            b(Act::EditObjectText, "Edit text", icon::CURSOR_TEXT),
            b(Act::Find, "Find", icon::MAGNIFYING_GLASS),
            Item::Planned(&REPLACE),
            menu(
                "Select",
                icon::CURSOR,
                &[
                    Act::Tool(Tool::Read),
                    Act::Tool(Tool::Select),
                    Act::SelectAllText,
                    Act::SelectAllObjects,
                    Act::SelectAllPages,
                ],
            ),
        ],
    ),
    group("Text flow", &[Item::Planned(&PARAGRAPH)]),
    group("Arrange", &[Item::Planned(&ALIGN)]),
];

static INSERT: &[Group] = &[
    group(
        "Content",
        &[
            b(Act::Tool(Tool::Text), "Text", icon::TEXT_T),
            b(Act::Tool(Tool::Image), "Image", icon::IMAGE),
        ],
    ),
    group(
        "Page marks",
        &[
            b(Act::HeaderFooter, "Header and footer", icon::ARTICLE),
            b(Act::Watermark, "Watermark", icon::DROP_HALF),
        ],
    ),
    group("Navigation", &[Item::Planned(&LINK)]),
    group(
        "Document",
        &[
            Item::Planned(&BLANK_DOCUMENT),
            b(Act::NewFromImages, "From images", icon::IMAGES),
        ],
    ),
];

static REVIEW: &[Group] = &[
    group(
        "Comments",
        &[
            b(Act::Tool(Tool::Note), "Note", icon::NOTE),
            split(
                Act::Comments,
                "Comments",
                icon::CHAT_TEXT,
                &[Act::CommentName],
            ),
        ],
    ),
    group(
        "Markup",
        &[
            Item::Marks,
            b(Act::Tool(Tool::TextBox), "Text box", icon::TEXTBOX),
            split(
                Act::Tool(Tool::Pen),
                "Draw",
                icon::PENCIL_SIMPLE,
                &[
                    Act::Tool(Tool::Pen),
                    Act::Tool(Tool::Rect),
                    Act::Tool(Tool::Ellipse),
                    Act::Tool(Tool::Line),
                ],
            ),
            b(Act::Tool(Tool::Stamp), "Stamp", icon::STAMP),
        ],
    ),
    group("Proofing", &[Item::Planned(&SPELLING)]),
    group(
        "Review",
        &[
            Item::Planned(&EXPORT_COMMENTS),
            b(Act::FlattenAnnots, "Flatten…", icon::STACK),
        ],
    ),
];

static PAGES: &[Group] = &[
    group(
        "Page actions",
        &[
            split(
                Act::RotR,
                "Rotate",
                icon::ARROW_CLOCKWISE,
                &[Act::RotR, Act::RotL],
            ),
            b(Act::Extract, "Extract", icon::FILES),
            split(
                Act::Delete,
                "Delete",
                icon::TRASH,
                &[Act::Delete, Act::RemoveBlank],
            ),
        ],
    ),
    group(
        "Add and arrange",
        &[
            menu(
                "Insert pages",
                icon::FILE_PLUS,
                &[Act::Blank, Act::InsertImages, Act::InsertPdf, Act::Merge],
            ),
            menu("Move", icon::ARROWS_OUT_CARDINAL, &actions::MOVES),
            b(Act::Split, "Split", icon::SCISSORS),
            b(Act::Organise, "Organise", icon::SQUARES_FOUR),
        ],
    ),
    group(
        "Page setup",
        &[
            b(Act::Crop, "Crop", icon::CROP),
            b(Act::PageLabels, "Page labels", icon::TAG),
        ],
    ),
    group(
        "Scanned pages",
        &[
            b(Act::Ocr, "Recognise text", icon::SCAN),
            b(Act::Scans, "Optimise", icon::MAGIC_WAND),
        ],
    ),
];

static FORMS: &[Group] = &[
    group(
        "Complete a form",
        &[
            split(
                Act::NextField,
                "Fill fields",
                icon::LIST_CHECKS,
                &[
                    Act::NextField,
                    Act::PrevField,
                    Act::ToggleFields,
                    Act::ClearField,
                    Act::ResetForm,
                    Act::FlattenForm,
                ],
            ),
            Item::Sign,
        ],
    ),
    group(
        "Certificate signatures",
        &[menu(
            "Digital signature",
            icon::SEAL_CHECK,
            &[
                Act::SignDigital,
                Act::Certify,
                Act::Signatures,
                Act::DigitalIds,
                Act::TrustedCertificates,
            ],
        )],
    ),
    group("Authoring", &[Item::Planned(&CREATE_FIELDS)]),
];

static PROTECT: &[Group] = &[
    group(
        "Redaction",
        &[
            split(
                Act::Tool(Tool::Redact),
                "Mark redaction",
                icon::SELECTION,
                &[
                    Act::Tool(Tool::Redact),
                    Act::MarkRedact,
                    Act::MarkResultsRedact,
                ],
            ),
            b(Act::ApplyRedactions, "Apply redactions…", icon::ERASER),
        ],
    ),
    group(
        "Security",
        &[
            b(Act::Security, "Passwords", icon::KEY),
            b(Act::RemoveHidden, "Remove hidden data", icon::SHIELD_CHECK),
        ],
    ),
];

static VIEW: &[Group] = &[
    group(
        "Document view",
        &[
            b(Act::FitWidth, "Fit width", icon::ARROWS_HORIZONTAL),
            split(
                Act::Layout(Layout::Facing),
                "Facing pages",
                icon::BOOK_OPEN,
                &[
                    Act::Layout(Layout::Continuous),
                    Act::Layout(Layout::Facing),
                    Act::Layout(Layout::Single),
                    Act::ToggleCover,
                ],
            ),
            menu(
                "Zoom",
                icon::MAGNIFYING_GLASS_PLUS,
                &[Act::ZoomIn, Act::ZoomOut, Act::Actual, Act::FitWidth],
            ),
        ],
    ),
    group(
        "Workspace",
        &[
            split(
                Act::ToggleSide,
                "Pages panel",
                icon::SIDEBAR_SIMPLE,
                &[
                    Act::ToggleSide,
                    Act::PagesView,
                    Act::Bookmarks,
                    Act::Comments,
                    Act::SearchView,
                    Act::Signatures,
                ],
            ),
            split(
                Act::ToggleInspector,
                "Properties",
                icon::SLIDERS_HORIZONTAL,
                &[Act::ToggleInspector, Act::ToggleDock],
            ),
        ],
    ),
    group(
        "Appearance",
        &[
            menu(
                "Theme",
                icon::CIRCLE_HALF,
                &[
                    Act::Theme(ThemePreference::System),
                    Act::Theme(ThemePreference::Light),
                    Act::Theme(ThemePreference::Dark),
                ],
            ),
            menu(
                "Accent colour",
                icon::SWATCHES,
                &[Act::Accent(Accent::Desktop), Act::Accent(Accent::LimarePdf)],
            ),
            menu(
                "Page colours",
                icon::PALETTE,
                &[
                    Act::PageColours(PageLook::Fixed(PageColours::Normal)),
                    Act::PageColours(PageLook::Automatic),
                    Act::PageColours(PageLook::Fixed(PageColours::Dark)),
                    Act::PageColours(PageLook::Fixed(PageColours::HighContrast)),
                    Act::PageColours(PageLook::Fixed(PageColours::Sepia)),
                    Act::TogglePicturesToo,
                ],
            ),
            menu(
                "Show",
                icon::EYE,
                &[
                    Act::ToggleHidden,
                    Act::ToggleFields,
                    Act::TogglePageLabels,
                    Act::TogglePlanned,
                    Act::ToggleCompact,
                ],
            ),
        ],
    ),
    group("Optional assistance", &[Item::Planned(&YOUR_AI)]),
];

/// Draws Edit's Font group for the text it acts on, calling the function it is given
/// between Italic and the colour (the planned Underline and Strike-through).
pub type FontGroup<'a> = dyn FnMut(&mut egui::Ui, &dyn Fn(&mut egui::Ui)) + 'a;

/// What the band needs from the app to draw it.
pub struct Parts<'a> {
    pub c: &'a Ctx,
    /// The saved signatures for Fill and sign's menu.
    pub sign: Option<&'a SignMenu>,
    pub font: &'a mut FontGroup<'a>,
    /// The commands on one row, as icons ([`Act::ToggleCompact`], or a short window).
    pub compact: bool,
}

/// The height of a group's controls: the Font group's two rows, or one row when compact.
fn controls(compact: bool) -> f32 {
    if compact {
        BUTTON
    } else {
        54.0
    }
}
/// The room at either side of a group, inside its divider.
const PAD: f32 = 10.0;
/// The height of a button.
const BUTTON: f32 = 28.0;
/// Extra room a folded group needs before it comes back, so that a width at the boundary
/// doesn't fold and unfold it by turns.
const UNFOLD_MARGIN: f32 = 12.0;

/// The band's names, the chosen one shown as such; a click chooses another.
pub fn names(ui: &mut egui::Ui, band: &mut Band) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for b in Band::ALL {
            if name_tab(ui, b.name(), *band == b).clicked() {
                *band = b;
            }
        }
    });
}

/// The button that makes the commands compact (`compact`) or full again, greyed while a
/// short window keeps them compact (`forced`). Returns whether it was clicked.
pub fn compact_toggle(ui: &mut egui::Ui, compact: bool, forced: bool) -> bool {
    let a = actions::get(Act::ToggleCompact);
    let glyph = if compact {
        icon::CARET_DOWN
    } else {
        icon::CARET_UP
    };
    let b = egui::Button::new(theme::icon(glyph).size(14.0))
        .frame_when_inactive(false)
        .min_size(egui::vec2(24.0, 24.0));
    let r = ui.add_enabled(!forced, b);
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, !forced, compact, a.label));
    let tip = match (forced, compact) {
        (true, _) => "Compact while the window is short",
        (false, true) => "Full commands, with their names",
        (false, false) => "Compact commands: on one row, as icons",
    };
    r.on_hover_text(tip).on_disabled_hover_text(tip).clicked()
}

/// A band's name above it: its background and an accent line under it when chosen.
fn name_tab(ui: &mut egui::Ui, name: &str, on: bool) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let family = if on {
        theme::medium()
    } else {
        egui::FontFamily::Proportional
    };
    let color = if on {
        theme::accent(dark)
    } else {
        ui.visuals().text_color()
    };
    let galley = ui
        .painter()
        .layout_no_wrap(name.into(), egui::FontId::new(14.0, family), color);
    let size = galley.size() + egui::vec2(24.0, 10.0);
    let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let top = egui::CornerRadius {
            nw: 5,
            ne: 5,
            sw: 0,
            se: 0,
        };
        if on || r.hovered() {
            p.rect_filled(rect, top, theme::accent_bg(dark));
        }
        if on {
            let line = Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 2.0), rect.max);
            p.rect_filled(line, 0, theme::accent(dark));
        }
        if r.has_focus() {
            let ring = ui.visuals().selection.stroke;
            p.rect_stroke(rect, top, ring, egui::StrokeKind::Inside);
        }
        p.galley(rect.center() - galley.size() / 2.0, galley, color);
    }
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Tab, true, name));
    ui.ctx()
        .accesskit_node_builder(r.id, |n| n.set_selected(on));
    r
}

/// The groups of `band`, side by side, divided; those without room folded, groups of
/// planned features first, then from the right ([`fold_order`]). Returns the places of the
/// groups whose controls leave the typing open.
pub fn show(ui: &mut egui::Ui, band: Band, parts: &mut Parts, acts: &mut Vec<Act>) -> Vec<Rect> {
    let planned = parts.c.planned;
    let groups: Vec<&Group> = band.groups().iter().filter(|g| g.shown(planned)).collect();
    let order = fold_order(&groups);
    // The groups' widths as last drawn, and their folded buttons', full or compact;
    // estimated until drawn.
    let compact = parts.compact;
    let key =
        |what: &str, i: usize| egui::Id::unique(("band-width", band.name(), what, i, compact));
    let width = |what: &str, i: usize, guess: f32| {
        ui.data(|d| d.get_temp::<f32>(key(what, i)))
            .unwrap_or(guess)
    };
    // A group of planned features alone, without room, is left out rather than folded.
    let need = |folded: usize| -> f32 {
        (0..groups.len())
            .map(|i| match order[..folded].contains(&i) {
                true if groups[i].all_planned() => 0.0,
                true => width("folded", i, 110.0),
                false => width("open", i, 160.0),
            })
            .sum()
    };
    let room = ui.available_width();
    let level = egui::Id::unique(("band-folded", band.name(), compact));
    let before = ui.data(|d| d.get_temp::<usize>(level)).unwrap_or(0);
    let folded = fold_level(before, groups.len(), room, need);
    ui.data_mut(|d| d.insert_temp(level, folded));

    let mut keep = Vec::new();
    // The groups' tops in line, their controls of one height.
    let row = egui::Layout::left_to_right(egui::Align::Min);
    ui.with_layout(row, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let n = groups.len();
        let left_out = |i: usize| order[..folded].contains(&i) && groups[i].all_planned();
        for (i, g) in groups.iter().enumerate() {
            if left_out(i) {
                continue;
            }
            if order[..folded].contains(&i) {
                let r = folded_group(ui, g, parts, acts);
                ui.data_mut(|d| d.insert_temp(key("folded", i), r.width()));
                continue;
            }
            let more = (i + 1..n).any(|j| !left_out(j));
            let r = group_ui(ui, g, parts, acts, more);
            ui.data_mut(|d| d.insert_temp(key("open", i), r.width()));
            if g.keeps_typing() {
                keep.push(r);
            }
        }
    });
    keep
}

/// The order `groups` fold in as room runs out: those of planned features alone first (left
/// out, as they run nothing), so that they never crowd out built commands, then the rest;
/// each from the right.
fn fold_order(groups: &[&Group]) -> Vec<usize> {
    let from_right = (0..groups.len()).rev();
    let (mut planned, rest): (Vec<usize>, Vec<usize>) =
        from_right.partition(|&i| groups[i].all_planned());
    planned.extend(rest);
    planned
}

/// How many of `n` groups to fold, in their order, given last frame's count `prev`, the
/// room and the width the band needs with each count folded.
fn fold_level(prev: usize, n: usize, room: f32, need: impl Fn(usize) -> f32) -> usize {
    let mut folded = prev.min(n);
    while folded > 0 && need(folded - 1) + UNFOLD_MARGIN <= room {
        folded -= 1;
    }
    while folded < n && need(folded) > room {
        folded += 1;
    }
    folded
}

/// One group: its controls, and a divider at its right (`divide`).
/// Returns where it is.
fn group_ui(
    ui: &mut egui::Ui,
    g: &Group,
    parts: &mut Parts,
    acts: &mut Vec<Act>,
    divide: bool,
) -> Rect {
    let dark = ui.visuals().dark_mode;
    let height = controls(parts.compact);
    let r = ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
        ui.add_space(PAD);
        ui.spacing_mut().item_spacing = egui::vec2(3.0, 4.0);
        let row = egui::vec2(ui.available_width(), height);
        let layout = egui::Layout::left_to_right(egui::Align::Center);
        ui.allocate_ui_with_layout(row, layout, |ui| {
            ui.set_min_height(height);
            field_look(ui.style_mut());
            items(ui, g, parts, acts);
        });
        ui.add_space(PAD);
    });
    let rect = r.response.rect;
    if divide {
        let x = rect.right() - 0.5;
        let stroke = egui::Stroke::new(1.0, theme::border(dark));
        ui.painter().vline(x, rect.top()..=rect.bottom(), stroke);
    }
    rect
}

/// Lists, fields and on/off buttons in the band outlined, as in the workspace; plain buttons
/// stay unframed until hovered.
fn field_look(s: &mut egui::Style) {
    let dark = s.visuals.dark_mode;
    let w = &mut s.visuals.widgets;
    w.inactive.bg_stroke = egui::Stroke::new(1.0, theme::border(dark));
    w.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    w.inactive.bg_fill = egui::Color32::TRANSPARENT;
}

/// A group without room: a button with its name, which opens its controls.
fn folded_group(ui: &mut egui::Ui, g: &Group, parts: &mut Parts, acts: &mut Vec<Act>) -> Rect {
    let height = controls(parts.compact);
    let row = egui::vec2(ui.available_width(), height);
    let centred = egui::Layout::left_to_right(egui::Align::Center);
    let r = ui.allocate_ui_with_layout(row, centred, |ui| {
        ui.set_min_height(height);
        ui.add_space(2.0);
        let caret = egui::Atom::from(theme::icon(icon::CARET_DOWN).size(10.0));
        let label = (g.name, caret);
        let button = egui::Button::new(label)
            .frame_when_inactive(false)
            .min_size(egui::vec2(0.0, BUTTON));
        let config = egui::containers::menu::MenuConfig::new()
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let (r, _) = egui::containers::menu::MenuButton::from_button(button)
            .config(config)
            .ui(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    items(ui, g, parts, acts);
                });
            });
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, g.name));
        ui.add_space(2.0);
    });
    r.response.rect
}

/// A group's controls, in a row.
fn items(ui: &mut egui::Ui, g: &Group, parts: &mut Parts, acts: &mut Vec<Act>) {
    let (c, compact) = (parts.c, parts.compact);
    let button = |ui: &mut egui::Ui, b: &Button, acts: &mut Vec<Act>| {
        button(ui, b, c, acts, compact);
    };
    for item in g.items {
        match item {
            Item::Button(b) => button(ui, b, acts),
            Item::Planned(p) if c.planned => planned(ui, p, compact),
            Item::Planned(_) => {}
            Item::Font => {
                let show = c.planned;
                (parts.font)(ui, &|ui| {
                    if show {
                        planned_style(ui, &UNDERLINE, "U", compact);
                    }
                });
            }
            Item::Sign => {
                let sign = Button {
                    act: Some(Act::Tool(Tool::Sign)),
                    label: "Fill and sign",
                    icon: icon::SIGNATURE,
                    menu: &[],
                };
                button(ui, &sign, acts);
                if let Some(m) = parts.sign {
                    caret(ui, "Fill and sign choices", |ui| {
                        crate::signing::menu_ui(ui, m, c, acts);
                    });
                } else {
                    let create = [
                        Act::CreateSign(SignRole::Signature),
                        Act::CreateSign(SignRole::Initials),
                        Act::FinishSigning,
                    ];
                    caret(ui, "Fill and sign choices", |ui| {
                        for a in create {
                            actions::menu_item(ui, actions::get(a), c, acts);
                        }
                    });
                }
            }
            Item::Marks => {
                let mark = Button {
                    act: Some(Act::Mark(Mark::Highlight)),
                    label: "Highlight",
                    icon: icon::HIGHLIGHTER,
                    menu: &[],
                };
                button(ui, &mark, acts);
                caret(ui, "Highlight options", |ui| markup::menu(ui, c, acts));
            }
        }
    }
}

/// Whether `act` is on: a tool in use, or an on/off command that is on.
fn is_on(act: Act, c: &Ctx) -> bool {
    match act {
        Act::Tool(t) => c.tool == Some(t) && t != Tool::Read,
        // Edit text is the Text tool, with nothing selected.
        Act::EditObjectText => c.tool == Some(Tool::Text),
        _ => actions::get(act).checked.is_some_and(|on| on(c)),
    }
}

/// A command's button, and the caret for its menu; or, with no command, a button that
/// opens the menu. Shown pressed while the command is on, or a tool in its menu is in use
/// (a menu's ticked choices don't press it). `compact`: its icon alone, named on hover.
fn button(ui: &mut egui::Ui, b: &Button, c: &Ctx, acts: &mut Vec<Act>, compact: bool) {
    let tool = |a: &Act| matches!(a, Act::Tool(_)) && is_on(*a, c);
    let on = b.act.is_some_and(|a| is_on(a, c)) || b.menu.iter().any(tool);
    let glyph = theme::icon(b.icon).size(16.0);
    let Some(act) = b.act else {
        let enabled = b.menu.iter().any(|&a| (actions::get(a).enabled)(c));
        let caret = egui::Atom::from(theme::icon(icon::CARET_DOWN).size(10.0));
        let button = match compact {
            true => egui::Button::new((glyph, caret)),
            false => egui::Button::new((glyph, b.label, caret)),
        }
        .frame_when_inactive(on)
        .selected(on)
        .min_size(egui::vec2(0.0, BUTTON));
        ui.add_enabled_ui(enabled, |ui| {
            let (r, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                for &a in b.menu {
                    actions::menu_item(ui, actions::get(a), c, acts);
                }
            });
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, b.label));
            let r = if compact { r.on_hover_text(b.label) } else { r };
            if let Some(&first) = b.menu.first() {
                let why = actions::why_not(c, actions::get(first));
                r.on_disabled_hover_text(format!("{}\n{why}", b.label));
            }
        });
        return;
    };
    let a = actions::get(act);
    let enabled = (a.enabled)(c);
    let button = match compact {
        true => egui::Button::new(glyph).min_size(egui::vec2(BUTTON, BUTTON)),
        false => egui::Button::new((glyph, b.label)).min_size(egui::vec2(0.0, BUTTON)),
    }
    .frame_when_inactive(on)
    .selected(on);
    let r = ui.add_enabled(enabled, button);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, b.label));
    let tip = actions::tooltip(ui.ctx(), a);
    let greyed = actions::greyed_tip(ui.ctx(), a, c);
    if r.on_hover_text(&tip)
        .on_disabled_hover_text(&greyed)
        .clicked()
    {
        acts.push(act);
    }
    if !b.menu.is_empty() {
        caret(ui, &format!("{} options", b.label), |ui| {
            for &m in b.menu {
                actions::menu_item(ui, actions::get(m), c, acts);
            }
        });
    }
}

/// The small caret right of a button, opening a menu of related commands; `name` for
/// screen readers and on hover.
fn caret(ui: &mut egui::Ui, name: &str, menu: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(-ui.spacing().item_spacing.x);
    let more = egui::Button::new(theme::icon(icon::CARET_DOWN).size(10.0))
        .frame_when_inactive(false)
        .min_size(egui::vec2(16.0, BUTTON));
    let (r, _) = egui::containers::menu::MenuButton::from_button(more).ui(ui, menu);
    r.on_hover_text(name)
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
}

/// A planned feature's button: greyed, marked Planned, never run, and no Tab stop.
fn planned(ui: &mut egui::Ui, p: &Planned, compact: bool) {
    let glyph = theme::icon(p.icon).size(16.0);
    let button = match compact {
        true => egui::Button::new(glyph).min_size(egui::vec2(BUTTON, BUTTON)),
        false => {
            egui::Button::new((glyph, p.label, planned_tag(ui))).min_size(egui::vec2(0.0, BUTTON))
        }
    }
    .frame_when_inactive(false);
    planned_button(ui, button, p);
}

/// A planned text style's button in the Font group: its letter underlined, greyed and
/// marked Planned (`compact`: the letter alone, named on hover).
fn planned_style(ui: &mut egui::Ui, p: &Planned, letter: &str, compact: bool) {
    let text = RichText::new(letter).size(15.0).underline();
    let button = match compact {
        true => egui::Button::new(text),
        false => egui::Button::new((text, planned_tag(ui))),
    }
    .min_size(egui::vec2(26.0, 26.0));
    planned_button(ui, button, p);
}

/// Shows the button of planned feature `p`: greyed, named for screen readers, its Planned
/// tag outlined.
fn planned_button(ui: &mut egui::Ui, button: egui::Button, p: &Planned) {
    let r = ui.add_enabled_ui(false, |ui| button.atom_ui(ui)).inner;
    outline_tag(ui, &r);
    let name = p.name();
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, false, &name));
    r.response
        .on_disabled_hover_text(format!("{} Planned.", p.what));
}

/// The atom id of a planned feature's tag, to find where it was drawn.
fn tag_id() -> egui::IdSalt {
    egui::IdSalt::new("planned-tag")
}

/// The small "Planned" tag inside a planned feature's button, a thin space either side
/// inside its outline.
fn planned_tag(ui: &egui::Ui) -> egui::Atom<'static> {
    let dark = ui.visuals().dark_mode;
    let text = RichText::new("\u{2009}Planned\u{2009}")
        .size(11.0)
        .color(theme::muted(dark));
    egui::AtomExt::atom_id(text, tag_id())
}

/// A dashed outline round the Planned tag of the button `r`, as the workspace marks planned
/// features; none when it has no tag (compact).
fn outline_tag(ui: &egui::Ui, r: &egui::WidgetAtomResponse) {
    let Some(tag) = r.rect(tag_id()) else {
        return;
    };
    let line = egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
    let corners = [
        tag.left_top(),
        tag.right_top(),
        tag.right_bottom(),
        tag.left_bottom(),
        tag.left_top(),
    ];
    ui.painter()
        .extend(egui::Shape::dashed_line(&corners, line, 2.0, 2.0));
}

/// A planned feature in a menu: greyed, with "Planned" after it.
pub fn planned_menu_item(ui: &mut egui::Ui, p: &Planned) {
    let b = egui::Button::new(p.label).shortcut_text("Planned");
    let r = ui.add_enabled(false, b);
    let name = p.name();
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, false, &name));
    r.on_disabled_hover_text(format!("{} Planned.", p.what));
}

/// Every planned feature shown: in the bands, the Font group and the File menu.
#[cfg(test)]
pub fn all_planned() -> Vec<&'static Planned> {
    let mut out: Vec<&Planned> = vec![&UNDERLINE];
    for band in Band::ALL {
        for g in band.groups() {
            for i in g.items {
                if let Item::Planned(p) = i {
                    out.push(p);
                }
            }
        }
    }
    out.extend(FILE_PLANNED);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::PageMove;

    /// A band of 5 groups where each folded one saves 40 points: 400 with none folded.
    fn need(folded: usize) -> f32 {
        400.0 - 40.0 * folded as f32
    }

    #[test]
    fn groups_fold_from_the_right_as_room_shrinks_and_return_as_it_grows() {
        assert_eq!(fold_level(0, 5, 400.0, need), 0);
        assert_eq!(fold_level(0, 5, 399.0, need), 1);
        assert_eq!(fold_level(0, 5, 300.0, need), 3);
        assert_eq!(fold_level(0, 5, 10.0, need), 5);
        assert_eq!(fold_level(5, 5, 1000.0, need), 0);
    }

    /// Edit's groups of planned features alone fold before its built commands.
    #[test]
    fn planned_groups_fold_first() {
        let groups: Vec<&Group> = Band::Edit.groups().iter().collect();
        let order = fold_order(&groups);
        let names: Vec<&str> = order.iter().map(|&i| groups[i].name).collect();
        let want = ["Arrange", "Text flow", "Editing", "Font", "Clipboard"];
        assert_eq!(names, want);
    }

    #[test]
    fn a_width_at_the_boundary_does_not_fold_and_unfold_by_turns() {
        // Folded one at 399; at 400 to 405 it stays folded, coming back with the margin.
        let mut n = fold_level(0, 5, 399.0, need);
        for room in [400.0, 405.0, 399.5, 404.0] {
            n = fold_level(n, 5, room, need);
            assert_eq!(n, 1, "at {room}");
        }
        assert_eq!(fold_level(n, 5, 400.0 + UNFOLD_MARGIN, need), 0);
    }

    /// Every command a band's button or menu runs is in the registry, once per band.
    #[test]
    fn bands_run_registered_commands() {
        for band in Band::ALL {
            let mut seen: Vec<(Act, &str)> = Vec::new();
            for g in band.groups() {
                assert!(!g.items.is_empty(), "{} is empty", g.name);
                for i in g.items {
                    let Item::Button(b) = i else { continue };
                    for &a in b.act.iter().chain(b.menu) {
                        let _ = actions::get(a);
                    }
                    if let Some(a) = b.act {
                        assert!(
                            !seen.iter().any(|(s, l)| *s == a && *l != b.label),
                            "{a:?} twice in {}",
                            band.name()
                        );
                        seen.push((a, b.label));
                    }
                }
            }
        }
    }

    /// ENH-41: each planned feature is named for what it will do and the plan's item that
    /// builds it; no two say the same.
    #[test]
    fn planned_features_name_their_plan_items() {
        let all = all_planned();
        assert!(all.len() >= 10);
        for p in &all {
            let id = p.item;
            let known = ["NEW-", "ENH-", "PF-"].iter().any(|k| id.starts_with(k));
            assert!(known && id.len() > 4, "{}: {id}", p.label);
            assert!(p.what.ends_with('.') && !p.what.is_empty(), "{}", p.label);
        }
        let mut labels: Vec<&str> = all.iter().map(|p| p.label).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), all.len());
    }

    /// D6 (build plan 6.4, ENH-37): the band and menu openings each common action takes
    /// from the normal workspace, Edit's band showing: none in Edit's band, one to choose
    /// another band, one more in a button's menu; at most two. (Print, Open, Save, Undo and
    /// Redo are on the commands row, no opening; `every_control_has_a_name_at_every_width`
    /// finds them there at every width. The Comments view is also the strip's.)
    #[test]
    fn common_actions_are_two_openings_away_at_most() {
        let openings = |act: Act| -> Option<usize> {
            let mut fewest = None::<usize>;
            for band in Band::ALL {
                let choose = usize::from(band != Band::Edit);
                for i in band.groups().iter().flat_map(|g| g.items) {
                    let n = match i {
                        Item::Button(b) if b.act == Some(act) => choose,
                        Item::Button(b) if b.menu.contains(&act) => choose + 1,
                        Item::Sign if act == Act::Tool(Tool::Sign) => choose,
                        _ => continue,
                    };
                    fewest = Some(fewest.map_or(n, |f| f.min(n)));
                }
            }
            fewest
        };
        for (act, want) in [
            (Act::EditObjectText, 0),
            (Act::Tool(Tool::Text), 1),
            (Act::Tool(Tool::Image), 1),
            (Act::Tool(Tool::Note), 1),
            (Act::Comments, 1),
            (Act::RotR, 1),
            (Act::RotL, 2),
            (Act::Extract, 1),
            (Act::Delete, 1),
            (Act::InsertImages, 2),
            (Act::InsertPdf, 2),
            (Act::Organise, 1),
            (Act::MovePages(PageMove::Earlier), 2),
            (Act::MovePages(PageMove::End), 2),
            (Act::Tool(Tool::Sign), 1),
            (Act::SignDigital, 2),
            (Act::Find, 0),
        ] {
            assert_eq!(openings(act), Some(want), "{act:?}");
        }
        // The font, size, Bold, Italic and colour: Edit's Font group, showing.
        let font = EDIT
            .iter()
            .flat_map(|g| g.items)
            .any(|i| matches!(i, Item::Font));
        assert!(font, "no Font group in Edit");
    }

    /// The everyday commands are a band's button away (build plan 2.2: at most two
    /// openings, the band's name and a menu).
    #[test]
    fn everyday_commands_are_in_the_bands() {
        let mut reach: Vec<Act> = Vec::new();
        for band in Band::ALL {
            for g in band.groups() {
                for i in g.items {
                    match i {
                        Item::Button(b) => reach.extend(b.act.iter().chain(b.menu)),
                        Item::Sign => reach.push(Act::Tool(Tool::Sign)),
                        Item::Marks => reach.push(Act::Mark(Mark::Highlight)),
                        Item::Font | Item::Planned(_) => {}
                    }
                }
            }
        }
        // The rest: File's commands in the File menu, Undo and Redo beside it; the Font
        // group's Bold and Italic; the marks and Create signature in Highlight's and Fill
        // and sign's own menus; what acts on a selection in its right-click menu and
        // floating bar; and keys. Each is in the File menu's groups too.
        let elsewhere = |a: Act| {
            actions::get(a).group == actions::Group::File
                || matches!(
                    a,
                    Act::Undo
                        | Act::Redo
                        | Act::Bold
                        | Act::Italic
                        | Act::Mark(_)
                        | Act::CreateSign(_)
                        | Act::FinishSigning
                        | Act::PrintSelection
                        | Act::SaveSelection
                        | Act::CopyObjectText
                        | Act::DeleteSelection
                        | Act::DeleteObject
                        | Act::MoveText
                        | Act::ReplaceImage
                        | Act::Escape
                        | Act::Nudge(..)
                        | Act::Extend(_)
                        | Act::MovePages(PageMove::Up | PageMove::Down)
                        | Act::NextPage
                        | Act::PrevPage
                        | Act::FirstPage
                        | Act::LastPage
                        | Act::Palette
                        | Act::NextRegion
                        | Act::PrevRegion
                        | Act::Props
                        | Act::About
                )
        };
        for a in actions::ACTIONS {
            let shown = reach.contains(&a.act) || elsewhere(a.act);
            assert!(shown, "{:?} has no place", a.act);
        }
    }
}
