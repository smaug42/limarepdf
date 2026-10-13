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
    /// Where each show's characters are, for locating matches within the line.
    pub pieces: Vec<Piece>,
}

/// The characters `chars.0..chars.1` of a line's text, shown by one operator, and the
/// corners of their box in page user space: start-bottom, end-bottom, end-top, start-top.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    pub chars: (usize, usize),
    pub corners: [(f32, f32); 4],
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
    pub inline: Vec<Vec<u8>>,
}

impl Parsed {
    /// Whether the page content shows any text, visible or not.
    pub fn shows_text(&self) -> bool {
        self.ops
            .iter()
            .any(|o| matches!(o.operator.as_str(), "Tj" | "TJ" | "'" | "\""))
    }
}

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
    ctm: M,
    visible: bool,
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
        let page = self.page_id_at(page_index)?;
        let data = self.doc.get_page_content(page);
        let (data, inline) = crate::inline::extract(&data)
            .map_err(|e| anyhow!("cannot parse the page content: {e}"))?;
        let ops = Content::decode(&data)
            .map_err(|e| anyhow!("cannot parse the page content: {e}"))?
            .operations;

        let res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
        let fonts = self.resolved_dict(res.get(b"Font").ok());
        let xobjs = self.resolved_dict(res.get(b"XObject").ok());
        let mut font_cache: HashMap<Vec<u8>, Option<FontCtx>> = HashMap::new();

        let mut stack: Vec<Gs> = Vec::new();
        let mut gs = Gs::default();
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
                    let mut bytes: Vec<u8> = Vec::new();
                    let mut text = String::new();
                    for it in items {
                        match it {
                            Object::String(s, _) => {
                                match fc {
                                    Some(f) => {
                                        for (code, space) in f.codes(s) {
                                            adv += (f.width(code) / 1000.0 * tfs
                                                + gs.tc
                                                + if space { gs.tw } else { 0.0 })
                                                * th;
                                        }
                                        text.push_str(&f.decode(s));
                                    }
                                    None => {
                                        for &b in s {
                                            adv += (0.5 * tfs
                                                + gs.tc
                                                + if b == 32 { gs.tw } else { 0.0 })
                                                * th;
                                        }
                                        text.extend(s.iter().map(|&b| b as char));
                                    }
                                }
                                bytes.extend_from_slice(s);
                            }
                            o => {
                                if let Some(x) = n(o) {
                                    adv -= x / 1000.0 * tfs * th;
                                    // A big negative kern is a visual gap (a space between words).
                                    if x < -200.0 && !text.ends_with(' ') && !text.is_empty() {
                                        text.push(' ');
                                    }
                                }
                            }
                        }
                    }
                    let _ = bytes;
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
                    let ri = RunInfo {
                        run: Run {
                            op: i,
                            show,
                            tm: start_tm,
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
                        ctm: gs.ctm,
                        visible: gs.tr != 3 && gs.tr != 7,
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
                t if t.starts_with(crate::inline::TAG) => objs.push(Obj {
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
                };
                if let Some(t) = &o.text {
                    po.hidden = t.hidden;
                    po.text = Some(t.text.clone());
                    po.font_name = Some(t.base.clone());
                    po.font_size = Some(t.size);
                    po.fill = t.fill;
                    po.font_reusable = t.reuse;
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
    /// Boxes of the places on a page (0-based) where `query` occurs, ignoring case, in
    /// displayed-page points (y down), like [`PageObject::bbox`]. Visible and invisible
    /// (OCR) text both count. Matches are found within one text line; a phrase broken
    /// across lines is not found.
    pub fn find_text_boxes(&self, page_index: usize, query: &str) -> Result<Vec<[f32; 4]>> {
        let fold = |s: &str| -> Vec<char> {
            s.chars()
                .map(|c| c.to_lowercase().next().unwrap_or(c))
                .collect()
        };
        let q = fold(query.trim());
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let page = self.page_id_at(page_index)?;
        let parsed = self.parse_page(page_index)?;
        let (vd, _, _) = self.vis_down(page);
        let to_vis = mx::inv(vd).ok_or_else(|| anyhow!("degenerate page transform"))?;
        let mut out = Vec::new();
        for t in parsed.objs.iter().filter_map(|o| o.text.as_ref()) {
            let text = fold(&t.text);
            let mut from = 0;
            while from + q.len() <= text.len() {
                let Some(pos) = text[from..].windows(q.len()).position(|w| w == q) else {
                    break;
                };
                let (a, b) = (from + pos, from + pos + q.len());
                for p in &t.pieces {
                    let (s, e) = p.chars;
                    let (lo, hi) = (a.max(s), b.min(e));
                    if lo >= hi {
                        continue;
                    }
                    // Interpolate along the piece by character position.
                    let n = (e - s) as f32;
                    let (f0, f1) = ((lo - s) as f32 / n, (hi - s) as f32 / n);
                    let c = p.corners;
                    let lerp = |p: (f32, f32), q: (f32, f32), f: f32| {
                        (p.0 + (q.0 - p.0) * f, p.1 + (q.1 - p.1) * f)
                    };
                    let pts = [
                        lerp(c[0], c[1], f0),
                        lerp(c[0], c[1], f1),
                        lerp(c[3], c[2], f1),
                        lerp(c[3], c[2], f0),
                    ];
                    let bb = mx::aabb(pts.iter().copied());
                    out.push(mx::transform_rect(to_vis, bb));
                }
                from = b;
            }
        }
        Ok(out)
    }
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
        let pieces = group
            .iter()
            .map(|r| {
                let n = r.text.chars().count();
                at += n;
                Piece {
                    chars: (at - n, at),
                    corners: r.corners,
                }
            })
            .collect();
        let first = &group[0];
        let bb = mx::aabb(group.iter().flat_map(|r| r.corners));
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
                base: first.base.clone(),
                size: first.size,
                origin: first.start,
                dir: first.dir,
                fill: first.fill,
                reuse: first.reuse,
                hidden: !group.iter().any(|r| r.visible),
                pieces,
            }),
            path: None,
            inline: false,
        });
    };
    for r in runs {
        let join = match cur.last() {
            None => true,
            Some(p) => {
                let same = p.font == r.font
                    && (p.size - r.size).abs() < 0.01 * p.size.max(0.01)
                    && p.fill == r.fill
                    && p.visible == r.visible
                    && p.dir.0 * r.dir.0 + p.dir.1 * r.dir.1 > 0.9999;
                let (dx, dy) = (r.start.0 - p.end.0, r.start.1 - p.end.1);
                let along = dx * p.dir.0 + dy * p.dir.1;
                let perp = (-dx * p.dir.1 + dy * p.dir.0).abs();
                same && perp < 0.3 * p.size && along > -0.3 * p.size && along < 0.7 * p.size
            }
        };
        if !join {
            flush(&mut cur, out);
        }
        if let Some(p) = cur.last_mut() {
            let (dx, dy) = (r.start.0 - p.end.0, r.start.1 - p.end.1);
            let along = dx * p.dir.0 + dy * p.dir.1;
            if along > 0.18 * p.size && !p.text.ends_with(' ') && !r.text.starts_with(' ') {
                p.text.push(' ');
            }
        }
        cur.push(r);
    }
    flush(&mut cur, out);
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
