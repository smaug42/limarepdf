//! Reducing pictures (ENH-19): Word's Compress Pictures for a PDF. Every picture drawn on the
//! chosen pages, in their content and the drawings (form XObjects) it uses, is measured at
//! the largest size those pages show it; above the chosen resolution it is scaled down and
//! saved again, its soft mask with it, and replaced only when the result is smaller.
//!
//! A picture is replaced in place, and only when everything that refers to it was walked on
//! the chosen pages. One also drawn on a page not chosen, or in a comment's or field's
//! drawing, or a pattern, is left as it is: a smaller copy beside the original the rest
//! still needs would make the file larger (pdfbox's `2990` grew by half when it was tried).
//!
//! Text, fonts, form fields, comments and the pages' content are never changed; only the
//! pictures' objects and, for a new one, the names in a page's resources.

use crate::matrix::{self as mx, M};
use crate::redact_img::{self, Codec, Samples};
use crate::{Pdf, Result};
use image::imageops::FilterType;
use image::{GrayImage, RgbImage};
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Deepest nesting of drawings followed.
const MAX_NESTING: usize = 12;
/// A picture within this share above the resolution is left as it is: scaling it down that
/// little would cost a round of compression for next to nothing.
const SLACK: f32 = 1.05;
/// Most distinct colours a picture saved losslessly (Flate) may have to stay lossless once
/// reduced: drawings, charts and screenshots, which JPEG would blur at their edges.
const FEW_COLOURS: usize = 256;

/// Word's Compress Pictures choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PictureQuality {
    /// Pictures as they are.
    HighFidelity,
    /// 330 pixels per inch.
    Hd,
    /// 220 pixels per inch.
    Print,
    /// 150 pixels per inch.
    Web,
    /// 96 pixels per inch.
    Email,
}

impl PictureQuality {
    pub const ALL: [PictureQuality; 5] = [
        PictureQuality::HighFidelity,
        PictureQuality::Hd,
        PictureQuality::Print,
        PictureQuality::Web,
        PictureQuality::Email,
    ];

    /// The resolution pictures are reduced to, in pixels per inch; `None`: as they are.
    pub fn ppi(self) -> Option<f32> {
        match self {
            PictureQuality::HighFidelity => None,
            PictureQuality::Hd => Some(330.0),
            PictureQuality::Print => Some(220.0),
            PictureQuality::Web => Some(150.0),
            PictureQuality::Email => Some(96.0),
        }
    }

    /// The JPEG quality pictures are saved at, 1 to 100.
    pub fn jpeg_quality(self) -> u8 {
        match self {
            PictureQuality::HighFidelity | PictureQuality::Hd => 90,
            PictureQuality::Print => 85,
            PictureQuality::Web => 80,
            PictureQuality::Email => 75,
        }
    }

    /// Its name, as Word gives it.
    pub fn name(self) -> &'static str {
        match self {
            PictureQuality::HighFidelity => "High fidelity",
            PictureQuality::Hd => "HD (330 ppi)",
            PictureQuality::Print => "Print (220 ppi)",
            PictureQuality::Web => "Web (150 ppi)",
            PictureQuality::Email => "Email (96 ppi)",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ReduceOptions {
    pub quality: PictureQuality,
    /// The JPEG quality instead of the quality's own, 1 to 100.
    pub jpeg_quality: Option<u8>,
}

impl Default for ReduceOptions {
    fn default() -> Self {
        Self {
            quality: PictureQuality::Print,
            jpeg_quality: None,
        }
    }
}

/// Why a picture was left as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeptWhy {
    /// Already at or under the resolution.
    Resolution,
    /// Of a kind not saved again: JPEG 2000, JBIG2, fax, CMYK and other colour spaces than
    /// grey and RGB, other than 8 bits a colour, a `/Decode` array, a stencil or colour-key
    /// mask, black and white.
    Kind,
    /// Saved again, it was no smaller.
    NotSmaller,
    /// Drawn only through something shared with what isn't being reduced: a drawing on pages
    /// not chosen, a comment's or field's drawing, a pattern.
    Shared,
    /// Its data couldn't be read.
    Unreadable,
    /// Written inside the page's content (an inline picture).
    Inline,
}

impl KeptWhy {
    /// What it means, for a report: "kept: …".
    pub fn describe(self) -> &'static str {
        match self {
            KeptWhy::Resolution => "already at or under the resolution",
            KeptWhy::Kind => "of a kind that can't be made smaller",
            KeptWhy::NotSmaller => "no smaller when saved again",
            KeptWhy::Shared => "shared with pages or items not being reduced",
            KeptWhy::Unreadable => "their data can't be read",
            KeptWhy::Inline => "written inside the page's content",
        }
    }
}

