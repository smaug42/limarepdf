mod common;

use common::compare::{differences, Image};
use common::saved::{tools, Saved};
use limarepdf_core::{render, ObjectKind, Pdf, RedactReport};
use lopdf::content::Content;
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

const W: f32 = 300.0;
const H: f32 = 200.0;

/// A document with one 300×200 pt page per entry, each drawing that content with `/F1`
/// (Helvetica, no widths in the file).
fn make(pages: &[&str]) -> Pdf {
    let mut p = Pdf::new();
    for i in 0..pages.len() {
        p.insert_blank(i, W, H).unwrap();
    }
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    for (i, c) in pages.iter().enumerate() {
        let page = p.page_ids()[i];
        let s = p
            .doc
            .add_object(Stream::new(Dictionary::new(), c.as_bytes().to_vec()));
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", Object::Reference(s));
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
    }
    p
}

fn content(p: &Pdf, page: usize) -> String {
    let id = p.page_ids()[page];
    String::from_utf8_lossy(&p.doc.get_page_content(id)).into_owned()
}

/// Pictures in the saved file as written, whether anything still uses them or not.
fn images_in_file(p: &mut Pdf) -> usize {
    Saved::of(p)
        .doc
        .objects
        .values()
        .filter(|o| {
            o.as_stream().is_ok_and(|s| {
                s.dict.get(b"Subtype").and_then(|x| x.as_name()).ok() == Some(b"Image")
            })
        })
        .count()
}

/// Every picture in the saved file as written: its last filter, components and decoded
/// samples (JPEGs decoded to 8-bit grey or RGB).
fn pictures(p: &mut Pdf) -> Vec<(String, usize, Vec<u8>)> {
    let saved = Saved::of(p);
    let mut out = Vec::new();
    for o in saved.doc.objects.values() {
        let Ok(s) = o.as_stream() else { continue };
        if s.dict.get(b"Subtype").and_then(|x| x.as_name()).ok() != Some(b"Image") {
            continue;
        }
        let filter = s
            .filters()
            .ok()
            .and_then(|f| f.last().map(|n| String::from_utf8_lossy(n).into_owned()))
            .unwrap_or_default();
        let gray = s.dict.get(b"ColorSpace").and_then(|x| x.as_name()).ok() != Some(b"DeviceRGB");
        if filter == "DCTDecode" {
            let img = image::load_from_memory(&s.content).unwrap();
            let (n, data) = if gray {
                (1, img.to_luma8().into_raw())
            } else {
                (3, img.to_rgb8().into_raw())
            };
            out.push((filter, n, data));
        } else {
            out.push((
                filter,
                if gray { 1 } else { 3 },
                s.decompressed_content().unwrap(),
            ));
        }
    }
    out
}

fn render_page(p: &mut Pdf) -> render::Bitmap {
    let bytes = p.snapshot().unwrap();
    let doc = render::load(bytes, None).unwrap();
    render::Renderer::new(&doc).render(0, 1.0).unwrap()
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.05)
}

#[test]
fn removes_the_marked_characters_and_keeps_the_rest_in_place() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Name: Zebra Quokka here) Tj ET"]);
    let here = p.find_text_boxes(0, "here").unwrap();
    let name = p.find_text_boxes(0, "Name:").unwrap();
    let marks = p.find_redaction_marks("zebra quokka").unwrap();
    assert_eq!(marks.len(), 1);

    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(rep.elsewhere.is_empty(), "{:?}", rep.elsewhere);
    assert_eq!(rep.pages.len(), 1);
    assert_eq!(rep.pages[0].text, vec!["Zebra Quokka".to_string()]);
    assert_eq!(rep.pages[0].chars, 12);

    let text = p.page_text(0);
    assert!(text.contains("Name:") && text.contains("here"), "{text}");
    assert!(!text.to_lowercase().contains("zebra"), "{text}");
    assert!(near(p.find_text_boxes(0, "here").unwrap()[0], here[0]));
    assert!(near(p.find_text_boxes(0, "Name:").unwrap()[0], name[0]));
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["Zebra", "Quokka", "ebra", "uokk"]);
    saved.assert_kept(&["Name:", "here"]);
}

#[test]
fn removes_characters_of_a_shift_jis_font_whole() {
    // 90ms-RKSJ-H mixes one-byte codes ("A", "B") with two-byte ones (the kanji). The
    // descendant has no widths, so every CID is 1000 wide: 20 pt each.
    let mut p = make(&["BT /F2 20 Tf 30 100 Td <41938C8B9E42> Tj ET"]);
    let desc = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "CIDFontType0", "BaseFont" => "HeiseiMin-W3",
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Japan1"),
            "Supplement" => 2,
        },
    });
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "HeiseiMin-W3",
        "Encoding" => "90ms-RKSJ-H", "DescendantFonts" => vec![desc.into()],
    });
    let page = p.page_ids()[0];
    p.doc.get_dictionary_mut(page).unwrap().set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F2" => font } },
    );
    assert_eq!(p.page_text(0), "A東京B");
    let b = p.find_text_boxes(0, "b").unwrap()[0];
    assert!((b[0] - 90.0).abs() < 0.5, "{b:?}");

    let marks = p.find_redaction_marks("東京").unwrap();
    assert_eq!(marks.len(), 1);
    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert_eq!(rep.pages[0].chars, 2);
    assert_eq!(p.page_text(0).replace(' ', ""), "AB");
    assert!(near(p.find_text_boxes(0, "b").unwrap()[0], b));
    let ops = Content::decode(&p.doc.get_page_content(page)).unwrap();
    // The two kanji go whole, with a 40 pt gap left in their place.
    let tj: Vec<&Object> = ops
        .operations
        .iter()
        .filter(|o| o.operator == "TJ")
        .flat_map(|o| o.operands[0].as_array().unwrap())
        .collect();
    assert_eq!(tj.len(), 3, "{tj:?}");
    assert_eq!(tj[0].as_str().unwrap(), b"A");
    assert_eq!(tj[1].as_float().unwrap(), -2000.0);
    assert_eq!(tj[2].as_str().unwrap(), b"B");
}

/// A page showing `content` with `/F2`, a Type 0 font with its own CMap: codespace lines
/// `space`, and only the codes `chars` mapped, to Adobe-Japan1 CIDs (34 "A", 36 "C", 37 "D").
/// CID 34 is 500 wide, every other 1000. No ToUnicode map and no font program.
fn own_cmap(space: &str, chars: &str, content: &str) -> Pdf {
    let mut p = make(&[content]);
    let n = chars.split_whitespace().count() / 2;
    let cmap = format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 2 >> def \
         /CMapName /Sparse def /CMapType 1 def {space} {n} begincidchar {chars} endcidchar \
         endcmap CMapName currentdict /CMap defineresource pop end end"
    );
    let cmap = p
        .doc
        .add_object(Stream::new(Dictionary::new(), cmap.into_bytes()));
    let desc = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "CIDFontType0", "BaseFont" => "HeiseiMin-W3",
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Japan1"),
            "Supplement" => 2,
        },
        "DW" => 1000, "W" => vec![34.into(), vec![500.into()].into()],
    });
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "HeiseiMin-W3",
        "Encoding" => cmap, "DescendantFonts" => vec![desc.into()],
    });
    let page = p.page_ids()[0];
    p.doc.get_dictionary_mut(page).unwrap().set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F2" => font } },
    );
    p
}

/// Every code is two bytes, but only "A", "C" and "D" are mapped. The middle string holds the
/// unmapped code `<0042>` and then "A". At 20 pt, poppler reads C at x 10 to 30, the unmapped
/// code to 50, A to 60 and D to 80.
fn sparse_cmap() -> Pdf {
    own_cmap(
        "1 begincodespacerange <0000> <FFFF> endcodespacerange",
        "<0041> 34 <0043> 36 <0044> 37",
        "BT /F2 20 Tf 10 100 Td <0043> Tj <00420041> Tj <0044> Tj ET",
    )
}

/// The left and right edges of where `p` reads `s` on page 1.
fn xs(p: &Pdf, s: &str) -> Vec<[f32; 2]> {
    let b = p.find_text_boxes(0, s).unwrap();
    b.iter().map(|b| [b[0], b[2]]).collect()
}

/// Every string in page 1's content, in order.
fn strings_shown(p: &Pdf) -> Vec<Vec<u8>> {
    let ops = Content::decode(&p.doc.get_page_content(p.page_ids()[0])).unwrap();
    fn strings(o: &Object, out: &mut Vec<Vec<u8>>) {
        match o {
            Object::String(b, _) => out.push(b.clone()),
            Object::Array(a) => a.iter().for_each(|x| strings(x, out)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    for o in &ops.operations {
        o.operands.iter().for_each(|x| strings(x, &mut all));
    }
    all
}

/// Whether two bytes `pair` appear together in any string page 1's content shows.
fn shows_pair(p: &Pdf, pair: [u8; 2]) -> bool {
    strings_shown(p)
        .iter()
        .any(|s| s.windows(2).any(|w| w == pair))
}

#[test]
fn reads_codes_of_its_own_cmap_by_the_codespace_as_other_readers_do() {
    let p = sparse_cmap();
    assert_eq!(xs(&p, "C"), [[10.0, 30.0]]);
    assert_eq!(xs(&p, "A"), [[50.0, 60.0]]);
    assert_eq!(xs(&p, "D"), [[60.0, 80.0]]);
    // One-byte codes below 0x80, two-byte codes from 0x8140 to 0x9FFC (each byte in its
    // range). 0x8150 and 0x42 aren't mapped; 0x8120 starts like a two-byte code, so poppler
    // takes two bytes for it too.
    let p = own_cmap(
        "2 begincodespacerange <00> <7F> <8140> <9FFC> endcodespacerange",
        "<41> 34 <8141> 36 <8142> 37",
        "BT /F2 20 Tf 10 100 Td <41815042814181208142> Tj ET",
    );
    assert_eq!(xs(&p, "A"), [[10.0, 20.0]]);
    assert_eq!(xs(&p, "C"), [[60.0, 80.0]]);
    assert_eq!(xs(&p, "D"), [[100.0, 120.0]]);
    // A one-byte code mapped though it starts like the three-byte codes (as pdf.js's issue
    // 11768 has): one byte, as poppler and hayro read it.
    let p = own_cmap(
        "2 begincodespacerange <00> <7F> <E08080> <EFBFBF> endcodespacerange",
        "<E0> 34 <43> 36 <44> 37",
        "BT /F2 20 Tf 10 100 Td <E0E0E04344> Tj ET",
    );
    assert_eq!(p.page_text(0), "AAACD");
    assert_eq!(xs(&p, "C"), [[40.0, 60.0]]);
    assert_eq!(xs(&p, "D"), [[60.0, 80.0]]);
}

/// PB-06: a code the CMap of a Type 0 font maps to no CID reads as U+FFFD, one for each
/// code, never through the one-byte encodings of simple fonts (`<0042>` read as "B").
/// Identity-H codes are all CIDs: one without text keeps that reading, which in real files
/// is often the text meant.
#[test]
fn codes_no_map_covers_read_as_replacement_characters() {
    assert_eq!(sparse_cmap().page_text(0), "C\u{FFFD}AD");
    // Unmapped codes of two bytes and of one.
    let p = own_cmap(
        "2 begincodespacerange <00> <7F> <8140> <9FFC> endcodespacerange",
        "<41> 34 <8141> 36 <8142> 37",
        "BT /F2 20 Tf 10 100 Td <41815042814181208142> Tj ET",
    );
    assert_eq!(p.page_text(0), "A\u{FFFD}\u{FFFD}C\u{FFFD}D");
    // Identity-H, with a ToUnicode map that leaves `<0042>` out, and with none.
    let identity = |gaps: bool| {
        let mut p = make(&["BT /F2 20 Tf 10 100 Td <004100420043> Tj ET"]);
        let desc = p.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => "Gaps",
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal("Identity"),
                "Supplement" => 0,
            },
        });
        let mut font = dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "Gaps",
            "Encoding" => "Identity-H", "DescendantFonts" => vec![desc.into()],
        };
        if gaps {
            let map = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                       /CMapName /Gaps def /CMapType 2 def \
                       1 begincodespacerange <0000> <FFFF> endcodespacerange \
                       2 beginbfchar <0041> <0041> <0043> <0043> endbfchar \
                       endcmap CMapName currentdict /CMap defineresource pop end end";
            let map = p
                .doc
                .add_object(Stream::new(Dictionary::new(), map.as_bytes().to_vec()));
            font.set("ToUnicode", map);
        }
        let font = p.doc.add_object(font);
        let page = p.page_ids()[0];
        p.doc.get_dictionary_mut(page).unwrap().set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F2" => font } },
        );
        p.page_text(0)
    };
    assert_eq!(identity(true), "A\u{FFFD}C");
    assert_eq!(identity(false), "ABC");
}

#[test]
fn an_area_over_a_character_of_a_sparse_cmap_removes_it_where_other_readers_put_it() {
    // Over A where poppler reads it.
    let mut p = sparse_cmap();
    let rep = p.redact(&[(0, [49.0, 80.0, 61.0, 108.0])]).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(!shows_pair(&p, [0x00, 0x41]));
    assert!(!p.page_text(0).contains('A'), "{}", p.page_text(0));
    assert_eq!(xs(&p, "C"), [[10.0, 30.0]]);
    assert_eq!(xs(&p, "D"), [[60.0, 80.0]]);
    // A mark found by search does the same.
    let mut p = sparse_cmap();
    let marks = p.find_redaction_marks("a").unwrap();
    assert_eq!(marks.len(), 1);
    assert!(p.redact(&marks).unwrap().left.is_empty());
    assert!(!shows_pair(&p, [0x00, 0x41]));
    assert_eq!(xs(&p, "C"), [[10.0, 30.0]]);
    assert_eq!(xs(&p, "D"), [[60.0, 80.0]]);
}

/// LimarePDF draws a code that its CMap doesn't map as one byte, as hayro does, so it can draw
/// a string's later characters elsewhere than other readers put them. An area over where
/// LimarePDF draws them removes them too.
#[test]
fn an_area_over_a_character_where_limarepdf_draws_it_removes_it() {
    // Poppler: A at x 70 to 80. Drawn: four one-byte codes, then A at x 110 to 120.
    let mut p = own_cmap(
        "1 begincodespacerange <0000> <FFFF> endcodespacerange",
        "<0041> 34 <0043> 36 <0044> 37",
        "BT /F2 20 Tf 10 100 Td <0043> Tj <004200420041> Tj <0044> Tj ET",
    );
    assert_eq!(xs(&p, "A"), [[70.0, 80.0]]);
    let rep = p.redact(&[(0, [109.0, 80.0, 121.0, 108.0])]).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(!shows_pair(&p, [0x00, 0x41]));
    assert_eq!(xs(&p, "C"), [[10.0, 30.0]]);
    assert_eq!(xs(&p, "D"), [[80.0, 100.0]]);
    // A CMap mapping a two-byte code outside its one-byte codespace: read by the codespace,
    // but drawn as mapped, where poppler puts it too (A at x 10 to 20).
    let mut p = own_cmap(
        "1 begincodespacerange <00> <FF> endcodespacerange",
        "<0041> 34 <43> 36 <44> 37",
        "BT /F2 20 Tf 10 100 Td <00414344> Tj ET",
    );
    let rep = p.redact(&[(0, [9.0, 80.0, 21.0, 108.0])]).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(!shows_pair(&p, [0x00, 0x41]));
}

#[test]
fn characters_of_a_cmap_without_a_codespace_go_with_their_whole_string() {
    // Where its codes start and end isn't certain, so a mark over A takes C with it. D, on
    // the line below, stays.
    let mut p = own_cmap(
        "",
        "<0041> 34 <0043> 36 <0044> 37",
        "BT /F2 20 Tf 10 100 Td <00430041> Tj 0 -50 Td <0044> Tj ET",
    );
    assert_eq!(xs(&p, "A"), [[30.0, 40.0]]);
    let rep = p.redact(&[(0, [29.0, 80.0, 41.0, 108.0])]).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(!shows_pair(&p, [0x00, 0x41]));
    assert!(!shows_pair(&p, [0x00, 0x43]));
    assert_eq!(xs(&p, "D"), [[10.0, 30.0]]);
}

/// A code mapped at one length that also starts a longer code: LimarePDF and hayro read the
/// shorter, poppler the longer mapping and pdf.js the longer codespace range (PB-03). Where
/// the characters are isn't certain, so a mark over where another reader shows D takes
/// the whole string.
#[test]
fn a_code_that_also_starts_a_longer_one_goes_with_its_whole_string() {
    // Poppler: C (`<4143>`) at x 10 to 30, D at 30 to 50. LimarePDF: A to 20, B (`<43>`) to
    // 40, D to 60.
    let mut p = own_cmap(
        "1 begincodespacerange <00> <FF> endcodespacerange",
        "<41> 34 <4143> 36 <43> 35 <44> 37",
        "BT /F2 20 Tf 10 100 Td <414344> Tj ET",
    );
    assert_eq!(xs(&p, "D"), [[40.0, 60.0]]);
    let rep = p.redact(&[(0, [31.0, 80.0, 39.0, 108.0])]).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(!p.page_text(0).contains('D'), "{}", p.page_text(0));
    // pdf.js: `<8140>` (unmapped, by the codespace) to x 30, D to 50; LimarePDF and hayro
    // take the mapped one-byte `<81>` (A), then B (`<40>`) and D.
    let mut p = own_cmap(
        "2 begincodespacerange <00> <7F> <8140> <9FFC> endcodespacerange",
        "<81> 34 <40> 35 <44> 37",
        "BT /F2 20 Tf 10 100 Td <814044> Tj ET",
    );
    assert_eq!(xs(&p, "D"), [[40.0, 60.0]]);
    let rep = p.redact(&[(0, [31.0, 80.0, 39.0, 108.0])]).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(!p.page_text(0).contains('D'), "{}", p.page_text(0));
}

/// PB-100: a CMap (codespace `<00>` to `<FF>`) mapping a one-byte code that also starts a
/// mapped two-byte code, `<41>` A and `<4142>` C, with `<44>` D and, in the second, `<42>`
/// B, else `<43>` B. Poppler reads a code that starts a longer mapped one with the byte
/// after it, mapped or not (unmapped, it draws nothing, a code 1000 wide); LimarePDF and
/// hayro read one byte. At 20 pt from x 10:
///
/// 1. `<414344>`: LimarePDF reads A to x 20, B to 40 and D to 60; poppler nothing
///    (`<4143>`) to 30 and D to 50.
/// 2. `<414442>`: LimarePDF reads A to 20, D to 40 and B to 60; poppler nothing (`<4144>`)
///    to 30 and B to 50, across LimarePDF's D and B.
fn starts_a_mapped_pair(second: bool) -> Pdf {
    let (chars, shown) = if second {
        ("<41> 34 <4142> 36 <42> 35 <44> 37", "<414442>")
    } else {
        ("<41> 34 <4142> 36 <43> 35 <44> 37", "<414344>")
    };
    own_cmap(
        "1 begincodespacerange <00> <FF> endcodespacerange",
        chars,
        &format!("BT /F2 20 Tf 10 100 Td {shown} Tj ET"),
    )
}

