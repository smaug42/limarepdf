//! Annotations on made-up pages: reading them, drawing those without a drawing,
//! removing and flattening them.

use limarepdf_core::render::{self, Renderer};
use limarepdf_core::{
    AnnotKind, DateStyle, InkColor, Mark, Pdf, Shape, ShapeLook, SignArt, SignRole, SignStyle,
    Stamp, StampColor, StampKind, StampPlace,
};
use lopdf::{dictionary, Dictionary, Object, ObjectId, StringFormat};

fn text(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

fn nums(v: &[f32]) -> Object {
    v.iter().map(|&x| x.into()).collect::<Vec<Object>>().into()
}

/// One 300 × 200 page.
fn page() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    p
}

/// Adds an annotation of `subtype` with `extra` entries to the page's `/Annots`.
fn add(p: &mut Pdf, page: usize, subtype: &str, rect: [f32; 4], extra: Dictionary) -> ObjectId {
    let mut d = dictionary! { "Type" => "Annot", "Subtype" => subtype, "Rect" => nums(&rect) };
    for (k, v) in extra {
        d.set(k, v);
    }
    let id = p.doc.add_object(d);
    let pg = p.page_ids()[page];
    let pd = p.doc.get_dictionary_mut(pg).unwrap();
    let mut annots = pd
        .get(b"Annots")
        .ok()
        .and_then(|a| a.as_array().ok())
        .cloned()
        .unwrap_or_default();
    annots.push(id.into());
    pd.set("Annots", annots);
    id
}

/// Pixels that are not white inside `r` (left, top, right, bottom in displayed points;
/// 1 px per point), and how many of them are dark.
fn ink(p: &mut Pdf, r: [usize; 4]) -> (usize, usize) {
    let bmp = shot(p);
    let (mut any, mut dark) = (0, 0);
    for y in r[1]..r[3].min(bmp.height) {
        for x in r[0]..r[2].min(bmp.width) {
            let i = (y * bmp.width + x) * 4;
            let px = &bmp.rgba[i..i + 3];
            if px.iter().any(|&c| c < 240) {
                any += 1;
            }
            if px.iter().all(|&c| c < 128) {
                dark += 1;
            }
        }
    }
    (any, dark)
}

/// The first page as printed and exported, 1 px per point.
fn shot(p: &mut Pdf) -> render::Bitmap {
    let hp = render::load(p.render_snapshot().unwrap(), None).unwrap();
    Renderer::new(&hp).render(0, 1.0).unwrap()
}

/// The pixel at (`x`, `y`), in displayed points.
fn px(b: &render::Bitmap, x: usize, y: usize) -> [u8; 3] {
    let i = (y * b.width + x) * 4;
    [b.rgba[i], b.rgba[i + 1], b.rgba[i + 2]]
}

/// Pixels inside `r` (as for [`ink`]) where `want` holds.
fn count(b: &render::Bitmap, r: [usize; 4], want: impl Fn([u8; 3]) -> bool) -> usize {
    (r[1]..r[3])
        .flat_map(|y| (r[0]..r[2]).map(move |x| (x, y)))
        .filter(|&(x, y)| want(px(b, x, y)))
        .count()
}

fn reddish(c: [u8; 3]) -> bool {
    c[0] > 120 && c[1] < 90 && c[2] < 90
}

#[test]
fn stamps_carets_and_attachments_without_a_drawing_are_drawn() {
    let name = |n: &str| Object::Name(n.as_bytes().to_vec());
    let kinds: Vec<(&str, Dictionary)> = vec![
        ("Stamp", dictionary! { "Name" => name("NotApproved") }),
        ("Stamp", dictionary! {}),
        ("Caret", dictionary! {}),
        ("FileAttachment", dictionary! {}),
        (
            "FileAttachment",
            dictionary! { "Name" => name("Paperclip") },
        ),
        ("FileAttachment", dictionary! { "Name" => name("Graph") }),
        ("FileAttachment", dictionary! { "Name" => name("Tag") }),
    ];
    for (sub, extra) in kinds {
        let mut p = page();
        add(&mut p, 0, sub, [100.0, 60.0, 200.0, 140.0], extra);
        let (shown, _) = ink(&mut p, [100, 60, 200, 140]);
        assert!(shown > 100, "{sub} not drawn: {shown}");
        let (outside, _) = ink(&mut p, [0, 0, 300, 55]);
        assert_eq!(outside, 0, "{sub} drawn outside its rectangle");
    }
    // NotApproved is red, and its wording fills most of the width.
    let mut p = page();
    add(
        &mut p,
        0,
        "Stamp",
        [20.0, 80.0, 280.0, 120.0],
        dictionary! { "Name" => name("NotApproved") },
    );
    let b = shot(&mut p);
    assert!(count(&b, [20, 80, 280, 120], reddish) > 500);
    assert!(
        count(&b, [40, 90, 60, 110], reddish) > 0,
        "wording too narrow"
    );
}

#[test]
fn the_seven_note_icons_all_differ() {
    let names = [
        "Comment",
        "Key",
        "Note",
        "Help",
        "NewParagraph",
        "Paragraph",
        "Insert",
    ];
    let mut seen: Vec<Vec<[u8; 3]>> = Vec::new();
    for n in names {
        let mut p = page();
        add(
            &mut p,
            0,
            "Text",
            [10.0, 170.0, 30.0, 190.0],
            dictionary! { "Name" => Object::Name(n.as_bytes().to_vec()) },
        );
        let b = shot(&mut p);
        let icon: Vec<[u8; 3]> = (10..30)
            .flat_map(|y| (10..30).map(move |x| (x, y)))
            .map(|(x, y)| px(&b, x, y))
            .collect();
        assert!(!seen.contains(&icon), "{n} looks like another icon");
        seen.push(icon);
    }
}

#[test]
fn leader_lines_move_a_line_out_from_its_ends() {
    let mut p = page();
    // From left to right at user y 100, with leaders 20 pt long: the line is drawn
    // at user y 120 (displayed 80), and the leaders go down to the ends and 5 pt past.
    add(
        &mut p,
        0,
        "Line",
        [80.0, 80.0, 220.0, 140.0],
        dictionary! {
            "C" => nums(&[0.0, 0.0, 0.0]),
            "L" => nums(&[100.0, 100.0, 200.0, 100.0]),
            "LL" => 20,
            "LLE" => 5,
        },
    );
    let (moved, _) = ink(&mut p, [120, 78, 180, 82]);
    assert!(moved > 50, "line not at its leaders' tips: {moved}");
    let (at_ends, _) = ink(&mut p, [120, 98, 180, 102]);
    assert_eq!(at_ends, 0, "line still between its ends");
    let (leader, _) = ink(&mut p, [98, 80, 102, 100]);
    assert!(leader > 10, "no leader line: {leader}");
    let (past, _) = ink(&mut p, [98, 70, 102, 78]);
    assert!(past > 0, "no leader extension");
}

