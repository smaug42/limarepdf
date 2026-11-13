//! How large the passes of the rasteriser a page is drawn in can be. hayro 0.9 holds what it
//! draws until a pass ends. Most of that grows with what the pass covers, but some grows with
//! the pass itself:
//!
//! - a soft mask is drawn whole on a mask the size of the pass, a byte a pixel, in every pass,
//!   whatever it masks (the bitmap it's drawn on first is let go before the pass's own is
//!   made);
//! - so is a stencil mask painted with a pattern, in each pass it touches;
//! - a shape filled with a pattern, or a shading painted with `sh`, is sampled into a picture
//!   of all it covers on the pass, 4 bytes a pixel, unless hayro can draw it as a gradient.
//!
//! PDFBox's 2171 paints 840 stencil masks with a pattern: up to 3.3 GB in passes of 2048
//! pixels at 288 dpi, over 6 GB in one. Yet each pass interprets the whole page again, so
//! fewer passes are faster: pdf.js's issue17190, a poster, took 62 s in passes of 2048 pixels
//! at 288 dpi and 5.7 s in one (PB-165). A file with none of these is drawn in one pass, and
//! so is a small drawing ([`UNWEIGHED`]). Otherwise a page is weighed once, with a device that
//! draws nothing (as long as reading the page for a pass), and drawn in the largest passes
//! that take at most [`PASS_BYTES`] more than its bitmap: 2171 in passes of 1024, 2.3 s and
//! 465 MiB, and issue17190 in one, 5.3 s.
//!
//! The same weighing tells how much hayro 0.9's glyph cache takes for a page. hayro keeps a
//! picture, 4 bytes a pixel, of every glyph it fills, at every size and at up to 4 places
//! within a pixel, for as long as its render cache, and has no way to let them go: a page of
//! 20,000 letters each at a size of its own took 553 MB to draw at 96 dpi, against 113 MB with
//! hayro 0.8, and each zoom of a page adds a set. So a page is weighed the first time it's
//! drawn at a scale where the whole page would be larger than [`UNWEIGHED`], even in tiles,
//! and drawn with the cache only while its glyphs and those of the pages drawn with it before
//! would take at most [`GLYPH_BYTES`] (see `render`); a page not weighed is drawn without it.
//! That page now takes 115 MB, in 0.31 s (1.90 s with hayro 0.8). The cache drew 60 files of
//! pdf.js's and PDFBox's tests at 96 dpi in 1.93 s against 2.69 s without; weighing them took
//! 0.30 s, once a page.

use crate::render::{Region, MAX_SIDE, PASS_SIDE};
use hayro::hayro_interpret::encode::EncodedShadingType;
use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::WritingMode;
use hayro::hayro_interpret::pattern::Pattern;
use hayro::hayro_interpret::shading::ShadingType;
use hayro::hayro_interpret::{
    interpret_page, BlendMode, CacheKey, ClipPath, Context, Device, DrawMode, DrawProps, Image,
    ImageDrawProps, InterpreterCache, InterpreterSettings, Paint, SoftMask,
};
use hayro::hayro_syntax::object::dict::keys::{PATTERN_TYPE, SHADING_TYPE, SMASK};
use hayro::hayro_syntax::object::{Dict, MaybeRef, Name, Object};
use hayro::hayro_syntax::page::Page;
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{Affine, BezPath, Rect, Shape};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Most bytes the soft masks, pattern-painted stencil masks and pattern fills of one pass,
/// and its bitmap if it isn't the page's, may take.
pub const PASS_BYTES: f64 = (512u64 << 20) as f64;

/// Most bytes hayro's glyph cache may take, as weighed: the page with most in 60 files of
/// pdf.js's and PDFBox's tests weighs 3.5 MB at 96 dpi and 28 MB at 288.
pub const GLYPH_BYTES: f64 = (128u64 << 20) as f64;

/// Largest side in pixels of a glyph hayro 0.9 keeps a picture of.
const GLYPH_SIDE: f64 = 128.0;

/// A region of at most this many pixels, none of its sides over [`PASS_SIDE`], is drawn in one
/// pass unweighed: thumbnails, the app's tiles, and a Letter or A4 page at 72 dpi, where going
/// through a page first could take as long as drawing it (pdf.js's bug1721218_reduced, 0.56 s
/// to draw, 0.48 s to weigh). PDFBox's 2171, whose 840 stencil masks all cover such a pass,
/// takes 404 MiB at 72 dpi.
const UNWEIGHED: u64 = 1 << 19;

