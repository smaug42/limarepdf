//! Reading a page has a budget (S1-06, H-07): a page whose drawings draw each other over and
//! over is refused quickly with a typed reason, and what relies on reading it (editing,
//! redacting, search) says so and changes nothing. A drawing that draws itself is read once.

mod common;

use common::apart::{self, Ran};
use limarepdf_core::{FontChoice, Pdf, Query, TextIndex};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};
use std::time::{Duration, Instant};

/// Pages with "Zebra" on them, `pages` of them.
fn zebras(pages: usize) -> Pdf {
    let (mut p, font) = (Pdf::new(), FontChoice::default());
    for i in 0..pages {
        p.insert_blank(i, 612.0, 792.0).unwrap();
        p.add_text(i, (72.0, 100.0), "Zebra", font, 20.0, [0.0; 3])
            .unwrap();
    }
    p
}

/// Page `page` of `p` draws a form that, with resources of its own, draws itself `times`
/// times over.
fn draw_itself(p: &mut Pdf, page: usize, times: usize) {
    let me = p.doc.new_object_id();
    let form = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => dictionary! { "XObject" => dictionary! { "Me" => me } },
        },
        "/Me Do ".repeat(times).into_bytes(),
    );
    p.doc.objects.insert(me, form.into());
    draw_form(p, page, me);
}

/// Page `page` of `p` draws `form` (as `/Me`) after what it drew.
fn draw_form(p: &mut Pdf, page: usize, form: ObjectId) {
    let id = p.page_ids()[page];
    let content = [p.doc.get_page_content(id).as_slice(), b" q /Me Do Q"].concat();
    let content = p.doc.add_object(Stream::new(Dictionary::new(), content));
    let page = p.doc.get_dictionary(id).unwrap();
    let mut res = match page.get(b"Resources").ok().cloned() {
        Some(Object::Reference(r)) => p.doc.get_dictionary(r).unwrap().clone(),
        Some(Object::Dictionary(d)) => d,
        _ => Dictionary::new(),
    };
    res.set("XObject", dictionary! { "Me" => form });
    let d = p.doc.get_dictionary_mut(id).unwrap();
    d.set("Contents", content);
    d.set("Resources", res);
}

/// Page `page` of `p` draws a tree of forms `depth` deep, each drawing the one below it
/// `width` times; the last ends a path that isn't there (`n`), one operator.
fn draw_tree(p: &mut Pdf, page: usize, width: usize, depth: usize) {
    let form = |content: String, res: Dictionary| {
        let d = dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => res,
        };
        Stream::new(d, content.into_bytes())
    };
    let mut below = p.doc.add_object(form("n".into(), Dictionary::new()));
    for _ in 0..depth {
        let res = dictionary! { "XObject" => dictionary! { "Me" => below } };
        below = p.doc.add_object(form("/Me Do ".repeat(width), res));
    }
    draw_form(p, page, below);
}

/// The steps reading a page drawing [`draw_tree`]'s tree takes, counted from the rule, not
/// by the parser: an operator is a step, and so is each form opened (its content is under
/// a KiB, so nothing more for its size).
fn tree_steps(width: u64, depth: u32) -> u64 {
    // The leaf: opened, and its one operator.
    let mut form = 2;
    for _ in 0..depth {
        // Opened, and `width` operators each opening the form below.
        form = 1 + width * (1 + form);
    }
    // The page draws "Zebra" (BT, Tf, Td, Tj, ET as add_text writes it, measured below
    // with no tree) and q, Do, Q.
    3 + form
}

/// The steps a page of [`zebras`] alone takes.
fn zebra_steps() -> u64 {
    use limarepdf_core::budget_seam;
    let p = zebras(1);
    let (read, taken) = budget_seam::with_limit(u64::MAX, || p.page_objects(0).map(|_| ()));
    read.unwrap();
    assert_eq!(taken.len(), 1, "one budget for the page: {taken:?}");
    taken[0]
}

