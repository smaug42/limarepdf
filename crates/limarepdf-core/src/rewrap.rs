//! A block of a page's text read for editing as a paragraph (NEW-01; the paragraph design's
//! sections 6 and 7): its text as one, each character's style and where it was drawn, its
//! frame, and its line-end hyphens read from the document's own words (6.4).
//!
//! Positions are in points on the displayed page (y down), as [`crate::Block`]'s.

use crate::colour::Paint;
use crate::decorate::Decoration;
use crate::edit::{pieces, plan, Drawn, PartFont, TextPart, Way};
use crate::fonts::FontChoice;
use crate::layout::Setter;
use crate::matrix::{self as mx, apply, M};
use crate::objects::{PageObject, Parsed, TextData, TextSpan};
use crate::own_font::{LineFont, PageFonts};
use crate::para_frames::kept_for;
use crate::paragraph::{Align, Block, Frame};
use crate::{Pdf, Result};
use anyhow::{anyhow, bail};
use rayon::prelude::*;
use std::collections::HashMap;
use unicode_linebreak::{linebreaks, BreakOpportunity};
use unicode_segmentation::UnicodeSegmentation;

/// Two pieces of a line this far apart (in the smaller text's sizes) have a word's space
/// between them, as the shows of one text object are read.
const PIECE_GAP: f32 = 0.18;

/// A block of a page's text read for editing as a paragraph ([`Pdf::paragraph`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    /// The page it is on (0-based).
    pub page: usize,
    /// Its text: each line's pieces left to right, a space between two a word's space
    /// apart, and the lines joined by a space, none where a line ends or the next starts
    /// with one, after a line-end hyphen or dash, or between Chinese or Japanese characters.
    /// A line-end hyphen read as discretionary ([`Hyphen::Discretionary`]) is a soft
    /// hyphen.
    pub text: String,
    /// Its parts in one style each, as a line's ([`PageObject::spans`]), their characters
    /// in `text`: one for each part of each of its lines' text objects, the spaces put
    /// between them in the part before. A part's rise is from its line's baseline.
    pub spans: Vec<TextSpan>,
    /// Each line's characters in `text`, from and to; what joins two lines is in neither.
    pub lines: Vec<(usize, usize)>,
    pub frame: Frame,
    /// Its line-end hyphens, in order.
    pub hyphens: Vec<LineEndHyphen>,
    /// Its lines' underlines and strike-throughs ([`PageObject::decorations`]) over its
    /// text: each from its first character there to its last.
    pub decorations: Vec<Decoration>,
    /// One for each character of `text`.
    pub(crate) chars: Vec<Char>,
    /// One for each part ([`Paragraph::spans`]).
    pub(crate) parts: Vec<Part>,
    /// One for each line.
    pub(crate) rows: Vec<Row>,
    /// Its frame is the one LimarePDF wrote it in, read from its page's record
    /// ([`crate::para_frames`]): its indent is its own whatever it is aligned to.
    pub(crate) kept: bool,
}

/// A hyphen ending a line of a paragraph before a lower-case letter ([`Paragraph::hyphens`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineEndHyphen {
    /// The hyphen's character in [`Paragraph::text`].
    pub at: usize,
    pub kind: Hyphen,
}

/// What a line-end hyphen is, by the document's own words (the paragraph design's 6.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hyphen {
    /// Added to break the word: a soft hyphen, or the word appears joined elsewhere in the
    /// document and not hyphenated. Shown only while its word ends a line.
    Discretionary,
    /// Part of the word: it appears hyphenated elsewhere and not joined. Kept wherever the
    /// word goes.
    Hard,
    /// Neither, or both: kept wherever the word goes, and marked when a rewrap brings it
    /// inside a line.
    Unknown,
}

/// A character of a paragraph ([`Paragraph::chars`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Char {
    /// The text object (an id as in [`Pdf::page_objects`]) and the character of its text
    /// it is, or `None` for a space put between two pieces or two lines.
    pub from: Option<(usize, usize)>,
    /// Where its glyph starts and ends across the page; for a space put in, the gap.
    pub x: (f32, f32),
}

/// A part of a paragraph in one style ([`Paragraph::spans`]): the part of a text object's
/// line it is, and how it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Part {
    /// The text object, and the part of its line ([`PageObject::spans`]).
    pub object: usize,
    pub span: usize,
    /// Character and word spacing in points, and horizontal scaling (1 is none), as its
    /// first show is drawn with them.
    pub spacing: [f32; 3],
    /// Its font's name in the page's resources, and its em ([`crate::fonts::FontCtx::em`]).
    pub font: Vec<u8>,
    pub em: f32,
    /// What set its colour, to write it again in the same ink.
    pub paint: Paint,
}

/// A line of a paragraph ([`Paragraph::lines`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    /// Its text objects, in content order, each with its text as it was read: what an edit
    /// checks the page against.
    pub pieces: Vec<(usize, String)>,
    /// Where its baseline is down the page.
    pub baseline: f32,
}

impl Paragraph {
    /// The text object and character of its text that character `k` of [`Paragraph::text`]
    /// was drawn as, if it was drawn.
    pub fn source(&self, k: usize) -> Option<(usize, usize)> {
        self.chars.get(k)?.from
    }

    /// The part ([`Paragraph::spans`]) character `k` of [`Paragraph::text`] is in.
    pub fn span_at(&self, k: usize) -> Option<usize> {
        (self.spans.iter()).position(|s| s.chars.0 <= k && k < s.chars.1)
    }

    /// The paragraph's text as it is, in its parts' styles: what an edit starts from
    /// ([`ParagraphSetting::set`]), each part in the font of the paragraph's part it is
    /// ([`PartFont::Span`]), its characters where they are ([`TextPart::from`]).
    pub fn parts(&self) -> Vec<TextPart> {
        let chars: Vec<char> = self.text.chars().collect();
        (self.spans.iter().enumerate())
            .map(|(i, s)| TextPart {
                text: chars[s.chars.0..s.chars.1].iter().collect(),
                font: PartFont::Span(i),
                size: s.font_size,
                color: s.fill.unwrap_or([0.0; 3]),
                rise: s.rise,
                from: Some(s.chars.0),
                origins: Vec::new(),
            })
            .collect()
    }

    /// Whether characters `k` and `k + 1` of [`Paragraph::text`] were drawn one after the
    /// other in one text object: what is drawn between them is kept while they stay
    /// neighbours.
    pub(crate) fn neighbours(&self, k: usize) -> bool {
        let at = |k: usize| self.chars.get(k).and_then(|c| c.from);
        matches!((at(k), at(k + 1)), (Some((a, i)), Some((b, j))) if a == b && j == i + 1)
    }

