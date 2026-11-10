//! Underlines and strike-throughs (NEW-07, batch 46's stage 4a; the design note
//! `limarepdf-underline-design.md`): other apps' read by one rule, in each producer's shape,
//! and drawings that look like them left alone; carried with their words through edits,
//! moves and deletion; LimarePDF's own written at the font's place.

mod common;

use common::apart::{self, Ran};
use common::saved::tools;
use limarepdf_core::{
    decoration_lines, DecoKind, Decorated, FontChoice, FontLines, ObjectKind, PageObject, Pdf,
    TextEdit,
};
use lopdf::content::Content;
use lopdf::{dictionary, Dictionary, Object, Stream};
use std::time::{Duration, Instant};

const LINE: &str = "This has an underlined phrase in it.";
/// Where the lines are drawn: their baseline's left end, in user space.
const X: f32 = 72.0;
const Y: f32 = 700.0;

/// A US Letter page drawing `content`, with Helvetica as `/F1` and the resources `more`.
fn page_with(content: &str, more: Dictionary) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    let f1 = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let s = (p.doc).add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let page = p.page_ids()[0];
    let mut res = dictionary! { "Font" => dictionary! { "F1" => f1 } };
    res.extend(&more);
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", s);
    d.set("Resources", res);
    p
}

fn page(content: &str) -> Pdf {
    page_with(content, Dictionary::new())
}

fn lines(p: &Pdf) -> Vec<PageObject> {
    let objs = p.page_objects(0).unwrap();
    objs.into_iter()
        .filter(|o| o.kind == ObjectKind::Text)
        .collect()
}

/// `text` drawn in Helvetica 12 at (X, Y).
fn shown(text: &str) -> String {
    format!("BT /F1 12 Tf {X} {Y} Td ({text}) Tj ET\n")
}

/// Where the characters `from..to` of `text`, drawn as [`shown`] draws it, start and end
/// along the page (as core reads them, which is what the rule measures against).
fn span(text: &str, word: &str) -> (f32, f32) {
    let p = page(&shown(text));
    let l = lines(&p).remove(0);
    let from = text.find(word).expect("the word");
    let from = text[..from].chars().count();
    let to = from + word.chars().count();
    let x = l.baseline.unwrap().0;
    (x + l.stops[from], x + l.stops[to])
}

/// A decoration read: what it covers, its kind and its bands.
type Read = (String, DecoKind, Vec<(f32, f32)>);

/// Each decoration read on the page.
fn read(p: &Pdf) -> Vec<Read> {
    let mut out = Vec::new();
    for l in lines(p) {
        let chars: Vec<char> = l.text.as_deref().unwrap_or("").chars().collect();
        for d in &l.decorations {
            let covered: String = chars[d.chars.0..d.chars.1].iter().collect();
            out.push((covered, d.kind, d.look.bands().to_vec()));
        }
    }
    out
}

fn near(a: f32, b: f32, by: f32) -> bool {
    (a - b).abs() <= by
}

/// LibreOffice's way: a stroked line in its own transform, after the text.
fn libreoffice(text: &str, under: &str, through: Option<&str>) -> String {
    let (x0, x1) = span(text, under);
    let mut c = shown(text);
    c += &format!(
        "q 1 0 0 1 {x0} {Y} cm 0.7 w 0 0 0 RG 0 -1.4 m {} -1.4 l S Q\n",
        x1 - x0
    );
    if let Some(w) = through {
        let (s0, s1) = span(text, w);
        c += &format!(
            "q 1 0 0 1 {s0} {Y} cm 0.7 w 0 0 0 RG 0 3.1 m {} 3.1 l S Q\n",
            s1 - s0
        );
    }
    c
}

#[test]
fn libreoffices_underline_and_strike_through_are_read_over_their_words() {
    let p = page(&libreoffice(LINE, "underlined phrase", Some("in it")));
    let got = read(&p);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(
        (got[0].0.as_str(), got[0].1),
        ("underlined phrase", DecoKind::Underline)
    );
    assert_eq!((got[1].0.as_str(), got[1].1), ("in it", DecoKind::Strike));
    let (v, t) = got[0].2[0];
    assert!(
        near(v, -1.4 / 12.0, 0.002) && near(t, 0.7 / 12.0, 0.002),
        "{v} {t}"
    );
    assert!(near(got[1].2[0].0, 3.1 / 12.0, 0.002));
    // The paths are the line's, and say so.
    let objs = p.page_objects(0).unwrap();
    let line = lines(&p)[0].id;
    let paths: Vec<_> = objs.iter().filter(|o| o.kind == ObjectKind::Path).collect();
    assert_eq!(paths.len(), 2);
    assert!(paths.iter().all(|o| o.decorates == Some(line)));
}

#[test]
fn chromes_filled_rectangles_under_a_flipped_page_are_read() {
    let (x0, x1) = span(LINE, "underlined phrase");
    // The page turned y-down, as browsers print it; the band before the text.
    let y = 792.0 - Y;
    let content = format!(
        "q 1 0 0 -1 0 792 cm 0 0 0 rg {x0} {} {} 0.75 re f\n\
         BT /F1 12 Tf 1 0 0 -1 {X} {y} Tm ({LINE}) Tj ET Q\n",
        y + 0.75,
        x1 - x0,
    );
    let got = read(&page(&content));
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "underlined phrase");
    assert!(near(got[0].2[0].0, -1.125 / 12.0, 0.002), "{:?}", got[0].2);
}

#[test]
fn reportlabs_stroked_line_before_the_text_is_read() {
    let (x0, x1) = span(LINE, "phrase in it");
    let content = format!(
        "0 0 0 RG 1 w {x0} {} m {x1} {} l S\n{}",
        Y - 1.5,
        Y - 1.5,
        shown(LINE)
    );
    let got = read(&page(&content));
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "phrase in it");
}

#[test]
fn two_bands_over_the_same_words_are_one_double_underline() {
    let (x0, x1) = span(LINE, "underlined");
    let w = x1 - x0;
    let content = format!(
        "{}0 0 0 rg {x0} {} {w} 0.6 re f {x0} {} {w} 0.6 re f\n",
        shown(LINE),
        Y - 1.4,
        Y - 3.0,
    );
    let got = read(&page(&content));
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].2.len(), 2, "two bands: {:?}", got[0].2);
}

