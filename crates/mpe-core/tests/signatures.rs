//! Checking digital signatures: valid signatures of every supported kind, the published
//! attack classes (universal forgery, signature wrapping, incremental saving, copied and
//! spoofed signatures), malformed input, and third-party samples with the verdict each
//! should get. Shadow attacks are stage 3's (allowed-changes analysis).

mod sigkit;

use lopdf::{dictionary, Object};
use mpe_core::{
    verify_signatures, DigitalSignature, LockAction, Permissions, SigKind, SigStatus, SigWarning,
    SignatureReport, TimeSource,
};
use sigkit::*;

fn report(b: &[u8]) -> SignatureReport {
    verify_signatures(b, None).unwrap()
}

fn only(b: &[u8]) -> DigitalSignature {
    let r = report(b);
    assert_eq!(r.signatures.len(), 1, "{:#?}", r.signatures);
    r.signatures.into_iter().next().unwrap()
}

#[track_caller]
fn assert_valid(s: &DigitalSignature) {
    assert_eq!(s.status, SigStatus::Valid, "{s:#?}");
}

#[track_caller]
fn assert_broken(s: &DigitalSignature, why: &str) {
    match &s.status {
        SigStatus::Broken(w) => assert!(w.contains(why), "broken, but for {w:?}, not {why:?}"),
        o => panic!("expected broken ({why}), got {o:?}"),
    }
}

#[track_caller]
fn assert_cant(s: &DigitalSignature, why: &str) {
    match &s.status {
        SigStatus::CantCheck(w) => {
            assert!(w.contains(why), "can't check, but for {w:?}, not {why:?}")
        }
        o => panic!("expected can't check ({why}), got {o:?}"),
    }
}

