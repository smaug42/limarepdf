//! Lines of text in more than one style: a line reads as one object whatever its fonts,
//! sizes and colours, and is written back in parts, each in its style.

use limarepdf_core::{
    render, words_text, FontChoice, PageObject, PartFont, Pdf, TextEdit, TextPart,
};
use lopdf::{dictionary, Dictionary, Object, Stream};

const RED: [f32; 3] = [1.0, 0.0, 0.0];
const BLACK: [f32; 3] = [0.0; 3];

/// A 300 × 200 page drawing `content`, with the fonts F1 (Helvetica), F2 (Helvetica Bold)
/// and F3 (a subset of Helvetica Oblique, which new text can't be written in), F4 (another
/// font object for Helvetica) and F5 (another subset of Helvetica Oblique).
fn make(content: &str) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let mut font = |base: &str| {
        p.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base,
            "Encoding" => "WinAnsiEncoding",
        })
    };
    let fonts = dictionary! {
        "F1" => font("Helvetica"),
        "F2" => font("Helvetica-Bold"),
        "F3" => font("ABCDEF+Helvetica-Oblique"),
        "F4" => font("Helvetica"),
        "F5" => font("GHIJKL+Helvetica-Oblique"),
    };
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", Object::Reference(s));
    d.set("Resources", dictionary! { "Font" => fonts });
    p
}

fn texts(p: &Pdf) -> Vec<String> {
    p.page_objects(0)
        .unwrap()
        .into_iter()
        .filter_map(|o| o.text)
        .collect()
}

fn line(p: &Pdf, text: &str) -> PageObject {
    p.page_objects(0)
        .unwrap()
        .into_iter()
        .find(|o| o.text.as_deref() == Some(text))
        .unwrap_or_else(|| panic!("no line {text:?} in {:?}", texts(p)))
}

/// Each span's font, size, fill, rise and text.
fn styles(o: &PageObject) -> Vec<(String, f32, [f32; 3], f32, String)> {
    let chars: Vec<char> = o.text.as_deref().unwrap().chars().collect();
    o.spans
        .iter()
        .map(|s| {
            (
                s.font_name.clone(),
                s.font_size,
                s.fill.unwrap(),
                s.rise,
                chars[s.chars.0..s.chars.1].iter().collect(),
            )
        })
        .collect()
}

fn style(
    font: &str,
    size: f32,
    fill: [f32; 3],
    rise: f32,
    text: &str,
) -> (String, f32, [f32; 3], f32, String) {
    (font.into(), size, fill, rise, text.into())
}

/// Part `i` of line `o`, with `text`, in the span's own style.
fn own(o: &PageObject, i: usize, text: &str) -> TextPart {
    let s = &o.spans[i];
    TextPart {
        text: text.into(),
        font: PartFont::Span(i),
        size: s.font_size,
        color: s.fill.unwrap_or(BLACK),
        rise: s.rise,
    }
}

/// The parts of line `o` as they are.
fn as_they_are(o: &PageObject) -> Vec<TextPart> {
    let chars: Vec<char> = o.text.as_deref().unwrap().chars().collect();
    (0..o.spans.len())
        .map(|i| {
            let (a, b) = o.spans[i].chars;
            own(o, i, &chars[a..b].iter().collect::<String>())
        })
        .collect()
}

fn content(p: &Pdf) -> String {
    let page = p.page_ids()[0];
    String::from_utf8_lossy(&p.doc.get_page_content(page)).into_owned()
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.05)
}

const MIXED: &str = "BT /F1 20 Tf 20 100 Td (A ) Tj /F2 20 Tf (bold) Tj /F1 20 Tf ( word) Tj ET\n";

