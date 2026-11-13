//! Shapes filled with a pattern of a triangle or patch shading (a mesh), drawn as the shading
//! clipped to the shape (PB-11, `mesh_fill.rs`). hayro 0.9 samples such a pattern into a
//! picture of all the shape covers, held until the pass ends; pdf.js's bug1260585, filling a
//! figure 2,640 times with a pattern of one triangle each, took 6 GB at 288 dpi. Each test
//! draws a page from the document as it is and from the copy pages are drawn from, where the
//! fills are redrawn with `sh`, and finds them the same.

use limarepdf_core::render::{self, Bitmap};
use limarepdf_core::Pdf;
use lopdf::content::Content;
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};

fn numbers(n: &[f32]) -> Object {
    Object::Array(n.iter().map(|&n| Object::Real(n)).collect())
}

/// A shading of triangles, each corner at a point on the page with its colour.
fn triangles(doc: &mut Document, corners: &[((f32, f32), [u8; 3])]) -> ObjectId {
    let mut data = Vec::new();
    for &((x, y), rgb) in corners {
        data.push(0);
        data.extend(((x / 612.0 * 65535.0) as u16).to_be_bytes());
        data.extend(((y / 792.0 * 65535.0) as u16).to_be_bytes());
        data.extend(rgb);
    }
    let d = dictionary! {
        "ShadingType" => 4,
        "ColorSpace" => "DeviceRGB",
        "BitsPerCoordinate" => 16,
        "BitsPerComponent" => 8,
        "BitsPerFlag" => 8,
        "Decode" => numbers(&[0.0, 612.0, 0.0, 792.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0]),
    };
    doc.add_object(Stream::new(d, data))
}

/// Two triangles from (`x`, `y`) to (`x + w`, `y + h`): red, green, blue and yellow corners.
fn square(doc: &mut Document, [x, y, w, h]: [f32; 4]) -> ObjectId {
    let (a, b, c, d) = ((x, y), (x + w, y), (x, y + h), (x + w, y + h));
    let (red, green, blue, yellow) = ([255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]);
    let corners = [
        (a, red),
        (b, green),
        (c, blue),
        (b, green),
        (c, blue),
        (d, yellow),
    ];
    triangles(doc, &corners)
}

/// A pattern of shading `shading`, with the entries `more`.
fn pattern(shading: ObjectId, more: Dictionary) -> Dictionary {
    let mut d = dictionary! { "Type" => "Pattern", "PatternType" => 2, "Shading" => shading };
    d.extend(&more);
    d
}

/// Resources naming pattern `M0`: [`square`] over `area`, with the entries `more`.
fn resources(doc: &mut Document, area: [f32; 4], more: Dictionary) -> Dictionary {
    let shading = square(doc, area);
    dictionary! { "Pattern" => dictionary! { "M0" => pattern(shading, more) } }
}

/// A 612 × 792 page drawing `contents`, with the entries `build` gives it, given the document
/// to add the objects they name to.
fn page_with(contents: &[&str], build: impl FnOnce(&mut Document) -> Dictionary) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, 612.0, 792.0).unwrap();
    let pid = p.page_ids()[0];
    let entries = build(&mut p.doc);
    let contents: Vec<Object> = (contents.iter())
        .map(|c| {
            p.doc
                .add_object(Stream::new(dictionary! {}, c.as_bytes().to_vec()))
                .into()
        })
        .collect();
    let d = p.doc.get_dictionary_mut(pid).unwrap();
    d.set("Contents", contents);
    for (key, value) in entries {
        d.set(key, value);
    }
    p
}

/// The page drawing `content`, which fills with `M0`: [`square`] over `area`, by a pattern
/// with the entries `more`.
fn filled(content: &str, area: [f32; 4], more: Dictionary) -> Pdf {
    page_with(
        &[content],
        |doc| dictionary! { "Resources" => resources(doc, area, more) },
    )
}

/// The first page of `bytes` drawn at 144 dpi.
fn drawn(bytes: Vec<u8>) -> Bitmap {
    let hp = render::load(bytes, None).unwrap();
    render::Renderer::new(&hp).render(0, 2.0).unwrap()
}

