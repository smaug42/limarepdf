//! The drawings a document's pages reach, followed as a viewer draws them: each page's
//! content and its annotations' appearances, then the forms, tiling patterns, Type 3 glyphs
//! and soft masks those draw, each with the resources it draws with. One walk, with a budget
//! ([`Work`]) charged across all of it, so a file built to be followed for ever is refused
//! (before anything is changed, when the walk comes first) instead of followed.
//!
//! A drawing with resources of its own looks the same wherever it is drawn, so it is followed
//! once; one without draws with the resources of where it is drawn, so it is followed once
//! for each different set of them.

use crate::budget::{self, TooComplex, Work};
use crate::Pdf;
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// The steps a walk may take: one for each drawing reached, each time it is reached, one for
/// each name looked up, and one for each entry of the resources a drawing without its own
/// is told apart by. 32 times the most the test files take (129,066, a 109-page file from
/// pdf.js's tests; the 1,080 files that open, measured 2026-11-04), so a document like it of
/// about 3,500 pages still passes.
pub(crate) const STEPS: u64 = 1 << 22;

/// What a walk's refusal says it was doing.
const WHAT: &str = "checking the drawings";

/// The keys of marked content's alternative text.
pub(crate) const ALT: [&[u8]; 3] = [b"ActualText", b"Alt", b"E"];

/// The budget for a walk.
pub(crate) fn work() -> Work {
    Work::new(WHAT, budget::limit(STEPS))
}

/// The budget for the walk after an edit that a walk with [`work`]'s budget passed, and that
/// added `added` objects. The edit's copies of drawings cost no more to follow than the
/// drawings they replace, which may still be drawn elsewhere, so twice the limit and a step
/// for each object added leaves the walk room to finish.
pub(crate) fn work_after(added: usize) -> Work {
    let limit = budget::limit(STEPS);
    let added = u64::try_from(added).unwrap_or(u64::MAX);
    Work::new(WHAT, limit.saturating_mul(2).saturating_add(added))
}

/// What drawings' content uses by name, by stream: `None` when it can't be read.
pub(crate) type Decoded = HashMap<ObjectId, Option<Rc<Uses>>>;

/// The names a drawing's content uses, and the alternative text in its marked content's own
/// properties.
#[derive(Default)]
pub(crate) struct Uses {
    /// Pictures and forms drawn (`Do`).
    pub xobjects: Vec<Vec<u8>>,
    /// Marked-content properties (`BDC`, `DP`).
    pub properties: Vec<Vec<u8>>,
    /// Fonts (`Tf`).
    pub fonts: Vec<Vec<u8>>,
    /// Patterns painted with (`scn`, `SCN`).
    pub patterns: Vec<Vec<u8>>,
    /// Graphics states (`gs`).
    pub states: Vec<Vec<u8>>,
    /// Alternative text of marked content whose properties are in the content itself.
    pub alt: Vec<String>,
}

impl Uses {
    pub(crate) fn of_ops(ops: &[Operation]) -> Uses {
        let mut u = Uses::default();
        let name = |o: Option<&Object>| o.and_then(|o| o.as_name().ok()).map(<[u8]>::to_vec);
        for o in ops {
            match o.operator.as_str() {
                "Do" => u.xobjects.extend(name(o.operands.first())),
                "Tf" => u.fonts.extend(name(o.operands.first())),
                "gs" => u.states.extend(name(o.operands.first())),
                "scn" | "SCN" => u.patterns.extend(name(o.operands.last())),
                "BDC" | "DP" => match o.operands.get(1) {
                    Some(Object::Name(n)) => u.properties.push(n.clone()),
                    Some(Object::Dictionary(d)) => u.alt.extend(alt_texts(None, d)),
                    _ => {}
                },
                _ => {}
            }
        }
        for v in [
            &mut u.xobjects,
            &mut u.properties,
            &mut u.fonts,
            &mut u.patterns,
            &mut u.states,
        ] {
            v.sort();
            v.dedup();
        }
        u
    }

