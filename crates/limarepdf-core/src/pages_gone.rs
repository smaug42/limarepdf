//! What pages leave behind when they go (extract, delete, split). The page tree is only one
//! way to reach a page: bookmarks, named destinations, links, the opening action, form
//! fields and the structure tree refer to pages and their annotations too, and a save keeps
//! everything still referred to. So the references to what went are cut, and the saved
//! file holds nothing of those pages.

use crate::forms::MAX_DEPTH;
use crate::{Pdf, Result};
use anyhow::bail;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::{HashMap, HashSet};

/// What went: the pages, the annotations only they listed, and the form fields left with no
/// widget; and the named destinations that went to those pages.
struct Gone {
    pages: HashSet<ObjectId>,
    objects: HashSet<ObjectId>,
    names: HashSet<Vec<u8>>,
}

/// The arrays of name and number tree pairs (`/Names`, `/Nums`) kept as objects of their
/// own, each with the key that holds it.
fn pair_arrays(doc: &lopdf::Document) -> Vec<(ObjectId, &'static [u8])> {
    fn walk(o: &Object, out: &mut Vec<(ObjectId, &'static [u8])>) {
        let d = match o {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &s.dict,
            Object::Array(a) => return a.iter().for_each(|x| walk(x, out)),
            _ => return,
        };
        for (k, v) in d.iter() {
            let key: Option<&'static [u8]> = match k.as_slice() {
                b"Names" => Some(b"Names"),
                b"Nums" => Some(b"Nums"),
                _ => None,
            };
            match (key, v) {
                (Some(key), Object::Reference(id)) => out.push((*id, key)),
                _ => walk(v, out),
            }
        }
    }
    let mut out = Vec::new();
    for o in doc.objects.values() {
        walk(o, &mut out);
    }
    out.retain(|(id, _)| doc.get_object(*id).is_ok_and(|o| o.as_array().is_ok()));
    out
}

impl Gone {
    /// Whether `o` is a destination to a page that went: `[page /Fit ...]`, or a dictionary
    /// with such a `/D`.
    fn dead_dest(&self, o: &Object) -> bool {
        match o {
            Object::Array(a) => a
                .first()
                .and_then(|p| p.as_reference().ok())
                .is_some_and(|p| self.pages.contains(&p)),
            Object::Dictionary(d) => d.get(b"D").is_ok_and(|d| self.dead_dest(d)),
            _ => false,
        }
    }

    /// Whether a value in `key` must go: a reference to what went, a destination (or an
    /// action going) to a page that went, or the name of a named destination that went.
    fn dead(&self, key: &[u8], v: &Object) -> bool {
        match v {
            Object::Reference(id) => self.objects.contains(id),
            Object::Array(_) | Object::Dictionary(_) => self.dead_dest(v),
            Object::Name(n) | Object::String(n, _) => {
                matches!(key, b"Dest" | b"D") && self.names.contains(n)
            }
            _ => false,
        }
    }

    /// Cut what went out of `o`: dictionary entries and array elements that refer to it,
    /// and name and number tree pairs whose value does. Direct objects can't loop, and
    /// lopdf's parser bounds how deeply they nest.
    fn cut(&self, o: &mut Object) {
        match o {
            Object::Dictionary(d) => self.cut_dict(d),
            Object::Stream(s) => self.cut_dict(&mut s.dict),
            Object::Array(a) => {
                a.retain(|x| !self.dead(b"", x));
                a.iter_mut().for_each(|x| self.cut(x));
            }
            _ => {}
        }
    }

    fn cut_dict(&self, d: &mut Dictionary) {
        let dead: Vec<Vec<u8>> = d
            .iter()
            .filter(|(k, v)| self.dead(k, v))
            .map(|(k, _)| k.clone())
            .collect();
        for k in dead {
            d.remove(&k);
        }
        for key in [&b"Names"[..], b"Nums"] {
            if let Ok(Object::Array(a)) = d.get_mut(key) {
                self.cut_pairs(a, key);
            }
        }
        for (_, v) in d.iter_mut() {
            self.cut(v);
        }
    }

