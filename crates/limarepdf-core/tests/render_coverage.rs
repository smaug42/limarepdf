//! What the renderer (hayro) draws correctly, checked by pixel.
//!
//! Each test asserts what ISO 32000 says should appear, not what hayro happens
//! to do. Cases hayro gets wrong are `#[ignore]`d with the reason; run
//! `cargo test -p limarepdf-core --test render_coverage -- --ignored` after a hayro
//! upgrade to see whether any gap has closed.

use limarepdf_core::{render, Cipher, Encryption, Pdf, Restrictions};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};

const W: f32 = 300.0;
const H: f32 = 200.0;
const TOL: i32 = 12;

type Rgb = [f64; 3];

/// A one-page document with `content` and `resources`.
fn make(content: &str, resources: Dictionary) -> Pdf {
    let mut p = Pdf::new();
    p.insert_blank(0, W, H).unwrap();
    let page = p.page_ids()[0];
    let s = p
        .doc
        .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let d = p.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", Object::Reference(s));
    d.set("Resources", resources);
    p
}

/// Sets every entry of `extra` in `d`, replacing existing keys. (lopdf's
/// `Dictionary::extend` turns a repeated key into an array instead.)
fn merge(d: &mut Dictionary, extra: Dictionary) {
    for (k, v) in extra {
        d.set(k, v);
    }
}

fn set_resources(p: &mut Pdf, res: Dictionary) {
    let page = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Resources", res);
}

fn form(p: &mut Pdf, bbox: [f32; 4], content: &str, extra: Dictionary) -> ObjectId {
    let mut d = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => bbox.iter().map(|&v| Object::Real(v)).collect::<Vec<_>>(),
    };
    merge(&mut d, extra);
    p.doc
        .add_object(Stream::new(d, content.as_bytes().to_vec()))
}

fn gs(entries: Dictionary) -> Dictionary {
    let mut d = dictionary! { "Type" => "ExtGState" };
    merge(&mut d, entries);
    d
}

fn nums(v: &[f32]) -> Object {
    Object::Array(v.iter().map(|&x| Object::Real(x)).collect())
}

struct Shot(render::Bitmap);

impl Shot {
    /// Pixel at a point in PDF user space (y up).
    fn at(&self, x: f32, y: f32) -> [u8; 3] {
        let (px, py) = (x as usize, (H - y) as usize);
        let i = (py * self.0.width + px) * 4;
        [self.0.rgba[i], self.0.rgba[i + 1], self.0.rgba[i + 2]]
    }

    fn expect(&self, x: f32, y: f32, want: [u8; 3], what: &str) {
        let got = self.at(x, y);
        let ok = (0..3).all(|k| (got[k] as i32 - want[k] as i32).abs() <= TOL);
        assert!(ok, "{what}: at ({x}, {y}) got {got:?}, want {want:?}");
    }

    /// Pixels inside a user-space rect (inset by 1) that are not white.
    fn marked(&self, r: [f32; 4]) -> usize {
        let mut n = 0;
        for y in (r[1] as usize + 1)..(r[3] as usize - 1) {
            for x in (r[0] as usize + 1)..(r[2] as usize - 1) {
                if self.at(x as f32, y as f32).iter().any(|&c| c < 240) {
                    n += 1;
                }
            }
        }
        n
    }
}

fn shot(p: &mut Pdf) -> Shot {
    shot_of(p.snapshot().unwrap())
}

/// Rendered the way the GUI does, from [`Pdf::render_snapshot`].
fn display_shot(p: &mut Pdf) -> Shot {
    shot_of(p.render_snapshot().unwrap())
}

fn shot_of(bytes: Vec<u8>) -> Shot {
    let hp = render::load(bytes, None).unwrap();
    Shot(render::Renderer::new(&hp).render(0, 1.0).unwrap())
}

fn to8(c: Rgb) -> [u8; 3] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

// ---------------------------------------------------------------------------
// Blend modes (ISO 32000-2, 11.3.5). Reference formulas, B(cb, cs).

