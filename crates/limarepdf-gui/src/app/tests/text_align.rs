//! Text alignment (NEW-15, batch 49's stage 1): Edit's Alignment group, Ctrl+L, Ctrl+E,
//! Ctrl+R and Ctrl+J align the paragraph being edited, as Word's Paragraph group does: it
//! rewraps in its frame at once, its typing open; an alignment pressed again goes back to
//! left; its own Undo takes each change back, and it goes into the document with the typing
//! as one step. Stage 2: a line edited alone opens its paragraph aligned, paragraph editing
//! staying off; the paragraphs of words selected while reading, or of a line selected with
//! the arrow, are aligned as one step, staying selected; a line on its own is left as it
//! was, with a notice. Stage 3: new text's lines are aligned among themselves while typed,
//! and a text box left, centred or right; neither justified, and a field not at all.

use super::paragraphs::{
    click_band, content, double_click_word, new_app, open_prose_paragraph, paragraph_lines,
    paragraph_now, prose_tab, prose_tab_with, PROSE,
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

/// The user's finding: realigned, kept and opened again, a paragraph's box moved each time,
/// read anew from its lines. Now it keeps its frame: widened by its right edge and kept,
/// opened again it has its width; centred and kept, opened again the same; aligned left and
/// kept, its lines are back at the left edge, the frame where it was.
#[test]
fn a_paragraph_realigned_kept_and_opened_again_keeps_its_box() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    let tree = open_prose_paragraph(&mut app, &ctx);
    let (left, right) = app.tabs[0].ed.para_edges().expect("editing a paragraph");
    let (handle, _) = node(&tree, "Paragraph's right edge").expect("the right edge");
    window(&mut app, &ctx, 1400.0, click_at(handle.center()));
    settled_at(&mut app, &ctx, 1400.0);
    let ten = key_with(Key::ArrowRight, egui::Modifiers::SHIFT);
    for _ in 0..6 {
        window(&mut app, &ctx, 1400.0, vec![ten.clone()]);
    }
    settled_at(&mut app, &ctx, 1400.0);
    let wide = (left, right + 60.0);
    let edges = |app: &App| app.tabs[0].ed.para_edges().expect("editing a paragraph");
    let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 0.05 && (a.1 - b.1).abs() < 0.05;
    assert!(near(edges(&app), wide), "{:?}", edges(&app));
    for (key, want) in [
        (None, TextAlign::Left),
        (Some(Key::E), TextAlign::Centre),
        (Some(Key::L), TextAlign::Left),
        (Some(Key::R), TextAlign::Right),
        (Some(Key::L), TextAlign::Left),
    ] {
        if let Some(key) = key {
            ctrl(&mut app, &ctx, key);
        }
        esc(&mut app, &ctx);
        assert!(!open(&app), "Esc keeps it");
        double_click_word(&mut app, &ctx, "fishing");
        assert!(open(&app), "{want:?}: not opened again");
        assert_eq!(aligned(&app), Some(want));
        assert!(near(edges(&app), wide), "{want:?}: {:?}", edges(&app));
    }
    esc(&mut app, &ctx);
    for o in paragraph_lines(&app.tabs[0].pdf) {
        assert!((o.bbox[0] - left).abs() < 0.6, "{o:?} not at {left}");
    }
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

// ---- Stage 2: page text not edited as a paragraph.

/// A second paragraph for [`two_paragraphs_tab`].
const SECOND: &str = "Down at the end of the wall the keeper painted the railings again, as he \
    did every spring, and the children came to watch the white paint go on in long slow \
    strokes while the sea moved under the stones below them.";

/// `text` in lines of 12-point Helvetica up to 330 points wide, as [`prose_tab`] has them.
fn wrap(text: &str) -> Vec<String> {
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
    lines
}

/// [`prose_tab`] with [`SECOND`] as a paragraph of its own under the first, a line's space
/// between them. Returns how many lines each paragraph has.
fn two_paragraphs_tab(app: &mut App, ctx: &egui::Context) -> (usize, usize) {
    let second = wrap(SECOND);
    let below: Vec<(&str, usize)> = (second.iter().enumerate())
        .map(|(j, l)| (l.as_str(), j + 2))
        .collect();
    let first = prose_tab_with(app, ctx, PROSE, &below);
    (first.len(), second.len())
}

/// The page's two paragraphs under the heading, of `sizes` lines, each top to bottom.
fn paragraphs_of(pdf: &Pdf, sizes: (usize, usize)) -> [Vec<limarepdf_core::PageObject>; 2] {
    let mut lines = paragraph_lines(pdf);
    lines.sort_by(|a, b| a.bbox[1].total_cmp(&b.bbox[1]));
    let second = lines.split_off(sizes.0);
    assert_eq!(second.len(), sizes.1, "{second:?}");
    [lines, second]
}

/// The page's words `from` to `to` (inclusive, by their text) selected with the read tool,
/// then the window settled.
fn select_words(app: &mut App, ctx: &egui::Context, from: &str, to: &str) {
    let words = app.tabs[0].pdf.page_words(0).unwrap();
    let at = |s: &str| words.iter().position(|w| w.text == s).expect(s);
    app.tabs[0].ed.selection = Selection::Words {
        anchor: (0, at(from)),
        end: (0, at(to)),
    };
    settled_at(app, ctx, 1400.0);
}

/// A line edited alone, paragraph editing off, a word typed over in it: the alignments act
/// on it, its paragraph's own (left) pressed. Ctrl+E keeps the typing, then opens its
/// paragraph centred with the keyboard in it, paragraph editing staying off; Esc keeps it,
/// every line centred between the frame's edges, the word typed in it: the line's step and
/// the paragraph's. A double-click edits a line again.
#[test]
fn a_line_edited_alone_opens_its_paragraph_aligned_paragraph_editing_staying_off() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    double_click_word(&mut app, &ctx, "fishing");
    assert!(app.tabs[0].ed.editing_line() && !open(&app), "not a line");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let centre = actions::get(Act::AlignText(TextAlign::Centre));
    assert!((centre.enabled)(&app.state(&ctx)), "greyed in a line");
    assert_eq!(pressed(&tree, "Align left"), Some(Toggled::True));
    let typed = vec![egui::Event::Text("sailing".into())];
    window(&mut app, &ctx, 1400.0, typed);
    let steps = app.tabs[0].history().1;
    ctrl(&mut app, &ctx, Key::E);
    assert!(open(&app), "its paragraph didn't open");
    assert_eq!(aligned(&app), Some(TextAlign::Centre));
    let on = app.settings.paragraphs || app.tabs[0].ed.paragraphs;
    assert!(!on, "paragraph editing turned on");
    let (focused, typing) = (ctx.memory(|m| m.focused()), app.tabs[0].ed.typing_id());
    assert_eq!(focused, typing, "the keyboard isn't in it");
    let (left, right) = app.tabs[0].ed.para_edges().expect("editing a paragraph");
    esc(&mut app, &ctx);
    assert!(!open(&app), "Esc keeps it");
    let now = app.tabs[0].history().1;
    assert_eq!(now, steps + 2, "the line's step and the paragraph's");
    assert!(paragraph_now(&app.tabs[0].pdf).contains("the sailing boats"));
    let middle = (left + right) / 2.0;
    for o in paragraph_lines(&app.tabs[0].pdf) {
        let at = (o.bbox[0] + o.bbox[2]) / 2.0;
        assert!((at - middle).abs() < 0.6, "{o:?} not centred on {middle}");
    }
    double_click_word(&mut app, &ctx, "sailing");
    assert!(app.tabs[0].ed.editing_line() && !open(&app), "not a line");
}

