//! A paragraph of page text edited on the page as one (NEW-01; the paragraph design's
//! sections 5, 6.4, 8 and 9): its lines rewrapped in their frame as it is typed, drawn on
//! the lines core will write and in the fonts it will write them in, kept as one change when
//! the editing ends.
//!
//! Text opens as paragraphs while paragraph editing is on (Edit > Text flow's Paragraph, the
//! inspector's Editing row, the right-click menu or the command search, remembered between
//! runs: [`Editor::paragraphs`]), and a line at a time while it is off, as LimarePDF's check
//! of how well it finds paragraphs judged the default (testing.md, "Finding paragraphs":
//! [`PARAGRAPH_DEFAULT`]).

use super::replace_all::follow;
use super::*;
use limarepdf_core::{Block, DocWords, Doubt, ParagraphSetting, Pdf, SetParagraph};
use std::collections::HashSet;
use std::sync::Arc;

/// Whether a double-click, a Text-tool click or Edit text in a block LimarePDF is sure of
/// opens it as a paragraph with paragraph editing off. Off while the check of detection
/// fails on the answer sets (the paragraph design's 4.5).
pub(super) const PARAGRAPH_DEFAULT: bool = false;

/// A paragraph of page text being edited on the page.
pub(super) struct ParaEdit {
    pub page: usize,
    /// The line it was opened from, selected meanwhile.
    pub line: usize,
    pub block: Block,
    pub setting: Arc<ParagraphSetting>,
    /// What is typed, a style for each character, and the paragraph as it was.
    pub text: Styled,
    pub orig: Styled,
    /// Undo and redo inside the paragraph, until it is kept.
    pub history: History,
    /// What is selected when it opens.
    pub pick: ParaPick,
    /// Takes the keyboard on the next frame.
    pub fresh: bool,
    /// Takes the keyboard back once a change in the inspector is finished.
    pub refocus: bool,
    pub refused: page_text::Refused,
    /// Where it was drawn last on the screen, as typed: the floating bar keeps clear of it.
    pub area: Rect,
    /// What was typed before it had the keyboard (over words selected across its lines),
    /// given to it once it has it.
    pub typed: Vec<egui::Event>,
    /// Replace typed it: once that is typed, the keyboard goes back to Search results and
    /// the paragraph waits, open, as F6 leaves it ([`Editor::typed_over`]).
    pub park: bool,
    /// What an input method is composing in it, from and to ([`page_text::composition`]).
    pub composing: Option<(usize, usize)>,
    /// Line-end hyphens LimarePDF can't tell are the word's (6.4) that were kept as typed
    /// hyphens (Keep hyphen): where they were in the paragraph as read.
    pub kept_hyphens: Vec<usize>,
    /// The text as set last frame, on its lines.
    pub set: SetParagraph,
    /// Where the caret was last frame (page points, the middle of its line), and the
    /// character its line as drawn starts at: switching to One line edits the line there,
    /// the caret after the same characters.
    pub caret_at: Option<(Pos2, usize)>,
    /// The character a right-click was on, for the menu's Join and Keep hyphen.
    pub menu_at: Option<usize>,
    /// What was selected last frame (characters, from the anchor to the caret).
    pub sel: (usize, usize),
    /// The frame's left and right edges as dragged (page points), its own until then.
    pub edges: (f32, f32),
    /// How its text is aligned in the frame: its own until changed (NEW-15).
    pub align: TextAlign,
    /// An edge being dragged (0 left, 1 right), and the edges when the drag started.
    pub dragging: Option<(usize, (f32, f32))>,
    /// A link or comment is over the paragraph, its old lines or what is typed: the
    /// inspector says they stay in place.
    pub marked_over: bool,
}

/// What a paragraph editor selects when it opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ParaPick {
    /// These characters of the paragraph's text (from, to).
    Chars(usize, usize),
    /// The place between characters nearest this point (page points) as it is drawn; with
    /// `true`, the word around it.
    At(Pos2, bool),
}

/// What comes once the typing kept for a switch between Paragraph and One line is in the
/// document: the other editor, at the place the caret was (page, page points), the caret
/// after the text of its line before it, as the line will read, where that is known. The
/// characters, not the place, say where the caret goes: a font that comes in meanwhile
/// draws the line at another width (BUG-44). A line edited alone whose alignment was
/// pressed opens its paragraph so aligned (NEW-15).
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Then {
    Line(usize, Pos2, Option<String>),
    Paragraph(usize, Pos2, Option<String>, Option<TextAlign>),
}

/// The text an alignment left as it was, by why, for its notice.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NotAligned {
    /// Lines on their own, which have no paragraph to align them in.
    pub alone: usize,
    /// Lines in a table or a form.
    pub table: usize,
    /// Lines that don't run upright left to right.
    pub turned: usize,
    /// Invisible recognised text, text inside a group and vertical writing.
    pub left: Left,
    /// Paragraphs LimarePDF couldn't write again.
    pub failed: usize,
}

impl NotAligned {
    /// What was left as it was, for a notice; `None` when nothing was.
    pub fn said(&self) -> Option<String> {
        let lines = |n: usize, one: &str, many: &str| match n {
            1 => format!("1 line {one}"),
            n => format!("{n} lines {many}"),
        };
        let mut parts = Vec::new();
        if self.alone > 0 {
            parts.push(lines(self.alone, "on its own", "on their own"));
        }
        // The same words after one line and several.
        let counts = [
            (self.table, "in a table or form"),
            (self.turned, "of turned text"),
            (self.left.hidden, "of invisible recognised text"),
            (self.left.grouped, "inside a group"),
            (self.left.vertical, "of vertical writing"),
        ];
        for (n, why) in counts {
            if n > 0 {
                parts.push(lines(n, why, why));
            }
        }
        match self.failed {
            0 => {}
            1 => parts.push("1 paragraph LimarePDF couldn't write again".into()),
            n => parts.push(format!("{n} paragraphs LimarePDF couldn't write again")),
        }
        let last = parts.pop()?;
        Some(if parts.is_empty() {
            format!("Not aligned: {last}")
        } else {
            format!("Not aligned: {} and {last}", parts.join(", "))
        })
    }
}

/// What aligning paragraphs in the document did ([`align_paragraphs`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Aligned {
    /// Letters of a line written anew went to a similar font.
    pub similar: bool,
    /// Paragraphs that couldn't be read or written again, left as they were.
    pub failed: usize,
}

/// Each page's paragraphs `blocks` (as the page read when they were found, in reading
/// order) aligned as `align` says between their frames' edges (the frame LimarePDF kept,
/// else the block's: [`Pdf::paragraph`]), each line keeping its words
/// ([`ParagraphSetting::set_in`]): a write each, a paragraph after the first on a page found
/// again by its lines, which the writes before it left as they were. One that can't be read
/// or written again is left as it was and counted; when none can be, the first one's
/// reason is the error.
pub fn align_paragraphs(
    pdf: &mut Pdf,
    words: &DocWords,
    blocks: &[(usize, Vec<Block>)],
    align: TextAlign,
) -> limarepdf_core::Result<Aligned> {
    let mut done = Aligned::default();
    let (mut written, mut first) = (false, None);
    for (page, blocks) in blocks {
        let page = *page;
        let before = pdf.page_objects(page)?;
        for (n, b) in blocks.iter().enumerate() {
            let mut one = || -> limarepdf_core::Result<bool> {
                let block = match n {
                    0 => b.clone(),
                    _ => (refound(b, &follow(&before, &pdf.page_objects(page)?)))
                        .ok_or_else(|| anyhow::anyhow!("the page changed"))?,
                };
                let para = pdf.paragraph(page, &block, words)?;
                let setting = pdf.paragraph_setting(&para)?;
                let parts = Styled::of_paragraph(&para, font_of).parts();
                let edges = (para.frame.left, para.frame.right);
                Ok(!(pdf.edit_paragraph_in(&setting, &parts, edges, align)?).is_empty())
            };
            match one() {
                Ok(lost) => {
                    done.similar |= lost;
                    written = true;
                }
                Err(e) => {
                    done.failed += 1;
                    first.get_or_insert(e);
                }
            }
        }
    }
    match first {
        Some(e) if !written => Err(e),
        _ => Ok(done),
    }
}