fn lum(c: Rgb) -> f64 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: Rgb) -> Rgb {
    let l = lum(c);
    let n = c.iter().cloned().fold(f64::MAX, f64::min);
    let x = c.iter().cloned().fold(f64::MIN, f64::max);
    let mut c = c;
    if n < 0.0 {
        c = c.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 {
        c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    c
}

fn set_lum(c: Rgb, l: f64) -> Rgb {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}

fn sat(c: Rgb) -> f64 {
    c.iter().cloned().fold(f64::MIN, f64::max) - c.iter().cloned().fold(f64::MAX, f64::min)
}

fn set_sat(c: Rgb, s: f64) -> Rgb {
    let mut i = [0usize, 1, 2];
    i.sort_by(|&a, &b| c[a].partial_cmp(&c[b]).unwrap());
    let (min, mid, max) = (i[0], i[1], i[2]);
    let mut out = [0.0; 3];
    if c[max] > c[min] {
        out[mid] = (c[mid] - c[min]) * s / (c[max] - c[min]);
        out[max] = s;
    }
    out
}

fn blend(mode: &str, cb: Rgb, cs: Rgb) -> Rgb {
    let sep = |f: fn(f64, f64) -> f64| [f(cb[0], cs[0]), f(cb[1], cs[1]), f(cb[2], cs[2])];
    fn multiply(b: f64, s: f64) -> f64 {
        b * s
    }
    fn screen(b: f64, s: f64) -> f64 {
        b + s - b * s
    }
    fn hard_light(b: f64, s: f64) -> f64 {
        if s <= 0.5 {
            multiply(b, 2.0 * s)
        } else {
            screen(b, 2.0 * s - 1.0)
        }
    }
    match mode {
        "Normal" => cs,
        "Multiply" => sep(multiply),
        "Screen" => sep(screen),
        "Overlay" => sep(|b, s| hard_light(s, b)),
        "Darken" => sep(f64::min),
        "Lighten" => sep(f64::max),
        "ColorDodge" => sep(|b, s| {
            if b == 0.0 {
                0.0
            } else if b >= 1.0 - s {
                1.0
            } else {
                b / (1.0 - s)
            }
        }),
        "ColorBurn" => sep(|b, s| {
            if b == 1.0 {
                1.0
            } else if 1.0 - b >= s {
                0.0
            } else {
                1.0 - (1.0 - b) / s
            }
        }),
        "HardLight" => sep(hard_light),
        "SoftLight" => sep(|b, s| {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 {
                    ((16.0 * b - 12.0) * b + 4.0) * b
                } else {
                    b.sqrt()
                };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }),
        "Difference" => sep(|b, s| (b - s).abs()),
        "Exclusion" => sep(|b, s| b + s - 2.0 * b * s),
        "Hue" => set_lum(set_sat(cs, sat(cb)), lum(cb)),
        "Saturation" => set_lum(set_sat(cb, sat(cs)), lum(cb)),
        "Color" => set_lum(cs, lum(cb)),
        "Luminosity" => set_lum(cb, lum(cs)),
        _ => unreachable!("{mode}"),
    }
}

const BACK: Rgb = [0.6, 0.3, 0.2];
const SRC: Rgb = [0.2, 0.6, 0.9];

/// Opaque backdrop rect, then an opaque source rect over its middle painted
/// with `/BM mode` from an ExtGState. Sampled where they overlap.
fn blend_case(mode: &str) {
    let want = to8(blend(mode, BACK, SRC));
    if mode != "Normal" {
        let normal = to8(SRC);
        assert!(
            (0..3).any(|k| (want[k] as i32 - normal[k] as i32).abs() > 2 * TOL),
            "test colours cannot tell {mode} from Normal"
        );
    }
    let content = format!(
        "{} {} {} rg 50 50 200 100 re f\n/G0 gs {} {} {} rg 100 75 100 50 re f\n",
        BACK[0], BACK[1], BACK[2], SRC[0], SRC[1], SRC[2]
    );
    let res =
        dictionary! { "ExtGState" => dictionary! { "G0" => gs(dictionary! { "BM" => mode }) } };
    let s = shot(&mut make(&content, res));
    s.expect(150.0, 100.0, want, mode);
    s.expect(75.0, 100.0, to8(BACK), "backdrop untouched");
}

macro_rules! blend_tests {
    ($($(#[$m:meta])* $name:ident: $mode:literal),* $(,)?) => {
        $( $(#[$m])* #[test] fn $name() { blend_case($mode) } )*
    };
}

blend_tests! {
    blend_normal: "Normal",
    blend_multiply: "Multiply",
    blend_screen: "Screen",
    blend_overlay: "Overlay",
    blend_darken: "Darken",
    blend_lighten: "Lighten",
    blend_color_dodge: "ColorDodge",
    blend_color_burn: "ColorBurn",
    blend_hard_light: "HardLight",
    blend_soft_light: "SoftLight",
    blend_difference: "Difference",
    blend_exclusion: "Exclusion",
    blend_hue: "Hue",
    blend_saturation: "Saturation",
    blend_color: "Color",
    blend_luminosity: "Luminosity",
}

// ---------------------------------------------------------------------------
// Transparency groups (11.4.5, 11.4.6).

/// Backdrop rect, then a form XObject with a transparency group whose single
/// object is painted with Multiply.
fn group_case(isolated: bool) -> Shot {
    let content = format!(
        "{} {} {} rg 50 50 200 100 re f /Fx Do",
        BACK[0], BACK[1], BACK[2]
    );
    let mut p = make(&content, Dictionary::new());
    let inner = format!(
        "/M gs {} {} {} rg 100 75 100 50 re f",
        SRC[0], SRC[1], SRC[2]
    );
    let fx = form(
        &mut p,
        [0.0, 0.0, W, H],
        &inner,
        dictionary! {
            "Group" => dictionary! { "S" => "Transparency", "I" => isolated },
            "Resources" => dictionary! { "ExtGState" => dictionary! { "M" => gs(dictionary! { "BM" => "Multiply" }) } },
        },
    );
    set_resources(
        &mut p,
        dictionary! { "XObject" => dictionary! { "Fx" => fx } },
    );
    shot(&mut p)
}

#[test]
fn isolated_group_blends_against_transparent_backdrop() {
    // Blending with a fully transparent backdrop yields the source colour.
    group_case(true).expect(150.0, 100.0, to8(SRC), "isolated group");
}

#[test]
#[ignore = "hayro 0.8 treats every group as isolated"]
fn non_isolated_group_blends_against_page_backdrop() {
    group_case(false).expect(
        150.0,
        100.0,
        to8(blend("Multiply", BACK, SRC)),
        "non-isolated group",
    );
}

#[test]
fn blend_mode_applies_to_whole_group() {
    // /BM set outside the form applies when the group is composited.
    let content = format!(
        "{} {} {} rg 50 50 200 100 re f /M gs /Fx Do",
        BACK[0], BACK[1], BACK[2]
    );
    let mut p = make(&content, Dictionary::new());
    let inner = format!("{} {} {} rg 100 75 100 50 re f", SRC[0], SRC[1], SRC[2]);
    let fx = form(
        &mut p,
        [0.0, 0.0, W, H],
        &inner,
        dictionary! { "Group" => dictionary! { "S" => "Transparency", "I" => true } },
    );
    set_resources(
        &mut p,
        dictionary! {
            "XObject" => dictionary! { "Fx" => fx },
            "ExtGState" => dictionary! { "M" => gs(dictionary! { "BM" => "Multiply" }) },
        },
    );
    shot(&mut p).expect(
        150.0,
        100.0,
        to8(blend("Multiply", BACK, SRC)),
        "group composited with Multiply",
    );
}

/// Blue then red, both at 50% fill alpha, overlapping, inside a group on a
/// white page.
fn knockout_case(knockout: bool) -> Shot {
    let mut p = make("/Fx Do", Dictionary::new());
    let inner = "/A gs 0 0 1 rg 60 60 100 80 re f 1 0 0 rg 120 60 100 80 re f";
    let fx = form(
        &mut p,
        [0.0, 0.0, W, H],
        inner,
        dictionary! {
            "Group" => dictionary! { "S" => "Transparency", "K" => knockout },
            "Resources" => dictionary! { "ExtGState" => dictionary! { "A" => gs(dictionary! { "ca" => 0.5 }) } },
        },
    );
    set_resources(
        &mut p,
        dictionary! { "XObject" => dictionary! { "Fx" => fx } },
    );
    let s = shot(&mut p);
    s.expect(90.0, 100.0, [128, 128, 255], "blue alone");
    s.expect(190.0, 100.0, [255, 128, 128], "red alone");
    s
}

#[test]
fn non_knockout_group_composites_objects_over_each_other() {
    // red 50% over (blue 50% over white) = (0.75, 0.25, 0.5)
    knockout_case(false).expect(140.0, 100.0, [191, 64, 128], "overlap");
}

#[test]
#[ignore = "hayro 0.8 ignores /K (knockout)"]
fn knockout_group_replaces_earlier_objects() {
    // In a knockout group the red rect composites with the group's initial
    // backdrop (white), not with the blue rect: red 50% over white.
    knockout_case(true).expect(140.0, 100.0, [255, 128, 128], "overlap");
}

// ---------------------------------------------------------------------------
// Colour-key masking (8.9.6.4).

#[test]
fn color_key_mask_hides_matching_pixels() {
    // 2x1 RGB image: red, blue. /Mask [..] hides red. Drawn over green.
    let mut p = make(
        "0 1 0 rg 50 50 200 100 re f q 100 0 0 50 100 75 cm /Im Do Q",
        Dictionary::new(),
    );
    let im = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 2,
            "Height" => 1,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Mask" => vec![250.into(), 255.into(), 0.into(), 5.into(), 0.into(), 5.into()],
        },
        vec![255, 0, 0, 0, 0, 255],
    ));
    set_resources(
        &mut p,
        dictionary! { "XObject" => dictionary! { "Im" => im } },
    );
    let s = shot(&mut p);
    s.expect(125.0, 100.0, [0, 255, 0], "masked red pixel shows backdrop");
    s.expect(175.0, 100.0, [0, 0, 255], "unmasked blue pixel");
}

// ---------------------------------------------------------------------------
// Annotations (12.5).

const RECT: [f32; 4] = [100.0, 60.0, 200.0, 140.0];
const MAGENTA: [u8; 3] = [255, 0, 255];

/// An appearance stream that fills its whole 10x10 BBox with `rgb`. The BBox
/// is smaller than RECT, so this also checks the BBox-to-Rect mapping.
fn ap_stream(p: &mut Pdf, rgb: &str) -> ObjectId {
    form(
        p,
        [0.0, 0.0, 10.0, 10.0],
        &format!("{rgb} rg 0 0 10 10 re f"),
        Dictionary::new(),
    )
}

/// One annotation on a blank page.
fn annotated(subtype: &str, extra: Dictionary, ap: bool) -> Pdf {
    let mut p = make("", Dictionary::new());
    let mut a = dictionary! {
        "Type" => "Annot",
        "Subtype" => subtype,
        "Rect" => nums(&RECT),
        "F" => 4, // Print
    };
    merge(&mut a, extra);
    if ap {
        let n = ap_stream(&mut p, "1 0 1");
        a.set("AP", dictionary! { "N" => n });
    }
    let id = p.doc.add_object(a);
    let page = p.page_ids()[0];
    p.doc
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", vec![Object::Reference(id)]);
    p
}

fn text_extra() -> Dictionary {
    dictionary! { "Contents" => Object::string_literal("note"), "C" => nums(&[1.0, 1.0, 0.0]) }
}
fn highlight_extra() -> Dictionary {
    dictionary! {
        "C" => nums(&[1.0, 1.0, 0.0]),
        "QuadPoints" => nums(&[100.0, 140.0, 200.0, 140.0, 100.0, 60.0, 200.0, 60.0]),
    }
}
fn link_extra() -> Dictionary {
    dictionary! {
        "Border" => nums(&[0.0, 0.0, 2.0]),
        "C" => nums(&[1.0, 0.0, 0.0]),
        "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com") },
    }
}
fn stamp_extra() -> Dictionary {
    dictionary! { "Name" => "Draft" }
}
fn free_text_extra() -> Dictionary {
    dictionary! {
        "Contents" => Object::string_literal("LimarePDF"),
        "DA" => Object::string_literal("0 0 1 rg /Helv 12 Tf"),
    }
}
fn widget_extra() -> Dictionary {
    dictionary! {
        "FT" => "Tx",
        "T" => Object::string_literal("f1"),
        "V" => Object::string_literal("LimarePDF"),
        "DA" => Object::string_literal("0 0 1 rg /Helv 12 Tf"),
        "MK" => dictionary! { "BG" => nums(&[0.0, 1.0, 0.0]) },
    }
}

/// Adds an /AcroForm that lists the page's widget, as a real form would.
fn with_acroform(mut p: Pdf) -> Pdf {
    let page = p.page_ids()[0];
    let annots = p
        .doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .clone();
    let helv = p.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let root = p.doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    p.doc.get_dictionary_mut(root).unwrap().set(
        "AcroForm",
        dictionary! {
            "Fields" => annots,
            "NeedAppearances" => true,
            "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
            "DA" => Object::string_literal("0 0 1 rg /Helv 12 Tf"),
        },
    );
    p
}

fn mid() -> (f32, f32) {
    ((RECT[0] + RECT[2]) / 2.0, (RECT[1] + RECT[3]) / 2.0)
}

macro_rules! ap_tests {
    ($($name:ident: $sub:literal, $extra:expr),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                let s = shot(&mut annotated($sub, $extra, true));
                let (x, y) = mid();
                s.expect(x, y, MAGENTA, concat!($sub, " /AP centre"));
                s.expect(RECT[0] + 2.0, RECT[1] + 2.0, MAGENTA, concat!($sub, " /AP corner"));
                s.expect(RECT[0] - 3.0, y, [255, 255, 255], concat!($sub, " outside Rect"));
            }
        )*
    };
}

