//! Pages with more annotations than LimarePDF shows (10,000): removing comments,
//! attachments and replies, applying redaction marks, redacting and flattening still deal
//! with every one, and with annotations written into the page's list directly.

mod common;

use common::saved::{tools, Saved};
use limarepdf_core::print::{Orientation, PageSetup, Paper, PerSheet, Scale};
use limarepdf_core::{Hidden, Pdf};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

/// How many annotations of a page LimarePDF shows.
const SHOWN: usize = 10_000;

/// A 300 × 200 pt page showing `content` with `/F1` (Helvetica).
fn page(content: &str) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let pg = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(pg).unwrap();
    d.set("Contents", s);
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    p
}

fn rect(r: [i64; 4]) -> Object {
    r.map(Object::from).to_vec().into()
}

/// A comment (sticky note) at `r` (page space) saying `text`.
fn note(r: [i64; 4], text: &str) -> Dictionary {
    dictionary! {
        "Type" => "Annot", "Subtype" => "Text", "Rect" => rect(r),
        "Contents" => Object::string_literal(text),
    }
}

fn add(p: &mut Pdf, d: Dictionary) -> ObjectId {
    p.doc.add_object(d)
}

/// The page's annotations as listed now.
fn annots(p: &Pdf) -> Vec<Object> {
    let d = p.doc.get_dictionary(p.page_ids()[0]).unwrap();
    d.get(b"Annots")
        .map_or(Vec::new(), |a| a.as_array().unwrap().clone())
}

fn set_annots(p: &mut Pdf, list: Vec<Object>) {
    let pg = p.page_ids()[0];
    p.doc.get_dictionary_mut(pg).unwrap().set("Annots", list);
}

/// [`SHOWN`] empty comments in the bottom-left corner, after the page's own.
fn fill(p: &mut Pdf) {
    let mut list = annots(p);
    for _ in 0..SHOWN {
        list.push(add(p, note([0, 0, 5, 5], "")).into());
    }
    set_annots(p, list);
}

/// Adds `d` to the end of the page's annotations.
fn push(p: &mut Pdf, d: Dictionary) -> ObjectId {
    let id = add(p, d);
    let mut list = annots(p);
    list.push(id.into());
    set_annots(p, list);
    id
}

/// A text string in UTF-16, as PDF makers write text beyond Latin-1.
fn utf16(s: &str) -> Object {
    let mut b = vec![0xFE, 0xFF];
    b.extend(s.encode_utf16().flat_map(u16::to_be_bytes));
    Object::String(b, lopdf::StringFormat::Hexadecimal)
}

/// A drawing of a 5 × 5 pt blue square, for annotations to share.
fn square_drawing(p: &mut Pdf) -> ObjectId {
    let form = dictionary! {
        "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 5, 5]),
    };
    p.doc
        .add_object(Stream::new(form, b"0 0 1 rg 0 0 5 5 re f".to_vec()))
}

/// A square mark at `r` (page space) drawn by `ap`.
fn square(r: [i64; 4], ap: ObjectId) -> Dictionary {
    dictionary! {
        "Type" => "Annot", "Subtype" => "Square", "Rect" => rect(r), "F" => 4,
        "AP" => dictionary! { "N" => ap },
    }
}

/// [`SHOWN`] squares with drawings of their own in the bottom-left corner, after the
/// page's own annotations: flattening draws every one.
fn fill_drawn(p: &mut Pdf) {
    let ap = square_drawing(p);
    let mut list = annots(p);
    for _ in 0..SHOWN {
        list.push(add(p, square([0, 0, 5, 5], ap)).into());
    }
    set_annots(p, list);
}

/// A text box at `r` saying `words`, without a drawing of its own.
fn text_box(r: [i64; 4], words: &str) -> Dictionary {
    dictionary! {
        "Type" => "Annot", "Subtype" => "FreeText", "Rect" => rect(r), "F" => 4,
        "Contents" => utf16(words), "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
    }
}

