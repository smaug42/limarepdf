use limarepdf_core::{
    render, FontChoice, ImageWatermark, ObjectKind, PathStyle, Pdf, Shape, TextEdit,
};
use lopdf::{dictionary, Dictionary, Object, Stream};

const W: u32 = 300;
const H: u32 = 200;

fn make(content: &str) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, W as f32, H as f32).unwrap();
    set_content(&mut p, content);
    p
}

fn set_content(p: &mut Pdf, content: &str) {
    set_content_in(p, "Helvetica", content);
}

/// Set the page content, with `/F1` a standard font (no widths in the file).
fn set_content_in(p: &mut Pdf, base: &str, content: &str) {
    let page = p.page_ids()[0];
    let font = p.doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base, "Encoding" => "WinAnsiEncoding" });
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", Object::Reference(s));
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
}

struct Shot(render::Bitmap);
impl Shot {
    fn px(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.0.width + x) * 4;
        [self.0.rgba[i], self.0.rgba[i + 1], self.0.rgba[i + 2]]
    }
    fn is(&self, x: usize, y: usize, c: [u8; 3]) -> bool {
        let p = self.px(x, y);
        (0..3).all(|k| (p[k] as i32 - c[k] as i32).abs() < 12)
    }
}

fn shot(p: &mut Pdf) -> Shot {
    let bytes = p.snapshot().unwrap();
    let hp = render::load(bytes, None).unwrap();
    Shot(render::Renderer::new(&hp).render(0, 1.0).unwrap())
}

const RED: [u8; 3] = [255, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];
const BASE: &str =
    "q 1 0 0 rg 50 50 100 40 re f Q\nBT /F1 24 Tf 0 0 1 rg 60 150 Td (Hello) Tj ET\n";

fn png(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(c));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn finds_text_boxes_in_visible_text() {
    let p = make(BASE);
    assert!(p.page_objects(0).unwrap().iter().all(|o| !o.hidden));
    // "Hello" at x = 60, baseline 150 on a 200 pt page (50 from the top), 24 pt
    // Helvetica, about 55 pt wide. "ell" follows the H (722 per 1000) and is
    // 556 + 222 + 222 wide: 77.33..101.33.
    let b = p.find_text_boxes(0, "ELL").unwrap();
    assert_eq!(b.len(), 1, "{b:?}");
    let b = b[0];
    assert!((b[0] - 77.33).abs() < 0.5, "{b:?}");
    assert!((b[2] - 101.33).abs() < 0.5, "{b:?}");
    assert!(b[1] > 28.0 && b[1] < 34.0, "{b:?}");
    assert!(b[3] > 52.0 && b[3] < 58.0, "{b:?}");
    assert_eq!(p.find_text_boxes(0, "l").unwrap().len(), 2);
    assert!(p.find_text_boxes(0, "   ").unwrap().is_empty());
}

#[test]
fn finds_text_in_reading_order_not_drawing_order() {
    // The lower line is drawn first.
    let p =
        make("BT /F1 18 Tf 20 100 Td (gamma) Tj ET\nBT /F1 18 Tf 20 150 Td (alpha beta) Tj ET\n");
    let m = p.find_text_matches(0, "beta gamma").unwrap();
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m[0].len(), 2, "{m:?}");
    assert!(
        m[0][0][1] < m[0][1][1],
        "beta's box is above gamma's: {m:?}"
    );
    assert_eq!(p.search("beta gamma").unwrap().len(), 1);
    assert!(p.find_text_matches(0, "gamma alpha").unwrap().is_empty());
    assert!(p.search("gamma alpha").unwrap().is_empty());
}

#[test]
fn match_boxes_follow_glyph_widths() {
    // Helvetica widths: i 222, space 278, W 944 (per 1000). At 20 pt from x = 10,
    // "WWWWW" starts after ten narrow i's and a space, at 10 + 2498 * 0.02 = 59.96,
    // and is 5 * 944 * 0.02 = 94.4 wide. Placed by character count it would start
    // near 109.
    let p = make("BT /F1 20 Tf 10 150 Td (iiiiiiiiii WWWWW) Tj ET");
    let b = p.find_text_boxes(0, "wwwww").unwrap();
    assert_eq!(b.len(), 1, "{b:?}");
    assert!((b[0][0] - 59.96).abs() < 0.5, "{b:?}");
    assert!((b[0][2] - 154.36).abs() < 0.5, "{b:?}");

    // Kerning and spacing inside a TJ array move the characters after them, and a
    // wide gap that stands for a space is a character too.
    let p = make("BT /F1 20 Tf 2 Tc 10 150 Td [(ii) -1000 (WW) -300 (i)] TJ ET");
    let w = p.page_words(0).unwrap();
    let texts: Vec<&str> = w.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(texts, ["ii", "WW", "i"]);
    // ii: 2 * (4.44 + 2) = 12.88, ending at 22.88 less the last Tc: 20.88.
    assert!((w[0].bbox[2] - 20.88).abs() < 0.5, "{w:?}");
    // The gap: 20 pt. WW from 10 + 12.88 + 20 = 42.88.
    assert!((w[1].bbox[0] - 42.88).abs() < 0.5, "{w:?}");

    // A line drawn in two pieces with a gap between them: the space LimarePDF adds for the
    // gap must not throw off the first piece. "iii" follows WWW (3 * 18.88) and is
    // 3 * 4.44 wide.
    let p = make("BT /F1 20 Tf 10 150 Td (WWWiii) Tj 80 0 Td (x) Tj ET");
    assert_eq!(p.page_words(0).unwrap().len(), 2);
    let b = p.find_text_boxes(0, "iii").unwrap();
    assert!((b[0][0] - 66.64).abs() < 0.5, "{b:?}");
    assert!((b[0][2] - 79.96).abs() < 0.5, "{b:?}");

    // Times without widths in the file: Adobe's widths, i 278, space 250, W 944, so
    // "WWW" starts at 10 + 1640 * 0.02 = 42.8 and is 56.64 wide.
    let mut p = Pdf::new();
    p.insert_blank(0, W as f32, H as f32).unwrap();
    let content = "BT /F1 20 Tf 10 150 Td (iiiii WWW) Tj ET";
    set_content_in(&mut p, "Times-Roman", content);
    let b = p.find_text_boxes(0, "www").unwrap();
    assert!((b[0][0] - 42.8).abs() < 0.5, "{b:?}");
    assert!((b[0][2] - 99.44).abs() < 0.5, "{b:?}");
}