#[test]
fn a_line_with_a_bold_word_is_one_object_in_three_parts() {
    let p = make(MIXED);
    let o = p.page_objects(0).unwrap();
    assert_eq!(o.len(), 1, "{o:?}");
    assert_eq!(o[0].text.as_deref(), Some("A bold word"));
    assert_eq!(
        styles(&o[0]),
        [
            style("Helvetica", 20.0, BLACK, 0.0, "A "),
            style("Helvetica-Bold", 20.0, BLACK, 0.0, "bold"),
            style("Helvetica", 20.0, BLACK, 0.0, " word"),
        ]
    );
    // The line's own font, size and colour are its first part's.
    assert_eq!(o[0].font_name.as_deref(), Some("Helvetica"));
    assert_eq!(o[0].font_size, Some(20.0));

    // Sizes and colours don't split a line either.
    let p = make("BT /F1 20 Tf 20 100 Td (Big ) Tj /F1 10 Tf 1 0 0 rg (small) Tj ET");
    let o = line(&p, "Big small");
    assert_eq!(
        styles(&o),
        [
            style("Helvetica", 20.0, BLACK, 0.0, "Big "),
            style("Helvetica", 10.0, RED, 0.0, "small"),
        ]
    );

    // A line in one style is one part, however many shows draw it.
    let p = make("BT /F1 20 Tf 20 100 Td (Hello) Tj ( world) Tj ET");
    let o = line(&p, "Hello world");
    assert_eq!(
        styles(&o),
        [style("Helvetica", 20.0, BLACK, 0.0, "Hello world")]
    );

    // Text further along than a word gap, or on another baseline, is another line.
    let p = make("BT /F1 20 Tf 20 100 Td (Left) Tj /F2 20 Tf [-3000 (Right)] TJ ET");
    assert_eq!(texts(&p), ["Left", "Right"]);
    let p = make("BT /F1 20 Tf 20 100 Td (One) Tj /F2 20 Tf 0 -30 Td (Two) Tj ET");
    assert_eq!(texts(&p), ["One", "Two"]);
}

#[test]
fn parts_in_two_font_objects_of_one_font_look_alike() {
    let p = make(
        "BT /F1 20 Tf 20 100 Td (One ) Tj /F4 20 Tf (font) Tj /F3 20 Tf ( two) Tj \
         /F5 20 Tf ( subsets) Tj ET",
    );
    let s = line(&p, "One font two subsets").spans;
    assert_eq!(s.len(), 4);
    assert!(s[0].looks_like(&s[1]) && s[2].looks_like(&s[3]));
    assert!(!s[1].looks_like(&s[2]));
    // Size, colour and rise tell parts apart.
    let p =
        make("BT /F1 20 Tf 20 100 Td (a) Tj /F1 10 Tf (b) Tj 1 0 0 rg (c) Tj 0 g 2 Ts (d) Tj ET");
    let s = line(&p, "abcd").spans;
    assert_eq!(s.len(), 4);
    assert!((1..4).all(|i| !s[i].looks_like(&s[i - 1])));
}

#[test]
fn raised_and_lowered_parts_keep_their_place() {
    // A superscript within the line's height joins it, raised.
    let p = make(
        "BT /F1 20 Tf 20 100 Td (E = mc) Tj /F1 12 Tf 5 Ts (2) Tj 0 Ts /F1 20 Tf ( holds) Tj ET",
    );
    let o = line(&p, "E = mc2 holds");
    assert_eq!(
        styles(&o),
        [
            style("Helvetica", 20.0, BLACK, 0.0, "E = mc"),
            style("Helvetica", 12.0, BLACK, 5.0, "2"),
            style("Helvetica", 20.0, BLACK, 0.0, " holds"),
        ]
    );
    // User (20, 100) on a 200 high page is (20, 100) with y down.
    let b = o.baseline.unwrap();
    assert!(
        (b.0 - 20.0).abs() < 0.01 && (b.1 - 100.0).abs() < 0.01,
        "{b:?}"
    );

    // A line that starts raised: the baseline is its larger text's, and the first part
    // is raised from it.
    let p = make("BT /F1 12 Tf 5 Ts 20 100 Td (1) Tj 0 Ts /F1 20 Tf (Note) Tj ET");
    let o = line(&p, "1Note");
    assert_eq!(
        styles(&o),
        [
            style("Helvetica", 12.0, BLACK, 5.0, "1"),
            style("Helvetica", 20.0, BLACK, 0.0, "Note"),
        ]
    );
    let b = o.baseline.unwrap();
    assert!(
        (b.0 - 20.0).abs() < 0.01 && (b.1 - 100.0).abs() < 0.01,
        "{b:?}"
    );

    // Written back as it is, each part is where it was.
    let mut q = make("BT /F1 12 Tf 5 Ts 20 100 Td (1) Tj 0 Ts /F1 20 Tf (Note) Tj ET");
    let before = (
        q.find_text_boxes(0, "1").unwrap(),
        q.find_text_boxes(0, "note").unwrap(),
    );
    let mut parts = as_they_are(&o);
    parts[1].color = RED;
    q.edit_text(0, o.id, &TextEdit::in_parts(parts)).unwrap();
    let o = line(&q, "1Note");
    assert_eq!(styles(&o)[0], style("Helvetica", 12.0, BLACK, 5.0, "1"));
    assert_eq!(styles(&o)[1], style("Helvetica", 20.0, RED, 0.0, "Note"));
    let after = (
        q.find_text_boxes(0, "1").unwrap(),
        q.find_text_boxes(0, "note").unwrap(),
    );
    assert!(
        near(before.0[0], after.0[0]) && near(before.1[0], after.1[0]),
        "{before:?} {after:?}"
    );
}

