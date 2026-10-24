//! `limarepdf-cli objects` lists a line in more than one style as one object, with its first
//! part's font and size, marked "mixed".

use std::path::{Path, PathBuf};
use std::process::Command;

/// A one-page PDF with a line in one style, a line with a bold word in it, and a line in
/// two font objects of one font, which looks like one style.
fn make_pdf(dir: &Path) -> PathBuf {
    let content = "BT /F1 12 Tf 10 150 Td (Plain words) Tj ET \
                   BT /F1 12 Tf 10 100 Td (A ) Tj /F2 12 Tf (bold) Tj /F1 12 Tf ( word) Tj ET \
                   BT /F1 12 Tf 10 50 Td (Same ) Tj /F3 12 Tf (look) Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R /F2 6 0 R /F3 7 0 R >> >> >>"
            .into(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
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
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("input.pdf");
    std::fs::write(&path, pdf).unwrap();
    path
}

#[test]
fn a_line_in_more_than_one_style_is_marked_mixed() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("objects");
    let pdf = make_pdf(&dir);
    let out = Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .arg("objects")
        .arg(&pdf)
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| l.contains("Text")).collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert!(
        lines[0].ends_with("\"Plain words\" (Helvetica 12.0pt)"),
        "{text}"
    );
    assert!(
        lines[1].ends_with("\"A bold word\" (Helvetica 12.0pt, mixed)"),
        "{text}"
    );
    assert!(
        lines[2].ends_with("\"Same look\" (Helvetica 12.0pt)"),
        "{text}"
    );
}
