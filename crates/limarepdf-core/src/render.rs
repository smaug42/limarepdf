//! Page rasterisation on top of `hayro` (pure Rust, CPU only).
//!
//! `hayro`'s render cache is not `Send`, so a [`Renderer`] lives on one thread.
//! For parallel work, create one renderer per worker (see [`render_pages_parallel`]).

use crate::installed_fonts::display_name;
pub use crate::installed_fonts::{InstalledFonts, Language};
use crate::page_colours::{self, PageColours};
use crate::passes::{self, Kinds, Weight};
use crate::{Pdf, Result};
use anyhow::{anyhow, Context};
use hayro::hayro_interpret::font::FontQuery;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{InterpreterCache, InterpreterSettings, InterpreterWarning};
use hayro::hayro_syntax::page::Page;
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf as HPdf};
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::{TRANSPARENT, WHITE};
use hayro::vello_cpu::peniko::ImageAlphaType;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{render_into, RenderCache, RenderSettings};
use image::codecs::jpeg::{JpegEncoder, PixelDensity};
use rayon::prelude::*;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// An opaque RGBA8 bitmap (row-major, 4 bytes per pixel).
#[derive(Clone)]
pub struct Bitmap {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Largest scale for drawing part of a page with [`Renderer::render_region`]: 6400% zoom
/// at 96 dpi on a screen with three pixels per point.
pub const MAX_REGION_SCALE: f32 = 256.0;

/// Most pixels a page's bitmap may have: 1 GiB of RGBA, enough for A3 at [`MAX_DPI`].
/// The file sets the page size, and the rasteriser allocates the whole bitmap at once
/// (up to [`MAX_SIDE`] pixels a side, 17 GB), so a failed allocation would close the app.
pub const MAX_PIXELS: u64 = 1 << 28;

/// Most pixels a side of one pass of the rasteriser on a page with a tiling pattern. hayro
/// draws a pattern's tile on a bitmap of its own, as much of the pattern as the pass shows,
/// up to a step, so this bounds the tile, and `unfillable` can tell at any zoom a pattern
/// whose tile would be too large. Other pages are drawn in one pass unless their soft masks
/// and the like would take too much memory (see `passes`). Pieces are a multiple of the
/// rasteriser's 256-pixel strips, so that they line up with one pass: the bitmap comes out
/// the same, but for an edge pixel here and there a level apart, as the app's tiles are, its
/// rounding from another origin.
pub const PASS_SIDE: u32 = 2048;

/// Most pixels a side of a page's bitmap may have (M-18): 255 of the rasteriser's 256-pixel
/// strips. Past 65,408 pixels wide its coverage counts overflow and it panics; a bitmap may
/// be drawn in pieces, but is held to the same side, so that what a page can be exported at
/// doesn't hang on which way it is turned.
pub const MAX_SIDE: u32 = 255 * 256;

/// An error when a bitmap `width` × `height` of page `index` would pass [`MAX_SIDE`] or
/// [`MAX_PIXELS`].
fn check_pixels(index: usize, width: u64, height: u64) -> Result<()> {
    let side = MAX_SIDE as u64;
    if width > side || height > side || width.saturating_mul(height) > MAX_PIXELS {
        anyhow::bail!(
            "page {} is too large to draw at this size ({width} × {height} pixels)",
            index + 1
        );
    }
    Ok(())
}

/// The size of `page`'s whole bitmap at `scale` (as [`Renderer::render`] clamps it), as
/// `hayro::render` works it out, or an error past [`check_pixels`]'s limits.
fn bitmap_size(page: &Page, index: usize, scale: f32) -> Result<(u32, u32)> {
    let scale = scale.clamp(0.05, 16.0);
    let (w, h) = page.render_dimensions();
    // Saturating, and 0 for a size that isn't a number.
    let (width, height) = ((w * scale) as u64, (h * scale) as u64);
    check_pixels(index, width, height)?;
    Ok((width as u32, height as u32))
}

/// A rectangle of the bitmap a whole page would be at some scale: left, top, width and
/// height in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    /// The part of the region inside a page bitmap `width` × `height`, at most
    /// [`MAX_SIDE`] a side; `None` when nothing is left.
    fn within(self, width: u32, height: u32) -> Option<Region> {
        let w = self.width.min(width.saturating_sub(self.x)).min(MAX_SIDE);
        let h = self.height.min(height.saturating_sub(self.y)).min(MAX_SIDE);
        (w > 0 && h > 0).then_some(Region {
            width: w,
            height: h,
            ..self
        })
    }
}

