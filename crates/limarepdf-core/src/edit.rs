//! Editing page content: move/resize, delete, change text, restyle paths, replace images, insert objects.

use crate::fonts::{winansi_bytes, FontChoice};
use crate::inline::Inline;
use crate::matrix::{self as mx, mul, M};
use crate::objects::{Obj, ObjectKind, Parsed, Show};
use crate::{Pdf, Result};
use anyhow::{anyhow, bail};
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream, StringFormat};

#[derive(Clone, Debug)]
pub struct TextEdit {
    pub text: String,
    /// `None` keeps the current font (when it can be reused), otherwise one of the 12 standard fonts.
    pub font: Option<FontChoice>,
    pub size: Option<f32>,
    pub color: Option<[f32; 3]>,
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

    fn standard_font(&mut self, page: ObjectId, f: FontChoice) -> Result<String> {
        let id = self.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => f.base_font(), "Encoding" => "WinAnsiEncoding",
        });
        self.add_resource(page, "Font", "LimareEd", Object::Reference(id))
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
                for j in obj.first..=obj.last {
                    let o2 = &parsed.ops[j];
                    if let Some(run) = t.runs.iter().find(|r| r.op == j) {
                        out.push(op("Tm", run.tm.iter().map(|&x| r(x)).collect()));
                        out.push(plain_show(o2, run.show));
                    } else {
                        match o2.operator.as_str() {
                            "Td" | "Tm" | "T*" => {}
                            "TD" => out.push(op(
                                "TL",
                                vec![r(-o2
                                    .operands
                                    .get(1)
                                    .and_then(|x| x.as_float().ok())
                                    .unwrap_or(0.0))],
                            )),
                            _ => out.push(o2.clone()),
                        }
                    }
                }
                // Restore the text matrix the following operators expect: here, as BT has
                // just reset it, and the line matrix later if a show carries on first.
                let tm = match following.show.take() {
                    Some(_) => t.tm_end,
                    None => t.tlm_end,
                };
                out.extend([
                    op("ET", vec![]),
                    op("Q", vec![]),
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
    /// otherwise the line is replaced by new text in a standard font at the same position.
    pub fn edit_text(&mut self, page_index: usize, id: usize, e: &TextEdit) -> Result<()> {
        let (page, parsed) = self.load_edit(page_index)?;
        let obj = Self::pick(&parsed, id)?.clone();
        let t = obj
            .text
            .clone()
            .ok_or_else(|| anyhow!("object {id} is not text"))?;
        let following = Following::of(&parsed.ops, &obj);
        let mut out = Vec::with_capacity(parsed.ops.len() + 8);
        if t.hidden {
            // Invisible text (an OCR layer): replace the line with one invisible show of
            // the new text, stretched over the line's original extent, so that it stays
            // invisible and lines up with the image. Style changes do not apply. Empty text
            // removes the line.
            for (i, o) in parsed.ops.iter().enumerate() {
                out.extend(following.before(i));
                match t.runs.iter().find(|r| r.op == i) {
                    Some(run) => out.extend(move_only(run.show)),
                    None => out.push(o.clone()),
                }
            }
            let mut out = closed(out, parsed.depth_left);
            if !e.text.trim().is_empty() {
                let (first, last) = (&t.pieces[0], &t.pieces[t.pieces.len() - 1]);
                let (s, f) = (first.corners[0], last.corners[1]);
                let (dx, dy) = t.dir;
                let width = ((f.0 - s.0) * dx + (f.1 - s.1) * dy).max(0.1);
                let size = t.size.max(1.0);
                let tz = (100.0 * width / crate::text_width(&e.text, size).max(0.1))
                    .clamp(1.0, 10_000.0);
                let fname = self.standard_font(page, FontChoice::from_name("Helvetica"))?;
                out.extend([
                    op("q", vec![]),
                    op("BT", vec![]),
                    op("Tr", vec![Object::Integer(3)]),
                    op("Tf", vec![name(&fname), r(size)]),
                    op("Tz", vec![r(tz)]),
                    op(
                        "Tm",
                        [dx, dy, -dy, dx, t.origin.0, t.origin.1]
                            .iter()
                            .map(|&x| r(x))
                            .collect(),
                    ),
                    op("Tj", vec![lit(winansi_bytes(&e.text))]),
                    op("ET", vec![]),
                    op("Q", vec![]),
                ]);
            }
            return self.commit(page, out, 0, &parsed.inline);
        }
        let ascii = e.text.chars().all(|c| (' '..='~').contains(&c));
        let keep_style = e.font.is_none() && e.size.is_none() && e.color.is_none();
        if keep_style && t.reuse && ascii {
            let first_run = t.runs[0].op;
            for (i, o) in parsed.ops.iter().enumerate() {
                out.extend(following.before(i));
                match t.runs.iter().find(|r| r.op == i) {
                    Some(run) if i == first_run => {
                        out.extend(move_only(run.show));
                        out.push(op("Tj", vec![lit(e.text.clone().into_bytes())]));
                    }
                    Some(run) => out.extend(move_only(run.show)),
                    None => out.push(o.clone()),
                }
            }
            return self.commit(page, out, parsed.depth_left, &parsed.inline);
        }
        // Replacement: remove the old shows, append the new text on top.
        for (i, o) in parsed.ops.iter().enumerate() {
            out.extend(following.before(i));
            match t.runs.iter().find(|r| r.op == i) {
                Some(run) => out.extend(move_only(run.show)),
                None => out.push(o.clone()),
            }
        }
        let mut out = closed(out, parsed.depth_left);
        let font = e.font.unwrap_or_else(|| FontChoice::from_name(&t.base));
        let size = e.size.unwrap_or(t.size).max(1.0);
        let color = e.color.or(t.fill).unwrap_or([0.0; 3]);
        let fname = self.standard_font(page, font)?;
        let (dx, dy) = t.dir;
        out.extend([
            op("q", vec![]),
            op("BT", vec![]),
            op("Tf", vec![name(&fname), r(size)]),
            rgb("rg", color),
            op(
                "Tm",
                [dx, dy, -dy, dx, t.origin.0, t.origin.1]
                    .iter()
                    .map(|&x| r(x))
                    .collect(),
            ),
            op("Tj", vec![lit(winansi_bytes(&e.text))]),
            op("ET", vec![]),
            op("Q", vec![]),
        ]);
        self.commit(page, out, 0, &parsed.inline)
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

    /// Replace the picture of an image object with a PNG/JPEG file; the new picture is fitted inside the old box.
    pub fn replace_image(&mut self, page_index: usize, id: usize, bytes: &[u8]) -> Result<()> {
        let (page, parsed) = self.load_edit(page_index)?;
        let obj = Self::pick(&parsed, id)?.clone();
        if obj.kind != ObjectKind::Image || obj.inline {
            bail!("only image objects stored as XObjects can be replaced");
        }
        let (img, w, h) = self.add_image_object(bytes)?;
        let nm = self.add_resource(page, "XObject", "LimareIm", Object::Reference(img))?;
        let (bw, bh) = (
            obj.ctm[0].hypot(obj.ctm[1]).max(1e-3),
            obj.ctm[2].hypot(obj.ctm[3]).max(1e-3),
        );
        let a = w as f32 / h as f32;
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
                out.extend([
                    op("q", vec![]),
                    cm(fit),
                    op("Do", vec![name(&nm)]),
                    op("Q", vec![]),
                ]);
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
        if text.is_empty() {
            bail!("empty text");
        }
        let text = text.to_string();
        self.append_layer(page_index, move |s, page| {
            let f = s.standard_font(page, font)?;
            Ok(vec![
                op("BT", vec![]),
                op("Tf", vec![name(&f), r(size.max(1.0))]),
                rgb("rg", color),
                op(
                    "Tm",
                    [1.0, 0.0, 0.0, -1.0, pos.0, pos.1]
                        .iter()
                        .map(|&x| r(x))
                        .collect(),
                ),
                op("Tj", vec![lit(winansi_bytes(&text))]),
                op("ET", vec![]),
            ])
        })
    }

    /// Add a picture filling `rect` (displayed-page points, y down).
    pub fn add_image(&mut self, page_index: usize, rect: [f32; 4], bytes: &[u8]) -> Result<()> {
        let (img, _, _) = self.add_image_object(bytes)?;
        self.append_layer(page_index, move |s, page| {
            let nm = s.add_resource(page, "XObject", "LimareIm", Object::Reference(img))?;
            let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
            Ok(vec![
                op("q", vec![]),
                cm([w, 0.0, 0.0, -h, rect[0], rect[3]]),
                op("Do", vec![name(&nm)]),
                op("Q", vec![]),
            ])
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
