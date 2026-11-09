//! Finding blank pages: pages that are drawn as one even colour, give or take a few specks.

use crate::render::{self, Bitmap, InstalledFonts, Missing, Renderer};
use crate::{Pdf, Result};
use anyhow::bail;
use rayon::prelude::*;
use std::sync::Arc;

/// The share of a page, in percent, that may differ from its background for the page to count as blank: enough
/// for scanner specks or a lone page number, not for a line of text.
pub const BLANK_TOLERANCE: f32 = 0.05;

/// How far a pixel's grey level (0 to 255) must be from the background's to count as a mark. Faint show-through
/// from the other side of a scanned sheet stays below it.
const MARK: i32 = 48;

/// Pages are drawn at 72 dpi, or smaller when their longer side would pass this many pixels.
const MAX_SIDE: f32 = 1200.0;

impl Pdf {
    /// The pages (0-based, in order) that look blank: at most `tolerance` percent of their pixels differ from the
    /// page's background colour, as drawn with its annotations. A page that can't be drawn is not blank, nor is one
    /// with something lost in drawing it ([`Missing::lost`]: a picture that can't be decoded, or text in a font no
    /// installed font can draw); text drawn with an installed font like the one asked for counts as drawn.
    pub fn blank_pages(&mut self, tolerance: f32) -> Result<Vec<usize>> {
        self.blank_pages_with(tolerance, None)
    }

    /// [`Pdf::blank_pages`], looking only in `fonts`, when given, for fonts that aren't embedded.
    fn blank_pages_with(
        &mut self,
        tolerance: f32,
        fonts: Option<Arc<InstalledFonts>>,
    ) -> Result<Vec<usize>> {
        let hp = render::load(self.render_snapshot()?, None)?;
        let renderer = || match &fonts {
            Some(fonts) => Renderer::with_fonts(&hp, fonts.clone()),
            None => Renderer::new(&hp),
        };
        let blank: Vec<bool> = (0..self.page_count())
            .into_par_iter()
            .map_init(
                || Checker::new(renderer()),
                |c, i| c.blank(i, tolerance, renderer),
            )
            .collect();
        Ok((0..blank.len()).filter(|&i| blank[i]).collect())
    }

    /// Delete the pages [`Pdf::blank_pages`] finds, and return their indices as they were before deleting.
    pub fn remove_blank_pages(&mut self, tolerance: f32) -> Result<Vec<usize>> {
        let blank = self.blank_pages(tolerance)?;
        if !blank.is_empty() && blank.len() == self.page_count() {
            bail!("every page looks blank, so none were removed");
        }
        if !blank.is_empty() {
            self.delete_pages(&blank)?;
        }
        Ok(blank)
    }
}

/// One thread's renderer, kept from page to page for the fonts it has read.
struct Checker<'a> {
    r: Renderer<'a>,
    /// Whether `r` has reported something lost on a page.
    lost: bool,
}

impl<'a> Checker<'a> {
    fn new(r: Renderer<'a>) -> Self {
        Self { r, lost: false }
    }

    /// Whether page `i` is blank: it looks blank, and nothing on it was lost in drawing it. `renderer` makes a new
    /// renderer.
    fn blank(&mut self, i: usize, tolerance: f32, renderer: impl Fn() -> Renderer<'a>) -> bool {
        let Ok((w, h)) = self.r.page_size(i) else {
            return false;
        };
        let scale = (MAX_SIDE / w.max(h).max(1.0)).min(1.0);
        let (looks, lost) = drawn(&self.r, i, scale, tolerance);
        let lost_before = self.lost;
        self.lost |= lost;
        // A renderer reports a font once, on the first page it reads it for, so once one has been lost a page is
        // drawn again with a new renderer, which reads every font afresh.
        looks && !lost && (!lost_before || drawn(&renderer(), i, scale, tolerance) == (true, false))
    }
}

