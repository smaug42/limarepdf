mod common;

use common::saved::{tools, Saved};
use limarepdf_core::{render, ObjectKind, Pdf};
use lopdf::content::Content;
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

const W: f32 = 300.0;
const H: f32 = 200.0;

/// A document with one 300×200 pt page per entry, each drawing that content with `/F1`
/// (Helvetica, no widths in the file).
fn make(pages: &[&str]) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..pages.len() {
        p.insert_blank(i, W, H).unwrap();
    }
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    for (i, c) in pages.iter().enumerate() {
        let page = p.page_ids()[i];
        let s = p
            .doc
            .add_object(Stream::new(Dictionary::new(), c.as_bytes().to_vec()));
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", Object::Reference(s));
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
    }
    p
}

fn content(p: &Pdf, page: usize) -> String {
    let id = p.page_ids()[page];
    String::from_utf8_lossy(&p.doc.get_page_content(id)).into_owned()
}

/// Pictures in the saved file as written, whether anything still uses them or not.
fn images_in_file(p: &mut Pdf) -> usize {
    Saved::of(p)
        .doc
        .objects
        .values()
        .filter(|o| {
            o.as_stream().is_ok_and(|s| {
                s.dict.get(b"Subtype").and_then(|x| x.as_name()).ok() == Some(b"Image")
            })
        })
        .count()
}

/// Every picture in the saved file as written: its last filter, components and decoded
/// samples (JPEGs decoded to 8-bit grey or RGB).
fn pictures(p: &mut Pdf) -> Vec<(String, usize, Vec<u8>)> {
    let saved = Saved::of(p);
    let mut out = Vec::new();
    for o in saved.doc.objects.values() {
        let Ok(s) = o.as_stream() else { continue };
        if s.dict.get(b"Subtype").and_then(|x| x.as_name()).ok() != Some(b"Image") {
            continue;
        }
        let filter = s
            .filters()
            .ok()
            .and_then(|f| f.last().map(|n| String::from_utf8_lossy(n).into_owned()))
            .unwrap_or_default();
        let gray = s.dict.get(b"ColorSpace").and_then(|x| x.as_name()).ok() != Some(b"DeviceRGB");
        if filter == "DCTDecode" {
            let img = image::load_from_memory(&s.content).unwrap();
            let (n, data) = if gray {
                (1, img.to_luma8().into_raw())
            } else {
                (3, img.to_rgb8().into_raw())
            };
            out.push((filter, n, data));
        } else {
            out.push((
                filter,
                if gray { 1 } else { 3 },
                s.decompressed_content().unwrap(),
            ));
        }
    }
    out
}

fn render_page(p: &mut Pdf) -> render::Bitmap {
    let bytes = p.snapshot().unwrap();
    let doc = render::load(bytes, None).unwrap();
    render::Renderer::new(&doc).render(0, 1.0).unwrap()
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.05)
}

#[test]
fn removes_the_marked_characters_and_keeps_the_rest_in_place() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Name: Zebra Quokka here) Tj ET"]);
    let here = p.find_text_boxes(0, "here").unwrap();
    let name = p.find_text_boxes(0, "Name:").unwrap();
    let marks = p.find_redaction_marks("zebra quokka").unwrap();
    assert_eq!(marks.len(), 1);

    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(rep.elsewhere.is_empty(), "{:?}", rep.elsewhere);
    assert_eq!(rep.pages.len(), 1);
    assert_eq!(rep.pages[0].text, vec!["Zebra Quokka".to_string()]);
    assert_eq!(rep.pages[0].chars, 12);

    let text = p.page_text(0);
    assert!(text.contains("Name:") && text.contains("here"), "{text}");
    assert!(!text.to_lowercase().contains("zebra"), "{text}");
    assert!(near(p.find_text_boxes(0, "here").unwrap()[0], here[0]));
    assert!(near(p.find_text_boxes(0, "Name:").unwrap()[0], name[0]));
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["Zebra", "Quokka", "ebra", "uokk"]);
    saved.assert_kept(&["Name:", "here"]);
}