    /// Whether character `k` of [`Paragraph::text`] is a spacing accent drawn over the
    /// letter after it, in another text object on its line, nothing put between them: the
    /// letter starts inside the accent's width, as TeX draws "ü" (PB-73). The move back
    /// between them as drawn is kept while they stay next to each other, as between
    /// [`Paragraph::neighbours`]; other pieces of a line keep the gap a word's text has.
    pub(crate) fn accent_over(&self, k: usize) -> bool {
        let (Some(c), Some(next)) = (self.chars.get(k), self.chars.get(k + 1)) else {
            return false;
        };
        let apart = matches!((c.from, next.from), (Some((a, _)), Some((b, _))) if a != b);
        let width = c.x.1 - c.x.0;
        let under = next.x.0 <= c.x.1 && next.x.0 >= c.x.0 - width;
        apart
            && under
            && self.lines.iter().any(|&(a, b)| a <= k && k + 1 < b)
            && self.text.chars().nth(k).is_some_and(spacing_accent)
    }

    /// What was drawn from the end of character `k`'s glyph to the start of character
    /// `k + 1`'s, across the page, in points.
    pub(crate) fn gap_after(&self, k: usize) -> f32 {
        self.chars[k + 1].x.0 - self.chars[k].x.1
    }
}

/// How wide a line may be in frame `f`, the first line of a paragraph (`first`) taking its
/// indent.
fn room_in(f: &Frame, first: bool) -> f32 {
    let indent = match (first, f.align) {
        (true, Align::Left | Align::Justified) => f.indent,
        _ => 0.0,
    };
    (f.right - f.left - indent).max(0.0)
}

/// A line of a paragraph being set, to place in a frame ([`Placing::on`]).
#[derive(Clone, Copy)]
struct Placing {
    /// It starts a paragraph, so it takes the first line's indent.
    opens: bool,
    /// How wide it is, the spaces it ends with left out.
    width: f32,
    /// The spaces between its words.
    gaps: usize,
    /// It ends the paragraph or a line typed to end there, so it isn't justified.
    last: bool,
}

impl Placing {
    /// Where the line starts across the page in frame `f`, and what justifying it there
    /// adds to each space between its words.
    fn on(&self, f: &Frame) -> (f32, f32) {
        let left = f.left + if self.opens { f.indent } else { 0.0 };
        match f.align {
            Align::Left => (left, 0.0),
            Align::Right => (f.right - self.width, 0.0),
            Align::Centre => ((f.left + f.right - self.width) / 2.0, 0.0),
            Align::Justified => {
                let stretch = match (self.last, self.gaps) {
                    (true, _) | (_, 0) => 0.0,
                    _ => ((room_in(f, self.opens) - self.width) / self.gaps as f32).max(0.0),
                };
                (left, stretch)
            }
        }
    }
}

/// Whether `c` is an accent drawn on its own, with a width of its own (Unicode's modifier
/// symbols): what TeX draws over a letter to make "ü" or "é" where its font has no such
/// letter.
fn spacing_accent(c: char) -> bool {
    "^`~\u{A8}\u{AF}\u{B4}\u{B8}\u{2C6}\u{2C7}".contains(c) || ('\u{2D8}'..='\u{2DD}').contains(&c)
}

/// The words of a document as its lines show them, counted: the evidence a line-end hyphen
/// is read by ([`Pdf::paragraph`]). Built once a document ([`Pdf::document_words`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocWords {
    counts: HashMap<String, u32>,
}

impl DocWords {
    /// The words of `lines`' text, counted: runs of letters, with hyphens inside (each line
    /// read without its soft hyphens).
    fn of<'a>(lines: impl IntoIterator<Item = &'a str>) -> Self {
        let mut counts: HashMap<String, u32> = HashMap::new();
        for line in lines {
            let chars: Vec<char> = line.chars().filter(|&c| c != '\u{AD}').collect();
            let mut k = 0;
            while k < chars.len() {
                match word_at(&chars, k) {
                    Some(end) => {
                        *counts.entry(folded(&chars[k..end])).or_default() += 1;
                        k = end;
                    }
                    None => k += 1,
                }
            }
        }
        Self { counts }
    }

    /// How many times `word` (folded, [`folded`]) appears.
    fn count(&self, word: &str) -> u32 {
        self.counts.get(word).copied().unwrap_or(0)
    }

    /// How many words are counted (each once).
    pub fn len(&self) -> usize {
        self.counts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }
}

/// A hyphen inside a word, of those a line's words are read with.
fn inner_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{2010}' | '\u{2011}')
}

/// Where the word starting at `chars[k]` ends: letters, with hyphens between them; `None`
/// when no word starts there (a letter must, after something that isn't one).
fn word_at(chars: &[char], k: usize) -> Option<usize> {
    let letter = |i: usize| chars.get(i).is_some_and(|c| c.is_alphabetic());
    if !letter(k) || (k > 0 && letter(k - 1)) {
        return None;
    }
    let mut end = k;
    while letter(end) || (chars.get(end).is_some_and(|&c| inner_hyphen(c)) && letter(end + 1)) {
        end += 1;
    }
    Some(end)
}

/// A word as words are compared: lower case, its hyphens plain.
fn folded(word: &[char]) -> String {
    (word.iter())
        .map(|&c| if inner_hyphen(c) { '-' } else { c })
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether `c` ends a line that joins the next without a space: a hyphen or a dash.
fn joining_mark(c: char) -> bool {
    matches!(
        c,
        '-' | '\u{AD}' | '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}'
    )
}

/// Whether `c` is Chinese or Japanese (ideographs, kana, their punctuation and full-width
/// forms), which lines join without a space.
fn unspaced(c: char) -> bool {
    matches!(c,
        '\u{2E80}'..='\u{2FDF}' | '\u{3000}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}'
        | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}'
        | '\u{FF00}'..='\u{FFEF}' | '\u{20000}'..='\u{3FFFF}')
}

impl Pdf {
    /// The words of the document's visible text as its lines show them, every page read
    /// (in parallel): what [`Pdf::paragraph`] reads line-end hyphens by. A page that can't
    /// be read gives none.
    pub fn document_words(&self) -> DocWords {
        let pages: Vec<Vec<String>> = (0..self.page_count())
            .into_par_iter()
            .map(|i| {
                let Ok(parsed) = self.parse_page(i) else {
                    return Vec::new();
                };
                (parsed.text_lines().filter(|t| !t.hidden))
                    .map(|t| t.text.clone())
                    .collect()
            })
            .collect();
        DocWords::of(pages.iter().flatten().map(String::as_str))
    }

    /// [`Pdf::document_words`] on `threads` threads of their own, so a long document's
    /// pages don't take every core from what else runs (the app's page drawing).
    pub fn document_words_on(&self, threads: usize) -> DocWords {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads.max(1));
        match pool.build() {
            Ok(pool) => pool.install(|| self.document_words()),
            Err(_) => self.document_words(),
        }
    }

    /// Block `block` of page `page_index` ([`Pdf::page_blocks`]) read for editing as a
    /// paragraph, its line-end hyphens read by `words` ([`Pdf::document_words`]). Refused
    /// when the page has changed since the block was found, or a line of it is turned,
    /// hidden, in vertical writing or in a text mode that clips. Its frame is the one
    /// LimarePDF wrote it in while its lines are where they were written (the page's record,
    /// [`crate::para_frames`]), else the block's.
    pub fn paragraph(
        &self,
        page_index: usize,
        block: &Block,
        words: &DocWords,
    ) -> Result<Paragraph> {
        let page = self.page_id_at(page_index)?;
        let parsed = self.parse_page(page_index)?;
        let objs = self.objects_of(page, &parsed)?;
        let (vd, _, _) = self.vis_down(page);
        let to_vis = mx::inv(vd).ok_or_else(|| anyhow!("degenerate page transform"))?;
        let mut para = read(page_index, &parsed, &objs, to_vis, block, words)?;
        if let Some(k) = kept_for(block, &self.kept_frames(page)) {
            (para.frame.left, para.frame.right, para.frame.indent) = (k.left, k.right, k.indent);
            para.kept = true;
        }
        Ok(para)
    }
}

