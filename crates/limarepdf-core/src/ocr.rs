//! Text recognition for scanned pages.
//!
//! Words are found with classical image processing (threshold, closing of small gaps
//! along rows, connected components), grouped into lines by ocrs' layout analysis, and
//! read by the ocrs recognition model. Only the recognition model is used: its licence
//! (CC-BY-SA-4.0) allows shipping it, while the ocrs detection model's licence is not
//! stated.

use crate::fonts::winansi_bytes;
use crate::overlay::FONT_PREFIX;
use crate::scan::binarise;
use crate::{render, text_width, Pdf, Result};
use anyhow::{anyhow, bail};
use image::GrayImage;
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use rayon::prelude::*;
use rten_imageproc::{RectF, RotatedRect};
use std::fmt::Write;

/// Resolution pages are rendered at for recognition.
const OCR_DPI: f32 = 300.0;
/// Longest side of a rendered page, in pixels, for very large pages.
const MAX_SIDE: f32 = 7000.0;

/// Recognised lines of a page and the scale it was rendered at, or why it failed.
type PageRead = std::result::Result<(Vec<OcrLine>, f32), String>;

/// What happened to one page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OcrResult {
    /// A text layer was added.
    Added { lines: usize, words: usize },
    /// The page was left as it was, for the reason given.
    Skipped(String),
}

/// A text recogniser. Loading the model takes a moment, so keep one and reuse it.
pub struct Ocr {
    engine: OcrEngine,
}

/// A recognised word and its box in image pixels: left, top, right, bottom.
#[derive(Clone, Debug)]
pub struct OcrWord {
    pub text: String,
    pub bbox: [f32; 4],
}

/// A recognised line of text, words in reading order.
#[derive(Clone, Debug)]
pub struct OcrLine {
    pub words: Vec<OcrWord>,
}

impl OcrLine {
    pub fn text(&self) -> String {
        let words: Vec<&str> = self.words.iter().map(|w| w.text.as_str()).collect();
        words.join(" ")
    }
}

/// The bundled ocrs recognition model (CC BY-SA 4.0; see `models/README.md`).
static MODEL: &[u8] = include_bytes!("../models/text-recognition.rten");

impl Ocr {
    /// The recogniser with the model built into LimarePDF (Latin script).
    pub fn bundled() -> Result<Self> {
        let model = rten::Model::load_static_slice(MODEL)
            .map_err(|e| anyhow!("cannot load the OCR model: {e}"))?;
        let engine = OcrEngine::new(OcrEngineParams {
            recognition_model: Some(model),
            ..Default::default()
        })?;
        Ok(Self { engine })
    }

    /// Load another ocrs text recognition model (`.rten` format).
    pub fn new(recognition_model: Vec<u8>) -> Result<Self> {
        let model = rten::Model::load(recognition_model)
            .map_err(|e| anyhow!("cannot load the OCR model: {e}"))?;
        let engine = OcrEngine::new(OcrEngineParams {
            recognition_model: Some(model),
            ..Default::default()
        })?;
        Ok(Self { engine })
    }

    /// Read the text in a greyscale image (`luma`: `width` x `height`, row by row).
    /// Lines are returned in reading order.
    pub fn read(&self, luma: &[u8], width: u32, height: u32) -> Result<Vec<OcrLine>> {
        let img = GrayImage::from_raw(width, height, luma.to_vec())
            .ok_or_else(|| anyhow!("image data does not match its size"))?;
        let words = find_words(&img);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let source = ImageSource::from_bytes(luma, (width, height))
            .map_err(|e| anyhow!("cannot prepare the image: {e}"))?;
        let input = self.engine.prepare_input(source)?;
        let rects: Vec<RotatedRect> = words
            .iter()
            .map(|b| RotatedRect::from_rect(RectF::from_tlbr(b[1], b[0], b[3], b[2])))
            .collect();
        let lines = self.engine.find_text_lines(&input, &rects);
        let out = self.engine.recognize_text(&input, &lines)?;
        Ok(out
            .into_iter()
            .flatten()
            .map(|line| OcrLine {
                words: line
                    .words()
                    .map(|w| {
                        let r = w.bounding_rect();
                        OcrWord {
                            text: w.to_string(),
                            bbox: [
                                r.left() as f32,
                                r.top() as f32,
                                r.right() as f32,
                                r.bottom() as f32,
                            ],
                        }
                    })
                    .collect(),
            })
            // Lines with fewer than two letters or digits are almost always marks that
            // are not text (logos, texture, stray specks).
            .filter(|l: &OcrLine| {
                let alnum = l.words.iter().flat_map(|w| w.text.chars());
                alnum.filter(|c| c.is_alphanumeric()).count() >= 2
            })
            .collect())
    }
}

