//! Compact saving (ENH-19): a saved file keeps identical streams once (a picture or font
//! put in twice, as merging files or inserting pages often does) and packs its other
//! objects into object streams with a cross-reference stream. Nothing on the page changes.
//!
//! The merging is undone once the file is written ([`put_back`]), so the document in memory
//! keeps each object it had: edits that follow a save find every object where it was.

use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Most rounds of merging: a round can make streams identical that weren't (two pictures
/// whose masks were merged), so merging goes on until a round finds nothing.
const MAX_ROUNDS: usize = 8;

/// The most bytes compressed XMP metadata is expanded to when a save looks for the PDF/A
/// part in it ([`claims_pdfa1`]): metadata is a few kilobytes; a file's could expand to
/// gigabytes on every save.
pub const MAX_XMP: usize = 4 << 20;

/// What [`merge_identical`] changed, to put back.
#[derive(Default)]
pub(crate) struct Merged {
    /// The streams taken out, each a copy of one kept.
    removed: Vec<(ObjectId, Object)>,
    /// The objects whose references were changed, as they were before.
    changed: HashMap<ObjectId, Object>,
    /// The trailer as it was, when a reference in it was changed.
    trailer: Option<Dictionary>,
}

#[cfg(test)]
impl Merged {
    /// How many streams were merged into others.
    fn count(&self) -> usize {
        self.removed.len()
    }
}

/// Whether two streams' dictionaries say the same, `/Length` aside (it may be a reference).
fn same_dict(a: &Dictionary, b: &Dictionary) -> bool {
    let keys = |d: &Dictionary| d.iter().filter(|(k, _)| k.as_slice() != b"Length").count();
    keys(a) == keys(b)
        && a.iter()
            .filter(|(k, _)| k.as_slice() != b"Length")
            .all(|(k, v)| b.get(k).is_ok_and(|w| w == v))
}

/// Streams that are written afresh or never written: lopdf makes new object streams and
/// cross-reference streams, and leaves out a linearization's.
fn written_afresh(d: &Dictionary) -> bool {
    matches!(
        d.get(b"Type").and_then(Object::as_name),
        Ok(b"ObjStm" | b"XRef")
    )
}

/// One round: each stream with an earlier identical one, mapped to the first of them.
fn duplicates(doc: &Document) -> HashMap<ObjectId, ObjectId> {
    let mut groups: HashMap<(usize, u64), Vec<ObjectId>> = HashMap::new();
    for (&id, o) in &doc.objects {
        let Object::Stream(s) = o else { continue };
        if written_afresh(&s.dict) {
            continue;
        }
        let mut h = DefaultHasher::new();
        s.content.hash(&mut h);
        groups
            .entry((s.content.len(), h.finish()))
            .or_default()
            .push(id);
    }
    let mut map = HashMap::new();
    for ids in groups.into_values().filter(|g| g.len() > 1) {
        // The objects are in order, so the lowest number is kept.
        let mut kept: Vec<ObjectId> = Vec::new();
        for id in ids {
            let Ok(Object::Stream(s)) = doc.get_object(id) else {
                continue;
            };
            let first = kept.iter().copied().find(|k| {
                matches!(doc.get_object(*k), Ok(Object::Stream(t))
                    if t.content == s.content && same_dict(&t.dict, &s.dict))
            });
            match first {
                Some(k) => {
                    map.insert(id, k);
                }
                None => kept.push(id),
            }
        }
    }
    map
}

/// `o` with every reference in `map` pointed at its stand-in; whether any was.
fn repoint(o: &mut Object, map: &HashMap<ObjectId, ObjectId>) -> bool {
    match o {
        Object::Reference(r) => match map.get(r) {
            Some(&to) => {
                *r = to;
                true
            }
            None => false,
        },
        Object::Array(a) => a.iter_mut().fold(false, |c, o| repoint(o, map) | c),
        Object::Dictionary(d) => repoint_dict(d, map),
        Object::Stream(s) => repoint_dict(&mut s.dict, map),
        _ => false,
    }
}

fn repoint_dict(d: &mut Dictionary, map: &HashMap<ObjectId, ObjectId>) -> bool {
    d.iter_mut().fold(false, |c, (_, o)| repoint(o, map) | c)
}

