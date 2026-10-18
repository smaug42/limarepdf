//! Adding and changing annotations: text markup over lines of text, notes, text boxes,
//! shapes, lines, freehand drawings and replies, an annotation's colour, opacity and
//! comment, and its place on the page. Each gets a drawing of its own straight away, so
//! other viewers show it as LimarePDF does.

use crate::annot_ap::{arrow_size, DRAWN, NOTE_SIZE};
use crate::annots::NO_VIEW;
use crate::doc::encode_text;
use crate::form_ap::{lines_height, nums};
use crate::form_write::{parse_da, standard_metrics};
use crate::matrix as mx;
use crate::overlay::utc_now;
use crate::{AnnotKind, Pdf, Result, Shape, ShapeLook, TextStyle};
use anyhow::{anyhow, bail};
use lopdf::{dictionary, Dictionary, Object, ObjectId, StringFormat};

/// Annotation flags (`/F`): printed; not zoomed or turned with the page (notes).
const PRINT: i64 = 1 << 2;
const NO_ZOOM: i64 = 1 << 3;
const NO_ROTATE: i64 = 1 << 4;
/// Room around the lines in `/Rect`, for the wavy line and strokes on the edge.
const PAD: f32 = 1.0;
/// How far a simplified pen stroke may stray from the one drawn, in points.
const INK_TOLERANCE: f32 = 0.35;
/// Most points kept from one pen stroke.
const MAX_STROKE: usize = 20_000;