/// The budget is charged across every form a page draws, however often: a tree of forms
/// three wide and four deep (121 forms opened) is read within the steps it needs, and
/// refused one step short of them, or with 40.
#[test]
fn a_page_drawing_forms_over_and_over_is_refused_past_its_budget() {
    use limarepdf_core::{budget_seam, TooComplex};
    let mut p = zebras(1);
    draw_tree(&mut p, 0, 3, 4);
    common::seeds::pdf(&p);
    let needed = zebra_steps() + tree_steps(3, 4);
    let (read, taken) = budget_seam::with_limit(u64::MAX, || p.page_objects(0).map(|_| ()));
    read.unwrap();
    assert_eq!(taken, [needed]);
    let (read, taken) = budget_seam::with_limit(needed, || p.page_objects(0).map(|_| ()));
    read.unwrap();
    assert_eq!(taken, [needed]);
    for short in [needed - 1, 40] {
        let (read, taken) = budget_seam::with_limit(short, || p.page_objects(0).map(|_| ()));
        let e = read.unwrap_err();
        let too = e.downcast_ref::<TooComplex>().expect("too complex");
        assert_eq!(too.limit, short, "{e:#}");
        assert!(taken.len() == 1 && taken[0] <= short, "{taken:?}");
    }
}

/// A form whose content can't be decoded is passed over, as before: the rest of the page
/// is read.
#[test]
fn a_form_that_can_t_be_decoded_is_passed_over() {
    let mut p = zebras(1);
    let broken = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Filter" => "FlateDecode",
        },
        b"not deflated at all".to_vec(),
    ));
    draw_form(&mut p, 0, broken);
    let objs = p.page_objects(0).unwrap();
    assert!(objs.iter().any(|o| o.text.as_deref() == Some("Zebra")));
    assert!(p.find_text_boxes(0, "zebra").unwrap().len() == 1);
}

/// The name page `page` of `p` knows its font by, as [`zebras`] wrote it.
fn font_name(p: &Pdf, page: usize) -> String {
    let id = p.page_ids()[page];
    let res = p.doc.get_dictionary(id).unwrap().get(b"Resources").unwrap();
    let res = p.doc.dereference(res).unwrap().1.as_dict().unwrap();
    let fonts = p.doc.dereference(res.get(b"Font").unwrap()).unwrap().1;
    let (name, _) = fonts.as_dict().unwrap().iter().next().unwrap();
    String::from_utf8(name.clone()).unwrap()
}

/// Page `page` of `p` draws `A`, a form writing "Alpha" that draws `B`, writing "Beta",
/// which draws `A` again (as pdf.js's `issue19800-eq.pdf`).
fn draw_each_other(p: &mut Pdf, page: usize) {
    let font = font_name(p, page);
    let form = |word: &str, next: &str| {
        let content = format!("BT /{font} 20 Tf 72 300 Td ({word}) Tj ET /{next} Do");
        let d = dictionary! { "Type" => "XObject", "Subtype" => "Form" };
        Stream::new(d, content.into_bytes())
    };
    let a = p.doc.add_object(form("Alpha", "B"));
    let b = p.doc.add_object(form("Beta", "A"));
    draw_form(p, page, a);
    let id = p.page_ids()[page];
    let page = p.doc.get_dictionary_mut(id).unwrap();
    let res = page.get_mut(b"Resources").unwrap().as_dict_mut().unwrap();
    res.set("XObject", dictionary! { "Me" => a, "A" => a, "B" => b });
}

/// Two forms drawing each other are each read once, on the way down: the drawing that
/// would repeat is passed over, and the rest of the page is read.
#[test]
fn forms_drawing_each_other_are_read_once() {
    let mut p = zebras(1);
    draw_each_other(&mut p, 0);
    common::seeds::pdf(&p);
    let text = p.page_text(0);
    let words: Vec<&str> = text.split_whitespace().collect();
    assert_eq!(words, ["Zebra", "Alpha", "Beta"], "{text}");
    let boxes = p.find_text_boxes(0, "beta").unwrap();
    assert_eq!(boxes.len(), 1);
}

