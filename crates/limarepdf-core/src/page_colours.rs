//! Page colour modes for display: dark pages, high contrast and sepia, worked out on a
//! rendered [`Bitmap`]. Nothing here touches the document; printing and exports keep
//! the page's own colours.
//!
//! Pictures can be left as they are: [`picture_boxes`] runs the page through the
//! interpreter once more with a device that only notes where raster images land, and
//! [`recolour`] skips those boxes. Stencil masks (one-colour shapes painted in the fill
//! colour, such as some logos and Type 3 glyphs) are not pictures and are recoloured.

use crate::render::Bitmap;
use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::{
    interpret_page, BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image,
    ImageDrawProps, InterpreterCache, InterpreterSettings, SoftMask,
};
use hayro::hayro_syntax::page::Page;
use hayro::kurbo::{Affine, BezPath, Rect, Shape};

/// How pages are coloured on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PageColours {
    /// The page's own colours.
    #[default]
    Normal,
    /// Light and dark swapped, hues kept: dark grey paper, light text, red stays red.
    Dark,
    /// White on black: anything inked, in any colour, turns white.
    HighContrast,
    /// Warm paper and dark brown text.
    Sepia,
}

/// Dark mode's paper and ink, close to the app's dark theme.
const DARK_PAPER: u8 = 30;
const DARK_INK: u8 = 222;
/// Sepia's paper and ink.
const SEPIA_PAPER: [u8; 3] = [244, 236, 216];
const SEPIA_INK: [u8; 3] = [64, 48, 32];

impl PageColours {
    /// The colour white paper turns into, for drawing a page before its bitmap arrives.
    pub fn paper(self) -> [u8; 3] {
        self.pixel([255, 255, 255])
    }

    /// One opaque pixel in this mode.
    pub fn pixel(self, [r, g, b]: [u8; 3]) -> [u8; 3] {
        match self {
            PageColours::Normal => [r, g, b],
            PageColours::Dark => {
                // Inverting HSL lightness keeps hue and chroma: shift every channel by
                // the same amount so the largest and smallest swap places around 255.
                let (max, min) = (r.max(g).max(b) as i32, r.min(g).min(b) as i32);
                let d = 255 - max - min;
                let span = (DARK_INK - DARK_PAPER) as i32;
                let f = |c: u8| (DARK_PAPER as i32 + (c as i32 + d) * span / 255) as u8;
                [f(r), f(g), f(b)]
            }
            PageColours::HighContrast => {
                // How much ink: the weakest channel, so yellow counts as much as black
                // (lightness would lose yellow text against white paper). Stretched so
                // pale tints go black and anything stronger goes white.
                let ink = 255 - r.min(g).min(b) as i32;
                let v = ((ink - 48) * 255 / 128).clamp(0, 255) as u8;
                [v, v, v]
            }
            PageColours::Sepia => {
                let f = |c: u8, i: usize| {
                    let (ink, paper) = (SEPIA_INK[i] as u32, SEPIA_PAPER[i] as u32);
                    (ink + c as u32 * (paper - ink) / 255) as u8
                };
                [f(r, 0), f(g, 1), f(b, 2)]
            }
        }
    }
}

/// A box in bitmap pixels: left, top, right, bottom (right and bottom exclusive).
pub type PixelBox = [usize; 4];

/// Recolour `bitmap` in `mode`, leaving the pixels inside `keep` as they are.
pub fn recolour(bitmap: &mut Bitmap, mode: PageColours, keep: &[PixelBox]) {
    if mode == PageColours::Normal {
        return;
    }
    let w = bitmap.width;
    // Per row, the column spans to keep; most rows have none.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (y, row) in bitmap.rgba.chunks_exact_mut(w * 4).enumerate() {
        spans.clear();
        spans.extend(
            keep.iter()
                .filter(|k| y >= k[1] && y < k[3])
                .map(|k| (k[0].min(w), k[2].min(w))),
        );
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            if spans.iter().any(|&(a, b)| x >= a && x < b) {
                continue;
            }
            // The bitmap is premultiplied; pages are opaque, but stay correct if not.
            let a = px[3] as u32;
            if a == 0 {
                continue;
            }
            let un = |c: u8| (c as u32 * 255 / a).min(255) as u8;
            let [r, g, b] = mode.pixel([un(px[0]), un(px[1]), un(px[2])]);
            let pre = |c: u8| (c as u32 * a / 255) as u8;
            px[0] = pre(r);
            px[1] = pre(g);
            px[2] = pre(b);
        }
    }
}

