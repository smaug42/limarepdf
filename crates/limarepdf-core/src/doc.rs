use crate::Result;
use anyhow::{anyhow, bail, Context};
use lopdf::dictionary;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use rayon::prelude::*;
use std::collections::HashSet;
use std::io::{Cursor, Write};
use std::path::Path;

/// A PDF document opened for editing.
#[derive(Clone)]
pub struct Pdf {
    pub doc: Document,
    pub(crate) encryption: Option<crate::Encryption>,
    /// Why other PDF apps may read the file this was opened from differently from LimarePDF,
    /// if they may. Signing and checking signatures depend on the two agreeing.
    pub(crate) differs: Option<String>,
    /// Whether the file's cross-reference information was damaged and LimarePDF rebuilt it
    /// to open the file.
    pub(crate) repaired: bool,
    /// Whether the file was encrypted and needed a password to open.
    pub(crate) password_needed: bool,
    /// The fonts LimarePDF embedded since the document was opened: in a signed document,
    /// only these are grown (see `embed.rs`).
    pub(crate) made_fonts: HashSet<ObjectId>,
}

/// Why a repaired document may read differently in other PDF apps.
const REPAIRED: &str = "its cross-reference information is damaged, and LimarePDF rebuilt it";
/// Why a document whose page tree was mended differs from what other readers show.
const PAGES_MENDED: &str = "its list of pages is damaged, and LimarePDF rebuilt it";
/// Why a document is refused when no pages can be found in it.
const NO_PAGES: &str =
    "LimarePDF can't find any pages in it (there are no pages, or the file is damaged)";

/// Document information dictionary fields.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct Info {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
}

const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

impl Pdf {
    // ---------------------------------------------------------------- I/O

    pub fn open(path: impl AsRef<Path>, password: Option<&str>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_bytes(&bytes, password)
    }

    /// Reads a document. A file that reads with no pages is repaired (see
    /// [`Pdf::repaired`]): its objects are found by scanning the bytes, in case the
    /// cross-reference information is damaged, and a page tree other readers find pages in
    /// is mended. A file in which no pages can be found is refused.
    pub fn from_bytes(bytes: &[u8], password: Option<&str>) -> Result<Self> {
        let opened = Self::read(bytes, password);
        let pages = match &opened {
            Ok(p) => p.page_count(),
            Err(lopdf::Error::InvalidPassword) => return opened.map_err(cannot_open),
            Err(_) => 0,
        };
        // lopdf rebuilds a table it can't read at all by scanning, and then has no table
        // offset (a real one can't be 0: the header is there). Its scan ends a stream at the
        // first `endstream` and ignores `/Length`, so LimarePDF's own repair is used
        // instead, and lopdf's only when that fails.
        let rebuilt_by_lopdf = matches!(&opened, Ok(p) if p.doc.xref_start == 0);
        if pages > 0 && !rebuilt_by_lopdf {
            return opened.map_err(cannot_open);
        }
        let rebuilt = match crate::repair::rebuild(bytes).map(|b| Self::read(&b, password)) {
            // Only the repaired file reads far enough to ask for the password.
            Some(Err(e @ lopdf::Error::InvalidPassword)) => return Err(cannot_open(e)),
            Some(r) => r.ok(),
            None => None,
        };
        // The page tree is mended in the rebuilt file rather than in lopdf's reading, so that
        // the pages' content is found too when the table's offsets are wrong.
        let repaired = rebuilt.and_then(|mut p| {
            crate::repair::unpack_object_streams(&mut p.doc);
            // The trailers may name no catalog, or the wrong object.
            if p.page_count() == 0 {
                let root = crate::repair::find_catalog(&p.doc)?;
                p.doc.trailer.set("Root", root);
            }
            let mut why = REPAIRED;
            if p.page_count() == 0 && crate::repair::mend_page_tree(&mut p.doc, bytes) {
                why = PAGES_MENDED;
            }
            (p.page_count() > 0).then_some((p, why))
        });
        let (mut p, why) = match repaired {
            Some(r) => r,
            // lopdf's own rebuild.
            None if pages > 0 => (opened.map_err(cannot_open)?, REPAIRED),
            // Rather than an empty document, which other readers wouldn't show either.
            None => {
                opened.map_err(cannot_open)?;
                bail!("cannot open PDF: {NO_PAGES}");
            }
        };
        p.repaired = true;
        p.differs = Some(why.into());
        Ok(p)
    }

    /// Whether the file was damaged (its cross-reference information, which says where each
    /// object is, or its list of pages), and LimarePDF rebuilt it to open the file. Some
    /// content may be missing. Such a
    /// document counts as changed: saving writes a whole repaired copy, never a revision
    /// added to the damaged file, and its signatures never check as valid and unchanged
    /// (see [`Pdf::differs`]).
    pub fn repaired(&self) -> bool {
        self.repaired
    }

    /// Why other PDF apps may show the file this was opened from differently from LimarePDF,
    /// if they may: hidden objects in use, data after its end, objects LimarePDF can't read
    /// as they do, or a repair. Signing such a file is refused, and its signatures can't be
    /// checked.
    pub fn differs(&self) -> Option<&str> {
        self.differs.as_deref()
    }

    /// Whether the file needed a password to open. An encrypted file with an empty open
    /// password (only a permissions password, limiting printing, copying or changes) opens
    /// without one, in every reader.
    pub fn password_needed(&self) -> bool {
        self.password_needed
    }

    /// Reads a document as its cross-reference information says.
    fn read(bytes: &[u8], password: Option<&str>) -> std::result::Result<Self, lopdf::Error> {
        let load = |b: &[u8]| {
            Document::load_from_with_options(
                Cursor::new(b),
                lopdf::LoadOptions::with_password(password.unwrap_or("")),
            )
        };
        let copy;
        let (doc, read) = match load(bytes) {
            Ok(doc) => (doc, bytes),
            // Some writers put `/Encrypt null` (meaning not encrypted) in the trailer, and
            // lopdf takes any `/Encrypt` entry as encryption. Read a copy with those
            // entries blanked; they are the same length, so every offset holds.
            Err(e) => match blank_null_encrypt(bytes).or_else(|| pad_user_hash(bytes)) {
                Some(c) => {
                    copy = c;
                    let doc = load(&copy).map_err(|_| e)?;
                    (doc, &copy[..])
                }
                None => return Err(e),
            },
        };
        // lopdf tries the empty password first, so a file opened without a password, or
        // with an empty one, didn't need one. Given another, it may still not have: without
        // a password lopdf decrypts the file only when the empty one opens it, and otherwise
        // keeps `/Encrypt` in the trailer.
        let password_needed = doc.was_encrypted()
            && password.is_some_and(|p| !p.is_empty())
            && Document::load_from_with_options(Cursor::new(read), lopdf::LoadOptions::default())
                .map_or(true, |d| d.trailer.has(b"Encrypt"));
        let mut pdf = Self {
            doc,
            encryption: None,
            differs: None,
            repaired: false,
            password_needed,
            made_fonts: Default::default(),
        };
        pdf.differs = pdf.match_other_readers(read);
        Ok(pdf)
    }

