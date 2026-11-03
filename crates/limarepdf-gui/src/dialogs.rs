//! The dialogs that remain: printing, the password prompt, messages and confirmations,
//! and the look every dialog shares ([`Shell`], [`content`], [`footer`]). Document and
//! page settings live in the inspector (see `inspector.rs`).

use crate::print_preview::{self, Preview, What};
use crate::spin::Spin;
use crate::theme;
use eframe::egui::{self, Align, Context, Key, Margin, RichText, Ui};
use limarepdf_core::print::{
    Binding, Booklet, BookletSides, Orientation, PageOrder, PageSetup, PaperList, PerSheet,
    Printer, Scale, Sides, PAGES_PER_SHEET,
};
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
        b.min_size(egui::vec2(80.0, theme::DIALOG_CONTROL))
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

/// The space at each side of a dialog's content, in a window wide enough: a dialog sized
/// to fit its content is that much wider on each side.
pub const SIDE: f32 = 24.0;

/// Below this window width, dialogs have less space around their parts.
const NARROW: f32 = 600.0;

/// The space around a dialog's parts, in points: at its sides, above its title, above and
/// below its content, and above and below its buttons.
struct Insets {
    side: i8,
    top: i8,
    content: i8,
    footer: i8,
}

fn insets(ctx: &Context) -> Insets {
    if ctx.content_rect().width() < NARROW {
        Insets {
            side: 16,
            top: 18,
            content: 20,
            footer: 14,
        }
    } else {
        Insets {
            side: SIDE as i8,
            top: 22,
            content: 22,
            footer: 16,
        }
    }
}

/// The least room kept between a dialog grown downward and the screen's bottom.
const EDGE: f32 = 8.0;

/// A dialog: a window over a dimmed app, which takes no input while it is open, with the
/// task as its title and, when that needs saying, the file it acts on. What goes in it is
/// drawn with [`content`] and [`footer`], in the dialogs' look ([`theme::dialog_style`]).
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
            id: egui::Id::unique(("dialog", title)),
            title,
            file: None,
            width: 470.0,
            esc: true,
        }
    }
    /// Tells apart dialogs that may share a title.
    pub fn id(self, salt: impl Hash + std::fmt::Debug) -> Self {
        Shell {
            id: egui::Id::unique(("dialog", salt)),
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
        let at = insets(ctx);
        // A menu still open as the dialog opens (its shortcut pressed with a menu open)
        // would show over the dimmed window: closed. The dialog's own lists open later.
        let layer = egui::LayerId::new(egui::Order::Foreground, self.id);
        let shown = ctx.memory(|m| m.areas().visible_last_frame(&layer));
        if !shown {
            egui::Popup::close_all(ctx);
        }
        // It opens centred, then keeps its top, so what is added to it (a field shown, a
        // section opened) extends it downward, as desktop dialogs do (BUG-50); egui centres
        // it again each frame. On a screen of another size it is centred again.
        let screen = ctx.content_rect();
        let kept_id = self.id.with("top");
        let kept: Option<(f32, egui::Rect)> = ctx.data(|d| d.get_temp(kept_id));
        let kept = kept.filter(|(_, on)| shown && *on == screen);
        let mut modal = egui::Modal::new(self.id).frame(frame);
        if let Some((top, _)) = kept {
            let at = egui::vec2(0.0, top - screen.top());
            let area = egui::Modal::default_area(self.id).anchor(egui::Align2::CENTER_TOP, at);
            modal = modal.area(area);
        }
        let r = modal.show(ctx, |ui| {
            ui.set_width(width);
            theme::dialog_style(ui.style_mut());
            egui::Frame::NONE
                .inner_margin(Margin {
                    left: at.side,
                    right: at.side,
                    top: at.top,
                    bottom: 0,
                })
                .show(ui, |ui| {
                    let title = RichText::new(self.title)
                        .size(20.0)
                        .family(theme::medium())
                        .strong();
                    ui.add(egui::Label::new(title).wrap());
                    if let Some(f) = self.file {
                        ui.add(egui::Label::new(RichText::new(f).weak()).wrap());
                    }
                });
            body(ui)
        });
        // The top kept from the first frame it was seen at, moved up only as far as keeps its
        // bottom on the screen.
        match ctx.memory(|m| m.area_rect(self.id)).filter(|_| shown) {
            Some(rect) => {
                let lowest = screen.bottom() - EDGE - rect.height();
                let top = rect.top().min(lowest).max(screen.top());
                ctx.data_mut(|d| d.insert_temp(kept_id, (top, screen)));
            }
            None => ctx.data_mut(|d| d.remove::<(f32, egui::Rect)>(kept_id)),
        }
        // At Tab on the last control egui gives up the keyboard, and finds the first control
        // only at the next Tab: it goes straight round instead (PB-15). Being offered first,
        // the first control takes the keyboard egui means to give on; the only one, which
        // egui doesn't offer it back to, is given it.
        let tab = ctx.input(|i| i.key_pressed(Key::Tab) && i.modifiers.is_none());
        if tab && r.is_top_modal && ctx.memory(|m| m.focused().is_none()) {
            let first = ctx.viewport(|v| {
                let mut widgets = v.this_pass.widgets.get_layer(layer);
                let first = widgets.find(|w| w.enabled && w.sense.is_focusable());
                first.map(|w| w.id)
            });
            if let Some(id) = first {
                ctx.memory_mut(|m| {
                    m.interested_in_focus(id, layer);
                    if m.focused().is_none() {
                        m.request_focus(id);
                    }
                });
            }
        }
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

/// A dialog's content, padded, and scrolling when the window is too short for it: drawn
/// under the dialog's title, with a [`footer`] to come.
pub fn content<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
    let top = ui.max_rect().min.y;
    content_below(ui, top, body)
}

/// [`content`] in a column of a dialog whose window's inside starts at `top`.
fn content_below<R>(ui: &mut Ui, top: f32, body: impl FnOnce(&mut Ui) -> R) -> R {
    let at = insets(ui.ctx());
    let above = ui.cursor().min.y - top;
    let footer = 2.0 * f32::from(at.footer) + theme::DIALOG_CONTROL + 1.0;
    // And some of the screen left above and below the window.
    let around = 2.0 * f32::from(at.content) + 16.0;
    let max = (ui.ctx().content_rect().height() - above - footer - around).max(60.0);
    egui::Frame::NONE
        .inner_margin(Margin::symmetric(at.side, at.content))
        .show(ui, |ui| {
            // Room up to `max` whatever the window's height last frame: given only that,
            // a tall dialog grew by its footer's height each frame until it reached `max`.
            let room = egui::vec2(ui.available_width(), max);
            ui.allocate_ui(room, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(max)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        theme::dialog_choices(ui.style_mut());
                        body(ui)
                    })
                    .inner
            })
            .inner
        })
        .inner
}

/// Space between the groups of a dialog's content.
const GAP: f32 = 20.0;

/// Space between the fields of a group, one under the other.
pub const FIELD_GAP: f32 = 12.0;

/// `space`, unless nothing is above in `ui`.
pub fn space_above(ui: &mut Ui, space: f32) {
    if ui.cursor().min.y > ui.max_rect().min.y + 1.0 {
        ui.add_space(space);
    }
}

/// [`GAP`], unless nothing is above in `ui`.
fn gap(ui: &mut Ui) {
    space_above(ui, GAP);
}

/// A titled group of settings in a dialog's content. `body` is drawn in a column of its
/// own, so [`space_above`] in it adds no space under the title.
pub fn group<R>(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui) -> R) -> R {
    gap(ui);
    ui.label(RichText::new(title).family(theme::medium()).strong());
    ui.add_space(4.0);
    ui.vertical(body).inner
}

/// A field in a dialog's content with its label above it, which also names it for screen
/// readers. `add` adds the field and returns it.
pub fn field(
    ui: &mut Ui,
    label: &str,
    add: impl FnOnce(&mut Ui) -> egui::Response,
) -> egui::Response {
    ui.label(label);
    ui.add_space(2.0);
    let r = add(ui);
    set_name(&r, label);
    r
}

/// Space between columns.
pub const COLUMN_GAP: f32 = 16.0;

/// Two parts of a dialog's content side by side, `a` `left` points wide and `b` the
/// rest, when the rest is at least `need`; otherwise one under the other, each the full
/// width. Each is given `state` and its width.
pub fn columns<T>(
    ui: &mut Ui,
    state: &mut T,
    left: f32,
    need: f32,
    a: impl FnOnce(&mut Ui, &mut T, f32),
    b: impl FnOnce(&mut Ui, &mut T, f32),
) {
    let width = ui.available_width();
    let rest = width - left - COLUMN_GAP;
    if rest < need {
        a(ui, state, width);
        ui.add_space(FIELD_GAP);
        b(ui, state, width);
        return;
    }
    let spacing = ui.spacing().item_spacing;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = COLUMN_GAP;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = spacing;
            ui.set_width(left);
            a(ui, state, left);
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = spacing;
            ui.set_width(rest);
            b(ui, state, rest);
        });
    });
}

/// A list in a dialog's content: an outlined box as wide as the content, holding
/// [`list_row`]s or [`list_pick`]s.
pub fn list_box<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
    let edge = ui.visuals().widgets.inactive.bg_stroke;
    egui::Frame::NONE
        .stroke(edge)
        .corner_radius(theme::RADIUS)
        .inner_margin(4)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        })
        .inner
}

/// A [`list_box`] of `n` rows with a rule between them, `row` drawing each; nothing when
/// there are none. Each row has an id of its own, so what a [`list_row`] remembers is
/// its own.
pub fn list_rows(ui: &mut Ui, n: usize, mut row: impl FnMut(&mut Ui, usize)) {
    if n == 0 {
        return;
    }
    list_box(ui, |ui| {
        for i in 0..n {
            if i > 0 {
                ui.add(egui::Separator::default().spacing(0.0));
            }
            ui.push_id(i, |ui| row(ui, i));
        }
    });
}

/// Below this width, a [`list_row`]'s buttons go under its text.
const ROW_BESIDE: f32 = 360.0;

/// A row of a [`list_rows`] box: `text`, what the row is, on the left, and `buttons` on
/// the right, in reading order; under the text when the box is narrow.
pub fn list_row(ui: &mut Ui, text: impl FnOnce(&mut Ui), buttons: impl FnOnce(&mut Ui)) {
    egui::Frame::NONE
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            if ui.available_width() < ROW_BESIDE {
                ui.vertical(text);
                ui.add_space(6.0);
                ui.horizontal_wrapped(buttons);
                return;
            }
            // The buttons in a box at the right as wide as they were when last drawn; when
            // that changed, the frame is drawn again.
            let id = ui.scope_id().with("row buttons");
            let last: Option<f32> = ui.data(|d| d.get_temp(id));
            ui.horizontal_top(|ui| {
                let room = ui.available_width() - ui.spacing().item_spacing.x;
                ui.vertical(|ui| {
                    ui.set_width((room - last.unwrap_or(0.0)).max(0.0));
                    text(ui);
                });
                let size = egui::vec2(last.unwrap_or(0.0), theme::DIALOG_CONTROL);
                let layout = egui::Layout::left_to_right(Align::Center);
                let width = ui
                    .allocate_ui_with_layout(size, layout, buttons)
                    .response
                    .rect
                    .width();
                if last != Some(width) {
                    ui.data_mut(|d| d.insert_temp(id, width));
                    ui.ctx().request_discard("row buttons measured");
                }
            });
        });
}

