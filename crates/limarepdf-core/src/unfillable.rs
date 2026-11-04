//! Pictures whose stated size their data can't come near filling (PB-81, found by fuzzing).
//! hayro sizes its buffers from a picture's stated width and height, not from its data: a
//! few bytes stating 255,555,555 × 2 pixels make it ask for gigabytes before it finds the
//! data short. In the copies pages are drawn from, such a picture is swapped for a
//! one-pixel picture that can't be decoded, so hayro leaves it out and the renderer reports
//! it as it reports any picture it can't decode ([`crate::render::Missing::Picture`]). The
//! document keeps the picture as it is.
//!
//! The app takes a snapshot after every edit, and looking for inline images means decoding
//! every content stream, so streams found fine are remembered ([`Checked`]) and only new or
//! changed ones are looked at again.

use crate::redact_img::{components, inline_stream};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashSet};
use std::hash::{Hash, Hasher};

/// Pictures stating more pixels than this are checked; smaller ones are drawn as they are,
/// however short their data (hayro fills in the rest).
const LARGE: u64 = 16 << 20;
/// A large picture whose data fills less than one part in this many of its stated size is
/// left out.
const PART: u64 = 8;
/// The most a content stream is decoded to, to look for inline images in it.
const MAX_CONTENT: usize = 1 << 30;

/// Streams already found fine, by [`fingerprint`]: a document's are checked once, then only
/// those that change.
#[derive(Clone, Default)]
pub(crate) struct Checked(HashSet<u64>);

impl Checked {
    /// The most remembered before starting again.
    const MOST: usize = 1 << 20;

    fn fine(&mut self, id: ObjectId, s: &Stream, check: impl FnOnce() -> bool) -> bool {
        let f = fingerprint(id, s);
        if self.0.contains(&f) {
            return true;
        }
        let fine = check();
        if fine {
            if self.0.len() >= Self::MOST {
                self.0.clear();
            }
            self.0.insert(f);
        }
        fine
    }
}

/// Stream `id`'s fingerprint for [`Checked`]: its number, its dictionary, its length and 16
/// spans of 32 bytes spread over its data, start and end among them. Reading every byte
/// would cost nearly as much as checking again. What LimarePDF writes itself never states
/// more than its data holds; what a file brings comes with bytes of its own.
fn fingerprint(id: ObjectId, s: &Stream) -> u64 {
    const SPAN: usize = 32;
    const SPANS: usize = 16;
    let mut h = DefaultHasher::new();
    id.hash(&mut h);
    hash_dict(&s.dict, &mut h);
    let c = &s.content;
    c.len().hash(&mut h);
    if c.len() <= SPAN * SPANS {
        c.hash(&mut h);
    } else {
        let step = (c.len() - SPAN) / (SPANS - 1);
        for i in 0..SPANS {
            c[i * step..i * step + SPAN].hash(&mut h);
        }
    }
    h.finish()
}

fn hash_dict(d: &Dictionary, h: &mut DefaultHasher) {
    for (k, v) in d.iter() {
        k.hash(h);
        hash_object(v, h);
    }
}

fn hash_object(o: &Object, h: &mut DefaultHasher) {
    std::mem::discriminant(o).hash(h);
    match o {
        Object::Null => {}
        Object::Boolean(b) => b.hash(h),
        Object::Integer(i) => i.hash(h),
        Object::Real(r) => r.to_bits().hash(h),
        Object::Name(n) | Object::String(n, _) => n.hash(h),
        Object::Array(a) => a.iter().for_each(|x| hash_object(x, h)),
        Object::Dictionary(d) => hash_dict(d, h),
        Object::Stream(s) => hash_dict(&s.dict, h),
        Object::Reference(r) => r.hash(h),
    }
}

/// Swap, in `doc`, every picture its data can't fill for one that can't be decoded (see
/// the module notes).
pub(crate) fn leave_out(doc: &mut Document) {
    let swaps = swaps(doc, &mut Checked::default());
    put(doc, swaps);
}