/// How many operators `op` the drawing copy of `p`'s objects draw.
fn count(p: &mut Pdf, op: &str) -> usize {
    let copy = Document::load_mem(&p.drawing_snapshot().unwrap()).unwrap();
    let ops = |s: &Stream| {
        let data = s
            .decompressed_content()
            .unwrap_or_else(|_| s.content.clone());
        Content::decode(&data).map_or(0, |c| {
            c.operations.iter().filter(|o| o.operator == op).count()
        })
    };
    copy.objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .map(ops)
        .sum()
}

/// Page `p` is drawn the same from the document as from the drawing copy, where `fills`
/// fills are drawn with `sh`; and the document is left as it was.
fn same(mut p: Pdf, fills: usize) {
    let before = p.snapshot().unwrap();
    let (a, b) = (drawn(before.clone()), drawn(p.drawing_snapshot().unwrap()));
    assert_eq!(p.snapshot().unwrap(), before);
    assert_eq!(count(&mut p, "sh"), fills);
    assert_eq!((a.width, a.height), (b.width, b.height));
    let apart = (a.rgba.chunks(4).zip(b.rgba.chunks(4)))
        .map(|(x, y)| x.iter().zip(y).map(|(x, y)| x.abs_diff(*y)).max().unwrap())
        .filter(|&d| d > 2)
        .count();
    let colours = a.rgba.chunks(4).filter(|x| x[..3] != [255; 3]).count();
    assert!(colours > 0);
    // A clip's edges are smoothed a little differently from a fill's: a skewed shape's
    // differ in 933 pixels of 1.9 million.
    assert!(apart * 1000 <= a.width * a.height, "{apart} pixels apart");
}

/// The control: a pattern filling the page, of a shading over part of it.
#[test]
fn a_fill_is_drawn_as_the_shading() {
    let content = "/Pattern cs /M0 scn 0 0 612 792 re f";
    same(
        filled(content, [100.0, 150.0, 400.0, 500.0], dictionary! {}),
        1,
    );
}

/// The shading is placed by the pattern's matrix, in the page's space, whatever the `cm`
/// in force when the shape is filled.
#[test]
fn a_fill_keeps_the_patterns_space() {
    let content =
        "q 1 0 0.3 1 20 -30 cm 0.9 0 0 1.1 0 0 cm /Pattern cs /M0 scn 50 60 400 500 re f Q";
    let matrix = dictionary! { "Matrix" => numbers(&[0.8, 0.2, 0.0, 0.9, 60.0, 40.0]) };
    same(filled(content, [0.0, 0.0, 500.0, 700.0], matrix), 1);
}

/// A shape that sets the clip still sets it when filled with the shading: the blue fill
/// after is clipped to it.
#[test]
fn a_fill_setting_the_clip_sets_it() {
    let content = "/Pattern cs /M0 scn 50 50 300 400 re W f 0 0 1 rg 0 0 612 792 re f";
    same(filled(content, [0.0, 0.0, 400.0, 600.0], dictionary! {}), 1);
}

/// A shape filled and outlined is outlined in green over the shading.
#[test]
fn a_fill_outlined_is_outlined() {
    let content = "/Pattern cs /M0 scn 0 0.5 0 RG 12 w 100 100 300 400 re B";
    same(
        filled(content, [50.0, 50.0, 450.0, 600.0], dictionary! {}),
        1,
    );
}

/// A shape filled by the even-odd rule keeps its hole.
#[test]
fn a_fill_by_the_even_odd_rule_keeps_its_hole() {
    let content = "/Pattern cs /M0 scn 100 100 400 500 re 200 200 150 200 re f*";
    same(
        filled(content, [50.0, 50.0, 500.0, 650.0], dictionary! {}),
        1,
    );
}

/// Painted in a form drawn shifted and scaled, the shading is in the form's space, the
/// pattern's.
#[test]
fn a_fill_in_a_form_is_drawn_as_the_shading() {
    let p = page_with(&["q 1 0 0 1 50 30 cm /F0 Do Q"], |doc| {
        let form = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => numbers(&[0.0, 0.0, 612.0, 792.0]),
            "Matrix" => numbers(&[0.8, 0.0, 0.0, 0.8, 10.0, 10.0]),
            "Resources" => resources(doc, [0.0, 0.0, 450.0, 650.0], dictionary! {}),
        };
        let content = b"q 1 0 0 1 20 0 cm /Pattern cs /M0 scn 0 0 500 600 re f Q".to_vec();
        let f = doc.add_object(Stream::new(form, content));
        dictionary! { "Resources" => dictionary! { "XObject" => dictionary! { "F0" => f } } }
    });
    same(p, 1);
}