/// A row of a [`list_box`] that is picked by clicking it: as wide as the box and as tall
/// as the dialog's other controls.
pub fn list_pick(ui: &mut Ui, on: bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    let b = egui::Button::selectable(on, text)
        .min_size(egui::vec2(ui.available_width(), theme::DIALOG_CONTROL));
    ui.add(b)
}

/// The width of the names of [`fact`]s.
const FACT_NAME: f32 = 104.0;

/// A fact in a dialog's content, to read rather than change: its name, weak, in a column
/// on the left, and its value beside it, wrapping; the name above it when the dialog is
/// narrow.
pub fn fact(ui: &mut Ui, name: &str, value: impl Into<egui::WidgetText>) {
    let name = RichText::new(name).weak();
    if ui.available_width() < 3.0 * FACT_NAME {
        ui.add(egui::Label::new(name).wrap());
        ui.add(egui::Label::new(value).wrap());
        ui.add_space(4.0);
        return;
    }
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(FACT_NAME - ui.spacing().item_spacing.x);
            ui.add(egui::Label::new(name).wrap());
        });
        ui.add(egui::Label::new(value).wrap());
    });
}

/// Names a field for screen readers, as the label beside it does on screen.
pub fn set_name(r: &egui::Response, name: &str) {
    r.ctx.accesskit_node_builder(r.id, |n| n.set_label(name));
}

/// A slider with the number it is set to beside it in a spin box, to type in or step, both
/// named `name` for screen readers. egui's own number box beside a slider can't be named
/// and is dragged, so the slider is drawn without it and the spin box added.
pub fn slider<N: egui::emath::Numeric>(
    ui: &mut Ui,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
    suffix: &str,
    name: &str,
) -> egui::Response {
    let r = ui.add(egui::Slider::new(value, range.clone()).show_value(false));
    set_name(&r, name);
    let n = ui.add(Spin::new(value, range).suffix(suffix).name(name));
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

/// The widgets of what `show` draws on a screen `size` points big, once settled.
#[cfg(test)]
pub fn drawn(size: egui::Vec2, show: impl FnMut(&Context)) -> egui::accesskit::TreeUpdate {
    drawn_window(size, show).0
}

/// The widgets of the dialog `show` draws on a screen `size` points big, once settled,
/// and where its window is (nothing when no dialog is open).
#[cfg(test)]
pub fn drawn_window(
    size: egui::Vec2,
    mut show: impl FnMut(&Context),
) -> (egui::accesskit::TreeUpdate, egui::Rect) {
    let ctx = Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut tree = None;
    for _ in 0..5 {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| show(ui.ctx()));
        out.textures_delta.clear();
        tree = out.platform_output.accesskit_update;
    }
    let window = ctx.memory(|m| m.top_modal_layer().and_then(|l| m.area_rect(l.id)));
    (
        tree.expect("the widgets"),
        window.unwrap_or(egui::Rect::NOTHING),
    )
}

/// Whether the dialog `window` and the controls named `names` in `tree` are inside a
/// screen `size` points big, the controls inside the window too; if not, what is wrong.
#[cfg(test)]
pub fn fits(
    tree: &egui::accesskit::TreeUpdate,
    window: egui::Rect,
    size: egui::Vec2,
    names: &[&str],
) -> Result<(), String> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size).expand(0.5);
    if !screen.contains_rect(window) {
        return Err(format!("{size:?}: window {window:?}"));
    }
    for name in names {
        let (_, n) = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(name))
            .ok_or(format!("{size:?}: no {name}"))?;
        let b = n.bounds().ok_or(format!("{size:?}: {name} not placed"))?;
        let r = egui::Rect::from_min_max(
            egui::pos2(b.x0 as f32, b.y0 as f32),
            egui::pos2(b.x1 as f32, b.y1 as f32),
        );
        if !window.expand(0.5).contains_rect(r) {
            return Err(format!("{size:?}: {name} at {r:?}, window {window:?}"));
        }
    }
    Ok(())
}

