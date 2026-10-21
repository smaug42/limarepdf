//! Building image XObjects and pages from PNG, JPEG and TIFF files.

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

/// Pixel size of a PNG, JPEG or TIFF file (the first page of a TIFF), as stored.
pub fn image_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    let (w, h, _) = header(bytes)?;
    Ok((w, h))
}

/// A picture's width and height in pixels as it is meant to be seen, after its EXIF
/// orientation.
pub(crate) fn decode_upright_size(bytes: &[u8]) -> Result<(f32, f32)> {
    let (w, h, o) = header(bytes)?;
    let (w, h) = (w as f32, h as f32);
    Ok(if swaps_axes(o) { (h, w) } else { (w, h) })
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

/// Decode a PNG, JPEG or TIFF file into one raster per page.
pub(crate) fn decode(bytes: &[u8]) -> Result<Vec<Raster>> {
    let fmt = image::guess_format(bytes)
        .map_err(|_| anyhow!("unrecognised image format (PNG, JPEG and TIFF are supported)"))?;
    let out = match fmt {
        image::ImageFormat::Jpeg => vec![decode_jpeg(bytes)?],
        image::ImageFormat::Tiff => decode_tiff(bytes)?,
        _ => vec![decode_other(bytes, fmt)?],
    };
    if out.iter().any(|r| r.w == 0 || r.h == 0) {
        bail!("empty image");
    }
    Ok(out)
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

fn decode_tiff(bytes: &[u8]) -> Result<Vec<Raster>> {
    let err = |e: tiff::TiffError| anyhow!("cannot decode the TIFF: {e}");
    let mut d = Decoder::new(Cursor::new(bytes)).map_err(err)?;
    let mut out = Vec::new();
    loop {
        out.push(tiff_page(&mut d).map_err(|e| anyhow!("TIFF page {}: {e}", out.len() + 1))?);
        if !d.more_images() {
            break;
        }
        d.next_image().map_err(err)?;
    }
    Ok(out)
}

fn tiff_page(d: &mut Decoder<Cursor<&[u8]>>) -> Result<Raster> {
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
    /// Decode an image file and add it to the document as an image XObject.
    /// Returns the object id and the stored pixel size. JPEG data is embedded as is.
    pub(crate) fn add_image_object(&mut self, bytes: &[u8]) -> Result<(ObjectId, u32, u32)> {
        let r = first(decode(bytes)?)?;
        let (w, h) = (r.w, r.h);
        Ok((self.add_raster(r), w, h))
    }

    /// Like [`Pdf::add_image_object`], but returns the pixel size as displayed and the
    /// orientation to draw it with (see [`orient_matrix`]).
    pub(crate) fn add_upright_image(
        &mut self,
        bytes: &[u8],
    ) -> Result<(ObjectId, f32, f32, Orientation)> {
        let r = first(decode(bytes)?)?;
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
        let mut rasters = Vec::new();
        for (i, f) in files.iter().enumerate() {
            rasters.extend(decode(f).map_err(|e| anyhow!("image {}: {e}", i + 1))?);
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