/// A page drawn from two streams, the fill begun in the first and finished in the second,
/// its resources an object of their own.
#[test]
fn a_fill_across_a_pages_streams_is_drawn_as_the_shading() {
    let contents = [
        "q 1 0 0 1 40 40 cm /Pattern cs",
        "/M0 scn 0 0 400 500 re f Q",
    ];
    let p = page_with(&contents, |doc| {
        let r = resources(doc, [50.0, 50.0, 450.0, 600.0], dictionary! {});
        dictionary! { "Resources" => doc.add_object(r) }
    });
    same(p, 1);
}

/// An annotation's appearance fills with the pattern.
#[test]
fn a_fill_in_an_appearance_is_drawn_as_the_shading() {
    let p = page_with(&[""], |doc| {
        let form = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => numbers(&[0.0, 0.0, 400.0, 600.0]),
            "Resources" => resources(doc, [0.0, 0.0, 400.0, 600.0], dictionary! {}),
        };
        let content = b"/Pattern cs /M0 scn 0 0 400 600 re f".to_vec();
        let appearance = doc.add_object(Stream::new(form, content));
        let annot = doc.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Stamp",
            "Rect" => numbers(&[100.0, 100.0, 500.0, 700.0]),
            "AP" => dictionary! { "N" => appearance },
        });
        dictionary! { "Annots" => vec![annot.into()] }
    });
    same(p, 1);
}

/// A tiling pattern's cell fills with the mesh pattern: the shading is in the cell's space.
#[test]
fn a_fill_in_a_tiling_patterns_cell_is_drawn_as_the_shading() {
    let p = page_with(&["/Pattern cs /P0 scn 0 0 612 792 re f"], |doc| {
        let tiling = dictionary! {
            "Type" => "Pattern",
            "PatternType" => 1,
            "PaintType" => 1,
            "TilingType" => 1,
            "BBox" => numbers(&[0.0, 0.0, 200.0, 200.0]),
            "XStep" => 200,
            "YStep" => 200,
            "Matrix" => numbers(&[1.0, 0.0, 0.0, 1.0, 6.0, 0.0]),
            "Resources" => resources(doc, [20.0, 20.0, 160.0, 160.0], dictionary! {}),
        };
        let cell = b"/Pattern cs /M0 scn 0 0 200 200 re f".to_vec();
        let p0 = doc.add_object(Stream::new(tiling, cell));
        dictionary! { "Resources" => dictionary! { "Pattern" => dictionary! { "P0" => p0 } } }
    });
    same(p, 1);
}

/// `sh` leaves out a shading's background, so a pattern of a shading with one is filled as
/// it is.
#[test]
fn a_fill_with_a_background_is_left_as_it_is() {
    let content = "/Pattern cs /M0 scn 0 0 612 792 re f";
    let mut p = filled(content, [100.0, 100.0, 300.0, 300.0], dictionary! {});
    for o in p.doc.objects.values_mut() {
        match o.as_stream_mut() {
            Ok(s) if s.dict.has(b"ShadingType") => s.dict.set("Background", numbers(&[0.0; 3])),
            _ => {}
        }
    }
    same(p, 0);
}

/// A stream two pages draw, with resources of their own, is drawn as it is on both.
#[test]
fn a_stream_two_pages_draw_is_left_as_it_is() {
    let content = "/Pattern cs /M0 scn 0 0 612 792 re f";
    let mut p = filled(content, [100.0, 100.0, 300.0, 300.0], dictionary! {});
    p.insert_blank(1, 612.0, 792.0).unwrap();
    let ids = p.page_ids();
    let first = p.doc.get_dictionary(ids[0]).unwrap().clone();
    let second = p.doc.get_dictionary_mut(ids[1]).unwrap();
    second.set("Contents", first.get(b"Contents").unwrap().clone());
    second.set("Resources", first.get(b"Resources").unwrap().clone());
    assert_eq!(count(&mut p, "sh"), 0);
}
