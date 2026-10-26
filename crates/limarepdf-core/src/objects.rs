//! A page-content object model: text lines, images, form XObjects and vector paths,
//! recovered from the page's content stream so they can be selected and edited.

use crate::fonts::{Family, FontChoice, FontCtx};
use crate::matrix::{self as mx, apply, mul, M};
use crate::{Pdf, Result};
use anyhow::{anyhow, bail};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Text,
    Image,
    /// A form XObject (a reusable group of content). It can be moved, resized and deleted as a whole.
    Form,
    Path,
}

/// A selectable object on a page. Coordinates are in points on the *displayed* page:
/// origin at the top-left, y growing downwards, rotation and crop box already applied.
#[derive(Clone, Debug)]
pub struct PageObject {
    pub id: usize,
    pub kind: ObjectKind,
    pub bbox: [f32; 4],
    pub text: Option<String>,
    pub font_name: Option<String>,
    pub font_size: Option<f32>,
    pub fill: Option<[f32; 3]>,
    pub stroke: Option<[f32; 3]>,
    pub line_width: Option<f32>,
    pub has_fill: bool,
    pub has_stroke: bool,
    /// True for inline images, which cannot be replaced (only moved, resized or deleted).
    pub inline: bool,
    /// For text: the font can be reused for plain ASCII edits (otherwise edits are re-set in a standard font).
    pub font_reusable: bool,
    /// Invisible text (render mode 3 or 7), such as an OCR layer. It is not drawn, but it
    /// is searched and can be selected and copied in viewers.
    pub hidden: bool,
    /// For text: the left end of the line's baseline, where an edited line is written.
    pub baseline: Option<(f32, f32)>,
    /// For text: the way the line runs on the displayed page, as a unit vector.
    pub dir: Option<(f32, f32)>,
    /// For text: where each character starts along the line, in points from `baseline`
    /// towards `dir`, and where the last one ends: one more than the line's characters.
    pub stops: Vec<f32>,
    /// For text: the line runs left to right across the displayed page, letters upright,
    /// and is not in vertical writing.
    pub upright: bool,
    /// For text: some of the line is in vertical writing (measured as if it ran across).
    pub vertical: bool,
    /// For text: the parts of the line in one style each, in order. `font_name`,
    /// `font_size`, `fill` and `font_reusable` are the first part's.
    pub spans: Vec<TextSpan>,
}

/// A part of a line of text in one style: [`PageObject::spans`].
#[derive(Clone, Debug, PartialEq)]
pub struct TextSpan {
    /// Its characters in the line's text, from and to.
    pub chars: (usize, usize),
    /// The PDF font's name.
    pub font_name: String,
    pub font_size: f32,
    pub fill: Option<[f32; 3]>,
    /// The font can be reused for plain ASCII edits ([`PageObject::font_reusable`]).
    pub font_reusable: bool,
    /// How far the part is raised above the line's baseline, in points (lowered when
    /// below zero), as a superscript is.
    pub rise: f32,
}

impl TextSpan {
    /// Whether two parts of a line look the same: the same font (two subsets of one font
    /// are one), size, colour and rise. Parts can look the same and still be apart, in
    /// two font objects of one font, for one.
    pub fn looks_like(&self, other: &TextSpan) -> bool {
        self.same_font(other)
            && (self.font_size - other.font_size).abs() < 0.01 * self.font_size.max(0.01)
            && self.fill == other.fill
            && (self.rise - other.rise).abs() < 0.01
    }

    /// Whether two parts of a line are in the same font, two subsets of one font being one.
    pub fn same_font(&self, other: &TextSpan) -> bool {
        let untagged = |name: &str| match name.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => {
                rest.to_string()
            }
            _ => name.to_string(),
        };
        untagged(&self.font_name) == untagged(&other.font_name)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Show {
    Tj,
    TJ,
    Quote,
    DQuote { aw: f32, ac: f32 },
}

#[derive(Clone, Debug)]
pub(crate) struct Run {
    pub op: usize,
    pub show: Show,
    /// Text matrix at the start of the show (after any line move of ' and ").
    pub tm: M,
    /// Character and word spacing in points, and horizontal scaling (1 is none), as the
    /// show was drawn with them.
    pub spacing: [f32; 3],
    /// The gap in points before the next show of the line, when it was read as a space.
    pub gap: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct TextData {
    pub runs: Vec<Run>,
    pub tm_end: M,
    pub tlm_end: M,
    pub text: String,
    pub base: String,
    pub size: f32,
    pub origin: (f32, f32),
    pub dir: (f32, f32),
    pub fill: Option<[f32; 3]>,
    pub reuse: bool,
    /// Every show of the line is invisible (see [`PageObject::hidden`]).
    pub hidden: bool,
    /// Some of the line is in vertical writing, which is measured as if it ran across.
    pub vertical: bool,
    /// Where each show's characters are, for locating matches within the line.
    pub pieces: Vec<Piece>,
    /// The parts of the line in one style each ([`PageObject::spans`]).
    pub spans: Vec<Span>,
}

/// Shows `runs.0..runs.1` of a line, its characters `chars.0..chars.1`, in one font, size,
/// fill and rise.
#[derive(Clone, Debug)]
pub(crate) struct Span {
    pub chars: (usize, usize),
    pub runs: (usize, usize),
    /// The font's name in the page's resources.
    pub font: Vec<u8>,
    pub base: String,
    pub reuse: bool,
    pub size: f32,
    pub fill: Option<[f32; 3]>,
    pub rise: f32,
}

/// The characters `chars.0..chars.1` of a line's text, shown by one operator, and the
/// corners of their box in page user space: start-bottom, end-bottom, end-top, start-top.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    pub chars: (usize, usize),
    pub corners: [(f32, f32); 4],
    /// Where each character starts and ends along the box, as fractions of its length
    /// from the start. Empty when not known; the characters are then spread evenly.
    pub spans: Vec<(f32, f32)>,
}

#[derive(Clone, Debug)]
pub(crate) struct PathData {
    pub fill: bool,
    pub stroke: bool,
    pub evenodd: bool,
    pub closed: bool,
    pub fill_c: Option<[f32; 3]>,
    pub stroke_c: Option<[f32; 3]>,
    pub lw: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct Obj {
    pub kind: ObjectKind,
    pub first: usize,
    pub last: usize,
    pub ctm: M,
    /// Bounding box in page user space.
    pub bbox: [f32; 4],
    pub text: Option<TextData>,
    pub path: Option<PathData>,
    pub inline: bool,
}

pub(crate) struct Parsed {
    pub ops: Vec<Operation>,
    pub objs: Vec<Obj>,
    /// Unbalanced `q` operators left open at the end of the stream.
    pub depth_left: usize,
    /// Raw inline images, referenced by placeholder operators in `ops`.
    pub inline: crate::inline::Inline,
    /// Text lines inside form XObjects the content draws, in page user space. They
    /// are read and searched like the others but are not page objects: editing works
    /// on the page's own operators.
    pub form_text: Vec<TextData>,
}

impl Parsed {
    /// Whether the page content shows any text, visible or not, including inside forms.
    pub fn shows_text(&self) -> bool {
        !self.form_text.is_empty()
            || self
                .ops
                .iter()
                .any(|o| matches!(o.operator.as_str(), "Tj" | "TJ" | "'" | "\""))
    }

