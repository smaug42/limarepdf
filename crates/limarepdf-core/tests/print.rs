//! What LimarePDF sends to the printer, and how it talks to `lp`. No printer is needed: `lp`
//! is replaced by a script that records its arguments and input. Every printout is laid out
//! on sheets of the paper by LimarePDF (ENH-14), so the sheets are checked against the
//! placements the model gives.

use limarepdf_core::print::{
    lp_args, send, sheets as plan, Binding, Booklet, BookletSides, Orientation, PageOrder,
    PageSetup, Paper, PerSheet, PrintOptions, Scale, Sides,
};
use limarepdf_core::{render, Cipher, Encryption, FieldValue, ObjectKind, Pdf, Restrictions};
use lopdf::{dictionary, Dictionary, Object, Stream};
#[cfg(unix)]
use std::path::PathBuf;

/// Letter, with the 12-point margins the user's HP reports.
fn letter() -> Paper {
    Paper {
        id: "na_letter_8.5x11in".into(),
        label: "Letter (8.5 × 11 in)".into(),
        size: (612.0, 792.0),
        margins: Some([12.0; 4]),
    }
}

fn setup(paper: Paper, orientation: Orientation, scale: Scale) -> PageSetup {
    PageSetup {
        paper,
        orientation,
        scale,
        per_sheet: PerSheet::default(),
        booklet: None,
    }
}

/// Paper exactly `w` x `h` points with no margins: a page that size goes on it as it is,
/// so a sheet can be read as its page.
fn as_is(w: f32, h: f32) -> PageSetup {
    let paper = Paper {
        id: "page".into(),
        label: "page".into(),
        size: (w, h),
        margins: Some([0.0; 4]),
    };
    setup(paper, Orientation::Automatic, Scale::ShrinkOversized)
}

/// Pages of three widths, the first with a red square at (50, 50)-(250, 150).
fn three_pages() -> Pdf {
    let mut p = Pdf::new();
    for (i, w) in [300.0, 400.0, 500.0].into_iter().enumerate() {
        p.insert_blank(i, w, 200.0).unwrap();
    }
    let page = p.page_ids()[0];
    let cs = p.doc.add_object(Stream::new(
        Dictionary::new(),
        b"1 0 0 rg 50 50 200 100 re f".to_vec(),
    ));
    p.doc.get_dictionary_mut(page).unwrap().set("Contents", cs);
    p
}

fn kinds(p: &Pdf, page: usize) -> Vec<ObjectKind> {
    let objs = p.page_objects(page).unwrap();
    objs.iter().map(|o| o.kind).collect()
}

fn widths(p: &Pdf) -> Vec<f32> {
    (0..p.page_count())
        .map(|i| p.page_size(i).unwrap().0.round())
        .collect()
}

/// A sheet drawn at 72 dpi, a pixel a point.
struct Pic {
    w: usize,
    h: usize,
    rgb: Vec<[u8; 3]>,
}

impl Pic {
    /// Colour at (x, y) points from the top-left.
    fn at(&self, x: usize, y: usize) -> [u8; 3] {
        self.rgb[y * self.w + x]
    }

    /// The box of the pixels `which` picks, in points from the sheet's bottom-left: left,
    /// bottom, right, top.
    fn inked(&self, which: impl Fn([u8; 3]) -> bool) -> Option<[f32; 4]> {
        let all = [0.0, 0.0, self.w as f32, self.h as f32];
        self.inked_in(which, all)
    }

    /// [`Pic::inked`] of the pixels whose middles are within the box `within` only.
    fn inked_in(&self, which: impl Fn([u8; 3]) -> bool, within: [f32; 4]) -> Option<[f32; 4]> {
        let [x0, y0, x1, y1] = within;
        let inside = |x: f32, y: f32| x >= x0 && x <= x1 && y >= y0 && y <= y1;
        self.inked_where(which, inside)
    }

    /// [`Pic::inked`] of the pixels whose middles, in points from the bottom-left, `keep`
    /// keeps.
    fn inked_where(
        &self,
        which: impl Fn([u8; 3]) -> bool,
        keep: impl Fn(f32, f32) -> bool,
    ) -> Option<[f32; 4]> {
        let h = self.h as f32;
        let inside = |x: usize, y: usize| keep(x as f32 + 0.5, h - y as f32 - 0.5);
        let mut b: Option<[usize; 4]> = None;
        for y in 0..self.h {
            for x in 0..self.w {
                if inside(x, y) && which(self.at(x, y)) {
                    let r = b.get_or_insert([x, y, x, y]);
                    *r = [r[0].min(x), r[1].min(y), r[2].max(x + 1), r[3].max(y + 1)];
                }
            }
        }
        let [x0, y0, x1, y1] = b?;
        let h = self.h as f32;
        Some([x0 as f32, h - y1 as f32, x1 as f32, h - y0 as f32])
    }
}

/// Sheet `i` of `pdf` as hayro draws it.
fn drawn(pdf: &[u8], i: usize) -> Pic {
    let hp = render::load(pdf.to_vec(), None).unwrap();
    let b = render::Renderer::new(&hp).render(i, 1.0).unwrap();
    let (px, _) = b.rgba.as_chunks::<4>();
    Pic {
        w: b.width,
        h: b.height,
        rgb: px.iter().map(|p| [p[0], p[1], p[2]]).collect(),
    }
}

