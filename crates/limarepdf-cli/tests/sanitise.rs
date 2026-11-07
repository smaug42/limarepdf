//! `limarepdf-cli sanitise --also outside-crop` (S1-02, PB-85): when the check after cutting
//! what lies outside the crop box finds what was cut may still be in the file, the command
//! fails and writes nothing; otherwise the output is without it.

use limarepdf_core::Pdf;
use std::path::{Path, PathBuf};
use std::process::Output;

const HELVETICA: &str =
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

/// A stream object of `dict`'s entries holding `data`, not compressed.
fn stream(dict: &str, data: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    )
}

/// A PDF at `dir/in.pdf` of `objects`, numbered from 1, the first its catalog.
fn write_pdf(dir: &Path, objects: &[String]) -> PathBuf {
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

/// A one-page PDF at `dir/in.pdf`, 300 by 200 points cropped to [50 20 250 180], with
/// "Inside" on the page and a form drawing "OutZebra" left of the crop box and "Kept" inside
/// it; with `tagged`, the document's tag tree points at the form.
fn make_pdf(dir: &Path, tagged: bool) -> PathBuf {
    let tags = if tagged {
        " /StructTreeRoot 7 0 R /MarkInfo << /Marked true >>"
    } else {
        ""
    };
    let mut objects = vec![
        format!("<< /Type /Catalog /Pages 2 0 R{tags} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /CropBox [50 20 250 180] \
         /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> /XObject << /Fm 6 0 R >> >> >>"
            .into(),
        stream("", "BT /F1 12 Tf 100 100 Td (Inside) Tj ET q /Fm Do Q"),
        HELVETICA.into(),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 300 200] \
             /Resources << /Font << /F1 5 0 R >> >>",
            "BT /F1 6 Tf 2 100 Td (OutZebra) Tj ET BT /F1 12 Tf 100 60 Td (Kept) Tj ET",
        ),
    ];
    if tagged {
        objects.extend([
            "<< /Type /StructTreeRoot /K 8 0 R >>".to_string(),
            "<< /Type /StructElem /S /P /P 7 0 R /Pg 3 0 R \
             /K [<< /Type /MCR /Pg 3 0 R /Stm 6 0 R /MCID 0 >>] >>"
                .into(),
        ]);
    }
    write_pdf(dir, &objects)
}

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("sanitise")
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

/// The tag tree still reaching the form cut: refused, exit code 1, the reason said, and no
/// output written; --list still counts the page. Without the tag tree, the same command
/// writes an output without "OutZebra" and with "Kept".
#[test]
fn content_outside_the_crop_box_redaction_leaves_is_refused() {
    let dir = folder("refused");
    let input = make_pdf(&dir, true);
    let was = std::fs::read(&input).unwrap();
    let args = [
        "sanitise",
        "in.pdf",
        "-o",
        "out.pdf",
        "--also",
        "outside-crop",
    ];
    let res = run(&dir, &args);
    let err = String::from_utf8_lossy(&res.stderr);
    assert_eq!(res.status.code(), Some(1), "{err}");
    assert!(
        err.contains(
            "Content outside the visible page area can't be removed for certain: the check \
             after redacting found removed content may still be in the file, through the tag \
             tree"
        ),
        "{err}"
    );
    assert!(!dir.join("out.pdf").exists(), "an output was written");
    assert_eq!(std::fs::read(&input).unwrap(), was);
    // --list counts the page, so it can be chosen.
    let res = run(&dir, &["sanitise", "in.pdf", "--list"]);
    let (said, err) = (
        String::from_utf8_lossy(&res.stdout),
        String::from_utf8_lossy(&res.stderr),
    );
    assert!(res.status.success(), "{err}");
    assert!(
        said.lines().any(|l| {
            let w: Vec<_> = l.split_whitespace().collect();
            w.first() == Some(&"outside-crop") && w.get(1) == Some(&"1")
        }),
        "{said}"
    );

    let dir = folder("control");
    make_pdf(&dir, false);
    let res = run(&dir, &args);
    let err = String::from_utf8_lossy(&res.stderr);
    assert!(res.status.success(), "{err}");
    let out = dir.join("out.pdf");
    assert!(!holds(&out, "OutZebra"), "OutZebra is still in the file");
    assert!(Pdf::open(&out, None).unwrap().page_text(0).contains("Kept"));
}
