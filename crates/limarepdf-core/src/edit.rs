//! Editing page content: move/resize, delete, change text, restyle paths, replace images, insert objects.

use crate::embed::Pen;
use crate::fonts::{FontChoice, FontCtx};
use crate::imgs::orient_matrix;
use crate::inline::Inline;
use crate::matrix::{self as mx, mul, M};
use crate::objects::{Obj, ObjectKind, Parsed, Run, Show, Span, TextData};
use crate::{PageObject, Pdf, Result};
use anyhow::{anyhow, bail};
use image::metadata::Orientation;
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId, Stream, StringFormat};
use std::collections::HashMap;

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
    /// they are then written with their own shows, cut out of the line's where that can be
    /// done ([`Pdf::edit_texts`]). `None` for anything else.
    pub from: Option<usize>,
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
    /// Whether [`Pdf::edit_text`] keeps the line's own font for this change: the font can
    /// be reused ([`crate::PageObject::font_reusable`]), the new text is plain ASCII and no
    /// style changes. Otherwise the line is written anew. For a line in one style changed
    /// in one style; [`crate::PageObject::keeps_font`] tells for each of `parts`.
    pub fn keeps_font(&self, reusable: bool) -> bool {
        let keep_style = self.font.is_none() && self.size.is_none() && self.color.is_none();
        reusable && ascii(&self.text) && keep_style
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

    /// This edit in one style, written as before parts were: an edit given in one style, or
    /// one in parts that only changes the words of a line in one style, in its font when
    /// that can be kept, which is changed in place. `None` when it is written in parts.
    fn one_style(&self, t: &TextData) -> Option<TextEdit> {
        if self.parts.is_empty() {
            return Some(self.clone());
        }
        let [s] = &t.spans[..] else {
            return None;
        };
        let unchanged = |q: &TextPart| {
            q.font == PartFont::Span(0)
                && q.size == s.size
                && q.color == s.fill.unwrap_or([0.0; 3])
                && q.rise == s.rise
        };
        let words = TextEdit {
            text: self.text.clone(),
            font: None,
            size: None,
            color: None,
            parts: Vec::new(),
        };
        (self.parts.iter().all(unchanged) && words.keeps_font(s.reuse)).then_some(words)
    }
}

impl PageObject {
    /// Whether [`Pdf::edit_text`] writes `part` of this line in the font of the line's
    /// part it names ([`PartFont::Span`]): its text is that part's own, unchanged (its
    /// shows are kept as they were), or some of that part's own characters, unchanged and
    /// ending at spaces ([`TextPart::from`]; their shows are cut out of the part's, which
    /// can fail on characters the page's font codes together), or the font can be reused
    /// and the text is plain ASCII. Otherwise it is written in the installed family the
    /// font is from or the nearest standard font.
    pub fn keeps_font(&self, part: &TextPart) -> bool {
        let PartFont::Span(i) = part.font else {
            return false;
        };
        let (Some(s), Some(text)) = (self.spans.get(i), &self.text) else {
            return false;
        };
        let line: Vec<char> = text.chars().collect();
        let (vertical, from) = (self.vertical, part.from);
        kept(&line, s.chars, s.font_reusable, vertical, &part.text, from).is_some()
    }
}

fn ascii(text: &str) -> bool {
    text.chars().all(|c| (' '..='~').contains(&c))
}

/// How a part of a line keeps the font of the line's part it is from.
#[derive(Clone, Copy, PartialEq)]
enum Kept {
    /// Its text is that part's own: written with its shows as they were.
    AsItWas,
    /// Its text is the line's characters from and to, unchanged, within that part:
    /// written with their shows cut out of the part's ([`shows_cut`]).
    Cut(usize, usize),
    /// Plain ASCII in a font that can be reused.
    Ascii,
}