/// Block `b` with each of its lines' objects where `now` says they are now ([`follow`]);
/// `None` when one of them changed or went.
pub(super) fn refound(b: &Block, now: &[Option<usize>]) -> Option<Block> {
    let mut block = b.clone();
    for l in &mut block.lines {
        for id in &mut l.pieces {
            *id = now.get(*id).copied().flatten()?;
        }
        l.pieces.sort_unstable();
    }
    Some(block)
}

/// Whether the selected text is, or would be, edited as a paragraph, whether it can be,
/// and why not ([`Editor::para_state`]): the inspector's Editing row.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParaState {
    /// A paragraph is being edited, or the line selected would open as one.
    pub on: bool,
    /// The selected line, or the one being edited, is in a block the switch can open as a
    /// paragraph.
    pub can: bool,
    /// Why it can't, for the switch's hover text.
    pub why: Option<&'static str>,
}

pub(super) fn para_id(scope: egui::Id, page: usize) -> egui::Id {
    scope.with(("paragraph-edit", page))
}

/// What hides each of a paragraph's old lines while it is edited (page points), as a line
/// being edited is hidden.
pub(super) fn covers(block: &Block) -> Vec<[f32; 4]> {
    block.lines.iter().map(|l| cover_round(l.bbox)).collect()
}

/// Why a block can't be edited as a paragraph, if it can't: it is one line, in a table, or
/// a turned line (the switch stays greyed and says so).
pub(super) fn refused(b: &Block) -> Option<&'static str> {
    match b.doubt {
        _ if b.lines.len() < 2 => Some(ON_ITS_OWN),
        Some(Doubt::OneLine) => Some(ON_ITS_OWN),
        Some(Doubt::Table) => Some("Text in a table edits a line at a time."),
        Some(Doubt::Turned) => Some(TURNED),
        _ => None,
    }
}

const ON_ITS_OWN: &str = "This line is on its own.";
const TURNED: &str = "Turned text edits a line at a time.";

/// The paragraph's text and styles as `parts` (as [`Styled::parts`] makes them) are
/// written, in pieces of one font each ([`ParagraphSetting::pieces`]), each with the font it
/// is drawn in: a part's own PDF font where its letters are kept, else the font its words go
/// to. For no text, the look typing takes in `text`.
fn para_pieces<'a>(
    setting: &'a ParagraphSetting,
    text: &Styled,
    parts: &[TextPart],
) -> Vec<(TextPart, Wanted<'a>)> {
    let para = setting.paragraph();
    let mut out = Vec::new();
    for piece in setting.pieces(parts) {
        let part = &parts[piece.part];
        let chars: Vec<char> = part.text.chars().collect();
        let sub = TextPart {
            text: chars[piece.chars.0..piece.chars.1].iter().collect(),
            from: None,
            origins: Vec::new(),
            ..part.clone()
        };
        let wanted = match piece.kept {
            Some(i) => {
                let name = para.spans.get(i).map_or("", |s| s.font_name.as_str());
                Wanted::Kept(name, setting.part_font(i))
            }
            None => Wanted::Standard(piece.away),
        };
        out.push((sub, wanted));
    }
    if out.is_empty() {
        for p in text.drawn_parts() {
            let f = text.font_of(&p);
            out.push((p, Wanted::Standard(f)));
        }
    }
    out
}

/// What the inspector says about how a paragraph's text is written, as `text` has it: the
/// words typed that go to another font than their part's own, as typed, as its font has no
/// code for a character in them, and the PDF fonts they were to be in, on hover.
fn para_font_note(setting: &ParagraphSetting, text: &Styled) -> Option<(String, String)> {
    let parts = text.parts();
    let para = setting.paragraph();
    let (mut words, mut used, mut names) = (Vec::<String>::new(), Vec::<String>::new(), Vec::new());
    for piece in setting.pieces(&parts) {
        let part = &parts[piece.part];
        let PartFont::Span(i) = part.font else {
            continue;
        };
        if piece.kept.is_some() {
            continue;
        }
        let chars: Vec<char> = part.text.chars().collect();
        let word: String = chars[piece.chars.0..piece.chars.1].iter().collect();
        let word = word.trim().to_string();
        if word.is_empty() {
            continue;
        }
        if !words.contains(&word) {
            words.push(word);
        }
        let f = piece.away.family_name();
        if !used.contains(&f) {
            used.push(f);
        }
        let name = (para.spans.get(i)).map(|s| untagged(&s.font_name).to_string());
        names.extend(name.filter(|n| !names.contains(n)));
    }
    if words.is_empty() {
        return None;
    }
    let note = format!(
        "Not in this PDF's font: {}. Those words use {}.",
        words.join(" "),
        used.join(", ")
    );
    Some((note, names.join(", ")))
}

/// Where the paragraph's text is written from, as `text` has it ([`source_font`]'s words
/// for a line): "Preserved", "Installed family", "Nearest standard", "Partly preserved",
/// "Mixed" or "Font chosen".
fn para_source(setting: &ParagraphSetting, text: &Styled) -> &'static str {
    let parts = text.parts();
    let mut found: Vec<&str> = Vec::new();
    for piece in setting.pieces(&parts) {
        if !matches!(parts[piece.part].font, PartFont::Span(_)) {
            continue;
        }
        let source = match piece.kept {
            Some(_) => "Preserved",
            None if !piece.away.is_standard() => "Installed family",
            None => "Nearest standard",
        };
        if !found.contains(&source) {
            found.push(source);
        }
    }
    match found[..] {
        [] => "Font chosen",
        [one] => one,
        _ if found.contains(&"Preserved") => "Partly preserved",
        _ => "Mixed",
    }
}

/// Ctrl+Z or Ctrl+Y as taken (`t`) in paragraph `pe` (the editor `id`): its typing, its
/// frame's edges and its alignment a step back or forward in its own history.
fn para_undo_redo(ctx: &egui::Context, id: egui::Id, t: &page_text::Taken, pe: &mut ParaEdit) {
    page_text::undo_redo(ctx, id, t, &mut pe.history, &mut pe.text);
    if t.undo || t.redo {
        pe.edges = pe.history.edges().unwrap_or(pe.edges);
        pe.align = pe.history.align().unwrap_or(pe.align);
    }
}

/// The frame's `edges` with edge `side` (0 left, 1 right) moved to `x` (page points), kept
/// on the page (`width` points wide) and no nearer the other edge than `narrowest` (its
/// widest word, [`ParagraphSetting::narrowest`]).
fn edge_to(edges: (f32, f32), side: usize, x: f32, narrowest: f32, width: f32) -> (f32, f32) {
    let (left, right) = edges;
    match side {
        0 => (x.min(right - narrowest).max(0.0), right),
        _ => (left, x.max(left + narrowest).min(width)),
    }
}

/// Take what an input method was still composing in paragraph `pe` as committed
/// ([`page_text::commit_left_styled`]), as when it loses the keyboard. Returns the first
/// character left out.
pub(super) fn commit_composition(pe: &mut ParaEdit) -> Option<String> {
    let r = pe.composing.take()?;
    let font = pe.text.typing_font(r);
    page_text::commit_left_styled(&mut pe.text, r, false, font)
}

impl ParaEdit {
    /// The unknown line-end hyphens a rewrap brought inside a line, not kept as typed
    /// hyphens: their characters in the text as typed.
    pub fn marked(&self) -> Vec<usize> {
        let para = self.setting.paragraph();
        let origins = self.text.origins();
        (self.set.marked.iter().copied())
            .filter(|&m| {
                let read = origins.get(m).copied().flatten();
                read.is_none_or(|k| !self.kept_hyphens.contains(&k))
            })
            .filter(|&m| m < origins.len())
            .filter(|_| !para.hyphens.is_empty())
            .collect()
    }

