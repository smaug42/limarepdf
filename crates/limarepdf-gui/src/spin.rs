//! The number box (ENH-48): a spin box, as word processors and PDF editors have them (Word,
//! LibreOffice, Acrobat). Type a number, or step it with the ▲ and ▼ at its right or the Up
//! and Down keys (Page Up and Page Down, ten steps); dragging doesn't change it. What is
//! typed takes effect on Enter or on leaving the box. A number out of range is replaced by
//! the nearest allowed, as LibreOffice does, and a note under the box says so for a few
//! seconds, as does a box left with something that isn't a number. Every number box in
//! LimarePDF is one of these, and a test keeps it so.

use crate::icon;
use crate::theme;
use eframe::egui::{
    self, accesskit, emath::Numeric, text::CCursorRange, Align2, Color32, Key, Margin, Modifiers,
    Rect, Response, Sense, Ui, Vec2, Widget,
};
use std::ops::RangeInclusive;

/// How long a note under a box stays, in seconds.
const NOTE_FOR: f64 = 6.0;

/// The width of the arrows at the box's right.
const ARROWS: f32 = 18.0;

/// A number box for a value within a range. Added with `ui.add`, as egui's widgets are; its
/// response is marked changed when the value changed, typed (on Enter or leaving the box)
/// or stepped.
pub struct Spin<'a, N: Numeric> {
    value: &'a mut N,
    range: RangeInclusive<N>,
    step: f64,
    suffix: &'a str,
    name: &'a str,
    mixed: bool,
    width: Option<f32>,
}

impl<'a, N: Numeric> Spin<'a, N> {
    /// A box for `value`, set within `range`. A value outside the range (a document's) is
    /// shown as it is until it is changed.
    pub fn new(value: &'a mut N, range: RangeInclusive<N>) -> Self {
        Spin {
            value,
            range,
            step: 1.0,
            suffix: "",
            name: "",
            mixed: false,
            width: None,
        }
    }
    /// What follows the number, such as " pt" or "%".
    pub fn suffix(self, suffix: &'a str) -> Self {
        Spin { suffix, ..self }
    }
    /// How much a step changes it: 1 unless set.
    pub fn step(self, step: f64) -> Self {
        Spin { step, ..self }
    }
    /// Its name, for screen readers and the note under it.
    pub fn name(self, name: &'a str) -> Self {
        Spin { name, ..self }
    }
    /// Shown empty, as when the objects it is for differ, until a number is typed or
    /// stepped.
    pub fn mixed(self, mixed: bool) -> Self {
        Spin { mixed, ..self }
    }
    /// The box's whole width, the arrows' too. Unless set, as wide as the range's ends.
    pub fn width(self, width: f32) -> Self {
        Spin {
            width: Some(width),
            ..self
        }
    }
}

/// `v` as a box for `N` shows it: a whole number for whole numbers, else to two places at
/// most.
fn written<N: Numeric>(v: f64) -> String {
    if N::INTEGRAL {
        return format!("{}", v.round() as i64);
    }
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.into()
    }
}

/// The number in `text`, with or without the box's `suffix`. Commas between groups of three
/// digits are thousands' ("1,000"); a comma otherwise, with no point, is the decimal point.
fn typed(text: &str, suffix: &str) -> Option<f64> {
    let mut t = text.trim();
    let s = suffix.trim();
    if !s.is_empty() && t.len() >= s.len() && t.is_char_boundary(t.len() - s.len()) {
        let (number, end) = t.split_at(t.len() - s.len());
        if end.eq_ignore_ascii_case(s) {
            t = number.trim_end();
        }
    }
    let mut t = t.to_string();
    if t.contains(',') && !t.contains('.') {
        let mut groups = t.split(',');
        let first = groups.next().unwrap_or("");
        let thousands = !first.is_empty()
            && first.len() <= 4
            && groups.all(|g| g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit()));
        t = match thousands {
            true => t.replace(',', ""),
            false => t.replacen(',', ".", 1),
        };
    }
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

