//! Paragraphs (NEW-01, batch 45's stage 2) and the lines they are read from.
//!
//! The blocks a page's text makes (`Pdf::page_blocks`): lines drawn in text objects of their
//! own, or in pieces, joined into paragraphs with their frames; headings, tables and other
//! blocks kept apart and said to be uncertain; lines of different tagged paragraphs never
//! joined. The answer sets' check is `tests/paragraph_check.rs`.
//!
//! Text in a Type 3 font whose matrix flips its glyphs back upright, as TeX's bitmap fonts
//! are drawn (PB-56), reads upright, at the size it is drawn, in whole lines.

mod common;

use common::saved::tools;
use limarepdf_core::{
    text_width, Align, Block, Detect, Doubt, Hyphen, ObjectKind, PageObject, Paragraph,
    ParagraphSetting, PartFont, Pdf, SetLine, SetParagraph, TextEdit, TextPart,
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
    add_tree(&mut p, elements);
    p
}

/// A structure tree holding `elements` as paragraphs: marked content `k` in element
/// `elements[k]`.
fn add_tree(p: &mut Pdf, elements: &[usize]) {
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
    let geometry = p.page_blocks_by(0, Detect { tags: false }).unwrap();
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
fn two_lines_are_justified_only_when_the_first_lines_spaces_are_stretched() {
    // Two lines broken at 300 points, the first the longer: its end is no edge.
    let text = "By noon the wind had freshened from the west, and the first of the boats \
        slipped its moorings.";
    let lines = wrap(text, 300.0, 11.0);
    assert_eq!(lines.len(), 2);
    assert!(text_width(&lines[0], 11.0) > text_width(&lines[1], 11.0));
    let p = letter(&lines_at(&lines, 72.0, 0.0, 700.0, 13.2, "F1", 11.0));
    let para = only_paragraph(&p);
    assert_eq!(para.frame.align, Align::Left);
    // A word typed in the first line moves its break: set again unstretched.
    let setting = p.paragraph_setting(&para).unwrap();
    let new = para.text.replacen("By noon", "By early noon", 1);
    let set = setting.set(&typed(&para, &new));
    assert!(set.lines.iter().all(|l| l.stretch == 0.0), "{set:?}");
    // The first line spread to 320 points by word spacing, the last as set: justified.
    let gaps = lines[0].matches(' ').count() as f32;
    let tw = (320.0 - text_width(&lines[0], 11.0)) / gaps;
    let content = format!(
        "BT /F1 11 Tf {tw} Tw 72 700 Td ({}) Tj ET\nBT 0 Tw 72 686.8 Td ({}) Tj ET\n",
        lines[0], lines[1]
    );
    let p = letter(&content);
    assert_eq!(only_paragraph(&p).frame.align, Align::Justified);
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

/// Three lines in TeX's bitmap font, the second reading on from the first.
fn tex_paragraph() -> Pdf {
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
    page_with(|p| tex_bitmap_font(p, &letters), &content)
}

#[test]
fn a_paragraph_in_a_flipped_type3_font_is_found_and_confident() {
    // Upright and whole (PB-56); new text in the font goes to another face, but an edit
    // keeps the paragraph's own letters, so the font doesn't make it uncertain.
    let p = tex_paragraph();
    let blocks = p.page_blocks(0).unwrap();
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    assert_eq!(blocks[0].lines.len(), 3);
    assert!(blocks[0].confident(), "{:?}", blocks[0].doubt);
    assert!(
        (blocks[0].frame.spacing - 14.0).abs() < 0.05,
        "{:?}",
        blocks[0].frame
    );
}

#[test]
fn a_paragraph_in_a_flipped_type3_font_keeps_its_letters_upright_across_its_lines() {
    let mut p = tex_paragraph();
    let para = only_paragraph(&p);
    let setting = p.paragraph_setting(&para).unwrap();
    // A word typed (in another font: the bitmap font takes no new text), and letters moved
    // to other lines by the rewrap.
    let new = para.text.replacen("ton tony", "ton big tony", 1);
    p.edit_paragraph(&setting, &typed(&para, &new)).unwrap();
    let again = only_paragraph(&p);
    assert_eq!(again.text, new);
    // Every line upright, its own letters drawn the right way up: nothing below the lines.
    assert!(lines(&p).iter().all(|o| o.upright), "{:?}", lines(&p));
    let rows = dark_rows(&mut p, 60..140);
    assert!(rows.iter().any(|&n| n > 0), "nothing drawn: {rows:?}");
    let last = rows.iter().rposition(|&n| n > 0).unwrap();
    assert!(last < 60, "drawn below the paragraph: {rows:?}");
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

// ---- A block read for editing as a paragraph (`Pdf::paragraph`).

/// Page 0's only block of `p` read as a paragraph, with the document's words.
fn only_paragraph(p: &Pdf) -> Paragraph {
    let blocks = p.page_blocks(0).unwrap();
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    p.paragraph(0, &blocks[0], &p.document_words()).unwrap()
}

#[test]
fn a_paragraphs_lines_are_read_as_one_text() {
    let lines = wrap(TEXT, 440.0, 11.0);
    let p = letter(&lines_at(&lines, 72.0, 0.0, 700.0, 13.2, "F1", 11.0));
    let para = only_paragraph(&p);
    assert_eq!(para.text, lines.join(" "));
    // Each line's characters, the space joining two in neither.
    let mut at = 0;
    for (l, &(a, b)) in lines.iter().zip(&para.lines) {
        assert_eq!((a, b), (at, at + l.chars().count()));
        at = b + 1;
    }
    // A part for each line's text object, the space after it in it.
    assert_eq!(para.spans.len(), lines.len());
    assert_eq!(para.spans[0].chars, (0, para.lines[0].1 + 1));
    assert_eq!(para.spans[0].font_name, "Helvetica");
    assert_eq!(para.source(0), Some((0, 0)));
    assert_eq!(para.source(para.lines[0].1), None);
    assert_eq!(para.source(para.lines[1].0), Some((1, 0)));
    assert!(para.hyphens.is_empty());
}

#[test]
fn pieces_of_a_line_join_with_a_space_only_where_they_stand_a_word_apart() {
    // The first line's words each in a text object of their own, the second's last word cut
    // in two across two text objects, as Word draws some.
    let lines = wrap(TEXT, 440.0, 11.0);
    let mut content = String::new();
    for (k, l) in lines.iter().enumerate() {
        let y = 700.0 - 13.2 * k as f32;
        match k {
            0 => {
                let mut x = 72.0;
                for word in l.split(' ') {
                    content.push_str(&format!("BT /F1 11 Tf {x} {y} Td ({word}) Tj ET\n"));
                    x += text_width(&format!("{word} "), 11.0);
                }
            }
            1 => {
                let cut = l.len() - 3;
                let (a, b) = l.split_at(cut);
                let x = 72.0 + text_width(a, 11.0);
                content.push_str(&format!("BT /F1 11 Tf 72 {y} Td ({a}) Tj ET\n"));
                content.push_str(&format!("BT /F1 11 Tf {x} {y} Td ({b}) Tj ET\n"));
            }
            _ => content.push_str(&format!("BT /F1 11 Tf 72 {y} Td ({l}) Tj ET\n")),
        }
    }
    let p = letter(&content);
    let para = only_paragraph(&p);
    assert_eq!(para.text, lines.join(" "));
    // A part for each of the first line's words, the second's two pieces and each other line.
    let first = lines[0].split(' ').count();
    assert_eq!(para.spans.len(), first + 2 + lines.len() - 2);
}

#[test]
fn line_end_hyphens_are_read_by_the_documents_own_words() {
    // "exam-ple" broken where the document also has "example" whole; "well-known" where it
    // also has "well-known"; "co-op" where it has neither.
    let lines = [
        "The harbour master gave one exam-",
        "ple of a well-",
        "known catch, a co-",
        "op of the fishing boats in the bay.",
    ];
    let lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    let mut content = lines_at(&lines, 72.0, 0.0, 700.0, 13.2, "F1", 11.0);
    content.push_str("BT /F1 11 Tf 72 400 Td (An example of a well-known name.) Tj ET\n");
    let p = letter(&content);
    let blocks = p.page_blocks(0).unwrap();
    let b = block_from(&p, &blocks, "The harbour");
    let para = p.paragraph(0, b, &p.document_words()).unwrap();
    assert_eq!(
        para.text,
        "The harbour master gave one exam\u{AD}ple of a well-known catch, a co-op of the \
         fishing boats in the bay."
    );
    let kinds: Vec<Hyphen> = para.hyphens.iter().map(|h| h.kind).collect();
    use Hyphen::{Discretionary, Hard, Unknown};
    assert_eq!(kinds, [Discretionary, Hard, Unknown]);
    let chars: Vec<char> = para.text.chars().collect();
    let at: Vec<char> = para.hyphens.iter().map(|h| chars[h.at]).collect();
    assert_eq!(at, ['\u{AD}', '-', '-']);
    // The hyphen is still the line's own character, drawn where it was.
    assert_eq!(para.source(para.hyphens[0].at), Some((0, 32)));
}

#[test]
fn a_raised_piece_keeps_its_rise_from_its_lines_baseline() {
    let lines = wrap(TEXT, 440.0, 11.0);
    let mut content = lines_at(&lines, 72.0, 0.0, 700.0, 13.2, "F1", 11.0);
    // A footnote mark after the first line's end, smaller and raised.
    let x = 72.0 + text_width(&lines[0], 11.0) + 1.0;
    content.push_str(&format!("BT /F1 7 Tf {x} 704 Td (1) Tj ET\n"));
    let p = letter(&content);
    let para = only_paragraph(&p);
    let k = para.lines[0].1 - 1;
    assert_eq!(para.text.chars().nth(k), Some('1'));
    let s = &para.spans[para.span_at(k).unwrap()];
    assert!((s.rise - 4.0).abs() < 0.1, "{s:?}");
    assert!((s.font_size - 7.0).abs() < 0.01, "{s:?}");
}

// ---- Rewrapping an edited paragraph (`ParagraphSetting::set`).

#[test]
fn a_rewrap_hides_a_discretionary_hyphen_inside_a_line_and_marks_an_unknown_one() {
    let lines = [
        "The harbour master gave one exam-",
        "ple to the crews of the boats, a co-",
        "op of the fishing boats in the bay.",
    ];
    let lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    let mut content = lines_at(&lines, 72.0, 0.0, 700.0, 13.2, "F1", 11.0);
    content.push_str("BT /F1 11 Tf 72 400 Td (An example of the harbour master's.) Tj ET\n");
    let p = letter(&content);
    let blocks = p.page_blocks(0).unwrap();
    let b = block_from(&p, &blocks, "The harbour");
    let para = p.paragraph(0, b, &p.document_words()).unwrap();
    let kinds: Vec<Hyphen> = para.hyphens.iter().map(|h| h.kind).collect();
    assert_eq!(kinds, [Hyphen::Discretionary, Hyphen::Unknown]);
    let setting = p.paragraph_setting(&para).unwrap();
    // As it is, the first line shows its hyphen.
    assert!(setting.set(&para.parts()).lines[0].hyphen);
    let new = para.text.replacen("The harbour master ", "", 1);
    let set = setting.set(&typed(&para, &new));
    let chars: Vec<char> = new.chars().collect();
    let soft = chars.iter().position(|&c| c == '\u{AD}').unwrap();
    let first = &set.lines[0];
    assert!(first.chars.0 <= soft && soft + 1 < first.chars.1, "{set:?}");
    assert!(!first.hyphen);
    // The hyphen of "co-op" is marked where it is inside a line, not where it ends one.
    let co = new[..new.find("co-op").unwrap()].chars().count() + 2;
    let line = (set.lines.iter())
        .find(|l| l.chars.0 <= co && co < l.chars.1)
        .unwrap();
    let ends = set_texts(&set, &new)[set.lines.iter().position(|l| l == line).unwrap()]
        .chars()
        .count();
    match co + 1 < line.chars.0 + ends {
        true => assert_eq!(set.marked, [co]),
        false => assert!(set.marked.is_empty()),
    }
}

/// The paragraph's text changed to `new` as typing changes it: the characters it starts and
/// ends with unchanged kept where they were, in their parts' styles; those typed take the
/// style of the character before them.
fn typed(para: &Paragraph, new: &str) -> Vec<TextPart> {
    let (old, new): (Vec<char>, Vec<char>) = (para.text.chars().collect(), new.chars().collect());
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - head;
    let tail = (old.iter().rev().zip(new.iter().rev()))
        .take_while(|(a, b)| a == b)
        .count()
        .min(room);
    let mut parts: Vec<TextPart> = Vec::new();
    let mut span = 0;
    for (j, &c) in new.iter().enumerate() {
        let origin = match j {
            _ if j < head => Some(j),
            _ if j >= new.len() - tail => Some(old.len() + j - new.len()),
            _ => None,
        };
        if let Some(k) = origin {
            span = para.span_at(k).unwrap();
        }
        let s = &para.spans[span];
        match parts.last_mut() {
            Some(p) if p.font == PartFont::Span(span) => {
                p.text.push(c);
                p.origins.push(origin);
            }
            _ => parts.push(TextPart {
                text: c.to_string(),
                font: PartFont::Span(span),
                size: s.font_size,
                color: s.fill.unwrap_or([0.0; 3]),
                rise: s.rise,
                from: None,
                origins: vec![origin],
            }),
        }
    }
    parts
}

/// The text of each of `set`'s lines in `text`, the spaces they end with left out.
fn set_texts(set: &SetParagraph, text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let text = |l: &SetLine| chars[l.chars.0..l.chars.1].iter().collect::<String>();
    (set.lines.iter())
        .map(|l| text(l).trim_end().to_string())
        .collect()
}

/// A left-aligned paragraph of `TEXT` and `MORE` broken greedily at `room` points, as one
/// block, and its setting.
fn ragged(room: f32) -> (Pdf, Vec<String>, ParagraphSetting) {
    let lines = wrap(&format!("{TEXT} {MORE}"), room, 11.0);
    let p = letter(&lines_at(&lines, 72.0, 0.0, 700.0, 13.2, "F1", 11.0));
    let para = only_paragraph(&p);
    let setting = p.paragraph_setting(&para).unwrap();
    (p, lines, setting)
}

#[test]
fn a_paragraph_set_unchanged_keeps_every_line() {
    let (_, lines, setting) = ragged(300.0);
    let para = setting.paragraph();
    let set = setting.set(&para.parts());
    assert!(set.unchanged());
    assert_eq!(set.lines.len(), lines.len());
    assert_eq!(set_texts(&set, &para.text), lines);
    // Typing that puts the text back as it was is no change either.
    let set = setting.set(&typed(para, &para.text.clone()));
    assert!(set.unchanged());
}

#[test]
fn a_letter_changed_rewrites_its_line_alone() {
    let (_, lines, setting) = ragged(300.0);
    let para = setting.paragraph();
    // An "e" in the third line made an "o", as wide in Helvetica.
    let (a, b) = para.lines[2];
    let chars: Vec<char> = para.text.chars().collect();
    let k = (a..b).find(|&k| chars[k] == 'e').unwrap();
    let mut new = chars.clone();
    new[k] = 'o';
    let new: String = new.into_iter().collect();
    let set = setting.set(&typed(para, &new));
    let kept: Vec<Option<usize>> = set.lines.iter().map(|l| l.kept).collect();
    let mut want: Vec<Option<usize>> = (0..lines.len()).map(Some).collect();
    want[2] = None;
    assert_eq!(kept, want);
    let mut texts = lines.clone();
    texts[2] = set_texts(&set, &new)[2].clone();
    assert_eq!(set_texts(&set, &new), texts);
    // Where the line was: 72 points in, its baseline as it was down the page.
    let l = &set.lines[2];
    let down = 792.0 - (700.0 - 13.2 * 2.0);
    assert!(near((l.x, l.baseline), (72.0, down)), "{l:?}");
}

#[test]
fn an_edit_is_written_in_stretches_kept_in_the_paragraphs_font_or_sent_to_another() {
    let (_, _, setting) = ragged(300.0);
    let para = setting.paragraph();
    // A word unembedded Helvetica can't write (it writes plain ASCII as itself).
    let new = para.text.replacen("quiet", "quiet Ωmega", 1);
    let parts = typed(para, &new);
    let chars: Vec<char> = new.chars().collect();
    let (mut at, mut away) = (0, String::new());
    for p in setting.pieces(&parts) {
        let text: Vec<char> = parts[p.part].text.chars().collect();
        let piece = &text[p.chars.0..p.chars.1];
        assert_eq!(piece, &chars[at..at + piece.len()], "{p:?}");
        at += piece.len();
        // Kept in its line's own part (a part a text object here).
        match p.kept {
            Some(i) => assert_eq!(parts[p.part].font, PartFont::Span(i), "{p:?}"),
            None => away.extend(piece),
        }
    }
    assert_eq!(at, chars.len());
    assert_eq!(away.trim(), "Ωmega");
    assert!(setting.part_font(0).is_some_and(|f| f.takes_new_text()));
    assert!(setting.part_font(para.spans.len()).is_none());
}

#[test]
fn the_documents_words_are_the_same_on_fewer_threads() {
    let (p, _, _) = ragged(300.0);
    assert_eq!(p.document_words_on(2), p.document_words());
    assert!(!p.document_words_on(1).is_empty());
}

/// Whether two points on the page are the same to within 0.05 points.
fn near(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < 0.05 && (a.1 - b.1).abs() < 0.05
}

#[test]
fn a_word_deleted_pulls_the_next_up_only_where_it_fits_now_and_didnt_before() {
    let room = 300.0;
    let (_, lines, setting) = ragged(room);
    let para = setting.paragraph();
    // A line whose next line's second word would fit on it once the first is deleted.
    let words: Vec<Vec<&str>> = lines.iter().map(|l| l.split(' ').collect()).collect();
    let takes = |k: usize| {
        let longer = format!("{} {}", lines[k], words[k + 1][1]);
        words[k + 1].len() > 2 && text_width(&longer, 11.0) <= room - 1.0
    };
    let k = (0..lines.len() - 1)
        .find(|&k| takes(k))
        .expect("a line that can take a word");
    let mut new_lines = lines.clone();
    new_lines[k + 1] = words[k + 1][1..].join(" ");
    let new = new_lines.join(" ");
    let set = setting.set(&typed(para, &new));
    let texts = set_texts(&set, &new);
    // The line before takes the word, and the lines after it are set again.
    assert_eq!(texts[k], format!("{} {}", lines[k], words[k + 1][1]));
    assert_eq!(set.lines[k].kept, None);
    for n in 0..k {
        assert_eq!(set.lines[n].kept, Some(n));
    }
    for l in &set.lines {
        assert!(l.width <= room + 0.01, "{l:?}");
    }
    assert_eq!(texts.join(" "), new);
}

#[test]
fn words_typed_into_a_line_rewrap_the_lines_after_it_within_the_frame() {
    let room = 300.0;
    let (_, lines, setting) = ragged(room);
    let para = setting.paragraph();
    let right = para.frame.right - para.frame.left;
    // Words typed after the second line's first word.
    let (a, _) = para.lines[1];
    let chars: Vec<char> = para.text.chars().collect();
    let at = a + chars[a..].iter().position(|&c| c == ' ').unwrap();
    let mut new: String = chars[..at].iter().collect();
    new.push_str(" very slowly and rather quietly");
    new.extend(&chars[at..]);
    let set = setting.set(&typed(para, &new));
    let texts = set_texts(&set, &new);
    assert_eq!(texts.join(" "), new);
    assert!(set.lines.len() >= lines.len());
    assert_eq!(set.lines[0].kept, Some(0));
    for l in &set.lines {
        assert!(l.width <= right + 0.01, "{l:?} in {right}");
        assert!((l.x - 72.0).abs() < 0.01, "{l:?}");
    }
    // Each line a line spacing below the one before.
    for w in set.lines.windows(2) {
        assert!((w[1].baseline - w[0].baseline - 13.2).abs() < 0.05, "{w:?}");
    }
}

#[test]
fn a_justified_line_set_again_reaches_the_right_edge() {
    // Justified as `justified_text_has_a_justified_frame` sets it: each line but the last
    // stretched by word spacing to 440 points.
    let lines = wrap(TEXT, 440.0, 11.0);
    let mut content = String::new();
    for (k, l) in lines.iter().enumerate() {
        let y = 700.0 - 13.2 * k as f32;
        let gaps = l.matches(' ').count() as f32;
        let tw = match k + 1 < lines.len() {
            true => (440.0 - text_width(l, 11.0)) / gaps,
            false => 0.0,
        };
        content.push_str(&format!("BT /F1 11 Tf {tw} Tw 72 {y} Td ({l}) Tj ET\n"));
    }
    let p = letter(&content);
    let para = only_paragraph(&p);
    assert_eq!(para.frame.align, Align::Justified);
    let setting = p.paragraph_setting(&para).unwrap();
    let new = para.text.replacen("quiet", "still", 1);
    let set = setting.set(&typed(&para, &new));
    let chars: Vec<char> = new.chars().collect();
    for l in &set.lines[..set.lines.len() - 1] {
        let line: String = chars[l.chars.0..l.chars.1].iter().collect();
        let gaps = line.trim_end().matches(' ').count() as f32;
        let end = l.x + l.width + l.stretch * gaps;
        if l.kept.is_none() {
            assert!((end - (72.0 + 440.0)).abs() < 0.5, "{l:?} ends at {end}");
        }
    }
    assert_eq!(set.lines.last().unwrap().stretch, 0.0);
}

// ---- Writing an edited paragraph in place (`Pdf::edit_paragraph`).

/// Page 0's content operators.
fn ops(p: &Pdf) -> Vec<lopdf::content::Operation> {
    let page = p.page_ids()[0];
    lopdf::content::Content::decode(&p.doc.get_page_content(page))
        .unwrap()
        .operations
}

/// Page 0's text objects, `BT` to `ET`, each as its content bytes.
fn text_objects(p: &Pdf) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<lopdf::content::Operation>> = Vec::new();
    let mut inside = false;
    for o in ops(p) {
        match o.operator.as_str() {
            "BT" => {
                inside = true;
                out.push(vec![o]);
            }
            "ET" => {
                inside = false;
                out.last_mut().unwrap().push(o);
            }
            _ if inside => out.last_mut().unwrap().push(o),
            _ => {}
        }
    }
    (out.into_iter())
        .map(|operations| lopdf::content::Content { operations }.encode().unwrap())
        .collect()
}

/// The document as saved, for comparing.
fn saved(p: &Pdf) -> Vec<u8> {
    let mut copy = p.clone();
    copy.doc.max_id = copy.doc.objects.keys().map(|id| id.0).max().unwrap_or(0);
    copy.snapshot().unwrap()
}

/// Page 0's text lines' texts, in content order.
fn line_texts(p: &Pdf) -> Vec<String> {
    lines(p).into_iter().filter_map(|o| o.text).collect()
}

#[test]
fn a_paragraph_kept_as_it_is_is_not_written() {
    let (mut p, _, setting) = ragged(300.0);
    let was = saved(&p);
    let parts = setting.paragraph().parts();
    let lost = p.edit_paragraph(&setting, &parts).unwrap();
    assert!(lost.is_empty());
    assert!(saved(&p) == was, "the document changed");
}

#[test]
fn a_letter_changed_is_written_in_its_lines_place_the_other_lines_byte_for_byte() {
    let (mut p, lines, setting) = ragged(300.0);
    let before = text_objects(&p);
    let para = setting.paragraph();
    let (a, b) = para.lines[2];
    let chars: Vec<char> = para.text.chars().collect();
    let k = (a..b).find(|&k| chars[k] == 'e').unwrap();
    let mut new = chars.clone();
    new[k] = 'o';
    let new: String = new.into_iter().collect();
    p.edit_paragraph(&setting, &typed(para, &new)).unwrap();
    let after = text_objects(&p);
    assert_eq!(after.len(), before.len(), "a text object added or lost");
    for (n, (x, y)) in before.iter().zip(&after).enumerate() {
        if n != 2 {
            assert!(x == y, "line {n} changed");
        }
    }
    let mut want = lines.clone();
    want[2] = new.chars().skip(a).take(b - a).collect();
    assert_eq!(line_texts(&p), want);
    // Where it was: the third line's baseline, 72 points in.
    let third = &lines_of(&p)[2];
    let at = third.baseline.unwrap();
    assert!(near(at, (72.0, 792.0 - 673.6)), "{third:?}");
    // Its letters still the line's font, Helvetica, nothing added to the page's fonts.
    assert_eq!(third.font_name.as_deref(), Some("Helvetica"));
}

/// Page 0's text lines.
fn lines_of(p: &Pdf) -> Vec<PageObject> {
    lines(p)
}

#[test]
fn words_typed_are_written_in_the_old_lines_places_and_read_back_as_typed() {
    let (mut p, lines, setting) = ragged(300.0);
    let objects_before = text_objects(&p).len();
    let ops_before = ops(&p).len();
    let para = setting.paragraph().clone();
    let (a, _) = para.lines[1];
    let chars: Vec<char> = para.text.chars().collect();
    let at = a + chars[a..].iter().position(|&c| c == ' ').unwrap();
    let mut new: String = chars[..at].iter().collect();
    new.push_str(" very slowly and rather quietly");
    new.extend(&chars[at..]);
    let set = setting.set(&typed(&para, &new));
    let lost = p.edit_paragraph(&setting, &typed(&para, &new)).unwrap();
    assert!(lost.is_empty(), "{lost:?}");
    // No text object added: the lines added are in the last one's.
    assert_eq!(text_objects(&p).len(), objects_before);
    let after = ops(&p);
    assert!(after.len() > ops_before);
    assert_eq!(after.last().unwrap().operator, "ET");
    // The page reads as typed, line by line as set.
    let texts = line_texts(&p);
    assert_eq!(texts.join(" "), new, "{texts:?}");
    assert_eq!(texts, set_texts(&set, &new));
    assert_eq!(texts[0], lines[0]);
    // Each line on its baseline, the first two where they were.
    let objs = lines_of(&p);
    let right = para.frame.right;
    for (o, l) in objs.iter().zip(&set.lines) {
        let at = o.baseline.unwrap();
        assert!(near(at, (l.x, l.baseline)), "{o:?} against {l:?}");
        assert!(o.bbox[2] <= right + 0.5, "{o:?} past {right}");
    }
    // Found again as one paragraph, its text as typed.
    let again = only_paragraph(&p);
    assert_eq!(again.text, new);
}

#[test]
fn a_paragraph_made_shorter_takes_out_the_lines_left_over_and_leaves_their_space() {
    let (mut p, lines, setting) = ragged(300.0);
    let objects_before = text_objects(&p).len();
    let para = setting.paragraph().clone();
    // The second and third lines' words deleted.
    let (a, _) = para.lines[1];
    let (b, _) = para.lines[3];
    let chars: Vec<char> = para.text.chars().collect();
    let new: String = chars[..a].iter().chain(&chars[b..]).collect();
    let set = setting.set(&typed(&para, &new));
    assert_eq!(set.lines.len(), lines.len() - 2, "{set:?}");
    p.edit_paragraph(&setting, &typed(&para, &new)).unwrap();
    // The text objects stay, the last two with nothing shown in them.
    assert_eq!(text_objects(&p).len(), objects_before);
    assert_eq!(line_texts(&p), set_texts(&set, &new));
    let objs = lines_of(&p);
    let last = objs.last().unwrap().baseline.unwrap().1;
    let was_last = 792.0 - (700.0 - 13.2 * (lines.len() - 3) as f32);
    assert!((last - was_last).abs() < 0.05, "{last} against {was_last}");
}

#[test]
fn one_undo_puts_an_edited_paragraph_back_byte_for_byte() {
    let (mut p, _, setting) = ragged(300.0);
    let para = setting.paragraph().clone();
    let new = para.text.replacen("harbour", "small harbour", 1);
    let was = saved(&p);
    let before = p.before();
    p.edit_paragraph(&setting, &typed(&para, &new)).unwrap();
    assert!(saved(&p) != was);
    let undo = before.undo(&p);
    p.restore(undo);
    assert!(saved(&p) == was, "Undo didn't put the document back");
}

#[test]
fn an_edited_paragraph_saved_and_reopened_is_found_again_as_typed() {
    let (mut p, _, setting) = ragged(300.0);
    let para = setting.paragraph().clone();
    let new = (para.text).replacen("the old men", "the old women and men", 1);
    p.edit_paragraph(&setting, &typed(&para, &new)).unwrap();
    let bytes = p.to_bytes().unwrap();
    let mut q = Pdf::from_bytes(&bytes, None).unwrap();
    let again = only_paragraph(&q);
    assert_eq!(again.text, new);
    // And edited again.
    let setting = q.paragraph_setting(&again).unwrap();
    let newer = again.text.replacen("women and ", "", 1);
    q.edit_paragraph(&setting, &typed(&again, &newer)).unwrap();
    assert_eq!(only_paragraph(&q).text, newer);
}

/// `p`'s paragraph starting "The harbour" edited by typing `word` after the second line's
/// first word: the paragraph, the edit's text, and how it is set.
fn type_into_second_line(p: &mut Pdf, word: &str) -> (Paragraph, String, SetParagraph) {
    let blocks = p.page_blocks(0).unwrap();
    let block = block_from(p, &blocks, "The harbour");
    let para = p.paragraph(0, block, &p.document_words()).unwrap();
    let setting = p.paragraph_setting(&para).unwrap();
    let (a, _) = para.lines[1];
    let chars: Vec<char> = para.text.chars().collect();
    let at = a + chars[a..].iter().position(|&c| c == ' ').unwrap();
    let mut new: String = chars[..at].iter().collect();
    new.push(' ');
    new.push_str(word);
    new.extend(&chars[at..]);
    let set = setting.set(&typed(&para, &new));
    let lost = p.edit_paragraph(&setting, &typed(&para, &new)).unwrap();
    assert!(lost.is_empty(), "{lost:?}");
    (para, new, set)
}

/// The text of page 0's lines in the paragraph starting "The harbour", in content order.
fn paragraph_texts(p: &Pdf) -> Vec<String> {
    let blocks = p.page_blocks(0).unwrap();
    let b = block_from(p, &blocks, "The harbour");
    let objs = p.page_objects(0).unwrap();
    (b.lines.iter())
        .map(|l| {
            (l.pieces.iter())
                .map(|&id| objs[id].text.clone().unwrap())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect()
}

/// `TEXT` broken at 300 points in one text object, a line move between lines and ' for some,
/// and a line after the paragraph placed from where its last line is.
fn one_object_page() -> Pdf {
    let lines = wrap(TEXT, 300.0, 11.0);
    let mut content = String::from("BT /F1 11 Tf 13.2 TL 72 700 Td\n");
    for (k, l) in lines.iter().enumerate() {
        content.push_str(&match (k, k % 2) {
            (0, _) => format!("({l}) Tj\n"),
            (_, 0) => format!("({l}) '\n"),
            _ => format!("T* ({l}) Tj\n"),
        });
    }
    content.push_str("0 -40 Td (After the paragraph.) Tj ET\n");
    letter(&content)
}

#[test]
fn lines_in_one_text_object_are_written_in_it_and_what_follows_stays_put() {
    let mut p = one_object_page();
    let after = |p: &Pdf| {
        let after = |o: &PageObject| o.text.as_deref() == Some("After the paragraph.");
        let line = lines_of(p).into_iter().find(after).unwrap();
        line.baseline.unwrap()
    };
    let was = after(&p);
    let (_, new, set) = type_into_second_line(&mut p, "very slowly and rather quietly");
    assert_eq!(text_objects(&p).len(), 1);
    assert_eq!(paragraph_texts(&p), set_texts(&set, &new));
    let now = after(&p);
    assert!(near(now, was), "{was:?} to {now:?}");
}

/// The text each marked content of page 0 shows, by its MCID, in Helvetica's single bytes.
fn marked_texts(p: &Pdf) -> Vec<(i64, String)> {
    let mut out: Vec<(i64, String)> = Vec::new();
    let mut mcid: Option<i64> = None;
    for o in ops(p) {
        match o.operator.as_str() {
            "BDC" => {
                let d = o.operands.get(1).and_then(|d| d.as_dict().ok());
                let id = d.and_then(|d| d.get(b"MCID").ok());
                mcid = id.and_then(|m| m.as_i64().ok());
                out.extend(mcid.map(|m| (m, String::new())));
            }
            "EMC" => mcid = None,
            "Tj" | "TJ" | "'" if mcid.is_some() => {
                let items = match o.operator.as_str() {
                    "TJ" => o.operands[0].as_array().unwrap().clone(),
                    _ => o.operands.clone(),
                };
                let text = &mut out.last_mut().unwrap().1;
                for it in items {
                    if let Object::String(b, _) = it {
                        text.extend(b.iter().map(|&c| c as char));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[test]
fn lines_tagged_one_by_one_keep_each_written_line_in_its_tag() {
    let lines = wrap(&format!("{TEXT} {MORE}"), 300.0, 11.0);
    let mut p = tagged(&lines, &vec![0; lines.len()]);
    let (para, new, set) = type_into_second_line(&mut p, "very slowly and rather quietly");
    assert!(set.lines.len() > para.lines.len(), "no line added");
    let chars: Vec<char> = new.chars().collect();
    let marked = marked_texts(&p);
    assert_eq!(marked.len(), lines.len(), "a tag added or lost");
    // Each line in its old line's tag, the lines added in the last one's.
    let rows = para.lines.len();
    for (k, (mcid, text)) in marked.iter().enumerate() {
        assert_eq!(*mcid, k as i64);
        let ls = match k + 1 == rows {
            true => &set.lines[k..],
            false => &set.lines[k..k + 1],
        };
        let line = |l: &SetLine| chars[l.chars.0..l.chars.1].iter().collect::<String>();
        let want: String = ls.iter().map(|l| line(l).trim_end().to_string()).collect();
        assert_eq!(text.replace(' ', ""), want.replace(' ', ""), "tag {k}");
    }
}

#[test]
fn a_paragraph_tagged_as_one_is_written_inside_its_tag() {
    let lines = wrap(&format!("{TEXT} {MORE}"), 300.0, 11.0);
    let mut content = String::from("/P <</MCID 0>> BDC\n");
    content.push_str(&lines_at(&lines, 72.0, 0.0, 700.0, 13.2, "F1", 11.0));
    content.push_str("EMC\n");
    let mut p = letter(&content);
    add_tree(&mut p, &[0]);
    let (_, new, _) = type_into_second_line(&mut p, "very slowly and rather quietly");
    let marked = marked_texts(&p);
    assert_eq!(marked.len(), 1);
    assert_eq!(marked[0].1.replace(' ', ""), new.replace(' ', ""));
    assert_eq!(ops(&p).last().unwrap().operator, "EMC");
}

/// `TEXT` and `MORE` drawn with 0.5 points between letters, broken at 300 points as drawn.
fn spaced_page() -> Pdf {
    let tc = 0.5;
    let mut lines: Vec<String> = Vec::new();
    for word in format!("{TEXT} {MORE}").split(' ') {
        let width = |s: &str| text_width(s, 11.0) + tc * s.chars().count() as f32;
        match lines.last_mut() {
            Some(l) if width(&format!("{l} {word}")) <= 300.0 => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    let mut content = String::new();
    for (k, l) in lines.iter().enumerate() {
        let y = 700.0 - 13.2 * k as f32;
        content.push_str(&format!("BT /F1 11 Tf {tc} Tc 72 {y} Td ({l}) Tj ET\n"));
    }
    letter(&content)
}

#[test]
fn a_paragraphs_character_spacing_is_kept_on_its_rewritten_lines() {
    let mut p = spaced_page();
    let right = only_paragraph(&p).frame.right;
    let (_, new, set) = type_into_second_line(&mut p, "very slowly and rather quietly");
    assert_eq!(paragraph_texts(&p), set_texts(&set, &new));
    // Each rewritten line as wide as it was set: its letters still 0.5 points apart.
    let objs = lines_of(&p);
    for (o, l) in objs.iter().zip(&set.lines) {
        assert!(o.bbox[2] <= right + 0.5, "{o:?} past {right}");
        let (drawn, set) = (o.bbox[2] - o.bbox[0], l.width);
        assert!((drawn - set).abs() < 0.6, "{drawn} drawn, {set} set: {o:?}");
    }
}

/// `TEXT` with "Prüfung" in it, broken at 300 points, its "ü" drawn as TeX draws an accented
/// letter (pdf.js `issue1127`, PB-73): the diaeresis alone (WinAnsi's ¨), moved over where
/// the "u" goes and a little lower, then the "u" and the rest of the line from a move back.
fn accented_page() -> Pdf {
    let text = TEXT.replacen("worrying", "Prüfung", 1);
    let lines = wrap(&text, 300.0, 11.0);
    let mut content = String::new();
    for (k, l) in lines.iter().enumerate() {
        let y = 700.0 - 13.2 * k as f32;
        let Some(at) = l.find('ü') else {
            content.push_str(&format!("BT /F1 11 Tf 72 {y} Td ({l}) Tj ET\n"));
            continue;
        };
        let (head, tail) = (&l[..at], &l[at + 'ü'.len_utf8()..]);
        // Helvetica's u is 556 wide and its ¨ 333: the accent centred over the u.
        let u = text_width(head, 11.0);
        let over = (556.0 - 333.0) / 2.0 * 11.0 / 1000.0;
        content.push_str(&format!(
            "BT /F1 11 Tf 72 {y} Td ({head}) Tj {} -0.4 Td (\\250) Tj {} 0.4 Td (u{tail}) Tj ET\n",
            u + over,
            -over
        ));
    }
    letter(&content)
}

/// How far the "u" of the accented line starts from its accent ([`accented_page`]), as the
/// page draws them: left of it, under it. The parser reads them as two lines when the "u"
/// is drawn from a move back too far to carry on from the accent, as `issue1127`'s are, and
/// as one when the move is in the accent's show.
fn accent_to_letter(p: &Pdf) -> f32 {
    // Each character of the page's lines, in content order, and where it starts across the
    // page (along each line from its baseline's start).
    let drawn: Vec<(char, f32)> = (lines(p).iter())
        .flat_map(|o| {
            let x = o.baseline.unwrap().0;
            let stops: Vec<f32> = o.stops.iter().map(|s| x + s).collect();
            let text = o.text.as_deref().unwrap_or("");
            text.chars().zip(stops).collect::<Vec<_>>()
        })
        .collect();
    let k = (drawn.iter().position(|d| d.0 == '\u{A8}')).expect("the accent");
    assert_eq!(drawn[k + 1].0, 'u', "{drawn:?}");
    drawn[k + 1].1 - drawn[k].1
}

#[test]
fn a_letter_drawn_as_its_accent_and_base_keeps_the_accent_over_it_when_its_line_is_rewritten() {
    let p = accented_page();
    let before = accent_to_letter(&p);
    assert!((before + 1.23).abs() < 0.05, "{before}");
    // A word typed before it: its line written again.
    let (p, new) = edited_to(p, |para| {
        para.text.replacen("Pr\u{A8}ufung", "big Pr\u{A8}ufung", 1)
    });
    assert!(new.contains("big Pr\u{A8}ufung"), "{new:?}");
    let after = accent_to_letter(&p);
    assert!((after - before).abs() < 0.05, "{after}, was {before}");
}

/// The accented letter of a rewritten line ([`accented_page`]) reads to Poppler as it did
/// before: the letter, then the accent as its combining mark (PB-73: after a rewrite,
/// `issue1127`'s read the mark first). Opt-in, as the cross-check above.
#[test]
#[ignore]
fn cross_check_an_accent_drawn_over_its_letter_reads_with_it_in_poppler() {
    let mut p = accented_page();
    let was = plain(&tools::pdftotext(&p.to_bytes().unwrap()));
    assert!(was.contains("Pru\u{308}fung"), "{was:?}");
    let (mut p, _) = edited_to(p, |para| {
        para.text.replacen("Pr\u{A8}ufung", "big Pr\u{A8}ufung", 1)
    });
    let got = plain(&tools::pdftotext(&p.to_bytes().unwrap()));
    assert!(got.contains("big Pru\u{308}fung"), "{got:?}");
}

/// A page turned a quarter for showing, `TEXT` and `MORE` drawn up the page so that they
/// read across when shown.
fn turned_page() -> Pdf {
    let lines = wrap(&format!("{TEXT} {MORE}"), 300.0, 11.0);
    let mut content = String::new();
    for (k, l) in lines.iter().enumerate() {
        let x = 100.0 + 13.2 * k as f32;
        content.push_str(&format!("BT /F1 11 Tf 0 1 -1 0 {x} 72 Tm ({l}) Tj ET\n"));
    }
    let mut p = letter(&content);
    let page = p.page_ids()[0];
    p.doc.get_dictionary_mut(page).unwrap().set("Rotate", 90);
    p
}

#[test]
fn a_paragraph_on_a_turned_page_is_written_where_it_shows() {
    let mut p = turned_page();
    let (_, new, set) = type_into_second_line(&mut p, "very slowly and rather quietly");
    assert_eq!(paragraph_texts(&p), set_texts(&set, &new));
    for (o, l) in lines_of(&p).iter().zip(&set.lines) {
        assert!(o.upright, "{o:?}");
        let at = o.baseline.unwrap();
        assert!(near(at, (l.x, l.baseline)), "{o:?} against {l:?}");
    }
}

/// `TEXT` and `MORE` justified to 440 points by word spacing, every line but the last, the
/// first indented 18 points.
fn indented_justified() -> Pdf {
    let lines = wrap(&format!("{TEXT} {MORE}"), 422.0, 11.0);
    let mut content = String::new();
    for (k, l) in lines.iter().enumerate() {
        let y = 700.0 - 13.2 * k as f32;
        let x = if k == 0 { 90.0 } else { 72.0 };
        let gaps = l.matches(' ').count() as f32;
        let tw = match k + 1 < lines.len() {
            true => (512.0 - x - text_width(l, 11.0)) / gaps,
            false => 0.0,
        };
        content.push_str(&format!("BT /F1 11 Tf {tw} Tw {x} {y} Td ({l}) Tj ET\n"));
    }
    letter(&content)
}

/// The paragraph's text with `brk` typed after the second line's second word.
fn with_break(para: &Paragraph, brk: char) -> String {
    let (a, _) = para.lines[1];
    let chars: Vec<char> = para.text.chars().collect();
    let spaces = (a..).filter(|&k| chars[k] == ' ');
    let at = spaces.take(2).last().unwrap();
    let mut new: String = chars[..=at].iter().collect();
    new.push(brk);
    new.extend(&chars[at + 1..]);
    new
}

#[test]
fn enter_typed_starts_a_paragraph_indented_and_shift_enter_a_line_neither_justified_before() {
    let p = indented_justified();
    let para = only_paragraph(&p);
    assert_eq!(para.frame.align, Align::Justified);
    assert!((para.frame.indent - 18.0).abs() < 0.05, "{:?}", para.frame);
    let setting = p.paragraph_setting(&para).unwrap();
    for (brk, indent) in [('\u{2029}', 18.0), ('\n', 0.0)] {
        let new = with_break(&para, brk);
        let set = setting.set(&typed(&para, &new));
        let texts = set_texts(&set, &new);
        assert_eq!(texts.join(" "), new.replace(brk, ""), "{brk:?}");
        // The line the break ends starts at the frame's edge, unstretched; the next starts
        // with the first line's indent after Enter only.
        let at = new.chars().position(|c| c == brk).unwrap();
        let n = set.lines.iter().position(|l| l.chars.1 == at + 1).unwrap();
        assert_eq!(n, 1, "{brk:?}: {set:?}");
        assert_eq!(set.lines[n].stretch, 0.0);
        assert!((set.lines[n].x - 72.0).abs() < 0.01, "{:?}", set.lines[n]);
        let next = &set.lines[n + 1];
        assert!((next.x - 72.0 - indent).abs() < 0.01, "{next:?}");
        // The other lines rewritten reach the right edge, but the last.
        let chars: Vec<char> = new.chars().collect();
        let last = set.lines.len() - 1;
        for l in (set.lines[n + 1..last].iter()).filter(|l| l.kept.is_none()) {
            let gaps = (chars[l.chars.0..l.chars.1].iter().collect::<String>())
                .trim_end()
                .matches(' ')
                .count();
            let end = l.x + l.width + l.stretch * gaps as f32;
            assert!((end - 512.0).abs() < 0.5, "{brk:?}: {l:?} ends at {end}");
        }
        // Written so: the break isn't written, the lines are where they were set.
        let mut q = p.clone();
        q.edit_paragraph(&setting, &typed(&para, &new)).unwrap();
        assert_eq!(line_texts(&q), texts, "{brk:?}");
        for (o, l) in lines_of(&q).iter().zip(&set.lines) {
            let at = o.baseline.unwrap();
            assert!(near(at, (l.x, l.baseline)), "{o:?} against {l:?}");
        }
        let before = &lines_of(&q)[n];
        assert!(before.bbox[2] < 500.0, "stretched: {before:?}");
    }
}

/// `TEXT` and `MORE` broken at 300 points, each line starting where `x` says for its width.
fn aligned_page(x: fn(f32) -> f32) -> Pdf {
    let lines = wrap(&format!("{TEXT} {MORE}"), 300.0, 11.0);
    let mut content = String::new();
    for (k, l) in lines.iter().enumerate() {
        let y = 700.0 - 13.2 * k as f32;
        let at = x(text_width(l, 11.0));
        content.push_str(&format!("BT /F1 11 Tf {at} {y} Td ({l}) Tj ET\n"));
    }
    letter(&content)
}

/// Each line centred on 306 points across the page.
fn centred(w: f32) -> f32 {
    306.0 - w / 2.0
}

/// Each line ending 456 points across the page.
fn right_aligned(w: f32) -> f32 {
    456.0 - w
}

#[test]
fn a_centred_paragraph_is_set_again_on_its_centre_and_a_right_aligned_one_to_its_edge() {
    let cases = [
        (Align::Centre, centred as fn(f32) -> f32, 306.0),
        (Align::Right, right_aligned, 456.0),
    ];
    for (align, x, want) in cases {
        // Where a line from `x0` to `x1` is placed by: its centre, or its end.
        let by = |x0: f32, x1: f32| match align {
            Align::Centre => (x0 + x1) / 2.0,
            _ => x1,
        };
        let mut p = aligned_page(x);
        assert_eq!(only_paragraph(&p).frame.align, align);
        let (_, new, set) = type_into_second_line(&mut p, "very slowly and rather quietly");
        for l in &set.lines {
            let at = by(l.x, l.x + l.width);
            assert!((at - want).abs() < 0.01, "{align:?}: {l:?}");
        }
        // Written there.
        assert_eq!(paragraph_texts(&p), set_texts(&set, &new));
        for (o, l) in lines_of(&p).iter().zip(&set.lines) {
            let at = o.baseline.unwrap();
            assert!(near(at, (l.x, l.baseline)), "{o:?} against {l:?}");
            let drawn = by(o.bbox[0], o.bbox[2]);
            assert!((drawn - want).abs() < 0.6, "{align:?}: {o:?}");
        }
    }
}

/// The paragraph's parts with `word` typed before character `at` of its text, in the style
/// of the part it goes into but `size` points high.
fn with_typed(para: &Paragraph, at: usize, word: &str, size: f32) -> Vec<TextPart> {
    let mut out = Vec::new();
    for p in para.parts() {
        let from = p.from.unwrap();
        let chars: Vec<char> = p.text.chars().collect();
        if !(from <= at && at < from + chars.len()) {
            out.push(p);
            continue;
        }
        let (head, tail) = chars.split_at(at - from);
        let typed = TextPart {
            text: word.to_string(),
            size,
            from: None,
            origins: vec![None; word.chars().count()],
            ..p.clone()
        };
        let tail = TextPart {
            text: tail.iter().collect(),
            from: Some(at),
            ..p.clone()
        };
        let head = TextPart {
            text: head.iter().collect(),
            ..p
        };
        let parts = [head, typed, tail].into_iter();
        out.extend(parts.filter(|p| !p.text.is_empty()));
    }
    out
}

#[test]
fn a_line_given_larger_text_moves_down_to_fit_it_and_the_lines_after_it_with_it() {
    let (mut p, _, setting) = ragged(300.0);
    let para = setting.paragraph().clone();
    // A 20-point word typed after the second line's first word.
    let (a, _) = para.lines[1];
    let chars: Vec<char> = para.text.chars().collect();
    let at = a + chars[a..].iter().position(|&c| c == ' ').unwrap();
    let parts = with_typed(&para, at, " Big", 20.0);
    let set = setting.set(&parts);
    let k = (set.lines.iter())
        .position(|l| l.chars.0 <= at + 1 && at + 1 < l.chars.1)
        .unwrap();
    assert_eq!(k, 1, "{set:?}");
    // Its baseline 1.2 times 20 points below the line before, 10.8 more than the
    // paragraph's spacing, and each line after it as much lower.
    let down = |n: usize| 792.0 - (700.0 - 13.2 * n as f32);
    for (n, l) in set.lines.iter().enumerate() {
        let push = if n >= k { 1.2 * 20.0 - 13.2 } else { 0.0 };
        let want = down(n) + push;
        assert!((l.baseline - want).abs() < 0.05, "line {n}: {l:?}");
    }
    p.edit_paragraph(&setting, &parts).unwrap();
    for (o, l) in lines_of(&p).iter().zip(&set.lines) {
        let at = o.baseline.unwrap();
        assert!(near(at, (l.x, l.baseline)), "{o:?} against {l:?}");
    }
}

#[test]
fn a_paragraph_set_tighter_than_single_spacing_keeps_its_spacing_when_rewrapped() {
    // 10-point text on 11-point baselines, words typed in the second line: each line set
    // again on the paragraph's own spacing, not pushed apart line after line.
    let lines = wrap(&format!("{TEXT} {MORE}"), 300.0, 10.0);
    let mut p = letter(&lines_at(&lines, 72.0, 0.0, 700.0, 11.0, "F1", 10.0));
    let (_, _, set) = type_into_second_line(&mut p, "very slowly and rather quietly");
    let down = |n: usize| 792.0 - (700.0 - 11.0 * n as f32);
    for (n, l) in set.lines.iter().enumerate() {
        assert!((l.baseline - down(n)).abs() < 0.05, "line {n}: {l:?}");
    }
}

/// Page 0's only paragraph edited to the text `new` makes of it ([`typed`]), and that text.
fn edited_to(mut p: Pdf, new: impl Fn(&Paragraph) -> String) -> (Pdf, String) {
    let para = only_paragraph(&p);
    let setting = p.paragraph_setting(&para).unwrap();
    let new = new(&para);
    p.edit_paragraph(&setting, &typed(&para, &new)).unwrap();
    (p, new)
}

/// Every kind of paragraph the tests above write, written: what each is, the document, and
/// the paragraph's text as typed.
fn written_paragraphs() -> Vec<(&'static str, Pdf, String)> {
    let into_second = |mut p: Pdf| {
        let (_, new, _) = type_into_second_line(&mut p, "very slowly and rather quietly");
        (p, new)
    };
    let deleted = |para: &Paragraph| para.text.replacen(" fishing boats sat low in the", "", 1);
    let lines = wrap(&format!("{TEXT} {MORE}"), 300.0, 11.0);
    let per_line = tagged(&lines, &vec![0; lines.len()]);
    let tex = |para: &Paragraph| para.text.replacen("ton tony", "ton big tony", 1);
    [
        ("words typed", into_second(ragged(300.0).0)),
        ("words deleted", edited_to(ragged(300.0).0, deleted)),
        ("one text object", into_second(one_object_page())),
        ("tagged line by line", into_second(per_line)),
        ("character spacing", into_second(spaced_page())),
        ("a turned page", into_second(turned_page())),
        ("justified, Enter typed", {
            edited_to(indented_justified(), |para| with_break(para, '\u{2029}'))
        }),
        ("centred", into_second(aligned_page(centred))),
        ("right-aligned", into_second(aligned_page(right_aligned))),
        ("TeX's bitmap font", edited_to(tex_paragraph(), tex)),
    ]
    .into_iter()
    .map(|(what, (p, new))| (what, p, new))
    .collect()
}

/// Text with its whitespace made single spaces.
fn plain(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every kind of paragraph written, read by Poppler (`pdftotext`) as typed. Opt-in: `cargo
/// test -p limarepdf-core --test paragraph -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_written_paragraphs_with_pdftotext() {
    for (what, mut p, new) in written_paragraphs() {
        let got = plain(&tools::pdftotext(&p.to_bytes().unwrap()));
        assert!(got.contains(&plain(&new)), "{what}: {got:?}");
    }
}
