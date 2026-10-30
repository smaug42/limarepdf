//! The on-page text editor (the word-processor design, section 3): text typed where it
//! will be on the page, in the font, size and colour it will have, with no box around
//! it, only a thin outline showing where typing goes. New text (Add text) and a line of
//! page text being edited share the drawing, the keys and the note about characters
//! that can't be written.

use crate::dialogs;
use crate::styled::{CharStyle, History, Styled};
use crate::theme;
use crate::typing_fonts::{self, Wanted};
use eframe::egui::epaint::text::{ByteIndex, VariationCoords};
use eframe::egui::{self, Color32, Galley, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use limarepdf_core::page_colours::PageColours;
use limarepdf_core::{emoji_sequence, unwritten_script, writable, FontChoice, TextPart};
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

/// The space from one line's baseline to the next, as a multiple of the size (Acrobat's).
pub const LEADING: f32 = 1.2;

/// How far a line's baseline is below the one before, by each one's largest size: the
/// line above's share below its baseline and the line's own above it, as word processors
/// space lines of different sizes; 1.2 times the size when they are the same.
pub fn line_gap(above: f32, below: f32) -> f32 {
    LEADING * (0.25 * above + 0.75 * below)
}
/// Shown at the caret when a character typed or pasted can't be written: no installed font
/// has it.
pub const REFUSED: &str = "No font on this computer has this character";
/// Shown at the caret when an emoji made of several characters is typed or pasted
/// ([`limarepdf_core::emoji_sequence`]).
pub const SEQUENCE: &str = "LimarePDF can't add this emoji yet: flags, skin tones, keycaps and \
                            joined emoji aren't supported";
/// Shown at the caret when a character of a script written right to left, or one whose
/// letters join or change shape, is typed or pasted.
pub const SCRIPT: &str = "LimarePDF can't add this script yet: right-to-left writing, and \
                          scripts whose letters join, such as Arabic, Hebrew and Hindi, aren't \
                          supported";
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

impl Look {
    /// The look of text in `s`.
    pub fn of(s: CharStyle) -> Self {
        Self {
            font: s.font,
            size: s.size,
            color: s.color,
        }
    }

    /// Text typed in this look.
    pub fn style(self) -> CharStyle {
        CharStyle {
            font: self.font,
            span: None,
            size: self.size,
            color: self.color,
            rise: 0.0,
        }
    }
}

/// New text being typed on the page (the Add text tool).
pub struct PageText {
    /// The tab's editor ids ([`crate::editor::Editor::scope`]).
    scope: egui::Id,
    pub page: usize,
    /// The left end of the first line's baseline, in displayed-page points (y down).
    pub at: (f32, f32),
    /// What is typed, a style for each character.
    pub text: Styled,
    history: History,
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
    /// The text's box as drawn, for its right-click menu.
    pub edit: egui::Response,
}

/// Whether `c` can be typed in `font`: line breaks, and what LimarePDF can write in it (the
/// standard font or its embedded face, or the installed family's face).
fn typable(c: char, font: FontChoice) -> bool {
    c == '\n' || writable(font, c)
}

/// `s` without the characters that can't be typed in `font` (tabs become spaces, carriage
/// returns go, and in a `single` line, line breaks become spaces), and the first one
/// refused. Each character as readers see it (a grapheme cluster) is kept or left out
/// whole, so an emoji sequence goes whole rather than leave a part that says something
/// else (a thumb without its skin tone).
fn keep_typable(s: &str, single: bool, font: FontChoice) -> (String, Option<String>) {
    let s: String = s
        .chars()
        .filter_map(|c| match c {
            '\r' => None,
            '\t' => Some(' '),
            '\n' if single => Some(' '),
            c => Some(c),
        })
        .collect();
    let (mut kept, mut refused) = (String::new(), None);
    for cluster in s.graphemes(true) {
        if !emoji_sequence(cluster) && cluster.chars().all(|c| typable(c, font)) {
            kept.push_str(cluster);
        } else {
            refused = refused.or_else(|| Some(cluster.to_string()));
        }
    }
    (kept, refused)
}

/// The note at the caret of a text editor for `refused`, a character (grapheme cluster) no
/// font can write: the typing's font, and every installed font it falls back to, lack it
/// ([`REFUSED`]), or it is an emoji sequence ([`SEQUENCE`]) or of a script no font is
/// written in yet ([`SCRIPT`]), which no font helps with.
pub fn note(refused: &str) -> &'static str {
    if emoji_sequence(refused) {
        SEQUENCE
    } else if refused.chars().any(unwritten_script) {
        SCRIPT
    } else {
        REFUSED
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

/// Bold or Italic switched for `sel` in `text`, as Ctrl+B or Ctrl+I asked (`t`); when the
/// family (`font`, the typing's) has no such face, nothing changes, and `refused` says so
/// at the caret.
pub fn switch_styles(
    ctx: &egui::Context,
    t: &Taken,
    text: &mut Styled,
    sel: (usize, usize),
    font: FontChoice,
    refused: &mut Refused,
) {
    for (asked, bold, face) in [(t.bold, true, "bold"), (t.italic, false, "italic")] {
        if !asked {
            continue;
        }
        if !text.switches(sel, bold) {
            refused.mark(ctx, format!("{} has no {face}.", font.family_name()));
        } else if bold {
            text.bold(sel);
        } else {
            text.italic(sel);
        }
    }
}

/// What this frame's typing asked of an editor, taken before the editor saw it.
#[derive(Default)]
pub struct Taken {
    pub bold: bool,
    pub italic: bool,
    /// Ctrl+Z.
    pub undo: bool,
    /// Ctrl+Y or Ctrl+Shift+Z.
    pub redo: bool,
    /// A character that can't be written was typed or pasted, and left out: the first (a
    /// grapheme cluster).
    pub refused: Option<String>,
}

/// Take this frame's typing in hand before the editor with the keyboard sees it: what
/// can't be written in `font` goes (see [`keep_typable`]); Ctrl+Z and Ctrl+Y are taken for
/// the caller's [`History`], as the editor's own undo would take back the text without
/// its styles; and with `styles`, Ctrl+B and Ctrl+I are taken to switch Bold and Italic.
pub fn take_typing(ui: &egui::Ui, single: bool, styles: bool, font: FontChoice) -> Taken {
    let mut t = Taken::default();
    ui.input_mut(|i| {
        let shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        t.redo = i.consume_key(shift, egui::Key::Z)
            | i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y);
        t.undo = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z);
        if styles {
            t.bold = i.consume_key(egui::Modifiers::COMMAND, egui::Key::B);
            t.italic = i.consume_key(egui::Modifiers::COMMAND, egui::Key::I);
        }
    });
    t.refused = leave_out_untypable(ui, single, font);
    t
}

