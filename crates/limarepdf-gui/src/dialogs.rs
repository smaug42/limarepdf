//! The dialogs that remain: printing, the password prompt, messages and confirmations,
//! and the look every dialog shares ([`Shell`], [`content`], [`footer`]). Document and
//! page settings live in the inspector (see `inspector.rs`).

use crate::theme;
use eframe::egui::{self, Align, Context, Key, Margin, RichText, Ui};
use limarepdf_core::print::{Printer, Sides};
use std::hash::Hash;
use std::path::PathBuf;

#[derive(PartialEq, Eq)]
pub enum Outcome {
    Open,
    Cancel,
    Apply,
}

// ------------------------------------------------------------------ the shared look

/// Whether a dialog's completion action comes before Cancel, as on Windows and KDE (and
/// on Linux generally), rather than last, as on macOS.
pub const ACTION_FIRST: bool = !cfg!(target_os = "macos");

/// How a dialog's button looks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    /// The completion action, in the accent colour.
    Primary,
    /// A completion action that loses work or breaks signatures, in red.
    Danger,
    /// Cancel and other choices.
    Plain,
}

/// A button in a dialog's footer.
pub struct Btn<'a> {
    pub text: &'a str,
    pub look: Look,
    pub enabled: bool,
    /// Shown on hover.
    pub tip: Option<&'a str>,
}

impl<'a> Btn<'a> {
    pub fn primary(text: &'a str) -> Self {
        Btn {
            text,
            look: Look::Primary,
            enabled: true,
            tip: None,
        }
    }
    pub fn danger(text: &'a str) -> Self {
        Btn {
            look: Look::Danger,
            ..Btn::primary(text)
        }
    }
    pub fn plain(text: &'a str) -> Self {
        Btn {
            look: Look::Plain,
            ..Btn::primary(text)
        }
    }
    pub fn cancel() -> Self {
        Btn::plain("Cancel")
    }
    pub fn enabled(self, enabled: bool) -> Self {
        Btn { enabled, ..self }
    }
    pub fn tip(self, tip: &'a str) -> Self {
        Btn {
            tip: Some(tip),
            ..self
        }
    }
    fn widget(&self) -> egui::Button<'static> {
        let b = match self.look {
            Look::Primary => theme::primary_button(self.text),
            Look::Danger => theme::danger_button(self.text),
            Look::Plain => egui::Button::new(self.text.to_owned()),
        };
        b.min_size(egui::vec2(72.0, 28.0))
    }
}

/// The indexes of `buttons`, given completion action first and Cancel last, in the order
/// they show from left to right on this platform.
pub fn ordered(buttons: &[Btn]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..buttons.len()).collect();
    if !ACTION_FIRST {
        order.reverse();
    }
    order
}

/// A dialog: a window over a dimmed app, which takes no input while it is open, with the
/// task as its title and, when that needs saying, the file it acts on. What goes in it is
/// drawn with [`content`] and [`footer`].
pub struct Shell<'a> {
    id: egui::Id,
    title: &'a str,
    file: Option<&'a str>,
    width: f32,
    esc: bool,
}

impl<'a> Shell<'a> {
    pub fn new(title: &'a str) -> Self {
        Shell {
            id: egui::Id::new(("dialog", title)),
            title,
            file: None,
            width: 420.0,
            esc: true,
        }
    }
    /// Tells apart dialogs that may share a title.
    pub fn id(self, salt: impl Hash + std::fmt::Debug) -> Self {
        Shell {
            id: egui::Id::new(("dialog", salt)),
            ..self
        }
    }
    /// The file the dialog acts on, shown under the title.
    pub fn file(self, file: Option<&'a str>) -> Self {
        Shell { file, ..self }
    }
    pub fn width(self, width: f32) -> Self {
        Shell { width, ..self }
    }
    /// Whether Esc closes the dialog, as Cancel does; not while it holds input that would be
    /// lost.
    pub fn esc(self, esc: bool) -> Self {
        Shell { esc, ..self }
    }