impl Pdf {
    /// Recognise the text on `pages` (0-based) and add it as an invisible, searchable,
    /// selectable text layer. Pages that already have text are skipped, so running this
    /// twice adds nothing. Run [`Pdf::optimize_scans`] first if wanted: it does not
    /// straighten pages that have text.
    pub fn ocr_pages(&mut self, pages: &[usize], ocr: &Ocr) -> Result<Vec<(usize, OcrResult)>> {
        if let Some(&bad) = pages.iter().find(|&&p| p >= self.page_count()) {
            bail!("no page {}", bad + 1);
        }
        let mut todo = Vec::new();
        let mut out: Vec<(usize, OcrResult)> = Vec::new();
        for &p in pages {
            match self.parse_page(p) {
                Ok(parsed) if parsed.shows_text() => {
                    out.push((p, OcrResult::Skipped("the page already has text".into())))
                }
                Ok(_) => todo.push(p),
                Err(e) => out.push((p, OcrResult::Skipped(format!("cannot read the page: {e}")))),
            }
        }
        let hp = render::load(self.snapshot()?, None)?;
        let found: Vec<(usize, PageRead)> = todo
            .par_iter()
            .map(|&p| {
                let read = || -> Result<(Vec<OcrLine>, f32)> {
                    let r = render::Renderer::new(&hp);
                    let (w, h) = r.page_size(p)?;
                    let scale = (OCR_DPI / 72.0).min(MAX_SIDE / w.max(h).max(1.0));
                    let b = r.render(p, scale)?;
                    let luma: Vec<u8> = b
                        .rgba
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|px| {
                            ((px[0] as u32 * 299 + px[1] as u32 * 587 + px[2] as u32 * 114) / 1000)
                                as u8
                        })
                        .collect();
                    Ok((ocr.read(&luma, b.width as u32, b.height as u32)?, scale))
                };
                (p, read().map_err(|e| format!("{e:#}")))
            })
            .collect();
        let mut font = None;
        for (p, r) in found {
            let result = match r {
                Err(why) => OcrResult::Skipped(why),
                Ok((lines, _)) if lines.is_empty() => OcrResult::Skipped("no text found".into()),
                Ok((lines, scale)) => {
                    let page = self.page_id_at(p)?;
                    let font = *font.get_or_insert_with(|| self.make_font());
                    let font_nm = self.add_resource(page, "Font", FONT_PREFIX, font.into())?;
                    let layer = self.text_layer(page, &lines, scale, &font_nm);
                    self.wrap_contents(page, None, Some(layer))?;
                    OcrResult::Added {
                        lines: lines.len(),
                        words: lines.iter().map(|l| l.words.len()).sum(),
                    }
                }
            };
            out.push((p, result));
        }
        out.sort_by_key(|(p, _)| pages.iter().position(|q| q == p));
        Ok(out)
    }

    /// Content stream drawing `lines` as invisible text. Word boxes are in pixels of
    /// the displayed page rendered at `scale`; `font` is the page's name for Helvetica.
    fn text_layer(
        &self,
        page: lopdf::ObjectId,
        lines: &[OcrLine],
        scale: f32,
        font: &str,
    ) -> Vec<u8> {
        let (m, _, vh) = self.visual_matrix(page);
        let mut s = format!(
            "q {} {} {} {} {} {} cm\n",
            m[0], m[1], m[2], m[3], m[4], m[5]
        );
        for line in lines {
            let top = line
                .words
                .iter()
                .map(|w| w.bbox[1])
                .fold(f32::MAX, f32::min);
            let bottom = line
                .words
                .iter()
                .map(|w| w.bbox[3])
                .fold(f32::MIN, f32::max);
            // The line box runs from ascender to descender: about 1.15 em in Helvetica.
            let size = ((bottom - top) / scale / 1.15).max(1.0);
            let baseline = vh - bottom / scale + 0.21 * size;
            let _ = writeln!(s, "BT 3 Tr /{font} {size:.2} Tf");
            for (i, w) in line.words.iter().enumerate() {
                let x = w.bbox[0] / scale;
                let width = (w.bbox[2] - w.bbox[0]) / scale;
                let natural = text_width(&w.text, size).max(0.1);
                let tz = (100.0 * width / natural).clamp(10.0, 1000.0);
                let mut text = w.text.clone();
                if i + 1 < line.words.len() {
                    text.push(' ');
                }
                let hex: String = winansi_bytes(&text)
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect();
                let _ = writeln!(s, "{tz:.1} Tz 1 0 0 1 {x:.2} {baseline:.2} Tm <{hex}> Tj");
            }
            s.push_str("ET\n");
        }
        s.push_str("Q\n");
        s.into_bytes()
    }
}