/// A line on its own edited alone has no paragraph to align: Ctrl+E leaves it as it was,
/// still being edited with the keyboard in it, and says why.
#[test]
fn a_line_on_its_own_isnt_aligned_and_says_why() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    let was = content(&app.tabs[0].pdf);
    double_click_word(&mut app, &ctx, "Harbour");
    let typing = app.tabs[0].ed.typing_id().expect("typing");
    ctrl(&mut app, &ctx, Key::E);
    assert!(app.tabs[0].ed.editing_line() && !open(&app), "not the line");
    let said = toast(&app);
    assert_eq!(said.as_deref(), Some("Not aligned: 1 line on its own"));
    let focused = ctx.memory(|m| m.focused());
    assert_eq!(focused, Some(typing), "the keyboard left the line");
    esc(&mut app, &ctx);
    assert_eq!(content(&app.tabs[0].pdf), was);
    assert_eq!(app.tabs[0].history().1, 0);
}

/// Words selected while reading, from the heading on its own into the second of two
/// paragraphs: the alignments show the paragraphs' own, Align left pressed. Ctrl+J
/// justifies both as one step, every line but each one's last reaching its frame's right
/// edge and the last as it was, the heading as it was and the notice saying so; the words
/// stay selected, Justify pressed. Ctrl+J again puts both back to the left, as Word does;
/// two Undos put the page back byte for byte.
#[test]
fn words_selected_align_every_paragraph_they_touch_as_one_step() {
    let (ctx, mut app) = new_app();
    let sizes = two_paragraphs_tab(&mut app, &ctx);
    let was = content(&app.tabs[0].pdf);
    let before = paragraphs_of(&app.tabs[0].pdf, sizes);
    select_words(&mut app, &ctx, "notes", "railings");
    let text = app.tabs[0].ed.selected_text();
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(pressed(&tree, "Align left"), Some(Toggled::True));
    ctrl(&mut app, &ctx, Key::J);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(app.tabs[0].history().1, 1, "one step");
    let said = toast(&app);
    assert_eq!(said.as_deref(), Some("Not aligned: 1 line on its own"));
    let still = app.tabs[0].ed.selected_text();
    assert_eq!(still, text, "the words aren't selected");
    assert_eq!(pressed(&tree, "Justify"), Some(Toggled::True));
    assert_eq!(line_texts(&app.tabs[0].pdf, 0)[0], "Harbour notes");
    let after = paragraphs_of(&app.tabs[0].pdf, sizes);
    for (was, now) in before.iter().zip(&after) {
        let right = was.iter().map(|o| o.bbox[2]).fold(0.0, f32::max);
        let ((last_was, _), (last, rest)) = (was.split_last().unwrap(), now.split_last().unwrap());
        for o in rest {
            assert!((o.bbox[2] - right).abs() < 0.6, "{o:?} short of {right}");
        }
        assert_eq!(last.bbox, last_was.bbox, "the last line moved");
    }
    ctrl(&mut app, &ctx, Key::J);
    assert_eq!(app.tabs[0].history().1, 2, "one step");
    for o in paragraph_lines(&app.tabs[0].pdf) {
        assert!((o.bbox[0] - 72.0).abs() < 0.6, "{o:?} not at the left edge");
    }
    for _ in 0..2 {
        app.act(&ctx, Act::Undo);
        settled_at(&mut app, &ctx, 1400.0);
    }
    assert_eq!(content(&app.tabs[0].pdf), was, "two Undos, byte for byte");
}

