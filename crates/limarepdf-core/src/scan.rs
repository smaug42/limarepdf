//! Optimising scanned pages: pages that are one image covering the page.
//!
//! The page's image XObject is decoded, cleaned up (deskew, despeckle), downsampled and
//! re-encoded in place: CCITT G4 for black and white, JPEG for greyscale and colour. The
//! page content is not touched, so the image keeps its position and size on the page.

use crate::imgs::jpeg_components;
use crate::{ObjectKind, Pdf, Result};
use image::imageops::FilterType;
use image::{GrayImage, RgbImage};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use rayon::prelude::*;
use std::collections::HashMap;

/// Largest decoded image accepted, in bytes.
const MAX_DECODED: usize = 1 << 30;

/// How scanned pages are re-encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanMode {
    /// Per page: black and white for text-like pages, else greyscale or colour.
    Auto,
    BlackWhite,
    Gray,
    Color,
}

#[derive(Clone, Copy, Debug)]
pub struct ScanOptions {
    pub mode: ScanMode,
    /// Greyscale and colour images above this resolution (dots per inch) are downsampled
    /// to it. Black-and-white images keep their resolution.
    pub max_dpi: f32,
    /// JPEG quality for greyscale and colour images, 1 to 100.
    pub jpeg_quality: u8,
    /// Straighten pages tilted by up to 5 degrees. Pages with text on them (an OCR layer,
    /// for example) are not straightened, since the text would no longer line up.
    pub deskew: bool,
    /// Remove isolated specks from black-and-white images.
    pub despeckle: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            mode: ScanMode::Auto,
            max_dpi: 200.0,
            jpeg_quality: 75,
            deskew: true,
            despeckle: true,
        }
    }
}

/// What happened to one page.
#[derive(Clone, Debug, PartialEq)]
pub enum ScanResult {
    Optimized {
        mode: ScanMode,
        /// Encoded image size in bytes, before and after.
        before: usize,
        after: usize,
        /// Tilt that was corrected, in degrees. Positive means the scan was tilted
        /// clockwise (lines ran down to the right).
        deskewed: Option<f32>,
    },
    /// The page was left as it was, for the reason given.
    Skipped(String),
}

/// A scanned page's image, found on the page.
struct Target {
    id: ObjectId,
    dpi: f32,
    has_text: bool,
}

/// Decoded pixels. `Bits` holds 0 (black) or 255 (white).
pub(crate) enum Img {
    Bits(GrayImage),
    Gray(GrayImage),
    Rgb(RgbImage),
}

impl Pdf {
    /// Re-encode the scanned pages among `pages` (0-based). Pages that are not one image
    /// covering the page, or whose image cannot be decoded, are skipped and reported. An
    /// image is only replaced when the result is smaller, or when it was straightened or
    /// despeckled.
    pub fn optimize_scans(
        &mut self,
        pages: &[usize],
        opts: &ScanOptions,
    ) -> Result<Vec<(usize, ScanResult)>> {
        let mut found: Vec<(usize, std::result::Result<Target, String>)> = Vec::new();
        for &p in pages {
            if p >= self.page_count() {
                anyhow::bail!("no page {}", p + 1);
            }
            found.push((p, self.scan_target(p)));
        }
        // One result per distinct image; a shared image is processed once.
        let mut first_use: HashMap<ObjectId, usize> = HashMap::new();
        let jobs: Vec<(ObjectId, f32, bool)> = found
            .iter()
            .filter_map(|(p, t)| t.as_ref().ok().map(|t| (*p, t)))
            .filter(|(p, t)| first_use.insert(t.id, *p).is_none())
            .map(|(_, t)| (t.id, t.dpi, t.has_text))
            .collect();
        let doc = &self.doc;
        let done: HashMap<ObjectId, std::result::Result<(Stream, ScanResult), String>> = jobs
            .par_iter()
            .map(|&(id, dpi, has_text)| {
                let r = match doc.get_object(id).and_then(Object::as_stream) {
                    Ok(s) => process(doc, s, dpi, has_text, opts),
                    Err(e) => Err(e.to_string()),
                };
                (id, r)
            })
            .collect();
        let mut out = Vec::new();
        for (p, t) in found {
            let result = match t {
                Err(why) => ScanResult::Skipped(why),
                Ok(t) => match &done[&t.id] {
                    Err(why) => ScanResult::Skipped(why.clone()),
                    Ok(_) if first_use[&t.id] != p => {
                        ScanResult::Skipped(format!("same image as page {}", first_use[&t.id] + 1))
                    }
                    Ok((_, r)) => r.clone(),
                },
            };
            out.push((p, result));
        }
        for (id, r) in done {
            if let Ok((s, _)) = r {
                self.doc.objects.insert(id, Object::Stream(s));
            }
        }
        Ok(out)
    }

