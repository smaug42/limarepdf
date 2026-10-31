//! New documents from part of a document: cropped pages, text laid out afresh, and a page
//! alone without some of its objects (ENH-21).

use limarepdf_core::render;
use limarepdf_core::{FontChoice, Pdf};
use lopdf::{dictionary, Object, Stream};

/// Two 300×200 pages with words at known places.
fn two_pages() -> Pdf {
    let mut p = Pdf::new();
    for i in 0..2 {
        p.insert_blank(i, 300.0, 200.0).unwrap();
    }
    let font = FontChoice::from_name("Helvetica");
    for (page, pos, text) in [
        (0, (20.0, 40.0), "Top"),
        (0, (20.0, 150.0), "Bottom"),
        (1, (200.0, 100.0), "Right"),
    ] {
        p.add_text(page, pos, text, font, 12.0, [0.0; 3]).unwrap();
    }
    p
}

#[test]
fn excerpt_crops_each_page_to_its_rectangle() {
    let p = two_pages();
    let out = p
        .excerpt(&[
            (0, [10.0, 120.0, 120.0, 170.0]),
            (1, [150.0, 80.0, 400.0, 120.0]),
        ])
        .unwrap();
    assert_eq!(out.page_count(), 2);
    assert_eq!(out.page_size(0).unwrap(), (110.0, 50.0));
    // The second rectangle is clipped at the page's right edge.
    assert_eq!(out.page_size(1).unwrap(), (150.0, 40.0));
    let words = out.page_words(0).unwrap();
    let bottom = words.iter().find(|w| w.text == "Bottom").unwrap();
    // Displayed positions now start at the crop's corner.
    assert!((bottom.bbox[0] - 10.0).abs() < 1.0, "{:?}", bottom.bbox);
    let bottom_edge = bottom.bbox[3];
    assert!(bottom_edge > 20.0 && bottom_edge < 35.0, "{bottom_edge}");
    // The source document is unchanged.
    assert_eq!(p.page_size(0).unwrap(), (300.0, 200.0));
}

#[test]
fn excerpt_keeps_an_existing_crop() {
    let mut p = two_pages();
    p.set_crop_margins(0, [10.0, 0.0, 0.0, 10.0]).unwrap();
    // "Bottom" is now at x 10, y 140 as displayed.
    let out = p.excerpt(&[(0, [0.0, 125.0, 100.0, 160.0])]).unwrap();
    let words = out.page_words(0).unwrap();
    let bottom = words.iter().find(|w| w.text == "Bottom").unwrap();
    assert!((bottom.bbox[0] - 10.0).abs() < 1.0, "{:?}", bottom.bbox);
}

#[test]
fn excerpt_refuses_empty_and_repeated_parts() {
    let p = two_pages();
    assert!(p.excerpt(&[]).is_err());
    assert!(p.excerpt(&[(0, [400.0, 0.0, 500.0, 50.0])]).is_err());
    let r = [0.0, 0.0, 50.0, 50.0];
    assert!(p.excerpt(&[(0, r), (0, r)]).is_err());
    assert!(p.excerpt(&[(5, r)]).is_err());
}