/// A line of a paragraph selected with the arrow: the floating bar has Centre, which
/// centres its paragraph as one step, the line staying selected where it moved; it shows
/// as centred. Ctrl+E again puts the paragraph back to the left, as Word does.
#[test]
fn a_line_selected_with_the_arrow_aligns_its_paragraph_from_the_floating_bar() {
    let (ctx, mut app) = new_app();
    let lines = prose_tab(&mut app, &ctx);
    app.act(&ctx, Act::Tool(Tool::Select));
    let last = lines.last().expect("lines").as_str();
    let objs = app.tabs[0].pdf.page_objects(0).unwrap();
    let id = objs.iter().position(|o| o.text.as_deref() == Some(last));
    select(&mut app, &ctx, &[id.expect("the last line")]);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let bar = bar_buttons(&tree);
    let centre = bar.iter().find(|(n, _)| n == "Centre");
    let (_, r) = centre.expect("no Centre on the bar");
    window(&mut app, &ctx, 1400.0, click_at(r.center()));
    settled_at(&mut app, &ctx, 1400.0);
    assert_eq!(app.tabs[0].history().1, 1, "one step");
    let (_, ids) = held(&app).expect("nothing selected");
    let now = app.tabs[0].pdf.page_objects(0).unwrap();
    let o = &now[ids[0]];
    let selected = (ids.len(), o.text.as_deref());
    assert_eq!(selected, (1, Some(last)), "another selected");
    assert!(o.bbox[0] > 100.0, "not centred: {o:?}");
    assert_eq!(aligned(&app), Some(TextAlign::Centre));
    ctrl(&mut app, &ctx, Key::E);
    assert_eq!(app.tabs[0].history().1, 2, "one step");
    for o in paragraph_lines(&app.tabs[0].pdf) {
        assert!((o.bbox[0] - 72.0).abs() < 0.6, "{o:?} not at the left edge");
    }
}

