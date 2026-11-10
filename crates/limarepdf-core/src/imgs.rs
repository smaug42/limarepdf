//! Building image XObjects and pages from PNG, JPEG and TIFF files.

use crate::budget::Work;
use crate::{Pdf, Result};
use anyhow::{anyhow, bail};
use image::metadata::Orientation;
use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};
use std::io::Cursor;
use tiff::decoder::{ifd::Value, Decoder, DecodingResult};
use tiff::tags::Tag;
use tiff::ColorType;

/// How [`Pdf::insert_image_pages`] sizes pages.
#[derive(Clone, Copy, Debug)]
pub struct ImageImport {
    /// Use this resolution (dots per inch) instead of the one stored in the file.
    pub dpi: Option<f32>,
    /// Resolution assumed when the file states none.
    pub fallback_dpi: f32,
}

impl Default for ImageImport {
    fn default() -> Self {
        Self {
            dpi: None,
            fallback_dpi: 150.0,
        }
    }
}

/// The most bytes the pictures one import adds may take once decoded ([`PicturesTooLarge`]):
/// they are kept decoded in memory until the document is saved. About 80 colour A4 pages
/// scanned at 300 dpi; a test can set another through [`crate::budget_seam`].
const MOST_DECODED: u64 = 2 << 30;

/// Pictures were refused because together they take more memory once decoded than one
/// import may add; nothing was changed. Found through the error's chain (`downcast_ref`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PicturesTooLarge {
    /// The bytes allowed.
    pub limit: u64,
}

impl std::fmt::Display for PicturesTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let limit = if self.limit >= 1 << 30 {
            format!("{} GiB", self.limit >> 30)
        } else {
            format!("{} MiB", self.limit.div_ceil(1 << 20))
        };
        write!(
            f,
            "the pictures are too large to add at once: opened, they take more than {limit} \
             of memory. Nothing was added; add fewer at a time"
        )
    }
}

impl std::error::Error for PicturesTooLarge {}

/// Image samples ready for an image XObject.
pub(crate) enum Pixels {
    /// JPEG data, embedded as is.
    Jpeg {
        data: Vec<u8>,
        components: u8,
        /// Adobe APP14 marker present: CMYK samples are stored inverted.
        adobe: bool,
    },
    /// Uncompressed samples, rows padded to whole bytes. `bpc` is 1 or 8.
    Raw {
        color_space: &'static str,
        bpc: u8,
        data: Vec<u8>,
        alpha: Option<Vec<u8>>,
    },
}

/// One decoded image (one page of a multi-page TIFF).
pub(crate) struct Raster {
    pub w: u32,
    pub h: u32,
    pub pixels: Pixels,
    /// Horizontal and vertical resolution in dots per inch, when the file states one.
    pub dpi: Option<(f32, f32)>,
    /// How the stored pixels must be transformed for display (EXIF / TIFF orientation).
    pub orientation: Orientation,
}

/// Pixel size of a PNG, JPEG or TIFF file (the first page of a TIFF) as it is meant to be
/// seen, after its EXIF or TIFF orientation.
pub fn image_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    let (w, h, o) = header(bytes)?;
    Ok(if swaps_axes(o) { (h, w) } else { (w, h) })
}

/// A PNG, JPEG or TIFF file (its first image) decoded for showing on screen: width,
/// height and RGBA pixels, upright as it is meant to be seen and at most `max` pixels on
/// its longer side.
pub fn picture_preview(bytes: &[u8], max: u32) -> Result<(u32, u32, Vec<u8>)> {
    let r = decode_first(bytes)?;
    let (w, h) = (r.w, r.h);
    let rgba = match r.pixels {
        Pixels::Jpeg { data, .. } => {
            image::load_from_memory_with_format(&data, image::ImageFormat::Jpeg)
                .map_err(|e| anyhow!("cannot decode the image: {e}"))?
                .to_rgba8()
                .into_raw()
        }
        Pixels::Raw {
            color_space,
            bpc,
            data,
            alpha,
        } => raw_rgba(w, h, color_space, bpc, &data, alpha.as_deref()),
    };
    let img = image::RgbaImage::from_raw(w, h, rgba)
        .ok_or_else(|| anyhow!("the image's samples are cut short"))?;
    let mut img = image::DynamicImage::ImageRgba8(img);
    img.apply_orientation(r.orientation);
    let img = if img.width().max(img.height()) > max {
        img.thumbnail(max, max)
    } else {
        img
    };
    Ok((img.width(), img.height(), img.to_rgba8().into_raw()))
}