/// PB-100: a font whose CMap maps a code that starts a longer mapped one. Where poppler puts
/// the characters of a string in it isn't certain, so a mark over any of them takes the
/// whole show, its width kept: the string after it stays where it was.
#[test]
fn a_cmap_mapping_a_code_that_starts_a_longer_one_takes_the_whole_show() {
    for second in [false, true] {
        let mut p = starts_a_mapped_pair(second);
        let page = p.page_ids()[0];
        let content = String::from_utf8(p.doc.get_page_content(page)).unwrap();
        let content = content.replace("Tj ET", "Tj <44> Tj ET");
        p.doc
            .change_page_content(page, content.into_bytes())
            .unwrap();
        assert_eq!(xs(&p, "D").last(), Some(&[60.0, 80.0]), "{second}");
        // Over LimarePDF's A.
        let rep = p.redact(&[(0, [11.0, 80.0, 19.0, 108.0])]).unwrap();
        assert!(rep.left.is_empty(), "{second}: {:?}", rep.left);
        assert_eq!(strings_shown(&p), [b"\x44"], "{second}");
        assert_eq!(xs(&p, "D"), [[60.0, 80.0]], "{second}");
    }
}

/// Two marks laid on each other over the last fifth of S and nearly all of e (Q14, PB-13):
/// S is a fifth under them, not two fifths, so it stays; e goes, and nothing readable is
/// left under them.
#[test]
fn marks_laid_on_each_other_count_once() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Seal) Tj ET"]);
    let s = p.find_text_boxes(0, "S").unwrap()[0];
    let e = p.find_text_boxes(0, "e").unwrap()[0];
    let mark = [
        s[2] - 0.2 * (s[2] - s[0]),
        s[1] - 10.0,
        e[2] - 0.5,
        s[3] + 10.0,
    ];
    let rep = p.redact(&[(0, mark), (0, mark)]).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert_eq!(rep.pages[0].chars, 1);
    let text = p.page_text(0).replace(' ', "");
    assert_eq!(text, "Sal");
    assert!(near(p.find_text_boxes(0, "S").unwrap()[0], s));
}

#[test]
fn keeps_tj_spacing_and_later_strings_in_place() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td [(Sec) -200 (retX) 50 (Yzw)] TJ ET"]);
    let sec = p.find_text_boxes(0, "Sec").unwrap()[0];
    let yzw = p.find_text_boxes(0, "Yzw").unwrap()[0];
    let marks = p.find_redaction_marks("retX").unwrap();
    let rep = p.redact(&marks).unwrap();
    assert_eq!(rep.pages[0].chars, 4);
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(near(p.find_text_boxes(0, "Sec").unwrap()[0], sec));
    assert!(near(p.find_text_boxes(0, "Yzw").unwrap()[0], yzw));
    assert!(p.find_text_boxes(0, "ret").unwrap().is_empty());
}

/// "Secret" whose t is shown as `[(t) take] TJ`: 278 takes back its whole width (Helvetica's
/// t is 278 wide), 500 more than it (as pdf.js's issue1912 does). "here" stands apart. At
/// 20 pt, "Secre" is 52.24 wide, so t is drawn from x 82.24 to 87.8.
fn taken_back(take: u16) -> Pdf {
    make(&[&format!(
        "BT /F1 20 Tf 30 100 Td (Secre) Tj [(t) {take}] TJ ET \
         BT /F1 20 Tf 120 100 Td (here) Tj ET"
    )])
}

/// PB-78: a letter whose show takes its width back is as wide as its glyph, so a mark found
/// by search takes it with its word and nothing of it is left.
#[test]
fn a_letter_whose_width_is_taken_back_goes_with_its_word() {
    for take in [278, 500] {
        let mut p = taken_back(take);
        assert_eq!(p.page_text(0).replace(' ', ""), "Secrethere", "{take}");
        let t = xs(&p, "t");
        let wide = t.len() == 1 && (t[0][0] - 82.24).abs() < 0.05 && (t[0][1] - 87.8).abs() < 0.05;
        assert!(wide, "{take}: t at {t:?}");
        let here = p.find_text_boxes(0, "here").unwrap()[0];
        let marks = p.find_redaction_marks("Secret").unwrap();
        let rep = p.redact(&marks).unwrap();
        assert!(rep.left.is_empty(), "{take}: {:?}", rep.left);
        assert_eq!(rep.pages[0].chars, 6, "{take}");
        assert_eq!(p.page_text(0).replace(' ', ""), "here", "{take}");
        let left = strings_shown(&p);
        assert!(!left.iter().any(|s| s.contains(&b't')), "{take}: {left:?}");
        assert!(
            near(p.find_text_boxes(0, "here").unwrap()[0], here),
            "{take}"
        );
    }
}

/// PB-78's control: an area drawn over the t's glyph already removes it, as the redactor
/// measures a letter by its glyph.
#[test]
fn an_area_over_a_letter_whose_width_is_taken_back_removes_it() {
    for take in [278, 500] {
        let mut p = taken_back(take);
        let rep = p.redact(&[(0, [82.0, 80.0, 88.5, 108.0])]).unwrap();
        assert!(rep.left.is_empty(), "{take}: {:?}", rep.left);
        assert_eq!(rep.pages[0].chars, 1, "{take}");
        assert_eq!(p.page_text(0).replace(' ', ""), "Secrehere", "{take}");
    }
}

#[test]
fn removes_hidden_text_and_its_alternative_text() {
    let mut p =
        make(&["/Span <</ActualText (Zebra)>> BDC BT 3 Tr /F1 20 Tf 30 100 Td (Zebra) Tj ET EMC"]);
    let marks = p.find_redaction_marks("zebra").unwrap();
    assert_eq!(marks.len(), 1, "hidden text is found");
    let rep = p.redact(&marks).unwrap();
    assert_eq!(rep.pages[0].chars, 5);
    assert!(!content(&p, 0).contains("ActualText"));
    Saved::of(&mut p).assert_gone(&["Zebra"]);
}

#[test]
fn removes_drawings_under_a_mark_and_clips_those_crossing_it() {
    // A red background across the page, and a small blue square at 100,50 (user space).
    let mut p = make(&["q 1 0 0 rg 10 10 280 180 re f Q q 0 0 1 rg 100 50 20 20 re f Q"]);
    // Displayed y runs down: user y 40..80 is displayed 120..160.
    let rep = p.redact(&[(0, [90.0, 120.0, 130.0, 160.0])]).unwrap();
    assert_eq!(rep.pages[0].paths, 1);
    assert_eq!(rep.pages[0].clipped, 1);
    let ops = Content::decode(content(&p, 0).as_bytes())
        .unwrap()
        .operations;
    let rects: Vec<Vec<f32>> = ops
        .iter()
        .filter(|o| o.operator == "re")
        .map(|o| o.operands.iter().map(|x| x.as_float().unwrap()).collect())
        .collect();
    assert!(rects.contains(&vec![10.0, 10.0, 280.0, 180.0]), "{rects:?}");
    assert!(!rects.contains(&vec![100.0, 50.0, 20.0, 20.0]), "{rects:?}");
    assert!(ops.iter().any(|o| o.operator == "W*"));

    let bytes = p.snapshot().unwrap();
    let doc = render::load(bytes, None).unwrap();
    let bmp = render::Renderer::new(&doc).render(0, 1.0).unwrap();
    let px = |x: usize, y: usize| {
        let i = (y * bmp.width + x) * 4;
        [bmp.rgba[i], bmp.rgba[i + 1], bmp.rgba[i + 2]]
    };
    assert!(
        px(110, 140)[0] < 40 && px(110, 140)[2] < 40,
        "the mark is black"
    );
    assert!(
        px(50, 50)[0] > 200 && px(50, 50)[2] < 60,
        "the background stays red outside"
    );
}

#[test]
fn blacks_out_the_covered_part_of_a_picture() {
    let mut p = make(&["BT /F1 20 Tf 30 30 Td (Caption) Tj ET"]);
    // A 10×10 picture filling 100..200 × 50..150 (displayed), translucent so it gets a
    // soft mask, which must be cut too.
    let img = image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 200, 0, 200]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    p.add_image(0, [100.0, 50.0, 200.0, 150.0], &png.into_inner())
        .unwrap();
    assert_eq!(images_in_file(&mut p), 2, "the picture and its soft mask");
    // Pixel columns 2..4 and rows 1..3 of the picture.
    let rep = p.redact(&[(0, [120.0, 60.0, 140.0, 80.0])]).unwrap();
    assert_eq!((rep.pages[0].images, rep.pages[0].images_cut), (0, 1));
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(p.page_text(0).contains("Caption"));
    let pics = pictures(&mut p);
    assert_eq!(pics.len(), 2, "the old picture and mask are gone");
    for (_, n, data) in pics {
        let px = |x: usize, y: usize| &data[(y * 10 + x) * n..(y * 10 + x + 1) * n];
        assert!(px(2, 1).iter().all(|&v| v == 0) && px(3, 2).iter().all(|&v| v == 0));
        assert!(px(1, 1).iter().any(|&v| v != 0) && px(4, 2).iter().any(|&v| v != 0));
        assert!(px(2, 3).iter().any(|&v| v != 0) && px(2, 0).iter().any(|&v| v != 0));
    }
    let bmp = render_page(&mut p);
    let at = |x: usize, y: usize| bmp.rgba[(y * bmp.width + x) * 4 + 1];
    assert!(at(170, 120) > 100, "the rest of the picture still shows");
}

/// A picture whose `/DecodeParms` is a reference (as some writers give it, pdf.js's
/// `issue17906` among them): its PNG predictor is undone before it is cut, so the rest of
/// it keeps its colours.
#[test]
fn cuts_a_picture_whose_decode_parameters_are_a_reference() {
    let mut p = make(&["q 100 0 0 100 100 50 cm /Im1 Do Q"]);
    // Each row: PNG's "no filter" byte, then its pixels.
    let row: Vec<u8> = [0u8].into_iter().chain([200, 30, 60].repeat(10)).collect();
    let rows = row.repeat(10);
    let parms = p.doc.add_object(dictionary! {
        "Predictor" => 15, "Colors" => 3, "BitsPerComponent" => 8, "Columns" => 10,
    });
    let mut s = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 10, "Height" => 10,
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
        },
        rows,
    );
    s.compress().unwrap();
    s.dict.set("DecodeParms", parms);
    let im = p.doc.add_object(s);
    let page = p.page_ids()[0];
    let res = p
        .doc
        .get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .unwrap();
    res.as_dict_mut()
        .unwrap()
        .set("XObject", dictionary! {"Im1" => im});
    // The top two rows' first two pixels (marks are measured from the page's top).
    let rep = p.redact(&[(0, [100.0, 50.0, 120.0, 70.0])]).unwrap();
    assert_eq!(rep.pages[0].images_cut, 1);
    let pics = pictures(&mut p);
    assert_eq!(pics.len(), 1);
    let (n, data) = (pics[0].1, &pics[0].2);
    for y in 0..10 {
        for x in 0..10 {
            let px = &data[(y * 10 + x) * n..(y * 10 + x + 1) * n];
            let cut = x < 2 && y < 2;
            let want = if cut { [0, 0, 0] } else { [200, 30, 60] };
            assert_eq!(px, want, "pixel ({x}, {y})");
        }
    }
}

#[test]
fn keeps_a_jpeg_picture_as_jpeg() {
    let mut p = make(&[""]);
    let img = image::RgbImage::from_pixel(16, 16, image::Rgb([240, 240, 240]));
    let mut jpg = std::io::Cursor::new(Vec::new());
    img.write_to(&mut jpg, image::ImageFormat::Jpeg).unwrap();
    p.add_image(0, [100.0, 50.0, 260.0, 210.0], &jpg.into_inner())
        .unwrap();
    let rep = p.redact(&[(0, [100.0, 50.0, 180.0, 130.0])]).unwrap();
    assert_eq!(rep.pages[0].images_cut, 1);
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    let pics = pictures(&mut p);
    assert_eq!(pics.len(), 1);
    assert_eq!(pics[0].0, "DCTDecode");
    let (n, data) = (pics[0].1, &pics[0].2);
    let v = |x: usize, y: usize| data[(y * 16 + x) * n] as i32;
    assert!(v(2, 2) < 20 && v(13, 13) > 200, "{} {}", v(2, 2), v(13, 13));
}

#[test]
fn turns_an_inline_image_into_a_cut_picture() {
    // A 4×4 grey inline image drawn at 50..150 (user space); the mark covers its top half.
    let mut c = b"q 100 0 0 100 50 50 cm BI /W 4 /H 4 /CS /G /BPC 8 ID ".to_vec();
    c.extend([0xC8u8; 16]);
    c.extend(b" EI Q");
    let mut p = make(&[""]);
    let page = p.page_ids()[0];
    let s = p.doc.add_object(Stream::new(Dictionary::new(), c));
    p.doc.get_dictionary_mut(page).unwrap().set("Contents", s);
    assert_eq!(p.page_objects(0).unwrap().len(), 1);
    let rep = p.redact(&[(0, [50.0, 50.0, 150.0, 100.0])]).unwrap();
    assert_eq!(rep.pages[0].images_cut, 1);
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(!content(&p, 0).contains("BI"));
    let pics = pictures(&mut p);
    assert_eq!(pics.len(), 1);
    let mut want = vec![0u8; 8];
    want.extend([0xC8; 8]);
    assert_eq!(pics[0].2, want);
}

#[test]
fn removes_a_picture_it_cannot_read_whole() {
    let mut p = make(&["q 100 0 0 100 50 50 cm /Im1 Do Q"]);
    let img = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 4, "Height" => 4,
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "JPXDecode",
        },
        vec![1, 2, 3],
    ));
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    let mut res = d.get(b"Resources").unwrap().as_dict().unwrap().clone();
    res.set("XObject", dictionary! { "Im1" => img });
    d.set("Resources", res);
    let rep = p.redact(&[(0, [60.0, 60.0, 70.0, 70.0])]).unwrap();
    assert_eq!((rep.pages[0].images, rep.pages[0].images_cut), (1, 0));
    assert_eq!(images_in_file(&mut p), 0);
}

#[test]
fn removes_a_fax_picture_too_large_to_decode() {
    // 65,535 pixels a side, 4 GB once decoded, from 8 KB: each 1 bit is a white row
    // ("vertical 0" below a white row).
    let mut p = make(&["q 100 0 0 100 50 50 cm /Im1 Do Q"]);
    let img = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 65_535, "Height" => 65_535,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 1,
            "Filter" => "CCITTFaxDecode",
            "DecodeParms" => dictionary! { "K" => -1, "Columns" => 65_535 },
        },
        vec![0xFF; 8192],
    ));
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    let mut res = d.get(b"Resources").unwrap().as_dict().unwrap().clone();
    res.set("XObject", dictionary! { "Im1" => img });
    d.set("Resources", res);
    let rep = p.redact(&[(0, [60.0, 60.0, 70.0, 70.0])]).unwrap();
    assert_eq!((rep.pages[0].images, rep.pages[0].images_cut), (1, 0));
    assert_eq!(images_in_file(&mut p), 0);
}

#[test]
fn drops_a_picture_from_shared_resources_when_no_page_draws_it() {
    let mut p = make(&[
        "q 50 0 0 50 100 50 cm /Im1 Do Q",
        "BT /F1 20 Tf 30 30 Td (Two) Tj ET",
    ]);
    let img = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        vec![0x80],
    ));
    let font = p
        .doc
        .get_dictionary(p.page_ids()[0])
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .clone();
    let mut shared = font.as_dict().unwrap().clone();
    shared.set("XObject", dictionary! { "Im1" => img });
    let shared: ObjectId = p.doc.add_object(shared);
    for page in p.page_ids() {
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("Resources", shared);
    }
    assert_eq!(images_in_file(&mut p), 1);
    let rep = p.redact(&[(0, [110.0, 100.0, 120.0, 110.0])]).unwrap();
    assert_eq!(rep.pages[0].images_cut, 1);
    // Only the blacked-out copy is left: the shared resources no longer list the old one.
    let pics = pictures(&mut p);
    assert_eq!(pics.len(), 1);
    assert_eq!(pics[0].2, vec![0]);
    assert!(p.page_text(1).contains("Two"));
}

/// A picture page 2 still draws, from inside forms nested deeper than redaction redacts into,
/// stays listed in the resources the pages share after page 1's use of it is blacked out.
#[test]
fn keeps_a_shared_picture_another_page_draws_from_deeply_nested_forms() {
    let mut p = make(&["q 50 0 0 50 100 50 cm /Im1 Do Q", "q /Fm1 Do Q"]);
    let img = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        vec![0x20],
    ));
    // Forms 1 to 12, each drawing the next, without resources of their own; the last draws
    // the picture.
    let mut xobjects = dictionary! { "Im1" => img };
    for i in 1..=12 {
        let content = if i == 12 {
            "q 50 0 0 50 100 50 cm /Im1 Do Q".to_string()
        } else {
            format!("/Fm{} Do", i + 1)
        };
        let bbox: Vec<Object> = [0, 0, 300, 200].into_iter().map(Object::Integer).collect();
        let fm = p.doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => bbox },
            content.into_bytes(),
        ));
        xobjects.set(format!("Fm{i}"), fm);
    }
    let shared = p.doc.add_object(dictionary! { "XObject" => xobjects });
    for page in p.page_ids() {
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("Resources", shared);
    }
    let dark = |p: &mut Pdf| {
        let doc = render::load(p.snapshot().unwrap(), None).unwrap();
        let b = render::Renderer::new(&doc).render(1, 1.0).unwrap();
        b.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[0] < 100)
            .count()
    };
    let before = dark(&mut p);
    assert!(before > 2000, "{before}");
    let rep = p.redact(&[(0, [110.0, 100.0, 120.0, 110.0])]).unwrap();
    assert_eq!(rep.pages[0].images_cut, 1);
    assert_eq!(dark(&mut p), before, "page 2's picture");
}

/// A form without resources of its own draws `/Pic`, which is a different picture on each
/// page. Page 1's own use of page 2's picture is blacked out; page 2 still draws it through
/// the form, so it stays in page 2's resources.
#[test]
fn keeps_a_picture_another_page_draws_through_a_form_using_that_page_s_names() {
    let mut p = make(&["q 50 0 0 50 100 50 cm /Im1 Do Q q /Fm Do Q", "q /Fm Do Q"]);
    let picture = |p: &mut Pdf, v: u8| {
        p.doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            vec![v],
        ))
    };
    let (img, other) = (picture(&mut p, 0x20), picture(&mut p, 0x40));
    let bbox: Vec<Object> = [0, 0, 300, 200].into_iter().map(Object::Integer).collect();
    let fm = p.doc.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => bbox },
        b"q 50 0 0 50 200 120 cm /Pic Do Q".to_vec(),
    ));
    let pages = p.page_ids();
    let xobjects = [
        dictionary! { "Im1" => img, "Fm" => fm, "Pic" => other },
        dictionary! { "Fm" => fm, "Pic" => img },
    ];
    for (page, xo) in pages.into_iter().zip(xobjects) {
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("Resources", dictionary! { "XObject" => xo });
    }
    let dark = |p: &mut Pdf| {
        let doc = render::load(p.snapshot().unwrap(), None).unwrap();
        let b = render::Renderer::new(&doc).render(1, 1.0).unwrap();
        b.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[0] < 100)
            .count()
    };
    let before = dark(&mut p);
    assert!(before > 2000, "{before}");
    let rep = p.redact(&[(0, [110.0, 100.0, 120.0, 110.0])]).unwrap();
    assert_eq!(rep.pages[0].images_cut, 1);
    assert_eq!(dark(&mut p), before, "page 2's picture");
}

