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
