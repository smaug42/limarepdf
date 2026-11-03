//! What another file's pages bring when they are added to a document (ENH-18): its
//! bookmarks, put among the document's in page order, and its named destinations, renamed
//! where the document has the name already. [`Pdf::insert_pages`] starts with
//! [`Pdf::carrying`], takes each file's with [`Pdf::carry`] before its objects join the
//! document, and puts them in place with [`Pdf::put_carried`].

use crate::outline::MAX_ITEMS;
use crate::{Pdf, Result};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, StringFormat};
use std::collections::{HashMap, HashSet};

/// How deep a value's direct values are looked through for destinations.
const MAX_NESTING: usize = 32;

/// The bookmarks and named destinations of the files being added, gathered for the
/// document they are added to.
pub(crate) struct Carried {
    /// Where the bookmarks go among the document's top-level ones.
    place: usize,
    /// How many more bookmarks the outline can be read with.
    room: usize,
    /// The names the document's named destinations and those already carried have.
    taken: HashSet<Vec<u8>>,
    /// The files' top-level bookmarks, in order.
    bookmarks: Vec<ObjectId>,
    names: Vec<(Vec<u8>, Object)>,
}

impl Pdf {
    /// Ready to carry bookmarks and named destinations into this document with pages going
    /// in at page `at` (0-based): the added bookmarks go at the top level, after the last
    /// one going to a page before `at`, first when none does, and after all of them when the
    /// pages go at the end.
    pub(crate) fn carrying(&self, at: usize) -> Carried {
        let nodes = self.outline_nodes();
        let tops: Vec<_> = nodes.iter().filter(|(_, i)| i.depth == 0).collect();
        let place = if at >= self.page_count() {
            tops.len()
        } else {
            (tops.iter())
                .rposition(|(_, i)| i.page.is_some_and(|p| p < at))
                .map_or(0, |k| k + 1)
        };
        Carried {
            place,
            room: MAX_ITEMS.saturating_sub(nodes.len()),
            taken: self.named_dests().into_iter().map(|(k, _)| k).collect(),
            bookmarks: Vec::new(),
            names: Vec::new(),
        }
    }

    /// Take this file's bookmarks and named destinations into `into`, its objects already
    /// numbered as they will be in the document. Its bookmarks are linked again as they are
    /// read, so loops and nesting past the outline's limits are left out, as many as the
    /// outline has room for. Each named destination gets a name the document doesn't have
    /// yet: its own, or that with `-2`, `-3` and so on after it. The destinations its links,
    /// bookmarks and actions give by name go to the name it now has; those that give a page
    /// by its number (from 0, which some makers write) go to that page itself, or nowhere
    /// when the file has no such page.
    pub(crate) fn carry(&mut self, into: &mut Carried) -> Result<()> {
        let mut renamed = HashMap::new();
        let mut names = Vec::new();
        let mut seen = HashSet::new();
        // The catalog's Dests come first, as they are looked up first.
        for (name, value) in self.named_dests() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let new = free_name(&name, &into.taken);
            into.taken.insert(new.clone());
            if new != name {
                renamed.insert(name, new.clone());
            }
            names.push((new, value));
        }
        let pages = self.page_ids();
        let fix = Fix {
            doc: &self.doc,
            pages: &pages,
            renamed: &renamed,
        };
        let changed: Vec<(ObjectId, Object)> = (self.doc.objects.iter())
            .filter_map(|(&id, o)| Some((id, fix.object(o, 0)?)))
            .collect();
        for (_, value) in &mut names {
            if let Some(fixed) = fix.dest(value, 0) {
                *value = fixed;
            }
        }
        self.doc.objects.extend(changed);
        into.names.extend(names);

        let mut nodes = self.outline_nodes();
        nodes.truncate(into.room);
        into.room -= nodes.len();
        let mut kids = vec![Vec::new(); nodes.len()];
        // The bookmarks around the one being read, from the top level down.
        let mut path: Vec<usize> = Vec::new();
        for (k, (id, item)) in nodes.iter().enumerate() {
            path.truncate(item.depth);
            match path.last() {
                Some(&parent) => kids[parent].push(*id),
                None => into.bookmarks.push(*id),
            }
            path.push(k);
        }
        for ((id, _), kids) in nodes.iter().zip(&kids) {
            self.relink(*id, kids)?;
        }
        Ok(())
    }

    /// Put what [`Pdf::carry`] took into this document, once the files' objects are in it.
    /// The named destinations go into one name tree with the document's own, in order.
    pub(crate) fn put_carried(&mut self, carried: Carried) -> Result<()> {
        if !carried.bookmarks.is_empty() {
            let mut tops: Vec<ObjectId> = (self.outline_nodes().into_iter())
                .filter(|(_, i)| i.depth == 0)
                .map(|(id, _)| id)
                .collect();
            let place = carried.place.min(tops.len());
            tops.splice(place..place, carried.bookmarks);
            let root = self.outline_root()?;
            self.relink(root, &tops)?;
            self.fix_outline_counts()?;
        }
        if !carried.names.is_empty() {
            let mut all = self.tree_dests();
            all.extend(carried.names);
            // A name a file has twice keeps the value found first, as looking it up would.
            let mut seen = HashSet::new();
            all.retain(|(k, _)| seen.insert(k.clone()));
            all.sort_by(|a, b| a.0.cmp(&b.0));
            let pairs = (all.into_iter())
                .flat_map(|(k, v)| [Object::String(k, StringFormat::Literal), v])
                .collect::<Vec<_>>();
            let tree = self.doc.add_object(dictionary! { "Names" => pairs });
            // The catalog's Names may be an object of its own, holding other trees too.
            let names = self.doc.catalog()?.get(b"Names").ok();
            let names = names.and_then(|o| o.as_reference().ok());
            match names.filter(|&id| self.doc.get_dictionary(id).is_ok()) {
                Some(id) => self.doc.get_dictionary_mut(id)?.set("Dests", tree),
                None => {
                    let catalog = self.doc.catalog_mut()?;
                    let direct = catalog.get(b"Names").is_ok_and(|o| o.as_dict().is_ok());
                    if direct {
                        catalog.get_mut(b"Names")?.as_dict_mut()?.set("Dests", tree);
                    } else {
                        catalog.set("Names", dictionary! { "Dests" => tree });
                    }
                }
            }
        }
        Ok(())
    }
}