/// The child process of the next test: reads a page whose form draws itself ten times over,
/// which unchecked is read 10⁸ times.
#[test]
#[ignore = "started by a_page_drawing_itself_is_read_at_once"]
fn too_complex_child() {
    if std::env::var("TOO_COMPLEX_CHILD").is_err() {
        return;
    }
    let mut p = zebras(1);
    draw_itself(&mut p, 0, 10);
    common::seeds::pdf(&p);
    match p.page_objects(0) {
        Ok(objs) => {
            let text: Vec<_> = objs.into_iter().filter_map(|o| o.text).collect();
            println!("complex|read|{}", text.join(" "));
        }
        Err(e) => println!("complex|refused|{e:#}"),
    }
}

/// A page whose form draws itself is read at once, the form once, not until the nesting
/// limit stops it (ten times over, eight deep: 10⁸ forms read).
#[test]
fn a_page_drawing_itself_is_read_at_once() {
    let start = Instant::now();
    let env = [("TOO_COMPLEX_CHILD", "1".to_string())];
    let ran = apart::run("too_complex_child", &env, Duration::from_secs(30));
    let Ran::Ended(text, status) = ran else {
        panic!("not read within 30 s");
    };
    let said = apart::reports(&text, "complex|");
    assert_eq!(said.len(), 1, "{}", apart::why_stopped(status));
    assert_eq!(said[0], "read|Zebra");
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "{:?}",
        start.elapsed()
    );
}

/// A page too complex to read is refused for editing, with nothing changed; its words
/// aren't found, and the page's text is empty, as for any page that can't be read.
#[test]
fn a_page_too_complex_to_read_isn_t_read_or_edited() {
    use limarepdf_core::{budget_seam, TooComplex};
    let mut p = zebras(1);
    draw_tree(&mut p, 0, 3, 4);
    common::seeds::pdf(&p);
    let ((), _) = budget_seam::with_limit(40, || {
        let e = p.page_objects(0).unwrap_err();
        let too = e.downcast_ref::<TooComplex>().expect("too complex");
        assert_eq!(too.what, "reading the page");
        assert!(p.page_words(0).is_err());
        assert!(p.find_text_boxes(0, "zebra").is_err());
        assert_eq!(p.page_text(0), "");
        let before = p.state_before();
        let e = p.delete_object(0, 0).unwrap_err();
        assert!(e.downcast_ref::<TooComplex>().is_some(), "{e:#}");
        let moved = [1.0, 0.0, 0.0, 1.0, 5.0, 5.0];
        let e = p.transform_object(0, 0, moved).unwrap_err();
        assert!(e.downcast_ref::<TooComplex>().is_some(), "{e:#}");
        assert_eq!(p.state_since(&before), []);
    });
}

/// A redaction over a page too complex to read is refused before any page changes: the
/// text under each mark is read first.
#[test]
fn a_redaction_over_a_page_too_complex_to_read_changes_nothing() {
    use limarepdf_core::{budget_seam, TooComplex};
    let mut p = zebras(2);
    draw_tree(&mut p, 1, 3, 4);
    let before = p.state_before();
    let whole = [0.0, 0.0, 612.0, 792.0];
    let (e, _) = budget_seam::with_limit(40, || p.redact(&[(0, whole), (1, whole)]));
    let e = e.unwrap_err();
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(too.what, "reading the page", "{e:#}");
    assert_eq!(p.state_since(&before), []);
}

/// Search reads the pages it can and names those it can't, with why.
#[test]
fn search_names_the_pages_it_can_t_read() {
    use limarepdf_core::budget_seam;
    let mut p = zebras(3);
    draw_tree(&mut p, 1, 3, 4);
    let mut index = TextIndex::new(&p);
    let (read, _) = budget_seam::with_limit(40, || index.read(&p, 0..3, &|| false));
    assert!(read);
    let found = index.find(&Query::new("zebra"), 0..3, 0);
    let pages: Vec<usize> = found.hits.iter().map(|h| h.page).collect();
    assert_eq!(pages, [0, 2]);
    assert_eq!(found.unread, [(1, "it is too complex to read".to_string())]);
    // Only the pages looked in are named.
    assert!(index.find(&Query::new("zebra"), 2..3, 0).unread.is_empty());
}