#[test]
fn editing_one_part_keeps_the_others_as_they_were() {
    // The middle word is in a font new text can't be written in, with its own spacing.
    let src =
        "BT /F1 20 Tf 20 100 Td (A ) Tj /F3 20 Tf 1 Tc (slanted) Tj 0 Tc /F1 20 Tf ( word) Tj ET";
    let mut p = make(src);
    let o = line(&p, "A slanted word");
    let width = |p: &Pdf| {
        let b = p.find_text_boxes(0, "slanted").unwrap()[0];
        b[2] - b[0]
    };
    let was = width(&p);
    let parts = vec![
        own(&o, 0, "An "),
        own(&o, 1, "slanted"),
        own(&o, 2, " word"),
    ];
    assert!(parts.iter().all(|t| o.keeps_font(t)));
    p.edit_text(0, o.id, &TextEdit::in_parts(parts)).unwrap();
    let o = line(&p, "An slanted word");
    assert_eq!(
        styles(&o),
        [
            style("Helvetica", 20.0, BLACK, 0.0, "An "),
            style("ABCDEF+Helvetica-Oblique", 20.0, BLACK, 0.0, "slanted"),
            style("Helvetica", 20.0, BLACK, 0.0, " word"),
        ]
    );
    // The slanted word is shown as it was, in its own font, with its spacing.
    let c = content(&p);
    assert!(c.contains("/F3 20 Tf"), "{c}");
    assert!(c.contains("[(slanted)] TJ"), "{c}");
    assert!((width(&p) - was).abs() < 0.05, "{} vs {was}", width(&p));

    // Changed, it can't keep that font: the nearest standard one.
    let parts = vec![own(&o, 0, "An "), own(&o, 1, "italic"), own(&o, 2, " word")];
    assert!(!o.keeps_font(&parts[1]));
    p.edit_text(0, o.id, &TextEdit::in_parts(parts)).unwrap();
    let o = line(&p, "An italic word");
    assert_eq!(
        styles(&o)[1],
        style("Helvetica-Oblique", 20.0, BLACK, 0.0, "italic")
    );
    // Plain ASCII in a font that can be kept keeps it; other text can't.
    assert!(o.keeps_font(&own(&o, 0, "Some ")));
    assert!(!o.keeps_font(&own(&o, 0, "Café ")));
    let chosen = TextPart {
        font: PartFont::Choice(FontChoice::from_name("Courier")),
        ..own(&o, 0, "An ")
    };
    assert!(!o.keeps_font(&chosen));
}

#[test]
fn a_gap_read_as_a_space_is_kept() {
    // "World" is placed a gap after "Hello", which reads as a space.
    let mut p = make("BT /F1 20 Tf 20 100 Td (Hello) Tj /F2 20 Tf 58 0 Td (World) Tj ET");
    let o = line(&p, "Hello World");
    let before = p.find_text_boxes(0, "world").unwrap()[0];
    let mut parts = as_they_are(&o);
    parts[0].color = RED;
    p.edit_text(0, o.id, &TextEdit::in_parts(parts)).unwrap();
    let o = line(&p, "Hello World");
    assert_eq!(styles(&o)[0], style("Helvetica", 20.0, RED, 0.0, "Hello "));
    let after = p.find_text_boxes(0, "world").unwrap()[0];
    assert!(near(before, after), "{before:?} then {after:?}");
}

