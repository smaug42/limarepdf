//! Signing with a digital ID: approval signatures and certification, in new boxes and in
//! empty fields, locks, the document's own rules, edits that ride along, saving signed
//! documents without breaking them, unusual files, and the check every signed file gets
//! before it is handed out. The IDs are the
//! OpenSSL-made test files in `tests/data/signatures/ids` (password "mpe-test-password").

mod sigkit;

use limarepdf_core::{
    append_edits, sign_document, verify_signatures, AnnotKind, AppendedEdits, ChangeKind, Cipher,
    DisallowedChanges, Encryption, FieldValue, IdStore, Info, InkColor, LockAction, NewId, Pdf,
    Permissions, Restrictions, SigKind, SigStatus, SignArt, SignPlace, SignRequest, SignRole,
    SignStyle, SignatureReport, SignedDocument, Stamp, StampPlace, Trust, TrustList, TrustRole,
    UnlockedId,
};
use lopdf::{dictionary, Dictionary, Object, ObjectId};
use sigkit::*;
use std::sync::OnceLock;

const PASSWORD: &str = "mpe-test-password";

/// The test ID file `name`, imported into a scratch folder and opened.
fn unlocked(name: &str) -> UnlockedId {
    let dir = std::env::temp_dir().join(format!("limarepdf-signing-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = IdStore::new(dir.join("ids"));
    let file = std::fs::read(data_dir().join("ids").join(format!("{name}.p12"))).unwrap();
    let id = store.import(&file, PASSWORD, None).unwrap().id;
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

/// A new ID whose owner's name is beyond Western European, made in a scratch folder.
fn czech() -> &'static UnlockedId {
    static ID: OnceLock<UnlockedId> = OnceLock::new();
    ID.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("limarepdf-signing-{}-czech", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = IdStore::new(dir.join("ids"));
        let new = NewId {
            name: "Šárka Dvořák".into(),
            ..NewId::default()
        };
        let id = store.create(&new, PASSWORD).unwrap().id;
        let u = store.unlock(&id, PASSWORD).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        u
    })
}

/// The ToUnicode maps of the fonts LimarePDF embedded in `bytes`.
fn embedded_maps(bytes: &[u8]) -> Vec<String> {
    let p = Pdf::from_bytes(bytes, None).unwrap();
    p.doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .filter(|d| d.has(b"LimarePDF"))
        .map(|d| {
            let id = d.get(b"ToUnicode").unwrap().as_reference().unwrap();
            let s = p.doc.get_object(id).unwrap().as_stream().unwrap();
            String::from_utf8(s.decompressed_content().unwrap_or(s.content.clone())).unwrap()
        })
        .collect()
}

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
fn refused<T: std::fmt::Debug>(r: limarepdf_core::Result<T>, part: &str) {
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
        crop: None,
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

/// A name beyond Western European is shown, in an embedded font. Signing again makes a
/// font of its own rather than grow the first, which the first signature covers.
#[test]
fn a_name_beyond_western_european_is_shown_and_signing_again_keeps_the_first_valid() {
    let s1 = sign(&base_pdf(), &req(czech(), new_box(0)));
    newest_is_good(&s1.bytes, czech());
    let maps = embedded_maps(&s1.bytes);
    assert_eq!(maps.len(), 1);
    // ř is U+0159, Š U+0160.
    assert!(
        maps[0].contains("<0159>") && maps[0].contains("<0160>"),
        "{}",
        maps[0]
    );
    let s2 = sign(&s1.bytes, &req(czech(), new_box(1)));
    let rep = newest_is_good(&s2.bytes, czech());
    assert!(
        rep.signatures[0].changes_allowed(),
        "{:?}",
        rep.signatures[0].changes
    );
    assert_eq!(embedded_maps(&s2.bytes).len(), 2);
}

/// Text boxes beyond Western European added to a signed document, in two saves: each is
/// a comment the signature allows, and the second doesn't rewrite the first's font.
#[test]
fn text_beyond_western_european_added_after_signing_keeps_the_signature_valid() {
    let s = sign(&base_pdf(), &req(rsa(), new_box(0))).bytes;
    let style = limarepdf_core::TextStyle::default();
    let add = |t: &'static str| {
        move |p: &mut Pdf| {
            p.add_text_box(0, [20.0, 120.0, 280.0, 160.0], t, &style, "Jane")
                .unwrap();
        }
    };
    let a = save(&s, add("Съешь же ещё этих"));
    assert!(a.disallowed.is_none(), "{:?}", a.disallowed);
    let b = save(&a.bytes, add("Zażółć gęślą jaźń"));
    assert!(b.disallowed.is_none(), "{:?}", b.disallowed);
    assert!(b.bytes.starts_with(&a.bytes));
    let rep = report(&b.bytes);
    assert_eq!(rep.signatures[0].status, SigStatus::Valid);
    assert!(
        rep.signatures[0].changes_allowed(),
        "{:?}",
        rep.signatures[0].changes
    );
    assert_eq!(embedded_maps(&b.bytes).len(), 2);
}

/// PB-07: text added after going back to an earlier copy (as undo does) grows the font this
/// session embedded instead of starting another, and the signature still allows the edits.
#[test]
fn going_back_to_an_earlier_copy_keeps_growing_the_sessions_font() {
    let s = sign(&base_pdf(), &req(rsa(), new_box(0))).bytes;
    let style = limarepdf_core::TextStyle::default();
    let mut p = Pdf::from_bytes(&s, None).unwrap();
    let add = |p: &mut Pdf, t: &str, y: f32| {
        p.add_text_box(0, [20.0, y, 280.0, y + 40.0], t, &style, "Jane")
            .unwrap();
    };
    add(&mut p, "Съешь же", 120.0);
    let earlier = p.snapshot().unwrap();
    add(&mut p, "ещё этих", 60.0);
    p.reopen(&earlier).unwrap();
    add(&mut p, "Zażółć gęślą jaźń", 60.0);
    let a = append_edits(&s, &p).unwrap();
    assert!(a.disallowed.is_none(), "{:?}", a.disallowed);
    let rep = report(&a.bytes);
    assert_eq!(rep.signatures[0].status, SigStatus::Valid);
    assert!(
        rep.signatures[0].changes_allowed(),
        "{:?}",
        rep.signatures[0].changes
    );
    assert_eq!(embedded_maps(&a.bytes).len(), 1);
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
        "don't allow",
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
    let e = sign_document(&s1.bytes, Some(&p), &req(p256(), new_box(1))).unwrap_err();
    assert!(format!("{e:#}").contains("don't allow"), "{e:#}");
    // The app tells the signer who will see what, from the error itself.
    let d = e
        .downcast_ref::<DisallowedChanges>()
        .expect("a DisallowedChanges error");
    assert_eq!(d.signers.len(), 1);
    assert!(!d.signers[0].1.is_empty());
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

/// PB-02: a fill-and-sign item past the first 10,000 annotations of its page, which the
/// app shows, is counted, drawn in, and locked like any other.
#[test]
fn fill_and_sign_items_past_the_annotations_shown_are_counted_and_locked() {
    let crowd = |p: &mut Pdf| {
        let page = p.page_ids()[0];
        let had = p.doc.get_dictionary(page).unwrap().get(b"Annots");
        let mut notes = had.map_or(Vec::new(), |a| a.as_array().unwrap().clone());
        notes.extend((0..10_000).map(|_| {
            p.doc
                .add_object(dictionary! {
                    "Type" => "Annot", "Subtype" => "Text",
                    "Rect" => vec![0.into(), 0.into(), 5.into(), 5.into()],
                })
                .into()
        }));
        p.doc.get_dictionary_mut(page).unwrap().set("Annots", notes);
        p.add_sign_text(0, (200.0, 150.0), "Jane Doe", InkColor::Blue, "Jane")
            .unwrap()
    };
    let base = base_pdf();
    let mut p = Pdf::from_bytes(&base, None).unwrap();
    crowd(&mut p);
    assert_eq!(p.sign_items(), 1);
    let s = sign_document(&base, Some(&p), &req(rsa(), new_box(0))).unwrap();
    assert_eq!(s.finished_items, 1);
    // Signed before: locked in place.
    let s1 = sign(&base, &req(rsa(), new_box(0)));
    let mut p = Pdf::from_bytes(&s1.bytes, None).unwrap();
    let item = crowd(&mut p);
    assert_eq!(p.sign_items(), 1);
    let s2 = sign_document(&s1.bytes, Some(&p), &req(p256(), new_box(1))).unwrap();
    newest_is_good(&s2.bytes, p256());
    let after = Pdf::from_bytes(&s2.bytes, None).unwrap();
    let flags = after.doc.get_dictionary(item).unwrap().get(b"F");
    assert!(flags.and_then(Object::as_i64).unwrap_or(0) & 128 != 0);
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

// ------------------------------------------------------------------ saving signed documents

/// `bytes` edited by `edit` and saved as the app saves a signed document.
fn save(bytes: &[u8], edit: impl FnOnce(&mut Pdf)) -> AppendedEdits {
    let mut p = Pdf::from_bytes(bytes, None).unwrap();
    edit(&mut p);
    append_edits(bytes, &p).unwrap()
}

fn note(p: &mut Pdf) {
    p.add_note(0, (40.0, 40.0), [1.0, 0.8, 0.0], "A comment", "Jane")
        .unwrap();
}

#[test]
fn saving_without_changes_adds_nothing() {
    let s = sign(&base_pdf(), &req(rsa(), new_box(0))).bytes;
    let a = save(&s, |_| {});
    assert_eq!(a.objects, 0);
    assert_eq!(a.bytes, s);
    assert!(a.disallowed.is_none());
    // Nor after an undo, which reads the document back from a copy LimarePDF wrote itself.
    let mut p = Pdf::from_bytes(&s, None).unwrap();
    let copy = p.snapshot().unwrap();
    let back = Pdf::from_bytes(&copy, None).unwrap();
    assert_eq!(append_edits(&s, &back).unwrap().objects, 0);
}

/// Removing the document information, a change to the trailer alone, is saved after what
/// was signed (R02): the earlier revisions stay byte for byte and the signature valid.
#[test]
fn a_trailer_only_change_is_added_after_what_was_signed() {
    let s = sign(&base_pdf(), &req(rsa(), new_box(0))).bytes;
    let titled = Info {
        title: Some("Report".into()),
        ..Info::default()
    };
    let a = save(&s, |p| p.set_info(&titled).unwrap());
    assert_eq!(Pdf::from_bytes(&a.bytes, None).unwrap().info(), titled);
    let b = save(&a.bytes, |p| {
        p.doc.trailer.remove(b"Info");
    });
    assert!(b.bytes.len() > a.bytes.len() && b.bytes.starts_with(&a.bytes));
    assert_eq!(
        Pdf::from_bytes(&b.bytes, None).unwrap().info(),
        Info::default()
    );
    assert_eq!(report(&b.bytes).signatures[0].status, SigStatus::Valid);
    // Saving it again adds nothing.
    let again = save(&b.bytes, |p| {
        p.doc.trailer.remove(b"Info");
    });
    assert_eq!(again.bytes, b.bytes);
}

#[test]
fn allowed_edits_are_added_after_what_was_signed() {
    let doc = form_doc(&[("Approver", None)], false);
    let s = sign(&doc, &req(rsa(), field("Approver"))).bytes;
    let a = save(&s, |p| {
        note(p);
        p.set_field("Name", FieldValue::Text("Jane Doe".into()))
            .unwrap();
    });
    assert!(a.disallowed.is_none());
    assert!(a.objects > 0 && a.bytes.starts_with(&s));
    let rep = report(&a.bytes);
    let sig = &rep.signatures[0];
    assert_eq!(sig.status, SigStatus::Valid);
    assert!(!sig.covers_whole_file);
    assert!(sig.changes_allowed(), "{:?}", sig.changes);
    let k = kinds(&rep, 0);
    assert!(k.contains(&ChangeKind::CommentAdded), "{k:?}");
    assert!(k.contains(&ChangeKind::FieldFilled("Name".into())), "{k:?}");
    // The saved file is what the next save adds to.
    let b = save(&a.bytes, note);
    assert!(b.disallowed.is_none());
    assert!(b.bytes.starts_with(&a.bytes));
    assert!(report(&b.bytes).signatures[0].changes_allowed());
}

#[test]
fn disallowed_edits_are_named_before_saving() {
    let s = sign(&base_pdf(), &req(rsa(), new_box(0))).bytes;
    let a = save(&s, |p| p.rotate_pages(&[0], 90).unwrap());
    let d = a.disallowed.as_ref().expect("the signer is named");
    assert_eq!(d.signers.len(), 1);
    assert!(d.signers[0].0.starts_with('"'), "{:?}", d.signers);
    assert!(!d.signers[0].1.is_empty());
    // Saved anyway: the signature stays valid for what was signed and reports the change.
    let rep = report(&a.bytes);
    assert_eq!(rep.signatures[0].status, SigStatus::Valid);
    assert!(!rep.signatures[0].changes_allowed());
    // A change the signature already reports isn't named again.
    let b = save(&a.bytes, |p| p.rotate_pages(&[0], 90).unwrap());
    assert!(b.disallowed.is_none(), "{:?}", b.disallowed);
    // A new kind of change is.
    let c = save(&a.bytes, |p| p.insert_blank(1, 300.0, 200.0).unwrap());
    assert!(c.disallowed.is_some());
}

#[test]
fn a_certification_decides_what_saving_may_add() {
    for (level, comments) in [
        (Permissions::NoChanges, false),
        (Permissions::FormFilling, false),
        (Permissions::FormFillingAndComments, true),
    ] {
        let mut r = req(rsa(), new_box(0));
        r.certify = Some(level);
        let s = sign(&form_doc(&[], false), &r).bytes;
        let a = save(&s, note);
        assert_eq!(a.disallowed.is_none(), comments, "{level:?}");
        let fill = save(&s, |p| {
            p.set_field("Name", FieldValue::Text("Jane".into()))
                .unwrap()
        });
        assert_eq!(
            fill.disallowed.is_none(),
            level != Permissions::NoChanges,
            "{level:?}"
        );
    }
}

#[test]
fn a_signed_documents_fields_are_drawn_for_display_only() {
    let s = sign(
        &form_doc(&[("Approver", None)], false),
        &req(rsa(), field("Approver")),
    )
    .bytes;
    assert!(!widget(&s, "Name").has(b"AP"));
    let mut p = Pdf::from_bytes(&s, None).unwrap();
    let shown = p.app_snapshot().unwrap();
    assert!(widget(&shown, "Name").has(b"AP"));
    // Showing it changed nothing to save.
    assert_eq!(append_edits(&s, &p).unwrap().objects, 0);
}

#[test]
fn a_deleted_comment_stays_deleted_without_being_written_again() {
    let s = sign(&base_pdf(), &req(rsa(), new_box(0))).bytes;
    let a = save(&s, note);
    let mut p = Pdf::from_bytes(&a.bytes, None).unwrap();
    assert_eq!(
        p.remove_annotations(|x| x.kind == AnnotKind::Note).unwrap(),
        1
    );
    let b = append_edits(&a.bytes, &p).unwrap();
    assert!(b.disallowed.is_none());
    assert!(Pdf::from_bytes(&b.bytes, None)
        .unwrap()
        .annotations()
        .iter()
        .all(|x| x.kind != AnnotKind::Note));
    // Saving the same document again adds nothing: the note was already written as gone.
    assert_eq!(append_edits(&b.bytes, &p).unwrap().objects, 0);
}

#[test]
fn edits_that_cant_be_added_after_the_signatures_are_refused() {
    let s = sign(&base_pdf(), &req(rsa(), new_box(0))).bytes;
    // A password set: only writing the whole file can add one.
    let mut p = Pdf::from_bytes(&s, None).unwrap();
    p.set_encryption(Some(Encryption {
        user_password: String::new(),
        owner_password: "owner".into(),
        cipher: Cipher::Aes128,
        restrictions: Restrictions::default(),
    }));
    refused(append_edits(&s, &p), "password");
    // An encrypted signed file.
    let enc = p.to_bytes().unwrap();
    let opened = Pdf::from_bytes(&enc, Some("")).unwrap();
    refused(append_edits(&enc, &opened), "encrypted");
    // A damaged one, unless nothing changed.
    let damaged = misplace_startxref(&sign(&table_pdf(), &req(rsa(), new_box(0))).bytes);
    let mut p = Pdf::from_bytes(&damaged, None).unwrap();
    assert_eq!(append_edits(&damaged, &p).unwrap().bytes, damaged);
    note(&mut p);
    refused(append_edits(&damaged, &p), "damaged");
    // Edits that take a signature away.
    let s = sign(
        &form_doc(&[("Approver", None)], false),
        &req(rsa(), field("Approver")),
    )
    .bytes;
    let mut p = Pdf::from_bytes(&s, None).unwrap();
    p.flatten_form(None).unwrap();
    refused(append_edits(&s, &p), "remove 1 of its signatures");
}

/// A one-page document with an empty signature field, written by hand the way some
/// writers write numbers: `300.0`, which lopdf writes back as `300`.
fn handmade_pdf() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0.0 0.0 300.0 200.0] /Contents 4 0 R \
         /Resources << >> /Annots [5 0 R] >>",
        "<< /Length 0 >>\nstream\n\nendstream",
        "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Sig1) /Rect [20.0 20.0 220.0 70.0] \
         /P 3 0 R /F 4 >>",
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for at in offsets {
        out.extend_from_slice(format!("{at:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[test]
fn numbers_written_again_without_their_decimals_are_no_change() {
    let s = sign(&handmade_pdf(), &req(rsa(), field("Sig1"))).bytes;
    // The signed version still has the page as written by hand.
    let signed = Pdf::from_bytes(&s, None).unwrap();
    let page = signed.page_ids()[0];
    let media = signed
        .doc
        .get_dictionary(page)
        .unwrap()
        .get(b"MediaBox")
        .unwrap();
    assert!(matches!(media.as_array().unwrap()[2], Object::Real(_)));
    let a = save(&s, note);
    assert!(a.disallowed.is_none());
    assert_eq!(kinds(&report(&a.bytes), 0), [ChangeKind::CommentAdded]);
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

/// `b` with its `startxref` a few bytes off: lopdf finds the table nearby and opens the
/// file.
fn misplace_startxref(b: &[u8]) -> Vec<u8> {
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
}

#[test]
fn a_damaged_file_is_written_out_in_full_unless_signed() {
    let damage = misplace_startxref;
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

// ------------------------------------------------------------------ hidden content

#[track_caller]
fn shows_one_million(text: &str, what: impl std::fmt::Debug) {
    assert!(
        text.contains("Pay one million") && !text.contains("hundred"),
        "{what:?}: {text:?}"
    );
}

/// lopdf on its own holds object 5 in every [`HIDES`] file; LimarePDF drops it, as other apps
/// do.
#[test]
fn limarepdf_shows_what_other_apps_show() {
    let control = Pdf::from_bytes(&hidden_payment(Hide::Not), None).unwrap();
    assert!(control.page_text(0).contains("Pay one hundred"));
    assert_eq!(control.differs(), None);
    for hide in HIDES {
        let bytes = hidden_payment(hide);
        let lopdf = lopdf::Document::load_mem(&bytes).unwrap();
        assert!(lopdf.objects.contains_key(&(5, 0)), "{hide:?}");
        let pdf = Pdf::from_bytes(&bytes, None).unwrap();
        assert!(!pdf.doc.objects.contains_key(&(5, 0)), "{hide:?}");
        // The app and the CLI say so.
        assert!(pdf.differs().is_some(), "{hide:?}");
        shows_one_million(&pdf.page_text(0), hide);
    }
}

/// Object 5 listed only by an older hybrid revision's cross-reference stream: lopdf reads that
/// stream as other apps do (0.45 didn't), so LimarePDF shows what they show, signs on top of
/// the file, and checks another app's signature of it.
#[test]
fn an_older_hybrid_revision_reads_as_other_apps_read_it() {
    let bytes = hidden_payment(Hide::OlderHybrid);
    let lopdf = lopdf::Document::load_mem(&bytes).unwrap();
    assert!(lopdf.objects.contains_key(&(5, 0)));
    let pdf = Pdf::from_bytes(&bytes, None).unwrap();
    assert_eq!(pdf.differs(), None);
    assert!(pdf.page_text(0).contains("Pay one hundred"));

    let s = sign(&bytes, &req(rsa(), field("Sign here")));
    assert!(!s.rewritten);
    assert!(s.bytes.starts_with(&bytes));
    newest_is_good(&s.bytes, rsa());

    let signed = sigkit::sign(&bytes, &id("rsa3072"), &Field::default(), &Cms::default());
    let rep = report(&signed);
    let [s] = &rep.signatures[..] else {
        panic!("{} signatures", rep.signatures.len());
    };
    assert!(matches!(s.status, SigStatus::Valid), "{:?}", s.status);
}

/// Signing doesn't build on a file other apps read differently: it writes what LimarePDF shows
/// out in full first, so the signature covers that.
#[test]
fn hidden_content_is_written_out_in_full_before_signing() {
    for hide in HIDES {
        let s = sign(&hidden_payment(hide), &req(rsa(), field("Sign here")));
        assert!(s.rewritten, "{hide:?}");
        newest_is_good(&s.bytes, rsa());
        let back = Pdf::from_bytes(&s.bytes, None).unwrap();
        shows_one_million(&back.page_text(0), hide);
    }
    let s = sign(&hidden_payment(Hide::Not), &req(rsa(), field("Sign here")));
    assert!(!s.rewritten);
}

/// An ordinary deletion: a later revision frees an object nothing uses. Other apps read
/// the file as LimarePDF does, so signing builds on it.
#[test]
fn an_unused_freed_object_doesnt_stop_signing_on_top() {
    let base = hidden_payment(Hide::Not);
    let added = append_raw(&base, &[(9, b"<< /Unused true >>")], &[], &[], "");
    let freed = append_raw(&added, &[], &[9], &[], "");
    assert!(lopdf::Document::load_mem(&freed)
        .unwrap()
        .objects
        .contains_key(&(9, 0)));
    let s = sign(&freed, &req(rsa(), field("Sign here")));
    assert!(!s.rewritten);
    assert!(s.bytes.starts_with(&freed));
    newest_is_good(&s.bytes, rsa());
}

/// The same files signed by another app, which signed the bytes as they are: LimarePDF can't
/// vouch for what that signature covers, and won't add to the file.
#[test]
fn signed_hidden_content_cant_be_checked_or_added_to() {
    for hide in HIDES {
        let signed = sigkit::sign(
            &hidden_payment(hide),
            &id("rsa3072"),
            &Field::default(),
            &Cms::default(),
        );
        let rep = report(&signed);
        let [s] = &rep.signatures[..] else {
            panic!("{hide:?}: {} signatures", rep.signatures.len());
        };
        assert!(
            matches!(&s.status, SigStatus::CantCheck(w) if w.contains("differently")),
            "{hide:?}: {:?}",
            s.status
        );
        refused(
            sign_document(&signed, None, &req(rsa(), field("Sign here"))),
            "differently",
        );
        let mut p = Pdf::from_bytes(&signed, None).unwrap();
        note(&mut p);
        refused(append_edits(&signed, &p), "differently");
    }
}

// ------------------------------------------------------------------ cross-checks

/// LimarePDF's signatures as poppler's pdfsig and OpenSSL judge them:
/// `cargo test -p limarepdf-core --test signing -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_with_pdfsig_and_openssl() {
    let dir = std::env::temp_dir().join(format!("limarepdf-signing-cross-{}", std::process::id()));
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
        // Saved with a comment and a filled field added after the signature.
        (
            "saved.pdf",
            save(&sign(&doc, &req(rsa(), field("Approver"))).bytes, |p| {
                note(p);
                p.set_field("Name", FieldValue::Text("Jane Doe".into()))
                    .unwrap();
            })
            .bytes,
            1,
        ),
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

/// The [`Hide`] files as poppler reads them, before and after LimarePDF signs them:
/// `cargo test -p limarepdf-core --test signing -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_hidden_content_with_poppler() {
    let dir = std::env::temp_dir().join(format!("limarepdf-signing-hidden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let run = |tool: &str, path: &std::path::Path| {
        let out = std::process::Command::new(tool)
            .arg(path)
            .args(if tool == "pdftotext" { &["-"][..] } else { &[] })
            .output()
            .expect("poppler installed");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    for hide in HIDES.into_iter().chain([Hide::OlderHybrid]) {
        let path = dir.join(format!("{hide:?}.pdf"));
        // Apps disagree on the hiding originals: poppler 26 follows the specification for the
        // cross-reference stream, but for the others it rebuilds the cross-reference by
        // scanning and shows "Pay one hundred". So LimarePDF vouches for none of them as they
        // are; what it signs reads the same everywhere. The hybrid file poppler reads as
        // LimarePDF does, "Pay one hundred", and LimarePDF signs on top of it.
        let original = hidden_payment(hide);
        let signed = sign(&original, &req(rsa(), field("Sign here"))).bytes;
        std::fs::write(&path, &signed).unwrap();
        let text = run("pdftotext", &path);
        if hide == Hide::OlderHybrid {
            assert!(text.contains("Pay one hundred"), "{text:?}");
        } else {
            shows_one_million(&text, hide);
        }
        let sigs = run("pdfsig", &path);
        assert_eq!(
            sigs.matches("Signature Validation: Signature is Valid.")
                .count(),
            1,
            "{hide:?}: {sigs}"
        );
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