/// Bounding box of a group of black pixels, right and bottom exclusive.
#[derive(Clone, Copy, Debug)]
struct Comp {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    pixels: usize,
    /// Index of one of its pixels.
    seed: usize,
}

impl Comp {
    fn w(&self) -> u32 {
        self.x1 - self.x0
    }
    fn h(&self) -> u32 {
        self.y1 - self.y0
    }
}

/// Groups of touching black (0) pixels, 8-connected.
fn components(b: &GrayImage) -> Vec<Comp> {
    let (w, h) = (b.width() as usize, b.height() as usize);
    let raw = b.as_raw();
    let mut seen = vec![false; w * h];
    let mut stack = Vec::new();
    let mut out = Vec::new();
    for start in 0..w * h {
        if seen[start] || raw[start] != 0 {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (sx, sy) = ((start % w) as u32, (start / w) as u32);
        let mut c = Comp {
            x0: sx,
            y0: sy,
            x1: sx + 1,
            y1: sy + 1,
            pixels: 0,
            seed: start,
        };
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            c.pixels += 1;
            c.x0 = c.x0.min(x as u32);
            c.y0 = c.y0.min(y as u32);
            c.x1 = c.x1.max(x as u32 + 1);
            c.y1 = c.y1.max(y as u32 + 1);
            for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let j = ny * w + nx;
                    if !seen[j] && raw[j] == 0 {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        out.push(c);
    }
    out
}

/// Whiten the group of touching black pixels that contains pixel `seed`.
fn erase(b: &mut GrayImage, seed: usize, stack: &mut Vec<usize>) {
    let (w, h) = (b.width() as usize, b.height() as usize);
    let raw: &mut [u8] = b;
    if raw[seed] != 0 {
        return;
    }
    raw[seed] = 255;
    stack.push(seed);
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
            for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                let j = ny * w + nx;
                if raw[j] == 0 {
                    raw[j] = 255;
                    stack.push(j);
                }
            }
        }
    }
}

/// Median of `heights`.
fn median(mut heights: Vec<u32>) -> Option<f32> {
    heights.sort_unstable();
    heights.get(heights.len() / 2).map(|&h| h as f32)
}