#[test]
fn reads_text_through_a_compact_tounicode_map() {
    // Codes 1 and 2 mean nothing in any encoding; only the ToUnicode map, laid out as
    // LibreOffice writes it, says they are "H" and "i". Code 3 is the "ﬁ" ligature,
    // which comes out as its letters.
    let mut p = make("BT /F1 20 Tf 10 150 Td (\x01\x02 \x03ne) Tj ET");
    let cmap = b"/CIDInit/ProcSet findresource begin\n12 dict begin\nbegincmap\n\
        /CIDSystemInfo<<\n/Registry (Adobe)\n/Ordering (UCS)\n/Supplement 0\n>> def\n\
        /CMapName/Adobe-Identity-UCS def\n/CMapType 2 def\n\
        1 begincodespacerange\n<00> <FF>\nendcodespacerange\n\
        5 beginbfchar\n<01> <0048>\n<02> <0069>\n<03> <FB01>\n<20> <0020>\n<6E> <006E>\n\
        endbfchar\n1 beginbfrange\n<65> <65> <0065>\nendbfrange\n\
        endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n";
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), cmap.to_vec()));
    let page = p.page_ids()[0];
    let fonts = p.doc.get_page_fonts(page).unwrap();
    let (_, font) = fonts.iter().next().unwrap();
    let mut font = (*font).clone();
    font.remove(b"Encoding");
    font.set("ToUnicode", Object::Reference(s));
    let id = p.doc.add_object(font);
    let res = dictionary! { "Font" => dictionary! { "F1" => Object::Reference(id) } };
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Resources", res);

    assert_eq!(p.page_text(0), "Hi fine");
    assert_eq!(p.search("hi").unwrap().len(), 1);
    assert_eq!(p.find_text_boxes(0, "hi").unwrap().len(), 1);
    assert_eq!(p.search("fine").unwrap().len(), 1);
}

/// A page showing the codes `hex` at (10, 150) in 20 pt, in a Type 0 font with the
/// predefined CMap `cmap`, no ToUnicode map and no font program, as Japanese filings often
/// are. The descendant says Adobe-Japan1 and makes CID 34 ("A") 500 wide, every other 1000.
fn cjk(cmap: &str, hex: &str) -> Pdf {
    let mut p = make(&format!("BT /F2 20 Tf 10 150 Td <{hex}> Tj ET"));
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
    let res = dictionary! { "Font" => dictionary! { "F2" => font } };
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Resources", res);
    p
}

#[test]
fn reads_text_through_predefined_cmaps() {
    // UCS-2 codes are Unicode already.
    let p = cjk("UniJIS-UCS2-H", "67714EAC0041");
    assert_eq!(p.page_text(0), "東京A");
    assert_eq!(p.search("東京").unwrap().len(), 1);
    // In UTF-16, a character past U+FFFF is one four-byte code.
    let p = cjk("UniJIS-UTF16-H", "D842DFB70041");
    assert_eq!(p.page_text(0), "𠮷A");
    // Shift-JIS: one byte for "A", two for each kanji. The CMap gives CIDs, which
    // Adobe-Japan1's UCS-2 map turns into text.
    let p = cjk("90ms-RKSJ-H", "41938C8B9E");
    assert_eq!(p.page_text(0), "A東京");
}

#[test]
fn predefined_cmap_widths_are_found_by_cid() {
    // "A" is code 0x0041 but CID 34, 500 wide: at 20 pt, 10 wide after two 20-wide kanji.
    let p = cjk("UniJIS-UCS2-H", "67714EAC0041");
    let b = p.find_text_boxes(0, "a").unwrap();
    assert_eq!(b.len(), 1, "{b:?}");
    assert!((b[0][0] - 50.0).abs() < 0.5, "{b:?}");
    assert!((b[0][2] - 60.0).abs() < 0.5, "{b:?}");
    // In Shift-JIS "A" is one byte, and the kanji after it start 10 along. (In
    // 90msp-RKSJ-H; 90ms-RKSJ-H gives it a half-width CID instead.)
    let p = cjk("90msp-RKSJ-H", "41938C8B9E");
    let b = p.find_text_boxes(0, "東京").unwrap();
    assert_eq!(b.len(), 1, "{b:?}");
    assert!((b[0][0] - 20.0).abs() < 0.5, "{b:?}");
    assert!((b[0][2] - 60.0).abs() < 0.5, "{b:?}");
}

#[test]
fn finds_phrases_across_lines() {
    let p = make("BT /F1 20 Tf 10 150 Td (one two) Tj 0 -30 Td (three four) Tj ET");
    // One hit, one match, with a box on each line.
    assert_eq!(p.search("two three").unwrap().len(), 1);
    let m = p.find_text_matches(0, "Two   THREE").unwrap();
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m[0].len(), 2, "{m:?}");
    let (two, three) = (m[0][0], m[0][1]);
    // "two" follows "one " (556 * 2 + 556 + 278 = 1946 per 1000 at 20 pt): 48.92.
    assert!((two[0] - 48.92).abs() < 0.5, "{two:?}");
    // "three" starts the second line, 30 pt lower.
    assert!((three[0] - 10.0).abs() < 0.5, "{three:?}");
    assert!((three[1] - two[1] - 30.0).abs() < 0.5, "{m:?}");
    // Within a line, a match is still one box; counts agree with search.
    let m = p.find_text_matches(0, "o").unwrap();
    assert!(m.iter().all(|b| b.len() == 1));
    assert_eq!(m.len(), p.search("o").unwrap().len());
    assert!(p.find_text_matches(0, "four one").unwrap().is_empty());
}

