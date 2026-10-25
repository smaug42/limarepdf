//! Text beyond Western European in every place LimarePDF writes text (the Unicode text
//! design, batch 2): new and edited lines, text boxes, fill-and-sign text, form values,
//! headers, footers, Bates numbers, watermarks, stamps and selected text as a plain page.
//! Each is read back after saving, and text the standard fonts can write still embeds
//! nothing. Emoji are written where a font has them, and refused with a message where
//! not (BUG-24).

use limarepdf_core::render::{self, Renderer};
use limarepdf_core::{
    typed_signature, Bates, Family, FieldValue, FontChoice, HeaderFooter, InkColor, Pdf, SignStyle,
    Stamp, StampColor, StampPlace, TextEdit, TextStyle, Watermark,
};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream, StringFormat};

const W: f32 = 400.0;
const H: f32 = 300.0;

/// Words in the scripts LimarePDF writes, each beyond WinAnsi.
const POLISH: &str = "Zażółć gęślą jaźń";
const GREEK: &str = "Ελληνικά γράμματα";
const RUSSIAN: &str = "Съешь же ещё этих";
const VIETNAMESE: &str = "Tiếng Việt có dấu";

fn page() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, W, H).unwrap();
    p
}

fn font(family: Family, bold: bool, italic: bool) -> FontChoice {
    FontChoice {
        family,
        bold,
        italic,
    }
}

fn reopened(p: &mut Pdf) -> Pdf {
    Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap()
}

/// The fonts LimarePDF embedded, by their names without the subset tag.
fn embedded(p: &Pdf) -> Vec<String> {
    let mut out: Vec<String> = p
        .doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .filter(|d| d.has(b"LimarePDF"))
        .map(|d| {
            let base = d.get(b"BaseFont").unwrap().as_name().unwrap();
            let base = String::from_utf8_lossy(base).into_owned();
            base.split_once('+').unwrap().1.to_string()
        })
        .collect();
    out.sort();
    out
}

/// How many pixels in `r` (pixels, from the top left) of the first page aren't white.
fn ink(p: &mut Pdf, r: [usize; 4]) -> usize {
    let doc = render::load(p.render_snapshot().unwrap(), None).unwrap();
    let b = Renderer::new(&doc).render(0, 1.0).unwrap();
    let mut n = 0;
    for y in r[1]..r[3].min(b.height) {
        for x in r[0]..r[2].min(b.width) {
            let i = 4 * (y * b.width + x);
            if b.rgba[i..i + 3].iter().any(|&c| c < 200) {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn new_lines_in_each_family_read_back_and_draw() {
    let mut p = page();
    let lines = [
        (font(Family::Helvetica, false, false), POLISH),
        (font(Family::Times, true, false), GREEK),
        (font(Family::Courier, false, true), RUSSIAN),
        (font(Family::Helvetica, false, false), VIETNAMESE),
    ];
    for (i, (f, text)) in lines.iter().enumerate() {
        let y = 40.0 + 40.0 * i as f32;
        p.add_text(0, (20.0, y), text, *f, 14.0, [0.0; 3]).unwrap();
    }
    let mut back = reopened(&mut p);
    let text = back.page_text(0);
    for (_, t) in lines {
        assert!(text.contains(t), "{t} in {text:?}");
    }
    // One font per face: the Vietnamese line grew the Polish line's.
    assert_eq!(
        embedded(&back),
        [
            "LiberationMono-Italic",
            "LiberationSans",
            "LiberationSerif-Bold"
        ]
    );
    // Drawn: every line has ink where it was put.
    for i in 0..4 {
        let y = 40 + 40 * i;
        assert!(ink(&mut back, [20, y - 12, 200, y + 2]) > 50, "line {i}");
    }
}

#[test]
fn text_the_standard_fonts_can_write_embeds_nothing() {
    let mut p = page();
    let helvetica = FontChoice::default();
    p.add_text(
        0,
        (20.0, 40.0),
        "Café œuvre € “quoted”",
        helvetica,
        12.0,
        [0.0; 3],
    )
    .unwrap();
    p.add_header_footer(&[0], &HeaderFooter::default()).unwrap();
    let style = TextStyle::default();
    p.add_text_box(0, [20.0, 100.0, 200.0, 140.0], "Müller", &style, "Sam")
        .unwrap();
    assert!(embedded(&p).is_empty());
}

#[test]
fn an_edited_line_is_set_in_the_embedded_face_of_its_kind() {
    let mut p = page();
    let page_id = p.page_ids()[0];
    let f1 = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Times-Bold",
        "Encoding" => "WinAnsiEncoding",
    });
    let content =
        b"BT /F1 20 Tf 20 200 Td (Hello) Tj ET\nBT 3 Tr /F1 12 Tf 20 100 Td (hidden) Tj ET\n";
    let c = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.to_vec()));
    let d = p.doc.get_dictionary_mut(page_id).unwrap();
    d.set("Contents", c);
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => f1 } },
    );
    let edit = |text: &str| TextEdit {
        text: text.into(),
        font: None,
        size: None,
        color: None,
        parts: Vec::new(),
    };
    p.edit_text(0, 0, &edit(GREEK)).unwrap();
    let hidden = p
        .page_objects(0)
        .unwrap()
        .into_iter()
        .find(|o| o.hidden)
        .unwrap();
    p.edit_text(0, hidden.id, &edit(RUSSIAN)).unwrap();
    let back = reopened(&mut p);
    let text = back.page_text(0);
    assert!(text.contains(GREEK) && text.contains(RUSSIAN), "{text:?}");
    assert!(
        !text.contains("Hello") && !text.contains("hidden"),
        "{text:?}"
    );
    // The visible line keeps its kind (serif, bold); invisible text is in Helvetica's face.
    assert_eq!(embedded(&back), ["LiberationSans", "LiberationSerif-Bold"]);
    let objs = back.page_objects(0).unwrap();
    assert!(objs
        .iter()
        .any(|o| o.hidden && o.text.as_deref() == Some(RUSSIAN)));
}

