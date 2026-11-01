//! The on-page text editor (the word-processor design, section 3): text typed where it
//! will be on the page, in the font, size and colour it will have, with no box around
//! it, only a thin outline showing where typing goes. New text (Add text) and a line of
//! page text being edited share the drawing, the keys and the note about characters
//! that can't be written.

use crate::dialogs;
use crate::styled::{CharStyle, History, Styled};
use crate::theme;
use crate::typing_fonts::{self, Wanted};
use eframe::egui::epaint::text::{ByteIndex, CharIndex, VariationCoords};
use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::{self, Color32, Galley, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use limarepdf_core::page_colours::PageColours;
use limarepdf_core::{
    emoji_sequence, unwritten_script, writable, BoxSetting, FontChoice, LineFont, ObjectKind,
    PartFont, SetLine, TextPart,
};
use std::ops::Range;
use std::sync::Arc;
use unicode_normalization::char::is_combining_mark;
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
    /// What an input method is composing in it, from and to ([`composition`]).
    composing: Option<(usize, usize)>,
}

/// What the editor did in a frame.
pub struct Shown {
    /// The pointer is over it, or it just opened: the page leaves the pointer alone.
    pub over: bool,
    /// Esc or Tab finished the typing, or the text went out of view.
    pub done: bool,
    /// The text's box as drawn, for its right-click menu.
    pub edit: egui::Response,
    /// What an input method was composing when the box lost the keyboard was taken as
    /// committed, and this, the first character that can't be written, left out.
    pub left: Option<String>,
}

/// Whether `c` can be typed in `font`: line breaks, and what LimarePDF can write in it (the
/// standard font or its embedded face, or the installed family's face).
fn typable(c: char, font: FontChoice) -> bool {
    matches!(c, '\n' | PARAGRAPH_BREAK | SOFT_HYPHEN) || writable(font, c)
}

/// What Enter types in a paragraph: a new paragraph in its frame (core's rewrap starts one
/// there, [`limarepdf_core::ParagraphSetting::set`]); Shift+Enter types a line break.
pub const PARAGRAPH_BREAK: char = '\u{2029}';

/// A discretionary hyphen in a paragraph's text: shown only where its word ends a line.
pub const SOFT_HYPHEN: char = '\u{AD}';

/// `s` without the characters that can't be typed in `font` (tabs become spaces, carriage
/// returns go, and in a `single` line, line and paragraph breaks become spaces), and the
/// first one refused. Each character as readers see it (a grapheme cluster) is kept or left
/// out whole, so an emoji sequence goes whole rather than leave a part that says something
/// else (a thumb without its skin tone).
fn keep_typable(s: &str, single: bool, font: FontChoice) -> (String, Option<String>) {
    let s: String = s
        .chars()
        .filter_map(|c| match c {
            '\r' => None,
            '\t' => Some(' '),
            '\n' | PARAGRAPH_BREAK if single => Some(' '),
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

/// The first character of `s` (a grapheme cluster) typing it on a line in `font` would
/// leave out ([`keep_typable`]), if any.
pub fn untypable(s: &str, font: FontChoice) -> Option<String> {
    keep_typable(s, true, font).1
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

/// What the inspector says of text typed in fonts that lack some of it, `pieces` being each
/// stretch with the font it is typed in: the words LimarePDF writes in another font, as
/// core sets them ([`limarepdf_core::fallbacks`]), "Not in Calibri: 漢字. Those characters
/// use Noto Sans CJK JP.", and on hover which word uses which font. `None` when every word
/// is in the font it is typed in.
pub fn fallback_note(pieces: &[(FontChoice, &str)]) -> Option<(String, String)> {
    let (mut asked, mut words) = (Vec::new(), Vec::<(String, String)>::new());
    for &(f, text) in pieces {
        for (word, used) in limarepdf_core::fallbacks(f, text) {
            let (word, used) = (word.trim().to_string(), used.family_name());
            if word.is_empty() || words.iter().any(|w| w.0 == word && w.1 == used) {
                continue;
            }
            let name = f.family_name();
            if !asked.contains(&name) {
                asked.push(name);
            }
            words.push((word, used));
        }
    }
    if words.is_empty() {
        return None;
    }
    let mut used: Vec<&str> = Vec::new();
    for (_, f) in &words {
        if !used.contains(&f.as_str()) {
            used.push(f);
        }
    }
    let used = match used.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => used.concat(),
    };
    let listed: Vec<&str> = words.iter().map(|w| w.0.as_str()).collect();
    let said = format!(
        "Not in {}: {}. Those characters use {used}.",
        asked.join(", "),
        listed.join(" ")
    );
    let why: Vec<String> = words.iter().map(|(w, f)| format!("{w}: {f}")).collect();
    Some((said, why.join("\n")))
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
    let mut t = take_undo(ui);
    ui.input_mut(|i| {
        if styles {
            t.bold = i.consume_key(egui::Modifiers::COMMAND, egui::Key::B);
            t.italic = i.consume_key(egui::Modifiers::COMMAND, egui::Key::I);
        }
    });
    t.refused = leave_out_untypable(ui, single, font);
    t
}

/// Take this frame's Ctrl+Z, and Ctrl+Y or Ctrl+Shift+Z, for the caller's [`History`].
pub fn take_undo(ui: &egui::Ui) -> Taken {
    let mut t = Taken::default();
    ui.input_mut(|i| {
        let shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        t.redo = i.consume_key(shift, egui::Key::Z)
            | i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y);
        t.undo = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z);
    });
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

/// What an input method is composing in the typing box `id` once the box has been drawn
/// this frame, from where it starts to where it ends (characters), given what it was
/// composing before (`was`). egui puts a composition in the box, selected, unchecked (the
/// text design, 5.4), and replaces it with what is committed, which the typing filter
/// checks ([`leave_out_untypable`]). A box without the keyboard keeps `was`: a composition
/// it was left with, by a click away or another tool, which [`commit_left`] takes as
/// committed, as Windows and macOS input methods commit when a box loses the keyboard.
pub fn composition(
    ctx: &egui::Context,
    id: egui::Id,
    was: Option<(usize, usize)>,
) -> Option<(usize, usize)> {
    if !ctx.memory(|m| m.has_focus(id)) {
        return was;
    }
    let last = ctx.input(|i| {
        (i.events.iter().rev()).find_map(|e| match e {
            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => Some(!text.is_empty()),
            egui::Event::Ime(egui::ImeEvent::Commit(_)) => Some(false),
            _ => None,
        })
    });
    let sel = selection(ctx, id).map(|(a, b)| (a.min(b), a.max(b)));
    match last {
        Some(true) => sel,
        Some(false) => None,
        // Typed over, or the caret moved: no longer a composition.
        None => was.filter(|&r| sel == Some(r)),
    }
}

/// `text` with a composition left in it at `range` (characters, from and to) taken as
/// committed ([`composition`]): what can't be written in `font` left out of it, as
/// [`leave_out_untypable`] leaves it out of committed text, and in a `single` line its line
/// breaks made spaces. Also returns the first character left out.
pub fn commit_left(
    text: &str,
    range: (usize, usize),
    single: bool,
    font: FontChoice,
) -> (String, Option<String>) {
    let chars: Vec<char> = text.chars().collect();
    let (a, b) = (range.0.min(chars.len()), range.1.min(chars.len()));
    let composed: String = chars[a..b.max(a)].iter().collect();
    let (kept, refused) = keep_typable(&composed, single, font);
    let mut out: String = chars[..a].iter().collect();
    out.push_str(&kept);
    out.extend(&chars[b.max(a)..]);
    (out, refused)
}

/// [`commit_left`] for styled text: the composition at `range` left in `text` taken as
/// committed, in `font`, the caret after it. Returns the first character left out.
pub fn commit_left_styled(
    text: &mut Styled,
    range: (usize, usize),
    single: bool,
    font: FontChoice,
) -> Option<String> {
    // The characters after the composition, which stay.
    let after = (text.text().chars().count()).saturating_sub(range.1.max(range.0));
    let (new, refused) = commit_left(text.text(), range, single, font);
    let caret = new.chars().count().saturating_sub(after);
    text.sync(&new, Some(caret));
    refused
}

/// What the tab says when what an input method was composing in a typing box left without
/// the keyboard ([`commit_left`]) had `refused`, a character that can't be written: it is
/// left out, as typing leaves it out ([`note`]).
pub fn left_note(refused: &str) -> String {
    format!("Left out \"{refused}\": {}", note(refused))
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

/// What a typing box holds, deleted by what readers see as one character (a grapheme
/// cluster; the text design, 5.2), where egui deletes a `char` at a time: Delete takes off
/// the character after the caret whole (a letter and its accents, a flag, a Hangul syllable
/// typed in its parts), and Backspace the one before it, except that of a letter typed with
/// separate accents it takes off the last accent, as browsers' text boxes do, so a wrong
/// accent can be typed again.
pub struct Clusters<'a>(pub &'a mut String);

impl egui::TextBuffer for Clusters<'_> {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        self.0
    }

    fn insert_text(&mut self, text: &str, at: CharIndex) -> usize {
        <String as egui::TextBuffer>::insert_text(self.0, text, at)
    }

    fn delete_char_range(&mut self, range: std::ops::Range<CharIndex>) {
        <String as egui::TextBuffer>::delete_char_range(self.0, range);
    }

    fn delete_previous_char(&mut self, caret: CCursor) -> CCursor {
        let i = caret.index.0.min(self.0.chars().count());
        let from = cluster_edge(self.0, i.saturating_sub(1), true);
        let cluster: Vec<char> = self.0.chars().skip(from).take(i - from).collect();
        let from = if accents_apart(&cluster) { i - 1 } else { from };
        self.delete_selected_ccursor_range([CCursor::new(from), CCursor::new(i)])
    }

    fn delete_next_char(&mut self, caret: CCursor) -> CCursor {
        let n = self.0.chars().count();
        let i = caret.index.0.min(n);
        let to = cluster_edge(self.0, (i + 1).min(n), false);
        self.delete_selected_ccursor_range([CCursor::new(i), CCursor::new(to)])
    }

    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Clusters<'static>>()
    }
}

