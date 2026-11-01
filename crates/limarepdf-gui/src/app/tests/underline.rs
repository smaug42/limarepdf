//! Underline and Strikethrough (NEW-07, batch 46's stage 4b; the design note's tests 9 to 12):
//! the commands on words selected with the read tool, in a line being edited on the page, in
//! new text and on a line selected with the arrow, by Word's rule, each one undo step; the
//! Font group's buttons pressed when all of the text has the style; Replace and Replace all
//! keeping the underline of what they type over; the comment marks named apart from them.

use super::paragraphs::new_app;
use super::replace::review;
use super::*;
use egui::accesskit::Toggled;
use limarepdf_core::{DecoKind, Decorated, ObjectKind, PartFont, TextPart};

/// Each line of page 1 of `pdf`: its text, and what each of its decorations covers (spaces
/// at its ends left out) with its kind.
fn decorated(pdf: &Pdf) -> Vec<(String, Vec<(String, DecoKind)>)> {
    let objs = pdf.page_objects(0).unwrap();
    (objs.into_iter().filter(|o| o.kind == ObjectKind::Text))
        .map(|o| {
            let chars: Vec<char> = o.text.as_deref().unwrap_or("").chars().collect();
            let over = |d: &limarepdf_core::Decoration| {
                let s: String = chars[d.chars.0..d.chars.1].iter().collect();
                (s.trim().to_string(), d.kind)
            };
            let decos = o.decorations.iter().map(over).collect();
            (o.text.clone().unwrap_or_default(), decos)
        })
        .collect()
}

/// Select the words `from` to `to` of page 1 with the read tool, as dragging over them does.
fn words(app: &mut App, ctx: &egui::Context, from: &str, to: &str) {
    settled_at(app, ctx, 1400.0);
    let words = app.tabs[0].pdf.page_words(0).unwrap();
    let at = |s: &str| words.iter().position(|w| w.text == s).expect(s);
    app.tabs[0].ed.selection = Selection::Words {
        anchor: (0, at(from)),
        end: (0, at(to)),
    };
    settled_at(app, ctx, 1400.0);
}

/// The pressed state of the Font group's button called `name` (the floating bar's buttons of
/// the same name have none).
fn pressed(tree: &egui::accesskit::TreeUpdate, name: &str) -> Option<Toggled> {
    (tree.nodes.iter())
        .filter(|(_, n)| n.label() == Some(name))
        .find_map(|(_, n)| n.toggled())
}

const U: DecoKind = DecoKind::Underline;
const S: DecoKind = DecoKind::Strike;

fn two_lines(app: &mut App, ctx: &egui::Context) {
    let at = |y: f32| egui::pos2(72.0, y);
    let lines = [
        ("the walrus ate fish".to_string(), at(100.0)),
        ("a seal swam".to_string(), at(140.0)),
    ];
    lines_tab(app, ctx, 792.0, &lines);
}

/// Words selected with the read tool, on one line and across lines: Underline by Word's
/// rule (partly underlined becomes all underlined, all underlined becomes plain), the Font
/// group's Underline pressed only when all of them are; Strikethrough of its own; each one
/// undo step, the words themselves never rewritten.
#[test]
fn selected_words_take_underline_and_strikethrough_by_words_rule() {
    let (ctx, mut app) = new_app();
    two_lines(&mut app, &ctx);
    let texts = ["the walrus ate fish", "a seal swam"];
    words(&mut app, &ctx, "walrus", "fish");
    let off = Some(Toggled::False);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(pressed(&tree, "Underline"), off);
    app.act(&ctx, Act::Underline);
    let got = decorated(&app.tabs[0].pdf);
    assert_eq!(got[0].1, [("walrus ate fish".into(), U)]);
    assert!(got[1].1.is_empty());
    assert_eq!(app.tabs[0].history().1, 1, "not one step");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(pressed(&tree, "Underline"), Some(Toggled::True));
    assert_eq!(pressed(&tree, "Strikethrough"), Some(Toggled::False));

    // Across lines, partly underlined: all of it underlined, one line under each line's
    // words, the one read carried on.
    words(&mut app, &ctx, "the", "seal");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(pressed(&tree, "Underline"), off);
    app.act(&ctx, Act::Underline);
    let got = decorated(&app.tabs[0].pdf);
    assert_eq!(got[0].1, [("the walrus ate fish".into(), U)]);
    assert_eq!(got[1].1, [("a seal".into(), U)]);
    // All underlined: taken off.
    app.act(&ctx, Act::Underline);
    let got = decorated(&app.tabs[0].pdf);
    assert!(got.iter().all(|(_, d)| d.is_empty()), "{got:?}");

    // Strikethrough, a style of its own.
    words(&mut app, &ctx, "seal", "swam");
    app.act(&ctx, Act::Strikethrough);
    let got = decorated(&app.tabs[0].pdf);
    assert_eq!(got[1].1, [("seal swam".into(), S)]);
    let texts_now: Vec<String> = got.iter().map(|(t, _)| t.clone()).collect();
    assert_eq!(texts_now, texts);
    assert_eq!(app.tabs[0].history().1, 4);
    app.act(&ctx, Act::Undo);
    assert!(decorated(&app.tabs[0].pdf)[1].1.is_empty());
}

