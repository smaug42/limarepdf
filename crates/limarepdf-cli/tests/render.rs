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
    write_pdf(dir, name, b"%PDF-1.7\n", &objects)
}

/// A PDF at `dir/name` starting with `header`, holding `objects` numbered from 1, the first
/// the catalogue.
fn write_pdf(dir: &Path, name: &str, header: &[u8], objects: &[String]) -> PathBuf {
    let mut pdf = header.to_vec();
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
    let path = dir.join(name);
    std::fs::write(&path, pdf).unwrap();
    path
}

/// A stream object holding `data`.
fn stream(dict: &str, data: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    )
}

/// A one-page PDF at `dir/name` drawing `content` with `resources`; `more` are objects 5
/// on.
fn page_pdf(
    dir: &Path,
    name: &str,
    header: &[u8],
    content: &str,
    resources: &str,
    more: &[String],
) -> PathBuf {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R \
             /Resources {resources} >>"
        ),
        stream("", content),
    ];
    objects.extend_from_slice(more);
    write_pdf(dir, name, header, &objects)
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

/// The "warning:" lines of `err`.
fn warnings(err: &str) -> Vec<&str> {
    err.lines().filter(|l| l.starts_with("warning:")).collect()
}

#[test]
fn pictures_that_cant_be_drawn_are_named() {
    let dir = folder("picture");
    let file = page_pdf(
        &dir,
        "picture.pdf",
        b"%PDF-1.7\n",
        "q 100 0 0 50 50 75 cm /Im Do Q",
        "<< /XObject << /Im 5 0 R >> >>",
        &[stream(
            "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceRGB \
             /BitsPerComponent 8 /Filter /DCTDecode",
            "this is not a JPEG",
        )],
    );
    let out = render(&file);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let w = warnings(&err);
    assert_eq!(w.len(), 1, "{err}");
    assert!(w[0].contains("a picture"), "{err}");
    assert!(std::fs::read(dir.join("picture-001.png"))
        .unwrap()
        .starts_with(b"\x89PNG"));
}

#[test]
fn fonts_that_are_not_installed_are_named() {
    let dir = folder("font");
    // A Latin font no computer has, not embedded: drawn with an installed font in its place,
    // or not at all where none can stand in. Either way it's named.
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
        /CMapName /Test def 1 begincodespacerange <0000> <FFFF> endcodespacerange \
        1 beginbfchar <0001> <0057> endbfchar endcmap \
        CMapName currentdict /CMap defineresource pop end end";
    let file = page_pdf(
        &dir,
        "font.pdf",
        b"%PDF-1.7\n",
        "BT /F1 96 Tf 40 50 Td <0001> Tj ET",
        "<< /Font << /F1 << /Type /Font /Subtype /Type0 /BaseFont /Quillwyrt \
         /Encoding /Identity-H /DescendantFonts [5 0 R] /ToUnicode 6 0 R >> >> >>",
        &[
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Quillwyrt /CIDSystemInfo \
             << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 1000 >>"
                .into(),
            stream("", cmap),
        ],
    );
    let out = render(&file);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let w = warnings(&err);
    assert_eq!(w.len(), 1, "{err}");
    assert!(w[0].contains("Quillwyrt"), "{err}");
}

#[test]
fn a_file_the_document_reader_cant_open_is_refused() {
    // As the app refuses it (PB-93): drawn from its bytes as they are, it would miss the
    // pictures left out of every copy pages are drawn from (PB-81).
    let dir = folder("unopened");
    let file = page_pdf(
        &dir,
        "headless.pdf",
        b"",
        "0 0 1 rg 0 0 50 50 re f",
        "<< >>",
        &[],
    );
    let out = render(&file);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains("cannot open PDF"), "{err}");
    assert!(!dir.join("headless-001.png").exists());
}
