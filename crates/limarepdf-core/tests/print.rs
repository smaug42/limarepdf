//! What LimarePDF sends to the printer, and how it talks to `lp`. No printer is needed: `lp`
//! is replaced by a script that records its arguments and input. Every printout is laid out
//! on sheets of the paper by LimarePDF (ENH-14), so the sheets are checked against the
//! placements the model gives.

use limarepdf_core::print::{
    lp_args, place, send, Orientation, PageSetup, Paper, PrintOptions, Scale, Sides,
};
use limarepdf_core::{render, Cipher, Encryption, FieldValue, ObjectKind, Pdf, Restrictions};
use lopdf::{dictionary, Dictionary, Object, Stream};
#[cfg(unix)]
use std::path::PathBuf;

/// Letter, with the 12-point margins the user's HP reports.
fn letter() -> Paper {
    Paper {
        id: "na_letter_8.5x11in".into(),
        label: "Letter (8.5 × 11 in)".into(),
        size: (612.0, 792.0),
        margins: Some([12.0; 4]),
    }
}

fn setup(paper: Paper, orientation: Orientation, scale: Scale) -> PageSetup {
    PageSetup {
        paper,
        orientation,
        scale,
    }
}

/// Paper exactly `w` x `h` points with no margins: a page that size goes on it as it is,
/// so a sheet can be read as its page.
fn as_is(w: f32, h: f32) -> PageSetup {
    let paper = Paper {
        id: "page".into(),
        label: "page".into(),
        size: (w, h),
        margins: Some([0.0; 4]),
    };
    setup(paper, Orientation::Automatic, Scale::ShrinkOversized)
}

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

/// A sheet drawn at 72 dpi, a pixel a point.
struct Pic {
    w: usize,
    h: usize,
    rgb: Vec<[u8; 3]>,
}

impl Pic {
    /// Colour at (x, y) points from the top-left.
    fn at(&self, x: usize, y: usize) -> [u8; 3] {
        self.rgb[y * self.w + x]
    }

    /// The box of the pixels `which` picks, in points from the sheet's bottom-left: left,
    /// bottom, right, top.
    fn inked(&self, which: impl Fn([u8; 3]) -> bool) -> Option<[f32; 4]> {
        let mut b: Option<[usize; 4]> = None;
        for y in 0..self.h {
            for x in 0..self.w {
                if which(self.at(x, y)) {
                    let r = b.get_or_insert([x, y, x, y]);
                    *r = [r[0].min(x), r[1].min(y), r[2].max(x + 1), r[3].max(y + 1)];
                }
            }
        }
        let [x0, y0, x1, y1] = b?;
        let h = self.h as f32;
        Some([x0 as f32, h - y1 as f32, x1 as f32, h - y0 as f32])
    }
}

/// Sheet `i` of `pdf` as hayro draws it.
fn drawn(pdf: &[u8], i: usize) -> Pic {
    let hp = render::load(pdf.to_vec(), None).unwrap();
    let b = render::Renderer::new(&hp).render(i, 1.0).unwrap();
    let (px, _) = b.rgba.as_chunks::<4>();
    Pic {
        w: b.width,
        h: b.height,
        rgb: px.iter().map(|p| [p[0], p[1], p[2]]).collect(),
    }
}

/// Colour at (x, y) pt from the top-left of page 1 of `pdf`, rendered at 72 dpi.
fn pixel(pdf: Vec<u8>, x: usize, y: usize) -> [u8; 3] {
    drawn(&pdf, 0).at(x, y)
}

fn red(c: [u8; 3]) -> bool {
    c[0] > 200 && c[1] < 80 && c[2] < 80
}

fn blue(c: [u8; 3]) -> bool {
    c[2] > 200 && c[0] < 80 && c[1] < 80
}

/// Anything drawn but blue.
fn page_ink(c: [u8; 3]) -> bool {
    c.iter().any(|&v| v < 250) && !blue(c)
}