/// In a line being edited on the page, Ctrl+U underlines what is selected (and isn't egui's
/// "delete to the start of the line"), and with the caret between words, what is typed next;
/// kept as one change.
#[test]
fn ctrl_u_underlines_in_a_line_being_edited() {
    let (ctx, mut app) = new_app();
    two_lines(&mut app, &ctx);
    super::paragraphs::double_click_word(&mut app, &ctx, "walrus");
    assert!(app.tabs[0].ed.editing_line(), "the line isn't open");
    let ctrl = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
    window(&mut app, &ctx, 1400.0, vec![key_with(Key::U, ctrl)]);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(pressed(&tree, "Underline"), Some(Toggled::True));
    let end = key_with(Key::End, egui::Modifiers::NONE);
    window(&mut app, &ctx, 1400.0, vec![end]);
    window(&mut app, &ctx, 1400.0, vec![key_with(Key::U, ctrl)]);
    let typed = egui::Event::Text(" today".into());
    window(&mut app, &ctx, 1400.0, vec![typed]);
    settled_at(&mut app, &ctx, 1400.0);
    let esc = key_with(Key::Escape, egui::Modifiers::NONE);
    window(&mut app, &ctx, 1400.0, vec![esc]);
    settled_at(&mut app, &ctx, 1400.0);
    let got = decorated(&app.tabs[0].pdf);
    assert_eq!(got[0].0, "the walrus ate fish today");
    let want = [("walrus".to_string(), U), ("today".to_string(), U)];
    assert_eq!(got[0].1, want);
    assert_eq!(app.tabs[0].history().1, 1, "not one step");
}

/// New text typed underlined (Ctrl+U before typing) is written underlined at its font's
/// place; a line selected with the arrow is struck through whole.
#[test]
fn new_text_and_a_selected_line_take_the_styles() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = app_with(&ctx, 1);
    let (min, k) = page_on_screen(&mut app, &ctx);
    app.act(&ctx, Act::Tool(Tool::Text));
    let at = min + egui::vec2(100.0, 200.0) * k;
    window(&mut app, &ctx, 1400.0, click_at(at));
    settled_at(&mut app, &ctx, 1400.0);
    let ctrl = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
    window(&mut app, &ctx, 1400.0, vec![key_with(Key::U, ctrl)]);
    let typed = egui::Event::Text("Signed".into());
    window(&mut app, &ctx, 1400.0, vec![typed]);
    settled_at(&mut app, &ctx, 1400.0);
    let esc = key_with(Key::Escape, egui::Modifiers::NONE);
    window(&mut app, &ctx, 1400.0, vec![esc]);
    settled_at(&mut app, &ctx, 1400.0);
    let objs = app.tabs[0].pdf.page_objects(0).unwrap();
    let line = (objs.iter())
        .find(|o| o.text.as_deref() == Some("Signed"))
        .expect("the new text");
    assert_eq!(line.decorations.len(), 1, "{:?}", line.decorations);
    let d = &line.decorations[0];
    assert_eq!((d.kind, d.chars), (U, (0, 6)));
    let want = limarepdf_core::decoration_lines(FontChoice::default()).under;
    let (c, t) = d.look.bands()[0];
    let near = (c - want.0).abs() < 0.002 && (t - want.1).abs() < 0.002;
    assert!(near, "{c} {t}");
    // The next new text carries on underlined, as typing would.
    assert!(app.tabs[0].ed.tlines.underline);

    // The line, selected with the arrow: struck through whole.
    let id = line.id;
    app.act(&ctx, Act::Tool(Tool::Select));
    select(&mut app, &ctx, &[id]);
    app.act(&ctx, Act::Strikethrough);
    let objs = app.tabs[0].pdf.page_objects(0).unwrap();
    let line = (objs.iter())
        .find(|o| o.text.as_deref() == Some("Signed"))
        .expect("the line");
    let kinds: Vec<(DecoKind, (usize, usize))> =
        line.decorations.iter().map(|d| (d.kind, d.chars)).collect();
    assert_eq!(kinds, [(U, (0, 6)), (S, (0, 6))]);
}

