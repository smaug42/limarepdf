//! Batch 42 (PF-37): redaction marks moved and resized on the page as other comments are,
//! and Apply removing what is under them where they now are, with app.rs's helpers.

use super::*;
use limarepdf_core::{Annot, AnnotKind};

/// Over "Walrus one" in [`objects_tab`] (18 points high, its baseline at 92, from x 72),
/// and 30 points lower, over "Walrus two" (baseline 122).
const ONE: [f32; 4] = [70.0, 76.0, 166.0, 96.0];
const TWO: [f32; 4] = [70.0, 106.0, 166.0, 126.0];

fn test_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    ctx
}

/// The active tab's one redaction mark, as read from the document now.
fn mark(app: &App) -> Annot {
    let annots = app.tabs[app.active].pdf.annotations();
    let marks: Vec<Annot> = (annots.into_iter())
        .filter(|a| a.kind == AnnotKind::Redact)
        .collect();
    assert_eq!(marks.len(), 1, "{} redaction marks", marks.len());
    marks.into_iter().next().unwrap()
}

/// [`objects_tab`] with one redaction mark over `areas` (page points, y down) on its first
/// page, as the Redact tool or Mark for redaction makes one; the window settled. The
/// mark's id.
fn marked(ctx: &egui::Context, areas: &[[f32; 4]]) -> (App, (u32, u16)) {
    let mut app = objects_tab(ctx);
    let add = |p: &mut Pdf| p.add_redaction_mark(0, areas, "").map(drop);
    assert!(app.tabs[0].edit(ctx, add));
    let id = mark(&app).id;
    settle(&mut app, ctx);
    (app, id)
}

/// Box `b` moved by (`dx`, `dy`).
fn shifted(b: [f32; 4], dx: f32, dy: f32) -> [f32; 4] {
    [b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy]
}

/// Whether boxes `a` and `b` are the same, but for the pointer's rounding.
fn same(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(&b).all(|(p, q)| (p - q).abs() < 0.6)
}

/// The mark's areas.
fn areas(app: &App) -> Vec<[f32; 4]> {
    mark(app).quads
}

/// Select the mark with a click inside it at `at` (page points), with the read tool.
fn click_mark(app: &mut App, ctx: &egui::Context, at: (f32, f32), id: (u32, u16)) {
    click_page(app, ctx, at, false);
    let sel = app.tabs[app.active].ed.selection.annot();
    assert_eq!(sel, Some((0, id)), "the mark isn't selected");
}

/// How many steps Undo can take back.
fn steps(app: &App) -> usize {
    app.tabs[app.active].history().1
}

/// Drag from page point `from` by (`dx`, `dy`) points, with the tool in use.
fn drag_by(app: &mut App, ctx: &egui::Context, from: (f32, f32), (dx, dy): (f32, f32)) {
    drag_page(app, ctx, from, (from.0 + dx, from.1 + dy), false);
}

/// Press `key` with `held` keys down, let go, and let the window settle.
fn press_key(app: &mut App, ctx: &egui::Context, key: Key, held: egui::Modifiers) {
    window_held(app, ctx, vec![key_with(key, held)], held);
    window_held(app, ctx, vec![], egui::Modifiers::NONE);
    settle(app, ctx);
}

/// PF-37: a redaction mark selected with the read tool moves with a drag from inside it,
/// as other comments do, its area with it, in one Undo step; Undo and Redo take it back
/// and do it again; Apply then removes what is under it where it now is, not where it was.
#[test]
fn a_redaction_mark_moves_with_a_drag() {
    let ctx = test_ctx();
    let (mut app, id) = marked(&ctx, &[ONE]);
    assert_eq!(app.tabs[0].ed.tool, Tool::Read);
    let was = mark(&app);
    click_mark(&mut app, &ctx, (100.0, 86.0), id);
    let before = steps(&app);
    drag_by(&mut app, &ctx, (100.0, 86.0), (0.0, 30.0));
    let now = mark(&app);
    let down = shifted(was.bbox, 0.0, 30.0);
    assert!(same(now.bbox, down), "{:?} to {:?}", was.bbox, now.bbox);
    assert_eq!(now.quads.len(), 1, "{:?}", now.quads);
    assert!(same(now.quads[0], TWO), "its area at {:?}", now.quads[0]);
    assert_eq!(steps(&app), before + 1, "not one Undo step");
    app.act(&ctx, Act::Undo);
    assert!(same(areas(&app)[0], ONE), "Undo: {:?}", areas(&app));
    app.act(&ctx, Act::Redo);
    assert!(same(areas(&app)[0], TWO), "Redo: {:?}", areas(&app));
    // Applied, the question answered: "Walrus two" goes, "Walrus one" stays.
    app.act(&ctx, Act::ApplyRedactions);
    click(&mut app, &ctx, "Apply redactions");
    let text = app.tabs[0].pdf.page_text(0);
    assert!(app.tabs[0].pdf.redaction_marks().is_empty(), "not applied");
    assert!(text.contains("Walrus one"), "{text:?}");
    assert!(text.contains("Walrus three"), "{text:?}");
    assert!(!text.contains("Walrus two"), "{text:?}");
}

