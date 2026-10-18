//! Removing hidden information before a document is shared: what isn't on any page but
//! travels with the file (metadata, scripts, attachments, the content of hidden layers,
//! private application data), and optionally comments, form values, bookmarks and what
//! lies outside the visible page area.

use crate::{Pdf, Result};
use anyhow::anyhow;
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::HashSet;

/// A kind of hidden information.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hidden {
    Metadata,
    Scripts,
    Attachments,
    HiddenLayers,
    PrivateData,
    Comments,
    FormValues,
    Bookmarks,
    OutsideCrop,
}

impl Hidden {
    pub const ALL: [Hidden; 9] = [
        Hidden::Metadata,
        Hidden::Scripts,
        Hidden::Attachments,
        Hidden::HiddenLayers,
        Hidden::PrivateData,
        Hidden::Comments,
        Hidden::FormValues,
        Hidden::Bookmarks,
        Hidden::OutsideCrop,
    ];

    /// The name the command line takes.
    pub fn name(self) -> &'static str {
        match self {
            Hidden::Metadata => "metadata",
            Hidden::Scripts => "scripts",
            Hidden::Attachments => "attachments",
            Hidden::HiddenLayers => "hidden-layers",
            Hidden::PrivateData => "private-data",
            Hidden::Comments => "comments",
            Hidden::FormValues => "form-values",
            Hidden::Bookmarks => "bookmarks",
            Hidden::OutsideCrop => "outside-crop",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|h| h.name() == s.trim())
    }

    /// A label for people.
    pub fn label(self) -> &'static str {
        match self {
            Hidden::Metadata => "Document information and XMP metadata",
            Hidden::Scripts => "JavaScript and actions that run by themselves",
            Hidden::Attachments => "Attached files",
            Hidden::HiddenLayers => "Hidden layers and their content",
            Hidden::PrivateData => "Thumbnails and private application data",
            Hidden::Comments => "Comments and markup",
            Hidden::FormValues => "Form field values",
            Hidden::Bookmarks => "Bookmarks",
            Hidden::OutsideCrop => "Content outside the visible page area",
        }
    }

    /// Whether it is removed unless asked not to. Comments, form values and bookmarks are
    /// often wanted; cutting outside the crop box changes the pages.
    pub fn default_on(self) -> bool {
        matches!(
            self,
            Hidden::Metadata
                | Hidden::Scripts
                | Hidden::Attachments
                | Hidden::HiddenLayers
                | Hidden::PrivateData
        )
    }
}

/// How many things of each kind [`Pdf::remove_hidden`] removed (kinds with none are left
/// out), in [`Hidden::ALL`] order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HiddenReport {
    pub removed: Vec<(Hidden, usize)>,
}

impl HiddenReport {
    pub fn count(&self, h: Hidden) -> usize {
        self.removed
            .iter()
            .find(|(k, _)| *k == h)
            .map_or(0, |(_, n)| *n)
    }
}

/// Actions that run something, send data or open other files by themselves.
const ACTIVE: [&[u8]; 6] = [
    b"JavaScript",
    b"Launch",
    b"SubmitForm",
    b"ImportData",
    b"Rendition",
    b"GoToE",
];

/// Deepest nesting of form XObjects looked into for hidden layers.
const MAX_NESTING: usize = 8;

impl Pdf {
    /// What [`Pdf::remove_hidden`] would remove of every kind, worked out on a copy.
    pub fn find_hidden(&self) -> Result<HiddenReport> {
        self.clone().remove_hidden(&Hidden::ALL)
    }