ap_tests! {
    annot_text_with_ap: "Text", text_extra(),
    annot_highlight_with_ap: "Highlight", highlight_extra(),
    annot_link_with_ap: "Link", link_extra(),
    annot_stamp_with_ap: "Stamp", stamp_extra(),
    annot_free_text_with_ap: "FreeText", free_text_extra(),
    annot_widget_with_ap: "Widget", widget_extra(),
}

#[test]
fn annot_widget_with_ap_in_form() {
    let s = shot(&mut with_acroform(annotated(
        "Widget",
        widget_extra(),
        true,
    )));
    let (x, y) = mid();
    s.expect(x, y, MAGENTA, "Widget /AP in AcroForm");
}

#[test]
fn annot_widget_picks_appearance_state() {
    for (state, want) in [("Yes", MAGENTA), ("Off", [0, 255, 255])] {
        let mut p = annotated(
            "Widget",
            dictionary! { "FT" => "Btn", "T" => Object::string_literal("cb"), "AS" => state },
            false,
        );
        let on = ap_stream(&mut p, "1 0 1");
        let off = ap_stream(&mut p, "0 1 1");
        let page = p.page_ids()[0];
        let id = p
            .doc
            .get_dictionary(page)
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_reference()
            .unwrap();
        p.doc.get_dictionary_mut(id).unwrap().set(
            "AP",
            dictionary! { "N" => dictionary! { "Yes" => on, "Off" => off } },
        );
        let (x, y) = mid();
        shot(&mut p).expect(x, y, want, &format!("/AS /{state}"));
    }
}