    /// The image that makes up a scanned page, or why the page does not qualify.
    fn scan_target(&self, page_index: usize) -> std::result::Result<Target, String> {
        let page = self.page_id_at(page_index).map_err(|e| e.to_string())?;
        let parsed = self.parse_page(page_index).map_err(|e| e.to_string())?;
        if parsed
            .objs
            .iter()
            .any(|o| matches!(o.kind, ObjectKind::Path | ObjectKind::Form))
        {
            return Err("the page has content other than an image".into());
        }
        let images: Vec<_> = parsed
            .objs
            .iter()
            .filter(|o| o.kind == ObjectKind::Image)
            .collect();
        let [img] = images[..] else {
            return Err("the page is not a single image".into());
        };
        if img.inline {
            return Err("the image is an inline image".into());
        }
        let pb = self.page_box(page);
        let b = img.bbox;
        let overlap = (b[2].min(pb[2]) - b[0].max(pb[0])).max(0.0)
            * (b[3].min(pb[3]) - b[1].max(pb[1])).max(0.0);
        if overlap < 0.8 * (pb[2] - pb[0]) * (pb[3] - pb[1]) {
            return Err("the image does not cover the page".into());
        }
        let name = parsed.ops[img.first..=img.last]
            .iter()
            .find(|o| o.operator == "Do")
            .and_then(|o| o.operands.first())
            .and_then(|o| o.as_name().ok())
            .ok_or("cannot find the image")?;
        let id = self
            .inherited(page, b"Resources")
            .and_then(|r| self.doc.dereference(&r).ok()?.1.as_dict().ok().cloned())
            .and_then(|r| {
                let xo = r.get(b"XObject").ok()?;
                let xo = self.doc.dereference(xo).ok()?.1.as_dict().ok()?;
                xo.get(name).ok()?.as_reference().ok()
            })
            .ok_or("cannot find the image")?;
        let width_px = self
            .doc
            .get_object(id)
            .and_then(Object::as_stream)
            .ok()
            .and_then(|s| s.dict.get(b"Width").ok()?.as_i64().ok())
            .ok_or("the image has no width")?;
        let width_in = img.ctm[0].hypot(img.ctm[1]) / 72.0;
        Ok(Target {
            id,
            dpi: width_px as f32 / width_in.max(1e-3),
            has_text: parsed.shows_text(),
        })
    }
}

fn process(
    doc: &Document,
    s: &Stream,
    dpi: f32,
    has_text: bool,
    o: &ScanOptions,
) -> std::result::Result<(Stream, ScanResult), String> {
    let img = decode(doc, s)?;
    let before = s.content.len();
    let wanted = match o.mode {
        ScanMode::Auto => auto_mode(&img),
        m => m,
    };
    // Colour cannot be made from grey, nor grey from black and white.
    let mode = match (&img, wanted) {
        (Img::Bits(_), _) => ScanMode::BlackWhite,
        (Img::Gray(_), ScanMode::Color) => ScanMode::Gray,
        _ => wanted,
    };
    let mut work = match (img, mode) {
        (Img::Bits(b), _) => Img::Bits(b),
        (img, ScanMode::BlackWhite) => Img::Bits(binarise(&to_gray(img))),
        (img, ScanMode::Gray) => Img::Gray(to_gray(img)),
        (img, _) => img,
    };
    let mut changed = false;
    let mut deskewed = None;
    if o.deskew && !has_text {
        if let Some(a) = estimate_skew(&work) {
            work = rotate(work, a);
            deskewed = Some(a.to_degrees());
            changed = true;
        }
    }
    if o.despeckle {
        if let Img::Bits(b) = &mut work {
            let max_area = ((dpi / 300.0).powi(2) * 4.0).round().max(1.0) as usize;
            changed |= despeckle(b, max_area) > 0;
        }
    }
    if dpi > o.max_dpi * 1.05 {
        let f = o.max_dpi / dpi;
        let size = |w: u32, h: u32| {
            (
                ((w as f32 * f).round() as u32).max(1),
                ((h as f32 * f).round() as u32).max(1),
            )
        };
        work = match work {
            Img::Gray(g) => {
                let (w, h) = size(g.width(), g.height());
                Img::Gray(image::imageops::resize(&g, w, h, FilterType::Triangle))
            }
            Img::Rgb(c) => {
                let (w, h) = size(c.width(), c.height());
                Img::Rgb(image::imageops::resize(&c, w, h, FilterType::Triangle))
            }
            bits => bits,
        };
    }
    let new = encode(s, work, o.jpeg_quality)?;
    let after = new.content.len();
    if after >= before && !changed {
        return Err(format!(
            "re-encoding would not make it smaller ({before} to {after} bytes)"
        ));
    }
    Ok((
        new,
        ScanResult::Optimized {
            mode,
            before,
            after,
            deskewed,
        },
    ))
}