/// Something on a page the renderer couldn't draw as the file asks, by font name where
/// the file gives one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Missing {
    /// A font the renderer can't use, or one that isn't embedded with no installed font
    /// at all to draw it, drawn with a standard font that may lack its characters.
    Font(Option<String>),
    /// Text in a language no installed font has: a CID font that isn't embedded, drawn
    /// with a Latin standard font that lacks its characters.
    Language(Language, Option<String>),
    /// A font that isn't embedded or installed, drawn with an installed font like it
    /// (the second name), so the text reads but looks different.
    StandIn(Option<String>, String),
    /// A picture that couldn't be decoded, or one left out of what is drawn: one its data
    /// can't fill, or a tiling pattern the renderer can't tile (`unfillable.rs`).
    Picture,
}

impl Missing {
    /// Whether something is lost (left out or drawn wrongly), rather than drawn with a
    /// font like the one asked for.
    pub fn lost(&self) -> bool {
        !matches!(self, Missing::StandIn(..))
    }

    /// The thing, for a list: "a font", "Japanese text", "Tahoma (drawn with Liberation
    /// Sans)".
    pub fn describe(&self) -> String {
        match self {
            Missing::Font(Some(f)) => format!("the font {f}"),
            Missing::Font(None) => "a font".into(),
            Missing::Language(l, _) => format!("{} text", l.name()),
            Missing::StandIn(f, with) => format!(
                "{} (drawn with {with})",
                f.as_deref().unwrap_or("a font with no name")
            ),
            Missing::Picture => "a picture".into(),
        }
    }
}

/// What couldn't be drawn as the file asks, in words: the app's notices and the CLI's
/// warning (PB-82).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Notices {
    /// Parts left out or drawn wrongly, other than fonts.
    pub parts: Option<String>,
    /// Fonts that aren't installed, saying what to install.
    pub fonts: Option<String>,
}

/// The [`Notices`] for `what`. Font names, which come from the file, are shown as
/// [`crate::display_line`] shows them.
pub fn notices(what: &BTreeSet<Missing>) -> Notices {
    let shown = |f: &str| crate::display_line(f, 60);
    let mut stand_ins = BTreeSet::new();
    let mut unnamed_stand_ins = false;
    let mut languages = BTreeSet::new();
    let mut unusable = BTreeSet::new();
    let mut parts = Vec::new();
    for m in what {
        match m {
            Missing::StandIn(Some(f), _) => {
                stand_ins.insert(shown(f));
            }
            Missing::StandIn(None, _) => unnamed_stand_ins = true,
            Missing::Language(l, _) => {
                languages.insert(*l);
            }
            Missing::Font(Some(f)) => {
                unusable.insert(shown(f));
            }
            Missing::Font(None) | Missing::Picture => parts.push(m.describe()),
        }
    }
    let list = crate::short_list;
    let fonts_named = |names: &[&str]| {
        if names.len() == 1 {
            format!("The font {}", names[0])
        } else {
            format!("The fonts {}", list(names))
        }
    };

    let mut fonts = Vec::new();
    if !languages.is_empty() {
        let names: Vec<&str> = languages.iter().map(|l| l.name()).collect();
        let suggest: Vec<&str> = languages.iter().map(|l| l.suggestion()).collect();
        fonts.push(if languages.len() == 1 {
            format!(
                "This document has {0} text, but no {0} font is installed, so that text can't \
                 be shown. Install a {0} font, such as {1}, to read it.",
                names[0], suggest[0]
            )
        } else {
            format!(
                "This document has {} text, but no font for them is installed, so that text \
                 can't be shown. Install fonts for them, such as {}, to read it.",
                list(&names),
                list(&suggest)
            )
        });
    }
    if !unusable.is_empty() {
        let names: Vec<&str> = unusable.iter().map(String::as_str).collect();
        fonts.push(format!(
            "{} {} installed and no font could stand in, so {} text may show wrong \
             characters. Install {} to read it.",
            fonts_named(&names),
            if names.len() == 1 { "isn't" } else { "aren't" },
            if names.len() == 1 { "its" } else { "their" },
            if names.len() == 1 { "it" } else { "them" },
        ));
    }
    if !stand_ins.is_empty() {
        let names: Vec<&str> = stand_ins.iter().map(String::as_str).collect();
        fonts.push(if names.len() == 1 {
            format!(
                "The font {0} isn't installed, so a similar font is shown in its place and \
                 some text may not look right. Install {0} to see the document as it was made.",
                names[0]
            )
        } else {
            format!(
                "{} aren't installed, so similar fonts are shown in their place and some text \
                 may not look right. Install them to see the document as it was made.",
                fonts_named(&names)
            )
        });
    } else if unnamed_stand_ins {
        fonts.push(
            "Some fonts this document uses aren't installed, so similar fonts are shown in \
             their place and some text may not look right."
                .into(),
        );
    }

    Notices {
        parts: (!parts.is_empty()).then(|| {
            format!(
                "Parts of this document couldn't be drawn ({}), so some pages may look \
                 incomplete.",
                parts.join(", ")
            )
        }),
        fonts: (!fonts.is_empty()).then(|| fonts.join(" ")),
    }
}

