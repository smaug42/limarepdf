//! Paragraphs (NEW-01, batch 45's stage 2) and the lines they are read from.
//!
//! The blocks a page's text makes (`Pdf::page_blocks`): lines drawn in text objects of their
//! own, or in pieces, joined into paragraphs with their frames; headings, tables and other
//! blocks kept apart and said to be uncertain; lines of different tagged paragraphs never
//! joined. The answer sets' check is `tests/paragraph_check.rs`.
//!
//! Text in a Type 3 font whose matrix flips its glyphs back upright, as TeX's bitmap fonts
//! are drawn (PB-56), reads upright, at the size it is drawn, in whole lines.

use limarepdf_core::{
    text_width, Align, Block, Detect, Doubt, ObjectKind, PageObject, Pdf, TextEdit,
};
use lopdf::{dictionary, Dictionary, Object, Stream};

/// A US Letter page drawing `content`, with Helvetica as `/F1` and Helvetica-Bold as `/F2`.
fn letter(content: &str) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    let f1 = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let f2 = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica-Bold", "Encoding" => "WinAnsiEncoding",
    });
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", s);
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => f1, "F2" => f2 } },
    );
    p
}

const TEXT: &str = "The harbour was quiet that morning, and the fishing boats sat low in the \
    water while gulls argued over the nets drying along the wall. Nobody on the quay was in a \
    hurry; the tide would turn when it turned, and the old men who mended the nets had seen \
    enough tides to know that worrying never brought one in sooner than the moon allowed.";

const MORE: &str = "By noon the wind had freshened from the west, and the first of the boats \
    slipped its moorings with a clatter of chain and a shout from the deck that carried right \
    across the water to the houses on the far side of the bay where washing flapped on lines.";

/// `text` broken into lines of at most `width` points in Helvetica at `size`.
fn wrap(text: &str, width: f32, size: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(l) if text_width(&format!("{l} {word}"), size) <= width => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// Lines in text objects of their own, as most producers draw them: the first at `x + indent`,
/// the rest at `x`, baselines `leading` apart down from `top` (points up the page), in `font`
/// at `size`.
fn lines_at(
    lines: &[String],
    x: f32,
    indent: f32,
    top: f32,
    leading: f32,
    font: &str,
    size: f32,
) -> String {
    let mut out = String::new();
    for (k, l) in lines.iter().enumerate() {
        let at = if k == 0 { x + indent } else { x };
        let y = top - leading * k as f32;
        out.push_str(&format!("BT /{font} {size} Tf {at} {y} Td ({l}) Tj ET\n"));
    }
    out
}

/// The blocks of page 0 that hold text reading `start`.
fn block_from<'a>(p: &Pdf, blocks: &'a [Block], start: &str) -> &'a Block {
    let objs = p.page_objects(0).unwrap();
    let text =
        |b: &Block| -> String { objs[b.lines[0].pieces[0]].text.clone().unwrap_or_default() };
    (blocks.iter())
        .find(|b| text(b).starts_with(start))
        .unwrap_or_else(|| {
            panic!(
                "no block from {start:?} in {:?}",
                blocks.iter().map(text).collect::<Vec<_>>()
            )
        })
}

#[test]
fn a_paragraph_drawn_a_line_at_a_time_is_one_confident_block() {
    let lines = wrap(TEXT, 440.0, 11.0);
    assert!(lines.len() >= 4, "{lines:?}");
    let p = letter(&lines_at(&lines, 72.0, 0.0, 700.0, 13.2, "F1", 11.0));
    let blocks = p.page_blocks(0).unwrap();
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    let b = &blocks[0];
    assert!(b.confident(), "{:?}", b.doubt);
    assert_eq!(b.lines.len(), lines.len());
    // One text object a line, in content order.
    let pieces: Vec<usize> = b.pieces().collect();
    assert_eq!(pieces, (0..lines.len()).collect::<Vec<_>>());
    let f = b.frame;
    assert!((f.left - 72.0).abs() < 0.5, "{f:?}");
    assert!((f.baseline - 92.0).abs() < 0.5, "{f:?}");
    assert!((f.spacing - 13.2).abs() < 0.05, "{f:?}");
    assert_eq!(f.align, Align::Left);
    assert!(f.indent.abs() < 0.5, "{f:?}");
    assert!(f.right > 72.0 + 400.0 && f.right <= 72.0 + 441.0, "{f:?}");
}

