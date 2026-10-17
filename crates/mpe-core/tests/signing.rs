//! Signing with a digital ID: approval signatures and certification, in new boxes and in
//! empty fields, locks, the document's own rules, edits that ride along, unusual files,
//! and the check every signed file gets before it is handed out. The IDs are the
//! OpenSSL-made test files in `tests/data/signatures/ids` (password "mpe-test-password").

mod sigkit;

use lopdf::{dictionary, Dictionary, Object, ObjectId};
use mpe_core::{
    sign_document, verify_signatures, ChangeKind, Cipher, Encryption, FieldValue, IdStore,
    InkColor, LockAction, Pdf, Permissions, Restrictions, SigKind, SigStatus, SignArt, SignPlace,
    SignRequest, SignRole, SignStyle, SignatureReport, SignedDocument, Stamp, StampPlace, Trust,
    TrustList, TrustRole, UnlockedId,
};
use sigkit::*;
use std::sync::OnceLock;

const PASSWORD: &str = "mpe-test-password";

/// The test ID file `name`, imported into a scratch folder and opened.
fn unlocked(name: &str) -> UnlockedId {
    let dir = std::env::temp_dir().join(format!("mpe-signing-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = IdStore::new(dir.join("ids"));
    let file = std::fs::read(data_dir().join("ids").join(format!("{name}.p12"))).unwrap();
    let id = store.import(&file, PASSWORD, None).unwrap();
    let u = store.unlock(&id, PASSWORD).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    u
}

macro_rules! test_id {
    ($f:ident, $file:literal) => {
        fn $f() -> &'static UnlockedId {
            static ID: OnceLock<UnlockedId> = OnceLock::new();
            ID.get_or_init(|| unlocked($file))
        }
    };
}
test_id!(rsa, "modern");
test_id!(p256, "p256");
test_id!(p384, "p384");
test_id!(issued, "chain");

fn req(id: &UnlockedId, place: SignPlace) -> SignRequest<'_> {
    SignRequest {
        id,
        place,
        reason: None,
        location: None,
        certify: None,
        art: None,
        accept_disallowed: false,
    }
}

/// A new box on the base page (300 by 200 points); `n` picks one of three places.
fn new_box(n: usize) -> SignPlace {
    let y = 10.0 + 60.0 * n as f32;
    SignPlace::NewBox {
        page: 0,
        rect: [20.0, y, 220.0, y + 50.0],
    }
}

fn field(name: &str) -> SignPlace {
    SignPlace::Field(name.into())
}

fn sign(bytes: &[u8], r: &SignRequest) -> SignedDocument {
    sign_document(bytes, None, r).unwrap()
}

fn report(b: &[u8]) -> SignatureReport {
    verify_signatures(b, None, &TrustList::default()).unwrap()
}

#[track_caller]
fn refused<T: std::fmt::Debug>(r: mpe_core::Result<T>, part: &str) {
    let e = format!("{:#}", r.expect_err("must be refused"));
    assert!(e.contains(part), "{e:?} should say {part:?}");
}

/// The newest signature checks as a perfect one by `id`.
#[track_caller]
fn newest_is_good(bytes: &[u8], id: &UnlockedId) -> SignatureReport {
    let rep = report(bytes);
    let s = rep.signatures.last().unwrap();
    assert_eq!(s.status, SigStatus::Valid, "{:?}", s.status);
    assert!(s.covers_whole_file);
    assert!(s.changes.is_empty(), "{:?}", s.changes);
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    assert_eq!(s.signer.as_ref().unwrap().der, id.cert.der);
    assert_eq!(s.sub_filter, "adbe.pkcs7.detached");
    rep
}