/// Two pages drawing the same form, which says "Secret Walrus".
fn shared_form() -> Pdf {
    let mut p = make(&["q /Fm1 Do Q", "q /Fm1 Do Q"]);
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let form = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
        b"BT /F1 20 Tf 30 100 Td (Secret Walrus) Tj ET".to_vec(),
    ));
    for page in p.page_ids() {
        p.doc.get_dictionary_mut(page).unwrap().set(
            "Resources",
            dictionary! { "XObject" => dictionary! { "Fm1" => form } },
        );
    }
    p
}

#[test]
fn redacts_inside_a_shared_form_only_on_the_marked_page() {
    let mut p = shared_form();
    let marks: Vec<_> = p
        .find_redaction_marks("walrus")
        .unwrap()
        .into_iter()
        .filter(|m| m.0 == 0)
        .collect();
    assert_eq!(marks.len(), 1);
    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert_eq!(rep.pages[0].chars, 6);
    assert!(!p.page_text(0).to_lowercase().contains("walrus"));
    assert!(p.page_text(0).contains("Secret"));
    assert!(
        p.page_text(1).contains("Walrus"),
        "the other page keeps the form as it was"
    );
    assert_eq!(
        rep.elsewhere,
        vec![("walrus".to_string(), vec!["page 2".to_string()])]
    );
}

// What the redacted content's resources still hold (H-01, H-02): alternative text in a named
// property, and forms whose copies draw with resources that list the original.

/// Adds `extra`'s entries to page `page`'s resources; a dictionary of names (`/XObject`,
/// `/Font`, …) is added to the one there.
fn add_res(p: &mut Pdf, page: usize, extra: Dictionary) {
    let id = p.page_ids()[page];
    let d = p.doc.get_dictionary_mut(id).unwrap();
    let mut res = d.get(b"Resources").unwrap().as_dict().unwrap().clone();
    for (k, v) in extra.iter() {
        match (res.get_mut(k), v) {
            (Ok(Object::Dictionary(have)), Object::Dictionary(more)) => {
                for (n, x) in more.iter() {
                    have.set(n.clone(), x.clone());
                }
            }
            _ => res.set(k.clone(), v.clone()),
        }
    }
    d.set("Resources", res);
}

/// A form over the whole page drawing `content`: with resources of its own when `res` is
/// given, else with those of where it is drawn.
fn form_with(p: &mut Pdf, content: &str, res: Option<Dictionary>) -> ObjectId {
    let bbox: Vec<Object> = [0, 0, 300, 200].into_iter().map(Object::Integer).collect();
    let mut d = dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => bbox };
    if let Some(res) = res {
        d.set("Resources", res);
    }
    p.doc
        .add_object(Stream::new(d, content.as_bytes().to_vec()))
}

/// A grey picture one row high whose samples are `samples`' bytes, so the saved-output check
/// finds it by them.
fn marked_picture(p: &mut Pdf, samples: &str) -> ObjectId {
    p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => samples.len() as i64,
            "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        samples.as_bytes().to_vec(),
    ))
}

/// Page 1's `/F1`.
fn helvetica_of(p: &Pdf) -> Object {
    let page = p.doc.get_dictionary(p.page_ids()[0]).unwrap();
    let res = page.get(b"Resources").unwrap().as_dict().unwrap();
    let fonts = res.get(b"Font").unwrap().as_dict().unwrap();
    fonts.get(b"F1").unwrap().clone()
}

/// The dictionary `o` is or refers to; empty when there is none.
fn dict_at(p: &Pdf, o: Option<&Object>) -> Dictionary {
    match o.map(|o| p.doc.dereference(o)) {
        Some(Ok((_, Object::Dictionary(d)))) => d.clone(),
        Some(Ok((_, Object::Stream(s)))) => s.dict.clone(),
        _ => Dictionary::new(),
    }
}

/// Marks over `query` on page `page` only.
fn marks_on(p: &Pdf, page: usize, query: &str) -> Vec<(usize, [f32; 4])> {
    let marks: Vec<_> = (p.find_redaction_marks(query).unwrap().into_iter())
        .filter(|m| m.0 == page)
        .collect();
    assert!(!marks.is_empty(), "{query} is on page {}", page + 1);
    marks
}

/// Page `page` as LimarePDF draws it, at 72 dpi.
fn drawn(p: &mut Pdf, page: usize) -> Image {
    let doc = render::load(p.snapshot().unwrap(), None).unwrap();
    Image::from_bitmap(&render::Renderer::new(&doc).render(page, 1.0).unwrap())
}

/// How many pixels of two drawings of a page at 72 dpi differ (beyond anti-aliasing) outside
/// `marks` (displayed-page points, y down) and two pixels around them.
fn changed_outside(a: &Image, b: &Image, marks: &[(usize, [f32; 4])]) -> usize {
    let (mask, w, _) = differences(a, b, 8);
    let near_mark = |x: f32, y: f32| {
        marks
            .iter()
            .any(|(_, r)| x >= r[0] - 2.0 && x <= r[2] + 2.0 && y >= r[1] - 2.0 && y <= r[3] + 2.0)
    };
    (mask.iter().enumerate())
        .filter(|&(i, &m)| m && !near_mark((i % w) as f32, (i / w) as f32))
        .count()
}

/// Pixels in `area` (x0, y0, x1, y1 at 72 dpi, y down) darker than light grey.
fn ink(img: &Image, area: [usize; 4]) -> usize {
    (area[1]..area[3])
        .flat_map(|y| (area[0]..area[2]).map(move |x| (x, y)))
        .filter(|&(x, y)| img.px(x, y).iter().any(|&c| c < 200))
        .count()
}

/// Redacting a marked span whose alternative text is in a named property (`/Span /P1 BDC`),
/// the property dictionary and the page's list of properties each direct or a reference:
/// the alternative text leaves the file, not only the content's own copy of the property.
#[test]
fn alternative_text_in_a_named_property_goes() {
    for key in ["ActualText", "Alt", "E"] {
        for listed_by_reference in [false, true] {
            let mut p = make(&["/Span /P1 BDC BT /F1 20 Tf 30 100 Td (Zebra) Tj ET EMC \
                 BT /F1 20 Tf 30 40 Td (Kept) Tj ET"]);
            let mut alt = Dictionary::new();
            alt.set(key, Object::string_literal("ZebraSecretAlt"));
            let prop = p.doc.add_object(alt);
            let list: Object = match listed_by_reference {
                false => dictionary! { "P1" => prop }.into(),
                true => p.doc.add_object(dictionary! { "P1" => prop }).into(),
            };
            add_res(&mut p, 0, dictionary! { "Properties" => list });
            common::seeds::pdf(&p);
            let marks = marks_on(&p, 0, "zebra");
            let rep = p.redact(&marks).unwrap();
            assert!(rep.left.is_empty(), "{key}: {:?}", rep.left);
            let saved = Saved::of(&mut p);
            saved.assert_gone(&["ZebraSecretAlt"]);
            saved.assert_kept(&["Kept"]);
        }
    }
}

/// As above, with the property in the resources of the form that draws the span, and of a
/// form inside another form.
#[test]
fn alternative_text_in_a_form_s_named_property_goes() {
    for nested in [false, true] {
        let mut p = make(&["q /Fm1 Do Q"]);
        let font = helvetica_of(&p);
        let prop = (p.doc)
            .add_object(dictionary! { "ActualText" => Object::string_literal("ZebraSecretAlt") });
        let res = dictionary! {
            "Font" => dictionary! { "F1" => font },
            "Properties" => dictionary! { "P1" => prop },
        };
        let span = "/Span /P1 BDC BT /F1 20 Tf 30 100 Td (Zebra) Tj ET EMC \
                    BT /F1 20 Tf 30 40 Td (Kept) Tj ET";
        let mut top = form_with(&mut p, span, Some(res));
        if nested {
            let xobjects = dictionary! { "XObject" => dictionary! { "Fm2" => top } };
            top = form_with(&mut p, "q /Fm2 Do Q", Some(xobjects));
        }
        add_res(
            &mut p,
            0,
            dictionary! { "XObject" => dictionary! { "Fm1" => top } },
        );
        common::seeds::pdf(&p);
        let marks = marks_on(&p, 0, "zebra");
        let rep = p.redact(&marks).unwrap();
        assert!(rep.left.is_empty(), "nested {nested}: {:?}", rep.left);
        let saved = Saved::of(&mut p);
        saved.assert_gone(&["ZebraSecretAlt"]);
        saved.assert_kept(&["Kept"]);
    }
}

/// The same named property also marks text on a page that is left alone: it stays there,
/// the redacted page no longer lists it, and the report says where the removed alternative
/// text still is (it differs from the text shown, so only the alternative text finds it).
#[test]
fn alternative_text_still_used_elsewhere_is_kept_and_reported() {
    let mut p = make(&[
        "/Span /P1 BDC BT /F1 20 Tf 30 100 Td (Kudu) Tj ET EMC",
        "/Span /P1 BDC BT /F1 20 Tf 30 100 Td (Other) Tj ET EMC",
    ]);
    let prop = (p.doc)
        .add_object(dictionary! { "ActualText" => Object::string_literal("ZebraSecretAlt") });
    for page in 0..2 {
        add_res(
            &mut p,
            page,
            dictionary! { "Properties" => dictionary! { "P1" => prop } },
        );
    }
    common::seeds::pdf(&p);
    let marks = marks_on(&p, 0, "kudu");
    let rep = p.redact(&marks).unwrap();
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["Kudu"]);
    saved.assert_kept(&["ZebraSecretAlt", "Other"]);
    let page = p.doc.get_dictionary(p.page_ids()[0]).unwrap();
    let res = dict_at(&p, page.get(b"Resources").ok());
    let props = dict_at(&p, res.get(b"Properties").ok());
    assert!(!props.has(b"P1"), "page 1 still lists the property");
    assert!(
        rep.elsewhere.iter().any(|(q, places)| q == "zebrasecretalt"
            && places.iter().any(|w| w == "alternative text on page 2")),
        "{:?}",
        rep.elsewhere
    );
}

/// Text under a mark and text kept, drawn by a form.
const FORM_SECRET: &str = "BT /F1 20 Tf 30 100 Td (FormSecretZebra) Tj ET \
                           BT /F1 20 Tf 30 40 Td (Kept) Tj ET";

/// Redacts "FormSecretZebra" on page 1, then checks it's gone from the saved file and what
/// was kept is still there.
fn redact_form_secret(p: &mut Pdf) {
    common::seeds::pdf(p);
    let marks = marks_on(p, 0, "formsecretzebra");
    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    let saved = Saved::of(p);
    saved.assert_gone(&["FormSecretZebra"]);
    saved.assert_kept(&["Kept"]);
}

/// A form without resources of its own draws with the page's, which list the form itself.
/// Its redacted copy is given those resources: the original form must not stay in the file
/// through them (the audit's probe).
#[test]
fn a_redacted_form_drawing_with_the_page_s_resources_leaves_no_original() {
    let mut p = make(&["q /Fm1 Do Q"]);
    let fm = form_with(&mut p, FORM_SECRET, None);
    add_res(
        &mut p,
        0,
        dictionary! { "XObject" => dictionary! { "Fm1" => fm } },
    );
    redact_form_secret(&mut p);
}

/// A form whose own resources list the form itself (it doesn't draw it).
#[test]
fn a_redacted_form_listing_itself_leaves_no_original() {
    let mut p = make(&["q /Fm1 Do Q"]);
    let font = helvetica_of(&p);
    let fm = form_with(&mut p, FORM_SECRET, None);
    let res = dictionary! {
        "Font" => dictionary! { "F1" => font }, "XObject" => dictionary! { "Me" => fm },
    };
    let s = p.doc.get_object_mut(fm).unwrap().as_stream_mut().unwrap();
    s.dict.set("Resources", res);
    add_res(
        &mut p,
        0,
        dictionary! { "XObject" => dictionary! { "Fm1" => fm } },
    );
    redact_form_secret(&mut p);
}

/// Two forms, each listing the other in its resources (neither draws the other); the page
/// draws both, and only the first has text under the mark. The second is kept as it is, but
/// must not keep the first's original in the file.
#[test]
fn a_redacted_form_listed_by_a_kept_form_leaves_no_original() {
    let mut p = make(&["q /Fm1 Do Q q /Fm2 Do Q"]);
    let font = helvetica_of(&p);
    let first = form_with(&mut p, FORM_SECRET, None);
    let second = form_with(&mut p, "BT /F1 20 Tf 30 160 Td (Second form) Tj ET", None);
    for (me, other) in [(first, second), (second, first)] {
        let res = dictionary! {
            "Font" => dictionary! { "F1" => font.clone() },
            "XObject" => dictionary! { "Other" => other },
        };
        let s = p.doc.get_object_mut(me).unwrap().as_stream_mut().unwrap();
        s.dict.set("Resources", res);
    }
    add_res(
        &mut p,
        0,
        dictionary! { "XObject" => dictionary! { "Fm1" => first, "Fm2" => second } },
    );
    redact_form_secret(&mut p);
    Saved::of(&mut p).assert_kept(&["Second form"]);
}

/// Three forms, each drawing the next, none with resources of its own; the last has the
/// text under the mark.
#[test]
fn redacted_forms_nested_three_deep_leave_no_originals() {
    let mut p = make(&["q /Fm1 Do Q"]);
    let third = form_with(&mut p, FORM_SECRET, None);
    let second = form_with(&mut p, "q /Fm3 Do Q", None);
    let first = form_with(&mut p, "q /Fm2 Do Q", None);
    add_res(
        &mut p,
        0,
        dictionary! { "XObject" => dictionary! { "Fm1" => first, "Fm2" => second, "Fm3" => third } },
    );
    redact_form_secret(&mut p);
}

/// A picture drawn only inside a form nested in a form without resources of its own, wholly
/// under a mark: neither the picture nor the forms it was drawn from stay in the file.
#[test]
fn a_picture_drawn_only_inside_a_redacted_form_leaves_no_original() {
    let mut p = make(&["q /Fm1 Do Q"]);
    let font = helvetica_of(&p);
    let pic = marked_picture(&mut p, "ImgSecretZebra");
    let inner = form_with(
        &mut p,
        "q 140 0 0 10 30 95 cm /Im1 Do Q BT /F1 20 Tf 30 40 Td (Kept) Tj ET",
        Some(dictionary! {
            "Font" => dictionary! { "F1" => font }, "XObject" => dictionary! { "Im1" => pic },
        }),
    );
    let outer = form_with(&mut p, "q /Fm2 Do Q", None);
    add_res(
        &mut p,
        0,
        dictionary! { "XObject" => dictionary! { "Fm1" => outer, "Fm2" => inner } },
    );
    common::seeds::pdf(&p);
    let rep = p.redact(&[(0, [25.0, 90.0, 175.0, 110.0])]).unwrap();
    assert_eq!(rep.pages[0].images_cut, 1, "{:?}", rep.pages[0]);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["ImgSecretZebra"]);
    saved.assert_kept(&["Kept"]);
}

/// The control: the same form without resources, drawn on a page left alone too, stays for
/// that page as it was, and the report says where its text still is.
#[test]
fn a_redacted_form_another_page_draws_is_kept_for_it() {
    let mut p = make(&["q /Fm1 Do Q", "q /Fm1 Do Q"]);
    let fm = form_with(&mut p, FORM_SECRET, None);
    for page in 0..2 {
        add_res(
            &mut p,
            page,
            dictionary! { "XObject" => dictionary! { "Fm1" => fm } },
        );
    }
    common::seeds::pdf(&p);
    let before = drawn(&mut p, 1);
    let marks = marks_on(&p, 0, "formsecretzebra");
    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(!rep.elsewhere.is_empty());
    let saved = Saved::of(&mut p);
    saved.assert_kept(&["FormSecretZebra", "Kept"]);
    let mut back = Pdf::from_bytes(&saved.bytes, None).unwrap();
    assert!(!back.page_text(0).contains("FormSecretZebra"));
    assert_eq!(changed_outside(&before, &drawn(&mut back, 1), &[]), 0);
}

/// Redacted form A draws form B, kept as it is. B has no resources of its own, so it draws
/// with A's: a picture, a font and a marked-content property that A's own content never
/// names. `chain`: B only draws C (no resources either), which needs them. `shared`: page 2
/// draws B too.
fn needs_through_a_redacted_form(chain: bool, shared: bool) -> Pdf {
    let pages: Vec<&str> = match shared {
        false => vec!["q /A Do Q"],
        true => vec!["q /A Do Q", "q /B Do Q"],
    };
    let mut p = make(&pages);
    let times = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Times-Roman", "Encoding" => "WinAnsiEncoding",
    });
    let prop = (p.doc)
        .add_object(dictionary! { "ActualText" => Object::string_literal("RetainedAltText") });
    let pic = marked_picture(&mut p, "RetainedPicture");
    let needs = "/Span /PB BDC BT /FB 14 Tf 30 40 Td (Retained text) Tj ET EMC \
                 q 20 0 0 20 230 30 cm /ImB Do Q";
    let mut xobjects = dictionary! { "ImB" => pic };
    let b = match chain {
        false => form_with(&mut p, needs, None),
        true => {
            xobjects.set("C", form_with(&mut p, needs, None));
            form_with(&mut p, "q /C Do Q", None)
        }
    };
    let a = form_with(
        &mut p,
        "BT /F1 20 Tf 30 100 Td (FormSecretZebra) Tj ET q /B Do Q",
        None,
    );
    xobjects.set("A", a);
    xobjects.set("B", b);
    for page in 0..pages.len() {
        let extra = dictionary! {
            "XObject" => xobjects.clone(),
            "Font" => dictionary! { "FB" => times },
            "Properties" => dictionary! { "PB" => prop },
        };
        add_res(&mut p, page, extra);
    }
    common::seeds::pdf(&p);
    p
}

/// After A is redacted, B (and C) still find in A's copy the picture, font and property
/// they draw with, and the pages look the same outside the mark.
#[test]
fn a_redacted_form_s_copy_keeps_what_a_kept_form_inside_it_needs() {
    for (chain, shared) in [(false, false), (true, false), (false, true), (true, true)] {
        let case = format!("chain {chain}, shared {shared}");
        let mut p = needs_through_a_redacted_form(chain, shared);
        let before: Vec<Image> = (0..p.page_count()).map(|i| drawn(&mut p, i)).collect();
        assert!(
            ink(&before[0], [230, 150, 250, 170]) > 100,
            "{case}: B's picture is drawn"
        );
        let marks = marks_on(&p, 0, "formsecretzebra");
        let rep = p.redact(&marks).unwrap();
        assert!(rep.left.is_empty(), "{case}: {:?}", rep.left);
        let saved = Saved::of(&mut p);
        saved.assert_gone(&["FormSecretZebra"]);
        saved.assert_kept(&["RetainedAltText", "Retained text", "RetainedPicture"]);

        let mut back = Pdf::from_bytes(&saved.bytes, None).unwrap();
        for (i, was) in before.iter().enumerate() {
            let text = back.page_text(i);
            assert!(
                text.contains("Retained text"),
                "{case}, page {}: {text}",
                i + 1
            );
            let on: Vec<_> = marks.iter().filter(|m| m.0 == i).copied().collect();
            let changed = changed_outside(was, &drawn(&mut back, i), &on);
            assert_eq!(changed, 0, "{case}, page {}", i + 1);
        }
        // Every form drawing B draws it with resources of its own that hold all B needs.
        let mut drawing_b = 0;
        for (id, o) in &back.doc.objects {
            let Ok(s) = o.as_stream() else { continue };
            if s.dict.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"Form") {
                continue;
            }
            let data = s
                .decompressed_content()
                .unwrap_or_else(|_| s.content.clone());
            if !data.windows(5).any(|w| w == b"/B Do") {
                continue;
            }
            drawing_b += 1;
            let res = dict_at(&back, s.dict.get(b"Resources").ok());
            let mut needed = vec![(&b"XObject"[..], &b"B"[..]), (b"XObject", b"ImB")];
            needed.extend([(&b"Font"[..], &b"FB"[..]), (b"Properties", b"PB")]);
            if chain {
                needed.push((b"XObject", b"C"));
            }
            for (kind, name) in needed {
                let names = dict_at(&back, res.get(kind).ok());
                let found = (names.get(name).ok())
                    .is_some_and(|o| back.doc.dereference(o).is_ok_and(|(i, _)| i.is_some()));
                assert!(
                    found,
                    "{case}: object {id:?} draws B without /{} /{}",
                    String::from_utf8_lossy(kind),
                    String::from_utf8_lossy(name)
                );
            }
        }
        assert_eq!(drawing_b, 1, "{case}: only A's copy draws B");
    }
}

