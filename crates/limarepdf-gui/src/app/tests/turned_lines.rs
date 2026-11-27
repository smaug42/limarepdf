//! #227's second part: a line reading down, back along or up the page, visible or
//! recognised, is edited along its line as it is drawn there, a click or drag in it going
//! by its letters, the input method's window at its caret, a tag saying what it runs over
//! beside it; a tilted line is edited in a plain box. With app.rs's helpers.

use super::ocr_correct::{
    double_click, drawn, editing, flat, in_helvetica, line, on_screen, opened, output, steps,
};
use super::*;
use lopdf::{dictionary, Dictionary, Object, Stream};
use std::f32::consts::{FRAC_PI_2, TAU};

/// A 612 x 792 page of Helvetica lines, 14 points: "Reads down the side" from (100, 92)
/// down the page, "Upside down line" from (500, 492) back along it, "Reads up the side"
/// from (460, 392) up it, and "Tilted line" from (200, 142) at 30°; and recognised
/// (invisible) lines reading down it from y 92, each after the first to the left of the one
/// before as lines below are: "Fisrt line down" (stretched by half) at x 300, "Second line
/// down" at 280 and "Third line down" at 260.
fn turned() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let content = "BT 0 Tr 100 Tz /F1 14 Tf 0 -1 1 0 100 700 Tm (Reads down the side) Tj ET\n\
                   BT /F1 14 Tf -1 0 0 -1 500 300 Tm (Upside down line) Tj ET\n\
                   BT /F1 14 Tf 0 1 -1 0 460 400 Tm (Reads up the side) Tj ET\n\
                   BT /F1 14 Tf 0.866 0.5 -0.5 0.866 200 650 Tm (Tilted line) Tj ET\n\
                   BT 3 Tr 150 Tz /F1 14 Tf 0 -1 1 0 300 700 Tm (Fisrt line down) Tj ET\n\
                   BT 3 Tr 100 Tz /F1 14 Tf 0 -1 1 0 280 700 Tm (Second line down) Tj ET\n\
                   BT 3 Tr /F1 14 Tf 0 -1 1 0 260 700 Tm (Third line down) Tj ET\n";
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", Object::Reference(s));
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    p
}

/// The page point `along` points along line `o` from its baseline's start and `lift` of
/// its size towards its letters' tops.
fn on(o: &limarepdf_core::PageObject, along: f32, lift: f32) -> (f32, f32) {
    let (x, y) = o.baseline.expect("a baseline");
    let (dx, dy) = o.dir.expect("a direction");
    let up = lift * o.font_size.unwrap_or(12.0);
    (x + dx * along + dy * up, y + dy * along - dx * up)
}

/// The page point between characters `i - 1` and `i` of line `o`, inside the line.
fn between(o: &limarepdf_core::PageObject, i: usize) -> (f32, f32) {
    on(o, o.stops[i], 0.3)
}

/// The page point in the middle of word `n` of line `o`, inside the line.
fn on_word(o: &limarepdf_core::PageObject, n: usize) -> (f32, f32) {
    let text = o.text.as_deref().unwrap();
    let words: Vec<&str> = text.split(' ').collect();
    let from: usize = words[..n].iter().map(|w| w.chars().count() + 1).sum();
    let to = from + words[n].chars().count();
    on(o, (o.stops[from] + o.stops[to]) / 2.0, 0.3)
}

/// Page point `at` on the screen, the page's top left at `min`, `k` pixels a point.
fn screen(min: egui::Pos2, k: f32, at: (f32, f32)) -> egui::Pos2 {
    min + egui::vec2(at.0, at.1) * k
}

/// `t` drawn along line `o` as it will be written: turned as the line is, from its
/// baseline's start, as long as the line is, its letters on the side of the baseline their
/// tops face; in blue when `blue`.
fn along_its_line(
    t: &egui::epaint::TextShape,
    o: &limarepdf_core::PageObject,
    min: egui::Pos2,
    k: f32,
    blue: bool,
) {
    let q = o.quarter_turns.expect("a quarter turn");
    let want = f32::from(q) * FRAC_PI_2;
    let off = (t.angle - want).rem_euclid(TAU);
    assert!(off.min(TAU - off) < 1e-3, "turned {}, not {want}", t.angle);
    let (dx, dy) = o.dir.unwrap();
    let (d, up) = (egui::vec2(dx, dy), egui::vec2(dy, -dx));
    let start = screen(min, k, o.baseline.unwrap());
    let (sin, cos) = t.angle.sin_cos();
    let r = t.galley.rect;
    let corners = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
    ]
    .map(|c| t.pos + egui::vec2(c.x * cos - c.y * sin, c.x * sin + c.y * cos));
    let along = corners.map(|c| (c - start).dot(d));
    let from = along.iter().copied().fold(f32::MAX, f32::min);
    let to = along.iter().copied().fold(f32::MIN, f32::max);
    assert!(
        from.abs() < 2.0,
        "from {from} along the line, not its start"
    );
    let long = (o.stops.last().unwrap() - o.stops[0]) * k;
    assert!(
        (to - from - long).abs() < 0.03 * long,
        "{} long, not {long}",
        to - from
    );
    let middle = corners
        .iter()
        .fold(egui::Pos2::ZERO, |s, c| s + c.to_vec2() / 4.0);
    let line = on_screen(o, min, k);
    assert!(line.contains(middle), "{middle:?} off the line at {line:?}");
    assert!((middle - start).dot(up) > 0.0, "its letters upside down");
    let c = t.galley.job.sections[0].format.color;
    let blued = c.b() as i32 > c.r() as i32 + 80;
    assert_eq!(blued, blue, "coloured {c:?}");
}

