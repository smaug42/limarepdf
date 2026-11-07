//! Laying pages out on sheets of paper for printing: which pages go on each sheet and where
//! (the placement model: [`sheets`], with [`place`] for one page a sheet and the booklet's
//! folding order), and the sheets
//! themselves ([`Pdf::print_sheets`]), each a sheet of the paper with its pages placed on it
//! whole, each a drawing of its own (a form XObject, so text stays text), the comments and
//! form fields meant for print drawn in. Every printout is made of these sheets, on every
//! platform, so what reaches the printer is what the placements say.

use crate::form_flatten::Flat;
use crate::matrix::{self as mx, M};
use crate::paper::Paper;
use crate::print::Sides;
use crate::{Pdf, Result};
use anyhow::anyhow;
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

/// How many pages can go on a sheet.
pub const PAGES_PER_SHEET: [u8; 6] = [1, 2, 4, 6, 9, 16];

/// The order the pages fill a sheet's places in, as the sheet is held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageOrder {
    /// Left to right, top to bottom.
    #[default]
    Across,
    /// Right to left, top to bottom.
    AcrossFromRight,
    /// Top to bottom, left to right.
    Down,
    /// Top to bottom, right to left.
    DownFromRight,
}

/// How many pages go on each sheet, in which order, and whether each has a border.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerSheet {
    /// One of [`PAGES_PER_SHEET`]; another number is taken as the next of them above it.
    pub pages: u8,
    pub order: PageOrder,
    /// A thin line around each page, with more than one page a sheet.
    pub borders: bool,
}

impl Default for PerSheet {
    fn default() -> Self {
        Self {
            pages: 1,
            order: PageOrder::Across,
            borders: false,
        }
    }
}

impl PerSheet {
    /// The grid the pages go in: its longer and shorter side, in places.
    fn grid(&self) -> (usize, usize) {
        match self.pages {
            0 | 1 => (1, 1),
            2 => (2, 1),
            3 | 4 => (2, 2),
            5 | 6 => (3, 2),
            7..=9 => (3, 3),
            _ => (4, 4),
        }
    }

    /// How many pages go on each sheet: one of [`PAGES_PER_SHEET`].
    pub fn count(&self) -> usize {
        let (long, short) = self.grid();
        long * short
    }
}

/// The edge a booklet is bound along, as it is read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Binding {
    /// On the left, read left to right.
    #[default]
    Left,
    /// On the right, read right to left.
    Right,
}

/// Which sides of a booklet's sheets print.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BookletSides {
    /// Both, on a printer that prints both sides.
    #[default]
    Both,
    /// The fronts alone, for a printer that prints one side: the backs follow once the
    /// sheets are put back in.
    Fronts,
    /// The backs alone, on the sheets whose fronts are printed.
    Backs,
}

/// Printing as a booklet: the sheets folded in half and stacked, two pages a side, in the
/// order that reads straight through once folded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Booklet {
    pub binding: Binding,
    pub sides: BookletSides,
}

impl Booklet {
    /// How many sides print for `pages` pages: four pages a sheet, two a side.
    pub fn sides_printed(&self, pages: usize) -> usize {
        let sheets = pages.div_ceil(4);
        match self.sides {
            BookletSides::Both => sheets * 2,
            BookletSides::Fronts | BookletSides::Backs => sheets,
        }
    }

    /// The sheet (from 0) the side printed `i`th (from 0) is on, and whether it is its
    /// front.
    pub fn side(&self, i: usize) -> (usize, bool) {
        match self.sides {
            BookletSides::Both => (i / 2, i.is_multiple_of(2)),
            BookletSides::Fronts => (i, true),
            BookletSides::Backs => (i, false),
        }
    }

    /// How the printer prints the sides: both sides flipped on the short edge, as the
    /// sheets are held sideways, or one side for the fronts or backs alone.
    pub fn sides_used(&self) -> Sides {
        match self.sides {
            BookletSides::Both => Sides::ShortEdge,
            BookletSides::Fronts | BookletSides::Backs => Sides::One,
        }
    }
}

/// The paper, and how the pages go on it.
#[derive(Clone, Debug, PartialEq)]
pub struct PageSetup {
    pub paper: Paper,
    pub orientation: Orientation,
    /// How big the pages are printed, with one page a sheet; with more, each is fitted to
    /// its place.
    pub scale: Scale,
    pub per_sheet: PerSheet,
    /// Printed as a booklet, which sets the order, the holding and the size of the pages
    /// itself, in place of [`PageSetup::orientation`], [`PageSetup::scale`] and
    /// [`PageSetup::per_sheet`].
    pub booklet: Option<Booklet>,
}

/// Where a page goes on its sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// The sheet's width and height in points: the paper's.
    pub sheet: (f32, f32),
    /// How many quarter turns anticlockwise it is turned on the paper: with one, its top is
    /// at the sheet's left. A page turned in its place on a sheet held sideways is turned
    /// twice.
    pub turns: u8,
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
        turns: turned as u8,
        scale,
        rect: [x, y, x + sw, y + sh],
        cut,
    }
}