/// The base page with a text field "Name" and an empty signature field for each of
/// `sigs` (with its `/Lock`, if any), and the form's `/NeedAppearances` set as given.
fn form_doc(sigs: &[(&str, Option<Dictionary>)], need_appearances: bool) -> Vec<u8> {
    let mut p = Pdf::from_bytes(&base_pdf(), None).unwrap();
    let doc = &mut p.doc;
    let page = doc.page_iter().next().unwrap();
    let rect =
        |y: i64, h: i64| -> Vec<Object> { vec![20.into(), y.into(), 200.into(), (y + h).into()] };
    let mut fields: Vec<ObjectId> = vec![doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
        "T" => Object::string_literal("Name"), "V" => Object::string_literal(""),
        "DA" => Object::string_literal("/Helv 10 Tf 0 g"),
        "Rect" => rect(150, 20), "P" => page, "F" => 4,
    })];
    for (i, (name, lock)) in sigs.iter().enumerate() {
        let mut d = dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Sig",
            "T" => Object::string_literal(*name),
            "Rect" => rect(20 + 60 * i as i64, 50), "P" => page, "F" => 4,
        };
        if let Some(l) = lock {
            d.set("Lock", l.clone());
        }
        fields.push(doc.add_object(d));
    }
    let refs: Vec<Object> = fields.iter().map(|&f| f.into()).collect();
    doc.get_dictionary_mut(page)
        .unwrap()
        .set("Annots", refs.clone());
    let helv = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let mut form = dictionary! {
        "Fields" => refs,
        "DA" => Object::string_literal("/Helv 0 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
    };
    if need_appearances {
        form.set("NeedAppearances", true);
    }
    doc.catalog_mut().unwrap().set("AcroForm", form);
    p.snapshot().unwrap()
}

fn lock(action: &str, fields: &[&str]) -> Dictionary {
    let mut d = dictionary! { "Type" => "SigFieldLock", "Action" => action };
    if !fields.is_empty() {
        let f: Vec<Object> = fields.iter().map(|f| Object::string_literal(*f)).collect();
        d.set("Fields", f);
    }
    d
}

/// The dictionary of the widget whose `/T` is `name`.
fn widget(bytes: &[u8], name: &str) -> Dictionary {
    let p = Pdf::from_bytes(bytes, None).unwrap();
    p.doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .find(|d| {
            d.get(b"T")
                .and_then(Object::as_str)
                .is_ok_and(|t| t == name.as_bytes())
        })
        .cloned()
        .unwrap_or_else(|| panic!("no widget {name}"))
}

/// The content of the normal appearance of the widget named `name`.
fn appearance(bytes: &[u8], name: &str) -> (Dictionary, String) {
    let p = Pdf::from_bytes(bytes, None).unwrap();
    let w = widget(bytes, name);
    let ap = w.get(b"AP").unwrap().as_dict().unwrap();
    let id = ap.get(b"N").unwrap().as_reference().unwrap();
    let s = p.doc.get_object(id).unwrap().as_stream().unwrap();
    (
        s.dict.clone(),
        String::from_utf8_lossy(&s.content).into_owned(),
    )
}

fn kinds(rep: &SignatureReport, i: usize) -> Vec<ChangeKind> {
    rep.signatures[i]
        .changes
        .iter()
        .map(|c| c.kind.clone())
        .collect()
}

// ------------------------------------------------------------------ signing

#[test]
fn signs_with_rsa_and_ec_ids() {
    let base = base_pdf();
    for (id, digest) in [(rsa(), "SHA-256"), (p256(), "SHA-256"), (p384(), "SHA-384")] {
        let mut r = req(id, new_box(0));
        r.reason = Some("I approve".into());
        r.location = Some("Montréal".into());
        let s = sign(&base, &r);
        assert!(s.bytes.starts_with(&base), "earlier bytes kept");
        assert_eq!(s.field, "Signature1");
        assert_eq!((s.finished_items, s.rewritten), (0, false));
        let rep = newest_is_good(&s.bytes, id);
        let sig = &rep.signatures[0];
        assert_eq!(rep.signatures.len(), 1);
        assert_eq!(sig.kind, SigKind::Approval);
        assert_eq!(sig.fields, ["Signature1"]);
        assert_eq!(sig.page, Some(0));
        assert_eq!(sig.digest, Some(digest));
        assert_eq!(sig.reason.as_deref(), Some("I approve"));
        assert_eq!(sig.location.as_deref(), Some("Montréal"));
        assert_eq!(sig.name.as_deref(), Some(id.cert.display_name()));
        // The box is where it was asked for (displayed-page points, y down).
        let [x0, y0, x1, y1] = sig.bbox.unwrap();
        assert!(
            (x0 - 20.0).abs() < 0.01 && (y0 - 10.0).abs() < 0.01,
            "{x0} {y0}"
        );
        assert!(
            (x1 - 220.0).abs() < 0.01 && (y1 - 60.0).abs() < 0.01,
            "{x1} {y1}"
        );
    }
}

