//! The document outline (bookmarks) and the destinations it points to.

use crate::doc::{decode_shown_text, encode_text};
use crate::{trees, Pdf, Result};
use anyhow::{bail, Context};
use lopdf::{dictionary, Dictionary, Object, ObjectId, StringFormat};
use std::collections::{HashMap, HashSet};
use std::ops::Range;

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

/// Where [`Pdf::add_bookmark`] puts a new bookmark, or [`Pdf::move_bookmark`] a bookmark.
/// The numbers are positions in [`Pdf::outline`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookmarkPlace {
    /// Last at the top level.
    End,
    /// Just before this bookmark, at its level.
    Before(usize),
    /// Just after this bookmark (and its children), at its level.
    After(usize),
    /// Last among this bookmark's children.
    Under(usize),
}

/// A move from the keyboard, as Word's outline makes them ([`bookmark_step`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookmarkStep {
    /// Before the bookmark above it at its level.
    Up,
    /// After the bookmark below it at its level.
    Down,
    /// Out of the bookmark it is in, to just after it.
    Promote,
    /// Into the bookmark above it at its level, last.
    Demote,
}

/// Where `step` moves bookmark `index` of `items` (as [`Pdf::outline`] lists them), or
/// `None` when it can't go that way: Up and Down stay among the bookmarks at its level, so
/// the first and last there go no further; a top-level bookmark can't be promoted, nor the
/// first at its level demoted.
pub fn bookmark_step(
    items: &[OutlineItem],
    index: usize,
    step: BookmarkStep,
) -> Option<BookmarkPlace> {
    items.get(index)?;
    let parent = parent(items, index);
    let level = children(items, parent);
    let k = level.iter().position(|&j| j == index)?;
    Some(match step {
        BookmarkStep::Up => BookmarkPlace::Before(level[k.checked_sub(1)?]),
        BookmarkStep::Down => BookmarkPlace::After(*level.get(k + 1)?),
        BookmarkStep::Promote => BookmarkPlace::After(parent?),
        BookmarkStep::Demote => BookmarkPlace::Under(level[k.checked_sub(1)?]),
    })
}

/// Whether [`Pdf::move_bookmark`] would move bookmark `index` of `items` (as
/// [`Pdf::outline`] lists them) to `place`: false when it is there already, an error when
/// it can't go there.
pub fn bookmark_moves(items: &[OutlineItem], index: usize, place: BookmarkPlace) -> Result<bool> {
    Ok(plan_move(items, index, place)?.is_some())
}

impl Pdf {
    /// The parts of [`crate::SplitBy::Bookmarks`]`(level)`, in page order.
    pub(crate) fn bookmark_split(&self, level: usize) -> Result<Vec<(Option<String>, Vec<usize>)>> {
        let parts = bookmark_parts(&self.outline(), level, self.page_count());
        if parts.is_empty() {
            bail!("this document has no bookmarks that go to its pages");
        }
        Ok(parts
            .into_iter()
            .map(|(title, pages)| (title, pages.collect()))
            .collect())
    }

    /// The bookmarks; empty when the document has none. Broken entries are skipped,
    /// never fatal: a bookmark whose target can't be found gets `page: None`.
    pub fn outline(&self) -> Vec<OutlineItem> {
        self.outline_nodes().into_iter().map(|(_, i)| i).collect()
    }