/// The subtype and comment text of each annotation the saved file's page lists, in order,
/// read from the file itself.
fn listed(s: &Saved) -> Vec<(String, String)> {
    let page = *s.doc.get_pages().values().next().unwrap();
    let Ok(list) = s.doc.get_dictionary(page).unwrap().get(b"Annots") else {
        return Vec::new();
    };
    let text = |o: Option<&Object>| match o {
        Some(Object::String(b, _)) => String::from_utf8_lossy(b).into_owned(),
        Some(Object::Name(n)) => String::from_utf8_lossy(n).into_owned(),
        _ => String::new(),
    };
    list.as_array()
        .unwrap()
        .iter()
        .map(|a| {
            let d = s.doc.get_dictionary(a.as_reference().unwrap()).unwrap();
            (text(d.get(b"Subtype").ok()), text(d.get(b"Contents").ok()))
        })
        .collect()
}

#[test]
fn removing_comments_takes_every_one() {
    let mut p = page("");
    fill(&mut p);
    let mut list = annots(&p);
    // One written into the list directly, and one past those shown.
    list.insert(
        5,
        Object::Dictionary(note([20, 20, 40, 40], "MARKER-DIRECT")),
    );
    list.push(add(&mut p, note([20, 20, 40, 40], "MARKER-LAST")).into());
    set_annots(&mut p, list);

    let rep = p.remove_hidden(&[Hidden::Comments]).unwrap();
    assert_eq!(rep.count(Hidden::Comments), SHOWN + 2);
    assert!(annots(&p).is_empty());
    Saved::of(&mut p).assert_gone(&["MARKER-DIRECT", "MARKER-LAST"]);
}

/// A page with "PAGESECRET" on its left half, [`SHOWN`] comments there, and past them: one
/// more there, one written into the list directly, and, on the right half, a comment
/// repeating the secret with its popup and a reply. Returns the right half's comment and
/// popup.
fn crowded() -> (Pdf, ObjectId, ObjectId) {
    let mut p = page("BT /F1 12 Tf 20 100 Td (PAGESECRET) Tj ET");
    fill(&mut p);
    let mut list = annots(&p);
    list.push(Object::Dictionary(note([20, 20, 40, 40], "MARKER-DIRECT")));
    list.push(add(&mut p, note([30, 30, 50, 50], "MARKER-LAST")).into());
    let kept = add(
        &mut p,
        note([200, 20, 280, 60], "MARKER-KEPT-OUTSIDE PAGESECRET"),
    );
    let popup = add(
        &mut p,
        dictionary! {
            "Type" => "Annot", "Subtype" => "Popup", "Rect" => rect([200, 100, 290, 180]),
            "Parent" => kept,
        },
    );
    p.doc.get_dictionary_mut(kept).unwrap().set("Popup", popup);
    let mut reply = note([250, 20, 260, 30], "MARKER-KEPT-REPLY");
    reply.set("IRT", kept);
    let reply = add(&mut p, reply);
    list.extend([kept.into(), popup.into(), reply.into()]);
    set_annots(&mut p, list);
    (p, kept, popup)
}

#[test]
fn redacting_removes_every_annotation_touching_a_mark() {
    let (mut p, kept, popup) = crowded();

    let rep = p.redact(&[(0, [0.0, 0.0, 150.0, 200.0])]).unwrap();
    assert_eq!(rep.pages[0].annots, SHOWN + 2);
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    // The comment past those shown still repeats what was removed, and the check says so.
    assert!(
        rep.elsewhere
            .iter()
            .any(|(_, places)| places.iter().any(|w| w.contains("comment on page 1"))),
        "{:?}",
        rep.elsewhere
    );
    let left: Vec<ObjectId> = annots(&p)
        .iter()
        .filter_map(|o| o.as_reference().ok())
        .collect();
    assert!(left.contains(&kept) && left.contains(&popup), "{left:?}");
    let s = Saved::of(&mut p);
    s.assert_gone(&["MARKER-DIRECT", "MARKER-LAST"]);
    s.assert_kept(&["MARKER-KEPT-OUTSIDE", "MARKER-KEPT-REPLY"]);
}