/// A character of a line while a paragraph is read: what it is, where it came from, and
/// its part.
type Read = (char, Char, usize);

/// [`Pdf::paragraph`] of page `page`, parsed as `parsed`, its objects `objs`.
fn read(
    page: usize,
    parsed: &Parsed,
    objs: &[PageObject],
    to_vis: M,
    block: &Block,
    words: &DocWords,
) -> Result<Paragraph> {
    let mut parts: Vec<Part> = Vec::new();
    let mut part_spans: Vec<TextSpan> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut lines: Vec<Vec<Read>> = Vec::new();
    for v in &block.lines {
        let mut pieces: Vec<(&PageObject, &TextData)> = Vec::new();
        for &id in &v.pieces {
            let o = objs.get(id).ok_or_else(|| anyhow!("the page changed"))?;
            let t = (parsed.objs.get(id).and_then(|o| o.text.as_ref()))
                .ok_or_else(|| anyhow!("the page changed"))?;
            if !o.upright || o.hidden || o.vertical {
                bail!("a line of this block is turned, hidden or vertical");
            }
            if t.runs.iter().any(|r| r.state.tr >= 4) {
                bail!("a line of this block is drawn as a clipping path");
            }
            pieces.push((o, t));
        }
        rows.push(Row {
            pieces: (pieces.iter())
                .map(|(o, t)| (o.id, t.text.clone()))
                .collect(),
            baseline: v.baseline,
        });
        // Left to right, by where each piece's text starts.
        pieces.sort_by(|a, b| a.0.bbox[0].total_cmp(&b.0.bbox[0]));
        let mut line: Vec<Read> = Vec::new();
        for (o, t) in pieces {
            let xs = glyphs(t, to_vis);
            let chars: Vec<char> = t.text.chars().collect();
            if xs.len() != chars.len() {
                bail!("the positions of a line's characters couldn't be read");
            }
            let size = o.font_size.unwrap_or(v.size);
            // A word's space between this piece and the last.
            if let (Some(&(c, last, _)), Some(&first)) = (line.last(), chars.first()) {
                let size = size.min(v.size);
                let gap = xs[0].0 - last.x.1;
                if gap > PIECE_GAP * size && !c.is_whitespace() && !first.is_whitespace() {
                    let part = line.last().map_or(0, |l| l.2);
                    let x = (last.x.1, xs[0].0);
                    line.push((' ', Char { from: None, x }, part));
                }
            }
            let lift = o.baseline.map_or(0.0, |b| v.baseline - b.1);
            let lift = if lift.abs() < 0.02 * size { 0.0 } else { lift };
            let before = line.len();
            for (si, s) in o.spans.iter().enumerate() {
                let drawn = t.spans.get(si).ok_or_else(|| anyhow!("the page changed"))?;
                let spacing = (t.runs.get(drawn.runs.0)).map_or([0.0, 0.0, 1.0], |r| r.spacing);
                parts.push(Part {
                    object: o.id,
                    span: si,
                    spacing,
                    font: drawn.font.clone(),
                    em: drawn.em,
                    paint: drawn.paint.clone(),
                });
                part_spans.push(TextSpan {
                    rise: s.rise + lift,
                    ..s.clone()
                });
                let part = parts.len() - 1;
                for (k, &c) in chars.iter().enumerate().take(s.chars.1).skip(s.chars.0) {
                    let from = Some((o.id, k));
                    line.push((c, Char { from, x: xs[k] }, part));
                }
            }
            if line.len() - before != chars.len() {
                bail!("a line's characters aren't all in its parts");
            }
        }
        lines.push(line);
    }
    // The lines joined, each line-end hyphen read.
    let mut text: Vec<Read> = Vec::new();
    let mut ranges = Vec::new();
    let mut hyphens = Vec::new();
    for k in 0..lines.len() {
        let mut line = std::mem::take(&mut lines[k]);
        let next = (lines.get(k + 1)).map(|l| l.iter().map(|r| r.0).collect::<Vec<_>>());
        let hyphen = next.as_ref().and_then(|n| line_end_hyphen(&line, n, words));
        if let Some((at, _)) = hyphen {
            // A space put before a hyphen drawn apart (LibreOffice's) is no space.
            if at > 0 && line[at - 1].0 == ' ' && line[at - 1].1.from.is_none() {
                line.remove(at - 1);
            }
        }
        let start = text.len();
        let ends = line.iter().rposition(|r| !r.0.is_whitespace());
        if let Some((_, kind)) = hyphen {
            let at = start + ends.expect("a hyphen");
            if kind == Hyphen::Discretionary {
                line[at - start].0 = '\u{AD}';
            }
            hyphens.push(LineEndHyphen { at, kind });
        }
        let last = line.last().map(|r| (r.0, r.1.x.1, r.2));
        text.extend(line);
        ranges.push((start, text.len()));
        let first = next.as_ref().and_then(|n| n.first());
        let (Some((c, x, part)), Some(first)) = (last, first) else {
            continue;
        };
        let after_mark = (ends.map(|e| text[start + e].0)).is_some_and(joining_mark)
            && ends.is_some_and(|e| e > 0 && !text[start + e - 1].0.is_whitespace());
        let joined = c.is_whitespace()
            || first.is_whitespace()
            || after_mark
            || (unspaced(c) && unspaced(*first));
        if !joined {
            let between = Char {
                from: None,
                x: (x, x),
            };
            text.push((' ', between, part));
        }
    }
    let mut spans: Vec<TextSpan> = Vec::new();
    let mut span_parts: Vec<usize> = Vec::new();
    for (k, r) in text.iter().enumerate() {
        match span_parts.last() {
            Some(&p) if p == r.2 => spans.last_mut().expect("a part").chars.1 = k + 1,
            _ => {
                spans.push(TextSpan {
                    chars: (k, k + 1),
                    ..part_spans[r.2].clone()
                });
                span_parts.push(r.2);
            }
        }
    }
    let mut para = Paragraph {
        page,
        text: text.iter().map(|r| r.0).collect(),
        spans,
        lines: ranges,
        frame: block.frame,
        hyphens,
        decorations: Vec::new(),
        chars: text.iter().map(|r| r.1).collect(),
        parts: span_parts.into_iter().map(|p| parts[p].clone()).collect(),
        rows,
        kept: false,
    };
    para.decorations = decorations_of(&para, &parsed.decorations);
    Ok(para)
}