/// Whether text `text` keeps the font of the part of `line` at `chars`, whose font is
/// `reusable`, and how; `from` is where the text starts in the line when it is the line's
/// own characters there ([`TextPart::from`]). Not in vertical writing, which the line is
/// written across.
fn kept(
    line: &[char],
    chars: (usize, usize),
    reusable: bool,
    vertical: bool,
    text: &str,
    from: Option<usize>,
) -> Option<Kept> {
    let own = line
        .get(chars.0..chars.1)
        .is_some_and(|c| c.iter().copied().eq(text.chars()));
    let cut = from.filter(|_| !vertical).and_then(|a| {
        let b = a + text.chars().count();
        let inside = chars.0 <= a && a < b && b <= chars.1;
        let same = inside && line[a..b].iter().copied().eq(text.chars());
        let at = |i: usize| cuttable(line, chars, i);
        (same && at(a) && at(b)).then_some(Kept::Cut(a, b))
    });
    if own && !vertical {
        Some(Kept::AsItWas)
    } else if cut.is_some() {
        cut
    } else if reusable && ascii(text) {
        Some(Kept::Ascii)
    } else {
        None
    }
}

/// Whether the part of `line` at `chars` may be cut at character `at`: at either end of
/// the part, or next to a space, so that no word is parted.
fn cuttable(line: &[char], chars: (usize, usize), at: usize) -> bool {
    let space = |i: Option<usize>| {
        i.and_then(|i| line.get(i))
            .is_some_and(|c| c.is_whitespace())
    };
    at == chars.0 || at == chars.1 || space(at.checked_sub(1)) || space(Some(at))
}

/// A line of parts to write ([`Pdf::written_lines`]): the line edited (the page's operators
/// and the line) or none for new text, its parts, and whether they may cut their shows out
/// of the line's.
type Asked<'a> = (
    Option<(&'a [Operation], &'a TextData)>,
    &'a [TextPart],
    bool,
);

/// A page's operators with text edits made ([`Pdf::edit_texts`]).
struct Edited {
    ops: Vec<Operation>,
    /// The `q`s left open, to close.
    depth: usize,
    /// The edits written with shows cut out of their lines: their index, their operators
    /// from and to, and the text they should read.
    cuts: Vec<(usize, (usize, usize), String)>,
    /// The ids of the lines where shows asked for couldn't be cut out.
    lost: Vec<usize>,
}

/// A part of a line as it is written by [`line_ops`]: the font's name in the page's
/// resources, its size, colour and rise, and its shows, each with the character spacing,
/// word spacing and horizontal scaling (1 is none) it is drawn with.
struct Written {
    font: Vec<u8>,
    size: f32,
    color: [f32; 3],
    rise: f32,
    shows: Vec<([f32; 3], Operation)>,
}

