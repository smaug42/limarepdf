//! Trust: the trust list file, and whether a signer is trusted directly or through a chain
//! to a trusted certificate authority. The chains are OpenSSL-made
//! (`sh tools/make-signature-keys.sh trust`); shapes OpenSSL won't make are built by hand.
//! Signatures are made on 2026-06-01; the chains are valid 2026 to 2036 unless noted.

mod sigkit;

use limarepdf_core::{
    read_certificates, verify_signatures, Added, SigStatus, SignatureReport, Trust, TrustList,
    TrustRole,
};
use lopdf::{dictionary, Object, Stream};
use sigkit::*;

fn trusting(entries: &[(&str, TrustRole)]) -> TrustList {
    let mut l = TrustList::default();
    for (c, r) in entries {
        l.add(&cert_der(c), *r, None).unwrap();
    }
    l
}

fn root() -> TrustList {
    trusting(&[("t-root", TrustRole::Authority)])
}

/// A PDF signed with the t-leaf key and certificate `leaf`, carrying `chain` too.
fn signed_by(leaf: &str, chain: &[&str]) -> Vec<u8> {
    sign(
        &base_pdf(),
        &id_with_cert("t-leaf", leaf),
        &Field::default(),
        &Cms {
            extra_certs: chain.iter().map(|c| cert_der(c)).collect(),
            ..Cms::default()
        },
    )
}

fn report(pdf: &[u8], list: &TrustList) -> SignatureReport {
    verify_signatures(pdf, None, list).unwrap()
}

fn trust(pdf: &[u8], list: &TrustList) -> Trust {
    let r = report(pdf, list);
    assert_eq!(r.signatures.len(), 1);
    let s = &r.signatures[0];
    assert_eq!(
        s.status,
        SigStatus::Valid,
        "the signature itself must be valid"
    );
    s.trust.clone().expect("a valid signature is judged")
}

#[track_caller]
fn unknown(t: &Trust, part: &str) {
    match t {
        Trust::Unknown(why) => assert!(why.contains(part), "{why}"),
        other => panic!("expected unknown ({part}), got {other:?}"),
    }
}

#[track_caller]
fn failed(t: &Trust, part: &str) {
    match t {
        Trust::Failed(why) => assert!(why.contains(part), "{why}"),
        other => panic!("expected a failure ({part}), got {other:?}"),
    }
}

fn names(chain: &[limarepdf_core::CertInfo]) -> Vec<&str> {
    chain.iter().map(|c| c.display_name()).collect()
}

// ------------------------------------------------------------------ signers

#[test]
fn self_made_id_is_unknown_until_trusted() {
    let pdf = signed();
    unknown(&trust(&pdf, &TrustList::default()), "self-made digital ID");
    let list = trusting(&[("rsa3072", TrustRole::Signer)]);
    assert_eq!(
        trust(&pdf, &list),
        Trust::Signer {
            label: "Test Signer RSA-3072".into()
        }
    );
    assert!(report(&pdf, &list).all_trusted());
    // Another self-made ID is someone else.
    let other = sign(
        &base_pdf(),
        &id("rsa2048"),
        &Field::default(),
        &Cms::default(),
    );
    unknown(&trust(&other, &list), "self-made");
    // A self-made ID with the name of a signer you trust is an impostor, or a new ID.
    let list = trusting(&[("t-leaf-direct", TrustRole::Signer)]);
    failed(
        &trust(&signed_by("t-lookalike", &[]), &list),
        "\"Trust Test Direct Signer\" has the same name as \"Trust Test Direct Signer\", which \
         you trust, but is a different certificate",
    );
}