impl Pdf {
    /// Mark lines of text on page `page_index` (zero-based) with a highlight, underline,
    /// strike-out or squiggly line: one annotation with one area per line. `lines` are in
    /// displayed-page points (y down), like [`crate::PageWord::bbox`]. `author` is written
    /// when not empty. Returns the new annotation's id.
    pub fn add_markup(
        &mut self,
        page_index: usize,
        kind: &AnnotKind,
        lines: &[[f32; 4]],
        color: [f32; 3],
        author: &str,
    ) -> Result<(u32, u16)> {
        let sub = match kind {
            AnnotKind::Highlight => "Highlight",
            AnnotKind::Underline => "Underline",
            AnnotKind::StrikeOut => "StrikeOut",
            AnnotKind::Squiggly => "Squiggly",
            k => bail!("{} is not text markup", k.label()),
        };
        let lines: Vec<[f32; 4]> = lines
            .iter()
            .copied()
            .filter(|b| b[2] > b[0] && b[3] > b[1])
            .collect();
        if lines.is_empty() {
            bail!("no text to mark");
        }
        let page = self.page_id_at(page_index)?;
        let (to_user, _, _) = self.vis_down(page);
        // Each line's corners in Acrobat's order (top left, top right, bottom left, bottom
        // right as the page is shown), so the marks follow the text on turned pages.
        let mut quads = Vec::with_capacity(lines.len() * 8);
        let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for b in &lines {
            for (x, y) in [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])] {
                let (u, v) = mx::apply(to_user, x, y);
                quads.extend([u, v]);
                (r[0], r[1]) = (r[0].min(u), r[1].min(v));
                (r[2], r[3]) = (r[2].max(u), r[3].max(v));
            }
        }
        let d = dictionary! {
            "Subtype" => sub,
            "Rect" => nums(&[r[0] - PAD, r[1] - PAD, r[2] + PAD, r[3] + PAD]),
            "QuadPoints" => nums(&quads),
            "C" => nums(&color),
            "F" => PRINT,
        };
        self.insert_annot(page, d, author)
    }

    /// Mark areas of page `page_index` (zero-based) for redaction: one redaction mark
    /// (`/Redact`) covering `areas`, in displayed-page points (y down). Nothing is removed
    /// until the marks are applied ([`Pdf::apply_redactions`]). Returns the mark's id.
    pub fn add_redaction_mark(
        &mut self,
        page_index: usize,
        areas: &[[f32; 4]],
        author: &str,
    ) -> Result<(u32, u16)> {
        let areas: Vec<[f32; 4]> = areas
            .iter()
            .map(|b| {
                [
                    b[0].min(b[2]),
                    b[1].min(b[3]),
                    b[0].max(b[2]),
                    b[1].max(b[3]),
                ]
            })
            .filter(|b| b[2] - b[0] > 0.1 && b[3] - b[1] > 0.1)
            .collect();
        if areas.is_empty() {
            bail!("nothing to mark");
        }
        let page = self.page_id_at(page_index)?;
        let (to_user, _, _) = self.vis_down(page);
        let mut quads = Vec::with_capacity(areas.len() * 8);
        let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for b in &areas {
            for (x, y) in [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])] {
                let (u, v) = mx::apply(to_user, x, y);
                quads.extend([u, v]);
                (r[0], r[1]) = (r[0].min(u), r[1].min(v));
                (r[2], r[3]) = (r[2].max(u), r[3].max(v));
            }
        }
        // `/IC` is the colour the area is painted when applied. Not printed while a mark.
        let d = dictionary! {
            "Subtype" => "Redact",
            "Rect" => nums(&[r[0] - PAD, r[1] - PAD, r[2] + PAD, r[3] + PAD]),
            "QuadPoints" => nums(&quads),
            "C" => nums(&crate::annot_ap::REDACT_COLOR),
            "IC" => nums(&[0.0, 0.0, 0.0]),
        };
        self.insert_annot(page, d, author)
    }

    /// Put a sticky note on page `page_index` (zero-based), its icon centred on `at`
    /// (displayed-page points, y down), with the comment `contents`. Returns its id.
    pub fn add_note(
        &mut self,
        page_index: usize,
        at: (f32, f32),
        color: [f32; 3],
        contents: &str,
        author: &str,
    ) -> Result<(u32, u16)> {
        let page = self.page_id_at(page_index)?;
        let (to_user, _, _) = self.vis_down(page);
        let s = NOTE_SIZE / 2.0;
        let r = mx::transform_rect(to_user, [at.0 - s, at.1 - s, at.0 + s, at.1 + s]);
        let mut d = dictionary! {
            "Subtype" => "Text",
            "Rect" => nums(&r),
            "C" => nums(&color),
            "Name" => "Comment",
            // Printed, and kept at its size and upright by viewers that zoom and turn
            // notes themselves.
            "F" => PRINT | NO_ZOOM | NO_ROTATE,
        };
        set_text(&mut d, "Contents", contents);
        self.insert_annot(page, d, author)
    }

    /// Write `text` on page `page_index` (zero-based) in a text box filling `rect`
    /// (displayed-page points, y down), set as `style` says. Returns its id.
    pub fn add_text_box(
        &mut self,
        page_index: usize,
        rect: [f32; 4],
        text: &str,
        style: &TextStyle,
        author: &str,
    ) -> Result<(u32, u16)> {
        if rect[2] - rect[0] < 1.0 || rect[3] - rect[1] < 1.0 {
            bail!("the text box is too small");
        }
        let page = self.page_id_at(page_index)?;
        let (to_user, _, _) = self.vis_down(page);
        let mut d = dictionary! {
            "Subtype" => "FreeText",
            "Rect" => nums(&mx::transform_rect(to_user, rect)),
            "F" => PRINT,
        };
        let turn = self.page_rotation(page);
        if turn != 0 {
            d.set("Rotate", turn);
        }
        set_text(&mut d, "Contents", text);
        text_style(&mut d, style);
        self.insert_annot(page, d, author)
    }

    /// Draw a rectangle, ellipse or line on page `page_index` (zero-based) as an
    /// annotation. `rect` is in displayed-page points (y down); a line runs from its first
    /// corner to the second. `color` is the line's colour, `None` for no outline. Returns
    /// its id.
    pub fn add_shape_annotation(
        &mut self,
        page_index: usize,
        shape: Shape,
        rect: [f32; 4],
        color: Option<[f32; 3]>,
        look: &ShapeLook,
        author: &str,
    ) -> Result<(u32, u16)> {
        let page = self.page_id_at(page_index)?;
        let (to_user, _, _) = self.vis_down(page);
        let mut d = if shape == Shape::Line {
            let (a, b) = (
                mx::apply(to_user, rect[0], rect[1]),
                mx::apply(to_user, rect[2], rect[3]),
            );
            if (b.0 - a.0).hypot(b.1 - a.1) < 1.0 {
                bail!("the line is too short");
            }
            let l = [a.0, a.1, b.0, b.1];
            dictionary! {
                "Subtype" => "Line",
                "L" => nums(&l),
                "Rect" => nums(&line_rect(&l, look.width, look.arrow)),
            }
        } else {
            let r = mx::transform_rect(to_user, rect);
            if r[2] - r[0] < 1.0 || r[3] - r[1] < 1.0 {
                bail!("the shape is too small");
            }
            let sub = if shape == Shape::Rect {
                "Square"
            } else {
                "Circle"
            };
            dictionary! { "Subtype" => sub, "Rect" => nums(&r) }
        };
        d.set("F", PRINT);
        match color {
            Some(c) => d.set("C", nums(&c)),
            None => d.set("C", Vec::<Object>::new()),
        }
        shape_entries(&mut d, look, true);
        self.insert_annot(page, d, author)
    }

    /// Draw freehand on page `page_index` (zero-based): `strokes` in displayed-page points
    /// (y down), each simplified, as one drawing (`/Ink`) in `color`, `width` points wide.
    /// Returns its id.
    pub fn add_ink(
        &mut self,
        page_index: usize,
        strokes: &[Vec<(f32, f32)>],
        color: [f32; 3],
        width: f32,
        author: &str,
    ) -> Result<(u32, u16)> {
        let page = self.page_id_at(page_index)?;
        let (to_user, _, _) = self.vis_down(page);
        let list: Vec<Vec<f32>> = strokes
            .iter()
            .map(|s| user_stroke(to_user, s))
            .filter(|s| !s.is_empty())
            .collect();
        if list.is_empty() {
            bail!("nothing was drawn");
        }
        let look = ShapeLook {
            width,
            ..ShapeLook::default()
        };
        let mut d = dictionary! {
            "Subtype" => "Ink",
            "Rect" => nums(&points_rect(&list, ink_pad(look.width))),
            "InkList" => ink_list(&list),
            "C" => nums(&color),
            "F" => PRINT,
        };
        shape_entries(&mut d, &look, false);
        self.insert_annot(page, d, author)
    }

    /// Add a stroke (displayed-page points on page `page_index`, simplified as in
    /// [`Pdf::add_ink`]) to the drawing `id`, and draw it again.
    pub fn add_ink_stroke(
        &mut self,
        page_index: usize,
        id: (u32, u16),
        stroke: &[(f32, f32)],
    ) -> Result<()> {
        let page = self.page_id_at(page_index)?;
        let (to_user, _, _) = self.vis_down(page);
        let new = user_stroke(to_user, stroke);
        if new.is_empty() {
            bail!("nothing was drawn");
        }
        let d = self.doc.get_dictionary(id)?;
        if d.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"Ink") {
            bail!("not a drawing");
        }
        let mut list = self.strokes(d);
        list.push(new);
        let pad = ink_pad(self.border(d).0);
        let d = self.doc.get_dictionary_mut(id)?;
        d.set("InkList", ink_list(&list));
        d.set("Rect", nums(&points_rect(&list, pad)));
        d.set("M", Object::string_literal(pdf_now()));
        self.draw_annotations(&[id]);
        Ok(())
    }

    /// Set a rectangle's, ellipse's, line's or drawing's line width, fill and arrowhead,
    /// and draw it again. A line's ends are rewritten only when the arrowhead is switched
    /// on or off, so other endings a file has are kept.
    pub fn set_shape_look(&mut self, id: (u32, u16), look: &ShapeLook) -> Result<()> {
        let d = self.doc.get_dictionary(id)?;
        let had_arrow = self.line_ends(d).iter().any(|e| e.as_slice() != b"None");
        let rect = match d.get(b"Subtype").and_then(|o| o.as_name())? {
            b"Square" | b"Circle" => None,
            b"Line" => self
                .numbers(d.get(b"L").ok())
                .filter(|l| l.len() >= 4)
                .map(|l| line_rect(&l, look.width, look.arrow)),
            b"Ink" => Some(points_rect(&self.strokes(d), ink_pad(look.width))),
            _ => bail!("not a shape, line or drawing"),
        };
        let d = self.doc.get_dictionary_mut(id)?;
        shape_entries(d, look, look.arrow != had_arrow);
        if let Some(r) = rect.filter(|r| r[2] > r[0] && r[3] > r[1]) {
            d.set("Rect", nums(&r));
        }
        d.set("M", Object::string_literal(pdf_now()));
        self.draw_annotations(&[id]);
        Ok(())
    }

    /// Answer the annotation `to` with a comment (`/IRT`). The reply is listed with what
    /// it answers, and is not shown on the page or printed. Returns its id.
    pub fn add_reply(&mut self, to: (u32, u16), text: &str, author: &str) -> Result<(u32, u16)> {
        if text.trim().is_empty() {
            bail!("the reply is empty");
        }
        let page = self.annot_page(to)?;
        let rect = self.doc.get_dictionary(to)?.get(b"Rect")?.clone();
        let mut d = dictionary! {
            "Subtype" => "Text",
            "Rect" => rect,
            "IRT" => to,
            "RT" => "R",
            "Name" => "Comment",
            "F" => NO_ZOOM | NO_ROTATE | NO_VIEW,
        };
        set_text(&mut d, "Contents", text);
        self.insert_annot(page, d, author)
    }

    /// Move a line's ends to `ends` (x1, y1, x2, y2 in displayed-page points on page
    /// `page_index`, zero-based), keeping its width and endings, and draw it again.
    pub fn set_line_ends(
        &mut self,
        page_index: usize,
        id: (u32, u16),
        ends: [f32; 4],
    ) -> Result<()> {
        let page = self.page_id_at(page_index)?;
        let (to_user, _, _) = self.vis_down(page);
        let (a, b) = (
            mx::apply(to_user, ends[0], ends[1]),
            mx::apply(to_user, ends[2], ends[3]),
        );
        if (b.0 - a.0).hypot(b.1 - a.1) < 1.0 {
            bail!("the line is too short");
        }
        let d = self.doc.get_dictionary(id)?;
        if d.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"Line") {
            bail!("not a line");
        }
        let width = self.border(d).0;
        let arrow = self.line_ends(d).iter().any(|e| e.as_slice() != b"None");
        let l = [a.0, a.1, b.0, b.1];
        let d = self.doc.get_dictionary_mut(id)?;
        d.set("L", nums(&l));
        d.set("Rect", nums(&line_rect(&l, width, arrow)));
        d.set("M", Object::string_literal(pdf_now()));
        self.draw_annotations(&[id]);
        Ok(())
    }

    /// Turn the drawing `id` on page `page_index` (zero-based) by `degrees` about the
    /// middle of its box, clockwise as the page is shown, and draw it again.
    pub fn rotate_ink(&mut self, page_index: usize, id: (u32, u16), degrees: f32) -> Result<()> {
        let page = self.page_id_at(page_index)?;
        let (to_user, _, _) = self.vis_down(page);
        let to_vis = mx::inv(to_user).ok_or_else(|| anyhow!("degenerate page transform"))?;
        let d = self.doc.get_dictionary(id)?;
        if d.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"Ink") {
            bail!("not a drawing");
        }
        let pad = ink_pad(self.border(d).0);
        let strokes = self.strokes(d);
        // The middle of the strokes as shown; y runs down, so a positive angle turns
        // clockwise.
        let r = points_rect(&strokes, 0.0);
        let (cx, cy) = mx::apply(to_vis, (r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
        let (s, c) = degrees.to_radians().sin_cos();
        let turn = [c, s, -s, c, cx - c * cx + s * cy, cy - s * cx - c * cy];
        let m = mx::mul(mx::mul(to_vis, turn), to_user);
        let list: Vec<Vec<f32>> = strokes.iter().map(|v| map_points(m, v)).collect();
        let d = self.doc.get_dictionary_mut(id)?;
        d.set("InkList", ink_list(&list));
        d.set("Rect", nums(&points_rect(&list, pad)));
        d.set("M", Object::string_literal(pdf_now()));
        self.draw_annotations(&[id]);
        Ok(())
    }

    /// A drawing's strokes as numbers, as `/InkList` holds them.
    fn strokes(&self, d: &Dictionary) -> Vec<Vec<f32>> {
        d.get(b"InkList")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .map(|list| list.iter().filter_map(|s| self.numbers(Some(s))).collect())
            .unwrap_or_default()
    }

    /// The page an annotation is on: its `/P`, else the page that lists it.
    fn annot_page(&self, id: ObjectId) -> Result<ObjectId> {
        let d = self.doc.get_dictionary(id)?;
        match d.get(b"P").and_then(|o| o.as_reference()) {
            Ok(p) => Ok(p),
            Err(_) => self
                .page_ids()
                .into_iter()
                .find(|&p| self.annot_refs(p).contains(&id))
                .ok_or_else(|| anyhow!("the annotation is on no page")),
        }
    }

    /// Set a text box's text size and colour, fill and border, and draw it again.
    pub fn set_text_style(&mut self, id: (u32, u16), style: &TextStyle) -> Result<()> {
        let d = self.doc.get_dictionary_mut(id)?;
        if d.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"FreeText") {
            bail!("not a text box");
        }
        text_style(d, style);
        d.set("M", Object::string_literal(pdf_now()));
        self.fit_text_box(id)?;
        self.draw_annotations(&[id]);
        Ok(())
    }

    /// Move an annotation on page `page_index` (zero-based) so that its box becomes `to`
    /// (displayed-page points, y down), stretching its lines, points and marked areas to
    /// match. A move keeps the drawing it has; a change of size draws it again when LimarePDF
    /// can, and otherwise the viewer stretches the old drawing.
    pub fn move_annotation(
        &mut self,
        page_index: usize,
        id: (u32, u16),
        to: [f32; 4],
    ) -> Result<()> {
        let page = self.page_id_at(page_index)?;
        let (to_user, _, _) = self.vis_down(page);
        let new = mx::transform_rect(to_user, to);
        let d = self.doc.get_dictionary(id)?;
        let old = self
            .rect(d.get(b"Rect")?)
            .ok_or_else(|| anyhow!("the annotation has no box"))?;
        let (ow, oh) = (old[2] - old[0], old[3] - old[1]);
        let (nw, nh) = (new[2] - new[0], new[3] - new[1]);
        let scale = |n: f32, o: f32| if o > 0.01 { n / o } else { 1.0 };
        let (sx, sy) = (scale(nw, ow), scale(nh, oh));
        let map = [sx, 0.0, 0.0, sy, new[0] - old[0] * sx, new[1] - old[1] * sy];
        let resized = (nw - ow).abs() > 0.01 || (nh - oh).abs() > 0.01;
        let drawn = d
            .get(b"Subtype")
            .and_then(|o| o.as_name())
            .is_ok_and(|s| DRAWN.contains(&s));
        let mut changed: Vec<(&[u8], Object)> = vec![(b"Rect", nums(&new).into())];
        for key in [&b"L"[..], b"Vertices", b"QuadPoints", b"CL"] {
            if let Some(v) = self.numbers(d.get(key).ok()) {
                changed.push((key, nums(&map_points(map, &v)).into()));
            }
        }
        let ink = d
            .get(b"InkList")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok());
        if let Some(list) = ink {
            let strokes: Vec<Object> = list
                .iter()
                .filter_map(|s| self.numbers(Some(s)))
                .map(|v| nums(&map_points(map, &v)).into())
                .collect();
            changed.push((b"InkList", strokes.into()));
        }
        let d = self.doc.get_dictionary_mut(id)?;
        for (k, v) in changed {
            d.set(k, v);
        }
        d.set("M", Object::string_literal(pdf_now()));
        if drawn && resized {
            self.draw_annotations(&[id]);
        }
        Ok(())
    }

    /// Make a text box tall enough for its text: it grows downward as the page is shown,
    /// keeping its width, and stops at the page's edge. It never shrinks. Not drawn here.
    fn fit_text_box(&mut self, id: ObjectId) -> Result<()> {
        let d = self.doc.get_dictionary(id)?;
        let rect = self
            .rect(d.get(b"Rect")?)
            .ok_or_else(|| anyhow!("the annotation has no box"))?;
        let text = d
            .get(b"Contents")
            .ok()
            .and_then(|o| self.text_value(o))
            .unwrap_or_default()
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let da = d
            .get(b"DA")
            .ok()
            .and_then(|o| self.string_bytes(o))
            .unwrap_or_default();
        let size = Some(parse_da(&da).1).filter(|&s| s > 0.0).unwrap_or(12.0);
        let border = self.border(d).0;
        let turn = d
            .get(b"Rotate")
            .ok()
            .and_then(|o| self.int_value(o))
            .unwrap_or(0)
            .rem_euclid(360);
        let pb = self.page_box(self.annot_page(id)?);
        let sideways = matches!(turn, 90 | 270);
        let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
        let (w, h) = if sideways { (h, w) } else { (w, h) };
        let need = lines_height(&text, w, size, border, &standard_metrics("Helvetica"));
        if need <= h + 0.01 {
            return Ok(());
        }
        let grow = need - h;
        // Down the page as shown: down, right, up or left in the page's own space.
        let mut r = rect;
        match turn {
            90 => r[2] = (r[2] + grow).min(pb[2]).max(r[2]),
            180 => r[3] = (r[3] + grow).min(pb[3]).max(r[3]),
            270 => r[0] = (r[0] - grow).max(pb[0]).min(r[0]),
            _ => r[1] = (r[1] - grow).max(pb[1]).min(r[1]),
        }
        self.doc.get_dictionary_mut(id)?.set("Rect", nums(&r));
        Ok(())
    }

    /// Add the entries every new annotation has, put it on `page` and draw it.
    pub(crate) fn insert_annot(
        &mut self,
        page: ObjectId,
        mut d: Dictionary,
        author: &str,
    ) -> Result<(u32, u16)> {
        let now = Object::string_literal(pdf_now());
        d.set("Type", "Annot");
        d.set("P", page);
        d.set("M", now.clone());
        d.set("CreationDate", now);
        set_text(&mut d, "T", author);
        let id = self.doc.add_object(d);
        let name = format!("limarepdf-{}-{}", id.0, pdf_now().trim_start_matches("D:"));
        if let Ok(a) = self.doc.get_dictionary_mut(id) {
            a.set("NM", Object::string_literal(name));
        }
        self.push_annot(page, id)?;
        let d = self.doc.get_dictionary(id)?;
        let reply = d.has(b"IRT");
        if d.get(b"Subtype")?.as_name()? == b"FreeText" {
            self.fit_text_box(id)?;
        }
        // Replies are not shown, so they need no drawing.
        if !reply {
            self.draw_annotations(&[id]);
        }
        Ok(id)
    }

    /// Change an annotation's colour (`None` for none), opacity (0 to 1) and comment.
    /// Kinds LimarePDF draws are drawn again when their look changes. Locked annotations are
    /// changed too; the app checks [`crate::Annot::locked`] first.
    pub fn set_annotation(
        &mut self,
        id: (u32, u16),
        color: Option<[f32; 3]>,
        opacity: f32,
        contents: &str,
    ) -> Result<()> {
        let old = self
            .annotations()
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| anyhow!("no such annotation"))?;
        let opacity = opacity.clamp(0.0, 1.0);
        let looks_new = old.color != color
            || (old.opacity - opacity).abs() > 1e-3
            || (old.kind == AnnotKind::TextBox && old.contents != contents)
            || !old.has_appearance;
        let d = self.doc.get_dictionary_mut(id)?;
        let drawn = d
            .get(b"Subtype")
            .and_then(|o| o.as_name())
            .is_ok_and(|s| DRAWN.contains(&s));
        match color {
            Some(c) => d.set("C", nums(&c)),
            None => d.set("C", Vec::<Object>::new()),
        }
        if opacity < 1.0 {
            d.set("CA", opacity);
        } else {
            d.remove(b"CA");
        }
        set_text(d, "Contents", contents);
        d.set("M", Object::string_literal(pdf_now()));
        if old.kind == AnnotKind::TextBox && old.contents != contents {
            self.fit_text_box(id)?;
        }
        if drawn && looks_new {
            self.draw_annotations(&[id]);
        }
        if old.kind == AnnotKind::Stamp && (old.opacity - opacity).abs() > 1e-3 {
            self.restamp(id)?;
        }
        Ok(())
    }

    /// Add `annot` to the end of the page's `/Annots`, which may be an array in the page
    /// or one the page refers to.
    pub(crate) fn push_annot(&mut self, page: ObjectId, annot: ObjectId) -> Result<()> {
        let pd = self.doc.get_dictionary_mut(page)?;
        match pd.get_mut(b"Annots") {
            Ok(Object::Array(a)) => a.push(annot.into()),
            Ok(Object::Reference(r)) => {
                let r = *r;
                match self.doc.get_object_mut(r) {
                    Ok(Object::Array(a)) => a.push(annot.into()),
                    _ => {
                        let pd = self.doc.get_dictionary_mut(page)?;
                        pd.set("Annots", vec![Object::from(annot)]);
                    }
                }
            }
            _ => pd.set("Annots", vec![Object::from(annot)]),
        }
        Ok(())
    }
}