#[test]
fn reads_text_inside_form_xobjects() {
    // The page draws form Fm1 moved 50 right; Fm1 (moved 5 more by its matrix) shows
    // "Inside" and draws Fm2, which shows "Deeper". Fm1 has its own font resources;
    // Fm2 has none and uses the page's.
    let mut p = make("q 1 0 0 1 50 0 cm /Fm1 Do Q");
    let page = p.page_ids()[0];
    let font = {
        let fonts = p.doc.get_page_fonts(page).unwrap();
        let (_, f) = fonts.iter().next().unwrap();
        p.doc.add_object((*f).clone())
    };
    let form = |content: &str, res: Option<Dictionary>| {
        let mut d = dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
            "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 5.into(), 0.into()],
        };
        if let Some(r) = res {
            d.set("Resources", r);
        }
        Stream::new(d, content.as_bytes().to_vec())
    };
    let fm2 = p
        .doc
        .add_object(form("BT /F1 20 Tf 10 100 Td (Deeper) Tj ET", None));
    let fm1_res = dictionary! {
        "Font" => dictionary! { "F1" => Object::Reference(font) },
        "XObject" => dictionary! { "Fm2" => Object::Reference(fm2) },
    };
    let fm1 = p.doc.add_object(form(
        "BT /F1 20 Tf 10 150 Td (Inside) Tj ET /Fm2 Do",
        Some(fm1_res),
    ));
    let res = dictionary! {
        "Font" => dictionary! { "F1" => Object::Reference(font) },
        "XObject" => dictionary! { "Fm1" => Object::Reference(fm1) },
    };
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Resources", res);

    assert!(p.page_has_text(0));
    assert_eq!(p.page_text(0), "Inside\nDeeper");
    // 10 in the form, 5 from its matrix, 50 from the page: 65.
    let b = p.find_text_boxes(0, "inside").unwrap();
    assert!((b[0][0] - 65.0).abs() < 0.5, "{b:?}");
    // Fm2 is moved by both matrices: 10 + 5 + 5 + 50.
    let b = p.find_text_boxes(0, "deeper").unwrap();
    assert!((b[0][0] - 70.0).abs() < 0.5, "{b:?}");
    // The form stays one object; its text lines are not objects of their own.
    let objs = p.page_objects(0).unwrap();
    assert_eq!(objs.len(), 1);
    assert_eq!(objs[0].kind, ObjectKind::Form);
}

#[test]
fn gaps_between_words_become_spaces() {
    // A title drawn as its own piece that opens with a gap, as Sphinx headings are.
    let p = make("BT /F1 20 Tf 10 150 Td [(1.1)] TJ [-500 (What)] TJ ET");
    assert_eq!(p.page_text(0), "1.1 What");
    // A justified line squeezes the word gap to under 0.2 em.
    let p = make("BT /F1 20 Tf 10 150 Td [(background) -198.6 (noise)] TJ ET");
    assert_eq!(p.page_text(0), "background noise");
    // Kerning between letters is not a gap.
    let p = make("BT /F1 20 Tf 10 150 Td [(A) 80 (V) -120 (A)] TJ ET");
    assert_eq!(p.page_text(0), "AVA");
}

#[test]
fn lists_words_for_selection() {
    let p = make("BT /F1 24 Tf 60 150 Td (Hello world) Tj 0 -40 Td (again) Tj ET");
    let w = p.page_words(0).unwrap();
    let texts: Vec<&str> = w.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(texts, ["Hello", "world", "again"]);
    assert_eq!(limarepdf_core::words_text(&w), "Hello world\nagain");
    assert_eq!(limarepdf_core::words_text(&w[1..2]), "world");
    // Same line: side by side. Next line: below.
    assert!(w[1].bbox[0] > w[0].bbox[2] - 1.0, "{:?}", w);
    assert!((w[1].bbox[1] - w[0].bbox[1]).abs() < 0.5);
    assert!(w[2].bbox[1] > w[0].bbox[3], "{:?}", w);
    assert!((w[0].bbox[0] - 60.0).abs() < 0.5);
    let shapes_only = make("q 1 0 0 rg 0 0 10 10 re f Q");
    assert!(shapes_only.page_words(0).unwrap().is_empty());
}

#[test]
fn lists_objects_in_visual_coordinates() {
    let p = make(BASE);
    let o = p.page_objects(0).unwrap();
    assert_eq!(o.len(), 2);
    assert_eq!(o[0].kind, ObjectKind::Path);
    assert!(o[0].has_fill && !o[0].has_stroke);
    // user y 50..90 on a 200 high page is 110..150 when y points down
    let b = o[0].bbox;
    assert!(
        (b[0] - 50.0).abs() < 0.5
            && (b[1] - 110.0).abs() < 0.5
            && (b[2] - 150.0).abs() < 0.5
            && (b[3] - 150.0).abs() < 0.5,
        "{b:?}"
    );
    assert_eq!(o[1].kind, ObjectKind::Text);
    assert_eq!(o[1].text.as_deref(), Some("Hello"));
    assert_eq!(o[1].font_size, Some(24.0));
    assert_eq!(o[1].fill, Some([0.0, 0.0, 1.0]));
}

#[test]
fn move_resize_and_delete_path() {
    let mut p = make(BASE);
    p.set_object_bbox(0, 0, [150.0, 110.0, 250.0, 150.0])
        .unwrap();
    let s = shot(&mut p);
    assert!(s.is(200, 130, RED));
    assert!(s.is(100, 130, WHITE));
    // resize to double height, keeping the top edge
    p.set_object_bbox(0, 0, [150.0, 110.0, 250.0, 190.0])
        .unwrap();
    let s = shot(&mut p);
    assert!(s.is(200, 180, RED));
    p.delete_object(0, 0).unwrap();
    let s = shot(&mut p);
    assert!(s.is(200, 130, WHITE) && s.is(200, 180, WHITE));
    // the text is untouched
    assert_eq!(p.page_objects(0).unwrap().len(), 1);
}