/// Colour at (x, y) pt from the top-left of page 1 of `pdf`, rendered at 72 dpi.
fn pixel(pdf: Vec<u8>, x: usize, y: usize) -> [u8; 3] {
    drawn(&pdf, 0).at(x, y)
}

fn red(c: [u8; 3]) -> bool {
    c[0] > 200 && c[1] < 80 && c[2] < 80
}

fn blue(c: [u8; 3]) -> bool {
    c[2] > 200 && c[0] < 80 && c[1] < 80
}

/// Anything drawn but blue.
fn page_ink(c: [u8; 3]) -> bool {
    c.iter().any(|&v| v < 250) && !blue(c)
}

/// Add a page with full box `media`, cropped to `crop`, turned `rotate`: blue all over its
/// full box, grey over what is shown, and a red bar 10 points deep along its top as shown.
fn marked_page(p: &mut Pdf, media: [f32; 4], crop: Option<[f32; 4]>, rotate: i64) {
    let at = p.page_count();
    p.insert_blank(at, media[2] - media[0], media[3] - media[1])
        .unwrap();
    let [x0, y0, x1, y1] = crop.unwrap_or(media);
    // Turned clockwise a quarter turn, a page shows its left edge at the top.
    let bar = match rotate {
        90 => [x0, y0, 10.0, y1 - y0],
        180 => [x0, y0, x1 - x0, 10.0],
        270 => [x1 - 10.0, y0, 10.0, y1 - y0],
        _ => [x0, y1 - 10.0, x1 - x0, 10.0],
    };
    let n = |v: [f32; 4]| v.map(|x| x.to_string()).join(" ");
    let mb = [media[0], media[1], media[2] - media[0], media[3] - media[1]];
    let content = format!(
        "0 0 1 rg {} re f 0.8 g {} re f 1 0 0 rg {} re f",
        n(mb),
        n([x0, y0, x1 - x0, y1 - y0]),
        n(bar)
    );
    let cs = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.into_bytes()));
    let page = p.page_ids()[at];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", cs);
    d.set("MediaBox", media.map(Object::Real).to_vec());
    if let Some(c) = crop {
        d.set("CropBox", c.map(Object::Real).to_vec());
    }
    d.set("Rotate", rotate);
}

/// Pages of every kind the model has to place: upright, wide, turned each way by the page
/// itself, cropped, and much larger than the paper.
fn marked_pages() -> Pdf {
    let mut p = Pdf::new();
    marked_page(&mut p, [0.0, 0.0, 300.0, 400.0], None, 0);
    marked_page(&mut p, [0.0, 0.0, 500.0, 300.0], None, 0);
    marked_page(&mut p, [0.0, 0.0, 400.0, 600.0], None, 90);
    marked_page(&mut p, [0.0, 0.0, 500.0, 300.0], None, 270);
    let crop = [50.0, 50.0, 350.0, 250.0];
    marked_page(&mut p, [0.0, 0.0, 400.0, 400.0], Some(crop), 180);
    // A0, with its full box away from the origin.
    marked_page(&mut p, [100.0, 100.0, 2484.0, 3470.0], None, 0);
    p
}

/// `a` within 1.5 points of `b` (pixels straddling an edge are partly drawn).
fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1.5)
}

/// `r` cut to the sheet `size`, or `None` when none of it is on it.
fn on_sheet(r: [f32; 4], size: (f32, f32)) -> Option<[f32; 4]> {
    let c = [
        r[0].max(0.0),
        r[1].max(0.0),
        r[2].min(size.0),
        r[3].min(size.1),
    ];
    (c[2] - c[0] > 1.0 && c[3] - c[1] > 1.0).then_some(c)
}

