//! Trust: the trust list file, and whether a signer is trusted directly or through a chain
//! to a trusted certificate authority. The chains are OpenSSL-made
//! (`sh tools/make-signature-keys.sh trust`); shapes OpenSSL won't make are built by hand.
//! Signatures are made on 2026-06-01; the chains are valid 2026 to 2036 unless noted.

mod sigkit;
#[allow(dead_code)]
#[path = "common/tool.rs"]
mod tool;

use limarepdf_core::{
    read_certificates, verify_signatures, Added, SigKind, SigStatus, SigWarning, SignatureReport,
    TimeSource, Trust, TrustList, TrustRole,
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

    // Signed with RSA-PSS's defaults (SHA-1, salt 20): an empty parameter list names
    // them (so the signature matches, and SHA-1 is refused), but left-out or NULL
    // parameters, which RFC 4055 doesn't allow, aren't read as them.
    let pss_signed = |alg: Vec<u8>| {
        make_cert_pss(
            &id_with_cert("ca", "ca"),
            &CertSpec {
                key_of: &cert_der("p256"),
                cn: "Crafted Signer",
                not_before: "260101000000Z",
                not_after: "360101000000Z",
                extensions: vec![ext_signing_key_usage(), aki.clone()],
                inner_alg: None,
            },
            alg,
            Hash::Sha1,
            20,
        )
    };
    failed(
        &trust(
            &signed_with_cert(pss_signed(pss_alg(Some(&seq(&[]))))),
            &ca(),
        ),
        "is signed with SHA-1",
    );
    for params in [None, Some(null())] {
        failed(
            &trust(
                &signed_with_cert(pss_signed(pss_alg(params.as_deref()))),
                &ca(),
            ),
            "leaves out its RSA-PSS parameters",
        );
    }

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
fn rsa_pss_only_authority_keys() {
    // An authority whose RSA-2048 key is limited by `key_alg` (RFC 4055), issued by the
    // test CA.
    let authority = |key_alg: &[u8]| {
        make_cert_with(
            &id_with_cert("ca", "ca"),
            &CertSpec {
                key_of: &cert_der("rsa2048"),
                cn: "PSS Authority",
                not_before: "260101000000Z",
                not_after: "360101000000Z",
                extensions: vec![
                    ext("2.5.29.19", true, &seq(&[&[0x01, 0x01, 0xff]])),
                    // keyCertSign and cRLSign.
                    ext("2.5.29.15", true, &[0x03, 0x02, 0x01, 0x06]),
                ],
                inner_alg: None,
            },
            Some(key_alg),
            false,
        )
    };
    // Signed with a P-256 certificate the authority issued, signing it with RSA-PSS or
    // PKCS#1 v1.5.
    let signed_under = |authority: &[u8], pss: bool| {
        let issuer = Id {
            key: id("rsa2048").key,
            cert: authority.to_vec(),
        };
        let leaf = make_cert_with(
            &issuer,
            &CertSpec {
                key_of: &cert_der("p256"),
                cn: "PSS Authority Signer",
                not_before: "260101000000Z",
                not_after: "360101000000Z",
                extensions: vec![ext_signing_key_usage()],
                inner_alg: None,
            },
            None,
            pss,
        );
        sign(
            &base_pdf(),
            &Id {
                key: id("p256").key,
                cert: leaf,
            },
            &Field::default(),
            &Cms {
                extra_certs: vec![authority.to_vec()],
                ..Cms::default()
            },
        )
    };
    let any_pss = authority(&pss_alg(None));
    assert!(trust(&signed_under(&any_pss, true), &ca()).is_trusted());
    let t = trust(&signed_under(&any_pss, false), &ca());
    assert!(!t.is_trusted(), "{t:?}");
    let sha384 = authority(&pss_alg(Some(&pss_params(
        Some(Hash::Sha384),
        Some(Hash::Sha384),
        Some(48),
        None,
    ))));
    let t = trust(&signed_under(&sha384, true), &ca());
    assert!(!t.is_trusted(), "{t:?}");

    // Trusted itself, the authority's limit is named.
    let mut direct = TrustList::default();
    direct.add(&any_pss, TrustRole::Authority, None).unwrap();
    assert!(trust(&signed_under(&any_pss, true), &direct).is_trusted());
    failed(
        &trust(&signed_under(&any_pss, false), &direct),
        "allows its key only for RSA-PSS signatures",
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

// ------------------------------------------------------------------ what a certificate is for

const SERVER_AUTH: &str = "1.3.6.1.5.5.7.3.1";
const EMAIL_PROTECTION: &str = "1.3.6.1.5.5.7.3.4";
const TIME_STAMPING: &str = "1.3.6.1.5.5.7.3.8";
const DOCUMENT_SIGNING: &str = "1.3.6.1.5.5.7.3.36";
const ANY_EKU: &str = "2.5.29.37.0";

/// Key usages (DER bit strings): digital signature; non-repudiation; key encipherment;
/// digital signature and key encipherment.
const KU_DS: &[u8] = &[0x03, 0x02, 0x07, 0x80];
const KU_NR: &[u8] = &[0x03, 0x02, 0x06, 0x40];
const KU_KE: &[u8] = &[0x03, 0x02, 0x05, 0x20];
const KU_DS_KE: &[u8] = &[0x03, 0x02, 0x05, 0xa0];

fn key_usage(bits: &[u8]) -> Vec<u8> {
    ext("2.5.29.15", true, bits)
}

/// An extended key usage extension listing `uses`.
fn eku(critical: bool, uses: &[&str]) -> Vec<u8> {
    let oids: Vec<Vec<u8>> = uses.iter().map(|u| oid(u)).collect();
    ext(
        "2.5.29.37",
        critical,
        &seq(&oids.iter().map(Vec::as_slice).collect::<Vec<_>>()),
    )
}

/// What RFC 3161 asks of a timestamp authority's certificate.
fn tsa_profile() -> Vec<Vec<u8>> {
    vec![key_usage(KU_DS), eku(true, &[TIME_STAMPING])]
}

/// An intermediate authority for the rsa3072 key named `cn`, issued by the test CA (`ca`),
/// with `more` extensions besides basic constraints.
fn intermediate(cn: &str, more: Vec<Vec<u8>>) -> Vec<u8> {
    let mut extensions = vec![
        ext("2.5.29.19", true, &seq(&[&[0x01, 0x01, 0xff]])),
        ext_authority_key_id(&cert_der("ca")),
    ];
    extensions.extend(more);
    make_cert(
        &id_with_cert("ca", "ca"),
        &CertSpec {
            key_of: &cert_der("rsa3072"),
            cn,
            not_before: "260101000000Z",
            not_after: "360101000000Z",
            extensions,
            inner_alg: None,
        },
    )
}

/// A certificate for the `key` key named `cn`, issued by `issuer` (a certificate for the
/// rsa3072 key), with `extensions`.
fn issued_under(issuer: &[u8], key: &str, cn: &str, extensions: Vec<Vec<u8>>) -> Vec<u8> {
    make_cert(
        &Id {
            key: id("rsa3072").key,
            cert: issuer.to_vec(),
        },
        &CertSpec {
            key_of: &cert_der(key),
            cn,
            not_before: "260101000000Z",
            not_after: "360101000000Z",
            extensions,
            inner_alg: None,
        },
    )
}

/// A PDF signed with the p256 key and the certificate `leaf`, carrying `chain`.
fn signed_carrying(leaf: Vec<u8>, chain: Vec<Vec<u8>>) -> Vec<u8> {
    sign(
        &base_pdf(),
        &Id {
            key: id("p256").key,
            cert: leaf,
        },
        &Field::default(),
        &Cms {
            extra_certs: chain,
            ..Cms::default()
        },
    )
}

/// A timestamp authority with the rsa2048 key and the certificate `cert`.
fn stamper(cert: Vec<u8>, carrying: Vec<Vec<u8>>) -> Stamper {
    Stamper {
        key: "rsa2048",
        cert,
        carrying,
    }
}

/// A timestamp authority's certificate for the rsa2048 key, issued by the test CA (`ca`),
/// with `extensions`.
fn tsa_cert(extensions: Vec<Vec<u8>>) -> Vec<u8> {
    make_cert(
        &id_with_cert("ca", "ca"),
        &CertSpec {
            key_of: &cert_der("rsa2048"),
            cn: "Crafted Timestamp Authority",
            not_before: "260101000000Z",
            not_after: "360101000000Z",
            extensions,
            inner_alg: None,
        },
    )
}

fn doc_timestamp_field() -> Field {
    Field {
        name: "Timestamp1",
        sub_filter: "ETSI.RFC3161",
        doc_timestamp: true,
        invisible: true,
        m: None,
        reason: None,
        ..Field::default()
    }
}

/// The two ways a timestamp from `by` reaches a document: a document timestamp at 13:00,
/// and one embedded at 12:05 in a signature by the rsa3072 self-made ID.
fn stamped_both_ways(by: Stamper) -> [Vec<u8>; 2] {
    let document = sign_with(&base_pdf(), &doc_timestamp_field(), |r| {
        timestamp_token_carrying(
            &by.id(),
            r,
            "20260601130000Z",
            Hash::Sha256,
            by.carrying.clone(),
        )
    });
    let embedded = sign(
        &base_pdf(),
        &id("rsa3072"),
        &Field::default(),
        &Cms {
            timestamp: Some(("20260601120500Z", false)),
            timestamp_by: Some(by),
            ..Cms::default()
        },
    );
    [document, embedded]
}

/// The verdict on the timestamp authority of the one signature in `pdf` (a document
/// timestamp, or a signature with an embedded timestamp), and the signature's warnings.
fn stamp_verdict(pdf: &[u8], list: &TrustList) -> (Trust, Vec<SigWarning>) {
    let r = report(pdf, list);
    assert_eq!(r.signatures.len(), 1);
    let s = &r.signatures[0];
    assert_eq!(s.status, SigStatus::Valid, "{s:#?}");
    let t = if s.kind == SigKind::DocumentTimestamp {
        s.trust.clone()
    } else {
        s.timestamp
            .as_ref()
            .expect("an embedded timestamp")
            .trust
            .clone()
    };
    (t.expect("judged"), s.warnings.clone())
}

#[test]
fn issuers_limit_what_they_may_issue_for() {
    let signer = |inter: &[u8], extensions: Vec<Vec<u8>>| {
        signed_carrying(
            issued_under(inter, "p256", "Crafted Signer", extensions),
            vec![inter.to_vec()],
        )
    };
    let signing = || vec![ext_signing_key_usage()];
    let under_root = "it was issued under \"Test Root CA\", which you trust, but ";

    // An intermediate for websites only vouches neither for a signer nor for a timestamp
    // authority, as a document timestamp or embedded.
    let web = intermediate("Website Intermediate", vec![eku(false, &[SERVER_AUTH])]);
    failed(
        &trust(&signer(&web, signing()), &ca()),
        &format!(
            "{under_root}the certificate of \"Website Intermediate\" doesn't allow it to issue \
             certificates for signing documents"
        ),
    );
    let authority = issued_under(
        &web,
        "rsa2048",
        "Crafted Timestamp Authority",
        tsa_profile(),
    );
    for pdf in stamped_both_ways(stamper(authority, vec![web.clone()])) {
        failed(
            &stamp_verdict(&pdf, &ca()).0,
            &format!(
                "{under_root}the certificate of \"Website Intermediate\" doesn't allow it to \
                 issue certificates for timestamps"
            ),
        );
    }

    // An intermediate for email protection allows signing, but each certificate keeps only
    // the uses the one above it allows: not a signer meant for document signing alone.
    let email = intermediate("Email Intermediate", vec![eku(false, &[EMAIL_PROTECTION])]);
    let document_signing = || vec![ext_signing_key_usage(), eku(false, &[DOCUMENT_SIGNING])];
    failed(
        &trust(&signer(&email, document_signing()), &ca()),
        &format!(
            "{under_root}the certificate of \"Email Intermediate\" doesn't allow it to issue \
             certificates for the uses the signer's certificate is meant for"
        ),
    );
    assert!(trust(&signer(&email, signing()), &ca()).is_trusted());
    let both = vec![
        ext_signing_key_usage(),
        eku(false, &[DOCUMENT_SIGNING, EMAIL_PROTECTION]),
    ];
    assert!(trust(&signer(&email, both), &ca()).is_trusted());

    // A trusted root that is itself for websites only (the ca key, self-signed).
    let named = make_cert(
        &id_with_cert("ca", "ca"),
        &CertSpec {
            key_of: &cert_der("ca"),
            cn: "Website Root",
            not_before: "260101000000Z",
            not_after: "360101000000Z",
            extensions: Vec::new(),
            inner_alg: None,
        },
    );
    let web_root = make_cert(
        &Id {
            key: id("ca").key,
            cert: named,
        },
        &CertSpec {
            key_of: &cert_der("ca"),
            cn: "Website Root",
            not_before: "260101000000Z",
            not_after: "360101000000Z",
            extensions: vec![
                ext("2.5.29.19", true, &seq(&[&[0x01, 0x01, 0xff]])),
                eku(false, &[SERVER_AUTH]),
            ],
            inner_alg: None,
        },
    );
    let leaf = make_cert(
        &Id {
            key: id("ca").key,
            cert: web_root.clone(),
        },
        &CertSpec {
            key_of: &cert_der("p256"),
            cn: "Crafted Signer",
            not_before: "260101000000Z",
            not_after: "360101000000Z",
            extensions: signing(),
            inner_alg: None,
        },
    );
    let mut list = TrustList::default();
    list.add(&web_root, TrustRole::Authority, None).unwrap();
    failed(
        &trust(&signed_carrying(leaf, Vec::new()), &list),
        "it was issued under \"Website Root\", which you trust, but the certificate of \
         \"Website Root\" doesn't allow it to issue certificates for signing documents",
    );

    // Two intermediates of the same name and key, the one for websites first: the signer is
    // trusted through the other.
    let open = intermediate("Website Intermediate", Vec::new());
    let leaf = issued_under(&web, "p256", "Crafted Signer", signing());
    match trust(
        &signed_carrying(leaf, vec![web.clone(), open.clone()]),
        &ca(),
    ) {
        Trust::Authority { chain, .. } => assert!(chain[1].der == open, "through the first"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn issuers_that_allow_the_use_vouch() {
    let document_signing = || vec![ext_signing_key_usage(), eku(false, &[DOCUMENT_SIGNING])];
    // Intermediates with no extended key usage or any use, for a signer meant for document
    // signing alone, and one for timestamps for a timestamp authority.
    for (cn, more) in [
        ("Open Intermediate", Vec::new()),
        ("Any Use Intermediate", vec![eku(false, &[ANY_EKU])]),
    ] {
        let inter = intermediate(cn, more);
        let leaf = issued_under(&inter, "p256", "Crafted Signer", document_signing());
        let t = trust(&signed_carrying(leaf, vec![inter]), &ca());
        assert!(t.is_trusted(), "{cn}: {t:?}");
    }
    let stamps = intermediate(
        "Timestamp Only Intermediate",
        vec![eku(false, &[TIME_STAMPING])],
    );
    let authority = issued_under(
        &stamps,
        "rsa2048",
        "Crafted Timestamp Authority",
        tsa_profile(),
    );
    for pdf in stamped_both_ways(stamper(authority, vec![stamps.clone()])) {
        let (t, warnings) = stamp_verdict(&pdf, &ca());
        assert!(t.is_trusted(), "{t:?}");
        assert!(warnings.is_empty(), "{warnings:?}");
    }
    let (tsa, inter) = tsa_under_intermediate();
    for pdf in stamped_both_ways(stamper(tsa.cert, vec![inter])) {
        let (t, _) = stamp_verdict(&pdf, &ca());
        assert!(t.is_trusted(), "{t:?}");
    }
}

#[test]
fn timestamp_authorities_need_the_rfc_3161_profile() {
    let ts = || eku(true, &[TIME_STAMPING]);
    let cases = [
        (
            "key encipherment alone",
            vec![key_usage(KU_KE), ts()],
            "the timestamp authority's certificate doesn't allow its key to sign",
        ),
        (
            "digital signature and key encipherment",
            vec![key_usage(KU_DS_KE), ts()],
            "the timestamp authority's key may also be used for other things, but RFC 3161 \
             requires a key kept for timestamps",
        ),
        (
            "extended key usage not critical",
            vec![key_usage(KU_DS), eku(false, &[TIME_STAMPING])],
            "the timestamp authority's certificate doesn't insist on being used only for \
             timestamps, as RFC 3161 requires",
        ),
        (
            "timestamps and email",
            vec![
                key_usage(KU_DS),
                eku(true, &[TIME_STAMPING, EMAIL_PROTECTION]),
            ],
            "the timestamp authority's certificate is also meant for other uses, but RFC 3161 \
             requires it to be for timestamps only",
        ),
        (
            "any use alone",
            vec![key_usage(KU_DS), eku(true, &[ANY_EKU])],
            "the timestamp authority's certificate isn't meant for timestamps",
        ),
        (
            "no extended key usage",
            vec![key_usage(KU_DS)],
            "the timestamp authority's certificate isn't meant for timestamps",
        ),
    ];
    for (what, extensions, why) in cases {
        let cert = tsa_cert(extensions);
        let mut itself = TrustList::default();
        itself.add(&cert, TrustRole::Signer, None).unwrap();
        for list in [itself, ca()] {
            for pdf in stamped_both_ways(stamper(cert.clone(), Vec::new())) {
                let (t, warnings) = stamp_verdict(&pdf, &list);
                assert!(
                    matches!(&t, Trust::Failed(w) if w.contains(why)),
                    "{what}: {t:?}"
                );
                assert_eq!(
                    warnings,
                    [SigWarning::NotATimestampAuthority(why.into())],
                    "{what}"
                );
            }
        }
    }
}

#[test]
fn timestamp_authorities_with_the_profile_are_trusted() {
    let ts = || eku(true, &[TIME_STAMPING]);
    // Trusted without a warning: the test authority as itself, and the profile with
    // non-repudiation alone or no key usage, as itself and under the test CA.
    let mut controls = vec![(
        "the test authority",
        Stamper {
            key: "rsa2048",
            cert: cert_der("tsa"),
            carrying: Vec::new(),
        },
        false,
    )];
    for (what, extensions) in [
        ("non-repudiation", vec![key_usage(KU_NR), ts()]),
        ("no key usage", vec![ts()]),
    ] {
        controls.push((what, stamper(tsa_cert(extensions), Vec::new()), true));
    }
    for (what, by, under_ca) in controls {
        let mut itself = TrustList::default();
        itself.add(&by.cert, TrustRole::Signer, None).unwrap();
        let lists = if under_ca {
            vec![itself, ca()]
        } else {
            vec![itself]
        };
        let pdfs = stamped_both_ways(by);
        for list in &lists {
            for pdf in &pdfs {
                let (t, warnings) = stamp_verdict(pdf, list);
                assert!(t.is_trusted(), "{what}: {t:?}");
                assert!(warnings.is_empty(), "{what}: {warnings:?}");
            }
        }
    }
}

#[test]
fn authorities_come_from_the_signature_and_its_timestamp() {
    // PB-116: a signature carrying the intermediate, with a timestamp from an authority
    // under the test CA whose token carries the CA. The authority's own certificate isn't
    // a certificate authority, and one carried by both is listed once.
    let names = |extra_certs: Vec<Vec<u8>>, stamped: bool| {
        let by = stamper(
            tsa_cert(vec![eku(true, &[TIME_STAMPING])]),
            vec![cert_der("ca")],
        );
        let pdf = sign(
            &base_pdf(),
            &id("t-leaf"),
            &Field::default(),
            &Cms {
                extra_certs,
                timestamp: stamped.then_some(("20260601120500Z", false)),
                timestamp_by: stamped.then_some(by),
                ..Cms::default()
            },
        );
        let r = report(&pdf, &TrustList::default());
        let s = &r.signatures[0];
        assert_eq!(s.status, SigStatus::Valid, "{s:#?}");
        assert_eq!(s.timestamp.is_some(), stamped);
        s.authorities()
            .iter()
            .map(|c| c.display_name().to_string())
            .collect::<Vec<_>>()
    };
    let inter = || cert_der("t-inter");
    assert_eq!(names(vec![inter()], false), ["Trust Test Intermediate"]);
    let both = ["Trust Test Intermediate", "Test Root CA"];
    assert_eq!(names(vec![inter()], true), both);
    assert_eq!(names(vec![inter(), cert_der("ca")], true), both);
    assert_eq!(names(Vec::new(), true), ["Test Root CA"]);
}

// ------------------------------------------------------------------ which time counts

/// Signed with the p256 key and a certificate from the test CA valid `from` `to`, at
/// `signer_time` by the signer's clock (or with no time of its own), with a timestamp from
/// the test authority (`tsa`) at 12:05.
fn stamped_signature(signer_time: Option<&'static str>, from: &str, to: &str) -> Vec<u8> {
    let leaf = make_cert(
        &id_with_cert("ca", "ca"),
        &CertSpec {
            key_of: &cert_der("p256"),
            cn: "Short-Lived Signer",
            not_before: from,
            not_after: to,
            extensions: vec![ext_signing_key_usage()],
            inner_alg: None,
        },
    );
    sign(
        &base_pdf(),
        &Id {
            key: id("p256").key,
            cert: leaf,
        },
        &Field {
            m: None,
            ..Field::default()
        },
        &Cms {
            signing_time: signer_time,
            timestamp: Some(("20260601120500Z", false)),
            ..Cms::default()
        },
    )
}

#[test]
fn only_a_trusted_timestamp_sets_the_signing_time() {
    // The signer's certificate ends at 12:03; the signer says 12:02, the timestamp 12:05.
    let pdf = stamped_signature(Some("260601120200Z"), "260101000000Z", "260601120300Z");
    let with_tsa = trusting(&[("ca", TrustRole::Authority), ("tsa", TrustRole::Signer)]);
    let by_the_signer = |r: &SignatureReport| {
        let s = &r.signatures[0];
        let t = s.time.unwrap();
        assert_eq!(
            (t.time.to_rfc3339(), t.source),
            ("2026-06-01T12:02:00+00:00".into(), TimeSource::SignerClock)
        );
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        unknown(
            s.timestamp.as_ref().unwrap().trust.as_ref().unwrap(),
            "self-made",
        );
        assert!(s.trust.as_ref().unwrap().is_trusted(), "{:?}", s.trust);
        assert!(!s.fully_trusted());
        assert!(!r.all_trusted());
    };
    let by_the_timestamp = |r: &SignatureReport| {
        let s = &r.signatures[0];
        let t = s.time.unwrap();
        assert_eq!(
            (t.time.to_rfc3339(), t.source),
            ("2026-06-01T12:05:00+00:00".into(), TimeSource::Timestamp)
        );
        assert_eq!(
            s.signer_time.map(|t| t.source),
            Some(TimeSource::SignerClock)
        );
        assert_eq!(s.warnings, [SigWarning::CertificateNotValidAtSigning]);
        assert!(s
            .timestamp
            .as_ref()
            .unwrap()
            .trust
            .as_ref()
            .unwrap()
            .is_trusted());
        failed(
            s.trust.as_ref().unwrap(),
            "wasn't valid at the signing time",
        );
        assert!(!r.all_trusted());
    };
    let mut r = report(&pdf, &ca());
    by_the_signer(&r);
    r.apply_trust(&with_tsa);
    by_the_timestamp(&r);
    r.apply_trust(&ca());
    by_the_signer(&r);
    by_the_timestamp(&report(&pdf, &with_tsa));

    // With no time from the signer, an untrusted timestamp leaves none, and the chain is
    // checked today. This certificate starts at 12:06.
    let pdf = stamped_signature(None, "260601120600Z", "360101000000Z");
    let r = report(&pdf, &ca());
    let s = &r.signatures[0];
    assert_eq!(s.time, None);
    assert!(s.trust.as_ref().unwrap().is_trusted(), "{:?}", s.trust);
    let r = report(&pdf, &with_tsa);
    let s = &r.signatures[0];
    assert_eq!(s.time.unwrap().source, TimeSource::Timestamp);
    assert_eq!(s.warnings, [SigWarning::CertificateNotValidAtSigning]);
    failed(
        s.trust.as_ref().unwrap(),
        "wasn't valid at the signing time",
    );

    // The signer and the timestamp authority both trusted.
    let pdf = stamped_signature(Some("260601120200Z"), "260101000000Z", "360101000000Z");
    let r = report(&pdf, &with_tsa);
    assert!(r.signatures[0].fully_trusted());
    assert!(r.all_trusted());

    // A document timestamp has no other time: from an authority not trusted, it's a claim.
    let stamped = sign_with(&base_pdf(), &doc_timestamp_field(), |r| {
        timestamp_token(&tsa(), r, "20260601130000Z", Hash::Sha256)
    });
    let mut r = report(&stamped, &TrustList::default());
    let t = r.signatures[0].time.unwrap();
    assert_eq!(
        (t.time.to_rfc3339(), t.source),
        (
            "2026-06-01T13:00:00+00:00".into(),
            TimeSource::UnconfirmedTimestamp
        )
    );
    r.apply_trust(&trusting(&[("tsa", TrustRole::Signer)]));
    assert_eq!(r.signatures[0].time.unwrap().source, TimeSource::Timestamp);
    assert!(r.all_trusted());
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

#[test]
fn file_round_trip() {
    let dir = tool::Scratch::new("trust-round-trip");
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
}

#[cfg(unix)]
#[test]
fn file_others_can_change_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tool::Scratch::new("trust-perms");
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
    let dir = tool::Scratch::new("trust-openssl");
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
        let ok = tool::output(cmd.arg(&leaf_pem))
            .expect("openssl installed")
            .status
            .success();
        let limarepdf = trust(&signed_by(leaf, chain), &root()).is_trusted();
        assert_eq!(limarepdf, ok, "{leaf}: LimarePDF and OpenSSL disagree");
    }

    // Hand-made certificates under the test CA, judged by `-purpose` (left out: issuers above
    // a timestamp authority, whose uses OpenSSL doesn't check, and any-use or code-signing
    // intermediates, which `smimesign` refuses and LimarePDF allows).
    let write = |der: &[u8], file: &str| {
        let p = dir.join(file);
        let pem = der::pem::encode_string("CERTIFICATE", der::pem::LineEnding::LF, der).unwrap();
        std::fs::write(&p, pem).unwrap();
        p
    };
    let ca_pem = pem_of(&["ca"], "ca.pem");
    let openssl_ok = |purpose: &str, leaf: &[u8], chain: &[Vec<u8>]| {
        let mut cmd = std::process::Command::new("openssl");
        cmd.args([
            "verify",
            "-attime",
            "1780315200",
            "-purpose",
            purpose,
            "-CAfile",
        ])
        .arg(&ca_pem);
        for (i, c) in chain.iter().enumerate() {
            cmd.arg("-untrusted")
                .arg(write(c, &format!("chain{i}.pem")));
        }
        tool::output(cmd.arg(write(leaf, "leaf.pem")))
            .expect("openssl installed")
            .status
            .success()
    };
    let web = intermediate("Website Intermediate", vec![eku(false, &[SERVER_AUTH])]);
    let open = intermediate("Open Intermediate", Vec::new());
    for inter in [web, open] {
        let leaf = issued_under(
            &inter,
            "p256",
            "Crafted Signer",
            vec![ext_signing_key_usage()],
        );
        let ok = openssl_ok("smimesign", &leaf, std::slice::from_ref(&inter));
        let limarepdf = trust(&signed_carrying(leaf, vec![inter]), &ca()).is_trusted();
        assert_eq!(
            limarepdf, ok,
            "an intermediate: LimarePDF and OpenSSL disagree"
        );
    }
    let ts = || eku(true, &[TIME_STAMPING]);
    let profiles: [(&str, Vec<Vec<u8>>); 9] = [
        ("the profile", tsa_profile()),
        ("non-repudiation", vec![key_usage(KU_NR), ts()]),
        ("no key usage", vec![ts()]),
        (
            "not critical",
            vec![key_usage(KU_DS), eku(false, &[TIME_STAMPING])],
        ),
        (
            "and email",
            vec![
                key_usage(KU_DS),
                eku(true, &[TIME_STAMPING, EMAIL_PROTECTION]),
            ],
        ),
        ("any use", vec![key_usage(KU_DS), eku(true, &[ANY_EKU])]),
        ("key encipherment", vec![key_usage(KU_KE), ts()]),
        ("and key encipherment", vec![key_usage(KU_DS_KE), ts()]),
        ("no extended key usage", vec![key_usage(KU_DS)]),
    ];
    for (what, extensions) in profiles {
        let cert = tsa_cert(extensions);
        let ok = openssl_ok("timestampsign", &cert, &[]);
        let [document, _] = stamped_both_ways(stamper(cert, Vec::new()));
        let limarepdf = stamp_verdict(&document, &ca()).0.is_trusted();
        assert_eq!(limarepdf, ok, "{what}: LimarePDF and OpenSSL disagree");
    }
}
