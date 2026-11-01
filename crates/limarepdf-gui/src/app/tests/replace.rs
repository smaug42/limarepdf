//! Find and replace (NEW-04a, batch 46's stage 2): Edit > Replace (Ctrl+H) opens Search
//! results with a Replace with field; Replace types it over the current match on the page,
//! the line waiting open and the next match current; Replace again keeps it, one undo step,
//! and replaces the next. Match case and Whole words only change what is found. What isn't
//! replaced says why.

use super::paragraphs::new_app;
use super::*;

/// A tab of one page with `lines` on it, 40 points apart, with the read tool; Replace open,
/// `query` searched for and `with` in Replace's field.
fn replacing(app: &mut App, ty: &Typist, lines: &[&str], query: &str, with: &str) {
    let at: Vec<(String, egui::Pos2)> = (lines.iter().enumerate())
        .map(|(i, l)| (l.to_string(), egui::pos2(72.0, 100.0 + 40.0 * i as f32)))
        .collect();
    lines_tab(app, ty.ctx, 792.0, &at);
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

/// Search results' Replace button: the lowest control called Replace, below the band's.
fn replace_button(tree: &egui::accesskit::TreeUpdate) -> egui::Pos2 {
    let button = egui::accesskit::Role::Button;
    let mut found: Vec<egui::Rect> = (tree.nodes.iter())
        .filter(|(_, n)| n.role() == button && n.label() == Some("Replace"))
        .filter_map(|(_, n)| n.bounds())
        .map(|b| {
            let at = |x: f64, y: f64| egui::pos2(x as f32, y as f32);
            egui::Rect::from_min_max(at(b.x0, b.y0), at(b.x1, b.y1))
        })
        .collect();
    found.sort_by(|a, b| a.min.y.total_cmp(&b.min.y));
    assert_eq!(found.len(), 2, "the band's Replace and Search results'");
    found[1].center()
}

/// Click Search results' Replace, then draw the frames a note it puts up takes to show (a
/// new egui Area is laid out unseen first). Clicked, not `app.act`, which outside a frame
/// reads the clock as 0, so that a note is already over.
fn click_replace(app: &mut App, ty: &Typist) -> egui::accesskit::TreeUpdate {
    let tree = ty.pause(app);
    ty.frame(app, click_at(replace_button(&tree)));
    ty.frame(app, vec![]);
    ty.frame(app, vec![])
}

/// What the line being edited on the page holds, as typed.
fn typed_in_line(tree: &egui::accesskit::TreeUpdate) -> Option<String> {
    let mut lines = (tree.nodes.iter())
        .filter(|(_, n)| n.label().is_some_and(|l| l.starts_with("Editing text: ")));
    let (_, line) = lines.next()?;
    line.value().map(str::to_string)
}

/// Replace types over the current match on the page and the next becomes current, the
/// keyboard back in Replace's field and the line waiting open; Replace again, by button
/// or Enter, keeps it as one undo step and replaces the next once the search is back.
#[test]
fn replace_types_over_each_match_in_turn() {
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    let lines = ["the walrus ate fish", "the walrus swam", "a walrus slept"];
    replacing(&mut app, &ty, &lines, "walrus", "otter");
    assert_eq!(app.tabs[0].hits.len(), 3);
    let tree = ty.pause(&mut app);
    ty.click(&mut app, replace_button(&tree));
    let tree = ty.pause(&mut app);
    let t = &app.tabs[0];
    assert!(t.ed.editing_line(), "the line isn't open");
    assert_eq!(typed_in_line(&tree).as_deref(), Some("the otter ate fish"));
    assert_eq!(line_texts(&t.pdf, 0), lines, "kept before Replace again");
    assert_eq!(t.hit_idx, 1, "the next match isn't current");
    assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Replace with"));

    // Replace again: the first kept, the second typed over.
    ty.click(&mut app, replace_button(&tree));
    searched_with(&mut app, |a| {
        ty.frame(a, vec![]);
    });
    let tree = ty.pause(&mut app);
    let t = &app.tabs[0];
    let now = line_texts(&t.pdf, 0);
    let first = ["the otter ate fish", "the walrus swam", "a walrus slept"];
    assert_eq!(now, first);
    assert_eq!(typed_in_line(&tree).as_deref(), Some("the otter swam"));
    let s = t.hits[t.hit_idx].snippet.to_string();
    assert!(s.ends_with("swam a walrus slept"), "{s}");

    // Enter in Replace's field does the same; then the last change is kept as typing is.
    assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Replace with"));
    ty.key(&mut app, Key::Enter, egui::Modifiers::NONE);
    searched_with(&mut app, |a| {
        ty.frame(a, vec![]);
    });
    ty.pause(&mut app);
    assert!(app.tabs[0].keep_typing(&ctx));
    let all = ["the otter ate fish", "the otter swam", "a otter slept"];
    assert_eq!(line_texts(&app.tabs[0].pdf, 0), all);
    // Each replacement is an undo step of its own.
    app.act(&ctx, Act::Undo);
    let now = line_texts(&app.tabs[0].pdf, 0);
    let two = ["the otter ate fish", "the otter swam", "a walrus slept"];
    assert_eq!(now, two);
    app.act(&ctx, Act::Undo);
    app.act(&ctx, Act::Undo);
    assert_eq!(line_texts(&app.tabs[0].pdf, 0), lines);
}

/// A match can be part of a word, and an empty Replace with deletes it.
#[test]
fn replace_changes_only_the_match() {
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    replacing(&mut app, &ty, &["the harbour was quiet"], "arbo", "ARBO");
    app.act(&ctx, Act::ReplaceMatch);
    ty.pause(&mut app);
    assert!(app.tabs[0].keep_typing(&ctx));
    assert_eq!(line_texts(&app.tabs[0].pdf, 0), ["the hARBOur was quiet"]);

    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    replacing(&mut app, &ty, &["the harbour was quiet"], "was ", "");
    app.act(&ctx, Act::ReplaceMatch);
    ty.pause(&mut app);
    assert!(app.tabs[0].keep_typing(&ctx));
    assert_eq!(line_texts(&app.tabs[0].pdf, 0), ["the harbour quiet"]);
}

/// Match case and Whole words only, in Search results, change what is found at once.
#[test]
fn match_case_and_whole_words_change_the_matches() {
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    let lines = ["Walrus walruses", "the walrus"];
    replacing(&mut app, &ty, &lines, "walrus", "");
    assert_eq!(app.tabs[0].hits.len(), 3);
    let tree = ty.pause(&mut app);
    ty.click(&mut app, middle(&tree, "Match case"));
    searched_with(&mut app, |a| {
        ty.frame(a, vec![]);
    });
    let found = |app: &App| -> Vec<String> {
        (app.tabs[0].hits.iter())
            .map(|h| h.snippet.found.clone())
            .collect()
    };
    assert_eq!(found(&app), ["walrus", "walrus"]);
    let tree = ty.pause(&mut app);
    ty.click(&mut app, middle(&tree, "Whole words only"));
    searched_with(&mut app, |a| {
        ty.frame(a, vec![]);
    });
    assert_eq!(found(&app), ["walrus"]);
    let s = app.tabs[0].hits[0].snippet.to_string();
    assert!(s.ends_with("walruses the walrus"), "{s}");
    let tree = ty.pause(&mut app);
    assert!(shows(&tree, "1 match"));
}

/// What isn't replaced says why, and nothing opens: a character LimarePDF can't write, at
/// Replace's field; a match running across paragraphs, at the match.
#[test]
fn what_is_not_replaced_says_why() {
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    replacing(&mut app, &ty, &["the walrus ate fish"], "walrus", "שלום");
    let tree = click_replace(&mut app, &ty);
    let t = &app.tabs[0];
    assert!(!t.ed.typing_on_page() && t.hit_idx == 0);
    assert!(shows(&tree, page_text::SCRIPT), "no note");

    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    let at = [
        ("the walrus".to_string(), egui::pos2(72.0, 100.0)),
        ("ate fish".to_string(), egui::pos2(72.0, 400.0)),
    ];
    lines_tab(&mut app, &ctx, 792.0, &at);
    ty.pause(&mut app);
    app.act(&ctx, Act::Replace);
    ty.frame(&mut app, vec![]);
    ty.frame(&mut app, vec![egui::Event::Text("walrus ate".into())]);
    ty.key(&mut app, Key::Enter, egui::Modifiers::NONE);
    searched_with(&mut app, |a| {
        ty.frame(a, vec![]);
    });
    app.tabs[0].replace.with = "otter".into();
    let tree = click_replace(&mut app, &ty);
    assert!(!app.tabs[0].ed.typing_on_page());
    let note = "Not replaced: the match runs across paragraphs";
    assert!(shows(&tree, note), "no note at the match");

    // Invisible recognised text, found by search, isn't replaced (batch 46's Q4).
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    subset_tab(&mut app, &ctx);
    ty.pause(&mut app);
    app.act(&ctx, Act::Replace);
    ty.frame(&mut app, vec![]);
    ty.frame(&mut app, vec![egui::Event::Text("hidden".into())]);
    ty.key(&mut app, Key::Enter, egui::Modifiers::NONE);
    searched_with(&mut app, |a| {
        ty.frame(a, vec![]);
    });
    assert_eq!(app.tabs[0].hits.len(), 1);
    app.tabs[0].replace.with = "shown".into();
    click_replace(&mut app, &ty);
    let tree = ty.pause(&mut app);
    assert!(!app.tabs[0].ed.typing_on_page());
    let note = "Not replaced: invisible recognised text";
    assert!(shows(&tree, note), "no note");
    assert!(!app.tabs[0].can_undo(), "something changed");
}

/// Ctrl+H, or the band's Replace, opens Search results with Replace's field, the keyboard
/// in the search field; Find shows them without it. Replace is greyed until there is a
/// match, saying why.
#[test]
fn ctrl_h_opens_replace_and_find_leaves_it_out() {
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    text_tab(&mut app, &ctx, &["the walrus"]);
    ty.pause(&mut app);
    let ctrl = egui::Modifiers::CTRL;
    let tree = ty.key(&mut app, Key::H, ctrl);
    let tree = match node(&tree, "Replace with") {
        Some(_) => tree,
        None => ty.pause(&mut app),
    };
    assert!(app.side.shown() && app.tabs[0].side == Side::Search);
    assert!(node(&tree, "Replace with").is_some(), "no Replace field");
    let tree = ty.pause(&mut app);
    assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Search for"));
    let c = app.state(&ctx);
    let a = actions::get(Act::ReplaceMatch);
    assert!(!(a.enabled)(&c));
    assert_eq!(actions::why_not(&c, a), "Find a match to replace first.");
    app.act(&ctx, Act::Find);
    let tree = ty.pause(&mut app);
    assert!(node(&tree, "Replace with").is_none(), "Find shows Replace");
    // The band's Replace is a command now, not a planned one.
    assert!(node(&tree, "Replace — Planned").is_none());
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let (at, _) = node(&tree, "Replace").expect("the band's Replace");
    ty.click(&mut app, at.center());
    let tree = ty.pause(&mut app);
    assert!(node(&tree, "Replace with").is_some(), "the band's Replace");
}