impl Placed {
    /// The matrix that puts a page shown `size` points large, its bottom-left corner at the
    /// origin, where this placement says: turned anticlockwise [`Placed::turns`] times,
    /// scaled, and moved to its box.
    pub(crate) fn matrix(&self, (w, h): (f32, f32)) -> M {
        // Turned about its bottom-left corner, then moved back above and right of it.
        let turn = match self.turns % 4 {
            1 => [0.0, 1.0, -1.0, 0.0, h, 0.0],
            2 => [-1.0, 0.0, 0.0, -1.0, w, h],
            3 => [0.0, -1.0, 1.0, 0.0, 0.0, w],
            _ => mx::ID,
        };
        let s = self.scale;
        mx::mul(turn, [s, 0.0, 0.0, s, self.rect[0], self.rect[1]])
    }
}

/// One sheet of a printout: the pages on it, and where each goes.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetPlan {
    /// Its width and height in points: the paper's.
    pub size: (f32, f32),
    /// Held sideways to be read: turned a quarter turn clockwise, its left edge at the top.
    pub sideways: bool,
    /// Its pages, by their place among the pages printed (from 0), each where it goes.
    pub pages: Vec<(usize, Placed)>,
    /// A thin line is drawn around each page.
    pub borders: bool,
}

/// The line drawn around each page with [`PerSheet::borders`], in points.
const BORDER: f32 = 0.5;

/// The sheets that print pages `sizes` points large as shown (after their own turn), in the
/// order printed, as `setup` says. With one page a sheet, each page is placed as [`place`]
/// says. With more, they go in a grid on the printable area, filling its places in
/// [`PerSheet::order`] as the sheet is held, each fitted to its place, larger or smaller,
/// and centred in it; one whose shape is the other way round from its place's is turned in
/// it. Automatic holds each sheet the way that prints its pages, upright, largest; Portrait
/// and Landscape hold it as they say. A booklet ([`PageSetup::booklet`]) gives its sides
/// instead, a sheet each, in folding order, a side with no pages left blank.
pub fn sheets(sizes: &[(f32, f32)], setup: &PageSetup) -> Vec<SheetPlan> {
    if let Some(b) = setup.booklet {
        return booklet(sizes, &setup.paper, b);
    }
    let per = setup.per_sheet;
    let n = per.count();
    if n == 1 {
        let one = |(i, &size)| {
            let p = place(size, setup);
            SheetPlan {
                size: p.sheet,
                sideways: p.turns == 1,
                pages: vec![(i, p)],
                borders: false,
            }
        };
        return sizes.iter().enumerate().map(one).collect();
    }
    let paper = (setup.paper.size.0.max(1.0), setup.paper.size.1.max(1.0));
    let sheet = |(k, pages): (usize, &[(f32, f32)])| {
        let sideways = match setup.orientation {
            Orientation::Automatic => {
                let area = |sideways| -> f32 {
                    let places = places(&setup.paper, sideways, &per);
                    let fitted = places.iter().zip(pages).map(|(&at, &size)| {
                        let (_, s, _) = fit(size, at, false);
                        size.0.max(1.0) * size.1.max(1.0) * s * s
                    });
                    fitted.sum()
                };
                // As it comes, unless sideways prints them larger.
                area(true) > area(false) * 1.001
            }
            o => (o == Orientation::Landscape) != (paper.0 > paper.1),
        };
        let places = places(&setup.paper, sideways, &per);
        let placed = places
            .into_iter()
            .zip(pages)
            .enumerate()
            .map(|(j, (at, &size))| {
                let (rect, scale, turned) = fit(size, at, true);
                let p = Placed {
                    sheet: paper,
                    turns: turned as u8 + sideways as u8,
                    scale,
                    rect: onto_paper(rect, paper, sideways),
                    cut: false,
                };
                (k * n + j, p)
            });
        SheetPlan {
            size: paper,
            sideways,
            pages: placed.collect(),
            borders: per.borders,
        }
    };
    sizes.chunks(n).enumerate().map(sheet).collect()
}

/// The places on `paper` held sideways or not for [`PerSheet::count`] pages, in the order
/// they are filled: each a box (left, bottom, right, top) on the sheet as held, its
/// bottom-left corner the origin. The grid's longer side is along the sheet's longer side.
fn places(paper: &Paper, sideways: bool, per: &PerSheet) -> Vec<[f32; 4]> {
    let ((w, h), [top, right, bottom, left]) = held(paper, sideways);
    let (long, short) = per.grid();
    let (cols, rows) = if w >= h { (long, short) } else { (short, long) };
    let (aw, ah) = ((w - left - right).max(1.0), (h - top - bottom).max(1.0));
    let (cw, ch) = (aw / cols as f32, ah / rows as f32);
    let place = |i: usize| {
        let (col, row) = match per.order {
            PageOrder::Across => (i % cols, i / cols),
            PageOrder::AcrossFromRight => (cols - 1 - i % cols, i / cols),
            PageOrder::Down => (i / rows, i % rows),
            PageOrder::DownFromRight => (cols - 1 - i / rows, i % rows),
        };
        // Rows count down from the top.
        let x = left + col as f32 * cw;
        let y = bottom + (rows - 1 - row) as f32 * ch;
        [x, y, x + cw, y + ch]
    };
    (0..cols * rows).map(place).collect()
}