/// What reducing did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reduced {
    /// The pictures made smaller.
    pub pictures: usize,
    /// Their stored size (with their masks) before and after, in bytes.
    pub before: usize,
    pub after: usize,
    /// The pictures left as they are, by reason.
    pub kept: BTreeMap<KeptWhy, usize>,
    /// The picture that lost the most.
    pub most: Option<Most>,
}

/// The picture that lost the most bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Most {
    /// The first page that draws it (0-based).
    pub page: usize,
    /// The picture: as it was in the document before, as it is now.
    pub picture: ObjectId,
    pub before: usize,
    pub after: usize,
}

/// One place a picture is drawn.
struct Use {
    page: usize,
    ctm: M,
    /// The object holding the `/XObject` dictionary its name was found in.
    home: ObjectId,
}

/// What walking the pages found.
#[derive(Default)]
struct Walk {
    uses: HashMap<ObjectId, Vec<Use>>,
    /// The `/ColorSpace` resources of the first place each picture is drawn.
    spaces: HashMap<ObjectId, Dictionary>,
    /// Inline pictures on each page.
    inline: HashMap<usize, usize>,
}

/// Resources as found: the dictionary and the object holding it.
struct Res {
    dict: Dictionary,
    home: ObjectId,
}

impl Pdf {
    /// Reduce the pictures drawn on `pages` (0-based) as `opts` says. One change to the
    /// document; nothing else on the pages changes.
    pub fn reduce_pictures(&mut self, pages: &[usize], opts: &ReduceOptions) -> Result<Reduced> {
        let mut report = Reduced::default();
        let Some(target) = opts.quality.ppi() else {
            return Ok(report);
        };
        let jpeg = opts
            .jpeg_quality
            .unwrap_or(opts.quality.jpeg_quality())
            .clamp(1, 100);
        let ids = self.page_ids();
        let chosen: HashSet<usize> = pages.iter().copied().collect();
        if let Some(p) = pages.iter().find(|&&p| p >= ids.len()) {
            anyhow::bail!("no page {}", p + 1);
        }
        // Every page is walked: a picture also drawn on a page not chosen keeps its original
        // there.
        let mut walk = Walk::default();
        for (i, &page) in ids.iter().enumerate() {
            let Ok(data) = self
                .doc
                .get_page_content_with_limit(page, crate::MAX_CONTENT)
            else {
                continue;
            };
            let res = self.resources_of(page, self.inherited(page, b"Resources").as_ref());
            let mut forms = Vec::new();
            self.walk(i, &data, res.as_ref(), mx::ID, &mut forms, &mut walk);
        }
        for (page, n) in &walk.inline {
            if chosen.contains(page) {
                *report.kept.entry(KeptWhy::Inline).or_default() += n;
            }
        }
        let wanted: HashSet<ObjectId> = walk
            .uses
            .iter()
            .filter(|(_, u)| u.iter().any(|u| chosen.contains(&u.page)))
            .map(|(id, _)| *id)
            .collect();
        let referrers = referrers(&self.doc, &wanted);

        // Each picture everything refers to from the chosen pages, at the resolution it is
        // shown largest.
        struct Job<'a> {
            id: ObjectId,
            ppi: Option<f32>,
            spaces: &'a Dictionary,
        }
        let empty = Dictionary::new();
        let mut jobs = Vec::new();
        for &id in &wanted {
            let uses = &walk.uses[&id];
            let homes: HashSet<ObjectId> = uses.iter().map(|u| u.home).collect();
            let alone = uses.iter().all(|u| chosen.contains(&u.page))
                && referrers
                    .get(&id)
                    .is_none_or(|r| r.iter().all(|h| homes.contains(h)));
            if !alone {
                *report.kept.entry(KeptWhy::Shared).or_default() += 1;
                continue;
            }
            let size = self
                .doc
                .get_object(id)
                .ok()
                .and_then(|o| o.as_stream().ok());
            let ppi = size.and_then(|s| {
                let (w, h) = (int(&s.dict, b"Width")?, int(&s.dict, b"Height")?);
                uses.iter()
                    .filter_map(|u| shown_ppi(w as f32, h as f32, u.ctm))
                    .reduce(f32::min)
            });
            jobs.push(Job {
                id,
                ppi,
                spaces: walk.spaces.get(&id).unwrap_or(&empty),
            });
        }
        let doc = &self.doc;
        let done: Vec<(usize, std::result::Result<Redone, KeptWhy>)> = jobs
            .par_iter()
            .enumerate()
            .map(|(j, job)| {
                let r = match (doc.get_object(job.id).and_then(Object::as_stream), job.ppi) {
                    (Ok(s), Some(ppi)) => redo(doc, s, job.spaces, ppi, target, jpeg),
                    // Drawn with no size: nothing of it shows.
                    (Ok(_), None) => Err(KeptWhy::Resolution),
                    (Err(_), _) => Err(KeptWhy::Unreadable),
                };
                (j, r)
            })
            .collect();

