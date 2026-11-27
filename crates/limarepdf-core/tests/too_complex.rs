//! Reading a page has a budget (S1-06, H-07): a page whose drawings draw each other over and
//! over is refused quickly with a typed reason, and what relies on reading it (editing,
//! redacting, search, finding its pictures) says so and changes nothing. A drawing that draws
//! itself is read once. Drawing annotations has a budget too: flattening and redacting
//! annotations too complex to draw are refused, and listing them places those past it at
//! their own `/Rect`. So does reading their points and text: a listing lists those past
//! it unread, and removing information is refused past it. A text is charged what decoding
//! it takes, a failure too. Changing one reads it alone, and
//! is refused past the points it is drawn with rather than write them back cut. Reading a
//! form's fields has a budget too, and what relies on reading them all is refused past it.

use crate::common;

use common::apart::{self, Ran};
use limarepdf_core::{render, FontChoice, Pdf, Query, TextIndex};
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

/// A spot colour whose tint transform is stitched `depth` levels deep, each level listing the
/// one below `width` times, down to one exponential function.
fn shared_tint(p: &mut Pdf, width: usize, depth: usize) -> Object {
    let mut part = p.doc.add_object(dictionary! {
        "FunctionType" => 2, "Domain" => vec![0.into(), 1.into()],
        "C0" => vec![0.into()], "C1" => vec![1.into()], "N" => 1,
    });
    for _ in 0..depth {
        let bounds = (1..width).map(|i| Object::Real(i as f32 / width as f32));
        part = p.doc.add_object(dictionary! {
            "FunctionType" => 3, "Domain" => vec![0.into(), 1.into()],
            "Functions" => vec![Object::Reference(part); width],
            "Bounds" => bounds.collect::<Vec<_>>(),
            "Encode" => (0..width).flat_map(|_| [0.into(), 1.into()]).collect::<Vec<Object>>(),
        });
    }
    Object::Array(vec![
        "Separation".into(),
        "Spot".into(),
        "DeviceGray".into(),
        part.into(),
    ])
}

/// The steps reading [`shared_tint`]'s space takes, counted from the rule: the space and its
/// alternate, each function once, and each part a stitching function lists.
fn tint_steps(width: u64, depth: u64) -> u64 {
    2 + (depth + 1) + depth * width
}

/// Page 0 of `p` draws a form with the colour spaces `spaces`, whose content is `content`.
fn draw_colours(p: &mut Pdf, spaces: Dictionary, content: String) {
    let form = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => dictionary! { "ColorSpace" => spaces },
        },
        content.into_bytes(),
    ));
    draw_form(p, 0, form);
}

/// Page 0 of `p` is read within the `needed` steps, taking them all, and refused one short.
fn read_within(p: &Pdf, needed: u64) {
    use limarepdf_core::{budget_seam, TooComplex};
    let (read, taken) = budget_seam::with_limit(needed, || p.page_objects(0).map(|_| ()));
    read.unwrap();
    assert_eq!(taken, [needed]);
    let (read, taken) = budget_seam::with_limit(needed - 1, || p.page_objects(0).map(|_| ()));
    let e = read.unwrap_err();
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(too.limit, needed - 1, "{e:#}");
    assert!(taken.len() == 1 && taken[0] < needed, "{taken:?}");
}

/// Reading a colour space is charged to its page's budget, each function read once: a page
/// choosing a spot colour stitched three levels deep from parts it shares (8,421 functions
/// read before) is read within the steps it needs, and refused one step short.
#[test]
fn a_page_s_colour_spaces_are_charged_to_its_budget() {
    let mut p = zebras(1);
    let space = shared_tint(&mut p, 20, 3);
    draw_colours(&mut p, dictionary! { "CS0" => space }, "/CS0 cs".into());
    // q, Do and Q; the form opened; its one operator.
    read_within(&p, zebra_steps() + 3 + 1 + 1 + tint_steps(20, 3));
}

/// A spot colour whose tint transform is a calculator program of `steps` steps, leaving the
/// tint as it was; and the program's length.
fn calculated_tint(p: &mut Pdf, steps: usize) -> (Object, u64) {
    let code = format!("{{ {}}}", "dup pop ".repeat(steps / 2));
    let len = code.len() as u64;
    let tint = p.doc.add_object(Stream::new(
        dictionary! {
            "FunctionType" => 4, "Domain" => vec![0.into(), 1.into()],
            "Range" => vec![0.into(), 1.into()],
        },
        code.into_bytes(),
    ));
    let space = vec![
        "Separation".into(),
        "Spot".into(),
        "DeviceGray".into(),
        tint.into(),
    ];
    (Object::Array(space), len)
}

/// Setting a colour is charged what showing it takes, each time: a spot colour whose tint
/// transform runs 8,192 steps, chosen and set 40 times to fill and as many to stroke, is
/// charged 8 more steps each time (one for each 1,024 values read and steps run: the tint,
/// the program and the grey it gives). Before, each was charged only as an operator.
#[test]
fn a_colour_is_charged_what_showing_it_takes_each_time_it_is_set() {
    let mut p = zebras(1);
    let (space, len) = calculated_tint(&mut p, 8192);
    let times = 40;
    let fill = " 0.5 scn".repeat(times);
    let stroke = " 0.5 SCN".repeat(times);
    let content = format!("/CS0 cs{fill} /CS0 CS{stroke}");
    draw_colours(&mut p, dictionary! { "CS0" => space }, content);
    // The space and its alternate read once, the program a step and one for each KiB.
    let read = 2 + 1 + len / 1024;
    let each = (1 + 8192 + 1) / 1024;
    let set = 2 * times as u64 + 2;
    // q, Do and Q; the form opened (under a KiB).
    read_within(&p, zebra_steps() + 3 + 1 + read + set * (1 + each));
}