#[test]
fn annot_hidden_flag_is_respected() {
    let s = shot(&mut annotated("Stamp", dictionary! { "F" => 2 }, true));
    assert_eq!(s.marked(RECT), 0, "Hidden annotation was drawn");
}

#[test]
#[ignore = "hayro 0.8 only checks the Hidden flag"]
fn annot_noview_flag_is_respected() {
    // NoView (bit 6): not displayed on screen, may be printed.
    let s = shot(&mut annotated("Stamp", dictionary! { "F" => 32 | 4 }, true));
    assert_eq!(s.marked(RECT), 0, "NoView annotation was drawn on screen");
}

#[test]
fn render_snapshot_hides_noview_annotations() {
    let mut p = annotated("Stamp", dictionary! { "F" => 32 | 4 }, true);
    let s = display_shot(&mut p);
    assert_eq!(s.marked(RECT), 0, "NoView annotation was drawn on screen");
    let page = p.page_ids()[0];
    let id = p
        .doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .as_reference()
        .unwrap();
    let flags = p
        .doc
        .get_dictionary(id)
        .unwrap()
        .get(b"F")
        .unwrap()
        .as_i64()
        .unwrap();
    assert_eq!(flags, 32 | 4, "document flags were changed");
}

#[test]
fn render_snapshot_keeps_other_annotations() {
    let s = display_shot(&mut annotated("Stamp", stamp_extra(), true));
    let (x, y) = mid();
    s.expect(x, y, MAGENTA, "Print-flagged annotation");
}