/// Leave what can't be written in `font` out of this frame's typing, before the editor
/// with the keyboard sees it (see [`keep_typable`]). Returns the first character left out
/// (a grapheme cluster). The editor keeps its own Ctrl+Z, for plain text with no styles.
pub fn leave_out_untypable(ui: &egui::Ui, single: bool, font: FontChoice) -> Option<String> {
    let mut refused = None;
    ui.input_mut(|i| {
        for e in &mut i.events {
            let s = match e {
                egui::Event::Text(s)
                | egui::Event::Paste(s)
                | egui::Event::MiddleClickPaste { text: s, .. }
                | egui::Event::Ime(egui::ImeEvent::Commit(s)) => s,
                _ => continue,
            };
            let (kept, gone) = keep_typable(s, single, font);
            *s = kept;
            refused = refused.take().or(gone);
        }
    });
    refused
}

/// Take `text`, typed in the editor `id`, back or forward a step of `history` when `t`
/// took Ctrl+Z or Ctrl+Y, selecting what was selected then.
pub fn undo_redo(
    ctx: &egui::Context,
    id: egui::Id,
    t: &Taken,
    history: &mut History,
    text: &mut Styled,
) {
    let sel = if t.undo {
        history.undo(text)
    } else if t.redo {
        history.redo(text)
    } else {
        None
    };
    if let Some(sel) = sel {
        select(ctx, id, sel);
    }
}

/// What is selected in the text editor `id`: the characters from the anchor to the caret.
pub fn selection(ctx: &egui::Context, id: egui::Id) -> Option<(usize, usize)> {
    let r = egui::TextEdit::load_state(ctx, id)?.cursor.char_range()?;
    Some((r.secondary.index.0, r.primary.index.0))
}

/// Where what is selected in a text box is noted while it has the keyboard ([`held`]).
const HELD: &str = "held-selection";