#[test]
fn trusted_signer_still_needs_a_usable_certificate() {
    // Expired at the signing time: trusting it doesn't change that.
    let pdf = sign(
        &base_pdf(),
        &id_with_cert("rsa2048", "expired"),
        &Field::default(),
        &Cms::default(),
    );
    failed(
        &trust(&pdf, &trusting(&[("expired", TrustRole::Signer)])),
        "wasn't valid at the signing time",
    );
    // A key not for signing, and one that is too short.
    let pdf = sign(
        &base_pdf(),
        &id_with_cert("rsa2048", "keyenc"),
        &Field::default(),
        &Cms::default(),
    );
    failed(
        &trust(&pdf, &trusting(&[("keyenc", TrustRole::Signer)])),
        "doesn't allow its key to sign",
    );
    let pdf = sign(
        &base_pdf(),
        &id("rsa1024"),
        &Field::default(),
        &Cms::default(),
    );
    failed(
        &trust(&pdf, &trusting(&[("rsa1024", TrustRole::Signer)])),
        "1024-bit RSA key",
    );
    // Meant for web servers only.
    let pdf = signed_by("t-leaf-server", &["t-inter"]);
    failed(
        &trust(&pdf, &trusting(&[("t-leaf-server", TrustRole::Signer)])),
        "other uses than signing documents",
    );
}

#[test]
fn broken_signatures_are_not_judged() {
    let mut pdf = signed();
    replace_once(&mut pdf, b"(Testing)", b"(Tasting)");
    let r = report(&pdf, &trusting(&[("rsa3072", TrustRole::Signer)]));
    assert!(matches!(r.signatures[0].status, SigStatus::Broken(_)));
    assert_eq!(r.signatures[0].trust, None);
    assert!(!r.all_trusted());
}

#[test]
fn no_signatures_is_not_all_trusted() {
    assert!(!report(&base_pdf(), &root()).all_trusted());
}

// ------------------------------------------------------------------ authorities

#[test]
fn chain_to_a_trusted_authority() {
    let pdf = signed_by("t-leaf", &["t-inter"]);
    match trust(&pdf, &root()) {
        Trust::Authority { label, chain } => {
            assert_eq!(label, "Trust Test Root");
            assert_eq!(
                names(&chain),
                [
                    "Trust Test Signer",
                    "Trust Test Intermediate",
                    "Trust Test Root"
                ]
            );
        }
        other => panic!("{other:?}"),
    }
    // Issued by the root itself; the root carried in the signature changes nothing.
    let pdf = signed_by("t-leaf-direct", &["t-root"]);
    match trust(&pdf, &root()) {
        Trust::Authority { chain, .. } => {
            assert_eq!(
                names(&chain),
                ["Trust Test Direct Signer", "Trust Test Root"]
            )
        }
        other => panic!("{other:?}"),
    }
    // An email-protection key usage is a signing one, as in Acrobat.
    assert!(trust(&signed_by("t-leaf-email", &["t-inter"]), &root()).is_trusted());
    // Policy mappings can't make a chain fail when no policy is required.
    assert!(trust(&signed_by("t-leaf-pm", &["t-pm"]), &root()).is_trusted());
    // Trusting the intermediate is enough on its own.
    assert!(trust(
        &signed_by("t-leaf", &[]),
        &trusting(&[("t-inter", TrustRole::Authority)])
    )
    .is_trusted());
}

#[test]
fn untrusted_or_missing_authorities() {
    // The whole chain carried, but nobody trusts its root.
    let pdf = signed_by("t-leaf", &["t-inter", "t-root"]);
    unknown(
        &trust(&pdf, &TrustList::default()),
        "issued by \"Trust Test Root\", which you haven't trusted",
    );
    // The intermediate isn't carried: there is no way to the trusted root.
    let pdf = signed_by("t-leaf", &[]);
    unknown(
        &trust(&pdf, &root()),
        "doesn't have the certificate of \"Trust Test Intermediate\"",
    );
    // Trusting a signer doesn't make it an authority.
    unknown(
        &trust(
            &signed_by("t-leaf", &["t-inter"]),
            &trusting(&[("t-leaf-direct", TrustRole::Signer)]),
        ),
        "nothing you trust vouches for it",
    );
}

#[test]
fn validation_data_completes_the_chain() {
    let pdf = signed_by("t-leaf", &[]);
    let with_dss = append_update(&pdf, |inc| {
        let prev = inc.get_prev_documents();
        let root_id = prev.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let mut catalog = prev.get_dictionary(root_id).unwrap().clone();
        let cert = inc
            .new_document
            .add_object(Stream::new(lopdf::Dictionary::new(), cert_der("t-inter")));
        catalog.set("DSS", dictionary! { "Certs" => vec![Object::from(cert)] });
        inc.new_document
            .set_object(root_id, Object::Dictionary(catalog));
    });
    let r = report(&with_dss, &root());
    assert_eq!(r.validation_certs.len(), 1);
    assert!(r.signatures[0].changes_allowed());
    assert!(r.signatures[0].trust.as_ref().unwrap().is_trusted());
}

