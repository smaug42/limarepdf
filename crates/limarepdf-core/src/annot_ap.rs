//! Drawing appearance streams for annotations that have none, from their entries: text
//! markup, notes, text boxes, shapes, lines and freehand ink. Shapes are drawn in default
//! user space, so their appearance's box is the annotation's `/Rect`; text boxes are laid
//! out from their own corner like form fields.

use crate::form_ap::{self, encode, fill_color, nums, op, stroke_color, Body, Border, Look};
use crate::form_write::{parse_da, standard_metrics};
use crate::Pdf;
use lopdf::content::Operation;
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

/// The subtypes LimarePDF can draw.
pub(crate) const DRAWN: [&[u8]; 13] = [
    b"Redact",
    b"Highlight",
    b"Underline",
    b"StrikeOut",
    b"Squiggly",
    b"Text",
    b"FreeText",
    b"Square",
    b"Circle",
    b"Line",
    b"Polygon",
    b"PolyLine",
    b"Ink",
];

/// Resource names in the appearances LimarePDF draws.
const FONT: &str = "Helv";
const GS: &str = "LimareGs";
/// Control-point distance for a quarter circle drawn as a Bézier curve.
const KAPPA: f32 = 0.5523;
/// A note's icon: its size in points and its colour when the note has none.
pub(crate) const NOTE_SIZE: f32 = 20.0;
const NOTE_COLOR: [f32; 3] = [1.0, 0.85, 0.2];
/// A redaction mark's outline: its colour when the mark has none, and its width.
pub(crate) const REDACT_COLOR: [f32; 3] = [0.85, 0.0, 0.0];
const REDACT_LINE: f32 = 1.5;
/// Text size for a text box whose `/DA` gives none.
const TEXT_SIZE: f32 = 12.0;
/// Most points read from one geometry entry, against hostile files.
const MAX_POINTS: usize = 100_000;

/// A drawn appearance, before it is put in the file.
struct Drawing {
    content: Vec<u8>,
    bbox: [f32; 4],
    /// Opacity below 1, or the Multiply blend mode for highlights.
    gs: Option<Dictionary>,
    /// Uses Helvetica as [`FONT`].
    font: bool,
    /// Turns the drawing to fit `/Rect`, for text boxes on turned pages.
    matrix: Option<[f32; 6]>,
}

