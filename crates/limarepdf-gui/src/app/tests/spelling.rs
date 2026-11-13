//! Spelling (NEW-06): words spelt wrongly marked as they are typed, the word being typed left
//! alone until it is left; a right-click on one offers its suggestions, Ignore all and Add to
//! dictionary; Review > Spelling (F7) goes through the comments and fields' words; and
//! dictionaries are added from their files.

use super::*;
use egui::accesskit::{Node, NodeId, Role, TreeUpdate};
use std::collections::HashMap;
use std::ops::Range;

fn test_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    ctx
}

/// An app checking Canadian English, its settings in `file` (none: kept nowhere).
fn canadian_app(file: Option<PathBuf>) -> App {
    let mut s = Settings::from_file(file);
    s.spelling_language = Some("en-CA".into());
    App::with(s, Recent::from_file(None))
}

/// Frames until the box `id` has drawn its marks: its dictionary read on another thread.
fn marks_of(app: &mut App, ty: &Typist, id: egui::Id) -> Vec<Range<usize>> {
    for _ in 0..400 {
        ty.frame(app, vec![]);
        if let Some(marks) = crate::spell::marked(ty.ctx, id) {
            return marks;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the dictionary wasn't read");
}

/// One word's marks.
fn one(r: Range<usize>) -> Vec<Range<usize>> {
    vec![r]
}

/// A right-click at `p`, then a pause.
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
    ty.pause(app)
}

/// The named items of the menus open, in order down them, with their middles.
fn menu_items(tree: &TreeUpdate) -> Vec<(String, egui::Pos2)> {
    let nodes: HashMap<NodeId, &Node> = tree.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let menus = tree.nodes.iter().filter(|(_, n)| n.role() == Role::Menu);
    let mut next: Vec<NodeId> = menus.map(|(id, _)| *id).collect();
    let mut out = Vec::new();
    while let Some(id) = next.pop() {
        let Some(n) = nodes.get(&id) else { continue };
        next.extend(n.children().iter().copied());
        if let (Some(name), Some(b), false) = (n.label(), n.bounds(), n.role() == Role::Menu) {
            let mid = |a: f64, b: f64| ((a + b) / 2.0) as f32;
            out.push((
                name.to_string(),
                egui::pos2(mid(b.x0, b.x1), mid(b.y0, b.y1)),
            ));
        }
    }
    out.sort_by(|a, b| a.1.y.total_cmp(&b.1.y));
    out
}

fn item(items: &[(String, egui::Pos2)], name: &str) -> egui::Pos2 {
    let found = items.iter().find(|(n, _)| n == name);
    found
        .map(|(_, p)| *p)
        .unwrap_or_else(|| panic!("no {name} in {items:?}"))
}

/// Whether a label shows `text` (a label's text is its value).
fn shows(tree: &TreeUpdate, text: &str) -> bool {
    (tree.nodes.iter()).any(|(_, n)| n.role() == Role::Label && n.value() == Some(text))
}

/// The new text being typed, as screen readers have it.
fn new_text(tree: &TreeUpdate) -> String {
    let typed = (tree.nodes.iter())
        .find(|(_, n)| n.label() == Some(page_text::NAME))
        .and_then(|(_, n)| n.value());
    typed.expect("no new text").to_string()
}

/// The middle of the new text's `nth` character of `of`.
fn at_char(tree: &TreeUpdate, nth: usize, of: usize) -> egui::Pos2 {
    let (r, _) = node(tree, page_text::NAME).expect("the new text");
    egui::pos2(
        r.min.x + r.width() * (nth as f32 + 0.5) / of as f32,
        r.center().y,
    )
}

/// Words spelt wrongly are marked once left: "teh" with the caret after it isn't, until a
/// space follows it, as in word processors. A right-click on one offers its suggestions,
/// then Ignore all and Add to dictionary, before the box's own commands; a suggestion
/// replaces the word as typing would.
#[test]
fn words_spelt_wrongly_are_marked_and_a_suggestion_replaces_one() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = canadian_app(None);
    let note = typing_doc(&mut app, &ctx);
    start_typing(&mut app, &ty, "new text", note);
    ty.text(&mut app, "Teh colour teh");
    let id = app.tabs[0].ed.typing_id().expect("not typing");
    assert_eq!(
        marks_of(&mut app, &ty, id),
        one(0..3),
        "the word being typed was marked"
    );
    ty.text(&mut app, " ");
    assert_eq!(marks_of(&mut app, &ty, id), [0..3, 11..14]);
    let tree = ty.pause(&mut app);
    let tree = right_click(&mut app, &ty, at_char(&tree, 1, 15));
    let items = menu_items(&tree);
    let names: Vec<&str> = items.iter().map(|(n, _)| n.as_str()).collect();
    let the = (names.iter().position(|n| *n == "The")).unwrap_or_else(|| panic!("{names:?}"));
    let ignore = names
        .iter()
        .position(|n| *n == "Ignore all")
        .expect("no Ignore all");
    let add = names
        .iter()
        .position(|n| *n == "Add to dictionary")
        .expect("no Add");
    let cut = names
        .iter()
        .position(|n| n.starts_with("Cut"))
        .expect("no Cut");
    assert!(the < ignore && ignore < add && add < cut, "{names:?}");
    ty.click(&mut app, item(&items, "The"));
    let tree = ty.pause(&mut app);
    assert_eq!(new_text(&tree), "The colour teh ");
    assert_eq!(marks_of(&mut app, &ty, id), one(11..14));
    assert_eq!(app.tabs[0].ed.typing_id(), Some(id), "the text closed");
}