/// A page with "Zebra" to redact, drawing a tree of forms `depth` deep, each with resources
/// of its own and drawing `width` others.
fn forms_tree(depth: usize, width: usize) -> Pdf {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Zebra) Tj ET BT /F1 20 Tf 30 40 Td (Kept) Tj ET"]);
    fn tree(p: &mut Pdf, depth: usize, width: usize) -> ObjectId {
        let mut xobjects = Dictionary::new();
        let mut content = String::new();
        if depth > 0 {
            for i in 0..width {
                xobjects.set(format!("F{i}"), tree(p, depth - 1, width));
                content.push_str(&format!("q 0.5 0 0 0.5 0 0 cm /F{i} Do Q "));
            }
        }
        let res = dictionary! { "XObject" => xobjects };
        form_with(p, &content, Some(res))
    }
    let top = tree(&mut p, depth, width);
    let id = p.page_ids()[0];
    let content = p.doc.get_page_content(id);
    let more = [content.as_slice(), b" q /Top Do Q"].concat();
    let s = p.doc.add_object(Stream::new(Dictionary::new(), more));
    p.doc.get_dictionary_mut(id).unwrap().set("Contents", s);
    add_res(
        &mut p,
        0,
        dictionary! { "XObject" => dictionary! { "Top" => top } },
    );
    p
}

/// Following the drawings has a budget: a document needing one step more than it allows
/// is refused with nothing changed, and one needing exactly as many is redacted.
#[test]
fn drawings_too_complex_to_follow_are_refused_with_nothing_changed() {
    use limarepdf_core::{budget_seam, TooComplex};
    let marks = marks_on(&forms_tree(4, 3), 0, "zebra");
    // The steps the check before redacting takes: its budget is the first one made.
    let (done, taken) =
        budget_seam::with_limit(u64::MAX / 4, || forms_tree(4, 3).redact(&marks).map(|_| ()));
    done.unwrap();
    let needed = taken[0];
    assert!(
        needed > 121,
        "each of the 121 forms is a step at least: {taken:?}"
    );

    let mut p = forms_tree(4, 3);
    let (done, _) = budget_seam::with_limit(needed, || p.redact(&marks));
    done.unwrap();
    Saved::of(&mut p).assert_gone(&["Zebra"]);

    let mut p = forms_tree(4, 3);
    let before = p.state_before();
    let (done, taken) = budget_seam::with_limit(needed - 1, || p.redact(&marks));
    let e = done.unwrap_err();
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(too.limit, needed - 1, "{e:#}");
    assert_eq!(
        taken.len(),
        1,
        "only the check before redacting ran: {taken:?}"
    );
    assert!(taken[0] < needed, "{taken:?}");
    assert_eq!(p.state_since(&before), []);
}

#[test]
fn removes_annotations_touching_a_mark() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Zebra) Tj ET"]);
    let page = p.page_ids()[0];
    let link = |p: &mut Pdf, r: [i64; 4]| {
        p.doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link",
            "Rect" => r.iter().map(|&x| Object::Integer(x)).collect::<Vec<_>>(),
        })
    };
    let on = link(&mut p, [30, 95, 80, 120]);
    let off = link(&mut p, [200, 10, 250, 30]);
    p.doc.get_dictionary_mut(page).unwrap().set(
        "Annots",
        vec![Object::Reference(on), Object::Reference(off)],
    );
    let marks = p.find_redaction_marks("zebra").unwrap();
    let rep = p.redact(&marks).unwrap();
    assert_eq!(rep.pages[0].annots, 1);
    let left = p
        .doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(left, vec![Object::Reference(off)]);
}

/// A tagged page saying "Keep WALRUS-TEXT" (alternative text on its structure element and
/// that element's parent), with a comment touching WALRUS-TEXT (its pop-up and a reply away
/// from it), a filled text field touching it, a thumbnail and private data. Away from it: a
/// footer, a comment and a field, which must stay.
fn tagged_and_annotated() -> Pdf {
    let mut p = make(&[
        "/P <</MCID 0>> BDC BT /F1 20 Tf 30 100 Td (Keep WALRUS-TEXT) Tj ET EMC \
         BT /F1 10 Tf 30 40 Td (Footer kept) Tj ET",
    ]);
    let page = p.page_ids()[0];
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let rect = |r: [i64; 4]| r.iter().map(|&x| Object::Integer(x)).collect::<Vec<_>>();
    let note = |p: &mut Pdf, r: [i64; 4], text: &str| {
        p.doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Text", "Rect" => rect(r), "P" => page,
            "T" => Object::string_literal("Sam"), "Contents" => Object::string_literal(text),
        })
    };
    let touching = note(&mut p, [100, 100, 120, 120], "WALRUS-NOTE");
    let popup = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Popup", "Rect" => rect([150, 130, 290, 190]),
        "Parent" => touching, "P" => page,
    });
    p.doc
        .get_dictionary_mut(touching)
        .unwrap()
        .set("Popup", popup);
    let reply = note(&mut p, [250, 170, 270, 190], "WALRUS-REPLY");
    p.doc
        .get_dictionary_mut(reply)
        .unwrap()
        .set("IRT", touching);
    let away = note(&mut p, [10, 170, 30, 190], "Kept note");
    let field = |p: &mut Pdf, name: &str, value: &str, r: [i64; 4]| {
        let ap = p.doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => rect([0, 0, r[2] - r[0], r[3] - r[1]]),
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            format!("/Tx BMC BT /F1 10 Tf 2 5 Td ({value}) Tj ET EMC").into_bytes(),
        ));
        p.doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "Rect" => rect(r), "P" => page,
            "F" => 4,
            "T" => Object::string_literal(name), "V" => Object::string_literal(value),
            "DV" => Object::string_literal(value), "AP" => dictionary! { "N" => ap },
        })
    };
    let secret = field(&mut p, "secret", "WALRUS-FIELD", [200, 95, 260, 115]);
    let other = field(&mut p, "other", "Kept value", [10, 10, 80, 25]);
    let thumb = p.doc.add_object(Stream::new(
        dictionary! {
            "Width" => 1, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        b"WALRUS-THUMB".to_vec(),
    ));
    let root = p.doc.new_object_id();
    let div = p.doc.new_object_id();
    let elem = p.doc.add_object(dictionary! {
        "Type" => "StructElem", "S" => "P", "P" => div, "Pg" => page, "K" => 0,
        "Alt" => Object::string_literal("WALRUS-ALT"),
    });
    p.doc.objects.insert(
        div,
        dictionary! {
            "Type" => "StructElem", "S" => "Div", "P" => root, "K" => vec![Object::Reference(elem)],
            "E" => Object::string_literal("WALRUS-EXPANSION"),
        }
        .into(),
    );
    p.doc.objects.insert(
        root,
        dictionary! {
            "Type" => "StructTreeRoot", "K" => div,
            "ParentTree" => dictionary! {
                "Nums" => vec![Object::Integer(0), vec![Object::Reference(elem)].into()],
            },
        }
        .into(),
    );
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set(
        "Annots",
        vec![touching, popup, reply, away, secret, other]
            .into_iter()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    d.set("StructParents", 0);
    d.set("Thumb", thumb);
    d.set(
        "PieceInfo",
        dictionary! { "Editor" => dictionary! { "Private" => Object::string_literal("WALRUS-PIECE") } },
    );
    let c = p.doc.catalog_mut().unwrap();
    c.set("StructTreeRoot", root);
    c.set("MarkInfo", dictionary! { "Marked" => true });
    c.set(
        "AcroForm",
        dictionary! {
            "Fields" => vec![Object::Reference(secret), Object::Reference(other)],
            "DA" => Object::string_literal("/F1 10 Tf 0 g"),
            "DR" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
    );
    p
}

#[test]
fn the_saved_output_holds_nothing_of_the_marked_text_or_what_touches_it() {
    let mut p = tagged_and_annotated();
    Saved::of(&mut p).assert_kept(&[
        "WALRUS-TEXT",
        "WALRUS-NOTE",
        "WALRUS-REPLY",
        "WALRUS-FIELD",
        "WALRUS-THUMB",
        "WALRUS-ALT",
        "WALRUS-EXPANSION",
        "WALRUS-PIECE",
    ]);
    let marks = p.find_redaction_marks("walrus-text").unwrap();
    assert_eq!(marks.len(), 1);
    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert_eq!(
        rep.pages[0].annots, 3,
        "the comment, its reply and the field"
    );

    let saved = Saved::of(&mut p);
    saved.assert_gone(&["WALRUS"]);
    saved.assert_kept(&["Keep", "Footer kept", "Kept note", "Kept value"]);
    let fields: Vec<String> = saved.fields().into_iter().map(|f| f.name).collect();
    assert_eq!(fields, ["other"]);
    assert!(saved.stray_pages().is_empty(), "{:?}", saved.stray_pages());
    let q = Pdf::from_bytes(&saved.bytes, None).unwrap();
    assert!(q.page_text(0).contains("Keep"));
}

/// A tagged page saying "Keep WALRUS-TEXT", the second word in a structure element with
/// `ancestors` elements above it, the topmost with alternative text; `parent_tree` makes the
/// page's entry in the parent tree out of the leaf holding it.
fn tagged_deep(ancestors: usize, parent_tree: impl Fn(&mut Pdf, Object) -> Object) -> Pdf {
    let content = "BT /F1 20 Tf 30 100 Td (Keep ) Tj ET \
                   /Span <</MCID 0>> BDC BT /F1 20 Tf 90 100 Td (WALRUS-TEXT) Tj ET EMC";
    let mut p = make(&[content]);
    let page = p.page_ids()[0];
    let root = p.doc.new_object_id();
    let mut parent = root;
    let mut top = None;
    for i in 0..ancestors {
        let mut d = dictionary! { "Type" => "StructElem", "S" => "Div", "P" => parent };
        if i == 0 {
            d.set("Alt", Object::string_literal("WALRUS-TOP-ALT"));
        }
        let id = p.doc.add_object(d);
        if i > 0 {
            p.doc
                .get_dictionary_mut(parent)
                .unwrap()
                .set("K", vec![Object::Reference(id)]);
        }
        top.get_or_insert(id);
        parent = id;
    }
    let elem = p.doc.add_object(dictionary! {
        "Type" => "StructElem", "S" => "Span", "P" => parent, "Pg" => page, "K" => 0,
    });
    if parent != root {
        p.doc
            .get_dictionary_mut(parent)
            .unwrap()
            .set("K", vec![Object::Reference(elem)]);
    }
    let leaf =
        dictionary! { "Nums" => vec![Object::Integer(0), vec![Object::Reference(elem)].into()] };
    let leaf = parent_tree(&mut p, leaf.into());
    p.doc.objects.insert(
        root,
        dictionary! {
            "Type" => "StructTreeRoot", "K" => top.unwrap_or(elem), "ParentTree" => leaf,
        }
        .into(),
    );
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("StructParents", 0);
    let c = p.doc.catalog_mut().unwrap();
    c.set("StructTreeRoot", root);
    c.set("MarkInfo", dictionary! { "Marked" => true });
    p
}

/// The leaf `levels` kids down a number tree of its own objects.
fn nested(levels: usize) -> impl Fn(&mut Pdf, Object) -> Object {
    move |p: &mut Pdf, leaf: Object| {
        let mut node = p.doc.add_object(leaf);
        for _ in 0..levels {
            node = p
                .doc
                .add_object(dictionary! { "Kids" => vec![Object::Reference(node)] });
        }
        Object::Reference(node)
    }
}

/// The alternative text of every element above marked content goes, however many there
/// are.
#[test]
fn alternative_text_far_up_the_structure_tree_goes() {
    let mut p = tagged_deep(70, nested(0));
    Saved::of(&mut p).assert_kept(&["WALRUS-TOP-ALT"]);
    let marks = p.find_redaction_marks("walrus-text").unwrap();
    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["WALRUS"]);
    saved.assert_kept(&["Keep"]);
}

/// A parent tree nested too deep to look the page up in makes redaction refuse, with nothing
/// changed, rather than leave the alternative text of what is under the marks; just inside the
/// limit, and in a tree that lists itself as a kid, it is found and goes.
#[test]
fn a_parent_tree_nested_too_deep_is_refused_unchanged() {
    let mut p = tagged_deep(1, nested(33));
    let marks = p.find_redaction_marks("walrus-text").unwrap();
    // Serialising moves lopdf's object counter on, so copies are compared.
    let before = p.clone().snapshot().unwrap();
    let err = format!("{:#}", p.redact(&marks).unwrap_err());
    assert!(err.contains("nested more than 32 deep"), "{err}");
    assert_eq!(p.clone().snapshot().unwrap(), before);

    let looped = |p: &mut Pdf, leaf: Object| {
        let leaf = p.doc.add_object(leaf);
        let node = p.doc.new_object_id();
        let kids = vec![Object::Reference(node), Object::Reference(leaf)];
        p.doc
            .objects
            .insert(node, dictionary! { "Kids" => kids }.into());
        Object::Reference(node)
    };
    for mut p in [tagged_deep(1, nested(32)), tagged_deep(1, looped)] {
        let marks = p.find_redaction_marks("walrus-text").unwrap();
        let rep = p.redact(&marks).unwrap();
        assert!(rep.left.is_empty(), "{:?}", rep.left);
        Saved::of(&mut p).assert_gone(&["WALRUS"]);
    }
}

/// A page saying "Keep WALRUS-TEXT", with a widget over the second word `levels` fields below
/// the form's top-level field, which holds a value.
fn deep_widget(levels: usize) -> Pdf {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Keep WALRUS-TEXT) Tj ET"]);
    let page = p.page_ids()[0];
    let rect: Vec<Object> = [100, 98, 140, 112]
        .into_iter()
        .map(Object::Integer)
        .collect();
    let widget = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "Rect" => rect, "P" => page, "F" => 4,
    });
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", vec![Object::Reference(widget)]);
    let mut kid = widget;
    for d in (0..levels).rev() {
        let node = p.doc.add_object(dictionary! {
            "T" => Object::string_literal(format!("level-{d}")),
            "Kids" => vec![Object::Reference(kid)],
        });
        p.doc.get_dictionary_mut(kid).unwrap().set("Parent", node);
        kid = node;
    }
    let top = p.doc.get_dictionary_mut(kid).unwrap();
    top.set("FT", "Tx");
    top.set("V", Object::string_literal("WALRUS-FIELD-VALUE"));
    p.doc.catalog_mut().unwrap().set(
        "AcroForm",
        dictionary! { "Fields" => vec![Object::Reference(kid)] },
    );
    p
}

/// A parent tree too large to search for the page makes redaction refuse, with nothing changed,
/// rather than read on and on (audit R05); the same leaf ahead of the rest is found and goes.
#[test]
fn a_parent_tree_too_large_to_search_is_refused_unchanged() {
    let wide = |first: bool| {
        move |p: &mut Pdf, leaf: Object| {
            let mut kids: Vec<Object> = (0..100_000)
                .map(|_| Object::Dictionary(dictionary! {}))
                .collect();
            let leaf = Object::Reference(p.doc.add_object(leaf));
            if first {
                kids.insert(0, leaf);
            } else {
                kids.push(leaf);
            }
            Object::Reference(p.doc.add_object(dictionary! { "Kids" => kids }))
        }
    };
    let mut p = tagged_deep(1, wide(false));
    let marks = p.find_redaction_marks("walrus-text").unwrap();
    // Serialising moves lopdf's object counter on, so copies are compared.
    let before = p.clone().snapshot().unwrap();
    let err = format!("{:#}", p.redact(&marks).unwrap_err());
    assert!(err.contains("too large to search"), "{err}");
    assert_eq!(p.clone().snapshot().unwrap(), before);

    let mut p = tagged_deep(1, wide(true));
    let marks = p.find_redaction_marks("walrus-text").unwrap();
    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    Saved::of(&mut p).assert_gone(&["WALRUS"]);
}

/// A widget under a mark whose field is nested too deep to follow makes redaction refuse,
/// with nothing changed, rather than keep the field's value; just inside the limit the field
/// goes, and a deep field away from the marks doesn't stop redaction.
#[test]
fn a_widget_under_a_mark_in_a_form_nested_too_deep_is_refused_unchanged() {
    let mut p = deep_widget(34);
    let marks = p.find_redaction_marks("walrus-text").unwrap();
    // Serialising moves lopdf's object counter on, so copies are compared.
    let before = p.clone().snapshot().unwrap();
    let err = format!("{:#}", p.redact(&marks).unwrap_err());
    assert!(err.contains("nested more than 32 deep"), "{err}");
    assert_eq!(p.clone().snapshot().unwrap(), before);

    let marks = p.find_redaction_marks("keep").unwrap();
    p.redact(&marks).unwrap();
    Saved::of(&mut p).assert_kept(&["WALRUS-TEXT", "WALRUS-FIELD-VALUE"]);

    let mut p = deep_widget(32);
    let marks = p.find_redaction_marks("walrus-text").unwrap();
    let rep = p.redact(&marks).unwrap();
    assert_eq!(rep.pages[0].annots, 1);
    let saved = Saved::of(&mut p);
    saved.assert_gone(&["WALRUS"]);
    assert!(saved.fields().is_empty());
}

#[test]
fn works_on_turned_pages() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Keep Zebra Keep) Tj ET"]);
    p.rotate_pages(&[0], 90).unwrap();
    let marks = p.find_redaction_marks("zebra").unwrap();
    let rep = p.redact(&marks).unwrap();
    assert_eq!(rep.pages[0].chars, 5);
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    let text = p.page_text(0);
    assert!(
        !text.to_lowercase().contains("zebra") && text.contains("Keep"),
        "{text}"
    );
}

#[test]
fn rejects_marks_with_no_size_or_page() {
    let mut p = make(&[""]);
    assert!(p.redact(&[(0, [10.0, 10.0, 10.0, 50.0])]).is_err());
    assert!(p.redact(&[(3, [10.0, 10.0, 20.0, 50.0])]).is_err());
}