    /// Add a bookmark to `page` (zero-based) and return its position in [`Pdf::outline`].
    /// It goes to the top of the page, keeping the zoom.
    pub fn add_bookmark(
        &mut self,
        title: &str,
        page: usize,
        place: BookmarkPlace,
    ) -> Result<usize> {
        let Some(&page_id) = self.page_ids().get(page) else {
            bail!(
                "page {} does not exist (document has {})",
                page + 1,
                self.page_count()
            );
        };
        let nodes = self.outline_nodes();
        let node = |i: usize| match nodes.get(i) {
            Some((id, item)) => Ok((*id, item.depth)),
            None => bail!("bookmark {} does not exist", i + 1),
        };
        let (parent, prev, next) = match place {
            BookmarkPlace::End => {
                let root = self.outline_root()?;
                (root, self.last_child(root), None)
            }
            BookmarkPlace::Before(i) => {
                let (id, _) = node(i)?;
                let parent = self.outline_link(id, b"Parent");
                let parent = parent.map_or_else(|| self.outline_root(), Ok)?;
                (parent, self.child_before(parent, id), Some(id))
            }
            BookmarkPlace::After(i) => {
                let (id, _) = node(i)?;
                let parent = self.outline_link(id, b"Parent");
                let parent = parent.map_or_else(|| self.outline_root(), Ok)?;
                (parent, Some(id), self.outline_link(id, b"Next"))
            }
            BookmarkPlace::Under(i) => {
                let (id, depth) = node(i)?;
                if depth + 1 >= MAX_DEPTH {
                    bail!("bookmarks can't be nested deeper than {MAX_DEPTH} levels");
                }
                // Open it, so the new bookmark shows.
                let d = self.doc.get_dictionary_mut(id)?;
                if d.get(b"Count")
                    .ok()
                    .and_then(|o| o.as_i64().ok())
                    .unwrap_or(0)
                    <= 0
                {
                    d.set("Count", 1);
                }
                (id, self.last_child(id), None)
            }
        };
        let top = self.page_box(page_id)[3];
        let mut item = dictionary! {
            "Title" => Object::String(encode_text(title), StringFormat::Literal),
            "Parent" => parent,
            "Dest" => vec![page_id.into(), "XYZ".into(), Object::Null, top.into(), Object::Null],
        };
        if let Some(p) = prev {
            item.set("Prev", p);
        }
        if let Some(n) = next {
            item.set("Next", n);
        }
        let id = self.doc.add_object(item);
        match prev {
            Some(p) => self.doc.get_dictionary_mut(p)?.set("Next", id),
            None => self.doc.get_dictionary_mut(parent)?.set("First", id),
        }
        match next {
            Some(n) => self.doc.get_dictionary_mut(n)?.set("Prev", id),
            None => self.doc.get_dictionary_mut(parent)?.set("Last", id),
        }
        self.fix_outline_counts()?;
        self.outline_nodes()
            .iter()
            .position(|(n, _)| *n == id)
            .context("the new bookmark is not in the outline")
    }

    /// Give bookmark `index` (a position in [`Pdf::outline`]) a new title.
    pub fn rename_bookmark(&mut self, index: usize, title: &str) -> Result<()> {
        let Some(&(id, _)) = self.outline_nodes().get(index) else {
            bail!("bookmark {} does not exist", index + 1);
        };
        let title = Object::String(encode_text(title), StringFormat::Literal);
        self.doc.get_dictionary_mut(id)?.set("Title", title);
        Ok(())
    }

    /// Remove bookmark `index` (a position in [`Pdf::outline`]) and its children.
    pub fn remove_bookmark(&mut self, index: usize) -> Result<()> {
        let Some(&(id, _)) = self.outline_nodes().get(index) else {
            bail!("bookmark {} does not exist", index + 1);
        };
        let parent = self.outline_link(id, b"Parent");
        let parent = parent.map_or_else(|| self.outline_root(), Ok)?;
        let next = self.outline_link(id, b"Next");
        let prev = self.child_before(parent, id);
        let set = |d: &mut Dictionary, key: &str, to: Option<ObjectId>| match to {
            Some(to) => d.set(key, to),
            None => {
                d.remove(key.as_bytes());
            }
        };
        match prev {
            Some(p) => set(self.doc.get_dictionary_mut(p)?, "Next", next),
            None => set(self.doc.get_dictionary_mut(parent)?, "First", next),
        }
        match next {
            Some(n) => set(self.doc.get_dictionary_mut(n)?, "Prev", prev),
            None => set(self.doc.get_dictionary_mut(parent)?, "Last", prev),
        }
        self.fix_outline_counts()?;
        // With the last bookmark gone, drop the outline altogether.
        let root = self.outline_root()?;
        if self.outline_link(root, b"First").is_none() {
            self.doc.catalog_mut()?.remove(b"Outlines");
        }
        Ok(())
    }