#[test]
fn a_cloudy_border_has_bumps() {
    let shape = |be: Option<Dictionary>| {
        let mut p = page();
        let mut d = dictionary! { "C" => nums(&[1.0, 0.0, 0.0]) };
        if let Some(be) = be {
            d.set("BE", be);
        }
        add(&mut p, 0, "Square", [100.0, 60.0, 200.0, 140.0], d);
        shot(&mut p)
    };
    let plain = shape(None);
    let cloudy = shape(Some(dictionary! { "S" => "C", "I" => 1 }));
    // A plain square is a straight edge; the cloud's bumps leave the middle of each side
    // and stay inside the rectangle.
    let edge = |b: &render::Bitmap| count(b, [100, 60, 200, 61], reddish);
    assert!(edge(&plain) > 80);
    assert!(edge(&cloudy) < 60, "{}", edge(&cloudy));
    assert!(count(&cloudy, [100, 60, 200, 140], reddish) > 200);
    assert_eq!(count(&cloudy, [0, 0, 300, 59], reddish), 0);
    // Intensity 0 is no cloud.
    let flat = shape(Some(dictionary! { "S" => "C", "I" => 0 }));
    assert_eq!(edge(&flat), edge(&plain));
}

#[test]
fn a_text_box_callout_points_from_the_box() {
    let mut p = page();
    // The box is the right part of the rectangle's bottom; the callout runs from its
    // left edge up to the top left, ending in an arrow.
    add(
        &mut p,
        0,
        "FreeText",
        [20.0, 40.0, 280.0, 160.0],
        dictionary! {
            "Contents" => text("MMMM"),
            "DA" => text("/Helv 20 Tf 1 0 0 rg"),
            "IT" => "FreeTextCallout",
            "RD" => nums(&[120.0, 0.0, 0.0, 60.0]),
            "CL" => nums(&[40.0, 150.0, 90.0, 120.0, 140.0, 70.0]),
            "LE" => "OpenArrow",
        },
    );
    let b = shot(&mut p);
    // Displayed y = 200 - user y. The text sits in the box, not at the rectangle's top.
    assert!(
        count(&b, [140, 100, 280, 160], reddish) > 100,
        "no text in the box"
    );
    assert_eq!(
        count(&b, [140, 40, 280, 95], reddish),
        0,
        "box not moved by /RD"
    );
    assert!(
        count(&b, [35, 45, 50, 60], reddish) > 0,
        "no callout at its point"
    );
    assert!(
        count(&b, [85, 75, 95, 85], reddish) > 0,
        "callout misses its knee"
    );
}

#[test]
fn flattening_leaves_file_attachments_without_a_drawing() {
    let mut p = page();
    let id = add(
        &mut p,
        0,
        "FileAttachment",
        [10.0, 10.0, 30.0, 30.0],
        dictionary! {},
    );
    p.flatten_annotations(None).unwrap();
    let d = p.doc.get_dictionary(id).unwrap();
    assert!(!d.has(b"AP"));
    assert_eq!(p.annotations().len(), 1);
}

#[test]
fn lists_comments_and_markup_but_not_links_widgets_or_popups() {
    let mut p = page();
    let note = add(
        &mut p,
        0,
        "Text",
        [10.0, 170.0, 30.0, 190.0],
        dictionary! {
            "Contents" => text("Check this"),
            "T" => text("Sam"),
            "M" => text("D:20240305142233Z"),
            "C" => nums(&[1.0, 0.0, 0.0]),
            "F" => 4,
        },
    );
    add(
        &mut p,
        0,
        "Popup",
        [40.0, 120.0, 140.0, 190.0],
        dictionary! { "Parent" => note },
    );
    add(&mut p, 0, "Link", [0.0, 0.0, 10.0, 10.0], dictionary! {});
    add(&mut p, 0, "Widget", [0.0, 0.0, 10.0, 10.0], dictionary! {});
    add(
        &mut p,
        0,
        "Highlight",
        [50.0, 100.0, 150.0, 112.0],
        dictionary! { "IRT" => note, "CA" => 0.5, "C" => nums(&[0.0, 0.0, 0.0, 1.0]) },
    );
    let a = p.annotations();
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].kind, AnnotKind::Note);
    assert_eq!(a[0].contents, "Check this");
    assert_eq!(a[0].author, "Sam");
    assert_eq!(a[0].modified, "2024-03-05 14:22");
    assert_eq!(a[0].color, Some([1.0, 0.0, 0.0]));
    // Displayed coordinates run down from the top of the 200 pt page.
    assert_eq!(a[0].bbox, [10.0, 10.0, 30.0, 30.0]);
    assert!(!a[0].has_appearance);
    assert_eq!(a[1].kind, AnnotKind::Highlight);
    assert_eq!(a[1].reply_to, Some(note));
    assert_eq!(a[1].opacity, 0.5);
    assert_eq!(a[1].color, Some([0.0; 3]));
}

#[test]
fn draws_missing_appearances_on_screen_without_changing_the_file() {
    let mut p = page();
    // Spec order: anticlockwise from the bottom left.
    let quad = [50.0, 100.0, 150.0, 100.0, 150.0, 112.0, 50.0, 112.0];
    let id = add(
        &mut p,
        0,
        "Highlight",
        [50.0, 100.0, 150.0, 112.0],
        dictionary! { "QuadPoints" => nums(&quad), "C" => nums(&[1.0, 1.0, 0.0]) },
    );
    let before = p.doc.objects.clone();
    // Displayed y = 200 - user y: the highlight covers rows 88 to 100.
    let (shown, _) = ink(&mut p, [50, 88, 150, 100]);
    assert!(shown > 1000, "highlight not drawn: {shown}");
    let (outside, _) = ink(&mut p, [0, 0, 300, 80]);
    assert_eq!(outside, 0);
    assert!(!p.doc.get_dictionary(id).unwrap().has(b"AP"));
    assert_eq!(p.doc.objects, before);
}