impl<N: Numeric> Widget for Spin<'_, N> {
    fn ui(self, ui: &mut Ui) -> Response {
        let Spin {
            value,
            range,
            step,
            suffix,
            name,
            mixed,
            width,
        } = self;
        let (lo, hi) = (range.start().to_f64(), range.end().to_f64());
        let id = ui.next_auto_id();
        ui.skip_ahead_auto_ids(1);
        let (typing_id, note_id) = (id.with("typing"), id.with("note"));
        let shown = |v: f64| format!("{}{suffix}", written::<N>(v));
        let font = egui::TextStyle::Body.resolve(ui.style());
        let measure = |t: String| {
            let g = ui
                .painter()
                .layout_no_wrap(t, font.clone(), Color32::PLACEHOLDER);
            g.size()
        };
        // As wide as the range's ends shown, so it doesn't change with the value.
        let widest = [lo, hi]
            .iter()
            .map(|&v| match N::INTEGRAL {
                true => shown(v),
                false => format!("{v:.2}{suffix}"),
            })
            .map(|t| measure(t).x)
            .fold(0.0, f32::max);
        let pad = ui.spacing().button_padding;
        let height = (measure("0".into()).y + 2.0 * pad.y).max(ui.spacing().interact_size.y);
        let width = width
            .unwrap_or(widest + 2.0 * pad.x + ARROWS)
            .max(ui.spacing().interact_size.x);
        let enabled = ui.is_enabled();

        // Up and Down while it has the keyboard step it; Page Up and Page Down ten steps.
        let had = ui.memory(|m| m.has_focus(id));
        let mut steps = 0i32;
        if had && enabled {
            ui.input_mut(|i| {
                let mut count =
                    |key, by: i32| by * i.count_and_consume_key(Modifiers::NONE, key) as i32;
                steps += count(Key::ArrowUp, 1) - count(Key::ArrowDown, 1);
                steps += count(Key::PageUp, 10) - count(Key::PageDown, 10);
            });
        }
        // What is being typed while it has the keyboard, else the value.
        let mut text = match ui.data(|d| d.get_temp::<String>(typing_id)) {
            Some(t) if had => t,
            _ if mixed => String::new(),
            _ => shown(value.to_f64()),
        };
        let margin = Margin {
            left: pad.x as i8,
            right: (ARROWS + 2.0) as i8,
            top: 0,
            bottom: 0,
        };
        let edit = egui::TextEdit::singleline(&mut text)
            .id(id)
            .desired_width(width - f32::from(margin.left) - f32::from(margin.right))
            .min_size(Vec2::new(width, height))
            .margin(margin)
            .align(Align2::LEFT_CENTER);
        let mut out = edit.show(ui);
        let mut r = out.response.response.clone();
        // The arrows keep their vertical keys while it has the keyboard.
        ui.memory_mut(|m| {
            let filter = egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                ..Default::default()
            };
            m.set_focus_lock_filter(id, filter);
        });
        if r.gained_focus() {
            // Typing replaces it: all of it selected.
            out.state
                .cursor
                .set_char_range(Some(CCursorRange::select_all_str(&text)));
            out.state.clone().store(ui.ctx(), id);
        }

        let old = value.to_f64();
        let mut note = None;
        let mut new = None;
        if r.lost_focus() {
            let esc = ui.input(|i| i.key_pressed(Key::Escape));
            let left_empty = mixed && text.trim().is_empty();
            if !esc && !left_empty {
                let range = format!("{} to {}{suffix}", written::<N>(lo), written::<N>(hi));
                match typed(&text, suffix) {
                    None => {
                        let stays = match mixed {
                            true => "nothing was changed".to_string(),
                            false => format!("it stays at {}", shown(old)),
                        };
                        note = Some(format!("{name} takes a number from {range}, so {stays}."));
                    }
                    Some(x) => {
                        let v = if N::INTEGRAL { x.round() } else { x }.clamp(lo, hi);
                        let used = shown(v);
                        if x < lo || x > hi {
                            note = Some(format!("{name} can be {range}, so {used} was used."));
                        } else if v != x {
                            note = Some(format!("{name} takes whole numbers, so {used} was used."));
                        }
                        new = Some(v);
                    }
                }
            }
            ui.data_mut(|d| d.remove::<String>(typing_id));
        } else if r.has_focus() {
            ui.data_mut(|d| d.insert_temp(typing_id, text.clone()));
        }

        // The arrows, at the box's right.
        let field = r.rect;
        let strip = Rect::from_min_max(
            egui::pos2(field.right() - ARROWS, field.top()),
            field.right_bottom(),
        );
        let (top, bottom) = strip.split_top_bottom_at_y(strip.center().y);
        let at = new.unwrap_or(old);
        let can_up = enabled && (mixed || at < hi);
        let can_down = enabled && (mixed || at > lo);
        let arrow = |ui: &mut Ui, rect: Rect, which: &str, can: bool| {
            let sense = if can { Sense::CLICK } else { Sense::hover() };
            let a = ui.interact(rect, id.with(which), sense);
            let named = match which {
                "up" => format!("Increase {name}"),
                _ => format!("Decrease {name}"),
            };
            a.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, can, &named));
            let visuals = ui.style().interact(&a);
            if a.hovered() && can {
                let fill = visuals.bg_fill;
                ui.painter().rect_filled(rect.shrink(1.0), 2.0, fill);
            }
            let colour = match can {
                true => visuals.text_color(),
                false => ui.visuals().weak_text_color(),
            };
            let glyph = match which {
                "up" => icon::CARET_UP,
                _ => icon::CARET_DOWN,
            };
            let font = egui::FontId::new(10.0, theme::icons());
            let at = Align2::CENTER_CENTER;
            ui.painter().text(rect.center(), at, glyph, font, colour);
            a.clicked()
        };
        if arrow(ui, top, "up", can_up) {
            steps += 1;
        }
        if arrow(ui, bottom, "down", can_down) {
            steps -= 1;
        }
        let edge = ui.visuals().widgets.noninteractive.bg_stroke;
        let line = strip.y_range().shrink(4.0);
        ui.painter().vline(strip.left(), line, edge);
        if steps != 0 {
            let from = if mixed && new.is_none() { 0.0 } else { at };
            let v = (from + f64::from(steps) * step).clamp(lo, hi);
            // Steps of a fraction land on two places, not 0.30000000000000004.
            let v = match N::INTEGRAL {
                true => v.round(),
                false => (v * 100.0).round() / 100.0,
            };
            new = Some(v);
            if r.has_focus() {
                let t = shown(v);
                ui.data_mut(|d| d.insert_temp(typing_id, t.clone()));
                let mut state = out.state;
                let all = CCursorRange::select_all_str(&t);
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), id);
            }
        }
        if let Some(v) = new.filter(|&v| v != old || mixed) {
            *value = N::from_f64(v);
            r.mark_changed();
        }

        if let Some(n) = note {
            let until = ui.input(|i| i.time) + NOTE_FOR;
            ui.data_mut(|d| d.insert_temp(note_id, (n, until)));
        }
        show_note(ui, note_id, field);
        if !name.is_empty() {
            crate::dialogs::set_name(&r, name);
        }
        r
    }
}