    /// The marked hyphen whose word character `at` is in.
    fn marked_at(&self, at: usize) -> Option<usize> {
        let text = self.text.text();
        self.marked().into_iter().find(|&m| {
            let (a, b) = word_around(text, m);
            (a.min(m)..=b.max(m + 1)).contains(&at)
        })
    }
}

impl Editor {
    /// The blocks of text on `page` ([`limarepdf_core::Pdf::page_blocks`]), read once until
    /// the document changes.
    fn blocks(&mut self, pdf: &limarepdf_core::Pdf, page: usize) -> &[Block] {
        self.blocks
            .entry(page)
            .or_insert_with(|| pdf.page_blocks(page).unwrap_or_default())
    }

    /// The block of `page` that line `id` is in, if any.
    fn block_of(&mut self, pdf: &limarepdf_core::Pdf, page: usize, id: usize) -> Option<Block> {
        let blocks = self.blocks(pdf, page);
        blocks.iter().find(|b| b.pieces().any(|p| p == id)).cloned()
    }

    /// The block of `page` line `id` is in, as far as the blocks have been read.
    fn read_block_of(&self, page: usize, id: usize) -> Option<&Block> {
        let blocks = self.blocks.get(&page)?;
        blocks.iter().find(|b| b.pieces().any(|p| p == id))
    }

    /// Whether line `id` of `page` opens as a paragraph: its block can be edited as one and
    /// paragraph editing is on, or LimarePDF is sure of it and that is the default
    /// ([`PARAGRAPH_DEFAULT`]).
    pub(super) fn opens_as_paragraph(
        &mut self,
        pdf: &limarepdf_core::Pdf,
        page: usize,
        id: usize,
    ) -> bool {
        let o = self.objs.get(&page).and_then(|v| v.get(id));
        if !o.is_some_and(drawn_in_place) {
            return false;
        }
        let Some(b) = self.block_of(pdf, page, id) else {
            return false;
        };
        refused(&b).is_none() && (self.paragraphs || PARAGRAPH_DEFAULT && b.confident())
    }

    /// Whether the selected line, or the one being edited, can be edited as a paragraph,
    /// and whether it is (being edited as one, or, selected, opening as one): for the
    /// inspector's switch and its hover text. The blocks are those read for the page
    /// ([`Editor::read_blocks`]).
    pub fn para_state(&self) -> ParaState {
        if self.para.is_some() {
            return ParaState {
                on: true,
                can: true,
                why: None,
            };
        }
        let none = ParaState {
            why: Some("Select a line of text first."),
            ..ParaState::default()
        };
        let Some((page, id)) = self.sel() else {
            return none;
        };
        let o = self.objs.get(&page).and_then(|v| v.get(id));
        if !o.is_some_and(|o| o.kind == ObjectKind::Text) {
            return none;
        }
        let why = match (o.is_some_and(drawn_in_place), self.read_block_of(page, id)) {
            (false, _) if o.is_some_and(|o| o.hidden) => {
                Some("Invisible text edits a line at a time.")
            }
            (false, _) => Some(TURNED),
            (true, None) if !self.blocks.contains_key(&page) => Some("Not available just now."),
            (true, None) => Some(ON_ITS_OWN),
            (true, Some(b)) => refused(b),
        };
        let can = why.is_none();
        ParaState {
            on: can && self.paragraphs && self.line_edit.is_none(),
            can,
            why,
        }
    }

    /// Read the blocks of `page` while one of its lines is selected, for the switch, or
    /// while words or lines selected there are, for the alignments' state.
    pub(super) fn read_blocks(&mut self, pdf: &limarepdf_core::Pdf, page: usize) {
        let line = self.sel().filter(|&(p, _)| p == page);
        let text = line.and_then(|(p, id)| self.objs.get(&p)?.get(id));
        let chosen = (self.chosen_now()).is_some_and(|c| c.line_ids().any(|(p, _)| p == page));
        if text.is_some_and(|o| o.kind == ObjectKind::Text && !o.hidden) || chosen {
            self.blocks(pdf, page);
        }
    }

    /// Whether a paragraph of page text is being edited on the page.
    pub fn editing_paragraph(&self) -> bool {
        self.para.is_some()
    }

    /// Whether there is text the alignment commands act on (NEW-15): a paragraph or line of
    /// page text being edited, words selected with the read tool, or lines of page text
    /// selected with the arrow. What can't be aligned among them is said when they act.
    pub fn aligns_text(&self) -> bool {
        let held = self.selection.objects().map_or(&[][..], |(_, held)| held);
        let lines = held.iter().any(|h| h.kind == ObjKind::Text);
        self.editing_line() || self.selection.words().is_some() || lines
    }

    /// How the text the alignment commands act on is aligned, when it is all aligned alike
    /// (NEW-15): the paragraph being edited, else the paragraphs of the other text they act
    /// on as far as their pages' blocks have been read ([`Self::read_blocks`]), those that
    /// can't be aligned left out.
    pub fn text_align(&self) -> Option<TextAlign> {
        if let Some(pe) = self.para.as_ref() {
            return Some(pe.align);
        }
        let mut aligns = (self.lines_to_align().into_iter()).filter_map(|(page, id)| {
            let o = self.objs.get(&page)?.get(id)?;
            let b = self.read_block_of(page, id).filter(|_| drawn_in_place(o))?;
            refused(b).is_none().then_some(b.frame.align)
        });
        let first = aligns.next()?;
        aligns.all(|a| a == first).then_some(first)
    }

    /// The lines of page text the alignment commands act on but for a paragraph being
    /// edited: the line edited alone, the lines of the words selected with the read tool or
    /// of the objects selected with the arrow, in reading order.
    fn lines_to_align(&self) -> Vec<(usize, usize)> {
        if let Some(le) = self.line_edit.as_ref() {
            return vec![(le.page, le.id)];
        }
        if let Some(c) = self.chosen_now() {
            return c.line_ids().collect();
        }
        let text = |&(page, id): &(usize, usize)| {
            let o = self.objs.get(&page).and_then(|v| v.get(id));
            o.is_some_and(|o| o.kind == ObjectKind::Text)
        };
        self.sel().into_iter().filter(text).collect()
    }

    /// Align the text the alignment commands act on as `align` says, as Word's alignment
    /// buttons do: Centre, Align right or Justify on text already so puts it back to left.
    /// The paragraph being edited rewraps in its frame at once, its typing open and the
    /// keyboard going back to it; its own Undo takes the change back, and it goes into the
    /// document with the typing. A line edited alone opens its paragraph so aligned, what
    /// was typed kept first and paragraph editing left as it is (batch 49's Q2). The
    /// paragraphs of the words selected with the read tool, or of the lines selected with
    /// the arrow, are aligned as one step, staying selected. Returns what to say of text left
    /// as it was: lines on their own, in a table or turned, and what [`Chosen`] leaves.
    pub fn align_text(&mut self, pdf: &Pdf, align: TextAlign) -> Option<String> {
        if let Some(pe) = self.para.as_mut() {
            pe.align = match align {
                a if a == pe.align => TextAlign::Left,
                a => a,
            };
            pe.refocus = true;
            return None;
        }
        if let Some((page, id)) = self.line_edit.as_ref().map(|le| (le.page, le.id)) {
            let mut left = NotAligned::default();
            match self.alignable(pdf, page, id, &mut left) {
                Some(block) => self.line_to_paragraph(&block, Some(align)),
                None => {
                    if let Some(le) = self.line_edit.as_mut() {
                        le.refocus = true;
                    }
                }
            }
            return left.said();
        }
        let mut left = NotAligned::default();
        if self.chosen_of().is_some() {
            self.sync_chosen(pdf);
            left.left = self.chosen.as_ref().map(|c| c.left).unwrap_or_default();
        }
        // Each paragraph once, in reading order.
        let (mut found, mut seen) = (Vec::<(usize, Block)>::new(), HashSet::new());
        for (page, id) in self.lines_to_align() {
            let Some(b) = self.alignable(pdf, page, id, &mut left) else {
                continue;
            };
            if seen.insert((page, b.pieces().next())) {
                found.push((page, b));
            }
        }
        // Word's rule over them all: the alignment they all have, pressed again, gives left.
        let all = !found.is_empty() && found.iter().all(|(_, b)| b.frame.align == align);
        let align = if all { TextAlign::Left } else { align };
        found.retain(|(_, b)| b.frame.align != align);
        if found.is_empty() {
            return left.said();
        }
        let mut pages: Vec<(usize, Vec<Block>)> = Vec::new();
        for (page, b) in found {
            match pages.last_mut() {
                Some((p, blocks)) if *p == page => blocks.push(b),
                _ => pages.push((page, vec![b])),
            }
        }
        let aligning = Action::AlignParagraphs(pages, align, left);
        self.actions.push(aligning);
        None
    }

