//! Text recognition for scanned pages.
//!
//! Words are found with classical image processing (threshold, closing of small gaps
//! along rows, connected components) and grouped into lines by ocrs' layout analysis.
//! Each line is read by a PaddleOCR text recognition model (Apache-2.0; see
//! `models/README.md`), run by rten: given the line's picture, the model gives for each
//! step of it (8 pixels across) how likely each character is there, read out as CTC
//! output is (repeats joined, blanks dropped).

use crate::embed::Face;
use crate::fonts::winansi_byte;
use crate::matrix as mx;
use crate::objects::{ordered_words, Parsed, TextData};
use crate::overlay::FONT_PREFIX;
use crate::scan::binarise;
use crate::{render, FontChoice, ObjectKind, Pdf, Result};
use anyhow::{anyhow, bail};
use image::imageops::{self, FilterType};
use image::{GrayImage, Luma};
use lopdf::{Object, ObjectId};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rayon::prelude::*;
use rten::Model;
use rten_imageproc::{BoundingRect, RectF, RotatedRect};
use rten_tensor::prelude::*;
use rten_tensor::{NdTensor, NdTensorView};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

/// Resolution pages are rendered at for recognition.
const OCR_DPI: f32 = 300.0;
/// Longest side of a rendered page, in pixels, for very large pages.
const MAX_SIDE: f32 = 7000.0;
/// Height of the line pictures the recognition model reads, in pixels.
const LINE_HEIGHT: u32 = 48;
/// Pixels across a line picture per step of the model's output.
const STEP: u32 = 8;
/// Widest line picture read: 120 times its height, longer than any line of a page.
const MAX_WIDTH: u32 = 120 * LINE_HEIGHT;
/// Lines read at once.
const BATCH: usize = 8;
/// Least height of a line picture, in letter heights: about a line from the top of its
/// capitals to the bottom of its descenders, so a line without them isn't read too large.
const LINE_MIN: f32 = 2.2;

/// Recognised lines of a page and the scale it was rendered at, or why it failed.
type PageRead = std::result::Result<(Vec<OcrLine>, f32), String>;

/// What happened to one page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OcrResult {
    /// A text layer was added.
    Added { lines: usize, words: usize },
    /// Recognised text was removed: this many lines ([`Pdf::remove_recognised_text`]).
    Removed { lines: usize },
    /// The page was left as it was, for the reason given.
    Skipped(String),
}

/// A text recogniser. Loading the model takes a moment, so keep one and reuse it.
pub struct Ocr {
    /// Groups words into lines; it has no models.
    layout: OcrEngine,
    model: Model,
    /// The model's classes that are read, each with the character it writes: none for the
    /// blank, class 0. Classes of characters LimarePDF doesn't read are left out.
    classes: Vec<(usize, Option<char>)>,
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

/// The bundled recognition model and its characters, one a line in the order of its
/// classes (Apache-2.0; see `models/README.md`).
static MODEL: &[u8] = include_bytes!("../models/text-recognition.onnx");
static CHARACTERS: &str = include_str!("../models/text-recognition-chars.txt");

/// Whether `c` is read: a letter, digit or sign of the languages written in Latin letters
/// that the text layer can write. Other scripts are left out, so a Latin letter isn't read
/// as the Greek or Cyrillic one that looks the same.
fn readable(c: char) -> bool {
    let latin = matches!(c as u32,
        0x20..=0x7E // ASCII
        | 0xA0..=0x24F // Latin-1, Latin Extended-A and -B
        | 0x1E00..=0x1EFF // Latin Extended Additional
        | 0x2000..=0x206F // General Punctuation
        | 0x20A0..=0x20CF // Currency Symbols
        | 0x2100..=0x214F // Letterlike Symbols
    );
    latin && (winansi_byte(c).is_some() || Face::liberation(FontChoice::default()).covers(c))
}

impl Ocr {
    /// The recogniser with the model built into LimarePDF, which reads the languages
    /// written in Latin letters.
    pub fn bundled() -> Result<Self> {
        let model = Model::load_static_slice(MODEL)
            .map_err(|e| anyhow!("cannot load the OCR model: {e}"))?;
        Self::with(model, CHARACTERS)
    }

    /// Load another PaddleOCR text recognition model (ONNX, reading lines 48 pixels high),
    /// with its characters in the order of its classes, one a line (its `character_dict`).
    pub fn new(model: Vec<u8>, characters: &str) -> Result<Self> {
        let model = Model::load(model).map_err(|e| anyhow!("cannot load the OCR model: {e}"))?;
        Self::with(model, characters)
    }

    fn with(model: Model, characters: &str) -> Result<Self> {
        let list = characters.strip_suffix('\n').unwrap_or(characters);
        let mut classes = vec![(0, None)];
        let mut n = 0;
        for line in list.split('\n') {
            n += 1;
            // A list checked out on Windows may end its lines with CR LF.
            let line = line.strip_suffix('\r').unwrap_or(line);
            let mut chars = line.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                bail!("line {n} of the OCR model's characters isn't one character");
            };
            if readable(c) {
                classes.push((n, Some(if c.is_whitespace() { ' ' } else { c })));
            }
        }
        // The class after the characters is the space between words.
        classes.push((n + 1, Some(' ')));
        let ocr = Self {
            layout: OcrEngine::new(OcrEngineParams::default())?,
            model,
            classes,
        };
        let probe = NdTensor::full([1, 3, LINE_HEIGHT as usize, STEP as usize], 1.0);
        let found = ocr.run(probe)?.size(2);
        if found != n + 2 {
            bail!("the OCR model has {found} classes, not {n} characters, a blank and a space");
        }
        Ok(ocr)
    }

