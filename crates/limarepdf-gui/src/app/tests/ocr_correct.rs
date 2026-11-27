//! #227: recognised text shown over its print, in blue, the print faded under it, and
//! corrected where it is shown: drawn as it will be written, stretched over the line, one
//! step to undo a line, still invisible; ↓ and ↑ go on to the recognised line below or
//! above. With app.rs's helpers.

use super::*;
use lopdf::{dictionary, Dictionary, Object, Stream};

/// A 612 x 792 page of recognised (invisible) Helvetica lines, 14 points, from x 72:
/// "Teh parcel left Ottawa", about half as wide again, on baseline 92, "It weighs 12
/// kilograms." on 122 and "on Friday." on 152; and a printed line, "Printed line", on 392.
/// The first is written as OCR writes a line: each word placed and stretched on its own.
fn recognised() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    // Text state outlasts BT and ET: each line sets its own.
    let content = "q 1 0 0 1 0 0 cm\n\
                   BT 3 Tr /F1 14 Tf\n\
                   150.0 Tz 1 0 0 1 72.00 700.00 Tm (Teh) Tj\n\
                   140.0 Tz 1 0 0 1 114.00 700.00 Tm (parcel) Tj\n\
                   160.0 Tz 1 0 0 1 173.00 700.00 Tm (left) Tj\n\
                   150.0 Tz 1 0 0 1 209.00 700.00 Tm (Ottawa) Tj\n\
                   ET\nQ\n\
                   BT 3 Tr /F1 14 Tf 100 Tz 72 670 Td (It weighs 12 kilograms.) Tj ET\n\
                   BT 3 Tr /F1 14 Tf 72 640 Td (on Friday.) Tj ET\n\
                   BT 0 Tr /F1 14 Tf 72 400 Td (Printed line) Tj ET\n";
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

/// An app with [`recognised`] open, the read tool on, the window settled with Helvetica's
/// stand-in, Liberation Sans, in; recognised text shown when `shown`.
fn recognised_tab(ctx: &egui::Context, shown: bool) -> App {
    opened(ctx, recognised(), shown)
}

/// [`recognised_tab`] with `pdf` open.
pub(super) fn opened(ctx: &egui::Context, pdf: Pdf, shown: bool) -> App {
    ctx.enable_accesskit();
    theme::install(ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    assert!(app.add_tab(ctx, pdf, None, None, false));
    app.tabs[0].ed.show_hidden = shown;
    settle(&mut app, ctx);
    liberation_in(&mut app, ctx);
    settle(&mut app, ctx);
    app
}

/// The line reading `text` on page 1.
pub(super) fn line(app: &App, text: &str) -> limarepdf_core::PageObject {
    let objs = app.tabs[0].pdf.page_objects(0).unwrap();
    let o = objs.iter().find(|o| o.text.as_deref() == Some(text));
    o.unwrap_or_else(|| panic!("no line {text:?}")).clone()
}

/// One frame, timed as `ty` times it.
pub(super) fn output(app: &mut App, ty: &Typist) -> egui::FullOutput {
    let now = ty.time.get().max(ty.ctx.input(|i| i.time)) + 0.05;
    ty.time.set(now);
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, TEST_HEIGHT),
        )),
        time: Some(ty.time.get()),
        ..Default::default()
    };
    let mut f = eframe::Frame::_new_kittest();
    let mut out = ty.ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut f));
    out.textures_delta.clear();
    out
}

/// What one frame, timed as `ty` times it, draws.
pub(super) fn drawn(app: &mut App, ty: &Typist) -> Vec<egui::epaint::ClippedShape> {
    output(app, ty).shapes
}

/// The shapes `shapes` draw, groups opened.
pub(super) fn flat(shapes: &[egui::epaint::ClippedShape]) -> Vec<&egui::Shape> {
    fn walk<'a>(s: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
        match s {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            s => out.push(s),
        }
    }
    let mut out = Vec::new();
    for c in shapes {
        walk(&c.shape, &mut out);
    }
    out
}

