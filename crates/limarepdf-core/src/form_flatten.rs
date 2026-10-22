//! Flattening a form: drawing its widgets into the pages' content as they look now, then
//! removing the fields.

use crate::forms::FieldValue;
use crate::matrix::{self as mx, M};
use crate::{quoted_list, Pdf, Result};
use anyhow::bail;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::HashSet;

/// Annotation flag (`/F`) for an annotation that is not shown.
const HIDDEN: i64 = 1 << 1;
/// Annotation flag (`/F`) for an annotation that is printed.
const PRINT: i64 = 1 << 2;
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

/// Printing draws the widgets meant for print, drops the others (buttons such as Print
/// Form usually), and keeps everything else.
fn print_widget(pdf: &Pdf, d: &Dictionary) -> Flat {
    if d.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"Widget") {
        return Flat::Keep;
    }
    let flags = d.get(b"F").ok().and_then(|o| pdf.int_value(o)).unwrap_or(0);
    if flags & PRINT != 0 {
        Flat::Draw
    } else {
        Flat::Drop
    }
}

impl Pdf {
    /// Draw the form's widgets into the pages' content as they look now and remove them,
    /// so the values can no longer be changed. `pages` (zero-based) limits this to some
    /// pages; a field with widgets on other pages stays fillable there. Fields left with
    /// no widgets are removed, and the form itself once none are left. Hidden widgets are
    /// removed without being drawn. Refuses, with nothing changed, when a shown field's
    /// value has no drawing and LimarePDF can't draw it, as it would be lost. Returns how
    /// many widgets were drawn.
    pub fn flatten_form(&mut self, pages: Option<&[usize]>) -> Result<usize> {
        self.flatten_widgets(pages, flat_widget, true)
    }

    /// Flatten the whole form for printing: widgets with the Print flag are drawn, the
    /// others removed. Some CUPS setups remove a form's fields without drawing them, so
    /// the copy sent to the printer has none left. Values LimarePDF can't draw are left
    /// out ([`Pdf::unprintable_fields`] names them first).
    pub(crate) fn flatten_form_for_print(&mut self) -> Result<usize> {
        self.flatten_widgets(None, print_widget, false)
    }

    /// The fields on `pages` (zero-based) whose values won't show on paper: printed
    /// widgets showing a value that has no drawing, which LimarePDF can't draw either.
    /// Changes nothing.
    pub fn unprintable_fields(&self, pages: &[usize]) -> Vec<String> {
        if self.undrawn(pages, print_widget).is_empty() {
            return Vec::new();
        }
        let mut copy = self.clone();
        copy.refresh_appearances();
        copy.undrawn(pages, print_widget)
    }

    /// `refuse`: when a value would be lost, refuse with nothing changed rather than
    /// leave it out.
    fn flatten_widgets(
        &mut self,
        pages: Option<&[usize]>,
        which: fn(&Pdf, &Dictionary) -> Flat,
        refuse: bool,
    ) -> Result<usize> {
        let ids = self.page_ids();
        let whole = pages.is_none();
        let pages = self.page_list(pages)?;
        // A field with a value but no drawing would otherwise vanish. Drawing can't add one
        // for every value, so when one is missing it is tried on a copy first.
        if refuse && !self.undrawn(&pages, which).is_empty() {
            let mut copy = self.clone();
            copy.refresh_appearances();
            let lost = copy.undrawn(&pages, which);
            if !lost.is_empty() {
                let (what, them) = match lost.len() {
                    1 => ("the value of the field", "it"),
                    _ => ("the values of these fields:", "them"),
                };
                bail!(
                    "LimarePDF can't draw {what} {}, so flattening would lose {them}. Nothing \
                     was changed.",
                    quoted_list(&lost)
                );
            }
            *self = copy;
        } else {
            self.refresh_appearances();
        }
        // Appearances without resources of their own use the form's.
        let dr = self.acro_form().and_then(|f| f.get(b"DR").ok()).cloned();
        let mut removed = HashSet::new();
        let mut drawn = 0;
        for i in pages {
            drawn += self.flatten_page(ids[i], which, dr.as_ref(), "LimareField", &mut removed)?;
        }
        if whole {
            self.remove_form()?;
        } else if !removed.is_empty() {
            self.prune_fields(&removed, false)?;
        }
        Ok(drawn)
    }

