//! `limarepdf-cli form fill`: a field past the 10,000 LimarePDF lists is filled by name, as
//! any other (PB-174). It was refused as not there.

use limarepdf_core::{FieldValue, Pdf};
use std::path::{Path, PathBuf};
use std::process::Output;

/// How many fields LimarePDF lists.
const LISTED: usize = 10_000;

/// A stream object of `dict`'s entries holding `data`, not compressed.
fn stream(dict: &str, data: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    )
}

/// A one-page PDF at `dir/in.pdf`, 300 by 200 points, with a form of [`LISTED`] empty text
/// fields sharing one drawing, then `last`, with none; each field is its own widget on the
/// page.
fn make_pdf(dir: &Path) -> PathBuf {
    const FIRST: usize = 7;
    let ids: Vec<String> = (0..=LISTED).map(|i| format!("{} 0 R", FIRST + i)).collect();
    let ids = ids.join(" ");
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Annots [{ids}] >>"),
        format!("<< /Fields [{ids}] /DA (/Helv 10 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >>"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10]", ""),
    ];
    for i in 0..LISTED {
        objects.push(format!(
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (f{i}) /F 4 /Rect [0 0 10 10] \
             /P 3 0 R /AP << /N 6 0 R >> >>"
        ));
    }
    objects.push(
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (last) /F 4 /Rect [10 150 160 170] \
         /P 3 0 R >>"
            .into(),
    );
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
        .join("form")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    limarepdf_core::child_seam::output(
        std::process::Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
            .current_dir(dir)
            .env("HOME", dir)
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .args(args),
    )
    .unwrap()
}

#[test]
fn fill_sets_a_field_past_those_listed() {
    let dir = folder("past-listed");
    make_pdf(&dir);
    let res = run(
        &dir,
        &[
            "form",
            "fill",
            "in.pdf",
            "--set",
            "last=Jane Doe",
            "-o",
            "out.pdf",
        ],
    );
    let out = String::from_utf8_lossy(&res.stdout);
    let err = String::from_utf8_lossy(&res.stderr);
    assert_eq!(res.status.code(), Some(0), "{err}");
    assert!(out.starts_with("filled 1 field(s); wrote "), "{out}");

    let pdf = Pdf::open(dir.join("out.pdf"), None).unwrap();
    let last = pdf.every_form_field().pop().unwrap();
    assert_eq!(last.name, "last");
    assert_eq!(last.value, FieldValue::Text("Jane Doe".into()));
}
