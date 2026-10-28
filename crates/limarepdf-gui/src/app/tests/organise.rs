//! Batch 41 (NEW-14): the Organise workspace, a grid of the document's pages beside the
//! page view, driven as people use it, with app.rs's helpers.

use super::*;

/// An app with one tab of `n` pages, page `i` 400 + `i` points wide and 600 high, as if
/// just opened.
fn grid_tab(ctx: &egui::Context, n: usize) -> App {
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let mut pdf = Pdf::new();
    for i in 0..n {
        pdf.insert_blank(i, 400.0 + i as f32, 600.0).unwrap();
    }
    assert!(app.add_tab(ctx, pdf, None, None, false));
    app
}

/// The pages in their order now, each by where it was when the tab was made (`first`, the
/// page ids then).
fn order(app: &App, first: &[(u32, u16)]) -> Vec<usize> {
    let ids = app.tabs[app.active].pdf.page_ids();
    let at = |id| first.iter().position(|f| *f == id);
    ids.into_iter().map(|id| at(id).expect("a page")).collect()
}

/// Let go of the keys held.
fn keys_up(app: &mut App, ty: &Typist) {
    let none = egui::Event::ModifiersChanged(egui::Modifiers::NONE);
    ty.frame(app, vec![none]);
}

/// The pages selected, 0-based.
fn chosen(app: &App) -> Vec<usize> {
    let t = &app.tabs[app.active];
    t.ed.selection
        .pages()
        .map_or(Vec::new(), |s| s.iter().copied().collect())
}

/// Click the control named `name` with `held` keys down, as people click: the clock moves
/// on, so two clicks are a double-click only when meant.
fn click_as(app: &mut App, ty: &Typist, name: &str, held: egui::Modifiers) {
    let tree = ty.pause(app);
    let p = middle(&tree, name);
    let held = |e: egui::Event| vec![egui::Event::ModifiersChanged(held), e];
    ty.frame(app, held(egui::Event::PointerMoved(p)));
    ty.frame(app, held(press(p, true)));
    ty.frame(app, held(press(p, false)));
    keys_up(app, ty);
    ty.pause(app);
}

/// Organise the active tab, from the command, and let the grid settle.
fn organise(app: &mut App, ty: &Typist) -> egui::accesskit::TreeUpdate {
    app.act(ty.ctx, Act::Organise);
    ty.pause(app)
}

fn test_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    ctx
}

/// Organise opens from the Pages band (no longer a Planned placeholder), the View menu's
/// command and the status bar's view buttons, and closes from them; the grid shows each
/// page as a card with its checkbox; nothing of it marks the document changed.
#[test]
fn organise_opens_from_the_band_the_view_menu_and_the_status_bar() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    app.band = Band::Pages;
    let tree = ty.pause(&mut app);
    assert!(node(&tree, "Organise — Planned").is_none(), "still planned");
    click_as(&mut app, &ty, "Organise", egui::Modifiers::NONE);
    assert!(app.tabs[0].organising(), "the band's button");
    let tree = ty.pause(&mut app);
    for i in 1..=6 {
        assert!(node(&tree, &format!("Page {i}")).is_some(), "no card {i}");
        let check = format!("Select page {i}");
        assert!(node(&tree, &check).is_some(), "no checkbox {i}");
    }
    assert!(shows(&tree, "Organise pages"), "no heading");
    // The status bar's buttons: the page view, and back.
    click_as(&mut app, &ty, "Page view", egui::Modifiers::NONE);
    assert!(!app.tabs[0].organising(), "the page view button");
    click_as(&mut app, &ty, "Organise pages", egui::Modifiers::NONE);
    assert!(app.tabs[0].organising(), "the organise button");
    // The View menu's command, on and off, ticked while on.
    let a = actions::get(Act::Organise);
    assert_eq!(a.group, actions::Group::View);
    let c = app.state(&ctx);
    assert!(a.checked.is_some_and(|on| on(&c)), "not ticked");
    app.act(&ctx, Act::Organise);
    assert!(!app.tabs[0].organising(), "the command");
    let t = &app.tabs[0];
    assert!(!t.dirty && t.revision == 0, "the document changed");
}

/// Click selects one page, Shift+click a range from it, Ctrl+click and the checkboxes add or
/// take out one; the heading says how many, the inspector shows them; Ctrl+A selects every
/// page and Esc none.
#[test]
fn clicks_and_checkboxes_select_pages_in_the_grid() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    organise(&mut app, &ty);
    let none = egui::Modifiers::NONE;
    click_as(&mut app, &ty, "Page 2", none);
    assert_eq!(chosen(&app), [1]);
    click_as(&mut app, &ty, "Page 4", egui::Modifiers::SHIFT);
    assert_eq!(chosen(&app), [1, 2, 3]);
    click_as(&mut app, &ty, "Page 3", egui::Modifiers::COMMAND);
    assert_eq!(chosen(&app), [1, 3]);
    click_as(&mut app, &ty, "Select page 6", none);
    assert_eq!(chosen(&app), [1, 3, 5]);
    click_as(&mut app, &ty, "Select page 2", none);
    assert_eq!(chosen(&app), [3, 5]);
    let tree = ty.pause(&mut app);
    assert!(shows(&tree, "6 pages · 2 selected"), "the heading's count");
    assert_eq!(inspector::title(&app.tabs[0]), "Selected pages");
    ty.key(&mut app, Key::A, egui::Modifiers::COMMAND);
    assert_eq!(chosen(&app), [0, 1, 2, 3, 4, 5], "Ctrl+A");
    ty.key(&mut app, Key::Escape, none);
    assert_eq!(chosen(&app), [0usize; 0], "Esc");
    assert!(app.tabs[0].organising(), "Esc left Organise");
    assert!(!app.tabs[0].dirty, "selecting changed the document");
}

/// A drag across the grid's background selects the cards it touches, as file managers do;
/// with Ctrl held, those are added to the pages already selected.
#[test]
fn a_drag_across_the_background_selects_the_cards_it_touches() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 20);
    let tree = organise(&mut app, &ty);
    let card = |tree: &egui::accesskit::TreeUpdate, i: usize| {
        node(tree, &format!("Page {i}")).expect("the card").0
    };
    let (one, two) = (card(&tree, 1), card(&tree, 2));
    assert!(two.min.x > one.max.x, "the cards aren't side by side");
    let row = (1..=20)
        .take_while(|&i| card(&tree, i).min.y == one.min.y)
        .count();
    assert!((2..=8).contains(&row), "{row} cards in a row");
    // From the gap right of the first card, half way down it, to the middle of the third
    // card of the second row.
    let from = egui::pos2((one.max.x + two.min.x) / 2.0, one.center().y);
    let to = card(&tree, row + 3).center();
    let swept = egui::Rect::from_two_pos(from, to);
    let touched: Vec<usize> = (1..=20)
        .filter(|&i| card(&tree, i).intersects(swept))
        .map(|i| i - 1)
        .collect();
    assert!(!touched.contains(&0) && touched.len() > 1, "{touched:?}");
    let drag = |app: &mut App, held: egui::Modifiers| {
        let with = |e: egui::Event| vec![egui::Event::ModifiersChanged(held), e];
        ty.frame(app, with(egui::Event::PointerMoved(from)));
        ty.frame(app, with(press(from, true)));
        for i in 1..=6 {
            let p = from + (to - from) * (i as f32 / 6.0);
            ty.frame(app, with(egui::Event::PointerMoved(p)));
        }
        ty.frame(app, with(press(to, false)));
        keys_up(app, &ty);
        ty.pause(app);
    };
    drag(&mut app, egui::Modifiers::NONE);
    assert_eq!(chosen(&app), touched);
    click_as(&mut app, &ty, "Page 1", egui::Modifiers::NONE);
    drag(&mut app, egui::Modifiers::COMMAND);
    let mut both = vec![0];
    both.extend(&touched);
    assert_eq!(chosen(&app), both, "with Ctrl");
    assert!(!app.tabs[0].dirty);
}