    /// Makes lopdf's reading of `bytes` what other PDF apps read: drops the objects they
    /// don't see (see [`crate::sig_changes::compare_reading`]). Returns why the readings
    /// may still differ, if they may.
    fn match_other_readers(&mut self, bytes: &[u8]) -> Option<String> {
        let reading = match crate::sig_changes::compare_reading(bytes, &self.doc) {
            Ok(r) => r,
            Err(why) => return Some(why),
        };
        for id in &reading.hidden {
            self.doc.objects.remove(id);
        }
        if !reading.ends_cleanly {
            return Some(
                "it doesn't end with its cross-reference pointer (data may follow its \
                 end-of-file marker)"
                    .into(),
            );
        }
        let used = crate::sig_changes::references(&self.doc);
        if let Some(id) = reading.hidden.iter().filter(|id| used.contains(id)).min() {
            return Some(format!(
                "it uses object {}, which its cross-reference information deletes or doesn't \
                 list",
                id.0
            ));
        }
        let unsure = used
            .iter()
            .filter(|id| reading.unsure.contains(&id.0))
            .min();
        unsure.map(|id| format!("LimarePDF can't read its object {} the way they do", id.0))
    }

    /// A new empty document with no pages.
    pub fn new() -> Self {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.add_object(dictionary! {
            "Type" => "Pages", "Kids" => Vec::<Object>::new(), "Count" => 0,
        });
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        Self {
            doc,
            encryption: None,
            differs: None,
            repaired: false,
            password_needed: false,
            made_fonts: Default::default(),
        }
    }

    fn write_to<W: Write>(&mut self, w: &mut W) -> Result<()> {
        self.doc.prune_objects();
        self.doc.compress();
        match &self.encryption {
            // Encryption must be the last step: compressing ciphertext would corrupt streams.
            Some(enc) => {
                let mut d = self.doc.clone();
                if d.trailer.get(b"ID").is_err() {
                    // The standard security handler derives keys from the file identifier.
                    let mut id = [0u8; 16];
                    rand::Rng::fill(&mut rand::rng(), &mut id);
                    let s = Object::String(id.to_vec(), lopdf::StringFormat::Hexadecimal);
                    d.trailer.set("ID", vec![s.clone(), s]);
                }
                let state = enc.state(&d)?;
                d.encrypt(&state)?;
                fix_encrypted(&mut d, enc.cipher);
                d.save_to(w)?;
            }
            None => self.doc.save_to(w)?,
        }
        Ok(())
    }

    /// Whether the document was read from an encrypted (password-protected) file, or is
    /// set to be saved with a password.
    pub fn encrypted(&self) -> bool {
        self.encryption.is_some()
            || self.doc.encryption_state.is_some()
            || self.doc.trailer.has(b"Encrypt")
    }

    /// Serialise without pruning, compressing or encrypting. Used for undo snapshots, where
    /// speed matters more than size.
    pub fn snapshot(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.doc.save_to(&mut out)?;
        Ok(out)
    }

    /// Like [`Pdf::snapshot`], with unused objects removed first, as a save removes them:
    /// for copies written to disk. Redacting and removing hidden information leave what
    /// they took out in objects nothing uses any more, which only pruning drops.
    pub fn pruned_snapshot(&mut self) -> Result<Vec<u8>> {
        self.doc.prune_objects();
        self.snapshot()
    }

    /// Like [`Pdf::snapshot`], for drawing pages as the app shows them (`limarepdf-cli
    /// render`). hayro honours the Hidden annotation flag but not NoView, so annotations
    /// flagged NoView are marked Hidden in the copy that is serialised. Form fields are drawn
    /// as [`Pdf::refresh_appearances`] would draw them, as the app draws them when opening a
    /// file ([`Pdf::unshown_fields`] names those it can't draw). The document itself is not
    /// changed.
    pub fn render_snapshot(&mut self) -> Result<Vec<u8>> {
        self.snapshot_hiding(|flags, _| flags & 32 != 0, true)
    }

    /// Like [`Pdf::render_snapshot`], for the app, which draws sticky notes itself at a
    /// fixed size on screen: notes are left out too.
    pub fn app_snapshot(&mut self) -> Result<Vec<u8>> {
        self.snapshot_hiding(|flags, sub| flags & 32 != 0 || sub == b"Text", false)
    }

    /// Like [`Pdf::app_snapshot`], with every page shown whole: each page's crop box is
    /// its full (media) box in the copy, so the app can show what cropping leaves out
    /// while the crop is being set. The document itself is not changed.
    pub fn uncropped_app_snapshot(&mut self) -> Result<Vec<u8>> {
        let mut own = Vec::new();
        for id in self.page_ids() {
            let full: Vec<Object> = self.media_box(id).iter().map(|&v| v.into()).collect();
            let d = self.doc.get_dictionary_mut(id)?;
            own.push((id, d.get(b"CropBox").ok().cloned()));
            d.set("CropBox", full);
        }
        let bytes = self.app_snapshot();
        for (id, crop) in own {
            let d = self.doc.get_dictionary_mut(id)?;
            match crop {
                Some(c) => d.set("CropBox", c),
                None => {
                    d.remove(b"CropBox");
                }
            }
        }
        bytes
    }

    /// Like [`Pdf::render_snapshot`], for printing: annotations without the Print flag
    /// (bit 3) are marked Hidden instead, and form fields are drawn as
    /// [`Pdf::refresh_appearances`] would draw them in every document, since the command
    /// line, unlike the app, doesn't draw them when opening a file.
    pub fn print_snapshot(&mut self) -> Result<Vec<u8>> {
        self.snapshot_hiding(|flags, _| flags & 4 == 0, true)
    }