#[test]
fn keeps_tj_spacing_and_later_strings_in_place() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td [(Sec) -200 (retX) 50 (Yzw)] TJ ET"]);
    let sec = p.find_text_boxes(0, "Sec").unwrap()[0];
    let yzw = p.find_text_boxes(0, "Yzw").unwrap()[0];
    let marks = p.find_redaction_marks("retX").unwrap();
    let rep = p.redact(&marks).unwrap();
    assert_eq!(rep.pages[0].chars, 4);
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(near(p.find_text_boxes(0, "Sec").unwrap()[0], sec));
    assert!(near(p.find_text_boxes(0, "Yzw").unwrap()[0], yzw));
    assert!(p.find_text_boxes(0, "ret").unwrap().is_empty());
}

#[test]
fn removes_hidden_text_and_its_alternative_text() {
    let mut p =
        make(&["/Span <</ActualText (Zebra)>> BDC BT 3 Tr /F1 20 Tf 30 100 Td (Zebra) Tj ET EMC"]);
    let marks = p.find_redaction_marks("zebra").unwrap();
    assert_eq!(marks.len(), 1, "hidden text is found");
    let rep = p.redact(&marks).unwrap();
    assert_eq!(rep.pages[0].chars, 5);
    assert!(!content(&p, 0).contains("ActualText"));
    Saved::of(&mut p).assert_gone(&["Zebra"]);
}

#[test]
fn removes_drawings_under_a_mark_and_clips_those_crossing_it() {
    // A red background across the page, and a small blue square at 100,50 (user space).
    let mut p = make(&["q 1 0 0 rg 10 10 280 180 re f Q q 0 0 1 rg 100 50 20 20 re f Q"]);
    // Displayed y runs down: user y 40..80 is displayed 120..160.
    let rep = p.redact(&[(0, [90.0, 120.0, 130.0, 160.0])]).unwrap();
    assert_eq!(rep.pages[0].paths, 1);
    assert_eq!(rep.pages[0].clipped, 1);
    let ops = Content::decode(content(&p, 0).as_bytes())
        .unwrap()
        .operations;
    let rects: Vec<Vec<f32>> = ops
        .iter()
        .filter(|o| o.operator == "re")
        .map(|o| o.operands.iter().map(|x| x.as_float().unwrap()).collect())
        .collect();
    assert!(rects.contains(&vec![10.0, 10.0, 280.0, 180.0]), "{rects:?}");
    assert!(!rects.contains(&vec![100.0, 50.0, 20.0, 20.0]), "{rects:?}");
    assert!(ops.iter().any(|o| o.operator == "W*"));

    let bytes = p.snapshot().unwrap();
    let doc = render::load(bytes, None).unwrap();
    let bmp = render::Renderer::new(&doc).render(0, 1.0).unwrap();
    let px = |x: usize, y: usize| {
        let i = (y * bmp.width + x) * 4;
        [bmp.rgba[i], bmp.rgba[i + 1], bmp.rgba[i + 2]]
    };
    assert!(
        px(110, 140)[0] < 40 && px(110, 140)[2] < 40,
        "the mark is black"
    );
    assert!(
        px(50, 50)[0] > 200 && px(50, 50)[2] < 60,
        "the background stays red outside"
    );
}