    /// Cut the pairs of a name or number tree (`key` is `/Names` or `/Nums`) whose value
    /// refers to what went, or whose name is a named destination that went; then what went
    /// out of the values that stay.
    fn cut_pairs(&self, a: &mut Vec<Object>, key: &[u8]) {
        let pairs = std::mem::take(a);
        let mut it = pairs.into_iter();
        while let (Some(k), Some(mut v)) = (it.next(), it.next()) {
            let name_gone = k.as_str().is_ok_and(|n| self.names.contains(n));
            if !(self.dead(b"", &v) || (key == b"Names" && name_gone)) {
                self.cut(&mut v);
                a.push(k);
                a.push(v);
            }
        }
    }
}

impl Pdf {
    /// Make `keep` the document's pages, in that order, and cut what the other pages leave
    /// behind. With `bookmarks`, bookmarks to the other pages go too (keeping one without a
    /// destination when it has kept children), as for a new document of some pages; without,
    /// they stay and lose their destination, as when deleting pages. Refuses, with nothing
    /// changed, when it can't tell everything that goes: a form nested too deep.
    pub(crate) fn keep_pages(&mut self, keep: &[ObjectId], bookmarks: bool) -> Result<()> {
        let kept: HashSet<ObjectId> = keep.iter().copied().collect();
        let all = self.page_ids();
        let pages: HashSet<ObjectId> = all.iter().copied().filter(|p| !kept.contains(p)).collect();
        let doomed = if bookmarks {
            self.bookmarks_to(&all, &pages)
        } else {
            Vec::new()
        };
        let gone = self.what_goes(keep, pages)?;
        self.flatten()?;
        self.set_page_order(keep)?;
        self.strip_gone_alt(&gone);
        let pairs: std::collections::HashMap<ObjectId, &[u8]> =
            pair_arrays(&self.doc).into_iter().collect();
        let ids: Vec<ObjectId> = self.doc.objects.keys().copied().collect();
        for id in ids {
            if gone.objects.contains(&id) {
                continue;
            }
            match (self.doc.objects.get_mut(&id), pairs.get(&id)) {
                (Some(Object::Array(a)), Some(key)) => gone.cut_pairs(a, key),
                (Some(o), _) => gone.cut(o),
                _ => {}
            }
        }
        if !doomed.is_empty() {
            let nodes: Vec<ObjectId> = self.outline_nodes().into_iter().map(|(id, _)| id).collect();
            let mut at: Vec<usize> = doomed
                .iter()
                .filter_map(|d| nodes.iter().position(|n| n == d))
                .collect();
            at.sort_unstable();
            for i in at.into_iter().rev() {
                self.remove_bookmark(i)?;
            }
        }
        Ok(())
    }

    /// The bookmarks that go to `pages` (of the page order `all`) and have no child that
    /// stays, nor a descendant that does.
    fn bookmarks_to(&self, all: &[ObjectId], pages: &HashSet<ObjectId>) -> Vec<ObjectId> {
        let nodes = self.outline_nodes();
        let to_gone: Vec<bool> = nodes
            .iter()
            .map(|(_, item)| {
                item.page
                    .is_some_and(|p| all.get(p).is_some_and(|id| pages.contains(id)))
            })
            .collect();
        // A bookmark stays when it doesn't go to a page that went, or a descendant stays.
        let mut kept_below = vec![false; nodes.len()];
        let mut open: Vec<usize> = Vec::new();
        for (j, (_, item)) in nodes.iter().enumerate() {
            while open.last().is_some_and(|&a| nodes[a].1.depth >= item.depth) {
                open.pop();
            }
            if !to_gone[j] {
                for &a in &open {
                    kept_below[a] = true;
                }
            }
            open.push(j);
        }
        nodes
            .iter()
            .enumerate()
            .filter(|&(i, _)| to_gone[i] && !kept_below[i])
            .map(|(_, (id, _))| *id)
            .collect()
    }