    /// The fields with a widget on `pages` that `which` draws, that is shown and shows a
    /// value, and that has no drawing: flattening it would lose the value. In the field
    /// tree's order.
    fn undrawn(&self, pages: &[usize], which: fn(&Pdf, &Dictionary) -> Flat) -> Vec<String> {
        let ids = self.page_ids();
        let mut drawn = HashSet::new();
        for page in pages.iter().filter_map(|&i| ids.get(i)) {
            for id in self.all_annot_refs(*page) {
                if let Ok(d) = self.doc.get_dictionary(id) {
                    if matches!(which(self, d), Flat::Draw) {
                        drawn.insert(id);
                    }
                }
            }
        }
        let mut out = Vec::new();
        for (field, node) in self.form_nodes() {
            let lost = node.widgets.iter().any(|w| {
                drawn.contains(w)
                    && self.doc.get_dictionary(*w).is_ok_and(|d| {
                        self.shows_value(&field.value, d) && self.drawing(d).is_none()
                    })
            });
            if lost && !out.contains(&field.name) {
                out.push(field.name);
            }
        }
        out
    }

    /// Whether widget `d` of a field holding `value` is shown and shows that value: not
    /// hidden, of some size, and for checkboxes and radio buttons, not in its off state.
    fn shows_value(&self, value: &FieldValue, d: &Dictionary) -> bool {
        let flags = d
            .get(b"F")
            .ok()
            .and_then(|o| self.int_value(o))
            .unwrap_or(0);
        let sized = d
            .get(b"Rect")
            .ok()
            .and_then(|o| self.rect(o))
            .is_some_and(|r| (r[2] - r[0]).abs() > 1e-3 && (r[3] - r[1]).abs() > 1e-3);
        let off = d.get(b"AS").and_then(|o| o.as_name()).ok() == Some(b"Off");
        let filled = match value {
            FieldValue::Text(t) => !t.is_empty(),
            FieldValue::Check(on) => *on && !off,
            FieldValue::Choice(c) => !c.is_empty() && !off,
            FieldValue::None => false,
        };
        flags & HIDDEN == 0 && sized && filled
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

    /// The drawing of an annotation as it looks now: its normal appearance, or for one with
    /// states (checkboxes, radio buttons), the current state's.
    fn drawing(&self, d: &Dictionary) -> Option<ObjectId> {
        let normal = self.dict_of(d.get(b"AP").ok()?)?.get(b"N").ok()?;
        let id = match self.doc.dereference(normal).ok()? {
            (Some(id), Object::Stream(_)) => id,
            (_, Object::Dictionary(states)) => {
                let state = d.get(b"AS").ok()?.as_name().ok()?;
                states.get(state).ok()?.as_reference().ok()?
            }
            _ => return None,
        };
        self.doc.get_object(id).ok()?.as_stream().ok()?;
        Some(id)
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
        let id = self.drawing(d)?;
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
    /// widgets, then the form when it has no fields left. Fields nested more than
    /// [`MAX_DEPTH`] deep are kept, or with `strict` (for removing information, where one
    /// kept could hold the value of a removed widget) make it fail.
    pub(crate) fn prune_fields(&mut self, removed: &HashSet<ObjectId>, strict: bool) -> Result<()> {
        let Some(form) = self.acro_form() else {
            return Ok(());
        };
        let list = |key: &[u8]| -> Option<Vec<Object>> {
            let o = form.get(key).ok()?;
            Some(self.doc.dereference(o).ok()?.1.as_array().ok()?.clone())
        };
        let all = list(b"Fields").unwrap_or_default();
        let order = list(b"CO");
        let mut gone = removed.clone();
        let mut seen = HashSet::new();
        let mut fields = Vec::new();
        for f in all {
            let keep = match f.as_reference() {
                Ok(id) => self.prune_field(id, &mut gone, &mut seen, 0, strict)?,
                Err(_) => true,
            };
            if keep {
                fields.push(f);
            }
        }
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

    /// Prune one field's kids; false when the field itself goes. With `strict`, fails when
    /// the kids are nested too deep to look at.
    fn prune_field(
        &mut self,
        id: ObjectId,
        gone: &mut HashSet<ObjectId>,
        seen: &mut HashSet<ObjectId>,
        depth: usize,
        strict: bool,
    ) -> Result<bool> {
        if gone.contains(&id) {
            return Ok(false);
        }
        if !seen.insert(id) {
            return Ok(true);
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
            return Ok(true);
        };
        if depth > MAX_DEPTH {
            if strict {
                bail!("the form's fields are nested more than {MAX_DEPTH} deep");
            }
            return Ok(true);
        }
        let n = kids.len();
        let mut kept = Vec::new();
        for k in kids {
            let keep = match k.as_reference() {
                Ok(k) => self.prune_field(k, gone, seen, depth + 1, strict)?,
                Err(_) => true,
            };
            if keep {
                kept.push(k);
            }
        }
        if kept.is_empty() {
            gone.insert(id);
            return Ok(false);
        }
        if kept.len() < n {
            if let Ok(d) = self.doc.get_dictionary_mut(id) {
                d.set("Kids", kept);
            }
        }
        Ok(true)
    }
}