// ---- Stage 3: new text, text boxes and fields.

/// `text` typed, then the window settled.
fn type_text(app: &mut App, ctx: &egui::Context, text: &str) {
    window(app, ctx, 1400.0, vec![egui::Event::Text(text.into())]);
    settled_at(app, ctx, 1400.0);
}

/// Justify's state: whether it can be used, and why not.
fn justify(app: &mut App, ctx: &egui::Context) -> (bool, &'static str) {
    let c = app.state(ctx);
    let a = actions::get(Act::AlignText(TextAlign::Justified));
    ((a.enabled)(&c), actions::why_not(&c, a))
}

/// The lines of text on the first page: where each reaches along it, from its first
/// character's start to its last's end, as the page reads them.
fn line_reaches(pdf: &Pdf) -> Vec<(f32, f32)> {
    (pdf.page_objects(0).unwrap().iter())
        .filter(|o| o.kind == limarepdf_core::ObjectKind::Text)
        .map(|o| {
            let x = o.baseline.unwrap().0;
            (x + o.stops[0], x + o.stops.last().unwrap())
        })
        .collect()
}

/// New text's lines, typed with the Text tool, are aligned among themselves as Word aligns a
/// paragraph's: Ctrl+E centres them while they are typed, the typing open and Centre
/// pressed; Justify is greyed, saying why (each line ends at Enter), and its key does
/// nothing; Ctrl+E again goes back to left and Ctrl+R aligns right; Ctrl+Z in the text takes
/// the last change back and Ctrl+Y does it again. Kept with Esc as one step: the widest line
/// starts where the text was put, the other ends where it ends. The next new text, the Text
/// tool chosen again, starts aligned right, as the last was.
#[test]
fn new_text_s_lines_are_aligned_while_typed_and_kept_so() {
    let (ctx, _) = new_app();
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::Tool(Tool::Text));
    settled_at(&mut app, &ctx, 1400.0);
    click_page(&mut app, &ctx, (100.0, 200.0), false);
    type_text(&mut app, &ctx, "The walrus and the carpenter");
    let enter = key_with(Key::Enter, egui::Modifiers::NONE);
    window(&mut app, &ctx, 1400.0, vec![enter]);
    type_text(&mut app, &ctx, "were walking");
    let typed = (app.tabs[0].ed.adding.as_ref()).map(|a| a.text.text().to_string());
    let want = "The walrus and the carpenter\nwere walking";
    assert_eq!(typed.as_deref(), Some(want), "not typed as new text");
    assert_eq!(aligned(&app), Some(TextAlign::Left));
    ctrl(&mut app, &ctx, Key::E);
    let tree = settled_at(&mut app, &ctx, 1400.0);
    assert!(app.tabs[0].ed.adding.is_some(), "Ctrl+E ended the typing");
    assert_eq!(aligned(&app), Some(TextAlign::Centre));
    assert_eq!(pressed(&tree, "Centre"), Some(Toggled::True));
    let why = "New text's lines end at Enter: there's nothing to justify.";
    assert_eq!(justify(&mut app, &ctx), (false, why));
    for (key, want) in [
        (Key::J, TextAlign::Centre),
        (Key::E, TextAlign::Left),
        (Key::R, TextAlign::Right),
    ] {
        ctrl(&mut app, &ctx, key);
        assert!(app.tabs[0].ed.adding.is_some(), "{key:?} ended the typing");
        assert_eq!(aligned(&app), Some(want), "{key:?}");
    }
    ctrl(&mut app, &ctx, Key::Z);
    assert_eq!(aligned(&app), Some(TextAlign::Left), "Ctrl+Z");
    ctrl(&mut app, &ctx, Key::Y);
    assert_eq!(aligned(&app), Some(TextAlign::Right), "Ctrl+Y");
    let typed = (app.tabs[0].ed.adding.as_ref()).map(|a| a.text.text().to_string());
    assert_eq!(typed.as_deref(), Some(want), "the typing changed");
    let steps = app.tabs[0].history().1;
    esc(&mut app, &ctx);
    assert!(app.tabs[0].ed.adding.is_none(), "Esc keeps it");
    assert_eq!(app.tabs[0].history().1, steps + 1, "one step");
    let got = line_reaches(&app.tabs[0].pdf);
    assert_eq!(got.len(), 2, "{got:?}");
    let widest = got.iter().map(|(a, b)| b - a).fold(0.0, f32::max);
    assert!((got[0].0 - 100.0).abs() < 0.05, "{got:?}");
    for (_, b) in &got {
        assert!((b - (100.0 + widest)).abs() < 0.05, "{got:?}");
    }
    // Placed, the text is selected with the read tool: the Text tool again.
    app.act(&ctx, Act::Tool(Tool::Text));
    settled_at(&mut app, &ctx, 1400.0);
    click_page(&mut app, &ctx, (100.0, 400.0), false);
    assert!(app.tabs[0].ed.adding.is_some(), "no new text");
    assert_eq!(aligned(&app), Some(TextAlign::Right), "the next new text");
}