    /// Shows the dialog. Returns what `body` returned, and whether Esc closed it.
    pub fn show<R>(self, ctx: &Context, body: impl FnOnce(&mut Ui) -> R) -> (R, bool) {
        let room = ctx.content_rect().width() - 32.0;
        let width = self.width.min(room).max(240.0);
        let frame = egui::Frame::window(&ctx.global_style()).inner_margin(0);
        let r = egui::Modal::new(self.id).frame(frame).show(ctx, |ui| {
            ui.set_width(width);
            egui::Frame::NONE
                .inner_margin(Margin {
                    left: 20,
                    right: 20,
                    top: 16,
                    bottom: 0,
                })
                .show(ui, |ui| {
                    let title = RichText::new(self.title)
                        .size(18.0)
                        .family(theme::semibold())
                        .strong();
                    ui.add(egui::Label::new(title).wrap());
                    if let Some(f) = self.file {
                        ui.add(egui::Label::new(RichText::new(f).weak()).wrap());
                    }
                });
            body(ui)
        });
        let esc = self.esc
            && r.is_top_modal
            && !r.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        (r.inner, esc)
    }
}

/// Whether a dialog is open: the app behind it takes no keys either.
pub fn any_open(ctx: &Context) -> bool {
    !ctx.memory(|m| m.is_above_modal_layer(egui::LayerId::background()))
}

/// A dialog's content, padded, and scrolling when the window is too short for it.
pub fn content<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
    let max = (ui.ctx().content_rect().height() - 200.0).max(120.0);
    egui::Frame::NONE
        .inner_margin(Margin::symmetric(20, 14))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(max)
                .auto_shrink([false, true])
                .show(ui, body)
                .inner
        })
        .inner
}

/// A titled group of settings in a dialog's content.
pub fn group<R>(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add_space(4.0);
    ui.label(RichText::new(title).family(theme::semibold()).strong());
    ui.add_space(2.0);
    let r = body(ui);
    ui.add_space(10.0);
    r
}

/// Names a field for screen readers, as the label beside it does on screen.
pub fn set_name(r: &egui::Response, name: &str) {
    r.ctx.accesskit_node_builder(r.id, |n| n.set_label(name));
}

/// A slider with the number it is set to beside it, to type in, both named `name` for
/// screen readers. egui's own number box beside a slider can't be named, so the slider is
/// drawn without it and this one added.
pub fn slider<N: egui::emath::Numeric>(
    ui: &mut Ui,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
    suffix: &str,
    name: &str,
) -> egui::Response {
    let span = range.end().to_f64() - range.start().to_f64();
    let r = ui.add(egui::Slider::new(value, range.clone()).show_value(false));
    set_name(&r, name);
    let number = egui::DragValue::new(value)
        .range(range)
        .suffix(suffix)
        .speed(span / 200.0);
    let n = ui.add(number);
    set_name(&n, name);
    r | n
}

/// Names each open menu or list after the control that opened it, once everything is
/// drawn. egui makes a popup's background a control Tab can reach, with no name: its id
/// is the opener's with "popup" (menus, lists and right-click menus alike), the
/// background's that with "move". The naming tests fail if egui changes that.
pub fn name_popups(ctx: &egui::Context) {
    let open: Vec<(egui::Id, egui::Id)> = ctx.viewport(|v| {
        let pass = &v.this_pass;
        let mut out = Vec::new();
        for (layer, state) in &pass.layers {
            for &popup in &state.open_popups {
                let back = popup.with("move");
                let opener = pass
                    .widgets
                    .get_layer(*layer)
                    .find(|w| w.id.with("popup") == popup);
                if let (true, Some(w)) = (pass.widgets.contains(back), opener) {
                    out.push((back, w.id));
                }
            }
        }
        out
    });
    for (back, opener) in open {
        let name = ctx.accesskit_node_builder(opener, |n| n.label().map(str::to_string));
        if let Some(name) = name.flatten() {
            ctx.accesskit_node_builder(back, |n| {
                n.set_role(egui::accesskit::Role::Menu);
                n.set_label(name);
            });
        }
    }
}

/// The controls in `tree` that Tab can reach whose names don't say what they are: no
/// name, a name without a letter (a glyph such as ◀), or one holding an icon (the icon
/// font's private-use characters). Each is given as its role and name.
#[cfg(test)]
pub fn unclear_names(tree: &egui::accesskit::TreeUpdate) -> Vec<String> {
    tree.nodes
        .iter()
        .filter(|(_, n)| n.supports_action(egui::accesskit::Action::Focus))
        .filter_map(|(_, n)| {
            let name = n.label().unwrap_or_default();
            let icon = name.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c));
            let clear = name.chars().any(char::is_alphabetic) && !icon;
            (!clear).then(|| format!("{:?} {name:?}", n.role()))
        })
        .collect()
}

