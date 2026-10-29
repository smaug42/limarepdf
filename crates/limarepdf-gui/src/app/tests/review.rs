//! Batch 42 (ENH-40): review that connects comments to the page. The Comments view's
//! cards, its filters and search, the keyboard route, Previous and Next comment, and replies
//! kept per comment as drafts, with app.rs's helpers.

use super::*;
use egui::accesskit::{Node, Role, Toggled, TreeUpdate};
use limarepdf_core::{Annot, AnnotKind};

fn test_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    ctx
}

/// A tab of one page with two notes by "Tester": "Walrus one" at (100, 100) and "Walrus
/// two" at (100, 300), page points; the window settled. Their ids.
fn two_notes(ctx: &egui::Context) -> (App, (u32, u16), (u32, u16)) {
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    let yellow = [1.0, 1.0, 0.0];
    let one = pdf
        .add_note(0, (100.0, 100.0), yellow, "Walrus one", "Tester")
        .unwrap();
    let two = pdf
        .add_note(0, (100.0, 300.0), yellow, "Walrus two", "Tester")
        .unwrap();
    assert!(app.add_tab(ctx, pdf, None, None, false));
    settle(&mut app, ctx);
    (app, one, two)
}

/// Select comment `id` on page 1, as a click on it does, and let the window settle.
fn select(app: &mut App, ctx: &egui::Context, id: (u32, u16)) {
    app.tabs[app.active].ed.selection = Selection::Annot { page: 0, id };
    settle(app, ctx);
}

/// What the inspector's reply box holds.
fn reply_box(tree: &egui::accesskit::TreeUpdate) -> Option<String> {
    use egui::accesskit::Role;
    tree.nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Reply") && n.role() == Role::MultilineTextInput)
        .map(|(_, n)| n.value().unwrap_or_default().to_string())
}

/// Q1: a reply typed and left for another comment stays with its comment, as Word and
/// Google Docs keep a draft: back in the reply box when that comment is selected again,
/// and not shown for the other one.
#[test]
fn a_reply_typed_stays_with_its_comment_when_another_is_selected() {
    let ctx = test_ctx();
    let (mut app, one, two) = two_notes(&ctx);
    select(&mut app, &ctx, one);
    click_named(&mut app, &ctx, "Reply");
    window(
        &mut app,
        &ctx,
        1400.0,
        vec![egui::Event::Text("Walrus draft".into())],
    );
    let tree = settle(&mut app, &ctx);
    assert_eq!(reply_box(&tree).as_deref(), Some("Walrus draft"));
    // Another comment: its own reply box, empty.
    select(&mut app, &ctx, two);
    let tree = settle(&mut app, &ctx);
    assert_eq!(
        reply_box(&tree).as_deref(),
        Some(""),
        "the draft shows for the other"
    );
    // Back: the draft is there again, and nothing was added to the document.
    select(&mut app, &ctx, one);
    let tree = settle(&mut app, &ctx);
    assert_eq!(
        reply_box(&tree).as_deref(),
        Some("Walrus draft"),
        "the draft was lost"
    );
    assert_eq!(app.tabs[0].pdf.annotations().len(), 2, "a reply was added");
    assert!(app.tabs[0].unsaved(), "the draft isn't unsaved work");
}

// ------------------------------------------------ helpers for the Comments view

/// The comments' colour in these tests.
const YELLOW: [f32; 3] = [1.0, 1.0, 0.0];
/// What a card's name ends with when its comment has a draft reply.
const DRAFT: &str = " · Draft reply";
/// The kinds of comment these tests make, as their cards name them.
const KINDS: [&str; 4] = ["Note", "Highlight", "Underline", "Reply"];

/// A document of `n` blank US Letter pages.
fn blank(n: usize) -> Pdf {
    let mut pdf = Pdf::new();
    for i in 0..n {
        pdf.insert_blank(i, 612.0, 792.0).unwrap();
    }
    pdf
}

/// A note by `author` saying `text`, at (100, `y`) on page `page` (0-based); its id.
fn note(pdf: &mut Pdf, page: usize, y: f32, text: &str, author: &str) -> (u32, u16) {
    pdf.add_note(page, (100.0, y), YELLOW, text, author)
        .unwrap()
}

/// A highlight by `author` over `area` (page points, y down) on page `page`, with no
/// comment; its id.
fn highlight(pdf: &mut Pdf, page: usize, area: [f32; 4], author: &str) -> (u32, u16) {
    let kind = AnnotKind::Highlight;
    pdf.add_markup(page, &kind, &[area], YELLOW, author)
        .unwrap()
}

/// The box around the words `from` to `to` of page 1's text, for a markup over them.
fn words_box(pdf: &Pdf, from: &str, to: &str) -> [f32; 4] {
    let words = pdf.page_words(0).unwrap();
    let bbox = |text: &str| words.iter().find(|w| w.text == text).expect(text).bbox;
    let (a, b) = (bbox(from), bbox(to));
    [a[0], a[1].min(b[1]), b[2], a[3].max(b[3])]
}

/// A page with the line "Otters ate fish" on it, at (72, 100).
fn otters() -> Pdf {
    let mut pdf = blank(1);
    let font = FontChoice::default();
    (pdf.add_text(0, (72.0, 100.0), "Otters ate fish", font, 20.0, [0.0; 3])).unwrap();
    pdf
}

/// An app with one tab of `pdf`, as if just opened, showing the Comments view; the window
/// settled.
fn comments_tab(ty: &Typist, pdf: Pdf) -> App {
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    assert!(app.add_tab(ty.ctx, pdf, None, None, false));
    app.show_view(Side::Comments);
    ty.pause(&mut app);
    app
}

/// The typist's clock caught up with egui's, after frames drawn without it.
fn sync(ty: &Typist) {
    ty.time.set(ty.ctx.input(|i| i.time));
}

/// Frames with no input for 0.4 s at the typist's pace: enough for the list to scroll, and
/// for the next click not to make a double-click with the last.
fn calm(app: &mut App, ty: &Typist) -> TreeUpdate {
    for _ in 0..7 {
        ty.frame(app, vec![]);
    }
    ty.frame(app, vec![])
}

/// A click at `p`, then calm.
fn tap(app: &mut App, ty: &Typist, p: egui::Pos2) -> TreeUpdate {
    ty.frame(app, click_at(p));
    calm(app, ty)
}

/// A right-click at `p`, then calm.
fn right_click(app: &mut App, ty: &Typist, p: egui::Pos2) -> TreeUpdate {
    let button = |pressed| egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: Default::default(),
    };
    ty.frame(app, vec![egui::Event::PointerMoved(p)]);
    ty.frame(app, vec![button(true)]);
    ty.frame(app, vec![button(false)]);
    calm(app, ty)
}

/// Press `key` alone, then calm.
fn press_key(app: &mut App, ty: &Typist, key: Key) -> TreeUpdate {
    ty.frame(app, vec![key_with(key, egui::Modifiers::NONE)]);
    calm(app, ty)
}

/// Type `text` where the keyboard is, then calm.
fn type_in(app: &mut App, ty: &Typist, text: &str) -> TreeUpdate {
    ty.frame(app, vec![egui::Event::Text(text.into())]);
    calm(app, ty)
}

/// Click the control named `name`, the biggest of that name.
fn click_on(app: &mut App, ty: &Typist, name: &str) -> TreeUpdate {
    let tree = calm(app, ty);
    tap(app, ty, middle(&tree, name))
}

fn rect(b: egui::accesskit::Rect) -> egui::Rect {
    let (min, max) = ((b.x0 as f32, b.y0 as f32), (b.x1 as f32, b.y1 as f32));
    egui::Rect::from_min_max(min.into(), max.into())
}

/// The control named `name` with role `role`.
fn with_role<'a>(tree: &'a TreeUpdate, name: &str, role: Role) -> Option<&'a Node> {
    let mut all = tree.nodes.iter();
    let (_, n) = all.find(|(_, n)| n.label() == Some(name) && n.role() == role)?;
    Some(n)
}