    /// Move bookmark `index` (a position in [`Pdf::outline`]), with the bookmarks inside it,
    /// to `place`, and return its new position. Only where it is changes: what it goes to
    /// and everything else it holds stay as they were. A bookmark moved into a closed one
    /// opens it. A move to where it is already changes nothing ([`bookmark_moves`] says so
    /// beforehand); one into itself, or nesting deeper than the outline allows, is refused.
    pub fn move_bookmark(&mut self, index: usize, place: BookmarkPlace) -> Result<usize> {
        let nodes = self.outline_nodes();
        let items: Vec<OutlineItem> = nodes.iter().map(|(_, i)| i.clone()).collect();
        let Some(plan) = plan_move(&items, index, place)? else {
            return Ok(index);
        };
        let moved = nodes[index].0;
        let root = self.outline_root()?;
        let id = |p: Option<usize>| p.map_or(root, |p| nodes[p].0);
        // The lists it leaves and joins are linked again from the outline as read, which
        // also puts right any wrong links a file had in them.
        if plan.from != plan.to {
            let left = children(&items, plan.from).into_iter();
            let left: Vec<ObjectId> = left.filter(|&k| k != index).map(|k| nodes[k].0).collect();
            self.relink(id(plan.from), &left)?;
        }
        let joined: Vec<ObjectId> = plan.level.iter().map(|&k| nodes[k].0).collect();
        self.relink(id(plan.to), &joined)?;
        if let Some(p) = plan.to {
            let d = self.doc.get_dictionary_mut(nodes[p].0)?;
            let count = d.get(b"Count").ok().and_then(|o| o.as_i64().ok());
            if count.unwrap_or(0) <= 0 {
                d.set("Count", 1);
            }
        }
        self.fix_outline_counts()?;
        self.outline_nodes()
            .iter()
            .position(|(n, _)| *n == moved)
            .context("the moved bookmark is not in the outline")
    }

    /// Link `kids` in order as `parent`'s children, both ways.
    fn relink(&mut self, parent: ObjectId, kids: &[ObjectId]) -> Result<()> {
        let d = self.doc.get_dictionary_mut(parent)?;
        match (kids.first(), kids.last()) {
            (Some(&first), Some(&last)) => {
                d.set("First", first);
                d.set("Last", last);
            }
            _ => {
                d.remove(b"First");
                d.remove(b"Last");
            }
        }
        for (n, &kid) in kids.iter().enumerate() {
            let d = self.doc.get_dictionary_mut(kid)?;
            d.set("Parent", parent);
            match n.checked_sub(1) {
                Some(b) => d.set("Prev", kids[b]),
                None => {
                    d.remove(b"Prev");
                }
            }
            match kids.get(n + 1) {
                Some(&next) => d.set("Next", next),
                None => {
                    d.remove(b"Next");
                }
            }
        }
        Ok(())
    }

    /// The child of `parent` before `id`, found by walking the list; a file's Prev links
    /// can be wrong.
    fn child_before(&self, parent: ObjectId, id: ObjectId) -> Option<ObjectId> {
        let mut prev = None;
        let mut seen = HashSet::new();
        let mut at = self.outline_link(parent, b"First");
        while let Some(a) = at.filter(|&a| a != id && seen.insert(a)) {
            prev = Some(a);
            at = self.outline_link(a, b"Next");
        }
        prev
    }

    /// The outline root's object, made (or moved out of the catalog) when needed.
    fn outline_root(&mut self) -> Result<ObjectId> {
        let current = self.doc.catalog()?.get(b"Outlines").ok().cloned();
        if let Some(Object::Reference(id)) = current {
            if self.doc.get_dictionary(id).is_ok() {
                return Ok(id);
            }
        }
        let root = match current {
            Some(Object::Dictionary(d)) => d,
            _ => dictionary! { "Type" => "Outlines" },
        };
        let id = self.doc.add_object(root);
        self.doc.catalog_mut()?.set("Outlines", id);
        Ok(id)
    }

    fn outline_link(&self, id: ObjectId, key: &[u8]) -> Option<ObjectId> {
        let d = self.doc.get_dictionary(id).ok()?;
        d.get(key).ok()?.as_reference().ok()
    }

    /// The last of `parent`'s children, found by walking them.
    fn last_child(&self, parent: ObjectId) -> Option<ObjectId> {
        let mut seen = HashSet::new();
        let mut last = None;
        let mut at = self.outline_link(parent, b"First");
        while let Some(a) = at.filter(|&a| seen.insert(a)) {
            last = Some(a);
            at = self.outline_link(a, b"Next");
        }
        last
    }