/// A text box is aligned left, centred or right, which its `/Q` keeps: a new one while it is
/// typed (the band's Centre), the typing open with the keyboard in it, kept so, and the next
/// new one the same; one on the page while it
/// is typed in, its typing kept; one selected, as one step Undo takes back, the alignment it
/// has pressed again going back to left. It can't be justified: Justify is greyed, saying
/// so.
#[test]
fn a_text_box_is_aligned_while_typed_or_selected() {
    let (ctx, _) = new_app();
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::Tool(Tool::TextBox));
    settled_at(&mut app, &ctx, 1400.0);
    click_page(&mut app, &ctx, (100.0, 200.0), false);
    let text = "The walrus and the carpenter were walking close at hand";
    type_text(&mut app, &ctx, text);
    let typing = app.tabs[0].ed.typing_id().expect("no text box");
    click_band(&mut app, &ctx, "Centre");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let typed = app.tabs[0].ed.writing.as_ref().map(|w| w.text.clone());
    assert_eq!(typed.as_deref(), Some(text), "Centre ended the typing");
    let focused = ctx.memory(|m| m.focused());
    assert_eq!(focused, Some(typing), "the keyboard left it");
    assert_eq!(aligned(&app), Some(TextAlign::Centre));
    assert_eq!(pressed(&tree, "Centre"), Some(Toggled::True));
    let why = "A text box can't keep justified text.";
    assert_eq!(justify(&mut app, &ctx), (false, why));
    esc(&mut app, &ctx);
    let boxes = app.tabs[0].pdf.annotations();
    let [a] = &boxes[..] else {
        panic!("{boxes:?}");
    };
    let id = a.id;
    let q = |app: &App| {
        let d = app.tabs[0].pdf.doc.get_dictionary(id).unwrap();
        d.get(b"Q").ok().map(|o| o.as_i64().unwrap())
    };
    let style = |app: &App| app.tabs[0].pdf.annotations()[0].text_style.unwrap();
    assert_eq!((style(&app).align, q(&app)), (TextAlign::Centre, Some(1)));
    let next = app.tabs[0].ed.box_style.align;
    assert_eq!(next, TextAlign::Centre, "the next box");
    // Typed in again: Ctrl+R aligns it right, the typing open.
    app.act(&ctx, Act::Tool(Tool::Select));
    let t = &mut app.tabs[0];
    t.ed.annots(&t.pdf, 0);
    t.ed.selection = Selection::Annot { page: 0, id };
    t.ed.write_selected();
    settled_at(&mut app, &ctx, 1400.0);
    let steps = app.tabs[0].history().1;
    ctrl(&mut app, &ctx, Key::R);
    let typed = app.tabs[0].ed.writing.as_ref().map(|w| w.text.clone());
    assert_eq!(typed.as_deref(), Some(text), "Ctrl+R ended the typing");
    assert_eq!((style(&app).align, q(&app)), (TextAlign::Right, Some(2)));
    assert_eq!(aligned(&app), Some(TextAlign::Right));
    assert_eq!(app.tabs[0].history().1, steps + 1, "one step");
    esc(&mut app, &ctx);
    // Selected: Ctrl+R again, as Word's rule, aligns it left; Undo puts it back right.
    let t = &mut app.tabs[0];
    t.ed.annots(&t.pdf, 0);
    t.ed.selection = Selection::Annot { page: 0, id };
    settled_at(&mut app, &ctx, 1400.0);
    assert!(app.tabs[0].ed.writing.is_none(), "still typed in");
    assert_eq!(aligned(&app), Some(TextAlign::Right));
    let steps = app.tabs[0].history().1;
    ctrl(&mut app, &ctx, Key::R);
    assert_eq!((style(&app).align, q(&app)), (TextAlign::Left, None));
    assert_eq!(app.tabs[0].history().1, steps + 1, "one step");
    app.act(&ctx, Act::Undo);
    settled_at(&mut app, &ctx, 1400.0);
    assert_eq!((style(&app).align, q(&app)), (TextAlign::Right, Some(2)));
    assert_eq!(app.tabs[0].pdf.annotations()[0].contents, text);
}