/// `paper` held sideways or not: its width and height as held, and how near each edge as
/// held (top, right, bottom and left) the printer can't print.
fn held(paper: &Paper, sideways: bool) -> ((f32, f32), [f32; 4]) {
    let (pw, ph) = (paper.size.0.max(1.0), paper.size.1.max(1.0));
    let [t, r, b, l] = paper.margins_or_assumed();
    // Held sideways, the paper's left edge is at the top, its top at the right.
    match sideways {
        true => ((ph, pw), [l, t, r, b]),
        false => ((pw, ph), [t, r, b, l]),
    }
}

/// A box on the sheet as held, sideways or not, as a box on the paper `paper` points large.
fn onto_paper(rect: [f32; 4], paper: (f32, f32), sideways: bool) -> [f32; 4] {
    // As held sideways, the paper's left edge is at the top.
    match sideways {
        true => [paper.0 - rect[3], rect[0], paper.0 - rect[1], rect[2]],
        false => rect,
    }
}

/// The sides of a booklet of pages `sizes` points large on `paper`, in the order printed.
/// The pages are padded with blank places to a multiple of four; sheet `k`'s front holds
/// the last page but `2k` and page `2k`, its back the next and the one before, so that the
/// sheets folded and stacked read straight through; bound on the right, each side is
/// mirrored. Each sheet is held with its longer side across, a page each side of the fold
/// in the paper's middle, fitted to its half, larger or smaller, and centred in it, never
/// turned. The halves are as wide as the narrower side's margin allows, so that the fold
/// stays in the middle.
fn booklet(sizes: &[(f32, f32)], paper: &Paper, b: Booklet) -> Vec<SheetPlan> {
    let size = (paper.size.0.max(1.0), paper.size.1.max(1.0));
    let sideways = size.0 < size.1;
    let ((w, h), [top, right, bottom, left]) = held(paper, sideways);
    let middle = w / 2.0;
    let half = (middle - left).min(w - right - middle).max(0.5);
    let (y0, y1) = (bottom, (h - top).max(bottom + 1.0));
    let halves = [
        [middle - half, y0, middle, y1],
        [middle, y0, middle + half, y1],
    ];
    let side = |mut pages: [usize; 2]| {
        if b.binding == Binding::Right {
            pages.reverse();
        }
        let placed = pages.into_iter().zip(halves).filter_map(|(i, at)| {
            let (rect, scale, _) = fit(*sizes.get(i)?, at, false);
            let p = Placed {
                sheet: size,
                turns: sideways as u8,
                scale,
                rect: onto_paper(rect, size, sideways),
                cut: false,
            };
            Some((i, p))
        });
        SheetPlan {
            size,
            sideways,
            pages: placed.collect(),
            borders: false,
        }
    };
    let last = sizes.len().div_ceil(4) * 4;
    let mut out = Vec::new();
    for k in 0..last / 4 {
        let front = [last - 1 - 2 * k, 2 * k];
        let back = [2 * k + 1, last - 2 - 2 * k];
        if b.sides != BookletSides::Backs {
            out.push(side(front));
        }
        if b.sides != BookletSides::Fronts {
            out.push(side(back));
        }
    }
    out
}