#[test]
fn marks_are_kept_until_applied() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Keep Zebra Keep) Tj ET"]);
    let boxes: Vec<[f32; 4]> = p
        .find_redaction_marks("zebra")
        .unwrap()
        .into_iter()
        .map(|m| m.1)
        .collect();
    p.add_redaction_mark(0, &boxes, "Sam").unwrap();
    assert!(p.apply_redactions().is_ok_and(|r| r.pages[0].chars == 5));
    assert!(p.redaction_marks().is_empty(), "applied marks are removed");
    assert!(!p.page_text(0).contains("Zebra"));
    assert!(p.apply_redactions().is_err(), "nothing left to apply");
}

#[test]
fn marks_survive_saving_and_are_not_flattened() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (Zebra) Tj ET"]);
    p.add_redaction_mark(
        0,
        &[[30.0, 80.0, 90.0, 105.0], [10.0, 10.0, 20.0, 20.0]],
        "",
    )
    .unwrap();
    let bytes = p.to_bytes().unwrap();
    let mut q = Pdf::from_bytes(&bytes, None).unwrap();
    let marks = q.redaction_marks();
    assert_eq!(marks.len(), 2);
    assert!(near(marks[0].1, [30.0, 80.0, 90.0, 105.0]), "{marks:?}");
    assert_eq!(q.annotations()[0].kind.label(), "Redaction mark");
    assert!(
        q.page_text(0).contains("Zebra"),
        "a mark removes nothing by itself"
    );
    q.flatten_annotations(None).unwrap();
    assert_eq!(q.redaction_marks().len(), 2);
}

/// Poppler and Ghostscript, reading the redacted output on their own:
/// `cargo test -p limarepdf-core --test redact -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_redacted_output_with_poppler_and_ghostscript() {
    let mut turned = make(&["BT /F1 20 Tf 30 100 Td (Keep Zebra Keep) Tj ET"]);
    turned.rotate_pages(&[0], 90).unwrap();
    // (document, what to redact, what each reader must find before and not after, what it
    // must still find after)
    let cases = [
        (
            make(&["BT /F1 20 Tf 30 100 Td (Name: Zebra Quokka here) Tj ET"]),
            "zebra quokka",
            &["Zebra", "Quokka"][..],
            &["Name:", "here"][..],
        ),
        (
            make(&["BT /F1 20 Tf 30 100 Td [(Sec) -200 (retX) 50 (Yzw)] TJ ET"]),
            "retX",
            &["retX"],
            &["Sec", "Yzw"],
        ),
        (turned, "zebra", &["Zebra"], &["Keep"]),
        (
            tagged_and_annotated(),
            "walrus-text",
            &["WALRUS-TEXT", "WALRUS-FIELD"],
            &["Keep", "Footer kept", "Kept value"],
        ),
    ];
    for (mut p, query, gone, kept) in cases {
        let before = p.to_bytes().unwrap();
        let marks = p.find_redaction_marks(query).unwrap();
        assert!(p.redact(&marks).unwrap().left.is_empty());
        let after = p.to_bytes().unwrap();
        for (tool, read) in [
            ("pdftotext", tools::pdftotext as fn(&[u8]) -> String),
            ("gs", tools::gs_text),
        ] {
            let (was, is) = (read(&before), read(&after));
            for w in gone {
                // Poppler doesn't draw form fields.
                if tool == "pdftotext" && w.ends_with("FIELD") {
                    continue;
                }
                assert!(was.contains(w), "{tool} finds {w} before: {was}");
                assert!(!is.contains(w), "{tool} finds {w} after: {is}");
            }
            for w in kept {
                if tool == "pdftotext" && w.starts_with("Kept value") {
                    continue;
                }
                assert!(is.contains(w), "{tool} doesn't find {w} after: {is}");
            }
        }
    }

    // Hidden text, which poppler reads and Ghostscript doesn't draw.
    let mut p = make(&[
        "/Span <</ActualText (Zebra)>> BDC BT 3 Tr /F1 20 Tf 30 100 Td (Zebra) Tj ET EMC \
         BT /F1 20 Tf 30 30 Td (Caption) Tj ET",
    ]);
    assert!(tools::pdftotext(&p.to_bytes().unwrap()).contains("Zebra"));
    let marks = p.find_redaction_marks("zebra").unwrap();
    p.redact(&marks).unwrap();
    let text = tools::pdftotext(&p.to_bytes().unwrap());
    assert!(
        !text.contains("Zebra") && text.contains("Caption"),
        "{text}"
    );

    // A form both pages draw, redacted on the first only.
    let mut p = shared_form();
    let marks: Vec<_> = p
        .find_redaction_marks("walrus")
        .unwrap()
        .into_iter()
        .filter(|m| m.0 == 0)
        .collect();
    p.redact(&marks).unwrap();
    let text = tools::pdftotext(&p.to_bytes().unwrap());
    // pdftotext ends each page with a form feed.
    let pages: Vec<&str> = text.split('\x0c').collect();
    assert!(
        !pages[0].contains("Walrus") && pages[0].contains("Secret"),
        "{text}"
    );
    assert!(pages[1].contains("Secret Walrus"), "{text}");

    // An area over a character of a sparse CMap, where poppler puts it: poppler then reads
    // the characters around it where they were, and nothing under the area.
    let words = poppler_words;
    let mut p = sparse_cmap();
    let before = words(&p.to_bytes().unwrap());
    assert_eq!(
        before,
        [("C".into(), 10.0, 30.0), ("AD".into(), 50.0, 80.0)],
        "poppler before"
    );
    assert!(p
        .redact(&[(0, [49.0, 80.0, 61.0, 108.0])])
        .unwrap()
        .left
        .is_empty());
    let after = words(&p.to_bytes().unwrap());
    assert_eq!(
        after,
        [("C".into(), 10.0, 30.0), ("D".into(), 60.0, 80.0)],
        "poppler after"
    );
}

/// The words poppler reads on the first page, each with its left and right edges.
fn poppler_words(bytes: &[u8]) -> Vec<(String, f32, f32)> {
    let html = tools::run("pdftotext", &["-bbox", "{}", "-"], bytes);
    let attr = |w: &str, k: &str| -> f32 {
        let v = &w[w.find(&format!("{k}=\"")).unwrap() + k.len() + 2..];
        v[..v.find('"').unwrap()].parse().unwrap()
    };
    html.split("<word ")
        .skip(1)
        .map(|w| {
            let text = &w[w.find('>').unwrap() + 1..w.find("</word>").unwrap()];
            (text.to_string(), attr(w, "xMin"), attr(w, "xMax"))
        })
        .collect()
}

/// The runs of columns of `img` with ink between rows `rows` (from and to).
fn inked(img: &Image, rows: [usize; 2]) -> Vec<[usize; 2]> {
    let mut out: Vec<[usize; 2]> = Vec::new();
    for x in 0..img.width {
        if ink(img, [x, rows[0], x + 1, rows[1]]) == 0 {
            continue;
        }
        match out.last_mut() {
            Some(r) if r[1] == x => r[1] = x + 1,
            _ => out.push([x, x + 1]),
        }
    }
    out
}

/// PB-100 read back by poppler: each letter of [`starts_a_mapped_pair`]'s strings marked
/// in turn, where LimarePDF reads it and where poppler does, and nothing poppler reads left
/// under the mark. Prints where each reads and draws the letters, before and after:
/// `cargo test -p limarepdf-core --test redact -- --ignored cross_check_a_code --nocapture`.
#[test]
#[ignore]
fn cross_check_a_code_that_starts_a_mapped_pair_with_poppler() {
    // (the second fixture, the letter marked, from and to x)
    let cases = [
        (false, "LimarePDF's A", 10.0, 20.0),
        (false, "LimarePDF's B", 20.0, 40.0),
        (false, "LimarePDF's D", 40.0, 60.0),
        (false, "poppler's unmapped pair", 10.0, 30.0),
        (false, "poppler's D", 30.0, 50.0),
        (true, "LimarePDF's A", 10.0, 20.0),
        (true, "LimarePDF's D", 20.0, 40.0),
        (true, "LimarePDF's B", 40.0, 60.0),
        (true, "poppler's unmapped pair", 10.0, 30.0),
        (true, "poppler's B", 30.0, 50.0),
    ];
    let rows = [80, 108];
    let mut left = Vec::new();
    for (second, what, x0, x1) in cases {
        let case = format!(
            "fixture {}, {what} (x {x0} to {x1})",
            1 + usize::from(second)
        );
        let mut p = starts_a_mapped_pair(second);
        let before = p.to_bytes().unwrap();
        let (ours, theirs) = (drawn(&mut p, 0), tools::pdftoppm(&before));
        println!("{case}");
        println!(
            "  before: LimarePDF reads {:?}, draws at {:?}; poppler reads {:?}, draws at {:?}",
            p.page_text(0),
            inked(&ours, rows),
            poppler_words(&before),
            inked(&theirs, rows)
        );
        let mark = [x0 + 1.0, 80.0, x1 - 1.0, 108.0];
        let rep = p.redact(&[(0, mark)]).unwrap();
        let after = p.to_bytes().unwrap();
        let read = poppler_words(&after);
        println!(
            "  after: left {:?}; shown {:02X?}; LimarePDF reads {:?}; poppler reads {:?}",
            rep.left,
            strings_shown(&p),
            p.page_text(0),
            read
        );
        let marks = [(0, mark)];
        println!(
            "  drawn differently outside the mark: LimarePDF {} px, poppler {} px",
            changed_outside(&ours, &drawn(&mut p, 0), &marks),
            changed_outside(&theirs, &tools::pdftoppm(&after), &marks)
        );
        let under: Vec<_> = (read.iter())
            .filter(|(_, a, b)| a.max(mark[0]) < b.min(mark[2]))
            .collect();
        if !under.is_empty() {
            left.push(format!("{case}: poppler reads {under:?}"));
        }
    }
    assert!(left.is_empty(), "{left:#?}");
}

/// Poppler reading a redacted form's copy that a kept form inside draws with: the kept
/// form's text is still read on every page (as its alternative text, which poppler reads in
/// its place, so the property is still found too), and the first page looks the same
/// outside the mark: `cargo test -p limarepdf-core --test redact -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_a_redacted_form_s_copy_with_poppler() {
    for (chain, shared) in [(false, false), (true, false), (false, true), (true, true)] {
        let case = format!("chain {chain}, shared {shared}");
        let mut p = needs_through_a_redacted_form(chain, shared);
        let before = p.to_bytes().unwrap();
        assert!(
            tools::pdftotext(&before).contains("FormSecretZebra"),
            "{case}"
        );
        let marks = marks_on(&p, 0, "formsecretzebra");
        assert!(p.redact(&marks).unwrap().left.is_empty(), "{case}");
        let after = p.to_bytes().unwrap();
        let text = tools::pdftotext(&after);
        // pdftotext ends each page with a form feed.
        let pages: Vec<&str> = text.split('\x0c').filter(|t| !t.is_empty()).collect();
        assert_eq!(pages.len(), p.page_count(), "{case}: {text}");
        for (i, t) in pages.iter().enumerate() {
            assert!(
                !t.contains("FormSecretZebra"),
                "{case}, page {}: {t}",
                i + 1
            );
            assert!(t.contains("RetainedAltText"), "{case}, page {}: {t}", i + 1);
        }
        let changed = changed_outside(&tools::pdftoppm(&before), &tools::pdftoppm(&after), &marks);
        assert_eq!(
            changed, 0,
            "{case}: poppler draws page 1 differently outside the mark"
        );
    }
}

/// PB-78 read back by poppler: "Secret" found and redacted leaves no t, whether its show
/// takes back its width or more:
/// `cargo test -p limarepdf-core --test redact -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_a_letter_whose_width_is_taken_back_with_poppler() {
    for take in [278, 500] {
        let mut p = taken_back(take);
        let before = tools::pdftotext(&p.to_bytes().unwrap());
        assert!(before.contains("Secret"), "{take}: {before}");
        let marks = p.find_redaction_marks("Secret").unwrap();
        assert!(p.redact(&marks).unwrap().left.is_empty(), "{take}");
        let after = tools::pdftotext(&p.to_bytes().unwrap());
        assert!(!after.contains('t'), "{take}: {after}");
        assert!(after.contains("here"), "{take}: {after}");
    }
}

/// A real public PDF: the Magick++ tutorial from ImageMagick's documentation, committed with its
/// licence (`tests/data/documents/README.md`). Its page 14 has text, a picture and drawings, and
/// its fonts are subsets, so its text isn't in the file as written and only a reader can tell
/// it's gone.
const TUTORIAL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/documents/Magick++_tutorial.pdf"
);

/// Redacting text through the whole tutorial and half of page 14's picture, read back by
/// poppler and Ghostscript:
/// `cargo test -p limarepdf-core --test redact -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_a_redacted_public_pdf_with_poppler_and_ghostscript() {
    let original = std::fs::read(TUTORIAL).unwrap_or_else(|e| panic!("{TUTORIAL}: {e}"));
    let gone = ["imprevisible", "DrawableFillColor"];
    let kept = ["anti-aliasing", "fully transparent", "DrawableStrokeColor"];
    let mut p = Pdf::from_bytes(&original, None).unwrap();
    let pages = p.page_count();
    for read in [tools::pdftotext as fn(&[u8]) -> String, tools::gs_text] {
        let text = read(&original);
        for w in gone.iter().chain(&kept) {
            assert!(text.contains(w), "{w} is in the tutorial");
        }
    }

    let mut marks = Vec::new();
    for w in gone {
        marks.extend(p.find_redaction_marks(w).unwrap());
    }
    let pic = p
        .page_objects(13)
        .unwrap()
        .into_iter()
        .find(|o| o.kind == ObjectKind::Image)
        .expect("page 14 has a picture")
        .bbox;
    marks.push((13, [pic[0], pic[1], (pic[0] + pic[2]) / 2.0, pic[3]]));
    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert!(rep.elsewhere.is_empty(), "{:?}", rep.elsewhere);
    assert_eq!(rep.pages.iter().map(|r| r.images_cut).sum::<usize>(), 1);

    let saved = Saved::of(&mut p);
    saved.assert_gone(&gone);
    assert!(saved.stray_pages().is_empty(), "{:?}", saved.stray_pages());
    assert_eq!(
        Pdf::from_bytes(&saved.bytes, None).unwrap().page_count(),
        pages
    );
    for (tool, read) in [
        ("pdftotext", tools::pdftotext as fn(&[u8]) -> String),
        ("gs", tools::gs_text),
    ] {
        let text = read(&saved.bytes);
        for w in gone {
            assert!(!text.contains(w), "{tool} finds {w}");
        }
        for w in kept {
            assert!(text.contains(w), "{tool} doesn't find {w}");
        }
    }

    // The picture as poppler reads it: black under the mark, as it was beside it.
    let (was, is) = (
        tools::pdfimages(&original, 14),
        tools::pdfimages(&saved.bytes, 14),
    );
    assert_eq!(is.len(), 2, "the picture and its soft mask");
    let was = image::load_from_memory(&was[0]).unwrap().to_rgb8();
    let is = image::load_from_memory(&is[0]).unwrap().to_rgb8();
    assert_eq!(was.dimensions(), is.dimensions());
    let (w, h) = is.dimensions();
    for y in 0..h {
        for x in 0..w / 2 - 2 {
            assert_eq!(is.get_pixel(x, y).0, [0, 0, 0], "at {x},{y}");
        }
        for x in w / 2 + 2..w {
            assert_eq!(is.get_pixel(x, y), was.get_pixel(x, y), "at {x},{y}");
        }
    }
}