// ------------------------------------------------------------------ decode

fn int(s: &Stream, key: &[u8]) -> Option<i64> {
    s.dict.get(key).ok()?.as_i64().ok()
}

/// Number of colour components of an image colour space, or `None` if unsupported.
fn components(doc: &Document, cs: &Object) -> Option<usize> {
    let cs = doc.dereference(cs).ok()?.1;
    match cs {
        Object::Name(n) => match n.as_slice() {
            b"DeviceGray" | b"CalGray" | b"G" => Some(1),
            b"DeviceRGB" | b"CalRGB" | b"RGB" => Some(3),
            b"DeviceCMYK" | b"CMYK" => Some(4),
            _ => None,
        },
        Object::Array(a) => match a.first()?.as_name().ok()? {
            b"ICCBased" => {
                let s = doc.dereference(a.get(1)?).ok()?.1.as_stream().ok()?;
                let n = s.dict.get(b"N").ok()?.as_i64().ok()?;
                matches!(n, 1 | 3 | 4).then_some(n as usize)
            }
            b"CalGray" => Some(1),
            b"CalRGB" => Some(3),
            _ => None,
        },
        _ => None,
    }
}

fn decode(doc: &Document, s: &Stream) -> std::result::Result<Img, String> {
    let d = &s.dict;
    let truthy = |k: &[u8]| d.get(k).ok().and_then(|o| o.as_bool().ok()) == Some(true);
    if d.has(b"SMask") || d.has(b"Mask") || d.has(b"Decode") || truthy(b"ImageMask") {
        return Err("the image has a mask or a Decode array".into());
    }
    let (w, h) = match (int(s, b"Width"), int(s, b"Height")) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w as u32, h as u32),
        _ => return Err("the image has no size".into()),
    };
    let filters: Vec<&[u8]> = s.filters().unwrap_or_default();
    match filters.as_slice() {
        [b"DCTDecode"] => {
            if jpeg_components(&s.content) == Some(4) {
                return Err("CMYK JPEG images are not optimised".into());
            }
            let img = image::load_from_memory_with_format(&s.content, image::ImageFormat::Jpeg)
                .map_err(|e| format!("cannot decode the JPEG: {e}"))?;
            Ok(if img.color().has_color() {
                Img::Rgb(img.to_rgb8())
            } else {
                Img::Gray(img.to_luma8())
            })
        }
        [b"CCITTFaxDecode"] => decode_ccitt(s, w, h),
        [.., b"JBIG2Decode"] => Err("JBIG2 images are already compact".into()),
        [.., b"JPXDecode"] => Err("JPEG 2000 images are not supported".into()),
        [.., b"DCTDecode" | b"CCITTFaxDecode"] => Err("unsupported filter chain".into()),
        _ => {
            let raw = s
                .decompressed_content_with_limit(MAX_DECODED)
                .map_err(|e| format!("cannot decompress the image: {e}"))?;
            let bpc = int(s, b"BitsPerComponent").unwrap_or(8);
            let n = d
                .get(b"ColorSpace")
                .ok()
                .and_then(|cs| components(doc, cs))
                .ok_or("unsupported colour space")?;
            decode_raw(&raw, w, h, n, bpc)
        }
    }
}

