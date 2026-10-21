//! Removing hidden information before a document is shared: what isn't on any page but
//! travels with the file (metadata, scripts, attachments, the content of hidden layers,
//! private application data), and optionally comments, form values, bookmarks and what
//! lies outside the visible page area.

use crate::inline::Inline;
use crate::{Pdf, Result};
use anyhow::anyhow;
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::{HashMap, HashSet};

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
    /// Compressed data looked into for hidden layers was complete but had a wrong checksum;
    /// it was read in full, as other readers do.
    pub faults_overlooked: bool,
}

impl HiddenReport {
    pub fn count(&self, h: Hidden) -> usize {
        self.removed
            .iter()
            .find(|(k, _)| *k == h)
            .map_or(0, |(_, n)| *n)
    }

    /// What to tell people about the removal besides the counts, if anything.
    pub fn note(&self) -> Option<&'static str> {
        self.faults_overlooked.then_some(
            "Some of this document's data had a small fault in it (other PDF readers overlook \
             it too). It was all read, and nothing was skipped.",
        )
    }
}

/// [`Pdf::remove_hidden`]'s refusal when compressed data that can hold hidden layers is cut
/// off or damaged part way, so not all of it could be checked. The message is the app's.
#[derive(Debug, thiserror::Error)]
#[error(
    "Part of this document is damaged, so LimarePDF can't check all of it for hidden layers. \
     Nothing was changed. To remove the other kinds of information, untick Hidden layers and \
     try again."
)]
pub struct LayersDamaged;

/// Actions that run something, send data or open other files by themselves.
const ACTIVE: [&[u8]; 6] = [
    b"JavaScript",
    b"Launch",
    b"SubmitForm",
    b"ImportData",
    b"Rendition",
    b"GoToE",
];

/// Deepest nesting of drawings (forms, patterns, glyphs, soft masks) looked into for hidden
/// layers. Deeper ones make the removal refuse rather than leave them.
const MAX_NESTING: usize = 32;

/// A drawing hidden layers are taken out of.
enum Drawn {
    Page(ObjectId),
    Stream(ObjectId),
}

/// The drawings that held content of hidden layers, without it, and what couldn't be looked
/// into.
#[derive(Default)]
struct LayerScan {
    edits: Vec<(Drawn, Vec<Operation>, Inline)>,
    blocked: Vec<String>,
    /// Some of what is in `blocked` is damaged data.
    damaged: bool,
    /// Some data read had a wrong checksum.
    faults: bool,
}

/// A drawing the layer scan has looked into. One without resources of its own draws with
/// those of where it is used, so its layer names can mean other layers there: for such a
/// drawing, the resources it was first looked into with, and the names each set of resources
/// it has been checked against gives hidden layers.
#[derive(Default)]
struct Seen {
    first: Option<Dictionary>,
    checked: Vec<HiddenNames>,
}

/// The names resources give content in hidden layers: in `/Properties`, and in `/XObject`
/// (pictures and forms in a hidden layer), each sorted. A drawing loses the same content
/// with any resources that give the same names.
type HiddenNames = (Vec<Vec<u8>>, Vec<Vec<u8>>);

/// Why a stream's content couldn't be read.
enum Unread {
    /// Its compressed data is cut off or damaged part way.
    Damaged,
    Other(anyhow::Error),
}

/// A stream's content, decoded, and whether it had a wrong checksum; or why it couldn't be
/// read.
type Read = std::result::Result<(Vec<u8>, bool), Unread>;

impl Pdf {
    /// What [`Pdf::remove_hidden`] would remove of every kind, worked out on a copy. Hidden
    /// layers are counted even when their removal would be refused.
    pub fn find_hidden(&self) -> Result<HiddenReport> {
        self.clone().remove_kinds(&Hidden::ALL, false)
    }

    /// Remove the kinds of hidden information in `what`. Unused objects go when the
    /// document is saved, which rewrites the whole file, so nothing removed is kept in it.
    /// Refuses, with nothing changed, when it couldn't remove all of a kind: hidden layers
    /// in drawings it can't read or nested too deep, a form nested too deep.
    pub fn remove_hidden(&mut self, what: &[Hidden]) -> Result<HiddenReport> {
        self.remove_kinds(what, true)
    }