/// The page commands act on the pages selected in the grid; a deleted page leaves the
/// selection, the others keeping theirs; Extract opens them in a new tab.
#[test]
fn page_commands_act_on_the_pages_selected_in_the_grid() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    let first = app.tabs[0].pdf.page_ids();
    organise(&mut app, &ty);
    click_as(&mut app, &ty, "Page 2", egui::Modifiers::NONE);
    click_as(&mut app, &ty, "Page 5", egui::Modifiers::COMMAND);
    app.act(&ctx, Act::RotR);
    let rotated: Vec<bool> = (0..6)
        .map(|i| {
            let (w, h) = app.tabs[0].pdf.page_size(i).unwrap();
            w > h
        })
        .collect();
    assert_eq!(rotated, [false, true, false, false, true, false]);
    assert_eq!(chosen(&app), [1, 4], "rotating kept the selection");
    // Pages 1 and 2 go: page 5, selected, is the third now.
    app.tabs[0].ed.selection = Selection::pages_from([0, 1].into());
    app.act(&ctx, Act::Delete);
    assert_eq!(order(&app, &first), [2, 3, 4, 5]);
    assert_eq!(chosen(&app), [0usize; 0], "deleted pages selected");
    app.tabs[0].ed.selection = Selection::pages_from([2].into());
    ty.pause(&mut app);
    app.act(&ctx, Act::Extract);
    assert_eq!(app.tabs.len(), 2, "no new tab");
    let size = app.tabs[1].pdf.page_size(0).unwrap();
    assert_eq!(size, (600.0, 404.0), "not the page selected");
}

/// The pictures of pages moved or deleted around are kept, by the page, not drawn again;
/// a turned page's is drawn again.
#[test]
fn pages_keep_their_pictures_when_pages_around_them_change() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    organise(&mut app, &ty);
    let drawn = |app: &mut App| {
        // Until every card in view has its picture.
        for _ in 0..200 {
            ty.frame(app, vec![]);
            if app.tabs[0].grid_waiting() == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(app.tabs[0].grid_waiting(), 0, "pictures still coming");
        app.tabs[0].grid_pictures()
    };
    let before = drawn(&mut app);
    assert_eq!(before.len(), 6, "{before:?}");
    // Page 2 goes: pages 3 to 6 move up one, with the pictures they had.
    app.tabs[0].ed.selection = Selection::pages_from([1].into());
    app.act(&ctx, Act::Delete);
    let after = app.tabs[0].grid_pictures();
    let kept: Vec<_> = [0, 2, 3, 4, 5].iter().map(|&i| before[i]).collect();
    assert_eq!(after, kept, "pictures drawn again");
    // Page 3 turned: its picture is drawn again, the others kept.
    app.tabs[0].ed.selection = Selection::pages_from([2].into());
    app.act(&ctx, Act::RotR);
    let turned = app.tabs[0].grid_pictures();
    assert_eq!(turned.len(), 4, "{turned:?}");
    let others = [turned[0], turned[1], turned[3]];
    assert_eq!(others, [kept[0], kept[1], kept[4]]);
    let again = drawn(&mut app);
    assert_eq!(again.len(), 5);
    assert_ne!(again[2], kept[3], "the turned page's old picture");
}

/// Back from Organise, the page view is at the page that was being read, wherever it went;
/// when it was deleted, at the page now in its place. A double-click on a card opens that
/// page instead.
#[test]
fn leaving_organise_returns_to_the_page_being_read() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    let first = app.tabs[0].pdf.page_ids();
    app.tabs[0].go_to(3);
    ty.pause(&mut app);
    assert_eq!(app.tabs[0].current, 3);
    organise(&mut app, &ty);
    app.tabs[0].ed.selection = Selection::pages_from([0, 1].into());
    app.act(&ctx, Act::Delete);
    ty.pause(&mut app);
    app.act(&ctx, Act::Organise);
    ty.pause(&mut app);
    assert_eq!(app.tabs[0].current, 1, "not the page being read");
    assert_eq!(order(&app, &first)[1], 3);
    // The page being read deleted: the one now in its place.
    organise(&mut app, &ty);
    app.tabs[0].ed.selection = Selection::pages_from([1].into());
    app.act(&ctx, Act::Delete);
    app.act(&ctx, Act::Organise);
    ty.pause(&mut app);
    assert_eq!(app.tabs[0].current, 1);
    assert_eq!(order(&app, &first)[1], 4);
    // A double-click on a card opens its page.
    organise(&mut app, &ty);
    let tree = ty.pause(&mut app);
    let p = middle(&tree, "Page 3");
    ty.frame(&mut app, vec![egui::Event::PointerMoved(p), press(p, true)]);
    ty.frame(&mut app, vec![press(p, false)]);
    ty.frame(&mut app, vec![press(p, true)]);
    ty.frame(&mut app, vec![press(p, false)]);
    ty.pause(&mut app);
    let picked = chosen(&app);
    assert!(!app.tabs[0].organising(), "the double-click: {picked:?}");
    assert_eq!(app.tabs[0].current, 2);
}

/// What is being typed on a page goes into the document before the grid shows; a tool
/// chosen while organising goes back to the page view, at the page selected.
#[test]
fn typing_is_kept_and_tools_go_back_to_the_page() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = app_with(&ctx, 1);
    start_typing(&mut app, &ty, "new text", (0, 0));
    ty.text(&mut app, "otter");
    let revision = app.tabs[0].revision;
    organise(&mut app, &ty);
    assert!(app.tabs[0].organising());
    let text = app.tabs[0].pdf.page_text(0);
    assert!(text.contains("otter"), "{text:?}");
    assert_eq!(app.tabs[0].revision, revision + 1, "one change");
    app.tabs[0].ed.selection = Selection::pages_from([1].into());
    app.act(&ctx, Act::Tool(Tool::Text));
    assert!(!app.tabs[0].organising(), "still organising");
    assert_eq!(app.tabs[0].ed.tool, Tool::Text);
    assert_eq!(app.tabs[0].current, 1, "not the page selected");
}

/// A long document's grid draws only the cards in view: pictures are asked for and kept
/// for those (and a row or so around), let go when scrolled far away.
#[test]
fn a_long_document_draws_only_the_cards_in_view() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 1500);
    organise(&mut app, &ty);
    let t = &app.tabs[0];
    let near = t.grid_pictures().len() + t.grid_waiting();
    assert!((1..=60).contains(&near), "{near} pictures");
    app.tabs[0].go_to(1499);
    let tree = ty.pause(&mut app);
    assert!(node(&tree, "Page 1500").is_some(), "no last card");
    assert!(node(&tree, "Page 1").is_none(), "the first card drawn");
    let t = &app.tabs[0];
    assert!(t.grid_pictures().len() + t.grid_waiting() <= 60);
    assert!(!t.grid_drawn(0), "the first page's picture is kept");
}

/// The grid zooms on its own: Ctrl+= and Ctrl+−, Ctrl with the wheel and the status bar
/// change the cards' size, not the page view's zoom, and the size is remembered.
#[test]
fn the_grid_has_its_own_zoom() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 4);
    let zoom = app.tabs[0].zoom;
    organise(&mut app, &ty);
    let size = app.settings.organise_zoom;
    ty.key(&mut app, Key::Equals, egui::Modifiers::COMMAND);
    let bigger = app.settings.organise_zoom;
    assert!(bigger > size, "{bigger} after {size}");
    ty.key(&mut app, Key::Minus, egui::Modifiers::COMMAND);
    ty.key(&mut app, Key::Minus, egui::Modifiers::COMMAND);
    assert!(app.settings.organise_zoom < size, "Ctrl+−");
    let tree = ty.pause(&mut app);
    let p = middle(&tree, "Page 1");
    // Ctrl with the wheel, as egui reports it.
    let small = app.settings.organise_zoom;
    let zoom_in = egui::Event::Zoom(1.25);
    ty.frame(&mut app, vec![egui::Event::PointerMoved(p), zoom_in]);
    ty.pause(&mut app);
    assert!(app.settings.organise_zoom > small, "Ctrl with the wheel");
    let wheeled = app.settings.organise_zoom;
    click_as(&mut app, &ty, "Zoom in", egui::Modifiers::NONE);
    assert!(app.settings.organise_zoom > wheeled, "the bar's Zoom in");
    assert_eq!(app.tabs[0].zoom, zoom, "the page view's zoom changed");
}

