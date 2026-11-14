//! What redaction leaves in the file outside what it rewrote (S1-01, amendments A2 and A3).
//! Redaction draws its copies of what changed by names of their own, only where they were
//! made, and leaves the originals as they are for the drawings left as they were. Then,
//! with what everything still draws (a [`Pdf::reach`] walk after the change):
//!
//! * Every resource dictionary in the file, whatever holds it and whether or not anything
//!   draws with it, stops listing an original nothing draws any more, a pattern or soft
//!   mask painted with before and not after (PB-177), and a marked-content property
//!   holding alternative text that was taken off that nothing uses any more.
//!   Dictionaries shared with what didn't change are edited where they are, for those
//!   entries only. The document's named appearances (Acrobat's cache of the icons it drew
//!   comments with) stop naming an appearance nothing draws any more. Every change is
//!   worked out before any is made.
//! * An original that a drawing left as it was still draws is kept for it, unchanged, and
//!   the pages drawing it are reported. The appearances of the annotations removed, and the
//!   pictures and forms they draw, count as originals, and so do an original picture's soft
//!   mask, mask and alternate versions: they go with it, or are kept with it, and a picture
//!   still drawn that shares one is kept and reported as an original is.
//! * What a drawing that can't be read may use is neither removed nor kept quietly: the
//!   check after redacting fails, naming the page.
//! * Last, the file is followed from its trailer as saving keeps it, and a route still
//!   reaching anything removed (the tag tree, private application data, any other part of
//!   the file) fails the check, saying which. Removed with what held them: alternative text
//!   stored as an object of its own that no property still in the file holds, and what an
//!   annotation removed held as objects of its own that no annotation still listed holds.
//!
//! Removing hidden layers takes the same steps (PB-153): the originals are the pictures and
//! forms drawn before and not after, and patterns and soft masks go as they do for
//! redaction. So does removing comments or attached files, and the annotations hidden layers
//! take (PB-182): the annotations removed, their appearances and what they held must not be
//! reachable any more.

use crate::reach::{self, mask_drawing, Reach, ALT};
use crate::{trees, Pdf};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{BTreeSet, HashMap, HashSet};

/// What redaction took out of the pages, to be looked for in the rest of the file.
#[derive(Default)]
pub(crate) struct Removed {
    /// The originals of the pictures and forms replaced or removed under a mark, and those
    /// the appearances of the annotations removed draw.
    pub xobjects: HashSet<ObjectId>,
    /// The alternative text taken off marked content.
    pub alts: HashSet<String>,
    /// The alternative text taken off marked content and tag-tree elements that was an
    /// object of its own.
    pub texts: HashSet<ObjectId>,
    /// The properties alternative text was taken off that were objects of their own.
    pub properties: HashSet<ObjectId>,
    /// What the redacted pages no longer hold: their content before redacting, their
    /// thumbnails and what their private application data pointed at.
    pub pages: HashSet<ObjectId>,
    /// The annotations removed, with the replies to them and their popups.
    pub annots: HashSet<ObjectId>,
    /// The appearances of the annotations removed.
    pub looks: HashSet<ObjectId>,
    /// What the annotations removed held as objects of their own (see
    /// [`Pdf::annot_holds`]).
    pub held: HashSet<ObjectId>,
    /// The patterns and soft masks' drawings painted with before the change
    /// ([`Reach::painted`]): those nothing paints with after it go.
    pub painted: HashSet<ObjectId>,
}

/// What [`Pdf::clear_retained`] found.
#[derive(Default)]
pub(crate) struct Retained {
    /// The pages (zero-based, in order) where a drawing left as it was still draws an
    /// original.
    pub pictures: Vec<usize>,
    /// The pages (zero-based, in order) with a drawing that can't be read and may draw an
    /// original or use a property, which were kept for it.
    pub unread: Vec<usize>,
    /// The other ways the file still keeps something removed, a sentence for each.
    pub left: Vec<String>,
}

/// What the check after removing says of page `p` (zero-based), where a drawing that can't be
/// read may use something removed.
pub(crate) fn unread_said(p: usize) -> String {
    format!(
        "a drawing on page {} couldn't be read, so removed content may still be in the file",
        p + 1
    )
}

/// The routes [`routes_to`] tells apart, in the order it follows them.
const ROUTES: [&str; 3] = [
    "another part of the file",
    "the tag tree",
    "private application data",
];

