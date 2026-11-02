//! Laying pages out on sheets of paper for printing: where each page goes on its sheet (the
//! placement model, [`place`]), and the sheets themselves ([`Pdf::print_sheets`]), each a
//! sheet of the paper with its page placed on it whole as a drawing of its own (a form
//! XObject, so text stays text), the comments and form fields meant for print drawn in.
//! Every printout is made of these sheets, on every platform, so what reaches the printer is
//! what the placements say.

use crate::form_flatten::Flat;
use crate::matrix::{self as mx, M};
use crate::paper::Paper;
use crate::{Pdf, Result};
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};
use std::collections::HashSet;
use std::io::Write;

/// How near each edge LimarePDF keeps clear when the printer doesn't say how near it can
/// print: a quarter of an inch, as GTK assumes.
pub const ASSUMED_MARGIN: f32 = 18.0;

/// The most a page can be enlarged with [`Scale::Custom`], in percent.
pub const MAX_PERCENT: u16 = 400;

/// Annotation flag (`/F`) for an annotation that is printed.
const PRINT: i64 = 1 << 2;

/// Which way the pages go on the paper.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    /// Each page as its shape says: one wider than it is tall across a sheet held sideways.
    #[default]
    Automatic,
    /// Every page on a sheet held upright.
    Portrait,
    /// Every page on a sheet held sideways.
    Landscape,
}

/// How big the pages are printed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scale {
    /// At their own size, and shrunk to the printable area when larger.
    #[default]
    ShrinkOversized,
    /// Enlarged or shrunk to fill the printable area.
    Fit,
    /// At their own size: what falls outside the printable area doesn't print.
    ActualSize,
    /// This percentage of their own size, 1 to [`MAX_PERCENT`].
    Custom(u16),
}

/// The paper, and how the pages go on it.
#[derive(Clone, Debug, PartialEq)]
pub struct PageSetup {
    pub paper: Paper,
    pub orientation: Orientation,
    pub scale: Scale,
}

/// Where a page goes on its sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// The sheet's width and height in points: the paper's.
    pub sheet: (f32, f32),
    /// Turned a quarter turn anticlockwise, so that its top is at the sheet's left.
    pub turned: bool,
    /// Its size on the sheet over its own.
    pub scale: f32,
    /// Its box on the sheet, in points from the sheet's bottom-left corner: left, bottom,
    /// right and top.
    pub rect: [f32; 4],
    /// Part of it falls outside the printable area, so won't print.
    pub cut: bool,
}

impl Paper {
    /// How near each edge the printer can't print (top, right, bottom and left, in points),
    /// or [`ASSUMED_MARGIN`] on every side when the printer doesn't say.
    pub fn margins_or_assumed(&self) -> [f32; 4] {
        self.margins.unwrap_or([ASSUMED_MARGIN; 4])
    }
}

/// Where a page `size` points large, as it is shown (after its own turn), goes on `setup`'s
/// paper. Turned when the orientation asks: Automatic turns a page whose shape is the other
/// way round from the sheet's. Scaled as `setup` says against the printable area, the paper
/// less its margins. Centred on the sheet as far as the printable area allows; one larger
/// than the printable area is centred on it, losing as much on each side.
pub fn place(size: (f32, f32), setup: &PageSetup) -> Placed {
    let (pw, ph) = (setup.paper.size.0.max(1.0), setup.paper.size.1.max(1.0));
    let (w, h) = (size.0.max(1.0), size.1.max(1.0));
    let turned = match setup.orientation {
        Orientation::Automatic => (w > h && pw < ph) || (w < h && pw > ph),
        o => (o == Orientation::Landscape) != (pw > ph),
    };
    let (w, h) = if turned { (h, w) } else { (w, h) };
    let [top, right, bottom, left] = setup.paper.margins_or_assumed();
    let (aw, ah) = ((pw - left - right).max(1.0), (ph - top - bottom).max(1.0));
    let fit = (aw / w).min(ah / h);
    let scale = match setup.scale {
        Scale::ShrinkOversized => fit.min(1.0),
        Scale::Fit => fit,
        Scale::ActualSize => 1.0,
        Scale::Custom(p) => p.clamp(1, MAX_PERCENT) as f32 / 100.0,
    };
    let (sw, sh) = (w * scale, h * scale);
    let along = |paper: f32, low: f32, area: f32, len: f32| {
        if len <= area {
            // Not `clamp`: rounding can put the highest start a hair below the lowest, for
            // a page scaled to the area's own width or height.
            ((paper - len) / 2.0).min(low + area - len).max(low)
        } else {
            low + (area - len) / 2.0
        }
    };
    let (x, y) = (along(pw, left, aw, sw), along(ph, bottom, ah, sh));
    // A hundredth of a point over is rounding.
    let cut = sw > aw + 0.01 || sh > ah + 0.01;
    Placed {
        sheet: (pw, ph),
        turned,
        scale,
        rect: [x, y, x + sw, y + sh],
        cut,
    }
}