fn sha256_colons(b: &[u8]) -> String {
    Hash::Sha256
        .of(&[b])
        .iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Rewrites the last byte range in `bytes` to `[0 b c d]`, keeping the file's length.
fn set_range(bytes: &mut [u8], new: [i64; 4]) {
    let [_, b, c, d] = last_range(bytes);
    let old = format!("0 {b} {c} {d}");
    let new = format!("{} {} {} {}", new[0], new[1], new[2], new[3]);
    let pos = bytes
        .windows(old.len())
        .rposition(|w| w == old.as_bytes())
        .unwrap();
    // The placeholder left padding spaces after the numbers.
    let end = pos + bytes[pos..].iter().position(|&x| x == b']').unwrap();
    let room = end - pos;
    assert!(new.len() <= room);
    bytes[pos..end].copy_from_slice(format!("{new:<room$}").as_bytes());
}

// ---------------------------------------------------------------- valid signatures

#[test]
fn rsa_3072_signature_is_valid() {
    let s = only(&signed());
    assert_valid(&s);
    assert!(s.covers_whole_file);
    assert_eq!(s.kind, SigKind::Approval);
    assert_eq!(s.fields, ["Signature1"]);
    assert_eq!(s.page, Some(0));
    assert_eq!(s.sub_filter, "adbe.pkcs7.detached");
    assert_eq!(s.digest, Some("SHA-256"));
    assert_eq!(s.algorithm, Some("RSA PKCS#1 v1.5"));
    assert_eq!(s.reason.as_deref(), Some("Testing"));
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    let c = s.signer.as_ref().unwrap();
    assert_eq!(c.common_name.as_deref(), Some("Test Signer RSA-3072"));
    assert_eq!(c.organisation.as_deref(), Some("MPE Tests"));
    assert_eq!(c.email.as_deref(), Some("rsa3072@example.com"));
    assert_eq!(c.key, "RSA 3072-bit");
    assert_eq!(c.key_bits, 3072);
    assert!(c.self_issued);
    assert_eq!(c.sha256, sha256_colons(&cert_der("rsa3072")));
    assert_eq!(c.not_before.to_rfc3339(), "2026-01-01T00:00:00+00:00");
    assert_eq!(c.not_after.to_rfc3339(), "2036-01-01T00:00:00+00:00");
    let t = s.time.unwrap();
    assert_eq!(t.source, TimeSource::SignerClock);
    assert_eq!(t.time.to_rfc3339(), "2026-06-01T12:00:00+00:00");
    assert_eq!(s.certificates.len(), 1);
}

#[test]
fn ec_signatures_are_valid() {
    for (name, hash, key) in [
        ("p256", Hash::Sha256, "ECDSA P-256"),
        ("p384", Hash::Sha384, "ECDSA P-384"),
        ("p256", Hash::Sha512, "ECDSA P-256"),
    ] {
        let o = Cms {
            hash,
            ..Cms::default()
        };
        let s = only(&sign(&base_pdf(), &id(name), &Field::default(), &o));
        assert_valid(&s);
        assert_eq!(s.algorithm, Some("ECDSA"));
        assert_eq!(s.signer.unwrap().key, key);
    }
}

#[test]
fn every_rsa_digest_and_padding() {
    for (hash, pss) in [
        (Hash::Sha256, true),
        (Hash::Sha384, false),
        (Hash::Sha512, false),
        (Hash::Sha512, true),
        (Hash::Sha1, false),
        (Hash::Sha1, true),
    ] {
        let o = Cms {
            hash,
            pss,
            ..Cms::default()
        };
        let s = only(&sign(&base_pdf(), &id("rsa3072"), &Field::default(), &o));
        assert_valid(&s);
        assert_eq!(
            s.algorithm,
            Some(if pss { "RSA-PSS" } else { "RSA PKCS#1 v1.5" })
        );
        let weak = s.warnings.contains(&SigWarning::WeakDigest("SHA-1"));
        assert_eq!(weak, hash == Hash::Sha1, "{hash:?}: {:?}", s.warnings);
    }
}

#[test]
fn cades_subfilter() {
    let f = Field {
        sub_filter: "ETSI.CAdES.detached",
        ..Field::default()
    };
    assert_valid(&only(&sign(
        &base_pdf(),
        &id("rsa2048"),
        &f,
        &Cms::default(),
    )));
}

#[test]
fn without_signed_attributes_the_time_comes_from_the_dictionary() {
    let o = Cms {
        signed_attrs: false,
        ..Cms::default()
    };
    let s = only(&sign(&base_pdf(), &id("rsa3072"), &Field::default(), &o));
    assert_valid(&s);
    let t = s.time.unwrap();
    assert_eq!(t.source, TimeSource::Dictionary);
    assert_eq!(t.time.to_rfc3339(), "2026-06-01T12:00:00+02:00");
}

#[test]
fn signed_attributes_are_checked_as_written() {
    // Out of DER order: re-encoding them before checking would give a false "broken".
    let o = Cms {
        unsorted: true,
        ..Cms::default()
    };
    assert_valid(&only(&sign(
        &base_pdf(),
        &id("p256"),
        &Field::default(),
        &o,
    )));
}

#[test]
fn ber_encoded_signature() {
    let o = Cms {
        ber: true,
        ..Cms::default()
    };
    assert_valid(&only(&sign(
        &base_pdf(),
        &id("rsa3072"),
        &Field::default(),
        &o,
    )));
}

#[test]
fn signer_named_by_key_identifier() {
    let o = Cms {
        ski: true,
        ..Cms::default()
    };
    assert_valid(&only(&sign(
        &base_pdf(),
        &id("p384"),
        &Field::default(),
        &o,
    )));
}

#[test]
fn adbe_pkcs7_sha1() {
    let f = Field {
        sub_filter: "adbe.pkcs7.sha1",
        ..Field::default()
    };
    let signer = id("rsa2048");
    let good = sign_with(&base_pdf(), &f, |ranges| {
        let digest = Hash::Sha1.of(ranges);
        cms(
            &signer,
            &[&digest],
            &Cms {
                econtent: Some(digest.clone()),
                ..Cms::default()
            },
        )
    });
    let s = only(&good);
    assert_valid(&s);
    assert!(s.warnings.contains(&SigWarning::WeakDigest("SHA-1")));
    let wrong = sign_with(&base_pdf(), &f, |_| {
        let digest = Hash::Sha1.of(&[b"another document"]);
        cms(
            &signer,
            &[&digest],
            &Cms {
                econtent: Some(digest.clone()),
                ..Cms::default()
            },
        )
    });
    assert_broken(&only(&wrong), "digest doesn't match");
}

#[test]
fn adbe_x509_rsa_sha1() {
    let f = Field {
        sub_filter: "adbe.x509.rsa_sha1",
        cert: Some(cert_der("rsa2048")),
        ..Field::default()
    };
    for (hash, weak) in [(Hash::Sha1, true), (Hash::Sha256, false)] {
        let b = sign_with(&base_pdf(), &f, |r| rsa_sha1_value(&id("rsa2048"), r, hash));
        let s = only(&b);
        assert_valid(&s);
        assert_eq!(s.warnings.contains(&SigWarning::WeakDigest("SHA-1")), weak);
        assert_eq!(
            s.signer.unwrap().common_name.as_deref(),
            Some("Test Signer RSA-2048")
        );
    }
    // Signed by another key than /Cert's.
    let b = sign_with(&base_pdf(), &f, |r| {
        rsa_sha1_value(&id("rsa3072"), r, Hash::Sha1)
    });
    assert_broken(&only(&b), "doesn't match");
    let no_cert = Field { cert: None, ..f };
    let b = sign_with(&base_pdf(), &no_cert, |r| {
        rsa_sha1_value(&id("rsa2048"), r, Hash::Sha1)
    });
    assert_cant(&only(&b), "/Cert");
}

#[test]
fn embedded_timestamp() {
    let o = Cms {
        timestamp: Some(("20260601120500Z", false)),
        ..Cms::default()
    };
    let s = only(&sign(&base_pdf(), &id("rsa3072"), &Field::default(), &o));
    assert_valid(&s);
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    let ts = s.timestamp.as_ref().unwrap();
    assert_eq!(ts.time.to_rfc3339(), "2026-06-01T12:05:00+00:00");
    assert_eq!(
        ts.authority.common_name.as_deref(),
        Some("Test Timestamp Authority")
    );
    assert_eq!(s.time.unwrap().source, TimeSource::Timestamp);

    // A timestamp for other data doesn't make the signature invalid, but isn't used.
    let o = Cms {
        timestamp: Some(("20260601120500Z", true)),
        ..Cms::default()
    };
    let s = only(&sign(&base_pdf(), &id("rsa3072"), &Field::default(), &o));
    assert_valid(&s);
    assert!(s.timestamp.is_none());
    assert_eq!(s.time.unwrap().source, TimeSource::SignerClock);
    assert!(
        matches!(&s.warnings[..], [SigWarning::TimestampProblem(w)] if w.contains("different data"))
    );
}

#[test]
fn document_timestamp() {
    let f = Field {
        name: "Timestamp1",
        sub_filter: "ETSI.RFC3161",
        doc_timestamp: true,
        m: None,
        reason: None,
        ..Field::default()
    };
    let b = sign_with(&signed(), &f.clone(), |r| {
        timestamp_token(&tsa(), r, "20260601130000Z", Hash::Sha256)
    });
    let r = report(&b);
    assert_eq!(r.signatures.len(), 2);
    assert_valid(&r.signatures[0]);
    let s = &r.signatures[1];
    assert_valid(s);
    assert_eq!(s.kind, SigKind::DocumentTimestamp);
    assert_eq!(s.time.unwrap().source, TimeSource::Timestamp);
    assert_eq!(
        s.time.unwrap().time.to_rfc3339(),
        "2026-06-01T13:00:00+00:00"
    );
    assert_eq!(
        s.signer.as_ref().unwrap().common_name.as_deref(),
        Some("Test Timestamp Authority")
    );
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    // An authority without the timestamping purpose.
    let b = sign_with(&base_pdf(), &f, |r| {
        timestamp_token(&id("rsa2048"), r, "20260601130000Z", Hash::Sha256)
    });
    assert!(only(&b)
        .warnings
        .contains(&SigWarning::NotATimestampAuthority));
    // A token for other data.
    let b = sign_with(&base_pdf(), &f, |_| {
        timestamp_token(&tsa(), &[b"other"], "20260601130000Z", Hash::Sha256)
    });
    assert_broken(&only(&b), "different data");
}

#[test]
fn second_signature_keeps_the_first_valid() {
    let f = Field {
        name: "Signature2",
        ..Field::default()
    };
    let b = sign(&signed(), &id("p256"), &f, &Cms::default());
    let r = report(&b);
    assert!(r.all_valid());
    let [first, second] = &r.signatures[..] else {
        panic!()
    };
    assert_eq!(first.fields, ["Signature1"]);
    assert!(!first.covers_whole_file);
    assert_eq!(second.fields, ["Signature2"]);
    assert!(second.covers_whole_file);
}

#[test]
fn signature_dictionary_inside_the_field() {
    let f = Field {
        direct: true,
        ..Field::default()
    };
    assert_valid(&only(&sign(
        &base_pdf(),
        &id("rsa3072"),
        &f,
        &Cms::default(),
    )));
}

fn doc_mdp(p: Option<i64>) -> Object {
    let mut params = dictionary! { "Type" => "TransformParams", "V" => "1.2" };
    if let Some(p) = p {
        params.set("P", p);
    }
    vec![Object::from(dictionary! {
        "Type" => "SigRef", "TransformMethod" => "DocMDP", "TransformParams" => params,
    })]
    .into()
}

#[test]
fn certification() {
    for (p, perms) in [
        (Some(1), Permissions::NoChanges),
        (Some(2), Permissions::FormFilling),
        (None, Permissions::FormFilling),
        (Some(3), Permissions::FormFillingAndComments),
    ] {
        let f = Field {
            reference: Some(doc_mdp(p)),
            perms: true,
            ..Field::default()
        };
        let r = report(&sign(&base_pdf(), &id("rsa3072"), &f, &Cms::default()));
        let (s, level) = r.certification().unwrap();
        assert_valid(s);
        assert_eq!(level, perms);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    }
    // Not registered in the catalog's /Perms.
    let f = Field {
        reference: Some(doc_mdp(Some(1))),
        ..Field::default()
    };
    let s = only(&sign(&base_pdf(), &id("rsa3072"), &f, &Cms::default()));
    assert_eq!(s.warnings, [SigWarning::CertificationNotRegistered]);
    // A certification after an approval signature.
    let f = Field {
        name: "Signature2",
        reference: Some(doc_mdp(Some(2))),
        perms: true,
        ..Field::default()
    };
    let r = report(&sign(&signed(), &id("p256"), &f, &Cms::default()));
    assert_eq!(
        r.signatures[1].warnings,
        [SigWarning::CertificationNotFirst]
    );
}

#[test]
fn field_locks() {
    let lock = dictionary! {
        "Type" => "SigRef", "TransformMethod" => "FieldMDP",
        "TransformParams" => dictionary! {
            "Type" => "TransformParams", "V" => "1.2", "Action" => "Include",
            "Fields" => vec![Object::string_literal("Name"), Object::string_literal("Date")],
        },
    };
    let f = Field {
        reference: Some(vec![Object::from(lock)].into()),
        ..Field::default()
    };
    let s = only(&sign(&base_pdf(), &id("rsa3072"), &f, &Cms::default()));
    let l = s.locks.unwrap();
    assert_eq!(l.action, LockAction::Include);
    assert_eq!(l.fields, ["Name", "Date"]);

    // An empty signature field that will lock everything.
    let b = append_update(&signed(), |inc| {
        let prev = inc.get_prev_documents();
        let root = prev.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let mut catalog = prev.get_object(root).unwrap().as_dict().unwrap().clone();
        let mut form = catalog.get(b"AcroForm").unwrap().as_dict().unwrap().clone();
        let mut fields = form.get(b"Fields").unwrap().as_array().unwrap().clone();
        let page = *prev.get_pages().get(&1).unwrap();
        let empty = inc.new_document.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Sig",
            "T" => Object::string_literal("Approver"),
            "Rect" => vec![0.into(), 0.into(), 50.into(), 20.into()], "P" => page,
            "Lock" => dictionary! { "Type" => "SigFieldLock", "Action" => "All" },
        });
        fields.push(empty.into());
        form.set("Fields", fields);
        catalog.set("AcroForm", form);
        inc.new_document
            .set_object(root, Object::Dictionary(catalog));
    });
    let r = report(&b);
    assert_eq!(r.signatures.len(), 1);
    let [e] = &r.empty_fields[..] else {
        panic!("{:?}", r.empty_fields)
    };
    assert_eq!(e.name, "Approver");
    assert_eq!(e.lock.as_ref().unwrap().action, LockAction::All);
}