/// The place nearest character `i` of `s` that is between two characters as readers see
/// them (grapheme clusters): `i` itself when it is one, else the edge before it when
/// `back`, or the edge after it; the end for `i` past it.
pub fn cluster_edge(s: &str, i: usize, back: bool) -> usize {
    let mut at = 0;
    for g in s.graphemes(true) {
        let n = g.chars().count();
        if i < at + n {
            return if i == at || back { at } else { at + n };
        }
        at += n;
    }
    at
}

/// Whether `cluster` is a letter typed with separate accents: a character, then only marks
/// that go on it (not the selectors that choose how an emoji or ideograph shows).
fn accents_apart(cluster: &[char]) -> bool {
    let mark = |c: char| {
        is_combining_mark(c) && !matches!(c, '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}')
    };
    cluster.len() > 1 && !mark(cluster[0]) && cluster[1..].iter().all(|&c| mark(c))
}

/// Keep the caret and what is selected in the typing box `id`, which holds `text`, between
/// characters as readers see them (the text design, 5.2). egui moves them a `char` at a
/// time, so an arrow key, a click or a drag that leaves one inside a letter typed with
/// separate accents, a flag or a Hangul syllable typed in its parts moves it on to that
/// character's edge, the way it was going from `before` (what was selected before the box
/// was drawn this frame), else to the edge before it. What an input method composes is
/// the input method's to place, so a frame with its events is left as it is. Returns what
/// is selected then.
pub fn snap_to_clusters(
    ctx: &egui::Context,
    id: egui::Id,
    text: &str,
    before: Option<CCursorRange>,
) -> Option<CCursorRange> {
    let mut st = egui::TextEdit::load_state(ctx, id)?;
    let now = st.cursor.char_range()?;
    let composing = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Ime(_))));
    if composing || before == Some(now) {
        return Some(now);
    }
    let snap = |c: CCursor, was: Option<CCursor>| {
        let back = was.is_none_or(|w| c.index.0 <= w.index.0);
        CCursor {
            index: CharIndex(cluster_edge(text, c.index.0, back)),
            ..c
        }
    };
    let snapped = CCursorRange {
        primary: snap(now.primary, before.map(|b| b.primary)),
        secondary: snap(now.secondary, before.map(|b| b.secondary)),
        ..now
    };
    if snapped != now {
        st.cursor.set_char_range(Some(snapped));
        st.store(ctx, id);
    }
    Some(snapped)
}