#[test]
fn draws_every_kind_limarepdf_knows() {
    let red = || nums(&[1.0, 0.0, 0.0]);
    let r = [100.0, 60.0, 200.0, 140.0];
    let kinds: Vec<(&str, Dictionary)> = vec![
        ("Highlight", dictionary! {}),
        ("Underline", dictionary! { "C" => red() }),
        ("StrikeOut", dictionary! { "C" => red() }),
        ("Squiggly", dictionary! { "C" => red() }),
        ("Text", dictionary! {}),
        (
            "FreeText",
            dictionary! { "Contents" => text("Hello there"), "DA" => text("/Helv 12 Tf 0 g") },
        ),
        (
            "Square",
            dictionary! { "C" => red(), "IC" => nums(&[0.0, 0.0, 1.0]) },
        ),
        (
            "Circle",
            dictionary! { "C" => red(), "BS" => dictionary! { "W" => 3 } },
        ),
        (
            "Line",
            dictionary! {
                "C" => red(),
                "L" => nums(&[110.0, 100.0, 190.0, 100.0]),
                "LE" => vec![Object::Name(b"None".to_vec()), Object::Name(b"ClosedArrow".to_vec())],
            },
        ),
        (
            "Polygon",
            dictionary! { "C" => red(), "Vertices" => nums(&[110.0, 70.0, 190.0, 70.0, 150.0, 130.0]) },
        ),
        (
            "Ink",
            dictionary! {
                "C" => red(),
                "InkList" => vec![nums(&[110.0, 70.0, 150.0, 130.0, 190.0, 70.0])],
            },
        ),
    ];
    for (sub, extra) in kinds {
        let mut p = page();
        add(&mut p, 0, sub, r, extra);
        let (shown, _) = ink(&mut p, [100, 60, 200, 140]);
        assert!(shown > 20, "{sub} not drawn: {shown}");
        let (outside, _) = ink(&mut p, [0, 0, 300, 55]);
        assert_eq!(outside, 0, "{sub} drawn outside its rectangle");
    }
}

#[test]
fn text_box_shows_its_text() {
    let mut p = page();
    add(
        &mut p,
        0,
        "FreeText",
        [20.0, 100.0, 280.0, 140.0],
        dictionary! {
            "Contents" => text("MMMM MMMM"),
            "DA" => text("/Helv 20 Tf 0 g"),
            "BS" => dictionary! { "W" => 0 },
        },
    );
    let (_, dark) = ink(&mut p, [20, 60, 280, 100]);
    assert!(dark > 100, "text not drawn: {dark}");
}

#[test]
fn removing_takes_replies_and_popups_too() {
    let mut p = page();
    let note = add(
        &mut p,
        0,
        "Text",
        [10.0, 170.0, 30.0, 190.0],
        dictionary! {},
    );
    let popup = add(
        &mut p,
        0,
        "Popup",
        [40.0, 120.0, 140.0, 190.0],
        dictionary! { "Parent" => note },
    );
    p.doc.get_dictionary_mut(note).unwrap().set("Popup", popup);
    add(
        &mut p,
        0,
        "Text",
        [10.0, 170.0, 30.0, 190.0],
        dictionary! { "IRT" => note },
    );
    add(
        &mut p,
        0,
        "Square",
        [50.0, 50.0, 90.0, 90.0],
        dictionary! {},
    );
    add(&mut p, 0, "Link", [0.0, 0.0, 10.0, 10.0], dictionary! {});
    let n = p.remove_annotations(|a| a.id == note).unwrap();
    assert_eq!(n, 2);
    let left = p.annotations();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].kind, AnnotKind::Square);
    let pg = p.page_ids()[0];
    let annots = p.doc.get_dictionary(pg).unwrap().get(b"Annots").unwrap();
    assert_eq!(annots.as_array().unwrap().len(), 2, "square and link stay");
    assert_eq!(
        p.remove_annotations(|a| a.kind == AnnotKind::Square)
            .unwrap(),
        1
    );
    assert_eq!(p.remove_annotations(|_| true).unwrap(), 0);
}

#[test]
fn flattening_keeps_the_look_and_removes_the_annotations() {
    let mut p = page();
    add(
        &mut p,
        0,
        "Square",
        [50.0, 50.0, 150.0, 150.0],
        dictionary! { "C" => nums(&[0.0, 0.0, 0.0]), "BS" => dictionary! { "W" => 4 } },
    );
    let note = add(
        &mut p,
        0,
        "Text",
        [200.0, 170.0, 220.0, 190.0],
        dictionary! {},
    );
    add(
        &mut p,
        0,
        "Popup",
        [40.0, 120.0, 140.0, 190.0],
        dictionary! { "Parent" => note },
    );
    add(
        &mut p,
        0,
        "Text",
        [200.0, 170.0, 220.0, 190.0],
        dictionary! { "IRT" => note },
    );
    add(&mut p, 0, "Link", [0.0, 0.0, 10.0, 10.0], dictionary! {});
    add(
        &mut p,
        0,
        "FileAttachment",
        [250.0, 10.0, 260.0, 20.0],
        dictionary! {},
    );
    let before = ink(&mut p, [0, 0, 300, 200]);
    assert_eq!(p.flatten_annotations(None).unwrap(), 2);
    let left = p.annotations();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].kind, AnnotKind::Other("FileAttachment".into()));
    assert_eq!(
        p.page_links(0).unwrap().len(),
        0,
        "link has no target, but stays"
    );
    let pg = p.page_ids()[0];
    let annots = p.doc.get_dictionary(pg).unwrap().get(b"Annots").unwrap();
    assert_eq!(annots.as_array().unwrap().len(), 2);
    // Reload, so nothing on screen comes from the old annotations.
    let bytes = p.to_bytes().unwrap();
    let mut p = Pdf::from_bytes(&bytes, None).unwrap();
    assert_eq!(ink(&mut p, [0, 0, 300, 200]), before);
}

#[test]
fn flattening_some_pages_leaves_the_others() {
    let mut p = page();
    p.insert_blank(1, 300.0, 200.0).unwrap();
    add(
        &mut p,
        0,
        "Square",
        [50.0, 50.0, 150.0, 150.0],
        dictionary! {},
    );
    add(
        &mut p,
        1,
        "Square",
        [50.0, 50.0, 150.0, 150.0],
        dictionary! {},
    );
    assert_eq!(p.flatten_annotations(Some(&[1])).unwrap(), 1);
    let left = p.annotations();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].page, 0);
    assert!(p.flatten_annotations(Some(&[5])).is_err());
}

#[test]
fn markup_is_added_with_a_drawing_and_survives_saving() {
    let mut p = page();
    let lines = [[20.0, 30.0, 200.0, 44.0], [20.0, 50.0, 120.0, 64.0]];
    let id = p
        .add_markup(0, &AnnotKind::Highlight, &lines, [1.0, 1.0, 0.0], "Sam")
        .unwrap();
    let mut p = Pdf::from_bytes(&p.snapshot().unwrap(), None).unwrap();
    let list = p.annotations();
    assert_eq!(list.len(), 1);
    let a = &list[0];
    assert_eq!(a.id, id);
    assert_eq!(a.kind, AnnotKind::Highlight);
    assert_eq!(a.author, "Sam");
    assert!(a.has_appearance, "LimarePDF draws it when adding it");
    assert!(!a.modified.is_empty());
    assert_eq!(a.quads.len(), 2);
    for (q, l) in a.quads.iter().zip(&lines) {
        for i in 0..4 {
            assert!((q[i] - l[i]).abs() < 0.01, "{q:?} vs {l:?}");
        }
    }
    // Yellow over the first line, nothing between the end of the second line and the
    // edge of the first.
    assert!(ink(&mut p, [25, 32, 195, 42]).0 > 1000);
    assert_eq!(ink(&mut p, [125, 52, 195, 62]).0, 0);
    assert!(p
        .add_markup(0, &AnnotKind::Note, &lines, [0.0; 3], "")
        .is_err());
    assert!(p
        .add_markup(0, &AnnotKind::Underline, &[], [0.0; 3], "")
        .is_err());
}

