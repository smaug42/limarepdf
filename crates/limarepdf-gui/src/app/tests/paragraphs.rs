//! Paragraphs of page text edited on the page as one (NEW-01; the paragraph design's
//! sections 5, 6.4, 8 and 9; batch 45's stage 3a): a line's block switched to Paragraph and
//! typed into, its lines rewrapping in its frame, kept as one undo step; the switch greyed
//! for a line on its own; typing over and deleting words across a paragraph's lines.

use super::*;

const PROSE: &str = "The harbour was quiet that morning, and the fishing boats sat low in the \
    water while gulls argued over the nets drying along the wall. Nobody on the quay was in a \
    hurry; the tide would turn when it turned, and the old men who mended the nets had seen \
    enough tides to know that worrying never brought one in sooner than the moon allowed.";

/// A tab of a Letter page with a heading on its own at the top and, below it, `PROSE` as one
/// paragraph of 12-point Helvetica lines 15 points apart, about 330 points wide, with the
/// read tool. Returns the paragraph's lines.
fn prose_tab(app: &mut App, ctx: &egui::Context) -> Vec<String> {
    prose_tab_of(app, ctx, PROSE)
}

/// [`prose_tab`] with `text` as the paragraph.
fn prose_tab_of(app: &mut App, ctx: &egui::Context, text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(l) if limarepdf_core::text_width(&format!("{l} {word}"), 12.0) <= 330.0 => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    let mut at = vec![("Harbour notes".to_string(), egui::pos2(72.0, 70.0))];
    let place = |i: usize| egui::pos2(72.0, 100.0 + 15.0 * i as f32);
    at.extend((lines.iter().enumerate()).map(|(i, l)| (l.clone(), place(i))));
    lines_tab(app, ctx, 792.0, &at);
    lines
}

/// `text` typed, in a frame of the window.
fn type_in(app: &mut App, ctx: &egui::Context, text: &str) {
    window(app, ctx, 1400.0, vec![egui::Event::Text(text.into())]);
}

pub(super) fn new_app() -> (egui::Context, App) {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let app = App::with(Settings::from_file(None), Recent::from_file(None));
    (ctx, app)
}

/// Double-click `word` on the active tab's first page with the read tool, then settle.
pub(super) fn double_click_word(app: &mut App, ctx: &egui::Context, word: &str) {
    let tree = settled_at(app, ctx, 1400.0);
    let (page, _) = node(&tree, "Page 1").expect("the page");
    let k = page.width() / 612.0;
    let words = app.tabs[app.active].pdf.page_words(0).unwrap();
    let w = words.iter().find(|w| w.text == word).expect(word).bbox;
    let p = page.min + egui::vec2((w[0] + w[2]) / 2.0, (w[1] + w[3]) / 2.0) * k;
    window(app, ctx, 1400.0, vec![egui::Event::PointerMoved(p)]);
    for _ in 0..2 {
        window(app, ctx, 1400.0, vec![press(p, true)]);
        window(app, ctx, 1400.0, vec![press(p, false)]);
    }
    settled_at(app, ctx, 1400.0);
}

/// Whether the status bar says `said`.
fn status_says(tree: &egui::accesskit::TreeUpdate, said: &str) -> bool {
    let mut values = tree.nodes.iter().filter_map(|(_, n)| n.value());
    values.any(|v| v.starts_with(said))
}

/// Click the command band's button called `name` (the topmost control of that name, above
/// the inspector's), then let half a second of frames pass, so that the next click isn't
/// taken with this one as a double-click.
fn click_band(app: &mut App, ctx: &egui::Context, name: &str) {
    let tree = settled_at(app, ctx, 1400.0);
    let at = (tree.nodes.iter())
        .filter(|(_, n)| n.label() == Some(name))
        .filter_map(|(_, n)| n.bounds())
        .min_by(|a, b| a.y0.total_cmp(&b.y0))
        .unwrap_or_else(|| panic!("no {name}"));
    let p = egui::pos2((at.x0 + at.x1) as f32 / 2.0, (at.y0 + at.y1) as f32 / 2.0);
    window(app, ctx, 1400.0, vec![egui::Event::PointerMoved(p)]);
    window(app, ctx, 1400.0, vec![press(p, true)]);
    window(app, ctx, 1400.0, vec![press(p, false)]);
    for _ in 0..30 {
        window(app, ctx, 1400.0, vec![]);
    }
}

