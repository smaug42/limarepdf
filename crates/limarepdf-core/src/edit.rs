//! Editing page content: move/resize, delete, change text, restyle paths, replace images, insert objects.

use crate::colour::Paint;
use crate::decorate::{Decorated, Decoration, Laid, Redraw};
use crate::embed::unwritten_script;
use crate::fonts::{FontChoice, FontCtx};
use crate::imgs::orient_matrix;
use crate::inline::Inline;
use crate::layout::{Pen, Setter};
use crate::matrix::{self as mx, mul, M};
use crate::objects::{reads_as_space, Obj, ObjectKind, Parsed, Run, Show, Span, TextData};
use crate::own_font::{LineFont, PageFonts};
use crate::{PageObject, Pdf, Result};
use anyhow::{anyhow, bail};
use image::metadata::Orientation;
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId, Stream, StringFormat};
use std::collections::{HashMap, HashSet};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, PartialEq)]
pub struct TextEdit {
    pub text: String,
    /// `None` keeps the current font (when it can be reused, else the installed family it is
    /// from or the nearest standard font: [`FontChoice::of_font`]), otherwise a standard font
    /// or an installed family's face.
    pub font: Option<FontChoice>,
    pub size: Option<f32>,
    pub color: Option<[f32; 3]>,
    /// The line in parts of different styles, `text` shared out among them in order. When
    /// empty, the whole of `text` is in the one style above; otherwise `font`, `size` and
    /// `color` aren't used.
    pub parts: Vec<TextPart>,
}

/// A part of a line of text in one style: [`TextEdit::parts`], [`Pdf::add_text_parts`].
#[derive(Clone, Debug, PartialEq)]
pub struct TextPart {
    pub text: String,
    pub font: PartFont,
    pub size: f32,
    pub color: [f32; 3],
    /// How far the part is raised above the line's baseline, in points (lowered when
    /// below zero).
    pub rise: f32,
    /// Where `text` starts in the edited line's text, when it is the line's own characters
    /// there, unchanged. In the font of the line's part they are in ([`PartFont::Span`])
    /// they are then written with their own codes and spacing ([`Pdf::edit_texts`]). `None`
    /// for anything else.
    pub from: Option<usize>,
    /// Where each of `text`'s characters is in the edited line's text as it was read, for
    /// those that are the line's own characters there, unchanged; `None` for one typed or
    /// moved. In the font of the line's part they are in, they keep the codes they were
    /// drawn with and the spacing numbers between neighbours. Empty when not known (then
    /// `from` says, when it is given).
    pub origins: Vec<Option<usize>>,
}

impl TextPart {
    /// Where character `j` of the part is in the edited line as it was read, if it is the
    /// line's own character there ([`TextPart::origins`], else [`TextPart::from`]).
    pub(crate) fn origin(&self, j: usize) -> Option<usize> {
        match self.origins.is_empty() {
            true => self.from.map(|f| f + j),
            false => self.origins.get(j).copied().flatten(),
        }
    }
}

/// A piece of a part of a line as [`Pdf::edit_text`] writes it ([`PageObject::kept_pieces`]).
#[derive(Clone, Debug, PartialEq)]
pub struct KeptPiece {
    /// Its characters in the part's text, from and to.
    pub chars: (usize, usize),
    /// Written in the font of the line's part the part names; otherwise in the installed
    /// family that font is from or the nearest standard font.
    pub kept: bool,
    /// The characters that font has no code for, which send the piece's words to another
    /// font, each once.
    pub missing: Vec<char>,
}

/// The font a part of a line is written in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PartFont {
    /// The font of the line's part with this index ([`crate::PageObject::spans`]), kept
    /// when [`crate::PageObject::keeps_font`] says so, else the installed family it is from
    /// or the nearest standard font ([`FontChoice::of_font`]).
    Span(usize),
    /// A standard font or an installed family's face.
    Choice(FontChoice),
}

impl TextEdit {
    /// Whether [`Pdf::edit_text`] keeps line `o`'s own font for all of this change, `fonts`
    /// being the line's ([`Pdf::line_fonts`]): no style changes, and every new character
    /// has a code in the font of the part it is in ([`PageObject::keeps_font`]).
    pub fn keeps_font(&self, o: &PageObject, fonts: &[LineFont]) -> bool {
        let first = o.spans.first().map(|s| (s.font_size, s.fill, s.rise));
        match self.parts_for(o.text.as_deref().unwrap_or(""), first) {
            Some(parts) => parts.iter().all(|p| o.keeps_font(p, fonts)),
            None => false,
        }
    }

    /// The parts this edit writes in a line whose text is `line`, its first part in
    /// `first` (size, colour and rise): its own, or a change of words alone as one part in
    /// that part's font, the characters it keeps at its start and end saying where they
    /// were. `None` for a change in one style that sets the font, size or colour, which is
    /// written anew.
    fn parts_for(
        &self,
        line: &str,
        first: Option<(f32, Option<[f32; 3]>, f32)>,
    ) -> Option<std::borrow::Cow<'_, [TextPart]>> {
        if !self.parts.is_empty() {
            return Some(self.parts.as_slice().into());
        }
        if self.font.is_some() || self.size.is_some() || self.color.is_some() {
            return None;
        }
        let (size, fill, rise) = first?;
        Some(
            vec![TextPart {
                text: self.text.clone(),
                font: PartFont::Span(0),
                size,
                color: fill.unwrap_or([0.0; 3]),
                rise,
                from: None,
                origins: kept_ends(line, &self.text),
            }]
            .into(),
        )
    }

    /// Whether this edit leaves line `t` as it is: the same text, each part its line's part's
    /// own, in its style. Only its decorations can then change, and its words aren't written.
    pub(crate) fn unchanged(&self, t: &TextData) -> bool {
        if self.text != t.text {
            return false;
        }
        if self.parts.is_empty() {
            return self.font.is_none() && self.size.is_none() && self.color.is_none();
        }
        let mut at = 0;
        self.parts.iter().all(|p| {
            let n = p.text.chars().count();
            let own = match p.font {
                PartFont::Span(i) => t.spans.get(i).is_some_and(|s| {
                    let colour = s.fill.unwrap_or([0.0; 3]);
                    s.chars == (at, at + n)
                        && (s.size - p.size).abs() <= 0.01 * s.size.max(0.01)
                        && (0..3).all(|k| (colour[k] - p.color[k]).abs() < 1e-3)
                        && (s.rise - p.rise).abs() < 0.01
                }),
                PartFont::Choice(_) => false,
            };
            at += n;
            own
        })
    }

    /// Where each of this edit's characters is in line `line` as it was read (its first part
    /// in `first`), `None` for one typed: as its parts say ([`TextPart::origin`]), or for a
    /// change in one style, the characters it keeps at its start and end.
    pub(crate) fn origins(
        &self,
        line: &str,
        first: Option<(f32, Option<[f32; 3]>, f32)>,
    ) -> Vec<Option<usize>> {
        match self.parts_for(line, first) {
            Some(parts) => (parts.iter())
                .flat_map(|p| (0..p.text.chars().count()).map(move |j| p.origin(j)))
                .collect(),
            None => kept_ends(line, &self.text),
        }
    }

    /// The line as `parts`, in their styles.
    pub fn in_parts(parts: Vec<TextPart>) -> Self {
        Self {
            text: parts.iter().map(|p| p.text.as_str()).collect(),
            font: None,
            size: None,
            color: None,
            parts,
        }
    }
}

/// Where each character of `new` is in `old`, for those it keeps at its start and end
/// (`None` for the others).
fn kept_ends(old: &str, new: &str) -> Vec<Option<usize>> {
    let (old, new): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - head;
    let tail = (old.iter().rev().zip(new.iter().rev()))
        .take_while(|(a, b)| a == b)
        .count()
        .min(room);
    (0..new.len())
        .map(|j| match j {
            _ if j < head => Some(j),
            _ if j >= new.len() - tail => Some(old.len() + j - new.len()),
            _ => None,
        })
        .collect()
}

impl PageObject {
    /// Whether [`Pdf::edit_text`] writes all of `part` of this line in the font of the
    /// line's part it names ([`PartFont::Span`]), `fonts` being the line's
    /// ([`Pdf::line_fonts`]): [`PageObject::kept_pieces`] keeps every piece.
    pub fn keeps_font(&self, part: &TextPart, fonts: &[LineFont]) -> bool {
        match part.font {
            PartFont::Span(i) if part.text.is_empty() => i < self.spans.len(),
            _ => self.kept_pieces(part, fonts).iter().all(|p| p.kept),
        }
    }

    /// How [`Pdf::edit_text`] writes `part` of this line, `fonts` being the line's
    /// ([`Pdf::line_fonts`]): its text in pieces, in order, each in the font of the line's
    /// part it names ([`PartFont::Span`]) or not. A part that is that part's own text,
    /// unchanged, keeps it. Otherwise each character keeps it when it is the line's own
    /// character, unchanged, where it was ([`TextPart::origins`]), drawn with a code of its
    /// own (a ligature's letters only together), or when the font has a code for it
    /// ([`LineFont::code`]); a space the font has none for is written as a move. A word
    /// with a character that can't sends the whole word to the installed family the font is
    /// from or the nearest standard font, with the spaces between two such words.
    pub fn kept_pieces(&self, part: &TextPart, fonts: &[LineFont]) -> Vec<KeptPiece> {
        let text: Vec<char> = part.text.chars().collect();
        let all = |kept| {
            vec![KeptPiece {
                chars: (0, text.len()),
                kept,
                missing: Vec::new(),
            }]
        };
        if text.is_empty() {
            return Vec::new();
        }
        let PartFont::Span(i) = part.font else {
            return all(false);
        };
        let (Some(s), Some(line)) = (self.spans.get(i), &self.text) else {
            return all(false);
        };
        let font = fonts.get(i);
        let line: Vec<char> = line.chars().collect();
        if !self.vertical && line.get(s.chars.0..s.chars.1) == Some(&text[..]) {
            return all(true);
        }
        let drawn = font.and_then(LineFont::drawn);
        let at = |o: usize| drawn?.get(o);
        let ways = plan(&line, s.chars, at, font, &text, |j| part.origin(j));
        pieces(&text, &ways)
    }
}

/// How a character of a part is written in the font of the line's part it names ([`plan`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Way {
    /// With what drew the line's character `o` as it was read ([`Drawn`]): its code (and
    /// the letters after it of a ligature, which follow it unchanged), or the move that is
    /// a space.
    Own(usize),
    /// Drawn by the code of a character before it, a ligature's.
    WithBefore,
    /// With the font's code for it ([`LineFont::code`]).
    Code,
    /// A space the font has no code for, written as a move.
    Space,
    /// The font can't write it.
    Missing,
}

/// How each of `text`'s characters is written in the font `font` of the part of `line` at
/// `span`, whose characters are drawn as `drawn` says (when known, by their place in
/// `line`); `origin` says where a character of `text` was in the line as read
/// ([`TextPart::origin`]).
pub(crate) fn plan<'d>(
    line: &[char],
    span: (usize, usize),
    drawn: impl Fn(usize) -> Option<&'d Drawn>,
    font: Option<&LineFont>,
    text: &[char],
    origin: impl Fn(usize) -> Option<usize>,
) -> Vec<Way> {
    // Where character `j` was, if it is the line's own character there, in this part.
    let own =
        |j: usize| origin(j).filter(|&o| span.0 <= o && o < span.1 && line.get(o) == text.get(j));
    let mut ways = vec![Way::Missing; text.len()];
    let mut j = 0;
    while j < text.len() {
        let c = text[j];
        let ink = own(j).and_then(|o| Some((o, &drawn(o)?.ink)));
        let (way, n) = match ink {
            Some((o, Ink::Code { at, n, .. }))
                if *at == o && (1..*n).all(|i| own(j + i) == Some(o + i)) =>
            {
                (Way::Own(o), *n)
            }
            Some((o, Ink::Move(_))) => (Way::Own(o), 1),
            // A script written right to left, or whose letters join, isn't written one
            // code a character (PB-53): it goes where it is refused, with its reason.
            _ if unwritten_script(c) => (Way::Missing, 1),
            _ if font.is_some_and(|f| f.code(c).is_ok()) => (Way::Code, 1),
            _ if c.is_whitespace() => (Way::Space, 1),
            _ => (Way::Missing, 1),
        };
        ways[j] = way;
        ways[j + 1..j + n].fill(Way::WithBefore);
        j += n;
    }
    ways
}

/// `text`'s characters in pieces kept in the line's font or not, by how they are written
/// (`ways`, [`plan`]): a word, as Unicode's word rule finds it (UAX #29: each Chinese or
/// Japanese ideograph is one, and punctuation stands apart), with a character the font
/// can't write goes to another font, with the spaces between two such words.
pub(crate) fn pieces(text: &[char], ways: &[Way]) -> Vec<KeptPiece> {
    let n = text.len();
    // Whether each character's word goes to another font; spaces belong to none.
    let mut lost = vec![false; n];
    let joined: String = text.iter().collect();
    let mut j = 0;
    for word in joined.split_word_bounds() {
        let end = j + word.chars().count();
        if !word.chars().all(char::is_whitespace) {
            let gone = ways[j..end].contains(&Way::Missing);
            lost[j..end].fill(gone);
        }
        j = end;
    }
    // Spaces between two words that go go with them.
    let words = lost.clone();
    let word = |mut range: Box<dyn Iterator<Item = usize>>| {
        range
            .find(|&k| !text[k].is_whitespace())
            .is_some_and(|k| words[k])
    };
    for j in 0..n {
        if text[j].is_whitespace() {
            lost[j] = word(Box::new((0..j).rev())) && word(Box::new(j + 1..n));
        }
    }
    let mut out: Vec<KeptPiece> = Vec::new();
    for j in 0..n {
        let kept = !lost[j];
        match out.last_mut() {
            Some(p) if p.kept == kept => p.chars.1 = j + 1,
            _ => out.push(KeptPiece {
                chars: (j, j + 1),
                kept,
                missing: Vec::new(),
            }),
        }
        let p = out.last_mut().expect("pushed");
        if ways[j] == Way::Missing && !p.missing.contains(&text[j]) {
            p.missing.push(text[j]);
        }
    }
    out
}

/// A line of parts to write ([`Pdf::written_lines`]): the line edited (the page as parsed,
/// and the line) or none for new text, its parts, and whether characters may be written
/// in the line's own fonts (only those whose text is a part's own, unchanged, otherwise).
type Asked<'a> = (Option<(&'a Parsed, &'a TextData)>, &'a [TextPart], bool);

/// The lines a [`Pdf::edit_texts`] changes, for their decorations: the edits, their lines as
/// read, which leave their words as they are, the decorations asked for (else carried) and
/// those each has as read.
struct Lines<'a> {
    edits: &'a [(usize, TextEdit)],
    objs: &'a [&'a Obj],
    unchanged: &'a [bool],
    asked: Option<&'a [Vec<Decorated>]>,
    old: &'a [Vec<&'a Decoration>],
}