/// A test font as an installed family, by name: Source Serif 4 (OpenType CFF), Cantarell
/// (variable) or [`EMOJI`].
fn installed(name: &str) -> Family {
    static NUMBERS: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let numbers = NUMBERS.get_or_init(|| {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/fonts");
        let files = [
            dir.join("SourceSerif4-Regular.otf"),
            dir.join("Cantarell-VF.otf"),
            dir.join("LimareEmojiTest.ttf"),
        ];
        limarepdf_core::add_font_files(&files.iter().map(|p| p.as_path()).collect::<Vec<_>>())
    });
    let n = numbers
        .iter()
        .copied()
        .find(|&n| limarepdf_core::typeface(n).unwrap().name == name)
        .unwrap();
    Family::Installed(n)
}

#[test]
fn lines_in_installed_fonts_keep_their_family_when_edited() {
    let serif = font(installed("Source Serif 4"), false, false);
    let bold = font(installed("Cantarell"), true, false);
    let mut p = page();
    p.add_text(0, (20.0, 40.0), "Plain words", serif, 14.0, [0.0; 3])
        .unwrap();
    p.add_text(0, (20.0, 80.0), POLISH, bold, 14.0, [0.0; 3])
        .unwrap();
    let mut back = reopened(&mut p);
    let fonts = ["Cantarell-Bold", "SourceSerif4-Regular"];
    assert_eq!(embedded(&back), fonts);

    // Edited with other text, ASCII or not, each line stays in its family, and the fonts
    // grow rather than new ones being made.
    let edit = |text: &str| TextEdit {
        text: text.into(),
        font: None,
        size: None,
        color: None,
        parts: Vec::new(),
    };
    let id = |p: &Pdf, text: &str| {
        let objs = p.page_objects(0).unwrap();
        objs.iter()
            .find(|o| o.text.as_deref() == Some(text))
            .unwrap()
            .id
    };
    let plain = id(&back, "Plain words");
    back.edit_text(0, plain, &edit(GREEK)).unwrap();
    let polish = id(&back, POLISH);
    back.edit_text(0, polish, &edit("Other words")).unwrap();
    let mut again = reopened(&mut back);
    assert_eq!(embedded(&again), fonts);
    let text = again.page_text(0);
    assert!(
        text.contains(GREEK) && text.contains("Other words"),
        "{text:?}"
    );
    assert!(ink(&mut again, [20, 25, 200, 45]) > 50);
    assert!(ink(&mut again, [20, 65, 200, 85]) > 50);
}