#[test]
fn a_line_drawn_in_pieces_is_one_visual_line() {
    // The middle line drawn word by word in text objects of their own, as Word draws some.
    let lines = wrap(TEXT, 440.0, 11.0);
    let mut content = String::new();
    for (k, l) in lines.iter().enumerate() {
        let y = 700.0 - 13.2 * k as f32;
        if k == 1 {
            let mut x = 72.0;
            for word in l.split(' ') {
                content.push_str(&format!("BT /F1 11 Tf {x} {y} Td ({word}) Tj ET\n"));
                x += text_width(&format!("{word} "), 11.0);
            }
        } else {
            content.push_str(&format!("BT /F1 11 Tf 72 {y} Td ({l}) Tj ET\n"));
        }
    }
    let p = letter(&content);
    let blocks = p.page_blocks(0).unwrap();
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    let words = lines[1].split(' ').count();
    assert_eq!(blocks[0].lines[1].pieces.len(), words);
    assert_eq!(
        blocks[0].lines[1].pieces,
        (1..1 + words).collect::<Vec<_>>()
    );
    assert!(blocks[0].confident(), "{:?}", blocks[0].doubt);
}

#[test]
fn a_heading_and_paragraphs_are_apart() {
    let first = wrap(TEXT, 440.0, 11.0);
    let second = wrap(MORE, 440.0, 11.0);
    let mut content = String::from("BT /F2 16 Tf 72 720 Td (Morning in the harbour) Tj ET\n");
    content.push_str(&lines_at(&first, 72.0, 0.0, 690.0, 13.2, "F1", 11.0));
    // The second paragraph's first line indented, as a book sets it, with no space between.
    let top = 690.0 - 13.2 * first.len() as f32;
    content.push_str(&lines_at(&second, 72.0, 18.0, top, 13.2, "F1", 11.0));
    let p = letter(&content);
    let blocks = p.page_blocks(0).unwrap();
    assert_eq!(blocks.len(), 3, "{blocks:?}");
    assert_eq!(
        block_from(&p, &blocks, "Morning").doubt,
        Some(Doubt::OneLine)
    );
    let a = block_from(&p, &blocks, "The harbour");
    let b = block_from(&p, &blocks, "By noon");
    assert!(
        a.confident() && b.confident(),
        "{:?} {:?}",
        a.doubt,
        b.doubt
    );
    assert_eq!((a.lines.len(), b.lines.len()), (first.len(), second.len()));
    assert!((b.frame.indent - 18.0).abs() < 0.5, "{:?}", b.frame);
    assert!((b.frame.left - 72.0).abs() < 0.5, "{:?}", b.frame);
}

#[test]
fn three_columns_on_common_baselines_are_three_paragraphs() {
    // Small type on loose spacing, ragged right, a paragraph a column with their baselines
    // in line, as LibreOffice sets three columns: the gap under each line is a band across
    // all three, which mustn't cut the columns into one-line pieces that then don't line up.
    let third = "Late in the afternoon the boats came back one by one, low with the catch, \
                 and the women on the quay sorted the fish into crates while the children ran \
                 errands between the sheds and the market hall.";
    let mut content = String::from("BT /F2 16 Tf 45 740 Td (A day in the harbour) Tj ET\n");
    let columns = [(45.0, TEXT), (223.0, MORE), (401.0, third)];
    let mut counts = Vec::new();
    for (x, text) in columns {
        let lines = wrap(text, 145.0, 8.0);
        counts.push(lines.len());
        content.push_str(&lines_at(&lines, x, 0.0, 700.0, 13.0, "F1", 8.0));
    }
    let p = letter(&content);
    let blocks = p.page_blocks(0).unwrap();
    for ((x, text), n) in columns.iter().zip(&counts) {
        let start: String = text.split(' ').take(2).collect::<Vec<_>>().join(" ");
        let b = block_from(&p, &blocks, &start);
        assert_eq!(b.lines.len(), *n, "the column at {x}: {blocks:?}");
        assert!((b.frame.left - x).abs() < 0.5, "{:?}", b.frame);
    }
    assert_eq!(blocks.len(), 4, "{blocks:?}");
}