/// A page's operators with text edits made ([`Pdf::edit_texts`]).
struct Edited {
    ops: Vec<Operation>,
    /// The `q`s left open, to close.
    depth: usize,
    /// The edits with characters written in their lines' own fonts, to read back: their
    /// index, where they are in `ops` (from and to; or, changed in place, the operator
    /// that shows them), and the text they should read.
    checks: Vec<(usize, Check, String)>,
    /// Where each edit's line is written in `ops`; `None` for invisible text.
    places: Vec<Option<Check>>,
    /// Where in `ops` each operator watched (by its index in the page as read) went.
    watched: HashMap<usize, usize>,
}

/// Where an edited line is in a page's operators, to read it back ([`Pdf::misread`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Check {
    /// Written on top, as these operators, from and to.
    OnTop(usize, usize),
    /// Changed in place: the show at this operator.
    InPlace(usize),
    /// Written in place inside a text object, as these operators, from and to: read with
    /// the lines it may join there (a paragraph's, `para_write.rs`).
    Within(usize, usize),
}

impl Check {
    /// The operators the line is written as, from and to.
    fn ops(self) -> std::ops::Range<usize> {
        match self {
            Check::OnTop(from, to) | Check::Within(from, to) => from..to,
            Check::InPlace(i) => i..i + 1,
        }
    }
}

/// Where a line written should sit ([`Pdf::misplaced`]), in points along the line on the
/// displayed page (across it, for a line upright on it).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Extent {
    /// The line's index, as its read-back check has it.
    pub n: usize,
    /// Where it is in the operators.
    pub check: Check,
    /// Where its first character starts, when that is known: a paragraph's line. A line
    /// edit's first word is in `before` when it ends before the edit.
    pub x: Option<f32>,
    /// Where its last character ends, when that is known: a paragraph's line, set to a
    /// width.
    pub end: Option<f32>,
    /// The furthest its last character may end: where its box stays inside its frame's
    /// right edge, less the character spacing after that character; none for a line wider
    /// than its frame.
    pub right: Option<f32>,
    /// Where the words before an edit start, from the line's first: they stay there.
    pub before: Vec<f32>,
    /// Where the words after an edit started, to the line's last: they all move as one.
    pub after: Vec<f32>,
}

/// A part of a line as it is written by [`line_ops`]: the font's name in the page's
/// resources, its size, colour (and what set it, when that is written as it was) and rise,
/// and its shows, each with the character spacing, word spacing and horizontal scaling (1
/// is none) it is drawn with.
pub(crate) struct Written {
    pub font: Vec<u8>,
    pub size: f32,
    pub color: [f32; 3],
    pub paint: Option<Paint>,
    pub rise: f32,
    /// In a font that stands its glyphs up only where text space is turned over (TeX's
    /// bitmap fonts, `FontCtx::flipped`): written in text space turned over
    /// ([`Pdf::turned`]), its rise the other way.
    pub flipped: bool,
    pub shows: Vec<([f32; 3], Operation)>,
}

/// What sets colour `color` for a part of line `t` in font `font`, when the line has it in a
/// colour space `rg` doesn't write (a spot colour, CMYK, PB-65): what set it there, the part
/// of the line the font names first. `None` writes `rg`.
fn kept_paint(t: &TextData, font: PartFont, color: [f32; 3]) -> Option<Paint> {
    let named = match font {
        PartFont::Span(i) => t.spans.get(i),
        PartFont::Choice(_) => None,
    };
    (named.into_iter().chain(&t.spans))
        .find(|s| s.fill == Some(color) && !s.paint.plain())
        .map(|s| s.paint.clone())
}

/// A line of text in parts, from `BT` to `ET`, starting at text matrix `tm`: each part's
/// font, colour, rise and spacing set where they change, then its shows.
fn line_ops(tm: M, parts: &[Written]) -> Vec<Operation> {
    let mut v = vec![op("BT", vec![])];
    v.extend(line_body(tm, parts, false));
    v.push(op("ET", vec![]));
    v
}

/// [`line_ops`] inside a text object: from the first part's font, colour and rise to the
/// last show. With `all`, the rise and spacing are set at the start whatever they are
/// (inside a text object already drawing, they are what it set); else they start as a
/// text object starts.
pub(crate) fn line_body(tm: M, parts: &[Written], all: bool) -> Vec<Operation> {
    let mut v = Vec::new();
    let mut font: Option<(&[u8], f32)> = None;
    let mut color = None;
    let (mut rise, mut spacing) = match all {
        true => (f32::NAN, [f32::NAN; 3]),
        false => (0.0, [0.0, 0.0, 1.0]),
    };
    for (k, p) in parts.iter().enumerate() {
        if font != Some((&p.font, p.size)) {
            v.push(op("Tf", vec![Object::Name(p.font.clone()), r(p.size)]));
            font = Some((&p.font, p.size));
        }
        if color != Some((p.color, &p.paint)) {
            match &p.paint {
                Some(paint) => v.extend(paint.ops()),
                None => v.push(rgb("rg", p.color)),
            }
            color = Some((p.color, &p.paint));
        }
        let up = if p.flipped { -p.rise } else { p.rise };
        if up != rise {
            v.push(op("Ts", vec![r(up)]));
            rise = up;
        }
        if k == 0 {
            v.push(op("Tm", tm.iter().map(|&x| r(x)).collect()));
        }
        for (s, show) in &p.shows {
            for (i, name) in ["Tc", "Tw", "Tz"].into_iter().enumerate() {
                if s[i] != spacing[i] {
                    let x = if i == 2 { s[i] * 100.0 } else { s[i] };
                    v.push(op(name, vec![r(x)]));
                }
            }
            spacing = *s;
            v.push(show.clone());
        }
    }
    v
}

/// The shows of part `s` of line `t` as they were, from the page's operators `ops`, each
/// with its spacing. A gap read as a space between two of them is put back as the move
/// from where the one ends to where the next starts: the next keeps the number it opens
/// with, which the gap counts (PB-47). The gap after the part's last show is the line's
/// to write ([`Pdf::written_lines`]).
fn shows_as_they_were(ops: &[Operation], t: &TextData, s: &Span) -> Shows {
    (s.runs.0..s.runs.1)
        .map(|k| {
            let run = &t.runs[k];
            let mut items = show_items(&ops[run.op], run.show);
            if k + 1 < s.runs.1 && run.gap > 0.0 {
                items.extend(to_next(t, k).map(|d| gap_move(d, run, s)));
            }
            (run.spacing, op("TJ", vec![Object::Array(items)]))
        })
        .collect()
}

/// How far show `k` of line `t` ends from where the next one starts, along the line: the
/// gap read as a space after it, less the number the next opens with.
fn to_next(t: &TextData, k: usize) -> Option<f32> {
    let (end, next) = (t.pieces[k].corners[1], t.pieces.get(k + 1)?.corners[0]);
    Some((next.0 - end.0) * t.dir.0 + (next.1 - end.1) * t.dir.1)
}

/// Shows of a part of a line, each with the character spacing, word spacing and horizontal
/// scaling (1 is none) it is drawn with ([`Written`]).
type Shows = Vec<([f32; 3], Operation)>;

/// The strings and spacing numbers a show draws, as a `TJ` array holds them.
pub(crate) fn show_items(o: &Operation, show: Show) -> Vec<Object> {
    match show {
        Show::TJ => o
            .operands
            .first()
            .and_then(|a| a.as_array().ok())
            .cloned()
            .unwrap_or_default(),
        Show::Tj | Show::Quote => o.operands.clone(),
        Show::DQuote { .. } => o.operands.iter().skip(2).cloned().collect(),
    }
}

/// The spacing part `s` of line `t` starts with.
fn run_spacing(t: &TextData, s: &Span) -> [f32; 3] {
    t.runs[s.runs.0].spacing
}

/// A move of `d` points after `run` of part `s`, as a `TJ` number: in thousandths of the
/// text size, in the font's own terms.
fn gap_move(d: f32, run: &Run, s: &Span) -> Object {
    r(-d * 1000.0 / show_units(run, s))
}

/// Shows kept as they were followed by a move `d` after the last, a number of its own as
/// [`shows_as_they_were`] writes one: [`tj`] would join it to a number the show ends with,
/// and the gap would no longer read as a space.
fn then_gap(shows: &mut Shows, d: Object) {
    if let Some(Object::Array(items)) = shows.last_mut().and_then(|(_, s)| s.operands.first_mut()) {
        items.push(d);
    }
}

/// The size of the units `run` of part `s` moves in: its text size in the font's own terms
/// times its horizontal scaling.
fn show_units(run: &Run, s: &Span) -> f32 {
    (s.size / s.em * run.spacing[2]).max(1e-3)
}

/// How one character of a line as it was read is drawn ([`drawn`]), to write it again with
/// its own code.
#[derive(Clone, Debug)]
pub(crate) struct Drawn {
    /// The show it is in ([`TextData::runs`]), and the spacing that show is drawn with
    /// ([`Run::spacing`]).
    run: usize,
    pub spacing: [f32; 3],
    pub ink: Ink,
    /// What the shows draw between the character before it and this one: what follows the
    /// one before in its show (spacing numbers, glyphs that read as no character, and the
    /// move to this one's show when that is another), in that show's units; then what comes
    /// before this one in its own show.
    pub after_last: Vec<Object>,
    pub before: Vec<Object>,
}

impl Drawn {
    /// The same drawing of a character read as character `to` of a longer text, not `from`
    /// of its line (a paragraph's, `rewrap.rs`, `to` at least `from`): a code's characters
    /// said by their places there.
    pub(crate) fn moved(&self, from: usize, to: usize) -> Drawn {
        let mut d = self.clone();
        if let Ink::Code { at, .. } = &mut d.ink {
            *at += to - from;
        }
        d
    }
}

/// What draws a character of a line ([`Drawn`]).
#[derive(Clone, Debug)]
pub(crate) enum Ink {
    /// The line's characters `at..at + n` are drawn by these bytes: one code (of several
    /// characters for a ligature), or a whole string whose codes LimarePDF can't be sure it
    /// parts as they are drawn ([`FontCtx::codes_known`], [`FontCtx::drawn_alike`]).
    Code {
        at: usize,
        n: usize,
        bytes: Vec<u8>,
        format: StringFormat,
    },
    /// A space drawn as a move: this spacing number, in its show's units.
    Move(f32),
}

/// How each character of line `t` is drawn by the page's operators `ops`, each of the
/// line's parts read with its font in `fonts` (`None` for one the page doesn't have, whose
/// bytes are then read one character each), as the parser read them: a code's characters,
/// a spacing number under -150 times the part's em ([`Span::em`]) after a character that
/// isn't a space read as a space, and
/// the gap read as a space after a show. The moves between shows are measured from where
/// one ends to where the next starts. `None` when the characters don't add up to the
/// line's, and in vertical writing, which is measured as if it ran across.
pub(crate) fn drawn(
    ops: &[Operation],
    t: &TextData,
    fonts: &[Option<&FontCtx>],
) -> Option<Vec<Drawn>> {
    if t.vertical {
        return None;
    }
    let along = |a: (f32, f32), b: (f32, f32)| (b.0 - a.0) * t.dir.0 + (b.1 - a.1) * t.dir.1;
    let mut out: Vec<Drawn> = Vec::with_capacity(t.text.len());
    // Where the text is after the last character's show (its end, or the start of the
    // next show when the gap to it was read as a space), and that show's units.
    let mut pos = t.pieces.first()?.corners[0];
    let mut units: Option<f32> = None;
    let mut after: Vec<Object> = Vec::new();
    for (s, &font) in t.spans.iter().zip(fonts) {
        for k in s.runs.0..s.runs.1 {
            let (run, (ra, rb)) = (&t.runs[k], t.pieces[k].chars);
            if ra == rb {
                continue;
            }
            let own = (s.size / s.em * run.spacing[2]).max(1e-3);
            let mut before = Vec::new();
            let d = along(pos, t.pieces[k].corners[0]);
            if d.abs() > 1e-3 {
                match units {
                    Some(u) => after.push(r(-d * 1000.0 / u)),
                    None => before.push(r(-d * 1000.0 / own)),
                }
            }
            let (mut at, mut text) = (ra, String::new());
            let mut put = |ink: Ink, after: &mut Vec<Object>, before: &mut Vec<Object>| {
                let (after_last, before) = (std::mem::take(after), std::mem::take(before));
                let drawn = Drawn {
                    run: k,
                    spacing: run.spacing,
                    ink,
                    after_last,
                    before,
                };
                out.push(drawn);
            };
            for it in show_items(&ops[run.op], run.show) {
                match &it {
                    Object::String(bytes, format) => {
                        let codes: Vec<(&[u8], String)> = match font {
                            Some(f) => (f.codes(bytes).into_iter())
                                .map(|(code, _, raw)| (raw, f.decode_code(code, raw)))
                                .collect(),
                            None => (bytes.chunks(1))
                                .map(|c| (c, (c[0] as char).to_string()))
                                .collect(),
                        };
                        let safe = font.is_none_or(|f| f.codes_known && f.drawn_alike(bytes));
                        let pieces: Vec<(Vec<u8>, String)> = match safe {
                            true => (codes.into_iter())
                                .map(|(raw, d)| (raw.to_vec(), d))
                                .collect(),
                            false => {
                                let text = codes.into_iter().map(|(_, d)| d).collect();
                                vec![(bytes.clone(), text)]
                            }
                        };
                        for (bytes, decoded) in pieces {
                            let n = decoded.chars().count();
                            if n == 0 {
                                // A glyph that reads as no character, between two that do.
                                before.push(Object::String(bytes, *format));
                                continue;
                            }
                            let ink = Ink::Code {
                                at,
                                n,
                                bytes,
                                format: *format,
                            };
                            for _ in 0..n {
                                put(ink.clone(), &mut after, &mut before);
                            }
                            text.push_str(&decoded);
                            at += n;
                        }
                    }
                    o => {
                        let x = o.as_float().unwrap_or(0.0);
                        if reads_as_space(x, s.em) && !text.ends_with(' ') && !text.is_empty() {
                            put(Ink::Move(x), &mut after, &mut before);
                            text.push(' ');
                            at += 1;
                        } else {
                            before.push(it.clone());
                        }
                    }
                }
            }
            // What follows the show's last character waits for the next one.
            after = std::mem::take(&mut before);
            pos = t.pieces[k].corners[1];
            if run.gap > 0.0 {
                let next = t.pieces.get(k + 1)?.corners[0];
                put(
                    Ink::Move(-along(pos, next) * 1000.0 / own),
                    &mut after,
                    &mut before,
                );
                at += 1;
                pos = next;
            }
            units = Some(own);
            if at != rb {
                return None;
            }
        }
    }
    (out.len() == t.text.chars().count()).then_some(out)
}

/// Show `show` followed by `more` (moves, and glyphs that read as no character), as a `TJ`.
fn then_move(show: &mut Operation, more: Vec<Object>) {
    let mut items = match show.operator.as_str() {
        "TJ" => show_items(show, Show::TJ),
        _ => show.operands.clone(),
    };
    items.extend(more);
    *show = tj(items);
}

/// Show `show` after a move `d` (a spacing number), as a `TJ`.
fn move_first(show: &mut Operation, d: Object) {
    let mut items = vec![d];
    items.extend(match show.operator.as_str() {
        "TJ" => show_items(show, Show::TJ),
        _ => show.operands.clone(),
    });
    *show = tj(items);
}