/// The decorations of paragraph `para`'s lines, of the page's `decorations` (each with its
/// line), over its text: each from its first character there to its last.
pub(crate) fn decorations_of(
    para: &Paragraph,
    decorations: &[(usize, Decoration)],
) -> Vec<Decoration> {
    let n = para.text.chars().count();
    let mut out = Vec::new();
    for (line, d) in decorations {
        let over = |k: &usize| {
            (para.source(*k)).is_some_and(|(o, c)| o == *line && d.chars.0 <= c && c < d.chars.1)
        };
        let ks: Vec<usize> = (0..n).filter(over).collect();
        if let (Some(&a), Some(&b)) = (ks.first(), ks.last()) {
            out.push(Decoration {
                chars: (a, b + 1),
                ..d.clone()
            });
        }
    }
    out
}

/// The hyphen ending line `line` before line `next` (its characters), if it ends in one
/// after a word and `next` starts with a lower-case word: its place in `line` (after the
/// spaces put between pieces), and what the document's words say it is.
fn line_end_hyphen(line: &[Read], next: &[char], words: &DocWords) -> Option<(usize, Hyphen)> {
    let at = line.iter().rposition(|r| !r.0.is_whitespace())?;
    let mark = line[at].0;
    if !matches!(mark, '-' | '\u{2010}' | '\u{AD}') {
        return None;
    }
    // The word before it, a space put between pieces allowed between them.
    let chars: Vec<char> = line[..at].iter().map(|r| r.0).collect();
    let mut end = chars.len();
    if end > 0 && chars[end - 1] == ' ' && line[end - 1].1.from.is_none() {
        end -= 1;
    }
    let ends_here = |k: &usize| word_at(&chars[..end], *k).is_some_and(|e| e == end);
    let start = (0..end).rev().find(ends_here)?;
    let skip = next.iter().take_while(|c| c.is_whitespace()).count();
    let after = word_at(next, skip)?;
    if !next[skip].is_lowercase() {
        return None;
    }
    if mark == '\u{AD}' {
        return Some((at, Hyphen::Discretionary));
    }
    let (prefix, rest) = (folded(&chars[start..end]), folded(&next[skip..after]));
    let joined = words.count(&format!("{prefix}{rest}"));
    let hyphenated = words.count(&format!("{prefix}-{rest}"));
    let kind = match (joined > 0, hyphenated > 0) {
        (true, false) => Hyphen::Discretionary,
        (false, true) => Hyphen::Hard,
        _ => Hyphen::Unknown,
    };
    Some((at, kind))
}

/// Where each character of line `t` starts and ends across the displayed page (`to_vis`
/// from page user space), its glyph's extent; characters a piece's widths aren't known for
/// spread evenly over it.
fn glyphs(t: &TextData, to_vis: M) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(t.text.len());
    for p in &t.pieces {
        let n = p.chars.1 - p.chars.0;
        let (a, b) = (p.corners[0], p.corners[1]);
        let x = |f: f32| apply(to_vis, a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f).0;
        for j in 0..n {
            let (f0, f1) = match p.spans.len() == n {
                true => p.spans[j],
                false => (j as f32 / n as f32, (j + 1) as f32 / n as f32),
            };
            out.push((x(f0), x(f1)));
        }
    }
    out
}

// ---- Rewrapping (the paragraph design's section 6).

/// A paragraph's text set in its frame as it is edited ([`Pdf::paragraph_setting`]): its
/// fonts read once, then each edit's lines laid out ([`ParagraphSetting::set`]).
#[derive(Debug)]
pub struct ParagraphSetting {
    para: Paragraph,
    chars: Vec<char>,
    /// Each part's font as it takes new text, and the font its words go to where it can't
    /// write them ([`FontChoice::of_font`]).
    fonts: Vec<(LineFont, FontChoice)>,
    /// How each of the paragraph's characters was drawn, by its place in the paragraph's
    /// text, when that is known.
    drawn: Vec<Option<Drawn>>,
    lang: Option<String>,
    /// A space where a part's font has none to write, in sizes.
    space_em: f32,
    /// The paragraph as it is, measured.
    old: Measured,
}

/// A line of a paragraph as an edit sets it ([`ParagraphSetting::set`]).
#[derive(Clone, Debug, PartialEq)]
pub struct SetLine {
    /// Its characters in the edited text, from and to, the spaces it ends with among them.
    pub chars: (usize, usize),
    /// The paragraph's line it is, kept as it was; `None` for a line written anew.
    pub kept: Option<usize>,
    /// Where it starts across the page, and its baseline down it.
    pub x: f32,
    pub baseline: f32,
    /// How wide it is, the spaces it ends with left out (justified, before the stretch).
    pub width: f32,
    /// What justifying it adds to each space between its words, in points.
    pub stretch: f32,
    /// It ends in a soft hyphen, which shows.
    pub hyphen: bool,
}

/// A stretch of an edit's part written in one font ([`ParagraphSetting::pieces`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParagraphPiece {
    /// The edit's part it is in, and its characters there, from and to.
    pub part: usize,
    pub chars: (usize, usize),
    /// Written in the font of the paragraph's part with this index ([`Paragraph::spans`]),
    /// its letters keeping their codes; `None` when written in `away`.
    pub kept: Option<usize>,
    /// The font its words go to where it isn't kept (a part in a font chosen: that font).
    pub away: FontChoice,
}

/// An edited paragraph set in its frame ([`ParagraphSetting::set`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetParagraph {
    pub lines: Vec<SetLine>,
    /// The line-end hyphens that may or may not belong to their word ([`Hyphen::Unknown`])
    /// a rewrap brought inside a line: their characters in the edited text, to mark.
    pub marked: Vec<usize>,
}

impl SetParagraph {
    /// Whether every line is kept as it was: nothing to write.
    pub fn unchanged(&self) -> bool {
        self.lines.iter().all(|l| l.kept.is_some())
    }
}

/// An edited paragraph's text measured as it is written ([`ParagraphSetting::measure`]).
#[derive(Clone, Debug, Default)]
struct Measured {
    chars: Vec<char>,
    /// Each character's advance in points; a soft hyphen's none.
    widths: Vec<f32>,
    /// What each soft hyphen adds when it ends a line and shows; 0 for the others.
    shown: Vec<f32>,
    /// Each character's size.
    sizes: Vec<f32>,
    /// Where each character was in the paragraph's text, if it is the paragraph's own
    /// there in the style it had ([`TextPart::origins`]).
    same: Vec<Option<usize>>,
    /// Where each was, whatever its style.
    from: Vec<Option<usize>>,
    /// Where a line may end: before each of these characters (the text's end the last),
    /// and whether it must.
    breaks: Vec<(usize, bool)>,
}

impl Measured {
    /// How wide characters `a..b` are as a line: the spaces they end with left out, a soft
    /// hyphen ending them shown.
    fn width(&self, a: usize, b: usize) -> f32 {
        let end = self.trimmed(a, b);
        let mut w: f32 = self.widths[a..end].iter().sum();
        if end > a && self.chars[end - 1] == '\u{AD}' {
            w += self.shown[end - 1];
        }
        w
    }

