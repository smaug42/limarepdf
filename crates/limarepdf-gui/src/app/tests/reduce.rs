//! Reduce file size (ENH-19's stage 2): File > Reduce file size… shows what each choice would
//! do, worked out in the background, and reduces as one undo step; a signed document can't.

use super::*;

/// An app with one document of two pages, each showing a photo-like picture of its own,
/// 1,200 × 800 pixels 4 inches wide (300 ppi).
fn photo_app(ctx: &egui::Context) -> App {
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let mut pdf = Pdf::new();
    for page in 0..2 {
        pdf.insert_blank(page, 612.0, 792.0).unwrap();
        let (w, h) = (1200, 800);
        let rgba = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                let b = (x / 4 * y / 4 + page * 50) % 200;
                [(x / 5) as u8, (y / 4) as u8, b as u8, 255]
            })
            .collect();
        let photo = limarepdf_core::render::Bitmap {
            width: w,
            height: h,
            rgba,
        };
        let png = limarepdf_core::render::ImageFormat::Png;
        let png = limarepdf_core::render::encode(&photo, png, 72.0).unwrap();
        pdf.add_image(page, [72.0, 200.0, 360.0, 392.0], &png)
            .unwrap();
    }
    assert!(app.add_tab(ctx, pdf, None, None, false));
    app
}

/// The widest picture's width in the active document, in pixels.
fn widest(app: &App) -> i64 {
    let doc = &app.tabs[app.active].pdf.doc;
    let widths = doc.objects.values().filter_map(|o| {
        let s = o.as_stream().ok()?;
        let image = s.dict.get(b"Subtype").ok()?.as_name().ok()? == b"Image";
        image.then(|| s.dict.get(b"Width").ok()?.as_i64().ok())?
    });
    widths.max().unwrap()
}

/// Draws the dialog until its estimate has come, and returns the last frame's tree.
fn settled(app: &mut App, ctx: &egui::Context) -> egui::accesskit::TreeUpdate {
    let size = egui::vec2(1000.0, 800.0);
    for _ in 0..500 {
        let tree = dialog_frame(app, ctx, size, 1.0, vec![]);
        match &app.dialog {
            Dialog::ReduceSize(_, s) if s.estimate.waiting() => {}
            _ => return tree,
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the estimate didn't come");
}

/// The dialog's state.
fn reducing(app: &mut App) -> &mut dlg::Reduce {
    match &mut app.dialog {
        Dialog::ReduceSize(_, s) => s,
        _ => panic!("Reduce file size isn't open"),
    }
}

#[test]
fn reduce_file_size_shows_what_each_choice_does_and_reduces_as_one_step() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = photo_app(&ctx);
    assert_eq!(widest(&app), 1200);
    app.act(&ctx, Act::ReduceSize);
    // Print, the first time; all pages, none being selected.
    let s = reducing(&mut app);
    assert_eq!(s.quality, limarepdf_core::PictureQuality::Print);
    assert_eq!(s.chosen(), [0, 1]);
    let tree = settled(&mut app, &ctx);
    for name in ["Reduce", "Cancel", "Print (220 ppi)", "Email (96 ppi)"] {
        assert!(node(&tree, name).is_some(), "{name}");
    }
    let s = reducing(&mut app);
    let now = *s.estimate.now.as_ref().unwrap().as_ref().unwrap();
    let print = match &s.estimate.shown {
        Some(Ok(shown)) => {
            assert_eq!(shown.report.pictures, 2);
            assert!(shown.parts.is_some(), "no preview");
            shown.after
        }
        _ => panic!("no estimate"),
    };
    assert!(print < now, "{print} of {now}");
    // Another choice is worked out again, smaller.
    s.quality = limarepdf_core::PictureQuality::Email;
    dialog_frame(&mut app, &ctx, egui::vec2(1000.0, 800.0), 1.0, vec![]);
    assert!(reducing(&mut app).estimate.waiting());
    settled(&mut app, &ctx);
    let email = match &reducing(&mut app).estimate.shown {
        Some(Ok(shown)) => shown.after,
        _ => panic!("no estimate"),
    };
    assert!(email < print, "{email} of {print}");
    // Nothing changed while it showed.
    assert_eq!(widest(&app), 1200);
    assert_eq!(app.tabs[app.active].history().1, 0);

    click(&mut app, &ctx, "Reduce");
    assert!(app.busy);
    wait_for(&mut app, &ctx, |a| !a.busy);
    let (title, text) = message(&app);
    assert_eq!(title, "Reduce file size");
    let about = crate::reduce_size::size_text(email);
    let said = format!("2 pictures were made smaller. The file will be about {about} when saved.");
    assert_eq!(text, said);
    assert_eq!(widest(&app), 384);
    assert_eq!(app.tabs[app.active].history().1, 1, "one undo step");
    assert_eq!(
        app.settings.reduce_quality,
        limarepdf_core::PictureQuality::Email
    );
    app.dialog = Dialog::None;
    app.act(&ctx, Act::Undo);
    assert_eq!(widest(&app), 1200);
    // Opened again: the quality last used.
    app.act(&ctx, Act::ReduceSize);
    assert_eq!(
        reducing(&mut app).quality,
        limarepdf_core::PictureQuality::Email
    );
    click(&mut app, &ctx, "Cancel");
    assert!(matches!(app.dialog, Dialog::None));
    assert_eq!(widest(&app), 1200);
}

#[test]
fn a_signed_document_cant_be_reduced_and_says_why() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = photo_app(&ctx);
    app.tabs[app.active].signed = true;
    let c = app.state(&ctx);
    let a = actions::get(Act::ReduceSize);
    assert!(!(a.enabled)(&c));
    assert_eq!(actions::why_not(&c, a), actions::SIGNED_KEEPS);
    app.act(&ctx, Act::ReduceSize);
    assert!(matches!(app.dialog, Dialog::None));
    // High fidelity keeps the pictures, so Reduce does nothing to do.
    app.tabs[app.active].signed = false;
    app.settings.reduce_quality = limarepdf_core::PictureQuality::HighFidelity;
    app.act(&ctx, Act::ReduceSize);
    let tree = settled(&mut app, &ctx);
    let reduce = tree.nodes.iter().find(|(_, n)| n.label() == Some("Reduce"));
    assert!(reduce.unwrap().1.is_disabled());
}
