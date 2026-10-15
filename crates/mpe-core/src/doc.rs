use crate::Result;
use anyhow::{anyhow, bail, Context};
use lopdf::dictionary;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use rayon::prelude::*;
use std::collections::HashSet;
use std::io::{BufWriter, Cursor, Write};
use std::path::Path;

/// A PDF document opened for editing.
#[derive(Clone)]
pub struct Pdf {
    pub doc: Document,
    pub(crate) encryption: Option<crate::Encryption>,
}

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

    pub fn from_bytes(bytes: &[u8], password: Option<&str>) -> Result<Self> {
        let doc = Document::load_from_with_options(
            Cursor::new(bytes),
            lopdf::LoadOptions::with_password(password.unwrap_or("")),
        )
        .map_err(|e| anyhow!("cannot open PDF: {e}"))?;
        Ok(Self {
            doc,
            encryption: None,
        })
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

    /// Serialise without pruning, compressing or encrypting. Used for undo snapshots, where
    /// speed matters more than size.
    pub fn snapshot(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.doc.save_to(&mut out)?;
        Ok(out)
    }

    /// Like [`Pdf::snapshot`], for on-screen rendering. hayro honours the Hidden annotation
    /// flag but not NoView, so annotations flagged NoView are marked Hidden in the copy that
    /// is serialised. The document itself is not changed.
    pub fn render_snapshot(&mut self) -> Result<Vec<u8>> {
        self.snapshot_hiding(|flags, _| flags & 32 != 0)
    }

    /// Like [`Pdf::render_snapshot`], for the app, which draws sticky notes itself at a
    /// fixed size on screen: notes are left out too.
    pub fn app_snapshot(&mut self) -> Result<Vec<u8>> {
        self.snapshot_hiding(|flags, sub| flags & 32 != 0 || sub == b"Text")
    }

    /// Like [`Pdf::render_snapshot`], for printing: annotations without the Print flag
    /// (bit 3) are marked Hidden instead.
    pub fn print_snapshot(&mut self) -> Result<Vec<u8>> {
        self.snapshot_hiding(|flags, _| flags & 4 == 0)
    }

    /// A snapshot in which annotations whose flags and subtype match `hide` are marked
    /// Hidden, and annotations without a drawing get MPE's (the document itself keeps
    /// none).
    fn snapshot_hiding(&mut self, hide: impl Fn(i64, &[u8]) -> bool) -> Result<Vec<u8>> {
        let ids = self.annots_where(hide);
        let missing = self.missing_appearances(&self.page_ids());
        if ids.is_empty() && missing.is_empty() {
            return self.snapshot();
        }
        let mut copy = self.clone();
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

    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let f =
            std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
        let mut w = BufWriter::with_capacity(1 << 20, f);
        self.write_to(&mut w)?;
        w.flush()?;
        Ok(())
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
    fn media_box(&self, id: ObjectId) -> [f32; 4] {
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

    /// Push inherited page attributes down onto each page and flatten the page tree.
    fn flatten(&mut self) -> Result<()> {
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

    fn set_page_order(&mut self, ids: &[ObjectId]) -> Result<()> {
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
        Ok(())
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
        self.flatten()?;
        self.set_page_order(&keep)
    }

    /// New document containing only the given pages (in the given order).
    pub fn extract(&self, idx: &[usize]) -> Result<Pdf> {
        self.check_indices(idx)?;
        if idx.is_empty() {
            bail!("no pages selected");
        }
        let mut out = self.clone();
        out.flatten()?;
        let ids = out.page_ids();
        let sel: Vec<ObjectId> = idx.iter().map(|&i| ids[i]).collect();
        out.set_page_order(&sel)?;
        Ok(out)
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
                .set("Rotate", (cur + degrees).rem_euclid(360));
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

    /// Append the pages of `others` to this document.
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

    /// Split into parts of at most `n` pages each.
    pub fn split_every(&self, n: usize) -> Result<Vec<Pdf>> {
        if n == 0 {
            bail!("part size must be at least 1");
        }
        let total = self.page_count();
        let chunks: Vec<Vec<usize>> = (0..total)
            .collect::<Vec<_>>()
            .chunks(n)
            .map(|c| c.to_vec())
            .collect();
        chunks.par_iter().map(|c| self.extract(c)).collect()
    }

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

/// Sides `[top, right, bottom, left]` of an unrotated page, as they appear once the page
/// is turned clockwise by `rotation` degrees (a multiple of 90).
fn rotate_sides(sides: [f32; 4], rotation: i64) -> [f32; 4] {
    let k = (rotation.rem_euclid(360) / 90) as usize;
    std::array::from_fn(|i| sides[(i + 4 - k) % 4])
}

/// Decode a PDF text string (UTF-16BE with BOM, UTF-8 with BOM, or PDFDocEncoding ~ Latin-1).
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
        b.iter().map(|&c| c as char).collect()
    }
}

pub(crate) fn encode_text(s: &str) -> Vec<u8> {
    if s.is_ascii() {
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
        let parts = re.split_every(2).unwrap();
        assert_eq!(
            parts.iter().map(|p| p.page_count()).collect::<Vec<_>>(),
            vec![2, 2, 1]
        );
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
}