    /// Why [`Pdf::remove_hidden`] would refuse to remove `what`, if it would; changes nothing.
    /// For checking before anything else is done for the removal.
    pub fn check_remove_hidden(&self, what: &[Hidden]) -> Result<()> {
        if what.contains(&Hidden::HiddenLayers) {
            if let Some((_, _, hidden)) = self.hidden_layers() {
                refuse_blocked(&self.scan_layers(&hidden))?;
            }
        }
        if what.contains(&Hidden::FormValues) {
            self.orphan_widgets(&self.value_nodes()?)?;
        }
        Ok(())
    }

    fn remove_kinds(&mut self, what: &[Hidden], strict: bool) -> Result<HiddenReport> {
        if strict {
            self.check_remove_hidden(what)?;
        }
        self.own_direct_annots()?;
        let mut report = HiddenReport::default();
        for h in Hidden::ALL {
            if !what.contains(&h) {
                continue;
            }
            let n = match h {
                Hidden::OutsideCrop => self.cut_outside_crop()?,
                Hidden::HiddenLayers => {
                    let (n, faults) = self.drop_hidden_layers(strict)?;
                    report.faults_overlooked |= faults;
                    n
                }
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
                Hidden::FormValues => self.clear_form_values()?,
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
            .every_annotation()
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
        let bad: HashSet<ObjectId> = self
            .doc
            .objects
            .iter()
            .filter(|(_, o)| o.as_dict().is_ok_and(is_active))
            .map(|(id, _)| *id)
            .collect();
        let mut n = self.drop_names(&[b"JavaScript"]);
        let xfa_only = self.xfa_only();
        for obj in self.doc.objects.values_mut() {
            n += without_scripts(obj, &bad, xfa_only);
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

    /// The layer properties, their default configuration, and the layers (optional content
    /// groups) switched off in it; `None` when no layer is.
    fn hidden_layers(&self) -> Option<(Dictionary, Dictionary, HashSet<ObjectId>)> {
        let props = self
            .doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"OCProperties").ok())
            .map(|o| self.resolved_dict(Some(o)))?;
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
        (!hidden.is_empty()).then_some((props, d, hidden))
    }

    /// Delete the content of the layers (optional content groups) that are switched off,
    /// and take those layers out of the layer list. Returns how many layers went, and
    /// whether data with a wrong checksum was read. When `strict`, refuses with nothing
    /// changed if some drawing couldn't be looked into; otherwise (finding) counts the
    /// layers and changes nothing then.
    fn drop_hidden_layers(&mut self, strict: bool) -> Result<(usize, bool)> {
        let Some((props, d, hidden)) = self.hidden_layers() else {
            return Ok((0, false));
        };
        let scan = self.scan_layers(&hidden);
        let faults = scan.faults;
        if !scan.blocked.is_empty() {
            if strict {
                refuse_blocked(&scan)?;
            }
            return Ok((hidden.len(), faults));
        }
        for (drawn, ops, inline) in scan.edits {
            match drawn {
                Drawn::Page(page) => self.commit(page, ops, 0, &inline)?,
                Drawn::Stream(id) => {
                    let bytes = inline
                        .encode(ops)
                        .map_err(|e| anyhow!("cannot encode a form: {e}"))?;
                    if let Ok(s) = self.doc.get_object_mut(id).and_then(|o| o.as_stream_mut()) {
                        s.dict.remove(b"Filter");
                        s.dict.remove(b"DecodeParms");
                        s.set_content(bytes);
                        let _ = s.compress();
                    }
                }
            }
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
            for id in self.all_annot_refs(page) {
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
        self.drop_hidden_properties(&hidden);

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
        Ok((hidden.len(), faults))
    }

    /// Every drawing that can hold content of the `hidden` layers, without it: the pages'
    /// content, then what their resources and their annotations' appearances draw (forms,
    /// tiling patterns, Type 3 glyphs, soft masks), nested up to [`MAX_NESTING`], each once
    /// (layer states are the document's, so a drawing with its own resources looks the same
    /// wherever it is used). One without resources of its own is checked against every
    /// place it is used from. What couldn't be read, is nested deeper, or would lose
    /// different content in different places, goes in `blocked`.
    fn scan_layers(&self, hidden: &HashSet<ObjectId>) -> LayerScan {
        let mut scan = LayerScan::default();
        let mut seen = HashMap::new();
        for (i, page) in self.page_ids().into_iter().enumerate() {
            let res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
            let content = self.page_content(page);
            let name = format!("page {}'s content", i + 1);
            self.scan_read(Drawn::Page(page), content, &res, hidden, &name, &mut scan);
            self.scan_resources(&res, hidden, &mut seen, 0, &mut scan);
            for annot in self.all_annot_refs(page) {
                let ap = self
                    .doc
                    .get_dictionary(annot)
                    .ok()
                    .and_then(|d| d.get(b"AP").ok())
                    .map(|o| self.resolved_dict(Some(o)));
                for (_, look) in ap.iter().flat_map(|ap| ap.iter()) {
                    // A drawing, or a dictionary of drawings by state.
                    let ids: Vec<ObjectId> = match self.doc.dereference(look) {
                        Ok((_, Object::Dictionary(states))) => states
                            .iter()
                            .filter_map(|(_, s)| s.as_reference().ok())
                            .collect(),
                        _ => look.as_reference().ok().into_iter().collect(),
                    };
                    for id in ids {
                        self.scan_drawing(id, &Dictionary::new(), hidden, &mut seen, 0, &mut scan);
                    }
                }
            }
        }
        let mut once = HashSet::new();
        scan.blocked.retain(|b| once.insert(b.clone()));
        scan
    }

    /// [`Pdf::scan_layers`] for the drawings resources `res` hold, at nesting `depth`.
    fn scan_resources(
        &self,
        res: &Dictionary,
        hidden: &HashSet<ObjectId>,
        seen: &mut HashMap<ObjectId, Seen>,
        depth: usize,
        scan: &mut LayerScan,
    ) {
        let entries = |key: &[u8]| self.resolved_dict(res.get(key).ok());
        let stream = |o: &Object| -> Option<(ObjectId, &lopdf::Stream)> {
            let id = o.as_reference().ok()?;
            Some((id, self.doc.get_object(id).ok()?.as_stream().ok()?))
        };
        let name_is = |d: &Dictionary, key: &[u8], v: &[u8]| {
            d.get(key).and_then(|o| o.as_name()).ok() == Some(v)
        };
        let mut drawings: Vec<ObjectId> = Vec::new();
        for (_, e) in entries(b"XObject").iter() {
            if let Some((id, s)) = stream(e) {
                if name_is(&s.dict, b"Subtype", b"Form") {
                    drawings.push(id);
                }
            }
        }
        for (_, e) in entries(b"Pattern").iter() {
            if let Some((id, _)) = stream(e) {
                drawings.push(id);
            }
        }
        for (_, e) in entries(b"ExtGState").iter() {
            let gs = self.resolved_dict(Some(e));
            let mask = self.resolved_dict(gs.get(b"SMask").ok());
            if let Some((id, _)) = mask.get(b"G").ok().and_then(stream) {
                drawings.push(id);
            }
        }
        let mut glyphs: Vec<(ObjectId, Dictionary)> = Vec::new();
        for (_, e) in entries(b"Font").iter() {
            let font = self.resolved_dict(Some(e));
            if !name_is(&font, b"Subtype", b"Type3") {
                continue;
            }
            let fres = font
                .get(b"Resources")
                .ok()
                .map_or_else(|| res.clone(), |o| self.resolved_dict(Some(o)));
            let procs = self.resolved_dict(font.get(b"CharProcs").ok());
            for (_, g) in procs.iter() {
                if let Some((id, _)) = stream(g) {
                    glyphs.push((id, fres.clone()));
                }
            }
        }
        let todo = drawings
            .into_iter()
            .map(|id| (id, res.clone()))
            .chain(glyphs);
        for (id, outer) in todo {
            if !seen.contains_key(&id) && depth >= MAX_NESTING {
                scan.blocked
                    .push(format!("drawings are nested more than {MAX_NESTING} deep"));
                return;
            }
            self.scan_drawing(id, &outer, hidden, seen, depth + 1, scan);
        }
    }

    /// [`Pdf::scan_layers`] for drawing `id`, which draws with its own resources, else with
    /// `outer`. One looked into already is only checked against `outer`, when it draws with
    /// it.
    fn scan_drawing(
        &self,
        id: ObjectId,
        outer: &Dictionary,
        hidden: &HashSet<ObjectId>,
        seen: &mut HashMap<ObjectId, Seen>,
        depth: usize,
        scan: &mut LayerScan,
    ) {
        let Ok(s) = self.doc.get_object(id).and_then(|o| o.as_stream()) else {
            seen.entry(id).or_default();
            return;
        };
        let own = s.dict.get(b"Resources").ok();
        if let Some(was) = seen.get_mut(&id) {
            if own.is_none() {
                self.check_shared(id, s, outer, hidden, was, scan);
            }
            return;
        }
        let res = own.map_or_else(|| outer.clone(), |o| self.resolved_dict(Some(o)));
        let mut was = Seen::default();
        if own.is_none() {
            was.first = Some(res.clone());
            was.checked.push(self.hidden_names(&res, hidden));
        }
        seen.insert(id, was);
        let name = format!("a drawing in the file (object {})", id.0);
        self.scan_read(Drawn::Stream(id), readable(s), &res, hidden, &name, scan);
        self.scan_resources(&res, hidden, seen, depth, scan);
    }

    /// Check drawing `id` (stream `s`), which has no resources of its own and was looked
    /// into with `was.first`, against resources `res` of another place it is used from.
    /// Where it would lose other content, the removal can't be done on the one drawing:
    /// that goes in `blocked`.
    fn check_shared(
        &self,
        id: ObjectId,
        s: &lopdf::Stream,
        res: &Dictionary,
        hidden: &HashSet<ObjectId>,
        was: &mut Seen,
        scan: &mut LayerScan,
    ) {
        let names = self.hidden_names(res, hidden);
        if was.checked.contains(&names) {
            return;
        }
        was.checked.push(names);
        let (Some(first), Ok((data, _))) = (&was.first, readable(s)) else {
            return;
        };
        // What can't be read is in `blocked` already, from the first look.
        let (Ok(a), Ok(b)) = (
            self.without_hidden(&data, first, hidden),
            self.without_hidden(&data, res, hidden),
        ) else {
            return;
        };
        let same = match (a, b) {
            (None, None) => true,
            (Some((a, _)), Some((b, _))) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(&b)
                        .all(|(x, y)| x.operator == y.operator && x.operands == y.operands)
            }
            _ => false,
        };
        if !same {
            scan.blocked.push(format!(
                "a drawing in the file (object {}) is shared between places that set its \
                 layers differently",
                id.0
            ));
        }
    }

    /// The names resources `res` give content in the `hidden` layers, as
    /// [`Pdf::without_hidden`] looks them up.
    fn hidden_names(&self, res: &Dictionary, hidden: &HashSet<ObjectId>) -> HiddenNames {
        let props = self.resolved_dict(res.get(b"Properties").ok());
        let mut layers: Vec<Vec<u8>> = props
            .iter()
            .filter(|(_, o)| self.oc_hidden(o, hidden))
            .map(|(n, _)| n.clone())
            .collect();
        let xobjs = self.resolved_dict(res.get(b"XObject").ok());
        let mut drawings: Vec<Vec<u8>> = xobjs
            .iter()
            .filter(|(_, e)| {
                self.doc
                    .dereference(e)
                    .ok()
                    .and_then(|(_, s)| s.as_stream().ok())
                    .and_then(|s| s.dict.get(b"OC").ok())
                    .is_some_and(|oc| self.oc_hidden(oc, hidden))
            })
            .map(|(n, _)| n.clone())
            .collect();
        layers.sort();
        drawings.sort();
        (layers, drawings)
    }

    /// Page `page`'s content streams, read with [`readable`] and joined as lopdf joins them,
    /// at most [`crate::MAX_CONTENT`] bytes in all.
    fn page_content(&self, page: ObjectId) -> Read {
        let mut all = Vec::new();
        let mut faults = false;
        for c in self.doc.get_page_contents(page) {
            let Ok(s) = self.doc.get_object(c).and_then(|o| o.as_stream()) else {
                continue;
            };
            let (data, fault) = readable(s)?;
            all.extend_from_slice(&data);
            all.push(b'\n');
            faults |= fault;
            if all.len() > crate::MAX_CONTENT {
                return Err(Unread::Other(anyhow!("it is too large")));
            }
        }
        Ok((all, faults))
    }

    /// Record in `scan` what [`Pdf::without_hidden`] makes of drawing `drawn`, read as
    /// `read` and drawn with resources `res`; `name` names it when it couldn't be read.
    fn scan_read(
        &self,
        drawn: Drawn,
        read: Read,
        res: &Dictionary,
        hidden: &HashSet<ObjectId>,
        name: &str,
        scan: &mut LayerScan,
    ) {
        let edit = read.and_then(|(data, fault)| {
            scan.faults |= fault;
            self.without_hidden(&data, res, hidden)
                .map_err(Unread::Other)
        });
        match edit {
            Ok(Some((ops, inline))) => scan.edits.push((drawn, ops, inline)),
            Ok(None) => {}
            Err(Unread::Damaged) => {
                scan.damaged = true;
                scan.blocked.push(format!("{name} is damaged"));
            }
            Err(Unread::Other(e)) => scan.blocked.push(format!("{name} can't be read ({e})")),
        }
    }

    /// Take the entries naming hidden layers out of every `/Properties` resource dictionary:
    /// the content that used them is gone.
    fn drop_hidden_properties(&mut self, hidden: &HashSet<ObjectId>) {
        let hidden_ocmds: HashSet<ObjectId> = self
            .doc
            .objects
            .iter()
            .filter(|(_, o)| o.as_dict().is_ok_and(|d| membership_hidden(d, hidden)))
            .map(|(id, _)| *id)
            .collect();
        let gone = |o: &Object| match o {
            Object::Reference(id) => hidden.contains(id) || hidden_ocmds.contains(id),
            Object::Dictionary(d) => membership_hidden(d, hidden),
            _ => false,
        };
        fn walk(o: &mut Object, gone: &dyn Fn(&Object) -> bool, lists: &mut Vec<ObjectId>) {
            let d = match o {
                Object::Dictionary(d) => d,
                Object::Stream(s) => &mut s.dict,
                Object::Array(a) => return a.iter_mut().for_each(|x| walk(x, gone, lists)),
                _ => return,
            };
            match d.get_mut(b"Properties") {
                Ok(Object::Dictionary(props)) => {
                    let names: Vec<Vec<u8>> = props
                        .iter()
                        .filter(|(_, v)| gone(v))
                        .map(|(k, _)| k.clone())
                        .collect();
                    for k in names {
                        props.remove(&k);
                    }
                }
                Ok(Object::Reference(id)) => lists.push(*id),
                _ => {}
            }
            for (_, v) in d.iter_mut() {
                walk(v, gone, lists);
            }
        }
        let mut lists = Vec::new();
        for o in self.doc.objects.values_mut() {
            walk(o, &gone, &mut lists);
        }
        for id in lists {
            if let Ok(props) = self.doc.get_dictionary_mut(id) {
                let names: Vec<Vec<u8>> = props
                    .iter()
                    .filter(|(_, v)| gone(v))
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in names {
                    props.remove(&k);
                }
            }
        }
    }

    /// Whether optional content `oc` (a layer, or a membership dictionary combining
    /// layers) is hidden when `hidden` layers are off.
    fn oc_hidden(&self, oc: &Object, hidden: &HashSet<ObjectId>) -> bool {
        if let Ok(id) = oc.as_reference() {
            if hidden.contains(&id) {
                return true;
            }
        }
        self.doc
            .dereference(oc)
            .ok()
            .and_then(|(_, o)| o.as_dict().ok())
            .is_some_and(|d| membership_hidden(d, hidden))
    }

    /// A content stream without what is in hidden layers: marked content tagged with a
    /// hidden layer, and pictures and forms that belong to one. `None` when nothing was.
    #[allow(clippy::type_complexity)]
    fn without_hidden(
        &self,
        data: &[u8],
        res: &Dictionary,
        hidden: &HashSet<ObjectId>,
    ) -> Result<Option<(Vec<Operation>, Inline)>> {
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
}

/// A stream's content, decoded with its filters in turn. lopdf returns what Flate data
/// decodes to up to a fault rather than an error, and can lose the end of data whose
/// checksum is wrong, so Flate data is checked to its end first: complete data with a wrong
/// or missing checksum is read in full (as other readers do) and reported; data cut off or
/// damaged part way is [`Unread::Damaged`]. Data in other filters that decodes to nothing
/// is an error, as lopdf gives no error for it either.
fn readable(s: &lopdf::Stream) -> Read {
    let max = crate::MAX_CONTENT;
    let filters = s.filters().unwrap_or_default();
    if filters.is_empty() {
        let data = s.get_plain_content_with_limit(max);
        return data
            .map(|d| (d, false))
            .map_err(|e| Unread::Other(anyhow!("{e}")));
    }
    let params = s.dict.get(b"DecodeParms").and_then(Object::as_dict).ok();
    let mut data = s.content.clone();
    let mut fault = false;
    for f in &filters {
        if *f == b"FlateDecode" && !data.is_empty() {
            match inflate(&data, max).map_err(Unread::Other)? {
                Inflated::Whole => {}
                Inflated::BadChecksum(fixed) => {
                    data = fixed;
                    fault = true;
                }
                Inflated::Damaged => return Err(Unread::Damaged),
            }
        }
        let mut d = Dictionary::new();
        d.set("Filter", Object::Name(f.to_vec()));
        if let Some(p) = params {
            d.set("DecodeParms", p.clone());
        }
        data = lopdf::Stream::new(d, data)
            .get_plain_content_with_limit(max)
            .map_err(|e| Unread::Other(anyhow!("{e}")))?;
    }
    let flate = filters.iter().any(|f| *f == b"FlateDecode");
    if data.is_empty() && !s.content.trim_ascii_end().is_empty() && !flate {
        return Err(Unread::Other(anyhow!("its compressed data doesn't decode")));
    }
    Ok((data, fault))
}

/// What [`inflate`] found.
enum Inflated {
    Whole,
    /// Complete, but its checksum is wrong or missing: the data with the right one.
    BadChecksum(Vec<u8>),
    /// Cut off, or damaged part way.
    Damaged,
}

/// Check zlib data `z` (Flate data) to its end, decoding at most `limit` bytes of it.
fn inflate(z: &[u8], limit: usize) -> Result<Inflated> {
    use flate2::{Decompress, FlushDecompress, Status};
    // A zlib header: deflate, a valid window, no preset dictionary, its check bits right.
    let header = z.len() >= 2
        && z[0] & 0x0f == 8
        && z[0] >> 4 <= 7
        && z[1] & 0x20 == 0
        && (u16::from(z[0]) << 8 | u16::from(z[1])) % 31 == 0;
    if !header {
        return Ok(Inflated::Damaged);
    }
    let body = &z[2..];
    let mut d = Decompress::new(false);
    let mut out = Vec::with_capacity(body.len().saturating_mul(2).min(limit + 1));
    loop {
        if out.len() == out.capacity() {
            if out.len() > limit {
                return Err(anyhow!("it is too large"));
            }
            out.reserve(out.len().max(1024).min(limit + 1 - out.len()));
        }
        let (used, made) = (d.total_in(), d.total_out());
        match d.decompress_vec(&body[used as usize..], &mut out, FlushDecompress::None) {
            Ok(Status::StreamEnd) => break,
            // Nothing more comes out with room for it: the data stops short.
            Ok(_) if d.total_in() == used && d.total_out() == made => return Ok(Inflated::Damaged),
            Ok(_) => {}
            Err(_) => return Ok(Inflated::Damaged),
        }
    }
    if out.len() > limit {
        return Err(anyhow!("it is too large"));
    }
    let end = 2 + d.total_in() as usize;
    let sum = adler32(&out).to_be_bytes();
    if z.get(end..end + 4) == Some(&sum[..]) {
        return Ok(Inflated::Whole);
    }
    let mut fixed = z[..end].to_vec();
    fixed.extend_from_slice(&sum);
    Ok(Inflated::BadChecksum(fixed))
}

/// zlib's checksum (RFC 1950).
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    // The most bytes that can be added before the sums must be reduced.
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

/// A refusal naming what couldn't be looked into, when anything couldn't. Damaged data is
/// told in plain words; it is the likely cause people meet.
fn refuse_blocked(scan: &LayerScan) -> Result<()> {
    if scan.damaged {
        return Err(LayersDamaged.into());
    }
    if scan.blocked.is_empty() {
        return Ok(());
    }
    Err(anyhow!(
        "hidden layers can't be removed for certain: {}. Nothing was changed; leave out \
         hidden layers to remove the rest",
        scan.blocked.join("; ")
    ))
}

/// Whether `d` is an optional content membership dictionary (`/Type /OCMD`) that hides
/// what it applies to when `hidden` layers are off.
fn membership_hidden(d: &Dictionary, hidden: &HashSet<ObjectId>) -> bool {
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

/// Whether an action dictionary runs something by itself.
fn is_active(d: &Dictionary) -> bool {
    d.get(b"S")
        .and_then(|o| o.as_name())
        .is_ok_and(|s| ACTIVE.contains(&s))
}

/// Take the actions that run something out of `o` and out of every dictionary written
/// directly inside it (an ordinary link's action whose `/Next` holds JavaScript, an
/// annotation written into a page's list), where `bad` are the objects that are such
/// actions. Ordinary actions stay. Returns how many were removed. Direct objects can't loop,
/// and lopdf's parser bounds how deeply they nest.
fn without_scripts(o: &mut Object, bad: &HashSet<ObjectId>, xfa_only: bool) -> usize {
    let d = match o {
        Object::Dictionary(d) => d,
        Object::Stream(s) => &mut s.dict,
        Object::Array(a) => {
            return a
                .iter_mut()
                .map(|x| without_scripts(x, bad, xfa_only))
                .sum()
        }
        _ => return 0,
    };
    let is_bad = |o: &Object| match o {
        Object::Reference(id) => bad.contains(id),
        Object::Dictionary(d) => is_active(d),
        _ => false,
    };
    // Additional actions: on opening, closing, printing, keystrokes, focus….
    let mut n = usize::from(d.remove(b"AA").is_some());
    for key in [&b"A"[..], b"OpenAction"] {
        if d.get(key).is_ok_and(is_bad) {
            d.remove(key);
            n += 1;
        }
    }
    match d.get_mut(b"Next") {
        Ok(Object::Array(a)) => {
            let before = a.len();
            a.retain(|o| !is_bad(o));
            n += before - a.len();
        }
        Ok(o) if is_bad(o) => {
            d.remove(b"Next");
            n += 1;
        }
        _ => {}
    }
    // A hybrid form's XFA part holds scripts of its own; the ordinary fields stay.
    if !xfa_only && d.has(b"Fields") && d.remove(b"XFA").is_some() {
        n += 1;
    }
    for (_, v) in d.iter_mut() {
        n += without_scripts(v, bad, xfa_only);
    }
    n
}
