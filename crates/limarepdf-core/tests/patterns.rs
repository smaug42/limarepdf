//! Tiling patterns whose tiles hayro sizes from their numbers (PB-11). hayro 0.8.0 sized a
//! tile's canvas from the pattern's step times the zoom, so PDFBox 2363's pattern, stepping
//! 32,767 units, asked for 7.6 GB at 96 dpi and panicked from 150 dpi (D12); 0.9.0 sizes it
//! from the area the pattern fills. Patterns 0.9.0 can't tile are left out of what is drawn
//! (`unfillable.rs`), however they are drawn: from a form, an annotation's appearance, a soft
//! mask, a Type 3 glyph or another pattern's cell.

use limarepdf_core::render::{self, Bitmap, Missing};
use limarepdf_core::Pdf;
use lopdf::{dictionary, Dictionary, Document, Object, Stream};

fn numbers(n: &[i64]) -> Object {
    Object::Array(n.iter().map(|&n| Object::Integer(n)).collect())
}

/// A 612 × 792 page drawing `content`, with the entries `build` gives it (its resources,
/// its annotations), given the document to add the objects they name to.
fn page_with(content: &str, build: impl FnOnce(&mut Document) -> Dictionary) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    let pid = p.page_ids()[0];
    let entries = build(&mut p.doc);
    let content = p
        .doc
        .add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
    let d = p.doc.get_dictionary_mut(pid).unwrap();
    d.set("Contents", Object::Reference(content));
    for (key, value) in entries {
        d.set(key, value);
    }
    p
}

/// Tiling pattern `pattern`, its cell painting red over `cell` (x, y, width, height) in the
/// pattern's space.
fn painting(pattern: Dictionary, cell: [i64; 4]) -> Stream {
    let [x, y, w, h] = cell;
    Stream::new(
        pattern,
        format!("1 0 0 rg {x} {y} {w} {h} re f").into_bytes(),
    )
}

/// The page filled with tiling pattern `pattern`, its cell painting red over `cell`.
fn patterned_with(pattern: Dictionary, cell: [i64; 4]) -> Pdf {
    page_with("/Pattern cs /P0 scn 0 0 612 792 re f", |doc| {
        let tile = doc.add_object(painting(pattern, cell));
        dictionary! { "Resources" => dictionary! { "Pattern" => dictionary! { "P0" => tile } } }
    })
}

/// The page filled with a tiling pattern as PDFBox 2363's object 12 is: turned a quarter, a
/// tile of `/BBox [-90 90 702 702]` stepping `step` units each way, painting red from 0 to
/// 600 across and 100 to 700 up, in the pattern's space. On the page, the red is from about
/// -15 to 585 across and 110 to 710 up.
fn patterned(step: i64) -> Pdf {
    let pattern = dictionary! {
        "Type" => "Pattern",
        "PatternType" => 1,
        "PaintType" => 1,
        "TilingType" => 2,
        "BBox" => numbers(&[-90, 90, 702, 702]),
        "XStep" => step,
        "YStep" => step,
        "Matrix" => numbers(&[0, 1, -1, 0, 685, 110]),
        "Resources" => dictionary! {},
    };
    patterned_with(pattern, [0, 100, 600, 600])
}

/// The pixel at (`x`, `y`) in points from the page's bottom left, drawn at `dpi`.
fn at(b: &Bitmap, dpi: f32, x: f32, y: f32) -> [u8; 4] {
    let k = dpi / 72.0;
    let (col, row) = ((x * k) as usize, ((792.0 - y) * k) as usize);
    let i = (row * b.width + col) * 4;
    b.rgba[i..i + 4].try_into().unwrap()
}

/// 2363's pattern draws at 96, 150 and 300 dpi: red where its one tile paints, white beside it.
#[test]
fn a_pattern_stepping_32767_units_draws() {
    let mut p = patterned(32767);
    let hp = render::load(p.app_snapshot().unwrap(), None).unwrap();
    let r = render::Renderer::new(&hp);
    for dpi in [96.0, 150.0, 300.0] {
        let b = r.render(0, dpi / 72.0).unwrap();
        assert_eq!(
            at(&b, dpi, 306.0, 396.0),
            [255, 0, 0, 255],
            "{dpi} dpi, inside"
        );
        assert_eq!(at(&b, dpi, 600.0, 50.0), [255; 4], "{dpi} dpi, beside");
        assert!(r.take_missing().is_empty(), "{dpi} dpi");
    }
}