impl Placed {
    /// The matrix that puts a page shown `size` points large, its bottom-left corner at the
    /// origin, where this placement says: turned anticlockwise when [`Placed::turned`],
    /// scaled, and moved to its box.
    pub(crate) fn matrix(&self, size: (f32, f32)) -> M {
        let turn = match self.turned {
            true => [0.0, 1.0, -1.0, 0.0, size.1, 0.0],
            false => mx::ID,
        };
        let s = self.scale;
        mx::mul(turn, [s, 0.0, 0.0, s, self.rect[0], self.rect[1]])
    }
}

/// Printing draws the annotations meant for print (form fields among them) and leaves out
/// the others, and pop-up notes, which print only as the comments they belong to.
fn print_annot(pdf: &Pdf, d: &Dictionary) -> Flat {
    let flags = d.get(b"F").ok().and_then(|o| pdf.int_value(o)).unwrap_or(0);
    let popup = d.get(b"Subtype").and_then(|o| o.as_name()).ok() == Some(b"Popup");
    if flags & PRINT != 0 && !popup {
        Flat::Draw
    } else {
        Flat::Drop
    }
}

/// The name a sheet gives its page's drawing.
const PAGE: &str = "LimarePage";

fn nums(v: &[f32]) -> Object {
    Object::Array(v.iter().map(|&x| x.into()).collect())
}

impl Pdf {
    /// The sheets that print `pages` (zero-based, in order) as `setup` says: a document of
    /// one sheet of the paper for each page, the page placed on it whole as [`place`] says,
    /// with its comments and form fields meant for print drawn in and the others left out,
    /// unencrypted. A field whose value LimarePDF can't draw prints empty
    /// ([`Pdf::unprintable_fields`] names them first). The document itself is unchanged.
    pub fn print_sheets(&self, pages: &[usize], setup: &PageSetup) -> Result<Pdf> {
        let mut out = self.extract(pages)?;
        out.encryption = None;
        out.flatten_form_for_print()?;
        let ids = out.page_ids();
        let missing = out.missing_appearances(&ids);
        out.draw_annotations(&missing);
        for page in ids {
            out.flatten_page(page, print_annot, None, "LimareAnnot", &mut HashSet::new())?;
            out.make_sheet(page, setup)?;
        }
        Ok(out)
    }