    /// What content `data` uses; `None` when it can't be read.
    pub(crate) fn of_data(data: &[u8]) -> Option<Uses> {
        let (data, _) = crate::inline::extract(data).ok()?;
        let content = Content::decode(&data).ok()?;
        Some(Uses::of_ops(&content.operations))
    }

    /// Every name `res` lists of the kinds content uses: what content that can't be read may
    /// use.
    fn listed(pdf: &Pdf, res: &Dictionary) -> Uses {
        let all = |key: &[u8]| -> Vec<Vec<u8>> {
            let d = pdf.resolved_dict(res.get(key).ok());
            d.iter().map(|(k, _)| k.clone()).collect()
        };
        Uses {
            xobjects: all(b"XObject"),
            properties: all(b"Properties"),
            fonts: all(b"Font"),
            patterns: all(b"Pattern"),
            states: all(b"ExtGState"),
            alt: Vec::new(),
        }
    }
}

/// The alternative text in marked-content properties `d`; `doc` to follow references (the
/// properties in content itself have none).
pub(crate) fn alt_texts(doc: Option<&lopdf::Document>, d: &Dictionary) -> Vec<String> {
    ALT.iter()
        .filter_map(|k| {
            let o = d.get(k).ok()?;
            let o = match doc {
                Some(doc) => doc.dereference(o).ok()?.1,
                None => o,
            };
            Some(crate::doc::decode_text(o.as_str().ok()?))
        })
        .collect()
}

/// What [`Pdf::reach`] found.
#[derive(Default)]
pub(crate) struct Reach {
    /// The drawings reached; node `i` below the page count is page `i`'s content, which
    /// draws its annotations' appearances too.
    pub nodes: Vec<Node>,
    /// The objects reached whose `/Resources` something draws with: pages and the page-tree
    /// nodes above them, forms, patterns, soft masks, appearances, Type 3 fonts.
    pub holders: Vec<ObjectId>,
    /// The pictures and forms drawn (`Do`) somewhere; content that can't be read counts as
    /// drawing all its resources list.
    pub drawn: HashSet<ObjectId>,
    /// The annotations' appearances: (the page's node, the appearance's node).
    pub appearances: Vec<(usize, usize)>,
}

/// A drawing [`Pdf::reach`] reached.
#[derive(Default)]
pub(crate) struct Node {
    /// The nodes of the drawings it draws.
    pub draws: Vec<usize>,
    /// The alternative text of its marked content.
    pub alt: Vec<String>,
    /// The pictures and forms it draws (`Do`).
    pub xobjects: Vec<ObjectId>,
    /// The `/Properties` entries its marked content uses, as its resources list them.
    pub properties: Vec<Object>,
    /// Its content can't be read, so it counts as drawing, and using, all its resources
    /// list.
    pub unread: bool,
}

impl Reach {
    /// The pages (zero-based) of `pages` drawing, at any depth, a node `hit` is true of
    /// (by its index).
    pub(crate) fn pages_drawing(&self, pages: usize, hit: impl Fn(usize) -> bool) -> Vec<usize> {
        self.climb(pages, hit, false)
    }

    /// The pages of `pages` certainly drawing a node `hit` is true of: through drawings that
    /// can be read, or as an annotation's appearance.
    pub(crate) fn pages_certainly_drawing(
        &self,
        pages: usize,
        hit: impl Fn(usize) -> bool,
    ) -> Vec<usize> {
        self.climb(pages, hit, true)
    }

    /// Which nodes are certainly drawn: the pages' content, their annotations' appearances,
    /// and what a drawing certainly drawn that can be read draws.
    pub(crate) fn certain(&self, pages: usize) -> Vec<bool> {
        let mut seen = vec![false; self.nodes.len()];
        let roots = (0..pages.min(seen.len())).chain(self.appearances.iter().map(|a| a.1));
        let mut todo = Vec::new();
        for i in roots {
            if !seen[i] {
                seen[i] = true;
                todo.push(i);
            }
        }
        while let Some(i) = todo.pop() {
            if self.nodes[i].unread {
                continue;
            }
            for &d in &self.nodes[i].draws {
                if !seen[d] {
                    seen[d] = true;
                    todo.push(d);
                }
            }
        }
        seen
    }