/// A mark with one area resizes by its box's handles, as a rectangle does; its area stays
/// exactly 1 point inside the box (core's padding), however far it is stretched.
#[test]
fn a_one_area_mark_resizes_by_its_handles() {
    let ctx = test_ctx();
    let (mut app, id) = marked(&ctx, &[ONE]);
    click_mark(&mut app, &ctx, (100.0, 86.0), id);
    let inside_by_one = |a: &Annot, what: &str| {
        assert_eq!(a.quads.len(), 1, "{what}: {:?}", a.quads);
        let (q, b) = (a.quads[0], a.bbox);
        let pad = [q[0] - b[0], q[1] - b[1], b[2] - q[2], b[3] - q[3]];
        let one = pad.iter().all(|p| (p - 1.0).abs() <= 0.05);
        assert!(one, "{what}: its area {q:?} in its box {b:?}");
    };
    let b = mark(&app).bbox;
    // The bottom-right handle, 40 points right and 20 down.
    let before = steps(&app);
    drag_by(&mut app, &ctx, (b[2], b[3]), (40.0, 20.0));
    let a = mark(&app);
    let to = [b[0], b[1], b[2] + 40.0, b[3] + 20.0];
    assert!(same(a.bbox, to), "not resized: {b:?} to {:?}", a.bbox);
    assert_eq!(steps(&app), before + 1, "not one Undo step");
    inside_by_one(&a, "resized");
    // To about three times its first size.
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let c = a.bbox;
    let far = (b[0] + 3.0 * w, b[1] + 3.0 * h);
    drag_page(&mut app, &ctx, (c[2], c[3]), far, false);
    let a = mark(&app);
    let to = [b[0], b[1], far.0, far.1];
    assert!(same(a.bbox, to), "not stretched: {c:?} to {:?}", a.bbox);
    inside_by_one(&a, "stretched");
}

/// A mark over several lines (an area each) moves as one, every area by the same amount;
/// it has no handles, so a drag that starts just inside its bottom-right corner moves it
/// too, leaving its size.
#[test]
fn a_mark_over_several_lines_moves_whole_without_handles() {
    let ctx = test_ctx();
    let (mut app, id) = marked(&ctx, &[ONE, TWO]);
    click_mark(&mut app, &ctx, (100.0, 86.0), id);
    let was = mark(&app);
    assert_eq!(was.quads.len(), 2);
    drag_by(&mut app, &ctx, (100.0, 86.0), (40.0, 150.0));
    let now = mark(&app);
    assert_eq!(now.quads.len(), 2);
    for (a, b) in now.quads.iter().zip(&was.quads) {
        assert!(same(*a, shifted(*b, 40.0, 150.0)), "an area {b:?} to {a:?}");
    }
    let (b, to) = (now.bbox, shifted(was.bbox, 40.0, 150.0));
    assert!(same(b, to), "its box {b:?}, not {to:?}");
    // Just inside the lower area's bottom-right corner, where a handle would be.
    let low = (now.quads.iter())
        .copied()
        .max_by(|a, b| a[3].total_cmp(&b[3]))
        .unwrap();
    drag_by(&mut app, &ctx, (low[2] - 1.0, low[3] - 1.0), (30.0, 20.0));
    let last = mark(&app);
    for (a, b) in last.quads.iter().zip(&now.quads) {
        assert!(same(*a, shifted(*b, 30.0, 20.0)), "an area {b:?} to {a:?}");
    }
    let (b, to) = (last.bbox, shifted(now.bbox, 30.0, 20.0));
    assert!(same(b, to), "its box {b:?}, not {to:?}");
}

