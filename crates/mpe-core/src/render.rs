//! Page rasterisation on top of `hayro` (pure Rust, CPU only).
//!
//! `hayro`'s render cache is not `Send`, so a [`Renderer`] lives on one thread.
//! For parallel work, create one renderer per worker (see [`render_pages_parallel`]).

use crate::{Pdf, Result};
use anyhow::{anyhow, Context};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf as HPdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, PixmapSettings, RenderCache, RenderSettings};
use image::codecs::jpeg::{JpegEncoder, PixelDensity};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

/// An opaque RGBA8 bitmap (row-major, 4 bytes per pixel).
#[derive(Clone)]
pub struct Bitmap {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct Renderer<'a> {
    pdf: &'a HPdf,
    cache: RenderCache<'a>,
    settings: InterpreterSettings,
}

impl<'a> Renderer<'a> {
    pub fn new(pdf: &'a HPdf) -> Self {
        Self {
            pdf,
            cache: RenderCache::new(),
            settings: InterpreterSettings::default(),
        }
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// Displayed page size in points.
    pub fn page_size(&self, index: usize) -> Result<(f32, f32)> {
        let p = self
            .pdf
            .pages()
            .get(index)
            .ok_or_else(|| anyhow!("no page {}", index + 1))?;
        Ok(p.render_dimensions())
    }

    /// Render a page at `scale` (1.0 = 72 dpi, 2.0 = 144 dpi).
    pub fn render(&self, index: usize, scale: f32) -> Result<Bitmap> {
        let page = self
            .pdf
            .pages()
            .get(index)
            .ok_or_else(|| anyhow!("no page {}", index + 1))?;
        let scale = scale.clamp(0.05, 16.0);
        let pm = render(
            page,
            &self.cache,
            &self.settings,
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );
        Ok(Bitmap {
            width: pm.width() as usize,
            height: pm.height() as usize,
            rgba: pm.data_as_u8_slice().to_vec(),
        })
    }

    /// Render so the bitmap is `width_px` wide.
    pub fn render_to_width(&self, index: usize, width_px: f32) -> Result<Bitmap> {
        let (w, _) = self.page_size(index)?;
        self.render(index, width_px / w.max(1.0))
    }
}

/// Parse PDF bytes for rendering.
pub fn load(bytes: Vec<u8>, password: Option<&str>) -> Result<HPdf> {
    HPdf::new_with_password(bytes, password.unwrap_or(""))
        .map_err(|e| anyhow!("cannot render PDF: {e:?}"))
}

/// Render the given pages in parallel (one renderer per rayon task).
pub fn render_pages_parallel(pdf: &HPdf, pages: &[usize], scale: f32) -> Vec<Result<Bitmap>> {
    pages
        .par_iter()
        .map(|&i| {
            let r = Renderer::new(pdf);
            r.render(i, scale)
        })
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
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let paths: Vec<PathBuf> = image_names(pages, stem, format)
        .into_iter()
        .map(|n| dir.join(n))
        .collect();
    let jobs: Vec<(usize, &PathBuf)> = pages.iter().copied().zip(&paths).collect();
    for chunk in jobs.chunks(batch) {
        chunk.par_iter().try_for_each(|&(i, path)| -> Result<()> {
            let b = Renderer::new(pdf).render(i, scale)?;
            let bytes = encode(&b, format, dpi)?;
            std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
        })?;
    }
    Ok(paths)
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