/// The status bar zooms as a word processor's: − and + buttons and a slider beside the
/// zoom's menu. The slider doesn't change a zoom beyond its ends by being drawn.
#[test]
fn the_status_bar_zooms_with_buttons_and_a_slider() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 2);
    let tree = ty.pause(&mut app);
    for name in ["Zoom out", "Zoom in", "Zoom"] {
        assert!(node(&tree, name).is_some(), "no {name}");
    }
    let zoom = app.tabs[0].zoom;
    click_as(&mut app, &ty, "Zoom in", egui::Modifiers::NONE);
    assert!(app.tabs[0].zoom > zoom, "Zoom in");
    click_as(&mut app, &ty, "Zoom out", egui::Modifiers::NONE);
    click_as(&mut app, &ty, "Zoom out", egui::Modifiers::NONE);
    assert!(app.tabs[0].zoom < zoom, "Zoom out");
    app.tabs[0].set_zoom(40.0);
    ty.pause(&mut app);
    assert_eq!(app.tabs[0].zoom, 40.0, "the slider changed the zoom");
    // Dragged, it zooms.
    let tree = ty.pause(&mut app);
    let (slider, _) = node(&tree, "Zoom").unwrap();
    let (from, to) = (slider.center(), slider.left_center() + egui::vec2(4.0, 0.0));
    ty.frame(&mut app, vec![egui::Event::PointerMoved(from)]);
    ty.frame(&mut app, vec![press(from, true)]);
    ty.frame(&mut app, vec![egui::Event::PointerMoved(to)]);
    ty.frame(&mut app, vec![press(to, false)]);
    ty.pause(&mut app);
    assert!(app.tabs[0].zoom < 1.0, "the slider: {}", app.tabs[0].zoom);
}

/// The side panel's Pages view folds away while organising, as the grid shows the pages,
/// and is back after; its button in the strip goes back to the page view.
#[test]
fn the_pages_view_folds_away_while_organising() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 3);
    app.show_view(Side::Pages);
    let tree = ty.pause(&mut app);
    assert!(shows(&tree, "Pages · 3"), "the Pages view isn't open");
    let tree = organise(&mut app, &ty);
    assert!(!shows(&tree, "Pages · 3"), "the Pages view is open");
    app.act(&ctx, Act::PagesView);
    let tree = ty.pause(&mut app);
    assert!(!app.tabs[0].organising(), "Pages view didn't go back");
    assert!(shows(&tree, "Pages · 3"), "the Pages view isn't back");
}

/// Every control of the grid and the status bar's new ones has a name that says what it is.
#[test]
fn the_grid_s_controls_have_clear_names() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 3);
    organise(&mut app, &ty);
    app.tabs[0].ed.selection = Selection::pages_from([1].into());
    let mut bad = Vec::new();
    clear_names(&mut app, &ctx, "Organise", &mut bad);
    assert_eq!(bad, [""; 0]);
}

/// Clicks one right after another each count: the heading's count changing with the first
/// lays the next frame out twice, and the second click, pressed then, was lost.
#[test]
fn quick_clicks_on_cards_each_count() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    let tree = organise(&mut app, &ty);
    let (a, b) = (middle(&tree, "Page 1"), middle(&tree, "Page 3"));
    let ctrl = |e: egui::Event| vec![egui::Event::ModifiersChanged(egui::Modifiers::COMMAND), e];
    ty.frame(&mut app, vec![egui::Event::PointerMoved(a), press(a, true)]);
    ty.frame(&mut app, vec![press(a, false)]);
    ty.frame(&mut app, ctrl(egui::Event::PointerMoved(b)));
    ty.frame(&mut app, ctrl(press(b, true)));
    ty.frame(&mut app, ctrl(press(b, false)));
    keys_up(&mut app, &ty);
    ty.pause(&mut app);
    assert_eq!(chosen(&app), [0, 2]);
}

// ------------------------------------------------ stage 2: moving pages, keys, cut and paste

/// Alt+Shift, held to move pages with the arrows, as Word moves paragraphs.
const ALT_SHIFT: egui::Modifiers = egui::Modifiers::ALT.plus(egui::Modifiers::SHIFT);

/// Press `key` with `held` keys down, as a keyboard sends them, and let the window settle.
fn key_as(
    app: &mut App,
    ty: &Typist,
    key: Key,
    held: egui::Modifiers,
) -> egui::accesskit::TreeUpdate {
    let down = egui::Event::ModifiersChanged(held);
    ty.frame(app, vec![down, key_with(key, held)]);
    keys_up(app, ty);
    ty.pause(app)
}

/// Where the card named "Page `n`" is on screen.
fn card_at(tree: &egui::accesskit::TreeUpdate, n: usize) -> egui::Rect {
    let name = format!("Page {n}");
    node(tree, &name).unwrap_or_else(|| panic!("no {name}")).0
}

/// How many of `n` cards are in the grid's first row.
fn per_row(tree: &egui::accesskit::TreeUpdate, n: usize) -> usize {
    let top = card_at(tree, 1).min.y;
    (1..=n)
        .take_while(|&i| card_at(tree, i).min.y == top)
        .count()
}

/// A point in the gap before card `n`: half way between it and the card before, at their
/// middle height, or in the margin left of it when it starts a row.
fn gap_before(tree: &egui::accesskit::TreeUpdate, n: usize) -> egui::Pos2 {
    let after = card_at(tree, n);
    let margin = egui::pos2(after.min.x - 6.0, after.center().y);
    if n == 1 {
        return margin;
    }
    let before = card_at(tree, n - 1);
    match before.min.y == after.min.y {
        true => egui::pos2((before.max.x + after.min.x) / 2.0, after.center().y),
        false => margin,
    }
}

/// Click at `p` as people click, the clock moving on (see [`click_as`]).
fn click_on(app: &mut App, ty: &Typist, p: egui::Pos2) -> egui::accesskit::TreeUpdate {
    ty.frame(app, vec![egui::Event::PointerMoved(p)]);
    ty.frame(app, vec![press(p, true)]);
    ty.frame(app, vec![press(p, false)]);
    ty.pause(app)
}

/// Right-click at `p`, and let the window settle.
fn right_click(app: &mut App, ty: &Typist, p: egui::Pos2) -> egui::accesskit::TreeUpdate {
    let button = |pressed| egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: Default::default(),
    };
    ty.frame(app, vec![egui::Event::PointerMoved(p)]);
    ty.frame(app, vec![button(true)]);
    ty.frame(app, vec![button(false)]);
    ty.pause(app)
}

/// Move the pointer from `from` to `to` in steps, as a hand moves it.
fn move_to(app: &mut App, ty: &Typist, from: egui::Pos2, to: egui::Pos2) {
    for i in 1..=6 {
        let p = from + (to - from) * (i as f32 / 6.0);
        ty.frame(app, vec![egui::Event::PointerMoved(p)]);
    }
    ty.frame(app, vec![]);
}

/// Press at `from` and move to `to` with the button held: a drag not yet let go.
fn drag_to(app: &mut App, ty: &Typist, from: egui::Pos2, to: egui::Pos2) {
    ty.frame(app, vec![egui::Event::PointerMoved(from)]);
    ty.frame(app, vec![press(from, true)]);
    move_to(app, ty, from, to);
}

/// Let go of the button at `p`, and let the window settle.
fn let_go(app: &mut App, ty: &Typist, p: egui::Pos2) {
    ty.frame(app, vec![press(p, false)]);
    ty.pause(app);
}