/// A line of text in parts, from `BT` to `ET`, starting at text matrix `tm`: each part's
/// font, colour, rise and spacing set where they change, then its shows.
fn line_ops(tm: M, parts: &[Written]) -> Vec<Operation> {
    let mut v = vec![op("BT", vec![])];
    let mut font: Option<(&[u8], f32)> = None;
    let mut color = None;
    let (mut rise, mut spacing) = (0.0, [0.0, 0.0, 1.0]);
    for (k, p) in parts.iter().enumerate() {
        if font != Some((&p.font, p.size)) {
            v.push(op("Tf", vec![Object::Name(p.font.clone()), r(p.size)]));
            font = Some((&p.font, p.size));
        }
        if color != Some(p.color) {
            v.push(rgb("rg", p.color));
            color = Some(p.color);
        }
        if p.rise != rise {
            v.push(op("Ts", vec![r(p.rise)]));
            rise = p.rise;
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
    v.push(op("ET", vec![]));
    v
}

/// The shows of part `s` of line `t` as they were, from the page's operators `ops`, each
/// with its spacing. A gap read as a space after a show is put back as the same gap.
fn shows_as_they_were(ops: &[Operation], t: &TextData, s: &Span) -> Shows {
    t.runs[s.runs.0..s.runs.1]
        .iter()
        .map(|run| {
            let mut items = show_items(&ops[run.op], run.show);
            items.extend(gap_move(run, s));
            (run.spacing, op("TJ", vec![Object::Array(items)]))
        })
        .collect()
}

/// Shows of a part of a line, each with the character spacing, word spacing and horizontal
/// scaling (1 is none) it is drawn with ([`Written`]).
type Shows = Vec<([f32; 3], Operation)>;

/// The strings and spacing numbers a show draws, as a `TJ` array holds them.
fn show_items(o: &Operation, show: Show) -> Vec<Object> {
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

/// The gap read as a space after `run` of part `s`, as a `TJ` move: in thousandths of the
/// text size.
fn gap_move(run: &Run, s: &Span) -> Option<Object> {
    let size = (s.size * run.spacing[2]).max(1e-3);
    (run.gap > 0.0).then(|| r(-run.gap * 1000.0 / size))
}

/// The shows of the line's characters `a..b`, within part `s` of line `t` (in `font`),
/// cut out of the part's shows on the page's operators `ops`: each show's strings cut
/// between character codes, read as the page was read ([`FontCtx::codes`]). A spacing
/// number goes with the character before it, or at the start of a show with the first
/// one after it; a gap read as a space is put back as the same move. A move that is the
/// first character, a space, comes apart: at the start of a show it would read as no
/// space, so it goes at the end of the shows before. `None` when a cut would go through a
/// character code (a ligature's letters, say), or through a string LimarePDF can't be
/// sure it parts as it draws ([`FontCtx::codes_known`], [`FontCtx::drawn_alike`]), or
/// when the characters don't add up as the line was read.
fn shows_cut(
    ops: &[Operation],
    t: &TextData,
    s: &Span,
    font: Option<&FontCtx>,
    (a, b): (usize, usize),
) -> Option<(Option<Object>, Shows)> {
    let mut out = Vec::new();
    let mut lead = None;
    for k in s.runs.0..s.runs.1 {
        let (run, (ra, rb)) = (&t.runs[k], t.pieces[k].chars);
        if ra > b || rb < a {
            continue;
        }
        // Each character's place in the line, and the run's text so far (a big negative
        // spacing number reads as a space only after a character that isn't one).
        let (mut at, mut text) = (ra, String::new());
        let has = |from: usize, to: usize| a <= from && to <= b;
        let mut kept: Vec<Object> = Vec::new();
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
                    let (mut part, mut left) = (Vec::new(), false);
                    for (raw, decoded) in codes {
                        let n = decoded.chars().count();
                        let inside = match n {
                            // A glyph with no characters, as a spacing number.
                            0 if text.is_empty() => has(at, at + 1),
                            0 => a < at && at <= b,
                            _ if has(at, at + n) => true,
                            _ if at < a && a < at + n || at < b && b < at + n => return None,
                            _ => false,
                        };
                        if inside {
                            part.extend_from_slice(raw);
                        } else {
                            left = true;
                        }
                        text.push_str(&decoded);
                        at += n;
                    }
                    let safe = font.is_none_or(|f| f.codes_known && f.drawn_alike(bytes));
                    if left && !part.is_empty() && !safe {
                        return None;
                    }
                    if !part.is_empty() {
                        kept.push(Object::String(part, *format));
                    }
                }
                o => {
                    let x = o.as_float().unwrap_or(0.0);
                    let space = x < -150.0 && !text.ends_with(' ') && !text.is_empty();
                    let inside = if space {
                        text.push(' ');
                        at += 1;
                        has(at - 1, at)
                    } else if text.is_empty() {
                        has(at, at + 1)
                    } else {
                        a < at && at <= b
                    };
                    if space && at - 1 == a && out.is_empty() && kept.is_empty() {
                        lead = Some(it.clone());
                    } else if inside {
                        kept.push(it.clone());
                    }
                }
            }
        }
        let gap = gap_move(run, s);
        if at + usize::from(gap.is_some()) != rb {
            return None;
        }
        if gap.is_some() && rb - 1 == a && out.is_empty() && kept.is_empty() {
            lead = gap;
        } else if gap.is_some() && has(rb - 1, rb) {
            kept.extend(gap);
        }
        if !kept.is_empty() {
            out.push((run.spacing, op("TJ", vec![Object::Array(kept)])));
        }
    }
    Some((lead, out))
}