    /// What goes with `pages`: them, the annotations only they list, and the form fields all
    /// of whose widgets are among those (and parents all of whose fields went); and the
    /// named destinations to them.
    fn what_goes(&self, keep: &[ObjectId], pages: HashSet<ObjectId>) -> Result<Gone> {
        let staying: HashSet<ObjectId> =
            keep.iter().flat_map(|&p| self.all_annot_refs(p)).collect();
        let mut objects: HashSet<ObjectId> = pages.clone();
        for &p in &pages {
            objects.extend(
                self.all_annot_refs(p)
                    .into_iter()
                    .filter(|a| !staying.contains(a)),
            );
        }
        let fields = self
            .acro_form()
            .and_then(|f| f.get(b"Fields").ok())
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .cloned()
            .unwrap_or_default();
        let mut seen = HashSet::new();
        for f in fields {
            if let Ok(f) = f.as_reference() {
                self.field_goes(f, &mut objects, &mut seen, 0)?;
            }
        }
        let mut gone = Gone {
            pages,
            objects,
            names: HashSet::new(),
        };
        gone.names = self.dead_names(&gone);
        Ok(gone)
    }

    /// Whether form field node `id` goes: a widget that went, or a node all of whose kids
    /// go. Those that go are added to `gone`. Fails when the fields are nested more than
    /// [`MAX_DEPTH`] deep, rather than keep a field (and its value) that might go.
    fn field_goes(
        &self,
        id: ObjectId,
        gone: &mut HashSet<ObjectId>,
        seen: &mut HashSet<ObjectId>,
        depth: usize,
    ) -> Result<bool> {
        if gone.contains(&id) {
            return Ok(true);
        }
        if !seen.insert(id) {
            return Ok(false);
        }
        if depth > MAX_DEPTH {
            bail!("the form's fields are nested more than {MAX_DEPTH} deep");
        }
        let kids: Vec<ObjectId> = self
            .doc
            .get_dictionary(id)
            .ok()
            .and_then(|d| d.get(b"Kids").ok())
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .map(|a| a.iter().filter_map(|k| k.as_reference().ok()).collect())
            .unwrap_or_default();
        if kids.is_empty() {
            return Ok(false);
        }
        let mut all = true;
        for k in kids {
            // Every kid is looked at, so nested fields that go are found too.
            all &= self.field_goes(k, gone, seen, depth + 1)?;
        }
        if all {
            gone.insert(id);
        }
        Ok(all)
    }

    /// The named destinations (in the catalog's `/Dests` and the `/Names /Dests` tree) that
    /// go to pages that went. The whole tree is read, each node once, however deep it is.
    fn dead_names(&self, gone: &Gone) -> HashSet<Vec<u8>> {
        let mut out = HashSet::new();
        let resolve = |o: &Object| -> Object {
            self.doc
                .dereference(o)
                .map_or_else(|_| o.clone(), |(_, o)| o.clone())
        };
        let catalog = self.doc.catalog().ok();
        let old = catalog
            .and_then(|c| c.get(b"Dests").ok())
            .map(|o| self.resolved_dict(Some(o)))
            .unwrap_or_default();
        for (k, v) in old.iter() {
            if gone.dead_dest(&resolve(v)) {
                out.insert(k.clone());
            }
        }
        let tree = catalog
            .and_then(|c| c.get(b"Names").ok())
            .map(|o| self.resolved_dict(Some(o)))
            .and_then(|n| n.get(b"Dests").ok().cloned());
        // Nodes are taken from a list, not by recursion, so depth costs no stack; nodes that
        // are objects are read once, so loops end, and direct ones can't loop.
        let mut todo: Vec<Object> = tree.into_iter().collect();
        let mut seen = HashSet::new();
        while let Some(node) = todo.pop() {
            if node.as_reference().is_ok_and(|id| !seen.insert(id)) {
                continue;
            }
            let node = self.resolved_dict(Some(&node));
            if let Ok(names) = node.get(b"Names").map(resolve) {
                for pair in names
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .chunks(2)
                {
                    if let [k, v] = pair {
                        if let (Ok(k), true) = (k.as_str(), gone.dead_dest(&resolve(v))) {
                            out.insert(k.to_vec());
                        }
                    }
                }
            }
            if let Ok(Object::Array(kids)) = node.get(b"Kids").map(resolve) {
                todo.extend(kids);
            }
        }
        out
    }