    /// A snapshot in which annotations whose flags and subtype match `hide` are marked
    /// Hidden, and annotations without a drawing get LimarePDF's (the document itself keeps
    /// none). In a signed document, or in any with `all_fields`, form fields are drawn this
    /// way too, as [`Pdf::refresh_appearances`] would draw them: the app doesn't draw a
    /// signed document's fields into it when opening it, so that saving adds only the edits
    /// made to what was signed.
    fn snapshot_hiding(
        &mut self,
        hide: impl Fn(i64, &[u8]) -> bool,
        all_fields: bool,
    ) -> Result<Vec<u8>> {
        let ids = self.annots_where(hide);
        let missing = self.missing_appearances(&self.page_ids());
        let fields =
            (all_fields || self.signature_count() > 0) && !self.appearances_to_draw().is_empty();
        if ids.is_empty() && missing.is_empty() && !fields {
            return self.snapshot();
        }
        let mut copy = self.clone();
        if fields {
            copy.refresh_appearances();
        }
        copy.draw_annotations(&missing);
        for (id, flags) in ids {
            if let Ok(a) = copy.doc.get_dictionary_mut(id) {
                a.set("F", flags | 2);
            }
        }
        let mut out = Vec::new();
        copy.doc.save_to(&mut out)?;
        Ok(out)
    }

    /// Annotations not Hidden (bit 2) whose flags and subtype match `hide`, with their
    /// flags.
    fn annots_where(&self, hide: impl Fn(i64, &[u8]) -> bool) -> Vec<(ObjectId, i64)> {
        let mut out = Vec::new();
        for page in self.page_ids() {
            let Some(annots) = self
                .doc
                .get_dictionary(page)
                .ok()
                .and_then(|d| d.get(b"Annots").ok())
            else {
                continue;
            };
            let Ok((_, Object::Array(annots))) = self.doc.dereference(annots) else {
                continue;
            };
            for r in annots {
                let Ok(id) = r.as_reference() else { continue };
                let Ok(d) = self.doc.get_dictionary(id) else {
                    continue;
                };
                let flags = d.get(b"F").ok().and_then(|f| self.num(f)).unwrap_or(0.0) as i64;
                let sub = d.get(b"Subtype").and_then(|o| o.as_name()).unwrap_or(b"");
                if flags & 2 == 0 && hide(flags, sub) {
                    out.push((id, flags));
                }
            }
        }
        out
    }

    pub fn to_bytes(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.write_to(&mut out)?;
        Ok(out)
    }

    /// Writes the document to `path`. It is written to a temporary file in the same folder
    /// first and then renamed over `path`, so a failure part way leaves any existing file
    /// as it was. A symbolic link is followed, and an existing file keeps its permissions;
    /// while it is written, the temporary file is readable only by this user (on Unix).
    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<()> {
        crate::atomic::replace(path.as_ref(), crate::atomic::Access::Keep, |w| {
            self.write_to(w)
        })
    }

    // -------------------------------------------------------------- pages

    pub fn page_count(&self) -> usize {
        self.doc.get_pages().len()
    }

    pub fn page_ids(&self) -> Vec<ObjectId> {
        self.doc.get_pages().into_values().collect()
    }

    pub(crate) fn inherited(&self, id: ObjectId, key: &[u8]) -> Option<Object> {
        let mut cur = id;
        for _ in 0..64 {
            let d = self.doc.get_dictionary(cur).ok()?;
            if let Ok(v) = d.get(key) {
                return Some(v.clone());
            }
            cur = d.get(b"Parent").ok()?.as_reference().ok()?;
        }
        None
    }

    pub(crate) fn num(&self, o: &Object) -> Option<f32> {
        let (_, o) = self.doc.dereference(o).ok()?;
        o.as_float().ok()
    }

    pub(crate) fn rect(&self, o: &Object) -> Option<[f32; 4]> {
        let (_, o) = self.doc.dereference(o).ok()?;
        let a = o.as_array().ok()?;
        if a.len() < 4 {
            return None;
        }
        let v: Vec<f32> = a
            .iter()
            .take(4)
            .map(|x| self.num(x))
            .collect::<Option<_>>()?;
        Some([
            v[0].min(v[2]),
            v[1].min(v[3]),
            v[0].max(v[2]),
            v[1].max(v[3]),
        ])
    }

    /// Visible box (CropBox, else MediaBox, else Letter) of a page in user space.
    pub(crate) fn page_box(&self, id: ObjectId) -> [f32; 4] {
        self.inherited(id, b"CropBox")
            .and_then(|o| self.rect(&o))
            .unwrap_or_else(|| self.media_box(id))
    }

    /// Full box (MediaBox, else Letter) of a page in user space.
    pub(crate) fn media_box(&self, id: ObjectId) -> [f32; 4] {
        self.inherited(id, b"MediaBox")
            .and_then(|o| self.rect(&o))
            .unwrap_or([0.0, 0.0, 612.0, 792.0])
    }

    pub(crate) fn page_rotation(&self, id: ObjectId) -> i64 {
        let r = self
            .inherited(id, b"Rotate")
            .and_then(|o| {
                self.doc
                    .dereference(&o)
                    .ok()
                    .and_then(|(_, o)| o.as_i64().ok())
            })
            .unwrap_or(0);
        r.rem_euclid(360)
    }

    /// Displayed size in points, after applying the page rotation.
    pub fn page_size(&self, index: usize) -> Result<(f32, f32)> {
        let id = *self
            .page_ids()
            .get(index)
            .ok_or_else(|| anyhow!("no page {}", index + 1))?;
        let [x0, y0, x1, y1] = self.page_box(id);
        let (w, h) = (x1 - x0, y1 - y0);
        Ok(if matches!(self.page_rotation(id), 90 | 270) {
            (h, w)
        } else {
            (w, h)
        })
    }

    /// Like [`Pdf::page_size`], for the whole (media) box, as if the page weren't cropped.
    pub fn full_page_size(&self, index: usize) -> Result<(f32, f32)> {
        let id = *self
            .page_ids()
            .get(index)
            .ok_or_else(|| anyhow!("no page {}", index + 1))?;
        let [x0, y0, x1, y1] = self.media_box(id);
        let (w, h) = (x1 - x0, y1 - y0);
        Ok(if matches!(self.page_rotation(id), 90 | 270) {
            (h, w)
        } else {
            (w, h)
        })
    }

    /// Push inherited page attributes down onto each page and flatten the page tree.
    pub(crate) fn flatten(&mut self) -> Result<()> {
        let ids = self.page_ids();
        for &id in &ids {
            for key in INHERITABLE {
                let missing = self.doc.get_dictionary(id)?.get(key).is_err();
                if missing {
                    if let Some(v) = self.inherited(id, key) {
                        self.doc.get_dictionary_mut(id)?.set(key.to_vec(), v);
                    }
                }
            }
        }
        self.set_page_order(&ids)
    }