/// Raw samples as RGBA. One-bit samples are grey, 0 black, with rows padded to whole bytes.
fn raw_rgba(w: u32, h: u32, space: &str, bpc: u8, data: &[u8], alpha: Option<&[u8]>) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let n = match space {
        "DeviceGray" => 1,
        "DeviceRGB" => 3,
        _ => 4,
    };
    let row = (w * n * bpc as usize).div_ceil(8);
    let sample = |x: usize, y: usize, c: usize| -> u8 {
        if bpc == 1 {
            let bit = x * n + c;
            let byte = data.get(y * row + bit / 8).copied().unwrap_or(0);
            if byte >> (7 - bit % 8) & 1 == 1 {
                255
            } else {
                0
            }
        } else {
            data.get(y * row + x * n + c).copied().unwrap_or(0)
        }
    };
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let px = match n {
                1 => [sample(x, y, 0); 3],
                3 => [sample(x, y, 0), sample(x, y, 1), sample(x, y, 2)],
                _ => {
                    let k = 255 - sample(x, y, 3) as u16;
                    [0, 1, 2].map(|c| ((255 - sample(x, y, c) as u16) * k / 255) as u8)
                }
            };
            let a = alpha.and_then(|a| a.get(y * w + x).copied()).unwrap_or(255);
            out.extend_from_slice(&[px[0], px[1], px[2], a]);
        }
    }
    out
}

/// A picture's width and height in pixels as it is meant to be seen, after its EXIF
/// orientation.
pub(crate) fn decode_upright_size(bytes: &[u8]) -> Result<(f32, f32)> {
    let (w, h) = image_dimensions(bytes)?;
    Ok((w as f32, h as f32))
}

/// Pixel size and orientation of a PNG, JPEG or TIFF file (the first page of a TIFF),
/// read from its header: decoding a large picture only to learn its size took seconds
/// and hundreds of megabytes, every page of a TIFF. Damage past the header shows when
/// the picture is decoded to be placed.
fn header(bytes: &[u8]) -> Result<(u32, u32, Orientation)> {
    let fmt = image::guess_format(bytes)
        .map_err(|_| anyhow!("unrecognised image format (PNG, JPEG and TIFF are supported)"))?;
    let (w, h, o) = match fmt {
        image::ImageFormat::Jpeg => {
            let info = jpeg_header(bytes)?;
            (info.w, info.h, info.orientation)
        }
        image::ImageFormat::Tiff => {
            let err = |e: tiff::TiffError| anyhow!("cannot decode the TIFF: {e}");
            let mut d = Decoder::new(Cursor::new(bytes)).map_err(err)?;
            let (w, h) = d.dimensions().map_err(err)?;
            let o = d
                .find_tag_unsigned::<u8>(Tag::Orientation)
                .map_err(err)?
                .and_then(Orientation::from_exif)
                .unwrap_or(Orientation::NoTransforms);
            (w, h, o)
        }
        _ => {
            let (w, h) = image::ImageReader::with_format(Cursor::new(bytes), fmt)
                .into_dimensions()
                .map_err(|e| anyhow!("cannot decode the image: {e}"))?;
            let o = if fmt == image::ImageFormat::Png {
                png_meta(bytes).1
            } else {
                Orientation::NoTransforms
            };
            (w, h, o)
        }
    };
    if w == 0 || h == 0 {
        bail!("empty image");
    }
    Ok((w, h, o))
}

pub(crate) fn first(rasters: Vec<Raster>) -> Result<Raster> {
    rasters
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("the file contains no image"))
}

/// Decode a PNG, JPEG or TIFF file into one raster per page, each charged to `work` by the
/// bytes it takes decoded before it is decoded ([`PicturesTooLarge`] when they run out).
pub(crate) fn decode(bytes: &[u8], work: &mut Work) -> Result<Vec<Raster>> {
    decode_pages(bytes, Some(work))
}

/// Decode a PNG, JPEG or TIFF file's picture: a TIFF's first page alone.
pub(crate) fn decode_first(bytes: &[u8]) -> Result<Raster> {
    first(decode_pages(bytes, None)?)
}

/// A file's pictures: every page of a TIFF, each charged to `work`, or the first alone
/// without a `work`.
fn decode_pages(bytes: &[u8], work: Option<&mut Work>) -> Result<Vec<Raster>> {
    let fmt = image::guess_format(bytes)
        .map_err(|_| anyhow!("unrecognised image format (PNG, JPEG and TIFF are supported)"))?;
    let out = match fmt {
        image::ImageFormat::Jpeg => {
            // Kept as it is: its file is what it takes.
            if let Some(work) = work {
                charge(work, bytes.len() as u64)?;
            }
            vec![decode_jpeg(bytes)?]
        }
        image::ImageFormat::Tiff => decode_tiff(bytes, work)?,
        _ => {
            if let Some(work) = work {
                let decoder = image::ImageReader::with_format(Cursor::new(bytes), fmt)
                    .into_decoder()
                    .map_err(|e| anyhow!("cannot decode the image: {e}"))?;
                charge(work, image::ImageDecoder::total_bytes(&decoder))?;
            }
            vec![decode_other(bytes, fmt)?]
        }
    };
    if out.iter().any(|r| r.w == 0 || r.h == 0) {
        bail!("empty image");
    }
    Ok(out)
}

/// Take `n` bytes from what the pictures being added may take decoded.
fn charge(work: &mut Work, n: u64) -> Result<()> {
    work.take(n)
        .map_err(|too| PicturesTooLarge { limit: too.limit }.into())
}