#[test]
fn an_underline_broken_where_descenders_cross_it_is_one_band() {
    // macOS draws an underline in pieces around the descenders, one path.
    let (x0, x1) = span(LINE, "phrase in it");
    let mid = (x0 + x1) / 2.0;
    let content = format!(
        "{}0 0 0 rg {x0} {} {} 0.6 re {} {} {} 0.6 re f\n",
        shown(LINE),
        Y - 1.4,
        mid - 1.0 - x0,
        mid + 1.0,
        Y - 1.4,
        x1 - mid - 1.0,
    );
    let got = read(&page(&content));
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "phrase in it");
}

#[test]
fn turned_pages_and_rotated_text_read_alike() {
    // A page turned a quarter: reading is on the page's own terms.
    let mut p = page(&libreoffice(LINE, "underlined phrase", None));
    let id = p.page_ids()[0];
    p.doc.get_dictionary_mut(id).unwrap().set("Rotate", 90);
    assert_eq!(read(&p).len(), 1);
    // Text running up the page, its underline to its right.
    let stops = span(LINE, "underlined phrase");
    let (s0, s1) = (stops.0 - X, stops.1 - X);
    let content = format!(
        "BT /F1 12 Tf 0 1 -1 0 300 100 Tm ({LINE}) Tj ET\n0 0 0 rg 300.9 {} 0.75 {} re f\n",
        100.0 + s0,
        s1 - s0,
    );
    let got = read(&page(&content));
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "underlined phrase");
}

/// Nothing read as a decoration on a page drawing `content`.
fn none_read(content: &str, what: &str) {
    let got = read(&page(content));
    assert!(got.is_empty(), "{what}: {got:?}");
}

#[test]
fn a_fraction_bar_is_not_its_numerators_underline() {
    let (n0, n1) = (100.0, 100.0 + span("123", "123").1 - X);
    let d = span("45", "45").1 - X;
    let mid = (n0 + n1) / 2.0;
    let content = format!(
        "BT /F1 12 Tf {n0} {Y} Td (123) Tj ET\n0 0 0 rg {n0} {} {} 0.5 re f\n\
         BT /F1 12 Tf {} {} Td (45) Tj ET\n",
        Y - 3.25,
        n1 - n0,
        mid - d / 2.0,
        Y - 14.0,
    );
    none_read(&content, "a fraction");
}

#[test]
fn a_table_rule_meeting_a_column_rule_is_left_alone() {
    let text = "Apples and pears";
    let (x0, x1) = span(text, text);
    let content = format!(
        "{}0 0 0 RG 0.5 w {x0} {} m {x1} {} l S {x0} {} m {x0} {} l S\n",
        shown(text),
        Y - 2.5,
        Y - 2.5,
        Y - 2.5,
        Y + 15.0,
    );
    none_read(&content, "a table's rules");
}

#[test]
fn a_rule_longer_than_its_heading_is_left_alone() {
    let content = format!(
        "{}0 0 0 RG 0.5 w {X} {} m 540 {} l S\n",
        shown("Heading text"),
        Y - 3.0,
        Y - 3.0
    );
    none_read(&content, "a heading rule");
}

#[test]
fn a_fill_in_line_after_a_label_is_left_alone() {
    let end = span("Name:", "Name:").1;
    let content = format!(
        "{}0 0 0 RG 0.5 w {} {} m {} {} l S\n",
        shown("Name:"),
        end + 6.0,
        Y - 2.0,
        end + 200.0,
        Y - 2.0,
    );
    none_read(&content, "a fill-in line");
}

#[test]
fn dashed_and_dotted_lines_are_left_alone() {
    let (x0, x1) = span(LINE, "underlined phrase");
    let dashed = format!(
        "{}0 0 0 RG 0.7 w [3 3] 0 d {x0} {} m {x1} {} l S\n",
        shown(LINE),
        Y - 1.4,
        Y - 1.4
    );
    none_read(&dashed, "a dashed line");
    // Dots of a point, a point apart, in one path.
    let mut dots = String::new();
    let mut x = x0;
    while x + 1.0 <= x1 {
        dots += &format!("{x} {} 1 0.7 re ", Y - 1.75);
        x += 2.0;
    }
    none_read(
        &format!("{}0 0 0 rg {dots}f\n", shown(LINE)),
        "a dotted line",
    );
}

#[test]
fn an_underline_in_another_colour_than_its_text_is_left_alone() {
    let (x0, x1) = span(LINE, "underlined phrase");
    let content = format!(
        "{}0 0 1 rg {x0} {} {} 0.7 re f\n",
        shown(LINE),
        Y - 1.75,
        x1 - x0
    );
    none_read(&content, "a blue underline under black text");
}

#[test]
fn one_character_alone_is_left_alone() {
    let text = "This is a test.";
    let (a0, a1) = span(text, "a");
    let content = format!(
        "{}0 0 0 rg {a0} {} {} 0.7 re f\n",
        shown(text),
        Y - 1.75,
        a1 - a0
    );
    none_read(&content, "one character's underline");
}

#[test]
fn a_rule_across_a_wide_gap_is_left_alone() {
    // A running head: a page number and a title in one line, a wide gap between.
    let content = format!("BT /F1 12 Tf {X} {Y} Td [(iv) -30000 (Thank you)] TJ ET\n");
    let l = lines(&page(&content)).remove(0);
    let end = l.baseline.unwrap().0 + l.stops[l.stops.len() - 1];
    let with = format!("{content}0 0 0 rg {X} {} {} 0.7 re f\n", Y - 3.0, end - X);
    none_read(&with, "a rule across a wide gap");
}

#[test]
fn a_band_on_the_baseline_is_left_alone() {
    let (x0, x1) = span(LINE, "underlined phrase");
    let content = format!(
        "{}0 0 0 rg {x0} {} {} 0.3 re f\n",
        shown(LINE),
        Y - 0.15,
        x1 - x0
    );
    none_read(&content, "a band on the baseline");
}

/// "Violin I" over a music staff's five lines, 0.45 points thick and 2.5 apart, stroked one
/// by one from `from` to `to` along the page, the lowest 0.3 points under the label's
/// baseline: where an underline would be, and the next two where a double strike-through
/// would (PB-101).
fn staff(from: f32, to: f32) -> String {
    let mut c = shown("Violin I") + "0 0 0 RG 0.45 w\n";
    for k in 0..5 {
        let y = Y - 0.3 + 2.5 * k as f32;
        c += &format!("{from} {y} m {to} {y} l S\n");
    }
    c
}