    fn pages_root(&self) -> Result<ObjectId> {
        Ok(self.doc.catalog()?.get(b"Pages")?.as_reference()?)
    }

    /// Every change to which pages there are, or their order, comes through here. Page labels move with their
    /// pages.
    pub(crate) fn set_page_order(&mut self, ids: &[ObjectId]) -> Result<()> {
        let labels = self.label_specs();
        let root = self.pages_root()?;
        {
            let d = self.doc.get_dictionary_mut(root)?;
            d.set("Type", "Pages");
            d.set(
                "Kids",
                ids.iter()
                    .map(|i| Object::Reference(*i))
                    .collect::<Vec<_>>(),
            );
            d.set("Count", ids.len() as i64);
            d.remove(b"Parent");
        }
        for &id in ids {
            self.doc
                .get_dictionary_mut(id)?
                .set("Parent", Object::Reference(root));
        }
        match labels {
            Some(specs) => self.relabel(ids, &specs),
            None => Ok(()),
        }
    }

    fn check_indices(&self, idx: &[usize]) -> Result<()> {
        let n = self.page_count();
        if let Some(&bad) = idx.iter().find(|&&i| i >= n) {
            bail!("page {} does not exist (document has {n})", bad + 1);
        }
        Ok(())
    }

    pub fn delete_pages(&mut self, idx: &[usize]) -> Result<()> {
        self.check_indices(idx)?;
        let del: HashSet<usize> = idx.iter().copied().collect();
        let ids = self.page_ids();
        let keep: Vec<ObjectId> = ids
            .iter()
            .enumerate()
            .filter(|(i, _)| !del.contains(i))
            .map(|(_, id)| *id)
            .collect();
        if keep.is_empty() {
            bail!("cannot delete every page");
        }
        // Bookmarks to the deleted pages stay, with no destination.
        self.keep_pages(&keep, false)
    }

    /// New document containing only the given pages (in the given order). Nothing of the
    /// other pages is kept: bookmarks, named destinations, links, form fields and the
    /// structure tree no longer refer to them.
    pub fn extract(&self, idx: &[usize]) -> Result<Pdf> {
        self.check_indices(idx)?;
        if idx.is_empty() {
            bail!("no pages selected");
        }
        // Built on a copy without the streams' data, which is copied in only for what the
        // kept pages still use: a whole copy per part made splitting a large file hold
        // many copies of it.
        let mut out = self.without_stream_data();
        let ids = out.page_ids();
        let sel: Vec<ObjectId> = idx.iter().map(|&i| ids[i]).collect();
        out.keep_pages(&sel, true)?;
        out.doc.prune_objects();
        for (id, o) in &mut out.doc.objects {
            if let (Object::Stream(s), Some(Object::Stream(from))) = (o, self.doc.objects.get(id)) {
                if s.content.is_empty() {
                    s.content = from.content.clone();
                }
            }
        }
        Ok(out)
    }

    /// A copy whose streams have their dictionaries but no data.
    fn without_stream_data(&self) -> Pdf {
        let d = &self.doc;
        let objects = d
            .objects
            .iter()
            .map(|(&id, o)| {
                let o = match o {
                    Object::Stream(s) => Object::Stream(Stream {
                        dict: s.dict.clone(),
                        content: Vec::new(),
                        allows_compression: s.allows_compression,
                        start_position: s.start_position,
                    }),
                    o => o.clone(),
                };
                (id, o)
            })
            .collect();
        Pdf {
            doc: Document {
                version: d.version.clone(),
                binary_mark: d.binary_mark.clone(),
                trailer: d.trailer.clone(),
                reference_table: d.reference_table.clone(),
                objects,
                max_id: d.max_id,
                max_bookmark_id: d.max_bookmark_id,
                bookmarks: d.bookmarks.clone(),
                bookmark_table: d.bookmark_table.clone(),
                xref_start: d.xref_start,
                encryption_state: d.encryption_state.clone(),
            },
            encryption: self.encryption.clone(),
            differs: self.differs.clone(),
            repaired: self.repaired,
            password_needed: self.password_needed,
            made_fonts: self.made_fonts.clone(),
        }
    }

    /// Rotate pages clockwise by a multiple of 90 degrees.
    pub fn rotate_pages(&mut self, idx: &[usize], degrees: i64) -> Result<()> {
        if degrees % 90 != 0 {
            bail!("rotation must be a multiple of 90");
        }
        self.check_indices(idx)?;
        self.flatten()?;
        let ids = self.page_ids();
        for &i in idx {
            let cur = self.page_rotation(ids[i]);
            self.doc
                .get_dictionary_mut(ids[i])?
                .set("Rotate", (cur + degrees.rem_euclid(360)).rem_euclid(360));
        }
        Ok(())
    }

    /// Insert a blank page so that it becomes page `after + 1` (`after == 0` inserts first).
    pub fn insert_blank(&mut self, after: usize, width: f32, height: f32) -> Result<()> {
        if after > self.page_count() {
            bail!("position {after} is beyond the end of the document");
        }
        self.flatten()?;
        let contents = self
            .doc
            .add_object(Stream::new(Dictionary::new(), Vec::new()));
        let page = self.doc.add_object(dictionary! {
            "Type" => "Page",
            "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
            "Resources" => Dictionary::new(),
            "Contents" => contents,
        });
        let mut ids = self.page_ids();
        ids.insert(after, page);
        self.set_page_order(&ids)
    }

    /// Set the crop box of a page; `rect` is `[left, bottom, right, top]` in user space.
    pub fn crop_page(&mut self, index: usize, rect: [f32; 4]) -> Result<()> {
        self.check_indices(&[index])?;
        if rect[2] <= rect[0] || rect[3] <= rect[1] {
            bail!("empty crop rectangle");
        }
        let id = self.page_ids()[index];
        let v: Vec<Object> = rect.iter().map(|&f| Object::Real(f)).collect();
        self.doc.get_dictionary_mut(id)?.set("CropBox", v);
        Ok(())
    }

    /// How far a page is cropped in from its full (media) box, in points:
    /// `[top, right, bottom, left]` as the page is displayed, rotation applied.
    pub fn crop_margins(&self, index: usize) -> Result<[f32; 4]> {
        self.check_indices(&[index])?;
        let id = self.page_ids()[index];
        let m = self.media_box(id);
        let [x0, y0, x1, y1] = self.page_box(id);
        let unrotated = [m[3] - y1, m[2] - x1, y0 - m[1], x0 - m[0]];
        Ok(rotate_sides(unrotated, self.page_rotation(id)).map(|v| v.max(0.0)))
    }