    /// The pages drawing a node `hit` is true of, through any drawing or, `certain`, only
    /// through those that can be read.
    fn climb(&self, pages: usize, hit: impl Fn(usize) -> bool, certain: bool) -> Vec<usize> {
        let mut up = vec![Vec::new(); self.nodes.len()];
        for (i, n) in self.nodes.iter().enumerate() {
            if certain && n.unread {
                continue;
            }
            for &d in &n.draws {
                up[d].push(i);
            }
        }
        for &(page, look) in &self.appearances {
            up[look].push(page);
        }
        let mut seen: Vec<bool> = (0..self.nodes.len()).map(hit).collect();
        let mut todo: Vec<usize> = (0..seen.len()).filter(|&i| seen[i]).collect();
        while let Some(i) = todo.pop() {
            for &u in &up[i] {
                if !seen[u] {
                    seen[u] = true;
                    todo.push(u);
                }
            }
        }
        (0..pages.min(seen.len())).filter(|&p| seen[p]).collect()
    }
}

/// A walk under way.
struct Walk<'a> {
    pdf: &'a Pdf,
    work: &'a mut Work,
    cache: &'a mut Decoded,
    reach: Reach,
    /// The node of each drawing followed, by stream and the fingerprint of the resources it
    /// draws with when they aren't its own.
    seen: HashMap<(ObjectId, Option<u64>), usize>,
    held: HashSet<ObjectId>,
    /// Drawings to follow: the node, what its content uses, its resources.
    todo: Vec<(usize, Option<Rc<Uses>>, Rc<Dictionary>)>,
}

/// Resources told apart: the same entries, the same fingerprint.
fn fingerprint(res: &Dictionary) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    format!("{res:?}").hash(&mut h);
    h.finish()
}

impl Pdf {
    /// What drawing `id` (a stream) uses by name, kept in `cache`; `None` when it can't be
    /// read.
    pub(crate) fn uses_of(&self, id: ObjectId, cache: &mut Decoded) -> Option<Rc<Uses>> {
        cache
            .entry(id)
            .or_insert_with(|| {
                let s = self.doc.get_object(id).ok()?.as_stream().ok()?;
                let data = s.get_plain_content_with_limit(crate::MAX_CONTENT).ok()?;
                Uses::of_data(&data).map(Rc::new)
            })
            .clone()
    }

    /// Follow every drawing the pages reach, charging `work`; `cache` keeps what each
    /// drawing's content uses, for a later walk while those streams are unchanged.
    pub(crate) fn reach(&self, work: &mut Work, cache: &mut Decoded) -> Result<Reach, TooComplex> {
        let pages = self.page_ids();
        let mut w = Walk {
            pdf: self,
            work,
            cache,
            reach: Reach::default(),
            seen: HashMap::new(),
            held: HashSet::new(),
            todo: Vec::new(),
        };
        w.reach.nodes.resize_with(pages.len(), Node::default);
        let no_resources = Rc::new(Dictionary::new());
        for (i, &page) in pages.iter().enumerate() {
            w.work.take(1)?;
            let mut cur = Some(page);
            while let Some(id) = cur.filter(|&id| !w.held.contains(&id)) {
                w.hold(id)?;
                cur = (self.doc.get_dictionary(id).ok())
                    .and_then(|d| d.get(b"Parent").ok())
                    .and_then(|o| o.as_reference().ok());
            }
            let res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
            let uses = (self.doc)
                .get_page_content_with_limit(page, crate::MAX_CONTENT)
                .ok()
                .and_then(|d| Uses::of_data(&d))
                .map(Rc::new);
            w.todo.push((i, uses, Rc::new(res)));
            for annot in self.all_annot_refs(page) {
                w.work.take(1)?;
                let ap = (self.doc.get_dictionary(annot).ok())
                    .and_then(|d| d.get(b"AP").ok())
                    .map(|o| self.resolved_dict(Some(o)));
                for (_, look) in ap.iter().flat_map(|ap| ap.iter()) {
                    // A drawing, or a dictionary of drawings by state.
                    let ids: Vec<ObjectId> = match self.doc.dereference(look) {
                        Ok((_, Object::Dictionary(states))) => (states.iter())
                            .filter_map(|(_, s)| s.as_reference().ok())
                            .collect(),
                        _ => look.as_reference().ok().into_iter().collect(),
                    };
                    for id in ids {
                        if let Some(n) = w.drawing(id, &no_resources)? {
                            w.reach.nodes[i].draws.push(n);
                            w.reach.appearances.push((i, n));
                        }
                    }
                }
            }
            w.run()?;
        }
        Ok(w.reach)
    }
}

