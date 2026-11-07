//! What redaction leaves in the file outside what it rewrote (S1-01, amendments A2 and A3).
//! Redaction draws its copies of what changed by names of their own, only where they were
//! made, and leaves the originals as they are for the drawings left as they were. Then,
//! with what everything still draws (a [`Pdf::reach`] walk after the change):
//!
//! * Every resource dictionary in the file, whatever holds it and whether or not anything
//!   draws with it, stops listing an original nothing draws any more, and a marked-content
//!   property holding alternative text that was taken off that nothing uses any more.
//!   Dictionaries shared with what didn't change are edited where they are, for those
//!   entries only. Every change is worked out before any is made.
//! * An original that a drawing left as it was still draws is kept for it, unchanged, and
//!   the pages drawing it are reported.
//! * What a drawing that can't be read may use is neither removed nor kept quietly: the
//!   check after redacting fails, naming the page.
//! * Last, the file is followed from its trailer as saving keeps it, and a route still
//!   reaching anything removed (the tag tree, private application data, any other part of
//!   the file) fails the check, saying which.

use crate::reach::{self, Reach};
use crate::Pdf;
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{BTreeSet, HashSet};

/// What redaction took out of the pages, to be looked for in the rest of the file.
#[derive(Default)]
pub(crate) struct Removed {
    /// The originals of the pictures and forms replaced or removed under a mark.
    pub xobjects: HashSet<ObjectId>,
    /// The alternative text taken off marked content.
    pub alts: HashSet<String>,
    /// What the redacted pages no longer hold: their content before redacting, their
    /// thumbnails and what their private application data pointed at.
    pub pages: HashSet<ObjectId>,
    /// The annotations removed, with the replies to them and their popups.
    pub annots: HashSet<ObjectId>,
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
        let mut o = doc.objects.get(&self.id)?;
        for s in &self.path {
            o = match s {
                Step::Key(k) => dict(o)?.get(k).ok()?,
                Step::Index(i) => o.as_array().ok()?.get(*i)?,
            };
        }
        dict(o)
    }

    fn get_mut<'a>(&self, doc: &'a mut Document) -> Option<&'a mut Dictionary> {
        let mut o = doc.objects.get_mut(&self.id)?;
        for s in &self.path {
            o = match s {
                Step::Key(k) => dict_mut(o)?.get_mut(k).ok()?,
                Step::Index(i) => o.as_array_mut().ok()?.get_mut(*i)?,
            };
        }
        dict_mut(o)
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

/// The `/XObject` and the `/Properties` dictionaries of every resource dictionary in the
/// file.
fn maps(doc: &Document) -> (HashSet<Place>, HashSet<Place>) {
    let (mut xobjects, mut properties) = (HashSet::new(), HashSet::new());
    for r in resources(doc) {
        let Some(res) = r.get(doc) else {
            continue;
        };
        for (key, out) in [
            (&b"XObject"[..], &mut xobjects),
            (b"Properties", &mut properties),
        ] {
            if let Ok(m) = res.get(key) {
                out.insert(r.to(Step::Key(key.to_vec()), m));
            }
        }
    }
    (xobjects, properties)
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

    /// After redacting: take what `removed` left in the file and nothing draws or uses any
    /// more out of every resource dictionary, as `reach` (a walk after the change) found,
    /// then look for anything removed that the file still keeps another way.
    pub(crate) fn clear_retained(&mut self, removed: &Removed, reach: &Reach) -> Retained {
        let n = self.page_count();
        let nodes = &reach.nodes;
        let certain = reach.certain(n);
        // Drawn or used by a drawing certainly drawn whose content can be read.
        let sure = |i: usize| certain[i] && !nodes[i].unread;
        let mut sure_x = HashSet::new();
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
        let originals = &removed.xobjects;
        let kept: HashSet<ObjectId> = (originals.iter())
            .filter(|x| sure_x.contains(*x))
            .copied()
            .collect();
        let unsure: HashSet<ObjectId> = (originals.iter())
            .filter(|x| reach.drawn.contains(*x) && !sure_x.contains(*x))
            .copied()
            .collect();
        let unused = |x: &ObjectId| originals.contains(x) && !reach.drawn.contains(x);
        let mut targets: HashSet<ObjectId> =
            originals.iter().filter(|x| unused(x)).copied().collect();

        // Worked out first, changing nothing: the entries to remove, and where.
        let (xmaps, pmaps) = maps(&self.doc);
        let mut edits: Vec<(Place, Vec<Vec<u8>>)> = Vec::new();
        for m in xmaps {
            let Some(d) = m.get(&self.doc) else {
                continue;
            };
            let gone: Vec<Vec<u8>> = (d.iter())
                .filter(|(_, v)| v.as_reference().is_ok_and(|id| unused(&id)))
                .map(|(k, _)| k.clone())
                .collect();
            if !gone.is_empty() {
                edits.push((m, gone));
            }
        }
        let mut unsure_props = Props::default();
        for m in pmaps.into_iter().filter(|_| !removed.alts.is_empty()) {
            let Some(d) = m.get(&self.doc) else {
                continue;
            };
            let mut gone = Vec::new();
            for (k, v) in d.iter() {
                let alt = reach::alt_texts(Some(&self.doc), &self.resolved_dict(Some(v)));
                if !alt.iter().any(|a| removed.alts.contains(a)) || sure_props.has(v) {
                    continue;
                }
                if any_props.has(v) {
                    unsure_props.add(v);
                    continue;
                }
                gone.push(k.clone());
                targets.extend(v.as_reference().ok());
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
        let pictures = match kept.is_empty() {
            true => Vec::new(),
            false => reach.pages_certainly_drawing(n, |i| {
                !nodes[i].unread && nodes[i].xobjects.iter().any(|x| kept.contains(x))
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

        let contents: HashSet<ObjectId> = (self.page_ids().into_iter())
            .flat_map(|p| self.content_ids(p))
            .collect();
        targets.extend(removed.pages.iter().filter(|id| !contents.contains(id)));
        targets.extend(&removed.annots);
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