    /// The model's output for a batch of line pictures, `[lines, 3, 48, width]`: for each
    /// line, each step's likelihood of each class.
    fn run(&self, pictures: NdTensor<f32, 4>) -> Result<NdTensor<f32, 3>> {
        let out = (self.model.run_one(pictures.into(), None))
            .map_err(|e| anyhow!("the OCR model failed: {e}"))?;
        out.try_into()
            .map_err(|e| anyhow!("the OCR model's output isn't lines of steps of classes: {e}"))
    }

    /// The characters of a line in the first `steps` of its output, each with its first
    /// and last step.
    fn decode(&self, out: NdTensorView<f32, 2>, steps: usize) -> Vec<(char, usize, usize)> {
        let mut read: Vec<(char, usize, usize)> = Vec::new();
        let mut last = 0;
        for t in 0..steps.min(out.size(0)) {
            let row = out.slice(t);
            let best = self
                .classes
                .iter()
                .max_by(|a, b| row[a.0].total_cmp(&row[b.0]));
            let &(class, c) = best.expect("the blank is always read");
            match c {
                Some(_) if class == last => read.last_mut().expect("read just before").2 = t,
                Some(c) => read.push((c, t, t)),
                None => {}
            }
            last = class;
        }
        read
    }

    /// Read the text in a greyscale image (`luma`: `width` x `height`, row by row).
    /// Lines are returned in reading order.
    pub fn read(&self, luma: &[u8], width: u32, height: u32) -> Result<Vec<OcrLine>> {
        let img = GrayImage::from_raw(width, height, luma.to_vec())
            .ok_or_else(|| anyhow!("image data does not match its size"))?;
        let (words, th) = find_words(&img);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        // Line grouping looks only at the word boxes, not at the image it is given.
        let none = ImageSource::from_bytes(&[255], (1, 1))
            .map_err(|e| anyhow!("cannot prepare the image: {e}"))?;
        let input = self.layout.prepare_input(none)?;
        let rects: Vec<RotatedRect> = words
            .iter()
            .map(|b| RotatedRect::from_rect(RectF::from_tlbr(b[1], b[0], b[3], b[2])))
            .collect();
        let lines: Vec<Vec<[f32; 4]>> = (self.layout.find_text_lines(&input, &rects).iter())
            .filter(|l| !l.is_empty())
            .map(|l| {
                (l.iter().map(|r| r.bounding_rect()))
                    .map(|r| [r.left(), r.top(), r.right(), r.bottom()])
                    .collect()
            })
            .collect();
        let boxes: Vec<[f32; 4]> = (lines.iter())
            .map(|l| {
                l.iter()
                    .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, r| {
                        [
                            b[0].min(r[0]),
                            b[1].min(r[1]),
                            b[2].max(r[2]),
                            b[3].max(r[3]),
                        ]
                    })
            })
            .collect();
        let slope = tilt(&lines);
        let pictures: Vec<LinePicture> = boxes
            .iter()
            .map(|&b| LinePicture::new(&img, b, slope, th))
            .collect();
        // Lines of about the same width read together, so little of each batch is padding.
        let mut order: Vec<usize> = (0..pictures.len()).collect();
        order.sort_by_key(|&i| pictures[i].image.width());
        let mut chars = vec![Vec::new(); pictures.len()];
        for batch in order.chunks(BATCH) {
            let widest = batch.iter().map(|&i| pictures[i].image.width()).max();
            let wide = widest.unwrap_or(STEP).div_ceil(STEP) * STEP;
            // White, (255 / 255 - 0.5) / 0.5, beyond each line's end.
            let shape = [batch.len(), 3, LINE_HEIGHT as usize, wide as usize];
            let mut t = NdTensor::full(shape, 1.0);
            for (k, &i) in batch.iter().enumerate() {
                for (x, y, px) in pictures[i].image.enumerate_pixels() {
                    let v = px.0[0] as f32 / 127.5 - 1.0;
                    for c in 0..3 {
                        t[[k, c, y as usize, x as usize]] = v;
                    }
                }
            }
            let out = self.run(t)?;
            for (k, &i) in batch.iter().enumerate() {
                let steps = pictures[i].image.width().div_ceil(STEP) as usize;
                chars[i] = self.decode(out.slice(k), steps);
            }
        }
        Ok((boxes.iter().zip(&pictures).zip(&chars).zip(&lines))
            .map(|(((&b, pic), chars), found)| OcrLine {
                words: line_words(&img, b, pic, chars, &gaps(found)),
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

/// How much the lines of a page slope, down per across: the slope that best fits the
/// middles of the words of each line, weighed by their width. A page scanned at a slight
/// tilt is read with its lines straightened.
fn tilt(lines: &[Vec<[f32; 4]>]) -> f32 {
    let mid = |w: &[f32; 4]| ((w[0] + w[2]) / 2.0, (w[1] + w[3]) / 2.0, w[2] - w[0]);
    let (mut num, mut den) = (0.0, 0.0);
    for words in lines {
        let (sx, sy, sk) = (words.iter().map(mid))
            .fold((0.0, 0.0, 0.0), |(sx, sy, sk), (x, y, k)| {
                (sx + k * x, sy + k * y, sk + k)
            });
        if sk <= 0.0 {
            continue;
        }
        let (cx, cy) = (sx / sk, sy / sk);
        for (x, y, k) in words.iter().map(mid) {
            num += k * (x - cx) * (y - cy);
            den += k * (x - cx) * (x - cx);
        }
    }
    // Lines grouped wrongly can't tilt a page more than a scan is.
    if den > 0.0 {
        (num / den).clamp(-0.1, 0.1)
    } else {
        0.0
    }
}

/// A line's picture for the recognition model, and where it is on the page.
struct LinePicture {
    image: GrayImage,
    /// The line straightened, before scaling: its column x is the page's column `x0 + x`,
    /// and its letters are in its rows `rows`.
    crop: GrayImage,
    rows: std::ops::Range<u32>,
    /// The page's x at the picture's left edge, and page pixels per picture pixel.
    x0: f32,
    scale: f32,
}

impl LinePicture {
    /// The line in box `b` of `page` (left, top, right, bottom), sloping by `slope`,
    /// straightened, at least as tall as a line of letters `th` high and with a margin,
    /// white beyond the page: the model reads a line best with a little space around it.
    fn new(page: &GrayImage, b: [f32; 4], slope: f32, th: f32) -> Self {
        // The box is taller than the line by how far the line drops across it.
        let line = (b[3] - b[1] - slope.abs() * (b[2] - b[0])).max(1.0);
        let h = line.max(LINE_MIN * th);
        let m = 0.1 * h;
        let x0 = (b[0] - m).floor();
        let middle = (b[0] + b[2]) / 2.0;
        let y0 = ((b[1] + b[3] - h) / 2.0 - m).floor();
        let w = ((b[2] + m - x0).ceil() as u32).max(1);
        let rows = ((m + (h - line) / 2.0).round() as u32)..((m + (h + line) / 2.0).round() as u32);
        let h = ((h + 2.0 * m).ceil() as u32).max(1);
        let (pw, ph) = page.dimensions();
        let at =
            |px: i64, py: i64| match (0..pw as i64).contains(&px) && (0..ph as i64).contains(&py) {
                true => page.get_pixel(px as u32, py as u32).0[0] as f32,
                false => 255.0,
            };
        let crop = GrayImage::from_fn(w, h, |x, y| {
            let px = x0 as i64 + x as i64;
            let py = y0 + y as f32 + slope * (px as f32 - middle);
            let (row, f) = (py.floor(), py - py.floor());
            let v = at(px, row as i64) * (1.0 - f) + at(px, row as i64 + 1) * f;
            Luma([v.round() as u8])
        });
        let across = (w as f32 * LINE_HEIGHT as f32 / h as f32).round() as u32;
        let across = across.clamp(STEP, MAX_WIDTH);
        LinePicture {
            image: imageops::resize(&crop, across, LINE_HEIGHT, FilterType::Triangle),
            crop,
            rows,
            x0,
            scale: w as f32 / across as f32,
        }
    }

    /// The page's x at the middle of step `t` of the model's output.
    fn at(&self, t: usize) -> f32 {
        self.x0 + (t as f32 + 0.5) * STEP as f32 * self.scale
    }
}

/// Where the words found in a line (their boxes, with their margins) are apart: the
/// middle and width of each gap. Words closer than about a third of a letter's height
/// make one box.
fn gaps(found: &[[f32; 4]]) -> Vec<(f32, f32)> {
    let mut lefts: Vec<[f32; 4]> = found.to_vec();
    lefts.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut reach = f32::MIN;
    let mut gaps = Vec::new();
    for w in lefts {
        if reach > f32::MIN && w[0] > reach {
            gaps.push(((reach + w[0]) / 2.0, w[0] - reach));
        }
        reach = reach.max(w[2]);
    }
    gaps
}

/// The words of the line in box `b` of `page`, read as `chars` from its picture `pic`
/// (each character with its first and last step), with their boxes: down, the line's;
/// across, the ink between the cuts either side, or where the model read the word if
/// there is none. Two letters with one of the line's `gaps` between them are of two words.
fn line_words(
    page: &GrayImage,
    b: [f32; 4],
    pic: &LinePicture,
    chars: &[(char, usize, usize)],
    gaps: &[(f32, f32)],
) -> Vec<OcrWord> {
    // Each word with its first and last step, and the first and last step of the space
    // after each but the last.
    let mut words: Vec<(String, usize, usize)> = Vec::new();
    let mut spaces: Vec<(usize, usize)> = Vec::new();
    let mut space: Option<(usize, usize)> = None;
    // The model leaves out some spaces that show on the page, more on small print. A gap
    // with a space read beside it is that space, placed only roughly. One without parts
    // words only if about as wide as those: letters set wide apart, for emphasis or in
    // loose old print, have narrower gaps between them.
    let mut beside = Vec::new();
    for &(g, wide) in gaps {
        let between = |p: &[(char, usize, usize)]| pic.at(p[0].2) < g && g < pic.at(p[1].1);
        let Some(k) = chars.windows(2).position(between) else {
            continue;
        };
        let near = &chars[k.saturating_sub(1)..(k + 3).min(chars.len())];
        beside.push((k + 1, wide, near.iter().any(|c| c.0 == ' ')));
    }
    let mut spaced: Vec<f32> = beside.iter().filter(|g| g.2).map(|g| g.1).collect();
    spaced.sort_by(f32::total_cmp);
    let least = spaced.get(spaced.len() / 2).map_or(0.0, |m| 0.6 * m);
    let mut apart = vec![false; chars.len()];
    for (i, wide, space) in beside {
        apart[i] = !space && wide >= least;
    }
    for (i, &(c, t0, t1)) in chars.iter().enumerate() {
        if c == ' ' {
            space = Some((space.map_or(t0, |s| s.0), t1));
            continue;
        }
        let gap = (space.take()).or_else(|| words.last().filter(|_| apart[i]).map(|w| (w.2, t0)));
        match (words.last_mut(), gap) {
            (Some(w), None) => {
                w.0.push(c);
                w.2 = t1;
            }
            (last, gap) => {
                if let (Some(_), Some(s)) = (last, gap) {
                    spaces.push(s);
                }
                words.push((c.to_string(), t0, t1));
            }
        }
    }
    // The line's ink, straightened: the columns with a pixel darker than halfway between
    // its lightest and darkest.
    let (x0, x1) = (b[0].max(0.0) as u32, (b[2].ceil() as u32).min(page.width()));
    let crop = |x: u32| (x as i64 - pic.x0 as i64) as u32;
    let (mut lo, mut hi) = (255, 0);
    for y in pic.rows.clone() {
        for x in x0..x1 {
            let v = pic.crop.get_pixel(crop(x), y).0[0];
            (lo, hi) = (lo.min(v), hi.max(v));
        }
    }
    let mid = ((lo as u32 + hi as u32) / 2) as u8;
    let ink: Vec<bool> = (x0..x1)
        .map(|x| {
            hi > lo
                && pic
                    .rows
                    .clone()
                    .any(|y| pic.crop.get_pixel(crop(x), y).0[0] < mid)
        })
        .collect();
    let half = 0.5 * STEP as f32 * pic.scale;
    // Where each word ends and the next begins: the middle of the widest gap in the ink
    // from the one's last character to the other's first, or of the space if there is no
    // gap. The model places a space only roughly, at times in the next word's first letter,
    // and words whose boxes touch are read back as one.
    let cuts: Vec<f32> = (words.windows(2).zip(&spaces))
        .map(|(pair, &(s0, s1))| {
            let from = ((pic.at(pair[0].2) - half).max(x0 as f32) as u32).min(x1);
            let to = ((pic.at(pair[1].1) + half).ceil() as u32).clamp(from, x1);
            let (mut widest, mut run) = (None::<(u32, u32)>, None);
            for x in from..=to {
                match (x < to && !ink[(x - x0) as usize], run) {
                    (true, None) => run = Some(x),
                    (false, Some(r)) => {
                        if widest.is_none_or(|(a, z)| x - r > z - a) {
                            widest = Some((r, x));
                        }
                        run = None;
                    }
                    _ => {}
                }
            }
            match widest {
                Some((a, z)) => (a + z) as f32 / 2.0,
                None => (pic.at(s0) + pic.at(s1)) / 2.0,
            }
        })
        .collect();
    (words.into_iter().enumerate())
        .map(|(i, (text, t0, t1))| {
            let from = if i == 0 { b[0] } else { cuts[i - 1] };
            let to = cuts.get(i).copied().unwrap_or(b[2]);
            let cols = |x: &u32| (from..to).contains(&(*x as f32)) && ink[(x - x0) as usize];
            let first = (x0..x1).find(cols);
            let last = (x0..x1).rev().find(cols);
            let (left, right) = match (first, last) {
                (Some(f), Some(l)) => (f as f32, l as f32 + 1.0),
                _ => (pic.at(t0) - half, pic.at(t1) + half),
            };
            OcrWord {
                text,
                bbox: [left, b[1], right, b[3]],
            }
        })
        .collect()
}

/// `lines` without the words that land on the page's own words, `own`: words the page has
/// as text already. `own` is in displayed-page points (y down), the words' boxes in pixels
/// of the page drawn at `scale`. A line left without words goes.
fn without_own(lines: Vec<OcrLine>, own: &[[f32; 4]], scale: f32) -> Vec<OcrLine> {
    let area = |b: [f32; 4]| (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0);
    // Half the smaller box covered by the other.
    let on_own = |w: &OcrWord| {
        let b = w.bbox.map(|v| v / scale);
        own.iter().any(|o| {
            let (x0, y0) = (b[0].max(o[0]), b[1].max(o[1]));
            let both = area([x0, y0, b[2].min(o[2]), b[3].min(o[3])]);
            both > 0.0 && both >= 0.5 * area(b).min(area(*o))
        })
    };
    (lines.into_iter())
        .map(|mut l| {
            l.words.retain(|w| !on_own(w));
            l
        })
        .filter(|l| !l.words.is_empty())
        .collect()
}

/// The page's recognised text, as ids in `parsed.objs`: its recognised lines and the forms
/// drawing only recognised text, with how many lines they hold. Or why it can't be taken
/// out: a form drawing other content too holds some, or an inline picture would be lost.
fn recognised_objs(parsed: &Parsed) -> std::result::Result<(Vec<usize>, usize), String> {
    let other = "the recognised text is in a form that draws other content too, so it is left";
    let mut objs = Vec::new();
    let mut lines = 0;
    for f in &parsed.recognised_forms {
        let form = (parsed.objs.iter()).position(|o| o.kind == ObjectKind::Form && o.first == f.op);
        match form {
            Some(id) if !f.other => {
                objs.push(id);
                lines += f.lines;
            }
            _ => return Err(other.into()),
        }
    }
    for (id, o) in parsed.objs.iter().enumerate() {
        if o.text.as_ref().is_some_and(TextData::recognised) {
            objs.push(id);
            lines += 1;
        }
    }
    if !objs.is_empty() && parsed.ops.iter().any(|o| o.operator == "BI") {
        return Err("the page has an inline image that would be lost in rewriting it".into());
    }
    Ok((objs, lines))
}

/// The name an operator names first: a font for `Tf`, a picture or a form for `Do`.
fn name_operand(o: &lopdf::content::Operation) -> Option<Vec<u8>> {
    Some(o.operands.first()?.as_name().ok()?.to_vec())
}

impl Pdf {
    /// Recognise the text on `pages` (0-based) and add it as an invisible, searchable,
    /// selectable text layer. Pages with recognised text ([`TextData::recognised`]) are
    /// skipped, so running this twice adds nothing; [`Pdf::ocr_pages_again`] reads them
    /// again. Pages with text of their own are skipped too, unless a picture covers half the
    /// page or more, as on a scan with a page number or a stamp: those are read, and words
    /// that land on the page's own text are left out. Run [`Pdf::optimize_scans`] first if
    /// wanted: it does not straighten pages that have text.
    pub fn ocr_pages(&mut self, pages: &[usize], ocr: &Ocr) -> Result<Vec<(usize, OcrResult)>> {
        self.ocr_with(pages, ocr, false)
    }

    /// [`Pdf::ocr_pages`], with pages that have recognised text read again: their recognised
    /// text is replaced with what is read, or kept when nothing is. A page whose recognised
    /// text is in a form drawing other content too is skipped, as
    /// [`Pdf::remove_recognised_text`] leaves it.
    pub fn ocr_pages_again(
        &mut self,
        pages: &[usize],
        ocr: &Ocr,
    ) -> Result<Vec<(usize, OcrResult)>> {
        self.ocr_with(pages, ocr, true)
    }

    fn ocr_with(
        &mut self,
        pages: &[usize],
        ocr: &Ocr,
        again: bool,
    ) -> Result<Vec<(usize, OcrResult)>> {
        let ids = self.page_ids();
        if let Some(&bad) = pages.iter().find(|&&p| p >= ids.len()) {
            bail!("no page {}", bad + 1);
        }
        let mut todo = Vec::new();
        let mut targets = HashMap::new();
        let mut out: Vec<(usize, OcrResult)> = Vec::new();
        let mut asked = HashSet::new();
        // A page asked twice is read once.
        for &p in pages.iter().filter(|&&p| asked.insert(p)) {
            match self.ocr_target(ids[p], again) {
                Ok(t) => {
                    todo.push(p);
                    targets.insert(p, t);
                }
                Err(why) => out.push((p, OcrResult::Skipped(why))),
            }
        }
        let hp = render::load(self.drawing_snapshot()?, None)?;
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
        let mut read = Vec::new();
        for (p, r) in found {
            let own = &targets[&p];
            match r.map(|(lines, scale)| (without_own(lines, own, scale), scale)) {
                Err(why) => out.push((p, OcrResult::Skipped(why))),
                Ok((lines, _)) if lines.is_empty() => {
                    let why = match own.is_empty() {
                        true => "no text found",
                        false => "no text found besides the page's own",
                    };
                    out.push((p, OcrResult::Skipped(why.into())))
                }
                Ok((lines, scale)) => read.push((p, lines, scale)),
            }
        }
        let layers: Vec<_> = (read.iter())
            .map(|(p, lines, scale)| (ids[*p], &lines[..], *scale))
            .collect();
        self.atomically(|pdf| {
            // The text read replaces the recognised text a page had.
            for &(page, _, _) in layers.iter().filter(|_| again) {
                let parsed = pdf.parse_page_of(page)?;
                let (objs, _) = recognised_objs(&parsed).map_err(|why| anyhow!(why))?;
                if !objs.is_empty() {
                    pdf.strip_recognised(page, &parsed, &objs)?;
                }
            }
            pdf.add_text_layers(&layers)
        })?;
        for (p, lines, _) in read {
            let words = lines.iter().map(|l| l.words.len()).sum();
            let added = OcrResult::Added {
                lines: lines.len(),
                words,
            };
            out.push((p, added));
        }
        // In the order asked, a page asked twice where it was first.
        let first: HashMap<usize, usize> = (pages.iter().enumerate().rev())
            .map(|(i, &p)| (p, i))
            .collect();
        out.sort_by_key(|(p, _)| first.get(p).copied());
        Ok(out)
    }

    /// The boxes of the words of the page's own text, if the page with object `page` is to
    /// be read ([`Pdf::ocr_pages`]), in displayed-page points (y down); or why it isn't.
    /// `again` reads pages with recognised text again.
    fn ocr_target(
        &self,
        page: ObjectId,
        again: bool,
    ) -> std::result::Result<Vec<[f32; 4]>, String> {
        let parsed =
            (self.parse_page_of(page)).map_err(|e| format!("cannot read the page: {e}"))?;
        let lines: Vec<&TextData> = parsed.text_lines().collect();
        let recognised = lines.iter().any(|t| t.recognised());
        if recognised && !again {
            return Err("the page already has recognised text".into());
        }
        if recognised {
            recognised_objs(&parsed)?;
        }
        let own: Vec<&TextData> = lines.into_iter().filter(|t| !t.recognised()).collect();
        // Text shown that isn't read as lines is the page's own, and where it is isn't known.
        let unread = own.is_empty() && !recognised && parsed.shows_text();
        if unread || !(own.is_empty() || recognised || self.large_picture(page, &parsed)) {
            return Err("the page already has text".into());
        }
        let (vd, _, _) = self.vis_down(page);
        let to_vis = mx::inv(vd).ok_or("cannot read the page: degenerate page transform")?;
        let words = ordered_words(&own, to_vis);
        Ok(words.into_iter().map(|(w, _, _)| w.bbox).collect())
    }

    /// Take the recognised text off `pages` (0-based): invisible text as OCR adds it
    /// ([`TextData::recognised`]), in a page's content or in a form drawing nothing else (as
    /// ocrmypdf adds it). The pages look as they did, and keep their own text, invisible text
    /// that clips too. The fonts and forms only the recognised text used leave the pages'
    /// resources. A page whose recognised text is in a form drawing other content too is
    /// left as it is.
    pub fn remove_recognised_text(&mut self, pages: &[usize]) -> Result<Vec<(usize, OcrResult)>> {
        let ids = self.page_ids();
        if let Some(&bad) = pages.iter().find(|&&p| p >= ids.len()) {
            bail!("no page {}", bad + 1);
        }
        let mut asked = HashSet::new();
        self.atomically(|pdf| {
            let mut out = Vec::new();
            for &p in pages.iter().filter(|&&p| asked.insert(p)) {
                let parsed = match pdf.parse_page_of(ids[p]) {
                    Ok(parsed) => parsed,
                    Err(e) => {
                        let why = format!("cannot read the page: {e}");
                        out.push((p, OcrResult::Skipped(why)));
                        continue;
                    }
                };
                let r = match recognised_objs(&parsed) {
                    Err(why) => OcrResult::Skipped(why),
                    Ok((objs, _)) if objs.is_empty() => {
                        OcrResult::Skipped("the page has no recognised text".into())
                    }
                    // Nothing is changed when it fails.
                    Ok((objs, lines)) => match pdf.strip_recognised(ids[p], &parsed, &objs) {
                        Ok(()) => OcrResult::Removed { lines },
                        Err(e) => OcrResult::Skipped(format!("{e:#}")),
                    },
                };
                out.push((p, r));
            }
            Ok(out)
        })
    }

    /// Take `objs` out of the page with object `page`, its content `parsed`: its
    /// recognised text ([`recognised_objs`]). The fonts and forms only they used leave the
    /// page's resources, unless a form the page still draws has no resources of its own and
    /// may use them too.
    fn strip_recognised(&mut self, page: ObjectId, parsed: &Parsed, objs: &[usize]) -> Result<()> {
        let mut ops = Pdf::without_objects(parsed, objs)?;
        let mut res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
        let xobjects = self.resolved_dict(res.get(b"XObject").ok());
        let drawn: Vec<Vec<u8>> = (ops.iter().filter(|o| o.operator == "Do"))
            .filter_map(name_operand)
            .collect();
        let borrows = drawn.iter().any(|n| {
            let s = xobjects
                .get(n)
                .ok()
                .and_then(|o| self.doc.dereference(o).ok());
            s.and_then(|(_, s)| s.as_stream().ok()).is_some_and(|s| {
                s.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Form")
                    && !s.dict.has(b"Resources")
            })
        });
        if borrows {
            return self.commit(page, ops, parsed.depth_left, &parsed.inline);
        }
        // The fonts no line left is in, and the forms nothing draws now.
        let (mut fonts, mut forms) = (HashSet::new(), HashSet::new());
        for o in objs.iter().map(|&id| &parsed.objs[id]) {
            match &o.text {
                Some(t) => fonts.extend(t.spans.iter().map(|s| s.font.clone())),
                None => forms.extend(name_operand(&parsed.ops[o.first])),
            }
        }
        let kept = (parsed.objs.iter().enumerate()).filter(|(id, _)| !objs.contains(id));
        for t in kept.filter_map(|(_, o)| o.text.as_ref()) {
            for s in &t.spans {
                fonts.remove(&s.font);
            }
        }
        for n in &drawn {
            forms.remove(n);
        }
        // Nothing shown is in those fonts now, so what chooses them goes too.
        ops.retain(|o| o.operator != "Tf" || !name_operand(o).is_some_and(|n| fonts.contains(&n)));
        self.commit(page, ops, parsed.depth_left, &parsed.inline)?;
        let unused = [("Font", fonts), ("XObject", forms)];
        if unused.iter().all(|(_, u)| u.is_empty()) {
            return Ok(());
        }
        for (cat, unused) in unused.iter().filter(|(_, u)| !u.is_empty()) {
            let mut d = self.resolved_dict(res.get(cat.as_bytes()).ok());
            for n in unused {
                d.remove(n);
            }
            res.set(*cat, d);
        }
        self.doc.get_dictionary_mut(page)?.set("Resources", res);
        Ok(())
    }

    /// Each page's recognised lines added on top of it as invisible text: (page, its
    /// lines, the scale their word boxes are in). One pen writes them all, so its fonts hold
    /// what every page needs: Helvetica itself when the text is all Western European,
    /// Liberation Sans embedded when not.
    fn add_text_layers(&mut self, layers: &[(ObjectId, &[OcrLine], f32)]) -> Result<()> {
        if layers.is_empty() {
            return Ok(());
        }
        // Each word a line of the pen's, with the space after it but the last of a line's.
        let words: Vec<String> = (layers.iter().flat_map(|(_, lines, _)| *lines))
            .flat_map(|line| {
                let last = line.words.len().saturating_sub(1);
                (line.words.iter().enumerate()).map(move |(i, w)| match i < last {
                    true => format!("{} ", w.text),
                    false => w.text.clone(),
                })
            })
            .collect();
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        let setter = self.setter(FontChoice::default(), &words.concat());
        let pen = self.pen_with(&setter, &words)?;
        let mut shown = 0;
        for &(page, lines, scale) in layers {
            let names = self.font_names(page, FONT_PREFIX, pen.fonts())?;
            let layer = self.text_layer(page, lines, scale, &names[0], |text, size| {
                let shows = pen.content(shown, size, &names);
                shown += 1;
                (setter.width(text, size), shows)
            });
            self.wrap_contents(page, None, Some(layer))?;
        }
        Ok(())
    }

    /// Content stream drawing `lines` as invisible text. Word boxes are in pixels of
    /// the displayed page rendered at `scale`; `font` is the page's name for the pen's
    /// first font, and `word(text, size)` is how wide a word is at `size` and what shows it
    /// (the pen's next line).
    fn text_layer(
        &self,
        page: ObjectId,
        lines: &[OcrLine],
        scale: f32,
        font: &str,
        mut word: impl FnMut(&str, f32) -> (f32, String),
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
                // A word ends about a space before the next: a box can reach the next where
                // no gap shows in the ink (a rule under a form's line), and words without a
                // gap between them read back as one. Boxes overlapping further are of lines
                // grouped wrongly, which giving up most of a word doesn't mend.
                let before = (line.words.get(i + 1)).map(|n| n.bbox[0] - 0.25 * size * scale);
                let end = match before {
                    Some(e) if e >= (w.bbox[0] + w.bbox[2]) / 2.0 => w.bbox[2].min(e),
                    _ => w.bbox[2],
                };
                let width = (end - w.bbox[0]) / scale;
                let (natural, shows) = word(&w.text, size);
                let tz = (100.0 * width / natural.max(0.1)).clamp(10.0, 1000.0);
                let shows = shows.trim_end();
                let _ = writeln!(s, "{tz:.1} Tz 1 0 0 1 {x:.2} {baseline:.2} Tm {shows}");
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

/// Word boxes (left, top, right, bottom, in pixels), padded for recognition, and the
/// typical letter height (about the x-height) they were found with.
fn find_words(g: &GrayImage) -> (Vec<[f32; 4]>, f32) {
    let none = (Vec::new(), 0.0);
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
        return none;
    };
    let floor = 0.25 * heavy.h() as f32;
    let Some(th) = median(
        sized
            .iter()
            .map(|c| c.h())
            .filter(|&ch| ch as f32 >= floor)
            .collect(),
    ) else {
        return none;
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
        return none;
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
    // Pieces of one word are joined too: a broken letter, the loose loop of a g, or
    // letters whose facing sides share no row, so their gap was not filled. Their boxes
    // overlap, or would with the margin added below, and line grouping cannot chain
    // overlapping boxes: it reads one of them as a line of its own. Joined, they must
    // still fit in one line.
    words.sort_unstable_by_key(|c| c.x0);
    let mut joined: Vec<Comp> = Vec::with_capacity(words.len());
    for c in words {
        let near = joined.iter_mut().rev().find(|j| {
            let overlap_y = j.y1.min(c.y1).saturating_sub(j.y0.max(c.y0)) as f32;
            let spaced =
                c.x0 as f32 <= j.x1 as f32 + 0.1 * th && overlap_y >= 0.6 * c.h().max(j.h()) as f32;
            let pieces = (c.x0 as f32) < j.x1 as f32 + 0.3 * th
                && overlap_y >= 0.5 * c.h().min(j.h()) as f32
                && (j.y1.max(c.y1) - j.y0.min(c.y0)) as f32 <= 3.0 * th;
            spaced || pieces
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
    let boxes = words
        .into_iter()
        .map(|c| {
            [
                (c.x0 as f32 - pad_x).max(0.0),
                (c.y0 as f32 - pad_y).max(0.0),
                (c.x1 as f32 + pad_x).min(w as f32),
                (c.y1 as f32 + pad_y).min(h as f32),
            ]
        })
        .collect();
    (boxes, th)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pieces of one word make one box, and its neighbours keep theirs: line grouping
    /// reads one of two overlapping boxes as a line of its own, which split lines of old
    /// print and of large scans.
    #[test]
    fn pieces_of_a_word_make_one_box() {
        let mut img = GrayImage::from_pixel(400, 200, image::Luma([255]));
        let mut ink = |x0: u32, y0: u32, x1: u32, y1: u32| {
            for y in y0..y1 {
                for x in x0..x1 {
                    img.put_pixel(x, y, image::Luma([0]));
                }
            }
        };
        // Three lines of four-letter words; letters 12 by 20 pixels, 4 apart.
        for top in [40, 90, 140] {
            for start in [20, 100, 200, 280] {
                for x in (start..start + 64).step_by(16) {
                    // These two are drawn below.
                    if (top, x) != (90, 100) && (top, x) != (140, 148) {
                        ink(x, top, x + 12, top + 20);
                    }
                }
            }
        }
        // A letter whose lower part broke off and sits low, starting left of the word.
        ink(100, 90, 112, 99);
        ink(97, 101, 107, 115);
        // Facing sides that share no row: the gap between them is not filled.
        ink(148, 140, 160, 150);
        ink(163, 151, 175, 166);
        let (boxes, _) = find_words(&img);
        assert_eq!(boxes.len(), 12, "{boxes:?}");
        for (i, a) in boxes.iter().enumerate() {
            for b in &boxes[i + 1..] {
                let apart = a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1];
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    /// Words keep the gap between them when the model reads the space late, in the next
    /// word's first letter: their boxes touched, and the text read back ran them together.
    #[test]
    fn words_are_cut_in_the_gap_between_them() {
        let mut page = GrayImage::from_pixel(120, 40, image::Luma([255]));
        for x in (10..50).chain(66..100) {
            for y in 10..30 {
                page.put_pixel(x, y, image::Luma([0]));
            }
        }
        // Steps 8 pixels wide: their middles at 4, 12, 20 and so on.
        let pic = LinePicture {
            image: GrayImage::new(1, 1),
            crop: page.clone(),
            rows: 5..35,
            x0: 0.0,
            scale: 1.0,
        };
        let chars = [
            ('a', 2, 2),
            ('b', 5, 5),
            (' ', 8, 8),
            ('c', 9, 9),
            ('d', 11, 11),
        ];
        let words = line_words(&page, [0.0, 5.0, 120.0, 35.0], &pic, &chars, &[]);
        let read: Vec<(&str, f32, f32)> = (words.iter())
            .map(|w| (w.text.as_str(), w.bbox[0], w.bbox[2]))
            .collect();
        assert_eq!(read, [("ab", 10.0, 50.0), ("cd", 66.0, 100.0)]);
        // Without the space, the gap between the words found on the page parts them.
        let found = [[64.0, 8.0, 102.0, 32.0], [8.0, 8.0, 52.0, 32.0]];
        assert_eq!(gaps(&found), [(58.0, 12.0)]);
        let chars = [('a', 2, 2), ('b', 5, 5), ('c', 9, 9), ('d', 11, 11)];
        let words = line_words(&page, [0.0, 5.0, 120.0, 35.0], &pic, &chars, &gaps(&found));
        let read: Vec<(&str, f32, f32)> = (words.iter())
            .map(|w| (w.text.as_str(), w.bbox[0], w.bbox[2]))
            .collect();
        assert_eq!(read, [("ab", 10.0, 50.0), ("cd", 66.0, 100.0)]);
        // A gap beside a space read, even one placed late, is that space.
        let chars = [
            ('a', 2, 2),
            ('b', 5, 5),
            (' ', 8, 8),
            ('c', 9, 9),
            ('d', 10, 10),
            ('e', 11, 11),
        ];
        let text = |gaps: &[(f32, f32)]| -> Vec<String> {
            let words = line_words(&page, [0.0, 5.0, 120.0, 35.0], &pic, &chars, gaps);
            words.into_iter().map(|w| w.text).collect()
        };
        for g in [58.0, 80.0] {
            assert_eq!(text(&[(g, 12.0)]), ["ab", "cde"]);
        }
        // Another gap parts words only if about as wide as the space's.
        assert_eq!(text(&[(58.0, 12.0), (88.0, 4.0)]), ["ab", "cde"]);
        assert_eq!(text(&[(58.0, 12.0), (88.0, 10.0)]), ["ab", "cd", "e"]);
        // Words found overlapping or touching leave no gap.
        assert!(gaps(&[
            [8.0, 8.0, 60.0, 32.0],
            [60.0, 8.0, 102.0, 32.0],
            [30.0, 8.0, 50.0, 32.0]
        ])
        .is_empty());
    }

    /// The tilt of a page is the slope its lines' words follow.
    #[test]
    fn tilt_follows_the_words() {
        let line = |y: f32| -> Vec<[f32; 4]> {
            (0..5)
                .map(|i| {
                    let x = 20.0 + 60.0 * i as f32;
                    let mid = y + 0.03 * (x + 20.0);
                    [x, mid - 10.0, x + 40.0, mid + 10.0]
                })
                .collect()
        };
        let slope = tilt(&[line(100.0), line(200.0), vec![[10.0, 5.0, 30.0, 25.0]]]);
        assert!((slope - 0.03).abs() < 1e-4, "{slope}");
        assert_eq!(tilt(&[vec![[10.0, 5.0, 30.0, 25.0]]]), 0.0);
    }

    /// A line of a tilted page is read straightened, its neighbours left out: a picture
    /// of its box was as tall as its drop across the page, its letters small.
    #[test]
    fn tilted_lines_are_straightened() {
        let mut page = GrayImage::from_pixel(400, 300, image::Luma([255]));
        for x in 0..400 {
            let top = 100.0 + 0.05 * x as f32;
            for y in top.ceil() as u32..(top + 40.0).floor() as u32 {
                page.put_pixel(x, y, image::Luma([0]));
            }
        }
        let pic = LinePicture::new(&page, [0.0, 100.0, 400.0, 160.0], 0.05, 10.0);
        let (h, middle) = (pic.crop.height(), (pic.rows.start + pic.rows.end) / 2);
        assert_eq!((pic.x0, h, pic.rows.clone()), (-4.0, 48, 4..44));
        // The page's columns 10 to 390.
        for x in 14..394 {
            let at = |y| pic.crop.get_pixel(x, y).0[0];
            assert!(
                at(middle) < 50 && at(0) > 200 && at(h - 1) > 200,
                "column {x}"
            );
        }
    }

    /// A blank page given `lines` as its text layer, as if recognised: words 8 points a
    /// letter, lines 14 points high.
    fn layered(lines: &[&str]) -> Pdf {
        let mut p = Pdf::new();
        p.insert_blank(0, 400.0, 200.0).unwrap();
        let lines: Vec<OcrLine> = (lines.iter().enumerate())
            .map(|(i, l)| {
                let top = 20.0 + 30.0 * i as f32;
                let mut x = 20.0;
                let words = (l.split(' '))
                    .map(|w| {
                        let wide = 8.0 * w.chars().count() as f32;
                        let bbox = [x, top, x + wide, top + 14.0];
                        x += wide + 6.0;
                        OcrWord {
                            text: w.into(),
                            bbox,
                        }
                    })
                    .collect();
                OcrLine { words }
            })
            .collect();
        let page = p.page_ids()[0];
        p.add_text_layers(&[(page, &lines, 1.0)]).unwrap();
        p
    }

    /// The base fonts of page 1.
    fn base_fonts(p: &Pdf) -> Vec<String> {
        let fonts = p.doc.get_page_fonts(p.page_ids()[0]).unwrap();
        (fonts.values())
            .map(|f| String::from_utf8_lossy(f.get(b"BaseFont").unwrap().as_name().unwrap()).into())
            .collect()
    }

    /// Letters beyond Western European are written, in Liberation Sans embedded: the layer
    /// was written in WinAnsi, which has no ł, ř or ő, so they read as "?".
    #[test]
    fn letters_beyond_western_european_are_written() {
        let lines = [
            "Zażółć gęślą jaźń",
            "Příliš žluťoučký kůň",
            "Árvíztűrő tükörfúrógép",
            "„Cytat” – koniec",
        ];
        let p = layered(&lines);
        let text = p.page_text(0);
        let read: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(read, lines, "{text}");
        let fonts = base_fonts(&p);
        assert!(
            fonts.iter().all(|f| f.ends_with("+LiberationSans")),
            "{fonts:?}"
        );
    }

    /// Words whose boxes touch are written a space apart: without a gap between them,
    /// they read back as one word.
    #[test]
    fn words_whose_boxes_touch_are_written_apart() {
        let mut p = Pdf::new();
        p.insert_blank(0, 400.0, 200.0).unwrap();
        let word = |text: &str, x0, x1| OcrWord {
            text: text.into(),
            bbox: [x0, 20.0, x1, 34.0],
        };
        let words = vec![word("Saturday,", 20.0, 92.0), word("April", 92.0, 132.0)];
        let page = p.page_ids()[0];
        p.add_text_layers(&[(page, &[OcrLine { words }], 1.0)])
            .unwrap();
        let first = p.find_text_matches(0, "Saturday,").unwrap();
        let second = p.find_text_matches(0, "April").unwrap();
        let (end, start) = (first[0][0][2], second[0][0][0]);
        // A quarter of the size, 14 / 1.15 points.
        assert!(
            (start - 92.0).abs() < 0.1 && start - end > 3.0,
            "{end} to {start}"
        );
    }

    /// Western European text is written in Helvetica itself, embedding nothing.
    #[test]
    fn western_european_text_is_written_in_helvetica() {
        let lines = ["Le café ouvre à huit heures", "“Quoted” – done"];
        let p = layered(&lines);
        let text = p.page_text(0);
        let read: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(read, lines, "{text}");
        assert_eq!(base_fonts(&p), ["Helvetica"]);
    }
}