#[test]
fn blacks_out_the_covered_part_of_a_picture() {
    let mut p = make(&["BT /F1 20 Tf 30 30 Td (Caption) Tj ET"]);
    // A 10×10 picture filling 100..200 × 50..150 (displayed), translucent so it gets a
    // soft mask, which must be cut too.
    let img = image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 200, 0, 200]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    p.add_image(0, [100.0, 50.0, 200.0, 150.0], &png.into_inner())
        .unwrap();
    assert_eq!(images_in_file(&mut p), 2, "the picture and its soft mask");
    // Pixel columns 2..4 and rows 1..3 of the picture.
    let rep = p.redact(&[(0, [120.0, 60.0, 140.0, 80.0])]).unwrap();
    assert_eq!((rep.pages[0].images, rep.pages[0].images_cut), (0, 1));
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(p.page_text(0).contains("Caption"));
    let pics = pictures(&mut p);
    assert_eq!(pics.len(), 2, "the old picture and mask are gone");
    for (_, n, data) in pics {
        let px = |x: usize, y: usize| &data[(y * 10 + x) * n..(y * 10 + x + 1) * n];
        assert!(px(2, 1).iter().all(|&v| v == 0) && px(3, 2).iter().all(|&v| v == 0));
        assert!(px(1, 1).iter().any(|&v| v != 0) && px(4, 2).iter().any(|&v| v != 0));
        assert!(px(2, 3).iter().any(|&v| v != 0) && px(2, 0).iter().any(|&v| v != 0));
    }
    let bmp = render_page(&mut p);
    let at = |x: usize, y: usize| bmp.rgba[(y * bmp.width + x) * 4 + 1];
    assert!(at(170, 120) > 100, "the rest of the picture still shows");
}

#[test]
fn keeps_a_jpeg_picture_as_jpeg() {
    let mut p = make(&[""]);
    let img = image::RgbImage::from_pixel(16, 16, image::Rgb([240, 240, 240]));
    let mut jpg = std::io::Cursor::new(Vec::new());
    img.write_to(&mut jpg, image::ImageFormat::Jpeg).unwrap();
    p.add_image(0, [100.0, 50.0, 260.0, 210.0], &jpg.into_inner())
        .unwrap();
    let rep = p.redact(&[(0, [100.0, 50.0, 180.0, 130.0])]).unwrap();
    assert_eq!(rep.pages[0].images_cut, 1);
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    let pics = pictures(&mut p);
    assert_eq!(pics.len(), 1);
    assert_eq!(pics[0].0, "DCTDecode");
    let (n, data) = (pics[0].1, &pics[0].2);
    let v = |x: usize, y: usize| data[(y * 16 + x) * n] as i32;
    assert!(v(2, 2) < 20 && v(13, 13) > 200, "{} {}", v(2, 2), v(13, 13));
}

#[test]
fn turns_an_inline_image_into_a_cut_picture() {
    // A 4×4 grey inline image drawn at 50..150 (user space); the mark covers its top half.
    let mut c = b"q 100 0 0 100 50 50 cm BI /W 4 /H 4 /CS /G /BPC 8 ID ".to_vec();
    c.extend([0xC8u8; 16]);
    c.extend(b" EI Q");
    let mut p = make(&[""]);
    let page = p.page_ids()[0];
    let s = p.doc.add_object(Stream::new(Dictionary::new(), c));
    p.doc.get_dictionary_mut(page).unwrap().set("Contents", s);
    assert_eq!(p.page_objects(0).unwrap().len(), 1);
    let rep = p.redact(&[(0, [50.0, 50.0, 150.0, 100.0])]).unwrap();
    assert_eq!(rep.pages[0].images_cut, 1);
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(!content(&p, 0).contains("BI"));
    let pics = pictures(&mut p);
    assert_eq!(pics.len(), 1);
    let mut want = vec![0u8; 8];
    want.extend([0xC8; 8]);
    assert_eq!(pics[0].2, want);
}

#[test]
fn removes_a_picture_it_cannot_read_whole() {
    let mut p = make(&["q 100 0 0 100 50 50 cm /Im1 Do Q"]);
    let img = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 4, "Height" => 4,
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "JPXDecode",
        },
        vec![1, 2, 3],
    ));
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    let mut res = d.get(b"Resources").unwrap().as_dict().unwrap().clone();
    res.set("XObject", dictionary! { "Im1" => img });
    d.set("Resources", res);
    let rep = p.redact(&[(0, [60.0, 60.0, 70.0, 70.0])]).unwrap();
    assert_eq!((rep.pages[0].images, rep.pages[0].images_cut), (1, 0));
    assert_eq!(images_in_file(&mut p), 0);
}