/// Whether the text `label` is shown above the control it names, at its left edge, as
/// [`field`] draws them; if not, what is wrong.
#[cfg(test)]
pub fn label_above(tree: &egui::accesskit::TreeUpdate, label: &str) -> Result<(), String> {
    let rect = |n: &egui::accesskit::Node| {
        n.bounds().map(|b| {
            egui::Rect::from_min_max(
                egui::pos2(b.x0 as f32, b.y0 as f32),
                egui::pos2(b.x1 as f32, b.y1 as f32),
            )
        })
    };
    let find = |f: &dyn Fn(&egui::accesskit::Node) -> bool| {
        let (_, n) = tree.nodes.iter().find(|(_, n)| f(n))?;
        rect(n)
    };
    let text = find(&|n| n.value() == Some(label)).ok_or(format!("no text {label:?}"))?;
    let focusable = |n: &egui::accesskit::Node| n.supports_action(egui::accesskit::Action::Focus);
    let control = find(&|n| n.label() == Some(label) && focusable(n))
        .ok_or(format!("no control named {label:?}"))?;
    let above = text.bottom() <= control.top() + 0.5;
    if !above || (text.left() - control.left()).abs() > 1.0 {
        return Err(format!("{label:?}: text {text:?}, control {control:?}"));
    }
    Ok(())
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
    let at = insets(ui.ctx());
    egui::Frame::NONE
        .inner_margin(Margin::symmetric(at.side, at.footer))
        .show(ui, |ui| {
            // A row only as tall as its controls: given the rest of the window, it would be
            // centred in it, and keep the window as tall as it ever was when the screen gets
            // shorter.
            let row = egui::vec2(ui.available_width(), theme::DIALOG_CONTROL);
            let layout = egui::Layout::right_to_left(Align::Center);
            ui.allocate_ui_with_layout(row, layout, |ui| {
                let id = ui.scope_id().with("footer buttons");
                let last = ui.data(|d| d.get_temp(id));
                let width = reading_order(ui, last, |ui| {
                    for i in ordered(buttons) {
                        let b = &buttons[i];
                        let mut r = ui.add_enabled(b.enabled, b.widget());
                        // Shown greyed too: egui's hover text is for enabled widgets only.
                        if let Some(tip) = b.tip {
                            r = r.on_hover_text(tip).on_disabled_hover_text(tip);
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
/// when nothing has it, as the dialog opens. Enter is seen first, as the field gives up the
/// keyboard for it: taking the keyboard back hides that (BUG-08).
fn entered(ui: &Ui, r: &egui::Response) -> bool {
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    if !enter && ui.memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
    enter
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
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
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
                "Just the selected words, set in Helvetica, or another installed font where \
                 it lacks a character. Fonts and pictures are lost, and characters no font \
                 has come out as ?."
            }
            Layout::TextFile => "Just the selected words, as plain text.",
        }
    }
}

fn layout_choice(ui: &mut Ui, layout: &mut Layout, choices: &[Layout]) {
    group(ui, "Layout", |ui| layout_radios(ui, layout, choices));
}

fn layout_radios(ui: &mut Ui, layout: &mut Layout, choices: &[Layout]) {
    for &x in choices {
        ui.radio_value(layout, x, x.label()).on_hover_text(x.tip());
    }
}

/// Looking for the printers, which the Print dialog does once open (BUG-11).
#[derive(Clone, Debug, PartialEq)]
pub enum Search {
    /// Search number `.0` is running.
    Running(u64),
    /// Done: [`Print::printers`] has the printers, perhaps none.
    Done,
    /// The printers couldn't be listed: why.
    Failed(String),
}

/// The paper sizes of the printer chosen, which the Print dialog asks for once it knows the
/// printer, off the window's thread as the printers are (ENH-14).
#[derive(Clone, Debug, PartialEq)]
pub enum Papers {
    /// Not asked for yet.
    Unasked,
    /// Search number `.0` is asking for printer `.1`'s.
    Asking(u64, String),
    /// Printer `.0`'s.
    Found(String, PaperList),
    /// Printer `.0`'s couldn't be found: why.
    Failed(String, String),
}

pub struct Print {
    pub printers: Vec<Printer>,
    pub search: Search,
    /// A search whose fuller list is still to come: shown printers listed at once, or found
    /// earlier in the session (ENH-47).
    pub listing: Option<u64>,
    /// The printers shown were found earlier in the session, not by this dialog's search.
    pub earlier: bool,
    pub printer: usize,
    /// The document printed, shown under the title.
    pub file: String,
    /// How many pages it has, for the preview to read the page range.
    pub page_count: usize,
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
    pub papers: Papers,
    /// The paper chosen, of the printer's.
    pub paper: usize,
    pub orientation: Orientation,
    pub scale: Scale,
    /// The percentage a custom scale is, kept while another scale is chosen.
    pub percent: u16,
    pub per_sheet: PerSheet,
    /// Printing as a booklet (NEW-05), bound on [`Print::binding`] and printing
    /// [`Print::booklet_sides`]; both kept while it isn't.
    pub booklet: bool,
    pub binding: Binding,
    pub booklet_sides: BookletSides,
    pub preview: Preview,
}
impl Print {
    /// `printers` has the default first.
    pub fn new(printers: Vec<Printer>, file: String) -> Self {
        Self {
            printers,
            search: Search::Done,
            listing: None,
            earlier: false,
            printer: 0,
            file,
            page_count: 1,
            all: true,
            pages: String::new(),
            error: None,
            copies: 1,
            sides: Sides::One,
            as_image: false,
            selection: None,
            papers: Papers::Unasked,
            paper: 0,
            orientation: Orientation::Automatic,
            scale: Scale::ShrinkOversized,
            percent: 100,
            per_sheet: PerSheet::default(),
            booklet: false,
            binding: Binding::Left,
            booklet_sides: BookletSides::Both,
            preview: Preview::default(),
        }
    }
    /// Looking for the printers, by search number `search`.
    pub fn looking(search: u64, file: String) -> Self {
        Self {
            search: Search::Running(search),
            ..Self::new(Vec::new(), file)
        }
    }
    /// The printers found, or why they couldn't be listed: shown, the printer chosen kept
    /// when it is still listed, else the one last printed to (`last`, its name) when listed,
    /// else the default. Why a fuller list couldn't be had is left unsaid when this search
    /// has listed printers already (ENH-47), not when those shown were found earlier in the
    /// session: the printing service has stopped answering.
    pub fn found(&mut self, found: Result<Vec<Printer>, String>, last: Option<&str>) {
        let chosen = self.chosen().map(str::to_string);
        match found {
            Ok(list) => {
                let keep = |name: Option<&str>| list.iter().position(|p| Some(&*p.name) == name);
                self.printer = keep(chosen.as_deref()).or(keep(last)).unwrap_or(0);
                self.printers = list;
                self.search = Search::Done;
                self.earlier = false;
            }
            Err(_) if chosen.is_some() && !self.earlier => {}
            Err(why) => (self.printers, self.search) = (Vec::new(), Search::Failed(why)),
        }
    }
    /// Shows the printers found earlier in the session, `known`, while search `search`
    /// looks again: as [`Print::found`] would.
    pub fn found_earlier(&mut self, known: Vec<Printer>, last: Option<&str>, search: u64) {
        self.found(Ok(known), last);
        self.earlier = true;
        self.listing = Some(search);
    }
    /// Whether the printers are found, and there is one.
    pub fn ready(&self) -> bool {
        self.search == Search::Done && !self.printers.is_empty()
    }
    /// The printer chosen, once the printers are found.
    fn chosen(&self) -> Option<&str> {
        let p = self.printers.get(self.printer).filter(|_| self.ready())?;
        Some(&p.name)
    }
    /// The printer whose paper sizes are to be asked for: the one chosen, when they aren't
    /// asked for, found or failed for it.
    pub fn papers_wanted(&self) -> Option<String> {
        let name = self.chosen()?;
        let asked = match &self.papers {
            Papers::Unasked => None,
            Papers::Asking(_, p) | Papers::Found(p, _) | Papers::Failed(p, _) => Some(p),
        };
        (asked.map(String::as_str) != Some(name)).then(|| name.to_string())
    }
    /// The paper sizes asked for by `search`, or why they couldn't be found: shown, with
    /// the paper last used with the printer chosen (by its id, `last`), else its default.
    pub fn papers_found(
        &mut self,
        search: u64,
        found: Result<PaperList, String>,
        last: Option<&str>,
    ) {
        let Papers::Asking(n, printer) = &self.papers else {
            return;
        };
        if *n != search {
            return;
        }
        let printer = printer.clone();
        match found {
            Ok(list) => self.use_papers(printer, list, last),
            Err(why) => self.papers = Papers::Failed(printer, why),
        }
    }
    /// `printer`'s paper sizes `list`, found now or earlier in the session: shown, with the
    /// paper last used with the printer chosen (by its id, `last`), else its default.
    pub fn use_papers(&mut self, printer: String, list: PaperList, last: Option<&str>) {
        let last = last.and_then(|id| list.papers.iter().position(|p| p.id == id));
        self.paper = last.or(list.default).unwrap_or(0);
        self.papers = Papers::Found(printer, list);
    }
    /// Why the printer's paper sizes couldn't be found, if they couldn't.
    pub fn papers_failed(&self) -> Option<&str> {
        match &self.papers {
            Papers::Failed(p, why) if Some(p.as_str()) == self.chosen() => Some(why),
            _ => None,
        }
    }
    /// The paper and how the pages go on it, once the printer's paper sizes are found.
    pub fn setup(&self) -> Option<PageSetup> {
        let Papers::Found(p, list) = &self.papers else {
            return None;
        };
        let paper = list.papers.get(self.paper)?.clone();
        (Some(p.as_str()) == self.chosen()).then_some(PageSetup {
            paper,
            orientation: self.orientation,
            scale: self.scale,
            per_sheet: self.per_sheet,
            booklet: self.booklet_chosen(),
        })
    }
    /// The booklet chosen, if printing as one.
    fn booklet_chosen(&self) -> Option<Booklet> {
        self.booklet.then_some(Booklet {
            binding: self.binding,
            sides: self.booklet_sides,
        })
    }
    /// How the printer prints the sides: a booklet's own, else as Print on says.
    pub fn sides_used(&self) -> Sides {
        self.booklet_chosen().map_or(self.sides, |b| b.sides_used())
    }
    /// For tests: Print with `printers`, the first one's paper sizes found: Letter, its
    /// default, and A4, each with the margins the user's HP reports.
    #[cfg(test)]
    pub fn with_papers(printers: Vec<Printer>, file: String) -> Self {
        use limarepdf_core::print::Paper;
        let paper = |id: &str, label: &str, size| Paper {
            id: id.into(),
            label: label.into(),
            size,
            margins: Some([12.0; 4]),
        };
        let list = PaperList {
            papers: vec![
                paper("na_letter_8.5x11in", "Letter (8.5 × 11 in)", (612.0, 792.0)),
                paper("iso_a4_210x297mm", "A4 (210 × 297 mm)", (595.28, 841.89)),
            ],
            default: Some(0),
        };
        let name = printers.first().map(|p| p.name.clone()).unwrap_or_default();
        Self {
            papers: Papers::Found(name, list),
            ..Self::new(printers, file)
        }
    }
    /// The names of the printer's paper sizes, once found.
    fn paper_labels(&self) -> Option<Vec<String>> {
        self.setup()?;
        match &self.papers {
            Papers::Found(_, list) => Some(list.papers.iter().map(|p| p.label.clone()).collect()),
            _ => None,
        }
    }
    /// What the preview shows: the pages asked for, or those last asked for while the page
    /// range can't be read, as the page setup lays them out.
    fn preview_ask(&self) -> Option<print_preview::Ask> {
        let setup = self.setup()?;
        let what = match self.selection {
            Some(layout) => What::Selection(layout),
            None => match limarepdf_core::parse_page_range(self.range(), self.page_count) {
                Ok(pages) => What::Pages(pages),
                Err(_) => self.preview.ask()?.what.clone(),
            },
        };
        Some(print_preview::Ask { what, setup })
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
/// The width of the Print dialog's settings, and of the preview beside them.
const PRINT_SETTINGS: f32 = 470.0;
const PREVIEW_WIDTH: f32 = 330.0;

/// The height of the preview under the settings, in a window too narrow for it beside them.
const PREVIEW_UNDER: f32 = 300.0;

/// The Print dialog. [`Outcome::Apply`] is Print once [`Print::ready`] and the printer's
/// paper sizes are found ([`Print::setup`]), and Try again when the printers or the paper
/// sizes couldn't be found. In a window wide enough the preview is beside the settings,
/// otherwise under them.
pub fn print(ctx: &Context, s: &mut Print) -> Outcome {
    let title = match s.selection {
        Some(_) => "Print selected text",
        None => "Print",
    };
    let file = s.file.clone();
    let beside = ctx.content_rect().width() - 32.0 >= PRINT_SETTINGS + PREVIEW_WIDTH;
    let width = match beside {
        true => PRINT_SETTINGS + PREVIEW_WIDTH,
        false => PRINT_SETTINGS,
    };
    let shell = Shell::new(title).file(Some(&file)).width(width);
    let (out, esc) = shell.show(ctx, |ui| {
        let why = match &s.search {
            Search::Running(_) => {
                content(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Looking for printers…");
                    })
                });
                let print = Btn::primary("Print").enabled(false);
                return match footer(ui, None, &[print, Btn::cancel()]) {
                    Some(_) => Outcome::Cancel,
                    None => Outcome::Open,
                };
            }
            Search::Failed(why) => Some(format!("LimarePDF couldn't list the printers: {why}.")),
            Search::Done if s.printers.is_empty() => Some(
                "No printers are set up. Add one in the system's printer settings, then try again."
                    .into(),
            ),
            Search::Done => None,
        };
        if let Some(why) = why {
            content(ui, |ui| ui.label(why));
            return match footer(ui, None, &[Btn::primary("Try again"), Btn::cancel()]) {
                Some(0) => Outcome::Apply,
                Some(_) => Outcome::Cancel,
                None => Outcome::Open,
            };
        }
        let ask = s.preview_ask();
        s.preview.set(ask);
        let waiting = s.setup().is_none() && s.papers_failed().is_none();
        let enter = if beside {
            let top = ui.max_rect().min.y;
            let spacing = ui.spacing().item_spacing;
            let row = ui.horizontal_top(|ui| {
                // The columns side by side, each spaced as usual inside.
                ui.spacing_mut().item_spacing.x = 0.0;
                let settings = ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = spacing;
                    ui.set_width(PRINT_SETTINGS);
                    content_below(ui, top, |ui| print_body(ui, s))
                });
                // As tall as the settings, below the title and above the buttons.
                let height = settings.response.rect.height();
                let at = insets(ui.ctx());
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = spacing;
                    ui.add_space(f32::from(at.content));
                    let size = egui::vec2(PREVIEW_WIDTH - f32::from(at.side), height);
                    let size = size - egui::vec2(0.0, 2.0 * f32::from(at.content));
                    s.preview.show(ui, size.max(egui::Vec2::ZERO), waiting);
                });
                settings.inner
            });
            row.inner
        } else {
            content(ui, |ui| {
                let enter = print_body(ui, s);
                ui.add_space(GAP);
                let size = egui::vec2(ui.available_width(), PREVIEW_UNDER);
                s.preview.show(ui, size, waiting);
                enter
            })
        };
        let can = s.setup().is_some();
        let print = match s.papers_failed() {
            Some(_) => Btn::primary("Try again"),
            None if can => Btn::primary("Print"),
            None => Btn::primary("Print")
                .enabled(false)
                .tip("Finding the printer's paper sizes…"),
        };
        match footer(ui, None, &[print, Btn::cancel()]) {
            Some(0) => Outcome::Apply,
            Some(_) => Outcome::Cancel,
            None if enter && can => Outcome::Apply,
            None => Outcome::Open,
        }
    });
    if esc {
        return Outcome::Cancel;
    }
    out
}

/// The choices for printing on one side or both.
const SIDES: [(Sides, &str); 3] = [
    (Sides::One, "One side"),
    (Sides::LongEdge, "Both sides, flip on long edge"),
    (Sides::ShortEdge, "Both sides, flip on short edge"),
];

/// The Print dialog's settings, as in the dialog mockup; true when Enter was pressed in
/// the page range.
fn print_body(ui: &mut Ui, s: &mut Print) -> bool {
    let wide = ui.available_width();
    field(ui, "Printer", |ui| {
        let name = |p: &Printer| {
            if p.default {
                format!("{} (default)", p.name)
            } else {
                p.name.clone()
            }
        };
        // A long queue name is cut short to the dialog's width, and shown whole on hover.
        let chosen = name(&s.printers[s.printer]);
        egui::ComboBox::from_id_salt("printer")
            .width(wide)
            .truncate()
            .selected_text(&chosen)
            .show_ui(ui, |ui| {
                // Open, as wide as the dialog, a long name over more than one line.
                ui.set_max_width(wide);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                for (i, p) in s.printers.iter().enumerate() {
                    ui.selectable_value(&mut s.printer, i, name(p));
                }
            })
            .response
            .on_hover_text(chosen)
    });
    let mut enter = false;
    if s.selection.is_none() {
        group(ui, "Pages", |ui| {
            let mut range = None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                ui.radio_value(&mut s.all, true, "All pages");
                range = Some(ui.radio_value(&mut s.all, false, "Page range"));
            });
            if s.all {
                return;
            }
            // Shown once Page range is chosen, taking the keyboard when chosen.
            ui.add_space(8.0);
            let r = field(ui, "Pages to print", |ui| {
                let edit = egui::TextEdit::singleline(&mut s.pages)
                    .hint_text("e.g. 1-3, 5, 8-")
                    .desired_width(f32::INFINITY);
                ui.add(theme::field(edit))
            });
            if range.is_some_and(|c| c.clicked()) {
                r.request_focus();
            }
            if r.changed() {
                s.error = None;
            }
            enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if let Some(e) = &s.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
        });
    }
    group(ui, "Copies and sides", |ui| copies_and_sides(ui, s));
    group(ui, "Page setup", |ui| page_setup(ui, s));
    group(ui, "Layout", |ui| print_layout(ui, s));
    // Windows always prints pages drawn by LimarePDF.
    if !cfg!(windows) {
        ui.add_space(GAP);
        ui.add(egui::Separator::default().spacing(0.0));
        ui.add_space(8.0);
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

/// Copies beside Print on, as two columns when the longest choice fits beside Copies;
/// one under the other otherwise.
fn copies_and_sides(ui: &mut Ui, s: &mut Print) {
    let width = ui.available_width();
    let copies_width = ((width - COLUMN_GAP) / 2.6).clamp(80.0, 95.0);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let longest = SIDES
        .iter()
        .map(|(_, l)| {
            let g = ui.painter().layout_no_wrap(
                l.to_string(),
                font.clone(),
                egui::Color32::PLACEHOLDER,
            );
            g.size().x
        })
        .fold(0.0, f32::max);
    // The list's padding and arrow.
    let sp = ui.spacing();
    let need = longest + 2.0 * sp.button_padding.x + sp.icon_width + sp.icon_spacing;
    let copies = |ui: &mut Ui, s: &mut Print, _| {
        field(ui, "Copies", |ui| {
            let spin = Spin::new(&mut s.copies, 1..=999).name("Copies");
            ui.add(spin.width(copies_width))
        });
    };
    // A booklet sets the sides itself: shown greyed, Print on's own choice kept.
    let sides = |ui: &mut Ui, s: &mut Print, width: f32| {
        let used = s.sides_used();
        let shown = SIDES.iter().find(|(x, _)| *x == used).map(|(_, l)| *l);
        ui.add_enabled_ui(!s.booklet, |ui| {
            field(ui, "Print on", |ui| {
                egui::ComboBox::from_id_salt("sides")
                    .width(width)
                    .selected_text(shown.unwrap_or_default())
                    .show_ui(ui, |ui| {
                        for (x, label) in SIDES {
                            ui.selectable_value(&mut s.sides, x, label);
                        }
                    })
                    .response
                    .on_disabled_hover_text(BOOKLET_SIDES)
            });
        });
    };
    columns(ui, s, copies_width, need, copies, sides);
}

/// The ways the pages can go on the paper.
const ORIENTATIONS: [(Orientation, &str); 3] = [
    (Orientation::Automatic, "Automatic"),
    (Orientation::Portrait, "Portrait"),
    (Orientation::Landscape, "Landscape"),
];

/// A scale's name in the list.
fn scale_name(s: Scale) -> &'static str {
    match s {
        Scale::ShrinkOversized => "Shrink oversized pages",
        Scale::Fit => "Fit to printable area",
        Scale::ActualSize => "Actual size",
        Scale::Custom(_) => "Custom",
    }
}

