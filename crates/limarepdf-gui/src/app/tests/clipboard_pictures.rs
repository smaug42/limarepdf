//! Pictures and the system's clipboard (NEW-09's stage 3): a picture or screenshot copied in
//! another app is pasted, with nothing being typed, in the middle of the page in view at its
//! own size (shrunk to fit), selected, in one step, as Word and PowerPoint paste one; a
//! selected picture is copied out as it shows.

use super::save_image::{halves, picture_tab};
use super::*;
use limarepdf_core::render::{self, ImageFormat};
use limarepdf_core::ObjectKind;
use std::sync::Arc;

/// A `w` × `h` picture as the clipboard gives one: opaque teal.
fn teal(w: usize, h: usize) -> egui::ColorImage {
    egui::ColorImage::new([w, h], vec![egui::Color32::from_rgb(0, 128, 128); w * h])
}

fn paste(image: egui::ColorImage) -> Vec<egui::Event> {
    vec![egui::Event::PasteImage(Arc::new(image))]
}

/// One frame of the window with `events`; the pictures it put on the system's clipboard.
fn images_copied(
    app: &mut App,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> Vec<egui::ColorImage> {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, TEST_HEIGHT),
        )),
        events,
        ..Default::default()
    };
    let mut f = eframe::Frame::_new_kittest();
    let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut f));
    out.textures_delta.clear();
    (out.platform_output.commands.into_iter())
        .filter_map(|c| match c {
            egui::OutputCommand::CopyImage(image) => Some(image),
            _ => None,
        })
        .collect()
}