/// The page's paragraph as it reads now: its lines below the heading, joined by spaces.
fn paragraph_now(pdf: &Pdf) -> String {
    let lines = line_texts(pdf, 0);
    lines[1..].join(" ")
}

/// The first page's content as it is.
fn content(pdf: &Pdf) -> Vec<u8> {
    pdf.doc.get_page_content(pdf.page_ids()[0])
}

fn key(key: Key) -> egui::Event {
    key_with(key, egui::Modifiers::NONE)
}

/// The page's words with the read tool, `from` to `to` (inclusive) selected, by their text.
fn select_words(app: &mut App, from: &str, to: &str) {
    let words = app.tabs[0].pdf.page_words(0).unwrap();
    let at = |s: &str| words.iter().position(|w| w.text == s).expect(s);
    app.tabs[0].ed.selection = Selection::Words {
        anchor: (0, at(from)),
        end: (0, at(to)),
    };
}

/// A line's block, edited a line at a time as the check judged (double-click), is switched
/// to Paragraph: the status bar and the editor say so; typed into, its lines rewrap in the
/// frame, read as typed; kept as one undo step, which one Undo takes back to the page's
/// content as it was, byte for byte.
#[test]
fn a_line_switched_to_paragraph_is_typed_into_as_one_and_kept_as_one_step() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    let was = content(&app.tabs[0].pdf);
    let text = paragraph_now(&app.tabs[0].pdf);
    double_click_word(&mut app, &ctx, "fishing");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing one line"), "a double-click");
    let state = app.tabs[0].ed.para_state();
    assert!(state.can && !state.on, "{state:?}");
    app.act(&ctx, Act::EditParagraph);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing a paragraph"), "switched");
    let mut labels = tree.nodes.iter().filter_map(|(_, n)| n.label());
    assert!(
        labels.any(|l| l.starts_with("Editing a paragraph: The harbour was quiet")),
        "the paragraph's box"
    );
    // The inspector's switch, and how what is typed is written.
    let says = inspector_says(&tree);
    let switch = ["Editing", "Paragraph", "One line"];
    for said in switch.into_iter().chain(["Source font", "Preserved"]) {
        assert!(says.iter().any(|s| s == said), "{said}: {says:?}");
    }
    let steps = app.tabs[0].history().1;
    // At the caret, after the word double-clicked.
    type_in(&mut app, &ctx, " very");
    settled_at(&mut app, &ctx, 1400.0);
    assert!(app.tabs[0].ed.pending(), "typed, not yet kept");
    window(&mut app, &ctx, 1400.0, vec![key(Key::Escape)]);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(!status_says(&tree, "Editing a paragraph"), "Esc ends it");
    assert_eq!(app.tabs[0].history().1, steps + 1, "one step");
    let now = paragraph_now(&app.tabs[0].pdf);
    assert_ne!(now, text);
    assert!(now.contains("fishing very boats"), "{now:?}");
    // The words typed, the rest as it was.
    assert_eq!(now.replacen(" very", "", 1), text);
    // Every line within the frame: nothing ran past the old right edge.
    let objs = app.tabs[0].pdf.page_objects(0).unwrap();
    for o in objs.iter().filter(|o| o.text.is_some() && o.bbox[1] > 90.0) {
        assert!(o.bbox[2] <= 72.0 + 331.0, "{:?} past the frame", o.text);
    }
    app.act(&ctx, Act::Undo);
    settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(content(&app.tabs[0].pdf), was, "one Undo, byte for byte");
}