#[test]
fn text_boxes_and_fill_and_sign_text_draw_in_the_embedded_face() {
    let mut p = page();
    let style = TextStyle::default();
    p.add_text_box(
        0,
        [20.0, 20.0, 380.0, 60.0],
        &format!("{POLISH}\n{GREEK}"),
        &style,
        "Sam",
    )
    .unwrap();
    p.add_sign_text(0, (200.0, 200.0), RUSSIAN, InkColor::Blue, "Sam")
        .unwrap();
    let mut back = reopened(&mut p);
    assert_eq!(embedded(&back), ["LiberationSans"], "one font for both");
    // The box grew to hold both lines, and both are drawn.
    let a = &back.annotations()[0];
    assert!(a.bbox[3] - a.bbox[1] >= 40.0, "{:?}", a.bbox);
    assert!(ink(&mut back, [20, 20, 380, 60]) > 100);
    assert!(ink(&mut back, [100, 185, 300, 210]) > 50);
    // Drawn into the page, the words read back.
    back.flatten_annotations(None).unwrap();
    let text = reopened(&mut back).page_text(0);
    for t in [POLISH, GREEK, RUSSIAN] {
        assert!(text.contains(t), "{t} in {text:?}");
    }
}

fn text(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

/// A text string in UTF-16, as PDF writes text beyond PDFDocEncoding.
fn utf16(s: &str) -> Object {
    let bytes = [0xFE, 0xFF]
        .into_iter()
        .chain(s.encode_utf16().flat_map(u16::to_be_bytes))
        .collect();
    Object::String(bytes, StringFormat::Hexadecimal)
}

/// A form with a single-line, a multi-line and a list field, `/Helv` in its resources.
fn form(p: &mut Pdf) -> Vec<ObjectId> {
    let page_id = p.page_ids()[0];
    let rect =
        |y: f32, h: f32| -> Object { vec![20.into(), y.into(), 380.into(), (y + h).into()].into() };
    let fields = [
        dictionary! { "FT" => "Tx", "T" => text("one"), "Rect" => rect(250.0, 24.0) },
        dictionary! { "FT" => "Tx", "T" => text("many"), "Ff" => 1 << 12, "Rect" => rect(150.0, 80.0) },
        dictionary! {
            "FT" => "Ch", "T" => text("pick"), "Rect" => rect(40.0, 80.0),
            "Opt" => vec![utf16("Ωmega"), text("Plain")],
        },
    ];
    let ids: Vec<ObjectId> = fields
        .into_iter()
        .map(|mut f| {
            f.set("Type", "Annot");
            f.set("Subtype", "Widget");
            f.set("P", page_id);
            p.doc.add_object(f)
        })
        .collect();
    let refs: Vec<Object> = ids.iter().map(|&i| i.into()).collect();
    p.doc
        .get_dictionary_mut(page_id)
        .unwrap()
        .set("Annots", refs.clone());
    let helv = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let acro = p.doc.add_object(dictionary! {
        "Fields" => refs,
        "DA" => text("/Helv 0 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
    });
    p.doc.catalog_mut().unwrap().set("AcroForm", acro);
    ids
}

#[test]
fn form_values_draw_in_the_embedded_face_and_keep_the_fields_font() {
    let mut p = page();
    let ids = form(&mut p);
    p.set_field("one", FieldValue::Text(POLISH.into())).unwrap();
    p.set_field("many", FieldValue::Text(format!("{GREEK}\n{RUSSIAN}")))
        .unwrap();
    p.set_field("pick", FieldValue::Choice(vec!["Ωmega".into()]))
        .unwrap();
    let mut back = reopened(&mut p);
    assert_eq!(embedded(&back), ["LiberationSans"]);
    // `/DA` still names the field's own font, which other apps draw it with.
    for &id in &ids {
        let d = back.doc.get_dictionary(id).unwrap();
        assert!(d.get(b"DA").is_err(), "no /DA written on the field");
    }
    let values: Vec<FieldValue> = back.form_fields().into_iter().map(|f| f.value).collect();
    assert_eq!(values[0], FieldValue::Text(POLISH.into()));
    assert!(
        ink(&mut back, [25, 26, 380, 50]) > 100,
        "the single line is drawn"
    );
    // Flattening no longer refuses these values, and the words read back.
    assert!(back.flatten_form(None).unwrap() > 0);
    let text = reopened(&mut back).page_text(0);
    for t in [POLISH, GREEK, RUSSIAN, "Ωmega", "Plain"] {
        assert!(text.contains(t), "{t} in {text:?}");
    }
}

#[test]
fn headers_footers_and_bates_numbers_share_one_embedded_font() {
    let mut p = page();
    p.insert_blank(1, W, H).unwrap();
    let hf = HeaderFooter {
        header: ["Страница {page}".into(), String::new(), POLISH.into()],
        footer: [String::new(), "{bates}".into(), String::new()],
        bates: Some(Bates {
            prefix: "ΑΒ-".into(),
            ..Bates::default()
        }),
        ..HeaderFooter::default()
    };
    p.add_header_footer(&[0, 1], &hf).unwrap();
    let back = reopened(&mut p);
    assert_eq!(embedded(&back), ["LiberationSans"]);
    for (i, bates) in [(0, "ΑΒ-000001"), (1, "ΑΒ-000002")] {
        let text = back.page_text(i);
        let n = format!("Страница {}", i + 1);
        for t in [n.as_str(), POLISH, bates] {
            assert!(text.contains(t), "{t} on page {i}: {text:?}");
        }
    }
    // Characters beyond Latin, Greek and Cyrillic are refused, with nothing changed.
    let mut p = page();
    let before = objects(&p);
    let cjk = HeaderFooter {
        header: ["日本".into(), String::new(), String::new()],
        ..HeaderFooter::default()
    };
    let e = p.add_header_footer(&[0], &cjk).unwrap_err().to_string();
    assert!(
        e.contains("日本") && e.contains("Latin, Greek and Cyrillic"),
        "{e}"
    );
    assert_eq!(objects(&p), before);
}

/// The document's objects, to tell whether anything changed.
fn objects(p: &Pdf) -> String {
    format!("{:?}", p.doc.objects)
}

#[test]
fn watermarks_read_back() {
    let mut p = page();
    let wm = Watermark {
        text: "ΠΡΟΣΧΕΔΙΟ".into(),
        font_size: 48.0,
        ..Watermark::default()
    };
    p.add_watermark(&[0], &wm).unwrap();
    let mut back = reopened(&mut p);
    assert_eq!(embedded(&back), ["LiberationSans"]);
    assert!(back.page_text(0).contains("ΠΡΟΣΧΕΔΙΟ"));
    assert!(ink(&mut back, [0, 0, W as usize, H as usize]) > 500);
}

#[test]
fn stamps_show_wording_and_names_beyond_western_european() {
    let mut p = page();
    let stamp = Stamp::Custom {
        text: "оплачено",
        color: StampColor::Green,
        name_and_date: true,
    };
    p.add_stamp(0, &stamp, StampPlace::Centre(200.0, 100.0), "Łukasz")
        .unwrap();
    // A name the font can't show is left off the line, and the stamp is still put down.
    p.add_stamp(0, &stamp, StampPlace::Centre(200.0, 220.0), "山田")
        .unwrap();
    let mut back = reopened(&mut p);
    assert_eq!(embedded(&back), ["LiberationSans-Bold"]);
    back.flatten_annotations(None).unwrap();
    let text = reopened(&mut back).page_text(0);
    assert_eq!(text.matches("ОПЛАЧЕНО").count(), 2, "{text:?}");
    assert!(
        text.contains("Łukasz,") && !text.contains("山田"),
        "{text:?}"
    );
}

#[test]
fn selected_text_as_a_plain_page_keeps_what_it_can_write() {
    let mut p = Pdf::from_text(&format!("{POLISH}\n{GREEK} 日本"), (W, H)).unwrap();
    let back = reopened(&mut p);
    assert_eq!(embedded(&back), ["LiberationSans"]);
    let text = back.page_text(0);
    assert!(text.contains(POLISH), "{text:?}");
    assert!(text.contains(&format!("{GREEK} ??")), "{text:?}");
}

/// Runs `name` with `args` (`{}` the file holding `bytes`): its output and messages.
fn tool(name: &str, args: &[&str], bytes: &[u8]) -> (String, String) {
    // A folder of its own for each run, as the cross-checks run side by side.
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("limarepdf-unicode-{}-{run}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.pdf"));
    std::fs::write(&path, bytes).unwrap();
    let args: Vec<String> = args
        .iter()
        .map(|a| a.replace("{}", path.to_str().unwrap()))
        .collect();
    let out = std::process::Command::new(name)
        .args(&args)
        .output()
        .unwrap_or_else(|e| panic!("{name} installed: {e}"));
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(out.status.success(), "{name} failed");
    let s = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    (s(&out.stdout), s(&out.stderr))
}

/// Every writer's text in one file, read by poppler (`pdftotext`, `pdffonts`) and
/// Ghostscript, which draws it without a message. Opt-in: `cargo test -p limarepdf-core
/// --test unicode_text -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_every_writer_with_poppler_and_ghostscript() {
    let mut p = page();
    form(&mut p);
    let serif = font(Family::Times, false, true);
    p.add_text(0, (20.0, 30.0), POLISH, serif, 12.0, [0.0; 3])
        .unwrap();
    let style = TextStyle::default();
    p.add_text_box(0, [20.0, 280.0, 380.0, 296.0], VIETNAMESE, &style, "Sam")
        .unwrap();
    p.set_field("one", FieldValue::Text(GREEK.into())).unwrap();
    p.set_field("many", FieldValue::Text(RUSSIAN.into()))
        .unwrap();
    let stamp = Stamp::Custom {
        text: "оплачено",
        color: StampColor::Blue,
        name_and_date: false,
    };
    p.add_stamp(0, &stamp, StampPlace::Centre(300.0, 130.0), "")
        .unwrap();
    let hf = HeaderFooter {
        footer: [String::new(), "Страница {page}".into(), String::new()],
        ..HeaderFooter::default()
    };
    p.add_header_footer(&[0], &hf).unwrap();
    let wm = Watermark {
        text: "ΠΡΟΣΧΕΔΙΟ".into(),
        font_size: 30.0,
        ..Watermark::default()
    };
    p.add_watermark(&[0], &wm).unwrap();
    p.flatten_form(None).unwrap();
    p.flatten_annotations(None).unwrap();
    let bytes = p.to_bytes().unwrap();
    let words = [
        POLISH,
        VIETNAMESE,
        GREEK,
        RUSSIAN,
        "ОПЛАЧЕНО",
        "Страница 1",
        "ΠΡΟΣΧΕΔΙΟ",
        "Ωmega",
    ];
    let (text, _) = tool("pdftotext", &["-raw", "{}", "-"], &bytes);
    let gs = [
        "-q",
        "-dNOPAUSE",
        "-dBATCH",
        "-dSAFER",
        "-sDEVICE=txtwrite",
        "-sOutputFile=-",
        "{}",
    ];
    let (gs_text, gs_errors) = tool("gs", &gs, &bytes);
    // Both space and break the turned watermark as drawn: compare without spaces.
    let bare = |s: &str| -> String { s.split_whitespace().collect() };
    let (text, gs_text) = (bare(&text), bare(&gs_text));
    for w in words.map(bare) {
        assert!(text.contains(&w), "poppler: {w:?} in {text:?}");
        assert!(gs_text.contains(&w), "Ghostscript: {w:?} in {gs_text:?}");
    }
    assert!(gs_errors.trim().is_empty(), "Ghostscript: {gs_errors}");
    let (fonts, errors) = tool("pdffonts", &["{}"], &bytes);
    assert!(errors.trim().is_empty(), "pdffonts: {errors}");
    let embedded: Vec<&str> = fonts
        .lines()
        .skip(2)
        .filter(|r| r.contains("CID"))
        .collect();
    // Sans (text box, form, header, watermark), Sans Bold (stamp), Serif Italic.
    assert_eq!(embedded.len(), 3, "{fonts}");
    for row in embedded {
        assert!(
            row.contains("CID TrueType") && row.contains("Identity-H"),
            "{row}"
        );
        let flags: Vec<&str> = row.split_whitespace().rev().skip(2).take(3).collect();
        assert_eq!(flags, ["yes", "yes", "yes"], "{row}");
    }
    let draw = [
        "-q",
        "-dNOPAUSE",
        "-dBATCH",
        "-dSAFER",
        "-sDEVICE=nullpage",
        "{}",
    ];
    let (out, errors) = tool("gs", &draw, &bytes);
    assert!(
        (out + &errors).trim().is_empty(),
        "Ghostscript drawing: {errors}"
    );
}

