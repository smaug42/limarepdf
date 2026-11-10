//! Underlines and strike-throughs (NEW-07; `limarepdf-underline-design.md`). PDF text has
//! none: a decoration is a thin band drawn under or through some of a line's characters.
//! Other apps' are read by one rule ([`read`], the design's section 2), each band kept with
//! its line, so that a line's edits, moves and deletion take it along. Reading is careful:
//! a band LimarePDF isn't sure of is left a drawing.

use crate::budget::{TooComplex, Work};
use crate::colour::Paint;
use crate::edit::{op, r, Check};
use crate::fonts::FontChoice;
use crate::layout::SetIn;
use crate::matrix::{self as mx, apply, M};
use crate::objects::{Obj, ObjectKind, Parsed, TextData};
use crate::own_encoding::MAX_PROGRAM;
use crate::Pdf;
use lopdf::content::Operation;
use lopdf::{Dictionary, Document, Object, ObjectId};
use skrifa::raw::{FontRef, TableProvider};
use unicode_segmentation::UnicodeSegmentation;

/// What [`read`] did, counted where it does it, for the tests to set against the steps it
/// took (CP-05): line directions compared with one another, bands' directions compared with
/// them, boxes the grids gave, characters given to [`fits`], boxes of form text looked at,
/// decorations found compared, and bands already in a decoration another band joins.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Tally {
    ways: u64,
    levels: u64,
    occupants: u64,
    chars: u64,
    forms: u64,
    found: u64,
    merged: u64,
    /// The steps [`read`] took.
    steps: u64,
}

#[cfg(test)]
thread_local! {
    static TALLY: std::cell::Cell<Tally> = const {
        std::cell::Cell::new(Tally {
            ways: 0,
            levels: 0,
            occupants: 0,
            chars: 0,
            forms: 0,
            found: 0,
            merged: 0,
            steps: 0,
        })
    };
}

/// Adds `n` to the tests' [`Tally`]'s `field`; nothing outside the tests.
macro_rules! tally {
    ($field:ident, $n:expr) => {
        #[cfg(test)]
        TALLY.with(|t| {
            let mut v = t.get();
            v.$field += $n as u64;
            t.set(v);
        });
    };
}

/// Which line a decoration is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecoKind {
    Underline,
    Strike,
}

/// How a decoration looks: each band's centre above its characters' baseline (below when
/// negative) and thickness, in ems of their size, and the ink it is drawn in.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    bands: Vec<(f32, f32)>,
    ink: Paint,
    colour: [f32; 3],
}

impl Look {
    /// Its bands: each one's centre above the baseline and thickness, in ems.
    pub fn bands(&self) -> &[(f32, f32)] {
        &self.bands
    }

    /// The colour it shows as.
    pub fn colour(&self) -> [f32; 3] {
        self.colour
    }
}

/// A decoration drawn for a line of text ([`crate::PageObject::decorations`]): over its
/// characters `chars` (from, to), in `look`, drawn by the page's paths `paths` (their ids).
#[derive(Clone, Debug, PartialEq)]
pub struct Decoration {
    pub kind: DecoKind,
    pub chars: (usize, usize),
    pub look: Look,
    pub paths: Vec<usize>,
}

/// A decoration to draw for an edited line or new text: over its characters `chars`
/// (from, to), in `look`, or with `None` at its font's own place and thickness in its
/// characters' ink.
#[derive(Clone, Debug, PartialEq)]
pub struct Decorated {
    pub kind: DecoKind,
    pub chars: (usize, usize),
    pub look: Option<Look>,
}

/// A path's construction, as far as telling a straight band needs.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Rect(f32, f32, f32, f32),
    Close,
    Curve,
}

/// A painted path that is one straight band, in page user space: its centre line from `a`
/// to `b` and its thickness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Straight {
    pub a: (f32, f32),
    pub b: (f32, f32),
    pub width: f32,
}

/// A painted path that is one band, as the parser found it: its first operator (which
/// finds its object once the objects are in order), the band, the colour it shows as and
/// what set that colour.
#[derive(Clone, Debug)]
pub(crate) struct Painted {
    pub first: usize,
    pub band: Straight,
    pub colour: Option<[f32; 3]>,
    pub ink: InkAt,
}

/// Where a colour was set in a page's operators: the operator that chose its colour space
/// (`cs`, `CS`), and the one that set the colour (`g`, `rg`, `k`, `sc`, `scn`, or a
/// stroke's own). The ink is made from them ([`InkAt::paint`]) only for a band read as a
/// decoration, so reading a page copies none.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InkAt {
    pub space: Option<usize>,
    pub set: Option<usize>,
}

impl InkAt {
    /// Set by operator `i` in a device space.
    pub(crate) fn set_by(i: usize) -> Self {
        InkAt {
            space: None,
            set: Some(i),
        }
    }

    /// The colour space chosen by operator `i`, in its first colour.
    pub(crate) fn space_of(i: usize) -> Self {
        InkAt {
            space: Some(i),
            set: None,
        }
    }

    /// The fill operators that set this colour again, from the page's operators `ops`, as
    /// the page's reading makes them ([`Paint`]).
    pub(crate) fn paint(&self, ops: &[Operation]) -> Paint {
        let fill = |op: &str| match op {
            "G" => "g",
            "RG" => "rg",
            "K" => "k",
            "SC" => "sc",
            "SCN" => "scn",
            _ => "",
        };
        let space = (self.space.and_then(|i| ops.get(i))).and_then(|o| o.operands.first());
        let base = space.map_or_else(Paint::default, Paint::space);
        match self.set.and_then(|i| ops.get(i)) {
            None => base,
            Some(o) => {
                let op = match fill(&o.operator) {
                    "" => o.operator.as_str(),
                    f => f,
                };
                match op {
                    "g" | "rg" | "k" => Paint::device(op, &o.operands),
                    _ => base.with(op, &o.operands),
                }
            }
        }
    }
}

/// The band path `segs` paints under transform `ctm`, if it is one: filled rectangles (an
/// `re`, or four corners closed, any way round), or one stroked straight segment without a
/// dash pattern, `lw` wide. Several filled rectangles of one thickness in a row are one band
/// when their gaps are under a quarter of its length: an underline broken where descenders
/// cross it (macOS draws them so), not a dotted line.
pub(crate) fn straight(
    segs: &[Seg],
    ctm: M,
    fill: bool,
    stroke: bool,
    lw: f32,
    dashed: bool,
) -> Option<Straight> {
    let at = |x: f32, y: f32| apply(ctm, x, y);
    match (fill, stroke, segs) {
        // The commonest: one rectangle.
        (true, false, [Seg::Rect(x, y, w, h)] | [Seg::Rect(x, y, w, h), Seg::Close]) => {
            band([at(*x, *y), at(x + w, *y), at(x + w, y + h), at(*x, y + h)])
        }
        (true, false, _) => {
            let mut pieces = Vec::new();
            let mut rest = segs;
            while !rest.is_empty() {
                let (piece, more) = filled_piece(rest, &at)?;
                pieces.push(piece);
                rest = more;
            }
            in_a_row(&pieces)
        }
        (
            false,
            true,
            [Seg::Move(x0, y0), Seg::Line(x1, y1)]
            | [Seg::Move(x0, y0), Seg::Line(x1, y1), Seg::Close],
        ) if !dashed => {
            let (a, b) = (at(*x0, *y0), at(*x1, *y1));
            let len = (b.0 - a.0).hypot(b.1 - a.1);
            if len < 1e-6 || lw <= 0.0 {
                return None;
            }
            // The pen's width across the segment as drawn: a round pen `lw` across, drawn by
            // `ctm`, is lw |det ctm| / |ctm u| across a segment along unit u.
            let det = (ctm[0] * ctm[3] - ctm[1] * ctm[2]).abs();
            let src = (x1 - x0).hypot(y1 - y0);
            Some(Straight {
                a,
                b,
                width: lw * det * src / len,
            })
        }
        _ => None,
    }
}

/// The filled rectangle `segs` starts with (an `re`, or four corners, closed or back to the
/// start), as a band, and the segments after it.
fn filled_piece<'s>(
    segs: &'s [Seg],
    at: &dyn Fn(f32, f32) -> (f32, f32),
) -> Option<(Straight, &'s [Seg])> {
    let skip_close = |s: &'s [Seg]| match s {
        [Seg::Close, more @ ..] => more,
        _ => s,
    };
    match segs {
        [Seg::Rect(x, y, w, h), more @ ..] => {
            let corners = [at(*x, *y), at(x + w, *y), at(x + w, y + h), at(*x, y + h)];
            Some((band(corners)?, skip_close(more)))
        }
        [Seg::Move(x0, y0), Seg::Line(x1, y1), Seg::Line(x2, y2), Seg::Line(x3, y3), more @ ..] => {
            // A fourth line back to the start closes it as `h` does.
            let more = match more {
                [Seg::Line(x, y), m @ ..] if (x - x0).abs() < 1e-3 && (y - y0).abs() < 1e-3 => m,
                m => m,
            };
            let corners = [at(*x0, *y0), at(*x1, *y1), at(*x2, *y2), at(*x3, *y3)];
            Some((band(corners)?, skip_close(more)))
        }
        _ => None,
    }
}

