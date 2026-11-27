//! What changed after signing, and whether the signer allowed it: the allowed-changes
//! analysis (Acrobat's rules), the shadow and incremental-saving attack shapes, malformed
//! appended revisions, and the signed version of each signature.

// lopdf's page list, of files made here (clippy.toml).
#![allow(clippy::disallowed_methods)]

use crate::sigkit;

use limarepdf_core::{
    verify_signatures, Change, ChangeKind, DigitalSignature, Pdf, Permissions, Refusal, SigStatus,
    SignatureReport, TrustList,
};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use sigkit::*;

fn report(b: &[u8]) -> SignatureReport {
    verify_signatures(b, None, &TrustList::default()).unwrap()
}

/// The signature in field `name`.
fn sig(b: &[u8], name: &str) -> DigitalSignature {
    report(b)
        .signatures
        .into_iter()
        .find(|s| s.fields.iter().any(|f| f == name))
        .unwrap_or_else(|| panic!("no signature in {name}"))
}

/// The changes after the signature in `Signature1`, as (kind, refusal).
fn changes(b: &[u8]) -> Vec<(ChangeKind, Option<Refusal>)> {
    summary(&sig(b, "Signature1").changes)
}

fn summary(c: &[Change]) -> Vec<(ChangeKind, Option<Refusal>)> {
    c.iter().map(|c| (c.kind.clone(), c.refused)).collect()
}

#[track_caller]
fn assert_refused(b: &[u8], want: &ChangeKind) {
    let got = changes(b);
    assert!(
        got.iter().any(|(k, r)| k == want && r.is_some()),
        "expected {want:?} refused, got {got:#?}"
    );
    assert!(!report(b).all_ok());
}

// ---------------------------------------------------------------- building documents

/// Edits the latest version in an appended revision.
fn edit(b: &[u8], f: impl FnOnce(&Document, &mut Document)) -> Vec<u8> {
    append_update(b, |inc| {
        let prev = inc.get_prev_documents().clone();
        f(&prev, &mut inc.new_document);
    })
}

/// Page 1 turned by `rotate` degrees, or not turned, in an appended revision.
fn turned(b: &[u8], rotate: Option<i64>) -> Vec<u8> {
    edit(b, |prev, new| {
        let mut d = dict(prev, page(prev));
        match rotate {
            Some(r) => d.set("Rotate", r),
            None => _ = d.remove(b"Rotate"),
        }
        new.set_object(page(prev), d);
    })
}

fn root(d: &Document) -> ObjectId {
    d.trailer.get(b"Root").unwrap().as_reference().unwrap()
}

fn page(d: &Document) -> ObjectId {
    *d.get_pages().get(&1).unwrap()
}

fn dict(d: &Document, id: ObjectId) -> Dictionary {
    d.get_object(id).unwrap().as_dict().unwrap().clone()
}

/// The object of the field (or annotation) titled `name`, in it or in an object of its own.
fn titled(d: &Document, name: &str) -> ObjectId {
    *d.objects
        .iter()
        .find(|(_, o)| {
            o.as_dict()
                .ok()
                .and_then(|o| o.get(b"T").ok())
                .and_then(|t| d.dereference(t).ok())
                .and_then(|(_, t)| t.as_str().ok())
                .is_some_and(|t| t == name.as_bytes())
        })
        .unwrap()
        .0
}

fn appearance(new: &mut Document, content: &str) -> ObjectId {
    new.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
        },
        content.as_bytes().to_vec(),
    ))
}

/// The page's annotations with `add` appended.
fn add_annot(prev: &Document, new: &mut Document, add: ObjectId) {
    let p = page(prev);
    let mut d = dict(prev, p);
    let mut annots = d
        .get(b"Annots")
        .ok()
        .and_then(|a| a.as_array().ok())
        .cloned()
        .unwrap_or_default();
    annots.push(add.into());
    d.set("Annots", annots);
    new.set_object(p, Object::Dictionary(d));
}

/// The base document with text fields named `names`.
fn form(names: &[&str]) -> Vec<u8> {
    edit(&base_pdf(), |prev, new| {
        let p = page(prev);
        let mut fields: Vec<Object> = Vec::new();
        for (i, n) in names.iter().enumerate() {
            let ap = appearance(new, "");
            let y = 100 + 30 * i as i64;
            let id = new.add_object(dictionary! {
                "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
                "T" => Object::string_literal(*n), "F" => 4, "P" => p,
                "Rect" => vec![150.into(), y.into(), 250.into(), (y + 20).into()],
                "AP" => dictionary! { "N" => ap },
            });
            fields.push(id.into());
        }
        let mut d = dict(prev, p);
        d.set("Annots", fields.clone());
        new.set_object(p, Object::Dictionary(d));
        let r = root(prev);
        let mut c = dict(prev, r);
        c.set(
            "AcroForm",
            dictionary! { "Fields" => fields, "DA" => Object::string_literal("/Helv 0 Tf 0 g") },
        );
        new.set_object(r, Object::Dictionary(c));
    })
}

/// Fills in the text field `name` with `value` and a new appearance.
fn fill(b: &[u8], name: &str, value: &str) -> Vec<u8> {
    edit(b, |prev, new| {
        let id = titled(prev, name);
        let mut d = dict(prev, id);
        let ap = appearance(new, &format!("BT ({value}) Tj ET"));
        d.set("V", Object::string_literal(value));
        d.set("AP", dictionary! { "N" => ap });
        new.set_object(id, Object::Dictionary(d));
    })
}

/// Adds a note on page 1, with `extra` entries.
fn comment(b: &[u8], extra: Dictionary) -> Vec<u8> {
    edit(b, |prev, new| {
        let mut d = dictionary! {
            "Type" => "Annot", "Subtype" => "Text", "Contents" => Object::string_literal("Hi"),
            "Rect" => vec![10.into(), 10.into(), 30.into(), 30.into()], "P" => page(prev),
        };
        d.extend(&extra);
        let id = new.add_object(d);
        add_annot(prev, new, id);
    })
}

fn approve(input: &[u8]) -> Vec<u8> {
    sign(input, &id("rsa3072"), &Field::default(), &Cms::default())
}

fn doc_mdp(p: i64) -> Object {
    vec![Object::from(dictionary! {
        "Type" => "SigRef", "TransformMethod" => "DocMDP",
        "TransformParams" => dictionary! { "Type" => "TransformParams", "V" => "1.2", "P" => p },
    })]
    .into()
}

fn certify(input: &[u8], p: i64) -> Vec<u8> {
    let f = Field {
        reference: Some(doc_mdp(p)),
        perms: true,
        ..Field::default()
    };
    sign(input, &id("rsa3072"), &f, &Cms::default())
}

/// Signs `input` in a new field `name` with another key.
fn countersign(input: &[u8], name: &'static str, invisible: bool) -> Vec<u8> {
    let f = Field {
        name,
        invisible,
        ..Field::default()
    };
    sign(input, &id("p256"), &f, &Cms::default())
}

fn timestamp(input: &[u8], invisible: bool) -> Vec<u8> {
    let f = Field {
        name: "Timestamp1",
        sub_filter: "ETSI.RFC3161",
        doc_timestamp: true,
        invisible,
        m: None,
        reason: None,
        ..Field::default()
    };
    sign_with(input, &f, |r| {
        timestamp_token(&tsa(), r, "20260601130000Z", Hash::Sha256)
    })
}

fn lock(action: &str, fields: &[&str], p: Option<i64>) -> Object {
    let mut params = dictionary! {
        "Type" => "TransformParams", "V" => "1.2", "Action" => Object::Name(action.into()),
        "Fields" => fields.iter().map(|f| Object::string_literal(*f)).collect::<Vec<_>>(),
    };
    if let Some(p) = p {
        params.set("P", p);
    }
    vec![Object::from(dictionary! {
        "Type" => "SigRef", "TransformMethod" => "FieldMDP", "TransformParams" => params,
    })]
    .into()
}

fn locked_by(input: &[u8], reference: Object) -> Vec<u8> {
    let f = Field {
        reference: Some(reference),
        ..Field::default()
    };
    sign(input, &id("rsa3072"), &f, &Cms::default())
}

/// The page's content stream.
fn contents(b: &[u8]) -> ObjectId {
    let d = Document::load_mem(b).unwrap();
    dict(&d, page(&d))
        .get(b"Contents")
        .unwrap()
        .as_reference()
        .unwrap()
}

/// A viewer navigation action, harmless, never run.
fn next_page() -> Dictionary {
    dictionary! { "S" => "Named", "N" => "NextPage" }
}

/// Widget entries running `next_page()` when the widget's page opens.
fn on_open() -> Dictionary {
    dictionary! { "AA" => dictionary! { "PO" => next_page() } }
}

/// Entries for a new widget, made with any objects they need.
type Entries = fn(&mut Document) -> Dictionary;

/// `b` with an empty, invisible signature field `Extra` added as the full audit's probe adds
/// it: one widget, or (`kid`) a field with the widget as its kid. `extra` makes entries for
/// the widget (an action, say), adding any objects they need.
fn extra_field(b: &[u8], kid: bool, extra: impl FnOnce(&mut Document) -> Dictionary) -> Vec<u8> {
    edit(b, |prev, new| {
        let mut widget = dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "P" => page(prev),
            "Rect" => vec![0.into(), 0.into(), 0.into(), 0.into()],
        };
        widget.extend(&extra(new));
        let mut field = dictionary! { "FT" => "Sig", "T" => Object::string_literal("Extra") };
        let top = if kid {
            let top = new.new_object_id();
            widget.set("Parent", top);
            let w = new.add_object(widget);
            add_annot(prev, new, w);
            field.set("Kids", vec![Object::from(w)]);
            new.set_object(top, Object::Dictionary(field));
            top
        } else {
            widget.extend(&field);
            let w = new.add_object(widget);
            add_annot(prev, new, w);
            w
        };
        list_field(prev, new, top);
    })
}

/// Field `top` listed last in the form's `/Fields`.
fn list_field(prev: &Document, new: &mut Document, top: ObjectId) {
    let r = root(prev);
    let mut c = dict(prev, r);
    // A file not yet signed may have no form.
    let mut form = (c.get(b"AcroForm").and_then(Object::as_dict).cloned()).unwrap_or_default();
    let mut fields = (form.get(b"Fields").and_then(Object::as_array).cloned()).unwrap_or_default();
    fields.push(top.into());
    form.set("Fields", fields);
    c.set("AcroForm", form);
    new.set_object(r, Object::Dictionary(c));
}

/// Field `top` moved first in the form's `/Fields`, the rest kept in order.
fn list_first(prev: &Document, new: &mut Document, top: ObjectId) {
    relist(prev, new, |fields| {
        fields.retain(|f| f.as_reference().ok() != Some(top));
        fields.insert(0, top.into());
    });
}

/// Field `top` no longer listed in the form's `/Fields`.
fn unlist(prev: &Document, new: &mut Document, top: ObjectId) {
    relist(prev, new, |fields| {
        fields.retain(|f| f.as_reference().ok() != Some(top))
    });
}

/// The form's `/Fields` changed by `change`.
fn relist(prev: &Document, new: &mut Document, change: impl FnOnce(&mut Vec<Object>)) {
    let r = root(prev);
    let mut c = dict(prev, r);
    let mut form = c.get(b"AcroForm").unwrap().as_dict().unwrap().clone();
    let mut fields = form.get(b"Fields").unwrap().as_array().unwrap().clone();
    change(&mut fields);
    form.set("Fields", fields);
    c.set("AcroForm", form);
    new.set_object(r, Object::Dictionary(c));
}

// ---------------------------------------------------------------- allowed changes

#[test]
fn nothing_changed_after_signing() {
    let b = approve(&base_pdf());
    let s = sig(&b, "Signature1");
    assert!(s.changes.is_empty());
    assert_eq!(s.signed_len, Some(b.len()));
    assert!(report(&b).all_ok());
    // White space after the end changes nothing.
    let mut w = b.clone();
    w.extend_from_slice(b"\r\n\n");
    assert!(sig(&w, "Signature1").changes.is_empty());
}

/// A revision with a cross-reference table listing only the head of the free list, after
/// one with a stream, changes nothing. lopdf lists object 0 as free after reading such a
/// table, but not before, and that was refused as "can't read object 0 reliably".
#[test]
fn a_table_after_a_cross_reference_stream_changes_nothing_by_itself() {
    let mut d = Document::load_mem(&base_pdf()).unwrap();
    d.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceStream;
    let mut base = Vec::new();
    d.save_to(&mut base).unwrap();
    let b = approve(&base);
    assert!(sig(&b, "Signature1").changes.is_empty());
    let b = append_raw(&b, &[], &[0], &[], "");
    assert!(String::from_utf8_lossy(&b).contains("\nxref\n0 1\n"));
    assert_eq!(changes(&b), []);
    assert!(report(&b).all_ok());
}