#[test]
fn warnings() {
    let s = only(&sign(
        &base_pdf(),
        &id("rsa1024"),
        &Field::default(),
        &Cms::default(),
    ));
    assert_valid(&s);
    assert_eq!(s.warnings, [SigWarning::ShortKey(1024)]);
    let s = only(&sign(
        &base_pdf(),
        &id_with_cert("rsa2048", "expired"),
        &Field::default(),
        &Cms::default(),
    ));
    assert_valid(&s);
    assert_eq!(s.warnings, [SigWarning::CertificateNotValidAtSigning]);
    let s = only(&sign(
        &base_pdf(),
        &id_with_cert("rsa2048", "keyenc"),
        &Field::default(),
        &Cms::default(),
    ));
    assert_valid(&s);
    assert_eq!(s.warnings, [SigWarning::KeyNotForSigning]);
}

#[test]
fn certificate_issued_by_an_authority() {
    let o = Cms {
        extra_certs: vec![cert_der("ca")],
        ..Cms::default()
    };
    let s = only(&sign(
        &base_pdf(),
        &id_with_cert("p256", "issued"),
        &Field::default(),
        &o,
    ));
    assert_valid(&s);
    let c = s.signer.as_ref().unwrap();
    assert!(!c.self_issued);
    assert_eq!(c.issuer_common_name.as_deref(), Some("Test Root CA"));
    assert_eq!(s.certificates.len(), 2);
}

#[test]
fn signature_shared_by_two_fields() {
    let f = Field {
        share_with: Some("Copy"),
        ..Field::default()
    };
    let s = only(&sign(&base_pdf(), &id("rsa3072"), &f, &Cms::default()));
    assert_valid(&s);
    assert_eq!(s.fields, ["Signature1", "Copy"]);
    assert_eq!(s.warnings, [SigWarning::SharedByFields]);
}