/// What a file holds that a page's passes are weighed for.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Kinds {
    /// Patterns, shadings or soft masks.
    pub any: bool,
    /// Patterns: only then can a stencil mask be painted with one.
    pub patterns: bool,
}

/// Look through every object of `pdf` for patterns, shadings and soft masks.
pub(crate) fn kinds(pdf: &Pdf) -> Kinds {
    let mut k = Kinds::default();
    for o in pdf.objects() {
        match o {
            Object::Dict(d) => look(&d, false, 3, &mut k),
            Object::Stream(s) => look(s.dict(), true, 3, &mut k),
            _ => {}
        }
        if k.patterns {
            break;
        }
    }
    k
}

/// Note in `k` what `d` is, and, `depth` levels down, what the dictionaries written inside it
/// are. `stream`: `d` is a stream's, whose `SMask` is an image's transparency, not a soft mask.
fn look(d: &Dict, stream: bool, depth: u8, k: &mut Kinds) {
    if d.contains_key(PATTERN_TYPE) {
        k.patterns = true;
        k.any = true;
        return;
    }
    let none = |n: Name| n.as_ref() == b"None";
    if d.contains_key(SHADING_TYPE)
        || !stream && d.contains_key(SMASK) && !d.get(SMASK).is_some_and(none)
    {
        k.any = true;
    }
    if depth > 0 {
        for (_, v) in d.entries() {
            if let MaybeRef::NotRef(Object::Dict(c)) = v {
                look(&c, false, depth - 1, k);
            }
        }
    }
}

/// What a page draws whose memory grows with the pass, in the pixels of the page's bitmap at
/// 72 dpi.
#[derive(Debug, Default)]
pub(crate) struct Weight {
    /// Soft masks, told apart as hayro tells them apart.
    masks: usize,
    /// Where stencil masks painted with a pattern are drawn.
    stencils: Vec<Rect>,
    /// Where shapes filled with a pattern or a shading are drawn, those stencil masks among
    /// them.
    fills: Vec<Rect>,
    /// Whether a fill is a tiling pattern, whose tile is held to [`PASS_SIDE`] by passes no
    /// larger (see `unfillable`).
    tiling: bool,
    /// The longer side of each glyph hayro would keep pictures of, smallest first, and how
    /// many it would keep: one for each place within a pixel the glyph is drawn at, so at
    /// most its uses, and at most 4.
    sides: Vec<(f32, u8)>,
    /// The sum of the pictures, of their sides and of their sides' squares, up to each side.
    sums: Vec<(f64, f64, f64)>,
}