/// Page `p` is drawn at 96 dpi white in its middle, a pattern reported as a picture left
/// out; the document keeps `pattern`.
fn left_out(mut p: Pdf, pattern: &Dictionary) {
    let hp = render::load(p.app_snapshot().unwrap(), None).unwrap();
    let r = render::Renderer::new(&hp);
    let b = r.render(0, 96.0 / 72.0).unwrap();
    assert_eq!(at(&b, 96.0, 306.0, 396.0), [255; 4]);
    assert!(r.take_missing().contains(&Missing::Picture));
    let kept = Pdf::from_bytes(&p.snapshot().unwrap(), None).unwrap();
    let kept = kept.doc.objects.values().any(|o| {
        o.as_stream().is_ok_and(|s| {
            let mut d = s.dict.clone();
            d.remove(b"Length");
            d == *pattern
        })
    });
    assert!(kept);
}

/// A cell 100,000 units a side stepping 1 would be drawn ten billion times into its tile
/// (drawing it ran past five minutes; failed on the old code).
#[test]
fn a_pattern_of_100000_steps_a_side_is_left_out() {
    let pattern = dictionary! {
        "Type" => "Pattern",
        "PatternType" => 1,
        "PaintType" => 1,
        "TilingType" => 1,
        "BBox" => numbers(&[0, 0, 100_000, 100_000]),
        "XStep" => 1,
        "YStep" => 1,
        "Resources" => dictionary! {},
    };
    left_out(patterned_with(pattern.clone(), [0, 0, 1, 1]), &pattern);
}

/// A pattern 60,000 units across and 700 up, by `matrix`.
fn wide(matrix: [i64; 6]) -> Dictionary {
    dictionary! {
        "Type" => "Pattern",
        "PatternType" => 1,
        "PaintType" => 1,
        "TilingType" => 1,
        "BBox" => numbers(&[0, 0, 60_000, 700]),
        "XStep" => 60_000,
        "YStep" => 700,
        "Matrix" => numbers(&matrix),
        "Resources" => dictionary! {},
    }
}

const UNSKEWED: [i64; 6] = [1, 0, 0, 1, 0, 0];
const SKEWED: [i64; 6] = [1, 0, 80, 1, 0, 0];

/// A pattern skewed 80 to 1 fills 64,000 of its units across the page, so its tile is as
/// wide as its step of 60,000, and its step up of 700 is 56,000 pixels long: 13 GB at 72 dpi
/// (asked for, on the old code); at 96, over 65,532 pixels a side, hayro panicked. Either
/// the rule on a tile's side or the one on its pixels leaves it out.
#[test]
fn a_skewed_pattern_whose_tile_would_take_gigabytes_is_left_out() {
    left_out(
        patterned_with(wide(SKEWED), [0, 0, 60_000, 700]),
        &wide(SKEWED),
    );
}

/// Resources naming the unskewed wide pattern, all red, `P0`.
fn wide_resources(doc: &mut Document) -> Dictionary {
    let tile = doc.add_object(painting(wide(UNSKEWED), [0, 0, 60_000, 700]));
    dictionary! { "Pattern" => dictionary! { "P0" => tile } }
}

/// Filling with `P0` all a page drawn skewed 80 to 1 shows: from -70,000 to 612 across and 0
/// to 792 up.
const FILL: &str = "/Pattern cs /P0 scn -70000 0 70612 792 re f";

/// A form filling with the unskewed wide pattern, with the entries `more` too.
fn form(doc: &mut Document, more: Dictionary) -> Object {
    let mut d = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => numbers(&[-70_000, 0, 1000, 792]),
        "Resources" => wide_resources(doc),
    };
    d.extend(&more);
    doc.add_object(Stream::new(d, FILL.as_bytes().to_vec()))
        .into()
}

