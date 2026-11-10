//! Saving pictures (NEW-09's stage 2): Word's Save as Picture for a picture on a page, and
//! every picture of a document to a folder.
//!
//! A JPEG upright on the page that other apps show as a PDF reader does (grey or colour, no
//! mask, no inverted colours, no EXIF turn) is written byte for byte: no quality lost. Any
//! other picture, or one turned or flipped on the page, is drawn alone, at its own pixel size,
//! on a page of that size by the same renderer as the pages, and saved as a lossless PNG as
//! it shows: CMYK, palettes, masks and every kind the renderer reads, its transparency kept.
//! The same drawing gives a picture's pixels for the system's clipboard (NEW-09's stage 3).

use crate::matrix::{self as mx, M};
use crate::reduce::{Drawn, Seen};
use crate::{redact_img, render, Pdf, Result};
use anyhow::{anyhow, bail, Context};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Most pixels drawn at once while saving every picture, about 1 GB of bitmaps.
const BATCH_PIXELS: u64 = 1 << 28;

/// How a picture is saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PictureFormat {
    /// The picture's own JPEG data, as it is.
    Jpeg,
    /// A lossless PNG of the picture as it shows.
    Png,
}

impl PictureFormat {
    pub fn extension(self) -> &'static str {
        match self {
            PictureFormat::Jpeg => "jpg",
            PictureFormat::Png => "png",
        }
    }

    /// Its name in a file chooser's list of kinds.
    pub fn name(self) -> &'static str {
        match self {
            PictureFormat::Jpeg => "JPEG picture",
            PictureFormat::Png => "PNG picture",
        }
    }
}

/// What [`Pdf::save_all_pictures`] did.
#[derive(Debug, Default)]
pub struct SavedPictures {
    /// The files written, in page order.
    pub written: Vec<PathBuf>,
    /// The pictures that couldn't be saved: the file's name and why.
    pub failed: Vec<(String, String)>,
}

/// A picture to save: what it is and where it shows, from its unit square to the displayed
/// page (y down).
struct Found<'a> {
    page: usize,
    /// Its place among the pictures its page draws, from 1.
    number: usize,
    picture: &'a Stream,
    shown: M,
}

impl Pdf {
    /// How the picture that is object `id` of page `page_index` (0-based) is saved.
    pub fn picture_format(&self, page_index: usize, id: usize) -> Result<PictureFormat> {
        let (s, shown) = self.picture_at(page_index, id)?;
        Ok(format_of(&self.doc, &s, shown))
    }

    /// The picture that is object `id` of page `page_index` (0-based) as a file: its JPEG
    /// data as it is, or a PNG as it shows.
    pub fn picture_file(&self, page_index: usize, id: usize) -> Result<(PictureFormat, Vec<u8>)> {
        let (s, shown) = self.picture_at(page_index, id)?;
        let format = format_of(&self.doc, &s, shown);
        let bytes = match format {
            PictureFormat::Jpeg => s.content.clone(),
            PictureFormat::Png => self.picture_png(&s, shown)?,
        };
        Ok((format, bytes))
    }

    /// The picture that is object `id` of page `page_index` (0-based) as it shows on the page,
    /// at its own pixel size, turned and flipped as it shows: its colours not multiplied by
    /// its transparency, as the system's clipboard takes a picture (NEW-09's stage 3).
    pub fn picture_pixels(&self, page_index: usize, id: usize) -> Result<render::Bitmap> {
        let (s, shown) = self.picture_at(page_index, id)?;
        self.picture_bitmap(&s, shown)
    }

    /// The file names [`Pdf::save_all_pictures`] gives every picture the pages draw, in page
    /// order: `<stem>-page-<page>-image-<n>.<ext>`, `n` its place among the pictures its page
    /// draws, numbers padded so the files sort in order. A picture drawn on several pages is
    /// saved once, with the first.
    pub fn picture_names(&self, stem: &str) -> Vec<String> {
        let seen = self.walk_pages().seen;
        let found = self.found(&seen);
        names(&found, stem, self.page_count())
            .into_iter()
            .zip(&found)
            .map(|(n, f)| {
                format!(
                    "{n}.{}",
                    format_of(&self.doc, f.picture, f.shown).extension()
                )
            })
            .collect()
    }