/// A form field's alignment is its form's: with one typed in, the alignments are greyed,
/// saying so, none pressed, and their keys change nothing.
#[test]
fn a_field_s_alignment_is_its_form_s() {
    let (ctx, mut app) = new_app();
    let dir = test_dir("align-field");
    let path = dir.join("form.pdf");
    form_file(&path, "");
    open_and_wait(&mut app, &ctx, &path, 1);
    let b = app.tabs[0].pdf.form_fields()[0].widgets[0].bbox;
    let middle = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
    click_page(&mut app, &ctx, middle, false);
    type_text(&mut app, &ctx, "Ann");
    let in_field = |app: &App| app.tabs[0].ed.form.focused_field().is_some();
    assert!(in_field(&app), "not in the field");
    let tree = settled_at(&mut app, &ctx, 1400.0);
    let c = app.state(&ctx);
    let why = "A field's alignment is set by its form.";
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
    assert!(in_field(&app), "Ctrl+E left the field");
    assert_eq!(app.tabs[0].history().1, 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

// ---- BUG-47: paragraphs split with Enter.

/// BUG-47, the user's finding on stage 3: lines typed after Enter in a paragraph being
/// edited, kept, were lines on their own: a double-click edited one as a line, and Ctrl+E
/// there said "Not aligned: 1 line on its own". Each new paragraph keeps the frame: with
/// paragraph editing on, a double-click opens one as a paragraph between the frame's edges;
/// with it off, Ctrl+E in one edited alone opens its paragraph centred, and Esc keeps it
/// centred between the frame's edges.
#[test]
fn paragraphs_split_with_enter_reopen_in_the_frame_and_align() {
    let (ctx, mut app) = new_app();
    prose_tab(&mut app, &ctx);
    double_click_word(&mut app, &ctx, "allowed.");
    app.act(&ctx, Act::EditParagraph);
    settled_at(&mut app, &ctx, 1400.0);
    let (left, right) = app.tabs[0].ed.para_edges().expect("editing a paragraph");
    let none = egui::Modifiers::NONE;
    window(&mut app, &ctx, 1400.0, vec![key_with(Key::End, none)]);
    for line in ["Gulls cried.", "Boats came in."] {
        window(&mut app, &ctx, 1400.0, vec![key_with(Key::Enter, none)]);
        window(&mut app, &ctx, 1400.0, vec![egui::Event::Text(line.into())]);
    }
    esc(&mut app, &ctx);
    assert!(!open(&app), "Esc keeps it");
    double_click_word(&mut app, &ctx, "Boats");
    assert!(open(&app), "a new paragraph opened as a line");
    let edges = app.tabs[0].ed.para_edges().expect("editing a paragraph");
    let same = (edges.0 - left).abs() < 0.05 && (edges.1 - right).abs() < 0.05;
    assert!(same, "{edges:?}, was {:?}", (left, right));
    esc(&mut app, &ctx);
    click_band(&mut app, &ctx, "Paragraph");
    assert!(!app.settings.paragraphs, "paragraph editing still on");
    double_click_word(&mut app, &ctx, "Gulls");
    assert!(app.tabs[0].ed.editing_line() && !open(&app), "not a line");
    ctrl(&mut app, &ctx, Key::E);
    assert!(open(&app), "its paragraph didn't open: {:?}", toast(&app));
    assert_eq!(aligned(&app), Some(TextAlign::Centre));
    esc(&mut app, &ctx);
    let gulls = (paragraph_lines(&app.tabs[0].pdf).into_iter())
        .find(|o| o.text.as_deref() == Some("Gulls cried."))
        .expect("the line");
    let (at, middle) = ((gulls.bbox[0] + gulls.bbox[2]) / 2.0, (left + right) / 2.0);
    assert!((at - middle).abs() < 0.6, "{gulls:?} off {middle}");
}

/// How long aligning takes on the window's thread, for performance.md: every paragraph of
/// a made-up document of 300 Letter pages, each one paragraph of ten 12-point Helvetica
/// lines, selected as words and justified, as one step: the command, of it finding the
/// pages' paragraphs and writing them (as `App::act` runs it), and the frame after it.
/// Opt-in, on an optimised build:
/// `cargo test -p limarepdf-gui --bin limarepdf --profile ci-test -- --ignored
/// speed_of_aligning_300_pages --nocapture`.
#[test]
#[ignore]
fn speed_of_aligning_300_pages() {
    let (ctx, mut app) = new_app();
    let lines = wrap(&[PROSE, SECOND].join(" "));
    let mut pdf = Pdf::new();
    for p in 0..300 {
        pdf.insert_blank(p, 612.0, 792.0).unwrap();
        for (i, l) in lines.iter().enumerate() {
            let at = (72.0, 100.0 + 15.0 * i as f32);
            (pdf.add_text(p, at, l, FontChoice::default(), 12.0, [0.0; 3])).unwrap();
        }
    }
    let last = pdf.page_words(299).unwrap().len() - 1;
    assert!(app.add_tab(&ctx, pdf, None, None, false));
    app.act(&ctx, Act::Tool(Tool::Read));
    app.tabs[0].ed.selection = Selection::Words {
        anchor: (0, 0),
        end: (299, last),
    };
    settled_at(&mut app, &ctx, 1400.0);
    let t = std::time::Instant::now();
    let tab = &mut app.tabs[0];
    let said = tab.ed.align_text(&tab.pdf, TextAlign::Justified);
    let found = t.elapsed();
    tab.apply_actions(&ctx);
    let command = t.elapsed();
    let t = std::time::Instant::now();
    window(&mut app, &ctx, 1400.0, vec![]);
    let frame = t.elapsed();
    assert_eq!(said, None);
    assert_eq!(app.tabs[0].history().1, 1, "not one step");
    assert_eq!(app.tabs[0].info, None);
    eprintln!(
        "{} lines a page on 300 pages justified: the command {command:?} (finding the \
         paragraphs {found:?}), the frame after {frame:?}",
        lines.len()
    );
}