/// The user's finding after 3a: the band's Paragraph, clicked while a line is edited, edits
/// its paragraph (the click ended the line's editing first, so it went back to reading), and
/// clicked again edits the line at the caret.
#[test]
fn the_bands_paragraph_switches_the_line_being_edited_to_its_paragraph_and_back() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    double_click_word(&mut app, &ctx, "fishing");
    assert!(app.tabs[0].ed.editing_line() && !app.tabs[0].ed.editing_paragraph());
    click_band(&mut app, &ctx, "Paragraph");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing a paragraph"), "not switched");
    click_band(&mut app, &ctx, "Paragraph");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing one line"), "not switched back");
}

/// The user's finding after 3a: the band's Paragraph with nothing selected turns paragraph
/// editing on, so a double-click edits a paragraph (a line on its own still a line), and
/// off again, so it edits a line.
#[test]
fn the_bands_paragraph_with_nothing_selected_turns_paragraph_editing_on_and_off() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    click_band(&mut app, &ctx, "Paragraph");
    assert!(app.settings.paragraphs, "not remembered");
    double_click_word(&mut app, &ctx, "fishing");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing a paragraph"), "not on");
    window(&mut app, &ctx, 1400.0, vec![key(Key::Escape)]);
    double_click_word(&mut app, &ctx, "Harbour");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing one line"), "a line on its own");
    window(&mut app, &ctx, 1400.0, vec![key(Key::Escape)]);
    click_band(&mut app, &ctx, "Paragraph");
    assert!(!app.settings.paragraphs);
    // Another word: two more clicks on "fishing" with no time between would be a third.
    double_click_word(&mut app, &ctx, "gulls");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing one line"), "not off");
}

/// The user's finding after 3a: Edit text, pressed (the Text tool, or text being edited on
/// the page), clicked again goes back to the read tool, what was typed kept.
#[test]
fn edit_text_pressed_again_goes_back_to_the_read_tool() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    let text = paragraph_now(&app.tabs[0].pdf);
    click_band(&mut app, &ctx, "Edit text");
    assert_eq!(app.tabs[0].ed.tool, Tool::Text);
    click_band(&mut app, &ctx, "Edit text");
    assert_eq!(app.tabs[0].ed.tool, Tool::Read, "the Text tool stayed");
    // A line edited with the read tool: ended, kept.
    double_click_word(&mut app, &ctx, "fishing");
    window(&mut app, &ctx, 1400.0, vec![key(Key::End)]);
    type_in(&mut app, &ctx, " too");
    click_band(&mut app, &ctx, "Edit text");
    let ed = &app.tabs[0].ed;
    assert!(!ed.typing_on_page(), "still editing");
    assert_eq!(ed.tool, Tool::Read, "not the read tool");
    assert_ne!(paragraph_now(&app.tabs[0].pdf), text, "the typing not kept");
    // A paragraph being edited: the same.
    let text = paragraph_now(&app.tabs[0].pdf);
    double_click_word(&mut app, &ctx, "gulls");
    click_band(&mut app, &ctx, "Paragraph");
    assert!(app.tabs[0].ed.editing_paragraph());
    window(&mut app, &ctx, 1400.0, vec![key(Key::End)]);
    type_in(&mut app, &ctx, " again");
    click_band(&mut app, &ctx, "Edit text");
    let ed = &app.tabs[0].ed;
    let read = !ed.typing_on_page() && ed.tool == Tool::Read;
    assert!(read, "still editing");
    assert_ne!(paragraph_now(&app.tabs[0].pdf), text, "the typing not kept");
}

/// A paragraph opened and left without a change makes no change and no undo step, and the
/// document stays unchanged.
#[test]
fn a_paragraph_left_as_it_was_makes_no_change() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    let was = content(&app.tabs[0].pdf);
    double_click_word(&mut app, &ctx, "fishing");
    app.act(&ctx, Act::EditParagraph);
    settled_at(&mut app, &ctx, 1400.0);
    assert!(app.tabs[0].ed.editing_paragraph());
    let steps = app.tabs[0].history().1;
    assert!(!app.tabs[0].ed.pending(), "nothing typed");
    window(&mut app, &ctx, 1400.0, vec![key(Key::Escape)]);
    settled_at(&mut app, &ctx, 1400.0);
    assert!(!app.tabs[0].ed.editing_paragraph());
    assert_eq!(app.tabs[0].history().1, steps);
    assert!(!app.tabs[0].dirty);
    assert_eq!(content(&app.tabs[0].pdf), was);
}