/// What the inspector's style controls change in the text box `id`: what is selected
/// there. egui keeps only the caret of a box without the keyboard, so while a list, the
/// size box or the colour picker has it, this is what was selected when the box last had
/// it; [`give_back`] selects that again.
pub fn held(ctx: &egui::Context, id: egui::Id) -> Option<(usize, usize)> {
    let note = id.with(HELD);
    if ctx.memory(|m| m.has_focus(id)) {
        let sel = selection(ctx, id);
        ctx.data_mut(|d| match sel {
            Some(s) => {
                d.insert_temp(note, s);
            }
            None => d.remove::<(usize, usize)>(note),
        });
        sel
    } else {
        ctx.data(|d| d.get_temp(note))
            .or_else(|| selection(ctx, id))
    }
}

/// Give the text box `id` the keyboard back with what was selected in it ([`held`]).
pub fn give_back(ctx: &egui::Context, id: egui::Id) {
    if let Some(sel) = ctx.data(|d| d.get_temp(id.with(HELD))) {
        select(ctx, id, sel);
    }
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Select the characters `sel` (from the anchor to the caret) in the text editor `id`.
pub fn select(ctx: &egui::Context, id: egui::Id, sel: (usize, usize)) {
    ctx.data_mut(|d| d.insert_temp(id.with(HELD), sel));
    if let Some(mut st) = egui::TextEdit::load_state(ctx, id) {
        let range = egui::text::CCursorRange::two(
            egui::text::CCursor::new(sel.0),
            egui::text::CCursor::new(sel.1),
        );
        st.cursor.set_char_range(Some(range));
        st.store(ctx, id);
    }
}

/// A right-click on what is selected in the text box `id` leaves it selected, for the menu
/// to act on, as in word processors (egui moves the caret for a press of any button); a
/// right-click elsewhere puts the caret there. `held` is what was selected before the box
/// was drawn this frame.
pub fn keep_on_right_click(
    ctx: &egui::Context,
    id: egui::Id,
    out: &mut egui::text_edit::TextEditOutput,
    held: Option<egui::text::CCursorRange>,
) {
    let Some(held) = held.filter(|r| !r.is_empty()) else {
        return;
    };
    let right = ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary));
    let at = ctx.input(|i| i.pointer.interact_pos());
    let (true, Some(at)) = (right && out.response.response.hovered(), at) else {
        return;
    };
    let on = out.galley.cursor_from_pos(at - out.galley_pos).index.0;
    let [from, to] = held.sorted_cursors();
    if (from.index.0..=to.index.0).contains(&on) {
        out.state.cursor.set_char_range(Some(held));
        out.state.clone().store(ctx, id);
        out.cursor_range = Some(held);
    }
}

/// Where the text box given the keyboard back by [`hand_back`] is noted, for a frame.
const HANDED_BACK: &str = "typing-handed-back";

/// Give the text box `id` the keyboard back in the middle of its typing (after a change
/// in the inspector, say), keeping what Ctrl+Z can take back there.
pub fn hand_back(ctx: &egui::Context, id: egui::Id) {
    ctx.memory_mut(|m| m.request_focus(id));
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(HANDED_BACK), id));
}

/// Start the typing history of a text box that has just got the keyboard afresh, unless
/// it was handed back ([`hand_back`]) or `kept` says it comes back to it with nothing kept
/// in the document since it was left (ENH-32); `before` had the keyboard last frame.
/// Returns what has it now. What was typed in a box before was kept in the document when
/// the box was left, where Undo takes it back, so Ctrl+Z in a box never brings back text
/// typed in another document, field or comment, or text the document has since undone.
pub fn fresh_history(
    ctx: &egui::Context,
    before: Option<egui::Id>,
    kept: impl Fn(egui::Id) -> bool,
) -> Option<egui::Id> {
    let now = ctx.memory(|m| m.focused());
    let note = egui::Id::unique(HANDED_BACK);
    let handed = ctx.data_mut(|d| {
        let id = d.get_temp::<egui::Id>(note);
        d.remove::<egui::Id>(note);
        id
    });
    if let Some(id) = now.filter(|&id| now != before && handed != Some(id) && !kept(id)) {
        if let Some(mut st) = egui::TextEdit::load_state(ctx, id) {
            st.clear_undoer();
            st.store(ctx, id);
        }
    }
    now
}