#[test]
fn issuers_must_be_allowed_to_issue() {
    let list = root();
    let cases = [
        (
            "t-leaf-nonca",
            "t-nonca",
            "doesn't say it is a certificate authority",
        ),
        (
            "t-leaf-nocertsign",
            "t-nocertsign",
            "doesn't allow its key to sign certificates",
        ),
        (
            "t-leaf-under-expired",
            "t-inter-expired",
            "Expired Intermediate\" wasn't valid at the signing time",
        ),
        (
            "t-leaf-nc",
            "t-nc",
            "name constraints, which LimarePDF doesn't check",
        ),
        (
            "t-leaf-pm-any",
            "t-pm-any",
            "has policy mappings that aren't allowed",
        ),
        ("t-leaf-weak", "t-weak", "1024-bit RSA key"),
        ("t-leaf-sha1", "t-sha1", "signed with SHA-1"),
    ];
    for (leaf, issuer, why) in cases {
        let t = trust(&signed_by(leaf, &[issuer]), &list);
        failed(&t, why);
        // Every failure names the trusted authority it was on the way to.
        failed(&t, "under \"Trust Test Root\", which you trust");
    }
    // An authority below one that allows none below it.
    failed(
        &trust(&signed_by("t-leaf-sub", &["t-sub", "t-inter"]), &list),
        "allows at most 0 authorities below it",
    );
}

#[test]
fn required_policies() {
    // An authority requiring a policy (policy constraints, as qualified-signature
    // authorities set): RFC 5280 6.1 with any policy acceptable.
    let list = root();
    let cases = [
        ("t-leaf-pc", "t-pc", true),
        ("t-leaf-pc-other", "t-pc", false),
        ("t-leaf-pc-none", "t-pc", false),
        // 1.2.3.4 is mapped to 1.2.3.5 below the authority.
        ("t-leaf-pc-mapped", "t-pc-map", true),
        ("t-leaf-pc-unmapped", "t-pc-map", false),
        // Any policy is no longer accepted below an authority that inhibits it.
        ("t-leaf-pc-any", "t-pc-any", false),
        ("t-leaf-pc-named", "t-pc-any", true),
    ];
    for (leaf, issuer, ok) in cases {
        let t = trust(&signed_by(leaf, &[issuer]), &list);
        if ok {
            assert!(t.is_trusted(), "{leaf}: {t:?}");
        } else {
            failed(&t, "no policy runs through every certificate");
        }
    }
}

#[test]
fn signer_certificate_problems_under_a_trusted_authority() {
    let list = root();
    failed(
        &trust(&signed_by("t-leaf-server", &["t-inter"]), &list),
        "other uses than signing documents",
    );
    failed(
        &trust(&signed_by("t-leaf-critical", &["t-inter"]), &list),
        "critical extension LimarePDF doesn't know (1.2.3.4)",
    );
    failed(
        &trust(&signed_by("t-leaf-expired", &["t-inter"]), &list),
        "\"Trust Test Expired Signer\" wasn't valid at the signing time",
    );
}

#[test]
fn forged_issuer_is_never_trusted() {
    // A signer "issued" by another key calling itself Trust Test Root.
    for chain in [&[][..], &["t-evil"][..]] {
        let t = trust(&signed_by("t-leaf-evil", chain), &root());
        failed(
            &t,
            "names \"Trust Test Root\", which you trust, as its issuer, but the issuer's \
             signature on the certificate doesn't match",
        );
    }
    // With a key identifier, the trusted root isn't even a candidate.
    let t = trust(&signed_by("t-leaf-evil-keyid", &[]), &root());
    unknown(&t, "a different certificate of that name issued it");
    // Carrying the impostor root: it is named for what it is.
    let t = trust(&signed_by("t-leaf-evil-keyid", &["t-evil"]), &root());
    failed(
        &t,
        "\"Trust Test Root\" has the same name as \"Trust Test Root\", which you trust, but is a \
         different certificate",
    );
    // Trusting the impostor root trusts only what it issued.
    let evil = trusting(&[("t-evil", TrustRole::Authority)]);
    assert!(trust(&signed_by("t-leaf-evil", &[]), &evil).is_trusted());
    assert!(!trust(&signed_by("t-leaf", &["t-inter"]), &evil).is_trusted());
}