/// The arrow keys move a mark selected with the read tool or the arrow, as they move other
/// comments: a point, ten with Shift, one Undo step each.
#[test]
fn the_arrow_keys_move_a_selected_mark() {
    let ctx = test_ctx();
    for tool in [Tool::Read, Tool::Select] {
        let (mut app, id) = marked(&ctx, &[ONE]);
        app.act(&ctx, Act::Tool(tool));
        click_mark(&mut app, &ctx, (100.0, 86.0), id);
        let before = steps(&app);
        let (none, shift) = (egui::Modifiers::NONE, egui::Modifiers::SHIFT);
        press_key(&mut app, &ctx, Key::ArrowRight, none);
        let at = areas(&app)[0];
        assert!(same(at, shifted(ONE, 1.0, 0.0)), "{tool:?}: {at:?}");
        let one = at[0] - ONE[0];
        assert!((one - 1.0).abs() < 0.05, "{tool:?}: moved {one}");
        assert_eq!(steps(&app), before + 1, "{tool:?}: not one Undo step");
        press_key(&mut app, &ctx, Key::ArrowRight, shift);
        let ten = areas(&app)[0][0] - ONE[0] - 1.0;
        assert!((ten - 10.0).abs() < 0.05, "{tool:?}: Shift, {ten}");
        press_key(&mut app, &ctx, Key::ArrowDown, none);
        let at = areas(&app)[0];
        assert!(same(at, shifted(ONE, 11.0, 1.0)), "{tool:?}: {at:?}");
        assert_eq!(steps(&app), before + 3, "{tool:?}: not a step each");
    }
}

/// A locked mark (annotation flag Locked) can be selected but never moves: not by a drag
/// from inside it, nor by its corner, nor by the arrow keys.
#[test]
fn a_locked_mark_stays_put() {
    let ctx = test_ctx();
    let (mut app, id) = marked(&ctx, &[ONE]);
    let lock = |p: &mut Pdf| {
        p.doc.get_dictionary_mut(id)?.set("F", 128_i64);
        Ok(())
    };
    assert!(app.tabs[0].edit(&ctx, lock));
    settle(&mut app, &ctx);
    let was = mark(&app);
    assert!(was.locked());
    click_mark(&mut app, &ctx, (100.0, 86.0), id);
    drag_by(&mut app, &ctx, (100.0, 86.0), (0.0, 30.0));
    let b = was.bbox;
    drag_by(&mut app, &ctx, (b[2], b[3]), (40.0, 20.0));
    press_key(&mut app, &ctx, Key::ArrowRight, egui::Modifiers::NONE);
    let now = mark(&app);
    assert_eq!((now.bbox, &now.quads), (was.bbox, &was.quads), "moved");
}

/// A mark moved and saved is where it was moved when the file is opened again, its area
/// with it.
#[test]
fn a_moved_mark_is_saved_where_it_was_moved() {
    let ctx = test_ctx();
    let (mut app, id) = marked(&ctx, &[ONE]);
    click_mark(&mut app, &ctx, (100.0, 86.0), id);
    drag_by(&mut app, &ctx, (100.0, 86.0), (0.0, 30.0));
    let moved = mark(&app);
    assert!(same(moved.quads[0], TWO), "not moved: {:?}", moved.quads);
    let dir = test_dir("moved-mark");
    let path = dir.join("moved.pdf");
    SAVE_AS.with(|s| s.set(Some(path.clone())));
    app.act(&ctx, Act::SaveAs);
    // Saved with its mark not applied, as asked.
    click(&mut app, &ctx, "Save with marks");
    saves_done(&mut app, &ctx);
    let back = Pdf::open(&path, None).expect("the saved file");
    let _ = std::fs::remove_dir_all(&dir);
    let marks: Vec<Annot> = (back.annotations().into_iter())
        .filter(|a| a.kind == AnnotKind::Redact)
        .collect();
    assert_eq!(marks.len(), 1, "{} marks", marks.len());
    let (b, to) = (marks[0].bbox, moved.bbox);
    assert!(same(b, to), "its box {b:?}, moved to {to:?}");
    let (q, to) = (&marks[0].quads, moved.quads[0]);
    assert!(q.len() == 1 && same(q[0], to), "{q:?}, moved to {to:?}");
}