impl Pdf {
    /// Give each of `ids` an appearance (`/AP /N`) drawn from its entries. Annotations
    /// LimarePDF can't draw are left alone. Returns how many were drawn.
    pub(crate) fn draw_annotations(&mut self, ids: &[ObjectId]) -> usize {
        let mut font = None;
        let mut n = 0;
        for &id in ids {
            let Some(d) = self.annotation_drawing(id) else {
                continue;
            };
            let mut res = Dictionary::new();
            if d.font {
                let f = *font.get_or_insert_with(|| {
                    self.doc.add_object(dictionary! {
                        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
                        "Encoding" => "WinAnsiEncoding",
                    })
                });
                res.set("Font", dictionary! { FONT => f });
            }
            if let Some(gs) = d.gs {
                res.set("ExtGState", dictionary! { GS => gs });
            }
            let mut form = dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "BBox" => nums(&d.bbox),
                "Resources" => res,
            };
            if let Some(m) = d.matrix {
                form.set("Matrix", nums(&m));
            }
            let ap = self.doc.add_object(Stream::new(form, d.content));
            if let Ok(a) = self.doc.get_dictionary_mut(id) {
                a.set("AP", dictionary! { "N" => ap });
                n += 1;
            }
        }
        n
    }

    /// The appearance for one annotation, or `None` for a type LimarePDF doesn't draw.
    fn annotation_drawing(&self, id: ObjectId) -> Option<Drawing> {
        let d = self.doc.get_dictionary(id).ok()?;
        let rect = self.rect(d.get(b"Rect").ok()?)?;
        let sub = d.get(b"Subtype").ok()?.as_name().ok()?;
        let c = self.colour(d, b"C");
        let ic = self.colour(d, b"IC");
        let alpha = d
            .get(b"CA")
            .ok()
            .and_then(|o| self.num(o))
            .map_or(1.0, |a| a.clamp(0.0, 1.0));
        let (bw, dash) = self.border(d);
        let line = Pen { c, ic, w: bw, dash };
        let mut ops = Vec::new();
        let mut blend = false;
        match sub {
            b"Highlight" | b"Underline" | b"StrikeOut" | b"Squiggly" => {
                let quads = self
                    .points(d.get(b"QuadPoints").ok())
                    .filter(|p| p.len() >= 4)
                    .map(|p| p.as_chunks::<4>().0.iter().map(orient).collect())
                    .unwrap_or_else(|| vec![orient(&rect_corners(rect))]);
                let default = if sub == b"Highlight" {
                    [1.0, 1.0, 0.0]
                } else {
                    [0.0; 3]
                };
                let c = line.c.unwrap_or_else(|| default.to_vec());
                markup(&mut ops, sub, &quads, &c);
                blend = sub == b"Highlight";
            }
            b"Square" | b"Circle" => {
                let r = self.inner(d, rect);
                let h = bw / 2.0;
                let r = [r[0] + h, r[1] + h, r[2] - h, r[3] - h];
                if r[2] > r[0] && r[3] > r[1] {
                    if sub == b"Square" {
                        ops.push(op("re", nums(&[r[0], r[1], r[2] - r[0], r[3] - r[1]])));
                    } else {
                        ellipse(&mut ops, r);
                    }
                    line.paint(&mut ops, true);
                }
            }
            b"Line" => {
                let p = self.points(d.get(b"L").ok()).filter(|p| p.len() >= 2)?;
                let ends = self.line_ends(d);
                ops.push(op("m", nums(&[p[0].0, p[0].1])));
                ops.push(op("l", nums(&[p[1].0, p[1].1])));
                line.paint(&mut ops, false);
                for (i, end) in ends.iter().enumerate() {
                    let (at, from) = if i == 0 { (p[0], p[1]) } else { (p[1], p[0]) };
                    line_end(&mut ops, end, at, from, &line);
                }
            }
            b"Polygon" | b"PolyLine" => {
                let p = self
                    .points(d.get(b"Vertices").ok())
                    .filter(|p| !p.is_empty())?;
                polyline(&mut ops, &p);
                let closed = sub == b"Polygon";
                if closed {
                    ops.push(op("h", vec![]));
                }
                line.paint(&mut ops, closed);
            }
            b"Ink" => {
                let list = d
                    .get(b"InkList")
                    .ok()
                    .and_then(|o| self.doc.dereference(o).ok())
                    .and_then(|(_, o)| o.as_array().ok())?;
                ops.push(op("J", vec![1.into()]));
                ops.push(op("j", vec![1.into()]));
                for s in list.iter().take(MAX_POINTS) {
                    if let Some(p) = self.points(Some(s)).filter(|p| !p.is_empty()) {
                        polyline(&mut ops, &p);
                        if p.len() == 1 {
                            ops.push(op("l", nums(&[p[0].0, p[0].1])));
                        }
                    }
                }
                line.paint(&mut ops, false);
            }
            b"Redact" => {
                // A red outline around each marked area.
                let quads = self
                    .points(d.get(b"QuadPoints").ok())
                    .filter(|p| p.len() >= 4)
                    .map(|p| p.as_chunks::<4>().0.to_vec())
                    .unwrap_or_else(|| vec![rect_corners(rect)]);
                let c = line.c.clone().unwrap_or_else(|| REDACT_COLOR.to_vec());
                ops.extend(stroke_color(&c));
                ops.push(op("w", nums(&[REDACT_LINE])));
                for q in &quads {
                    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                    for &(x, y) in q {
                        (b[0], b[1]) = (b[0].min(x), b[1].min(y));
                        (b[2], b[3]) = (b[2].max(x), b[3].max(y));
                    }
                    ops.push(op("re", nums(&[b[0], b[1], b[2] - b[0], b[3] - b[1]])));
                }
                ops.push(op("S", vec![]));
            }
            b"Text" => note(&mut ops, rect, line.c.as_deref()),
            b"FreeText" => return Some(self.free_text(d, rect, alpha)),
            _ => return None,
        }
        Some(Drawing {
            content: with_gs(encode(ops), alpha, blend),
            bbox: rect,
            gs: gs_dict(alpha, blend),
            font: false,
            matrix: None,
        })
    }

    /// A text box: its `/Contents` in Helvetica, with the size and colour from `/DA` (or
    /// the `/DS` style string), on the `/C` background with a border.
    fn free_text(&self, d: &Dictionary, rect: [f32; 4], alpha: f32) -> Drawing {
        // `/Rotate` (as Acrobat writes it) turns the text with its page, so it reads
        // upright as the page is shown.
        let turn = d
            .get(b"Rotate")
            .ok()
            .and_then(|o| self.int_value(o))
            .unwrap_or(0)
            .rem_euclid(360);
        let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
        let (w, h, matrix) = match turn {
            90 => (h, w, Some([0.0, 1.0, -1.0, 0.0, 0.0, 0.0])),
            180 => (w, h, Some([-1.0, 0.0, 0.0, -1.0, 0.0, 0.0])),
            270 => (h, w, Some([0.0, -1.0, 1.0, 0.0, 0.0, 0.0])),
            _ => (w, h, None),
        };
        let da = d
            .get(b"DA")
            .ok()
            .and_then(|o| self.string_bytes(o))
            .unwrap_or_default();
        let (_, size, color) = parse_da(&da);
        let style = d
            .get(b"DS")
            .ok()
            .and_then(|o| self.text_value(o))
            .unwrap_or_default();
        let (ds_size, ds_color) = parse_style(&style);
        let size = if size > 0.0 {
            size
        } else {
            ds_size.unwrap_or(TEXT_SIZE)
        };
        let color = color.or_else(|| ds_color.map(|c| op("rg", nums(&c))));
        let text_rgb: Vec<f32> = color
            .as_ref()
            .map(|o| {
                o.operands
                    .iter()
                    .filter_map(|x| x.as_float().ok())
                    .collect()
            })
            .unwrap_or_else(|| vec![0.0]);
        let (bw, dash) = self.border(d);
        let look = Look {
            w,
            h,
            bg: self.colour(d, b"C"),
            border: (bw > 0.0).then(|| Border {
                color: text_rgb,
                width: bw,
                style: if dash.is_some() { b'D' } else { b'S' },
            }),
            font: FONT.as_bytes().to_vec(),
            size,
            color,
            q: d.get(b"Q")
                .ok()
                .and_then(|o| self.int_value(o))
                .unwrap_or(0),
        };
        let text = d
            .get(b"Contents")
            .ok()
            .and_then(|o| self.text_value(o))
            .unwrap_or_default()
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let content = form_ap::draw(&look, &Body::Lines(&text), &standard_metrics("Helvetica"));
        Drawing {
            content: with_gs(content, alpha, false),
            bbox: [0.0, 0.0, w, h],
            gs: gs_dict(alpha, false),
            font: true,
            matrix,
        }
    }

    /// A colour entry: 1 (grey), 3 (RGB) or 4 (CMYK) numbers. An empty array means none.
    pub(crate) fn colour(&self, d: &Dictionary, key: &[u8]) -> Option<Vec<f32>> {
        let (_, o) = self.doc.dereference(d.get(key).ok()?).ok()?;
        let v: Vec<f32> = o
            .as_array()
            .ok()?
            .iter()
            .map(|x| self.num(x))
            .collect::<Option<_>>()?;
        matches!(v.len(), 1 | 3 | 4).then_some(v)
    }

    /// The border width (from `/BS /W`, else `/Border`, else 1) and dash pattern.
    pub(crate) fn border(&self, d: &Dictionary) -> (f32, Option<Vec<f32>>) {
        if let Some(bs) = d.get(b"BS").ok().and_then(|o| self.dict_of(o)) {
            let w = bs.get(b"W").ok().and_then(|o| self.num(o)).unwrap_or(1.0);
            let dashed = bs.get(b"S").and_then(|o| o.as_name()).ok() == Some(b"D");
            let dash = dashed.then(|| {
                self.numbers(bs.get(b"D").ok())
                    .filter(|v| !v.is_empty() && v.iter().any(|&x| x > 0.0))
                    .unwrap_or_else(|| vec![3.0])
            });
            return (w.max(0.0), dash);
        }
        let w = self
            .numbers(d.get(b"Border").ok())
            .and_then(|v| v.get(2).copied())
            .unwrap_or(1.0);
        (w.max(0.0), None)
    }

    /// The rectangle less the `/RD` differences, when they leave something.
    fn inner(&self, d: &Dictionary, r: [f32; 4]) -> [f32; 4] {
        match self.numbers(d.get(b"RD").ok()).as_deref() {
            Some(&[l, b, rt, t, ..]) if l + rt < r[2] - r[0] && b + t < r[3] - r[1] => {
                [r[0] + l, r[1] + b, r[2] - rt, r[3] - t]
            }
            _ => r,
        }
    }

    /// The two `/LE` line-ending names.
    pub(crate) fn line_ends(&self, d: &Dictionary) -> Vec<Vec<u8>> {
        d.get(b"LE")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .map(|a| {
                a.iter()
                    .take(2)
                    .filter_map(|n| n.as_name().ok().map(<[u8]>::to_vec))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn numbers(&self, o: Option<&Object>) -> Option<Vec<f32>> {
        let (_, o) = self.doc.dereference(o?).ok()?;
        o.as_array()
            .ok()?
            .iter()
            .take(2 * MAX_POINTS)
            .map(|x| self.num(x))
            .collect()
    }

    /// An array of numbers as points.
    fn points(&self, o: Option<&Object>) -> Option<Vec<(f32, f32)>> {
        let v = self.numbers(o)?;
        Some(v.as_chunks::<2>().0.iter().map(|&[x, y]| (x, y)).collect())
    }
}

/// How lines and fills are painted.
struct Pen {
    c: Option<Vec<f32>>,
    ic: Option<Vec<f32>>,
    w: f32,
    dash: Option<Vec<f32>>,
}

impl Pen {
    /// Set the colours and paint the current path: stroked in `/C`, and filled in `/IC`
    /// when `fill` allows it. With neither colour the path is dropped.
    fn paint(&self, ops: &mut Vec<Operation>, fill: bool) {
        let fill = if fill { self.ic.as_deref() } else { None };
        let stroke = self.c.as_deref().filter(|_| self.w > 0.0);
        let mut set = Vec::new();
        set.extend(fill.and_then(fill_color));
        if let Some(c) = stroke {
            set.extend(stroke_color(c));
            set.push(op("w", nums(&[self.w])));
            if let Some(d) = &self.dash {
                set.push(op("d", vec![nums(d).into(), 0.into()]));
            }
        }
        // The colours go before the path: a path must be painted straight after it ends.
        let start = ops
            .iter()
            .rposition(|o| matches!(o.operator.as_str(), "S" | "f" | "B" | "n"))
            .map_or(0, |i| i + 1);
        let start = ops[start..]
            .iter()
            .position(|o| o.operator == "m" || o.operator == "re")
            .map_or(ops.len(), |i| start + i);
        ops.splice(start..start, set);
        let paint = match (fill.is_some(), stroke.is_some()) {
            (true, true) => "B",
            (true, false) => "f",
            (false, true) => "S",
            (false, false) => "n",
        };
        ops.push(op(paint, vec![]));
    }
}

/// A quadrilateral as top-left, top-right, bottom-left, bottom-right. Files write
/// `/QuadPoints` in two orders: Acrobat's (top edge left to right, then bottom edge left to
/// right) and the specification's (anticlockwise from the bottom left). In Acrobat's
/// the first and second pairs run the same way.
fn orient(q: &[(f32, f32); 4]) -> [(f32, f32); 4] {
    let dot = (q[1].0 - q[0].0) * (q[3].0 - q[2].0) + (q[1].1 - q[0].1) * (q[3].1 - q[2].1);
    if dot > 0.0 {
        [q[0], q[1], q[2], q[3]]
    } else {
        [q[3], q[2], q[0], q[1]]
    }
}

fn rect_corners(r: [f32; 4]) -> [(f32, f32); 4] {
    [(r[0], r[3]), (r[2], r[3]), (r[0], r[1]), (r[2], r[1])]
}

fn lerp(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// Text markup over each line's quadrilateral: a filled highlight, or a line under,
/// through or wavy under the text, sized to the line's height.
fn markup(ops: &mut Vec<Operation>, sub: &[u8], quads: &[[(f32, f32); 4]], c: &[f32]) {
    if sub == b"Highlight" {
        ops.extend(fill_color(c));
    } else {
        ops.extend(stroke_color(c));
    }
    for &[tl, tr, bl, br] in quads {
        let h = dist(tl, bl);
        if h < 1e-3 || dist(bl, br) < 1e-3 {
            continue;
        }
        match sub {
            b"Highlight" => {
                polyline(ops, &[tl, tr, br, bl]);
                ops.push(op("h", vec![]));
                ops.push(op("f", vec![]));
            }
            b"Squiggly" => {
                let w = (h / 20.0).max(0.5);
                let (len, amp) = (dist(bl, br), h / 12.0);
                let step = (h / 6.0).max(1.0);
                let (u, n) = (lerp(bl, br, 1.0 / len), lerp(bl, tl, 1.0 / h));
                let (u, n) = ((u.0 - bl.0, u.1 - bl.1), (n.0 - bl.0, n.1 - bl.1));
                let k = (len / step).ceil().min(MAX_POINTS as f32) as usize;
                let pts: Vec<(f32, f32)> = (0..=k)
                    .map(|i| {
                        let along = (i as f32 * step).min(len);
                        let up = amp * (1.0 + (i % 2) as f32);
                        (bl.0 + u.0 * along + n.0 * up, bl.1 + u.1 * along + n.1 * up)
                    })
                    .collect();
                ops.push(op("w", nums(&[w])));
                polyline(ops, &pts);
                ops.push(op("S", vec![]));
            }
            _ => {
                let w = (h / 14.0).max(0.5);
                let t = if sub == b"StrikeOut" { 0.4 } else { 0.07 };
                let (a, b) = (lerp(bl, tl, t), lerp(br, tr, t));
                ops.push(op("w", nums(&[w])));
                ops.push(op("m", nums(&[a.0, a.1])));
                ops.push(op("l", nums(&[b.0, b.1])));
                ops.push(op("S", vec![]));
            }
        }
    }
}

fn polyline(ops: &mut Vec<Operation>, p: &[(f32, f32)]) {
    for (i, &(x, y)) in p.iter().enumerate() {
        ops.push(op(if i == 0 { "m" } else { "l" }, nums(&[x, y])));
    }
}

/// An ellipse filling `r`, as four Bézier curves.
fn ellipse(ops: &mut Vec<Operation>, r: [f32; 4]) {
    let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
    let (rx, ry) = ((r[2] - r[0]) / 2.0, (r[3] - r[1]) / 2.0);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    ops.push(op("m", nums(&[cx + rx, cy])));
    ops.push(op(
        "c",
        nums(&[cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry]),
    ));
    ops.push(op(
        "c",
        nums(&[cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy]),
    ));
    ops.push(op(
        "c",
        nums(&[cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry]),
    ));
    ops.push(op(
        "c",
        nums(&[cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy]),
    ));
    ops.push(op("h", vec![]));
}

/// How long a line ending is on a line `w` points wide.
pub(crate) fn arrow_size(w: f32) -> f32 {
    6.0 * w.max(1.0)
}

/// A line ending (`/LE`) at `at`, for a line coming from `from`. Arrows point outwards;
/// the `R` forms point back along the line.
fn line_end(ops: &mut Vec<Operation>, end: &[u8], at: (f32, f32), from: (f32, f32), pen: &Pen) {
    let len = dist(from, at);
    if len < 1e-3 {
        return;
    }
    let s = arrow_size(pen.w);
    let d = ((at.0 - from.0) / len, (at.1 - from.1) / len);
    let n = (-d.1, d.0);
    let pt = |a: f32, b: f32| (at.0 + d.0 * a + n.0 * b, at.1 + d.1 * a + n.1 * b);
    let (closed, filled) = match end {
        b"OpenArrow" => {
            polyline(ops, &[pt(-s, s / 2.0), at, pt(-s, -s / 2.0)]);
            (false, false)
        }
        b"ROpenArrow" => {
            polyline(ops, &[pt(s, s / 2.0), at, pt(s, -s / 2.0)]);
            (false, false)
        }
        b"ClosedArrow" => {
            polyline(ops, &[pt(-s, s / 2.0), at, pt(-s, -s / 2.0)]);
            (true, true)
        }
        b"RClosedArrow" => {
            polyline(ops, &[pt(s, s / 2.0), at, pt(s, -s / 2.0)]);
            (true, true)
        }
        b"Butt" => {
            polyline(ops, &[pt(0.0, s / 2.0), pt(0.0, -s / 2.0)]);
            (false, false)
        }
        b"Slash" => {
            // 30 degrees off the perpendicular, as Acrobat draws it.
            let (a, b) = (s / 2.0 * 0.5, s / 2.0 * 0.866);
            polyline(ops, &[pt(a, b), pt(-a, -b)]);
            (false, false)
        }
        b"Square" => {
            let h = s / 2.0;
            polyline(ops, &[pt(-h, -h), pt(h, -h), pt(h, h), pt(-h, h)]);
            (true, true)
        }
        b"Diamond" => {
            let h = s / 2.0;
            polyline(ops, &[pt(-h, 0.0), pt(0.0, -h), pt(h, 0.0), pt(0.0, h)]);
            (true, true)
        }
        b"Circle" => {
            let h = s / 2.0;
            ellipse(ops, [at.0 - h, at.1 - h, at.0 + h, at.1 + h]);
            (false, true)
        }
        _ => return,
    };
    if closed {
        ops.push(op("h", vec![]));
    }
    // Endings are drawn solid even on a dashed line.
    let pen = Pen {
        c: pen.c.clone(),
        ic: pen.ic.clone(),
        w: pen.w,
        dash: None,
    };
    pen.paint(ops, filled);
}

/// A note's icon at the top left of its rectangle: a sheet with a folded corner and
/// three lines of "text".
fn note(ops: &mut Vec<Operation>, r: [f32; 4], c: Option<&[f32]>) {
    let s = NOTE_SIZE.min(r[2] - r[0]).min(r[3] - r[1]);
    if s <= 2.0 {
        return;
    }
    let (x0, y1) = (r[0] + 0.5, r[3] - 0.5);
    let (x1, y0) = (x0 + s - 1.0, y1 - s + 1.0);
    let f = s * 0.3;
    ops.extend(fill_color(c.unwrap_or(&NOTE_COLOR)));
    ops.push(op("G", nums(&[0.3])));
    ops.push(op("w", nums(&[(s / 25.0).max(0.5)])));
    ops.push(op("j", vec![1.into()]));
    polyline(
        ops,
        &[(x0, y0), (x1, y0), (x1, y1 - f), (x1 - f, y1), (x0, y1)],
    );
    ops.push(op("h", vec![]));
    ops.push(op("B", vec![]));
    polyline(ops, &[(x1 - f, y1), (x1 - f, y1 - f), (x1, y1 - f)]);
    ops.push(op("S", vec![]));
    let pad = s * 0.18;
    for i in 1..=3 {
        let y = y1 - f - (i as f32) * (s - f - pad) / 4.0;
        let right = if i == 3 { x0 + s * 0.55 } else { x1 - pad };
        ops.push(op("m", nums(&[x0 + pad, y])));
        ops.push(op("l", nums(&[right, y])));
    }
    ops.push(op("S", vec![]));
}

/// The graphics state for opacity below 1 and the Multiply blend mode.
fn gs_dict(alpha: f32, blend: bool) -> Option<Dictionary> {
    if alpha >= 1.0 && !blend {
        return None;
    }
    let mut d = dictionary! { "Type" => "ExtGState", "CA" => alpha, "ca" => alpha };
    if blend {
        d.set("BM", "Multiply");
    }
    Some(d)
}

fn with_gs(content: Vec<u8>, alpha: f32, blend: bool) -> Vec<u8> {
    if alpha >= 1.0 && !blend {
        return content;
    }
    let mut out = format!("/{GS} gs\n").into_bytes();
    out.extend(content);
    out
}

/// The text size and colour from a `/DS` style string, such as
/// `font: Helvetica 12pt; color: #FF0000`.
fn parse_style(s: &str) -> (Option<f32>, Option<[f32; 3]>) {
    let (mut size, mut color) = (None, None);
    for part in s.split(';') {
        let Some((key, value)) = part.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "font-size" | "font" => {
                size = value
                    .split_whitespace()
                    .find_map(|w| w.strip_suffix("pt")?.parse::<f32>().ok())
                    .filter(|&x| x > 0.0)
                    .or(size);
            }
            "color" => {
                let hex = value.strip_prefix('#').filter(|h| h.len() == 6);
                color = hex.and_then(|h| {
                    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
                    Some([c(0)?, c(2)?, c(4)?].map(|v| v as f32 / 255.0))
                });
            }
            _ => {}
        }
    }
    (size, color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_orders_both_give_the_same_corners() {
        let acrobat = [(0.0, 10.0), (50.0, 10.0), (0.0, 0.0), (50.0, 0.0)];
        let spec = [(0.0, 0.0), (50.0, 0.0), (50.0, 10.0), (0.0, 10.0)];
        assert_eq!(orient(&acrobat), orient(&spec));
        assert_eq!(orient(&acrobat), rect_corners([0.0, 0.0, 50.0, 10.0]));
    }

    #[test]
    fn style_string_gives_size_and_colour() {
        let (size, color) = parse_style("font: Helvetica,sans-serif 14.5pt; color:#FF0000");
        assert_eq!(size, Some(14.5));
        assert_eq!(color, Some([1.0, 0.0, 0.0]));
        assert_eq!(parse_style("text-align:left"), (None, None));
    }

    #[test]
    fn colours_go_before_the_path() {
        let pen = Pen {
            c: Some(vec![1.0, 0.0, 0.0]),
            ic: Some(vec![0.0, 0.0, 1.0]),
            w: 2.0,
            dash: None,
        };
        let mut ops = vec![op("m", nums(&[0.0, 0.0])), op("l", nums(&[1.0, 1.0]))];
        pen.paint(&mut ops, true);
        let names: Vec<&str> = ops.iter().map(|o| o.operator.as_str()).collect();
        assert_eq!(names, ["rg", "RG", "w", "m", "l", "B"]);
    }
}