/// A test seam: drawing panics in a program a test runs with [`panic_seam::VAR`] set, so the
/// CLI's tests reach its panic guard (no file is known to make hayro 0.9.0 panic). Built only
/// for tests, as [`crate::sync_seam`] is.
#[cfg(any(test, feature = "test-seams"))]
pub mod panic_seam {
    /// Set, every page drawn panics, as hayro 0.8.0 did on PDFBox 2363 at 150 dpi.
    pub const VAR: &str = "LIMAREPDF_TEST_DRAW_PANICS";

    pub(super) fn here() {
        if std::env::var_os(VAR).is_some() {
            panic!("a test made drawing panic");
        }
    }
}

/// A test seam: how large the pieces a page is drawn in are, and the memory they're held to,
/// on the thread drawing, so that a test reaches pieces without a page that needs them.
#[cfg(any(test, feature = "test-seams"))]
pub mod pass_seam {
    use std::cell::Cell;

    thread_local! {
        static SIDE: Cell<Option<u32>> = const { Cell::new(None) };
        static BYTES: Cell<Option<f64>> = const { Cell::new(None) };
    }

    /// The side of the pieces the last page or region drawn on this thread was drawn in.
    pub fn last_side() -> Option<u32> {
        SIDE.get()
    }

    /// Hold the pieces drawn on this thread to `bytes`, not `passes::PASS_BYTES`.
    pub fn hold_to(bytes: f64) {
        BYTES.set(Some(bytes));
    }

    pub(super) fn note(side: u32) {
        SIDE.set(Some(side));
    }

    pub(crate) fn bytes() -> Option<f64> {
        BYTES.get()
    }
}

/// A test seam: the bytes hayro's glyph cache is held to on the thread drawing, so that a test
/// reaches the limit with a page that draws quickly.
#[cfg(any(test, feature = "test-seams"))]
pub mod glyph_seam {
    use std::cell::Cell;

    thread_local! {
        static BYTES: Cell<Option<f64>> = const { Cell::new(None) };
    }

    /// Hold the glyph cache of renderers drawing on this thread to `bytes`, not
    /// `passes::GLYPH_BYTES`.
    pub fn hold_to(bytes: f64) {
        BYTES.set(Some(bytes));
    }

    pub(crate) fn bytes() -> Option<f64> {
        BYTES.get()
    }
}

pub struct Renderer<'a> {
    pdf: &'a HPdf,
    /// Let go, for a new one, when its glyphs would take too much (see `passes`).
    cache: RefCell<RenderCache<'a>>,
    /// The pages and scales drawn with [`Renderer::cache`]'s glyph cache, as page index and
    /// scale's bits, and the bytes their glyphs may take.
    glyphs: RefCell<(HashSet<(usize, u32)>, f64)>,
    /// For finding pictures in [`Renderer::render_in`]; hayro keeps its own cache's
    /// interpreter half private.
    boxes: InterpreterCache<'a>,
    settings: InterpreterSettings,
    /// What hayro reported it couldn't draw, since [`Renderer::take_missing`] was last
    /// called.
    missing: Arc<Mutex<BTreeSet<Missing>>>,
    /// What the file holds that pages' passes are weighed for, once looked for.
    kinds: OnceLock<Kinds>,
    /// Pages' weights, once weighed.
    weights: Mutex<HashMap<usize, Arc<Weight>>>,
}

impl<'a> Renderer<'a> {
    /// A renderer that draws fonts that aren't embedded with installed fonts, where
    /// hayro asks for them (see [`InstalledFonts`]).
    pub fn new(pdf: &'a HPdf) -> Self {
        Self::build(pdf, None)
    }

    /// A renderer that looks only in `fonts` for fonts that aren't embedded.
    pub fn with_fonts(pdf: &'a HPdf, fonts: Arc<InstalledFonts>) -> Self {
        Self::build(pdf, Some(fonts))
    }