/// Add a page with full box `media`, cropped to `crop`, turned `rotate`: blue all over its
/// full box, grey over what is shown, and a red bar 10 points deep along its top as shown.
fn marked_page(p: &mut Pdf, media: [f32; 4], crop: Option<[f32; 4]>, rotate: i64) {
    let at = p.page_count();
    p.insert_blank(at, media[2] - media[0], media[3] - media[1])
        .unwrap();
    let [x0, y0, x1, y1] = crop.unwrap_or(media);
    // Turned clockwise a quarter turn, a page shows its left edge at the top.
    let bar = match rotate {
        90 => [x0, y0, 10.0, y1 - y0],
        180 => [x0, y0, x1 - x0, 10.0],
        270 => [x1 - 10.0, y0, 10.0, y1 - y0],
        _ => [x0, y1 - 10.0, x1 - x0, 10.0],
    };
    let n = |v: [f32; 4]| v.map(|x| x.to_string()).join(" ");
    let mb = [media[0], media[1], media[2] - media[0], media[3] - media[1]];
    let content = format!(
        "0 0 1 rg {} re f 0.8 g {} re f 1 0 0 rg {} re f",
        n(mb),
        n([x0, y0, x1 - x0, y1 - y0]),
        n(bar)
    );
    let cs = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.into_bytes()));
    let page = p.page_ids()[at];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", cs);
    d.set("MediaBox", media.map(Object::Real).to_vec());
    if let Some(c) = crop {
        d.set("CropBox", c.map(Object::Real).to_vec());
    }
    d.set("Rotate", rotate);
}

/// Pages of every kind the model has to place: upright, wide, turned each way by the page
/// itself, cropped, and much larger than the paper.
fn marked_pages() -> Pdf {
    let mut p = Pdf::new();
    marked_page(&mut p, [0.0, 0.0, 300.0, 400.0], None, 0);
    marked_page(&mut p, [0.0, 0.0, 500.0, 300.0], None, 0);
    marked_page(&mut p, [0.0, 0.0, 400.0, 600.0], None, 90);
    marked_page(&mut p, [0.0, 0.0, 500.0, 300.0], None, 270);
    let crop = [50.0, 50.0, 350.0, 250.0];
    marked_page(&mut p, [0.0, 0.0, 400.0, 400.0], Some(crop), 180);
    // A0, with its full box away from the origin.
    marked_page(&mut p, [100.0, 100.0, 2484.0, 3470.0], None, 0);
    p
}

/// `a` within 1.5 points of `b` (pixels straddling an edge are partly drawn).
fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1.5)
}

/// `r` cut to the sheet `size`, or `None` when none of it is on it.
fn on_sheet(r: [f32; 4], size: (f32, f32)) -> Option<[f32; 4]> {
    let c = [
        r[0].max(0.0),
        r[1].max(0.0),
        r[2].min(size.0),
        r[3].min(size.1),
    ];
    (c[2] - c[0] > 1.0 && c[3] - c[1] > 1.0).then_some(c)
}

/// Each sheet of `sheets`, as `draw` draws it, holds its page of `doc` where [`place`] puts
/// it: the page's shown area grey (nothing of what its crop box leaves out, which is blue),
/// and its top's red bar along the top of its box, or along its left when turned.
fn check_sheets(doc: &Pdf, sheets: &[u8], setup: &PageSetup, draw: impl Fn(&[u8], usize) -> Pic) {
    for i in 0..doc.page_count() {
        let placed = place(doc.page_size(i).unwrap(), setup);
        let pic = draw(sheets, i);
        assert_eq!((pic.w, pic.h), (612, 792), "sheet {}", i + 1);
        let what = format!("page {} at {:?}", i + 1, placed);
        let page = pic.inked(page_ink).expect("the page drawn");
        let want = on_sheet(placed.rect, placed.sheet).unwrap();
        assert!(near(page, want), "{what}: drawn at {page:?}");
        assert_eq!(pic.inked(blue), None, "{what}: past its crop box");
        let [x0, y0, x1, y1] = placed.rect;
        let bar = 10.0 * placed.scale;
        let want = match placed.turned {
            true => [x0, y0, x0 + bar, y1],
            false => [x0, y1 - bar, x1, y1],
        };
        match on_sheet(want, placed.sheet) {
            Some(want) => {
                let got = pic.inked(red).expect("the top drawn");
                assert!(near(got, want), "{what}: its top at {got:?}, not {want:?}");
            }
            None => assert_eq!(pic.inked(red), None, "{what}: its top shown"),
        }
    }
}