/// [`snap_to_clusters`] for the typing box `id` drawn as `out`, holding `text`.
pub fn snap_drawn(
    ctx: &egui::Context,
    id: egui::Id,
    out: &mut egui::text_edit::TextEditOutput,
    text: &str,
    before: Option<CCursorRange>,
) {
    if let Some(r) = snap_to_clusters(ctx, id, text, before) {
        out.state.cursor.set_char_range(Some(r));
        if out.cursor_range.is_some() {
            out.cursor_range = Some(r);
        }
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

/// The runs `parts` are drawn in at `k` pixels per point, in a document whose language is
/// `lang`: each part in the font it will be written in, the words that font lacks in the
/// fonts core writes them in ([`limarepdf_core::set_in`]), its colour as `color` shows it.
pub fn runs(
    ctx: &egui::Context,
    parts: &[(TextPart, Wanted)],
    k: f32,
    color: impl Fn([f32; 3]) -> Color32,
    lang: Option<&str>,
) -> Vec<Run> {
    let mut out = Vec::new();
    for (p, wanted) in parts {
        let run = |chars: usize, wanted: Wanted| Run {
            chars,
            font: egui::FontId::new((p.size * k).max(1.0), typing_fonts::family(ctx, wanted)),
            coords: typing_fonts::coords(wanted),
            color: color(p.color),
            rise: p.rise * k,
        };
        let n = p.text.chars().count();
        // A part kept in a font whose own glyphs can't be drawn (one LimarePDF wrote, which
        // grows as text is added, say) is drawn in the installed family it is from, else its
        // standard stand-in, as text set anew in it is.
        let wanted = match *wanted {
            Wanted::Kept(name, font) if font.and_then(LineFont::preview_key).is_none() => {
                Wanted::Standard(crate::editor::font_of(name))
            }
            wanted => wanted,
        };
        match wanted {
            // What can't be written goes as it is typed: drawn in the font asked for.
            Wanted::Standard(f) if n > 0 => {
                for ((a, b), set) in limarepdf_core::set_in(f, &p.text, lang) {
                    out.push(run(b - a, Wanted::Standard(set.unwrap_or(f))));
                }
            }
            wanted => out.push(run(n, wanted)),
        }
    }
    out
}

/// Text typed in a box of its own, a form field's or a text box's, laid out in the fonts it
/// is written in: `font` at `px` pixels, the words it lacks in the fonts core writes them in
/// ([`runs`]), in `color`, in a document whose language is `lang`; on one line, or on the
/// `lines` core breaks it into (each line's bytes, [`limarepdf_core::BoxSetting::lines`])
/// `leading` times `px` apart. Also returns how far its first baseline is below the top.
pub fn boxed_galley(
    ctx: &egui::Context,
    text: &str,
    font: FontChoice,
    px: f32,
    color: Color32,
    lines: Option<(&[Range<usize>], f32)>,
    lang: Option<&str>,
) -> (Arc<Galley>, f32) {
    let part = TextPart {
        text: text.to_string(),
        font: PartFont::Choice(font),
        size: px,
        color: [0.0; 3],
        rise: 0.0,
        from: None,
        origins: Vec::new(),
    };
    let mut runs = runs(ctx, &[(part, Wanted::Standard(font))], 1.0, |_| color, lang);
    let Some((lines, leading)) = lines else {
        return galley(ctx, text, &runs);
    };
    // Each line a row of its own: a line break goes after each line core broke that doesn't
    // end with one, in the run of the character before it. Where they go, in bytes.
    let mut broken = String::with_capacity(text.len() + lines.len());
    let mut added: Vec<usize> = Vec::new();
    let mut soft: Vec<bool> = Vec::new();
    let mut from = 0;
    for i in 0..lines.len() {
        let to = lines.get(i + 1).map_or(text.len(), |next| next.start);
        broken.push_str(&text[from..to]);
        from = to;
        let made = i + 1 < lines.len() && !broken.ends_with('\n');
        if made {
            // The characters before it, those added before among them.
            let mut before = text[..to].chars().count() + added.len();
            added.push(broken.len());
            broken.push('\n');
            if let Some(r) = runs.iter_mut().find(|r| {
                let first = before <= r.chars;
                before = before.saturating_sub(r.chars);
                first
            }) {
                r.chars += 1;
            }
        }
        soft.push(made);
    }
    broken.push_str(&text[from..]);
    let (laid, base) = galley_spaced(ctx, &broken, &runs, leading);
    // Laid out, those line breaks take no character, and the text is as typed: what the
    // caret, selection and double-click go by.
    let mut g = (*laid).clone();
    for (row, &made) in g.rows.iter_mut().zip(&soft) {
        if made {
            row.ends_with_newline = false;
        }
    }
    let typed = |b: usize| ByteIndex(b - added.iter().filter(|&&a| a < b).count());
    let mut job = (*g.job).clone();
    job.text = text.to_string();
    for s in &mut job.sections {
        s.byte_range = typed(s.byte_range.start.0)..typed(s.byte_range.end.0);
    }
    g.job = Arc::new(job);
    (Arc::new(g), base)
}

/// Text typed in a box that wraps it, laid out as it is written ([`set_in_box`]).
pub struct Boxed {
    pub galley: Arc<Galley>,
    /// What puts it where it is written in the box: as far in from the box's sides as its
    /// border and padding, its first baseline 0.8 times its size below that.
    pub margin: egui::Margin,
    /// Its size, in pixels.
    pub px: f32,
}

/// `text` typed in a box that wraps it, set as `set` sets it ([`boxed_galley`]): in `font`,
/// at `k` pixels a point, in `color`, in a document whose language is `lang`.
pub fn set_in_box(
    ctx: &egui::Context,
    set: &BoxSetting,
    text: &str,
    font: FontChoice,
    k: f32,
    color: Color32,
    lang: Option<&str>,
) -> Boxed {
    let (size, lines) = set.lines(text, lang);
    let px = (size * k).max(1.0);
    let lines = Some((&lines[..], set.leading()));
    let (galley, base) = boxed_galley(ctx, text, font, px, color, lines, lang);
    let side = set.margin() * k;
    let at = |v: f32| v.round().clamp(-128.0, 127.0) as i8;
    let margin = egui::Margin {
        left: at(side),
        right: at(side),
        top: at(side + 0.8 * px - base),
        bottom: at(side),
    };
    Boxed { galley, margin, px }
}

/// A paragraph being typed, `text`, laid out on the lines core sets it in (`lines`,
/// [`limarepdf_core::ParagraphSetting::set`]): each line a row of its own at its place on the
/// page (`at` puts a place in page points on the screen), its spaces stretched as a
/// justified line's are written, a soft hyphen shown only at the end of a line that ends
/// with it, drawn in `runs` ([`runs`], `k` screen points a page point). egui wraps nothing:
/// the rows are core's lines, so the caret, the selection and an input method go by the
/// breaks that are written. Also returns where the galley's top left is on the screen.
pub fn paragraph_galley(
    ctx: &egui::Context,
    text: &str,
    runs: &[Run],
    lines: &[SetLine],
    k: f32,
    at: &dyn Fn(f32, f32) -> Pos2,
) -> (Arc<Galley>, Pos2) {
    let chars: Vec<char> = text.chars().collect();
    // What is laid out: each line ending in a line break (put in where the text has none),
    // a paragraph break shown as one, a soft hyphen ending a line where it shows as a
    // hyphen. For each character laid out: the typed character it is (none for a break put
    // in), what it adds after it (a justified line's spaces), and whether it is hidden (a
    // soft hyphen inside a line).
    let mut shown = String::with_capacity(text.len() + lines.len());
    let mut of: Vec<Option<usize>> = Vec::with_capacity(chars.len() + lines.len());
    let mut extra: Vec<(usize, f32)> = Vec::new();
    let mut hidden: Vec<usize> = Vec::new();
    let mut runs = runs.to_vec();
    let mut soft: Vec<bool> = Vec::new();
    let mut done = 0;
    for (i, l) in lines.iter().enumerate() {
        let (a, b) = (l.chars.0.min(chars.len()), l.chars.1.min(chars.len()));
        let mut end = b;
        while end > a && chars[end - 1].is_whitespace() {
            end -= 1;
        }
        for (j, &c) in chars.iter().enumerate().take(b).skip(a.max(done)) {
            let n = of.len();
            shown.push(match c {
                PARAGRAPH_BREAK => '\n',
                SOFT_HYPHEN if l.hyphen && j + 1 == end => '-',
                SOFT_HYPHEN => {
                    hidden.push(n);
                    c
                }
                ' ' if j < end && l.stretch > 0.0 => {
                    extra.push((n, l.stretch * k));
                    c
                }
                c => c,
            });
            of.push(Some(j));
        }
        done = done.max(b);
        let made = i + 1 < lines.len() && !shown.ends_with('\n');
        if made {
            // In the run of the character before it.
            let mut before = of.len();
            if let Some(r) = runs.iter_mut().find(|r| {
                let first = before <= r.chars;
                before = before.saturating_sub(r.chars);
                first
            }) {
                r.chars += 1;
            }
            shown.push('\n');
            of.push(None);
        }
        soft.push(made);
    }
    for (j, &c) in chars.iter().enumerate().skip(done) {
        shown.push(c);
        of.push(Some(j));
    }
    // Each laid-out character's bytes.
    let bytes: Vec<usize> = (shown.char_indices().map(|(b, _)| b))
        .chain([shown.len()])
        .collect();
    let laid = ctx.fonts_mut(|f| {
        let (mut job, _) = job(f, &shown, &runs, LEADING);
        // The stretched spaces and hidden soft hyphens, each a section of its own: what
        // follows a space starts the stretch further on (egui spaces a section's start, not
        // its end), on the same line; a soft hyphen, which egui's shaping gives no width,
        // has no ink.
        let mut special: Vec<(usize, Option<f32>)> = (extra.iter())
            .map(|&(n, e)| (n, Some(e)))
            .chain(hidden.iter().map(|&n| (n, None)))
            .collect();
        special.sort_by_key(|s| s.0);
        let mut sections = Vec::with_capacity(job.sections.len() + 2 * special.len());
        let mut next = special.iter().peekable();
        let mut carry = 0.0;
        // An empty section (empty text) stays, for its look.
        let mut push = |range: Range<usize>, s: &egui::text::LayoutSection, carry: &mut f32| {
            if range.is_empty() && !s.byte_range.is_empty() {
                return;
            }
            sections.push(egui::text::LayoutSection {
                leading_space: s.leading_space + std::mem::take(carry),
                byte_range: ByteIndex(range.start)..ByteIndex(range.end),
                format: s.format.clone(),
            });
        };
        for s in std::mem::take(&mut job.sections) {
            let (from, to) = (s.byte_range.start.0, s.byte_range.end.0);
            let mut at = from;
            while let Some(&&(n, e)) = next.peek().filter(|(n, _)| bytes[*n] < to) {
                next.next();
                let (a, b) = (bytes[n], bytes[n + 1]);
                if a < at {
                    continue;
                }
                push(at..a, &s, &mut carry);
                let mut own = s.clone();
                if e.is_none() {
                    own.format.color = Color32::TRANSPARENT;
                }
                push(a..b, &own, &mut carry);
                carry = e.unwrap_or(0.0);
                at = b;
            }
            push(at..to, &s, &mut carry);
            if shown[from..to].contains('\n') {
                carry = 0.0;
            }
        }
        job.sections = sections;
        f.layout_job(job)
    });
    let mut g = (*laid).clone();
    if g.rows.len() != lines.len() {
        return (laid, at(0.0, 0.0));
    }
    // The breaks put in take no character; the job's text is the text as typed, what the
    // caret, selection and double-click go by.
    for (row, &made) in g.rows.iter_mut().zip(&soft) {
        if made {
            row.ends_with_newline = false;
        }
    }
    let typed_byte: Vec<usize> = {
        let starts: Vec<usize> = (text.char_indices().map(|(b, _)| b))
            .chain([text.len()])
            .collect();
        let mut out = Vec::with_capacity(bytes.len());
        let mut next = 0;
        for o in &of {
            out.push(starts[next]);
            if let Some(j) = o {
                next = j + 1;
            }
        }
        out.push(text.len());
        out
    };
    let to_typed = |b: usize| {
        let n = bytes.partition_point(|&x| x < b);
        ByteIndex(typed_byte[n.min(typed_byte.len() - 1)])
    };
    let mut job = (*g.job).clone();
    job.text = text.to_string();
    for s in &mut job.sections {
        s.byte_range = to_typed(s.byte_range.start.0)..to_typed(s.byte_range.end.0);
    }
    g.job = Arc::new(job);
    // Each row at its line's place: its baseline on the line's, its start at the line's x.
    let rise = |n: usize| {
        let mut left = n;
        (runs.iter())
            .find(|r| {
                let here = left < r.chars;
                left = left.saturating_sub(r.chars);
                here
            })
            .map_or(0.0, |r| r.rise)
    };
    // Each row's first character laid out: a glyph a character, and the break ending it.
    let mut start = 0;
    let mut places = Vec::with_capacity(lines.len());
    for (row, l) in g.rows.iter().zip(lines) {
        let base = match row.row.glyphs.first() {
            Some(gl) => gl.pos.y + rise(start),
            None => row.row.size.y * 0.8,
        };
        places.push(at(l.x, l.baseline) - Vec2::new(0.0, base));
        start += row.row.glyphs.len() + 1;
    }
    let min = (places.iter()).fold(Pos2::new(f32::MAX, f32::MAX), |m, p| m.min(*p));
    let (mut rect, mut mesh) = (Rect::NOTHING, Rect::NOTHING);
    for (row, p) in g.rows.iter_mut().zip(&places) {
        row.pos = (*p - min).to_pos2();
        rect = rect.union(row.rect());
        mesh = mesh.union(row.row.visuals.mesh_bounds.translate(row.pos.to_vec2()));
    }
    g.rect = rect;
    g.mesh_bounds = mesh;
    (Arc::new(g), min)
}

/// How a paragraph being typed on the page is drawn this frame ([`paragraph_field`]).
pub struct ParaStyle<'a> {
    /// The text as the editor has it laid out on its lines ([`paragraph_galley`]), and
    /// where its top left is on the screen.
    pub laid: &'a dyn Fn(&str) -> (Arc<Galley>, Pos2),
    /// The page colour mode, for the caret and selection: light on dark paper.
    pub mode: PageColours,
    /// The outline's colour.
    pub accent: Color32,
    /// The frame's left and right edges on the screen, which the outline takes in, with a
    /// handle on each to drag ([`Field::edges`]).
    pub frame: (f32, f32),
    /// What the text, as its rows are laid out on the screen, runs over, and where
    /// ([`overflow`]). `None` when it fits.
    pub warn: &'a dyn Fn(&[Rect]) -> Option<Overflow>,
}

/// The names of a paragraph's frame's handles, left then right, as screen readers say them.
pub const EDGE_NAMES: [&str; 2] = ["Paragraph's left edge", "Paragraph's right edge"];

/// What hides text being edited under the typing, so it doesn't show twice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cover {
    /// One colour, the most common around it.
    Flat(Color32),
    /// The page drawn without it (ENH-21): the picture, and where it goes on the screen.
    Page(egui::TextureId, Rect),
}