/// Word boxes (left, top, right, bottom, in pixels), padded for recognition.
fn find_words(g: &GrayImage) -> Vec<[f32; 4]> {
    let (w, h) = g.dimensions();
    let mut bits = binarise(g);
    let comps: Vec<Comp> = components(&bits);
    let not_page_sized =
        |c: &Comp| (c.h() as f32) < 0.15 * h as f32 && (c.w() as f32) < 0.6 * w as f32;
    // Typical letter height, in two steps. First the ink-weighted median height, which
    // many specks (dust, photocopy texture) cannot drag down; then the plain median of the
    // groups at least a quarter of that, so that large bold headings, which hold much of
    // the ink, do not make body text look like specks.
    let mut sized: Vec<&Comp> = comps
        .iter()
        .filter(|c| c.h() >= 4 && not_page_sized(c))
        .collect();
    sized.sort_unstable_by_key(|c| c.h());
    let ink: usize = sized.iter().map(|c| c.pixels).sum();
    let mut acc = 0;
    let Some(heavy) = sized.iter().find(|c| {
        acc += c.pixels;
        acc * 2 >= ink
    }) else {
        return Vec::new();
    };
    let floor = 0.25 * heavy.h() as f32;
    let Some(th) = median(
        sized
            .iter()
            .map(|c| c.h())
            .filter(|&ch| ch as f32 >= floor)
            .collect(),
    ) else {
        return Vec::new();
    };
    // Erase frames and outlines: groups whose box is nearly empty. Size alone is not a
    // reason, since blurred letters on neighbouring lines can touch and form big groups.
    // Small marks stay: they are punctuation, dots, accents and pieces of broken letters.
    let mut stack = Vec::new();
    let mut letters = 0;
    for c in &comps {
        let (cw, ch) = (c.w() as f32, c.h() as f32);
        if ch < 0.4 * th {
            continue;
        }
        let fill = c.pixels as f32 / (cw * ch);
        if fill >= 0.06 {
            letters += 1;
        } else {
            erase(&mut bits, c.seed, &mut stack);
        }
    }
    if letters < 3 {
        return Vec::new();
    }
    // Join the letters of a word: fill gaps along each row up to this width. Letter gaps
    // are around a fifth of the median letter height, word spaces about half of it.
    let gap = (0.4 * th).round().max(1.0) as usize;
    for row in bits.chunks_mut(w as usize) {
        let mut last_black: Option<usize> = None;
        for x in 0..row.len() {
            if row[x] == 0 {
                if let Some(l) = last_black {
                    if x - l > 1 && x - l <= gap + 1 {
                        row[l + 1..x].fill(0);
                    }
                }
                last_black = Some(x);
            }
        }
    }
    // Words, without page-sized things. `th` is effectively the x-height (most letters are
    // lower case), so any word is at least about that tall; flatter groups are dotted rules
    // and specks.
    // Very wide groups are texture or rules, not words: even long words are well under 15
    // x-heights across.
    let joined_up = components(&bits);
    // Small marks left after joining: dots and accents to join to the word below.
    let small: Vec<Comp> = joined_up
        .iter()
        .filter(|c| (c.h() as f32) < 0.4 * th && (c.w() as f32) < 0.6 * th)
        .copied()
        .collect();
    let mut words: Vec<Comp> = joined_up
        .into_iter()
        .filter(|c| not_page_sized(c) && c.h() as f32 >= 0.7 * th)
        .collect();
    // Join boxes of about the same height that touch or overlap on the same row (widely
    // spaced letters, as in logos). Measuring the overlap against the taller box keeps a
    // group spanning several lines (blurred letters that touch) from swallowing words.
    words.sort_unstable_by_key(|c| c.x0);
    let mut joined: Vec<Comp> = Vec::with_capacity(words.len());
    for c in words {
        let near = joined.iter_mut().rev().find(|j| {
            let overlap_y = j.y1.min(c.y1).saturating_sub(j.y0.max(c.y0)) as f32;
            c.x0 as f32 <= j.x1 as f32 + 0.1 * th && overlap_y >= 0.6 * c.h().max(j.h()) as f32
        });
        match near {
            Some(j) => {
                j.x0 = j.x0.min(c.x0);
                j.y0 = j.y0.min(c.y0);
                j.x1 = j.x1.max(c.x1);
                j.y1 = j.y1.max(c.y1);
                j.pixels += c.pixels;
            }
            None => joined.push(c),
        }
    }
    // Scanner edges: nothing touching the outermost 1% of the image is read.
    let (mx, my) = (w / 100, h / 100);
    joined.retain(|c| c.x0 > mx && c.y0 > my && c.x1 < w - mx && c.y1 < h - my);
    // Thin upright slivers with nothing beside them are pieces of borders and rules.
    let sliver = |c: &Comp| (c.w() as f32) < 0.35 * th && (c.h() as f32) > 1.3 * th;
    let alone: Vec<bool> = joined
        .iter()
        .map(|c| {
            sliver(c)
                && !joined.iter().any(|o| {
                    !std::ptr::eq(o, c)
                        && o.y1.min(c.y1) > o.y0.max(c.y0)
                        && (o.x0 as f32 - c.x1 as f32).max(c.x0 as f32 - o.x1 as f32) < 1.5 * th
                })
        })
        .collect();
    let mut keep = alone.iter().map(|a| !a);
    joined.retain(|_| keep.next().unwrap());
    let mut words = joined;
    // Dots and accents: join them to the word just below. Other small marks are specks.
    for s in small {
        let below = words.iter_mut().find(|wd| {
            s.x1 > wd.x0
                && s.x0 < wd.x1
                && s.y1 <= wd.y0 + 1
                && (wd.y0 - s.y1.min(wd.y0)) as f32 <= 0.6 * th
        });
        if let Some(wd) = below {
            wd.y0 = wd.y0.min(s.y0);
        }
    }
    // A little margin: line spacing can be tight, and a box that reaches into the next
    // line confuses both line grouping and recognition.
    let (pad_x, pad_y) = (0.15 * th, 0.1 * th);
    words
        .into_iter()
        .map(|c| {
            [
                (c.x0 as f32 - pad_x).max(0.0),
                (c.y0 as f32 - pad_y).max(0.0),
                (c.x1 as f32 + pad_x).min(w as f32),
                (c.y1 as f32 + pad_y).min(h as f32),
            ]
        })
        .collect()
}