// Without /AP the viewer is expected to generate an appearance from the
// annotation's own entries (12.5.6). Each case checks for that appearance.

#[test]
#[ignore = "hayro 0.8 draws annotations only from /AP"]
fn annot_text_without_ap() {
    let s = shot(&mut annotated("Text", text_extra(), false));
    assert!(s.marked(RECT) > 0, "no icon drawn for Text annotation");
}

#[test]
#[ignore = "hayro 0.8 draws annotations only from /AP"]
fn annot_highlight_without_ap() {
    let s = shot(&mut annotated("Highlight", highlight_extra(), false));
    let (x, y) = mid();
    s.expect(x, y, [255, 255, 0], "QuadPoints filled with /C");
}

#[test]
#[ignore = "hayro 0.8 draws annotations only from /AP"]
fn annot_link_without_ap() {
    let s = shot(&mut annotated("Link", link_extra(), false));
    let (_, y) = mid();
    s.expect(RECT[0] + 1.0, y, [255, 0, 0], "2pt /Border in /C");
    let (x, _) = mid();
    s.expect(x, y, [255, 255, 255], "Link interior stays clear");
}

#[test]
fn annot_link_without_ap_or_border_draws_nothing() {
    let mut extra = link_extra();
    extra.set("Border", nums(&[0.0, 0.0, 0.0]));
    let s = shot(&mut annotated("Link", extra, false));
    assert_eq!(
        s.marked([RECT[0] - 2.0, RECT[1] - 2.0, RECT[2] + 2.0, RECT[3] + 2.0]),
        0
    );
}