fn decode_raw(raw: &[u8], w: u32, h: u32, n: usize, bpc: i64) -> std::result::Result<Img, String> {
    let (wu, hu) = (w as usize, h as usize);
    let short = || "the image data is too short".to_string();
    match (n, bpc) {
        (1, 1) => {
            let stride = wu.div_ceil(8);
            if raw.len() < stride * hu {
                return Err(short());
            }
            Ok(Img::Bits(GrayImage::from_fn(w, h, |x, y| {
                let byte = raw[y as usize * stride + x as usize / 8];
                let white = byte & (0x80 >> (x % 8)) != 0;
                image::Luma([if white { 255 } else { 0 }])
            })))
        }
        (_, 8 | 16) => {
            let step = bpc as usize / 8;
            if raw.len() < wu * hu * n * step {
                return Err(short());
            }
            let s = |i: usize| raw[i * step];
            Ok(match n {
                1 => Img::Gray(GrayImage::from_fn(w, h, |x, y| {
                    image::Luma([s(y as usize * wu + x as usize)])
                })),
                3 => Img::Rgb(RgbImage::from_fn(w, h, |x, y| {
                    let i = (y as usize * wu + x as usize) * 3;
                    image::Rgb([s(i), s(i + 1), s(i + 2)])
                })),
                _ => Img::Rgb(RgbImage::from_fn(w, h, |x, y| {
                    // Naive CMYK to RGB; scans are rarely CMYK.
                    let i = (y as usize * wu + x as usize) * 4;
                    let k = s(i + 3) as u16;
                    let c = |v: u8| 255 - (v as u16 + k).min(255) as u8;
                    image::Rgb([c(s(i)), c(s(i + 1)), c(s(i + 2))])
                })),
            })
        }
        _ => Err(format!("{bpc}-bit images are not supported")),
    }
}

pub(crate) fn decode_ccitt(s: &Stream, w: u32, h: u32) -> std::result::Result<Img, String> {
    let parms = s.dict.get(b"DecodeParms").and_then(Object::as_dict).ok();
    let p = |k: &[u8]| parms.and_then(|d| d.get(k).ok()?.as_i64().ok());
    let aligned = parms
        .and_then(|d| d.get(b"EncodedByteAlign").ok()?.as_bool().ok())
        .unwrap_or(false);
    if p(b"K").unwrap_or(0) >= 0 || aligned {
        return Err("only CCITT Group 4 images are supported".into());
    }
    let cols = p(b"Columns").unwrap_or(1728) as u32;
    if cols != w || w > u16::MAX as u32 || h > u16::MAX as u32 {
        return Err("unsupported CCITT image size".into());
    }
    let mut px = Vec::with_capacity((w * h) as usize);
    let data = s.content.iter().copied();
    fax::decoder::decode_g4(data, w as u16, Some(h as u16), |line| {
        px.extend(fax::decoder::pels(line, w as u16).map(|c| match c {
            fax::Color::Black => 0u8,
            fax::Color::White => 255,
        }));
    })
    .ok_or("cannot decode the CCITT image")?;
    px.resize((w * h) as usize, 255);
    GrayImage::from_raw(w, h, px)
        .map(Img::Bits)
        .ok_or_else(|| "cannot decode the CCITT image".into())
}

// --------------------------------------------------------------- analysis

fn to_gray(img: Img) -> GrayImage {
    match img {
        Img::Bits(g) | Img::Gray(g) => g,
        Img::Rgb(c) => image::DynamicImage::ImageRgb8(c).to_luma8(),
    }
}

fn auto_mode(img: &Img) -> ScanMode {
    let gray = match img {
        Img::Bits(_) => return ScanMode::BlackWhite,
        Img::Gray(g) => g.clone(),
        Img::Rgb(c) => {
            // Colour when more than 0.2% of pixels are clearly coloured; paper tint
            // and scanner noise stay below the 48-level spread.
            let coloured = c
                .pixels()
                .filter(|p| {
                    let (mx, mn) = (p.0.iter().max().unwrap(), p.0.iter().min().unwrap());
                    mx - mn > 48
                })
                .count();
            if coloured * 500 > c.pixels().len() {
                return ScanMode::Color;
            }
            image::DynamicImage::ImageRgb8(c.clone()).to_luma8()
        }
    };
    // Text-like when under 10% of pixels are mid-tones (edges of glyphs); photographs
    // and shaded drawings have far more.
    let mid = gray
        .pixels()
        .filter(|p| (64..192).contains(&p.0[0]))
        .count();
    if mid * 10 < gray.pixels().len() {
        ScanMode::BlackWhite
    } else {
        ScanMode::Gray
    }
}