#[test]
fn markup_follows_the_text_on_a_turned_page() {
    let mut p = page();
    p.rotate_pages(&[0], 90).unwrap();
    // A line across the page as it is shown (200 wide, 300 high).
    let line = [20.0, 50.0, 180.0, 70.0];
    p.add_markup(0, &AnnotKind::Underline, &[line], [0.0; 3], "")
        .unwrap();
    let a = &p.annotations()[0];
    assert!((a.quads[0][1] - 50.0).abs() < 0.01 && (a.quads[0][3] - 70.0).abs() < 0.01);
    assert!(a.author.is_empty());
    // The line is under the text as shown, not along its side.
    assert!(ink(&mut p, [25, 64, 175, 71]).1 > 100);
    assert_eq!(ink(&mut p, [25, 50, 175, 62]).0, 0);
}

#[test]
fn changing_colour_opacity_and_comment() {
    let mut p = page();
    let id = p
        .add_markup(
            0,
            &AnnotKind::StrikeOut,
            &[[20.0, 30.0, 200.0, 44.0]],
            [0.0; 3],
            "",
        )
        .unwrap();
    p.set_annotation(id, Some([1.0, 0.0, 0.0]), 0.5, "Delete this")
        .unwrap();
    let a = &p.annotations()[0];
    assert_eq!(a.color, Some([1.0, 0.0, 0.0]));
    assert!((a.opacity - 0.5).abs() < 1e-3);
    assert_eq!(a.contents, "Delete this");
    p.set_annotation(id, Some([1.0, 0.0, 0.0]), 1.0, "")
        .unwrap();
    let a = &p.annotations()[0];
    assert_eq!(a.opacity, 1.0);
    assert!(a.contents.is_empty());
    // The struck line is red now: some ink, none of it dark.
    let (any, dark) = ink(&mut p, [25, 32, 195, 42]);
    assert!(any > 50 && dark == 0, "{any} {dark}");
    assert!(p.set_annotation((999, 0), None, 1.0, "").is_err());
}

#[test]
fn a_note_is_placed_where_clicked() {
    let mut p = page();
    let id = p
        .add_note(0, (100.0, 50.0), [1.0, 0.85, 0.2], "Check this", "Sam")
        .unwrap();
    let p2 = Pdf::from_bytes(&p.snapshot().unwrap(), None).unwrap();
    let a = &p2.annotations()[0];
    assert_eq!((a.id, &a.kind), (id, &AnnotKind::Note));
    assert_eq!(
        (a.contents.as_str(), a.author.as_str()),
        ("Check this", "Sam")
    );
    assert!(a.has_appearance);
    assert!((a.bbox[0] - 90.0).abs() < 0.01 && (a.bbox[1] - 40.0).abs() < 0.01);
    assert!(ink(&mut p, [90, 40, 110, 60]).1 > 5, "the icon is drawn");
    assert_eq!(ink(&mut p, [120, 40, 200, 60]).0, 0);
    // The app draws notes itself, so its page images leave them out.
    let hp = render::load(p.app_snapshot().unwrap(), None).unwrap();
    let bmp = Renderer::new(&hp).render(0, 1.0).unwrap();
    assert!(
        bmp.rgba.iter().all(|&c| c == 255),
        "no note in the app's image"
    );
}

#[test]
fn a_text_box_is_written_and_restyled() {
    let mut p = page();
    let style = limarepdf_core::TextStyle {
        size: 20.0,
        ..Default::default()
    };
    let r = [20.0, 60.0, 280.0, 100.0];
    let id = p.add_text_box(0, r, "MMMM MMMM", &style, "").unwrap();
    let a = &p.annotations()[0];
    assert_eq!(a.kind, AnnotKind::TextBox);
    assert_eq!(a.contents, "MMMM MMMM");
    assert_eq!(a.text_style, Some(style));
    assert!(ink(&mut p, [20, 60, 280, 100]).1 > 100, "text drawn");
    let red = limarepdf_core::TextStyle {
        size: 20.0,
        color: [1.0, 0.0, 0.0],
        fill: Some([0.9, 0.9, 1.0]),
        border: 2.0,
    };
    p.set_text_style(id, &red).unwrap();
    assert_eq!(p.annotations()[0].text_style, Some(red));
    let (any, dark) = ink(&mut p, [20, 60, 280, 100]);
    assert!(any > 5000 && dark == 0, "filled, red text: {any} {dark}");
    assert!(p
        .add_text_box(0, [5.0, 5.0, 5.5, 50.0], "x", &style, "")
        .is_err());
}

#[test]
fn a_text_box_grows_to_fit_its_text_but_not_off_the_page() {
    let mut p = page();
    let style = limarepdf_core::TextStyle {
        size: 12.0,
        border: 1.0,
        ..Default::default()
    };
    let four = "one\ntwo\nthree\nfour";
    let id = p
        .add_text_box(0, [20.0, 20.0, 120.0, 40.0], four, &style, "")
        .unwrap();
    let b = p.annotations()[0].bbox;
    assert_eq!(
        (b[0], b[1], b[2]),
        (20.0, 20.0, 120.0),
        "only the bottom moves"
    );
    assert!(b[3] > 75.0 && b[3] < 85.0, "four lines fit: {b:?}");
    // Fewer lines don't shrink it; more lines grow it, up to the page's bottom edge.
    p.set_annotation(id, None, 1.0, "one").unwrap();
    assert_eq!(p.annotations()[0].bbox, b);
    let long = "many words in a row ".repeat(20);
    p.set_annotation(id, None, 1.0, &long).unwrap();
    let b = p.annotations()[0].bbox;
    assert!((b[3] - 200.0).abs() < 0.01, "stops at the edge: {b:?}");
}

#[test]
fn a_text_box_reads_upright_on_a_turned_page() {
    let mut p = page();
    p.rotate_pages(&[0], 90).unwrap();
    let style = limarepdf_core::TextStyle {
        size: 20.0,
        ..Default::default()
    };
    // Wide and short as the page is shown (200 wide, 300 high).
    p.add_text_box(0, [10.0, 100.0, 190.0, 140.0], "MMMM MMMM", &style, "")
        .unwrap();
    let a = &p.annotations()[0];
    assert!((a.bbox[2] - 190.0).abs() < 0.01 && (a.bbox[3] - 140.0).abs() < 0.01);
    // The text runs across: ink in both halves of the box, also once flattened.
    assert!(ink(&mut p, [10, 100, 90, 140]).1 > 50);
    assert!(ink(&mut p, [100, 100, 180, 140]).1 > 50);
    let before = ink(&mut p, [10, 100, 190, 140]);
    assert_eq!(p.flatten_annotations(None).unwrap(), 1);
    assert!(p.annotations().is_empty());
    assert_eq!(ink(&mut p, [10, 100, 190, 140]), before);
}