/// Whether the control named `name` with role `role` is ticked or chosen; `None` when it
/// isn't drawn.
fn ticked(tree: &TreeUpdate, name: &str, role: Role) -> Option<bool> {
    with_role(tree, name, role).map(|n| n.toggled() == Some(Toggled::True))
}

/// The middle of the control named `name` nearest `p`: a menu's item, not the band's of
/// the same name.
fn nearest(tree: &TreeUpdate, name: &str, p: egui::Pos2) -> Option<egui::Pos2> {
    (tree.nodes.iter())
        .filter(|(_, n)| n.label() == Some(name))
        .filter_map(|(_, n)| n.bounds().map(|b| rect(b).center()))
        .min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)))
}

/// Whether a label in the window reads `text`, all of it.
fn reads(tree: &TreeUpdate, text: &str) -> bool {
    (tree.nodes.iter()).any(|(_, n)| n.value() == Some(text))
}

/// How many controls are named `name`.
fn named(tree: &TreeUpdate, name: &str) -> usize {
    (tree.nodes.iter())
        .filter(|(_, n)| n.label() == Some(name))
        .count()
}

/// A card of the Comments view, as drawn.
struct Card {
    name: String,
    rect: egui::Rect,
    selected: bool,
    /// A stop of its own for Tab.
    stop: bool,
}

impl Card {
    /// What it says of its comment: the words after its kind, author and date, without the
    /// mention of a draft; for a card with no words, its kind.
    fn says(&self) -> &str {
        let name = self.name.strip_suffix(DRAFT).unwrap_or(&self.name);
        match name.split_once(": ") {
            Some((_, words)) => words,
            None => name.split(" · ").next().unwrap_or(name),
        }
    }

    fn drafted(&self) -> bool {
        self.name.ends_with(DRAFT)
    }

    /// A point inside it, in its first line, away from the Reply button under its words.
    fn spot(&self) -> egui::Pos2 {
        let r = self.rect;
        egui::pos2(
            r.left() + r.width() / 4.0,
            r.top() + r.height().min(20.0) / 2.0,
        )
    }
}

/// Whether `name` is a card's: a kind, then its author, date or words.
fn is_card(name: &str) -> bool {
    KINDS.iter().any(|k| {
        let rest = name.strip_prefix(k);
        rest.is_some_and(|r| r.starts_with(" · ") || r.starts_with(": "))
    })
}

/// The Comments view's cards drawn in `tree`, top to bottom.
fn cards(tree: &TreeUpdate) -> Vec<Card> {
    let focus = egui::accesskit::Action::Focus;
    let mut out: Vec<Card> = (tree.nodes.iter())
        .filter(|(_, n)| n.role() == Role::Button)
        .filter_map(|(_, n)| {
            let name = n.label().filter(|l| is_card(l))?;
            Some(Card {
                name: name.to_string(),
                rect: rect(n.bounds()?),
                selected: n.toggled() == Some(Toggled::True) || n.is_selected() == Some(true),
                stop: n.supports_action(focus),
            })
        })
        .collect();
    out.sort_by(|a, b| a.rect.top().total_cmp(&b.rect.top()));
    out
}

/// What the cards drawn say, top to bottom (see [`Card::says`]).
fn listed(tree: &TreeUpdate) -> Vec<String> {
    cards(tree).iter().map(|c| c.says().to_string()).collect()
}

/// The card drawn that says `says`.
fn card(tree: &TreeUpdate, says: &str) -> Option<Card> {
    cards(tree).into_iter().find(|c| c.says() == says)
}

/// Click the card that says `says`, as people click.
fn click_card(app: &mut App, ty: &Typist, says: &str) -> TreeUpdate {
    let tree = calm(app, ty);
    let c = card(&tree, says);
    let c = c.unwrap_or_else(|| panic!("no card says {says:?}: {:?}", listed(&tree)));
    tap(app, ty, c.spot())
}

/// Whether `v` is the count above the cards.
fn is_count(v: &str) -> bool {
    let counted = v.ends_with(" comment") || v.ends_with(" comments");
    counted && v.starts_with(|c: char| c.is_ascii_digit())
}

/// The count above the cards ("5 comments", "2 of 5 comments"), when shown.
fn count(tree: &TreeUpdate) -> Option<String> {
    (tree.nodes.iter())
        .filter_map(|(_, n)| n.value())
        .find(|v| is_count(v))
        .map(str::to_string)
}

/// Whether the Comments view's Filter menu is open.
fn filter_open(tree: &TreeUpdate) -> bool {
    with_role(tree, "All pages", Role::RadioButton).is_some()
}

/// Open the Filter menu, when it isn't; the window then.
fn open_filter(app: &mut App, ty: &Typist) -> TreeUpdate {
    let tree = calm(app, ty);
    if filter_open(&tree) {
        return tree;
    }
    let tree = tap(app, ty, middle(&tree, "Filter"));
    assert!(filter_open(&tree), "the Filter menu didn't open");
    tree
}

/// Click `name` (of `role`) in the Filter menu, opening it first when it isn't open; the
/// window then.
fn tick(app: &mut App, ty: &Typist, name: &str, role: Role) -> TreeUpdate {
    let tree = open_filter(app, ty);
    let n = with_role(&tree, name, role);
    let n = n.unwrap_or_else(|| panic!("no {name} in the Filter menu"));
    let p = rect(n.bounds().expect("its place")).center();
    tap(app, ty, p)
}

/// Close the Filter menu by its button, when it is open.
fn close_filter(app: &mut App, ty: &Typist) {
    let tree = calm(app, ty);
    if filter_open(&tree) {
        let tree = tap(app, ty, middle(&tree, "Filter"));
        assert!(!filter_open(&tree), "the Filter menu stayed open");
    }
}

/// Type `words` in the Comments view's search box, after a click in it; the window once
/// the typing has paused.
fn search_for(app: &mut App, ty: &Typist, words: &str) -> TreeUpdate {
    click_on(app, ty, "Search comments");
    ty.frame(app, vec![egui::Event::Text(words.into())]);
    ty.pause(app)
}

/// Select comment `id` on page `page` of the tab in front and type `text` in its reply box,
/// after a click in it, as people do.
fn draft_on(app: &mut App, ty: &Typist, page: usize, id: (u32, u16), text: &str) {
    app.tabs[app.active].ed.selection = Selection::Annot { page, id };
    let tree = calm(app, ty);
    let (r, _) = node(&tree, "Reply").expect("no reply box");
    tap(app, ty, r.center());
    let tree = type_in(app, ty, text);
    let typed = reply_box(&tree);
    assert_eq!(typed.as_deref(), Some(text), "not typed in the reply box");
}

/// The comment selected in the tab in front, if any.
fn chosen(app: &App) -> Option<(u32, u16)> {
    app.tabs[app.active].ed.selection.annot().map(|(_, id)| id)
}

/// The steps Undo can take back in the tab in front.
fn steps(app: &App) -> usize {
    app.tabs[app.active].history().1
}

/// Run `act` as the commands do, then calm; what the tab said, if anything.
fn run(app: &mut App, ty: &Typist, act: Act) -> Option<String> {
    app.act(ty.ctx, act);
    let said = app.tabs[app.active].info.clone();
    calm(app, ty);
    said
}

/// The replies in `pdf`, each as the comment it answers and its own words, sorted.
fn replies(pdf: &Pdf) -> Vec<(String, String)> {
    let all = pdf.annotations();
    let said = |id: (u32, u16)| all.iter().find(|a| a.id == id).map(|a| a.contents.clone());
    let mut out: Vec<(String, String)> = (all.iter())
        .filter_map(|a| Some((said(a.reply_to?).unwrap_or_default(), a.contents.clone())))
        .collect();
    out.sort();
    out
}

/// `pairs` as [`replies`] gives them.
fn pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    let pair = |(a, b): &(&str, &str)| (a.to_string(), b.to_string());
    pairs.iter().map(pair).collect()
}