    fn build(pdf: &'a HPdf, fonts: Option<Arc<InstalledFonts>>) -> Self {
        let missing = Arc::new(Mutex::new(BTreeSet::new()));
        let mut settings = InterpreterSettings::default();
        let note = missing.clone();
        settings.warning_sink = Arc::new(move |w| {
            let m = match w {
                InterpreterWarning::ImageDecodeFailure => Missing::Picture,
                InterpreterWarning::UnsupportedFont => Missing::Font(None),
                // Also sent for a check box that is off and has only an appearance for
                // on, which is normal and draws nothing.
                _ => return,
            };
            note.lock().unwrap_or_else(|e| e.into_inner()).insert(m);
        });
        // hayro asks for a fallback only for CID fonts that aren't embedded. Without an
        // installed font, the default answer is a standard font, which lacks Chinese,
        // Japanese and Korean characters and draws Latin CID text with wrong characters.
        let resolve = settings.font_resolver.clone();
        let note = missing.clone();
        settings.font_resolver = Arc::new(move |q| {
            let FontQuery::Fallback(f) = q else {
                return resolve(q);
            };
            let found = match &fonts {
                Some(fonts) => fonts.find(f),
                None => InstalledFonts::system().find(f),
            };
            let name = display_name(f);
            let m = match (&found, Language::of(f)) {
                (Some(found), _) => found
                    .stand_in
                    .clone()
                    .map(|with| Missing::StandIn(name, with)),
                (None, Some(l)) => Some(Missing::Language(l, name)),
                (None, None) => Some(Missing::Font(name)),
            };
            if let Some(m) = m {
                note.lock().unwrap_or_else(|e| e.into_inner()).insert(m);
            }
            match found {
                Some(found) => Some((found.data, found.index)),
                None => resolve(q),
            }
        });
        Self {
            pdf,
            cache: RefCell::new(RenderCache::new()),
            glyphs: RefCell::new((HashSet::new(), 0.0)),
            boxes: InterpreterCache::new(),
            settings,
            missing,
            kinds: OnceLock::new(),
            weights: Mutex::new(HashMap::new()),
        }
    }