/// The width of the percentage beside Custom.
const PERCENT: f32 = 76.0;

/// The widest of `labels` in a list, with the list's padding and arrow.
fn list_width(ui: &Ui, labels: impl IntoIterator<Item = &'static str>) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let widest = labels
        .into_iter()
        .map(|l| {
            let g = ui.painter().layout_no_wrap(
                l.to_string(),
                font.clone(),
                egui::Color32::PLACEHOLDER,
            );
            g.size().x
        })
        .fold(0.0, f32::max);
    let sp = ui.spacing();
    widest + 2.0 * sp.button_padding.x + sp.icon_width + sp.icon_spacing
}

/// Page setup: the paper, of the sizes the printer takes once they are found; Orientation
/// beside Scale, as two columns when they fit, the percentage beside Custom.
fn page_setup(ui: &mut Ui, s: &mut Print) {
    let wide = ui.available_width();
    if let Some(why) = s.papers_failed() {
        ui.label("Paper size");
        ui.add_space(2.0);
        let why = format!("LimarePDF couldn't find the paper sizes this printer takes: {why}.");
        ui.colored_label(ui.visuals().error_fg_color, why);
    } else if let Some(labels) = s.paper_labels() {
        let chosen = labels.get(s.paper).cloned().unwrap_or_default();
        field(ui, "Paper size", |ui| {
            egui::ComboBox::from_id_salt("paper")
                .width(wide)
                .truncate()
                .selected_text(&chosen)
                .show_ui(ui, |ui| {
                    ui.set_max_width(wide);
                    for (i, label) in labels.iter().enumerate() {
                        ui.selectable_value(&mut s.paper, i, label);
                    }
                })
                .response
                .on_hover_text(chosen)
        });
    } else {
        ui.label("Paper size");
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Finding the printer's paper sizes…");
        });
    }
    ui.add_space(FIELD_GAP);
    let orientation_width = list_width(ui, ORIENTATIONS.map(|(_, l)| l));
    let scales = [
        Scale::ShrinkOversized,
        Scale::Fit,
        Scale::ActualSize,
        Scale::Custom(s.percent),
    ];
    let custom = matches!(s.scale, Scale::Custom(_));
    let beside = match custom {
        true => PERCENT + ui.spacing().item_spacing.x,
        false => 0.0,
    };
    let need = list_width(ui, scales.map(scale_name)) + beside;
    let orientation = |ui: &mut Ui, s: &mut Print, width: f32| {
        let shown = ORIENTATIONS.iter().find(|(o, _)| *o == s.orientation);
        ui.add_enabled_ui(!s.booklet, |ui| {
            field(ui, "Orientation", |ui| {
                egui::ComboBox::from_id_salt("orientation")
                    .width(width)
                    .selected_text(shown.map(|(_, l)| *l).unwrap_or_default())
                    .show_ui(ui, |ui| {
                        for (o, label) in ORIENTATIONS {
                            ui.selectable_value(&mut s.orientation, o, label);
                        }
                    })
                    .response
                    .on_disabled_hover_text(BOOKLET_SETS)
            });
        });
    };
    let scale = |ui: &mut Ui, s: &mut Print, width: f32| {
        // With several pages a sheet, or a booklet, each is fitted to its place.
        let one = s.per_sheet.pages <= 1 && !s.booklet;
        let why = if s.booklet { BOOKLET_SETS } else { FITTED };
        ui.add_enabled_ui(one, |ui| {
            ui.label("Scale");
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let list = egui::ComboBox::from_id_salt("scale")
                    .width(width - beside)
                    .selected_text(scale_name(s.scale))
                    .show_ui(ui, |ui| {
                        for x in scales {
                            ui.selectable_value(&mut s.scale, x, scale_name(x));
                        }
                    })
                    .response
                    .on_disabled_hover_text(why);
                set_name(&list, "Scale");
                if let Scale::Custom(_) = s.scale {
                    let max = limarepdf_core::print::MAX_PERCENT;
                    let n = Spin::new(&mut s.percent, 1..=max)
                        .suffix("%")
                        .name("Scale percentage")
                        .width(PERCENT);
                    let tip = format!("1 to {max}% of each page's own size");
                    ui.add(n).on_hover_text(tip).on_disabled_hover_text(why);
                    s.scale = Scale::Custom(s.percent);
                }
            });
        });
    };
    columns(ui, s, orientation_width, need, orientation, scale);
}

/// Why Scale is greyed with several pages a sheet.
const FITTED: &str = "With more than one page a sheet, each page is fitted to its place.";

/// Why Page order and borders are greyed with one page a sheet.
const SEVERAL_ONLY: &str = "For more than one page a sheet.";

/// Why Orientation, Scale, Pages per sheet and borders are greyed for a booklet.
const BOOKLET_SETS: &str =
    "A booklet sets this itself: two pages a side, each fitted to its half of the sheet.";

/// Why Print on is greyed for a booklet.
const BOOKLET_SIDES: &str = "A booklet prints both sides, flipped on the short edge, or one \
                             side when printing its fronts or backs alone.";

/// The edges a booklet can be bound along.
const BINDINGS: [(Binding, &str); 2] = [(Binding::Left, "Left"), (Binding::Right, "Right")];

/// Which sides of a booklet's sheets can print.
const BOOKLET_SIDE_CHOICES: [(BookletSides, &str); 3] = [
    (BookletSides::Both, "Both sides"),
    (BookletSides::Fronts, "Front sides only"),
    (BookletSides::Backs, "Back sides only"),
];

/// The orders pages can fill a sheet in.
const PAGE_ORDERS: [(PageOrder, &str); 4] = [
    (PageOrder::Across, "Left to right, top to bottom"),
    (PageOrder::AcrossFromRight, "Right to left, top to bottom"),
    (PageOrder::Down, "Top to bottom, left to right"),
    (PageOrder::DownFromRight, "Top to bottom, right to left"),
];