/// Bring the tab called `name` to the front by a click on it, unsaved or not.
fn to_tab(app: &mut App, ty: &Typist, name: &str) {
    let tree = calm(app, ty);
    let unsaved = format!("{name}, unsaved");
    let area = |n: &Node| n.bounds().map_or(0.0, |b| b.width() * b.height());
    let (_, n) = (tree.nodes.iter())
        .filter(|(_, n)| n.label().is_some_and(|l| l == name || l == unsaved))
        .max_by(|(_, a), (_, b)| area(a).total_cmp(&area(b)))
        .unwrap_or_else(|| panic!("no tab {name}"));
    let p = rect(n.bounds().expect("its place")).center();
    tap(app, ty, p);
}

// ------------------------------------------------ the cards

/// Cards are headed by their page: its label when the document has page labels and they
/// are shown (the default), else its number. A card is named by its comment's kind, author
/// and date, then its words; an empty part is left out. The count says how many there are.
#[test]
fn cards_name_their_page_by_its_label_and_say_kind_author_and_comment() {
    use limarepdf_core::{LabelRange, LabelStyle};
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut pdf = blank(2);
    let roman = LabelRange {
        page: 0,
        style: Some(LabelStyle::LowerRoman),
        prefix: String::new(),
        first: 1,
    };
    pdf.set_label_ranges(&[roman]).unwrap();
    note(&mut pdf, 1, 100.0, "Walrus on two", "Tester");
    note(&mut pdf, 1, 300.0, "Walrus unsigned", "");
    let mut app = comments_tab(&ty, pdf);
    assert!(app.tabs[0].use_labels, "labels aren't shown by default");
    let tree = calm(&mut app, &ty);
    assert!(reads(&tree, "Page ii"), "no heading Page ii");
    assert!(
        !reads(&tree, "Page i"),
        "a heading for a page with no comments"
    );
    assert_eq!(count(&tree).as_deref(), Some("2 comments"));
    let cs = cards(&tree);
    assert_eq!(cs.len(), 2, "{:?}", listed(&tree));
    let first = &cs[0].name;
    let date =
        (first.strip_prefix("Note · Tester · ")).and_then(|r| r.strip_suffix(": Walrus on two"));
    let dated = date.is_some_and(|d| d.starts_with(|c: char| c.is_ascii_digit()));
    assert!(dated, "{first}");
    // No author: the kind and the date alone before the words.
    let second = &cs[1].name;
    let parts: Vec<&str> = second.split(" · ").collect();
    assert_eq!(parts.len(), 2, "{second}");
    assert_eq!(parts[0], "Note", "{second}");
    assert!(parts[1].ends_with(": Walrus unsigned"), "{second}");
    // Labels not shown: the page's number.
    app.act(&ctx, Act::TogglePageLabels);
    let tree = calm(&mut app, &ty);
    assert!(reads(&tree, "Page 2"), "no heading Page 2 with labels off");
    assert!(!reads(&tree, "Page ii"), "still called by its label");
}

/// A document with no comments says so, with no cards, count or filter.
#[test]
fn a_document_without_comments_says_so() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = comments_tab(&ty, blank(1));
    let tree = calm(&mut app, &ty);
    let none = "This document has no comments or markup.";
    assert!(reads(&tree, none), "it doesn't say there are none");
    assert!(cards(&tree).is_empty(), "{:?}", listed(&tree));
    assert_eq!(count(&tree), None);
    assert!(
        node(&tree, "Filter").is_none(),
        "a filter with nothing to filter"
    );
}

/// A text markup's card with no comment says the words it marks, in curly quotes; with a
/// comment, it says the comment, from its first line with words on it.
#[test]
fn a_markup_card_without_a_comment_says_the_words_it_marks() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut pdf = otters();
    let ate_fish = words_box(&pdf, "ate", "fish");
    highlight(&mut pdf, 0, ate_fish, "Tester");
    let kind = AnnotKind::Underline;
    let otters = words_box(&pdf, "Otters", "Otters");
    let under = pdf
        .add_markup(0, &kind, &[otters], YELLOW, "Tester")
        .unwrap();
    let comment = "\nWalrus first line\nWalrus second line";
    (pdf.set_annotation(under, Some(YELLOW), 1.0, comment)).unwrap();
    let mut app = comments_tab(&ty, pdf);
    let tree = calm(&mut app, &ty);
    let cs = cards(&tree);
    let marked = cs.iter().find(|c| c.name.starts_with("Highlight · "));
    let marked = marked.unwrap_or_else(|| panic!("no Highlight card: {:?}", listed(&tree)));
    assert!(
        marked.name.starts_with("Highlight · Tester · "),
        "{}",
        marked.name
    );
    assert_eq!(marked.says(), "“ate fish”", "{}", marked.name);
    let under = cs.iter().find(|c| c.name.starts_with("Underline · "));
    let under = under.unwrap_or_else(|| panic!("no Underline card: {:?}", listed(&tree)));
    assert!(
        under.says().starts_with("Walrus first line"),
        "{}",
        under.name
    );
}

/// Both ways: a click on a card selects its comment and shows its page, the card selected;
/// a mark selected on the page (by the app, or by a click on it) selects its card, the list
/// scrolling to it. Only the cards in view are drawn: the last of a long list isn't, until
/// it is scrolled to.
#[test]
fn a_card_and_its_mark_select_each_other() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut pdf = blank(2);
    let first = note(&mut pdf, 0, 100.0, "Walrus first", "Tester");
    for i in 0..60 {
        let at = (400.0, 150.0 + i as f32 * 10.0);
        let text = format!("Filler {i}");
        pdf.add_note(0, at, YELLOW, &text, "Tester").unwrap();
    }
    let last = note(&mut pdf, 1, 300.0, "Walrus last", "Tester");
    let mut app = comments_tab(&ty, pdf);
    let tree = calm(&mut app, &ty);
    assert!(
        card(&tree, "Walrus first").is_some(),
        "the first card isn't drawn"
    );
    assert!(
        card(&tree, "Walrus last").is_none(),
        "a card out of view is drawn"
    );
    // Clicked, while page 2 is read: its comment selected, its page shown.
    app.tabs[0].go_to(1);
    calm(&mut app, &ty);
    let tree = click_card(&mut app, &ty, "Walrus first");
    assert_eq!(app.tabs[0].ed.selection.annot(), Some((0, first)));
    assert_eq!(app.tabs[0].current, 0, "its page isn't shown");
    let selected = card(&tree, "Walrus first").is_some_and(|c| c.selected);
    assert!(selected, "its card isn't selected");
    // Selected on the page by the app: its card selected and scrolled to.
    app.tabs[0].ed.selection = Selection::Annot { page: 1, id: last };
    let tree = calm(&mut app, &ty);
    let c = card(&tree, "Walrus last").expect("the list didn't scroll to the card");
    assert!(c.selected, "the card isn't selected");
    assert!(
        c.rect.bottom() <= TEST_HEIGHT,
        "the card is below the window: {:?}",
        c.rect
    );
    let other = card(&tree, "Walrus first");
    assert!(
        other.is_none_or(|c| !c.selected),
        "the first is still selected"
    );
    // Clicked on the page: the first again, its card scrolled back to.
    app.tabs[0].go_to(0);
    calm(&mut app, &ty);
    click_page(&mut app, &ctx, (100.0, 100.0), false);
    sync(&ty);
    let sel = app.tabs[0].ed.selection.annot();
    assert_eq!(sel, Some((0, first)), "the click didn't select it");
    let tree = calm(&mut app, &ty);
    let c = card(&tree, "Walrus first").expect("the list didn't scroll back");
    assert!(c.selected, "the card isn't selected");
}