/// Black and white by Sauvola's local threshold, so shadows and uneven lighting (phone
/// photos of paper) do not turn whole regions black or white. A pixel is black when it
/// is darker than `mean * (1 + K * (sd / 128 - 1))` of its neighbourhood, about a
/// sixteenth of the image's long side across. The local mean and standard deviation are
/// computed on a coarse grid and interpolated, which keeps memory small on big pages.
pub(crate) fn binarise(g: &GrayImage) -> GrayImage {
    // K = 0.4 measured best across the OCR test pages (0.2 to 0.7 tried); higher values
    // leave less texture and fewer specks.
    const K: f32 = 0.4;
    const GRID: u32 = 512;
    const RADIUS: usize = 16;
    let (w, h) = g.dimensions();
    let f = w.max(h).div_ceil(GRID).max(1);
    let (sw, sh) = (w.div_ceil(f) as usize, h.div_ceil(f) as usize);
    // Per cell: sum, sum of squares, count.
    let mut cells = vec![(0f64, 0f64, 0f64); sw * sh];
    for (x, y, p) in g.enumerate_pixels() {
        let v = p.0[0] as f64;
        let c = &mut cells[(y / f) as usize * sw + (x / f) as usize];
        c.0 += v;
        c.1 += v * v;
        c.2 += 1.0;
    }
    // Box-filter the cells with a summed-area table.
    let mut sat = vec![(0f64, 0f64, 0f64); (sw + 1) * (sh + 1)];
    for y in 0..sh {
        for x in 0..sw {
            let c = cells[y * sw + x];
            let (a, b, d) = (
                sat[y * (sw + 1) + x + 1],
                sat[(y + 1) * (sw + 1) + x],
                sat[y * (sw + 1) + x],
            );
            sat[(y + 1) * (sw + 1) + x + 1] = (
                c.0 + a.0 + b.0 - d.0,
                c.1 + a.1 + b.1 - d.1,
                c.2 + a.2 + b.2 - d.2,
            );
        }
    }
    let mut thresh = vec![0f32; sw * sh];
    for y in 0..sh {
        for x in 0..sw {
            let (x0, x1) = (x.saturating_sub(RADIUS), (x + RADIUS + 1).min(sw));
            let (y0, y1) = (y.saturating_sub(RADIUS), (y + RADIUS + 1).min(sh));
            let at = |xx: usize, yy: usize| sat[yy * (sw + 1) + xx];
            let (a, b, c, d) = (at(x1, y1), at(x0, y1), at(x1, y0), at(x0, y0));
            let n = a.2 - b.2 - c.2 + d.2;
            let mean = (a.0 - b.0 - c.0 + d.0) / n;
            let var = ((a.1 - b.1 - c.1 + d.1) / n - mean * mean).max(0.0);
            thresh[y * sw + x] = (mean as f32) * (1.0 + K * (var.sqrt() as f32 / 128.0 - 1.0));
        }
    }
    // Interpolate the threshold between cell centres.
    let t_at = |x: u32, y: u32| -> f32 {
        let fx = ((x as f32 + 0.5) / f as f32 - 0.5).clamp(0.0, (sw - 1) as f32);
        let fy = ((y as f32 + 0.5) / f as f32 - 0.5).clamp(0.0, (sh - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(sw - 1), (y0 + 1).min(sh - 1));
        let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
        let t = |xx: usize, yy: usize| thresh[yy * sw + xx];
        (t(x0, y0) * (1.0 - ax) + t(x1, y0) * ax) * (1.0 - ay)
            + (t(x0, y1) * (1.0 - ax) + t(x1, y1) * ax) * ay
    };
    let mut out = GrayImage::new(w, h);
    let rows = out.par_chunks_mut(w as usize).enumerate();
    rows.for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let v = g.get_pixel(x as u32, y as u32).0[0] as f32;
            let black = v <= t_at(x as u32, y as u32);
            *px = if black { 0 } else { 255 };
        }
    });
    out
}