/// The page draws form `F0`, which `more` adds to the form's entries.
fn drawing_form(content: &str, more: Dictionary) -> Pdf {
    page_with(content, |doc| {
        let f = form(doc, more);
        dictionary! { "Resources" => dictionary! { "XObject" => dictionary! { "F0" => f } } }
    })
}

/// Unskewed, the wide pattern's tile is the page across and 700 up: drawn from a form, it
/// draws (the control for the tests below).
#[test]
fn a_wide_pattern_in_a_form_drawn_unskewed_draws() {
    let mut p = drawing_form("/F0 Do", dictionary! {});
    let hp = render::load(p.app_snapshot().unwrap(), None).unwrap();
    let r = render::Renderer::new(&hp);
    let b = r.render(0, 96.0 / 72.0).unwrap();
    assert_eq!(at(&b, 96.0, 306.0, 396.0), [255, 0, 0, 255]);
    assert!(r.take_missing().is_empty());
}

/// The wide pattern, unskewed itself, is skewed 80 to 1 by the `cm` the form is drawn with
/// (failed without the walk through what pages draw: hayro panicked).
#[test]
fn a_pattern_in_a_form_drawn_skewed_is_left_out() {
    left_out(
        drawing_form("q 1 0 80 1 0 0 cm /F0 Do Q", dictionary! {}),
        &wide(UNSKEWED),
    );
}

/// The wide pattern is skewed by the matrix of the form it is in (failed without the walk:
/// hayro panicked).
#[test]
fn a_pattern_in_a_skewed_form_is_left_out() {
    let p = drawing_form("/F0 Do", dictionary! { "Matrix" => numbers(&SKEWED) });
    left_out(p, &wide(UNSKEWED));
}

/// The wide pattern is skewed by the matrix of the annotation appearance it is in, which its
/// rectangle, the skewed box itself, doesn't undo (failed without the walk: hayro panicked).
#[test]
fn a_pattern_in_a_skewed_appearance_is_left_out() {
    let p = page_with("", |doc| {
        let appearance = form(doc, dictionary! { "Matrix" => numbers(&SKEWED) });
        let annot = doc.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Stamp",
            "Rect" => numbers(&[-70_000, 0, 1000 + 80 * 792, 792]),
            "AP" => dictionary! { "N" => appearance },
        });
        dictionary! { "Annots" => vec![annot.into()] }
    });
    left_out(p, &wide(UNSKEWED));
}

/// The wide pattern is skewed by the `cm` in force when a soft mask drawing it is set; the
/// mask hides the blue fill (failed without the walk: hayro panicked).
#[test]
fn a_pattern_in_a_soft_mask_set_skewed_is_left_out() {
    let content = "q 1 0 80 1 0 0 cm /G0 gs 0 0 1 rg -70000 0 70612 792 re f Q";
    let p = page_with(content, |doc| {
        let group = dictionary! { "S" => "Transparency", "CS" => "DeviceRGB" };
        let g = form(doc, dictionary! { "Group" => group });
        let mask = dictionary! { "Type" => "Mask", "S" => "Luminosity", "G" => g };
        let state = dictionary! { "Type" => "ExtGState", "SMask" => mask };
        dictionary! { "Resources" => dictionary! { "ExtGState" => dictionary! { "G0" => state } } }
    });
    left_out(p, &wide(UNSKEWED));
}

/// The wide pattern is skewed by the text matrix a Type 3 glyph drawing it is shown with
/// (failed without the walk: hayro panicked).
#[test]
fn a_pattern_in_a_type3_glyph_shown_skewed_is_left_out() {
    let p = page_with("BT /T3 1 Tf 1 0 80 1 0 0 Tm (a) Tj ET", |doc| {
        let glyph = format!("0 0 d0 {FILL}").into_bytes();
        let glyph = doc.add_object(Stream::new(dictionary! {}, glyph));
        let font = dictionary! {
            "Type" => "Font",
            "Subtype" => "Type3",
            "FontBBox" => numbers(&[0, 0, 0, 0]),
            "FontMatrix" => numbers(&UNSKEWED),
            "CharProcs" => dictionary! { "a" => glyph },
            "Encoding" => dictionary! {
                "Type" => "Encoding",
                "Differences" => vec![97.into(), Object::Name(b"a".to_vec())],
            },
            "FirstChar" => 97,
            "LastChar" => 97,
            "Widths" => numbers(&[0]),
            "Resources" => wide_resources(doc),
        };
        let font = doc.add_object(font);
        dictionary! { "Resources" => dictionary! { "Font" => dictionary! { "T3" => font } } }
    });
    left_out(p, &wide(UNSKEWED));
}

