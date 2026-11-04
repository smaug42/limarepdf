//! Saving pictures (NEW-09's stage 2): a selected picture's Save image as… (its right-click
//! menu, its floating bar, Edit in ☰ and the command search) writes it through the save
//! chooser, a JPEG upright on the page as it is and any other picture a PNG; File > Export >
//! Save all images… writes every picture to the folder chosen, asking before it replaces
//! files. Neither changes the document.

use super::*;
use limarepdf_core::render::{self, Bitmap, ImageFormat};
use limarepdf_core::ObjectKind;

/// Where the photo is on the page (points, y down).
const PHOTO: [f32; 4] = [100.0, 100.0, 300.0, 250.0];

/// A 4 × 2 picture, red on the left and blue on the right.
pub(super) fn halves() -> Bitmap {
    let rgba = (0..8)
        .flat_map(|i| match i % 4 < 2 {
            true => [200, 20, 20, 255],
            false => [20, 20, 200, 255],
        })
        .collect();
    Bitmap {
        width: 4,
        height: 2,
        rgba,
    }
}

/// A tab of one page showing `picture` (a PNG or JPEG file's bytes) at [`PHOTO`] under a
/// caption, the arrow in use; the picture's id on the page.
pub(super) fn picture_tab(picture: &[u8]) -> (egui::Context, App, usize) {
    let (ctx, mut app) = super::paragraphs::new_app();
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    pdf.add_image(0, PHOTO, picture).unwrap();
    pdf.add_text(
        0,
        (100.0, 280.0),
        "A walrus",
        FontChoice::default(),
        12.0,
        [0.0; 3],
    )
    .unwrap();
    let objs = pdf.page_objects(0).unwrap();
    let id = (objs.iter())
        .position(|o| o.kind == ObjectKind::Image)
        .unwrap();
    assert!(app.add_tab(&ctx, pdf, None, None, false));
    app.act(&ctx, Act::Tool(Tool::Select));
    settle(&mut app, &ctx);
    (ctx, app, id)
}

/// Wait for the job running to finish; the message it ends with.
fn job_message(app: &mut App, ctx: &egui::Context) -> String {
    wait_for(app, ctx, |a| !a.busy);
    match &app.dialog {
        Dialog::Message(_, m) => m.clone(),
        _ => panic!("no message"),
    }
}

/// The document wasn't changed.
fn unchanged(app: &App) {
    let t = &app.tabs[app.active];
    assert!(!t.unsaved() && !t.can_undo(), "the document changed");
}

#[test]
fn save_image_is_in_the_menus_the_bar_and_the_command_search() {
    let a = actions::get(Act::SaveImage);
    assert_eq!(a.label, "Save image as…");
    assert!(a.palette && !a.icon.is_empty());
    assert_eq!(
        menu::path(Act::SaveImage).unwrap(),
        ["Edit", "Save image as…"]
    );
    let all = actions::get(Act::SaveAllImages);
    assert_eq!(all.label, "Save all images…");
    assert_eq!(
        menu::path(Act::SaveAllImages).unwrap(),
        ["Export", "Save all images…"]
    );
    assert!(Act::SaveImage.uses_document() && Act::SaveAllImages.uses_document());

    let png = render::encode(&halves(), ImageFormat::Png, 72.0).unwrap();
    let (ctx, mut app, id) = picture_tab(&png);
    // Nothing selected: greyed, saying why; Save all images works on the document.
    let c = app.state(&ctx);
    assert!(!(a.enabled)(&c));
    assert_eq!(actions::why_not(&c, a), "Select a picture first.");
    assert!((all.enabled)(&c));

    select(&mut app, &ctx, &[id]);
    let c = app.state(&ctx);
    assert!((a.enabled)(&c));
    // Beside Replace image in its right-click menu and on its floating bar.
    let menu = actions::context_items(&app.tabs[0].ed.selection);
    assert!(menu[0].contains(&Act::ReplaceImage) && menu[0].contains(&Act::SaveImage));
    let bar = actions::toolbar_items(&app.tabs[0].ed.selection, &c);
    assert!(
        bar.iter().flatten().any(|a| a.act == Act::SaveImage),
        "not on the bar"
    );
    let (min, k) = page_on_screen(&mut app, &ctx);
    let p = min + egui::vec2(200.0, 175.0) * k;
    let button = |pressed| egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: Default::default(),
    };
    let right = vec![egui::Event::PointerMoved(p), button(true), button(false)];
    window(&mut app, &ctx, 1400.0, right);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let item =
        |n: &egui::accesskit::Node| n.label().is_some_and(|l| l.starts_with("Save image as…"));
    assert!(
        tree.nodes.iter().any(|(_, n)| item(n)),
        "not in the right-click menu"
    );

    let found = |q| (actions::ACTIONS.iter()).any(|a| a.palette && actions::matches(a, q));
    assert!(found("save image") && found("save all images") && found("export images"));
}