/// A step into a direct object: a dictionary's key or an array's index.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Step {
    Key(Vec<u8>),
    Index(usize),
}

/// Where a dictionary is in the file: object `id` itself, or the direct dictionary at `path`
/// inside it.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Place {
    id: ObjectId,
    path: Vec<Step>,
}

fn dict(o: &Object) -> Option<&Dictionary> {
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn dict_mut(o: &mut Object) -> Option<&mut Dictionary> {
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&mut s.dict),
        _ => None,
    }
}

impl Place {
    fn get<'a>(&self, doc: &'a Document) -> Option<&'a Dictionary> {
        dict(self.object(doc)?)
    }

    fn object<'a>(&self, doc: &'a Document) -> Option<&'a Object> {
        let mut o = doc.objects.get(&self.id)?;
        for s in &self.path {
            o = match s {
                Step::Key(k) => dict(o)?.get(k).ok()?,
                Step::Index(i) => o.as_array().ok()?.get(*i)?,
            };
        }
        Some(o)
    }

    fn get_mut<'a>(&self, doc: &'a mut Document) -> Option<&'a mut Dictionary> {
        dict_mut(self.object_mut(doc)?)
    }

    fn object_mut<'a>(&self, doc: &'a mut Document) -> Option<&'a mut Object> {
        let mut o = doc.objects.get_mut(&self.id)?;
        for s in &self.path {
            o = match s {
                Step::Key(k) => dict_mut(o)?.get_mut(k).ok()?,
                Step::Index(i) => o.as_array_mut().ok()?.get_mut(*i)?,
            };
        }
        Some(o)
    }

    /// The array here, if it is one.
    fn list<'a>(&self, doc: &'a Document) -> Option<&'a Vec<Object>> {
        self.object(doc)?.as_array().ok()
    }

    fn array_mut<'a>(&self, doc: &'a mut Document) -> Option<&'a mut Vec<Object>> {
        self.object_mut(doc)?.as_array_mut().ok()
    }

    /// Where `o`, found by `step` from here, is: the object it refers to, or here.
    fn to(&self, step: Step, o: &Object) -> Place {
        match o {
            Object::Reference(id) => Place {
                id: *id,
                path: Vec::new(),
            },
            _ => {
                let mut path = self.path.clone();
                path.push(step);
                Place { id: self.id, path }
            }
        }
    }
}

/// Every resource dictionary in the file: the value of a `/Resources` key, or of the
/// interactive form's `/DR`, at any depth in any object.
fn resources(doc: &Document) -> HashSet<Place> {
    let nested = |o: &Object| matches!(o, Object::Array(_) | Object::Dictionary(_));
    let mut out = HashSet::new();
    for (&id, o) in &doc.objects {
        let mut todo = vec![(
            o,
            Place {
                id,
                path: Vec::new(),
            },
        )];
        while let Some((o, here)) = todo.pop() {
            if let Object::Array(a) = o {
                for (i, x) in a.iter().enumerate().filter(|(_, x)| nested(x)) {
                    todo.push((x, here.to(Step::Index(i), x)));
                }
                continue;
            }
            let Some(d) = dict(o) else {
                continue;
            };
            for (k, v) in d.iter() {
                if matches!(k.as_slice(), b"Resources" | b"DR") {
                    out.insert(here.to(Step::Key(k.clone()), v));
                }
                if nested(v) {
                    todo.push((v, here.to(Step::Key(k.clone()), v)));
                }
            }
        }
    }
    out
}

/// The `keys` dictionaries (`/XObject`, `/Properties`…) of every resource dictionary in the
/// file, a set for each key.
fn maps<const N: usize>(doc: &Document, keys: [&[u8]; N]) -> [HashSet<Place>; N] {
    let mut out: [HashSet<Place>; N] = std::array::from_fn(|_| HashSet::new());
    for r in resources(doc) {
        let Some(res) = r.get(doc) else {
            continue;
        };
        for (key, set) in keys.iter().zip(&mut out) {
            if let Ok(m) = res.get(key) {
                set.insert(r.to(Step::Key(key.to_vec()), m));
            }
        }
    }
    out
}

