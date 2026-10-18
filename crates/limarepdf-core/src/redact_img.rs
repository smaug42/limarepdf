//! Redaction of pictures: the samples under the marks are set to zero and the picture is
//! saved again, so what was there is gone from the file. The black box drawn over the
//! mark hides whatever colour zero stands for in the picture's colour space.
//!
//! JPEG pictures are saved as JPEG again; others are saved losslessly (Flate). A
//! picture's soft mask and stencil mask are cut the same way, as they can show shapes.

use crate::matrix::{self as mx, M};
use image::codecs::jpeg::JpegEncoder;
use image::ExtendedColorType;
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, Stream};

/// The most decoded sample bytes a picture may have.
const MAX_SAMPLES: usize = 1 << 30;
/// JPEG quality used when a cut JPEG picture is saved again and its own can't be told.
const JPEG_QUALITY: u8 = 85;
/// Added to a JPEG's own quality when it is saved again, to limit the loss of a second
/// round of compression, up to [`JPEG_MAX`].
const JPEG_MARGIN: u8 = 5;
const JPEG_MAX: u8 = 95;
/// Largest average sample left under a mark in a JPEG picture: re-encoding a black area
/// beside a bright one leaves faint ripples.
const JPEG_DARK: f64 = 12.0;

type Res<T> = std::result::Result<T, String>;

/// A picture after cutting, with the masks that were cut along with it.
pub(crate) struct Cut {
    pub image: Stream,
    pub masks: Vec<(&'static str, Stream)>,
}

/// How a picture is saved again.
#[derive(Clone, Copy, PartialEq)]
enum Codec {
    /// Flate, losslessly.
    Raw,
    /// JPEG at this quality.
    Jpeg(u8),
    /// CCITT Group 4, as it came; `BlackIs1` as the original said.
    Ccitt { black_is_1: bool },
}

/// A picture's decoded samples: `h` rows of `w × n` samples of `bpc` bits, each row
/// starting on a byte.
struct Samples {
    w: usize,
    h: usize,
    n: usize,
    bpc: usize,
    data: Vec<u8>,
    codec: Codec,
}

impl Samples {
    fn stride(&self) -> usize {
        (self.w * self.n * self.bpc).div_ceil(8)
    }

    /// The bit range of pixel columns `x0..x1` within a row.
    fn bits(&self, x0: usize, x1: usize) -> (usize, usize) {
        let px = self.n * self.bpc;
        (x0 * px, x1 * px)
    }

    fn clear(&mut self, r: [usize; 4]) {
        let (s, e) = self.bits(r[0], r[2]);
        let stride = self.stride();
        for y in r[1]..r[3] {
            let row = &mut self.data[y * stride..(y + 1) * stride];
            for b in s..e {
                row[b / 8] &= !(0x80 >> (b % 8));
            }
        }
    }