#[test]
fn signature_value_padding() {
    let good = signed();
    let [_, _, c, _] = last_range(&good);
    // White space in the hex string is allowed; it hides nothing.
    let mut b = good.clone();
    b[c - 5..c - 1].copy_from_slice(b" \n00");
    assert_valid(&only(&b));
    // Something other than zeros after the signature value.
    let mut b = good.clone();
    b[c - 3..c - 1].copy_from_slice(b"11");
    let s = only(&b);
    assert_valid(&s);
    assert_eq!(s.warnings, [SigWarning::DataAfterSignature]);
}

#[test]
fn unsigned_document_has_no_signatures() {
    let r = report(&base_pdf());
    assert!(r.signatures.is_empty() && r.empty_fields.is_empty());
    assert!(r.all_valid());
}

// ---------------------------------------------------------------- byte range attacks

#[test]
fn changed_byte_breaks_the_signature() {
    let good = signed();
    // Before the signature value (the file header) and after it (the reason).
    let mut b = good.clone();
    replace_once(&mut b, b"%PDF-1.7", b"%PDF-1.6");
    assert_broken(&only(&b), "digest doesn't match");
    let mut b = good.clone();
    replace_once(&mut b, b"(Testing)", b"(Tasting)");
    assert_broken(&only(&b), "digest doesn't match");
}

#[test]
fn universal_signature_forgery() {
    let good = signed();
    let [_, b, c, d] = last_range(&good);
    // The value replaced by zeros, or emptied.
    let mut x = good.clone();
    x[b + 1..c - 1].fill(b'0');
    assert_cant(&only(&x), "malformed");
    // No byte range.
    let mut x = good.clone();
    replace_once(&mut x, b"/ByteRange", b"/ByteRangX");
    assert_broken(&only(&x), "no byte range");
    // Byte ranges that cover nothing, overlap, run past the end or are negative.
    for (range, why) in [
        ([0, 0, 0, 0], "out of order"),
        ([0, c as i64, b as i64, d as i64], "out of order"),
        ([0, b as i64, c as i64, d as i64 + 1], "past the end"),
        ([0, b as i64, c as i64, -1], "negative"),
        ([1, b as i64, c as i64, d as i64], "beginning"),
    ] {
        let mut x = good.clone();
        set_range(&mut x, range);
        assert_broken(&only(&x), why);
    }
    // /Contents renamed: the value no longer belongs to the dictionary.
    let mut x = good.clone();
    replace_once(&mut x, b"/Contents<", b"/Contentz<");
    assert_broken(&only(&x), "isn't inside");
}

#[test]
fn signature_wrapping() {
    let good = signed();
    let [_, _, c, _] = last_range(&good);
    // An entry written into the unsigned gap, in the value's zero padding: the file keeps
    // its length and still parses, and the signed bytes are untouched.
    let evil = b">/Evil<";
    let mut x = good.clone();
    x[c - 21 - evil.len()..c - 21].copy_from_slice(evil);
    assert_broken(&only(&x), "something other than the signature value");
    // Content inserted after the value, shifting the rest: the signed bytes no longer
    // match (or the file no longer opens at all).
    let mut x = good[..c].to_vec();
    x.extend_from_slice(b"\n99 0 obj\n(injected)\nendobj\n");
    x.extend_from_slice(&good[c..]);
    if let Ok(r) = verify_signatures(&x, None) {
        assert!(r.signatures.iter().all(|s| s.status != SigStatus::Valid));
    }
}

#[test]
fn signed_part_must_end_a_version() {
    let mut x = signed();
    let [_, b, c, d] = last_range(&x);
    set_range(&mut x, [0, b as i64, c as i64, d as i64 - 3]);
    assert_broken(&only(&x), "doesn't end where a version");
}

#[test]
fn copied_signature_is_refused() {
    let good = signed();
    let original = sig_id(&good, "Signature1");
    let b = append_update(&good, |inc| {
        let prev = inc.get_prev_documents();
        let copy = prev.get_object(original).unwrap().clone();
        let root = prev.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let mut catalog = prev.get_object(root).unwrap().as_dict().unwrap().clone();
        let mut form = catalog.get(b"AcroForm").unwrap().as_dict().unwrap().clone();
        let page = *prev.get_pages().get(&1).unwrap();
        let sig = inc.new_document.add_object(copy);
        let field = inc.new_document.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Sig",
            "T" => Object::string_literal("Forged"), "V" => sig,
            "Rect" => vec![0.into(), 0.into(), 0.into(), 0.into()], "P" => page,
        });
        form.set("Fields", vec![Object::from(field)]);
        catalog.set("AcroForm", form);
        inc.new_document
            .set_object(root, Object::Dictionary(catalog));
    });
    let s = only(&b);
    assert_eq!(s.fields, ["Forged"]);
    assert_broken(&s, "added after the version");
}

#[test]
fn replaced_signature_details_are_shown_as_signed() {
    let good = signed();
    let id = sig_id(&good, "Signature1");
    let b = append_update(&good, |inc| {
        let mut d = inc
            .get_prev_documents()
            .get_object(id)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone();
        d.set("Reason", Object::string_literal("Forged"));
        inc.new_document.set_object(id, Object::Dictionary(d));
    });
    let s = only(&b);
    assert_valid(&s);
    assert!(!s.covers_whole_file);
    assert_eq!(s.reason.as_deref(), Some("Testing"));
    assert_eq!(s.warnings, [SigWarning::DetailsChangedAfterSigning]);
}

#[test]
fn incremental_update_keeps_the_signature_and_is_reported() {
    let good = signed();
    let b = append_update(&good, |inc| {
        let prev = inc.get_prev_documents();
        let page = *prev.get_pages().get(&1).unwrap();
        let mut d = prev.get_object(page).unwrap().as_dict().unwrap().clone();
        d.set("Rotate", 90);
        inc.new_document.set_object(page, Object::Dictionary(d));
    });
    let s = only(&b);
    assert_valid(&s);
    assert!(!s.covers_whole_file);
    assert_eq!(s.byte_range.map(|[_, _, c, d]| c + d), Some(good.len()));
}

// ---------------------------------------------------------------- CMS attacks

