//! Undo history (PF-42): what an edit changed in a document, kept as the values of the
//! objects it changed. The document is copied before the edit and compared with itself after
//! it; only what differs is kept, so a note added to a scanned document keeps a few
//! kilobytes rather than a copy of every page. Undo and Redo put the values in place, without
//! reading the document again, so what was read when it was opened (that it was repaired,
//! decrypted, or reads differently in other apps) stays as it was.

use crate::Pdf;
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::BTreeMap;
use std::mem::{size_of, swap, take};

/// A document as it was before an edit: compared with it after the edit ([`Before::undo`]),
/// or put back when the edit fails ([`Pdf::go_back`]).
pub struct Before(Document);

/// What puts a document back as it was before an edit: the old values of the objects the edit
/// changed or removed, the objects it added (to be removed again), and the trailer and the
/// version when the edit changed them; the highest object number is never lowered (see
/// [`Pdf::restore`]). What lopdf read when the document was opened (the cross-reference data,
/// the encryption state) is no edit's to change and is left as it is.
#[derive(Clone)]
pub struct Restore {
    /// Each object's value to put back; `None`: it didn't exist, so it is removed.
    objects: BTreeMap<ObjectId, Option<Object>>,
    header: Option<Header>,
    size: usize,
}

/// The document's fields besides its objects that an edit may change.
#[derive(Clone)]
struct Header {
    version: String,
    binary_mark: Vec<u8>,
    trailer: Dictionary,
    max_id: u32,
}

impl Header {
    fn take(doc: &mut Document) -> Self {
        Header {
            version: take(&mut doc.version),
            binary_mark: take(&mut doc.binary_mark),
            trailer: take(&mut doc.trailer),
            max_id: doc.max_id,
        }
    }

    fn same(&self, doc: &Document) -> bool {
        self.version == doc.version
            && self.binary_mark == doc.binary_mark
            && self.max_id == doc.max_id
            && same_dict(&self.trailer, &doc.trailer)
    }

    fn swap(&mut self, doc: &mut Document) {
        swap(&mut self.version, &mut doc.version);
        swap(&mut self.binary_mark, &mut doc.binary_mark);
        swap(&mut self.trailer, &mut doc.trailer);
        swap(&mut self.max_id, &mut doc.max_id);
    }

    fn size(&self) -> usize {
        self.version.capacity() + self.binary_mark.capacity() + dict_heap(&self.trailer)
    }
}

impl Pdf {
    /// The document as it is now, before an edit.
    pub fn before(&self) -> Before {
        Before(self.doc.clone())
    }

    /// Put the document back as it was before an edit that failed, but for the highest
    /// object number (see [`Pdf::restore`]). The fonts LimarePDF embedded meanwhile stay
    /// known, as [`Pdf::reopen`] keeps them.
    pub fn go_back(&mut self, before: Before) {
        let top = self.doc.max_id;
        self.doc = before.0;
        self.doc.max_id = self.doc.max_id.max(top);
    }

    /// Put a [`Restore`]'s values in place: Undo, or Redo with the one that reversed it.
    /// Returns the [`Restore`] that reverses this one. The highest object number isn't
    /// lowered: a number an object had isn't given to another in the session, so what the
    /// app keeps by object (a draft reply by its comment, the selection, pages) can't come
    /// to mean another one after Undo.
    pub fn restore(&mut self, r: Restore) -> Restore {
        let top = self.doc.max_id;
        let objects = (r.objects.into_iter())
            .map(|(id, value)| {
                let was = match value {
                    Some(o) => self.doc.objects.insert(id, o),
                    None => self.doc.objects.remove(&id),
                };
                (id, was)
            })
            .collect();
        let header = r.header.map(|mut h| {
            h.swap(&mut self.doc);
            h
        });
        self.doc.max_id = self.doc.max_id.max(top);
        Restore::new(objects, header)
    }
}

impl Before {
    /// What the edit since changed, as the [`Restore`] that puts `now` back as it was.
    pub fn undo(self, now: &Pdf) -> Restore {
        let mut doc = self.0;
        let mut old = take(&mut doc.objects);
        let mut objects = Vec::new();
        for (&id, is) in &now.doc.objects {
            match old.remove(&id) {
                Some(was) if same(&was, is) => {}
                // Changed, or added (`None`).
                was => objects.push((id, was)),
            }
        }
        // Those left were removed.
        objects.extend(old.into_iter().map(|(id, was)| (id, Some(was))));
        let header = Header::take(&mut doc);
        let header = (!header.same(&now.doc)).then_some(header);
        Restore::new(objects.into_iter().collect(), header)
    }
}