/// A lossless picture is saved as a PNG of its own pixels, the chooser's name given its
/// extension; the document isn't changed.
#[test]
fn a_lossless_picture_is_saved_as_a_png() {
    let png = render::encode(&halves(), ImageFormat::Png, 72.0).unwrap();
    let (ctx, mut app, id) = picture_tab(&png);
    select(&mut app, &ctx, &[id]);
    let dir = test_dir("save-image-png");
    SAVE_AS.with(|s| s.set(Some(dir.join("walrus"))));
    app.act(&ctx, Act::SaveImage);
    let said = job_message(&mut app, &ctx);
    let file = dir.join("walrus.png");
    assert!(said.contains("walrus.png"), "{said}");
    let (w, h, rgba) =
        limarepdf_core::picture_preview(&std::fs::read(&file).unwrap(), 100).unwrap();
    assert_eq!((w, h, rgba), (4, 2, halves().rgba));
    unchanged(&app);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// An upright JPEG is saved as it is; nothing is written when the chooser is cancelled.
#[test]
fn an_upright_jpeg_is_saved_as_it_is() {
    let jpeg = render::encode(&halves(), ImageFormat::Jpeg(90), 72.0).unwrap();
    let (ctx, mut app, id) = picture_tab(&jpeg);
    select(&mut app, &ctx, &[id]);
    app.act(&ctx, Act::SaveImage);
    assert!(
        !app.busy && matches!(app.dialog, Dialog::None),
        "cancelled, yet it ran"
    );
    let dir = test_dir("save-image-jpeg");
    SAVE_AS.with(|s| s.set(Some(dir.join("walrus.jpg"))));
    app.act(&ctx, Act::SaveImage);
    job_message(&mut app, &ctx);
    assert_eq!(std::fs::read(dir.join("walrus.jpg")).unwrap(), jpeg);
    unchanged(&app);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Every picture to the folder chosen, named by page and number; again, it asks before
/// replacing them. A document without pictures says so without asking for a folder.
#[test]
fn save_all_images_writes_every_picture_to_a_folder() {
    let png = render::encode(&halves(), ImageFormat::Png, 72.0).unwrap();
    let (ctx, mut app, _) = picture_tab(&png);
    app.tabs[0].label = Some("walrus".into());
    let dir = test_dir("save-all-images");
    FOLDER.with(|f| f.set(Some(dir.clone())));
    app.act(&ctx, Act::SaveAllImages);
    let said = job_message(&mut app, &ctx);
    assert!(said.starts_with("Saved 1 picture to "), "{said}");
    let file = dir.join("walrus-page-001-image-1.png");
    assert!(
        file.exists(),
        "{:?}",
        std::fs::read_dir(&dir).unwrap().collect::<Vec<_>>()
    );
    unchanged(&app);

    std::fs::write(&file, b"mine").unwrap();
    app.dialog = Dialog::None;
    FOLDER.with(|f| f.set(Some(dir.clone())));
    app.act(&ctx, Act::SaveAllImages);
    assert!(
        matches!(app.dialog, Dialog::ConfirmReplace(..)),
        "it didn't ask"
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"mine");
    click(&mut app, &ctx, "Replace");
    job_message(&mut app, &ctx);
    assert_ne!(std::fs::read(&file).unwrap(), b"mine");
    std::fs::remove_dir_all(&dir).unwrap();

    app.dialog = Dialog::None;
    pages_tab(&mut app, &ctx, 2);
    FOLDER.with(|f| f.set(Some(dir.clone())));
    app.act(&ctx, Act::SaveAllImages);
    let none = matches!(&app.dialog, Dialog::Message(_, m) if m == "This document has no pictures to save.");
    assert!(none, "it didn't say so");
    assert!(FOLDER.with(|f| f.take()).is_some(), "it asked for a folder");
}