/// A JPEG file's header, refused unless LimarePDF can embed the file.
fn jpeg_header(bytes: &[u8]) -> Result<JpegInfo> {
    let info = jpeg_info(bytes).ok_or_else(|| anyhow!("cannot read the JPEG header"))?;
    if !matches!(info.components, 1 | 3 | 4) {
        bail!(
            "JPEG with {} colour components is not supported",
            info.components
        );
    }
    Ok(info)
}

fn decode_jpeg(bytes: &[u8]) -> Result<Raster> {
    let info = jpeg_header(bytes)?;
    Ok(Raster {
        w: info.w,
        h: info.h,
        pixels: Pixels::Jpeg {
            data: bytes.to_vec(),
            components: info.components,
            adobe: info.adobe,
        },
        dpi: info.dpi,
        orientation: info.orientation,
    })
}

/// Number of colour components in a JPEG file's frame header.
pub(crate) fn jpeg_components(bytes: &[u8]) -> Option<u8> {
    jpeg_info(bytes).map(|i| i.components)
}

struct JpegInfo {
    w: u32,
    h: u32,
    components: u8,
    adobe: bool,
    dpi: Option<(f32, f32)>,
    orientation: Orientation,
    /// The sum of the luminance quantisation table.
    luma_q: Option<u32>,
}

/// Whether JPEG `bytes` is a `w` × `h` grey or colour picture that other apps show as a PDF
/// reader does: one or three components, the size the PDF gives it, and no EXIF turn (a PDF
/// reader ignores one; a photo viewer applies it).
pub(crate) fn jpeg_as_shown(bytes: &[u8], w: i64, h: i64) -> bool {
    jpeg_info(bytes).is_some_and(|i| {
        matches!(i.components, 1 | 3)
            && (i.w as i64, i.h as i64) == (w, h)
            && i.orientation == Orientation::NoTransforms
    })
}

/// Sum of the IJG standard luminance quantisation table, which quality 50 uses as is.
const IJG_LUMA_SUM: u32 = 3_688;

/// The quality (1 to 100, IJG scale) a JPEG was most likely saved at, estimated from
/// its luminance quantisation table.
pub(crate) fn jpeg_quality(bytes: &[u8]) -> Option<u8> {
    let sum = jpeg_info(bytes)?.luma_q?;
    let scale = sum as f64 * 100.0 / IJG_LUMA_SUM as f64;
    let q = if scale <= 100.0 {
        (200.0 - scale) / 2.0
    } else {
        5000.0 / scale
    };
    Some(q.round().clamp(1.0, 100.0) as u8)
}

/// `s` with its `/DecodeParms` written in place, when the file gives them (or an entry of
/// their array) as a reference; `None` when nothing is a reference. lopdf reads only
/// parameters written in place and otherwise decodes as if there were none: a PNG
/// predictor left in, every row a byte longer, the colours shifted (pdf.js's
/// `issue17906`). Decode the copy, never write it back.
pub(crate) fn direct_parms(doc: &lopdf::Document, s: &Stream) -> Option<Stream> {
    let resolve = |o: &Object| match o {
        Object::Reference(r) => doc.get_object(*r).ok().cloned(),
        o => Some(o.clone()),
    };
    let each = |a: &[Object]| {
        Object::Array(
            a.iter()
                .map(|e| resolve(e).unwrap_or(Object::Null))
                .collect(),
        )
    };
    let direct = match s.dict.get(b"DecodeParms").ok()? {
        r @ Object::Reference(_) => match resolve(r)? {
            Object::Array(a) => each(&a),
            o => o,
        },
        Object::Array(a) if a.iter().any(|e| matches!(e, Object::Reference(_))) => each(a),
        _ => return None,
    };
    let mut t = s.clone();
    t.dict.set("DecodeParms", direct);
    Some(t)
}

/// How a picture's colour space shows its numbers, as the renderer reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Space {
    /// Grey or RGB of `n` components shown as they are, as other apps show a JPEG or PNG
    /// file: the device spaces, their abbreviations `G` and `RGB` (which the renderer, and
    /// poppler, read before any name the page gives them: PB-107), and a profile whose
    /// header names sRGB as its device, which the renderer takes as plain RGB.
    Plain(usize),
    /// Grey or RGB of `n` components through a calibration (CalGray, CalRGB) or another
    /// colour profile: the numbers show changed.
    Calibrated(usize),
    /// Anything else: CMYK, Lab, palettes, separations, or a space that can't be read.
    Other,
}

