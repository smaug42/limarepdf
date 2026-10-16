//! Flattening a form: drawing its widgets into the pages' content as they look now, then
//! removing the fields.

use crate::matrix::{self as mx, M};
use crate::{Pdf, Result};
use anyhow::bail;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::HashSet;

/// Annotation flag (`/F`) for an annotation that is not shown.
const HIDDEN: i64 = 1 << 1;
const MAX_DEPTH: usize = 32;

/// What flattening a page does with one of its annotations.
pub(crate) enum Flat {
    /// Leave it on the page.
    Keep,
    /// Draw it into the content and remove it.
    Draw,
    /// Remove it without drawing it.
    Drop,
}

/// Flattening a form draws its widgets and keeps everything else.
fn flat_widget(_: &Pdf, d: &Dictionary) -> Flat {
    if d.get(b"Subtype").and_then(|o| o.as_name()).ok() == Some(b"Widget") {
        Flat::Draw
    } else {
        Flat::Keep
    }
}

impl Pdf {
    /// Draw the form's widgets into the pages' content as they look now and remove them,
    /// so the values can no longer be changed. `pages` (zero-based) limits this to some
    /// pages; a field with widgets on other pages stays fillable there. Fields left with
    /// no widgets are removed, and the form itself once none are left. Hidden widgets are
    /// removed without being drawn. Returns how many widgets were drawn.
    pub fn flatten_form(&mut self, pages: Option<&[usize]>) -> Result<usize> {
        let ids = self.page_ids();
        let whole = pages.is_none();
        let pages = self.page_list(pages)?;
        // A field with a value but no drawing would otherwise vanish.
        self.refresh_appearances();
        // Appearances without resources of their own use the form's.
        let dr = self.acro_form().and_then(|f| f.get(b"DR").ok()).cloned();
        let mut removed = HashSet::new();
        let mut drawn = 0;
        for i in pages {
            drawn +=
                self.flatten_page(ids[i], flat_widget, dr.as_ref(), "MpeField", &mut removed)?;
        }
        if whole {
            self.remove_form()?;
        } else if !removed.is_empty() {
            self.prune_fields(&removed)?;
        }
        Ok(drawn)
    }

    /// The chosen pages (zero-based), checked, or every page.
    pub(crate) fn page_list(&self, pages: Option<&[usize]>) -> Result<Vec<usize>> {
        let n = self.page_count();
        match pages {
            Some(p) => {
                if let Some(&bad) = p.iter().find(|&&i| i >= n) {
                    bail!("page {} does not exist", bad + 1);
                }
                Ok(p.to_vec())
            }
            None => Ok((0..n).collect()),
        }
    }

    /// Draw and remove one page's annotations as `which` says, adding the removed ones to
    /// `removed`. Appearances without resources get `dr`; the drawings are named from
    /// `prefix` in the page's resources.
    pub(crate) fn flatten_page(
        &mut self,
        page: ObjectId,
        which: fn(&Pdf, &Dictionary) -> Flat,
        dr: Option<&Object>,
        prefix: &str,
        removed: &mut HashSet<ObjectId>,
    ) -> Result<usize> {
        let Some(annots) = self
            .doc
            .get_dictionary(page)?
            .get(b"Annots")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .cloned()
        else {
            return Ok(0);
        };
        let mut kept = Vec::new();
        let mut shown = Vec::new();
        for a in &annots {
            let found = a
                .as_reference()
                .ok()
                .and_then(|id| Some((id, self.doc.get_dictionary(id).ok()?)));
            let Some((id, d)) = found else {
                kept.push(a.clone());
                continue;
            };
            match which(self, d) {
                Flat::Keep => {
                    kept.push(a.clone());
                    continue;
                }
                Flat::Drop => {}
                Flat::Draw => shown.extend(self.placement(d)),
            }
            removed.insert(id);
        }
        if kept.len() == annots.len() {
            return Ok(0);
        }

        let mut res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
        let mut xobjects = self.resolved_dict(res.get(b"XObject").ok());
        let mut content = Vec::new();
        let mut n = 0;
        for (ap, m) in &shown {
            if let (Some(dr), Ok(s)) = (
                dr,
                self.doc.get_object_mut(*ap).and_then(|o| o.as_stream_mut()),
            ) {
                if !s.dict.has(b"Resources") {
                    s.dict.set("Resources", dr.clone());
                }
            }
            let name = loop {
                n += 1;
                let name = format!("{prefix}{n}");
                if !xobjects.has(name.as_bytes()) {
                    break name;
                }
            };
            xobjects.set(name.as_str(), *ap);
            let m = m.map(|x| format!("{x:.4}")).join(" ");
            content.extend(format!("q {m} cm /{name} Do Q\n").bytes());
        }
        res.set("XObject", xobjects);
        let pd = self.doc.get_dictionary_mut(page)?;
        if kept.is_empty() {
            pd.remove(b"Annots");
        } else {
            pd.set("Annots", kept);
        }
        if !shown.is_empty() {
            pd.set("Resources", res);
            self.wrap_contents(page, None, Some(content))?;
        }
        Ok(shown.len())
    }