#[test]
fn the_look_and_the_dates_follow_acrobat() {
    let s = sign(&base_pdf(), &req(rsa(), new_box(0)));
    let w = widget(&s.bytes, "Signature1");
    // Printed and locked, as Acrobat writes signed widgets.
    assert_eq!(w.get(b"F").unwrap().as_i64().unwrap(), 4 | 128);
    let (dict, text) = appearance(&s.bytes, "Signature1");
    assert!(
        text.contains("(Digitally) Tj") || text.contains("(Digitally signed"),
        "{text}"
    );
    assert!(text.contains("(Date: "), "{text}");
    assert!(text.contains("/Sig Do"), "the name in handwriting: {text}");
    let bbox = dict.get(b"BBox").unwrap().as_array().unwrap();
    assert_eq!(bbox.len(), 4);
    // `/M` in Adobe's form, D:YYYYMMDDHHmmSS+HH'mm'.
    let p = Pdf::from_bytes(&s.bytes, None).unwrap();
    let sig_id = w.get(b"V").unwrap().as_reference().unwrap();
    let sig = p.doc.get_dictionary(sig_id).unwrap();
    let m = String::from_utf8(sig.get(b"M").unwrap().as_str().unwrap().to_vec()).unwrap();
    let ok = m.len() == 23
        && m.starts_with("D:")
        && m[2..16].bytes().all(|b| b.is_ascii_digit())
        && matches!(&m[16..17], "+" | "-")
        && &m[19..20] == "'"
        && m.ends_with('\'');
    assert!(ok, "{m}");
    assert_eq!(
        sig.get(b"Filter").unwrap().as_name().unwrap(),
        b"Adobe.PPKLite"
    );
}

#[test]
fn a_picture_signature_is_drawn_beside_the_text() {
    let png = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/images/icon.png"),
    )
    .unwrap();
    let mut r = req(rsa(), new_box(0));
    r.art = Some(SignArt::Picture {
        bytes: &png,
        clear_paper: false,
    });
    let s = sign(&base_pdf(), &r);
    newest_is_good(&s.bytes, rsa());
    let (_, text) = appearance(&s.bytes, "Signature1");
    assert!(text.contains("/Sig Do"), "{text}");
}

#[test]
fn a_second_signature_keeps_the_first_valid() {
    let s1 = sign(&base_pdf(), &req(rsa(), new_box(0)));
    let s2 = sign(&s1.bytes, &req(p256(), new_box(1)));
    assert_eq!(s2.field, "Signature2");
    assert!(s2.bytes.starts_with(&s1.bytes));
    let rep = newest_is_good(&s2.bytes, p256());
    assert_eq!(rep.signatures.len(), 2);
    let first = &rep.signatures[0];
    assert_eq!(first.status, SigStatus::Valid);
    assert!(!first.covers_whole_file);
    assert!(first.changes_allowed(), "{:?}", first.changes);
    assert!(kinds(&rep, 0).contains(&ChangeKind::Signed("Signature2".into())));
    // And a third, on a document with two.
    let s3 = sign(&s2.bytes, &req(p384(), new_box(2)));
    let rep = newest_is_good(&s3.bytes, p384());
    assert!(rep.signatures.iter().all(|s| s.changes_allowed()));
}

#[test]
fn signs_an_empty_signature_field() {
    let doc = form_doc(&[("Approver", None)], false);
    let s = sign(&doc, &req(rsa(), field("Approver")));
    assert_eq!(s.field, "Approver");
    let rep = newest_is_good(&s.bytes, rsa());
    assert_eq!(rep.signatures[0].fields, ["Approver"]);
    assert!(rep.empty_fields.is_empty());
    let w = widget(&s.bytes, "Approver");
    assert_eq!(w.get(b"F").unwrap().as_i64().unwrap(), 4 | 128);
    assert!(w.has(b"AP"));
}

#[test]
fn refuses_fields_it_cant_sign() {
    let doc = form_doc(&[("Approver", None)], false);
    refused(
        sign_document(&doc, None, &req(rsa(), field("Nope"))),
        "no field named \"Nope\"",
    );
    refused(
        sign_document(&doc, None, &req(rsa(), field("Name"))),
        "isn't a signature field",
    );
    let s = sign(&doc, &req(rsa(), field("Approver")));
    refused(
        sign_document(&s.bytes, None, &req(p256(), field("Approver"))),
        "already signed",
    );
}