        for (j, r) in done {
            let job = &jobs[j];
            let redone = match r {
                Ok(r) => r,
                Err(why) => {
                    *report.kept.entry(why).or_default() += 1;
                    continue;
                }
            };
            let mut image = redone.image;
            if let Some(mask) = redone.mask {
                let m = self.doc.add_object(mask);
                image.dict.set("SMask", m);
            }
            self.doc.objects.insert(job.id, Object::Stream(image));
            report.pictures += 1;
            report.before += redone.before;
            report.after += redone.after;
            let page = walk.uses[&job.id].iter().map(|u| u.page).min().unwrap_or(0);
            let lost = redone.before - redone.after;
            if report
                .most
                .as_ref()
                .is_none_or(|m| m.before - m.after < lost)
            {
                report.most = Some(Most {
                    page,
                    picture: job.id,
                    before: redone.before,
                    after: redone.after,
                });
            }
        }
        Ok(report)
    }

    /// Resources given by `o` (a dictionary or a reference to one) of the page or drawing
    /// `owner`, with the object holding its `/XObject` dictionary.
    fn resources_of(&self, owner: ObjectId, o: Option<&Object>) -> Option<Res> {
        let o = o?;
        let (home, dict) = match o {
            Object::Reference(r) => (*r, self.doc.get_dictionary(*r).ok()?),
            Object::Dictionary(d) => (owner, d),
            _ => return None,
        };
        let home = match dict.get(b"XObject") {
            Ok(Object::Reference(r)) => *r,
            _ => home,
        };
        Some(Res {
            dict: dict.clone(),
            home,
        })
    }

    /// Record every picture `data` draws (with resources `res`, from transform `ctm`) in
    /// `walk`, following drawings it uses; `forms` are the drawings being walked.
    fn walk(
        &self,
        page: usize,
        data: &[u8],
        res: Option<&Res>,
        ctm: M,
        forms: &mut Vec<ObjectId>,
        walk: &mut Walk,
    ) {
        let Ok((data, inline)) = crate::inline::extract(data) else {
            return;
        };
        let Ok(content) = Content::decode(&data) else {
            return;
        };
        let deref = |o: &Object| -> Option<Dictionary> {
            Some(self.doc.dereference(o).ok()?.1.as_dict().ok()?.clone())
        };
        let xobjects = res.and_then(|r| deref(r.dict.get(b"XObject").ok()?));
        let spaces = res
            .and_then(|r| deref(r.dict.get(b"ColorSpace").ok()?))
            .unwrap_or_default();
        let (mut ctm, mut stack) = (ctm, Vec::new());
        for op in &content.operations {
            match op.operator.as_str() {
                "q" => stack.push(ctm),
                "Q" => ctm = stack.pop().unwrap_or(ctm),
                "cm" => {
                    let n: Vec<f32> = op
                        .operands
                        .iter()
                        .filter_map(|o| o.as_float().ok())
                        .collect();
                    if let [a, b, c, d, e, f] = n[..] {
                        ctm = mx::mul([a, b, c, d, e, f], ctm);
                    }
                }
                "Do" => {
                    let (Some(Object::Name(name)), Some(xo), Some(res)) =
                        (op.operands.first(), xobjects.as_ref(), res)
                    else {
                        continue;
                    };
                    let Ok(Object::Reference(id)) = xo.get(name) else {
                        continue;
                    };
                    let Ok(s) = self.doc.get_object(*id).and_then(Object::as_stream) else {
                        continue;
                    };
                    match s.dict.get(b"Subtype").and_then(Object::as_name) {
                        Ok(b"Image") => {
                            walk.spaces.entry(*id).or_insert_with(|| spaces.clone());
                            walk.uses.entry(*id).or_default().push(Use {
                                page,
                                ctm,
                                home: res.home,
                            });
                        }
                        Ok(b"Form") if forms.len() < MAX_NESTING && !forms.contains(id) => {
                            let Ok(inner) = s.get_plain_content_with_limit(crate::MAX_CONTENT)
                            else {
                                continue;
                            };
                            let fm = s
                                .dict
                                .get(b"Matrix")
                                .ok()
                                .and_then(|o| self.matrix_of(o))
                                .unwrap_or(mx::ID);
                            // A drawing without resources of its own uses those it is drawn
                            // with.
                            let own = self.resources_of(*id, s.dict.get(b"Resources").ok());
                            let inner_res = own.as_ref().unwrap_or(res);
                            forms.push(*id);
                            self.walk(page, &inner, Some(inner_res), mx::mul(fm, ctm), forms, walk);
                            forms.pop();
                        }
                        _ => {}
                    }
                }
                t if inline.raw(t).is_some() => *walk.inline.entry(page).or_default() += 1,
                _ => {}
            }
        }
    }
}