#[test]
fn loops_and_long_chains_end() {
    let pdf = signed_by("t-leaf-loop", &["t-loop-a", "t-loop-b"]);
    unknown(&trust(&pdf, &root()), "");
    let deep: Vec<String> = (1..=10).map(|n| format!("t-deep-{n}")).collect();
    let deep: Vec<&str> = deep.iter().map(String::as_str).collect();
    unknown(
        &trust(&signed_by("t-leaf-deep", &deep), &root()),
        "too long to follow",
    );
    // Nine of them is within reach.
    assert!(trust(
        &signed_by("t-leaf-deep", &deep),
        &trusting(&[("t-deep-2", TrustRole::Authority)])
    )
    .is_trusted());
}

// ------------------------------------------------------------------ timestamps

#[test]
fn timestamp_authorities_are_judged_too() {
    let pdf = sign(
        &base_pdf(),
        &id("rsa3072"),
        &Field::default(),
        &Cms {
            timestamp: Some(("20260601120500Z", false)),
            ..Cms::default()
        },
    );
    let r = report(&pdf, &TrustList::default());
    let ts = r.signatures[0].timestamp.as_ref().unwrap();
    unknown(ts.trust.as_ref().unwrap(), "self-made");
    let r = report(&pdf, &trusting(&[("tsa", TrustRole::Signer)]));
    let ts = r.signatures[0].timestamp.as_ref().unwrap();
    assert!(ts.trust.as_ref().unwrap().is_trusted());
    // The signer is a separate question.
    unknown(r.signatures[0].trust.as_ref().unwrap(), "self-made");

    // A document timestamp: its authority is its signer, and must be meant for timestamps.
    let f = Field {
        name: "Timestamp1",
        sub_filter: "ETSI.RFC3161",
        doc_timestamp: true,
        invisible: true,
        m: None,
        reason: None,
        ..Field::default()
    };
    let stamped = sign_with(&base_pdf(), &f, |r| {
        timestamp_token(&tsa(), r, "20260601130000Z", Hash::Sha256)
    });
    assert!(trust(&stamped, &trusting(&[("tsa", TrustRole::Signer)])).is_trusted());
    let wrong = sign_with(&base_pdf(), &f, |r| {
        timestamp_token(&id("rsa2048"), r, "20260601130000Z", Hash::Sha256)
    });
    failed(
        &trust(&wrong, &trusting(&[("rsa2048", TrustRole::Signer)])),
        "isn't meant for timestamps",
    );
}

/// A timestamp authority (the rsa2048 key) issued by an intermediate authority (the
/// rsa3072 key) under the test CA (`ca`), and the intermediate's certificate.
fn tsa_under_intermediate() -> (Id, Vec<u8>) {
    let ca_bc = ext("2.5.29.19", true, &seq(&[&[0x01, 0x01, 0xff]]));
    let inter = make_cert(
        &id_with_cert("ca", "ca"),
        &CertSpec {
            key_of: &cert_der("rsa3072"),
            cn: "Timestamp Intermediate",
            not_before: "260101000000Z",
            not_after: "360101000000Z",
            extensions: vec![ca_bc, ext_authority_key_id(&cert_der("ca"))],
            inner_alg: None,
        },
    );
    let issuer = Id {
        key: id("rsa3072").key,
        cert: inter.clone(),
    };
    let cert = make_cert(
        &issuer,
        &CertSpec {
            key_of: &cert_der("rsa2048"),
            cn: "Intermediate Timestamp Authority",
            not_before: "260101000000Z",
            not_after: "360101000000Z",
            extensions: vec![
                ext("2.5.29.15", true, &[0x03, 0x02, 0x07, 0x80]),
                ext("2.5.29.37", true, &seq(&[&oid("1.3.6.1.5.5.7.3.8")])),
            ],
            inner_alg: None,
        },
    );
    let tsa = Id {
        key: id("rsa2048").key,
        cert,
    };
    (tsa, inter)
}