/// The page setups the sheets are checked with: every scale, and every orientation.
fn setups() -> Vec<PageSetup> {
    let mut v = Vec::new();
    for scale in [
        Scale::ShrinkOversized,
        Scale::Fit,
        Scale::ActualSize,
        Scale::Custom(50),
        Scale::Custom(400),
    ] {
        v.push(setup(letter(), Orientation::Automatic, scale));
    }
    for o in [Orientation::Portrait, Orientation::Landscape] {
        v.push(setup(letter(), o, Scale::ShrinkOversized));
    }
    let mut unknown = letter();
    unknown.margins = None;
    v.push(setup(unknown, Orientation::Automatic, Scale::Fit));
    v
}

#[test]
fn each_page_is_on_its_sheet_where_the_model_places_it() {
    let doc = marked_pages();
    let all: Vec<usize> = (0..doc.page_count()).collect();
    for s in setups() {
        let mut sheets = doc.print_sheets(&all, &s).unwrap();
        assert_eq!(widths(&sheets), [612.0; 6], "{s:?}");
        check_sheets(&doc, &sheets.to_bytes().unwrap(), &s, drawn);
    }
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
    let s = setup(letter(), Orientation::Portrait, Scale::ActualSize);
    let data = p.print_data(&[2, 0], &s, false).unwrap();
    // CUPS has no password, so the copy it gets must open without one.
    let sent = Pdf::from_bytes(&data, None).unwrap();
    assert_eq!(widths(&sent), [612.0, 612.0]);
    // Vector content stays vector: the page is a drawing on its sheet, not an image.
    assert_eq!(kinds(&sent, 1), [ObjectKind::Form]);
    // The square, page 1's (50, 50)-(250, 150), on the second sheet: the 300 x 200 page
    // centred, (156, 296) from the sheet's bottom-left. 306 x 396 from the top-left is in it.
    let c = drawn(&data, 1).at(306, 396);
    assert!(red(c), "{c:?}");
    // The document being printed is unchanged.
    assert_eq!(p.page_count(), 3);
    assert!(p.print_data(&[], &s, false).is_err());
}

#[test]
fn printing_leaves_the_document_as_it_was() {
    let p = marked_pages();
    let before = p.clone().to_bytes().unwrap();
    let all: Vec<usize> = (0..p.page_count()).collect();
    let s = setup(letter(), Orientation::Automatic, Scale::Fit);
    p.print_sheets(&all, &s).unwrap();
    p.print_data(&all, &s, false).unwrap();
    assert!(
        p.clone().to_bytes().unwrap() == before,
        "printing changed it"
    );
}

#[test]
fn a_page_that_leaves_its_drawing_turned_is_placed_as_it_is_shown() {
    // A page whose content turns the page upside down and never turns it back, as some PDF
    // makers write them (PDF allows it): a red square at its top-left.
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let page = p.page_ids()[0];
    let cs = p.doc.add_object(Stream::new(
        Dictionary::new(),
        b"1 0 0 -1 0 200 cm 1 0 0 rg 10 10 20 20 re f".to_vec(),
    ));
    p.doc.get_dictionary_mut(page).unwrap().set("Contents", cs);
    let s = setup(letter(), Orientation::Portrait, Scale::ActualSize);
    let sheets = p.print_data(&[0], &s, false).unwrap();
    // The page centred, its top-left at (156, 296) from the sheet's top-left.
    let got = drawn(&sheets, 0).inked(red).unwrap();
    assert!(near(got, [166.0, 466.0, 186.0, 486.0]), "{got:?}");
    // Nothing is left for CUPS to draw over it: no annotations, no form.
    let sent = Pdf::from_bytes(&sheets, None).unwrap();
    let page = sent.doc.get_dictionary(sent.page_ids()[0]).unwrap();
    assert!(page.get(b"Annots").is_err());
    assert!(sent.form_fields().is_empty());
}