/// Set a text entry, or remove it when `text` is empty.
fn set_text(d: &mut Dictionary, key: &str, text: &str) {
    if text.is_empty() {
        d.remove(key.as_bytes());
    } else {
        d.set(
            key,
            Object::String(encode_text(text), StringFormat::Literal),
        );
    }
}

/// A text box's style entries: `/DA` for the text, `/C` for the fill (removed for none)
/// and `/BS` for the border.
fn text_style(d: &mut Dictionary, s: &TextStyle) {
    let [r, g, b] = s.color;
    let da = format!("/Helv {} Tf {r} {g} {b} rg", s.size.clamp(1.0, 400.0));
    d.set("DA", Object::string_literal(da));
    match s.fill {
        Some(c) => d.set("C", nums(&c)),
        None => {
            d.remove(b"C");
        }
    }
    d.set("BS", dictionary! { "W" => s.border.max(0.0), "S" => "S" });
}

/// A shape's look: the `/BS` width; the `/IC` fill of a rectangle or ellipse; and, when
/// `ends` is set, a line's arrowhead (`/LE`).
fn shape_entries(d: &mut Dictionary, look: &ShapeLook, ends: bool) {
    d.set(
        "BS",
        dictionary! { "W" => look.width.clamp(0.0, 100.0), "S" => "S" },
    );
    match d.get(b"Subtype").and_then(|o| o.as_name()).ok() {
        Some(b"Square" | b"Circle") => match look.fill {
            Some(c) => d.set("IC", nums(&c)),
            None => {
                d.remove(b"IC");
            }
        },
        Some(b"Line") if ends => {
            if look.arrow {
                d.set("LE", vec![Object::from("None"), Object::from("OpenArrow")]);
            } else {
                d.remove(b"LE");
            }
        }
        _ => {}
    }
}