/// The banner for marks not yet applied says "it" for one mark and "them" for more.
#[test]
fn the_banner_for_one_mark_says_it() {
    let one = crate::banners::redactions(1).text;
    assert!(one.contains("nothing under it has been removed"), "{one}");
    let two = crate::banners::redactions(2).text;
    assert!(two.contains("nothing under them has been removed"), "{two}");
}

/// PB-40: the notice and the Redact tool's pane count marks, not areas: a mark over a line
/// of text and one drawn as a box are two marks.
#[test]
fn the_notice_and_the_pane_count_marks() {
    let one = crate::banners::redactions(1).text;
    assert!(
        one.starts_with("A mark for redaction (outlined in red)"),
        "{one}"
    );
    let two = crate::banners::redactions(2).text;
    assert!(
        two.starts_with("2 marks for redaction (outlined in red)"),
        "{two}"
    );
    for text in [one, two] {
        assert!(!text.contains("area"), "{text}");
    }
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = objects_tab(&ctx);
    app.tabs[0].ed.set_tool(Tool::Redact);
    let shown = |tree: &egui::accesskit::TreeUpdate, s: &str| {
        (tree.nodes.iter()).any(|(_, n)| n.value() == Some(s))
    };
    assert!(shown(&ty.pause(&mut app), "No marks yet."), "none");
    for (area, says) in [(ONE, "1 mark."), (TWO, "2 marks.")] {
        let add = |p: &mut Pdf| p.add_redaction_mark(0, &[area], "").map(drop);
        assert!(app.tabs[0].edit(&ctx, add));
        assert!(shown(&ty.pause(&mut app), says), "not {says:?}");
    }
}

/// Apply redactions… in the Protect band is greyed with no marks to apply and says why on
/// hover, as the command does wherever it is; with a mark, it is on.
#[test]
fn apply_redactions_is_greyed_with_no_marks() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = objects_tab(&ctx);
    app.band = Band::Protect;
    let why = "Mark something for redaction first.";
    let apply = "Apply redactions…";
    let greyed = |tree: &egui::accesskit::TreeUpdate| {
        let named = tree.nodes.iter().filter(|(_, n)| n.label() == Some(apply));
        let states: Vec<bool> = named.map(|(_, n)| n.is_disabled()).collect();
        assert!(!states.is_empty(), "no {apply}");
        states.iter().all(|&off| off)
    };
    let tree = ty.pause(&mut app);
    assert!(greyed(&tree), "on with no marks");
    let (r, _) = node(&tree, apply).expect("the band's Apply redactions…");
    ty.frame(&mut app, vec![egui::Event::PointerMoved(r.center())]);
    let tree = ty.pause(&mut app);
    let says = (tree.nodes.iter()).any(|(_, n)| n.value().is_some_and(|v| v.contains(why)));
    assert!(says, "no reason on hover");
    let a = actions::get(Act::ApplyRedactions);
    let c = app.state(&ctx);
    assert!(!(a.enabled)(&c), "the command is on");
    assert_eq!(actions::why_not(&c, a), why);
    // A mark made: on.
    let add = |p: &mut Pdf| p.add_redaction_mark(0, &[ONE], "").map(drop);
    assert!(app.tabs[0].edit(&ctx, add));
    let tree = ty.pause(&mut app);
    assert!(!greyed(&tree), "greyed with a mark");
    assert!((a.enabled)(&app.state(&ctx)), "the command is greyed");
}

/// Whether box `b` is inside `area`, but for half a point.
fn inside(b: [f32; 4], area: [f32; 4]) -> bool {
    area[0] <= b[0] + 0.5 && area[1] <= b[1] + 0.5 && area[2] >= b[2] - 0.5 && area[3] >= b[3] - 0.5
}

/// Whether boxes `a` and `b` overlap by more than a twentieth of a point each way.
fn overlap(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0].max(b[0]) + 0.05 < a[2].min(b[2]) && a[1].max(b[1]) + 0.05 < a[3].min(b[3])
}