/// What is typed in a paragraph is kept as one change before a command that uses the
/// document (as Save keeps it), as typing in a line is.
#[test]
fn a_paragraphs_typing_is_kept_before_a_command_uses_the_document() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    double_click_word(&mut app, &ctx, "fishing");
    app.act(&ctx, Act::EditParagraph);
    settled_at(&mut app, &ctx, 1400.0);
    type_in(&mut app, &ctx, " very");
    settled_at(&mut app, &ctx, 1400.0);
    let steps = app.tabs[0].history().1;
    assert!(app.tabs[0].ed.pending());
    assert!(app.tabs[0].keep_typing(&ctx));
    assert!(!app.tabs[0].ed.pending());
    assert_eq!(app.tabs[0].history().1, steps + 1);
    assert!(paragraph_now(&app.tabs[0].pdf).contains("fishing very boats"));
}

/// Switched to Paragraph, paragraph editing stays on: a double-click elsewhere in it opens
/// the paragraph again; switched back, it edits the line at the caret, and a double-click
/// then edits a line again.
#[test]
fn the_switch_stays_on_and_one_line_switches_back() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    double_click_word(&mut app, &ctx, "fishing");
    app.act(&ctx, Act::EditParagraph);
    settled_at(&mut app, &ctx, 1400.0);
    window(&mut app, &ctx, 1400.0, vec![key(Key::Escape)]);
    settled_at(&mut app, &ctx, 1400.0);
    double_click_word(&mut app, &ctx, "gulls");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing a paragraph"), "remembered");
    app.act(&ctx, Act::EditParagraph);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing one line"), "switched back");
    let mut labels = tree.nodes.iter().filter_map(|(_, n)| n.label());
    assert!(labels.any(|l| l.starts_with("Editing text: ")), "a line");
    window(&mut app, &ctx, 1400.0, vec![key(Key::Escape)]);
    double_click_word(&mut app, &ctx, "nets");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing one line"), "forgotten");
}

/// A line on its own can't be edited as a paragraph: the inspector's switch is greyed for
/// it, saying why; the band's Paragraph still turns paragraph editing on (ticked), the line
/// staying a line.
#[test]
fn a_line_on_its_own_cant_be_switched_and_says_why() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    double_click_word(&mut app, &ctx, "Harbour");
    let state = app.tabs[0].ed.para_state();
    assert!(!state.can && !state.on, "{state:?}");
    assert_eq!(state.why, Some("This line is on its own."));
    let mut c = Ctx::default();
    app.tabs[0].fill_ctx(&mut c);
    let a = actions::get(Act::EditParagraph);
    assert!((a.enabled)(&c) && !(a.checked.unwrap())(&c));
    app.act(&ctx, Act::EditParagraph);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing one line"), "still a line");
    let mut c = Ctx::default();
    app.tabs[0].fill_ctx(&mut c);
    assert!((a.checked.unwrap())(&c), "paragraph editing not on");
    assert!(app.settings.paragraphs);
}

/// Enter starts a new paragraph in the frame and Shift+Enter a new line: each written as a
/// line of its own below the last.
#[test]
fn enter_starts_a_new_paragraph_and_shift_enter_a_new_line() {
    let (ctx, mut app) = new_app();
    let lines = prose_tab(&mut app, &ctx);
    // The caret at the end: after the last word, and End.
    double_click_word(&mut app, &ctx, "allowed.");
    app.act(&ctx, Act::EditParagraph);
    settled_at(&mut app, &ctx, 1400.0);
    window(&mut app, &ctx, 1400.0, vec![key(Key::End)]);
    window(&mut app, &ctx, 1400.0, vec![key(Key::Enter)]);
    type_in(&mut app, &ctx, "Gulls cried.");
    let shift = key_with(Key::Enter, egui::Modifiers::SHIFT);
    window(&mut app, &ctx, 1400.0, vec![shift]);
    type_in(&mut app, &ctx, "Boats came in.");
    let (text, _) = app.tabs[0].ed.para_text().expect("the paragraph");
    let want = "allowed.\u{2029}Gulls cried.\nBoats came in.";
    assert!(text.ends_with(want), "{text:?}");
    window(&mut app, &ctx, 1400.0, vec![key(Key::Escape)]);
    settled_at(&mut app, &ctx, 1400.0);
    let now = line_texts(&app.tabs[0].pdf, 0);
    let n = now.len();
    assert_eq!(&now[n - 2..], ["Gulls cried.", "Boats came in."], "{now:?}");
    assert_eq!(now[n - 3], lines[lines.len() - 1], "{now:?}");
}

