//! Drawing appearance streams for annotations that have none, from their entries: text
//! markup, notes, text boxes, shapes, lines, freehand ink, stamps, carets and file
//! attachment icons. Shapes are drawn in default user space, so their appearance's box is
//! the annotation's `/Rect`; text boxes are laid out from their own corner like form
//! fields.

use crate::embed;
use crate::form_ap::{
    self, encode, fill_color, nums, op, stroke_color, Body, Border, Look, Metrics,
};
use crate::form_write::parse_da;
use crate::matrix as mx;
use crate::overlay::escape;
use crate::stamps::{bold_width, rounded, StampColor, StampKind, CAP};
use crate::{Family, FontChoice, Pdf};
use lopdf::content::Operation;
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

/// The subtypes LimarePDF draws, and draws again when they are edited.
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

/// Subtypes LimarePDF draws only while the file has no drawing for them: their own
/// drawings are artwork that editing leaves alone.
const ICONS: [&[u8]; 3] = [b"Stamp", b"Caret", b"FileAttachment"];

/// Whether LimarePDF can draw an annotation of subtype `s` that has no drawing.
pub(crate) fn drawable(s: &[u8]) -> bool {
    DRAWN.contains(&s) || ICONS.contains(&s)
}

/// Resource names in the appearances LimarePDF draws.
const FONT: &str = "Helv";
const GS: &str = "LimareGs";
/// Control-point distance for a quarter circle drawn as a Bézier curve.
const KAPPA: f32 = 0.5523;
/// A note's icon: its size in points and its colour when the note has none.
pub(crate) const NOTE_SIZE: f32 = 20.0;
const NOTE_COLOR: [f32; 3] = [1.0, 0.85, 0.2];
/// The colours of a caret and a file attachment's icon when they have none.
const CARET_COLOR: [f32; 3] = [0.1, 0.3, 0.75];
const ATTACH_COLOR: [f32; 3] = [0.3, 0.4, 0.65];
/// The grey of the outlines and symbols on icons.
const ICON_INK: f32 = 0.25;
/// A redaction mark's outline: its colour when the mark has none, and its width.
pub(crate) const REDACT_COLOR: [f32; 3] = [0.85, 0.0, 0.0];
const REDACT_LINE: f32 = 1.5;
/// A cloudy border's bump radius at intensity 1, in points.
const CLOUD: f32 = 4.75;
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
    /// The font it uses as [`FONT`], if any.
    font: Option<embed::Pen>,
    /// Turns the drawing to fit `/Rect`, for text boxes on turned pages.
    matrix: Option<[f32; 6]>,
}