#[test]
fn moving_and_resizing_take_the_geometry_along() {
    let mut p = page();
    let line = add(
        &mut p,
        0,
        "Line",
        [10.0, 10.0, 110.0, 30.0],
        dictionary! { "L" => nums(&[10.0, 20.0, 110.0, 20.0]), "C" => nums(&[0.0, 0.0, 0.0]),
        "BS" => dictionary! { "W" => 3 } },
    );
    let note = p
        .add_note(0, (200.0, 100.0), [1.0, 0.0, 0.0], "", "")
        .unwrap();
    // The line is drawn for the screen only; the note has a drawing in the file.
    let before = p.annotations();
    let nb = before.iter().find(|a| a.id == note).unwrap().bbox;
    // Move the note 50 points right: same size, same drawing.
    let to = [nb[0] + 50.0, nb[1], nb[2] + 50.0, nb[3]];
    p.move_annotation(0, note, to).unwrap();
    let after = p.annotations();
    let na = after.iter().find(|a| a.id == note).unwrap();
    assert!((na.bbox[0] - to[0]).abs() < 0.01 && (na.bbox[1] - to[1]).abs() < 0.01);
    assert!(ink(&mut p, [240, 90, 260, 110]).0 > 50, "the note moved");
    assert_eq!(ink(&mut p, [190, 90, 210, 110]).0, 0);
    // Stretch the line to twice its length, 100 points higher on the page.
    let lb = after.iter().find(|a| a.id == line).unwrap().bbox;
    p.move_annotation(
        0,
        line,
        [lb[0], lb[1] - 100.0, lb[0] + 200.0, lb[3] - 100.0],
    )
    .unwrap();
    let d = p.doc.get_dictionary(line).unwrap();
    let l: Vec<f32> = d
        .get(b"L")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.as_float().unwrap())
        .collect();
    assert_eq!(l, vec![10.0, 120.0, 210.0, 120.0]);
    assert!(
        ink(&mut p, [150, 75, 200, 85]).1 > 20,
        "drawn longer, higher"
    );
}

#[test]
fn shapes_and_lines_are_annotations_that_can_be_restyled() {
    let mut p = page();
    let look = ShapeLook {
        fill: Some([1.0, 1.0, 0.0]),
        width: 2.0,
        arrow: false,
    };
    let sq = p
        .add_shape_annotation(
            0,
            Shape::Rect,
            [20.0, 20.0, 120.0, 80.0],
            Some([0.0; 3]),
            &look,
            "Sam",
        )
        .unwrap();
    let arrow = ShapeLook {
        fill: None,
        width: 3.0,
        arrow: true,
    };
    let line = p
        .add_shape_annotation(
            0,
            Shape::Line,
            [150.0, 150.0, 280.0, 150.0],
            Some([0.0; 3]),
            &arrow,
            "",
        )
        .unwrap();
    let list = p.annotations();
    let a = list.iter().find(|a| a.id == sq).unwrap();
    assert_eq!((&a.kind, a.shape), (&AnnotKind::Square, Some(look)));
    assert_eq!(a.bbox, [20.0, 20.0, 120.0, 80.0]);
    let l = list.iter().find(|a| a.id == line).unwrap();
    assert_eq!(l.kind, AnnotKind::Line);
    assert_eq!(l.shape, Some(arrow));
    assert_eq!(l.paths, [vec![(150.0, 150.0), (280.0, 150.0)]]);
    // Filled yellow inside, a dark outline, and an arrowhead that reaches above the line.
    let (any, dark) = ink(&mut p, [30, 30, 110, 70]);
    assert!(any > 3000 && dark == 0, "{any} {dark}");
    assert!(ink(&mut p, [18, 18, 122, 23]).1 > 100, "outline");
    assert!(ink(&mut p, [262, 142, 278, 146]).1 > 3, "arrowhead");
    // No fill, a thin line and no arrowhead.
    p.set_shape_look(sq, &ShapeLook::default()).unwrap();
    p.set_shape_look(
        line,
        &ShapeLook {
            arrow: false,
            ..arrow
        },
    )
    .unwrap();
    assert_eq!(ink(&mut p, [30, 30, 110, 70]).0, 0);
    assert_eq!(ink(&mut p, [262, 142, 278, 146]).0, 0);
    assert!(p.doc.get_dictionary(line).unwrap().get(b"LE").is_err());
    let a = p.annotations();
    assert_eq!(a[0].shape, Some(ShapeLook::default()));
    assert!(!a[1].shape.unwrap().arrow);
    // Too small to be meant.
    assert!(p
        .add_shape_annotation(0, Shape::Ellipse, [5.0, 5.0, 5.5, 9.0], None, &look, "")
        .is_err());
    assert!(p
        .add_shape_annotation(0, Shape::Line, [5.0, 5.0, 5.2, 5.0], None, &look, "")
        .is_err());
}

#[test]
fn a_line_can_be_pointed_elsewhere() {
    let mut p = page();
    let arrow = ShapeLook {
        fill: None,
        width: 3.0,
        arrow: true,
    };
    let line = p
        .add_shape_annotation(
            0,
            Shape::Line,
            [20.0, 100.0, 120.0, 100.0],
            Some([0.0; 3]),
            &arrow,
            "",
        )
        .unwrap();
    // Turn it to point straight down from its start.
    p.set_line_ends(0, line, [20.0, 100.0, 20.0, 180.0])
        .unwrap();
    let a = &p.annotations()[0];
    assert_eq!(a.paths, [vec![(20.0, 100.0), (20.0, 180.0)]]);
    assert_eq!(a.shape, Some(arrow), "width and arrowhead kept");
    assert!(a.bbox[3] > 180.0 && a.bbox[0] < 20.0);
    assert_eq!(
        ink(&mut p, [40, 95, 120, 105]).0,
        0,
        "gone from where it was"
    );
    assert!(
        ink(&mut p, [17, 120, 23, 170]).1 > 50,
        "drawn where it points"
    );
    assert!(p.set_line_ends(0, line, [5.0, 5.0, 5.0, 5.2]).is_err());
}

const ENDINGS: [&str; 9] = [
    "OpenArrow",
    "ClosedArrow",
    "ROpenArrow",
    "RClosedArrow",
    "Butt",
    "Slash",
    "Square",
    "Diamond",
    "Circle",
];