/// The middle of the biggest control whose name starts with `text`: a menu's items have
/// their keys after their names.
fn named_like(tree: &egui::accesskit::TreeUpdate, text: &str) -> Option<egui::Pos2> {
    let area = |n: &egui::accesskit::Node| n.bounds().map_or(0.0, |r| r.width() * r.height());
    let (_, n) = (tree.nodes.iter())
        .filter(|(_, n)| n.label().is_some_and(|l| l.starts_with(text)))
        .max_by(|(_, a), (_, b)| area(a).total_cmp(&area(b)))?;
    let r = n.bounds()?;
    let mid = |a: f64, b: f64| ((a + b) / 2.0) as f32;
    Some(egui::pos2(mid(r.x0, r.x1), mid(r.y0, r.y1)))
}

/// Where the page that was `page` when the tab was made is now.
fn place_of(app: &App, first: &[(u32, u16)], page: usize) -> usize {
    let now = order(app, first);
    now.iter().position(|&p| p == page).expect("the page")
}

/// The pages cut, waiting to be pasted, 0-based.
fn cut(app: &App) -> Vec<usize> {
    app.tabs[app.active].cut_pages().into_iter().collect()
}

/// The steps Undo can take back in the active tab.
fn steps(app: &App) -> usize {
    app.tabs[app.active].history().1
}

/// The grid's pictures once every card in view has its picture.
fn pictures_drawn(app: &mut App, ty: &Typist) -> Vec<egui::TextureId> {
    for _ in 0..200 {
        ty.frame(app, vec![]);
        if app.tabs[0].grid_waiting() == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(app.tabs[0].grid_waiting(), 0, "pictures still coming");
    app.tabs[0].grid_pictures()
}

/// A card dragged to the gap between two others moves its page there, as a slide sorter
/// moves slides; a card not selected moves alone, the pages selected staying. While it is
/// dragged the grid shows where it would land; let go, the page is there and selected, in
/// one step Undo takes back.
#[test]
fn dragging_a_card_moves_its_page_where_it_is_let_go() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 12);
    let first = app.tabs[0].pdf.page_ids();
    let all: Vec<usize> = (0..12).collect();
    organise(&mut app, &ty);
    click_as(&mut app, &ty, "Page 8", egui::Modifiers::NONE);
    let tree = ty.pause(&mut app);
    let (from, to) = (middle(&tree, "Page 2"), gap_before(&tree, 6));
    drag_to(&mut app, &ty, from, to);
    assert_eq!(app.tabs[0].grid_drop(), Some(5), "where it would land");
    assert_eq!(order(&app, &first), all, "moved before it was let go");
    let_go(&mut app, &ty, to);
    assert_eq!(order(&app, &first), [0, 2, 3, 4, 1, 5, 6, 7, 8, 9, 10, 11]);
    assert_eq!(chosen(&app), [4], "not the page moved, where it is now");
    assert_eq!(app.tabs[0].grid_drop(), None);
    app.act(&ctx, Act::Undo);
    assert_eq!(order(&app, &first), all, "not one step");
}

/// A selected card dragged takes every page selected with it, in their order, and they
/// stay selected where they land.
#[test]
fn dragging_a_selected_card_moves_every_page_selected() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 12);
    let first = app.tabs[0].pdf.page_ids();
    organise(&mut app, &ty);
    click_as(&mut app, &ty, "Page 1", egui::Modifiers::NONE);
    click_as(&mut app, &ty, "Page 3", egui::Modifiers::COMMAND);
    assert_eq!(chosen(&app), [0, 2]);
    let tree = ty.pause(&mut app);
    let (from, to) = (middle(&tree, "Page 3"), gap_before(&tree, 8));
    drag_to(&mut app, &ty, from, to);
    assert_eq!(app.tabs[0].grid_drop(), Some(7), "where they would land");
    let_go(&mut app, &ty, to);
    assert_eq!(order(&app, &first), [1, 3, 4, 5, 6, 0, 2, 7, 8, 9, 10, 11]);
    assert_eq!(chosen(&app), [5, 6], "not the pages moved");
}

/// Esc while cards are dragged, or letting go of them outside the grid, leaves every page
/// where it was.
#[test]
fn esc_or_letting_go_outside_the_grid_cancels_a_drag() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 12);
    let first = app.tabs[0].pdf.page_ids();
    let all: Vec<usize> = (0..12).collect();
    let tree = organise(&mut app, &ty);
    let (from, to) = (middle(&tree, "Page 2"), gap_before(&tree, 6));
    drag_to(&mut app, &ty, from, to);
    assert_eq!(app.tabs[0].grid_drop(), Some(5), "not dragging");
    ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
    assert_eq!(app.tabs[0].grid_drop(), None, "Esc");
    let past = to + egui::vec2(20.0, 0.0);
    move_to(&mut app, &ty, to, past);
    let_go(&mut app, &ty, past);
    assert_eq!(order(&app, &first), all, "moved after Esc");
    // Let go over the strip at the window's left edge.
    let tree = ty.pause(&mut app);
    let (from, to) = (middle(&tree, "Page 2"), gap_before(&tree, 6));
    drag_to(&mut app, &ty, from, to);
    assert_eq!(app.tabs[0].grid_drop(), Some(5), "not dragging again");
    let away = egui::pos2(2.0, to.y);
    move_to(&mut app, &ty, to, away);
    let_go(&mut app, &ty, away);
    assert_eq!(order(&app, &first), all, "moved when let go outside");
    assert_eq!(app.tabs[0].grid_drop(), None);
    let t = &app.tabs[0];
    assert!(!t.dirty && t.revision == 0, "the document changed");
}

/// The grid is driven with the keys as a file manager's: the arrows go from card to card
/// selecting each, up and down a row at a time; Shift selects from the card selected to
/// the one gone to; Ctrl moves without selecting, and Space then adds the card or takes it
/// out; Home and End go to the first and last card, Page Down a screen of rows down, as far
/// as the last row; Enter opens the page. None of it changes the document.
#[test]
fn the_keys_go_from_card_to_card_selecting_them() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let n = 40;
    let mut app = grid_tab(&ctx, n);
    let tree = organise(&mut app, &ty);
    let cols = per_row(&tree, n);
    assert!((2..=8).contains(&cols), "{cols} cards in a row");
    let none = egui::Modifiers::NONE;
    let at = |app: &App| app.tabs[0].grid_focus();
    assert_eq!(at(&app), 0, "not at the page being read");
    key_as(&mut app, &ty, Key::ArrowRight, none);
    assert_eq!((chosen(&app), at(&app)), (vec![1], 1), "Right");
    key_as(&mut app, &ty, Key::ArrowDown, none);
    let below = 1 + cols;
    assert_eq!((chosen(&app), at(&app)), (vec![below], below), "Down");
    key_as(&mut app, &ty, Key::ArrowRight, egui::Modifiers::SHIFT);
    assert_eq!(chosen(&app), [below, below + 1], "Shift+Right");
    key_as(&mut app, &ty, Key::ArrowRight, egui::Modifiers::COMMAND);
    assert_eq!(at(&app), below + 2, "Ctrl+Right");
    assert_eq!(chosen(&app), [below, below + 1], "Ctrl+Right selected");
    // Space as a keyboard sends it: the key, and a space typed.
    let space = vec![key_with(Key::Space, none), egui::Event::Text(" ".into())];
    ty.frame(&mut app, space);
    ty.pause(&mut app);
    assert_eq!(chosen(&app), [below, below + 1, below + 2], "Space");
    key_as(&mut app, &ty, Key::Space, egui::Modifiers::COMMAND);
    assert_eq!(chosen(&app), [below, below + 1], "Ctrl+Space");
    let tree = key_as(&mut app, &ty, Key::End, none);
    assert_eq!((chosen(&app), at(&app)), (vec![n - 1], n - 1), "End");
    let last = format!("Page {n}");
    assert!(node(&tree, &last).is_some(), "no last card");
    let tree = key_as(&mut app, &ty, Key::Home, none);
    assert_eq!((chosen(&app), at(&app)), (vec![0], 0), "Home");
    assert!(node(&tree, "Page 1").is_some(), "no first card");
    // Page Down: a screen of rows at a time, as far as the last row.
    let mut was = 0;
    for _ in 0..10 {
        let tree = key_as(&mut app, &ty, Key::PageDown, none);
        let now = at(&app);
        assert!((was..n).contains(&now), "Page Down from {was} to {now}");
        if was == 0 {
            assert!(now >= cols && now % cols == 0, "the first Page Down: {now}");
        }
        assert_eq!(chosen(&app), [now], "Page Down");
        let card = format!("Page {}", now + 1);
        assert!(node(&tree, &card).is_some(), "{card} isn't shown");
        was = now;
    }
    assert!(was >= (n - 1) / cols * cols, "Page Down stopped at {was}");
    key_as(&mut app, &ty, Key::Home, none);
    key_as(&mut app, &ty, Key::PageDown, none);
    let page = at(&app);
    let t = &app.tabs[0];
    assert!(!t.dirty && t.revision == 0, "the keys changed the document");
    key_as(&mut app, &ty, Key::Enter, none);
    assert!(!app.tabs[0].organising(), "Enter didn't open the page");
    assert_eq!(app.tabs[0].current, page, "not the card's page");
}

