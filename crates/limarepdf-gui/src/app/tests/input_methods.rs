//! Input methods in text typed on the page (the text design, 5.4; PB-54): what is being
//! composed shows unchecked, what is committed is checked as typing is, a composition left
//! when the box loses the keyboard is taken as committed and checked, an input method types
//! over words selected with the read tool, and its candidate window follows the caret.

use super::*;
use egui::accesskit::TreeUpdate;

/// A frame of the window 1400 points wide with `events`: its tree, and what it asks of the
/// input method (on, with its candidate window at a place, or off).
fn frame(
    app: &mut App,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> (TreeUpdate, Option<egui::output::IMEOutput>) {
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
    let tree = out
        .platform_output
        .accesskit_update
        .expect("the window's widgets");
    (tree, out.platform_output.ime)
}

fn preedit(text: &str) -> egui::Event {
    egui::Event::Ime(egui::ImeEvent::Preedit {
        text: text.into(),
        active_range_chars: None,
    })
}

fn commit(text: &str) -> egui::Event {
    egui::Event::Ime(egui::ImeEvent::Commit(text.into()))
}

/// What the line or text being typed on the page holds now, as screen readers read it.
fn typed(tree: &TreeUpdate) -> Option<String> {
    let named = |n: &egui::accesskit::Node| {
        n.label()
            .is_some_and(|l| l.starts_with("Editing text") || l == page_text::NAME)
    };
    let (_, n) = tree.nodes.iter().find(|(_, n)| named(n))?;
    n.value().map(str::to_string)
}

/// With words selected with the read tool, the input method is on, its candidate window at
/// the selection's end, where a word processor's caret is; its first composition opens
/// their line, in their place, selected as egui shows a composition, and the window follows
/// the caret as the composition grows; what is committed is typed there (PB-54's second
/// part).
#[test]
fn an_input_method_types_over_words_selected_with_the_read_tool() {
    let ctx = egui::Context::default();
    let mut app = two_pages_tab(&ctx);
    let seal = "The seal fish by the sea and the seal ate fish by the shore 1";
    // "otter ate".
    app.tabs[0].ed.selection = Selection::Words {
        anchor: (0, 17),
        end: (0, 18),
    };
    settled_at(&mut app, &ctx, 1400.0);
    let (tree, ime) = frame(&mut app, &ctx, vec![]);
    let ime = ime.expect("the input method is off");
    let (page, _) = node(&tree, "Page 1").expect("the page");
    let k = page.width() / 612.0;
    let ate = app.tabs[0].pdf.page_words(0).unwrap()[18].clone();
    assert_eq!(ate.text, "ate");
    let end = page.min.x + ate.bbox[2] * k;
    assert!(
        (ime.cursor_rect.min.x - end).abs() < 1.0,
        "{:?}, not at {end}",
        ime.cursor_rect
    );

    frame(&mut app, &ctx, vec![preedit("se")]);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let otter = "The otter ate fish by the sea and the seal ate fish by the shore 1";
    let name = format!("Editing text: {otter}");
    let (_, focused) = node(&tree, &name).expect("the line isn't open");
    assert!(focused, "the line hasn't the keyboard");
    let id = app.tabs[0].ed.typing_id().expect("typing");
    assert_eq!(
        page_text::selection(&ctx, id),
        Some((4, 6)),
        "the composition"
    );
    let (_, ime) = frame(&mut app, &ctx, vec![]);
    let at_se = ime.expect("the input method").cursor_rect.min.x;
    frame(&mut app, &ctx, vec![preedit("sea")]);
    let (_, ime) = frame(&mut app, &ctx, vec![]);
    let at_sea = ime.expect("the input method").cursor_rect.min.x;
    assert!(at_sea > at_se + 2.0, "{at_se} then {at_sea}");

    frame(&mut app, &ctx, vec![commit("seal")]);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(typed(&tree).as_deref(), Some(seal));
    assert_eq!(
        page_text::selection(&ctx, id),
        Some((8, 8)),
        "the caret after it"
    );
    frame(
        &mut app,
        &ctx,
        vec![key_with(Key::Escape, egui::Modifiers::NONE)],
    );
    settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(line_texts(&app.tabs[0].pdf, 0)[1], seal);
}

/// What an input method composes shows unchecked; a composition left when the line or new
/// text loses the keyboard, by a click away or another tool, is taken as committed, as
/// Windows and macOS input methods commit: what can't be written is left out of it, as it
/// is of typing, and the tab says so (PB-54's first part).
#[test]
fn a_composition_left_without_the_keyboard_is_taken_as_committed() {
    // U+2FFFE: a character no font has.
    let (composed, left) = ("a\u{2FFFE}b", "\u{2FFFE}");
    for case in [
        "line, click away",
        "line, another tool",
        "new text, another tool",
    ] {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        lines_tab(&mut app, &ctx, 400.0, &paragraph(72.0, 100.0, 3, 1, false));
        let empty = egui::vec2(300.0, 300.0);
        if case.starts_with("line") {
            edit_otter(&mut app, &ctx, 1400.0, case);
        } else {
            app.act(&ctx, Act::Tool(Tool::Text));
            let tree = settled_at(&mut app, &ctx, 1400.0);
            let (page, _) = node(&tree, "Page 1").expect("the page");
            let p = page.min + empty * (page.width() / 612.0);
            for events in [vec![egui::Event::PointerMoved(p)], vec![press(p, true)]] {
                window(&mut app, &ctx, 1400.0, events);
            }
            window(&mut app, &ctx, 1400.0, vec![press(p, false)]);
            // The new text takes the keyboard in the frames after.
            settled_at(&mut app, &ctx, 1400.0);
        }
        frame(&mut app, &ctx, vec![preedit(composed)]);
        let tree = settled_at(&mut app, &ctx, 1400.0);
        let shown = typed(&tree).unwrap_or_else(|| panic!("{case}: not typing"));
        assert!(shown.contains(composed), "{case}: {shown:?}");

        if case.ends_with("click away") {
            let (page, _) = node(&tree, "Page 1").expect("the page");
            let p = page.min + empty * (page.width() / 612.0);
            window(&mut app, &ctx, 1400.0, vec![egui::Event::PointerMoved(p)]);
            window(&mut app, &ctx, 1400.0, vec![press(p, true)]);
            window(&mut app, &ctx, 1400.0, vec![press(p, false)]);
        } else {
            app.act(&ctx, Act::Tool(Tool::Select));
        }
        settled_at(&mut app, &ctx, 1400.0);
        let t = &app.tabs[0];
        assert_eq!(t.ed.typing_id(), None, "{case}: still typing");
        let lines = line_texts(&t.pdf, 0);
        let want = match case.starts_with("line") {
            true => "The ab ate",
            false => "ab",
        };
        assert!(lines.iter().any(|l| l == want), "{case}: {lines:?}");
        assert!(!lines.iter().any(|l| l.contains(left)), "{case}: {lines:?}");
        assert_eq!(toast(&app), Some(page_text::left_note(left)), "{case}");
    }
}
