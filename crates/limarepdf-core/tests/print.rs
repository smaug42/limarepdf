//! What LimarePDF sends to CUPS, and how it talks to `lp`. No printer is needed: `lp` is
//! replaced by a script that records its arguments and input.

use limarepdf_core::print::{lp_args, paper_in_lpoptions, send, PrintOptions, Sides};
use limarepdf_core::{render, Cipher, Encryption, FieldValue, ObjectKind, Pdf, Restrictions};
use lopdf::{dictionary, Dictionary, Object, Stream};
#[cfg(unix)]
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
fn page_content_cannot_turn_what_cups_adds() {
    // A page whose content turns the page upside down and never turns it back, as some
    // PDF makers write them (PDF allows it).
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let page = p.page_ids()[0];
    let cs = p.doc.add_object(Stream::new(
        Dictionary::new(),
        b"1 0 0 -1 0 200 cm 1 0 0 rg 10 10 20 20 re f".to_vec(),
    ));
    p.doc.get_dictionary_mut(page).unwrap().set("Contents", cs);
    let data = p.print_data(&[0], false).unwrap();

    // Draw an annotation into the sent page as CUPS does: after the page's content, without
    // restoring the graphics state first. A blue square at (100, 150)-(200, 190).
    let mut sent = Pdf::from_bytes(&data, None).unwrap();
    let page = sent.page_ids()[0];
    let mark = sent.doc.add_object(Stream::new(
        Dictionary::new(),
        b"0 0 1 rg 100 150 100 40 re f".to_vec(),
    ));
    let mut contents = match sent.doc.get_dictionary(page).unwrap().get(b"Contents") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(c) => vec![c.clone()],
        Err(_) => Vec::new(),
    };
    contents.push(Object::Reference(mark));
    sent.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Contents", contents);

    // It lands where it was meant to, near the top (y = 30 from the top), not turned
    // into the bottom half.
    let c = pixel(sent.to_bytes().unwrap(), 150, 30);
    assert!(c[2] > 200 && c[0] < 60, "{c:?}");
}

#[test]
fn form_values_are_sent_drawn_in() {
    // Two filled text fields: one printed (flag 4), one only shown on screen.
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let field = |name: &str, y: i64, flags: i64| {
        let rect: Vec<Object> = [20, y, 280, y + 30].map(Object::from).to_vec();
        dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "F" => flags,
            "T" => Object::string_literal(name), "Rect" => rect,
        }
    };
    let ids = [
        p.doc.add_object(field("Name", 140, 4)),
        p.doc.add_object(field("Note", 40, 0)),
    ];
    let refs: Vec<Object> = ids.iter().map(|&i| i.into()).collect();
    let page = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", refs.clone());
    let helv = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let form = p.doc.add_object(dictionary! {
        "Fields" => refs,
        "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
    });
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
    p.set_field("Name", FieldValue::Text("Alice Example".into()))
        .unwrap();
    p.set_field("Note", FieldValue::Text("Screen only".into()))
        .unwrap();

    let sent = Pdf::from_bytes(&p.print_data(&[0], false).unwrap(), None).unwrap();
    // No fields left for CUPS to drop; the printed one is part of the page.
    assert!(sent.form_fields().is_empty());
    let text = sent.page_text(0);
    assert!(text.contains("Alice Example"), "{text:?}");
    assert!(!text.contains("Screen only"), "{text:?}");
    // The document being printed keeps its form.
    assert_eq!(p.form_fields().len(), 2);
}

#[test]
fn fields_that_would_print_empty_are_named_before_printing() {
    // Three filled fields with no drawing: two printed (flag 4), one only shown on screen.
    // LimarePDF can't draw the Hebrew values (written right to left); it can draw the
    // Latin one.
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    p.insert_blank(1, 300.0, 200.0).unwrap();
    let hebrew = || {
        let mut b = vec![0xFE, 0xFF];
        b.extend("שלום".encode_utf16().flat_map(u16::to_be_bytes));
        Object::String(b, lopdf::StringFormat::Hexadecimal)
    };
    let field = |name: &str, y: i64, flags: i64, value: Object| {
        let rect: Vec<Object> = [20, y, 280, y + 30].map(Object::from).to_vec();
        dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "F" => flags,
            "T" => Object::string_literal(name), "Rect" => rect, "V" => value,
        }
    };
    let ids = [
        p.doc.add_object(field("Name", 140, 4, hebrew())),
        p.doc.add_object(field("Note", 90, 0, hebrew())),
        p.doc
            .add_object(field("City", 40, 4, Object::string_literal("Oslo"))),
    ];
    let refs: Vec<Object> = ids.iter().map(|&i| i.into()).collect();
    let page = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", refs.clone());
    let form = p.doc.add_object(dictionary! {
        "Fields" => refs, "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
    });
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
    // Saving numbers an object of its own, so each save is of a copy.
    let before = p.clone().to_bytes().unwrap();

    assert_eq!(p.unprintable_fields(&[0, 1]), ["Name"]);
    assert!(p.unprintable_fields(&[1]).is_empty());
    assert!(
        p.clone().to_bytes().unwrap() == before,
        "asking changed the document"
    );

    // Printing anyway sends the page without that value, and with the others drawn in.
    let sent = Pdf::from_bytes(&p.print_data(&[0], false).unwrap(), None).unwrap();
    assert!(sent.form_fields().is_empty());
    let text = sent.page_text(0);
    assert!(text.contains("Oslo"), "{text:?}");
}