impl Restore {
    /// `objects` built in one go (`collect`), which fills the map's nodes (see `map_size`).
    fn new(objects: BTreeMap<ObjectId, Option<Object>>, header: Option<Header>) -> Self {
        let held = |v: &Option<Object>| v.as_ref().map_or(0, heap);
        let size = map_size(objects.len())
            + objects.values().map(held).sum::<usize>()
            + header.as_ref().map_or(0, Header::size);
        Restore {
            objects,
            header,
            size,
        }
    }

    /// The memory it holds, estimated from the objects' sizes, in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Whether it changes nothing: the edit left the document as it was.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty() && self.header.is_none()
    }

    /// How many objects it puts back, adds or removes.
    pub fn objects(&self) -> usize {
        self.objects.len()
    }

    /// Join `later`, the [`Restore`] of an edit made after this one's, into this one, so that
    /// restoring it goes back to before this one's edit: an object both changed keeps its
    /// value from before the first.
    pub fn merge(&mut self, later: Restore) {
        let mut objects = take(&mut self.objects);
        for (id, was) in later.objects {
            objects.entry(id).or_insert(was);
        }
        let header = self.header.take().or(later.header);
        *self = Restore::new(objects.into_iter().collect(), header);
    }
}

impl std::fmt::Debug for Restore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Restore")
            .field("objects", &self.objects.len())
            .field("header", &self.header.is_some())
            .field("size", &self.size)
            .finish()
    }
}

/// Whether two objects are the same, as written: a dictionary's keys in the same order
/// (lopdf's own comparison ignores the order), reals to the bit (0.0 and -0.0 are written
/// differently), and a stream's flags as well as its content.
fn same(a: &Object, b: &Object) -> bool {
    match (a, b) {
        (Object::Real(x), Object::Real(y)) => x.to_bits() == y.to_bits(),
        (Object::Array(x), Object::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same(x, y))
        }
        (Object::Dictionary(x), Object::Dictionary(y)) => same_dict(x, y),
        (Object::Stream(x), Object::Stream(y)) => {
            x.content == y.content
                && x.allows_compression == y.allows_compression
                && x.start_position == y.start_position
                && same_dict(&x.dict, &y.dict)
        }
        _ => a == b,
    }
}

fn same_dict(x: &Dictionary, y: &Dictionary) -> bool {
    x.len() == y.len()
        && (x.iter().zip(y.iter())).all(|((xk, xv), (yk, yv))| xk == yk && same(xv, yv))
}

/// The nodes of a map of `n` entries built in one go: eleven entries a node, the first a
/// whole node however few it holds, and a pointer to each node from the one above.
fn map_size(n: usize) -> usize {
    const B: usize = 11;
    if n == 0 {
        return 0;
    }
    let node = B * size_of::<(ObjectId, Option<Object>)>() + 16;
    n.div_ceil(B) * (node + size_of::<usize>())
}

/// The memory an object holds besides its own size.
fn heap(o: &Object) -> usize {
    match o {
        Object::Name(n) => n.capacity(),
        Object::String(s, _) => s.capacity(),
        Object::Array(a) => a.capacity() * size_of::<Object>() + a.iter().map(heap).sum::<usize>(),
        Object::Dictionary(d) => dict_heap(d),
        Object::Stream(s) => s.content.capacity() + dict_heap(&s.dict),
        _ => 0,
    }
}