#[test]
fn characters_it_cant_write_are_refused_with_nothing_changed() {
    let mut p = page();
    let before = objects(&p);
    let helvetica = FontChoice::default();
    // Right-to-left scripts aren't written yet, though the face has Hebrew.
    for t in ["日本", "שלום", "Zażółć 日"] {
        let e = p
            .add_text(0, (20.0, 40.0), t, helvetica, 12.0, [0.0; 3])
            .unwrap_err()
            .to_string();
        assert!(e.contains("Latin, Greek and Cyrillic"), "{t}: {e}");
    }
    let wm = Watermark {
        text: "草稿".into(),
        ..Watermark::default()
    };
    assert!(p.add_watermark(&[0], &wm).is_err());
    assert_eq!(objects(&p), before);
}

// ---------------------------------------------------------------- emoji (BUG-24)

/// The made-up font with emoji (`tools/make_emoji_test_font.py`): the Latin letters, 😀
/// with an outline, 😎 in colour layers only, and a glyph for each part of [`SEQUENCES`].
const EMOJI: &str = "Limare Emoji Test";

/// Emoji made of several characters: a skin tone, a family, a flag, a keycap and the emoji
/// variation selector.
const SEQUENCES: [&str; 5] = [
    "👍🏽",
    "👨\u{200D}👩\u{200D}👧",
    "🇬🇧",
    "1\u{FE0F}\u{20E3}",
    "❤\u{FE0F}",
];

