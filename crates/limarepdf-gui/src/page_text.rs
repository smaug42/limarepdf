//! The on-page text editor (the word-processor design, section 3): text typed where it
//! will be on the page, in the font, size and colour it will have, with no box around
//! it, only a thin outline showing where typing goes. New text (Add text) and a line of
//! page text being edited share the drawing, the keys and the note about characters
//! that can't be written.

use crate::{dialogs, typing_fonts};
use eframe::egui::epaint::text::{ByteIndex, VariationCoords};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use limarepdf_core::page_colours::PageColours;
use limarepdf_core::{installed_could_write, writable, FontChoice};

/// The space from one line's baseline to the next, as a multiple of the size (Acrobat's).
pub const LEADING: f32 = 1.2;
/// Shown at the caret when a character typed or pasted can't be written.
pub const REFUSED: &str =
    "LimarePDF can't add this character yet: Latin, Greek and Cyrillic characters work";
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
    refused: Refused,
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

/// Whether `c` can be typed in `font`: line breaks, and what LimarePDF can write in it (the
/// standard font or its embedded face, or the installed family's face).
fn typable(c: char, font: FontChoice) -> bool {
    c == '\n' || writable(font, c)
}

/// `s` without the characters that can't be typed in `font` (tabs become spaces, carriage
/// returns go, and in a `single` line, line breaks become spaces), and the first one
/// refused.
fn keep_typable(s: &str, single: bool, font: FontChoice) -> (String, Option<char>) {
    let mut refused = None;
    let kept = s
        .chars()
        .filter_map(|c| match c {
            '\r' => None,
            '\t' => Some(' '),
            '\n' if single => Some(' '),
            c if typable(c, font) => Some(c),
            _ => {
                refused = refused.or(Some(c));
                None
            }
        })
        .collect();
    (kept, refused)
}

/// The note at the caret of a text editor for `c`, refused in `font`: the font is named
/// when an installed font that has `c` could write it (Chinese in Helvetica, or what an
/// installed family lacks), as choosing another font may help.
pub fn note(font: FontChoice, c: char) -> String {
    if !installed_could_write(c) {
        REFUSED.to_string()
    } else if font.is_standard() {
        format!(
            "{} can't write this character; choose an installed font that has it",
            font.family_name()
        )
    } else {
        limarepdf_core::refusal(font, &c.to_string())
    }
}

/// `f` with Bold switched when `bold` and Italic when `italic` (Ctrl+B, Ctrl+I), each only
/// to a face the family has.
pub fn switched(mut f: FontChoice, bold: bool, italic: bool) -> FontChoice {
    let has = |f: FontChoice| f.typeface().is_none_or(|t| t.has(f.bold, f.italic));
    for (on, flip) in [(bold, true), (italic, false)] {
        let mut g = f;
        if flip {
            g.bold ^= on;
        } else {
            g.italic ^= on;
        }
        if has(g) {
            f = g;
        }
    }
    f
}

/// What this frame's typing asked of an editor, taken before the editor saw it.
#[derive(Default)]
pub struct Taken {
    pub bold: bool,
    pub italic: bool,
    /// A character that can't be written was typed or pasted, and left out: the first.
    pub refused: Option<char>,
}

/// Take this frame's typing in hand before the editor with the keyboard sees it: what
/// can't be written in `font` goes (see [`keep_typable`]), and with `styles`, Ctrl+B and
/// Ctrl+I are taken to switch Bold and Italic.
pub fn take_typing(ui: &egui::Ui, single: bool, styles: bool, font: FontChoice) -> Taken {
    let mut t = Taken::default();
    ui.input_mut(|i| {
        if styles {
            t.bold = i.consume_key(egui::Modifiers::COMMAND, egui::Key::B);
            t.italic = i.consume_key(egui::Modifiers::COMMAND, egui::Key::I);
        }
        for e in &mut i.events {
            let s = match e {
                egui::Event::Text(s)
                | egui::Event::Paste(s)
                | egui::Event::Ime(egui::ImeEvent::Commit(s)) => s,
                _ => continue,
            };
            let (kept, gone) = keep_typable(s, single, font);
            *s = kept;
            t.refused = t.refused.or(gone);
        }
    });
    t
}

