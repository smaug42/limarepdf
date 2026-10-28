//! What the user has selected in a document. There is one selection at a time: pages in
//! the thumbnails, objects on one page (one or several with the arrow), a run of words,
//! possibly across pages (read tool), or one annotation. Selecting one kind replaces
//! another, except that clearing objects, words or an annotation leaves a page selection
//! alone.

use limarepdf_core::{ObjectKind, PageObject};
use std::collections::BTreeSet;
use std::ops::RangeInclusive;

/// A word: its page and its index in that page's words.
pub type WordAt = (usize, usize);

/// Where Shift with an arrow, Home or End moves the moving end of words selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toward {
    /// The word before, in reading order.
    Left,
    /// The word after.
    Right,
    /// The line above, the word nearest across.
    Up,
    /// The line below.
    Down,
    /// The first word of its line.
    LineStart,
    /// The last word of its line.
    LineEnd,
}

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

/// A selected object: its index in its page's object list, and what it is, so that it can
/// be found again when a change to the page moves it in the list.
#[derive(Clone, Debug, PartialEq)]
pub struct Selected {
    pub id: usize,
    pub kind: ObjKind,
    /// Where it is, or where a change just made is to have put it (page points).
    pub bbox: [f32; 4],
    /// A line's text.
    pub text: Option<String>,
}

impl Selected {
    pub fn of(id: usize, o: &PageObject) -> Self {
        Selected {
            id,
            kind: ObjKind::of(o),
            bbox: o.bbox,
            text: o.text.clone(),
        }
    }

    /// Whether `o` can be this object: of its kind, with its text, and where it is (its box
    /// meets this one's).
    fn fits(&self, o: &PageObject) -> bool {
        // Boxes of lines and thin shapes can be flat.
        const TOL: f32 = 1.0;
        let (a, b) = (self.bbox, o.bbox);
        ObjKind::of(o) == self.kind
            && o.text == self.text
            && a[0] <= b[2] + TOL
            && b[0] <= a[2] + TOL
            && a[1] <= b[3] + TOL
            && b[1] <= a[3] + TOL
    }