/// Delete takes out the pages selected with the grid's keys; one Undo brings them back.
#[test]
fn delete_takes_out_the_pages_selected_with_the_keys() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 12);
    let first = app.tabs[0].pdf.page_ids();
    organise(&mut app, &ty);
    let none = egui::Modifiers::NONE;
    key_as(&mut app, &ty, Key::ArrowRight, none);
    key_as(&mut app, &ty, Key::ArrowRight, egui::Modifiers::SHIFT);
    assert_eq!(chosen(&app), [1, 2]);
    key_as(&mut app, &ty, Key::Delete, none);
    assert_eq!(order(&app, &first), [0, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    app.act(&ctx, Act::Undo);
    let all: Vec<usize> = (0..12).collect();
    assert_eq!(order(&app, &first), all, "not one step");
}

/// Tab reaches the grid, one stop for all its cards; there the arrows go from card to
/// card, the keyboard staying in the grid, and Esc lets go of the pages selected, not of
/// the keyboard.
#[test]
fn tab_reaches_the_grid_and_its_keys_stay_there() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 12);
    let tree = organise(&mut app, &ty);
    let cols = per_row(&tree, 12);
    let grid = app.tabs[0].grid_id();
    let focused = || ctx.memory(|m| m.focused());
    let none = egui::Modifiers::NONE;
    let mut presses = 0;
    while focused() != Some(grid) {
        presses += 1;
        assert!(presses <= 150, "Tab didn't reach the grid");
        ty.key(&mut app, Key::Tab, none);
    }
    let tree = ty.pause(&mut app);
    assert_eq!(focused_name(&ctx, &tree).as_deref(), Some("Page grid"));
    key_as(&mut app, &ty, Key::ArrowRight, none);
    assert_eq!(chosen(&app), [1], "Right");
    assert_eq!(focused(), Some(grid), "Right left the grid");
    key_as(&mut app, &ty, Key::ArrowDown, none);
    assert_eq!(chosen(&app), [1 + cols], "Down");
    key_as(&mut app, &ty, Key::ArrowLeft, none);
    key_as(&mut app, &ty, Key::ArrowUp, none);
    assert_eq!(chosen(&app), [0], "Left and Up");
    assert_eq!(focused(), Some(grid), "the arrows left the grid");
    key_as(&mut app, &ty, Key::Escape, none);
    assert_eq!(chosen(&app), [0usize; 0], "Esc");
    assert_eq!(focused(), Some(grid), "Esc left the grid");
    assert!(app.tabs[0].organising(), "Esc left Organise");
}

/// Alt+Shift with an arrow moves the pages selected, as Word moves paragraphs: a place
/// earlier or later, a row up or down, with Home and End to the start and the end. The
/// pages stay selected, and each move is one step for Undo. With no pages selected the
/// keys do nothing.
#[test]
fn alt_shift_and_an_arrow_move_the_pages_selected() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let n = 12;
    let mut app = grid_tab(&ctx, n);
    let first = app.tabs[0].pdf.page_ids();
    let tree = organise(&mut app, &ty);
    let cols = per_row(&tree, n);
    assert!((2..=8).contains(&cols), "{cols} cards in a row");
    click_as(&mut app, &ty, "Page 3", egui::Modifiers::NONE);
    let go = |app: &mut App, key: Key, to: usize, what: &str| {
        let before = steps(app);
        key_as(app, &ty, key, ALT_SHIFT);
        assert_eq!(place_of(app, &first, 2), to, "{what}");
        assert_eq!(chosen(app), [to], "{what}: not selected");
        assert_eq!(steps(app), before + 1, "{what}: not one step");
    };
    go(&mut app, Key::ArrowLeft, 1, "Alt+Shift+Left");
    assert_eq!(order(&app, &first)[..3], [0, 2, 1]);
    go(&mut app, Key::ArrowRight, 2, "Alt+Shift+Right");
    go(&mut app, Key::ArrowRight, 3, "Alt+Shift+Right again");
    go(&mut app, Key::ArrowDown, 3 + cols, "Alt+Shift+Down");
    go(&mut app, Key::ArrowUp, 3, "Alt+Shift+Up");
    go(&mut app, Key::Home, 0, "Alt+Shift+Home");
    go(&mut app, Key::End, n - 1, "Alt+Shift+End");
    app.act(&ctx, Act::Undo);
    assert_eq!(place_of(&app, &first, 2), 0, "Undo took back more");
    // No pages selected: nothing moves.
    app.tabs[0].ed.selection = Selection::None;
    let (history, now) = (app.tabs[0].history(), order(&app, &first));
    for key in [Key::ArrowLeft, Key::ArrowRight, Key::ArrowDown, Key::End] {
        key_as(&mut app, &ty, key, ALT_SHIFT);
    }
    assert_eq!(app.tabs[0].history(), history, "none selected");
    assert_eq!(order(&app, &first), now);
}

/// The Move commands are in the Pages menu's Move pages, with Alt+Shift and an arrow for
/// keys; in the Pages band's Move menu; and in the right-click menu of the pages selected,
/// with Insert PDF pages.
#[test]
fn the_move_commands_are_in_the_menus_and_the_band() {
    use actions::PageMove as M;
    let moves = [
        (M::Earlier, "Earlier", Key::ArrowLeft),
        (M::Later, "Later", Key::ArrowRight),
        (M::Up, "Up a row", Key::ArrowUp),
        (M::Down, "Down a row", Key::ArrowDown),
        (M::Start, "To the start", Key::Home),
        (M::End, "To the end", Key::End),
    ];
    for (to, label, key) in moves {
        let a = actions::get(Act::MovePages(to));
        assert_eq!((a.label, a.submenu), (label, "Move pages"));
        assert_eq!(a.group, actions::Group::Pages, "{label}");
        let keys = egui::KeyboardShortcut::new(ALT_SHIFT, key);
        assert!(a.keys.contains(&keys), "{label}: {:?}", a.keys);
    }
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    let first = app.tabs[0].pdf.page_ids();
    app.band = Band::Pages;
    organise(&mut app, &ty);
    click_as(&mut app, &ty, "Page 2", egui::Modifiers::NONE);
    click_as(&mut app, &ty, "Move", egui::Modifiers::NONE);
    let tree = ty.pause(&mut app);
    let four = ["Earlier", "Later", "To the start", "To the end"];
    for item in four {
        let shown = named_like(&tree, item).is_some();
        assert!(shown, "no {item} in the band's Move");
    }
    click_on(&mut app, &ty, named_like(&tree, "To the end").unwrap());
    assert_eq!(order(&app, &first), [0, 2, 3, 4, 5, 1]);
    assert_eq!(chosen(&app), [5], "not the page moved");
    // The right-click menu of the page selected.
    let tree = ty.pause(&mut app);
    let tree = right_click(&mut app, &ty, middle(&tree, "Page 6"));
    assert_eq!(chosen(&app), [5], "the right-click changed the selection");
    let pdf = named_like(&tree, "Insert PDF pages");
    assert!(pdf.is_some(), "no Insert PDF pages in the right-click menu");
    let sub = named_like(&tree, "Move pages").expect("no Move pages in the right-click menu");
    let tree = click_on(&mut app, &ty, sub);
    for item in four {
        let shown = named_like(&tree, item).is_some();
        assert!(shown, "no {item} in the right-click menu");
    }
}