/// The quarter rule (`CHAR_SHARE`), either side of it: a character goes when at least a
/// quarter of its box (its glyph's width, from 0.21 of the size below the baseline to 0.79
/// above) is under the marks, by area. At 60 points, A has its right 30 % under a mark and
/// goes, B its right 20 % and stays; C has its bottom 30 % under one and goes, D its
/// bottom 20 % and stays. The check afterwards agrees, and B and D stay where they were.
#[test]
fn a_character_goes_when_a_quarter_of_it_is_under_the_marks() {
    let mut p = make(&[
        "BT /F1 60 Tf 20 100 Td (A) Tj ET BT /F1 60 Tf 90 100 Td (B) Tj ET \
         BT /F1 60 Tf 160 100 Td (C) Tj ET BT /F1 60 Tf 230 100 Td (D) Tj ET",
    ]);
    let at = |p: &Pdf, s: &str| {
        let b = p.find_text_boxes(0, s).unwrap();
        assert_eq!(b.len(), 1, "{s}: {b:?}");
        b[0]
    };
    let [a, b, c, d] = ["A", "B", "C", "D"].map(|s| at(&p, s));
    // The reading's box is the redaction's: the glyph's width, the size high.
    for (s, x) in [("A", a), ("B", b), ("C", c), ("D", d)] {
        assert!((x[3] - x[1] - 60.0).abs() < 0.1, "{s}: {x:?}");
        assert!(x[3] > 100.0 && x[1] < 100.0, "{s}: {x:?}");
    }
    // A share of the box from its right edge, all its height and past it.
    let right = |x: [f32; 4], share: f32| {
        let w = x[2] - x[0];
        [x[2] - share * w, x[1] - 5.0, x[2] + 5.0, x[3] + 5.0]
    };
    // A share of the box from its bottom, all its width and past it.
    let bottom = |x: [f32; 4], share: f32| {
        let h = x[3] - x[1];
        [x[0] - 5.0, x[3] - share * h, x[2] + 5.0, x[3] + 5.0]
    };
    let marks = [
        (0, right(a, 0.3)),
        (0, right(b, 0.2)),
        (0, bottom(c, 0.3)),
        (0, bottom(d, 0.2)),
    ];
    let rep = p.redact(&marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert_eq!(rep.pages[0].chars, 2, "{:?}", rep.pages[0]);
    assert_eq!(p.page_text(0).replace([' ', '\n'], ""), "BD");
    assert!(near(at(&p, "B"), b) && near(at(&p, "D"), d));
}

/// Appends `objects` (number, body) to `out`, then a cross-reference section for them
/// and a trailer saying `size` objects, linked to the section at `prev` when there is one;
/// returns where the section starts.
fn revision(
    out: &mut Vec<u8>,
    objects: &[(usize, String)],
    size: usize,
    prev: Option<usize>,
) -> usize {
    let mut at: Vec<(usize, usize)> = Vec::new();
    for (n, body) in objects {
        at.push((*n, out.len()));
        out.extend_from_slice(format!("{n} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    at.sort();
    let xref = out.len();
    let mut table = String::from("xref\n0 1\n0000000000 65535 f \n");
    for (n, offset) in at {
        table.push_str(&format!("{n} 1\n{offset:010} 00000 n \n"));
    }
    let prev = prev.map(|p| format!(" /Prev {p}")).unwrap_or_default();
    table.push_str(&format!(
        "trailer\n<< /Size {size} /Root 1 0 R{prev} >>\nstartxref\n{xref}\n%%EOF\n"
    ));
    out.extend_from_slice(table.as_bytes());
    xref
}

/// A 300 by 200 pt page saved in two revisions, as an app that saves by appending leaves
/// it. In the first, the page draws "Keep QUOKKASECRET Keep" (object 4) and the draft line
/// "Draft QUOKKASECRET note" (object 6). The second, appended after it, has object 4 again
/// (the same line, and a blue square) and the page drawing it and "Footer kept" (object 7)
/// in place of the draft, which stays in the file with nothing using it.
fn two_revisions() -> Vec<u8> {
    let stream = |s: &str| format!("<< /Length {} >>\nstream\n{s}\nendstream", s.len());
    let page = |contents: &str| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents [{contents}] \
             /Resources << /Font << /F1 5 0 R >> >> >>"
        )
    };
    let line = "BT /F1 20 Tf 30 100 Td (Keep QUOKKASECRET Keep) Tj ET";
    let mut out = b"%PDF-1.7\n".to_vec();
    let first: [(usize, String); 6] = [
        (1, "<< /Type /Catalog /Pages 2 0 R >>".to_string()),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string()),
        (3, page("4 0 R 6 0 R")),
        (4, stream(line)),
        (
            5,
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                .to_string(),
        ),
        (
            6,
            stream("BT /F1 12 Tf 30 150 Td (Draft QUOKKASECRET note) Tj ET"),
        ),
    ];
    let xref = revision(&mut out, &first, 7, None);
    let second: [(usize, String); 3] = [
        (4, stream(&format!("{line}\n0 0 1 rg 30 20 40 20 re f"))),
        (3, page("4 0 R 7 0 R")),
        (7, stream("BT /F1 12 Tf 30 50 Td (Footer kept) Tj ET")),
    ];
    revision(&mut out, &second, 8, Some(xref));
    out
}

/// Journey B on a file saved by appending a revision: the marked text is also in the
/// earlier revision, in the page content it replaced and in a draft that nothing uses any
/// more. Opened (the latest revision read, not repaired), marked over the text and the
/// marks applied, the saved file holds no copy of the text in any form: not in its bytes as
/// written, nor in any stream decoded, used or not; what wasn't marked stays.
#[test]
fn an_earlier_revision_of_the_file_leaves_nothing_in_the_redacted_output() {
    let bytes = two_revisions();
    let copies = bytes.windows(12).filter(|w| w == b"QUOKKASECRET").count();
    assert_eq!(copies, 3, "the text in both revisions");
    let mut p = Pdf::from_bytes(&bytes, None).unwrap();
    assert!(!p.repaired(), "read by scanning, not by its revisions");
    let text = p.page_text(0);
    assert!(text.contains("Keep QUOKKASECRET Keep"), "{text:?}");
    assert!(
        text.contains("Footer kept") && !text.contains("Draft"),
        "not the latest revision: {text:?}"
    );
    // The draft is read with the rest, though nothing uses it.
    assert!(p.doc.objects.contains_key(&(6, 0)), "the draft wasn't read");
    let boxes: Vec<[f32; 4]> = (p.find_redaction_marks("quokkasecret").unwrap().into_iter())
        .map(|m| m.1)
        .collect();
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    p.add_redaction_mark(0, &boxes, "").unwrap();
    let rep = p.apply_redactions().unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    assert_eq!(rep.pages[0].chars, 12, "{:?}", rep.pages[0]);
    assert!(p.redaction_marks().is_empty());

    let saved = Saved::of(&mut p);
    saved.assert_gone(&["QUOKKASECRET", "QUOKKA", "SECRET", "Draft"]);
    saved.assert_kept(&["Keep", "Footer kept"]);
    let back = Pdf::from_bytes(&saved.bytes, None).unwrap();
    let text = back.page_text(0);
    let kept = text.contains("Keep") && text.contains("Footer kept");
    assert!(kept && !text.contains("QUOKKA"), "{text:?}");
}

// What redaction leaves in the saved file, and keeps for drawings left as they are, through
// resources a page inherits, patterns, soft masks, Type 3 fonts and resources pages share
// (CP-01–CP-03, S1-01 reopened). Each saved file is checked as written and reopened, and its
// pages drawn again outside the marks.

type Marks = Vec<(usize, [f32; 4])>;
/// A document and the marks to redact in it.
type Case = (Pdf, Marks);

/// What the report says when a drawing left as it was on page `page` (1-based) still draws
/// an original picture or form.
fn notice(page: usize) -> String {
    format!(
        "An original picture affected by redaction is still drawn elsewhere on page {page} and \
         may retain the removed content."
    )
}

/// Redacts `marks`, then checks that nothing is left under them, that the saved file holds
/// none of `gone` and all of `kept`, and that every page of it reopened draws as before
/// outside its marks. The report and the reopened file.
fn redacted(
    p: &mut Pdf,
    marks: &[(usize, [f32; 4])],
    gone: &[&str],
    kept: &[&str],
) -> (RedactReport, Pdf) {
    let before: Vec<Image> = (0..p.page_count()).map(|i| drawn(p, i)).collect();
    let rep = p.redact(marks).unwrap();
    assert!(rep.left.is_empty(), "{:?}", rep.left);
    let saved = Saved::of(p);
    saved.assert_gone(gone);
    saved.assert_kept(kept);
    let mut back = Pdf::from_bytes(&saved.bytes, None).unwrap();
    for (page, was) in before.iter().enumerate() {
        let on: Vec<_> = marks.iter().filter(|m| m.0 == page).copied().collect();
        let changed = changed_outside(was, &drawn(&mut back, page), &on);
        assert_eq!(
            changed,
            0,
            "page {} draws differently outside the marks",
            page + 1
        );
    }
    (rep, back)
}

/// Redacts `marks`, which the check afterwards must refuse, saying `why`.
fn refused(p: &mut Pdf, marks: &[(usize, [f32; 4])], why: &str) -> RedactReport {
    let rep = p.redact(marks).unwrap();
    assert!(rep.left.iter().any(|l| l == why), "{why}: {:?}", rep.left);
    rep
}

/// A form saying "FormSecretZebra" and "Kept", with page 1's font in resources of its own.
fn secret_form(p: &mut Pdf) -> ObjectId {
    let font = helvetica_of(p);
    let res = dictionary! { "Font" => dictionary! { "F1" => font } };
    form_with(p, FORM_SECRET, Some(res))
}

/// A page for each of `pages`, page 1 drawing a secret form as `/Fm1`: the document, the
/// form and the marks over the form's secret.
fn secret_form_page(pages: &[&str]) -> (Pdf, ObjectId, Marks) {
    let mut p = make(pages);
    let fm = secret_form(&mut p);
    add_res(
        &mut p,
        0,
        dictionary! { "XObject" => dictionary! { "Fm1" => fm } },
    );
    let marks = marks_on(&p, 0, "formsecretzebra");
    (p, fm, marks)
}

/// A tiling pattern one page in size, drawing `form` as `/Original`.
fn pattern_drawing(p: &mut Pdf, form: ObjectId) -> ObjectId {
    let bbox: Vec<Object> = [0, 0, 300, 200].into_iter().map(Object::Integer).collect();
    let d = dictionary! {
        "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
        "BBox" => bbox, "XStep" => 300, "YStep" => 200,
        "Resources" => dictionary! { "XObject" => dictionary! { "Original" => form } },
    };
    p.doc.add_object(Stream::new(d, b"/Original Do".to_vec()))
}

/// CP-01: a page marking text with a named property whose alternative text is listed by the
/// resources the page inherits from the page tree, not by its own. `sibling`: page 2
/// inherits the same resources and marks its text with the same property.
fn inherited_property(sibling: bool) -> Case {
    let mut pages = vec![
        "/Span /P1 BDC BT /F1 20 Tf 30 100 Td (Zebra) Tj ET EMC \
         BT /F1 20 Tf 30 40 Td (Kept) Tj ET",
    ];
    if sibling {
        pages.push("/Span /P1 BDC BT /F1 20 Tf 30 100 Td (Other) Tj ET EMC");
    }
    let mut p = make(&pages);
    let font = helvetica_of(&p);
    let prop = (p.doc)
        .add_object(dictionary! { "ActualText" => Object::string_literal("InheritedSecretAlt") });
    let parent = |p: &Pdf, page: ObjectId| {
        let d = p.doc.get_dictionary(page).unwrap();
        d.get(b"Parent").unwrap().as_reference().unwrap()
    };
    let tree = parent(&p, p.page_ids()[0]);
    for page in p.page_ids() {
        assert_eq!(parent(&p, page), tree);
        p.doc.get_dictionary_mut(page).unwrap().remove(b"Resources");
    }
    p.doc.get_dictionary_mut(tree).unwrap().set(
        "Resources",
        dictionary! {
            "Font" => dictionary! { "F1" => font },
            "Properties" => dictionary! { "P1" => prop },
        },
    );
    let marks = marks_on(&p, 0, "zebra");
    (p, marks)
}

#[test]
fn alternative_text_in_a_property_the_page_inherits_goes() {
    let (mut p, marks) = inherited_property(false);
    redacted(&mut p, &marks, &["InheritedSecretAlt", "Zebra"], &["Kept"]);
}

/// The control: page 2 still marks its text with the inherited property, so the property
/// stays for page 2, and the report says where the removed alternative text still is.
#[test]
fn alternative_text_in_an_inherited_property_another_page_uses_is_kept_and_reported() {
    let (mut p, marks) = inherited_property(true);
    let kept = ["InheritedSecretAlt", "Other", "Kept"];
    let (rep, back) = redacted(&mut p, &marks, &["Zebra"], &kept);
    assert!(
        rep.elsewhere
            .iter()
            .any(|(q, places)| q == "inheritedsecretalt"
                && places.iter().any(|w| w == "alternative text on page 2")),
        "{:?}",
        rep.elsewhere
    );
    let page = back.doc.get_dictionary(back.page_ids()[0]).unwrap();
    let res = dict_at(&back, page.get(b"Resources").ok());
    let props = dict_at(&back, res.get(b"Properties").ok());
    assert!(!props.has(b"P1"), "page 1 still lists the property");
}

/// CP-02: page 1 draws a secret form and lists a pattern drawing the same form, which
/// nothing paints with. `painted`: page 2 paints the whole page with the pattern.
fn pattern_drawing_the_form(painted: bool) -> Case {
    let mut pages = vec!["q /Fm1 Do Q"];
    if painted {
        pages.push("q /Pattern cs /Pt scn 0 0 300 200 re f Q");
    }
    let (mut p, fm, marks) = secret_form_page(&pages);
    let pt = pattern_drawing(&mut p, fm);
    for page in 0..pages.len() {
        add_res(
            &mut p,
            page,
            dictionary! { "Pattern" => dictionary! { "Pt" => pt } },
        );
    }
    (p, marks)
}

#[test]
fn a_redacted_form_an_unused_pattern_draws_leaves_no_original() {
    let (mut p, marks) = pattern_drawing_the_form(false);
    let (rep, _) = redacted(&mut p, &marks, &["FormSecretZebra"], &["Kept"]);
    assert!(
        rep.pictures_elsewhere.is_empty(),
        "{:?}",
        rep.pictures_elsewhere
    );
}

/// The control: page 2 paints with the pattern, so the original stays for it as it was,
/// and the report says so.
#[test]
fn a_redacted_form_a_pattern_another_page_paints_with_is_kept_for_it_and_reported() {
    let (mut p, marks) = pattern_drawing_the_form(true);
    let (rep, back) = redacted(&mut p, &marks, &[], &["FormSecretZebra", "Kept"]);
    assert!(!back.page_text(0).contains("FormSecretZebra"));
    assert_eq!(rep.pictures_elsewhere, [1]);
    assert_eq!(rep.picture_notices(), [notice(2)]);
}

/// CP-03: page 1 draws a picture under the mark, and beside it a form, left as it is, which
/// draws the same picture by the same name with the page's resources.
fn picture_also_drawn_by_a_kept_form(picture: fn(&mut Pdf) -> ObjectId) -> Case {
    let mut p = make(&["q 50 0 0 50 10 100 cm /Im1 Do Q q 1 0 0 1 110 100 cm /B Do Q"]);
    let im = picture(&mut p);
    let bbox: Vec<Object> = [0, 0, 50, 50].into_iter().map(Object::Integer).collect();
    let b = p.doc.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => bbox },
        b"q 50 0 0 50 0 0 cm /Im1 Do Q".to_vec(),
    ));
    add_res(
        &mut p,
        0,
        dictionary! { "XObject" => dictionary! { "Im1" => im, "B" => b } },
    );
    (p, vec![(0, [10.0, 50.0, 60.0, 100.0])])
}

/// The audit's picture: one white pixel.
fn white_pixel(p: &mut Pdf) -> ObjectId {
    p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        vec![255],
    ))
}

/// The made-up CMYK JPEG, which redaction can't cut, so it removes it whole.
fn cmyk_picture(p: &mut Pdf) -> ObjectId {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/images/cmyk-cyan-red.jpg"
    );
    p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 16, "Height" => 8,
            "ColorSpace" => "DeviceCMYK", "BitsPerComponent" => 8, "Filter" => "DCTDecode",
        },
        std::fs::read(path).unwrap(),
    ))
}

/// The form beside the mark draws the original picture as before, not the blacked-out copy,
/// and the report says once that page 1 still draws it. The problem is older than the
/// checkpoint that found it: before the copy was given the picture's name, the form drew
/// nothing.
#[test]
fn a_picture_a_kept_form_also_draws_stays_for_the_form_and_is_reported() {
    let (mut p, marks) = picture_also_drawn_by_a_kept_form(white_pixel);
    let (rep, _) = redacted(&mut p, &marks, &[], &[]);
    assert_eq!(rep.pages[0].images_cut, 1, "{:?}", rep.pages[0]);
    let mut samples: Vec<Vec<u8>> = (pictures(&mut p).into_iter()).map(|(_, _, s)| s).collect();
    samples.sort();
    assert_eq!(
        samples,
        [vec![0], vec![255]],
        "the blacked-out copy and the original"
    );
    assert_eq!(rep.pictures_elsewhere, [0]);
    assert_eq!(rep.picture_notices(), [notice(1)]);
}

/// As above, with a picture redaction can't read and so removes whole: it stays for the
/// form all the same.
#[test]
fn a_picture_removed_whole_that_a_kept_form_also_draws_stays_for_the_form() {
    let (mut p, marks) = picture_also_drawn_by_a_kept_form(cmyk_picture);
    let (rep, _) = redacted(&mut p, &marks, &[], &[]);
    assert_eq!(rep.pages[0].images, 1, "{:?}", rep.pages[0]);
    assert_eq!(images_in_file(&mut p), 1, "the original, for the form");
    assert_eq!(rep.pictures_elsewhere, [0]);
}

/// Pages 1 and 2 draw the same picture; page 1's is under the mark.
fn picture_shared_with_page_2() -> Case {
    let draw = "q 50 0 0 50 10 100 cm /Im1 Do Q";
    let mut p = make(&[draw, draw]);
    let im = marked_picture(&mut p, "SharedPicture");
    for page in 0..2 {
        add_res(
            &mut p,
            page,
            dictionary! { "XObject" => dictionary! { "Im1" => im } },
        );
    }
    (p, vec![(0, [10.0, 50.0, 60.0, 100.0])])
}

#[test]
fn a_picture_page_2_also_draws_stays_for_it_and_is_reported() {
    let (mut p, marks) = picture_shared_with_page_2();
    let (rep, _) = redacted(&mut p, &marks, &[], &["SharedPicture"]);
    assert_eq!(rep.pages[0].images_cut, 1, "{:?}", rep.pages[0]);
    assert_eq!(rep.pictures_elsewhere, [1]);
    assert_eq!(rep.picture_notices(), [notice(2)]);
}

/// Page 1 draws a secret form and lists a soft mask whose group draws the same form over
/// white. `used`: page 1 paints through the mask below the mark, where the form's "Kept"
/// shows as a hole.
fn soft_mask_drawing_the_form(used: bool) -> Case {
    let content = match used {
        false => "q /Fm1 Do Q",
        true => "q /Fm1 Do Q q /GS1 gs 0 0 1 rg 20 20 260 50 re f Q",
    };
    let (mut p, fm, marks) = secret_form_page(&[content]);
    let bbox: Vec<Object> = [0, 0, 300, 200].into_iter().map(Object::Integer).collect();
    let group = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => bbox,
            "Group" => dictionary! { "S" => "Transparency", "CS" => "DeviceGray" },
            "Resources" => dictionary! { "XObject" => dictionary! { "Original" => fm } },
        },
        b"q 1 g 0 0 300 200 re f Q /Original Do".to_vec(),
    ));
    let mask = dictionary! { "Type" => "Mask", "S" => "Luminosity", "G" => group };
    let gs = dictionary! { "Type" => "ExtGState", "SMask" => mask };
    add_res(
        &mut p,
        0,
        dictionary! { "ExtGState" => dictionary! { "GS1" => gs } },
    );
    (p, marks)
}

#[test]
fn a_redacted_form_an_unused_soft_mask_draws_leaves_no_original() {
    let (mut p, marks) = soft_mask_drawing_the_form(false);
    let (rep, _) = redacted(&mut p, &marks, &["FormSecretZebra"], &["Kept"]);
    assert!(
        rep.pictures_elsewhere.is_empty(),
        "{:?}",
        rep.pictures_elsewhere
    );
}

/// The control: the page paints through the mask, so its group keeps the original.
#[test]
fn a_redacted_form_a_soft_mask_in_use_draws_is_kept_for_it_and_reported() {
    let (mut p, marks) = soft_mask_drawing_the_form(true);
    let (rep, back) = redacted(&mut p, &marks, &[], &["FormSecretZebra", "Kept"]);
    assert!(!back.page_text(0).contains("FormSecretZebra"));
    assert_eq!(rep.pictures_elsewhere, [0]);
    assert_eq!(rep.picture_notices(), [notice(1)]);
}

/// Page 1 draws a secret form and lists a Type 3 font whose one glyph draws the same form,
/// 120 by 80 pt. `used`: page 1 shows the glyph below the mark.
fn type3_drawing_the_form(used: bool) -> Case {
    let content = match used {
        false => "q /Fm1 Do Q",
        true => "q /Fm1 Do Q BT /T3 20 Tf 150 10 Td (a) Tj ET",
    };
    let (mut p, fm, marks) = secret_form_page(&[content]);
    let glyph = (p.doc).add_object(Stream::new(
        Dictionary::new(),
        b"300 0 d0 /Original Do".to_vec(),
    ));
    let nums = |v: &[f32]| v.iter().map(|&x| Object::Real(x)).collect::<Vec<_>>();
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type3",
        "FontBBox" => nums(&[0.0, 0.0, 300.0, 200.0]),
        "FontMatrix" => nums(&[0.02, 0.0, 0.0, 0.02, 0.0, 0.0]),
        "CharProcs" => dictionary! { "g" => glyph },
        "Encoding" => dictionary! {
            "Type" => "Encoding", "Differences" => vec![Object::Integer(97), Object::Name(b"g".to_vec())],
        },
        "FirstChar" => 97, "LastChar" => 97, "Widths" => vec![Object::Integer(300)],
        "Resources" => dictionary! { "XObject" => dictionary! { "Original" => fm } },
    });
    add_res(
        &mut p,
        0,
        dictionary! { "Font" => dictionary! { "T3" => font } },
    );
    (p, marks)
}

#[test]
fn a_redacted_form_an_unused_type_3_font_draws_leaves_no_original() {
    let (mut p, marks) = type3_drawing_the_form(false);
    let (rep, _) = redacted(&mut p, &marks, &["FormSecretZebra"], &["Kept"]);
    assert!(
        rep.pictures_elsewhere.is_empty(),
        "{:?}",
        rep.pictures_elsewhere
    );
}