/// Whether page `i`, drawn by `r` at `scale`, looks blank, and whether `r` reported something lost in drawing it.
fn drawn(r: &Renderer, i: usize, scale: f32, tolerance: f32) -> (bool, bool) {
    r.take_missing();
    let looks = r
        .render(i, scale)
        .is_ok_and(|b| marked_share(&b) * 100.0 <= tolerance);
    (looks, r.take_missing().iter().any(Missing::lost))
}

/// The share of pixels that stand out from the background, the commonest grey level.
fn marked_share(b: &Bitmap) -> f32 {
    let mut hist = [0usize; 256];
    for p in b.rgba.as_chunks::<4>().0 {
        let l = (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000;
        hist[l as usize] += 1;
    }
    let total: usize = hist.iter().sum();
    if total == 0 {
        return 0.0;
    }
    let bg = (0..256).max_by_key(|&l| hist[l]).unwrap_or(255) as i32;
    let marks: usize = (0..256)
        .filter(|&l| (l as i32 - bg).abs() > MARK)
        .map(|l| hist[l])
        .sum();
    marks as f32 / total as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Language;
    use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};
    use std::collections::BTreeSet;
    use std::path::Path;

    /// A few characters at 10 points: under the tolerance whether drawn or not, as a lone page number is.
    const TEXT: &str = "BT /F1 10 Tf 300 40 Td <000100010001> Tj ET";

    const PICTURE: &str = "q 200 0 0 200 100 300 cm /Im Do Q";

    /// `n` empty Letter pages.
    fn pages(n: usize) -> Pdf {
        let mut p = Pdf::new();
        for i in 0..n {
            p.insert_blank(i, 612.0, 792.0).unwrap();
        }
        p
    }

    /// Gives page `page` the content `content` and the resources `resources`.
    fn draw(p: &mut Pdf, page: usize, content: &str, resources: Dictionary) {
        let s = p
            .doc
            .add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
        let id = p.page_ids()[page];
        let d = p.doc.get_dictionary_mut(id).unwrap();
        d.set("Contents", s);
        d.set("Resources", resources);
    }

    /// A picture that can't be decoded, as `/Im`.
    fn broken_picture(p: &mut Pdf) -> Dictionary {
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
        dictionary! { "XObject" => dictionary! { "Im" => im } }
    }

    /// `font` as `/F1`.
    fn font(font: ObjectId) -> Dictionary {
        dictionary! { "Font" => dictionary! { "F1" => font } }
    }

    /// A Japanese CID font that isn't embedded.
    fn japanese(p: &mut Pdf) -> ObjectId {
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
        p.doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => "KozMinPro-Regular",
            "Encoding" => "UniJIS-UCS2-H",
            "DescendantFonts" => vec![Object::Reference(cid)],
        })
    }

    /// A Latin CID font named `name` that isn't embedded, with a ToUnicode map from code 1 to "W".
    fn latin(p: &mut Pdf, name: &str) -> ObjectId {
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
        p.doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => name,
            "Encoding" => "Identity-H",
            "DescendantFonts" => vec![Object::Reference(cid)],
            "ToUnicode" => to_unicode,
        })
    }

    fn no_fonts() -> Arc<InstalledFonts> {
        Arc::new(InstalledFonts::none())
    }

    /// Zeyada, one of the handwriting fonts bundled for typed signatures, as the only installed font.
    fn zeyada() -> Arc<InstalledFonts> {
        let zeyada = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/Zeyada.ttf");
        Arc::new(InstalledFonts::from_files(&[&zeyada]))
    }

    fn kozmin() -> Missing {
        Missing::Language(Language::Japanese, Some("KozMinPro-Regular".into()))
    }

    /// What one renderer reports for each page, drawing them in turn.
    fn reports(p: &mut Pdf, fonts: Arc<InstalledFonts>) -> Vec<BTreeSet<Missing>> {
        let hp = render::load(p.render_snapshot().unwrap(), None).unwrap();
        let r = Renderer::with_fonts(&hp, fonts);
        (0..r.page_count())
            .map(|i| {
                r.render(i, 1.0).unwrap();
                r.take_missing()
            })
            .collect()
    }

    #[test]
    fn one_renderer_reports_a_broken_picture_on_each_page() {
        // hayro decodes a picture each time it's drawn, so a second page showing it reports it again.
        let mut p = pages(2);
        let im = broken_picture(&mut p);
        draw(&mut p, 0, PICTURE, im.clone());
        draw(&mut p, 1, PICTURE, im);
        let picture = BTreeSet::from([Missing::Picture]);
        assert_eq!(reports(&mut p, no_fonts()), [picture.clone(), picture]);
    }

    #[test]
    fn text_in_a_font_that_cant_be_drawn_is_not_blank() {
        // Pages: 0 Japanese text with no font for Japanese, 1 text in a Latin CID font that isn't embedded or
        // installed, 2 empty.
        let mut p = pages(3);
        let f = japanese(&mut p);
        draw(&mut p, 0, TEXT, font(f));
        let f = latin(&mut p, "Zeyada");
        draw(&mut p, 1, TEXT, font(f));
        assert_eq!(
            reports(&mut p, no_fonts()),
            [
                BTreeSet::from([kozmin()]),
                BTreeSet::from([Missing::Font(Some("Zeyada".into()))]),
                BTreeSet::new(),
            ]
        );
        assert_eq!(
            p.blank_pages_with(BLANK_TOLERANCE, Some(no_fonts()))
                .unwrap(),
            [2]
        );
    }

    #[test]
    fn a_font_read_on_an_earlier_page_is_checked_again() {
        // Page 0 shows Japanese text and a black square, page 1 the same font's text alone. One renderer reports
        // the font on page 0 only, as it keeps the fonts it has read.
        let mut p = pages(2);
        let f = japanese(&mut p);
        draw(
            &mut p,
            0,
            &format!("0 g 100 300 200 200 re f {TEXT}"),
            font(f),
        );
        draw(&mut p, 1, TEXT, font(f));
        assert_eq!(
            reports(&mut p, no_fonts()),
            [BTreeSet::from([kozmin()]), BTreeSet::new()]
        );
        let hp = render::load(p.render_snapshot().unwrap(), None).unwrap();
        let fonts = no_fonts();
        let renderer = || Renderer::with_fonts(&hp, fonts.clone());
        let mut c = Checker::new(renderer());
        assert!(!c.blank(0, BLANK_TOLERANCE, renderer));
        assert!(
            !c.blank(1, BLANK_TOLERANCE, renderer),
            "drawn again with a new renderer, which reports the font"
        );
    }

    #[test]
    fn text_drawn_with_a_stand_in_font_can_be_blank() {
        // D-21: Tahoma isn't installed, and Zeyada stands in for it, drawing the text. A few characters are
        // still blank, as a lone page number is.
        let mut p = pages(1);
        let f = latin(&mut p, "Tahoma,Bold");
        draw(&mut p, 0, TEXT, font(f));
        assert_eq!(
            reports(&mut p, zeyada()),
            [BTreeSet::from([Missing::StandIn(
                Some("Tahoma".into()),
                "Zeyada".into()
            )])]
        );
        assert_eq!(
            p.blank_pages_with(BLANK_TOLERANCE, Some(zeyada())).unwrap(),
            [0]
        );
    }

    #[test]
    fn a_blank_page_after_a_lost_picture_is_still_blank() {
        // Page 0's picture can't be decoded, so page 1, which looks blank, is drawn again: still blank.
        let mut p = pages(2);
        let im = broken_picture(&mut p);
        draw(&mut p, 0, PICTURE, im);
        let hp = render::load(p.render_snapshot().unwrap(), None).unwrap();
        let fonts = no_fonts();
        let renderer = || Renderer::with_fonts(&hp, fonts.clone());
        let mut c = Checker::new(renderer());
        c.blank(0, BLANK_TOLERANCE, renderer);
        assert!(c.blank(1, BLANK_TOLERANCE, renderer));
    }
}