    /// Save every picture the pages draw, in their content and the drawings it uses, to
    /// `dir` with the names [`Pdf::picture_names`] gives, each whole or not at all, replacing
    /// files of the same name. A picture that can't be saved is reported and the others go
    /// on. When every file was written but one wasn't confirmed on disk, the error is that
    /// file's [`crate::NotConfirmed`].
    pub fn save_all_pictures(&self, dir: &Path, stem: &str) -> Result<SavedPictures> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let seen = self.walk_pages().seen;
        let found = self.found(&seen);
        let stems = names(&found, stem, self.page_count());
        let mut out = SavedPictures::default();
        let mut unconfirmed = None;
        // Drawn in parallel, in batches that keep the bitmaps to about 1 GB.
        let mut start = 0;
        while start < found.len() {
            let mut end = start;
            let mut pixels = 0;
            while end < found.len() && (end == start || pixels < BATCH_PIXELS) {
                pixels += pixels_of(&found[end]);
                end += 1;
            }
            let files: Vec<(String, Result<Vec<u8>>)> = (start..end)
                .into_par_iter()
                .map(|i| {
                    let f = &found[i];
                    let format = format_of(&self.doc, f.picture, f.shown);
                    let name = format!("{}.{}", stems[i], format.extension());
                    let bytes = match format {
                        PictureFormat::Jpeg => Ok(f.picture.content.clone()),
                        PictureFormat::Png => self.picture_png(f.picture, f.shown),
                    };
                    (name, bytes)
                })
                .collect();
            for (name, bytes) in files {
                let path = dir.join(&name);
                match bytes.and_then(|b| crate::write_file(&path, &b)) {
                    Ok(()) => out.written.push(path),
                    // Written: the others go on.
                    Err(e) if e.is::<crate::NotConfirmed>() => {
                        out.written.push(path);
                        unconfirmed.get_or_insert(e);
                    }
                    Err(e) => out.failed.push((name, format!("{e:#}"))),
                }
            }
            start = end;
        }
        unconfirmed.map_or(Ok(out), Err)
    }

    /// The pictures of `seen` to save, each object once, with its place on its page.
    fn found<'a>(&'a self, seen: &'a [Seen]) -> Vec<Found<'a>> {
        let ids = self.page_ids();
        let mut once = HashSet::new();
        let mut out = Vec::new();
        let (mut page, mut number, mut to_vis) = (usize::MAX, 0, None);
        for s in seen {
            if s.page != page {
                (page, number) = (s.page, 0);
                to_vis = ids.get(page).and_then(|&id| self.to_vis(id));
            }
            number += 1;
            let picture = match &s.picture {
                Drawn::Object(id) => {
                    let Ok(Object::Stream(p)) = self.doc.get_object(*id) else {
                        continue;
                    };
                    if !once.insert(*id) {
                        continue;
                    }
                    p
                }
                Drawn::Inline(p) => &**p,
            };
            let Some(to_vis) = to_vis else {
                continue;
            };
            out.push(Found {
                page: s.page,
                number,
                picture,
                shown: mx::mul(s.ctm, to_vis),
            });
        }
        out
    }

    /// From page `page`'s user space to the displayed page (y down).
    fn to_vis(&self, page: ObjectId) -> Option<M> {
        mx::inv(self.vis_down(page).0)
    }

    /// The picture that is object `id` of page `page_index`, as a picture object, and where it
    /// shows on the displayed page.
    fn picture_at(&self, page_index: usize, id: usize) -> Result<(Stream, M)> {
        let page = self.page_id_at(page_index)?;
        let parsed = self.parse_page(page_index)?;
        let obj = Self::pick(&parsed, id)?;
        if obj.kind != crate::ObjectKind::Image {
            bail!("that isn't a picture");
        }
        let res = self.inherited(page, b"Resources");
        let res = res.as_ref().and_then(|r| self.doc.dereference(r).ok());
        let res = res.and_then(|(_, r)| r.as_dict().ok());
        let named = |key: &[u8]| -> Option<&Dictionary> {
            self.doc
                .dereference(res?.get(key).ok()?)
                .ok()?
                .1
                .as_dict()
                .ok()
        };
        let op = &parsed.ops[obj.first];
        let picture = if let Some(raw) = parsed.inline.raw(&op.operator) {
            let spaces = named(b"ColorSpace").cloned().unwrap_or_default();
            redact_img::inline_stream(raw, &spaces)
        } else {
            let name = op.operands.first().and_then(|o| o.as_name().ok());
            let at = name.and_then(|n| named(b"XObject")?.get(n).ok());
            let at = at.and_then(|o| self.doc.dereference(o).ok());
            at.and_then(|(_, o)| o.as_stream().ok()).cloned()
        };
        let picture = picture.ok_or_else(|| anyhow!("the picture's data can't be found"))?;
        let to_vis = self
            .to_vis(page)
            .ok_or_else(|| anyhow!("the page can't be read"))?;
        Ok((picture, mx::mul(obj.ctm, to_vis)))
    }

    /// Picture `s`, shown as `shown` on the displayed page, as a PNG: drawn alone at its own
    /// pixel size, turned and flipped as it shows, on a clear backdrop.
    fn picture_png(&self, s: &Stream, shown: M) -> Result<Vec<u8>> {
        png_file(&self.picture_bitmap(s, shown)?)
    }

    /// Picture `s`, shown as `shown` on the displayed page, drawn alone at its own pixel size,
    /// turned and flipped as it shows, on a clear backdrop.
    fn picture_bitmap(&self, s: &Stream, shown: M) -> Result<render::Bitmap> {
        let (w, h) = size(s).ok_or_else(|| anyhow!("the picture has no usable size"))?;
        let (place, pw, ph) = placed(shown, w, h)?;
        if pw as u64 * ph as u64 > render::MAX_PIXELS {
            bail!("the picture is too large to save ({pw} × {ph} pixels)");
        }
        drawn_alone(&self.doc, s, place, pw, ph)
    }
}