/// The control: the page shows the glyph, so the font keeps the original.
#[test]
fn a_redacted_form_a_type_3_glyph_in_use_draws_is_kept_for_it_and_reported() {
    let (mut p, marks) = type3_drawing_the_form(true);
    let (rep, back) = redacted(&mut p, &marks, &[], &["FormSecretZebra", "Kept"]);
    assert!(!back.page_text(0).contains("FormSecretZebra"));
    assert_eq!(rep.pictures_elsewhere, [0]);
    assert_eq!(rep.picture_notices(), [notice(1)]);
}

/// Page 1 draws a form saying "FormSecretZebra" twice, the second time 70 pt lower, and
/// only the first is under the mark. The second is drawn by the page itself, or
/// (`through_kept_form`) by a form left as it is.
fn form_drawn_inside_and_outside_a_mark(through_kept_form: bool) -> Case {
    let content = match through_kept_form {
        false => "q /Fm Do Q q 1 0 0 1 0 -70 cm /Fm Do Q",
        true => "q /Fm Do Q q /B Do Q",
    };
    let mut p = make(&[content]);
    let font = helvetica_of(&p);
    let rect = |r: [i64; 4]| r.into_iter().map(Object::Integer).collect::<Vec<_>>();
    let fm = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([20, 90, 290, 125]),
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
        b"BT /F1 20 Tf 30 100 Td (FormSecretZebra) Tj ET".to_vec(),
    ));
    let mut xobjects = dictionary! { "Fm" => fm };
    if through_kept_form {
        let b = p.doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0, 0, 300, 60]) },
            b"q 1 0 0 1 0 -70 cm /Fm Do Q".to_vec(),
        ));
        xobjects.set("B", b);
    }
    add_res(&mut p, 0, dictionary! { "XObject" => xobjects });
    let marks: Vec<_> = (marks_on(&p, 0, "formsecretzebra").into_iter())
        .filter(|m| m.1[3] < 130.0)
        .collect();
    assert_eq!(marks.len(), 1, "{marks:?}");
    (p, marks)
}

/// The drawing outside the mark still says "FormSecretZebra", from the original form kept
/// for it, and the report says so.
#[test]
fn a_form_drawn_inside_and_outside_a_mark_keeps_its_original_outside() {
    for through_kept_form in [true, false] {
        let (mut p, marks) = form_drawn_inside_and_outside_a_mark(through_kept_form);
        let (rep, back) = redacted(&mut p, &marks, &[], &["FormSecretZebra"]);
        let text = back.page_text(0);
        assert_eq!(
            text.matches("FormSecretZebra").count(),
            1,
            "through a kept form {through_kept_form}: {text}"
        );
        assert_eq!(
            rep.pictures_elsewhere,
            [0],
            "through a kept form {through_kept_form}"
        );
    }
}

/// Pages 1 and 2 share one resource dictionary, by reference: page 1 draws a secret form,
/// which a pattern nothing paints with draws too; page 2 draws a picture of its own and text.
fn shared_resources_by_reference() -> Case {
    let mut p = make(&[
        "q /Fm1 Do Q",
        "q 50 0 0 50 100 50 cm /Im2 Do Q BT /F1 20 Tf 30 30 Td (Two) Tj ET",
    ]);
    let font = helvetica_of(&p);
    let fm = secret_form(&mut p);
    let im = marked_picture(&mut p, "PageTwoPicture");
    let pt = pattern_drawing(&mut p, fm);
    let res = p.doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font },
        "XObject" => dictionary! { "Fm1" => fm, "Im2" => im },
        "Pattern" => dictionary! { "Pt" => pt },
    });
    for page in p.page_ids() {
        p.doc
            .get_dictionary_mut(page)
            .unwrap()
            .set("Resources", res);
    }
    let marks = marks_on(&p, 0, "formsecretzebra");
    (p, marks)
}

/// The original goes from the shared dictionary, edited where it is: page 2 still uses that
/// dictionary, draws as before and keeps its picture.
#[test]
fn a_redacted_form_listed_by_resources_another_page_shares_leaves_no_original() {
    let (mut p, marks) = shared_resources_by_reference();
    let kept = ["Kept", "PageTwoPicture", "Two"];
    let (_, back) = redacted(&mut p, &marks, &["FormSecretZebra"], &kept);
    let page = back.doc.get_dictionary(back.page_ids()[1]).unwrap();
    let res = page.get(b"Resources").unwrap();
    assert!(matches!(res, Object::Reference(_)), "{res:?}");
    let xobjects = dict_at(&back, dict_at(&back, Some(res)).get(b"XObject").ok());
    assert!(
        xobjects.has(b"Im2") && !xobjects.has(b"Fm1"),
        "{xobjects:?}"
    );
}

// Other ways the file can keep what redaction removed, which redaction doesn't cut: it
// refuses instead, saying which (A3).

/// A tag tree whose one element's marked content is in `form`, drawn on page 1.
fn tag_tree_pointing_at(p: &mut Pdf, form: ObjectId) {
    let page = p.page_ids()[0];
    let root = p.doc.new_object_id();
    let mcr = dictionary! { "Type" => "MCR", "Pg" => page, "Stm" => form, "MCID" => 0 };
    let elem = p.doc.add_object(dictionary! {
        "Type" => "StructElem", "S" => "P", "P" => root, "Pg" => page,
        "K" => vec![Object::Dictionary(mcr)],
    });
    p.doc.objects.insert(
        root,
        dictionary! { "Type" => "StructTreeRoot", "K" => elem }.into(),
    );
    let c = p.doc.catalog_mut().unwrap();
    c.set("StructTreeRoot", root);
    c.set("MarkInfo", dictionary! { "Marked" => true });
}

#[test]
fn a_redacted_form_the_tag_tree_points_at_is_refused() {
    let (mut p, fm, marks) = secret_form_page(&["q /Fm1 Do Q"]);
    tag_tree_pointing_at(&mut p, fm);
    let why = "removed content may still be in the file, through the tag tree";
    refused(&mut p, &marks, why);
}

/// A link touching the mark goes, but the tag tree still points at it.
#[test]
fn a_removed_link_the_tag_tree_points_at_is_refused() {
    let mut p = make(&["BT /F1 20 Tf 30 100 Td (LinkZebra) Tj ET \
                        BT /F1 20 Tf 30 40 Td (Kept) Tj ET"]);
    let page = p.page_ids()[0];
    let rect: Vec<Object> = [28, 95, 120, 120]
        .into_iter()
        .map(Object::Integer)
        .collect();
    let uri = Object::string_literal("https://example.com/LinkSecretZebra");
    let link = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link", "Rect" => rect, "P" => page,
        "A" => dictionary! { "S" => "URI", "URI" => uri }, "StructParent" => 1,
    });
    let root = p.doc.new_object_id();
    let objr = dictionary! { "Type" => "OBJR", "Obj" => link, "Pg" => page };
    let elem = p.doc.add_object(dictionary! {
        "Type" => "StructElem", "S" => "Link", "P" => root, "Pg" => page,
        "K" => vec![Object::Dictionary(objr)],
    });
    let nums = vec![Object::Integer(1), Object::Reference(elem)];
    p.doc.objects.insert(
        root,
        dictionary! {
            "Type" => "StructTreeRoot", "K" => elem,
            "ParentTree" => dictionary! { "Nums" => nums },
        }
        .into(),
    );
    (p.doc.get_dictionary_mut(page).unwrap()).set("Annots", vec![Object::Reference(link)]);
    let c = p.doc.catalog_mut().unwrap();
    c.set("StructTreeRoot", root);
    c.set("MarkInfo", dictionary! { "Marked" => true });
    let marks = marks_on(&p, 0, "linkzebra");
    let why = "removed content may still be in the file, through the tag tree";
    let rep = refused(&mut p, &marks, why);
    assert_eq!(rep.pages[0].annots, 1, "{:?}", rep.pages[0]);
}

#[test]
fn a_redacted_form_another_page_s_private_data_points_at_is_refused() {
    let pages = ["q /Fm1 Do Q", "BT /F1 20 Tf 30 100 Td (Two) Tj ET"];
    let (mut p, fm, marks) = secret_form_page(&pages);
    let piece = dictionary! { "Editor" => dictionary! { "Private" => fm } };
    let page = p.page_ids()[1];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("PieceInfo", piece);
    let why = "removed content may still be in the file, through private application data";
    refused(&mut p, &marks, why);
}

/// A key of the catalog's own, which LimarePDF doesn't know, points at the original form or
/// at the page content redaction replaced.
#[test]
fn removed_content_another_part_of_the_file_points_at_is_refused() {
    for to_content in [false, true] {
        let (mut p, target, marks) = match to_content {
            false => secret_form_page(&["q /Fm1 Do Q"]),
            true => {
                let p = make(&[FORM_SECRET]);
                let page = p.doc.get_dictionary(p.page_ids()[0]).unwrap();
                let content = page.get(b"Contents").unwrap().as_reference().unwrap();
                let marks = marks_on(&p, 0, "formsecretzebra");
                (p, content, marks)
            }
        };
        p.doc.catalog_mut().unwrap().set("ZzPrivate", target);
        let why = "removed content may still be in the file, through another part of the file";
        refused(&mut p, &marks, why);
    }
}

/// Page 2 draws a form that can't be read, which lists the original: whether it draws it
/// can't be told.
#[test]
fn a_redacted_form_a_drawing_that_can_t_be_read_lists_is_refused() {
    let (mut p, fm, marks) = secret_form_page(&["q /Fm1 Do Q", "q /Bad Do Q"]);
    let bbox: Vec<Object> = [0, 0, 300, 200].into_iter().map(Object::Integer).collect();
    let bad = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => bbox, "Filter" => "JBIG2Decode",
            "Resources" => dictionary! { "XObject" => dictionary! { "Original" => fm } },
        },
        b"not a drawing".to_vec(),
    ));
    add_res(
        &mut p,
        1,
        dictionary! { "XObject" => dictionary! { "Bad" => bad } },
    );
    let why = "a drawing on page 2 couldn't be read, so removed content may still be in the file";
    refused(&mut p, &marks, why);
}

/// Poppler drawing every page of the files above, redacted, as before outside the marks,
/// and no longer reading on page 1 what was removed there:
/// `cargo test -p limarepdf-core --test redact -- --ignored cross_check`.
#[test]
#[ignore]
fn cross_check_what_redaction_keeps_for_drawings_left_as_they_are_with_poppler() {
    // (what, the case, how many secrets poppler no longer reads on page 1)
    let cases = [
        ("an inherited property", inherited_property(false), 1),
        (
            "an inherited property page 2 uses",
            inherited_property(true),
            1,
        ),
        ("an unused pattern", pattern_drawing_the_form(false), 1),
        (
            "a pattern page 2 paints with",
            pattern_drawing_the_form(true),
            1,
        ),
        (
            "a picture a kept form draws",
            picture_also_drawn_by_a_kept_form(white_pixel),
            0,
        ),
        (
            "a picture removed whole a kept form draws",
            picture_also_drawn_by_a_kept_form(cmyk_picture),
            0,
        ),
        ("a picture page 2 draws", picture_shared_with_page_2(), 0),
        ("an unused soft mask", soft_mask_drawing_the_form(false), 1),
        // Poppler reads the text a soft mask draws, so it still reads the original kept for
        // the mask: what the report's notice for page 1 warns of.
        ("a soft mask in use", soft_mask_drawing_the_form(true), 0),
        ("an unused Type 3 font", type3_drawing_the_form(false), 1),
        ("a Type 3 font in use", type3_drawing_the_form(true), 1),
        (
            "a form inside and outside the mark",
            form_drawn_inside_and_outside_a_mark(false),
            1,
        ),
        (
            "a form inside the mark and in a kept form",
            form_drawn_inside_and_outside_a_mark(true),
            1,
        ),
        (
            "resources shared by reference",
            shared_resources_by_reference(),
            1,
        ),
    ];
    // pdftotext ends each page with a form feed; it reads alternative text in place of the
    // text it stands for.
    let secrets = |bytes: &[u8]| {
        let text = tools::pdftotext(bytes);
        let first = text.split('\x0c').next().unwrap_or_default().to_string();
        first.matches("Zebra").count() + first.matches("InheritedSecretAlt").count()
    };
    for (case, (mut p, marks), removed) in cases {
        let before = p.to_bytes().unwrap();
        let rep = p.redact(&marks).unwrap();
        assert!(rep.left.is_empty(), "{case}: {:?}", rep.left);
        let after = p.to_bytes().unwrap();
        for page in 0..p.page_count() {
            let on: Vec<_> = marks.iter().filter(|m| m.0 == page).copied().collect();
            let was = tools::pdftoppm_page(&before, page + 1);
            let is = tools::pdftoppm_page(&after, page + 1);
            assert_eq!(
                changed_outside(&was, &is, &on),
                0,
                "{case}: poppler draws page {} differently outside the marks",
                page + 1
            );
        }
        let (was, is) = (secrets(&before), secrets(&after));
        assert_eq!(is + removed, was, "{case}: poppler reads on page 1");
    }
}

// What the objects redaction removes point to (S1-26, A-01): alternative text and a removed
// comment's text stored as objects of their own, a removed comment's appearances and the
// pictures they draw, and a blacked-out picture's original soft mask and alternate version.
// Each goes with what held it; a route still reaching one is refused, as a route reaching
// what held it is.

/// A second reference to something redaction removed.
#[derive(Clone, Copy, Debug)]
enum Route {
    /// A key of the catalog's own.
    Catalog,
    /// Page 2's private application data.
    PrivateData,
    /// Another tag-tree element's alternative text.
    TagTree,
}

impl Route {
    const ALL: [Route; 3] = [Route::Catalog, Route::PrivateData, Route::TagTree];

    /// What the refusal calls the route.
    fn says(self) -> &'static str {
        match self {
            Route::Catalog => "another part of the file",
            Route::PrivateData => "private application data",
            Route::TagTree => "the tag tree",
        }
    }

    /// Points at `target` this way.
    fn point_at(self, p: &mut Pdf, target: ObjectId) {
        match self {
            Route::Catalog => p.doc.catalog_mut().unwrap().set("ZzPrivate", target),
            Route::PrivateData => {
                let piece = dictionary! { "Editor" => dictionary! { "Private" => target } };
                let page = p.page_ids()[1];
                (p.doc.get_dictionary_mut(page).unwrap()).set("PieceInfo", piece);
            }
            Route::TagTree => tag_with_alt(p, target),
        }
    }
}

/// A tag-tree element of its own whose alternative text is `text`; the tree is made when the
/// file has none.
fn tag_with_alt(p: &mut Pdf, text: ObjectId) {
    let root = match p.doc.catalog().unwrap().get(b"StructTreeRoot") {
        Ok(o) => o.as_reference().unwrap(),
        Err(_) => {
            let k: Vec<Object> = Vec::new();
            let root = (p.doc).add_object(dictionary! { "Type" => "StructTreeRoot", "K" => k });
            let c = p.doc.catalog_mut().unwrap();
            c.set("StructTreeRoot", root);
            c.set("MarkInfo", dictionary! { "Marked" => true });
            root
        }
    };
    let elem = p.doc.add_object(dictionary! {
        "Type" => "StructElem", "S" => "Figure", "P" => root, "ActualText" => text,
    });
    let d = p.doc.get_dictionary_mut(root).unwrap();
    let k = d.get_mut(b"K").unwrap().as_array_mut().unwrap();
    k.push(Object::Reference(elem));
}

/// A case: what it is, the document, its marks and the route still reaching what they remove.
type Routed = (String, Pdf, Marks, Route);

/// Redacts each case, which the check afterwards must refuse, naming its route. Panics
/// listing every case it didn't refuse, with what it said.
fn all_refused(cases: Vec<Routed>) {
    let mut not = Vec::new();
    for (what, mut p, marks, route) in cases {
        let why = format!(
            "removed content may still be in the file, through {}",
            route.says()
        );
        let rep = p.redact(&marks).unwrap();
        if !rep.left.contains(&why) {
            not.push(format!("{what}, through {}: {:?}", route.says(), rep.left));
        }
    }
    assert!(not.is_empty(), "not refused:\n{}", not.join("\n"));
}

/// Where the alternative text a mark takes off is held.
#[derive(Clone, Copy, Debug)]
enum AltIn {
    /// A property written in the page's `/Properties`.
    DirectProperty,
    /// A property stored as an object of its own.
    PropertyObject,
    /// A property stored as an object of its own, the text written in it.
    TextInPropertyObject,
    /// The tag-tree element of the marked content.
    TagTree,
}

/// What page 2 has (besides the tag tree's text, a property only).
#[derive(Clone, Copy, PartialEq)]
enum Two {
    /// "Two", nothing marked.
    Text,
    /// "Other", marked with a property of its own holding the same text, written as page 1's.
    OwnProperty,
    /// "Other", marked with page 1's property itself.
    SameProperty,
}

/// Page 1 marks "Zebra" with alternative text, "IndirectAltSecretZebra", held as `held`
/// says, and says "Kept"; page 2 has what `two` says. The text is stored as an object of
/// its own, or written in the property for `TextInPropertyObject`. The document, what holds
/// the text as an object of its own (the text, or that property) and the marks.
fn indirect_alt(held: AltIn, two: Two) -> (Pdf, ObjectId, Marks) {
    let span = match held {
        AltIn::TagTree => "/P <</MCID 0>> BDC",
        _ => "/Span /P1 BDC",
    };
    let one = format!(
        "{span} BT /F1 20 Tf 30 100 Td (Zebra) Tj ET EMC BT /F1 20 Tf 30 40 Td (Kept) Tj ET"
    );
    let second = match two {
        Two::Text => "BT /F1 20 Tf 30 100 Td (Two) Tj ET",
        _ => "/Span /P1 BDC BT /F1 20 Tf 30 100 Td (Other) Tj ET EMC",
    };
    let mut p = make(&[&one, second]);
    let text = Object::string_literal("IndirectAltSecretZebra");
    let page = p.page_ids()[0];
    let holder = match held {
        AltIn::TagTree => {
            let secret = p.doc.add_object(text);
            let root = p.doc.new_object_id();
            let elem = p.doc.add_object(dictionary! {
                "Type" => "StructElem", "S" => "Span", "P" => root, "Pg" => page, "K" => 0,
                "ActualText" => secret,
            });
            let nums = vec![Object::Integer(0), vec![Object::Reference(elem)].into()];
            p.doc.objects.insert(
                root,
                dictionary! {
                    "Type" => "StructTreeRoot", "K" => vec![Object::Reference(elem)],
                    "ParentTree" => dictionary! { "Nums" => nums },
                }
                .into(),
            );
            (p.doc.get_dictionary_mut(page).unwrap()).set("StructParents", 0);
            let c = p.doc.catalog_mut().unwrap();
            c.set("StructTreeRoot", root);
            c.set("MarkInfo", dictionary! { "Marked" => true });
            secret
        }
        _ => {
            let secret = match held {
                AltIn::TextInPropertyObject => None,
                _ => Some(p.doc.add_object(text.clone())),
            };
            let mut prop = || -> Object {
                let alt = secret.map_or(text.clone(), Object::from);
                let prop = dictionary! { "ActualText" => alt };
                match held {
                    AltIn::DirectProperty => prop.into(),
                    _ => p.doc.add_object(prop).into(),
                }
            };
            let first = prop();
            let props = match two {
                Two::Text => vec![first.clone()],
                Two::OwnProperty => vec![first.clone(), prop()],
                Two::SameProperty => vec![first.clone(), first.clone()],
            };
            for (page, prop) in props.into_iter().enumerate() {
                add_res(
                    &mut p,
                    page,
                    dictionary! { "Properties" => dictionary! { "P1" => prop } },
                );
            }
            secret.unwrap_or_else(|| first.as_reference().unwrap())
        }
    };
    let marks = marks_on(&p, 0, "zebra");
    common::seeds::pdf(&p);
    (p, holder, marks)
}