    /// The block of `page` line `id` is in, when it can be aligned as a paragraph; else
    /// `None`, the line counted in `left` by why.
    fn alignable(
        &mut self,
        pdf: &Pdf,
        page: usize,
        id: usize,
        left: &mut NotAligned,
    ) -> Option<Block> {
        let o = self.objects(pdf, page).get(id)?;
        if o.hidden {
            left.left.hidden += 1;
            return None;
        }
        if o.vertical {
            left.left.vertical += 1;
            return None;
        }
        if !drawn_in_place(o) {
            left.turned += 1;
            return None;
        }
        let Some(b) = self.block_of(pdf, page, id) else {
            left.alone += 1;
            return None;
        };
        match refused(&b) {
            None => Some(b),
            Some(TURNED) => {
                left.turned += 1;
                None
            }
            Some(ON_ITS_OWN) => {
                left.alone += 1;
                None
            }
            Some(_) => {
                left.table += 1;
                None
            }
        }
    }

    /// End the line being edited, keeping what was typed, and once that is in the document
    /// open the paragraph of its block `block` at the caret's place, the caret after the
    /// same characters ([`Then::Paragraph`]), aligned as `align` says where given.
    fn line_to_paragraph(&mut self, block: &Block, align: Option<TextAlign>) {
        let Some(le) = self.line_edit.as_ref() else {
            return;
        };
        let at = le.caret_at.map_or_else(
            || {
                let b = block.lines[0].bbox;
                Pos2::new(b[0], (b[1] + b[3]) / 2.0)
            },
            |(at, _)| at,
        );
        // The line's text before the caret, as typed and written (on one line).
        let typed = self.insp.as_ref().map(|i| i.text.text());
        let before = le.caret_at.zip(typed).map(|((_, c), t)| {
            let flat = |c: char| if matches!(c, '\n' | '\r') { ' ' } else { c };
            t.chars().take(c).map(flat).collect()
        });
        self.then = Some(Then::Paragraph(le.page, at, before, align));
        self.end_line_edit();
    }

    /// Whether a handle of the frame of the paragraph being edited has the keyboard.
    pub fn edge_held(&self, ctx: &egui::Context) -> bool {
        let Some(pe) = self.para.as_ref() else {
            return false;
        };
        let id = para_id(self.scope, pe.page);
        let held = |s: usize| ctx.memory(|m| m.has_focus(page_text::edge_id(id, s)));
        held(0) || held(1)
    }

    /// How many line-end hyphens the paragraph being edited has marked.
    #[cfg(test)]
    pub fn marked_hyphens(&self) -> usize {
        self.para.as_ref().map_or(0, |pe| pe.marked().len())
    }

    /// The frame's left and right edges of the paragraph being edited (page points).
    #[cfg(test)]
    pub fn para_edges(&self) -> Option<(f32, f32)> {
        Some(self.para.as_ref()?.edges)
    }

    /// The text of the paragraph being edited, as typed, and what is selected in it.
    #[cfg(test)]
    pub fn para_text(&self) -> Option<(String, (usize, usize))> {
        let pe = self.para.as_ref()?;
        Some((pe.text.text().to_string(), pe.sel))
    }

    /// Edit the block of `page` line `id` is in as a paragraph, `pick` (on that line's
    /// text) selected. Whether it opened: a block that can't be read as one (a text mode
    /// that clips, a page changed meanwhile) or set up for editing doesn't.
    pub(super) fn start_paragraph(
        &mut self,
        pdf: &limarepdf_core::Pdf,
        page: usize,
        id: usize,
        pick: Pick,
    ) -> bool {
        let Some(block) = self.block_of(pdf, page, id) else {
            return false;
        };
        if refused(&block).is_some() {
            return false;
        }
        let words = self.doc_words.clone().unwrap_or_default();
        let Ok(para) = pdf.paragraph(page, &block, &words) else {
            return false;
        };
        let Ok(setting) = pdf.paragraph_setting(&para) else {
            return false;
        };
        // The line's characters as the paragraph has them.
        let line = self.objs.get(&page).and_then(|v| v.get(id));
        let text = line.and_then(|o| o.text.as_deref());
        let line_len = text.map_or(0, |t| t.chars().count());
        let at = |i: usize| -> Option<usize> {
            let n = para.text.chars().count();
            let of = |j: usize| (0..n).find(|&k| para.source(k) == Some((id, j)));
            match i >= line_len && line_len > 0 {
                true => of(line_len - 1).map(|k| k + 1),
                false => of(i),
            }
        };
        let pick = match pick {
            Pick::Chars(a, b) => match (at(a), at(b.max(a))) {
                (Some(a), Some(b)) => ParaPick::Chars(a, b),
                _ => ParaPick::Chars(0, 0),
            },
            Pick::At(p, word) => ParaPick::At(p, word),
        };
        if self.sel() != Some((page, id)) {
            self.select_object(page, id);
        }
        self.sync_insp();
        self.line_edit = None;
        self.cover = None;
        // The links and comments on the page, which the inspector says stay in place.
        self.annots(pdf, page);
        self.links
            .entry(page)
            .or_insert_with(|| pdf.page_links(page).unwrap_or_default());
        let text = Styled::of_paragraph(&para, font_of);
        let (edges, align) = ((para.frame.left, para.frame.right), para.frame.align);
        self.para = Some(ParaEdit {
            page,
            line: id,
            block,
            setting: Arc::new(setting),
            orig: text.clone(),
            history: History::with_frame(&text, edges, align),
            text,
            pick,
            fresh: true,
            refocus: false,
            refused: page_text::Refused::default(),
            area: Rect::NOTHING,
            typed: Vec::new(),
            park: false,
            composing: None,
            kept_hyphens: Vec::new(),
            set: SetParagraph::default(),
            caret_at: None,
            menu_at: None,
            sel: (0, 0),
            edges,
            align,
            dragging: None,
            marked_over: false,
        });
        true
    }

    /// Edit line `id` of `page` on the page with `pick` selected: as a paragraph when its
    /// block opens as one ([`Self::opens_as_paragraph`]), else the line alone. A point
    /// clicked is placed where the page draws the line's characters ([`on_line`]).
    pub(super) fn start_text_edit(
        &mut self,
        pdf: &limarepdf_core::Pdf,
        page: usize,
        id: usize,
        pick: Pick,
    ) {
        let line = self.objs.get(&page).and_then(|v| v.get(id));
        let pick = line.map_or(pick, |o| on_line(o, pick));
        if !(self.opens_as_paragraph(pdf, page, id) && self.start_paragraph(pdf, page, id, pick)) {
            self.start_line_edit(page, id, pick);
        }
    }