/// Each sheet of `sheets`, as `draw` draws it, holds its pages of `doc` where the plan
/// ([`plan`]) puts them: each page's shown area grey (nothing of what its crop box leaves
/// out, which is blue), and its top's red bar along the side of its box its turns put it
/// at: the top, the left turned once, the bottom twice.
fn check_sheets(doc: &Pdf, sheets: &[u8], setup: &PageSetup, draw: impl Fn(&[u8], usize) -> Pic) {
    for (k, sheet) in plan(&doc.page_sizes(), setup).iter().enumerate() {
        let pic = draw(sheets, k);
        assert_eq!((pic.w, pic.h), (612, 792), "sheet {}", k + 1);
        // Where two pages touch, Poppler can fill the row of pixels past one's edge with
        // what its crop box leaves out: nowhere further.
        let on_an_edge = |x: f32, y: f32| {
            sheet.pages.iter().any(|(_, p)| {
                let [x0, y0, x1, y1] = p.rect;
                let on = |v: f32, a: f32, b: f32| v >= a - 1.0 && v <= b + 1.0;
                let near = |v: f32, e: f32| (v - e).abs() <= 1.0;
                (on(y, y0, y1) && (near(x, x0) || near(x, x1)))
                    || (on(x, x0, x1) && (near(y, y0) || near(y, y1)))
            })
        };
        let off_edges = |x: f32, y: f32| sheet.pages.len() < 2 || !on_an_edge(x, y);
        let past = pic.inked_where(blue, off_edges);
        assert_eq!(past, None, "sheet {}: past a crop box", k + 1);
        for &(i, placed) in &sheet.pages {
            let what = format!("page {} on sheet {} at {:?}", i + 1, k + 1, placed);
            let [x0, y0, x1, y1] = placed.rect;
            // Its own pixels: pages side by side can touch, so not those on its edges.
            let Some(own) = on_sheet([x0 + 0.5, y0 + 0.5, x1 - 0.5, y1 - 0.5], placed.sheet) else {
                panic!("{what}: not on the sheet");
            };
            let page = pic.inked_in(page_ink, own).expect("the page drawn");
            let want = on_sheet(placed.rect, placed.sheet).unwrap();
            assert!(near(page, want), "{what}: drawn at {page:?}");
            let bar = 10.0 * placed.scale;
            // Thinner than two pixels, the bar's colour mixes with the page's.
            if bar < 2.0 {
                continue;
            }
            let want = match placed.turns {
                1 => [x0, y0, x0 + bar, y1],
                2 => [x0, y0, x1, y0 + bar],
                3 => [x1 - bar, y0, x1, y1],
                _ => [x0, y1 - bar, x1, y1],
            };
            match on_sheet(want, placed.sheet) {
                Some(want) => {
                    let got = pic.inked_in(red, own).expect("the top drawn");
                    assert!(near(got, want), "{what}: its top at {got:?}, not {want:?}");
                }
                None => assert_eq!(pic.inked_in(red, own), None, "{what}: its top shown"),
            }
        }
    }
}

/// The page setups the sheets are checked with: every scale, every orientation, and pages
/// per sheet in every order, with borders and without.
fn setups() -> Vec<PageSetup> {
    let mut v = Vec::new();
    for scale in [
        Scale::ShrinkOversized,
        Scale::Fit,
        Scale::ActualSize,
        Scale::Custom(50),
        Scale::Custom(400),
    ] {
        v.push(setup(letter(), Orientation::Automatic, scale));
    }
    for o in [Orientation::Portrait, Orientation::Landscape] {
        v.push(setup(letter(), o, Scale::ShrinkOversized));
    }
    let mut unknown = letter();
    unknown.margins = None;
    v.push(setup(unknown, Orientation::Automatic, Scale::Fit));
    for (pages, orientation, order, borders) in [
        (2, Orientation::Automatic, PageOrder::Across, false),
        (4, Orientation::Portrait, PageOrder::Down, true),
        (6, Orientation::Landscape, PageOrder::AcrossFromRight, false),
        (16, Orientation::Automatic, PageOrder::DownFromRight, true),
    ] {
        let mut s = setup(letter(), orientation, Scale::ShrinkOversized);
        s.per_sheet = PerSheet {
            pages,
            order,
            borders,
        };
        v.push(s);
    }
    // NEW-05: booklets, both bindings, and the fronts and backs alone.
    for (binding, sides) in [
        (Binding::Left, BookletSides::Both),
        (Binding::Right, BookletSides::Both),
        (Binding::Left, BookletSides::Fronts),
        (Binding::Right, BookletSides::Backs),
    ] {
        let mut s = setup(letter(), Orientation::Automatic, Scale::ShrinkOversized);
        s.booklet = Some(Booklet { binding, sides });
        v.push(s);
    }
    v
}

#[test]
fn each_page_is_on_its_sheet_where_the_model_places_it() {
    let doc = marked_pages();
    let all: Vec<usize> = (0..doc.page_count()).collect();
    for s in setups() {
        let mut sheets = doc.print_sheets(&all, &s).unwrap();
        let count = plan(&doc.page_sizes(), &s).len();
        assert_eq!(widths(&sheets), vec![612.0; count], "{s:?}");
        check_sheets(&doc, &sheets.to_bytes().unwrap(), &s, drawn);
    }
}

/// Four pages a sheet: a sheet of four drawings, each page's vector content its own; six
/// pages make two sheets.
#[test]
fn several_pages_go_on_a_sheet_each_a_drawing() {
    let doc = marked_pages();
    let mut s = setup(letter(), Orientation::Automatic, Scale::Fit);
    s.per_sheet.pages = 4;
    let data = doc.print_data(&[0, 1, 2, 3, 4, 5], &s, false).unwrap();
    let sent = Pdf::from_bytes(&data, None).unwrap();
    assert_eq!(widths(&sent), [612.0, 612.0]);
    assert_eq!(kinds(&sent, 0), [ObjectKind::Form; 4]);
    assert_eq!(kinds(&sent, 1), [ObjectKind::Form; 2]);
}

/// The preview's one sheet, made alone with only its pages, is drawn as it is among all
/// the sheets.
#[test]
fn a_sheet_made_alone_is_drawn_as_among_all() {
    let doc = marked_pages();
    let pages = [5, 0, 1, 2, 3, 4];
    let mut s = setup(letter(), Orientation::Automatic, Scale::ShrinkOversized);
    s.per_sheet = PerSheet {
        pages: 4,
        order: PageOrder::Down,
        borders: true,
    };
    let all = doc.print_sheets(&pages, &s).unwrap().to_bytes().unwrap();
    let sizes: Vec<_> = pages.iter().map(|&i| doc.page_size(i).unwrap()).collect();
    for (k, mut sheet) in plan(&sizes, &s).into_iter().enumerate() {
        let own: Vec<usize> = sheet.pages.iter().map(|(i, _)| pages[*i]).collect();
        for (j, p) in sheet.pages.iter_mut().enumerate() {
            p.0 = j;
        }
        let mut alone = doc.print_planned(&own, &[sheet]).unwrap();
        let alone = alone.to_bytes().unwrap();
        assert!(
            drawn(&alone, 0).rgb == drawn(&all, k).rgb,
            "sheet {} differs",
            k + 1
        );
    }
}