#[test]
fn move_text_keeps_following_content_in_place() {
    let mut p = make("BT /F1 20 Tf 20 100 Td (One) Tj 0 -30 Td (Two) Tj ET\n");
    let before = p.page_objects(0).unwrap();
    assert_eq!(before.len(), 2);
    p.set_object_bbox(0, 0, {
        let b = before[0].bbox;
        [b[0] + 100.0, b[1], b[2] + 100.0, b[3]]
    })
    .unwrap();
    let after = p.page_objects(0).unwrap();
    assert_eq!(after.len(), 2, "{after:?}");
    let one = after
        .iter()
        .find(|o| o.text.as_deref() == Some("One"))
        .unwrap();
    let two = after
        .iter()
        .find(|o| o.text.as_deref() == Some("Two"))
        .unwrap();
    let (b1, b2) = (
        before
            .iter()
            .find(|o| o.text.as_deref() == Some("One"))
            .unwrap(),
        before
            .iter()
            .find(|o| o.text.as_deref() == Some("Two"))
            .unwrap(),
    );
    assert!(
        (one.bbox[0] - b1.bbox[0] - 100.0).abs() < 0.5,
        "{one:?} vs {b1:?}"
    );
    assert!(
        (two.bbox[0] - b2.bbox[0]).abs() < 0.5 && (two.bbox[1] - b2.bbox[1]).abs() < 0.5,
        "Two moved: {two:?} vs {b2:?}"
    );
}

#[test]
fn move_text_with_tj_array_and_quote() {
    let mut p = make("BT /F1 14 Tf 16 TL 20 150 Td [(He) -50 (llo)] TJ (Next) ' ET\n");
    let before = p.page_objects(0).unwrap();
    assert!(before.len() >= 2, "{before:?}");
    let t0 = before[0].bbox;
    p.set_object_bbox(0, 0, [t0[0], t0[1] + 40.0, t0[2], t0[3] + 40.0])
        .unwrap();
    let after = p.page_objects(0).unwrap();
    assert_eq!(after.len(), before.len());
    assert!((after[0].bbox[1] - t0[1] - 40.0).abs() < 0.5);
    assert!((after[1].bbox[1] - before[1].bbox[1]).abs() < 0.5);
}

/// Where the ink of colour `c` starts: its leftmost column and lowest row.
fn ink(s: &Shot, c: [u8; 3]) -> Option<(usize, usize)> {
    let mut at: Option<(usize, usize)> = None;
    for y in 0..s.0.height {
        for x in 0..s.0.width {
            if s.is(x, y, c) {
                at = Some(at.map_or((x, y), |(ax, ay)| (ax.min(x), ay.max(y))));
            }
        }
    }
    at
}

#[test]
fn removing_or_changing_text_keeps_the_text_after_it_in_place() {
    // "World" carries on from where "Hello" ends, and "Next" starts a line below
    // "Hello"'s, so where both are drawn depends on what is done to "Hello".
    const LINE: &str = "BT /F1 20 Tf 20 100 Td (Hello) Tj 1 0 0 rg (World) Tj \
                        0 -30 Td 0 0.6 0 rg (Next) Tj ET\n";
    const HIDDEN: &str = "BT /F1 20 Tf 20 100 Td 3 Tr (Hello) Tj 0 Tr 1 0 0 rg (World) Tj \
                          0 -30 Td 0 0.6 0 rg (Next) Tj ET\n";
    const GREEN: [u8; 3] = [0, 153, 0];
    fn edit(text: &str, font: Option<&str>) -> TextEdit {
        TextEdit {
            text: text.into(),
            font: font.map(FontChoice::from_name),
            size: None,
            color: None,
        }
    }
    type Change = fn(&mut Pdf, usize);
    let cases: [(&str, &str, Change); 5] = [
        ("deleted", LINE, |p, id| p.delete_object(0, id).unwrap()),
        ("changed in place", LINE, |p, id| {
            p.edit_text(0, id, &edit("Hi", None)).unwrap()
        }),
        ("set in another font", LINE, |p, id| {
            p.edit_text(0, id, &edit("Hi", Some("Courier"))).unwrap()
        }),
        ("changed while invisible", HIDDEN, |p, id| {
            p.edit_text(0, id, &edit("Hi", None)).unwrap()
        }),
        ("moved", LINE, |p, id| {
            let b = p.page_objects(0).unwrap()[id].bbox;
            p.set_object_bbox(0, id, [b[0], b[1] - 40.0, b[2], b[3] - 40.0])
                .unwrap()
        }),
    ];
    let mut moved = Vec::new();
    for (what, content, change) in cases {
        let mut p = make(content);
        let find = |p: &Pdf, t: &str| {
            p.page_objects(0)
                .unwrap()
                .into_iter()
                .find(|o| o.text.as_deref() == Some(t))
                .unwrap_or_else(|| panic!("{what}: no {t}"))
        };
        let hello = find(&p, "Hello").id;
        let kept = [
            ("World", find(&p, "World").bbox),
            ("Next", find(&p, "Next").bbox),
        ];
        let before = shot(&mut p);
        change(&mut p, hello);
        for (t, was) in kept {
            let now = find(&p, t).bbox;
            if now.iter().zip(was).any(|(a, b)| (a - b).abs() > 0.01) {
                moved.push(format!("{what}: {t} moved from {was:?} to {now:?}"));
            }
        }
        let after = shot(&mut p);
        for c in [RED, GREEN] {
            let (was, now) = (ink(&before, c).unwrap(), ink(&after, c).unwrap());
            if was.0.abs_diff(now.0) > 1 || was.1 != now.1 {
                moved.push(format!("{what}: {c:?} drawn from {was:?}, then {now:?}"));
            }
        }
    }
    assert!(moved.is_empty(), "{moved:#?}");
}

#[test]
fn text_after_vertical_writing_is_left_as_it_was() {
    // A font writing downwards: LimarePDF measures its text as if it ran across, so it
    // doesn't know where the line ends, and leaves the text after it as it was.
    let mut p = make("BT /F2 20 Tf 150 180 Td <00410042> Tj 1 0 0 rg <0043> Tj ET\n");
    let desc = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => "V", "DW" => 1000,
    });
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "V",
        "Encoding" => "Identity-V", "DescendantFonts" => vec![desc.into()],
    });
    let page = p.page_ids()[0];
    let res = dictionary! { "Font" => dictionary! { "F2" => font } };
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Resources", res);
    let objs = p.page_objects(0).unwrap();
    assert_eq!(objs.len(), 2, "{objs:?}");
    p.delete_object(0, objs[0].id).unwrap();
    let ops = lopdf::content::Content::decode(&p.doc.get_page_content(page))
        .unwrap()
        .operations;
    let names: Vec<&str> = ops.iter().map(|o| o.operator.as_str()).collect();
    assert_eq!(names, ["BT", "Tf", "Td", "rg", "Tj", "ET"]);
}

