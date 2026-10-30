//! The text pipeline (the text design, NEW-02; batch 45): every writer sets new text through
//! one pipeline (`src/layout.rs`). Each word goes to the first font that has all of it,
//! falling back where the font asked for lacks a character; it is shaped as the font asks
//! (kerning, ligatures, marks); it is measured and broken into lines as it is written; and it
//! reads back as typed. The test fonts are the fallbacks where nothing else is installed, so
//! the tests mean the same on every machine: Limare Han Test for Chinese, Japanese and
//! Korean, Limare Emoji Test for 😀.

mod common;

use common::saved::tools;
use limarepdf_core::{
    add_font_files, bundled_face, fallbacks, typeface, FieldValue, FontChoice, HeaderFooter,
    InkColor, ObjectKind, PageObject, Pdf, Stamp, StampColor, StampPlace, TextEdit, TextStyle,
    Watermark,
};
use lopdf::{dictionary, Object, Stream};
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/fonts/");
const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/");

/// A page wide enough for [`TEXT`] on one line.
const W: f32 = 600.0;
const H: f32 = 300.0;

/// Text every writer writes: a ligature (fi), kerning pairs (AV, To, Te), a letter typed
/// with its accent apart, Greek, Cyrillic, Chinese, Japanese, Korean and a single emoji.
const TEXT: &str = "fine AVATAR To Te cafe\u{301} Ωmega жёлтый 漢字 日本語 한국어 😀";
/// Shorter, for the writers that set text large.
const SHORT: &str = "fine AV To cafe\u{301} Ωmega жёлтый 漢字 한 😀";
/// A character no font has: a noncharacter (some fonts map unassigned ones).
const NONE: char = '\u{2FFFE}';

fn page() -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, W, H).unwrap();
    p
}

fn reopened(p: &mut Pdf) -> Pdf {
    Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap()
}