/// NEW-05: a one-page booklet's back is blank, and is still a sheet: among all the sheets,
/// and made alone for the preview, with no pages of its own.
#[test]
fn a_booklets_blank_back_is_a_sheet() {
    let doc = marked_pages();
    let mut s = setup(letter(), Orientation::Automatic, Scale::ShrinkOversized);
    s.booklet = Some(Booklet::default());
    let mut sheets = doc.print_sheets(&[2], &s).unwrap();
    assert_eq!(widths(&sheets), [612.0, 612.0]);
    let all = sheets.to_bytes().unwrap();
    let back = plan(&[doc.page_size(2).unwrap()], &s).remove(1);
    assert!(back.pages.is_empty() && back.sideways);
    let mut alone = doc.print_planned(&[], &[back]).unwrap();
    assert_eq!(widths(&alone), [612.0]);
    let alone = alone.to_bytes().unwrap();
    let any = |c: [u8; 3]| c != [255, 255, 255];
    assert_eq!(drawn(&alone, 0).inked(any), None);
    assert!(drawn(&alone, 0).rgb == drawn(&all, 1).rgb);
}

/// With borders, a line is drawn along each page's edges; without, nothing is.
#[test]
fn borders_are_drawn_around_each_page() {
    let mut doc = Pdf::new();
    for i in 0..3 {
        doc.insert_blank(i, 300.0, 200.0).unwrap();
    }
    let mut s = setup(letter(), Orientation::Portrait, Scale::ShrinkOversized);
    s.per_sheet.pages = 4;
    for borders in [true, false] {
        s.per_sheet.borders = borders;
        let data = doc.print_data(&[0, 1, 2], &s, false).unwrap();
        let pic = drawn(&data, 0);
        let sheet = &plan(&[(300.0, 200.0); 3], &s)[0];
        // The darkest of the pixels around (x, y) points from the bottom-left.
        let darkest = |x: f32, y: f32| {
            let (x, y) = (x.round() as usize, (792.0 - y).round() as usize);
            let around = (x - 1..=x + 1).flat_map(|x| (y - 1..=y + 1).map(move |y| (x, y)));
            around.map(|(x, y)| pic.at(x, y)[0]).min().unwrap()
        };
        for (i, p) in &sheet.pages {
            let [x0, y0, x1, y1] = p.rect;
            let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            for (x, y) in [(x0, my), (x1, my), (mx, y0), (mx, y1)] {
                let v = darkest(x, y);
                assert_eq!(v < 230, borders, "page {} at ({x}, {y}): {v}", i + 1);
            }
        }
        // The fourth place, at the bottom right, is empty: no page, no border.
        let dark = |c: [u8; 3]| c.iter().all(|&v| v < 230);
        assert_eq!(pic.inked_in(dark, [310.0, 16.0, 596.0, 392.0]), None);
        assert!(borders || pic.inked(dark).is_none());
    }
}

#[test]
fn sends_the_chosen_pages_unencrypted() {
    let mut p = three_pages();
    p.set_encryption(Some(Encryption {
        user_password: "user".into(),
        owner_password: "owner".into(),
        cipher: Cipher::Aes128,
        restrictions: Restrictions::default(),
    }));
    let s = setup(letter(), Orientation::Portrait, Scale::ActualSize);
    let data = p.print_data(&[2, 0], &s, false).unwrap();
    // CUPS has no password, so the copy it gets must open without one.
    let sent = Pdf::from_bytes(&data, None).unwrap();
    assert_eq!(widths(&sent), [612.0, 612.0]);
    // Vector content stays vector: the page is a drawing on its sheet, not an image.
    assert_eq!(kinds(&sent, 1), [ObjectKind::Form]);
    // The square, page 1's (50, 50)-(250, 150), on the second sheet: the 300 x 200 page
    // centred, (156, 296) from the sheet's bottom-left. 306 x 396 from the top-left is in it.
    let c = drawn(&data, 1).at(306, 396);
    assert!(red(c), "{c:?}");
    // The document being printed is unchanged.
    assert_eq!(p.page_count(), 3);
    assert!(p.print_data(&[], &s, false).is_err());
}

#[test]
fn printing_leaves_the_document_as_it_was() {
    let p = marked_pages();
    let before = p.clone().to_bytes().unwrap();
    let all: Vec<usize> = (0..p.page_count()).collect();
    let s = setup(letter(), Orientation::Automatic, Scale::Fit);
    p.print_sheets(&all, &s).unwrap();
    p.print_data(&all, &s, false).unwrap();
    assert!(
        p.clone().to_bytes().unwrap() == before,
        "printing changed it"
    );
}

