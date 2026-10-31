//! What LimarePDF's pages draw a font in that the PDF names but doesn't hold (ENH-25):
//! [`LineFont::stand_in`] checked against the renderer's own choice.

use limarepdf_core::render::{load, Renderer};
use limarepdf_core::{text_width, Family, FontChoice, LineFont, Pdf, StandIn};
use lopdf::{dictionary, Dictionary, Object, Stream};

/// A 240×40 page with "Walrus Quiz" in a font `subtype` named `base`, not embedded, with
/// Helvetica's widths and the descriptor `desc` (none with `None`).
fn page_in(subtype: &str, base: &str, desc: Option<Dictionary>) -> Pdf {
    let mut pdf = Pdf::new();
    pdf.insert_blank(0, 240.0, 40.0).unwrap();
    let widths: Vec<Object> = (32u8..=126)
        .map(|b| Object::Real(text_width(&(b as char).to_string(), 1000.0)))
        .collect();
    let mut font = dictionary! {
        "Type" => "Font", "Subtype" => subtype, "BaseFont" => base,
        "FirstChar" => 32, "LastChar" => 126, "Widths" => widths,
        "Encoding" => "WinAnsiEncoding",
    };
    if let Some(entries) = desc {
        let mut d = dictionary! { "Type" => "FontDescriptor", "FontName" => base };
        d.extend(&entries);
        font.set("FontDescriptor", pdf.doc.add_object(d));
    }
    let font = pdf.doc.add_object(font);
    let content = b"BT /F1 24 Tf 10 12 Td (Walrus Quiz) Tj ET".to_vec();
    let s = pdf.doc.add_object(Stream::new(Dictionary::new(), content));
    let page = pdf.page_ids()[0];
    let d = pdf.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", Object::Reference(s));
    d.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font } },
    );
    pdf
}

/// The font of the page's line.
fn line_font(pdf: &Pdf) -> LineFont {
    let objects = pdf.page_objects(0).unwrap();
    let line = objects.iter().find(|o| o.text.is_some()).unwrap();
    pdf.line_fonts(0, line.id).unwrap().remove(0)
}

/// The page as LimarePDF draws it, at 3 pixels a point.
fn drawn(pdf: &mut Pdf) -> Vec<u8> {
    let h = load(pdf.snapshot().unwrap(), None).unwrap();
    Renderer::new(&h).render(0, 3.0).unwrap().rgba
}

fn choice(family: Family, bold: bool, italic: bool) -> FontChoice {
    FontChoice {
        family,
        bold,
        italic,
    }
}

/// Each font is drawn as the standard font its stand-in names would be: the same pixels.
#[test]
fn a_font_the_pdf_doesn_t_hold_is_drawn_in_the_standard_font_its_stand_in_names() {
    use Family::{Courier, Helvetica, Times};
    let cases = [
        // By the name: the standard font it is, or a word in it.
        ("Type1", "Helvetica", None, choice(Helvetica, false, false)),
        (
            "TrueType",
            "Arial",
            Some(dictionary! { "Flags" => 32 }),
            choice(Helvetica, false, false),
        ),
        (
            "TrueType",
            "Arial",
            Some(dictionary! { "Flags" => 32, "FontWeight" => 700 }),
            choice(Helvetica, true, false),
        ),
        (
            "TrueType",
            "ArialMT",
            Some(dictionary! { "Flags" => 32, "ItalicAngle" => -12 }),
            choice(Helvetica, false, true),
        ),
        (
            "TrueType",
            "CourierNewPS-BoldMT",
            Some(dictionary! { "Flags" => 35 }),
            choice(Courier, true, false),
        ),
        (
            "TrueType",
            "DejaVuSansMono",
            Some(dictionary! { "Flags" => 33 }),
            choice(Helvetica, false, false),
        ),
        (
            "Type1",
            "ABCDEF+TimesNewRomanPS-ItalicMT",
            Some(dictionary! { "Flags" => 34 }),
            choice(Times, false, true),
        ),
        // No such word: by the serif flag, and "Bold" and "Italic" in the name.
        (
            "TrueType",
            "Cambria",
            Some(dictionary! { "Flags" => 34 }),
            choice(Times, false, false),
        ),
        (
            "TrueType",
            "Calibri",
            Some(dictionary! { "Flags" => 32, "FontWeight" => 700 }),
            choice(Helvetica, false, false),
        ),
        (
            "TrueType",
            "Georgia,BoldItalic",
            Some(dictionary! { "Flags" => 34 }),
            choice(Times, true, true),
        ),
        (
            "TrueType",
            "Georgia",
            Some(dictionary! { "Flags" => 98 }),
            choice(Times, false, true),
        ),
        (
            "TrueType",
            "Consolas",
            Some(dictionary! { "Flags" => 33 }),
            choice(Helvetica, false, false),
        ),
        ("TrueType", "Calibri", None, choice(Helvetica, false, false)),
    ];
    let mut faces = Vec::new();
    for (subtype, base, desc, want) in cases {
        let mut pdf = page_in(subtype, base, desc);
        assert_eq!(
            line_font(&pdf).stand_in(),
            Some(&StandIn::Standard(want)),
            "{base}"
        );
        // Against a font the renderer can only take one way: the standard font itself where
        // the case's name names it (drawn in its own widths), else one named by the
        // family's word and the style's (stretched to the PDF's widths, as the case is).
        let lower = base.to_ascii_lowercase();
        let exact = ["helvetica", "courier", "times"]
            .iter()
            .any(|w| lower.contains(w));
        let reference = match exact {
            true => want.base_font().to_string(),
            false => {
                let family = match want.family {
                    Family::Times => "Serif",
                    Family::Courier => "Mono",
                    _ => "Sans",
                };
                let bold = if want.bold { "Bold" } else { "" };
                let italic = if want.italic { "Italic" } else { "" };
                format!("{family}{bold}{italic}")
            }
        };
        let face = drawn(&mut page_in("TrueType", &reference, None));
        assert!(drawn(&mut pdf) == face, "{base} isn't drawn as {reference}");
        faces.push((want, face));
    }
    // The pixels tell the faces apart: bold from regular, serif from sans.
    let (sans, bold) = (&faces[1], &faces[2]);
    assert!(sans.0 != bold.0 && sans.1 != bold.1);
    let (cambria, calibri) = (&faces[7], &faces[8]);
    assert!(cambria.0 != calibri.0 && cambria.1 != calibri.1);
}