    /// A shown widget's current appearance, and the matrix that puts it on the page: the
    /// appearance's box, after its own `/Matrix`, is fitted to the widget's `/Rect`.
    fn placement(&self, d: &Dictionary) -> Option<(ObjectId, M)> {
        let flags = d
            .get(b"F")
            .ok()
            .and_then(|o| self.int_value(o))
            .unwrap_or(0);
        if flags & HIDDEN != 0 {
            return None;
        }
        let rect = self.rect(d.get(b"Rect").ok()?)?;
        let normal = self.dict_of(d.get(b"AP").ok()?)?.get(b"N").ok()?;
        let id = match self.doc.dereference(normal).ok()? {
            (Some(id), Object::Stream(_)) => id,
            (_, Object::Dictionary(states)) => {
                let state = d.get(b"AS").ok()?.as_name().ok()?;
                states.get(state).ok()?.as_reference().ok()?
            }
            _ => return None,
        };
        let s = self.doc.get_object(id).ok()?.as_stream().ok()?;
        let bbox = self.rect(s.dict.get(b"BBox").ok()?)?;
        let matrix: M = s
            .dict
            .get(b"Matrix")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .and_then(|a| {
                let v: Vec<f32> = a.iter().filter_map(|x| self.num(x)).collect();
                v.try_into().ok()
            })
            .unwrap_or(mx::ID);
        let b = mx::transform_rect(matrix, bbox);
        let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
        if bw < 1e-3 || bh < 1e-3 {
            return None;
        }
        let (sx, sy) = ((rect[2] - rect[0]) / bw, (rect[3] - rect[1]) / bh);
        Some((
            id,
            [sx, 0.0, 0.0, sy, rect[0] - b[0] * sx, rect[1] - b[1] * sy],
        ))
    }

    /// Take the removed widgets out of the field tree, then the fields left without
    /// widgets, then the form when it has no fields left.
    pub(crate) fn prune_fields(&mut self, removed: &HashSet<ObjectId>) -> Result<()> {
        let Some(form) = self.acro_form() else {
            return Ok(());
        };
        let list = |key: &[u8]| -> Option<Vec<Object>> {
            let o = form.get(key).ok()?;
            Some(self.doc.dereference(o).ok()?.1.as_array().ok()?.clone())
        };
        let fields = list(b"Fields").unwrap_or_default();
        let order = list(b"CO");
        let mut gone = removed.clone();
        let mut seen = HashSet::new();
        let fields: Vec<Object> = fields
            .into_iter()
            .filter(|f| {
                f.as_reference()
                    .map_or(true, |f| self.prune_field(f, &mut gone, &mut seen, 0))
            })
            .collect();
        if fields.is_empty() {
            return self.remove_form();
        }
        let form = self.acro_form_id()?;
        let d = self.doc.get_dictionary_mut(form)?;
        d.set("Fields", fields);
        if let Some(order) = order {
            let order: Vec<Object> = order
                .into_iter()
                .filter(|o| o.as_reference().map_or(true, |o| !gone.contains(&o)))
                .collect();
            d.set("CO", order);
        }
        self.form_changed();
        Ok(())
    }

    fn remove_form(&mut self) -> Result<()> {
        let catalog = self.doc.trailer.get(b"Root")?.as_reference()?;
        self.doc.get_dictionary_mut(catalog)?.remove(b"AcroForm");
        Ok(())
    }

    /// Prune one field's kids; false when the field itself goes.
    fn prune_field(
        &mut self,
        id: ObjectId,
        gone: &mut HashSet<ObjectId>,
        seen: &mut HashSet<ObjectId>,
        depth: usize,
    ) -> bool {
        if gone.contains(&id) {
            return false;
        }
        if depth > MAX_DEPTH || !seen.insert(id) {
            return true;
        }
        let Some(kids) = self
            .doc
            .get_dictionary(id)
            .ok()
            .and_then(|d| d.get(b"Kids").ok())
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .cloned()
        else {
            return true;
        };
        let n = kids.len();
        let kids: Vec<Object> = kids
            .into_iter()
            .filter(|k| {
                k.as_reference()
                    .map_or(true, |k| self.prune_field(k, gone, seen, depth + 1))
            })
            .collect();
        if kids.is_empty() {
            gone.insert(id);
            return false;
        }
        if kids.len() < n {
            if let Ok(d) = self.doc.get_dictionary_mut(id) {
                d.set("Kids", kids);
            }
        }
        true
    }
}
