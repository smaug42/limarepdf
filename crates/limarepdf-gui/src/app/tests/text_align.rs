//! Text alignment (NEW-15, batch 49's stage 1): Edit's Alignment group, Ctrl+L, Ctrl+E,
//! Ctrl+R and Ctrl+J align the paragraph being edited, as Word's Paragraph group does: it
//! rewraps in its frame at once, its typing open; an alignment pressed again goes back to
//! left; its own Undo takes each change back, and it goes into the document with the typing
//! as one step.

use super::paragraphs::{
    click_band, content, new_app, open_prose_paragraph, paragraph_lines, paragraph_now, prose_tab,
};
use super::*;
use crate::actions;
use crate::editor::TextAlign;
use egui::accesskit::Toggled;

/// The pressed state of the band's button called `name`.
fn pressed(tree: &egui::accesskit::TreeUpdate, name: &str) -> Option<Toggled> {
    (tree.nodes.iter())
        .filter(|(_, n)| n.label() == Some(name))
        .find_map(|(_, n)| n.toggled())
}

/// How the paragraph being edited is aligned.
fn aligned(app: &App) -> Option<TextAlign> {
    app.tabs[0].ed.text_align()
}

/// Whether a paragraph is being edited.
fn open(app: &App) -> bool {
    app.tabs[0].ed.editing_paragraph()
}

/// `key` with Ctrl (⌘ on a Mac), then the window settled.
fn ctrl(app: &mut App, ctx: &egui::Context, key: Key) {
    window(app, ctx, 1400.0, vec![key_with(key, CTRL)]);
    settled_at(app, ctx, 1400.0);
}

/// Esc, then the window settled: the paragraph kept.
fn esc(app: &mut App, ctx: &egui::Context) {
    let none = egui::Modifiers::NONE;
    window(app, ctx, 1400.0, vec![key_with(Key::Escape, none)]);
    settled_at(app, ctx, 1400.0);
}

/// The band's Centre centres the paragraph being edited and is pressed, Align left no
/// longer; the editing stays open, the keyboard in the text. Kept with Esc: one step, every
/// line centred between the frame's edges, the words as they were; one Undo puts the page
/// back byte for byte.
#[test]
fn the_bands_centre_centres_the_paragraph_being_edited_kept_as_one_step() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    let was = content(&app.tabs[0].pdf);
    let text = paragraph_now(&app.tabs[0].pdf);
    let tree = open_prose_paragraph(&mut app, &ctx);
    let typing = app.tabs[0].ed.typing_id().expect("typing");
    assert_eq!(pressed(&tree, "Align left"), Some(Toggled::True));
    assert_eq!(pressed(&tree, "Centre"), Some(Toggled::False));
    let (left, right) = app.tabs[0].ed.para_edges().expect("editing a paragraph");
    click_band(&mut app, &ctx, "Centre");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(open(&app), "the click ended the editing");
    assert_eq!(aligned(&app), Some(TextAlign::Centre));
    assert_eq!(pressed(&tree, "Centre"), Some(Toggled::True));
    assert_eq!(pressed(&tree, "Align left"), Some(Toggled::False));
    let focused = ctx.memory(|m| m.focused());
    assert_eq!(focused, Some(typing), "the keyboard left the text");
    let steps = app.tabs[0].history().1;
    esc(&mut app, &ctx);
    assert!(!open(&app), "Esc keeps it");
    assert_eq!(app.tabs[0].history().1, steps + 1, "one step");
    assert_eq!(paragraph_now(&app.tabs[0].pdf), text, "the words changed");
    let middle = (left + right) / 2.0;
    for o in paragraph_lines(&app.tabs[0].pdf) {
        let at = (o.bbox[0] + o.bbox[2]) / 2.0;
        assert!((at - middle).abs() < 0.6, "{o:?} not centred on {middle}");
    }
    app.act(&ctx, Act::Undo);
    settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(content(&app.tabs[0].pdf), was, "one Undo, byte for byte");
}

/// Ctrl+E, Ctrl+R, Ctrl+J and Ctrl+L align the paragraph being edited, as Word's keys do,
/// one pressed again going back to left; Ctrl+Z in it takes each change back in turn, and
/// Ctrl+Y does it again. Justified and kept, its lines but the last reach the frame's right
/// edge.
#[test]
fn word_processor_keys_align_the_paragraph_and_its_own_undo_takes_each_back() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    open_prose_paragraph(&mut app, &ctx);
    assert_eq!(aligned(&app), Some(TextAlign::Left));
    let keys = [
        (Key::E, TextAlign::Centre),
        (Key::R, TextAlign::Right),
        (Key::J, TextAlign::Justified),
        (Key::J, TextAlign::Left),
        (Key::E, TextAlign::Centre),
        (Key::L, TextAlign::Left),
    ];
    for (key, want) in keys {
        ctrl(&mut app, &ctx, key);
        assert!(open(&app), "{key:?} ended the editing");
        assert_eq!(aligned(&app), Some(want), "{key:?}");
    }
    let back = [
        TextAlign::Centre,
        TextAlign::Left,
        TextAlign::Justified,
        TextAlign::Right,
        TextAlign::Centre,
        TextAlign::Left,
    ];
    for want in back {
        ctrl(&mut app, &ctx, Key::Z);
        assert_eq!(aligned(&app), Some(want), "Ctrl+Z");
    }
    for _ in 0..3 {
        ctrl(&mut app, &ctx, Key::Y);
    }
    assert_eq!(aligned(&app), Some(TextAlign::Justified), "Ctrl+Y");
    let (_, right) = app.tabs[0].ed.para_edges().expect("editing a paragraph");
    esc(&mut app, &ctx);
    let lines = paragraph_lines(&app.tabs[0].pdf);
    let last = lines.len() - 1;
    for o in &lines[..last] {
        assert!((o.bbox[2] - right).abs() < 0.6, "{o:?} short of {right}");
    }
    let short = lines[last].bbox[2] < right - 20.0;
    assert!(short, "the last line stretched");
}

/// With no paragraph being edited, the alignments are greyed, saying where to click, and
/// none is pressed; their keys change nothing.
#[test]
fn the_alignments_are_greyed_outside_a_paragraph_and_say_why() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    let was = content(&app.tabs[0].pdf);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let c = app.state(&ctx);
    let why = "Click in a paragraph's text first.";
    for how in [
        TextAlign::Left,
        TextAlign::Centre,
        TextAlign::Right,
        TextAlign::Justified,
    ] {
        let a = actions::get(Act::AlignText(how));
        assert!(!(a.enabled)(&c), "{how:?}");
        assert_eq!(actions::why_not(&c, a), why);
        assert_eq!(pressed(&tree, a.label), Some(Toggled::False), "{how:?}");
    }
    ctrl(&mut app, &ctx, Key::E);
    assert_eq!(content(&app.tabs[0].pdf), was);
    assert_eq!(app.tabs[0].history().1, 0);
}
