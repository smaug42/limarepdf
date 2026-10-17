//! The appended-revision writer: saves edits as a revision appended to the file as it was
//! opened. Every earlier byte stays as it was, so earlier signatures still cover exactly
//! what their signers signed, and checkers can list what changed since. Signing uses it;
//! see docs/signatures.md.
//!
//! Each object of the edited document is compared with the opened file, and only those
//! that differ are written. Losing an edit is the risk, so the result is read back twice
//! before it is handed out: MPE's strict reader of cross-reference sections checks the new
//! revision, and lopdf's reading of the whole result must give every edited object as it
//! is in memory. Anything else and nothing is written.

use crate::{sig_changes, Pdf, Result};
use anyhow::{anyhow, bail};
use lopdf::{Dictionary, Document, IncrementalDocument, Object, ObjectId};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A file with a revision appended.
pub(crate) struct Appended {
    pub bytes: Vec<u8>,
    /// Where each object written in the new revision starts.
    pub offsets: HashMap<ObjectId, usize>,
}

/// Entries of the trailer carried into the new revision. The rest are written afresh
/// (`/Prev`, `/Size`, a cross-reference stream's own entries) or left out (a hybrid
/// file's `/XRefStm`, which belongs to the section it was in).
const TRAILER_KEYS: [&[u8]; 3] = [b"Root", b"Info", b"ID"];

/// `original` (the file as it was opened, unencrypted) with a revision appended holding
/// every object of `edited` that differs from it. Objects the edits deleted are written
/// as `null` when something still refers to them, so they read as gone.
pub(crate) fn append_revision(original: &[u8], edited: &Document) -> Result<Appended> {
    let base = Pdf::from_bytes(original, None)?.doc;
    if base.encryption_state.is_some() || base.trailer.has(b"Encrypt") {
        bail!("MPE can't add to encrypted documents");
    }
    if let Some(why) = base_problem(original, &base) {
        bail!("{why}");
    }
    let startxref = base.xref_start;
    if !edited.trailer.has(b"Root") {
        bail!("the document has no catalog");
    }

    let mut changed: BTreeMap<ObjectId, Object> = BTreeMap::new();
    for (id, obj) in &edited.objects {
        if is_machinery(obj) {
            continue;
        }
        if !base
            .objects
            .get(id)
            .is_some_and(|old| same_stored(old, obj))
        {
            changed.insert(*id, obj.clone());
        }
    }
    let used = referenced(edited);
    let nulled: Vec<ObjectId> = base
        .objects
        .iter()
        .filter(|(id, o)| !edited.objects.contains_key(id) && used.contains(id) && !is_machinery(o))
        .map(|(id, _)| *id)
        .collect();
    for id in &nulled {
        changed.insert(*id, Object::Null);
    }
    let written: Vec<ObjectId> = changed.keys().copied().collect();
    let top = written.iter().map(|id| id.0).max().unwrap_or(0);

    let mut inc = IncrementalDocument::create_from(original.to_vec(), base);
    let new = &mut inc.new_document;
    let mut trailer = Dictionary::new();
    for key in TRAILER_KEYS {
        if let Ok(v) = edited.trailer.get(key) {
            trailer.set(key, v.clone());
        }
    }
    trailer.set("Prev", Object::Integer(i64::try_from(startxref)?));
    new.trailer = trailer;
    new.objects = changed;
    new.max_id = new.max_id.max(edited.max_id).max(top);
    let mut bytes = Vec::with_capacity(original.len() + 16 * 1024);
    inc.save_to(&mut bytes)
        .map_err(|e| anyhow!("writing the new revision failed: {e}"))?;

    // Read it back before handing it out.
    let lost = |why: String| {
        anyhow!("the new revision didn't read back correctly ({why}); nothing was written")
    };
    if !bytes.starts_with(original) {
        return Err(lost("earlier bytes changed".into()));
    }
    let offsets = sig_changes::check_appended(&bytes, original.len(), &written).map_err(lost)?;
    let back = Pdf::from_bytes(&bytes, None)
        .map_err(|e| lost(format!("{e:#}")))?
        .doc;
    for (id, obj) in &edited.objects {
        if !is_machinery(obj) && !back.objects.get(id).is_some_and(|b| same_stored(b, obj)) {
            return Err(lost(format!("object {} differs", id.0)));
        }
    }
    for id in &nulled {
        if !matches!(back.objects.get(id), None | Some(Object::Null)) {
            return Err(lost(format!("object {} is still there", id.0)));
        }
    }
    Ok(Appended { bytes, offsets })
}

/// Why nothing can be appended to `original` (lopdf's reading of it is `base`), if
/// something can't. lopdf repairs some damage when it opens a file (a wrong `startxref`
/// value, a broken cross-reference table), and a revision appended to the repaired reading
/// would point at the damage, so only files lopdf read as written are built on. A file
/// without signatures can be written out in full instead.
pub(crate) fn base_problem(original: &[u8], base: &Document) -> Option<String> {
    match sig_changes::last_startxref(original) {
        None => Some(
            "the file doesn't end with a complete cross-reference pointer (it may be damaged \
             or have data after its end)"
                .into(),
        ),
        Some(at) if at == 0 || base.xref_start != at => Some(
            "the file's cross-reference information is damaged (MPE repaired it to open the \
             file)"
                .into(),
        ),
        Some(_) => None,
    }
}