/// Bands in a row as one: of one thickness, on one line, apart from each other, with gaps
/// under a quarter of the whole length.
fn in_a_row(pieces: &[Straight]) -> Option<Straight> {
    let first = *pieces.first()?;
    if pieces.len() == 1 {
        return Some(first);
    }
    let len = (first.b.0 - first.a.0).hypot(first.b.1 - first.a.1);
    if len < 1e-6 {
        return None;
    }
    let d = ((first.b.0 - first.a.0) / len, (first.b.1 - first.a.1) / len);
    let along = |p: (f32, f32)| (p.0 - first.a.0) * d.0 + (p.1 - first.a.1) * d.1;
    let across = |p: (f32, f32)| -(p.0 - first.a.0) * d.1 + (p.1 - first.a.1) * d.0;
    let mut spans = Vec::with_capacity(pieces.len());
    for p in pieces {
        let level = across(p.a).abs().max(across(p.b).abs()) <= 0.1 * first.width + 0.01;
        if !level || (p.width - first.width).abs() > 0.1 * first.width + 0.01 {
            return None;
        }
        let (u0, u1) = (along(p.a), along(p.b));
        spans.push((u0.min(u1), u0.max(u1)));
    }
    spans.sort_by(|x, y| x.0.total_cmp(&y.0));
    let (start, end) = (spans[0].0, spans.iter().fold(f32::MIN, |m, s| m.max(s.1)));
    let mut gaps = 0.0;
    for w in spans.windows(2) {
        let gap = w[1].0 - w[0].1;
        if gap < -0.01 {
            return None;
        }
        gaps += gap.max(0.0);
    }
    if gaps >= 0.25 * (end - start) {
        return None;
    }
    let point = |u: f32| (first.a.0 + d.0 * u, first.a.1 + d.1 * u);
    Some(Straight {
        a: point(start),
        b: point(end),
        width: first.width,
    })
}

/// The band four corners in order make, when they make a parallelogram: its centre line
/// along the longer side, its thickness across it.
fn band(p: [(f32, f32); 4]) -> Option<Straight> {
    let sub = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0, a.1 - b.1);
    let mid = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let (e1, e2) = (sub(p[1], p[0]), sub(p[3], p[0]));
    let (l1, l2) = (e1.0.hypot(e1.1), e2.0.hypot(e2.1));
    let off = sub(
        (p[0].0 + p[2].0, p[0].1 + p[2].1),
        (p[1].0 + p[3].0, p[1].1 + p[3].1),
    );
    if off.0.hypot(off.1) > 0.01 * (l1 + l2) + 1e-3 || l1.max(l2) < 1e-6 {
        return None;
    }
    let area = (e1.0 * e2.1 - e1.1 * e2.0).abs();
    Some(match l1 >= l2 {
        true => Straight {
            a: mid(p[0], p[3]),
            b: mid(p[1], p[2]),
            width: area / l1,
        },
        false => Straight {
            a: mid(p[0], p[1]),
            b: mid(p[3], p[2]),
            width: area / l2,
        },
    })
}

/// A line's along-and-up frame in page user space: its baseline's start, the way it runs and
/// the way its letters stand up, both unit vectors.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    pub o: (f32, f32),
    pub d: (f32, f32),
    pub u: (f32, f32),
}

impl Frame {
    /// The frame of line `t` drawn under `ctm`: its letters' up is the text matrix's, turned
    /// over for a font that stands its glyphs up only so ([`TextData::flipped`]).
    pub(crate) fn of(t: &TextData, ctm: M) -> Option<Frame> {
        let m = mx::mul(t.runs.first()?.tm, ctm);
        let d = t.dir;
        let side = (-d.1, d.0);
        let mut up = m[2] * side.0 + m[3] * side.1;
        if t.flipped {
            up = -up;
        }
        let u = if up < 0.0 { (d.1, -d.0) } else { side };
        Some(Frame { o: t.origin, d, u })
    }

    /// Where `p` is: how far along the line and how far up from its baseline.
    pub(crate) fn at(&self, p: (f32, f32)) -> (f32, f32) {
        let q = (p.0 - self.o.0, p.1 - self.o.1);
        (
            q.0 * self.d.0 + q.1 * self.d.1,
            q.0 * self.u.0 + q.1 * self.u.1,
        )
    }

    /// The matrix from the frame (along, up) to user space.
    pub(crate) fn matrix(&self) -> M {
        [self.d.0, self.d.1, self.u.0, self.u.1, self.o.0, self.o.1]
    }

    /// The along and up ranges of box `b` (user space).
    fn span_of(&self, b: [f32; 4]) -> ([f32; 2], [f32; 2]) {
        let corners = [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])];
        let mut along = [f32::INFINITY, f32::NEG_INFINITY];
        let mut up = along;
        for c in corners {
            let (x, y) = self.at(c);
            along = [along[0].min(x), along[1].max(x)];
            up = [up[0].min(y), up[1].max(y)];
        }
        (along, up)
    }
}

/// A line's characters as laid out: each one's start and end along its frame, size, rise,
/// colour and ink, and the places between them that are word edges (UAX #29).
pub(crate) struct Laid {
    pub frame: Frame,
    pub chars: Vec<char>,
    pub edges: Vec<(f32, f32)>,
    pub size: Vec<f32>,
    pub rise: Vec<f32>,
    /// Each character's part of the line ([`TextData::spans`]; one past them for a
    /// character in none), and each part's colour, ink and font's name in the page's
    /// resources.
    pub part: Vec<usize>,
    fills: Vec<Option<[f32; 3]>>,
    paints: Vec<Paint>,
    pub fonts: Vec<Vec<u8>>,
    /// One more than the characters: whether a word starts or ends there, found when first
    /// asked.
    word_edges: std::cell::OnceCell<Vec<bool>>,
}

impl Laid {
    /// Line `t`, drawn under `ctm`.
    pub(crate) fn of(t: &TextData, ctm: M) -> Option<Laid> {
        let frame = Frame::of(t, ctm)?;
        let chars: Vec<char> = t.text.chars().collect();
        let n = chars.len();
        if n == 0 {
            return None;
        }
        let mut edges = vec![(f32::NAN, f32::NAN); n];
        for p in &t.pieces {
            let k = p.chars.1.saturating_sub(p.chars.0);
            let (s, e) = (frame.at(p.corners[0]).0, frame.at(p.corners[1]).0);
            for j in 0..k {
                let (f0, f1) = match p.spans.len() == k {
                    true => p.spans[j],
                    false => (j as f32 / k as f32, (j + 1) as f32 / k as f32),
                };
                if let Some(slot) = edges.get_mut(p.chars.0 + j) {
                    *slot = (s + (e - s) * f0, s + (e - s) * f1);
                }
            }
        }
        // A character no piece places (none should be) sits where the one before ends.
        let mut last = 0.0;
        for slot in &mut edges {
            if slot.0.is_nan() {
                *slot = (last, last);
            }
            last = slot.1;
        }
        let mut size = vec![t.size; n];
        let mut rise = vec![0.0; n];
        let mut part = vec![t.spans.len(); n];
        for (k, s) in t.spans.iter().enumerate() {
            for i in s.chars.0..s.chars.1.min(n) {
                (size[i], rise[i], part[i]) = (s.size, s.rise, k);
            }
        }
        let fills = (t.spans.iter().map(|s| s.fill)).chain([t.fill]).collect();
        let paints = (t.spans.iter().map(|s| s.paint.clone()))
            .chain([Paint::default()])
            .collect();
        let fonts = (t.spans.iter().map(|s| s.font.clone()))
            .chain([Vec::new()])
            .collect();
        Some(Laid {
            frame,
            chars,
            edges,
            size,
            rise,
            part,
            fills,
            paints,
            fonts,
            word_edges: Default::default(),
        })
    }

    /// The largest size among characters `a..b`.
    fn size_over(&self, a: usize, b: usize) -> f32 {
        self.size[a..b].iter().fold(0.0, |m, &s| m.max(s))
    }

    /// The colour character `i` shows as.
    fn fill(&self, i: usize) -> Option<[f32; 3]> {
        self.fills[self.part[i]]
    }

    /// What set character `i`'s colour.
    pub(crate) fn paint(&self, i: usize) -> &Paint {
        &self.paints[self.part[i]]
    }

    /// Whether a word starts or ends before character `i` (UAX #29); the line's ends are.
    fn word_edge(&self, i: usize) -> bool {
        let edges = self.word_edges.get_or_init(|| {
            let n = self.chars.len();
            let text: String = self.chars.iter().collect();
            let mut edge = vec![false; n + 1];
            let mut at = 0;
            for w in text.split_word_bounds() {
                edge[at] = true;
                at += w.chars().count();
            }
            edge[n] = true;
            edge
        });
        edges[i]
    }
}

