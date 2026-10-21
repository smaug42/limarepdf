//! Page rasterisation on top of `hayro` (pure Rust, CPU only).
//!
//! `hayro`'s render cache is not `Send`, so a [`Renderer`] lives on one thread.
//! For parallel work, create one renderer per worker (see [`render_pages_parallel`]).

use crate::installed_fonts::display_name;
pub use crate::installed_fonts::{InstalledFonts, Language};
use crate::page_colours::{self, PageColours};
use crate::{Pdf, Result};
use anyhow::{anyhow, Context};
use hayro::hayro_interpret::font::FontQuery;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{InterpreterCache, InterpreterSettings, InterpreterWarning};
use hayro::hayro_syntax::page::Page;
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf as HPdf};
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{render_into, RenderCache, RenderSettings};
use image::codecs::jpeg::{JpegEncoder, PixelDensity};
use rayon::prelude::*;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
/// (up to 65,535 pixels a side, 17 GB), so a failed allocation would close the app.
pub const MAX_PIXELS: u64 = 1 << 28;

/// Most pixels drawn in one pass of the rasteriser; a bigger bitmap is drawn in bands of
/// full width. hayro holds everything it draws until the pass ends, which on a page of
/// many paths is a few hundred bytes a pixel (PDFBox test file 5134, 73,000 paths: 2.7 GB
/// for its 8 million pixels at 72 dpi), so this bounds the memory of one drawing.
pub const BAND_PIXELS: u32 = 1 << 22;

/// An error when a bitmap `width` × `height` of page `index` would pass [`MAX_PIXELS`].
fn check_pixels(index: usize, width: u32, height: u32) -> Result<()> {
    if width as u64 * height as u64 > MAX_PIXELS {
        anyhow::bail!(
            "page {} is too large to draw at this size ({width} × {height} pixels)",
            index + 1
        );
    }
    Ok(())
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
    /// `u16::MAX` a side (the rasteriser's limit); `None` when nothing is left.
    fn within(self, width: u32, height: u32) -> Option<Region> {
        let w = self
            .width
            .min(width.saturating_sub(self.x))
            .min(u16::MAX as u32);
        let h = self
            .height
            .min(height.saturating_sub(self.y))
            .min(u16::MAX as u32);
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
    /// A picture that couldn't be decoded.
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

pub struct Renderer<'a> {
    pdf: &'a HPdf,
    cache: RenderCache<'a>,
    /// For finding pictures in [`Renderer::render_in`]; hayro keeps its own cache's
    /// interpreter half private.
    boxes: InterpreterCache<'a>,
    settings: InterpreterSettings,
    /// What hayro reported it couldn't draw, since [`Renderer::take_missing`] was last
    /// called.
    missing: Arc<Mutex<BTreeSet<Missing>>>,
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
            cache: RenderCache::new(),
            boxes: InterpreterCache::new(),
            settings,
            missing,
        }
    }

    /// What couldn't be drawn in the renders since the last call. hayro caches fonts and
    /// pictures across pages, so each is reported once per renderer, on the first page
    /// that uses it.
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

    /// [`Renderer::render`], drawn in bands of at most [`BAND_PIXELS`]. `stop` is asked
    /// before each band, and `None` is returned once it says to stop.
    pub fn render_unless(
        &self,
        index: usize,
        scale: f32,
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<Bitmap>> {
        let page = self.page(index)?;
        let scale = scale.clamp(0.05, 16.0);
        // The bitmap's size as `hayro::render` works it out.
        let (w, h) = page.render_dimensions();
        let (width, height) = ((w * scale) as u16 as u32, (h * scale) as u16 as u32);
        check_pixels(index, width, height)?;
        // Whole strips of 4 rows, the rasteriser's tile height, so the bands' tiles line
        // up with those of one pass and the bitmap comes out the same.
        let rows = (BAND_PIXELS / width.max(1)).max(4) / 4 * 4;
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        let mut y = 0;
        while y < height {
            if stop() {
                return Ok(None);
            }
            let band = Region {
                x: 0,
                y,
                width,
                height: rows.min(height - y),
            };
            rgba.extend_from_slice(self.draw(page, scale, band).data_as_u8_slice());
            y += band.height;
        }
        Ok(Some(Bitmap {
            width: width as usize,
            height: height as usize,
            rgba,
        }))
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
        check_pixels(index, r.width, r.height)?;
        let pm = self.draw(page, scale, r);
        Ok(Bitmap {
            width: pm.width() as usize,
            height: pm.height() as usize,
            rgba: pm.data_as_u8_slice().to_vec(),
        })
    }

    /// Draw the part `r` (within the bitmap, at most `u16::MAX` a side) of `page` drawn at
    /// `scale`, on white, as `hayro::render` draws a whole page.
    fn draw(&self, page: &'a Page<'a>, scale: f32, r: Region) -> Pixmap {
        let mut ctx = RenderContext::new(r.width as u16, r.height as u16);
        render_into(
            page,
            &self.cache,
            &self.settings,
            &RenderSettings::default(),
            &mut ctx,
            page_transform(page, scale, r.x, r.y),
        );
        ctx.flush();
        let mut pm = Pixmap::new(ctx.width(), ctx.height());
        ctx.render_with(
            &mut pm,
            &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(WHITE),
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
/// from [`image_names`], replacing files of the same name. Pages are rendered in
/// parallel, in batches that keep the bitmaps in memory to about 1 GB. Returns the files
/// written.
pub fn write_images(
    pdf: &HPdf,
    pages: &[usize],
    dpi: f32,
    format: ImageFormat,
    dir: &Path,
    stem: &str,
) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let paths: Vec<PathBuf> = image_names(pages, stem, format)
        .into_iter()
        .map(|n| dir.join(n))
        .collect();
    encode_batched(pdf, pages, dpi, format, |k, bytes| {
        let path = &paths[k];
        std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
    })?;
    Ok(paths)
}

/// Render `pages` at `dpi` (at most [`MAX_DPI`]) and encode each as an image file, in
/// page order, as [`write_images`] would write them.
pub fn encode_pages(
    pdf: &HPdf,
    pages: &[usize],
    dpi: f32,
    format: ImageFormat,
) -> Result<Vec<Vec<u8>>> {
    encode_batched(pdf, pages, dpi, format, |_, bytes| Ok(bytes))
}

/// Render and encode `pages` in parallel, on as many threads as keep the bitmaps in memory
/// to about 1 GB, passing each file and its position in `pages` to `f`. Returns the
/// results of `f` in order.
fn encode_batched<T: Send>(
    pdf: &HPdf,
    pages: &[usize],
    dpi: f32,
    format: ImageFormat,
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
    pool.install(|| {
        jobs.par_iter()
            .map_init(
                || Renderer::new(pdf),
                |r, &(k, i)| {
                    let b = r.render(i, scale)?;
                    f(k, encode(&b, format, dpi)?)
                },
            )
            .collect()
    })
}

impl Pdf {
    /// Export `pages` (0-based) as image files, as [`write_images`] does. Pages look as
    /// they do on screen: annotations flagged NoView are left out.
    pub fn export_images(
        &mut self,
        pages: &[usize],
        dpi: f32,
        format: ImageFormat,
        dir: &Path,
        stem: &str,
    ) -> Result<Vec<PathBuf>> {
        let hp = load(self.render_snapshot()?, None)?;
        write_images(&hp, pages, dpi, format, dir, stem)
    }
}