impl Weight {
    /// Weigh `page`, drawn with `transform` (page space to its bitmap at 72 dpi). `stencils`:
    /// look at the paint of stencil masks, which decodes them.
    pub(crate) fn of<'a>(
        page: &'a Page<'a>,
        cache: &InterpreterCache<'a>,
        settings: &InterpreterSettings,
        transform: Affine,
        stencils: bool,
    ) -> Weight {
        let (w, h) = page.render_dimensions();
        let full = Rect::new(0.0, 0.0, w as f64, h as f64);
        // What can't be drawn is told when the page is drawn.
        let mut settings = settings.clone();
        settings.warning_sink = Arc::new(|_| {});
        let mut ctx = Context::new(transform, full, cache, page.xref(), settings);
        let mut dev = Weigh {
            page: full,
            stencils,
            seen: HashSet::new(),
            tile: 0,
            glyphs: HashMap::new(),
            outlines: HashMap::new(),
            weight: Weight::default(),
        };
        interpret_page(page, &mut ctx, &mut dev);
        let mut w = dev.weight;
        w.sides.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (mut n, mut one, mut two) = (0.0, 0.0, 0.0);
        w.sums = w
            .sides
            .iter()
            .map(|&(side, copies)| {
                let (side, copies) = (side as f64, copies as f64);
                n += copies;
                one += copies * side;
                two += copies * side * side;
                (n, one, two)
            })
            .collect();
        w
    }

    /// About the most bytes hayro's glyph cache takes for the page drawn at `scale`: the
    /// pictures of each glyph at most [`GLYPH_SIDE`] a side, their bounds rounded out.
    pub(crate) fn glyph_bytes(&self, scale: f32) -> f64 {
        let s = scale as f64;
        let k = self
            .sides
            .partition_point(|&(side, _)| side as f64 * s <= GLYPH_SIDE);
        let Some(&(n, one, two)) = k.checked_sub(1).and_then(|i| self.sums.get(i)) else {
            return 0.0;
        };
        // (side × s + 2)² for each picture, 4 bytes a pixel.
        4.0 * (s * s * two + 4.0 * s * one + 4.0 * n)
    }

    /// The side of the square passes to draw region `r` of a page with this weight in at
    /// `scale`: the region's longer side if it fits in one pass, else the largest multiple of
    /// 256 (the rasteriser's strips) whose passes stay under [`PASS_BYTES`], at least 256.
    pub(crate) fn side(&self, scale: f32, r: Region) -> u32 {
        let whole = r.width.max(r.height);
        let most = if self.tiling { PASS_SIDE } else { MAX_SIDE };
        if whole <= most && self.fits(scale, r, whole) {
            return whole;
        }
        let (mut lo, mut hi) = (1, most.min(whole) / 256);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if self.fits(scale, r, mid * 256) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo * 256
    }

    /// Whether every pass of side `s` drawing `r` at `scale` takes at most [`PASS_BYTES`] more
    /// than the bitmap of `r`.
    fn fits(&self, scale: f32, r: Region, s: u32) -> bool {
        let (nx, ny) = (r.width.div_ceil(s) as usize, r.height.div_ceil(s) as usize);
        let s = s as f64;
        // Per pass, the stencil masks touching it and the bytes of the fills on it, each added
        // to the block of passes it touches in an array of differences.
        let mut stencils = vec![0.0; (nx + 1) * (ny + 1)];
        let mut fills = vec![0.0; (nx + 1) * (ny + 1)];
        let scale = scale as f64;
        let add = |sums: &mut Vec<f64>, at: &Rect, v: f64| {
            let at = Affine::scale(scale).transform_rect_bbox(*at);
            let at = at - hayro::kurbo::Vec2::new(r.x as f64, r.y as f64);
            if at.x1 <= 0.0 || at.y1 <= 0.0 || at.x0 >= r.width as f64 || at.y0 >= r.height as f64 {
                return;
            }
            let cell = |v: f64, n: usize| ((v / s).floor().max(0.0) as usize).min(n - 1);
            let (i0, i1) = (cell(at.x0, nx), cell(at.x1, nx) + 1);
            let (j0, j1) = (cell(at.y0, ny), cell(at.y1, ny) + 1);
            let w = nx + 1;
            sums[j0 * w + i0] += v;
            sums[j0 * w + i1] -= v;
            sums[j1 * w + i0] -= v;
            sums[j1 * w + i1] += v;
        };
        for at in &self.stencils {
            add(&mut stencils, at, 1.0);
        }
        for at in &self.fills {
            add(&mut fills, at, 4.0 * (at.area() * scale * scale).min(s * s));
        }
        let w = nx + 1;
        for sums in [&mut stencils, &mut fills] {
            for j in 0..=ny {
                for i in 0..=nx {
                    let left = if i > 0 { sums[j * w + i - 1] } else { 0.0 };
                    let up = if j > 0 { sums[(j - 1) * w + i] } else { 0.0 };
                    let both = if i > 0 && j > 0 {
                        sums[(j - 1) * w + i - 1]
                    } else {
                        0.0
                    };
                    sums[j * w + i] += left + up - both;
                }
            }
        }
        let most = PASS_BYTES;
        #[cfg(any(test, feature = "test-seams"))]
        let most = crate::render::pass_seam::bytes().unwrap_or(most);
        // A mask is drawn on a bitmap of the pass's size, let go before the pass's own is
        // made; in one pass, that one is the page's.
        let bitmap = if nx * ny > 1 { 4.0 } else { 0.0 };
        (0..ny).all(|j| {
            (0..nx).all(|i| {
                let pw = (r.width as f64 - i as f64 * s).min(s);
                let ph = (r.height as f64 - j as f64 * s).min(s);
                let per_pixel = bitmap + self.masks as f64 + stencils[j * w + i];
                pw * ph * per_pixel + fills[j * w + i] <= most
            })
        })
    }

    /// Whether a region `r` is drawn without weighing its page.
    pub(crate) fn unweighed(r: Region) -> bool {
        r.width <= PASS_SIDE
            && r.height <= PASS_SIDE
            && r.width as u64 * r.height as u64 <= UNWEIGHED
    }
}