/// Slope of text lines in radians (y grows downwards), found by the projection-profile
/// method: dark pixels are binned by `y - x * tan(a)`, and the angle whose bins are most
/// uneven wins. `None` when the page is level or there is too little to measure.
fn estimate_skew(img: &Img) -> Option<f32> {
    let bits;
    let b = match img {
        Img::Bits(b) => b,
        other => {
            bits = binarise(&to_gray(match other {
                Img::Gray(g) => Img::Gray(g.clone()),
                Img::Rgb(c) => Img::Rgb(c.clone()),
                Img::Bits(_) => unreachable!(),
            }));
            &bits
        }
    };
    let (w, h) = b.dimensions();
    let mut dark: Vec<(f32, f32)> = Vec::new();
    for (x, y, p) in b.enumerate_pixels() {
        if p.0[0] == 0 {
            dark.push((x as f32, y as f32));
        }
    }
    let n = dark.len();
    if n < 500 || n * 2 > (w * h) as usize {
        return None;
    }
    let stride = (n / 200_000).max(1);
    let sample: Vec<(f32, f32)> = dark.into_iter().step_by(stride).collect();
    let span = (w as f32 * 5f32.to_radians().tan()).ceil() as i64;
    let score = |a: f32| -> f64 {
        let t = a.tan();
        let mut bins = vec![0u32; h as usize + 2 * span as usize + 1];
        for &(x, y) in &sample {
            let i = (y - x * t).round() as i64 + span;
            if let Some(c) = bins.get_mut(i as usize) {
                *c += 1;
            }
        }
        bins.iter().map(|&c| (c as f64).powi(2)).sum()
    };
    let search = |lo: f32, hi: f32, step: f32| {
        let mut best = (0.0f32, f64::MIN);
        let mut a = lo;
        while a <= hi + 1e-6 {
            let s = score(a.to_radians());
            if s > best.1 {
                best = (a, s);
            }
            a += step;
        }
        best.0
    };
    let coarse = search(-5.0, 5.0, 0.25);
    let fine = search(coarse - 0.25, coarse + 0.25, 0.025);
    // Require a clear gain over leaving the page as it is.
    (fine.abs() >= 0.1 && score(fine.to_radians()) > score(0.0) * 1.02).then(|| fine.to_radians())
}

/// Turn the image so lines of slope `a` become level. The size stays the same; uncovered
/// corners are white.
fn rotate(img: Img, a: f32) -> Img {
    fn turn(src: &[u8], w: u32, h: u32, ch: usize, a: f32, nearest: bool) -> Vec<u8> {
        let (s, c) = a.sin_cos();
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        let (wi, hi) = (w as usize, h as usize);
        let mut out = vec![255u8; src.len()];
        let rows = out.par_chunks_mut(wi * ch).enumerate();
        rows.for_each(|(y, row)| {
            for x in 0..wi {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let sx = cx + c * dx - s * dy - 0.5;
                let sy = cy + s * dx + c * dy - 0.5;
                let px = &mut row[x * ch..x * ch + ch];
                if nearest {
                    let (ix, iy) = (sx.round() as isize, sy.round() as isize);
                    if ix >= 0 && iy >= 0 && (ix as usize) < wi && (iy as usize) < hi {
                        let i = (iy as usize * wi + ix as usize) * ch;
                        px.copy_from_slice(&src[i..i + ch]);
                    }
                    continue;
                }
                let (x0, y0) = (sx.floor(), sy.floor());
                let (fx, fy) = (sx - x0, sy - y0);
                for k in 0..ch {
                    let at = |xx: f32, yy: f32| -> f32 {
                        if xx < 0.0 || yy < 0.0 || xx >= w as f32 || yy >= h as f32 {
                            255.0
                        } else {
                            src[(yy as usize * wi + xx as usize) * ch + k] as f32
                        }
                    };
                    let v = at(x0, y0) * (1.0 - fx) * (1.0 - fy)
                        + at(x0 + 1.0, y0) * fx * (1.0 - fy)
                        + at(x0, y0 + 1.0) * (1.0 - fx) * fy
                        + at(x0 + 1.0, y0 + 1.0) * fx * fy;
                    px[k] = v.round() as u8;
                }
            }
        });
        out
    }
    match img {
        Img::Bits(b) => {
            let (w, h) = b.dimensions();
            Img::Bits(GrayImage::from_raw(w, h, turn(b.as_raw(), w, h, 1, a, true)).unwrap())
        }
        Img::Gray(g) => {
            let (w, h) = g.dimensions();
            Img::Gray(GrayImage::from_raw(w, h, turn(g.as_raw(), w, h, 1, a, false)).unwrap())
        }
        Img::Rgb(c) => {
            let (w, h) = c.dimensions();
            Img::Rgb(RgbImage::from_raw(w, h, turn(c.as_raw(), w, h, 3, a, false)).unwrap())
        }
    }
}