    /// What couldn't be drawn in the renders since the last call. hayro keeps the fonts it
    /// has read from page to page, so a font is reported on the first page that uses it, and
    /// again only after the renderer lets its cache go for its glyphs (see `passes`); a
    /// picture is decoded each time it's drawn, and reported each time.
    pub fn take_missing(&self) -> BTreeSet<Missing> {
        std::mem::take(&mut *self.missing.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    fn page(&self, index: usize) -> Result<&'a Page<'a>> {
        let pdf: &'a HPdf = self.pdf;
        pdf.pages()
            .get(index)
            .ok_or_else(|| anyhow!("no page {}", index + 1))
    }

    /// Displayed page size in points.
    pub fn page_size(&self, index: usize) -> Result<(f32, f32)> {
        Ok(self.page(index)?.render_dimensions())
    }

    /// Render a page at `scale` (1.0 = 72 dpi, 2.0 = 144 dpi). A bitmap of more than
    /// [`MAX_PIXELS`] is an error.
    pub fn render(&self, index: usize, scale: f32) -> Result<Bitmap> {
        self.render_unless(index, scale, &|| false)?
            .ok_or_else(|| anyhow!("drawing page {} was stopped", index + 1))
    }

    /// [`Renderer::render`], drawn in one pass or, on a page whose passes would take too
    /// much memory, in pieces (see `passes`). `stop` is asked before each piece, and `None`
    /// is returned once it says to stop.
    pub fn render_unless(
        &self,
        index: usize,
        scale: f32,
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<Bitmap>> {
        self.render_whole(index, scale, false, stop)
    }

    /// Page `index` at 72 dpi on a clear backdrop, for a picture drawn alone on a page of its
    /// size: the bitmap keeps the picture's transparency, its colours not multiplied by it.
    pub(crate) fn render_clear(&self, index: usize) -> Result<Bitmap> {
        let mut b = self
            .render_whole(index, 1.0, true, &|| false)?
            .ok_or_else(|| anyhow!("drawing page {} was stopped", index + 1))?;
        for px in b.rgba.as_chunks_mut::<4>().0 {
            let a = px[3] as u32;
            if a != 0 && a != 255 {
                for c in &mut px[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        Ok(b)
    }

    /// [`Renderer::render_unless`], on white or, `clear`, on a clear backdrop.
    fn render_whole(
        &self,
        index: usize,
        scale: f32,
        clear: bool,
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<Bitmap>> {
        let page = self.page(index)?;
        let (width, height) = bitmap_size(page, index, scale)?;
        let scale = scale.clamp(0.05, 16.0);
        let whole = Region {
            x: 0,
            y: 0,
            width,
            height,
        };
        Ok(self.draw_pieces(index, page, scale, whole, clear, stop))
    }

    /// The side of the square pieces to draw region `r` of page `index` in at `scale`, and
    /// the page's weight, unless it isn't weighed: it's weighed the first time the whole page
    /// at the scale it's drawn at, of which `r` is part, would be larger than a region drawn
    /// unweighed, as the app's tiles of a page zoomed in are.
    fn pass_side(
        &self,
        index: usize,
        page: &'a Page<'a>,
        scale: f32,
        r: Region,
    ) -> (u32, Option<Arc<Weight>>) {
        let whole = r.width.max(r.height);
        let kinds = *self.kinds.get_or_init(|| passes::kinds(self.pdf));
        let known = self.lock_weights().get(&index).cloned();
        let (w, h) = page.render_dimensions();
        let (width, height) = ((w * scale) as u32, (h * scale) as u32);
        let all = Region {
            x: 0,
            y: 0,
            width,
            height,
        };
        if known.is_none() && Weight::unweighed(all) {
            return (whole, None);
        }
        let weight = known.unwrap_or_else(|| {
            let transform = page_transform(page, 1.0, 0, 0);
            let w = Weight::of(page, &self.boxes, &self.settings, transform, kinds.patterns);
            let w = Arc::new(w);
            self.lock_weights().insert(index, w.clone());
            w
        });
        let side = if kinds.any && !Weight::unweighed(r) {
            weight.side(scale, r)
        } else {
            whole
        };
        (side, Some(weight))
    }

    /// Whether to draw page `index`, of `weight`, at `scale` with hayro's glyph cache: only
    /// if it's weighed, and its glyphs take at most [`passes::GLYPH_BYTES`]. The cache is let
    /// go first if they and those of the pages drawn with it before would take more.
    fn glyph_cache(&self, index: usize, weight: Option<&Weight>, scale: f32) -> bool {
        let Some(weight) = weight else {
            return false;
        };
        let most = passes::GLYPH_BYTES;
        #[cfg(any(test, feature = "test-seams"))]
        let most = glyph_seam::bytes().unwrap_or(most);
        let bytes = weight.glyph_bytes(scale);
        if bytes > most {
            return false;
        }
        let mut glyphs = self.glyphs.borrow_mut();
        let (drawn, taken) = &mut *glyphs;
        if !drawn.contains(&(index, scale.to_bits())) {
            if *taken + bytes > most {
                *self.cache.borrow_mut() = RenderCache::new();
                drawn.clear();
                *taken = 0.0;
            }
            drawn.insert((index, scale.to_bits()));
            *taken += bytes;
        }
        true
    }

    fn lock_weights(&self) -> std::sync::MutexGuard<'_, HashMap<usize, Arc<Weight>>> {
        self.weights.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// [`Renderer::draw`] region `r` of page `index`, in pieces if it must be (see
    /// [`Renderer::pass_side`]), asking `stop` before each; `None` once it says to stop.
    fn draw_pieces(
        &self,
        index: usize,
        page: &'a Page<'a>,
        scale: f32,
        r: Region,
        clear: bool,
        stop: &dyn Fn() -> bool,
    ) -> Option<Bitmap> {
        let (width, height) = (r.width as usize, r.height as usize);
        let (side, weight) = self.pass_side(index, page, scale, r);
        #[cfg(any(test, feature = "test-seams"))]
        pass_seam::note(side);
        let glyphs = self.glyph_cache(index, weight.as_deref(), scale);
        if side >= r.width && side >= r.height {
            if stop() {
                return None;
            }
            let rgba = self
                .draw(page, scale, r, clear, glyphs)
                .take_rgba8(ImageAlphaType::AlphaPremultiplied);
            return Some(Bitmap {
                width,
                height,
                rgba,
            });
        }
        let mut rgba = vec![0; width * height * 4];
        for y in (0..r.height).step_by(side as usize) {
            for x in (0..r.width).step_by(side as usize) {
                if stop() {
                    return None;
                }
                let piece = Region {
                    x: r.x + x,
                    y: r.y + y,
                    width: side.min(r.width - x),
                    height: side.min(r.height - y),
                };
                let pm = self.draw(page, scale, piece, clear, glyphs);
                let line = piece.width as usize * 4;
                for (i, row) in pm.data_as_u8_slice().chunks_exact(line).enumerate() {
                    let at = ((y as usize + i) * width + x as usize) * 4;
                    rgba[at..at + line].copy_from_slice(row);
                }
            }
        }
        Some(Bitmap {
            width,
            height,
            rgba,
        })
    }

    /// Render a page at `scale` for display in a colour mode, as
    /// [`Renderer::render_unless`]. With `keep_pictures`, raster pictures keep their own
    /// colours. Never used for printing or export.
    pub fn render_in(
        &self,
        index: usize,
        scale: f32,
        mode: PageColours,
        keep_pictures: bool,
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<Bitmap>> {
        let Some(mut b) = self.render_unless(index, scale, stop)? else {
            return Ok(None);
        };
        let transform = page_transform(self.page(index)?, scale.clamp(0.05, 16.0), 0, 0);
        self.recolour(index, &mut b, transform, mode, keep_pictures);
        Ok(Some(b))
    }

    /// Render the part `region` of a page drawn at `scale` (at most
    /// [`MAX_REGION_SCALE`]), for showing a page in tiles at high zoom. The region is cut
    /// to the page; one wholly outside it is an error.
    pub fn render_region(&self, index: usize, scale: f32, region: Region) -> Result<Bitmap> {
        let page = self.page(index)?;
        let scale = scale.clamp(0.05, MAX_REGION_SCALE);
        // The whole page's bitmap size, as `render` works it out.
        let (w, h) = page.render_dimensions();
        let r = region
            .within((w * scale) as u32, (h * scale) as u32)
            .ok_or_else(|| anyhow!("the region is outside page {}", index + 1))?;
        check_pixels(index, r.width as u64, r.height as u64)?;
        self.draw_pieces(index, page, scale, r, false, &|| false)
            .ok_or_else(|| anyhow!("drawing page {} was stopped", index + 1))
    }

    /// Draw the part `r` (within the bitmap, at most [`MAX_SIDE`] a side) of `page` drawn
    /// at `scale`, on white, as `hayro::render` draws a whole page, or, `clear`, on a clear
    /// backdrop (the pixels' colours multiplied by their alpha). `glyphs`: with hayro's
    /// glyph cache.
    fn draw(&self, page: &'a Page<'a>, scale: f32, r: Region, clear: bool, glyphs: bool) -> Pixmap {
        debug_assert!(r.width <= MAX_SIDE && r.height <= MAX_SIDE, "{r:?}");
        #[cfg(any(test, feature = "test-seams"))]
        panic_seam::here();
        let mut ctx = RenderContext::new(r.width as u16, r.height as u16);
        // Text isn't hinted. hayro 0.9 hints only filled text, so a stroked or clipping
        // outline of a glyph falls off its fill by up to a pixel; poppler and pdf.js don't
        // hint, so small text was drawn unlike theirs; and a hinted glyph's outline is kept
        // at every size it's drawn at.
        let settings = RenderSettings {
            hinting: false,
            glyph_caching: glyphs,
            ..Default::default()
        };
        render_into(
            page,
            &self.cache.borrow(),
            &self.settings,
            &settings,
            &mut ctx,
            page_transform(page, scale, r.x, r.y),
        );
        ctx.flush();
        let mut pm = Pixmap::new(ctx.width(), ctx.height());
        ctx.render_with(
            &mut pm,
            &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(if clear { TRANSPARENT } else { WHITE }),
                ..Default::default()
            },
        );
        pm
    }

    /// [`Renderer::render_region`] for display in a colour mode, as
    /// [`Renderer::render_in`].
    pub fn render_region_in(
        &self,
        index: usize,
        scale: f32,
        region: Region,
        mode: PageColours,
        keep_pictures: bool,
    ) -> Result<Bitmap> {
        let mut b = self.render_region(index, scale, region)?;
        let scale = scale.clamp(0.05, MAX_REGION_SCALE);
        let transform = page_transform(self.page(index)?, scale, region.x, region.y);
        self.recolour(index, &mut b, transform, mode, keep_pictures);
        Ok(b)
    }

    /// Recolour `b`, page `index` drawn with `transform`, in `mode`.
    fn recolour(
        &self,
        index: usize,
        b: &mut Bitmap,
        transform: Affine,
        mode: PageColours,
        keep_pictures: bool,
    ) {
        if mode == PageColours::Normal {
            return;
        }
        let keep = match (keep_pictures, self.pdf.pages().get(index)) {
            (true, Some(page)) => page_colours::picture_boxes(
                page,
                &self.boxes,
                &self.settings,
                transform,
                b.width,
                b.height,
            ),
            _ => Vec::new(),
        };
        page_colours::recolour(b, mode, &keep);
    }

    /// Render so the bitmap is `width_px` wide.
    pub fn render_to_width(&self, index: usize, width_px: f32) -> Result<Bitmap> {
        let (w, _) = self.page_size(index)?;
        self.render(index, width_px / w.max(1.0))
    }
}

/// From `page`'s space to the pixels of a bitmap of it drawn at `scale`, whose top left is
/// at pixel (`x`, `y`) of the whole page's bitmap.
fn page_transform(page: &Page, scale: f32, x: u32, y: u32) -> Affine {
    Affine::translate((-(x as f64), -(y as f64)))
        * Affine::scale(scale as f64)
        * page.initial_transform(true).to_kurbo()
}

/// Parse PDF bytes for rendering.
pub fn load(bytes: Vec<u8>, password: Option<&str>) -> Result<HPdf> {
    HPdf::new_with_password(bytes, password.unwrap_or("")).map_err(|e| {
        let why = match e {
            LoadPdfError::Invalid => "the file's structure can't be read",
            LoadPdfError::Decryption(DecryptionError::PasswordProtected) => "it needs a password",
            LoadPdfError::Decryption(DecryptionError::UnsupportedAlgorithm) => {
                "it is encrypted in a way that isn't supported"
            }
            LoadPdfError::Decryption(_) => "its encryption is damaged",
        };
        anyhow!("cannot render PDF: {why}")
    })
}

/// Render the given pages in parallel (one renderer per worker thread, so fonts are read
/// once per thread rather than once per page).
pub fn render_pages_parallel(pdf: &HPdf, pages: &[usize], scale: f32) -> Vec<Result<Bitmap>> {
    pages
        .par_iter()
        .map_init(|| Renderer::new(pdf), |r, &i| r.render(i, scale))
        .collect()
}

/// File format for exported page images.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    /// JPEG at the given quality, 1 to 100.
    Jpeg(u8),
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg(_) => "jpg",
        }
    }
}

/// Highest export resolution: the renderer's largest scale.
pub const MAX_DPI: f32 = 16.0 * 72.0;

/// Encode a bitmap as an RGB image file that records `dpi`, so the image is imported
/// back at the page's size.
pub fn encode(b: &Bitmap, format: ImageFormat, dpi: f32) -> Result<Vec<u8>> {
    let (pixels, _) = b.rgba.as_chunks::<4>();
    let rgb: Vec<u8> = pixels.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let (w, h) = (b.width as u32, b.height as u32);
    let mut out = Vec::new();
    match format {
        ImageFormat::Png => {
            let mut e = png::Encoder::new(&mut out, w, h);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            e.set_compression(png::Compression::Fast);
            let ppm = (dpi / 0.0254).round() as u32;
            e.set_pixel_dims(Some(png::PixelDimensions {
                xppu: ppm,
                yppu: ppm,
                unit: png::Unit::Meter,
            }));
            let mut wr = e.write_header()?;
            wr.write_image_data(&rgb)?;
            wr.finish()?;
        }
        ImageFormat::Jpeg(q) => {
            let mut e = JpegEncoder::new_with_quality(&mut out, q.clamp(1, 100));
            e.set_pixel_density(PixelDensity::dpi(dpi.round() as u16));
            e.encode(&rgb, w, h, image::ExtendedColorType::Rgb8)?;
        }
    }
    Ok(out)
}

/// File names for exported pages (0-based): `<stem>-<page>.<ext>`, page numbers counted
/// from 1 and padded to at least three digits so the files sort in page order.
pub fn image_names(pages: &[usize], stem: &str, format: ImageFormat) -> Vec<String> {
    let digits = pages.iter().max().map_or(1, |&p| (p + 1).to_string().len());
    let digits = digits.max(3);
    pages
        .iter()
        .map(|p| format!("{stem}-{:0digits$}.{}", p + 1, format.extension()))
        .collect()
}

/// Render `pages` at `dpi` (at most [`MAX_DPI`]) and write them to `dir` with the names
/// from [`image_names`], each whole or not at all, replacing files of the same name. Pages
/// are rendered in parallel, in batches that keep the bitmaps in memory to about 1 GB. A
/// page too large to draw whole at `dpi` refuses the export before any file is written.
/// Returns the files written, and adds to `missing` what couldn't be drawn as the file asks
/// (as [`Renderer::take_missing`]). When every file was written but one wasn't confirmed
/// on disk, the error is that file's [`crate::NotConfirmed`], and `missing` is filled all
/// the same.
pub fn write_images(
    pdf: &HPdf,
    pages: &[usize],
    dpi: f32,
    format: ImageFormat,
    dir: &Path,
    stem: &str,
    missing: &mut BTreeSet<Missing>,
) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let paths: Vec<PathBuf> = image_names(pages, stem, format)
        .into_iter()
        .map(|n| dir.join(n))
        .collect();
    let unconfirmed = std::sync::Mutex::new(None);
    encode_batched(pdf, pages, dpi, format, missing, |k, bytes| {
        let path = &paths[k];
        match crate::write_file(path, &bytes) {
            // Written: the other pages go on.
            Err(e) if e.is::<crate::NotConfirmed>() => {
                let mut u = unconfirmed.lock().unwrap_or_else(|e| e.into_inner());
                u.get_or_insert(e);
                Ok(())
            }
            r => r.with_context(|| format!("writing {}", path.display())),
        }
    })?;
    let unconfirmed = unconfirmed.into_inner().unwrap_or_else(|e| e.into_inner());
    unconfirmed.map_or(Ok(paths), Err)
}

/// Render `pages` at `dpi` (at most [`MAX_DPI`]) and encode each as an image file, in
/// page order, as [`write_images`] would write them.
pub fn encode_pages(
    pdf: &HPdf,
    pages: &[usize],
    dpi: f32,
    format: ImageFormat,
) -> Result<Vec<Vec<u8>>> {
    encode_batched(pdf, pages, dpi, format, &mut BTreeSet::new(), |_, bytes| {
        Ok(bytes)
    })
}

/// Render and encode `pages` in parallel, on as many threads as keep the bitmaps in memory
/// to about 1 GB, passing each file and its position in `pages` to `f`. Every page's size
/// is checked before any is drawn. Returns the results of `f` in order, and adds what
/// couldn't be drawn as the file asks to `missing`, from the pages drawn.
fn encode_batched<T: Send>(
    pdf: &HPdf,
    pages: &[usize],
    dpi: f32,
    format: ImageFormat,
    missing: &mut BTreeSet<Missing>,
    f: impl Fn(usize, Vec<u8>) -> Result<T> + Sync,
) -> Result<Vec<T>> {
    let dpi = dpi.clamp(0.05 * 72.0, MAX_DPI);
    // hayro truncates the pixel size, and 612 pt at 100 dpi is 849.99994 px in f32: nudge
    // the scale so whole sizes come out whole.
    let scale = dpi / 72.0 * (1.0 + 1e-5);
    let all = pdf.pages();
    let mut biggest = 0.0f32;
    for &i in pages {
        let p = all.get(i).ok_or_else(|| anyhow!("no page {}", i + 1))?;
        bitmap_size(p, i, scale)?;
        let (w, h) = p.render_dimensions();
        biggest = biggest.max(w * h);
    }
    // RGBA bitmap, RGB copy and encoded file.
    let page_bytes = (biggest * scale * scale * 8.0).max(1.0);
    let batch = ((1u64 << 30) as f32 / page_bytes) as usize;
    let batch = batch.clamp(1, rayon::current_num_threads());
    let jobs: Vec<(usize, usize)> = pages.iter().copied().enumerate().collect();
    // Each thread keeps its renderer, and so the fonts it has read, from page to page.
    let pool = rayon::ThreadPoolBuilder::new().num_threads(batch).build()?;
    let found = Mutex::new(BTreeSet::new());
    let done = pool.install(|| {
        jobs.par_iter()
            .map_init(
                || Renderer::new(pdf),
                |r, &(k, i)| {
                    let b = r.render(i, scale);
                    let lost = r.take_missing();
                    found.lock().unwrap_or_else(|e| e.into_inner()).extend(lost);
                    f(k, encode(&b?, format, dpi)?)
                },
            )
            .collect()
    });
    missing.extend(found.into_inner().unwrap_or_else(|e| e.into_inner()));
    done
}

impl Pdf {
    /// Export `pages` (0-based) as image files, as [`write_images`] does, adding to
    /// `missing` what couldn't be drawn. Pages look as they do on screen: annotations
    /// flagged NoView are left out.
    pub fn export_images(
        &mut self,
        pages: &[usize],
        dpi: f32,
        format: ImageFormat,
        dir: &Path,
        stem: &str,
        missing: &mut BTreeSet<Missing>,
    ) -> Result<Vec<PathBuf>> {
        let hp = load(self.render_snapshot()?, None)?;
        write_images(&hp, pages, dpi, format, dir, stem, missing)
    }
}