#[test]
fn a_page_of_several_content_streams_is_one_drawing() {
    // The square drawn by three streams, the middle one compressed, as editors append them.
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let spaced = format!("1 0 0 rg{}10 10 20 20 re", " ".repeat(200));
    let mut middle = Stream::new(Dictionary::new(), spaced.into_bytes());
    middle.compress().unwrap();
    assert!(middle.dict.has(b"Filter"), "not compressed");
    let ids: Vec<Object> = [
        Stream::new(Dictionary::new(), b"q".to_vec()),
        middle,
        Stream::new(Dictionary::new(), b"f Q".to_vec()),
    ]
    .into_iter()
    .map(|s| p.doc.add_object(s).into())
    .collect();
    let page = p.page_ids()[0];
    p.doc.get_dictionary_mut(page).unwrap().set("Contents", ids);
    let sheets = p.print_data(&[0], &as_is(300.0, 200.0), false).unwrap();
    // At the page's own place: (10, 10)-(30, 30) from the bottom-left.
    let got = drawn(&sheets, 0).inked(red).unwrap();
    assert!(near(got, [10.0, 10.0, 30.0, 30.0]), "{got:?}");
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

    // Upright, so that its words read across.
    let s = setup(letter(), Orientation::Portrait, Scale::ShrinkOversized);
    let sent = Pdf::from_bytes(&p.print_data(&[0], &s, false).unwrap(), None).unwrap();
    // No fields left for CUPS to drop; the printed one is part of the sheet.
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
    let s = setup(letter(), Orientation::Automatic, Scale::ShrinkOversized);
    let sent = Pdf::from_bytes(&p.print_data(&[0], &s, false).unwrap(), None).unwrap();
    assert!(sent.form_fields().is_empty());
    let text = sent.page_text(0);
    assert!(text.contains("Oslo"), "{text:?}");
}

#[test]
fn filled_fields_that_have_no_drawing_are_drawn_both_ways() {
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
    let p = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert!(p.unprintable_fields(&[0]).is_empty());

    // Dark pixels in the field's box (y 80-120 from the top), as sent both ways.
    let dark = |pdf: Vec<u8>| {
        let pic = drawn(&pdf, 0);
        (80..120)
            .flat_map(|y| (20..280).map(move |x| (x, y)))
            .filter(|&(x, y)| pic.at(x, y).iter().all(|&c| c < 128))
            .count()
    };
    let s = as_is(300.0, 200.0);
    assert!(
        dark(p.print_data(&[0], &s, false).unwrap()) > 0,
        "not drawn"
    );
    assert!(
        dark(p.print_data(&[0], &s, true).unwrap()) > 0,
        "blank as images"
    );
    // The document being printed keeps no drawing of LimarePDF's.
    assert!(p.doc.get_dictionary(w).unwrap().get(b"AP").is_err());
}

#[test]
fn as_image_sends_one_picture_per_sheet() {
    let p = three_pages();
    let s = setup(letter(), Orientation::Portrait, Scale::ActualSize);
    let data = p.print_data(&[0, 1], &s, true).unwrap();
    let sent = Pdf::from_bytes(&data, None).unwrap();
    assert_eq!(widths(&sent), [612.0, 612.0]);
    for i in 0..2 {
        assert_eq!(kinds(&sent, i), [ObjectKind::Image], "sheet {}", i + 1);
    }
    // The square survives, in red, where it is on the vector sheet.
    let c = pixel(data, 306, 396);
    assert!(red(c), "{c:?}");
}