    /// How much of `o`'s box and this one's overlap, in square points.
    fn overlap(&self, o: &PageObject) -> f32 {
        let (a, b) = (self.bbox, o.bbox);
        let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
        let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
        w * h
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Selection {
    #[default]
    None,
    /// Pages chosen in the thumbnails; never empty.
    Pages(BTreeSet<usize>),
    /// Objects on one page, in the order they were selected; never empty. `loose`: page
    /// text among them moves and resizes with the read tool, as text just added does
    /// (Move or resize text).
    Objects {
        page: usize,
        held: Vec<Selected>,
        loose: bool,
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

    /// Objects `held` on `page`; `None` when there are none.
    pub fn objects_from(page: usize, held: Vec<Selected>) -> Self {
        if held.is_empty() {
            Selection::None
        } else {
            Selection::Objects {
                page,
                held,
                loose: false,
            }
        }
    }

    /// The selected object's page and index, when one object is selected.
    pub fn object(&self) -> Option<(usize, usize)> {
        match self {
            Selection::Objects { page, held, .. } if held.len() == 1 => Some((*page, held[0].id)),
            _ => None,
        }
    }

    /// The selected objects' page and the objects, one or more.
    pub fn objects(&self) -> Option<(usize, &[Selected])> {
        match self {
            Selection::Objects { page, held, .. } => Some((*page, held)),
            _ => None,
        }
    }

    /// The selected objects' page and their indices.
    pub fn object_ids(&self) -> Option<(usize, Vec<usize>)> {
        let (page, held) = self.objects()?;
        Some((page, held.iter().map(|h| h.id).collect()))
    }

    /// Whether object `id` on `page` is selected.
    pub fn holds(&self, page: usize, id: usize) -> bool {
        self.objects()
            .is_some_and(|(p, held)| p == page && held.iter().any(|h| h.id == id))
    }

    /// Whether the selected objects' page text moves with the read tool.
    pub fn loose(&self) -> bool {
        matches!(self, Selection::Objects { loose: true, .. })
    }

    /// Let the selected objects' page text move with the read tool (or not).
    pub fn set_loose(&mut self, on: bool) {
        if let Selection::Objects { loose, .. } = self {
            *loose = on;
        }
    }

    /// Let go of the selected objects `gone` names, the others staying as they are.
    pub fn let_go(&mut self, gone: impl Fn(&Selected) -> bool) {
        if let Selection::Objects { held, .. } = self {
            held.retain(|h| !gone(h));
            if held.is_empty() {
                *self = Selection::None;
            }
        }
    }

    /// Before a change to `page`: `f` says, of each selected object there, where it will be
    /// or what it will read, so it can be found again ([`Selection::refind`]).
    pub fn expect(&mut self, page: usize, mut f: impl FnMut(&mut Selected)) {
        if let Selection::Objects { page: on, held, .. } = self {
            if *on == page {
                held.iter_mut().for_each(&mut f);
            }
        }
    }

    /// After `page` changed, its objects now `objs`: find each selected object again, at its
    /// index when the object there fits it ([`Selected::fits`]), else the one that fits it and
    /// overlaps it most (a rewritten line goes to the end of the page's content), else let
    /// it go, rather than hold another object in its place.
    pub fn refind(&mut self, page: usize, objs: &[PageObject]) {
        let Selection::Objects { page: on, held, .. } = self else {
            return;
        };
        if *on != page {
            return;
        }
        // Those still in their places first, so none is taken by one that moved.
        let mut found: Vec<Option<usize>> = (held.iter())
            .map(|h| objs.get(h.id).filter(|o| h.fits(o)).map(|_| h.id))
            .collect();
        let mut taken: BTreeSet<usize> = found.iter().flatten().copied().collect();
        for (h, f) in held.iter().zip(&mut found) {
            if f.is_some() {
                continue;
            }
            *f = (objs.iter().enumerate())
                .filter(|(i, o)| !taken.contains(i) && h.fits(o))
                .max_by(|(_, a), (_, b)| h.overlap(a).total_cmp(&h.overlap(b)))
                .map(|(i, _)| i);
            taken.extend(*f);
        }
        let mut now = found.into_iter();
        held.retain_mut(|h| match now.next().flatten() {
            Some(i) => {
                *h = Selected::of(i, &objs[i]);
                true
            }
            None => false,
        });
        if held.is_empty() {
            *self = Selection::None;
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

    /// Clear an object selection, of one object or several; anything else stays.
    pub fn clear_object(&mut self) {
        if matches!(self, Selection::Objects { .. }) {
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
            Selection::Objects { page, .. } | Selection::Annot { page, .. } if *page >= n => {
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

    /// An object of `kind` at `bbox`, reading `text` if it is a line.
    fn obj(kind: ObjectKind, bbox: [f32; 4], text: Option<&str>) -> PageObject {
        PageObject {
            id: 0,
            kind,
            bbox,
            text: text.map(String::from),
            font_name: None,
            font_size: None,
            fill: None,
            stroke: None,
            line_width: None,
            has_fill: false,
            has_stroke: false,
            inline: false,
            font_reusable: false,
            hidden: false,
            baseline: None,
            dir: None,
            stops: Vec::new(),
            upright: true,
            vertical: false,
            spans: Vec::new(),
        }
    }

    fn line(y: f32, text: &str) -> PageObject {
        obj(ObjectKind::Text, [20.0, y, 120.0, y + 12.0], Some(text))
    }

    fn holding(objs: &[PageObject], ids: &[usize]) -> Selection {
        let held = ids.iter().map(|&i| Selected::of(i, &objs[i])).collect();
        Selection::objects_from(0, held)
    }

    #[test]
    fn selection_on_a_removed_page_is_dropped() {
        let path = obj(ObjectKind::Path, [0.0, 0.0, 10.0, 10.0], None);
        let mut s = Selection::objects_from(3, vec![Selected::of(0, &path)]);
        s.retain_pages(3);
        assert_eq!(s, Selection::None);
        let mut s = Selection::objects_from(2, vec![Selected::of(0, &path)]);
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

    #[test]
    fn one_object_or_several() {
        let objs = [line(10.0, "A"), line(30.0, "B"), line(50.0, "C")];
        assert_eq!(Selection::objects_from(0, Vec::new()), Selection::None);
        let one = holding(&objs, &[1]);
        assert_eq!(one.object(), Some((0, 1)));
        let two = holding(&objs, &[2, 0]);
        assert_eq!(two.object(), None, "not one object");
        assert_eq!(two.object_ids(), Some((0, vec![2, 0])));
        assert!(two.holds(0, 0) && two.holds(0, 2) && !two.holds(0, 1) && !two.holds(1, 0));
        let mut s = two.clone();
        s.clear_words();
        s.clear_annot();
        assert_eq!(s, two);
        s.clear_object();
        assert_eq!(s, Selection::None);
    }

    #[test]
    fn objects_are_found_again_after_a_change() {
        let objs = [
            obj(ObjectKind::Path, [0.0, 0.0, 10.0, 10.0], None),
            line(30.0, "One"),
            line(50.0, "Two"),
            line(70.0, "Three"),
        ];
        // Unchanged: all where they were.
        let mut s = holding(&objs, &[1, 3]);
        s.refind(0, &objs);
        assert_eq!(s, holding(&objs, &[1, 3]));
        // Another page changed: nothing to do.
        s.refind(1, &[]);
        assert_eq!(s, holding(&objs, &[1, 3]));

        // "One" and "Three" restyled: rewritten, they go to the end of the content, a little
        // larger, and "Two" and the shape move up the list.
        let after = [
            objs[0].clone(),
            objs[2].clone(),
            obj(ObjectKind::Text, [20.0, 28.0, 130.0, 44.0], Some("One")),
            obj(ObjectKind::Text, [20.0, 68.0, 130.0, 84.0], Some("Three")),
        ];
        s.refind(0, &after);
        assert_eq!(s.object_ids(), Some((0, vec![2, 3])), "{s:?}");
        // What they are now is what is held, for the next change.
        assert_eq!(s.objects().unwrap().1[0].bbox, [20.0, 28.0, 130.0, 44.0]);

        // A move: where each is to be is said first; at its index, it fits.
        let mut s = holding(&objs, &[0, 2]);
        s.expect(0, |h| {
            h.bbox = [h.bbox[0] + 150.0, h.bbox[1], h.bbox[2] + 150.0, h.bbox[3]]
        });
        let mut moved = objs.clone();
        moved[0].bbox = [150.0, 0.0, 160.0, 10.0];
        moved[2].bbox = [170.0, 50.0, 270.0, 62.0];
        s.refind(0, &moved);
        assert_eq!(s.object_ids(), Some((0, vec![0, 2])));
        // Not said: the object at its index no longer fits there, and nothing else does.
        let mut s = holding(&objs, &[0, 2]);
        s.refind(0, &moved);
        assert_eq!(s, Selection::None, "never another object in its place");
    }

    #[test]
    fn an_object_gone_is_let_go_not_swapped_for_another() {
        let objs = [line(30.0, "One"), line(50.0, "Two"), line(70.0, "Two")];
        // The first "Two" gone: the other at its index reads the same but is elsewhere.
        let mut s = holding(&objs, &[1]);
        s.refind(0, &[objs[0].clone(), objs[2].clone()]);
        assert_eq!(s, Selection::None);
        // Of two held, the one that is gone goes, the other stays, at its new index.
        let mut s = holding(&objs, &[0, 1]);
        s.set_loose(true);
        s.refind(0, &[objs[1].clone(), objs[2].clone()]);
        assert_eq!(s.object_ids(), Some((0, vec![0])));
        assert!(s.loose(), "the selection stays as it was otherwise");
        // Two lines alike that changed places in the list: each is found where it is on the
        // page, not by its old index.
        let mut s = holding(&objs, &[1, 2]);
        let swapped = [objs[0].clone(), objs[2].clone(), objs[1].clone()];
        s.refind(0, &swapped);
        let held = s.objects().unwrap().1;
        assert_eq!(
            held.iter().map(|h| (h.id, h.bbox[1])).collect::<Vec<_>>(),
            [(2, 50.0), (1, 70.0)]
        );
    }
}