#[test]
fn removes_a_fax_picture_too_large_to_decode() {
    // 65,535 pixels a side, 4 GB once decoded, from 8 KB: each 1 bit is a white row
    // ("vertical 0" below a white row).
    let mut p = make(&["q 100 0 0 100 50 50 cm /Im1 Do Q"]);
    let img = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 65_535, "Height" => 65_535,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 1,
            "Filter" => "CCITTFaxDecode",
            "DecodeParms" => dictionary! { "K" => -1, "Columns" => 65_535 },
        },
        vec![0xFF; 8192],
    ));
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    let mut res = d.get(b"Resources").unwrap().as_dict().unwrap().clone();
    res.set("XObject", dictionary! { "Im1" => img });
    d.set("Resources", res);
    let rep = p.redact(&[(0, [60.0, 60.0, 70.0, 70.0])]).unwrap();
    assert_eq!((rep.pages[0].images, rep.pages[0].images_cut), (1, 0));
    assert_eq!(images_in_file(&mut p), 0);
}

#[test]
fn drops_a_picture_from_shared_resources_when_no_page_draws_it() {
    let mut p = make(&[
        "q 50 0 0 50 100 50 cm /Im1 Do Q",
        "BT /F1 20 Tf 30 30 Td (Two) Tj ET",
    ]);
    let img = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        vec![0x80],
    ));
    let font = p
        .doc
        .get_dictionary(p.page_ids()[0])
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .clone();
    let mut shared = font.as_dict().unwrap().clone();
    shared.set("XObject", dictionary! { "Im1" => img });
    let shared: ObjectId = p.doc.add_object(shared);
    for page in p.page_ids() {
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("Resources", shared);
    }
    assert_eq!(images_in_file(&mut p), 1);
    let rep = p.redact(&[(0, [110.0, 100.0, 120.0, 110.0])]).unwrap();
    assert_eq!(rep.pages[0].images_cut, 1);
    // Only the blacked-out copy is left: the shared resources no longer list the old one.
    let pics = pictures(&mut p);
    assert_eq!(pics.len(), 1);
    assert_eq!(pics[0].2, vec![0]);
    assert!(p.page_text(1).contains("Two"));
}

/// Two pages drawing the same form, which says "Secret Walrus".
fn shared_form() -> Pdf {
    let mut p = make(&["q /Fm1 Do Q", "q /Fm1 Do Q"]);
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let form = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
        b"BT /F1 20 Tf 30 100 Td (Secret Walrus) Tj ET".to_vec(),
    ));
    for page in p.page_ids() {
        p.doc.get_dictionary_mut(page).unwrap().set(
            "Resources",
            dictionary! { "XObject" => dictionary! { "Fm1" => form } },
        );
    }
    p
}

#[test]
fn redacts_inside_a_shared_form_only_on_the_marked_page() {
    let mut p = shared_form();
    let marks: Vec<_> = p
        .find_redaction_marks("walrus")
        .unwrap()
        .into_iter()
        .filter(|m| m.0 == 0)
        .collect();
    assert_eq!(marks.len(), 1);
    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert_eq!(rep.pages[0].chars, 6);
    assert!(!p.page_text(0).to_lowercase().contains("walrus"));
    assert!(p.page_text(0).contains("Secret"));
    assert!(
        p.page_text(1).contains("Walrus"),
        "the other page keeps the form as it was"
    );
    assert_eq!(
        rep.elsewhere,
        vec![("walrus".to_string(), vec!["page 2".to_string()])]
    );
}