#[test]
fn as_image_draws_filled_fields_that_have_no_drawing() {
    // A printed field with a value and no drawing, in a file just opened, as the command
    // line prints it (the app draws such fields when it opens a file; the command line
    // doesn't).
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let rect: Vec<Object> = [20, 80, 280, 120].map(Object::from).to_vec();
    let w = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "F" => 4,
        "T" => Object::string_literal("City"), "V" => Object::string_literal("MMMMMMMM"),
        "Rect" => rect,
    });
    let page = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", vec![w.into()]);
    let form = p.doc.add_object(dictionary! {
        "Fields" => vec![w.into()], "DA" => Object::string_literal("/Helv 20 Tf 0 g"),
    });
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
    let mut p = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert!(p.unprintable_fields(&[0]).is_empty());

    // Dark pixels in the field's box (y 80-120 from the top), as sent both ways.
    let dark = |pdf: Vec<u8>| {
        let hp = render::load(pdf, None).unwrap();
        let b = render::Renderer::new(&hp).render(0, 1.0).unwrap();
        (80..120)
            .flat_map(|y| (20..280).map(move |x| (y * b.width + x) * 4))
            .filter(|&i| b.rgba[i] < 128 && b.rgba[i + 1] < 128 && b.rgba[i + 2] < 128)
            .count()
    };
    assert!(dark(p.print_data(&[0], false).unwrap()) > 0, "not drawn");
    assert!(
        dark(p.print_data(&[0], true).unwrap()) > 0,
        "blank as images"
    );
    // The document being printed keeps no drawing of LimarePDF's.
    assert!(p.doc.get_dictionary(w).unwrap().get(b"AP").is_err());
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
    let want =
        "-d Office -n 2 -o media=A4 -o collate=true -o sides=two-sided-long-edge -t report.pdf";
    assert_eq!(lp_args(&o, Some("A4")).join(" "), want);
    // The default printer, one copy, paper unknown: no -d, no media, no collate, explicitly
    // one-sided.
    let o = PrintOptions::default();
    assert_eq!(lp_args(&o, None).join(" "), "-n 1 -o sides=one-sided");
}

#[test]
fn default_paper_from_lpoptions() {
    // As an HP LaserJet's queue lists its options.
    let text = "PageSize/Media Size: A6 A5 Postcard *Letter Letter.Duplex A4 Custom.WIDTHxHEIGHT\n\
                Duplex/Double-Sided Printing: DuplexNoTumble DuplexTumble *None\n\
                ColorModel/Output Mode: *Gray\n";
    assert_eq!(paper_in_lpoptions(text).as_deref(), Some("Letter"));
    let a4 = "InputSlot/Media Source: *Auto Manual\nPageSize/Page Size: Letter *A4\n";
    assert_eq!(paper_in_lpoptions(a4).as_deref(), Some("A4"));
    // No mark, a custom size, or no PageSize line at all: leave the paper to the printer.
    assert_eq!(paper_in_lpoptions("PageSize/Media Size: Letter A4\n"), None);
    assert_eq!(
        paper_in_lpoptions("PageSize/Media Size: Letter *Custom.WIDTHxHEIGHT\n"),
        None
    );
    assert_eq!(
        paper_in_lpoptions("Duplex/Double-Sided Printing: *None\n"),
        None
    );
    assert_eq!(paper_in_lpoptions(""), None);
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
    let args = lp_args(
        &PrintOptions {
            printer: Some("Office".into()),
            ..Default::default()
        },
        Some("Letter"),
    );
    let reply = send(&lp, &args, &data).unwrap();
    assert_eq!(reply, "request id is Office-7 (1 file(s))");
    assert_eq!(std::fs::read(dir.join("job.pdf")).unwrap(), data);
    let got = std::fs::read_to_string(dir.join("args.txt")).unwrap();
    assert_eq!(got.lines().collect::<Vec<_>>(), args);

    // lp's own error reaches the user.
    let args = lp_args(
        &PrintOptions {
            printer: Some("Missing".into()),
            ..Default::default()
        },
        None,
    );
    let e = send(&lp, &args, &data).unwrap_err().to_string();
    assert!(e.contains("does not exist"), "{e}");
}

#[test]
fn missing_lp_is_explained() {
    let e = send("limarepdf-no-such-lp", &[], b"%PDF").unwrap_err();
    assert!(e.to_string().contains("printing needs CUPS"), "{e}");
}