/// `lines` in text objects of their own, each in the marked content of its element: line k
/// in element `elements[k]`, with a structure tree holding the elements as paragraphs.
fn tagged(lines: &[String], elements: &[usize]) -> Pdf {
    let mut content = String::new();
    for (k, l) in lines.iter().enumerate() {
        let y = 700.0 - 13.2 * k as f32;
        content.push_str(&format!(
            "/P <</MCID {k}>> BDC BT /F1 11 Tf 72 {y} Td ({l}) Tj ET EMC\n"
        ));
    }
    let mut p = letter(&content);
    let root = p.doc.new_object_id();
    let n = elements.iter().max().unwrap() + 1;
    let elems: Vec<lopdf::ObjectId> = (0..n)
        .map(|_| {
            p.doc
                .add_object(dictionary! { "Type" => "StructElem", "S" => "P", "P" => root })
        })
        .collect();
    let parents: Vec<Object> = elements.iter().map(|&e| elems[e].into()).collect();
    let kids: Vec<Object> = elems.iter().map(|&e| e.into()).collect();
    let tree = dictionary! { "Nums" => vec![0.into(), Object::Array(parents)] };
    p.doc.objects.insert(
        root,
        Object::Dictionary(
            dictionary! { "Type" => "StructTreeRoot", "K" => kids, "ParentTree" => tree },
        ),
    );
    let page = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("StructParents", 0);
    let catalog = p.doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let c = p.doc.get_dictionary_mut(catalog).unwrap();
    c.set("StructTreeRoot", root);
    c.set("MarkInfo", dictionary! { "Marked" => true });
    p
}

#[test]
fn lines_of_different_tagged_paragraphs_are_never_joined() {
    // Two paragraphs with nothing between them to see: the tags say where one ends.
    let mut lines = wrap(TEXT, 440.0, 11.0);
    let n = lines.len();
    lines.extend(wrap(MORE, 440.0, 11.0));
    let elements: Vec<usize> = (0..lines.len()).map(|k| usize::from(k >= n)).collect();
    let p = tagged(&lines, &elements);
    let blocks = p.page_blocks(0).unwrap();
    assert_eq!(blocks.len(), 2, "{blocks:?}");
    assert_eq!(blocks[0].lines.len(), n);
    // Without the tags, the next word's fit says the first ends short of its measure.
    let geometry = p
        .page_blocks_by(
            0,
            Detect {
                tags: false,
                fonts: true,
            },
        )
        .unwrap();
    assert_eq!(geometry.len(), 2, "{geometry:?}");
    // One element for all: the tags don't keep apart what is one.
    let one = tagged(&lines, &vec![0; lines.len()]);
    assert_eq!(one.page_blocks(0).unwrap().len(), 2);
}

#[test]
fn a_table_is_not_a_paragraph() {
    // Three columns of cells on shared baselines.
    let mut content = String::new();
    for row in 0..6 {
        let y = 700.0 - 14.0 * row as f32;
        for (col, x) in [72.0, 250.0, 430.0].into_iter().enumerate() {
            content.push_str(&format!(
                "BT /F1 10 Tf {x} {y} Td (cell {row} of column {col}) Tj ET\n"
            ));
        }
    }
    let p = letter(&content);
    let blocks = p.page_blocks(0).unwrap();
    assert!(!blocks.is_empty());
    assert!(blocks.iter().all(|b| !b.confident()), "{blocks:?}");
}