#[test]
fn a_staff_under_a_label_is_left_alone() {
    none_read(&staff(36.0, 576.0), "a staff across the page");
    let (x0, x1) = span("Violin I", "Violin I");
    none_read(&staff(x0, x1), "a staff's pieces exactly under the label");
}

/// The staff's pieces exactly under "Violin I", the label edited to "Vln. I": shorter, so the
/// staff's right end, at `x` (displayed, the staff between 82.3 and 92.3 down), is clear of
/// it.
fn a_staff_with_its_label_edited() -> (Pdf, f32) {
    let (x0, x1) = span("Violin I", "Violin I");
    let mut p = page(&staff(x0, x1));
    let id = lines(&p)[0].id;
    p.edit_text(0, id, &same_text("Vln. I")).unwrap();
    (p, x1 - 1.5)
}

/// Editing or moving the label leaves the staff's pieces under it where they are, as they
/// were drawn: none carried with the words (PB-101).
#[test]
fn editing_or_moving_a_label_leaves_its_staff_alone() {
    let (x0, x1) = span("Violin I", "Violin I");
    let drawn = |p: &Pdf| -> Vec<_> { painted(p).iter().map(|o| (o.bbox, o.has_fill)).collect() };
    let was = drawn(&page(&staff(x0, x1)));
    assert_eq!(was.len(), 5);
    let same = |p: &Pdf, what: &str| {
        let now = drawn(p);
        let ok = now.len() == 5
            && (now.iter().zip(&was))
                .all(|((a, fa), (b, fb))| fa == fb && (0..4).all(|i| near(a[i], b[i], 0.01)));
        assert!(ok, "{what}: {was:?} became {now:?}");
    };
    let (mut p, x) = a_staff_with_its_label_edited();
    same(&p, "edited");
    // All five lines still drawn at the staff's end.
    assert_eq!(crossed(&column(&mut p, x, (80.0, 95.0))), 5);
    let mut p = page(&staff(x0, x1));
    let l = lines(&p).remove(0);
    let b = l.bbox;
    p.set_object_bbox(0, l.id, [b[0] + 200.0, b[1], b[2] + 200.0, b[3]])
        .unwrap();
    same(&p, "moved");
}

/// Poppler draws the staff's five lines at its end after the label is edited too. Opt-in:
/// needs `pdftoppm`.
#[test]
#[ignore = "opt-in: needs poppler"]
fn cross_check_a_staff_with_its_label_edited_with_poppler() {
    let (mut p, x) = a_staff_with_its_label_edited();
    assert_eq!(crossed(&poppler_column(&mut p, x, (80.0, 95.0))), 5);
}

#[test]
fn a_form_s_underlined_text_is_not_the_page_s() {
    let inner = libreoffice(LINE, "underlined phrase", None);
    let mut p = page("q /X1 Do Q\n");
    let font = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let form = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
        inner.into_bytes(),
    ));
    let id = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(id).unwrap();
    let mut res = d.get(b"Resources").unwrap().as_dict().unwrap().clone();
    res.set("XObject", dictionary! { "X1" => form });
    d.set("Resources", res);
    let objs = p.page_objects(0).unwrap();
    assert!(objs
        .iter()
        .all(|o| o.decorations.is_empty() && o.decorates.is_none()));
}

/// Started by [`huge_text_beside_a_line_is_read_at_once`] in a process of its own: an "A"
/// drawn 10⁵ times over, and 10⁹ times (its cells kept to ±10⁶), beside a line.
#[test]
#[ignore = "started by huge_text_beside_a_line_is_read_at_once"]
fn huge_text_child() {
    if std::env::var("HUGE_TEXT_CHILD").is_err() {
        return;
    }
    for s in ["100000", "1000000000"] {
        let p = page(&format!(
            "q {s} 0 0 {s} 0 0 cm BT /F1 12 Tf (A) Tj ET Q 0 0 m 10 0 l S\n"
        ));
        let text: Vec<_> = lines(&p).into_iter().filter_map(|o| o.text).collect();
        println!("huge|{s}|{}", text.join(" "));
    }
}

/// Text drawn hundreds of metres across beside a line is read at once (S1-07, H-08: finding
/// the lines near the line went through every cell of 64 points the text's box covers,
/// billions, up to four million million).
#[test]
fn huge_text_beside_a_line_is_read_at_once() {
    let start = Instant::now();
    let env = [("HUGE_TEXT_CHILD", "1".to_string())];
    let ran = apart::run("huge_text_child", &env, Duration::from_secs(30));
    let Ran::Ended(text, status) = ran else {
        panic!("not read within 30 s");
    };
    let said = apart::reports(&text, "huge|");
    let want = ["100000|A", "1000000000|A"];
    assert_eq!(said, want, "{}", apart::why_stopped(status));
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "{:?}",
        start.elapsed()
    );
}

/// [`LINE`] in Helvetica 2,400 at (X, Y), with LibreOffice's band under its phrase (as
/// [`libreoffice`] draws it at 12, 200 times over), `width` thick at 12.
fn big(width: f32) -> Pdf {
    let k = 200.0;
    let (a, b) = span(LINE, "underlined phrase");
    let (x0, len) = (X + (a - X) * k, (b - a) * k);
    page(&format!(
        "BT /F1 {} Tf {X} {Y} Td ({LINE}) Tj ET\n\
         q 1 0 0 1 {x0} {Y} cm {} w 0 0 0 RG 0 {v} m {len} {v} l S Q\n",
        12.0 * k,
        width * k,
        v = -1.4 * k,
    ))
}

/// The cells of 64 points box `b` touches in the grids that find what is near a band, by
/// the rule: each, up to 4,096 of them, or else its edges'.
fn grid_cells(b: [f32; 4]) -> u64 {
    let k = |x: f32| (x / 64.0).floor() as i64;
    let w = (k(b[2]) - k(b[0]) + 1) as u64;
    let h = (k(b[3]) - k(b[1]) + 1) as u64;
    match w * h > 4096 && w.min(h) > 2 {
        true => 2 * (w + h) - 4,
        false => w * h,
    }
}

/// The steps of its budget reading `p`'s page takes.
fn steps(p: &Pdf) -> u64 {
    use limarepdf_core::budget_seam;
    let (objs, taken) = budget_seam::with_limit(u64::MAX, || p.page_objects(0).map(|_| ()));
    objs.unwrap();
    assert_eq!(taken.len(), 1, "one budget for the page: {taken:?}");
    taken[0]
}