/// A blue square annotation at (100, 50)-(200, 150) with these flags and subtype.
fn annotation(p: &mut Pdf, flags: i64, subtype: &str) {
    let ap = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
        },
        b"0 0 1 rg 0 0 10 10 re f".to_vec(),
    ));
    let rect: Vec<Object> = [100, 50, 200, 150].map(Object::from).to_vec();
    let a = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => subtype, "Rect" => rect, "F" => flags,
        "AP" => dictionary! { "N" => ap },
    });
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Annots", vec![Object::Reference(a)]);
}

#[test]
fn only_annotations_meant_for_print_are_printed() {
    // Flag 4 = Print, 32 = NoView: printed, though not shown on screen; 2 = Hidden. A pop-up
    // note prints only as the comment it belongs to.
    let cases = [
        (4, "Square", true),
        (4 | 32, "Square", true),
        (0, "Square", false),
        (4 | 2, "Square", false),
        (4, "Popup", false),
    ];
    for (flags, subtype, printed) in cases {
        let mut p = three_pages();
        annotation(&mut p, flags, subtype);
        for as_image in [false, true] {
            let data = p.print_data(&[0], &as_is(300.0, 200.0), as_image).unwrap();
            let c = pixel(data, 150, 100);
            assert_eq!(
                blue(c),
                printed,
                "{subtype} {flags} as image {as_image}: {c:?}"
            );
        }
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
        ..Default::default()
    };
    let want = "-d Office -n 2 -o media=A4 -o print-scaling=none -o collate=true \
                -o sides=two-sided-long-edge -t report.pdf";
    assert_eq!(lp_args(&o, "A4").join(" "), want);
    // The default printer, one copy: no -d, no collate, explicitly one-sided, and the sheets
    // never scaled by CUPS.
    let o = PrintOptions::default();
    assert_eq!(
        lp_args(&o, "na_letter_8.5x11in").join(" "),
        "-n 1 -o media=na_letter_8.5x11in -o print-scaling=none -o sides=one-sided"
    );
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
    let p = three_pages();
    let data = p.print_data(&[1], &as_is(400.0, 200.0), false).unwrap();
    let o = PrintOptions {
        printer: Some("Office".into()),
        ..Default::default()
    };
    let args = lp_args(&o, "Letter");
    let reply = send(&lp, &args, &data).unwrap();
    assert_eq!(reply, "request id is Office-7 (1 file(s))");
    assert_eq!(std::fs::read(dir.join("job.pdf")).unwrap(), data);
    let got = std::fs::read_to_string(dir.join("args.txt")).unwrap();
    assert_eq!(got.lines().collect::<Vec<_>>(), args);

    // lp's own error reaches the user.
    let o = PrintOptions {
        printer: Some("Missing".into()),
        ..Default::default()
    };
    let e = send(&lp, &lp_args(&o, "Letter"), &data).unwrap_err();
    assert!(e.to_string().contains("does not exist"), "{e}");
}

#[test]
fn missing_lp_is_explained() {
    let e = send("limarepdf-no-such-lp", &[], b"%PDF").unwrap_err();
    assert!(e.to_string().contains("printing needs CUPS"), "{e}");
}

/// A scratch folder of this test's own.
#[cfg(unix)]
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Page `i` of the PDF file `path` as Poppler draws it at 72 dpi.
#[cfg(unix)]
fn poppler(path: &std::path::Path, i: usize) -> Pic {
    let stem = path.with_extension(format!("p{i}"));
    let png = path.with_extension(format!("p{i}.png"));
    let n = (i + 1).to_string();
    let ok = std::process::Command::new("pdftoppm")
        .args(["-r", "72", "-singlefile", "-png", "-f", &n, "-l", &n])
        .arg(path)
        .arg(&stem)
        .status()
        .expect("pdftoppm")
        .success();
    assert!(ok, "pdftoppm failed");
    let img = image::open(png).unwrap().to_rgb8();
    Pic {
        w: img.width() as usize,
        h: img.height() as usize,
        rgb: img.pixels().map(|p| p.0).collect(),
    }
}