/// A line's `/Rect`: around its two points (`l`, as `/L` holds them), with room for its
/// width and arrowhead.
fn line_rect(l: &[f32], width: f32, arrow: bool) -> [f32; 4] {
    let pad = width / 2.0 + PAD + if arrow { arrow_size(width) } else { 0.0 };
    points_rect(&[l[..4].to_vec()], pad)
}

/// Room around a drawing's points for its strokes.
fn ink_pad(width: f32) -> f32 {
    width.max(0.0) / 2.0 + PAD
}

/// The box around every x, y pair in `lists`, `pad` larger on each side.
fn points_rect(lists: &[Vec<f32>], pad: f32) -> [f32; 4] {
    let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for &[x, y] in lists.iter().flat_map(|v| v.as_chunks::<2>().0) {
        (r[0], r[1]) = (r[0].min(x), r[1].min(y));
        (r[2], r[3]) = (r[2].max(x), r[3].max(y));
    }
    [r[0] - pad, r[1] - pad, r[2] + pad, r[3] + pad]
}

/// Strokes as an `/InkList`.
fn ink_list(list: &[Vec<f32>]) -> Object {
    list.iter()
        .map(|s| Object::from(nums(s)))
        .collect::<Vec<_>>()
        .into()
}

/// A pen stroke in displayed-page points as user-space numbers, simplified.
fn user_stroke(to_user: mx::M, stroke: &[(f32, f32)]) -> Vec<f32> {
    let pts: Vec<(f32, f32)> = stroke
        .iter()
        .take(MAX_STROKE)
        .map(|&(x, y)| mx::apply(to_user, x, y))
        .filter(|p| p.0.is_finite() && p.1.is_finite())
        .collect();
    simplify(&pts, INK_TOLERANCE)
        .into_iter()
        .flat_map(|(x, y)| [x, y])
        .collect()
}

