//! `limarepdf-cli verify --signed-version` never changes its input, whatever name the output
//! is given for it (audit R16); and `verify` uses an embedded timestamp's time only when its
//! authority is trusted.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SIGNED: &str =
    "../limarepdf-core/tests/data/signatures/pyhanko/minimal-two-fields-signed-twice.pdf";

/// A fresh folder holding a copy of the signed fixture as `input.pdf`, and an empty trust list.
fn setup(name: &str) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("verify")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("input.pdf");
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join(SIGNED), &input).unwrap();
    std::fs::write(dir.join("trust.txt"), "").unwrap();
    (dir, input)
}

/// `verify input.pdf --signed-version 1 -o output`, run in `dir`.
fn signed_version(dir: &Path, input: &Path, output: &Path) -> Output {
    limarepdf_core::child_seam::output(
        Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
            .current_dir(dir)
            .arg("verify")
            .arg(input)
            .args(["--trust-file", "trust.txt", "--signed-version", "1", "-o"])
            .arg(output),
    )
    .unwrap()
}

/// Fails, without printing whole files, unless `path` holds exactly `before`.
fn unchanged(path: &Path, before: &[u8]) {
    let now = std::fs::read(path).unwrap();
    assert!(
        now == before,
        "{} changed: {} bytes, was {}",
        path.display(),
        now.len(),
        before.len()
    );
}

fn refused(out: &Output) {
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("must be saved to another file"), "{err}");
}

/// Saved to a file of its own, the signed version is exactly the first bytes of the input, as
/// many as the report says.
#[test]
fn the_signed_version_is_saved_to_another_file() {
    let (dir, input) = setup("other");
    let before = std::fs::read(&input).unwrap();
    let output = dir.join("signed.pdf");
    let out = signed_version(&dir, &input, &output);
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let saved = std::fs::read(&output).unwrap();
    assert!(saved.len() < before.len());
    assert!(
        said.contains(&format!("({} of {} bytes)", saved.len(), before.len())),
        "{said}"
    );
    assert!(saved == before[..saved.len()]);
    unchanged(&input, &before);
}

/// The input's own name, given as it is or as another path to it, is refused with the input
/// unchanged.
#[test]
fn the_input_itself_is_refused() {
    let (dir, input) = setup("same");
    let before = std::fs::read(&input).unwrap();
    for output in [
        input.clone(),
        "input.pdf".into(),
        "../same/./input.pdf".into(),
    ] {
        refused(&signed_version(&dir, &input, &output));
        unchanged(&input, &before);
    }
}

/// A symbolic link to the input is refused with the input unchanged.
#[cfg(unix)]
#[test]
fn a_symbolic_link_to_the_input_is_refused() {
    let (dir, input) = setup("symlink");
    let before = std::fs::read(&input).unwrap();
    let link = dir.join("link.pdf");
    std::os::unix::fs::symlink(&input, &link).unwrap();
    refused(&signed_version(&dir, &input, &link));
    unchanged(&input, &before);
}

/// A hard link to the input leaves the input unchanged: on Unix it is refused, as the same
/// file; elsewhere the output replaces the link's name with a new file. Writing into it once
/// cut the input short.
#[test]
fn a_hard_link_to_the_input_leaves_it_unchanged() {
    let (dir, input) = setup("hardlink");
    let before = std::fs::read(&input).unwrap();
    let alias = dir.join("alias.pdf");
    std::fs::hard_link(&input, &alias).unwrap();
    let out = signed_version(&dir, &input, &alias);
    unchanged(&input, &before);
    if cfg!(unix) {
        refused(&out);
        unchanged(&alias, &before);
    }
}

