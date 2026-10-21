//! Text from a file printed by `limarepdf-cli` can't send terminal escape codes or reorder
//! the line with bidi controls: listings show it cleaned (JSON output stays exact).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Text with an escape code (ESC [31m turns the terminal red), the one-byte CSI, a bidi
/// override and a line break, as a UTF-16 hex string (so PDFDocEncoding can't remap them).
fn nasty(visible: &str) -> String {
    let text = format!("{visible}\u{1b}[31m\u{9b}2J\u{202e}x\ny");
    let hex: String = text.encode_utf16().map(|u| format!("{u:04X}")).collect();
    format!("<FEFF{hex}>")
}

/// A one-page PDF with that text in its title, a bookmark, a comment's author and date (an
/// unreadable date shows as "-"), a
/// form field's name, a page label prefix, a font name and the page's text.
fn make_pdf(dir: &Path) -> PathBuf {
    let objects = [
        format!(
            "<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R /AcroForm << /Fields [8 0 R] >> \
             /PageLabels << /Nums [0 << /S /D /P {} >>] >> >>",
            nasty("Label")
        ),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> /Annots [7 0 R 8 0 R] >>"
            .into(),
        {
            let content = "BT /F1 12 Tf 10 100 Td (Findme \x1b[31m red) Tj ET";
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            )
        },
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helv#1B#5B31m#E2#80#AEx >>".into(),
        "<< /Type /Outlines /First 9 0 R /Last 9 0 R /Count 1 >>".into(),
        format!(
            "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /T {} /M {} /Contents (hi) >>",
            nasty("Author"),
            nasty("D:2026")
        ),
        format!(
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T {} /Rect [50 50 150 70] /P 3 0 R >>",
            nasty("Field")
        ),
        format!(
            "<< /Title {} /Parent 6 0 R /Dest [3 0 R /Fit] >>",
            nasty("Chapter")
        ),
        format!("<< /Title {} >>", nasty("Title")),
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
            "trailer\n<< /Size {} /Root 1 0 R /Info 10 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    let path = dir.join("escapes.pdf");
    std::fs::write(&path, pdf).unwrap();
    path
}

fn run(args: &[&str], file: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .args(args)
        .arg(file)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    text
}

#[test]
fn listings_show_file_text_without_control_characters() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("escapes");
    std::fs::create_dir_all(&dir).unwrap();
    let file = make_pdf(&dir);
    for (args, visible) in [
        (&["info"][..], "Title"),
        (&["bookmarks", "list"], "Chapter"),
        (&["annot", "list"], "Author"),
        (&["form", "list"], "Field"),
        (&["labels", "list"], "Label"),
        (&["objects"], "Helv"),
    ] {
        let out = run(args, &file);
        assert!(out.contains(visible), "{args:?} lost {visible:?}: {out:?}");
        for bad in ['\u{1b}', '\u{9b}', '\u{202e}'] {
            assert!(!out.contains(bad), "{args:?} printed {bad:?}: {out:?}");
        }
    }
    // The page text, found by search.
    let out = Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .arg("search")
        .arg(&file)
        .arg("Findme")
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&out.stdout);
    assert!(out.contains("Findme"), "search: {out:?}");
    assert!(!out.contains('\u{1b}'), "search printed ESC: {out:?}");
}
