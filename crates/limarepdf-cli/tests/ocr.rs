//! `ocr` and `remove-ocr` (#223, #224): pages with recognised text skipped unless read
//! `--again`; recognised text taken out, the page's own text kept. Turned scans turned
//! upright and reported (#225), tilted ones reported (#320). Areas read with `--area`
//! (#226).

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

/// A 4 x 2 inch scan at 300 dpi of two lines of text, tilted `tilt` degrees clockwise and
/// turned `turn` degrees clockwise.
fn scan(turn: i64, tilt: f32) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 288.0, 144.0).unwrap();
    for (y, l) in [
        (40.0, "The parcel left Ottawa on Friday."),
        (70.0, "It weighs 12 kilograms."),
    ] {
        p.add_text(0, (16.0, y), l, FontChoice::default(), 14.0, [0.0; 3])
            .unwrap();
    }
    // About the page's middle, in displayed-page space (y down).
    let (s, c) = tilt.to_radians().sin_cos();
    let (cx, cy) = (144.0, 72.0);
    let m = [c, s, -s, c, cx - c * cx + s * cy, cy - s * cx - c * cy];
    p.transform_objects(0, &[(0, m), (1, m)]).unwrap();
    p.rotate_pages(&[0], turn).unwrap();
    let hp = render::load(p.snapshot().unwrap(), None).unwrap();
    let b = Renderer::new(&hp).render(0, 300.0 / 72.0).unwrap();
    let png = encode(&b, ImageFormat::Png, 300.0).unwrap();
    let opts = ImageImport {
        dpi: Some(300.0),
        ..Default::default()
    };
    Pdf::from_images(&[png], &opts).unwrap()
}

/// `in.pdf`: [`scan`] upright, and a line of the page's own text below it.
fn scan_pdf(dir: &Path) {
    let mut scan = scan(0, 0.0);
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

/// `--area` reads a box, and read again in a larger box its line is replaced, not doubled;
/// a box that isn't one, or isn't on a page, is refused (#226).
#[test]
fn areas_are_read() {
    let dir = folder("area");
    scan_pdf(&dir);
    let out = ok(&dir, "ocr in.pdf -o top.pdf --area 1:0,20,288,35");
    assert!(out.contains("page 1, area 0,20,288,35: 1 lines, "), "{out}");
    let top = text(&dir, "top.pdf");
    assert!(
        top.contains("Ottawa") && !top.contains("kilograms"),
        "{top}"
    );

    let out = ok(&dir, "ocr top.pdf -o all.pdf --area 1:0,0,288,144");
    assert!(out.contains("page 1, area 0,0,288,144: 2 lines, "), "{out}");
    let all = text(&dir, "all.pdf");
    assert_eq!(all.matches("Ottawa").count(), 1, "{all}");
    assert!(
        all.contains("kilograms") && all.contains("LIM-000123"),
        "{all}"
    );

    for (args, why) in [
        ("--area 1:0,0,288", "should be PAGE:X,Y,W,H"),
        ("--area 0:0,0,288,144", "should be PAGE:X,Y,W,H"),
        ("--area 1:0,0,inf,144", "should be PAGE:X,Y,W,H"),
        ("--area 2:0,0,288,144", "no page 2"),
        ("--area 1:0,0,288,144 --again", "cannot be used with"),
        ("--area 1:0,0,288,144 --pages 1", "cannot be used with"),
    ] {
        let args: Vec<_> = ["ocr", "in.pdf", "-o", "bad.pdf"]
            .into_iter()
            .chain(args.split(' '))
            .collect();
        let res = run(&dir, &args);
        let err = String::from_utf8_lossy(&res.stderr);
        assert!(
            !res.status.success() && err.contains(why),
            "{args:?}: {err}"
        );
        assert!(!dir.join("bad.pdf").exists(), "{args:?}");
    }
}

/// A page scanned upside down or on its side is turned upright, and the report says so
/// (#225).
#[test]
fn turned_scans_are_reported() {
    let dir = folder("turned");
    for (turn, was) in [(180, "upside down"), (90, "on its side")] {
        std::fs::write(dir.join("in.pdf"), scan(turn, 0.0).to_bytes().unwrap()).unwrap();
        let out = ok(&dir, "ocr in.pdf -o read.pdf");
        let turned = format!("page 1: turned upright (it was {was}), 2 lines, ");
        assert!(out.contains(&turned), "{out}");
        assert!(text(&dir, "read.pdf").contains("Ottawa"), "{turn}");
    }
}

/// A page tilted 2° or more is reported tilted, with a note on straightening it (#320); one
/// tilted less isn't.
#[test]
fn tilted_scans_are_reported() {
    let dir = folder("tilted");
    for tilt in [3.0, -3.0, 1.0] {
        std::fs::write(dir.join("in.pdf"), scan(0, tilt).to_bytes().unwrap()).unwrap();
        let res = run(&dir, &["ocr", "in.pdf", "-o", "read.pdf"]);
        let (out, err) = (
            String::from_utf8_lossy(&res.stdout),
            String::from_utf8_lossy(&res.stderr),
        );
        assert!(res.status.success(), "{err}");
        let shown = tilt.abs() >= 2.0;
        assert_eq!(out.contains("page 1: tilted "), shown, "{tilt}: {out}");
        assert!(out.contains("2 lines, "), "{tilt}: {out}");
        let noted = err.contains("note: a tilted page stays tilted");
        assert_eq!(noted, shown, "{tilt}: {err}");
        assert!(text(&dir, "read.pdf").contains("Ottawa"), "{tilt}");
    }
}