/// Controls at the right end of a row (whose `ui` is laid out right to left) laid out
/// left to right, so Tab reaches them in reading order: in a box as wide as they were
/// when last drawn, `last`. Returns their width; when that changed, the frame is drawn
/// again, so a misplaced box is never shown.
pub fn reading_order(ui: &mut Ui, last: Option<f32>, add: impl FnOnce(&mut Ui)) -> f32 {
    let size = egui::vec2(last.unwrap_or(0.0), ui.available_height());
    let layout = egui::Layout::left_to_right(Align::Center);
    let width = ui
        .allocate_ui_with_layout(size, layout, add)
        .response
        .rect
        .width();
    if last != Some(width) {
        ui.ctx().request_discard("controls measured");
    }
    width
}

/// A dialog's footer: `note` on the left (what is happening, or what is wrong), the buttons
/// on the right in the platform's order ([`ordered`]). Returns the index in `buttons` of
/// the one clicked.
pub fn footer(ui: &mut Ui, note: Option<RichText>, buttons: &[Btn]) -> Option<usize> {
    ui.add(egui::Separator::default().spacing(0.0));
    let mut hit = None;
    egui::Frame::NONE
        .inner_margin(Margin::symmetric(20, 12))
        .show(ui, |ui| {
            // A row only as tall as its controls: given the rest of the window, it would be
            // centred in it, and keep the window as tall as it ever was when the screen gets
            // shorter.
            let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
            let layout = egui::Layout::right_to_left(Align::Center);
            ui.allocate_ui_with_layout(row, layout, |ui| {
                let id = ui.id().with("footer buttons");
                let last = ui.data(|d| d.get_temp(id));
                let width = reading_order(ui, last, |ui| {
                    for i in ordered(buttons) {
                        let b = &buttons[i];
                        let mut r = ui.add_enabled(b.enabled, b.widget());
                        if let Some(tip) = b.tip {
                            r = r.on_hover_text(tip);
                        }
                        if r.clicked() {
                            hit = Some(i);
                        }
                    }
                });
                ui.data_mut(|d| d.insert_temp(id, width));
                if let Some(note) = note {
                    ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(note).wrap());
                    });
                }
            });
        });
    hit
}

/// A dialog with `action`, and Cancel unless `action` is the only answer (OK for a
/// message); Esc cancels. `body` returns true to apply, as Enter in its field does.
fn modal(
    ctx: &Context,
    shell: Shell,
    action: Btn,
    cancel: bool,
    body: impl FnOnce(&mut Ui) -> bool,
) -> Outcome {
    let (out, esc) = shell.show(ctx, |ui| {
        let enter = content(ui, body);
        let mut buttons = vec![action];
        if cancel {
            buttons.push(Btn::cancel());
        }
        match footer(ui, None, &buttons) {
            Some(0) => Outcome::Apply,
            Some(_) => Outcome::Cancel,
            None if enter => Outcome::Apply,
            None => Outcome::Open,
        }
    });
    if esc {
        return Outcome::Cancel;
    }
    out
}

/// Whether Enter was pressed in the field `r`, a dialog's only field: it takes the keyboard
/// when nothing has it, as the dialog opens.
fn entered(ui: &Ui, r: &egui::Response) -> bool {
    if ui.memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
    r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
}

/// A box for a page range, such as `1-3,5,8-` or `all`, `width` wide if given. Going into
/// it while it says `all` selects that, so typing page numbers replaces it.
pub fn pages_box(ui: &mut Ui, text: &mut String, width: Option<f32>) -> egui::Response {
    let all = text.trim() == "all";
    let mut edit = egui::TextEdit::singleline(text);
    if let Some(w) = width {
        edit = edit.desired_width(w);
    }
    let mut out = edit.show(ui);
    if all && out.response.response.gained_focus() {
        let range = egui::text::CCursorRange::select_all(&out.galley);
        out.state.cursor.set_char_range(Some(range));
        out.state.store(ui.ctx(), out.response.response.id);
    }
    out.response
        .response
        .on_hover_text("e.g. 1-3,5,8-  or  all")
}

// ------------------------------------------------------------------ print