    /// Crop a page by margins in points from its full (media) box:
    /// `[top, right, bottom, left]` as the page is displayed, rotation applied. All
    /// zero shows the whole page again.
    pub fn set_crop_margins(&mut self, index: usize, margins: [f32; 4]) -> Result<()> {
        self.check_indices(&[index])?;
        if margins.iter().any(|&v| v < 0.0) {
            bail!("crop margins cannot be negative");
        }
        let id = self.page_ids()[index];
        let back = 360 - self.page_rotation(id);
        let [t, r, b, l] = rotate_sides(margins, back);
        let m = self.media_box(id);
        if m[0] + l >= m[2] - r || m[1] + b >= m[3] - t {
            bail!("the crop margins leave nothing of page {}", index + 1);
        }
        self.crop_page(index, [m[0] + l, m[1] + b, m[2] - r, m[3] - t])
    }

    /// Reorder pages. `order` must be a permutation of `0..page_count`.
    pub fn reorder(&mut self, order: &[usize]) -> Result<()> {
        let n = self.page_count();
        let mut seen = vec![false; n];
        if order.len() != n
            || order
                .iter()
                .any(|&i| i >= n || std::mem::replace(&mut seen[i], true))
        {
            bail!("order must list each of the {n} pages exactly once");
        }
        self.flatten()?;
        let ids = self.page_ids();
        let new: Vec<ObjectId> = order.iter().map(|&i| ids[i]).collect();
        self.set_page_order(&new)
    }

    /// Append the pages of `others` to this document. Their forms aren't merged into this
    /// one's, so their fields can't be filled in here, and values without a drawing of their
    /// own show blank: callers first ask to draw them onto the pages with
    /// [`Pdf::flatten_form`] ([`Pdf::undrawable_fields`] names what it would refuse).
    pub fn append(&mut self, others: Vec<Pdf>) -> Result<()> {
        self.flatten()?;
        let mut ids = self.page_ids();
        for mut o in others {
            o.flatten()?;
            o.doc.renumber_objects_with(self.doc.max_id + 1);
            let new_ids = o.page_ids();
            self.doc.max_id = self.doc.max_id.max(o.doc.max_id);
            self.doc.objects.extend(o.doc.objects);
            ids.extend(new_ids);
        }
        self.set_page_order(&ids)
    }

    /// Merge documents in order into a new one.
    pub fn merge(mut docs: Vec<Pdf>) -> Result<Pdf> {
        if docs.is_empty() {
            bail!("nothing to merge");
        }
        let mut base = docs.remove(0);
        base.append(docs)?;
        Ok(base)
    }

    /// The parts a split makes, in order: each one's title (its first bookmark's, when
    /// split at bookmarks) and its pages (zero-based).
    pub fn split_parts(&self, by: SplitBy) -> Result<Vec<(Option<String>, Vec<usize>)>> {
        match by {
            SplitBy::Every(0) => bail!("part size must be at least 1"),
            SplitBy::Every(n) => {
                let pages: Vec<usize> = (0..self.page_count()).collect();
                Ok(pages.chunks(n).map(|c| (None, c.to_vec())).collect())
            }
            SplitBy::Bookmarks(level) => self.bookmark_split(level),
        }
    }

    /// The files [`Pdf::split`] writes, in order: part `n` (from 1) at `path(n, title)`.
    /// Callers check them with [`existing`] before splitting, as a split replaces files.
    pub fn split_files(
        &self,
        by: SplitBy,
        path: impl Fn(usize, Option<&str>) -> std::path::PathBuf,
    ) -> Result<Vec<std::path::PathBuf>> {
        let parts = self.split_parts(by)?;
        Ok(parts
            .iter()
            .enumerate()
            .map(|(i, (title, _))| path(i + 1, title.as_deref()))
            .collect())
    }

    /// Split into files, part `n` (from 1) written to `path(n, title)`, replacing files of
    /// the same name ([`Pdf::split_files`] lists them). Each part is written and dropped as
    /// soon as it is made, so no more parts are held at once than are being made. Returns
    /// how many files were written. When every file was written but one wasn't confirmed on
    /// disk, the error is that file's [`crate::NotConfirmed`].
    pub fn split(
        &self,
        by: SplitBy,
        path: impl Fn(usize, Option<&str>) -> std::path::PathBuf + Sync,
    ) -> Result<usize> {
        let parts = self.split_parts(by)?;
        let written = std::sync::atomic::AtomicUsize::new(0);
        let unconfirmed = std::sync::Mutex::new(None);
        let done = parts
            .par_iter()
            .enumerate()
            .try_for_each(|(i, (title, pages))| {
                match self.extract(pages)?.save(path(i + 1, title.as_deref())) {
                    // Written: the other parts go on.
                    Err(e) if e.is::<crate::NotConfirmed>() => {
                        let mut u = unconfirmed.lock().unwrap_or_else(|e| e.into_inner());
                        u.get_or_insert(e);
                    }
                    r => r?,
                }
                written.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok::<_, anyhow::Error>(())
            });
        let written = written.into_inner();
        let unconfirmed = unconfirmed.into_inner().unwrap_or_else(|e| e.into_inner());
        match done {
            Ok(()) => unconfirmed.map_or(Ok(parts.len()), Err),
            Err(e) if written > 0 => Err(e.context(format!(
                "{written} of the {} files were written before this",
                parts.len()
            ))),
            Err(e) => Err(e),
        }
    }
}

/// How [`Pdf::split`] divides a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitBy {
    /// Parts of at most this many pages.
    Every(usize),
    /// Where the bookmarks down to this level (1 = top level only) point: one part per
    /// bookmarked page, titled by its first bookmark. Pages before the first bookmark are
    /// a part of their own, untitled.
    Bookmarks(usize),
}

/// The file name for part `n` (1-based) of a split: `stem-003.pdf`, or `stem-003 Title.pdf` with the title cut to
/// 60 characters and kept to what every file system accepts.
pub fn part_file_name(stem: &str, n: usize, title: Option<&str>) -> String {
    let safe: String = title
        .unwrap_or("")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_.,()&'".contains(c) {
                c
            } else {
                ' '
            }
        })
        .collect();
    let safe: String = safe
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(60)
        .collect();
    // Windows drops trailing dots and spaces from names.
    let safe = safe.trim_end_matches(['.', ' ']);
    if safe.is_empty() {
        format!("{stem}-{n:03}.pdf")
    } else {
        format!("{stem}-{n:03} {safe}.pdf")
    }
}