/// In each of `maps`, dictionaries of one key of resource dictionaries, the names of the
/// entries `gone` says go.
fn unlisted(
    doc: &Document,
    maps: HashSet<Place>,
    gone: impl Fn(&Object) -> bool,
) -> Vec<(Place, Vec<Vec<u8>>)> {
    let mut edits = Vec::new();
    for m in maps {
        let Some(d) = m.get(doc) else {
            continue;
        };
        let names: Vec<Vec<u8>> = (d.iter())
            .filter(|(_, v)| gone(v))
            .map(|(k, _)| k.clone())
            .collect();
        if !names.is_empty() {
            edits.push((m, names));
        }
    }
    edits
}

/// The leaves of the document's named appearances (the catalog's `/Names /AP` tree, where
/// Acrobat keeps the icons it drew comments with), down to [`trees::MAX_DEPTH`]: one listed
/// deeper still keeps the route check failing.
fn named_appearances(doc: &Document) -> Vec<Place> {
    let at = |here: &Place, key: &[u8]| {
        let o = here.get(doc)?.get(key).ok()?;
        Some(here.to(Step::Key(key.to_vec()), o))
    };
    let Ok(root) = doc.trailer.get(b"Root").and_then(|o| o.as_reference()) else {
        return Vec::new();
    };
    let catalog = Place {
        id: root,
        path: Vec::new(),
    };
    let Some(tree) = at(&catalog, b"Names").and_then(|n| at(&n, b"AP")) else {
        return Vec::new();
    };
    let (mut todo, mut seen, mut out) = (vec![(tree, 0)], HashSet::new(), Vec::new());
    while let Some((here, depth)) = todo.pop() {
        if depth > trees::MAX_DEPTH || !seen.insert(here.clone()) {
            continue;
        }
        let Some(d) = here.get(doc) else {
            continue;
        };
        if d.has(b"Names") {
            out.push(here.clone());
        }
        let Some(kids) = at(&here, b"Kids") else {
            continue;
        };
        for (i, k) in (kids.list(doc).into_iter().flatten()).enumerate() {
            todo.push((kids.to(Step::Index(i), k), depth + 1));
        }
    }
    out
}

/// The pairs of the named appearances' leaf `leaf`, as an array to edit.
fn names_mut<'a>(doc: &'a mut Document, leaf: &Place) -> Option<&'a mut Vec<Object>> {
    let names = leaf.get(doc)?.get(b"Names").ok()?;
    leaf.to(Step::Key(b"Names".to_vec()), names).array_mut(doc)
}

/// Marked-content properties as resource dictionaries list them: by the object they refer
/// to, or as written there.
#[derive(Default)]
struct Props {
    refs: HashSet<ObjectId>,
    direct: Vec<Object>,
}

impl Props {
    fn add(&mut self, o: &Object) {
        match o {
            Object::Reference(id) => {
                self.refs.insert(*id);
            }
            o if !self.direct.contains(o) => self.direct.push(o.clone()),
            _ => {}
        }
    }

    fn has(&self, o: &Object) -> bool {
        match o {
            Object::Reference(id) => self.refs.contains(id),
            o => self.direct.contains(o),
        }
    }

    fn is_empty(&self) -> bool {
        self.refs.is_empty() && self.direct.is_empty()
    }

    /// The alternative text the properties hold as objects of their own.
    fn alt_refs(&self, doc: &Document) -> HashSet<ObjectId> {
        let refs = (self.refs.iter()).filter_map(|id| doc.get_dictionary(*id).ok());
        let direct = self.direct.iter().filter_map(|o| o.as_dict().ok());
        refs.chain(direct).flat_map(alt_refs).collect()
    }
}

/// The alternative text in `d` stored as objects of their own.
pub(crate) fn alt_refs(d: &Dictionary) -> Vec<ObjectId> {
    (ALT.iter())
        .filter_map(|k| d.get(k).and_then(|o| o.as_reference()).ok())
        .collect()
}