/// CI's finding on Windows after 3a: a line switched to its paragraph, or back, while its
/// font is still loading (drawn in Inter, wider, its caret past the line's end) is edited
/// as switched to, not left with nothing being edited.
#[test]
fn a_line_switched_while_its_font_loads_is_still_switched() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    crate::typing_fonts::hold(&ctx, "Helvetica");
    double_click_word(&mut app, &ctx, "allowed.");
    assert!(app.tabs[0].ed.editing_line());
    app.act(&ctx, Act::EditParagraph);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing a paragraph"), "not switched");
    window(&mut app, &ctx, 1400.0, vec![key(Key::End)]);
    app.act(&ctx, Act::EditParagraph);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing one line"), "not switched back");
}

/// Words selected with the read tool across lines of a paragraph LimarePDF is sure of are
/// typed over there, and deleted there, the paragraph rewrapping (stage 3's Q3), as a word
/// processor takes a selection out of its text.
#[test]
fn words_across_a_paragraphs_lines_are_typed_over_and_deleted_there() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    let blocks = app.tabs[0].pdf.page_blocks(0).unwrap();
    let sure = blocks.iter().any(|b| b.confident() && b.lines.len() > 3);
    assert!(sure, "{blocks:?}");
    let text = paragraph_now(&app.tabs[0].pdf);
    settled_at(&mut app, &ctx, 1400.0);
    select_words(&mut app, "boats", "water");
    type_in(&mut app, &ctx, "ships");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(status_says(&tree, "Editing a paragraph"), "not there");
    window(&mut app, &ctx, 1400.0, vec![key(Key::Escape)]);
    settled_at(&mut app, &ctx, 1400.0);
    let want = text.replacen("boats sat low in the water", "ships", 1);
    assert_eq!(paragraph_now(&app.tabs[0].pdf), want);
    // Deleted across lines: taken out, the caret where they were. On the page as it was:
    // rewritten, its block needn't read as one LimarePDF is sure of.
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    settled_at(&mut app, &ctx, 1400.0);
    select_words(&mut app, "argued", "Nobody");
    app.act(&ctx, Act::DeleteSelection);
    settled_at(&mut app, &ctx, 1400.0);
    assert!(app.tabs[0].ed.editing_paragraph());
    window(&mut app, &ctx, 1400.0, vec![key(Key::Escape)]);
    settled_at(&mut app, &ctx, 1400.0);
    // A space beside them goes with them, by Word's rule.
    let want = text.replacen("argued over the nets drying along the wall. Nobody ", "", 1);
    assert_eq!(paragraph_now(&app.tabs[0].pdf), want);
}

/// Words selected across two paragraphs (the heading and the paragraph under it) aren't
/// typed over: a note at the selection's end says to type within one paragraph.
#[test]
fn typing_over_words_in_two_paragraphs_says_to_type_within_one() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    settled_at(&mut app, &ctx, 1400.0);
    select_words(&mut app, "notes", "harbour");
    let lines = line_texts(&app.tabs[0].pdf, 0);
    type_in(&mut app, &ctx, "x");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(app.tabs[0].ed.typing_id(), None, "typed");
    let said = |n: &egui::accesskit::Node| n.value().is_some_and(|v| v.starts_with(NOTE));
    assert!(tree.nodes.iter().any(|(_, n)| said(n)), "no note");
    assert_eq!(line_texts(&app.tabs[0].pdf, 0), lines);
}

const NOTE: &str = "Type over words within one paragraph.";