#[test]
fn edit_text_in_place_and_with_new_font() {
    let mut p = make(BASE);
    p.edit_text(
        0,
        1,
        &TextEdit {
            text: "Howdy".into(),
            font: None,
            size: None,
            color: None,
        },
    )
    .unwrap();
    let o = p.page_objects(0).unwrap();
    assert_eq!(o[1].text.as_deref(), Some("Howdy"));
    assert_eq!(o[1].font_size, Some(24.0));
    p.edit_text(
        0,
        1,
        &TextEdit {
            text: "Ünï €".into(),
            font: Some(FontChoice::from_name("Courier-Bold")),
            size: Some(30.0),
            color: Some([0.0, 0.5, 0.0]),
        },
    )
    .unwrap();
    let o = p.page_objects(0).unwrap();
    let t = o.iter().find(|x| x.kind == ObjectKind::Text).unwrap();
    assert_eq!(t.text.as_deref(), Some("Ünï €"));
    assert_eq!(t.font_size, Some(30.0));
    assert_eq!(t.fill, Some([0.0, 0.5, 0.0]));
    assert_eq!(o.iter().filter(|x| x.kind == ObjectKind::Text).count(), 1);
    // saved file re-opens
    let bytes = p.to_bytes().unwrap();
    let q = Pdf::from_bytes(&bytes, None).unwrap();
    assert_eq!(q.page_objects(0).unwrap().len(), o.len());
}

#[test]
fn restyle_path() {
    let mut p = make(BASE);
    p.set_path_style(
        0,
        0,
        &PathStyle {
            fill: Some([0.0, 1.0, 0.0]),
            stroke: Some([0.0, 0.0, 0.0]),
            line_width: 4.0,
        },
    )
    .unwrap();
    let s = shot(&mut p);
    assert!(s.is(100, 130, [0, 255, 0]));
    assert!(s.is(100, 110, [0, 0, 0]) || s.is(100, 111, [0, 0, 0]) || s.is(100, 109, [0, 0, 0]));
    p.set_path_style(
        0,
        0,
        &PathStyle {
            fill: None,
            stroke: Some([0.0, 0.0, 1.0]),
            line_width: 2.0,
        },
    )
    .unwrap();
    let s = shot(&mut p);
    assert!(s.is(100, 130, WHITE));
}

#[test]
fn insert_shapes_text_and_images() {
    let mut p = make("");
    p.add_shape(
        0,
        Shape::Rect,
        [10.0, 10.0, 60.0, 40.0],
        &PathStyle {
            fill: Some([1.0, 0.0, 0.0]),
            stroke: None,
            line_width: 1.0,
        },
    )
    .unwrap();
    p.add_shape(
        0,
        Shape::Ellipse,
        [100.0, 10.0, 160.0, 70.0],
        &PathStyle {
            fill: Some([0.0, 0.0, 1.0]),
            stroke: None,
            line_width: 1.0,
        },
    )
    .unwrap();
    p.add_shape(
        0,
        Shape::Line,
        [10.0, 100.0, 90.0, 100.0],
        &PathStyle {
            fill: None,
            stroke: Some([0.0, 0.0, 0.0]),
            line_width: 4.0,
        },
    )
    .unwrap();
    p.add_image(
        0,
        [200.0, 100.0, 260.0, 150.0],
        &png(8, 8, [0, 200, 0, 255]),
    )
    .unwrap();
    p.add_text(
        0,
        (20.0, 180.0),
        "Added",
        FontChoice::from_name("Times-Italic"),
        18.0,
        [0.0, 0.0, 0.0],
    )
    .unwrap();
    let s = shot(&mut p);
    assert!(s.is(30, 25, RED));
    assert!(s.is(130, 40, [0, 0, 255]));
    assert!(s.is(50, 100, [0, 0, 0]));
    assert!(s.is(230, 125, [0, 200, 0]));
    assert!(s.is(50, 25 + 100, WHITE));
    let o = p.page_objects(0).unwrap();
    assert_eq!(o.iter().filter(|x| x.kind == ObjectKind::Image).count(), 1);
    assert!(o.iter().any(|x| x.text.as_deref() == Some("Added")));
    assert_eq!(o.iter().filter(|x| x.kind == ObjectKind::Path).count(), 3);
    // replace the image with a wide one: fitted inside the old box
    let img = o.iter().find(|x| x.kind == ObjectKind::Image).unwrap().id;
    p.replace_image(0, img, &png(40, 10, [200, 0, 200, 255]))
        .unwrap();
    let s = shot(&mut p);
    assert!(s.is(230, 125, [200, 0, 200]));
    assert!(s.is(230, 105, WHITE), "letterboxed above");
    // move and delete the image
    let img = p
        .page_objects(0)
        .unwrap()
        .iter()
        .find(|x| x.kind == ObjectKind::Image)
        .unwrap()
        .id;
    p.delete_object(0, img).unwrap();
    assert!(shot(&mut p).is(230, 125, WHITE));
}

#[test]
fn transparent_png_gets_soft_mask() {
    let mut p = make("");
    p.add_image(0, [0.0, 0.0, 100.0, 100.0], &png(4, 4, [255, 0, 0, 0]))
        .unwrap();
    let s = shot(&mut p);
    assert!(
        s.is(50, 50, WHITE),
        "fully transparent image must not paint"
    );
}

