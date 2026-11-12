//! Compact saving (ENH-19): a save packs objects into object streams with a
//! cross-reference stream and keeps identical streams once, changing nothing on the page;
//! encrypted documents and those claiming PDF/A-1 keep the classic layout.

mod common;

use common::saved::Saved;
use limarepdf_core::{render, Cipher, Encryption, Pdf, Restrictions};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

/// A page of text and a grey square, on each of `n` pages, each page with its own copy of
/// the same picture.
fn document(n: usize) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..n {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let pixels: Vec<u8> = (0..64 * 64).map(|i| (i % 251) as u8).collect();
    for (i, page) in p.page_ids().into_iter().enumerate() {
        let mut picture = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 64, "Height" => 64,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            pixels.clone(),
        );
        picture.compress().unwrap();
        let picture = p.doc.add_object(picture);
        let content = format!(
            "BT /F1 12 Tf 20 160 Td (Page {}) Tj ET q 100 0 0 100 150 40 cm /Im1 Do Q",
            i + 1
        );
        let content = p
            .doc
            .add_object(Stream::new(Dictionary::new(), content.into_bytes()));
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", content);
        d.set(
            "Resources",
            dictionary! {
                "Font" => dictionary! {"F1" => font},
                "XObject" => dictionary! {"Im1" => picture},
            },
        );
    }
    p
}

/// The pictures the file's objects hold.
fn pictures(doc: &lopdf::Document) -> Vec<ObjectId> {
    doc.objects
        .iter()
        .filter(|(_, o)| {
            o.as_stream().is_ok_and(|s| {
                s.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image")
            })
        })
        .map(|(id, _)| *id)
        .collect()
}

/// How many pixels of two drawings differ at all (every pixel when the sizes differ).
fn changed(a: &render::Bitmap, b: &render::Bitmap) -> usize {
    if (a.width, a.height) != (b.width, b.height) {
        return a.width * a.height;
    }
    let (x, y) = (a.rgba.as_chunks::<4>().0, b.rgba.as_chunks::<4>().0);
    x.iter().zip(y).filter(|(p, q)| p != q).count()
}

/// Every page of `a` and `b` drawn at 72 dpi, pixel for pixel the same.
fn same_drawing(a: Vec<u8>, b: Vec<u8>, password: Option<&str>) {
    let (a, b) = (
        render::load(a, password).unwrap(),
        render::load(b, password).unwrap(),
    );
    let (ra, rb) = (render::Renderer::new(&a), render::Renderer::new(&b));
    assert_eq!(ra.page_count(), rb.page_count());
    for i in 0..ra.page_count() {
        let (x, y) = (ra.render(i, 1.0).unwrap(), rb.render(i, 1.0).unwrap());
        assert_eq!(changed(&x, &y), 0, "page {} draws differently", i + 1);
    }
}

/// The document written as saves wrote it before ENH-19: pruned, compressed, a table.
fn classic(p: &Pdf) -> Vec<u8> {
    let mut d = p.doc.clone();
    d.prune_objects();
    d.compress();
    let mut out = Vec::new();
    d.save_to(&mut out).unwrap();
    out
}

#[test]
fn a_save_packs_its_objects_and_reads_back_as_written() {
    // Enough pages that packing is chosen without writing both ways.
    let mut p = document(40);
    let before = classic(&p);
    let saved = Saved::of(&mut p);
    let text = String::from_utf8_lossy(&saved.bytes);
    assert!(text.contains("/ObjStm"), "no object stream");
    assert!(text.contains("/XRef"), "no cross-reference stream");
    assert!(!text.contains("\nxref"), "a cross-reference table");
    assert!(text.starts_with("%PDF-1.5") || text.starts_with("%PDF-1.7"));
    assert!(saved.unreachable().is_empty());
    let back = Pdf::from_bytes(&saved.bytes, None).unwrap();
    assert_eq!(back.page_count(), 40);
    assert_eq!(back.page_text(1).trim(), "Page 2");
    assert!(
        saved.bytes.len() < before.len(),
        "{} ≥ {}",
        saved.bytes.len(),
        before.len()
    );
    same_drawing(before, saved.bytes, None);
}

#[test]
fn identical_pictures_are_kept_once_in_the_file_and_apart_in_the_document() {
    let mut p = document(3);
    let ids = pictures(&p.doc);
    assert_eq!(ids.len(), 3);
    let (version, max_id, trailer) = (p.doc.version.clone(), p.doc.max_id, p.doc.trailer.clone());
    let bytes = p.to_bytes().unwrap();
    // The document keeps each page's own copy, so later edits find them where they were.
    assert_eq!(pictures(&p.doc), ids);
    assert_eq!((p.doc.version.clone(), p.doc.max_id), (version, max_id));
    assert_eq!(p.doc.trailer.len(), trailer.len());
    for page in p.page_ids() {
        let res = p
            .doc
            .get_dictionary(page)
            .unwrap()
            .get(b"Resources")
            .unwrap();
        let xo = res
            .as_dict()
            .unwrap()
            .get(b"XObject")
            .unwrap()
            .as_dict()
            .unwrap();
        assert!(ids.contains(&xo.get(b"Im1").unwrap().as_reference().unwrap()));
    }
    let back = Saved::new(bytes.clone());
    assert_eq!(pictures(&back.doc).len(), 1);
    same_drawing(classic(&p), bytes, None);
}

