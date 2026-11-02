//! Tools as modes, as word processors and Acrobat have them (the user's findings on NEW-15's
//! stage 3): a tool's button turns it on, and clicked again turns it off, back to the read
//! tool, what was typed kept; Edit text shows pressed only while page text is edited or the
//! Text tool was chosen with it, not because Insert > Text is on; and the Text tool fills in
//! a form field it clicks, as the read tool does, rather than putting new text over it.

use super::paragraphs::{click_band, new_app};
use super::*;
use egui::accesskit::Toggled;

/// The pressed state of the band's button called `name`.
fn pressed(tree: &egui::accesskit::TreeUpdate, name: &str) -> Option<Toggled> {
    (tree.nodes.iter())
        .filter(|(_, n)| n.label() == Some(name))
        .find_map(|(_, n)| n.toggled())
}

/// `text` typed, then the window settled.
fn type_text(app: &mut App, ctx: &egui::Context, text: &str) {
    window(app, ctx, 1400.0, vec![egui::Event::Text(text.into())]);
    settled_at(app, ctx, 1400.0);
}

/// Whether the active tab's first page draws `text` as a line of its own.
fn on_page(app: &App, text: &str) -> bool {
    let objs = app.tabs[app.active].pdf.page_objects(0).unwrap();
    objs.iter().any(|o| o.text.as_deref() == Some(text))
}

/// Insert > Text turns the Text tool on and is pressed; clicked again, the read tool comes
/// back. With new text typed, clicking Text again keeps the text, as choosing another tool
/// does. Every other tool's command does the same: chosen again, it gives way to the read
/// tool; the read tool chosen again stays.
#[test]
fn a_tool_chosen_again_goes_back_to_the_read_tool() {
    let (ctx, _) = new_app();
    let mut app = app_with(&ctx, 1);
    app.band = Band::Insert;
    click_band(&mut app, &ctx, "Text");
    assert_eq!(app.tabs[0].ed.tool, Tool::Text);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(pressed(&tree, "Text"), Some(Toggled::True));
    click_band(&mut app, &ctx, "Text");
    assert_eq!(app.tabs[0].ed.tool, Tool::Read, "Text clicked again");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(pressed(&tree, "Text"), Some(Toggled::False));

    click_band(&mut app, &ctx, "Text");
    click_page(&mut app, &ctx, (100.0, 300.0), false);
    assert!(app.tabs[0].ed.adding.is_some(), "no new text");
    type_text(&mut app, &ctx, "Seal");
    click_band(&mut app, &ctx, "Text");
    settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(app.tabs[0].ed.tool, Tool::Read, "Text clicked while typing");
    assert!(on_page(&app, "Seal"), "the new text wasn't kept");

    for t in [
        Tool::Select,
        Tool::Note,
        Tool::TextBox,
        Tool::Pen,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Line,
        Tool::Stamp,
        Tool::Redact,
        Tool::Sign,
    ] {
        app.act(&ctx, Act::Tool(t));
        assert_eq!(app.tabs[0].ed.tool, t);
        app.act(&ctx, Act::Tool(t));
        assert_eq!(app.tabs[0].ed.tool, Tool::Read, "{t:?} chosen again");
    }
    app.act(&ctx, Act::Tool(Tool::Read));
    app.act(&ctx, Act::Tool(Tool::Read));
    assert_eq!(app.tabs[0].ed.tool, Tool::Read);
}

/// A picture waiting to be put down: Image chosen again lets it go and brings back the read
/// tool, without asking for another picture.
#[test]
fn image_chosen_again_lets_the_picture_go() {
    let (ctx, _) = new_app();
    let mut app = app_with(&ctx, 1);
    app.tabs[0].arm_image(turned_photo());
    assert_eq!(app.tabs[0].ed.tool, Tool::Image);
    app.act(&ctx, Act::Tool(Tool::Image));
    assert_eq!(app.tabs[0].ed.tool, Tool::Read);
    let waiting = app.tabs[0].ed.pending_image.is_some();
    assert!(!waiting, "the picture is still waiting");
}