/// BUG-25: spaces at a line's ends, in its own font or in a show of their own in another,
/// don't widen its box over what is next to it (PDFBox 4320: a label's tab in Helvetica
/// Bold covered the next label).
#[test]
fn spaces_at_a_lines_ends_are_not_in_its_box() {
    let p = make(
        "BT /F1 6 Tf 20 100 Td (Anrede) Tj /F2 25 Tf (   ) Tj ET \
         BT /F1 6 Tf 50 100 Td (Titel) Tj ET \
         BT /F1 20 Tf 20 150 Td (Name ) Tj ET \
         BT /F1 20 Tf 20 50 Td (  lead) Tj ET",
    );
    let word = |w: &str| p.find_text_boxes(0, w).unwrap()[0];
    for (text, w) in [
        ("Anrede   ", "anrede"),
        ("Name ", "name"),
        ("  lead", "lead"),
    ] {
        let b = line(&p, text).bbox;
        assert!(near(b, word(w)), "{text:?}: {b:?}, its word {:?}", word(w));
    }
    assert!(line(&p, "Anrede   ").bbox[2] < line(&p, "Titel").bbox[0]);
}

/// BUG-26: a line says which way it runs on the displayed page and where its characters
/// are along it, from its baseline's start, on a turned page too.
#[test]
fn a_line_says_which_way_it_runs_and_where_its_characters_are() {
    let mut p = make(MIXED);
    p.rotate_pages(&[0], 90).unwrap();
    let o = line(&p, "A bold word");
    let close = |a: f32, b: f32| (a - b).abs() < 0.05;
    let dir = o.dir.unwrap();
    assert!(close(dir.0, 0.0) && close(dir.1, 1.0), "{dir:?}");
    let stops = &o.stops;
    assert_eq!(stops.len(), "A bold word".len() + 1);
    assert!(close(stops[0], 0.0), "{stops:?}");
    assert!(stops.windows(2).all(|w| w[1] > w[0]), "{stops:?}");
    // "bold" starts and ends where the search finds it.
    let bold = p.find_text_boxes(0, "bold").unwrap()[0];
    let (_, y) = o.baseline.unwrap();
    let (from, to) = (stops[2] + y, stops[6] + y);
    assert!(
        close(from, bold[1]) && close(to, bold[3]),
        "{stops:?} {bold:?}"
    );
}

#[test]
fn a_restyled_word_reads_back_as_part_of_its_line() {
    let mut p = make(MIXED);
    let o = line(&p, "A bold word");
    let courier = FontChoice::from_name("Courier-Bold");
    let parts = vec![
        own(&o, 0, "A "),
        TextPart {
            text: "bold".into(),
            font: PartFont::Choice(courier),
            size: 24.0,
            color: RED,
            rise: 0.0,
        },
        own(&o, 2, " word"),
    ];
    p.edit_text(0, o.id, &TextEdit::in_parts(parts)).unwrap();
    let want = [
        style("Helvetica", 20.0, BLACK, 0.0, "A "),
        style("Courier-Bold", 24.0, RED, 0.0, "bold"),
        style("Helvetica", 20.0, BLACK, 0.0, " word"),
    ];
    assert_eq!(texts(&p), ["A bold word"]);
    assert_eq!(styles(&line(&p, "A bold word")), want);
    // And after saving and opening again.
    let q = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert_eq!(styles(&line(&q, "A bold word")), want);

    // A raised part is written raised.
    let o = line(&p, "A bold word");
    let mut parts = as_they_are(&o);
    parts[2] = TextPart {
        size: 12.0,
        rise: 5.0,
        ..own(&o, 2, " word")
    };
    p.edit_text(0, o.id, &TextEdit::in_parts(parts)).unwrap();
    assert_eq!(
        styles(&line(&p, "A bold word"))[2],
        style("Helvetica", 12.0, BLACK, 5.0, " word")
    );
}

