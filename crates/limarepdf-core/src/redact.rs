//! Redaction: remove the text, pictures and drawings under marked areas of a page, not
//! just cover them, then check that nothing readable is left there.
//!
//! The page's content is rewritten operator by operator:
//!
//! * Text: every character whose box touches a mark is taken out of its string, and a
//!   `TJ` spacing number takes its place, so the characters left stay where they were.
//! * Drawings: the parts of a path (sub-paths) mostly under a mark are removed; parts
//!   that only cross a mark's edge are kept but clipped out of it. Shadings are clipped.
//! * Pictures under a mark are removed.
//! * Form XObjects (grouped content) under a mark are redacted inside, as a copy for this
//!   page.
//! * Alternative text (`/ActualText`, `/Alt`, `/E`) on marked content that lost anything is
//!   removed, here and in the structure tree, so copying can't bring the text back.
//!
//! Annotations touching a mark go, as do the page's thumbnail and private application
//! data. Pictures and drawings the page no longer uses are dropped from shared
//! resources, so saving (which prunes unused objects) leaves nothing of them in the file.

use crate::fonts::FontCtx;
use crate::matrix::{self as mx, apply, mul, M};
use crate::objects::span_boxes;
use crate::redact_img::{black_out, inline_stream, is_clear};
use crate::{Pdf, Result};
use anyhow::{anyhow, bail};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId, Stream};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};

/// What a redaction removed from one page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageRedaction {
    /// The page (zero-based).
    pub page: usize,
    /// Characters of text removed, visible or not.
    pub chars: usize,
    /// The removed text, one entry per run of removed characters.
    pub text: Vec<String>,
    /// Pictures removed whole: those LimarePDF can't read (such as JPEG 2000 and JBIG2).
    pub images: usize,
    /// Pictures blacked out where they are under a mark.
    pub images_cut: usize,
    /// Parts of drawings removed.
    pub paths: usize,
    /// Drawings and shadings that cross a mark's edge: kept, but cut out of the mark.
    pub clipped: usize,
    /// Comments, links and form fields removed (with replies to them).
    pub annots: usize,
}

/// What [`Pdf::redact`] removed, and what its check afterwards found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RedactReport {
    pub pages: Vec<PageRedaction>,
    /// Anything readable still under a mark after redacting. Empty when the check passed.
    pub left: Vec<String>,
    /// Removed text that can still be found elsewhere in the document (other pages,
    /// bookmarks, comments, form fields, document information), with where.
    pub elsewhere: Vec<(String, Vec<String>)>,
}

/// Overlap, in points, below which two boxes only meet and don't touch, so that the
/// characters beside a mark made from a text selection stay.
const EPS: f32 = 0.05;
/// A part of a drawing at least this much under a mark is removed; less, it is clipped.
const MOSTLY: f32 = 0.5;
/// A character at least this much under a mark is removed. Less is mostly visible beside
/// the black box anyway, and kerned neighbours (whose boxes overlap a little) stay.
const CHAR_SHARE: f32 = 0.25;
/// Half the size of the clip rectangle that a mark is cut out of, in points.
const FAR: f32 = 1.0e5;
/// Deepest nesting of form XObjects redacted inside; deeper ones under a mark go whole.
const MAX_NESTING: usize = 8;
/// Places listed for each piece of removed text found elsewhere.
const MAX_PLACES: usize = 10;

/// The unit square pictures are drawn in.
const UNIT: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// A name for a new entry in a resource dictionary.
fn fresh_name(d: &Dictionary) -> Vec<u8> {
    (1..)
        .map(|n| format!("LimareRd{n}").into_bytes())
        .find(|k| !d.has(k))
        .unwrap_or_default()
}

fn op(name: &str, operands: Vec<Object>) -> Operation {
    Operation::new(name, operands)
}

fn r(x: f32) -> Object {
    Object::Real(x)
}

fn norm(b: [f32; 4]) -> [f32; 4] {
    [
        b[0].min(b[2]),
        b[1].min(b[3]),
        b[0].max(b[2]),
        b[1].max(b[3]),
    ]
}

/// Whether `[a0, a1]` overlaps `[b0, b1]` by more than [`EPS`]; a span with no length
/// overlaps when its middle is inside.
fn overlap_1d(a0: f32, a1: f32, b0: f32, b1: f32) -> bool {
    if a1 - a0 < EPS {
        let m = (a0 + a1) / 2.0;
        return m > b0 && m < b1;
    }
    a0.max(b0) + EPS < a1.min(b1)
}

/// Whether box `a` touches the mark `b` (both normalised, same space).
fn touches(a: [f32; 4], b: [f32; 4]) -> bool {
    overlap_1d(a[0], a[2], b[0], b[2]) && overlap_1d(a[1], a[3], b[1], b[3])
}

/// The share of `[a0, a1]` inside `[b0, b1]`; for a span with no length, 1 or 0.
fn share_1d(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    if a1 - a0 < EPS {
        let m = (a0 + a1) / 2.0;
        return if m > b0 && m < b1 { 1.0 } else { 0.0 };
    }
    ((a1.min(b1) - a0.max(b0)).max(0.0) / (a1 - a0)).min(1.0)
}

/// The share of box `a` under the marks, up to 1.
fn share_under(a: [f32; 4], marks: &[[f32; 4]]) -> f32 {
    marks
        .iter()
        .map(|b| share_1d(a[0], a[2], b[0], b[2]) * share_1d(a[1], a[3], b[1], b[3]))
        .sum::<f32>()
        .min(1.0)
}

/// Whether a character's box (normalised) is under the marks enough to be removed.
fn char_under(b: [f32; 4], marks: &[[f32; 4]]) -> bool {
    b.iter().all(|x| x.is_finite()) && share_under(b, marks) >= CHAR_SHARE
}

fn nums(op: &Operation) -> Vec<f32> {
    op.operands
        .iter()
        .filter_map(|o| o.as_float().ok())
        .collect()
}