/// A revision written by hand: object 1 as `body` in a new object stream, and a
/// cross-reference stream giving object 0 the entry `zero` (type and fields, `/W [1 4 2]`).
/// When `named`, that entry is in a second cross-reference stream the first names with
/// `/XRefStm` instead, and the first lists nothing for object 0.
fn object_one_in_a_stream(b: &[u8], body: &str, zero: (u8, u32, u16), named: bool) -> Vec<u8> {
    let d = Document::load_mem(b).unwrap();
    let r = root(&d);
    let prev = last_startxref(b);
    let (stm, x, s) = (d.max_id + 1, d.max_id + 2, d.max_id + 3);
    let mut out = [b, b"\n"].concat();
    let at_stm = out.len();
    let data = format!("1 0 {body}");
    out.extend_from_slice(
        format!(
            "{stm} 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Length {} >>\nstream\n{data}\n\
             endstream\nendobj\n",
            data.len()
        )
        .as_bytes(),
    );
    let rows = |entries: &[(u8, u32, u16)]| {
        let mut rows = Vec::new();
        for &(kind, f1, f2) in entries {
            rows.push(kind);
            rows.extend_from_slice(&f1.to_be_bytes());
            rows.extend_from_slice(&f2.to_be_bytes());
        }
        rows
    };
    let at_s = out.len();
    if named {
        let zero = rows(&[zero]);
        out.extend_from_slice(
            format!(
                "{s} 0 obj\n<< /Type /XRef /W [1 4 2] /Index [0 1] /Size {} /Length {} >>\n\
                 stream\n",
                s + 1,
                zero.len()
            )
            .as_bytes(),
        );
        out.extend_from_slice(&zero);
        out.extend_from_slice(b"\nendstream\nendobj\n");
    }
    let at_x = out.len();
    let mut entries = vec![(2, stm, 0), (1, at_stm as u32, 0), (1, at_x as u32, 0)];
    let (index, size, more) = if named {
        entries.push((1, at_s as u32, 0));
        (format!("1 1 {stm} 3"), s + 1, format!(" /XRefStm {at_s}"))
    } else {
        entries.insert(0, zero);
        (format!("0 2 {stm} 2"), x + 1, String::new())
    };
    let rows = rows(&entries);
    out.extend_from_slice(
        format!(
            "{x} 0 obj\n<< /Type /XRef /W [1 4 2] /Index [{index}] /Size {size} /Root {} {} R \
             /Prev {prev}{more} /Length {} >>\nstream\n",
            r.0,
            r.1,
            rows.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&rows);
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{at_x}\n%%EOF\n").as_bytes());
    out
}

/// A hybrid revision written by hand: object 1 as `body` in a new object stream, listed in
/// a cross-reference stream that a table names with `/XRefStm`. The table lists object 0, a
/// new number N with `table_n`, rows of type and offset (`f` or `n`), and the stream with
/// `stream_n`, rows of type and fields (`/W [1 4 2]`). Returns the file and N.
fn hybrid_revision(
    b: &[u8],
    body: &str,
    table_n: &[(char, u32)],
    stream_n: &[(u8, u32, u16)],
) -> (Vec<u8>, u32) {
    let d = Document::load_mem(b).unwrap();
    let r = root(&d);
    let prev = last_startxref(b);
    let (stm, x, n) = (d.max_id + 1, d.max_id + 2, d.max_id + 3);
    let mut out = [b, b"\n"].concat();
    let at_stm = out.len();
    let data = format!("1 0 {body}");
    out.extend_from_slice(
        format!(
            "{stm} 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Length {} >>\nstream\n{data}\n\
             endstream\nendobj\n",
            data.len()
        )
        .as_bytes(),
    );
    let mut entries = vec![(2u8, stm, 0u16)];
    let mut index = "1 1".to_string();
    for &e in stream_n {
        entries.push(e);
        index.push_str(&format!(" {n} 1"));
    }
    let mut rows = Vec::new();
    for (kind, f1, f2) in entries {
        rows.push(kind);
        rows.extend_from_slice(&f1.to_be_bytes());
        rows.extend_from_slice(&f2.to_be_bytes());
    }
    let at_x = out.len();
    out.extend_from_slice(
        format!(
            "{x} 0 obj\n<< /Type /XRef /W [1 4 2] /Index [{index}] /Size {} /Length {} >>\n\
             stream\n",
            n + 1,
            rows.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&rows);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    let at_t = out.len();
    let mut table = format!(
        "xref\n0 1\n0000000000 65535 f\r\n{stm} 2\n{at_stm:010} 00000 n\r\n{at_x:010} 00000 n\r\n"
    );
    for &(kind, offset) in table_n {
        table.push_str(&format!("{n} 1\n{offset:010} 00000 {kind}\r\n"));
    }
    out.extend_from_slice(
        format!(
            "{table}trailer\n<< /Size {} /Root {} {} R /Prev {prev} /XRefStm {at_x} >>\n\
             startxref\n{at_t}\n%%EOF\n",
            n + 1,
            r.0,
            r.1
        )
        .as_bytes(),
    );
    (out, n)
}

/// The `startxref` value of the revision `b` ends with.
fn last_startxref(b: &[u8]) -> String {
    let tail = String::from_utf8_lossy(&b[b.len() - 40..]).into_owned();
    let prev = tail.split("startxref").last().unwrap();
    prev.split_whitespace().next().unwrap().to_string()
}

/// A signed file whose cross-reference is a stream, with object 1 (the catalog) written
/// plainly: the file, where object 1 starts, and its dictionary as written.
fn object_one_signed() -> (Vec<u8>, u32, String) {
    let mut d = Document::load_mem(&base_pdf()).unwrap();
    d.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceStream;
    let mut base = Vec::new();
    d.save_to(&mut base).unwrap();
    let b = approve(&base);
    let d = Document::load_mem(&b).unwrap();
    let Some(&lopdf::xref::XrefEntry::Normal { offset, .. }) = d.reference_table.get(1) else {
        panic!("object 1 isn't written plainly");
    };
    let at = offset as usize;
    let text = String::from_utf8_lossy(&b[at..at + 200]).into_owned();
    assert!(text.starts_with("1 0 obj"));
    let old = text["1 0 obj".len()..]
        .split("endobj")
        .next()
        .unwrap()
        .trim()
        .to_string();
    (b, offset, old)
}

/// An entry in use for object 0, added between two signatures and freed after the second,
/// is refused. One pointing at the signed bytes of object 1 has lopdf read that old copy
/// over a new one in an object stream (it keeps the object an entry's bytes name, and
/// never puts an object-stream member over one it has). The second signature can't be
/// checked then, so the first was compared with the end alone, where object 0 is only
/// free, and the page tree turned for the second signer, then turned back, passed as
/// allowed.
#[test]
fn an_entry_in_use_for_object_0_is_refused() {
    let (b, offset, old) = object_one_signed();
    let max_id = Document::load_mem(&b).unwrap().max_id;
    let turned = format!("{}/Rotate 180>>", old.strip_suffix(">>").unwrap());
    // Object 0 at object 1's signed bytes, or in the new object stream.
    for zero in [(1, offset, 0), (2, max_id + 1, 0)] {
        let r2 = object_one_in_a_stream(&b, &turned, zero, false);
        let s2 = countersign(&r2, "Signature2", true);
        let b = append_raw(&s2, &[(1, old.as_bytes())], &[0], &[], "");
        let want = ChangeKind::Malformed("LimarePDF can't read object 0 reliably".into());
        let got = changes(&b);
        assert!(
            got.contains(&(want, Some(Refusal::Never))),
            "{zero:?}: {got:#?}"
        );
    }
}

/// A cross-reference stream naming another with `/XRefStm`, after signing, is refused. The
/// specification gives that key to a table's trailer only, and other apps don't follow it
/// from a stream, but lopdf does: there an entry in use for object 0, at object 1's signed
/// bytes, brought back that old copy past the check of
/// [`an_entry_in_use_for_object_0_is_refused`], and the page tree turned for the second
/// signer, then turned back, passed as allowed.
#[test]
fn a_cross_reference_stream_naming_another_is_refused() {
    let want = ChangeKind::Malformed(
        "a later revision's cross-reference stream names another one (/XRefStm), which PDF \
         apps read differently"
            .into(),
    );
    let (end, _) = turned_for_the_second_signer(|b, turned, offset| {
        let r2 = object_one_in_a_stream(b, turned, (1, offset, 0), true);
        // LimarePDF says other apps read the file differently.
        assert!(Pdf::from_bytes(&r2, None).unwrap().differs().is_some());
        (r2, 0)
    });
    let got = changes(&end);
    assert!(
        got.contains(&(want.clone(), Some(Refusal::Never))),
        "{got:#?}"
    );
    // The key alone is refused, whatever the stream it names lists.
    let (b, _, old) = object_one_signed();
    let plain = object_one_in_a_stream(&b, &old, (0, 0, 65535), true);
    assert_eq!(changes(&plain), [(want, Some(Refusal::Never))]);
}

/// The page tree turned for a second signer by a revision that lopdf reads with object 1's
/// signed copy, and turned back after: `revision` makes the first from the signed file,
/// object 1 turned and where its signed copy starts, returning it and the number lopdf
/// reads at that copy, which the last revision frees. Returns the file and that number.
fn turned_for_the_second_signer(
    revision: impl Fn(&[u8], &str, u32) -> (Vec<u8>, u32),
) -> (Vec<u8>, u32) {
    let (b, offset, old) = object_one_signed();
    let turned = format!("{}/Rotate 180>>", old.strip_suffix(">>").unwrap());
    let (r2, n) = revision(&b, &turned, offset);
    let lopdf = Document::load_mem(&r2).unwrap();
    assert!(matches!(
        lopdf.reference_table.get(n),
        Some(&lopdf::xref::XrefEntry::Normal { offset: o, .. }) if o == offset
    ));
    let s2 = countersign(&r2, "Signature2", true);
    (append_raw(&s2, &[(1, old.as_bytes())], &[n], &[], ""), n)
}

/// A hybrid revision whose table frees a number its cross-reference stream places, after
/// signing, is refused. Apps that read the table first take the number as free, but lopdf
/// lets the stream's entry replace a free one: placed at object 1's signed bytes, it
/// brought back that old copy over a new one in an object stream, and the page tree
/// turned for the second signer, then turned back, passed as allowed.
#[test]
fn a_hybrid_table_freeing_what_its_stream_places_is_refused() {
    let want = |n| {
        ChangeKind::Malformed(format!(
            "a later revision lists object {n} as free in its cross-reference table but in \
             use in its cross-reference stream, which PDF apps read differently"
        ))
    };
    let (end, n) = turned_for_the_second_signer(|b, turned, offset| {
        hybrid_revision(b, turned, &[('f', 0)], &[(1, offset, 0)])
    });
    let got = changes(&end);
    assert!(got.contains(&(want(n), Some(Refusal::Never))), "{got:#?}");
    // The two entries alone are refused, with nothing changed.
    let (b, offset, old) = object_one_signed();
    let (plain, n) = hybrid_revision(&b, &old, &[('f', 0)], &[(1, offset, 0)]);
    assert_eq!(changes(&plain), [(want(n), Some(Refusal::Never))]);
}

/// A revision giving a number two different entries in one cross-reference section, after
/// signing, is refused. LimarePDF takes the first and lopdf the last: free, then at object
/// 1's signed bytes, lopdf brought back that old copy over a new one in an object stream,
/// and the page tree turned for the second signer, then turned back, passed as allowed.
#[test]
fn a_number_listed_twice_differently_is_refused() {
    let want = |n| {
        ChangeKind::Malformed(format!(
            "a later revision gives object {n} two different cross-reference entries, which \
             PDF apps read differently"
        ))
    };
    // Twice in the table, or twice in the cross-reference stream.
    for in_stream in [false, true] {
        let revision = |b: &[u8], body: &str, offset| match in_stream {
            false => hybrid_revision(b, body, &[('f', 0), ('n', offset)], &[]),
            true => hybrid_revision(b, body, &[], &[(0, 0, 0), (1, offset, 0)]),
        };
        let (end, n) = turned_for_the_second_signer(revision);
        let got = changes(&end);
        assert!(got.contains(&(want(n), Some(Refusal::Never))), "{got:#?}");
        // The two entries alone are refused, with nothing changed.
        let (b, offset, old) = object_one_signed();
        let (plain, n) = revision(&b, &old, offset);
        assert_eq!(changes(&plain), [(want(n), Some(Refusal::Never))]);
    }
    // The same entry twice reads alike everywhere.
    let (b, _, old) = object_one_signed();
    let (twice, _) = hybrid_revision(&b, &old, &[('f', 0), ('f', 0)], &[]);
    assert_eq!(changes(&twice), []);
}

/// A later signature whose version other apps may read differently from LimarePDF can't be
/// checked, but its version is still compared with the ones before and after it, which
/// refuses the earlier signatures. A new number at object 1's signed bytes had lopdf read
/// that old copy over a new one in an object stream, and the page tree turned for the second
/// signer, then turned back, passed as allowed, when the versions either side were compared
/// as one.
#[test]
fn a_later_signed_version_read_differently_is_refused() {
    let (end, n) = turned_for_the_second_signer(|b, turned, offset| {
        let r2 = hybrid_revision(b, turned, &[('n', offset)], &[]);
        assert!(Pdf::from_bytes(&r2.0, None).unwrap().differs().is_some());
        r2
    });
    assert!(matches!(
        sig(&end, "Signature2").status,
        SigStatus::CantCheck(why) if why.starts_with("other PDF apps may read the version")
    ));
    let want = ChangeKind::Malformed(format!(
        "object {n} points outside the revision that changes it"
    ));
    let got = changes(&end);
    assert!(got.contains(&(want, Some(Refusal::Never))), "{got:#?}");
    // A signature after it, over a version read alike everywhere, is unaffected.
    let s3 = countersign(&end, "Signature3", true);
    assert_eq!(sig(&s3, "Signature3").status, SigStatus::Valid);
    assert!(sig(&s3, "Signature3").changes.is_empty());
}

/// Moves the end of the last signature's range by `by` bytes, keeping its width.
fn range_end_moved(b: &mut [u8], by: isize) {
    let key = b"/ByteRange";
    let at = b.windows(key.len()).rposition(|w| w == key).unwrap();
    let at = at + b[at..].iter().position(|&c| c == b'[').unwrap() + 1;
    let close = at + b[at..].iter().position(|&c| c == b']').unwrap();
    let text = String::from_utf8(b[at..close].to_vec()).unwrap();
    let d: usize = text.split_whitespace().last().unwrap().parse().unwrap();
    let i = text.rfind(&d.to_string()).unwrap();
    let moved = format!(
        "{}{:<w$}",
        &text[..i],
        d.checked_add_signed(by).unwrap(),
        w = text.len() - i
    );
    b[at..close].copy_from_slice(moved.as_bytes());
}

/// A later signature signing a part of the file that isn't one of its versions refuses the
/// earlier ones. Its range ending inside its revision's end-of-file marker, just before it,
/// or after a comment following it, LimarePDF can't check it, but Poppler shows it as valid
/// when it was signed over that range, and the file cut there shows the page as turned for
/// the second signer: turned back after, that passed as allowed.
#[test]
fn a_later_signed_version_ending_elsewhere_is_refused() {
    let want = ChangeKind::Malformed(
        "a later signature signs a part of the file that isn't one of the document's \
         versions, which PDF apps may show as signed"
            .into(),
    );
    let s2 = countersign(
        &turned(&approve(&base_pdf()), Some(180)),
        "Signature2",
        true,
    );
    let eof = s2.windows(5).rposition(|w| w == b"%%EOF").unwrap();
    let mid = eof as isize + 2 - s2.len() as isize;
    let before = eof as isize - s2.len() as isize;
    for (by, comment) in [(mid, ""), (before, ""), (3, "%x\n")] {
        let mut b = s2.clone();
        b.extend_from_slice(comment.as_bytes());
        range_end_moved(&mut b, by);
        let end = turned(&b, None);
        assert_eq!(
            sig(&end, "Signature2").status,
            SigStatus::Broken(
                "the signed part doesn't end where a version of the document ends".into()
            )
        );
        let got = changes(&end);
        assert!(
            got.contains(&(want.clone(), Some(Refusal::Never))),
            "{by}: {got:#?}"
        );
    }
}

/// A later signature that can't be checked for another reason refuses nothing earlier: the
/// version it signed reads alike everywhere, and is compared with the rest.
#[test]
fn a_later_signature_broken_otherwise_refuses_nothing() {
    let mut b = countersign(&approve(&base_pdf()), "Signature2", false);
    // A digit of Signature2's value, in the gap its range leaves, changed.
    let key = b"/ByteRange";
    let at = b.windows(key.len()).rposition(|w| w == key).unwrap();
    let text = String::from_utf8_lossy(&b[at..at + 80]).into_owned();
    let range: Vec<usize> = (text.split(['[', ']']).nth(1).unwrap().split_whitespace())
        .map(|n| n.parse().unwrap())
        .collect();
    let at = range[1] + 41;
    b[at] = if b[at] == b'0' { b'1' } else { b'0' };
    assert!(!matches!(sig(&b, "Signature2").status, SigStatus::Valid));
    assert_eq!(
        changes(&b),
        [(ChangeKind::Signed("Signature2".into()), None)]
    );
}

/// A later signature whose range ends in the white space just after an earlier signer's
/// version signs that version, and refuses nothing for its signer, as one ending right at it
/// wouldn't. That white space was taken for a part of the file that isn't a version.
#[test]
fn a_later_signed_version_ending_in_white_space_after_one_refuses_nothing() {
    let mut b = approve(&base_pdf());
    let end = b.len();
    b.extend_from_slice(b"\n   \n");
    let mut b = countersign(&b, "Signature2", true);
    // Signature2's range, the same width, ending 2 bytes into the white space.
    let key = b"/ByteRange";
    let at = b.windows(key.len()).rposition(|w| w == key).unwrap();
    let at = at + b[at..].iter().position(|&c| c == b'[').unwrap() + 1;
    let close = at + b[at..].iter().position(|&c| c == b']').unwrap();
    let range = format!("0 1 2 {:<w$}", end, w = close - at - 6);
    b[at..close].copy_from_slice(range.as_bytes());
    assert!(!matches!(sig(&b, "Signature2").status, SigStatus::Valid));
    assert_eq!(
        changes(&b),
        [(ChangeKind::Signed("Signature2".into()), None)]
    );
}

/// A later signature past the most signed versions LimarePDF opens can't be checked, but its
/// version is still compared with the ones before and after it, and as LimarePDF can't read
/// it, the earlier signatures are refused. 99 signatures with empty values used up the limit
/// before the second signer's, and the page turned for that signer, then turned back, passed
/// as allowed.
#[test]
fn a_later_signed_version_past_the_limit_is_refused() {
    let mut b = approve(&base_pdf());
    for k in 1..100 {
        let f = Field {
            name: format!("Filler{k}").leak(),
            invisible: true,
            // Each placeholder its own size, as the empty values leave them in place.
            room: 16 + k,
            ..Field::default()
        };
        b = sign_with(&b, &f, |_| Vec::new());
    }
    let s2 = countersign(&turned(&b, Some(180)), "Signature2", true);
    let end = turned(&s2, None);
    assert_eq!(
        sig(&end, "Signature2").status,
        SigStatus::CantCheck(
            "the document has more signed versions than LimarePDF checks (at most 100, 1024 MB \
             together)"
                .into()
        )
    );
    let want = ChangeKind::Malformed(
        "the document has more signed versions than LimarePDF checks (at most 100, 1024 MB \
         together)"
            .into(),
    );
    let got = changes(&end);
    assert!(got.contains(&(want, Some(Refusal::Never))), "{got:#?}");
}

/// A later signed version that isn't one of the revisions the file's chain reads is refused.
/// The last revision lists again what the two after the first signature wrote, the page not
/// turned, and points back past them to the first signature's: the second signer's version,
/// with the page turned, was never compared, and the turn passed unseen.
#[test]
fn a_later_signed_version_off_the_chain_is_refused() {
    let b = approve(&base_pdf());
    let s2 = countersign(&turned(&b, Some(180)), "Signature2", true);
    let doc = Document::load_mem(&s2).unwrap();
    let mut out = s2.clone();
    let mut rows = String::new();
    for (&n, e) in &doc.reference_table.entries {
        let &lopdf::xref::XrefEntry::Normal { offset, generation } = e else {
            continue;
        };
        let mut at = offset as usize;
        if at < b.len() {
            continue;
        }
        if (n, generation) == page(&doc) {
            let len = s2[at..].windows(6).position(|w| w == b"endobj").unwrap() + 6;
            let text = String::from_utf8_lossy(&s2[at..at + len]).replace("/Rotate 180", "");
            at = out.len();
            out.extend_from_slice(text.as_bytes());
            out.push(b'\n');
        }
        rows += &format!("{n} 1\n{at:010} {generation:05} n \n");
    }
    let xref = out.len();
    let (num, gen) = root(&doc);
    let tail = format!(
        "xref\n{rows}trailer\n<< /Size {} /Root {num} {gen} R /Prev {} >>\nstartxref\n{xref}\n\
         %%EOF\n",
        doc.max_id + 1,
        last_startxref(&b)
    );
    out.extend_from_slice(tail.as_bytes());
    let rotate = |b: &[u8]| {
        let d = Document::load_mem(b).unwrap();
        dict(&d, page(&d)).get(b"Rotate").ok().cloned()
    };
    // Other apps show the page turned where the second signer signed, and not at the end.
    assert_eq!(rotate(&out[..s2.len()]), Some(Object::Integer(180)));
    assert_eq!(rotate(&out), None);
    let want = ChangeKind::Malformed(
        "a later signature signs a part of the file that isn't one of the document's \
         versions, which PDF apps may show as signed"
            .into(),
    );
    let got = changes(&out);
    assert!(got.contains(&(want, Some(Refusal::Never))), "{got:#?}");
}

/// A cross-reference stream whose rows are under 3 bytes wide, after signing, is refused.
/// lopdf can't read one unless its data is padded, and drops it from a hybrid revision, where
/// other apps read it: there object 1 turned for the second signer, which lopdf read as
/// signed, then turned back. The second signature's version reads differently, which refuses
/// the first too; this check names the cause.
#[test]
fn a_cross_reference_stream_with_narrow_rows_is_refused() {
    let want = ChangeKind::Malformed(
        "a later revision's cross-reference stream has entries under 3 bytes wide, which some \
         PDF apps can't read"
            .into(),
    );
    let (b, offset, old) = object_one_signed();
    let turned = format!("{}/Rotate 180>>", old.strip_suffix(">>").unwrap());
    let r2 = object_one_in_rows(&b, &turned, [0, 2], true);
    let lopdf = Document::load_mem(&r2).unwrap();
    assert!(matches!(
        lopdf.reference_table.get(1),
        Some(&lopdf::xref::XrefEntry::Normal { offset: o, .. }) if o == offset
    ));
    assert!(Pdf::from_bytes(&r2, None).unwrap().differs().is_some());
    let s2 = countersign(&r2, "Signature2", true);
    let end = append_raw(&s2, &[(1, old.as_bytes())], &[], &[], "");
    let got = changes(&end);
    assert!(
        got.contains(&(want.clone(), Some(Refusal::Never))),
        "{got:#?}"
    );
    // The stream alone is refused, with nothing changed, in a hybrid revision or its own.
    for hybrid in [true, false] {
        let plain = object_one_in_rows(&b, &old, [0, 2], hybrid);
        assert_eq!(changes(&plain), [(want.clone(), Some(Refusal::Never))]);
        // Rows of 3 bytes read alike everywhere, with the type or without.
        for w in [[1, 2], [0, 3]] {
            assert_eq!(changes(&object_one_in_rows(&b, &old, w, hybrid)), []);
        }
    }
}

/// A revision written by hand: object 1 as `body`, listed in a cross-reference stream with
/// `/W [w[0] w[1] 0]`: type 1 in `w[0]` bytes (none or one) and its offset in `w[1]`. When
/// `hybrid`, a table lists only object 0 and that stream, and names it with `/XRefStm`.
fn object_one_in_rows(b: &[u8], body: &str, w: [usize; 2], hybrid: bool) -> Vec<u8> {
    let rows = |at_1: u32| {
        assert!(at_1 < 1 << (8 * w[1]));
        let mut row = vec![1; w[0]];
        row.extend_from_slice(&at_1.to_be_bytes()[4 - w[1]..]);
        row
    };
    let keys = format!("/W [{} {} 0] /Index [1 1]", w[0], w[1]);
    object_one_listed(b, body, &[], &keys, |at| rows(at[0]), "", hybrid)
}

/// A revision written by hand: object 1 as `body`, then `more` (number and text), listed in a
/// cross-reference stream whose dictionary holds `keys` (`/W`, `/Index` and any other) and
/// whose data `rows` makes from where object 1 and each of `more` start, `tail` written after
/// the data, before the line end and `endstream`. When `hybrid`, a table lists only object 0
/// and that stream, and names it with `/XRefStm`.
fn object_one_listed(
    b: &[u8],
    body: &str,
    more: &[(u32, Vec<u8>)],
    keys: &str,
    rows: impl FnOnce(&[u32]) -> Vec<u8>,
    tail: &str,
    hybrid: bool,
) -> Vec<u8> {
    let d = Document::load_mem(b).unwrap();
    let r = root(&d);
    let prev = last_startxref(b);
    let x = more.iter().map(|(n, _)| *n).fold(d.max_id, u32::max) + 1;
    let mut out = [b, b"\n"].concat();
    let mut at = vec![u32::try_from(out.len()).unwrap()];
    out.extend_from_slice(format!("1 0 obj\n{body}\nendobj\n").as_bytes());
    for (_, text) in more {
        at.push(u32::try_from(out.len()).unwrap());
        out.extend_from_slice(text);
    }
    let row = rows(&at);
    let at_x = out.len();
    let size = format!("/Size {}", x + 1);
    let links = format!("/Root {} {} R /Prev {prev}", r.0, r.1);
    let own = if hybrid { "" } else { &links };
    out.extend_from_slice(
        format!(
            "{x} 0 obj\n<< /Type /XRef {keys} {size} {own} /Length {} >>\nstream\n",
            row.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&row);
    out.extend_from_slice(format!("{tail}\nendstream\nendobj\n").as_bytes());
    let at_t = out.len();
    if hybrid {
        out.extend_from_slice(
            format!(
                "xref\n0 1\n0000000000 65535 f\r\n{x} 1\n{at_x:010} 00000 n\r\ntrailer\n<< \
                 {size} {links} /XRefStm {at_x} >>\n"
            )
            .as_bytes(),
        );
    }
    let start = if hybrid { at_t } else { at_x };
    out.extend_from_slice(format!("startxref\n{start}\n%%EOF\n").as_bytes());
    out
}

/// Cross-reference streams lopdf reads otherwise, and those the specification doesn't allow,
/// after signing, are refused, with object 1 turned in them or as signed. lopdf drops a hybrid
/// revision's stream with a comment before `endstream` and reads object 1 as signed; it takes
/// an object stream index past 65535 modulo 65536 and a 5-byte offset modulo 2^32, so reads
/// object 1 where the specification leads nowhere. Bytes before `endstream`, a number with an
/// exponent or `inf` in the dictionary, a type 3 entry, an odd `/Index` and a `/W` of 4 widths
/// are malformed.
#[test]
fn a_cross_reference_stream_lopdf_reads_otherwise_is_refused() {
    use lopdf::xref::XrefEntry;
    let (b, offset, old) = object_one_signed();
    let turned = format!("{}/Rotate 180>>", old.strip_suffix(">>").unwrap());
    let refused = |why: String| vec![(ChangeKind::Malformed(why), Some(Refusal::Never))];
    let lopdf_reads = |f: &[u8]| {
        Document::load_mem(f)
            .unwrap()
            .reference_table
            .get(1)
            .cloned()
    };
    let plain = "/W [1 4 0] /Index [1 1]";
    for body in [&turned, &old] {
        let f = object_one_listed(&b, body, &[], plain, |at| row(1, at[0]), "%c", true);
        assert!(matches!(
            lopdf_reads(&f),
            Some(XrefEntry::Normal { offset: o, .. }) if o == offset
        ));
        assert_eq!(
            changes(&f),
            refused("LimarePDF can't read object 1 reliably".into())
        );
        for (keys, tail, kind) in [
            (plain, " x", 1),
            ("/W [1 4 0] /Index [1 1] /Foo 1e5", "", 1),
            ("/W [1 4 0] /Index [1 1] /Foo inf", "", 1),
            (plain, "", 3),
            ("/W [1 4 0] /Index [1 1 5]", "", 1),
            ("/W [1 4 0 0] /Index [1 1]", "", 1),
        ] {
            for hybrid in [true, false] {
                let f = object_one_listed(&b, body, &[], keys, |at| row(kind, at[0]), tail, hybrid);
                let at = xref_stream_at(&f);
                let bad = format!("the cross-reference section at byte {at} is malformed");
                assert_eq!(changes(&f), refused(bad), "{keys} {tail:?} {kind}");
            }
        }
        for hybrid in [true, false] {
            let in_50 = [(50, object_stream(50, &[(1, 0)], body, ""))];
            let f = object_one_listed(
                &b,
                body,
                &in_50,
                "/W [1 4 4] /Index [1 1 50 1]",
                |at| {
                    [
                        row(2, 50),
                        65536u32.to_be_bytes().into(),
                        row(1, at[1]),
                        vec![0; 4],
                    ]
                    .concat()
                },
                "",
                hybrid,
            );
            assert!(matches!(
                lopdf_reads(&f),
                Some(XrefEntry::Compressed {
                    container: 50,
                    index: 0
                })
            ));
            assert_eq!(
                changes(&f),
                refused("LimarePDF can't read object 1 reliably".into())
            );
            let wide = |at: &[u32]| [vec![1], row(1, at[0])].concat();
            let f = object_one_listed(&b, body, &[], "/W [1 5 0] /Index [1 1]", wide, "", hybrid);
            let at_1 = b.len() as u32 + 1;
            assert!(matches!(
                lopdf_reads(&f),
                Some(XrefEntry::Normal { offset: o, .. }) if o == at_1
            ));
            assert_eq!(
                changes(&f),
                refused("object 1 points outside the revision that changes it".into())
            );
        }
    }
    // The same streams written plainly read alike everywhere.
    for hybrid in [true, false] {
        let f = object_one_listed(&b, &old, &[], plain, |at| row(1, at[0]), "", hybrid);
        assert_eq!(changes(&f), []);
    }
}

/// A cross-reference section Poppler can't read as written, after signing, is refused:
/// Poppler then scans the whole file for objects and takes the last copy of each it finds, so
/// a stray turned copy of the page after the signed version showed turned in Poppler while
/// the revision passed as unchanged (review round 4). Poppler can't make room for many more
/// objects than 2^23 (the newest trailer's or a cross-reference stream's `/Size`, or a
/// subsection reaching past that), and takes a number past its whole numbers, an offset past
/// 2^63 - 1 or a type wider than 4 bytes as damage. Those just within the limits pass, as
/// Poppler reads them as written.
#[test]
fn a_cross_reference_section_poppler_cant_read_is_refused() {
    const ROOM: u32 = 1 << 23;
    let check = |f: &[u8], at: usize, refused: bool, case: &str| {
        let why = format!("the cross-reference section at byte {at} is malformed");
        let want = vec![(ChangeKind::Malformed(why), Some(Refusal::Never))];
        assert_eq!(changes(f), if refused { want } else { vec![] }, "{case}");
    };
    let b = approve(&base_pdf());
    let d = Document::load_mem(&b).unwrap();
    let (p, _, turned) = page_texts(&b);
    let (r, prev, free) = (root(&d), last_startxref(&b), d.max_id + 1);
    let table = |rows: &str, size: u32| {
        let stray = format!("% {} 0 obj {turned} endobj\n", p.0);
        let mut out = [&b[..], b"\n", stray.as_bytes()].concat();
        let at = out.len();
        out.extend_from_slice(
            format!(
                "xref\n0 1\n0000000000 65535 f\r\n{rows}trailer\n<< /Size {size} /Root {} {} R \
                 /Prev {prev} >>\nstartxref\n{at}\n%%EOF\n",
                r.0, r.1
            )
            .as_bytes(),
        );
        (out, at)
    };
    for (rows, size, refused) in [
        (String::new(), ROOM, false),
        (String::new(), ROOM + 1, true),
        (format!("{ROOM} 0\n"), free + 1, false),
        (format!("{} 0\n", ROOM + 1), free + 1, true),
        (
            format!("{free} 1\n0000000000 2147483647 f\r\n"),
            free + 1,
            false,
        ),
        (
            format!("{free} 1\n0000000000 2147483648 f\r\n"),
            free + 1,
            true,
        ),
    ] {
        let (f, at) = table(&rows, size);
        check(&f, at, refused, &format!("{rows:?} /Size {size}"));
    }
    // An older table's `/Size`, which Poppler doesn't make room for.
    let (older, at) = table("", ROOM + 1);
    let newer = format!(
        "xref\n0 1\n0000000000 65535 f\r\ntrailer\n<< /Size {free} /Root {} {} R /Prev {at} >>\n\
         startxref\n{}\n%%EOF\n",
        r.0,
        r.1,
        older.len()
    );
    let f = [older, newer.into_bytes()].concat();
    check(&f, 0, false, "an older /Size");

    let (b, _, old) = object_one_signed();
    fn gen(g: u32) -> Vec<u8> {
        g.to_be_bytes().into()
    }
    fn far(f1: u64) -> Vec<u8> {
        [vec![0], f1.to_be_bytes().into(), gen(0)].concat()
    }
    type Rows = fn(u32) -> Vec<u8>;
    let rows: [(&str, Rows, bool); 11] = [
        (
            "/W [4 4 0] /Index [1 1]",
            |at| [vec![0; 3], row(1, at)].concat(),
            false,
        ),
        (
            "/W [5 4 0] /Index [1 1]",
            |at| [vec![0; 4], row(1, at)].concat(),
            true,
        ),
        (
            "/W [1 4 0] /Index [1 1 8388607 1]",
            |at| [row(1, at), row(0, 0)].concat(),
            false,
        ),
        (
            "/W [1 4 0] /Index [1 1 8388608 1]",
            |at| [row(1, at), row(0, 0)].concat(),
            true,
        ),
        (
            "/W [1 4 4] /Index [1 1 1000 1]",
            |at| [row(1, at), gen(0), row(0, 0), gen(i32::MAX as u32)].concat(),
            false,
        ),
        (
            "/W [1 4 4] /Index [1 1 1000 1]",
            |at| [row(1, at), gen(0), row(0, 0), gen(1 << 31)].concat(),
            true,
        ),
        (
            "/W [1 4 4] /Index [0 1 1 1]",
            |at| [row(0, 0), gen(u32::MAX), row(1, at), gen(0)].concat(),
            false,
        ),
        (
            "/W [1 4 4] /Index [1 1 1000 1]",
            |at| [row(1, at), gen(0), row(0, 0), gen(u32::MAX)].concat(),
            true,
        ),
        (
            "/W [1 4 4] /Index [0 1 1 1]",
            |at| [row(0, 0), gen(u32::MAX - 1), row(1, at), gen(0)].concat(),
            true,
        ),
        (
            "/W [1 8 4] /Index [1 1 1000 1]",
            |at| {
                [
                    vec![1],
                    u64::from(at).to_be_bytes().into(),
                    gen(0),
                    far(i64::MAX as u64),
                ]
                .concat()
            },
            false,
        ),
        (
            "/W [1 8 4] /Index [1 1 1000 1]",
            |at| {
                [
                    vec![1],
                    u64::from(at).to_be_bytes().into(),
                    gen(0),
                    far(1 << 63),
                ]
                .concat()
            },
            true,
        ),
    ];
    for (i, (keys, rows, refused)) in rows.into_iter().enumerate() {
        for hybrid in [true, false] {
            let f = object_one_listed(&b, &old, &[], keys, |at| rows(at[0]), "", hybrid);
            check(
                &f,
                xref_stream_at(&f),
                refused,
                &format!("{i} {keys} {hybrid}"),
            );
        }
    }
    // The number in a cross-reference stream's header, and its own `/Size`.
    let keys = "/W [1 4 0] /Index [1 1]";
    let plain = object_one_listed(&b, &old, &[], keys, |at| row(1, at[0]), "", false);
    let at = xref_stream_at(&plain);
    let header = at + plain[at..].iter().position(|&c| c == b' ').unwrap();
    for (num, refused) in [("2147483647", false), ("2147483648", true)] {
        let f = [&plain[..at], num.as_bytes(), &plain[header..]].concat();
        check(&f, at, refused, num);
    }
    // Poppler makes room for every cross-reference stream's `/Size`, under a newer table too.
    let s = at + plain[at..].windows(6).position(|w| w == b"/Size ").unwrap() + 6;
    let e = s + plain[s..].iter().position(|c| !c.is_ascii_digit()).unwrap();
    let r = root(&Document::load_mem(&plain).unwrap());
    for (size, refused) in [
        (ROOM.to_string(), false),
        ((ROOM + 1).to_string(), true),
        ("-1".into(), true),
    ] {
        let f = [&plain[..s], size.as_bytes(), &plain[e..]].concat();
        check(&f, at, refused, &size);
        let newer = format!(
            "xref\n0 1\n0000000000 65535 f\r\ntrailer\n<< /Size {} /Root {} {} R /Prev {at} >>\n\
             startxref\n{}\n%%EOF\n",
            String::from_utf8_lossy(&plain[s..e]),
            r.0,
            r.1,
            f.len()
        );
        let f = [f, newer.into_bytes()].concat();
        check(&f, at, refused, &format!("{size} under a table"));
    }
}

/// A table subsection that starts at object 1 with a free entry, after signing, is refused:
/// Poppler (for one at offset 0 with generation 65535, when no newer section lists object 1)
/// and pdf.js (for any) take it for one that should start at 0, as files from IBM's patent
/// server have it, and read each of its entries as the object before (review round 4). A
/// revision writing objects 2 to the one after the catalog over unchanged that way had them
/// find the next object where they looked for the catalog, scan the whole file for objects
/// and show a turned copy of the page it doesn't list, while the revision passed as
/// unchanged. A subsection of that one free entry alone, which they read as object 0's,
/// leaves object 1 to the older sections there, while LimarePDF reads it deleted: judged as
/// usual, it passes when nothing uses object 1.
#[test]
fn a_subsection_other_apps_number_from_0_is_refused() {
    use lopdf::xref::XrefEntry;
    // The base document numbered from 2, so that object 1 is free.
    let mut d = Document::load_mem(&base_pdf()).unwrap();
    d.renumber_objects_with(2);
    let mut base = Vec::new();
    d.save_to(&mut base).unwrap();
    let b = approve(&base);
    let d = Document::load_mem(&b).unwrap();
    assert!(!d.objects.contains_key(&(1, 0)));
    let (p, _, turned) = page_texts(&b);
    let (r, prev, last) = (root(&d), last_startxref(&b), root(&d).0 + 1);
    // Object 1's entry: as given, or in use for a new object 1 when none is given; with no
    // `head`, in object 0's subsection.
    let free = Some("0000000000 65535 f\r\n");
    let odd = Some("0000000007 00003 f\r\n");
    let reused = Some("0000000000 00001 f\r\n");
    for (head, one, mid, refused) in [
        (format!("1 {last}\n"), free, "", true),
        (format!("1 {last}\n"), odd, "", true),
        (format!("1 {last}\n"), None, "", false),
        (String::new(), reused, "2 _\n", false),
        ("1 1\n".into(), free, "2 _\n", false),
    ] {
        let stray = format!("{} 0 obj {turned} endobj\n", p.0);
        let mut f = [&b[..], b"\n", stray.as_bytes()].concat();
        let one = one.map_or_else(
            || {
                let row = format!("{:010} 00000 n\r\n", f.len());
                f.extend_from_slice(b"1 0 obj\nnull\nendobj\n");
                row
            },
            String::from,
        );
        let mut rows = String::new();
        for num in 2..=last {
            let Some(&XrefEntry::Normal { offset, .. }) = d.reference_table.get(num) else {
                panic!("object {num} isn't written plainly");
            };
            let obj = &b[offset as usize..];
            let end = obj.windows(6).position(|w| w == b"endobj").unwrap() + 6;
            rows += &format!("{:010} 00000 n\r\n", f.len());
            f.extend_from_slice(&obj[..end]);
            f.push(b'\n');
        }
        // lopdf's save drops the cross-reference stream it loaded but not its number, which no
        // section lists and Poppler would scan the file for: listed free.
        let gaps: String = (last + 1..=d.max_id)
            .filter(|&num| d.reference_table.get(num).is_none())
            .map(|num| format!("{num} 1\n0000000000 00001 f\r\n"))
            .collect();
        let at = f.len();
        let mid = mid.replace('_', &(last - 1).to_string());
        let zero = if head.is_empty() { 2 } else { 1 };
        f.extend_from_slice(
            format!(
                "xref\n0 {zero}\n0000000000 65535 f\r\n{head}{one}{mid}{rows}{gaps}trailer\n<< \
                 /Size {} /Root {} {} R /Prev {prev} >>\nstartxref\n{at}\n%%EOF\n",
                d.max_id + 1,
                r.0,
                r.1
            )
            .as_bytes(),
        );
        let why = format!("the cross-reference section at byte {at} is malformed");
        let want = vec![(ChangeKind::Malformed(why), Some(Refusal::Never))];
        let case = format!("{head:?} {one:?} {mid:?}");
        assert_eq!(changes(&f), if refused { want } else { vec![] }, "{case}");
    }
}

/// A cross-reference stream row of type `kind` and a 4-byte field `f1`.
fn row(kind: u8, f1: u32) -> Vec<u8> {
    [vec![kind], f1.to_be_bytes().into()].concat()
}

/// Where the last cross-reference stream in `f` starts.
fn xref_stream_at(f: &[u8]) -> usize {
    let dict = f.windows(14).rposition(|w| w == b"<< /Type /XRef").unwrap();
    f[..dict - 1].iter().rposition(|&c| c == b'\n').unwrap() + 1
}

/// A pointer to a cross-reference section that names the white space before it, after
/// signing, is refused when lopdf finds another section near it. lopdf takes the nearest
/// `xref` within 64 bytes of a pointer that doesn't lead straight to a section: a decoy table
/// there named the signed catalog, or left out the section's entries, so lopdf read the page
/// as signed while other apps read it turned, and the turn passed as allowed.
#[test]
fn a_pointer_lopdf_moves_to_a_section_near_it_is_refused() {
    let want = |at| {
        ChangeKind::Malformed(format!(
            "PDF apps may take different cross-reference sections near byte {at}: the pointer \
             to it doesn't lead straight to one"
        ))
    };
    let b = approve(&base_pdf());
    let rotate = |d: &Document| dict(d, page(d)).get(b"Rotate").ok().cloned();
    for decoy in [Decoy::Stream, Decoy::Table, Decoy::XRefStm, Decoy::Prev] {
        let (turned, at) = behind_a_decoy(&b, decoy, true, true);
        // lopdf reads the page as signed.
        assert_eq!(rotate(&Document::load_mem(&turned).unwrap()), None);
        let got = changes(&turned);
        assert!(
            got.contains(&(want(at), Some(Refusal::Never))),
            "{decoy:?}: {got:#?}"
        );
        assert!(Pdf::from_bytes(&turned, None).unwrap().differs().is_some());
        // The decoy alone is refused, with nothing changed.
        let (plain, at) = behind_a_decoy(&b, decoy, false, true);
        assert_eq!(
            changes(&plain),
            [(want(at), Some(Refusal::Never))],
            "{decoy:?}"
        );
        // A `/Prev` must name where its revision's own pointer does, white space or not.
        if let Decoy::Prev = decoy {
            continue;
        }
        // Without it, lopdf takes the section after the white space too, and reads the turn.
        let (alone, _) = behind_a_decoy(&b, decoy, true, false);
        assert_eq!(
            rotate(&Document::load_mem(&alone).unwrap()),
            Some(Object::Integer(180))
        );
        assert!(Pdf::from_bytes(&alone, None).unwrap().differs().is_none());
        let got = changes(&alone);
        assert!(
            !got.is_empty()
                && got
                    .iter()
                    .all(|(k, _)| !matches!(k, ChangeKind::Malformed(_))),
            "{decoy:?}: {got:#?}"
        );
    }
}

/// Which pointer names the white space before a revision's cross-reference section, in
/// [`behind_a_decoy`].
#[derive(Clone, Copy, Debug)]
enum Decoy {
    /// `startxref`, before a cross-reference stream, which the decoy names with `/Prev`.
    Stream,
    /// `startxref`, before a table past more white space; the decoy names a cross-reference
    /// stream listing the same with `/Prev`.
    Table,
    /// A hybrid table's `/XRefStm`, before its stream.
    XRefStm,
    /// The next revision's `/Prev`, before a cross-reference stream.
    Prev,
}

/// A revision written by hand, page 1 turned when `rotate`, whose pointer (by `decoy`) names
/// the white space just before its section. When `near`, a decoy table (`xref 0 0`) ends at
/// that white space, which lopdf takes instead. Through `startxref`, the revision adds a new
/// catalog over a copy of the page, and the decoy's trailer names the signed catalog;
/// otherwise the page itself changes, and the decoy leads past the section listing it.
/// Returns the file and where the pointer points.
fn behind_a_decoy(b: &[u8], decoy: Decoy, rotate: bool, near: bool) -> (Vec<u8>, usize) {
    let d = Document::load_mem(b).unwrap();
    let (r, p) = (root(&d), page(&d));
    let prev = last_startxref(b);
    let n = d.max_id + 1;
    let mut page = dict(&d, p);
    if rotate {
        page.set("Rotate", 180);
    }
    let mut out = [b, b"\n"].concat();
    let put = |out: &mut Vec<u8>, num: u32, body: &Dictionary| {
        let at = out.len();
        out.extend_from_slice(format!("{num} 0 obj\n{body:?}\nendobj\n").as_bytes());
        at
    };
    // A cross-reference stream listing objects at `offsets` (`/W [1 4 2]`).
    let stream = |out: &mut Vec<u8>, num: u32, index: String, offsets: &[usize], more: String| {
        let mut rows = Vec::new();
        for &at in offsets {
            rows.push(1);
            rows.extend_from_slice(&u32::try_from(at).unwrap().to_be_bytes());
            rows.extend_from_slice(&[0, 0]);
        }
        out.extend_from_slice(
            format!(
                "{num} 0 obj\n<< /Type /XRef /W [1 4 2] /Index [{index}] {more} /Length {} >>\n\
                 stream\n",
                rows.len()
            )
            .as_bytes(),
        );
        out.extend_from_slice(&rows);
        out.extend_from_slice(b"\nendstream\nendobj\n");
    };
    // The decoy, ending where the pointer points.
    let decoy_then = |out: &mut Vec<u8>, trailer: String| {
        if near {
            let text = format!("xref\n0 0\ntrailer<<{trailer}>>\n");
            assert!(text.len() <= 64, "{text}");
            out.extend_from_slice(text.as_bytes());
        }
        out.len()
    };
    let at = match decoy {
        Decoy::Stream | Decoy::Table => {
            // A new catalog over a new page tree holding a copy of the page.
            let (c, q, p2) = (n, n + 1, n + 2);
            let mut catalog = dict(&d, r);
            let pages = catalog.get(b"Pages").unwrap().as_reference().unwrap();
            catalog.set("Pages", (q, 0));
            let mut tree = dict(&d, pages);
            tree.set("Kids", vec![Object::Reference((p2, 0))]);
            page.set("Parent", (q, 0));
            let offsets = vec![
                put(&mut out, c, &catalog),
                put(&mut out, q, &tree),
                put(&mut out, p2, &page),
            ];
            let size = n + 4;
            let own = format!("/Size {size} /Root {c} 0 R /Prev {prev}");
            if let Decoy::Stream = decoy {
                // The stream follows one byte of white space; the decoy's `/Prev` names it.
                let trailer = |x: usize| format!("/Size {size}/Root {} 0 R/Prev {x:05}", r.0);
                let len = format!("xref\n0 0\ntrailer<<{}>>\n", trailer(0)).len();
                let x = out.len() + if near { len } else { 0 } + 1;
                let at = decoy_then(&mut out, trailer(x));
                out.push(b'\n');
                assert_eq!(out.len(), x);
                let mut offsets = offsets;
                offsets.push(x);
                stream(&mut out, n + 3, format!("{c} 4"), &offsets, own);
                at
            } else {
                // The same objects in a stream the decoy names with `/Prev`, then the table
                // past more white space than lopdf looks through.
                let s = out.len();
                stream(
                    &mut out,
                    n + 3,
                    format!("{c} 3"),
                    &offsets,
                    format!("/Size {size} /Prev {prev}"),
                );
                let at = decoy_then(&mut out, format!("/Size {size}/Root {} 0 R/Prev {s}", r.0));
                out.extend_from_slice(if near { &[b' '; 70][..] } else { b"\n" });
                let mut table = format!("xref\n0 1\n0000000000 65535 f\r\n{c} 3\n");
                for o in &offsets {
                    table += &format!("{o:010} 00000 n\r\n");
                }
                out.extend_from_slice(format!("{table}trailer\n<< {own} >>\n").as_bytes());
                at
            }
        }
        Decoy::XRefStm => {
            let at_p = put(&mut out, p.0, &page);
            let at = decoy_then(&mut out, format!("/Size {}", n + 1));
            out.push(b'\n');
            let x = out.len();
            stream(
                &mut out,
                n,
                format!("{} 1", p.0),
                &[at_p],
                format!("/Size {}", n + 1),
            );
            let t = out.len();
            out.extend_from_slice(
                format!(
                    "xref\n0 1\n0000000000 65535 f\r\n{n} 1\n{x:010} 00000 n\r\ntrailer\n<< \
                     /Size {} /Root {} 0 R /Prev {prev} /XRefStm {at} >>\nstartxref\n{t}\n\
                     %%EOF\n",
                    n + 1,
                    r.0
                )
                .as_bytes(),
            );
            return (out, at);
        }
        Decoy::Prev => {
            // A revision read alike everywhere, but for the decoy before its section, then
            // one whose `/Prev` names the white space before that section.
            let at_p = put(&mut out, p.0, &page);
            let at = decoy_then(
                &mut out,
                format!("/Size {}/Root {} 0 R/Prev {prev}", n + 1, r.0),
            );
            out.push(b'\n');
            let x = out.len();
            stream(
                &mut out,
                n,
                format!("{} 1 {n} 1", p.0),
                &[at_p, x],
                format!("/Size {} /Root {} 0 R /Prev {prev}", n + 1, r.0),
            );
            let t = out.len() + 10 + x.to_string().len() + 7;
            out.extend_from_slice(
                format!(
                    "startxref\n{x}\n%%EOF\nxref\n0 1\n0000000000 65535 f\r\ntrailer\n<< /Size \
                     {} /Root {} 0 R /Prev {at} >>\nstartxref\n{t}\n%%EOF\n",
                    n + 1,
                    r.0
                )
                .as_bytes(),
            );
            return (out, at);
        }
    };
    out.extend_from_slice(format!("startxref\n{at}\n%%EOF\n").as_bytes());
    (out, at)
}

/// A cross-reference stream or object stream whose data other apps may decode differently
/// from lopdf is refused, after signing and when opening. lopdf decodes damaged Flate data
/// as far as it can, where some apps fail and rebuild the cross-reference information by
/// scanning the file, finding objects no section lists: a page turned after signing and
/// listed nowhere, behind a stream with its zlib header garbled, passed as unchanged, and
/// Poppler shows it turned. So did a predictor Poppler drops and lopdf applies, a filter
/// named only by its abbreviation, which lopdf doesn't read, and an object stream with an
/// empty list of filters, which lopdf decodes as nothing. Data in other filters, and
/// predictors the spec doesn't define, are refused too.
#[test]
fn a_stream_other_apps_cant_decode_is_refused() {
    let in_xref = |at| {
        ChangeKind::Malformed(format!(
            "the cross-reference stream at byte {at} holds data PDF apps may decode \
             differently: damaged, or not plain Flate data"
        ))
    };
    let b = approve(&base_pdf());
    let p = page(&Document::load_mem(&b).unwrap());
    let in_objstm = ChangeKind::Malformed(format!(
        "other PDF apps may read the document differently from LimarePDF (LimarePDF can't \
         read its object {} the way they do)",
        p.0
    ));
    let rotate = |d: &Document| dict(d, page(d)).get(b"Rotate").ok().cloned();
    for data in [
        Data::BadHeader,
        Data::BadChecksum,
        Data::NoChecksum,
        Data::CutOff,
        Data::Hex,
        Data::NoColors,
        Data::CutRow,
        Data::Tiff,
        Data::OddPredictor,
        Data::Abbreviated,
        Data::NoFilters,
    ] {
        for route in [Route::Stream, Route::Hybrid, Route::Packed] {
            let (bad, at) = behind_a_stream(&b, data, route);
            // lopdf reads the stream, and the page as signed. It reads a cross-reference
            // stream with no filters as empty, fails on one with a cut row, and rebuilds
            // the information; it fails on an object stream with a cut row.
            let unread = matches!(data, Data::NoFilters | Data::CutRow);
            if !matches!((data, route), (Data::CutRow, Route::Packed)) {
                let rebuilt = unread && matches!(route, Route::Stream);
                assert_eq!(
                    rotate(&Document::load_mem(&bad).unwrap()),
                    rebuilt.then_some(Object::Integer(180)),
                    "{data:?}, {route:?}"
                );
            }
            // Such a cross-reference stream lists nothing.
            let want = match (data, route) {
                (Data::CutRow, Route::Packed) => {
                    ChangeKind::Malformed(format!("LimarePDF can't read object {} reliably", p.0))
                }
                (_, Route::Packed) => in_objstm.clone(),
                _ if unread => ChangeKind::Malformed(format!(
                    "the cross-reference section at byte {at} is malformed"
                )),
                _ => in_xref(at),
            };
            assert_eq!(
                changes(&bad),
                [(want, Some(Refusal::Never))],
                "{data:?}, {route:?}"
            );
            let opened = Pdf::from_bytes(&bad, None).unwrap();
            assert!(opened.differs().is_some(), "{data:?}, {route:?}");
        }
    }
    // Unfiltered data and whole Flate data read alike everywhere, and no reader sees the
    // page listed nowhere.
    for data in [
        Data::Flate,
        Data::FlateInArray,
        Data::FlateThenNewline,
        Data::Predicted,
        Data::FilterNull,
    ] {
        for route in [Route::Stream, Route::Hybrid, Route::Packed] {
            let (ok, _) = behind_a_stream(&b, data, route);
            assert_eq!(changes(&ok), [], "{data:?}, {route:?}");
            let opened = Pdf::from_bytes(&ok, None).unwrap();
            assert!(opened.differs().is_none(), "{data:?}, {route:?}");
        }
    }
    // No reader takes anything from an object stream whose objects are listed elsewhere.
    for data in [
        Data::Flate,
        Data::BadHeader,
        Data::NoColors,
        Data::Abbreviated,
    ] {
        for route in [Route::Unused, Route::UnusedPacked] {
            let (ok, _) = behind_a_stream(&b, data, route);
            assert_eq!(changes(&ok), [], "{data:?}, {route:?}");
            let opened = Pdf::from_bytes(&ok, None).unwrap();
            assert!(opened.differs().is_none(), "{data:?}, {route:?}");
        }
    }
}

/// How [`behind_a_stream`] writes its stream's data.
#[derive(Clone, Copy, Debug)]
enum Data {
    /// Whole Flate data, `/Filter /FlateDecode`.
    Flate,
    /// The same, `/Filter [/FlateDecode]`.
    FlateInArray,
    /// Whole Flate data, a line end after it.
    FlateThenNewline,
    /// Whole Flate data with the PNG predictor rows of 7 bytes take.
    Predicted,
    /// Unfiltered data, `/Filter null`.
    FilterNull,
    /// Flate data with its zlib header garbled, which lopdf decodes as raw deflate data.
    BadHeader,
    /// Flate data with a wrong checksum.
    BadChecksum,
    /// Flate data ending before its checksum.
    NoChecksum,
    /// Flate data cut off part way, after everything the stream holds.
    CutOff,
    /// Hexadecimal data (`/ASCIIHexDecode`).
    Hex,
    /// Flate data with the PNG predictor and `/Colors 0`, which Poppler drops and lopdf
    /// takes as one color.
    NoColors,
    /// The PNG predictor's data, its last row cut short.
    CutRow,
    /// Flate data with the TIFF predictor, which Poppler and pdf.js decode differently
    /// from lopdf at some widths (16 bits, or rows padded out to a byte).
    Tiff,
    /// Flate data with predictor 3, which the spec doesn't define: lopdf and Poppler ignore
    /// it, and pdf.js fails.
    OddPredictor,
    /// Unfiltered data with `/F /FlateDecode`, which Poppler takes as `/Filter` and lopdf
    /// ignores.
    Abbreviated,
    /// Unfiltered data with `/Filter []`, which lopdf decodes as nothing and Poppler as
    /// unfiltered.
    NoFilters,
}

impl Data {
    /// The stream dictionary's filter entries and its data, holding `plain`.
    fn write(self, plain: &[u8]) -> (&'static str, Vec<u8>) {
        use std::io::Write;
        match self {
            Data::Hex => {
                let hex: String = plain.iter().map(|b| format!("{b:02x}")).collect();
                return ("/Filter /ASCIIHexDecode", format!("{hex}>").into_bytes());
            }
            Data::FilterNull => return ("/Filter null", plain.to_vec()),
            Data::Abbreviated => return ("/F /FlateDecode", plain.to_vec()),
            Data::NoFilters => return ("/Filter []", plain.to_vec()),
            _ => {}
        }
        let (plain, level) = match self {
            // Stored and padded, so what is left after a cut still holds all of `plain`.
            Data::CutOff => ([plain, &[b' '; 64]].concat(), flate2::Compression::none()),
            // Rows of 7 bytes padded out with spaces, each byte the difference from the one
            // before it.
            Data::Tiff => {
                let mut d = plain.to_vec();
                d.resize(plain.len().div_ceil(7) * 7, b' ');
                for row in d.chunks_mut(7) {
                    for i in (1..row.len()).rev() {
                        row[i] = row[i].wrapping_sub(row[i - 1]);
                    }
                }
                (d, flate2::Compression::default())
            }
            // Rows of 7 bytes padded out with spaces, each the differences from the row
            // above it (PNG's Up).
            Data::Predicted | Data::NoColors | Data::CutRow => {
                let mut padded = plain.to_vec();
                padded.resize(plain.len().div_ceil(7) * 7, b' ');
                let mut above = [0; 7];
                let mut d = Vec::new();
                for row in padded.chunks(7) {
                    d.push(2);
                    d.extend(row.iter().zip(above).map(|(b, a)| b.wrapping_sub(a)));
                    above.copy_from_slice(row);
                }
                if let Data::CutRow = self {
                    d.truncate(d.len() - 3);
                }
                (d, flate2::Compression::default())
            }
            _ => (plain.to_vec(), flate2::Compression::default()),
        };
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), level);
        z.write_all(&plain).unwrap();
        let mut z = z.finish().unwrap();
        match self {
            Data::BadHeader => z[..2].copy_from_slice(&[0, 0]),
            Data::BadChecksum => *z.last_mut().unwrap() ^= 1,
            Data::NoChecksum => z.truncate(z.len() - 4),
            Data::CutOff => z.truncate(z.len() - 4 - 32),
            Data::FlateThenNewline => z.extend_from_slice(b"\r\n"),
            _ => {}
        }
        let filter = match self {
            Data::FlateInArray => "/Filter [/FlateDecode]",
            Data::Predicted | Data::CutRow => {
                "/Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 7 >>"
            }
            Data::NoColors => {
                "/Filter /FlateDecode /DecodeParms << /Predictor 12 /Colors 0 /Columns 7 >>"
            }
            Data::Tiff => "/Filter /FlateDecode /DecodeParms << /Predictor 2 /Columns 7 >>",
            Data::OddPredictor => "/Filter /FlateDecode /DecodeParms << /Predictor 3 /Columns 7 >>",
            _ => "/Filter /FlateDecode",
        };
        (filter, z)
    }
}

/// Where [`behind_a_stream`] writes its data.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Route {
    /// In the revision's cross-reference stream.
    Stream,
    /// In the cross-reference stream a revision's table names with `/XRefStm`.
    Hybrid,
    /// In an object stream holding the page as signed.
    Packed,
    /// In an object stream holding the page as signed, which is listed as a plain object.
    Unused,
    /// In an object stream holding the page as signed, which is listed in another object
    /// stream.
    UnusedPacked,
}

