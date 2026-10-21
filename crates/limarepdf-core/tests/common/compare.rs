//! Comparing two versions of a document: their pages drawn (pixels, allowing for
//! anti-aliasing) and what a reader finds in them (pages, annotations, form fields, bookmarks,
//! page text). Used by `render_compare.rs` (against poppler) and `save_reopen.rs` (a document
//! against itself saved and opened again); kept general for other before-and-after checks.

use limarepdf_core::render::Bitmap;
use limarepdf_core::Pdf;
use rayon::prelude::*;

/// A page drawn, as RGB.
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

impl Image {
    pub fn from_bitmap(b: &Bitmap) -> Self {
        let rgb = b
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&[r, g, b, _]| [r, g, b])
            .collect();
        Image {
            width: b.width,
            height: b.height,
            rgb,
        }
    }

    pub fn px(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
}

/// Whether `a`'s pixel at (x, y) is, in some channel, further than `tolerance` outside the
/// range of `b`'s pixels within one of it. Any blend of neighbouring colours, as anti-aliasing
/// at an edge a fraction of a pixel away gives, is inside the range.
fn unmatched(a: &Image, b: &Image, x: usize, y: usize, w: usize, h: usize, tolerance: u8) -> bool {
    let p = a.px(x, y);
    let (mut lo, mut hi) = ([255u8; 3], [0u8; 3]);
    for qy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
        for qx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
            let q = b.px(qx, qy);
            for k in 0..3 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
    }
    (0..3).any(|k| p[k].saturating_add(tolerance) < lo[k] || p[k] > hi[k].saturating_add(tolerance))
}

/// The pixels that differ beyond anti-aliasing and `tolerance`, over the area both images
/// cover: the mask, its width and its height.
pub fn differences(a: &Image, b: &Image, tolerance: u8) -> (Vec<bool>, usize, usize) {
    let (w, h) = (a.width.min(b.width), a.height.min(b.height));
    let mut mask = vec![false; w * h];
    mask.par_chunks_mut(w.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, m) in row.iter_mut().enumerate() {
                *m = unmatched(a, b, x, y, w, h, tolerance)
                    || unmatched(b, a, x, y, w, h, tolerance);
            }
        });
    (mask, w, h)
}

/// `a`, `b` and the differences (red on a faded `b`), side by side.
pub fn side_by_side(a: &Image, b: &Image, mask: &[bool], w: usize) -> image::RgbImage {
    const GAP: usize = 8;
    let h = a.height.max(b.height);
    let total = a.width + b.width + w + 2 * GAP;
    let mut out = image::RgbImage::from_pixel(total as u32, h as u32, image::Rgb([128; 3]));
    let mut put = |img: &Image, left: usize| {
        for y in 0..img.height {
            for x in 0..img.width {
                out.put_pixel((left + x) as u32, y as u32, image::Rgb(img.px(x, y)));
            }
        }
    };
    put(a, 0);
    put(b, a.width + GAP);
    let left = a.width + b.width + 2 * GAP;
    for (i, &m) in mask.iter().enumerate() {
        let (x, y) = (i % w, i / w);
        let c = if m {
            [230, 0, 0]
        } else {
            b.px(x, y).map(|v| 255 - (255 - v) / 4)
        };
        out.put_pixel((left + x) as u32, y as u32, image::Rgb(c));
    }
    out
}

/// What a reader finds in a document, each item as text so that two versions compare item by
/// item. Object numbers are left out: a save may renumber objects. Positions are rounded to
/// hundredths of a point, as a save may write numbers shorter.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    pub pages: usize,
    /// Each page's displayed size.
    pub sizes: Vec<String>,
    pub labels: Option<Vec<String>>,
    /// Page, kind, place, comment, author, flags, whether it has a drawing, whether it is a
    /// reply.
    pub annotations: Vec<String>,
    /// Name, kind, value, options, flags and where each widget is.
    pub fields: Vec<String>,
    /// Depth, title, page.
    pub outline: Vec<String>,
    /// Each page's text, as search and copying read it.
    pub text: Vec<String>,
}

fn round(v: f32) -> String {
    format!("{:.2}", (v * 100.0).round() / 100.0)
}

fn rect(r: &[f32; 4]) -> String {
    r.map(round).join(" ")
}

impl Facts {
    pub fn of(p: &Pdf) -> Self {
        let pages = p.page_count();
        let sizes = (0..pages)
            .map(|i| match p.page_size(i) {
                Ok((w, h)) => format!("{} × {}", round(w), round(h)),
                Err(e) => format!("no size: {e}"),
            })
            .collect();
        let annotations = p
            .annotations()
            .iter()
            .map(|a| {
                format!(
                    "page {} {:?} [{}] {:?} by {:?}, flags {}, drawing {}, reply {}",
                    a.page + 1,
                    a.kind,
                    rect(&a.bbox),
                    a.contents,
                    a.author,
                    a.flags,
                    a.has_appearance,
                    a.reply_to.is_some()
                )
            })
            .collect();
        let fields = p
            .form_fields()
            .iter()
            .map(|f| {
                let widgets: Vec<String> = f
                    .widgets
                    .iter()
                    .map(|w| format!("page {} [{}]", w.page + 1, rect(&w.bbox)))
                    .collect();
                format!(
                    "{:?} {:?} = {:?}, options {:?}, read-only {}, required {}, widgets {}",
                    f.name,
                    f.kind,
                    f.value,
                    f.options,
                    f.read_only,
                    f.required,
                    widgets.join(", ")
                )
            })
            .collect();
        let outline = p
            .outline()
            .iter()
            .map(|o| {
                let page = o
                    .page
                    .map_or("nowhere here".to_string(), |n| format!("page {}", n + 1));
                format!("{} {:?}, {page}", o.depth, o.title)
            })
            .collect();
        let text = (0..pages).into_par_iter().map(|i| p.page_text(i)).collect();
        Facts {
            pages,
            sizes,
            labels: p.page_labels(),
            annotations,
            fields,
            outline,
            text,
        }
    }

    /// Where `other` differs from `self`, each in a line; empty when they are the same.
    pub fn differences(&self, other: &Facts) -> Vec<String> {
        let mut out = Vec::new();
        if self.pages != other.pages {
            out.push(format!("{} pages, then {}", self.pages, other.pages));
        }
        if self.labels != other.labels {
            out.push(format!(
                "page labels {:?}, then {:?}",
                self.labels, other.labels
            ));
        }
        let lists = [
            ("page size", &self.sizes, &other.sizes),
            ("annotation", &self.annotations, &other.annotations),
            ("form field", &self.fields, &other.fields),
            ("bookmark", &self.outline, &other.outline),
        ];
        for (what, a, b) in lists {
            if a.len() != b.len() {
                out.push(format!("{} {what}s, then {}", a.len(), b.len()));
            }
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                if x != y {
                    out.push(format!("{what} {}: {x}, then {y}", i + 1));
                }
            }
        }
        for (i, (x, y)) in self.text.iter().zip(&other.text).enumerate() {
            if x != y {
                out.push(format!(
                    "page {}'s text differs: {}",
                    i + 1,
                    first_change(x, y)
                ));
            }
        }
        out
    }
}

/// Where `b` first differs from `a`, with a little of each from there.
fn first_change(a: &str, b: &str) -> String {
    let at = a
        .char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or(a.len().min(b.len()), |((i, _), _)| i);
    // The text before is the same in both, so `at` is a character boundary in each.
    let bit = |s: &str| s[at.min(s.len())..].chars().take(40).collect::<String>();
    format!("at character {at}, {:?}, then {:?}", bit(a), bit(b))
}