impl Pdf {
    // ----------------------------------------------------------- metadata

    fn info_id(&self) -> Option<ObjectId> {
        self.doc.trailer.get(b"Info").ok()?.as_reference().ok()
    }

    pub fn info(&self) -> Info {
        let dict = match self.doc.trailer.get(b"Info") {
            Ok(o) => self
                .doc
                .dereference(o)
                .ok()
                .and_then(|(_, o)| o.as_dict().ok()),
            Err(_) => None,
        };
        let get = |k: &[u8]| -> Option<String> {
            let o = dict?.get(k).ok()?;
            let (_, o) = self.doc.dereference(o).ok()?;
            o.as_str().ok().map(decode_text)
        };
        Info {
            title: get(b"Title"),
            author: get(b"Author"),
            subject: get(b"Subject"),
            keywords: get(b"Keywords"),
            creator: get(b"Creator"),
            producer: get(b"Producer"),
        }
    }

    /// Set fields that are `Some`; an empty string removes the field.
    pub fn set_info(&mut self, info: &Info) -> Result<()> {
        let id = match self.info_id() {
            Some(id) if self.doc.get_dictionary(id).is_ok() => id,
            _ => {
                let id = self.doc.add_object(Dictionary::new());
                self.doc.trailer.set("Info", Object::Reference(id));
                id
            }
        };
        let d = self.doc.get_dictionary_mut(id)?;
        for (key, val) in [
            ("Title", &info.title),
            ("Author", &info.author),
            ("Subject", &info.subject),
            ("Keywords", &info.keywords),
            ("Creator", &info.creator),
            ("Producer", &info.producer),
        ] {
            if let Some(v) = val {
                if v.is_empty() {
                    d.remove(key.as_bytes());
                } else {
                    d.set(
                        key,
                        Object::String(encode_text(v), lopdf::StringFormat::Literal),
                    );
                }
            }
        }
        Ok(())
    }
}

impl Default for Pdf {
    fn default() -> Self {
        Self::new()
    }
}

fn cannot_open(e: lopdf::Error) -> anyhow::Error {
    anyhow!("cannot open PDF: {e}")
}

/// Sides `[top, right, bottom, left]` of an unrotated page, as they appear once the page
/// is turned clockwise by `rotation` degrees (a multiple of 90).
fn rotate_sides(sides: [f32; 4], rotation: i64) -> [f32; 4] {
    let k = (rotation.rem_euclid(360) / 90) as usize;
    std::array::from_fn(|i| sides[(i + 4 - k) % 4])
}

/// Decode a PDF text string (UTF-16BE with BOM, UTF-8 with BOM, or PDFDocEncoding ~ Latin-1).
/// `bytes` with the newest trailer's `/Encrypt null` entry turned into spaces, or `None`
/// when it has none. A null value means the entry is absent, so this doesn't change the
/// document. Only that trailer is touched (the one lopdf reads `/Encrypt` from), so the
/// same bytes elsewhere, say in a page's text, stay as they are.
fn blank_null_encrypt(bytes: &[u8]) -> Option<Vec<u8>> {
    // The newest trailer is a table's trailer or a cross-reference stream's dictionary:
    // from where the last `startxref` points (counted from the header, as lopdf does) to
    // the next `startxref` or `stream`.
    let header = bytes.windows(5).position(|w| w == b"%PDF-").unwrap_or(0);
    let s = bytes.windows(9).rposition(|w| w == b"startxref")? + 9;
    let digits: Vec<u8> = bytes[s..]
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .take_while(|b| b.is_ascii_digit())
        .take(15)
        .copied()
        .collect();
    let at = header.checked_add(std::str::from_utf8(&digits).ok()?.parse().ok()?)?;
    let rest = bytes.get(at..)?;
    let end = [&b"startxref"[..], b"stream"]
        .iter()
        .filter_map(|k| rest.windows(k.len()).position(|w| w == *k))
        .min()?;
    let trailer = at..at + end;

    let mut copy: Option<Vec<u8>> = None;
    let mut from = trailer.start;
    while let Some(i) = bytes
        .get(from..trailer.end)
        .and_then(|t| t.windows(8).position(|w| w == b"/Encrypt"))
    {
        let start = from + i;
        let mut end = start + 8;
        while bytes.get(end).is_some_and(|b| b" \t\r\n\x0c\0".contains(b)) {
            end += 1;
        }
        let after = bytes.get(end + 4);
        if end + 4 <= trailer.end
            && bytes[end..].starts_with(b"null")
            && after.is_none_or(|b| !b.is_ascii_alphanumeric() && !b"#_.-".contains(b))
        {
            end += 4;
            copy.get_or_insert_with(|| bytes.to_vec())[start..end].fill(b' ');
        }
        from = end;
    }
    copy
}

/// `bytes` with a copy of the encryption dictionary appended as an update, its `/U`
/// padded to 32 bytes, or `None` when that isn't what stops lopdf. At revisions 3 and 4
/// readers compare only the first 16 bytes of `/U`, and some writers store just those
/// (PDFBox's test file 1910); lopdf refuses any length but 32, and then asks for a
/// password the file doesn't have. `/U` has no other use at these revisions, so the
/// padding changes nothing else. Appending keeps every offset in the file as it is.
fn pad_user_hash(bytes: &[u8]) -> Option<Vec<u8>> {
    // Without a password lopdf reads just the trailer and the encryption dictionary.
    let doc =
        Document::load_from_with_options(Cursor::new(bytes), lopdf::LoadOptions::default()).ok()?;
    let (n, g) = doc
        .trailer
        .get(b"Encrypt")
        .and_then(Object::as_reference)
        .ok()?;
    let mut enc = doc.get_dictionary((n, g)).ok()?.clone();
    let standard = enc.get(b"Filter").and_then(Object::as_name).ok() == Some(b"Standard");
    let revision = enc.get(b"R").and_then(Object::as_i64).ok();
    let u = enc.get(b"U").and_then(Object::as_str).ok()?;
    if !standard || !matches!(revision, Some(3 | 4)) || !(16..32).contains(&u.len()) {
        return None;
    }
    let mut padded = u.to_vec();
    padded.resize(32, 0);
    enc.set(
        "U",
        Object::String(padded, lopdf::StringFormat::Hexadecimal),
    );

    // lopdf counts offsets from the header, as in `blank_null_encrypt`.
    let header = bytes.windows(5).position(|w| w == b"%PDF-").unwrap_or(0);
    let mut out = bytes.to_vec();
    out.push(b'\n');
    let mut trailer = Dictionary::new();
    for key in [&b"Size"[..], b"Root", b"Info", b"ID"] {
        if let Ok(v) = doc.trailer.get(key) {
            trailer.set(key, v.clone());
        }
    }
    trailer.set("Encrypt", (n, g));
    trailer.set("Prev", doc.xref_start as i64);
    let at = out.len() - header;
    let mut text = format!("{n} {g} obj\n");
    crate::repair::write_object(&mut text, &Object::Dictionary(enc), 0);
    text.push_str("\nendobj\n");
    out.extend_from_slice(text.as_bytes());
    let xref = out.len() - header;
    let mut text = format!("xref\n{n} 1\n{at:010} {g:05} n\r\ntrailer\n");
    crate::repair::write_object(&mut text, &Object::Dictionary(trailer), 0);
    text.push_str(&format!("\nstartxref\n{xref}\n%%EOF\n"));
    out.extend_from_slice(text.as_bytes());
    Some(out)
}