/// Layout: printing selected text, how it is laid out; then Print as a booklet; Pages per
/// sheet beside Page order, and Borders around pages under them, the last two greyed with
/// one page a sheet. As a booklet, Pages per sheet and the borders are greyed, Binding is
/// in Page order's place and Sides to print under them.
fn print_layout(ui: &mut Ui, s: &mut Print) {
    if let Some(layout) = &mut s.selection {
        layout_radios(ui, layout, &[Layout::Original, Layout::Plain]);
        ui.add_space(FIELD_GAP);
    }
    ui.checkbox(&mut s.booklet, "Print as a booklet");
    ui.add_space(FIELD_GAP);
    let label = "Pages per sheet";
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(label.into(), font, egui::Color32::PLACEHOLDER);
    // As wide as its label, or its widest number.
    let count_width = galley.size().x.max(list_width(ui, ["16"]));
    let need = list_width(ui, PAGE_ORDERS.map(|(_, l)| l));
    let count = |ui: &mut Ui, s: &mut Print, width: f32| {
        ui.add_enabled_ui(!s.booklet, |ui| {
            field(ui, label, |ui| {
                egui::ComboBox::from_id_salt("pages per sheet")
                    .width(width)
                    .selected_text(s.per_sheet.pages.to_string())
                    .show_ui(ui, |ui| {
                        for n in PAGES_PER_SHEET {
                            ui.selectable_value(&mut s.per_sheet.pages, n, n.to_string());
                        }
                    })
                    .response
                    .on_disabled_hover_text(BOOKLET_SETS)
            });
        });
    };
    let binding = |ui: &mut Ui, s: &mut Print, width: f32| {
        let shown = BINDINGS.iter().find(|(b, _)| *b == s.binding);
        field(ui, "Binding", |ui| {
            egui::ComboBox::from_id_salt("binding")
                .width(width)
                .selected_text(shown.map(|(_, l)| *l).unwrap_or_default())
                .show_ui(ui, |ui| {
                    for (b, label) in BINDINGS {
                        ui.selectable_value(&mut s.binding, b, label);
                    }
                })
                .response
        });
    };
    let order = |ui: &mut Ui, s: &mut Print, width: f32| {
        if s.booklet {
            return binding(ui, s, width);
        }
        let shown = PAGE_ORDERS.iter().find(|(o, _)| *o == s.per_sheet.order);
        ui.add_enabled_ui(s.per_sheet.pages > 1, |ui| {
            field(ui, "Page order", |ui| {
                egui::ComboBox::from_id_salt("page order")
                    .width(width)
                    .selected_text(shown.map(|(_, l)| *l).unwrap_or_default())
                    .show_ui(ui, |ui| {
                        for (o, label) in PAGE_ORDERS {
                            ui.selectable_value(&mut s.per_sheet.order, o, label);
                        }
                    })
                    .response
                    .on_disabled_hover_text(SEVERAL_ONLY)
            });
        });
    };
    columns(ui, s, count_width, need, count, order);
    if s.booklet {
        ui.add_space(FIELD_GAP);
        let labels = BOOKLET_SIDE_CHOICES.map(|(_, l)| l);
        let width = list_width(ui, labels).min(ui.available_width());
        let shown = BOOKLET_SIDE_CHOICES
            .iter()
            .find(|(b, _)| *b == s.booklet_sides);
        field(ui, "Sides to print", |ui| {
            egui::ComboBox::from_id_salt("booklet sides")
                .width(width)
                .selected_text(shown.map(|(_, l)| *l).unwrap_or_default())
                .show_ui(ui, |ui| {
                    for (b, label) in BOOKLET_SIDE_CHOICES {
                        ui.selectable_value(&mut s.booklet_sides, b, label);
                    }
                })
                .response
        });
    }
    ui.add_space(8.0);
    let why = if s.booklet {
        BOOKLET_SETS
    } else {
        SEVERAL_ONLY
    };
    ui.add_enabled_ui(s.per_sheet.pages > 1 && !s.booklet, |ui| {
        ui.checkbox(&mut s.per_sheet.borders, "Borders around pages")
            .on_disabled_hover_text(why);
    });
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
        ui.add(
            egui::Label::new(
                "Information that travels with the file but isn't on its pages. What is ticked \
                 is removed; once saved, it is gone from the file. Undo brings it back until \
                 you close the document.",
            )
            .wrap(),
        );
        // Comments and what follows are often wanted, so not ticked at first.
        let kept = s
            .kinds
            .iter()
            .position(|(h, ..)| *h == limarepdf_core::Hidden::Comments)
            .unwrap_or(s.kinds.len());
        let (removed, kept) = s.kinds.split_at_mut(kept);
        group(ui, "Removed by default", |ui| hidden_kinds(ui, removed));
        if !kept.is_empty() {
            group(ui, "Kept by default", |ui| hidden_kinds(ui, kept));
        }
        false
    })
}

/// A checkbox for each kind of hidden information, saying how many the document has. One
/// the document has none of is greyed out and unticked, as nothing of it is removed; the
/// choice stays for documents that have it (PB-08).
fn hidden_kinds(ui: &mut Ui, kinds: &mut [(limarepdf_core::Hidden, usize, bool)]) {
    for (h, n, on) in kinds {
        let found = if *n == 0 {
            "none".to_string()
        } else {
            format!("{n} found")
        };
        let mut none = false;
        let shown = if *n == 0 { &mut none } else { on };
        ui.add_enabled(
            *n > 0,
            egui::Checkbox::new(shown, format!("{} ({found})", h.label())),
        );
    }
}

pub struct Password {
    pub path: PathBuf,
    pub input: String,
    pub error: Option<String>,
}
/// The password of a file to open (Open: [`Choice::First`]), or (`appending`) of a file
/// whose pages are being added, with Skip this file ([`Choice::Second`]) as well.
pub fn password(ctx: &Context, s: &mut Password, appending: bool) -> Choice {
    // Cut short as the tab's title is.
    let name = s.path.file_name();
    let name = name.map(|n| limarepdf_core::display_line(&n.to_string_lossy(), 120));
    let shell = Shell::new("Password required").file(name.as_deref());
    let (text, action) = match appending {
        false => ("open it", "Open"),
        true => ("add its pages", "Add pages"),
    };
    let (out, esc) = shell.show(ctx, |ui| {
        let enter = content(ui, |ui| {
            let text = format!("This document is protected. Type its password to {text}.");
            ui.add(egui::Label::new(text).wrap());
            ui.add_space(GAP);
            let r = field(ui, "Password", |ui| {
                let edit = egui::TextEdit::singleline(&mut s.input)
                    .password(true)
                    .desired_width(f32::INFINITY);
                ui.add(theme::field(edit))
            });
            if let Some(e) = &s.error {
                let e = RichText::new(e).color(ui.visuals().error_fg_color);
                ui.add(egui::Label::new(e).wrap());
            }
            entered(ui, &r)
        });
        let mut buttons = vec![Btn::primary(action)];
        if appending {
            buttons.push(Btn::plain("Skip this file"));
        }
        buttons.push(Btn::cancel());
        match footer(ui, None, &buttons) {
            Some(0) => Choice::First,
            Some(i) if i + 1 < buttons.len() => Choice::Second,
            Some(_) => Choice::Cancel,
            None if enter => Choice::First,
            None => Choice::Open,
        }
    });
    if esc {
        return Choice::Cancel;
    }
    out
}