#[test]
fn a_page_that_leaves_its_drawing_turned_is_placed_as_it_is_shown() {
    // A page whose content turns the page upside down and never turns it back, as some PDF
    // makers write them (PDF allows it): a red square at its top-left.
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let page = p.page_ids()[0];
    let cs = p.doc.add_object(Stream::new(
        Dictionary::new(),
        b"1 0 0 -1 0 200 cm 1 0 0 rg 10 10 20 20 re f".to_vec(),
    ));
    p.doc.get_dictionary_mut(page).unwrap().set("Contents", cs);
    let s = setup(letter(), Orientation::Portrait, Scale::ActualSize);
    let sheets = p.print_data(&[0], &s, false).unwrap();
    // The page centred, its top-left at (156, 296) from the sheet's top-left.
    let got = drawn(&sheets, 0).inked(red).unwrap();
    assert!(near(got, [166.0, 466.0, 186.0, 486.0]), "{got:?}");
    // Nothing is left for CUPS to draw over it: no annotations, no form.
    let sent = Pdf::from_bytes(&sheets, None).unwrap();
    let page = sent.doc.get_dictionary(sent.page_ids()[0]).unwrap();
    assert!(page.get(b"Annots").is_err());
    assert!(sent.form_fields().is_empty());
}

#[test]
fn a_page_of_several_content_streams_is_one_drawing() {
    // The square drawn by three streams, the middle one compressed, as editors append them.
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let spaced = format!("1 0 0 rg{}10 10 20 20 re", " ".repeat(200));
    let mut middle = Stream::new(Dictionary::new(), spaced.into_bytes());
    middle.compress().unwrap();
    assert!(middle.dict.has(b"Filter"), "not compressed");
    let ids: Vec<Object> = [
        Stream::new(Dictionary::new(), b"q".to_vec()),
        middle,
        Stream::new(Dictionary::new(), b"f Q".to_vec()),
    ]
    .into_iter()
    .map(|s| p.doc.add_object(s).into())
    .collect();
    let page = p.page_ids()[0];
    p.doc.get_dictionary_mut(page).unwrap().set("Contents", ids);
    let sheets = p.print_data(&[0], &as_is(300.0, 200.0), false).unwrap();
    // At the page's own place: (10, 10)-(30, 30) from the bottom-left.
    let got = drawn(&sheets, 0).inked(red).unwrap();
    assert!(near(got, [10.0, 10.0, 30.0, 30.0]), "{got:?}");
}

#[test]
fn form_values_are_sent_drawn_in() {
    // Two filled text fields: one printed (flag 4), one only shown on screen.
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let field = |name: &str, y: i64, flags: i64| {
        let rect: Vec<Object> = [20, y, 280, y + 30].map(Object::from).to_vec();
        dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "F" => flags,
            "T" => Object::string_literal(name), "Rect" => rect,
        }
    };
    let ids = [
        p.doc.add_object(field("Name", 140, 4)),
        p.doc.add_object(field("Note", 40, 0)),
    ];
    let refs: Vec<Object> = ids.iter().map(|&i| i.into()).collect();
    let page = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", refs.clone());
    let helv = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let form = p.doc.add_object(dictionary! {
        "Fields" => refs,
        "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
    });
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
    p.set_field("Name", FieldValue::Text("Alice Example".into()))
        .unwrap();
    p.set_field("Note", FieldValue::Text("Screen only".into()))
        .unwrap();

    // Upright, so that its words read across.
    let s = setup(letter(), Orientation::Portrait, Scale::ShrinkOversized);
    let sent = Pdf::from_bytes(&p.print_data(&[0], &s, false).unwrap(), None).unwrap();
    // No fields left for CUPS to drop; the printed one is part of the sheet.
    assert!(sent.form_fields().is_empty());
    let text = sent.page_text(0);
    assert!(text.contains("Alice Example"), "{text:?}");
    assert!(!text.contains("Screen only"), "{text:?}");
    // The document being printed keeps its form.
    assert_eq!(p.form_fields().len(), 2);
}

#[test]
fn fields_that_would_print_empty_are_named_before_printing() {
    // Three filled fields with no drawing: two printed (flag 4), one only shown on screen.
    // LimarePDF can't draw the Hebrew values (written right to left); it can draw the
    // Latin one.
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    p.insert_blank(1, 300.0, 200.0).unwrap();
    let hebrew = || {
        let mut b = vec![0xFE, 0xFF];
        b.extend("שלום".encode_utf16().flat_map(u16::to_be_bytes));
        Object::String(b, lopdf::StringFormat::Hexadecimal)
    };
    let field = |name: &str, y: i64, flags: i64, value: Object| {
        let rect: Vec<Object> = [20, y, 280, y + 30].map(Object::from).to_vec();
        dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "F" => flags,
            "T" => Object::string_literal(name), "Rect" => rect, "V" => value,
        }
    };
    let ids = [
        p.doc.add_object(field("Name", 140, 4, hebrew())),
        p.doc.add_object(field("Note", 90, 0, hebrew())),
        p.doc
            .add_object(field("City", 40, 4, Object::string_literal("Oslo"))),
    ];
    let refs: Vec<Object> = ids.iter().map(|&i| i.into()).collect();
    let page = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", refs.clone());
    let form = p.doc.add_object(dictionary! {
        "Fields" => refs, "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
    });
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
    // Saving numbers an object of its own, so each save is of a copy.
    let before = p.clone().to_bytes().unwrap();

    assert_eq!(p.unprintable_fields(&[0, 1]), ["Name"]);
    assert!(p.unprintable_fields(&[1]).is_empty());
    assert!(
        p.clone().to_bytes().unwrap() == before,
        "asking changed the document"
    );

    // Printing anyway sends the page without that value, and with the others drawn in.
    let s = setup(letter(), Orientation::Automatic, Scale::ShrinkOversized);
    let sent = Pdf::from_bytes(&p.print_data(&[0], &s, false).unwrap(), None).unwrap();
    assert!(sent.form_fields().is_empty());
    let text = sent.page_text(0);
    assert!(text.contains("Oslo"), "{text:?}");
}