fn int(d: &Dictionary, key: &[u8]) -> Option<i64> {
    d.get(key).ok()?.as_i64().ok()
}

/// The resolution a `w` × `h` picture is shown at when drawn with `ctm`: the lower of its
/// two directions' pixels per inch. `None` when it is drawn with no size.
fn shown_ppi(w: f32, h: f32, ctm: M) -> Option<f32> {
    let across = ctm[0].hypot(ctm[1]) / 72.0;
    let down = ctm[2].hypot(ctm[3]) / 72.0;
    (across > 1e-4 && down > 1e-4 && across.is_finite() && down.is_finite())
        .then(|| (w / across).min(h / down))
}

/// For each of `ids`, the objects whose values refer to it.
fn referrers(doc: &Document, ids: &HashSet<ObjectId>) -> HashMap<ObjectId, HashSet<ObjectId>> {
    fn each(o: &Object, f: &mut dyn FnMut(ObjectId)) {
        match o {
            Object::Reference(r) => f(*r),
            Object::Array(a) => a.iter().for_each(|o| each(o, f)),
            Object::Dictionary(d) => d.iter().for_each(|(_, o)| each(o, f)),
            Object::Stream(s) => s.dict.iter().for_each(|(_, o)| each(o, f)),
            _ => {}
        }
    }
    let mut out: HashMap<ObjectId, HashSet<ObjectId>> = HashMap::new();
    for (&holder, o) in &doc.objects {
        each(o, &mut |r| {
            if ids.contains(&r) {
                out.entry(r).or_default().insert(holder);
            }
        });
    }
    out
}