/// Colour space `cs` with the names it goes by followed through `spaces` (a page's
/// `/ColorSpace` resources), as it is written there; `None` when they lead nowhere. At most
/// 8 names are followed, and never back to one already met. The names that are spaces of
/// their own (the device spaces and their abbreviations, CalGray, CalRGB, CalCMYK,
/// Pattern) are not looked up.
pub(crate) fn followed<'a>(
    doc: &'a lopdf::Document,
    cs: &'a Object,
    spaces: &'a Dictionary,
) -> Option<&'a Object> {
    let mut cs = cs;
    let mut seen: Vec<&[u8]> = Vec::new();
    loop {
        match doc.dereference(cs).ok()?.1 {
            Object::Name(n)
                if !matches!(
                    n.as_slice(),
                    b"DeviceGray"
                        | b"G"
                        | b"DeviceRGB"
                        | b"RGB"
                        | b"DeviceCMYK"
                        | b"CMYK"
                        | b"CalGray"
                        | b"CalRGB"
                        | b"CalCMYK"
                        | b"Pattern"
                ) =>
            {
                if seen.len() == 8 || seen.contains(&n.as_slice()) {
                    return None;
                }
                seen.push(n);
                cs = spaces.get(n).ok()?;
            }
            _ => return Some(cs),
        }
    }
}

/// How colour space `cs`, its names followed through `spaces` (see [`followed`]), shows
/// its numbers. One reading for saving a picture (whether its JPEG shows elsewhere as here),
/// optimising a scan and reducing pictures.
pub(crate) fn device_equivalent(doc: &lopdf::Document, cs: &Object, spaces: &Dictionary) -> Space {
    let Some(cs) = followed(doc, cs, spaces).and_then(|o| doc.dereference(o).ok()) else {
        return Space::Other;
    };
    let kind = match cs.1 {
        Object::Name(n) => n.as_slice(),
        Object::Array(a) => match a.first().and_then(|o| o.as_name().ok()) {
            Some(n) => n,
            None => return Space::Other,
        },
        _ => return Space::Other,
    };
    match kind {
        b"DeviceGray" | b"G" if matches!(cs.1, Object::Name(_)) => Space::Plain(1),
        b"DeviceRGB" | b"RGB" if matches!(cs.1, Object::Name(_)) => Space::Plain(3),
        b"CalGray" => Space::Calibrated(1),
        b"CalRGB" => Space::Calibrated(3),
        b"ICCBased" => {
            let Some(s) = cs.1.as_array().ok().and_then(|a| a.get(1)) else {
                return Space::Other;
            };
            let Ok(s) = doc.dereference(s).and_then(|(_, o)| o.as_stream()) else {
                return Space::Other;
            };
            match s.dict.get(b"N").and_then(Object::as_i64) {
                Ok(3) if srgb_profile(s) => Space::Plain(3),
                Ok(n @ (1 | 3)) => Space::Calibrated(n as usize),
                _ => Space::Other,
            }
        }
        _ => Space::Other,
    }
}

/// Whether colour profile `s` names sRGB as its device in its header (bytes 52 to 55),
/// which the renderer takes as plain RGB without reading the rest.
fn srgb_profile(s: &Stream) -> bool {
    // A profile is a few kilobytes; one too large to read isn't taken as sRGB.
    s.get_plain_content_with_limit(1 << 24)
        .is_ok_and(|p| p.get(52..56) == Some(b"sRGB".as_slice()))
}

/// Whether an image XObject's data looks readable, judged from its start without decoding
/// it: a JPEG must have a header giving its size, and compressed data a valid zlib header.
/// Other kinds are taken as readable.
pub(crate) fn image_data_readable(s: &Stream) -> bool {
    let b = &s.content;
    match s.filters().unwrap_or_default().first().copied() {
        Some(b"DCTDecode" | b"DCT") => {
            b.starts_with(&[0xFF, 0xD8]) && jpeg_info(b).is_some_and(|i| i.w > 0 && i.h > 0)
        }
        Some(b"FlateDecode" | b"Fl") => match b[..] {
            [cmf, flg, ..] => cmf & 0x0F == 8 && (u16::from(cmf) * 256 + u16::from(flg)) % 31 == 0,
            _ => false,
        },
        Some(_) => true,
        None => !b.is_empty(),
    }
}

/// Read the JPEG markers up to the first scan.
fn jpeg_info(b: &[u8]) -> Option<JpegInfo> {
    let u16_at = |i: usize| Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]));
    let mut info = JpegInfo {
        w: 0,
        h: 0,
        components: 0,
        adobe: false,
        dpi: None,
        orientation: Orientation::NoTransforms,
        luma_q: None,
    };
    let mut i = 2;
    loop {
        if *b.get(i)? != 0xFF {
            return None;
        }
        while *b.get(i)? == 0xFF {
            i += 1;
        }
        let marker = *b.get(i)?;
        i += 1;
        if matches!(marker, 0x01 | 0xD0..=0xD7) {
            continue;
        }
        if matches!(marker, 0xD9 | 0xDA) {
            break;
        }
        let len = u16_at(i)? as usize;
        let seg = b.get(i + 2..i + len)?;
        match marker {
            0xE0 if seg.starts_with(b"JFIF\0") && seg.len() >= 12 => {
                let (x, y) = (
                    u16::from_be_bytes([seg[8], seg[9]]) as f32,
                    u16::from_be_bytes([seg[10], seg[11]]) as f32,
                );
                info.dpi = match seg[7] {
                    1 => Some((x, y)),
                    2 => Some((x * 2.54, y * 2.54)),
                    _ => None,
                };
            }
            0xE1 if seg.starts_with(b"Exif\0\0") => {
                if let Some(o) = Orientation::from_exif_chunk(&seg[6..]) {
                    info.orientation = o;
                }
            }
            0xEE if seg.starts_with(b"Adobe") => info.adobe = true,
            0xDB => {
                // Quantisation tables: the first (luminance) one gives the quality.
                let mut k = 0;
                while let Some(&pq) = seg.get(k) {
                    let wide = pq >> 4 == 1;
                    let n = if wide { 128 } else { 64 };
                    let t = seg.get(k + 1..k + 1 + n)?;
                    if pq & 0x0F == 0 {
                        info.luma_q = Some(if wide {
                            t.as_chunks::<2>()
                                .0
                                .iter()
                                .map(|&c| u16::from_be_bytes(c) as u32)
                                .sum()
                        } else {
                            t.iter().map(|&v| v as u32).sum()
                        });
                    }
                    k += 1 + n;
                }
            }
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) && seg.len() >= 6 => {
                info.h = u16::from_be_bytes([seg[1], seg[2]]) as u32;
                info.w = u16::from_be_bytes([seg[3], seg[4]]) as u32;
                info.components = seg[5];
            }
            _ => {}
        }
        i += len;
    }
    (info.components > 0).then_some(info)
}