#[test]
fn works_on_rotated_pages() {
    for rot in [90, 180, 270] {
        let mut p = make("");
        p.rotate_pages(&[0], rot).unwrap();
        let (vw, vh) = p.page_size(0).unwrap();
        // top-left quarter of the displayed page
        p.add_shape(
            0,
            Shape::Rect,
            [0.0, 0.0, vw / 4.0, vh / 4.0],
            &PathStyle {
                fill: Some([1.0, 0.0, 0.0]),
                stroke: None,
                line_width: 1.0,
            },
        )
        .unwrap();
        let o = p.page_objects(0).unwrap();
        let b = o[0].bbox;
        assert!(
            b[0].abs() < 0.5 && b[1].abs() < 0.5 && (b[2] - vw / 4.0).abs() < 0.5,
            "rot {rot}: {b:?}"
        );
        let bytes = p.snapshot().unwrap();
        let hp = render::load(bytes, None).unwrap();
        let bmp = render::Renderer::new(&hp).render(0, 1.0).unwrap();
        let s = Shot(bmp);
        assert!(s.is(5, 5, RED), "rot {rot}");
        assert!(s.is(s.0.width - 5, s.0.height - 5, WHITE), "rot {rot}");
        // move it to the bottom-right
        p.set_object_bbox(0, 0, [vw * 0.75, vh * 0.75, vw, vh])
            .unwrap();
        let bytes = p.snapshot().unwrap();
        let hp = render::load(bytes, None).unwrap();
        let s = Shot(render::Renderer::new(&hp).render(0, 1.0).unwrap());
        assert!(s.is(s.0.width - 5, s.0.height - 5, RED), "rot {rot} moved");
        assert!(s.is(5, 5, WHITE), "rot {rot} moved");
    }
}

#[test]
fn nested_state_and_ctm_are_respected() {
    // a rectangle drawn under a scaled/translated CTM inside q/Q, then a second one after
    let mut p =
        make("q 2 0 0 2 10 10 cm 1 0 0 rg 10 10 20 10 re f Q\n0 0 1 rg 200 150 40 20 re f\n");
    let o = p.page_objects(0).unwrap();
    assert_eq!(o.len(), 2);
    let b = o[0].bbox; // user (30..70, 30..50) -> vis y 150..170
    assert!(
        (b[0] - 30.0).abs() < 0.5 && (b[2] - 70.0).abs() < 0.5 && (b[1] - 150.0).abs() < 0.5,
        "{b:?}"
    );
    let second = o[1].bbox; // user (200..240, 150..170) -> vis y 30..50
    let near = |a: [f32; 4], b: [f32; 4]| (0..4).all(|k| (a[k] - b[k]).abs() < 0.5);
    assert!(near(second, [200.0, 30.0, 240.0, 50.0]), "{second:?}");
    // the moved rectangle overlaps the second one's left edge, which is drawn over it
    p.set_object_bbox(0, 0, [130.0, 10.0, 210.0, 50.0]).unwrap();
    let s = shot(&mut p);
    assert!(s.is(170, 30, RED));
    assert!(s.is(50, 160, WHITE));
    // the untouched second rectangle is still blue, where it was, and still on top
    let blue = [0, 0, 255];
    assert!(s.is(220, 40, blue));
    assert!(s.is(205, 40, blue));
    assert!(s.is(245, 40, WHITE) && s.is(220, 55, WHITE));
    let o2 = p.page_objects(0).unwrap();
    assert_eq!(o2.len(), 2);
    let b0 = o2[0].bbox;
    assert!(
        (b0[0] - 130.0).abs() < 0.5 && (b0[3] - 50.0).abs() < 0.5,
        "{b0:?}"
    );
    assert!(near(o2[1].bbox, second), "{:?}", o2[1].bbox);
}

#[test]
fn unbalanced_q_does_not_leak_into_new_content() {
    let mut p = make("q 0.5 0 0 0.5 0 0 cm 1 0 0 rg 20 20 100 100 re f\n");
    p.add_shape(
        0,
        Shape::Rect,
        [200.0, 10.0, 260.0, 60.0],
        &PathStyle {
            fill: Some([0.0, 0.0, 1.0]),
            stroke: None,
            line_width: 1.0,
        },
    )
    .unwrap();
    let s = shot(&mut p);
    assert!(s.is(230, 35, [0, 0, 255]));
}

