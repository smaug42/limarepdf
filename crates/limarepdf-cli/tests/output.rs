//! When `limarepdf-cli` can't write its output (audit R15): a reader that went away (`… | head`)
//! stops it quietly with its usual status (decision D5); any other failure, such as a full
//! disk, is an error with a plain message and status 1, saying whether a file was saved.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A one-page PDF with a line of text and a filled text field.
fn make_pdf(dir: &Path) -> PathBuf {
    let content = "BT /F1 12 Tf 10 100 Td (Some words) Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> /Annots [6 0 R] >>"
            .into(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Name) /V (Walrus) \
         /Rect [50 50 150 70] /P 3 0 R >>"
            .into(),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("input.pdf");
    std::fs::write(&path, pdf).unwrap();
    path
}

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("output")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn run(args: &[&str], dir: &Path, stdout: Stdio) -> Output {
    Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .current_dir(dir)
        .args(args)
        .stdout(stdout)
        .output()
        .unwrap()
}

/// Output to a pipe whose reader has already gone, so every write fails as `… | head` would.
fn closed_pipe() -> Stdio {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    writer.into()
}

/// Output to a full disk.
#[cfg(target_os = "linux")]
fn full() -> Stdio {
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap()
        .into()
}

/// Commands that only report, writing to a full disk, fail with a plain message.
#[cfg(target_os = "linux")]
#[test]
fn a_report_that_cant_be_written_fails() {
    let dir = folder("report");
    make_pdf(&dir);
    for args in [
        &["info", "input.pdf"][..],
        &["text", "input.pdf"],
        &["form", "list", "input.pdf", "--json"],
    ] {
        let out = run(args, &dir, full());
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {err}");
        assert!(
            err.contains("couldn't write the output: "),
            "{args:?}: {err}"
        );
        assert!(err.contains("No space left on device"), "{args:?}: {err}");
        assert!(!err.contains("saved"), "{args:?}: {err}");
        assert!(!err.contains("internal error"), "{args:?}: {err}");
    }
}

/// A command that saved its file before its report couldn't be written says the file was
/// saved; one whose report failed before saving saves nothing.
#[cfg(target_os = "linux")]
#[test]
fn a_failed_report_says_whether_the_file_was_saved() {
    let dir = folder("saved");
    make_pdf(&dir);
    let out = run(
        &[
            "set-info",
            "input.pdf",
            "-o",
            "titled.pdf",
            "--title",
            "Walrus",
        ],
        &dir,
        full(),
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(
        err.contains("the file was saved, but its report couldn't be written: "),
        "{err}"
    );
    let saved = std::fs::read(dir.join("titled.pdf")).unwrap();
    assert!(saved.windows(6).any(|w| w == b"Walrus"));

    // Remove blank pages reports what it found before it saves.
    let out = run(
        &["remove-blank", "input.pdf", "-o", "unblank.pdf"],
        &dir,
        full(),
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("couldn't write the output: "), "{err}");
    assert!(!err.contains("saved"), "{err}");
    assert!(!dir.join("unblank.pdf").exists());
}

/// A reader that went away stops the output quietly, with the status the command would have
/// had: 0 normally, and 1 for `verify` when a check fails (here: no signatures, with
/// `--require-trusted`).
#[test]
fn a_reader_that_went_away_stops_quietly() {
    let dir = folder("closed");
    make_pdf(&dir);
    for args in [
        &["info", "input.pdf"][..],
        &["text", "input.pdf"],
        &["form", "list", "input.pdf", "--json"],
    ] {
        let out = run(args, &dir, closed_pipe());
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {err}");
        assert_eq!(err, "", "{args:?}");
    }
    let out = run(
        &["verify", "input.pdf", "--require-trusted"],
        &dir,
        closed_pipe(),
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(!err.contains("couldn't write"), "{err}");
}

/// Output that can be written is as before.
#[test]
fn output_that_can_be_written_is_unchanged() {
    let dir = folder("normal");
    make_pdf(&dir);
    let out = run(&["text", "input.pdf"], &dir, Stdio::piped());
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "--- page 1 ---\nSome words\n"
    );
    let out = run(
        &["form", "list", "input.pdf", "--json"],
        &dir,
        Stdio::piped(),
    );
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "{\n  \"Name\": \"Walrus\"\n}\n"
    );
}