/// A band a line's characters fit (the rule's first five tests): over characters `a..b`,
/// whose size is `s`, how far its ends are from theirs.
struct Fit {
    kind: DecoKind,
    a: usize,
    b: usize,
    s: f32,
    /// Its centre above the characters' baseline, and its ends along the line.
    v: f32,
    along: (f32, f32),
    off: f32,
}

/// How band `st` drawn in `colour` fits line `l`, if it does: thin and long for the
/// characters it covers, level with the line, under them (or through them), its ends at
/// word edges within 0.15 em of theirs, over two characters or more that aren't spaces with
/// no jump between neighbours wider than 2 em, in the colour of every one of them.
fn fits(l: &Laid, st: &Straight, colour: [f32; 3]) -> Option<Fit> {
    tally!(chars, l.chars.len());
    let (ua, va) = l.frame.at(st.a);
    let (ub, vb) = l.frame.at(st.b);
    let (u0, u1) = (ua.min(ub), ua.max(ub));
    let n = l.chars.len();
    // The character whose edge is nearest `u`, among `range`.
    let near = |range: std::ops::Range<usize>, edge: &dyn Fn(usize) -> f32, u: f32| {
        range.min_by(|&x, &y| (edge(x) - u).abs().total_cmp(&(edge(y) - u).abs()))
    };
    let a = near(0..n, &|i| l.edges[i].0, u0)?;
    let last = near(a..n, &|i| l.edges[i].1, u1)?;
    let b = last + 1;
    let s = l.size_over(a, b);
    if s <= 0.0 || st.width > 0.15 * s || u1 - u0 < 0.2 * s || (va - vb).abs() > 0.05 * s {
        return None;
    }
    let rise = l.rise[a];
    let rise = match l.rise[a..b].iter().all(|r| (r - rise).abs() < 0.01) {
        true => rise,
        false => 0.0,
    };
    let v = (va + vb) / 2.0 - rise;
    let kind = if (-0.40 * s..=0.05 * s).contains(&v) && v - st.width / 2.0 <= -0.02 * s {
        DecoKind::Underline
    } else if (0.15 * s..=0.45 * s).contains(&v) {
        DecoKind::Strike
    } else {
        return None;
    };
    let (e0, e1) = ((l.edges[a].0 - u0).abs(), (l.edges[last].1 - u1).abs());
    if e0 > 0.15 * s || e1 > 0.15 * s || !l.word_edge(a) || !l.word_edge(b) {
        return None;
    }
    let shown = (a..b).filter(|&i| !l.chars[i].is_whitespace());
    if shown.clone().count() < 2 || (a..last).any(|i| l.edges[i + 1].0 - l.edges[i].0 > 2.0 * s) {
        return None;
    }
    let same = |c: [f32; 3]| (0..3).all(|k| (c[k] - colour[k]).abs() <= 0.02);
    if !shown.into_iter().all(|i| l.fill(i).is_some_and(same)) {
        return None;
    }
    Some(Fit {
        kind,
        a,
        b,
        s,
        v,
        along: (u0, u1),
        off: e0 + e1,
    })
}

/// Indices of boxes kept in square cells, to find those near a box quickly.
struct Grid {
    cell: f32,
    cells: std::collections::HashMap<(i32, i32), Vec<usize>>,
    /// For each box, the last lookup that gave it: a lookup gives each box once without
    /// sorting what it found.
    seen: Vec<u32>,
    lookup: u32,
}

impl Grid {
    fn new(
        boxes: impl IntoIterator<Item = (usize, [f32; 4])>,
        work: &mut Work,
    ) -> Result<Grid, TooComplex> {
        let mut g = Grid {
            cell: 64.0,
            cells: Default::default(),
            seen: Vec::new(),
            lookup: 0,
        };
        let cell = g.cell;
        let mut most = 0;
        for (i, b) in boxes {
            each_cell(cell, b, work, |c| g.cells.entry(c).or_default().push(i))?;
            most = most.max(i + 1);
        }
        g.seen = vec![0; most];
        Ok(g)
    }

    /// The indices of boxes in the cells box `b` touches, each once, in `out`. Each box in
    /// those cells, as many times as it is there, is a step of `work` with the cells, taken
    /// before any is looked at (CP-05).
    fn near_into(
        &mut self,
        b: [f32; 4],
        out: &mut Vec<usize>,
        work: &mut Work,
    ) -> Result<(), TooComplex> {
        out.clear();
        let mut at: Vec<&[usize]> = Vec::new();
        each_cell(self.cell, b, work, |c| {
            at.extend(self.cells.get(&c).map(Vec::as_slice))
        })?;
        work.take(at.iter().map(|v| v.len() as u64).sum())?;
        if self.lookup == u32::MAX {
            self.seen.fill(0);
            self.lookup = 0;
        }
        self.lookup += 1;
        for &i in at.into_iter().flatten() {
            tally!(occupants, 1);
            if self.seen[i] != self.lookup {
                self.seen[i] = self.lookup;
                out.push(i);
            }
        }
        Ok(())
    }

    /// [`Grid::near_into`], in a list of its own.
    fn near(&mut self, b: [f32; 4], work: &mut Work) -> Result<Vec<usize>, TooComplex> {
        let mut out = Vec::new();
        self.near_into(b, &mut out, work)?;
        Ok(out)
    }
}

/// The most cells one box may touch in a [`Grid`] (S1-07): a box drawn hundreds of metres
/// across, past them, touches none, so it meets nothing.
const MOST_CELLS: u64 = 1 << 15;

/// Calls `f` with each `cell`-sized square cell box `b` touches, and returns the cells it
/// went through. A box over very many cells (a page-wide background) touches only its edge
/// rows and columns, where it can meet a band, going through those alone, each cell once;
/// a box with more than [`MOST_CELLS`] even so touches none. The cells are steps of `work`,
/// taken before any is touched.
fn each_cell(
    cell: f32,
    b: [f32; 4],
    work: &mut Work,
    mut f: impl FnMut((i32, i32)),
) -> Result<u64, TooComplex> {
    if !b.iter().all(|x| x.is_finite()) {
        return Ok(0);
    }
    let k = |x: f32| (x / cell).floor().clamp(-1e6, 1e6) as i32;
    let (x0, y0, x1, y1) = (k(b[0]), k(b[1]), k(b[2]), k(b[3]));
    if x1 < x0 || y1 < y0 {
        return Ok(0);
    }
    let (w, h) = ((x1 - x0 + 1) as u64, (y1 - y0 + 1) as u64);
    // With a side of one or two cells, every cell is an edge's.
    let whole = w * h <= 4096 || w.min(h) <= 2;
    let n = match whole {
        true => w * h,
        false => 2 * (w + h) - 4,
    };
    if n > MOST_CELLS {
        return Ok(0);
    }
    work.take(n)?;
    let mut passes = 0;
    let mut f = |c| {
        passes += 1;
        f(c)
    };
    if whole {
        for x in x0..=x1 {
            for y in y0..=y1 {
                f((x, y));
            }
        }
    } else {
        for x in x0..=x1 {
            f((x, y0));
            f((x, y1));
        }
        for y in y0 + 1..y1 {
            f((x0, y));
            f((x1, y));
        }
    }
    Ok(passes)
}

fn grown(b: [f32; 4], by: f32) -> [f32; 4] {
    [b[0] - by, b[1] - by, b[2] + by, b[3] + by]
}