/// The text drawn reading `text` in Liberation Sans, the font Helvetica is written in.
pub(super) fn in_helvetica<'a>(
    shapes: &'a [egui::epaint::ClippedShape],
    text: &str,
) -> Vec<&'a egui::epaint::TextShape> {
    let liberation = egui::FontFamily::Name("Liberation Sans".into());
    (flat(shapes).into_iter())
        .filter_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t),
            _ => None,
        })
        .filter(|t| {
            let first = t.galley.job.sections.first();
            first.is_some_and(|s| s.format.font_id.family == liberation)
        })
        .collect()
}

/// Line `o` on the screen, the page's top left at `min`, `k` pixels a point.
pub(super) fn on_screen(o: &limarepdf_core::PageObject, min: egui::Pos2, k: f32) -> egui::Rect {
    let b = o.bbox;
    egui::Rect::from_min_max(
        min + egui::vec2(b[0], b[1]) * k,
        min + egui::vec2(b[2], b[3]) * k,
    )
}

/// `t` drawn at line `o`, as it will be written: from its baseline's left end, as wide as
/// it is on the page, in blue.
fn at_its_line(
    t: &egui::epaint::TextShape,
    o: &limarepdf_core::PageObject,
    min: egui::Pos2,
    k: f32,
) {
    let (x, _) = o.baseline.expect("a baseline");
    let want = (o.bbox[2] - o.bbox[0]) * k;
    let r = t.galley.rect.translate(t.pos.to_vec2());
    let from = min.x + x * k;
    assert!((r.min.x - from).abs() < 2.0, "from {} not {from}", r.min.x);
    let wide = r.width();
    assert!((wide - want).abs() < 0.03 * want, "{wide} wide, not {want}");
    let line = on_screen(o, min, k);
    assert!(line.contains(r.center()), "{r:?} off the line at {line:?}");
    let c = t.galley.job.sections[0].format.color;
    assert!(c.b() as i32 > c.r() as i32 + 80, "not blue: {c:?}");
}

/// A double-click at page point `at` on page 1, timed as `ty` times it.
pub(super) fn double_click(app: &mut App, ty: &Typist, at: (f32, f32)) {
    let (min, k) = ty.page(app);
    let p = min + egui::vec2(at.0, at.1) * k;
    ty.frame(app, vec![egui::Event::PointerMoved(p)]);
    for _ in 0..2 {
        ty.frame(app, vec![press(p, true)]);
        ty.frame(app, vec![press(p, false)]);
    }
    ty.pause(app);
}

/// The line being edited, by the editor's name.
pub(super) fn editing(tree: &egui::accesskit::TreeUpdate) -> Option<String> {
    (tree.nodes.iter())
        .filter_map(|(_, n)| {
            n.label()?
                .strip_prefix("Editing text: ")
                .map(str::to_string)
        })
        .next()
}

/// How many steps Undo can take back.
pub(super) fn steps(app: &App) -> usize {
    app.tabs[app.active].history().1
}

/// Shown, recognised text is drawn over its line in blue, in the font it is written in,
/// stretched as wide as the line is; the print under it faded by a veil of the paper's
/// colour. Hidden again, it isn't drawn. Printed text isn't drawn over.
#[test]
fn shown_recognised_text_is_drawn_over_its_line() {
    let ctx = egui::Context::default();
    let mut app = recognised_tab(&ctx, false);
    let ty = Typist::new(&ctx);
    let shapes = drawn(&mut app, &ty);
    assert!(
        in_helvetica(&shapes, "Teh parcel left Ottawa").is_empty(),
        "drawn unshown"
    );

    app.act(&ctx, Act::ToggleHidden);
    let (min, k) = ty.page(&mut app);
    let shapes = drawn(&mut app, &ty);
    for text in [
        "Teh parcel left Ottawa",
        "It weighs 12 kilograms.",
        "on Friday.",
    ] {
        let o = line(&app, text);
        let drawn = in_helvetica(&shapes, text);
        assert_eq!(drawn.len(), 1, "{text:?} drawn {} times", drawn.len());
        at_its_line(drawn[0], &o, min, k);
        // The veil: light, not quite opaque, and not tinted blue.
        let middle = on_screen(&o, min, k).center();
        let veiled = flat(&shapes).into_iter().any(|s| match s {
            egui::Shape::Rect(r) => {
                let f = r.fill;
                r.rect.contains(middle) && (60..250).contains(&f.a()) && f.r() >= f.b()
            }
            _ => false,
        });
        assert!(veiled, "no veil over {text:?}");
    }
    let printed = flat(&shapes).into_iter().any(|s| match s {
        egui::Shape::Text(t) => t.galley.text().contains("Printed line"),
        _ => false,
    });
    assert!(!printed, "printed text drawn over");
}