/// Picture `s` of `doc` drawn alone by the renderer the pages are drawn with, placed by
/// `place` on a `pw` × `ph` page, on a clear backdrop: its colours as they show, in sRGB.
pub(crate) fn drawn_alone(
    doc: &Document,
    s: &Stream,
    place: M,
    pw: u32,
    ph: u32,
) -> Result<render::Bitmap> {
    let bytes = alone(doc, s, place, pw, ph)?;
    let pdf = render::load(bytes, None)?;
    let r = render::Renderer::new(&pdf);
    let b = r.render_clear(0)?;
    if !r.take_missing().is_empty() {
        bail!("the picture's data can't be read");
    }
    Ok(b)
}

/// A document of one `pw` × `ph` page drawing picture `s` placed by `place`, with the
/// objects the picture refers to (its mask, colour space and so on) copied from `doc`.
fn alone(doc: &Document, s: &Stream, place: M, pw: u32, ph: u32) -> Result<Vec<u8>> {
    let mut pic = s.clone();
    // What would hide it, or isn't needed to draw it.
    for k in [
        &b"OC"[..],
        b"Metadata",
        b"Alternates",
        b"OPI",
        b"StructParent",
    ] {
        pic.dict.remove(k);
    }
    let mut out = Document::with_version("1.7");
    let mut todo: Vec<ObjectId> = Vec::new();
    refs(&Object::Dictionary(pic.dict.clone()), &mut todo);
    while let Some(id) = todo.pop() {
        if out.objects.contains_key(&id) {
            continue;
        }
        let Ok(o) = doc.get_object(id) else {
            continue;
        };
        refs(o, &mut todo);
        out.objects.insert(id, o.clone());
        out.max_id = out.max_id.max(id.0);
    }
    let im = out.add_object(pic);
    let n = |v: f32| Object::Real(v);
    let content = lopdf::content::Content {
        operations: vec![
            lopdf::content::Operation::new("q", vec![]),
            lopdf::content::Operation::new("cm", place.iter().map(|&v| n(v)).collect()),
            lopdf::content::Operation::new("Do", vec![Object::Name(b"Im".to_vec())]),
            lopdf::content::Operation::new("Q", vec![]),
        ],
    };
    let content = out.add_object(Stream::new(Dictionary::new(), content.encode()?));
    let pages = out.new_object_id();
    let page = out.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages,
        "MediaBox" => vec![0.into(), 0.into(), (pw as i64).into(), (ph as i64).into()],
        "Resources" => dictionary! {"XObject" => dictionary! {"Im" => im}},
        "Contents" => content,
    });
    out.objects.insert(
        pages,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
        }),
    );
    let catalog = out.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages});
    out.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    out.save_to(&mut bytes)?;
    Ok(bytes)
}