#[test]
#[ignore = "hayro 0.8 draws annotations only from /AP"]
fn annot_stamp_without_ap() {
    let s = shot(&mut annotated("Stamp", stamp_extra(), false));
    assert!(s.marked(RECT) > 0, "no stamp drawn");
}

#[test]
#[ignore = "hayro 0.8 draws annotations only from /AP"]
fn annot_free_text_without_ap() {
    let s = shot(&mut annotated("FreeText", free_text_extra(), false));
    assert!(s.marked(RECT) > 0, "FreeText /Contents not drawn");
}

#[test]
#[ignore = "hayro 0.8 draws annotations only from /AP"]
fn annot_widget_without_ap() {
    let s = shot(&mut with_acroform(annotated(
        "Widget",
        widget_extra(),
        false,
    )));
    assert!(s.marked(RECT) > 0, "text field value/background not drawn");
}

// ---------------------------------------------------------------------------
// Encrypted input. LimarePDF normally renders from an unencrypted snapshot, but
// `render::load` takes a password, so check hayro can decrypt directly.

#[test]
fn renders_encrypted_files_with_password() {
    for cipher in [
        Cipher::Rc4_40,
        Cipher::Rc4_128,
        Cipher::Aes128,
        Cipher::Aes256,
    ] {
        let mut p = make("1 0 0 rg 50 50 200 100 re f", Dictionary::new());
        p.set_encryption(Some(Encryption {
            user_password: "user".into(),
            owner_password: "owner".into(),
            cipher,
            restrictions: Restrictions::default(),
        }));
        let bytes = p.to_bytes().unwrap();
        let hp = render::load(bytes, Some("user")).unwrap_or_else(|e| panic!("{cipher:?}: {e}"));
        let s = Shot(render::Renderer::new(&hp).render(0, 1.0).unwrap());
        s.expect(150.0, 100.0, [255, 0, 0], &format!("{cipher:?}"));
    }
}