#[test]
fn filled_fields_that_have_no_drawing_are_drawn_both_ways() {
    // A printed field with a value and no drawing, in a file just opened, as the command
    // line prints it (the app draws such fields when it opens a file; the command line
    // doesn't).
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let rect: Vec<Object> = [20, 80, 280, 120].map(Object::from).to_vec();
    let w = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "F" => 4,
        "T" => Object::string_literal("City"), "V" => Object::string_literal("MMMMMMMM"),
        "Rect" => rect,
    });
    let page = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", vec![w.into()]);
    let form = p.doc.add_object(dictionary! {
        "Fields" => vec![w.into()], "DA" => Object::string_literal("/Helv 20 Tf 0 g"),
    });
    p.doc.catalog_mut().unwrap().set("AcroForm", form);
    let p = Pdf::from_bytes(&p.to_bytes().unwrap(), None).unwrap();
    assert!(p.unprintable_fields(&[0]).is_empty());

    // Dark pixels in the field's box (y 80-120 from the top), as sent both ways.
    let dark = |pdf: Vec<u8>| {
        let pic = drawn(&pdf, 0);
        (80..120)
            .flat_map(|y| (20..280).map(move |x| (x, y)))
            .filter(|&(x, y)| pic.at(x, y).iter().all(|&c| c < 128))
            .count()
    };
    let s = as_is(300.0, 200.0);
    assert!(
        dark(p.print_data(&[0], &s, false).unwrap()) > 0,
        "not drawn"
    );
    assert!(
        dark(p.print_data(&[0], &s, true).unwrap()) > 0,
        "blank as images"
    );
    // The document being printed keeps no drawing of LimarePDF's.
    assert!(p.doc.get_dictionary(w).unwrap().get(b"AP").is_err());
}

#[test]
fn as_image_sends_one_picture_per_sheet() {
    let p = three_pages();
    let s = setup(letter(), Orientation::Portrait, Scale::ActualSize);
    let data = p.print_data(&[0, 1], &s, true).unwrap();
    let sent = Pdf::from_bytes(&data, None).unwrap();
    assert_eq!(widths(&sent), [612.0, 612.0]);
    for i in 0..2 {
        assert_eq!(kinds(&sent, i), [ObjectKind::Image], "sheet {}", i + 1);
    }
    // The square survives, in red, where it is on the vector sheet.
    let c = pixel(data, 306, 396);
    assert!(red(c), "{c:?}");
}

/// A blue square annotation at (100, 50)-(200, 150) with these flags and subtype.
fn annotation(p: &mut Pdf, flags: i64, subtype: &str) {
    let ap = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
        },
        b"0 0 1 rg 0 0 10 10 re f".to_vec(),
    ));
    let rect: Vec<Object> = [100, 50, 200, 150].map(Object::from).to_vec();
    let a = p.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => subtype, "Rect" => rect, "F" => flags,
        "AP" => dictionary! { "N" => ap },
    });
    let page = p.page_ids()[0];
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Annots", vec![Object::Reference(a)]);
}

#[test]
fn only_annotations_meant_for_print_are_printed() {
    // Flag 4 = Print, 32 = NoView: printed, though not shown on screen; 2 = Hidden. A pop-up
    // note prints only as the comment it belongs to.
    let cases = [
        (4, "Square", true),
        (4 | 32, "Square", true),
        (0, "Square", false),
        (4 | 2, "Square", false),
        (4, "Popup", false),
    ];
    for (flags, subtype, printed) in cases {
        let mut p = three_pages();
        annotation(&mut p, flags, subtype);
        for as_image in [false, true] {
            let data = p.print_data(&[0], &as_is(300.0, 200.0), as_image).unwrap();
            let c = pixel(data, 150, 100);
            assert_eq!(
                blue(c),
                printed,
                "{subtype} {flags} as image {as_image}: {c:?}"
            );
        }
    }
}

#[test]
fn lp_arguments() {
    let o = PrintOptions {
        printer: Some("Office".into()),
        copies: 2,
        sides: Sides::LongEdge,
        as_image: false,
        title: "report.pdf".into(),
        ..Default::default()
    };
    let want = "-d Office -n 2 -o media=A4 -o print-scaling=none -o collate=true \
                -o sides=two-sided-long-edge -t report.pdf";
    assert_eq!(lp_args(&o, "A4").join(" "), want);
    // The default printer, one copy: no -d, no collate, explicitly one-sided, and the sheets
    // never scaled by CUPS.
    let o = PrintOptions::default();
    assert_eq!(
        lp_args(&o, "na_letter_8.5x11in").join(" "),
        "-n 1 -o media=na_letter_8.5x11in -o print-scaling=none -o sides=one-sided"
    );
}