/// The note at the caret about a refused character, for a few seconds after: since when,
/// and what it says.
#[derive(Clone, Default)]
pub struct Refused(Option<(f64, String)>);

impl Refused {
    /// A character was refused just now; `note` says why.
    pub fn mark(&mut self, ctx: &egui::Context, note: String) {
        self.0 = Some((ctx.input(|i| i.time), note));
    }

    /// Show the note under the caret of `out` (the editor `id`) while it lasts.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        out: &egui::text_edit::TextEditOutput,
        id: egui::Id,
    ) {
        let Some((at, note)) = self.0.clone() else {
            return;
        };
        let left = NOTE_SECS - (ctx.input(|i| i.time) - at);
        if left <= 0.0 {
            self.0 = None;
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
                    ui.label(note);
                });
            });
    }
}

/// `c` as the page shows it in `mode`: text being typed matches the page around it.
pub fn shown(mode: PageColours, c: [f32; 3]) -> Color32 {
    let [r, g, b] = mode.pixel(c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
    Color32::from_rgb(r, g, b)
}

/// A stretch of text being typed in one look.
#[derive(Clone, Debug)]
pub struct Run {
    /// How many characters it has.
    pub chars: usize,
    pub font: egui::FontId,
    /// The weight of a variable font, as it will be written.
    pub coords: VariationCoords,
    pub color: Color32,
    /// How far it is raised above the baseline, in pixels.
    pub rise: f32,
}

/// How text being typed on the page is drawn this frame.
pub struct Style<'a> {
    /// The runs of the text as the editor has it, which within a frame can be ahead of
    /// the caller's copy; at least one, the look of what is typed into empty text.
    pub runs: &'a dyn Fn(&str) -> Vec<Run>,
    /// The page colour mode, for the caret and selection: light on dark paper.
    pub mode: PageColours,
    /// The outline's colour.
    pub accent: Color32,
}

/// One frame of an on-page editor.
pub struct Field {
    pub out: egui::text_edit::TextEditOutput,
    /// The band around the outline, which moves the text; `None` when it can't be moved.
    pub grip: Option<egui::Response>,
    pub outline: Rect,
}