/// An embedded timestamp counts only when its authority is trusted: trusting the signer
/// alone gives the signer's own time, and `--require-trusted` fails.
#[test]
fn an_untrusted_timestamp_is_neither_used_nor_trusted() {
    use limarepdf_core::{TrustList, TrustRole};
    let (dir, input) = setup("timestamps");
    let r = limarepdf_core::verify_signatures(
        &std::fs::read(&input).unwrap(),
        None,
        &TrustList::default(),
    )
    .unwrap();
    let mut list = TrustList::default();
    let signer = &r.signatures[0].signer.as_ref().unwrap().der;
    list.add(signer, TrustRole::Signer, None).unwrap();
    list.save(&dir.join("trust.txt")).unwrap();
    let verify = |json: bool| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"));
        cmd.current_dir(&dir).arg("verify").arg(&input).args([
            "--trust-file",
            "trust.txt",
            "--require-trusted",
        ]);
        if json {
            cmd.arg("--json");
        }
        let out = limarepdf_core::child_seam::output(&mut cmd).unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    let (code, text) = verify(false);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("(from the signer's computer)"), "{text}");
    assert!(!text.contains("(from a timestamp authority)"), "{text}");
    assert!(
        text.contains(", claimed by ")
            && text.contains("(LimarePDF can't confirm this authority, so this time isn't used)"),
        "{text}"
    );
    // The sample's signatures give their time only in the signature dictionary (`/M`).
    let (code, json) = verify(true);
    assert_eq!(code, Some(1), "{json}");
    assert!(
        json.contains("\"time_source\": \"signature_dictionary\""),
        "{json}"
    );
    assert!(!json.contains("\"time_source\": \"timestamp\""), "{json}");

    // The authorities trusted too: their time, and every signature trusted.
    for s in &r.signatures {
        let der = &s.timestamp.as_ref().expect("a timestamp").authority.der;
        if !list.entries.iter().any(|e| e.cert.der == *der) {
            list.add(der, TrustRole::Signer, None).unwrap();
        }
    }
    list.save(&dir.join("trust.txt")).unwrap();
    let (code, text) = verify(false);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("(from a timestamp authority)"), "{text}");
    assert!(!text.contains("(from the signer's computer)"), "{text}");
    assert!(text.contains(" (trusted)"), "{text}");
}

/// The digests each signature and its embedded timestamp rely on, in the text and the JSON
/// (the sample uses SHA-256 throughout).
#[test]
fn verify_names_the_digests() {
    let (dir, _) = setup("digests");
    let (code, text, err) = cli(&dir, "verify input.pdf");
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(
        text.matches(
            "\n   Algorithm:   RSA 4096-bit, RSA PKCS#1 v1.5, SHA-256, adbe.pkcs7.detached\n"
        )
        .count(),
        2,
        "{text}"
    );
    // Under each Timestamp row.
    assert_eq!(
        text.matches("\n                digest: SHA-256\n").count(),
        2,
        "{text}"
    );
    let (code, json, err) = cli(&dir, "verify input.pdf --json");
    assert_eq!(code, Some(0), "{err}");
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let sigs = v["signatures"].as_array().unwrap();
    assert_eq!(sigs.len(), 2, "{json}");
    for s in sigs {
        assert_eq!(
            (&s["digest"], &s["signature_digest"]),
            (&"SHA-256".into(), &"SHA-256".into()),
            "{s}"
        );
        let ts = &s["timestamp"];
        assert_eq!(
            (&ts["digest"], &ts["signature_digest"]),
            (&"SHA-256".into(), &"SHA-256".into()),
            "{ts}"
        );
    }
}