    /// Whether the pixels in `r` are all zero (for JPEG pictures: dark enough).
    fn is_clear(&self, r: [usize; 4]) -> bool {
        let (s, e) = self.bits(r[0], r[2]);
        let stride = self.stride();
        let rows = r[1]..r[3];
        if matches!(self.codec, Codec::Jpeg(_)) {
            let (mut sum, mut count) = (0u64, 0u64);
            for y in rows {
                for v in &self.data[y * stride + s / 8..y * stride + e / 8] {
                    sum += *v as u64;
                    count += 1;
                }
            }
            return count == 0 || sum as f64 / count as f64 <= JPEG_DARK;
        }
        rows.into_iter().all(|y| {
            let row = &self.data[y * stride..(y + 1) * stride];
            (s..e).all(|b| row[b / 8] & (0x80 >> (b % 8)) == 0)
        })
    }
}

/// The pixel rectangles (x0, y0, x1, y1; row 0 at the top) of a `w × h` picture drawn
/// with `ctm` that touch any of `areas` (page user space). Pixels only partly under an
/// area are included. `None` when the picture is drawn with no size.
fn pixel_rects(w: usize, h: usize, ctm: M, areas: &[[f32; 4]]) -> Option<Vec<[usize; 4]>> {
    let inv = mx::inv(ctm)?;
    let mut out = Vec::new();
    for &a in areas {
        // The area in the picture's unit square, y up.
        let b = mx::transform_rect(inv, a);
        let c = |v: f32| v.clamp(0.0, 1.0) as f64;
        let (u0, v0, u1, v1) = (c(b[0]), c(b[1]), c(b[2]), c(b[3]));
        // A thousandth of a pixel is allowed for rounding, so an area ending on a pixel's
        // edge doesn't take in the next pixel.
        const SLACK: f64 = 1e-3;
        let x0 = (u0 * w as f64 + SLACK).floor() as usize;
        let x1 = ((u1 * w as f64 - SLACK).ceil().max(0.0) as usize).min(w);
        let y0 = ((1.0 - v1) * h as f64 + SLACK).floor() as usize;
        let y1 = (((1.0 - v0) * h as f64 - SLACK).ceil().max(0.0) as usize).min(h);
        if x0 < x1 && y0 < y1 && u1 > 0.0 && u0 < 1.0 && v1 > 0.0 && v0 < 1.0 {
            out.push([x0, y0, x1, y1]);
        }
    }
    Some(out)
}

fn int(d: &Dictionary, key: &[u8]) -> Option<i64> {
    d.get(key).ok()?.as_i64().ok()
}

/// How many colour components a colour space has. Named spaces are looked up in
/// `spaces` (the resources' `/ColorSpace`).
fn components(doc: &Document, cs: &Object, spaces: &Dictionary, depth: usize) -> Option<usize> {
    if depth > 8 {
        return None;
    }
    let cs = doc.dereference(cs).ok()?.1;
    match cs {
        Object::Name(n) => match n.as_slice() {
            b"DeviceGray" | b"CalGray" | b"G" => Some(1),
            b"DeviceRGB" | b"CalRGB" | b"RGB" => Some(3),
            b"DeviceCMYK" | b"CMYK" => Some(4),
            b"Pattern" => None,
            other => components(doc, spaces.get(other).ok()?, spaces, depth + 1),
        },
        Object::Array(a) => match a.first()?.as_name().ok()? {
            b"ICCBased" => {
                let s = doc.dereference(a.get(1)?).ok()?.1.as_stream().ok()?;
                usize::try_from(int(&s.dict, b"N")?)
                    .ok()
                    .filter(|n| (1..=32).contains(n))
            }
            b"CalGray" | b"Indexed" | b"I" | b"Separation" => Some(1),
            b"CalRGB" | b"Lab" => Some(3),
            b"DeviceN" => Some(doc.dereference(a.get(1)?).ok()?.1.as_array().ok()?.len()),
            _ => components(doc, a.first()?, spaces, depth + 1),
        },
        _ => None,
    }
}

/// Decode a picture (or mask) to its samples. `mask` marks a stencil mask (`/Mask`
/// given as a picture), which has one 1-bit component whatever it says.
fn decode(doc: &Document, s: &Stream, spaces: &Dictionary, mask: bool) -> Res<Samples> {
    let d = &s.dict;
    let (w, h) = match (int(d, b"Width"), int(d, b"Height")) {
        (Some(w), Some(h)) if w > 0 && h > 0 && w <= 1 << 16 && h <= 1 << 16 => {
            (w as usize, h as usize)
        }
        _ => return Err("the picture has no usable size".into()),
    };
    let stencil = mask
        || d.get(b"ImageMask")
            .and_then(|o| o.as_bool())
            .unwrap_or(false);
    let filters: Vec<&[u8]> = s.filters().unwrap_or_default();
    match filters.as_slice() {
        [b"DCTDecode"] => {
            let comps = crate::imgs::jpeg_components(&s.content);
            if comps == Some(4) || comps.is_none() {
                return Err("CMYK or unreadable JPEG".into());
            }
            let img = image::load_from_memory_with_format(&s.content, image::ImageFormat::Jpeg)
                .map_err(|e| format!("cannot decode the JPEG: {e}"))?;
            if (img.width() as usize, img.height() as usize) != (w, h) {
                return Err("the JPEG's size doesn't match the picture's".into());
            }
            let (n, data) = if comps == Some(1) {
                (1, img.to_luma8().into_raw())
            } else {
                (3, img.to_rgb8().into_raw())
            };
            let quality = crate::imgs::jpeg_quality(&s.content).map_or(JPEG_QUALITY, |q| {
                q.saturating_add(JPEG_MARGIN).min(JPEG_MAX)
            });
            Ok(Samples {
                w,
                h,
                n,
                bpc: 8,
                data,
                codec: Codec::Jpeg(quality),
            })
        }
        [b"CCITTFaxDecode"] => {
            let crate::scan::Img::Bits(g) = crate::scan::decode_ccitt(s, w as u32, h as u32)?
            else {
                return Err("cannot decode the CCITT picture".into());
            };
            let black_is_1 = d
                .get(b"DecodeParms")
                .and_then(|o| o.as_dict())
                .and_then(|p| p.get(b"BlackIs1"))
                .and_then(|o| o.as_bool())
                .unwrap_or(false);
            let stride = w.div_ceil(8);
            let mut data = vec![0u8; stride * h];
            for (i, &v) in g.as_raw().iter().enumerate() {
                let (x, y) = (i % w, i / w);
                if (v == 0) == black_is_1 {
                    data[y * stride + x / 8] |= 0x80 >> (x % 8);
                }
            }
            Ok(Samples {
                w,
                h,
                n: 1,
                bpc: 1,
                data,
                codec: Codec::Ccitt { black_is_1 },
            })
        }
        [.., last]
            if !matches!(
                *last,
                b"FlateDecode"
                    | b"LZWDecode"
                    | b"ASCII85Decode"
                    | b"ASCIIHexDecode"
                    | b"RunLengthDecode"
                    | b"BrotliDecode"
            ) =>
        {
            Err(format!(
                "{} pictures can't be read",
                String::from_utf8_lossy(last)
            ))
        }
        _ => {
            let (n, bpc) = if stencil {
                (1, 1)
            } else {
                let cs = d
                    .get(b"ColorSpace")
                    .map_err(|_| "the picture has no colour space")?;
                let n = components(doc, cs, spaces, 0).ok_or("unknown colour space")?;
                let bpc = int(d, b"BitsPerComponent").unwrap_or(8);
                if !matches!(bpc, 1 | 2 | 4 | 8 | 16) {
                    return Err("unusual bits per component".into());
                }
                (n, bpc as usize)
            };
            let need = (w * n * bpc).div_ceil(8) * h;
            if need > MAX_SAMPLES {
                return Err("the picture is too large".into());
            }
            let mut data = s
                .decompressed_content_with_limit(MAX_SAMPLES)
                .map_err(|e| format!("cannot decompress the picture: {e}"))?;
            if data.len() < need {
                return Err("the picture's data is too short".into());
            }
            data.truncate(need);
            Ok(Samples {
                w,
                h,
                n,
                bpc,
                data,
                codec: Codec::Raw,
            })
        }
    }
}

/// The picture `s` with its samples replaced.
fn encode(s: &Stream, px: Samples) -> Res<Stream> {
    let mut dict = s.dict.clone();
    for k in [&b"Filter"[..], b"DecodeParms", b"Length"] {
        dict.remove(k);
    }
    match px.codec {
        Codec::Jpeg(quality) => {
            let mut out = Vec::new();
            let ct = if px.n == 1 {
                ExtendedColorType::L8
            } else {
                ExtendedColorType::Rgb8
            };
            JpegEncoder::new_with_quality(&mut out, quality)
                .encode(&px.data, px.w as u32, px.h as u32, ct)
                .map_err(|e| format!("cannot save the JPEG: {e}"))?;
            dict.set("Filter", "DCTDecode");
            Ok(Stream::new(dict, out))
        }
        Codec::Ccitt { black_is_1 } => {
            let stride = px.stride();
            let mut enc = fax::encoder::Encoder::new(fax::VecWriter::new());
            for row in px.data.chunks_exact(stride) {
                let pels = (0..px.w).map(|x| {
                    let bit = row[x / 8] & (0x80 >> (x % 8)) != 0;
                    if bit == black_is_1 {
                        fax::Color::Black
                    } else {
                        fax::Color::White
                    }
                });
                enc.encode_line(pels, px.w as u16)
                    .map_err(|_| "CCITT encoding failed")?;
            }
            let data = enc.finish().map_err(|_| "CCITT encoding failed")?.finish();
            let mut parms = Dictionary::new();
            parms.set("K", -1);
            parms.set("Columns", px.w as i64);
            parms.set("Rows", px.h as i64);
            if black_is_1 {
                parms.set("BlackIs1", true);
            }
            dict.set("Filter", "CCITTFaxDecode");
            dict.set("DecodeParms", parms);
            Ok(Stream::new(dict, data))
        }
        Codec::Raw => {
            let mut out = Stream::new(dict, px.data);
            let _ = out.compress();
            Ok(out)
        }
    }
}

/// Black out the part of picture `s`, drawn with `ctm`, under `areas` (page user
/// space). `Ok(None)` when none of its pixels are under them; `Err` when it can't be
/// read, so it has to be removed whole.
pub(crate) fn black_out(
    doc: &Document,
    s: &Stream,
    spaces: &Dictionary,
    ctm: M,
    areas: &[[f32; 4]],
) -> Res<Option<Cut>> {
    let mut px = decode(doc, s, spaces, false)?;
    let rects = pixel_rects(px.w, px.h, ctm, areas).ok_or("the picture has no size")?;
    if rects.is_empty() {
        return Ok(None);
    }
    for &r in &rects {
        px.clear(r);
    }
    let mut masks = Vec::new();
    for (key, stencil) in [("SMask", false), ("Mask", true)] {
        let Ok(m) = s.dict.get(key.as_bytes()) else {
            continue;
        };
        // A colour-key mask (an array of ranges) holds no picture.
        let Ok(ms) = doc.dereference(m).and_then(|(_, o)| o.as_stream()) else {
            continue;
        };
        let mut mp = decode(doc, ms, spaces, stencil)?;
        let rects = pixel_rects(mp.w, mp.h, ctm, areas).ok_or("the mask has no size")?;
        for r in rects {
            mp.clear(r);
        }
        masks.push((key, encode(ms, mp)?));
    }
    Ok(Some(Cut {
        image: encode(s, px)?,
        masks,
    }))
}

/// Whether picture `s`, drawn with `ctm`, has nothing but zero samples under `areas`.
pub(crate) fn is_clear(
    doc: &Document,
    s: &Stream,
    spaces: &Dictionary,
    ctm: M,
    areas: &[[f32; 4]],
) -> Res<bool> {
    let px = decode(doc, s, spaces, false)?;
    let rects = pixel_rects(px.w, px.h, ctm, areas).ok_or("the picture has no size")?;
    Ok(rects.into_iter().all(|r| px.is_clear(r)))
}

/// An inline image as a picture object, with its abbreviated keys and names written out.
pub(crate) fn inline_stream(raw: &[u8], spaces: &Dictionary) -> Option<Stream> {
    let (dict, data) = crate::inline::split(raw)?;
    let mut text = dict.to_vec();
    text.extend_from_slice(b" LIMAREDICT");
    let ops = Content::decode(&text).ok()?.operations;
    let [entry] = ops.as_slice() else {
        return None;
    };
    let full_name = |n: &[u8]| -> Vec<u8> {
        match n {
            b"G" => b"DeviceGray".to_vec(),
            b"RGB" => b"DeviceRGB".to_vec(),
            b"CMYK" => b"DeviceCMYK".to_vec(),
            b"I" => b"Indexed".to_vec(),
            b"AHx" => b"ASCIIHexDecode".to_vec(),
            b"A85" => b"ASCII85Decode".to_vec(),
            b"LZW" => b"LZWDecode".to_vec(),
            b"Fl" => b"FlateDecode".to_vec(),
            b"RL" => b"RunLengthDecode".to_vec(),
            b"CCF" => b"CCITTFaxDecode".to_vec(),
            b"DCT" => b"DCTDecode".to_vec(),
            other => other.to_vec(),
        }
    };
    let names = |o: &Object| -> Object {
        match o {
            Object::Name(n) => Object::Name(full_name(n)),
            Object::Array(a) => Object::Array(
                a.iter()
                    .map(|x| match x {
                        Object::Name(n) => Object::Name(full_name(n)),
                        x => x.clone(),
                    })
                    .collect(),
            ),
            o => o.clone(),
        }
    };
    let mut d = Dictionary::new();
    d.set("Type", "XObject");
    d.set("Subtype", "Image");
    for pair in entry.operands.chunks(2) {
        let [Object::Name(k), v] = pair else {
            return None;
        };
        let key: &[u8] = match k.as_slice() {
            b"BPC" => b"BitsPerComponent",
            b"CS" => b"ColorSpace",
            b"D" => b"Decode",
            b"DP" => b"DecodeParms",
            b"F" => b"Filter",
            b"H" => b"Height",
            b"IM" => b"ImageMask",
            b"I" => b"Interpolate",
            b"W" => b"Width",
            b"L" | b"Length" => continue,
            k => k,
        };
        let v = match key {
            b"Filter" => names(v),
            b"ColorSpace" => match v {
                // A colour space from the resources by name.
                Object::Name(n) if spaces.has(n) => spaces.get(n).ok()?.clone(),
                v => names(v),
            },
            _ => v.clone(),
        };
        d.set(key.to_vec(), v);
    }
    Some(Stream::new(d, data.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_rects_follow_the_picture_square() {
        // A 10×10 picture drawn 100 pt square at (0, 0): row 0 is the top.
        let ctm = [100.0, 0.0, 0.0, 100.0, 0.0, 0.0];
        let r = pixel_rects(10, 10, ctm, &[[15.0, 75.0, 35.0, 95.0]]).unwrap();
        assert_eq!(r, vec![[1, 0, 4, 3]]);
        assert!(pixel_rects(10, 10, ctm, &[[150.0, 0.0, 160.0, 10.0]])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn clears_bits_inside_bytes() {
        let mut s = Samples {
            w: 10,
            h: 2,
            n: 1,
            bpc: 1,
            data: vec![0xFF; 4],
            codec: Codec::Raw,
        };
        s.clear([3, 1, 6, 2]);
        assert_eq!(s.data, vec![0xFF, 0xFF, 0b1110_0011, 0xFF]);
        assert!(s.is_clear([3, 1, 6, 2]) && !s.is_clear([2, 1, 6, 2]));
    }

    #[test]
    fn estimates_a_jpeg_quality() {
        let img = image::GrayImage::from_fn(32, 32, |x, y| image::Luma([(x * 7 + y * 3) as u8]));
        for q in [50u8, 75, 90] {
            let mut out = Vec::new();
            JpegEncoder::new_with_quality(&mut out, q)
                .encode(img.as_raw(), 32, 32, ExtendedColorType::L8)
                .unwrap();
            let est = crate::imgs::jpeg_quality(&out).unwrap();
            assert!(est.abs_diff(q) <= 2, "{q}: {est}");
        }
    }

    #[test]
    fn reads_inline_image_abbreviations() {
        let raw = b"BI /W 2 /H 1 /CS /RGB /BPC 8 /F /AHx ID 00ff00ff0000> EI";
        let s = inline_stream(raw, &Dictionary::new()).unwrap();
        assert_eq!(
            s.dict.get(b"ColorSpace").unwrap().as_name().unwrap(),
            b"DeviceRGB"
        );
        assert_eq!(
            s.dict.get(b"Filter").unwrap().as_name().unwrap(),
            b"ASCIIHexDecode"
        );
        assert_eq!(
            s.decompressed_content().unwrap(),
            vec![0, 255, 0, 255, 0, 0]
        );
    }
}