/// How selected text is printed or saved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    /// The pages cropped to the selection, as they look.
    Original,
    /// The words alone, laid out afresh in Helvetica.
    Plain,
    /// A .txt file (saving only).
    TextFile,
}
impl Layout {
    fn label(self) -> &'static str {
        match self {
            Layout::Original => "As it looks (cropped pages)",
            Layout::Plain => "Plain text (PDF)",
            Layout::TextFile => "Text file (.txt)",
        }
    }
    fn tip(self) -> &'static str {
        match self {
            Layout::Original => {
                "Each page at its full width, cut down from top and bottom to the selected \
                 lines. Fonts, pictures and layout stay; anything else beside those lines \
                 shows too."
            }
            Layout::Plain => {
                "Just the selected words, set in Helvetica. Fonts and pictures are lost, and \
                 characters outside Western European alphabets come out as ?."
            }
            Layout::TextFile => "Just the selected words, as plain text.",
        }
    }
}

fn layout_choice(ui: &mut Ui, layout: &mut Layout, choices: &[Layout]) {
    group(ui, "Layout", |ui| {
        for &x in choices {
            ui.radio_value(layout, x, x.label()).on_hover_text(x.tip());
        }
    });
}

pub struct Print {
    pub printers: Vec<Printer>,
    pub printer: usize,
    /// The document printed, shown under the title.
    pub file: String,
    /// Print every page, rather than [`Print::pages`].
    pub all: bool,
    pub pages: String,
    /// Why the pages asked for can't be printed, shown under them.
    pub error: Option<String>,
    pub copies: u32,
    pub sides: Sides,
    pub as_image: bool,
    /// Printing the selected text, in this layout, instead of pages.
    pub selection: Option<Layout>,
}
impl Print {
    /// `printers` has the default first.
    pub fn new(printers: Vec<Printer>, file: String) -> Self {
        Self {
            printers,
            printer: 0,
            file,
            all: true,
            pages: String::new(),
            error: None,
            copies: 1,
            sides: Sides::One,
            as_image: false,
            selection: None,
        }
    }
    /// The pages asked for, as a page range.
    pub fn range(&self) -> &str {
        if self.all {
            "all"
        } else {
            &self.pages
        }
    }
}
pub fn print(ctx: &Context, s: &mut Print) -> Outcome {
    let title = match s.selection {
        Some(_) => "Print selected text",
        None => "Print",
    };
    let file = s.file.clone();
    let (out, esc) = Shell::new(title).file(Some(&file)).show(ctx, |ui| {
        if s.printers.is_empty() {
            content(ui, |ui| {
                ui.label("No printers are set up. Add one in the system's printer settings.")
            });
            return match footer(ui, None, &[Btn::primary("OK")]) {
                Some(_) => Outcome::Cancel,
                None => Outcome::Open,
            };
        }
        let enter = content(ui, |ui| print_body(ui, s));
        match footer(ui, None, &[Btn::primary("Print"), Btn::cancel()]) {
            Some(0) => Outcome::Apply,
            Some(_) => Outcome::Cancel,
            None if enter => Outcome::Apply,
            None => Outcome::Open,
        }
    });
    if esc {
        return Outcome::Cancel;
    }
    out
}

/// The Print dialog's settings; true when Enter was pressed in the page range.
fn print_body(ui: &mut Ui, s: &mut Print) -> bool {
    let wide = ui.available_width();
    group(ui, "Printer", |ui| {
        let name = |p: &Printer| {
            if p.default {
                format!("{} (default)", p.name)
            } else {
                p.name.clone()
            }
        };
        let r = egui::ComboBox::from_id_salt("printer")
            .width(wide)
            .selected_text(name(&s.printers[s.printer]))
            .show_ui(ui, |ui| {
                for (i, p) in s.printers.iter().enumerate() {
                    ui.selectable_value(&mut s.printer, i, name(p));
                }
            });
        set_name(&r.response, "Printer");
    });
    let mut enter = false;
    if s.selection.is_none() {
        group(ui, "Pages", |ui| {
            ui.radio_value(&mut s.all, true, "All pages");
            ui.horizontal(|ui| {
                ui.radio_value(&mut s.all, false, "Page range");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut s.pages)
                        .hint_text("e.g. 1-3, 5, 8-")
                        .desired_width(160.0),
                );
                set_name(&r, "Page range");
                if r.changed() {
                    s.all = false;
                    s.error = None;
                }
                enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            });
            if let Some(e) = &s.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
        });
    }
    group(ui, "Copies and sides", |ui| {
        ui.horizontal(|ui| {
            ui.label("Copies");
            let r = ui.add(egui::DragValue::new(&mut s.copies).range(1..=999));
            set_name(&r, "Copies");
        });
        ui.label("Print on");
        for (x, label) in [
            (Sides::One, "One side"),
            (Sides::LongEdge, "Both sides, flip on long edge"),
            (Sides::ShortEdge, "Both sides, flip on short edge"),
        ] {
            ui.radio_value(&mut s.sides, x, label);
        }
    });
    if let Some(layout) = &mut s.selection {
        layout_choice(ui, layout, &[Layout::Original, Layout::Plain]);
    }
    // Windows always prints pages drawn by LimarePDF.
    if !cfg!(windows) {
        egui::CollapsingHeader::new("Advanced")
            .default_open(s.as_image)
            .show(ui, |ui| {
                let tip = "LimarePDF draws each page at 300 dpi and sends pictures of them, so \
                           the printout matches the screen. Text is less sharp and the job is \
                           larger.";
                ui.checkbox(&mut s.as_image, "Print as image")
                    .on_hover_text(tip);
                ui.weak("Use this if normal printing gives a wrong printout.");
            });
    }
    enter
}