/// How picture `s`, shown as `shown` on the displayed page, is saved: [`PictureFormat::Jpeg`]
/// for a JPEG upright on the page that other apps show as a PDF reader does.
fn format_of(doc: &Document, s: &Stream, shown: M) -> PictureFormat {
    if upright(shown) && jpeg_as_is(doc, s) {
        PictureFormat::Jpeg
    } else {
        PictureFormat::Png
    }
}

/// Whether a picture shown as `shown` is upright on the displayed page: its top edge at the
/// top, left at the left, not turned; stretched or not.
fn upright(shown: M) -> bool {
    let [a, b, c, d, _, _] = shown;
    a > 0.0 && d < 0.0 && b.abs() <= 1e-3 * a && c.abs() <= -1e-3 * d
}

/// Whether picture `s` is a JPEG another app shows as a PDF reader does, written as it is.
fn jpeg_as_is(doc: &Document, s: &Stream) -> bool {
    let d = &s.dict;
    let filters = s.filters().unwrap_or_default();
    // A mask, inverted colours (`/Decode`) or the colours left untransformed
    // (`/ColorTransform`) would show differently outside the PDF.
    let plain = filters == [b"DCTDecode"]
        && ["SMask", "Mask", "Decode", "ImageMask", "SMaskInData"]
            .iter()
            .all(|k| !d.has(k.as_bytes()))
        && d.get(b"DecodeParms")
            .ok()
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .is_none_or(|p| !p.has(b"ColorTransform"));
    let Some((w, h)) = size(s) else {
        return false;
    };
    plain
        && crate::imgs::jpeg_as_shown(&s.content, w as i64, h as i64)
        && plain_space(doc, d, crate::imgs::jpeg_components(&s.content))
}

/// Whether a picture's colour space is plain grey or RGB with `comps` components: what a
/// JPEG file holds without the PDF. A colour profile marked sRGB counts, as it shows the
/// same; another profile, or a calibrated space, shows its numbers as other colours.
fn plain_space(doc: &Document, d: &Dictionary, comps: Option<u8>) -> bool {
    let Ok(cs) = d.get(b"ColorSpace") else {
        return false;
    };
    matches!(
        crate::imgs::device_equivalent(doc, cs, &Dictionary::new()),
        crate::imgs::Space::Plain(n) if comps == Some(n as u8)
    )
}

/// Picture `s`'s width and height in pixels.
fn size(s: &Stream) -> Option<(u32, u32)> {
    let int = |k: &[u8]| s.dict.get(k).ok()?.as_i64().ok();
    let (w, h) = (int(b"Width")?, int(b"Height")?);
    let ok = |v: i64| (1..=1 << 16).contains(&v);
    (ok(w) && ok(h)).then_some((w as u32, h as u32))
}

/// About how many pixels saving `f` draws: none for a JPEG written as it is.
fn pixels_of(f: &Found) -> u64 {
    match size(f.picture) {
        Some((w, h)) => w as u64 * h as u64 * 2,
        None => 0,
    }
}