/// Ctrl+X cuts the pages selected, to paste elsewhere, as a file manager cuts files: the
/// document is unchanged, the clipboard gets something; Ctrl+V with another page selected
/// puts them after it, in their order and selected, in one step Undo takes back.
#[test]
fn cut_and_paste_move_pages() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 12);
    let first = app.tabs[0].pdf.page_ids();
    organise(&mut app, &ty);
    click_as(&mut app, &ty, "Page 2", egui::Modifiers::NONE);
    click_as(&mut app, &ty, "Page 3", egui::Modifiers::SHIFT);
    assert_eq!(chosen(&app), [1, 2]);
    let copied = copied_by(&mut app, &ctx, vec![egui::Event::Cut]);
    ty.pause(&mut app);
    assert_eq!(cut(&app), [1, 2]);
    assert!(!copied.is_empty(), "nothing on the clipboard");
    let t = &app.tabs[0];
    assert!(!t.dirty && t.revision == 0, "Cut changed the document");
    assert_eq!(chosen(&app), [1, 2], "Cut changed the selection");
    click_as(&mut app, &ty, "Page 8", egui::Modifiers::NONE);
    ty.frame(&mut app, vec![egui::Event::Paste("x".into())]);
    ty.pause(&mut app);
    assert_eq!(order(&app, &first), [0, 3, 4, 5, 6, 7, 1, 2, 8, 9, 10, 11]);
    assert_eq!(chosen(&app), [6, 7], "not the pages pasted");
    assert_eq!(cut(&app), [0usize; 0], "still cut");
    app.act(&ctx, Act::Undo);
    let all: Vec<usize> = (0..12).collect();
    assert_eq!(order(&app, &first), all, "not one step");
}

/// Esc after Ctrl+X takes back the cut, the pages staying selected; Esc again lets go of
/// them.
#[test]
fn esc_takes_back_a_cut_then_the_selection() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    organise(&mut app, &ty);
    app.tabs[0].ed.selection = Selection::pages_from([1, 2].into());
    copied_by(&mut app, &ctx, vec![egui::Event::Cut]);
    ty.pause(&mut app);
    assert_eq!(cut(&app), [1, 2]);
    let none = egui::Modifiers::NONE;
    key_as(&mut app, &ty, Key::Escape, none);
    assert_eq!(cut(&app), [0usize; 0], "Esc");
    assert_eq!(chosen(&app), [1, 2], "Esc let go of the pages too");
    key_as(&mut app, &ty, Key::Escape, none);
    assert_eq!(chosen(&app), [0usize; 0], "Esc again");
    assert!(app.tabs[0].organising(), "Esc left Organise");
}

/// Ctrl+V with the pages cut still selected, no other place chosen, leaves them where
/// they are and says what to do.
#[test]
fn paste_onto_the_pages_cut_changes_nothing_and_says_so() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    let first = app.tabs[0].pdf.page_ids();
    organise(&mut app, &ty);
    app.tabs[0].ed.selection = Selection::pages_from([1, 2].into());
    copied_by(&mut app, &ctx, vec![egui::Event::Cut]);
    ty.pause(&mut app);
    assert_eq!(cut(&app), [1, 2]);
    app.toasts.clear();
    let history = app.tabs[0].history();
    ty.frame(&mut app, vec![egui::Event::Paste("x".into())]);
    ty.frame(&mut app, vec![]);
    let said = toast(&app).expect("nothing said");
    assert!(!said.contains("typed"), "{said}");
    assert_eq!(app.tabs[0].history(), history, "Paste changed the document");
    assert_eq!(order(&app, &first), [0, 1, 2, 3, 4, 5]);
}

/// Paste greyed while organising says to cut pages first; in the page view, where to
/// click, as before. Its tooltip names its key as the system writes it (PB-36), and its
/// hint no longer spells out Ctrl+V.
#[test]
fn paste_says_to_cut_pages_first_while_organising() {
    for os in KEY_STYLES {
        paste_says_to_cut_pages_first_on(os);
    }
}

fn paste_says_to_cut_pages_first_on(os: egui::os::OperatingSystem) {
    let ctx = test_ctx();
    ctx.set_os(os);
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 3);
    let paste = actions::get(Act::Paste);
    organise(&mut app, &ty);
    let why = actions::why_not(&app.state(&ctx), paste);
    assert_eq!(why, "Cut pages first.", "{os:?}: organising");
    app.act(&ctx, Act::Organise);
    ty.pause(&mut app);
    let why = actions::why_not(&app.state(&ctx), paste);
    let typed = "Click in text being typed, or where new text goes, first.";
    assert_eq!(why, typed, "{os:?}: the page view");
    let key = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, Key::V);
    let tip = actions::tooltip(&ctx, paste);
    assert!(tip.contains(&ctx.format_shortcut(&key)), "{os:?}: {tip}");
    assert!(!paste.hint.contains("Ctrl+V"), "{}", paste.hint);
}

/// A click between cards puts the insertion bar there, as a slide sorter's, letting go of
/// the pages selected: the place new pages go. Insert blank page puts its page there, the
/// size of the page before, selected, and the bar goes. A click before the first card puts
/// the bar at the start; a click on a card, or Esc, takes it away.
#[test]
fn a_click_between_cards_shows_where_new_pages_go() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 12);
    let tree = organise(&mut app, &ty);
    assert!(per_row(&tree, 12) >= 4, "cards 3 and 4 aren't side by side");
    click_as(&mut app, &ty, "Page 2", egui::Modifiers::NONE);
    assert_eq!(app.tabs[0].grid_bar(), None);
    let tree = ty.pause(&mut app);
    click_on(&mut app, &ty, gap_before(&tree, 4));
    assert_eq!(app.tabs[0].grid_bar(), Some(3), "between cards 3 and 4");
    assert_eq!(chosen(&app), [0usize; 0], "the pages stayed selected");
    app.act(&ctx, Act::Blank);
    let w = widths(&app, 0);
    assert_eq!(w.len(), 13, "no page added");
    let at_bar = [400.0, 401.0, 402.0, 402.0, 403.0];
    assert_eq!(w[..5], at_bar, "not at the bar, the size before");
    assert_eq!(chosen(&app), [3], "the new page isn't selected");
    assert_eq!(app.tabs[0].grid_bar(), None, "the bar stayed");
    // Before the first card: the start.
    let tree = ty.pause(&mut app);
    click_on(&mut app, &ty, gap_before(&tree, 1));
    assert_eq!(app.tabs[0].grid_bar(), Some(0), "before the first card");
    click_as(&mut app, &ty, "Page 5", egui::Modifiers::NONE);
    assert_eq!(app.tabs[0].grid_bar(), None, "a click on a card");
    assert_eq!(chosen(&app), [4]);
    let tree = ty.pause(&mut app);
    click_on(&mut app, &ty, gap_before(&tree, 4));
    assert_eq!(app.tabs[0].grid_bar(), Some(3), "again");
    key_as(&mut app, &ty, Key::Escape, egui::Modifiers::NONE);
    assert_eq!(app.tabs[0].grid_bar(), None, "Esc");
}

