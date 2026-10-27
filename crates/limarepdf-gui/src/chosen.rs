//! Words selected with the read tool, or lines of page text selected with the arrow, as
//! Bold, Italic and the inspector's font, size and colour change them (ENH-05, ENH-06):
//! each line the words are in, its text as styled, and the characters selected there,
//! written back as the lines' changes in one undo step.

use crate::editor::font_of;
use crate::selection::WordAt;
use crate::styled::{Across, Styled};
use limarepdf_core::{ObjectKind, PageObject, PageWord, TextEdit};
use std::collections::BTreeSet;

/// Lines with selected words that can't be restyled, by why.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Left {
    /// Invisible text, such as recognised (OCR) text, which has no look to change.
    pub hidden: usize,
    /// Text inside a group (a form XObject), which isn't a line of the page's own.
    pub grouped: usize,
    /// Text in vertical writing, which would come out written across.
    pub vertical: usize,
}

impl Left {
    /// What was left as it was, for a notice; `None` when nothing was.
    pub fn said(&self) -> Option<String> {
        let lines = |n: usize| match n {
            1 => "1 line".to_string(),
            n => format!("{n} lines"),
        };
        let mut parts = Vec::new();
        if self.hidden > 0 {
            let n = lines(self.hidden);
            parts.push(format!("{n} of invisible recognised text"));
        }
        if self.grouped > 0 {
            parts.push(format!("{} inside a group", lines(self.grouped)));
        }
        if self.vertical > 0 {
            parts.push(format!("{} of vertical writing", lines(self.vertical)));
        }
        let last = parts.pop()?;
        Some(if parts.is_empty() {
            format!("Not changed: {last}")
        } else {
            format!("Not changed: {} and {last}", parts.join(", "))
        })
    }
}

/// What a [`Chosen`] was made from.
#[derive(Clone, Debug, PartialEq)]
pub enum Of {
    /// Words selected with the read tool: the first and last.
    Words((WordAt, WordAt)),
    /// Objects selected with the arrow, the lines of page text among them restyled whole:
    /// their page and indices.
    Lines(usize, Vec<usize>),
}

/// Words or lines selected, ready to restyle.
#[derive(Clone, Debug)]
pub struct Chosen {
    /// The selection, as it was when this was made.
    pub of: Of,
    /// Each line restyled: its page and object, and its text as it is in the document.
    lines: Vec<(usize, usize, Styled)>,
    /// The lines' text as restyled, and what is selected in each, in the same order.
    pub text: Across,
    /// The PDF fonts of the selected characters, each once.
    pub in_pdf: Vec<String>,
    pub left: Left,
}

impl Chosen {
    /// Nothing yet of the selection `of`.
    pub fn new(of: Of) -> Self {
        Self {
            of,
            lines: Vec::new(),
            text: Across::default(),
            in_pdf: Vec::new(),
            left: Left::default(),
        }
    }

    /// The selected words of page `page`, `words`, in reading order, on a page whose
    /// objects are `objs`. The words of a line selected one after another, with only spaces
    /// between them, are one run of its characters, the spaces with them.
    pub fn add_page(&mut self, page: usize, words: &[PageWord], objs: &[PageObject]) {
        let mut lines: Vec<(usize, Vec<(usize, usize)>)> = Vec::new();
        let mut grouped = BTreeSet::new();
        for w in words {
            match w.object {
                Some(id) => match lines.iter_mut().find(|(i, _)| *i == id) {
                    Some((_, chars)) => chars.push(w.chars),
                    None => lines.push((id, vec![w.chars])),
                },
                None => {
                    grouped.insert(w.line);
                }
            }
        }
        self.left.grouped += grouped.len();
        for (id, mut chars) in lines {
            let Some(o) = objs.get(id).filter(|o| o.kind == ObjectKind::Text) else {
                continue;
            };
            if o.hidden {
                self.left.hidden += 1;
                continue;
            }
            if o.vertical {
                self.left.vertical += 1;
                continue;
            }
            let all: Vec<char> = o.text.as_deref().unwrap_or("").chars().collect();
            chars.sort_unstable();
            let mut ranges: Vec<(usize, usize)> = Vec::new();
            for (a, b) in chars {
                match ranges.last_mut() {
                    Some(r) if r.1 <= a && all[r.1..a].iter().all(|c| c.is_whitespace()) => r.1 = b,
                    _ => ranges.push((a, b)),
                }
            }
            self.add_line(page, id, o, ranges);
        }
    }

    /// The lines of page text among objects `ids` of page `page`, whose objects are `objs`,
    /// each whole. Invisible lines and vertical writing are counted as left as they were;
    /// other objects aren't text and aren't counted.
    pub fn add_lines(&mut self, page: usize, ids: &[usize], objs: &[PageObject]) {
        for &id in ids {
            let Some(o) = objs.get(id).filter(|o| o.kind == ObjectKind::Text) else {
                continue;
            };
            if o.hidden {
                self.left.hidden += 1;
                continue;
            }
            if o.vertical {
                self.left.vertical += 1;
                continue;
            }
            let n = o.text.as_deref().unwrap_or("").chars().count();
            if n > 0 {
                self.add_line(page, id, o, vec![(0, n)]);
            }
        }
    }

    /// Line `o`, object `id` of page `page`, with characters `ranges` selected.
    fn add_line(&mut self, page: usize, id: usize, o: &PageObject, ranges: Vec<(usize, usize)>) {
        for s in &o.spans {
            let selected = ranges.iter().any(|&(a, b)| s.chars.0 < b && a < s.chars.1);
            if selected && !self.in_pdf.contains(&s.font_name) {
                self.in_pdf.push(s.font_name.clone());
            }
        }
        let text = Styled::of(o, font_of);
        self.lines.push((page, id, text.clone()));
        self.text.lines.push((text, ranges));
    }

    /// Whether any of the selected words can be restyled.
    pub fn restylable(&self) -> bool {
        !self.lines.is_empty()
    }

    /// Whether a line looks different from how it is in the document.
    pub fn changed(&self) -> bool {
        let now = self.text.lines.iter().map(|(t, _)| t);
        self.lines
            .iter()
            .zip(now)
            .any(|((.., was), t)| !t.same_as(was))
    }

    /// The changes made: for each page, in order, its changed lines' edits, each line by
    /// its object as the page is now.
    pub fn changes(&self) -> Vec<(usize, Vec<(usize, TextEdit)>)> {
        let mut out: Vec<(usize, Vec<(usize, TextEdit)>)> = Vec::new();
        let now = self.text.lines.iter().map(|(t, _)| t);
        for ((page, id, was), t) in self.lines.iter().zip(now) {
            if t.same_as(was) {
                continue;
            }
            let e = (*id, TextEdit::in_parts(t.parts()));
            match out.last_mut() {
                Some((p, edits)) if p == page => edits.push(e),
                _ => out.push((*page, vec![e])),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What was left as it was is said in plain words, each kind once.
    #[test]
    fn what_was_left_is_said() {
        assert_eq!(Left::default().said(), None);
        let one = Left {
            hidden: 1,
            ..Left::default()
        };
        let said = one.said().unwrap();
        assert_eq!(said, "Not changed: 1 line of invisible recognised text");
        let all = Left {
            hidden: 2,
            grouped: 1,
            vertical: 3,
        };
        assert_eq!(
            all.said().unwrap(),
            "Not changed: 2 lines of invisible recognised text, 1 line inside a group \
             and 3 lines of vertical writing"
        );
    }
}