fn meets(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// Whether `p` holds for any of `items`, each looked at a step of `work`, taken before it
/// is (CP-05): those after the first that holds aren't looked at, or taken.
fn any_of<T>(
    items: &[T],
    work: &mut Work,
    mut p: impl FnMut(&T) -> bool,
) -> Result<bool, TooComplex> {
    for x in items {
        work.take(1)?;
        if p(x) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A text line's box in user space, from its pieces.
fn text_box(t: &TextData) -> [f32; 4] {
    mx::aabb(t.pieces.iter().flat_map(|p| p.corners))
}

/// A line as far as telling which bands can't be its takes ([`read`]): its object, frame,
/// largest size, and how far along its frame its text reaches.
struct Lite {
    obj: usize,
    frame: Frame,
    size: f32,
    reach: (f32, f32),
}

impl Lite {
    /// Object `o` (index `i`), when it is a visible line written across.
    fn of(i: usize, o: &Obj) -> Option<Lite> {
        let t = o.text.as_ref()?;
        if t.hidden || t.vertical || t.pieces.is_empty() {
            return None;
        }
        let frame = Frame::of(t, o.ctm)?;
        let size = t.spans.iter().fold(t.size, |m, s| m.max(s.size));
        let mut reach = (f32::INFINITY, f32::NEG_INFINITY);
        for c in t.pieces.iter().flat_map(|p| &p.corners[..2]) {
            let u = frame.at(*c).0;
            reach = (reach.0.min(u), reach.1.max(u));
        }
        (size > 0.0).then_some(Lite {
            obj: i,
            frame,
            size,
            reach,
        })
    }

    /// Whether band `st` can be this line's ([`fits`] asks more): thin for its largest text,
    /// level with it, near its baseline, and along its text somewhere.
    fn may_take(&self, st: &Straight) -> bool {
        let s = self.size;
        let (ua, va) = self.frame.at(st.a);
        let (ub, vb) = self.frame.at(st.b);
        st.width <= 0.15 * s
            && (va - vb).abs() <= 0.05 * s
            && va.max(vb) >= -0.6 * s
            && va.min(vb) <= 0.8 * s
            && ua.max(ub) >= self.reach.0
            && ua.min(ub) <= self.reach.1
    }
}

/// Whether a band at height `v`, among `vs` (the heights of the bands like it near it, its
/// own aside), is one of three or more in a row evenly spaced (their gaps within a tenth of
/// each other), each gap wider than `width` and narrower than `em`: a staff's lines rather
/// than a decoration (PB-101). Heights within half `width` of each other are one line drawn
/// twice.
fn evenly_spaced(vs: &mut Vec<f32>, v: f32, width: f32, em: f32) -> bool {
    vs.push(v);
    vs.sort_by(f32::total_cmp);
    vs.dedup_by(|a, b| *a - *b < width / 2.0);
    let Some(i) = vs.iter().position(|x| (x - v).abs() < width / 2.0) else {
        return false;
    };
    let spaced =
        |g: f32, h: f32| g.min(h) > width && g.max(h) < em && (g - h).abs() <= 0.1 * g.max(h);
    (i.saturating_sub(2)..=i).any(|j| match vs.get(j..j + 3) {
        Some(&[a, b, c]) => spaced(b - a, c - b),
        _ => false,
    })
}

/// The decorations of the page's own lines among `objs` (the page's objects as parsed, in
/// user space), each with its line's index: a painted band is one when it fits a line
/// ([`fits`], the closest fitting line taking it), meets no other drawing at its ends
/// (bands under the same line's text and a fill behind it aside), no other text's box
/// overlaps it or sits centred under it within its ends (a fraction's denominator), and it
/// is not one of a staff's lines ([`evenly_spaced`]).
/// `form_text` is the text inside forms the page draws, which counts as other text. Two
/// bands of one kind over the same characters are one decoration (a double underline).
/// The cells its grids touch are steps of the page's `work` (S1-07), and so is the rest of
/// what can grow faster than the page (CP-05), each part taken before it is done: each
/// line's way set against the ways before it, and each thin band's against the ways, up to
/// the first that is the same ([`any_of`]); each box the grids give for a band; each
/// character of a line a band is tried against; each line of a form's text looked at for a
/// band; each decoration found before set against a band's; and each band already in the
/// decoration a band joins. What isn't (a band's object found by halving, each line laid
/// out once, the decorations put in order) grows no faster than n log n of the page's lines
/// and bands.
pub(crate) fn read(
    objs: &[Obj],
    painted: &[Painted],
    ops: &[Operation],
    form_text: &[TextData],
    work: &mut Work,
) -> Result<Vec<(usize, Decoration)>, TooComplex> {
    #[cfg(test)]
    let start = work.taken();
    if painted.is_empty() {
        return Ok(Vec::new());
    }
    // Each line's frame, largest size and reach along it: enough to pass over the bands
    // that can't be its, before its characters are laid out (when first needed).
    let lines: Vec<Lite> = (objs.iter().enumerate())
        .filter_map(|(i, o)| Lite::of(i, o))
        .collect();
    let biggest = lines.iter().fold(0.0f32, |m, l| m.max(l.size));
    if lines.is_empty() || biggest <= 0.0 {
        return Ok(Vec::new());
    }
    // The ways the page's lines run: a band running none of them (a table's column rule
    // beside lines across) is no line's.
    let mut ways: Vec<(f32, f32)> = Vec::new();
    for l in &lines {
        let d = l.frame.d;
        let known = any_of(&ways, work, |w| {
            tally!(ways, 1);
            (w.0 * d.1 - w.1 * d.0).abs() < 1e-3
        })?;
        if !known {
            ways.push(d);
        }
    }
    let may_be = |st: &Straight, work: &mut Work| -> Result<bool, TooComplex> {
        let len = (st.b.0 - st.a.0).hypot(st.b.1 - st.a.1);
        let along = ((st.b.0 - st.a.0) / len, (st.b.1 - st.a.1) / len);
        let level = |w: &(f32, f32)| {
            tally!(levels, 1);
            (w.0 * along.1 - w.1 * along.0).abs() < 0.02
        };
        Ok(st.width <= 0.15 * biggest && len > 0.0 && any_of(&ways, work, level)?)
    };
    // Each band that may be a line's, with its object's index (the objects are in the order
    // of their first operators).
    let mut bands: Vec<(usize, &Painted)> = Vec::new();
    for p in painted.iter().filter(|p| p.colour.is_some()) {
        if may_be(&p.band, work)? {
            if let Ok(i) = objs.binary_search_by_key(&p.first, |o| o.first) {
                bands.push((i, p));
            }
        }
    }
    if bands.is_empty() {
        return Ok(Vec::new());
    }
    let mut by_line = Grid::new(
        (lines.iter().enumerate()).map(|(k, l)| (k, grown(objs[l.obj].bbox, 0.5 * l.size))),
        work,
    )?;
    let mut straight_of: Option<std::collections::HashMap<usize, Straight>> = None;
    let mut laid: Vec<Option<Option<Laid>>> = (0..lines.len()).map(|_| None).collect();
    let mut others: Option<Grid> = None;
    let mut form_boxes: Option<Vec<[f32; 4]>> = None;
    let mut found: Vec<(usize, Decoration)> = Vec::new();
    let mut near = Vec::new();
    for &(pi, p) in &bands {
        let (st, ink) = (p.band, &p.ink);
        let Some(colour) = p.colour else { continue };
        let bb = grown(mx::aabb([st.a, st.b]), st.width / 2.0);
        let mut best: Option<(usize, Fit)> = None;
        by_line.near_into(bb, &mut near, work)?;
        for &k in &near {
            if !lines[k].may_take(&st) {
                continue;
            }
            let o = &objs[lines[k].obj];
            let l = laid[k].get_or_insert_with(|| Laid::of(o.text.as_ref()?, o.ctm));
            let Some(l) = l.as_ref() else { continue };
            work.take(l.chars.len() as u64)?;
            if let Some(f) = fits(l, &st, colour) {
                // The closest, or of two as close the first line: whatever order the grid
                // gave them in.
                let closer = |(bk, b): &(usize, Fit)| f.off.total_cmp(&b.off).then(k.cmp(bk));
                if best.as_ref().is_none_or(|b| closer(b).is_lt()) {
                    best = Some((k, f));
                }
            }
        }
        let Some((k, f)) = best else { continue };
        let line = &lines[k].obj;
        let l = laid[k].as_ref().and_then(Option::as_ref).expect("laid out");
        if others.is_none() {
            others = Some(Grid::new(
                (objs.iter().enumerate())
                    .filter(|(_, o)| o.kind != ObjectKind::Text)
                    .map(|(i, o)| (i, o.bbox)),
                work,
            )?);
        }
        let others = others.as_mut().expect("built");
        let straight_of =
            straight_of.get_or_insert_with(|| bands.iter().map(|(i, p)| (*i, p.band)).collect());
        let form_boxes = form_boxes.get_or_insert_with(|| {
            (form_text.iter())
                .filter(|t| !t.hidden)
                .map(text_box)
                .collect()
        });
        let s = f.s;
        let d = (0.05 * s).max(0.5);
        // Across the band, where it reaches (up from the baseline).
        let (vb, vt) = (f.v - st.width / 2.0, f.v + st.width / 2.0);
        let rise = l.rise[f.a];
        let (lo, hi) = (vb + rise, vt + rise);
        let n = l.chars.len();
        let text_ends = (
            l.edges[0].0.min(l.edges[n - 1].1),
            l.edges[0].0.max(l.edges[n - 1].1),
        );
        // Another drawing meeting either end, but a band under this line's text, or a fill
        // reaching past both ends behind it.
        let ends = [st.a, st.b].map(|p| grown([p.0, p.1, p.0, p.1], d + st.width / 2.0));
        let joined = others.near(grown(bb, d + st.width), work)?;
        let joined = joined.into_iter().any(|q| {
            let o = &objs[q];
            if q == pi || !ends.iter().any(|e| meets(*e, o.bbox)) {
                return false;
            }
            let under = straight_of.get(&q).is_some_and(|t| {
                let (ua, va) = l.frame.at(t.a);
                let (ub, vb2) = l.frame.at(t.b);
                t.width <= 0.15 * s
                    && ua.min(ub) >= text_ends.0 - d
                    && ua.max(ub) <= text_ends.1 + d
                    && (va - vb2).abs() <= 0.05 * s
            });
            let (along, up) = l.frame.span_of(o.bbox);
            let behind = along[0] <= f.along.0 - d
                && along[1] >= f.along.1 + d
                && up[0] <= lo
                && up[1] >= hi;
            !under && !behind
        });
        if joined {
            continue;
        }
        // Other text: its box over the band, or centred under it within its ends.
        work.take(form_boxes.len() as u64)?;
        let other_boxes = (by_line.near(grown(bb, 0.6 * s + d), work)?.into_iter())
            .filter(|&j| j != k)
            .map(|j| objs[lines[j].obj].bbox)
            .chain(form_boxes.iter().copied().filter(|b| {
                tally!(forms, 1);
                meets(*b, grown(bb, 0.6 * s + d))
            }));
        let mid = (f.along.0 + f.along.1) / 2.0;
        let crowded = other_boxes.into_iter().any(|b| {
            let (along, up) = l.frame.span_of(b);
            let across = along[1].min(f.along.1) - along[0].max(f.along.0);
            let over = across > 0.1 * s && up[1].min(hi) - up[0].max(lo) > 0.0;
            // A denominator is narrower than its bar or right under it; the next line of a
            // justified paragraph, as wide as its underline, is neither.
            let narrower = (f.along.1 - f.along.0) - (along[1] - along[0]) >= 0.05 * s;
            let under = up[1] <= lo + 0.05 * s
                && up[1] >= lo - 0.6 * s
                && along[0] >= f.along.0 - 0.15 * s
                && along[1] <= f.along.1 + 0.15 * s
                && ((along[0] + along[1]) / 2.0 - mid).abs() <= 0.2 * s
                && (narrower || up[1] >= lo - 0.1 * s);
            over || under
        });
        if crowded {
            continue;
        }
        // Bands of its thickness near it, level with this line and along the band: three or
        // more evenly spaced are a staff's lines.
        let like_it = |t: &Straight| {
            let ((ua, va), (ub, vb)) = (l.frame.at(t.a), l.frame.at(t.b));
            let along = ua.max(ub).min(f.along.1) - ua.min(ub).max(f.along.0);
            let alike = (t.width - st.width).abs() <= 0.1 * st.width.max(t.width) + 0.01;
            (alike && (va - vb).abs() <= 0.05 * s && along > 0.0).then_some((va + vb) / 2.0)
        };
        let mut like = (others.near(grown(bb, 2.0 * s + d), work)?.into_iter())
            .filter(|&q| q != pi)
            .filter_map(|q| straight_of.get(&q).and_then(like_it))
            .collect();
        let own = (l.frame.at(st.a).1 + l.frame.at(st.b).1) / 2.0;
        if evenly_spaced(&mut like, own, st.width, s) {
            continue;
        }
        let look = Look {
            bands: vec![(f.v / s, st.width / s)],
            ink: ink.paint(ops),
            colour,
        };
        work.take(found.len() as u64)?;
        match found.iter_mut().find(|(at, dd)| {
            tally!(found, 1);
            at == line && dd.kind == f.kind && dd.chars == (f.a, f.b)
        }) {
            // Highest first, a band as high as one already there after it.
            Some((_, dd)) => {
                work.take(dd.look.bands.len() as u64)?;
                tally!(merged, dd.look.bands.len());
                let band = look.bands[0];
                let at = (dd.look.bands).partition_point(|x| x.0.total_cmp(&band.0).is_ge());
                dd.look.bands.insert(at, band);
                dd.paths.push(pi);
            }
            None => found.push((
                *line,
                Decoration {
                    kind: f.kind,
                    chars: (f.a, f.b),
                    look,
                    paths: vec![pi],
                },
            )),
        }
    }
    found.sort_by_key(|(line, d)| (*line, d.chars.0));
    tally!(steps, work.taken() - start);
    Ok(found)
}

/// A font's underline and strike-out, where LimarePDF draws its own ([`decoration_lines`],
/// [`crate::LineFont::lines`]): each one's centre above the baseline and its thickness, in
/// ems.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontLines {
    pub under: (f32, f32),
    pub strike: (f32, f32),
}

impl FontLines {
    /// The underline of the standard fonts (each AFM file's: its centre 100 below the
    /// baseline, 50 thick, per 1,000), and a strike-out for a font that gives none.
    pub const STANDARD: FontLines = FontLines {
        under: (-0.1, 0.05),
        strike: (0.26, 0.05),
    };

    /// The band of `kind`: its centre above the baseline and its thickness, in ems.
    pub fn of(&self, kind: DecoKind) -> (f32, f32) {
        match kind {
            DecoKind::Underline => self.under,
            DecoKind::Strike => self.strike,
        }
    }
}

const STANDARD: FontLines = FontLines::STANDARD;

/// A band's centre above the baseline and its thickness, in ems.
type Band = (f32, f32);

/// What a font program says of its underline (`post`) and strike-out (`OS/2`), each given
/// by its top, the OpenType way.
fn program_lines(data: &[u8]) -> (Option<Band>, Option<Band>) {
    face_lines(data, 0)
}

/// [`program_lines`] for the face at `index` in font file `data` (a collection's).
fn face_lines(data: &[u8], index: u32) -> (Option<Band>, Option<Band>) {
    let Ok(f) = FontRef::from_index(data, index) else {
        return (None, None);
    };
    let em = f
        .head()
        .map_or(1000.0, |h| h.units_per_em() as f32)
        .max(1.0);
    let under = f.post().ok().and_then(|p| {
        let top = p.underline_position().to_i16() as f32;
        let thick = p.underline_thickness().to_i16() as f32;
        (thick > 0.0).then(|| ((top - thick / 2.0) / em, thick / em))
    });
    let strike = f.os2().ok().and_then(|o| {
        let (top, thick) = (o.y_strikeout_position() as f32, o.y_strikeout_size() as f32);
        (thick > 0.0 && top > 0.0).then(|| ((top - thick / 2.0) / em, thick / em))
    });
    (under, strike)
}

/// The underline and strike-out of font dictionary `font`: its embedded program's (a
/// TrueType or OpenType one), else the standard font's nearest it, with the strike-out of
/// that font's Liberation look-alike.
pub(crate) fn font_lines(doc: &Document, font: &Dictionary) -> FontLines {
    lines_of(doc, font, None)
}

/// [`font_lines`], its embedded TrueType or OpenType program `program` when that has been
/// read already.
pub(crate) fn lines_of(doc: &Document, font: &Dictionary, program: Option<&[u8]>) -> FontLines {
    let get = |d: &Dictionary, key: &[u8]| -> Option<Object> {
        Some(doc.dereference(d.get(key).ok()?).ok()?.1.clone())
    };
    let dict = |o: Option<Object>| match o {
        Some(Object::Dictionary(d)) => Some(d),
        _ => None,
    };
    let base = (get(font, b"BaseFont").and_then(|o| o.as_name().ok().map(|n| n.to_vec())))
        .map(|n| String::from_utf8_lossy(&n).into_owned())
        .unwrap_or_default();
    let read = || {
        // A Type 0 font's program is its descendant's.
        let descendant = (get(font, b"DescendantFonts").and_then(|o| o.as_array().ok().cloned()))
            .and_then(|a| dict(doc.dereference(a.first()?).ok().map(|(_, o)| o.clone())));
        let fd = dict(get(descendant.as_ref().unwrap_or(font), b"FontDescriptor"))?;
        [&b"FontFile2"[..], b"FontFile3"].iter().find_map(|&k| {
            let (_, o) = doc.dereference(fd.get(k).ok()?).ok()?;
            let s = o.as_stream().ok()?;
            let sub = s.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok());
            match (k, sub) {
                (b"FontFile3", Some(sub)) if sub != b"OpenType" => None,
                _ => s.get_plain_content_with_limit(MAX_PROGRAM).ok(),
            }
        })
    };
    let (under, strike) = match program {
        Some(data) => program_lines(data),
        None => read().as_deref().map_or((None, None), program_lines),
    };
    // The nearest standard font by name, which reads no installed fonts (a line's fonts are
    // read as it is selected).
    lines_given(under, strike, FontChoice::from_name(&base))
}

/// A font's underline and strike-out from what its program says of them (`under`,
/// `strike`): the standard fonts' underline where it says none, and the strike-out of its
/// look-alike `alike`'s Liberation face, else [`FontLines::STANDARD`]'s. What LimarePDF
/// writes ([`font_lines`]) and what it draws for text to be written ([`SetIn::lines`]) are
/// both given so.
fn lines_given(under: Option<Band>, strike: Option<Band>, alike: FontChoice) -> FontLines {
    FontLines {
        under: under.unwrap_or(STANDARD.under),
        strike: (strike.or_else(|| alike_strike(alike))).unwrap_or(STANDARD.strike),
    }
}

/// The strike-out of the Liberation look-alike of standard font `f`.
fn alike_strike(f: FontChoice) -> Option<Band> {
    let face = crate::embed::liberation(f);
    (crate::embed::bundled_face(face).map(program_lines)).and_then(|(_, s)| s)
}

impl SetIn {
    /// The underline and strike-out LimarePDF draws for text set so, as for the font it is
    /// written in ([`font_lines`]): a standard font written as itself, nothing embedded, has
    /// its AFM underline and its Liberation look-alike's strike-out; an embedded face (a
    /// standard font's bundled one, or an installed family's) its own underline (`post`)
    /// and the strike-out of the standard font its name is nearest, as its subset keeps no
    /// `OS/2` table. Read once for each.
    pub fn lines(self) -> FontLines {
        static READ: std::sync::Mutex<Vec<(SetIn, FontLines)>> = std::sync::Mutex::new(Vec::new());
        let mut read = READ.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, lines)) = read.iter().find(|(s, _)| *s == self) {
            return *lines;
        }
        let face = match self {
            SetIn::Standard(_) => None,
            SetIn::Face(f) => crate::embed::Face::of(f).ok(),
        };
        let lines = match face {
            Some(face) => {
                let (data, index) = face.file();
                let alike = FontChoice::from_name(face.ps());
                lines_given(face_lines(data, index).0, None, alike)
            }
            None => lines_given(None, None, self.choice()),
        };
        read.push((self, lines));
        lines
    }
}