/// The name written into comments and markup; empty for none.
pub fn comment_name(ctx: &Context, name: &mut String) -> Outcome {
    let shell = Shell::new("Your name for comments");
    modal(ctx, shell, Btn::primary("Save"), true, |ui| {
        ui.add(
            egui::Label::new(
                "Comments and markup you add are saved with this name, so others see who made \
                 them. Leave it empty to save no name.",
            )
            .wrap(),
        );
        ui.add_space(GAP);
        let r = field(ui, "Name", |ui| {
            let edit = egui::TextEdit::singleline(name)
                .hint_text("No name")
                .desired_width(f32::INFINITY);
            ui.add(theme::field(edit))
        });
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
            ui.add_space(GAP);
            list_rows(ui, offers.len(), |ui, i| {
                let o = &offers[i];
                let text = |ui: &mut Ui| {
                    let title = o.title.as_deref();
                    let title = title.unwrap_or("A copy that can't be restored");
                    ui.add(egui::Label::new(RichText::new(title).strong()).wrap());
                    ui.add(egui::Label::new(RichText::new(&o.detail).weak()).wrap());
                };
                // Named for screen readers with the document each acts on.
                let what = o.title.clone().unwrap_or_else(|| format!("copy {}", i + 1));
                list_row(ui, text, |ui| {
                    if o.title.is_some() {
                        let r = ui.add(theme::primary_button("Restore"));
                        set_name(&r, &format!("Restore {what}"));
                        if r.clicked() {
                            out = RestoreOut::Restore(i);
                        }
                    } else {
                        let r = ui.button("Show file");
                        set_name(&r, &format!("Show the file of {what}"));
                        if r.clicked() {
                            out = RestoreOut::ShowFile(i);
                        }
                    }
                    let r = ui.button("Discard");
                    set_name(&r, &format!("Discard {what}"));
                    if r.clicked() {
                        out = RestoreOut::Discard(i);
                    }
                });
            });
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

// ------------------------------------------------------------------ replace all

/// What Replace all's review says ([`replace_all`]).
#[derive(Clone, Debug, Default)]
pub struct ReplaceReview {
    /// What is searched for, and what replaces it (nothing: it is deleted).
    pub query: String,
    pub with: String,
    /// The matches replaced, and how many pages they are on.
    pub replaced: usize,
    pub pages: usize,
    /// Of those, the matches that would run past the page's edge or over other text, and
    /// those pages, named.
    pub over: usize,
    pub over_pages: String,
    /// Of those, the matches that go into another font.
    pub other_font: usize,
    /// The matches not replaced: how many, and why, said after their number.
    pub left: Vec<(usize, &'static str)>,
    /// Replace where it fits replaces some: some fit and others don't.
    pub fits: bool,
}

impl ReplaceReview {
    /// What it says, a paragraph each: how many are replaced, those that would run over,
    /// those in another font, what isn't replaced, and Undo; with none replaced, that and
    /// why.
    pub fn said(&self) -> Vec<String> {
        let not_replaced = || {
            let why: Vec<String> = (self.left.iter())
                .map(|(n, why)| format!("{n} {why}"))
                .collect();
            format!("Not replaced: {}.", why.join("; "))
        };
        if self.replaced == 0 {
            let none = format!("No match of “{}” can be replaced.", self.query);
            return vec![none, not_replaced()];
        }
        let one = self.replaced == 1;
        let pages = match self.pages {
            1 => "1 page".to_string(),
            n => format!("{n} pages"),
        };
        let done = match (self.with.is_empty(), one) {
            (true, true) => "is deleted".to_string(),
            (true, false) => "are deleted".to_string(),
            (false, true) => format!("is replaced with “{}”", self.with),
            (false, false) => format!("are replaced with “{}”", self.with),
        };
        let found = limarepdf_core::match_count(self.replaced);
        let mut out = vec![format!("{found} of “{}” on {pages} {done}.", self.query)];
        // Some of those: "It" when there is one.
        let of_them = |n: usize| match one {
            true => "It".to_string(),
            false => format!("{n} of them"),
        };
        if self.over > 0 {
            let mut s = format!(
                "{} would run past the page's edge or over other text, on {}.",
                of_them(self.over),
                self.over_pages
            );
            if self.fits {
                s.push_str(" Replace where it fits leaves those as they are.");
            }
            out.push(s);
        }
        if self.other_font > 0 {
            let (goes, it) = match self.other_font {
                1 => ("goes", "it"),
                _ => ("go", "them"),
            };
            out.push(format!(
                "{} {goes} into another font: this PDF's font hasn't the letters for {it}.",
                of_them(self.other_font)
            ));
        }
        if !self.left.is_empty() {
            out.push(not_replaced());
        }
        out.push("Undo brings it all back.".to_string());
        out
    }
}

/// Replace all's review, `r`: Replace all ([`Choice::First`]), Replace where it fits
/// ([`Choice::Second`], when it replaces some, but not all), or Cancel.
pub fn replace_all(ctx: &Context, file: Option<&str>, r: &ReplaceReview) -> Choice {
    let shell = Shell::new("Replace all").file(file);
    let (out, esc) = shell.show(ctx, |ui| {
        content(ui, |ui| {
            for (i, s) in r.said().iter().enumerate() {
                if i > 0 {
                    ui.add_space(6.0);
                }
                ui.add(egui::Label::new(s).wrap());
            }
        });
        let mut buttons = vec![Btn::primary("Replace all")];
        if r.fits {
            buttons.push(Btn::plain("Replace where it fits"));
        }
        buttons.push(Btn::cancel());
        match footer(ui, None, &buttons) {
            Some(0) => Choice::First,
            Some(1) if r.fits => Choice::Second,
            Some(_) => Choice::Cancel,
            None => Choice::Open,
        }
    });
    if esc {
        return Choice::Cancel;
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

    /// The Print dialog on a screen `size` points big, at `scale` pixels per point.
    struct PrintWin {
        ctx: Context,
        s: Print,
        size: egui::Vec2,
        scale: f32,
        /// What the last frame returned.
        out: Outcome,
    }

    impl PrintWin {
        fn new(size: (f32, f32), scale: f32) -> Self {
            let ctx = Context::default();
            ctx.enable_accesskit();
            theme::install(&ctx);
            let printers = vec![
                Printer {
                    name: "Office laser".into(),
                    default: true,
                },
                Printer {
                    name: "Hall inkjet".into(),
                    default: false,
                },
            ];
            PrintWin {
                ctx,
                s: Print::with_papers(printers, "walrus.pdf".into()),
                size: size.into(),
                scale,
                out: Outcome::Open,
            }
        }

        /// Draws the dialog with `events`, and returns its AccessKit tree.
        fn frame(&mut self, events: Vec<egui::Event>) -> egui::accesskit::TreeUpdate {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                events,
                ..Default::default()
            };
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .native_pixels_per_point = Some(self.scale);
            let PrintWin { ctx, s, out, .. } = self;
            let mut o = ctx.run_ui(raw, |_| *out = print(ctx, s));
            o.textures_delta.clear();
            o.platform_output.accesskit_update.expect("the dialog")
        }

        fn settled(&mut self) -> egui::accesskit::TreeUpdate {
            for _ in 0..4 {
                self.frame(vec![]);
            }
            self.frame(vec![])
        }

        /// Clicks the control named `label`; returns the tree once settled.
        fn click(&mut self, label: &str) -> egui::accesskit::TreeUpdate {
            let tree = self.settled();
            let p = named(&tree, label).expect(label).center();
            let press = |pressed| egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(vec![egui::Event::PointerMoved(p), press(true)]);
            self.frame(vec![press(false)]);
            self.settled()
        }

        fn window(&self) -> egui::Rect {
            let id = egui::Id::unique(("dialog", "Print"));
            self.ctx.memory(|m| m.area_rect(id)).expect("the window")
        }
    }

    fn rect(b: egui::accesskit::Rect) -> egui::Rect {
        egui::Rect::from_min_max(
            egui::pos2(b.x0 as f32, b.y0 as f32),
            egui::pos2(b.x1 as f32, b.y1 as f32),
        )
    }

    /// Where the control named `label` is, if shown.
    fn named(tree: &egui::accesskit::TreeUpdate, label: &str) -> Option<egui::Rect> {
        let (_, n) = tree.nodes.iter().find(|(_, n)| n.label() == Some(label))?;
        n.bounds().map(rect)
    }

    /// Where the text `value` is, if shown.
    fn text(tree: &egui::accesskit::TreeUpdate, value: &str) -> Option<egui::Rect> {
        let (_, n) = tree.nodes.iter().find(|(_, n)| n.value() == Some(value))?;
        n.bounds().map(rect)
    }

    /// Whether the control named `label` has the keyboard.
    fn focused(tree: &egui::accesskit::TreeUpdate, label: &str) -> bool {
        tree.nodes
            .iter()
            .any(|(id, n)| n.label() == Some(label) && *id == tree.focus)
    }

    /// V01/V02: Print as in the dialog mockup, wide, at half width and at 400 points, at
    /// each scale: All pages and Page range on one row, the pages to print below them only
    /// once Page range is chosen (taking the keyboard, and keeping what was typed), Copies
    /// beside Print on unless the window is too narrow, and the dialogs' insets and button
    /// height.
    #[test]
    fn print_follows_the_mockup_at_every_size() {
        for size in [(1200.0, 800.0), (800.0, 800.0), (400.0, 800.0)] {
            for scale in [1.0, 1.5, 2.0] {
                let at = format!("{size:?} at {scale}");
                let mut w = PrintWin::new(size, scale);
                let tree = w.settled();
                let get = |label: &str| named(&tree, label).expect(label);
                let (all, range) = (get("All pages"), get("Page range"));
                assert!((all.center().y - range.center().y).abs() < 1.0, "{at}");
                assert!(range.left() > all.right(), "{at}");
                assert!(named(&tree, "Pages to print").is_none(), "{at}");
                let (copies, sides) = (get("Copies"), get("Print on"));
                if size.0 >= 800.0 {
                    assert!((copies.top() - sides.top()).abs() < 1.0, "{at}");
                    assert!(sides.left() > copies.right(), "{at}");
                } else {
                    assert!(sides.top() > copies.bottom(), "{at}");
                }
                // The insets and the footer's buttons.
                let window = w.window();
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size.into());
                assert!(screen.expand(0.5).contains_rect(window), "{at}: {window:?}");
                // Inside the window's outline.
                let edge = w.ctx.global_style().visuals.window_stroke.width;
                let side = edge + if size.0 < NARROW { 16.0 } else { SIDE };
                let title = text(&tree, "Print").expect("the title");
                assert!(
                    (title.left() - window.left() - side).abs() < 1.0,
                    "{at}: {title:?} in {window:?}"
                );
                assert!((get("Printer").left() - window.left() - side).abs() < 1.0);
                let cancel = get("Cancel");
                assert!(cancel.height() >= theme::DIALOG_CONTROL - 0.5, "{at}");
                assert!(window.contains_rect(cancel), "{at}");

                let tree = w.click("Page range");
                assert!(!w.s.all, "{at}");
                let field = named(&tree, "Pages to print").expect("the pages to print");
                let row = named(&tree, "Page range").unwrap();
                assert!(field.top() > row.bottom(), "{at}");
                assert!(
                    field.bottom() < named(&tree, "Copies").unwrap().top(),
                    "{at}"
                );
                assert!(focused(&tree, "Pages to print"), "{at}");
                assert!(field.height() >= theme::DIALOG_CONTROL - 0.5, "{at}");
                w.frame(vec![egui::Event::Text("2-3".into())]);
                assert_eq!(w.s.pages, "2-3", "{at}");

                let tree = w.click("All pages");
                assert!(w.s.all && named(&tree, "Pages to print").is_none(), "{at}");
                let tree = w.click("Page range");
                assert!(text(&tree, "2-3").is_some(), "{at}: typing lost");
                assert_eq!(w.s.range(), "2-3");
                let enter = egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                };
                w.frame(vec![enter]);
                assert!(w.out == Outcome::Apply, "{at}: Enter didn't print");
            }
        }
    }

    /// Print fits a short screen, its buttons in view, with the pages to print shown.
    #[test]
    fn print_fits_a_short_screen() {
        for size in [(1200.0, 300.0), (400.0, 300.0)] {
            let mut w = PrintWin::new(size, 1.0);
            w.s.all = false;
            w.s.pages = "1-3".into();
            let tree = w.settled();
            let window = w.window();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size.into());
            assert!(
                screen.expand(0.5).contains_rect(window),
                "{size:?}: {window:?}"
            );
            for b in ["Print", "Cancel"] {
                let r = named(&tree, b).expect(b);
                assert!(window.contains_rect(r), "{size:?}: {b} {r:?}");
            }
        }
    }

    /// Print on is a list, as in the mockup, and picking from it sets the sides.
    #[test]
    fn print_on_is_picked_from_a_list() {
        let mut w = PrintWin::new((1200.0, 800.0), 1.0);
        w.click("Print on");
        w.click("Both sides, flip on long edge");
        assert_eq!(w.s.sides, Sides::LongEdge);
        let tree = w.settled();
        assert!(named(&tree, "Both sides, flip on short edge").is_none());
    }

    /// ENH-14: Page setup under Copies and sides, its fields labelled above them: the
    /// printer's paper sizes, then Orientation beside Scale where they fit; picking from
    /// the lists sets them, and Custom shows its percentage beside Scale. The preview is
    /// beside the settings in a wide window and under them in a narrow one, the sheet's
    /// number under it.
    #[test]
    fn page_setup_and_the_preview_follow_the_mockup() {
        for size in [(1200.0, 800.0), (400.0, 800.0)] {
            let at = format!("{size:?}");
            let mut w = PrintWin::new(size, 1.0);
            let tree = w.settled();
            let get = |label: &str| named(&tree, label).expect(label);
            for f in ["Paper size", "Orientation", "Scale"] {
                label_above(&tree, f).unwrap();
            }
            assert!(get("Paper size").top() > get("Print on").bottom(), "{at}");
            let (o, scale) = (get("Orientation"), get("Scale"));
            let beside = (o.top() - scale.top()).abs() < 1.0 && scale.left() > o.right();
            if size.0 >= 800.0 {
                assert!(beside, "{at}: {o:?} {scale:?}");
            } else {
                assert!(scale.top() > o.bottom(), "{at}: {o:?} {scale:?}");
            }
            assert!(named(&tree, "Scale percentage").is_none(), "{at}");
            let prev = get("Previous sheet");
            assert!(text(&tree, "1 of 1").is_some(), "{at}: the sheet's number");
            if size.0 >= 800.0 {
                let window = w.window();
                assert!(window.contains_rect(prev), "{at}: {prev:?} in {window:?}");
                assert!(prev.left() > get("Printer").right(), "{at}: not beside");
            } else {
                // Under the settings, scrolled to.
                assert!(prev.top() > get("Scale").bottom(), "{at}: not under");
            }
            w.click("Paper size");
            w.click("A4 (210 × 297 mm)");
            assert_eq!(w.s.paper, 1, "{at}");
            assert_eq!(w.s.setup().unwrap().paper.id, "iso_a4_210x297mm");
            w.click("Orientation");
            w.click("Landscape");
            assert_eq!(w.s.orientation, Orientation::Landscape, "{at}");
            w.click("Scale");
            let tree = w.click("Custom");
            assert_eq!(w.s.scale, Scale::Custom(100), "{at}");
            let scale = named(&tree, "Scale").unwrap();
            let n = named(&tree, "Scale percentage").expect("the percentage");
            let window = w.window();
            assert!((scale.center().y - n.center().y).abs() < 1.0, "{at}");
            let fits = n.left() > scale.right() && window.contains_rect(n);
            assert!(fits, "{at}: {scale:?} {n:?} in {window:?}");
            w.s.percent = 40;
            w.settled();
            assert_eq!(w.s.scale, Scale::Custom(40), "{at}");
        }
    }

    /// NEW-05: Layout under Page setup: Pages per sheet beside Page order where they fit,
    /// one under the other otherwise, Borders around pages under them; the order and the
    /// borders greyed with one page a sheet, Scale greyed with more; picking sets the page
    /// setup.
    #[test]
    fn layout_puts_several_pages_on_a_sheet() {
        let greyed = |tree: &egui::accesskit::TreeUpdate, name: &str| {
            let (_, n) = (tree.nodes.iter())
                .find(|(_, n)| n.label() == Some(name))
                .expect(name);
            n.is_disabled()
        };
        for size in [(1200.0, 1200.0), (400.0, 1200.0)] {
            let at = format!("{size:?}");
            let mut w = PrintWin::new(size, 1.0);
            let tree = w.settled();
            let get = |label: &str| named(&tree, label).expect(label);
            for f in ["Pages per sheet", "Page order"] {
                label_above(&tree, f).unwrap();
            }
            let (count, order) = (get("Pages per sheet"), get("Page order"));
            assert!(count.top() > get("Scale").bottom(), "{at}");
            let beside = (count.top() - order.top()).abs() < 1.0 && order.left() > count.right();
            if size.0 >= 800.0 {
                assert!(beside, "{at}: {count:?} {order:?}");
            } else {
                assert!(order.top() > count.bottom(), "{at}: {count:?} {order:?}");
            }
            let borders = get("Borders around pages");
            assert!(borders.top() > order.bottom(), "{at}");
            assert!(w.window().contains_rect(borders), "{at}");
            assert!(greyed(&tree, "Page order") && greyed(&tree, "Borders around pages"));
            assert!(!greyed(&tree, "Scale"), "{at}");
            assert!(unclear_names(&tree).is_empty(), "{at}");
            w.click("Pages per sheet");
            let tree = w.click("4");
            assert_eq!(w.s.per_sheet.pages, 4, "{at}");
            assert!(!greyed(&tree, "Page order") && !greyed(&tree, "Borders around pages"));
            assert!(greyed(&tree, "Scale"), "{at}: Scale isn't greyed");
            w.click("Page order");
            w.click("Top to bottom, right to left");
            w.click("Borders around pages");
            let per = w.s.setup().unwrap().per_sheet;
            assert_eq!(per.order, PageOrder::DownFromRight, "{at}");
            assert!(per.borders && per.pages == 4, "{at}");
            // The preview counts the sheets: one, of the document's one page.
            let tree = w.settled();
            assert!(text(&tree, "1 of 1").is_some(), "{at}");
        }
    }

    /// NEW-05: Print as a booklet at the top of Layout. Ticked, Pages per sheet is greyed
    /// with Binding beside it in Page order's place where they fit, Sides to print under
    /// them; Borders, Orientation and Scale are greyed, and Print on greyed showing the
    /// booklet's sides, the choice made there kept for when it is unticked. The preview
    /// counts the sides printed.
    #[test]
    fn layout_prints_a_booklet() {
        let greyed = |tree: &egui::accesskit::TreeUpdate, name: &str| {
            let (_, n) = (tree.nodes.iter())
                .find(|(_, n)| n.label() == Some(name))
                .expect(name);
            n.is_disabled()
        };
        for size in [(1200.0, 1200.0), (400.0, 1200.0)] {
            let at = format!("{size:?}");
            let mut w = PrintWin::new(size, 1.0);
            w.click("Print on");
            w.click("Both sides, flip on long edge");
            let tree = w.settled();
            assert!(named(&tree, "Binding").is_none(), "{at}");
            assert!(named(&tree, "Sides to print").is_none(), "{at}");
            let tree = w.click("Print as a booklet");
            assert!(w.s.booklet, "{at}");
            let get = |label: &str| named(&tree, label).expect(label);
            for f in ["Pages per sheet", "Binding", "Sides to print"] {
                label_above(&tree, f).unwrap();
            }
            assert!(named(&tree, "Page order").is_none(), "{at}");
            let (count, binding) = (get("Pages per sheet"), get("Binding"));
            assert!(count.top() > get("Print as a booklet").bottom(), "{at}");
            let beside =
                (count.top() - binding.top()).abs() < 1.0 && binding.left() > count.right();
            if size.0 >= 800.0 {
                assert!(beside, "{at}: {count:?} {binding:?}");
            } else {
                assert!(
                    binding.top() > count.bottom(),
                    "{at}: {count:?} {binding:?}"
                );
            }
            let sides = get("Sides to print");
            assert!(sides.top() > binding.bottom(), "{at}");
            assert!(get("Borders around pages").top() > sides.bottom(), "{at}");
            assert!(w.window().contains_rect(sides), "{at}");
            for name in [
                "Pages per sheet",
                "Borders around pages",
                "Orientation",
                "Scale",
                "Print on",
            ] {
                assert!(greyed(&tree, name), "{at}: {name} isn't greyed");
            }
            assert!(!greyed(&tree, "Binding") && !greyed(&tree, "Sides to print"));
            assert!(unclear_names(&tree).is_empty(), "{at}");
            assert_eq!(w.s.sides_used(), Sides::ShortEdge, "{at}");
            w.click("Binding");
            w.click("Right");
            w.click("Sides to print");
            w.click("Back sides only");
            let b = w.s.setup().unwrap().booklet.expect("a booklet");
            assert_eq!(b.binding, Binding::Right, "{at}");
            assert_eq!(b.sides, BookletSides::Backs, "{at}");
            assert_eq!(w.s.sides_used(), Sides::One, "{at}");
            // The document's one page: the back of its one sheet, blank.
            let tree = w.settled();
            assert!(text(&tree, "1 of 1").is_some(), "{at}");
            assert!(text(&tree, "Back of sheet 1").is_some(), "{at}");
            // Unticked, Print on is as it was chosen.
            let tree = w.click("Print as a booklet");
            assert!(w.s.setup().unwrap().booklet.is_none(), "{at}");
            assert_eq!(w.s.sides_used(), Sides::LongEdge, "{at}");
            assert!(
                !greyed(&tree, "Print on") && !greyed(&tree, "Orientation"),
                "{at}"
            );
            assert!(named(&tree, "Binding").is_none(), "{at}");
        }
    }

    /// Printing selected text, its layout is chosen in the same Layout group, above Pages
    /// per sheet: one group of that name.
    #[test]
    fn printing_a_selection_chooses_its_layout_in_layout() {
        let mut w = PrintWin::new((1200.0, 1200.0), 1.0);
        w.s.selection = Some(Layout::Original);
        let tree = w.settled();
        let plain = named(&tree, Layout::Plain.label()).expect("the layouts");
        let count = named(&tree, "Pages per sheet").unwrap();
        assert!(count.top() > plain.bottom());
        // Shown as often as another group's title.
        let shown = |title: &str| {
            let titles = tree.nodes.iter().filter(|(_, n)| n.value() == Some(title));
            titles.count()
        };
        assert_eq!(shown("Layout"), shown("Page setup"));
    }

    /// ENH-14: Print waits for the printer's paper sizes, greyed and saying so, Enter in the
    /// page range doing nothing meanwhile; sizes that can't be found are said, with Try
    /// again.
    #[test]
    fn print_waits_for_the_paper_sizes() {
        let mut w = PrintWin::new((1200.0, 800.0), 1.0);
        w.s.papers = Papers::Asking(7, "Office laser".into());
        let tree = w.settled();
        assert!(text(&tree, "Finding the printer's paper sizes…").is_some());
        let greyed = |tree: &egui::accesskit::TreeUpdate, name: &str| {
            let (_, n) = (tree.nodes.iter())
                .find(|(_, n)| n.label() == Some(name))
                .expect(name);
            n.is_disabled()
        };
        assert!(greyed(&tree, "Print"), "Print isn't greyed");
        assert!(named(&tree, "Paper size").is_none());
        w.click("Page range");
        let enter = egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        w.frame(vec![egui::Event::Text("1".into())]);
        w.frame(vec![enter]);
        assert!(w.out == Outcome::Open, "Enter printed without a paper");
        let why = "the printing service didn't answer within 10 seconds";
        w.s.papers_found(7, Err(why.into()), None);
        let tree = w.settled();
        let said = format!("LimarePDF couldn't find the paper sizes this printer takes: {why}.");
        assert!(text(&tree, &said).is_some(), "the reason isn't said");
        assert!(!greyed(&tree, "Try again"));
        let p = named(&tree, "Try again").unwrap().center();
        let press = |pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        w.frame(vec![egui::Event::PointerMoved(p), press(true)]);
        w.frame(vec![press(false)]);
        assert!(w.out == Outcome::Apply, "Try again did nothing");
    }

    /// ENH-14: the paper sizes asked for are the chosen printer's; an earlier search's come
    /// too late and are left out; the paper last used with the printer is chosen, else its
    /// default.
    #[test]
    fn the_paper_sizes_are_the_chosen_printers() {
        let printers = vec![
            Printer {
                name: "Office laser".into(),
                default: true,
            },
            Printer {
                name: "Hall inkjet".into(),
                default: false,
            },
        ];
        let mut s = Print::with_papers(printers, "walrus.pdf".into());
        let list = match &s.papers {
            Papers::Found(_, list) => list.clone(),
            p => panic!("{p:?}"),
        };
        assert_eq!(s.papers_wanted(), None);
        s.printer = 1;
        assert_eq!(s.papers_wanted().as_deref(), Some("Hall inkjet"));
        assert!(s.setup().is_none(), "the other printer's paper");
        s.papers = Papers::Asking(3, "Hall inkjet".into());
        assert_eq!(s.papers_wanted(), None);
        s.papers_found(2, Ok(PaperList::default()), None);
        assert_eq!(s.papers, Papers::Asking(3, "Hall inkjet".into()));
        s.papers_found(3, Ok(list.clone()), Some("iso_a4_210x297mm"));
        assert_eq!(s.setup().unwrap().paper.id, "iso_a4_210x297mm");
        // A paper the printer no longer takes: its default.
        s.papers = Papers::Asking(4, "Hall inkjet".into());
        s.papers_found(4, Ok(list), Some("na_legal_8.5x14in"));
        assert_eq!(s.setup().unwrap().paper.id, "na_letter_8.5x11in");
        // Not ready while the printers are looked for.
        let s = Print::looking(9, "walrus.pdf".into());
        assert_eq!(s.papers_wanted(), None);
    }

    /// BUG-50: a dialog keeps its top as its content grows (Page range's field shown,
    /// Advanced opened), extending downward as desktop dialogs do; it moved up by half of
    /// what was added, kept centred.
    #[test]
    fn print_keeps_its_top_as_it_grows() {
        // Tall enough for it all, Layout's group too (NEW-05).
        let mut w = PrintWin::new((1200.0, 1200.0), 1.0);
        w.settled();
        let before = w.window();
        w.click("Page range");
        let ranged = w.window();
        assert!(ranged.height() > before.height() + 10.0, "{ranged:?}");
        let kept = |r: egui::Rect| (r.top() - before.top()).abs() < 0.5;
        assert!(kept(ranged), "{before:?} to {ranged:?}");
        // Advanced isn't on Windows, which always prints pages drawn by LimarePDF.
        if !cfg!(windows) {
            w.click("Advanced");
            for _ in 0..10 {
                w.frame(vec![]);
            }
            let open = w.window();
            assert!(open.height() > ranged.height() + 10.0, "{open:?}");
            assert!(kept(open), "{before:?} to {open:?}");
        }
    }

    /// ENH-48: Copies is a spin box: a number out of range typed in becomes the nearest
    /// allowed, and a note under it, in the dialog, says so; ▲ steps it.
    #[test]
    fn copies_out_of_range_uses_the_nearest_and_says_so() {
        let mut w = PrintWin::new((1200.0, 800.0), 1.0);
        w.click("Copies");
        w.frame(vec![egui::Event::Text("0".into())]);
        let enter = egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        w.frame(vec![enter]);
        let tree = w.settled();
        assert_eq!(w.s.copies, 1);
        let said = "Copies can be 1 to 999, so 1 was used.";
        let note = text(&tree, said).expect("the note");
        let copies = named(&tree, "Copies").unwrap();
        assert!(note.top() >= copies.bottom(), "{note:?} under {copies:?}");
        let window = w.window();
        assert!(window.contains_rect(note), "{note:?} outside {window:?}");
        let tree = w.click("Increase Copies");
        assert_eq!(w.s.copies, 2);
        assert!(named(&tree, "Decrease Copies").is_some());
    }

    /// ENH-47: printers found choose the one last printed to when it is listed, else the
    /// default; a fuller list keeps the printer chosen; a list that can't be had is said only
    /// when no printers are shown.
    #[test]
    fn the_printer_chosen_is_kept_as_more_are_found() {
        let p = |name: &str, default| Printer {
            name: name.into(),
            default,
        };
        let mut s = Print::looking(3, "walrus.pdf".into());
        s.found(Ok(vec![p("Office", true)]), Some("Hall"));
        assert!(s.ready());
        assert_eq!(s.printers[s.printer].name, "Office");
        let more = vec![p("Office", true), p("Attic", false), p("Hall", false)];
        s.found(Ok(more.clone()), Some("Hall"));
        assert_eq!(s.printers[s.printer].name, "Office", "the choice changed");
        s.printer = 1;
        s.found(Ok(more.clone()), Some("Hall"));
        assert_eq!(s.printers[s.printer].name, "Attic");
        s.found(Err("lpstat: scheduler is not running".into()), None);
        assert!(s.ready() && s.printers.len() == 3, "{:?}", s.search);
        // A new search: the printer last printed to.
        let mut s = Print::looking(4, "walrus.pdf".into());
        s.found(Ok(more), Some("Hall"));
        assert_eq!(s.printers[s.printer].name, "Hall");
        let mut s = Print::looking(5, "walrus.pdf".into());
        s.found(Err("lpstat: scheduler is not running".into()), None);
        assert!(matches!(s.search, Search::Failed(_)));
        // Printers found earlier in the session: a service that no longer answers is said.
        let mut s = Print::looking(6, "walrus.pdf".into());
        s.found_earlier(vec![p("Office", true)], None, 6);
        assert!(s.ready() && s.listing == Some(6) && s.earlier);
        s.found(Err("lpstat: scheduler is not running".into()), None);
        assert!(matches!(s.search, Search::Failed(_)), "{:?}", s.search);
    }

    /// A long printer name keeps Print inside a narrow screen, chosen and in the open list.
    #[test]
    fn a_long_printer_name_fits_chosen_and_listed() {
        let long = "Upstairs_colour_inkjet_beside_the_staff_room_on_the_second_floor_near_the_lift";
        let mut w = PrintWin::new((400.0, 800.0), 1.0);
        w.s.printers[1].name = long.into();
        w.s.printer = 1;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, w.size).expand(0.5);
        w.settled();
        assert!(screen.contains_rect(w.window()), "{:?}", w.window());
        let tree = w.click("Printer");
        let item = named(&tree, long).expect("the listed printer");
        assert!(screen.contains_rect(item), "{item:?}");
    }

    /// A page range that can't be printed is reported under it, above Copies.
    #[test]
    fn a_wrong_page_range_is_reported_under_it() {
        let mut w = PrintWin::new((1200.0, 800.0), 1.0);
        w.s.all = false;
        w.s.pages = "9".into();
        w.s.error = Some("There is no page 9.".into());
        let tree = w.settled();
        let e = text(&tree, "There is no page 9.").expect("the error");
        assert!(e.top() > named(&tree, "Pages to print").unwrap().bottom());
        assert!(e.bottom() < named(&tree, "Copies").unwrap().top());
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
        let id = egui::Id::unique(("dialog", "Recover unsaved changes"));
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

    /// Batch 7b: Password required and Your name for comments label their fields above
    /// them, as Print does, wide and narrow.
    #[test]
    fn password_and_name_have_labels_above_their_fields() {
        for size in [egui::vec2(1200.0, 800.0), egui::vec2(400.0, 800.0)] {
            let mut s = Password {
                path: PathBuf::from("walrus.pdf"),
                input: String::new(),
                error: Some("That isn't the password.".into()),
            };
            let tree = drawn(size, |ctx| {
                password(ctx, &mut s, false);
            });
            label_above(&tree, "Password").unwrap();
            let field = named(&tree, "Password").unwrap();
            let e = text(&tree, "That isn't the password.").expect("the error");
            assert!(e.top() >= field.bottom(), "{size:?}: not under it");
            let mut name = "Sam".to_string();
            let tree = drawn(size, |ctx| {
                comment_name(ctx, &mut name);
            });
            label_above(&tree, "Name").unwrap();
        }
    }

    /// Batch 7b: Remove hidden information in two headed groups, what is ticked at first
    /// and what isn't, each heading above its kinds.
    #[test]
    fn hidden_information_is_in_two_groups() {
        use limarepdf_core::Hidden;
        let mut s = RemoveHidden {
            kinds: Hidden::ALL
                .iter()
                .map(|&h| (h, 1, (h as usize) < Hidden::Comments as usize))
                .collect(),
        };
        let tree = drawn(egui::vec2(1200.0, 800.0), |ctx| {
            remove_hidden(ctx, Some("walrus.pdf"), &mut s);
        });
        let kind = |h: Hidden| {
            let name = format!("{} (1 found)", h.label());
            named(&tree, &name).unwrap_or_else(|| panic!("no {name}"))
        };
        let removed = text(&tree, "Removed by default").expect("the first heading");
        let kept = text(&tree, "Kept by default").expect("the second heading");
        assert!(removed.bottom() <= kind(Hidden::Metadata).top());
        assert!(kind(Hidden::PrivateData).bottom() <= kept.top());
        assert!(kept.bottom() <= kind(Hidden::Comments).top());
    }

    /// PB-08: a kind the document has none of is greyed out and unticked, though chosen; one
    /// it has shows the choice.
    #[test]
    fn hidden_kinds_the_document_lacks_are_unticked() {
        use egui::accesskit::Toggled;
        use limarepdf_core::Hidden;
        let mut s = RemoveHidden {
            kinds: vec![(Hidden::Metadata, 0, true), (Hidden::Scripts, 2, true)],
        };
        let tree = drawn(egui::vec2(1200.0, 800.0), |ctx| {
            remove_hidden(ctx, None, &mut s);
        });
        let node = |name: String| {
            let n = tree.nodes.iter().find(|(_, n)| n.label() == Some(&name));
            n.map(|(_, n)| (n.toggled(), n.is_disabled()))
        };
        let none = node(format!("{} (none)", Hidden::Metadata.label()));
        assert_eq!(none, Some((Some(Toggled::False), true)));
        let found = node(format!("{} (2 found)", Hidden::Scripts.label()));
        assert_eq!(found, Some((Some(Toggled::True), false)));
        assert!(s.kinds[0].2, "the choice is kept");
    }

    /// Batch 7b: recovered documents are rows of one list, each with its buttons beside
    /// it, or under it when the window is narrow.
    #[test]
    fn recovered_documents_are_rows_with_their_buttons() {
        let offers = [Offer {
            title: Some("Report.pdf".into()),
            detail: "From /home/sam/Documents, 3 pages".into(),
        }];
        for (width, beside) in [(1000.0, true), (400.0, false)] {
            let tree = drawn(egui::vec2(width, 800.0), |ctx| {
                restore(ctx, &offers);
            });
            let title = text(&tree, "Report.pdf").expect("the title");
            let detail = text(&tree, "From /home/sam/Documents, 3 pages").unwrap();
            let restore = named(&tree, "Restore Report.pdf");
            let discard = named(&tree, "Discard Report.pdf");
            let (restore, discard) = (restore.unwrap(), discard.unwrap());
            if beside {
                let text = title.right().max(detail.right());
                assert!(restore.left() > text, "{width}");
                assert!(restore.y_range().intersects(title.y_range()), "{width}");
            } else {
                assert!(restore.top() >= detail.bottom(), "{width}");
            }
            assert!(discard.left() > restore.right(), "{width}: Tab order");
        }
    }

    /// G08: Recover unsaved changes, with several documents and a copy that can't be
    /// restored, wide, narrow and short: each row's buttons say which document they act on,
    /// every control is clearly named, the window and Not now are on screen, and Esc keeps
    /// the copies (Not now).
    #[test]
    fn each_recovered_document_s_buttons_say_which_it_is() {
        let offers = [
            Offer {
                title: Some("Report.pdf".into()),
                detail: "From /home/sam/Documents, 3 pages".into(),
            },
            Offer {
                title: Some("Minutes.pdf".into()),
                detail: "From /home/sam/Documents, 1 page".into(),
            },
            Offer {
                title: None,
                detail: "It can't be restored: its file is damaged.".into(),
            },
        ];
        for size in [
            (1000.0, 800.0),
            (400.0, 800.0),
            (1000.0, 300.0),
            (400.0, 300.0),
        ] {
            let size = egui::Vec2::from(size);
            let tree = drawn(size, |ctx| {
                restore(ctx, &offers);
            });
            assert_eq!(unclear_names(&tree), [""; 0], "{size:?}");
            for name in [
                "Restore Report.pdf",
                "Discard Report.pdf",
                "Restore Minutes.pdf",
                "Discard Minutes.pdf",
                "Show the file of copy 3",
                "Discard copy 3",
            ] {
                assert!(named(&tree, name).is_some(), "{size:?}: no {name}");
            }
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size).expand(0.5);
            let not_now = named(&tree, "Not now").expect("Not now");
            assert!(screen.contains_rect(not_now), "{size:?}: {not_now:?}");

            let ctx = Context::default();
            theme::install(&ctx);
            let mut out = RestoreOut::Open;
            for events in [vec![], vec![], vec![], vec![], vec![esc()]] {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                };
                ctx.run_ui(raw, |_| out = restore(&ctx, &offers))
                    .textures_delta
                    .clear();
            }
            assert_eq!(out, RestoreOut::NotNow, "{size:?}");
        }
    }

    fn esc() -> egui::Event {
        egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    /// G08: a message's Copy puts its title and text on the clipboard, for passing on an
    /// error's details, and leaves the message open; OK closes it.
    #[test]
    fn a_message_can_be_copied() {
        let ctx = Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let (title, text) = ("Couldn't save", "The folder /home/sam can't be written to.");
        let frame = |events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let mut outcome = Outcome::Open;
            let mut o = ctx.run_ui(raw, |_| outcome = message(&ctx, title, text));
            o.textures_delta.clear();
            (outcome, o.platform_output)
        };
        let press = |at: egui::Pos2| {
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            [
                vec![egui::Event::PointerMoved(at), button(true)],
                vec![button(false)],
            ]
        };
        for _ in 0..3 {
            frame(vec![]);
        }
        let tree = frame(vec![]).1.accesskit_update.expect("the message");
        let copy = named(&tree, "Copy").expect("Copy").center();
        let [down, up] = press(copy);
        frame(down);
        let (outcome, output) = frame(up);
        let copied = output
            .commands
            .iter()
            .any(|c| *c == egui::OutputCommand::CopyText(format!("{title}\n\n{text}")));
        assert!(copied, "{:?}", output.commands);
        assert!(outcome == Outcome::Open, "Copy closed the message");
        let tree = frame(vec![]).1.accesskit_update.expect("the message");
        let ok = named(&tree, "OK").expect("OK").center();
        let [down, up] = press(ok);
        frame(down);
        assert!(frame(up).0 == Outcome::Cancel, "OK didn't close it");
    }
}