/// How long a frame of the window takes while a paragraph of about 2,000 characters is
/// typed into (each keystroke sets it again and lays its box out on core's lines), for
/// performance.md: the median and the slowest of 40 keystrokes, typed in its first line.
/// Opt-in, in a release build: `cargo test --release -p limarepdf-gui --bin limarepdf --
/// --ignored speed_of_typing_in_a_paragraph --nocapture`.
#[test]
#[ignore]
fn speed_of_typing_in_a_paragraph() {
    let (ctx, mut app) = new_app();
    let text = [PROSE; 6].join(" ");
    prose_tab_of(&mut app, &ctx, &text);
    double_click_word(&mut app, &ctx, "fishing");
    app.act(&ctx, Act::EditParagraph);
    settled_at(&mut app, &ctx, 1400.0);
    let (typed, _) = app.tabs[0].ed.para_text().expect("the paragraph");
    let mut times = Vec::new();
    for i in 0..40 {
        let t = std::time::Instant::now();
        type_in(&mut app, &ctx, if i % 5 == 4 { " " } else { "a" });
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    let n = typed.chars().count();
    eprintln!(
        "a keystroke in a paragraph of {n} characters: median {:.2} ms, slowest {:.2} ms",
        times[times.len() / 2],
        times[times.len() - 1]
    );
}

/// A line-end hyphen LimarePDF can't tell belongs to its word (the paragraph design's 6.4),
/// brought inside a line by a rewrap, is kept as typed and marked; Join takes it out, the
/// word made one, and the edit is written so.
#[test]
fn a_hyphen_brought_inside_a_line_is_marked_and_joined_on_asking() {
    let (ctx, mut app) = new_app();
    // "co-operation" broken after its hyphen at a line's end; the document has neither form
    // elsewhere, so the hyphen is unknown.
    let lines = [
        "The harbour was quiet that morning, and the fishing boats sat",
        "low in the water, waiting a long while for the tide and its co-",
        "operation, while gulls argued over the nets drying along the",
        "wall of the old quay where nobody on it was in any hurry.",
    ];
    let at = (lines.iter().enumerate())
        .map(|(i, l)| (l.to_string(), egui::pos2(72.0, 100.0 + 15.0 * i as f32)))
        .collect::<Vec<_>>();
    lines_tab(&mut app, &ctx, 792.0, &at);
    let blocks = app.tabs[0].pdf.page_blocks(0).unwrap();
    let one = blocks.len() == 1 && blocks[0].lines.len() == 4;
    assert!(one, "{blocks:?}");
    // Switched to Paragraph once, so its block opens as one.
    double_click_word(&mut app, &ctx, "fishing");
    app.act(&ctx, Act::EditParagraph);
    settled_at(&mut app, &ctx, 1400.0);
    assert!(app.tabs[0].ed.editing_paragraph());
    window(&mut app, &ctx, 1400.0, vec![key(Key::Escape)]);
    settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(app.tabs[0].ed.marked_hyphens(), 0, "nothing moved yet");
    // Words before it, in its line, taken out: the block was switched to Paragraph, so it
    // rewraps, and "co-operation," fits on that line now.
    select_words(&mut app, "waiting", "its");
    app.act(&ctx, Act::DeleteSelection);
    settled_at(&mut app, &ctx, 1400.0);
    assert!(app.tabs[0].ed.editing_paragraph());
    assert_eq!(app.tabs[0].ed.marked_hyphens(), 1, "marked");
    let mut c = Ctx::default();
    app.tabs[0].fill_ctx(&mut c);
    assert!((actions::get(Act::JoinHyphen).enabled)(&c), "no Join");
    app.act(&ctx, Act::JoinHyphen);
    settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(app.tabs[0].ed.marked_hyphens(), 0, "joined");
    window(&mut app, &ctx, 1400.0, vec![key(Key::Escape)]);
    settled_at(&mut app, &ctx, 1400.0);
    let now = line_texts(&app.tabs[0].pdf, 0).join(" ");
    let joined = "in the water, cooperation, while gulls";
    assert!(now.contains(joined), "{now:?}");
}