/// A revision written by hand: page 1 turned and listed nowhere, which a reader rebuilding
/// the cross-reference information by scanning finds, then a cross-reference stream listing
/// only itself, its data written as `data`. In a hybrid revision, a table listing the
/// stream too names it. When packed, the page as signed is in an object stream written as
/// `data` before the turned copy, and the cross-reference stream, written plainly, lists it
/// there. When unused, nothing is turned, and the page as signed is listed elsewhere: written
/// plainly before the object stream holding it, or in an unfiltered object stream after it.
/// Returns the file and where the cross-reference stream starts.
fn behind_a_stream(b: &[u8], data: Data, route: Route) -> (Vec<u8>, usize) {
    let d = Document::load_mem(b).unwrap();
    let (r, p) = (root(&d), page(&d));
    let prev = last_startxref(b);
    let n = d.max_id + 1;
    let page = dict(&d, p);
    let mut turned = page.clone();
    turned.set("Rotate", 180);
    let mut out = [b, b"\n"].concat();
    // Rows of type and offset or container (`/W [1 4 2]`, the index 0).
    let row = |kind: u8, f1: usize| {
        let mut r = vec![kind];
        r.extend_from_slice(&u32::try_from(f1).unwrap().to_be_bytes());
        r.extend_from_slice(&[0, 0]);
        r
    };
    let mut rows = Vec::new();
    let packed = matches!(route, Route::Packed | Route::Unused | Route::UnusedPacked);
    let (x, index) = if packed {
        match route {
            Route::Unused => {
                rows.extend(row(1, out.len()));
                out.extend_from_slice(format!("{} 0 obj\n{page:?}\nendobj\n", p.0).as_bytes());
            }
            Route::UnusedPacked => rows.extend(row(2, (n + 1).try_into().unwrap())),
            _ => rows.extend(row(2, n.try_into().unwrap())),
        }
        let at = out.len();
        let head = format!("{} 0 ", p.0);
        let body = format!("{head}{page:?}");
        let (filter, z) = data.write(body.as_bytes());
        out.extend_from_slice(
            format!(
                "{n} 0 obj\n<< /Type /ObjStm /N 1 /First {} {filter} /Length {} >>\n\
                 stream\n",
                head.len(),
                z.len()
            )
            .as_bytes(),
        );
        out.extend_from_slice(&z);
        out.extend_from_slice(b"\nendstream\nendobj\n");
        rows.extend(row(1, at));
        if route == Route::UnusedPacked {
            rows.extend(row(1, out.len()));
            out.extend_from_slice(
                format!(
                    "{} 0 obj\n<< /Type /ObjStm /N 1 /First {} /Length {} >>\n\
                     stream\n{body}\nendstream\nendobj\n",
                    n + 1,
                    head.len(),
                    body.len()
                )
                .as_bytes(),
            );
            (n + 2, format!("{} 1 {n} 3", p.0))
        } else {
            (n + 1, format!("{} 1 {n} 2", p.0))
        }
    } else {
        (n, format!("{n} 1"))
    };
    if !matches!(route, Route::Unused | Route::UnusedPacked) {
        out.extend_from_slice(format!("{} 0 obj\n{turned:?}\nendobj\n", p.0).as_bytes());
    }
    let at = out.len();
    rows.extend(row(1, at));
    let (filter, rows) = if packed {
        (String::new(), rows)
    } else {
        let (filter, z) = data.write(&rows);
        (format!(" {filter}"), z)
    };
    let size = format!("/Size {}", x + 1);
    let trailer = format!("{size} /Root {} {} R /Prev {prev}", r.0, r.1);
    let own = if route == Route::Hybrid {
        &size
    } else {
        &trailer
    };
    out.extend_from_slice(
        format!(
            "{x} 0 obj\n<< /Type /XRef /W [1 4 2] /Index [{index}] {own}{filter} /Length {} \
             >>\nstream\n",
            rows.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&rows);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    let mut start = at;
    if route == Route::Hybrid {
        start = out.len();
        out.extend_from_slice(
            format!(
                "xref\n0 1\n0000000000 65535 f\r\n{x} 1\n{at:010} 00000 n\r\ntrailer\n<< \
                 {trailer} /XRefStm {at} >>\n"
            )
            .as_bytes(),
        );
    }
    out.extend_from_slice(format!("startxref\n{start}\n%%EOF\n").as_bytes());
    (out, at)
}

/// lopdf makes a stream of a dictionary followed by `stream` only with a line end after the
/// keyword, spaces and tabs aside; Poppler and pdf.js make one regardless, and finding no
/// data where they look, read the file again from its objects, taking any written in the
/// "data". The page written again as it was after signing, then `stream` and a comment, a
/// form feed or a letter, passed as unchanged, and they showed another page planted there.
/// Written again with `endobj`, it changes nothing.
#[test]
fn a_dictionary_followed_by_stream_without_a_line_end_is_refused() {
    let b = approve(&base_pdf());
    let doc = Document::load_mem(&b).unwrap();
    let (p, r) = (page(&doc), root(&doc));
    let want = ChangeKind::Malformed(format!(
        "other PDF apps may read the document differently from LimarePDF (LimarePDF can't \
         read its object {} the way they do)",
        p.0
    ));
    let rewritten = |after: &str| {
        let mut out = b.clone();
        out.push(b'\n');
        let at = out.len();
        let page = dict(&doc, p);
        out.extend_from_slice(format!("{} {} obj\n{page:?}\n{after}\n", p.0, p.1).as_bytes());
        let xref = out.len();
        out.extend_from_slice(
            format!(
                "xref\n0 1\n0000000000 65535 f \n{} 1\n{at:010} {:05} n \ntrailer\n<< /Size {} \
                 /Root {} {} R /Prev {} >>\nstartxref\n{xref}\n%%EOF\n",
                p.0,
                p.1,
                doc.max_id + 1,
                r.0,
                r.1,
                last_startxref(&b)
            )
            .as_bytes(),
        );
        out
    };
    for keyword in ["stream %", "stream\x0c", "stream x"] {
        assert_refused(&rewritten(&format!("{keyword}\nendstream\nendobj")), &want);
    }
    assert_eq!(changes(&rewritten("endobj")), vec![]);
}

/// Checking that other apps read each object as LimarePDF does reads no further than lopdf
/// reads: not on into a string after an object, a number or where an entry points, which
/// lopdf reads no further than its first byte, and which, never ended, runs on through the
/// rest of the file. 20,000 objects so followed took 15 s to open, 300 times as long as with
/// a letter in place of the string's first byte, and a minute at twice as many; the
/// signature read as valid.
#[test]
fn objects_followed_by_strings_are_read_as_fast_as_others() {
    const N: u32 = 20_000;
    let b = approve(&base_pdf());
    let first = Document::load_mem(&b).unwrap().max_id + 1;
    // `#` an object's number, `&` its partner's, `@` the string's first byte.
    let opened = |main: &str, partner: &str, opening: &str| {
        let text = |t: &str, n: u32| {
            t.replace('#', &n.to_string())
                .replace('&', &(n + N).to_string())
                .replace('@', opening)
                .into_bytes()
        };
        let mut objects: Vec<(u32, Vec<u8>)> =
            (first..first + N).map(|n| (n, text(main, n))).collect();
        if !partner.is_empty() {
            objects.extend((first..first + N).map(|n| (n + N, text(partner, n))));
        }
        let f = by_hand(&b, &objects, &[]);
        let start = std::time::Instant::now();
        let got = changes(&f);
        (start.elapsed(), got)
    };
    let mut slow = Vec::new();
    for (main, partner, opening) in [
        // After a number, looking for `N G R`, and after the object.
        ("# 0 obj\n5 @\n", "", "<"),
        ("# 0 obj\n5 @\n", "", "("),
        // After a dictionary, and after a number or two there, perhaps a header's.
        ("# 0 obj\n<< /A 5 >> @\n", "", "("),
        ("# 0 obj\n<< /A 5 >> 7 @\n", "", "("),
        ("# 0 obj\n<< /A 5 >> 7 0 @\n", "", "("),
        // In a header.
        ("# @\n", "", "("),
        // An object lopdf took from another entry.
        ("& 0 obj\n5 endobj\n", "& 0 obj\n@\n", "<"),
        // After an object stream's data, and after the object giving its length.
        (
            "# 0 obj\n<< /Type /ObjStm /N 0 /First 0 /Length 0 >>\nstream\n@\nendstream\nendobj\n",
            "",
            "(",
        ),
        (
            "# 0 obj\n<< /Type /ObjStm /N 0 /First 0 /Length & 0 R >>\nstream\n\nendstream\n\
             endobj\n",
            "& 0 obj\n0 @\n",
            "(",
        ),
    ] {
        let (took, got) = opened(main, partner, opening);
        let (twin, want) = opened(main, partner, "x");
        if took > twin * 20 {
            slow.push(format!(
                "{main:?} {partner:?} {opening}: {took:?}, {twin:?} with x"
            ));
        }
        assert_eq!(got, want, "{main:?} {partner:?} {opening}");
    }
    assert!(slow.is_empty(), "{slow:#?}");
}

/// lopdf and pdf.js read each object in an object stream at its offset, and Poppler in
/// windows as long as from one offset to the next, the first starting at `/First`. A page
/// turned after signing, written before the page as signed, passed as unchanged, and Poppler
/// showed it turned; so it did with a comment before the first object and the turned page
/// where that shifts Poppler's window for the page. Offsets out of order, an object running
/// past the next offset, `stream` after an object, and an object Poppler's window finds
/// inside a comment are refused too; white space, comments, `endobj` and a stray object
/// elsewhere are not. pdf.js reads on past the content, failing on a sign at its end, plain
/// or packed, on `-.` after packed data, where it reads zeros, and taking the last object
/// glued to `endstream` as a command. Data `/Length` frames short of a comment, plain or
/// packed, is refused too: other apps take the data as long as `/Length`, and lopdf on to
/// `endstream`.
#[test]
fn an_object_stream_other_apps_lay_out_differently_is_refused() {
    let b = approve(&base_pdf());
    let p = page(&Document::load_mem(&b).unwrap());
    let want = ChangeKind::Malformed(format!(
        "other PDF apps may read the document differently from LimarePDF (LimarePDF can't \
         read its object {} the way they do)",
        p.0
    ));
    for layout in [
        Layout::Gap,
        Layout::Shifted,
        Layout::Unordered,
        Layout::Overlong,
        Layout::Stream,
        Layout::InComment,
        Layout::SignAtEnd,
        Layout::PackedSignAtEnd,
        Layout::PackedPointAtEnd,
        Layout::Glued,
        Layout::Short,
        Layout::PackedCommented,
    ] {
        let bad = laid_out(&b, layout);
        // lopdf reads the page as signed.
        let d = Document::load_mem(&bad).unwrap();
        assert_eq!(dict(&d, p).get(b"Rotate").ok(), None, "{layout:?}");
        assert_eq!(
            changes(&bad),
            [(want.clone(), Some(Refusal::Never))],
            "{layout:?}"
        );
        let opened = Pdf::from_bytes(&bad, None).unwrap();
        assert!(opened.differs().is_some(), "{layout:?}");
    }
    for layout in [
        Layout::Plain,
        Layout::Spaced,
        Layout::EndObj,
        Layout::Stray,
        Layout::Packed,
        Layout::PointAtEnd,
        Layout::MinusInside,
        Layout::GluedNumber,
    ] {
        let ok = laid_out(&b, layout);
        assert_eq!(changes(&ok), [], "{layout:?}");
        let opened = Pdf::from_bytes(&ok, None).unwrap();
        assert!(opened.differs().is_none(), "{layout:?}");
    }
}

/// How [`laid_out`] lays out its object stream.
#[derive(Clone, Copy, Debug)]
enum Layout {
    /// The page, then `null`, each at its offset, one space between them.
    Plain,
    /// The same with comments and white space before the page and between the two, and a
    /// reference in place of `null`.
    Spaced,
    /// `endobj` between the page and the other object.
    EndObj,
    /// A string between the page and the other object.
    Stray,
    /// The page turned in a gap before the page's offset, the page as signed at it.
    Gap,
    /// A comment as long as the page turned, then the other object, the page turned, and
    /// the page as signed at its offset, the comment's length past the turned one.
    Shifted,
    /// The page turned, as the other object, then the page as signed: the page's offset
    /// above the other's.
    Unordered,
    /// The other object's offset inside the page.
    Overlong,
    /// Stream data after the page, which pdf.js reads as the page's.
    Stream,
    /// A 4-byte comment, then the other object and a comment ending 4 bytes before the
    /// page.
    InComment,
    /// As [`Layout::Plain`], Flate data.
    Packed,
    /// As [`Layout::Plain`], then `-.`, which pdf.js reads as 0 before the line end.
    PointAtEnd,
    /// As [`Layout::Plain`], then `-`, which pdf.js reads on to `endstream` and fails.
    SignAtEnd,
    /// The same, Flate data, after which pdf.js reads a zero and fails.
    PackedSignAtEnd,
    /// As [`Layout::PointAtEnd`], Flate data, after which pdf.js reads a zero and fails.
    PackedPointAtEnd,
    /// As [`Layout::Plain`], `endstream` right after it.
    Glued,
    /// The page, then `23`, `endstream` right after it, which every reader ends the number at.
    GluedNumber,
    /// As [`Layout::Plain`], the other object a real with a minus after its point, which
    /// every reader skips.
    MinusInside,
    /// As [`Layout::Plain`], then a comment `/Length` leaves out. Other apps take the data to
    /// `/Length`, skipping the comment before `endstream`; lopdf looks for `endstream`.
    Short,
    /// As [`Layout::Packed`], a comment after the data `/Length` frames.
    PackedCommented,
}

/// A revision written by hand: the page as signed and another object, unused, in an
/// unfiltered object stream laid out as `layout`, listed by a cross-reference stream. The
/// index is padded to past the first offset, which Poppler then reads from `/First`.
fn laid_out(b: &[u8], layout: Layout) -> Vec<u8> {
    let d = Document::load_mem(b).unwrap();
    let (r, p) = (root(&d), page(&d));
    let prev = last_startxref(b);
    let n = d.max_id + 1;
    let (q, x) = (n + 1, n + 2);
    let page = dict(&d, p);
    let mut turned = page.clone();
    turned.set("Rotate", 180);
    let (signed, turned) = (format!("{page:?}"), format!("{turned:?}"));
    // The objects' data and the page's and the other's offsets.
    let (body, at_p, at_q) = match layout {
        Layout::Plain | Layout::Packed | Layout::Glued | Layout::PackedCommented => {
            (format!("{signed} null"), 0, signed.len() + 1)
        }
        Layout::GluedNumber => (format!("{signed} 23"), 0, signed.len() + 1),
        Layout::MinusInside => (format!("{signed} 0.00-4142502"), 0, signed.len() + 1),
        Layout::Short => (format!("{signed} null %cut"), 0, signed.len() + 1),
        Layout::PointAtEnd | Layout::PackedPointAtEnd => {
            (format!("{signed} null -."), 0, signed.len() + 1)
        }
        Layout::SignAtEnd | Layout::PackedSignAtEnd => {
            (format!("{signed} null\n-"), 0, signed.len() + 1)
        }
        Layout::Spaced => {
            let head = "% packed\n\n";
            let body = format!("{head}{signed}\n% next\n  1 0 R");
            (body, head.len(), head.len() + signed.len() + 10)
        }
        Layout::EndObj => (format!("{signed}\nendobj\nnull"), 0, signed.len() + 8),
        Layout::Gap => {
            let at = turned.len() + 1;
            (
                format!("{turned}\n{signed} null"),
                at,
                at + signed.len() + 1,
            )
        }
        Layout::Shifted => {
            let k = turned.len() + 2;
            let body = format!("%{}\nnull {turned}\n\n{signed}", "-".repeat(k - 2));
            (body, 2 * k + 5, k)
        }
        Layout::Unordered => (format!("{turned}\n{signed}"), turned.len() + 1, 0),
        Layout::Overlong => (format!("{signed} null"), 0, signed.find('/').unwrap()),
        Layout::Stray => (format!("{signed} (stray) null"), 0, signed.len() + 9),
        Layout::Stream => {
            let body = format!("{signed}stream\nxx\nendstream\nnull");
            (body, 0, signed.len() + 20)
        }
        Layout::InComment => (format!("% a\nnull\n%b\n  {signed}"), 14, 4),
    };
    // The page is listed first unless the other object is first in the stream.
    let q_first = matches!(layout, Layout::Shifted | Layout::InComment);
    let pairs = if q_first {
        format!("{q} {at_q} {} {at_p}", p.0)
    } else {
        format!("{} {at_p} {q} {at_q}", p.0)
    };
    let index = format!("{pairs:<0$}\n", turned.len() + 2);
    let mut out = [b, b"\n"].concat();
    let at_c = out.len();
    let mut data = format!("{index}{body}").into_bytes();
    let mut filter = "";
    if matches!(
        layout,
        Layout::Packed
            | Layout::PackedSignAtEnd
            | Layout::PackedPointAtEnd
            | Layout::PackedCommented
    ) {
        use std::io::Write;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&data).unwrap();
        data = z.finish().unwrap();
        filter = " /Filter /FlateDecode";
    }
    out.extend_from_slice(
        format!(
            "{n} 0 obj\n<< /Type /ObjStm /N 2 /First {} /Length {}{filter} >>\nstream\n",
            index.len(),
            data.len()
                - if matches!(layout, Layout::Short) {
                    4
                } else {
                    0
                }
        )
        .as_bytes(),
    );
    out.extend_from_slice(&data);
    let eol = match layout {
        Layout::Glued | Layout::GluedNumber => "",
        Layout::PackedCommented => "%x\n",
        _ => "\n",
    };
    out.extend_from_slice(format!("{eol}endstream\nendobj\n").as_bytes());
    let at = out.len();
    // Rows of type, offset or container, and index (`/W [1 4 2]`): the page, the object
    // stream, the other object, the cross-reference stream.
    let row = |kind: u8, f1: usize, f2: u16| {
        let mut r = vec![kind];
        r.extend_from_slice(&u32::try_from(f1).unwrap().to_be_bytes());
        r.extend_from_slice(&f2.to_be_bytes());
        r
    };
    let n_at = usize::try_from(n).unwrap();
    let (i_p, i_q) = if q_first { (1, 0) } else { (0, 1) };
    let rows = [
        row(2, n_at, i_p),
        row(1, at_c, 0),
        row(2, n_at, i_q),
        row(1, at, 0),
    ]
    .concat();
    out.extend_from_slice(
        format!(
            "{x} 0 obj\n<< /Type /XRef /W [1 4 2] /Index [{} 1 {n} 3] /Size {} /Root {} {} R \
             /Prev {prev} /Length {} >>\nstream\n",
            p.0,
            x + 1,
            r.0,
            r.1,
            rows.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&rows);
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{at}\n%%EOF\n").as_bytes());
    out
}

/// An unfiltered object stream numbered `n`: `members` (number and offset), then `body`.
fn object_stream(n: u32, members: &[(u32, usize)], body: &str, n_entry: &str) -> Vec<u8> {
    let pairs: Vec<String> = members.iter().map(|(m, at)| format!("{m} {at}")).collect();
    let index = format!("{}\n", pairs.join(" "));
    let n_entry = if n_entry.is_empty() {
        members.len().to_string()
    } else {
        n_entry.to_string()
    };
    format!(
        "{n} 0 obj\n<< /Type /ObjStm /N {n_entry} /First {} /Length {} >>\nstream\n\
         {index}{body}\nendstream\nendobj\n",
        index.len(),
        index.len() + body.len()
    )
    .into_bytes()
}

/// A revision written by hand: `objects`, each listed under its number where it starts
/// (its text holds its own header, which may name another number), then `compressed`,
/// entries of number, container and index, all listed by a cross-reference stream.
fn by_hand(b: &[u8], objects: &[(u32, Vec<u8>)], compressed: &[(u32, u32, u16)]) -> Vec<u8> {
    let d = Document::load_mem(b).unwrap();
    let r = root(&d);
    let prev = last_startxref(b);
    let mut out = [b, b"\n"].concat();
    let mut rows: std::collections::BTreeMap<u32, (u8, u32, u16)> = Default::default();
    for (n, text) in objects {
        rows.insert(*n, (1, u32::try_from(out.len()).unwrap(), 0));
        out.extend_from_slice(text);
    }
    for &(n, c, i) in compressed {
        rows.insert(n, (2, c, i));
    }
    let x = rows.keys().copied().max().unwrap().max(d.max_id) + 1;
    rows.insert(x, (1, u32::try_from(out.len()).unwrap(), 0));
    let at = out.len();
    let index: Vec<String> = rows.keys().map(|n| format!("{n} 1")).collect();
    let mut data = Vec::new();
    for (kind, f1, f2) in rows.values() {
        data.push(*kind);
        data.extend_from_slice(&f1.to_be_bytes());
        data.extend_from_slice(&f2.to_be_bytes());
    }
    out.extend_from_slice(
        format!(
            "{x} 0 obj\n<< /Type /XRef /W [1 4 2] /Index [{}] /Size {} /Root {} {} R \
             /Prev {prev} /Length {} >>\nstream\n",
            index.join(" "),
            x + 1,
            r.0,
            r.1,
            data.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&data);
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{at}\n%%EOF\n").as_bytes());
    out
}

/// A revision written by hand with a cross-reference table listing object 0: `objects`, each
/// listed under its number where it starts, then `unlisted`, text no entry leads to; `free`
/// listed free; `more` added to the trailer.
fn in_table(
    b: &[u8],
    objects: &[(u32, Vec<u8>)],
    free: &[u32],
    unlisted: &[u8],
    more: &str,
) -> Vec<u8> {
    let d = Document::load_mem(b).unwrap();
    let r = root(&d);
    let prev = last_startxref(b);
    let mut out = [b, b"\n"].concat();
    let mut rows = "0 1\n0000000000 65535 f\r\n".to_string();
    for (n, text) in objects {
        rows.push_str(&format!("{n} 1\n{:010} 00000 n\r\n", out.len()));
        out.extend_from_slice(text);
    }
    out.extend_from_slice(unlisted);
    for n in free {
        rows.push_str(&format!("{n} 1\n0000000000 00001 f\r\n"));
    }
    let numbers = objects.iter().map(|(n, _)| n).chain(free);
    let size = numbers.copied().max().unwrap_or(0).max(d.max_id) + 1;
    let at = out.len();
    out.extend_from_slice(
        format!(
            "xref\n{rows}trailer\n<< /Size {size} /Root {} {} R /Prev {prev}{more} >>\n\
             startxref\n{at}\n%%EOF\n",
            r.0, r.1
        )
        .as_bytes(),
    );
    out
}

/// What [`by_hand`] writes for an object: its header, `body` and `endobj`.
fn written(n: u32, body: &str) -> Vec<u8> {
    format!("{n} 0 obj\n{body}\nendobj\n").into_bytes()
}

/// The page as signed, and turned, as written in an object stream.
fn page_texts(b: &[u8]) -> (ObjectId, String, String) {
    let d = Document::load_mem(b).unwrap();
    let p = page(&d);
    let mut turned = dict(&d, p);
    turned.set("Rotate", 90);
    (p, format!("{:?}", dict(&d, p)), format!("{turned:?}"))
}

/// The refusal of a later version that other apps read differently, for object `n`.
fn read_otherwise(n: u32) -> Vec<(ChangeKind, Option<Refusal>)> {
    vec![(
        ChangeKind::Malformed(format!(
            "other PDF apps may read the document differently from LimarePDF (LimarePDF \
             can't read its object {n} the way they do)"
        )),
        Some(Refusal::Never),
    )]
}

/// pdf.js takes an object stream's object as its own when the stream's number is the byte
/// offset the object's entry gives, whatever its generation and index, and keeps it for good:
/// a turned copy of the page in a stream numbered as the page's offset passed as nothing,
/// and pdf.js showed the page turned. Under another number it is a stray copy readers ignore.
#[test]
fn an_object_stream_numbered_as_an_objects_offset_is_refused() {
    let b = approve(&base_pdf());
    let (p, _, turned) = page_texts(&b);
    let d = Document::load_mem(&b).unwrap();
    let Some(lopdf::xref::XrefEntry::Normal { offset, .. }) = d.reference_table.get(p.0) else {
        panic!("the page isn't in an object of its own");
    };
    let copy = |c: u32| by_hand(&b, &[(c, object_stream(c, &[(p.0, 0)], &turned, ""))], &[]);
    let bad = copy(*offset);
    assert_eq!(changes(&bad), read_otherwise(p.0));
    assert!(Pdf::from_bytes(&bad, None).unwrap().differs().is_some());
    let ok = copy(offset + 1);
    assert_eq!(changes(&ok), []);
    assert!(Pdf::from_bytes(&ok, None).unwrap().differs().is_none());
}

/// lopdf keeps an object under the number its header gives. An object stream it can't open
/// (`/N` a reference), listed under number C, and another whose header names C too, listed
/// under another number: lopdf took the second as C, other apps the first. With the page
/// tree's root in C, a signature passed as valid though Poppler and pdf.js showed the first
/// stream's copy, which turns the page.
#[test]
fn an_object_stream_two_entries_lead_to_is_refused() {
    let base = base_pdf();
    let d = Document::load_mem(&base).unwrap();
    let pages = d
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    let mut turned = dict(&d, pages);
    turned.set("Rotate", 90);
    let (signed, turned) = (format!("{:?}", dict(&d, pages)), format!("{turned:?}"));
    let n = d.max_id + 1;
    let (c, k, count) = (n, n + 1, n + 2);
    let first = object_stream(c, &[(pages.0, 0)], &turned, &format!("{count} 0 R"));
    let second = object_stream(c, &[(pages.0, 0)], &signed, "");
    let base = by_hand(
        &base,
        &[(c, first), (k, second), (count, written(count, "1"))],
        &[(pages.0, c, 0)],
    );
    let b = approve(&base);
    let status = sig(&b, "Signature1").status;
    assert!(
        matches!(&status, SigStatus::CantCheck(why)
            if why.starts_with("other PDF apps may read the version")),
        "{status:?}"
    );
    assert!(Pdf::from_bytes(&b, None).unwrap().differs().is_some());
}

/// lopdf writes a stream's `/Length` as the length it read, so an object other apps may
/// read differently that serves only as a stream's `/Length` went unused, and its mark was
/// dropped. Here the number sits in an object stream laid out with something before it.
#[test]
fn an_object_read_otherwise_used_as_a_length_is_refused() {
    let b = approve(&base_pdf());
    let n = Document::load_mem(&b).unwrap().max_id + 1;
    let (c, len, s) = (n, n + 1, n + 2);
    let with = |body: &str, at| {
        let stream = format!("<< /Length {len} 0 R >>\nstream\nhello\nendstream");
        by_hand(
            &b,
            &[
                (c, object_stream(c, &[(len, at)], body, "")),
                (s, written(s, &stream)),
            ],
            &[(len, c, 0)],
        )
    };
    let bad = with("7\n5", 2);
    assert_eq!(changes(&bad), read_otherwise(len));
    assert!(Pdf::from_bytes(&bad, None).unwrap().differs().is_some());
    let ok = with("5", 0);
    assert_eq!(changes(&ok), []);
    assert!(Pdf::from_bytes(&ok, None).unwrap().differs().is_none());
}

/// Poppler and pdf.js take an object from an object stream whatever generation a reference
/// to it gives; lopdf only under generation 0. A page reached as `N 5 R` passed as missing
/// in lopdf's reading, and the others showed it.
#[test]
fn a_reference_to_an_object_in_a_stream_under_another_generation_is_refused() {
    let b = approve(&base_pdf());
    let n = Document::load_mem(&b).unwrap().max_id + 1;
    let (c, q, user) = (n, n + 1, n + 2);
    let with = |gen: u16| {
        by_hand(
            &b,
            &[
                (c, object_stream(c, &[(q, 0)], "<< /A 1 >>", "")),
                (user, written(user, &format!("[{q} {gen} R]"))),
            ],
            &[(q, c, 0)],
        )
    };
    let bad = with(5);
    assert_eq!(changes(&bad), read_otherwise(q));
    assert!(Pdf::from_bytes(&bad, None).unwrap().differs().is_some());
    let ok = with(0);
    assert_eq!(changes(&ok), []);
    assert!(Pdf::from_bytes(&ok, None).unwrap().differs().is_none());
}

/// lopdf and pdf.js take a key given twice as its last value, and so does Poppler's lookup
/// while the dictionary has fewer than 32 entries, the repeats counted; from 32 Poppler sorts
/// them and may take another, and of a font resource dictionary it takes the first, at any
/// size. The page written again with other page sizes before its own passed as unchanged, at
/// 32 entries, in its own object and in an object stream, and so did a trailer naming another
/// catalog before its own; below 32, the page naming a new font `/F1` before its own did, and
/// Poppler drew its text in the new font (review round 4). Values written alike read alike.
#[test]
fn a_dictionary_giving_a_key_twice_is_refused_unless_written_alike() {
    let b = approve(&base_pdf());
    let d = Document::load_mem(&b).unwrap();
    let (p, signed, _) = page_texts(&b);
    let own = dict(&d, p).len();
    let size = format!("{:?}", dict(&d, p).get(b"MediaBox").unwrap());
    // The page with `/MediaBox`, written `value`, given until it has `entries` entries.
    let sized = |entries: usize, value: &str| {
        let sizes = format!(" /MediaBox {value}").repeat(entries - own);
        format!("<<{sizes}{}", signed.strip_prefix("<<").unwrap())
    };
    let c = d.max_id + 1;
    // The page written again in its own object, or in an object stream when `packed`.
    let rewritten = |page: &str, packed| {
        if packed {
            let objstm = object_stream(c, &[(p.0, 0)], page, "");
            by_hand(&b, &[(c, objstm)], &[(p.0, c, 0)])
        } else {
            by_hand(&b, &[(p.0, written(p.0, page))], &[])
        }
    };
    let differs = |b: &[u8]| Pdf::from_bytes(b, None).unwrap().differs().is_some();
    for packed in [false, true] {
        for entries in [own + 1, 31, 32] {
            let bad = rewritten(&sized(entries, "[0 0 1 1]"), packed);
            assert_eq!(
                changes(&bad),
                read_otherwise(p.0),
                "{entries}, packed {packed}"
            );
            assert!(differs(&bad), "{entries}, packed {packed}");
            let ok = rewritten(&sized(entries, &size), packed);
            assert_eq!(changes(&ok), [], "{entries}, packed {packed}");
            assert!(!differs(&ok), "{entries}, packed {packed}");
        }
    }
    // A revision listing only object 0, whose trailer has `entries` entries, the first `/Root`
    // naming `first`.
    let r = root(&d);
    let trailer = |entries: usize, first: ObjectId| {
        let mut out = [&b[..], b"\n"].concat();
        let at = out.len();
        let more: String = (4..entries).map(|i| format!(" /E{i} 0")).collect();
        out.extend_from_slice(
            format!(
                "xref\n0 1\n0000000000 65535 f\r\ntrailer\n<< /Size {c} /Prev {}{more} /Root {} \
                 {} R /Root {} {} R >>\nstartxref\n{at}\n%%EOF\n",
                last_startxref(&b),
                first.0,
                first.1,
                r.0,
                r.1
            )
            .as_bytes(),
        );
        (out, at)
    };
    let (bad, at) = trailer(32, p);
    let want = ChangeKind::Malformed(format!(
        "the cross-reference section at byte {at} is malformed"
    ));
    assert_eq!(changes(&bad), [(want, Some(Refusal::Never))]);
    assert!(differs(&bad));
    for (entries, first) in [(31, p), (32, r)] {
        let (ok, _) = trailer(entries, first);
        assert_eq!(changes(&ok), [], "{entries}");
        assert!(!differs(&ok), "{entries}");
    }

    // A page drawing in font `/F1`, signed; then its fonts naming a new font `/F1` (or `/F#31`,
    // the same) before its own.
    let mut doc = Document::load_mem(&base_pdf()).unwrap();
    let helvetica =
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" };
    let resources = dictionary! { "Font" => dictionary! { "F1" => helvetica } };
    let mut drawn = dict(&doc, page(&doc));
    drawn.set("Resources", resources);
    doc.set_object(page(&doc), drawn);
    let mut unsigned = Vec::new();
    doc.save_to(&mut unsigned).unwrap();
    let b = approve(&unsigned);
    let d = Document::load_mem(&b).unwrap();
    let (p, signed, _) = page_texts(&b);
    assert!(signed.contains("/F1 <<"), "{signed}");
    let n = d.max_id + 1;
    let courier = written(n, "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>");
    for name in ["F1", "F#31"] {
        let twice = signed.replacen("/F1 <<", &format!("/F1 {n} 0 R /{name} <<"), 1);
        let bad = by_hand(
            &b,
            &[(p.0, written(p.0, &twice)), (n, courier.clone())],
            &[],
        );
        assert_eq!(changes(&bad), read_otherwise(p.0), "{name}");
        assert!(differs(&bad), "{name}");
    }
    let ok = by_hand(&b, &[(p.0, written(p.0, &signed)), (n, courier)], &[]);
    assert_eq!(changes(&ok), []);
    assert!(!differs(&ok));
}

/// Poppler, when it can't read the object a reference names and the newest cross-reference
/// section is a table, rebuilds its list of objects by scanning the file and reads every object
/// from that list; pdf.js reads the whole file again when a reference gives an object another
/// generation than its own. After signing, a revision whose table left a reference with nothing
/// to read, with a turned copy of the page after it that no entry lists, passed as nothing, and
/// Poppler showed the page turned. They follow references from the catalog and the document
/// information only, not from other keys of the trailer. Poppler follows them in what it draws
/// too, where its drawing looks in a dictionary: a comment added with its appearance drawing
/// an inline image whose filter's settings are an object nothing lists, or marking content
/// with such an object as its text, passed as an added comment (review round 4). A drawing
/// LimarePDF can't decode may refer to anything.
#[test]
fn a_reference_other_apps_read_the_file_again_for_is_refused() {
    let b = approve(&base_pdf());
    let d = Document::load_mem(&b).unwrap();
    let (p, signed, turned) = page_texts(&b);
    let n = d.max_id + 1;
    let (user, gone, freed) = (n, n + 1, n + 2);
    let dead = written(p.0, &turned);
    // The page as signed, referring to `num` under `gen` too.
    let page = |num: u32, gen: u16| {
        let rest = signed.strip_prefix("<<").unwrap();
        (p.0, written(p.0, &format!("<< /X [{num} {gen} R]{rest}")))
    };
    // Why the reading differs, if it does. The page changed is refused as such, which leaves
    // the reading's own refusal out.
    let differs = |b: &[u8]| {
        let pdf = Pdf::from_bytes(b, None).unwrap();
        pdf.differs().map(str::to_owned)
    };
    let read_otherwise = |b: &[u8], n: u32| {
        let why = format!("LimarePDF can't read its object {n} the way they do");
        let refused = changes(b).iter().any(|c| c.1 == Some(Refusal::Never));
        refused && differs(b) == Some(why)
    };
    let read_alike = |b: &[u8]| {
        let malformed = |c: &(ChangeKind, _)| matches!(c.0, ChangeKind::Malformed(_));
        differs(b).is_none() && !changes(b).iter().any(malformed)
    };
    // To a number nothing lists, or one listed free, in a table: refused, from the page or the
    // document information.
    let info = written(user, &format!("<< /Title (T) /X [{gone} 0 R] >>"));
    let as_info = format!(" /Info {user} 0 R");
    let bad = [
        (gone, in_table(&b, &[page(gone, 0)], &[], &dead, "")),
        (freed, in_table(&b, &[page(freed, 0)], &[freed], &dead, "")),
        (gone, in_table(&b, &[(user, info)], &[], &dead, &as_info)),
    ];
    for (i, (to, bad)) in bad.iter().enumerate() {
        assert!(read_otherwise(bad, *to), "{i}");
    }
    // In what a comment added draws, however Poppler reads a reference there: glued to the
    // numbers, or apart by a comment.
    let (note, form) = (n + 3, n + 4);
    let filtered = |data: &str, filter: &str| {
        let mut on = dict(&d, p);
        let mut annots = on
            .get(b"Annots")
            .ok()
            .and_then(|a| a.as_array().ok())
            .cloned()
            .unwrap_or_default();
        annots.push(Object::Reference((note, 0)));
        on.set("Annots", annots);
        let square = format!(
            "<< /Type /Annot /Subtype /Square /Rect [0 0 9 9] /P {} 0 R /AP << /N {form} 0 R >> >>",
            p.0
        );
        let len = data.len();
        let drawn = format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 9 9]{filter} /Length {len} >>\nstream\n\
             {data}\nendstream"
        );
        [
            (p.0, written(p.0, &format!("{on:?}"))),
            (note, written(note, &square)),
            (form, written(form, &drawn)),
        ]
    };
    let commented = |data: &str| filtered(data, "");
    let drawings = [
        format!("q BI /W 1 /H 1 /BPC 8 /CS /G /F {gone} 0 R ID x EI Q"),
        format!("/Span << /ActualText {gone} 0 R >> BDC EMC"),
        format!("/Span <</ActualText {gone}+0R>> BDC EMC"),
        format!("/Span <</ActualText {gone} -%c\nR>> BDC EMC"),
    ];
    for (i, data) in drawings.iter().enumerate() {
        let bad = in_table(&b, &commented(data), &[], &dead, "");
        assert!(read_otherwise(&bad, gone), "{i}");
        assert!(read_alike(&by_hand(&b, &commented(data), &[])), "{i}");
    }
    // To an object only an object stream holds, which no entry lists: the reading drops it, and
    // says it is used.
    let (stray, stm) = (n + 5, n + 6);
    let mut hiding = commented(&format!("/Span << /ActualText {stray} 0 R >> BDC EMC")).to_vec();
    hiding.push((stm, object_stream(stm, &[(stray, 0)], "(hidden)", "")));
    let why = format!(
        "it uses object {stray}, which its cross-reference information deletes or doesn't list"
    );
    assert_eq!(differs(&by_hand(&b, &hiding, &[])), Some(why));
    // To an object the file holds, or no reference (one in a string isn't): read alike.
    let held = format!("/Span << /ActualText {} 0 R >> BDC EMC", p.0);
    let alike = [held, format!("{gone} 0 0 RG {gone} 0. R ({gone} 0 R) Tj")];
    for (i, data) in alike.iter().enumerate() {
        let ok = in_table(&b, &commented(data), &[], &dead, "");
        assert!(read_alike(&ok), "{i}");
    }
    // Drawn data LimarePDF can't decode: what it refers to isn't known, in either.
    let unread = filtered("x", " /Filter /JBIG2Decode");
    let bad = [
        in_table(&b, &unread, &[], &dead, ""),
        by_hand(&b, &unread, &[]),
    ];
    for (i, bad) in bad.iter().enumerate() {
        assert!(read_otherwise(bad, form), "{i}");
    }
    // In a cross-reference stream Poppler reads nothing there, as lopdf and pdf.js do.
    assert!(read_alike(&by_hand(&b, &[page(gone, 0)], &[])));
    // From another key of the trailer, or an object nothing refers to: nothing read.
    let lone = (user, written(user, &format!("[{gone} 0 R]")));
    let more = format!(" /AdditionalStreams [/x {gone} 0 R]");
    let ok = in_table(&b, &[lone], &[], &dead, &more);
    assert_eq!(changes(&ok), []);
    assert!(differs(&ok).is_none());
    // To the page under another generation than its own: refused in either.
    let other = [page(p.0, p.1 + 1)];
    let bad = [
        in_table(&b, &other, &[], &dead, ""),
        by_hand(&b, &other, &[]),
    ];
    for (i, bad) in bad.iter().enumerate() {
        assert!(read_otherwise(bad, p.0), "{i}");
    }
    // To the page as listed: nothing to read again.
    assert!(read_alike(&in_table(&b, &[page(p.0, p.1)], &[], &dead, "")));
}

