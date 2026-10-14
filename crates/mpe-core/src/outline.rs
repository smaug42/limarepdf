//! The document outline (bookmarks) and the destinations it points to.

use crate::doc::decode_text;
use crate::Pdf;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::{HashMap, HashSet};

/// Limits that keep a malformed or hostile file from running away.
const MAX_ITEMS: usize = 100_000;
const MAX_DEPTH: usize = 64;

/// One bookmark. [`Pdf::outline`] lists them in reading order, each followed by its
/// children, which have a `depth` one greater.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineItem {
    pub title: String,
    /// 0 for a top-level bookmark.
    pub depth: usize,
    /// The page it goes to (zero-based); `None` when it goes to another file, a web
    /// address, or nowhere this document has.
    pub page: Option<usize>,
    /// The file asks for its children to be shown when it opens.
    pub open: bool,
    pub has_children: bool,
}

impl Pdf {
    /// The bookmarks; empty when the document has none. Broken entries are skipped,
    /// never fatal: a bookmark whose target can't be found gets `page: None`.
    pub fn outline(&self) -> Vec<OutlineItem> {
        let mut out = Vec::new();
        let first = self
            .doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"Outlines").ok())
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .and_then(|d| d.get(b"First").ok())
            .and_then(|o| o.as_reference().ok());
        if let Some(first) = first {
            let pages = self.page_index();
            let mut seen = HashSet::new();
            self.walk_outline(first, 0, &pages, &mut seen, &mut out);
        }
        out
    }

    fn walk_outline(
        &self,
        first: ObjectId,
        depth: usize,
        pages: &HashMap<ObjectId, usize>,
        seen: &mut HashSet<ObjectId>,
        out: &mut Vec<OutlineItem>,
    ) {
        let mut next = Some(first);
        while let Some(id) = next {
            // A loop in the Next or First links ends the walk there.
            if out.len() >= MAX_ITEMS || !seen.insert(id) {
                return;
            }
            let Ok(node) = self.doc.get_dictionary(id) else {
                return;
            };
            let child = node.get(b"First").ok().and_then(|o| o.as_reference().ok());
            let title = node
                .get(b"Title")
                .ok()
                .and_then(|o| self.doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_str().ok())
                .map(decode_text)
                .unwrap_or_default();
            // Titles sometimes carry line breaks, tabs or runs of spaces; show them on
            // one line with single spaces.
            let title = title
                .split(|c: char| c.is_whitespace() || c.is_control())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let open = node
                .get(b"Count")
                .ok()
                .and_then(|o| o.as_i64().ok())
                .is_some_and(|n| n > 0);
            out.push(OutlineItem {
                title,
                depth,
                page: self.item_target(node, pages),
                open,
                has_children: child.is_some() && depth + 1 < MAX_DEPTH,
            });
            if let Some(child) = child.filter(|_| depth + 1 < MAX_DEPTH) {
                self.walk_outline(child, depth + 1, pages, seen, out);
            }
            next = node.get(b"Next").ok().and_then(|o| o.as_reference().ok());
        }
    }

    /// Where an outline item goes: its `Dest`, or the `D` of a `GoTo` action.
    fn item_target(&self, node: &Dictionary, pages: &HashMap<ObjectId, usize>) -> Option<usize> {
        if let Ok(dest) = node.get(b"Dest") {
            return self.dest_page(dest, pages, 0);
        }
        let (_, action) = self.doc.dereference(node.get(b"A").ok()?).ok()?;
        let action = action.as_dict().ok()?;
        if action.get(b"S").ok()?.as_name().ok()? != b"GoTo" {
            return None;
        }
        self.dest_page(action.get(b"D").ok()?, pages, 0)
    }

    /// The page a destination goes to. It may be an explicit `[page /Fit ...]` array, a
    /// name or string looked up in the named destinations, or a dictionary with `D`.
    pub(crate) fn dest_page(
        &self,
        dest: &Object,
        pages: &HashMap<ObjectId, usize>,
        hops: usize,
    ) -> Option<usize> {
        if hops > 4 {
            return None;
        }
        let (_, dest) = self.doc.dereference(dest).ok()?;
        match dest {
            Object::Array(a) => match a.first()? {
                Object::Reference(id) => pages.get(id).copied(),
                // Not allowed for this document's own pages, but some makers write a
                // zero-based page number.
                Object::Integer(n) => usize::try_from(*n).ok().filter(|&n| n < pages.len()),
                _ => None,
            },
            Object::Dictionary(d) => self.dest_page(d.get(b"D").ok()?, pages, hops + 1),
            Object::Name(n) | Object::String(n, _) => {
                let named = self.named_dest(n)?;
                self.dest_page(&named, pages, hops + 1)
            }
            _ => None,
        }
    }

    /// A named destination: from the catalog's `Dests` dictionary (PDF 1.1) or the
    /// `Dests` name tree under `Names`.
    fn named_dest(&self, name: &[u8]) -> Option<Object> {
        let catalog = self.doc.catalog().ok()?;
        let old = catalog
            .get(b"Dests")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .and_then(|d| d.get(name).ok())
            .cloned();
        if old.is_some() {
            return old;
        }
        let (_, names) = self.doc.dereference(catalog.get(b"Names").ok()?).ok()?;
        let tree = names.as_dict().ok()?.get(b"Dests").ok()?;
        let (_, tree) = self.doc.dereference(tree).ok()?;
        self.name_tree_find(tree.as_dict().ok()?, name, 0)
    }

    fn name_tree_find(&self, node: &Dictionary, key: &[u8], depth: usize) -> Option<Object> {
        if depth > 32 {
            return None;
        }
        let array = |k: &[u8]| -> Option<&Vec<Object>> {
            let (_, o) = self.doc.dereference(node.get(k).ok()?).ok()?;
            o.as_array().ok()
        };
        if let Some(names) = array(b"Names") {
            for pair in names.chunks(2) {
                if let [k, v] = pair {
                    let k = self.doc.dereference(k).ok();
                    if k.and_then(|(_, o)| o.as_str().ok()) == Some(key) {
                        return Some(v.clone());
                    }
                }
            }
        }
        for kid in array(b"Kids")? {
            let Some(kid) = self
                .doc
                .dereference(kid)
                .ok()
                .and_then(|(_, o)| o.as_dict().ok())
            else {
                continue;
            };
            // Skip kids whose key range can't hold the name.
            let limits = kid
                .get(b"Limits")
                .ok()
                .and_then(|o| o.as_array().ok())
                .and_then(|l| Some((l.first()?.as_str().ok()?, l.get(1)?.as_str().ok()?)));
            if let Some((lo, hi)) = limits {
                if key < lo || key > hi {
                    continue;
                }
            }
            if let Some(v) = self.name_tree_find(kid, key, depth + 1) {
                return Some(v);
            }
        }
        None
    }

    /// Each page object's zero-based index.
    pub(crate) fn page_index(&self) -> HashMap<ObjectId, usize> {
        self.doc
            .get_pages()
            .into_iter()
            .map(|(n, id)| (id, n as usize - 1))
            .collect()
    }
}