/// Only the cards in view are drawn: with 3,000 notes over three pages the window holds a
/// screenful of cards, not all of them; the count says them all, and the last card is
/// drawn once its comment is selected.
#[test]
fn only_the_cards_in_view_are_drawn() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut pdf = blank(3);
    let mut last = None;
    for i in 0..3000 {
        let at = (60.0 + (i % 8) as f32 * 60.0, 50.0 + (i % 1000) as f32 * 0.7);
        let text = format!("Walrus {i}");
        last = Some(pdf.add_note(i / 1000, at, YELLOW, &text, "Tester").unwrap());
    }
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    assert!(app.add_tab(&ctx, pdf, None, None, false));
    app.show_view(Side::Comments);
    calm(&mut app, &ty);
    let tree = calm(&mut app, &ty);
    let drawn = cards(&tree).len();
    assert!((1..100).contains(&drawn), "{drawn} cards drawn");
    let all = count(&tree).unwrap_or_default();
    let counted = ["3000 comments", "3,000 comments"].contains(&all.as_str());
    assert!(counted, "{all:?}");
    let id = last.unwrap();
    app.tabs[0].ed.selection = Selection::Annot { page: 2, id };
    let tree = calm(&mut app, &ty);
    let c = card(&tree, "Walrus 2999").expect("the last card isn't drawn once selected");
    assert!(c.selected, "its card isn't selected");
    let drawn = cards(&tree).len();
    assert!(drawn < 100, "{drawn} cards drawn");
}

// ------------------------------------------------ the filter and the search

/// The Filter menu shows comments by type, author and page: a checkbox per kind in the
/// document and per author ("(No name)" for none), all ticked at first, and All pages or
/// This page (the page being read). The count says how many of all are shown. Clear
/// filters shows every type, author and page again, the search staying.
#[test]
fn the_filter_shows_comments_by_type_author_and_page() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut pdf = blank(2);
    note(&mut pdf, 0, 100.0, "Walrus one", "Tester");
    highlight(&mut pdf, 0, [100.0, 200.0, 300.0, 220.0], "Tester");
    note(&mut pdf, 0, 300.0, "Walrus two", "Other");
    note(&mut pdf, 1, 100.0, "Walrus three", "");
    highlight(&mut pdf, 1, [100.0, 200.0, 300.0, 220.0], "Other");
    let mut app = comments_tab(&ty, pdf);
    let tree = calm(&mut app, &ty);
    assert_eq!(count(&tree).as_deref(), Some("5 comments"));
    let all = [
        "Walrus one",
        "Highlight",
        "Walrus two",
        "Walrus three",
        "Highlight",
    ];
    assert_eq!(listed(&tree), all);
    let tree = open_filter(&mut app, &ty);
    for heading in ["Type", "Author", "Page"] {
        assert!(reads(&tree, heading), "no {heading} heading");
    }
    for name in ["Note", "Highlight", "Tester", "Other", "(No name)"] {
        assert_eq!(ticked(&tree, name, Role::CheckBox), Some(true), "{name}");
    }
    assert_eq!(ticked(&tree, "All pages", Role::RadioButton), Some(true));
    assert_eq!(ticked(&tree, "This page", Role::RadioButton), Some(false));
    assert!(node(&tree, "Clear filters").is_some(), "no Clear filters");
    // By type, then by author.
    let tree = tick(&mut app, &ty, "Highlight", Role::CheckBox);
    assert_eq!(
        count(&tree).as_deref(),
        Some("3 of 5 comments"),
        "Highlight"
    );
    assert_eq!(listed(&tree), ["Walrus one", "Walrus two", "Walrus three"]);
    let tree = tick(&mut app, &ty, "Other", Role::CheckBox);
    assert_eq!(listed(&tree), ["Walrus one", "Walrus three"], "Other");
    let tree = tick(&mut app, &ty, "(No name)", Role::CheckBox);
    assert_eq!(listed(&tree), ["Walrus one"], "(No name)");
    assert_eq!(count(&tree).as_deref(), Some("1 of 5 comments"));
    let tree = tick(&mut app, &ty, "(No name)", Role::CheckBox);
    assert_eq!(
        listed(&tree),
        ["Walrus one", "Walrus three"],
        "(No name) again"
    );
    // This page: the page being read, page 2.
    app.tabs[0].go_to(1);
    let tree = tick(&mut app, &ty, "This page", Role::RadioButton);
    assert_eq!(listed(&tree), ["Walrus three"], "This page");
    assert!(app.tabs[0].review.this_page);
    // A search, then Clear filters: every type, author and page, the search kept.
    close_filter(&mut app, &ty);
    search_for(&mut app, &ty, "walrus");
    let tree = tick(&mut app, &ty, "Clear filters", Role::Button);
    let r = &app.tabs[0].review;
    let cleared = r.hide_kinds.is_empty() && r.hide_authors.is_empty() && !r.this_page;
    assert!(cleared, "not cleared");
    assert_eq!(r.query, "walrus", "the search went with the filters");
    assert_eq!(listed(&tree), ["Walrus one", "Walrus two", "Walrus three"]);
    assert_eq!(count(&tree).as_deref(), Some("3 of 5 comments"));
}

/// The search box shows the threads whose comment, marked words or replies hold the words
/// typed, in any case; a reply found shows with its comment. Words on the page that
/// nothing marks find nothing.
#[test]
fn the_search_box_finds_comments_replies_and_marked_words() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut pdf = otters();
    let ate_fish = words_box(&pdf, "ate", "fish");
    highlight(&mut pdf, 0, ate_fish, "Tester");
    note(&mut pdf, 0, 300.0, "Walrus one", "Tester");
    let seal = note(&mut pdf, 0, 400.0, "Seal two", "Tester");
    pdf.add_reply(seal, "Penguin answer", "Tester").unwrap();
    let mut app = comments_tab(&ty, pdf);
    let tree = calm(&mut app, &ty);
    let all = ["“ate fish”", "Walrus one", "Seal two", "Penguin answer"];
    assert_eq!(listed(&tree), all);
    let tree = search_for(&mut app, &ty, "WALRUS");
    assert_eq!(listed(&tree), ["Walrus one"], "by the comment, in any case");
    let found = |app: &mut App, words: &str| {
        app.tabs[0].review.query = words.into();
        listed(&calm(app, &ty))
    };
    let penguin = found(&mut app, "penguin");
    assert_eq!(penguin, ["Seal two", "Penguin answer"], "by a reply");
    assert_eq!(
        found(&mut app, "Fish"),
        ["“ate fish”"],
        "by the words marked"
    );
    assert_eq!(found(&mut app, "otters"), [""; 0], "by words nothing marks");
    assert_eq!(found(&mut app, ""), all, "with nothing typed");
}