/// Pages cut go where the insertion bar is when pasted; a click between cards keeps them
/// cut.
#[test]
fn pages_cut_are_pasted_at_the_insertion_bar() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 12);
    let first = app.tabs[0].pdf.page_ids();
    organise(&mut app, &ty);
    click_as(&mut app, &ty, "Page 1", egui::Modifiers::NONE);
    copied_by(&mut app, &ctx, vec![egui::Event::Cut]);
    let tree = ty.pause(&mut app);
    assert_eq!(cut(&app), [0]);
    click_on(&mut app, &ty, gap_before(&tree, 7));
    assert_eq!(app.tabs[0].grid_bar(), Some(6), "between cards 6 and 7");
    assert_eq!(cut(&app), [0], "the click took back the cut");
    ty.frame(&mut app, vec![egui::Event::Paste("x".into())]);
    ty.pause(&mut app);
    assert_eq!(order(&app, &first), [1, 2, 3, 4, 5, 0, 6, 7, 8, 9, 10, 11]);
    assert_eq!(chosen(&app), [5], "not the page pasted");
}

/// Insert PDF pages, in the Pages band's Insert pages, puts the pages of another PDF
/// after the page selected, or at the insertion bar, selected; one Undo takes them out.
#[test]
fn insert_pdf_pages_go_after_the_selection_or_at_the_bar() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let dir = test_dir("insert-pdf-pages");
    let file = dir.join("two.pdf");
    let mut two = Pdf::new();
    two.insert_blank(0, 300.0, 500.0).unwrap();
    two.insert_blank(1, 301.0, 500.0).unwrap();
    two.save(&file).unwrap();
    let mut app = grid_tab(&ctx, 6);
    let start = widths(&app, 0);
    app.band = Band::Pages;
    organise(&mut app, &ty);
    click_as(&mut app, &ty, "Insert pages", egui::Modifiers::NONE);
    let tree = ty.pause(&mut app);
    let named = named_like(&tree, "Insert PDF pages");
    assert!(named.is_some(), "not in the band's Insert pages");
    key_as(&mut app, &ty, Key::Escape, egui::Modifiers::NONE);
    click_as(&mut app, &ty, "Page 2", egui::Modifiers::NONE);
    app.insert_files(&ctx, std::slice::from_ref(&file));
    wait_for(&mut app, &ctx, |a| !a.busy);
    ty.pause(&mut app);
    let after = [400.0, 401.0, 300.0, 301.0, 402.0, 403.0, 404.0, 405.0];
    assert_eq!(widths(&app, 0), after, "not after the page selected");
    assert_eq!(chosen(&app), [2, 3], "the pages inserted aren't selected");
    app.act(&ctx, Act::Undo);
    assert_eq!(widths(&app, 0), start, "not one step");
    // At the bar, before the first card.
    let tree = ty.pause(&mut app);
    click_on(&mut app, &ty, gap_before(&tree, 1));
    assert_eq!(app.tabs[0].grid_bar(), Some(0));
    app.insert_files(&ctx, std::slice::from_ref(&file));
    wait_for(&mut app, &ctx, |a| !a.busy);
    ty.pause(&mut app);
    let w = widths(&app, 0);
    assert_eq!(w[..3], [300.0, 301.0, 400.0], "not at the bar");
    assert_eq!(chosen(&app), [0, 1], "the pages inserted aren't selected");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The thumbnails beside the page view are one keyboard stop, not one for each page; there
/// the arrows select the next page and the page view goes to it, Shift selects from the
/// page selected, Alt+Shift moves the page selected (a row being one page), and Enter
/// shows the page.
#[test]
fn the_thumbnails_are_one_keyboard_stop_driven_with_the_keys() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    let first = app.tabs[0].pdf.page_ids();
    app.show_view(Side::Pages);
    let tree = ty.pause(&mut app);
    let (_, list) = (tree.nodes.iter())
        .find(|(_, n)| n.label() == Some("Page thumbnails"))
        .expect("no Page thumbnails");
    let focus = egui::accesskit::Action::Focus;
    assert!(list.supports_action(focus), "the thumbnails aren't a stop");
    let b = list.bounds().expect("where the thumbnails are");
    let inside = |n: &egui::accesskit::Node| {
        n.bounds().is_some_and(|r| {
            let (x, y) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
            (b.x0..=b.x1).contains(&x) && (b.y0..=b.y1).contains(&y)
        })
    };
    let page = |l: &str| l.starts_with("Page ") && l[5..].parse::<usize>().is_ok();
    let thumbs: Vec<_> = (tree.nodes.iter())
        .filter(|(_, n)| n.label().is_some_and(page) && inside(n))
        .collect();
    assert!(!thumbs.is_empty(), "no thumbnails");
    let stops = thumbs.iter().any(|(_, n)| n.supports_action(focus));
    assert!(!stops, "a thumbnail is a stop of its own");
    let id = app.tabs[0].list_id();
    ctx.memory_mut(|m| m.request_focus(id));
    ty.pause(&mut app);
    let focused = || ctx.memory(|m| m.focused());
    assert_eq!(focused(), Some(id), "not the thumbnails");
    let none = egui::Modifiers::NONE;
    key_as(&mut app, &ty, Key::ArrowDown, none);
    assert_eq!(chosen(&app), [1], "Down");
    assert_eq!(app.tabs[0].current, 1, "the page view didn't go to it");
    key_as(&mut app, &ty, Key::ArrowDown, egui::Modifiers::SHIFT);
    assert_eq!(chosen(&app), [1, 2], "Shift+Down");
    key_as(&mut app, &ty, Key::ArrowDown, none);
    assert_eq!(chosen(&app), [3], "Down again");
    assert_eq!(app.tabs[0].current, 3, "the page view didn't go to it");
    key_as(&mut app, &ty, Key::ArrowUp, ALT_SHIFT);
    assert_eq!(order(&app, &first), [0, 1, 3, 2, 4, 5], "Alt+Shift+Up");
    assert_eq!(chosen(&app), [2], "the page moved isn't selected");
    key_as(&mut app, &ty, Key::Enter, none);
    assert_eq!(app.tabs[0].current, 2, "Enter");
    assert!(!app.tabs[0].organising(), "Enter organised");
    assert_eq!(focused(), Some(id), "the keys left the thumbnails");
}

/// The pictures of pages moved are kept through Undo and Redo, by the page, not drawn
/// again; a turned page turned back by Undo has its picture drawn again, not the turned
/// one kept.
#[test]
fn pictures_are_kept_through_undo_and_redo_of_moves() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    organise(&mut app, &ty);
    let before = pictures_drawn(&mut app, &ty);
    assert_eq!(before.len(), 6, "{before:?}");
    app.tabs[0].ed.selection = Selection::pages_from([1].into());
    key_as(&mut app, &ty, Key::ArrowRight, ALT_SHIFT);
    let moved: Vec<_> = [0, 2, 1, 3, 4, 5].iter().map(|&i| before[i]).collect();
    let after = pictures_drawn(&mut app, &ty);
    assert_eq!(after, moved, "pictures drawn again for the move");
    app.act(&ctx, Act::Undo);
    assert_eq!(app.tabs[0].grid_pictures(), before, "Undo");
    app.act(&ctx, Act::Redo);
    assert_eq!(app.tabs[0].grid_pictures(), after, "Redo");
    // A page turned, then turned back by Undo: its picture drawn again.
    app.tabs[0].ed.selection = Selection::pages_from([2].into());
    app.act(&ctx, Act::RotR);
    let turned = pictures_drawn(&mut app, &ty);
    assert_eq!(turned.len(), 6, "{turned:?}");
    assert_ne!(turned[2], after[2], "the turned page's picture kept");
    app.act(&ctx, Act::Undo);
    let again = pictures_drawn(&mut app, &ty);
    assert_eq!(again.len(), 6, "{again:?}");
    assert_ne!(again[2], turned[2], "the turned picture kept after Undo");
    let others = |p: &[egui::TextureId]| [p[0], p[1], p[3], p[4], p[5]];
    assert_eq!(others(&again), others(&turned), "the others drawn again");
}