    /// Where characters `a..b` end without the spaces (and line breaks) they end with.
    fn trimmed(&self, a: usize, b: usize) -> usize {
        let mut end = b;
        while end > a && self.chars[end - 1].is_whitespace() {
            end -= 1;
        }
        end
    }

    /// Where a line from character `a` that may be `room` wide ends, breaking where
    /// Unicode's rule allows (UAX #14) and taking what fits, as Word and Google Docs break; a
    /// word wider than the line is broken between characters. At least one character.
    fn line_end(&self, a: usize, room: f32) -> usize {
        let mut best = None;
        for &(b, must) in self.breaks.iter().filter(|(b, _)| *b > a) {
            if self.width(a, b) <= room + 1e-3 {
                best = Some(b);
                if must {
                    break;
                }
                continue;
            }
            if best.is_some() {
                break;
            }
            // Nothing fits: as many of the word's clusters as do.
            let text: String = self.chars[a..b].iter().collect();
            let mut end = a;
            let mut k = a;
            for g in text.graphemes(true) {
                k += g.chars().count();
                if end > a && self.width(a, k) > room + 1e-3 {
                    break;
                }
                end = k;
            }
            return end.max(a + 1);
        }
        best.unwrap_or(self.chars.len()).max(a + 1)
    }
}

impl Pdf {
    /// Paragraph `para` ([`Pdf::paragraph`]) set up for editing: its fonts read as they take
    /// new text, and the font each part's words go to where it can't write them (which
    /// reads the installed fonts' list the first time). Refused when the page has changed.
    pub fn paragraph_setting(&self, para: &Paragraph) -> Result<ParagraphSetting> {
        let page = self.page_id_at(para.page)?;
        let parsed = self.parse_page(para.page)?;
        for (id, text) in para.rows.iter().flat_map(|r| &r.pieces) {
            let now = parsed.objs.get(*id).and_then(|o| o.text.as_ref());
            if now.map(|t| &t.text) != Some(text) {
                bail!("the page changed");
            }
        }
        let res = self.font_resources(page);
        let mut read = PageFonts::new(&self.doc, &res);
        let mut of: HashMap<usize, Vec<LineFont>> = HashMap::new();
        for part in &para.parts {
            if let Some(t) = parsed.objs[part.object].text.as_ref() {
                of.entry(part.object).or_insert_with(|| read.of(&parsed, t));
            }
        }
        let fonts: Vec<(LineFont, FontChoice)> = (para.parts.iter().zip(&para.spans))
            .map(|(part, s)| {
                let font = of.get(&part.object).and_then(|f| f.get(part.span)).cloned();
                let font = font.ok_or_else(|| anyhow!("the page changed"))?;
                Ok((font, FontChoice::of_font(&s.font_name)))
            })
            .collect::<Result<_>>()?;
        let drawn = (para.chars.iter().enumerate())
            .map(|(k, c)| {
                let (o, at) = c.from?;
                let d = of.get(&o)?.first()?.drawn()?.get(at)?;
                Some(d.moved(at, k))
            })
            .collect();
        let chars: Vec<char> = para.text.chars().collect();
        // A space where a font has none: the narrower half of the paragraph's spaces.
        let mut spaces: Vec<f32> = (chars.iter().zip(&para.chars).enumerate())
            .filter(|(_, (c, ch))| **c == ' ' && ch.from.is_some())
            .filter_map(|(k, (_, ch))| {
                let size = para.spans.get(para.span_at(k)?)?.font_size;
                (size > 0.0).then(|| (ch.x.1 - ch.x.0) / size)
            })
            .collect();
        spaces.sort_by(f32::total_cmp);
        let space_em = spaces.get(spaces.len() / 4).copied().unwrap_or(0.25);
        let mut setting = ParagraphSetting {
            para: para.clone(),
            chars,
            fonts,
            drawn,
            lang: self.language(),
            space_em,
            old: Measured::default(),
        };
        setting.old = setting.measure(&para.parts());
        Ok(setting)
    }
}

impl ParagraphSetting {
    pub fn paragraph(&self) -> &Paragraph {
        &self.para
    }

    /// The font of the paragraph's part `i`, as it takes new text.
    pub(crate) fn font(&self, i: usize) -> &LineFont {
        &self.fonts[i].0
    }

    /// The font of the paragraph's part `i` ([`Paragraph::spans`]) as it takes new text,
    /// for drawing what is typed in it as it will be written.
    pub fn part_font(&self, i: usize) -> Option<&LineFont> {
        self.fonts.get(i).map(|f| &f.0)
    }

    /// How `parts` (as [`ParagraphSetting::set`] takes them) are written, in stretches of
    /// one font each, in order: kept in the font of the paragraph's part they are in where
    /// they can be, else in the font their words go to. What the typing box draws each
    /// stretch in.
    pub fn pieces(&self, parts: &[TextPart]) -> Vec<ParagraphPiece> {
        let mut out: Vec<ParagraphPiece> = Vec::new();
        for pl in self.planned(parts) {
            let kept = pl.kept.map(|k| k.0);
            match out.last_mut() {
                Some(p) if p.part == pl.part && p.kept == kept && p.away == pl.away => {
                    p.chars.1 = pl.at + 1;
                }
                _ => out.push(ParagraphPiece {
                    part: pl.part,
                    chars: (pl.at, pl.at + 1),
                    kept,
                    away: pl.away,
                }),
            }
        }
        out
    }

    /// How character `k` of the paragraph's text was drawn, when that is known.
    pub(crate) fn drawn(&self, k: usize) -> Option<&Drawn> {
        self.drawn.get(k)?.as_ref()
    }

    /// A space where a part's font has none to write, in sizes.
    pub(crate) fn space_em(&self) -> f32 {
        self.space_em
    }

    /// The paragraph's text edited to `parts` (its parts' fonts naming the paragraph's
    /// parts, [`Paragraph::spans`], their origins its text, [`Paragraph::parts`]) set in its
    /// frame, as the paragraph design's section 6 sets it: the lines before the first one
    /// the edit can change kept, then each line taking the words that fit, until a line
    /// ends where one did with the rest unchanged, whose lines are kept. The line before
    /// the edit takes words only when its next line's first word fits now and didn't
    /// before, and a line ends where it did when the words after that would have fitted
    /// before too: the producer's breaks stand where the edit doesn't move them.
    pub fn set(&self, parts: &[TextPart]) -> SetParagraph {
        let f = &self.para.frame;
        self.set_between(parts, (f.left, f.right))
    }

    /// [`ParagraphSetting::set`] with the frame's left and right edges at `edges` (across
    /// the page, in points), as they are dragged. Where they moved, every line is broken
    /// anew to the frame's new width, as a word processor rewraps a resized text box: a
    /// line stays as it was only where it starts and ends as it did in text that hasn't
    /// changed, and the frame moves it nowhere, across the page or in its stretch.
    pub fn set_between(&self, parts: &[TextPart], edges: (f32, f32)) -> SetParagraph {
        self.set_in(parts, edges, self.para.frame.align)
    }