/// Poppler draws the sheets as the model places their pages too. Opt-in: needs `pdftoppm`.
#[cfg(unix)]
#[test]
#[ignore]
fn cross_check_sheets_with_poppler() {
    let dir = scratch("sheets-poppler");
    let doc = marked_pages();
    let all: Vec<usize> = (0..doc.page_count()).collect();
    for (n, s) in setups().iter().enumerate() {
        let path = dir.join(format!("sheets-{n}.pdf"));
        let bytes = doc.print_sheets(&all, s).unwrap().to_bytes().unwrap();
        std::fs::write(&path, &bytes).unwrap();
        check_sheets(&doc, &bytes, s, |_, i| poppler(&path, i));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// The bytes of `http://localhost:631{path}`, asked without anything but the standard
/// library.
#[cfg(unix)]
fn local_cups(path: &str) -> Option<Vec<u8>> {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect("localhost:631").ok()?;
    let ask = format!("GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n");
    s.write_all(ask.as_bytes()).ok()?;
    let mut got = Vec::new();
    s.read_to_end(&mut got).ok()?;
    let end = got.windows(4).position(|w| w == b"\r\n\r\n")?;
    let ok = got.starts_with(b"HTTP/1.1 200") || got.starts_with(b"HTTP/1.0 200");
    ok.then(|| got[end + 4..].to_vec())
}

/// CUPS passes a sheet on as it is when sent with `print-scaling=none`, as `lp_args` sends
/// it: a Letter page printed at its actual size covers its whole sheet, corner to corner,
/// after CUPS's own filter (`cupsfilter`, what CUPS gives the printer's driver), with the
/// default printer's description file. Opt-in: needs CUPS, `cupsfilter`, `pdftoppm` and a
/// printer set up; nothing is sent to the printer.
#[cfg(unix)]
#[test]
#[ignore]
fn cross_check_sheets_pass_cupsfilter_unscaled() {
    let Some(queue) = limarepdf_core::print::printers()
        .ok()
        .and_then(|v| v.into_iter().find(|p| p.default))
    else {
        eprintln!("no default printer: nothing to check");
        return;
    };
    let dir = scratch("sheets-cupsfilter");
    let Some(ppd) = local_cups(&format!("/printers/{}.ppd", queue.name)) else {
        let name = queue.name;
        eprintln!("{name} has no description file to read: nothing to check");
        return;
    };
    std::fs::write(dir.join("queue.ppd"), ppd).unwrap();
    // A Letter page with black squares in its corners.
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    let corners = "0 g 0 0 36 36 re f 576 0 36 36 re f 0 756 36 36 re f 576 756 36 36 re f";
    let cs = p
        .doc
        .add_object(Stream::new(Dictionary::new(), corners.as_bytes().to_vec()));
    let page = p.page_ids()[0];
    p.doc.get_dictionary_mut(page).unwrap().set("Contents", cs);
    let mut paper = letter();
    paper.id = "Letter".into();
    let s = setup(paper, Orientation::Automatic, Scale::ActualSize);
    let sheet = dir.join("sheet.pdf");
    std::fs::write(&sheet, p.print_data(&[0], &s, false).unwrap()).unwrap();
    let args = lp_args(&PrintOptions::default(), "Letter");
    let options: Vec<&String> = args.iter().filter(|a| a.contains('=')).collect();
    let mut cmd = std::process::Command::new("cupsfilter");
    cmd.arg("-p").arg(dir.join("queue.ppd"));
    cmd.args(["-i", "application/pdf", "-m", "application/vnd.cups-pdf"]);
    for o in options {
        cmd.args(["-o", o]);
    }
    let out = cmd.arg(&sheet).output().expect("cupsfilter");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let filtered = dir.join("filtered.pdf");
    std::fs::write(&filtered, out.stdout).unwrap();
    let pic = poppler(&filtered, 0);
    let dark = |c: [u8; 3]| c.iter().all(|&v| v < 128);
    let ink = pic.inked(dark).expect("the corners");
    let whole = near(ink, [0.0, 0.0, 612.0, 792.0]);
    assert!(whole, "the corners at {ink:?}");
    std::fs::remove_dir_all(dir).unwrap();
}