    /// Set every `Count` from the tree as it now is: the number of descendants shown,
    /// negative for a closed bookmark.
    fn fix_outline_counts(&mut self) -> Result<()> {
        let root = self.outline_root()?;
        let shown = self.fix_counts(root, 0, &mut HashSet::new())?;
        let d = self.doc.get_dictionary_mut(root)?;
        match shown {
            0 => {
                d.remove(b"Count");
            }
            n => d.set("Count", n),
        }
        Ok(())
    }

    /// Fix the counts below `id`, returning how many descendants it shows when open.
    fn fix_counts(
        &mut self,
        id: ObjectId,
        depth: usize,
        seen: &mut HashSet<ObjectId>,
    ) -> Result<i64> {
        let mut shown = 0;
        let mut child = self.outline_link(id, b"First");
        while let Some(c) = child.filter(|&c| depth < MAX_DEPTH && seen.insert(c)) {
            let below = self.fix_counts(c, depth + 1, seen)?;
            let d = self.doc.get_dictionary_mut(c)?;
            let open = d
                .get(b"Count")
                .ok()
                .and_then(|o| o.as_i64().ok())
                .unwrap_or(0)
                > 0;
            match below {
                0 => {
                    d.remove(b"Count");
                }
                n if open => d.set("Count", n),
                n => d.set("Count", -n),
            }
            shown += 1 + if open { below } else { 0 };
            child = self.outline_link(c, b"Next");
        }
        Ok(shown)
    }

    /// The bookmarks with their objects, in [`Pdf::outline`] order. A bookmark keeps its
    /// object when it is moved, renamed or opened, and through Undo, so the object tells
    /// which bookmark is which from one edit to the next.
    pub fn outline_nodes(&self) -> Vec<(ObjectId, OutlineItem)> {
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
        out: &mut Vec<(ObjectId, OutlineItem)>,
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
                .map(decode_shown_text)
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
            let item = OutlineItem {
                title,
                depth,
                page: self.item_target(node, pages),
                open,
                has_children: child.is_some() && depth + 1 < MAX_DEPTH,
            };
            out.push((id, item));
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
        // A tree too deep or too large to look through is a destination that can't be followed.
        trees::name(&self.doc, tree, name).found()
    }

    /// Each page object's zero-based index.
    pub(crate) fn page_index(&self) -> HashMap<ObjectId, usize> {
        crate::doc::pages(&self.doc)
            .enumerate()
            .map(|(n, id)| (id, n))
            .collect()
    }
}

/// A move [`plan_move`] worked out, by positions in the outline before it: the bookmark it
/// is in (`None` for the top level) and the one it goes into, and that one's children in
/// their new order, the moved bookmark among them.
struct Move {
    from: Option<usize>,
    to: Option<usize>,
    level: Vec<usize>,
}

/// Where moving bookmark `index` of `items` to `place` puts it, or `None` when it is there
/// already.
fn plan_move(items: &[OutlineItem], index: usize, place: BookmarkPlace) -> Result<Option<Move>> {
    // A list cut short at the limit would lose the bookmarks past it when linked again.
    if items.len() >= MAX_ITEMS {
        bail!("this document has too many bookmarks to move them");
    }
    let Some(item) = items.get(index) else {
        bail!("bookmark {} does not exist", index + 1);
    };
    let end = subtree_end(items, index);
    let target = |t: usize| {
        if t >= items.len() {
            bail!("bookmark {} does not exist", t + 1);
        }
        if t != index && (index..end).contains(&t) {
            bail!("a bookmark can't go inside itself");
        }
        Ok(t)
    };
    let (to, at) = match place {
        BookmarkPlace::End => (None, None),
        BookmarkPlace::Before(t) | BookmarkPlace::After(t) => {
            if target(t)? == index {
                return Ok(None);
            }
            (parent(items, t), Some(t))
        }
        BookmarkPlace::Under(t) => {
            if target(t)? == index {
                bail!("a bookmark can't go inside itself");
            }
            (Some(t), None)
        }
    };
    let depth = to.map_or(0, |p| items[p].depth + 1);
    let below = (index..end)
        .map(|k| items[k].depth - item.depth)
        .max()
        .unwrap_or(0);
    if depth + below >= MAX_DEPTH {
        bail!("bookmarks can't be nested deeper than {MAX_DEPTH} levels");
    }
    let mut level = children(items, to);
    level.retain(|&k| k != index);
    let k = match (place, at) {
        (BookmarkPlace::Before(_), Some(t)) => level.iter().position(|&k| k == t),
        (BookmarkPlace::After(_), Some(t)) => level.iter().position(|&k| k == t).map(|k| k + 1),
        _ => None,
    };
    level.insert(k.unwrap_or(level.len()), index);
    let from = parent(items, index);
    if from == to && children(items, from) == level {
        return Ok(None);
    }
    Ok(Some(Move { from, to, level }))
}