#[test]
fn an_encrypted_document_saves_one_object_at_a_time_and_reopens() {
    for cipher in [
        Cipher::Rc4_40,
        Cipher::Rc4_128,
        Cipher::Aes128,
        Cipher::Aes256,
    ] {
        let mut p = document(2);
        p.set_encryption(Some(Encryption {
            user_password: "user".into(),
            owner_password: "owner".into(),
            cipher,
            restrictions: Restrictions::default(),
        }));
        let bytes = p.to_bytes().unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("/ObjStm"),
            "{cipher:?}: an object stream in the clear"
        );
        let back = Pdf::from_bytes(&bytes, Some("user")).unwrap();
        assert_eq!(back.page_count(), 2, "{cipher:?}");
        assert_eq!(back.page_text(0).trim(), "Page 1", "{cipher:?}");
        // The pictures are merged in an encrypted file too.
        let mut back = back;
        assert_eq!(pictures(&back.doc).len(), 1, "{cipher:?}");
        back.set_encryption(None);
        same_drawing(classic(&back), bytes, Some("user"));
    }
}

#[test]
fn a_document_claiming_pdfa1_keeps_the_layout_of_pdf_1_4() {
    let mut p = document(1);
    p.doc.version = "1.4".into();
    let xmp = br#"<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/" pdfaid:part="1" pdfaid:conformance="B"/></rdf:RDF></x:xmpmeta><?xpacket end="w"?>"#;
    let meta = p.doc.add_object(Stream::new(
        dictionary! {"Type" => "Metadata", "Subtype" => "XML"},
        xmp.to_vec(),
    ));
    p.doc.catalog_mut().unwrap().set("Metadata", meta);
    let saved = Saved::of(&mut p);
    let text = String::from_utf8_lossy(&saved.bytes);
    assert!(text.starts_with("%PDF-1.4"));
    assert!(text.contains("\nxref"), "no cross-reference table");
    assert!(!text.contains("/ObjStm") && !text.contains("/XRef"));
    // Identical pictures are still kept once: PDF 1.4 allows that.
    assert_eq!(pictures(&saved.doc).len(), 1);

    // PDF/A-2 allows both.
    let mut p = document(1);
    let xmp2 = String::from_utf8_lossy(xmp).replace(r#"part="1""#, r#"part="2""#);
    let meta = p.doc.add_object(Stream::new(
        dictionary! {"Type" => "Metadata", "Subtype" => "XML"},
        xmp2.into_bytes(),
    ));
    p.doc.catalog_mut().unwrap().set("Metadata", meta);
    assert!(String::from_utf8_lossy(&p.to_bytes().unwrap()).contains("/ObjStm"));
}

/// XMP metadata `len` bytes long saying the document is PDF/A part `part`, the part named
/// at the end, after spaces.
fn xmp_of(len: usize, part: u32) -> Vec<u8> {
    let head = br#"<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/""#;
    let tail = format!(
        r#" pdfaid:part="{part}" pdfaid:conformance="B"/></rdf:RDF></x:xmpmeta><?xpacket end="w"?>"#
    );
    let mut x = head.to_vec();
    x.resize(len - tail.len(), b' ');
    x.extend(tail.as_bytes());
    assert_eq!(x.len(), len);
    x
}

/// A one-page document whose metadata is `xmp`, Flate-compressed.
fn with_metadata(xmp: Vec<u8>) -> Pdf {
    let mut p = document(1);
    let mut s = Stream::new(dictionary! {"Type" => "Metadata", "Subtype" => "XML"}, xmp);
    s.compress().unwrap();
    assert!(s.dict.has(b"Filter"));
    let meta = p.doc.add_object(s);
    p.doc.catalog_mut().unwrap().set("Metadata", meta);
    p
}

/// Whether `bytes` have the layout of PDF 1.4: no object streams, a cross-reference table.
fn is_classic(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    text.contains("\nxref") && !text.contains("/ObjStm") && !text.contains("/XRef")
}

/// Metadata is expanded up to [`MAX_XMP`] bytes to learn whether the document is PDF/A-1
/// (M-05): every save expanded it whole, however far that went. (A save compresses every
/// stream first, so this is all metadata.) Past the limit the document is taken to be
/// PDF/A-1, whose layout is valid for any, so the save still succeeds, the same each time;
/// up to it, the part is read as before.
#[test]
fn metadata_is_read_only_so_far() {
    use limarepdf_core::MAX_XMP;
    // Part 2 named past the limit isn't seen.
    let mut p = with_metadata(xmp_of(MAX_XMP + 1, 2));
    let once = p.to_bytes().unwrap();
    assert!(is_classic(&once));
    assert_eq!(p.to_bytes().unwrap(), once);
    assert_eq!(Pdf::from_bytes(&once, None).unwrap().page_count(), 1);
    // Nor part 1.
    let mut p = with_metadata(xmp_of(MAX_XMP + 1, 1));
    assert!(is_classic(&p.to_bytes().unwrap()));
    // Up to the limit, the part is read.
    let mut p = with_metadata(xmp_of(MAX_XMP, 2));
    assert!(!is_classic(&p.to_bytes().unwrap()));
    let mut p = with_metadata(xmp_of(MAX_XMP, 1));
    assert!(is_classic(&p.to_bytes().unwrap()));
}

#[test]
fn a_small_document_is_written_the_smaller_way() {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let bytes = p.to_bytes().unwrap();
    assert!(bytes.len() <= classic(&p).len());
    assert_eq!(Pdf::from_bytes(&bytes, None).unwrap().page_count(), 1);
}

#[test]
fn a_compact_file_saves_compactly_again() {
    let mut p = document(2);
    let once = p.to_bytes().unwrap();
    let mut back = Pdf::from_bytes(&once, None).unwrap();
    let twice = back.to_bytes().unwrap();
    let saved = Saved::new(twice.clone());
    assert!(saved.unreachable().is_empty());
    assert_eq!(pictures(&saved.doc).len(), 1);
    same_drawing(once, twice, None);
}

/// Opt-in: every fetched sample saved compactly and as before, its listed pages drawn the
/// same and the file no larger. `COMPACT_ONLY=name` for some; `COMPACT_EVERY=n` for every
/// n-th. Run with `prlimit --data` (one process).
#[test]
#[ignore = "needs the fetched samples: sh tools/fetch-render-samples.sh"]
fn compact_saving_on_the_corpus() {
    let only = std::env::var("COMPACT_ONLY").unwrap_or_default();
    let every = common::corpus::every("COMPACT_EVERY");
    let samples: Vec<_> = common::corpus::samples(&only)
        .into_iter()
        .filter(|s| s.path().exists())
        .enumerate()
        .filter(|(i, _)| i % every == 0)
        .map(|(_, s)| s)
        .collect();
    assert!(
        !samples.is_empty(),
        "no samples: run sh tools/fetch-render-samples.sh"
    );
    let (mut was, mut now, mut files, mut larger, mut skipped) = (0usize, 0usize, 0, 0, 0);
    let mut problems = Vec::new();
    for s in &samples {
        // Named first, so a file that ends the process is known.
        eprintln!("{}", s.name);
        let bytes = std::fs::read(s.path()).unwrap();
        let Ok(mut p) = Pdf::from_bytes(&bytes, s.password.as_deref()) else {
            skipped += 1;
            continue;
        };
        if p.encrypted() {
            // Written as before, one object at a time.
            skipped += 1;
            continue;
        }
        let old = classic(&p);
        let new = match p.to_bytes() {
            Ok(b) => b,
            Err(e) => {
                problems.push(format!("{}: not written: {e}", s.name));
                continue;
            }
        };
        files += 1;
        was += old.len();
        now += new.len();
        if new.len() > old.len() {
            larger += 1;
            problems.push(format!("{}: {} → {} bytes", s.name, old.len(), new.len()));
        }
        if let Err(e) = Pdf::from_bytes(&new, None) {
            problems.push(format!("{}: doesn't open again: {e}", s.name));
            continue;
        }
        let (Ok(a), Ok(b)) = (render::load(old, None), render::load(new, None)) else {
            problems.push(format!("{}: hayro can't load one of them", s.name));
            continue;
        };
        let (ra, rb) = (render::Renderer::new(&a), render::Renderer::new(&b));
        for page in s.first..=s.last.min(ra.page_count()) {
            // At a width of their own: some samples have pages too large for hayro at 72 dpi.
            match (
                ra.render_to_width(page - 1, 400.0),
                rb.render_to_width(page - 1, 400.0),
            ) {
                (Ok(x), Ok(y)) => {
                    let n = changed(&x, &y);
                    if n > 0 {
                        problems.push(format!("{} p{page}: {n} pixels differ", s.name));
                    }
                }
                (Err(_), Err(_)) => {}
                _ => problems.push(format!("{} p{page}: drawn in one only", s.name)),
            }
        }
    }
    println!(
        "{files} files: {was} → {now} bytes ({:.1}% smaller); {larger} larger; {skipped} skipped",
        100.0 * (1.0 - now as f64 / was.max(1) as f64)
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