/// lopdf takes a stream's data as long as its `/Length` when a line end at most comes before
/// `endstream`, and otherwise on to an `endstream` of its own finding; a `/Length` written as
/// a real, `87.`, it takes whatever follows, and with no `/Length` it takes nothing. Poppler
/// and pdf.js take the data as long as a whole number `/Length` when `endstream` follows,
/// white space and comments aside, and otherwise look for `endstream` themselves: pdf.js for
/// its first eight bytes anywhere in the data, Poppler for the word, skipping comments, past
/// the first token at the end of the `/Length` it takes, 0 for a real. After signing, the
/// page's content written again with its `/Length` as a real and a red rectangle before
/// `endstream` passed as unchanged, and Poppler and pdf.js drew the rectangle (review round
/// 4); so did the content with its `/Length` ending inside a string, the rest of the line a
/// comment to them, and they left out the square drawn after it; and so did a `/Length` past
/// the data. Data differing by white space at its end reads alike.
#[test]
fn a_stream_framed_otherwise_is_refused() {
    let base = base_pdf();
    let d = Document::load_mem(&base).unwrap();
    let p = page(&d);
    let c = d.max_id + 1;
    // The content stream: `entries` in its dictionary, then `body`, its data and `endstream`.
    let content =
        |entries: &str, body: &str| written(c, &format!("<< {entries} >>\nstream\n{body}"));
    // A blue square with a red one in it, the red marked with a string starting `%`.
    let marked = "0 0 1 rg 10 10 50 50 re f /Span << /ActualText (";
    let drawn = format!("{marked}%) >> BDC 1 0 0 rg 20 20 30 30 re f EMC");
    let length = |n: usize| format!("/Length {n}");
    let real = |n: usize| format!("/Length {n}.");
    let l = drawn.len();
    let mut on = dict(&d, p);
    on.set("Contents", Object::Reference((c, 0)));
    let page = written(p.0, &format!("{on:?}"));
    let signed = content(&length(l), &format!("{drawn}\nendstream"));
    let b = approve(&by_hand(&base, &[(p.0, page), (c, signed)], &[]));
    let differs = |b: &[u8]| Pdf::from_bytes(b, None).unwrap().differs().is_some();
    let red = " 1 0 0 rg 0 0 300 200 re f";
    let span = |word: &str| format!("/Span << /ActualText ({word}\n) >> BDC{red} EMC");
    // lopdf reads the squares, and the content as signed; or something else, a change.
    let same = read_otherwise(c);
    let changed = vec![(ChangeKind::PageContent, Some(Refusal::Never))];
    for (i, (entries, body, read)) in [
        (real(l), format!("{drawn}\n{red}\nendstream"), &same),
        (length(marked.len()), format!("{drawn}\nendstream"), &same),
        // Poppler reads on past `endstream`.
        (length(l + 3), format!("{drawn}\nendstream"), &same),
        // lopdf takes nothing, the others the squares.
        (String::new(), format!("{drawn}\nendstream"), &changed),
        // pdf.js stops in the string, the others draw the rectangle.
        (
            length(0),
            format!("{drawn}\n{}\nendstream", span("endstrea")),
            &changed,
        ),
        (
            length(0),
            format!("{drawn}\n{}\nendstream", span("endsteam")),
            &changed,
        ),
        // Poppler takes the rest of the line for a comment, and draws the rectangle.
        (
            real(l + 3),
            format!("{drawn} %c  endstream\n{red}\nendstream"),
            &changed,
        ),
        // Poppler takes only the word `endstream`, and draws the rectangle.
        (
            real(l),
            format!("{drawn}\nendstreamx\n{red}\nendstream"),
            &same,
        ),
        // Poppler's first token, a string, runs past `endstream`; it draws the rectangle.
        (
            real(1),
            format!("(\nendstream\n){red}\nendstream"),
            &changed,
        ),
        // Poppler takes none of the data, a comment.
        (real(2), "%c\nendstream".into(), &changed),
    ]
    .iter()
    .enumerate()
    {
        let bad = by_hand(&b, &[(c, content(entries, body))], &[]);
        assert_eq!(&changes(&bad), *read, "{i}");
        assert!(differs(&bad), "{i}");
    }
    // Others take the data as long as its `/Length`, lopdf the white space after it too.
    let mut ok = vec![];
    for (framing, read) in [("\n", &[]), ("\r\n", &[]), ("\r", &[]), ("", &[])] {
        ok.push((length(l), format!("{drawn}{framing}endstream"), &read[..]));
    }
    for framing in ["\n\n", " \n", "\n\r"] {
        ok.push((
            length(l),
            format!("{drawn}{framing}endstream"),
            &changed[..],
        ));
    }
    // All look for `endstream`, or take the data as long as a real that ends it; Poppler
    // takes none of white space.
    ok.push((length(0), format!("{drawn}\nendstream"), &[]));
    ok.push((real(l), format!("{drawn}\nendstream"), &[]));
    ok.push((real(l), format!("{drawn}\n%{red}\nendstream"), &[]));
    ok.push((real(1), " \nendstream".into(), &changed));
    // In a file that isn't encrypted, lopdf doesn't cut the object at an `endobj` in its data:
    // all take the data as long as its `/Length`.
    let past = format!("{drawn}\nendstream\nendobj\n{red}");
    ok.push((length(past.len()), format!("{past}\nendstream"), &changed));
    for (i, (entries, body, read)) in ok.iter().enumerate() {
        let ok = by_hand(&b, &[(c, content(entries, body))], &[]);
        assert_eq!(changes(&ok), *read, "{i}");
        assert!(!differs(&ok), "{i}");
    }
}