impl Cover {
    /// Hide `r`, on the screen.
    fn paint(&self, painter: &egui::Painter, r: Rect) {
        match *self {
            Cover::Flat(fill) => {
                painter.rect_filled(r, 0.0, fill);
            }
            Cover::Page(tex, at) => {
                // The part of the picture under `r`.
                let uv = |p: Pos2| {
                    let v = (p - at.min) / at.size().max(Vec2::splat(1e-3));
                    Pos2::new(v.x.clamp(0.0, 1.0), v.y.clamp(0.0, 1.0))
                };
                let part = Rect::from_min_max(uv(r.min), uv(r.max));
                painter.image(tex, r, part, Color32::WHITE);
            }
        }
    }
}

/// The id of a paragraph editor `id`'s frame's left (`0`) or right (`1`) handle.
pub fn edge_id(id: egui::Id, side: usize) -> egui::Id {
    id.with(("frame-edge", side))
}

/// Draw a paragraph, `text`, for typing (the editor `id`) on the lines it will be written
/// on ([`ParaStyle::laid`]), outlined round its frame, with a handle on the outline's left
/// and right edges ([`edge_id`]) to drag or move from the keyboard. `covers`, rectangles on
/// the screen, are hidden under it first as `hide` says: the lines being edited, so they
/// don't show twice.
pub fn paragraph_field(
    ctx: &egui::Context,
    id: egui::Id,
    text: &mut String,
    style: &ParaStyle,
    covers: &[Rect],
    hide: Option<Cover>,
) -> Field {
    let (galley, origin) = (style.laid)(text);
    let inner = Rect::from_min_size(origin, galley.size());
    let rows: Vec<Rect> = (galley.rows.iter())
        .map(|r| r.rect().translate(origin.to_vec2()))
        .collect();
    let warning = (style.warn)(&rows);
    let (left, right) = style.frame;
    let x = left.min(inner.min.x)..=right.max(inner.max.x);
    let outline = Rect::from_x_y_ranges(x, inner.y_range()).expand(3.0);
    let covered = (covers.iter()).fold(outline, |r, c| r.union(*c));
    let mut tag = None;
    let mut edges = None;
    let out = egui::Area::new(id.with("area"))
        .fixed_pos(covered.min)
        .constrain(false)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            *ui.visuals_mut() = match style.mode {
                PageColours::Dark | PageColours::HighContrast => egui::Visuals::dark(),
                PageColours::Normal | PageColours::Sepia => egui::Visuals::light(),
            };
            ui.expand_to_include_rect(covered);
            if let Some(how) = hide {
                for c in covers {
                    how.paint(ui.painter(), *c);
                }
            }
            tag = outline_ui(ui, outline, style.accent, warning);
            let mut layouter = |_: &egui::Ui, t: &dyn egui::TextBuffer, _: f32| {
                let (g, at) = (style.laid)(t.as_str());
                // Laid out with what this frame typed, it starts where the box does.
                if at == origin {
                    return g;
                }
                let mut g = (*g).clone();
                for row in &mut g.rows {
                    row.pos += at - origin;
                }
                g.rect = g.rect.translate(at - origin);
                g.mesh_bounds = g.mesh_bounds.translate(at - origin);
                Arc::new(g)
            };
            let mut buffer = Clusters(text);
            let te = egui::TextEdit::multiline(&mut buffer)
                .id(id)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .desired_rows(1)
                .desired_width(inner.width())
                .min_size(inner.size())
                .clip_text(false)
                .return_key(None)
                .layouter(&mut layouter);
            let held = egui::TextEdit::load_state(ctx, id).and_then(|s| s.cursor.char_range());
            let mut out = ui
                .scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| te.show(ui))
                .inner;
            keep_on_right_click(ctx, id, &mut out, held);
            snap_drawn(ctx, id, &mut out, buffer.0, held);
            // After the text, so they are above it where they meet.
            let colour = outline_colour(ui, style.accent, warning);
            edges = Some([0, 1].map(|side| edge_ui(ui, id, outline, side, colour)));
            out
        })
        .inner;
    Field {
        out,
        grip: None,
        outline,
        tag,
        edges,
    }
}