/// The deepest q/Q nesting in the page's content.
fn max_q_depth(p: &Pdf) -> usize {
    let page = p.page_ids()[0];
    let ops = lopdf::content::Content::decode(&p.doc.get_page_content(page))
        .unwrap()
        .operations;
    let (mut depth, mut max) = (0usize, 0usize);
    for o in &ops {
        match o.operator.as_str() {
            "q" => {
                depth += 1;
                max = max.max(depth);
            }
            "Q" => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}

/// Browsers' printed pages flip and scale the page with a `cm` outside any q/Q, then flip
/// each line back with its text matrix. Text written after the old content must not inherit
/// that matrix.
#[test]
fn new_text_ignores_an_unsaved_page_matrix() {
    // "Hello" is 24 pt at x = 20, baseline y = 150 in user space (50 from the top).
    let content = ".5 0 0 -.5 0 200 cm\nBT /F1 48 Tf 1 0 0 -1 40 100 Tm (Hello) Tj ET\n";
    let upright = |b: [f32; 4], x: f32, base: f32, size: f32| {
        // Ascent above the baseline, descent below it, displayed y down.
        (b[0] - x).abs() < 0.5
            && (b[1] - (base - 0.79 * size)).abs() < 0.5
            && (b[3] - (base + 0.21 * size)).abs() < 0.5
    };
    let o = make(content).page_objects(0).unwrap();
    assert_eq!(o[0].font_size, Some(24.0));
    assert!(upright(o[0].bbox, 20.0, 50.0, 24.0), "{:?}", o[0].bbox);
    for e in [
        TextEdit {
            text: "Hello again".into(),
            font: None,
            size: None,
            color: Some([0.0, 0.0, 1.0]),
        },
        TextEdit {
            text: "Hello again".into(),
            font: Some(FontChoice::from_name("Courier")),
            size: None,
            color: None,
        },
    ] {
        let mut p = make(content);
        p.edit_text(0, 0, &e).unwrap();
        let o = p.page_objects(0).unwrap();
        let t = o.iter().find(|x| x.kind == ObjectKind::Text).unwrap();
        assert_eq!(t.text.as_deref(), Some("Hello again"));
        assert_eq!(t.font_size, Some(24.0), "{e:?}");
        assert!(upright(t.bbox, 20.0, 50.0, 24.0), "{e:?}: {:?}", t.bbox);
        // Once wrapped, the old content is not wrapped again by later edits.
        for _ in 0..5 {
            let id = p
                .page_objects(0)
                .unwrap()
                .iter()
                .find(|x| x.kind == ObjectKind::Text)
                .unwrap()
                .id;
            p.edit_text(0, id, &e).unwrap();
        }
        assert_eq!(max_q_depth(&p), 1);
    }
    // Invisible text (an OCR layer) is replaced over the same extent.
    let mut p = make(&content.replace("BT ", "BT 3 Tr "));
    p.edit_text(
        0,
        0,
        &TextEdit {
            text: "Hullo".into(),
            font: None,
            size: None,
            color: None,
        },
    )
    .unwrap();
    let o = p.page_objects(0).unwrap();
    assert!(o[0].hidden && o[0].text.as_deref() == Some("Hullo"));
    assert_eq!(o[0].font_size, Some(24.0));
    assert!(upright(o[0].bbox, 20.0, 50.0, 24.0), "{:?}", o[0].bbox);
    let mut p = make(content);
    p.add_text(
        0,
        (20.0, 180.0),
        "Added",
        FontChoice::from_name("Helvetica"),
        18.0,
        [0.0, 0.0, 0.0],
    )
    .unwrap();
    let o = p.page_objects(0).unwrap();
    let t = o
        .iter()
        .find(|x| x.text.as_deref() == Some("Added"))
        .unwrap();
    assert_eq!(t.font_size, Some(18.0));
    assert!(upright(t.bbox, 20.0, 180.0, 18.0), "{:?}", t.bbox);
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    let mut p = make(BASE);
    assert!(p.delete_object(0, 99).is_err());
    assert!(p.set_object_bbox(0, 0, [10.0, 10.0, 10.0, 10.0]).is_err());
    assert!(p
        .add_image(0, [0.0, 0.0, 10.0, 10.0], b"not an image")
        .is_err());
    assert!(p
        .edit_text(
            0,
            0,
            &TextEdit {
                text: "x".into(),
                font: None,
                size: None,
                color: None
            }
        )
        .is_err());
    assert!(p.page_objects(5).is_err());
}

/// A red 4 × 4 inline image drawn 100 pt square at (50, 50), then `after`.
fn inline_pdf_with(after: &[u8]) -> Vec<u8> {
    let mut content = b"q 100 0 0 100 50 50 cm\nBI /W 4 /H 4 /BPC 8 /CS /RGB ID ".to_vec();
    content.extend(std::iter::repeat_n([255u8, 0, 0], 16).flatten());
    content.extend(b"\nEI Q\n");
    content.extend(after);
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
    ];
    let mut s = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    s.extend(&content);
    s.extend(b"\nendstream");
    objs.push(s);
    objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = vec![];
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).into_bytes());
        out.extend(o);
        out.extend(b"\nendobj\n");
    }
    let x = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).into_bytes());
    for o in offs {
        out.extend(format!("{o:010} 00000 n \n").into_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n",
            objs.len() + 1
        )
        .into_bytes(),
    );
    out
}

fn inline_pdf() -> Vec<u8> {
    inline_pdf_with(b"BT /F1 12 Tf 50 200 Td (after inline) Tj ET\n")
}

#[test]
fn inline_images_come_back_only_where_they_were() {
    // Text that reads like LimarePDF's placeholder for an inline image, and an operator
    // spelled like one (in a compatibility section, where readers skip operators they
    // don't know). A square is deleted, so that the page is written again.
    const SQUARE: &[u8] = b"0 0 1 rg 260 10 20 20 re f\n";
    let text = "see LIMAREINLINEIMGA or LIMAREINLINEIMGKKA";
    let kinds = |o: &[limarepdf_core::PageObject], k| o.iter().filter(|o| o.kind == k).count();
    let edited = |after: &[u8]| {
        let mut p = Pdf::from_bytes(&inline_pdf_with(&[after, SQUARE].concat()), None).unwrap();
        let objs = p.page_objects(0).unwrap();
        assert_eq!(kinds(&objs, ObjectKind::Image), 1, "{objs:?}");
        let square = objs.iter().find(|o| o.kind == ObjectKind::Path).unwrap().id;
        p.delete_object(0, square).unwrap();
        let bytes = p.to_bytes().unwrap();
        let q = Pdf::from_bytes(&bytes, None).unwrap();
        let objs = q.page_objects(0).unwrap();
        assert_eq!(kinds(&objs, ObjectKind::Image), 1, "{objs:?}");
        assert_eq!(kinds(&objs, ObjectKind::Path), 0, "{objs:?}");
        let hp = render::load(bytes, None).unwrap();
        let s = Shot(render::Renderer::new(&hp).render(0, 1.0).unwrap());
        assert!(s.is(100, 200, RED), "the image is still drawn");
        assert!(s.is(200, 100, WHITE), "and only where it was");
        q
    };

    let q = edited(format!("BT /F1 12 Tf 20 20 Td ({text}) Tj ET\n").as_bytes());
    assert_eq!(q.page_text(0), text);
    // The text after it keeps hayro from taking the image to end at the operator's "EI".
    edited(b"q 100 0 0 100 150 150 cm BX LIMAREINLINEIMGA EX Q\nBT /F1 12 Tf 20 20 Td (x) Tj ET\n");
}

#[test]
fn inline_images_survive_edits() {
    let mut p = Pdf::from_bytes(&inline_pdf(), None).unwrap();
    let before = {
        let bytes = p.snapshot().unwrap();
        let hp = render::load(bytes, None).unwrap();
        Shot(render::Renderer::new(&hp).render(0, 1.0).unwrap())
    };
    assert!(
        before.is(100, 200, RED),
        "inline image renders before the edit"
    );
    let o = p.page_objects(0).unwrap();
    assert!(o[0].inline);
    // edit the text, then move the inline image
    p.edit_text(
        0,
        1,
        &TextEdit {
            text: "changed".into(),
            font: None,
            size: None,
            color: None,
        },
    )
    .unwrap();
    let b = p.page_objects(0).unwrap()[0].bbox;
    p.set_object_bbox(0, 0, [b[0] + 100.0, b[1], b[2] + 100.0, b[3]])
        .unwrap();
    let bytes = p.to_bytes().unwrap();
    let hp = render::load(bytes, None).unwrap();
    let s = Shot(render::Renderer::new(&hp).render(0, 1.0).unwrap());
    assert!(s.is(200, 200, RED), "inline image moved and still drawn");
    assert!(s.is(100, 200, WHITE));
}