#[test]
fn removes_annotations_touching_a_mark() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Zebra) Tj ET"]);
    let page = p.page_ids()[0];
    let link = |p: &mut Pdf, r: [i64; 4]| {
        p.doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link",
            "Rect" => r.iter().map(|&x| Object::Integer(x)).collect::<Vec<_>>(),
        })
    };
    let on = link(&mut p, [30, 95, 80, 120]);
    let off = link(&mut p, [200, 10, 250, 30]);
    p.doc.get_dictionary_mut(page).unwrap().set(
        "Annots",
        vec![Object::Reference(on), Object::Reference(off)],
    );
    let marks = p.find_redaction_marks("zebra").unwrap();
    let rep = p.redact(&marks).unwrap();
    assert_eq!(rep.pages[0].annots, 1);
    let left = p
        .doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(left, vec![Object::Reference(off)]);
}

/// A tagged page saying "Keep WALRUS-TEXT" (alternative text on its structure element and
/// that element's parent), with a comment touching WALRUS-TEXT (its pop-up and a reply away
/// from it), a filled text field touching it, a thumbnail and private data. Away from it: a
/// footer, a comment and a field, which must stay.
fn tagged_and_annotated() -> Pdf {
    let mut p = make(&[
        "/P <</MCID 0>> BDC BT /F1 20 Tf 30 100 Td (Keep WALRUS-TEXT) Tj ET EMC \
         BT /F1 10 Tf 30 40 Td (Footer kept) Tj ET",
    ]);
    let page = p.page_ids()[0];
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let rect = |r: [i64; 4]| r.iter().map(|&x| Object::Integer(x)).collect::<Vec<_>>();
    let note = |p: &mut Pdf, r: [i64; 4], text: &str| {
        p.doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Text", "Rect" => rect(r), "P" => page,
            "T" => Object::string_literal("Sam"), "Contents" => Object::string_literal(text),
        })
    };
    let touching = note(&mut p, [100, 100, 120, 120], "WALRUS-NOTE");
    let popup = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Popup", "Rect" => rect([150, 130, 290, 190]),
        "Parent" => touching, "P" => page,
    });
    p.doc
        .get_dictionary_mut(touching)
        .unwrap()
        .set("Popup", popup);
    let reply = note(&mut p, [250, 170, 270, 190], "WALRUS-REPLY");
    p.doc
        .get_dictionary_mut(reply)
        .unwrap()
        .set("IRT", touching);
    let away = note(&mut p, [10, 170, 30, 190], "Kept note");
    let field = |p: &mut Pdf, name: &str, value: &str, r: [i64; 4]| {
        let ap = p.doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => rect([0, 0, r[2] - r[0], r[3] - r[1]]),
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            format!("/Tx BMC BT /F1 10 Tf 2 5 Td ({value}) Tj ET EMC").into_bytes(),
        ));
        p.doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "Rect" => rect(r), "P" => page,
            "F" => 4,
            "T" => Object::string_literal(name), "V" => Object::string_literal(value),
            "DV" => Object::string_literal(value), "AP" => dictionary! { "N" => ap },
        })
    };
    let secret = field(&mut p, "secret", "WALRUS-FIELD", [200, 95, 260, 115]);
    let other = field(&mut p, "other", "Kept value", [10, 10, 80, 25]);
    let thumb = p.doc.add_object(Stream::new(
        dictionary! {
            "Width" => 1, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        b"WALRUS-THUMB".to_vec(),
    ));
    let root = p.doc.new_object_id();
    let div = p.doc.new_object_id();
    let elem = p.doc.add_object(dictionary! {
        "Type" => "StructElem", "S" => "P", "P" => div, "Pg" => page, "K" => 0,
        "Alt" => Object::string_literal("WALRUS-ALT"),
    });
    p.doc.objects.insert(
        div,
        dictionary! {
            "Type" => "StructElem", "S" => "Div", "P" => root, "K" => vec![Object::Reference(elem)],
            "E" => Object::string_literal("WALRUS-EXPANSION"),
        }
        .into(),
    );
    p.doc.objects.insert(
        root,
        dictionary! {
            "Type" => "StructTreeRoot", "K" => div,
            "ParentTree" => dictionary! {
                "Nums" => vec![Object::Integer(0), vec![Object::Reference(elem)].into()],
            },
        }
        .into(),
    );
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set(
        "Annots",
        vec![touching, popup, reply, away, secret, other]
            .into_iter()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    d.set("StructParents", 0);
    d.set("Thumb", thumb);
    d.set(
        "PieceInfo",
        dictionary! { "Editor" => dictionary! { "Private" => Object::string_literal("WALRUS-PIECE") } },
    );
    let c = p.doc.catalog_mut().unwrap();
    c.set("StructTreeRoot", root);
    c.set("MarkInfo", dictionary! { "Marked" => true });
    c.set(
        "AcroForm",
        dictionary! {
            "Fields" => vec![Object::Reference(secret), Object::Reference(other)],
            "DA" => Object::string_literal("/F1 10 Tf 0 g"),
            "DR" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
    );
    p
}

#[test]
fn the_saved_output_holds_nothing_of_the_marked_text_or_what_touches_it() {
    let mut p = tagged_and_annotated();
    Saved::of(&mut p).assert_kept(&[
        "WALRUS-TEXT",
        "WALRUS-NOTE",
        "WALRUS-REPLY",
        "WALRUS-FIELD",
        "WALRUS-THUMB",
        "WALRUS-ALT",
        "WALRUS-EXPANSION",
        "WALRUS-PIECE",
    ]);
    let marks = p.find_redaction_marks("walrus-text").unwrap();
    assert_eq!(marks.len(), 1);
    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert_eq!(
        rep.pages[0].annots, 3,
        "the comment, its reply and the field"
    );

    let saved = Saved::of(&mut p);
    saved.assert_gone(&["WALRUS"]);
    saved.assert_kept(&["Keep", "Footer kept", "Kept note", "Kept value"]);
    let fields: Vec<String> = saved.fields().into_iter().map(|f| f.name).collect();
    assert_eq!(fields, ["other"]);
    assert!(saved.stray_pages().is_empty(), "{:?}", saved.stray_pages());
    let q = Pdf::from_bytes(&saved.bytes, None).unwrap();
    assert!(q.page_text(0).contains("Keep"));
}

#[test]
fn works_on_turned_pages() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Keep Zebra Keep) Tj ET"]);
    p.rotate_pages(&[0], 90).unwrap();
    let marks = p.find_redaction_marks("zebra").unwrap();
    let rep = p.redact(&marks).unwrap();
    assert_eq!(rep.pages[0].chars, 5);
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    let text = p.page_text(0);
    assert!(
        !text.to_lowercase().contains("zebra") && text.contains("Keep"),
        "{text}"
    );
}