/// Where a `w` × `h` picture shown as `shown` goes on a page of its own: turned and flipped as
/// it shows, each side its own pixels long, the page fitting it. The page's user space
/// placement and its width and height.
fn placed(shown: M, w: u32, h: u32) -> Result<(M, u32, u32)> {
    let [a, b, c, d, _, _] = shown.map(|v| v as f64);
    let (lu, lv) = (a.hypot(b), c.hypot(d));
    if !(lu > 1e-9 && lv > 1e-9) {
        bail!("the picture is drawn with no size");
    }
    // Each side as long as its pixels, in the direction it shows. A side within a
    // ten-thousandth of upright or across is taken as exactly so, for whole pixels.
    let side = |x: f64, y: f64, l: f64, px: u32| {
        let (mut x, mut y) = (x / l, y / l);
        if x.abs() < 1e-4 {
            x = 0.0;
            y = y.signum();
        } else if y.abs() < 1e-4 {
            y = 0.0;
            x = x.signum();
        }
        (x * px as f64, y * px as f64)
    };
    let (ux, uy) = side(a, b, lu, w);
    let (vx, vy) = side(c, d, lv, h);
    let xs = [0.0, ux, vx, ux + vx];
    let ys = [0.0, uy, vy, uy + vy];
    let min = |v: [f64; 4]| v.into_iter().fold(f64::INFINITY, f64::min);
    let max = |v: [f64; 4]| v.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let (x0, y0) = (min(xs), min(ys));
    let pw = (max(xs) - x0 - 1e-6).ceil().max(1.0);
    let ph = (max(ys) - y0 - 1e-6).ceil().max(1.0);
    if pw > u16::MAX as f64 || ph > u16::MAX as f64 {
        bail!("the picture is too large to save");
    }
    // From the unit square to the page's pixels (y down), then to its user space (y up).
    let down = [ux, uy, vx, vy, -x0, -y0].map(|v| v as f32);
    let place = mx::mul(down, [1.0, 0.0, 0.0, -1.0, 0.0, ph as f32]);
    Ok((place, pw as u32, ph as u32))
}

/// Every reference in `o`, onto `to`.
fn refs(o: &Object, to: &mut Vec<ObjectId>) {
    match o {
        Object::Reference(r) => to.push(*r),
        Object::Array(a) => a.iter().for_each(|o| refs(o, to)),
        Object::Dictionary(d) => d.iter().for_each(|(_, o)| refs(o, to)),
        Object::Stream(s) => s.dict.iter().for_each(|(_, o)| refs(o, to)),
        _ => {}
    }
}

/// The file stems of `found`'s pictures: `<stem>-page-<page>-image-<n>`, page numbers padded
/// to three digits at least (as exported pages are) and picture numbers to the most on a page.
fn names(found: &[Found], stem: &str, pages: usize) -> Vec<String> {
    let pd = pages.to_string().len().max(3);
    let most = found.iter().map(|f| f.number).max().unwrap_or(1);
    let nd = most.to_string().len();
    found
        .iter()
        .map(|f| format!("{stem}-page-{:0pd$}-image-{:0nd$}", f.page + 1, f.number))
        .collect()
}

/// `b` (colours not multiplied by alpha) as a lossless PNG file: grey when every pixel is,
/// without alpha when every pixel is opaque.
pub fn png_file(b: &render::Bitmap) -> Result<Vec<u8>> {
    let px = b.rgba.as_chunks::<4>().0;
    let grey = px.iter().all(|p| p[0] == p[1] && p[1] == p[2]);
    let opaque = px.iter().all(|p| p[3] == 255);
    let (colour, data): (png::ColorType, Vec<u8>) = match (grey, opaque) {
        (true, true) => (png::ColorType::Grayscale, px.iter().map(|p| p[0]).collect()),
        (true, false) => (
            png::ColorType::GrayscaleAlpha,
            px.iter().flat_map(|p| [p[0], p[3]]).collect(),
        ),
        (false, true) => (
            png::ColorType::Rgb,
            px.iter().flat_map(|p| [p[0], p[1], p[2]]).collect(),
        ),
        (false, false) => (png::ColorType::Rgba, b.rgba.clone()),
    };
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, b.width as u32, b.height as u32);
    e.set_color(colour);
    e.set_depth(png::BitDepth::Eight);
    let mut wr = e.write_header()?;
    wr.write_image_data(&data)?;
    wr.finish()?;
    Ok(out)
}