/// `p`'s page is read within `with` steps, and refused a step short, giving nothing.
fn read_within_and_refused_short(p: &Pdf, with: u64) {
    use limarepdf_core::{budget_seam, TooComplex};
    let (got, taken) = budget_seam::with_limit(with, || p.page_objects(0).map(|_| ()));
    got.unwrap();
    assert_eq!(taken, [with]);
    let (got, taken) = budget_seam::with_limit(with - 1, || p.page_objects(0).map(|_| ()));
    let e = got.unwrap_err();
    let too = e.downcast_ref::<TooComplex>().expect("too complex");
    assert_eq!(too.limit, with - 1, "{e:#}");
    assert!(taken.len() == 1 && taken[0] < with, "{taken:?}");
}

/// The cells the grids go through to find what is near a band are steps of the page's
/// budget (S1-07): a line in Helvetica 2,400 with a thin band under it takes at least its
/// box's edge cells more than with a band too thick to be an underline (no grid made; the
/// same operators), and the page is refused a step short of what it takes. (The thin band
/// isn't read as an underline at this size either: that is the backlog's, not the grid's.)
#[test]
fn the_cells_near_a_band_are_steps_of_the_page_s_budget() {
    let (thin, thick) = (big(0.7), big(3.0));
    let b = lines(&thin)[0].bbox;
    let own = grid_cells(b);
    let area = ((b[2] - b[0]) / 64.0) * ((b[3] - b[1]) / 64.0);
    assert!(
        area > 4096.0 && own > 1000,
        "the line's box, {own} edge cells"
    );
    let (with, without) = (steps(&thin), steps(&thick));
    assert!(
        with >= without + own,
        "{with} steps with the grids, {without} without, the line's box {own} cells"
    );
    read_within_and_refused_short(&thin, with);
}

/// [`LINE`] drawn `d` times over at (X, Y), each with LibreOffice's band under the whole
/// of it, `width` thick at 12.
fn stacked(d: usize, width: f32) -> Pdf {
    let (x0, x1) = span(LINE, LINE);
    let band = format!(
        "q 1 0 0 1 {x0} {Y} cm {width} w 0 0 0 RG 0 -1.4 m {} -1.4 l S Q\n",
        x1 - x0
    );
    page(&(shown(LINE).repeat(d) + &band.repeat(d)))
}

/// The characters a band is tried against are steps of the page's budget (CP-05): 16
/// lines drawn over one another, each with a thin band, take at least a step for each of
/// each line's characters for each band more than with bands too thick to try (the same
/// operators), and the page is refused a step short of what it takes.
#[test]
fn the_characters_a_band_is_tried_against_are_steps_of_the_page_s_budget() {
    let d = 16;
    let (thin, thick) = (stacked(d, 0.7), stacked(d, 3.0));
    let tried = (d * d * LINE.chars().count()) as u64;
    let (with, without) = (steps(&thin), steps(&thick));
    assert!(
        with >= without + tried,
        "{with} steps with the bands tried, {without} without, {tried} characters tried"
    );
    read_within_and_refused_short(&thin, with);
}

/// The page's content as operators.
fn operators(p: &Pdf) -> Vec<lopdf::content::Operation> {
    let id = p.page_ids()[0];
    let data = p.doc.get_page_content(id);
    Content::decode(&data).unwrap().operations
}

/// The text operators of `ops`, as they print.
fn text_ops(ops: &[lopdf::content::Operation]) -> Vec<String> {
    let text = [
        "BT", "ET", "Tf", "Td", "TD", "Tm", "T*", "Tj", "TJ", "'", "\"", "Tc", "Tw", "Tz", "Ts",
    ];
    (ops.iter())
        .filter(|o| text.contains(&o.operator.as_str()))
        .map(|o| format!("{} {:?}", o.operator, o.operands))
        .collect()
}

fn same_text(text: &str) -> TextEdit {
    TextEdit {
        text: text.into(),
        font: None,
        size: None,
        color: None,
        parts: Vec::new(),
    }
}

fn chars_of(text: &str, word: &str) -> (usize, usize) {
    let from = text[..text.find(word).unwrap()].chars().count();
    (from, from + word.chars().count())
}

#[test]
fn underlining_words_adds_bands_and_leaves_the_words_as_they_were() {
    let mut p = page(&shown(LINE));
    let before = text_ops(&operators(&p));
    let id = lines(&p)[0].id;
    let deco = |kind, word| Decorated {
        kind,
        chars: chars_of(LINE, word),
        look: None,
    };
    let asked = vec![
        deco(DecoKind::Underline, "underlined"),
        deco(DecoKind::Strike, "in it"),
    ];
    p.edit_texts_decorated(0, &[(id, same_text(LINE), asked)])
        .unwrap();
    assert_eq!(text_ops(&operators(&p)), before);
    let got = read(&p);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(
        (got[0].0.as_str(), got[0].1),
        ("underlined", DecoKind::Underline)
    );
    // Helvetica's own underline (its AFM file's), its strike-out its look-alike's.
    let (v, t) = got[0].2[0];
    assert!(near(v, -0.1, 0.002) && near(t, 0.05, 0.002), "{v} {t}");
    assert_eq!((got[1].0.as_str(), got[1].1), ("in it", DecoKind::Strike));
    let strike = got[1].2[0].0;
    assert!((0.2..0.35).contains(&strike), "{strike}");
    // And taken off again, the words still as they were.
    let id = lines(&p)[0].id;
    p.edit_texts_decorated(0, &[(id, same_text(LINE), Vec::new())])
        .unwrap();
    assert!(read(&p).is_empty());
    assert_eq!(text_ops(&operators(&p)), before);
}

/// The paths on the page that paint something.
fn painted(p: &Pdf) -> Vec<PageObject> {
    let objs = p.page_objects(0).unwrap();
    objs.into_iter()
        .filter(|o| o.kind == ObjectKind::Path)
        .collect()
}

#[test]
fn an_edit_carries_another_apps_underline_with_its_words_in_its_look() {
    let mut p = page(&libreoffice(LINE, "underlined phrase", None));
    let id = lines(&p)[0].id;
    let text = "This has a nicely underlined phrase in it.";
    p.edit_text(0, id, &same_text(text)).unwrap();
    let got = read(&p);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "underlined phrase");
    // Its look as read: LibreOffice's place and thickness.
    let (v, t) = got[0].2[0];
    assert!(
        near(v, -1.4 / 12.0, 0.002) && near(t, 0.7 / 12.0, 0.002),
        "{v} {t}"
    );
    // Drawn again where the words are now; the old line no longer paints.
    let paths = painted(&p);
    assert_eq!(paths.len(), 1, "{paths:?}");
    let l = lines(&p).remove(0);
    let (from, _) = chars_of(text, "underlined");
    let x = l.baseline.unwrap().0 + l.stops[from];
    assert!(near(paths[0].bbox[0], x, 0.05), "{} {x}", paths[0].bbox[0]);
}