/// The refusal of an emoji sequence.
fn sequence_refused(s: &str) -> String {
    format!(
        "LimarePDF can't write \"{s}\" yet: flags, skin tones, keycaps and joined emoji aren't \
         supported"
    )
}

#[test]
fn new_and_edited_lines_write_single_emoji_in_a_font_that_has_them() {
    let emoji = font(installed(EMOJI), false, false);
    let mut p = page();
    p.add_text(0, (20.0, 40.0), "Walrus 😀", emoji, 14.0, [0.0; 3])
        .unwrap();
    p.add_text(0, (20.0, 80.0), "Seal", emoji, 14.0, [0.0; 3])
        .unwrap();
    let mut back = reopened(&mut p);
    let seal = back.page_objects(0).unwrap();
    let seal = seal.iter().find(|o| o.text.as_deref() == Some("Seal"));
    let edit = |text: &str| TextEdit {
        text: text.into(),
        font: None,
        size: None,
        color: None,
        parts: Vec::new(),
    };
    back.edit_text(0, seal.unwrap().id, &edit("Seal 😀"))
        .unwrap();
    let mut again = reopened(&mut back);
    assert_eq!(embedded(&again), ["LimareEmojiTest-Regular"]);
    let text = again.page_text(0);
    assert!(
        text.contains("Walrus 😀") && text.contains("Seal 😀"),
        "{text:?}"
    );
    // The emoji after "Seal" is drawn: a box about 13 points square.
    assert!(ink(&mut again, [60, 65, 90, 85]) > 100);

    // Drawn only in colour, or a sequence: refused, with nothing changed.
    let before = objects(&again);
    let e = again
        .add_text(0, (20.0, 120.0), "Cool 😎", emoji, 14.0, [0.0; 3])
        .unwrap_err()
        .to_string();
    assert_eq!(e, "Limare Emoji Test has no \"😎\"; choose another font");
    let walrus = again.page_objects(0).unwrap();
    let walrus = walrus
        .iter()
        .find(|o| o.text.as_deref() == Some("Walrus 😀"))
        .unwrap()
        .id;
    for s in SEQUENCES {
        let text = format!("Seal {s}");
        let e = again
            .add_text(0, (20.0, 120.0), &text, emoji, 14.0, [0.0; 3])
            .unwrap_err();
        assert_eq!(e.to_string(), sequence_refused(s));
        let e = again.edit_text(0, walrus, &edit(s)).unwrap_err();
        assert_eq!(e.to_string(), sequence_refused(s));
    }
    assert_eq!(objects(&again), before);
}