/// Frames until a picture reaches the system's clipboard; the pictures copied.
fn wait_for_copy(app: &mut App, ctx: &egui::Context) -> Vec<egui::ColorImage> {
    for _ in 0..500 {
        let copied = images_copied(app, ctx, vec![]);
        if !copied.is_empty() {
            return copied;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("nothing was copied");
}

/// The pictures on page 1 of the tab in front: their ids and where they are.
fn pictures(app: &App) -> Vec<(usize, [f32; 4])> {
    let objs = app.tabs[app.active].pdf.page_objects(0).unwrap();
    (objs.iter())
        .filter(|o| o.kind == ObjectKind::Image)
        .map(|o| (o.id, o.bbox))
        .collect()
}

/// The note shown last, as a toast.
fn note(app: &App) -> String {
    app.toasts.last().map_or(String::new(), |t| t.text.clone())
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 0.5)
}

/// Ctrl+V with a picture on the clipboard and nothing typed: the picture goes in the middle
/// of the page in view, 96 pixels an inch, selected, in one step that Undo takes back; its
/// pixels as they were.
#[test]
fn a_picture_pasted_goes_in_the_middle_of_the_page_selected() {
    let png = render::encode(&halves(), ImageFormat::Png, 72.0).unwrap();
    let (ctx, mut app, _) = picture_tab(&png);
    let before = pictures(&app);
    images_copied(&mut app, &ctx, paste(teal(400, 200)));
    settle(&mut app, &ctx);
    let after = pictures(&app);
    assert_eq!(after.len(), before.len() + 1, "no picture added");
    let (id, bbox) = *after.last().unwrap();
    assert!(near(bbox, [156.0, 321.0, 456.0, 471.0]), "{bbox:?}");
    let t = &app.tabs[0];
    assert_eq!(t.ed.selection.object_ids(), Some((0, vec![id])));
    assert_eq!(t.history().1, 1, "not one step");
    let b = t.pdf.picture_pixels(0, id).unwrap();
    assert_eq!((b.width, b.height), (400, 200));
    assert!((b.rgba.chunks(4)).all(|c| c == [0, 128, 128, 255]));
    assert_eq!(app.status, "Pasted the picture.");
    app.act(&ctx, Act::Undo);
    assert_eq!(pictures(&app), before, "Undo left it");
}

/// A screenshot larger than the page is shrunk to fit inside half-inch margins.
#[test]
fn a_large_screenshot_is_shrunk_to_fit_the_page() {
    let png = render::encode(&halves(), ImageFormat::Png, 72.0).unwrap();
    let (ctx, mut app, _) = picture_tab(&png);
    images_copied(&mut app, &ctx, paste(teal(1920, 1080)));
    let (_, bbox) = *pictures(&app).last().unwrap();
    assert!(near(bbox, [36.0, 244.125, 576.0, 547.875]), "{bbox:?}");
}

/// While text is being typed the picture isn't pasted, and in Organise neither: a note
/// says what to do.
#[test]
fn a_picture_isnt_pasted_into_text_or_organise() {
    let png = render::encode(&halves(), ImageFormat::Png, 72.0).unwrap();
    let (ctx, mut app, _) = picture_tab(&png);
    let before = pictures(&app);
    app.act(&ctx, Act::Tool(Tool::Text));
    let (page, k) = page_on_screen(&mut app, &ctx);
    window(
        &mut app,
        &ctx,
        1400.0,
        click_at(page + egui::vec2(400.0, 60.0) * k),
    );
    assert!(app.tabs[0].ed.typing_id().is_some(), "not typing");
    images_copied(&mut app, &ctx, paste(teal(40, 20)));
    assert_eq!(pictures(&app), before, "pasted into the text");
    assert!(
        note(&app).starts_with("A picture can't go into text"),
        "{}",
        note(&app)
    );
    app.act(&ctx, Act::Escape);
    app.toasts.clear();
    app.act(&ctx, Act::Organise);
    settle(&mut app, &ctx);
    images_copied(&mut app, &ctx, paste(teal(40, 20)));
    assert_eq!(pictures(&app), before, "pasted in Organise");
    assert!(
        note(&app).contains("go back to the page view"),
        "{}",
        note(&app)
    );
}

/// The clipboard holds one thing: a picture pasted after pages were copied replaces them.
#[test]
fn a_picture_pasted_replaces_the_pages_copied() {
    let png = render::encode(&halves(), ImageFormat::Png, 72.0).unwrap();
    let (ctx, mut app, _) = picture_tab(&png);
    app.tabs[0].ed.selection = Selection::pages_from([0].into());
    copied_by(&mut app, &ctx, vec![egui::Event::Copy]);
    assert!(app.pages_to_paste(), "the pages weren't copied");
    app.tabs[0].ed.selection = Selection::None;
    images_copied(&mut app, &ctx, paste(teal(40, 20)));
    assert!(!app.pages_to_paste(), "the pages are still waiting");
    assert_eq!(app.tabs[0].page_count(), 1, "pages pasted");
    assert_eq!(pictures(&app).len(), 2, "the picture wasn't pasted");
}

/// The band's Paste can be used in the page view with nothing to paste in LimarePDF: it asks
/// the system for its clipboard, and says so when nothing it can paste comes.
#[test]
fn paste_asks_the_system_and_says_when_nothing_comes() {
    let png = render::encode(&halves(), ImageFormat::Png, 72.0).unwrap();
    let (ctx, mut app, _) = picture_tab(&png);
    let mut c = Ctx::default();
    app.tabs[0].fill_ctx(&mut c);
    assert!((actions::get(Act::Paste).enabled)(&c), "Paste is greyed");
    app.act(&ctx, Act::Paste);
    assert!(app.paste_asked.is_some(), "the system wasn't asked");
    images_copied(&mut app, &ctx, vec![]);
    images_copied(&mut app, &ctx, vec![]);
    assert!(app.paste_asked.is_none());
    assert_eq!(note(&app), "There's nothing on the clipboard to paste here");
    // A picture that comes is pasted, without the note.
    app.toasts.clear();
    app.act(&ctx, Act::Paste);
    images_copied(&mut app, &ctx, vec![]);
    images_copied(&mut app, &ctx, paste(teal(40, 20)));
    assert_eq!(note(&app), "");
    assert_eq!(pictures(&app).len(), 2, "the picture wasn't pasted");
}

/// Ctrl+C with a picture selected puts it on the system's clipboard as it shows, without
/// changing the document.
#[test]
fn copy_puts_the_selected_picture_on_the_clipboard() {
    let png = render::encode(&halves(), ImageFormat::Png, 72.0).unwrap();
    let (ctx, mut app, id) = picture_tab(&png);
    select(&mut app, &ctx, &[id]);
    let mut copied = images_copied(&mut app, &ctx, vec![egui::Event::Copy]);
    if copied.is_empty() {
        copied = wait_for_copy(&mut app, &ctx);
    }
    assert_eq!(copied.len(), 1);
    let image = &copied[0];
    assert_eq!(image.size, [4, 2]);
    let rgba: Vec<u8> = (image.pixels.iter())
        .flat_map(|c| c.to_srgba_unmultiplied())
        .collect();
    assert_eq!(rgba, halves().rgba);
    assert!(
        app.status.starts_with("Copied the picture"),
        "{}",
        app.status
    );
    let t = &app.tabs[0];
    assert!(!t.unsaved() && !t.can_undo(), "the document changed");
}

/// A picture still being drawn when something else is copied doesn't reach the clipboard.
#[test]
fn a_picture_copied_late_is_dropped() {
    let png = render::encode(&halves(), ImageFormat::Png, 72.0).unwrap();
    let (ctx, mut app, _) = picture_tab(&png);
    let late = app.picture_copies;
    app.picture_copies += 1;
    app.tx
        .send(Loaded::PictureCopied(late, Ok(halves())))
        .unwrap();
    assert!(
        images_copied(&mut app, &ctx, vec![]).is_empty(),
        "copied late"
    );
}

/// A tab of one Letter page with a line of text across its middle, the read tool in use.
fn line_tab() -> (egui::Context, App) {
    let (ctx, mut app) = super::paragraphs::new_app();
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    let line = "The walrus swims slowly in the cold grey northern sea";
    (pdf.add_text(
        0,
        (100.0, 396.0),
        line,
        FontChoice::default(),
        12.0,
        [0.0; 3],
    ))
    .unwrap();
    assert!(app.add_tab(&ctx, pdf, None, None, false));
    settle(&mut app, &ctx);
    (ctx, app)
}

/// Double-click page point `at` on the first page.
fn double_click_at(app: &mut App, ctx: &egui::Context, at: (f32, f32)) {
    let (min, k) = page_on_screen(app, ctx);
    let p = min + egui::vec2(at.0, at.1) * k;
    window(app, ctx, 1400.0, vec![egui::Event::PointerMoved(p)]);
    for _ in 0..2 {
        window(app, ctx, 1400.0, vec![press(p, true)]);
        window(app, ctx, 1400.0, vec![press(p, false)]);
    }
    settle(app, ctx);
}

/// A picture pasted over a line of text is in front of it, as Word puts a picture in front
/// of text: a double-click on it leaves it selected rather than opening the line it hides,
/// with the read tool and the arrow; the line's words beside it still open it.
#[test]
fn a_picture_in_front_of_text_takes_the_double_click() {
    let (ctx, mut app) = line_tab();
    images_copied(&mut app, &ctx, paste(teal(400, 200)));
    settle(&mut app, &ctx);
    let (id, bbox) = *pictures(&app).last().unwrap();
    assert!(near(bbox, [156.0, 321.0, 456.0, 471.0]), "{bbox:?}");
    let text = app.tabs[0].pdf.page_objects(0).unwrap()[0].bbox;
    let middle = (300.0, (text[1] + text[3]) / 2.0);
    for tool in [Tool::Read, Tool::Select] {
        app.act(&ctx, Act::Tool(tool));
        app.tabs[0].ed.selection = Selection::None;
        double_click_at(&mut app, &ctx, middle);
        let t = &app.tabs[0];
        assert_eq!(t.ed.typing_id(), None, "{tool:?}: the hidden line opened");
        assert_eq!(t.ed.selection.object_ids(), Some((0, vec![id])), "{tool:?}");
        // Pausing, so that the next click isn't a third.
        for _ in 0..40 {
            window(&mut app, &ctx, 1400.0, vec![]);
        }
    }
    app.act(&ctx, Act::Tool(Tool::Read));
    super::paragraphs::double_click_word(&mut app, &ctx, "The");
    assert!(
        app.tabs[0].ed.typing_id().is_some(),
        "the line beside it didn't open"
    );
}

/// A picture pasted over form fields says that their answers show above it, as in every
/// PDF reader; one clear of them says nothing.
#[test]
fn a_picture_over_form_fields_says_they_show_above_it() {
    let (ctx, mut app) = super::paragraphs::new_app();
    assert!(app.add_tab(&ctx, super::forms::mixed_form(), None, None, false));
    settle(&mut app, &ctx);
    images_copied(&mut app, &ctx, paste(teal(40, 20)));
    assert_eq!(pictures(&app).len(), 1, "not pasted");
    assert_eq!(note(&app), "", "clear of the fields");
    images_copied(&mut app, &ctx, paste(teal(1920, 1080)));
    assert_eq!(pictures(&app).len(), 2, "not pasted");
    assert_eq!(note(&app), crate::tab::OVER_FIELDS);
}