/// Journey B, the marking step: words selected with the read tool by a drag across
/// "Walrus two", then Mark for redaction. One mark is made, in one Undo step, with one area
/// over both words and over no other text, and nothing else is added; the words are let go
/// so the mark shows. Applied, it takes those words and no others.
#[test]
fn selected_words_are_marked_for_redaction() {
    let ctx = test_ctx();
    let mut app = objects_tab(&ctx);
    assert_eq!(app.tabs[0].ed.tool, Tool::Read);
    let annots = app.tabs[0].pdf.annotations().len();
    // From inside its "W" to inside the "o" of "two".
    drag_page(&mut app, &ctx, (74.0, 116.0), (158.0, 116.0), false);
    let words = app.tabs[0].ed.selection.words();
    assert!(words.is_some(), "no words: {:?}", app.tabs[0].ed.selection);
    let before = steps(&app);
    app.act(&ctx, Act::MarkRedact);
    settle(&mut app, &ctx);
    let m = mark(&app);
    let now = app.tabs[0].pdf.annotations().len();
    assert_eq!(now, annots + 1, "more added");
    assert_eq!(steps(&app), before + 1, "not one Undo step");
    assert_eq!(m.quads.len(), 1, "{:?}", m.quads);
    let area = m.quads[0];
    let page = app.tabs[0].pdf.page_words(0).unwrap();
    let two = page.iter().find(|w| w.text == "two").expect("the word two");
    for w in &page {
        let (b, word) = (w.bbox, &w.text);
        if w.line == two.line {
            assert!(inside(b, area), "{word} at {b:?} not under {area:?}");
        } else {
            assert!(!overlap(b, area), "{word} at {b:?} under {area:?}");
        }
    }
    let sel = &app.tabs[0].ed.selection;
    assert!(sel.words().is_none(), "still selected: {sel:?}");
    let mut applied = app.tabs[0].pdf.clone();
    applied.apply_redactions().unwrap();
    let text = applied.page_text(0);
    assert!(!text.contains("two"), "{text:?}");
    assert_eq!(text.matches("Walrus").count(), 2, "{text:?}");
    for kept in ["Walrus one", "Walrus three", "hidden words"] {
        assert!(text.contains(kept), "{kept} went: {text:?}");
    }
}

/// "Secret" with its t shown as `[(t) 278] TJ`, taking back its whole width, and "here"
/// apart: Helvetica at 20 pt, the baseline 100 points down, t drawn from x 82.24 to 87.8.
fn taken_back() -> Pdf {
    use lopdf::dictionary;
    let content = "BT /F1 20 Tf 30 100 Td (Secre) Tj [(t) 278] TJ ET \
                   BT /F1 20 Tf 120 100 Td (here) Tj ET";
    page_of(content, |p, _| {
        let font = p.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        dictionary! { "Font" => dictionary! { "F1" => font } }
    })
}

/// PB-78: "Secret" marked for redaction from words selected and from a search takes its t,
/// whose show takes its width back: the mark reaches the t's end, and applied, it leaves
/// only "here".
#[test]
fn a_letter_whose_width_is_taken_back_is_marked_with_its_word() {
    let ctx = test_ctx();
    for route in ["selection", "search"] {
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        assert!(app.add_tab(&ctx, taken_back(), None, None, false));
        settle(&mut app, &ctx);
        if route == "selection" {
            // From inside the S to inside the t.
            drag_page(&mut app, &ctx, (32.0, 94.0), (86.0, 94.0), false);
            let sel = &app.tabs[0].ed.selection;
            assert!(sel.words().is_some(), "no words: {sel:?}");
            app.act(&ctx, Act::MarkRedact);
        } else {
            app.tabs[0].query = "Secret".into();
            app.act(&ctx, Act::MarkResultsRedact);
        }
        settle(&mut app, &ctx);
        let tab = &app.tabs[0];
        assert!(tab.error.is_none(), "{route}: {:?}", tab.error);
        let quads = mark(&app).quads;
        let mut applied = app.tabs[0].pdf.clone();
        let rep = applied.apply_redactions().unwrap();
        let text = applied.page_text(0);
        assert_eq!(
            text.replace(' ', ""),
            "here",
            "{route}: {text:?}, {:?}",
            rep.left
        );
        assert!(rep.left.is_empty(), "{route}: {:?}", rep.left);
        let end = quads.iter().map(|q| q[2]).fold(f32::MIN, f32::max);
        assert!(end > 87.75, "{route}: the mark ends at x {end}: {quads:?}");
    }
}