#[test]
fn an_underline_whose_words_keep_their_place_stays_as_drawn() {
    let mut p = page(&libreoffice(LINE, "underlined phrase", None));
    let id = lines(&p)[0].id;
    p.edit_text(0, id, &same_text("This has an underlined phrase in it!"))
        .unwrap();
    assert_eq!(read(&p).len(), 1);
    // Still LibreOffice's stroked line.
    let paths = painted(&p);
    assert_eq!(paths.len(), 1);
    assert!(paths[0].has_stroke && !paths[0].has_fill, "{:?}", paths[0]);
}

/// `content` drawn under `m` (a `cm` round it), moved so the point (X, Y) is drawn at
/// (X, 400): on the page whatever `m` does to it.
fn drawn_by(m: [f32; 4], content: &str) -> String {
    let [a, b, c, d] = m;
    let (e, f) = (X - a * X - c * Y, 400.0 - b * X - d * Y);
    format!("q {a} {b} {c} {d} {e} {f} cm\n{content}Q\n")
}

/// A stroked underline is read as thick as its pen draws it across the band, whatever the
/// line is drawn by (M-13). A round pen 0.7 wide, drawn by `m`, is 0.7 |det m| / |m x|
/// across a band along x, and 1.4 below the baseline is 1.4 |det m| / |m x| below it; the
/// size each is read against is 12 |m x|.
#[test]
fn a_stroked_underline_is_read_as_thick_as_it_is_drawn() {
    let (sin, cos) = 30f32.to_radians().sin_cos();
    let cases = [
        ("as it is", [1.0, 0.0, 0.0, 1.0]),
        ("twice the size", [2.0, 0.0, 0.0, 2.0]),
        ("twice as wide", [2.0, 0.0, 0.0, 1.0]),
        ("twice as high", [1.0, 0.0, 0.0, 2.0]),
        ("turned 30°", [cos, sin, -sin, cos]),
        ("leaning", [1.0, 0.0, 0.3, 1.0]),
        ("on a sloping baseline", [1.0, 0.5, 0.0, 1.0]),
    ];
    let mut wrong = vec![];
    for (what, m) in cases {
        let p = page(&drawn_by(m, &libreoffice(LINE, "underlined phrase", None)));
        let got = read(&p);
        assert_eq!(got.len(), 1, "{what}: {got:?}");
        assert_eq!(got[0].0, "underlined phrase", "{what}");
        let [a, b, c, d] = m;
        let k = (a * d - b * c).abs() / (12.0 * (a * a + b * b));
        let (v, t) = got[0].2[0];
        let want = (-1.4 * k, 0.7 * k);
        if !(near(v, want.0, 0.002) && near(t, want.1, 0.002)) {
            wrong.push(format!("{what}: read {v} {t}, drawn {want:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Whether each pixel is dark down the column through displayed x `x` of page 0, drawn at
/// 4 pixels a point, from displayed y `y0` to `y1`.
fn column(p: &mut Pdf, x: f32, (y0, y1): (f32, f32)) -> Vec<bool> {
    let hp = limarepdf_core::render::load(p.snapshot().unwrap(), None).unwrap();
    let bitmap = limarepdf_core::render::Renderer::new(&hp)
        .render(0, 4.0)
        .unwrap();
    let x = (x * 4.0) as usize;
    ((y0 * 4.0) as usize..(y1 * 4.0) as usize)
        .map(|y| bitmap.rgba[(y * bitmap.width + x) * 4] < 128)
        .collect()
}

/// The same column as Poppler draws the page (`pdftoppm`, at 288 dots an inch).
fn poppler_column(p: &mut Pdf, x: f32, (y0, y1): (f32, f32)) -> Vec<bool> {
    let dir = common::tool::Scratch::new("decoration");
    let path = dir.join("page.pdf");
    std::fs::write(&path, p.to_bytes().unwrap()).unwrap();
    let stem = dir.join("page");
    let run = common::tool::output(
        std::process::Command::new("pdftoppm")
            .args(["-r", "288", "-singlefile", "-png"])
            .arg(&path)
            .arg(&stem),
    )
    .expect("pdftoppm");
    assert!(run.status.success());
    let img = image::open(stem.with_extension("png")).unwrap().to_rgb8();
    let x = (x * 4.0) as u32;
    ((y0 * 4.0) as u32..(y1 * 4.0) as u32)
        .map(|y| img.get_pixel(x, y).0[0] < 128)
        .collect()
}

/// How many pixels of `column` are dark.
fn dark(column: &[bool]) -> usize {
    column.iter().filter(|&&d| d).count()
}

/// How many runs of dark pixels, light between, `column` has: the lines it crosses.
fn crossed(column: &[bool]) -> usize {
    (column.iter().enumerate())
        .filter(|&(i, &d)| d && (i == 0 || !column[i - 1]))
        .count()
}

/// LibreOffice's underline drawn at twice the size, carried through an edit that moves its
/// words: the page, where `x` is, the middle of "nder" (displayed, its baseline at 392),
/// and how thick, in points, the band is read before it is drawn again.
fn carried_at_twice_the_size() -> (Pdf, f32, f32) {
    let m = [2.0, 0.0, 0.0, 2.0];
    let mut p = page(&drawn_by(m, &libreoffice(LINE, "underlined phrase", None)));
    let thick = read(&p)[0].2[0].1 * 24.0;
    let id = lines(&p)[0].id;
    let text = "This has a nicely underlined phrase in it.";
    p.edit_text(0, id, &same_text(text)).unwrap();
    let got = read(&p);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "underlined phrase");
    assert!(near(got[0].2[0].0 * 24.0, -2.8, 0.03), "{got:?}");
    // Drawn again, a filled band where the words are now.
    let paths = painted(&p);
    assert_eq!(paths.len(), 1, "{paths:?}");
    assert!(paths[0].has_fill, "{:?}", paths[0]);
    let l = lines(&p).remove(0);
    let (from, to) = chars_of(text, "nder");
    let x = l.baseline.unwrap().0 + (l.stops[from] + l.stops[to]) / 2.0;
    (p, x, thick)
}

/// Carried through the edit, the band is drawn again as thick as it was drawn first: 1.4
/// points (M-13 drew it half that).
#[test]
fn an_underline_carried_at_twice_the_size_keeps_its_thickness() {
    let (mut p, x, thick) = carried_at_twice_the_size();
    // At 4 pixels a point, 5.6 pixels deep, between the baseline and a descender's depth.
    let rows = dark(&column(&mut p, x, (392.5, 398.0)));
    let ok = near(thick, 1.4, 0.03) && (5..=7).contains(&rows);
    assert!(ok, "read {thick} points thick, drawn {rows} pixels deep");
}

/// Poppler draws the band carried at twice the size 1.4 points deep too, within a pixel at
/// 4 pixels a point. Opt-in: needs `pdftoppm`.
#[test]
#[ignore = "opt-in: needs poppler"]
fn cross_check_carried_underlines_thickness_with_poppler() {
    let (mut p, x, _) = carried_at_twice_the_size();
    let rows = dark(&poppler_column(&mut p, x, (392.5, 398.0)));
    assert!((5..=7).contains(&rows), "{rows} rows");
}

#[test]
fn deleting_words_cuts_the_underline_over_them() {
    let mut p = page(&libreoffice(LINE, "underlined phrase", None));
    let id = lines(&p)[0].id;
    p.edit_text(0, id, &same_text("This has an phrase in it."))
        .unwrap();
    let got = read(&p);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "phrase");
    // Every word underlined gone: nothing drawn.
    let id = lines(&p)[0].id;
    p.edit_text(0, id, &same_text("This has an in it."))
        .unwrap();
    assert!(read(&p).is_empty());
    assert!(painted(&p).is_empty());
}

#[test]
fn moving_and_deleting_a_line_takes_its_underline() {
    let mut p = page(&libreoffice(LINE, "underlined phrase", Some("in it")));
    let l = lines(&p).remove(0);
    let was: Vec<[f32; 4]> = painted(&p).iter().map(|o| o.bbox).collect();
    let b = l.bbox;
    p.set_object_bbox(
        0,
        l.id,
        [b[0] + 10.0, b[1] + 20.0, b[2] + 10.0, b[3] + 20.0],
    )
    .unwrap();
    assert_eq!(read(&p).len(), 2);
    let now: Vec<[f32; 4]> = painted(&p).iter().map(|o| o.bbox).collect();
    assert_eq!(now.len(), 2);
    for (a, b) in was.iter().zip(&now) {
        assert!(
            near(b[0] - a[0], 10.0, 0.01) && near(b[1] - a[1], 20.0, 0.01),
            "{a:?} {b:?}"
        );
    }
    let id = lines(&p)[0].id;
    p.delete_object(0, id).unwrap();
    assert!(lines(&p).is_empty());
    assert!(painted(&p).is_empty());
}

#[test]
fn a_carried_underline_keeps_its_spot_colour() {
    let red = dictionary! {
        "FunctionType" => 2, "Domain" => vec![0.into(), 1.into()],
        "C0" => vec![1.into(), 1.into(), 1.into()], "C1" => vec![1.into(), 0.into(), 0.into()],
        "N" => 1,
    };
    let space: Vec<Object> = vec![
        "Separation".into(),
        "Red".into(),
        "DeviceRGB".into(),
        red.into(),
    ];
    let more = dictionary! { "ColorSpace" => dictionary! { "CS0" => space } };
    let (x0, x1) = span(LINE, "underlined phrase");
    let content = format!(
        "/CS0 cs 1 scn BT /F1 12 Tf {X} {Y} Td ({LINE}) Tj ET {x0} {} {} 0.7 re f\n",
        Y - 1.75,
        x1 - x0
    );
    let mut p = page_with(&content, more);
    assert_eq!(read(&p).len(), 1);
    let id = lines(&p)[0].id;
    p.edit_text(
        0,
        id,
        &same_text("This has a nicely underlined phrase in it."),
    )
    .unwrap();
    let got = lines(&p).remove(0).decorations;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].look.colour(), [1.0, 0.0, 0.0]);
    // Drawn in the spot colour itself, not the red it shows as.
    let ops = operators(&p);
    let band = ops.iter().rposition(|o| o.operator == "re").unwrap();
    let spot = |o: &&lopdf::content::Operation| {
        o.operator == "cs" && o.operands.first() == Some(&Object::Name(b"CS0".to_vec()))
    };
    assert!(ops[..band].iter().rev().take(6).any(|o| spot(&o)));
}

#[test]
fn undo_and_redo_put_an_underlines_move_back() {
    let mut p = page(&libreoffice(LINE, "underlined phrase", None));
    let id = lines(&p)[0].id;
    let before = p.before();
    let was = read(&p);
    let was_paths: Vec<[f32; 4]> = painted(&p).iter().map(|o| o.bbox).collect();
    p.edit_text(
        0,
        id,
        &same_text("This has a nicely underlined phrase in it."),
    )
    .unwrap();
    let now = read(&p);
    let undo = before.undo(&p);
    let redo = p.restore(undo);
    assert_eq!(read(&p), was);
    assert_eq!(
        painted(&p).iter().map(|o| o.bbox).collect::<Vec<_>>(),
        was_paths
    );
    p.restore(redo);
    assert_eq!(read(&p), now);
}

#[test]
fn new_text_is_underlined_at_its_fonts_place() {
    let mut p = page("");
    let text = "Signed and sealed";
    let part = limarepdf_core::TextPart {
        text: text.into(),
        font: limarepdf_core::PartFont::Choice(limarepdf_core::FontChoice::default()),
        size: 12.0,
        color: [0.0; 3],
        rise: 0.0,
        from: None,
        origins: Vec::new(),
    };
    let decos = [
        Decorated {
            kind: DecoKind::Underline,
            chars: chars_of(text, "Signed"),
            look: None,
        },
        Decorated {
            kind: DecoKind::Strike,
            chars: chars_of(text, "sealed"),
            look: None,
        },
    ];
    p.add_text_decorated(0, (72.0, 100.0), &[part], &decos)
        .unwrap();
    let got = read(&p);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(
        (got[0].0.as_str(), got[0].1),
        ("Signed", DecoKind::Underline)
    );
    assert!(near(got[0].2[0].0, -0.1, 0.002) && near(got[0].2[0].1, 0.05, 0.002));
    assert_eq!((got[1].0.as_str(), got[1].1), ("sealed", DecoKind::Strike));
    // Drawn where read: dark under "Signed" a tenth of an em below the baseline.
    let rows = dark_rows(&mut p, 4.0, (72.0, 100.0 + 1.2));
    assert!(rows > 0, "nothing drawn under the word");
}

/// How many dark pixels page 0, drawn at `scale` pixels a point, has in the row through
/// displayed point `at` (y down), from there to 30 points right.
fn dark_rows(p: &mut Pdf, scale: f32, at: (f32, f32)) -> usize {
    let hp = limarepdf_core::render::load(p.snapshot().unwrap(), None).unwrap();
    let bitmap = limarepdf_core::render::Renderer::new(&hp)
        .render(0, scale)
        .unwrap();
    let y = (at.1 * scale) as usize;
    let x0 = (at.0 * scale) as usize;
    (x0..x0 + (30.0 * scale) as usize)
        .filter(|&x| bitmap.rgba[(y * bitmap.width + x) * 4] < 128)
        .count()
}

#[test]
fn redacting_an_underlined_word_takes_its_part_of_the_underline() {
    let mut p = page(&libreoffice(LINE, "underlined phrase", None));
    let (x0, x1) = span(LINE, "phrase");
    let top = 792.0 - Y - 10.0;
    let report = p.redact(&[(0, [x0, top, x1, 792.0 - Y + 3.0])]).unwrap();
    // The word goes; the underline, crossing the mark's edge, is cut out of it (as any
    // drawing crossing a mark is), and is still drawn under "underlined".
    let done = &report.pages[0];
    assert_eq!(done.text, ["phrase"]);
    assert_eq!((done.clipped, done.paths), (1, 0));
    assert!(report.left.is_empty(), "{:?}", report.left);
    let ops = operators(&p);
    let band = ops.iter().position(|o| o.operator == "S").unwrap();
    assert!(ops[..band]
        .iter()
        .any(|o| o.operator == "W" || o.operator == "W*"));
}

#[test]
#[ignore = "opt-in: needs poppler"]
fn cross_check_new_texts_underline_with_poppler() {
    let mut p = page("");
    let text = "Signed and sealed";
    let part = limarepdf_core::TextPart {
        text: text.into(),
        font: limarepdf_core::PartFont::Choice(limarepdf_core::FontChoice::default()),
        size: 12.0,
        color: [0.0; 3],
        rise: 0.0,
        from: None,
        origins: Vec::new(),
    };
    let decos = [Decorated {
        kind: DecoKind::Underline,
        chars: chars_of(text, "Signed"),
        look: None,
    }];
    p.add_text_decorated(0, (72.0, 100.0), &[part], &decos)
        .unwrap();
    let bytes = p.snapshot().unwrap();
    let words = tools::run("pdftotext", &["{}", "-"], &bytes);
    assert_eq!(
        words.split_whitespace().collect::<Vec<_>>(),
        ["Signed", "and", "sealed"]
    );
    // Poppler draws the band where LimarePDF reads it: as a path whose top is 1.2 - 0.3
    // points under the baseline (y down: 100.9), drawn by Cairo into SVG.
    let svg = tools::run("pdftocairo", &["-svg", "{}", "-"], &bytes);
    let numbers: Vec<f32> = (svg.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')))
        .filter_map(|n| n.parse().ok())
        .collect();
    assert!(
        numbers
            .windows(2)
            .any(|w| near(w[0], 72.0, 0.6) && near(w[1], 100.9, 0.1)),
        "no band at the baseline's start in {svg}"
    );
}

const PROSE: &str = "The harbour was quiet that morning, and the fishing boats sat low in the \
    water while gulls argued over the nets drying along the wall. Nobody on the quay was in a \
    hurry; the tide would turn when it turned, and the old men who mended the nets had seen \
    enough tides to know that worrying never brought one in sooner than the moon allowed.";

/// `PROSE` broken greedily into lines of at most 300 points in Helvetica 11, drawn 13.2
/// points apart from (X, Y), with LibreOffice's underline under `word`.
fn underlined_paragraph(word: &str) -> Pdf {
    let mut rows: Vec<String> = Vec::new();
    for w in PROSE.split_whitespace() {
        match rows.last_mut() {
            Some(l) if limarepdf_core::text_width(&format!("{l} {w}"), 11.0) <= 300.0 => {
                l.push(' ');
                l.push_str(w);
            }
            _ => rows.push(w.to_string()),
        }
    }
    let mut content = String::new();
    for (k, l) in rows.iter().enumerate() {
        let y = Y - 13.2 * k as f32;
        content += &format!("BT /F1 11 Tf {X} {y} Td ({l}) Tj ET\n");
    }
    let p = page(&content);
    let line = (rows.iter())
        .position(|l| l.contains(word))
        .expect("the word's line");
    let o = lines(&p).into_iter().nth(line).unwrap();
    let text = o.text.clone().unwrap();
    let from = text[..text.find(word).unwrap()].chars().count();
    let to = from + word.chars().count();
    let x = o.baseline.unwrap().0;
    let (x0, x1) = (x + o.stops[from], x + o.stops[to]);
    let y = Y - 13.2 * line as f32;
    content += &format!(
        "q 1 0 0 1 {x0} {y} cm 0.6 w 0 0 0 RG 0 -1.3 m {} -1.3 l S Q\n",
        x1 - x0
    );
    page(&content)
}

/// The paragraph's text changed to `new` as typing changes it ([`limarepdf_core::TextPart`]s
/// keeping where the characters it starts and ends with were).
fn typed_paragraph(para: &limarepdf_core::Paragraph, new: &str) -> Vec<limarepdf_core::TextPart> {
    let (old, new): (Vec<char>, Vec<char>) = (para.text.chars().collect(), new.chars().collect());
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - head;
    let tail = (old.iter().rev().zip(new.iter().rev()))
        .take_while(|(a, b)| a == b)
        .count()
        .min(room);
    let mut parts: Vec<limarepdf_core::TextPart> = Vec::new();
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
            Some(p) if p.font == limarepdf_core::PartFont::Span(span) => {
                p.text.push(c);
                p.origins.push(origin);
            }
            _ => parts.push(limarepdf_core::TextPart {
                text: c.to_string(),
                font: limarepdf_core::PartFont::Span(span),
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

#[test]
fn a_paragraphs_rewrap_carries_an_underline_with_its_words() {
    let word = "gulls argued over the nets";
    let mut p = underlined_paragraph(word);
    assert_eq!(read(&p).len(), 1, "{:?}", read(&p));
    let blocks = p.page_blocks(0).unwrap();
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    let para = p.paragraph(0, &blocks[0], &p.document_words()).unwrap();
    let setting = p.paragraph_setting(&para).unwrap();
    // Words typed at the start move the underlined ones along, onto other lines.
    let new = format!("Early on a grey Sunday in March, {}", para.text);
    p.edit_paragraph(&setting, &typed_paragraph(&para, &new))
        .unwrap();
    // Read again, the underline is over the same words, on whichever lines they are now.
    let covered: Vec<String> = read(&p).into_iter().map(|(c, ..)| c).collect();
    assert_eq!(covered.join(" "), word, "{covered:?}");
    // Its old line no longer paints: each path painted is one of the underline's.
    let decorations: usize = lines(&p).iter().map(|l| l.decorations.len()).sum();
    assert_eq!(painted(&p).len(), decorations);
}

/// A paragraph read for editing says its lines' underlines over its own text (batch 46's 4b:
/// the app's paragraph editor shows and carries them).
#[test]
fn a_paragraph_says_its_lines_underlines_over_its_text() {
    let word = "gulls argued over the nets";
    let p = underlined_paragraph(word);
    let blocks = p.page_blocks(0).unwrap();
    let para = p.paragraph(0, &blocks[0], &p.document_words()).unwrap();
    assert_eq!(para.decorations.len(), 1, "{:?}", para.decorations);
    let d = &para.decorations[0];
    let covered: String = (para.text.chars())
        .skip(d.chars.0)
        .take(d.chars.1 - d.chars.0)
        .collect();
    assert_eq!((covered.as_str(), d.kind), (word, DecoKind::Underline));
    let (v, _) = d.look.bands()[0];
    assert!(near(v, -1.3 / 11.0, 0.002), "{:?}", d.look);
}

/// A paragraph whose words stay as they are, given its underline and one more (Ctrl+U in the
/// app's paragraph editor): no line is set anew, every text operator stays as it was, and
/// both underlines are read over their words.
#[test]
fn underlining_a_paragraphs_words_leaves_its_lines_as_they_were() {
    let word = "gulls argued over the nets";
    let mut p = underlined_paragraph(word);
    let before = text_ops(&operators(&p));
    let blocks = p.page_blocks(0).unwrap();
    let para = p.paragraph(0, &blocks[0], &p.document_words()).unwrap();
    let setting = p.paragraph_setting(&para).unwrap();
    let mut decos: Vec<Decorated> = (para.decorations.iter())
        .map(|d| Decorated {
            kind: d.kind,
            chars: d.chars,
            look: Some(d.look.clone()),
        })
        .collect();
    decos.push(Decorated {
        kind: DecoKind::Underline,
        chars: chars_of(&para.text, "fishing boats"),
        look: None,
    });
    let f = &para.frame;
    let edges = (f.left, f.right);
    (p.edit_paragraph_decorated(&setting, &para.parts(), edges, f.align, &decos)).unwrap();
    assert_eq!(text_ops(&operators(&p)), before);
    let mut covered: Vec<String> = read(&p).into_iter().map(|(c, ..)| c).collect();
    covered.sort();
    assert_eq!(covered, ["fishing boats", word], "{covered:?}");
}

/// Source Serif 4 (the test fonts') as an installed family.
fn serif() -> FontChoice {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/fonts/");
    let path = format!("{dir}SourceSerif4-Regular.otf");
    let n = limarepdf_core::add_font_files(&[std::path::Path::new(&path)])[0];
    FontChoice {
        family: limarepdf_core::Family::Installed(n),
        bold: false,
        italic: false,
    }
}

/// Where the app draws a band while typing is where it is written (batch 46's 4b): the place
/// a chosen font says ([`decoration_lines`]) is where new text in it is decorated, and the
/// place a line's own font says ([`limarepdf_core::LineFont::lines`]) is that too.
#[test]
fn the_places_fonts_say_are_where_bands_are_written() {
    let p = page(&shown(LINE));
    let fonts = p.line_fonts(0, lines(&p)[0].id).unwrap();
    assert_eq!(fonts[0].lines(), decoration_lines(FontChoice::default()));
    assert_eq!(fonts[0].lines().under, (-0.1, 0.05), "Helvetica's AFM");
    let text = "Signed and sealed";
    for f in [FontChoice::default(), serif()] {
        let mut p = page("");
        let part = limarepdf_core::TextPart {
            text: text.into(),
            font: limarepdf_core::PartFont::Choice(f),
            size: 12.0,
            color: [0.0; 3],
            rise: 0.0,
            from: None,
            origins: Vec::new(),
        };
        let deco = |kind, word| Decorated {
            kind,
            chars: chars_of(text, word),
            look: None,
        };
        let decos = [
            deco(DecoKind::Underline, "Signed"),
            deco(DecoKind::Strike, "sealed"),
        ];
        p.add_text_decorated(0, (72.0, 100.0), &[part], &decos)
            .unwrap();
        let said = decoration_lines(f);
        let got = read(&p);
        assert_eq!(got.len(), 2, "{got:?}");
        let close = |a: (f32, f32), b: (f32, f32)| near(a.0, b.0, 0.002) && near(a.1, b.1, 0.002);
        let name = f.family_name();
        assert!(close(got[0].2[0], said.under), "{name}: {got:?} {said:?}");
        assert!(close(got[1].2[0], said.strike), "{name}: {got:?} {said:?}");
        let l = lines(&p).remove(0);
        for font in p.line_fonts(0, l.id).unwrap() {
            let own = font.lines();
            let same = close(own.under, said.under) && close(own.strike, said.strike);
            assert!(same, "{name}");
        }
    }
    // An installed face says its own place, not the standard fonts'.
    assert_ne!(decoration_lines(serif()), FontLines::STANDARD);
}

/// The page drawn under a line being typed leaves out the line's underline with its words
/// (stage 4a's known gap), so the old band doesn't show where the line was.
#[test]
fn the_page_drawn_under_typing_leaves_out_a_lines_underline() {
    let p = page(&libreoffice(LINE, "underlined phrase", Some("in it")));
    let id = lines(&p)[0].id;
    let alone = Pdf::from_bytes(&p.page_without(0, &[id]).unwrap(), None).unwrap();
    let blank = |l: &PageObject| l.text.as_deref().unwrap_or("").trim().is_empty();
    assert!(lines(&alone).iter().all(blank));
    assert!(painted(&alone).is_empty(), "{:?}", painted(&alone));
}