/// Every control has a name that says what it is with pages cut and the insertion bar
/// showing, the Pages band's new ones too, and the thumbnails' stop.
#[test]
fn the_grid_s_controls_have_clear_names_with_pages_cut_and_the_bar() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    app.band = Band::Pages;
    organise(&mut app, &ty);
    app.tabs[0].ed.selection = Selection::pages_from([1].into());
    copied_by(&mut app, &ctx, vec![egui::Event::Cut]);
    let tree = ty.pause(&mut app);
    click_on(&mut app, &ty, gap_before(&tree, 4));
    assert_eq!(cut(&app), [1], "not cut");
    assert!(app.tabs[0].grid_bar().is_some(), "no insertion bar");
    let mut bad = Vec::new();
    clear_names(&mut app, &ctx, "Organise with pages cut", &mut bad);
    app.act(&ctx, Act::Organise);
    app.show_view(Side::Pages);
    clear_names(&mut app, &ctx, "The thumbnails", &mut bad);
    assert_eq!(bad, [""; 0]);
}

/// Where thumbnail "Page `page`" is on screen: the one of that name among the thumbnails,
/// not the page of the page view.
fn thumb_at(tree: &egui::accesskit::TreeUpdate, page: usize) -> egui::Rect {
    let rect = |b: egui::accesskit::Rect| {
        let (min, max) = ((b.x0 as f32, b.y0 as f32), (b.x1 as f32, b.y1 as f32));
        egui::Rect::from_min_max(min.into(), max.into())
    };
    let list = (tree.nodes.iter())
        .find(|(_, n)| n.label() == Some("Page thumbnails"))
        .and_then(|(_, n)| n.bounds())
        .map(rect)
        .expect("no Page thumbnails");
    let name = format!("Page {page}");
    (tree.nodes.iter())
        .filter(|(_, n)| n.label() == Some(name.as_str()))
        .filter_map(|(_, n)| n.bounds().map(rect))
        .find(|r| list.contains(r.center()))
        .unwrap_or_else(|| panic!("no thumbnail {name}"))
}

/// Esc while a thumbnail is dragged beside the page view leaves the pages where they were;
/// the same drag let go moves the page.
#[test]
fn esc_cancels_a_thumbnail_drag() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    let first = app.tabs[0].pdf.page_ids();
    app.show_view(Side::Pages);
    let tree = ty.pause(&mut app);
    let (one, two, three) = (thumb_at(&tree, 1), thumb_at(&tree, 2), thumb_at(&tree, 3));
    let from = one.center();
    // Between the second and the third: before the third.
    let to = egui::pos2(from.x, (two.max.y + three.min.y) / 2.0);
    drag_to(&mut app, &ty, from, to);
    ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
    let_go(&mut app, &ty, to);
    assert_eq!(order(&app, &first), [0, 1, 2, 3, 4, 5], "moved after Esc");
    let t = &app.tabs[0];
    assert!(!t.dirty && t.revision == 0, "the document changed");
    // The same drag, let go.
    ty.pause(&mut app);
    drag_to(&mut app, &ty, from, to);
    let_go(&mut app, &ty, to);
    assert_eq!(order(&app, &first), [1, 0, 2, 3, 4, 5], "not moved");
}

/// Cut puts the names of the pages cut on the system's clipboard, so that the paste keys
/// work where the clipboard was empty.
#[test]
fn cut_puts_the_names_of_the_pages_on_the_clipboard() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    organise(&mut app, &ty);
    app.tabs[0].ed.selection = Selection::pages_from([1, 2].into());
    let copied = copied_by(&mut app, &ctx, vec![egui::Event::Cut]);
    let names = format!("Pages 2, 3 of {}", app.tabs[0].name());
    assert_eq!(copied, [names]);
    assert_eq!(cut(&app), [1, 2]);
}

/// Insert PDF pages from a file with form fields asks first, as Append PDFs does; once
/// answered, its page goes after the page selected, and is selected.
#[test]
fn insert_pdf_pages_with_form_fields_go_where_asked_once_answered() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let dir = test_dir("insert-pdf-form");
    let file = dir.join("form.pdf");
    form_file(&file, "Ada");
    let mut app = grid_tab(&ctx, 6);
    organise(&mut app, &ty);
    click_as(&mut app, &ty, "Page 2", egui::Modifiers::NONE);
    app.insert_files(&ctx, std::slice::from_ref(&file));
    wait_for(&mut app, &ctx, |a| !a.busy);
    let asked = matches!(app.dialog, Dialog::ConfirmAppendForms(_));
    assert!(asked, "not asked about the form fields");
    assert_eq!(widths(&app, 0).len(), 6, "added before the answer");
    click(&mut app, &ctx, "Add pages");
    assert!(matches!(app.dialog, Dialog::None), "the question stayed");
    // The dialog's clicks moved egui's clock on: the typist's goes on from there.
    ty.time.set(ctx.input(|i| i.time));
    ty.pause(&mut app);
    let after = [400.0, 401.0, 612.0, 402.0, 403.0, 404.0, 405.0];
    assert_eq!(widths(&app, 0), after, "not after the page selected");
    assert_eq!(chosen(&app), [2], "the page inserted isn't selected");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// In the page view, Alt+Shift+Left with an object selected, not pages, leaves the object
/// where it is: the keys that move pages don't nudge objects. Left alone still does.
#[test]
fn alt_shift_and_an_arrow_leave_an_object_where_it_is() {
    let ctx = test_ctx();
    let mut app = objects_tab(&ctx);
    app.act(&ctx, Act::Tool(Tool::Select));
    select(&mut app, &ctx, &[0]);
    let (before, history) = (boxes(&app, 0), app.tabs[0].history());
    let down = egui::Event::ModifiersChanged(ALT_SHIFT);
    let left = key_with(Key::ArrowLeft, ALT_SHIFT);
    window(&mut app, &ctx, 1400.0, vec![down, left]);
    window_held(&mut app, &ctx, vec![], egui::Modifiers::NONE);
    settle(&mut app, &ctx);
    assert_eq!(boxes(&app, 0), before, "Alt+Shift+Left nudged it");
    assert_eq!(app.tabs[0].history(), history, "Alt+Shift+Left changed it");
    let left = key_with(Key::ArrowLeft, egui::Modifiers::NONE);
    window(&mut app, &ctx, 1400.0, vec![left]);
    settle(&mut app, &ctx);
    let x = boxes(&app, 0)[0][0];
    assert!((x - before[0][0] + 1.0).abs() < 0.05, "Left: {x}");
}

/// In the page view with nothing selected, the paste keys put pages cut after the page
/// being read, as the band's Paste does (the docs agent's find).
#[test]
fn the_paste_keys_put_pages_cut_after_the_page_being_read() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    let first = app.tabs[0].pdf.page_ids();
    app.show_view(Side::Pages);
    app.tabs[0].ed.selection = Selection::pages_from([0].into());
    ty.frame(&mut app, vec![egui::Event::Cut]);
    assert_eq!(cut(&app), [0]);
    app.tabs[0].ed.selection = Selection::None;
    app.tabs[0].go_to(3);
    ty.pause(&mut app);
    ty.frame(&mut app, vec![egui::Event::Paste("x".into())]);
    ty.pause(&mut app);
    assert_eq!(order(&app, &first), [1, 2, 3, 0, 4, 5]);
    assert!(cut(&app).is_empty(), "still cut");
}

/// Crop chosen while organising goes back to the page view, where the crop box is
/// dragged, at the first page selected, as the tools do.
#[test]
fn crop_goes_back_to_the_page_view() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = grid_tab(&ctx, 6);
    organise(&mut app, &ty);
    app.tabs[0].ed.selection = Selection::pages_from([2, 4].into());
    app.act(&ctx, Act::Crop);
    ty.pause(&mut app);
    let t = &app.tabs[0];
    assert!(!t.organising(), "still organising");
    assert!(t.crop_view(), "the crop box isn't shown");
    assert_eq!(t.current, 2, "not at the first page selected");
    assert_eq!(chosen(&app), [2, 4], "the pages let go");
}