/// A 2-point black line from (60, 100) to (240, 100) shown, dashed 3 on 3 off when
/// `dashed`, with `end` at its right end.
fn line_with(end: &str, dashed: bool) -> (Pdf, ObjectId) {
    let mut p = page();
    let bs = if dashed {
        dictionary! { "W" => 2, "S" => "D", "D" => nums(&[3.0, 3.0]) }
    } else {
        dictionary! { "W" => 2, "S" => "S" }
    };
    let id = add(
        &mut p,
        0,
        "Line",
        [40.0, 80.0, 260.0, 120.0],
        dictionary! {
            "C" => nums(&[0.0; 3]),
            "L" => nums(&[60.0, 100.0, 240.0, 100.0]),
            "LE" => vec![Object::from("None"), Object::Name(end.as_bytes().to_vec())],
            "BS" => bs,
        },
    );
    (p, id)
}

/// Ink just above the line's right end, where its ending is drawn but its shaft isn't,
/// and along the shaft.
fn ending_and_shaft(p: &mut Pdf, y: usize) -> (usize, usize) {
    (
        ink(p, [224, y - 8, 256, y - 2]).0,
        ink(p, [80, y - 1, 200, y + 1]).0,
    )
}

#[test]
fn endings_on_dashed_lines_are_solid() {
    for end in ENDINGS {
        let (mut solid, _) = line_with(end, false);
        let (mut dashed, id) = line_with(end, true);
        let (want, shaft) = ending_and_shaft(&mut solid, 100);
        // A butt is a 2-point tick: 8 pixels here.
        assert!(want >= 8, "{end}: no ending drawn: {want}");
        let (got, dashes) = ending_and_shaft(&mut dashed, 100);
        assert_eq!(got, want, "{end}: drawn with gaps");
        assert!(
            dashes < shaft * 3 / 4,
            "{end}: shaft not dashed: {dashes} of {shaft}"
        );

        // Drawn again after an edit, and flattened into the page.
        dashed
            .set_line_ends(0, (id.0, id.1), [60.0, 120.0, 240.0, 120.0])
            .unwrap();
        solid
            .set_line_ends(0, (id.0, id.1), [60.0, 120.0, 240.0, 120.0])
            .unwrap();
        let want = ending_and_shaft(&mut solid, 120).0;
        assert_eq!(ending_and_shaft(&mut dashed, 120).0, want, "{end}: edited");
        dashed.flatten_annotations(None).unwrap();
        assert!(dashed.annotations().is_empty());
        let flat = ending_and_shaft(&mut dashed, 120).0;
        assert_eq!(flat, want, "{end}: flattened");
    }
}