    /// The paragraph's change, typed and not yet in the document, as the change that makes
    /// it: none when it is as it was.
    pub(super) fn para_change(&self) -> Option<Action> {
        let pe = self.para.as_ref()?;
        let f = &pe.setting.paragraph().frame;
        let frame_as_it_was = pe.edges == (f.left, f.right) && pe.align == f.align;
        if pe.text.same_as(&pe.orig) && frame_as_it_was {
            return None;
        }
        let parts = pe.text.parts();
        // A change of underlines or strike-throughs alone sets no line anew.
        let decos = pe.text.decorations();
        let set = pe.setting.set_in(&parts, pe.edges, pe.align);
        if set.unchanged() && decos == pe.orig.decorations() {
            return None;
        }
        Some(Action::EditParagraph(
            pe.page,
            pe.setting.clone(),
            parts,
            decos,
            (pe.edges, pe.align),
        ))
    }

    /// Stop editing the paragraph on the page, keeping what was typed. With the arrow, or
    /// with `keep_selected`, its line stays selected; with another tool, as a word processor
    /// leaves no selection behind, nothing is.
    pub(super) fn end_paragraph(&mut self, keep_selected: bool) {
        let Some(line) = self.para.as_ref().map(|p| (p.page, p.line)) else {
            return;
        };
        self.keep_later(Typed::Paragraph);
        self.para = None;
        if !keep_selected && self.tool != Tool::Select && self.sel() == Some(line) {
            self.selection.clear_object();
        }
    }

    /// Paragraph editing on (`on`) or off: Edit > Text flow's Paragraph, the inspector's
    /// Editing row, the menus and the command search ([`Editor::paragraphs`]; the app keeps
    /// the choice). The text being edited follows: what was typed is kept first, as one
    /// change, and the other editor opens where the caret was; on, a line selected and not
    /// being edited opens as its paragraph, the caret at the line's end. A line that can't be
    /// a paragraph stays a line.
    pub fn set_paragraph(&mut self, pdf: &limarepdf_core::Pdf, on: bool) {
        self.paragraphs = on;
        if let Some(pe) = self.para.as_ref().filter(|_| !on) {
            let (page, block) = (pe.page, &pe.block);
            let at = pe.caret_at.map_or_else(
                || {
                    let l = &block.lines[0];
                    Pos2::new(l.bbox[0], (l.bbox[1] + l.bbox[3]) / 2.0)
                },
                |(at, _)| at,
            );
            // Its line's text before the caret, as written: a soft hyphen shows only at a
            // line's end.
            let before = pe.caret_at.map(|(_, from)| {
                let chars = pe.text.text().chars().skip(from);
                let chars = chars.take(pe.sel.1.saturating_sub(from));
                chars.filter(|&c| c != page_text::SOFT_HYPHEN).collect()
            });
            self.then = Some(Then::Line(page, at, before));
            self.end_paragraph(true);
            return;
        }
        if !on || self.para.is_some() {
            return;
        }
        let Some((page, id)) = self.sel() else {
            return;
        };
        let Some(block) = self.block_of(pdf, page, id) else {
            return;
        };
        if refused(&block).is_some() {
            return;
        }
        if self.line_edit.is_some() {
            self.line_to_paragraph(&block, None);
            return;
        }
        let end = (self.objs.get(&page).and_then(|v| v.get(id)))
            .and_then(|o| o.text.as_deref())
            .map_or(0, |t| t.chars().count());
        self.start_paragraph(pdf, page, id, Pick::Chars(end, end));
    }

    /// Open the editor a switch asked for ([`Then`]) once what was typed before it is in
    /// the document, at the line under the caret's place on `page`.
    pub(super) fn follow_then(&mut self, pdf: &limarepdf_core::Pdf, page: usize, k: f32) {
        let Some(then) = self.then.clone() else {
            return;
        };
        let (Then::Line(p, at, before) | Then::Paragraph(p, at, before, _)) = &then;
        let (p, at) = (*p, *at);
        if p != page || !self.later.is_empty() || !self.actions.is_empty() {
            return;
        }
        self.then = None;
        self.objects(pdf, page);
        let hit = self.text_hit(page, at, 3.0 / k.max(0.01));
        let Some(id) = hit.or_else(|| self.row_hit(page, at)) else {
            return;
        };
        // The caret after the same characters where the line reads as they say (spaces
        // that ended it dropped), else at the place.
        let text = (self.objs.get(&page).and_then(|v| v.get(id))).and_then(|o| o.text.as_deref());
        let after = before.as_deref().zip(text).and_then(|(b, t)| {
            let b = [b, b.trim_end()].into_iter().find(|b| t.starts_with(b))?;
            Some(b.chars().count())
        });
        let pick = after.map_or(Pick::At(at, false), |n| Pick::Chars(n, n));
        match then {
            Then::Line(..) => self.start_line_edit(page, id, pick),
            Then::Paragraph(.., align) => {
                if self.start_paragraph(pdf, page, id, pick) {
                    if let Some(a) = align {
                        self.align_text(pdf, a);
                    }
                    return;
                }
                self.start_line_edit(page, id, pick);
                // The line's paragraph, asked to be aligned, can't be read now: say why.
                if align.is_some() {
                    let mut left = NotAligned::default();
                    if self.alignable(pdf, page, id, &mut left).is_some() {
                        left.failed += 1;
                    }
                    self.align_note = left.said();
                }
            }
        }
    }

    /// The line of page text in the row of `p` on `page` nearest it: what a caret past a
    /// line's end was in, placed while the line was drawn in a stand-in font, wider or
    /// narrower than its own, which is still loading.
    fn row_hit(&self, page: usize, p: Pos2) -> Option<usize> {
        let objs = self.objs.get(&page)?;
        let gap = |b: [f32; 4]| (b[0] - p.x).max(p.x - b[2]).max(0.0);
        (objs.iter().enumerate())
            .filter(|(_, o)| o.kind == ObjectKind::Text && !o.hidden)
            .filter(|(_, o)| (o.bbox[1]..=o.bbox[3]).contains(&p.y))
            .min_by(|(_, a), (_, b)| gap(a.bbox).total_cmp(&gap(b.bbox)))
            .map(|(i, _)| i)
    }

    /// Open the paragraph of the words selected with the read tool, the words selected in
    /// it, when they are all in one block that can be a paragraph, with paragraph editing
    /// on, or, `across` its lines, one LimarePDF is sure of (stage 3's Q3): typing over them,
    /// or deleting them, rewraps it. Whether it opened.
    pub(super) fn words_paragraph(&mut self, pdf: &limarepdf_core::Pdf, across: bool) -> bool {
        self.sync_chosen(pdf);
        let Some(c) = self.chosen.as_ref() else {
            return false;
        };
        let ids: Vec<(usize, usize)> = c.line_ids().collect();
        let (Some(&(page, first)), Some(&(_, last))) = (ids.first(), ids.last()) else {
            return false;
        };
        // The selected characters of the first and last lines.
        let (Some((a, _)), Some((_, b))) = (c.selected_in(page, first), c.selected_in(page, last))
        else {
            return false;
        };
        if ids.iter().any(|&(p, _)| p != page) {
            return false;
        }
        let ids: Vec<usize> = ids.iter().map(|&(_, id)| id).collect();
        if !self.paragraph_over(pdf, page, &ids, (a, b), across) {
            return false;
        }
        self.selection.clear_words();
        true
    }

