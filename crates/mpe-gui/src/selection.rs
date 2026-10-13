//! What the user has selected in a document. There is one selection at a time: pages in
//! the thumbnails, one object on a page (arrow tool), or a run of words (text tool).
//! Selecting one kind replaces another, except that clearing an object or words leaves
//! a page selection alone.

use mpe_core::{ObjectKind, PageObject};
use std::collections::BTreeSet;

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
    /// Words on one page: the first and last word index, in either order.
    Words {
        page: usize,
        first: usize,
        last: usize,
    },
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

    /// The selected words' page and first and last index (either order).
    pub fn words(&self) -> Option<(usize, usize, usize)> {
        match *self {
            Selection::Words { page, first, last } => Some((page, first, last)),
            _ => None,
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
            Selection::Object { page, .. } | Selection::Words { page, .. } if *page >= n => {
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
            page: 0,
            first: 3,
            last: 1,
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
    }
}