#[test]
fn text_boxes_and_fill_and_sign_text_refuse_what_they_cant_write() {
    // Set in Helvetica, or its embedded face, which has no emoji or Chinese.
    let mut p = page();
    let style = TextStyle::default();
    let before = objects(&p);
    for t in ["Walrus 😀", "Seal 中", "Seal 👍🏽"] {
        let e = p
            .add_text_box(0, [20.0, 20.0, 380.0, 60.0], t, &style, "Sam")
            .unwrap_err()
            .to_string();
        assert!(e.starts_with("LimarePDF can't write"), "{t}: {e}");
        let e = p
            .add_sign_text(0, (200.0, 200.0), t, InkColor::Blue, "Sam")
            .unwrap_err()
            .to_string();
        assert!(e.starts_with("LimarePDF can't write"), "{t}: {e}");
    }
    let e = p
        .add_text_box(0, [20.0, 20.0, 380.0, 60.0], "Walrus 😀", &style, "")
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "LimarePDF can't write \"😀\" yet; Latin, Greek and Cyrillic characters are supported"
    );
    assert_eq!(objects(&p), before);

    // A text box's text changed to what it can't write is refused, and it keeps its text
    // and drawing.
    let id = p
        .add_text_box(0, [20.0, 20.0, 380.0, 60.0], POLISH, &style, "Sam")
        .unwrap();
    let before = objects(&p);
    let a = p.annotations()[0].clone();
    for t in ["Walrus 😀", "Seal 👍🏽"] {
        assert!(p.set_annotation(id, a.color, a.opacity, t).is_err(), "{t}");
    }
    assert_eq!(objects(&p), before);
    assert_eq!(p.annotations()[0].contents, POLISH);
}