/// NEW-05: a booklet prints both sides flipped on the short edge, whatever Print on says,
/// and its fronts or backs alone on one side.
#[test]
fn a_booklet_sets_the_sides_lp_is_given() {
    let mut o = PrintOptions {
        sides: Sides::LongEdge,
        booklet: Some(Booklet::default()),
        ..Default::default()
    };
    let sides = |o: &PrintOptions| lp_args(o, "A4").last().cloned().unwrap();
    assert_eq!(sides(&o), "sides=two-sided-short-edge");
    o.booklet = Some(Booklet {
        sides: BookletSides::Backs,
        ..Default::default()
    });
    assert_eq!(sides(&o), "sides=one-sided");
    o.booklet = None;
    assert_eq!(sides(&o), "sides=two-sided-long-edge");
}

/// A stand-in for `lp`: saves its arguments and input next to itself, prints a reply,
/// and fails when its first argument is `-d Missing`.
#[cfg(unix)]
fn fake_lp() -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("print");
    std::fs::create_dir_all(&dir).unwrap();
    let lp = dir.join("lp");
    let script = format!(
        "#!/bin/sh\n\
         if [ \"$2\" = Missing ]; then echo 'lp: The printer or class does not exist.' >&2; exit 1; fi\n\
         printf '%s\\n' \"$@\" > '{d}/args.txt'\n\
         cat > '{d}/job.pdf'\n\
         echo 'request id is Office-7 (1 file(s))'\n",
        d = dir.display()
    );
    std::fs::write(&lp, script).unwrap();
    std::fs::set_permissions(&lp, std::fs::Permissions::from_mode(0o755)).unwrap();
    lp
}

#[cfg(unix)]
#[test]
fn sends_the_document_to_lp() {
    let lp = fake_lp();
    let dir = lp.parent().unwrap();
    let p = three_pages();
    let data = p.print_data(&[1], &as_is(400.0, 200.0), false).unwrap();
    let o = PrintOptions {
        printer: Some("Office".into()),
        ..Default::default()
    };
    let args = lp_args(&o, "Letter");
    let reply = send(&lp, &args, &data).unwrap();
    assert_eq!(reply, "request id is Office-7 (1 file(s))");
    assert_eq!(std::fs::read(dir.join("job.pdf")).unwrap(), data);
    let got = std::fs::read_to_string(dir.join("args.txt")).unwrap();
    assert_eq!(got.lines().collect::<Vec<_>>(), args);

    // lp's own error reaches the user.
    let o = PrintOptions {
        printer: Some("Missing".into()),
        ..Default::default()
    };
    let e = send(&lp, &lp_args(&o, "Letter"), &data).unwrap_err();
    assert!(e.to_string().contains("does not exist"), "{e}");
}

#[test]
fn missing_lp_is_explained() {
    let e = send("limarepdf-no-such-lp", &[], b"%PDF").unwrap_err();
    assert!(e.to_string().contains("printing needs CUPS"), "{e}");
}

/// A scratch folder of this test's own.
#[cfg(unix)]
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Page `i` of the PDF file `path` as Poppler draws it at 72 dpi.
#[cfg(unix)]
fn poppler(path: &std::path::Path, i: usize) -> Pic {
    let stem = path.with_extension(format!("p{i}"));
    let png = path.with_extension(format!("p{i}.png"));
    let n = (i + 1).to_string();
    let ok = std::process::Command::new("pdftoppm")
        .args(["-r", "72", "-singlefile", "-png", "-f", &n, "-l", &n])
        .arg(path)
        .arg(&stem)
        .status()
        .expect("pdftoppm")
        .success();
    assert!(ok, "pdftoppm failed");
    let img = image::open(png).unwrap().to_rgb8();
    Pic {
        w: img.width() as usize,
        h: img.height() as usize,
        rgb: img.pixels().map(|p| p.0).collect(),
    }
}