/// The characters PDFDocEncoding gives bytes 0x18 to 0x1F, and 0x80 to 0xA0 (0x9F is
/// undefined); every other byte means what it does in Latin-1.
const PDF_DOC_18: [char; 8] = ['˘', 'ˇ', 'ˆ', '˙', '˝', '˛', '˚', '˜'];
const PDF_DOC_80: [char; 33] = [
    '•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘', '’', '‚', '™',
    'ﬁ', 'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ', 'š', 'ž', '\u{9f}', '€',
];

/// A text string (properties, bookmarks, field names and values, comments, page labels):
/// UTF-16 or UTF-8 after a byte order mark, else PDFDocEncoding. Bytes PDFDocEncoding
/// leaves undefined (control codes, 0x7F, 0x9F) are kept as the same code points, as
/// pdf.js reads them, so nothing is lost and names differing only there stay different.
pub(crate) fn decode_text(b: &[u8]) -> String {
    if b.starts_with(&[0xFE, 0xFF]) {
        let u: Vec<u16> = b[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        String::from_utf16_lossy(&u)
    } else if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(&b[3..]).into_owned()
    } else {
        b.iter()
            .map(|&c| match c {
                0x18..=0x1F => PDF_DOC_18[usize::from(c - 0x18)],
                0x80..=0xA0 => PDF_DOC_80[usize::from(c - 0x80)],
                _ => c as char,
            })
            .collect()
    }
}

/// Writes `bytes` (such as a signed document) to `path` the way [`Pdf::save`] does: whole
/// or not at all.
pub fn write_file(path: impl AsRef<Path>, bytes: &[u8]) -> Result<()> {
    crate::atomic::replace(path.as_ref(), crate::atomic::Access::Keep, |w| {
        Ok(w.write_all(bytes)?)
    })
}

/// Those of `paths` that something is at already: a file, a folder or a link (a broken
/// one too), which writing there would replace or fail on.
pub fn existing(paths: &[std::path::PathBuf]) -> Vec<&Path> {
    paths
        .iter()
        .filter(|p| std::fs::symlink_metadata(p).is_ok())
        .map(|p| p.as_path())
        .collect()
}

/// A text string for [`decode_text`]: printable ASCII, tabs and line breaks as they are
/// (they mean the same in PDFDocEncoding), anything else as UTF-16.
pub(crate) fn encode_text(s: &str) -> Vec<u8> {
    if s.bytes()
        .all(|c| matches!(c, b' '..=b'~' | b'\t' | b'\n' | b'\r'))
    {
        s.as_bytes().to_vec()
    } else {
        let mut v = vec![0xFE, 0xFF];
        for u in s.encode_utf16() {
            v.extend_from_slice(&u.to_be_bytes());
        }
        v
    }
}

/// Adjust what lopdf writes for an encrypted document so other readers open it:
/// - Strings become hexadecimal. Ciphertext and password hashes are binary, and hayro 0.8
///   reads two line feeds in a row inside a literal string as one, so about one file in
///   2,500 failed its password check there.
/// - AES-256 gets `/Length 256`, which lopdf leaves out; without it poppler uses a short
///   key and shows the pages as garbage.
fn fix_encrypted(d: &mut Document, cipher: crate::Cipher) {
    for o in d.objects.values_mut() {
        hex_strings(o);
    }
    for (_, o) in d.trailer.iter_mut() {
        hex_strings(o);
    }
    if cipher == crate::Cipher::Aes256 {
        let id = d.trailer.get(b"Encrypt").and_then(Object::as_reference);
        let dict = match id {
            Ok(id) => d.get_dictionary_mut(id).ok(),
            Err(_) => d
                .trailer
                .get_mut(b"Encrypt")
                .and_then(Object::as_dict_mut)
                .ok(),
        };
        if let Some(e) = dict {
            e.set("Length", 256);
        }
    }
}