    /// Turn page `page` into its sheet: its content and resources become a drawing whose box
    /// is the page's visible box, placed on a page the paper's size.
    fn make_sheet(&mut self, page: ObjectId, setup: &PageSetup) -> Result<()> {
        let bbox = self.page_box(page);
        let (shown, w, h) = self.visual_matrix(page);
        let placed = place((w, h), setup);
        let m = mx::mul(mx::inv(shown).unwrap_or(mx::ID), placed.matrix((w, h)));
        let mut form = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => nums(&bbox),
        };
        if let Some(r) = self.inherited(page, b"Resources") {
            form.set("Resources", r);
        }
        if let Ok(g) = self.doc.get_dictionary(page)?.get(b"Group") {
            form.set("Group", g.clone());
        }
        let drawing = self.page_drawing(page, form)?;
        let drawing = self.doc.add_object(drawing);
        let m = m.map(|x| format!("{x:.4}")).join(" ");
        let content = format!("q {m} cm /{PAGE} Do Q\n").into_bytes();
        let content = self.doc.add_object(Stream::new(Dictionary::new(), content));
        let (pw, ph) = placed.sheet;
        let d = self.doc.get_dictionary_mut(page)?;
        for key in [
            &b"Annots"[..],
            b"BleedBox",
            b"TrimBox",
            b"ArtBox",
            b"UserUnit",
            b"VP",
            b"Thumb",
            b"B",
            b"AA",
            b"StructParents",
            b"Tabs",
        ] {
            d.remove(key);
        }
        // Set on the sheet itself, so that nothing comes from the page tree.
        d.set("MediaBox", nums(&[0.0, 0.0, pw, ph]));
        d.set("CropBox", nums(&[0.0, 0.0, pw, ph]));
        d.set("Rotate", 0);
        d.set(
            "Resources",
            dictionary! { "XObject" => dictionary! { PAGE => drawing } },
        );
        d.set("Contents", content);
        Ok(())
    }

    /// Page `page`'s content as a form XObject with the entries `form`: a page with one
    /// content stream gives it as it is stored, one with several their content joined.
    fn page_drawing(&self, page: ObjectId, mut form: Dictionary) -> Result<Stream> {
        let streams = self.doc.get_page_contents(page);
        if let [one] = streams[..] {
            if let Ok(s) = self.doc.get_object(one).and_then(|o| o.as_stream()) {
                for key in [&b"Filter"[..], b"DecodeParms"] {
                    if let Ok(v) = s.dict.get(key) {
                        form.set(key.to_vec(), v.clone());
                    }
                }
                return Ok(Stream::new(form, s.content.clone()));
            }
        }
        let joined = self
            .doc
            .get_page_content_with_limit(page, crate::MAX_CONTENT)?;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        z.write_all(&joined)?;
        form.set("Filter", "FlateDecode");
        Ok(Stream::new(form, z.finish()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Letter, with the margins the user's HP reports (4.23 mm, 12 points).
    fn letter(margins: Option<[f32; 4]>) -> Paper {
        Paper {
            id: "na_letter_8.5x11in".into(),
            label: "Letter (8.5 × 11 in)".into(),
            size: (612.0, 792.0),
            margins,
        }
    }

    fn setup(paper: Paper, orientation: Orientation, scale: Scale) -> PageSetup {
        PageSetup {
            paper,
            orientation,
            scale,
        }
    }

    const HP: Option<[f32; 4]> = Some([12.0; 4]);

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
    }

    #[test]
    fn a_small_page_prints_at_its_own_size_centred() {
        // 4 x 3 in, wider than tall: turned onto the upright sheet.
        let s = setup(letter(HP), Orientation::Automatic, Scale::ShrinkOversized);
        let p = place((288.0, 216.0), &s);
        assert!(p.turned && !p.cut);
        assert_eq!(p.scale, 1.0);
        assert!(close(p.rect, [198.0, 252.0, 414.0, 540.0]), "{p:?}");
        assert_eq!(p.sheet, (612.0, 792.0));
    }

    #[test]
    fn a_page_the_papers_size_shrinks_to_the_printable_area() {
        let s = setup(letter(HP), Orientation::Automatic, Scale::ShrinkOversized);
        let p = place((612.0, 792.0), &s);
        assert!(!p.turned && !p.cut);
        // The width limits: 588 of 612.
        assert!((p.scale - 588.0 / 612.0).abs() < 1e-5, "{p:?}");
        let h = 792.0 * 588.0 / 612.0;
        let y = (792.0 - h) / 2.0;
        assert!(close(p.rect, [12.0, y, 600.0, y + h]), "{p:?}");
    }

    #[test]
    fn fit_enlarges_a_small_page_to_the_printable_area() {
        let s = setup(letter(HP), Orientation::Automatic, Scale::Fit);
        let p = place((288.0, 216.0), &s);
        // Turned, 216 x 288; the height limits: 768 / 288.
        assert!(p.turned && !p.cut);
        assert!((p.scale - 768.0 / 288.0).abs() < 1e-5, "{p:?}");
        assert!(close(p.rect, [18.0, 12.0, 594.0, 780.0]), "{p:?}");
    }

    #[test]
    fn actual_size_centres_a_page_larger_than_the_paper_and_says_it_is_cut() {
        // A4 on Letter: wider and taller than the printable area.
        let s = setup(letter(HP), Orientation::Automatic, Scale::ActualSize);
        let p = place((595.28, 841.89), &s);
        assert!(!p.turned && p.cut);
        assert_eq!(p.scale, 1.0);
        let (x, y) = (12.0 + (588.0 - 595.28) / 2.0, 12.0 + (768.0 - 841.89) / 2.0);
        assert!(close(p.rect, [x, y, x + 595.28, y + 841.89]), "{p:?}");
    }

    #[test]
    fn custom_scale_goes_from_one_to_four_hundred_percent() {
        let at = |percent| {
            let s = setup(letter(HP), Orientation::Portrait, Scale::Custom(percent));
            place((100.0, 100.0), &s)
        };
        assert_eq!(at(50).scale, 0.5);
        assert_eq!(at(1).scale, 0.01);
        assert_eq!(at(0).scale, 0.01);
        assert_eq!(at(400).scale, 4.0);
        assert_eq!(at(1000).scale, 4.0);
        // 400 x 400 fits; centred.
        assert!(close(at(400).rect, [106.0, 196.0, 506.0, 596.0]));
        assert!(!at(400).cut);
        // A tiny page stays on the sheet, centred.
        let tiny = at(1).rect;
        assert!(close(tiny, [305.5, 395.5, 306.5, 396.5]), "{tiny:?}");
    }

    #[test]
    fn orientation_turns_the_pages_it_asks_for() {
        let portrait_page = (300.0, 500.0);
        let wide_page = (500.0, 300.0);
        let turned = |o, size, paper: Paper| place(size, &setup(paper, o, Scale::Fit)).turned;
        let up = letter(HP);
        assert!(!turned(Orientation::Automatic, portrait_page, up.clone()));
        assert!(turned(Orientation::Automatic, wide_page, up.clone()));
        assert!(!turned(Orientation::Portrait, wide_page, up.clone()));
        assert!(turned(Orientation::Landscape, portrait_page, up.clone()));
        // A square page isn't turned by Automatic.
        assert!(!turned(Orientation::Automatic, (400.0, 400.0), up));
        // Paper wider than it is tall (Ledger): held sideways as it comes.
        let ledger = Paper {
            id: "Ledger".into(),
            label: "Ledger (17 × 11 in)".into(),
            size: (1224.0, 792.0),
            margins: None,
        };
        assert!(turned(
            Orientation::Automatic,
            portrait_page,
            ledger.clone()
        ));
        assert!(!turned(Orientation::Automatic, wide_page, ledger.clone()));
        assert!(turned(Orientation::Portrait, wide_page, ledger.clone()));
        assert!(!turned(Orientation::Landscape, wide_page, ledger));
    }

    #[test]
    fn unknown_margins_keep_a_quarter_inch_clear() {
        let s = setup(letter(None), Orientation::Automatic, Scale::Fit);
        let p = place((612.0, 792.0), &s);
        // 576 x 756: the width limits.
        assert!((p.scale - 576.0 / 612.0).abs() < 1e-5, "{p:?}");
        assert_eq!(p.rect[0], 18.0);
        assert!((p.rect[2] - 594.0).abs() < 0.01);
        assert_eq!(letter(None).margins_or_assumed(), [18.0; 4]);
    }

    #[test]
    fn uneven_margins_keep_a_page_inside_the_printable_area() {
        // A deep top margin pushes a page centred on the sheet down below it.
        let paper = letter(Some([100.0, 12.0, 12.0, 12.0]));
        let s = setup(paper, Orientation::Portrait, Scale::ActualSize);
        let p = place((500.0, 600.0), &s);
        assert!(!p.cut);
        assert!(close(p.rect, [56.0, 92.0, 556.0, 692.0]), "{p:?}");
    }

    /// A page scaled to the printable area can come out a hair larger or smaller than it
    /// in floating point: placed all the same, inside the area. The margins the user's HP
    /// reports over IPP, 423 hundredths of a millimetre, made clamping its start panic.
    #[test]
    fn a_page_filling_the_printable_area_is_placed_whatever_the_rounding() {
        let hp = Some([423.0 / 2540.0 * 72.0; 4]);
        let sizes = [
            (612.0, 792.0),
            (792.0, 612.0),
            (595.28, 841.89),
            (842.0, 1191.0),
            (100.0, 100.0),
            (588.02, 768.02),
        ];
        let m = 423.0 / 2540.0 * 72.0;
        let orientations = [
            Orientation::Automatic,
            Orientation::Portrait,
            Orientation::Landscape,
        ];
        for orientation in orientations {
            for scale in [Scale::ShrinkOversized, Scale::Fit, Scale::ActualSize] {
                for size in sizes {
                    let p = place(size, &setup(letter(hp), orientation, scale));
                    let [x0, y0, x1, y1] = p.rect;
                    let inside = x0 >= m - 0.01 && y0 >= m - 0.01;
                    let inside = inside && x1 <= 612.0 - m + 0.01 && y1 <= 792.0 - m + 0.01;
                    assert!(inside || p.cut, "{size:?} {orientation:?} {scale:?}: {p:?}");
                }
            }
        }
    }

    #[test]
    fn a_huge_page_shrinks_onto_the_sheet() {
        // A0 on Letter.
        let s = setup(letter(HP), Orientation::Automatic, Scale::ShrinkOversized);
        let p = place((2384.0, 3370.0), &s);
        assert!(!p.cut && !p.turned);
        let [x0, y0, x1, y1] = p.rect;
        assert!(x0 >= 12.0 - 0.01 && y0 >= 12.0 - 0.01, "{p:?}");
        assert!(x1 <= 600.0 + 0.01 && y1 <= 780.0 + 0.01, "{p:?}");
    }

    #[test]
    fn the_matrix_puts_the_page_in_its_box_with_its_top_to_the_left_when_turned() {
        let s = setup(letter(HP), Orientation::Automatic, Scale::ShrinkOversized);
        let size = (288.0, 216.0);
        let p = place(size, &s);
        let m = p.matrix(size);
        let r = mx::transform_rect(m, [0.0, 0.0, size.0, size.1]);
        assert!(close(r, p.rect), "{r:?} {p:?}");
        // The page's top-left corner goes to the box's bottom-left, its top-right to the
        // box's top-left: the top runs up the left side.
        let (x, y) = mx::apply(m, 0.0, size.1);
        assert!((x - p.rect[0]).abs() < 0.01 && (y - p.rect[1]).abs() < 0.01);
        let (x, y) = mx::apply(m, size.0, size.1);
        assert!((x - p.rect[0]).abs() < 0.01 && (y - p.rect[3]).abs() < 0.01);
    }
}