/// Put `swaps` ([`swaps`] of `doc` or of what it was copied from) in `doc`.
pub(crate) fn put(doc: &mut Document, swaps: Vec<(ObjectId, Stream)>) {
    for (id, s) in swaps {
        doc.objects.insert(id, Object::Stream(s));
    }
}

/// What [`leave_out`] would put in `doc`: for a picture object, the picture; for a content
/// stream drawing an inline image, the stream (decoded, the picture swapped). Streams in
/// `checked` are passed over, and those found fine added to it.
pub(crate) fn swaps(doc: &Document, checked: &mut Checked) -> Vec<(ObjectId, Stream)> {
    let mut out = Vec::new();
    let mut contents = BTreeSet::new();
    for (&id, o) in &doc.objects {
        match o {
            Object::Stream(s) => match s.dict.get(b"Subtype").and_then(Object::as_name) {
                Ok(b"Image")
                    if large(doc, &s.dict) && !checked.fine(id, s, || !unfillable(doc, s)) =>
                {
                    out.push((id, broken()))
                }
                Ok(b"Form") => {
                    contents.insert(id);
                }
                _ if matches!(s.dict.get(b"PatternType").and_then(Object::as_i64), Ok(1)) => {
                    contents.insert(id);
                }
                _ => {}
            },
            // A Type 3 font's glyphs are content too.
            Object::Dictionary(d)
                if matches!(d.get(b"Subtype").and_then(Object::as_name), Ok(b"Type3")) =>
            {
                let procs = d
                    .get(b"CharProcs")
                    .ok()
                    .and_then(|o| doc.dereference(o).ok());
                if let Some((_, Object::Dictionary(procs))) = procs {
                    contents.extend(procs.iter().filter_map(|(_, r)| r.as_reference().ok()));
                }
            }
            _ => {}
        }
    }
    for page in doc.get_pages().into_values() {
        contents.extend(doc.get_page_contents(page));
    }
    for id in contents {
        let Ok(s) = doc.get_object(id).and_then(Object::as_stream) else {
            continue;
        };
        let mut swap = None;
        checked.fine(id, s, || {
            swap = content_swap(doc, s);
            swap.is_none()
        });
        out.extend(swap.map(|s| (id, s)));
    }
    out
}

/// Content stream `s` with every inline image its data can't fill swapped, decoded; `None`
/// when there is none.
fn content_swap(doc: &Document, s: &Stream) -> Option<Stream> {
    let decoded;
    let data = if s.dict.has(b"Filter") {
        decoded = s.decompressed_content_with_limit(MAX_CONTENT).ok()?;
        &decoded
    } else {
        &s.content
    };
    if !data.windows(2).any(|w| w == b"BI") {
        return None;
    }
    let (stripped, inline) = crate::inline::extract(data).ok()?;
    let mut any = false;
    let content = inline.restore(&stripped, |raw| {
        let picture = inline_stream(raw, &Dictionary::new());
        if picture.is_some_and(|p| unfillable(doc, &p)) {
            any = true;
            BROKEN_INLINE.to_vec()
        } else {
            raw.to_vec()
        }
    });
    if !any {
        return None;
    }
    let mut dict = s.dict.clone();
    dict.remove(b"Filter");
    dict.remove(b"DecodeParms");
    Some(Stream::new(dict, content))
}

/// What stands in for a picture left out: a one-pixel grey picture whose data isn't a JPEG.
fn broken() -> Stream {
    Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 1,
            "Height" => 1,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        },
        BROKEN_DATA.to_vec(),
    )
}

const BROKEN_DATA: &[u8] = b"not a picture";
/// [`broken`] as an inline image.
const BROKEN_INLINE: &[u8] = b"BI /W 1 /H 1 /CS /G /BPC 8 /F /DCT ID not a picture EI";