    /// Open the paragraph of `page` lines `ids` (in reading order) are all in, from
    /// character `a` of the first to `b` of the last selected, when it can be one, with
    /// paragraph editing on, or, `across` its lines, one LimarePDF is sure of. Whether it
    /// opened.
    pub(super) fn paragraph_over(
        &mut self,
        pdf: &limarepdf_core::Pdf,
        page: usize,
        ids: &[usize],
        (a, b): (usize, usize),
        across: bool,
    ) -> bool {
        let (Some(&first), Some(&last)) = (ids.first(), ids.last()) else {
            return false;
        };
        let Some(block) = self.block_of(pdf, page, first) else {
            return false;
        };
        let inside = ids.iter().all(|&id| block.pieces().any(|p| p == id));
        let sure = self.paragraphs || across && block.confident();
        if !inside || !sure || refused(&block).is_some() {
            return false;
        }
        if !self.start_paragraph(pdf, page, first, Pick::Chars(a, a)) {
            return false;
        }
        // The selection's end on its own line.
        let end = {
            let para = self.para.as_ref().map(|pe| pe.setting.paragraph());
            let n = para.map_or(0, |p| p.text.chars().count());
            let len = (self.objs.get(&page).and_then(|v| v.get(last)))
                .and_then(|o| o.text.as_deref())
                .map_or(0, |t| t.chars().count());
            para.and_then(|para| {
                let of = |j: usize| (0..n).find(|&k| para.source(k) == Some((last, j)));
                match b >= len && len > 0 {
                    true => of(len - 1).map(|k| k + 1),
                    false => of(b),
                }
            })
        };
        if let (Some(pe), Some(end)) = (self.para.as_mut(), end) {
            if let ParaPick::Chars(a, _) = pe.pick {
                pe.pick = ParaPick::Chars(a, end);
            }
        }
        true
    }