/// "FIRSTWORD" and "SECONDWORD" each under a redaction mark, the second mark past
/// [`SHOWN`] comments.
fn marked() -> Pdf {
    let mut p =
        page("BT /F1 12 Tf 20 100 Td (FIRSTWORD) Tj ET BT /F1 12 Tf 180 100 Td (SECONDWORD) Tj ET");
    p.add_redaction_mark(0, &[[15.0, 80.0, 110.0, 110.0]], "Tester")
        .unwrap();
    fill(&mut p);
    p.add_redaction_mark(0, &[[175.0, 80.0, 295.0, 110.0]], "Tester")
        .unwrap();
    assert_eq!(annots(&p).len(), SHOWN + 2);
    p
}

#[test]
fn a_mark_past_those_shown_is_applied() {
    let mut p = marked();
    assert_eq!(p.redaction_marks().len(), 2);

    let rep = p.apply_redactions().unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    // The marks are gone; the comments stay.
    assert_eq!(annots(&p).len(), SHOWN);
    assert!(p.redaction_marks().is_empty());
    Saved::of(&mut p).assert_gone(&["FIRSTWORD", "SECONDWORD"]);
}

#[test]
fn an_attachment_past_those_shown_is_removed() {
    let mut p = page("");
    let file = p.doc.add_object(Stream::new(
        dictionary! { "Type" => "EmbeddedFile" },
        b"MARKER-ATTACHED-DATA".to_vec(),
    ));
    let spec = add(
        &mut p,
        dictionary! {
            "Type" => "Filespec", "F" => Object::string_literal("a.txt"),
            "EF" => dictionary! { "F" => file },
        },
    );
    fill(&mut p);
    let mut list = annots(&p);
    list.push(
        add(
            &mut p,
            dictionary! {
                "Type" => "Annot", "Subtype" => "FileAttachment", "Rect" => rect([20, 20, 40, 40]),
                "FS" => spec,
            },
        )
        .into(),
    );
    set_annots(&mut p, list);

    let rep = p.remove_hidden(&[Hidden::Attachments]).unwrap();
    assert_eq!(rep.count(Hidden::Attachments), 1);
    assert_eq!(annots(&p).len(), SHOWN);
    Saved::of(&mut p).assert_gone(&["MARKER-ATTACHED-DATA"]);
}

#[test]
fn a_reply_past_those_shown_goes_with_its_comment() {
    let mut p = page("");
    let gone = add(&mut p, note([20, 20, 40, 40], "MARKER-GONE"));
    let other = add(&mut p, note([60, 20, 80, 40], "MARKER-OTHER"));
    set_annots(&mut p, vec![gone.into(), other.into()]);
    fill(&mut p);
    let mut list = annots(&p);
    for (to, text) in [(gone, "MARKER-GONE-REPLY"), (other, "MARKER-OTHER-REPLY")] {
        let mut reply = note([20, 60, 40, 80], text);
        reply.set("IRT", to);
        list.push(add(&mut p, reply).into());
    }
    set_annots(&mut p, list);

    assert_eq!(p.remove_annotations(|a| a.id == gone).unwrap(), 2);
    let s = Saved::of(&mut p);
    s.assert_gone(&["MARKER-GONE"]);
    s.assert_kept(&["MARKER-OTHER", "MARKER-OTHER-REPLY"]);
}

/// A comment past those shown, without the `/P` naming its page, is still found on its
/// page: answering it works (it was "on no page").
#[test]
fn a_comment_past_those_shown_can_be_answered() {
    let mut p = page("");
    fill(&mut p);
    let last = add(&mut p, note([20, 20, 40, 40], "MARKER-LAST"));
    let mut list = annots(&p);
    list.push(last.into());
    set_annots(&mut p, list);

    let reply = p.add_reply(last, "MARKER-ANSWER", "Sam").unwrap();
    let d = p.doc.get_dictionary(reply).unwrap();
    assert_eq!(
        d.get(b"P").unwrap().as_reference().unwrap(),
        p.page_ids()[0]
    );
}