/// A paragraph's frame's handle on the outline's `side` edge (0 left, 1 right) for the
/// editor `id`: a small square in the middle of the edge, as an object's handles are drawn,
/// filled while it is dragged or has the keyboard; the whole edge drags it, the pointer a
/// horizontal resize over it. Named for screen readers as a splitter ([`EDGE_NAMES`]).
fn edge_ui(
    ui: &mut egui::Ui,
    id: egui::Id,
    outline: Rect,
    side: usize,
    colour: Color32,
) -> egui::Response {
    let x = [outline.min.x, outline.max.x][side];
    let strip = Rect::from_x_y_ranges(x - 4.0..=x + 4.0, outline.y_range());
    let handle = Rect::from_center_size(Pos2::new(x, outline.center().y), Vec2::splat(7.0));
    let edge = edge_id(id, side);
    let r = ui.interact(strip.union(handle), edge, Sense::click_and_drag());
    let name = EDGE_NAMES[side];
    ui.ctx().accesskit_node_builder(edge, |n| {
        n.set_role(egui::accesskit::Role::Splitter);
        n.set_label(name);
    });
    if r.hovered() || r.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let held = r.dragged() || r.has_focus();
    let painter = ui.painter();
    if r.has_focus() {
        painter.vline(x, outline.y_range(), Stroke::new(2.0, colour));
    }
    let fill = if held { colour } else { Color32::WHITE };
    painter.rect_filled(handle, 1.0, fill);
    painter.rect_stroke(handle, 1.0, Stroke::new(1.0, colour), StrokeKind::Inside);
    r
}

/// The outline's colour: the accent, or the caution colour when the text runs over.
fn outline_colour(ui: &egui::Ui, accent: Color32, warning: Option<Overflow>) -> Color32 {
    match warning {
        Some(_) => theme::tone(ui.visuals().dark_mode, theme::Tone::Caution),
        None => None,
    }
    .unwrap_or(accent)
}

/// The outline round text being typed on the page (ENH-22): the accent, or the caution
/// colour when it runs over (`warning`, said on a tag just below the row where it happens,
/// PB-35), and inside it a line that stands out against it, so it shows on paper of either
/// colour. Returns where the tag is on the screen.
fn outline_ui(
    ui: &mut egui::Ui,
    outline: Rect,
    accent: Color32,
    warning: Option<Overflow>,
) -> Option<Rect> {
    let edge = outline_colour(ui, accent, warning);
    let halo = Stroke::new(1.0, theme::halo(edge));
    let painter = ui.painter();
    painter.rect_stroke(outline, 2.0, Stroke::new(1.0, edge), StrokeKind::Inside);
    painter.rect_stroke(outline.shrink(1.0), 1.0, halo, StrokeKind::Inside);
    let w = warning?;
    let text = egui::RichText::new(w.why)
        .size(11.0)
        .color(theme::halo(edge));
    let g = painter.layout_no_wrap(w.why.into(), egui::FontId::proportional(11.0), edge);
    let size = g.size() + Vec2::new(8.0, 4.0);
    let at = match w.leftward {
        true => w.at - Vec2::new(size.x, 0.0),
        false => w.at,
    };
    let tag = Rect::from_min_size(at, size);
    ui.expand_to_include_rect(tag);
    ui.painter().rect_filled(tag, 3.0, edge);
    ui.put(tag, egui::Label::new(text).selectable(false));
    Some(tag)
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
    /// What the text, as its rows are laid out on the screen (without the room for the
    /// caret), runs over, and where ([`overflow`]). `None` when it fits.
    pub warn: &'a dyn Fn(&[Rect]) -> Option<Overflow>,
}

/// One frame of an on-page editor.
pub struct Field {
    pub out: egui::text_edit::TextEditOutput,
    /// The band around the outline, which moves the text; `None` when it can't be moved.
    pub grip: Option<egui::Response>,
    pub outline: Rect,
    /// Where the tag saying what the text runs over is, when it does.
    pub tag: Option<Rect>,
    /// A paragraph's frame's handles, left and right ([`paragraph_field`]).
    pub edges: Option<[egui::Response; 2]>,
}

/// `text` laid out as [`job`] lays it out in `runs`, its lines [`LEADING`] times their size
/// apart, and how far its first baseline is below the top.
pub fn galley(ctx: &egui::Context, text: &str, runs: &[Run]) -> (Arc<Galley>, f32) {
    galley_spaced(ctx, text, runs, LEADING)
}

/// [`galley`] with its lines `leading` times their size apart.
fn galley_spaced(
    ctx: &egui::Context,
    text: &str,
    runs: &[Run],
    leading: f32,
) -> (Arc<Galley>, f32) {
    ctx.fonts_mut(|f| {
        let (j, ascent) = job(f, text, runs, leading);
        (f.layout_job(j), ascent)
    })
}

/// `text` laid out as it is drawn in `runs` (the last one takes any characters left over):
/// never wrapped, each line's baseline [`line_gap`] below the one before (scaled from
/// [`LEADING`] to `leading`), the runs of a line on one baseline and raised ones above it.
/// Also returns how far the first baseline is below the top.
fn job(
    f: &mut egui::epaint::FontsView,
    text: &str,
    runs: &[Run],
    leading: f32,
) -> (egui::text::LayoutJob, f32) {
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
        let gap = |below| line_gap(size, below) * leading / LEADING;
        let height = match next {
            Some((below, base)) => 2.0 * (gap(below) + ascent + r.rise - base) - own,
            None => leading * size,
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

/// What text being typed on the page runs over, and where ([`overflow`]): said on a tag
/// just below the row where it happens, in the caution colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Overflow {
    /// "Past the page edge", "Over other text", "Over a picture" or "Over a shape".
    pub why: &'static str,
    /// Where the tag goes on the screen: its top left, or with `leftward` its top right.
    pub at: Pos2,
    /// The tag ends at `at`: the page's right edge was crossed, so it stays on the page.
    pub leftward: bool,
}

/// What a page object is, as a tag says text runs over it.
pub fn over(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Text => "Over other text",
        ObjectKind::Image => "Over a picture",
        _ => "Over a shape",
    }
}