    /// The paragraph being edited on `page` (drawn at `r`, `k` pixels per point): drawn on
    /// the lines core will write it on, in the fonts it will write it in, over its old
    /// lines, hidden underneath. Esc, Tab or clicking away keeps the change; Enter starts a
    /// new paragraph in the frame and Shift+Enter a new line. Returns true when the page
    /// should not take the pointer this frame: it is over the editor, or the editor just
    /// opened or closed.
    pub(super) fn para_ui(&mut self, ui: &mut egui::Ui, page: usize, r: Rect, k: f32) -> bool {
        let Some((line, block)) = (self.para.as_ref())
            .filter(|pe| pe.page == page)
            .map(|pe| (pe.line, pe.block.clone()))
        else {
            return false;
        };
        if self.sel() != Some((page, line)) {
            self.end_paragraph(true);
            return false;
        }
        let to_screen = |x: f32, y: f32| Pos2::new(r.min.x + x * k, r.min.y + y * k);
        let on_screen =
            |b: [f32; 4]| Rect::from_min_max(to_screen(b[0], b[1]), to_screen(b[2], b[3]));
        let to_pt = |p: Pos2| Pos2::new((p.x - r.min.x) / k, (p.y - r.min.y) / k);
        // What the paragraph's rows may run over (PB-35): the page's other visible things
        // its old lines didn't already overlap (a box or picture behind it counts once the
        // text runs past it), not a background covering most of the page.
        let old = (block.lines.iter().map(|l| l.bbox)).reduce(union);
        let old = old.unwrap_or_default();
        let own: Vec<usize> = block.pieces().collect();
        let others = self.obstacles(page, r, k, |i, o| own.contains(&i) || meet(o.bbox, old));
        let warn = |rows: &[Rect]| page_text::overflow(r, rows, &others);
        // The links and comments on the page, whose places the inspector speaks of.
        let shown = |a: &&Annot| a.flags & 2 == 0;
        let annots = self.annots.get(&page).into_iter().flatten().filter(shown);
        let links = self.links.get(&page).into_iter().flatten();
        let marks: Vec<[f32; 4]> = annots
            .map(|a| a.bbox)
            .chain(links.map(|l| l.bbox))
            .collect();
        let page_w = r.width() / k;
        let ctx = ui.ctx().clone();
        let id = para_id(self.scope, page);
        let mode = self.page_colours;
        let lang = self.lang.clone();
        let accent = ui.visuals().selection.stroke.color;
        let hide = self.hiding(to_screen);
        let Some(pe) = self.para.as_mut() else {
            return false;
        };
        let typed_over = ctx.memory(|m| m.has_focus(id)) && !pe.typed.is_empty();
        if typed_over {
            // Typed as it opened, over what it selected: before the keys since, as typed.
            let typed = std::mem::take(&mut pe.typed);
            ctx.input_mut(|i| {
                i.events.splice(0..0, typed);
            });
        }
        if ctx.memory(|m| m.has_focus(id)) {
            // Before the editor sees the keys: what can't be written in the font it goes
            // into isn't typed; Ctrl+B and Ctrl+I switch what is selected; Enter starts a
            // new paragraph and Shift+Enter a new line, as word processors type them.
            let sel = selection(&ctx, id).unwrap_or((0, 0));
            let font = pe.text.typing_font(sel);
            let t = page_text::take_typing(ui, false, true, font);
            let shift = egui::Modifiers::SHIFT;
            let brk = ui.input_mut(|i| {
                if i.consume_key(shift, egui::Key::Enter) {
                    Some('\n')
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                    Some(page_text::PARAGRAPH_BREAK)
                } else {
                    None
                }
            });
            // Put in the text itself, over what is selected: egui types no line break from
            // a text event.
            if let Some(c) = brk {
                let (a, b) = (sel.0.min(sel.1), sel.0.max(sel.1));
                let chars: Vec<char> = pe.text.text().chars().collect();
                let (a, b) = (a.min(chars.len()), b.min(chars.len()));
                let mut s: String = chars[..a].iter().collect();
                s.push(c);
                s.extend(&chars[b..]);
                pe.text.sync(&s, Some(a + 1));
                page_text::select(&ctx, id, (a + 1, a + 1));
            }
            para_undo_redo(&ctx, id, &t, pe);
            page_text::switch_styles(&ctx, &t, &mut pe.text, sel, font, &mut pe.refused);
            if let Some(c) = t.refused {
                pe.refused.mark(&ctx, page_text::note(&c).to_string());
            }
        }
        // The frame's edges moved from the keyboard while a handle has it: Left and Right
        // a point, ten with Shift; Ctrl+Z and Ctrl+Y the paragraph's own, as in its text.
        let held = |s: usize| ctx.memory(|m| m.has_focus(page_text::edge_id(id, s)));
        if let Some(side) = [0, 1].into_iter().find(|&s| held(s)) {
            let keys = egui::EventFilter {
                horizontal_arrows: true,
                escape: true,
                ..Default::default()
            };
            ctx.memory_mut(|m| m.set_focus_lock_filter(page_text::edge_id(id, side), keys));
            para_undo_redo(&ctx, id, &page_text::take_undo(ui), pe);
            let step = ui.input_mut(|i| {
                let mut step = 0.0;
                for (key, d) in [(egui::Key::ArrowLeft, -1.0), (egui::Key::ArrowRight, 1.0)] {
                    // Shift first: a plain key's match takes it too.
                    let ten = i.count_and_consume_key(egui::Modifiers::SHIFT, key);
                    let one = i.count_and_consume_key(egui::Modifiers::NONE, key);
                    step += d * (10 * ten + one) as f32;
                }
                step
            });
            if step != 0.0 {
                let narrowest = pe.setting.narrowest_in(&pe.text.parts(), pe.align);
                let x = [pe.edges.0, pe.edges.1][side] + step;
                pe.edges = edge_to(pe.edges, side, x, narrowest, page_w);
            }
        }
        // The text as egui has it this frame, set on its lines in the faces it is written
        // in, between the frame's edges, aligned as it is.
        let (styled, setting, edges) = (pe.text.clone(), pe.setting.clone(), pe.edges);
        let align = pe.align;
        let laid = |t: &str| {
            let text = styled.preview(t);
            let parts = text.parts();
            let set = setting.set_in(&parts, edges, align);
            let pieces = para_pieces(&setting, &text, &parts);
            let colour = |c| page_text::shown(mode, c);
            let runs = page_text::runs(&ctx, &pieces, k, colour, lang.as_deref());
            page_text::paragraph_galley(&ctx, t, &runs, &set.lines, k, &to_screen)
        };
        let bands = |t: &str| {
            let text = styled.preview(t);
            let parts = text.parts();
            let set = setting.set_in(&parts, edges, align);
            let pieces = para_pieces(&setting, &text, &parts);
            let colour = |c| page_text::shown(mode, c);
            let runs = page_text::runs(&ctx, &pieces, k, colour, lang.as_deref());
            let bands = page_text::char_bands(&runs, &text.drawn_decos(), &colour);
            (bands, set.lines.iter().map(|l| l.stretch * k).collect())
        };
        let style = page_text::ParaStyle {
            laid: &laid,
            mode,
            accent,
            frame: (to_screen(edges.0, 0.0).x, to_screen(edges.1, 0.0).x),
            warn: &warn,
            bands: &bands,
        };
        // Each old line hidden, as a line being edited is.
        let covers: Vec<Rect> = covers(&block).into_iter().map(on_screen).collect();
        let mut typed = pe.text.text().to_string();
        let f = page_text::paragraph_field(&ctx, id, &mut typed, &style, &covers, hide);
        let out = f.out;
        // The styles follow what was typed.
        let sel = out
            .cursor_range
            .map(|c| (c.secondary.index.0, c.primary.index.0));
        if pe.text.sync(&typed, sel.map(|s| s.1)) {
            ctx.request_repaint();
        }
        if let Some(sel) = sel {
            pe.text.caret_at(sel);
            pe.sel = sel;
        }
        // An edge dragged by its handle, from where the press was, rewrapping as it goes; a
        // click on a handle gives it the keyboard.
        let handles = f.edges.as_ref();
        for (side, e) in handles.into_iter().flatten().enumerate() {
            if e.drag_started() {
                pe.dragging = Some((side, pe.edges));
            }
            if e.clicked() {
                e.request_focus();
            }
        }
        let pointer = ctx.input(|i| i.pointer.press_origin().zip(i.pointer.latest_pos()));
        if let (Some((side, start)), Some((a, b))) = (pe.dragging, pointer) {
            let e = handles.map(|h| &h[side]);
            if e.is_some_and(|e| e.dragged()) {
                let narrowest = pe.setting.narrowest_in(&pe.text.parts(), pe.align);
                let x = [start.0, start.1][side] + (b.x - a.x) / k;
                let now = edge_to(start, side, x, narrowest, page_w);
                if now != pe.edges {
                    pe.edges = now;
                    ctx.request_repaint();
                }
            }
            if e.is_none_or(|e| !e.dragged()) {
                pe.dragging = None;
            }
        }
        pe.set = setting.set_in(&pe.text.parts(), pe.edges, pe.align);
        let now = ctx.input(|i| i.time);
        pe.history.observe(&pe.text, sel, now, settled(&ctx));
        let dragging = pe.dragging.is_some();
        pe.history.observe_edges(pe.edges, now, dragging);
        pe.history.observe_align(pe.align);
        let on_edge = (handles.into_iter().flatten()).any(|e| e.has_focus());
        let over_edge = (handles.into_iter().flatten()).any(|e| e.hovered() || e.dragged());
        let edit = &out.response.response;
        // Where the caret is, and where its row starts, for a switch to One line; what a
        // right-click was on.
        let galley_at = out.galley_pos;
        if let Some(c) = out.cursor_range {
            let at = out.galley.pos_from_cursor(c.primary).center() + galley_at.to_vec2();
            let row = out.galley.layout_from_cursor(c.primary).row;
            let rows = out.galley.rows.iter().take(row);
            let from = rows.map(|r| r.char_count_including_newline().0).sum();
            pe.caret_at = Some((to_pt(at), from));
        }
        if edit.secondary_clicked() {
            pe.menu_at = (edit.interact_pointer_pos())
                .map(|p| out.galley.cursor_from_pos(p - galley_at).index.0);
        }
        // The hyphens LimarePDF can't tell are the word's, brought inside a line: dotted
        // under, as spelling marks are drawn.
        let ink = theme::tone(ui.visuals().dark_mode, theme::Tone::Caution).unwrap_or(accent);
        let cursor = |m: usize| egui::text::CCursor::new(egui::epaint::text::CharIndex(m));
        let layer = egui::LayerId::new(egui::Order::Foreground, id.with("marks"));
        let painter = ctx.layer_painter(layer);
        for m in pe.marked() {
            let from = out.galley.pos_from_cursor(cursor(m));
            let to = out.galley.pos_from_cursor(cursor(m + 1));
            let y = from.max.y - 1.0;
            let (a, b) = (from.min.x.min(to.min.x), from.min.x.max(to.min.x));
            let b = b.max(from.min.x + 3.0);
            let mut x = a - 1.0;
            while x <= b + 1.0 {
                painter.circle_filled(galley_at + Vec2::new(x, y), 0.8, ink);
                x += 2.5;
            }
        }
        // What an input method composes shows unchecked; left without the keyboard, it is
        // taken as committed (PB-54).
        pe.composing = page_text::composition(&ctx, id, pe.composing);
        if !edit.has_focus() {
            let left = commit_composition(pe);
            self.left_composing = self.left_composing.take().or(left);
        }
        self.typing_box = Some(edit.clone());
        // The floating bar keeps clear of the tag too.
        pe.area = f.outline.union(edit.rect);
        if let Some(tag) = f.tag {
            pe.area = pe.area.union(tag);
        }
        // A link or comment over its old lines or what is typed (its outline).
        let (a, b) = (to_pt(f.outline.min), to_pt(f.outline.max));
        let typed = [a.x, a.y, b.x, b.y];
        let over = |m: &[f32; 4]| meet(*m, typed) || block.lines.iter().any(|l| meet(*m, l.bbox));
        pe.marked_over = marks.iter().any(over);
        let area = pe.area;
        let words: Vec<&str> = pe.text.text().split_whitespace().take(8).collect();
        let first = words.join(" ");
        dialogs::set_name(edit, &format!("Editing a paragraph: {first}"));
        pe.refused.show(&ctx, &out, id);
        if pe.fresh {
            // The caret where clicked, or the words selected, as in a word processor.
            let (from, to) = match pe.pick {
                ParaPick::Chars(a, b) => (a, b),
                ParaPick::At(p, word) => {
                    let at = to_screen(p.x, p.y) - galley_at;
                    let i = out.galley.cursor_from_pos(at).index.0;
                    match word {
                        true => word_around(pe.text.text(), i),
                        false => (i, i),
                    }
                }
            };
            page_text::select(&ctx, id, (from, to));
            pe.history.observe(&pe.text, Some((from, to)), now, true);
            edit.request_focus();
            pe.fresh = false;
            return true;
        }
        let pointed = edit.hovered() || over_edge;
        // Replace's text is in: the paragraph waits, open, for Replace again or a click,
        // giving up the keyboard now, as it would be taken back if it still had it next
        // frame.
        if typed_over && std::mem::take(&mut pe.park) {
            self.parked = Some(Parked::Typing(id));
            self.typed_over = true;
            ctx.memory_mut(|m| m.surrender_focus(id));
            return pointed;
        }
        // Under a dialog it waits, open, for it to close, and takes the keyboard back then.
        if crate::dialogs::any_open(&ctx) {
            pe.refocus = true;
            return pointed;
        }
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if esc && (edit.lost_focus() || edit.has_focus() || on_edge) {
            // Esc keeps what was typed, as a click away does; Ctrl+Z takes it back.
            self.end_paragraph(false);
            return true;
        }
        // F6 took the keyboard to another region: it waits, open, for it to come back.
        if !edit.has_focus() && self.parked == Some(Parked::Typing(id)) {
            return pointed;
        }
        // A frame's handle has the keyboard: the paragraph stays open, as with the text.
        if on_edge && ui.clip_rect().intersects(area) {
            return pointed;
        }
        // The band's Font group, the inspector and the floating bar change the paragraph,
        // which takes the keyboard back once the change is finished.
        let bar = drawn_at(&ctx, self.bar_rect);
        let in_inspector = keeping(
            &ctx,
            Some(drawn_at(&ctx, self.insp_rect)),
            &[&self.keep_typing[..], &[bar]].concat(),
        );
        let picking = egui::Popup::is_any_open(&ctx);
        let Some(pe) = self.para.as_mut() else {
            return false;
        };
        if !edit.has_focus() && (pe.refocus || in_inspector || picking) {
            if settled(&ctx) && !picking && !in_inspector {
                page_text::give_back(&ctx, id);
                pe.refocus = false;
            } else {
                pe.refocus = true;
            }
            return pointed;
        }
        // Scrolled out of view, or the keyboard went elsewhere: keep what was typed.
        if !edit.has_focus() || !ui.clip_rect().intersects(pe.area) {
            self.end_paragraph(false);
            return false;
        }
        pointed
    }