/// The filter and the search are each tab's own, kept while another tab is in front, and
/// never written into the file: the file saved and read back holds the same comments, and
/// opened again shows them all.
#[test]
fn filters_and_search_are_the_tabs_own_and_never_saved() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let make = || {
        let mut pdf = blank(1);
        note(&mut pdf, 0, 100.0, "Walrus one", "Tester");
        highlight(&mut pdf, 0, [100.0, 200.0, 300.0, 220.0], "Tester");
        pdf
    };
    let mut app = comments_tab(&ty, make());
    tick(&mut app, &ty, "Note", Role::CheckBox);
    close_filter(&mut app, &ty);
    let tree = search_for(&mut app, &ty, "zebra");
    assert_eq!(count(&tree).as_deref(), Some("0 of 2 comments"));
    // Another tab: all its comments, by no filter or search.
    assert!(app.add_tab(&ctx, make(), None, None, false));
    app.show_view(Side::Comments);
    let tree = ty.pause(&mut app);
    let r = &app.tabs[1].review;
    assert!(
        r.hide_kinds.is_empty() && r.query.is_empty(),
        "the first tab's filter"
    );
    assert_eq!(
        count(&tree).as_deref(),
        Some("2 comments"),
        "the first tab's filter"
    );
    // Back: its own filter and search.
    app.active = 0;
    let tree = ty.pause(&mut app);
    assert_eq!(
        count(&tree).as_deref(),
        Some("0 of 2 comments"),
        "the filter went"
    );
    assert_eq!(app.tabs[0].review.query, "zebra");
    assert!(!app.tabs[0].review.hide_kinds.is_empty());
    // Saved: the same comments, and nothing of the filter or the search.
    let before = app.tabs[0].pdf.annotations();
    let dir = test_dir("review-filters");
    let path = dir.join("filtered.pdf");
    SAVE_AS.with(|s| s.set(Some(path.clone())));
    app.act(&ctx, Act::SaveAs);
    saves_done(&mut app, &ctx);
    sync(&ty);
    let bytes = std::fs::read(&path).expect("not saved");
    let back = Pdf::from_bytes(&bytes, None).unwrap().annotations();
    let what = |annots: Vec<Annot>| -> Vec<(String, String, String)> {
        let what = |a: Annot| (a.label(), a.author, a.contents);
        annots.into_iter().map(what).collect()
    };
    assert_eq!(what(back), what(before), "the comments changed");
    let zebra = bytes.windows(5).any(|w| w == b"zebra");
    assert!(!zebra, "the search was written into the file");
    // Opened again: every comment shown.
    let mut again = App::with(Settings::from_file(None), Recent::from_file(None));
    open_and_wait(&mut again, &ctx, &path, 1);
    sync(&ty);
    again.show_view(Side::Comments);
    let tree = ty.pause(&mut again);
    assert_eq!(
        count(&tree).as_deref(),
        Some("2 comments"),
        "a filter came with the file"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A comment the filter hides, selected on the page, still shows in the inspector, though
/// its card isn't drawn; Next from the card before it goes past it.
#[test]
fn a_comment_the_filter_hides_can_still_be_selected_on_the_page() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut pdf = blank(1);
    let one = note(&mut pdf, 0, 100.0, "Walrus one", "Tester");
    let two = note(&mut pdf, 0, 300.0, "Walrus two", "Other");
    let three = note(&mut pdf, 0, 400.0, "Walrus three", "Tester");
    let mut app = comments_tab(&ty, pdf);
    tick(&mut app, &ty, "Other", Role::CheckBox);
    close_filter(&mut app, &ty);
    let tree = calm(&mut app, &ty);
    assert_eq!(listed(&tree), ["Walrus one", "Walrus three"]);
    click_page(&mut app, &ctx, (100.0, 300.0), false);
    sync(&ty);
    assert_eq!(chosen(&app), Some(two), "the click didn't select it");
    let tree = calm(&mut app, &ty);
    let says = inspector_says(&tree);
    assert!(
        says.iter().any(|s| s == "Walrus two"),
        "not in the inspector: {says:?}"
    );
    assert_eq!(
        listed(&tree),
        ["Walrus one", "Walrus three"],
        "its card is drawn"
    );
    assert!(
        cards(&tree).iter().all(|c| !c.selected),
        "a card is selected"
    );
    app.tabs[0].ed.selection = Selection::Annot { page: 0, id: one };
    calm(&mut app, &ty);
    run(&mut app, &ty, Act::NextComment);
    assert_eq!(chosen(&app), Some(three), "Next went to the comment hidden");
}

// ------------------------------------------------ the keyboard and replies

/// The comment list is one stop for Tab, as the thumbnails are, and a click on a card gives
/// it the keyboard. There Up and Down select the card before or after, Home and End the
/// first and last, the page view following; Delete deletes the comment selected, in one
/// undo step; Enter takes the keyboard to the reply box; Esc gives it back to the page,
/// the comment still selected, and a second Esc lets the comment go.
#[test]
fn the_comment_list_takes_the_keys_as_the_thumbnails_do() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut pdf = blank(2);
    let one = note(&mut pdf, 0, 100.0, "Walrus one", "Tester");
    let two = note(&mut pdf, 0, 300.0, "Walrus two", "Tester");
    let three = note(&mut pdf, 0, 500.0, "Walrus three", "Tester");
    let four = note(&mut pdf, 1, 100.0, "Walrus four", "Tester");
    let mut app = comments_tab(&ty, pdf);
    let tree = calm(&mut app, &ty);
    let list = (tree.nodes.iter()).find(|(_, n)| n.label() == Some("Comment list"));
    let (_, list) = list.expect("no Comment list");
    assert_eq!(list.role(), Role::ListBox);
    let focus = egui::accesskit::Action::Focus;
    assert!(list.supports_action(focus), "the list isn't a stop for Tab");
    let cs = cards(&tree);
    assert_eq!(cs.len(), 4, "{:?}", listed(&tree));
    let tree = click_card(&mut app, &ty, "Walrus one");
    let name = focused_name(&ctx, &tree);
    assert_eq!(
        name.as_deref(),
        Some("Comment list"),
        "the click didn't give it the keyboard"
    );
    assert_eq!(ctx.memory(|m| m.focused()), Some(app.tabs[0].comments_id()));
    let at = |app: &App| (app.tabs[0].ed.selection.annot(), app.tabs[0].current);
    press_key(&mut app, &ty, Key::ArrowDown);
    assert_eq!(at(&app), (Some((0, two)), 0), "Down");
    press_key(&mut app, &ty, Key::ArrowDown);
    assert_eq!(at(&app), (Some((0, three)), 0), "Down again");
    press_key(&mut app, &ty, Key::End);
    assert_eq!(at(&app), (Some((1, four)), 1), "End, its page shown");
    press_key(&mut app, &ty, Key::Home);
    assert_eq!(at(&app), (Some((0, one)), 0), "Home, its page shown");
    press_key(&mut app, &ty, Key::ArrowUp);
    assert_eq!(at(&app), (Some((0, one)), 0), "Up from the first");
    let tree = press_key(&mut app, &ty, Key::ArrowDown);
    let selected = card(&tree, "Walrus two").is_some_and(|c| c.selected);
    assert!(selected, "its card isn't selected");
    let name = focused_name(&ctx, &tree);
    assert_eq!(
        name.as_deref(),
        Some("Comment list"),
        "the keys left the list"
    );
    // Delete: the comment selected goes, in one undo step.
    let before = steps(&app);
    let tree = press_key(&mut app, &ty, Key::Delete);
    let left = ["Walrus one", "Walrus three", "Walrus four"];
    assert_eq!(comments(&app), left, "Delete");
    assert_eq!(steps(&app), before + 1, "not one undo step");
    assert!(card(&tree, "Walrus two").is_none(), "its card stayed");
    app.act(&ctx, Act::Undo);
    calm(&mut app, &ty);
    assert_eq!(comments(&app).len(), 4, "Undo");
    // Enter: to the reply box.
    click_card(&mut app, &ty, "Walrus one");
    let tree = press_key(&mut app, &ty, Key::Enter);
    assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Reply"), "Enter");
    assert_eq!(chosen(&app), Some(one));
    // Esc: back to the page, still selected; again, nothing selected.
    click_card(&mut app, &ty, "Walrus three");
    let tree = press_key(&mut app, &ty, Key::Escape);
    let name = focused_name(&ctx, &tree);
    assert_ne!(
        name.as_deref(),
        Some("Comment list"),
        "Esc left the keyboard in the list"
    );
    assert_eq!(chosen(&app), Some(three), "Esc let the comment go");
    press_key(&mut app, &ty, Key::Escape);
    assert_eq!(chosen(&app), None, "the second Esc kept the comment");
    // The cards aren't stops of their own: the list is the one stop.
    let stops: Vec<&str> = (cs.iter())
        .filter(|c| c.stop)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(stops, [""; 0], "cards that are stops of their own");
}