/// The note under the box `field` saying what was used for what was typed, while it lasts;
/// screen readers say it when it comes.
fn show_note(ui: &Ui, note_id: egui::Id, field: Rect) {
    let Some((note, until)) = ui.data(|d| d.get_temp::<(String, f64)>(note_id)) else {
        return;
    };
    let now = ui.input(|i| i.time);
    if now >= until {
        ui.data_mut(|d| d.remove::<(String, f64)>(note_id));
        return;
    }
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs_f64(until - now));
    egui::Area::new(note_id)
        .order(egui::Order::Tooltip)
        .fixed_pos(field.left_bottom() + Vec2::new(0.0, 4.0))
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(260.0);
                let l = ui.add(egui::Label::new(note).wrap());
                ui.ctx()
                    .accesskit_node_builder(l.id, |n| n.set_live(accesskit::Live::Polite));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Context, Event, Pos2, RawInput};

    /// A box for `value` drawn in its own window, frame by frame.
    struct Boxed<N: Numeric> {
        ctx: Context,
        value: N,
        range: RangeInclusive<N>,
        suffix: &'static str,
        step: f64,
        mixed: bool,
        /// Whether the last frame's response was marked changed.
        changed: bool,
        rect: Rect,
    }

    impl<N: Numeric> Boxed<N> {
        fn new(value: N, range: RangeInclusive<N>) -> Self {
            let ctx = Context::default();
            ctx.enable_accesskit();
            theme::install(&ctx);
            let mut b = Boxed {
                ctx,
                value,
                range,
                suffix: "",
                step: 1.0,
                mixed: false,
                changed: false,
                rect: Rect::NOTHING,
            };
            b.frame(vec![]);
            b.frame(vec![]);
            b
        }

        fn frame(&mut self, events: Vec<Event>) -> accesskit::TreeUpdate {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))),
                events,
                ..Default::default()
            };
            let (mut changed, mut rect) = (false, Rect::NOTHING);
            let Boxed {
                ctx,
                value,
                range,
                suffix,
                step,
                mixed,
                ..
            } = self;
            let mut out = ctx.run_ui(raw, |ui| {
                let spin = Spin::new(value, range.clone())
                    .suffix(suffix)
                    .step(*step)
                    .mixed(*mixed)
                    .name("Copies");
                let r = ui.add(spin);
                (changed, rect) = (r.changed(), r.rect);
                ui.label("below");
            });
            out.textures_delta.clear();
            (self.changed, self.rect) = (changed, rect);
            out.platform_output.accesskit_update.expect("the widgets")
        }

        fn click_at(&mut self, p: Pos2) {
            let press = |pressed| Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(p), press(true)]);
            self.frame(vec![press(false)]);
        }

        fn key(&mut self, key: Key) {
            let ev = |pressed| Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![ev(true), ev(false)]);
        }

        /// Clicks in the box, types `text` over what it shows, and presses Enter.
        fn type_in(&mut self, text: &str) {
            self.click_at(self.rect.left_center() + Vec2::new(8.0, 0.0));
            self.frame(vec![Event::Text(text.into())]);
            self.key(Key::Enter);
        }

        fn text(&mut self) -> String {
            let tree = self.frame(vec![]);
            let (_, n) = (tree.nodes.iter())
                .find(|(_, n)| n.label() == Some("Copies"))
                .expect("the box");
            n.value().unwrap_or_default().to_string()
        }

        fn note(&mut self) -> Option<String> {
            let tree = self.frame(vec![]);
            let live = |n: &accesskit::Node| n.live() == Some(accesskit::Live::Polite);
            let found = tree.nodes.iter().find(|(_, n)| live(n));
            found.and_then(|(_, n)| n.value().map(str::to_string))
        }
    }

    /// Typed and Enter: the value, changed once; typing replaces what was shown.
    #[test]
    fn a_number_typed_is_kept_on_enter() {
        let mut b = Boxed::new(1u32, 1..=999);
        assert_eq!(b.text(), "1");
        b.type_in("25");
        assert_eq!(b.value, 25);
        assert!(b.note().is_none());
        // Dragging across it changes nothing.
        let (from, to) = (b.rect.center(), b.rect.center() + Vec2::new(80.0, 0.0));
        b.frame(vec![
            Event::PointerMoved(from),
            Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ]);
        for i in 1..=8 {
            let p = from + (to - from) * (i as f32 / 8.0);
            b.frame(vec![Event::PointerMoved(p)]);
        }
        b.frame(vec![Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        assert_eq!(b.value, 25, "dragged");
    }

    /// Out of range: the nearest allowed, and a note saying so.
    #[test]
    fn a_number_out_of_range_is_brought_into_it_and_said() {
        let mut b = Boxed::new(5u32, 1..=999);
        b.type_in("5000");
        assert_eq!(b.value, 999);
        assert_eq!(
            b.note().as_deref(),
            Some("Copies can be 1 to 999, so 999 was used.")
        );
        let mut b = Boxed::new(5u32, 1..=999);
        b.type_in("0");
        assert_eq!(b.value, 1);
        assert_eq!(
            b.note().as_deref(),
            Some("Copies can be 1 to 999, so 1 was used.")
        );
        // A fraction where whole numbers are wanted: the nearest whole number.
        let mut b = Boxed::new(5u32, 1..=999);
        b.type_in("2.6");
        assert_eq!(b.value, 3);
        assert_eq!(
            b.note().as_deref(),
            Some("Copies takes whole numbers, so 3 was used.")
        );
        // Not a number: left as it was.
        let mut b = Boxed::new(5u32, 1..=999);
        b.type_in("many");
        assert_eq!(b.value, 5);
        assert_eq!(
            b.note().as_deref(),
            Some("Copies takes a number from 1 to 999, so it stays at 5.")
        );
    }

    /// ▲ and ▼ step it, greyed at the ends; Up and Down, Page Up and Page Down with the
    /// keyboard in it.
    #[test]
    fn the_arrows_and_keys_step_it() {
        let mut b = Boxed::new(2u32, 1..=3);
        let strip = b.rect.right() - ARROWS / 2.0;
        let (up, down) = (
            Pos2::new(strip, b.rect.top() + 4.0),
            Pos2::new(strip, b.rect.bottom() - 4.0),
        );
        b.click_at(up);
        assert_eq!(b.value, 3);
        b.click_at(up);
        assert_eq!(b.value, 3, "past the end");
        let tree = b.frame(vec![]);
        let more = (tree.nodes.iter()).find(|(_, n)| n.label() == Some("Increase Copies"));
        let greyed = more.is_some_and(|(_, n)| n.is_disabled());
        assert!(greyed, "not greyed at the end");
        b.click_at(down);
        b.click_at(down);
        assert_eq!(b.value, 1);
        // The keys, with the keyboard in the box.
        let mut b = Boxed::new(10.0f32, 0.25..=40.0);
        b.step = 0.25;
        b.suffix = " pt";
        b.click_at(b.rect.left_center() + Vec2::new(8.0, 0.0));
        b.key(Key::ArrowUp);
        assert_eq!(b.value, 10.25);
        assert!(b.changed || b.value == 10.25);
        b.key(Key::PageDown);
        assert_eq!(b.value, 7.75);
        b.key(Key::Escape);
        assert_eq!(b.text(), "7.75 pt");
    }

    /// A value outside the range, as a document's can be, is shown as it is; a box for
    /// objects that differ shows nothing until a number is typed.
    #[test]
    fn a_value_outside_the_range_or_mixed_is_shown_as_it_is() {
        let mut b = Boxed::new(0.1f32, 0.25..=40.0);
        b.suffix = " pt";
        assert_eq!(b.text(), "0.1 pt");
        assert_eq!(b.value, 0.1);
        let mut b = Boxed::new(1.0f32, 0.25..=40.0);
        b.mixed = true;
        assert_eq!(b.text(), "");
        b.type_in("3");
        assert_eq!(b.value, 3.0);
    }

    #[test]
    fn numbers_are_read_as_typed() {
        assert_eq!(typed("12", ""), Some(12.0));
        assert_eq!(typed(" 14 pt ", " pt"), Some(14.0));
        assert_eq!(typed("14PT", " pt"), Some(14.0));
        assert_eq!(typed("50%", "%"), Some(50.0));
        assert_eq!(typed("1,000", ""), Some(1000.0));
        assert_eq!(typed("12,345,678", ""), Some(12_345_678.0));
        assert_eq!(typed("0,5", ""), Some(0.5));
        assert_eq!(typed("2.75", " pt"), Some(2.75));
        assert_eq!(typed("-12", ""), Some(-12.0));
        assert_eq!(typed("walrus", ""), None);
        assert_eq!(typed("inf", ""), None);
        assert_eq!(written::<f32>(0.30000001), "0.3");
        assert_eq!(written::<f32>(12.0), "12");
        assert_eq!(written::<u32>(7.0), "7");
    }

    /// ENH-48: every number box is a spin box. egui's own, which a drag changes, isn't
    /// used anywhere else in the app, so a new box can't be one by mistake.
    #[test]
    fn every_number_box_is_a_spin_box() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        let mut dirs = vec![src];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                let rust = path.extension().is_some_and(|e| e == "rs");
                if path.is_dir() {
                    dirs.push(path);
                } else if rust && !path.ends_with("spin.rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    if text.contains(concat!("Drag", "Value")) {
                        found.push(path.display().to_string());
                    }
                }
            }
        }
        // Each named, to be made with `spin::Spin`.
        assert_eq!(found, Vec::<String>::new(), "egui's number box");
    }
}