/// The underline and strike-out LimarePDF draws for text `f` writes all of
/// ([`SetIn::of`]): a standard font's as itself, an installed family's face's.
pub fn decoration_lines(f: FontChoice) -> FontLines {
    SetIn::of(f).lines()
}

/// The decorations of a line's new text carried from its old ones `old` by where each new
/// character came from (`origins`: its place in the line as read, `None` for one typed): a
/// character keeps its decoration, one typed takes that of the character before it (at the
/// start, of the first one that came from the line), as typing carries on a word
/// processor's underline.
pub(crate) fn carried(old: &[&Decoration], origins: &[Option<usize>]) -> Vec<Decorated> {
    let n = origins.len();
    let mut out = Vec::new();
    for kind in [DecoKind::Underline, DecoKind::Strike] {
        let of: Vec<&Decoration> = old.iter().copied().filter(|d| d.kind == kind).collect();
        if of.is_empty() {
            continue;
        }
        let mut state: Vec<Option<usize>> = (origins.iter())
            .map(|o| o.and_then(|o| of.iter().position(|d| d.chars.0 <= o && o < d.chars.1)))
            .collect();
        let first = origins.iter().position(Option::is_some);
        for j in 0..n {
            if origins[j].is_none() {
                state[j] = match j {
                    0 => first.and_then(|k| state[k]),
                    _ => state[j - 1],
                };
            }
        }
        let mut j = 0;
        while j < n {
            let Some(d) = state[j] else {
                j += 1;
                continue;
            };
            let mut k = j + 1;
            while k < n && state[k] == Some(d) {
                k += 1;
            }
            out.push(Decorated {
                kind,
                chars: (j, k),
                look: Some(of[d].look.clone()),
            });
            j = k;
        }
    }
    out
}