/// Lowercase, with whitespace runs as one space: how removed text is looked for.
fn fold(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[derive(Clone)]
struct Gs {
    ctm: M,
    lw: f32,
    font: Vec<u8>,
    size: f32,
    tc: f32,
    tw: f32,
    th: f32,
    tl: f32,
    rise: f32,
}

impl Gs {
    fn new(ctm: M) -> Self {
        Self {
            ctm,
            lw: 1.0,
            font: Vec::new(),
            size: 0.0,
            tc: 0.0,
            tw: 0.0,
            th: 1.0,
            tl: 0.0,
            rise: 0.0,
        }
    }
}

/// A content stream after redaction.
struct Rewritten {
    ops: Vec<Operation>,
    inline: crate::inline::Inline,
    res: Dictionary,
    /// `q` operators left open at the end.
    depth: usize,
    changed: bool,
}

/// One open marked-content sequence: where its `BDC`/`BMC` is in the output, and whether
/// anything inside it was removed.
struct Marked {
    at: usize,
    touched: bool,
}

struct Redactor<'a> {
    pdf: &'a Pdf,
    /// The marks, in page user space.
    areas: Vec<[f32; 4]>,
    next_id: u32,
    /// New objects (redacted copies of forms), added to the document afterwards.
    added: Vec<(ObjectId, Object)>,
    /// Pictures and forms the page no longer draws.
    dropped: HashSet<ObjectId>,
    /// Marked-content ids on the page whose content lost something.
    mcids: HashSet<i64>,
    out: PageRedaction,
}

impl<'a> Redactor<'a> {
    fn new(pdf: &'a Pdf, areas: Vec<[f32; 4]>) -> Self {
        Self {
            pdf,
            areas,
            next_id: pdf.doc.max_id + 1,
            added: Vec::new(),
            dropped: HashSet::new(),
            mcids: HashSet::new(),
            out: PageRedaction::default(),
        }
    }

    fn hit(&self, b: [f32; 4]) -> bool {
        let b = norm(b);
        b.iter().all(|x| x.is_finite()) && self.areas.iter().any(|&a| touches(b, a))
    }

    fn char_hit(&self, b: [f32; 4]) -> bool {
        char_under(norm(b), &self.areas)
    }

    /// Black out the part of picture `s` (drawn with `ctm`) under the marks, as a new
    /// object. `Ok(None)` when none of it is under them; `Err` when it can't be read.
    fn cut_image(
        &mut self,
        s: &Stream,
        spaces: &Dictionary,
        ctm: M,
    ) -> std::result::Result<Option<ObjectId>, String> {
        let Some(cut) = black_out(&self.pdf.doc, s, spaces, ctm, &self.areas)? else {
            return Ok(None);
        };
        let mut image = cut.image;
        for (key, mask) in cut.masks {
            let id = self.alloc();
            self.added.push((id, Object::Stream(mask)));
            image.dict.set(key, Object::Reference(id));
        }
        let id = self.alloc();
        self.added.push((id, Object::Stream(image)));
        Ok(Some(id))
    }

    fn removed_char(&mut self, s: &str) {
        self.out.chars += s.chars().count();
    }

    fn alloc(&mut self) -> ObjectId {
        let id = (self.next_id, 0);
        self.next_id += 1;
        id
    }

