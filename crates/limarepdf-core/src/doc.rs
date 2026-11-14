use crate::forms::Reach;
use crate::pages_gone::DrawnBefore;
use crate::{reach, Result};
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
    /// Streams the copies pages are drawn from found fine (`unfillable.rs`).
    pub(crate) checked: crate::unfillable::Checked,
    /// Whether a caller puts the document back when an operation fails ([`Pdf::holding`]).
    pub(crate) held: crate::history::Held,
}

/// The document's pages in order: the page tree's entries whose object is a page. lopdf also
/// lists an entry whose object is missing or isn't a page (one without `/Type /Page`) as a
/// blank stand-in page. Those are passed over, as the renderer passes over a missing one, so
/// a page number means the same page when editing as on screen, and a page tree with no real
/// page is repaired (see [`Pdf::from_bytes`]).
pub(crate) fn pages(doc: &Document) -> impl Iterator<Item = ObjectId> + '_ {
    page_tree(doc).filter(|&id| {
        doc.get_dictionary(id)
            .and_then(Dictionary::get_type)
            .is_ok_and(|t| t == b"Page")
    })
}

/// Every entry of the page tree in order, stand-ins too, as lopdf's `page_iter` lists them,
/// without its guess at how many there are: that adds up the `/Count` the file gives each
/// branch, so in a damaged file it can be short (counting the pages then stops with an
/// overflow), overflow itself, or be so large that collecting the pages makes room for more
/// memory than there is, which ends the app (PB-173). Used instead of `page_iter` and
/// `get_pages`.
pub(crate) fn page_tree(doc: &Document) -> impl Iterator<Item = ObjectId> + '_ {
    #[allow(clippy::disallowed_methods)] // The one use, its guess hidden.
    let mut tree = doc.page_iter();
    std::iter::from_fn(move || tree.next())
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

/// An A4 page in points (210 by 297 mm), worked out as hayro does: what it draws a page
/// without a media box as.
pub(crate) const A4: (f32, f32) = {
    const MM: f64 = 1.0 / (10.0 * 2.54) * 72.0;
    ((210.0 * MM) as f32, (297.0 * MM) as f32)
};

impl Pdf {
    // ---------------------------------------------------------------- I/O

    pub fn open(path: impl AsRef<Path>, password: Option<&str>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_bytes(&bytes, password)
    }

    /// Puts `bytes`, a copy of this same document from earlier in the session (as
    /// [`Pdf::snapshot`] writes it), in its place. The fonts LimarePDF embedded this
    /// session are kept, so a signed document goes on growing them rather than starting new
    /// ones (PB-07); the copy keeps their ids. On an error nothing changes.
    pub fn reopen(&mut self, bytes: &[u8]) -> Result<()> {
        let made = self.made_fonts.clone();
        *self = Self::from_bytes(bytes, None)?;
        self.made_fonts = made;
        Ok(())
    }