/// A picture saved again.
struct Redone {
    image: Stream,
    mask: Option<Stream>,
    before: usize,
    after: usize,
}

/// How many colour components `cs` has when it is grey or RGB (in any of their forms), or
/// `None` for any other colour space.
fn plain_components(doc: &Document, cs: &Object, spaces: &Dictionary) -> Option<usize> {
    let resolved = doc.dereference(cs).ok()?.1;
    let plain = match resolved {
        Object::Name(n) => match n.as_slice() {
            b"DeviceGray" | b"CalGray" | b"G" | b"DeviceRGB" | b"CalRGB" | b"RGB" => true,
            other => return plain_components(doc, spaces.get(other).ok()?, spaces),
        },
        Object::Array(a) => matches!(
            a.first().and_then(|o| o.as_name().ok()),
            Some(b"ICCBased" | b"CalGray" | b"CalRGB")
        ),
        _ => false,
    };
    let n = redact_img::components(doc, cs, spaces, 0)?;
    (plain && matches!(n, 1 | 3)).then_some(n)
}

/// Picture `s` (with `/ColorSpace` resources `spaces`), shown at `ppi`, saved again at
/// `target` pixels per inch, or why it is kept.
fn redo(
    doc: &Document,
    s: &Stream,
    spaces: &Dictionary,
    ppi: f32,
    target: f32,
    jpeg: u8,
) -> std::result::Result<Redone, KeptWhy> {
    if ppi <= target * SLACK {
        return Err(KeptWhy::Resolution);
    }
    let d = &s.dict;
    let stencil = d
        .get(b"ImageMask")
        .and_then(Object::as_bool)
        .unwrap_or(false);
    if stencil || d.has(b"Mask") || d.has(b"Decode") || int(d, b"BitsPerComponent") != Some(8) {
        return Err(KeptWhy::Kind);
    }
    let filters: Vec<&[u8]> = s.filters().unwrap_or_default();
    let jpeg_in = filters.as_slice() == [b"DCTDecode".as_slice()];
    if !filters.iter().all(|f| {
        matches!(
            *f,
            b"DCTDecode"
                | b"FlateDecode"
                | b"LZWDecode"
                | b"RunLengthDecode"
                | b"ASCII85Decode"
                | b"ASCIIHexDecode"
        )
    }) || (filters.contains(&b"DCTDecode".as_slice()) && !jpeg_in)
    {
        return Err(KeptWhy::Kind);
    }
    let cs = d.get(b"ColorSpace").map_err(|_| KeptWhy::Kind)?;
    let n = plain_components(doc, cs, spaces).ok_or(KeptWhy::Kind)?;
    if jpeg_in && crate::imgs::jpeg_components(&s.content) != Some(n as u8) {
        return Err(KeptWhy::Kind);
    }
    let px = redact_img::decode(doc, s, spaces, false).map_err(|_| KeptWhy::Unreadable)?;
    if px.n != n || px.bpc != 8 {
        return Err(KeptWhy::Kind);
    }
    let mask = match d.get(b"SMask") {
        Ok(m) => {
            let ms = doc
                .dereference(m)
                .and_then(|(_, o)| o.as_stream())
                .map_err(|_| KeptWhy::Unreadable)?;
            if ms.dict.has(b"Decode") || int(&ms.dict, b"BitsPerComponent") != Some(8) {
                return Err(KeptWhy::Kind);
            }
            let mp = redact_img::decode(doc, ms, spaces, false).map_err(|_| KeptWhy::Unreadable)?;
            if mp.n != 1 || mp.bpc != 8 {
                return Err(KeptWhy::Kind);
            }
            Some((ms, mp))
        }
        Err(_) => None,
    };
    let scale = target / ppi;
    let size = |w: usize, h: usize| {
        (
            ((w as f32 * scale).round() as usize).max(1),
            ((h as f32 * scale).round() as usize).max(1),
        )
    };
    let (w, h) = size(px.w, px.h);
    let codec = if jpeg_in {
        let own = crate::imgs::jpeg_quality(&s.content);
        Codec::Jpeg(own.map_or(jpeg, |q| q.min(jpeg)))
    } else if few_colours(&px) {
        Codec::Raw
    } else {
        Codec::Jpeg(jpeg)
    };
    let before = s.content.len() + mask.as_ref().map_or(0, |(ms, _)| ms.content.len());
    let data = resize(&px, w, h);
    let image = redact_img::encode(
        s,
        Samples {
            w,
            h,
            n,
            bpc: 8,
            data,
            codec,
        },
    )
    .map_err(|_| KeptWhy::Unreadable)?;
    let mask = match mask {
        Some((ms, mp)) => {
            // A mask the picture's size (one with `/Matte` must be) stays its size.
            let (mw, mh) = if (mp.w, mp.h) == (px.w, px.h) {
                (w, h)
            } else {
                size(mp.w, mp.h)
            };
            let data = resize(&mp, mw, mh);
            let m = Samples {
                w: mw,
                h: mh,
                n: 1,
                bpc: 8,
                data,
                codec: Codec::Raw,
            };
            Some(redact_img::encode(ms, m).map_err(|_| KeptWhy::Unreadable)?)
        }
        None => None,
    };
    let after = image.content.len() + mask.as_ref().map_or(0, |m| m.content.len());
    if after >= before {
        return Err(KeptWhy::NotSmaller);
    }
    Ok(Redone {
        image,
        mask,
        before,
        after,
    })
}