/// The bookmark bookmark `k` is in, or `None` at the top level.
fn parent(items: &[OutlineItem], k: usize) -> Option<usize> {
    let depth = items[k].depth;
    (0..k).rev().find(|&j| items[j].depth < depth)
}

/// The position just past bookmark `k` and the bookmarks inside it.
fn subtree_end(items: &[OutlineItem], k: usize) -> usize {
    let depth = items[k].depth;
    let inside = items[k + 1..].iter().take_while(|i| i.depth > depth);
    k + 1 + inside.count()
}

/// The positions of the bookmarks directly inside `parent` (`None` for the top level).
fn children(items: &[OutlineItem], parent: Option<usize>) -> Vec<usize> {
    let (start, end, depth) = match parent {
        None => (0, items.len(), 0),
        Some(p) => (p + 1, subtree_end(items, p), items[p].depth + 1),
    };
    (start..end).filter(|&j| items[j].depth == depth).collect()
}

/// The page ranges that bookmarks down to `level` start, in page order, each titled by the first bookmark on its
/// first page. Pages before the first bookmark come first, untitled. Empty when no such bookmark has a page.
fn bookmark_parts(
    items: &[OutlineItem],
    level: usize,
    total: usize,
) -> Vec<(Option<String>, Range<usize>)> {
    let mut starts: Vec<(usize, &str)> = items
        .iter()
        .filter(|i| i.depth < level)
        .filter_map(|i| Some((i.page.filter(|&p| p < total)?, i.title.trim())))
        .collect();
    // A stable sort, so the first bookmark in the outline names a page several go to.
    starts.sort_by_key(|s| s.0);
    starts.dedup_by_key(|s| s.0);
    let mut parts = Vec::new();
    if let Some(&(first, _)) = starts.first().filter(|s| s.0 > 0) {
        parts.push((None, 0..first));
    }
    for (k, &(start, title)) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map_or(total, |s| s.0);
        parts.push((Some(title.to_string()), start..end));
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, depth: usize, page: Option<usize>) -> OutlineItem {
        OutlineItem {
            title: title.into(),
            depth,
            page,
            open: false,
            has_children: false,
        }
    }

    #[test]
    fn parts_start_at_bookmarked_pages() {
        let items = [
            item("One", 0, Some(2)),
            item("One A", 1, Some(3)),
            item("Elsewhere", 0, None),
            item("Two", 0, Some(5)),
            item("Also two", 0, Some(5)),
            item("Past the end", 0, Some(99)),
        ];
        let titled = |t: &str, r: Range<usize>| (Some(t.to_string()), r);
        assert_eq!(
            bookmark_parts(&items, 1, 8),
            vec![(None, 0..2), titled("One", 2..5), titled("Two", 5..8)]
        );
        assert_eq!(
            bookmark_parts(&items, 2, 8),
            vec![
                (None, 0..2),
                titled("One", 2..3),
                titled("One A", 3..5),
                titled("Two", 5..8)
            ]
        );
        // Out of page order in the outline, and a bookmark on the first page.
        let items = [item("B", 0, Some(1)), item("A", 0, Some(0))];
        assert_eq!(
            bookmark_parts(&items, 1, 3),
            vec![titled("A", 0..1), titled("B", 1..3)]
        );
        assert!(bookmark_parts(&[item("x", 0, None)], 1, 3).is_empty());
    }
}