    /// Reads a document. A file that reads with no pages is repaired (see
    /// [`Pdf::repaired`]): its objects are found by scanning the bytes, in case the
    /// cross-reference information is damaged, and a page tree other readers find pages in
    /// is mended. A file in which no pages can be found is refused.
    pub fn from_bytes(bytes: &[u8], password: Option<&str>) -> Result<Self> {
        let opened = Self::read(bytes, password, false);
        let pages = match &opened {
            Ok(p) => p.page_count(),
            Err(lopdf::Error::InvalidPassword) => return opened.map_err(cannot_open),
            Err(_) => 0,
        };
        // lopdf rebuilds a table it can't read at all by scanning, and then has no table
        // offset (a real one can't be 0: the header is there). Its scan ends a stream at the
        // first `endstream` and ignores `/Length`, so LimarePDF's own repair is used
        // instead, and lopdf's only when that fails. So too when lopdf took a table it found
        // near where the file says one is, and the file doesn't read as other apps read it:
        // lopdf then passes over the objects that aren't where that table says.
        let rebuilt_by_lopdf = matches!(&opened, Ok(p) if p.doc.xref_start == 0
            || (p.doc.xref_recovery == Some(lopdf::xref::XrefRecovery::OffsetCorrected)
                && p.differs.is_some()));
        if pages > 0 && !rebuilt_by_lopdf {
            return opened.map_err(cannot_open);
        }
        // Read by the rebuilt table only: lopdf's own rebuild would scan again, its way.
        let rebuilt = match crate::repair::rebuild(bytes).map(|b| Self::read(&b, password, true)) {
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

    /// Reads a document as its cross-reference information says, or, unless `as_written`, as
    /// lopdf rebuilds that information when the catalog doesn't lead to a page tree.
    fn read(
        bytes: &[u8],
        password: Option<&str>,
        as_written: bool,
    ) -> std::result::Result<Self, lopdf::Error> {
        let load = |b: &[u8]| {
            let options = lopdf::LoadOptions {
                filter: as_written.then_some(crate::repair::as_written as lopdf::FilterFunc),
                ..lopdf::LoadOptions::with_password(password.unwrap_or(""))
            };
            Document::load_from_with_options(Cursor::new(b), options)
        };
        let copy;
        let (doc, read) = match load(bytes) {
            Ok(doc) => (doc, bytes),
            // Some writers put `/Encrypt null` (meaning not encrypted) in the trailer, and
            // lopdf takes any `/Encrypt` entry as encryption. Read a copy with those
            // entries blanked; they are the same length, so every offset holds.
            Err(e) => match blank_null_encrypt(bytes).or_else(|| encryption_lopdf_reads(bytes)) {
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
            checked: Default::default(),
            held: Default::default(),
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
            checked: Default::default(),
            held: Default::default(),
        }
    }

    /// Writes the document as a save does: unused objects removed, streams compressed,
    /// identical streams kept once, and the rest packed into object streams with a
    /// cross-reference stream (ENH-19). An encrypted document's objects are written one by
    /// one, as lopdf can't encrypt an object stream it makes, and a document claiming PDF/A-1
    /// keeps the layout of PDF 1.4. The document is left as it was, its unused objects
    /// removed and its streams compressed.
    fn write_to<W: Write>(&mut self, w: &mut W) -> Result<()> {
        self.doc.prune_objects();
        self.doc.compress();
        match &self.encryption {
            // Encryption must be the last step: compressing ciphertext would corrupt streams.
            Some(enc) => {
                let mut d = self.doc.clone();
                // A copy: nothing to put back.
                crate::compact::merge_identical(&mut d);
                if d.trailer.get(b"ID").is_err() {
                    // The standard security handler derives keys from the file identifier.
                    let id = crate::pkcs12::random::<16>()?;
                    let s = Object::String(id.to_vec(), lopdf::StringFormat::Hexadecimal);
                    d.trailer.set("ID", vec![s.clone(), s]);
                }
                let state = enc.state(&d)?;
                d.encrypt(&state)?;
                fix_encrypted(&mut d, enc.cipher);
                d.save_to(w)?;
            }
            None => self.write_compact(w)?,
        }
        Ok(())
    }

    /// Writes the document compactly (see [`Pdf::write_to`]), leaving it as it was: lopdf's
    /// writer changes the version, the highest object number, the trailer and the kind of
    /// cross-reference data, which are put back with the merged streams. A document of few
    /// objects is written both ways in memory and the smaller kept: an object stream's own
    /// entries can outweigh what packing a handful of objects saves.
    fn write_compact<W: Write>(&mut self, w: &mut W) -> Result<()> {
        let merged = crate::compact::merge_identical(&mut self.doc);
        let kept = (
            self.doc.version.clone(),
            self.doc.max_id,
            self.doc.trailer.clone(),
            self.doc.reference_table.cross_reference_type,
        );
        let written = if crate::compact::claims_pdfa1(&self.doc) {
            self.write_classic(w)
        } else if crate::compact::few_to_pack(&self.doc) {
            let mut packed = Vec::new();
            self.write_packed(&mut packed).and_then(|()| {
                // As saves wrote it before ENH-19, by the document's own kind of table.
                self.put_header(&kept);
                let mut plain = Vec::new();
                self.doc.save_to(&mut plain)?;
                let smaller = if plain.len() < packed.len() {
                    plain
                } else {
                    packed
                };
                w.write_all(&smaller)
            })
        } else {
            self.write_packed(w)
        };
        self.put_header(&kept);
        crate::compact::put_back(&mut self.doc, merged);
        written?;
        Ok(())
    }

    /// The version, highest object number, trailer and kind of cross-reference data put back
    /// after lopdf's writer changed them.
    fn put_header(&mut self, kept: &(String, u32, Dictionary, lopdf::xref::XrefType)) {
        self.doc.version = kept.0.clone();
        self.doc.max_id = kept.1;
        self.doc.trailer = kept.2.clone();
        self.doc.reference_table.cross_reference_type = kept.3;
    }

    /// Writes every object on its own, with a cross-reference table (PDF 1.4's layout).
    fn write_classic<W: Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
        self.doc.save_to(w)
    }

    /// Writes the objects packed into object streams, with a cross-reference stream.
    fn write_packed<W: Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        let options = lopdf::SaveOptions {
            use_object_streams: true,
            use_xref_streams: true,
            ..Default::default()
        };
        self.doc.save_with_options(w, options)
    }

    /// Whether the document was read from an encrypted (password-protected) file, or is
    /// set to be saved with a password.
    pub fn encrypted(&self) -> bool {
        self.encryption.is_some()
            || self.doc.encryption_state.is_some()
            || self.doc.trailer.has(b"Encrypt")
    }

    /// Serialise without pruning, compressing or encrypting, for copies where speed matters
    /// more than size. The document is left as it was: lopdf's writer takes a new object
    /// number for a cross-reference stream and writes the table's keys into the trailer.
    pub fn snapshot(&mut self) -> Result<Vec<u8>> {
        let (max_id, trailer) = (self.doc.max_id, self.doc.trailer.clone());
        let mut out = Vec::new();
        let written = self.doc.save_to(&mut out);
        (self.doc.max_id, self.doc.trailer) = (max_id, trailer);
        written?;
        Ok(out)
    }

    /// Like [`Pdf::snapshot`], for drawing: a picture whose stated size its data can't come
    /// near filling, which would make the renderer ask for gigabytes, is left out of the
    /// copy, and reported as a picture that can't be decoded when the page is drawn (PB-81);
    /// so is a tiling pattern the renderer can't tile (PB-11). A shape filled with a pattern
    /// of a triangle or patch shading is drawn as the shading clipped to the shape, which
    /// the renderer samples only where the shading is (PB-11). An annotation in a layer
    /// that's off is marked Hidden, as the renderer would draw it (PB-181). The document
    /// itself is not changed.
    pub fn drawing_snapshot(&mut self) -> Result<Vec<u8>> {
        let swaps = crate::unfillable::swaps(&self.doc, &mut self.checked);
        let ids = self.annots_where(|_| false);
        if swaps.is_empty() && ids.is_empty() {
            return self.snapshot();
        }
        let mut copy = self.clone();
        crate::unfillable::put(&mut copy.doc, swaps);
        copy.mark_hidden(ids);
        copy.snapshot()
    }

    /// Like [`Pdf::snapshot`], with unused objects removed first, as a save removes them:
    /// for copies written to disk. Redacting and removing hidden information leave what
    /// they took out in objects nothing uses any more, which only pruning drops.
    pub fn pruned_snapshot(&mut self) -> Result<Vec<u8>> {
        self.doc.prune_objects();
        self.snapshot()
    }

    /// Like [`Pdf::snapshot`], for drawing pages as the app shows them (`limarepdf-cli
    /// render`): annotations flagged Hidden or NoView aren't drawn, as hayro 0.9 honours both
    /// (`annot_noview_flag_is_respected` in `tests/render_coverage.rs`), and nor are those in
    /// a layer that's off, which it doesn't (PB-181). Form fields are drawn
    /// as [`Pdf::refresh_appearances`] would draw them, as the app draws them when opening a
    /// file, but every one, also past the fields LimarePDF lists ([`Pdf::unshown_fields`]
    /// names those it can't draw). Pictures their data can't
    /// fill and patterns the renderer can't tile are left out, as in
    /// [`Pdf::drawing_snapshot`]. The document itself is not changed.
    pub fn render_snapshot(&mut self) -> Result<Vec<u8>> {
        self.snapshot_hiding(|_| false, true)
    }

    /// Like [`Pdf::render_snapshot`], for the app, which draws sticky notes itself at a
    /// fixed size on screen: notes are left out too.
    pub fn app_snapshot(&mut self) -> Result<Vec<u8>> {
        self.snapshot_hiding(|sub| sub == b"Text", false)
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

    /// A snapshot in which annotations whose subtype `hide` picks, and those in a layer
    /// that's off (which the renderer would draw, PB-181), are marked Hidden, and
    /// annotations without a drawing get LimarePDF's (the document itself keeps
    /// none). In a signed document, or in any with `all_fields`, form fields are drawn this
    /// way too, as [`Pdf::refresh_appearances`] would draw them (every field with
    /// `all_fields`, those LimarePDF lists for the app's): the app doesn't draw a signed
    /// document's fields into it when opening it, so that saving adds only the edits made to
    /// what was signed. Pictures their data can't fill and patterns the renderer
    /// can't tile are left out, as in [`Pdf::drawing_snapshot`].
    fn snapshot_hiding(
        &mut self,
        hide: impl Fn(&[u8]) -> bool,
        all_fields: bool,
    ) -> Result<Vec<u8>> {
        let ids = self.annots_where(hide);
        let missing = self.missing_appearances(&self.page_ids());
        let reach = if all_fields {
            Reach::Every
        } else {
            Reach::Listed
        };
        let fields = (all_fields || self.signature_count() > 0)
            && !self
                .appearances_to_draw(self.field_nodes(reach).0)
                .is_empty();
        let swaps = crate::unfillable::swaps(&self.doc, &mut self.checked);
        if ids.is_empty() && missing.is_empty() && !fields && swaps.is_empty() {
            return self.snapshot();
        }
        let mut copy = self.clone();
        crate::unfillable::put(&mut copy.doc, swaps);
        if fields {
            copy.refresh_reaching(reach);
        }
        copy.draw_annotations(&missing);
        copy.mark_hidden(ids);
        let mut out = Vec::new();
        copy.doc.save_to(&mut out)?;
        Ok(out)
    }

    /// Sets the Hidden flag (bit 2) of annotations `ids`, given with their flags.
    fn mark_hidden(&mut self, ids: Vec<(ObjectId, i64)>) {
        for (id, flags) in ids {
            if let Ok(a) = self.doc.get_dictionary_mut(id) {
                a.set("F", flags | 2);
            }
        }
    }

    /// Annotations not Hidden (bit 2) whose subtype `hide` picks or that are in a layer
    /// that's off, with their flags.
    fn annots_where(&self, hide: impl Fn(&[u8]) -> bool) -> Vec<(ObjectId, i64)> {
        let mut layers = self.layers_off();
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
                if flags & 2 == 0
                    && (hide(sub) || (layers.as_mut()).is_some_and(|l| self.annot_layer_off(d, l)))
                {
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
        self.save_expecting(path, None)
    }

    /// [`Pdf::save`], over a file expected to be as `expect` found it: when something else
    /// changed it while this was written, it isn't replaced, and the error is a
    /// [`ChangedOnDisk`](crate::ChangedOnDisk).
    pub fn save_expecting(
        &mut self,
        path: impl AsRef<Path>,
        expect: Option<&crate::disk::Version>,
    ) -> Result<()> {
        crate::atomic::replace(path.as_ref(), crate::atomic::Access::Keep, expect, |w| {
            self.write_to(w)
        })
    }

    // -------------------------------------------------------------- pages

    pub fn page_count(&self) -> usize {
        pages(&self.doc).count()
    }

    pub fn page_ids(&self) -> Vec<ObjectId> {
        pages(&self.doc).collect()
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

    /// Visible box of a page in user space: its CropBox where it overlaps the MediaBox, as
    /// the PDF standard reduces a crop box reaching past it and as hayro draws the page
    /// (BUG-43); an overlap with no area is an A4 page from its corner, as hayro's.
    pub(crate) fn page_box(&self, id: ObjectId) -> [f32; 4] {
        let media = self.media_box(id);
        let Some(crop) = self.inherited(id, b"CropBox").and_then(|o| self.rect(&o)) else {
            return media;
        };
        let (x0, y0) = (crop[0].max(media[0]), crop[1].max(media[1]));
        let (x1, y1) = (crop[2].min(media[2]).max(x0), crop[3].min(media[3]).max(y0));
        if x1 - x0 < 1e-3 || y1 - y0 < 1e-3 {
            return [x0, y0, x0 + A4.0, y0 + A4.1];
        }
        [x0, y0, x1, y1]
    }

    /// Full box of a page in user space: its MediaBox, else A4, as hayro draws a page
    /// without one (BUG-43).
    pub(crate) fn media_box(&self, id: ObjectId) -> [f32; 4] {
        self.inherited(id, b"MediaBox")
            .and_then(|o| self.rect(&o))
            .unwrap_or([0.0, 0.0, A4.0, A4.1])
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
        let id = self.page_id_at(index)?;
        Ok(self.shown_size(id, self.page_box(id)))
    }

    /// Every page's [`Pdf::page_size`], in order, with the pages listed once.
    pub fn page_sizes(&self) -> Vec<(f32, f32)> {
        self.page_ids()
            .into_iter()
            .map(|id| self.shown_size(id, self.page_box(id)))
            .collect()
    }

    /// Like [`Pdf::page_size`], for the whole (media) box, as if the page weren't cropped.
    pub fn full_page_size(&self, index: usize) -> Result<(f32, f32)> {
        let id = self.page_id_at(index)?;
        Ok(self.shown_size(id, self.media_box(id)))
    }

    /// Every page's [`Pdf::full_page_size`], in order, with the pages listed once.
    pub fn full_page_sizes(&self) -> Vec<(f32, f32)> {
        self.page_ids()
            .into_iter()
            .map(|id| self.shown_size(id, self.media_box(id)))
            .collect()
    }

    /// The size of page `id`'s box `[x0, y0, x1, y1]` as shown: turned with the page.
    fn shown_size(&self, id: ObjectId, [x0, y0, x1, y1]: [f32; 4]) -> (f32, f32) {
        let (w, h) = (x1 - x0, y1 - y0);
        if matches!(self.page_rotation(id), 90 | 270) {
            (h, w)
        } else {
            (w, h)
        }
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

    /// Delete the given pages. Nothing of them is kept: bookmarks, named destinations, links,
    /// form fields and the structure tree no longer refer to them, and what only they drew
    /// goes from what they share with the others. Refused, with nothing changed, when the
    /// file would still keep something only they held (PB-137).
    pub fn delete_pages(&mut self, idx: &[usize]) -> Result<()> {
        self.check_indices(idx)?;
        let del: HashSet<usize> = idx.iter().copied().collect();
        let ids = self.page_ids();
        let (left, keep): (Vec<usize>, Vec<ObjectId>) = (ids.iter().enumerate())
            .filter(|(i, _)| !del.contains(i))
            .map(|(i, id)| (i, *id))
            .unzip();
        if keep.is_empty() {
            bail!("cannot delete every page");
        }
        self.atomically(|p| {
            let mut cache = reach::Decoded::new();
            let before = p.drawn_before(&mut cache)?;
            // Bookmarks to the deleted pages stay, with no destination.
            let removed = p.keep_pages(&keep, false)?;
            p.clear_gone(removed, &before, &mut cache, "the pages", &left)
        })
    }

    /// New document containing only the given pages (in the given order). Nothing of the
    /// other pages is kept: bookmarks, named destinations, links, form fields and the
    /// structure tree no longer refer to them, and what only they drew goes from what they
    /// share with these. Refused when the new document would still keep something only they
    /// held (PB-137).
    pub fn extract(&self, idx: &[usize]) -> Result<Pdf> {
        self.check_indices(idx)?;
        let mut cache = reach::Decoded::new();
        let before = self.drawn_before(&mut cache)?;
        self.extract_with(idx, Some((&before, &mut cache)))
    }

    /// [`Pdf::extract`], given what the pages draw ([`Pdf::drawn_before`], with what each
    /// drawing's content uses when that was kept); or with no check of what the other pages
    /// leave, for printing, which makes sheets of the pages to draw them and keeps nothing.
    pub(crate) fn extract_with(
        &self,
        idx: &[usize],
        check: Option<(&DrawnBefore, &mut reach::Decoded)>,
    ) -> Result<Pdf> {
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
        let removed = out.keep_pages(&sel, true)?;
        let used: HashSet<ObjectId> = out.doc.traverse_objects(|_| {}).into_iter().collect();
        for (id, o) in &mut out.doc.objects {
            if let (Object::Stream(s), Some(Object::Stream(from))) = (o, self.doc.objects.get(id)) {
                if s.content.is_empty() && used.contains(id) {
                    s.content = from.content.clone();
                }
            }
        }
        if let Some((before, cache)) = check {
            out.clear_gone(removed, before, cache, "the other pages", idx)?;
        }
        out.doc.prune_objects();
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
                xref_recovery: d.xref_recovery,
                encryption_state: d.encryption_state.clone(),
                linearization: d.linearization.clone(),
                xref_section_count: d.xref_section_count,
            },
            encryption: self.encryption.clone(),
            differs: self.differs.clone(),
            repaired: self.repaired,
            password_needed: self.password_needed,
            made_fonts: self.made_fonts.clone(),
            checked: self.checked.clone(),
            held: Default::default(),
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
        Ok(self.margins_of(self.page_ids()[index]))
    }

    /// Every page's [`Pdf::crop_margins`], in order, with the pages listed once.
    pub fn all_crop_margins(&self) -> Vec<[f32; 4]> {
        let ids = self.page_ids();
        ids.into_iter().map(|id| self.margins_of(id)).collect()
    }

    fn margins_of(&self, id: ObjectId) -> [f32; 4] {
        let m = self.media_box(id);
        let [x0, y0, x1, y1] = self.page_box(id);
        let unrotated = [m[3] - y1, m[2] - x1, y0 - m[1], x0 - m[0]];
        rotate_sides(unrotated, self.page_rotation(id)).map(|v| v.max(0.0))
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

    /// Append the pages of `others` to this document, with their bookmarks after its own
    /// and their named destinations (see [`Pdf::insert_pages`]). Their forms aren't merged
    /// into this one's, so their fields can't be filled in here, and values without a
    /// drawing of their own show blank: callers first ask to draw them onto the pages with
    /// [`Pdf::flatten_form`] ([`Pdf::undrawable_fields`] names what it would refuse).
    pub fn append(&mut self, others: Vec<Pdf>) -> Result<()> {
        self.insert_pages(self.page_count(), others)
    }

    /// Insert the pages of `others`, in order, so that the first becomes page `at`
    /// (0-based; the page count puts them at the end). Their bookmarks come too, at the top
    /// level in page order: after the last bookmark there going to a page before `at`. So
    /// do their named destinations, one taken already renamed (`intro-2`) along with what
    /// goes to it. Their forms aren't merged, as for [`Pdf::append`]. Refused, with nothing
    /// changed, when it fails.
    pub fn insert_pages(&mut self, at: usize, others: Vec<Pdf>) -> Result<()> {
        if at > self.page_count() {
            bail!("position {at} is beyond the end of the document");
        }
        self.atomically(|p| p.put_pages(at, others))
    }

    fn put_pages(&mut self, at: usize, others: Vec<Pdf>) -> Result<()> {
        self.flatten()?;
        let mut ids = self.page_ids();
        let mut carried = self.carrying(at);
        let mut added = Vec::new();
        for mut o in others {
            o.flatten()?;
            o.doc.renumber_objects_with(self.doc.max_id + 1);
            o.carry(&mut carried)?;
            let new_ids = o.page_ids();
            self.doc.max_id = self.doc.max_id.max(o.doc.max_id);
            self.doc.objects.extend(o.doc.objects);
            added.extend(new_ids);
        }
        ids.splice(at..at, added);
        self.set_page_order(&ids)?;
        self.put_carried(carried)
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
    /// the same name ([`Pdf::split_files`] lists them). Every part is made and checked as
    /// [`Pdf::extract`] checks it before any file is written, so a part that would keep
    /// something of the other pages writes none (PB-137). Then each part is made again,
    /// written and dropped, so no more parts are held at once than are being made. Returns
    /// how many files were written. When every file was written but one wasn't confirmed on
    /// disk, the error is that file's [`crate::NotConfirmed`].
    pub fn split(
        &self,
        by: SplitBy,
        path: impl Fn(usize, Option<&str>) -> std::path::PathBuf + Sync,
    ) -> Result<usize> {
        let parts = self.split_parts(by)?;
        // Walked once for every part.
        let before = self.drawn_before(&mut reach::Decoded::new())?;
        parts
            .par_iter()
            .enumerate()
            .try_for_each(|(i, (_, pages))| {
                let mut cache = reach::Decoded::new();
                self.extract_with(pages, Some((&before, &mut cache)))
                    .map(drop)
                    .with_context(|| {
                        format!(
                            "part {} of {} can't be made, so no file was written",
                            i + 1,
                            parts.len()
                        )
                    })
            })?;
        let written = std::sync::atomic::AtomicUsize::new(0);
        let unconfirmed = std::sync::Mutex::new(None);
        let done = parts
            .par_iter()
            .enumerate()
            .try_for_each(|(i, (title, pages))| {
                let mut cache = reach::Decoded::new();
                let mut part = self.extract_with(pages, Some((&before, &mut cache)))?;
                match part.save(path(i + 1, title.as_deref())) {
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
            o.as_str().ok().map(decode_shown_text)
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

/// A protected document was opened with no password, or not with one of its own (L-04).
/// It is a type of its own so that the apps ask for the password for this, not for any
/// error whose words mention one (a missing file named password-guide.pdf).
#[derive(Debug)]
pub struct NeedsPassword;

impl std::fmt::Display for NeedsPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot open PDF: {}", lopdf::Error::InvalidPassword)
    }
}

impl std::error::Error for NeedsPassword {}

fn cannot_open(e: lopdf::Error) -> anyhow::Error {
    match e {
        lopdf::Error::InvalidPassword => NeedsPassword.into(),
        e => anyhow!("cannot open PDF: {e}"),
    }
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

/// `bytes` with a copy of the encryption dictionary appended as an update in a form lopdf
/// reads, or `None` when the dictionary's form isn't what stops lopdf. Appending keeps every
/// offset in the file as it is. Two forms stop it:
/// - The trailer holding the dictionary itself (pdf.js's test files issue6010_1 and _2):
///   lopdf reads one only through a reference, and then refuses every password, the right
///   one too (PB-183). The copy is an object of its own, numbered after every other.
/// - At revisions 3 and 4 readers compare only the first 16 bytes of `/U`, and some writers
///   store just those (PDFBox's test file 1910); lopdf refuses any length but 32, and then
///   asks for a password the file doesn't have. `/U` has no other use at these revisions,
///   so the copy's is padded, which changes nothing else.
fn encryption_lopdf_reads(bytes: &[u8]) -> Option<Vec<u8>> {
    // Without a password lopdf reads just the trailer and the encryption dictionary.
    let doc =
        Document::load_from_with_options(Cursor::new(bytes), lopdf::LoadOptions::default()).ok()?;
    let size = doc.trailer.get(b"Size").and_then(Object::as_i64).ok();
    let ((n, g), mut enc, mut changed) = match doc.trailer.get(b"Encrypt").ok()? {
        Object::Reference(id) => (*id, doc.get_dictionary(*id).ok()?.clone(), false),
        Object::Dictionary(d) => {
            let size = size.and_then(|s| u32::try_from(s).ok()).unwrap_or(0);
            let n = size.max(doc.reference_table.max_id().max(doc.max_id) + 1);
            ((n, 0), d.clone(), true)
        }
        _ => return None,
    };
    let standard = enc.get(b"Filter").and_then(Object::as_name).ok() == Some(b"Standard");
    let revision = enc.get(b"R").and_then(Object::as_i64).ok();
    let short = (enc.get(b"U").and_then(Object::as_str).ok())
        .filter(|u| standard && matches!(revision, Some(3 | 4)) && (16..32).contains(&u.len()))
        .map(<[u8]>::to_vec);
    if let Some(mut padded) = short {
        padded.resize(32, 0);
        enc.set(
            "U",
            Object::String(padded, lopdf::StringFormat::Hexadecimal),
        );
        changed = true;
    }
    if !changed {
        return None;
    }

    // lopdf counts offsets from the header, as in `blank_null_encrypt`.
    let header = bytes.windows(5).position(|w| w == b"%PDF-").unwrap_or(0);
    let mut out = bytes.to_vec();
    out.push(b'\n');
    let mut trailer = Dictionary::new();
    for key in [&b"Root"[..], b"Info", b"ID"] {
        if let Ok(v) = doc.trailer.get(key) {
            trailer.set(key, v.clone());
        }
    }
    trailer.set("Size", size.unwrap_or(0).max(i64::from(n) + 1));
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

/// [`decode_text`] for text that is only shown (properties, bookmark titles, comments, field
/// descriptions), never used to find something: unmarked bytes that are valid UTF-8 with a
/// character of more than one byte are read as UTF-8, as the program that wrote them meant
/// (PB-18). In PDFDocEncoding such bytes are runs like "Ã©", which text hardly ever holds.
/// Windows-1252 can't be told from PDFDocEncoding this way, so it reads by the standard.
pub(crate) fn decode_shown_text(b: &[u8]) -> String {
    let marked = b.starts_with(&[0xFE, 0xFF]) || b.starts_with(&[0xEF, 0xBB, 0xBF]);
    match std::str::from_utf8(b) {
        Ok(s) if !marked && !s.is_ascii() => s.to_owned(),
        _ => decode_text(b),
    }
}

/// Writes `bytes` (such as a signed document) to `path` the way [`Pdf::save`] does: whole
/// or not at all.
pub fn write_file(path: impl AsRef<Path>, bytes: &[u8]) -> Result<()> {
    write_file_expecting(path, bytes, None)
}

/// [`write_file`], over a file expected to be as `expect` found it, as
/// [`Pdf::save_expecting`].
pub fn write_file_expecting(
    path: impl AsRef<Path>,
    bytes: &[u8],
    expect: Option<&crate::disk::Version>,
) -> Result<()> {
    crate::atomic::replace(path.as_ref(), crate::atomic::Access::Keep, expect, |w| {
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
/// - Strings become hexadecimal. Ciphertext and password hashes are binary, and hayro before
///   0.9 read two line feeds in a row inside a literal string as one, so about one file in
///   2,500 failed its password check there. Hexadecimal is safe from any reader with that slip.
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

    /// Writing a copy, for drawing the pages or a recovery copy, leaves the document as it was:
    /// lopdf's writer takes a new object number for a cross-reference stream and writes the
    /// table's keys into the document's own trailer (batch 43).
    #[test]
    fn snapshots_leave_the_document_as_it_was() {
        use lopdf::xref::XrefType;
        let mut p = sample(2);
        for xref in [
            XrefType::CrossReferenceStream,
            XrefType::CrossReferenceTable,
        ] {
            p.doc.reference_table.cross_reference_type = xref;
            let (max_id, trailer) = (p.doc.max_id, p.doc.trailer.clone());
            let first = p.snapshot().unwrap();
            p.app_snapshot().unwrap();
            assert!(p.snapshot().unwrap() == first, "{xref:?}");
            assert_eq!(p.doc.max_id, max_id, "{xref:?}");
            assert!(p.doc.trailer == trailer, "{xref:?}: {:?}", p.doc.trailer);
        }
    }

    #[test]
    fn rotate_and_size() {
        let mut p = sample(1);
        p.rotate_pages(&[0], 90).unwrap();
        assert_eq!(p.page_size(0).unwrap(), (300.0, 200.0));
        p.rotate_pages(&[0], 270).unwrap();
        assert_eq!(p.page_size(0).unwrap(), (200.0, 300.0));
    }

    /// Another document's pages go in at the place asked for, in their order: first, in
    /// the middle and at the end; past the end is refused, with nothing changed.
    #[test]
    fn insert_pages_puts_them_where_asked() {
        let widths = |p: &Pdf| -> Vec<f32> {
            (0..p.page_count())
                .map(|i| p.page_size(i).unwrap().0)
                .collect()
        };
        // sample(n)'s page i is 200 + i points wide.
        let mut p = sample(3);
        p.insert_pages(1, vec![sample(2)]).unwrap();
        assert_eq!(widths(&p), [200.0, 200.0, 201.0, 201.0, 202.0]);
        p.insert_pages(0, vec![sample(1)]).unwrap();
        assert_eq!(widths(&p)[..2], [200.0, 200.0]);
        let n = p.page_count();
        p.insert_pages(n, vec![sample(1), sample(2)]).unwrap();
        assert_eq!(widths(&p)[n..], [200.0, 200.0, 201.0]);
        let bytes = p.to_bytes().unwrap();
        assert_eq!(Pdf::from_bytes(&bytes, None).unwrap().page_count(), 9);
        assert!(p.insert_pages(10, vec![sample(1)]).is_err());
        assert_eq!(p.page_count(), 9);
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
