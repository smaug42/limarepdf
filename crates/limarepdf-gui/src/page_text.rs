//! The on-page text editor (the word-processor design, section 3): new text typed where
//! it will be on the page, in the font, size and colour it will have, with no box around
//! it, only a thin outline showing where typing goes.

use crate::{dialogs, typing_fonts};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use limarepdf_core::{winansi_byte, FontChoice};

/// The space from one line's baseline to the next, as a multiple of the size (Acrobat's).
pub const LEADING: f32 = 1.2;
/// Shown at the caret when a character typed or pasted can't be written.
pub const REFUSED: &str = "LimarePDF can't add this character yet";
/// How long that note shows, in seconds.
const NOTE_SECS: f64 = 3.0;
/// The name screen readers give the editor.
pub const NAME: &str = "Text being added";

/// How new text is set: the inspector's New text choices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub font: FontChoice,
    pub size: f32,
    pub color: [f32; 3],
}

/// New text being typed on the page (the Add text tool).
pub struct PageText {
    pub page: usize,
    /// The left end of the first line's baseline, in displayed-page points (y down).
    pub at: (f32, f32),
    pub text: String,
    /// Takes the keyboard on the next frame.
    fresh: bool,
    /// Takes the keyboard back once a change in the inspector is finished.
    pub refocus: bool,
    /// When (egui time) a character was last refused.
    refused: Option<f64>,
    /// Being moved by its outline: where it was when the drag started.
    moving: Option<(f32, f32)>,
}

/// What the editor did in a frame.
pub struct Shown {
    /// The pointer is over it, or it just opened: the page leaves the pointer alone.
    pub over: bool,
    /// Esc or Tab finished the typing, or the text went out of view.
    pub done: bool,
}

/// Whether `c` can be typed: line breaks, and what the standard fonts can write.
fn typable(c: char) -> bool {
    c == '\n' || winansi_byte(c).is_some()
}

/// `s` without the characters that can't be typed (tabs become spaces, carriage returns
/// go), and whether any was refused.
fn keep_typable(s: &str) -> (String, bool) {
    let mut refused = false;
    let kept = s
        .chars()
        .filter_map(|c| match c {
            '\r' => None,
            '\t' => Some(' '),
            c if typable(c) => Some(c),
            _ => {
                refused = true;
                None
            }
        })
        .collect();
    (kept, refused)
}

/// `text` laid out as it is drawn: one row per line, `line` pixels apart, never wrapped.
fn job(text: &str, font: &egui::FontId, color: Color32, line: f32) -> egui::text::LayoutJob {
    egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat {
            font_id: font.clone(),
            color,
            line_height: Some(line),
            ..Default::default()
        },
    )
}

impl PageText {
    /// New text starting at `at` (the baseline's left end, page points) on `page`.
    pub fn new(page: usize, at: (f32, f32)) -> Self {
        Self {
            page,
            at,
            text: String::new(),
            fresh: true,
            refocus: false,
            refused: None,
            moving: None,
        }
    }

    /// Whether there is anything to add.
    pub fn has_text(&self) -> bool {
        !self.text.trim().is_empty()
    }

