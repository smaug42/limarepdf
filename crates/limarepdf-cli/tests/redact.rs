//! `limarepdf-cli redact`: what is under an area, or every occurrence of a text, is taken
//! out of the output for good and the rest kept; areas marked with `--mark-only` are
//! applied later with `--apply`. The input is never written over, and nothing is written
//! when there is nothing to redact.

use limarepdf_core::Pdf;
use std::path::{Path, PathBuf};
use std::process::Output;

/// A one-page PDF at `dir/in.pdf`, 300 by 200 points, with the lines "Name: Zebra here" and
/// "Second Quokka line" in Helvetica (no widths in the file), its content not compressed.
fn make_pdf(dir: &Path) -> PathBuf {
    let content = "BT /F1 20 Tf 30 150 Td (Name: Zebra here) Tj ET\n\
                   BT /F1 20 Tf 30 100 Td (Second Quokka line) Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .into(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    let size = objects.len() + 1;
    pdf.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let path = dir.join("in.pdf");
    std::fs::write(&path, pdf).unwrap();
    path
}

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("redact")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .current_dir(dir)
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .args(args)
        .output()
        .unwrap()
}

/// Runs the command, which must succeed.
fn ok(dir: &Path, args: &[&str]) {
    let res = run(dir, args);
    let err = String::from_utf8_lossy(&res.stderr);
    assert!(res.status.success(), "{args:?}: {err}");
}

/// Runs the command, which must fail; returns its error.
fn refused(dir: &Path, args: &[&str]) -> String {
    let res = run(dir, args);
    let err = String::from_utf8_lossy(&res.stderr).into_owned();
    assert_eq!(res.status.code(), Some(1), "{args:?}: {err}");
    err
}

fn text_of(path: &Path) -> String {
    Pdf::open(path, None).unwrap().page_text(0)
}

/// Whether the file at `path` holds `word` as written: in its bytes, or in any stream in
/// it decoded, used or not.
fn holds(path: &Path, word: &str) -> bool {
    let bytes = std::fs::read(path).unwrap();
    let w = word.as_bytes();
    let has = |b: &[u8]| b.windows(w.len()).any(|x| x == w);
    let pdf = Pdf::from_bytes(&bytes, None).unwrap();
    let in_stream = pdf.doc.objects.values().any(|o| {
        o.as_stream().is_ok_and(|s| {
            let data = s.decompressed_content();
            has(data.as_deref().unwrap_or(&s.content))
        })
    });
    has(&bytes) || in_stream
}

/// Journey B from the command line: `redact --area` over "Zebra" (its box as LimarePDF
/// reads it, given as PAGE:X,Y,W,H from the top left) writes an output without it, in its
/// text or anywhere in the file, and with the rest of both lines; `--text quokka` takes
/// every "Quokka", whatever its case, and leaves the rest. An output naming the input is
/// refused, and a text not found is nothing to redact: no file written either way, and the
/// input never changed. (Another file at the output's name is replaced, as for every
/// command writing one file; only commands writing several refuse, see `no_replace.rs`.)
#[test]
fn redact_removes_the_text_under_the_area_and_keeps_the_rest() {
    let dir = folder("area");
    let input = make_pdf(&dir);
    let before = std::fs::read(&input).unwrap();
    assert!(holds(&input, "Zebra") && holds(&input, "Quokka"));
    let b = Pdf::open(&input, None).unwrap().find_text_boxes(0, "Zebra");
    let b = b.unwrap();
    assert_eq!(b.len(), 1, "{b:?}");
    let b = b[0];
    let area = format!("1:{},{},{},{}", b[0], b[1], b[2] - b[0], b[3] - b[1]);
    let by_area = ["redact", "in.pdf", "-o", "area.pdf", "--area", &area];
    ok(&dir, &by_area);
    let out = dir.join("area.pdf");
    let text = text_of(&out);
    assert!(!text.contains("Zebra"), "{text:?}");
    for kept in ["Name:", "here", "Second Quokka line"] {
        assert!(text.contains(kept), "{kept} went: {text:?}");
    }
    assert!(!holds(&out, "Zebra"), "Zebra is still in the file");

    let by_text = ["redact", "in.pdf", "-o", "text.pdf", "--text", "quokka"];
    ok(&dir, &by_text);
    let out = dir.join("text.pdf");
    let text = text_of(&out);
    assert!(!text.to_lowercase().contains("quokka"), "{text:?}");
    for kept in ["Name: Zebra here", "Second", "line"] {
        assert!(text.contains(kept), "{kept} went: {text:?}");
    }
    assert!(!holds(&out, "Quokka"), "Quokka is still in the file");

    let over = ["redact", "in.pdf", "-o", "in.pdf", "--text", "zebra"];
    let err = refused(&dir, &over);
    assert!(err.contains("must be saved to another file"), "{err}");
    let missing = ["redact", "in.pdf", "-o", "none.pdf", "--text", "walrus"];
    let err = refused(&dir, &missing);
    assert!(err.contains("nothing to redact"), "{err}");
    assert!(!dir.join("none.pdf").exists(), "written anyway");
    let now = std::fs::read(&input).unwrap();
    assert!(now == before, "the input changed");
}

/// `--mark-only` writes the marks for checking and takes nothing out; `--apply` on that
/// file then takes out what is under them, and the marks with them.
#[test]
fn marks_made_with_mark_only_are_applied_later() {
    let dir = folder("mark-only");
    make_pdf(&dir);
    let mark = ["redact", "in.pdf", "-o", "marked.pdf", "--text", "zebra"];
    ok(&dir, &[&mark[..], &["--mark-only"]].concat());
    let marked = Pdf::open(dir.join("marked.pdf"), None).unwrap();
    assert_eq!(marked.redaction_marks().len(), 1);
    let text = marked.page_text(0);
    assert!(text.contains("Name: Zebra here"), "removed: {text:?}");

    let apply = ["redact", "marked.pdf", "-o", "applied.pdf", "--apply"];
    ok(&dir, &apply);
    let out = dir.join("applied.pdf");
    let applied = Pdf::open(&out, None).unwrap();
    assert!(applied.redaction_marks().is_empty(), "the marks are left");
    let text = applied.page_text(0);
    assert!(!text.contains("Zebra"), "{text:?}");
    for kept in ["Name:", "here", "Second Quokka line"] {
        assert!(text.contains(kept), "{kept} went: {text:?}");
    }
    assert!(!holds(&out, "Zebra"), "Zebra is still in the file");
}