#[test]
fn refuses_bad_boxes() {
    let base = base_pdf();
    let at = |page, rect| SignPlace::NewBox { page, rect };
    refused(
        sign_document(&base, None, &req(rsa(), at(0, [10.0, 10.0, 15.0, 60.0]))),
        "at least",
    );
    refused(
        sign_document(&base, None, &req(rsa(), at(0, [250.0, 10.0, 400.0, 60.0]))),
        "on the page",
    );
    refused(
        sign_document(&base, None, &req(rsa(), at(3, [10.0, 10.0, 100.0, 60.0]))),
        "no page 4",
    );
    refused(
        sign_document(
            &base,
            None,
            &req(rsa(), at(0, [f32::NAN, 10.0, 100.0, 60.0])),
        ),
        "at least",
    );
}

#[test]
fn refuses_expired_ids() {
    let expired = unlocked("expired");
    refused(
        sign_document(&base_pdf(), None, &req(&expired, new_box(0))),
        "expired on",
    );
}

#[test]
fn refuses_encrypted_documents() {
    let mut p = Pdf::from_bytes(&base_pdf(), None).unwrap();
    p.set_encryption(Some(Encryption {
        user_password: String::new(),
        owner_password: "owner".into(),
        cipher: Cipher::Aes128,
        restrictions: Restrictions::default(),
    }));
    let enc = p.to_bytes().unwrap();
    refused(
        sign_document(&enc, None, &req(rsa(), new_box(0))),
        "encrypted",
    );
    // An edited copy that would be saved encrypted is refused too.
    refused(
        sign_document(&base_pdf(), Some(&p), &req(rsa(), new_box(0))),
        "encrypted",
    );
}

#[test]
fn an_id_from_an_authority_carries_its_chain() {
    let s = sign(&base_pdf(), &req(issued(), new_box(0)));
    let rep = newest_is_good(&s.bytes, issued());
    assert_eq!(rep.signatures[0].certificates.len(), 2);
    let mut list = TrustList::default();
    list.add(&cert_der("ca"), TrustRole::Authority, None)
        .unwrap();
    let rep = verify_signatures(&s.bytes, None, &list).unwrap();
    assert!(
        matches!(rep.signatures[0].trust, Some(Trust::Authority { .. })),
        "{:?}",
        rep.signatures[0].trust
    );
}

// ------------------------------------------------------------------ certification

#[test]
fn certifies_at_each_level() {
    let doc = form_doc(&[("Approver", None)], false);
    for p in [
        Permissions::NoChanges,
        Permissions::FormFilling,
        Permissions::FormFillingAndComments,
    ] {
        let mut r = req(rsa(), new_box(2));
        r.certify = Some(p);
        let s = sign(&doc, &r);
        let rep = newest_is_good(&s.bytes, rsa());
        assert_eq!(rep.signatures[0].kind, SigKind::Certification(p));
        assert_eq!(rep.certification().map(|(_, q)| q), Some(p));
    }
}

#[test]
fn a_certification_allows_only_what_it_says() {
    let doc = form_doc(&[("Approver", None)], false);
    let certify = |p| {
        let mut r = req(rsa(), new_box(2));
        r.certify = Some(p);
        sign(&doc, &r).bytes
    };
    // Form filling and signing: the existing field can be signed, a new box can't.
    let forms = certify(Permissions::FormFilling);
    let s = sign(&forms, &req(p256(), field("Approver")));
    let rep = newest_is_good(&s.bytes, p256());
    assert!(
        rep.signatures[0].changes_allowed(),
        "{:?}",
        rep.signatures[0].changes
    );
    refused(
        sign_document(&forms, None, &req(p256(), new_box(0))),
        "only in the signature fields it already has",
    );
    // No changes: nothing.
    let none = certify(Permissions::NoChanges);
    refused(
        sign_document(&none, None, &req(p256(), field("Approver"))),
        "allowing no changes",
    );
    // Certification comes first or not at all.
    let signed = sign(&doc, &req(rsa(), field("Approver"))).bytes;
    let mut r = req(p256(), new_box(2));
    r.certify = Some(Permissions::FormFilling);
    refused(sign_document(&signed, None, &r), "only before anyone signs");
}

// ------------------------------------------------------------------ locks