    /// Draw the editor over the page drawn at `r`, `k` pixels per point, and take the
    /// keyboard. Ctrl+B and Ctrl+I switch `look`'s Bold and Italic. `accent` outlines it;
    /// `settled` says a change in the inspector is finished, so the keyboard can come back.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        r: Rect,
        k: f32,
        look: &mut Look,
        accent: Color32,
        settled: bool,
    ) -> Shown {
        let ctx = ui.ctx().clone();
        let id = egui::Id::new(("page-text", self.page));
        let focused = ctx.memory(|m| m.has_focus(id));
        if focused {
            // Before the editor sees the keys: what can't be written isn't typed.
            let (mut bold, mut italic, mut refused) = (false, false, false);
            ui.input_mut(|i| {
                bold = i.consume_key(egui::Modifiers::COMMAND, egui::Key::B);
                italic = i.consume_key(egui::Modifiers::COMMAND, egui::Key::I);
                for e in &mut i.events {
                    let t = match e {
                        egui::Event::Text(t)
                        | egui::Event::Paste(t)
                        | egui::Event::Ime(egui::ImeEvent::Commit(t)) => t,
                        _ => continue,
                    };
                    let (kept, gone) = keep_typable(t);
                    *t = kept;
                    refused |= gone;
                }
            });
            look.font.bold ^= bold;
            look.font.italic ^= italic;
            if refused {
                self.refused = Some(ctx.input(|i| i.time));
            }
        }
        let family = typing_fonts::family(&ctx, typing_fonts::Wanted::Standard(look.font));
        let px = (look.size * k).max(1.0);
        let font = egui::FontId::new(px, family);
        let [cr, cg, cb] = look
            .color
            .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        let color = Color32::from_rgb(cr, cg, cb);
        let line = LEADING * px;
        let layout = |text: &str| ctx.fonts_mut(|f| f.layout_job(job(text, &font, color, line)));
        // The first row's baseline sits on the click: its top is the font's ascent above.
        let probe = layout("M");
        let ascent = probe
            .rows
            .first()
            .and_then(|row| Some(row.pos.y + row.glyphs.first()?.pos.y))
            .unwrap_or(px * 0.9);
        let size = layout(&self.text).size();
        // Room for the caret after the last letter, and a little width when empty.
        let size = Vec2::new(size.x.max(px * 0.5) + px * 0.25, size.y.max(line));
        let origin = Pos2::new(r.min.x + self.at.0 * k, r.min.y + self.at.1 * k);
        let inner = Rect::from_min_size(origin - Vec2::new(0.0, ascent), size);
        let outline = inner.expand(3.0);
        let grip = outline.expand(3.0);
        let (grab, out) = egui::Area::new(id.with("area"))
            .fixed_pos(grip.min)
            .constrain(false)
            .order(egui::Order::Foreground)
            .show(&ctx, |ui| {
                // The caret and selection in light colours in either theme: the text sits
                // on the paper.
                *ui.visuals_mut() = egui::Visuals::light();
                // The outline around the text moves it; the text itself takes the pointer.
                ui.expand_to_include_rect(grip);
                let grab = ui.interact(grip, id.with("grip"), Sense::drag());
                ui.painter().rect_stroke(
                    outline,
                    2.0,
                    Stroke::new(1.0, accent),
                    StrokeKind::Inside,
                );
                let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _: f32| {
                    ui.ctx()
                        .fonts_mut(|f| f.layout_job(job(text.as_str(), &font, color, line)))
                };
                let te = egui::TextEdit::multiline(&mut self.text)
                    .id(id)
                    .font(font.clone())
                    .text_color(color)
                    .frame(egui::Frame::NONE)
                    .desired_rows(1)
                    .desired_width(inner.width())
                    .min_size(inner.size())
                    .clip_text(false)
                    .layouter(&mut layouter);
                let out = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| te.show(ui))
                    .inner;
                (grab, out)
            })
            .inner;
        let edit = &out.response.response;
        dialogs::set_name(edit, NAME);
        // Moved by how far the pointer is from where it was pressed.
        if grab.drag_started() {
            self.moving = Some(self.at);
        }
        let from = ctx.input(|i| i.pointer.press_origin().zip(i.pointer.latest_pos()));
        if let (Some(start), Some((a, b)), true) = (self.moving, from, grab.dragged()) {
            let d = (b - a) / k;
            self.at = (start.0 + d.x, start.1 + d.y);
        }
        if grab.drag_stopped() {
            self.moving = None;
        }
        if grab.hovered() && !edit.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::Move);
        }
        self.note(&ctx, &out, id);
        if self.fresh || (self.refocus && settled && !edit.has_focus()) {
            edit.request_focus();
            let fresh = std::mem::take(&mut self.fresh);
            self.refocus = false;
            if fresh {
                return Shown {
                    over: true,
                    done: false,
                };
            }
        }
        let (esc, tab) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Tab),
            )
        });
        let done = (esc && (edit.lost_focus() || edit.has_focus()))
            || (tab && edit.lost_focus())
            || !ui.clip_rect().intersects(outline);
        Shown {
            over: done || grab.contains_pointer() || edit.contains_pointer(),
            done,
        }
    }

    /// The note at the caret about a refused character, for a few seconds after.
    fn note(&mut self, ctx: &egui::Context, out: &egui::text_edit::TextEditOutput, id: egui::Id) {
        let Some(at) = self.refused else {
            return;
        };
        let left = NOTE_SECS - (ctx.input(|i| i.time) - at);
        if left <= 0.0 {
            self.refused = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
        let caret = out
            .cursor_range
            .map(|c| out.galley.pos_from_cursor(c.primary))
            .unwrap_or(Rect::from_min_size(Pos2::ZERO, Vec2::ZERO));
        let pos = out.galley_pos + caret.left_bottom().to_vec2() + Vec2::new(0.0, 4.0);
        egui::Area::new(id.with("refused"))
            .fixed_pos(pos)
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(REFUSED);
                });
            });
    }
}

/// The lines of `text` to write, each with how far its baseline is below the first
/// line's, in points: blank lines are left out but keep their space.
pub fn lines(text: &str, size: f32) -> Vec<(f32, &str)> {
    text.split('\n')
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| (i as f32 * LEADING * size, l.trim_end()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_can_be_written_is_typed() {
        assert_eq!(keep_typable("Walrus Š œ €"), ("Walrus Š œ €".into(), false));
        assert_eq!(
            keep_typable("Seal\r\nΩ\tfish"),
            ("Seal\n fish".into(), true)
        );
        assert_eq!(keep_typable("中文"), (String::new(), true));
    }

    #[test]
    fn lines_are_spaced_and_blank_ones_left_out() {
        assert_eq!(
            lines("Walrus  \n\n  Seal\n ", 10.0),
            [(0.0, "Walrus"), (24.0, "  Seal")]
        );
        assert!(lines(" \n", 10.0).is_empty());
    }
}