/// Save the selected text: choose the layout; the file dialog follows.
pub fn save_selection(ctx: &Context, file: Option<&str>, layout: &mut Layout) -> Outcome {
    let shell = Shell::new("Save selected text").file(file);
    modal(ctx, shell, Btn::primary("Save…"), true, |ui| {
        let all = [Layout::Original, Layout::Plain, Layout::TextFile];
        layout_choice(ui, layout, &all);
        false
    })
}

/// Remove hidden information: each kind, how many the document has, and whether it is
/// chosen.
pub struct RemoveHidden {
    pub kinds: Vec<(limarepdf_core::Hidden, usize, bool)>,
}

pub fn remove_hidden(ctx: &Context, file: Option<&str>, s: &mut RemoveHidden) -> Outcome {
    let any = s.kinds.iter().any(|(_, n, on)| *on && *n > 0);
    let remove = Btn::danger("Remove").enabled(any);
    let shell = Shell::new("Remove hidden information").file(file);
    modal(ctx, shell, remove, true, |ui| {
        ui.label(
            "Information that travels with the file but isn't on its pages. What is ticked is \
             removed; once saved, it is gone from the file. Undo brings it back until you close \
             the document.",
        );
        ui.add_space(6.0);
        for (h, n, on) in &mut s.kinds {
            if *h == limarepdf_core::Hidden::Comments {
                ui.add_space(4.0);
                ui.label("Often wanted, so not ticked at first:");
            }
            let found = if *n == 0 {
                "none".to_string()
            } else {
                format!("{n} found")
            };
            ui.add_enabled(
                *n > 0,
                egui::Checkbox::new(on, format!("{} ({found})", h.label())),
            );
        }
        false
    })
}

pub struct Password {
    pub path: PathBuf,
    pub input: String,
    pub error: Option<String>,
}
pub fn password(ctx: &Context, s: &mut Password) -> Outcome {
    let name = s.path.file_name().map(|n| n.to_string_lossy().into_owned());
    let shell = Shell::new("Password required").file(name.as_deref());
    modal(ctx, shell, Btn::primary("Open"), true, |ui| {
        ui.label("This document is protected. Type its password to open it.");
        let r = ui.add(
            egui::TextEdit::singleline(&mut s.input)
                .password(true)
                .hint_text("Password"),
        );
        if let Some(e) = &s.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        entered(ui, &r)
    })
}

/// The name written into comments and markup; empty for none.
pub fn comment_name(ctx: &Context, name: &mut String) -> Outcome {
    let shell = Shell::new("Your name for comments");
    modal(ctx, shell, Btn::primary("Save"), true, |ui| {
        ui.label(
            "Comments and markup you add are saved with this name, so others see who made \
             them. Leave it empty to save no name.",
        );
        let r = ui.add(egui::TextEdit::singleline(name).hint_text("No name"));
        entered(ui, &r)
    })
}

/// A message, with OK, and Copy for passing on what it says (an error's details, say).
pub fn message(ctx: &Context, title: &str, text: &str) -> Outcome {
    let (out, esc) = Shell::new(title).show(ctx, |ui| {
        content(ui, |ui| ui.add(egui::Label::new(text).wrap()));
        match footer(ui, None, &[Btn::primary("OK"), Btn::plain("Copy")]) {
            Some(0) => Outcome::Cancel,
            Some(_) => {
                ui.ctx().copy_text(format!("{title}\n\n{text}"));
                Outcome::Open
            }
            None => Outcome::Open,
        }
    });
    if esc {
        return Outcome::Cancel;
    }
    out
}