/// A device that draws nothing and weighs what is drawn.
struct Weigh {
    page: Rect,
    stencils: bool,
    /// Soft masks and tiling patterns gone through.
    seen: HashSet<u128>,
    /// How many tiling patterns' cells in the device is, drawn on bitmaps of their own: only
    /// their glyphs are weighed.
    tile: u32,
    /// Each glyph and size hayro keeps pictures of, as it tells them apart, and where it is in
    /// `weight.sides`.
    glyphs: HashMap<(u128, [u64; 4]), usize>,
    /// The bounds of each glyph's outline.
    outlines: HashMap<u128, Rect>,
    weight: Weight,
}

impl Weigh {
    fn mask<'a>(&mut self, mask: Option<&SoftMask<'a>>) {
        let Some(m) = mask else {
            return;
        };
        if self.seen.insert(m.cache_key()) {
            if self.tile == 0 {
                self.weight.masks += 1;
            }
            m.interpret(self);
        }
    }

    /// Note the glyphs of `run` hayro keeps pictures of: those of a run of outlines only,
    /// filled with a colour, written across.
    fn glyphs(&mut self, run: &GlyphRun<'_, '_>, props: &DrawProps, mode: &DrawMode) {
        let filled = matches!(mode, DrawMode::Fill(_) | DrawMode::FillAndStroke(..));
        if !filled
            || !matches!(props.paint, Paint::Color(_))
            || run.writing_mode() != WritingMode::Horizontal
            || !run
                .glyphs()
                .iter()
                .all(|g| matches!(&**g, Glyph::Outline(_)))
        {
            return;
        }
        for g in run.glyphs() {
            let Glyph::Outline(o) = &**g else {
                continue;
            };
            let id = o.identifier().cache_key();
            let [a, b, c, d, _, _] = (props.transform * g.transform()).as_coeffs();
            let key = (id, [a, b, c, d].map(f64::to_bits));
            if let Some(&i) = self.glyphs.get(&key) {
                let copies = &mut self.weight.sides[i].1;
                *copies = (*copies + 1).min(4);
                continue;
            }
            let bounds = *self
                .outlines
                .entry(id)
                .or_insert_with(|| o.outline().bounding_box());
            let at = Affine::new([a, b, c, d, 0.0, 0.0]).transform_rect_bbox(bounds);
            self.glyphs.insert(key, self.weight.sides.len());
            self.weight
                .sides
                .push((at.width().max(at.height()) as f32, 1));
        }
    }

    fn fill(&mut self, paint: &Paint, at: Rect) {
        let Paint::Pattern(p) = paint else {
            return;
        };
        if let Pattern::Tiling(t) = &**p {
            if self.seen.insert(t.cache_key()) {
                self.tile += 1;
                t.interpret(self, t.matrix, false);
                self.tile -= 1;
            }
        }
        let at = at.intersect(self.page);
        if self.tile > 0 || at.area() <= 0.0 {
            return;
        }
        match &**p {
            Pattern::Tiling(_) => self.weight.tiling = true,
            Pattern::Shading(s) => {
                // An axial or radial shading hayro can draw as a gradient isn't sampled. Only
                // those are encoded: encoding a mesh samples it, which takes as long as drawing.
                // Whether one can be is told by its shape alone; at any tolerance, its stops
                // take a few samples, not up to 1,024.
                if let ShadingType::RadialAxial { .. } = &*s.shading.shading_type {
                    let e = s.encode();
                    if let EncodedShadingType::RadialAxial(g) = &e.shading_type {
                        if g.as_svg_gradient(&e, at, f32::INFINITY).is_some() {
                            return;
                        }
                    }
                }
            }
        }
        self.weight.fills.push(at);
    }
}

/// How far a stroke drawn with `props` reaches past its path, in the page's pixels.
fn reach(props: &DrawProps, mode: &DrawMode) -> f64 {
    match mode {
        DrawMode::Stroke(s) | DrawMode::FillAndStroke(_, s) => {
            s.line_width as f64 * props.transform.determinant().abs().sqrt()
        }
        _ => 0.0,
    }
}

