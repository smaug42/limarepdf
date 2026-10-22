//! Looking a key up in a name tree or a number tree, such as the named destinations or the
//! index from pages to their accessibility tags.

use lopdf::{Document, Object};
use std::cmp::Ordering;
use std::collections::HashSet;

/// How deep a node may be nested to be looked in, the root being 0.
pub(crate) const MAX_DEPTH: usize = 32;
/// How many kids one lookup takes up at most. Each node object is read once however often
/// it is listed, so real trees come nowhere near this; it bounds the work in hostile ones.
const MAX_VISITS: usize = 100_000;

/// What looking a key up came to.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Lookup {
    Found(Object),
    /// Every node whose key range could hold the key was read, and none has it.
    Missing,
    /// Gave up: a node that could hold the key is nested more than [`MAX_DEPTH`] deep.
    TooDeep,
    /// Gave up: the tree has more kids than one lookup takes up.
    TooLarge,
}

impl Lookup {
    /// The value, if found; a lookup that gave up has none either.
    pub(crate) fn found(self) -> Option<Object> {
        match self {
            Lookup::Found(v) => Some(v),
            _ => None,
        }
    }
}

/// The value of `key` in the name tree `root`.
pub(crate) fn name(doc: &Document, root: &Object, key: &[u8]) -> Lookup {
    let order = |o: &Object| Some(key.cmp(o.as_str().ok()?));
    find(doc, root, b"Names", order, MAX_VISITS).0
}

/// The value of `key` in the number tree `root`.
pub(crate) fn number(doc: &Document, root: &Object, key: i64) -> Lookup {
    let order = |o: &Object| Some(key.cmp(&o.as_i64().ok()?));
    find(doc, root, b"Nums", order, MAX_VISITS).0
}