#[test]
fn rejects_marks_with_no_size_or_page() {
    let mut p = make(&[""]);
    assert!(p.redact(&[(0, [10.0, 10.0, 10.0, 50.0])]).is_err());
    assert!(p.redact(&[(3, [10.0, 10.0, 20.0, 50.0])]).is_err());
}

#[test]
fn marks_are_kept_until_applied() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Keep Zebra Keep) Tj ET"]);
    let boxes: Vec<[f32; 4]> = p
        .find_redaction_marks("zebra")
        .unwrap()
        .into_iter()
        .map(|m| m.1)
        .collect();
    p.add_redaction_mark(0, &boxes, "Sam").unwrap();
    assert!(p.apply_redactions().is_ok_and(|r| r.pages[0].chars == 5));
    assert!(p.redaction_marks().is_empty(), "applied marks are removed");
    assert!(!p.page_text(0).contains("Zebra"));
    assert!(p.apply_redactions().is_err(), "nothing left to apply");
}

#[test]
fn marks_survive_saving_and_are_not_flattened() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Zebra) Tj ET"]);
    p.add_redaction_mark(
        0,
        &[[30.0, 80.0, 90.0, 105.0], [10.0, 10.0, 20.0, 20.0]],
        "",
    )
    .unwrap();
    let bytes = p.to_bytes().unwrap();
    let mut q = Pdf::from_bytes(&bytes, None).unwrap();
    let marks = q.redaction_marks();
    assert_eq!(marks.len(), 2);
    assert!(near(marks[0].1, [30.0, 80.0, 90.0, 105.0]), "{marks:?}");
    assert_eq!(q.annotations()[0].kind.label(), "Redaction mark");
    assert!(
        q.page_text(0).contains("Zebra"),
        "a mark removes nothing by itself"
    );
    q.flatten_annotations(None).unwrap();
    assert_eq!(q.redaction_marks().len(), 2);
}