#[test]
fn a_field_lock_makes_the_named_fields_read_only() {
    let doc = form_doc(
        &[
            ("Approver", Some(lock("Include", &["Name"]))),
            ("Other", None),
        ],
        false,
    );
    let s1 = sign(&doc, &req(p256(), field("Other")));
    let s2 = sign(&s1.bytes, &req(rsa(), field("Approver")));
    let rep = newest_is_good(&s2.bytes, rsa());
    let approver = rep
        .signatures
        .iter()
        .find(|s| s.fields == ["Approver"])
        .unwrap();
    let locks = approver.locks.as_ref().expect("FieldMDP written");
    assert_eq!(locks.action, LockAction::Include);
    assert_eq!(locks.fields, ["Name"]);
    let p = Pdf::from_bytes(&s2.bytes, None).unwrap();
    let name = p
        .form_fields()
        .into_iter()
        .find(|f| f.name == "Name")
        .unwrap();
    assert!(name.read_only);
    // The earlier signature sees the field made read-only as part of signing: allowed.
    let other = &rep.signatures[0];
    assert!(other.changes_allowed(), "{:?}", other.changes);
    assert!(other
        .changes
        .iter()
        .any(|c| c.kind == ChangeKind::FieldMadeReadOnly("Name".into())));
}

#[test]
fn fields_locked_by_a_signer_cant_be_signed() {
    let doc = form_doc(
        &[("Approver", Some(lock("All", &[]))), ("Other", None)],
        false,
    );
    let s = sign(&doc, &req(rsa(), field("Approver")));
    refused(
        sign_document(&s.bytes, None, &req(p256(), field("Other"))),
        "locked the field \"Other\"",
    );
    // A new signature field counts as a change to the locked form.
    refused(
        sign_document(&s.bytes, None, &req(p256(), new_box(2))),
        "doesn't allow",
    );
}

// ------------------------------------------------------------------ edits

#[test]
fn form_values_filled_since_opening_ride_along() {
    let doc = form_doc(&[("Other", None)], false);
    let s1 = sign(&doc, &req(p256(), field("Other")));
    let mut p = Pdf::from_bytes(&s1.bytes, None).unwrap();
    p.set_field("Name", FieldValue::Text("Jane Doe".into()))
        .unwrap();
    let s2 = sign_document(&s1.bytes, Some(&p), &req(rsa(), new_box(2))).unwrap();
    let rep = newest_is_good(&s2.bytes, rsa());
    assert!(rep.signatures[0].changes_allowed());
    assert!(kinds(&rep, 0).contains(&ChangeKind::FieldFilled("Name".into())));
    let filled = Pdf::from_bytes(&s2.bytes, None).unwrap();
    let name = filled
        .form_fields()
        .into_iter()
        .find(|f| f.name == "Name")
        .unwrap();
    assert_eq!(name.value, FieldValue::Text("Jane Doe".into()));
}

#[test]
fn disallowed_edits_need_the_signers_consent() {
    let s1 = sign(&base_pdf(), &req(rsa(), new_box(0)));
    let mut p = Pdf::from_bytes(&s1.bytes, None).unwrap();
    p.rotate_pages(&[0], 90).unwrap();
    refused(
        sign_document(&s1.bytes, Some(&p), &req(p256(), new_box(1))),
        "doesn't allow",
    );
    let mut r = req(p256(), new_box(1));
    r.accept_disallowed = true;
    let s2 = sign_document(&s1.bytes, Some(&p), &r).unwrap();
    let rep = newest_is_good(&s2.bytes, p256());
    assert!(!rep.signatures[0].changes_allowed());
}