/// Show `show` followed by the move `x`, as a `TJ`.
fn then_move(show: &mut Operation, x: Object) {
    let mut items = match show.operator.as_str() {
        "TJ" => show_items(show, Show::TJ),
        _ => show.operands.clone(),
    };
    items.push(x);
    *show = op("TJ", vec![Object::Array(items)]);
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

fn op(name: &str, operands: Vec<Object>) -> Operation {
    Operation::new(name, operands)
}
fn r(x: f32) -> Object {
    Object::Real(x)
}
fn name(n: &str) -> Object {
    Object::Name(n.as_bytes().to_vec())
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
fn lit(b: Vec<u8>) -> Object {
    Object::String(b, StringFormat::Literal)
}

/// What a show operation leaves behind when its text is removed (the line move of ' and ").
fn move_only(show: Show) -> Vec<Operation> {
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
fn closed(mut ops: Vec<Operation>, depth_left: usize) -> Vec<Operation> {
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
struct Following {
    /// The show that carries on from the line, and the first line move after it.
    show: Option<usize>,
    line: Option<usize>,
    tm: M,
    tlm: M,
    vertical: bool,
}

impl Following {
    fn of(ops: &[Operation], obj: &Obj) -> Self {
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
    fn load_edit(&self, page_index: usize) -> Result<(ObjectId, Parsed)> {
        let page = self.page_id_at(page_index)?;
        let parsed = self.parse_page(page_index)?;
        if parsed.ops.iter().any(|o| o.operator == "BI") {
            bail!("this page has an inline image that cannot be preserved on rewrite; editing the page is disabled");
        }
        Ok((page, parsed))
    }

    fn pick(parsed: &Parsed, id: usize) -> Result<&Obj> {
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
        let mut n = 1;
        let nm = loop {
            let c = format!("{prefix}{n}");
            if !d.has(c.as_bytes()) {
                break c;
            }
            n += 1;
        };
        d.set(nm.as_bytes().to_vec(), obj);
        res.set(cat, d);
        self.doc.get_dictionary_mut(page)?.set("Resources", res);
        Ok(nm)
    }

    /// The font to write `text` in `f` with, added to the page's resources: its name there,
    /// and the pen that writes the text (the standard font, or its embedded face when the
    /// text needs it).
    fn text_font(&mut self, page: ObjectId, f: FontChoice, text: &str) -> Result<(String, Pen)> {
        let pen = self.pen(f, text)?;
        let font = self.pen_font(&pen);
        Ok((
            self.add_resource(page, "Font", "LimareEd", font.into())?,
            pen,
        ))
    }

    /// D in user space for a displayed-page (y down) delta.
    fn user_delta(vd: M, d_vis: M) -> Result<M> {
        let inv = mx::inv(vd).ok_or_else(|| anyhow!("degenerate page transform"))?;
        Ok(mul(mul(inv, d_vis), vd))
    }

    /// Move and/or resize an object so that its bounding box becomes `new_box` (displayed-page points, y down).
    pub fn set_object_bbox(
        &mut self,
        page_index: usize,
        id: usize,
        new_box: [f32; 4],
    ) -> Result<()> {
        let objs = self.page_objects(page_index)?;
        let old = objs
            .get(id)
            .ok_or_else(|| anyhow!("object {id} no longer exists"))?
            .bbox;
        let (ow, oh) = (old[2] - old[0], old[3] - old[1]);
        let (nw, nh) = (new_box[2] - new_box[0], new_box[3] - new_box[1]);
        if nw < 0.5 || nh < 0.5 {
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
        self.transform_object(page_index, id, d)
    }

    /// Apply `d_vis` (a matrix in displayed-page space, y down) to an object.
    pub fn transform_object(&mut self, page_index: usize, id: usize, d_vis: M) -> Result<()> {
        let (page, parsed) = self.load_edit(page_index)?;
        let (vd, _, _) = self.vis_down(page);
        let obj = Self::pick(&parsed, id)?.clone();
        let d = Self::user_delta(vd, d_vis)?;
        let ictm = mx::inv(obj.ctm)
            .ok_or_else(|| anyhow!("the object has a degenerate transform and cannot be edited"))?;
        let m = mul(mul(obj.ctm, d), ictm);
        let mut following = Following::of(&parsed.ops, &obj);
        let mut out: Vec<Operation> = Vec::with_capacity(parsed.ops.len() + 8);
        for (i, o) in parsed.ops.iter().enumerate() {
            out.extend(following.before(i));
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
                let tm = match following.show.take() {
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
            if obj.text.is_some() && i > obj.first && i <= obj.last {
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
        let (page, parsed) = self.load_edit(page_index)?;
        let obj = Self::pick(&parsed, id)?.clone();
        let following = Following::of(&parsed.ops, &obj);
        let mut out = Vec::with_capacity(parsed.ops.len());
        for (i, o) in parsed.ops.iter().enumerate() {
            out.extend(following.before(i));
            let inside = i >= obj.first && i <= obj.last;
            if !inside {
                out.push(o.clone());
                continue;
            }
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
        self.commit(page, out, parsed.depth_left, &parsed.inline)
    }

    /// Change the text of a text object. Keeps the original font when it can be reused for plain ASCII;
    /// otherwise the line is replaced by new text in a standard font at the same position, or its
    /// embedded face for characters the standard font lacks. A line in parts
    /// ([`TextEdit::parts`]) is written as one line on top, each part in its style.
    pub fn edit_text(&mut self, page_index: usize, id: usize, e: &TextEdit) -> Result<()> {
        self.edit_texts(page_index, &[(id, e.clone())]).map(|_| ())
    }

    /// Change several text objects of one page at once, each as [`Pdf::edit_text`] would,
    /// in one rewrite of the page's content: the ids are as the page is now, and the lines
    /// written anew go on top in the order given. A part that keeps its own characters'
    /// shows ([`TextPart::from`]) is read back: a line that doesn't read as it should then
    /// is written without them. Returns the ids of the lines where characters that asked
    /// to keep their shows couldn't, and were written in another font.
    pub fn edit_texts(
        &mut self,
        page_index: usize,
        edits: &[(usize, TextEdit)],
    ) -> Result<Vec<usize>> {
        let (page, parsed) = self.load_edit(page_index)?;
        let mut cut = vec![true; edits.len()];
        loop {
            let e = self.edited_ops(page, &parsed, edits, &cut)?;
            let misread = self.misread(page, &parsed.inline, &e.ops, &e.cuts)?;
            if misread.is_empty() {
                self.commit(page, e.ops, e.depth, &parsed.inline)?;
                return Ok(e.lost);
            }
            for k in misread {
                cut[k] = false;
            }
        }
    }

    /// The page's operators with `edits` made ([`Pdf::edit_texts`]), each edit's parts
    /// cutting shows out of its line only where `cut` says.
    fn edited_ops(
        &mut self,
        page: ObjectId,
        parsed: &Parsed,
        edits: &[(usize, TextEdit)],
        cut: &[bool],
    ) -> Result<Edited> {
        // A line is written in place of its first show (one style, its font kept), or on
        // top of the page without its shows.
        enum Plan {
            InPlace(usize, String),
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
        // The lines written in parts, together, so they share the fonts they add.
        let in_parts: Vec<usize> = (0..edits.len())
            .filter(|&k| !lines[k].1.hidden && edits[k].1.one_style(lines[k].1).is_none())
            .collect();
        let asked: Vec<_> = (in_parts.iter())
            .map(|&k| {
                (
                    Some((&parsed.ops[..], lines[k].1)),
                    &edits[k].1.parts[..],
                    cut[k],
                )
            })
            .collect();
        let mut written = self.written_lines(page, &asked)?.into_iter();
        let mut lost = Vec::new();
        let mut plans = Vec::new();
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
                    let (fname, pen) = self.text_font(page, FontChoice::default(), &e.text)?;
                    let tz =
                        (100.0 * width / pen.width(&e.text, size).max(0.1)).clamp(1.0, 10_000.0);
                    ops.extend([
                        op("q", vec![]),
                        op("BT", vec![]),
                        op("Tr", vec![Object::Integer(3)]),
                        op("Tf", vec![name(&fname), r(size)]),
                        op("Tz", vec![r(tz)]),
                        at,
                        op("Tj", vec![pen.string(&e.text)]),
                        op("ET", vec![]),
                        op("Q", vec![]),
                    ]);
                }
                Plan::OnTop(ops)
            } else if let Some(one) = e.one_style(t) {
                if one.keeps_font(t.reuse) {
                    // In place of the first show with characters, which is in the line's font.
                    let first_run = (t.pieces.iter())
                        .position(|p| p.chars.1 > p.chars.0)
                        .map_or(t.runs[0].op, |k| t.runs[k].op);
                    Plan::InPlace(first_run, one.text)
                } else {
                    // Replacement: the new text on top, in one style.
                    let font = one.font.unwrap_or_else(|| FontChoice::of_font(&t.base));
                    let size = one.size.unwrap_or(t.size).max(1.0);
                    let color = one.color.or(t.fill).unwrap_or([0.0; 3]);
                    let (fname, pen) = self.text_font(page, font, &one.text)?;
                    Plan::OnTop(vec![
                        op("q", vec![]),
                        op("BT", vec![]),
                        op("Tf", vec![name(&fname), r(size)]),
                        rgb("rg", color),
                        at,
                        op("Tj", vec![pen.string(&one.text)]),
                        op("ET", vec![]),
                        op("Q", vec![]),
                    ])
                }
            } else {
                // In parts, written on top as one line.
                let (parts, gone) = written.next().expect("written");
                if gone {
                    lost.push(edits[k].0);
                }
                let mut ops = Vec::new();
                if !parts.is_empty() {
                    ops.push(op("q", vec![]));
                    ops.extend(line_ops(tm, &parts));
                    ops.push(op("Q", vec![]));
                }
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
        let mut shows: HashMap<usize, (Show, usize)> = HashMap::new();
        for (k, (_, t)) in lines.iter().enumerate() {
            shows.extend(t.runs.iter().map(|r| (r.op, (r.show, k))));
        }
        let mut out = Vec::with_capacity(parsed.ops.len() + 8 * edits.len());
        for (i, o) in parsed.ops.iter().enumerate() {
            out.extend(following.iter().filter_map(|(_, f)| f.before(i)));
            match shows.get(&i) {
                Some(&(show, k)) => {
                    out.extend(move_only(show));
                    if let Plan::InPlace(first, text) = &plans[k] {
                        if *first == i {
                            out.push(op("Tj", vec![lit(text.clone().into_bytes())]));
                        }
                    }
                }
                None => out.push(o.clone()),
            }
        }
        if plans.iter().all(|p| matches!(p, Plan::InPlace(..))) {
            let (ops, depth, cuts) = (out, parsed.depth_left, Vec::new());
            return Ok(Edited {
                ops,
                depth,
                cuts,
                lost,
            });
        }
        let mut ops = closed(out, parsed.depth_left);
        let mut cuts = Vec::new();
        for (k, plan) in plans.into_iter().enumerate() {
            if let Plan::OnTop(line) = plan {
                let from = ops.len();
                ops.extend(line);
                let shows_cut = (edits[k].1.parts.iter()).any(|p| p.from.is_some()) && cut[k];
                if shows_cut && !lines[k].1.hidden {
                    cuts.push((k, (from, ops.len()), edits[k].1.text.clone()));
                }
            }
        }
        Ok(Edited {
            ops,
            depth: 0,
            cuts,
            lost,
        })
    }

    /// Which of the lines written with shows cut out of their old ones (their index, their
    /// operators from and to in `ops`, and the text they should read) don't read as the
    /// same words, as the page will be read with `ops` as its content.
    fn misread(
        &self,
        page: ObjectId,
        inline: &Inline,
        ops: &[Operation],
        cuts: &[(usize, (usize, usize), String)],
    ) -> Result<Vec<usize>> {
        if cuts.is_empty() {
            return Ok(Vec::new());
        }
        let bytes = (inline.encode(ops.to_vec()))
            .map_err(|e| anyhow!("cannot encode the page content: {e}"))?;
        let parsed = self.parse_as_page(page, &bytes)?;
        let words = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        Ok(cuts
            .iter()
            .filter(|(_, (from, to), want)| {
                let read: String = (parsed.objs.iter())
                    .filter(|o| (*from..*to).contains(&o.first))
                    .filter_map(|o| o.text.as_ref())
                    .map(|t| t.text.as_str())
                    .collect();
                words(&read) != words(want)
            })
            .map(|(k, ..)| *k)
            .collect())
    }

    /// `parts` as they are written, with the fonts they need added to the page's
    /// resources ([`Pdf::written_lines`]).
    fn written(
        &mut self,
        page: ObjectId,
        line: Option<(&[Operation], &TextData)>,
        parts: &[TextPart],
    ) -> Result<Vec<Written>> {
        let mut lines = self.written_lines(page, &[(line, parts, false)])?;
        Ok(lines.remove(0).0)
    }

    /// Lines of parts as they are written ([`Asked`]), with the fonts they need added to the
    /// page's resources, one for each font chosen. The parts of an edited line keep the
    /// font of the line's part they name when they can ([`PageObject::keeps_font`]),
    /// cutting their shows out of that part's when allowed; new text has none to name.
    /// Empty parts are left out. With each line, whether characters that asked to keep
    /// their shows couldn't.
    fn written_lines(
        &mut self,
        page: ObjectId,
        lines: &[Asked],
    ) -> Result<Vec<(Vec<Written>, bool)>> {
        // A part keeps its font, written with these shows, or is written in a font added.
        enum How {
            Own(Vec<u8>, Vec<([f32; 3], Operation)>),
            New(FontChoice),
        }
        let res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
        let page_fonts = self.resolved_dict(res.get(b"Font").ok());
        let mut hows = Vec::new();
        {
            // The page's fonts as the page is read, for cutting shows.
            let mut ctxs: HashMap<&[u8], Option<FontCtx>> = HashMap::new();
            for &(line, parts, cut) in lines {
                let chars: Vec<char> = line.map_or(Vec::new(), |(_, t)| t.text.chars().collect());
                let mut gone = false;
                let mut line_hows = Vec::new();
                for p in parts.iter().filter(|p| !p.text.is_empty()) {
                    let how = match (p.font, line) {
                        (PartFont::Choice(c), _) => How::New(c),
                        (PartFont::Span(i), Some((ops, t))) => {
                            let s = (t.spans.get(i)).ok_or_else(|| {
                                anyhow!("the line has no part {i} (the page changed)")
                            })?;
                            let ascii_show = || {
                                let spacing = t.runs[s.runs.0].spacing;
                                let show = op("Tj", vec![lit(p.text.clone().into_bytes())]);
                                How::Own(s.font.clone(), vec![(spacing, show)])
                            };
                            match kept(&chars, s.chars, s.reuse, t.vertical, &p.text, p.from) {
                                Some(Kept::AsItWas) => {
                                    How::Own(s.font.clone(), shows_as_they_were(ops, t, s))
                                }
                                Some(Kept::Cut(a, b)) => {
                                    let ctx = ctxs.entry(s.font.as_slice()).or_insert_with(|| {
                                        let d = page_fonts.get(&s.font).ok()?;
                                        let (_, d) = self.doc.dereference(d).ok()?;
                                        d.as_dict().ok().map(|d| FontCtx::new(&self.doc, d))
                                    });
                                    let shows = (cut)
                                        .then(|| shows_cut(ops, t, s, ctx.as_ref(), (a, b)))
                                        .flatten();
                                    match shows {
                                        Some((lead, mut shows)) => {
                                            // A space made by a move ends the part before.
                                            let before = line_hows.last_mut();
                                            match (lead, before) {
                                                (Some(x), Some((_, _, tail))) => *tail = Some(x),
                                                (Some(x), None) => {
                                                    let x = op("TJ", vec![Object::Array(vec![x])]);
                                                    shows.insert(0, (run_spacing(t, s), x));
                                                }
                                                (None, _) => {}
                                            }
                                            How::Own(s.font.clone(), shows)
                                        }
                                        None if s.reuse && ascii(&p.text) => ascii_show(),
                                        None => {
                                            gone = true;
                                            How::New(FontChoice::of_font(&s.base))
                                        }
                                    }
                                }
                                Some(Kept::Ascii) => ascii_show(),
                                None => How::New(FontChoice::of_font(&s.base)),
                            }
                        }
                        (PartFont::Span(_), None) => {
                            bail!("new text has no font of its own to keep")
                        }
                    };
                    line_hows.push((p, how, None));
                }
                hows.push((line_hows, gone));
            }
        }
        // One font added for each one chosen, holding all the text written in it.
        let mut texts: Vec<(FontChoice, String)> = Vec::new();
        for (p, how, _) in hows.iter().flat_map(|(h, _)| h) {
            if let How::New(c) = how {
                match texts.iter_mut().find(|(f, _)| f == c) {
                    Some((_, text)) => text.push_str(&p.text),
                    None => texts.push((*c, p.text.clone())),
                }
            }
        }
        let mut fonts = Vec::new();
        for (c, text) in texts {
            let (name, pen) = self.text_font(page, c, &text)?;
            fonts.push((c, name, pen));
        }
        let write = |(p, how, tail): (&TextPart, How, Option<Object>)| {
            let (font, mut shows) = match how {
                How::Own(font, shows) => (font, shows),
                How::New(c) => {
                    let (_, name, pen) = fonts.iter().find(|(f, ..)| *f == c).expect("added");
                    let show = op("Tj", vec![pen.string(&p.text)]);
                    (name.as_bytes().to_vec(), vec![([0.0, 0.0, 1.0], show)])
                }
            };
            match (tail, shows.last_mut()) {
                (Some(x), Some((_, show))) => then_move(show, x),
                (Some(x), None) => {
                    shows.push(([0.0, 0.0, 1.0], op("TJ", vec![Object::Array(vec![x])])))
                }
                (None, _) => {}
            }
            Written {
                font,
                size: p.size.max(1.0),
                color: p.color,
                rise: p.rise,
                shows,
            }
        };
        Ok(hows
            .into_iter()
            .map(|(h, gone)| (h.into_iter().map(write).collect(), gone))
            .collect())
    }

    pub fn set_path_style(&mut self, page_index: usize, id: usize, st: &PathStyle) -> Result<()> {
        let (page, parsed) = self.load_edit(page_index)?;
        let obj = Self::pick(&parsed, id)?.clone();
        let p = obj
            .path
            .clone()
            .ok_or_else(|| anyhow!("object {id} is not a path"))?;
        let scale = obj.ctm[0].hypot(obj.ctm[1]).max(1e-6);
        let paint = match (st.fill.is_some(), st.stroke.is_some()) {
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
        };
        let mut out = Vec::with_capacity(parsed.ops.len() + 6);
        for (i, o) in parsed.ops.iter().enumerate() {
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
        if parts.iter().all(|p| p.text.is_empty()) {
            bail!("empty text");
        }
        let parts = parts.to_vec();
        self.append_layer(page_index, move |s, page| {
            let written = s.written(page, None, &parts)?;
            Ok(line_ops([1.0, 0.0, 0.0, -1.0, pos.0, pos.1], &written))
        })
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
        let want = |text: &str| vec![(3, (0, ops.len()), text.to_string())];
        let misread = |text: &str| p.misread(page, &inline, &ops, &want(text)).unwrap();
        assert_eq!(misread("one two"), [3]);
        assert_eq!(misread("onetwo"), Vec::<usize>::new());
        assert_eq!(misread(" onetwo "), Vec::<usize>::new());
    }
}