impl Walk<'_> {
    fn hold(&mut self, id: ObjectId) -> Result<(), TooComplex> {
        if self.held.insert(id) {
            self.work.take(1)?;
            self.reach.holders.push(id);
        }
        Ok(())
    }

    /// The node of drawing `id`, drawn where the resources are `outer`; followed later when
    /// new. `None` when it isn't a stream.
    fn drawing(
        &mut self,
        id: ObjectId,
        outer: &Rc<Dictionary>,
    ) -> Result<Option<usize>, TooComplex> {
        self.work.take(1)?;
        let pdf = self.pdf;
        let Ok(s) = pdf.doc.get_object(id).and_then(|o| o.as_stream()) else {
            return Ok(None);
        };
        let own = s.dict.get(b"Resources").ok();
        let key = match own {
            Some(_) => (id, None),
            None => {
                self.work.take(outer.len() as u64)?;
                (id, Some(fingerprint(outer)))
            }
        };
        if let Some(&n) = self.seen.get(&key) {
            return Ok(Some(n));
        }
        let n = self.reach.nodes.len();
        self.reach.nodes.push(Node::default());
        self.seen.insert(key, n);
        let res = match own {
            Some(o) => {
                self.hold(id)?;
                Rc::new(pdf.resolved_dict(Some(o)))
            }
            None => outer.clone(),
        };
        let uses = pdf.uses_of(id, self.cache);
        self.todo.push((n, uses, res));
        Ok(Some(n))
    }

    /// Follow the drawings waiting, and those they draw.
    fn run(&mut self) -> Result<(), TooComplex> {
        let pdf = self.pdf;
        while let Some((n, uses, res)) = self.todo.pop() {
            let entries = |key: &[u8]| pdf.resolved_dict(res.get(key).ok());
            let listed;
            let u = match &uses {
                Some(u) => u.as_ref(),
                None => {
                    listed = Uses::listed(pdf, &res);
                    &listed
                }
            };
            let node = &mut self.reach.nodes[n];
            node.unread = uses.is_none();
            node.alt.extend(u.alt.iter().cloned());
            let properties = entries(b"Properties");
            for nm in &u.properties {
                self.work.take(1)?;
                let entry = properties.get(nm).ok();
                let d = pdf.resolved_dict(entry);
                let node = &mut self.reach.nodes[n];
                node.alt.extend(alt_texts(Some(&pdf.doc), &d));
                node.properties.extend(entry.cloned());
            }
            let mut draws = Vec::new();
            let xobjects = entries(b"XObject");
            for nm in &u.xobjects {
                self.work.take(1)?;
                let Some(id) = xobjects.get(nm).ok().and_then(|o| o.as_reference().ok()) else {
                    continue;
                };
                self.reach.drawn.insert(id);
                self.reach.nodes[n].xobjects.push(id);
                if is_form(pdf, id) {
                    draws.extend(self.drawing(id, &res)?);
                }
            }
            // A tiling pattern is a stream; a shading pattern, a dictionary, draws nothing.
            let patterns = entries(b"Pattern");
            for nm in &u.patterns {
                self.work.take(1)?;
                if let Some(id) = patterns.get(nm).ok().and_then(|o| o.as_reference().ok()) {
                    draws.extend(self.drawing(id, &res)?);
                }
            }
            let states = entries(b"ExtGState");
            for nm in &u.states {
                self.work.take(1)?;
                let gs = pdf.resolved_dict(states.get(nm).ok());
                let mask = pdf.resolved_dict(gs.get(b"SMask").ok());
                if let Some(id) = mask.get(b"G").ok().and_then(|o| o.as_reference().ok()) {
                    draws.extend(self.drawing(id, &res)?);
                }
            }
            let fonts = entries(b"Font");
            for nm in &u.fonts {
                self.work.take(1)?;
                let entry = fonts.get(nm).ok();
                let font = pdf.resolved_dict(entry);
                if font.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"Type3") {
                    continue;
                }
                // The glyphs draw with the font's resources, else with those of where the
                // font is used.
                let fres = match font.get(b"Resources") {
                    Ok(o) => {
                        if let Some(id) = entry.and_then(|e| e.as_reference().ok()) {
                            self.hold(id)?;
                        }
                        Rc::new(pdf.resolved_dict(Some(o)))
                    }
                    Err(_) => res.clone(),
                };
                let procs = pdf.resolved_dict(font.get(b"CharProcs").ok());
                for (_, g) in procs.iter() {
                    if let Ok(id) = g.as_reference() {
                        draws.extend(self.drawing(id, &fres)?);
                    }
                }
            }
            self.reach.nodes[n].draws.extend(draws);
        }
        Ok(())
    }
}