/// Object streams and cross-reference streams: how a file stores objects, never written
/// as objects themselves (lopdf leaves them out of an appended revision too).
fn is_machinery(o: &Object) -> bool {
    matches!(o, Object::Stream(s) if s.dict.get(b"Type").and_then(Object::as_name)
        .is_ok_and(|t| t == b"ObjStm" || t == b"XRef"))
}

/// Whether two objects hold the same once written and read again: a stream by its
/// dictionary and its bytes (not where lopdf found it in the file), a string by its bytes
/// (not hex or literal), and a whole number the same as an integer or a real (lopdf
/// writes the real 150.0 as `150`, which reads back as an integer).
pub(crate) fn same_stored(a: &Object, b: &Object) -> bool {
    same_at(a, b, 0)
}

fn same_at(a: &Object, b: &Object, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let dicts = |x: &Dictionary, y: &Dictionary| {
        x.len() == y.len()
            && x.iter()
                .all(|(k, v)| y.get(k).is_ok_and(|w| same_at(v, w, depth + 1)))
    };
    match (a, b) {
        (Object::Stream(x), Object::Stream(y)) => x.content == y.content && dicts(&x.dict, &y.dict),
        (Object::Dictionary(x), Object::Dictionary(y)) => dicts(x, y),
        (Object::Array(x), Object::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_at(p, q, depth + 1))
        }
        (Object::String(x, _), Object::String(y, _)) => x == y,
        (Object::Real(r), Object::Integer(i)) | (Object::Integer(i), Object::Real(r)) => {
            r.fract() == 0.0 && *r as i64 == *i
        }
        _ => a == b,
    }
}

/// Every object something in the document refers to.
fn referenced(doc: &Document) -> HashSet<ObjectId> {
    fn walk(o: &Object, depth: usize, out: &mut HashSet<ObjectId>) {
        if depth > MAX_DEPTH {
            return;
        }
        match o {
            Object::Reference(id) => {
                out.insert(*id);
            }
            Object::Array(a) => a.iter().for_each(|x| walk(x, depth + 1, out)),
            Object::Dictionary(d) => d.iter().for_each(|(_, x)| walk(x, depth + 1, out)),
            Object::Stream(s) => s.dict.iter().for_each(|(_, x)| walk(x, depth + 1, out)),
            _ => {}
        }
    }
    let mut out = HashSet::new();
    doc.trailer.iter().for_each(|(_, x)| walk(x, 0, &mut out));
    doc.objects.values().for_each(|o| walk(o, 0, &mut out));
    out
}

/// How deep [`referenced`] looks into nested arrays and dictionaries.
const MAX_DEPTH: usize = 256;

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    fn base() -> Vec<u8> {
        let mut p = Pdf::new();
        p.insert_blank(0, 300.0, 200.0).unwrap();
        p.snapshot().unwrap()
    }

    #[test]
    fn appends_only_what_changed() {
        let original = base();
        let mut p = Pdf::from_bytes(&original, None).unwrap();
        let page = p.page_ids()[0];
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("UserUnit", Object::Real(1.0));
        let new = p
            .doc
            .add_object(dictionary! { "Note" => Object::Real(2.5) });
        let out = append_revision(&original, &p.doc).unwrap();
        assert!(out.bytes.starts_with(&original));
        let mut written: Vec<ObjectId> = out.offsets.keys().copied().collect();
        written.sort();
        assert_eq!(written, [page, new]);
        // Nothing changed: an empty revision.
        let same = Pdf::from_bytes(&original, None).unwrap();
        assert!(append_revision(&original, &same.doc)
            .unwrap()
            .offsets
            .is_empty());
    }

    #[test]
    fn a_deleted_object_still_used_reads_as_gone() {
        let original = base();
        let mut p = Pdf::from_bytes(&original, None).unwrap();
        let page = p.page_ids()[0];
        let contents = p
            .doc
            .get_dictionary(page)
            .unwrap()
            .get(b"Contents")
            .unwrap()
            .as_reference()
            .unwrap();
        p.doc.objects.remove(&contents);
        let out = append_revision(&original, &p.doc).unwrap();
        let back = Pdf::from_bytes(&out.bytes, None).unwrap();
        assert!(matches!(
            back.doc.objects.get(&contents),
            None | Some(Object::Null)
        ));
    }

    #[test]
    fn nothing_is_handed_out_that_reads_back_differently() {
        let original = base();
        let mut p = Pdf::from_bytes(&original, None).unwrap();
        // lopdf writes NaN as `NaN`, which doesn't read back as the same number.
        p.doc
            .add_object(dictionary! { "X" => Object::Real(f32::NAN) });
        let e = append_revision(&original, &p.doc).err().expect("refused");
        assert!(format!("{e:#}").contains("didn't read back"), "{e:#}");
    }

    #[test]
    fn numbers_compare_by_value() {
        assert!(same_stored(&Object::Real(150.0), &Object::Integer(150)));
        assert!(!same_stored(&Object::Real(150.5), &Object::Integer(150)));
        let lit = Object::String(b"a".to_vec(), lopdf::StringFormat::Literal);
        let hex = Object::String(b"a".to_vec(), lopdf::StringFormat::Hexadecimal);
        assert!(same_stored(&lit, &hex));
        assert!(!same_stored(&Object::Integer(1), &Object::Boolean(true)));
    }
}