/// H-06: a text box past those shown, without a drawing of its own, is drawn into the page
/// by flattening, as the first 10,000 are. It was removed without being drawn.
#[test]
fn flattening_draws_a_text_box_past_those_shown() {
    let mut p = page("");
    fill_drawn(&mut p);
    push(&mut p, text_box([20, 100, 200, 140], "LAST-LOST"));

    let done = p.flatten_annotations(None).unwrap();
    assert_eq!((done.drawn, done.kept), (SHOWN + 1, vec![]));
    assert!(annots(&p).is_empty());
    let back = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    let text = back.page_text(0);
    assert!(text.contains("LAST-LOST"), "{text}");
}

/// A text box past those shown whose text its font can't write is refused with nothing
/// changed, as one among the first 10,000 is (BUG-39), rather than removed undrawn.
#[test]
fn flattening_refuses_a_text_box_past_those_shown_it_cannot_write() {
    let mut p = page("");
    fill_drawn(&mut p);
    push(&mut p, text_box([20, 100, 200, 140], "שלום LAST-LOST"));
    let before = p.state_before();

    let e = p.flatten_annotations(None).unwrap_err();
    assert_eq!(
        e.to_string(),
        "LimarePDF can't draw the text box on page 1: it has \"שלום\", in a script LimarePDF \
         can't write yet (right-to-left writing, and scripts whose letters join, aren't \
         supported); nothing was flattened"
    );
    assert_eq!(p.state_since(&before), []);
}

/// A mark flattening should draw but can't place on the page stays as it was, with its
/// popup and its reply, among the first 10,000 or past them: S1-05's design keeps it and
/// says so. It was removed without being drawn. A hidden mark still goes undrawn.
#[test]
fn flattening_keeps_what_it_cannot_draw_with_its_popup_and_reply() {
    for past in [false, true] {
        let mut p = page("");
        if past {
            fill_drawn(&mut p);
        }
        // A drawing without the box that places it on the page.
        let unboxed = p.doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form" },
            b"0 0 1 rg 0 0 50 50 re f".to_vec(),
        ));
        let mut mark = square([100, 100, 150, 150], unboxed);
        mark.set("Contents", Object::string_literal("MARKER-UNBOXED"));
        let mark = push(&mut p, mark);
        let popup = push(
            &mut p,
            dictionary! {
                "Type" => "Annot", "Subtype" => "Popup", "Rect" => rect([150, 100, 250, 150]),
                "Parent" => mark,
            },
        );
        p.doc.get_dictionary_mut(mark).unwrap().set("Popup", popup);
        let mut reply = note([100, 100, 110, 110], "MARKER-REPLY");
        reply.set("IRT", mark);
        push(&mut p, reply);
        // Ink without its strokes: no drawing, and none LimarePDF can make.
        push(
            &mut p,
            dictionary! {
                "Type" => "Annot", "Subtype" => "Ink", "Rect" => rect([20, 150, 60, 190]),
                "F" => 4, "Contents" => Object::string_literal("MARKER-STROKELESS"),
            },
        );
        // The controls: a mark it can draw is drawn, and a hidden one goes undrawn.
        let ap = square_drawing(&mut p);
        push(&mut p, square([20, 20, 25, 25], ap));
        let mut hidden = square([30, 20, 35, 25], ap);
        hidden.set("F", 2);
        hidden.set("Contents", Object::string_literal("MARKER-HIDDEN"));
        push(&mut p, hidden);

        // The two it can't draw are reported on their page; their popup and reply stay
        // with them, and aren't counted.
        let done = p.flatten_annotations(None).unwrap();
        assert_eq!(done.kept, [0, 0], "past those shown: {past}");
        let drawn = if past { SHOWN + 1 } else { 1 };
        assert_eq!(done.drawn, drawn, "past those shown: {past}");
        let s = Saved::of(&mut p);
        let kept = [
            ("Square", "MARKER-UNBOXED"),
            ("Popup", ""),
            ("Text", "MARKER-REPLY"),
            ("Ink", "MARKER-STROKELESS"),
        ]
        .map(|(a, b)| (a.to_string(), b.to_string()));
        assert_eq!(listed(&s), kept, "past those shown: {past}");
        // The popup still belongs to the mark, and the mark to the page.
        let page = *s.doc.get_pages().values().next().unwrap();
        let ids: Vec<ObjectId> = (s.doc.get_dictionary(page).unwrap().get(b"Annots"))
            .and_then(|a| a.as_array())
            .unwrap()
            .iter()
            .map(|a| a.as_reference().unwrap())
            .collect();
        let at = |id: ObjectId, key: &[u8]| {
            let d = s.doc.get_dictionary(id).unwrap();
            d.get(key).and_then(|o| o.as_reference()).unwrap()
        };
        assert_eq!(at(ids[0], b"Popup"), ids[1]);
        assert_eq!(at(ids[1], b"Parent"), ids[0]);
        assert_eq!(at(ids[2], b"IRT"), ids[0]);
        // The mark it can draw is drawn into the page.
        let content = s.doc.get_page_content(page);
        assert!(String::from_utf8_lossy(&content).contains(" Do"));
    }
}