/// Whether 8-bit picture `px` has at most [`FEW_COLOURS`] colours.
fn few_colours(px: &Samples) -> bool {
    let mut seen = HashSet::new();
    for c in px.data.chunks_exact(px.n) {
        seen.insert(c);
        if seen.len() > FEW_COLOURS {
            return false;
        }
    }
    true
}

/// 8-bit samples `px` (grey or RGB) scaled to `w` × `h`.
fn resize(px: &Samples, w: usize, h: usize) -> Vec<u8> {
    let (sw, sh) = (px.w as u32, px.h as u32);
    let (w, h) = (w as u32, h as u32);
    if px.n == 1 {
        let img = GrayImage::from_raw(sw, sh, px.data.clone()).expect("the samples' size");
        image::imageops::resize(&img, w, h, FilterType::CatmullRom).into_raw()
    } else {
        let img = RgbImage::from_raw(sw, sh, px.data.clone()).expect("the samples' size");
        image::imageops::resize(&img, w, h, FilterType::CatmullRom).into_raw()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_resolution_shown_is_the_lower_of_both_directions_and_follows_turns() {
        // 600 × 300 pixels drawn 2 × 1 inches: 300 ppi both ways.
        assert_eq!(
            shown_ppi(600.0, 300.0, [144.0, 0.0, 0.0, 72.0, 0.0, 0.0]),
            Some(300.0)
        );
        // Squeezed across: 600 over 1 inch, 300 over 1 inch.
        assert_eq!(
            shown_ppi(600.0, 300.0, [72.0, 0.0, 0.0, 72.0, 0.0, 0.0]),
            Some(300.0)
        );
        // Turned a quarter: the same sizes.
        let turned = shown_ppi(600.0, 300.0, [0.0, 144.0, -72.0, 0.0, 0.0, 0.0]).unwrap();
        assert!((turned - 300.0).abs() < 1e-3);
        assert_eq!(
            shown_ppi(600.0, 300.0, [0.0, 0.0, 0.0, 72.0, 0.0, 0.0]),
            None
        );
    }

    #[test]
    fn few_colours_counts_up_to_its_limit() {
        let px = |data: Vec<u8>| Samples {
            w: data.len() / 3,
            h: 1,
            n: 3,
            bpc: 8,
            data,
            codec: Codec::Raw,
        };
        assert!(few_colours(&px([1, 2, 3].repeat(1000))));
        let many: Vec<u8> = (0..300u32)
            .flat_map(|i| [(i % 256) as u8, (i / 256) as u8, 0])
            .collect();
        assert!(!few_colours(&px(many)));
    }
}