#[test]
fn document_timestamps_are_judged_with_the_certificates_they_carry() {
    let (tsa, inter) = tsa_under_intermediate();
    let f = Field {
        name: "Timestamp1",
        sub_filter: "ETSI.RFC3161",
        doc_timestamp: true,
        invisible: true,
        m: None,
        reason: None,
        ..Field::default()
    };
    let stamped = |extra: Vec<Vec<u8>>| {
        sign_with(&base_pdf(), &f, |r| {
            timestamp_token_carrying(&tsa, r, "20260601130000Z", Hash::Sha256, extra)
        })
    };
    // The intermediate carried in the token completes the way to the trusted authority.
    let carried = stamped(vec![inter.clone()]);
    match trust(&carried, &ca()) {
        Trust::Authority { chain, .. } => assert_eq!(
            names(&chain),
            [
                "Intermediate Timestamp Authority",
                "Timestamp Intermediate",
                "Test Root CA"
            ]
        ),
        other => panic!("{other:?}"),
    }
    let r = report(&carried, &ca());
    assert_eq!(
        names(&r.signatures[0].certificates),
        ["Intermediate Timestamp Authority", "Timestamp Intermediate"]
    );
    // Not carried: there is no way to the trusted authority.
    unknown(
        &trust(&stamped(Vec::new()), &ca()),
        "doesn't have the certificate of \"Timestamp Intermediate\"",
    );
    // Carrying the root too makes nothing trusted that the list doesn't vouch for.
    unknown(
        &trust(&stamped(vec![inter, cert_der("ca")]), &TrustList::default()),
        "which you haven't trusted",
    );
}

#[test]
fn trust_can_be_applied_again() {
    let pdf = signed_by("t-leaf", &["t-inter"]);
    let mut r = report(&pdf, &TrustList::default());
    assert!(!r.all_trusted());
    r.apply_trust(&root());
    assert!(r.all_trusted());
    r.apply_trust(&TrustList::default());
    assert!(!r.all_trusted());
}

// ------------------------------------------------------------------ hand-made certificates

/// A signer certificate for the p256 key, issued by the test CA (`ca`), with `extensions`.
fn crafted(extensions: Vec<Vec<u8>>, inner_alg: Option<Vec<u8>>) -> Vec<u8> {
    make_cert(
        &id_with_cert("ca", "ca"),
        &CertSpec {
            key_of: &cert_der("p256"),
            cn: "Crafted Signer",
            not_before: "260101000000Z",
            not_after: "360101000000Z",
            extensions,
            inner_alg,
        },
    )
}

fn signed_with_cert(cert: Vec<u8>) -> Vec<u8> {
    let signer = Id {
        key: id("p256").key,
        cert,
    };
    sign(&base_pdf(), &signer, &Field::default(), &Cms::default())
}

fn ca() -> TrustList {
    trusting(&[("ca", TrustRole::Authority)])
}

#[test]
fn crafted_certificates() {
    let aki = ext_authority_key_id(&cert_der("ca"));
    // Well formed: trusted, which shows the kit makes real certificates.
    let good = crafted(vec![ext_signing_key_usage(), aki.clone()], None);
    assert!(trust(&signed_with_cert(good.clone()), &ca()).is_trusted());

    // The same extension twice.
    let twice = crafted(
        vec![
            ext_signing_key_usage(),
            ext_signing_key_usage(),
            aki.clone(),
        ],
        None,
    );
    failed(
        &trust(&signed_with_cert(twice), &ca()),
        "repeats an extension",
    );

    // A key usage that can't be read doesn't count as no limit.
    let garbled = crafted(
        vec![ext("2.5.29.15", true, &[0x04, 0x01, 0x00]), aki.clone()],
        None,
    );
    failed(
        &trust(&signed_with_cert(garbled), &ca()),
        "has an extension LimarePDF can't read (2.5.29.15)",
    );

    // The two copies of the signature algorithm differ.
    let differ = crafted(
        vec![ext_signing_key_usage(), aki.clone()],
        Some(seq(&[&oid("1.2.840.113549.1.1.12"), &null()])),
    );
    failed(
        &trust(&signed_with_cert(differ), &ca()),
        "its two signature algorithms differ",
    );

    // Changed after the authority signed it: one letter of the name.
    let mut changed = good.clone();
    let at = changed.windows(7).position(|w| w == b"Crafted").unwrap();
    changed[at] = b'D';
    failed(
        &trust(&signed_with_cert(changed), &ca()),
        "the issuer's signature on the certificate doesn't match",
    );
}