#[test]
fn cms_tampering() {
    let md5 = seq(&[&oid("1.2.840.113549.2.5"), &null()]);
    let cases: Vec<(Cms, Result<(), &str>)> = vec![
        (
            Cms {
                corrupt: true,
                ..Cms::default()
            },
            Err("broken: doesn't match the signer's certificate"),
        ),
        (
            Cms {
                message_digest: Md::Wrong,
                ..Cms::default()
            },
            Err("broken: digest doesn't match"),
        ),
        (
            Cms {
                message_digest: Md::Missing,
                ..Cms::default()
            },
            Err("broken: doesn't hold the digest"),
        ),
        (
            Cms {
                message_digest: Md::Twice,
                ..Cms::default()
            },
            Err("broken: more than once"),
        ),
        (
            Cms {
                content_type: None,
                ..Cms::default()
            },
            Err("broken: no content type"),
        ),
        (
            Cms {
                content_type: Some(TST_INFO),
                ..Cms::default()
            },
            Err("broken: another kind of content"),
        ),
        (
            Cms {
                ess: Ess::Wrong,
                ..Cms::default()
            },
            Err("broken: names a different certificate"),
        ),
        (
            Cms {
                ess: Ess::None,
                ..Cms::default()
            },
            Ok(()),
        ),
        (
            Cms {
                include_cert: false,
                ..Cms::default()
            },
            Err("cant: isn't in the signature"),
        ),
        (
            Cms {
                second_signer: true,
                ..Cms::default()
            },
            Err("cant: 2 signers"),
        ),
        (
            Cms {
                econtent: Some(b"other".to_vec()),
                ..Cms::default()
            },
            Err("broken: content other than this document"),
        ),
        (
            Cms {
                digest_alg: Some(md5),
                ..Cms::default()
            },
            Err("cant: MD5"),
        ),
        (
            Cms {
                sig_alg: Some(seq(&[&oid("1.2.3.4")])),
                ..Cms::default()
            },
            Err("cant: doesn't know"),
        ),
        (
            Cms {
                sig_alg: Some(seq(&[&oid("1.2.840.10045.4.3.2")])),
                ..Cms::default()
            },
            Err("broken: doesn't fit"),
        ),
    ];
    for (o, want) in cases {
        let s = only(&sign(&base_pdf(), &id("rsa2048"), &Field::default(), &o));
        match want {
            Ok(()) => assert_valid(&s),
            Err(w) => match w.split_once(": ").unwrap() {
                ("broken", why) => assert_broken(&s, why),
                (_, why) => assert_cant(&s, why),
            },
        }
    }
    // The detached content held in the signature is fine when it is exactly the ranges.
    let b = sign_with(&base_pdf(), &Field::default(), |r| {
        cms(
            &id("rsa2048"),
            r,
            &Cms {
                econtent: Some(r.concat()),
                ..Cms::default()
            },
        )
    });
    assert_valid(&only(&b));
    // Signed with one key, carrying another's certificate.
    let o = Cms {
        ess: Ess::None,
        ..Cms::default()
    };
    let s = only(&sign(
        &base_pdf(),
        &id_with_cert("rsa3072", "rsa2048"),
        &Field::default(),
        &o,
    ));
    assert_broken(&s, "doesn't match");
    // A key on a curve MPE doesn't check.
    let s = only(&sign(
        &base_pdf(),
        &id_with_cert("p256", "p521"),
        &Field::default(),
        &o,
    ));
    assert_cant(&s, "elliptic curve");
}

#[test]
fn unknown_or_missing_kind() {
    let f = Field {
        sub_filter: "x.unknown",
        ..Field::default()
    };
    assert_cant(
        &only(&sign(&base_pdf(), &id("rsa2048"), &f, &Cms::default())),
        "doesn't know this kind",
    );
    let mut b = signed();
    replace_once(&mut b, b"/SubFilter", b"/SubFiltex");
    assert_cant(&only(&b), "no /SubFilter");
}

// ---------------------------------------------------------------- malformed input

#[test]
fn truncated_files_never_pass() {
    let good = signed();
    let cuts = (1..good.len())
        .step_by(61)
        .chain(good.len() - 40..good.len());
    for cut in cuts {
        if let Ok(r) = verify_signatures(&good[..cut], None) {
            assert!(
                r.signatures.iter().all(|s| s.status != SigStatus::Valid),
                "valid after cutting at {cut}"
            );
        }
    }
}

#[test]
fn changed_signature_values_never_pass() {
    let good = signed();
    let [_, b, c, _] = last_range(&good);
    let hex = &good[b + 1..c - 1];
    let used = hex.iter().rposition(|&x| x != b'0').unwrap() + 1;
    // The signature value is the last 384 bytes (768 hex digits) of the CMS; the signed
    // attributes hold the document digest.
    let value = (used - 768)..used;
    let digest_hex: String = Hash::Sha256
        .of(&[&good[..b], &good[c..]])
        .iter()
        .map(|x| format!("{x:02X}"))
        .collect();
    let at = hex
        .windows(64)
        .position(|w| w == digest_hex.as_bytes())
        .unwrap();
    let flip = |i: usize, must_fail: bool| {
        let mut x = good.clone();
        let p = b + 1 + i;
        x[p] = if x[p] == b'0' { b'1' } else { b'0' };
        let r = verify_signatures(&x, None).unwrap();
        if must_fail {
            assert!(
                r.signatures.iter().all(|s| s.status != SigStatus::Valid),
                "valid after changing hex digit {i}"
            );
        }
    };
    // Every digit is tried for panics; those the signature covers must never pass.
    for i in 0..used {
        flip(i, value.contains(&i) || (at..at + 64).contains(&i));
    }
}

#[test]
fn hostile_numbers_and_structures() {
    let good = signed();
    let [_, b, c, _] = last_range(&good);
    let mut x = good.clone();
    set_range(&mut x, [0, b as i64, c as i64, 99_999_999_999]);
    assert_broken(&only(&x), "past the end");
    // A length field claiming four gigabytes, at the start of the value.
    let mut x = good.clone();
    x[b + 1..b + 13].copy_from_slice(b"3084FFFFFFFF");
    assert_cant(&only(&x), "malformed");
    // The value deeply nested.
    let mut x = good.clone();
    let deep = "3080".repeat(1000);
    x[b + 1..b + 1 + deep.len()].copy_from_slice(deep.as_bytes());
    assert_cant(&only(&x), "malformed");
}