fn decode_other(bytes: &[u8], fmt: image::ImageFormat) -> Result<Raster> {
    let img = image::load_from_memory_with_format(bytes, fmt)
        .map_err(|e| anyhow!("cannot decode the image: {e}"))?;
    let (w, h) = (img.width(), img.height());
    let gray = !img.color().has_color();
    let (color, alpha) = if img.color().has_alpha() {
        let la = if gray {
            img.to_luma_alpha8().into_raw()
        } else {
            img.to_rgba8().into_raw()
        };
        split_alpha(la, if gray { 1 } else { 3 })
    } else if gray {
        (img.to_luma8().into_raw(), None)
    } else {
        (img.to_rgb8().into_raw(), None)
    };
    let (dpi, orientation) = if fmt == image::ImageFormat::Png {
        png_meta(bytes)
    } else {
        (None, Orientation::NoTransforms)
    };
    Ok(Raster {
        w,
        h,
        pixels: Pixels::Raw {
            color_space: if gray { "DeviceGray" } else { "DeviceRGB" },
            bpc: 8,
            data: color,
            alpha,
        },
        dpi,
        orientation,
    })
}

/// Resolution (pHYs) and orientation (eXIf) chunks of a PNG file.
fn png_meta(b: &[u8]) -> (Option<(f32, f32)>, Orientation) {
    let (mut dpi, mut orientation) = (None, Orientation::NoTransforms);
    let mut i = 8;
    while let Some(head) = b.get(i..i + 8) {
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let Some(data) = b.get(i + 8..i + 8 + len) else {
            break;
        };
        match &head[4..8] {
            b"pHYs" if len >= 9 && data[8] == 1 => {
                let ppm = |k: usize| {
                    u32::from_be_bytes([data[k], data[k + 1], data[k + 2], data[k + 3]]) as f32
                };
                dpi = Some((ppm(0) * 0.0254, ppm(4) * 0.0254));
            }
            b"eXIf" => {
                if let Some(o) = Orientation::from_exif_chunk(data) {
                    orientation = o;
                }
            }
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
        i += 12 + len;
    }
    (dpi, orientation)
}

/// A TIFF's pages: every one, each charged to `work` by its decoded size before it is
/// decoded, or the first alone without a `work`.
fn decode_tiff(bytes: &[u8], mut work: Option<&mut Work>) -> Result<Vec<Raster>> {
    let err = |e: tiff::TiffError| anyhow!("cannot decode the TIFF: {e}");
    let mut d = Decoder::new(Cursor::new(bytes)).map_err(err)?;
    let mut out = Vec::new();
    loop {
        let n = out.len() + 1;
        let page = |e: anyhow::Error| anyhow!("TIFF page {n}: {e}");
        let size = tiff_size(&mut d).map_err(page)?;
        if let Some(work) = work.as_deref_mut() {
            charge(work, size)?;
        }
        out.push(tiff_page(&mut d).map_err(page)?);
        if work.is_none() || !d.more_images() {
            break;
        }
        d.next_image().map_err(err)?;
    }
    Ok(out)
}

/// The bytes the TIFF page `d` is at takes decoded, read from its header; refused for a
/// colour type [`tiff_page`] can't take, or a size past what a `u64` holds, before it is
/// decoded.
fn tiff_size(d: &mut Decoder<Cursor<&[u8]>>) -> Result<u64> {
    let (w, h) = d.dimensions()?;
    let bits = match d.colortype()? {
        ColorType::Gray(b @ (1 | 8 | 16)) => b,
        ColorType::GrayA(8) => 16,
        ColorType::RGB(b @ (8 | 16)) => 3 * b,
        ColorType::RGBA(b @ (8 | 16)) => 4 * b,
        ColorType::CMYK(8) => 32,
        ct => bail!("colour type {ct:?} is not supported"),
    };
    // A row's bytes fit (under 2³² × 64 bits); the rows may not.
    (u64::from(w) * u64::from(bits))
        .div_ceil(8)
        .checked_mul(u64::from(h))
        .ok_or_else(|| anyhow!("the picture is too large ({w} × {h} pixels)"))
}

fn tiff_page(d: &mut Decoder<Cursor<&[u8]>>) -> Result<Raster> {
    #[cfg(test)]
    tests::PAGES.set(tests::PAGES.get() + 1);
    let (w, h) = d.dimensions()?;
    let ct = d.colortype()?;
    let num = |v: Option<Value>| match v? {
        Value::Rational(n, d) if d != 0 => Some(n as f32 / d as f32),
        Value::Short(n) => Some(n as f32),
        Value::Unsigned(n) => Some(n as f32),
        Value::Float(n) => Some(n),
        _ => None,
    };
    let x = num(d.find_tag(Tag::XResolution)?);
    let y = num(d.find_tag(Tag::YResolution)?);
    let unit = d
        .find_tag_unsigned::<u16>(Tag::ResolutionUnit)?
        .unwrap_or(2);
    let dpi = match (x, y, unit) {
        (Some(x), Some(y), 2) => Some((x, y)),
        (Some(x), Some(y), 3) => Some((x * 2.54, y * 2.54)),
        _ => None,
    };
    let orientation = d
        .find_tag_unsigned::<u8>(Tag::Orientation)?
        .and_then(Orientation::from_exif)
        .unwrap_or(Orientation::NoTransforms);
    let img = d.read_image()?;
    let to8 = |v: Vec<u16>| v.into_iter().map(|s| (s >> 8) as u8).collect::<Vec<u8>>();
    let (color_space, bpc, data, alpha) = match (ct, img) {
        (ColorType::Gray(1), DecodingResult::U8(v)) => ("DeviceGray", 1, v, None),
        (ColorType::Gray(8), DecodingResult::U8(v)) => ("DeviceGray", 8, v, None),
        (ColorType::Gray(16), DecodingResult::U16(v)) => ("DeviceGray", 8, to8(v), None),
        (ColorType::GrayA(8), DecodingResult::U8(v)) => {
            let (c, a) = split_alpha(v, 1);
            ("DeviceGray", 8, c, a)
        }
        (ColorType::RGB(8), DecodingResult::U8(v)) => ("DeviceRGB", 8, v, None),
        (ColorType::RGB(16), DecodingResult::U16(v)) => ("DeviceRGB", 8, to8(v), None),
        (ColorType::RGBA(8), DecodingResult::U8(v)) => {
            let (c, a) = split_alpha(v, 3);
            ("DeviceRGB", 8, c, a)
        }
        (ColorType::RGBA(16), DecodingResult::U16(v)) => {
            let (c, a) = split_alpha(to8(v), 3);
            ("DeviceRGB", 8, c, a)
        }
        (ColorType::CMYK(8), DecodingResult::U8(v)) => ("DeviceCMYK", 8, v, None),
        (ct, _) => bail!("colour type {ct:?} is not supported"),
    };
    Ok(Raster {
        w,
        h,
        pixels: Pixels::Raw {
            color_space,
            bpc,
            data,
            alpha,
        },
        dpi,
        orientation,
    })
}

/// Split interleaved samples with a trailing alpha channel. The alpha is dropped when
/// fully opaque.
fn split_alpha(v: Vec<u8>, colors: usize) -> (Vec<u8>, Option<Vec<u8>>) {
    let n = colors + 1;
    let mut color = Vec::with_capacity(v.len() / n * colors);
    let mut alpha = Vec::with_capacity(v.len() / n);
    for px in v.chunks_exact(n) {
        color.extend_from_slice(&px[..colors]);
        alpha.push(px[colors]);
    }
    let opaque = alpha.iter().all(|&a| a == 255);
    (color, (!opaque).then_some(alpha))
}

/// Where the stored image's top-left, bottom-left and bottom-right corners land on a
/// `1 x 1` display box (y up), for each orientation.
fn corners(o: Orientation) -> [(f32, f32); 3] {
    use Orientation::*;
    match o {
        NoTransforms => [(0., 1.), (0., 0.), (1., 0.)],
        FlipHorizontal => [(1., 1.), (1., 0.), (0., 0.)],
        Rotate180 => [(1., 0.), (1., 1.), (0., 1.)],
        FlipVertical => [(0., 0.), (0., 1.), (1., 1.)],
        Rotate90 => [(1., 1.), (0., 1.), (0., 0.)],
        Rotate270 => [(0., 0.), (1., 0.), (1., 1.)],
        Rotate90FlipH => [(0., 1.), (1., 1.), (1., 0.)],
        Rotate270FlipH => [(1., 0.), (0., 0.), (0., 1.)],
    }
}

/// Where a point of a picture as it is seen (`u`, `v`: shares of its width and height, y
/// down) is in the stored picture (shares of its width and height, y down).
pub(crate) fn stored_point(o: Orientation, u: f32, v: f32) -> (f32, f32) {
    let [tl, bl, br] = corners(o);
    // Seen = top-left + s * (bottom-right - bottom-left) + t * (bottom-left - top-left), y
    // up; those two steps are unit lengths along the axes, so this undoes it.
    let (dx, dy) = (u - tl.0, 1.0 - v - tl.1);
    let (across, down) = ((br.0 - bl.0, br.1 - bl.1), (bl.0 - tl.0, bl.1 - tl.1));
    (across.0 * dx + across.1 * dy, down.0 * dx + down.1 * dy)
}

pub(crate) fn swaps_axes(o: Orientation) -> bool {
    use Orientation::*;
    matches!(o, Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH)
}

/// Matrix that draws an image upright in a `pw` x `ph` box at the origin, following the
/// file's orientation.
pub(crate) fn orient_matrix(o: Orientation, pw: f32, ph: f32) -> [f32; 6] {
    let [tl, bl, br] = corners(o).map(|(x, y)| (x * pw, y * ph));
    [
        br.0 - bl.0,
        br.1 - bl.1,
        tl.0 - bl.0,
        tl.1 - bl.1,
        bl.0,
        bl.1,
    ]
}

impl Pdf {
    /// Decode an image file and add it to the document as an image XObject. Returns the
    /// object id, the pixel size as displayed and the orientation to draw it with (see
    /// [`orient_matrix`]). JPEG data is embedded as is.
    pub(crate) fn add_upright_image(
        &mut self,
        bytes: &[u8],
    ) -> Result<(ObjectId, f32, f32, Orientation)> {
        let r = decode_first(bytes)?;
        let o = r.orientation;
        let (w, h) = (r.w as f32, r.h as f32);
        let (w, h) = if swaps_axes(o) { (h, w) } else { (w, h) };
        Ok((self.add_raster(r), w, h, o))
    }

    pub(crate) fn add_raster(&mut self, r: Raster) -> ObjectId {
        let mut dict = dictionary! {
            "Type" => "XObject", "Subtype" => "Image",
            "Width" => r.w as i64, "Height" => r.h as i64,
        };
        let content = match r.pixels {
            Pixels::Jpeg {
                data,
                components,
                adobe,
            } => {
                dict.set("BitsPerComponent", 8);
                dict.set("Filter", "DCTDecode");
                dict.set(
                    "ColorSpace",
                    match components {
                        1 => "DeviceGray",
                        3 => "DeviceRGB",
                        _ => "DeviceCMYK",
                    },
                );
                if components == 4 && adobe {
                    dict.set(
                        "Decode",
                        [1, 0, 1, 0, 1, 0, 1, 0].map(Object::from).to_vec(),
                    );
                }
                data
            }
            Pixels::Raw {
                color_space,
                bpc,
                data,
                alpha,
            } => {
                dict.set("BitsPerComponent", bpc as i64);
                dict.set("ColorSpace", color_space);
                if let Some(alpha) = alpha {
                    let sm = dictionary! {
                        "Type" => "XObject", "Subtype" => "Image",
                        "Width" => r.w as i64, "Height" => r.h as i64,
                        "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
                    };
                    let sid = self.doc.add_object(Stream::new(sm, alpha));
                    dict.set("SMask", Object::Reference(sid));
                }
                data
            }
        };
        self.doc.add_object(Stream::new(dict, content))
    }

    /// Insert one page per image at position `at` (0 = before the first page). Each page
    /// of a multi-page TIFF becomes a page. Pages are sized from the image resolution and
    /// shown in the orientation the file asks for. Returns the number of pages added.
    /// Nothing is changed if any file cannot be decoded.
    pub fn insert_image_pages(
        &mut self,
        at: usize,
        files: &[Vec<u8>],
        opts: &ImageImport,
    ) -> Result<usize> {
        if at > self.page_count() {
            bail!("position {at} is beyond the end of the document");
        }
        let mut work = Work::new("adding the pictures", crate::budget::limit(MOST_DECODED));
        let mut rasters = Vec::new();
        for (i, f) in files.iter().enumerate() {
            rasters.extend(decode(f, &mut work).map_err(|e| {
                if e.is::<PicturesTooLarge>() {
                    e
                } else {
                    anyhow!("image {}: {e}", i + 1)
                }
            })?);
        }
        let n = rasters.len();
        for (k, r) in rasters.into_iter().enumerate() {
            let usable = |d: f32| (10.0..=10_000.0).contains(&d);
            let (dx, dy) = match (opts.dpi, r.dpi) {
                (Some(d), _) => (d, d),
                (None, Some((x, y))) if usable(x) && usable(y) => (x, y),
                _ => (opts.fallback_dpi, opts.fallback_dpi),
            };
            let (iw, ih) = (r.w as f32 * 72.0 / dx, r.h as f32 * 72.0 / dy);
            let o = r.orientation;
            let (pw, ph) = if swaps_axes(o) { (ih, iw) } else { (iw, ih) };
            let m = orient_matrix(o, pw, ph);
            let img = self.add_raster(r);
            self.insert_blank(at + k, pw, ph)?;
            let page = self.page_ids()[at + k];
            let content = format!(
                "q {} {} {} {} {} {} cm /Im0 Do Q",
                m[0], m[1], m[2], m[3], m[4], m[5]
            );
            let cid = self
                .doc
                .add_object(Stream::new(Dictionary::new(), content.into_bytes()));
            let d = self.doc.get_dictionary_mut(page)?;
            d.set("Contents", cid);
            d.set(
                "Resources",
                dictionary! { "XObject" => dictionary! { "Im0" => img } },
            );
        }
        Ok(n)
    }

    /// A new document with one page per image. See [`Pdf::insert_image_pages`].
    pub fn from_images(files: &[Vec<u8>], opts: &ImageImport) -> Result<Pdf> {
        let mut p = Pdf::new();
        p.insert_image_pages(0, files, opts)?;
        Ok(p)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::Cell;

    thread_local! {
        /// TIFF pages decoded on this thread.
        pub(crate) static PAGES: Cell<usize> = const { Cell::new(0) };
    }

    /// The TIFF pages `f` decodes.
    fn pages_decoded(f: impl FnOnce()) -> usize {
        PAGES.set(0);
        f();
        PAGES.get()
    }

    /// A TIFF of `n` 8 × 8 RGB pages.
    fn tiff_of(n: usize) -> Vec<u8> {
        use tiff::encoder::{colortype::RGB8, TiffEncoder};
        let mut buf = Cursor::new(Vec::new());
        let mut enc = TiffEncoder::new(&mut buf).unwrap();
        for _ in 0..n {
            enc.write_image::<RGB8>(8, 8, &[200; 8 * 8 * 3]).unwrap();
        }
        buf.into_inner()
    }

    /// What shows or places one picture from a TIFF decodes its first page alone (M-06):
    /// each decoded every page, and kept them all until done.
    #[test]
    fn one_picture_from_a_tiff_decodes_one_page() {
        let tiff = tiff_of(50);
        let n = pages_decoded(|| {
            picture_preview(&tiff, 100).unwrap();
        });
        assert_eq!(n, 1, "the preview");
        let n = pages_decoded(|| {
            crate::sign::picture_view(&tiff, 100).unwrap();
        });
        assert_eq!(n, 1, "the signature picture");
        let mut p = Pdf::new();
        let n = pages_decoded(|| {
            p.add_upright_image(&tiff).unwrap();
        });
        assert_eq!(n, 1, "placing it");
        // Importing makes a page of each.
        let n = pages_decoded(|| {
            let doc = Pdf::from_images(&[tiff], &ImageImport::default()).unwrap();
            assert_eq!(doc.page_count(), 50);
        });
        assert_eq!(n, 50, "importing");
    }

    /// A TIFF whose header alone says it is `u32::MAX` by `u32::MAX` RGB pixels in one
    /// strip, with one byte of data.
    fn tiff_claiming_the_most() -> Vec<u8> {
        let mut t = b"II*\0".to_vec();
        t.extend(8u32.to_le_bytes());
        // Tag, type (3 SHORT, 4 LONG), count, value or offset; in tag order.
        let entries: [(u16, u16, u32, u32); 10] = [
            (256, 4, 1, u32::MAX),
            (257, 4, 1, u32::MAX),
            (258, 3, 3, 134),
            (259, 3, 1, 1),
            (262, 3, 1, 2),
            (273, 4, 1, 140),
            (277, 3, 1, 3),
            (278, 4, 1, u32::MAX),
            (279, 4, 1, 1),
            (284, 3, 1, 1),
        ];
        t.extend((entries.len() as u16).to_le_bytes());
        for (tag, ty, count, value) in entries {
            t.extend(tag.to_le_bytes());
            t.extend(ty.to_le_bytes());
            t.extend(count.to_le_bytes());
            t.extend(value.to_le_bytes());
        }
        t.extend(0u32.to_le_bytes());
        assert_eq!(t.len(), 134);
        t.extend([8u16, 8, 8].iter().flat_map(|b| b.to_le_bytes()));
        t.push(0);
        t
    }

    /// A TIFF whose header gives a size past what a `u64` holds is refused from its header,
    /// with nothing decoded, wherever a TIFF is read (CP-04): its size overflowed first.
    #[test]
    fn a_tiff_too_large_to_count_is_refused_from_its_header() {
        let tiff = tiff_claiming_the_most();
        let too_large = |r: Result<()>| {
            let e = format!("{:#}", r.unwrap_err());
            assert!(e.contains("too large"), "{e}");
        };
        let n = pages_decoded(|| {
            too_large(picture_preview(&tiff, 100).map(drop));
            too_large(crate::sign::picture_view(&tiff, 100).map(drop));
            too_large(Pdf::new().add_upright_image(&tiff).map(drop));
            too_large(
                Pdf::from_images(std::slice::from_ref(&tiff), &ImageImport::default()).map(drop),
            );
        });
        assert_eq!(n, 0);
    }
}