/// The parts of picture `id` that are pictures of their own: its soft mask, its mask (not a
/// colour-key one) and its alternate versions.
fn parts(doc: &Document, id: ObjectId) -> Vec<ObjectId> {
    let Ok(s) = doc.get_object(id).and_then(|o| o.as_stream()) else {
        return Vec::new();
    };
    let mut out: Vec<ObjectId> = ([&b"SMask"[..], b"Mask"].iter())
        .filter_map(|k| s.dict.get(k).and_then(|o| o.as_reference()).ok())
        .collect();
    let alternates = (s.dict.get(b"Alternates").ok())
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_array().ok());
    for a in alternates.into_iter().flatten() {
        let d = doc.dereference(a).ok().and_then(|(_, o)| o.as_dict().ok());
        out.extend(d.and_then(|d| d.get(b"Image").and_then(|o| o.as_reference()).ok()));
    }
    out.retain(|id| doc.get_object(*id).is_ok_and(|o| o.as_stream().is_ok()));
    out
}

/// The parts, at any depth, of the pictures `drawn`, with the pictures holding each.
fn part_holders(doc: &Document, drawn: &HashSet<ObjectId>) -> HashMap<ObjectId, Vec<ObjectId>> {
    let mut out: HashMap<ObjectId, Vec<ObjectId>> = HashMap::new();
    for &x in drawn {
        let mut seen = HashSet::from([x]);
        let mut todo = parts(doc, x);
        while let Some(p) = todo.pop() {
            if seen.insert(p) {
                out.entry(p).or_default().push(x);
                todo.extend(parts(doc, p));
            }
        }
    }
    out
}

/// The parts, at any depth, of the `originals` nothing `drawn` draws, which go with them. A
/// part drawn itself, or a part of a picture drawn, is kept: it, or that picture, is added
/// to `originals`.
fn parts_gone(
    doc: &Document,
    originals: &mut HashSet<ObjectId>,
    drawn: &HashSet<ObjectId>,
) -> HashSet<ObjectId> {
    let mut gone = HashSet::new();
    let mut holders = None;
    let mut todo: Vec<ObjectId> = (originals.iter())
        .filter(|x| !drawn.contains(*x))
        .flat_map(|&x| parts(doc, x))
        .collect();
    while let Some(p) = todo.pop() {
        if originals.contains(&p) || gone.contains(&p) {
            continue;
        }
        if drawn.contains(&p) {
            originals.insert(p);
            continue;
        }
        let holders = holders.get_or_insert_with(|| part_holders(doc, drawn));
        if let Some(by) = holders.get(&p) {
            originals.extend(by);
            continue;
        }
        gone.insert(p);
        todo.extend(parts(doc, p));
    }
    gone
}

/// The references in `o` itself, not following them.
pub(crate) fn references_in(o: &Object) -> Vec<ObjectId> {
    let mut out = Vec::new();
    let mut todo = vec![o];
    while let Some(o) = todo.pop() {
        match o {
            Object::Reference(id) => out.push(*id),
            Object::Array(a) => todo.extend(a),
            Object::Dictionary(d) => todo.extend(d.iter().map(|(_, v)| v)),
            Object::Stream(s) => todo.extend(s.dict.iter().map(|(_, v)| v)),
            _ => {}
        }
    }
    out
}

/// Which of [`ROUTES`] still reach any of `targets`, following the file from its trailer as
/// saving keeps it: the tag tree and private application data after everything else, so
/// what something else points at too counts as that.
fn routes_to(doc: &Document, targets: &HashSet<ObjectId>) -> [bool; 3] {
    let mut found = [false; 3];
    if targets.is_empty() {
        return found;
    }
    let mut roots: [Vec<&Object>; 3] = [
        doc.trailer.iter().map(|(_, o)| o).collect(),
        Vec::new(),
        Vec::new(),
    ];
    let mut seen = HashSet::new();
    for route in 0..ROUTES.len() {
        let mut todo = std::mem::take(&mut roots[route]);
        while let Some(o) = todo.pop() {
            let d = match o {
                Object::Reference(id) => {
                    if seen.insert(*id) {
                        found[route] |= targets.contains(id);
                        todo.extend(doc.objects.get(id));
                    }
                    continue;
                }
                Object::Array(a) => {
                    todo.extend(a);
                    continue;
                }
                Object::Dictionary(d) => d,
                Object::Stream(s) => &s.dict,
                _ => continue,
            };
            for (k, v) in d.iter() {
                let to = match k.as_slice() {
                    b"StructTreeRoot" => 1,
                    b"PieceInfo" => 2,
                    _ => 0,
                };
                match to.max(route) {
                    to if to == route => todo.push(v),
                    to => roots[to].push(v),
                }
            }
        }
    }
    found
}