#[test]
fn many_candidate_issuers_stay_bounded() {
    // 250 authorities all named like the signer's issuer, carrying the issuer's key, so
    // each one's signature on the signer checks out; none leads anywhere trusted.
    let ca_id = || id_with_cert("ca", "ca");
    let bc = ext("2.5.29.19", true, &seq(&[&[0x01, 0x01, 0xff]]));
    let crowd = |n: usize| {
        make_cert(
            &ca_id(),
            &CertSpec {
                key_of: &cert_der("ca"),
                cn: "Crowd",
                not_before: &format!("2601{:02}{:02}0000Z", 1 + n / 24 % 28, n % 24),
                not_after: "360101000000Z",
                extensions: vec![bc.clone()],
                inner_alg: None,
            },
        )
    };
    let issuer = Id {
        key: ca_id().key,
        cert: crowd(0),
    };
    let leaf = make_cert(
        &issuer,
        &CertSpec {
            key_of: &cert_der("p256"),
            cn: "Crowd Signer",
            not_before: "260101000000Z",
            not_after: "360101000000Z",
            extensions: vec![ext_signing_key_usage()],
            inner_alg: None,
        },
    );
    let pdf = sign(
        &base_pdf(),
        &Id {
            key: id("p256").key,
            cert: leaf,
        },
        &Field {
            room: 400_000,
            ..Field::default()
        },
        &Cms {
            extra_certs: (0..250).map(crowd).collect(),
            ..Cms::default()
        },
    );
    let started = std::time::Instant::now();
    let t = trust(&pdf, &trusting(&[("t-root", TrustRole::Authority)]));
    assert!(!t.is_trusted(), "{t:?}");
    assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
}

// ------------------------------------------------------------------ the trust list