/// A `TJ` showing `items`, spacing numbers next to each other written as one and none left
/// that moves nothing: a reader takes each number alone, so a move back followed by one
/// forward again (across a `Tm` of the line as read) would read as a word gap.
pub(crate) fn tj(items: Vec<Object>) -> Operation {
    let number = |o: &Object| match o {
        Object::Integer(_) | Object::Real(_) => o.as_float().ok(),
        _ => None,
    };
    let mut out: Vec<Object> = Vec::with_capacity(items.len());
    for it in items {
        match (number(&it), out.last().and_then(number)) {
            (Some(x), Some(before)) => *out.last_mut().expect("a number") = r(before + x),
            _ => out.push(it),
        }
    }
    out.retain(|o| number(o).is_none_or(|x| x.abs() >= 0.01));
    op("TJ", vec![Object::Array(out)])
}

/// Line `t`, edited to `parts` written as `written`, as one show in place of its first:
/// the operator it goes in place of, and the show. Only for a line in one part, its style
/// unchanged, in one spacing, written all in its own font, that starts with a show of
/// characters (one of none before them may be in another font).
fn in_place(t: &TextData, parts: &[TextPart], written: &[Written]) -> Option<(usize, Operation)> {
    let ([s], Some(first)) = (&t.spans[..], t.runs.first()) else {
        return None;
    };
    let unchanged = |q: &TextPart| {
        q.font == PartFont::Span(0)
            && q.size == s.size
            && q.color == s.fill.unwrap_or([0.0; 3])
            && q.rise == s.rise
    };
    let starts = t.pieces.first().is_some_and(|p| p.chars.1 > p.chars.0);
    let spacing = first.spacing;
    let ok = !t.vertical
        && starts
        && parts.iter().all(unchanged)
        && t.runs.iter().all(|r| r.spacing == spacing)
        && !written.is_empty()
        && (written.iter())
            .all(|w| w.font == s.font && w.shows.iter().all(|(sp, _)| *sp == spacing));
    if !ok {
        return None;
    }
    let items = (written.iter())
        .flat_map(|w| &w.shows)
        .flat_map(|(_, show)| show_items(show, Show::TJ))
        .collect();
    Some((first.op, tj(items)))
}

/// Bytes `bytes` put at the end of a show's items `items`: in the string there when it is
/// in format `format`, else in a new one.
pub(crate) fn put_code(items: &mut Vec<Object>, bytes: &[u8], format: StringFormat) {
    match items.last_mut() {
        Some(Object::String(b, f)) if *f == format => b.extend_from_slice(bytes),
        _ => items.push(Object::String(bytes.to_vec(), format)),
    }
}

/// Whether a show's items `items` draw a character a move after them reads as a space
/// after: a string with bytes in it.
fn has_text(items: &[Object]) -> bool {
    items
        .iter()
        .any(|o| matches!(o, Object::String(b, _) if !b.is_empty()))
}

/// Writes the characters of a line kept in its own fonts, piece by piece
/// ([`Pdf::written_lines`]), keeping the spacing between neighbours that were neighbours
/// in the line as read.
struct OwnPen<'a> {
    t: &'a TextData,
    drawn: Option<&'a [Drawn]>,
    /// The line's character the last one written was, when it was written with what drew
    /// it there.
    last: Option<usize>,
    /// Near which of the line's characters the writing is, for a space's width.
    near: usize,
    /// Something of the line has been written.
    started: bool,
}

/// What a piece written by [`OwnPen`] puts at the end of the piece before it: what the
/// line drew between their characters, in that piece's last show's units, then a space
/// as a move (a move at the start of a show reads as no space): a spacing number, and the
/// size of its show's units (the text size times the horizontal scaling).
#[derive(Default)]
struct Tail {
    items: Vec<Object>,
    space: Option<(f32, f32)>,
}

impl<'a> OwnPen<'a> {
    fn new(t: &'a TextData, drawn: Option<&'a [Drawn]>) -> Self {
        Self {
            t,
            drawn,
            last: None,
            near: 0,
            started: false,
        }
    }

    /// Something not written with this pen (a part as it was, or in another font) comes
    /// next: the characters after it have no neighbours before them.
    fn apart(&mut self) {
        self.last = None;
        self.started = true;
    }

    /// The width of a space the font has no code for, as a spacing number in a show of
    /// spacing `spacing` of part `s`: the nearest move of the part read as a space, else a
    /// quarter of an em (of the font's own, [`Span::em`]).
    fn word_gap(&self, s: &Span, spacing: [f32; 3]) -> f32 {
        let gaps = (self.drawn.into_iter().flatten().enumerate())
            .filter(|(_, d)| (s.runs.0..s.runs.1).contains(&d.run))
            .filter_map(|(i, d)| match d.ink {
                Ink::Move(x) => Some((i, x * self.t.runs[d.run].spacing[2])),
                _ => None,
            });
        let nearest = gaps.min_by_key(|(i, _)| i.abs_diff(self.near));
        nearest.map_or(-250.0 * s.em, |(_, x)| x / spacing[2].max(1e-3))
    }

