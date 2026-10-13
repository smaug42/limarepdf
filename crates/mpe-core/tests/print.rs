//! What MPE sends to CUPS, and how it talks to `lp`. No printer is needed: `lp` is
//! replaced by a script that records its arguments and input.

use lopdf::{dictionary, Dictionary, Object, Stream};
use mpe_core::print::{lp_args, send, PrintOptions, Sides};
use mpe_core::{render, Cipher, Encryption, ObjectKind, Pdf, Restrictions};
use std::path::PathBuf;

/// Pages of three widths, the first with a red square at (50, 50)-(250, 150).
fn three_pages() -> Pdf {
    let mut p = Pdf::new();
    for (i, w) in [300.0, 400.0, 500.0].into_iter().enumerate() {
        p.insert_blank(i, w, 200.0).unwrap();
    }
    let page = p.page_ids()[0];
    let cs = p.doc.add_object(Stream::new(
        Dictionary::new(),
        b"1 0 0 rg 50 50 200 100 re f".to_vec(),
    ));
    p.doc.get_dictionary_mut(page).unwrap().set("Contents", cs);
    p
}

fn kinds(p: &Pdf, page: usize) -> Vec<ObjectKind> {
    let objs = p.page_objects(page).unwrap();
    objs.iter().map(|o| o.kind).collect()
}

fn widths(p: &Pdf) -> Vec<f32> {
    (0..p.page_count())
        .map(|i| p.page_size(i).unwrap().0.round())
        .collect()
}

/// Colour at (x, y) pt from the top-left of page 1 of `pdf`, rendered at 72 dpi.
fn pixel(pdf: Vec<u8>, x: usize, y: usize) -> [u8; 3] {
    let hp = render::load(pdf, None).unwrap();
    let b = render::Renderer::new(&hp).render(0, 1.0).unwrap();
    let i = (y * b.width + x) * 4;
    [b.rgba[i], b.rgba[i + 1], b.rgba[i + 2]]
}

#[test]
fn sends_the_chosen_pages_unencrypted() {
    let mut p = three_pages();
    p.set_encryption(Some(Encryption {
        user_password: "user".into(),
        owner_password: "owner".into(),
        cipher: Cipher::Aes128,
        restrictions: Restrictions::default(),
    }));
    let data = p.print_data(&[2, 0], false).unwrap();
    // CUPS has no password, so the copy it gets must open without one.
    let sent = Pdf::from_bytes(&data, None).unwrap();
    assert_eq!(widths(&sent), [500.0, 300.0]);
    // Vector content stays vector: the square is a path, not an image.
    assert_eq!(kinds(&sent, 1), [ObjectKind::Path]);
    // The document being printed is unchanged.
    assert_eq!(p.page_count(), 3);
    assert!(p.print_data(&[], false).is_err());
}

#[test]
fn as_image_sends_one_picture_per_page() {
    let mut p = three_pages();
    let data = p.print_data(&[0, 1], true).unwrap();
    let sent = Pdf::from_bytes(&data, None).unwrap();
    assert_eq!(widths(&sent), [300.0, 400.0]);
    for i in 0..2 {
        assert_eq!(kinds(&sent, i), [ObjectKind::Image], "page {}", i + 1);
    }
    // The square survives, in red, 150 x 100 pt from the top-left being inside it.
    let c = pixel(data, 150, 100);
    assert!(c[0] > 200 && c[1] < 60 && c[2] < 60, "{c:?}");
}

/// A blue square annotation at (100, 50)-(200, 150) with these flags.
fn annotation(p: &mut Pdf, flags: i64) {
    let ap = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
        },
        b"0 0 1 rg 0 0 10 10 re f".to_vec(),
    ));
    let rect: Vec<Object> = [100, 50, 200, 150].map(Object::from).to_vec();
    let a = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Square", "Rect" => rect, "F" => flags,
        "AP" => dictionary! { "N" => ap },
    });
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Annots", vec![Object::Reference(a)]);
}

#[test]
fn as_image_prints_only_annotations_meant_for_print() {
    let blue = |c: [u8; 3]| c[2] > 200 && c[0] < 60;
    // Flag 4 = Print, 32 = NoView: printed, though not shown on screen.
    for (flags, printed) in [(4, true), (4 | 32, true), (0, false)] {
        let mut p = three_pages();
        annotation(&mut p, flags);
        let c = pixel(p.print_data(&[0], true).unwrap(), 150, 100);
        assert_eq!(blue(c), printed, "flags {flags}: {c:?}");
    }
}

#[test]
fn lp_arguments() {
    let o = PrintOptions {
        printer: Some("Office".into()),
        copies: 2,
        sides: Sides::LongEdge,
        as_image: false,
        title: "report.pdf".into(),
    };
    let want = "-d Office -n 2 -o collate=true -o sides=two-sided-long-edge -t report.pdf";
    assert_eq!(lp_args(&o).join(" "), want);
    // The default printer, one copy: no -d, no collate, explicitly one-sided.
    let o = PrintOptions::default();
    assert_eq!(lp_args(&o).join(" "), "-n 1 -o sides=one-sided");
}

/// A stand-in for `lp`: saves its arguments and input next to itself, prints a reply,
/// and fails when its first argument is `-d Missing`.
#[cfg(unix)]
fn fake_lp() -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("print");
    std::fs::create_dir_all(&dir).unwrap();
    let lp = dir.join("lp");
    let script = format!(
        "#!/bin/sh\n\
         if [ \"$2\" = Missing ]; then echo 'lp: The printer or class does not exist.' >&2; exit 1; fi\n\
         printf '%s\\n' \"$@\" > '{d}/args.txt'\n\
         cat > '{d}/job.pdf'\n\
         echo 'request id is Office-7 (1 file(s))'\n",
        d = dir.display()
    );
    std::fs::write(&lp, script).unwrap();
    std::fs::set_permissions(&lp, std::fs::Permissions::from_mode(0o755)).unwrap();
    lp
}

#[cfg(unix)]
#[test]
fn sends_the_document_to_lp() {
    let lp = fake_lp();
    let dir = lp.parent().unwrap();
    let mut p = three_pages();
    let data = p.print_data(&[1], false).unwrap();
    let args = lp_args(&PrintOptions {
        printer: Some("Office".into()),
        ..Default::default()
    });
    let reply = send(&lp, &args, &data).unwrap();
    assert_eq!(reply, "request id is Office-7 (1 file(s))");
    assert_eq!(std::fs::read(dir.join("job.pdf")).unwrap(), data);
    let got = std::fs::read_to_string(dir.join("args.txt")).unwrap();
    assert_eq!(got.lines().collect::<Vec<_>>(), args);

    // lp's own error reaches the user.
    let args = lp_args(&PrintOptions {
        printer: Some("Missing".into()),
        ..Default::default()
    });
    let e = send(&lp, &args, &data).unwrap_err().to_string();
    assert!(e.contains("does not exist"), "{e}");
}

#[test]
fn missing_lp_is_explained() {
    let e = send("mpe-no-such-lp", &[], b"%PDF").unwrap_err();
    assert!(e.to_string().contains("printing needs CUPS"), "{e}");
}