#[test]
fn writers_in_the_standard_fonts_refuse_emoji_with_nothing_changed() {
    let mut p = page();
    form(&mut p);
    let before = objects(&p);
    for t in ["Walrus 😀", "Seal 👍🏽"] {
        let field = p.set_field("one", FieldValue::Text(t.into()));
        assert!(field.is_err(), "{t}");
        let hf = HeaderFooter {
            header: [t.into(), String::new(), String::new()],
            ..HeaderFooter::default()
        };
        assert!(p.add_header_footer(&[0], &hf).is_err(), "{t}");
        let bates = HeaderFooter {
            footer: [String::new(), "{bates}".into(), String::new()],
            bates: Some(Bates {
                prefix: t.into(),
                ..Bates::default()
            }),
            ..HeaderFooter::default()
        };
        assert!(p.add_header_footer(&[0], &bates).is_err(), "{t}");
        let wm = Watermark {
            text: t.into(),
            ..Watermark::default()
        };
        assert!(p.add_watermark(&[0], &wm).is_err(), "{t}");
        let stamp = Stamp::Custom {
            text: t,
            color: StampColor::Green,
            name_and_date: false,
        };
        let e = p
            .add_stamp(0, &stamp, StampPlace::Centre(200.0, 100.0), "")
            .unwrap_err();
        assert!(e.to_string().starts_with("a stamp can't show"), "{e}");
        for style in SignStyle::ALL {
            assert!(typed_signature(t, style).is_err(), "{t} {style:?}");
        }
    }
    assert_eq!(objects(&p), before);
    // A name the stamp can't show is left off its line, as for other scripts.
    let stamp = Stamp::Custom {
        text: "paid",
        color: StampColor::Green,
        name_and_date: true,
    };
    p.add_stamp(0, &stamp, StampPlace::Centre(200.0, 100.0), "Sam 😀")
        .unwrap();
    p.flatten_annotations(None).unwrap();
    let text = reopened(&mut p).page_text(0);
    assert!(text.contains("PAID") && !text.contains("Sam"), "{text:?}");
}