/// Paper the size of [`page`] with no margins: a sheet is its page as it is.
fn as_is() -> PageSetup {
    PageSetup {
        paper: Paper {
            id: "page".into(),
            label: "page".into(),
            size: (300.0, 200.0),
            margins: Some([0.0; 4]),
        },
        orientation: Orientation::Automatic,
        scale: Scale::ShrinkOversized,
        per_sheet: PerSheet::default(),
        booklet: None,
    }
}

/// H-06: printing draws a text box past those shown, without a drawing of its own, as it
/// does the first 10,000. It was left off the paper.
#[test]
fn printing_draws_a_text_box_past_those_shown() {
    let mut p = page("");
    fill_drawn(&mut p);
    push(&mut p, text_box([20, 100, 200, 140], "LAST-LOST"));
    let sheets = p.print_sheets(&[0], &as_is()).unwrap();
    let text = sheets.page_text(0);
    assert!(text.contains("LAST-LOST"), "{text}");
}

/// H-06: pictures of the page, `render`'s and the app's, draw a text box past those shown,
/// without a drawing of its own, as they do the first 10,000. It was left out.
#[test]
fn pictures_draw_a_text_box_past_those_shown() {
    let mut p = page("");
    fill_drawn(&mut p);
    push(&mut p, text_box([20, 100, 200, 140], "LAST-LOST"));
    for (what, bytes) in [
        ("render", p.render_snapshot().unwrap()),
        ("app", p.app_snapshot().unwrap()),
    ] {
        let s = Saved::new(bytes);
        let places = s.places("LAST-LOST");
        assert!(
            places
                .iter()
                .any(|p| p.ends_with("a string in its content")),
            "{what}: {places:?}"
        );
    }
}

/// Ghostscript, reading the redacted output on its own:
/// `cargo test -p limarepdf-core --test many_annotations -- --ignored cross_check`.
/// Poppler can't: it refuses a page listing 10,000 annotations or more as malformed.
#[test]
#[ignore]
fn cross_check_crowded_pages_with_ghostscript() {
    let mut p = marked();
    let before = tools::gs_text(&p.to_bytes().unwrap());
    assert!(
        before.contains("FIRSTWORD") && before.contains("SECONDWORD"),
        "{before}"
    );
    p.apply_redactions().unwrap();
    let after = tools::gs_text(&p.to_bytes().unwrap());
    assert!(
        !after.contains("FIRSTWORD") && !after.contains("SECONDWORD"),
        "{after}"
    );

    let (mut p, _, _) = crowded();
    assert!(tools::gs_text(&p.to_bytes().unwrap()).contains("PAGESECRET"));
    p.redact(&[(0, [0.0, 0.0, 150.0, 200.0])]).unwrap();
    let after = tools::gs_text(&p.to_bytes().unwrap());
    assert!(!after.contains("PAGESECRET"), "{after}");
}

/// Poppler, reading the flattened page on its own: the text box past those shown is in the
/// page. The page lists no annotations once flattened, so Poppler reads it.
#[test]
#[ignore]
fn cross_check_flattening_past_those_shown_with_poppler() {
    let mut p = page("");
    fill_drawn(&mut p);
    push(&mut p, text_box([20, 100, 200, 140], "LAST-LOST"));
    p.flatten_annotations(None).unwrap();
    let text = tools::pdftotext(&p.to_bytes().unwrap());
    assert!(text.contains("LAST-LOST"), "{text}");
}
