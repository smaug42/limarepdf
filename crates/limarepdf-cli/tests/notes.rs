//! `limarepdf-cli` says on stderr when other PDF apps may show a file differently, as the
//! app's banner does.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A one-page PDF, followed by `after` (data past its end-of-file marker).
fn make_pdf(dir: &Path, name: &str, after: &str) -> PathBuf {
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
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n{after}").as_bytes(),
    );
    let path = dir.join(name);
    std::fs::write(&path, pdf).unwrap();
    path
}

fn info_stderr(file: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .arg("info")
        .arg(file)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{err}");
    err
}

#[test]
fn files_other_apps_may_show_differently_get_a_note() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("notes");
    std::fs::create_dir_all(&dir).unwrap();
    let plain = make_pdf(&dir, "plain.pdf", "");
    assert_eq!(info_stderr(&plain), "");
    // An object after the end, which apps find the newest revision past in different ways.
    let after = make_pdf(&dir, "after.pdf", "3 0 obj\n<< /Type /Page >>\nendobj\n");
    let err = info_stderr(&after);
    assert!(
        err.starts_with("note: ") && err.contains("other PDF apps may show it differently"),
        "{err}"
    );
}
