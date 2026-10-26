//! Pages with more annotations than LimarePDF shows (10,000): removing comments,
//! attachments and replies, applying redaction marks and redacting still deal with every
//! one, and with annotations written into the page's list directly.

mod common;

use common::saved::{tools, Saved};
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