/// A pattern of 100-unit cells, its matrix `matrix`, with `resources`.
fn square(matrix: [i64; 6], resources: Dictionary) -> Dictionary {
    dictionary! {
        "Type" => "Pattern",
        "PatternType" => 1,
        "PaintType" => 1,
        "TilingType" => 1,
        "BBox" => numbers(&[0, 0, 100, 100]),
        "XStep" => 100,
        "YStep" => 100,
        "Matrix" => numbers(&matrix),
        "Resources" => resources,
    }
}

/// The page filled with a pattern skewed 2 to 1 whose cell fills with pattern `inner`,
/// painting red.
fn in_a_cell(inner: &Dictionary) -> Pdf {
    page_with("/Pattern cs /P0 scn 0 0 612 792 re f", |doc| {
        let p1 = doc.add_object(painting(inner.clone(), [0, 0, 100, 100]));
        let resources = dictionary! { "Pattern" => dictionary! { "P1" => p1 } };
        let outer = square([1, 0, 2, 1, 0, 0], resources);
        let cell = b"/Pattern cs /P1 scn 0 0 100 100 re f".to_vec();
        let p0 = doc.add_object(Stream::new(outer, cell));
        dictionary! { "Resources" => dictionary! { "Pattern" => dictionary! { "P0" => p0 } } }
    })
}

/// Unskewed, a pattern in the cell of one skewed 2 to 1 draws (the control for the test
/// below).
#[test]
fn a_pattern_in_a_skewed_patterns_cell_draws() {
    let mut p = in_a_cell(&square(UNSKEWED, dictionary! {}));
    let hp = render::load(p.app_snapshot().unwrap(), None).unwrap();
    let r = render::Renderer::new(&hp);
    let b = r.render(0, 96.0 / 72.0).unwrap();
    assert_eq!(at(&b, 96.0, 306.0, 396.0), [255, 0, 0, 255]);
    assert!(r.take_missing().is_empty());
}

/// On a pass of 2,048 pixels a side, at any zoom, a pattern skewed 3 to 1 has a tile of up to
/// 8,192 by 6,476 pixels and one skewed 2 to 1 up to 6,144 by 4,579; both draw. In the cell
/// of the second, the first is drawn on its tile, which skews it further: its tile would be
/// up to 12,288 by 7,663 pixels, 377 MB (failed without the walk, and without the rule on a
/// tile's pixels: drawn).
#[test]
fn a_pattern_in_a_skewed_patterns_cell_is_left_out() {
    let inner = square([1, 0, 3, 1, 0, 0], dictionary! {});
    left_out(in_a_cell(&inner), &inner);
}

/// A pattern skewed 40 to 1 stepping 110 units up has a tile 40 times as long as what the
/// pass it is drawn on shows of it up, to a step: at 2,400% zoom, 85 units of a pass of 2,048
/// pixels, 81,946 pixels (failed without the rule on a tile's side: hayro panicked).
#[test]
fn a_pattern_whose_tile_would_be_too_long_is_left_out() {
    let mut pattern = square([1, 0, 40, 1, 0, 0], dictionary! {});
    pattern.set("BBox", numbers(&[0, 0, 2, 110]));
    pattern.set("XStep", 2);
    pattern.set("YStep", 110);
    let mut p = patterned_with(pattern.clone(), [0, 0, 2, 110]);
    let hp = render::load(p.app_snapshot().unwrap(), None).unwrap();
    let r = render::Renderer::new(&hp);
    let region = render::Region {
        x: 0,
        y: 0,
        width: 512,
        height: 2400,
    };
    let b = r.render_region(0, 24.0, region).unwrap();
    assert_eq!(b.rgba[..4], [255; 4]);
    assert!(r.take_missing().contains(&Missing::Picture));
    left_out(p, &pattern);
}