    /// Path operators that cut each mark touching `b` (or every mark) out of the clip, in
    /// the user space of `ctm`.
    fn clip_out(&self, ctm: M, b: Option<[f32; 4]>) -> Vec<Operation> {
        let Some(inv) = mx::inv(ctm) else {
            return Vec::new();
        };
        let quad = |b: [f32; 4], out: &mut Vec<Operation>| {
            let pts = [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])];
            for (k, (x, y)) in pts.into_iter().enumerate() {
                let (x, y) = apply(inv, x, y);
                out.push(op(if k == 0 { "m" } else { "l" }, vec![r(x), r(y)]));
            }
            out.push(op("h", vec![]));
        };
        let mut out = Vec::new();
        for &a in &self.areas {
            if b.is_some_and(|b| !touches(b, a)) {
                continue;
            }
            quad([-FAR, -FAR, FAR, FAR], &mut out);
            quad(a, &mut out);
            out.push(op("W*", vec![]));
            out.push(op("n", vec![]));
        }
        out
    }

    /// Redact one content stream drawn with `res` from the transform `ctm`.
    fn content(
        &mut self,
        data: &[u8],
        res: &Dictionary,
        ctm: M,
        nesting: usize,
    ) -> Result<Rewritten> {
        let (data, inline) = crate::inline::extract(data)
            .map_err(|e| anyhow!("cannot parse the page content: {e}"))?;
        let ops = Content::decode(&data)
            .map_err(|e| anyhow!("cannot parse the page content: {e}"))?
            .operations;
        let pdf = self.pdf;
        let mut res = res.clone();
        let fonts = pdf.resolved_dict(res.get(b"Font").ok());
        let mut xobjs = pdf.resolved_dict(res.get(b"XObject").ok());
        let props = pdf.resolved_dict(res.get(b"Properties").ok());
        let spaces = pdf.resolved_dict(res.get(b"ColorSpace").ok());
        let mut font_cache: HashMap<Vec<u8>, Option<FontCtx>> = HashMap::new();

        let mut out: Vec<Operation> = Vec::with_capacity(ops.len());
        let mut changed = false;
        let mut stack: Vec<Gs> = Vec::new();
        let mut gs = Gs::new(ctm);
        let (mut tm, mut tlm) = (mx::ID, mx::ID);
        let mut path: Vec<Operation> = Vec::new();
        let mut clip = false;
        let mut marked: Vec<Marked> = Vec::new();
        // XObject names whose drawing was removed or replaced here.
        let mut unnamed: HashSet<Vec<u8>> = HashSet::new();
        let mut xobjs_changed = false;

        for o in ops {
            let v = nums(&o);
            let name = o.operator.clone();
            let is_path = matches!(name.as_str(), "m" | "l" | "c" | "v" | "y" | "re" | "h");
            let is_paint = matches!(
                name.as_str(),
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n"
            );
            if is_path || name == "W" || name == "W*" {
                clip |= !is_path;
                path.push(o);
                continue;
            }
            if is_paint {
                let removed = self.path(&mut out, std::mem::take(&mut path), o, clip, &gs);
                if removed {
                    changed = true;
                    marked.iter_mut().for_each(|m| m.touched = true);
                }
                clip = false;
                continue;
            }
            // Anything else ends a path that was never painted (not valid, kept as it is).
            out.append(&mut path);
            clip = false;
            match name.as_str() {
                "q" => stack.push(gs.clone()),
                "Q" => {
                    if let Some(g) = stack.pop() {
                        gs = g;
                    }
                }
                "cm" if v.len() == 6 => gs.ctm = mul([v[0], v[1], v[2], v[3], v[4], v[5]], gs.ctm),
                "w" if !v.is_empty() => gs.lw = v[0],
                "BT" => {
                    tm = mx::ID;
                    tlm = mx::ID;
                }
                "Tf" => {
                    if let Some(Object::Name(nm)) = o.operands.first() {
                        gs.font = nm.clone();
                    }
                    if let Some(s) = o.operands.get(1).and_then(|x| x.as_float().ok()) {
                        gs.size = s;
                    }
                }
                "Tc" if !v.is_empty() => gs.tc = v[0],
                "Tw" if !v.is_empty() => gs.tw = v[0],
                "Tz" if !v.is_empty() => gs.th = v[0] / 100.0,
                "TL" if !v.is_empty() => gs.tl = v[0],
                "Ts" if !v.is_empty() => gs.rise = v[0],
                "Td" | "TD" if v.len() >= 2 => {
                    if name == "TD" {
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
                    let font = font_cache
                        .entry(gs.font.clone())
                        .or_insert_with(|| {
                            let d = fonts.get(&gs.font).ok()?;
                            let (_, d) = pdf.doc.dereference(d).ok()?;
                            d.as_dict().ok().map(|d| FontCtx::new(&pdf.doc, d))
                        })
                        .as_ref();
                    if let Some(new) = self.show(&o, &mut gs, &mut tm, &mut tlm, font) {
                        changed = true;
                        marked.iter_mut().for_each(|m| m.touched = true);
                        out.extend(new);
                        continue;
                    }
                }
                "BDC" | "BMC" => {
                    marked.push(Marked {
                        at: out.len(),
                        touched: false,
                    });
                }
                "EMC" => {
                    if let Some(m) = marked.pop() {
                        if m.touched {
                            if let Some(b) = out.get_mut(m.at) {
                                changed |= self.strip_alt(b, &props, nesting);
                            }
                        }
                    }
                }
                "sh" => {
                    let clips = self.clip_out(gs.ctm, None);
                    if !clips.is_empty() {
                        out.push(op("q", vec![]));
                        out.extend(clips);
                        out.push(o);
                        out.push(op("Q", vec![]));
                        self.out.clipped += 1;
                        changed = true;
                        continue;
                    }
                }
                "Do" => {
                    let Some(Object::Name(nm)) = o.operands.first().cloned() else {
                        out.push(o);
                        continue;
                    };
                    let entry = xobjs.get(&nm).ok().cloned();
                    let id = entry.as_ref().and_then(|e| e.as_reference().ok());
                    let stream = entry
                        .as_ref()
                        .and_then(|e| pdf.doc.dereference(e).ok())
                        .and_then(|(_, s)| s.as_stream().ok());
                    let Some(s) = stream else {
                        out.push(o);
                        continue;
                    };
                    let sub = s
                        .dict
                        .get(b"Subtype")
                        .and_then(|o| o.as_name())
                        .unwrap_or(b"");
                    match sub {
                        b"Image" if self.hit(mx::transform_rect(gs.ctm, UNIT)) => {
                            let cut = self.cut_image(s, &spaces, gs.ctm);
                            if !matches!(cut, Ok(None)) {
                                changed = true;
                                marked.iter_mut().for_each(|m| m.touched = true);
                            }
                            match cut {
                                Ok(None) => {}
                                Ok(Some(new)) => {
                                    let fresh = fresh_name(&xobjs);
                                    xobjs.set(fresh.clone(), Object::Reference(new));
                                    xobjs_changed = true;
                                    unnamed.insert(nm);
                                    self.dropped.extend(id);
                                    self.out.images_cut += 1;
                                    out.push(op("Do", vec![Object::Name(fresh)]));
                                    continue;
                                }
                                Err(_) => {
                                    self.out.images += 1;
                                    unnamed.insert(nm);
                                    self.dropped.extend(id);
                                    continue;
                                }
                            }
                        }
                        b"Form" => {
                            let bbox = s
                                .dict
                                .get(b"BBox")
                                .ok()
                                .and_then(|o| pdf.rect(o))
                                .unwrap_or([0.0, 0.0, 0.0, 0.0]);
                            let fm = s
                                .dict
                                .get(b"Matrix")
                                .ok()
                                .and_then(|o| pdf.matrix(o))
                                .unwrap_or(mx::ID);
                            let fctm = mul(fm, gs.ctm);
                            if self.hit(mx::transform_rect(fctm, bbox)) {
                                let redone = if nesting + 1 < MAX_NESTING {
                                    self.form(s, &res, fctm, nesting + 1)?
                                } else {
                                    None
                                };
                                let Some(new) = redone.map(|f| f.map(Object::Reference)) else {
                                    // Too deep, or unreadable: the whole form goes.
                                    self.out.images += 1;
                                    unnamed.insert(nm);
                                    self.dropped.extend(id);
                                    changed = true;
                                    marked.iter_mut().for_each(|m| m.touched = true);
                                    continue;
                                };
                                if let Some(new) = new {
                                    let fresh = fresh_name(&xobjs);
                                    xobjs.set(fresh.clone(), new);
                                    xobjs_changed = true;
                                    unnamed.insert(nm);
                                    self.dropped.extend(id);
                                    changed = true;
                                    marked.iter_mut().for_each(|m| m.touched = true);
                                    out.push(op("Do", vec![Object::Name(fresh)]));
                                    continue;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                t if inline.raw(t).is_some() && self.hit(mx::transform_rect(gs.ctm, UNIT)) => {
                    // An inline image becomes a picture object, cut like any other.
                    let pic = inline.raw(t).and_then(|raw| inline_stream(raw, &spaces));
                    let cut = match &pic {
                        Some(s) => self.cut_image(s, &spaces, gs.ctm),
                        None => Err("unreadable inline image".into()),
                    };
                    if let Ok(None) = cut {
                        out.push(o);
                        continue;
                    }
                    changed = true;
                    marked.iter_mut().for_each(|m| m.touched = true);
                    if let Ok(Some(new)) = cut {
                        let fresh = fresh_name(&xobjs);
                        xobjs.set(fresh.clone(), Object::Reference(new));
                        xobjs_changed = true;
                        self.out.images_cut += 1;
                        out.push(op("Do", vec![Object::Name(fresh)]));
                    } else {
                        self.out.images += 1;
                    }
                    continue;
                }
                _ => {}
            }
            out.push(o);
        }
        out.append(&mut path);

        // Names no drawing uses any more leave the resources, so what they point at isn't
        // kept in the file through this page.
        let used: HashSet<&[u8]> = out
            .iter()
            .filter(|o| o.operator == "Do")
            .filter_map(|o| o.operands.first()?.as_name().ok())
            .collect();
        for nm in &unnamed {
            if !used.contains(nm.as_slice()) {
                xobjs.remove(nm);
                xobjs_changed = true;
            }
        }
        if xobjs_changed {
            res.set("XObject", xobjs);
        }
        Ok(Rewritten {
            ops: out,
            inline,
            res,
            depth: stack.len(),
            changed,
        })
    }

    /// Redact inside a form XObject drawn with `ctm`. `Ok(None)`: the form can't be read
    /// (so it has to go whole); `Ok(Some(None))`: nothing in it was under a mark;
    /// `Ok(Some(Some(id)))`: the redacted copy.
    fn form(
        &mut self,
        s: &Stream,
        res: &Dictionary,
        ctm: M,
        nesting: usize,
    ) -> Result<Option<Option<ObjectId>>> {
        let Ok(data) = s.get_plain_content_with_limit(crate::MAX_CONTENT) else {
            return Ok(None);
        };
        let own = s
            .dict
            .get(b"Resources")
            .ok()
            .map(|o| self.pdf.resolved_dict(Some(o)));
        let inner = match self.content(&data, own.as_ref().unwrap_or(res), ctm, nesting) {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };
        if !inner.changed {
            return Ok(Some(None));
        }
        let mut ops = inner.ops;
        ops.extend((0..inner.depth).map(|_| op("Q", vec![])));
        let Ok(bytes) = inner.inline.encode(ops) else {
            return Ok(None);
        };
        let mut dict = s.dict.clone();
        for k in [&b"Filter"[..], b"DecodeParms", b"Length"] {
            dict.remove(k);
        }
        dict.set("Resources", inner.res);
        let mut stream = Stream::new(dict, bytes);
        let _ = stream.compress();
        let id = self.alloc();
        self.added.push((id, Object::Stream(stream)));
        Ok(Some(Some(id)))
    }

    /// Redact one painted path (`path` then `paint`). Returns whether anything changed.
    fn path(
        &mut self,
        out: &mut Vec<Operation>,
        path: Vec<Operation>,
        mut paint: Operation,
        clip: bool,
        g: &Gs,
    ) -> bool {
        // A clipping path only limits what is drawn: it is left as it is.
        if clip || paint.operator == "n" || path.is_empty() {
            out.extend(path);
            out.push(paint);
            return false;
        }
        let stroke = matches!(paint.operator.as_str(), "S" | "s" | "B" | "B*" | "b" | "b*");
        let c = g.ctm;
        let sw = if stroke {
            g.lw.abs() * c[0].hypot(c[1]).max(c[2].hypot(c[3])) / 2.0
        } else {
            0.0
        };
        // Sub-paths: each starts at `m` or `re`; anything before the first joins it.
        let mut subs: Vec<Vec<Operation>> = Vec::new();
        for o in path {
            if matches!(o.operator.as_str(), "m" | "re") || subs.is_empty() {
                subs.push(Vec::new());
            }
            if let Some(s) = subs.last_mut() {
                s.push(o);
            }
        }
        let boxes: Vec<[f32; 4]> = subs
            .iter()
            .map(|s| {
                let mut pts = Vec::new();
                for o in s {
                    let v = nums(o);
                    match o.operator.as_str() {
                        "re" if v.len() == 4 => pts.extend([
                            (v[0], v[1]),
                            (v[0] + v[2], v[1]),
                            (v[0] + v[2], v[1] + v[3]),
                            (v[0], v[1] + v[3]),
                        ]),
                        _ => pts.extend(v.as_chunks::<2>().0.iter().map(|&[x, y]| (x, y))),
                    }
                }
                let b = mx::aabb(pts.into_iter().map(|(x, y)| apply(c, x, y)));
                [b[0] - sw, b[1] - sw, b[2] + sw, b[3] + sw]
            })
            .collect();
        let mut keep = Vec::new();
        let mut removed = 0;
        let mut crossing: Option<[f32; 4]> = None;
        let last = subs.len() - 1;
        let mut last_removed = false;
        for (i, (s, b)) in subs.into_iter().zip(boxes).enumerate() {
            if !b.iter().all(|x| x.is_finite()) || !self.hit(b) {
                keep.push(s);
            } else if share_under(b, &self.areas) >= MOSTLY {
                removed += 1;
                last_removed |= i == last;
            } else {
                crossing = Some(crossing.map_or(b, |c| {
                    [
                        c[0].min(b[0]),
                        c[1].min(b[1]),
                        c[2].max(b[2]),
                        c[3].max(b[3]),
                    ]
                }));
                keep.push(s);
            }
        }
        if removed == 0 && crossing.is_none() {
            out.extend(keep.into_iter().flatten());
            out.push(paint);
            return false;
        }
        self.out.paths += removed;
        if keep.is_empty() {
            return true;
        }
        // `s` and `b` close the last sub-path; if it went, nothing else is to be closed.
        if last_removed {
            let open = match paint.operator.as_str() {
                "s" => Some("S"),
                "b" => Some("B"),
                "b*" => Some("B*"),
                _ => None,
            };
            if let Some(n) = open {
                paint = op(n, vec![]);
            }
        }
        let clips = crossing
            .map(|b| self.clip_out(c, Some(b)))
            .unwrap_or_default();
        let wrap = !clips.is_empty();
        if wrap {
            self.out.clipped += 1;
            out.push(op("q", vec![]));
            out.extend(clips);
        }
        out.extend(keep.into_iter().flatten());
        out.push(paint);
        if wrap {
            out.push(op("Q", vec![]));
        }
        true
    }

    /// Redact one text-showing operator. `None` when nothing in it is under a mark (it
    /// stays as it is); otherwise the operators that replace it.
    fn show(
        &mut self,
        o: &Operation,
        g: &mut Gs,
        tm: &mut M,
        tlm: &mut M,
        font: Option<&FontCtx>,
    ) -> Option<Vec<Operation>> {
        let mut lead: Vec<Operation> = Vec::new();
        let items: Vec<&Object> = match o.operator.as_str() {
            "TJ" => o
                .operands
                .first()
                .and_then(|a| a.as_array().ok())
                .map(|a| a.iter().collect())
                .unwrap_or_default(),
            "\"" => {
                let aw = o
                    .operands
                    .first()
                    .and_then(|x| x.as_float().ok())
                    .unwrap_or(0.0);
                let ac = o
                    .operands
                    .get(1)
                    .and_then(|x| x.as_float().ok())
                    .unwrap_or(0.0);
                g.tw = aw;
                g.tc = ac;
                lead.extend([op("Tw", vec![r(aw)]), op("Tc", vec![r(ac)])]);
                o.operands.iter().skip(2).collect()
            }
            _ => o.operands.iter().collect(),
        };
        if matches!(o.operator.as_str(), "'" | "\"") {
            *tlm = mul(mx::translate(0.0, -g.tl), *tlm);
            *tm = *tlm;
            lead.push(op("T*", vec![]));
        }
        let start = *tm;
        let m = mul(start, g.ctm);
        let (tfs, th) = (g.size, g.th);
        let scale = tfs * th;
        let (d0, a0) = (-0.21 * tfs + g.rise, 0.79 * tfs + g.rise);
        let known = font.is_some_and(|f| f.codes_known);

        // Each character code: its bytes, its text, where it starts and how far it moves.
        struct Code {
            bytes: Vec<u8>,
            text: String,
            at: f32,
            glyph: f32,
            adv: f32,
        }
        let mut adv = 0.0f32;
        // Strings as their codes, spacing numbers as `Err`.
        let mut parts: Vec<std::result::Result<Vec<Code>, f32>> = Vec::new();
        for it in &items {
            match it {
                Object::String(s, _) => {
                    let mut codes = Vec::new();
                    match font {
                        Some(f) => {
                            let step = if f.type0 { 2 } else { 1 };
                            for (k, (code, space)) in f.codes(s).into_iter().enumerate() {
                                let raw = &s[k * step..(k * step + step).min(s.len())];
                                let glyph = f.width(code) / 1000.0 * scale;
                                let a = glyph + (g.tc + if space { g.tw } else { 0.0 }) * th;
                                codes.push(Code {
                                    bytes: raw.to_vec(),
                                    text: f.decode_code(code, raw),
                                    at: adv,
                                    glyph,
                                    adv: a,
                                });
                                adv += a;
                            }
                        }
                        None => {
                            for &b in s.iter() {
                                let glyph = 0.5 * scale;
                                let a = glyph + (g.tc + if b == 32 { g.tw } else { 0.0 }) * th;
                                codes.push(Code {
                                    bytes: vec![b],
                                    text: (b as char).to_string(),
                                    at: adv,
                                    glyph,
                                    adv: a,
                                });
                                adv += a;
                            }
                        }
                    }
                    parts.push(Ok(codes));
                }
                x => {
                    let n = x.as_float().unwrap_or(0.0);
                    adv -= n / 1000.0 * scale;
                    parts.push(Err(n));
                }
            }
        }
        *tm = mul(mx::translate(adv, 0.0), start);
        let glyph_box = |at: f32, w: f32| mx::transform_rect(m, norm([at, d0, at + w, a0]));

        if !known {
            // The string can't be split into characters reliably: if any of it is under
            // a mark, all of it goes, keeping its advance. The box is widened, as the
            // widths may be wrong.
            let w = adv.abs().max(0.5 * scale.abs());
            let b = norm([-w * 0.5, d0, adv + w * 0.5, a0]);
            if !self.hit(mx::transform_rect(m, b)) {
                return None;
            }
            for c in parts.iter().flatten().flatten() {
                self.removed_char(&c.text);
            }
            if scale.abs() > 1e-6 {
                lead.push(op(
                    "TJ",
                    vec![Object::Array(vec![r(-adv / scale * 1000.0)])],
                ));
            }
            return Some(lead);
        }

        let gone: Vec<Vec<bool>> = parts
            .iter()
            .map(|p| match p {
                Ok(codes) => codes
                    .iter()
                    .map(|c| self.char_hit(glyph_box(c.at, c.glyph)))
                    .collect(),
                Err(_) => Vec::new(),
            })
            .collect();
        if !gone.iter().flatten().any(|&x| x) {
            return None;
        }

        // Rebuild as one TJ: kept characters in their strings, removed ones as spacing.
        let mut arr: Vec<Object> = Vec::new();
        let mut pending = 0.0f32;
        let flush = |pending: &mut f32, arr: &mut Vec<Object>| {
            if pending.abs() > 1e-4 {
                arr.push(r(*pending));
            }
            *pending = 0.0;
        };
        for ((part, gone), item) in parts.iter().zip(&gone).zip(&items) {
            match part {
                Err(n) => pending += n,
                Ok(codes) => {
                    let fmt = match item {
                        Object::String(_, f) => *f,
                        _ => lopdf::StringFormat::Literal,
                    };
                    let mut keep: Vec<u8> = Vec::new();
                    for (c, &x) in codes.iter().zip(gone) {
                        if x {
                            if !keep.is_empty() {
                                arr.push(Object::String(std::mem::take(&mut keep), fmt));
                            }
                            self.removed_char(&c.text);
                            if scale.abs() > 1e-6 {
                                pending -= c.adv / scale * 1000.0;
                            }
                        } else {
                            if keep.is_empty() {
                                flush(&mut pending, &mut arr);
                            }
                            keep.extend_from_slice(&c.bytes);
                        }
                    }
                    if !keep.is_empty() {
                        arr.push(Object::String(keep, fmt));
                    }
                }
            }
        }
        flush(&mut pending, &mut arr);
        if !arr.is_empty() {
            lead.push(op("TJ", vec![Object::Array(arr)]));
        }
        Some(lead)
    }

    /// Take the alternative text off a marked-content operator whose content lost
    /// something. Returns whether it changed.
    fn strip_alt(&mut self, b: &mut Operation, props: &Dictionary, nesting: usize) -> bool {
        if b.operator != "BDC" {
            return false;
        }
        let dict = match b.operands.get(1) {
            Some(Object::Dictionary(d)) => d.clone(),
            Some(Object::Name(n)) => self.pdf.resolved_dict(props.get(n).ok()),
            _ => return false,
        };
        if nesting == 0 {
            if let Ok(id) = dict.get(b"MCID").and_then(|o| o.as_i64()) {
                self.mcids.insert(id);
            }
        }
        const ALT: [&[u8]; 3] = [b"ActualText", b"Alt", b"E"];
        if !ALT.iter().any(|k| dict.has(k)) {
            return false;
        }
        let mut d = dict;
        for k in ALT {
            d.remove(k);
        }
        b.operands[1] = Object::Dictionary(d);
        true
    }
}

impl Pdf {
    pub(crate) fn matrix(&self, o: &Object) -> Option<M> {
        let (_, o) = self.doc.dereference(o).ok()?;
        let a = o.as_array().ok()?;
        let v: Vec<f32> = a
            .iter()
            .take(6)
            .map(|x| self.num(x))
            .collect::<Option<_>>()?;
        (v.len() == 6).then(|| [v[0], v[1], v[2], v[3], v[4], v[5]])
    }

    /// The areas the document's redaction marks cover: (page, box in displayed-page
    /// points, y down), one per marked area.
    pub fn redaction_marks(&self) -> Vec<(usize, [f32; 4])> {
        self.annotations()
            .into_iter()
            .filter(|a| a.kind == crate::AnnotKind::Redact)
            .flat_map(|a| {
                let boxes = if a.quads.is_empty() {
                    vec![a.bbox]
                } else {
                    a.quads
                };
                boxes.into_iter().map(move |b| (a.page, b))
            })
            .collect()
    }

    /// Apply the document's redaction marks: remove everything under them as
    /// [`Pdf::redact`] does, then remove the marks.
    pub fn apply_redactions(&mut self) -> Result<RedactReport> {
        let marks = self.redaction_marks();
        if marks.is_empty() {
            bail!("the document has no redaction marks");
        }
        let report = self.redact(&marks)?;
        let ids = self
            .annotations()
            .into_iter()
            .filter(|a| a.kind == crate::AnnotKind::Redact)
            .map(|a| a.id)
            .collect();
        self.drop_annotations(ids)?;
        Ok(report)
    }

    /// Marks covering every match of `query` (as [`Pdf::find_text_matches`] finds them):
    /// (page, box in displayed-page points, y down), one box per line of each match.
    pub fn find_redaction_marks(&self, query: &str) -> Result<Vec<(usize, [f32; 4])>> {
        let mut out = Vec::new();
        for p in 0..self.page_count() {
            for m in self.find_text_matches(p, query)? {
                out.extend(m.into_iter().map(|b| (p, b)));
            }
        }
        Ok(out)
    }

    /// Remove everything under the marks: (page, box in displayed-page points, y down).
    /// Text, drawings and pictures are taken out of the page content (not covered), as
    /// are annotations touching a mark; then each mark is painted black. Afterwards the
    /// pages are read again to check that nothing readable is left under the marks, and
    /// the removed text is looked for elsewhere in the document. The old content is gone
    /// from the file once it is saved.
    pub fn redact(&mut self, marks: &[(usize, [f32; 4])]) -> Result<RedactReport> {
        let n = self.page_count();
        let mut by_page: BTreeMap<usize, Vec<[f32; 4]>> = BTreeMap::new();
        for &(p, b) in marks {
            if p >= n {
                bail!("there is no page {}", p + 1);
            }
            let b = norm(b);
            if !b.iter().all(|x| x.is_finite()) || b[2] - b[0] < EPS || b[3] - b[1] < EPS {
                bail!("a mark on page {} has no size", p + 1);
            }
            by_page.entry(p).or_default().push(b);
        }
        let mut report = RedactReport::default();
        let mut dropped = HashSet::new();
        for (&p, rects) in &by_page {
            let page = self.page_id_at(p)?;
            let (vd, _, _) = self.vis_down(page);
            let areas: Vec<[f32; 4]> = rects.iter().map(|&b| mx::transform_rect(vd, b)).collect();
            let data = self
                .doc
                .get_page_content_with_limit(page, crate::MAX_CONTENT)?;
            let res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
            let text = self.text_under(p, &areas);
            let mut rd = Redactor::new(self, areas.clone());
            let done = rd.content(&data, &res, mx::ID, 0)?;
            let Redactor {
                added,
                dropped: gone,
                mcids,
                mut out,
                next_id,
                ..
            } = rd;

            for (id, obj) in added {
                self.doc.objects.insert(id, obj);
            }
            self.doc.max_id = self.doc.max_id.max(next_id - 1);
            // The old content in q/Q, so none of its state reaches the black boxes.
            let mut ops = Vec::with_capacity(done.ops.len() + 8);
            ops.push(op("q", vec![]));
            ops.extend(done.ops);
            ops.extend((0..=done.depth).map(|_| op("Q", vec![])));
            ops.extend([op("q", vec![]), op("g", vec![r(0.0)])]);
            for a in &areas {
                ops.push(op(
                    "re",
                    vec![r(a[0]), r(a[1]), r(a[2] - a[0]), r(a[3] - a[1])],
                ));
            }
            ops.extend([op("f", vec![]), op("Q", vec![])]);
            self.commit(page, ops, 0, &done.inline)?;
            let pd = self.doc.get_dictionary_mut(page)?;
            pd.set("Resources", done.res);
            pd.remove(b"Thumb");
            pd.remove(b"PieceInfo");
            self.strip_struct_alt(page, &mcids);
            out.annots = self.redact_annots(page, &areas)?;
            out.page = p;
            out.text = text;
            dropped.extend(gone);
            report.pages.push(out);
        }
        self.drop_unused_xobjects(&dropped)?;
        report.left = self.check_left(&by_page);
        report.elsewhere = self.find_elsewhere(&report);
        Ok(report)
    }

    /// The text of page `p` under `areas` (page user space), as the page is read (so with
    /// the spaces search sees): one entry per run of characters on a line.
    fn text_under(&self, p: usize, areas: &[[f32; 4]]) -> Vec<String> {
        let Ok(parsed) = self.parse_page(p) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for line in parsed.text_lines() {
            let mut run = String::new();
            for (i, c) in line.text.chars().enumerate() {
                if c.is_whitespace() {
                    if !run.is_empty() && !run.ends_with(' ') {
                        run.push(' ');
                    }
                } else if span_boxes(&line.pieces, i, i + 1)
                    .into_iter()
                    .any(|b| char_under(norm(b), areas))
                {
                    run.push(c);
                } else if !run.trim().is_empty() {
                    out.push(run.trim().to_string());
                    run.clear();
                } else {
                    run.clear();
                }
            }
            if !run.trim().is_empty() {
                out.push(run.trim().to_string());
            }
        }
        out
    }

    /// Remove the annotations on `page` touching any of `areas` (page user space).
    /// Returns how many went, replies included.
    fn redact_annots(&mut self, page: ObjectId, areas: &[[f32; 4]]) -> Result<usize> {
        let mut gone = HashSet::new();
        let mut widgets = HashSet::new();
        for id in self.annot_refs(page) {
            let Ok(d) = self.doc.get_dictionary(id) else {
                continue;
            };
            let sub = d.get(b"Subtype").and_then(|o| o.as_name()).unwrap_or(b"");
            if sub == b"Redact" {
                continue;
            }
            let Some(b) = d.get(b"Rect").ok().and_then(|o| self.rect(o)) else {
                continue;
            };
            if areas.iter().any(|&a| touches(norm(b), a)) {
                gone.insert(id);
                if sub == b"Widget" {
                    widgets.insert(id);
                }
            }
        }
        if gone.is_empty() {
            return Ok(0);
        }
        let n = self.drop_annotations(gone)?;
        if !widgets.is_empty() {
            self.prune_fields(&widgets)?;
        }
        Ok(n)
    }

    /// Take the alternative text off the structure elements (and their parents) of the
    /// page's marked content `mcids`.
    fn strip_struct_alt(&mut self, page: ObjectId, mcids: &HashSet<i64>) {
        if mcids.is_empty() {
            return;
        }
        let key = self
            .doc
            .get_dictionary(page)
            .ok()
            .and_then(|d| d.get(b"StructParents").ok())
            .and_then(|o| o.as_i64().ok());
        let tree = self
            .doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"StructTreeRoot").ok())
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .and_then(|d| d.get(b"ParentTree").ok())
            .cloned();
        let (Some(key), Some(tree)) = (key, tree) else {
            return;
        };
        let Some(list) = self.number_tree_get(&tree, key, 0) else {
            return;
        };
        let list = self
            .doc
            .dereference(&list)
            .ok()
            .and_then(|(_, o)| o.as_array().ok())
            .cloned()
            .unwrap_or_default();
        let mut elems = HashSet::new();
        for &mcid in mcids {
            let mut cur = usize::try_from(mcid)
                .ok()
                .and_then(|i| list.get(i))
                .and_then(|o| o.as_reference().ok());
            for _ in 0..64 {
                let Some(id) = cur else { break };
                if !elems.insert(id) {
                    break;
                }
                let Ok(d) = self.doc.get_dictionary_mut(id) else {
                    break;
                };
                if d.get(b"Type").and_then(|o| o.as_name()).ok() == Some(b"StructTreeRoot") {
                    break;
                }
                for k in [&b"ActualText"[..], b"Alt", b"E"] {
                    d.remove(k);
                }
                cur = d.get(b"P").and_then(|o| o.as_reference()).ok();
            }
        }
    }

    fn number_tree_get(&self, node: &Object, key: i64, depth: usize) -> Option<Object> {
        if depth > 32 {
            return None;
        }
        let d = self.doc.dereference(node).ok()?.1.as_dict().ok()?;
        if let Ok(nums) = d.get(b"Nums").and_then(|o| o.as_array()) {
            for [k, v] in nums.as_chunks::<2>().0 {
                if k.as_i64().ok() == Some(key) {
                    return Some(v.clone());
                }
            }
        }
        let kids = d.get(b"Kids").ok()?;
        for kid in self.doc.dereference(kids).ok()?.1.as_array().ok()? {
            let kd = self.doc.dereference(kid).ok()?.1.as_dict().ok()?;
            let inside = kd
                .get(b"Limits")
                .and_then(|o| o.as_array())
                .ok()
                .and_then(|l| Some((l.first()?.as_i64().ok()?, l.get(1)?.as_i64().ok()?)))
                .is_none_or(|(lo, hi)| lo <= key && key <= hi);
            if inside {
                if let Some(v) = self.number_tree_get(kid, key, depth + 1) {
                    return Some(v);
                }
            }
        }
        None
    }

    /// Of the pictures and forms `dropped`, take those no page draws any more out of the
    /// resources of pages and page-tree nodes, so they aren't kept in the file.
    pub(crate) fn drop_unused_xobjects(&mut self, dropped: &HashSet<ObjectId>) -> Result<()> {
        if dropped.is_empty() {
            return Ok(());
        }
        let mut used = HashSet::new();
        let mut holders = Vec::new();
        for page in self.page_ids() {
            let res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
            if let Ok(data) = self
                .doc
                .get_page_content_with_limit(page, crate::MAX_CONTENT)
            {
                self.xobjects_drawn(&data, &res, 0, &mut used);
            }
            let mut cur = Some(page);
            while let Some(id) = cur {
                if holders.contains(&id) || holders.len() > 100_000 {
                    break;
                }
                holders.push(id);
                cur = self
                    .doc
                    .get_dictionary(id)
                    .ok()
                    .and_then(|d| d.get(b"Parent").ok())
                    .and_then(|o| o.as_reference().ok());
            }
        }
        let unused: HashSet<ObjectId> = dropped.difference(&used).copied().collect();
        if unused.is_empty() {
            return Ok(());
        }
        for h in holders {
            self.unlist_xobjects(h, &unused)?;
        }
        Ok(())
    }

    /// Collect the pictures and forms a content stream draws, forms' insides included.
    fn xobjects_drawn(
        &self,
        data: &[u8],
        res: &Dictionary,
        depth: usize,
        used: &mut HashSet<ObjectId>,
    ) {
        let Ok((data, _)) = crate::inline::extract(data) else {
            return;
        };
        let Ok(content) = Content::decode(&data) else {
            return;
        };
        let xobjs = self.resolved_dict(res.get(b"XObject").ok());
        for o in content.operations.iter().filter(|o| o.operator == "Do") {
            let Some(id) = o
                .operands
                .first()
                .and_then(|n| n.as_name().ok())
                .and_then(|n| xobjs.get(n).ok())
                .and_then(|e| e.as_reference().ok())
            else {
                continue;
            };
            if !used.insert(id) || depth >= MAX_NESTING {
                continue;
            }
            if let Ok(s) = self.doc.get_object(id).and_then(|o| o.as_stream()) {
                if s.dict.get(b"Subtype").and_then(|o| o.as_name()).ok() == Some(b"Form") {
                    let own = s
                        .dict
                        .get(b"Resources")
                        .ok()
                        .map(|o| self.resolved_dict(Some(o)));
                    if let Ok(c) = s.get_plain_content_with_limit(crate::MAX_CONTENT) {
                        self.xobjects_drawn(&c, own.as_ref().unwrap_or(res), depth + 1, used);
                    }
                }
            }
        }
    }

    /// Remove the `/XObject` entries pointing at `gone` from `holder`'s own resources.
    fn unlist_xobjects(&mut self, holder: ObjectId, gone: &HashSet<ObjectId>) -> Result<()> {
        let Ok(hd) = self.doc.get_dictionary(holder) else {
            return Ok(());
        };
        let (res_id, mut res) = match hd.get(b"Resources") {
            Ok(Object::Reference(id)) => match self.doc.get_dictionary(*id) {
                Ok(d) => (Some(*id), d.clone()),
                Err(_) => return Ok(()),
            },
            Ok(Object::Dictionary(d)) => (None, d.clone()),
            _ => return Ok(()),
        };
        let (xo_id, mut xo) = match res.get(b"XObject") {
            Ok(Object::Reference(id)) => match self.doc.get_dictionary(*id) {
                Ok(d) => (Some(*id), d.clone()),
                Err(_) => return Ok(()),
            },
            Ok(Object::Dictionary(d)) => (None, d.clone()),
            _ => return Ok(()),
        };
        let names: Vec<Vec<u8>> = xo
            .iter()
            .filter(|(_, v)| v.as_reference().is_ok_and(|id| gone.contains(&id)))
            .map(|(k, _)| k.clone())
            .collect();
        if names.is_empty() {
            return Ok(());
        }
        for k in &names {
            xo.remove(k);
        }
        match xo_id {
            Some(id) => *self.doc.get_dictionary_mut(id)? = xo,
            None => {
                res.set("XObject", xo);
                match res_id {
                    Some(id) => *self.doc.get_dictionary_mut(id)? = res,
                    None => self.doc.get_dictionary_mut(holder)?.set("Resources", res),
                }
            }
        }
        Ok(())
    }

    /// Read the redacted pages again: any character or picture still touching a mark.
    fn check_left(&self, by_page: &BTreeMap<usize, Vec<[f32; 4]>>) -> Vec<String> {
        let mut left = Vec::new();
        for (&p, rects) in by_page {
            let Ok(page) = self.page_id_at(p) else {
                continue;
            };
            let (vd, _, _) = self.vis_down(page);
            let areas: Vec<[f32; 4]> = rects.iter().map(|&b| mx::transform_rect(vd, b)).collect();
            let under = |b: [f32; 4]| areas.iter().any(|&a| touches(norm(b), a));
            let parsed = match self.parse_page(p) {
                Ok(x) => x,
                Err(e) => {
                    left.push(format!(
                        "page {}: the page can't be read to check it: {e}",
                        p + 1
                    ));
                    continue;
                }
            };
            let mut found = String::new();
            for line in parsed.text_lines() {
                for (i, c) in line.text.chars().enumerate() {
                    if !c.is_whitespace()
                        && span_boxes(&line.pieces, i, i + 1)
                            .into_iter()
                            .any(|b| char_under(norm(b), &areas))
                    {
                        found.push(c);
                    }
                }
            }
            if !found.is_empty() {
                left.push(format!(
                    "page {}: text is still under a mark: {found}",
                    p + 1
                ));
            }
            // Pictures under a mark must have been blacked out there.
            let res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
            let xobjs = self.resolved_dict(res.get(b"XObject").ok());
            let spaces = self.resolved_dict(res.get(b"ColorSpace").ok());
            let mut pics = 0;
            for o in &parsed.objs {
                if o.kind != crate::ObjectKind::Image || !under(o.bbox) {
                    continue;
                }
                let Some(draw) = parsed.ops.get(o.first) else {
                    continue;
                };
                let pic = match parsed.inline.raw(&draw.operator) {
                    Some(raw) => inline_stream(raw, &spaces),
                    None => draw
                        .operands
                        .first()
                        .and_then(|n| n.as_name().ok())
                        .and_then(|n| xobjs.get(n).ok())
                        .and_then(|e| self.doc.dereference(e).ok())
                        .and_then(|(_, s)| s.as_stream().ok())
                        .cloned(),
                };
                let clear = pic.is_some_and(|s| {
                    is_clear(&self.doc, &s, &spaces, o.ctm, &areas).unwrap_or(false)
                });
                if !clear {
                    pics += 1;
                }
            }
            if pics > 0 {
                left.push(format!(
                    "page {}: {pics} picture(s) still show something under a mark",
                    p + 1
                ));
            }
        }
        left
    }

    /// Look for the removed text elsewhere: on any page, in bookmarks, comments, form
    /// fields and the document information.
    fn find_elsewhere(&self, report: &RedactReport) -> Vec<(String, Vec<String>)> {
        let mut queries: Vec<String> = Vec::new();
        let mut add = |q: String| {
            if q.chars().filter(|c| !c.is_whitespace()).count() >= 2 && !queries.contains(&q) {
                queries.push(q);
            }
        };
        for t in report.pages.iter().flat_map(|p| &p.text) {
            let q = fold(t);
            let words: Vec<&str> = q.split(' ').collect();
            if (2..=3).contains(&words.len()) {
                for w in &words {
                    if w.chars().count() >= 4 {
                        add(w.to_string());
                    }
                }
            }
            add(q);
        }
        if queries.is_empty() {
            return Vec::new();
        }
        let pages: Vec<String> = (0..self.page_count())
            .into_par_iter()
            .map(|p| fold(&self.page_text(p)))
            .collect();
        let mut other: Vec<(String, String)> = Vec::new();
        for b in self.outline() {
            other.push((
                format!("bookmark \u{201c}{}\u{201d}", b.title),
                fold(&b.title),
            ));
        }
        for a in self.annotations() {
            let all = format!("{} {} {}", a.contents, a.author, a.subject);
            other.push((format!("a comment on page {}", a.page + 1), fold(&all)));
        }
        for f in self.form_fields() {
            if let crate::FieldValue::Text(v) = &f.value {
                other.push((format!("form field \u{201c}{}\u{201d}", f.name), fold(v)));
            }
        }
        let info = self.info();
        for (what, v) in [
            ("document title", &info.title),
            ("document author", &info.author),
            ("document subject", &info.subject),
            ("document keywords", &info.keywords),
        ] {
            if let Some(v) = v {
                other.push((what.to_string(), fold(v)));
            }
        }
        let mut out = Vec::new();
        for q in queries {
            let mut places: Vec<String> = pages
                .iter()
                .enumerate()
                .filter(|(_, t)| t.contains(&q))
                .map(|(i, _)| format!("page {}", i + 1))
                .collect();
            for (what, t) in &other {
                if t.contains(&q) && !places.contains(what) {
                    places.push(what.clone());
                }
            }
            if !places.is_empty() {
                places.truncate(MAX_PLACES);
                out.push((q, places));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touching_needs_real_overlap() {
        let m = [10.0, 10.0, 20.0, 20.0];
        assert!(touches([15.0, 15.0, 30.0, 30.0], m));
        assert!(
            !touches([20.0, 10.0, 30.0, 20.0], m),
            "sharing an edge is not touching"
        );
        assert!(!touches([20.02, 10.0, 30.0, 20.0], m));
        assert!(
            touches([15.0, 12.0, 15.0, 18.0], m),
            "a box with no width inside"
        );
        assert!(!touches([25.0, 12.0, 25.0, 18.0], m));
    }

    #[test]
    fn share_of_a_box_under_marks() {
        let m = [0.0, 0.0, 10.0, 10.0];
        assert_eq!(share_under([0.0, 0.0, 10.0, 10.0], &[m]), 1.0);
        assert!((share_under([5.0, 0.0, 15.0, 10.0], &[m]) - 0.5).abs() < 1e-6);
        assert_eq!(
            share_under([2.0, 5.0, 4.0, 5.0], &[m]),
            1.0,
            "a flat line inside"
        );
    }
}
