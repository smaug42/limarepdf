//! `limarepdf-cli render` draws filled form fields that have no drawing of their own, as the
//! app and printing do (BUG-23), and names those it can't draw.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A one-page PDF at `dir/name` whose text field "Name" holds `value` (a PDF string) and has
/// no drawing, as an app that leaves the drawing to the reader writes it (`/NeedAppearances`).
fn make_pdf(dir: &Path, name: &str, value: &str) -> PathBuf {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] /DA (/Helv 12 Tf 0 g) \
         /NeedAppearances true >> >>"
            .to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R] >>".into(),
        format!(
            "<< /Type /Annot /Subtype /Widget /FT /Tx /F 4 /T (Name) /Rect [20 20 180 50] \
             /V {value} >>"
        ),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let path = dir.join(name);
    std::fs::write(&path, pdf).unwrap();
    path
}

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("render-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Renders `file` into its folder.
fn render(file: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .arg("render")
        .arg(file)
        .arg("--out-dir")
        .arg(file.parent().unwrap())
        .output()
        .unwrap()
}

#[test]
fn filled_fields_without_a_drawing_are_drawn() {
    let dir = folder("drawn");
    let filled = make_pdf(&dir, "filled.pdf", "(Hello)");
    let empty = make_pdf(&dir, "empty.pdf", "()");
    for f in [&filled, &empty] {
        let out = render(f);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{err}");
        assert!(!err.contains("warning"), "{err}");
    }
    let image = |name: &str| std::fs::read(dir.join(name)).unwrap();
    assert!(image("filled-001.png").starts_with(b"\x89PNG"));
    assert!(
        image("filled-001.png") != image("empty-001.png"),
        "the filled field came out empty"
    );
}

#[test]
fn fields_that_cant_be_drawn_are_named() {
    let dir = folder("named");
    // Hebrew, which LimarePDF can't draw yet (right to left).
    let file = make_pdf(&dir, "form.pdf", "<FEFF05E905DC05D505DD>");
    let out = render(&file);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert!(err.contains("warning") && err.contains("\"Name\""), "{err}");
    assert!(std::fs::read(dir.join("form-001.png"))
        .unwrap()
        .starts_with(b"\x89PNG"));
}