/// Journey B, the applying step: a mark applied (the question answered) takes its text
/// off the page and the mark with it, in one Undo step; one Undo brings back the text
/// and the mark where it was, and Redo takes them away again.
#[test]
fn one_undo_takes_back_applied_redactions() {
    let ctx = test_ctx();
    let (mut app, _) = marked(&ctx, &[TWO]);
    let text = |app: &App| app.tabs[0].pdf.page_text(0);
    let all = ["Walrus one", "Walrus two", "Walrus three"];
    let was = text(&app);
    assert!(all.iter().all(|l| was.contains(l)), "{was:?}");
    let before = steps(&app);
    app.act(&ctx, Act::ApplyRedactions);
    click(&mut app, &ctx, "Apply redactions");
    let gone = |app: &App, when: &str| {
        let now = text(app);
        assert!(!now.contains("two"), "{when}: {now:?}");
        let kept = now.contains("Walrus one") && now.contains("Walrus three");
        assert!(kept, "{when}: {now:?}");
        let marks = app.tabs[0].pdf.redaction_marks();
        assert!(marks.is_empty(), "{when}: marks left {marks:?}");
    };
    gone(&app, "applied");
    assert_eq!(steps(&app), before + 1, "not one Undo step");
    assert_eq!(message(&app).0, "Redactions applied");
    click(&mut app, &ctx, "OK");
    assert!(matches!(app.dialog, Dialog::None));

    app.act(&ctx, Act::Undo);
    let now = text(&app);
    assert!(all.iter().all(|l| now.contains(l)), "Undo: {now:?}");
    let back = areas(&app);
    let one = back.len() == 1 && same(back[0], TWO);
    assert!(one, "Undo: the mark at {back:?}");
    assert_eq!(steps(&app), before, "Undo");

    app.act(&ctx, Act::Redo);
    gone(&app, "Redo");
    assert_eq!(steps(&app), before + 1, "Redo");
}

/// A 300 × 200 pt page drawing `content` with the resources `objects` returns, given the
/// page once it has added the objects they list.
fn page_of(
    content: &str,
    objects: impl FnOnce(&mut Pdf, lopdf::ObjectId) -> lopdf::Dictionary,
) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let page = p.page_ids()[0];
    let res = objects(&mut p, page);
    let s = p.doc.add_object(lopdf::Stream::new(
        lopdf::Dictionary::new(),
        content.as_bytes().to_vec(),
    ));
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", s);
    d.set("Resources", res);
    p
}

/// A tab showing `pdf`, with one redaction mark over `areas` (page points, y down) on its
/// page; the window settled.
fn marked_tab(ctx: &egui::Context, pdf: Pdf, areas: &[[f32; 4]]) -> App {
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    assert!(app.add_tab(ctx, pdf, None, None, false));
    let add = |p: &mut Pdf| p.add_redaction_mark(0, areas, "").map(drop);
    assert!(app.tabs[0].edit(ctx, add));
    settle(&mut app, ctx);
    app
}

/// CP-03: applying a mark over a picture that a form left as it is also draws says once,
/// in the message, that the page still draws the original.
#[test]
fn applying_says_where_an_original_picture_is_still_drawn() {
    use lopdf::{dictionary, Object, Stream};
    let ctx = test_ctx();
    let content = "q 50 0 0 50 10 100 cm /Im1 Do Q q 1 0 0 1 110 100 cm /B Do Q";
    let pdf = page_of(content, |p, _| {
        let picture = dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        };
        let im = p.doc.add_object(Stream::new(picture, vec![255]));
        let bbox: Vec<Object> = [0, 0, 50, 50].map(Object::from).to_vec();
        let form = dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => bbox };
        let b = (p.doc).add_object(Stream::new(form, b"q 50 0 0 50 0 0 cm /Im1 Do Q".to_vec()));
        dictionary! { "XObject" => dictionary! { "Im1" => im, "B" => b } }
    });
    let mut app = marked_tab(&ctx, pdf, &[[10.0, 50.0, 60.0, 100.0]]);
    app.act(&ctx, Act::ApplyRedactions);
    click(&mut app, &ctx, "Apply redactions");
    let (title, text) = message(&app);
    assert_eq!(title, "Redactions applied");
    let notice = "An original picture affected by redaction is still drawn elsewhere on page 1 \
                  and may retain the removed content.";
    assert_eq!(text.matches(notice).count(), 1, "{text}");
}