/// Poppler draws the flattened endings solid too. Opt-in: needs `pdftoppm`.
#[test]
#[ignore]
fn cross_check_dashed_line_endings_with_poppler() {
    let dir = std::env::temp_dir().join(format!("limarepdf-dashed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let poppler = |p: &mut Pdf, name: &str| {
        p.flatten_annotations(None).unwrap();
        let path = dir.join(format!("{name}.pdf"));
        std::fs::write(&path, p.to_bytes().unwrap()).unwrap();
        let stem = dir.join(name);
        let ok = std::process::Command::new("pdftoppm")
            .args(["-r", "72", "-singlefile", "-png"])
            .arg(&path)
            .arg(&stem)
            .status()
            .expect("pdftoppm")
            .success();
        assert!(ok);
        let img = image::open(stem.with_extension("png")).unwrap().to_rgb8();
        let ink = |r: [u32; 4]| {
            (r[1]..r[3])
                .flat_map(|y| (r[0]..r[2]).map(move |x| (x, y)))
                .filter(|&(x, y)| img.get_pixel(x, y).0.iter().any(|&c| c < 240))
                .count()
        };
        (ink([224, 92, 256, 98]), ink([80, 99, 200, 101]))
    };
    for end in ENDINGS {
        let (want, shaft) = poppler(&mut line_with(end, false).0, &format!("{end}-solid"));
        let (got, dashes) = poppler(&mut line_with(end, true).0, &format!("{end}-dashed"));
        assert!(want >= 8, "{end}: no ending drawn");
        assert_eq!(got, want, "{end}: drawn with gaps");
        assert!(dashes < shaft * 3 / 4, "{end}: shaft not dashed");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pen_strokes_are_simplified_and_join_one_drawing() {
    let mut p = page();
    // A straight stroke drawn with a slight wobble, 100 points long.
    let wobbly: Vec<(f32, f32)> = (0..=100)
        .map(|i| (20.0 + i as f32, 50.0 + if i % 2 == 0 { 0.1 } else { -0.1 }))
        .collect();
    let id = p.add_ink(0, &[wobbly], [0.0; 3], 2.0, "").unwrap();
    let a = &p.annotations()[0];
    assert_eq!(a.kind, AnnotKind::Ink);
    assert!(a.paths[0].len() <= 3, "{:?}", a.paths);
    p.add_ink_stroke(0, id, &[(20.0, 120.0), (120.0, 120.0)])
        .unwrap();
    let a = &p.annotations()[0];
    assert_eq!(a.paths.len(), 2);
    assert!(a.bbox[1] < 50.0 && a.bbox[3] > 120.0, "{:?}", a.bbox);
    assert!(ink(&mut p, [30, 48, 110, 53]).1 > 50);
    assert!(ink(&mut p, [30, 118, 110, 123]).1 > 50);
    let note = p.add_note(0, (200.0, 100.0), [1.0; 3], "", "").unwrap();
    assert!(p.add_ink_stroke(0, note, &[(0.0, 0.0)]).is_err());
    assert!(p.add_ink(0, &[vec![]], [0.0; 3], 1.0, "").is_err());
}

#[test]
fn a_drawing_turns_about_its_middle() {
    let mut p = page();
    p.rotate_pages(&[0], 90).unwrap();
    // Shown 200 wide and 300 high: a stroke across, then turned a quarter clockwise.
    let id = p
        .add_ink(0, &[vec![(50.0, 100.0), (150.0, 100.0)]], [0.0; 3], 2.0, "")
        .unwrap();
    p.rotate_ink(0, id, 90.0).unwrap();
    let a = &p.annotations()[0];
    let s = &a.paths[0];
    let near = |q: (f32, f32), x: f32, y: f32| (q.0 - x).abs() < 0.01 && (q.1 - y).abs() < 0.01;
    assert!(near(s[0], 100.0, 50.0) && near(s[1], 100.0, 150.0), "{s:?}");
    assert!(a.bbox[0] > 95.0 && a.bbox[2] < 105.0, "{:?}", a.bbox);
    assert!(p.rotate_ink(0, (999, 0), 10.0).is_err());
}

#[test]
fn replies_are_listed_but_not_drawn() {
    let mut p = page();
    let note = p
        .add_note(0, (100.0, 100.0), [1.0, 0.85, 0.2], "Is this right?", "Sam")
        .unwrap();
    let before = ink(&mut p, [0, 0, 300, 200]);
    let reply = p.add_reply(note, "Yes.", "Alex").unwrap();
    let p2 = Pdf::from_bytes(&p.snapshot().unwrap(), None).unwrap();
    let list = p2.annotations();
    let r = list.iter().find(|a| a.id == reply).unwrap();
    assert_eq!(r.reply_to, Some(note));
    assert_eq!((r.contents.as_str(), r.author.as_str()), ("Yes.", "Alex"));
    assert!(!r.on_page() && !r.has_appearance);
    assert!(list.iter().find(|a| a.id == note).unwrap().on_page());
    assert_eq!(ink(&mut p, [0, 0, 300, 200]), before, "nothing more drawn");
    assert!(p.add_reply(note, "  ", "").is_err());
    // Deleting what it answers takes the reply too.
    assert_eq!(p.remove_annotations(|a| a.id == note).unwrap(), 2);
}

fn png(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(c));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn a_stamp_is_centred_where_clicked_and_survives_flattening() {
    let mut p = page();
    let stamp = Stamp::Text {
        kind: StampKind::Approved,
        name_and_date: true,
    };
    let (w, h) = stamp.size("Sam").unwrap();
    p.add_stamp(0, &stamp, StampPlace::Centre(150.0, 100.0), "Sam")
        .unwrap();
    let a = &p.annotations()[0];
    assert_eq!(a.kind, AnnotKind::Stamp);
    assert_eq!(a.author, "Sam");
    let c = ((a.bbox[0] + a.bbox[2]) / 2.0, (a.bbox[1] + a.bbox[3]) / 2.0);
    assert!((c.0 - 150.0).abs() < 0.01 && (c.1 - 100.0).abs() < 0.01);
    assert!((a.bbox[2] - a.bbox[0] - w).abs() < 0.01 && (a.bbox[3] - a.bbox[1] - h).abs() < 0.01);
    let before = ink(&mut p, [0, 0, 300, 200]);
    assert!(before.0 > 100, "the stamp is drawn");
    let bytes = p.to_bytes().unwrap();
    let mut p = Pdf::from_bytes(&bytes, None).unwrap();
    assert_eq!(p.flatten_annotations(None).unwrap(), 1);
    assert_eq!(ink(&mut p, [0, 0, 300, 200]), before);
}

#[test]
fn a_stamp_with_your_own_wording_is_drawn_in_its_colour_and_unsupported_letters_are_refused() {
    let mut p = page();
    let stamp = Stamp::Custom {
        text: "paid",
        color: StampColor::Green,
        name_and_date: false,
    };
    let id = p
        .add_stamp(0, &stamp, StampPlace::Centre(150.0, 100.0), "")
        .unwrap();
    let d = p.doc.get_dictionary(id).unwrap();
    assert!(d.get(b"Name").is_err(), "no standard icon name");
    assert_eq!(d.get(b"C").unwrap(), &nums(&StampColor::Green.rgb()));
    assert!(ink(&mut p, [0, 0, 300, 200]).0 > 100, "the stamp is drawn");
    let bad = Stamp::Custom {
        text: "日本",
        color: StampColor::Red,
        name_and_date: false,
    };
    assert!(bad.size("").is_err());
    assert!(p
        .add_stamp(0, &bad, StampPlace::Centre(150.0, 100.0), "")
        .is_err());
    assert_eq!(p.annotations().len(), 1);
}

#[test]
fn a_picture_stamp_fills_its_box_keeping_its_shape_and_stretches_when_resized() {
    let mut p = page();
    let black = png(40, 20, [0, 0, 0, 255]);
    let id = p
        .add_stamp(
            0,
            &Stamp::Picture(&black),
            StampPlace::Fill([0.0, 0.0, 300.0, 200.0]),
            "",
        )
        .unwrap();
    let a = &p.annotations()[0];
    let want = [0.0, 25.0, 300.0, 175.0];
    assert!(a.bbox.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.01));
    assert!(ink(&mut p, [5, 30, 295, 170]).1 > 290 * 140 - 10);
    assert_eq!(ink(&mut p, [0, 0, 300, 20]).0, 0);
    // Resizing changes only the box; the drawing is fitted to it.
    p.move_annotation(0, id, [10.0, 10.0, 60.0, 60.0]).unwrap();
    assert!(ink(&mut p, [12, 12, 58, 58]).1 > 46 * 46 - 10);
    assert_eq!(ink(&mut p, [100, 100, 300, 200]).0, 0);
}

#[test]
fn a_stamps_opacity_can_be_changed() {
    let mut p = page();
    let black = png(10, 10, [0, 0, 0, 255]);
    let id = p
        .add_stamp(
            0,
            &Stamp::Picture(&black),
            StampPlace::Fill([50.0, 50.0, 150.0, 150.0]),
            "",
        )
        .unwrap();
    assert!(ink(&mut p, [55, 55, 145, 145]).1 > 8000);
    p.set_annotation(id, None, 0.3, "").unwrap();
    assert!((p.annotations()[0].opacity - 0.3).abs() < 1e-3);
    let (any, dark) = ink(&mut p, [55, 55, 145, 145]);
    assert!(any > 8000 && dark == 0, "faded, not gone");
}

#[test]
fn a_stamp_reads_upright_on_a_turned_page() {
    let mut p = page();
    p.rotate_pages(&[0], 90).unwrap();
    let stamp = Stamp::Text {
        kind: StampKind::Confidential,
        name_and_date: false,
    };
    p.add_stamp(0, &stamp, StampPlace::Centre(100.0, 150.0), "")
        .unwrap();
    // Wider than tall as the page is shown, and drawn across that width.
    let b = p.annotations()[0].bbox;
    assert!(b[2] - b[0] > 2.0 * (b[3] - b[1]));
    let (x0, x1) = (b[0] as usize, b[2] as usize);
    let (y0, y1) = (b[1] as usize, b[3] as usize);
    let mid = (x0 + x1) / 2;
    assert!(ink(&mut p, [x0 + 12, y0 + 8, mid, y1 - 8]).0 > 50);
    assert!(ink(&mut p, [mid, y0 + 8, x1 - 12, y1 - 8]).0 > 50);
}

#[test]
fn a_typed_signature_is_drawn_as_shapes_labelled_and_survives_flattening() {
    let mut p = page();
    let stamp = Stamp::Signature {
        art: SignArt::Typed {
            text: "Sam Smith",
            style: SignStyle::Caveat,
        },
        role: SignRole::Signature,
        color: InkColor::Blue,
        date: true,
    };
    let (w, h) = stamp.size("").unwrap();
    assert!(w <= 150.01 && w > 100.0, "{w} x {h}");
    let id = p
        .add_stamp(0, &stamp, StampPlace::Centre(150.0, 100.0), "Sam")
        .unwrap();
    let d = p.doc.get_dictionary(id).unwrap();
    assert_eq!(
        d.get(b"Name").unwrap().as_name().unwrap(),
        b"LimareSignature"
    );
    assert_eq!(d.get(b"C").unwrap(), &nums(&InkColor::Blue.rgb()));
    let a = &p.annotations()[0];
    assert_eq!(
        (a.kind.clone(), a.label()),
        (AnnotKind::Stamp, "Signature".into())
    );
    assert_eq!(a.subject, "Signature");
    let before = ink(&mut p, [0, 0, 300, 200]);
    assert!(before.0 > 200, "the signature is drawn");
    let bytes = p.to_bytes().unwrap();
    let mut p = Pdf::from_bytes(&bytes, None).unwrap();
    assert_eq!(p.annotations()[0].label(), "Signature");
    assert_eq!(p.flatten_annotations(None).unwrap(), 1);
    assert_eq!(ink(&mut p, [0, 0, 300, 200]), before);
}

#[test]
fn drawn_initials_are_smaller_than_a_signature_and_marks_are_small() {
    let mut p = page();
    let strokes = vec![
        vec![(0.0, 0.0), (30.0, 40.0), (60.0, 0.0)],
        vec![(80.0, 20.0)],
    ];
    let sig = |role| Stamp::Signature {
        art: SignArt::Drawn {
            strokes: &strokes,
            width: 3.0,
        },
        role,
        color: InkColor::Black,
        date: false,
    };
    let (sw, _) = sig(SignRole::Signature).size("").unwrap();
    let (iw, ih) = sig(SignRole::Initials).size("").unwrap();
    assert!(iw < sw && ih <= 30.01);
    p.add_stamp(
        0,
        &sig(SignRole::Initials),
        StampPlace::Centre(60.0, 60.0),
        "",
    )
    .unwrap();
    assert!(
        ink(&mut p, [20, 30, 100, 90]).1 > 20,
        "the strokes are drawn"
    );
    for (i, mark) in Mark::ALL.into_iter().enumerate() {
        let m = Stamp::Mark {
            mark,
            color: InkColor::Black,
        };
        assert_eq!(m.size("").unwrap(), (12.0, 12.0));
        let x = 150.0 + 40.0 * i as f32;
        p.add_stamp(0, &m, StampPlace::Centre(x, 150.0), "")
            .unwrap();
        let x = x as usize;
        assert!(
            ink(&mut p, [x - 6, 144, x + 6, 156]).1 > 5,
            "{mark:?} drawn"
        );
    }
    let labels: Vec<String> = p.annotations().iter().map(|a| a.label()).collect();
    assert_eq!(labels, ["Initials", "Tick", "Cross", "Dot"]);
}

#[test]
fn a_signature_picture_loses_its_paper_and_margins() {
    // Light grey paper with a dark bar in the middle.
    let mut img = image::RgbaImage::from_pixel(60, 30, image::Rgba([225, 225, 220, 255]));
    for y in 12..18 {
        for x in 10..50 {
            img.put_pixel(x, y, image::Rgba([20, 20, 60, 255]));
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    let bytes = out.into_inner();
    let cut = |clear_paper, crop| Stamp::Signature {
        art: SignArt::Picture {
            bytes: &bytes,
            clear_paper,
            crop,
        },
        role: SignRole::Signature,
        color: InkColor::Black,
        date: false,
    };
    let pic = |clear_paper| cut(clear_paper, None);
    // Cropped to the bar and a 2-pixel margin: 44 by 10.
    let (w, h) = pic(true).size("").unwrap();
    assert!((w / h - 4.4).abs() < 0.01, "{w} x {h}");
    let (w, h) = pic(false).size("").unwrap();
    assert!((w / h - 2.0).abs() < 0.01, "{w} x {h}");
    // Only the left half: 30 by 30, or its part of the bar, which runs to the half's edge,
    // with the margin on its other three sides: 22 by 10.
    let half = Some([0.0, 0.0, 0.5, 1.0]);
    let (w, h) = cut(false, half).size("").unwrap();
    assert!((w / h - 1.0).abs() < 0.01, "{w} x {h}");
    let (w, h) = cut(true, half).size("").unwrap();
    assert!((w / h - 2.2).abs() < 0.01, "{w} x {h}");
    let mut p = page();
    p.add_stamp(0, &pic(true), StampPlace::Centre(150.0, 100.0), "")
        .unwrap();
    // The bar shows; the paper's margin around it doesn't.
    assert!(ink(&mut p, [100, 95, 200, 105]).1 > 500);
    assert_eq!(ink(&mut p, [0, 0, 300, 80]).0, 0);
}

#[test]
fn a_signature_reads_upright_on_a_turned_page() {
    let mut p = page();
    p.rotate_pages(&[0], 90).unwrap();
    let stamp = Stamp::Signature {
        art: SignArt::Typed {
            text: "Sam",
            style: SignStyle::Zeyada,
        },
        role: SignRole::Signature,
        color: InkColor::Black,
        date: false,
    };
    let (w, h) = stamp.size("").unwrap();
    p.add_stamp(0, &stamp, StampPlace::Centre(100.0, 150.0), "")
        .unwrap();
    let b = p.annotations()[0].bbox;
    assert!((b[2] - b[0] - w).abs() < 0.01 && (b[3] - b[1] - h).abs() < 0.01);
}

#[test]
fn finishing_signing_draws_in_only_the_fill_and_sign_items() {
    let mut p = page();
    let tick = Stamp::Mark {
        mark: Mark::Tick,
        color: InkColor::Black,
    };
    p.add_stamp(0, &tick, StampPlace::Centre(30.0, 30.0), "")
        .unwrap();
    let date = DateStyle::DayMonthYear.format(2026, 10, 6);
    assert_eq!(date, "6 October 2026");
    let id = p
        .add_sign_text(0, (150.0, 100.0), &date, InkColor::Blue, "Sam")
        .unwrap();
    let a = p.annotations().into_iter().find(|a| a.id == id).unwrap();
    assert_eq!(a.kind, AnnotKind::TextBox);
    assert_eq!(a.contents, "6 October 2026");
    // A stamp and a comment that aren't fill-and-sign items stay as they are.
    let approved = Stamp::Text {
        kind: StampKind::Approved,
        name_and_date: false,
    };
    p.add_stamp(0, &approved, StampPlace::Centre(150.0, 170.0), "")
        .unwrap();
    p.add_note(0, (260.0, 20.0), [1.0, 1.0, 0.0], "a note", "")
        .unwrap();
    assert_eq!(p.sign_items(), 2);
    // The tick and the date, away from the other stamp and the note.
    let before = ink(&mut p, [0, 0, 240, 125]);
    assert!(before.0 > 50);
    assert_eq!(p.finish_signing().unwrap(), 2);
    assert_eq!(p.sign_items(), 0);
    let left: Vec<AnnotKind> = p.annotations().into_iter().map(|a| a.kind).collect();
    assert_eq!(left, [AnnotKind::Stamp, AnnotKind::Note]);
    assert_eq!(
        ink(&mut p, [0, 0, 240, 125]),
        before,
        "drawn in where they were"
    );
}