/// Whether Ctrl+Z and Ctrl+Y would take back or redo something in the text box `id`, which
/// keeps egui's own history (a form field, a note, the inspector's boxes): the band's Undo
/// and Redo act on its typing as the keys do. `None` when `id` isn't a text box.
pub fn box_history(ctx: &egui::Context, id: egui::Id) -> Option<(bool, bool)> {
    let undoer = egui::TextEdit::load_state(ctx, id)?.undoer();
    // A state no box is in: undoing from it gives the latest step kept, unchanged.
    let none = (egui::text::CCursorRange::default(), "\u{FFFF}".to_string());
    let Some(last) = undoer.clone().undo(&none).cloned() else {
        return Some((false, false));
    };
    // Changed since that step, or with a step before it.
    let changed = undoer.is_in_flux();
    let undo = changed || undoer.has_undo(&last);
    let redo = !changed && undoer.clone().redo(&last).is_some();
    Some((undo, redo))
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
        if self.0.is_none() {
            return;
        }
        let caret = out
            .cursor_range
            .map(|c| out.galley.pos_from_cursor(c.primary))
            .unwrap_or(Rect::from_min_size(Pos2::ZERO, Vec2::ZERO));
        let pos = out.galley_pos + caret.left_bottom().to_vec2() + Vec2::new(0.0, 4.0);
        self.show_at(ctx, pos, id);
    }

    /// Show the note with its top left at `pos` on the screen while it lasts, as under a
    /// caret; `id` names it.
    pub fn show_at(&mut self, ctx: &egui::Context, pos: Pos2, id: egui::Id) {
        let Some((at, note)) = self.0.clone() else {
            return;
        };
        let left = NOTE_SECS - (ctx.input(|i| i.time) - at);
        if left <= 0.0 {
            self.0 = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
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

/// The runs `parts` are drawn in at `k` pixels per point: each part in the font it will be
/// written in, its colour as `color` shows it.
pub fn runs(
    ctx: &egui::Context,
    parts: &[(TextPart, Wanted)],
    k: f32,
    color: impl Fn([f32; 3]) -> Color32,
) -> Vec<Run> {
    parts
        .iter()
        .map(|(p, wanted)| {
            let wanted = *wanted;
            Run {
                chars: p.text.chars().count(),
                font: egui::FontId::new((p.size * k).max(1.0), typing_fonts::family(ctx, wanted)),
                coords: typing_fonts::coords(wanted),
                color: color(p.color),
                rise: p.rise * k,
            }
        })
        .collect()
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
    /// Why the text, as it is laid out (on screen, without the room for the caret), runs
    /// over: past the page's edge or over other text. `None` when it fits.
    pub warn: &'a dyn Fn(Rect) -> Option<&'static str>,
}

/// One frame of an on-page editor.
pub struct Field {
    pub out: egui::text_edit::TextEditOutput,
    /// The band around the outline, which moves the text; `None` when it can't be moved.
    pub grip: Option<egui::Response>,
    pub outline: Rect,
}

/// `text` laid out as [`job`] lays it out in `runs`, and how far its first baseline is
/// below the top.
pub fn galley(ctx: &egui::Context, text: &str, runs: &[Run]) -> (Arc<Galley>, f32) {
    ctx.fonts_mut(|f| {
        let (j, ascent) = job(f, text, runs);
        (f.layout_job(j), ascent)
    })
}

/// `text` laid out as it is drawn in `runs` (the last one takes any characters left over):
/// never wrapped, each line's baseline [`line_gap`] below the one before, the runs of a
/// line on one baseline and raised ones above it. Also returns how far the first baseline
/// is below the top.
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
    // How far each font's baseline is below the top of a row of its own, and the font's
    // height (egui's height for such a row, before rounding).
    let mut metrics: Vec<(egui::FontId, (f32, f32))> = Vec::new();
    let mut metric = |f: &mut egui::epaint::FontsView, font: &egui::FontId| {
        if let Some((_, m)) = metrics.iter().find(|(id, _)| id == font) {
            return *m;
        }
        let probe = f.layout_job(egui::text::LayoutJob::single_section(
            "M".into(),
            egui::TextFormat::simple(font.clone(), Color32::WHITE),
        ));
        let m = probe
            .rows
            .first()
            .and_then(|row| {
                let g = row.glyphs.first()?;
                Some((row.pos.y + g.pos.y, g.font_height))
            })
            .unwrap_or((font.size * 0.9, font.size * LEADING));
        metrics.push((font.clone(), m));
        m
    };
    let mut job = egui::text::LayoutJob {
        text: text.to_string(),
        ..Default::default()
    };
    let fallback = Run {
        chars: 0,
        font: egui::FontId::proportional(12.0),
        coords: VariationCoords::default(),
        color: Color32::BLACK,
        rise: 0.0,
    };
    let run_of = |i: usize| runs.get(i).unwrap_or(&fallback);
    // Each line's largest size and the run of its first part in that size.
    let biggest: Vec<(f32, usize)> = lines
        .iter()
        .map(|line| {
            line.iter().fold((0.0, 0), |big, &(_, i)| {
                let size = run_of(i).font.size;
                if size > big.0 {
                    (size, i)
                } else {
                    big
                }
            })
        })
        .collect();
    // egui puts a part's baseline its ascent below the row's top, plus its `valign`'s share
    // (top none, bottom all) of the row's height less the part's line height, plus half
    // its line height less its font's: a part as high as the row is centred in it. So the
    // line's biggest part, as high as the row, sets the baseline, and the rows are made as
    // high as put each baseline its gap below the one before; the last 1.2 times its size.
    // egui rounds the heights, so they are rounded here, and each baseline follows the
    // rounded heights below it. Each row's height, and its baseline below its top:
    let mut rows = vec![(0.0, 0.0); lines.len()];
    let mut next: Option<(f32, f32)> = None;
    for j in (0..lines.len()).rev() {
        let (size, k) = biggest[j];
        let r = run_of(k);
        let (ascent, own) = metric(f, &r.font);
        let height = match next {
            Some((below, base)) => 2.0 * (line_gap(size, below) + ascent + r.rise - base) - own,
            None => LEADING * size,
        }
        .round()
        .max(1.0);
        let base = ascent + r.rise + 0.5 * (height - own);
        rows[j] = (height, base);
        next = Some((size, base));
    }
    for (j, line) in lines.into_iter().enumerate() {
        let ((height, base), k) = (rows[j], biggest[j].1);
        for (bytes, i) in line {
            let r = run_of(i);
            let (ascent, own) = metric(f, &r.font);
            // How far below its place in a row of its own the part goes: `base - rise`.
            let down = base - r.rise - ascent;
            let (valign, line_height) = if i == k {
                (egui::Align::BOTTOM, height)
            } else if down <= 0.5 * (height - own) {
                (egui::Align::TOP, own + 2.0 * down)
            } else {
                (egui::Align::BOTTOM, 2.0 * (height - down) - own)
            };
            job.sections.push(egui::text::LayoutSection {
                leading_space: 0.0,
                byte_range: ByteIndex(bytes.start)..ByteIndex(bytes.end),
                format: egui::TextFormat {
                    font_id: r.font.clone(),
                    coords: r.coords.clone(),
                    color: r.color,
                    line_height: Some(line_height.clamp(0.0, height)),
                    valign,
                    ..Default::default()
                },
            });
        }
    }
    (job, rows.first().map_or(0.0, |r| r.1))
}

/// Text laid out at `text` on screen runs past the edge of the page drawn at `page`: a
/// point's leeway for rounding.
pub fn past_edge(page: Rect, text: Rect) -> Option<&'static str> {
    (!page.expand(1.0).contains_rect(text)).then_some("Past the page edge")
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
    let laid = |text: &str| galley(ctx, text, &(style.runs)(text));
    // The first row's baseline sits on the origin.
    let (galley, ascent) = laid(text);
    let size = galley.size();
    // Room for the caret after the last letter, and a little width when empty.
    let size = Vec2::new(size.x.max(px * 0.5) + px * 0.25, size.y.max(line));
    let inner = Rect::from_min_size(origin - Vec2::new(0.0, ascent), size);
    // What the text runs over, where it is laid out.
    let warning = (style.warn)(Rect::from_min_size(inner.min, galley.size()));
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
            // The outline around the text moves it; the text itself takes the pointer. Not a
            // stop for the keyboard: Esc keeps the text, selected, and the arrow keys move
            // it, as in word processors.
            let grab = movable.then(|| {
                let r = ui.interact(grip, id.with("grip"), Sense::DRAG);
                dialogs::set_name(&r, "Move the text");
                r
            });
            // Two lines (ENH-22): the accent, or the caution colour when the text runs over,
            // and inside it one that stands out against it, so the outline shows on paper
            // of either colour.
            let edge = match warning {
                Some(_) => theme::tone(ui.visuals().dark_mode, theme::Tone::Caution),
                None => None,
            }
            .unwrap_or(style.accent);
            let halo = Stroke::new(1.0, theme::halo(edge));
            let painter = ui.painter();
            painter.rect_stroke(outline, 2.0, Stroke::new(1.0, edge), StrokeKind::Inside);
            painter.rect_stroke(outline.shrink(1.0), 1.0, halo, StrokeKind::Inside);
            // What it runs over, on a tag just below the outline's right end.
            if let Some(why) = warning {
                let text = egui::RichText::new(why).size(11.0).color(theme::halo(edge));
                let g = painter.layout_no_wrap(why.into(), egui::FontId::proportional(11.0), edge);
                let size = g.size() + Vec2::new(8.0, 4.0);
                let at = Pos2::new(outline.max.x - size.x, outline.max.y + 2.0);
                let tag = Rect::from_min_size(at, size);
                ui.expand_to_include_rect(tag);
                ui.painter().rect_filled(tag, 3.0, edge);
                ui.put(tag, egui::Label::new(text).selectable(false));
            }
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
            let held = egui::TextEdit::load_state(ctx, id).and_then(|s| s.cursor.char_range());
            let mut out = ui
                .scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| te.show(ui))
                .inner;
            keep_on_right_click(ctx, id, &mut out, held);
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
    /// New text starting at `at` (the baseline's left end, page points) on `page`, typed
    /// in `look`.
    pub fn new(scope: egui::Id, page: usize, at: (f32, f32), look: Look) -> Self {
        let text = Styled::plain("", look.style());
        Self {
            scope,
            page,
            at,
            history: History::new(&text),
            text,
            fresh: true,
            refocus: false,
            refused: Refused::default(),
            moving: None,
        }
    }

    /// The editor's id.
    pub fn id(&self) -> egui::Id {
        self.scope.with(("page-text", self.page))
    }

    /// Whether it is being moved by its outline.
    pub fn moving(&self) -> bool {
        self.moving.is_some()
    }

    /// The typing's own history, which Ctrl+Z and Ctrl+Y take back and forward.
    pub fn history(&self) -> &History {
        &self.history
    }

    /// Whether there is anything to add.
    pub fn has_text(&self) -> bool {
        !self.text.text().trim().is_empty()
    }

    /// Draw the editor over the page drawn at `r`, `k` pixels per point, in the page
    /// colour `mode`, and take the keyboard. Ctrl+B and Ctrl+I switch Bold and Italic as
    /// the inspector's buttons do, and Ctrl+Z and Ctrl+Y undo and redo typing and styles.
    /// `accent` outlines it; `settled` says a change in the inspector is finished, so the
    /// keyboard can come back.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        r: Rect,
        k: f32,
        mode: PageColours,
        accent: Color32,
        settled: bool,
    ) -> Shown {
        let ctx = ui.ctx().clone();
        let id = self.id();
        if ctx.memory(|m| m.has_focus(id)) {
            // Before the editor sees the keys: what can't be written isn't typed.
            let sel = selection(&ctx, id).unwrap_or((0, 0));
            let font = self.text.typing_font(sel);
            let t = take_typing(ui, false, true, font);
            undo_redo(&ctx, id, &t, &mut self.history, &mut self.text);
            switch_styles(&ctx, &t, &mut self.text, sel, font, &mut self.refused);
            if let Some(c) = t.refused {
                self.refused.mark(&ctx, note(&c).to_string());
            }
        }
        let text = &self.text;
        let runs = |t: &str| {
            let text = text.preview(t);
            let parts: Vec<_> = (text.drawn_parts().into_iter())
                .map(|p| {
                    let font = Wanted::Standard(text.font_of(&p));
                    (p, font)
                })
                .collect();
            runs(&ctx, &parts, k, |c| shown(mode, c))
        };
        let style = Style {
            runs: &runs,
            mode,
            accent,
            warn: &|text| past_edge(r, text),
        };
        let origin = Pos2::new(r.min.x + self.at.0 * k, r.min.y + self.at.1 * k);
        let mut typed = text.text().to_string();
        let f = field(&ctx, id, &mut typed, origin, &style, true, None, true);
        // The styles follow what was typed.
        let sel = f
            .out
            .cursor_range
            .map(|c| (c.secondary.index.0, c.primary.index.0));
        if self.text.sync(&typed, sel.map(|s| s.1)) {
            ctx.request_repaint();
        }
        if let Some(sel) = sel {
            self.text.caret_at(sel);
        }
        let now = ctx.input(|i| i.time);
        self.history.observe(&self.text, sel, now, settled);
        let edit = &f.out.response.response;
        dialogs::set_name(edit, NAME);
        let mut on_grip = false;
        let (esc, tab) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Tab),
            )
        });
        // Esc while it is moved by its outline cancels the move, as Esc cancels a drag under
        // way (egui stops it): it goes back where it was, and takes the keyboard back once
        // let go.
        if let Some(start) = self.moving.filter(|_| esc) {
            self.moving = None;
            self.at = start;
            self.refocus = true;
            return Shown {
                over: true,
                done: false,
                edit: edit.clone(),
            };
        }
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
        if self.fresh {
            // Nothing selected yet: none of an earlier text's selection held.
            ctx.data_mut(|d| d.remove::<(usize, usize)>(id.with(HELD)));
        }
        if self.fresh || (self.refocus && settled && !edit.has_focus()) {
            give_back(&ctx, id);
            let fresh = std::mem::take(&mut self.fresh);
            self.refocus = false;
            if fresh {
                return Shown {
                    over: true,
                    done: false,
                    edit: edit.clone(),
                };
            }
        }
        let done = (esc && (edit.lost_focus() || edit.has_focus()))
            || (tab && edit.lost_focus())
            || !ui.clip_rect().intersects(f.outline);
        Shown {
            over: done || on_grip || edit.contains_pointer(),
            done,
            edit: edit.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ENH-22: the outline of text being typed is two lines, the accent and inside it one
    /// that contrasts with it, so it shows on paper of the accent's colour too. Text that
    /// runs over (as its editor's check says) has the outline in the caution colour and a
    /// tag saying so.
    #[test]
    fn the_outline_shows_on_any_paper_and_says_when_text_runs_over() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        // Drawn at once, not faded in.
        ctx.all_styles_mut(|s| s.animation_time = 0.0);
        let accent = Color32::from_rgb(13, 148, 136);
        let caution = theme::tone(false, theme::Tone::Caution).unwrap();
        let drawn = |warn: Option<&'static str>| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
                ..Default::default()
            };
            let frame = || {
                let mut out = ctx.run_ui(raw.clone(), |ui| {
                    let runs = |t: &str| {
                        vec![Run {
                            chars: t.chars().count(),
                            font: egui::FontId::proportional(12.0),
                            coords: VariationCoords::default(),
                            color: Color32::BLACK,
                            rise: 0.0,
                        }]
                    };
                    let style = Style {
                        runs: &runs,
                        mode: PageColours::Normal,
                        accent,
                        warn: &|_| warn,
                    };
                    let (mut text, id) = ("Walrus".to_string(), ui.scope_id().with("typing"));
                    let at = Pos2::new(100.0, 100.0);
                    field(ui.ctx(), id, &mut text, at, &style, false, None, false);
                });
                out.textures_delta.clear();
                out
            };
            // An area is drawn from its second frame.
            frame();
            let out = frame();
            let strokes: Vec<(Rect, Color32)> = (out.shapes.iter())
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r) if r.stroke.width > 0.0 => Some((r.rect, r.stroke.color)),
                    _ => None,
                })
                .collect();
            let tree = out.platform_output.accesskit_update.expect("the tree");
            let says = (tree.nodes.iter()).any(|(_, n)| n.value() == Some("Past the page edge"));
            (strokes, says)
        };
        let (strokes, says) = drawn(None);
        let outer = strokes.iter().find(|s| s.1 == accent).expect("the accent");
        let halo = theme::halo(accent);
        assert!([Color32::WHITE, Color32::BLACK].contains(&halo));
        let inner = strokes
            .iter()
            .find(|s| s.1 == halo)
            .expect("the inner line");
        assert!(
            outer.0.contains_rect(inner.0) && outer.0 != inner.0,
            "{strokes:?}"
        );
        assert!(!says, "a tag for text that fits");
        let (strokes, says) = drawn(Some("Past the page edge"));
        assert!(strokes.iter().any(|s| s.1 == caution), "{strokes:?}");
        assert!(!strokes.iter().any(|s| s.1 == accent), "{strokes:?}");
        assert!(says, "no tag");
    }

    /// A line in parts is drawn as it will be written: its parts on one baseline whatever
    /// their sizes, raised ones above it, and each line's baseline [`line_gap`] below the
    /// one before.
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
        // The second line's baseline is below the first by a quarter of the 30-pixel
        // line's height and three quarters of its own 10-pixel line's.
        let second = &galley.rows[1];
        let below = second.pos.y + second.glyphs[0].pos.y - top;
        assert!(near(below, line_gap(30.0, 10.0)), "{below}");
        assert!(near(line_gap(30.0, 10.0), 18.0) && near(line_gap(10.0, 10.0), 12.0));
        // It is 1.2 times its own size high.
        assert!(
            near(galley.size().y - second.pos.y, 12.0),
            "{:?}",
            galley.size()
        );
        // Three lines of one size: 1.2 times the size apart.
        let one = [run(8, 20.0, 0.0)];
        let (galley, _) = super::galley(&ctx, "a\nb\nc", &one);
        let bases: Vec<f32> = galley
            .rows
            .iter()
            .map(|r| r.pos.y + r.glyphs[0].pos.y)
            .collect();
        assert!(near(bases[1] - bases[0], 24.0) && near(bases[2] - bases[1], 24.0));
        // Small above big above middling: each baseline its gap below the one before, and
        // the big line's small part on its baseline.
        let mixed = [
            run(2, 10.0, 0.0),
            run(2, 30.0, 0.0),
            run(2, 8.0, 0.0),
            run(2, 14.0, 0.0),
        ];
        let (galley, top) = super::galley(&ctx, "a\nbcd\nef", &mixed);
        let base = |r: usize, g: usize| galley.rows[r].pos.y + galley.rows[r].glyphs[g].pos.y;
        assert!(near(base(0, 0), top), "{} {top}", base(0, 0));
        assert!(near(base(1, 0) - base(0, 0), line_gap(10.0, 30.0)));
        assert!(
            near(base(1, 2), base(1, 0)),
            "{} {}",
            base(1, 2),
            base(1, 0)
        );
        assert!(near(base(2, 0) - base(1, 0), line_gap(30.0, 14.0)));
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
        // What no installed font has (a noncharacter here) is left out.
        assert_eq!(
            keep_typable("Seal\r\n\u{2FFFE}\tfish", false, sans),
            ("Seal\n fish".into(), Some("\u{2FFFE}".into()))
        );
        // Right-to-left scripts aren't written yet, though the face has Hebrew.
        assert_eq!(
            keep_typable("שלום", false, sans),
            (String::new(), Some("ש".into()))
        );
        // In a single line, line breaks become spaces.
        assert_eq!(
            keep_typable("Seal\r\nfish", true, sans),
            ("Seal fish".into(), None)
        );
        // Every installed font was tried, so the note points to none: what no font has,
        // right-to-left and shaped scripts and emoji sequences each have a note of their own.
        assert_eq!(note("\u{2FFFE}"), REFUSED);
        assert_eq!(note("\u{7}"), REFUSED);
        for c in ["ש", "مرحبا", "क", "ไทย"] {
            assert_eq!(note(c), SCRIPT, "{c:?}");
        }
        for s in ["👍🏽", "🇬🇧", "👨\u{200D}👩"] {
            assert_eq!(note(s), SEQUENCE, "{s:?}");
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
        // Caveat has no Greek: Greek words are typed, and written in a font that has them.
        let caveat = installed("Caveat");
        assert_eq!(
            keep_typable("Жук Ωmega", false, caveat),
            ("Жук Ωmega".into(), None)
        );
        // Single emoji in a font with their outlines; a sequence goes whole though the font
        // has every part.
        let emoji = installed("Limare Emoji Test");
        assert_eq!(
            keep_typable("Seal 😀 👍🏽 fish", false, emoji),
            ("Seal 😀  fish".into(), Some("👍🏽".into()))
        );
        assert_eq!(note("👍🏽"), SEQUENCE);
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
}