    /// [`ParagraphSetting::set_between`] with the paragraph aligned as `align` says
    /// (NEW-15). A change of alignment alone breaks no line anew, as a word processor
    /// realigns a paragraph: each line keeps its words and is placed again in the frame,
    /// written anew where that moves it or changes its stretch. Centred and right-aligned
    /// text takes no first-line indent; left or justified, a paragraph takes its own (none
    /// for one read as centred or right-aligned).
    pub fn set_in(&self, parts: &[TextPart], edges: (f32, f32), align: Align) -> SetParagraph {
        let new = self.measure(parts);
        let old = &self.old;
        let para = &self.para;
        let frame = self.frame_in(edges, align);
        let moved = (frame.left - para.frame.left).abs() > 1e-3
            || (frame.right - para.frame.right).abs() > 1e-3;
        let realigned = align != para.frame.align;
        let (nl, ol) = (new.chars.len(), old.chars.len());
        let starts: Vec<usize> = para.lines.iter().map(|l| l.0).collect();
        let rows = starts.len();
        let prefix = (0..nl.min(ol))
            .take_while(|&j| new.same[j] == Some(j))
            .count();
        let suffix = (0..nl.min(ol) - prefix)
            .take_while(|&i| new.same[nl - 1 - i] == Some(ol - 1 - i))
            .count();
        // Where the text after a character of the edited text is unchanged, the place it
        // was in the paragraph's text.
        let was = |j: usize| (j >= nl - suffix).then(|| j + ol - nl);
        // The old lines' ends in their own text.
        let old_end = |n: usize| starts.get(n + 1).copied().unwrap_or(ol);
        let kept = |n: usize, shift: isize| -> SetLine {
            let (a, b) = (starts[n], old_end(n));
            let at = |k: usize| (k as isize + shift) as usize;
            let end = old.trimmed(a, b);
            SetLine {
                chars: (at(a), at(b)),
                kept: Some(n),
                x: para.chars.get(a).map_or(para.frame.left, |c| c.x.0),
                baseline: para.rows[n].baseline,
                width: old.width(a, b),
                stretch: 0.0,
                hyphen: end > a && old.chars[end - 1] == '\u{AD}',
            }
        };
        if prefix == nl && nl == ol && !moved && !realigned {
            return SetParagraph {
                lines: (0..rows).map(|n| kept(n, 0)).collect(),
                marked: Vec::new(),
            };
        }
        // The line the first change is in, a space joining two lines in the one before;
        // with the edges moved, the first.
        let changed = match moved {
            true => 0,
            false => starts.iter().rposition(|&a| a <= prefix).unwrap_or(0),
        };
        let starts_paragraph = |m: &Measured, a: usize| a == 0 || m.chars[a - 1] == '\u{2029}';
        let mut first = changed;
        if changed > 0 {
            let line_before = starts[changed - 1];
            let fits = |m: &Measured| {
                let first_word = (m.breaks.iter())
                    .map(|&(b, _)| b)
                    .find(|&b| b > starts[changed])
                    .unwrap_or(m.chars.len());
                let room = room_in(&frame, starts_paragraph(m, line_before));
                m.width(line_before, first_word) <= room + 1e-3
            };
            if fits(&new) && !fits(old) {
                first = changed - 1;
            }
        }
        let mut lines: Vec<SetLine> = (0..first).map(|n| kept(n, 0)).collect();
        let mut a = starts[first];
        let mut n = first;
        let mut push = 0.0f32;
        // The largest text the paragraph's spacing holds as it is.
        let holds = old.sizes.iter().copied().fold(0.0, f32::max);
        let size = para.spans.first().map_or(12.0, |s| s.font_size);
        let spacing = match para.frame.spacing {
            s if s > 0.0 => s,
            _ => 1.2 * size,
        };
        let baseline = |n: usize| match para.rows.get(n) {
            Some(r) => r.baseline,
            None => para.rows[rows - 1].baseline + spacing * (n + 1 - rows) as f32,
        };
        while a < nl {
            if !moved && n > first && push == 0.0 && n < rows && was(a) == Some(starts[n]) {
                lines.extend((n..rows).map(|m| kept(m, nl as isize - ol as isize)));
                break;
            }
            let opens = starts_paragraph(&new, a);
            let room = room_in(&frame, opens);
            let mut e = new.line_end(a, room);
            // A line that starts where it did ends where it did when the words the greedy
            // end adds would have fitted on it before too (the producer measured them so),
            // while the frame is as it was.
            let where_it_did = !moved
                && n < rows
                && ((a <= prefix && a == starts[n]) || was(a) == Some(starts[n]));
            let end_before = old_end(n.min(rows - 1));
            if where_it_did && end_before >= ol - suffix {
                let end_now = (end_before as isize + nl as isize - ol as isize) as usize;
                let fitted = was(e).is_some_and(|e_was| old.width(starts[n], e_was) <= room + 1e-3);
                if e > end_now && end_now > a && fitted {
                    e = end_now;
                }
            }
            let end = new.trimmed(a, e);
            let width = new.width(a, e);
            // A line with larger text than the paragraph's gets the room below the one
            // before that it needs, 1.2 times its size, at most what its size grew by: a
            // paragraph set tighter keeps its spacing.
            if n > 0 {
                let big = new.sizes[a..end].iter().copied().fold(0.0, f32::max);
                let gap = baseline(n) - baseline(n - 1);
                push += (1.2 * big - gap).min(1.2 * (big - holds)).max(0.0);
            }
            let last = e >= nl || new.chars[e - 1] == '\u{2029}' || new.chars[e - 1] == '\n';
            let gaps = new.chars[a..end].iter().filter(|c| **c == ' ').count();
            let line = Placing {
                opens,
                width,
                gaps,
                last,
            };
            let (x, stretch) = line.on(&frame);
            // With the edges moved or the text realigned, a line that starts and ends as it
            // did in text that hasn't changed, which the frame moves nowhere, stays as it was.
            if (moved || realigned) && push == 0.0 && n < rows {
                let same = (e <= prefix && a == starts[n] && e == old_end(n))
                    || (was(a) == Some(starts[n]) && was(e) == Some(old_end(n)));
                let (x0, stretch0) = line.on(&para.frame);
                if same && (x - x0).abs() < 1e-3 && (stretch - stretch0).abs() < 1e-3 {
                    lines.push(kept(n, a as isize - starts[n] as isize));
                    a = e;
                    n += 1;
                    continue;
                }
            }
            lines.push(SetLine {
                chars: (a, e),
                kept: None,
                x,
                baseline: baseline(n) + push,
                width,
                stretch,
                hyphen: end > a && new.chars[end - 1] == '\u{AD}',
            });
            a = e;
            n += 1;
        }
        // Realigned in the frame as it was: the lines kept keep their words, each written
        // anew where the new alignment places it otherwise than the old one did.
        if realigned && !moved {
            for l in lines.iter_mut().filter(|l| l.kept.is_some()) {
                let (a, e) = l.chars;
                let end = new.trimmed(a, e);
                let line = Placing {
                    opens: starts_paragraph(&new, a),
                    width: l.width,
                    gaps: new.chars[a..end].iter().filter(|c| **c == ' ').count(),
                    last: e >= nl || matches!(new.chars[e - 1], '\u{2029}' | '\n'),
                };
                let (x, stretch) = line.on(&frame);
                let (x0, stretch0) = line.on(&para.frame);
                if (x - x0).abs() > 1e-3 || (stretch - stretch0).abs() > 1e-3 {
                    (l.kept, l.x, l.stretch) = (None, x, stretch);
                }
            }
        }
        // The hyphens that may be their word's a rewrap brought inside a line.
        let mut marked = Vec::new();
        for h in para.hyphens.iter().filter(|h| h.kind == Hyphen::Unknown) {
            let Some(j) = new.from.iter().position(|&f| f == Some(h.at)) else {
                continue;
            };
            let line = lines.iter().find(|l| l.chars.0 <= j && j < l.chars.1);
            if let Some(l) = line.filter(|l| l.kept.is_none()) {
                if j + 1 < new.trimmed(l.chars.0, l.chars.1) {
                    marked.push(j);
                }
            }
        }
        SetParagraph { lines, marked }
    }