/// Whether object `id` is a form XObject.
pub(crate) fn is_form(pdf: &Pdf, id: ObjectId) -> bool {
    (pdf.doc.get_object(id).and_then(|o| o.as_stream()))
        .is_ok_and(|s| s.dict.get(b"Subtype").and_then(|o| o.as_name()).ok() == Some(b"Form"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Stream};

    /// A page drawing `Fm`, a form drawing itself without resources of its own, and `Fm2`,
    /// with its own, drawing itself through them.
    fn self_drawing() -> Pdf {
        let mut p = Pdf::new();
        p.insert_blank(0, 300.0, 200.0).unwrap();
        let form = |res: Option<Dictionary>, content: &[u8]| {
            let mut d = dictionary! { "Type" => "XObject", "Subtype" => "Form" };
            if let Some(res) = res {
                d.set("Resources", res);
            }
            Stream::new(d, content.to_vec())
        };
        let fm = p.doc.add_object(form(None, b"q /Fm Do Q"));
        let fm2 = p.doc.new_object_id();
        let own = dictionary! { "XObject" => dictionary! { "Me" => fm2 } };
        p.doc.objects.insert(fm2, form(Some(own), b"/Me Do").into());
        let page = p.page_ids()[0];
        let content = p
            .doc
            .add_object(Stream::new(Dictionary::new(), b"/Fm Do /Fm2 Do".to_vec()));
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", content);
        d.set(
            "Resources",
            dictionary! { "XObject" => dictionary! { "Fm" => fm, "Fm2" => fm2 } },
        );
        p
    }

    #[test]
    fn drawings_drawing_themselves_are_followed_once() {
        let p = self_drawing();
        let mut work = Work::new("walking", 1000);
        let reach = p.reach(&mut work, &mut Decoded::new()).unwrap();
        // The page, then each form once.
        assert_eq!(reach.nodes.len(), 3);
        assert_eq!(reach.drawn.len(), 2);
        let taken = work.taken();
        assert!(taken > 0);
        // The same walk with one step fewer is refused.
        let mut less = Work::new("walking", taken - 1);
        assert!(p.reach(&mut less, &mut Decoded::new()).is_err());
    }

    #[test]
    fn pages_drawing_a_node_are_found_through_what_they_draw() {
        let p = self_drawing();
        let reach = p.reach(&mut work(), &mut Decoded::new()).unwrap();
        let last = reach.nodes.len() - 1;
        assert_eq!(reach.pages_drawing(1, |i| i == last), vec![0]);
        assert_eq!(reach.pages_drawing(1, |_| false), Vec::<usize>::new());
    }
}