#[test]
fn justified_text_has_a_justified_frame() {
    // Every line but the last spread to the right edge with word spacing.
    let lines = wrap(TEXT, 440.0, 11.0);
    let mut content = String::new();
    for (k, l) in lines.iter().enumerate() {
        let y = 700.0 - 13.2 * k as f32;
        let spaces = l.matches(' ').count().max(1) as f32;
        let tw = if k + 1 < lines.len() {
            (440.0 - text_width(l, 11.0)) / spaces
        } else {
            0.0
        };
        content.push_str(&format!("BT /F1 11 Tf {tw} Tw 72 {y} Td ({l}) Tj ET\n"));
    }
    let p = letter(&content);
    let blocks = p.page_blocks(0).unwrap();
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    assert!(blocks[0].confident(), "{:?}", blocks[0].doubt);
    assert_eq!(blocks[0].frame.align, Align::Justified);
    assert!(
        (blocks[0].frame.right - 512.0).abs() < 2.0,
        "{:?}",
        blocks[0].frame
    );
}

#[test]
fn hidden_and_turned_text_is_in_no_paragraph() {
    let lines = wrap(TEXT, 440.0, 11.0);
    let mut content = lines_at(&lines, 72.0, 0.0, 700.0, 13.2, "F1", 11.0);
    // An OCR layer's invisible line, and a line turned up the margin.
    content.push_str("q BT 3 Tr /F1 11 Tf 72 500 Td (invisible words) Tj ET Q\n");
    content.push_str("BT /F1 11 Tf 0 1 -1 0 40 300 Tm (up the margin) Tj ET\n");
    let p = letter(&content);
    let blocks = p.page_blocks(0).unwrap();
    let in_blocks: Vec<usize> = blocks.iter().flat_map(Block::pieces).collect();
    let n = lines.len();
    assert!(!in_blocks.contains(&n), "the hidden line is in {blocks:?}");
    let turned = blocks
        .iter()
        .find(|b| b.pieces().any(|i| i == n + 1))
        .expect("the turned line");
    assert!(!turned.confident());
    assert!(blocks.iter().any(|b| b.confident() && b.lines.len() == n));
}

#[test]
fn a_paragraph_in_a_flipped_type3_font_is_found_and_its_font_said() {
    // Three lines in TeX's bitmap font, the second reading on from the first: upright and
    // whole (PB-56), but new text in the font goes to another face (the plan's Q3).
    let mut content = String::from("BT /T3 1 Tf\n");
    for (k, line) in ["Stony Brook ton tony", "rook torn knot Brook", "tony rook"]
        .iter()
        .enumerate()
    {
        let y = 700.0 - 14.0 * k as f32;
        content.push_str(&format!("0.12 0 0 -0.12 72 {y} Tm ({line}) Tj\n"));
    }
    content.push_str("ET");
    let letters = [STONY_BROOK, &[(' ', 30)]].concat();
    let p = page_with(|p| tex_bitmap_font(p, &letters), &content);
    let any_font = Detect {
        tags: false,
        fonts: false,
    };
    let blocks = p.page_blocks_by(0, any_font).unwrap();
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    assert_eq!(blocks[0].lines.len(), 3);
    assert!(blocks[0].confident(), "{:?}", blocks[0].doubt);
    assert!(
        (blocks[0].frame.spacing - 14.0).abs() < 0.05,
        "{:?}",
        blocks[0].frame
    );
    let blocks = p.page_blocks(0).unwrap();
    assert_eq!(blocks[0].lines.len(), 3);
    assert_eq!(blocks[0].doubt, Some(Doubt::Font));
}

/// The page's lines of text, as read.
fn lines(p: &Pdf) -> Vec<PageObject> {
    let objs = p.page_objects(0).unwrap().into_iter();
    objs.filter(|o| o.kind == ObjectKind::Text).collect()
}

fn texts(p: &Pdf) -> Vec<String> {
    lines(p)
        .into_iter()
        .map(|o| o.text.unwrap_or_default())
        .collect()
}

/// A 612 × 792 page drawing `content`, with the font `font` adds as `/T3`.
fn page_with(font: impl FnOnce(&mut Pdf) -> Dictionary, content: &str) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    let font = font(&mut p);
    let f = p.doc.add_object(font);
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", s);
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "T3" => f } },
    );
    p
}