#[test]
fn filling_in_a_form_is_allowed() {
    let b = fill(&approve(&form(&["Name", "Date"])), "Name", "Jane");
    let s = sig(&b, "Signature1");
    assert_eq!(
        summary(&s.changes),
        [(ChangeKind::FieldFilled("Name".into()), None)]
    );
    assert_eq!(s.changes[0].page, Some(0));
    assert_eq!(s.status, SigStatus::Valid);
    assert!(report(&b).all_ok());
}

#[test]
fn another_signature_is_allowed() {
    let b = countersign(&approve(&base_pdf()), "Signature2", false);
    assert_eq!(
        changes(&b),
        [(ChangeKind::Signed("Signature2".into()), None)]
    );
    assert!(sig(&b, "Signature2").changes.is_empty());
    assert!(report(&b).all_ok());

    // Signing an empty signature field added by the first signer.
    let with_field = edit(&approve(&base_pdf()), |prev, new| {
        let id = new.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Sig",
            "T" => Object::string_literal("Approver"), "P" => page(prev),
            "Rect" => vec![0.into(), 100.into(), 50.into(), 120.into()],
        });
        add_annot(prev, new, id);
        let r = root(prev);
        let mut c = dict(prev, r);
        let mut form = c.get(b"AcroForm").unwrap().as_dict().unwrap().clone();
        let mut fields = form.get(b"Fields").unwrap().as_array().unwrap().clone();
        fields.push(id.into());
        form.set("Fields", fields);
        c.set("AcroForm", form);
        new.set_object(r, Object::Dictionary(c));
    });
    assert_eq!(
        changes(&with_field),
        [(ChangeKind::SignatureFieldAdded("Approver".into()), None)]
    );
    let f = Field {
        name: "Approver",
        existing: true,
        ..Field::default()
    };
    let b = sign(&with_field, &id("p256"), &f, &Cms::default());
    assert_eq!(changes(&b), [(ChangeKind::Signed("Approver".into()), None)]);
    assert!(report(&b).all_ok());
}