impl Pdf {
    /// The content streams of `page`, and the array listing them when that is an object of
    /// its own.
    pub(crate) fn content_ids(&self, page: ObjectId) -> Vec<ObjectId> {
        let Some(c) = (self.doc.get_dictionary(page).ok()).and_then(|d| d.get(b"Contents").ok())
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let list = match c {
            Object::Reference(id) => {
                out.push(*id);
                self.doc.objects.get(id)
            }
            o => Some(o),
        };
        if let Some(Object::Array(a)) = list {
            out.extend(a.iter().filter_map(|o| o.as_reference().ok()));
        }
        out
    }

    /// What annotation `d` holds as objects of their own, besides its appearances: `/AP`
    /// and its dictionaries of appearances by state, its text and its rich text.
    pub(crate) fn annot_holds(&self, d: &Dictionary) -> Vec<ObjectId> {
        let mut out = self.appearances_of(d).1;
        out.extend(
            ([&b"Contents"[..], b"RC"].iter())
                .filter_map(|k| d.get(k).and_then(|o| o.as_reference()).ok()),
        );
        out
    }

    /// Into `removed`, the appearances of the annotations it holds, which have left their
    /// pages, and what else they held as objects of their own.
    pub(crate) fn annots_held(&self, removed: &mut Removed) {
        for &id in &removed.annots {
            let Ok(d) = self.doc.get_dictionary(id) else {
                continue;
            };
            removed.looks.extend(self.appearances_of(d).0);
            removed.held.extend(self.annot_holds(d));
        }
    }