/// A Type 3 font as dvips writes TeX's bitmap fonts: its glyphs in pixels at 600 dpi (83
/// to the em of a 10-point font), its matrix `[1 0 0 -1 0 0]` turning them up the right
/// way when the text matrix turns text space upside down. Each letter of `letters` is a
/// box `width` pixels wide.
fn tex_bitmap_font(p: &mut Pdf, letters: &[(char, i64)]) -> Dictionary {
    box_font(p, letters, [1.0, 0.0, 0.0, -1.0], (0, 58))
}

/// A Type 3 font of boxes: each letter of `letters` one `width` glyph units wide, giving
/// its box (`d1`) and drawn from `ink.0` to `ink.1` up glyph space; its matrix `matrix`.
fn box_font(p: &mut Pdf, letters: &[(char, i64)], matrix: [f32; 4], ink: (i64, i64)) -> Dictionary {
    let mut procs = Dictionary::new();
    let mut diffs: Vec<Object> = Vec::new();
    let mut widths: Vec<Object> = Vec::new();
    let first = letters.iter().map(|&(c, _)| c as i64).min().unwrap();
    let last = letters.iter().map(|&(c, _)| c as i64).max().unwrap();
    let (lo, hi) = ink;
    for code in first..=last {
        match letters.iter().find(|&&(c, _)| c as i64 == code) {
            Some(&(c, w)) => {
                let glyph = format!(
                    "{w} 0 2 {lo} {} {hi} d1 2 {lo} {} {} re f",
                    w - 2,
                    w - 4,
                    hi - lo
                );
                let id = p
                    .doc
                    .add_object(Stream::new(Dictionary::new(), glyph.into_bytes()));
                let name = if c == ' ' {
                    "space".into()
                } else {
                    c.to_string()
                };
                procs.set(name.clone(), id);
                diffs.extend([Object::Integer(code), Object::Name(name.into())]);
                widths.push(w.into());
            }
            None => widths.push(0.into()),
        }
    }
    let [a, b, c, d] = matrix;
    dictionary! {
        "Type" => "Font", "Subtype" => "Type3",
        "FontBBox" => vec![0.into(), (lo - 10).into(), 90.into(), (hi + 10).into()],
        "FontMatrix" => vec![a.into(), b.into(), c.into(), d.into(), 0.into(), 0.into()],
        "CharProcs" => procs,
        "Encoding" => dictionary! { "Type" => "Encoding", "Differences" => diffs },
        "FirstChar" => first, "LastChar" => last, "Widths" => widths,
    }
}

/// The letters of "Stony Brook" with their widths in a 10-point Computer Modern Roman at
/// 600 dpi.
const STONY_BROOK: &[(char, i64)] = &[
    ('B', 59),
    ('S', 46),
    ('k', 44),
    ('n', 46),
    ('o', 42),
    ('r', 33),
    ('t', 32),
    ('y', 44),
];

/// "Stony Brook" as dvips draws it: each piece placed from the line's start with `TD`, in
/// text space one pixel to the unit, turned upside down by the text matrix.
fn stony_brook() -> Pdf {
    let content = "BT /T3 1 Tf 0.12 0 0 -0.12 72 700 Tm (Ston) Tj 166 0 TD (y) Tj \
                   74 0 TD (Brook) Tj ET";
    page_with(|p| tex_bitmap_font(p, STONY_BROOK), content)
}

#[test]
fn text_in_a_flipped_type3_font_reads_upright_as_one_line() {
    let p = stony_brook();
    assert_eq!(texts(&p), ["Stony Brook"]);
    let line = &lines(&p)[0];
    assert!(line.upright, "{line:?}");
    assert_eq!(line.dir, Some((1.0, 0.0)));
}