    /// The narrowest the frame can be made with `parts` in it, from its left edge to its
    /// right: its widest word whole on a line, with the first line's indent where it takes
    /// one.
    pub fn narrowest(&self, parts: &[TextPart]) -> f32 {
        self.narrowest_in(parts, self.para.frame.align)
    }

    /// [`ParagraphSetting::narrowest`] with the paragraph aligned as `align` says, which
    /// says whether its first line takes an indent.
    pub fn narrowest_in(&self, parts: &[TextPart], align: Align) -> f32 {
        let m = self.measure(parts);
        let mut widest = 0.0f32;
        let mut a = 0;
        for &(b, _) in &m.breaks {
            widest = widest.max(m.width(a, b));
            a = b;
        }
        let p = &self.para.frame;
        let f = self.frame_in((p.left, p.right), align);
        widest + (f.right - f.left - room_in(&f, true)).max(0.0)
    }

    /// The paragraph's frame with its left and right edges at `edges` and its text aligned
    /// as `align` says, with the first line's indent the paragraph has: one read as centred
    /// or right-aligned has none, its first line's place being its width's, unless its frame
    /// is the one LimarePDF wrote it in, which keeps its own.
    pub(crate) fn frame_in(&self, edges: (f32, f32), align: Align) -> Frame {
        let f = &self.para.frame;
        let indent = match f.align {
            _ if self.para.kept => f.indent,
            Align::Left | Align::Justified => f.indent,
            Align::Right | Align::Centre => 0.0,
        };
        Frame {
            left: edges.0,
            right: edges.1,
            align,
            indent,
            ..*f
        }
    }

    /// How each character of `parts` is written: in its part's own font where it can be
    /// ([`PageObject::kept_pieces`]'s rule, across the paragraph), else in the font its words
    /// go to.
    pub(crate) fn planned(&self, parts: &[TextPart]) -> Vec<Planned> {
        let para = &self.para;
        let mut out = Vec::new();
        for (pi, p) in parts.iter().enumerate() {
            let text: Vec<char> = p.text.chars().collect();
            let span = match p.font {
                PartFont::Span(i) => para.spans.get(i).map(|s| (i, s)),
                PartFont::Choice(_) => None,
            };
            let Some((i, s)) = span else {
                let away = match p.font {
                    PartFont::Choice(c) => c,
                    PartFont::Span(_) => FontChoice::default(),
                };
                out.extend((0..text.len()).map(|j| Planned {
                    part: pi,
                    at: j,
                    kept: None,
                    away,
                    from: p.origin(j),
                }));
                continue;
            };
            let (font, other) = &self.fonts[i];
            let drawn = |o: usize| self.drawn.get(o)?.as_ref();
            let origin = |j: usize| p.origin(j);
            let ways = plan(&self.chars, s.chars, drawn, Some(font), &text, origin);
            let mut kept = vec![false; text.len()];
            for piece in pieces(&text, &ways).into_iter().filter(|p| p.kept) {
                kept[piece.chars.0..piece.chars.1].fill(true);
            }
            out.extend((0..text.len()).map(|j| Planned {
                part: pi,
                at: j,
                kept: kept[j].then_some((i, ways[j])),
                away: *other,
                from: p.origin(j),
            }));
        }
        out
    }

    /// Whether character `c` of part `p` of an edit, which was character `k` of the
    /// paragraph's text, is unchanged there: the same character, in the part it was in, in
    /// that part's style.
    fn unchanged(&self, p: &TextPart, c: char, k: usize) -> bool {
        let PartFont::Span(i) = p.font else {
            return false;
        };
        let Some(s) = self.para.spans.get(i) else {
            return false;
        };
        self.para.span_at(k) == Some(i)
            && self.chars.get(k) == Some(&c)
            && p.size == s.font_size
            && p.color == s.fill.unwrap_or([0.0; 3])
            && p.rise == s.rise
    }

    /// A setter for each font that words of an edit go to ([`away_pieces`], of the edit's
    /// text `chars`), each with all of the text it writes, as the writer's has.
    pub(crate) fn setters(
        &self,
        chars: &[char],
        away: &[(FontChoice, usize, usize, usize)],
    ) -> Vec<(FontChoice, Setter)> {
        let mut out: Vec<(FontChoice, Setter)> = Vec::new();
        for &(f, ..) in away {
            if out.iter().any(|(g, _)| *g == f) {
                continue;
            }
            let all: String = (away.iter().filter(|w| w.0 == f))
                .flat_map(|w| chars[w.2..w.3].iter())
                .collect();
            out.push((f, Setter::new(f, &all, self.lang.as_deref())));
        }
        out
    }

    /// The character spacing, word spacing (none in a paragraph read as justified, where it
    /// stretched the spaces, whatever the paragraph is aligned to now) and horizontal scaling
    /// a part `p` of an edit is written with: its paragraph part's, or for one in a font
    /// chosen, the paragraph's first part's.
    pub(crate) fn spacing(&self, p: &TextPart) -> [f32; 3] {
        let parts = &self.para.parts;
        let i = match p.font {
            PartFont::Span(i) => i,
            PartFont::Choice(_) => 0,
        };
        let part = parts.get(i).or(parts.first());
        let [tc, tw, th] = part.map_or([0.0, 0.0, 1.0], |p| p.spacing);
        match self.para.frame.align {
            Align::Justified => [tc, 0.0, th],
            _ => [tc, tw, th],
        }
    }