/// `name`, or the first of `name-2`, `name-3` and so on that `taken` hasn't, in UTF-16 for
/// a name written in it.
fn free_name(name: &[u8], taken: &HashSet<Vec<u8>>) -> Vec<u8> {
    let utf16 = name.starts_with(&[0xFE, 0xFF]);
    let mut new = name.to_vec();
    let mut n = 1;
    while taken.contains(&new) {
        n += 1;
        new = name.to_vec();
        let suffix = format!("-{n}");
        if utf16 {
            new.extend(suffix.encode_utf16().flat_map(u16::to_be_bytes));
        } else {
            new.extend(suffix.bytes());
        }
    }
    new
}

/// Changes an added file's destinations to what they are in the document it joins.
struct Fix<'a> {
    doc: &'a Document,
    /// The file's pages, in order.
    pages: &'a [ObjectId],
    renamed: &'a HashMap<Vec<u8>, Vec<u8>>,
}

impl Fix<'_> {
    /// `o` with its destinations changed, or `None` when none needs it: each dictionary's
    /// `Dest`, and the `D` of a `GoTo` action, in it and in its direct values. Referenced
    /// objects are changed as objects of their own.
    fn object(&self, o: &Object, depth: usize) -> Option<Object> {
        if depth > MAX_NESTING {
            return None;
        }
        match o {
            Object::Dictionary(d) => self.dict(d, depth).map(Object::Dictionary),
            Object::Stream(s) => {
                let dict = self.dict(&s.dict, depth)?;
                let mut s = s.clone();
                s.dict = dict;
                Some(Object::Stream(s))
            }
            Object::Array(a) => {
                let mut out: Option<Vec<Object>> = None;
                for (n, v) in a.iter().enumerate() {
                    if let Some(v) = self.object(v, depth + 1) {
                        out.get_or_insert_with(|| a.clone())[n] = v;
                    }
                }
                out.map(Object::Array)
            }
            _ => None,
        }
    }

    fn dict(&self, d: &Dictionary, depth: usize) -> Option<Dictionary> {
        let goto = d.get(b"S").and_then(Object::as_name).ok() == Some(b"GoTo".as_slice());
        let mut out: Option<Dictionary> = None;
        for (k, v) in d.iter() {
            let fixed = match k.as_slice() {
                b"Dest" => self.dest(v, 0),
                b"D" if goto => self.dest(v, 0),
                _ => self.object(v, depth + 1),
            };
            if let Some(fixed) = fixed {
                out.get_or_insert_with(|| d.clone()).set(k.clone(), fixed);
            }
        }
        out
    }

    /// The destination `dest` as it is to be in the document, or `None` when it stays as it
    /// is. A name becomes a string, as the document's name tree holds it.
    fn dest(&self, dest: &Object, hops: usize) -> Option<Object> {
        let named = |n: &Vec<u8>| {
            let n = self.renamed.get(n).unwrap_or(n);
            Object::String(n.clone(), StringFormat::Literal)
        };
        match dest {
            Object::Name(n) => Some(named(n)),
            Object::String(n, _) => self.renamed.contains_key(n).then(|| named(n)),
            Object::Array(a) => {
                let Object::Integer(n) = a.first()? else {
                    return None;
                };
                let page = usize::try_from(*n).ok().and_then(|n| self.pages.get(n));
                let mut a = a.clone();
                a[0] = page.map_or(Object::Null, |&id| id.into());
                Some(Object::Array(a))
            }
            Object::Dictionary(d) if hops < 4 => {
                let fixed = self.dest(d.get(b"D").ok()?, hops + 1)?;
                let mut d = d.clone();
                d.set("D", fixed);
                Some(Object::Dictionary(d))
            }
            Object::Reference(id) if hops < 4 => {
                self.dest(self.doc.get_object(*id).ok()?, hops + 1)
            }
            _ => None,
        }
    }
}
