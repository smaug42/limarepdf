//! New documents from part of a document: cropped pages, and text laid out afresh.

use mpe_core::{FontChoice, Pdf};

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
