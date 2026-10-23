//! New documents made from part of a document: its pages cropped to a selection, or a
//! text laid out afresh. Used to print or save selected text.

use crate::embed::writable;
use crate::overlay::text_width;
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
        let lines: Vec<String> = text.lines().flat_map(|l| wrap(l, width)).collect();

        let mut out = Pdf::new();
        let pen = out.pen(helvetica, &lines.concat())?;
        let font = out.pen_font(&pen);
        for (n, chunk) in lines.chunks(per_page).enumerate() {
            let mut ops = vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), TEXT_SIZE.into()]),
                Operation::new("TL", vec![LEADING.into()]),
                Operation::new("Td", vec![margin.into(), (h - margin - TEXT_SIZE).into()]),
            ];
            for line in chunk {
                ops.push(Operation::new("Tj", vec![pen.string(line)]));
                ops.push(Operation::new("T*", vec![]));
            }
            ops.push(Operation::new("ET", vec![]));
            let content = Content { operations: ops }.encode()?;
            out.insert_blank(n, w, h)?;
            let contents = out.doc.add_object(Stream::new(dictionary! {}, content));
            let id = out.page_ids()[n];
            let page = out.doc.get_dictionary_mut(id)?;
            page.set("Contents", contents);
            let fonts = dictionary! { "F1" => font };
            page.set("Resources", dictionary! { "Font" => fonts });
        }
        Ok(out)
    }
}

/// Break one line into lines no wider than `width` at the text size, at spaces where
/// possible. An empty line stays one empty line.
fn wrap(line: &str, width: f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in line.split_whitespace() {
        let joined = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if text_width(&joined, TEXT_SIZE) <= width {
            cur = joined;
            continue;
        }
        if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        // A word wider than the line is broken wherever it overflows.
        for c in word.chars() {
            cur.push(c);
            if text_width(&cur, TEXT_SIZE) > width && cur.chars().count() > 1 {
                cur.pop();
                out.push(std::mem::replace(&mut cur, c.to_string()));
            }
        }
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_spaces_and_breaks_long_words() {
        assert_eq!(wrap("", 100.0), vec![String::new()]);
        assert_eq!(wrap("a b", 100.0), vec!["a b"]);
        let text = "the quick brown fox jumps over the lazy dog";
        let lines = wrap(text, 60.0);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| text_width(l, TEXT_SIZE) <= 60.0));
        assert_eq!(lines.join(" "), text);
        let long = wrap(&"m".repeat(40), 60.0);
        assert!(long.len() > 1);
        assert_eq!(long.concat(), "m".repeat(40));
    }
}