#[test]
fn encrypt_null_is_not_encryption() {
    // `/Encrypt null` in the last trailer (a classic one or a cross-reference stream's
    // dictionary) means not encrypted; such files must open and check. The DSS spoofing
    // sample has it.
    let with_null = |b: &[u8]| {
        let o = match b.windows(7).rposition(|w| w == b"trailer") {
            Some(t) => t + b[t..].windows(2).position(|w| w == b"<<").unwrap() + 2,
            None => b.windows(10).rposition(|w| w == b"/Type/XRef").unwrap() + 10,
        };
        [&b[..o], b"/Encrypt null ".as_slice(), &b[o..]].concat()
    };
    assert!(mpe_core::Pdf::from_bytes(&with_null(&base_pdf()), None).is_ok());
    let updated = append_update(&signed(), |inc| {
        let prev = inc.get_prev_documents();
        let page = *prev.get_pages().get(&1).unwrap();
        let mut d = prev.get_object(page).unwrap().as_dict().unwrap().clone();
        d.set("Rotate", 90);
        inc.new_document.set_object(page, Object::Dictionary(d));
    });
    let s = only(&with_null(&updated));
    assert_valid(&s);
    assert!(!s.covers_whole_file);
}

// ---------------------------------------------------------------- third-party samples

#[derive(Debug)]
enum Want {
    Valid,
    Broken(&'static str),
    Cant(&'static str),
}

fn check_sample(path: &std::path::Path, want: &[Want], why: &str) {
    let bytes = std::fs::read(path).unwrap();
    let r = verify_signatures(&bytes, None).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let got: Vec<&SigStatus> = r.signatures.iter().map(|s| &s.status).collect();
    assert_eq!(got.len(), want.len(), "{} ({why}): {got:?}", path.display());
    for (g, w) in got.iter().zip(want) {
        let ok = match (g, w) {
            (SigStatus::Valid, Want::Valid) => true,
            (SigStatus::Broken(g), Want::Broken(w)) | (SigStatus::CantCheck(g), Want::Cant(w)) => {
                g.contains(w)
            }
            _ => false,
        };
        assert!(ok, "{} ({why}): got {g:?}, want {w:?}", path.display());
    }
}

/// What changed after a signature, as expected in a sample.
#[derive(Debug)]
enum Chg {
    Nothing,
    Allowed,
    /// Refused, with part of the refused kind's name.
    Refused(&'static str),
}

/// Checks the changes after each signature: `Nothing` for every signature unless the
/// file is listed in `table`.
fn check_changes(path: &std::path::Path, table: &[(&str, &[Chg], &str)]) {
    let r = report(&std::fs::read(path).unwrap());
    let name = path.file_name().unwrap().to_str().unwrap();
    let listed = table.iter().find(|(f, _, _)| *f == name);
    for (i, s) in r.signatures.iter().enumerate() {
        let want = listed.map_or(&Chg::Nothing, |(_, w, _)| &w[i]);
        let refused: Vec<String> = s
            .changes
            .iter()
            .filter(|c| !c.allowed())
            .map(|c| format!("{:?}", c.kind))
            .collect();
        let ok = match want {
            Chg::Nothing => s.changes.is_empty(),
            Chg::Allowed => !s.changes.is_empty() && refused.is_empty(),
            Chg::Refused(k) => refused.iter().any(|r| r.contains(k)),
        };
        assert!(
            ok,
            "{name} signature {}: want {want:?}, got {:#?}",
            i + 1,
            s.changes
        );
    }
}

#[test]
fn pyhanko_and_node_signpdf_samples() {
    use Want::*;
    let dir = data_dir();
    let samples: &[(&str, &[Want], &str)] = &[
        (
            "pyhanko/certified-with-indirect-refs-in-dir.pdf",
            &[Valid],
            "certification",
        ),
        (
            "pyhanko/minimal-two-fields-signed-twice.pdf",
            &[Valid, Valid],
            "two signatures",
        ),
        (
            "pyhanko/one-byterange.pdf",
            &[Broken("four whole numbers")],
            "one range only",
        ),
        (
            "pyhanko/pades-lta-dss-indirect-arrs-test.pdf",
            &[Valid, Valid],
            "PAdES and a document timestamp",
        ),
        (
            "pyhanko/pades-with-old-style-signing-cert-attr.pdf",
            &[Valid],
            "ESS signing certificate v1",
        ),
        // pyHanko refuses it too ("detached ... encapsulated"); pdfsig accepts it.
        (
            "pyhanko/pdf-sig-with-econtent.pdf",
            &[Broken("content other than this document")],
            "content in a detached signature",
        ),
        // pdfsig accepts these three; pyHanko and MPE refuse the coverage.
        (
            "pyhanko/signature-gap-too-big.pdf",
            &[Broken("something other than the signature value")],
            "gap too big",
        ),
        (
            "pyhanko/tail-uncovered.pdf",
            &[Broken("doesn't end where a version")],
            "unsigned tail",
        ),
        (
            "pyhanko/weird-byterange.pdf",
            &[Broken("something other than the signature value")],
            "weird range",
        ),
        (
            "pyhanko/sig-no-signed-attrs.pdf",
            &[Valid],
            "no signed attributes",
        ),
        // pdfsig calls this invalid; it can't find signers named by key identifier.
        (
            "pyhanko/sig-with-ski-sid.pdf",
            &[Valid],
            "signer by key identifier",
        ),
        (
            "node-signpdf/signed.pdf",
            &[Valid],
            "signed attributes not in DER order",
        ),
        (
            "node-signpdf/incrementally_signed.pdf",
            &[Valid],
            "signed in an appended revision",
        ),
    ];
    // Changes after signing; every other signature has none.
    let changes: &[(&str, &[Chg], &str)] = &[
        (
            "minimal-two-fields-signed-twice.pdf",
            &[Chg::Allowed, Chg::Nothing],
            "Acrobat Reader signs the second field, locking its widget",
        ),
        (
            "pades-lta-dss-indirect-arrs-test.pdf",
            &[Chg::Allowed, Chg::Allowed],
            "document timestamp, then validation data",
        ),
    ];
    for (file, want, why) in samples {
        check_sample(&dir.join(file), want, why);
        check_changes(&dir.join(file), changes);
    }
    let r = report(
        &std::fs::read(dir.join("pyhanko/certified-with-indirect-refs-in-dir.pdf")).unwrap(),
    );
    assert!(r.certification().is_some());
    let r =
        report(&std::fs::read(dir.join("pyhanko/pades-lta-dss-indirect-arrs-test.pdf")).unwrap());
    assert_eq!(r.signatures[1].kind, SigKind::DocumentTimestamp);
}

/// The EU DSS samples: `sh tools/fetch-dss-samples.sh`, then
/// `cargo test -p mpe-core --test signatures -- --ignored dss`.
#[test]
#[ignore]
fn dss_samples() {
    use Want::*;
    let dir = data_dir().join("dss");
    assert!(
        dir.join("pades-bes.pdf").exists(),
        "run sh tools/fetch-dss-samples.sh first"
    );
    let samples: &[(&str, &[Want], &str)] = &[
        (
            "adbe_crl_signed.pdf",
            &[Valid, Valid, Valid],
            "Adobe-style, embedded CRLs",
        ),
        (
            "adbe_ocsp_signed.pdf",
            &[Valid],
            "Adobe-style, embedded OCSP",
        ),
        (
            "BadEncodedCMS.pdf",
            &[Cant("can't be read")],
            "certificate not DER-encoded",
        ),
        ("certified-changes-permitted.pdf", &[Valid], "certified"),
        ("certified-no-change-permitted.pdf", &[Valid], "certified"),
        (
            "doc-firmado.pdf",
            &[Valid, Valid],
            "signature and document timestamp",
        ),
        // pdfsig calls it invalid.
        (
            "fieldmdp-include.pdf",
            &[Valid],
            "certified with field locks",
        ),
        (
            "hello_signed_INCSAVE_signed.pdf",
            &[Valid, Valid],
            "two signatures",
        ),
        (
            "hello_signed_INCSAVE_signed_EDITED.pdf",
            &[Valid, Broken("added after the version")],
            "copied signature",
        ),
        // Shadow and incremental attacks: crypto intact; stage 3 judges the changes.
        ("hide.pdf", &[Valid], "shadow hide"),
        ("replace.pdf", &[Valid], "shadow replace"),
        (
            "modified_after_signature.pdf",
            &[Valid],
            "changed afterwards",
        ),
        (
            "pdf-signed-added-page.pdf",
            &[Valid],
            "page added afterwards",
        ),
        (
            "pades-alter-signature-appearance-modify-stream.pdf",
            &[Valid, Valid],
            "appearance changed",
        ),
        ("pades-signed-annot-added.pdf", &[Valid], "comment added"),
        (
            "malformed-rsa-digestinfo.pdf",
            &[Broken("doesn't match")],
            "malformed DigestInfo",
        ),
        (
            "pades-5-signatures-and-1-document-timestamp.pdf",
            &[Valid, Valid, Valid, Valid, Valid, Valid],
            "six",
        ),
        (
            "pades-bes-no-certificates.pdf",
            &[Cant("isn't in the signature")],
            "no certificate",
        ),
        ("pades-bes.pdf", &[Valid], "PAdES"),
        // DSS: valid, but not ETSI-conformant (the content is held in the signature).
        ("pades-enveloping-cms.pdf", &[Valid], "enveloping"),
        (
            "pades-enveloping-other-pdf.pdf",
            &[Valid, Valid],
            "another PDF held",
        ),
        (
            "pades-signed-filled-form.pdf",
            &[Valid],
            "form filled afterwards",
        ),
        (
            "pades-spoofing-replaced-reason.pdf",
            &[Valid],
            "reason replaced afterwards",
        ),
        (
            "pades-unsupported-signature-algorithm.pdf",
            &[Cant("signature algorithm")],
            "curve as algorithm",
        ),
        (
            "pdf-byterange-overlap.pdf",
            &[Valid, Broken("something other than the signature value")],
            "overlap",
        ),
        // pdfsig accepts it.
        (
            "pdf-double-signer-info.pdf",
            &[Cant("2 signers")],
            "two signers",
        ),
        (
            "pdf-eof.pdf",
            &[Broken("past the end")],
            "range past the end",
        ),
        (
            "pdf-signed-corrupted.pdf",
            &[Broken("past the end")],
            "truncated",
        ),
        ("pdf-signed-original.pdf", &[Valid], "plain"),
        (
            "pdf-signed-out-of-byteRange.pdf",
            &[Broken("past the end")],
            "range outside",
        ),
        (
            "pdf-spoofing-attack.pdf",
            &[Broken("added after the version")],
            "copied signature, /Encrypt null",
        ),
        (
            "pkcs7-no-message-digest.pdf",
            &[Valid],
            "no signed attributes",
        ),
        ("pkcs7.pdf", &[Valid], "adbe.pkcs7.detached"),
        ("pkcs7_sha1.pdf", &[Valid, Valid], "adbe.pkcs7.sha1"),
        (
            "Signature-P-IT_ADO-1 (HASH_FAILURE) (ECDSA).pdf",
            &[Valid],
            "ECDSA (DSS: valid)",
        ),
        ("Signature-P-PK7-2.pdf", &[Valid], "PKCS#7"),
        (
            "signature-referenced-from-multiple-fields.pdf",
            &[Valid],
            "shared",
        ),
        // Brainpool, not supported (nor by pdfsig).
        (
            "TeleSec_PKS_eIDAS_QES_CA_1-baseline-b.pdf",
            &[Cant("elliptic curve")],
            "brainpool",
        ),
        (
            "wrong-digest-algo.pdf",
            &[Valid, Valid, Valid, Valid],
            "signature OID as digest",
        ),
    ];
    use Chg::{Allowed as A, Nothing as N, Refused as R};
    let changes: &[(&str, &[Chg], &str)] = &[
        (
            "adbe_crl_signed.pdf",
            &[A, A, N],
            "timestamps, validation data",
        ),
        (
            "doc-firmado.pdf",
            &[A, N],
            "timestamp, validation data, form fonts",
        ),
        (
            "hello_signed_INCSAVE_signed.pdf",
            &[A, N],
            "second signature",
        ),
        // The added signature is the copied one, reported broken on its own.
        (
            "hello_signed_INCSAVE_signed_EDITED.pdf",
            &[A, N],
            "second signature",
        ),
        // Shadow hide: an entry pointing at another object.
        ("hide.pdf", &[R("Malformed")], "shadow hide"),
        // An appended object no reader can parse (-1.#INF).
        (
            "modified_after_signature.pdf",
            &[R("Malformed")],
            "unreadable object",
        ),
        // The last two revisions' /Prev point where no cross-reference is; DSS (PDFBox)
        // repairs the file and calls all six valid.
        (
            "pades-5-signatures-and-1-document-timestamp.pdf",
            &[
                R("Malformed"),
                R("Malformed"),
                R("Malformed"),
                R("Malformed"),
                R("Malformed"),
                N,
            ],
            "broken revision chain",
        ),
        (
            "pades-alter-signature-appearance-modify-stream.pdf",
            &[R("SignatureChanged"), R("SignatureChanged")],
            "appearance changed",
        ),
        ("pades-enveloping-other-pdf.pdf", &[A, N], "timestamp"),
        ("pades-signed-annot-added.pdf", &[A], "Acrobat comment"),
        (
            "pades-signed-filled-form.pdf",
            &[A],
            "form filled, Reader rights kept",
        ),
        (
            "pades-spoofing-replaced-reason.pdf",
            &[R("SignatureChanged")],
            "reason replaced",
        ),
        ("pdf-byterange-overlap.pdf", &[A, N], "second signature"),
        (
            "pdf-signed-added-page.pdf",
            &[R("PagesChanged")],
            "page added",
        ),
        ("pkcs7_sha1.pdf", &[A, N], "iText adds a signature"),
        // Shadow replace: a text field missing from the form's field list.
        ("replace.pdf", &[R("AnnotationChanged")], "shadow replace"),
        ("wrong-digest-algo.pdf", &[A, A, A, N], "timestamps"),
    ];
    for (file, want, why) in samples {
        check_sample(&dir.join(file), want, why);
        check_changes(&dir.join(file), changes);
    }
    // Not even openable: lopdf can't parse it.
    assert!(verify_signatures(
        &std::fs::read(dir.join("malformed-pades.pdf")).unwrap(),
        None
    )
    .is_err());
    let warn = |file: &str| -> Vec<SigWarning> {
        let r = report(&std::fs::read(dir.join(file)).unwrap());
        r.signatures.into_iter().flat_map(|s| s.warnings).collect()
    };
    assert!(warn("pades-spoofing-replaced-reason.pdf")
        .contains(&SigWarning::DetailsChangedAfterSigning));
    assert!(
        warn("signature-referenced-from-multiple-fields.pdf").contains(&SigWarning::SharedByFields)
    );
    assert!(
        warn("pades-signed-filled-form.pdf").contains(&SigWarning::CertificateNotValidAtSigning)
    );
    let r = report(&std::fs::read(dir.join("fieldmdp-include.pdf")).unwrap());
    assert!(r.signatures[0].locks.is_some() || r.certification().is_some());
}

// ---------------------------------------------------------------- cross-checks

/// The length of the DER element at the start of `b` (a short or long definite length).
fn der_len(b: &[u8]) -> usize {
    match b[1] {
        n if n < 0x80 => 2 + usize::from(n),
        n => {
            let k = usize::from(n & 0x7f);
            2 + k
                + b[2..2 + k]
                    .iter()
                    .fold(0, |a, &x| (a << 8) | usize::from(x))
        }
    }
}

/// MPE's verdicts on its test files against poppler's pdfsig and OpenSSL, where they can
/// judge: `cargo test -p mpe-core --test signatures -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_with_pdfsig_and_openssl() {
    let dir = std::env::temp_dir().join(format!("mpe-sig-cross-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut changed = signed();
    replace_once(&mut changed, b"(Testing)", b"(Tasting)");
    let files: Vec<(&str, Vec<u8>, &str)> = vec![
        ("rsa3072.pdf", signed(), "Signature is Valid."),
        (
            "p256.pdf",
            sign(&base_pdf(), &id("p256"), &Field::default(), &Cms::default()),
            "Signature is Valid.",
        ),
        (
            "p384.pdf",
            sign(
                &base_pdf(),
                &id("p384"),
                &Field::default(),
                &Cms {
                    hash: Hash::Sha384,
                    ..Cms::default()
                },
            ),
            "Signature is Valid.",
        ),
        (
            "pss.pdf",
            sign(
                &base_pdf(),
                &id("rsa3072"),
                &Field::default(),
                &Cms {
                    pss: true,
                    ..Cms::default()
                },
            ),
            "Signature is Valid.",
        ),
        (
            "unsorted.pdf",
            sign(
                &base_pdf(),
                &id("rsa2048"),
                &Field::default(),
                &Cms {
                    unsorted: true,
                    ..Cms::default()
                },
            ),
            "Signature is Valid.",
        ),
        ("changed.pdf", changed, "Digest Mismatch."),
        (
            "corrupt.pdf",
            sign(
                &base_pdf(),
                &id("rsa2048"),
                &Field::default(),
                &Cms {
                    corrupt: true,
                    ..Cms::default()
                },
            ),
            "Signature is Invalid.",
        ),
    ];
    for (name, bytes, pdfsig_says) in &files {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let out = std::process::Command::new("pdfsig")
            .arg(&path)
            .output()
            .expect("pdfsig installed");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains(pdfsig_says), "{name}: pdfsig said {text}");
        let mpe_valid = report(bytes).all_valid();
        assert_eq!(mpe_valid, *pdfsig_says == "Signature is Valid.", "{name}");
        // OpenSSL checks the CMS over the signed ranges.
        let [_, b, c, d] = last_range(bytes);
        let hex = &bytes[b + 1..c - 1];
        let raw: Vec<u8> = hex
            .chunks(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect();
        std::fs::write(dir.join("sig.der"), &raw[..der_len(&raw)]).unwrap();
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
        assert_eq!(ok, mpe_valid, "{name}: openssl and MPE disagree");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