/// Poppler draws the sheets as the model places their pages too. Opt-in: needs `pdftoppm`.
#[cfg(unix)]
#[test]
#[ignore]
fn cross_check_sheets_with_poppler() {
    let dir = scratch("sheets-poppler");
    let doc = marked_pages();
    let all: Vec<usize> = (0..doc.page_count()).collect();
    for (n, s) in setups().iter().enumerate() {
        let path = dir.join(format!("sheets-{n}.pdf"));
        let bytes = doc.print_sheets(&all, s).unwrap().to_bytes().unwrap();
        std::fs::write(&path, &bytes).unwrap();
        check_sheets(&doc, &bytes, s, |_, i| poppler(&path, i));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// The bytes of `http://localhost:631{path}`, asked without anything but the standard
/// library.
#[cfg(unix)]
fn local_cups(path: &str) -> Option<Vec<u8>> {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect("localhost:631").ok()?;
    let ask = format!("GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n");
    s.write_all(ask.as_bytes()).ok()?;
    let mut got = Vec::new();
    s.read_to_end(&mut got).ok()?;
    let end = got.windows(4).position(|w| w == b"\r\n\r\n")?;
    let ok = got.starts_with(b"HTTP/1.1 200") || got.starts_with(b"HTTP/1.0 200");
    ok.then(|| got[end + 4..].to_vec())
}

/// CUPS passes a sheet on as it is when sent with `print-scaling=none`, as `lp_args` sends
/// it: a Letter page printed at its actual size covers its whole sheet, corner to corner,
/// after CUPS's own filter (`cupsfilter`, what CUPS gives the printer's driver), with the
/// default printer's description file. Opt-in: needs CUPS, `cupsfilter`, `pdftoppm` and a
/// printer set up; nothing is sent to the printer.
#[cfg(unix)]
#[test]
#[ignore]
fn cross_check_sheets_pass_cupsfilter_unscaled() {
    let Some(queue) = limarepdf_core::print::printers()
        .ok()
        .and_then(|v| v.into_iter().find(|p| p.default))
    else {
        eprintln!("no default printer: nothing to check");
        return;
    };
    let dir = scratch("sheets-cupsfilter");
    let Some(ppd) = local_cups(&format!("/printers/{}.ppd", queue.name)) else {
        let name = queue.name;
        eprintln!("{name} has no description file to read: nothing to check");
        return;
    };
    std::fs::write(dir.join("queue.ppd"), ppd).unwrap();
    // A Letter page with black squares in its corners.
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    let corners = "0 g 0 0 36 36 re f 576 0 36 36 re f 0 756 36 36 re f 576 756 36 36 re f";
    let cs = p
        .doc
        .add_object(Stream::new(Dictionary::new(), corners.as_bytes().to_vec()));
    let page = p.page_ids()[0];
    p.doc.get_dictionary_mut(page).unwrap().set("Contents", cs);
    let mut paper = letter();
    paper.id = "Letter".into();
    let mut s = setup(paper, Orientation::Automatic, Scale::ActualSize);
    // The sheet `data` after CUPS's filter, as Poppler draws it.
    let filtered_with = |name: &str, data: Vec<u8>, o: &PrintOptions| {
        let sheet = dir.join(format!("{name}.pdf"));
        std::fs::write(&sheet, data).unwrap();
        let args = lp_args(o, "Letter");
        let options: Vec<&String> = args.iter().filter(|a| a.contains('=')).collect();
        let mut cmd = std::process::Command::new("cupsfilter");
        cmd.arg("-p").arg(dir.join("queue.ppd"));
        cmd.args(["-i", "application/pdf", "-m", "application/vnd.cups-pdf"]);
        for o in options {
            cmd.args(["-o", o]);
        }
        let out = cmd.arg(&sheet).output().expect("cupsfilter");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{err}");
        let filtered = dir.join(format!("{name}-filtered.pdf"));
        std::fs::write(&filtered, out.stdout).unwrap();
        poppler(&filtered, 0)
    };
    let filtered = |name: &str, data| filtered_with(name, data, &PrintOptions::default());
    let dark = |c: [u8; 3]| c.iter().all(|&v| v < 128);
    let pic = filtered("sheet", p.print_data(&[0], &s, false).unwrap());
    let ink = pic.inked(dark).expect("the corners");
    let whole = near(ink, [0.0, 0.0, 612.0, 792.0]);
    assert!(whole, "the corners at {ink:?}");
    // Four of the page a sheet: the corners of the four reach where the plan puts them.
    for i in 1..4 {
        p.insert_blank(i, 612.0, 792.0).unwrap();
        let page = p.page_ids()[i];
        p.doc.get_dictionary_mut(page).unwrap().set("Contents", cs);
    }
    s.per_sheet.pages = 4;
    let sheet = &plan(&[(612.0, 792.0); 4], &s)[0];
    let rects = sheet.pages.iter().map(|(_, p)| p.rect);
    let both = |a: [f32; 4], b: [f32; 4]| {
        let [x0, y0] = [a[0].min(b[0]), a[1].min(b[1])];
        [x0, y0, a[2].max(b[2]), a[3].max(b[3])]
    };
    let want = rects.reduce(both).unwrap();
    let pic = filtered("four", p.print_data(&[0, 1, 2, 3], &s, false).unwrap());
    let ink = pic.inked(dark).expect("the corners");
    assert!(near(ink, want), "the corners at {ink:?}, not {want:?}");
    // NEW-05: a booklet's first side, sent flipped on the short edge: its two pages where
    // the plan puts them, meeting at the fold.
    s.per_sheet.pages = 1;
    s.booklet = Some(Booklet::default());
    let o = PrintOptions {
        booklet: s.booklet,
        ..Default::default()
    };
    let front = &plan(&[(612.0, 792.0); 4], &s)[0];
    let want = (front.pages.iter().map(|(_, p)| p.rect))
        .reduce(both)
        .unwrap();
    let data = p.print_data(&[0, 1, 2, 3], &s, false).unwrap();
    let pic = filtered_with("booklet", data, &o);
    let ink = pic.inked(dark).expect("the corners");
    assert!(near(ink, want), "the corners at {ink:?}, not {want:?}");
    std::fs::remove_dir_all(dir).unwrap();
}