#[test]
fn from_text_lays_out_lines_and_pages() {
    let text = (1..=120)
        .map(|i| format!("Line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = Pdf::from_text(&text, (612.0, 792.0)).unwrap();
    // 792 - 144 points of lines at 14.5 points each: 44 lines a page.
    assert_eq!(out.page_count(), 3);
    assert_eq!(out.page_size(2).unwrap(), (612.0, 792.0));
    let first = out.page_text(0);
    assert!(first.starts_with("Line 1\nLine 2\n"), "{first:?}");
    assert!(out.page_text(2).trim_end().ends_with("Line 120"));
    // It saves and reopens.
    let bytes = out.to_bytes().unwrap();
    assert_eq!(Pdf::from_bytes(&bytes, None).unwrap().page_count(), 3);
}

#[test]
fn from_text_refuses_nothing_and_tiny_pages() {
    assert!(Pdf::from_text("  \n ", (612.0, 792.0)).is_err());
    assert!(Pdf::from_text("text", (40.0, 40.0)).is_err());
}

/// A 300×200 page: a gradient from red to green across it, a blue box, and over both, in
/// one text object, "Walrus" in yellow and a line below it "Otter" in black, the content
/// being `text` in place of that text object.
fn scene(text: &str) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let content = format!(
        "q 0 0 300 200 re W n /Sh0 sh Q 0 0 1 rg 20 60 200 90 re f {text} \
         BT /F1 12 Tf 0 g 200 20 Td (Seal) Tj ET"
    );
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let shading = p.doc.add_object(dictionary! {
        "ShadingType" => 2, "ColorSpace" => "DeviceRGB",
        "Coords" => vec![0.into(), 0.into(), 300.into(), 0.into()],
        "Function" => dictionary! {
            "FunctionType" => 2, "Domain" => vec![0.into(), 1.into()],
            "C0" => vec![1.into(), 0.into(), 0.into()], "C1" => vec![0.into(), 1.into(), 0.into()],
            "N" => 1,
        },
        "Extend" => vec![true.into(), true.into()],
    });
    let s = p
        .doc
        .add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", Object::Reference(s));
    d.set(
        "Resources",
        dictionary! {
            "Font" => dictionary! { "F1" => font },
            "Shading" => dictionary! { "Sh0" => shading },
        },
    );
    p
}

const BOTH: &str = "BT /F1 24 Tf 1 1 0 rg 30 120 Td (Walrus) Tj 0 -40 Td 0 g (Otter) Tj ET";
const OTTER: &str = "BT /F1 24 Tf 1 1 0 rg 30 120 Td 0 -40 Td 0 g (Otter) Tj ET";

/// The ids of page `page`'s lines reading `text`.
fn lines_reading(p: &Pdf, page: usize, text: &str) -> Vec<usize> {
    let objects = p.page_objects(page).unwrap();
    (objects.iter())
        .filter(|o| o.text.as_deref() == Some(text))
        .map(|o| o.id)
        .collect()
}

/// The first page of a document's bytes as LimarePDF draws it, at 2 pixels a point.
fn drawn(bytes: Vec<u8>) -> render::Bitmap {
    let h = render::load(bytes, None).unwrap();
    render::Renderer::new(&h).render(0, 2.0).unwrap()
}

/// The page drawn without "Walrus" is the page drawn as if it had never been written: the
/// gradient and the box under it, the line after it in its place.
#[test]
fn a_page_without_a_line_is_drawn_as_if_it_had_never_been_there() {
    let mut p = scene(BOTH);
    let before = p.snapshot().unwrap();
    let walrus = lines_reading(&p, 0, "Walrus");
    assert_eq!(walrus.len(), 1);
    let without = p.page_without(0, &walrus).unwrap();
    let want = drawn(scene(OTTER).app_snapshot().unwrap());
    let got = drawn(without.clone());
    assert_eq!((got.width, got.height), (want.width, want.height));
    assert!(got.rgba == want.rgba, "not drawn as without the line");
    assert!(drawn(p.app_snapshot().unwrap()).rgba != want.rgba);
    let alone = Pdf::from_bytes(&without, None).unwrap();
    assert_eq!(alone.page_count(), 1);
    let text = alone.page_text(0);
    assert!(
        text.contains("Otter") && !text.contains("Walrus"),
        "{text:?}"
    );
    // The document is as it was.
    assert_eq!(p.snapshot().unwrap(), before);
}

/// A turned and cropped page whose box and resources come from the page tree keeps its
/// place and look: as the page with the line deleted is drawn.
#[test]
fn a_turned_cropped_page_without_a_line_is_drawn_in_place() {
    let mut p = scene(BOTH);
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    let res = d.remove(b"Resources").unwrap();
    let media = d.remove(b"MediaBox").unwrap();
    d.set("Rotate", 90);
    let crop = vec![10.into(), 15.into(), 280.into(), 190.into()];
    d.set("CropBox", crop);
    let pages = d.get(b"Parent").unwrap().as_reference().unwrap();
    let tree = p.doc.get_dictionary_mut(pages).unwrap();
    tree.set("Resources", res);
    tree.set("MediaBox", media);
    let walrus = lines_reading(&p, 0, "Walrus");
    let got = drawn(p.page_without(0, &walrus).unwrap());
    let mut deleted = p.clone();
    deleted.delete_objects(0, &walrus).unwrap();
    let want = drawn(deleted.app_snapshot().unwrap());
    assert_eq!((got.width, got.height), (want.width, want.height));
    assert_eq!((got.width, got.height), (350, 540));
    assert!(got.rgba == want.rgba, "not drawn in place");
}

/// What a hidden layer holds stays hidden, and the page's annotations are left out.
#[test]
fn a_hidden_layer_stays_hidden_and_annotations_are_left_out() {
    let mut p = scene("/OC /L0 BDC 1 0 0 rg 240 150 40 40 re f EMC");
    let layer = p
        .doc
        .add_object(dictionary! { "Type" => "OCG", "Name" => Object::string_literal("Secret") });
    let catalog = p.doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    p.doc.get_dictionary_mut(catalog).unwrap().set(
        "OCProperties",
        dictionary! {
            "OCGs" => vec![layer.into()],
            "D" => dictionary! { "OFF" => vec![layer.into()] },
        },
    );
    let look = p.doc.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 40.into(), 40.into()] },
        b"0 0 0 rg 0 0 40 40 re f".to_vec(),
    ));
    let square = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Square",
        "Rect" => vec![240.into(), 10.into(), 280.into(), 50.into()],
        "AP" => dictionary! { "N" => look },
    });
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    let res = d.get_mut(b"Resources").unwrap().as_dict_mut().unwrap();
    res.set("Properties", dictionary! { "L0" => layer });
    d.set("Annots", vec![square.into()]);
    let page_drawn = drawn(p.app_snapshot().unwrap());
    let got = drawn(p.page_without(0, &[]).unwrap());
    let px = |b: &render::Bitmap, x: usize, y: usize| {
        let i = (y * b.width + x) * 4;
        [b.rgba[i], b.rgba[i + 1], b.rgba[i + 2]]
    };
    // The layer's red box (at 260, 30 as shown) isn't drawn in either, as it would be
    // without the document's layer settings.
    assert_eq!(px(&got, 520, 60), px(&page_drawn, 520, 60));
    assert!(px(&got, 520, 60)[0] < 200 || px(&got, 520, 60)[1] > 50);
    let mut shown = p.clone();
    let catalog = shown.doc.get_dictionary_mut(catalog).unwrap();
    catalog.remove(b"OCProperties");
    assert_eq!(
        px(&drawn(shown.app_snapshot().unwrap()), 520, 60),
        [255, 0, 0]
    );
    // The square (at 260, 170 as shown) is on the page, not in the page alone.
    assert_eq!(px(&page_drawn, 520, 340), [0, 0, 0]);
    assert_ne!(px(&got, 520, 340), [0, 0, 0]);
}

/// Only what the page uses is copied: the page alone from a long document is as small as
/// from a document of that page.
#[test]
fn a_page_alone_holds_only_what_it_uses() {
    let mut long = Pdf::new();
    for i in 0..200 {
        long.insert_blank(i, 300.0, 200.0).unwrap();
        let content = format!("0 0 1 rg {} 10 20 20 re f ", i % 250).repeat(500);
        let s = long
            .doc
            .add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let id = long.page_ids()[i];
        long.doc.get_dictionary_mut(id).unwrap().set("Contents", s);
    }
    let whole = long.snapshot().unwrap().len();
    let alone = long.page_without(120, &[]).unwrap().len();
    let one = long.excerpt(&[(120, [0.0, 0.0, 300.0, 200.0])]).unwrap();
    let one = one.clone().snapshot().unwrap().len();
    assert!(whole > 100 * alone, "{whole} vs {alone}");
    assert!(alone < one + 1000, "{alone} vs {one}");
}