/// The selected card has a Reply to this comment button, which puts the keyboard in the
/// reply box; there Ctrl+Enter adds the reply, as Add reply does, and it shows as a card
/// right under its comment, indented. A card's right-click menu offers Reply… and Delete,
/// or Delete, with its replies once it has some.
#[test]
fn the_reply_button_and_ctrl_enter_add_a_reply_under_its_comment() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let (mut app, one, _) = two_notes(&ctx);
    sync(&ty);
    app.show_view(Side::Comments);
    ty.pause(&mut app);
    let tree = click_card(&mut app, &ty, "Walrus one");
    assert_eq!(chosen(&app), Some(one));
    assert_eq!(
        named(&tree, "Reply to this comment"),
        1,
        "not one Reply button"
    );
    let tree = click_on(&mut app, &ty, "Reply to this comment");
    let name = focused_name(&ctx, &tree);
    assert_eq!(
        name.as_deref(),
        Some("Reply"),
        "the keyboard isn't in the reply box"
    );
    type_in(&mut app, &ty, "Walrus answer");
    let ctrl_enter = key_with(Key::Enter, egui::Modifiers::COMMAND);
    ty.frame(&mut app, vec![ctrl_enter]);
    let tree = calm(&mut app, &ty);
    let added = pairs(&[("Walrus one", "Walrus answer")]);
    let now = replies(&app.tabs[0].pdf);
    assert_eq!(now, added, "Ctrl+Enter didn't add the reply");
    assert_eq!(app.tabs[0].ed.draft(one), None, "still a draft");
    let left = reply_box(&tree);
    assert_eq!(left.as_deref(), Some(""), "the reply box kept it");
    assert_eq!(listed(&tree), ["Walrus one", "Walrus answer", "Walrus two"]);
    let cs = cards(&tree);
    assert!(cs[1].name.starts_with("Reply · "), "{}", cs[1].name);
    let indented = cs[1].rect.left() > cs[0].rect.left();
    assert!(
        indented,
        "the reply isn't indented: {:?} {:?}",
        cs[1].rect, cs[0].rect
    );
    // The right-click menus.
    let deletes = named(&tree, "Delete");
    let c = card(&tree, "Walrus two").unwrap();
    let tree = right_click(&mut app, &ty, c.spot());
    assert_eq!(named(&tree, "Reply…"), 1, "no Reply… in its menu");
    assert_eq!(named(&tree, "Delete"), deletes + 1, "no Delete in its menu");
    press_key(&mut app, &ty, Key::Escape);
    let tree = calm(&mut app, &ty);
    let c = card(&tree, "Walrus one").unwrap();
    let tree = right_click(&mut app, &ty, c.spot());
    let with_replies = named(&tree, "Delete, with its replies");
    assert_eq!(with_replies, 1, "its replies aren't said");
}

/// Replies saved in a file and opened again show under the comments they answer, indented,
/// by their author.
#[test]
fn saved_replies_show_under_their_comments_when_opened_again() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let dir = test_dir("review-replies");
    let path = dir.join("replies.pdf");
    let mut pdf = blank(1);
    let one = note(&mut pdf, 0, 100.0, "Walrus one", "Tester");
    note(&mut pdf, 0, 300.0, "Walrus two", "Tester");
    pdf.add_reply(one, "Walrus answer", "Replier").unwrap();
    pdf.save(&path).unwrap();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    open_and_wait(&mut app, &ctx, &path, 1);
    sync(&ty);
    app.show_view(Side::Comments);
    let tree = ty.pause(&mut app);
    assert_eq!(listed(&tree), ["Walrus one", "Walrus answer", "Walrus two"]);
    let cs = cards(&tree);
    let reply = &cs[1].name;
    assert!(reply.starts_with("Reply · Replier · "), "{reply}");
    let indented = cs[1].rect.left() > cs[0].rect.left();
    assert!(
        indented,
        "the reply isn't indented: {:?} {:?}",
        cs[1].rect, cs[0].rect
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A comment deleted, from its card's menu or on the page, takes its card and its
/// selection with it, and Undo brings its card back; a selected comment Undo takes away
/// isn't left selected.
#[test]
fn deleted_comments_take_their_cards_away() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let (mut app, one, two) = two_notes(&ctx);
    sync(&ty);
    app.show_view(Side::Comments);
    ty.pause(&mut app);
    // From its card's menu.
    let tree = click_card(&mut app, &ty, "Walrus two");
    assert_eq!(chosen(&app), Some(two));
    let c = card(&tree, "Walrus two").unwrap();
    let tree = right_click(&mut app, &ty, c.spot());
    let p = nearest(&tree, "Delete", c.spot()).expect("no Delete in its menu");
    let tree = tap(&mut app, &ty, p);
    assert_eq!(comments(&app), ["Walrus one"], "not deleted");
    assert_eq!(listed(&tree), ["Walrus one"], "its card stayed");
    assert_ne!(chosen(&app), Some(two), "still selected");
    app.act(&ctx, Act::Undo);
    let tree = calm(&mut app, &ty);
    assert_eq!(listed(&tree), ["Walrus one", "Walrus two"], "Undo");
    // On the page, selected there.
    app.tabs[0].ed.selection = Selection::Annot { page: 0, id: one };
    calm(&mut app, &ty);
    app.act(&ctx, Act::DeleteSelection);
    let tree = calm(&mut app, &ty);
    assert_eq!(comments(&app), ["Walrus two"], "not deleted on the page");
    assert_eq!(listed(&tree), ["Walrus two"], "its card stayed");
    assert_eq!(chosen(&app), None, "still selected");
    app.act(&ctx, Act::Undo);
    let tree = calm(&mut app, &ty);
    assert_eq!(listed(&tree), ["Walrus one", "Walrus two"], "Undo");
    // Added, selected, then taken away by Undo.
    let mut three = None;
    let add = |p: &mut Pdf| -> limarepdf_core::Result<()> {
        three = Some(p.add_note(0, (100.0, 400.0), YELLOW, "Walrus three", "Tester")?);
        Ok(())
    };
    assert!(app.tabs[0].edit(&ctx, add));
    let three = three.unwrap();
    app.tabs[0].ed.selection = Selection::Annot { page: 0, id: three };
    let tree = calm(&mut app, &ty);
    assert!(card(&tree, "Walrus three").is_some_and(|c| c.selected));
    app.act(&ctx, Act::Undo);
    let tree = calm(&mut app, &ty);
    assert_ne!(chosen(&app), Some(three), "left selected once gone");
    assert_eq!(
        listed(&tree),
        ["Walrus one", "Walrus two"],
        "its card stayed"
    );
}

// ------------------------------------------------ drafts (Q1)

/// Q1: a draft's card ends "Draft reply", and each tab keeps its own drafts while another
/// is in front, even for comments of the same ids in another document; nothing is added
/// meanwhile.
#[test]
fn a_draft_shows_on_its_card_and_stays_with_its_tab() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let make = || {
        let mut pdf = blank(1);
        let one = note(&mut pdf, 0, 100.0, "Walrus one", "Tester");
        let two = note(&mut pdf, 0, 300.0, "Walrus two", "Tester");
        (pdf, one, two)
    };
    let (pdf, one, two) = make();
    let mut app = comments_tab(&ty, pdf);
    let (other, other_one, _) = make();
    assert!(app.add_tab(&ctx, other, None, None, false));
    app.tabs[0].path = Some(PathBuf::from("/nonexistent/seal.pdf"));
    app.tabs[1].path = Some(PathBuf::from("/nonexistent/otter.pdf"));
    app.active = 0;
    draft_on(&mut app, &ty, 0, one, "Walrus left");
    // Left by a click on the page where nothing is: kept.
    click_page(&mut app, &ctx, (400.0, 200.0), false);
    sync(&ty);
    assert_eq!(chosen(&app), None, "the click on nothing kept it selected");
    let kept = app.tabs[0].ed.draft(one);
    assert_eq!(kept, Some("Walrus left"), "a click away lost it");
    app.tabs[0].ed.selection = Selection::Annot { page: 0, id: two };
    let tree = calm(&mut app, &ty);
    let cs = cards(&tree);
    let drafted: Vec<bool> = cs.iter().map(Card::drafted).collect();
    let names: Vec<&str> = cs.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(drafted, [true, false], "{names:?}");
    assert_eq!(app.tabs[0].ed.draft(one), Some("Walrus left"));
    assert_eq!(app.tabs[0].ed.draft(two), None);
    // The other tab: no drafts, then one of its own.
    to_tab(&mut app, &ty, "otter.pdf");
    assert_eq!(app.active, 1, "the other tab isn't in front");
    app.show_view(Side::Comments);
    let tree = ty.pause(&mut app);
    let drafts = cards(&tree).iter().any(Card::drafted);
    assert!(!drafts, "the first tab's draft shows in the other");
    assert_eq!(app.tabs[1].ed.draft(other_one), None);
    draft_on(&mut app, &ty, 0, other_one, "Walrus right");
    // Back: each tab its own.
    to_tab(&mut app, &ty, "seal.pdf");
    assert_eq!(app.active, 0, "the first tab isn't in front");
    app.tabs[0].ed.selection = Selection::Annot { page: 0, id: one };
    let tree = calm(&mut app, &ty);
    let kept = reply_box(&tree);
    let kept = kept.as_deref();
    assert_eq!(kept, Some("Walrus left"), "the draft left its tab");
    assert_eq!(app.tabs[1].ed.draft(other_one), Some("Walrus right"));
    for t in &app.tabs {
        assert_eq!(t.pdf.annotations().len(), 2, "a reply was added");
    }
}