/// A tab of one page with `text` in Helvetica 12 at (72, 100), `word` underlined at its
/// font's place, with the read tool.
fn underlined_tab(app: &mut App, ctx: &egui::Context, text: &str, word: &str) {
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    let part = TextPart {
        text: text.into(),
        font: PartFont::Choice(FontChoice::default()),
        size: 12.0,
        color: [0.0; 3],
        rise: 0.0,
        from: None,
        origins: Vec::new(),
    };
    let from = text[..text.find(word).unwrap()].chars().count();
    let d = Decorated {
        kind: U,
        chars: (from, from + word.chars().count()),
        look: None,
    };
    (pdf.add_text_decorated(0, (72.0, 100.0), &[part], &[d])).unwrap();
    assert!(app.add_tab(ctx, pdf, None, None, false));
    app.act(ctx, Act::Tool(Tool::Read));
}

/// A line's underline isn't selected apart from it: a click on the underline with the arrow
/// selects its line, and Select all on the page selects the line alone (the underline moves
/// and is deleted with it, in core).
#[test]
fn a_click_on_an_underline_selects_its_line() {
    let (ctx, mut app) = new_app();
    underlined_tab(&mut app, &ctx, "the walrus ate fish", "walrus");
    app.act(&ctx, Act::Tool(Tool::Select));
    settle(&mut app, &ctx);
    let objs = app.tabs[0].pdf.page_objects(0).unwrap();
    let line = objs.iter().position(|o| o.text.is_some()).unwrap();
    let band = (objs.iter())
        .find(|o| o.decorates == Some(line))
        .expect("the underline")
        .bbox;
    let mid = ((band[0] + band[2]) / 2.0, (band[1] + band[3]) / 2.0);
    click_page(&mut app, &ctx, mid, false);
    assert_eq!(held(&app), Some((0, vec![line])));
    app.act(&ctx, Act::SelectAllObjects);
    assert_eq!(held(&app), Some((0, vec![line])));
}

/// Ctrl+U in a box without text styles (Search's field here, as a comment's text or a form
/// field) does nothing, as in a word processor's plain fields, rather than egui's own
/// "delete to the start of the line" (PB-77).
#[test]
fn ctrl_u_in_a_box_without_styles_does_nothing() {
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    two_lines(&mut app, &ctx);
    ty.pause(&mut app);
    app.act(&ctx, Act::Find);
    ty.frame(&mut app, vec![]);
    ty.frame(&mut app, vec![egui::Event::Text("walrus ate".into())]);
    ty.pause(&mut app);
    assert_eq!(app.tabs[0].query, "walrus ate");
    let ctrl = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
    ty.key(&mut app, Key::U, ctrl);
    ty.pause(&mut app);
    assert_eq!(app.tabs[0].query, "walrus ate");
    let got = decorated(&app.tabs[0].pdf);
    assert!(got.iter().all(|(_, d)| d.is_empty()), "{got:?}");
}

