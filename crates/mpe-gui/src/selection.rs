//! What the user has selected in a document. There is one selection at a time: pages in
//! the thumbnails, one object on a page (arrow tool), a run of words, possibly across
//! pages (read tool), or one annotation. Selecting one kind replaces another, except that
//! clearing an object, words or an annotation leaves a page selection alone.

use mpe_core::{ObjectKind, PageObject};
use std::collections::BTreeSet;
use std::ops::RangeInclusive;

/// A word: its page and its index in that page's words.
pub type WordAt = (usize, usize);

/// What kind of object is selected, for deciding what can be done with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjKind {
    Text,
    /// Invisible text, such as an OCR layer.
    HiddenText,
    Image,
    Path,
    /// A form XObject: a reusable group of content.
    Form,
}

impl ObjKind {
    pub fn of(o: &PageObject) -> Self {
        match o.kind {
            ObjectKind::Text if o.hidden => ObjKind::HiddenText,
            ObjectKind::Text => ObjKind::Text,
            ObjectKind::Image => ObjKind::Image,
            ObjectKind::Path => ObjKind::Path,
            ObjectKind::Form => ObjKind::Form,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    #[default]
    None,
    /// Pages chosen in the thumbnails; never empty.
    Pages(BTreeSet<usize>),
    /// One object, by page and index in that page's object list.
    Object {
        page: usize,
        id: usize,
        kind: ObjKind,
    },
    /// Words in reading order from `anchor`, where the selection started, to `end`, which
    /// moves; `end` may come before `anchor`.
    Words { anchor: WordAt, end: WordAt },
    /// One annotation (a comment or markup), by its page and object id.
    Annot { page: usize, id: (u32, u16) },
}

impl Selection {
    /// Pages from a set: `None` when the set is empty.
    pub fn pages_from(set: BTreeSet<usize>) -> Self {
        if set.is_empty() {
            Selection::None
        } else {
            Selection::Pages(set)
        }
    }

    pub fn pages(&self) -> Option<&BTreeSet<usize>> {
        match self {
            Selection::Pages(s) => Some(s),
            _ => None,
        }
    }

    /// The selected object's page and index.
    pub fn object(&self) -> Option<(usize, usize)> {
        match *self {
            Selection::Object { page, id, .. } => Some((page, id)),
            _ => None,
        }
    }

    /// One word.
    pub fn word(at: WordAt) -> Self {
        Selection::Words {
            anchor: at,
            end: at,
        }
    }

    /// The selected words' first and last word, in reading order.
    pub fn words(&self) -> Option<(WordAt, WordAt)> {
        match *self {
            Selection::Words { anchor, end } => Some((anchor.min(end), anchor.max(end))),
            _ => None,
        }
    }

    /// The selected word indices on `page`, which has `n` words.
    pub fn words_on(&self, page: usize, n: usize) -> Option<RangeInclusive<usize>> {
        let (a, b) = self.words()?;
        if page < a.0 || page > b.0 || n == 0 {
            return None;
        }
        let from = if page == a.0 { a.1 } else { 0 };
        let to = if page == b.0 { b.1.min(n - 1) } else { n - 1 };
        (from <= to).then_some(from..=to)
    }

    /// The selected annotation's page and id.
    pub fn annot(&self) -> Option<(usize, (u32, u16))> {
        match *self {
            Selection::Annot { page, id } => Some((page, id)),
            _ => None,
        }
    }

    /// Clear an annotation selection; anything else stays.
    pub fn clear_annot(&mut self) {
        if matches!(self, Selection::Annot { .. }) {
            *self = Selection::None;
        }
    }

    /// Clear an object selection; anything else stays.
    pub fn clear_object(&mut self) {
        if matches!(self, Selection::Object { .. }) {
            *self = Selection::None;
        }
    }

    /// Clear a word selection; anything else stays.
    pub fn clear_words(&mut self) {
        if matches!(self, Selection::Words { .. }) {
            *self = Selection::None;
        }
    }

    /// After the page count changes: keep only pages below `n`, and drop an object or
    /// word selection on a page that no longer exists.
    pub fn retain_pages(&mut self, n: usize) {
        match self {
            Selection::Pages(s) => {
                s.retain(|&p| p < n);
                if s.is_empty() {
                    *self = Selection::None;
                }
            }
            Selection::Object { page, .. } | Selection::Annot { page, .. } if *page >= n => {
                *self = Selection::None
            }
            Selection::Words { anchor, end } if anchor.0.max(end.0) >= n => {
                *self = Selection::None;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(v: &[usize]) -> Selection {
        Selection::pages_from(v.iter().copied().collect())
    }

    #[test]
    fn clearing_objects_or_words_keeps_pages() {
        let mut s = pages(&[1, 2]);
        s.clear_object();
        s.clear_words();
        assert_eq!(s, pages(&[1, 2]));
        let mut s = Selection::Words {
            anchor: (0, 3),
            end: (0, 1),
        };
        s.clear_object();
        assert!(s.words().is_some(), "clear_object must not drop words");
        s.clear_words();
        assert_eq!(s, Selection::None);
    }

    #[test]
    fn empty_page_set_is_no_selection() {
        assert_eq!(pages(&[]), Selection::None);
        let mut s = pages(&[4, 5]);
        s.retain_pages(4);
        assert_eq!(s, Selection::None);
        let mut s = pages(&[1, 5]);
        s.retain_pages(4);
        assert_eq!(s, pages(&[1]));
    }

    #[test]
    fn selection_on_a_removed_page_is_dropped() {
        let mut s = Selection::Object {
            page: 3,
            id: 0,
            kind: ObjKind::Path,
        };
        s.retain_pages(3);
        assert_eq!(s, Selection::None);
        let mut s = Selection::Object {
            page: 2,
            id: 0,
            kind: ObjKind::Path,
        };
        s.retain_pages(3);
        assert_eq!(s.object(), Some((2, 0)));
        let mut s = Selection::Annot {
            page: 3,
            id: (7, 0),
        };
        s.retain_pages(3);
        assert_eq!(s, Selection::None);
        let mut s = Selection::Words {
            anchor: (1, 0),
            end: (3, 2),
        };
        s.retain_pages(3);
        assert_eq!(s, Selection::None);
    }

    #[test]
    fn words_across_pages() {
        // Dragged backwards from page 3, word 2, to page 1, word 5.
        let s = Selection::Words {
            anchor: (3, 2),
            end: (1, 5),
        };
        assert_eq!(s.words(), Some(((1, 5), (3, 2))));
        assert_eq!(s.words_on(0, 10), None);
        assert_eq!(s.words_on(1, 10), Some(5..=9));
        assert_eq!(s.words_on(2, 4), Some(0..=3));
        assert_eq!(s.words_on(2, 0), None);
        assert_eq!(s.words_on(3, 10), Some(0..=2));
        assert_eq!(s.words_on(4, 10), None);
        // A start past the page's last word selects nothing there.
        assert_eq!(s.words_on(1, 5), None);
        let one = Selection::word((2, 7));
        assert_eq!(one.words_on(2, 8), Some(7..=7));
        assert_eq!(one.words_on(2, 3), None);
    }
}