#[test]
fn comments_are_allowed_unless_certified_without_them() {
    let added = comment(&approve(&base_pdf()), Dictionary::new());
    assert_eq!(changes(&added), [(ChangeKind::CommentAdded, None)]);
    // Removing a comment made before signing, and editing one, are comment changes too.
    let removed = edit(
        &approve(&comment(&base_pdf(), Dictionary::new())),
        |prev, new| {
            let p = page(prev);
            let mut d = dict(prev, p);
            let mut annots = d.get(b"Annots").unwrap().as_array().unwrap().clone();
            // The comment, then the signature's widget.
            let widget = annots.pop().unwrap();
            annots[0] = widget;
            d.set("Annots", annots);
            new.set_object(p, Object::Dictionary(d));
        },
    );
    assert_eq!(changes(&removed), [(ChangeKind::CommentRemoved, None)]);
    // Taking the signature's own widget off the page hides the signature.
    let hidden = edit(&approve(&base_pdf()), |prev, new| {
        let p = page(prev);
        let mut d = dict(prev, p);
        d.set("Annots", Vec::<Object>::new());
        new.set_object(p, Object::Dictionary(d));
    });
    assert_refused(&hidden, &ChangeKind::SignatureChanged("Signature1".into()));
    let edited = edit(
        &approve(&comment(&base_pdf(), Dictionary::new())),
        |prev, new| {
            let id = *prev
                .objects
                .iter()
                .find(|(_, o)| {
                    o.as_dict()
                        .is_ok_and(|d| d.has(b"Contents") && d.has(b"Rect"))
                })
                .unwrap()
                .0;
            let mut d = dict(prev, id);
            d.set("Contents", Object::string_literal("Changed"));
            new.set_object(id, Object::Dictionary(d));
        },
    );
    assert_eq!(changes(&edited), [(ChangeKind::CommentChanged, None)]);

    let p2 = comment(&certify(&base_pdf(), 2), Dictionary::new());
    assert_eq!(
        changes(&p2),
        [(
            ChangeKind::CommentAdded,
            Some(Refusal::Certification(Permissions::FormFilling))
        )]
    );
    let p3 = comment(&certify(&base_pdf(), 3), Dictionary::new());
    assert_eq!(changes(&p3), [(ChangeKind::CommentAdded, None)]);
}

#[test]
fn certification_limits() {
    // No changes: validation data and an invisible document timestamp only.
    let p1 = certify(&form(&["Name"]), 1);
    let dss = edit(&p1, |prev, new| {
        let cert = new.add_object(Stream::new(Dictionary::new(), cert_der("ca")));
        let r = root(prev);
        let mut c = dict(prev, r);
        c.set("DSS", dictionary! { "Certs" => vec![Object::from(cert)] });
        new.set_object(r, Object::Dictionary(c));
    });
    let stamped = timestamp(&dss, true);
    assert_eq!(
        changes(&stamped),
        [
            (ChangeKind::DocumentTimestamp("Timestamp1".into()), None),
            (ChangeKind::ValidationData, None),
        ]
    );
    assert!(report(&stamped).all_ok());
    let visible = timestamp(&p1, false);
    assert!(changes(&visible)[0].1.is_some());
    let filled = fill(&p1, "Name", "Jane");
    assert_eq!(
        changes(&filled),
        [(
            ChangeKind::FieldFilled("Name".into()),
            Some(Refusal::Certification(Permissions::NoChanges))
        )]
    );

    // Form filling and signing: a new signature field must be invisible.
    let p2 = certify(&form(&["Name"]), 2);
    assert!(report(&fill(&p2, "Name", "Jane")).all_ok());
    assert!(report(&countersign(&p2, "Signature2", true)).all_ok());
    assert_eq!(
        changes(&countersign(&p2, "Signature2", false)),
        [(
            ChangeKind::Signed("Signature2".into()),
            Some(Refusal::Certification(Permissions::FormFilling))
        )]
    );
}

#[test]
fn field_locks() {
    let b = locked_by(&form(&["Name", "Date"]), lock("Include", &["Name"], None));
    assert_eq!(
        changes(&fill(&b, "Name", "Jane")),
        [(
            ChangeKind::FieldFilled("Name".into()),
            Some(Refusal::Locked)
        )]
    );
    assert_eq!(
        changes(&fill(&b, "Date", "Today")),
        [(ChangeKind::FieldFilled("Date".into()), None)]
    );
    let all_but = locked_by(&form(&["Name", "Date"]), lock("Exclude", &["Date"], None));
    assert!(!report(&fill(&all_but, "Name", "Jane")).all_ok());
    assert!(report(&fill(&all_but, "Date", "Today")).all_ok());
    // "Lock document": nothing may change.
    let doc = locked_by(&base_pdf(), lock("All", &[], Some(1)));
    assert_eq!(
        changes(&comment(&doc, Dictionary::new())),
        [(ChangeKind::CommentAdded, Some(Refusal::Locked))]
    );
}

#[test]
fn document_information_may_change() {
    let b = approve(&base_pdf());
    let num = Document::load_mem(&b).unwrap().max_id + 1;
    let b = append_raw(
        &b,
        &[(num, b"<< /Title (New title) >>")],
        &[],
        &[],
        &format!("/Info {num} 0 R"),
    );
    assert_eq!(changes(&b), [(ChangeKind::Metadata, None)]);
}

#[test]
fn rewriting_strings_differently_is_no_change() {
    let b = approve(&base_pdf());
    let d = Document::load_mem(&b).unwrap();
    let field = titled(&d, "Signature1");
    let value = sig_id(&b, "Signature1");
    let p = page(&d);
    // The same field with its name as a hex string.
    let body = format!(
        "<< /Type /Annot /Subtype /Widget /FT /Sig /T <5369676E617475726531> /V {} 0 R \
         /F 132 /Rect [20 20 120 60] /P {} 0 R >>",
        value.0, p.0
    );
    let b = append_raw(&b, &[(field.0, body.as_bytes())], &[], &[], "");
    let s = sig(&b, "Signature1");
    assert!(!s.covers_whole_file);
    assert!(s.changes.is_empty(), "{:#?}", s.changes);
}

// ---------------------------------------------------------------- disallowed changes

#[test]
fn page_content_changes_are_refused() {
    let b = approve(&base_pdf());
    let c = contents(&b);
    let in_place = edit(&b, |_, new| {
        new.set_object(
            c,
            Stream::new(Dictionary::new(), b"0 0 m 100 100 l S".to_vec()),
        );
    });
    assert_refused(&in_place, &ChangeKind::PageContent);
    let replaced = edit(&b, |prev, new| {
        let s = new.add_object(Stream::new(Dictionary::new(), b"0 0 m 1 1 l S".to_vec()));
        let p = page(prev);
        let mut d = dict(prev, p);
        d.set("Contents", s);
        new.set_object(p, Object::Dictionary(d));
    });
    assert_refused(&replaced, &ChangeKind::PageContent);
    let rotated = edit(&b, |prev, new| {
        let p = page(prev);
        let mut d = dict(prev, p);
        d.set("Rotate", 90);
        new.set_object(p, Object::Dictionary(d));
    });
    assert_refused(&rotated, &ChangeKind::PageContent);
}

#[test]
fn adding_a_page_is_refused() {
    let b = edit(&approve(&base_pdf()), |prev, new| {
        let pages = dict(prev, root(prev))
            .get(b"Pages")
            .unwrap()
            .as_reference()
            .unwrap();
        let mut d = dict(prev, pages);
        let extra = new.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
        });
        let mut kids = d.get(b"Kids").unwrap().as_array().unwrap().clone();
        kids.push(extra.into());
        d.set("Kids", kids);
        d.set("Count", 2);
        new.set_object(pages, Object::Dictionary(d));
    });
    assert_refused(&b, &ChangeKind::PagesChanged);
}

#[test]
fn changing_a_signature_is_refused() {
    let b = approve(&base_pdf());
    let id = sig_id(&b, "Signature1");
    let reason = edit(&b, |prev, new| {
        let mut d = dict(prev, id);
        d.set("Reason", Object::string_literal("Forged"));
        new.set_object(id, Object::Dictionary(d));
    });
    assert_refused(&reason, &ChangeKind::SignatureChanged("Signature1".into()));
    let look = edit(&b, |prev, new| {
        let f = titled(prev, "Signature1");
        let ap = appearance(new, "BT (Someone else) Tj ET");
        let mut d = dict(prev, f);
        d.set("AP", dictionary! { "N" => ap });
        new.set_object(f, Object::Dictionary(d));
    });
    assert_refused(&look, &ChangeKind::SignatureChanged("Signature1".into()));
}

#[test]
fn new_fields_and_field_changes_are_refused() {
    let b = approve(&form(&["Name"]));
    let added = edit(&b, |prev, new| {
        let id = new.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
            "T" => Object::string_literal("Extra"), "P" => page(prev),
            "Rect" => vec![0.into(), 0.into(), 50.into(), 20.into()],
        });
        add_annot(prev, new, id);
        let r = root(prev);
        let mut c = dict(prev, r);
        let mut form = c.get(b"AcroForm").unwrap().as_dict().unwrap().clone();
        let mut fields = form.get(b"Fields").unwrap().as_array().unwrap().clone();
        fields.push(id.into());
        form.set("Fields", fields);
        c.set("AcroForm", form);
        new.set_object(r, Object::Dictionary(c));
    });
    assert_refused(&added, &ChangeKind::FieldAdded("Extra".into()));
    // Moving a field, or hiding it, isn't filling it in.
    let moved = edit(&b, |prev, new| {
        let f = titled(prev, "Name");
        let mut d = dict(prev, f);
        d.set("F", 6);
        new.set_object(f, Object::Dictionary(d));
    });
    assert_refused(&moved, &ChangeKind::FieldChanged("Name".into()));
}

/// The base document with fields `A`, with no kids, and `B`, with text field `G` and an
/// untitled field with no kids as its kids; `indirect`, the titles of `B` and `G` each in an
/// object of its own.
fn parent_form(indirect: bool) -> Vec<u8> {
    edit(&base_pdf(), |prev, new| {
        let title = |new: &mut Document, t: &str| match indirect {
            true => new.add_object(Object::string_literal(t)).into(),
            false => Object::string_literal(t),
        };
        let a = new.add_object(dictionary! {
            "T" => Object::string_literal("A"), "Kids" => Vec::<Object>::new(),
        });
        let b = new.new_object_id();
        let ap = appearance(new, "");
        let t = title(new, "G");
        let g = new.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
            "T" => t, "F" => 4, "P" => page(prev), "Parent" => b,
            "Rect" => vec![150.into(), 100.into(), 250.into(), 120.into()],
            "AP" => dictionary! { "N" => ap },
        });
        add_annot(prev, new, g);
        let u = new.add_object(dictionary! { "Kids" => Vec::<Object>::new(), "Parent" => b });
        let kids = vec![Object::from(g), Object::from(u)];
        let field = dictionary! { "T" => title(new, "B"), "Kids" => kids };
        new.set_object(b, Object::Dictionary(field));
        let r = root(prev);
        let mut c = dict(prev, r);
        c.set(
            "AcroForm",
            dictionary! {
                "Fields" => vec![Object::from(a), Object::from(b)],
                "DA" => Object::string_literal("/Helv 0 Tf 0 g"),
            },
        );
        new.set_object(r, Object::Dictionary(c));
    })
}

/// Field `id` as `new` has it so far, or else as `prev` has it, changed by `change`.
fn change(prev: &Document, new: &mut Document, id: ObjectId, change: impl FnOnce(&mut Dictionary)) {
    let mut d = (new.get_dictionary(id).cloned()).unwrap_or_else(|_| dict(prev, id));
    change(&mut d);
    new.set_object(id, Object::Dictionary(d));
}

/// Field `id` renamed `to`, in its title's object if the title has one of its own.
fn rename(prev: &Document, new: &mut Document, id: ObjectId, to: &str) {
    let to = Object::string_literal(to);
    match dict(prev, id).get(b"T").unwrap().as_reference() {
        Ok(t) => new.set_object(t, to),
        Err(_) => change(prev, new, id, |d| d.set("T", to)),
    }
}