/// Whether `o` refers to any of `ids`.
fn refers(o: &Object, ids: &HashMap<ObjectId, ObjectId>) -> bool {
    match o {
        Object::Reference(r) => ids.contains_key(r),
        Object::Array(a) => a.iter().any(|o| refers(o, ids)),
        Object::Dictionary(d) => d.iter().any(|(_, o)| refers(o, ids)),
        Object::Stream(s) => s.dict.iter().any(|(_, o)| refers(o, ids)),
        _ => false,
    }
}

/// Keep each identical stream once: every reference to a copy goes to the first, and the
/// copies are taken out. [`put_back`] undoes it.
pub(crate) fn merge_identical(doc: &mut Document) -> Merged {
    let mut merged = Merged::default();
    for _ in 0..MAX_ROUNDS {
        let map = duplicates(doc);
        if map.is_empty() {
            break;
        }
        for (&id, o) in doc.objects.iter_mut() {
            if map.contains_key(&id) || !refers(o, &map) {
                continue;
            }
            merged.changed.entry(id).or_insert_with(|| o.clone());
            repoint(o, &map);
        }
        if doc.trailer.iter().any(|(_, o)| refers(o, &map)) {
            merged.trailer.get_or_insert_with(|| doc.trailer.clone());
            repoint_dict(&mut doc.trailer, &map);
        }
        for id in map.keys() {
            if let Some(o) = doc.objects.remove(id) {
                merged.removed.push((*id, o));
            }
        }
    }
    merged
}

/// Undo [`merge_identical`]: the copies back, and every changed object as it was.
pub(crate) fn put_back(doc: &mut Document, merged: Merged) {
    // A copy taken out in a later round may have had its references changed in an earlier
    // one: the values from before any change are put in last.
    doc.objects.extend(merged.removed);
    doc.objects.extend(merged.changed);
    if let Some(t) = merged.trailer {
        doc.trailer = t;
    }
}

/// Fewer objects than this to pack, and the document is written both ways to keep the
/// smaller: below it an object stream's own entries can outweigh what packing saves
/// (three corpus files of 4 to 6 objects grew by 9 to 31 bytes).
const FEW: usize = 32;

/// Whether the document has fewer than [`FEW`] objects an object stream could take.
pub(crate) fn few_to_pack(doc: &Document) -> bool {
    doc.objects
        .iter()
        .filter(|(id, o)| id.1 == 0 && !matches!(o, Object::Stream(_)))
        .take(FEW)
        .count()
        < FEW
}

/// Whether the document says it is PDF/A-1 (its XMP metadata's `pdfaid:part` is 1). PDF/A-1
/// is built on PDF 1.4, which has neither object streams nor cross-reference streams.
/// Compressed metadata (all of it, on a save, which compresses streams first) is expanded
/// to at most [`MAX_XMP`] bytes; past that the document is taken to be PDF/A-1, as its
/// layout is valid for any document. Metadata that can't be expanded is looked at as it is
/// stored.
pub(crate) fn claims_pdfa1(doc: &Document) -> bool {
    let Some(s) = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Metadata").ok())
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_stream().ok())
    else {
        return false;
    };
    if !s.dict.has(b"Filter") {
        return pdfa_part(&s.content) == Some(1);
    }
    match s.decompressed_content_with_limit(MAX_XMP) {
        Ok(xmp) => pdfa_part(&xmp) == Some(1),
        Err(lopdf::Error::Decompress(lopdf::DecompressError::MemoryLimitExceeded { .. })) => true,
        Err(_) => pdfa_part(&s.content) == Some(1),
    }
}

/// The PDF/A part named in XMP metadata: as an attribute (`pdfaid:part="1"`) or an element
/// (`<pdfaid:part>1</pdfaid:part>`).
fn pdfa_part(xmp: &[u8]) -> Option<u32> {
    const KEY: &[u8] = b"pdfaid:part";
    let mut from = 0;
    while let Some(at) = xmp[from..].windows(KEY.len()).position(|w| w == KEY) {
        let rest = &xmp[from + at + KEY.len()..];
        from += at + KEY.len();
        let rest = &rest[rest.iter().position(|b| !b.is_ascii_whitespace())?..];
        let value = match rest.first() {
            Some(b'=') => {
                let r = rest[1..].trim_ascii_start();
                match r.first() {
                    Some(b'"' | b'\'') => &r[1..],
                    _ => continue,
                }
            }
            Some(b'>') => rest[1..].trim_ascii_start(),
            _ => continue,
        };
        let digits: Vec<u8> = value
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .copied()
            .collect();
        if let Some(n) = std::str::from_utf8(&digits)
            .ok()
            .and_then(|d| d.parse().ok())
        {
            return Some(n);
        }
    }
    None
}