/// A question: the task as its title, the document it is about, what happens, and the
/// button that does it, beside Cancel.
pub struct Ask<'a> {
    title: &'a str,
    file: Option<&'a str>,
    text: &'a str,
    details: Option<&'a str>,
    yes: Btn<'a>,
}

impl<'a> Ask<'a> {
    pub fn new(title: &'a str, text: &'a str, yes: Btn<'a>) -> Self {
        Ask {
            title,
            file: None,
            text,
            details: None,
            yes,
        }
    }
    /// The document the question is about.
    pub fn file(self, file: Option<&'a str>) -> Self {
        Ask { file, ..self }
    }
    /// The why and what of it, folded away under Details.
    pub fn details(self, details: &'a str) -> Self {
        Ask {
            details: Some(details),
            ..self
        }
    }
}

pub fn confirm(ctx: &Context, a: Ask) -> Outcome {
    let shell = Shell::new(a.title).file(a.file);
    modal(ctx, shell, a.yes, true, |ui| {
        ui.add(egui::Label::new(a.text).wrap());
        if let Some(d) = a.details {
            ui.add_space(4.0);
            egui::CollapsingHeader::new("Details").show(ui, |ui| {
                ui.add(egui::Label::new(d).wrap());
            });
        }
        false
    })
}

// ------------------------------------------------------------------ recovery

/// A document offered in the restore window (the recovery design's section 7).
pub struct Offer {
    /// Its title, or `None` for a copy that can't be restored.
    pub title: Option<String>,
    /// Where from, when, how many pages, and until when it is kept; or why it can't be
    /// restored.
    pub detail: String,
}

/// What was pressed in the restore window: a button for the offer at an index, or Not now.
#[derive(PartialEq, Eq, Debug)]
pub enum RestoreOut {
    Open,
    Restore(usize),
    Discard(usize),
    ShowFile(usize),
    NotNow,
}

/// At start, the documents LimarePDF kept recovery copies of when it last stopped without
/// closing normally, each with Restore and Discard (Show file and Discard for a copy that
/// can't be restored); Not now keeps them all for the next start.
pub fn restore(ctx: &Context, offers: &[Offer]) -> RestoreOut {
    let mut out = RestoreOut::Open;
    let shell = Shell::new("Recover unsaved changes").width(520.0);
    let (later, esc) = shell.show(ctx, |ui| {
        content(ui, |ui| {
            ui.add(
                egui::Label::new(
                    "LimarePDF stopped without closing normally. It kept copies of these \
                     documents with their unsaved changes, as they were at the time shown. A \
                     restored document opens as a new, unsaved document; the original file \
                     isn't changed.",
                )
                .wrap(),
            );
            for (i, o) in offers.iter().enumerate() {
                ui.add_space(6.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match &o.title {
                        Some(t) => ui.strong(t),
                        None => ui.strong("A copy that can't be restored"),
                    };
                    ui.add(egui::Label::new(RichText::new(&o.detail).weak()).wrap());
                    ui.horizontal(|ui| {
                        if o.title.is_some() {
                            if ui.add(theme::primary_button("Restore")).clicked() {
                                out = RestoreOut::Restore(i);
                            }
                        } else if ui.button("Show file").clicked() {
                            out = RestoreOut::ShowFile(i);
                        }
                        if ui.button("Discard").clicked() {
                            out = RestoreOut::Discard(i);
                        }
                    });
                });
            }
        });
        let note = RichText::new("Not now keeps the copies until the date shown.").weak();
        footer(ui, Some(note), &[Btn::plain("Not now")]).is_some()
    });
    // Esc defers too: nothing is lost.
    if later || esc {
        return RestoreOut::NotNow;
    }
    out
}

/// What was picked in [`choose`].
#[derive(PartialEq, Eq, Debug)]
pub enum Choice {
    Open,
    Cancel,
    First,
    Second,
}

