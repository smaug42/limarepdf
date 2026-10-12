//! Page rasterisation on top of `hayro` (pure Rust, CPU only).
//!
//! `hayro`'s render cache is not `Send`, so a [`Renderer`] lives on one thread.
//! For parallel work, create one renderer per worker (see [`render_pages_parallel`]).

use crate::Result;
use anyhow::anyhow;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf as HPdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, PixmapSettings, RenderCache, RenderSettings};
use rayon::prelude::*;

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
