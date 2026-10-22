//! `limarepdf-cli print` refuses, before anything reaches the printer, when filled fields
//! wouldn't show on paper. `lp` is replaced by a script that records the job, so no
//! printer is needed and none is ever used.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// A one-page PDF with a printed text field holding Japanese text and no drawing, as an app
/// that leaves the drawing to the reader writes it.
fn make_pdf(dir: &Path) -> PathBuf {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] /DA (/Helv 12 Tf 0 g) >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R] >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /F 4 /T (Name) /Rect [20 20 180 50] \
         /V <FEFF65E5672C8A9E> >>",
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
    let path = dir.join("form.pdf");
    std::fs::write(&path, pdf).unwrap();
    path
}

/// Stand-ins for `lp`, which saves the job, and `lpoptions`, which knows no paper size.
fn fake_cups(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let lp = format!(
        "#!/bin/sh\ncat > '{}/job.pdf'\necho 'request id is Test-1 (1 file(s))'\n",
        dir.display()
    );
    for (name, script) in [("lp", lp.as_str()), ("lpoptions", "#!/bin/sh\n")] {
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn fields_that_would_print_empty_stop_printing() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("print-fields");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    fake_cups(&dir);
    let file = make_pdf(&dir);
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let out = Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .args(["print", "--printer", "limarepdf-test"])
        .arg(&file)
        .env("PATH", path)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "printed: {err}");
    assert!(
        err.contains("\"Name\"") && err.contains("Nothing was sent to the printer"),
        "{err}"
    );
    assert!(!dir.join("job.pdf").exists(), "a job reached lp");
}