/// A question with two answers, `first` the safer one, and Cancel when `cancel`; Esc
/// cancels.
pub fn choose(
    ctx: &Context,
    shell: Shell,
    text: &str,
    first: Btn,
    second: Btn,
    cancel: bool,
) -> Choice {
    let (out, esc) = shell.show(ctx, |ui| {
        content(ui, |ui| ui.add(egui::Label::new(text).wrap()));
        let mut buttons = vec![first, second];
        if cancel {
            buttons.push(Btn::cancel());
        }
        match footer(ui, None, &buttons) {
            Some(0) => Choice::First,
            Some(1) => Choice::Second,
            Some(_) => Choice::Cancel,
            None => Choice::Open,
        }
    });
    if esc {
        return Choice::Cancel;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shows a pages box holding `text` for one frame, given `events`; returns where it is.
    fn frame(ctx: &Context, text: &mut String, events: Vec<egui::Event>) -> egui::Rect {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 200.0),
            )),
            events,
            ..Default::default()
        };
        let mut rect = egui::Rect::NOTHING;
        let mut out = ctx.run_ui(raw, |ui| rect = pages_box(ui, text, None).rect);
        out.textures_delta.clear();
        rect
    }

    /// Clicks in the middle of a pages box holding `text`, then types `typed`.
    fn click_and_type(text: &str, typed: &str) -> String {
        let ctx = Context::default();
        let mut text = text.to_string();
        let at = frame(&ctx, &mut text, vec![]).center();
        let click = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &ctx,
            &mut text,
            vec![egui::Event::PointerMoved(at), click(true)],
        );
        frame(&ctx, &mut text, vec![click(false)]);
        frame(&ctx, &mut text, vec![]);
        frame(&ctx, &mut text, vec![egui::Event::Text(typed.into())]);
        text
    }

    /// The completion action comes first, Cancel last, except on macOS, where it is the
    /// other way round.
    #[test]
    fn buttons_follow_the_platform_order() {
        let buttons = [
            Btn::danger("Replace"),
            Btn::plain("Save as…"),
            Btn::cancel(),
        ];
        let order = ordered(&buttons);
        if cfg!(target_os = "macos") {
            assert_eq!(order, [2, 1, 0]);
        } else {
            assert_eq!(order, [0, 1, 2]);
        }
    }

    #[test]
    fn typing_into_a_pages_box_saying_all_replaces_it() {
        assert_eq!(click_and_type("all", "2-4"), "2-4");
    }

    #[test]
    fn other_page_ranges_are_edited_where_clicked() {
        // The click lands past the end of the text, so typing adds to it.
        assert_eq!(click_and_type("1-3", ",5"), "1-3,5");
    }

    /// G02: a dialog drawn on a tall screen fits again when the screen gets shorter (its
    /// footer once kept it as tall as it had been), with its buttons on screen and a footer
    /// note that wraps kept beside them, inside the window.
    #[test]
    fn a_dialog_fits_again_when_the_screen_gets_shorter() {
        let ctx = Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let offers: Vec<Offer> = (1..=4)
            .map(|i| Offer {
                title: Some(format!("Report {i}.pdf")),
                detail: "From /home/sam/Documents, 3 pages, kept until 1 December".into(),
            })
            .collect();
        let id = egui::Id::new(("dialog", "Recover unsaved changes"));
        for size in [
            (400.0, 800.0),
            (400.0, 300.0),
            (1000.0, 800.0),
            (700.0, 300.0),
        ] {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size.into());
            let mut tree = None;
            for _ in 0..5 {
                let raw = egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(raw, |_| {
                    restore(&ctx, &offers);
                });
                out.textures_delta.clear();
                tree = out.platform_output.accesskit_update;
            }
            let tree = tree.expect("the dialog's widgets");
            let at = |label: &str| {
                let node = tree
                    .nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some(label) || n.value() == Some(label));
                let b = node.and_then(|(_, n)| n.bounds()).expect(label);
                egui::Rect::from_min_max(
                    egui::pos2(b.x0 as f32, b.y0 as f32),
                    egui::pos2(b.x1 as f32, b.y1 as f32),
                )
            };
            let window = ctx.memory(|m| m.area_rect(id)).expect("the window");
            let not_now = at("Not now");
            let note = at("Not now keeps the copies until the date shown.");
            let screen = screen.expand(0.5);
            assert!(screen.contains_rect(window), "{size:?}: window {window:?}");
            assert!(window.contains_rect(not_now), "{size:?}: {not_now:?}");
            assert!(window.contains_rect(note), "{size:?}: note {note:?}");
            let beside = note.y_range().intersects(not_now.y_range());
            assert!(beside, "{size:?}: {note:?}, {not_now:?}");
        }
    }
}
