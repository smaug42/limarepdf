//! Find and replace (NEW-04a, batch 46's stage 2): Edit > Replace (Ctrl+H) opens Search
//! results with a Replace with field; Replace types it over the current match on the page,
//! the line waiting open and the next match current; Replace again keeps it, one undo step,
//! and replaces the next. Match case and Whole words only change what is found. What isn't
//! replaced says why. Replace all (NEW-04b, stage 3) replaces every match on a copy,
//! reviewed first, as one undo step.

use super::paragraphs::new_app;
use super::*;

/// A tab of one page with `lines` on it, 40 points apart, with the read tool; Replace open,
/// `query` searched for and `with` in Replace's field.
fn replacing(app: &mut App, ty: &Typist, lines: &[&str], query: &str, with: &str) {
    let at: Vec<(String, egui::Pos2)> = (lines.iter().enumerate())
        .map(|(i, l)| (l.to_string(), egui::pos2(72.0, 100.0 + 40.0 * i as f32)))
        .collect();
    replacing_at(app, ty, &at, query, with);
}

/// [`replacing`] with each line where given (its baseline's left end, in points).
fn replacing_at(app: &mut App, ty: &Typist, at: &[(String, egui::Pos2)], query: &str, with: &str) {
    lines_tab(app, ty.ctx, 792.0, at);
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

/// F3 and Shift+F3, and Ctrl+G and Ctrl+Shift+G, go to the next match and the one before,
/// round the ends, from Replace's field (replacing nothing) and from the page; with nothing
/// searched, F3 opens Find.
#[test]
fn f3_and_ctrl_g_go_through_the_matches_from_anywhere() {
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    let lines = ["the walrus ate fish", "the walrus swam", "a walrus slept"];
    replacing(&mut app, &ty, &lines, "walrus", "otter");
    app.tabs[0].replace.focus = true;
    let tree = ty.pause(&mut app);
    assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Replace with"));
    let none = egui::Modifiers::NONE;
    let shift = egui::Modifiers::SHIFT;
    let ctrl = egui::Modifiers::COMMAND;
    let mut steps = Vec::new();
    for (key, m) in [
        (Key::F3, none),
        (Key::F3, none),
        (Key::F3, none),
        (Key::F3, shift),
        (Key::G, ctrl),
        (Key::G, ctrl.plus(shift)),
    ] {
        ty.key(&mut app, key, m);
        steps.push(app.tabs[0].hit_idx);
    }
    assert_eq!(steps, [1, 2, 0, 2, 0, 2]);
    let tree = ty.pause(&mut app);
    let t = &app.tabs[0];
    assert!(!t.ed.typing_on_page() && !t.can_undo(), "replaced");
    assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Replace with"));
    // With nothing having the keyboard, as after a click on the page.
    ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    ty.key(&mut app, Key::F3, none);
    assert_eq!(app.tabs[0].hit_idx, 0);

    // Nothing searched: F3 opens Find, the keyboard in its field.
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    text_tab(&mut app, &ctx, &["the walrus"]);
    ty.pause(&mut app);
    ty.key(&mut app, Key::F3, none);
    let tree = ty.pause(&mut app);
    assert!(app.side.shown() && app.tabs[0].side == Side::Search);
    assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Search for"));
}

/// Alt+R with the keyboard in Search results replaces the current match, as Word's Find and
/// Replace window's Replace; on a Mac, as there, it doesn't. Menus show F3 for Find next,
/// on a Mac ⌘G.
#[test]
fn alt_r_replaces_from_search_results_but_on_a_mac() {
    use egui::os::OperatingSystem::{Mac, Nix};
    for os in [Nix, Mac] {
        let (ctx, mut app) = new_app();
        ctx.set_os(os);
        let ty = Typist::new(&ctx);
        replacing(&mut app, &ty, &["the walrus ate fish"], "walrus", "otter");
        let mac = os == Mac;
        ty.key(&mut app, Key::R, egui::Modifiers::ALT);
        ty.pause(&mut app);
        assert_eq!(app.tabs[0].ed.typing_on_page(), !mac, "{os:?}");
        let next = actions::get(Act::FindNext);
        let shown = actions::shown_key(&ctx, next).map(|k| ctx.format_shortcut(&k));
        let want = if mac { "⌘G" } else { "F3" };
        assert_eq!(shown.as_deref(), Some(want), "{os:?}");
    }
}

/// Wait for Replace all's preparation, off the window's thread, to come back.
fn prepared(app: &mut App, ctx: &egui::Context) {
    wait_for(app, ctx, |a| !a.busy);
}

/// Replace all, once the search is done, and its review once it is shown.
pub(super) fn review(app: &mut App, ty: &Typist) -> dlg::ReplaceReview {
    searched_with(app, |a| {
        ty.frame(a, vec![]);
    });
    app.act(ty.ctx, Act::ReplaceAll);
    assert!(app.busy, "not preparing");
    prepared(app, ty.ctx);
    match &app.dialog {
        Dialog::ConfirmReplaceAll(q) => q.review.clone(),
        _ => panic!("no review"),
    }
}

/// Replace all prepares every replacement on a copy and says what it does first, the
/// document as it was; Replace all then makes them one undo step, said with Undo.
#[test]
fn replace_all_replaces_every_match_as_one_step() {
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    let lines = [
        "the walrus ate fish",
        "the walrus swam",
        "a walrus met a walrus",
    ];
    replacing(&mut app, &ty, &lines, "walrus", "otter");
    assert_eq!(app.tabs[0].hits.len(), 4);
    let r = review(&mut app, &ty);
    assert_eq!((r.replaced, r.pages, r.over, r.other_font), (4, 1, 0, 0));
    assert!(r.left.is_empty() && !r.fits);
    let first = "4 matches of “walrus” on 1 page are replaced with “otter”.";
    assert_eq!(r.said(), [first, "Undo brings it all back."]);
    assert_eq!(
        line_texts(&app.tabs[0].pdf, 0),
        lines,
        "changed before the answer"
    );
    click(&mut app, &ctx, "Replace all");
    let t = &app.tabs[0];
    let all = [
        "the otter ate fish",
        "the otter swam",
        "a otter met a otter",
    ];
    assert_eq!(line_texts(&t.pdf, 0), all);
    assert_eq!(t.notice.as_deref(), Some("Replaced 4 matches"));
    assert_eq!(t.history().1, 1, "not one step");
    app.act(&ctx, Act::Undo);
    assert_eq!(line_texts(&app.tabs[0].pdf, 0), lines);

    // An empty replacement deletes each, a space with it, as deleting a word does.
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    replacing(&mut app, &ty, &["a walrus met a walrus"], "walrus", "");
    let r = review(&mut app, &ty);
    let first = "2 matches of “walrus” on 1 page are deleted.";
    assert_eq!(r.said()[0], first);
    click(&mut app, &ctx, "Replace all");
    assert_eq!(line_texts(&app.tabs[0].pdf, 0), ["a met a"]);
}

/// A replacement that would run past the page's edge is said in the review, with its page;
/// Replace where it fits leaves it as it was, and the notice says so. Cancel changes
/// nothing.
#[test]
fn replace_where_it_fits_leaves_what_runs_over() {
    let long = "a long-tusked otter of the far north";
    let at = [
        ("the walrus".to_string(), egui::pos2(72.0, 100.0)),
        ("a walrus".to_string(), egui::pos2(470.0, 200.0)),
    ];
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    replacing_at(&mut app, &ty, &at, "walrus", long);
    let r = review(&mut app, &ty);
    assert_eq!((r.replaced, r.over, r.fits), (2, 1, true));
    let over = "1 of them would run past the page's edge or over other text, on page 1. \
                Replace where it fits leaves those as they are.";
    assert_eq!(r.said()[1], over);
    click(&mut app, &ctx, "Replace where it fits");
    let t = &app.tabs[0];
    assert_eq!(
        line_texts(&t.pdf, 0),
        [format!("the {long}"), "a walrus".into()]
    );
    let said = "Replaced 1 match; 1 that didn't fit is left as it was";
    assert_eq!(t.notice.as_deref(), Some(said));

    // Replace all replaces it too; Cancel nothing.
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    replacing_at(&mut app, &ty, &at, "walrus", long);
    review(&mut app, &ty);
    click(&mut app, &ctx, "Cancel");
    assert_eq!(line_texts(&app.tabs[0].pdf, 0), ["the walrus", "a walrus"]);
    assert!(!app.tabs[0].can_undo(), "something changed");
    review(&mut app, &ty);
    click(&mut app, &ctx, "Replace all");
    let all = [format!("the {long}"), format!("a {long}")];
    assert_eq!(line_texts(&app.tabs[0].pdf, 0), all);
}

/// What Replace all can't replace is counted by why, the rest replaced; with nothing it
/// can replace, a message says why, and nothing changes.
#[test]
fn replace_all_says_what_it_leaves_and_why() {
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    subset_tab(&mut app, &ctx);
    ty.pause(&mut app);
    app.act(&ctx, Act::Replace);
    ty.frame(&mut app, vec![]);
    ty.frame(&mut app, vec![egui::Event::Text("hidden".into())]);
    ty.key(&mut app, Key::Enter, egui::Modifiers::NONE);
    app.tabs[0].replace.with = "shown".into();
    searched_with(&mut app, |a| {
        ty.frame(a, vec![]);
    });
    app.act(&ctx, Act::ReplaceAll);
    prepared(&mut app, &ctx);
    let (title, text) = message(&app);
    assert_eq!(title, "Replace all");
    let said = "No match of “hidden” can be replaced. Not replaced: 1 is invisible recognised \
                text.";
    assert_eq!(text, said);
    assert!(!app.tabs[0].can_undo(), "something changed");

    // A replacement LimarePDF can't write in a line's font is left there.
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    replacing(&mut app, &ty, &["the walrus", "a walrus"], "walrus", "שלום");
    app.act(&ctx, Act::ReplaceAll);
    prepared(&mut app, &ctx);
    let (_, text) = message(&app);
    let why = "Not replaced: 2 need a character LimarePDF can't write there.";
    assert!(text.ends_with(why), "{text}");
}

/// Alt+A with the keyboard in Search results is Replace all, as in Word's Find and Replace
/// window; on a Mac, as there, it isn't. Replace all waits for the search to finish.
#[test]
fn alt_a_replaces_all_from_search_results_but_on_a_mac() {
    use egui::os::OperatingSystem::{Mac, Nix};
    for os in [Nix, Mac] {
        let (ctx, mut app) = new_app();
        ctx.set_os(os);
        let ty = Typist::new(&ctx);
        replacing(&mut app, &ty, &["the walrus ate fish"], "walrus", "otter");
        ty.key(&mut app, Key::A, egui::Modifiers::ALT);
        prepared(&mut app, &ctx);
        let asked = matches!(app.dialog, Dialog::ConfirmReplaceAll(_));
        assert_eq!(asked, os != Mac, "{os:?}");
    }
    let (ctx, mut app) = new_app();
    let ty = Typist::new(&ctx);
    replacing(&mut app, &ty, &["the walrus ate fish"], "walrus", "otter");
    let mut c = app.state(&ctx);
    c.search_running = true;
    let a = actions::get(Act::ReplaceAll);
    assert_eq!(actions::why_not(&c, a), "Waits until the search is done.");
}