/// Each of `want`'s characters' place among `got`'s, by their longest common run of
/// characters that aren't spaces; `None` for a space or one not found there.
fn align(want: &[char], got: &[char]) -> Vec<Option<usize>> {
    let a: Vec<usize> = (0..want.len())
        .filter(|&i| !want[i].is_whitespace())
        .collect();
    let b: Vec<usize> = (0..got.len())
        .filter(|&j| !got[j].is_whitespace())
        .collect();
    let (n, m) = (a.len(), b.len());
    let mut out = vec![None; want.len()];
    if n * m > 4_000_000 {
        // Too long to compare every pair: in order, each where it is next found.
        let mut j = 0;
        for &i in &a {
            while j < m && got[b[j]] != want[i] {
                j += 1;
            }
            if j == m {
                break;
            }
            out[i] = Some(b[j]);
            j += 1;
        }
        return out;
    }
    // How long a common run the characters from `i` and from `j` on have.
    let w = m + 1;
    let mut run = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            run[i * w + j] = match want[a[i]] == got[b[j]] {
                true => run[(i + 1) * w + j + 1] + 1,
                false => run[(i + 1) * w + j].max(run[i * w + j + 1]),
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if want[a[i]] == got[b[j]] {
            out[a[i]] = Some(b[j]);
            (i, j) = (i + 1, j + 1);
        } else if run[(i + 1) * w + j] >= run[i * w + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// The operators drawing bands `bands` (each one's centre above the baseline and thickness,
/// in ems of `size`) in `frame` from `along.0` to `along.1`, raised by `rise`, in `ink`: each
/// band its own filled rectangle, as the reading rule reads one.
fn band_ops(
    frame: &Frame,
    along: (f32, f32),
    bands: &[(f32, f32)],
    size: f32,
    rise: f32,
    ink: &Paint,
) -> Vec<Operation> {
    let mut out = vec![op("q", vec![])];
    out.extend(ink.ops());
    out.push(op("cm", frame.matrix().iter().map(|&x| r(x)).collect()));
    for &(c, t) in bands {
        let y = rise + (c - t / 2.0) * size;
        let rect = [along.0, y, along.1 - along.0, t * size];
        out.push(op("re", rect.iter().map(|&x| r(x)).collect()));
        out.push(op("f", vec![]));
    }
    out.push(op("Q", vec![]));
    out
}

/// A text line written, read back: its characters as laid out, and which of them were
/// written (those of the shows at the place written), in order.
struct Back {
    lines: Vec<Laid>,
    chars: Vec<(usize, usize)>,
}

impl Back {
    /// The characters shown at `place` in `back`, the page read back.
    fn at(back: &Parsed, place: Check) -> Back {
        let shown = |o: usize| match place {
            Check::InPlace(i) => o == i,
            Check::OnTop(from, to) | Check::Within(from, to) => (from..to).contains(&o),
        };
        let mut out = Back {
            lines: Vec::new(),
            chars: Vec::new(),
        };
        for o in &back.objs {
            let Some(t) = &o.text else { continue };
            if !t.runs.iter().any(|run| shown(run.op)) {
                continue;
            }
            let Some(laid) = Laid::of(t, o.ctm) else {
                continue;
            };
            let k = out.lines.len();
            for (run, piece) in t.runs.iter().zip(&t.pieces) {
                if shown(run.op) {
                    out.chars
                        .extend((piece.chars.0..piece.chars.1).map(|c| (k, c)));
                }
            }
            out.lines.push(laid);
        }
        out
    }

    fn char(&self, i: usize) -> char {
        let (k, c) = self.chars[i];
        self.lines[k].chars[c]
    }

    fn edges(&self, i: usize) -> (f32, f32) {
        let (k, c) = self.chars[i];
        self.lines[k].edges[c]
    }

    /// Where character `i` starts (or ends) on the page.
    fn point(&self, i: usize, end: bool) -> (f32, f32) {
        let (k, c) = self.chars[i];
        let l = &self.lines[k];
        let u = if end { l.edges[c].1 } else { l.edges[c].0 };
        apply(l.frame.matrix(), u, 0.0)
    }
}

/// A line's text as edited, for its decorations ([`Pdf::decoration_ops`]): the text, the
/// decorations wanted over it, its old ones (read on the line as `was` laid it out), and
/// where each of its characters was in the line as read (`None` for one typed).
#[derive(Clone, Copy)]
pub(crate) struct Redraw<'a> {
    pub text: &'a str,
    pub want: &'a [Decorated],
    pub old: &'a [&'a Decoration],
    pub was: Option<&'a Laid>,
    pub origins: &'a [Option<usize>],
}

impl<'a> Redraw<'a> {
    /// New text's: nothing old.
    pub(crate) fn new(text: &'a str, want: &'a [Decorated]) -> Self {
        Redraw {
            text,
            want,
            old: &[],
            was: None,
            origins: &[],
        }
    }
}

impl Pdf {
    /// What a line's decorations become now that its text is written at `place` in `back`
    /// (the page read back): the operators drawing those wanted (over the text's
    /// characters), and for each of its old ones whether it stays as drawn: when a
    /// decoration wanted is it, in its look, over the same characters where they were.
    pub(crate) fn decoration_ops(
        &self,
        page: ObjectId,
        back: &Parsed,
        place: Check,
        r: &Redraw,
    ) -> (Vec<Operation>, Vec<bool>) {
        let Redraw {
            text,
            want,
            old,
            was,
            origins,
        } = *r;
        let mut kept = vec![false; old.len()];
        let written = Back::at(back, place);
        let chars: Vec<char> = text.chars().collect();
        let got: Vec<char> = (0..written.chars.len()).map(|i| written.char(i)).collect();
        let map = align(&chars, &got);
        let (Some(first), Some(last)) = (
            chars.iter().position(|c| !c.is_whitespace()),
            chars.iter().rposition(|c| !c.is_whitespace()),
        ) else {
            return (Vec::new(), kept);
        };
        let fonts = self.font_resources(page);
        let mut lines_of: std::collections::HashMap<Vec<u8>, FontLines> = Default::default();
        let mut ops = Vec::new();
        'want: for w in want {
            // Spaces at either end of the line are left bare, as word processors leave them.
            let (a, b) = (
                w.chars.0.max(first),
                w.chars.1.min(last + 1).min(chars.len()),
            );
            let placed: Vec<(usize, usize)> = (a..b)
                .filter_map(|i| Some((i, map.get(i).copied().flatten()?)))
                .collect();
            let (Some(&(i0, g0)), Some(&(i1, g1))) = (placed.first(), placed.last()) else {
                continue;
            };
            // A decoration as it was, over the same characters, where they were.
            for (k, d) in old.iter().enumerate() {
                let same = !kept[k]
                    && d.kind == w.kind
                    && w.look.as_ref() == Some(&d.look)
                    && w.chars.1 - w.chars.0 == d.chars.1 - d.chars.0
                    && (0..w.chars.1 - w.chars.0)
                        .all(|j| origins.get(w.chars.0 + j) == Some(&Some(d.chars.0 + j)));
                let near = |p: (f32, f32), q: (f32, f32)| (p.0 - q.0).hypot(p.1 - q.1) < 0.05;
                let stays = same
                    && was.is_some_and(|l| {
                        let m = l.frame.matrix();
                        let (s, e) = (l.edges[d.chars.0].0, l.edges[d.chars.1 - 1].1);
                        a == w.chars.0
                            && b == w.chars.1
                            && near(apply(m, s, 0.0), written.point(g0, false))
                            && near(apply(m, e, 0.0), written.point(g1, true))
                    });
                if stays {
                    kept[k] = true;
                    continue 'want;
                }
            }
            // From the first character to the last, over spaces at its ends: from where the
            // character before ends, to where the one after starts.
            let before = (0..a).rev().find_map(|i| map[i]);
            let after = (b..chars.len()).find_map(|i| map[i]);
            let start = match (i0 > a, before) {
                (true, Some(p)) if written.chars[p].0 == written.chars[g0].0 => written.edges(p).1,
                _ => written.edges(g0).0,
            };
            let end = match (i1 + 1 < b, after) {
                (true, Some(n)) if written.chars[n].0 == written.chars[g1].0 => written.edges(n).0,
                _ => written.edges(g1).1,
            };
            // One stretch a part of a written line, each at its own size and in its font.
            let mut groups: Vec<(usize, usize)> = Vec::new();
            for &(_, g) in &placed {
                let key = |g: usize| {
                    let (k, c) = written.chars[g];
                    (k, written.lines[k].part[c])
                };
                match groups.last_mut() {
                    Some((_, to)) if key(*to) == key(g) => *to = g,
                    _ => groups.push((g, g)),
                }
            }
            for (n, &(from, to)) in groups.iter().enumerate() {
                let (k, c) = written.chars[from];
                let l = &written.lines[k];
                let s = (from..=to)
                    .filter(|&g| written.chars[g].0 == k)
                    .fold(0.0f32, |m, g| m.max(l.size[written.chars[g].1]));
                let next = groups.get(n + 1).map(|&(f, _)| f);
                let along = (
                    if n == 0 { start } else { written.edges(from).0 },
                    match next {
                        None => end,
                        Some(f) if written.chars[f].0 == k => written.edges(f).0,
                        Some(_) => written.edges(to).1,
                    },
                );
                if along.1 <= along.0 {
                    continue;
                }
                let (bands, ink) = match &w.look {
                    Some(look) => (look.bands.clone(), look.ink.clone()),
                    None => {
                        let name = &l.fonts[l.part[c]];
                        let lines = *lines_of.entry(name.clone()).or_insert_with(|| {
                            (fonts.get(name).ok())
                                .and_then(|o| self.doc.dereference(o).ok())
                                .and_then(|(_, o)| o.as_dict().ok())
                                .map_or(STANDARD, |d| font_lines(&self.doc, d))
                        });
                        (vec![lines.of(w.kind)], l.paint(c).clone())
                    }
                };
                ops.extend(band_ops(&l.frame, along, &bands, s, l.rise[c], &ink));
            }
        }
        (ops, kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box `w` by `h` cells of 64 from cell (0, 0).
    fn of_cells(w: u32, h: u32) -> [f32; 4] {
        [0.0, 0.0, w as f32 * 64.0 - 1.0, h as f32 * 64.0 - 1.0]
    }

    /// The cells [`each_cell`] gives `b` (cells of 64), the passes it made, and the steps it
    /// took, with no limit.
    fn walked(b: [f32; 4]) -> (Vec<(i32, i32)>, u64, u64) {
        let mut work = Work::new("walking", u64::MAX);
        let mut cells = Vec::new();
        let passes = each_cell(64.0, b, &mut work, |c| cells.push(c)).unwrap();
        (cells, passes, work.taken())
    }

    /// The cells on the edges of a box `w` by `h` cells, from (0, 0), each once: worked out
    /// apart from [`each_cell`].
    fn edges(w: i32, h: i32) -> Vec<(i32, i32)> {
        let mut out: Vec<_> = (0..w)
            .flat_map(|x| (0..h).map(move |y| (x, y)))
            .filter(|&(x, y)| x == 0 || y == 0 || x == w - 1 || y == h - 1)
            .collect();
        out.sort_unstable();
        out
    }

    /// A box over 5,000 by 5,000 cells goes through its edges only (H-08: it went through
    /// all 25 million), each cell once, a step each.
    #[test]
    fn a_box_over_many_cells_goes_through_its_edges_only() {
        let (mut cells, passes, taken) = walked(of_cells(5000, 5000));
        assert!(passes <= 4 * 5001, "{passes} passes");
        assert_eq!(cells.len() as u64, passes);
        assert_eq!(taken, passes);
        cells.sort_unstable();
        assert_eq!(cells, edges(5000, 5000));
        // Just over the 4,096 cells a box goes through whole, and strips, all edge.
        for (w, h) in [(65, 64), (1, 5000), (2, 5000)] {
            let (mut cells, passes, taken) = walked(of_cells(w as u32, h as u32));
            cells.sort_unstable();
            assert_eq!(cells, edges(w, h), "{w} by {h}");
            assert_eq!((passes, taken), (cells.len() as u64, cells.len() as u64));
        }
    }

    /// A box of up to 4,096 cells touches each, as it always did.
    #[test]
    fn a_box_of_few_cells_touches_every_one() {
        let (mut cells, passes, taken) = walked(of_cells(64, 64));
        cells.sort_unstable();
        let all: Vec<_> = (0..64).flat_map(|x| (0..64).map(move |y| (x, y))).collect();
        assert_eq!(cells, all);
        assert_eq!((passes, taken), (4096, 4096));
    }

    /// A box whose edges cross [`MOST_CELLS`] touches none and takes no steps; one at them
    /// touches them all.
    #[test]
    fn a_box_past_the_most_cells_touches_none() {
        // 2 × (8,193 + 8,193) − 4 edge cells, and a strip.
        for (w, h) in [(8193, 8193), (1, MOST_CELLS as u32)] {
            let (cells, passes, taken) = walked(of_cells(w, h));
            assert_eq!(
                (cells.len() as u64, passes, taken),
                (MOST_CELLS, MOST_CELLS, MOST_CELLS)
            );
        }
        for (w, h) in [(8193, 8194), (1, MOST_CELLS as u32 + 1)] {
            let (cells, passes, taken) = walked(of_cells(w, h));
            assert_eq!((cells.len(), passes, taken), (0, 0, 0), "{w} by {h}");
        }
    }

    /// A box as big as can be (its cells kept to ±10⁶, about four million million of them)
    /// is passed over at once.
    #[test]
    fn a_box_as_big_as_can_be_is_passed_over_at_once() {
        let (cells, passes, taken) = walked([-1e30, -1e30, 1e30, 1e30]);
        assert_eq!((cells.len(), passes, taken), (0, 0, 0));
    }

    /// A box's cells are taken from the budget before any is touched: one step short, the
    /// walk is refused and touches none.
    #[test]
    fn a_box_short_of_steps_touches_no_cell() {
        let edge = edges(65, 64).len() as u64;
        let mut work = Work::new("walking", edge - 1);
        let mut touched = 0;
        let e = each_cell(64.0, of_cells(65, 64), &mut work, |_| touched += 1).unwrap_err();
        assert_eq!(e.limit, edge - 1);
        assert_eq!((touched, work.taken()), (0, 0));
        let mut work = Work::new("walking", edge);
        assert_eq!(
            each_cell(64.0, of_cells(65, 64), &mut work, |_| ()),
            Ok(edge)
        );
    }

    /// [`any_of`] takes a step before each item it looks at and none past the first that
    /// holds; short of steps, it looks at no item it hasn't taken one for.
    #[test]
    fn any_of_takes_a_step_for_each_item_looked_at() {
        let mut work = Work::new("looking", 10);
        let mut looked = 0;
        let got = any_of(&[1, 2, 3, 4], &mut work, |&x| {
            looked += 1;
            x == 2
        });
        assert_eq!((got, looked, work.taken()), (Ok(true), 2, 2));
        let mut work = Work::new("looking", 2);
        let mut looked = 0;
        let e = any_of(&[1, 2, 3], &mut work, |_| {
            looked += 1;
            false
        });
        assert_eq!((e.unwrap_err().limit, looked, work.taken()), (2, 2, 2));
    }

    /// Three or more bands evenly spaced are a staff's lines, a band at its foot, middle or
    /// head, a line drawn twice among them or not; two alone, uneven gaps, gaps of an em,
    /// bands touching, and a band past the staff aren't (PB-101).
    #[test]
    fn three_or_more_bands_evenly_spaced_are_a_staff() {
        let staff = |vs: &[f32], v: f32| evenly_spaced(&mut vs.to_vec(), v, 0.45, 12.0);
        assert!(staff(&[2.2, 4.7, 7.2, 9.7], -0.3));
        assert!(staff(&[-0.3, 2.2, 7.2, 9.7], 4.7));
        assert!(staff(&[-0.3, 2.2, 4.7, 7.2], 9.7));
        assert!(staff(&[2.2, 2.2, 4.7], -0.3));
        assert!(!staff(&[2.2], -0.3));
        assert!(!staff(&[2.2, 5.2], -0.3));
        assert!(!staff(&[12.0, 24.0], 0.0));
        assert!(!staff(&[0.4, 0.8], 0.0));
        assert!(!staff(&[-0.3, 2.2, 4.7], 12.0));
    }

    const LINE: &str = "This has an underlined phrase in it.";

    /// How many of each thing [`read`] meets on the pages below: one and more, to many.
    const DENSITIES: [usize; 4] = [1, 4, 16, 64];

    /// A US Letter page drawing `content`, with Helvetica as `/F1`, and form `/X1` drawing
    /// `form` (in the page's lower half) when there is one.
    fn page(content: &str, form: Option<&str>) -> Pdf {
        use lopdf::{dictionary, Stream};
        let mut p = Pdf::new();
        p.insert_blank(0, 612.0, 792.0).unwrap();
        let f1 = p.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let mut res = dictionary! { "Font" => dictionary! { "F1" => f1 } };
        if let Some(form) = form {
            let x1 = p.doc.add_object(Stream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Form",
                    "BBox" => vec![0.into(), 0.into(), 612.into(), 310.into()],
                    "Resources" => dictionary! { "Font" => dictionary! { "F1" => f1 } },
                },
                form.as_bytes().to_vec(),
            ));
            res.set("XObject", dictionary! { "X1" => x1 });
        }
        let s = (p.doc).add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
        let page = p.page_ids()[0];
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", s);
        d.set("Resources", res);
        p
    }

    /// [`LINE`] in Helvetica `size` at (72, `y`).
    fn line(size: f32, y: f32) -> String {
        format!("BT /F1 {size} Tf 72 {y} Td ({LINE}) Tj ET\n")
    }

    /// Where [`LINE`] in Helvetica `size` starts and ends along the page, as core reads it.
    fn ends(size: f32) -> (f32, f32) {
        let objs = page(&line(size, 700.0), None).page_objects(0).unwrap();
        let l = objs.iter().find(|o| o.kind == ObjectKind::Text).unwrap();
        let x = l.baseline.unwrap().0;
        (x + l.stops[0], x + l.stops[LINE.chars().count()])
    }

    /// The decorations [`read`] finds on `p`'s page, and what it did finding them.
    fn tallied(p: &Pdf) -> (Vec<(usize, Decoration)>, Tally) {
        TALLY.with(|t| t.set(Tally::default()));
        let parsed = p.parse_page(0).unwrap();
        (parsed.decorations, TALLY.with(|t| t.get()))
    }

    /// The steps [`read`] took cover all it did that can grow faster than the page (CP-05).
    fn covered(t: Tally, what: &str) {
        let did = t.ways + t.levels + t.occupants + t.chars + t.forms + t.found + t.merged;
        assert!(
            t.steps >= did,
            "{what}: {} steps for {did} done, {t:?}",
            t.steps
        );
    }

    /// `d` filled bands under the whole of [`LINE`] at 12 at (72, 700), from 4.6 below its
    /// baseline to 0.4 below, drawn lowest first: the gaps between them two and three parts
    /// in turn, so they are not a staff's lines ([`evenly_spaced`]).
    fn bands_under(d: usize) -> String {
        let (x0, x1) = ends(12.0);
        let mut c = line(12.0, 700.0) + "0 0 0 rg\n";
        let at = |i: usize| (5 * (i / 2) + 2 * (i % 2)) as f32;
        for i in 0..d {
            let v = match d {
                1 => -1.4,
                _ => -4.6 + 4.2 * at(i) / at(d - 1),
            };
            c += &format!("{x0} {} {} 0.06 re f\n", 700.0 + v - 0.03, x1 - x0);
        }
        c
    }

    /// Lines drawn over one another, a band under each: every band is tried against every
    /// line, character by character, and those characters are steps. Each line's text is
    /// over the others' bands, so none is an underline but the one line's alone.
    #[test]
    fn lines_drawn_over_one_another_take_steps_for_each_character_tried() {
        let n = LINE.chars().count() as u64;
        let (x0, x1) = ends(12.0);
        let band = format!(
            "q 1 0 0 1 {x0} 700 cm 0.7 w 0 0 0 RG 0 -1.4 m {} -1.4 l S Q\n",
            x1 - x0
        );
        for d in DENSITIES {
            let p = page(&(line(12.0, 700.0).repeat(d) + &band.repeat(d)), None);
            let (found, t) = tallied(&p);
            assert_eq!(found.len(), usize::from(d == 1), "{d} lines");
            let d = d as u64;
            assert_eq!(t.chars, d * d * n, "{d} lines: {t:?}");
            covered(t, &format!("{d} lines"));
        }
    }

    /// Bands drawn under one line, all over the same characters: each joining the
    /// decoration is set among those already in it, and each is looked at for meeting the
    /// others' ends, and those are steps. They are one underline of all the bands, the
    /// highest first.
    #[test]
    fn bands_under_one_line_take_steps_for_each_band_they_join() {
        for d in DENSITIES {
            let (found, t) = tallied(&page(&bands_under(d), None));
            assert_eq!(found.len(), 1, "{d} bands");
            let bands = &found[0].1.look.bands;
            assert_eq!(bands.len(), d, "{d} bands");
            assert!(bands.windows(2).all(|w| w[0].0 > w[1].0), "{bands:?}");
            let d = d as u64;
            assert_eq!(t.merged, d * (d - 1) / 2, "{d} bands: {t:?}");
            assert!(t.occupants >= d * d, "{d} bands: {t:?}");
            covered(t, &format!("{d} bands"));
        }
    }

    /// Lines running each its own way, and a band no line's: each way is set against the
    /// ways before it, and those are steps.
    #[test]
    fn lines_running_many_ways_take_steps_for_each_way_compared() {
        for d in DENSITIES {
            let mut c = String::new();
            for i in 0..d {
                let (s, co) = (i as f32).to_radians().sin_cos();
                c += &format!(
                    "BT /F1 12 Tf {co} {s} {} {co} 72 700 Tm ({LINE}) Tj ET\n",
                    -s
                );
            }
            c += "0 0 0 rg 72 100 100 0.5 re f\n";
            let (found, t) = tallied(&page(&c, None));
            assert!(found.is_empty(), "{d} ways");
            let d = d as u64;
            assert_eq!(t.ways, d * (d - 1) / 2, "{d} ways: {t:?}");
            covered(t, &format!("{d} ways"));
        }
    }

    /// Bands under one line, and a form's lines far from them: each form line is looked at
    /// for each band, and those are steps. The bands are one underline, as without the form.
    #[test]
    fn a_form_s_lines_take_steps_for_each_band_looked_at() {
        for d in DENSITIES {
            let form: String = (0..d).map(|i| line(4.0, 300.0 - 4.0 * i as f32)).collect();
            let c = bands_under(d) + "q /X1 Do Q\n";
            let (found, t) = tallied(&page(&c, Some(&form)));
            assert_eq!(found.len(), 1, "{d} form lines");
            assert_eq!(found[0].1.look.bands.len(), d, "{d} form lines");
            let d = d as u64;
            assert_eq!(t.forms, d * d, "{d} form lines: {t:?}");
            covered(t, &format!("{d} form lines"));
        }
    }

    /// Lines one under another, each underlined: each underline is set against those found
    /// before it, and those are steps. Each line has its own.
    #[test]
    fn underlined_lines_take_steps_for_each_underline_found_before() {
        let (x0, x1) = ends(6.0);
        for d in DENSITIES {
            let mut c = String::from("0 0 0 rg\n");
            for i in 0..d {
                let y = 776.0 - 12.0 * i as f32;
                c += &line(6.0, y);
                c += &format!("{x0} {} {} 0.35 re f\n", y - 0.875, x1 - x0);
            }
            let (found, t) = tallied(&page(&c, None));
            assert_eq!(found.len(), d, "{d} lines");
            let d = d as u64;
            assert_eq!(t.found, d * (d - 1) / 2, "{d} lines: {t:?}");
            covered(t, &format!("{d} lines"));
        }
    }
}