/// `B.G` of [`parent_form`], locked by the signature, moved or renamed by edits to other fields
/// or to titles in objects of their own alone: listed among `A`'s kids too (its own parent still
/// `B`), named after the first list it's on it would be `A.G`; moved under `B`'s untitled kid,
/// still named `B.G`; `B` renamed `C`; or `G` renamed `H`. Each is refused as a change to
/// `B.G`, and filling it in is judged by the name it was signed with. Listed under `A`, filling
/// it in was allowed, as filling in a field the lock doesn't name; moved under the untitled kid,
/// so was moving it; with titles in objects of their own, so was renaming.
#[test]
fn a_field_renamed_by_changes_to_other_fields_is_refused() {
    let moved = (ChangeKind::FieldChanged("B.G".into()), Some(Refusal::Never));
    // `B`'s title in an object of its own changed is part of `B` changed: filling it in, which
    // the lock allows, beside `B` renamed, which nothing does.
    let title = (ChangeKind::FieldFilled("B".into()), None);
    let cases = [
        "listed under A",
        "moved under the untitled kid",
        "B renamed C",
        "G renamed H",
    ];
    let mut wrong = Vec::new();
    for (indirect, titles) in [(false, "titles in fields"), (true, "titles apart")] {
        let signed = locked_by(&parent_form(indirect), lock("Include", &["B.G"], None));
        for (case, filled) in cases.iter().flat_map(|c| [(c, false), (c, true)]) {
            let e = edit(&signed, |prev, new| {
                let (a, b, g) = (titled(prev, "A"), titled(prev, "B"), titled(prev, "G"));
                match *case {
                    "listed under A" => change(prev, new, a, |d| d.set("Kids", vec![g.into()])),
                    "moved under the untitled kid" => {
                        let kids = dict(prev, b).get(b"Kids").unwrap().clone();
                        let u = kids.as_array().unwrap()[1].as_reference().unwrap();
                        change(prev, new, u, |d| d.set("Kids", vec![g.into()]));
                        change(prev, new, b, |d| d.set("Kids", vec![u.into()]));
                    }
                    "B renamed C" => rename(prev, new, b, "C"),
                    _ => rename(prev, new, g, "H"),
                }
                if filled {
                    let ap = appearance(new, "BT (Jane) Tj ET");
                    change(prev, new, g, |d| {
                        d.set("V", Object::string_literal("Jane"));
                        d.set("AP", dictionary! { "N" => ap });
                    });
                }
            });
            let got = changes(&e);
            let allowed = got.iter().any(|c| c.1.is_none() && *c != title);
            if !got.contains(&moved) || allowed || report(&e).all_ok() {
                wrong.push(format!("{titles}, {case}, filled {filled}: {got:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A field listed again by another field than the one it's named by, or no longer: `B.G` of
/// [`parent_form`] under `A` listed after `B`, or under `B`'s untitled kid; text field `Name`
/// under signature field `Approver` listed after it, its kids in an array of their own; or
/// `Name` under a signature field added after signing. Each is refused as a change to that
/// field, after a lock naming it, an approval or a certification allowing no changes. So is a
/// new signature field's widget listed under `Name` too, as a change to `Name`, or in
/// `/Fields` too, as a field added. Listed twice by the same field or by `/Fields`, a field is
/// the same either way, so `A` and a new signature field listed twice are allowed. Only the
/// listing a field is named by was compared, so those refused were allowed before.
#[test]
fn a_field_listed_again_is_refused() {
    let id = |b: &[u8], name| titled(&Document::load_mem(b).unwrap(), name);
    // Field `name`'s kids in `b`.
    let kids = |b: &[u8], name| {
        let d = Document::load_mem(b).unwrap();
        dict(&d, titled(&d, name)).get(b"Kids").unwrap().clone()
    };
    // Field `owner`'s kids, or the kids array `owner`, made `kid`, or none.
    let list = |b: &[u8], owner: ObjectId, kid: Option<ObjectId>| {
        let kids: Vec<Object> = kid.into_iter().map(Object::from).collect();
        edit(b, |prev, new| match prev.get_object(owner).unwrap() {
            Object::Array(_) => new.set_object(owner, kids),
            _ => change(prev, new, owner, |d| d.set("Kids", kids)),
        })
    };
    let moved = |field: &str| ChangeKind::FieldChanged(field.into());
    let mut wrong = Vec::new();
    // `e` refused for `kind`, or allowed if none.
    let mut check = |what: &str, e: &[u8], kind: Option<ChangeKind>| {
        let (got, ok) = (changes(e), report(e).all_ok());
        let found = kind.map_or(ok, |k| !ok && got.contains(&(k, Some(Refusal::Never))));
        if !found {
            wrong.push(format!("{what}: {got:?}"));
        }
    };
    let fields = edit(&parent_form(false), |p, n| relist(p, n, |f| f.reverse()));
    let (a, g) = (id(&fields, "A"), id(&fields, "G"));
    let under_b = kids(&fields, "B");
    let u = under_b.as_array().unwrap()[1].as_reference().unwrap();
    let twice = list(&fields, a, Some(g));
    let approver = edit(&form(&["Name"]), |prev, new| {
        let kids = new.add_object(Vec::<Object>::new());
        let f = new.add_object(dictionary! {
            "FT" => "Sig", "T" => Object::string_literal("Approver"), "Kids" => kids,
        });
        list_field(prev, new, f);
    });
    let array = kids(&approver, "Approver").as_reference().unwrap();
    let name = id(&approver, "Name");
    let naming = lock("Include", &["B.G", "Name"], None);
    let sign = |b: &[u8], signer| match signer {
        "P1" => certify(b, 1),
        "an approval" => approve(b),
        _ => locked_by(b, naming.clone()),
    };
    // Each named after its field first.
    let cases = [
        ("B.G under A too", &fields, a, Some(g)),
        ("B.G under the untitled kid too", &fields, u, Some(g)),
        ("B.G no longer under A too", &twice, a, None),
        ("Name under Approver too", &approver, array, Some(name)),
    ];
    for signer in ["a lock naming it", "an approval", "P1"] {
        for (case, unsigned, owner, kid) in cases {
            let e = list(&sign(unsigned, signer), owner, kid);
            let field = case.split(' ').next();
            check(&format!("{case} after {signer}"), &e, field.map(moved));
        }
        let signed = sign(&fields, signer);
        let e = edit(&signed, |p, n| relist(p, n, |l| l.push(a.into())));
        check(&format!("A listed twice after {signer}"), &e, None);
    }
    // Signature field `Extra` added with a widget: the field and the widget.
    let add_extra = |prev: &Document, new: &mut Document| {
        let (f, w) = (new.new_object_id(), new.new_object_id());
        let widget = dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "P" => page(prev), "Parent" => f,
            "Rect" => vec![0.into(), 0.into(), 0.into(), 0.into()],
        };
        new.set_object(w, widget);
        add_annot(prev, new, w);
        let kids = vec![Object::from(w)];
        let field = dictionary! {
            "FT" => "Sig", "T" => Object::string_literal("Extra"), "Kids" => kids,
        };
        new.set_object(f, field);
        (f, w)
    };
    // After an approval, which allows adding it.
    let approved = approve(&form(&["Name"]));
    let e = edit(&approved, |prev, new| {
        let kids = vec![Object::from(titled(prev, "Name"))];
        let f = new.add_object(dictionary! {
            "FT" => "Sig", "T" => Object::string_literal("Extra"), "Kids" => kids,
        });
        list_field(prev, new, f);
    });
    check("Name under new Extra too", &e, Some(moved("Name")));
    let e = edit(&approved, |prev, new| {
        let (f, w) = add_extra(prev, new);
        let (to, kids) = (titled(prev, "Name"), vec![Object::from(w)]);
        change(prev, new, to, |d| d.set("Kids", kids));
        list_first(prev, new, f);
    });
    check("Extra's widget under Name too", &e, Some(moved("Name")));
    let e = edit(&approved, |prev, new| {
        let (f, w) = add_extra(prev, new);
        relist(prev, new, |l| l.extend([Object::from(f), Object::from(w)]));
    });
    let added = ChangeKind::FieldAdded("Extra".into());
    check("Extra's widget in /Fields too", &e, Some(added));
    let e = edit(&approved, |prev, new| {
        let (f, _) = add_extra(prev, new);
        relist(prev, new, |l| l.extend([Object::from(f), Object::from(f)]));
    });
    check("Extra listed twice", &e, None);
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The base document with widget `G` on the page and its `/Parent` text field `B`, valued
/// `old`: `B` listed nowhere, with `G` its kid, and `G` alone in `/Fields` (`G listed`) or
/// `/Fields` empty (`G unlisted`); both in `/Fields`, `B` with no kids (`B listed apart`); or
/// `B` alone in `/Fields`, with `G` its kid (`B listed`).
fn parent_apart(shape: &str) -> Vec<u8> {
    edit(&base_pdf(), |prev, new| {
        let b = new.new_object_id();
        let ap = appearance(new, "");
        let g = new.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "T" => Object::string_literal("G"),
            "F" => 4, "P" => page(prev), "Parent" => b,
            "Rect" => vec![150.into(), 100.into(), 250.into(), 120.into()],
            "AP" => dictionary! { "N" => ap },
        });
        add_annot(prev, new, g);
        let apart = shape == "B listed apart";
        let kids: Vec<Object> = if apart { vec![] } else { vec![g.into()] };
        new.set_object(
            b,
            dictionary! {
                "T" => Object::string_literal("B"), "FT" => "Tx",
                "V" => Object::string_literal("old"), "Kids" => kids,
            },
        );
        let fields: Vec<Object> = match shape {
            "G listed" => vec![g.into()],
            "G unlisted" => vec![],
            "B listed" => vec![b.into()],
            _ => vec![g.into(), b.into()],
        };
        let r = root(prev);
        let mut c = dict(prev, r);
        c.set(
            "AcroForm",
            dictionary! { "Fields" => fields, "DA" => Object::string_literal("/Helv 0 Tf 0 g") },
        );
        new.set_object(r, Object::Dictionary(c));
    })
}

/// A widget's `/Parent` other than the field listing it, which readers following `/Parent`
/// take its name, type, value and actions from: `B` of [`parent_apart`], listed nowhere,
/// renamed and refilled, refilled, made a list, or given a script. Each is refused after a
/// certification, an approval or a lock naming the widget either way, as a change to field
/// `G`, or to an annotation with `G` not in `/Fields`; so is filling `G` in, named `B.G` by
/// such readers, with `B` listed nowhere or listed apart. Only filling `G` in after a lock
/// naming `G` or a certification allowing no changes was refused before. With `B` listed and
/// `G` its kid, filling `G` in is still allowed after a certification allowing it or an
/// approval.
#[test]
fn a_field_s_parent_other_than_its_listing_is_refused() {
    // An edit to `B`'s dictionary.
    type FieldEdit = fn(&mut Dictionary);
    let edits: [(&str, FieldEdit); 4] = [
        ("B renamed C and refilled", |d| {
            d.set("T", Object::string_literal("C"));
            d.set("V", Object::string_literal("new"));
        }),
        ("B refilled", |d| d.set("V", Object::string_literal("new"))),
        ("B made a list", |d| d.set("FT", "Ch")),
        ("B given a script", |d| {
            let js = Object::string_literal("app.alert(1)");
            let action = dictionary! { "S" => "JavaScript", "JS" => js };
            d.set("AA", dictionary! { "F" => action });
        }),
    ];
    let signers = [
        "P1",
        "P2",
        "an approval",
        "a lock naming B.G",
        "a lock naming G",
    ];
    let sign = |b: &[u8], signer: &str| match signer {
        "P1" => certify(b, 1),
        "P2" => certify(b, 2),
        "an approval" => approve(b),
        "a lock naming B.G" => locked_by(b, lock("Include", &["B.G"], None)),
        _ => locked_by(b, lock("Include", &["G"], None)),
    };
    let mut wrong = Vec::new();
    for shape in ["G listed", "G unlisted", "B listed apart", "B listed"] {
        let want = match shape {
            "G unlisted" => (ChangeKind::AnnotationChanged, Some(Refusal::Never)),
            _ => (ChangeKind::FieldChanged("G".into()), Some(Refusal::Never)),
        };
        for signer in signers {
            let signed = sign(&parent_apart(shape), signer);
            // `B` listed is a field of its own, its changes judged as such.
            let edits = if shape.starts_with("B listed") {
                &[][..]
            } else {
                &edits[..]
            };
            for (what, edit_b) in edits {
                let e = edit(&signed, |prev, new| {
                    let parent = dict(prev, titled(prev, "G")).get(b"Parent").cloned();
                    change(prev, new, parent.unwrap().as_reference().unwrap(), edit_b);
                });
                let got = changes(&e);
                let allowed = got.iter().any(|c| c.1.is_none());
                if !got.contains(&want) || allowed || report(&e).all_ok() {
                    wrong.push(format!("{shape}, {what} after {signer}: {got:?}"));
                }
            }
            if shape != "G unlisted" {
                let e = edit(&signed, |prev, new| {
                    let ap = appearance(new, "BT (Jane) Tj ET");
                    change(prev, new, titled(prev, "G"), |d| {
                        d.set("V", Object::string_literal("Jane"));
                        d.set("AP", dictionary! { "N" => ap });
                    });
                });
                let got = changes(&e);
                let mistaken = match shape {
                    "B listed" if ["P2", "an approval"].contains(&signer) => !report(&e).all_ok(),
                    "B listed" => false,
                    _ => !got.contains(&want) || report(&e).all_ok(),
                };
                if mistaken {
                    wrong.push(format!("{shape}, G filled after {signer}: {got:?}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn actions_and_scripts_are_refused() {
    let b = approve(&base_pdf());
    let js = dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("app.alert(1)") };
    let open = edit(&b, |prev, new| {
        let r = root(prev);
        let mut c = dict(prev, r);
        c.set("OpenAction", js.clone());
        new.set_object(r, Object::Dictionary(c));
    });
    assert_refused(&open, &ChangeKind::Actions);
    // A comment that runs a script when the page opens isn't just a comment.
    let note = comment(&b, dictionary! { "AA" => dictionary! { "PO" => js } });
    assert_refused(&note, &ChangeKind::Actions);
    // A link isn't a comment.
    let link = edit(&b, |prev, new| {
        let id = new.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link",
            "Rect" => vec![0.into(), 0.into(), 300.into(), 200.into()],
            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com") },
        });
        add_annot(prev, new, id);
    });
    assert_refused(&link, &ChangeKind::AnnotationChanged);
}

/// What a signature made in each way allows: an ordinary one, and certifications allowing
/// form filling and signing (P2), and comments too (P3).
fn signed_three_ways() -> [(Vec<u8>, &'static str); 3] {
    [
        (approve(&base_pdf()), "an ordinary signature"),
        (certify(&base_pdf(), 2), "P2"),
        (certify(&base_pdf(), 3), "P3"),
    ]
}

#[test]
fn a_new_signature_field_s_actions_are_refused() {
    // The ways a new signature field can carry an action (A-03).
    let actions: [(&str, bool, Entries); 7] = [
        ("/AA", false, |_| on_open()),
        ("/A", false, |_| dictionary! { "A" => next_page() }),
        ("/AA an object of its own", false, |new| {
            dictionary! { "AA" => new.add_object(dictionary! { "PO" => next_page() }) }
        }),
        ("an action in /AA an object of its own", false, |new| {
            dictionary! { "AA" => dictionary! { "PO" => new.add_object(next_page()) } }
        }),
        ("/A with /Next to a script", false, |new| {
            let js = new.add_object(dictionary! {
                "S" => "JavaScript", "JS" => Object::string_literal("app.alert(1)"),
            });
            let mut a = next_page();
            a.set("Next", js);
            dictionary! { "A" => a }
        }),
        ("/AA on a kid", true, |_| on_open()),
        ("/A on a kid", true, |_| dictionary! { "A" => next_page() }),
    ];
    let added = (ChangeKind::SignatureFieldAdded("Extra".into()), None);
    let mut wrong = Vec::new();
    for (signed, after) in signed_three_ways() {
        // Without an action, allowed as before; a null one is none.
        let none: [(&str, Entries); 2] = [
            ("no action", |_| Dictionary::new()),
            ("/AA null", |_| dictionary! { "AA" => Object::Null }),
        ];
        for ((what, make), kid) in none.into_iter().flat_map(|n| [(n, false), (n, true)]) {
            let b = extra_field(&signed, kid, make);
            let got = changes(&b);
            if got != [added.clone()] || !report(&b).all_ok() {
                wrong.push(format!("{what} (kid {kid}) after {after}: {got:?}"));
            }
        }
        for (what, kid, make) in actions {
            let b = extra_field(&signed, kid, make);
            let got = changes(&b);
            let want = [(ChangeKind::Actions, Some(Refusal::Never)), added.clone()];
            if got != want || report(&b).all_ok() {
                wrong.push(format!("{what} after {after}: {got:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A signature field added after signing with a text field of its own among its kids: the
/// text field is a field added, which nothing allows, listed beside the signature field. It was
/// reported as nothing before, the signature field's adding allowed after an approval or a
/// certification allowing signing.
#[test]
fn a_field_inside_a_new_signature_field_is_refused() {
    let held = ChangeKind::FieldAdded("Extra.Text".into());
    let held = (held, Some(Refusal::Never));
    let signed = [
        (approve(&base_pdf()), "an approval"),
        (certify(&base_pdf(), 2), "P2"),
        (certify(&base_pdf(), 1), "P1"),
    ];
    let mut wrong = Vec::new();
    for (signed, after) in signed {
        let b = edit(&signed, |prev, new| {
            let top = new.new_object_id();
            let w = new.add_object(dictionary! {
                "Type" => "Annot", "Subtype" => "Widget", "P" => page(prev), "Parent" => top,
                "Rect" => vec![Object::from(0); 4],
            });
            let t = new.add_object(dictionary! {
                "Type" => "Annot", "Subtype" => "Widget", "P" => page(prev), "Parent" => top,
                "Rect" => vec![Object::from(0); 4],
                "FT" => "Tx", "T" => Object::string_literal("Text"),
            });
            add_annot(prev, new, w);
            add_annot(prev, new, t);
            let kids = vec![Object::from(w), t.into()];
            let field = dictionary! {
                "FT" => "Sig", "T" => Object::string_literal("Extra"), "Kids" => kids,
            };
            new.set_object(top, Object::Dictionary(field));
            list_field(prev, new, top);
        });
        let got = changes(&b);
        if !got.contains(&held) || report(&b).all_ok() {
            wrong.push(format!("after {after}: {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_signature_field_signed_as_it_is_added_with_an_action_is_refused() {
    let mut wrong = Vec::new();
    for (signed, after) in signed_three_ways() {
        for action in [false, true] {
            let f = Field {
                name: "Signature2",
                invisible: true,
                widget: if action { on_open() } else { Dictionary::new() },
                ..Field::default()
            };
            let b = sign(&signed, &id("p256"), &f, &Cms::default());
            let got = changes(&b);
            let mut want = vec![(ChangeKind::Signed("Signature2".into()), None)];
            if action {
                want.insert(0, (ChangeKind::Actions, Some(Refusal::Never)));
            }
            if got != want || report(&b).all_ok() == action {
                wrong.push(format!("action {action} after {after}: {got:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn signing_an_existing_field_while_adding_an_action_is_refused() {
    let f = Field {
        name: "Extra",
        existing: true,
        widget: on_open(),
        ..Field::default()
    };
    // In the version the first signature signed: a changed field.
    let before = approve(&extra_field(&base_pdf(), false, |_| Dictionary::new()));
    let b = sign(&before, &id("p256"), &f, &Cms::default());
    let got = changes(&b);
    assert!(got.iter().any(|(_, r)| r.is_some()), "{got:#?}");
    assert!(!report(&b).all_ok());
    // Added after it: a new field to the first signature, its action that field's.
    let after = extra_field(&approve(&base_pdf()), false, |_| Dictionary::new());
    let b = sign(&after, &id("p256"), &f, &Cms::default());
    let want = vec![
        (ChangeKind::Actions, Some(Refusal::Never)),
        (ChangeKind::Signed("Extra".into()), None),
    ];
    assert_eq!(changes(&b), want);
    assert!(!report(&b).all_ok());
}

/// `b` with an empty signature field `Deep` on the page, `depth` untitled fields below the
/// top of the form.
fn deep_signature_field(b: &[u8], depth: usize) -> Vec<u8> {
    edit(b, |prev, new| {
        let p = page(prev);
        let w = new.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Sig",
            "T" => Object::string_literal("Deep"), "F" => 4, "P" => p,
            "Rect" => vec![150.into(), 100.into(), 250.into(), 140.into()],
        });
        add_annot(prev, new, w);
        let mut top = w;
        for _ in 0..depth {
            top = new.add_object(dictionary! { "Kids" => vec![Object::from(top)] });
        }
        let r = root(prev);
        let mut c = dict(prev, r);
        c.set(
            "AcroForm",
            dictionary! { "Fields" => vec![Object::from(top)] },
        );
        new.set_object(r, Object::Dictionary(c));
    })
}

/// Gives field (or widget) `id` a made-up signature (a document timestamp if `stamp`), not a
/// real one, and a drawing.
fn fake_sign(prev: &Document, new: &mut Document, id: ObjectId, stamp: bool) {
    let v = new.add_object(dictionary! {
        "Type" => if stamp { "DocTimeStamp" } else { "Sig" },
        "Filter" => "Adobe.PPKLite",
        "SubFilter" => if stamp { "ETSI.RFC3161" } else { "adbe.pkcs7.detached" },
        "ByteRange" => vec![0.into(), 10.into(), 20.into(), 10.into()],
        "Contents" => Object::String(vec![0; 4], lopdf::StringFormat::Hexadecimal),
    });
    let ap = appearance(new, "BT (PAID) Tj ET");
    let mut d = dict(prev, id);
    d.set("V", v);
    d.set("AP", dictionary! { "N" => ap });
    new.set_object(id, Object::Dictionary(d));
}

/// `b` with a made-up signature in field `Deep` (see [`fake_sign`]).
fn fake_signature(b: &[u8], stamp: bool) -> Vec<u8> {
    edit(b, |prev, new| {
        fake_sign(prev, new, titled(prev, "Deep"), stamp)
    })
}

/// The first kid field `id` lists.
fn first_kid(d: &Document, id: ObjectId) -> ObjectId {
    let field = dict(d, id);
    let kids = field.get(b"Kids").unwrap().as_array().unwrap();
    kids[0].as_reference().unwrap()
}

/// A field signed after a certification allowing no changes (with a timestamp) or after an
/// ordinary signature, made up both times: at the top of the form the report lists it (as
/// broken) and the signing is allowed; nested 33 deep, past what the report lists, nothing
/// would check it, so it's refused.
#[test]
fn a_signature_the_report_doesnt_list_is_refused() {
    let mut wrong = Vec::new();
    for depth in [0, 33] {
        let field = deep_signature_field(&base_pdf(), depth);
        for (stamp, signed) in [(true, certify(&field, 1)), (false, approve(&field))] {
            let b = fake_signature(&signed, stamp);
            let r = report(&b);
            let listed = (r.signatures.iter()).any(|s| s.fields.iter().any(|f| f == "Deep"));
            let kind = match stamp {
                true => ChangeKind::DocumentTimestamp("Deep".into()),
                false => ChangeKind::Signed("Deep".into()),
            };
            let got = changes(&b);
            let fine = match listed {
                true => depth == 0 && got == [(kind, None)],
                false => depth == 33 && !got.iter().any(|(k, r)| *k == kind || r.is_none()),
            };
            if !fine || got.is_empty() {
                wrong.push(format!("depth {depth}, timestamp {stamp}: {got:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// An empty signature field given, after a certification allowing no changes or after an
/// approval and a document timestamp, the timestamp's signature or the first one, and a
/// drawing: the report lists the earlier signature in it too, but nothing was signed in that
/// step, so it's refused. It was allowed as that field's signing or timestamp. A copy of the
/// timestamp's dictionary in an object of its own is a signature the report finds copied.
#[test]
fn an_earlier_signature_put_in_a_field_is_refused() {
    let field = extra_field(&base_pdf(), false, |_| Dictionary::new());
    let what = "a signature in field \"Extra\" it can't check";
    let refused = (ChangeKind::Unrecognised(what.into()), Some(Refusal::Never));
    let mut wrong = Vec::new();
    for (signed, after) in [(certify(&field, 1), "P1"), (approve(&field), "an approval")] {
        let stamped = timestamp(&signed, true);
        for from in ["Timestamp1", "Signature1"] {
            let b = edit(&stamped, |prev, new| {
                let v = dict(prev, titled(prev, from)).get(b"V").unwrap().clone();
                let extra = titled(prev, "Extra");
                let mut d = dict(prev, extra);
                let look = appearance(new, "BT (PAID) Tj ET");
                d.set("V", v);
                d.set("AP", dictionary! { "N" => look });
                new.set_object(extra, Object::Dictionary(d));
            });
            let got = changes(&b);
            let credited = |k: &ChangeKind| {
                let in_extra = |n: &String| n == "Extra";
                matches!(k, ChangeKind::Signed(n) | ChangeKind::DocumentTimestamp(n) if in_extra(n))
            };
            let fine = got.contains(&refused) && !got.iter().any(|(k, _)| credited(k));
            if !fine || report(&b).all_ok() {
                wrong.push(format!("after {after}, {from}'s: {got:?}"));
            }
        }
        // A copy in an object of its own: a new signature, found copied.
        let b = edit(&stamped, |prev, new| {
            let stamp = dict(prev, titled(prev, "Timestamp1"));
            let v = stamp.get(b"V").unwrap().as_reference().unwrap();
            let copy = new.add_object(dict(prev, v));
            let extra = titled(prev, "Extra");
            let mut d = dict(prev, extra);
            d.set("V", copy);
            new.set_object(extra, Object::Dictionary(d));
        });
        let status = sig(&b, "Extra").status;
        let copied = matches!(&status,
            SigStatus::Broken(why) if why.contains("added after the version"));
        if !copied || report(&b).all_ok() {
            wrong.push(format!("after {after}, a copy: {status:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A made-up signature on the widget of an empty signature field rather than on the field:
/// the report reads a field's signature from the field, so it doesn't list this one, and with
/// nothing to check it the signing is refused. It was allowed (after an ordinary signature,
/// and as a document timestamp after a certification allowing no changes).
#[test]
fn a_signature_on_a_fields_widget_is_refused() {
    let field = extra_field(&base_pdf(), true, |_| Dictionary::new());
    let what = "a signature in field \"Extra\" it can't check";
    let want = [(ChangeKind::Unrecognised(what.into()), Some(Refusal::Never))];
    let mut wrong = Vec::new();
    for (stamp, signed) in [(true, certify(&field, 1)), (false, approve(&field))] {
        let b = edit(&signed, |prev, new| {
            let w = first_kid(prev, titled(prev, "Extra"));
            fake_sign(prev, new, w, stamp);
        });
        let got = changes(&b);
        if got != want || report(&b).all_ok() {
            wrong.push(format!("timestamp {stamp}: {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// An empty signature field whose widget is a kid with a drawing of its own, signed after an
/// approval and after a certification allowing form filling (made up), the kid's drawing
/// redrawn in place as it is signed: allowed. It was refused, the drawing taken for a change
/// to an unsigned field.
#[test]
fn signing_redraws_a_widget_kids_own_drawing() {
    let field = extra_field(&base_pdf(), true, |new| {
        dictionary! { "AP" => dictionary! { "N" => appearance(new, "") } }
    });
    let want = [(ChangeKind::Signed("Extra".into()), None)];
    let mut wrong = Vec::new();
    for (signed, after) in [(approve(&field), "an approval"), (certify(&field, 2), "P2")] {
        let b = edit(&signed, |prev, new| {
            let top = titled(prev, "Extra");
            let mut v = dict(prev, top);
            v.set("V", new.add_object(dictionary! { "Type" => "Sig" }));
            new.set_object(top, Object::Dictionary(v));
            let kid = dict(prev, first_kid(prev, top));
            let ap = kid.get(b"AP").unwrap().as_dict().unwrap();
            let look = ap.get(b"N").unwrap().as_reference().unwrap();
            let mut s = prev.get_object(look).unwrap().as_stream().unwrap().clone();
            s.set_content(b"BT (Signed) Tj ET".to_vec());
            new.set_object(look, Object::Stream(s));
        });
        let got = changes(&b);
        if got != want {
            wrong.push(format!("after {after}: {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The base document with text field `A`, its widget `F` (titled if `named`) on the page, and
/// after it empty signature field `B`, whose kids are in an array of their own: `F` again,
/// `depth` untitled fields down, if `shared`; none if not.
fn two_parents(depth: usize, named: bool, shared: bool) -> Vec<u8> {
    edit(&base_pdf(), |prev, new| {
        let a = new.new_object_id();
        let mut w = dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "F" => 4, "P" => page(prev), "Parent" => a,
            "Rect" => vec![150.into(), 100.into(), 250.into(), 140.into()],
        };
        if named {
            w.set("T", Object::string_literal("F"));
        }
        let f = new.add_object(w);
        add_annot(prev, new, f);
        let (t, kids) = (Object::string_literal("A"), vec![Object::from(f)]);
        let field = dictionary! { "FT" => "Tx", "T" => t, "Kids" => kids };
        new.set_object(a, Object::Dictionary(field));
        let mut top = f;
        for _ in 0..depth {
            top = new.add_object(dictionary! { "Kids" => vec![Object::from(top)] });
        }
        let kids: Vec<Object> = if shared { vec![top.into()] } else { Vec::new() };
        let kids = new.add_object(kids);
        let b = new.add_object(dictionary! {
            "FT" => "Sig", "T" => Object::string_literal("B"), "Kids" => kids,
        });
        let r = root(prev);
        let mut c = dict(prev, r);
        c.set(
            "AcroForm",
            dictionary! { "Fields" => vec![Object::from(a), Object::from(b)] },
        );
        new.set_object(r, Object::Dictionary(c));
    })
}

/// `F` of [`two_parents`] made `B`'s by edits to lists of kids and fields alone (`A` listing
/// it no more, or `B` listed first, `B` listing it too if it didn't), and given a made-up
/// signature in the same revision: refused as a change to `A`'s field. Before, listed by `B`
/// only in the revision, `F`'s signing was never judged and nothing was reported; 33 levels
/// down, past the 32 the form was followed to, `F` stayed `A`'s: refused as removed when `A`
/// dropped it, or taken for `A.F` filled in when `B` came first, allowed after an approval.
#[test]
fn a_widget_moved_under_a_signature_field_and_signed_is_refused() {
    let mut wrong = Vec::new();
    for (depth, named, shared) in [(33, true, true), (0, false, false)] {
        let form = two_parents(depth, named, shared);
        for (stamp, signed) in [(true, certify(&form, 1)), (false, approve(&form))] {
            for drop in [true, false] {
                let b = edit(&signed, |prev, new| {
                    let (a, b) = (titled(prev, "A"), titled(prev, "B"));
                    let f = first_kid(prev, a);
                    if !shared {
                        let kids = dict(prev, b).get(b"Kids").unwrap().as_reference().unwrap();
                        new.set_object(kids, vec![Object::from(f)]);
                    }
                    if drop {
                        let mut d = dict(prev, a);
                        d.set("Kids", Vec::<Object>::new());
                        new.set_object(a, Object::Dictionary(d));
                    } else {
                        list_first(prev, new, b);
                    }
                    fake_sign(prev, new, f, stamp);
                });
                let got = changes(&b);
                let moved = ChangeKind::FieldChanged((if named { "A.F" } else { "A" }).into());
                if !got.contains(&(moved, Some(Refusal::Never))) || report(&b).all_ok() {
                    let case = format!("depth {depth}, timestamp {stamp}, A drops it {drop}");
                    wrong.push(format!("{case}: {got:?}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The base document with empty signature field `B`, its widget `W` on the page, and text
/// field `A`, whose kids are in an array of their own: `W` again if `shared`, `B` listed first
/// so `W` is `B`'s; none if not, `A` listed first.
fn widget_of_a_signature_field(shared: bool) -> Vec<u8> {
    edit(&base_pdf(), |prev, new| {
        let b = new.new_object_id();
        let w = new.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "F" => 4, "P" => page(prev), "Parent" => b,
            "Rect" => vec![150.into(), 100.into(), 250.into(), 140.into()],
        });
        add_annot(prev, new, w);
        let (t, kids) = (Object::string_literal("B"), vec![Object::from(w)]);
        let field = dictionary! { "FT" => "Sig", "T" => t, "Kids" => kids };
        new.set_object(b, Object::Dictionary(field));
        let kids: Vec<Object> = if shared { vec![w.into()] } else { Vec::new() };
        let kids = new.add_object(kids);
        let a = new.add_object(dictionary! {
            "FT" => "Tx", "T" => Object::string_literal("A"), "Kids" => kids,
        });
        let fields = match shared {
            true => vec![Object::from(b), a.into()],
            false => vec![Object::from(a), b.into()],
        };
        let r = root(prev);
        let mut c = dict(prev, r);
        c.set("AcroForm", dictionary! { "Fields" => fields });
        new.set_object(r, Object::Dictionary(c));
    })
}

/// `W` of [`widget_of_a_signature_field`] made `A`'s by edits to lists of kids and fields alone
/// (`A` listed first, or `A` listing it too), and filled in as `A`'s in the same revision:
/// refused as a change to `B`'s field, after an approval and after a certification allowing
/// form filling, either of which allows filling in `A`.
#[test]
fn a_widget_moved_out_of_a_signature_field_is_refused() {
    let moved = (ChangeKind::FieldChanged("B".into()), Some(Refusal::Never));
    let mut wrong = Vec::new();
    for shared in [true, false] {
        let form = widget_of_a_signature_field(shared);
        for (after, signed) in [("an approval", approve(&form)), ("P2", certify(&form, 2))] {
            let e = edit(&signed, |prev, new| {
                let (a, b) = (titled(prev, "A"), titled(prev, "B"));
                let w = first_kid(prev, b);
                if shared {
                    list_first(prev, new, a);
                } else {
                    let kids = dict(prev, a).get(b"Kids").unwrap().as_reference().unwrap();
                    new.set_object(kids, vec![Object::from(w)]);
                }
                let mut d = dict(prev, a);
                d.set("V", Object::string_literal("PAID"));
                new.set_object(a, Object::Dictionary(d));
                let ap = appearance(new, "BT (PAID) Tj ET");
                let mut d = dict(prev, w);
                d.set("AP", dictionary! { "N" => ap });
                new.set_object(w, Object::Dictionary(d));
            });
            let got = changes(&e);
            if !got.contains(&moved) || report(&e).all_ok() {
                wrong.push(format!("shared {shared}, after {after}: {got:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The base document with text field `T`, a widget of its own on the page, and empty signature
/// field `B`, whose kids are in an array of their own: `T` if `inside`, `B` alone listed in the
/// form; none if not, `T` listed first.
fn typed_and_signature_field(inside: bool) -> Vec<u8> {
    edit(&base_pdf(), |prev, new| {
        let b = new.new_object_id();
        let mut t = dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "F" => 4, "P" => page(prev),
            "Rect" => vec![150.into(), 100.into(), 250.into(), 140.into()],
            "FT" => "Tx", "T" => Object::string_literal("T"),
        };
        if inside {
            t.set("Parent", b);
        }
        let t = new.add_object(t);
        add_annot(prev, new, t);
        let kids: Vec<Object> = if inside { vec![t.into()] } else { Vec::new() };
        let kids = new.add_object(kids);
        let (n, kids) = (Object::string_literal("B"), Object::from(kids));
        let field = dictionary! { "FT" => "Sig", "T" => n, "Kids" => kids };
        new.set_object(b, Object::Dictionary(field));
        let fields = match inside {
            true => vec![Object::from(b)],
            false => vec![Object::from(t), b.into()],
        };
        let r = root(prev);
        let mut c = dict(prev, r);
        c.set("AcroForm", dictionary! { "Fields" => fields });
        new.set_object(r, Object::Dictionary(c));
    })
}

/// `T` of [`typed_and_signature_field`] moved into `B` or out of it by edits to lists of kids
/// and fields alone (listed in its new place only, or in both, the new one first):
/// refused as a change to `T`'s field, after an approval and after a certification allowing no
/// changes. Its own type keeps it a text field, so only its parent and name changing show the
/// move; before they were compared, it was reported as nothing.
#[test]
fn a_typed_field_moved_into_or_out_of_a_signature_field_is_refused() {
    let mut wrong = Vec::new();
    for inside in [false, true] {
        let form = typed_and_signature_field(inside);
        for (after, signed) in [("an approval", approve(&form)), ("P1", certify(&form, 1))] {
            for both in [false, true] {
                let e = edit(&signed, |prev, new| {
                    let (t, b) = (titled(prev, "T"), titled(prev, "B"));
                    let kids = dict(prev, b).get(b"Kids").unwrap().as_reference().unwrap();
                    match (inside, both) {
                        (false, false) => {
                            new.set_object(kids, vec![Object::from(t)]);
                            unlist(prev, new, t);
                        }
                        (false, true) => {
                            new.set_object(kids, vec![Object::from(t)]);
                            list_first(prev, new, b);
                        }
                        (true, false) => {
                            new.set_object(kids, Vec::<Object>::new());
                            list_field(prev, new, t);
                        }
                        (true, true) => list_first(prev, new, t),
                    }
                });
                let name = if inside { "B.T" } else { "T" };
                let moved = (ChangeKind::FieldChanged(name.into()), Some(Refusal::Never));
                let got = changes(&e);
                if !got.contains(&moved) || report(&e).all_ok() {
                    let case = format!("inside {inside}, after {after}, both {both}");
                    wrong.push(format!("{case}: {got:?}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The base document with node `X` of no kind and empty signature field `T`, both listing
/// widget `K` (titled, of no kind of its own, with no `/Parent`): `X` listed first in the form
/// if `x_first`. `K` is `T`'s, `T.K`, either way.
fn kid_of_no_kind_and_of_a_signature_field(x_first: bool) -> Vec<u8> {
    edit(&base_pdf(), |prev, new| {
        let k = new.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "F" => 4, "P" => page(prev),
            "Rect" => vec![150.into(), 100.into(), 250.into(), 140.into()],
            "T" => Object::string_literal("K"),
        });
        add_annot(prev, new, k);
        let kids = vec![Object::from(k)];
        let x = dictionary! { "T" => Object::string_literal("X"), "Kids" => kids.clone() };
        let x = new.add_object(x);
        let t = dictionary! { "FT" => "Sig", "T" => Object::string_literal("T"), "Kids" => kids };
        let t = new.add_object(t);
        let fields = match x_first {
            true => vec![Object::from(x), t.into()],
            false => vec![Object::from(t), x.into()],
        };
        let r = root(prev);
        let mut c = dict(prev, r);
        c.set("AcroForm", dictionary! { "Fields" => fields });
        new.set_object(r, Object::Dictionary(c));
    })
}

/// `K` of [`kid_of_no_kind_and_of_a_signature_field`] signed, then approved, then its
/// signature taken out: refused as a change to the signature in `T.K`, whichever of `X` and
/// `T` is listed first. Before, with `X` first, the check named and typed `K` through `X`, a
/// field of no kind, while the report showed the signature in `T.K`: its removal passed as
/// `X.K` filled in, allowed.
#[test]
fn a_signature_in_a_field_a_node_of_no_kind_lists_first_stays_one() {
    let mut wrong = Vec::new();
    for x_first in [true, false] {
        let form = kid_of_no_kind_and_of_a_signature_field(x_first);
        let k = Field {
            name: "K",
            existing: true,
            ..Field::default()
        };
        let signed = approve(&sign(&form, &id("p256"), &k, &Cms::default()));
        let shown: Vec<_> = report(&signed)
            .signatures
            .into_iter()
            .map(|s| s.fields)
            .collect();
        assert_eq!(
            shown,
            [vec!["T.K"], vec!["Signature1"]],
            "X first {x_first}"
        );
        let e = edit(&signed, |prev, new| {
            let k = titled(prev, "K");
            let mut d = dict(prev, k);
            d.remove(b"V");
            new.set_object(k, Object::Dictionary(d));
        });
        let got = changes(&e);
        let removed = (
            ChangeKind::SignatureChanged("T.K".into()),
            Some(Refusal::Never),
        );
        if !got.contains(&removed) || report(&e).all_ok() {
            wrong.push(format!("X first {x_first}: {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// `K` of [`kid_of_no_kind_and_of_a_signature_field`] locked by the first signature, by the
/// name the report shows, `T.K`, then signed: refused as locked, whichever of `X` and `T` is
/// listed first. Before, with `X` first, the check named it `X.K`, which no lock named, and
/// allowed its signing as `X.K` filled in.
#[test]
fn a_lock_reaches_a_field_a_node_of_no_kind_lists_first() {
    let mut wrong = Vec::new();
    for x_first in [true, false] {
        let form = kid_of_no_kind_and_of_a_signature_field(x_first);
        let locked = locked_by(&form, lock("Include", &["T.K"], None));
        let k = Field {
            name: "K",
            existing: true,
            ..Field::default()
        };
        let b = sign(&locked, &id("p256"), &k, &Cms::default());
        let got = changes(&b);
        let signed = (ChangeKind::Signed("T.K".into()), Some(Refusal::Locked));
        if !got.contains(&signed) || report(&b).all_ok() {
            wrong.push(format!("X first {x_first}: {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The base document with signature field `Signature1`, its widget `W` on the page, and empty
/// signature field `S2`: the kids of each in an array of their own if `arrays` (`S2`'s none),
/// else `Signature1`'s listed in it and `S2` with none.
fn two_signature_fields(arrays: bool) -> Vec<u8> {
    edit(&base_pdf(), |prev, new| {
        let s1 = new.new_object_id();
        let w = new.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "F" => 4, "P" => page(prev), "Parent" => s1,
            "Rect" => vec![150.into(), 100.into(), 250.into(), 140.into()],
        });
        add_annot(prev, new, w);
        let mut first = dictionary! { "FT" => "Sig", "T" => Object::string_literal("Signature1") };
        let mut second = dictionary! { "FT" => "Sig", "T" => Object::string_literal("S2") };
        let kids = vec![Object::from(w)];
        if arrays {
            first.set("Kids", new.add_object(kids));
            second.set("Kids", new.add_object(Vec::<Object>::new()));
        } else {
            first.set("Kids", kids);
        }
        new.set_object(s1, Object::Dictionary(first));
        let s2 = new.add_object(second);
        let r = root(prev);
        let mut c = dict(prev, r);
        let fields = vec![Object::from(s1), s2.into()];
        c.set("AcroForm", dictionary! { "Fields" => fields });
        new.set_object(r, Object::Dictionary(c));
    })
}

/// `W` of [`two_signature_fields`] made `S2`'s once `Signature1` is signed, by edits to the
/// arrays of kids alone, or by `S2` listed first and then signed with `W` as its kid: refused
/// as a change to `Signature1`'s field, as the signature would show as `S2`'s. `W` holds no
/// value, so only its parent and name changing show the move; before they were compared, it
/// was allowed.
#[test]
fn a_signed_fields_widget_moved_to_another_signature_field_is_refused() {
    let moved = ChangeKind::FieldChanged("Signature1".into());
    let mut wrong = Vec::new();
    for arrays in [true, false] {
        let f = Field {
            existing: true,
            ..Field::default()
        };
        let form = two_signature_fields(arrays);
        let signed = sign(&form, &id("rsa3072"), &f, &Cms::default());
        let e = if arrays {
            edit(&signed, |prev, new| {
                let kids = |field| {
                    let d = dict(prev, titled(prev, field));
                    d.get(b"Kids").unwrap().as_reference().unwrap()
                };
                let (from, to) = (kids("Signature1"), kids("S2"));
                let w = prev.get_object(from).unwrap().as_array().unwrap()[0].clone();
                new.set_object(from, Vec::<Object>::new());
                new.set_object(to, vec![w]);
            })
        } else {
            let first = edit(&signed, |prev, new| {
                list_first(prev, new, titled(prev, "S2"))
            });
            let d = Document::load_mem(&first).unwrap();
            let w = first_kid(&d, titled(&d, "Signature1"));
            let f = Field {
                name: "S2",
                existing: true,
                widget: dictionary! { "Kids" => vec![Object::from(w)] },
                ..Field::default()
            };
            sign(&first, &id("p256"), &f, &Cms::default())
        };
        let got = changes(&e);
        if !got.contains(&(moved.clone(), Some(Refusal::Never))) || report(&e).all_ok() {
            wrong.push(format!("arrays {arrays}: {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Signed, timestamped, then an untitled signature field added and listed first with the
/// timestamp's field among its kids (and a valued text field too, `hidden`): the field added is
/// judged as one, refused after a certification or a lock allowing no changes, and the
/// timestamp's field moved, never allowed. Before, the timestamp found in the new field was
/// taken for its signing, and the field added went unreported.
#[test]
fn a_new_signature_field_holding_an_earlier_signature_is_no_signing() {
    let form = form(&["Name"]);
    let locked = locked_by(&form, lock("All", &[], Some(1)));
    let signed = [
        (certify(&form, 1), "P1", true),
        (locked, "a P1 lock", true),
        (approve(&form), "an approval", false),
    ];
    let added = ChangeKind::SignatureFieldAdded("".into());
    let moved = ChangeKind::FieldChanged("Timestamp1".into());
    let mut wrong = Vec::new();
    for (signed, after, refused) in signed {
        let stamped = timestamp(&signed, true);
        for hidden in [false, true] {
            let b = edit(&stamped, |prev, new| {
                let w = new.new_object_id();
                let mut kids = vec![Object::from(titled(prev, "Timestamp1"))];
                if hidden {
                    let t = new.add_object(dictionary! {
                        "FT" => "Tx", "T" => Object::string_literal("Hidden"),
                        "V" => Object::string_literal("1,000,000"), "Parent" => w,
                    });
                    kids.push(t.into());
                }
                let field = dictionary! { "FT" => "Sig", "Kids" => kids };
                new.set_object(w, Object::Dictionary(field));
                list_first(prev, new, w);
            });
            let got = changes(&b);
            let judged = (got.iter()).any(|(k, r)| *k == added && r.is_some() == refused);
            let never = got.contains(&(moved.clone(), Some(Refusal::Never)));
            if !judged || !never || report(&b).all_ok() {
                wrong.push(format!("hidden {hidden} after {after}: {got:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_change_undone_before_the_next_signature_still_counts() {
    let b = approve(&base_pdf());
    let rotate = |b: &[u8], r: Option<i64>| {
        edit(b, |prev, new| {
            let p = page(prev);
            let mut d = dict(prev, p);
            match r {
                Some(r) => d.set("Rotate", r),
                None => _ = d.remove(b"Rotate"),
            }
            new.set_object(p, Object::Dictionary(d));
        })
    };
    // Turned, countersigned, turned back: the page ends as the first signer signed it, but
    // the second signer signed something else. Before, as the second signature ends just
    // before the end of line its revision's end takes in, the first was checked against
    // the end alone, and only the second signing was seen. So too when the signed version is
    // followed by "\r\n", the most end of line a revision's end takes in.
    let turned = countersign(&rotate(&b, Some(90)), "Signature2", false);
    assert!(turned.ends_with(b"%%EOF"));
    for eol in ["", "\r\n"] {
        let turned = [&turned, eol.as_bytes()].concat();
        let b = rotate(&turned, None);
        let d = Document::load_mem(&b).unwrap();
        assert!(!dict(&d, page(&d)).has(b"Rotate"));
        assert_refused(&b, &ChangeKind::PageContent);
        assert!(!sig(&b, "Signature2").changes_allowed());
    }
}

// ---------------------------------------------------------------- attacks

/// Shadow attack, hide: an update that only frees an object still in use. Some apps find it
/// by scanning, so it is refused, by LimarePDF's own reader.
#[test]
fn freeing_an_object_still_in_use_is_refused() {
    let b = approve(&base_pdf());
    let c = contents(&b);
    let freed = append_raw(&b, &[], &[c.0], &[], "");
    let got = changes(&freed);
    assert!(
        matches!(&got[..], [(ChangeKind::Unrecognised(w), Some(Refusal::Never))] if w.contains("deleted but is still used")),
        "{got:#?}"
    );
}

/// Shadow attack, hide-and-replace: an update re-pointing an object at bytes inside the
/// signed part (an unused object planted before signing).
#[test]
fn pointing_back_into_the_signed_part_is_refused() {
    let b = approve(&base_pdf());
    let c = contents(&b);
    let moved = append_raw(&b, &[], &[], &[(c.0, 9)], "");
    assert!(matches!(
        &changes(&moved)[..],
        [(ChangeKind::Malformed(w), _)] if w.contains("outside the revision")
    ));
    // An entry pointing at another object's header (DSS hide.pdf). The kit starts the
    // revision on a new line.
    let at = b.len() + usize::from(!b.ends_with(b"\n"));
    let num = Document::load_mem(&b).unwrap().max_id + 1;
    let wrong = append_raw(&b, &[(num, b"<< >>")], &[], &[(c.0, at)], "");
    assert!(matches!(
        &changes(&wrong)[..],
        [(ChangeKind::Malformed(w), _)] if w.contains("isn't where")
    ));
}

/// With several objects that don't read as their entries say, the reason names the same one
/// (the lowest number) every time the file is checked, so saving it doesn't take a reason
/// that merely names another object for a new change (DSS modified_after_signature.pdf).
#[test]
fn a_malformed_revision_gives_the_same_reason_each_time() {
    let b = approve(&base_pdf());
    let d = Document::load_mem(&b).unwrap();
    let c = contents(&b);
    let p = page(&d);
    let at = b.len() + usize::from(!b.ends_with(b"\n"));
    let num = d.max_id + 1;
    let wrong = append_raw(&b, &[(num, b"<< >>")], &[], &[(c.0, at), (p.0, at)], "");
    let low = c.0.min(p.0);
    for _ in 0..20 {
        let got = changes(&wrong);
        assert!(
            matches!(&got[..], [(ChangeKind::Malformed(w), _)]
                if *w == format!("object {low} isn't where its cross-reference entry says")),
            "{got:#?}"
        );
    }
}

#[test]
fn data_after_the_last_revision_is_refused() {
    let mut b = fill(&approve(&form(&["Name"])), "Name", "Jane");
    b.extend_from_slice(b"9 0 obj << /Junk true >> endobj\n");
    assert!(matches!(
        &changes(&b)[..],
        [(ChangeKind::Malformed(w), _)] if w.contains("complete revision")
    ));
}

/// Shadow attack shape: an object the page draws is also listed as an annotation, so
/// changing it might pass as "comment changed".
#[test]
fn page_content_posing_as_a_comment_is_still_page_content() {
    let base = edit(&base_pdf(), |prev, new| {
        let p = page(prev);
        let c = dict(prev, p)
            .get(b"Contents")
            .unwrap()
            .as_reference()
            .unwrap();
        let mut d = dict(prev, p);
        d.set("Annots", vec![Object::from(c)]);
        new.set_object(p, Object::Dictionary(d));
    });
    let b = approve(&base);
    let c = contents(&b);
    let changed = edit(&b, |_, new| {
        new.set_object(c, Stream::new(Dictionary::new(), b"0 0 m 9 9 l S".to_vec()));
    });
    assert_refused(&changed, &ChangeKind::PageContent);
}

/// An object the signed version referred to but didn't have, filled in later.
#[test]
fn filling_in_a_missing_object_is_refused() {
    let base = edit(&base_pdf(), |prev, new| {
        let p = page(prev);
        let mut d = dict(prev, p);
        d.set("Thumb", Object::Reference((900, 0)));
        new.set_object(p, Object::Dictionary(d));
    });
    let b = approve(&base);
    let b = append_raw(&b, &[(900, b"<< /Late true >>")], &[], &[], "");
    let got = changes(&b);
    assert!(
        got.iter()
            .any(|(k, r)| matches!(k, ChangeKind::Unrecognised(_)) && r.is_some()),
        "{got:#?}"
    );
}

/// An object only the data of what is drawn refers to, which Poppler follows where its drawing
/// looks in a dictionary (marked content's, an inline image's settings), is part of that
/// drawing: changed later, it changes the page's content, or the comment's appearance. One
/// nothing refers to changes nothing (review round 4).
#[test]
fn an_object_drawn_data_refers_to_is_part_of_the_drawing() {
    let mut ids = [(0, 0); 3];
    let base = edit(&base_pdf(), |prev, new| {
        let text = |s: &str| Object::string_literal(s);
        ids = ["Pay one hundred", "Approved", "Unused"].map(|s| new.add_object(text(s)));
        let marked = |id: ObjectId| format!("/Span <</ActualText {} 0 R>> BDC EMC", id.0);
        let p = dict(prev, page(prev));
        let c = p.get(b"Contents").unwrap().as_reference().unwrap();
        let content = Stream::new(Dictionary::new(), marked(ids[0]).into_bytes());
        new.set_object(c, content);
        let ap = appearance(new, &marked(ids[1]));
        let note = new.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Square", "P" => page(prev),
            "Rect" => vec![0.into(), 0.into(), 9.into(), 9.into()],
            "AP" => dictionary! { "N" => ap },
        });
        add_annot(prev, new, note);
    });
    let b = approve(&base);
    let [text, said, lone] = ids;
    let changed = |id: ObjectId| {
        let more = Object::string_literal("Pay one million");
        edit(&b, |_, new| new.set_object(id, more))
    };
    assert_refused(&changed(text), &ChangeKind::PageContent);
    let comment = [(ChangeKind::CommentChanged, None)];
    assert_eq!(changes(&changed(said)), comment);
    assert_eq!(changes(&changed(lone)), []);
}

/// Truncated or damaged appended data never panics and never passes as unchanged.
#[test]
fn damaged_appended_data_is_never_clean() {
    let b = fill(&approve(&form(&["Name"])), "Name", "Jane");
    let end = sig(&b, "Signature1").signed_len.unwrap();
    let clean = |x: &[u8]| -> Option<bool> {
        let r = verify_signatures(x, None, &TrustList::default()).ok()?;
        let s = r.signatures.iter().find(|s| s.fields == ["Signature1"])?;
        Some(s.status == SigStatus::Valid && s.changes.is_empty())
    };
    for cut in (end + 1..b.len()).step_by(3) {
        let x = &b[..cut];
        if clean(x) == Some(true) {
            assert!(x[end..].iter().all(u8::is_ascii_whitespace), "cut at {cut}");
        }
    }
    for at in (end..b.len()).step_by(5) {
        for flip in [0x01u8, 0x20, 0x80] {
            let mut x = b.clone();
            x[at] ^= flip;
            assert_ne!(clean(&x), Some(true), "byte {at} ^ {flip:#x}");
        }
    }
}

// ---------------------------------------------------------------- signed versions

#[test]
fn the_signed_version_is_the_file_as_signed() {
    let signed = approve(&form(&["Name"]));
    let b = comment(&fill(&signed, "Name", "Jane"), Dictionary::new());
    let s = sig(&b, "Signature1");
    let len = s.signed_len.unwrap();
    assert_eq!(&b[..len], &signed[..]);
    let then = sig(&b[..len], "Signature1");
    assert_eq!(then.status, SigStatus::Valid);
    assert!(then.covers_whole_file && then.changes.is_empty());
}