/// Whiten groups of touching black pixels (8-connected) no larger than `max_area`.
/// Returns the number of pixels changed.
fn despeckle(b: &mut GrayImage, max_area: usize) -> usize {
    let (w, h) = (b.width() as usize, b.height() as usize);
    let mut seen = vec![false; w * h];
    let mut changed = 0;
    let mut stack = Vec::new();
    let mut group = Vec::new();
    for start in 0..w * h {
        if seen[start] || b.as_raw()[start] != 0 {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        group.clear();
        while let Some(i) = stack.pop() {
            group.push(i);
            let (x, y) = (i % w, i / w);
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if !seen[j] && b.as_raw()[j] == 0 {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            if group.len() > max_area {
                // Too big to be a speck: keep marking it seen, but stop collecting.
                group.truncate(max_area + 1);
            }
        }
        if group.len() <= max_area {
            let raw: &mut [u8] = b;
            for &i in &group {
                raw[i] = 255;
            }
            changed += group.len();
        }
    }
    changed
}

// ----------------------------------------------------------------- encode

/// A new image stream for `img`, keeping the old stream's other entries (such as /OC or
/// /Metadata).
fn encode(old: &Stream, img: Img, quality: u8) -> std::result::Result<Stream, String> {
    let mut dict = old.dict.clone();
    for k in [
        "Filter",
        "DecodeParms",
        "ColorSpace",
        "BitsPerComponent",
        "Width",
        "Height",
        "Length",
        "DL",
    ] {
        dict.remove(k.as_bytes());
    }
    let (w, h, content) = match img {
        Img::Bits(b) => {
            let (w, h) = b.dimensions();
            if w > u16::MAX as u32 {
                return Err("the image is too wide for CCITT".into());
            }
            let mut enc = fax::encoder::Encoder::new(fax::VecWriter::new());
            for row in b.as_raw().chunks_exact(w as usize) {
                let pels = row.iter().map(|&v| {
                    if v == 0 {
                        fax::Color::Black
                    } else {
                        fax::Color::White
                    }
                });
                enc.encode_line(pels, w as u16)
                    .map_err(|_| "CCITT encoding failed")?;
            }
            let data = enc.finish().map_err(|_| "CCITT encoding failed")?.finish();
            dict.set("ColorSpace", "DeviceGray");
            dict.set("BitsPerComponent", 1);
            dict.set("Filter", "CCITTFaxDecode");
            dict.set(
                "DecodeParms",
                dictionary! { "K" => -1, "Columns" => w as i64, "Rows" => h as i64 },
            );
            (w, h, data)
        }
        Img::Gray(g) => {
            let (w, h) = g.dimensions();
            dict.set("ColorSpace", "DeviceGray");
            let data = jpeg(g.as_raw(), w, h, image::ExtendedColorType::L8, quality)?;
            (w, h, data)
        }
        Img::Rgb(c) => {
            let (w, h) = c.dimensions();
            dict.set("ColorSpace", "DeviceRGB");
            let data = jpeg(c.as_raw(), w, h, image::ExtendedColorType::Rgb8, quality)?;
            (w, h, data)
        }
    };
    if !dict.has(b"Filter") {
        dict.set("BitsPerComponent", 8);
        dict.set("Filter", "DCTDecode");
    }
    dict.set("Width", w as i64);
    dict.set("Height", h as i64);
    Ok(Stream::new(dict, content).with_compression(false))
}

fn jpeg(
    px: &[u8],
    w: u32,
    h: u32,
    color: image::ExtendedColorType,
    quality: u8,
) -> std::result::Result<Vec<u8>, String> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
        .encode(px, w, h, color)
        .map_err(|e| format!("JPEG encoding failed: {e}"))?;
    Ok(out)
}