#[test]
fn a_line_in_one_style_changed_in_words_only_is_changed_in_place() {
    // The same change in one style or as one part writes the same page, the line changed
    // where it is: no new text object.
    let one = TextEdit {
        text: "Howdy".into(),
        font: None,
        size: None,
        color: None,
        parts: Vec::new(),
    };
    let mut a = make("BT /F1 20 Tf 20 100 Td (Hello) Tj ET");
    let mut b = make("BT /F1 20 Tf 20 100 Td (Hello) Tj ET");
    a.edit_text(0, 0, &one).unwrap();
    let o = line(&b, "Hello");
    b.edit_text(0, 0, &TextEdit::in_parts(vec![own(&o, 0, "Howdy")]))
        .unwrap();
    assert_eq!(content(&a), content(&b));
    assert_eq!(content(&b).matches("BT").count(), 1, "{}", content(&b));

    // Restyled whole, it keeps its font where it can, as parts do: written as it was in
    // red, where in one style it is written anew in the nearest standard font.
    let mut p = make("BT /F1 20 Tf 3 Tc 20 100 Td (Hello) Tj ET");
    let o = line(&p, "Hello");
    let was = p.find_text_boxes(0, "hello").unwrap()[0];
    let parts = vec![TextPart {
        color: RED,
        ..own(&o, 0, "Hello")
    }];
    p.edit_text(0, 0, &TextEdit::in_parts(parts)).unwrap();
    let c = content(&p);
    assert!(c.contains("/F1 20 Tf") && c.contains("[(Hello)] TJ"), "{c}");
    assert_eq!(
        styles(&line(&p, "Hello")),
        [style("Helvetica", 20.0, RED, 0.0, "Hello")]
    );
    // Its character spacing kept too.
    assert!(near(p.find_text_boxes(0, "hello").unwrap()[0], was));
}

#[test]
fn characters_a_part_cant_write_are_refused() {
    let mut p = make(MIXED);
    let o = line(&p, "A bold word");
    let before = content(&p);
    let parts = vec![
        own(&o, 0, "A "),
        TextPart {
            font: PartFont::Choice(FontChoice::from_name("Courier")),
            ..own(&o, 1, "שלום")
        },
        own(&o, 2, " word"),
    ];
    let e = p
        .edit_text(0, o.id, &TextEdit::in_parts(parts))
        .unwrap_err();
    assert!(e.to_string().contains("can't"), "{e}");
    // Greek in the bold part's font is written in its embedded face.
    let mut p = make(MIXED);
    let parts = vec![own(&o, 0, "A "), own(&o, 1, "Ωmega"), own(&o, 2, " word")];
    p.edit_text(0, o.id, &TextEdit::in_parts(parts)).unwrap();
    assert_eq!(texts(&p), ["A Ωmega word"]);
    assert_ne!(content(&p), before);
}

#[test]
fn parts_that_dont_fit_the_line_are_errors() {
    let mut p = make(MIXED);
    let o = line(&p, "A bold word");
    let mut e = TextEdit::in_parts(as_they_are(&o));
    e.text.push('!');
    assert!(p.edit_text(0, o.id, &e).is_err());
    let e = TextEdit::in_parts(vec![
        own(&o, 0, "A "),
        TextPart {
            font: PartFont::Span(9),
            ..own(&o, 1, "x")
        },
    ]);
    assert!(p.edit_text(0, o.id, &e).is_err());
    assert_eq!(texts(&p), ["A bold word"]);
    // New text has no part of its own to take a font from.
    assert!(p
        .add_text_parts(0, (20.0, 50.0), &[own(&o, 0, "New")])
        .is_err());
}

#[test]
fn text_after_a_line_in_parts_stays_as_it_was() {
    // "Far" carries on from where the line ends, in the bold font and red; "Next" is a line
    // below, in red too.
    let src = "BT /F1 20 Tf 20 100 Td (A ) Tj /F2 20 Tf (bold) Tj 1 0 0 rg [-3000 (Far)] TJ \
               0 -30 Td (Next) Tj ET\n";
    type Change = fn(&mut Pdf, &PageObject);
    let cases: [(&str, Change); 4] = [
        ("edited", |p, o| {
            let mut parts = as_they_are(o);
            parts[1].text = "bolder".into();
            p.edit_text(0, o.id, &TextEdit::in_parts(parts)).unwrap();
        }),
        ("restyled", |p, o| {
            let mut parts = as_they_are(o);
            parts[0].font = PartFont::Choice(FontChoice::from_name("Times-Roman"));
            p.edit_text(0, o.id, &TextEdit::in_parts(parts)).unwrap();
        }),
        ("moved", |p, o| {
            let b = o.bbox;
            p.set_object_bbox(0, o.id, [b[0], b[1] - 40.0, b[2], b[3] - 40.0])
                .unwrap();
        }),
        ("deleted", |p, o| p.delete_object(0, o.id).unwrap()),
    ];
    for (what, change) in cases {
        let mut p = make(src);
        let o = line(&p, "A bold");
        let kept = [line(&p, "Far"), line(&p, "Next")];
        change(&mut p, &o);
        for k in kept {
            let now = line(&p, k.text.as_deref().unwrap());
            assert!(
                near(now.bbox, k.bbox),
                "{what}: {:?} moved: {:?} to {:?}",
                k.text,
                k.bbox,
                now.bbox
            );
            assert_eq!(
                (now.font_name.as_deref(), now.fill),
                (Some("Helvetica-Bold"), Some(RED)),
                "{what}: {:?}",
                k.text
            );
        }
    }
}