/// Save adds every draft as a reply first, one undo step each, so the saved file holds
/// them all; closing or quitting with a draft asks first, and Discard drops it.
#[test]
fn save_adds_every_draft_and_closing_with_one_asks() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let (mut app, one, two) = two_notes(&ctx);
    sync(&ty);
    app.show_view(Side::Comments);
    ty.pause(&mut app);
    draft_on(&mut app, &ty, 0, one, "Walrus draft one");
    draft_on(&mut app, &ty, 0, two, "Walrus draft two");
    let before = steps(&app);
    let dir = test_dir("review-drafts");
    let path = dir.join("drafts.pdf");
    SAVE_AS.with(|s| s.set(Some(path.clone())));
    app.act(&ctx, Act::Save);
    saves_done(&mut app, &ctx);
    sync(&ty);
    let asked = !matches!(app.dialog, Dialog::None);
    assert!(!asked, "something was asked or said");
    assert_eq!(steps(&app), before + 2, "not one undo step each");
    let t = &app.tabs[0];
    let drafts = (t.ed.draft(one), t.ed.draft(two));
    assert_eq!(drafts, (None, None), "still drafts");
    let back = Pdf::from_bytes(&std::fs::read(&path).unwrap(), None).unwrap();
    let both = [
        ("Walrus one", "Walrus draft one"),
        ("Walrus two", "Walrus draft two"),
    ];
    assert_eq!(replies(&back), pairs(&both), "not saved as replies");
    assert!(!app.tabs[0].unsaved(), "unsaved after saving");
    std::fs::remove_dir_all(&dir).unwrap();
    // Closing with a draft asks; so does quitting; Discard drops it.
    let (mut app, one, _) = two_notes(&ctx);
    sync(&ty);
    draft_on(&mut app, &ty, 0, one, "Walrus left open");
    app.act(&ctx, Act::CloseTab);
    let asked = matches!(app.dialog, Dialog::ConfirmClose(..));
    assert!(asked, "closed without asking");
    assert_eq!(app.tabs.len(), 1, "closed, and the draft lost");
    app.dialog = Dialog::None;
    let kept = app.tabs[0].ed.draft(one);
    assert_eq!(kept, Some("Walrus left open"), "Cancel");
    app.act(&ctx, Act::Quit);
    assert!(!app.quit_ok, "quit, and the draft lost");
    let asked = matches!(app.dialog, Dialog::ConfirmClose(_, true));
    assert!(asked, "quit without asking");
    app.dialog = Dialog::None;
    close_by_button(&mut app, &ctx, "Untitled *");
    let asked = matches!(app.dialog, Dialog::ConfirmClose(..));
    assert!(asked, "closed by its button without asking");
    click(&mut app, &ctx, "Discard");
    assert!(app.tabs.is_empty(), "Discard didn't close it");
}

/// A command that uses the document (a page turned, here) adds the drafts as replies
/// first, one undo step each, then does its change.
#[test]
fn a_page_command_adds_the_drafts_first() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let (mut app, one, _) = two_notes(&ctx);
    sync(&ty);
    draft_on(&mut app, &ty, 0, one, "Walrus before the turn");
    let before = steps(&app);
    app.tabs[0].ed.selection = Selection::None;
    calm(&mut app, &ty);
    app.act(&ctx, Act::RotR);
    calm(&mut app, &ty);
    let added = pairs(&[("Walrus one", "Walrus before the turn")]);
    let now = replies(&app.tabs[0].pdf);
    assert_eq!(now, added, "the draft wasn't added first");
    assert_eq!(app.tabs[0].ed.draft(one), None, "still a draft");
    let size = app.tabs[0].pdf.page_size(0).unwrap();
    assert_eq!(size, (792.0, 612.0), "not turned");
    assert_eq!(steps(&app), before + 2, "not a step for each");
}

/// Undo and Redo leave drafts as drafts, neither added nor lost. A draft whose comment
/// Undo takes away comes back with it at Redo, isn't added at Save (there is nothing to
/// add it to) and doesn't count as unsaved work.
#[test]
fn undo_and_redo_leave_drafts_as_drafts() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut pdf = blank(1);
    let one = note(&mut pdf, 0, 100.0, "Walrus one", "Tester");
    let mut app = comments_tab(&ty, pdf);
    // The second note added as a change, for Undo to take away.
    let mut two = None;
    let add = |p: &mut Pdf| -> limarepdf_core::Result<()> {
        two = Some(p.add_note(0, (100.0, 300.0), YELLOW, "Walrus two", "Tester")?);
        Ok(())
    };
    assert!(app.tabs[0].edit(&ctx, add));
    let two = two.unwrap();
    draft_on(&mut app, &ty, 0, one, "Walrus kept");
    draft_on(&mut app, &ty, 0, two, "Walrus aside");
    // Nothing selected, so the reply box doesn't have the keyboard (Undo would take back
    // its typing).
    app.tabs[0].ed.selection = Selection::None;
    calm(&mut app, &ty);
    let drafts = |app: &App| {
        let ed = &app.tabs[0].ed;
        let text = |id| ed.draft(id).map(String::from);
        (text(one), text(two))
    };
    let both: (Option<String>, _) = (Some("Walrus kept".into()), Some("Walrus aside".into()));
    app.act(&ctx, Act::Undo);
    let tree = calm(&mut app, &ty);
    let left = comments(&app);
    assert_eq!(left, ["Walrus one"], "Undo took a reply, or added one");
    assert_eq!(drafts(&app).0, both.0, "Undo lost the draft");
    assert!(card(&tree, "Walrus one").is_some_and(|c| c.drafted()));
    app.act(&ctx, Act::Redo);
    let tree = calm(&mut app, &ty);
    let back = comments(&app);
    assert_eq!(back, ["Walrus one", "Walrus two"], "Redo added a reply");
    assert_eq!(drafts(&app), both, "Redo didn't bring the draft back");
    let drafted: Vec<bool> = cards(&tree).iter().map(Card::drafted).collect();
    assert_eq!(drafted, [true, true]);
    // Taken away again: not unsaved for it, and not added at Save.
    app.act(&ctx, Act::Undo);
    calm(&mut app, &ty);
    let dir = test_dir("review-undo-drafts");
    let path = dir.join("undone.pdf");
    SAVE_AS.with(|s| s.set(Some(path.clone())));
    app.act(&ctx, Act::Save);
    saves_done(&mut app, &ctx);
    sync(&ty);
    let asked = !matches!(app.dialog, Dialog::None);
    assert!(!asked, "something was asked or said");
    let back = Pdf::from_bytes(&std::fs::read(&path).unwrap(), None).unwrap();
    let kept = pairs(&[("Walrus one", "Walrus kept")]);
    assert_eq!(replies(&back), kept, "not as the drafts left");
    let unsaved = app.tabs[0].unsaved();
    assert!(!unsaved, "the draft set aside counts as unsaved");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Reload with a draft asks first, as for any typing not saved: Cancel keeps the words