    /// Join the marked hyphen (`all`: every one) in the paragraph being edited, its word
    /// made one; or keep it as a typed hyphen (Keep hyphen), no longer marked. With neither
    /// `all` nor a hyphen at the right-click or the caret, nothing changes.
    pub fn settle_hyphen(&mut self, ctx: &egui::Context, join: bool, all: bool) {
        let scope = self.scope;
        let Some(pe) = self.para.as_mut() else {
            return;
        };
        let id = para_id(scope, pe.page);
        let caret = pe.sel.1;
        let at = pe.menu_at.take().unwrap_or(caret);
        let marks: Vec<usize> = match all {
            true => pe.marked(),
            false => pe.marked_at(at).into_iter().collect(),
        };
        if marks.is_empty() {
            return;
        }
        if join {
            // Last first, so the places before stay.
            for &m in marks.iter().rev() {
                pe.text.delete((m, m + 1));
            }
            let before = marks.iter().filter(|&&m| m < caret).count();
            page_text::select(ctx, id, (caret - before, caret - before));
        } else {
            let origins = pe.text.origins();
            let read = (marks.iter()).filter_map(|&m| origins.get(m).copied().flatten());
            pe.kept_hyphens.extend(read);
        }
        pe.refocus = true;
    }

    /// Whether the paragraph being edited has a marked hyphen at the right-click or the
    /// caret, for the menu's Join and Keep hyphen.
    pub fn hyphen_here(&self) -> bool {
        let Some(pe) = self.para.as_ref() else {
            return false;
        };
        let at = pe.menu_at.unwrap_or(pe.sel.1);
        pe.marked_at(at).is_some()
    }

    /// The inspector's rows for the paragraph being edited: how it is edited (the
    /// Paragraph and One line switch), its look where the caret or selection is, its PDF
    /// fonts and how what is typed is written, and what goes to another font.
    pub(super) fn paragraph_inspector(&mut self, ui: &mut egui::Ui) {
        let scope = self.scope;
        if let Some(on) = editing_switch(ui, self.para_state()) {
            self.switch = Some(on);
        }
        let Some(pe) = self.para.as_mut() else {
            return;
        };
        let id = para_id(scope, pe.page);
        let n = pe.text.text().chars().count();
        let sel = page_text::held(ui.ctx(), id).unwrap_or((n, n));
        let text = InLine {
            text: &mut pe.text,
            sel,
        };
        let [font, size] = look_facts(&text);
        let para = pe.setting.paragraph();
        let (from, to) = (sel.0.min(sel.1), sel.0.max(sel.1));
        let (from, to) = match from == to {
            true => (from.saturating_sub(1), from.max(1)),
            false => (from, to),
        };
        let origins = pe.text.origins();
        let mut names: Vec<&str> = Vec::new();
        for k in (from..to).filter_map(|j| origins.get(j).copied().flatten()) {
            if let Some(s) = para.span_at(k).and_then(|i| para.spans.get(i)) {
                let name = untagged(&s.font_name);
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        let rows = [
            font,
            size,
            ("PDF font", names.join(", ")),
            ("Source font", para_source(&pe.setting, &pe.text).into()),
        ];
        facts(ui, "paragraph-look", &rows);
        if let Some((said, why)) = para_font_note(&pe.setting, &pe.text) {
            note(ui, &said).on_hover_text(why);
        }
        // Words a font chosen for them lacks, which go to an installed font.
        let parts = pe.text.parts();
        let chosen: Vec<(FontChoice, String)> = (parts.iter())
            .filter_map(|p| match p.font {
                PartFont::Choice(f) => Some((f, p.text.clone())),
                PartFont::Span(_) => None,
            })
            .collect();
        let chosen: Vec<(FontChoice, &str)> =
            chosen.iter().map(|(f, t)| (*f, t.as_str())).collect();
        if let Some((said, why)) = page_text::fallback_note(&chosen) {
            note(ui, &said).on_hover_text(why);
        }
        if pe.marked_over {
            note(ui, "Links and comments over this paragraph stay in place.");
        }
        let marked = pe.marked().len();
        if marked > 0 {
            ui.horizontal(|ui| {
                note(ui, format!("Line-end hyphens kept: {marked}")).on_hover_text(
                    "LimarePDF can't tell whether these hyphens belong to their words, so they \
                     stay as typed. Join a word from its right-click menu.",
                );
                note(ui, "·");
                let join = egui::RichText::new("Join all").size(12.0);
                if ui.link(join).clicked() {
                    self.hyphens_wanted = Some(true);
                }
            });
        }
    }
}

/// The inspector's Editing row for page text: Paragraph and One line, the one in use
/// pressed; Paragraph greyed, saying why on hover, for a line that can't be edited as one.
/// Returns the one pressed, when it isn't the one in use: the tab makes the switch
/// ([`Editor::switch`]).
pub(super) fn editing_switch(ui: &mut egui::Ui, state: ParaState) -> Option<bool> {
    let muted = theme::muted(ui.visuals().dark_mode);
    let mut asked = None;
    // A row as wide as the panel, not a grid of its own, which would widen it.
    ui.horizontal(|ui| {
        ui.set_min_height(17.0);
        ui.label(egui::RichText::new("Editing").size(12.0).color(muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            // Right to left: One line, then Paragraph before it.
            let small = |s: &str| egui::RichText::new(s).size(12.0);
            let one = ui.selectable_label(!state.on, small("One line"));
            let para = ui.add_enabled_ui(state.can || state.on, |ui| {
                ui.selectable_label(state.on, small("Paragraph"))
            });
            let para = match state.why {
                Some(why) => para.inner.on_disabled_hover_text(why),
                None => para.inner,
            };
            if para.clicked() && !state.on {
                asked = Some(true);
            }
            if one.clicked() && state.on {
                asked = Some(false);
            }
        });
    });
    ui.add_space(4.0);
    asked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What an alignment's notice says of the text it left as it was: each reason with how
    /// many lines (or paragraphs), in the order they are met, nothing when none was left.
    #[test]
    fn not_aligned_says_what_was_left_and_why() {
        assert_eq!(NotAligned::default().said(), None);
        let one = NotAligned {
            alone: 1,
            ..Default::default()
        };
        let said = "Not aligned: 1 line on its own";
        assert_eq!(one.said().as_deref(), Some(said));
        let many = NotAligned {
            alone: 2,
            table: 1,
            turned: 3,
            left: Left {
                hidden: 1,
                grouped: 0,
                vertical: 2,
            },
            failed: 2,
        };
        let said = "Not aligned: 2 lines on their own, 1 line in a table or form, 3 lines of \
                    turned text, 1 line of invisible recognised text, 2 lines of vertical \
                    writing and 2 paragraphs LimarePDF couldn't write again";
        assert_eq!(many.said().as_deref(), Some(said));
        let failed = NotAligned {
            failed: 1,
            ..Default::default()
        };
        let said = "Not aligned: 1 paragraph LimarePDF couldn't write again";
        assert_eq!(failed.said().as_deref(), Some(said));
    }
}