fn hex_strings(o: &mut Object) {
    match o {
        Object::String(_, f) => *f = lopdf::StringFormat::Hexadecimal,
        Object::Array(a) => a.iter_mut().for_each(hex_strings),
        Object::Dictionary(d) => d.iter_mut().for_each(|(_, v)| hex_strings(v)),
        Object::Stream(s) => s.dict.iter_mut().for_each(|(_, v)| hex_strings(v)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(n: usize) -> Pdf {
        let mut p = Pdf::new();
        for i in 0..n {
            p.insert_blank(i, 200.0 + i as f32, 300.0).unwrap();
        }
        p
    }

    #[test]
    fn part_file_names() {
        assert_eq!(part_file_name("book", 3, None), "book-003.pdf");
        assert_eq!(
            part_file_name("book", 12, Some("Chapter 2: Over/Under?")),
            "book-012 Chapter 2 Over Under.pdf"
        );
        assert_eq!(part_file_name("book", 1, Some(" ... ")), "book-001.pdf");
        let long = "x".repeat(100);
        assert_eq!(
            part_file_name("b", 1, Some(&long)).len(),
            "b-001 .pdf".len() + 60
        );
    }

    #[test]
    fn insert_delete_extract() {
        let mut p = sample(5);
        assert_eq!(p.page_count(), 5);
        p.delete_pages(&[0, 4]).unwrap();
        assert_eq!(p.page_count(), 3);
        assert_eq!(p.page_size(0).unwrap(), (201.0, 300.0));
        let e = p.extract(&[2, 0]).unwrap();
        assert_eq!(e.page_count(), 2);
        assert_eq!(e.page_size(0).unwrap(), (203.0, 300.0));
        assert!(p.delete_pages(&[0, 1, 2]).is_err());
    }

    #[test]
    fn rotate_and_size() {
        let mut p = sample(1);
        p.rotate_pages(&[0], 90).unwrap();
        assert_eq!(p.page_size(0).unwrap(), (300.0, 200.0));
        p.rotate_pages(&[0], 270).unwrap();
        assert_eq!(p.page_size(0).unwrap(), (200.0, 300.0));
    }

    #[test]
    fn merge_split_roundtrip() {
        let a = sample(3);
        let b = sample(2);
        let mut m = Pdf::merge(vec![a, b]).unwrap();
        assert_eq!(m.page_count(), 5);
        let bytes = m.to_bytes().unwrap();
        let re = Pdf::from_bytes(&bytes, None).unwrap();
        assert_eq!(re.page_count(), 5);
        let parts = re.split_parts(SplitBy::Every(2)).unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|(_, pages)| re.extract(pages).unwrap().page_count())
                .collect::<Vec<_>>(),
            vec![2, 2, 1]
        );
        assert!(re.split_parts(SplitBy::Every(0)).is_err());
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("limarepdf-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The files a split is checked against are the ones it writes, and [`existing`] finds
    /// the ones already there, a broken link among them (BUG-05).
    #[test]
    fn split_files_are_the_files_written() {
        let dir = scratch("split-files");
        let p = sample(5);
        let name = |n, t: Option<&str>| dir.join(part_file_name("doc", n, t));
        let files = p.split_files(SplitBy::Every(2), name).unwrap();
        assert!(existing(&files).is_empty());
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("gone"), &files[2]).unwrap();
        assert_eq!(p.split(SplitBy::Every(2), name).unwrap(), 3);
        let mut written: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        written.sort();
        assert_eq!(written, files);
        assert_eq!(
            existing(&files),
            files.iter().map(|f| f.as_path()).collect::<Vec<_>>()
        );
        assert!(p.split_files(SplitBy::Every(0), name).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A part that can't be written fails the split; every other part is whole or not there,
    /// and no temporary file is left (BUG-05's mid-run error).
    #[test]
    fn a_failed_part_leaves_no_cut_files() {
        let dir = scratch("split-fails");
        let p = sample(3);
        let name = |n, t: Option<&str>| match n {
            2 => dir.join("missing").join(part_file_name("doc", n, t)),
            _ => dir.join(part_file_name("doc", n, t)),
        };
        let e = p.split(SplitBy::Every(1), name).unwrap_err();
        assert!(format!("{e:#}").contains("missing"), "{e:#}");
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let n = path.file_name().unwrap().to_string_lossy().into_owned();
            assert!(["doc-001.pdf", "doc-003.pdf"].contains(&n.as_str()), "{n}");
            assert_eq!(Pdf::open(&path, None).unwrap().page_count(), 1, "{n}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reorder_and_metadata() {
        let mut p = sample(3);
        p.reorder(&[2, 1, 0]).unwrap();
        assert_eq!(p.page_size(0).unwrap().0, 202.0);
        assert!(p.reorder(&[0, 0, 1]).is_err());
        p.set_info(&Info {
            title: Some("Tést".into()),
            author: Some("A".into()),
            ..Default::default()
        })
        .unwrap();
        let i = p.info();
        assert_eq!(i.title.as_deref(), Some("Tést"));
        assert_eq!(i.author.as_deref(), Some("A"));
    }

    #[test]
    fn only_the_newest_trailers_null_encrypt_is_blanked() {
        let mut b = b"%PDF-1.7\n1 0 obj\n(/Encrypt null)\nendobj\n".to_vec();
        let xref = b.len();
        b.extend_from_slice(
            format!(
                "xref\n0 2\n0000000000 65535 f \n0000000009 00000 n \ntrailer\n<< /Size 2 \
                 /Encrypt null >>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        let copy = blank_null_encrypt(&b).unwrap();
        assert_eq!(copy.len(), b.len());
        let text = String::from_utf8(copy).unwrap();
        assert!(text.contains("(/Encrypt null)"));
        assert!(text.contains("<< /Size 2               >>"));
        // With a stray `startxref` after the end, nothing is blanked.
        b.extend_from_slice(b"startxref\n");
        assert!(blank_null_encrypt(&b).is_none());
    }

    #[test]
    fn text_strings_read_as_pdfdocencoding() {
        // Every character where PDFDocEncoding differs from Latin-1.
        let mut b: Vec<u8> = (0x18..=0x1F).collect();
        b.extend(0x80..=0x9E);
        b.push(0xA0);
        assert_eq!(decode_text(&b), "˘ˇˆ˙˝˛˚˜•†‡…—–ƒ⁄‹›−‰„“”‘’‚™ﬁﬂŁŒŠŸŽıłœšž€");
        assert_eq!(
            decode_text(b"Budget \x80 \xa0100 \x84 \x8dapproved\x8e"),
            "Budget • €100 — “approved”"
        );
        // The rest is Latin-1, line breaks and tabs included.
        assert_eq!(decode_text(b"Caf\xe9 \xa9\t1\r\n2"), "Café ©\t1\r\n2");
        // Bytes PDFDocEncoding leaves undefined are kept as they are, so nothing is lost
        // and names that differ only there stay different.
        assert_eq!(
            decode_text(b"a\x01\x7f\x9f\xadb"),
            "a\u{1}\u{7f}\u{9f}\u{ad}b"
        );
        // Marked strings are read as before.
        assert_eq!(decode_text(b"\xfe\xff\x20\xac\x00!"), "€!");
        assert_eq!(decode_text("\u{feff}€!".as_bytes()), "€!");
    }

    #[test]
    fn text_strings_read_back_as_written() {
        let all: String = (0u32..0x250).filter_map(char::from_u32).collect();
        for s in [
            "plain ASCII",
            "tab\tand\r\nlines",
            "Budget • €100 — “approved”",
            "PDFDocEncoding's accents as control codes: \u{18}\u{1f}",
            "and those it leaves undefined: \u{1}\u{7f}\u{9f}",
            &all,
        ] {
            assert_eq!(decode_text(&encode_text(s)), s, "{s:?}");
        }
        assert_eq!(encode_text("plain\tASCII\r\n"), b"plain\tASCII\r\n");
    }
}