/// Poppler and Ghostscript, reading the redacted output on their own:
/// `cargo test -p limarepdf-core --test redact -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_redacted_output_with_poppler_and_ghostscript() {
    let mut turned = make(&["BT /F1 20 Tf 30 100 Td (Keep Zebra Keep) Tj ET"]);
    turned.rotate_pages(&[0], 90).unwrap();
    // (document, what to redact, what each reader must find before and not after, what it
    // must still find after)
    let cases = [
        (
            make(&["BT /F1 20 Tf 30 100 Td (Name: Zebra Quokka here) Tj ET"]),
            "zebra quokka",
            &["Zebra", "Quokka"][..],
            &["Name:", "here"][..],
        ),
        (
            make(&["BT /F1 20 Tf 30 100 Td [(Sec) -200 (retX) 50 (Yzw)] TJ ET"]),
            "retX",
            &["retX"],
            &["Sec", "Yzw"],
        ),
        (turned, "zebra", &["Zebra"], &["Keep"]),
        (
            tagged_and_annotated(),
            "walrus-text",
            &["WALRUS-TEXT", "WALRUS-FIELD"],
            &["Keep", "Footer kept", "Kept value"],
        ),
    ];
    for (mut p, query, gone, kept) in cases {
        let before = p.to_bytes().unwrap();
        let marks = p.find_redaction_marks(query).unwrap();
        assert!(p.redact(&marks).unwrap().left.is_empty());
        let after = p.to_bytes().unwrap();
        for (tool, read) in [
            ("pdftotext", tools::pdftotext as fn(&[u8]) -> String),
            ("gs", tools::gs_text),
        ] {
            let (was, is) = (read(&before), read(&after));
            for w in gone {
                // Poppler doesn't draw form fields.
                if tool == "pdftotext" && w.ends_with("FIELD") {
                    continue;
                }
                assert!(was.contains(w), "{tool} finds {w} before: {was}");
                assert!(!is.contains(w), "{tool} finds {w} after: {is}");
            }
            for w in kept {
                if tool == "pdftotext" && w.starts_with("Kept value") {
                    continue;
                }
                assert!(is.contains(w), "{tool} doesn't find {w} after: {is}");
            }
        }
    }

    // Hidden text, which poppler reads and Ghostscript doesn't draw.
    let mut p = make(&[
        "/Span <</ActualText (Zebra)>> BDC BT 3 Tr /F1 20 Tf 30 100 Td (Zebra) Tj ET EMC \
         BT /F1 20 Tf 30 30 Td (Caption) Tj ET",
    ]);
    assert!(tools::pdftotext(&p.to_bytes().unwrap()).contains("Zebra"));
    let marks = p.find_redaction_marks("zebra").unwrap();
    p.redact(&marks).unwrap();
    let text = tools::pdftotext(&p.to_bytes().unwrap());
    assert!(
        !text.contains("Zebra") && text.contains("Caption"),
        "{text}"
    );

    // A form both pages draw, redacted on the first only.
    let mut p = shared_form();
    let marks: Vec<_> = p
        .find_redaction_marks("walrus")
        .unwrap()
        .into_iter()
        .filter(|m| m.0 == 0)
        .collect();
    p.redact(&marks).unwrap();
    let text = tools::pdftotext(&p.to_bytes().unwrap());
    // pdftotext ends each page with a form feed.
    let pages: Vec<&str> = text.split('\x0c').collect();
    assert!(
        !pages[0].contains("Walrus") && pages[0].contains("Secret"),
        "{text}"
    );
    assert!(pages[1].contains("Secret Walrus"), "{text}");
}