/// A font the PDF holds, and Symbol, have none; a CID font without a family word in its
/// name asks for an installed font, by its names, the style its name gives and its
/// character collection (its CMap's, else its `/CIDSystemInfo`).
#[test]
fn a_cid_font_the_pdf_doesn_t_hold_asks_for_an_installed_font() {
    let mut held = page_in(
        "TrueType",
        "LiberationSans",
        Some(dictionary! { "Flags" => 32 }),
    );
    let program = limarepdf_core::bundled_face("Liberation Sans")
        .unwrap()
        .to_vec();
    let file = held.doc.add_object(Stream::new(Dictionary::new(), program));
    for o in held.doc.objects.values_mut() {
        if let Object::Dictionary(d) = o {
            if d.has_type(b"FontDescriptor") {
                d.set("FontFile2", file);
            }
        }
    }
    assert_eq!(line_font(&held).stand_in(), None);
    let symbol = page_in("Type1", "Symbol", None);
    assert_eq!(line_font(&symbol).stand_in(), None);

    let cid = |base: &str, encoding: &str| {
        let mut pdf = page_in("TrueType", "Placeholder", None);
        let desc = pdf.doc.add_object(dictionary! {
            "Type" => "FontDescriptor", "FontName" => base, "Flags" => 4,
        });
        let info = dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Japan1"), "Supplement" => 4,
        };
        let descendant = pdf.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => base,
            "CIDSystemInfo" => info, "FontDescriptor" => desc, "DW" => 1000,
        });
        let type0 = dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => base,
            "Encoding" => encoding, "DescendantFonts" => vec![descendant.into()],
        };
        let font = pdf.doc.add_object(type0);
        let page = pdf.page_ids()[0];
        let d = pdf.doc.get_dictionary_mut(page).unwrap();
        d.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
        let content = b"BT /F1 24 Tf 10 12 Td <00410042> Tj ET".to_vec();
        let s = pdf.doc.add_object(Stream::new(Dictionary::new(), content));
        d_contents(&mut pdf, s);
        line_font(&pdf).stand_in().cloned()
    };
    let Some(StandIn::Installed(q, standard)) = cid("ABCDEF+MSMincho-Bold", "Identity-H") else {
        panic!("no question for an installed font");
    };
    assert_eq!(standard, choice(Family::Helvetica, true, false));
    let key = q.key();
    for part in ["\"MSMincho-Bold\"", "AdobeJapan1", "true false"] {
        assert!(key.contains(part), "{key}");
    }
    // A predefined CMap's own collection.
    let Some(StandIn::Installed(q, _)) = cid("KozMinPr6N-Regular", "UniGB-UCS2-H") else {
        panic!("no question for an installed font");
    };
    assert!(q.key().contains("AdobeGB1"), "{}", q.key());
    // A family word in its name: the standard font, as for a simple font.
    assert_eq!(
        cid("ArialUnicodeMS", "Identity-H"),
        Some(StandIn::Standard(choice(Family::Helvetica, false, false)))
    );
}

fn d_contents(pdf: &mut Pdf, s: lopdf::ObjectId) {
    let page = pdf.page_ids()[0];
    let d = pdf.doc.get_dictionary_mut(page).unwrap();
    d.set("Contents", Object::Reference(s));
}