/// Search for `query`, with `with` in Replace's field.
fn search(app: &mut App, ty: &Typist, query: &str, with: &str) {
    ty.pause(app);
    app.act(ty.ctx, Act::Replace);
    ty.frame(app, vec![]);
    ty.frame(app, vec![egui::Event::Text(query.into())]);
    ty.key(app, Key::Enter, egui::Modifiers::NONE);
    searched_with(app, |a| {
        ty.frame(a, vec![]);
    });
    app.tabs[0].replace.with = with.into();
    ty.pause(app);
}

/// Replace, and Replace all, type over an underlined word as typing over it does: the
/// replacement takes its underline.
#[test]
fn replace_keeps_the_underline_of_what_it_types_over() {
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    underlined_tab(&mut app, &ctx, "the walrus ate fish", "walrus");
    search(&mut app, &ty, "walrus", "otter");
    app.act(&ctx, Act::ReplaceMatch);
    ty.pause(&mut app);
    assert!(app.tabs[0].keep_typing(&ctx));
    let got = decorated(&app.tabs[0].pdf);
    assert_eq!(got[0].0, "the otter ate fish");
    assert_eq!(got[0].1, [("otter".into(), U)]);

    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    underlined_tab(&mut app, &ctx, "the walrus ate fish", "walrus");
    search(&mut app, &ty, "walrus", "sea otter");
    review(&mut app, &ty);
    click(&mut app, &ctx, "Replace all");
    let got = decorated(&app.tabs[0].pdf);
    assert_eq!(got[0].0, "the sea otter ate fish");
    assert_eq!(got[0].1, [("sea otter".into(), U)]);
}

/// The comment marks are named apart from the text styles; Underline and Strikethrough are
/// greyed as Bold is, say why, and have their keys: Ctrl+U, and Cmd+Shift+X on a Mac only.
#[test]
fn the_styles_and_the_comment_marks_are_named_apart() {
    use crate::markup::Mark;
    let label = |m| actions::get(Act::Mark(m)).label;
    assert_eq!(label(Mark::Underline), "Underline comment");
    assert_eq!(label(Mark::StrikeOut), "Strike-out comment");
    assert_eq!(label(Mark::Squiggly), "Squiggly underline");
    let open = Ctx {
        doc: true,
        ..Ctx::default()
    };
    let bold = actions::why_not(&open, actions::get(Act::Bold));
    for act in [Act::Underline, Act::Strikethrough] {
        let a = actions::get(act);
        assert!(!(a.enabled)(&open));
        assert_eq!(actions::why_not(&open, a), bold);
        assert!(act.uses_document());
    }
    let ctx = egui::Context::default();
    theme::install(&ctx);
    ctx.run_ui(Default::default(), |_| {})
        .textures_delta
        .clear();
    let tip = |act| tooltip(&ctx, actions::get(act));
    ctx.set_os(egui::os::OperatingSystem::Nix);
    let ctrl_u = ctx.format_shortcut(&actions::get(Act::Underline).keys[0]);
    assert_eq!(tip(Act::Underline), format!("Underline ({ctrl_u})"));
    assert_eq!(tip(Act::Strikethrough), "Strikethrough");
    ctx.set_os(egui::os::OperatingSystem::Mac);
    assert_eq!(tip(Act::Underline), "Underline (⌘U)");
    assert_eq!(tip(Act::Strikethrough), "Strikethrough (⇧⌘X)");
    // In the words' right-click menu, the line being edited's floating bar and the typing's
    // menu, beside Bold and Italic.
    let sel = Selection::Words {
        anchor: (0, 0),
        end: (0, 1),
    };
    let styles = [Act::Bold, Act::Italic, Act::Underline, Act::Strikethrough];
    let menu: Vec<Act> = (actions::context_items(&sel).iter())
        .flat_map(|g| g.iter().copied())
        .collect();
    for act in styles {
        assert!(menu.contains(&act), "{act:?}");
    }
    let editing = Ctx {
        doc: true,
        line_edit: true,
        object: true,
        obj_kind: Some(crate::selection::ObjKind::Text),
        obj_text: true,
        obj_lines: true,
        ..Ctx::default()
    };
    let bar: Vec<Act> = (actions::toolbar_items(&Selection::None, &editing).into_iter())
        .flatten()
        .map(|a| a.act)
        .collect();
    assert_eq!(bar[..4], styles);
}