/// `text` laid out as it is drawn in `runs` (the last one takes any characters left over):
/// never wrapped, each line 1.2 times its largest size below the one before, the runs of
/// a line on one baseline and raised ones above it. Also returns how far the first
/// baseline is below the top.
fn job(f: &mut egui::epaint::FontsView, text: &str, runs: &[Run]) -> (egui::text::LayoutJob, f32) {
    // Each line's pieces: their bytes, and their run.
    let mut lines: Vec<Vec<(std::ops::Range<usize>, usize)>> = vec![Vec::new()];
    let (mut run, mut left) = (0, runs.first().map_or(0, |r| r.chars));
    for (b, c) in text.char_indices() {
        while left == 0 && run + 1 < runs.len() {
            run += 1;
            left = runs[run].chars;
        }
        left = left.saturating_sub(1);
        let bytes = b..b + c.len_utf8();
        let line = lines.last_mut().expect("a line");
        match line.last_mut() {
            Some((r, i)) if *i == run => r.end = bytes.end,
            _ => line.push((bytes, run)),
        }
        if c == '\n' {
            lines.push(Vec::new());
        }
    }
    if let Some(line) = lines.last_mut().filter(|l| l.is_empty()) {
        line.push((text.len()..text.len(), run));
    }
    // How far each font's baseline is below the top of a row of its own.
    let mut ascents: Vec<(egui::FontId, f32)> = Vec::new();
    let mut ascent = |f: &mut egui::epaint::FontsView, font: &egui::FontId| {
        if let Some((_, a)) = ascents.iter().find(|(id, _)| id == font) {
            return *a;
        }
        let probe = f.layout_job(egui::text::LayoutJob::single_section(
            "M".into(),
            egui::TextFormat::simple(font.clone(), Color32::WHITE),
        ));
        let a = probe
            .rows
            .first()
            .and_then(|row| Some(row.pos.y + row.glyphs.first()?.pos.y))
            .unwrap_or(font.size * 0.9);
        ascents.push((font.clone(), a));
        a
    };
    let mut job = egui::text::LayoutJob {
        text: text.to_string(),
        ..Default::default()
    };
    let mut first = None;
    for line in lines {
        let fallback = Run {
            chars: 0,
            font: egui::FontId::proportional(12.0),
            coords: VariationCoords::default(),
            color: Color32::BLACK,
            rise: 0.0,
        };
        let run_of = |i: usize| runs.get(i).unwrap_or(&fallback);
        let height = LEADING
            * line
                .iter()
                .map(|(_, i)| run_of(*i).font.size)
                .fold(0.0, f32::max);
        let top = line
            .iter()
            .map(|(_, i)| ascent(f, &run_of(*i).font))
            .fold(0.0, f32::max);
        first.get_or_insert(top);
        for (bytes, i) in line {
            let r = run_of(i);
            // egui puts a run's baseline its ascent plus the row's height less its own
            // line height below the row's top, so this height puts it `top - rise` down.
            let own = (ascent(f, &r.font) + height - top + r.rise).clamp(0.0, height);
            job.sections.push(egui::text::LayoutSection {
                leading_space: 0.0,
                byte_range: ByteIndex(bytes.start)..ByteIndex(bytes.end),
                format: egui::TextFormat {
                    font_id: r.font.clone(),
                    coords: r.coords.clone(),
                    color: r.color,
                    line_height: Some(own),
                    ..Default::default()
                },
            });
        }
    }
    (job, first.unwrap_or(0.0))
}