impl<'a> Device<'a> for Weigh {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, mode: &DrawMode) {
        self.mask(props.soft_mask.as_ref());
        if let DrawMode::Invisible = mode {
            return;
        }
        let d = reach(&props, mode);
        let at = props.transform.transform_rect_bbox(path.bounding_box());
        self.fill(&props.paint, at.inflate(d, d));
    }

    fn push_clip_path(&mut self, _: &ClipPath) {}

    fn push_transparency_group(&mut self, _: f32, mask: Option<SoftMask<'a>>, _: BlendMode) {
        self.mask(mask.as_ref());
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, mode: &DrawMode) {
        self.mask(props.soft_mask.as_ref());
        if let DrawMode::Invisible = mode {
            return;
        }
        self.glyphs(run, &props, mode);
        let pattern = matches!(props.paint, Paint::Pattern(_));
        let mut at = Rect::ZERO;
        for g in run.glyphs() {
            match &**g {
                Glyph::Type3(t) => t.interpret(self, props.transform, g.transform(), &props.paint),
                Glyph::Outline(o) if pattern => {
                    let b = (props.transform * g.transform())
                        .transform_rect_bbox(o.outline().bounding_box());
                    at = if at.area() > 0.0 { at.union(b) } else { b };
                }
                Glyph::Outline(_) => {}
            }
        }
        let d = reach(&props, mode);
        self.fill(&props.paint, at.inflate(d, d));
    }

    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        self.mask(props.soft_mask.as_ref());
        let Image::Stencil(s) = &image else {
            return;
        };
        if !self.stencils || self.tile > 0 {
            return;
        }
        let r = Rect::new(0.0, 0.0, image.width() as f64, image.height() as f64);
        let at = props.transform.transform_rect_bbox(r).intersect(self.page);
        if at.area() <= 0.0 {
            return;
        }
        let mut paint = None;
        s.with_stencil(|_, p| paint = Some(p.clone()), Some((1, 1)));
        if let Some(p @ Paint::Pattern(_)) = paint {
            self.weight.stencils.push(at);
            self.fill(&p, at);
        }
    }

    fn pop_clip(&mut self) {}

    fn pop_transparency_group(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(width: u32, height: u32) -> Region {
        Region {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    #[test]
    fn small_regions_are_drawn_unweighed() {
        // A Letter and an A4 page at 72 dpi, and a tile.
        assert!(Weight::unweighed(region(612, 792)));
        assert!(Weight::unweighed(region(595, 842)));
        assert!(Weight::unweighed(region(512, 512)));
        assert!(!Weight::unweighed(region(850, 1100)));
        // Few pixels, but a side longer than a tiling pattern's passes.
        assert!(!Weight::unweighed(region(100, PASS_SIDE + 1)));
    }

    #[test]
    fn a_tiling_pattern_holds_passes_to_pass_side() {
        let tiled = Weight {
            fills: vec![Rect::new(0.0, 0.0, 10.0, 10.0)],
            tiling: true,
            ..Weight::default()
        };
        assert_eq!(tiled.side(1.0, region(10_000, 100)), PASS_SIDE);
        assert_eq!(Weight::default().side(1.0, region(10_000, 100)), 10_000);
    }

    #[test]
    fn passes_shrink_with_soft_masks_and_stencils() {
        // A pixel of a piece takes 4 + 16 bytes: at most 5181 pixels a side in 512 MiB.
        let masked = Weight {
            masks: 16,
            ..Weight::default()
        };
        assert_eq!(masked.side(1.0, region(8192, 8192)), 5120);
        // In one pass, 16: 5792 pixels a side.
        assert_eq!(masked.side(1.0, region(5792, 5792)), 5792);
        assert_eq!(masked.side(1.0, region(5793, 5793)), 5120);
        // 100 stencil masks in a corner: 4 + 100 bytes a pixel there, at most 2272 a side.
        let stencilled = Weight {
            stencils: vec![Rect::new(0.0, 0.0, 100.0, 100.0); 100],
            ..Weight::default()
        };
        assert_eq!(stencilled.side(1.0, region(8192, 8192)), 2048);
        // Away from them, the page is drawn in one pass.
        let away = Region {
            x: 4096,
            ..region(4096, 4096)
        };
        assert_eq!(stencilled.side(1.0, away), 4096);
    }
}