    /// Remove the kinds of hidden information in `what`. Unused objects go when the
    /// document is saved, which rewrites the whole file, so nothing removed is kept in it.
    pub fn remove_hidden(&mut self, what: &[Hidden]) -> Result<HiddenReport> {
        let mut report = HiddenReport::default();
        for h in Hidden::ALL {
            if !what.contains(&h) {
                continue;
            }
            let n = match h {
                Hidden::OutsideCrop => self.cut_outside_crop()?,
                Hidden::HiddenLayers => self.drop_hidden_layers()?,
                Hidden::Comments => {
                    self.remove_annotations(|a| !matches!(a.kind, crate::AnnotKind::Redact))?
                }
                Hidden::Attachments => self.drop_attachments()?,
                Hidden::Scripts => self.drop_scripts(),
                Hidden::Metadata => self.drop_metadata(),
                Hidden::PrivateData => self.drop_private_data(),
                Hidden::Bookmarks => {
                    let n = self.outline().len();
                    if let Ok(c) = self.doc.catalog_mut() {
                        c.remove(b"Outlines");
                    }
                    n
                }
                Hidden::FormValues => {
                    let n = self
                        .form_fields()
                        .iter()
                        .filter(|f| match &f.value {
                            crate::FieldValue::Text(t) => !t.is_empty(),
                            crate::FieldValue::Check(c) => *c,
                            crate::FieldValue::Choice(c) => !c.is_empty(),
                            crate::FieldValue::None => false,
                        })
                        .count();
                    if n > 0 {
                        self.reset_form()?;
                    }
                    n
                }
            };
            if n > 0 {
                report.removed.push((h, n));
            }
        }
        Ok(report)
    }

    /// Remove `keys` from every dictionary in the file. Returns how many were removed.
    fn drop_keys_everywhere(&mut self, keys: &[&[u8]]) -> usize {
        let mut n = 0;
        for obj in self.doc.objects.values_mut() {
            let d = match obj {
                Object::Dictionary(d) => d,
                Object::Stream(s) => &mut s.dict,
                _ => continue,
            };
            for k in keys {
                if d.remove(k).is_some() {
                    n += 1;
                }
            }
        }
        n
    }