const ALT_IN: [AltIn; 4] = [
    AltIn::DirectProperty,
    AltIn::PropertyObject,
    AltIn::TextInPropertyObject,
    AltIn::TagTree,
];

/// Alternative text a mark takes off, stored as an object of its own and held by a property
/// written in the page's resources, by one stored as an object of its own or by the tag
/// tree, or written in a property stored as an object of its own, while a key of the
/// catalog, page 2's private data or another tag-tree element still points at what holds it
/// as an object of its own.
#[test]
fn removed_alternative_text_something_else_points_at_is_refused() {
    let mut cases = Vec::new();
    for held in ALT_IN {
        for route in Route::ALL {
            let (mut p, holder, marks) = indirect_alt(held, Two::Text);
            route.point_at(&mut p, holder);
            cases.push((format!("{held:?}"), p, marks, route));
        }
    }
    all_refused(cases);
}

/// The controls: with nothing else pointing at it, the text goes with what held it.
#[test]
fn removed_alternative_text_stored_as_an_object_of_its_own_goes() {
    for held in ALT_IN {
        let (mut p, _, marks) = indirect_alt(held, Two::Text);
        let gone = ["IndirectAltSecretZebra", "Zebra"];
        redacted(&mut p, &marks, &gone, &["Kept", "Two"]);
    }
}

/// The control: page 2's property holds the same text, so the text stays for page 2, and the
/// report says where the removed alternative text still is. (A property written in the
/// resources is told apart by what it says, so page 1's, the same as page 2's, stays too.)
#[test]
fn removed_alternative_text_a_property_page_2_uses_holds_is_kept_and_reported() {
    for held in [
        AltIn::DirectProperty,
        AltIn::PropertyObject,
        AltIn::TextInPropertyObject,
    ] {
        kept_for_page_2(held, Two::OwnProperty);
    }
}

/// The control: page 2 uses page 1's property itself, so the property stays for page 2,
/// and the report says where the removed alternative text still is.
#[test]
fn removed_alternative_text_in_the_property_page_2_uses_too_is_kept_and_reported() {
    for held in [AltIn::PropertyObject, AltIn::TextInPropertyObject] {
        kept_for_page_2(held, Two::SameProperty);
    }
}

/// Redacts page 1's "Zebra" in `indirect_alt(held, two)`, checking that the alternative
/// text is kept and reported as on page 2.
fn kept_for_page_2(held: AltIn, two: Two) {
    let (mut p, _, marks) = indirect_alt(held, two);
    let kept = ["IndirectAltSecretZebra", "Other", "Kept"];
    let (rep, _) = redacted(&mut p, &marks, &[], &kept);
    assert!(
        rep.elsewhere
            .iter()
            .any(|(q, places)| q == "indirectaltsecretzebra"
                && places.iter().any(|w| w == "alternative text on page 2")),
        "{held:?}: {:?}",
        rep.elsewhere
    );
}

/// What a removed comment holds as an object of its own.
#[derive(Clone, Copy, Debug)]
enum Held {
    /// Its appearance (`/AP /N`).
    Normal,
    /// Its appearance in the state shown (`/AP /N /On`, `/AS /On`).
    ByState,
    /// Its dictionary of appearances by state.
    StateObject,
    /// Its `/AP`.
    ApObject,
    /// Its appearance while pressed (`/AP /D`).
    Down,
    /// Its text (`/Contents`).
    Contents,
    /// Its rich text (`/RC`).
    RichText,
    /// The picture its appearance draws.
    Picture,
}

const HELD: [Held; 8] = [
    Held::Normal,
    Held::ByState,
    Held::StateObject,
    Held::ApObject,
    Held::Down,
    Held::Contents,
    Held::RichText,
    Held::Picture,
];

/// Page 1 says "Zebra" and "Kept", with a square comment over "Zebra": its appearance, held
/// as `held` says, draws "AppearanceSecretZebra" with page 1's font and a picture
/// "AppearancePictureSecretZebra", neither showing (the text invisible, the picture clipped
/// to nothing), and its text "CommentSecretZebra" and rich text "RichSecretZebra" are objects
/// of their own. Page 2 says "Two". The document, the object `held` names and the marks.
fn annotated(held: Held) -> (Pdf, ObjectId, Marks) {
    let mut p = make(&[
        "BT /F1 20 Tf 30 100 Td (Zebra) Tj ET BT /F1 20 Tf 30 40 Td (Kept) Tj ET",
        "BT /F1 20 Tf 30 100 Td (Two) Tj ET",
    ]);
    let page = p.page_ids()[0];
    let font = helvetica_of(&p);
    let picture = marked_picture(&mut p, "AppearancePictureSecretZebra");
    let mut look = |content: &str| {
        let bbox: Vec<Object> = [0, 0, 92, 25].into_iter().map(Object::Integer).collect();
        let res = dictionary! {
            "Font" => dictionary! { "F1" => font.clone() },
            "XObject" => dictionary! { "Im" => picture },
        };
        let d = dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => bbox, "Resources" => res };
        p.doc
            .add_object(Stream::new(d, content.as_bytes().to_vec()))
    };
    let secret = look(
        "BT 3 Tr /F1 10 Tf 2 8 Td (AppearanceSecretZebra) Tj ET \
         q 0 0 0 0 re W n 92 0 0 25 0 0 cm /Im Do Q",
    );
    let other = look("");
    let states = dictionary! { "On" => secret, "Off" => other };
    let (state_object, ap_object) = (
        p.doc.add_object(states.clone()),
        p.doc.add_object(dictionary! { "N" => secret }),
    );
    let ap: Object = match held {
        Held::ByState => dictionary! { "N" => states }.into(),
        Held::StateObject => dictionary! { "N" => state_object }.into(),
        Held::ApObject => ap_object.into(),
        Held::Down => dictionary! { "N" => other, "D" => secret }.into(),
        _ => dictionary! { "N" => secret }.into(),
    };
    let contents = (p.doc).add_object(Object::string_literal("CommentSecretZebra"));
    let rich = (p.doc).add_object(Object::string_literal("RichSecretZebra"));
    let rect: Vec<Object> = [28, 95, 120, 120]
        .into_iter()
        .map(Object::Integer)
        .collect();
    let mut annot = dictionary! {
        "Type" => "Annot", "Subtype" => "Square", "Rect" => rect, "P" => page, "AP" => ap,
        "Contents" => contents, "RC" => rich,
    };
    if matches!(held, Held::ByState | Held::StateObject) {
        annot.set("AS", "On");
    }
    let annot = p.doc.add_object(annot);
    (p.doc.get_dictionary_mut(page).unwrap()).set("Annots", vec![Object::Reference(annot)]);
    let target = match held {
        Held::StateObject => state_object,
        Held::ApObject => ap_object,
        Held::Contents => contents,
        Held::RichText => rich,
        Held::Picture => picture,
        _ => secret,
    };
    let marks = marks_on(&p, 0, "zebra");
    common::seeds::pdf(&p);
    (p, target, marks)
}

/// Adds to page 1, away from the mark, `annot` (without `/Rect`) over [200, 10, 292, 35].
fn add_annot_away(p: &mut Pdf, mut annot: Dictionary) {
    let page = p.page_ids()[0];
    let rect: Vec<Object> = [200, 10, 292, 35]
        .into_iter()
        .map(Object::Integer)
        .collect();
    annot.set("Rect", rect);
    annot.set("P", page);
    let id = p.doc.add_object(annot);
    let d = p.doc.get_dictionary_mut(page).unwrap();
    let list = d.get_mut(b"Annots").unwrap().as_array_mut().unwrap();
    list.push(Object::Reference(id));
}

/// A removed comment's appearance (shown, in the state shown, while pressed), its dictionary
/// of appearances by state, its `/AP`, its text, its rich text and the picture its
/// appearance draws, each an object of its own, which page 2's private data still points
/// at; and its appearance, which a key of the catalog does.
#[test]
fn what_a_removed_comment_held_that_something_else_points_at_is_refused() {
    let mut cases = Vec::new();
    for held in HELD {
        let (mut p, target, marks) = annotated(held);
        Route::PrivateData.point_at(&mut p, target);
        cases.push((format!("{held:?}"), p, marks, Route::PrivateData));
    }
    let (mut p, target, marks) = annotated(Held::Normal);
    Route::Catalog.point_at(&mut p, target);
    cases.push(("Normal".to_string(), p, marks, Route::Catalog));
    all_refused(cases);
}

/// The controls: with nothing else pointing at them, what the comment held goes with it,
/// and page 1's font, which its appearance used too, stays.
#[test]
fn what_a_removed_comment_held_goes_with_it() {
    let gone = [
        "AppearanceSecretZebra",
        "AppearancePictureSecretZebra",
        "CommentSecretZebra",
        "RichSecretZebra",
        "Zebra",
    ];
    for held in HELD {
        let (mut p, _, marks) = annotated(held);
        let (rep, back) = redacted(&mut p, &marks, &gone, &["Kept", "Two"]);
        assert!(
            rep.pictures_elsewhere.is_empty(),
            "{held:?}: {:?}",
            rep.pictures_elsewhere
        );
        let page = back.doc.get_dictionary(back.page_ids()[0]).unwrap();
        let res = dict_at(&back, page.get(b"Resources").ok());
        let fonts = dict_at(&back, res.get(b"Font").ok());
        let font = dict_at(&back, fonts.get(b"F1").ok());
        let name = font.get(b"BaseFont").and_then(|o| o.as_name()).ok();
        assert_eq!(name, Some(&b"Helvetica"[..]), "{held:?}");
    }
}

/// The control: a comment away from the mark shows the same appearance, so the appearance
/// and its picture stay for that comment, and the report says page 1 still draws them.
#[test]
fn a_removed_comment_s_appearance_a_comment_that_stays_shows_is_kept_and_reported() {
    let (mut p, look, marks) = annotated(Held::Normal);
    add_annot_away(
        &mut p,
        dictionary! { "Type" => "Annot", "Subtype" => "Square", "AP" => dictionary! { "N" => look } },
    );
    let gone = ["CommentSecretZebra", "RichSecretZebra"];
    let kept = [
        "AppearanceSecretZebra",
        "AppearancePictureSecretZebra",
        "Kept",
    ];
    let (rep, _) = redacted(&mut p, &marks, &gone, &kept);
    assert_eq!(rep.pictures_elsewhere, [0]);
    assert_eq!(rep.picture_notices(), [notice(1)]);
}

/// The control: a comment away from the mark has the same text object, so the text stays for
/// that comment.
#[test]
fn a_removed_comment_s_text_a_comment_that_stays_has_is_kept() {
    let (mut p, text, marks) = annotated(Held::Contents);
    add_annot_away(
        &mut p,
        dictionary! { "Type" => "Annot", "Subtype" => "Text", "Contents" => text },
    );
    let gone = ["AppearanceSecretZebra", "RichSecretZebra"];
    redacted(&mut p, &marks, &gone, &["CommentSecretZebra", "Kept"]);
}

/// The audit's picture: a red pixel and a blue one.
const RED_BLUE: [u8; 6] = [255, 0, 0, 0, 0, 255];
/// Its soft mask's samples.
const MASK: [u8; 2] = [200, 100];

/// A picture two pixels wide and one high: `samples` in `space`.
fn two_pixels(p: &mut Pdf, space: &str, samples: &[u8]) -> ObjectId {
    p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 2, "Height" => 1,
            "ColorSpace" => space, "BitsPerComponent" => 8,
        },
        samples.to_vec(),
    ))
}

/// Page 1 draws the audit's picture, 150 by 100 pt, which has an alternate version of the
/// same pixels and a soft mask; the mark covers its red pixel. Page 2 draws `page_2`. The
/// document, (the alternate version, the soft mask) and the marks.
fn picture_with_parts(page_2: &str) -> (Pdf, (ObjectId, ObjectId), Marks) {
    let mut p = make(&["q 150 0 0 100 0 0 cm /Im Do Q", page_2]);
    let alternate = two_pixels(&mut p, "DeviceRGB", &RED_BLUE);
    let mask = two_pixels(&mut p, "DeviceGray", &MASK);
    let original = two_pixels(&mut p, "DeviceRGB", &RED_BLUE);
    let alternates = vec![Object::Dictionary(
        dictionary! { "Image" => alternate, "DefaultForPrinting" => true },
    )];
    let s = p
        .doc
        .get_object_mut(original)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    s.dict.set("Alternates", alternates);
    s.dict.set("SMask", mask);
    add_res(
        &mut p,
        0,
        dictionary! { "XObject" => dictionary! { "Im" => original } },
    );
    common::seeds::pdf(&p);
    (p, (alternate, mask), vec![(0, [0.0, 100.0, 75.0, 200.0])])
}

/// The samples of every picture in the saved file, and how many of its pictures list
/// alternate versions.
fn samples_and_alternates(p: &mut Pdf) -> (Vec<Vec<u8>>, usize) {
    let samples = (pictures(p).into_iter()).map(|(_, _, s)| s).collect();
    let alternates = (Saved::of(p).doc.objects.values())
        .filter(|o| o.as_stream().is_ok_and(|s| s.dict.has(b"Alternates")))
        .count();
    (samples, alternates)
}

/// The audit's case: the blacked-out picture is written without the alternate version and
/// with its soft mask cut, so nothing in the saved file holds the original pixels.
#[test]
fn a_blacked_out_picture_s_alternate_version_and_soft_mask_go() {
    let (mut p, _, marks) = picture_with_parts("BT /F1 20 Tf 30 100 Td (Two) Tj ET");
    let (rep, _) = redacted(&mut p, &marks, &[], &["Two"]);
    assert_eq!(rep.pages[0].images_cut, 1, "{:?}", rep.pages[0]);
    let (samples, alternates) = samples_and_alternates(&mut p);
    let original = samples.iter().any(|s| *s == RED_BLUE || *s == MASK);
    assert!(!original && alternates == 0, "{samples:?}, {alternates}");
    let report = (&rep.left, &rep.elsewhere, &rep.pictures_elsewhere);
    assert!(
        report.0.is_empty() && report.1.is_empty() && report.2.is_empty(),
        "{report:?}"
    );
}

/// The control: page 2 draws the alternate version itself, so it stays for page 2, and the
/// report says page 2 still draws an original.
#[test]
fn a_blacked_out_picture_s_alternate_version_page_2_draws_is_kept_and_reported() {
    let (mut p, (alternate, _), marks) = picture_with_parts("q 150 0 0 100 0 0 cm /Alt Do Q");
    add_res(
        &mut p,
        1,
        dictionary! { "XObject" => dictionary! { "Alt" => alternate } },
    );
    let (rep, _) = redacted(&mut p, &marks, &[], &[]);
    let (samples, alternates) = samples_and_alternates(&mut p);
    assert!(
        samples.contains(&RED_BLUE.to_vec()) && alternates == 0,
        "{samples:?}, {alternates}"
    );
    assert_eq!(rep.pictures_elsewhere, [1]);
    assert_eq!(rep.picture_notices(), [notice(2)]);
}

/// The control (the user's answer of 2026-11-08, Q1 (a)): page 2 draws another picture with
/// the same soft mask, so the mask stays for that picture, and the report says page 2 still
/// draws an original.
#[test]
fn a_blacked_out_picture_s_soft_mask_a_picture_page_2_draws_shares_is_kept_and_reported() {
    let (mut p, (_, mask), marks) = picture_with_parts("q 150 0 0 100 0 0 cm /Green Do Q");
    let green = two_pixels(&mut p, "DeviceRGB", &[0, 255, 0, 0, 255, 0]);
    let s = p
        .doc
        .get_object_mut(green)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    s.dict.set("SMask", mask);
    add_res(
        &mut p,
        1,
        dictionary! { "XObject" => dictionary! { "Green" => green } },
    );
    let (rep, _) = redacted(&mut p, &marks, &[], &[]);
    let (samples, _) = samples_and_alternates(&mut p);
    assert!(
        samples.contains(&MASK.to_vec()) && !samples.contains(&RED_BLUE.to_vec()),
        "{samples:?}"
    );
    assert_eq!(rep.pictures_elsewhere, [1]);
    assert_eq!(rep.picture_notices(), [notice(2)]);
}

/// The time each step of redacting one word in a long document takes, the median of three
/// runs, for PB-44's baseline (WP-04 makes it faster): every page has a line of text sharing
/// one font, and the middle page has "Secret" in it too. Run on a release build:
/// `cargo test -p limarepdf-core --release --test redact -- --ignored --nocapture
/// redaction_timings_in_long_documents`.
#[test]
#[ignore]
fn redaction_timings_in_long_documents() {
    use std::time::{Duration, Instant};
    for pages in [1_000, 5_000, 10_000] {
        let mut doc = lopdf::Document::with_version("1.7");
        let tree = doc.new_object_id();
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
        });
        let kids: Vec<Object> = (0..pages)
            .map(|i| {
                let secret = if i == pages / 2 { " Secret" } else { "" };
                let text = format!(
                    "BT /F1 12 Tf 30 100 Td (Page {} keeps this line{secret}) Tj ET",
                    i + 1
                );
                let s = doc.add_object(Stream::new(Dictionary::new(), text.into_bytes()));
                doc.add_object(dictionary! {
                    "Type" => "Page", "Parent" => tree, "Contents" => s,
                    "MediaBox" => vec![0.into(), 0.into(), W.into(), H.into()],
                    "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                })
                .into()
            })
            .collect();
        doc.objects.insert(
            tree,
            dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages as i64 }.into(),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => tree });
        doc.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();

        let mut runs: Vec<[Duration; 4]> = (0..3)
            .map(|_| {
                let t = Instant::now();
                let mut p = Pdf::from_bytes(&bytes, None).unwrap();
                let open = t.elapsed();
                let t = Instant::now();
                let marks = p.find_redaction_marks("Secret").unwrap();
                let find = t.elapsed();
                assert_eq!(marks.len(), 1, "{marks:?}");
                let t = Instant::now();
                p.redact(&marks).unwrap();
                let redact = t.elapsed();
                let t = Instant::now();
                p.to_bytes().unwrap();
                [open, find, redact, t.elapsed()]
            })
            .collect();
        let median = |step: usize| {
            runs.sort_by_key(|r| r[step]);
            runs[1][step].as_secs_f64()
        };
        let [open, find, redact, save] = [0, 1, 2, 3].map(median);
        println!(
            "{pages} pages ({} bytes): open {open:.3} s, find {find:.3} s, redact {redact:.3} s, \
             save {save:.3} s",
            bytes.len()
        );
    }
}

/// The blacked-out picture's alternate version or its original soft mask, which page 2's
/// private data still points at.
#[test]
fn a_blacked_out_picture_s_alternate_version_or_soft_mask_something_else_points_at_is_refused() {
    let mut cases = Vec::new();
    for (what, alternate) in [("the alternate version", true), ("the soft mask", false)] {
        let (mut p, parts, marks) = picture_with_parts("BT /F1 20 Tf 30 100 Td (Two) Tj ET");
        let target = if alternate { parts.0 } else { parts.1 };
        Route::PrivateData.point_at(&mut p, target);
        cases.push((what.to_string(), p, marks, Route::PrivateData));
    }
    all_refused(cases);
}
