//! `limarepdf-cli merge` and form fields (BUG-19): the inputs after the first keep their
//! answers only drawn onto their pages, which `--flatten-forms` asks for.

use limarepdf_core::Pdf;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A one-page PDF at `dir/name` whose objects after the page tree are `rest` (from object
/// 4 on), with `catalog` added to its catalog and `annots` to its page.
fn make_pdf(dir: &Path, name: &str, catalog: &str, annots: &str, rest: &[&str]) -> PathBuf {
    let head = [
        format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] {annots} >>"),
    ];
    let objects: Vec<String> = head
        .into_iter()
        .chain(rest.iter().map(|s| s.to_string()))
        .collect();
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    let n = objects.len() + 1;
    pdf.extend_from_slice(format!("xref\n0 {n}\n0000000000 65535 f \n").as_bytes());
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {n} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let path = dir.join(name);
    std::fs::write(&path, pdf).unwrap();
    path
}

/// A form whose text field Name holds `answer` (a PDF string) with no drawing of its own,
/// as apps that leave the drawing to the reader save it.
fn form_pdf(dir: &Path, name: &str, answer: &str) -> PathBuf {
    let field = format!(
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Name) /V {answer} \
         /Rect [10 150 160 170] /P 3 0 R /F 4 >>"
    );
    make_pdf(
        dir,
        name,
        "/AcroForm << /Fields [4 0 R] /NeedAppearances true /DA (/Helv 0 Tf 0 g) \
         /DR << /Font << /Helv 5 0 R >> >> >>",
        "/Annots [4 0 R]",
        &[
            &field,
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        ],
    )
}

fn merge(dir: &Path, inputs: &[&Path], extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .arg("merge")
        .args(inputs)
        .arg("-o")
        .arg(dir.join("out.pdf"))
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn merge_draws_the_form_fields_of_later_inputs_only_when_asked() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("merge-forms");
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("out.pdf");
    let _ = std::fs::remove_file(&out);
    let plain = make_pdf(&dir, "plain.pdf", "", "", &[]);
    let form = form_pdf(&dir, "form.pdf", "(Ada Lovelace)");
    // שלום, which LimarePDF can't write in a field yet (right to left).
    let foreign = form_pdf(&dir, "foreign.pdf", "<FEFF05E905DC05D505DD>");

    let r = merge(&dir, &[&plain, &form], &[]);
    let err = String::from_utf8_lossy(&r.stderr);
    assert!(!r.status.success());
    assert!(
        err.contains("form.pdf has form fields") && err.contains("--flatten-forms"),
        "{err}"
    );
    assert!(!out.exists(), "written anyway");

    let r = merge(&dir, &[&plain, &foreign], &["--flatten-forms"]);
    let err = String::from_utf8_lossy(&r.stderr);
    assert!(!r.status.success());
    assert!(
        err.contains("\"Name\" in") && err.contains("Nothing was written."),
        "{err}"
    );
    assert!(!out.exists(), "written anyway");

    let r = merge(&dir, &[&plain, &form], &["--flatten-forms"]);
    let said = String::from_utf8_lossy(&r.stdout);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(said.contains("drew the form fields of"), "{said}");
    let m = Pdf::open(&out, None).unwrap();
    assert!(m.page_text(1).contains("Ada Lovelace"));
    assert!(m.form_fields().is_empty());

    // The first input's form is kept as it is, fillable.
    let r = merge(&dir, &[&form, &plain], &[]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let m = Pdf::open(&out, None).unwrap();
    assert_eq!(m.form_fields().len(), 1);
}