/// Looks the key up in the pairs under `pairs` (`Names` or `Nums`), taking up at most `budget`
/// nodes, the root included, and says how many it took up. `order` compares the key with a key
/// in the tree, `None` when that can't be read. Kids whose `Limits` leave the key out are passed
/// over. Kids are taken up one at a time from a list per level, not by recursion, and each node
/// object is read once, so kids listed many times, and loops, cost one visit per listing
/// (audit R05).
fn find(
    doc: &Document,
    root: &Object,
    pairs: &[u8],
    order: impl Fn(&Object) -> Option<Ordering>,
    budget: usize,
) -> (Lookup, usize) {
    fn deref<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Object> {
        doc.dereference(o).ok().map(|(_, o)| o)
    }
    let resolve = |o| deref(doc, o);
    // The kids still to take up at each level, with their depth.
    let mut todo = vec![(std::slice::from_ref(root).iter(), 0)];
    let mut visits = 0;
    let mut seen = HashSet::new();
    let mut deep = false;
    while let Some((kids, depth)) = todo.last_mut() {
        let depth = *depth;
        let Some(node) = kids.next() else {
            todo.pop();
            continue;
        };
        visits += 1;
        if visits > budget {
            return (Lookup::TooLarge, budget);
        }
        let Ok((id, Object::Dictionary(d))) = doc.dereference(node) else {
            continue;
        };
        let limits = d
            .get(b"Limits")
            .ok()
            .and_then(resolve)
            .and_then(|l| l.as_array().ok())
            .and_then(|l| Some((order(resolve(l.first()?)?)?, order(resolve(l.get(1)?)?)?)));
        if depth > 0 && limits.is_some_and(|(lo, hi)| lo.is_lt() || hi.is_gt()) {
            continue;
        }
        if depth > MAX_DEPTH {
            deep = true;
            continue;
        }
        if id.is_some_and(|id| !seen.insert(id)) {
            continue;
        }
        if let Some(Object::Array(list)) = d.get(pairs).ok().and_then(resolve) {
            for [k, v] in list.as_chunks::<2>().0 {
                if resolve(k).and_then(&order) == Some(Ordering::Equal) {
                    return (Lookup::Found(v.clone()), visits);
                }
            }
        }
        if let Some(Object::Array(kids)) = d.get(b"Kids").ok().and_then(resolve) {
            todo.push((kids.iter(), depth + 1));
        }
    }
    let gave_up = if deep {
        Lookup::TooDeep
    } else {
        Lookup::Missing
    };
    (gave_up, visits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, StringFormat};

    fn text(s: &str) -> Object {
        Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
    }

    fn names(doc: &Document, root: &Object, key: &str, budget: usize) -> (Lookup, usize) {
        let order = |o: &Object| Some(key.as_bytes().cmp(o.as_str().ok()?));
        find(doc, root, b"Names", order, budget)
    }

    /// The leaf holding `key` (worth 7), `levels` kids down a chain of node objects, each
    /// listing the next twice.
    fn shared(doc: &mut Document, key: &str, levels: usize) -> Object {
        let mut node = doc.add_object(dictionary! { "Names" => vec![text(key), 7.into()] });
        for _ in 0..levels {
            let kids = vec![Object::Reference(node), Object::Reference(node)];
            node = doc.add_object(dictionary! { "Kids" => kids });
        }
        Object::Reference(node)
    }

    /// Kids listed twice at every level are taken up twice but read once, so a lookup through
    /// 32 levels takes up at most 65 nodes, not 2^32 (audit R05).
    #[test]
    fn shared_kids_are_read_once() {
        let mut doc = Document::with_version("1.7");
        let root = shared(&mut doc, "deep", MAX_DEPTH);
        assert_eq!(
            names(&doc, &root, "deep", MAX_VISITS),
            (Lookup::Found(7.into()), 33)
        );
        assert_eq!(
            names(&doc, &root, "absent", MAX_VISITS),
            (Lookup::Missing, 65)
        );
    }

    /// A key in a node nested one past the limit gives up as too deep, never as missing; a
    /// key found elsewhere is still found.
    #[test]
    fn past_the_depth_limit_gives_up() {
        let mut doc = Document::with_version("1.7");
        let deep = shared(&mut doc, "deep", MAX_DEPTH + 1);
        assert_eq!(name(&doc, &deep, "deep".as_bytes()), Lookup::TooDeep);
        assert_eq!(name(&doc, &deep, "absent".as_bytes()), Lookup::TooDeep);
        let near = doc.add_object(dictionary! { "Names" => vec![text("near"), 3.into()] });
        let root = Object::Dictionary(dictionary! { "Kids" => vec![deep, near.into()] });
        assert_eq!(
            name(&doc, &root, "near".as_bytes()),
            Lookup::Found(3.into())
        );
    }

    /// Past the budget the lookup gives up as too large, having taken up no more than the
    /// budget; a key met before then is found, and with the whole tree in budget a key not in
    /// it is missing.
    #[test]
    fn past_the_budget_gives_up() {
        let doc = Document::with_version("1.7");
        let leaf = |k: &str| Object::Dictionary(dictionary! { "Names" => vec![text(k), 1.into()] });
        let mut kids = vec![leaf("first")];
        kids.extend((0..20).map(|_| Object::Dictionary(dictionary! {})));
        kids.push(leaf("last"));
        let root = Object::Dictionary(dictionary! { "Kids" => kids });
        assert_eq!(names(&doc, &root, "last", 10), (Lookup::TooLarge, 10));
        assert_eq!(names(&doc, &root, "absent", 10), (Lookup::TooLarge, 10));
        assert_eq!(
            names(&doc, &root, "first", 10),
            (Lookup::Found(1.into()), 2)
        );
        assert_eq!(
            names(&doc, &root, "last", 23),
            (Lookup::Found(1.into()), 23)
        );
        assert_eq!(names(&doc, &root, "absent", 23), (Lookup::Missing, 23));
    }

    /// Kids whose `Limits` leave the key out are passed over, those with no readable limits
    /// are looked in, and a node that lists itself as a kid is read once.
    #[test]
    fn limits_and_loops() {
        let mut doc = Document::with_version("1.7");
        let ranged = |lo: i64, hi: i64, v: i64| {
            Object::Dictionary(dictionary! {
                "Limits" => vec![lo.into(), hi.into()], "Nums" => vec![5.into(), v.into()],
            })
        };
        // The key lies outside the first kid's range, though it is listed there.
        let root = Object::Dictionary(dictionary! {
            "Kids" => vec![ranged(10, 20, 1), ranged(0, 9, 2)],
        });
        assert_eq!(number(&doc, &root, 5), Lookup::Found(2.into()));
        let odd = Object::Dictionary(dictionary! {
            "Limits" => vec![text("x"), 9.into()], "Nums" => vec![5.into(), 3.into()],
        });
        let root = Object::Dictionary(dictionary! { "Kids" => vec![odd] });
        assert_eq!(number(&doc, &root, 5), Lookup::Found(3.into()));

        let id = doc.new_object_id();
        let node = dictionary! { "Kids" => vec![Object::Reference(id)], "Nums" => vec![1.into(), 1.into()] };
        doc.objects.insert(id, node.into());
        assert_eq!(number(&doc, &Object::Reference(id), 2), Lookup::Missing);
        assert_eq!(
            number(&doc, &Object::Reference(id), 1),
            Lookup::Found(1.into())
        );
    }
}
