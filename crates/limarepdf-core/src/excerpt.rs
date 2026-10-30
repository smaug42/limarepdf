//! New documents made from part of a document: its pages cropped to a selection, or a
//! text laid out afresh. Used to print or save selected text.

use crate::embed::writable;
use crate::{FontChoice, Pdf, Result};
use anyhow::bail;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Stream};
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