/// Draw `text` for typing (the editor `id`) with its first baseline's left end at
/// `origin` on screen: one line, or several 1.2 times the size apart. `cover`, a
/// rectangle on screen and its colour, is painted under it first: the line being edited,
/// hidden so it doesn't show twice. A `movable` editor has a grip around its outline.
#[allow(clippy::too_many_arguments)]
pub fn field(
    ctx: &egui::Context,
    id: egui::Id,
    text: &mut String,
    origin: Pos2,
    style: &Style,
    multiline: bool,
    cover: Option<(Rect, Color32)>,
    movable: bool,
) -> Field {
    let runs = (style.runs)(text);
    let first = runs.first().cloned();
    let font = first
        .as_ref()
        .map_or_else(|| egui::FontId::proportional(12.0), |r| r.font.clone());
    let color = first.map_or(Color32::BLACK, |r| r.color);
    let px = runs.iter().map(|r| r.font.size).fold(1.0, f32::max);
    let line = LEADING * px;
    let laid = |text: &str| {
        let runs = (style.runs)(text);
        ctx.fonts_mut(|f| {
            let (j, ascent) = job(f, text, &runs);
            (f.layout_job(j), ascent)
        })
    };
    // The first row's baseline sits on the origin.
    let (galley, ascent) = laid(text);
    let size = galley.size();
    // Room for the caret after the last letter, and a little width when empty.
    let size = Vec2::new(size.x.max(px * 0.5) + px * 0.25, size.y.max(line));
    let inner = Rect::from_min_size(origin - Vec2::new(0.0, ascent), size);
    let outline = inner.expand(3.0);
    let grip = outline.expand(3.0);
    let top_left = cover.map_or(grip.min, |(c, _)| grip.min.min(c.min));
    let (grab, out) = egui::Area::new(id.with("area"))
        .fixed_pos(top_left)
        .constrain(false)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            // The caret and selection as on paper of this colour: dark on light paper,
            // light on dark.
            *ui.visuals_mut() = match style.mode {
                PageColours::Dark | PageColours::HighContrast => egui::Visuals::dark(),
                PageColours::Normal | PageColours::Sepia => egui::Visuals::light(),
            };
            ui.expand_to_include_rect(grip);
            if let Some((c, fill)) = cover {
                ui.expand_to_include_rect(c);
                ui.painter().rect_filled(c, 0.0, fill);
            }
            // The outline around the text moves it; the text itself takes the pointer.
            let grab = movable.then(|| {
                let r = ui.interact(grip, id.with("grip"), Sense::drag());
                dialogs::set_name(&r, "Move the text");
                r
            });
            ui.painter().rect_stroke(
                outline,
                2.0,
                Stroke::new(1.0, style.accent),
                StrokeKind::Inside,
            );
            let mut layouter =
                |_: &egui::Ui, text: &dyn egui::TextBuffer, _: f32| laid(text.as_str()).0;
            let te = if multiline {
                egui::TextEdit::multiline(text)
            } else {
                egui::TextEdit::singleline(text)
            };
            let te = te
                .id(id)
                .font(font.clone())
                .text_color(color)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
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
    Field {
        out,
        grip: grab,
        outline,
    }
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
            refused: Refused::default(),
            moving: None,
        }
    }

    /// Whether there is anything to add.
    pub fn has_text(&self) -> bool {
        !self.text.trim().is_empty()
    }

    /// Draw the editor over the page drawn at `r`, `k` pixels per point, in the page
    /// colour `mode`, and take the keyboard. Ctrl+B and Ctrl+I switch `look`'s Bold and
    /// Italic. `accent` outlines it; `settled` says a change in the inspector is finished,
    /// so the keyboard can come back.
    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        r: Rect,
        k: f32,
        look: &mut Look,
        mode: PageColours,
        accent: Color32,
        settled: bool,
    ) -> Shown {
        let ctx = ui.ctx().clone();
        let id = egui::Id::new(("page-text", self.page));
        if ctx.memory(|m| m.has_focus(id)) {
            // Before the editor sees the keys: what can't be written isn't typed.
            let t = take_typing(ui, false, true, look.font);
            look.font = switched(look.font, t.bold, t.italic);
            if let Some(c) = t.refused {
                self.refused.mark(&ctx, note(look.font, c));
            }
        }
        let wanted = typing_fonts::Wanted::Standard(look.font);
        let run = Run {
            chars: 0,
            font: egui::FontId::new((look.size * k).max(1.0), typing_fonts::family(&ctx, wanted)),
            coords: typing_fonts::coords(wanted),
            color: shown(mode, look.color),
            rise: 0.0,
        };
        let runs = |t: &str| {
            vec![Run {
                chars: t.chars().count(),
                ..run.clone()
            }]
        };
        let style = Style {
            runs: &runs,
            mode,
            accent,
        };
        let origin = Pos2::new(r.min.x + self.at.0 * k, r.min.y + self.at.1 * k);
        let f = field(&ctx, id, &mut self.text, origin, &style, true, None, true);
        let edit = &f.out.response.response;
        dialogs::set_name(edit, NAME);
        let mut on_grip = false;
        if let Some(grab) = &f.grip {
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
            on_grip = grab.contains_pointer();
        }
        self.refused.show(&ctx, &f.out, id);
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
            || !ui.clip_rect().intersects(f.outline);
        Shown {
            over: done || on_grip || edit.contains_pointer(),
            done,
        }
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

    /// A line in parts is drawn as it will be written: its parts on one baseline whatever
    /// their sizes, raised ones above it, and each line 1.2 times its largest size below
    /// the one before.
    #[test]
    fn parts_of_a_line_share_its_baseline() {
        let ctx = egui::Context::default();
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        let run = |chars, size, rise| Run {
            chars,
            font: egui::FontId::proportional(size),
            coords: VariationCoords::default(),
            color: Color32::BLACK,
            rise,
        };
        let runs = [
            run(2, 12.0, 0.0),
            run(2, 30.0, 0.0),
            run(3, 12.0, 6.0),
            run(2, 10.0, 0.0),
        ];
        let (galley, top) = ctx.fonts_mut(|f| {
            let (j, top) = job(f, "abcdef\ngh", &runs);
            (f.layout_job(j), top)
        });
        let row = &galley.rows[0];
        let base: Vec<f32> = row.glyphs.iter().map(|g| row.pos.y + g.pos.y).collect();
        let near = |a: f32, b: f32| (a - b).abs() <= 1.0;
        assert!(near(base[0], top) && near(base[2], top), "{base:?} {top}");
        assert!(near(base[4], top - 6.0), "{base:?} {top}");
        // The first line is as high as its 30-pixel text needs; the second as its 10.
        assert!(near(galley.rows[1].pos.y, 36.0), "{:?}", galley.rows[1].pos);
        assert!(near(galley.size().y, 48.0), "{:?}", galley.size());
    }

    #[test]
    fn only_what_can_be_written_is_typed() {
        let sans = FontChoice::default();
        assert_eq!(
            keep_typable("Walrus Š œ €", false, sans),
            ("Walrus Š œ €".into(), None)
        );
        // Latin, Greek and Cyrillic beyond WinAnsi, in the embedded face.
        assert_eq!(
            keep_typable("Łódź Ωmega Жук", false, sans),
            ("Łódź Ωmega Жук".into(), None)
        );
        assert_eq!(
            keep_typable("Seal\r\n中\tfish", false, sans),
            ("Seal\n fish".into(), Some('中'))
        );
        assert_eq!(
            keep_typable("中文", false, sans),
            (String::new(), Some('中'))
        );
        // Right-to-left scripts aren't written yet, though the face has Hebrew.
        assert_eq!(
            keep_typable("שלום", false, sans),
            (String::new(), Some('ש'))
        );
        // In a single line, line breaks become spaces.
        assert_eq!(
            keep_typable("Seal\r\nfish", true, sans),
            ("Seal fish".into(), None)
        );
        // The note points to installed fonts for what one could write, not for
        // right-to-left or shaped scripts, emoji or control characters.
        assert_eq!(
            note(sans, '中'),
            "Helvetica can't write this character; choose an installed font that has it"
        );
        let times = FontChoice::from_name("Times-Bold");
        assert_eq!(
            note(times, '한'),
            "Times can't write this character; choose an installed font that has it"
        );
        for c in ['ש', 'क', '😀', '\u{7}'] {
            assert_eq!(note(sans, c), REFUSED, "{c:?}");
        }
    }

    fn installed(name: &str) -> FontChoice {
        FontChoice {
            family: crate::font_list::test_family(name),
            ..Default::default()
        }
    }

    #[test]
    fn installed_fonts_type_what_they_have() {
        let caveat = installed("Caveat");
        assert_eq!(
            keep_typable("Жук Ωmega", false, caveat),
            ("Жук mega".into(), Some('Ω'))
        );
        assert_eq!(
            note(caveat, 'Ω'),
            "Caveat has no \"Ω\"; choose another font"
        );
        // Ctrl+B and Ctrl+I switch only to faces the family has.
        let bold = switched(caveat, true, false);
        assert!(bold.bold && !bold.italic);
        assert_eq!(switched(caveat, false, true), caveat, "no italic");
        let serif = installed("Source Serif 4");
        assert_eq!(switched(serif, true, true), serif);
        let helvetica = FontChoice::default();
        let both = switched(helvetica, true, true);
        assert!(both.bold && both.italic);
    }

    #[test]
    fn typed_text_shows_in_the_page_colours() {
        let red = [1.0, 0.0, 0.0];
        assert_eq!(
            shown(PageColours::Normal, red),
            Color32::from_rgb(255, 0, 0)
        );
        for mode in [
            PageColours::Dark,
            PageColours::Sepia,
            PageColours::HighContrast,
        ] {
            let [r, g, b] = mode.pixel([0, 0, 0]);
            assert_eq!(
                shown(mode, [0.0; 3]),
                Color32::from_rgb(r, g, b),
                "{mode:?}"
            );
        }
        // Black text on dark paper is light.
        assert!(shown(PageColours::Dark, [0.0; 3]).r() > 200);
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