#[test]
fn a_kern_in_a_tex_bitmap_font_is_no_space() {
    // dvips moves the pen inside a word with a `TJ` number in pixels, thousandths of a
    // unit of text space a size of 1 gives: 3,000 is 3 pixels, under 0.04 em in a font of
    // 83 pixels to the em, where a word space is about 28 pixels (PB-64).
    let content = "BT /T3 1 Tf 0.12 0 0 -0.12 72 700 Tm \
                   [(Ston) -2000 (y) -28000 (Bro) -3000 (ok)] TJ ET";
    let letters = [STONY_BROOK, &[(' ', 30)]].concat();
    let p = page_with(|p| tex_bitmap_font(p, &letters), content);
    assert_eq!(texts(&p), ["Stony Brook"]);
    let words: Vec<String> = (p.page_words(0).unwrap().into_iter())
        .map(|w| w.text)
        .collect();
    assert_eq!(words, ["Stony", "Brook"]);
}

#[test]
fn text_in_a_flipped_type3_font_has_the_size_it_is_drawn_at() {
    let p = stony_brook();
    let line = &lines(&p)[0];
    // 83 pixels to the em at 0.12 points a pixel: about 10 points.
    let size = line.font_size.unwrap();
    assert!((8.5..11.5).contains(&size), "{size}");
    assert!(line.spans.iter().all(|s| (s.font_size - size).abs() < 0.01));
    // The box stands on the baseline (y down on the displayed page), the letters above it.
    let [_, top, _, bottom] = line.bbox;
    let base = line.baseline.unwrap().1;
    assert!(bottom - top > 0.6 * size, "{:?}", line.bbox);
    assert!(
        top < base && base < bottom && base - top > bottom - base,
        "{:?}",
        line.bbox
    );
}

/// Rows `rows` of page 0 drawn at a pixel a point: how many dark pixels each has.
fn dark_rows(p: &mut Pdf, rows: std::ops::Range<usize>) -> Vec<usize> {
    let hp = limarepdf_core::render::load(p.snapshot().unwrap(), None).unwrap();
    let bitmap = limarepdf_core::render::Renderer::new(&hp)
        .render(0, 1.0)
        .unwrap();
    let dark = |x: usize, y: usize| bitmap.rgba[(y * bitmap.width + x) * 4] < 128;
    rows.map(|y| (0..bitmap.width).filter(|&x| dark(x, y)).count())
        .collect()
}

#[test]
fn an_edited_line_in_a_flipped_type3_font_keeps_its_letters_upright() {
    let mut p = stony_brook();
    let id = lines(&p)[0].id;
    let size = lines(&p)[0].font_size.unwrap();
    let e = TextEdit {
        text: "Stony Point".into(),
        font: None,
        size: None,
        color: None,
        parts: Vec::new(),
    };
    p.edit_text(0, id, &e).unwrap();
    let edited = lines(&p);
    assert_eq!(texts(&p), ["Stony Point"]);
    let new = edited[0].font_size.unwrap();
    assert!((new - size).abs() < 0.05 * size, "{new} against {size}");
    assert!(edited[0].upright);
    // "Stony" keeps its letters in the font (it has no name), turned over with text space
    // so they stand up; "Point", typed, is in another font.
    let spans = &edited[0].spans;
    assert_eq!(spans[0].font_name, "", "{spans:?}");
    assert_eq!(spans[0].chars.0, 0);
    assert!(spans[0].chars.1 >= 5, "{spans:?}");
    assert!(!spans.last().unwrap().font_name.is_empty(), "{spans:?}");
    // Everything stands on the baseline, 92 points down the page: nothing hangs below it.
    let rows = dark_rows(&mut p, 70..110);
    let (above, below): (usize, usize) = (rows[..23].iter().sum(), rows[24..].iter().sum());
    assert!(above > 200 && below == 0, "{rows:?}");
}