fn int(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<i64> {
    doc.dereference(d.get(key).ok()?).ok()?.1.as_i64().ok()
}

/// A picture's stated width and height, from its dictionary `d`.
fn stated(doc: &Document, d: &Dictionary) -> Option<(u64, u64)> {
    let w = u64::try_from(int(doc, d, b"Width")?).ok()?;
    let h = u64::try_from(int(doc, d, b"Height")?).ok()?;
    Some((w, h))
}

/// Whether a picture states more than [`LARGE`] pixels: only those are checked further.
fn large(doc: &Document, d: &Dictionary) -> bool {
    stated(doc, d).is_some_and(|(w, h)| w.saturating_mul(h) > LARGE)
}

/// Whether picture `s` states more than [`LARGE`] pixels and its data fills less than one
/// [`PART`] of them. Data in filters that keep no count of what they hold (JPEG, JPEG 2000,
/// JBIG2), or that LimarePDF can't decode, is left to hayro, as is a fax picture's width:
/// only its rows are bounded by its data (each takes at least a bit).
pub(crate) fn unfillable(doc: &Document, s: &Stream) -> bool {
    let d = &s.dict;
    let Some((w, h)) = stated(doc, d) else {
        return false;
    };
    if !large(doc, d) {
        return false;
    }
    let mask = matches!(d.get(b"ImageMask"), Ok(Object::Boolean(true)));
    // When unsure, the fewest bits a pixel: the least data the picture can need.
    let (bits, comps) = if mask {
        (1, 1)
    } else {
        let bits = int(doc, d, b"BitsPerComponent").filter(|b| (1..=16).contains(b));
        let cs = d.get(b"ColorSpace").ok();
        let comps = cs.and_then(|cs| components(doc, cs, &Dictionary::new(), 0));
        (bits.unwrap_or(1) as u64, comps.unwrap_or(1) as u64)
    };
    let needed = (w.saturating_mul(comps).saturating_mul(bits))
        .div_ceil(8)
        .saturating_mul(h);

    let direct = crate::imgs::direct_parms(doc, s);
    let s = direct.as_ref().unwrap_or(s);
    if !s.dict.has(b"Filter") {
        return (s.content.len() as u64).saturating_mul(PART) < needed;
    }
    let Ok(filters) = s.filters() else {
        return false;
    };
    let lengthwise = |f: &[u8]| {
        matches!(
            f,
            b"FlateDecode"
                | b"LZWDecode"
                | b"ASCII85Decode"
                | b"ASCIIHexDecode"
                | b"RunLengthDecode"
        )
    };
    let Some((&last, before)) = filters.split_last() else {
        return false;
    };
    if !before.iter().all(|f| lengthwise(f)) {
        return false;
    }
    if last == b"CCITTFaxDecode" {
        let parms = match s.dict.get(b"DecodeParms") {
            Ok(Object::Array(a)) => a.last().and_then(|p| p.as_dict().ok()),
            Ok(Object::Dictionary(p)) => Some(p),
            _ => None,
        };
        let rows = parms.and_then(|p| int(doc, p, b"Rows")).unwrap_or(0);
        let rows = u64::try_from(rows).unwrap_or(0).max(h);
        let cap = rows.div_ceil(8 * PART);
        let len = if before.is_empty() {
            Some(s.content.len() as u64)
        } else {
            let mut prefix = s.clone();
            let names = before.iter().map(|f| Object::Name(f.to_vec())).collect();
            prefix.dict.set("Filter", Object::Array(names));
            match s.dict.get(b"DecodeParms") {
                Ok(Object::Array(a)) => {
                    let kept = a[..a.len().saturating_sub(1)].to_vec();
                    prefix.dict.set("DecodeParms", Object::Array(kept));
                }
                _ => {
                    prefix.dict.remove(b"DecodeParms");
                }
            }
            decoded_len(&prefix, cap)
        };
        return len.is_some_and(|len| len.saturating_mul(8 * PART) < rows);
    }
    if !lengthwise(last) {
        return false;
    }
    decoded_len(s, needed / PART).is_some_and(|len| len.saturating_mul(PART) < needed)
}

/// How long `s` decodes to, if no longer than `cap`: `None` when it is longer or can't be
/// decoded (left to hayro).
fn decoded_len(s: &Stream, cap: u64) -> Option<u64> {
    let cap = usize::try_from(cap).unwrap_or(usize::MAX);
    s.decompressed_content_with_limit(cap)
        .ok()
        .map(|d| d.len() as u64)
}