#[test]
fn adding_and_removing() {
    let mut l = TrustList::default();
    assert_eq!(
        l.add(&cert_der("rsa3072"), TrustRole::Signer, Some("Jane\nDoe"))
            .unwrap(),
        Added::New
    );
    // Labels stay on one line.
    assert_eq!(l.entries[0].label, "Jane Doe");
    assert!(l.entries[0].added.is_some());
    let again = l
        .add(&cert_der("rsa3072"), TrustRole::Signer, None)
        .unwrap_err();
    assert!(
        again.to_string().contains("already trusted as a signer"),
        "{again}"
    );
    // Not an authority.
    let e = l
        .add(&cert_der("rsa3072"), TrustRole::Authority, None)
        .unwrap_err();
    assert!(
        e.to_string()
            .contains("doesn't say it is a certificate authority"),
        "{e}"
    );
    let e = l
        .add(&cert_der("t-nocertsign"), TrustRole::Authority, None)
        .unwrap_err();
    assert!(e.to_string().contains("sign certificates"), "{e}");
    assert!(l
        .add(b"not a certificate", TrustRole::Signer, None)
        .is_err());
    // A role change.
    l.add(&cert_der("t-root"), TrustRole::Signer, None).unwrap();
    assert_eq!(
        l.add(&cert_der("t-root"), TrustRole::Authority, None)
            .unwrap(),
        Added::RoleChanged(TrustRole::Signer)
    );
    assert_eq!(l.entries.len(), 2);
    // Found by fingerprint in any spelling.
    let fp = l.entries[1].cert.sha256.clone();
    assert_eq!(l.find(&fp), Some(1));
    assert_eq!(l.find(&fp.replace(':', "").to_lowercase()), Some(1));
    assert_eq!(l.find(&fp.replace(':', " ")), Some(1));
    assert_eq!(l.find(""), None);
    assert_eq!(l.find("00"), None);
    assert_eq!(l.remove(1).unwrap().label, "Trust Test Root");
    assert!(l.remove(5).is_none());
    assert_eq!(l.entries.len(), 1);
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("limarepdf-trust-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn file_round_trip() {
    let dir = scratch("round-trip");
    let path = dir.join("settings").join("trust.txt");
    // Missing is empty.
    assert!(TrustList::load(&path).unwrap().entries.is_empty());
    let mut l = TrustList::default();
    l.add(&cert_der("rsa3072"), TrustRole::Signer, Some("Jane"))
        .unwrap();
    l.add(&cert_der("t-root"), TrustRole::Authority, None)
        .unwrap();
    l.save(&path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let back = TrustList::load(&path).unwrap();
    assert_eq!(back.entries, l.entries);
    assert!(back.problems.is_empty());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("affect LimarePDF only"));
    assert!(text.contains("[signer]\nlabel = Jane\n"));
    assert!(text.contains("[authority]\nlabel = Trust Test Root\n"));
    assert!(!dir.join("settings").join("trust.txt.tmp").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn file_others_can_change_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("perms");
    let path = dir.join("trust.txt");
    trusting(&[("rsa3072", TrustRole::Signer)])
        .save(&path)
        .unwrap();
    for mode in [0o620, 0o602, 0o666] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        let e = TrustList::load(&path).unwrap_err();
        assert!(e.to_string().contains("other users"), "{e}");
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(TrustList::load(&path).unwrap().entries.len(), 1);
    // Saving makes it private again.
    TrustList::load(&path).unwrap().save(&path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unusable_entries_are_left_out_and_kept() {
    let good = trusting(&[("rsa3072", TrustRole::Signer)]).to_text();
    let good_block = &good[good.find("[signer]").unwrap()..];
    let pem_root = root().to_text();
    let root_block = &pem_root[pem_root.find("[authority]").unwrap()..];
    // The fingerprint written doesn't match the certificate.
    let wrong_fp = root_block
        .lines()
        .map(|l| {
            if l.starts_with("sha256 = ") {
                "sha256 = 00:11".to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    // A signer-only certificate listed as an authority.
    let not_ca = good_block.replace("[signer]", "[authority]");
    let text = format!(
        "{good}\n{wrong_fp}\n\n[friend]\nlabel = x\n\n[signer]\nlabel = no certificate\n\n\
         [signer]\n-----BEGIN CERTIFICATE-----\nMIIB\n\n{not_ca}\n[signer]\nadded = yesterday\n\
         \n{good_block}"
    );
    let l = TrustList::parse(&text);
    assert_eq!(l.entries.len(), 1, "{:?}", l.problems);
    let p = l.problems.join("\n");
    for part in [
        "fingerprint written doesn't match",
        "unknown kind of entry [friend]",
        "has no certificate",
        "isn't complete",
        "can't be a certificate authority",
        "repeats the certificate",
        "unreadable date",
    ] {
        assert!(p.contains(part), "{part}: {p}");
    }
    // Nothing unusable gives trust: the root with the wrong fingerprint isn't trusted.
    let pdf = signed_by("t-leaf-direct", &[]);
    unknown(&trust(&pdf, &l), "");
    // Saving keeps unusable entries as written.
    let saved = l.to_text();
    assert!(saved.contains("sha256 = 00:11"));
    assert!(saved.contains("[friend]"));
    assert_eq!(TrustList::parse(&saved).entries, l.entries);
}

#[test]
fn reading_certificate_files() {
    let der = cert_der("t-root");
    let one = read_certificates(&der).unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].der, der);
    // PEM, with text around it, and two in one file.
    let text = trusting(&[
        ("t-root", TrustRole::Signer),
        ("t-inter", TrustRole::Signer),
    ])
    .to_text();
    let two = read_certificates(text.as_bytes()).unwrap();
    assert_eq!(names(&two), ["Trust Test Root", "Trust Test Intermediate"]);
    // A PKCS#7 bundle (.p7c), as Acrobat exports a certificate.
    let p7c = seq(&[
        &oid(SIGNED_DATA),
        &ctx(
            0,
            &seq(&[
                &int(1),
                &set(&[]),
                &seq(&[&oid(DATA)]),
                &ctx(0, &[cert_der("t-leaf"), cert_der("t-inter")].concat()),
                &set(&[]),
            ]),
        ),
    ]);
    assert_eq!(
        names(&read_certificates(&p7c).unwrap()),
        ["Trust Test Signer", "Trust Test Intermediate"]
    );
    for bad in [
        &b""[..],
        b"hello",
        b"-----BEGIN CERTIFICATE-----\nAAAA\n",
        &der[..100],
    ] {
        assert!(read_certificates(bad).is_err());
    }
    let key = b"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n";
    assert!(read_certificates(key).is_err());
}

/// LimarePDF's chain verdicts against `openssl verify` at the signing time, where OpenSSL judges
/// the same thing: `cargo test -p limarepdf-core --test signature_trust -- --ignored`.
#[test]
#[ignore]
fn cross_check_with_openssl() {
    let dir = scratch("openssl");
    std::fs::create_dir_all(&dir).unwrap();
    let pem_of = |names: &[&str], file: &str| {
        let mut out = String::new();
        for n in names {
            out.push_str(
                &der::pem::encode_string("CERTIFICATE", der::pem::LineEnding::LF, &cert_der(n))
                    .unwrap(),
            );
        }
        let p = dir.join(file);
        std::fs::write(&p, out).unwrap();
        p
    };
    let root_pem = pem_of(&["t-root"], "root.pem");
    // (signer, intermediates). Left out because LimarePDF is stricter by design, where OpenSSL
    // says OK: name constraints (OpenSSL applies them, LimarePDF refuses them), a 1024-bit RSA
    // authority and a SHA-1 link (allowed at OpenSSL's default security level), and a policy
    // mapping to any policy (forbidden by RFC 5280 6.1.4 (a); OpenSSL accepts it).
    let cases: &[(&str, &[&str])] = &[
        ("t-leaf", &["t-inter"]),
        ("t-leaf-direct", &[]),
        ("t-leaf", &[]),
        ("t-leaf-nonca", &["t-nonca"]),
        ("t-leaf-nocertsign", &["t-nocertsign"]),
        ("t-leaf-sub", &["t-sub", "t-inter"]),
        ("t-leaf-under-expired", &["t-inter-expired"]),
        ("t-leaf-expired", &["t-inter"]),
        ("t-leaf-evil", &["t-evil"]),
        ("t-leaf-critical", &["t-inter"]),
        ("t-leaf-server", &["t-inter"]),
        ("t-leaf-email", &["t-inter"]),
        ("t-leaf-pm", &["t-pm"]),
        ("t-leaf-pc", &["t-pc"]),
        ("t-leaf-pc-other", &["t-pc"]),
        ("t-leaf-pc-none", &["t-pc"]),
        ("t-leaf-pc-mapped", &["t-pc-map"]),
        ("t-leaf-pc-unmapped", &["t-pc-map"]),
        ("t-leaf-pc-any", &["t-pc-any"]),
        ("t-leaf-pc-named", &["t-pc-any"]),
    ];
    for (leaf, chain) in cases {
        let leaf_pem = pem_of(&[leaf], "leaf.pem");
        let mut cmd = std::process::Command::new("openssl");
        // The signing time, 2026-06-01 12:00 UTC; S/MIME signing checks key usage as
        // LimarePDF does for signers.
        cmd.args([
            "verify",
            "-attime",
            "1780315200",
            "-purpose",
            "smimesign",
            // RFC 5280 policy processing with any policy acceptable, as LimarePDF does.
            "-policy_check",
            "-policy",
            "2.5.29.32.0",
            "-CAfile",
        ])
        .arg(&root_pem);
        if !chain.is_empty() {
            cmd.arg("-untrusted").arg(pem_of(chain, "chain.pem"));
        }
        let ok = cmd
            .arg(&leaf_pem)
            .output()
            .expect("openssl installed")
            .status
            .success();
        let limarepdf = trust(&signed_by(leaf, chain), &root()).is_trusted();
        assert_eq!(limarepdf, ok, "{leaf}: LimarePDF and OpenSSL disagree");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