/// A press or release of the right button at `p`.
fn right(p: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: Default::default(),
    }
}

/// A double-click on a word of a line reading down, back along or up the page edits it
/// along its line, drawn as it will be written; typed over, Enter keeps it as one step,
/// still turned where it was.
#[test]
fn a_turned_line_is_edited_along_its_line() {
    let ctx = egui::Context::default();
    let mut app = opened(&ctx, turned(), false);
    let ty = Typist::new(&ctx);
    for (text, typed) in [
        ("Reads down the side", "Reads along the side"),
        ("Upside down line", "Upside over line"),
        ("Reads up the side", "Reads past the side"),
    ] {
        let o = line(&app, text);
        let before = steps(&app);
        double_click(&mut app, &ty, on_word(&o, 1));
        let tree = ty.pause(&mut app);
        assert_eq!(editing(&tree).as_deref(), Some(text));
        let (min, k) = ty.page(&mut app);
        let shapes = drawn(&mut app, &ty);
        let shown = in_helvetica(&shapes, text);
        assert_eq!(shown.len(), 1, "{text:?} drawn {} times", shown.len());
        along_its_line(shown[0], &o, min, k, false);

        ty.text(&mut app, typed.split(' ').nth(1).unwrap());
        ty.key(&mut app, Key::Enter, egui::Modifiers::NONE);
        ty.pause(&mut app);
        assert_eq!(app.tabs[0].ed.typing_id(), None, "{text:?} still open");
        let now = line(&app, typed);
        assert_eq!(now.quarter_turns, o.quarter_turns, "{typed:?}");
        let (a, b) = (now.baseline.unwrap(), o.baseline.unwrap());
        assert!(
            (a.0 - b.0).abs() < 0.5 && (a.1 - b.1).abs() < 0.5,
            "{a:?}, not {b:?}"
        );
        assert_eq!(steps(&app), before + 1, "{typed:?} not one step");
    }
}

/// In a turned line being edited, a click puts the caret between the letters clicked,
/// with the input method's window there, across the line; a drag selects the letters
/// along it; a right-click on them keeps them selected and opens their menu.
#[test]
fn a_click_or_drag_in_a_turned_line_goes_by_its_letters() {
    let ctx = egui::Context::default();
    let mut app = opened(&ctx, turned(), false);
    let ty = Typist::new(&ctx);
    let o = line(&app, "Reads down the side");
    double_click(&mut app, &ty, on_word(&o, 1));
    let id = app.tabs[0].ed.typing_id().expect("the line isn't open");
    assert_eq!(
        page_text::selection(&ctx, id),
        Some((6, 10)),
        "down isn't selected"
    );
    let (min, k) = ty.page(&mut app);
    let at = |i: usize| screen(min, k, between(&o, i));

    ty.click(&mut app, at(5));
    assert_eq!(app.tabs[0].ed.typing_id(), Some(id), "the click closed it");
    assert_eq!(page_text::selection(&ctx, id), Some((5, 5)));
    let ime = output(&mut app, &ty).platform_output.ime;
    let ime = ime.expect("the input method is off");
    let start = screen(min, k, o.baseline.unwrap());
    let d = egui::vec2(0.0, 1.0);
    let caret = ime.cursor_rect;
    let along = (caret.center() - start).dot(d);
    assert!(
        (along - o.stops[5] * k).abs() < 2.0,
        "{caret:?} not at the caret"
    );
    // egui's caret is 3 pixels thick.
    assert!(
        caret.height() < 3.1 && caret.width() > 7.0 * k,
        "{caret:?} not across"
    );
    let middle = on_screen(&o, min, k).center();
    assert!(
        ime.rect.contains(middle),
        "{:?} not over the line",
        ime.rect
    );

    ty.frame(&mut app, vec![egui::Event::PointerMoved(at(0))]);
    ty.frame(&mut app, vec![press(at(0), true)]);
    for i in 1..=5 {
        ty.frame(&mut app, vec![egui::Event::PointerMoved(at(i))]);
    }
    ty.frame(&mut app, vec![press(at(5), false)]);
    ty.pause(&mut app);
    assert_eq!(
        page_text::selection(&ctx, id),
        Some((0, 5)),
        "Reads isn't selected"
    );

    ty.frame(&mut app, vec![egui::Event::PointerMoved(at(2))]);
    ty.frame(&mut app, vec![right(at(2), true)]);
    ty.frame(&mut app, vec![right(at(2), false)]);
    let tree = ty.pause(&mut app);
    assert_eq!(
        page_text::selection(&ctx, id),
        Some((0, 5)),
        "let go of Reads"
    );
    let menu = tree
        .nodes
        .iter()
        .any(|(_, n)| n.role() == egui::accesskit::Role::Menu);
    assert!(menu, "no menu");
}