    /// `parts` measured as they are written ([`ParagraphSetting::planned`]): a character
    /// kept in its part's own font by where it was drawn (and its spacing to its neighbour
    /// there, while that is still its neighbour), or by the font's width for its code; the
    /// words that go to another font by that font. The parts' character spacing and scaling
    /// apply throughout, their word spacing to single-byte spaces.
    fn measure(&self, parts: &[TextPart]) -> Measured {
        let para = &self.para;
        let plan = self.planned(parts);
        let mut m = Measured {
            chars: parts.iter().flat_map(|p| p.text.chars()).collect(),
            ..Default::default()
        };
        let n = m.chars.len();
        m.widths = vec![0.0; n];
        m.shown = vec![0.0; n];
        m.sizes = plan.iter().map(|pl| parts[pl.part].size).collect();
        m.from = plan.iter().map(|pl| pl.from).collect();
        m.same = (plan.iter().zip(&m.chars))
            .map(|(pl, &c)| pl.from.filter(|&k| self.unchanged(&parts[pl.part], c, k)))
            .collect();
        for j in 0..n {
            let pl = plan[j];
            let p = &parts[pl.part];
            let c = m.chars[j];
            let Some((i, way)) = pl.kept else {
                continue;
            };
            let font = &self.fonts[i].0;
            let [tc, tw, th] = self.spacing(p);
            let scale = p.size / para.spans[i].font_size.max(1e-3);
            let glyph = |k: usize| {
                let x = para.chars[k].x;
                (x.1 - x.0) * scale
            };
            let space = |c: char| {
                let w = match font.advance(c) {
                    Some(w) => w / 1000.0 * p.size * th,
                    None => self.space_em * p.size,
                };
                let word = font.code(c).is_ok_and(|b| b == [32]);
                w + tc * th + if word { tw * th } else { 0.0 }
            };
            m.widths[j] = match way {
                _ if c == '\u{AD}' => {
                    let k = pl.from.filter(|_| matches!(way, Way::Own(_)));
                    let hyphen = font.advance('-').map(|w| w / 1000.0 * p.size * th);
                    m.shown[j] = match k {
                        Some(k) => glyph(k) + tc * th,
                        None => hyphen.unwrap_or(0.33 * p.size),
                    };
                    0.0
                }
                _ if c.is_whitespace() => space(' '),
                Way::Own(_) | Way::WithBefore => {
                    let k = pl.from.expect("its own character");
                    let next = plan.get(j + 1);
                    let neighbour = next.is_some_and(|q| {
                        matches!(q.kept, Some((_, Way::Own(_) | Way::WithBefore)))
                            && q.from == Some(k + 1)
                            && !m.chars[j + 1].is_whitespace()
                    }) && (para.neighbours(k) || para.accent_over(k));
                    let after = match neighbour {
                        true => para.gap_after(k) * scale,
                        false => tc * th,
                    };
                    glyph(k) + after
                }
                Way::Code => {
                    let w = font.advance(c).unwrap_or(500.0);
                    w / 1000.0 * p.size * th + tc * th
                }
                Way::Space | Way::Missing => space(' '),
            };
        }
        // Each font's words, measured with a setter that has all of its text, as the
        // writer's has.
        let away = away_pieces(&plan);
        for (f, setter) in self.setters(&m.chars, &away) {
            for &(_, pi, a, b) in away.iter().filter(|w| w.0 == f) {
                let p = &parts[pi];
                let (size, [tc, _, th]) = (p.size, self.spacing(p));
                let text: String = m.chars[a..b].iter().collect();
                // Each stretch between two places a line may break, its spaces apart.
                let mut at = a;
                for seg in segments(&text) {
                    let n = seg.chars().count();
                    let word = seg.trim_end_matches(char::is_whitespace);
                    let wn = word.chars().count();
                    let plain: String = word.chars().filter(|&c| c != '\u{AD}').collect();
                    if wn > 0 {
                        m.widths[at] = setter.width(&plain, size) * th + tc * th * wn as f32;
                    }
                    if word.ends_with('\u{AD}') {
                        m.shown[at + wn - 1] = setter.width("-", size) * th + tc * th;
                    }
                    for k in at + wn..at + n {
                        m.widths[k] = setter.width(" ", size) * th + tc * th;
                    }
                    at += n;
                }
            }
        }
        let text: String = m.chars.iter().collect();
        let mut char_at = vec![0usize; text.len() + 1];
        for (k, (b, _)) in text.char_indices().enumerate() {
            char_at[b] = k;
        }
        char_at[text.len()] = m.chars.len();
        m.breaks = (linebreaks(&text))
            .map(|(b, o)| (char_at[b], o == BreakOpportunity::Mandatory))
            .collect();
        m
    }
}

/// How a character of an edited paragraph is written ([`ParagraphSetting::planned`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Planned {
    /// The edit's part it is in, and its place there.
    pub part: usize,
    pub at: usize,
    /// In the font of the paragraph's part with this index, written as this says
    /// ([`plan`]); `None` when it is written in another font.
    pub kept: Option<(usize, Way)>,
    /// The font its word goes to when it isn't kept.
    pub away: FontChoice,
    /// Where it was in the paragraph's text, whatever its style.
    pub from: Option<usize>,
}

/// The stretches of an edit written in another font than their parts' own (`plan`, by
/// [`ParagraphSetting::planned`]): each one's font, the edit's part it is in, and its
/// characters in the edit's text, from and to.
pub(crate) fn away_pieces(plan: &[Planned]) -> Vec<(FontChoice, usize, usize, usize)> {
    let mut away: Vec<(FontChoice, usize, usize, usize)> = Vec::new();
    for (j, pl) in plan.iter().enumerate().filter(|(_, pl)| pl.kept.is_none()) {
        match away.last_mut() {
            Some(w) if w.0 == pl.away && w.1 == pl.part && w.3 == j => w.3 = j + 1,
            _ => away.push((pl.away, pl.part, j, j + 1)),
        }
    }
    away
}

/// `text` cut where a line may break (UAX #14), each stretch with the spaces after it.
fn segments(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut from = 0;
    for (b, _) in linebreaks(text) {
        if b > from {
            out.push(&text[from..b]);
            from = b;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Vec<Read> {
        let drawn = Char {
            from: Some((0, 0)),
            x: (0.0, 0.0),
        };
        text.chars().map(|c| (c, drawn, 0)).collect()
    }

    #[test]
    fn words_are_letters_with_hyphens_inside() {
        let w = DocWords::of(["A well-known ex\u{AD}ample, 42 times-", "Well‐known again"]);
        assert_eq!(w.count("well-known"), 2);
        assert_eq!(w.count("example"), 1);
        assert_eq!(w.count("times"), 1);
        assert_eq!(w.count("42"), 0);
        assert_eq!(w.count("a"), 1);
    }

    #[test]
    fn a_line_end_hyphen_is_read_by_the_documents_words() {
        let words = DocWords::of([
            "an example here",
            "a well-known name",
            "both co-op and coop",
        ]);
        let next = |s: &str| s.chars().collect::<Vec<_>>();
        let read = |a: &str, b: &str| line_end_hyphen(&line(a), &next(b), &words).map(|h| h.1);
        assert_eq!(read("for exam-", "ple, said"), Some(Hyphen::Discretionary));
        assert_eq!(read("a well-", "known one"), Some(Hyphen::Hard));
        assert_eq!(read("the co-", "op shop"), Some(Hyphen::Unknown));
        assert_eq!(read("never seen-", "before"), Some(Hyphen::Unknown));
        assert_eq!(read("a soft\u{AD}", "ware"), Some(Hyphen::Discretionary));
        // Not before a lower-case word, nor after one.
        assert_eq!(read("Jean-", "Pierre came"), None);
        assert_eq!(read("pages 10-", "20"), None);
        assert_eq!(read("and -", "then"), None);
    }
}