impl Pdf {
    /// Give each of `ids` an appearance (`/AP /N`) drawn from its entries. Annotations
    /// LimarePDF can't draw are left alone. Returns how many were drawn.
    pub(crate) fn draw_annotations(&mut self, ids: &[ObjectId]) -> usize {
        let mut fonts: Vec<(&str, ObjectId)> = Vec::new();
        let mut n = 0;
        for &id in ids {
            let pen = self.box_pen(id);
            let Some(d) = self.annotation_drawing(id, pen) else {
                continue;
            };
            let mut res = Dictionary::new();
            if let Some(pen) = d.font {
                let f = match &pen {
                    embed::Pen::Embedded(c) => c.font,
                    embed::Pen::Standard(f) => {
                        let name = f.base_font();
                        match fonts.iter().find(|(n, _)| *n == name) {
                            Some(&(_, f)) => f,
                            None => {
                                let f = self.doc.add_object(pen.font());
                                fonts.push((name, f));
                                f
                            }
                        }
                    }
                };
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

    /// The font a text box's text is drawn in ([`Pdf::pen`]): Helvetica, or its embedded
    /// face for characters Helvetica lacks. Characters neither has show as `?`, as they
    /// did before LimarePDF embedded fonts. `None` for other annotations.
    pub(crate) fn box_pen(&mut self, id: ObjectId) -> Option<embed::Pen> {
        let d = self.doc.get_dictionary(id).ok()?;
        if d.get(b"Subtype").and_then(Object::as_name).ok()? != b"FreeText" {
            return None;
        }
        let text = self.box_text(d);
        let helvetica = FontChoice::default();
        Some(
            self.pen(helvetica, &text)
                .unwrap_or(embed::Pen::Standard(helvetica)),
        )
    }

    /// A text box's text, its `/Contents` with line breaks as `\n`.
    pub(crate) fn box_text(&self, d: &Dictionary) -> String {
        d.get(b"Contents")
            .ok()
            .and_then(|o| self.text_value(o))
            .unwrap_or_default()
            .replace("\r\n", "\n")
            .replace('\r', "\n")
    }

    /// The appearance for one annotation, or `None` for a type LimarePDF doesn't draw.
    /// `pen` is a text box's font ([`Pdf::box_pen`]).
    fn annotation_drawing(&self, id: ObjectId, pen: Option<embed::Pen>) -> Option<Drawing> {
        let d = self.doc.get_dictionary(id).ok()?;
        let rect = self.rect(d.get(b"Rect").ok()?)?;
        let sub = d.get(b"Subtype").ok()?.as_name().ok()?;
        let has_ap = d
            .get(b"AP")
            .ok()
            .and_then(|o| self.dict_of(o))
            .is_some_and(|ap| ap.has(b"N"));
        if ICONS.contains(&sub) && has_ap {
            return None;
        }
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
                let cloud = self.cloud_radius(d);
                // Without `/RD` to leave room for them, a cloud's bumps go inside the
                // rectangle.
                let h = bw / 2.0
                    + match cloud {
                        Some(c) if r == rect => c,
                        _ => 0.0,
                    };
                let r = [r[0] + h, r[1] + h, r[2] - h, r[3] - h];
                if r[2] > r[0] && r[3] > r[1] {
                    match cloud {
                        Some(c) if sub == b"Square" => cloud_path(
                            &mut ops,
                            &[(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])],
                            c,
                        ),
                        Some(c) => cloud_path(&mut ops, &ellipse_points(r, c), c),
                        None if sub == b"Square" => {
                            ops.push(op("re", nums(&[r[0], r[1], r[2] - r[0], r[3] - r[1]])));
                        }
                        None => ellipse(&mut ops, r),
                    }
                    line.paint(&mut ops, true);
                }
            }
            b"Line" => {
                let p = self.points(d.get(b"L").ok()).filter(|p| p.len() >= 2)?;
                let ends = self.line_ends(d);
                let get = |k: &[u8]| d.get(k).ok().and_then(|o| self.num(o)).unwrap_or(0.0);
                let ll = get(b"LL");
                let (mut lle, mut llo) = (get(b"LLE").max(0.0), get(b"LLO").max(0.0));
                if ll < 0.0 {
                    (lle, llo) = (-lle, -llo);
                }
                // Leader lines run from the ends at right angles, to the left going from
                // the first point to the second for a positive `/LL`, as Acrobat draws
                // them; the line itself joins them `/LL` out.
                let len = dist(p[0], p[1]);
                let leaders = ll != 0.0 && len > 1e-3;
                let n = if leaders {
                    ((p[0].1 - p[1].1) / len, (p[1].0 - p[0].0) / len)
                } else {
                    (0.0, 0.0)
                };
                let out = |q: (f32, f32), t: f32| (q.0 + n.0 * t, q.1 + n.1 * t);
                let q = [out(p[0], llo + ll), out(p[1], llo + ll)];
                ops.push(op("m", nums(&[q[0].0, q[0].1])));
                ops.push(op("l", nums(&[q[1].0, q[1].1])));
                line.paint(&mut ops, false);
                if leaders {
                    for e in [p[0], p[1]] {
                        let (a, b) = (out(e, llo), out(e, llo + ll + lle));
                        ops.push(op("m", nums(&[a.0, a.1])));
                        ops.push(op("l", nums(&[b.0, b.1])));
                    }
                    line.paint(&mut ops, false);
                }
                for (i, end) in ends.iter().enumerate() {
                    let (at, from) = if i == 0 { (q[0], q[1]) } else { (q[1], q[0]) };
                    line_end(&mut ops, end, at, from, &line);
                }
            }
            b"Polygon" | b"PolyLine" => {
                let p = self
                    .points(d.get(b"Vertices").ok())
                    .filter(|p| !p.is_empty())?;
                let closed = sub == b"Polygon";
                match self.cloud_radius(d) {
                    Some(c) if closed && p.len() >= 3 => cloud_path(&mut ops, &p, c),
                    _ => {
                        polyline(&mut ops, &p);
                        if closed {
                            ops.push(op("h", vec![]));
                        }
                    }
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
            b"Text" => note(&mut ops, rect, line.c.as_deref(), name(d)),
            b"FreeText" => return Some(self.free_text(d, rect, alpha, pen?)),
            b"Stamp" => return Some(stamp(rect, name(d), alpha)),
            b"Caret" => {
                let r = self.inner(d, rect);
                caret(&mut ops, r, line.c.as_deref().unwrap_or(&CARET_COLOR));
            }
            b"FileAttachment" => {
                attachment(&mut ops, rect, line.c.as_deref(), name(d));
            }
            _ => return None,
        }
        Some(Drawing {
            content: with_gs(encode(ops), alpha, blend),
            bbox: rect,
            gs: gs_dict(alpha, blend),
            font: None,
            matrix: None,
        })
    }

    /// The bump radius of a cloudy border (`/BE` with style `/C`), when it has one.
    fn cloud_radius(&self, d: &Dictionary) -> Option<f32> {
        let be = d.get(b"BE").ok().and_then(|o| self.dict_of(o))?;
        if be.get(b"S").and_then(|o| o.as_name()).ok() != Some(b"C") {
            return None;
        }
        let i = be
            .get(b"I")
            .ok()
            .and_then(|o| self.num(o))
            .unwrap_or(0.0)
            .clamp(0.0, 2.0);
        (i > 0.05).then_some(CLOUD * i)
    }

    /// A text box: its `/Contents` in Helvetica (or its embedded face: `pen`), with the size
    /// and colour from `/DA` (or the `/DS` style string), on the `/C` background with a
    /// border, which may be cloudy. A callout (`/CL`) is a line from what the box points at
    /// to the box, whose place in `/Rect` `/RD` then gives.
    fn free_text(&self, d: &Dictionary, rect: [f32; 4], alpha: f32, pen: embed::Pen) -> Drawing {
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
        // The drawing's own space, in which the text reads upright, and the box in it.
        let m = matrix.unwrap_or(mx::ID);
        let b = mx::transform_rect(m, [0.0, 0.0, w, h]);
        let to_page = mx::mul(m, mx::translate(rect[0] - b[0], rect[1] - b[1]));
        let to_own = mx::inv(to_page).unwrap_or(mx::ID);
        let inner = self.inner(d, rect);
        let tb = mx::transform_rect(to_own, inner);
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
        let bg = self.colour(d, b"C");
        let mut ops = Vec::new();
        if let Some(p) = self.points(d.get(b"CL").ok()).filter(|p| p.len() >= 2) {
            let p: Vec<(f32, f32)> = p
                .iter()
                .take(3)
                .map(|&(x, y)| mx::apply(to_own, x, y))
                .collect();
            let pen = Pen {
                c: Some(text_rgb.clone()),
                ic: self.colour(d, b"IC"),
                w: if bw > 0.0 { bw } else { 1.0 },
                dash: None,
            };
            polyline(&mut ops, &p);
            pen.paint(&mut ops, false);
            if let Some(end) = self.line_ends(d).first() {
                line_end(&mut ops, end, p[0], p[1], &pen);
            }
        }
        let cloud = self.cloud_radius(d);
        let mut tb = tb;
        if let Some(c) = cloud {
            let h = bw / 2.0 + if inner == rect { c } else { 0.0 };
            let r = [tb[0] + h, tb[1] + h, tb[2] - h, tb[3] - h];
            if r[2] > r[0] && r[3] > r[1] {
                // The text keeps inside the bumps.
                tb = r;
                let corners = [(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])];
                cloud_path(&mut ops, &corners, c);
                let pen = Pen {
                    c: Some(text_rgb.clone()),
                    ic: bg.clone(),
                    w: bw,
                    dash: dash.clone(),
                };
                pen.paint(&mut ops, true);
            }
        }
        let look = Look {
            w: tb[2] - tb[0],
            h: tb[3] - tb[1],
            // A cloudy border and its background are drawn above instead.
            bg: if cloud.is_some() { None } else { bg },
            border: (bw > 0.0 && cloud.is_none()).then(|| Border {
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
        let text = self.box_text(d);
        let body = form_ap::draw(&look, &Body::Lines(&text), &Metrics::of(pen.clone()));
        let mut content = encode(ops);
        content.extend(format!("\nq 1 0 0 1 {} {} cm\n", tb[0], tb[1]).into_bytes());
        content.extend(body);
        content.extend(b"\nQ\n");
        Drawing {
            content: with_gs(content, alpha, false),
            bbox: [0.0, 0.0, w, h],
            gs: gs_dict(alpha, false),
            font: Some(pen),
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

    /// The `/LE` line-ending names: two for a line, one (a name, not an array) for a text
    /// box's callout.
    pub(crate) fn line_ends(&self, d: &Dictionary) -> Vec<Vec<u8>> {
        let Some((_, o)) = d.get(b"LE").ok().and_then(|o| self.doc.dereference(o).ok()) else {
            return Vec::new();
        };
        if let Ok(n) = o.as_name() {
            return vec![n.to_vec()];
        }
        o.as_array()
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

/// An annotation's `/Name`, or nothing.
fn name(d: &Dictionary) -> &[u8] {
    d.get(b"Name").and_then(|o| o.as_name()).unwrap_or(&[])
}

/// An icon's square, `s` points wide with its bottom left at (`x`, `y`), on which
/// symbols are drawn in coordinates from 0 to 1.
#[derive(Clone, Copy)]
struct Sq {
    x: f32,
    y: f32,
    s: f32,
}

impl Sq {
    fn at(self, (u, v): (f32, f32)) -> (f32, f32) {
        (self.x + u * self.s, self.y + v * self.s)
    }

    fn poly(self, ops: &mut Vec<Operation>, p: &[(f32, f32)]) {
        polyline(ops, &p.iter().map(|&p| self.at(p)).collect::<Vec<_>>());
    }

    fn to(self, ops: &mut Vec<Operation>, p: (f32, f32)) {
        let p = self.at(p);
        ops.push(op("l", nums(&[p.0, p.1])));
    }

    fn curve(self, ops: &mut Vec<Operation>, a: (f32, f32), b: (f32, f32), to: (f32, f32)) {
        let (a, b, to) = (self.at(a), self.at(b), self.at(to));
        ops.push(op("c", nums(&[a.0, a.1, b.0, b.1, to.0, to.1])));
    }

    fn circle(self, ops: &mut Vec<Operation>, c: (f32, f32), r: f32) {
        let (lo, hi) = (self.at((c.0 - r, c.1 - r)), self.at((c.0 + r, c.1 + r)));
        ellipse(ops, [lo.0, lo.1, hi.0, hi.1]);
    }

    fn line(self, ops: &mut Vec<Operation>, a: (f32, f32), b: (f32, f32)) {
        self.poly(ops, &[a, b]);
    }
}

/// A note's icon at the top left of its rectangle, for its `/Name`: one of the seven
/// standard icons (Comment, Key, Note, Help, NewParagraph, Paragraph and Insert; any
/// other name shows Note), shaped in the note's colour with a dark outline, as other
/// readers draw them.
fn note(ops: &mut Vec<Operation>, r: [f32; 4], c: Option<&[f32]>, name: &[u8]) {
    let s = NOTE_SIZE.min(r[2] - r[0]).min(r[3] - r[1]);
    if s <= 2.0 {
        return;
    }
    let q = Sq {
        x: r[0],
        y: r[3] - s,
        s,
    };
    let outline = (s / 20.0).max(0.5);
    let marks = (s / 12.0).max(0.5);
    ops.extend(fill_color(c.unwrap_or(&NOTE_COLOR)));
    ops.push(op("G", nums(&[ICON_INK])));
    ops.push(op("w", nums(&[outline])));
    ops.push(op("J", vec![1.into()]));
    ops.push(op("j", vec![1.into()]));
    // A closed shape, filled in the note's colour and outlined.
    let shape = |ops: &mut Vec<Operation>, p: &[(f32, f32)]| {
        q.poly(ops, p);
        ops.push(op("h", vec![]));
        ops.push(op("B", vec![]));
    };
    let width = |ops: &mut Vec<Operation>, w: f32| ops.push(op("w", nums(&[w])));
    match name {
        b"Comment" => {
            shape(
                ops,
                &[
                    (0.06, 0.92),
                    (0.94, 0.92),
                    (0.94, 0.3),
                    (0.46, 0.3),
                    (0.2, 0.06),
                    (0.25, 0.3),
                    (0.06, 0.3),
                ],
            );
            width(ops, marks);
            q.line(ops, (0.22, 0.72), (0.78, 0.72));
            q.line(ops, (0.22, 0.5), (0.64, 0.5));
            ops.push(op("S", vec![]));
        }
        b"Key" => {
            shape(
                ops,
                &[
                    (0.44, 0.58),
                    (0.94, 0.58),
                    (0.94, 0.3),
                    (0.84, 0.3),
                    (0.84, 0.44),
                    (0.74, 0.44),
                    (0.74, 0.34),
                    (0.64, 0.34),
                    (0.64, 0.44),
                    (0.44, 0.44),
                ],
            );
            q.circle(ops, (0.27, 0.51), 0.22);
            ops.push(op("B", vec![]));
            q.circle(ops, (0.21, 0.51), 0.06);
            ops.push(op("S", vec![]));
        }
        b"Help" => {
            q.circle(ops, (0.5, 0.5), 0.45);
            ops.push(op("B", vec![]));
            width(ops, (s / 9.0).max(0.5));
            q.poly(ops, &[(0.36, 0.64)]);
            q.curve(ops, (0.36, 0.82), (0.64, 0.82), (0.64, 0.64));
            q.curve(ops, (0.64, 0.5), (0.5, 0.52), (0.5, 0.38));
            ops.push(op("S", vec![]));
            ops.push(op("g", nums(&[ICON_INK])));
            q.circle(ops, (0.5, 0.22), 0.06);
            ops.push(op("f", vec![]));
        }
        b"NewParagraph" => {
            shape(ops, &[(0.5, 0.96), (0.2, 0.6), (0.8, 0.6)]);
            width(ops, marks);
            q.poly(
                ops,
                &[(0.14, 0.06), (0.14, 0.44), (0.42, 0.06), (0.42, 0.44)],
            );
            q.poly(ops, &[(0.58, 0.06), (0.58, 0.44), (0.7, 0.44)]);
            q.curve(ops, (0.86, 0.44), (0.86, 0.24), (0.7, 0.24));
            q.to(ops, (0.58, 0.24));
            ops.push(op("S", vec![]));
        }
        b"Paragraph" => {
            q.poly(ops, &[(0.5, 0.94)]);
            q.curve(ops, (0.1, 0.94), (0.1, 0.48), (0.5, 0.48));
            ops.push(op("h", vec![]));
            ops.push(op("B", vec![]));
            shape(
                ops,
                &[
                    (0.5, 0.94),
                    (0.88, 0.94),
                    (0.88, 0.84),
                    (0.8, 0.84),
                    (0.8, 0.06),
                    (0.7, 0.06),
                    (0.7, 0.84),
                    (0.62, 0.84),
                    (0.62, 0.06),
                    (0.5, 0.06),
                ],
            );
        }
        b"Insert" => shape(ops, &[(0.06, 0.1), (0.5, 0.94), (0.94, 0.1), (0.5, 0.36)]),
        _ => {
            // A sheet with its top right corner folded over.
            shape(
                ops,
                &[
                    (0.1, 0.04),
                    (0.9, 0.04),
                    (0.9, 0.72),
                    (0.66, 0.96),
                    (0.1, 0.96),
                ],
            );
            q.poly(ops, &[(0.66, 0.96), (0.66, 0.72), (0.9, 0.72)]);
            ops.push(op("S", vec![]));
            width(ops, marks);
            q.line(ops, (0.24, 0.6), (0.76, 0.6));
            q.line(ops, (0.24, 0.42), (0.76, 0.42));
            q.line(ops, (0.24, 0.24), (0.56, 0.24));
            ops.push(op("S", vec![]));
        }
    }
}

/// A file attachment's icon, as large as fits in the middle of its rectangle, for its
/// `/Name`: PushPin (also for unknown names), Paperclip, Graph or Tag. GraphPushPin and
/// PaperclipTag, older names, show Graph and Paperclip.
fn attachment(ops: &mut Vec<Operation>, r: [f32; 4], c: Option<&[f32]>, name: &[u8]) {
    let s = (r[2] - r[0]).min(r[3] - r[1]);
    if s <= 2.0 {
        return;
    }
    let q = Sq {
        x: (r[0] + r[2] - s) / 2.0,
        y: (r[1] + r[3] - s) / 2.0,
        s,
    };
    let c = c.unwrap_or(&ATTACH_COLOR);
    ops.extend(fill_color(c));
    ops.push(op("G", nums(&[ICON_INK])));
    ops.push(op("w", nums(&[(s / 16.0).max(0.5)])));
    ops.push(op("J", vec![1.into()]));
    ops.push(op("j", vec![1.into()]));
    match name {
        b"Graph" | b"GraphPushPin" => {
            q.poly(ops, &[(0.14, 0.86), (0.14, 0.14), (0.86, 0.14)]);
            ops.push(op("S", vec![]));
            for (u, top) in [(0.24, 0.5), (0.45, 0.76), (0.66, 0.38)] {
                q.poly(
                    ops,
                    &[(u, 0.14), (u + 0.14, 0.14), (u + 0.14, top), (u, top)],
                );
                ops.push(op("h", vec![]));
            }
            ops.push(op("B", vec![]));
        }
        b"Paperclip" | b"PaperclipTag" => {
            ops.extend(stroke_color(c));
            ops.push(op("w", nums(&[(s / 10.0).max(0.5)])));
            q.poly(ops, &[(0.56, 0.62), (0.56, 0.3)]);
            q.curve(ops, (0.56, 0.2), (0.4, 0.2), (0.4, 0.3));
            q.to(ops, (0.4, 0.75));
            q.curve(ops, (0.4, 0.9), (0.66, 0.9), (0.66, 0.75));
            q.to(ops, (0.66, 0.25));
            q.curve(ops, (0.66, 0.06), (0.32, 0.06), (0.32, 0.25));
            q.to(ops, (0.32, 0.62));
            ops.push(op("S", vec![]));
        }
        b"Tag" => {
            q.poly(
                ops,
                &[(0.1, 0.5), (0.38, 0.8), (0.9, 0.8), (0.9, 0.2), (0.38, 0.2)],
            );
            ops.push(op("h", vec![]));
            ops.push(op("B", vec![]));
            q.circle(ops, (0.36, 0.5), 0.07);
            ops.push(op("S", vec![]));
        }
        _ => {
            q.line(ops, (0.44, 0.44), (0.14, 0.14));
            ops.push(op("S", vec![]));
            q.circle(ops, (0.6, 0.6), 0.25);
            ops.push(op("B", vec![]));
        }
    }
}

/// A caret marking where text goes in: a filled ^ across `r`.
fn caret(ops: &mut Vec<Operation>, r: [f32; 4], c: &[f32]) {
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    if w < 0.5 || h < 0.5 {
        return;
    }
    let cx = (r[0] + r[2]) / 2.0;
    ops.extend(fill_color(c));
    polyline(
        ops,
        &[(r[0], r[1]), (cx, r[3]), (r[2], r[1]), (cx, r[1] + h * 0.4)],
    );
    ops.push(op("h", vec![]));
    ops.push(op("f", vec![]));
}

/// A stamp with no drawing: its `/Name` in bold capitals in a rounded frame, coloured
/// for its meaning as LimarePDF's own stamps are. `NotApproved` reads NOT APPROVED; a
/// stamp with no name is a Draft, as the standard says.
fn stamp(rect: [f32; 4], name: &[u8], alpha: f32) -> Drawing {
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    let (word, color) = stamp_look(name);
    let [r, g, b] = color.rgb();
    let side = w.min(h);
    let frame = (side * 0.06).clamp(0.5, 2.5);
    let mut s = format!("{r} {g} {b} RG {r} {g} {b} rg\n");
    if side > 2.0 * frame {
        let half = frame / 2.0;
        s += &format!(
            "{frame} w {} S\n",
            rounded([half, half, w - half, h - half], (side * 0.15).min(6.0))
        );
    }
    let pad = frame + side * 0.12;
    let size = ((w - 2.0 * pad) / bold_width(&word, 1.0)).min((h - 2.0 * pad) / CAP);
    if size >= 1.0 {
        let x = (w - bold_width(&word, size)) / 2.0;
        let y = (h - CAP * size) / 2.0;
        s += &format!(
            "BT /{FONT} {size} Tf {x} {y} Td ({}) Tj ET\n",
            escape(&word)
        );
    }
    Drawing {
        content: with_gs(s.into_bytes(), alpha, false),
        bbox: [0.0, 0.0, w, h],
        gs: gs_dict(alpha, false),
        font: Some(embed::Pen::Standard(FontChoice {
            family: Family::Helvetica,
            bold: true,
            italic: false,
        })),
        matrix: None,
    }
}

/// A stamp's wording and colour from its `/Name`.
fn stamp_look(name: &[u8]) -> (String, StampColor) {
    let name = String::from_utf8_lossy(name);
    // Acrobat's own stamp sets prefix their names: SBApproved, SHSignHere.
    let name = ["SB", "SH"]
        .iter()
        .find_map(|p| name.strip_prefix(p))
        .filter(|n| n.starts_with(|c: char| c.is_ascii_uppercase()))
        .unwrap_or(&name);
    let mut word = String::new();
    let mut prev = ' ';
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && prev.is_ascii_lowercase() {
                word.push(' ');
            }
            word.push(c.to_ascii_uppercase());
        } else if !word.is_empty() && !word.ends_with(' ') {
            word.push(' ');
        }
        prev = c;
    }
    let word = match word.trim() {
        "" => "DRAFT".to_string(),
        w => w.to_string(),
    };
    let key: String = word
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    let color = StampKind::from_name(&word).map_or_else(
        || match key.as_str() {
            "expired" | "notforpublicrelease" | "topsecret" | "rejected" => StampColor::Red,
            "forpublicrelease" | "sold" | "completed" | "accepted" => StampColor::Green,
            _ => StampColor::Blue,
        },
        StampKind::color,
    );
    (word, color)
}

/// A cloudy border around the closed shape through `p`: bumps of about radius `r` along
/// each side, bulging outwards.
fn cloud_path(ops: &mut Vec<Operation>, p: &[(f32, f32)], r: f32) {
    let n = p.len();
    let area: f32 = (0..n)
        .map(|i| {
            let (a, b) = (p[i], p[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    // Outwards is to the right of each side going anticlockwise.
    let out = if area >= 0.0 { 1.0 } else { -1.0 };
    let mut left = MAX_POINTS;
    ops.push(op("m", nums(&[p[0].0, p[0].1])));
    for i in 0..n {
        let (a, b) = (p[i], p[(i + 1) % n]);
        let len = dist(a, b);
        if len < 1e-3 {
            continue;
        }
        let k = ((len / (2.0 * r)).round().max(1.0) as usize).min(left.max(1));
        left = left.saturating_sub(k);
        let nrm = (out * (b.1 - a.1) / len, -out * (b.0 - a.0) / len);
        // Control points 4/3 of the radius out draw close to a half circle.
        let h = len / k as f32 / 2.0 * 4.0 / 3.0;
        for j in 0..k {
            let s = lerp(a, b, j as f32 / k as f32);
            let e = lerp(a, b, (j + 1) as f32 / k as f32);
            ops.push(op(
                "c",
                nums(&[
                    s.0 + nrm.0 * h,
                    s.1 + nrm.1 * h,
                    e.0 + nrm.0 * h,
                    e.1 + nrm.1 * h,
                    e.0,
                    e.1,
                ]),
            ));
        }
    }
    ops.push(op("h", vec![]));
}

/// Points round the ellipse filling `r`, anticlockwise, two bump radii `c` apart.
fn ellipse_points(r: [f32; 4], c: f32) -> Vec<(f32, f32)> {
    let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
    let (rx, ry) = ((r[2] - r[0]) / 2.0, (r[3] - r[1]) / 2.0);
    let around = std::f32::consts::PI * (rx + ry);
    let n = ((around / (2.0 * c)).round() as usize).clamp(8, MAX_POINTS);
    (0..n)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
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
                let hex = value
                    .strip_prefix('#')
                    .filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()));
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
    fn style_colours_must_be_six_hex_digits() {
        // Six bytes that aren't six characters were cut inside a character, a panic (E15);
        // a sign was taken as part of a number.
        for bad in [
            "#aé123", "#éaaaa", "#+1+2+3", "#12345", "#1234567", "#12 456", "#gg0000",
        ] {
            assert_eq!(parse_style(&format!("color:{bad}")).1, None, "{bad}");
        }
        assert_eq!(
            parse_style("color:#00ff80").1,
            Some([0.0, 1.0, 128.0 / 255.0])
        );
        assert_eq!(parse_style("font: 12pt; color:#aé123"), (Some(12.0), None));
    }

    #[test]
    fn stamp_names_read_as_words_in_their_colour() {
        let look = |n: &str| stamp_look(n.as_bytes());
        assert_eq!(
            look("NotForPublicRelease"),
            ("NOT FOR PUBLIC RELEASE".into(), StampColor::Red)
        );
        assert_eq!(look("SBApproved"), ("APPROVED".into(), StampColor::Green));
        assert_eq!(look("AsIs"), ("AS IS".into(), StampColor::Blue));
        assert_eq!(look("my_stamp"), ("MY STAMP".into(), StampColor::Blue));
        assert_eq!(look(""), ("DRAFT".into(), StampColor::Blue));
    }

    #[test]
    fn cloud_bumps_bulge_outwards_either_way_round() {
        for square in [
            [(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)],
            [(0.0, 0.0), (0.0, 20.0), (20.0, 20.0), (20.0, 0.0)],
        ] {
            let mut ops = Vec::new();
            cloud_path(&mut ops, &square, 5.0);
            // Every control point lies outside the square.
            for o in ops.iter().filter(|o| o.operator == "c") {
                let v: Vec<f32> = o.operands.iter().map(|x| x.as_float().unwrap()).collect();
                for (x, y) in [(v[0], v[1]), (v[2], v[3])] {
                    assert!(!(0.01..19.99).contains(&x) || !(0.01..19.99).contains(&y));
                }
            }
        }
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