/// A3: applying a mark whose removed content the file would keep through the tag tree is
/// refused, saying why, and changes nothing: not the page, not the mark, and not the
/// pattern nothing paints with, whose copy of the original form goes before the check
/// refuses.
#[test]
fn applying_what_the_tag_tree_would_keep_changes_nothing() {
    use lopdf::{dictionary, Object, Stream};
    let ctx = test_ctx();
    let pdf = page_of("q /Fm1 Do Q", |p, page| {
        let font = p.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let bbox: Vec<Object> = [0, 0, 300, 200].map(Object::from).to_vec();
        let form = dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => bbox.clone(),
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        };
        let text = "BT /F1 20 Tf 30 100 Td (FormSecretZebra) Tj ET \
                    BT /F1 20 Tf 30 40 Td (Kept) Tj ET";
        let fm = p
            .doc
            .add_object(Stream::new(form, text.as_bytes().to_vec()));
        let pattern = dictionary! {
            "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
            "BBox" => bbox, "XStep" => 300, "YStep" => 200,
            "Resources" => dictionary! { "XObject" => dictionary! { "Original" => fm } },
        };
        let pt = p
            .doc
            .add_object(Stream::new(pattern, b"/Original Do".to_vec()));
        let root = p.doc.new_object_id();
        let mcr = dictionary! { "Type" => "MCR", "Pg" => page, "Stm" => fm, "MCID" => 0 };
        let elem = p.doc.add_object(dictionary! {
            "Type" => "StructElem", "S" => "P", "P" => root, "Pg" => page,
            "K" => vec![Object::Dictionary(mcr)],
        });
        let tree = dictionary! { "Type" => "StructTreeRoot", "K" => elem };
        p.doc.objects.insert(root, tree.into());
        let c = p.doc.catalog_mut().unwrap();
        c.set("StructTreeRoot", root);
        c.set("MarkInfo", dictionary! { "Marked" => true });
        dictionary! {
            "XObject" => dictionary! { "Fm1" => fm }, "Pattern" => dictionary! { "Pt" => pt },
        }
    });
    let boxes: Vec<[f32; 4]> = (pdf.find_redaction_marks("formsecretzebra").unwrap())
        .into_iter()
        .map(|m| m.1)
        .collect();
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    let mut app = marked_tab(&ctx, pdf, &boxes);
    let (was, before) = (app.tabs[0].state(), app.tabs[0].pdf.state_before());
    app.act(&ctx, Act::ApplyRedactions);
    click(&mut app, &ctx, "Apply redactions");
    let t = &mut app.tabs[0];
    let said = t.error.take().unwrap_or_default();
    let why = "removed content may still be in the file, through the tag tree";
    assert!(
        said.starts_with("Nothing was changed") && said.contains(why),
        "{said}"
    );
    assert!(t.state() == was, "the tab changed");
    assert_eq!(t.pdf.state_since(&before), []);
    assert_eq!(t.pdf.redaction_marks().len(), 1);
    assert!(t.pdf.page_text(0).contains("FormSecretZebra"));
    let at = |o: &Object| t.pdf.doc.dereference(o).unwrap().1.clone();
    let res = at(t
        .pdf
        .doc
        .get_dictionary(t.pdf.page_ids()[0])
        .unwrap()
        .get(b"Resources")
        .unwrap());
    let pt = at(res.as_dict().unwrap().get(b"Pattern").unwrap());
    let pt = at(pt.as_dict().unwrap().get(b"Pt").unwrap());
    let listed = &pt.as_stream().unwrap().dict;
    let listed = at(listed.get(b"Resources").unwrap());
    let listed = at(listed.as_dict().unwrap().get(b"XObject").unwrap());
    assert!(listed.as_dict().unwrap().has(b"Original"), "{listed:?}");
}