    /// Characters `range` of a part's text `text` in part `s` of the line, written as
    /// `ways` says ([`plan`]; none of them [`Way::Missing`]) in its font `font`, of size
    /// `size`: the shows, and what goes at the end of the piece before.
    fn write(
        &mut self,
        s: &Span,
        font: Option<&LineFont>,
        text: &[char],
        ways: &[Way],
        range: std::ops::Range<usize>,
        size: f32,
    ) -> (Tail, Shows) {
        let mut shows: Vec<([f32; 3], Vec<Object>)> = Vec::new();
        let mut tail = Tail::default();
        let first = !self.started;
        // The spacing of the first character written with what drew it, for the others.
        let own_spacing = (range.clone())
            .find_map(|j| match ways[j] {
                Way::Own(o) => Some(self.t.runs[self.drawn?[o].run].spacing),
                _ => None,
            })
            .unwrap_or_else(|| run_spacing(self.t, s));
        for j in range {
            let spacing = shows.last().map_or(own_spacing, |(sp, _)| *sp);
            let (item, spacing) = match ways[j] {
                Way::Own(o) => {
                    let Some(d) = self.drawn.and_then(|d| d.get(o)) else {
                        continue;
                    };
                    let next_to = o > 0 && self.last == Some(o - 1);
                    if next_to {
                        match shows.last_mut() {
                            Some((_, items)) => items.extend(d.after_last.iter().cloned()),
                            None => tail.items.extend(d.after_last.iter().cloned()),
                        }
                    }
                    let spacing = self.t.runs[d.run].spacing;
                    if shows.last().is_none_or(|(sp, _)| *sp != spacing) {
                        shows.push((spacing, Vec::new()));
                    }
                    if next_to || (!self.started && o == 0) {
                        let items = &mut shows.last_mut().expect("pushed").1;
                        items.extend(d.before.iter().cloned());
                    }
                    self.near = o;
                    match &d.ink {
                        Ink::Code { n, .. } => self.last = Some(o + n - 1),
                        Ink::Move(_) => self.last = Some(o),
                    }
                    (d.ink.clone(), spacing)
                }
                Way::WithBefore | Way::Missing => continue,
                Way::Code => {
                    let Some(bytes) = font.and_then(|f| f.code(text[j]).ok()) else {
                        continue;
                    };
                    let format = match bytes.len() {
                        1 => StringFormat::Literal,
                        _ => StringFormat::Hexadecimal,
                    };
                    self.last = None;
                    let code = Ink::Code {
                        at: 0,
                        n: 1,
                        bytes,
                        format,
                    };
                    (code, spacing)
                }
                Way::Space => {
                    self.last = None;
                    (Ink::Move(self.word_gap(s, spacing)), spacing)
                }
            };
            self.started = true;
            if shows.last().is_none_or(|(sp, _)| *sp != spacing) {
                shows.push((spacing, Vec::new()));
            }
            match item {
                Ink::Code { bytes, format, .. } => {
                    put_code(&mut shows.last_mut().expect("pushed").1, &bytes, format)
                }
                Ink::Move(x) => {
                    // A move reads as a space only after a character in its show: else at
                    // the end of the show before, or of the piece before.
                    let k = shows.len() - 1;
                    if has_text(&shows[k].1) || (k == 0 && first) {
                        shows[k].1.push(r(x));
                    } else if k > 0 {
                        let th = shows[k - 1].0[2] / spacing[2].max(1e-3);
                        shows[k - 1].1.push(r(x / th.max(1e-3)));
                    } else {
                        tail.space = Some((x, size * spacing[2]));
                    }
                }
            }
        }
        let shows = (shows.into_iter())
            .filter(|(_, items)| !items.is_empty())
            .map(|(sp, items)| (sp, tj(items)))
            .collect();
        (tail, shows)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathStyle {
    pub fill: Option<[f32; 3]>,
    pub stroke: Option<[f32; 3]>,
    /// In points on the page.
    pub line_width: f32,
}

impl Default for PathStyle {
    fn default() -> Self {
        Self {
            fill: None,
            stroke: Some([0.0, 0.0, 0.0]),
            line_width: 1.5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Ellipse,
    /// Drawn from the first corner of the rectangle to the opposite one.
    Line,
}

pub(crate) fn op(name: &str, operands: Vec<Object>) -> Operation {
    Operation::new(name, operands)
}
pub(crate) fn r(x: f32) -> Object {
    Object::Real(x)
}
fn name(n: &str) -> Object {
    Object::Name(n.as_bytes().to_vec())
}
/// `prefix1`, `prefix2`…: the names [`Pdf::add_resource`] tries, and those a form's standard
/// font tries after its own (S1-31).
pub(crate) fn numbered(prefix: &str) -> impl Iterator<Item = String> + '_ {
    (1u64..).map(move |n| format!("{prefix}{n}"))
}
fn cm(m: M) -> Operation {
    op("cm", m.iter().map(|&x| r(x)).collect())
}
/// The `cm` that turns a picture stored in orientation `o` upright within its unit
/// square, when it needs turning.
fn upright(o: Orientation) -> Option<Operation> {
    (o != Orientation::NoTransforms).then(|| cm(orient_matrix(o, 1.0, 1.0)))
}
fn rgb(o: &str, c: [f32; 3]) -> Operation {
    op(o, c.iter().map(|&x| r(x.clamp(0.0, 1.0))).collect())
}
#[cfg(test)]
fn lit(b: Vec<u8>) -> Object {
    Object::String(b, StringFormat::Literal)
}

/// What a show operation leaves behind when its text is removed (the line move of ' and ").
pub(crate) fn move_only(show: Show) -> Vec<Operation> {
    match show {
        Show::Tj | Show::TJ => vec![],
        Show::Quote => vec![op("T*", vec![])],
        Show::DQuote { aw, ac } => vec![
            op("Tw", vec![r(aw)]),
            op("Tc", vec![r(ac)]),
            op("T*", vec![]),
        ],
    }
}

/// Whether `ops` change the graphics or text state outside any q/Q, so that content added
/// after them would inherit it. Pages printed by browsers flip and scale the whole page with
/// a `cm` at this level, for instance.
fn leaks_state(ops: &[Operation]) -> bool {
    let mut depth = 0usize;
    ops.iter().any(|o| match o.operator.as_str() {
        "q" => {
            depth += 1;
            false
        }
        "Q" => {
            depth = depth.saturating_sub(1);
            false
        }
        _ if depth > 0 => false,
        // Text positioning (reset by BT), showing, paths without clipping, marked content,
        // and drawing that leaves the state alone.
        "BT" | "ET" | "Td" | "Tm" | "T*" | "Tj" | "TJ" | "'" | "m" | "l" | "c" | "v" | "y"
        | "h" | "re" | "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" | "BMC"
        | "BDC" | "EMC" | "MP" | "DP" | "Do" | "sh" | "BI" | "ID" | "EI" | "BX" | "EX" => false,
        _ => true,
    })
}

/// `ops` made ready for content to be appended in the page's default state: the q/Q they
/// leave open closed, and the whole wrapped in q/Q when they change the state outside one.
/// Content that needs no wrapping is left as it is, so repeated edits don't nest deeper.
pub(crate) fn closed(mut ops: Vec<Operation>, depth_left: usize) -> Vec<Operation> {
    let wrap = leaks_state(&ops);
    for _ in 0..depth_left {
        ops.push(op("Q", vec![]));
    }
    if wrap {
        ops.insert(0, op("q", vec![]));
        ops.push(op("Q", vec![]));
    }
    ops
}

/// What keeps the text after a line in place when the line's shows are taken out, rewritten
/// or moved. The text after it can carry on from where the line ended (a show with no line
/// move before it), and the shows that are left don't end where the old ones did. `Tm` puts
/// the text matrix back before that show; as `Tm` sets the line matrix too, which the old
/// shows never changed, another `Tm` puts that back before the next line move. Not after
/// vertical writing, whose end isn't known (it is measured as if it ran across).
#[derive(Clone, Copy)]
pub(crate) struct Following {
    /// The show that carries on from the line, and the first line move after it.
    show: Option<usize>,
    line: Option<usize>,
    tm: M,
    tlm: M,
    vertical: bool,
}

impl Following {
    pub(crate) fn of(ops: &[Operation], obj: &Obj) -> Self {
        let (tm, tlm) = obj
            .text
            .as_ref()
            .map_or((mx::ID, mx::ID), |t| (t.tm_end, t.tlm_end));
        let vertical = obj.text.as_ref().is_some_and(|t| t.vertical);
        // The first operator from `from` on that sets the text matrix, or that shows text.
        let next = |from: usize, shows: bool| {
            (from..ops.len()).find(|&i| match ops[i].operator.as_str() {
                "Tj" | "TJ" => shows,
                "'" | "\"" | "Td" | "TD" | "T*" | "Tm" | "BT" | "ET" => true,
                _ => false,
            })
        };
        let is = |i: &usize, names: &[&str]| names.contains(&ops[*i].operator.as_str());
        let show = obj
            .text
            .as_ref()
            .and_then(|_| next(obj.last + 1, true))
            .filter(|i| is(i, &["Tj", "TJ"]));
        let line = show
            .and_then(|s| next(s + 1, false))
            .filter(|i| is(i, &["'", "\"", "Td", "TD", "T*"]));
        Self {
            show,
            line,
            tm,
            tlm,
            vertical,
        }
    }

    /// What goes in before operator `i`.
    fn before(&self, i: usize) -> Option<Operation> {
        if self.vertical {
            return None;
        }
        let m = if self.show == Some(i) {
            self.tm
        } else if self.line == Some(i) {
            self.tlm
        } else {
            return None;
        };
        Some(op("Tm", m.iter().map(|&x| r(x)).collect()))
    }

    /// What goes in before which operators after several lines, `fs` in the page's order:
    /// for each operator, what each line puts in before it ([`Following::before`]), the
    /// later lines' last. Found once, so a page of many lines is gone through once.
    pub(crate) fn all(fs: impl IntoIterator<Item = Following>) -> HashMap<usize, Vec<Operation>> {
        let mut out: HashMap<usize, Vec<Operation>> = HashMap::new();
        for f in fs {
            for i in [f.show, f.line].into_iter().flatten() {
                out.entry(i).or_default().extend(f.before(i));
            }
        }
        out
    }
}

/// Operators that set the graphics or text state, which stays set after them.
const SETS_STATE: [&str; 25] = [
    "Tf", "Tc", "Tw", "Tz", "TL", "Ts", "Tr", "g", "rg", "k", "G", "RG", "K", "cs", "CS", "sc",
    "scn", "SC", "SCN", "gs", "w", "J", "j", "M", "d",
];

/// The same text shown with a plain Tj/TJ (no line move), for use after an explicit Tm.
fn plain_show(o: &Operation, show: Show) -> Operation {
    match show {
        Show::Tj | Show::TJ => o.clone(),
        Show::Quote => op("Tj", o.operands.clone()),
        Show::DQuote { .. } => op("Tj", o.operands.iter().skip(2).cloned().collect()),
    }
}

impl Pdf {
    pub(crate) fn load_edit(&self, page_index: usize) -> Result<(ObjectId, Parsed)> {
        let page = self.page_id_at(page_index)?;
        let parsed = self.parse_page(page_index)?;
        if parsed.ops.iter().any(|o| o.operator == "BI") {
            bail!("this page has an inline image that cannot be preserved on rewrite; editing the page is disabled");
        }
        Ok((page, parsed))
    }

    pub(crate) fn pick(parsed: &Parsed, id: usize) -> Result<&Obj> {
        parsed
            .objs
            .get(id)
            .ok_or_else(|| anyhow!("object {id} no longer exists (the page changed)"))
    }

    /// Write `ops` back as the page's only content stream. Open `q` operators are closed first.
    pub(crate) fn commit(
        &mut self,
        page: ObjectId,
        mut ops: Vec<Operation>,
        depth_left: usize,
        inline: &Inline,
    ) -> Result<()> {
        for _ in 0..depth_left {
            ops.push(op("Q", vec![]));
        }
        let expect = ops.len();
        let bytes = inline
            .encode(ops)
            .map_err(|e| anyhow!("cannot encode the page content: {e}"))?;
        // Read back as the page will be read next time, inline images lifted out first.
        let (plain, _) = crate::inline::extract(&bytes)
            .map_err(|e| anyhow!("edited content does not parse: {e}"))?;
        let back =
            Content::decode(&plain).map_err(|e| anyhow!("edited content does not parse: {e}"))?;
        if back.operations.len() != expect {
            bail!("the edited page content did not round-trip safely; nothing was changed");
        }
        let id = self.doc.add_object(Stream::new(Dictionary::new(), bytes));
        self.doc
            .get_dictionary_mut(page)?
            .set("Contents", Object::Reference(id));
        Ok(())
    }

    /// Register `obj` in the page's resource category (`Font`, `XObject`) under a fresh name.
    pub(crate) fn add_resource(
        &mut self,
        page: ObjectId,
        cat: &str,
        prefix: &str,
        obj: Object,
    ) -> Result<String> {
        let mut res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
        let mut d = self.resolved_dict(res.get(cat.as_bytes()).ok());
        let nm = numbered(prefix)
            .find(|c| !d.has(c.as_bytes()))
            .ok_or_else(|| anyhow!("no free name for a {cat} resource"))?;
        d.set(nm.as_bytes().to_vec(), obj);
        res.set(cat, d);
        self.doc.get_dictionary_mut(page)?.set("Resources", res);
        Ok(nm)
    }

    /// The page's own `/Resources` entry, for [`Pdf::put_resources`] to put back when a
    /// try at writing is given up (PB-46). [`Pdf::add_resource`] writes the names it adds
    /// into a dictionary of the page's own and never changes one it refers to, so this
    /// entry is all such a try changed there.
    pub(crate) fn page_resources(&self, page: ObjectId) -> Result<Option<Object>> {
        Ok(self
            .doc
            .get_dictionary(page)?
            .get(b"Resources")
            .ok()
            .cloned())
    }

    /// Put the page's own `/Resources` entry back as [`Pdf::page_resources`] found it: the
    /// names a try that was given up added are gone, and the next try adds only its own.
    pub(crate) fn put_resources(&mut self, page: ObjectId, was: Option<Object>) -> Result<()> {
        let d = self.doc.get_dictionary_mut(page)?;
        match was {
            Some(res) => d.set("Resources", res),
            None => {
                d.remove(b"Resources");
            }
        }
        Ok(())
    }

    /// The pen that writes `texts` as `setter` sets them ([`Pdf::pen_with`]: the standard
    /// font, or its embedded face when the text needs it, and the fonts it falls back to),
    /// its fonts added to the page's resources: their names there, and the pen.
    pub(crate) fn text_pen(
        &mut self,
        page: ObjectId,
        setter: &Setter,
        texts: &[&str],
    ) -> Result<(Vec<String>, Pen)> {
        let pen = self.pen_with(setter, texts)?;
        let names = self.font_names(page, "LimareEd", pen.fonts())?;
        Ok((names, pen))
    }

    /// D in user space for a displayed-page (y down) delta.
    fn user_delta(vd: M, d_vis: M) -> Result<M> {
        let inv = mx::inv(vd).ok_or_else(|| anyhow!("degenerate page transform"))?;
        Ok(mul(mul(inv, d_vis), vd))
    }

    /// The objects `ids` name, in the order given, which must be the page's order (ids
    /// ascending); and for each of the page's operators, which of them it belongs to. An id
    /// given twice is refused, as is one whose operators another of them has.
    fn pick_all(
        parsed: &Parsed,
        ids: impl IntoIterator<Item = usize>,
    ) -> Result<(Vec<Obj>, Vec<Option<usize>>)> {
        let mut owner = vec![None; parsed.ops.len()];
        let mut objs = Vec::new();
        for (k, id) in ids.into_iter().enumerate() {
            let obj = Self::pick(parsed, id)?;
            for slot in &mut owner[obj.first..=obj.last] {
                if slot.replace(k).is_some() {
                    bail!("object {id} was given twice, or shares its content with another one");
                }
            }
            objs.push(obj.clone());
        }
        Ok((objs, owner))
    }

    /// Move and/or resize an object so that its bounding box becomes `new_box` (displayed-page points, y down).
    pub fn set_object_bbox(
        &mut self,
        page_index: usize,
        id: usize,
        new_box: [f32; 4],
    ) -> Result<()> {
        self.set_object_boxes(page_index, &[(id, new_box)])
    }

    /// Move and/or resize several objects of one page at once, each so that its bounding box
    /// becomes the one given (displayed-page points, y down), in one rewrite of the page's
    /// content: the ids are as the page is now. If any can't be changed, nothing is.
    pub fn set_object_boxes(
        &mut self,
        page_index: usize,
        boxes: &[(usize, [f32; 4])],
    ) -> Result<()> {
        let objs = self.page_objects(page_index)?;
        let moves = boxes
            .iter()
            .map(|&(id, new_box)| {
                let old = objs
                    .get(id)
                    .ok_or_else(|| anyhow!("object {id} no longer exists"))?
                    .bbox;
                let (ow, oh) = (old[2] - old[0], old[3] - old[1]);
                let (nw, nh) = (new_box[2] - new_box[0], new_box[3] - new_box[1]);
                // Only making it that small is refused: a hairline moves as thin as it is.
                let shrunk = |n: f32, o: f32| n < 0.5 && n < o - 1e-3;
                if shrunk(nw, ow) || shrunk(nh, oh) {
                    bail!("the object would become too small");
                }
                let sx = if ow > 1e-3 { nw / ow } else { 1.0 };
                let sy = if oh > 1e-3 { nh / oh } else { 1.0 };
                let d = [
                    sx,
                    0.0,
                    0.0,
                    sy,
                    new_box[0] - old[0] * sx,
                    new_box[1] - old[1] * sy,
                ];
                Ok((id, d))
            })
            .collect::<Result<Vec<_>>>()?;
        self.transform_objects(page_index, &moves)
    }

    /// Apply `d_vis` (a matrix in displayed-page space, y down) to an object.
    pub fn transform_object(&mut self, page_index: usize, id: usize, d_vis: M) -> Result<()> {
        self.transform_objects(page_index, &[(id, d_vis)])
    }

    /// Apply a matrix in displayed-page space (y down) to each of several objects of one
    /// page, in one rewrite of its content: the ids are as the page is now. If any can't be
    /// changed, nothing is.
    pub fn transform_objects(&mut self, page_index: usize, moves: &[(usize, M)]) -> Result<()> {
        let (page, parsed) = self.load_edit(page_index)?;
        let (vd, _, _) = self.vis_down(page);
        let mut moves = moves.to_vec();
        // A line's underlines and strike-throughs move as it does.
        let with: Vec<(usize, M)> = (parsed.decorations.iter())
            .filter_map(|(line, d)| Some((d, moves.iter().find(|(id, _)| id == line)?.1)))
            .flat_map(|(d, m)| d.paths.iter().map(move |&p| (p, m)))
            .filter(|(p, _)| !moves.iter().any(|(id, _)| id == p))
            .collect();
        moves.extend(with);
        moves.sort_by_key(|&(id, _)| id);
        let (objs, owner) = Self::pick_all(&parsed, moves.iter().map(|&(id, _)| id))?;
        let ms = (objs.iter().zip(&moves))
            .map(|(obj, &(_, d_vis))| {
                let d = Self::user_delta(vd, d_vis)?;
                let ictm = mx::inv(obj.ctm).ok_or_else(|| {
                    anyhow!("the object has a degenerate transform and cannot be edited")
                })?;
                Ok(mul(mul(obj.ctm, d), ictm))
            })
            .collect::<Result<Vec<M>>>()?;
        let following: Vec<Following> =
            objs.iter().map(|o| Following::of(&parsed.ops, o)).collect();
        // A moved line puts the text matrix back itself for a show that carries on from it
        // (below): only the line move after that needs putting back.
        let fixes = Following::all(following.iter().map(|f| Following { show: None, ..*f }));
        let fix = |i: usize| fixes.get(&i).into_iter().flatten().cloned();
        let mut out: Vec<Operation> = Vec::with_capacity(parsed.ops.len() + 8 * objs.len());
        for (i, o) in parsed.ops.iter().enumerate() {
            let Some(k) = owner[i] else {
                out.extend(fix(i));
                out.push(o.clone());
                continue;
            };
            let (obj, m) = (&objs[k], ms[k]);
            // A moved line is written whole at its first operator: what goes in before the
            // others would go into nothing.
            if obj.text.is_some() && i > obj.first {
                continue;
            }
            out.extend(fix(i));
            if i == obj.first && obj.kind != ObjectKind::Text {
                out.push(op("q", vec![]));
                out.push(cm(m));
            }
            if let (Some(t), true) = (&obj.text, i == obj.first) {
                out.extend([op("ET", vec![]), op("q", vec![]), cm(m), op("BT", vec![])]);
                // What the line's operators set besides its text and where it goes (the
                // font and colour of each of its parts, for one) is set again after the
                // Q, so the content after the line is drawn as before.
                let mut state = Vec::new();
                for j in obj.first..=obj.last {
                    let o2 = &parsed.ops[j];
                    if let Some(run) = t.runs.iter().find(|r| r.op == j) {
                        if let Show::DQuote { aw, ac } = run.show {
                            let set = [op("Tw", vec![r(aw)]), op("Tc", vec![r(ac)])];
                            out.extend(set.clone());
                            state.extend(set);
                        }
                        out.push(op("Tm", run.tm.iter().map(|&x| r(x)).collect()));
                        out.push(plain_show(o2, run.show));
                    } else {
                        match o2.operator.as_str() {
                            "Td" | "Tm" | "T*" => {}
                            "TD" => {
                                let tl = o2.operands.get(1).and_then(|x| x.as_float().ok());
                                let o3 = op("TL", vec![r(-tl.unwrap_or(0.0))]);
                                out.push(o3.clone());
                                state.push(o3);
                            }
                            name => {
                                out.push(o2.clone());
                                if SETS_STATE.contains(&name) {
                                    state.push(o2.clone());
                                }
                            }
                        }
                    }
                }
                // Restore the text matrix the following operators expect: here, as BT has
                // just reset it, and the line matrix later if a show carries on first.
                let tm = match following[k].show {
                    Some(_) => t.tm_end,
                    None => t.tlm_end,
                };
                out.extend([op("ET", vec![]), op("Q", vec![])]);
                out.extend(state);
                out.extend([
                    op("BT", vec![]),
                    op("Tm", tm.iter().map(|&x| r(x)).collect()),
                ]);
                continue;
            }
            out.push(o.clone());
            if i == obj.last && obj.kind != ObjectKind::Text {
                out.push(op("Q", vec![]));
            }
        }
        self.commit(page, out, parsed.depth_left, &parsed.inline)
    }

    pub fn delete_object(&mut self, page_index: usize, id: usize) -> Result<()> {
        self.delete_objects(page_index, &[id])
    }

    /// Delete several objects of one page at once, in one rewrite of its content: the ids
    /// are as the page is now. If any can't be deleted, nothing is. A line's underlines and
    /// strike-throughs go with it.
    pub fn delete_objects(&mut self, page_index: usize, ids: &[usize]) -> Result<()> {
        let (page, parsed) = self.load_edit(page_index)?;
        let out = Self::without_objects(&parsed, &Self::with_decorations(&parsed, ids))?;
        self.commit(page, out, parsed.depth_left, &parsed.inline)
    }

    /// Objects `ids` of the page `parsed` reads, with the paths drawing the underlines and
    /// strike-throughs of the lines among them.
    pub(crate) fn with_decorations(parsed: &Parsed, ids: &[usize]) -> Vec<usize> {
        let mut all = ids.to_vec();
        for (_, d) in (parsed.decorations.iter()).filter(|(line, _)| ids.contains(line)) {
            for &p in &d.paths {
                if !all.contains(&p) {
                    all.push(p);
                }
            }
        }
        all
    }

    /// The page's operators, as `parsed`, with the objects `ids` taken out: a text object's
    /// shows (their moves kept, so what follows stays where it was), a path's painting (its
    /// construction and any clip kept), a picture or form's drawing.
    pub(crate) fn without_objects(parsed: &Parsed, ids: &[usize]) -> Result<Vec<Operation>> {
        let mut ids = ids.to_vec();
        ids.sort_unstable();
        let (objs, owner) = Self::pick_all(parsed, ids)?;
        let fixes = Following::all(objs.iter().map(|o| Following::of(&parsed.ops, o)));
        let mut out = Vec::with_capacity(parsed.ops.len());
        for (i, o) in parsed.ops.iter().enumerate() {
            out.extend(fixes.get(&i).into_iter().flatten().cloned());
            let Some(obj) = owner[i].map(|k| &objs[k]) else {
                out.push(o.clone());
                continue;
            };
            match obj.kind {
                ObjectKind::Text => {
                    let t = obj.text.as_ref().unwrap();
                    match t.runs.iter().find(|r| r.op == i) {
                        Some(run) => out.extend(move_only(run.show)),
                        None => out.push(o.clone()),
                    }
                }
                ObjectKind::Path => {
                    // Keep the path construction (and any clip) but paint nothing.
                    if i == obj.last {
                        out.push(op("n", vec![]));
                    } else {
                        out.push(o.clone());
                    }
                }
                ObjectKind::Image | ObjectKind::Form => {}
            }
        }
        Ok(out)
    }

    /// Change the text of a text object. Its characters keep the line's own font where it
    /// has a verified code for each ([`PageObject::kept_pieces`]); a word it can't write goes
    /// to the installed family the font is from or the nearest standard font (or its
    /// embedded face), which fall back to other fonts for characters they lack
    /// (`layout.rs`). A line in parts ([`TextEdit::parts`]) is written as one line, each
    /// part in its style; a line changed in words alone, all in its own font, in place.
    pub fn edit_text(&mut self, page_index: usize, id: usize, e: &TextEdit) -> Result<()> {
        self.edit_texts(page_index, &[(id, e.clone())]).map(|_| ())
    }

    /// Change several text objects of one page at once, each as [`Pdf::edit_text`] would,
    /// in one rewrite of the page's content: the ids are as the page is now, and the lines
    /// written anew go on top in the order given. A line with characters written in its own
    /// fonts ([`PageObject::kept_pieces`]) is read back: one that doesn't read as it should
    /// then is written with only its parts left as they were in them. Returns the ids of
    /// those lines, whose characters were written in another font after all. A line's
    /// underlines and strike-throughs ([`PageObject::decorations`]) go with its characters
    /// ([`crate::decorate`]'s `carried`): drawn where they are now, in their own look, and
    /// left as they were drawn where their characters didn't move.
    pub fn edit_texts(
        &mut self,
        page_index: usize,
        edits: &[(usize, TextEdit)],
    ) -> Result<Vec<usize>> {
        self.edit_texts_with(page_index, edits, None)
    }

    /// [`Pdf::edit_texts`] with each line's underlines and strike-throughs as given (over its
    /// edit's text), in place of its old ones: an old one given again in its look over the
    /// same characters, where they were, stays as it was drawn.
    pub fn edit_texts_decorated(
        &mut self,
        page_index: usize,
        edits: &[(usize, TextEdit, Vec<Decorated>)],
    ) -> Result<Vec<usize>> {
        let (plain, asked): (Vec<_>, Vec<_>) = (edits.iter())
            .map(|(id, e, d)| ((*id, e.clone()), d.clone()))
            .unzip();
        self.edit_texts_with(page_index, &plain, Some(&asked))
    }

    fn edit_texts_with(
        &mut self,
        page_index: usize,
        edits: &[(usize, TextEdit)],
        asked: Option<&[Vec<Decorated>]>,
    ) -> Result<Vec<usize>> {
        let (page, parsed) = self.load_edit(page_index)?;
        // The lines' decorations as read, and the operators that paint them.
        let old: Vec<Vec<&Decoration>> = (edits.iter())
            .map(|(id, _)| {
                (parsed.decorations.iter())
                    .filter(|(line, _)| line == id)
                    .map(|(_, d)| d)
                    .collect()
            })
            .collect();
        let lines: Vec<&Obj> = (edits.iter())
            .map(|(id, _)| Self::pick(&parsed, *id))
            .collect::<Result<_>>()?;
        // A line whose words stay as they are isn't written: only its decorations change.
        let unchanged: Vec<bool> = (edits.iter().zip(&lines))
            .map(|((_, e), o)| o.text.as_ref().is_some_and(|t| e.unchanged(t)))
            .collect();
        let writing: Vec<(usize, TextEdit)> = (edits.iter().zip(&unchanged))
            .filter(|(_, u)| !**u)
            .map(|(e, _)| e.clone())
            .collect();
        let mut watch: HashSet<usize> = (old.iter().flatten())
            .flat_map(|d| {
                d.paths
                    .iter()
                    .filter_map(|&p| Some(parsed.objs.get(p)?.last))
            })
            .collect();
        for (o, _) in lines.iter().zip(&unchanged).filter(|(_, u)| **u) {
            watch.extend(o.first..=o.last);
        }
        let objs: Vec<&Obj> = (lines.iter().zip(&unchanged))
            .filter(|(_, u)| !**u)
            .map(|(o, _)| *o)
            .collect();
        let (vd, _, _) = self.vis_down(page);
        let to_vis = mx::inv(vd).ok_or_else(|| anyhow!("degenerate page transform"))?;
        let mut own = vec![true; writing.len()];
        // Lines written again because their words moved: held where the first try was.
        let mut moved = vec![false; writing.len()];
        let mut lost = Vec::new();
        let resources = self.page_resources(page)?;
        loop {
            let e = self.edited_ops(page, &parsed, &writing, &own, &watch)?;
            let misread = self.misread(page, &parsed.inline, &e.ops, &e.checks)?;
            // Lines that read as typed keep their words where they were (S1-14).
            let misplaced = match misread.is_empty() {
                true => {
                    let held: Vec<bool> = own.iter().zip(&moved).map(|(o, m)| *o || *m).collect();
                    let want = kept_extents(&objs, &writing, &e.places, &held, to_vis);
                    self.misplaced(page, &parsed.inline, &e.ops, &want, 0.5)?
                }
                false => Vec::new(),
            };
            if misread.is_empty() && misplaced.is_empty() {
                let lines = Lines {
                    edits,
                    objs: &lines,
                    unchanged: &unchanged,
                    asked,
                    old: &old,
                };
                let (ops, depth) = self.decorated(page, &parsed, &lines, e)?;
                self.commit(page, ops, depth, &parsed.inline)?;
                return Ok(lost);
            }
            // The try is given up: the fonts it named go with it (PB-46).
            self.put_resources(page, resources.clone())?;
            for k in misread {
                if !own[k] {
                    bail!(
                        "a line of the edit doesn't read back as it was typed; nothing was changed"
                    );
                }
                own[k] = false;
                lost.push(writing[k].0);
            }
            for k in misplaced {
                if !own[k] {
                    bail!("a line of the edit doesn't keep its words where they were; nothing was changed");
                }
                own[k] = false;
                moved[k] = true;
                lost.push(writing[k].0);
            }
        }
    }

    /// The page's operators `e`, with each edited line's decorations drawn for its new text:
    /// as asked, else its old ones carried with their characters. An old one not drawn again
    /// as it was stops painting; new ones go on top.
    fn decorated(
        &self,
        page: ObjectId,
        parsed: &Parsed,
        l: &Lines,
        e: Edited,
    ) -> Result<(Vec<Operation>, usize)> {
        let (edits, lines, old) = (l.edits, l.objs, l.old);
        let origins: Vec<Vec<Option<usize>>> = (edits.iter().zip(lines))
            .map(|((_, edit), o)| {
                let t = o.text.as_ref().expect("text");
                edit.origins(&t.text, t.spans.first().map(|s| (s.size, s.fill, s.rise)))
            })
            .collect();
        let want: Vec<Vec<Decorated>> = match l.asked {
            Some(asked) => asked.to_vec(),
            None => (old.iter().zip(&origins))
                .map(|(o, origins)| crate::decorate::carried(o, origins))
                .collect(),
        };
        if want.iter().all(Vec::is_empty) && old.iter().all(Vec::is_empty) {
            return Ok((e.ops, e.depth));
        }
        let bytes = (parsed.inline.encode(e.ops.clone()))
            .map_err(|err| anyhow!("cannot encode the page content: {err}"))?;
        let back = self.parse_as_page(page, &bytes)?;
        // Where each line is: written, or as it was where its operators went.
        let mut written = e.places.iter();
        let places: Vec<Option<Check>> = (lines.iter().zip(l.unchanged))
            .map(|(o, &u)| match u {
                true => Some(Check::Within(
                    *e.watched.get(&o.first)?,
                    *e.watched.get(&o.last)? + 1,
                )),
                false => written.next().copied().flatten(),
            })
            .collect();
        let mut ops = e.ops;
        let mut added = Vec::new();
        for (k, o) in lines.iter().enumerate() {
            let t = o.text.as_ref().expect("text");
            let kept = match places[k] {
                Some(place) => {
                    let was = Laid::of(t, o.ctm);
                    let line = Redraw {
                        text: &edits[k].1.text,
                        want: &want[k],
                        old: &old[k],
                        was: was.as_ref(),
                        origins: &origins[k],
                    };
                    let (more, kept) = self.decoration_ops(page, &back, place, &line);
                    added.extend(more);
                    kept
                }
                None => vec![false; old[k].len()],
            };
            for (d, kept) in old[k].iter().zip(kept) {
                for &p in d.paths.iter().filter(|_| !kept) {
                    let at = (parsed.objs.get(p)).and_then(|p| e.watched.get(&p.last));
                    if let Some(&at) = at {
                        ops[at] = op("n", vec![]);
                    }
                }
            }
        }
        if added.is_empty() {
            return Ok((ops, e.depth));
        }
        let mut ops = closed(ops, e.depth);
        ops.extend(added);
        Ok((ops, 0))
    }

    /// The page's operators with `edits` made ([`Pdf::edit_texts`]), each edit's characters
    /// written in its line's own fonts only where `own` says.
    fn edited_ops(
        &mut self,
        page: ObjectId,
        parsed: &Parsed,
        edits: &[(usize, TextEdit)],
        own: &[bool],
        watch: &HashSet<usize>,
    ) -> Result<Edited> {
        // A line is written in place of its first show, as one show in its own font, or on
        // top of the page without its shows.
        enum Plan {
            InPlace(usize, Operation),
            OnTop(Vec<Operation>),
        }
        let mut lines: Vec<(&Obj, &TextData)> = Vec::new();
        for (id, e) in edits {
            let joined: String = e.parts.iter().map(|p| p.text.as_str()).collect();
            if !e.parts.is_empty() && joined != e.text {
                bail!("the parts of the edit don't make up its text");
            }
            if edits.iter().filter(|(other, _)| other == id).count() > 1 {
                bail!("object {id} is changed twice in one edit");
            }
            let obj = Self::pick(parsed, *id)?;
            let t = (obj.text.as_ref()).ok_or_else(|| anyhow!("object {id} is not text"))?;
            lines.push((obj, t));
        }
        // The parts each line is written in, the lines in parts written together, so they
        // share the fonts they add.
        let parts: Vec<_> = (edits.iter().zip(&lines))
            .map(|((_, e), (_, t))| {
                let first = t.spans.first().map(|s| (s.size, s.fill, s.rise));
                (!t.hidden).then(|| e.parts_for(&t.text, first)).flatten()
            })
            .collect();
        let asked: Vec<Asked> = (parts.iter().zip(&lines).zip(own))
            .filter_map(|((p, (_, t)), &own)| Some((Some((parsed, *t)), &p.as_ref()?[..], own)))
            .collect();
        let mut written = self.written_lines(page, &asked)?.into_iter();
        let mut plans = Vec::new();
        // Whether each line is read back, with what it should read.
        let mut check = vec![None; edits.len()];
        for (k, ((_, e), (_, t))) in edits.iter().zip(&lines).enumerate() {
            let (dx, dy) = t.dir;
            let tm = [dx, dy, -dy, dx, t.origin.0, t.origin.1];
            let at = op("Tm", tm.iter().map(|&x| r(x)).collect());
            let plan = if t.hidden {
                // Invisible text (an OCR layer): the line replaced with one invisible show
                // of the new text, stretched over the line's original extent, so that it
                // stays invisible and lines up with the image. Style changes do not apply.
                // Empty text removes the line.
                let mut ops = Vec::new();
                if !e.text.trim().is_empty() {
                    let (first, last) = (&t.pieces[0], &t.pieces[t.pieces.len() - 1]);
                    let (s, f) = (first.corners[0], last.corners[1]);
                    let width = ((f.0 - s.0) * dx + (f.1 - s.1) * dy).max(0.1);
                    let size = t.size.max(1.0);
                    let setter = self.setter(FontChoice::default(), &e.text);
                    let (names, pen) = self.text_pen(page, &setter, &[&e.text])?;
                    let wide = setter.width(&e.text, size).max(0.1);
                    let tz = (100.0 * width / wide).clamp(1.0, 10_000.0);
                    ops.extend([
                        op("q", vec![]),
                        op("BT", vec![]),
                        op("Tr", vec![Object::Integer(3)]),
                        op("Tf", vec![name(&names[0]), r(size)]),
                        op("Tz", vec![r(tz)]),
                        at,
                    ]);
                    ops.extend(pen.ops(0, size, 0.0, &names));
                    ops.extend([op("ET", vec![]), op("Q", vec![])]);
                }
                Plan::OnTop(ops)
            } else if let Some(parts) = &parts[k] {
                // In parts: in place, as one show, when the line is in one style and
                // spacing, changed in words only, all in its own font; else on top as one
                // line.
                let (written, pen) = written.next().expect("written");
                if pen {
                    check[k] = Some(e.text.clone());
                }
                match in_place(t, parts, &written) {
                    Some((first, show)) => Plan::InPlace(first, show),
                    None => {
                        let mut ops = Vec::new();
                        if !written.is_empty() {
                            ops.push(op("q", vec![]));
                            ops.extend(self.turned(page, line_ops(tm, &written), &written)?);
                            ops.push(op("Q", vec![]));
                        }
                        Plan::OnTop(ops)
                    }
                }
            } else if e.text.is_empty() {
                Plan::OnTop(Vec::new())
            } else {
                // Restyled in one style: the new text on top.
                let font = e.font.unwrap_or_else(|| FontChoice::of_font(&t.base));
                let size = e.size.unwrap_or(t.size).max(1.0);
                let color = e.color.or(t.fill).unwrap_or([0.0; 3]);
                let setter = self.setter(font, &e.text);
                let (names, pen) = self.text_pen(page, &setter, &[&e.text])?;
                let mut ops = vec![
                    op("q", vec![]),
                    op("BT", vec![]),
                    op("Tf", vec![name(&names[0]), r(size)]),
                ];
                match kept_paint(t, PartFont::Span(0), color) {
                    Some(paint) => ops.extend(paint.ops()),
                    None => ops.push(rgb("rg", color)),
                }
                ops.push(at);
                ops.extend(pen.ops(0, size, 0.0, &names));
                ops.extend([op("ET", vec![]), op("Q", vec![])]);
                Plan::OnTop(ops)
            };
            plans.push(plan);
        }
        // The page with the lines' shows taken out, or the first changed in place. What
        // follows each line is kept where it was, the lines taken in the page's order, so
        // that the last line before a line move sets where it goes.
        let mut following: Vec<(usize, Following)> = (lines.iter())
            .map(|(o, _)| (o.first, Following::of(&parsed.ops, o)))
            .collect();
        following.sort_by_key(|(first, _)| *first);
        let fixes = Following::all(following.into_iter().map(|(_, f)| f));
        let mut shows: HashMap<usize, (Show, usize)> = HashMap::new();
        for (k, (_, t)) in lines.iter().enumerate() {
            shows.extend(t.runs.iter().map(|r| (r.op, (r.show, k))));
        }
        let mut out = Vec::with_capacity(parsed.ops.len() + 8 * edits.len());
        let mut checks = Vec::new();
        let mut places = vec![None; edits.len()];
        let mut watched = HashMap::new();
        for (i, o) in parsed.ops.iter().enumerate() {
            out.extend(fixes.get(&i).into_iter().flatten().cloned());
            match shows.get(&i) {
                Some(&(show, k)) => {
                    out.extend(move_only(show));
                    if let Plan::InPlace(first, line) = &plans[k] {
                        if *first == i {
                            if let Some(want) = check[k].take() {
                                checks.push((k, Check::InPlace(out.len()), want));
                            }
                            places[k] = Some(Check::InPlace(out.len()));
                            out.push(line.clone());
                        }
                    }
                }
                None => {
                    if watch.contains(&i) {
                        watched.insert(i, out.len());
                    }
                    out.push(o.clone())
                }
            }
        }
        if plans.iter().all(|p| matches!(p, Plan::InPlace(..))) {
            let (ops, depth) = (out, parsed.depth_left);
            return Ok(Edited {
                ops,
                depth,
                checks,
                places,
                watched,
            });
        }
        // Closing what the page leaves open may put a `q` before it all.
        let shift = usize::from(leaks_state(&out));
        let checked = (checks.iter_mut().map(|(_, at, _)| at)).chain(places.iter_mut().flatten());
        for at in checked {
            if let Check::InPlace(i) = at {
                *i += shift;
            }
        }
        for at in watched.values_mut() {
            *at += shift;
        }
        let mut ops = closed(out, parsed.depth_left);
        for (k, plan) in plans.into_iter().enumerate() {
            if let Plan::OnTop(line) = plan {
                let from = ops.len();
                ops.extend(line);
                if let Some(want) = check[k].take() {
                    checks.push((k, Check::OnTop(from, ops.len()), want));
                }
                if !lines[k].1.hidden {
                    places[k] = Some(Check::OnTop(from, ops.len()));
                }
            }
        }
        Ok(Edited {
            ops,
            depth: 0,
            checks,
            places,
            watched,
        })
    }

    /// Which of the lines with characters written in their own fonts (their index, where
    /// they are in `ops`, and the text they should read) don't read as the same words, as
    /// the page will be read with `ops` as its content. A line changed in place may now read
    /// as one with the text next to it.
    pub(crate) fn misread(
        &self,
        page: ObjectId,
        inline: &Inline,
        ops: &[Operation],
        checks: &[(usize, Check, String)],
    ) -> Result<Vec<usize>> {
        if checks.is_empty() {
            return Ok(Vec::new());
        }
        let bytes = (inline.encode(ops.to_vec()))
            .map_err(|e| anyhow!("cannot encode the page content: {e}"))?;
        let parsed = self.parse_as_page(page, &bytes)?;
        // A ligature character typed reads back as its letters, as the page is read; and
        // accents typed apart, which the writer puts together (NFC), are compared so.
        let words = |s: &str| {
            let s: String = s.chars().map(crate::fonts::unligature).collect::<String>();
            let s: String = s.nfc().collect();
            s.split_whitespace().map(str::to_string).collect::<Vec<_>>()
        };
        // The lines of text it reads as, joined as the parser joins a line's pieces: a line
        // written with a wide move in it (a font with no space of its own) reads as two a
        // word apart, a raised mark as one touching its word.
        let read = |keep: &dyn Fn(&Obj) -> bool| -> String {
            let mut out = String::new();
            let mut last: Option<&TextData> = None;
            let lines = parsed.objs.iter().filter(|o| keep(o));
            for t in lines.filter_map(|o| o.text.as_ref()) {
                if last.is_some_and(|l| apart(l, t)) {
                    out.push(' ');
                }
                out.push_str(&t.text);
                last = Some(t);
            }
            out
        };
        Ok(checks
            .iter()
            .filter(|(_, at, want)| {
                let want = words(want);
                match *at {
                    Check::OnTop(from, to) => {
                        words(&read(&|o| (from..to).contains(&o.first))) != want
                    }
                    Check::InPlace(i) => {
                        let got = words(&read(&|o| o.first <= i && i <= o.last));
                        want.is_empty() || !got.windows(want.len()).any(|w| w == want)
                    }
                    Check::Within(from, to) => {
                        let got = words(&read(&|o| o.first < to && from <= o.last));
                        !want.is_empty() && !got.windows(want.len()).any(|w| w == want)
                    }
                }
            })
            .map(|(k, ..)| *k)
            .collect())
    }

    /// Which of the lines written (`expected`) don't sit where they should, by more than
    /// `tol` points, as the page will be read with `ops` as its content: the first character
    /// starts elsewhere, the last ends elsewhere, the box runs past the edge it stays inside,
    /// a word before the edit moved, or the words after it didn't move as one (PB-47). Read
    /// where the reader puts each character, with its spacing, scaling and widths.
    pub(crate) fn misplaced(
        &self,
        page: ObjectId,
        inline: &Inline,
        ops: &[Operation],
        expected: &[Extent],
        tol: f32,
    ) -> Result<Vec<usize>> {
        if expected.is_empty() {
            return Ok(Vec::new());
        }
        let bytes = (inline.encode(ops.to_vec()))
            .map_err(|e| anyhow!("cannot encode the page content: {e}"))?;
        let parsed = self.parse_as_page(page, &bytes)?;
        let (vd, _, _) = self.vis_down(page);
        let to_vis = mx::inv(vd).ok_or_else(|| anyhow!("degenerate page transform"))?;
        Ok(expected
            .iter()
            .filter(|e| {
                let written = e.check.ops();
                let chars: Vec<(char, f32, f32)> = (parsed.objs.iter())
                    .flat_map(|o| placed(o, to_vis, &|op| written.contains(&op)))
                    .collect();
                off(e, &chars, tol)
            })
            .map(|e| e.n)
            .collect())
    }

    /// `ops`, the line [`line_ops`] writes from `parts` on page `page`, with each stretch
    /// of its parts that are [`Written::flipped`] drawn in text space turned over, so their
    /// glyphs stand up: a `Tm` turned over before a stretch's first show, where the line has
    /// got to there, and one the right way up before the next part's. Where the line has got
    /// to is read from the line as written, as the page will read it; turning text space
    /// over doesn't move it along the line.
    pub(crate) fn turned(
        &self,
        page: ObjectId,
        ops: Vec<Operation>,
        parts: &[Written],
    ) -> Result<Vec<Operation>> {
        if !parts.iter().any(|p| p.flipped) {
            return Ok(ops);
        }
        let bytes = (Content { operations: ops })
            .encode()
            .map_err(|e| anyhow!("cannot encode the edited line: {e}"))?;
        let parsed = self.parse_as_page(page, &bytes)?;
        let at: HashMap<usize, M> = (parsed.objs.iter().filter_map(|o| o.text.as_ref()))
            .flat_map(|t| t.runs.iter().map(|r| (r.op, r.tm)))
            .collect();
        // Each show's part, in order: `line_ops` writes every show as a `TJ`.
        let mut owner = (parts.iter()).flat_map(|p| std::iter::repeat_n(p.flipped, p.shows.len()));
        let mut out = Vec::with_capacity(parsed.ops.len() + 2 * parts.len());
        let mut turned = false;
        for (i, o) in parsed.ops.into_iter().enumerate() {
            if o.operator == "TJ" {
                let flipped = owner.next().unwrap_or(false);
                if let (true, Some(m)) = (flipped != turned, at.get(&i)) {
                    let m = match flipped {
                        true => [m[0], m[1], -m[2], -m[3], m[4], m[5]],
                        false => *m,
                    };
                    out.push(op("Tm", m.iter().map(|&x| r(x)).collect()));
                    turned = flipped;
                }
            }
            out.push(o);
        }
        Ok(out)
    }

    /// New text in `parts` as it is written, with the fonts it needs added to the page's
    /// resources ([`Pdf::written_lines`]).
    fn written(&mut self, page: ObjectId, parts: &[TextPart]) -> Result<Vec<Written>> {
        let mut lines = self.written_lines(page, &[(None, parts, false)])?;
        Ok(lines.remove(0).0)
    }

    /// Lines of parts as they are written ([`Asked`]), with the fonts they need added to the
    /// page's resources, one for each font chosen. The parts of an edited line are written
    /// in the font of the line's part they name where they can ([`PageObject::kept_pieces`]:
    /// a part as it was with its shows as they were, others character by character), the
    /// rest in the installed family that font is from or the nearest standard font; new
    /// text has none to name. Empty parts are left out. With each line, whether characters
    /// were written in its own fonts character by character, which reads back.
    fn written_lines(
        &mut self,
        page: ObjectId,
        lines: &[Asked],
    ) -> Result<Vec<(Vec<Written>, bool)>> {
        // A piece of a part keeps its font, written with these shows at this size in the
        // font's own terms (`Span::em`), turned over or not ([`Written::flipped`]), or is
        // written in a font added; with what goes at its end from the piece after it, and
        // the gap before it in points, which its first show opens with.
        enum How {
            Own(Vec<u8>, Shows, f32, bool),
            New(FontChoice),
        }
        type Piece = (TextPart, How, Tail, f32);
        // Each line's pieces, whether any was written character by character, and the line
        // edited.
        type Line<'t> = (Vec<Piece>, bool, Option<&'t TextData>);
        let mut hows: Vec<Line> = Vec::new();
        let font_dict = self.font_resources(page);
        let mut page_fonts = PageFonts::new(&self.doc, &font_dict);
        for &(line, parts, own) in lines {
            let mut line_hows: Vec<Piece> = Vec::new();
            let parts = parts.iter().filter(|p| !p.text.is_empty());
            let Some((parsed, t)) = line else {
                for p in parts {
                    let PartFont::Choice(c) = p.font else {
                        bail!("new text has no font of its own to keep");
                    };
                    line_hows.push((p.clone(), How::New(c), Tail::default(), 0.0));
                }
                hows.push((line_hows, false, None));
                continue;
            };
            let fonts = page_fonts.of(parsed, t);
            let drawn = fonts.first().and_then(LineFont::drawn);
            let chars: Vec<char> = t.text.chars().collect();
            let mut pen = OwnPen::new(t, drawn);
            let mut used = false;
            // The gap read as a space after a part kept as it was, written when what comes
            // after it is known: the show it follows, and the part.
            let mut open: Option<(usize, &Span)> = None;
            for p in parts {
                let text: Vec<char> = p.text.chars().collect();
                let whole = match p.font {
                    PartFont::Span(i) => (t.spans.get(i)).filter(|s| {
                        !t.vertical && chars.get(s.chars.0..s.chars.1) == Some(&text[..])
                    }),
                    PartFont::Choice(_) => None,
                };
                // A part kept as it was after it keeps the number it opens with: the move
                // is to where that starts (PB-47). Else the whole gap goes after the show
                // where it reads as a space there, else it is what this part opens with,
                // as the page had it (PB-30).
                let mut lead = 0.0;
                if let Some((k, was)) = open.take() {
                    let run = &t.runs[k];
                    let move_to = match whole {
                        Some(s) if s.runs.0 == k + 1 => to_next(t, k),
                        _ if reads_as_space(-run.gap * 1000.0 / show_units(run, was), was.em) => {
                            Some(run.gap)
                        }
                        _ => {
                            lead = run.gap;
                            None
                        }
                    };
                    if let (Some(d), Some((_, How::Own(_, shows, ..), ..))) =
                        (move_to, line_hows.last_mut())
                    {
                        then_gap(shows, gap_move(d, run, was));
                    }
                }
                let i = match p.font {
                    PartFont::Choice(c) => {
                        pen.apart();
                        line_hows.push((p.clone(), How::New(c), Tail::default(), lead));
                        continue;
                    }
                    PartFont::Span(i) => i,
                };
                let s = (t.spans.get(i))
                    .ok_or_else(|| anyhow!("the line has no part {i} (the page changed)"))?;
                let font = fonts.get(i);
                // Its size in the font's own terms, for what is kept in it.
                let own_size = p.size.max(1.0) / s.em;
                let flipped = font.is_some_and(LineFont::flipped);
                if whole.is_some() {
                    pen.apart();
                    let shows = shows_as_they_were(&parsed.ops, t, s);
                    let how = How::Own(s.font.clone(), shows, own_size, flipped);
                    line_hows.push((p.clone(), how, Tail::default(), lead));
                    let last = s.runs.1.checked_sub(1).filter(|&k| k >= s.runs.0);
                    open = last.filter(|&k| t.runs[k].gap > 0.0).map(|k| (k, s));
                    continue;
                }
                let other = FontChoice::of_font(&s.base);
                if !own {
                    pen.apart();
                    line_hows.push((p.clone(), How::New(other), Tail::default(), lead));
                    continue;
                }
                let at = |o: usize| drawn?.get(o);
                let ways = plan(&chars, s.chars, at, font, &text, |j| p.origin(j));
                for piece in pieces(&text, &ways) {
                    let (a, b) = piece.chars;
                    let sub = TextPart {
                        text: text[a..b].iter().collect(),
                        from: None,
                        origins: Vec::new(),
                        ..p.clone()
                    };
                    if !piece.kept {
                        pen.apart();
                        let lead = std::mem::take(&mut lead);
                        line_hows.push((sub, How::New(other), Tail::default(), lead));
                        continue;
                    }
                    let (tail, shows) = pen.write(s, font, &text, &ways, a..b, own_size);
                    used = true;
                    if let Some((_, _, before, _)) = line_hows.last_mut() {
                        before.items.extend(tail.items);
                        before.space = tail.space.or(before.space);
                    }
                    let how = How::Own(s.font.clone(), shows, own_size, flipped);
                    let lead = std::mem::take(&mut lead);
                    line_hows.push((sub, how, Tail::default(), lead));
                }
            }
            // The line ends after a part kept as it was: its gap after it, as before.
            if let (Some((k, was)), Some((_, How::Own(_, shows, ..), ..))) =
                (open, line_hows.last_mut())
            {
                let run = &t.runs[k];
                then_gap(shows, gap_move(run.gap, run, was));
            }
            hows.push((line_hows, used, Some(t)));
        }
        drop(page_fonts);
        // One pen for each font chosen, writing every piece set in it, its fonts (the one
        // chosen and those it falls back to) added to the page once.
        let mut texts: Vec<(FontChoice, Vec<&str>)> = Vec::new();
        for (p, how, ..) in hows.iter().flat_map(|(h, ..)| h) {
            if let How::New(c) = how {
                match texts.iter_mut().find(|(f, _)| f == c) {
                    Some((_, list)) => list.push(&p.text),
                    None => texts.push((*c, vec![&p.text])),
                }
            }
        }
        let mut pens = Vec::new();
        for (c, list) in &texts {
            let setter = self.setter(*c, &list.concat());
            let (names, pen) = self.text_pen(page, &setter, list)?;
            pens.push((*c, names, pen));
        }
        // How many pieces each pen has written so far.
        let mut done = vec![0; pens.len()];
        let mut write = |(p, how, tail, lead): Piece, t: Option<&TextData>| {
            let size = match how {
                How::Own(_, _, own, _) => own,
                How::New(_) => p.size.max(1.0),
            };
            let paint = t.and_then(|t| kept_paint(t, p.font, p.color));
            let mut out: Vec<Written> = match how {
                How::Own(font, shows, _, flipped) => vec![Written {
                    font,
                    size,
                    color: p.color,
                    paint,
                    rise: p.rise,
                    flipped,
                    shows,
                }],
                How::New(c) => {
                    let k = pens.iter().position(|(f, ..)| *f == c).expect("made");
                    let (_, names, pen) = &pens[k];
                    let shows = pen.shows(done[k]);
                    done[k] += 1;
                    (shows.iter())
                        .map(|s| Written {
                            font: names[s.font].as_bytes().to_vec(),
                            size,
                            color: p.color,
                            paint: paint.clone(),
                            rise: p.rise + s.rise * size / 1000.0,
                            flipped: false,
                            shows: vec![([0.0, 0.0, 1.0], tj(s.items.clone()))],
                        })
                        .collect()
                }
            };
            // The gap before it opens its first show, as the page had it (PB-30).
            if let Some((sp, show)) = out.first_mut().and_then(|w| w.shows.first_mut()) {
                if lead > 0.0 {
                    let here = (size * sp[2]).max(1e-3);
                    move_first(show, r(-lead * 1000.0 / here));
                }
            }
            let mut items = tail.items;
            if let Some(last) = out.last_mut() {
                let shows = &mut last.shows;
                if let Some((x, units)) = tail.space {
                    let th = shows.last().map_or(1.0, |(sp, _)| sp[2]);
                    let here = (size * th).max(1e-3);
                    items.push(r(if units == here { x } else { x * units / here }));
                }
                match shows.last_mut() {
                    _ if items.is_empty() => {}
                    Some((_, show)) => then_move(show, items),
                    None => shows.push(([0.0, 0.0, 1.0], tj(items))),
                }
            }
            out
        };
        // A piece with nothing to show (a space that went to the piece before) is left out.
        let shown = |w: &Written| !w.shows.is_empty();
        let mut lines = Vec::with_capacity(hows.len());
        for (h, used, t) in hows {
            let written: Vec<Written> = (h.into_iter())
                .flat_map(|piece| write(piece, t))
                .filter(shown)
                .collect();
            lines.push((written, used));
        }
        Ok(lines)
    }

    pub fn set_path_style(&mut self, page_index: usize, id: usize, st: &PathStyle) -> Result<()> {
        self.set_path_styles(page_index, &[(id, *st)])
    }

    /// Restyle several paths of one page at once, in one rewrite of its content: the ids are
    /// as the page is now. If any isn't a path, nothing is changed.
    pub fn set_path_styles(
        &mut self,
        page_index: usize,
        styles: &[(usize, PathStyle)],
    ) -> Result<()> {
        let (page, parsed) = self.load_edit(page_index)?;
        let mut styles = styles.to_vec();
        styles.sort_by_key(|&(id, _)| id);
        let (objs, owner) = Self::pick_all(&parsed, styles.iter().map(|&(id, _)| id))?;
        let paints = (objs.iter().zip(&styles))
            .map(|(obj, (id, st))| {
                let p = (obj.path.as_ref()).ok_or_else(|| anyhow!("object {id} is not a path"))?;
                Ok(match (st.fill.is_some(), st.stroke.is_some()) {
                    (true, true) => match (p.evenodd, p.closed) {
                        (false, false) => "B",
                        (true, false) => "B*",
                        (false, true) => "b",
                        (true, true) => "b*",
                    },
                    (true, false) => {
                        if p.evenodd {
                            "f*"
                        } else {
                            "f"
                        }
                    }
                    (false, true) => {
                        if p.closed {
                            "s"
                        } else {
                            "S"
                        }
                    }
                    (false, false) => "n",
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut out = Vec::with_capacity(parsed.ops.len() + 6 * objs.len());
        for (i, o) in parsed.ops.iter().enumerate() {
            let Some(k) = owner[i] else {
                out.push(o.clone());
                continue;
            };
            let (obj, st, paint) = (&objs[k], &styles[k].1, paints[k]);
            let scale = obj.ctm[0].hypot(obj.ctm[1]).max(1e-6);
            if i == obj.first {
                out.push(op("q", vec![]));
                if let Some(f) = st.fill {
                    out.push(rgb("rg", f));
                }
                if let Some(s) = st.stroke {
                    out.push(rgb("RG", s));
                    out.push(op("w", vec![r((st.line_width / scale).max(0.001))]));
                }
            }
            if i == obj.last {
                out.push(op(paint, vec![]));
                out.push(op("Q", vec![]));
            } else {
                out.push(o.clone());
            }
        }
        self.commit(page, out, parsed.depth_left, &parsed.inline)
    }

    /// Replace the picture of an image object with a PNG/JPEG file; the new picture is fitted
    /// inside the old box, upright as the file's orientation asks.
    pub fn replace_image(&mut self, page_index: usize, id: usize, bytes: &[u8]) -> Result<()> {
        let (page, parsed) = self.load_edit(page_index)?;
        let obj = Self::pick(&parsed, id)?.clone();
        if obj.kind != ObjectKind::Image || obj.inline {
            bail!("only image objects stored as XObjects can be replaced");
        }
        let (img, w, h, turn) = self.add_upright_image(bytes)?;
        let nm = self.add_resource(page, "XObject", "LimareIm", Object::Reference(img))?;
        let (bw, bh) = (
            obj.ctm[0].hypot(obj.ctm[1]).max(1e-3),
            obj.ctm[2].hypot(obj.ctm[3]).max(1e-3),
        );
        let a = w / h;
        let fit = if a > bw / bh {
            let sy = (bw / a) / bh;
            [1.0, 0.0, 0.0, sy, 0.0, (1.0 - sy) / 2.0]
        } else {
            let sx = (bh * a) / bw;
            [sx, 0.0, 0.0, 1.0, (1.0 - sx) / 2.0, 0.0]
        };
        let mut out = Vec::with_capacity(parsed.ops.len() + 3);
        for (i, o) in parsed.ops.iter().enumerate() {
            if i == obj.first {
                out.extend([op("q", vec![]), cm(fit)]);
                out.extend(upright(turn));
                out.extend([op("Do", vec![name(&nm)]), op("Q", vec![])]);
            } else {
                out.push(o.clone());
            }
        }
        self.commit(page, out, parsed.depth_left, &parsed.inline)
    }

    /// Run `body` against the page content with a new `q <displayed-page → user space> cm` layer appended.
    fn append_layer(
        &mut self,
        page_index: usize,
        body: impl FnOnce(&mut Self, ObjectId) -> Result<Vec<Operation>>,
    ) -> Result<()> {
        let (page, parsed) = self.load_edit(page_index)?;
        let (vd, _, _) = self.vis_down(page);
        let mut out = closed(parsed.ops.clone(), parsed.depth_left);
        out.extend([op("q", vec![]), cm(vd)]);
        out.extend(body(self, page)?);
        out.push(op("Q", vec![]));
        self.commit(page, out, 0, &parsed.inline)
    }

    /// Add a line of text. `pos` is the left end of the baseline in displayed-page points (y down).
    pub fn add_text(
        &mut self,
        page_index: usize,
        pos: (f32, f32),
        text: &str,
        font: FontChoice,
        size: f32,
        color: [f32; 3],
    ) -> Result<()> {
        let part = TextPart {
            text: text.to_string(),
            font: PartFont::Choice(font),
            size,
            color,
            rise: 0.0,
            from: None,
            origins: Vec::new(),
        };
        self.add_text_parts(page_index, pos, &[part])
    }

    /// Add a line of text in parts of different styles, at `pos` as [`Pdf::add_text`] adds
    /// one. Each part names its font with [`PartFont::Choice`].
    pub fn add_text_parts(
        &mut self,
        page_index: usize,
        pos: (f32, f32),
        parts: &[TextPart],
    ) -> Result<()> {
        self.add_text_decorated(page_index, pos, parts, &[])
    }

    /// [`Pdf::add_text_parts`] with underlines and strike-throughs over its text (the parts'
    /// texts one after the other), each at its font's place unless given a look.
    pub fn add_text_decorated(
        &mut self,
        page_index: usize,
        pos: (f32, f32),
        parts: &[TextPart],
        decorations: &[Decorated],
    ) -> Result<()> {
        if parts.iter().all(|p| p.text.is_empty()) {
            bail!("empty text");
        }
        let parts = parts.to_vec();
        let decorations = decorations.to_vec();
        self.append_layer(page_index, move |s, page| {
            let written = s.written(page, &parts)?;
            let mut ops = line_ops([1.0, 0.0, 0.0, -1.0, pos.0, pos.1], &written);
            if !decorations.is_empty() {
                // Where the line went, read as the page will read it, in the layer's terms.
                let bytes = (Content {
                    operations: ops.clone(),
                })
                .encode()
                .map_err(|e| anyhow!("cannot encode the new text: {e}"))?;
                let back = s.parse_as_page(page, &bytes)?;
                let text: String = parts.iter().map(|p| p.text.as_str()).collect();
                let place = Check::OnTop(0, ops.len());
                let (more, _) =
                    s.decoration_ops(page, &back, place, &Redraw::new(&text, &decorations));
                ops.extend(more);
            }
            Ok(ops)
        })
    }

    /// New text of several lines (each ended by Enter), each line written as
    /// [`Pdf::add_text_decorated`] writes one: its baseline `lines[i].0` points below the
    /// first's, whose left end is `pos`, and aligned among the others as `align` says, each
    /// line measured as it is written. The widest starts at `pos`; the others start there too
    /// (left), are centred under it, or end where it ends (right). Justified is written left,
    /// as each line ends where Enter ended it.
    pub fn add_text_lines(
        &mut self,
        page_index: usize,
        pos: (f32, f32),
        lines: &[(f32, Vec<TextPart>, Vec<Decorated>)],
        align: crate::Align,
    ) -> Result<()> {
        // Left (and justified) lines all start at `pos`: nothing to measure.
        let share = align.share();
        let widths = if share == 0.0 {
            vec![0.0; lines.len()]
        } else {
            (lines.iter())
                .map(|(_, parts, _)| self.new_text_width(parts))
                .collect::<Result<Vec<f32>>>()?
        };
        let widest = widths.iter().copied().fold(0.0, f32::max);
        for ((dy, parts, decos), w) in lines.iter().zip(widths) {
            let x = pos.0 + (widest - w) * share;
            self.add_text_decorated(page_index, (x, pos.1 + dy), parts, decos)?;
        }
        Ok(())
    }

    /// How wide a line of new text in `parts` is as it is written, in points: each part set
    /// as [`Pdf::written_lines`] sets new text, the parts in one font by one setter.
    fn new_text_width(&self, parts: &[TextPart]) -> Result<f32> {
        let parts: Vec<&TextPart> = parts.iter().filter(|p| !p.text.is_empty()).collect();
        let mut texts: Vec<(FontChoice, String)> = Vec::new();
        for p in &parts {
            let PartFont::Choice(c) = p.font else {
                bail!("new text has no font of its own to keep");
            };
            match texts.iter_mut().find(|(f, _)| *f == c) {
                Some((_, all)) => all.push_str(&p.text),
                None => texts.push((c, p.text.clone())),
            }
        }
        let setters: Vec<(FontChoice, Setter)> = (texts.iter())
            .map(|(c, all)| (*c, self.setter(*c, all)))
            .collect();
        let width = |p: &TextPart| {
            let PartFont::Choice(c) = p.font else {
                return 0.0;
            };
            let set = setters.iter().find(|(f, _)| *f == c).map(|(_, s)| s);
            set.map_or(0.0, |s| s.width(&p.text, p.size.max(1.0)))
        };
        Ok(parts.iter().map(|p| width(p)).sum())
    }

    /// Add a picture filling `rect` (displayed-page points, y down), upright as the file's
    /// orientation asks.
    pub fn add_image(&mut self, page_index: usize, rect: [f32; 4], bytes: &[u8]) -> Result<()> {
        let (img, _, _, o) = self.add_upright_image(bytes)?;
        self.append_layer(page_index, move |s, page| {
            let nm = s.add_resource(page, "XObject", "LimareIm", Object::Reference(img))?;
            let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
            let mut ops = vec![op("q", vec![]), cm([w, 0.0, 0.0, -h, rect[0], rect[3]])];
            ops.extend(upright(o));
            ops.extend([op("Do", vec![name(&nm)]), op("Q", vec![])]);
            Ok(ops)
        })
    }

    /// Add a rectangle, ellipse or line inside `rect` (displayed-page points, y down).
    pub fn add_shape(
        &mut self,
        page_index: usize,
        shape: Shape,
        rect: [f32; 4],
        st: &PathStyle,
    ) -> Result<()> {
        let st = *st;
        self.append_layer(page_index, move |_, _| {
            let [x0, y0, x1, y1] = rect;
            let mut v = vec![];
            if let Some(f) = st.fill {
                v.push(rgb("rg", f));
            }
            if let Some(s) = st.stroke {
                v.push(rgb("RG", s));
                v.push(op("w", vec![r(st.line_width.max(0.1))]));
            }
            match shape {
                Shape::Rect => v.push(op(
                    "re",
                    vec![
                        r(x0.min(x1)),
                        r(y0.min(y1)),
                        r((x1 - x0).abs()),
                        r((y1 - y0).abs()),
                    ],
                )),
                Shape::Line => {
                    v.push(op("m", vec![r(x0), r(y0)]));
                    v.push(op("l", vec![r(x1), r(y1)]));
                }
                Shape::Ellipse => {
                    let (cx, cy, rx, ry) = (
                        (x0 + x1) / 2.0,
                        (y0 + y1) / 2.0,
                        (x1 - x0).abs() / 2.0,
                        (y1 - y0).abs() / 2.0,
                    );
                    let k = 0.552_284_7;
                    v.push(op("m", vec![r(cx + rx), r(cy)]));
                    let pts = [
                        [
                            (cx + rx, cy + k * ry),
                            (cx + k * rx, cy + ry),
                            (cx, cy + ry),
                        ],
                        [
                            (cx - k * rx, cy + ry),
                            (cx - rx, cy + k * ry),
                            (cx - rx, cy),
                        ],
                        [
                            (cx - rx, cy - k * ry),
                            (cx - k * rx, cy - ry),
                            (cx, cy - ry),
                        ],
                        [
                            (cx + k * rx, cy - ry),
                            (cx + rx, cy - k * ry),
                            (cx + rx, cy),
                        ],
                    ];
                    for c in pts {
                        v.push(op("c", c.iter().flat_map(|&(x, y)| [r(x), r(y)]).collect()));
                    }
                    v.push(op("h", vec![]));
                }
            }
            let paint = match (shape, st.fill.is_some(), st.stroke.is_some()) {
                (Shape::Line, _, _) => "S",
                (_, true, true) => "B",
                (_, true, false) => "f",
                (_, false, true) => "S",
                (_, false, false) => "n",
            };
            v.push(op(paint, vec![]));
            Ok(v)
        })
    }
}

/// Whether line of text `b`, read after line `a`, is a word apart from it: it starts more
/// than 0.18 of the smaller size along the line past `a`'s end (the parser's word gap), or
/// on another line. A raised mark right after its word isn't.
fn apart(a: &TextData, b: &TextData) -> bool {
    let (Some(end), Some(start)) = (a.pieces.last(), b.pieces.first()) else {
        return true;
    };
    // Where along a piece's box (which starts before a show's opening move) a character
    // starts or ends, by its share of the box.
    let at = |p: &crate::objects::Piece, f: f32| {
        let (c0, c1) = (p.corners[0], p.corners[1]);
        (c0.0 + (c1.0 - c0.0) * f, c0.1 + (c1.1 - c0.1) * f)
    };
    let e = at(end, end.spans.last().map_or(1.0, |s| s.1));
    let s = at(start, start.spans.first().map_or(0.0, |s| s.0));
    let (dx, dy) = (s.0 - e.0, s.1 - e.1);
    let along = dx * a.dir.0 + dy * a.dir.1;
    let across = (-dx * a.dir.1 + dy * a.dir.0).abs();
    along > 0.18 * a.size.min(b.size) || across > 0.7 * a.size.max(b.size)
}

/// The characters line `o` shows by the operators `keep` takes, each with where it starts
/// and ends along the line on the displayed page (`to_vis` takes page space there), from
/// the page's corner: across it, for a line upright on it.
fn placed(o: &Obj, to_vis: M, keep: &dyn Fn(usize) -> bool) -> Vec<(char, f32, f32)> {
    let Some(t) = &o.text else {
        return Vec::new();
    };
    let m = mul(mul(t.runs[0].tm, o.ctm), to_vis);
    let len = m[0].hypot(m[1]);
    if len < 1e-6 {
        return Vec::new();
    }
    let dir = (m[0] / len, m[1] / len);
    let chars: Vec<char> = t.text.chars().collect();
    let mut out = Vec::new();
    for (_, p) in t.runs.iter().zip(&t.pieces).filter(|(r, _)| keep(r.op)) {
        // Where along a piece's box a character starts or ends, by its share of the box.
        let along = |f: f32| {
            let (c0, c1) = (p.corners[0], p.corners[1]);
            let (x, y) = mx::apply(to_vis, c0.0 + (c1.0 - c0.0) * f, c0.1 + (c1.1 - c0.1) * f);
            x * dir.0 + y * dir.1
        };
        let n = p.chars.1 - p.chars.0;
        for j in 0..n {
            let (f0, f1) = match p.spans.len() == n {
                true => p.spans[j],
                false => (j as f32 / n as f32, (j + 1) as f32 / n as f32),
            };
            out.push((chars[p.chars.0 + j], along(f0), along(f1)));
        }
    }
    out
}

/// Where the lines of a line edit (`edits`, their lines as read in `lines`), written where
/// `places` says, should sit ([`Pdf::misplaced`]; Q3 of S1-14): the words that end before
/// the first character changed where they were, and those that start after the last one
/// all moved by one amount. A character is kept as it was drawn when it is the line's own,
/// in the part of the line it was in, at its size and rise; in a line written in other
/// fonts because it didn't read as typed (`held` false), only in a part kept whole. A line
/// written again because its words moved is held as its first try was. Invisible lines,
/// upright ones and lines restyled as one aren't looked at.
fn kept_extents(
    lines: &[&Obj],
    edits: &[(usize, TextEdit)],
    places: &[Option<Check>],
    held: &[bool],
    to_vis: M,
) -> Vec<Extent> {
    let mut out = Vec::new();
    for (k, ((_, e), o)) in edits.iter().zip(lines).enumerate() {
        let (Some(check), Some(t)) = (places[k], &o.text) else {
            continue;
        };
        if t.hidden || t.vertical {
            continue;
        }
        let first = t.spans.first().map(|s| (s.size, s.fill, s.rise));
        let Some(parts) = e.parts_for(&t.text, first) else {
            continue;
        };
        let was: Vec<char> = t.text.chars().collect();
        let drawn = placed(o, to_vis, &|_| true);
        if !drawn.iter().map(|c| c.0).eq(was.iter().copied()) {
            continue;
        }
        // Where each character written was in the line, when it is kept as it was drawn.
        let mut kept: Vec<Option<usize>> = Vec::new();
        for p in parts.iter() {
            let text: Vec<char> = p.text.chars().collect();
            let span = match p.font {
                PartFont::Span(i) => t.spans.get(i),
                PartFont::Choice(_) => None,
            };
            let span = span.filter(|s| {
                (s.size - p.size).abs() <= 0.01 * s.size.max(0.01) && (s.rise - p.rise).abs() < 0.01
            });
            let whole = span.is_some_and(|s| was.get(s.chars.0..s.chars.1) == Some(&text[..]));
            let span = span.filter(|_| held[k] || whole);
            kept.extend((0..text.len()).map(|j| {
                let s = span?;
                p.origin(j).filter(|o| (s.chars.0..s.chars.1).contains(o))
            }));
        }
        // The characters kept as they were at the line's start and end.
        let most = kept.len().min(was.len());
        let head = (0..most).take_while(|&j| kept[j] == Some(j)).count();
        let tail = (0..most - head)
            .take_while(|&m| kept[kept.len() - 1 - m] == Some(was.len() - 1 - m))
            .count();
        // A word starts where its first character does, and is looked at when all of it is
        // kept, with the character before it: the word the change is in may move (Q3).
        let end = |i: usize| {
            (i..was.len())
                .find(|&j| was[j].is_whitespace())
                .unwrap_or(was.len())
        };
        let starts = |from: usize, to: usize| -> Vec<f32> {
            (from..to)
                .filter(|&i| !was[i].is_whitespace() && (i == 0 || was[i - 1].is_whitespace()))
                .filter(|&i| end(i) <= to)
                .map(|i| drawn[i].1)
                .collect()
        };
        let before = starts(0, head);
        let after = starts((was.len() - tail + 1).min(was.len()), was.len());
        if before.is_empty() && after.len() < 2 {
            continue;
        }
        out.push(Extent {
            n: k,
            check,
            x: None,
            end: None,
            right: None,
            before,
            after,
        });
    }
    out
}

/// Whether a line written, its characters where `chars` says ([`placed`]), sits more than
/// `tol` from where `e` says it should ([`Pdf::misplaced`]). A line with no ink sits
/// anywhere.
fn off(e: &Extent, chars: &[(char, f32, f32)], tol: f32) -> bool {
    // Where its ink starts and ends: an accent drawn back over its letter is inside them.
    let ink: Vec<&(char, f32, f32)> = chars.iter().filter(|c| !c.0.is_whitespace()).collect();
    if ink.is_empty() {
        return false;
    }
    let start = ink.iter().map(|c| c.1.min(c.2)).fold(f32::MAX, f32::min);
    let reach = ink.iter().map(|c| c.1.max(c.2)).fold(f32::MIN, f32::max);
    if e.x.is_some_and(|x| (start - x).abs() > tol)
        || e.end.is_some_and(|end| (reach - end).abs() > tol)
        || e.right.is_some_and(|right| reach > right + tol)
    {
        return true;
    }
    // Where each word starts.
    let starts: Vec<f32> = (chars.iter().enumerate())
        .filter(|&(i, c)| !c.0.is_whitespace() && (i == 0 || chars[i - 1].0.is_whitespace()))
        .map(|(_, c)| c.1)
        .collect();
    if starts.len() < e.before.len() + e.after.len() {
        return true;
    }
    if (e.before.iter().zip(&starts)).any(|(want, got)| (got - want).abs() > tol) {
        return true;
    }
    let moved: Vec<f32> = (e.after.iter().rev().zip(starts.iter().rev()))
        .map(|(was, got)| got - was)
        .collect();
    let (least, most) =
        (moved.iter()).fold((f32::MAX, f32::MIN), |(a, b), &d| (a.min(d), b.max(d)));
    !moved.is_empty() && most - least > tol
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    /// A line written with shows cut out of the old one is found out when it reads as other
    /// words than it should, and only then: so it can be written again without them.
    #[test]
    fn a_line_that_reads_as_other_words_is_found_out() {
        let mut p = Pdf::new();
        p.insert_blank(0, 300.0, 200.0).unwrap();
        let font = p.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let page = p.page_ids()[0];
        let res = dictionary! { "Font" => dictionary! { "F1" => font } };
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("Resources", res);
        let ops = vec![
            op("BT", vec![]),
            op("Tf", vec![name("F1"), r(20.0)]),
            op("Tj", vec![lit(b"one".to_vec())]),
            op(
                "TJ",
                vec![Object::Array(vec![r(-100.0), lit(b"two".to_vec())])],
            ),
            op("ET", vec![]),
        ];
        let (_, inline) = crate::inline::extract(b"").unwrap();
        let want = |text: &str| vec![(3, Check::OnTop(0, ops.len()), text.to_string())];
        let misread = |text: &str| p.misread(page, &inline, &ops, &want(text)).unwrap();
        assert_eq!(misread("one two"), [3]);
        assert_eq!(misread("onetwo"), Vec::<usize>::new());
        assert_eq!(misread(" onetwo "), Vec::<usize>::new());
    }

    /// A line written a point from where it was set is found out, one written 0.3 points
    /// from it isn't (S1-14): read where the reader puts its characters.
    #[test]
    fn a_line_written_a_point_from_where_it_was_set_is_found_out() {
        let mut p = Pdf::new();
        p.insert_blank(0, 300.0, 200.0).unwrap();
        let font = p.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let page = p.page_ids()[0];
        let res = dictionary! { "Font" => dictionary! { "F1" => font } };
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("Resources", res);
        // Each line set at 20 across the page: "one" written at 21, "two" at 20.3.
        let line = |x: f32, y: f32, text: &[u8]| {
            vec![
                op("BT", vec![]),
                op("Tf", vec![name("F1"), r(20.0)]),
                op("Tm", vec![r(1.0), r(0.0), r(0.0), r(1.0), r(x), r(y)]),
                op("Tj", vec![lit(text.to_vec())]),
                op("ET", vec![]),
            ]
        };
        let mut ops = line(21.0, 150.0, b"one");
        ops.extend(line(20.3, 100.0, b"two"));
        let (_, inline) = crate::inline::extract(b"").unwrap();
        // Helvetica's "one" is 1,668 thousandths of an em wide, its "two" 1,556.
        let set = |n: usize, check: Check, wide: f32| Extent {
            n,
            check,
            x: Some(20.0),
            end: Some(20.0 + wide * 0.02),
            right: Some(100.0),
            before: Vec::new(),
            after: Vec::new(),
        };
        let want = [
            set(0, Check::OnTop(0, 5), 1668.0),
            set(1, Check::OnTop(5, 10), 1556.0),
        ];
        let off = p.misplaced(page, &inline, &ops, &want, 0.5).unwrap();
        assert_eq!(off, [0]);
        // Its box past the frame's edge: found out too.
        let narrow = Extent {
            right: Some(40.0),
            ..want[1].clone()
        };
        let off = p.misplaced(page, &inline, &ops, &[narrow], 0.5).unwrap();
        assert_eq!(off, [1]);
    }

    /// A line edit's words before the change are found out when one moves, and those after it
    /// when they don't all move together (S1-14, Q3).
    #[test]
    fn a_line_edits_words_moved_apart_are_found_out() {
        let mut p = Pdf::new();
        p.insert_blank(0, 300.0, 200.0).unwrap();
        let font = p.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let page = p.page_ids()[0];
        let res = dictionary! { "Font" => dictionary! { "F1" => font } };
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("Resources", res);
        // "aa bb cc" at 20 across the page, its words starting at 20, 47.8 and 75.6
        // (Helvetica's "aa " is 1,390 thousandths of an em); the second line's "cc" a point on.
        let line = |y: f32, show: Operation| {
            vec![
                op("BT", vec![]),
                op("Tf", vec![name("F1"), r(20.0)]),
                op("Tm", vec![r(1.0), r(0.0), r(0.0), r(1.0), r(20.0), r(y)]),
                show,
                op("ET", vec![]),
            ]
        };
        let mut ops = line(150.0, op("Tj", vec![lit(b"aa bb cc".to_vec())]));
        let apart = vec![lit(b"aa bb ".to_vec()), r(-50.0), lit(b"cc".to_vec())];
        ops.extend(line(100.0, op("TJ", vec![Object::Array(apart)])));
        let (_, inline) = crate::inline::extract(b"").unwrap();
        let words = |n: usize, before: &[f32], after: &[f32]| Extent {
            n,
            check: Check::OnTop(5 * n, 5 * n + 5),
            x: None,
            end: None,
            right: None,
            before: before.to_vec(),
            after: after.to_vec(),
        };
        let off = |want: &[Extent]| p.misplaced(page, &inline, &ops, want, 0.5).unwrap();
        // The words after the change all 3 points on from where they were: the reflow.
        assert_eq!(
            off(&[words(0, &[20.0], &[44.8, 72.6])]),
            Vec::<usize>::new()
        );
        // The first word a point on.
        assert_eq!(off(&[words(0, &[19.0], &[])]), [0]);
        // "bb" where it was, "cc" a point on.
        assert_eq!(off(&[words(1, &[20.0], &[47.8, 75.6])]), [1]);
    }

    /// A line written with a word's space as a move wide enough to read as two lines of text
    /// (a font with no space of its own) reads as its words, the two a word apart: so it
    /// isn't written again in another font for nothing.
    #[test]
    fn a_line_read_back_in_two_pieces_reads_as_their_words() {
        let mut p = Pdf::new();
        p.insert_blank(0, 300.0, 200.0).unwrap();
        let font = p.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let page = p.page_ids()[0];
        let res = dictionary! { "Font" => dictionary! { "F1" => font } };
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("Resources", res);
        // "two" a whole em after "one": two lines of text as read.
        let ops = vec![
            op("BT", vec![]),
            op("Tf", vec![name("F1"), r(20.0)]),
            op("Tj", vec![lit(b"one".to_vec())]),
            op(
                "TJ",
                vec![Object::Array(vec![r(-1000.0), lit(b"two".to_vec())])],
            ),
            op("ET", vec![]),
        ];
        let (_, inline) = crate::inline::extract(b"").unwrap();
        let parsed = p.parse_as_page(page, b"BT /F1 20 Tf (one) Tj [-1000 (two)] TJ ET");
        assert_eq!(parsed.unwrap().text_lines().count(), 2);
        for at in [Check::OnTop(0, 5), Check::Within(0, 5)] {
            let check = |text: &str| vec![(0, at, text.to_string())];
            let misread = |text: &str| p.misread(page, &inline, &ops, &check(text)).unwrap();
            assert_eq!(misread("one two"), Vec::<usize>::new(), "{at:?}");
        }
        // A footnote mark raised right after its word reads as a line of its own, touching
        // it: "one2" (the corpus edit run's pdfbox `2296`, "U.S.1", taken as misread when
        // every two lines read were a word apart).
        let ops = vec![
            op("BT", vec![]),
            op("Tf", vec![name("F1"), r(20.0)]),
            op("Tj", vec![lit(b"one".to_vec())]),
            op("Ts", vec![r(8.0)]),
            op("Tf", vec![name("F1"), r(10.0)]),
            op("Tj", vec![lit(b"2".to_vec())]),
            op("ET", vec![]),
        ];
        let parsed = p.parse_as_page(page, b"BT /F1 20 Tf (one) Tj 8 Ts /F1 10 Tf (2) Tj ET");
        assert_eq!(parsed.unwrap().text_lines().count(), 2);
        let check = |text: &str| vec![(0, Check::Within(0, 7), text.to_string())];
        let misread = |text: &str| p.misread(page, &inline, &ops, &check(text)).unwrap();
        assert_eq!(misread("one2"), Vec::<usize>::new());
        assert_eq!(misread("one 2"), [0]);
    }
}
