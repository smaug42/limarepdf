//! #226: text recognised in an area, a box dragged with Recognise text in an area or a
//! selected picture's, each a background task and one step to undo, with app.rs's helpers.

use super::*;
use limarepdf_core::render::{self, encode, ImageFormat, Renderer};
use limarepdf_core::ImageImport;

fn test_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    ctx
}

/// A 4 x 2 inch page scanned at 300 dpi: "The parcel left Ottawa on Friday." on baseline 40
/// and "It weighs 12 kilograms." on baseline 70, from x 16. With `stamp`, a line of the
/// page's own text below them, "LIM-000123" at 200,130.
fn scan(stamp: bool) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 288.0, 144.0).unwrap();
    for (y, l) in [
        (40.0, "The parcel left Ottawa on Friday."),
        (70.0, "It weighs 12 kilograms."),
    ] {
        p.add_text(0, (16.0, y), l, FontChoice::default(), 14.0, [0.0; 3])
            .unwrap();
    }
    let hp = render::load(p.snapshot().unwrap(), None).unwrap();
    let b = Renderer::new(&hp).render(0, 300.0 / 72.0).unwrap();
    let png = encode(&b, ImageFormat::Png, 300.0).unwrap();
    let opts = ImageImport {
        dpi: Some(300.0),
        ..Default::default()
    };
    let mut scan = Pdf::from_images(&[png], &opts).unwrap();
    if stamp {
        let (stamp, font) = ("LIM-000123", FontChoice::default());
        scan.add_text(0, (200.0, 130.0), stamp, font, 8.0, [0.0; 3])
            .unwrap();
    }
    scan
}

/// An app with [`scan`] open, the window settled.
fn scan_tab(ctx: &egui::Context, stamp: bool) -> App {
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    assert!(app.add_tab(ctx, scan(stamp), None, None, false));
    settle(&mut app, ctx);
    app
}

/// Drag from page point `from` to `to` with the tool in use: [`drag_page`] on the 288-point
/// page.
fn drag_box(app: &mut App, ctx: &egui::Context, from: (f32, f32), to: (f32, f32)) {
    let tree = settled_at(app, ctx, 1400.0);
    let (page, _) = node(&tree, "Page 1").expect("the page");
    let (min, k) = (page.min, page.width() / 288.0);
    let at = |(x, y): (f32, f32)| min + egui::vec2(x, y) * k;
    let (a, b) = (at(from), at(to));
    window(app, ctx, 1400.0, vec![egui::Event::PointerMoved(a)]);
    window(app, ctx, 1400.0, vec![press(a, true)]);
    for i in 1..=4 {
        let p = a + (b - a) * (i as f32 / 4.0);
        window(app, ctx, 1400.0, vec![egui::Event::PointerMoved(p)]);
    }
    window(app, ctx, 1400.0, vec![press(b, false)]);
    settle(app, ctx);
}