#[test]
fn fill_and_sign_items_are_finished_or_locked() {
    let stamp = |p: &mut Pdf| {
        let art = SignArt::Typed {
            text: "Jane Doe",
            style: SignStyle::Caveat,
        };
        let s = Stamp::Signature {
            art,
            role: SignRole::Signature,
            color: InkColor::Blue,
            date: false,
        };
        p.add_stamp(0, &s, StampPlace::Centre(200.0, 150.0), "Jane")
            .unwrap();
    };
    // Unsigned: drawn into the page before signing.
    let base = base_pdf();
    let mut p = Pdf::from_bytes(&base, None).unwrap();
    stamp(&mut p);
    let s = sign_document(&base, Some(&p), &req(rsa(), new_box(0))).unwrap();
    assert_eq!(s.finished_items, 1);
    newest_is_good(&s.bytes, rsa());
    assert_eq!(Pdf::from_bytes(&s.bytes, None).unwrap().sign_items(), 0);
    // Signed before: locked in place, which the first signature allows as a comment.
    let s1 = sign(&base, &req(rsa(), new_box(0)));
    let mut p = Pdf::from_bytes(&s1.bytes, None).unwrap();
    stamp(&mut p);
    let s2 = sign_document(&s1.bytes, Some(&p), &req(p256(), new_box(1))).unwrap();
    assert_eq!(s2.finished_items, 0);
    let rep = newest_is_good(&s2.bytes, p256());
    assert!(
        rep.signatures[0].changes_allowed(),
        "{:?}",
        rep.signatures[0].changes
    );
    let after = Pdf::from_bytes(&s2.bytes, None).unwrap();
    assert_eq!(after.sign_items(), 1);
    let locked = after
        .doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .filter(|d| {
            d.get(b"Subtype")
                .and_then(Object::as_name)
                .is_ok_and(|s| s == b"Stamp")
        })
        .all(|d| d.get(b"F").and_then(Object::as_i64).unwrap_or(0) & 128 != 0);
    assert!(locked);
}

#[test]
fn fields_are_drawn_before_signing_when_the_form_asks_readers_to() {
    let doc = form_doc(&[("Approver", None)], true);
    let s = sign(&doc, &req(rsa(), field("Approver")));
    newest_is_good(&s.bytes, rsa());
    let p = Pdf::from_bytes(&s.bytes, None).unwrap();
    let form = p.doc.catalog().unwrap().get(b"AcroForm").unwrap();
    let form = match form {
        Object::Reference(id) => p.doc.get_dictionary(*id).unwrap(),
        o => o.as_dict().unwrap(),
    };
    assert!(!form.has(b"NeedAppearances"));
    assert!(widget(&s.bytes, "Name").has(b"AP"));
}

// ------------------------------------------------------------------ unusual files

/// The base page written with a classic cross-reference table (the test files are
/// otherwise written with cross-reference streams).
fn table_pdf() -> Vec<u8> {
    let mut doc = lopdf::Document::load_mem(&base_pdf()).unwrap();
    doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    assert!(out.windows(6).any(|w| w == b"\nxref\n"));
    out
}

#[test]
fn signs_files_with_cross_reference_tables() {
    let table = table_pdf();
    let s1 = sign(&table, &req(rsa(), new_box(0)));
    let tail = String::from_utf8_lossy(&s1.bytes[table.len()..]).into_owned();
    assert!(
        tail.contains("\nxref\n") && tail.contains("trailer"),
        "a table is appended"
    );
    newest_is_good(&s1.bytes, rsa());
    let s2 = sign(&s1.bytes, &req(p256(), new_box(1)));
    let rep = newest_is_good(&s2.bytes, p256());
    assert!(rep.signatures[0].changes_allowed());
}

#[test]
fn signs_files_with_object_streams() {
    let mut doc = lopdf::Document::load_mem(&base_pdf()).unwrap();
    let mut modern = Vec::new();
    doc.save_modern(&mut modern).unwrap();
    let s1 = sign(&modern, &req(rsa(), new_box(0)));
    let tail = String::from_utf8_lossy(&s1.bytes[modern.len()..]).into_owned();
    assert!(
        tail.contains("/XRef"),
        "a cross-reference stream is appended"
    );
    newest_is_good(&s1.bytes, rsa());
    let s2 = sign(&s1.bytes, &req(p256(), new_box(1)));
    let rep = newest_is_good(&s2.bytes, p256());
    assert!(rep.signatures[0].changes_allowed());
}

#[test]
fn a_damaged_file_is_written_out_in_full_unless_signed() {
    // `startxref` a few bytes off: lopdf finds the table nearby and opens the file.
    let damage = |b: &[u8]| {
        let at = b.windows(9).rposition(|w| w == b"startxref").unwrap() + 10;
        let n: usize = std::str::from_utf8(&b[at..])
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let mut out = b[..at].to_vec();
        out.extend_from_slice(format!("{}\n%%EOF\n", n - 3).as_bytes());
        out
    };
    let damaged = damage(&table_pdf());
    let s = sign(&damaged, &req(rsa(), new_box(0)));
    assert!(s.rewritten);
    newest_is_good(&s.bytes, rsa());
    let signed = sign(&table_pdf(), &req(rsa(), new_box(0))).bytes;
    refused(
        sign_document(&damage(&signed), None, &req(p256(), new_box(1))),
        "damaged",
    );
}