#[test]
fn image_watermark_is_fitted_centred_and_layered() {
    const GREEN: [u8; 3] = [0, 200, 0];
    for foreground in [true, false] {
        let mut p = make(BASE);
        // 40 x 20 at half the page fits a 150 x 100 box by width: 150 x 75, centred,
        // so x 75..225 and y 62.5..137.5 from the top.
        let mut wm = ImageWatermark::new(png(40, 20, [0, 200, 0, 255]));
        wm.opacity = 1.0;
        wm.foreground = foreground;
        p.add_image_watermark(&[0], &wm).unwrap();
        let s = shot(&mut p);
        assert!(s.is(200, 70, GREEN));
        assert!(s.is(80, 130, if foreground { GREEN } else { RED }));
        assert!(s.is(70, 130, RED));
        assert!(s.is(230, 100, WHITE));
        assert!(s.is(150, 55, WHITE));
    }
}

#[test]
fn watermarks_keep_their_own_opacity_and_inherited_resources() {
    let mut p = make(BASE);
    // Move the page's resources up to the page tree, as many files do.
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    let res = d.remove(b"Resources").unwrap();
    let parent = d.get(b"Parent").unwrap().as_reference().unwrap();
    p.doc
        .get_dictionary_mut(parent)
        .unwrap()
        .set("Resources", res);
    p.add_header_footer(&[0], &Default::default()).unwrap();
    let mut wm = ImageWatermark::new(png(4, 4, [0, 0, 0, 255]));
    wm.opacity = 0.2;
    p.add_image_watermark(&[0], &wm).unwrap();
    wm.opacity = 0.6;
    p.add_image_watermark(&[0], &wm).unwrap();
    let s = shot(&mut p);
    assert!(s.is(62, 45, [0, 0, 255]), "page text keeps its font");
    let res = p.doc.get_dictionary(page).unwrap().get(b"Resources");
    let gs = res.unwrap().as_dict().unwrap().get(b"ExtGState").unwrap();
    assert_eq!(gs.as_dict().unwrap().len(), 2);
}

#[test]
fn headers_and_footers_leave_a_page_font_of_the_same_name_alone() {
    // The page's own font is Courier, named as LimarePDF names its Helvetica: "iiii" at
    // 20 pt is 4 * 12 = 48 wide in Courier, 17.76 in Helvetica.
    let width = |p: &Pdf, text: &str| -> Vec<f32> {
        let w = p.page_words(0).unwrap();
        w.iter()
            .filter(|w| w.text == text)
            .map(|w| w.bbox[2] - w.bbox[0])
            .collect()
    };
    for place in ["direct", "indirect", "inherited"] {
        let mut p = Pdf::new();
        p.insert_blank(0, W as f32, H as f32).unwrap();
        let page = p.page_ids()[0];
        let content = "BT /LimareF1 20 Tf 10 150 Td (iiii) Tj ET";
        let font = p.doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier", "Encoding" => "WinAnsiEncoding" });
        let s = p
            .doc
            .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
        let fonts = dictionary! { "LimareF1" => font };
        let res = match place {
            "direct" => Object::Dictionary(dictionary! { "Font" => fonts }),
            _ => {
                let f = p.doc.add_object(fonts);
                p.doc.add_object(dictionary! { "Font" => f }).into()
            }
        };
        let d = p.doc.get_dictionary_mut(page).unwrap();
        d.set("Contents", Object::Reference(s));
        if place == "inherited" {
            d.remove(b"Resources");
            let parent = d.get(b"Parent").unwrap().as_reference().unwrap();
            p.doc
                .get_dictionary_mut(parent)
                .unwrap()
                .set("Resources", res);
        } else {
            d.set("Resources", res);
        }
        let w = width(&p, "iiii");
        assert!(w.len() == 1 && (w[0] - 48.0).abs() < 0.5, "{place}: {w:?}");
        for n in 1..=2 {
            p.add_header_footer(&[0], &Default::default()).unwrap();
            if n == 1 {
                // (After two, the footers lie on top of each other.)
                assert!(p.page_text(0).contains("Page 1 of 1"), "{place}");
            }
            let w = width(&p, "iiii");
            assert!(
                w.len() == 1 && (w[0] - 48.0).abs() < 0.5,
                "{place} {n}: {w:?}"
            );
            // The footer itself is still Helvetica: "Page" at 10 pt is 23.35 wide.
            let w = width(&p, "Page");
            assert_eq!(w.len(), n, "{place} {n}");
            assert!(
                w.iter().all(|w| (w - 23.35).abs() < 0.5),
                "{place} {n}: {w:?}"
            );
        }
    }
}

/// Every character of the standard fonts' WinAnsi set is written as itself, those of
/// codes 128 to 159 included, and none outside it.
#[test]
fn added_text_keeps_the_whole_winansi_set() {
    let high = "€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–—˜™š›œžŸ";
    assert_eq!(high.chars().count(), 27);
    for c in high.chars() {
        assert!(
            limarepdf_core::winansi_byte(c).is_some_and(|b| b >= 128),
            "{c}"
        );
    }
    for c in ['\n', '\t', 'Ω', 'Ж', '中', '\u{81}'] {
        assert_eq!(limarepdf_core::winansi_byte(c), None, "{c:?}");
    }
    let mut p = make("");
    let text = format!("Šœ Ÿ ƒ {high} àÿ");
    p.add_text(
        0,
        (20.0, 100.0),
        &text,
        FontChoice::default(),
        12.0,
        [0.0; 3],
    )
    .unwrap();
    let o = p.page_objects(0).unwrap();
    assert!(
        o.iter().any(|x| x.text.as_deref() == Some(text.as_str())),
        "{:?}",
        o.iter().map(|x| x.text.clone()).collect::<Vec<_>>()
    );
}