/// A double-click on a shown recognised line edits it where it is shown, drawn as it will
/// be written, the word clicked selected: typed over, Enter keeps it as one step, the line
/// still invisible, searched and saved as corrected; Undo takes it back. Unshown, the
/// double-click selects a word, as before.
#[test]
fn a_recognised_line_is_corrected_where_it_is_shown() {
    let ctx = egui::Context::default();
    let mut app = recognised_tab(&ctx, false);
    let ty = Typist::new(&ctx);
    double_click(&mut app, &ty, (85.0, 88.0));
    assert_eq!(app.tabs[0].ed.typing_id(), None, "edited unshown");
    let words = app.tabs[0].ed.selection.words();
    assert!(words.is_some(), "no word selected");

    app.act(&ctx, Act::ToggleHidden);
    let before = steps(&app);
    double_click(&mut app, &ty, (85.0, 88.0));
    let tree = ty.pause(&mut app);
    assert_eq!(editing(&tree).as_deref(), Some("Teh parcel left Ottawa"));
    let (min, k) = ty.page(&mut app);
    let o = line(&app, "Teh parcel left Ottawa");
    let shapes = drawn(&mut app, &ty);
    let shown = in_helvetica(&shapes, "Teh parcel left Ottawa");
    assert_eq!(shown.len(), 1, "drawn {} times", shown.len());
    at_its_line(shown[0], &o, min, k);

    ty.text(&mut app, "The");
    ty.key(&mut app, Key::Enter, egui::Modifiers::NONE);
    ty.pause(&mut app);
    assert_eq!(app.tabs[0].ed.typing_id(), None, "still open");
    let text = app.tabs[0].pdf.page_text(0);
    let corrected = text.contains("The parcel left Ottawa") && !text.contains("Teh");
    assert!(corrected, "{text:?}");
    assert!(line(&app, "The parcel left Ottawa").hidden, "made visible");
    assert_eq!(steps(&app), before + 1, "not one step");
    let saved = saved_pdf(&app.tabs[0]).page_text(0);
    assert!(saved.contains("The parcel left Ottawa"), "{saved:?}");

    app.act(&ctx, Act::Find);
    ty.frame(&mut app, vec![]);
    ty.frame(&mut app, vec![egui::Event::Text("The parcel".into())]);
    ty.key(&mut app, Key::Enter, egui::Modifiers::NONE);
    searched_with(&mut app, |a| {
        ty.frame(a, vec![]);
    });
    assert_eq!(app.tabs[0].hits.len(), 1, "not found");

    app.act(&ctx, Act::Undo);
    let text = app.tabs[0].pdf.page_text(0);
    assert!(text.contains("Teh parcel left Ottawa"), "{text:?}");
}

