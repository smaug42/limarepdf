//! Digital IDs: making self-signed IDs, importing PKCS#12 files other apps write
//! (OpenSSL-made, in the shapes OpenSSL, Windows and older tools use:
//! `sh tools/make-signature-keys.sh ids`), exporting, and the ID folder. The test files'
//! password is "mpe-test-password" unless noted.

mod sigkit;

use limarepdf_core::{
    password_problem, verify_signatures, DigitalId, IdStore, NewId, SigStatus, Trust, TrustList,
    TrustRole, WrongPassword,
};
use sigkit::*;
use std::path::PathBuf;

const PASSWORD: &str = "mpe-test-password";

fn p12(name: &str) -> Vec<u8> {
    std::fs::read(data_dir().join("ids").join(format!("{name}.p12"))).unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("limarepdf-ids-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn store(name: &str) -> IdStore {
    IdStore::new(scratch(name).join("ids"))
}

fn sha256_of(der: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[track_caller]
fn wrong_password(r: limarepdf_core::Result<impl std::fmt::Debug>) {
    let e = r.expect_err("must be refused");
    assert!(e.downcast_ref::<WrongPassword>().is_some(), "{e:#}");
}

#[track_caller]
fn refused(r: limarepdf_core::Result<impl std::fmt::Debug>, part: &str) {
    let e = format!("{:#}", r.expect_err("must be refused"));
    assert!(e.contains(part), "{e:?} should say {part:?}");
}

fn jane() -> NewId {
    NewId {
        name: "Jane Doe".into(),
        organisation: Some("Example Ltd".into()),
        email: Some("jane@example.com".into()),
    }
}

// ------------------------------------------------------------------ importing

/// Imports test file `file` and checks the ID: the certificate `cert` (in keys/), the key,
/// how many other certificates came with it, and that it opens again with the password.
#[track_caller]
fn imports(file: &str, password: &str, cert: &str, key: &str, chain: usize) -> DigitalId {
    let s = store(&format!("import-{file}"));
    let id = s.import(&p12(file), password, None).unwrap();
    assert_eq!(id.cert.sha256, sha256_of(&cert_der(cert)), "{file}");
    assert_eq!(id.cert.key, key, "{file}");
    assert_eq!(id.chain.len(), chain, "{file}");
    assert!(id.default);
    let list = s.list();
    assert_eq!(list.ids, std::slice::from_ref(&id), "{file}");
    assert!(list.problems.is_empty(), "{:?}", list.problems);
    // Kept under the same password, in LimarePDF's own encryption.
    let u = s.unlock(&id, password).unwrap();
    assert_eq!(u.cert, id.cert);
    assert_eq!(u.key_description(), key);
    id
}

#[test]
fn imports_openssl_3_default() {
    let id = imports("modern", PASSWORD, "rsa3072", "RSA 3072-bit", 0);
    assert_eq!(id.name, "test-signer-rsa-3072");
}

#[test]
fn imports_legacy_rc2_and_3des() {
    imports("legacy", PASSWORD, "rsa2048", "RSA 2048-bit", 0);
}

#[test]
fn imports_windows_triple_des() {
    imports("3des", PASSWORD, "rsa2048", "RSA 2048-bit", 0);
}

#[test]
fn imports_other_pbes2_ciphers() {
    imports("aes128", PASSWORD, "rsa2048", "RSA 2048-bit", 0);
    imports("pbes2-3des", PASSWORD, "rsa2048", "RSA 2048-bit", 0);
}

#[test]
fn imports_other_integrity_checks() {
    imports("sha512-mac", PASSWORD, "rsa2048", "RSA 2048-bit", 0);
    imports("pbmac1", PASSWORD, "rsa2048", "RSA 2048-bit", 0);
    imports("pbmac1-sha512", PASSWORD, "rsa2048", "RSA 2048-bit", 0);
}

#[test]
fn imports_unencrypted_parts() {
    imports("plain-certs", PASSWORD, "rsa2048", "RSA 2048-bit", 0);
    imports("plain-key", PASSWORD, "rsa2048", "RSA 2048-bit", 0);
}

#[test]
fn imports_ec_keys() {
    imports("p256", PASSWORD, "p256", "ECDSA P-256", 0);
    imports("p384", PASSWORD, "p384", "ECDSA P-384", 0);
}

#[test]
fn imports_with_non_ascii_and_empty_passwords() {
    let rsa = |file, password| imports(file, password, "rsa2048", "RSA 2048-bit", 0);
    rsa("non-ascii", "pässwörd-тест");
    rsa("non-ascii-legacy", "pässwörd-тест");
    rsa("empty-password", "");
}

#[test]
fn imports_an_id_with_its_authority() {
    let id = imports("chain", PASSWORD, "issued", "ECDSA P-256", 1);
    assert_eq!(id.chain[0].sha256, sha256_of(&cert_der("ca")));
    assert!(!id.cert.self_issued);
}

#[test]
fn imports_expired_ids_but_marks_them() {
    let id = imports("expired", PASSWORD, "expired", "RSA 2048-bit", 0);
    assert!(!id.valid_now());
}

#[test]
fn import_takes_the_name_given() {
    let s = store("import-named");
    let id = s.import(&p12("p256"), PASSWORD, Some("Work ID")).unwrap();
    assert_eq!(id.name, "work-id");
    assert_eq!(id.cert.display_name(), "Test Signer P-256");
    // The same ID twice is refused.
    refused(
        s.import(&p12("p256"), PASSWORD, None),
        "already in LimarePDF as \"Test Signer P-256\" (work-id)",
    );
}

#[test]
fn wrong_passwords_are_refused() {
    let s = store("wrong-password");
    wrong_password(s.import(&p12("modern"), "mpe-test-passwore", None));
    wrong_password(s.import(&p12("legacy"), "", None));
    wrong_password(s.import(&p12("pbmac1"), "MPE-TEST-PASSWORD", None));
    wrong_password(s.import(&p12("empty-password"), " ", None));
    assert!(s.list().ids.is_empty());
    assert!(!s.dir().exists(), "nothing is written for a refused file");
}

#[test]
fn files_limarepdf_cant_sign_with_are_refused() {
    let s = store("refused");
    let r = |f: &str| s.import(&p12(f), PASSWORD, None);
    refused(r("no-mac"), "no integrity check");
    refused(r("rsa1024"), "1024-bit RSA key");
    refused(r("rsa6144"), "6144-bit RSA key");
    refused(r("key-encipherment"), "doesn't allow its key to sign");
    refused(r("p521"), "elliptic curve LimarePDF can't sign with");
    refused(r("no-key"), "holds no private key");
    refused(r("wrong-cert"), "no certificate for its private key");
    refused(
        s.import(b"%PDF-1.7 not an ID", PASSWORD, None),
        "isn't a digital ID",
    );
    refused(s.import(&[], PASSWORD, None), "isn't a digital ID");
    assert!(!s.dir().exists(), "nothing is written for a refused file");
}

#[test]
fn damaged_files_never_panic() {
    // Every truncation, and a byte changed at every position, of a legacy and a modern
    // file: refused (or, where the change is outside what the integrity check covers and
    // harmless, imported), never a panic.
    let s = store("damaged");
    for name in ["legacy", "p256"] {
        let file = p12(name);
        for n in (0..file.len()).step_by(7) {
            assert!(s.import(&file[..n], PASSWORD, None).is_err());
        }
        for i in (0..file.len()).step_by(3) {
            let mut f = file.clone();
            f[i] ^= 0x41;
            if let Ok(id) = s.import(&f, PASSWORD, None) {
                // Only the wrapping may differ: the ID itself must be the same.
                let want = match name {
                    "legacy" => "rsa2048",
                    _ => "p256",
                };
                assert_eq!(id.cert.sha256, sha256_of(&cert_der(want)), "byte {i}");
                s.remove(&id).unwrap();
            }
        }
    }
}

// ------------------------------------------------------------------ making

#[test]
fn creates_self_signed_ids() {
    let s = store("create");
    let id = s.create(&jane(), "correct horse").unwrap();
    assert_eq!(id.name, "jane-doe");
    assert_eq!(id.path, s.dir().join("jane-doe.p12"));
    let c = &id.cert;
    let subject = "CN=Jane Doe,O=Example Ltd,EMAIL=jane@example.com";
    assert_eq!(c.subject, subject);
    assert_eq!(c.common_name.as_deref(), Some("Jane Doe"));
    assert_eq!(c.organisation.as_deref(), Some("Example Ltd"));
    assert_eq!(c.email.as_deref(), Some("jane@example.com"));
    assert!(c.self_issued);
    assert_eq!(c.key, "RSA 3072-bit");
    let days = (c.not_after - c.not_before).num_days();
    assert!((1826..=1827).contains(&days), "{days} days");
    assert!(id.valid_now());
    assert!(id.chain.is_empty());
    // LimarePDF's own checks accept it as a signer once trusted.
    let mut trust = TrustList::default();
    trust.add(&c.der, TrustRole::Signer, None).unwrap();
    // Listed, as the default, without the password.
    let list = s.list();
    assert_eq!(list.ids, std::slice::from_ref(&id));
    // Opens with the password only.
    assert_eq!(s.unlock(&id, "correct horse").unwrap().cert, id.cert);
    wrong_password(s.unlock(&id, "correct horsf"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&id.path), 0o600);
        assert_eq!(mode(s.dir()), 0o700);
    }
}

#[test]
fn new_ids_need_a_good_password_and_name() {
    let s = store("create-refused");
    refused(s.create(&jane(), "short"), "at least 8 characters");
    refused(
        s.create(
            &NewId {
                name: "  ".into(),
                ..NewId::default()
            },
            "correct horse",
        ),
        "needs a name",
    );
    refused(
        s.create(
            &NewId {
                email: Some("not an address".into()),
                ..jane()
            },
            "correct horse",
        ),
        "isn't an email address",
    );
    assert!(password_problem("12345678").is_none());
    assert!(!s.dir().exists());
}

#[test]
fn several_ids_and_the_default() {
    let s = store("several");
    let first = s.create(&jane(), "correct horse").unwrap();
    let second = s.create(&jane(), "battery staple").unwrap();
    assert_eq!(second.name, "jane-doe-2");
    let other = s.import(&p12("p256"), PASSWORD, None).unwrap();
    let list = s.list();
    let names: Vec<_> = list.ids.iter().map(|i| (&*i.name, i.default)).collect();
    assert_eq!(
        names,
        [
            ("jane-doe", false),
            ("jane-doe-2", false),
            ("test-signer-p-256", true)
        ]
    );
    // Finding: by number, name, certificate name or fingerprint.
    assert_eq!(s.find("1").unwrap(), list.ids[0]);
    assert_eq!(s.find("JANE-DOE-2").unwrap().cert, second.cert);
    assert_eq!(s.find("test signer p-256").unwrap().cert, other.cert);
    let by_fingerprint = s.find(&first.cert.sha256.to_lowercase()).unwrap();
    assert_eq!(by_fingerprint.cert, first.cert);
    refused(s.find("Jane Doe"), "several digital IDs are called");
    refused(s.find("4"), "there is no digital ID 4; there are 3");
    refused(s.find("nobody"), "there is no digital ID \"nobody\"");
    // The default moves when chosen, and goes when its ID is removed.
    let first = s.find("1").unwrap();
    s.set_default(&first).unwrap();
    assert_eq!(s.default_id().unwrap().name, "jane-doe");
    s.remove(&first).unwrap();
    assert!(s.default_id().is_none());
    s.remove(&s.find("jane-doe-2").unwrap()).unwrap();
    // One ID left is the default.
    assert_eq!(s.default_id().unwrap().name, "test-signer-p-256");
}

#[test]
fn the_folder_lists_what_it_can() {
    let s = store("folder");
    assert!(s.list().ids.is_empty());
    let id = s.create(&jane(), "correct horse").unwrap();
    std::fs::write(s.dir().join("garbage.p12"), b"not a PKCS#12 file").unwrap();
    // A file OpenSSL made (certificates encrypted) can't be listed without its password.
    std::fs::write(s.dir().join("openssl.p12"), p12("modern")).unwrap();
    std::fs::write(s.dir().join("notes.txt"), b"ignored").unwrap();
    let list = s.list();
    assert_eq!(list.ids, std::slice::from_ref(&id));
    assert_eq!(list.problems.len(), 2, "{:?}", list.problems);
    assert!(list
        .problems
        .iter()
        .all(|p| p.contains("limarepdf-cli id import")));
    // Files with readable certificates put there by hand are listed, but one LimarePDF can't
    // sign with is refused when opened.
    std::fs::remove_file(s.dir().join("garbage.p12")).unwrap();
    std::fs::remove_file(s.dir().join("openssl.p12")).unwrap();
    std::fs::write(s.dir().join("by-hand.p12"), p12("plain-certs")).unwrap();
    std::fs::write(s.dir().join("keyenc.p12"), p12("key-encipherment-plain")).unwrap();
    let list = s.list();
    assert_eq!(list.ids.len(), 3);
    let by_hand = s.unlock(&s.find("by-hand").unwrap(), PASSWORD).unwrap();
    assert_eq!(by_hand.cert.sha256, sha256_of(&cert_der("rsa2048")));
    refused(
        s.unlock(&s.find("keyenc").unwrap(), PASSWORD),
        "LimarePDF can't sign with this ID",
    );
    std::fs::remove_file(s.dir().join("by-hand.p12")).unwrap();
    std::fs::remove_file(s.dir().join("keyenc.p12")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&id.path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let list = s.list();
        assert!(
            list.problems
                .iter()
                .any(|p| p.contains("can be read by other users")),
            "{:?}",
            list.problems
        );
    }
}

// ------------------------------------------------------------------ exporting

#[test]
fn export_and_import_elsewhere() {
    let s = store("export");
    let id = s.create(&jane(), "correct horse").unwrap();
    let u = s.unlock(&id, "correct horse").unwrap();
    refused(u.to_p12("short"), "at least 8 characters");
    let file = u.to_p12("new password").unwrap();
    // Certificates are encrypted in exported files, as other apps write them.
    let elsewhere = store("export-elsewhere");
    wrong_password(elsewhere.import(&file, "correct horse", None));
    let again = elsewhere.import(&file, "new password", None).unwrap();
    assert_eq!(again.cert, id.cert);
    assert_eq!(again.name, "jane-doe");
    // An imported ID with an authority exports with it.
    let s = store("export-chain");
    let id = s.import(&p12("chain"), PASSWORD, None).unwrap();
    let file = s.unlock(&id, PASSWORD).unwrap().to_p12(PASSWORD).unwrap();
    let back = store("export-chain-back")
        .import(&file, PASSWORD, None)
        .unwrap();
    assert_eq!(back.chain, id.chain);
}

// ------------------------------------------------------------------ cross-checks

/// LimarePDF's files read by OpenSSL, and an LimarePDF-made ID's key signing a PDF that LimarePDF then
/// checks and trusts: `cargo test -p limarepdf-core --test digital_ids -- --ignored`.
#[test]
#[ignore]
fn cross_check_with_openssl() {
    let dir = scratch("openssl");
    let s = IdStore::new(dir.join("ids"));
    let id = s.create(&jane(), "correct horse").unwrap();
    let unlocked = s.unlock(&id, "correct horse").unwrap();
    let exported = unlocked.to_p12("export pässword").unwrap();
    let out = dir.join("exported.p12");
    std::fs::write(&out, &exported).unwrap();
    let openssl = |args: &[&str]| {
        let o = std::process::Command::new("openssl")
            .args(args)
            .output()
            .expect("openssl");
        assert!(
            o.status.success(),
            "openssl {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr)
    };
    // Both the folder's file (certificates readable) and the exported one.
    for (file, pass) in [
        (id.path.to_str().unwrap(), "pass:correct horse"),
        (out.to_str().unwrap(), "pass:export pässword"),
    ] {
        let info = openssl(&["pkcs12", "-info", "-noout", "-in", file, "-passin", pass]);
        assert!(info.contains("MAC: sha256, Iteration 1200000"), "{info}");
        assert!(
            info.contains("PBES2, PBKDF2, AES-256-CBC, Iteration 600000, PRF hmacWithSHA256"),
            "{info}"
        );
        // OpenSSL takes the key and certificate out; they must be LimarePDF's.
        let path = |name: &str| dir.join(name).to_str().unwrap().to_string();
        let (pem, key, cert) = (path("out.pem"), path("key.der"), path("cert.der"));
        openssl(&[
            "pkcs12", "-in", file, "-passin", pass, "-nodes", "-out", &pem,
        ]);
        openssl(&[
            "pkcs8", "-topk8", "-nocrypt", "-in", &pem, "-outform", "DER", "-out", &key,
        ]);
        openssl(&["x509", "-in", &pem, "-outform", "DER", "-out", &cert]);
        assert_eq!(std::fs::read(&cert).unwrap(), id.cert.der);
        // That key signs a PDF LimarePDF checks as valid, and trusts once told to.
        let signer = Id {
            key: key_from_pkcs8(&std::fs::read(&key).unwrap()),
            cert: id.cert.der.clone(),
        };
        // No signing time anywhere, so the certificate is judged today: the ID is new.
        let pdf = sign(
            &base_pdf(),
            &signer,
            &Field {
                m: None,
                ..Field::default()
            },
            &Cms {
                signing_time: None,
                ..Cms::default()
            },
        );
        let mut trust = TrustList::default();
        let r = verify_signatures(&pdf, None, &trust).unwrap();
        assert_eq!(r.signatures[0].status, SigStatus::Valid);
        trust.add(&id.cert.der, TrustRole::Signer, None).unwrap();
        let r = verify_signatures(&pdf, None, &trust).unwrap();
        assert_eq!(
            r.signatures[0].trust,
            Some(Trust::Signer {
                label: "Jane Doe".into()
            })
        );
        // OpenSSL agrees the certificate is fit for signing.
        let text = openssl(&["x509", "-inform", "DER", "-in", &cert, "-text", "-noout"]);
        for part in [
            "Digital Signature, Non Repudiation",
            "E-mail Protection, 1.2.840.113583.1.1.5",
            "email:jane@example.com",
        ] {
            assert!(text.contains(part), "{text}");
        }
    }
}
