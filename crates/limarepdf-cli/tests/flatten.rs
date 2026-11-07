//! `limarepdf-cli annot flatten`: an annotation LimarePDF can't draw is left as it was,
//! rather than removed undrawn, and a warning on stderr says how many and on which pages
//! (H-06). The output is still written, and the exit status is 0.

use limarepdf_core::Pdf;
use std::path::{Path, PathBuf};
use std::process::Output;

/// A square annotation over 20,20–120,70 drawn by object `ap`.
fn square(ap: usize) -> String {
    format!("<< /Type /Annot /Subtype /Square /F 4 /Rect [20 20 120 70] /AP << /N {ap} 0 R >> >>")
}

/// A stream object of `dict`'s entries holding `data`, not compressed.
fn stream(dict: &str, data: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    )
}

/// A two-page PDF at `dir/in.pdf`, 300 by 200 points. Page 1 has a square LimarePDF can
/// draw; with `unboxed`, each page also has one whose drawing has no box, so it can't be
/// placed on the page.
fn make_pdf(dir: &Path, unboxed: bool) -> PathBuf {
    let fill = "0 0 1 rg 0 0 5 5 re f";
    let (first, second) = if unboxed {
        ("[6 0 R 7 0 R]", "[8 0 R]")
    } else {
        ("[6 0 R]", "[]")
    };
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Annots {first} >>"),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Annots {second} >>"),
        stream("/Subtype /Form /BBox [0 0 5 5]", fill),
        square(5),
        square(9),
        square(9),
        stream("/Subtype /Form", fill),
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
        .join("flatten")
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

/// Flattens `dir/in.pdf` into `dir/out.pdf`, which must succeed; returns stdout and stderr.
fn flatten(dir: &Path) -> (String, String) {
    let res = run(dir, &["annot", "flatten", "in.pdf", "-o", "out.pdf"]);
    let out = String::from_utf8_lossy(&res.stdout).into_owned();
    let err = String::from_utf8_lossy(&res.stderr).into_owned();
    assert_eq!(res.status.code(), Some(0), "{err}");
    (out, err)
}

/// The number of annotations on each page of `path`.
fn annotations(path: &Path) -> Vec<usize> {
    let pdf = Pdf::open(path, None).unwrap();
    let all = pdf.annotations();
    (0..pdf.page_count())
        .map(|i| all.iter().filter(|a| a.page == i).count())
        .collect()
}

#[test]
fn what_it_cannot_draw_is_kept_with_a_warning() {
    let dir = folder("kept");
    make_pdf(&dir, true);
    let (out, err) = flatten(&dir);
    assert!(
        out.starts_with("flattened 1 annotation(s); wrote "),
        "{out}"
    );
    assert_eq!(
        err,
        "warning: 2 annotation(s) LimarePDF can't draw were left as they were (pages 1, 2)\n"
    );
    assert_eq!(annotations(&dir.join("out.pdf")), [1, 1]);
}

/// Control: with everything drawn, no warning, as before.
#[test]
fn all_drawn_gives_no_warning() {
    let dir = folder("all-drawn");
    make_pdf(&dir, false);
    let (out, err) = flatten(&dir);
    assert!(
        out.starts_with("flattened 1 annotation(s); wrote "),
        "{out}"
    );
    assert_eq!(err, "");
    assert_eq!(annotations(&dir.join("out.pdf")), [0, 0]);
}