/// The one overflow rule for text typed on the page (PB-35): the first of `rows` (the text's
/// rows as laid out on the screen) that runs past the edge of the page drawn at `page` (a
/// point's leeway for rounding), or, across the middle of its height, where the rows above
/// and below touch, over one of `others` (on the screen, each with what [`over`] calls it):
/// what, and where, at the leftmost thing it runs over, or the side of the page it crosses.
pub fn overflow(page: Rect, rows: &[Rect], others: &[(Rect, &'static str)]) -> Option<Overflow> {
    let edge = page.expand(1.0);
    for row in rows.iter().filter(|r| r.is_positive()) {
        let below = row.max.y + 2.0;
        if !edge.contains_rect(*row) {
            let (x, leftward) = match () {
                _ if row.max.x > edge.max.x => (page.max.x, true),
                _ if row.min.x < edge.min.x => (page.min.x, false),
                _ => (row.min.x, false),
            };
            return Some(Overflow {
                why: "Past the page edge",
                at: Pos2::new(x, below),
                leftward,
            });
        }
        let (mid, half) = (row.center().y, row.height() * 0.25);
        let band = Rect::from_x_y_ranges(row.x_range(), mid - half..=mid + half);
        let first = (others.iter())
            .filter(|(o, _)| o.intersects(band))
            .min_by(|a, b| a.0.min.x.total_cmp(&b.0.min.x));
        if let Some((o, why)) = first {
            return Some(Overflow {
                why,
                at: Pos2::new(o.min.x.max(row.min.x), below),
                leftward: false,
            });
        }
    }
    None
}

/// Draw `text` for typing (the editor `id`) with its first baseline's left end at
/// `origin` on screen: one line, or several 1.2 times the size apart. `cover`, a
/// rectangle on screen and what hides it, is painted under it first: the line being
/// edited, hidden so it doesn't show twice. A `movable` editor has a grip around its
/// outline.
#[allow(clippy::too_many_arguments)]
pub fn field(
    ctx: &egui::Context,
    id: egui::Id,
    text: &mut String,
    origin: Pos2,
    style: &Style,
    multiline: bool,
    cover: Option<(Rect, Cover)>,
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
    let rows: Vec<Rect> = (galley.rows.iter())
        .map(|r| r.rect().translate(inner.min.to_vec2()))
        .collect();
    let warning = (style.warn)(&rows);
    let outline = inner.expand(3.0);
    let grip = outline.expand(3.0);
    let top_left = cover.map_or(grip.min, |(c, _)| grip.min.min(c.min));
    let mut tag = None;
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
            if let Some((c, how)) = cover {
                ui.expand_to_include_rect(c);
                how.paint(ui.painter(), c);
            }
            // The outline around the text moves it; the text itself takes the pointer. Not a
            // stop for the keyboard: Esc keeps the text, selected, and the arrow keys move
            // it, as in word processors.
            let grab = movable.then(|| {
                let r = ui.interact(grip, id.with("grip"), Sense::DRAG);
                dialogs::set_name(&r, "Move the text");
                r
            });
            tag = outline_ui(ui, outline, style.accent, warning);
            let mut layouter =
                |_: &egui::Ui, text: &dyn egui::TextBuffer, _: f32| laid(text.as_str()).0;
            let mut buffer = Clusters(text);
            let te = if multiline {
                egui::TextEdit::multiline(&mut buffer)
            } else {
                egui::TextEdit::singleline(&mut buffer)
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
            snap_drawn(ctx, id, &mut out, buffer.0, held);
            (grab, out)
        })
        .inner;
    Field {
        out,
        grip: grab,
        outline,
        tag,
        edges: None,
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
            composing: None,
        }
    }

    /// The editor's id.
    pub fn id(&self) -> egui::Id {
        self.scope.with(("page-text", self.page))
    }

    /// Take what an input method was still composing as committed ([`commit_left`]), as
    /// when the box loses the keyboard. Returns the first character left out.
    pub fn commit_composition(&mut self) -> Option<String> {
        let r = self.composing.take()?;
        let font = self.text.typing_font(r);
        commit_left_styled(&mut self.text, r, false, font)
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
    /// keyboard can come back. `lang` is the document's language, which sets the fonts of
    /// Chinese, Japanese and Korean words. `others` are what it may run over on the screen
    /// ([`overflow`]).
    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        r: Rect,
        k: f32,
        mode: PageColours,
        accent: Color32,
        settled: bool,
        lang: Option<&str>,
        others: &[(Rect, &'static str)],
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
            runs(&ctx, &parts, k, |c| shown(mode, c), lang)
        };
        let style = Style {
            runs: &runs,
            mode,
            accent,
            warn: &|rows| overflow(r, rows, others),
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
        let edit = &f.out.response.response;
        // What an input method composes shows unchecked; left without the keyboard, it is
        // taken as committed (PB-54).
        self.composing = composition(&ctx, id, self.composing);
        let left = match edit.has_focus() {
            true => None,
            false => self.commit_composition(),
        };
        let now = ctx.input(|i| i.time);
        self.history.observe(&self.text, sel, now, settled);
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
                left,
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
                    left,
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
            left,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A paragraph's typing box lays out core's lines ([`paragraph_galley`]): a row a line,
    /// each starting at its line's place with its baseline on the line's; a soft hyphen
    /// shown as a hyphen where it ends a line and taking no room inside one; a justified
    /// line's spaces wider by its stretch; a paragraph break and a line break each ending
    /// their row; the galley's text the text as typed, which the caret goes by.
    #[test]
    fn a_paragraphs_box_lays_out_cores_lines_where_they_are_written() {
        let ctx = egui::Context::default();
        (ctx.run_ui(Default::default(), |_| {}).textures_delta).clear();
        let text = "One two\u{AD}thr\u{AD}ee four five\u{2029}Six\nseven";
        let chars: Vec<char> = text.chars().collect();
        let at_char = |s: &str| {
            let b = text.find(s).unwrap();
            text[..b].chars().count()
        };
        let ends = ["thr", "Six", "seven"].map(at_char);
        let ends = [ends[0], ends[1], ends[2], chars.len()];
        let mut lines = Vec::new();
        let mut from = 0;
        for (i, &to) in ends.iter().enumerate() {
            lines.push(SetLine {
                chars: (from, to),
                kept: None,
                x: if i == 1 { 80.0 } else { 72.0 },
                baseline: 100.0 + 14.0 * i as f32,
                width: 0.0,
                stretch: if i == 0 { 2.0 } else { 0.0 },
                hyphen: i == 0,
            });
            from = to;
        }
        let k = 2.0;
        let run = Run {
            chars: chars.len(),
            font: egui::FontId::proportional(12.0 * k),
            coords: VariationCoords::default(),
            color: Color32::BLACK,
            rise: 0.0,
        };
        let at = |x: f32, y: f32| Pos2::new(10.0 + x * k, 20.0 + y * k);
        let (g, origin) = paragraph_galley(&ctx, text, &[run], &lines, k, &at);
        assert_eq!(g.rows.len(), lines.len());
        assert_eq!(g.job.text, text);
        for (row, l) in g.rows.iter().zip(&lines) {
            let start = origin + row.pos.to_vec2();
            let base = start.y + row.row.glyphs[0].pos.y;
            let want = at(l.x, l.baseline);
            assert!((start.x - want.x).abs() < 0.01, "{l:?}: at {}", start.x);
            assert!((base - want.y).abs() < 0.01, "{l:?}: baseline {base}");
            // Its characters, the break ending it among them where the text has one.
            let n = row.row.glyphs.len() + usize::from(row.ends_with_newline);
            assert_eq!(n, l.chars.1 - l.chars.0, "{l:?}");
        }
        let glyphs = |i: usize| &g.rows[i].row.glyphs;
        // The soft hyphen ending the first line shows; the one inside the second takes no
        // room.
        assert_eq!(glyphs(0).last().unwrap().chr, '-');
        let inside = glyphs(1).iter().position(|c| c.chr == SOFT_HYPHEN).unwrap();
        let (h, next) = (&glyphs(1)[inside], &glyphs(1)[inside + 1]);
        assert!((next.pos.x - h.pos.x).abs() < 0.01, "{h:?} {next:?}");
        // The first line's space, stretched 2 points (4 on the screen).
        let space = glyphs(0).iter().position(|c| c.chr == ' ').unwrap();
        let (s, after) = (&glyphs(0)[space], &glyphs(0)[space + 1]);
        let more = after.pos.x - s.pos.x - s.advance_width;
        assert!((more - 2.0 * k).abs() < 0.5, "stretched by {more}");
        // The paragraph break and the line break end their rows as breaks.
        assert!(g.rows[1].ends_with_newline && g.rows[2].ends_with_newline);
        // A break put in takes no character.
        assert!(!g.rows[0].ends_with_newline);
    }

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
        let drawn = |warn: Option<Overflow>| {
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
        let past = Overflow {
            why: "Past the page edge",
            at: Pos2::new(300.0, 120.0),
            leftward: true,
        };
        let (strokes, says) = drawn(Some(past));
        assert!(strokes.iter().any(|s| s.1 == caution), "{strokes:?}");
        assert!(!strokes.iter().any(|s| s.1 == accent), "{strokes:?}");
        assert!(says, "no tag");
    }

    /// PB-35: one rule for text typed on the page, row by row. The first row that runs past
    /// the page's edge, or across the middle of its height over something, says what and
    /// where: at the leftmost thing it runs over, or the side of the page it crosses; a row
    /// that only grazes something above or below it says nothing.
    #[test]
    fn text_runs_over_where_its_first_row_meets_something_or_the_pages_edge() {
        let page = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(600.0, 800.0));
        let row = |x0: f32, x1: f32, y: f32| Rect::from_x_y_ranges(x0..=x1, y..=y + 14.0);
        let others = [
            (row(300.0, 340.0, 100.0), "Over a picture"),
            (row(200.0, 240.0, 100.0), "Over a shape"),
            // Only its last two points reach into the row's top.
            (row(100.0, 500.0, 88.0), "Over other text"),
        ];
        let at = |w: Option<Overflow>| w.map(|w| (w.why, w.at, w.leftward));
        assert_eq!(overflow(page, &[row(50.0, 180.0, 100.0)], &others), None);
        let (shape, past) = ("Over a shape", "Past the page edge");
        let into = overflow(page, &[row(50.0, 350.0, 100.0)], &others);
        assert_eq!(at(into), Some((shape, Pos2::new(200.0, 116.0), false)));
        // A row before it that runs over comes first; one past the right edge says so there.
        let rows = [row(50.0, 610.0, 80.0), row(50.0, 350.0, 100.0)];
        let right = overflow(page, &rows, &others);
        assert_eq!(at(right), Some((past, Pos2::new(600.0, 96.0), true)));
        let left = overflow(page, &[row(-20.0, 100.0, 300.0)], &others);
        assert_eq!(at(left), Some((past, Pos2::new(0.0, 316.0), false)));
        // Empty rows (nothing typed yet) run over nothing.
        assert_eq!(overflow(page, &[row(220.0, 220.0, 100.0)], &others), None);
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
            let (j, top) = job(f, "abcdef\ngh", &runs, LEADING);
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

    /// The text in a typing box, and what is selected there (from the anchor to the caret),
    /// after `key` with `modifiers` is pressed with the caret at `caret` in `text`.
    fn after_key(
        text: &str,
        caret: usize,
        key: egui::Key,
        modifiers: egui::Modifiers,
    ) -> (String, (usize, usize)) {
        let ctx = egui::Context::default();
        let id = egui::Id::unique("typing");
        let mut text = text.to_string();
        let frame = |events: Vec<egui::Event>, text: &mut String| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                let runs = |t: &str| {
                    vec![Run {
                        chars: t.chars().count(),
                        font: egui::FontId::proportional(20.0),
                        coords: VariationCoords::default(),
                        color: Color32::BLACK,
                        rise: 0.0,
                    }]
                };
                let style = Style {
                    runs: &runs,
                    mode: PageColours::Normal,
                    accent: Color32::BLUE,
                    warn: &|_| None,
                };
                let at = Pos2::new(100.0, 100.0);
                field(ui.ctx(), id, text, at, &style, true, None, false);
            });
            out.textures_delta.clear();
        };
        frame(vec![], &mut text);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(vec![], &mut text);
        select(&ctx, id, (caret, caret));
        let pressed = egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frame(vec![pressed], &mut text);
        frame(vec![], &mut text);
        let sel = selection(&ctx, id).expect("a caret");
        (text, sel)
    }

    /// The text design's 5.2: the arrows, Delete and Backspace go by what readers see as one
    /// character (a grapheme cluster), not by `char` as egui does: a letter typed with
    /// separate accents, a flag, a Hangul syllable typed in its parts. Backspace after a
    /// letter with separate accents takes off its last accent, as browsers' text boxes do,
    /// so a wrong accent can be typed again.
    #[test]
    fn the_caret_and_deleting_go_by_what_readers_see_as_one_character() {
        use egui::{Key, Modifiers as M};
        let cafe = "Cafe\u{301}";
        let gak = "\u{1100}\u{1161}\u{11A8}";
        let cases = [
            (cafe, 5, Key::ArrowLeft, M::NONE, cafe, (3, 3)),
            (cafe, 3, Key::ArrowRight, M::NONE, cafe, (5, 5)),
            (cafe, 5, Key::ArrowLeft, M::SHIFT, cafe, (5, 3)),
            ("Cafe\u{301}!", 3, Key::Delete, M::NONE, "Caf!", (3, 3)),
            (cafe, 5, Key::Backspace, M::NONE, "Cafe", (4, 4)),
            (
                "e\u{301}\u{308}",
                3,
                Key::Backspace,
                M::NONE,
                "e\u{301}",
                (2, 2),
            ),
            ("🇬🇧!", 2, Key::ArrowLeft, M::NONE, "🇬🇧!", (0, 0)),
            ("🇬🇧!", 0, Key::ArrowRight, M::NONE, "🇬🇧!", (2, 2)),
            ("🇬🇧", 2, Key::Backspace, M::NONE, "", (0, 0)),
            (gak, 3, Key::ArrowLeft, M::NONE, gak, (0, 0)),
            (gak, 3, Key::Backspace, M::NONE, "", (0, 0)),
            (gak, 0, Key::Delete, M::NONE, "", (0, 0)),
            // An emoji with its presentation selector goes whole: the selector isn't an
            // accent.
            ("❤\u{FE0F}", 2, Key::Backspace, M::NONE, "", (0, 0)),
        ];
        for (text, caret, key, m, want, sel) in cases {
            let got = after_key(text, caret, key, m);
            assert_eq!(got, (want.to_string(), sel), "{text:?} {key:?} {m:?}");
        }
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

    /// The text design's acceptance (10): a Latin word with a letter the chosen font lacks
    /// goes whole to another font, an ideograph alone, and the inspector's note names both,
    /// with the fonts core writes them in, in batch 31's words.
    #[test]
    fn the_note_names_the_words_that_go_to_another_font() {
        crate::font_list::test_family("Limare Han Test");
        let caveat = installed("Caveat");
        assert_eq!(fallback_note(&[(caveat, "Walrus ate fish")]), None);
        // Caveat has no Greek and no Chinese.
        let text = "say Ωmega or 漢字 now";
        let gone = limarepdf_core::fallbacks(caveat, text);
        let [(omega, f), (han, g)] = &gone[..] else {
            panic!("{gone:?}");
        };
        assert_eq!((omega.trim(), han.trim()), ("Ωmega", "漢字"));
        let (f, g) = (f.family_name(), g.family_name());
        let used = match f == g {
            true => f.clone(),
            false => format!("{f} and {g}"),
        };
        let (said, why) = fallback_note(&[(caveat, text)]).expect("a note");
        let want = format!("Not in Caveat: Ωmega 漢字. Those characters use {used}.");
        assert_eq!(said, want);
        assert_eq!(why, format!("Ωmega: {f}\n漢字: {g}"));
    }

    /// The typing box draws each word in the font core writes it in (the text design, 5.1):
    /// a word the chosen font lacks in the font it falls back to, the rest in the font
    /// chosen, so the caret goes where the text will be written.
    #[test]
    fn a_word_the_font_lacks_is_drawn_in_the_font_it_is_written_in() {
        crate::font_list::test_family("Limare Han Test");
        let serif = installed("Source Serif 4");
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        let part = TextPart {
            text: "Kan漢ji".into(),
            font: limarepdf_core::PartFont::Choice(serif),
            size: 12.0,
            color: [0.0; 3],
            rise: 0.0,
            from: None,
            origins: Vec::new(),
        };
        let gone = limarepdf_core::fallbacks(serif, "Kan漢ji");
        let [(han, other)] = &gone[..] else {
            panic!("{gone:?}");
        };
        assert_eq!(han, "漢");
        let parts = [(part, Wanted::Standard(serif))];
        let drawn = |ctx: &egui::Context| runs(ctx, &parts, 1.0, |_| Color32::BLACK, None);
        let fonts = [serif, *other, serif];
        let want = |ctx: &egui::Context| {
            (fonts.iter())
                .map(|&f| typing_fonts::family(ctx, Wanted::Standard(f)))
                .collect::<Vec<_>>()
        };
        // Each face loads on another thread: drawn in it once it is in.
        let done = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let got = drawn(&ctx);
            let chars: Vec<usize> = got.iter().map(|r| r.chars).collect();
            assert_eq!(chars, [3, 1, 2]);
            let families: Vec<_> = got.into_iter().map(|r| r.font.family).collect();
            // In the faces' own files, not their stand-ins.
            let file = |f: &egui::FontFamily| matches!(f, egui::FontFamily::Name(n) if n.starts_with("file "));
            let loaded = file(&families[0]) && file(&families[1]);
            if loaded || std::time::Instant::now() > done {
                assert!(loaded, "{families:?}");
                assert_eq!(families, want(&ctx));
                assert_ne!(families[0], families[1]);
                break;
            }
            ctx.run_ui(Default::default(), |_| {})
                .textures_delta
                .clear();
            std::thread::sleep(std::time::Duration::from_millis(20));
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

    /// Kerning pairs (AV, To, Te, WA), a ligature (fi) and an accented letter.
    const SET: &str = "AVATAR To Te fine WAVE café";

    /// Helvetica (drawn in its Liberation face, written kerned with its pairs) and installed
    /// fonts: Source Serif 4 (CFF, with ligatures) and Caveat (TrueType, variable).
    fn half_pixel_fonts() -> [FontChoice; 3] {
        [
            FontChoice::default(),
            installed("Source Serif 4"),
            installed("Caveat"),
        ]
    }

    /// [`SET`] written in `f` at 12 points from (20, 40) on a page of its own: the document's
    /// bytes, and where each character starts along the line, in points, as LimarePDF reads it
    /// back (and where the last one ends).
    fn written(f: FontChoice) -> (Vec<u8>, Vec<f32>) {
        let mut pdf = limarepdf_core::Pdf::new();
        pdf.insert_blank(0, 600.0, 200.0).unwrap();
        (pdf.add_text(0, (20.0, 40.0), SET, f, 12.0, [0.0; 3])).unwrap();
        let bytes = pdf.to_bytes().unwrap();
        let back = limarepdf_core::Pdf::from_bytes(&bytes, None).unwrap();
        let line = (back.page_objects(0).unwrap().into_iter())
            .find(|o| o.text.as_deref() == Some(SET))
            .expect("the line written");
        (bytes, line.stops)
    }

    /// Where the typing box draws each character of [`SET`] in `f` at 12 points, `k` pixels a
    /// point, that starts a glyph, once its face has loaded (a minute at most): its index, and
    /// how far along the line it starts, in pixels.
    fn drawn(ctx: &egui::Context, f: FontChoice, k: f32) -> Vec<(usize, f32)> {
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        let part = TextPart {
            text: SET.into(),
            font: limarepdf_core::PartFont::Choice(f),
            size: 12.0,
            color: [0.0; 3],
            rise: 0.0,
            from: None,
            origins: Vec::new(),
        };
        let parts = [(part, Wanted::Standard(f))];
        let done = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let runs = runs(ctx, &parts, k, |_| Color32::BLACK, None);
            // In the face's own file, or Helvetica's Liberation face, not a stand-in.
            let loaded = (runs.iter()).all(|r| match &r.font.family {
                egui::FontFamily::Name(n) if f.is_standard() => &**n == "Liberation Sans",
                egui::FontFamily::Name(n) => n.starts_with("file "),
                _ => false,
            });
            if loaded || std::time::Instant::now() > done {
                assert!(loaded, "{runs:?}");
                let (g, _) = galley(ctx, SET, &runs);
                let row = &g.rows[0];
                let (mut pen, mut out) = (0.0, Vec::new());
                for (i, g) in row.glyphs.iter().enumerate() {
                    if g.advance_width > 0.0 {
                        // epaint draws a glyph at its pen position to the nearest quarter
                        // pixel (its sub-pixel bins), the whole pixel in `pos`.
                        let at = (pen * 4.0_f32).round() / 4.0;
                        assert_eq!(g.pos.x, at.floor(), "character {i}'s pen at {pen}");
                        out.push((i, row.pos.x + at));
                    }
                    pen += g.advance_width;
                }
                return out;
            }
            ctx.run_ui(Default::default(), |_| {})
                .textures_delta
                .clear();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// The typing box draws text where it will be written (the text design's acceptance,
    /// 10): at 2 pixels a point, each character starting a glyph starts within half a pixel
    /// of where LimarePDF reads it back from the line written.
    #[test]
    fn each_character_is_drawn_within_half_a_pixel_of_where_it_is_written() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let k = 2.0;
        for f in half_pixel_fonts() {
            let (_, stops) = written(f);
            let at: Vec<(usize, f32, f32)> = (drawn(&ctx, f, k).into_iter())
                .map(|(i, x)| (i, x, stops[i] * k))
                .collect();
            assert!(
                at.iter().all(|&(_, x, want)| (x - want).abs() <= 0.5),
                "{} {SET:?}: each character's drawn and written place, in pixels: {at:?}",
                f.family_name()
            );
        }
    }

    /// The typing box's word starts against Poppler's reading of the line written: each
    /// word's start, as `pdftotext -bbox` gives it, within half a pixel at 2 pixels a point
    /// of where the typing box draws it. Opt-in: `cargo test -p limarepdf-gui --bin limarepdf
    /// -- --ignored cross_check`.
    #[test]
    #[ignore]
    fn cross_check_word_starts_with_pdftotext() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let k = 2.0;
        let dir = std::env::temp_dir().join(format!("limarepdf-half-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Where each word starts in `SET`, by character.
        let starts: Vec<usize> = (SET.split_word_bound_indices())
            .filter(|(_, w)| !w.trim().is_empty())
            .map(|(b, _)| SET[..b].chars().count())
            .collect();
        for f in half_pixel_fonts() {
            let (bytes, _) = written(f);
            let path = dir.join("line.pdf");
            std::fs::write(&path, bytes).unwrap();
            let out = std::process::Command::new("pdftotext")
                .args(["-bbox".as_ref(), path.as_os_str(), "-".as_ref()])
                .output()
                .expect("pdftotext (Poppler) installed");
            let html = String::from_utf8_lossy(&out.stdout);
            let mins: Vec<f32> = (html.split("<word xMin=\"").skip(1))
                .map(|w| w.split('"').next().unwrap().parse().unwrap())
                .collect();
            assert_eq!(mins.len(), starts.len(), "{}: {html}", f.family_name());
            let drawn = drawn(&ctx, f, k);
            for (&i, x_min) in starts.iter().zip(mins) {
                let (_, x) = drawn.iter().find(|d| d.0 == i).expect("its first glyph");
                let want = (x_min - 20.0) * k;
                assert!(
                    (x - want).abs() <= 0.5,
                    "{}: the word at {i} drawn at {x}, Poppler's at {want}",
                    f.family_name()
                );
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
