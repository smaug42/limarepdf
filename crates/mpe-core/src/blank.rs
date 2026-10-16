//! Finding blank pages: pages that are drawn as one even colour, give or take a few specks.

use crate::render::{self, Bitmap, Renderer};
use crate::{Pdf, Result};
use anyhow::bail;
use rayon::prelude::*;

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
    /// page's background colour, as drawn with its annotations. A page that can't be drawn is not blank.
    pub fn blank_pages(&mut self, tolerance: f32) -> Result<Vec<usize>> {
        let hp = render::load(self.render_snapshot()?, None)?;
        let blank: Vec<bool> = (0..self.page_count())
            .into_par_iter()
            .map(|i| {
                let r = Renderer::new(&hp);
                let Ok((w, h)) = r.page_size(i) else {
                    return false;
                };
                let scale = (MAX_SIDE / w.max(h).max(1.0)).min(1.0);
                r.render(i, scale)
                    .is_ok_and(|b| marked_share(&b) * 100.0 <= tolerance)
            })
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