/// Takes in the task's result once it ends, for two minutes at most: recognising is slow in
/// a test build.
fn task_done(app: &mut App, ctx: &egui::Context) {
    for _ in 0..12000 {
        app.poll_loads(ctx);
        if !app.busy {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the task didn't end");
}

/// How many steps Undo can take back.
fn steps(app: &App) -> usize {
    app.tabs[app.active].history().1
}

fn scanned(app: &App) -> bool {
    (app.tabs[0].banners.iter()).any(|b| b.kind == BannerKind::Scanned)
}

/// Recognise an area, from the OCR section, turns on a tool whose box is read in the
/// background, as one step to undo, said in a toast: the line in the box is added, the one
/// below it isn't. The tool stays for the next box, and the scan banner stays, as the rest of
/// the page is still a scan.
#[test]
fn a_box_dragged_over_a_scan_is_read_as_one_step() {
    let ctx = test_ctx();
    let mut app = scan_tab(&ctx, false);
    assert!(scanned(&app), "no scan banner");
    app.reveal(Section::Ocr);
    click_named(&mut app, &ctx, "Recognise an area");
    assert_eq!(app.tabs[0].ed.tool, Tool::OcrArea);
    let before = steps(&app);
    drag_box(&mut app, &ctx, (2.0, 20.0), (286.0, 55.0));
    assert!(app.busy, "nothing started");
    task_done(&mut app, &ctx);
    let text = app.tabs[0].pdf.page_text(0);
    let read = text.contains("Ottawa") && !text.contains("kilograms");
    assert!(read, "{text:?}");
    assert_eq!(steps(&app), before + 1);
    let said = toast(&app).expect("no toast");
    assert!(said.contains("words recognised in the area"), "{said}");
    assert!(matches!(app.dialog, Dialog::None), "a message too");
    assert!(scanned(&app), "the scan banner went");
    assert_eq!(app.tabs[0].ed.tool, Tool::OcrArea);

    app.act(&ctx, Act::Undo);
    let text = app.tabs[0].pdf.page_text(0);
    assert!(!text.contains("Ottawa"), "{text:?}");
}

/// A box with no text in it adds nothing, and a message says why.
#[test]
fn a_box_with_no_text_says_so() {
    let ctx = test_ctx();
    let mut app = scan_tab(&ctx, false);
    app.act(&ctx, Act::Tool(Tool::OcrArea));
    let before = steps(&app);
    drag_box(&mut app, &ctx, (200.0, 90.0), (280.0, 140.0));
    task_done(&mut app, &ctx);
    assert_eq!(steps(&app), before, "a step to undo");
    let (title, text) = message(&app);
    assert_eq!(title, "Recognise text in an area");
    assert!(text.contains("no text found"), "{text}");
}

/// A box dragged while a task runs isn't read, and a toast says to wait; the task under way
/// goes on.
#[test]
fn a_box_while_a_task_runs_is_refused() {
    let ctx = test_ctx();
    let mut app = scan_tab(&ctx, false);
    app.act(&ctx, Act::Tool(Tool::OcrArea));
    let (go, gate) = channel();
    app.job_gate = Some(gate);
    drag_box(&mut app, &ctx, (2.0, 20.0), (286.0, 55.0));
    assert!(app.busy, "nothing started");
    let jobs = app.jobs.len();
    drag_box(&mut app, &ctx, (2.0, 56.0), (286.0, 85.0));
    assert_eq!(app.jobs.len(), jobs, "a second task started");
    let said = toast(&app).expect("no toast");
    assert!(said.contains("task under way"), "{said}");
    go.send(()).unwrap();
    task_done(&mut app, &ctx);
    let text = app.tabs[0].pdf.page_text(0);
    let read = text.contains("Ottawa") && !text.contains("kilograms");
    assert!(read, "{text:?}");
}

/// A selected picture's Recognise text reads the picture's box, on a page with text of its
/// own, which recognising pages leaves alone: both lines added, the page's own line not
/// read again.
#[test]
fn a_selected_pictures_text_is_recognised() {
    let ctx = test_ctx();
    let mut app = scan_tab(&ctx, true);
    app.act(&ctx, Act::Tool(Tool::Select));
    let picture = (app.tabs[0].pdf.page_objects(0).unwrap().iter())
        .position(|o| o.kind == limarepdf_core::ObjectKind::Image)
        .expect("the picture");
    select(&mut app, &ctx, &[picture]);
    click_named(&mut app, &ctx, "Recognise text");
    assert!(app.busy, "nothing started");
    task_done(&mut app, &ctx);
    let text = app.tabs[0].pdf.page_text(0);
    let read = text.contains("Ottawa") && text.contains("kilograms");
    assert!(read, "{text:?}");
    assert_eq!(text.matches("LIM-000123").count(), 1, "{text:?}");
    let said = toast(&app).expect("no toast");
    assert!(said.contains("words recognised in the area"), "{said}");
}