#[test]
fn wrong_password_is_explained() {
    let mut p = make("", Dictionary::new());
    p.set_encryption(Some(Encryption {
        user_password: "user".into(),
        owner_password: "owner".into(),
        cipher: Cipher::Aes128,
        restrictions: Restrictions::default(),
    }));
    let Err(e) = render::load(p.to_bytes().unwrap(), Some("wrong")) else {
        panic!("loaded with the wrong password");
    };
    assert_eq!(e.to_string(), "cannot render PDF: it needs a password");
    let Err(e) = render::load(b"not a PDF".to_vec(), None) else {
        panic!("loaded a file that isn't a PDF");
    };
    assert_eq!(
        e.to_string(),
        "cannot render PDF: the file's structure can't be read"
    );
}

// ---------------------------------------------------------------------------
// What the renderer reports it couldn't draw (the app's banner).

use render::{InstalledFonts, Language, Missing};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

fn missing_of(p: &mut Pdf) -> BTreeSet<Missing> {
    drawn_with(p, Arc::new(InstalledFonts::none())).1
}

/// The page drawn when only `fonts` are installed, and what was missing.
fn drawn_with(p: &mut Pdf, fonts: Arc<InstalledFonts>) -> (render::Bitmap, BTreeSet<Missing>) {
    let hp = render::load(p.snapshot().unwrap(), None).unwrap();
    let r = render::Renderer::with_fonts(&hp, fonts);
    let b = r.render(0, 1.0).unwrap();
    let found = r.take_missing();
    assert!(r.take_missing().is_empty(), "taking them clears them");
    (b, found)
}

#[test]
fn ordinary_pages_report_nothing_missing() {
    // A font that isn't embedded and isn't standard gets a standard stand-in, which is
    // close enough: not reported.
    let mut p = make(
        "0 0 1 rg 50 50 100 50 re f BT /F1 24 Tf 60 120 Td (Hello) Tj ET",
        dictionary! { "Font" => dictionary! { "F1" => dictionary! {
            "Type" => "Font", "Subtype" => "TrueType", "BaseFont" => "Arial",
        } } },
    );
    assert!(missing_of(&mut p).is_empty());
}

#[test]
fn undecodable_pictures_are_reported() {
    let mut p = make("q 100 0 0 50 100 75 cm /Im Do Q", Dictionary::new());
    let im = p.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 4,
            "Height" => 4,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        },
        b"this is not a JPEG".to_vec(),
    ));
    set_resources(
        &mut p,
        dictionary! { "XObject" => dictionary! { "Im" => im } },
    );
    assert_eq!(missing_of(&mut p), [Missing::Picture].into());
}