#[test]
fn letters_kept_in_a_flipped_type3_font_stand_up_after_a_typed_word_and_in_place() {
    let edit = |text: &str| {
        let mut p = stony_brook();
        let e = TextEdit {
            text: text.into(),
            font: None,
            size: None,
            color: None,
            parts: Vec::new(),
        };
        p.edit_text(0, lines(&p)[0].id, &e).unwrap();
        p
    };
    // A word typed first, in another font, then the line's own letters turned over.
    let mut p = edit("Big Stony Brook");
    assert_eq!(texts(&p), ["Big Stony Brook"]);
    let line = lines(&p).remove(0);
    assert!(line.upright);
    let last = line.spans.last().unwrap();
    assert_eq!(
        (last.font_name.as_str(), last.chars.1),
        ("", 15),
        "{:?}",
        line.spans
    );
    let rows = dark_rows(&mut p, 70..110);
    assert!(rows[24..].iter().all(|&n| n == 0), "{rows:?}");
    // A letter taken out leaves the rest where they were, drawn as before.
    let mut p = edit("Stony Broo");
    assert_eq!(texts(&p), ["Stony Broo"]);
    let line = lines(&p).remove(0);
    assert!(line.upright && line.spans.iter().all(|s| s.font_name.is_empty()));
    let rows = dark_rows(&mut p, 70..110);
    assert!(rows[24..].iter().all(|&n| n == 0), "{rows:?}");
}

/// "Stony Brook" in the same font set the right way up: its matrix `[1 0 0 1 0 0]`, the
/// text matrix scaling text space to points.
fn stony_brook_upright() -> Pdf {
    let content = "BT /T3 1 Tf 0.12 0 0 0.12 72 700 Tm (Stony Brook) Tj ET";
    let letters = [STONY_BROOK, &[(' ', 30)]].concat();
    page_with(
        |p| box_font(p, &letters, [1.0, 0.0, 0.0, 1.0], (0, 58)),
        content,
    )
}

#[test]
fn text_in_a_type3_font_drawn_upside_down_and_turned_by_its_matrix_reads_upright() {
    // Glyphs drawn below the baseline, 92 units to the em, turned up by the font's
    // matrix, in text space as it is (as pdf.js's `bug1245391` draws them).
    let content = "BT /T3 20 Tf 72 700 Td (Stony Brook) Tj ET";
    let letters = [STONY_BROOK, &[(' ', 30)]].concat();
    let s = 1.0 / 92.0;
    let mut p = page_with(
        |p| box_font(p, &letters, [s, 0.0, 0.0, -s], (-70, 0)),
        content,
    );
    let line = lines(&p).remove(0);
    assert_eq!(line.text.as_deref(), Some("Stony Brook"));
    assert!(line.upright, "{line:?}");
    assert_eq!(line.font_size, Some(20.0));
    let [_, top, _, bottom] = line.bbox;
    let base = line.baseline.unwrap().1;
    assert!(base - top > bottom - base, "{:?} on {base}", line.bbox);
    // Upright as written, its letters stay in it where an edit leaves them.
    let e = TextEdit {
        text: "Stony Point".into(),
        font: None,
        size: None,
        color: None,
        parts: Vec::new(),
    };
    p.edit_text(0, line.id, &e).unwrap();
    let edited = lines(&p).remove(0);
    assert_eq!(edited.text.as_deref(), Some("Stony Point"));
    assert_eq!(edited.spans[0].font_name, "", "{:?}", edited.spans);
}

#[test]
fn an_edited_line_in_a_type3_font_of_its_own_scale_keeps_its_letters_at_their_size() {
    let mut p = stony_brook_upright();
    let line = lines(&p).remove(0);
    assert_eq!(line.text.as_deref(), Some("Stony Brook"));
    assert!(line.upright);
    let size = line.font_size.unwrap();
    assert!((8.5..11.5).contains(&size), "{size}");
    let e = TextEdit {
        text: "Stony Point".into(),
        font: None,
        size: None,
        color: None,
        parts: Vec::new(),
    };
    p.edit_text(0, line.id, &e).unwrap();
    let edited = lines(&p).remove(0);
    assert_eq!(edited.text.as_deref(), Some("Stony Point"));
    // "Stony " kept in its own font, "Point" in another, both at the line's size.
    assert_eq!(edited.spans.len(), 2, "{:?}", edited.spans);
    assert_eq!(edited.spans[0].font_name, "");
    for s in &edited.spans {
        assert!(
            (s.font_size - size).abs() < 0.05 * size,
            "{s:?} against {size}"
        );
    }
}
