//! Adding and changing annotations: text markup over lines of text, and an annotation's
//! colour, opacity and comment. Each gets a drawing of its own straight away, so other
//! viewers show it as MPE does.

use crate::annot_ap::DRAWN;
use crate::doc::encode_text;
use crate::form_ap::nums;
use crate::matrix as mx;
use crate::overlay::utc_now;
use crate::{AnnotKind, Pdf, Result};
use anyhow::{anyhow, bail};
use lopdf::{dictionary, Object, ObjectId, StringFormat};

/// The Print flag (`/F`), so the annotation is printed.
const PRINT: i64 = 1 << 2;
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
        let now = Object::string_literal(pdf_now());
        let mut d = dictionary! {
            "Type" => "Annot",
            "Subtype" => sub,
            "Rect" => nums(&[r[0] - PAD, r[1] - PAD, r[2] + PAD, r[3] + PAD]),
            "QuadPoints" => nums(&quads),
            "C" => nums(&color),
            "F" => PRINT,
            "P" => page,
            "M" => now.clone(),
            "CreationDate" => now,
        };
        if !author.is_empty() {
            d.set(
                "T",
                Object::String(encode_text(author), StringFormat::Literal),
            );
        }
        let id = self.doc.add_object(d);
        let name = format!("mpe-{}-{}", id.0, pdf_now().trim_start_matches("D:"));
        if let Ok(a) = self.doc.get_dictionary_mut(id) {
            a.set("NM", Object::string_literal(name));
        }
        self.push_annot(page, id)?;
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
        if contents.is_empty() {
            d.remove(b"Contents");
        } else {
            d.set(
                "Contents",
                Object::String(encode_text(contents), StringFormat::Literal),
            );
        }
        d.set("M", Object::string_literal(pdf_now()));
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

/// The time now as a PDF date in UTC.
fn pdf_now() -> String {
    let [y, mo, d, h, mi, s] = utc_now();
    format!("D:{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}Z")
}