    /// Every text line: the page's own, then those inside forms.
    pub fn text_lines(&self) -> impl Iterator<Item = &TextData> {
        self.objs
            .iter()
            .filter_map(|o| o.text.as_ref())
            .chain(&self.form_text)
    }
}

/// Deepest nesting of form XObjects whose text is read.
const MAX_FORM_NESTING: usize = 8;

#[derive(Clone)]
struct Gs {
    ctm: M,
    fill: Option<[f32; 3]>,
    stroke: Option<[f32; 3]>,
    lw: f32,
    font: Vec<u8>,
    size: f32,
    tc: f32,
    tw: f32,
    th: f32,
    tl: f32,
    rise: f32,
    tr: i64,
}

impl Default for Gs {
    fn default() -> Self {
        Self {
            ctm: mx::ID,
            fill: Some([0.0; 3]),
            stroke: Some([0.0; 3]),
            lw: 1.0,
            font: Vec::new(),
            size: 0.0,
            tc: 0.0,
            tw: 0.0,
            th: 1.0,
            tl: 0.0,
            rise: 0.0,
            tr: 0,
        }
    }
}

fn n(o: &Object) -> Option<f32> {
    match o {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

fn nums(op: &Operation) -> Vec<f32> {
    op.operands.iter().filter_map(n).collect()
}

fn color_from(v: &[f32]) -> Option<[f32; 3]> {
    match v.len() {
        1 => Some([v[0]; 3]),
        3 => Some([v[0], v[1], v[2]]),
        4 => {
            let k = 1.0 - v[3];
            Some([(1.0 - v[0]) * k, (1.0 - v[1]) * k, (1.0 - v[2]) * k])
        }
        _ => None,
    }
}

/// One show operation while a text object is being read.
struct RunInfo {
    run: Run,
    text: String,
    font: Vec<u8>,
    base: String,
    reuse: bool,
    size: f32,
    start: (f32, f32),
    end: (f32, f32),
    dir: (f32, f32),
    fill: Option<[f32; 3]>,
    corners: [(f32, f32); 4],
    /// Each character's start and end in text space along the run, from 0 to the
    /// advance.
    spans: Vec<(f32, f32)>,
    adv: f32,
    /// Distance in user space from `start` to the first character, along `dir`.
    lead: f32,
    ctm: M,
    visible: bool,
    vertical: bool,
    tm_after: M,
    tlm_after: M,
}

impl Pdf {
    pub(crate) fn page_id_at(&self, index: usize) -> Result<ObjectId> {
        self.page_ids()
            .get(index)
            .copied()
            .ok_or_else(|| anyhow!("no page {}", index + 1))
    }

    /// Matrix from displayed-page coordinates (y down, points) to page user space, with the displayed size.
    pub(crate) fn vis_down(&self, page: ObjectId) -> (M, f32, f32) {
        let (v, vw, vh) = self.visual_matrix(page);
        (mul([1.0, 0.0, 0.0, -1.0, 0.0, vh], v), vw, vh)
    }

    pub(crate) fn parse_page(&self, page_index: usize) -> Result<Parsed> {
        self.parse_page_of(self.page_id_at(page_index)?)
    }

    /// [`Pdf::parse_page`] for the page with object `page`.
    fn parse_page_of(&self, page: ObjectId) -> Result<Parsed> {
        let data = self
            .doc
            .get_page_content_with_limit(page, crate::MAX_CONTENT)?;
        let res = self.inherited(page, b"Resources");
        self.parse_content(&data, res.as_ref(), mx::ID, 0)
    }

    /// Parse a content stream drawn with `resources`, starting from the transform `ctm`.
    /// `nesting` counts the form XObjects it is inside of.
    fn parse_content(
        &self,
        data: &[u8],
        resources: Option<&Object>,
        ctm: M,
        nesting: usize,
    ) -> Result<Parsed> {
        let (data, inline) = crate::inline::extract(data)
            .map_err(|e| anyhow!("cannot parse the page content: {e}"))?;
        let ops = Content::decode(&data)
            .map_err(|e| anyhow!("cannot parse the page content: {e}"))?
            .operations;

        let res = self.resolved_dict(resources);
        let fonts = self.resolved_dict(res.get(b"Font").ok());
        let xobjs = self.resolved_dict(res.get(b"XObject").ok());
        let mut font_cache: HashMap<Vec<u8>, Option<FontCtx>> = HashMap::new();
        let mut form_text: Vec<TextData> = Vec::new();

        let mut stack: Vec<Gs> = Vec::new();
        let mut gs = Gs {
            ctm,
            ..Gs::default()
        };
        let (mut tm, mut tlm) = (mx::ID, mx::ID);
        let mut objs: Vec<Obj> = Vec::new();
        let mut block: Vec<RunInfo> = Vec::new();
        let mut depth = 0usize;
        // current path
        let mut path_first: Option<usize> = None;
        let mut path_pts: Vec<(f32, f32)> = Vec::new();
        let mut clip_pending = false;

        for (i, op) in ops.iter().enumerate() {
            let v = nums(op);
            match op.operator.as_str() {
                "q" => {
                    stack.push(gs.clone());
                    depth += 1;
                }
                "Q" => {
                    if let Some(g) = stack.pop() {
                        gs = g;
                    }
                    depth = depth.saturating_sub(1);
                }
                "cm" if v.len() == 6 => gs.ctm = mul([v[0], v[1], v[2], v[3], v[4], v[5]], gs.ctm),
                "w" if !v.is_empty() => gs.lw = v[0],
                "g" | "rg" | "k" => gs.fill = color_from(&v),
                "G" | "RG" | "K" => gs.stroke = color_from(&v),
                "sc" | "scn" => gs.fill = color_from(&v).or(gs.fill),
                "SC" | "SCN" => gs.stroke = color_from(&v).or(gs.stroke),
                "BT" => {
                    tm = mx::ID;
                    tlm = mx::ID;
                    block.clear();
                }
                "Tf" => {
                    if let Some(Object::Name(nm)) = op.operands.first() {
                        gs.font = nm.clone();
                    }
                    if let Some(s) = op.operands.get(1).and_then(n) {
                        gs.size = s;
                    }
                }
                "Tc" if !v.is_empty() => gs.tc = v[0],
                "Tw" if !v.is_empty() => gs.tw = v[0],
                "Tz" if !v.is_empty() => gs.th = v[0] / 100.0,
                "TL" if !v.is_empty() => gs.tl = v[0],
                "Ts" if !v.is_empty() => gs.rise = v[0],
                "Tr" if !v.is_empty() => gs.tr = v[0] as i64,
                "Td" | "TD" if v.len() >= 2 => {
                    if op.operator == "TD" {
                        gs.tl = -v[1];
                    }
                    tlm = mul(mx::translate(v[0], v[1]), tlm);
                    tm = tlm;
                }
                "Tm" if v.len() == 6 => {
                    tlm = [v[0], v[1], v[2], v[3], v[4], v[5]];
                    tm = tlm;
                }
                "T*" => {
                    tlm = mul(mx::translate(0.0, -gs.tl), tlm);
                    tm = tlm;
                }
                "Tj" | "TJ" | "'" | "\"" => {
                    let (show, items): (Show, Vec<&Object>) = match op.operator.as_str() {
                        "Tj" => (Show::Tj, op.operands.iter().collect()),
                        "TJ" => (
                            Show::TJ,
                            op.operands
                                .first()
                                .and_then(|o| o.as_array().ok())
                                .map(|a| a.iter().collect())
                                .unwrap_or_default(),
                        ),
                        "'" => (Show::Quote, op.operands.iter().collect()),
                        _ => {
                            let aw = op.operands.first().and_then(n).unwrap_or(0.0);
                            let ac = op.operands.get(1).and_then(n).unwrap_or(0.0);
                            gs.tw = aw;
                            gs.tc = ac;
                            (
                                Show::DQuote { aw, ac },
                                op.operands.iter().skip(2).collect(),
                            )
                        }
                    };
                    if matches!(show, Show::Quote | Show::DQuote { .. }) {
                        tlm = mul(mx::translate(0.0, -gs.tl), tlm);
                        tm = tlm;
                    }
                    let fc = font_cache
                        .entry(gs.font.clone())
                        .or_insert_with(|| {
                            let d = fonts.get(&gs.font).ok()?;
                            let (_, d) = self.doc.dereference(d).ok()?;
                            d.as_dict()
                                .ok()
                                .map(|d: &Dictionary| FontCtx::new(&self.doc, d))
                        })
                        .as_ref();
                    let start_tm = tm;
                    let tfs = gs.size;
                    let th = gs.th;
                    let mut adv = 0.0f32;
                    let mut text = String::new();
                    // The text is decoded one character code at a time, with each
                    // character's place along the run. A glyph that decodes to several
                    // characters (a ligature) shares its width among them.
                    let mut spans: Vec<(f32, f32)> = Vec::new();
                    for it in items {
                        match it {
                            Object::String(s, _) => match fc {
                                Some(f) => {
                                    for (code, space, raw) in f.codes(s) {
                                        let glyph = f.width(code, raw.len()) / 1000.0 * tfs * th;
                                        let decoded = f.decode_code(code, raw);
                                        let chars = decoded.chars().count();
                                        let part = glyph / chars.max(1) as f32;
                                        for j in 0..chars {
                                            let a = adv + part * j as f32;
                                            spans.push((a, a + part));
                                        }
                                        text.push_str(&decoded);
                                        adv +=
                                            glyph + (gs.tc + if space { gs.tw } else { 0.0 }) * th;
                                    }
                                }
                                None => {
                                    for &b in s.iter() {
                                        let glyph = 0.5 * tfs * th;
                                        spans.push((adv, adv + glyph));
                                        adv += glyph
                                            + (gs.tc + if b == 32 { gs.tw } else { 0.0 }) * th;
                                    }
                                    text.extend(s.iter().map(|&b| b as char));
                                }
                            },
                            o => {
                                if let Some(x) = n(o) {
                                    let from = adv;
                                    adv -= x / 1000.0 * tfs * th;
                                    // A big negative kern is a visual gap: a space between
                                    // words. Justified lines squeeze these to under 0.2 em;
                                    // kerning between letters stays under about 0.14 em.
                                    if x < -150.0 && !text.ends_with(' ') && !text.is_empty() {
                                        text.push(' ');
                                        spans.push((from, adv));
                                    }
                                }
                            }
                        }
                    }
                    tm = mul(mx::translate(adv, 0.0), tm);
                    let trm = mul(mul(start_tm, gs.ctm), mx::ID);
                    let lin = (trm[0].hypot(trm[1])).max(1e-6);
                    let eff = tfs * lin;
                    let dir = (trm[0] / lin, trm[1] / lin);
                    let base_pt = apply(
                        mul(mx::translate(0.0, gs.rise), mul(start_tm, gs.ctm)),
                        0.0,
                        0.0,
                    );
                    let end_pt = apply(
                        mul(mx::translate(0.0, gs.rise), mul(start_tm, gs.ctm)),
                        adv,
                        0.0,
                    );
                    let m = mul(start_tm, gs.ctm);
                    let (d0, a0) = (-0.21 * tfs + gs.rise, 0.79 * tfs + gs.rise);
                    let corners = [
                        apply(m, 0.0, d0),
                        apply(m, adv, d0),
                        apply(m, adv, a0),
                        apply(m, 0.0, a0),
                    ];
                    // A TJ can open with a gap before its first character.
                    let lead = spans.first().map_or(0.0, |&(a, _)| {
                        let (p, q) = (apply(m, 0.0, 0.0), apply(m, a, 0.0));
                        (q.0 - p.0) * dir.0 + (q.1 - p.1) * dir.1
                    });
                    let ri = RunInfo {
                        run: Run {
                            op: i,
                            show,
                            tm: start_tm,
                            spacing: [gs.tc * lin, gs.tw * lin, th],
                            gap: 0.0,
                        },
                        text,
                        font: gs.font.clone(),
                        base: fc.map(|f| f.base.clone()).unwrap_or_default(),
                        reuse: fc.map(|f| f.reusable()).unwrap_or(false),
                        size: eff,
                        start: base_pt,
                        end: end_pt,
                        dir,
                        fill: gs.fill,
                        corners,
                        spans,
                        adv,
                        lead,
                        ctm: gs.ctm,
                        visible: gs.tr != 3 && gs.tr != 7,
                        vertical: fc.is_some_and(|f| f.vertical),
                        tm_after: tm,
                        tlm_after: tlm,
                    };
                    block.push(ri);
                }
                "ET" => {
                    group_runs(std::mem::take(&mut block), &mut objs);
                }
                // ---- paths
                "m" | "l" if v.len() >= 2 => {
                    path_first.get_or_insert(i);
                    path_pts.push((v[0], v[1]));
                }
                "c" if v.len() == 6 => {
                    path_first.get_or_insert(i);
                    path_pts.extend([(v[0], v[1]), (v[2], v[3]), (v[4], v[5])]);
                }
                "v" | "y" if v.len() == 4 => {
                    path_first.get_or_insert(i);
                    path_pts.extend([(v[0], v[1]), (v[2], v[3])]);
                }
                "re" if v.len() == 4 => {
                    path_first.get_or_insert(i);
                    path_pts.extend([(v[0], v[1]), (v[0] + v[2], v[1] + v[3])]);
                }
                "h" => {
                    path_first.get_or_insert(i);
                }
                "W" | "W*" => clip_pending = true,
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
                    let paint = op.operator.as_str();
                    if let (Some(first), true) = (
                        path_first,
                        paint != "n" && !clip_pending && !path_pts.is_empty(),
                    ) {
                        let fill = matches!(paint, "f" | "F" | "f*" | "B" | "B*" | "b" | "b*");
                        let stroke = matches!(paint, "S" | "s" | "B" | "B*" | "b" | "b*");
                        let closed = matches!(paint, "s" | "b" | "b*");
                        let evenodd = matches!(paint, "f*" | "B*" | "b*");
                        let sw = if stroke {
                            gs.lw * gs.ctm[0].hypot(gs.ctm[1]) / 2.0
                        } else {
                            0.0
                        };
                        let mut bb = mx::transform_rect(gs.ctm, mx::aabb(path_pts.iter().copied()));
                        bb = [bb[0] - sw, bb[1] - sw, bb[2] + sw, bb[3] + sw];
                        let _ = clip_pending;
                        objs.push(Obj {
                            kind: ObjectKind::Path,
                            first,
                            last: i,
                            ctm: gs.ctm,
                            bbox: bb,
                            text: None,
                            path: Some(PathData {
                                fill,
                                stroke,
                                evenodd,
                                closed,
                                fill_c: gs.fill,
                                stroke_c: gs.stroke,
                                lw: gs.lw,
                            }),
                            inline: false,
                        });
                    }
                    path_first = None;
                    path_pts.clear();
                    clip_pending = false;
                }
                "Do" => {
                    if let Some(Object::Name(nm)) = op.operands.first() {
                        let xo = xobjs
                            .get(nm)
                            .ok()
                            .and_then(|o| self.doc.dereference(o).ok())
                            .and_then(|(_, o)| o.as_stream().ok());
                        if let Some(s) = xo {
                            let sub = s
                                .dict
                                .get(b"Subtype")
                                .ok()
                                .and_then(|o| o.as_name().ok())
                                .unwrap_or(b"");
                            let (kind, rect) = match sub {
                                b"Image" => (ObjectKind::Image, [0.0, 0.0, 1.0, 1.0]),
                                b"Form" => {
                                    let bbox = s
                                        .dict
                                        .get(b"BBox")
                                        .ok()
                                        .and_then(|o| self.rect_of(o))
                                        .unwrap_or([0.0, 0.0, 1.0, 1.0]);
                                    let fm = s
                                        .dict
                                        .get(b"Matrix")
                                        .ok()
                                        .and_then(|o| self.matrix_of(o))
                                        .unwrap_or(mx::ID);
                                    // Its text, for reading and searching. A form without
                                    // resources of its own uses the page's.
                                    if nesting < MAX_FORM_NESTING {
                                        let own = s.dict.get(b"Resources").ok();
                                        let c = s.get_plain_content_with_limit(crate::MAX_CONTENT);
                                        let inner = c.ok().and_then(|c| {
                                            let ctm = mul(fm, gs.ctm);
                                            let r = own.or(resources);
                                            self.parse_content(&c, r, ctm, nesting + 1).ok()
                                        });
                                        if let Some(p) = inner {
                                            form_text
                                                .extend(p.objs.into_iter().filter_map(|o| o.text));
                                            form_text.extend(p.form_text);
                                        }
                                    }
                                    (ObjectKind::Form, mx::transform_rect(fm, bbox))
                                }
                                _ => continue,
                            };
                            objs.push(Obj {
                                kind,
                                first: i,
                                last: i,
                                ctm: gs.ctm,
                                bbox: mx::transform_rect(gs.ctm, rect),
                                text: None,
                                path: None,
                                inline: false,
                            });
                        }
                    }
                }
                t if inline.raw(t).is_some() => objs.push(Obj {
                    kind: ObjectKind::Image,
                    first: i,
                    last: i,
                    ctm: gs.ctm,
                    bbox: mx::transform_rect(gs.ctm, [0.0, 0.0, 1.0, 1.0]),
                    text: None,
                    path: None,
                    inline: true,
                }),
                _ => {}
            }
        }
        objs.sort_by_key(|o| o.first);
        objs.retain(|o| o.bbox.iter().all(|x| x.is_finite()));
        Ok(Parsed {
            ops,
            objs,
            depth_left: depth,
            inline,
            form_text,
        })
    }

    fn rect_of(&self, o: &Object) -> Option<[f32; 4]> {
        let (_, o) = self.doc.dereference(o).ok()?;
        let a = o.as_array().ok()?;
        if a.len() < 4 {
            return None;
        }
        let v: Vec<f32> = a
            .iter()
            .take(4)
            .map(|x| self.num(x))
            .collect::<Option<_>>()?;
        Some([
            v[0].min(v[2]),
            v[1].min(v[3]),
            v[0].max(v[2]),
            v[1].max(v[3]),
        ])
    }

    fn matrix_of(&self, o: &Object) -> Option<M> {
        let (_, o) = self.doc.dereference(o).ok()?;
        let a = o.as_array().ok()?;
        if a.len() < 6 {
            return None;
        }
        let v: Vec<f32> = a
            .iter()
            .take(6)
            .map(|x| self.num(x))
            .collect::<Option<_>>()?;
        Some([v[0], v[1], v[2], v[3], v[4], v[5]])
    }

    /// The selectable objects of a page (0-based index), in drawing order (last is on top).
    pub fn page_objects(&self, page_index: usize) -> Result<Vec<PageObject>> {
        let page = self.page_id_at(page_index)?;
        let parsed = self.parse_page(page_index)?;
        let (vd, _, _) = self.vis_down(page);
        let to_vis = mx::inv(vd).ok_or_else(|| anyhow!("degenerate page transform"))?;
        Ok(parsed
            .objs
            .iter()
            .enumerate()
            .map(|(id, o)| {
                let bbox = mx::transform_rect(to_vis, o.bbox);
                let mut po = PageObject {
                    id,
                    kind: o.kind,
                    bbox,
                    text: None,
                    font_name: None,
                    font_size: None,
                    fill: None,
                    stroke: None,
                    line_width: None,
                    has_fill: false,
                    has_stroke: false,
                    inline: o.inline,
                    font_reusable: false,
                    hidden: false,
                    baseline: None,
                    dir: None,
                    stops: Vec::new(),
                    upright: false,
                    vertical: false,
                    spans: Vec::new(),
                };
                if let Some(t) = &o.text {
                    po.hidden = t.hidden;
                    po.text = Some(t.text.clone());
                    po.font_name = Some(t.base.clone());
                    po.font_size = Some(t.size);
                    po.fill = t.fill;
                    po.font_reusable = t.reuse;
                    po.baseline = Some(apply(to_vis, t.origin.0, t.origin.1));
                    po.vertical = t.vertical;
                    po.spans = t
                        .spans
                        .iter()
                        .map(|s| TextSpan {
                            chars: s.chars,
                            font_name: s.base.clone(),
                            font_size: s.size,
                            fill: s.fill,
                            font_reusable: s.reuse,
                            rise: s.rise,
                        })
                        .collect();
                    // The text's own across and up directions, on the displayed page (y down).
                    let m = mul(mul(t.runs[0].tm, o.ctm), to_vis);
                    let (across, up) = ((m[0], m[1]), (m[2], m[3]));
                    let level = across.0 > 0.0 && across.1.abs() <= 1e-3 * across.0;
                    po.upright = level && up.1 < 0.0 && !t.vertical;
                    let len = across.0.hypot(across.1);
                    if let (Some(at), true) = (po.baseline, len > 1e-6) {
                        let dir = (across.0 / len, across.1 / len);
                        po.dir = Some(dir);
                        po.stops = caret_stops(&t.pieces, to_vis, at, dir);
                    }
                }
                if let Some(p) = &o.path {
                    po.fill = p.fill_c;
                    po.stroke = p.stroke_c;
                    po.has_fill = p.fill;
                    po.has_stroke = p.stroke;
                    po.line_width = Some(p.lw * o.ctm[0].hypot(o.ctm[1]));
                }
                po
            })
            .collect())
    }
}

impl Pdf {
    /// Boxes of the places on a page (0-based) where `query` occurs, as
    /// [`Pdf::find_text_matches`] finds them, all in one list.
    pub fn find_text_boxes(&self, page_index: usize, query: &str) -> Result<Vec<[f32; 4]>> {
        let matches = self.find_text_matches(page_index, query)?;
        Ok(matches.into_iter().flatten().collect())
    }

    /// Whether a page (0-based) shows any text, visible or not, including text inside
    /// form XObjects. False when the page cannot be read.
    pub fn page_has_text(&self, page_index: usize) -> bool {
        self.parse_page(page_index).is_ok_and(|p| p.shows_text())
    }

    /// Whether a page's own resources hold a picture whose data is plainly unreadable
    /// (judged from the start of its data, not by decoding it). Pictures inside form
    /// XObjects aren't looked at.
    pub fn page_has_unreadable_picture(&self, page_index: usize) -> bool {
        let Ok(page) = self.page_id_at(page_index) else {
            return false;
        };
        let dict = |o: &Object| self.doc.dereference(o).ok()?.1.as_dict().ok().cloned();
        let Some(xobjects) = self
            .inherited(page, b"Resources")
            .and_then(|r| dict(&r))
            .and_then(|r| dict(r.get(b"XObject").ok()?))
        else {
            return false;
        };
        xobjects.iter().any(|(_, o)| {
            self.doc
                .dereference(o)
                .ok()
                .and_then(|(_, o)| o.as_stream().ok())
                .is_some_and(|s| {
                    s.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image")
                        && !crate::imgs::image_data_readable(s)
                })
        })
    }

    /// The words of a page (0-based), visible and invisible (OCR), in reading order worked
    /// out from their positions (see [`crate::reading`]), not the order the page draws
    /// them in. `line` numbers the visual lines. Boxes are in displayed-page points (y
    /// down).
    pub fn page_words(&self, page_index: usize) -> Result<Vec<PageWord>> {
        let (words, _, _) = self.search_parts(self.page_id_at(page_index)?)?;
        Ok(words.into_iter().map(|(w, _, _)| w).collect())
    }

    /// What search reads of the page with object `page`: its words in reading order, each
    /// with its text line and first character there (see [`ordered_words`]), each text
    /// line's pieces, and the matrix from page user space to displayed-page points.
    pub(crate) fn search_parts(&self, page: ObjectId) -> Result<SearchParts> {
        let parsed = self.parse_page_of(page)?;
        let (vd, _, _) = self.vis_down(page);
        let to_vis = mx::inv(vd).ok_or_else(|| anyhow!("degenerate page transform"))?;
        let lines: Vec<&TextData> = parsed.text_lines().collect();
        let words = ordered_words(&lines, to_vis);
        Ok((
            words,
            lines.iter().map(|l| l.pieces.clone()).collect(),
            to_vis,
        ))
    }
}

/// See [`Pdf::search_parts`].
pub(crate) type SearchParts = (Vec<(PageWord, usize, usize)>, Vec<Vec<Piece>>, M);

/// The words of a page's text lines in reading order (see [`Pdf::page_words`]), each with
/// the index of the text line it came from and its first character there.
fn ordered_words(lines: &[&TextData], to_vis: M) -> Vec<(PageWord, usize, usize)> {
    let mut out = Vec::new();
    for (li, t) in lines.iter().enumerate() {
        let chars: Vec<char> = t.text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_whitespace() {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            let boxes = span_boxes(&t.pieces, start, i);
            if boxes.is_empty() {
                continue;
            }
            let bb = mx::aabb(boxes.iter().flat_map(|b| [(b[0], b[1]), (b[2], b[3])]));
            let word = PageWord {
                text: chars[start..i].iter().collect(),
                bbox: mx::transform_rect(to_vis, bb),
                line: li,
            };
            out.push(Some((word, li, start)));
        }
    }
    let boxes: Vec<[f32; 4]> = out.iter().flatten().map(|w| w.0.bbox).collect();
    crate::reading::reading_order(&boxes)
        .into_iter()
        .filter_map(|(i, row)| {
            let mut w = out[i].take()?;
            w.0.line = row;
            Some(w)
        })
        .collect()
}

/// A word on a page: its text, its box in displayed-page points (y down), and the index
/// of the text line it belongs to.
#[derive(Clone, Debug, PartialEq)]
pub struct PageWord {
    pub text: String,
    pub bbox: [f32; 4],
    pub line: usize,
}

/// Text of a run of words, as for copying: spaces within a line, a line break between
/// lines.
pub fn words_text(words: &[PageWord]) -> String {
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            let same_line = words[i - 1].line == w.line;
            out.push(if same_line { ' ' } else { '\n' });
        }
        out.push_str(&w.text);
    }
    out
}

/// User-space boxes covering characters `a..b` of a line, one per piece they touch.
/// Within a piece, characters are placed by their glyph widths, or spread evenly when
/// those are not known.
pub(crate) fn span_boxes(pieces: &[Piece], a: usize, b: usize) -> Vec<[f32; 4]> {
    let lerp =
        |p: (f32, f32), q: (f32, f32), f: f32| (p.0 + (q.0 - p.0) * f, p.1 + (q.1 - p.1) * f);
    let mut out = Vec::new();
    for p in pieces {
        let (s, e) = p.chars;
        let (lo, hi) = (a.max(s), b.min(e));
        if lo >= hi {
            continue;
        }
        let (f0, f1) = if p.spans.len() == e - s {
            (p.spans[lo - s].0, p.spans[hi - 1 - s].1)
        } else {
            let n = (e - s) as f32;
            ((lo - s) as f32 / n, (hi - s) as f32 / n)
        };
        let c = p.corners;
        let pts = [
            lerp(c[0], c[1], f0),
            lerp(c[0], c[1], f1),
            lerp(c[3], c[2], f1),
            lerp(c[3], c[2], f0),
        ];
        out.push(mx::aabb(pts));
    }
    out
}

/// Where each character of a line's `pieces` starts along it and where the last one ends,
/// in displayed-page points from `origin` towards `dir` ([`PageObject::stops`]). Within a
/// piece, characters are placed as [`span_boxes`] places them.
fn caret_stops(pieces: &[Piece], to_vis: M, origin: (f32, f32), dir: (f32, f32)) -> Vec<f32> {
    // How far along the line the point `f` of the way from `a` to `b` (user space) is.
    let along = |(a, b): ((f32, f32), (f32, f32)), f: f32| {
        let (x, y) = apply(to_vis, a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
        (x - origin.0) * dir.0 + (y - origin.1) * dir.1
    };
    let mut stops = Vec::new();
    let mut end = None;
    for p in pieces {
        let n = p.chars.1 - p.chars.0;
        let ends = (p.corners[0], p.corners[1]);
        for j in 0..n {
            let (f0, f1) = if p.spans.len() == n {
                p.spans[j]
            } else {
                (j as f32 / n as f32, (j + 1) as f32 / n as f32)
            };
            stops.push(along(ends, f0));
            end = Some(along(ends, f1));
        }
    }
    stops.extend(end);
    stops
}

/// A line's box in user space: its pieces' boxes, less the pieces showing only whitespace
/// and the whitespace at the line's two ends, which draw nothing and would reach over
/// what is next to the line (a label's tab in a larger font over the next label). Shows
/// with no characters count where they are among the line's text.
fn line_box(pieces: &[Piece], text: &str) -> [f32; 4] {
    let chars: Vec<char> = text.chars().collect();
    let first = chars.iter().position(|c| !c.is_whitespace()).unwrap_or(0);
    let last = chars
        .iter()
        .rposition(|c| !c.is_whitespace())
        .map_or(chars.len(), |i| i + 1);
    let mut points = Vec::new();
    for p in pieces {
        let (s, e) = p.chars;
        if s == e {
            if s >= first && s <= last {
                points.extend(p.corners);
            }
        } else if chars[s..e].iter().all(|c| c.is_whitespace()) {
            continue;
        } else if s >= first && e <= last {
            points.extend(p.corners);
        } else {
            let part = span_boxes(std::slice::from_ref(p), s.max(first), e.min(last));
            points.extend(part.iter().flat_map(|b| [(b[0], b[1]), (b[2], b[3])]));
        }
    }
    mx::aabb(points)
}

/// Turn the show operations of one BT..ET block into line objects.
fn group_runs(runs: Vec<RunInfo>, out: &mut Vec<Obj>) {
    let mut cur: Vec<RunInfo> = Vec::new();
    let flush = |cur: &mut Vec<RunInfo>, out: &mut Vec<Obj>| {
        if cur.is_empty() {
            return;
        }
        let group = std::mem::take(cur);
        let text: String = group
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        if text.trim().is_empty() {
            return;
        }
        let mut at = 0;
        let pieces: Vec<Piece> = group
            .iter()
            .map(|r| {
                let n = r.text.chars().count();
                at += n;
                let spans = if r.spans.len() == n && r.adv.abs() > 1e-6 {
                    r.spans
                        .iter()
                        .map(|&(a, b)| (a / r.adv, b / r.adv))
                        .collect()
                } else {
                    Vec::new()
                };
                Piece {
                    chars: (at - n, at),
                    corners: r.corners,
                    spans,
                }
            })
            .collect();
        let first = &group[0];
        // The line's baseline is its largest text's (raised and lowered parts are smaller),
        // and each part's rise is how far above it the part starts; float noise isn't one.
        let main = group
            .iter()
            .fold(first, |m, r| if r.size > m.size * 1.01 { r } else { m });
        let up = (-first.dir.1, first.dir.0);
        let rise = |r: &RunInfo| {
            let h = (r.start.0 - main.start.0) * up.0 + (r.start.1 - main.start.1) * up.1;
            if h.abs() < 0.02 * main.size {
                0.0
            } else {
                h
            }
        };
        let lift = rise(first);
        let origin = (first.start.0 - lift * up.0, first.start.1 - lift * up.1);
        let spans = styled_spans(&group, &pieces, rise, main.size);
        let bb = line_box(&pieces, &text);
        out.push(Obj {
            kind: ObjectKind::Text,
            first: first.run.op,
            last: group.last().unwrap().run.op,
            ctm: first.ctm,
            bbox: bb,
            text: Some(TextData {
                runs: group.iter().map(|r| r.run.clone()).collect(),
                tm_end: group.last().unwrap().tm_after,
                tlm_end: group.last().unwrap().tlm_after,
                text,
                base: spans[0].base.clone(),
                size: spans[0].size,
                origin,
                dir: first.dir,
                fill: spans[0].fill,
                reuse: spans[0].reuse,
                hidden: !group.iter().any(|r| r.visible),
                vertical: group.iter().any(|r| r.vertical),
                pieces,
                spans,
            }),
            path: None,
            inline: false,
        });
    };
    for r in runs {
        // Shows join into a line whatever their font, size and colour, when they are next
        // to each other on one baseline, measured against the larger text of the two.
        let join = match cur.last() {
            None => true,
            Some(p) => {
                let same = p.visible == r.visible && p.dir.0 * r.dir.0 + p.dir.1 * r.dir.1 > 0.9999;
                let (dx, dy) = (r.start.0 - p.end.0, r.start.1 - p.end.1);
                let along = dx * p.dir.0 + dy * p.dir.1 + r.lead;
                let perp = (-dx * p.dir.1 + dy * p.dir.0).abs();
                let size = p.size.max(r.size);
                same && perp < 0.3 * size && along > -0.3 * size && along < 0.7 * size
            }
        };
        if !join {
            flush(&mut cur, out);
        }
        if let Some(p) = cur.last_mut() {
            // From the end of the last piece to this piece's first character. A space
            // between words is measured against the smaller text of the two.
            let (dx, dy) = (r.start.0 - p.end.0, r.start.1 - p.end.1);
            let along = dx * p.dir.0 + dy * p.dir.1 + r.lead;
            let size = p.size.min(r.size);
            if along > 0.18 * size && !p.text.ends_with(' ') && !r.text.starts_with(' ') {
                p.text.push(' ');
                p.run.gap = along;
                // The space covers the gap, measured in the run's own units.
                let c = p.corners;
                let len = (c[1].0 - c[0].0).hypot(c[1].1 - c[0].1);
                if !p.spans.is_empty() && len > 1e-6 {
                    let gap = along * p.adv / len;
                    p.spans.push((p.adv, p.adv + gap));
                }
            }
        }
        cur.push(r);
    }
    flush(&mut cur, out);
}

/// The parts of a line in one style each: shows next to each other in the same font,
/// size, fill and rise are one part. A show with no characters goes with the part
/// before it, or takes the style of the one after it at the start of the line.
fn styled_spans(
    group: &[RunInfo],
    pieces: &[Piece],
    rise: impl Fn(&RunInfo) -> f32,
    size: f32,
) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    for (k, (r, p)) in group.iter().zip(pieces).enumerate() {
        let lift = rise(r);
        let own = |runs_from: usize| Span {
            chars: p.chars,
            runs: (runs_from, k + 1),
            font: r.font.clone(),
            base: r.base.clone(),
            reuse: r.reuse,
            size: r.size,
            fill: r.fill,
            rise: lift,
        };
        let empty = p.chars.0 == p.chars.1;
        match spans.last_mut() {
            Some(s)
                if empty
                    || (s.font == r.font
                        && (s.size - r.size).abs() < 0.01 * s.size.max(0.01)
                        && s.fill == r.fill
                        && (s.rise - lift).abs() < 0.02 * size) =>
            {
                s.chars.1 = p.chars.1;
                s.runs.1 = k + 1;
            }
            Some(s) if s.chars.0 == s.chars.1 => *s = own(s.runs.0),
            _ => spans.push(own(k)),
        }
    }
    spans
}

#[allow(dead_code)]
pub(crate) fn family_of(base: &str) -> FontChoice {
    let _ = Family::Helvetica;
    FontChoice::from_name(base)
}

#[allow(dead_code)]
fn _unused(_: &Dictionary) -> Result<()> {
    bail!("unused")
}