/// The objects reachable from the trailer, for tests.
#[cfg(test)]
fn reachable(doc: &mut Document) -> std::collections::HashSet<ObjectId> {
    doc.traverse_objects(|_| {}).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Stream};

    fn picture(doc: &mut Document, data: &[u8]) -> ObjectId {
        doc.add_object(Stream::new(
            dictionary! {"Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8},
            data.to_vec(),
        ))
    }

    #[test]
    fn identical_streams_are_kept_once_and_put_back() {
        let mut doc = Document::with_version("1.7");
        let a = picture(&mut doc, b"\x10");
        let b = picture(&mut doc, b"\x10");
        let c = picture(&mut doc, b"\x20");
        let holder = doc.add_object(dictionary! {"Im1" => a, "Im2" => b, "Im3" => c});
        doc.trailer.set("Root", holder);
        let before = doc.objects.clone();
        let merged = merge_identical(&mut doc);
        assert_eq!(merged.count(), 1);
        assert!(!doc.objects.contains_key(&b));
        let d = doc.get_dictionary(holder).unwrap();
        assert_eq!(d.get(b"Im2").unwrap().as_reference().unwrap(), a);
        assert_eq!(d.get(b"Im3").unwrap().as_reference().unwrap(), c);
        // The holder, the first picture and the third.
        assert_eq!(reachable(&mut doc).len(), 3);
        put_back(&mut doc, merged);
        assert!(doc.objects == before);
    }

    #[test]
    fn pictures_become_identical_once_their_masks_are_merged() {
        let mut doc = Document::with_version("1.7");
        let m1 = picture(&mut doc, b"\x01");
        let m2 = picture(&mut doc, b"\x01");
        let p = |doc: &mut Document, m: ObjectId| {
            doc.add_object(Stream::new(
                dictionary! {"Subtype" => "Image", "Width" => 1, "Height" => 1, "SMask" => m},
                b"\x05".to_vec(),
            ))
        };
        let p1 = p(&mut doc, m1);
        let p2 = p(&mut doc, m2);
        let holder = doc.add_object(dictionary! {"A" => p1, "B" => p2});
        doc.trailer.set("Root", holder);
        let before = (doc.objects.clone(), doc.trailer.clone());
        let merged = merge_identical(&mut doc);
        assert_eq!(merged.count(), 2);
        assert_eq!(reachable(&mut doc).len(), 3);
        put_back(&mut doc, merged);
        assert!(doc.objects == before.0);
        assert_eq!(doc.trailer.len(), before.1.len());
    }

    #[test]
    fn different_dictionaries_are_not_merged() {
        let mut doc = Document::with_version("1.7");
        let a = picture(&mut doc, b"\x10");
        let b = doc.add_object(Stream::new(
            dictionary! {"Subtype" => "Image", "Width" => 2, "Height" => 1},
            b"\x10".to_vec(),
        ));
        let holder = doc.add_object(dictionary! {"A" => a, "B" => b});
        doc.trailer.set("Root", holder);
        assert_eq!(merge_identical(&mut doc).count(), 0);
    }

    #[test]
    fn reads_the_pdfa_part_as_attribute_or_element() {
        assert_eq!(
            pdfa_part(br#"<rdf:Description pdfaid:part="1" pdfaid:conformance="B"/>"#),
            Some(1)
        );
        assert_eq!(pdfa_part(b"<pdfaid:part>2</pdfaid:part>"), Some(2));
        assert_eq!(pdfa_part(b"<pdfaid:part >\n 1 </pdfaid:part>"), Some(1));
        assert_eq!(
            pdfa_part(b"xmlns:pdfaid='http://www.aiim.org/pdfa/ns/id/' pdfaid:part = '3'"),
            Some(3)
        );
        assert_eq!(pdfa_part(b"no such key"), None);
    }
}