/// Where raster pictures are drawn on `page` rendered with `transform` (page space to
/// bitmap pixels), in pixels of a bitmap `width` × `height`, each clipped to the clip in
/// force when it was drawn.
pub fn picture_boxes<'a>(
    page: &'a Page<'a>,
    cache: &InterpreterCache<'a>,
    settings: &InterpreterSettings,
    transform: Affine,
    width: usize,
    height: usize,
) -> Vec<PixelBox> {
    let full = Rect::new(0.0, 0.0, width as f64, height as f64);
    let mut ctx = Context::new(transform, full, cache, page.xref(), settings.clone());
    let mut dev = Boxes {
        clips: vec![full],
        found: Vec::new(),
    };
    let crop = hayro::hayro_interpret::util::RectExt::to_kurbo(&page.intersected_crop_box());
    let mut crop = crop.to_path(0.1);
    crop.apply_affine(transform);
    dev.push_clip_path(&ClipPath {
        path: crop,
        fill: hayro::hayro_interpret::FillRule::NonZero,
    });
    interpret_page(page, &mut ctx, &mut dev);
    dev.found
        .into_iter()
        .filter_map(|r| {
            let r = r.intersect(full);
            let b = [
                r.x0.floor() as usize,
                r.y0.floor() as usize,
                r.x1.ceil() as usize,
                r.y1.ceil() as usize,
            ];
            (b[2] > b[0] && b[3] > b[1]).then_some(b)
        })
        .collect()
}

/// A device that draws nothing and notes each raster image's box.
struct Boxes {
    /// Bounding boxes of the clips in force, innermost last.
    clips: Vec<Rect>,
    found: Vec<Rect>,
}

impl<'a> Device<'a> for Boxes {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}

    fn push_clip_path(&mut self, clip: &ClipPath) {
        let outer = *self.clips.last().unwrap_or(&Rect::ZERO);
        self.clips.push(outer.intersect(clip.path.bounding_box()));
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}

    fn draw_glyph_run(&mut self, _: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {}

    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        if let Image::Raster(_) = image {
            let r = Rect::new(0.0, 0.0, image.width() as f64, image.height() as f64);
            let r = props.transform.transform_rect_bbox(r);
            let clip = *self.clips.last().unwrap_or(&Rect::ZERO);
            let r = r.intersect(clip);
            if r.area() > 0.0 {
                self.found.push(r);
            }
        }
    }

    fn pop_clip(&mut self) {
        // The outermost entry is the whole bitmap and stays.
        if self.clips.len() > 1 {
            self.clips.pop();
        }
    }

    fn pop_transparency_group(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_swaps_light_and_dark_and_keeps_hue() {
        let m = PageColours::Dark;
        assert_eq!(m.pixel([255, 255, 255]), [DARK_PAPER; 3]);
        assert_eq!(m.pixel([0, 0, 0]), [DARK_INK; 3]);
        // Pure red keeps red as its strongest channel, equal others.
        let [r, g, b] = m.pixel([255, 0, 0]);
        assert!(r > g && g == b);
        // A pale blue background turns into a dark blue one.
        let [r, g, b] = m.pixel([220, 230, 255]);
        assert!(b > r && b > g && b < 100);
    }

    #[test]
    fn high_contrast_and_sepia_ends() {
        assert_eq!(PageColours::HighContrast.pixel([255, 255, 255]), [0; 3]);
        assert_eq!(PageColours::HighContrast.pixel([0, 0, 0]), [255; 3]);
        assert_eq!(PageColours::HighContrast.pixel([240, 240, 240]), [0; 3]);
        assert_eq!(PageColours::HighContrast.pixel([255, 230, 0]), [255; 3]);
        assert_eq!(PageColours::Sepia.paper(), SEPIA_PAPER);
        assert_eq!(PageColours::Sepia.pixel([0, 0, 0]), SEPIA_INK);
        assert_eq!(PageColours::Normal.pixel([1, 2, 3]), [1, 2, 3]);
    }

    #[test]
    fn recolour_skips_kept_boxes() {
        let mut b = Bitmap {
            width: 4,
            height: 2,
            rgba: [255u8; 32].to_vec(),
        };
        recolour(&mut b, PageColours::Dark, &[[1, 0, 3, 1]]);
        let px = |x: usize, y: usize| b.rgba[(y * 4 + x) * 4];
        assert_eq!(px(0, 0), DARK_PAPER);
        assert_eq!(px(1, 0), 255);
        assert_eq!(px(2, 0), 255);
        assert_eq!(px(3, 0), DARK_PAPER);
        assert_eq!(px(1, 1), DARK_PAPER);
        assert_eq!(b.rgba[3], 255);
    }
}
