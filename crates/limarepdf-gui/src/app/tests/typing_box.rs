//! Text typed on the page as core sets it (the text design, 5.1, and its acceptance, 10):
//! each word in the font core writes it in, falling back where the font chosen lacks it, and
//! the inspector saying which words go to another font.

use super::*;
use std::time::{Duration, Instant};

/// A frame of the window 1400 points wide with `events`, timed as [`Typist::frame`] times
/// it: what it draws.
fn shapes(app: &mut App, ty: &Typist, events: Vec<egui::Event>) -> Vec<egui::epaint::ClippedShape> {
    ty.time.set(ty.time.get() + 0.05);
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, TEST_HEIGHT),
        )),
        time: Some(ty.time.get()),
        events,
        ..Default::default()
    };
    let mut f = eframe::Frame::_new_kittest();
    let mut out = ty.ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut f));
    out.textures_delta.clear();
    out.shapes
}

/// The font `word` is drawn in, in the drawn text that holds `whole`.
fn family_of(
    shapes: &[egui::epaint::ClippedShape],
    whole: &str,
    word: &str,
) -> Option<egui::FontFamily> {
    fn texts<'a>(s: &'a egui::Shape, out: &mut Vec<&'a egui::epaint::TextShape>) {
        match s {
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            egui::Shape::Text(t) => out.push(t),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for c in shapes {
        texts(&c.shape, &mut found);
    }
    let job = &found
        .into_iter()
        .find(|t| t.galley.job.text.contains(whole))?
        .galley
        .job;
    let at = job.text.find(word)?;
    let s =
        (job.sections.iter()).find(|s| s.byte_range.start.0 <= at && at < s.byte_range.end.0)?;
    Some(s.format.font_id.family.clone())
}

/// Whether `f` is an installed font's own file, as the typing fonts name them, not a
/// stand-in drawn while it loads or instead of it.
fn a_face(f: &Option<egui::FontFamily>) -> bool {
    matches!(f, Some(egui::FontFamily::Name(n)) if n.starts_with("file "))
}

/// The fonts `words` are drawn in, in the drawn text that holds `whole`, once frames have
/// let `done` say their fonts have loaded (a minute at most: fonts load on other threads).
fn settled_families(
    app: &mut App,
    ty: &Typist,
    whole: &str,
    words: &[&str],
    done: impl Fn(&[Option<egui::FontFamily>]) -> bool,
) -> Vec<Option<egui::FontFamily>> {
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        let drawn = shapes(app, ty, vec![]);
        let got: Vec<_> = words.iter().map(|w| family_of(&drawn, whole, w)).collect();
        if done(&got) || Instant::now() > until {
            return got;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A form field's value and a text box's text are typed in the fonts they are written in,
/// as text on the page is: the field's font (Helvetica here) and Helvetica, and the words
/// that lacks in the font core writes them in, not the interface's font.
#[test]
fn a_form_field_and_a_text_box_are_typed_in_the_fonts_they_are_written_in() {
    crate::font_list::test_family("Limare Han Test");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let ty = Typist::new(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let dir = test_dir("typed-faces");
    let path = dir.join("form.pdf");
    form_file(&path, "");
    open_and_wait(&mut app, &ctx, &path, 1);
    let sans = Some(egui::FontFamily::Name("Liberation Sans".into()));
    let done = |f: &[Option<egui::FontFamily>]| f[0] == sans && a_face(&f[1]);
    ty.click_field(&mut app, "Name", 0);
    ty.text(&mut app, "Ann 漢字");
    let got = settled_families(&mut app, &ty, "Ann 漢字", &["Ann", "漢字"], done);
    assert!(done(&got), "the field: {got:?}");
    std::fs::remove_dir_all(&dir).unwrap();

    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::Tool(Tool::TextBox));
    let (min, k) = ty.page(&mut app);
    ty.click(&mut app, min + egui::vec2(100.0, 100.0) * k);
    ty.text(&mut app, "Box 漢字");
    let typed = app.tabs[0].ed.writing.as_ref().map(|w| w.text.clone());
    assert_eq!(
        typed.as_deref(),
        Some("Box 漢字"),
        "not typed in a text box"
    );
    let got = settled_families(&mut app, &ty, "Box 漢字", &["Box", "漢字"], done);
    assert!(done(&got), "the text box: {got:?}");
}

/// A line LimarePDF wrote, opened for editing, shows its words as they are on the page: a
/// font LimarePDF made has no preview of its own glyphs, so its words are drawn in the
/// installed font it was made from, or the fonts core would write them in, not in a
/// stand-in that lacks them.
#[test]
fn a_line_s_words_in_a_font_limarepdf_wrote_are_drawn_as_they_are() {
    crate::font_list::test_family("Limare Han Test");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let ty = Typist::new(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 400.0).unwrap();
    let line = "Walrus 漢字 ends";
    pdf.add_text(
        0,
        (72.0, 100.0),
        line,
        FontChoice::default(),
        14.0,
        [0.0; 3],
    )
    .unwrap();
    assert!(app.add_tab(&ctx, pdf, None, None, false));
    app.act(&ctx, Act::Tool(Tool::Read));
    let tree = ty.pause(&mut app);
    let (page, _) = node(&tree, "Page 1").expect("the page");
    let words = app.tabs[0].pdf.page_words(0).unwrap();
    let ends = words.iter().find(|w| w.text == "ends").expect("ends").bbox;
    let at = egui::vec2((ends[0] + ends[2]) / 2.0, (ends[1] + ends[3]) / 2.0);
    let p = page.min + at * (page.width() / 612.0);
    ty.frame(&mut app, vec![egui::Event::PointerMoved(p)]);
    for _ in 0..2 {
        ty.frame(&mut app, vec![press(p, true)]);
        ty.frame(&mut app, vec![press(p, false)]);
    }
    let tree = ty.pause(&mut app);
    let name = format!("Editing text: {line}");
    assert!(node(&tree, &name).is_some(), "the line isn't open");
    let got = settled_families(&mut app, &ty, line, &["漢字"], |f| a_face(&f[0]));
    assert!(a_face(&got[0]), "{got:?}");
}

/// Typed in a line, words its font lacks go to an installed font that has them, and the
/// inspector says which, and to what: "Not in Helvetica: 漢字. Those characters use …".
#[test]
fn the_inspector_says_which_words_go_to_another_font() {
    crate::font_list::test_family("Limare Han Test");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    lines_tab(&mut app, &ctx, 400.0, &paragraph(72.0, 100.0, 3, 1, false));
    edit_otter(&mut app, &ctx, 1400.0, "opened");
    let says = inspector_says(&settled_at(&mut app, &ctx, 1400.0));
    assert!(
        !says.iter().any(|s| s.starts_with("Not in Helvetica")),
        "{says:?}"
    );
    window(
        &mut app,
        &ctx,
        1400.0,
        vec![egui::Event::Text("漢字".into())],
    );
    let says = inspector_says(&settled_at(&mut app, &ctx, 1400.0));
    let gone = limarepdf_core::fallbacks(FontChoice::default(), "The 漢字 ate");
    let [(word, used)] = &gone[..] else {
        panic!("{gone:?}");
    };
    assert_eq!(word, "漢字");
    let used = used.family_name();
    let want = format!("Not in Helvetica: 漢字. Those characters use {used}.");
    assert!(says.contains(&want), "{says:?}");
    // Readable: the interface falls back to installed fonts for what its own lack (ENH-33),
    // in each of its fonts, once they are read at start.
    let families = [
        egui::FontFamily::Proportional,
        theme::semibold(),
        egui::FontFamily::Monospace,
    ];
    let drawn = |ctx: &egui::Context| {
        ctx.fonts_mut(|f| {
            (families.iter()).all(|family| {
                let id = egui::FontId::new(12.0, family.clone());
                f.has_glyphs(&id, "漢字 한 あ")
            })
        })
    };
    let until = Instant::now() + Duration::from_secs(60);
    while !drawn(&ctx) && Instant::now() < until {
        window(&mut app, &ctx, 1400.0, vec![]);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(drawn(&ctx), "the interface can't draw 漢字 한 あ");
}