/// A real public PDF: the Magick++ tutorial ImageMagick installs. Its page 14 has text, a
/// picture and drawings (testing.md's redaction check uses it too), and its fonts are
/// subsets, so its text isn't in the file as written and only a reader can tell it's gone.
const TUTORIAL: &str = "/usr/share/doc/ImageMagick-7/www/magick++/tutorial/Magick++_tutorial.pdf";

/// Redacting text through the whole tutorial and half of page 14's picture, read back by
/// poppler and Ghostscript:
/// `cargo test -p limarepdf-core --test redact -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_a_redacted_public_pdf_with_poppler_and_ghostscript() {
    let original = std::fs::read(TUTORIAL)
        .unwrap_or_else(|e| panic!("{TUTORIAL}, from ImageMagick's documentation: {e}"));
    let gone = ["imprevisible", "DrawableFillColor"];
    let kept = ["anti-aliasing", "fully transparent", "DrawableStrokeColor"];
    let mut p = Pdf::from_bytes(&original, None).unwrap();
    let pages = p.page_count();
    for read in [tools::pdftotext as fn(&[u8]) -> String, tools::gs_text] {
        let text = read(&original);
        for w in gone.iter().chain(&kept) {
            assert!(text.contains(w), "{w} is in the tutorial");
        }
    }

    let mut marks = Vec::new();
    for w in gone {
        marks.extend(p.find_redaction_marks(w).unwrap());
    }
    let pic = p
        .page_objects(13)
        .unwrap()
        .into_iter()
        .find(|o| o.kind == ObjectKind::Image)
        .expect("page 14 has a picture")
        .bbox;
    marks.push((13, [pic[0], pic[1], (pic[0] + pic[2]) / 2.0, pic[3]]));
    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(rep.elsewhere.is_empty(), "{:?}", rep.elsewhere);
    assert_eq!(rep.pages.iter().map(|r| r.images_cut).sum::<usize>(), 1);

    let saved = Saved::of(&mut p);
    saved.assert_gone(&gone);
    assert!(saved.stray_pages().is_empty(), "{:?}", saved.stray_pages());
    assert_eq!(
        Pdf::from_bytes(&saved.bytes, None).unwrap().page_count(),
        pages
    );
    for (tool, read) in [
        ("pdftotext", tools::pdftotext as fn(&[u8]) -> String),
        ("gs", tools::gs_text),
    ] {
        let text = read(&saved.bytes);
        for w in gone {
            assert!(!text.contains(w), "{tool} finds {w}");
        }
        for w in kept {
            assert!(text.contains(w), "{tool} doesn't find {w}");
        }
    }

    // The picture as poppler reads it: black under the mark, as it was beside it.
    let (was, is) = (
        tools::pdfimages(&original, 14),
        tools::pdfimages(&saved.bytes, 14),
    );
    assert_eq!(is.len(), 2, "the picture and its soft mask");
    let was = image::load_from_memory(&was[0]).unwrap().to_rgb8();
    let is = image::load_from_memory(&is[0]).unwrap().to_rgb8();
    assert_eq!(was.dimensions(), is.dimensions());
    let (w, h) = is.dimensions();
    for y in 0..h {
        for x in 0..w / 2 - 2 {
            assert_eq!(is.get_pixel(x, y).0, [0, 0, 0], "at {x},{y}");
        }
        for x in w / 2 + 2..w {
            assert_eq!(is.get_pixel(x, y), was.get_pixel(x, y), "at {x},{y}");
        }
    }
}
