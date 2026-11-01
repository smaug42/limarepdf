//! New documents made from part of a document: its pages cropped to a selection, or a
//! text laid out afresh. Used to print or save selected text. And a page alone without
//! some of its objects, which the app shows under text being edited.

use crate::embed::writable;
use crate::{FontChoice, Pdf, Result};
use anyhow::bail;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use std::collections::HashSet;

/// Font size and line spacing of [`Pdf::from_text`], in points.
const TEXT_SIZE: f32 = 11.0;
const LEADING: f32 = 14.5;

impl Pdf {
    /// A new document of the given pages, in the given order, each cropped to a rectangle
    /// in displayed-page points (y down). Rectangles are clipped to their page; each page
    /// may appear once.
    pub fn excerpt(&self, parts: &[(usize, [f32; 4])]) -> Result<Pdf> {
        let idx: Vec<usize> = parts.iter().map(|p| p.0).collect();
        if idx.iter().collect::<HashSet<_>>().len() != idx.len() {
            bail!("a page is listed twice");
        }
        let mut out = self.extract(&idx)?;
        for (i, &(page, r)) in parts.iter().enumerate() {
            let (w, h) = self.page_size(page)?;
            let [x0, x1] = [r[0], r[2]].map(|v| v.clamp(0.0, w));
            let [y0, y1] = [r[1], r[3]].map(|v| v.clamp(0.0, h));
            if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
                bail!("nothing to keep on page {}", page + 1);
            }
            let [t, rt, b, l] = self.crop_margins(page)?;
            out.set_crop_margins(i, [t + y0, rt + w - x1, b + h - y1, l + x0])?;
        }
        Ok(out)
    }

    /// A new document with `text` set in 11-point Helvetica on pages of `size` (width and
    /// height in points), wrapped within the margins, or in its embedded face when the text
    /// needs it. Line breaks are kept. Characters neither can write (beyond Latin, Greek
    /// and Cyrillic) come out as `?`.
    pub fn from_text(text: &str, size: (f32, f32)) -> Result<Pdf> {
        if text.trim().is_empty() {
            bail!("there is no text");
        }
        let helvetica = FontChoice::default();
        let text: String = text
            .chars()
            .map(|c| match c {
                c if c.is_whitespace() || writable(helvetica, c) => c,
                _ => '?',
            })
            .collect();
        let (w, h) = size;
        let margin = (w.min(h) / 8.5).clamp(18.0, 72.0);
        let width = w - 2.0 * margin;
        let per_page = ((h - 2.0 * margin) / LEADING).floor() as usize;
        if width < 36.0 || per_page == 0 {
            bail!("the page is too small for text");
        }
        let mut out = Pdf::new();
        let setter = out.setter(helvetica, &text);
        // A line break at the end starts no line of its own.
        let body = text.strip_suffix('\n').unwrap_or(&text);
        let body = body.strip_suffix('\r').unwrap_or(body);
        let lines = setter.lines(body, width, TEXT_SIZE);
        let shown: Vec<&str> = lines.iter().map(String::as_str).collect();
        let pen = out.pen_with(&setter, &shown)?;
        let names = pen.names("F1");
        for (n, chunk) in lines.chunks(per_page).enumerate() {
            let mut ops = vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), TEXT_SIZE.into()]),
                Operation::new("TL", vec![LEADING.into()]),
                Operation::new("Td", vec![margin.into(), (h - margin - TEXT_SIZE).into()]),
            ];
            for i in (n * per_page)..(n * per_page + chunk.len()) {
                ops.extend(pen.ops(i, TEXT_SIZE, 0.0, &names));
                ops.push(Operation::new("T*", vec![]));
            }
            ops.push(Operation::new("ET", vec![]));
            let content = Content { operations: ops }.encode()?;
            out.insert_blank(n, w, h)?;
            let contents = out.doc.add_object(Stream::new(dictionary! {}, content));
            let id = out.page_ids()[n];
            let page = out.doc.get_dictionary_mut(id)?;
            page.set("Contents", contents);
            page.set("Resources", dictionary! { "Font" => pen.font_dict(&names) });
        }
        Ok(out)
    }

    /// Page `page_index` alone, as a new document's bytes, with the objects `ids` (as
    /// [`Pdf::page_objects`] numbers them) taken out as [`Pdf::delete_objects`] takes them, a
    /// line's underlines and strike-throughs with it: what the app shows under text being
    /// edited, so its old lines don't show through (ENH-21). The page keeps its boxes, turn, resources and transparency group, and the
    /// document's optional content settings (a hidden layer stays hidden); its comments and
    /// fields that have looks of their own are drawn as the app's pages draw them (PB-76;
    /// sticky notes, which the app draws over the page itself, left out). Only
    /// what the page's content, resources and annotations' looks use is copied, so it costs
    /// as much in a long document as in a short one. The document itself is not changed.
    pub fn page_without(&self, page_index: usize, ids: &[usize]) -> Result<Vec<u8>> {
        let page = self.page_id_at(page_index)?;
        let parsed = self.parse_page_of(page)?;
        let mut ops = Self::without_objects(&parsed, &Self::with_decorations(&parsed, ids))?;
        ops.extend((0..parsed.depth_left).map(|_| Operation::new("Q", vec![])));
        let content = (parsed.inline.encode(ops))
            .map_err(|e| anyhow::anyhow!("cannot encode the page content: {e}"))?;
        let src = self.doc.get_dictionary(page)?;
        let mut d = dictionary! {
            "Type" => "Page",
            "MediaBox" => rect(self.media_box(page)),
            "Rotate" => self.page_rotation(page),
        };
        if let Some(crop) = self.inherited(page, b"CropBox").and_then(|o| self.rect(&o)) {
            d.set("CropBox", rect(crop));
        }
        let res = self.inherited(page, b"Resources");
        let group = src.get(b"Group").ok().cloned();
        let unit = src.get(b"UserUnit").ok().cloned();
        let layers = (self.doc.catalog().ok())
            .and_then(|c| c.get(b"OCProperties").ok())
            .cloned();
        let annots = self.drawn_annots(page);
        // Objects keep their numbers, so what refers to them still does.
        let mut out = Document::with_version("1.7");
        let roots = [&res, &group, &layers];
        let looks = annots.iter().map(|(_, a)| a);
        self.copy_used(roots.into_iter().flatten().chain(looks), &mut out);
        for (key, value) in [("Resources", res), ("Group", group), ("UserUnit", unit)] {
            if let Some(v) = value {
                d.set(key, v);
            }
        }
        out.max_id = self.doc.max_id;
        if !annots.is_empty() {
            // Each in its own number again, as an annotation is referred to.
            for (id, a) in &annots {
                out.objects.insert(*id, a.clone());
            }
            let refs: Vec<Object> = annots.iter().map(|(id, _)| (*id).into()).collect();
            d.set("Annots", refs);
        }
        let contents = out.add_object(Stream::new(dictionary! {}, content));
        let page = out.new_object_id();
        let pages = out.add_object(dictionary! {
            "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
        });
        d.set("Contents", contents);
        d.set("Parent", pages);
        out.objects.insert(page, Object::Dictionary(d));
        let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => pages };
        if let Some(layers) = layers {
            catalog.set("OCProperties", layers);
        }
        let catalog = out.add_object(catalog);
        out.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        out.save_to(&mut bytes)?;
        Ok(bytes)
    }

    /// Copy into `out` the objects `roots` refer to, and those they refer to in turn, with
    /// their numbers; never a page or the page tree, which a picture or a font can name.
    /// The annotations on `page` that the app's pages draw ([`Pdf::app_snapshot`]), each
    /// with its number, for a page alone: those with a look of their own, not hidden or
    /// kept from view, and not sticky notes (the app draws those over the page itself).
    /// Each is cut from what reaches the rest of the document: its page, its field's other
    /// parts and value, its pop-up, the comment it replies to, its actions.
    fn drawn_annots(&self, page: ObjectId) -> Vec<(ObjectId, Object)> {
        const REACHING: [&[u8]; 9] = [
            b"P", b"Parent", b"Kids", b"Popup", b"IRT", b"A", b"AA", b"Dest", b"V",
        ];
        let Some(annots) = (self.doc.get_dictionary(page).ok()).and_then(|d| d.get(b"Annots").ok())
        else {
            return Vec::new();
        };
        let Ok((_, Object::Array(annots))) = self.doc.dereference(annots) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for r in annots {
            let Ok(id) = r.as_reference() else { continue };
            let Ok(d) = self.doc.get_dictionary(id) else {
                continue;
            };
            let flags = d.get(b"F").ok().and_then(|f| self.num(f)).unwrap_or(0.0) as i64;
            let sub = d.get(b"Subtype").and_then(|o| o.as_name()).unwrap_or(b"");
            if flags & (2 | 32) != 0 || matches!(sub, b"Text" | b"Popup") || !d.has(b"AP") {
                continue;
            }
            let mut d = d.clone();
            for key in REACHING {
                d.remove(key);
            }
            out.push((id, Object::Dictionary(d)));
        }
        out
    }

    fn copy_used<'a>(&self, roots: impl IntoIterator<Item = &'a Object>, out: &mut Document) {
        let mut stack: Vec<&Object> = roots.into_iter().collect();
        while let Some(o) = stack.pop() {
            match o {
                Object::Reference(id) if !out.objects.contains_key(id) => {
                    let Ok(found) = self.doc.get_object(*id) else {
                        continue;
                    };
                    let tree = |d: &Dictionary| d.has_type(b"Page") || d.has_type(b"Pages");
                    if found.as_dict().is_ok_and(tree) {
                        continue;
                    }
                    out.objects.insert(*id, found.clone());
                    stack.push(found);
                }
                Object::Array(a) => stack.extend(a),
                Object::Dictionary(d) => stack.extend(d.iter().map(|(_, o)| o)),
                Object::Stream(s) => stack.extend(s.dict.iter().map(|(_, o)| o)),
                _ => {}
            }
        }
    }
}

/// A rectangle as a PDF array.
fn rect(r: [f32; 4]) -> Vec<Object> {
    r.iter().map(|&v| v.into()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::text_width;

    #[test]
    fn wraps_within_the_margins_and_breaks_long_words() {
        let lines = |text: &str| {
            let p = Pdf::from_text(text, (200.0, 300.0)).unwrap();
            let t = p.page_text(0);
            t.lines().map(str::to_string).collect::<Vec<_>>()
        };
        // 200 points wide: 23.5-point margins leave 153 for the text.
        let text = "the quick brown fox jumps over the lazy dog and runs";
        let got = lines(text);
        assert!(got.len() > 1, "{got:?}");
        assert!(got.iter().all(|l| text_width(l, TEXT_SIZE) <= 153.0));
        assert_eq!(got.join(" "), text);
        let long = lines(&"m".repeat(40));
        assert!(long.len() > 1);
        assert_eq!(long.concat(), "m".repeat(40));
    }
}