    /// After redacting: take what `removed` left in the file and nothing draws or uses any
    /// more out of every resource dictionary, as `reach` (a walk after the change) found,
    /// then look for anything removed that the file still keeps another way.
    pub(crate) fn clear_retained(&mut self, removed: &Removed, reach: &Reach) -> Retained {
        let n = self.page_count();
        let nodes = &reach.nodes;
        let certain = reach.certain(n);
        // Drawn or used by a drawing certainly drawn whose content can be read; an
        // annotation's appearance is drawn whether or not it can be read.
        let sure = |i: usize| certain[i] && !nodes[i].unread;
        let mut sure_x: HashSet<ObjectId> = reach.looks.keys().copied().collect();
        let (mut sure_props, mut any_props) = (Props::default(), Props::default());
        for (i, node) in nodes.iter().enumerate() {
            for p in &node.properties {
                any_props.add(p);
                if sure(i) {
                    sure_props.add(p);
                }
            }
            if sure(i) {
                sure_x.extend(node.xobjects.iter().copied());
            }
        }
        let drawn: HashSet<ObjectId> = (reach.drawn.iter())
            .chain(reach.looks.keys())
            .copied()
            .collect();
        let mut originals: HashSet<ObjectId> =
            removed.xobjects.union(&removed.looks).copied().collect();
        let parts = parts_gone(&self.doc, &mut originals, &drawn);
        let kept: HashSet<ObjectId> = (originals.iter())
            .filter(|x| sure_x.contains(*x))
            .copied()
            .collect();
        let unsure: HashSet<ObjectId> = (originals.iter())
            .filter(|x| drawn.contains(*x) && !sure_x.contains(*x))
            .copied()
            .collect();
        let unused =
            |x: &ObjectId| parts.contains(x) || (originals.contains(x) && !drawn.contains(x));
        let painted: HashSet<ObjectId> = (removed.painted.difference(&reach.painted))
            .copied()
            .collect();
        let mut targets: HashSet<ObjectId> = (originals.iter().chain(&parts))
            .filter(|x| unused(x))
            .copied()
            .chain(painted.iter().copied())
            .collect();

        // Worked out first, changing nothing: the entries to remove, and where.
        let keys = [&b"XObject"[..], b"Pattern", b"ExtGState", b"Properties"];
        let [xmaps, patterns, states, pmaps] = maps(&self.doc, keys);
        let doc = &self.doc;
        let mut edits = unlisted(doc, xmaps, |v| v.as_reference().is_ok_and(|id| unused(&id)));
        let painted_with = |id: Option<ObjectId>| id.is_some_and(|id| painted.contains(&id));
        edits.extend(unlisted(doc, patterns, |v| {
            painted_with(v.as_reference().ok())
        }));
        edits.extend(unlisted(doc, states, |v| {
            painted_with(mask_drawing(self, &self.resolved_dict(Some(v))))
        }));
        // The named appearances' pairs naming an appearance nothing draws any more.
        let named: Vec<(Place, HashSet<usize>)> = (named_appearances(doc).into_iter())
            .filter_map(|leaf| {
                let list = leaf.get(doc)?.get(b"Names").ok()?;
                let list = doc.dereference(list).ok()?.1.as_array().ok()?;
                let gone: HashSet<usize> = (list.as_chunks::<2>().0.iter().enumerate())
                    .filter(|(_, [_, v])| v.as_reference().is_ok_and(|id| unused(&id)))
                    .map(|(i, _)| i)
                    .collect();
                (!gone.is_empty()).then_some((leaf, gone))
            })
            .collect();
        let mut unsure_props = Props::default();
        let mut texts = removed.texts.clone();
        for m in pmaps.into_iter().filter(|_| !removed.alts.is_empty()) {
            let Some(d) = m.get(&self.doc) else {
                continue;
            };
            let mut gone = Vec::new();
            for (k, v) in d.iter() {
                let prop = self.resolved_dict(Some(v));
                let alt = reach::alt_texts(Some(&self.doc), &prop);
                if !alt.iter().any(|a| removed.alts.contains(a)) || sure_props.has(v) {
                    continue;
                }
                if any_props.has(v) {
                    unsure_props.add(v);
                    continue;
                }
                gone.push(k.clone());
                targets.extend(v.as_reference().ok());
                texts.extend(alt_refs(&prop));
            }
            if !gone.is_empty() {
                edits.push((m, gone));
            }
        }
        let mut unread = BTreeSet::new();
        if !unsure.is_empty() || !unsure_props.is_empty() {
            unread.extend(reach.pages_drawing(n, |i| {
                nodes[i].xobjects.iter().any(|x| unsure.contains(x))
                    || nodes[i].properties.iter().any(|p| unsure_props.has(p))
            }));
        }
        let kept_looks: HashSet<usize> = (reach.looks.iter())
            .filter(|(id, _)| kept.contains(*id))
            .map(|(_, &i)| i)
            .collect();
        let pictures = match kept.is_empty() {
            true => Vec::new(),
            false => reach.pages_certainly_drawing(n, |i| {
                kept_looks.contains(&i)
                    || (!nodes[i].unread && nodes[i].xobjects.iter().any(|x| kept.contains(x)))
            }),
        };

        // Then made together. A place missing now was inside an entry removed before it.
        for (m, gone) in edits {
            if let Some(d) = m.get_mut(&mut self.doc) {
                for k in &gone {
                    d.remove(k);
                }
            }
        }
        for (leaf, gone) in named {
            if let Some(list) = names_mut(&mut self.doc, &leaf) {
                let mut i = 0;
                list.retain(|_| {
                    i += 1;
                    !gone.contains(&((i - 1) / 2))
                });
            }
        }

        let contents: HashSet<ObjectId> = (self.page_ids().into_iter())
            .flat_map(|p| self.content_ids(p))
            .collect();
        targets.extend(removed.pages.iter().filter(|id| !contents.contains(id)));
        targets.extend(&removed.annots);
        let still = any_props.alt_refs(&self.doc);
        targets.extend(texts.iter().filter(|id| !still.contains(id)));
        let used = |id: &&ObjectId| any_props.has(&Object::Reference(**id));
        targets.extend(removed.properties.iter().filter(|id| !used(id)));
        if !removed.held.is_empty() {
            let listed: HashSet<ObjectId> = (self.page_ids().into_iter())
                .flat_map(|p| self.all_annot_refs(p))
                .filter_map(|a| self.doc.get_dictionary(a).ok())
                .flat_map(|d| self.annot_holds(d))
                .collect();
            targets.extend(removed.held.iter().filter(|id| !listed.contains(id)));
        }
        targets.retain(|id| self.doc.objects.contains_key(id));
        let left = (ROUTES.iter().zip(routes_to(&self.doc, &targets)))
            .filter(|(_, found)| *found)
            .map(|(route, _)| format!("removed content may still be in the file, through {route}"))
            .collect();
        Retained {
            pictures,
            unread: unread.into_iter().collect(),
            left,
        }
    }
}