/// Fewer points along much the same line (Douglas–Peucker): every point dropped is within
/// `tol` of the line kept. The first and last points always stay.
pub(crate) fn simplify(p: &[(f32, f32)], tol: f32) -> Vec<(f32, f32)> {
    if p.len() < 3 {
        return p.to_vec();
    }
    let mut keep = vec![false; p.len()];
    keep[0] = true;
    keep[p.len() - 1] = true;
    let mut todo = vec![(0, p.len() - 1)];
    while let Some((a, b)) = todo.pop() {
        let (pa, pb) = (p[a], p[b]);
        let (dx, dy) = (pb.0 - pa.0, pb.1 - pa.1);
        let len = dx.hypot(dy);
        let off = |q: (f32, f32)| {
            if len < 1e-6 {
                (q.0 - pa.0).hypot(q.1 - pa.1)
            } else {
                ((q.0 - pa.0) * dy - (q.1 - pa.1) * dx).abs() / len
            }
        };
        let far = (a + 1..b)
            .map(|i| (i, off(p[i])))
            .max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((i, _)) = far.filter(|&(_, d)| d > tol) {
            keep[i] = true;
            todo.push((a, i));
            todo.push((i, b));
        }
    }
    p.iter()
        .zip(keep)
        .filter_map(|(&q, k)| k.then_some(q))
        .collect()
}

/// `v` as x, y pairs, each moved by `m`.
fn map_points(m: mx::M, v: &[f32]) -> Vec<f32> {
    v.as_chunks::<2>()
        .0
        .iter()
        .flat_map(|&[x, y]| {
            let (x, y) = mx::apply(m, x, y);
            [x, y]
        })
        .collect()
}

/// The time now as a PDF date in UTC.
fn pdf_now() -> String {
    let [y, mo, d, h, mi, s] = utc_now();
    format!("D:{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplifying_keeps_corners_and_drops_wobble() {
        // A straight run with a small wobble, then a corner.
        let p = [
            (0.0, 0.0),
            (5.0, 0.1),
            (10.0, -0.1),
            (15.0, 0.0),
            (15.0, 10.0),
        ];
        assert_eq!(simplify(&p, 0.35), [(0.0, 0.0), (15.0, 0.0), (15.0, 10.0)]);
        assert_eq!(simplify(&p[..2], 0.35), &p[..2]);
        // A stroke that comes back to where it started keeps its far point.
        let back = [(0.0, 0.0), (5.0, 5.0), (0.0, 0.0)];
        assert_eq!(simplify(&back, 0.35), back);
    }
}