#[test]
fn signatures_on_turned_pages_read_upright() {
    let mut p = Pdf::from_bytes(&base_pdf(), None).unwrap();
    p.rotate_pages(&[0], 90).unwrap();
    let turned = p.snapshot().unwrap();
    // Shown turned, the page is 200 wide and 300 high.
    let place = SignPlace::NewBox {
        page: 0,
        rect: [10.0, 200.0, 190.0, 260.0],
    };
    let s = sign(&turned, &req(rsa(), place));
    let rep = newest_is_good(&s.bytes, rsa());
    let [x0, y0, x1, y1] = rep.signatures[0].bbox.unwrap();
    assert!((x0 - 10.0).abs() < 0.01 && (y0 - 200.0).abs() < 0.01);
    assert!((x1 - 190.0).abs() < 0.01 && (y1 - 260.0).abs() < 0.01);
    let w = widget(&s.bytes, "Signature1");
    let mk = w.get(b"MK").unwrap().as_dict().unwrap();
    assert_eq!(mk.get(b"R").unwrap().as_i64().unwrap(), 90);
    let (dict, _) = appearance(&s.bytes, "Signature1");
    assert!(dict.has(b"Matrix"));
}

// ------------------------------------------------------------------ cross-checks

/// MPE's signatures as poppler's pdfsig and OpenSSL judge them:
/// `cargo test -p mpe-core --test signing -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_with_pdfsig_and_openssl() {
    let dir = std::env::temp_dir().join(format!("mpe-signing-cross-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let doc = form_doc(&[("Approver", None)], false);
    let rsa1 = sign(&base_pdf(), &req(rsa(), new_box(0))).bytes;
    let mut certified = req(p384(), new_box(2));
    certified.certify = Some(Permissions::FormFilling);
    let cert = sign(&doc, &certified).bytes;
    let files: Vec<(&str, Vec<u8>, usize)> = vec![
        ("rsa.pdf", rsa1.clone(), 1),
        (
            "p256.pdf",
            sign(&base_pdf(), &req(p256(), new_box(0))).bytes,
            1,
        ),
        (
            "issued.pdf",
            sign(&base_pdf(), &req(issued(), new_box(0))).bytes,
            1,
        ),
        ("two.pdf", sign(&rsa1, &req(p384(), new_box(1))).bytes, 2),
        ("certified.pdf", cert.clone(), 1),
        (
            "certified-then-signed.pdf",
            sign(&cert, &req(rsa(), field("Approver"))).bytes,
            2,
        ),
    ];
    for (name, bytes, count) in &files {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let out = std::process::Command::new("pdfsig")
            .arg(&path)
            .output()
            .expect("pdfsig installed");
        let text = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            text.matches("Signature Validation: Signature is Valid.")
                .count(),
            *count,
            "{name}: {text}"
        );
        // OpenSSL checks the newest signature's CMS over its byte ranges.
        let [_, b, c, d] = last_range(bytes);
        let raw: Vec<u8> = bytes[b + 1..c - 1]
            .chunks(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect();
        let len = der_len(&raw);
        assert!(
            raw[len..].iter().all(|&x| x == 0),
            "{name}: zero padding only"
        );
        std::fs::write(dir.join("sig.der"), &raw[..len]).unwrap();
        std::fs::write(
            dir.join("data.bin"),
            [&bytes[..b], &bytes[c..c + d]].concat(),
        )
        .unwrap();
        let ok = std::process::Command::new("openssl")
            .args([
                "cms",
                "-verify",
                "-binary",
                "-noverify",
                "-inform",
                "DER",
                "-in",
            ])
            .arg(dir.join("sig.der"))
            .arg("-content")
            .arg(dir.join("data.bin"))
            .args(["-out", "/dev/null"])
            .status()
            .expect("openssl installed")
            .success();
        assert!(ok, "{name}: openssl cms -verify failed");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The length of the DER element at the start of `b`.
fn der_len(b: &[u8]) -> usize {
    let n = b[1];
    if n < 0x80 {
        2 + usize::from(n)
    } else {
        let k = usize::from(n & 0x7f);
        2 + k
            + b[2..2 + k]
                .iter()
                .fold(0, |a, &x| (a << 8) | usize::from(x))
    }
}