/// Insert > Text on its own leaves Edit text unpressed: Edit text is for page text being
/// edited. Edit text clicked then turns the Text tool on as its own (both pressed), and
/// clicked again turns it off; Insert > Text clicked turns it off too. A line being edited
/// shows Edit text pressed, whichever tool opened it.
#[test]
fn edit_text_is_pressed_for_text_being_edited_not_for_insert_text() {
    let (ctx, _) = new_app();
    let mut app = app_with(&ctx, 1);
    app.band = Band::Insert;
    click_band(&mut app, &ctx, "Text");
    app.band = Band::Edit;
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let edit_text = |tree| pressed(tree, "Edit text");
    assert_eq!(edit_text(&tree), Some(Toggled::False), "Insert > Text");

    click_band(&mut app, &ctx, "Edit text");
    assert_eq!(app.tabs[0].ed.tool, Tool::Text);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(edit_text(&tree), Some(Toggled::True), "Edit text");
    click_band(&mut app, &ctx, "Edit text");
    assert_eq!(app.tabs[0].ed.tool, Tool::Read, "Edit text clicked again");

    click_band(&mut app, &ctx, "Edit text");
    app.band = Band::Insert;
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(pressed(&tree, "Text"), Some(Toggled::True), "the Text tool");
    click_band(&mut app, &ctx, "Text");
    let tool = app.tabs[0].ed.tool;
    assert_eq!(tool, Tool::Read, "Insert > Text after Edit text");

    // A line of page text clicked with Insert > Text's tool: being edited, Edit text shows.
    let (at, black) = ((72.0, 200.0), [0.0; 3]);
    let walrus = |p: &mut Pdf| p.add_text(0, at, "Walrus", Default::default(), 12.0, black);
    assert!(app.tabs[0].edit(&ctx, walrus));
    app.band = Band::Insert;
    click_band(&mut app, &ctx, "Text");
    let w = app.tabs[0].pdf.page_words(0).unwrap()[0].bbox;
    click_page(&mut app, &ctx, middle(w), false);
    assert!(app.tabs[0].ed.editing_line(), "the line isn't being edited");
    app.band = Band::Edit;
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(edit_text(&tree), Some(Toggled::True), "a line edited");
}

/// The middle of the box `b`.
fn middle(b: [f32; 4]) -> (f32, f32) {
    ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0)
}

/// The text of the window's node whose text starts with `s`, and its top on screen.
fn line_at(tree: &egui::accesskit::TreeUpdate, s: &str) -> Option<(String, f64)> {
    tree.nodes.iter().find_map(|(_, n)| {
        let v = n.value().filter(|v| v.starts_with(s))?;
        Some((v.to_string(), n.bounds()?.y0))
    })
}

/// A greyed command's tooltip says why on a line of its own, marked as not available now,
/// between its name and what it does, so the reason isn't lost among them: Justify, hovered
/// while new text is typed.
#[test]
fn a_greyed_commands_tooltip_says_why_on_its_own_line() {
    let (ctx, _) = new_app();
    let ty = Typist::new(&ctx);
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::Tool(Tool::Text));
    let (min, k) = ty.page(&mut app);
    ty.click(&mut app, min + egui::vec2(100.0, 300.0) * k);
    ty.text(&mut app, "Seal");
    let tree = ty.pause(&mut app);
    let justify = (tree.nodes.iter())
        .filter(|(_, n)| n.label() == Some("Justify"))
        .find_map(|(_, n)| n.bounds())
        .expect("no Justify");
    let (x, y) = (justify.x0 + justify.x1, justify.y0 + justify.y1);
    let at = egui::pos2(x as f32 / 2.0, y as f32 / 2.0);
    ty.frame(&mut app, vec![egui::Event::PointerMoved(at)]);
    let tree = ty.pause(&mut app);
    let why = "Not available now. New text's lines end at Enter: there's nothing to justify.";
    let (name, top) = line_at(&tree, "Justify (").expect("no tooltip");
    assert_eq!(name, "Justify (Ctrl+J)");
    let (reason, middle) = line_at(&tree, "Not available now.").expect("no reason");
    assert_eq!(reason, why);
    let (_, bottom) = line_at(&tree, "Spread the paragraph's lines").expect("no hint");
    assert!(top < middle && middle < bottom, "{top} {middle} {bottom}");
}

/// The Text tool's click on a form field fills it in, as the read tool's does: the field
/// takes the typing and no new text is started over it. A click away lets go of the field
/// and starts new text there; that new text is kept when a field is clicked, and the field
/// takes the keyboard.
#[test]
fn the_text_tool_fills_in_a_field_it_clicks() {
    let (ctx, mut app) = new_app();
    let dir = test_dir("text-tool-field");
    let path = dir.join("form.pdf");
    form_file(&path, "");
    open_and_wait(&mut app, &ctx, &path, 1);
    let field = middle(app.tabs[0].pdf.form_fields()[0].widgets[0].bbox);
    let adding = |app: &App| app.tabs[0].ed.adding.is_some();
    let in_name = |app: &App| app.tabs[0].ed.form.typing_in() == Some(("Name".into(), 0));
    app.act(&ctx, Act::Tool(Tool::Text));
    click_page(&mut app, &ctx, field, false);
    type_text(&mut app, &ctx, "Ann");
    assert!(!adding(&app), "new text over the field");
    assert!(in_name(&app), "the field isn't being typed in");
    assert_eq!(app.tabs[0].ed.tool, Tool::Text);

    click_page(&mut app, &ctx, (100.0, 300.0), false);
    assert!(!in_name(&app), "still in the field");
    assert!(adding(&app), "no new text away from the fields");
    type_text(&mut app, &ctx, "Seal");
    click_page(&mut app, &ctx, field, false);
    type_text(&mut app, &ctx, "!");
    settled_at(&mut app, &ctx, 1400.0);
    assert!(on_page(&app, "Seal"), "the new text wasn't kept");
    assert!(!adding(&app), "the new text is still open");
    assert!(in_name(&app), "the field didn't take the keyboard");
    std::fs::remove_dir_all(&dir).unwrap();
}
