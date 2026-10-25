//! What can't be written in LimarePDF's own folders is said, apart from what was done
//! (BUG-14, audit N03): an ID imported whose default can't be noted is kept, with a warning,
//! and a signed copy is written whole even when the ID used can't be noted as the default.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A one-page PDF at `dir/name`.
fn make_pdf(dir: &Path, name: &str) -> PathBuf {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let path = dir.join(name);
    std::fs::write(&path, pdf).unwrap();
    path
}

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("unwritable_settings")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .current_dir(dir)
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .args(args)
        .output()
        .unwrap()
}

/// Runs the command, which must succeed; returns what it printed and its warnings.
fn ok(dir: &Path, args: &[&str]) -> (String, String) {
    let res = run(dir, args);
    let out = String::from_utf8_lossy(&res.stdout).into_owned();
    let err = String::from_utf8_lossy(&res.stderr).into_owned();
    assert!(res.status.success(), "{args:?}: {err}");
    (out, err)
}

/// An ID folder holding test ID `p256`, its password in `dir/pw`, whose default can't be
/// noted: a folder has the name of the file that notes it.
fn ids_without_default(dir: &Path) -> &'static [&'static str] {
    let p12 = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../limarepdf-core/tests/data/signatures/ids/p256.p12"
    );
    std::fs::copy(p12, dir.join("p256.p12")).unwrap();
    std::fs::write(dir.join("pw"), "mpe-test-password\n").unwrap();
    std::fs::create_dir_all(dir.join("ids").join("default.txt")).unwrap();
    &["--password-file", "pw", "--id-folder", "ids"]
}

#[test]
fn an_id_imported_without_its_default_is_kept_and_said() {
    let dir = folder("import");
    let opts = ids_without_default(&dir);
    let (out, err) = ok(&dir, &[&["id", "import", "p256.p12"], opts].concat());
    assert!(out.starts_with("Imported the digital ID"), "{out}");
    assert!(!out.contains("It is the default"), "{out}");
    assert!(
        err.starts_with("warning: it couldn't be made the default for signing ("),
        "{err}"
    );
    // It is there once: importing it again is refused as already there.
    let (list, _) = ok(&dir, &["id", "list", "--id-folder", "ids"]);
    assert_eq!(list.matches("(test-signer-p-256)").count(), 1, "{list}");
    let again = run(&dir, &[&["id", "import", "p256.p12"], opts].concat());
    let err = String::from_utf8_lossy(&again.stderr);
    assert!(!again.status.success(), "{err}");
    assert!(err.contains("already in LimarePDF"), "{err}");
}

#[test]
fn a_signed_copy_is_written_when_the_default_can_t_be_kept() {
    let dir = folder("sign");
    let opts = ids_without_default(&dir);
    // Imported with the default noted, then the note made unwritable.
    std::fs::remove_dir(dir.join("ids").join("default.txt")).unwrap();
    ok(&dir, &[&["id", "import", "p256.p12"], opts].concat());
    std::fs::remove_file(dir.join("ids").join("default.txt")).unwrap();
    std::fs::create_dir(dir.join("ids").join("default.txt")).unwrap();
    make_pdf(&dir, "in.pdf");
    let sign = "sign in.pdf -o out.pdf --id 1 --page 1 --rect 20,20,100,40";
    let sign: Vec<&str> = sign.split(' ').collect();
    let (out, err) = ok(&dir, &[&sign[..], opts].concat());
    assert!(out.starts_with("Signed as"), "{out}");
    assert!(
        err.starts_with("warning: the ID couldn't be kept as the default for signing: "),
        "{err}"
    );
    let (verified, _) = ok(&dir, &["verify", "out.pdf"]);
    assert!(verified.contains("valid"), "{verified}");
}