fn file(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

/// The test fonts as installed families: Source Serif 4 (CFF), Caveat (TrueType, variable),
/// Limare Emoji Test and Limare Han Test (no Latin letters but A, so not offered: a fallback
/// only). The face of the family named `name`.
fn installed(name: &str) -> FontChoice {
    static NUMBERS: OnceLock<Vec<u16>> = OnceLock::new();
    let numbers = NUMBERS.get_or_init(|| {
        let files = [
            format!("{FONTS}SourceSerif4-Regular.otf"),
            format!("{ASSETS}Caveat.ttf"),
            format!("{FONTS}LimareEmojiTest.ttf"),
            format!("{FONTS}LimareHanTest.ttf"),
        ];
        let paths: Vec<&std::path::Path> = files.iter().map(std::path::Path::new).collect();
        add_font_files(&paths)
    });
    let n = (numbers.iter().copied())
        .find(|&n| typeface(n).unwrap().name == name)
        .unwrap_or_else(|| panic!("{name} among the test fonts"));
    FontChoice {
        family: limarepdf_core::Family::Installed(n),
        bold: false,
        italic: false,
    }
}

/// `text` as compared: accents put together, and spaces (and line ends) as single spaces.
fn plain(text: &str) -> String {
    let text: String = text.nfc().collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `want` is among the first page's text of `p`, saved and opened again.
fn reads(p: &mut Pdf, want: &str) -> bool {
    plain(&reopened(p).page_text(0)).contains(&plain(want))
}

/// The first page's line of text that reads `text`.
fn line(p: &Pdf, text: &str) -> PageObject {
    let objs = p.page_objects(0).unwrap().into_iter();
    let mut found = objs.filter(|o| o.kind == ObjectKind::Text && o.text.as_deref() == Some(text));
    found.next().unwrap_or_else(|| panic!("no line {text:?}"))
}

/// A form with one text field, "one", across the page, drawn in Helvetica.
fn form(p: &mut Pdf) {
    let page = p.page_ids()[0];
    let rect: Vec<Object> = [20, 250, 580, 274].map(Object::from).into();
    let field = dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "P" => page,
        "T" => Object::string_literal("one"), "Rect" => rect,
    };
    let id = p.doc.add_object(field);
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", vec![id.into()]);
    let form = p.doc.add_object(dictionary! {
        "Fields" => vec![id.into()], "DA" => Object::string_literal("/Helv 0 Tf 0 g"),
    });
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
}

/// Every writer's text, each on a page of its own: what it is, and the document.
fn every_writer() -> Vec<(String, Pdf)> {
    installed("Limare Han Test");
    let mut out = Vec::new();
    let serif = installed("Source Serif 4");
    for f in [FontChoice::default(), serif, installed("Caveat")] {
        let mut p = page();
        p.add_text(0, (20.0, 40.0), TEXT, f, 12.0, [0.0; 3])
            .unwrap();
        out.push((format!("new text in {}", f.family_name()), p));
    }
    let mut p = page();
    p.add_text(0, (20.0, 40.0), "x", FontChoice::default(), 12.0, [0.0; 3])
        .unwrap();
    let id = line(&p, "x").id;
    let restyled = TextEdit {
        text: TEXT.into(),
        font: Some(serif),
        size: None,
        color: None,
        parts: Vec::new(),
    };
    p.edit_text(0, id, &restyled).unwrap();
    out.push(("a line restyled".into(), p));
    let mut p = page();
    let style = TextStyle::default();
    (p.add_text_box(0, [20.0, 20.0, 580.0, 60.0], TEXT, &style, "Sam")).unwrap();
    p.flatten_annotations(None).unwrap();
    out.push(("a text box".into(), p));
    let mut p = page();
    (p.add_sign_text(0, (300.0, 200.0), TEXT, InkColor::Blue, "Sam")).unwrap();
    p.flatten_annotations(None).unwrap();
    out.push(("fill and sign text".into(), p));
    let mut p = page();
    form(&mut p);
    p.set_field("one", FieldValue::Text(TEXT.into())).unwrap();
    p.flatten_form(None).unwrap();
    out.push(("a form value".into(), p));
    let mut p = page();
    let hf = HeaderFooter {
        header: [TEXT.into(), String::new(), String::new()],
        ..HeaderFooter::default()
    };
    p.add_header_footer(&[0], &hf).unwrap();
    out.push(("a header".into(), p));
    let mut p = page();
    let wm = Watermark {
        text: SHORT.into(),
        font_size: 14.0,
        rotation: 0.0,
        ..Watermark::default()
    };
    p.add_watermark(&[0], &wm).unwrap();
    out.push(("a watermark".into(), p));
    let mut p = page();
    let stamp = Stamp::Custom {
        text: SHORT,
        color: StampColor::Blue,
        name_and_date: false,
    };
    (p.add_stamp(0, &stamp, StampPlace::Fill([10.0, 100.0, 590.0, 200.0]), "")).unwrap();
    p.flatten_annotations(None).unwrap();
    out.push(("a stamp".into(), p));
    out.push((
        "selected text as a plain page".into(),
        Pdf::from_text(TEXT, (W, H)).unwrap(),
    ));
    out
}

/// What a writer's text should read: a stamp's in capitals.
fn wanted(what: &str) -> String {
    match what {
        "a stamp" => SHORT.to_uppercase(),
        "a watermark" => SHORT.to_string(),
        _ => TEXT.to_string(),
    }
}

#[test]
fn every_writer_s_text_reads_back_as_typed() {
    for (what, mut p) in every_writer() {
        let want = wanted(&what);
        let got = plain(&reopened(&mut p).page_text(0));
        assert!(got.contains(&plain(&want)), "{what}: {got:?}");
    }
}

/// Every writer's text, read by Poppler (`pdftotext`). Opt-in: `cargo test -p limarepdf-core
/// --test text_pipeline -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_every_writer_with_pdftotext() {
    for (what, mut p) in every_writer() {
        let got = plain(&tools::pdftotext(&p.to_bytes().unwrap()));
        assert!(got.contains(&plain(&wanted(&what))), "{what}: {got:?}");
    }
}