/// A page `size` points large fitted to the box `at`, larger or smaller, and centred in
/// it: its box, its scale, and whether it is turned a quarter turn anticlockwise, as it is
/// when `turn` lets it and its shape is the other way round from the box's.
fn fit(size: (f32, f32), at: [f32; 4], turn: bool) -> ([f32; 4], f32, bool) {
    let (w, h) = (size.0.max(1.0), size.1.max(1.0));
    let (aw, ah) = (at[2] - at[0], at[3] - at[1]);
    let turned = turn && ((w > h && aw < ah) || (w < h && aw > ah));
    let (w, h) = if turned { (h, w) } else { (w, h) };
    let s = (aw / w).min(ah / h);
    let (sw, sh) = (w * s, h * s);
    let (x, y) = (at[0] + (aw - sw) / 2.0, at[1] + (ah - sh) / 2.0);
    ([x, y, x + sw, y + sh], s, turned)
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

/// A page made a drawing to place on sheets.
struct Drawing {
    id: ObjectId,
    /// From the page's own space to the page as shown, its bottom-left corner the origin.
    shown: M,
    /// Its size as shown.
    size: (f32, f32),
    /// The page's transparency group.
    group: Option<Object>,
}

impl Pdf {
    /// The sheets that print `pages` (zero-based, in order) as `setup` says ([`sheets`]): a
    /// document of the sheets of the paper, each page placed whole where the plan says,
    /// with its comments and form fields meant for print drawn in and the others left out,
    /// unencrypted. A field whose value LimarePDF can't draw prints empty
    /// ([`Pdf::unprintable_fields`] names them first). The document itself is unchanged.
    pub fn print_sheets(&self, pages: &[usize], setup: &PageSetup) -> Result<Pdf> {
        if pages.is_empty() {
            return Err(anyhow!("no pages selected"));
        }
        let all = self.page_sizes();
        let sizes = pages.iter().map(|&i| match all.get(i) {
            Some(&s) => Ok(s),
            None => Err(anyhow!(
                "page {} does not exist (document has {})",
                i + 1,
                all.len()
            )),
        });
        let sizes = sizes.collect::<Result<Vec<_>>>()?;
        self.print_planned(pages, &sheets(&sizes, setup))
    }

    /// The sheets `plan` makes of `pages` (zero-based), as [`Pdf::print_sheets`] makes
    /// them: the pages each sheet names are `pages`' by their place in it. The preview
    /// makes one sheet at a time with it, a booklet's blank side from no pages.
    pub fn print_planned(&self, pages: &[usize], plan: &[SheetPlan]) -> Result<Pdf> {
        let mut out = match pages {
            [] => Pdf::new(),
            _ => self.extract(pages)?,
        };
        out.encryption = None;
        out.flatten_form_for_print()?;
        let ids = out.page_ids();
        let missing = out.missing_appearances(&ids);
        out.draw_annotations(&missing);
        let mut drawings = Vec::with_capacity(ids.len());
        for &page in &ids {
            out.flatten_page(
                page,
                print_annot,
                None,
                "LimareAnnot",
                &mut HashSet::new(),
                None,
            )?;
            drawings.push(out.as_drawing(page)?);
        }
        // What goes to the pages, which the sheets replace, goes; their labels too.
        let catalog = out.doc.catalog_mut()?;
        for key in [
            &b"PageLabels"[..],
            b"OpenAction",
            b"Outlines",
            b"StructTreeRoot",
        ] {
            catalog.remove(key);
        }
        let mut made = Vec::with_capacity(plan.len());
        for sheet in plan {
            made.push(out.make_sheet(sheet, &drawings)?);
        }
        out.set_page_order(&made)?;
        for id in ids {
            out.doc.objects.remove(&id);
        }
        Ok(out)
    }

    /// Page `page` as a drawing: its content and resources as a form XObject whose box is
    /// the page's visible box.
    fn as_drawing(&mut self, page: ObjectId) -> Result<Drawing> {
        let bbox = self.page_box(page);
        let (shown, w, h) = self.visual_matrix(page);
        let mut form = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => nums(&bbox),
        };
        if let Some(r) = self.inherited(page, b"Resources") {
            form.set("Resources", r);
        }
        let group = self.doc.get_dictionary(page)?.get(b"Group").ok().cloned();
        if let Some(g) = &group {
            form.set("Group", g.clone());
        }
        let drawing = self.page_drawing(page, form)?;
        Ok(Drawing {
            id: self.doc.add_object(drawing),
            shown: mx::inv(shown).unwrap_or(mx::ID),
            size: (w, h),
            group,
        })
    }

    /// A new page, `sheet`, with its pages' `drawings` placed on it, and their borders.
    fn make_sheet(&mut self, sheet: &SheetPlan, drawings: &[Drawing]) -> Result<ObjectId> {
        let mut xobjects = Dictionary::new();
        let mut content = String::new();
        let mut group = None;
        for (n, (i, placed)) in sheet.pages.iter().enumerate() {
            let d = drawings
                .get(*i)
                .ok_or_else(|| anyhow!("a sheet names page {} of {}", i + 1, drawings.len()))?;
            let name = match n {
                0 => PAGE.to_string(),
                n => format!("{PAGE}{n}"),
            };
            let m = mx::mul(d.shown, placed.matrix(d.size));
            let m = m.map(|x| format!("{x:.4}")).join(" ");
            content += &format!("q {m} cm /{name} Do Q\n");
            xobjects.set(name, d.id);
            group = group.or(d.group.clone());
        }
        if sheet.borders && !sheet.pages.is_empty() {
            content += &format!("q 0 G {BORDER} w\n");
            for (_, p) in &sheet.pages {
                let [x0, y0, x1, y1] = p.rect;
                let (w, h) = (x1 - x0, y1 - y0);
                content += &format!("{x0:.4} {y0:.4} {w:.4} {h:.4} re\n");
            }
            content += "S Q\n";
        }
        let content = self
            .doc
            .add_object(Stream::new(Dictionary::new(), content.into_bytes()));
        let (pw, ph) = sheet.size;
        // Everything set on the sheet itself, so that nothing comes from the page tree.
        let mut d = dictionary! {
            "Type" => "Page",
            "MediaBox" => nums(&[0.0, 0.0, pw, ph]),
            "CropBox" => nums(&[0.0, 0.0, pw, ph]),
            "Rotate" => 0,
            "Resources" => dictionary! { "XObject" => xobjects },
            "Contents" => content,
        };
        if let Some(g) = group {
            d.set("Group", g);
        }
        Ok(self.doc.add_object(d))
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
            per_sheet: PerSheet::default(),
            booklet: None,
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
        assert!(p.turns == 1 && !p.cut);
        assert_eq!(p.scale, 1.0);
        assert!(close(p.rect, [198.0, 252.0, 414.0, 540.0]), "{p:?}");
        assert_eq!(p.sheet, (612.0, 792.0));
    }

    #[test]
    fn a_page_the_papers_size_shrinks_to_the_printable_area() {
        let s = setup(letter(HP), Orientation::Automatic, Scale::ShrinkOversized);
        let p = place((612.0, 792.0), &s);
        assert!(p.turns == 0 && !p.cut);
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
        assert!(p.turns == 1 && !p.cut);
        assert!((p.scale - 768.0 / 288.0).abs() < 1e-5, "{p:?}");
        assert!(close(p.rect, [18.0, 12.0, 594.0, 780.0]), "{p:?}");
    }

    #[test]
    fn actual_size_centres_a_page_larger_than_the_paper_and_says_it_is_cut() {
        // A4 on Letter: wider and taller than the printable area.
        let s = setup(letter(HP), Orientation::Automatic, Scale::ActualSize);
        let p = place((595.28, 841.89), &s);
        assert!(p.turns == 0 && p.cut);
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
        let turned = |o, size, paper: Paper| place(size, &setup(paper, o, Scale::Fit)).turns == 1;
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
        assert!(!p.cut && p.turns == 0);
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

    /// Turned twice, a page's top runs along the bottom of its box; three times, up its
    /// right side.
    #[test]
    fn the_matrix_turns_a_page_any_number_of_quarter_turns() {
        let size = (300.0, 200.0);
        for (turns, rect, top_left, top_right) in [
            (2, [10.0, 20.0, 160.0, 120.0], (160.0, 20.0), (10.0, 20.0)),
            (3, [10.0, 20.0, 110.0, 170.0], (110.0, 170.0), (110.0, 20.0)),
        ] {
            let p = Placed {
                sheet: (612.0, 792.0),
                turns,
                scale: 0.5,
                rect,
                cut: false,
            };
            let m = p.matrix(size);
            let r = mx::transform_rect(m, [0.0, 0.0, size.0, size.1]);
            assert!(close(r, rect), "{turns}: {r:?}");
            let (x, y) = mx::apply(m, 0.0, size.1);
            assert!((x - top_left.0).abs() < 0.01 && (y - top_left.1).abs() < 0.01);
            let (x, y) = mx::apply(m, size.0, size.1);
            assert!((x - top_right.0).abs() < 0.01 && (y - top_right.1).abs() < 0.01);
        }
    }

    fn several(paper: Paper, orientation: Orientation, pages: u8, order: PageOrder) -> PageSetup {
        PageSetup {
            per_sheet: PerSheet {
                pages,
                order,
                borders: false,
            },
            ..setup(paper, orientation, Scale::ShrinkOversized)
        }
    }

    const LETTER: (f32, f32) = (612.0, 792.0);

    /// Two upright pages a sheet: side by side on a sheet held sideways, each turned onto
    /// the paper, the first at the paper's bottom (the left as it is held), each fitted to
    /// half the printable area.
    #[test]
    fn two_upright_pages_go_side_by_side_on_a_sheet_held_sideways() {
        let s = several(letter(HP), Orientation::Automatic, 2, PageOrder::Across);
        let plan = sheets(&[LETTER; 3], &s);
        assert_eq!(plan.len(), 2);
        let first = &plan[0];
        assert!(first.sideways);
        assert_eq!(first.size, LETTER);
        // Held sideways the printable area is 768 by 588: each half 384 by 588.
        let k = 384.0 / 612.0;
        let h = 792.0 * k;
        let (y0, y1) = (12.0 + (588.0 - h) / 2.0, 12.0 + (588.0 + h) / 2.0);
        let [(0, a), (1, b)] = first.pages[..] else {
            panic!("{first:?}");
        };
        assert!(a.turns == 1 && b.turns == 1 && !a.cut);
        assert!((a.scale - k).abs() < 1e-5, "{a:?}");
        let (x0, x1) = (612.0 - y1, 612.0 - y0);
        assert!(close(a.rect, [x0, 12.0, x1, 396.0]), "{a:?}");
        assert!(close(b.rect, [x0, 396.0, x1, 780.0]), "{b:?}");
        // The third alone on the second sheet, in the first place.
        let [(2, c)] = plan[1].pages[..] else {
            panic!("{:?}", plan[1]);
        };
        assert!(close(c.rect, a.rect));
    }

    /// The places' centres on a sheet held as it comes, by page: left to right as `x`
    /// grows, top to bottom as `y` falls.
    fn centres(plan: &SheetPlan) -> Vec<(f32, f32)> {
        let c = |p: &Placed| ((p.rect[0] + p.rect[2]) / 2.0, (p.rect[1] + p.rect[3]) / 2.0);
        plan.pages.iter().map(|(_, p)| c(p)).collect()
    }

    #[test]
    fn the_pages_fill_the_places_in_the_order_chosen() {
        // The places' middles: left and right, top and bottom.
        let (l, r, t, b) = (159.0, 453.0, 588.0, 204.0);
        for (order, want) in [
            (PageOrder::Across, [(l, t), (r, t), (l, b), (r, b)]),
            (PageOrder::AcrossFromRight, [(r, t), (l, t), (r, b), (l, b)]),
            (PageOrder::Down, [(l, t), (l, b), (r, t), (r, b)]),
            (PageOrder::DownFromRight, [(r, t), (r, b), (l, t), (l, b)]),
        ] {
            let s = several(letter(HP), Orientation::Portrait, 4, order);
            let plan = sheets(&[(100.0, 100.0); 4], &s);
            let got = centres(&plan[0]);
            let near = got
                .iter()
                .zip(want)
                .all(|(a, b)| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01);
            assert!(near, "{order:?}: {got:?}");
        }
        // Held sideways, the order is as it is read: the second page right of the first,
        // which is up the paper.
        let s = several(letter(HP), Orientation::Landscape, 4, PageOrder::Across);
        let plan = sheets(&[(100.0, 100.0); 4], &s);
        assert!(plan[0].sideways);
        let c = centres(&plan[0]);
        assert!(c[1].1 > c[0].1 && (c[1].0 - c[0].0).abs() < 0.01, "{c:?}");
        // The third under the first: nearer the paper's right edge.
        assert!(c[2].0 > c[0].0 && (c[2].1 - c[0].1).abs() < 0.01, "{c:?}");
    }

    /// Each count's grid, its longer side along the sheet's longer side as held: upright,
    /// columns by rows; sideways, the paper's columns are the rows as held.
    #[test]
    fn each_count_has_its_grid() {
        let distinct = |v: Vec<f32>| {
            let mut v: Vec<i32> = v.into_iter().map(|x| x.round() as i32).collect();
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        let grid = |plan: &SheetPlan| {
            let c = centres(plan);
            (
                distinct(c.iter().map(|p| p.0).collect()),
                distinct(c.iter().map(|p| p.1).collect()),
            )
        };
        let small = [(100.0, 100.0); 16];
        for (n, upright, sideways) in [
            (2, (1, 2), (1, 2)),
            (4, (2, 2), (2, 2)),
            (6, (2, 3), (2, 3)),
            (9, (3, 3), (3, 3)),
            (16, (4, 4), (4, 4)),
        ] {
            let s = several(letter(HP), Orientation::Portrait, n, PageOrder::Across);
            let plan = sheets(&small[..n as usize], &s);
            assert_eq!(plan.len(), 1, "{n}");
            assert_eq!(s.per_sheet.count(), n as usize);
            assert!(!plan[0].sideways);
            assert_eq!(grid(&plan[0]), upright, "{n} upright");
            // Held sideways, the grid turns with the sheet: on the paper it is the same.
            let s = several(letter(HP), Orientation::Landscape, n, PageOrder::Across);
            let plan = sheets(&small[..n as usize], &s);
            assert!(plan[0].sideways);
            assert_eq!(grid(&plan[0]), sideways, "{n} sideways");
        }
        // A number between counts takes the next above it.
        let s = several(letter(HP), Orientation::Portrait, 3, PageOrder::Across);
        assert_eq!(s.per_sheet.count(), 4);
        assert_eq!(sheets(&small[..5], &s).len(), 2);
    }

    /// A page whose shape is the other way round from its place's is turned in it:
    /// upright pages two a sheet held upright, wide ones two a sheet held sideways (turned
    /// twice on the paper), and the one wide page among upright ones four a sheet.
    #[test]
    fn a_page_is_turned_in_a_place_the_other_way_round() {
        let s = several(letter(HP), Orientation::Portrait, 2, PageOrder::Across);
        let plan = sheets(&[LETTER; 2], &s);
        let (_, p) = plan[0].pages[0];
        assert!(!plan[0].sideways && p.turns == 1, "{p:?}");
        // Each place 588 by 384: the page turned, 792 by 612, fits by its height.
        assert!((p.scale - 384.0 / 612.0).abs() < 1e-5, "{p:?}");
        let s = several(letter(HP), Orientation::Landscape, 2, PageOrder::Across);
        let plan = sheets(&[(792.0, 612.0); 2], &s);
        assert!(plan[0].sideways && plan[0].pages[0].1.turns == 2);
        let s = several(letter(HP), Orientation::Automatic, 4, PageOrder::Across);
        let plan = sheets(&[LETTER, LETTER, (792.0, 612.0), LETTER], &s);
        assert!(!plan[0].sideways);
        let turns: Vec<u8> = plan[0].pages.iter().map(|(_, p)| p.turns).collect();
        assert_eq!(turns, [0, 0, 1, 0]);
    }

    /// Automatic holds the sheet the way the pages, upright, come out largest: wide pages
    /// four a sheet on a sheet held sideways, upright ones on a sheet held upright.
    #[test]
    fn automatic_holds_the_sheet_the_way_its_pages_print_largest() {
        let s = several(letter(HP), Orientation::Automatic, 4, PageOrder::Across);
        let wide = sheets(&[(792.0, 612.0); 4], &s);
        assert!(wide[0].sideways);
        assert!(wide[0].pages.iter().all(|(_, p)| p.turns == 1));
        let upright = sheets(&[LETTER; 4], &s);
        assert!(!upright[0].sideways);
        assert!(upright[0].pages.iter().all(|(_, p)| p.turns == 0));
        // Square pages: as the paper comes.
        assert!(!sheets(&[(100.0, 100.0); 4], &s)[0].sideways);
        // Each sheet is held its own way: two upright pages side by side, two wide ones
        // one above the other.
        let s = several(letter(HP), Orientation::Automatic, 2, PageOrder::Across);
        let mixed = sheets(&[LETTER, LETTER, (792.0, 612.0), (792.0, 612.0)], &s);
        assert!(mixed[0].sideways && !mixed[1].sideways);
    }

    /// However many a sheet, held either way, every page is fitted inside the printable
    /// area, in a place of its own, none cut.
    #[test]
    fn several_pages_stay_in_the_printable_area_apart() {
        let sizes = [
            LETTER,
            (792.0, 612.0),
            (595.28, 841.89),
            (2384.0, 3370.0),
            (100.0, 100.0),
            (50.0, 400.0),
        ];
        let uneven = Some([30.0, 12.0, 12.0, 50.0]);
        let papers = [letter(HP), letter(None), letter(uneven)];
        for paper in papers {
            let [t, r, b, l] = paper.margins_or_assumed();
            for o in [
                Orientation::Automatic,
                Orientation::Portrait,
                Orientation::Landscape,
            ] {
                for &n in &PAGES_PER_SHEET[1..] {
                    let s = several(paper.clone(), o, n, PageOrder::DownFromRight);
                    for sheet in sheets(&sizes.repeat(3), &s) {
                        let what = format!("{n} {o:?} {:?}", paper.margins);
                        for (i, (_, p)) in sheet.pages.iter().enumerate() {
                            let [x0, y0, x1, y1] = p.rect;
                            assert!(!p.cut, "{what}");
                            assert!(x0 >= l - 0.01 && y0 >= b - 0.01, "{what}: {p:?}");
                            assert!(x1 <= 612.0 - r + 0.01, "{what}: {p:?}");
                            assert!(y1 <= 792.0 - t + 0.01, "{what}: {p:?}");
                            for (_, q) in &sheet.pages[i + 1..] {
                                let apart = x1 <= q.rect[0] + 0.01
                                    || q.rect[2] <= x0 + 0.01
                                    || y1 <= q.rect[1] + 0.01
                                    || q.rect[3] <= y0 + 0.01;
                                assert!(apart, "{what}: {p:?} {q:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    /// Borders are drawn with more than one page a sheet only.
    #[test]
    fn borders_only_with_several_pages_a_sheet() {
        let mut s = setup(letter(HP), Orientation::Automatic, Scale::ShrinkOversized);
        s.per_sheet.borders = true;
        assert!(!sheets(&[LETTER], &s)[0].borders);
        s.per_sheet.pages = 4;
        assert!(sheets(&[LETTER], &s)[0].borders);
    }

    fn booklet(paper: Paper, binding: Binding, sides: BookletSides) -> PageSetup {
        PageSetup {
            booklet: Some(Booklet { binding, sides }),
            ..setup(paper, Orientation::Automatic, Scale::ShrinkOversized)
        }
    }

    /// Each side's pages as it is held, left to right: up the paper on a sheet held
    /// sideways, across it otherwise.
    fn read_across(plan: &[SheetPlan]) -> Vec<Vec<usize>> {
        let side = |s: &SheetPlan| {
            let mut v: Vec<(f32, usize)> = (s.pages.iter())
                .map(|&(i, p)| match s.sideways {
                    true => (p.rect[1], i),
                    false => (p.rect[0], i),
                })
                .collect();
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            v.into_iter().map(|(_, i)| i).collect()
        };
        plan.iter().map(side).collect()
    }

    /// NEW-05: eight pages bound on the left: the first sheet's front the last page and
    /// the first, its back the second and the second-last; the second sheet the middle
    /// ones, so that the sheets folded and stacked read straight through.
    #[test]
    fn a_booklet_puts_its_pages_in_folding_order() {
        let s = booklet(letter(HP), Binding::Left, BookletSides::Both);
        let plan = sheets(&[LETTER; 8], &s);
        let want: [&[usize]; 4] = [&[7, 0], &[1, 6], &[5, 2], &[3, 4]];
        assert_eq!(read_across(&plan), want);
        assert!(plan.iter().all(|s| s.sideways && !s.borders));
        // Bound on the right, read from the right: each side mirrored.
        let s = booklet(letter(HP), Binding::Right, BookletSides::Both);
        let want: [&[usize]; 4] = [&[0, 7], &[6, 1], &[2, 5], &[4, 3]];
        assert_eq!(read_across(&sheets(&[LETTER; 8], &s)), want);
    }

    /// A booklet's pages are padded with blank places to a whole number of sheets: the
    /// blanks come last, so a side can be left with one page or none.
    #[test]
    fn a_booklet_is_padded_to_whole_sheets_with_blanks_at_the_end() {
        let s = booklet(letter(HP), Binding::Left, BookletSides::Both);
        let five: [&[usize]; 4] = [&[0], &[1], &[2], &[3, 4]];
        assert_eq!(read_across(&sheets(&[LETTER; 5], &s)), five);
        // One page: its front on the right half, the back blank.
        let one = sheets(&[LETTER], &s);
        assert_eq!(one.len(), 2);
        assert_eq!(read_across(&one), [vec![0], vec![]]);
        assert!(one[1].sideways && one[1].size == LETTER);
        let [(0, p)] = one[0].pages[..] else {
            panic!("{one:?}");
        };
        // The right half as held is the paper's top half.
        assert!(p.rect[1] >= 396.0 - 0.01, "{p:?}");
        assert!(sheets(&[], &s).is_empty());
    }

    /// For a printer that prints one side: the fronts alone, then the backs alone, each
    /// in the sheets' order.
    #[test]
    fn a_booklets_fronts_and_backs_print_alone() {
        let fronts = booklet(letter(HP), Binding::Left, BookletSides::Fronts);
        let want: [&[usize]; 2] = [&[7, 0], &[5, 2]];
        assert_eq!(read_across(&sheets(&[LETTER; 8], &fronts)), want);
        let backs = booklet(letter(HP), Binding::Left, BookletSides::Backs);
        let want: [&[usize]; 2] = [&[1, 6], &[3, 4]];
        assert_eq!(read_across(&sheets(&[LETTER; 8], &backs)), want);
        // Counted before the pages' sizes are known, and each side named.
        let b = fronts.booklet.unwrap();
        assert_eq!(b.sides_printed(8), 2);
        assert_eq!(b.side(1), (1, true));
        let b = backs.booklet.unwrap();
        assert_eq!(b.sides_printed(9), 3);
        assert_eq!(b.side(2), (2, false));
        let both = Booklet::default();
        assert_eq!(both.sides_printed(9), 6);
        assert_eq!(both.sides_printed(0), 0);
        assert_eq!((both.side(2), both.side(3)), ((1, true), (1, false)));
    }

    /// Both sides print flipped on the short edge, as the sheets are held sideways; the
    /// fronts or backs alone on one side.
    #[test]
    fn a_booklet_prints_both_sides_flipped_on_the_short_edge() {
        let mut b = Booklet::default();
        assert_eq!(b.sides_used(), Sides::ShortEdge);
        b.sides = BookletSides::Fronts;
        assert_eq!(b.sides_used(), Sides::One);
        b.sides = BookletSides::Backs;
        assert_eq!(b.sides_used(), Sides::One);
    }

    /// Each page is fitted to half the sheet, larger or smaller, the two halves meeting at
    /// the fold in the paper's middle.
    #[test]
    fn a_booklets_pages_meet_at_the_fold() {
        let s = booklet(letter(HP), Binding::Left, BookletSides::Both);
        let plan = sheets(&[LETTER, LETTER, (100.0, 100.0), (2384.0, 3370.0)], &s);
        // Held sideways, 792 by 612, 12 points clear: each half 384 by 588.
        let k = 384.0 / 612.0;
        let [(3, left), (0, right)] = plan[0].pages[..] else {
            panic!("{:?}", plan[0]);
        };
        assert!((right.scale - k).abs() < 1e-5, "{right:?}");
        assert!(right.turns == 1 && !right.cut);
        assert!((right.rect[1] - 396.0).abs() < 0.01, "{right:?}");
        assert!((left.rect[3] - 396.0).abs() < 0.01, "{left:?}");
        // A small page is enlarged to its half.
        let [(1, _), (2, small)] = plan[1].pages[..] else {
            panic!("{:?}", plan[1]);
        };
        assert!((small.scale - 3.84).abs() < 1e-4, "{small:?}");
    }

    /// Uneven margins: the halves are as wide as the narrower side allows, so the fold
    /// stays in the paper's middle.
    #[test]
    fn uneven_margins_keep_a_booklets_fold_in_the_middle() {
        // Held sideways, the paper's bottom is at the left (12 clear), its top at the
        // right (30 clear): each half 366 wide.
        let paper = letter(Some([30.0, 12.0, 12.0, 50.0]));
        let s = booklet(paper, Binding::Left, BookletSides::Both);
        // Two pages: the first on the front's right half, the second on the back's left.
        let plan = sheets(&[LETTER; 2], &s);
        let [(0, right)] = plan[0].pages[..] else {
            panic!("{:?}", plan[0]);
        };
        let [(1, left)] = plan[1].pages[..] else {
            panic!("{:?}", plan[1]);
        };
        assert!((right.rect[1] - 396.0).abs() < 0.01, "{right:?}");
        assert!(right.rect[3] <= 792.0 - 30.0 + 0.01, "{right:?}");
        assert!((left.rect[3] - 396.0).abs() < 0.01, "{left:?}");
        assert!((left.scale - right.scale).abs() < 1e-5);
        assert!(
            (left.rect[1] - (792.0 - right.rect[3])).abs() < 0.01,
            "{left:?}"
        );
    }

    /// A page is never turned in its half: a wide page is fitted upright, as Acrobat does
    /// (bindings along the top are for later).
    #[test]
    fn a_booklets_pages_are_not_turned_in_their_halves() {
        let s = booklet(letter(HP), Binding::Left, BookletSides::Both);
        let plan = sheets(&[(792.0, 612.0); 4], &s);
        for (_, p) in plan.iter().flat_map(|s| &s.pages) {
            assert_eq!(p.turns, 1, "{p:?}");
            assert!((p.scale - 384.0 / 792.0).abs() < 1e-5, "{p:?}");
        }
    }

    /// The booklet sets the holding and the size itself: the orientation, the scale and
    /// pages per sheet change nothing. Paper listed wider than it is tall is held as it
    /// comes.
    #[test]
    fn a_booklet_ignores_orientation_scale_and_pages_per_sheet() {
        let plain = booklet(letter(HP), Binding::Left, BookletSides::Both);
        let other = PageSetup {
            orientation: Orientation::Portrait,
            scale: Scale::ActualSize,
            per_sheet: PerSheet {
                pages: 4,
                order: PageOrder::DownFromRight,
                borders: true,
            },
            ..plain.clone()
        };
        let sizes = [LETTER; 6];
        assert_eq!(sheets(&sizes, &plain), sheets(&sizes, &other));
        let ledger = Paper {
            id: "Ledger".into(),
            label: "Ledger (17 × 11 in)".into(),
            size: (1224.0, 792.0),
            margins: HP,
        };
        let plan = sheets(&sizes, &booklet(ledger, Binding::Left, BookletSides::Both));
        assert!(plan.iter().all(|s| !s.sideways));
        // Six pages: the first sheet's front the first page alone, on the right half.
        let [(0, p)] = plan[0].pages[..] else {
            panic!("{:?}", plan[0]);
        };
        assert!(p.turns == 0 && p.rect[0] >= 612.0 - 0.01, "{p:?}");
        assert!(p.rect[2] <= 1224.0 - 12.0 + 0.01, "{p:?}");
    }
}
