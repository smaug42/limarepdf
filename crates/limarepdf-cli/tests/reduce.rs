//! `reduce` (ENH-19): pictures scaled down to the quality chosen, what was left said, the
//! sizes before and after; a signed document refused.

use limarepdf_core::render::{encode, Bitmap, ImageFormat};
use limarepdf_core::Pdf;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("reduce")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_limarepdf-cli"))
        .current_dir(dir)
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .args(args)
        .output()
        .unwrap()
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let res = run(dir, args);
    let err = String::from_utf8_lossy(&res.stderr).into_owned();
    assert!(res.status.success(), "{args:?}: {err}");
    String::from_utf8_lossy(&res.stdout).into_owned()
}

/// `in.pdf`: one page showing a 1,200 × 800 photo-like picture 4 inches wide (300 ppi).
fn photo_pdf(dir: &Path) {
    let (w, h) = (1200, 800);
    let rgba = (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            let b = (x / 4 * y / 4) % 200;
            [(x / 5) as u8, (y / 4) as u8, b as u8, 255]
        })
        .collect();
    let photo = Bitmap {
        width: w,
        height: h,
        rgba,
    };
    let png = encode(&photo, ImageFormat::Png, 72.0).unwrap();
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    p.add_image(0, [72.0, 200.0, 360.0, 392.0], &png).unwrap();
    std::fs::write(dir.join("in.pdf"), p.to_bytes().unwrap()).unwrap();
}

/// The width of the widest picture in `name`, in pixels.
fn picture_width(dir: &Path, name: &str) -> i64 {
    let p = Pdf::open(dir.join(name), None).unwrap();
    let widths = p.doc.objects.values().filter_map(|o| {
        let s = o.as_stream().ok()?;
        let image = s.dict.get(b"Subtype").ok()?.as_name().ok()? == b"Image";
        image.then(|| s.dict.get(b"Width").ok()?.as_i64().ok())?
    });
    widths.max().unwrap()
}

fn size(dir: &Path, name: &str) -> u64 {
    std::fs::metadata(dir.join(name)).unwrap().len()
}

#[test]
fn reduces_the_pictures_and_says_what_it_did() {
    let dir = folder("web");
    photo_pdf(&dir);
    let web = ["reduce", "in.pdf", "-o", "out.pdf", "--quality", "web"];
    let out = ok(&dir, &web);
    assert!(out.contains("pictures made smaller: 1, "), "{out}");
    assert!(out.contains("wrote out.pdf: "), "{out}");
    assert_eq!(picture_width(&dir, "out.pdf"), 600);
    assert!(size(&dir, "out.pdf") * 2 < size(&dir, "in.pdf"));

    // Already at Web's resolution: kept, and said so.
    let again = ["reduce", "out.pdf", "-o", "again.pdf", "--quality", "web"];
    let out = ok(&dir, &again);
    assert!(out.contains("pictures made smaller: 0, "), "{out}");
    let kept = "kept as they are: 1, already at or under the resolution";
    assert!(out.contains(kept), "{out}");

    // High fidelity: the pictures as they are, the file saved compactly.
    let hifi = "reduce in.pdf -o hifi.pdf --quality high-fidelity";
    let out = ok(&dir, &hifi.split(' ').collect::<Vec<_>>());
    assert!(!out.contains("pictures"), "{out}");
    assert_eq!(picture_width(&dir, "hifi.pdf"), 1200);

    let zero = ["reduce", "in.pdf", "-o", "x.pdf", "--jpeg-quality", "0"];
    assert!(!run(&dir, &zero).status.success());
}

#[test]
fn a_signed_document_is_refused() {
    let dir = folder("signed");
    photo_pdf(&dir);
    let p12 = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../limarepdf-core/tests/data/signatures/ids/p256.p12"
    );
    std::fs::copy(p12, dir.join("p256.p12")).unwrap();
    std::fs::write(dir.join("pw"), "mpe-test-password\n").unwrap();
    let import = "id import p256.p12 --password-file pw --id-folder ids";
    ok(&dir, &import.split(' ').collect::<Vec<_>>());
    // A stroke of ink on white paper.
    let (w, h) = (40, 20);
    let rgba = (0..w * h)
        .flat_map(|i| {
            let stroke = (5..35).contains(&(i % w)) && (8..12).contains(&(i / w));
            if stroke {
                [20, 20, 30, 255]
            } else {
                [255, 255, 255, 255]
            }
        })
        .collect();
    let ink = Bitmap {
        width: w,
        height: h,
        rgba,
    };
    let png = encode(&ink, ImageFormat::Png, 72.0).unwrap();
    std::fs::write(dir.join("ink.png"), png).unwrap();
    let sign = "sign in.pdf -o signed.pdf --id 1 --page 1 --rect 20,20,200,60 --image ink.png \
                --password-file pw --id-folder ids";
    ok(&dir, &sign.split_whitespace().collect::<Vec<_>>());
    let res = run(&dir, &["reduce", "signed.pdf", "-o", "out.pdf"]);
    let err = String::from_utf8_lossy(&res.stderr);
    assert!(!res.status.success());
    let refused = "Signed documents keep exactly what was signed, so they can't be made smaller.";
    assert!(err.contains(refused), "{err}");
    assert!(!dir.join("out.pdf").exists());
}