/// `limarepdf-cli ARGS --trust-file trust.txt`, run in `dir` (`args` split at spaces): the
/// exit code, what it printed and its errors.
fn cli(dir: &Path, args: &str) -> (Option<i32>, String, String) {
    let out = limarepdf_core::child_seam::output(
        Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
            .current_dir(dir)
            .args(args.split(' '))
            .args(["--trust-file", "trust.txt"]),
    )
    .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A committed sample signed file, copied into `dir` as `name`.
fn sample_in(dir: &Path, sample: &str, name: &str) {
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../limarepdf-core/tests/data/signatures")
        .join(sample);
    std::fs::copy(from, dir.join(name)).unwrap();
}

/// PB-116: `trust add --timestamp` trusts the authority of a signature's embedded timestamp,
/// and `verify` says how; with the signer trusted too, the signature is fully trusted, with
/// the timestamp's time.
#[test]
fn a_timestamp_authority_is_trusted_from_the_pdf() {
    let (dir, _) = setup("trust-timestamp");
    let (code, text, err) = cli(&dir, "verify input.pdf");
    assert_eq!(code, Some(0), "{err}");
    let hint = "this time isn't used); to trust it, run: limarepdf-cli trust add \"input.pdf\" \
                --signature 2 --timestamp (and compare its fingerprint with the authority's)";
    assert!(text.contains(hint), "{text}");

    let (code, _, err) = cli(&dir, "trust add input.pdf --signature 1");
    assert_eq!(code, Some(0), "{err}");
    let (code, said, err) = cli(&dir, "trust add input.pdf --signature 2 --timestamp");
    assert_eq!(code, Some(0), "{err}");
    for line in [
        "Now trusting \"Globalsign TSA for AATL on DSS - SHA384 - G5\" as a signer.",
        "Its timestamps are now trusted, and their times used as signing times.",
    ] {
        assert!(said.contains(line), "{said}");
    }
    // Both signatures' timestamps are from that authority.
    let (code, text, err) = cli(&dir, "verify input.pdf --require-trusted");
    assert_eq!(code, Some(0), "{text}{err}");
    assert!(text.contains("(from a timestamp authority)"), "{text}");
    assert!(!text.contains("(from the signer's computer)"), "{text}");
    assert!(!text.contains("--timestamp"), "{text}");
}

/// PB-116: `--timestamp` needs a signature with an embedded timestamp, in a PDF, and chooses
/// nothing else.
#[test]
fn trusting_a_timestamp_authority_needs_one() {
    let (dir, _) = setup("trust-timestamp-refused");
    sample_in(&dir, "node-signpdf/signed.pdf", "plain.pdf");
    sample_in(
        &dir,
        "pyhanko/pades-lta-dss-indirect-arrs-test.pdf",
        "lta.pdf",
    );
    sample_in(&dir, "keys/tsa.crt.der", "tsa.cer");
    let before = std::fs::read(dir.join("trust.txt")).unwrap();
    for (args, code, says) in [
        (
            "input.pdf --signature 1 --timestamp --authority",
            2,
            "cannot be used with",
        ),
        (
            "input.pdf --signature 1 --timestamp --pick 1",
            1,
            "--pick chooses an authority (with --authority) or a certificate in a file",
        ),
        ("tsa.cer --timestamp", 1, "--timestamp is for PDF files"),
        (
            "plain.pdf --timestamp",
            1,
            "signature 1 has no embedded timestamp",
        ),
        (
            "lta.pdf --signature 2 --timestamp",
            1,
            "signature 2 is a document timestamp: leave out --timestamp to trust its authority",
        ),
    ] {
        let (got, _, err) = cli(&dir, &format!("trust add {args}"));
        assert_eq!(got, Some(code), "{args}: {err}");
        assert!(err.contains(says), "{args}: {err}");
        unchanged(&dir.join("trust.txt"), &before);
    }
}

/// PB-116: `trust add --authority` lists the certificate authorities a signature and its
/// timestamp carry, and trusting the one that issued the timestamp authority's certificate
/// trusts the timestamp. The signature here carries them too: that those only its timestamp
/// carries are offered is tested in limarepdf-core (signature_trust.rs) and the app.
#[test]
fn trusting_the_authority_above_a_timestamp_trusts_it() {
    let (dir, _) = setup("trust-timestamp-authority");
    sample_in(
        &dir,
        "pyhanko/pades-lta-dss-indirect-arrs-test.pdf",
        "lta.pdf",
    );
    let before = std::fs::read(dir.join("trust.txt")).unwrap();
    let (code, _, err) = cli(&dir, "trust add lta.pdf --signature 1 --authority");
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("there are 2 to choose from"), "{err}");
    assert!(err.contains("  1. CN=Root CA,"), "{err}");
    assert!(err.contains("  2. CN=Intermediate CA,"), "{err}");
    unchanged(&dir.join("trust.txt"), &before);
    let (_, text, err) = cli(&dir, "verify lta.pdf");
    assert!(text.contains("(from the signer's computer)"), "{text}{err}");
    let (code, said, err) = cli(&dir, "trust add lta.pdf --signature 1 --authority --pick 1");
    assert_eq!(code, Some(0), "{err}");
    assert!(
        said.contains("Now trusting \"Root CA\" as a certificate authority."),
        "{said}"
    );
    let (_, text, err) = cli(&dir, "verify lta.pdf");
    assert!(
        text.contains("by Time Stamping Authority (trusted)"),
        "{text}{err}"
    );
    assert!(text.contains("(from a timestamp authority)"), "{text}");
}