    /// Take the alternative text off the structure elements with content on a page that went,
    /// and off every element above one, as their text describes it too; also when an element
    /// has content on kept pages (decision D3). An element is on a page that went by its own
    /// `/Pg`, or by a content item of its `/K`: marked content on that page (the item's `/Pg`,
    /// else the element's, else the nearest parent's), or an object reference to something
    /// that went. Items are followed whether they are held directly or as objects of their
    /// own, in an array or not (audit R06). Each element is looked at once, so loops in the
    /// tree end, and as nothing is followed by recursion there is no depth to stop at.
    fn strip_gone_alt(&mut self, gone: &Gone) {
        let doc = &self.doc;
        let elems: HashSet<ObjectId> = doc
            .objects
            .iter()
            .filter(|(_, o)| {
                o.as_dict()
                    .is_ok_and(|d| match d.get(b"Type").and_then(|t| t.as_name()) {
                        Ok(t) => t == b"StructElem",
                        // Border styles and actions have an `/S` too, but no parent or kids.
                        Err(_) => {
                            d.get(b"S").is_ok_and(|s| s.as_name().is_ok())
                                && (d.has(b"P") || d.has(b"K"))
                        }
                    })
            })
            .map(|(id, _)| *id)
            .collect();
        let reference = |d: &Dictionary, key: &[u8]| d.get(key).and_then(Object::as_reference).ok();
        // The page each element's marked content is on: its own, else the nearest parent's.
        let mut page_of: HashMap<ObjectId, Option<ObjectId>> = HashMap::new();
        for &e in &elems {
            let mut path = Vec::new();
            let mut on_path = HashSet::new();
            let mut cur = Some(e);
            let page = loop {
                let Some(id) = cur.filter(|id| elems.contains(id) && on_path.insert(*id)) else {
                    break None;
                };
                if let Some(&p) = page_of.get(&id) {
                    break p;
                }
                path.push(id);
                let d = doc.get_dictionary(id).ok();
                if let Some(p) = d.and_then(|d| reference(d, b"Pg")) {
                    break Some(p);
                }
                cur = d.and_then(|d| reference(d, b"P"));
            };
            for id in path {
                page_of.insert(id, page);
            }
        }
        let gone_page = |p: Option<ObjectId>| p.is_some_and(|p| gone.pages.contains(&p));
        // Elements with content on a page that went, and each element's parents: its `/P`, and
        // the elements listing it in their `/K`.
        let mut todo = Vec::new();
        let mut parents: HashMap<ObjectId, Vec<ObjectId>> = HashMap::new();
        for &e in &elems {
            let Ok(d) = doc.get_dictionary(e) else {
                continue;
            };
            let own = page_of.get(&e).copied().flatten();
            if let Some(p) = reference(d, b"P").filter(|p| elems.contains(p)) {
                parents.entry(e).or_default().push(p);
            }
            let items = match d.get(b"K").map(|k| doc.dereference(k)) {
                Ok(Ok((_, Object::Array(a)))) => a.iter().collect(),
                Ok(Ok((_, k))) => vec![k],
                _ => Vec::new(),
            };
            let mut on_gone = gone_page(reference(d, b"Pg"));
            for item in items {
                let Ok((id, item)) = doc.dereference(item) else {
                    continue;
                };
                match item {
                    Object::Integer(_) => on_gone |= gone_page(own),
                    Object::Dictionary(k) => match id.filter(|id| elems.contains(id)) {
                        Some(kid) => parents.entry(kid).or_default().push(e),
                        // A marked-content or object reference.
                        None => {
                            let obj = reference(k, b"Obj");
                            on_gone |= obj.is_some_and(|o| gone.objects.contains(&o))
                                || gone_page(reference(k, b"Pg").or(own));
                        }
                    },
                    _ => {}
                }
            }
            if on_gone {
                todo.push(e);
            }
        }
        let mut strip = HashSet::new();
        while let Some(e) = todo.pop() {
            if strip.insert(e) {
                todo.extend(parents.get(&e).into_iter().flatten());
            }
        }
        for id in strip {
            if let Ok(d) = self.doc.get_dictionary_mut(id) {
                for k in [&b"ActualText"[..], b"Alt", b"E"] {
                    d.remove(k);
                }
            }
        }
    }
}