/// Typed past the page's top, a line reading up it says so on a tag beside it, upright, on
/// the side its lines below would be, inside the page.
#[test]
fn a_turned_line_says_what_it_runs_over_beside_it() {
    let ctx = egui::Context::default();
    let mut app = opened(&ctx, turned(), false);
    let ty = Typist::new(&ctx);
    let o = line(&app, "Reads up the side");
    double_click(&mut app, &ty, on_word(&o, 3));
    ty.key(&mut app, Key::End, egui::Modifiers::NONE);
    ty.text(&mut app, &"W".repeat(30));
    let tree = ty.pause(&mut app);
    // A label's text is its value.
    let tag = (tree.nodes.iter())
        .find(|(_, n)| n.value() == Some("Past the page edge"))
        .and_then(|(_, n)| n.bounds())
        .expect("no tag");
    let tag = egui::Rect::from_min_max(
        egui::pos2(tag.x0 as f32, tag.y0 as f32),
        egui::pos2(tag.x1 as f32, tag.y1 as f32),
    );
    let (page, _) = node(&tree, "Page 1").expect("the page");
    let (min, k) = (page.min, page.width() / 612.0);
    let line = on_screen(&o, min, k);
    assert!(tag.width() > tag.height(), "{tag:?} not upright");
    assert!(
        tag.min.x >= line.max.x - 1.0,
        "{tag:?} not right of {line:?}"
    );
    assert!(page.contains_rect(tag), "{tag:?} off the page at {page:?}");
    assert!(tag.min.y < line.min.y, "{tag:?} not where it runs over");
}

/// A double-click on a shown recognised line reading down the page corrects it along its
/// line, stretched as it is written there, in blue; ↓ and ↑ go on to the recognised lines
/// below and above, to its left and right, the caret where it was along them; what is
/// typed there is kept invisible, still turned.
#[test]
fn a_turned_recognised_line_is_corrected_along_its_line() {
    let ctx = egui::Context::default();
    let mut app = opened(&ctx, turned(), true);
    let ty = Typist::new(&ctx);
    let o = line(&app, "Fisrt line down");
    double_click(&mut app, &ty, on_word(&o, 0));
    let tree = ty.pause(&mut app);
    assert_eq!(editing(&tree).as_deref(), Some("Fisrt line down"));
    let (min, k) = ty.page(&mut app);
    let shapes = drawn(&mut app, &ty);
    let shown = in_helvetica(&shapes, "Fisrt line down");
    assert_eq!(shown.len(), 1, "drawn {} times", shown.len());
    along_its_line(shown[0], &o, min, k, true);

    ty.text(&mut app, "First");
    let mut go = |key: Key| {
        ty.key(&mut app, key, egui::Modifiers::NONE);
        editing(&ty.pause(&mut app))
    };
    let second = Some("Second line down");
    assert_eq!(go(Key::ArrowDown).as_deref(), second);
    assert_eq!(go(Key::ArrowDown).as_deref(), Some("Third line down"));
    assert_eq!(go(Key::ArrowUp).as_deref(), second);
    ty.text(&mut app, "X");
    ty.key(&mut app, Key::Enter, egui::Modifiers::NONE);
    ty.pause(&mut app);
    let first = line(&app, "First line down");
    assert!(first.hidden && first.quarter_turns == Some(1), "{first:?}");
    let o = (app.tabs[0].pdf.page_objects(0).unwrap().into_iter())
        .find(|o| o.text.as_deref().is_some_and(|t| t.contains('X')))
        .expect("typed in no line");
    assert!(o.hidden && o.quarter_turns == Some(1), "{o:?}");
    let text = o.text.as_deref().unwrap();
    let placed = text.replace('X', "") == "Second line down" && !text.starts_with('X');
    assert!(placed, "{text:?}");
}

/// A tilted line is still edited in a plain box, its text level there.
#[test]
fn a_tilted_line_is_edited_in_a_box() {
    let ctx = egui::Context::default();
    let mut app = opened(&ctx, turned(), false);
    let ty = Typist::new(&ctx);
    let o = line(&app, "Tilted line");
    assert_eq!(o.quarter_turns, None);
    double_click(&mut app, &ty, on_word(&o, 0));
    let tree = ty.pause(&mut app);
    assert_eq!(editing(&tree).as_deref(), Some("Tilted line"));
    let shapes = drawn(&mut app, &ty);
    let texts: Vec<f32> = (flat(&shapes).into_iter())
        .filter_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == "Tilted line" => Some(t.angle),
            _ => None,
        })
        .collect();
    assert!(
        !texts.is_empty() && texts.iter().all(|&a| a == 0.0),
        "{texts:?}"
    );
}
