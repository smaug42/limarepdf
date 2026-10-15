//! Adding and changing annotations: text markup over lines of text, notes and text
//! boxes, an annotation's colour, opacity and comment, and its place on the page. Each
//! gets a drawing of its own straight away, so other viewers show it as MPE does.

use crate::annot_ap::{DRAWN, NOTE_SIZE};
use crate::doc::encode_text;
use crate::form_ap::{lines_height, nums};
use crate::form_write::{parse_da, standard_metrics};
use crate::matrix as mx;
use crate::overlay::utc_now;
use crate::{AnnotKind, Pdf, Result, TextStyle};
use anyhow::{anyhow, bail};
use lopdf::{dictionary, Dictionary, Object, ObjectId, StringFormat};

/// Annotation flags (`/F`): printed; not zoomed or turned with the page (notes).
const PRINT: i64 = 1 << 2;
const NO_ZOOM: i64 = 1 << 3;
const NO_ROTATE: i64 = 1 << 4;
/// Room around the lines in `/Rect`, for the wavy line and strokes on the edge.
const PAD: f32 = 1.0;

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
    /// match. A move keeps the drawing it has; a change of size draws it again when MPE
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
        let page = match d.get(b"P").and_then(|o| o.as_reference()) {
            Ok(p) => p,
            Err(_) => self
                .page_ids()
                .into_iter()
                .find(|&p| self.annot_refs(p).contains(&id))
                .ok_or_else(|| anyhow!("the annotation is on no page"))?,
        };
        let pb = self.page_box(page);
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
    fn insert_annot(
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
        let name = format!("mpe-{}-{}", id.0, pdf_now().trim_start_matches("D:"));
        if let Ok(a) = self.doc.get_dictionary_mut(id) {
            a.set("NM", Object::string_literal(name));
        }
        self.push_annot(page, id)?;
        if self.doc.get_dictionary(id)?.get(b"Subtype")?.as_name()? == b"FreeText" {
            self.fit_text_box(id)?;
        }
        self.draw_annotations(&[id]);
        Ok(id)
    }

    /// Change an annotation's colour (`None` for none), opacity (0 to 1) and comment.
    /// Kinds MPE draws are drawn again when their look changes. Locked annotations are
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
        Ok(())
    }

    /// Add `annot` to the end of the page's `/Annots`, which may be an array in the page
    /// or one the page refers to.
    fn push_annot(&mut self, page: ObjectId, annot: ObjectId) -> Result<()> {
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