#[test]
fn cjk_fonts_that_are_not_embedded_are_reported() {
    let mut p = make("BT /F1 24 Tf 60 120 Td <0001> Tj ET", Dictionary::new());
    let cid = p.doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType0",
        "BaseFont" => "KozMinPro-Regular",
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Japan1"),
            "Supplement" => 4,
        },
        "FontDescriptor" => dictionary! {
            "Type" => "FontDescriptor",
            "FontName" => "KozMinPro-Regular",
            "Flags" => 4,
            "FontBBox" => vec![0.into(), 0.into(), 1000.into(), 1000.into()],
            "ItalicAngle" => 0,
            "Ascent" => 880,
            "Descent" => -120,
            "CapHeight" => 700,
            "StemV" => 80,
        },
    });
    set_resources(
        &mut p,
        dictionary! { "Font" => dictionary! { "F1" => dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => "KozMinPro-Regular",
            "Encoding" => "UniJIS-UCS2-H",
            "DescendantFonts" => vec![Object::Reference(cid)],
        } } },
    );
    // With no installed font for Japanese: drawn with a Latin standard font, which lacks
    // its characters, and reported by language, to say what to install.
    let japanese = Missing::Language(Language::Japanese, Some("KozMinPro-Regular".into()));
    assert_eq!(missing_of(&mut p), [japanese.clone()].into());
    let (_, missing) = drawn_with(&mut p, Arc::new(bundled()));
    // A Latin font isn't used for Japanese.
    assert_eq!(missing, [japanese].into());
}

/// Zeyada, one of the handwriting fonts bundled for typed signatures, as the only
/// installed font.
fn bundled() -> InstalledFonts {
    let zeyada = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/Zeyada.ttf");
    InstalledFonts::from_files(&[&zeyada])
}

/// A page showing a "W" in a Latin CID font named `name` that isn't embedded, with a
/// ToUnicode map from code 1 to "W", as some programs write plain text.
fn latin_cid_font(name: &str) -> Pdf {
    let mut p = make("BT /F1 96 Tf 40 50 Td <0001> Tj ET", Dictionary::new());
    let cid = p.doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType2",
        "BaseFont" => name,
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0,
        },
        "DW" => 1000,
    });
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
        /CMapName /Test def 1 begincodespacerange <0000> <FFFF> endcodespacerange \
        1 beginbfchar <0001> <0057> endbfchar endcmap \
        CMapName currentdict /CMap defineresource pop end end";
    let to_unicode = p
        .doc
        .add_object(Stream::new(Dictionary::new(), cmap.as_bytes().to_vec()));
    set_resources(
        &mut p,
        dictionary! { "Font" => dictionary! { "F1" => dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => name,
            "Encoding" => "Identity-H",
            "DescendantFonts" => vec![Object::Reference(cid)],
            "ToUnicode" => to_unicode,
        } } },
    );
    p
}

#[test]
fn cid_fonts_that_are_not_embedded_use_installed_fonts() {
    let mut p = latin_cid_font("Zeyada");
    let (with, missing) = drawn_with(&mut p, Arc::new(bundled()));
    assert!(missing.is_empty(), "{missing:?}");
    let (without, missing) = drawn_with(&mut p, Arc::new(InstalledFonts::none()));
    assert_eq!(missing, [Missing::Font(Some("Zeyada".into()))].into());
    let ink = |b: &render::Bitmap| b.rgba.chunks(4).filter(|px| px[0] < 128).count();
    assert!(ink(&with) > 50, "the installed font draws the W");
    assert_ne!(
        with.rgba, without.rgba,
        "and it isn't the standard stand-in"
    );
}

#[test]
fn fonts_that_are_not_installed_get_a_stand_in_and_are_named() {
    // Tahoma isn't installed: an installed font stands in, drawing the W rather than a
    // wrong character, and the notice names both.
    let mut p = latin_cid_font("Tahoma,Bold");
    let (stand_in, missing) = drawn_with(&mut p, Arc::new(bundled()));
    assert_eq!(
        missing,
        [Missing::StandIn(Some("Tahoma".into()), "Zeyada".into())].into()
    );
    assert!(!missing.iter().any(Missing::lost));
    let mut q = latin_cid_font("Zeyada");
    let (zeyada, _) = drawn_with(&mut q, Arc::new(bundled()));
    assert_eq!(stand_in.rgba, zeyada.rgba, "drawn with the stand-in");
}