/// A font file, and the variation settings it is set at.
type Set<'a> = (&'a [u8], Vec<(&'a [u8; 4], f32)>);

/// Where harfrust places each character that starts a glyph of `text` set in the font file
/// and settings `set`: its index, and how far along the line it starts at `size`, in points.
fn shaped(set: &Set, text: &str, size: f32) -> Vec<(usize, f32)> {
    let font = harfrust::FontRef::new(set.0).unwrap();
    let shaper_data = harfrust::ShaperData::new(&font);
    let instance = (!set.1.is_empty()).then(|| {
        let at = set.1.iter().map(|(t, v)| harfrust::Variation {
            tag: harfrust::Tag::new(t),
            value: *v,
        });
        harfrust::ShaperInstance::from_variations(&font, at)
    });
    let builder = shaper_data.shaper(&font);
    let shaper = builder.instance(instance.as_ref()).build();
    let mut buffer = harfrust::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    buffer.set_cluster_level(harfrust::BufferClusterLevel::MonotoneCharacters);
    let out = shaper.shape(buffer, harfrust::ShapeOptions::new());
    let k = size / shaper.units_per_em() as f32;
    let starts: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    let (mut pen, mut last) = (0, None);
    let mut at = Vec::new();
    for (info, place) in out.glyph_infos().iter().zip(out.glyph_positions()) {
        if last != Some(info.cluster) {
            let i = starts.partition_point(|&b| b < info.cluster as usize);
            at.push((i, (pen + place.x_offset) as f32 * k));
            last = Some(info.cluster);
        }
        pen += place.x_advance;
    }
    at
}

#[test]
fn glyphs_land_where_harfrust_places_them() {
    let serif = file(&format!("{FONTS}SourceSerif4-Regular.otf"));
    let caveat = file(&format!("{ASSETS}Caveat.ttf"));
    let sans = bundled_face("Liberation Sans").unwrap().to_vec();
    let size = 12.0;
    // Helvetica's embedded face (Ł isn't in WinAnsi); Source Serif 4, a CFF font whose
    // codes are its glyphs' numbers; Caveat, TrueType and variable, with codes of its own
    // and contextual forms.
    let (words, polish) = ("AVATAR To fine Te WAVE", "AVATAR To Te Łódź WAVE");
    let regular = vec![(b"wght", 400.0)];
    let cases: [(FontChoice, Set, &str); 3] = [
        (FontChoice::default(), (&sans, vec![]), polish),
        (installed("Source Serif 4"), (&serif, vec![]), words),
        (installed("Caveat"), (&caveat, regular), words),
    ];
    for (f, set, text) in cases {
        let mut p = page();
        p.add_text(0, (20.0, 40.0), text, f, size, [0.0; 3])
            .unwrap();
        let back = reopened(&mut p);
        let o = line(&back, text);
        for (i, x) in shaped(&set, text, size) {
            let got = o.stops[i];
            assert!(
                (got - x).abs() < 0.01,
                "{} {text:?}: character {i} at {got}, harfrust {x}",
                f.family_name()
            );
        }
    }
}

#[test]
fn a_word_goes_whole_to_a_font_that_has_it_and_an_ideograph_alone() {
    installed("Limare Han Test");
    let (serif, caveat) = (installed("Source Serif 4"), installed("Caveat"));
    // Caveat has no Greek: the Greek word goes whole, its Latin letters with it.
    let gone = fallbacks(caveat, "say Ωmega now");
    assert_eq!(gone.len(), 1, "{gone:?}");
    assert_eq!(gone[0].0, "Ωmega");
    assert_ne!(gone[0].1, caveat);
    // Source Serif 4 has no Chinese: the ideograph goes alone, to a font that has it.
    let gone = fallbacks(serif, "Kan漢ji");
    assert_eq!(gone.len(), 1, "{gone:?}");
    assert_eq!(gone[0].0, "漢");
    let mut p = page();
    p.add_text(0, (20.0, 40.0), "Kan漢ji", serif, 12.0, [0.0; 3])
        .unwrap();
    let back = reopened(&mut p);
    let o = line(&back, "Kan漢ji");
    let fonts: Vec<(&str, (usize, usize))> = (o.spans.iter())
        .map(|s| (s.font_name.as_str(), s.chars))
        .collect();
    assert_eq!(fonts.len(), 3, "{fonts:?}");
    assert!(fonts[0].0.ends_with("SourceSerif4-Regular") && fonts[0].1 == (0, 3));
    assert!(!fonts[1].0.ends_with("SourceSerif4-Regular") && fonts[1].1 == (3, 4));
    assert!(fonts[2].0.ends_with("SourceSerif4-Regular") && fonts[2].1 == (4, 6));
    // What no font has is refused, and said, with nothing changed.
    let mut p = page();
    let before = format!("{:?}", p.doc.objects);
    let kan = format!("Kan{NONE}");
    let e = (p.add_text(0, (20.0, 40.0), &kan, serif, 12.0, [0.0; 3])).unwrap_err();
    let none = format!("No font on this computer has \"{NONE}\"");
    assert_eq!(e.to_string(), none);
    assert_eq!(format!("{:?}", p.doc.objects), before);
}

/// The font objects LimarePDF wrote in `p`, by their names without the subset tag.
fn own_fonts(p: &Pdf) -> Vec<(lopdf::ObjectId, String)> {
    let mut out: Vec<(lopdf::ObjectId, String)> = (p.doc.objects.iter())
        .filter_map(|(&id, o)| Some((id, o.as_dict().ok()?)))
        .filter(|(_, d)| d.has(b"LimarePDF"))
        .map(|(id, d)| {
            let base = d.get(b"BaseFont").unwrap().as_name().unwrap();
            let base = String::from_utf8_lossy(base).into_owned();
            (id, base.split_once('+').unwrap().1.to_string())
        })
        .collect();
    out.sort();
    out
}

/// The `/CIDToGIDMap` of font object `id`'s descendant: its glyph for each code, or `None`
/// for `/Identity`.
fn cid_map(p: &Pdf, id: lopdf::ObjectId) -> Option<Vec<u16>> {
    let font = p.doc.get_dictionary(id).unwrap();
    let cid = font.get(b"DescendantFonts").unwrap().as_array().unwrap()[0]
        .as_reference()
        .unwrap();
    let cid = p.doc.get_dictionary(cid).unwrap();
    let map = cid.get(b"CIDToGIDMap").unwrap().as_reference().ok()?;
    let stream = p.doc.get_object(map).unwrap().as_stream().unwrap();
    let bytes = (stream.decompressed_content()).unwrap_or(stream.content.clone());
    let glyphs = bytes.chunks(2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    Some(glyphs.collect())
}

#[test]
fn letters_two_scripts_draw_with_one_glyph_read_back_as_themselves() {
    // Limare Han Test draws Latin A and Cyrillic А with one glyph: two codes of the font's
    // own, each with its text, map to it.
    let han = installed("Limare Han Test");
    let mut p = page();
    p.add_text(0, (20.0, 40.0), "AА", han, 12.0, [0.0; 3])
        .unwrap();
    let back = reopened(&mut p);
    assert_eq!(back.page_text(0).trim(), "AА");
    let fonts = own_fonts(&back);
    assert_eq!(fonts.len(), 1, "{fonts:?}");
    let map = cid_map(&back, fonts[0].0).unwrap();
    assert_eq!(map.len(), 3, "code 0 and the two codes: {map:?}");
    assert_eq!(map[1], map[2]);
}

/// The font object for `face` LimarePDF wrote in `p` made as LimarePDF 0.11 made it: each
/// code its glyph's number in the font file, written for one character, widths rounded, no
/// map.
fn as_0_11(p: &mut Pdf, face: &str) {
    let (id, _) = own_fonts(p).into_iter().find(|(_, n)| n == face).unwrap();
    let font = p.doc.get_dictionary(id).unwrap().clone();
    let own = font.get(b"LimarePDF").unwrap().as_dict().unwrap().clone();
    let texts = own.get(b"Texts").unwrap().as_array().unwrap();
    let glyphs: Vec<Object> = (texts.chunks(2))
        .flat_map(|pair| {
            let c = String::from_utf8(pair[1].as_str().unwrap().to_vec()).unwrap();
            let c = c.chars().next().unwrap();
            [pair[0].clone(), Object::Integer(c as i64)]
        })
        .collect();
    let mut old = own.clone();
    old.remove(b"Texts");
    old.remove(b"Codes");
    old.remove(b"Widths");
    old.set("Glyphs", glyphs);
    let cid = font.get(b"DescendantFonts").unwrap().as_array().unwrap()[0]
        .as_reference()
        .unwrap();
    let d = p.doc.get_dictionary_mut(cid).unwrap();
    d.set("CIDToGIDMap", "Identity");
    let w = &d.get(b"W").unwrap().as_array().unwrap()[1];
    let rounded: Vec<Object> = (w.as_array().unwrap().iter())
        .map(|x| Object::Integer(x.as_float().unwrap().round() as i64))
        .collect();
    d.set("W", vec![0.into(), rounded.into()]);
    (p.doc.get_dictionary_mut(id).unwrap()).set("LimarePDF", old);
}

#[test]
fn fonts_limarepdf_0_11_wrote_go_on_growing_beside_new_ones() {
    let han = installed("Limare Han Test");
    let mut p = page();
    p.add_text(0, (20.0, 40.0), "漢A", han, 12.0, [0.0; 3])
        .unwrap();
    as_0_11(&mut p, "LimareHanTest-Regular");
    let mut p = reopened(&mut p);
    // 字 and the space join the old font, its codes still its glyphs' numbers; Cyrillic А
    // needs glyph A for another text, which that font can't give it: a new font has it.
    p.add_text(0, (20.0, 80.0), "字A А", han, 12.0, [0.0; 3])
        .unwrap();
    let back = reopened(&mut p);
    let text = back.page_text(0);
    assert!(text.contains("漢A") && text.contains("字A А"), "{text:?}");
    let fonts = own_fonts(&back);
    assert_eq!(fonts.len(), 2, "{fonts:?}");
    let maps: Vec<Option<Vec<u16>>> = fonts.iter().map(|(id, _)| cid_map(&back, *id)).collect();
    // The old font keeps Identity; the new one has one code of its own.
    assert!(maps.contains(&None), "{maps:?}");
    let one = |m: &Option<Vec<u16>>| m.as_ref().is_some_and(|m| m.len() == 2);
    assert!(maps.iter().any(one), "{maps:?}");
}

#[test]
fn a_radical_a_font_s_map_gives_reads_as_the_ideograph_it_draws() {
    // PB-57: Chromium's Chinese and Japanese text maps some glyphs to radicals, ⼯ for 工.
    let han = installed("Limare Han Test");
    let mut p = page();
    p.add_text(0, (20.0, 40.0), "漢字", han, 12.0, [0.0; 3])
        .unwrap();
    let (id, _) = own_fonts(&p).remove(0);
    let to_unicode = (p.doc.get_dictionary(id).unwrap())
        .get(b"ToUnicode")
        .unwrap()
        .as_reference()
        .unwrap();
    let map = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
               1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
               2 beginbfchar <0001> <2F2F> <0002> <2E9F> endbfchar\n\
               endcmap CMapName currentdict /CMap defineresource pop end end\n";
    let stream = Stream::new(lopdf::Dictionary::new(), map.as_bytes().to_vec());
    p.doc.objects.insert(to_unicode, stream.into());
    // Kangxi ⼯ and the supplement's ⺟ (mother).
    assert!(reads(&mut p, "工母"), "{:?}", reopened(&mut p).page_text(0));
}
