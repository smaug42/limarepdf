//! `ocr` and `remove-ocr` (#223, #224): pages with recognised text skipped unless read
//! `--again`; recognised text taken out, the page's own text kept.

use limarepdf_core::render::{self, encode, ImageFormat, Renderer};
use limarepdf_core::{FontChoice, ImageImport, Pdf};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("ocr")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    limarepdf_core::child_seam::output(
        Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
            .current_dir(dir)
            .env("HOME", dir)
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .args(args),
    )
    .unwrap()
}

fn ok(dir: &Path, args: &str) -> String {
    let args: Vec<_> = args.split(' ').collect();
    let res = run(dir, &args);
    let err = String::from_utf8_lossy(&res.stderr).into_owned();
    assert!(res.status.success(), "{args:?}: {err}");
    String::from_utf8_lossy(&res.stdout).into_owned()
}

/// `in.pdf`: a 4 x 2 inch scan at 300 dpi of two lines of text, and a line of the page's own
/// text below it.
fn scan_pdf(dir: &Path) {
    let mut p = Pdf::new();
    p.insert_blank(0, 288.0, 144.0).unwrap();
    for (y, l) in [
        (40.0, "The parcel left Ottawa on Friday."),
        (70.0, "It weighs 12 kilograms."),
    ] {
        p.add_text(0, (16.0, y), l, FontChoice::default(), 14.0, [0.0; 3])
            .unwrap();
    }
    let hp = render::load(p.snapshot().unwrap(), None).unwrap();
    let b = Renderer::new(&hp).render(0, 300.0 / 72.0).unwrap();
    let png = encode(&b, ImageFormat::Png, 300.0).unwrap();
    let opts = ImageImport {
        dpi: Some(300.0),
        ..Default::default()
    };
    let mut scan = Pdf::from_images(&[png], &opts).unwrap();
    let (stamp, font) = ("LIM-000123", FontChoice::default());
    scan.add_text(0, (200.0, 130.0), stamp, font, 8.0, [0.0; 3])
        .unwrap();
    std::fs::write(dir.join("in.pdf"), scan.to_bytes().unwrap()).unwrap();
}

fn text(dir: &Path, name: &str) -> String {
    Pdf::open(dir.join(name), None).unwrap().page_text(0)
}

#[test]
fn recognised_text_is_read_again_and_removed() {
    let dir = folder("again");
    scan_pdf(&dir);
    let out = ok(&dir, "ocr in.pdf -o read.pdf");
    assert!(out.contains("page 1: 2 lines, "), "{out}");
    assert!(text(&dir, "read.pdf").contains("Ottawa"));

    let out = ok(&dir, "ocr read.pdf -o skipped.pdf");
    let skipped = "page 1: skipped: the page already has recognised text";
    assert!(out.contains(skipped), "{out}");

    let out = ok(&dir, "ocr read.pdf -o again.pdf --again");
    assert!(out.contains("page 1: 2 lines, "), "{out}");
    assert_eq!(text(&dir, "again.pdf"), text(&dir, "read.pdf"));

    let out = ok(&dir, "remove-ocr again.pdf -o removed.pdf");
    assert!(out.contains("page 1: 2 lines removed"), "{out}");
    assert_eq!(text(&dir, "removed.pdf"), "LIM-000123");

    let out = ok(&dir, "remove-ocr removed.pdf -o none.pdf --pages 1");
    let none = "page 1: skipped: the page has no recognised text";
    assert!(out.contains(none), "{out}");
}
