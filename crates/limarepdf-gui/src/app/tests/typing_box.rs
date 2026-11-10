//! Text typed on the page as core sets it (the text design, 5.1, and its acceptance, 10):
//! each word in the font core writes it in, falling back where the font chosen lacks it, and
//! the inspector saying which words go to another font.

use super::*;
use limarepdf_core::BoxSetting;
use lopdf::{dictionary, Dictionary, Object, ObjectId};
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

/// The text `shapes` draw.
fn drawn_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<&egui::epaint::TextShape> {
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
    found
}

/// The font `word` is drawn in, in the drawn text that holds `whole`.
fn family_of(
    shapes: &[egui::epaint::ClippedShape],
    whole: &str,
    word: &str,
) -> Option<egui::FontFamily> {
    let job = &drawn_texts(shapes)
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

/// The rows of the drawn text that reads `text`, each its characters; `None` when no drawn
/// text reads it, or its rows' characters don't count up to it.
fn rows_of(shapes: &[egui::epaint::ClippedShape], text: &str) -> Option<Vec<String>> {
    let shape = (drawn_texts(shapes).into_iter()).find(|t| t.galley.job.text == text)?;
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    let mut rows = Vec::new();
    for row in &shape.galley.rows {
        let n = row.row.glyphs.len();
        rows.push(chars.get(at..at + n)?.iter().collect());
        at += n + usize::from(row.ends_with_newline);
    }
    (at == chars.len()).then_some(rows)
}

/// The rows drawn for `text` once they are `want`, else after a minute.
fn settled_rows(app: &mut App, ty: &Typist, text: &str, want: &[String]) -> Option<Vec<String>> {
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        let got = rows_of(&shapes(app, ty, vec![]), text);
        if got.as_deref() == Some(want) || Instant::now() > until {
            return got;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A text box and a multi-line form field wrap their text where it is written (the text
/// design, 5.1): the typing box's rows are the lines core breaks it into, each with the
/// spaces it ends with, and their characters count up to what was typed, which the caret
/// and selection go by.
#[test]
fn a_text_box_and_a_multi_line_field_wrap_where_their_text_is_written() {
    crate::font_list::test_family("Limare Han Test");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let ty = Typist::new(&ctx);
    let text = "The walrus and the carpenter were walking close at hand 漢字漢字漢字漢字 they \
                wept like anything to see such quantities of well-known sand";
    let lines = |set: &BoxSetting| -> Vec<String> {
        let (_, lines) = set.lines(text, None);
        lines.into_iter().map(|r| text[r].to_string()).collect()
    };

    // A text box with a thick border: its text is set inside it.
    let mut app = app_with(&ctx, 1);
    app.act(&ctx, Act::Tool(Tool::TextBox));
    app.tabs[0].ed.box_style.border = 4.0;
    let (min, k) = ty.page(&mut app);
    ty.click(&mut app, min + egui::vec2(100.0, 100.0) * k);
    ty.text(&mut app, text);
    let ed = &app.tabs[0].ed;
    let w = ed.writing.as_ref().expect("a text box being typed in");
    assert_eq!(w.text, text);
    let (width, style) = (w.rect[2] - w.rect[0], ed.box_style);
    let want = lines(&BoxSetting::text_box(width, style.border, style.size));
    assert!(want.len() > 2, "{want:?}");
    let got = settled_rows(&mut app, &ty, text, &want);
    assert_eq!(got, Some(want), "the text box");

    let mut pdf = form_doc(&[("Notes", 1)]);
    let notes = (pdf.doc.objects.iter())
        .find_map(|(&id, o)| {
            let d = o.as_dict().ok()?;
            (d.get(b"T").ok()?.as_str().ok()? == b"Notes").then_some(id)
        })
        .expect("the field");
    let d = pdf.doc.get_dictionary_mut(notes).unwrap();
    d.set("Ff", 4096);
    d.set("DA", lopdf::Object::string_literal("/Helv 10 Tf 0 g"));
    let dir = test_dir("box-lines");
    let path = dir.join("form.pdf");
    pdf.save(&path).unwrap();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    open_and_wait(&mut app, &ctx, &path, 1);
    ty.click_field(&mut app, "Notes", 0);
    ty.text(&mut app, text);
    let pdf = &app.tabs[0].pdf;
    let widget = pdf.form_fields()[0].widgets[0].id;
    let set = pdf.field_setting("Notes", widget).expect("a setting");
    let want = lines(&set);
    assert!(want.len() > 1, "{want:?}");
    let got = settled_rows(&mut app, &ty, text, &want);
    assert_eq!(got, Some(want), "the field");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Where each row of the drawn text that reads `text` reaches on the screen: its first
/// letter's left and its last letter's right, spaces at its ends left out. Glyphs are
/// placed by the pen, the advances before them, as epaint rounds their `pos.x`.
fn row_reaches(shapes: &[egui::epaint::ClippedShape], text: &str) -> Option<Vec<(f32, f32)>> {
    let shape = (drawn_texts(shapes).into_iter()).find(|t| t.galley.job.text == text)?;
    let rows = shape.galley.rows.iter().filter_map(|row| {
        let g = &row.row.glyphs;
        let first = g.iter().position(|c| !c.chr.is_whitespace())?;
        let last = g.iter().rposition(|c| !c.chr.is_whitespace())?;
        let pen = |i: usize| g[0].pos.x + g[..i].iter().map(|c| c.advance_width).sum::<f32>();
        let x = shape.pos.x + row.pos.x;
        Some((x + pen(first), x + pen(last + 1)))
    });
    Some(rows.collect())
}

/// The rows drawn for `text` once `fits` takes their reaches, else after a minute.
fn settled_reaches(
    app: &mut App,
    ty: &Typist,
    text: &str,
    fits: impl Fn(&[(f32, f32)]) -> bool,
) -> Vec<(f32, f32)> {
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        let got = row_reaches(&shapes(app, ty, vec![]), text).unwrap_or_default();
        if (!got.is_empty() && fits(&got)) || Instant::now() > until {
            return got;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether rows (their reaches, [`row_reaches`]) have their middles within a pixel of
/// `middle`.
fn centred_on(rows: &[(f32, f32)], middle: f32) -> bool {
    let off = |&(a, b): &(f32, f32)| (a + b) / 2.0 - middle;
    rows.iter().all(|r| off(r).abs() < 1.0)
}

/// PB-59: a text box and a form field show their text aligned while it is typed as it is
/// written, as `/Q` aligns it: a centred text box's rows centred in it, a right-aligned
/// multi-line field's rows ending where the field's text ends (its side less its border and
/// padding), a centred single-line field's text centred.
#[test]
fn a_text_box_and_fields_show_their_text_aligned_while_it_is_typed() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let ty = Typist::new(&ctx);
    let text = "The walrus and the carpenter were walking close at hand";
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    let rect = [72.0, 100.0, 272.0, 160.0];
    let style = limarepdf_core::TextStyle::default();
    let id = pdf.add_text_box(0, rect, text, &style, "Sam").unwrap();
    pdf.doc.get_dictionary_mut(id).unwrap().set("Q", 1);
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    assert!(app.add_tab(&ctx, pdf, None, None, false));
    let t = &mut app.tabs[0];
    t.ed.annots(&t.pdf, 0);
    t.ed.selection = Selection::Annot { page: 0, id };
    t.ed.write_selected();
    let (min, k) = ty.page(&mut app);
    let middle = min.x + (rect[0] + rect[2]) / 2.0 * k;
    let centred = |rows: &[(f32, f32)]| centred_on(rows, middle);
    let got = settled_reaches(&mut app, &ty, text, centred);
    let said = format!("the text box: {got:?}, middle {middle}");
    assert!(got.len() > 1 && centred(&got), "{said}");

    let mut pdf = form_doc(&[("Notes", 1), ("Name", 1)]);
    let field = |pdf: &Pdf, name: &[u8]| {
        (pdf.doc.objects.iter())
            .find_map(|(&id, o)| {
                let d = o.as_dict().ok()?;
                (d.get(b"T").ok()?.as_str().ok()? == name).then_some(id)
            })
            .expect("the field")
    };
    let notes = field(&pdf, b"Notes");
    let d = pdf.doc.get_dictionary_mut(notes).unwrap();
    d.set("Ff", 4096);
    d.set("DA", lopdf::Object::string_literal("/Helv 10 Tf 0 g"));
    d.set("Q", 2);
    let name = field(&pdf, b"Name");
    let d = pdf.doc.get_dictionary_mut(name).unwrap();
    d.set("DA", lopdf::Object::string_literal("/Helv 10 Tf 0 g"));
    d.set("Q", 1);
    let dir = test_dir("box-aligned");
    let path = dir.join("form.pdf");
    pdf.save(&path).unwrap();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    open_and_wait(&mut app, &ctx, &path, 1);
    let notes = "The walrus and the carpenter were walking close at hand they wept like \
                 anything to see such quantities of sand";
    ty.click_field(&mut app, "Notes", 0);
    ty.text(&mut app, notes);
    let (min, k) = ty.page(&mut app);
    let pdf = &app.tabs[0].pdf;
    let fields = pdf.form_fields();
    let widget = &fields.iter().find(|f| f.name == "Notes").unwrap().widgets[0];
    let set = pdf.field_setting("Notes", widget.id).expect("a setting");
    let end = min.x + (widget.bbox[2] - set.margin()) * k;
    let right = |rows: &[(f32, f32)]| rows.iter().all(|&(_, b)| (b - end).abs() < 1.0);
    let got = settled_reaches(&mut app, &ty, notes, right);
    let said = format!("the multi-line field: {got:?}, end {end}");
    assert!(got.len() > 1 && right(&got), "{said}");

    ty.click_field(&mut app, "Name", 0);
    ty.text(&mut app, "Walrus");
    let b = (app.tabs[0].pdf.form_fields().into_iter())
        .find(|f| f.name == "Name")
        .unwrap()
        .widgets[0]
        .bbox;
    let middle = min.x + (b[0] + b[2]) / 2.0 * k;
    let centred = |rows: &[(f32, f32)]| centred_on(rows, middle);
    let got = settled_reaches(&mut app, &ty, "Walrus", centred);
    let said = format!("the single-line field: {got:?}, middle {middle}");
    assert!(centred(&got), "{said}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A text box open for typing shows the look its inspector gives it meanwhile (ENH-13): a
/// bigger size and a thick border change its typing box's lines as they change the written
/// ones, as a new box's do with the Text box tool's look.
#[test]
fn an_open_text_box_follows_its_inspector_s_changes() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let ty = Typist::new(&ctx);
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    let text = "The walrus and the carpenter were walking close at hand";
    let style = limarepdf_core::TextStyle::default();
    let rect = [72.0, 100.0, 272.0, 160.0];
    let id = pdf.add_text_box(0, rect, text, &style, "Sam").unwrap();
    let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
    assert!(app.add_tab(&ctx, pdf, None, None, false));
    let t = &mut app.tabs[0];
    t.ed.annots(&t.pdf, 0);
    t.ed.selection = Selection::Annot { page: 0, id };
    t.ed.write_selected();
    ty.pause(&mut app);
    // Bigger, with a thick border, as the inspector makes it.
    let a = app.tabs[0].pdf.annotations()[0].clone();
    let style = limarepdf_core::TextStyle {
        size: 18.0,
        border: 6.0,
        ..a.text_style.expect("its look")
    };
    let edit = crate::editor::AnnotEdit {
        color: a.color,
        opacity: a.opacity,
        contents: a.contents.clone(),
        text: Some(style),
        shape: None,
    };
    app.tabs[0].ed.actions.push(EdAction::SetAnnot(0, id, edit));
    app.tabs[0].apply_actions(&ctx);
    let typed = app.tabs[0].ed.writing.as_ref().map(|w| w.text.clone());
    assert_eq!(typed.as_deref(), Some(text), "the box closed");
    let (_, lines) = BoxSetting::text_box(rect[2] - rect[0], 6.0, 18.0).lines(text, None);
    let want: Vec<String> = lines.into_iter().map(|r| text[r].to_string()).collect();
    let got = settled_rows(&mut app, &ty, text, &want);
    assert_eq!(got, Some(want));
}

/// A text box another app wrote with `extra` entries, on a 612 × 792 page, from 72 to 272
/// across and 480 to 640 up: `text` in Helvetica 12 with a border 1 wide, and no drawing of
/// its own.
fn another_apps_box(text: &str, extra: Dictionary) -> (Pdf, ObjectId) {
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 612.0, 792.0).unwrap();
    let rect: Vec<Object> = [72, 480, 272, 640].map(Object::from).into();
    let mut d = dictionary! {
        "Type" => "Annot", "Subtype" => "FreeText", "Rect" => rect,
        "Contents" => Object::string_literal(text),
        "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
        "BS" => dictionary! { "W" => 1 },
    };
    for (k, v) in extra {
        d.set(k, v);
    }
    let id = pdf.doc.add_object(d);
    let page = pdf.page_ids()[0];
    let annots = vec![Object::Reference(id)];
    pdf.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", annots);
    (pdf, id)
}

/// The lines of text box `id` as the app's pages draw it: what each line of its drawing
/// shows (each starts with a `Tm`).
fn written_lines(pdf: &Pdf, id: ObjectId) -> Vec<String> {
    let shown = Pdf::from_bytes(&pdf.clone().app_snapshot().unwrap(), None).unwrap();
    let d = shown.doc.get_dictionary(id).unwrap();
    let ap = shown.doc.dereference(d.get(b"AP").unwrap()).unwrap().1;
    let n = ap
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap()
        .as_reference()
        .unwrap();
    let s = shown.doc.get_object(n).unwrap().as_stream().unwrap();
    let content = s.decompressed_content().unwrap_or(s.content.clone());
    let mut lines: Vec<String> = Vec::new();
    for o in lopdf::content::Content::decode(&content)
        .unwrap()
        .operations
    {
        let shows = match o.operator.as_str() {
            "Tm" => {
                lines.push(String::new());
                continue;
            }
            "Tj" => o.operands,
            "TJ" => o.operands[0].as_array().cloned().unwrap_or_default(),
            _ => continue,
        };
        for s in shows {
            if let (Ok(b), Some(l)) = (s.as_str(), lines.last_mut()) {
                l.push_str(&String::from_utf8_lossy(b));
            }
        }
    }
    lines
}

/// Where the control with the keyboard is on the screen.
fn focused_bounds(tree: &egui::accesskit::TreeUpdate) -> Option<egui::Rect> {
    let (_, n) = tree.nodes.iter().find(|(id, _)| *id == tree.focus)?;
    let b = n.bounds()?;
    let at = |x: f64, y: f64| egui::pos2(x as f32, y as f32);
    Some(egui::Rect::from_min_max(at(b.x0, b.y0), at(b.x1, b.y1)))
}

/// The rows drawn for `text`, each without the spaces it ends with (as its line is
/// written), once they are `want`, else after a minute.
fn settled_shown_rows(
    app: &mut App,
    ty: &Typist,
    text: &str,
    want: &[String],
) -> Option<Vec<String>> {
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        let rows = rows_of(&shapes(app, ty, vec![]), text);
        let got: Option<Vec<String>> =
            rows.map(|rows| rows.iter().map(|r| r.trim_end().to_string()).collect());
        if got.as_deref() == Some(want) || Instant::now() > until {
            return got;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// PB-60: a text box another app wrote is typed in as LimarePDF writes it: inside its `/RD`
/// margins, inside a cloudy border's bumps, at the size its `/DS` style gives, and, turned
/// against its page, upright, as wide and as high as its text is written and centred over
/// it (drawn turned again once kept). The typing box's rows are the lines its drawing
/// shows, and it sits where they are written.
#[test]
fn a_text_box_another_app_wrote_is_typed_in_as_it_is_written() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install(&ctx);
    let ty = Typist::new(&ctx);
    let text = "The walrus and the carpenter were walking close at hand they wept like \
                anything to see such quantities of sand";
    let margins = dictionary! { "RD" => vec![12.into(), 12.into(), 12.into(), 12.into()] };
    let cloud = dictionary! { "BE" => dictionary! { "S" => "C", "I" => 2 } };
    let style = dictionary! {
        "DA" => Object::string_literal("/Helv 0 Tf 0 g"),
        "DS" => Object::string_literal("font: 18pt Helvetica"),
    };
    let turned = dictionary! { "Rotate" => 90 };
    let mut out = Vec::new();
    for (what, extra) in [
        ("margins", margins),
        ("a cloud", cloud),
        ("a style's size", style),
        ("turned", turned),
    ] {
        let (pdf, id) = another_apps_box(text, extra);
        let want = written_lines(&pdf, id);
        let mut app = App::with(Settings::from_file(None), Recent::from_file(None));
        assert!(app.add_tab(&ctx, pdf, None, None, false));
        let t = &mut app.tabs[0];
        t.ed.annots(&t.pdf, 0);
        t.ed.selection = Selection::Annot { page: 0, id };
        t.ed.write_selected();
        let (min, k) = ty.page(&mut app);
        let got = settled_shown_rows(&mut app, &ty, text, &want);
        if got.as_ref() != Some(&want) {
            out.push(format!("{what}: typed {got:?}, written {want:?}"));
        }
        // Where its lines are written, from its box as shown (in points, y down): inside the
        // margins (12) or the bumps (half the border and 9.5); turned, its sides swapped
        // about its middle.
        let b = app.tabs[0].pdf.annotations()[0].bbox;
        let inset = |n: f32| [b[0] + n, b[1] + n, b[2] - n, b[3] - n];
        let (x, y, w, h) = (
            (b[0] + b[2]) / 2.0,
            (b[1] + b[3]) / 2.0,
            b[3] - b[1],
            b[2] - b[0],
        );
        let area = match what {
            "margins" => inset(12.0),
            "a cloud" => inset(10.0),
            "turned" => [x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0],
            _ => b,
        };
        let at = |x: f32, y: f32| min + egui::vec2(x, y) * k;
        let written = egui::Rect::from_min_max(at(area[0], area[1]), at(area[2], area[3]));
        let typed = focused_bounds(&ty.pause(&mut app));
        let near = |r: egui::Rect| {
            (r.min - written.min).length() < 1.0 && (r.max - written.max).length() < 1.0
        };
        if !typed.is_some_and(near) {
            out.push(format!(
                "{what}: typed in {typed:?}, written in {written:?}"
            ));
        }
    }
    assert!(out.is_empty(), "{out:#?}");
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