    /// Remove `keys` from the catalog's `/Names` dictionary. Returns how many were there.
    fn drop_names(&mut self, keys: &[&[u8]]) -> usize {
        let names = self
            .doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"Names").ok())
            .cloned();
        let d = match names {
            Some(Object::Reference(id)) => self.doc.get_dictionary_mut(id).ok(),
            Some(Object::Dictionary(_)) => self
                .doc
                .catalog_mut()
                .ok()
                .and_then(|c| c.get_mut(b"Names").ok())
                .and_then(|o| o.as_dict_mut().ok()),
            _ => None,
        };
        let Some(d) = d else { return 0 };
        keys.iter().filter(|k| d.remove(k).is_some()).count()
    }

    fn drop_metadata(&mut self) -> usize {
        let info = usize::from(self.doc.trailer.remove(b"Info").is_some());
        info + self.drop_keys_everywhere(&[b"Metadata"])
    }

    fn drop_private_data(&mut self) -> usize {
        let mut n = self.drop_keys_everywhere(&[b"PieceInfo", b"Thumb"]);
        if let Ok(c) = self.doc.catalog_mut() {
            n += usize::from(c.remove(b"SpiderInfo").is_some());
        }
        n + self.drop_names(&[b"IDS", b"URLS"])
    }

    fn drop_attachments(&mut self) -> Result<usize> {
        let mut n = self.drop_names(&[b"EmbeddedFiles"]);
        if let Ok(c) = self.doc.catalog_mut() {
            n += usize::from(c.remove(b"Collection").is_some());
        }
        n += self.drop_keys_everywhere(&[b"AF"]);
        let files: HashSet<ObjectId> = self
            .annotations()
            .into_iter()
            .filter(|a| matches!(&a.kind, crate::AnnotKind::Other(s) if s == "FileAttachment"))
            .map(|a| a.id)
            .collect();
        if !files.is_empty() {
            n += self.drop_annotations(files)?;
        }
        Ok(n)
    }

    fn drop_scripts(&mut self) -> usize {
        let active = |d: &Dictionary| {
            d.get(b"S")
                .and_then(|o| o.as_name())
                .is_ok_and(|s| ACTIVE.contains(&s))
        };
        let bad: HashSet<ObjectId> = self
            .doc
            .objects
            .iter()
            .filter(|(_, o)| o.as_dict().is_ok_and(active))
            .map(|(id, _)| *id)
            .collect();
        let is_bad = |o: &Object| match o {
            Object::Reference(id) => bad.contains(id),
            Object::Dictionary(d) => active(d),
            _ => false,
        };
        let mut n = self.drop_names(&[b"JavaScript"]);
        let xfa_only = self.xfa_only();
        for obj in self.doc.objects.values_mut() {
            let d = match obj {
                Object::Dictionary(d) => d,
                Object::Stream(s) => &mut s.dict,
                _ => continue,
            };
            // Additional actions: on opening, closing, printing, keystrokes, focus….
            n += usize::from(d.remove(b"AA").is_some());
            for key in [&b"A"[..], b"OpenAction"] {
                if d.get(key).is_ok_and(is_bad) {
                    d.remove(key);
                    n += 1;
                }
            }
            match d.get(b"Next").ok().cloned() {
                Some(Object::Array(a)) => {
                    let kept: Vec<Object> = a.iter().filter(|o| !is_bad(o)).cloned().collect();
                    if kept.len() < a.len() {
                        n += a.len() - kept.len();
                        d.set("Next", kept);
                    }
                }
                Some(o) if is_bad(&o) => {
                    d.remove(b"Next");
                    n += 1;
                }
                _ => {}
            }
            // A hybrid form's XFA part holds scripts of its own; the ordinary fields stay.
            if !xfa_only && d.has(b"Fields") && d.remove(b"XFA").is_some() {
                n += 1;
            }
        }
        if let Ok(c) = self.doc.catalog_mut() {
            n += usize::from(c.remove(b"AA").is_some());
        }
        n
    }

    /// Delete what lies outside each page's crop box, then make the crop box the page.
    /// Returns how many pages were cut.
    fn cut_outside_crop(&mut self) -> Result<usize> {
        let mut marks = Vec::new();
        let mut cut = Vec::new();
        for (i, page) in self.page_ids().into_iter().enumerate() {
            let (media, crop) = (self.media_box(page), self.page_box(page));
            let inside = crop[0] > media[0] + 0.5
                || crop[1] > media[1] + 0.5
                || crop[2] < media[2] - 0.5
                || crop[3] < media[3] - 0.5;
            if !inside {
                continue;
            }
            // Around the shown page (displayed points), far enough to cover the media box.
            let (_, w, h) = self.vis_down(page);
            let m = (media[2] - media[0]).abs() + (media[3] - media[1]).abs();
            for r in [
                [-m, -m, w + m, 0.0],
                [-m, h, w + m, h + m],
                [-m, 0.0, 0.0, h],
                [w, 0.0, w + m, h],
            ] {
                marks.push((i, r));
            }
            cut.push((page, crop));
        }
        if marks.is_empty() {
            return Ok(0);
        }
        self.redact(&marks)?;
        for (page, crop) in &cut {
            let d = self.doc.get_dictionary_mut(*page)?;
            d.set(
                "MediaBox",
                crop.iter().map(|&v| Object::Real(v)).collect::<Vec<_>>(),
            );
            for k in [&b"CropBox"[..], b"TrimBox", b"BleedBox", b"ArtBox"] {
                d.remove(k);
            }
        }
        Ok(cut.len())
    }

    /// Delete the content of the layers (optional content groups) that are switched off,
    /// and take those layers out of the layer list. Returns how many layers went.
    fn drop_hidden_layers(&mut self) -> Result<usize> {
        let Some(props) = self
            .doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"OCProperties").ok())
            .map(|o| self.resolved_dict(Some(o)))
        else {
            return Ok(0);
        };
        let refs = |o: Option<&Object>| -> Vec<ObjectId> {
            o.and_then(|o| self.doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_array().ok())
                .map(|a| a.iter().filter_map(|x| x.as_reference().ok()).collect())
                .unwrap_or_default()
        };
        let all = refs(props.get(b"OCGs").ok());
        let d = self.resolved_dict(props.get(b"D").ok());
        let base_off = d.get(b"BaseState").and_then(|o| o.as_name()).ok() == Some(b"OFF");
        let hidden: HashSet<ObjectId> = if base_off {
            let on: HashSet<ObjectId> = refs(d.get(b"ON").ok()).into_iter().collect();
            all.iter().copied().filter(|g| !on.contains(g)).collect()
        } else {
            refs(d.get(b"OFF").ok()).into_iter().collect()
        };
        if hidden.is_empty() {
            return Ok(0);
        }

        // Page content, then every form XObject the pages draw (each once: a form looks
        // the same wherever it is drawn, as layer states are the document's).
        let mut forms_seen = HashSet::new();
        for page in self.page_ids() {
            let res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
            let data = self
                .doc
                .get_page_content_with_limit(page, crate::MAX_CONTENT)?;
            if let Some((ops, inline)) = self.without_hidden(&data, &res, &hidden)? {
                self.commit(page, ops, 0, &inline)?;
            }
            self.hidden_in_forms(&res, &hidden, &mut forms_seen, 0)?;
        }
        // Pictures and forms in a hidden layer are no longer drawn: out of the resources.
        let layered: HashSet<ObjectId> = self
            .doc
            .objects
            .iter()
            .filter(|(_, o)| {
                o.as_stream()
                    .ok()
                    .and_then(|s| s.dict.get(b"OC").ok())
                    .is_some_and(|oc| self.oc_hidden(oc, &hidden))
            })
            .map(|(id, _)| *id)
            .collect();
        self.drop_unused_xobjects(&layered)?;
        // Annotations in a hidden layer.
        let mut gone = HashSet::new();
        for page in self.page_ids() {
            for id in self.annot_refs(page) {
                let oc = self
                    .doc
                    .get_dictionary(id)
                    .ok()
                    .and_then(|d| d.get(b"OC").ok());
                if oc.is_some_and(|o| self.oc_hidden(o, &hidden)) {
                    gone.insert(id);
                }
            }
        }
        if !gone.is_empty() {
            self.drop_annotations(gone)?;
        }

        // The layer list: hidden layers out, the rest on.
        let keep = |o: &Object| o.as_reference().map_or(true, |id| !hidden.contains(&id));
        fn prune(a: &[Object], keep: &dyn Fn(&Object) -> bool) -> Vec<Object> {
            a.iter()
                .filter(|o| keep(o))
                .map(|o| match o {
                    Object::Array(inner) => Object::Array(prune(inner, keep)),
                    o => o.clone(),
                })
                .collect()
        }
        let mut props = props;
        if let Ok(Object::Array(a)) = props.get(b"OCGs").cloned() {
            props.set("OCGs", prune(&a, &keep));
        }
        let mut d = d;
        for k in [&b"OFF"[..], b"BaseState", b"AS", b"Locked", b"RBGroups"] {
            d.remove(k);
        }
        for k in [&b"ON"[..], b"Order"] {
            if let Ok(Object::Array(a)) = d.get(k).cloned() {
                d.set(k.to_vec(), prune(&a, &keep));
            }
        }
        props.set("D", d);
        props.remove(b"Configs");
        self.doc
            .catalog_mut()
            .map_err(|e| anyhow!("{e}"))?
            .set("OCProperties", props);
        Ok(hidden.len())
    }

    /// Whether optional content `oc` (a layer, or a membership dictionary combining
    /// layers) is hidden when `hidden` layers are off.
    fn oc_hidden(&self, oc: &Object, hidden: &HashSet<ObjectId>) -> bool {
        if let Ok(id) = oc.as_reference() {
            if hidden.contains(&id) {
                return true;
            }
        }
        let Some(d) = self
            .doc
            .dereference(oc)
            .ok()
            .and_then(|(_, o)| o.as_dict().ok())
        else {
            return false;
        };
        if d.get(b"Type").and_then(|o| o.as_name()).ok() != Some(b"OCMD") {
            return false;
        }
        let groups: Vec<bool> = match d.get(b"OCGs") {
            Ok(Object::Reference(id)) => vec![hidden.contains(id)],
            Ok(Object::Array(a)) => a
                .iter()
                .filter_map(|o| o.as_reference().ok())
                .map(|id| hidden.contains(&id))
                .collect(),
            _ => return false,
        };
        if groups.is_empty() {
            return false;
        }
        let policy = d.get(b"P").and_then(|o| o.as_name()).unwrap_or(b"AnyOn");
        let visible = match policy {
            b"AllOn" => groups.iter().all(|h| !h),
            b"AnyOff" => groups.iter().any(|h| *h),
            b"AllOff" => groups.iter().all(|h| *h),
            _ => groups.iter().any(|h| !h),
        };
        !visible
    }

    /// A content stream without what is in hidden layers: marked content tagged with a
    /// hidden layer, and pictures and forms that belong to one. `None` when nothing was.
    #[allow(clippy::type_complexity)]
    fn without_hidden(
        &self,
        data: &[u8],
        res: &Dictionary,
        hidden: &HashSet<ObjectId>,
    ) -> Result<Option<(Vec<Operation>, Vec<Vec<u8>>)>> {
        let (data, inline) = crate::inline::extract(data).map_err(|e| anyhow!("{e}"))?;
        let ops = Content::decode(&data)
            .map_err(|e| anyhow!("cannot parse the page content: {e}"))?
            .operations;
        let props = self.resolved_dict(res.get(b"Properties").ok());
        let xobjs = self.resolved_dict(res.get(b"XObject").ok());
        let mut out = Vec::with_capacity(ops.len());
        let mut changed = false;
        // While skipping: the marked-content depth, and q minus Q skipped.
        let mut skip: Option<(usize, i64)> = None;
        for o in ops {
            if let Some((depth, qs)) = skip.as_mut() {
                match o.operator.as_str() {
                    "BDC" | "BMC" => *depth += 1,
                    "EMC" => *depth -= 1,
                    "q" => *qs += 1,
                    "Q" => *qs -= 1,
                    _ => {}
                }
                if *depth == 0 {
                    // Keep the graphics state stack as it was.
                    let (q, n) = if *qs > 0 { ("q", *qs) } else { ("Q", -*qs) };
                    out.extend((0..n).map(|_| Operation::new(q, vec![])));
                    skip = None;
                }
                continue;
            }
            let oc_layer = |prop: Option<&Object>| -> bool {
                match prop {
                    Some(Object::Name(n)) => props.get(n).is_ok_and(|o| self.oc_hidden(o, hidden)),
                    Some(o) => self.oc_hidden(o, hidden),
                    None => false,
                }
            };
            match o.operator.as_str() {
                "BDC"
                    if o.operands.first().and_then(|t| t.as_name().ok()) == Some(b"OC")
                        && oc_layer(o.operands.get(1)) =>
                {
                    skip = Some((1, 0));
                    changed = true;
                    continue;
                }
                "Do" => {
                    let oc = o
                        .operands
                        .first()
                        .and_then(|n| n.as_name().ok())
                        .and_then(|n| xobjs.get(n).ok())
                        .and_then(|e| self.doc.dereference(e).ok())
                        .and_then(|(_, s)| s.as_stream().ok())
                        .and_then(|s| s.dict.get(b"OC").ok());
                    if oc.is_some_and(|oc| self.oc_hidden(oc, hidden)) {
                        changed = true;
                        continue;
                    }
                }
                _ => {}
            }
            out.push(o);
        }
        Ok(changed.then_some((out, inline)))
    }

    /// Take hidden layers out of the form XObjects drawn with resources `res`, and the
    /// forms inside them.
    fn hidden_in_forms(
        &mut self,
        res: &Dictionary,
        hidden: &HashSet<ObjectId>,
        seen: &mut HashSet<ObjectId>,
        depth: usize,
    ) -> Result<()> {
        if depth >= MAX_NESTING {
            return Ok(());
        }
        let xobjs = self.resolved_dict(res.get(b"XObject").ok());
        for (_, e) in xobjs.iter() {
            let Ok(id) = e.as_reference() else { continue };
            if !seen.insert(id) {
                continue;
            }
            let Ok(s) = self.doc.get_object(id).and_then(|o| o.as_stream()) else {
                continue;
            };
            if s.dict.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"Form") {
                continue;
            }
            let own = s
                .dict
                .get(b"Resources")
                .ok()
                .map(|o| self.resolved_dict(Some(o)));
            let fres = own.unwrap_or_else(|| res.clone());
            let Ok(data) = s.get_plain_content_with_limit(crate::MAX_CONTENT) else {
                continue;
            };
            if let Some((ops, inline)) = self.without_hidden(&data, &fres, hidden)? {
                let bytes = Content { operations: ops }
                    .encode()
                    .map_err(|e| anyhow!("cannot encode a form: {e}"))?;
                let bytes = crate::inline::restore(bytes, &inline);
                if let Ok(s) = self.doc.get_object_mut(id).and_then(|o| o.as_stream_mut()) {
                    s.dict.remove(b"Filter");
                    s.dict.remove(b"DecodeParms");
                    s.set_content(bytes);
                    let _ = s.compress();
                }
            }
            self.hidden_in_forms(&fres, hidden, seen, depth + 1)?;
        }
        Ok(())
    }
}