/// ↓ and ↑ keep the recognised line being corrected and edit the one below or above, the
/// caret where it was along the line; ↓ on the last line stays on it.
#[test]
fn down_and_up_go_on_to_the_next_recognised_line() {
    let ctx = egui::Context::default();
    let mut app = recognised_tab(&ctx, true);
    let ty = Typist::new(&ctx);
    double_click(&mut app, &ty, (85.0, 88.0));
    ty.text(&mut app, "The");
    let mut go = |key: Key| {
        ty.key(&mut app, key, egui::Modifiers::NONE);
        editing(&ty.pause(&mut app))
    };
    let weighs = Some("It weighs 12 kilograms.");
    assert_eq!(go(Key::ArrowDown).as_deref(), weighs);
    let friday = Some("on Friday.");
    assert_eq!(go(Key::ArrowDown).as_deref(), friday);
    assert_eq!(go(Key::ArrowUp).as_deref(), weighs);
    assert_eq!(go(Key::ArrowDown).as_deref(), friday);
    assert_eq!(go(Key::ArrowDown).as_deref(), friday, "left the last line");
    let text = app.tabs[0].pdf.page_text(0);
    assert!(text.contains("The parcel left Ottawa"), "{text:?}");
    // Typed on there after ↑: along the line where the caret was, not at its start.
    ty.key(&mut app, Key::ArrowUp, egui::Modifiers::NONE);
    ty.pause(&mut app);
    ty.text(&mut app, "X");
    ty.key(&mut app, Key::Enter, egui::Modifiers::NONE);
    ty.pause(&mut app);
    let o = (app.tabs[0].pdf.page_objects(0).unwrap().into_iter())
        .find(|o| o.text.as_deref().is_some_and(|t| t.contains('X')))
        .expect("typed in no line");
    assert!(o.hidden, "made visible");
    let text = o.text.as_deref().unwrap();
    assert!(text.starts_with("It weighs"), "{text:?}");
}

/// Edit text on a recognised line selected with the arrow edits it where it is shown, the
/// caret at its end.
#[test]
fn edit_text_on_a_selected_recognised_line_edits_it_in_place() {
    let ctx = egui::Context::default();
    let mut app = recognised_tab(&ctx, true);
    let ty = Typist::new(&ctx);
    app.act(&ctx, Act::Tool(Tool::Select));
    click_page(&mut app, &ctx, (85.0, 88.0), false);
    let objs = app.tabs[0].pdf.page_objects(0).unwrap();
    let id = objs.iter().position(|o| o.hidden);
    assert_eq!(held(&app), Some((0, vec![id.unwrap()])));
    app.act(&ctx, Act::EditObjectText);
    let tree = ty.pause(&mut app);
    assert_eq!(editing(&tree).as_deref(), Some("Teh parcel left Ottawa"));
    let (min, k) = ty.page(&mut app);
    let o = line(&app, "Teh parcel left Ottawa");
    let shapes = drawn(&mut app, &ty);
    let shown = in_helvetica(&shapes, "Teh parcel left Ottawa");
    assert_eq!(shown.len(), 1, "drawn {} times", shown.len());
    at_its_line(shown[0], &o, min, k);
    ty.text(&mut app, " today");
    ty.key(&mut app, Key::Enter, egui::Modifiers::NONE);
    ty.pause(&mut app);
    let o = line(&app, "Teh parcel left Ottawa today");
    assert!(o.hidden, "made visible");
}

/// #227, read by Poppler: the line [`a_recognised_line_is_corrected_where_it_is_shown`]
/// corrects, saved. Opt-in: `cargo test -p limarepdf-gui --bin limarepdf -- --ignored
/// cross_check`.
#[test]
#[ignore]
fn cross_check_corrected_recognised_text_with_poppler() {
    let ctx = egui::Context::default();
    let mut app = recognised_tab(&ctx, true);
    let ty = Typist::new(&ctx);
    double_click(&mut app, &ty, (85.0, 88.0));
    ty.text(&mut app, "The");
    ty.key(&mut app, Key::Enter, egui::Modifiers::NONE);
    ty.pause(&mut app);
    let dir = test_dir("ocr-corrected-poppler");
    let path = dir.join("corrected.pdf");
    std::fs::write(&path, app.tabs[0].pdf.clone().snapshot().unwrap()).unwrap();
    let pages = poppler_pages(&path);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(pages[0].contains("The parcel left Ottawa"), "{pages:?}");
    assert!(!pages[0].contains("Teh"), "{pages:?}");
}