/// A form field being filled in and a comment's box in the inspector mark their words too.
#[test]
fn form_fields_and_the_inspectors_comment_box_mark_words() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = canadian_app(None);
    let note = typing_doc(&mut app, &ctx);
    for case in ["form field", "comment in the inspector"] {
        start_typing(&mut app, &ty, case, note);
        let id = ctx
            .memory(|m| m.focused())
            .expect("no box has the keyboard");
        ty.frame(&mut app, vec![key_with(Key::End, egui::Modifiers::NONE)]);
        ty.text(&mut app, " teh ");
        let marks = marks_of(&mut app, &ty, id);
        assert_eq!(marks.len(), 1, "{case}: {marks:?}");
        ty.key(&mut app, Key::Escape, egui::Modifiers::NONE);
        ty.pause(&mut app);
    }
}

/// Ignore all leaves every "teh", in any capitals, for the session; Add to dictionary keeps
/// the word as written for next time, in the word list beside the settings.
#[test]
fn ignore_all_and_add_to_dictionary_leave_words_unmarked() {
    let dir = std::env::temp_dir().join(format!("limarepdf-spelling-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = canadian_app(Some(dir.join("settings.txt")));
    let note = typing_doc(&mut app, &ctx);
    start_typing(&mut app, &ty, "new text", note);
    ty.text(&mut app, "Limare teh Teh ");
    let id = app.tabs[0].ed.typing_id().expect("not typing");
    assert_eq!(marks_of(&mut app, &ty, id), [0..6, 7..10, 11..14]);
    let tree = ty.pause(&mut app);
    let tree = right_click(&mut app, &ty, at_char(&tree, 8, 15));
    ty.click(&mut app, item(&menu_items(&tree), "Ignore all"));
    assert_eq!(marks_of(&mut app, &ty, id), one(0..6));
    let tree = ty.pause(&mut app);
    let tree = right_click(&mut app, &ty, at_char(&tree, 2, 15));
    ty.click(&mut app, item(&menu_items(&tree), "Add to dictionary"));
    assert_eq!(marks_of(&mut app, &ty, id), Vec::<Range<usize>>::new());
    ty.pause(&mut app);
    let list = std::fs::read_to_string(dir.join("words.txt")).expect("no word list");
    assert_eq!(list, "Limare\n", "ignored words aren't kept");
    assert!(app.settings.take_unsaved().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Check spelling as you type turned off marks nothing, and is kept; the language chosen in
/// Spelling's menu is kept too.
#[test]
fn spelling_turned_off_marks_nothing_and_the_language_is_kept() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = canadian_app(None);
    let note = typing_doc(&mut app, &ctx);
    start_typing(&mut app, &ty, "new text", note);
    ty.text(&mut app, "teh color ");
    let id = app.tabs[0].ed.typing_id().expect("not typing");
    assert_eq!(marks_of(&mut app, &ty, id), [0..3, 4..9]);
    app.act(&ctx, Act::SpellAsYouType);
    assert!(!app.settings.spelling);
    ty.pause(&mut app);
    assert_eq!(crate::spell::marked(&ctx, id), Some(Vec::new()));
    app.act(&ctx, Act::SpellAsYouType);
    let (langs, active) = crate::spell::languages(&ctx);
    assert_eq!(langs[active].name, "English (Canada)");
    let us = langs
        .iter()
        .position(|l| l.tag == "en-US")
        .expect("no US English");
    app.act(&ctx, Act::SpellingLanguage(us));
    assert_eq!(app.settings.spelling_language.as_deref(), Some("en-US"));
    ty.pause(&mut app);
    // "color" is right in US English.
    assert_eq!(marks_of(&mut app, &ty, id), one(0..3));
}

/// Press Spelling's key: F7, Option+Cmd+L on a Mac.
fn press_spelling(app: &mut App, ty: &Typist) {
    let k = actions::get(Act::Spelling).keys[0];
    ty.key(app, k.logical_key, k.modifiers);
}

/// Review > Spelling (its key) goes through the comments' words spelt wrongly; Change all changes
/// every one alike, as one step to undo, and the view then says there are none.
#[test]
fn review_spelling_changes_every_word_alike_as_one_step() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = canadian_app(None);
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    let yellow = [1.0, 1.0, 0.0];
    pdf.add_note(0, (100.0, 600.0), yellow, "Teh otter swims", "Tester")
        .unwrap();
    pdf.add_note(
        0,
        (100.0, 400.0),
        yellow,
        "The seal and teh walrus",
        "Tester",
    )
    .unwrap();
    assert!(app.add_tab(&ctx, pdf, None, None, false));
    ty.pause(&mut app);
    press_spelling(&mut app, &ty);
    assert_eq!(app.tabs[0].side, Side::Spelling);
    let mut tree = ty.pause(&mut app);
    for _ in 0..400 {
        if node(&tree, "Change all").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        tree = ty.frame(&mut app, vec![]);
    }
    assert!(shows(&tree, "2 words to check"), "not two to check");
    assert!(shows(&tree, "Note on page 1"));
    assert!(shows(&tree, "English (Canada)"));
    ty.click(&mut app, middle(&tree, "Change all"));
    let tree = ty.pause(&mut app);
    let contents = |app: &App| -> Vec<String> {
        app.tabs[0]
            .pdf
            .annotations()
            .into_iter()
            .map(|a| a.contents)
            .collect()
    };
    assert_eq!(
        contents(&app),
        ["The otter swims", "The seal and the walrus"]
    );
    let none = "No words spelt wrongly in the comments, text boxes and form fields.";
    assert!(shows(&tree, none), "still words to check");
    app.act(&ctx, Act::Undo);
    assert_eq!(
        contents(&app),
        ["Teh otter swims", "The seal and teh walrus"]
    );
}

/// Spelling's key from text being typed opens Spelling, the text still being typed, as in
/// Word.
#[test]
fn f7_while_typing_opens_spelling() {
    let ctx = test_ctx();
    let ty = Typist::new(&ctx);
    let mut app = canadian_app(None);
    let note = typing_doc(&mut app, &ctx);
    start_typing(&mut app, &ty, "new text", note);
    ty.text(&mut app, "Seal");
    let id = app.tabs[0].ed.typing_id().expect("not typing");
    press_spelling(&mut app, &ty);
    assert_eq!(app.tabs[0].side, Side::Spelling);
    assert_eq!(app.tabs[0].ed.typing_id(), Some(id), "the typing ended");
}

/// Dictionaries added from their files go to the dictionaries folder and into the Language
/// menu; one that can't be read, and a `.dic` without its `.aff`, are said, with why.
#[test]
fn dictionaries_are_added_and_those_that_cant_be_are_said() {
    let dir = std::env::temp_dir().join(format!("limarepdf-dicts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let picked = dir.join("picked");
    std::fs::create_dir_all(&picked).unwrap();
    let write = |name: &str, data: &[u8]| {
        std::fs::write(picked.join(name), data).unwrap();
        picked.join(name)
    };
    let de_aff = write("de_DE.aff", b"SET UTF-8\n");
    let de_dic = write("de_DE.dic", "2\nHaus\nStraße\n".as_bytes());
    write("ru_RU.aff", b"SET KOI8-R\n");
    let ru_dic = write("ru_RU.dic", b"1\n\xc1\n");
    let lone = write("lone.dic", b"1\nx\n");
    let ctx = test_ctx();
    let mut app = canadian_app(Some(dir.join("settings").join("settings.txt")));
    app.spelling_frame(&ctx);
    app.add_dictionaries(&ctx, &[de_dic, de_aff, ru_dic, lone]);
    let folder = dir.join("settings").join("dictionaries");
    assert!(folder.join("de_DE.dic").is_file() && folder.join("de_DE.aff").is_file());
    assert!(
        !folder.join("ru_RU.dic").exists(),
        "an unreadable one was kept"
    );
    let (langs, _) = crate::spell::languages(&ctx);
    assert!(
        langs.iter().any(|l| l.name == "German (Germany)"),
        "{langs:?}"
    );
    assert!(app.status.contains("German (Germany)"), "{}", app.status);
    let Dialog::Message(title, text) = &app.dialog else {
        panic!("nothing said");
    };
    assert_eq!(title, "Some dictionaries not added");
    assert!(text.contains("ru_RU.dic is written in KOI8-R"), "{text}");
    assert!(text.contains("lone.dic needs its .aff file"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The `de_DE` pair in a dictionaries folder: its `.aff` and `.dic`, each if there.
fn de_pair(folder: &Path) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let read = |kind: &str| std::fs::read(folder.join(format!("de_DE.{kind}"))).ok();
    (read("aff"), read("dic"))
}

/// Replacing (or adding) the `de_DE` dictionary with this app's writes numbered in `fail`
/// failing (A-11): the folder (`de_DE` in it first when `old`) and what was said after.
fn replace_de_failing(name: &str, old: bool, fail: &[usize]) -> (PathBuf, PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("limarepdf-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let picked = dir.join("picked");
    let folder = dir.join("settings").join("dictionaries");
    std::fs::create_dir_all(&picked).unwrap();
    std::fs::create_dir_all(&folder).unwrap();
    if old {
        std::fs::write(folder.join("de_DE.aff"), b"SET UTF-8\n").unwrap();
        std::fs::write(folder.join("de_DE.dic"), "1\nHaus\n".as_bytes()).unwrap();
    }
    std::fs::write(picked.join("de_DE.aff"), b"SET UTF-8\nTRY e\n").unwrap();
    std::fs::write(picked.join("de_DE.dic"), "2\nHaus\nStraße\n".as_bytes()).unwrap();
    let ctx = test_ctx();
    let mut app = canadian_app(Some(dir.join("settings").join("settings.txt")));
    app.spelling_frame(&ctx);
    crate::settings::write_seam::fail_writes(fail);
    app.add_dictionaries(&ctx, &[picked.join("de_DE.dic"), picked.join("de_DE.aff")]);
    crate::settings::write_seam::fail_writes(&[]);
    let Dialog::Message(_, text) = &app.dialog else {
        panic!("nothing said");
    };
    (dir, folder, text.clone())
}

/// A dictionary replaced whose `.dic` can't be written leaves the one there before whole: never
/// its new affix rules with the old words (A-11).
#[test]
fn a_dictionary_half_replaced_is_put_back() {
    let (dir, folder, text) = replace_de_failing("half-dict", true, &[2]);
    let old = (Some(b"SET UTF-8\n".to_vec()), Some(b"1\nHaus\n".to_vec()));
    assert_eq!(de_pair(&folder), old, "the old pair wasn't kept whole");
    assert!(text.contains("de_DE.dic couldn't be saved"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A new dictionary whose `.dic` can't be written leaves nothing of it behind (A-11).
#[test]
fn a_new_dictionary_half_written_leaves_nothing() {
    let (dir, folder, text) = replace_de_failing("half-new-dict", false, &[2]);
    assert_eq!(de_pair(&folder), (None, None), "a file was left behind");
    assert!(text.contains("de_DE.dic couldn't be saved"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// When the dictionary there before can't be put back either, it isn't listed rather than
/// listed mixed, and that is said (A-11).
#[test]
fn a_dictionary_that_cant_be_put_back_isnt_left_mixed() {
    let (dir, folder, text) = replace_de_failing("unrestored-dict", true, &[2, 3]);
    let words = "2\nHaus\nStraße\n".as_bytes().to_vec();
    let new = (Some(b"SET UTF-8\nTRY e\n".to_vec()), Some(words));
    let old = (Some(b"SET UTF-8\n".to_vec()), Some(b"1\nHaus\n".to_vec()));
    let pair = de_pair(&folder);
    let listed = pair.0.is_some() && pair.1.is_some();
    assert!(!listed || pair == old || pair == new, "mixed: {pair:?}");
    assert!(text.contains("couldn't be put back"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A chosen `.dic` or extension too large to be a dictionary is refused as too large, without
/// being read whole, and nothing of it is kept.
#[test]
fn dictionaries_too_large_are_refused_and_nothing_kept() {
    let dir = std::env::temp_dir().join(format!("limarepdf-big-dicts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let picked = dir.join("picked");
    std::fs::create_dir_all(&picked).unwrap();
    // Sparse: a file's length costs nothing until it is read.
    let sized = |name: &str, len: u64| {
        std::fs::File::create(picked.join(name))
            .unwrap()
            .set_len(len)
            .unwrap();
        picked.join(name)
    };
    std::fs::write(picked.join("big.aff"), b"SET UTF-8\n").unwrap();
    let big_dic = sized("big.dic", (64 << 20) + 1);
    let big_oxt = sized("big.oxt", (128 << 20) + 1);
    let ctx = test_ctx();
    let mut app = canadian_app(Some(dir.join("settings").join("settings.txt")));
    app.spelling_frame(&ctx);
    app.add_dictionaries(&ctx, &[big_oxt, big_dic]);
    let folder = dir.join("settings").join("dictionaries");
    let kept = std::fs::read_dir(&folder).map_or(0, |d| d.count());
    assert_eq!(kept, 0, "something was kept");
    let Dialog::Message(title, text) = &app.dialog else {
        panic!("nothing said");
    };
    assert_eq!(title, "Dictionary not added");
    for name in ["big.dic", "big.oxt"] {
        let said = format!("{name} is too large to be a dictionary");
        assert!(text.contains(&said), "{text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