/// The number of dark pixels on the first page of the PDF `bytes` as Poppler draws it, at
/// 2 pixels per point.
fn poppler_ink(bytes: &[u8]) -> usize {
    let dir = std::env::temp_dir().join(format!("limarepdf-emoji-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("in.pdf");
    std::fs::write(&pdf, bytes).unwrap();
    let out = dir.join("out");
    let run = std::process::Command::new("pdftoppm")
        .args(["-r", "144", "-gray", "-singlefile"])
        .arg(&pdf)
        .arg(&out)
        .status()
        .expect("pdftoppm installed");
    assert!(run.success(), "pdftoppm failed");
    let pgm = std::fs::read(dir.join("out.pgm")).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    // A binary PGM: "P5", width, height and the largest value, then a byte a pixel.
    let mut fields = 0;
    let mut at = 0;
    while fields < 4 {
        while pgm[at].is_ascii_whitespace() {
            at += 1;
        }
        while !pgm[at].is_ascii_whitespace() {
            at += 1;
        }
        fields += 1;
    }
    pgm[at + 1..].iter().filter(|&&v| v < 128).count()
}

/// Single emoji in fonts that draw them with outlines, as Poppler reads and draws them: the
/// made-up font, and Symbola and DejaVu Sans where installed (each has the Latin letters and
/// outline emoji). A colour emoji font isn't offered. Opt-in: `cargo test -p limarepdf-core
/// --test unicode_text -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_emoji_with_poppler() {
    let mut fonts = vec![(EMOJI, installed(EMOJI), "😀")];
    for (name, emoji) in [("Symbola", "🐋 😀 🦊"), ("DejaVu Sans", "😀 🐱 🌕")] {
        let found = limarepdf_core::typefaces()
            .into_iter()
            .find(|(_, t)| t.name == name);
        match found {
            Some((n, _)) => fonts.push((name, Family::Installed(n), emoji)),
            None => eprintln!("{name} isn't installed: not checked"),
        }
    }
    for (name, family, emoji) in fonts {
        let f = font(family, false, false);
        let line = format!("Walrus {emoji}");
        let mut p = page();
        p.add_text(0, (20.0, 40.0), &line, f, 24.0, [0.0; 3])
            .unwrap();
        let mut p = reopened(&mut p);
        assert!(p.page_text(0).contains(&line), "{name}: LimarePDF");
        let bytes = p.to_bytes().unwrap();
        // Through the text map, characters beyond the first plane included.
        let (text, _) = tool("pdftotext", &["{}", "-"], &bytes);
        assert!(text.contains(&line), "{name}: poppler read {text:?}");
        let (fonts, errors) = tool("pdffonts", &["{}"], &bytes);
        assert!(errors.trim().is_empty(), "{name}: pdffonts: {errors}");
        let row = fonts.lines().nth(2).unwrap();
        let flags: Vec<&str> = row.split_whitespace().rev().skip(2).take(3).collect();
        assert_eq!(flags, ["yes", "yes", "yes"], "{name}: {row}");
        // Poppler and LimarePDF draw the emoji: more ink than the word alone.
        let mut words = page();
        words
            .add_text(0, (20.0, 40.0), "Walrus", f, 24.0, [0.0; 3])
            .unwrap();
        let (with, without) = (poppler_ink(&bytes), poppler_ink(&words.to_bytes().unwrap()));
        assert!(with > without + 200, "{name}: poppler {with} {without}");
        let whole = [0, 0, W as usize, H as usize];
        let (with, without) = (ink(&mut p, whole), ink(&mut words, whole));
        assert!(with > without + 50, "{name}: LimarePDF {with} {without}");
        eprintln!("{name}: {emoji} written, read and drawn");
    }
    // Noto Color Emoji draws in colour pictures only, with no outlines or Latin letters.
    let noto = std::path::Path::new("/usr/share/fonts/noto/NotoColorEmoji.ttf");
    if noto.exists() {
        assert!(limarepdf_core::add_font_files(&[noto]).is_empty());
    } else {
        eprintln!("Noto Color Emoji isn't installed: not checked");
    }
}