/// Setting a colour of many inks is charged them each time: 4,096 inks chosen 40 times, and
/// an indexed colour over them set 40 times, are charged 4 more steps each time. Before,
/// each was charged only as an operator.
#[test]
fn a_colour_of_many_inks_is_charged_them_each_time_it_is_set() {
    let mut p = zebras(1);
    let inks: u64 = 4096;
    let tint = p.doc.add_object(dictionary! {
        "FunctionType" => 2, "Domain" => vec![0.into(), 1.into()],
        "C0" => vec![0.into()], "C1" => vec![1.into()], "N" => 1,
    });
    let names = (0..inks).map(|i| Object::Name(format!("Ink{i}").into_bytes()));
    let many = Object::Array(vec![
        "DeviceN".into(),
        Object::Array(names.collect()),
        "DeviceGray".into(),
        tint.into(),
    ]);
    let table = Object::string_literal(vec![255; inks as usize]);
    let indexed = Object::Array(vec!["Indexed".into(), many.clone(), 0.into(), table]);
    let times = 40;
    let fill = "/CS0 cs ".repeat(times);
    let stroke = " 0 SC".repeat(times);
    let content = format!("{fill} /CS1 CS{stroke}");
    let spaces = dictionary! { "CS0" => many, "CS1" => indexed };
    draw_colours(&mut p, spaces, content);
    // Each space read once: the inks' space, one more for each 1,024 inks, its tint
    // transform and its alternate; the indexed space, its table's KiB and the inks under it.
    let many_read = 1 + inks / 1024 + 1 + 1;
    let indexed_read = 1 + inks / 1024 + many_read;
    // The inks, the tint's one output and the grey it gives; and the index over them.
    let many_each = (inks + 1 + 1) / 1024;
    let indexed_each = (1 + inks + 1 + 1) / 1024;
    let set = times as u64 * (1 + many_each) + (1 + times as u64) * (1 + indexed_each);
    read_within(&p, zebra_steps() + 3 + 1 + many_read + indexed_read + set);
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
    let ran = apart::run(
        module_path!(),
        "too_complex_child",
        &env,
        Duration::from_secs(30),
    );
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

/// Page 1 of `p`, saved, drawn at 72 dpi.
fn drawn(p: &mut Pdf) -> render::Bitmap {
    let doc = render::load(p.snapshot().unwrap(), None).unwrap();
    render::Renderer::new(&doc).render(0, 1.0).unwrap()
}

/// A redaction over a form that draws itself twice follows the form once: drawn again
/// inside itself, it goes whole, twice, not once for each copy down to the nesting limit
/// (PB-104), and the page outside the mark draws as before.
#[test]
fn a_redaction_over_a_form_drawing_itself_follows_it_once() {
    let mut p = zebras(1);
    draw_itself(&mut p, 0, 2);
    let before = drawn(&mut p);
    // Over the form, clear of "Zebra": 300 to 400 points (and pixels) each way.
    let rep = p.redact(&[(0, [300.0, 300.0, 400.0, 400.0])]).unwrap();
    assert_eq!(rep.pages[0].images, 2, "{rep:?}");
    assert_eq!(p.page_text(0).trim(), "Zebra");
    let after = drawn(&mut p);
    let outside = |i: usize| {
        let (x, y) = (i % after.width, i / after.width);
        !(299..=401).contains(&x) || !(299..=401).contains(&y)
    };
    let px = |b: &render::Bitmap, i: usize| b.rgba[i * 4..i * 4 + 4].to_vec();
    let changed = (0..after.width * after.height)
        .filter(|&i| outside(i) && px(&before, i) != px(&after, i))
        .count();
    assert_eq!(changed, 0);
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

/// Finding the pictures, which follows every drawing every page draws, has the same budget:
/// naming, saving or reducing the pictures of a document with a page too complex to follow
/// is refused, with nothing written or changed. Failed on the old code: named, not refused.
#[test]
fn pictures_on_a_page_too_complex_to_follow_are_refused() {
    use limarepdf_core::{budget_seam, ReduceOptions, TooComplex};
    let mut p = zebras(2);
    draw_tree(&mut p, 1, 3, 4);
    let scratch = common::tool::Scratch::new("too-complex");
    let dir = scratch.join("pictures");
    let before = p.state_before();
    let ((), _) = budget_seam::with_limit(40, || {
        let e = p.picture_names("zebras").unwrap_err();
        let too = e.downcast_ref::<TooComplex>().expect("too complex");
        assert_eq!(too.what, "finding the pictures", "{e:#}");
        let e = p.save_all_pictures(&dir, "zebras").unwrap_err();
        assert!(e.downcast_ref::<TooComplex>().is_some(), "{e:#}");
        let opts = ReduceOptions::default();
        let e = p.reduce_pictures(&[0], &opts).unwrap_err();
        assert!(e.downcast_ref::<TooComplex>().is_some(), "{e:#}");
    });
    assert!(!dir.exists());
    assert_eq!(p.state_since(&before), []);
    // Within the steps it needs, the tree is followed: it draws no pictures.
    assert!(p.picture_names("zebras").unwrap().is_empty());
}

/// The child process of the next test: names the pictures of a page drawing a tree of
/// forms ten wide and eight deep, which unchecked is followed 10⁸ times.
#[test]
#[ignore = "started by pictures_of_a_page_drawing_forms_over_and_over_are_refused_at_once"]
fn too_complex_pictures_child() {
    if std::env::var("TOO_COMPLEX_PICTURES_CHILD").is_err() {
        return;
    }
    let mut p = zebras(1);
    draw_tree(&mut p, 0, 10, 8);
    match p.picture_names("zebras") {
        Ok(names) => println!("pictures|named|{}", names.len()),
        Err(e) => println!("pictures|refused|{e:#}"),
    }
}

/// Failed on the old code: not named within 30 s. The refusal comes after the whole budget,
/// 2²³ steps (4 s in a debug build, over 10 s on a slow runner), so no closer bound on the
/// time: the refusal's message says the budget stopped it.
#[test]
fn pictures_of_a_page_drawing_forms_over_and_over_are_refused_at_once() {
    let env = [("TOO_COMPLEX_PICTURES_CHILD", "1".to_string())];
    let ran = apart::run(
        module_path!(),
        "too_complex_pictures_child",
        &env,
        Duration::from_secs(30),
    );
    let Ran::Ended(text, status) = ran else {
        panic!("not named within 30 s");
    };
    let said = apart::reports(&text, "pictures|");
    assert_eq!(said.len(), 1, "{}", apart::why_stopped(status));
    assert!(said[0].starts_with("refused|"), "{}", said[0]);
    assert!(said[0].contains("finding the pictures"), "{}", said[0]);
}

/// Adds `d` to the end of page 1's annotations.
fn annotate(p: &mut Pdf, d: Dictionary) -> ObjectId {
    let id = p.doc.add_object(d);
    let page = p.page_ids()[0];
    let pd = p.doc.get_dictionary_mut(page).unwrap();
    let mut list = (pd.get(b"Annots")).map_or(Vec::new(), |a| a.as_array().unwrap().clone());
    list.push(id.into());
    pd.set("Annots", list);
    id
}

/// `n` freehand drawings on page 1 of `p`, without drawings of their own, sharing one
/// 10,000-point stroke zigzagging from (100, 200) to (299, 250). Their `/Rect` has no area
/// (at 100, 200), so each is placed where it is drawn (PB-114).
fn shared_strokes(p: &mut Pdf, n: usize) -> Vec<ObjectId> {
    let stroke: Vec<Object> = (0..10_000i64)
        .flat_map(|i| [100 + i % 200, 200 + i % 2 * 50].map(Object::from))
        .collect();
    let stroke = p.doc.add_object(stroke);
    let rect = [100i64, 200, 100, 200].map(Object::from).to_vec();
    (0..n)
        .map(|_| {
            let d = dictionary! {
                "Type" => "Annot", "Subtype" => "Ink", "Rect" => rect.clone(),
                "InkList" => vec![Object::Reference(stroke)], "C" => vec![Object::from(0i64)],
            };
            annotate(p, d)
        })
        .collect()
}

/// Drawing annotations has a budget (WP-04 step 5): 20 drawings sharing one 10,000-point
/// stroke draw it 20 times, and flattening them past the budget is refused, with nothing
/// changed; within it, they are drawn. Each was drawn, however many shared the stroke.
#[test]
fn annotations_too_complex_to_draw_aren_t_flattened() {
    use limarepdf_core::{budget_seam, TooComplex};
    let mut p = zebras(1);
    shared_strokes(&mut p, 20);
    common::seeds::pdf(&p);
    let before = p.state_before();
    let (e, _) = budget_seam::with_limit(100_000, || p.flatten_annotations(None));
    let e = e.expect_err("refused");
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(too.what, "drawing the annotations", "{e:#}");
    assert_eq!(p.state_since(&before), []);
    let (done, _) = budget_seam::with_limit(4_000_000, || p.flatten_annotations(None));
    assert_eq!(done.unwrap().drawn, 20);
}

/// Listing annotations draws those placed where they are drawn (PB-114) within one budget,
/// and lists those past it at their own `/Rect` rather than leave them out: the first are
/// placed where they are drawn, the rest at (100, 200), 592 down the page.
#[test]
fn annotations_past_the_drawing_budget_are_listed_at_their_own_box() {
    use limarepdf_core::{budget_seam, Annot};
    let mut p = zebras(1);
    shared_strokes(&mut p, 20);
    let own = |a: &Annot| a.bbox == [100.0, 592.0, 100.0, 592.0];
    for limit in [u64::MAX, 500_000] {
        let (all, _) = budget_seam::with_limit(limit, || p.annotations());
        let (page, _) = budget_seam::with_limit(limit, || p.page_annotations(0).unwrap());
        for listed in [all, page] {
            assert_eq!(listed.len(), 20);
            let drawn = listed.iter().take_while(|a| !own(a)).count();
            assert!(listed[drawn..].iter().all(own), "{limit}");
            if limit == u64::MAX {
                assert_eq!(drawn, 20);
            } else {
                assert!((1..20).contains(&drawn), "{drawn} drawn within {limit}");
            }
        }
    }
}

/// The text [`sharing_a_text`]'s text boxes share: 20,004 characters.
fn long_text() -> String {
    "zebra ".repeat(3_334)
}

/// A page of [`zebras`] with 20 text boxes, below "Zebra", sharing one text
/// ([`long_text`]).
fn sharing_a_text() -> Pdf {
    let mut p = zebras(1);
    let text = p.doc.add_object(Object::string_literal(long_text()));
    boxes_sharing(&mut p, text);
    p
}

/// 20 text boxes on page 1 of `p`, below "Zebra", sharing `text`.
fn boxes_sharing(p: &mut Pdf, text: ObjectId) {
    let rect = [72i64, 400, 300, 500].map(Object::from).to_vec();
    for _ in 0..20 {
        let d = dictionary! {
            "Type" => "Annot", "Subtype" => "FreeText", "Rect" => rect.clone(),
            "Contents" => text, "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
        };
        annotate(p, d);
    }
}

/// A text box's text is charged to drawing it: 20 text boxes sharing one 20,004-character
/// text set it 20 times, and flattening them past the budget is refused, with nothing
/// changed. Each was set, however many shared the text.
#[test]
fn text_boxes_sharing_a_long_text_are_charged_it_each() {
    use limarepdf_core::{budget_seam, TooComplex};
    let mut p = sharing_a_text();
    let before = p.state_before();
    let (e, _) = budget_seam::with_limit(200_000, || p.flatten_annotations(None));
    let e = e.expect_err("refused");
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(too.what, "drawing the annotations", "{e:#}");
    assert_eq!(p.state_since(&before), []);
    let (done, _) = budget_seam::with_limit(2_000_000, || p.flatten_annotations(None));
    assert_eq!(done.unwrap().drawn, 20);
}

/// How many annotations of the one-page `bytes` have a drawing.
fn with_drawings(bytes: &[u8]) -> usize {
    let p = Pdf::from_bytes(bytes, None).unwrap();
    let page = p.doc.get_dictionary(p.page_ids()[0]).unwrap();
    let list = page.get(b"Annots").and_then(Object::as_array).unwrap();
    let drawn = |id| p.doc.get_dictionary(id).unwrap().has(b"AP");
    (list.iter())
        .filter(|a| drawn(a.as_reference().unwrap()))
        .count()
}

/// Showing a document draws the annotations without drawings of their own within the
/// budget and leaves those past it without one, as other readers show them, rather than
/// not show it: the app's pages, and the page under text being edited, draw the first
/// and not the rest.
#[test]
fn a_document_with_annotations_past_the_drawing_budget_is_still_shown() {
    use limarepdf_core::budget_seam;
    let mut p = zebras(1);
    shared_strokes(&mut p, 20);
    for limit in [u64::MAX, 500_000] {
        let (app, _) = budget_seam::with_limit(limit, || p.app_snapshot());
        let (page, _) = budget_seam::with_limit(limit, || p.page_without(0, &[]));
        for shown in [app, page] {
            let drawn = with_drawings(&shown.unwrap());
            if limit == u64::MAX {
                assert_eq!(drawn, 20);
            } else {
                assert!((1..20).contains(&drawn), "{drawn} drawn within {limit}");
            }
        }
    }
}

/// Redacting draws the annotations placed where they are drawn (PB-114) to find those the
/// marks cover, within the budget; past it the redaction is refused, with nothing changed,
/// rather than miss one.
#[test]
fn a_redaction_over_annotations_too_complex_to_draw_changes_nothing() {
    use limarepdf_core::{budget_seam, TooComplex};
    let mut p = zebras(1);
    shared_strokes(&mut p, 20);
    let before = p.state_before();
    let mark = [(0, [150.0, 500.0, 160.0, 600.0])];
    let (e, _) = budget_seam::with_limit(100_000, || p.redact(&mark));
    let e = e.expect_err("refused");
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(too.what, "drawing the annotations", "{e:#}");
    assert_eq!(p.state_since(&before), []);
    let (done, _) = budget_seam::with_limit(4_000_000, || p.redact(&mark));
    assert_eq!(done.unwrap().pages[0].annots, 20);
}

/// An annotation placed where it is drawn (PB-114) is moved from there; past the budget
/// moving it is refused, with nothing changed.
#[test]
fn moving_an_annotation_too_complex_to_draw_is_refused() {
    use limarepdf_core::{budget_seam, TooComplex};
    let mut p = zebras(1);
    let id = shared_strokes(&mut p, 1)[0];
    let before = p.state_before();
    let to = [10.0, 10.0, 50.0, 50.0];
    let (e, _) = budget_seam::with_limit(1_000, || p.move_annotation(0, id, to));
    let e = e.expect_err("refused");
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(too.what, "drawing the annotations", "{e:#}");
    assert_eq!(p.state_since(&before), []);
}

/// `n` freehand drawings on page 1 of `p`, each with a `/Rect` of its own, listing one
/// 10,000-point stroke `strokes` times.
fn drawings(p: &mut Pdf, n: usize, strokes: usize) {
    let stroke: Vec<Object> = (0..10_000i64)
        .flat_map(|i| [100 + i % 200, 200 + i % 2 * 50].map(Object::from))
        .collect();
    let stroke = p.doc.add_object(stroke);
    let rect = [100i64, 200, 300, 250].map(Object::from).to_vec();
    for _ in 0..n {
        let d = dictionary! {
            "Type" => "Annot", "Subtype" => "Ink", "Rect" => rect.clone(),
            "InkList" => vec![Object::Reference(stroke); strokes],
            "C" => vec![Object::from(0i64)],
        };
        annotate(p, d);
    }
}

/// The points listed for `a`.
fn points(a: &limarepdf_core::Annot) -> usize {
    a.paths.iter().map(Vec::len).sum()
}

/// A drawing is listed with the 100,000 points it is drawn with, however many its strokes
/// have: one listing a 10,000-point stroke 1,000 times lists the first ten. Before, it
/// listed all 10⁷ points.
#[test]
fn a_drawing_is_listed_with_the_points_it_is_drawn_with() {
    let mut p = zebras(1);
    drawings(&mut p, 1, 1_000);
    let listed = p.annotations();
    assert_eq!(listed[0].paths.len(), 10);
    assert_eq!(points(&listed[0]), 100_000);
}

/// Listing annotations reads their points within one budget a listing ("reading the
/// annotations"), and lists those past it unread rather than leave them out: of 20
/// drawings listing a 10,000-point stroke ten times, the first are listed with their
/// 100,000 points and the rest with none. Before, each was read in full.
#[test]
fn annotations_past_the_reading_budget_are_listed_unread() {
    use limarepdf_core::budget_seam;
    let mut p = zebras(1);
    drawings(&mut p, 20, 10);
    let (all, _) = budget_seam::with_limit(1_000_000, || p.annotations());
    let (page, _) = budget_seam::with_limit(1_000_000, || p.page_annotations(0).unwrap());
    for listed in [all, page] {
        assert_eq!(listed.len(), 20);
        let read = listed.iter().take_while(|a| points(a) == 100_000).count();
        assert!((1..20).contains(&read), "{read} read");
        assert!(listed[..read].iter().all(|a| !a.unread));
        assert!(listed[read..]
            .iter()
            .all(|a| a.paths.is_empty() && a.unread));
    }
}

/// Their text is read within the same budget: of 20 text boxes sharing a 20,004-character
/// text, the first are listed with it and the rest without. Before, each was read in full.
#[test]
fn text_past_the_reading_budget_is_listed_unread() {
    use limarepdf_core::budget_seam;
    let p = sharing_a_text();
    let (all, _) = budget_seam::with_limit(200_000, || p.annotations());
    let (page, _) = budget_seam::with_limit(200_000, || p.page_annotations(0).unwrap());
    for listed in [all, page] {
        assert_eq!(listed.len(), 20);
        let read = listed
            .iter()
            .take_while(|a| a.contents == long_text())
            .count();
        assert!((1..20).contains(&read), "{read} read");
        assert!(listed[..read]
            .iter()
            .all(|a| !a.unread && a.text_style.is_some()));
        let unread = |a: &limarepdf_core::Annot| a.unread && a.text_style.is_none();
        assert!(listed[read..]
            .iter()
            .all(|a| a.contents.is_empty() && unread(a)));
    }
}

/// A text box's style is read once when it is listed, and listed as it is drawn: its
/// 1,000-byte `/DA`, which has no colour, and its `/DS`, which is red, are each charged
/// once, and its text is listed red. Before, the `/DA` was charged twice and the text
/// listed black.
#[test]
fn a_text_box_s_style_is_read_once_and_listed_as_drawn() {
    use limarepdf_core::budget_seam;
    let mut p = zebras(1);
    let da = format!("/Helv 12 Tf{}", " ".repeat(989));
    let ds = "font: 12pt Helvetica; color:#FF0000";
    let rect = [72i64, 400, 300, 500].map(Object::from).to_vec();
    let d = dictionary! {
        "Type" => "Annot", "Subtype" => "FreeText", "Rect" => rect,
        "DA" => Object::string_literal(da), "DS" => Object::string_literal(ds),
        "Contents" => Object::string_literal("Zebra"),
    };
    annotate(&mut p, d);
    let (listed, taken) = budget_seam::with_limit(u64::MAX, || p.annotations());
    let colour = listed[0].text_style.map(|s| s.color);
    // Drawing none; reading one for its entry in `/Annots`, 64 for the annotation, then its
    // `/DA`, `/DS` and text.
    let read = vec![0, 1 + 64 + 1_000 + ds.len() as u64 + 5];
    assert_eq!((colour, taken), (Some([1.0, 0.0, 0.0]), read));
}

/// A text added to `p`, a stream that deflates `inflated` and reads it as hexadecimal once
/// inflated (`[/FlateDecode /ASCIIHexDecode]`).
fn hex_text(p: &mut Pdf, inflated: Vec<u8>) -> ObjectId {
    let mut s = Stream::new(Dictionary::new(), inflated);
    s.compress().unwrap();
    let filters = vec![Object::from("FlateDecode"), Object::from("ASCIIHexDecode")];
    s.dict.set("Filter", filters);
    p.doc.add_object(s)
}

/// A page of [`zebras`] with 20 comments sharing one text, a [`hex_text`] of `inflated`.
fn comments_sharing(inflated: Vec<u8>) -> Pdf {
    let mut p = zebras(1);
    let text = hex_text(&mut p, inflated);
    let rect = [72i64, 600, 92, 620].map(Object::from).to_vec();
    for _ in 0..20 {
        let d = dictionary! {
            "Type" => "Annot", "Subtype" => "Text", "Rect" => rect.clone(), "Contents" => text,
        };
        annotate(&mut p, d);
    }
    p
}

/// The annotations of [`comments_sharing`]'s page listed within `limit`, and how many of
/// them, the first, were read; the rest are listed unread, by both listings alike.
fn listed_within(p: &Pdf, limit: u64) -> (Vec<limarepdf_core::Annot>, usize) {
    use limarepdf_core::budget_seam;
    let (all, _) = budget_seam::with_limit(limit, || p.annotations());
    let (page, _) = budget_seam::with_limit(limit, || p.page_annotations(0).unwrap());
    let read = |listed: &[limarepdf_core::Annot]| {
        assert_eq!(listed.len(), 20);
        let read = listed.iter().take_while(|a| !a.unread).count();
        assert!(listed[read..].iter().all(|a| a.unread));
        read
    };
    let n = read(&all);
    assert_eq!(read(&page), n);
    (all, n)
}

/// A text that fails to decode is charged what decoding it took: of 20 comments sharing one
/// that inflates to 100,000 bytes and then isn't hexadecimal, the first are listed read,
/// without it, and the rest unread. Before, the failure was free, and all 20 were read.
#[test]
fn a_text_that_fails_to_decode_is_charged_what_decoding_took() {
    let p = comments_sharing(vec![b'z'; 100_000]);
    let (listed, read) = listed_within(&p, 5_000_000);
    assert!((1..20).contains(&read), "{read} read");
    assert!(listed[..read].iter().all(|a| a.contents.is_empty()));
}

/// A text is charged each filter it is decoded through: of 20 comments sharing one that
/// inflates to 1,000,000 spaces and a hexadecimal "A", the first are listed with the "A" and
/// the rest unread. Before, only the "A" was charged, and all 20 were read.
#[test]
fn a_text_is_charged_each_filter_it_is_decoded_through() {
    let mut inflated = vec![b' '; 1_000_000];
    inflated.extend(b"41");
    let p = comments_sharing(inflated);
    let (listed, read) = listed_within(&p, 10_000_000);
    assert!((1..20).contains(&read), "{read} read");
    assert!(listed[..read].iter().all(|a| a.contents == "A"));
}

/// A text box's text is charged what decoding it takes each time it is read, checked and
/// drawn alike: flattening 20 text boxes sharing one that inflates to 100,000 bytes and
/// then isn't hexadecimal is refused past the budget, with nothing changed, and done within
/// it. Before, a text box's text was read uncharged, and its failure was free.
#[test]
fn text_boxes_whose_text_fails_to_decode_are_charged_it() {
    use limarepdf_core::{budget_seam, TooComplex};
    let mut p = zebras(1);
    let text = hex_text(&mut p, vec![b'z'; 100_000]);
    boxes_sharing(&mut p, text);
    let before = p.state_before();
    let (e, _) = budget_seam::with_limit(1_000_000, || p.flatten_annotations(None));
    let e = e.expect_err("refused");
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(too.what, "drawing the annotations", "{e:#}");
    assert_eq!(p.state_since(&before), []);
    let (done, _) = budget_seam::with_limit(40_000_000, || p.flatten_annotations(None));
    assert_eq!(done.unwrap().drawn, 20);
}

/// Fitting a text box to its text charges setting it each time it is measured: a
/// 20,004-character one ([`long_text`]), measured in its box and again grown to the foot of
/// the page, takes two steps a byte each time, after one to read it. Before, only reading
/// it was charged.
#[test]
fn fitting_a_text_box_charges_its_text() {
    use limarepdf_core::{budget_seam, TextStyle};
    let mut p = zebras(1);
    let rect = [72i64, 400, 300, 500].map(Object::from).to_vec();
    let d = dictionary! {
        "Type" => "Annot", "Subtype" => "FreeText", "Rect" => rect,
        "Contents" => Object::string_literal(long_text()),
        "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
    };
    let id = annotate(&mut p, d);
    let style = TextStyle::default();
    let (done, taken) = budget_seam::with_limit(u64::MAX, || p.set_text_style(id, &style));
    done.unwrap();
    // Checking, fitting and drawing it, each with a budget of its own.
    assert_eq!(taken.len(), 3, "{taken:?}");
    assert!(taken[1] >= 5 * 20_004, "{taken:?}");
}

/// Removing annotations, which offers each to be chosen by what it says, is refused past
/// the budget, with nothing changed, rather than keep one unread: an annotation written
/// into the page's list directly isn't given an object of its own. Within it, they go.
#[test]
fn removing_annotations_past_the_reading_budget_is_refused() {
    use limarepdf_core::{budget_seam, TooComplex};
    let mut p = sharing_a_text();
    let page = p.page_ids()[0];
    let pd = p.doc.get_dictionary_mut(page).unwrap();
    let mut list = pd.get(b"Annots").unwrap().as_array().unwrap().clone();
    let rect = [72i64, 600, 92, 620].map(Object::from).to_vec();
    let note = dictionary! {
        "Type" => "Annot", "Subtype" => "Text", "Rect" => rect,
        "T" => Object::string_literal("Someone"),
    };
    list.insert(0, Object::Dictionary(note));
    pd.set("Annots", list);
    let before = p.state_before();
    let (e, _) = budget_seam::with_limit(200_000, || {
        p.remove_annotations(None, |a| a.author == "Someone")
    });
    let e = e.expect_err("refused");
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(too.what, "reading the annotations", "{e:#}");
    assert_eq!(p.state_since(&before), []);
    let (done, _) = budget_seam::with_limit(2_000_000, || {
        p.remove_annotations(None, |a| a.contents == long_text())
    });
    assert_eq!(done.unwrap(), 20);
}

/// Deleting one annotation, as the app does, reads no other: one listed unread past the
/// budget is deleted, with its reply, where removing it by what it is would be refused.
#[test]
fn an_annotation_past_the_reading_budget_is_deleted_by_itself() {
    use limarepdf_core::budget_seam;
    let mut p = sharing_a_text();
    let (listed, _) = budget_seam::with_limit(200_000, || p.annotations());
    let last = listed.last().unwrap();
    assert!(last.unread);
    let rect = [72i64, 600, 92, 620].map(Object::from).to_vec();
    let reply = dictionary! {
        "Type" => "Annot", "Subtype" => "Text", "Rect" => rect,
        "IRT" => Object::Reference(last.id),
    };
    annotate(&mut p, reply);
    let (e, _) =
        budget_seam::with_limit(200_000, || p.remove_annotations(None, |a| a.id == last.id));
    assert!(e.is_err());
    let (done, _) = budget_seam::with_limit(200_000, || p.remove_annotation(last.id));
    assert_eq!(done.unwrap(), 2);
    let left = p.annotations();
    assert_eq!(left.len(), 19);
    assert!(left.iter().all(|a| a.id != last.id));
}

/// Redacting looks for the text it removed in every comment, within the budget; past it
/// the redaction is refused, with nothing changed, rather than miss one. Within it, the
/// comments repeating "Zebra" are named.
#[test]
fn a_redaction_with_comments_past_the_reading_budget_changes_nothing() {
    use limarepdf_core::{budget_seam, TooComplex};
    let mut p = sharing_a_text();
    let before = p.state_before();
    // Over "Zebra", above the text boxes.
    let mark = [(0, [0.0, 0.0, 612.0, 200.0])];
    let (e, _) = budget_seam::with_limit(200_000, || p.redact(&mark));
    let e = e.expect_err("refused");
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(too.what, "reading the annotations", "{e:#}");
    assert_eq!(p.state_since(&before), []);
    let (done, _) = budget_seam::with_limit(2_000_000, || p.redact(&mark));
    let elsewhere = done.unwrap().elsewhere;
    let comments = vec!["a comment on page 1".to_string()];
    assert_eq!(elsewhere, [("zebra".to_string(), comments)]);
}

/// Applying the redaction marks reads them, and looks for what they removed in every
/// comment, within the budget; past it, it is refused, with nothing changed.
#[test]
fn applying_redaction_marks_past_the_reading_budget_changes_nothing() {
    use limarepdf_core::{budget_seam, TooComplex};
    let mut p = sharing_a_text();
    // Over "Zebra", above the text boxes, after them.
    let rect = [0i64, 592, 612, 792].map(Object::from).to_vec();
    annotate(
        &mut p,
        dictionary! { "Type" => "Annot", "Subtype" => "Redact", "Rect" => rect },
    );
    let before = p.state_before();
    let (e, _) = budget_seam::with_limit(200_000, || p.apply_redactions());
    let e = e.expect_err("refused");
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(too.what, "reading the annotations", "{e:#}");
    assert_eq!(p.state_since(&before), []);
    let (done, _) = budget_seam::with_limit(2_000_000, || p.apply_redactions());
    assert_eq!(done.unwrap().elsewhere.len(), 1);
}

/// Removing the comments needs only what each annotation is: it reads nothing more, and
/// isn't refused however much they say.
#[test]
fn removing_the_comments_reads_only_what_each_is() {
    use limarepdf_core::{budget_seam, Hidden};
    let mut p = sharing_a_text();
    let (rep, _) = budget_seam::with_limit(200_000, || p.remove_hidden(&[Hidden::Comments]));
    assert_eq!(rep.unwrap().count(Hidden::Comments), 20);
}

/// Changing an annotation reads it alone: the last of 20 text boxes, listed unread past the
/// budget, is changed within it. Before, it was found by listing them all, and refused.
/// Past what reading it alone takes, it is refused, with nothing changed, rather than write
/// over what wasn't read.
#[test]
fn changing_an_annotation_reads_it_alone() {
    use limarepdf_core::{budget_seam, TooComplex};
    let mut p = sharing_a_text();
    let (listed, _) = budget_seam::with_limit(200_000, || p.annotations());
    let last = listed.last().unwrap().id;
    assert!(listed.last().unwrap().unread);
    let red = Some([1.0, 0.0, 0.0]);
    let before = p.state_before();
    let (e, _) = budget_seam::with_limit(10_000, || p.set_annotation(last, red, 1.0, &long_text()));
    let e = e.expect_err("refused");
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(too.what, "reading the annotations", "{e:#}");
    assert_eq!(p.state_since(&before), []);
    let (done, _) =
        budget_seam::with_limit(200_000, || p.set_annotation(last, red, 1.0, &long_text()));
    done.unwrap();
    assert_eq!(p.annotations().last().unwrap().color, red);
}

/// A freehand drawing on page 1 of `p`, with a `/Rect` of its own, listing one stroke of
/// `points` points zigzagging from (100, 200) `times` times.
fn drawing(p: &mut Pdf, points: usize, times: usize) -> ObjectId {
    let stroke: Vec<Object> = (0..points as i64)
        .flat_map(|i| [100 + i % 200, 200 + i % 2 * 50].map(Object::from))
        .collect();
    let stroke = p.doc.add_object(stroke);
    let rect = [100i64, 200, 300, 250].map(Object::from).to_vec();
    let d = dictionary! {
        "Type" => "Annot", "Subtype" => "Ink", "Rect" => rect,
        "InkList" => vec![Object::Reference(stroke); times],
        "C" => vec![Object::from(0i64)],
    };
    annotate(p, d)
}

/// Whether `e` is the refusal of a change past the 100,000 points an annotation is drawn
/// with.
fn past_its_points(e: &anyhow::Error) -> bool {
    use limarepdf_core::TooComplex;
    e.downcast_ref::<TooComplex>()
        .is_some_and(|t| (t.what, t.limit) == ("changing the annotation", 100_000))
}

/// Changing a drawing reads its strokes within the 100,000 points it is drawn with, and is
/// refused past them, with nothing changed, rather than write them back cut: one listing a
/// 60,000-point stroke twice, and one with a 150,000-point stroke, aren't added to,
/// restyled, turned or moved. Before, the first was written back whole (and 10,000 strokes
/// would have been too), the second cut to 100,000 points. One of 100,000 points is.
#[test]
fn changing_a_drawing_past_its_points_is_refused() {
    use limarepdf_core::ShapeLook;
    let look = ShapeLook {
        fill: None,
        width: 3.0,
        arrow: false,
    };
    let change = |p: &mut Pdf, id, n| match n {
        0 => p.add_ink_stroke(0, id, &[(150.0, 150.0), (200.0, 160.0)]),
        1 => p.set_shape_look(id, &look),
        2 => p.rotate_ink(0, id, 30.0),
        _ => p.move_annotation(0, id, [50.0, 50.0, 400.0, 300.0]),
    };
    for (points, times) in [(60_000, 2), (150_000, 1)] {
        let mut p = zebras(1);
        let id = drawing(&mut p, points, times);
        let before = p.state_before();
        for n in 0..4 {
            let e = change(&mut p, id, n).expect_err("refused");
            assert!(past_its_points(&e), "{points} × {times}, {n}: {e:#}");
            assert_eq!(p.state_since(&before), [], "{points} × {times}, {n}");
        }
    }
    let mut p = zebras(1);
    let id = drawing(&mut p, 50_000, 2);
    for n in 1..4 {
        change(&mut p, id, n).unwrap();
    }
}

/// Adding a stroke to a drawing is refused when the drawing would then have more than the
/// 100,000 points it is drawn with, with nothing changed. Before, it was added.
#[test]
fn adding_a_stroke_past_a_drawing_s_points_is_refused() {
    let stroke = [(150.0, 150.0), (200.0, 160.0)];
    let mut p = zebras(1);
    let id = drawing(&mut p, 99_999, 1);
    let before = p.state_before();
    let e = p.add_ink_stroke(0, id, &stroke).expect_err("refused");
    assert!(past_its_points(&e), "{e:#}");
    assert_eq!(p.state_since(&before), []);
    let mut p = zebras(1);
    let id = drawing(&mut p, 99_998, 1);
    p.add_ink_stroke(0, id, &stroke).unwrap();
}

/// Moving an annotation reads each of its entries of points within the 100,000 it is drawn
/// with, and is refused past them, with nothing changed, rather than write them back cut: a
/// polygon of 150,000 points isn't moved. Before, it was moved and cut to 100,000. One of
/// 100,000 is.
#[test]
fn moving_an_annotation_past_its_points_is_refused() {
    let polygon = |p: &mut Pdf, points: i64| {
        let vertices: Vec<Object> = (0..points)
            .flat_map(|i| [100 + i % 200, 200 + i % 2 * 50].map(Object::from))
            .collect();
        let rect = [100i64, 200, 300, 250].map(Object::from).to_vec();
        let d = dictionary! {
            "Type" => "Annot", "Subtype" => "Polygon", "Rect" => rect,
            "Vertices" => vertices, "C" => vec![Object::from(0i64)],
        };
        annotate(p, d)
    };
    let to = [50.0, 50.0, 400.0, 300.0];
    let mut p = zebras(1);
    let id = polygon(&mut p, 150_000);
    let before = p.state_before();
    let e = p.move_annotation(0, id, to).expect_err("refused");
    assert!(past_its_points(&e), "{e:#}");
    assert_eq!(p.state_since(&before), []);
    let mut p = zebras(1);
    let id = polygon(&mut p, 100_000);
    p.move_annotation(0, id, to).unwrap();
}

/// A page with a form of one field titled with 64 KiB, which `kids` text fields "k0", "k1"
/// and on share: each kid's name holds it. Returns the title.
fn titled_form(p: &mut Pdf, kids: usize) -> String {
    let title = "a".repeat(65_536);
    let parent = p.doc.new_object_id();
    let kids: Vec<Object> = (0..kids)
        .map(|i| {
            let t = Object::string_literal(format!("k{i}"));
            let kid = dictionary! { "FT" => "Tx", "T" => t, "V" => "v", "Parent" => parent };
            p.doc.add_object(kid).into()
        })
        .collect();
    let field = dictionary! { "T" => Object::string_literal(title.as_str()), "Kids" => kids };
    p.doc.objects.insert(parent, field.into());
    let form = dictionary! { "Fields" => vec![parent.into()] };
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
    title
}

/// Reading a form's fields has a budget of 2²⁶ steps, the bytes of their names among them:
/// past it, flattening, resetting, removing the values (and listing what removing hidden
/// information would remove), setting a field it didn't reach and redacting, which looks
/// for the removed text in every field, are refused with a typed reason, with nothing
/// changed. Before, a walk held what the fields did: each of 1,100 kids
/// a copy of the 64 KiB title, 72 MB. 1,000 are read whole.
#[test]
fn a_form_holding_more_than_is_read_of_one_is_refused() {
    use limarepdf_core::{FieldValue, Hidden, TooComplex};
    let x = || FieldValue::Text("x".into());
    let page = [0.0, 0.0, 612.0, 792.0];
    let mut p = zebras(1);
    let title = titled_form(&mut p, 1_100);
    let before = p.state_before();
    let refused = [
        p.flatten_form(None).err(),
        p.reset_form().err(),
        p.set_field(&format!("{title}.k1099"), x()).err(),
        p.check_remove_hidden(&[Hidden::FormValues]).err(),
        p.remove_hidden(&[Hidden::FormValues]).err(),
        p.find_hidden().err(),
        p.redact(&[(0, page)]).err(),
    ];
    let reading = ("reading the form's fields", 1 << 26);
    for (n, e) in refused.into_iter().enumerate() {
        let e = e.unwrap_or_else(|| panic!("{n} not refused"));
        let too = e.downcast_ref::<TooComplex>().expect("too complex");
        assert_eq!((too.what, too.limit), reading, "{n}");
    }
    assert_eq!(p.state_since(&before), []);
    let mut p = zebras(1);
    let title = titled_form(&mut p, 1_000);
    p.set_field(&format!("{title}.k999"), x()).unwrap();
    p.reset_form().unwrap();
    p.check_remove_hidden(&[Hidden::FormValues]).unwrap();
    p.find_hidden().unwrap();
    p.remove_hidden(&[Hidden::FormValues]).unwrap();
    p.redact(&[(0, page)]).unwrap();
    p.flatten_form(None).unwrap();
}

/// Lists `field` after the fields of the form `titled_form` made.
fn list_after(p: &mut Pdf, field: Dictionary) {
    let id = p.doc.add_object(field);
    let form = p.doc.catalog_mut().unwrap().get_mut(b"AcroForm").unwrap();
    let fields = form.as_dict_mut().unwrap().get_mut(b"Fields").unwrap();
    fields.as_array_mut().unwrap().push(id.into());
}

/// A signature field listed after a form holding more than is read of one is still found, so
/// a file signed there is taken for signed (the app keeps its bytes, and `limarepdf-cli`
/// doesn't save it whole without `--break-signatures`): `signature_count` and
/// `has_signature_fields` look past the walk's steps for it. They missed it, and the file was
/// taken for unsigned. An empty one is found, and not counted as signed.
#[test]
fn a_signature_past_what_is_read_of_a_form_is_found() {
    let signature = |p: &mut Pdf, signed: bool| {
        let mut field = dictionary! { "FT" => "Sig", "T" => Object::string_literal("s") };
        if signed {
            let sig = dictionary! { "Type" => "Sig", "Filter" => "Adobe.PPKLite" };
            field.set("V", p.doc.add_object(sig));
        }
        list_after(p, field);
    };
    for kids in [1_000, 1_100] {
        let mut p = zebras(1);
        titled_form(&mut p, kids);
        assert!(!p.has_signature_fields(), "{kids}");
        signature(&mut p, true);
        assert_eq!(p.signature_count(), 1, "{kids}");
        assert!(p.has_signature_fields(), "{kids}");
        let mut p = zebras(1);
        titled_form(&mut p, kids);
        signature(&mut p, false);
        assert_eq!(p.signature_count(), 0, "{kids}");
        assert!(p.has_signature_fields(), "{kids}");
    }
}

/// A form with an XFA part (fields in XML, for Adobe Reader) holding more than is read of
/// one isn't taken for XFA-only when none of the fields read has a widget: the fields after
/// may have them. It was, and removing scripts kept the XFA part's, as it does an XFA-only
/// form's, which would be left with nothing. Read whole, with no widget, it is XFA-only.
#[test]
fn a_form_holding_more_than_is_read_of_one_is_not_taken_for_xfa_only() {
    use limarepdf_core::Hidden;
    for (kids, xfa_only) in [(1_000, true), (1_100, false)] {
        let mut p = zebras(1);
        titled_form(&mut p, kids);
        let form = p.doc.catalog_mut().unwrap().get_mut(b"AcroForm").unwrap();
        let xfa = Object::string_literal("<xdp:xdp><script/></xdp:xdp>");
        form.as_dict_mut().unwrap().set("XFA", xfa);
        assert_eq!(p.xfa_only(), xfa_only, "{kids}");
        p.remove_hidden(&[Hidden::Scripts]).unwrap();
        let form = p.doc.catalog().unwrap().get(b"AcroForm").unwrap();
        assert_eq!(form.as_dict().unwrap().has(b"XFA"), xfa_only, "{kids}");
    }
}