/// typed; Discard and reload shows the file as it is, without them.
#[test]
fn reload_with_a_draft_asks_first() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let dir = test_dir("review-reload");
    let path = dir.join("notes.pdf");
    let mut pdf = blank(1);
    let one = note(&mut pdf, 0, 100.0, "Walrus one", "Tester");
    pdf.save(&path).unwrap();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    open_and_wait(&mut app, &ctx, &path, 1);
    sync(&ty);
    app.show_view(Side::Comments);
    ty.pause(&mut app);
    draft_on(&mut app, &ty, 0, one, "Walrus typed");
    app.act(&ctx, Act::Reload);
    let asked = matches!(app.dialog, Dialog::ConfirmReload(_));
    assert!(asked, "reloaded without asking");
    click(&mut app, &ctx, "Cancel");
    sync(&ty);
    let typed = "Walrus typed".to_string();
    let drafted = app.tabs[0].ed.draft(one) == Some(typed.as_str());
    let kept = drafted || comments(&app).contains(&typed);
    assert!(kept, "Cancel lost the words typed");
    app.act(&ctx, Act::Reload);
    let asked = matches!(app.dialog, Dialog::ConfirmReload(_));
    assert!(asked, "the second time");
    let old = app.tabs[0].id;
    click(&mut app, &ctx, "Discard and reload");
    wait_for(&mut app, &ctx, |a| a.tabs[0].id != old);
    assert_eq!(comments(&app), ["Walrus one"], "not the file as it is");
    assert_eq!(app.tabs[0].ed.draft(one), None, "the draft stayed");
    assert!(!app.tabs[0].unsaved());
    std::fs::remove_dir_all(&dir).unwrap();
}

// ------------------------------------------------ Previous and Next comment (Q2)

/// Q2: Previous and Next comment go through the cards in the Comments view's order, with its
/// filter and search, selecting each comment and showing its page. From nothing selected
/// Next goes to the first card on or after the page being read, Previous to the last on or
/// before it; at the ends they stay, saying so, and with no cards they say there are none.
#[test]
fn previous_and_next_comment_go_through_the_cards() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut pdf = blank(3);
    let a = note(&mut pdf, 0, 100.0, "Walrus a", "Tester");
    let h = highlight(&mut pdf, 0, [100.0, 300.0, 300.0, 320.0], "Tester");
    let b1 = note(&mut pdf, 1, 100.0, "Walrus b1", "Tester");
    let b2 = note(&mut pdf, 1, 300.0, "Walrus b2", "Tester");
    let c = note(&mut pdf, 2, 100.0, "Walrus c", "Tester");
    let mut app = comments_tab(&ty, pdf);
    let at = |app: &App| (chosen(app), app.tabs[0].current);
    // From nothing selected, reading page 2.
    app.tabs[0].go_to(1);
    calm(&mut app, &ty);
    run(&mut app, &ty, Act::NextComment);
    assert_eq!(at(&app), (Some(b1), 1), "Next from nothing");
    app.tabs[0].ed.selection = Selection::None;
    app.tabs[0].go_to(1);
    calm(&mut app, &ty);
    run(&mut app, &ty, Act::PrevComment);
    assert_eq!(at(&app), (Some(b2), 1), "Previous from nothing");
    // Through the cards from the first, each one's page shown.
    app.tabs[0].ed.selection = Selection::Annot { page: 0, id: a };
    calm(&mut app, &ty);
    let mut went = Vec::new();
    for _ in 0..4 {
        run(&mut app, &ty, Act::NextComment);
        went.push(at(&app));
    }
    let cards_order = [(Some(h), 0), (Some(b1), 1), (Some(b2), 1), (Some(c), 2)];
    assert_eq!(went, cards_order);
    let tree = calm(&mut app, &ty);
    let shown = card(&tree, "Walrus c").is_some_and(|c| c.selected);
    assert!(shown, "its card isn't selected");
    // At the ends: they stay, saying so.
    let said = run(&mut app, &ty, Act::NextComment);
    assert_eq!(chosen(&app), Some(c), "Next went past the last");
    assert_eq!(said.as_deref(), Some("No more comments after this one"));
    run(&mut app, &ty, Act::PrevComment);
    assert_eq!(chosen(&app), Some(b2), "Previous");
    app.tabs[0].ed.selection = Selection::Annot { page: 0, id: a };
    calm(&mut app, &ty);
    let said = run(&mut app, &ty, Act::PrevComment);
    assert_eq!(chosen(&app), Some(a), "Previous went past the first");
    assert_eq!(said.as_deref(), Some("No more comments before this one"));
    // The filter: the highlight hidden is gone past.
    tick(&mut app, &ty, "Highlight", Role::CheckBox);
    close_filter(&mut app, &ty);
    app.tabs[0].ed.selection = Selection::Annot { page: 0, id: a };
    calm(&mut app, &ty);
    run(&mut app, &ty, Act::NextComment);
    assert_eq!(chosen(&app), Some(b1), "Next went to the highlight hidden");
    // The search: only the cards found, from the page being read.
    app.tabs[0].review.query = "walrus c".into();
    app.tabs[0].ed.selection = Selection::None;
    app.tabs[0].go_to(0);
    calm(&mut app, &ty);
    run(&mut app, &ty, Act::NextComment);
    assert_eq!(at(&app), (Some(c), 2), "Next with a search");
    // No cards: none to go to.
    app.tabs[0].review.query.clear();
    tick(&mut app, &ty, "Note", Role::CheckBox);
    close_filter(&mut app, &ty);
    let said = run(&mut app, &ty, Act::NextComment);
    assert_eq!(said.as_deref(), Some("No comments to go to"));
    assert_eq!(chosen(&app), Some(c), "the selection changed");
}

/// From nothing selected, with no comment past the page being read, Previous and Next say
/// there are no more after or before the page.
#[test]
fn previous_and_next_comment_say_when_none_is_past_the_page() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut pdf = blank(3);
    note(&mut pdf, 1, 100.0, "Walrus middle", "Tester");
    let mut app = comments_tab(&ty, pdf);
    app.tabs[0].go_to(2);
    calm(&mut app, &ty);
    let said = run(&mut app, &ty, Act::NextComment);
    assert_eq!(said.as_deref(), Some("No more comments after this page"));
    assert_eq!(chosen(&app), None);
    app.tabs[0].go_to(0);
    calm(&mut app, &ty);
    let said = run(&mut app, &ty, Act::PrevComment);
    assert_eq!(said.as_deref(), Some("No more comments before this page"));
    assert_eq!(chosen(&app), None);
}

/// Previous and Next comment are in the Review band's Comments group (as Previous and
/// Next), in ☰'s View menu right after Comments, and in the command search; the band's
/// Next and the command search's go to the next card.
#[test]
fn previous_and_next_comment_are_in_the_band_the_menu_and_the_command_search() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let steps = [
        (Act::PrevComment, "Previous comment", "Previous"),
        (Act::NextComment, "Next comment", "Next"),
    ];
    let group = (Band::Review.groups().iter()).find(|g| g.name == "Comments");
    let group = group.expect("no Comments group in the Review band");
    let buttons: Vec<(Option<Act>, &str)> = (group.items.iter())
        .filter_map(|i| match i {
            band::Item::Button(b) => Some((b.act, b.label)),
            _ => None,
        })
        .collect();
    for (act, label, short) in steps {
        let a = actions::get(act);
        assert_eq!(a.label, label);
        assert_eq!(a.group, Group::View, "{label}");
        let listed = a.menu && a.palette;
        assert!(listed, "{label}: not in ☰ or the command search");
        let there = buttons.contains(&(Some(act), short));
        assert!(there, "no {short} in the Comments group: {buttons:?}");
    }
    let view: Vec<Act> = (ACTIONS.iter())
        .filter(|a| a.group == Group::View && a.menu)
        .map(|a| a.act)
        .collect();
    let i = view.iter().position(|&a| a == Act::Comments);
    let i = i.expect("no Comments in ☰'s View");
    let after = view.get(i + 1..i + 3);
    let both = [Act::PrevComment, Act::NextComment];
    assert_eq!(after, Some(&both[..]), "not right after Comments in ☰");
    // Used: the band's Next, then the command search's Next comment.
    let (mut app, one, two) = two_notes(&ctx);
    sync(&ty);
    app.band = Band::Review;
    let tree = ty.pause(&mut app);
    assert!(node(&tree, "Previous").is_some(), "no Previous in the band");
    click_on(&mut app, &ty, "Next");
    assert_eq!(chosen(&app), Some(one), "the band's Next");
    from_palette(&mut app, &ty, "Next comment");
    assert_eq!(chosen(&app), Some(two), "the command search's Next comment");
}