/// A dictionary's memory: its table of indices (a word and a control byte for each of a power
/// of two of buckets, at least 8/7 of the entries, and a group of control bytes more) and its
/// entries (a hash, the key and the value each), room for as many as the table holds being
/// taken by a copy, with what the keys and values hold.
fn dict_heap(d: &Dictionary) -> usize {
    let n = d.len();
    if n == 0 {
        return 0;
    }
    let buckets = match n {
        0..4 => 4,
        4..8 => 8,
        _ => (n * 8 / 7).next_power_of_two(),
    };
    let table = buckets * (size_of::<usize>() + 1) + 16;
    let room = if buckets < 8 {
        buckets - 1
    } else {
        buckets / 8 * 7
    };
    let entries = n.max(room) * (size_of::<usize>() + size_of::<Vec<u8>>() + size_of::<Object>());
    let held: usize = d.iter().map(|(k, v)| k.capacity() + heap(v)).sum();
    entries + table + held
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    fn sample() -> Pdf {
        let mut p = Pdf::new();
        p.insert_blank(0, 200.0, 300.0).unwrap();
        p
    }

    /// The document as written, with the highest object number the highest in use: Undo
    /// doesn't lower it, and puts back everything else.
    fn bytes(p: &Pdf) -> Vec<u8> {
        let mut copy = p.clone();
        copy.doc.max_id = copy.doc.objects.keys().map(|id| id.0).max().unwrap_or(0);
        copy.snapshot().unwrap()
    }

    /// An object number isn't given to another object after Undo, Redo or a failed edit:
    /// the next object added takes a new one.
    #[test]
    fn object_numbers_are_not_used_again() {
        let mut p = sample();
        let before = p.before();
        let added = p.doc.add_object(dictionary! { "N" => 1 });
        p.restore(before.undo(&p));
        assert!(!p.doc.objects.contains_key(&added));
        assert!(p.doc.add_object(dictionary! { "N" => 2 }).0 > added.0);
        let before = p.before();
        let failed = p.doc.add_object(dictionary! { "N" => 3 });
        p.go_back(before);
        assert!(p.doc.add_object(dictionary! { "N" => 4 }).0 > failed.0);
    }

    /// Objects changed, added and removed are put back, and the reverse puts them again;
    /// what the edit left alone isn't kept.
    #[test]
    fn a_restore_holds_what_changed_and_reverses() {
        let mut p = sample();
        let gone = p.doc.add_object(dictionary! { "Gone" => 1 });
        let (was, objects) = (bytes(&p), p.doc.objects.len());
        let before = p.before();
        p.doc.objects.remove(&gone);
        let added = p.doc.add_object(dictionary! { "Added" => 2 });
        let page = p.page_ids()[0];
        p.doc.get_dictionary_mut(page).unwrap().set("Rotate", 90);
        let now = bytes(&p);
        let undo = before.undo(&p);
        assert_eq!(undo.objects(), 3, "{undo:?}");
        assert!(undo.size() < 8192, "{undo:?}");
        let redo = p.restore(undo);
        assert!(bytes(&p) == was);
        assert_eq!(p.doc.objects.len(), objects);
        assert!(!p.doc.objects.contains_key(&added));
        let undo = p.restore(redo);
        assert!(bytes(&p) == now);
        assert_eq!(undo.objects(), 3);
        // Nothing changed: nothing to put back.
        let before = p.before();
        assert!(before.undo(&p).is_empty());
    }

    /// A change of key order alone, or of a real's sign at zero, is a change: each is written
    /// differently.
    #[test]
    fn key_order_and_negative_zero_count_as_changes() {
        let mut p = sample();
        let id = p.doc.add_object(dictionary! { "A" => 1, "B" => 0.0 });
        let was = bytes(&p);
        let before = p.before();
        let d = p.doc.get_dictionary_mut(id).unwrap();
        let a = d.remove(b"A").unwrap();
        d.set("A", a);
        let undo = before.undo(&p);
        assert_eq!(undo.objects(), 1);
        p.restore(undo);
        assert!(bytes(&p) == was);
        let before = p.before();
        p.doc.get_dictionary_mut(id).unwrap().set("B", -0.0);
        assert_eq!(before.undo(&p).objects(), 1);
    }

    /// Two edits joined go back to before the first, and forward to after the second.
    #[test]
    fn merged_restores_go_back_to_before_the_first() {
        let mut p = sample();
        let was = bytes(&p);
        let page = p.page_ids()[0];
        let before = p.before();
        p.doc.get_dictionary_mut(page).unwrap().set("Rotate", 90);
        let added = p.doc.add_object(dictionary! { "N" => 1 });
        let mut first = before.undo(&p);
        let before = p.before();
        p.doc.get_dictionary_mut(page).unwrap().set("Rotate", 180);
        p.doc.get_dictionary_mut(added).unwrap().set("N", 2);
        p.doc.add_object(dictionary! { "N" => 3 });
        let now = bytes(&p);
        let size = first.size();
        first.merge(before.undo(&p));
        assert_eq!(first.objects(), 3);
        assert!(first.size() >= size);
        let redo = p.restore(first);
        assert!(bytes(&p) == was);
        p.restore(redo);
        assert!(bytes(&p) == now);
    }

    /// A failed edit is put back whole.
    #[test]
    fn going_back_puts_the_document_as_it_was() {
        let mut p = sample();
        let was = bytes(&p);
        let before = p.before();
        p.insert_blank(1, 100.0, 100.0).unwrap();
        p.doc.trailer.set("Info", Object::Null);
        p.go_back(before);
        assert!(bytes(&p) == was);
    }
}