#[test]
fn a_line_in_parts_is_searched_selected_moved_and_redacted_as_one() {
    let mut p = make(MIXED);
    let words: Vec<String> = p
        .page_words(0)
        .unwrap()
        .into_iter()
        .map(|w| w.text)
        .collect();
    assert_eq!(words, ["A", "bold", "word"]);
    assert_eq!(words_text(&p.page_words(0).unwrap()), "A bold word");
    // "bold" after "A ": 20 + (667 + 278) / 50 = 38.9, then 611 + 611 + 278 + 611 more.
    let b = p.find_text_boxes(0, "bold").unwrap();
    assert_eq!(b.len(), 1);
    assert!(
        (b[0][0] - 38.9).abs() < 0.2 && (b[0][2] - 81.12).abs() < 0.2,
        "{b:?}"
    );
    // One match, with a box for each part.
    let m = p.find_text_matches(0, "a bold word").unwrap();
    assert_eq!((m.len(), m[0].len()), (1, 3), "{m:?}");

    // Moved as one: every part goes, each in its style.
    let o = line(&p, "A bold word");
    let was = o.bbox;
    p.set_object_bbox(0, o.id, [was[0] + 50.0, was[1], was[2] + 50.0, was[3]])
        .unwrap();
    let o = line(&p, "A bold word");
    assert!((o.bbox[0] - was[0] - 50.0).abs() < 0.05, "{:?}", o.bbox);
    assert_eq!(o.spans.len(), 3);
    assert_eq!(o.spans[1].font_name, "Helvetica-Bold");

    // The bold word redacted: its characters go, the others stay.
    let mut p = make(MIXED);
    let b = p.find_text_boxes(0, "bold").unwrap()[0];
    let report = p.redact(&[(0, b)]).unwrap();
    assert_eq!(report.pages[0].text, ["bold"]);
    let t = p.page_text(0);
    assert!(
        t.contains('A') && t.contains("word") && !t.contains("bold"),
        "{t:?}"
    );

    // Drawn as before when written back as it is: the same ink.
    let ink = |p: &mut Pdf| {
        let hp = render::load(p.snapshot().unwrap(), None).unwrap();
        let bm = render::Renderer::new(&hp).render(0, 1.0).unwrap();
        (0..bm.width)
            .filter(|&x| (85..101).any(|y| bm.rgba[(y * bm.width + x) * 4] < 128))
            .count()
    };
    let mut a = make(MIXED);
    let mut b = make(MIXED);
    let o = line(&b, "A bold word");
    b.edit_text(0, o.id, &TextEdit::in_parts(as_they_are(&o)))
        .unwrap();
    assert_eq!(ink(&mut a), ink(&mut b));
}

#[test]
fn new_text_can_be_added_in_parts() {
    let mut p = make("");
    let f = |name: &str| PartFont::Choice(FontChoice::from_name(name));
    let part = |text: &str, font: &str, size: f32, rise: f32| TextPart {
        text: text.into(),
        font: f(font),
        size,
        color: BLACK,
        rise,
    };
    p.add_text_parts(
        0,
        (20.0, 100.0),
        &[
            part("Plain ", "Helvetica", 20.0, 0.0),
            part("bold", "Helvetica-Bold", 20.0, 0.0),
            part("2", "Helvetica", 12.0, 5.0),
        ],
    )
    .unwrap();
    let o = line(&p, "Plain bold2");
    assert_eq!(
        styles(&o),
        [
            style("Helvetica", 20.0, BLACK, 0.0, "Plain "),
            style("Helvetica-Bold", 20.0, BLACK, 0.0, "bold"),
            style("Helvetica", 12.0, BLACK, 5.0, "2"),
        ]
    );
    let b = o.baseline.unwrap();
    assert!(
        (b.0 - 20.0).abs() < 0.01 && (b.1 - 100.0).abs() < 0.01,
        "{b:?}"
    );
    assert!(p
        .add_text_parts(0, (20.0, 50.0), &[part("", "Helvetica", 20.0, 0.0)])
        .is_err());
}
