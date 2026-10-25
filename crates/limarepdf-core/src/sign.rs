//! Fill-and-sign artwork: signatures and initials (drawn with the pen, typed in a
//! handwriting font, or from a picture) and tick, cross and dot marks. [`crate::Stamp`]
//! puts them on pages as stamp annotations. Typed signatures are the font's glyph
//! outlines drawn as shapes, so no font goes into the PDF and they look the same in every
//! reader. None of this proves who signed: that is what digital signatures are for.

use crate::annot_write::simplify;
use crate::form_ap::nums;
use crate::form_flatten::Flat;
use crate::imgs::{decode, first, orient_matrix, stored_point, swaps_axes, Pixels, Raster};
use crate::overlay::{escape, local_today, text_width};
use crate::stamps::KAPPA;
use crate::{Pdf, Result, TextStyle};
use anyhow::{anyhow, bail};
use lopdf::{dictionary, Dictionary, ObjectId, Stream};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{FontRef, LocationRef, MetadataProvider, Size};
use std::collections::HashSet;

/// The handwriting fonts, under the SIL Open Font Licence or Apache 2.0 (see
/// docs/licence.md).
static CAVEAT: &[u8] = include_bytes!("../assets/fonts/Caveat.ttf");
static CEDARVILLE: &[u8] = include_bytes!("../assets/fonts/Cedarville-Cursive.ttf");
static HOMEMADE_APPLE: &[u8] = include_bytes!("../assets/fonts/HomemadeApple-Regular.ttf");
static ZEYADA: &[u8] = include_bytes!("../assets/fonts/Zeyada.ttf");

/// The size typed signatures are drawn at before they are fitted, in points.
const TYPE_SIZE: f32 = 40.0;
/// Resource names in the artwork.
const FONT: &str = "LimareF";
const IMAGE: &str = "LimareIm";
/// The date line's text size, and its gap below the signature, in points.
const DATE_SIZE: f32 = 9.0;
const DATE_GAP: f32 = 3.0;
/// Helvetica's capital height and descent, as shares of the text size.
const CAP: f32 = 0.72;
const DESCENT: f32 = 0.21;
/// A mark's box, in points.
const MARK: f32 = 12.0;
/// The most points kept from one pen stroke, and the most strokes.
const MAX_POINTS: usize = 20_000;
const MAX_STROKES: usize = 500;

/// A handwriting style for typed signatures, named after its font.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignStyle {
    /// Caveat: everyday handwriting, mostly unjoined.
    Caveat,
    /// Cedarville Cursive: joined school cursive.
    Cedarville,
    /// Homemade Apple: joined, as with a felt pen.
    HomemadeApple,
    /// Zeyada: a quick scrawl.
    Zeyada,
}

impl SignStyle {
    pub const ALL: [SignStyle; 4] = [
        SignStyle::Caveat,
        SignStyle::Cedarville,
        SignStyle::HomemadeApple,
        SignStyle::Zeyada,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SignStyle::Caveat => "Caveat",
            SignStyle::Cedarville => "Cedarville Cursive",
            SignStyle::HomemadeApple => "Homemade Apple",
            SignStyle::Zeyada => "Zeyada",
        }
    }

    /// Its number, 1 to 4, as the command line and the settings file give it.
    pub fn number(self) -> u8 {
        match self {
            SignStyle::Caveat => 1,
            SignStyle::Cedarville => 2,
            SignStyle::HomemadeApple => 3,
            SignStyle::Zeyada => 4,
        }
    }

    pub fn from_number(n: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.number() == n)
    }

    fn font(self) -> &'static [u8] {
        match self {
            SignStyle::Caveat => CAVEAT,
            SignStyle::Cedarville => CEDARVILLE,
            SignStyle::HomemadeApple => HOMEMADE_APPLE,
            SignStyle::Zeyada => ZEYADA,
        }
    }
}

/// The colour of a signature or mark: a black or a blue pen.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum InkColor {
    #[default]
    Black,
    Blue,
}

impl InkColor {
    pub const ALL: [InkColor; 2] = [InkColor::Black, InkColor::Blue];

    pub fn rgb(self) -> [f32; 3] {
        match self {
            InkColor::Black => [0.0, 0.0, 0.0],
            InkColor::Blue => [0.1, 0.2, 0.55],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            InkColor::Black => "black",
            InkColor::Blue => "blue",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|c| c.name().eq_ignore_ascii_case(s.trim()))
    }
}

/// Whether a signature is a full signature or initials; they differ only in size.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignRole {
    Signature,
    Initials,
}

impl SignRole {
    /// The stamp's `/Name`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            SignRole::Signature => "LimareSignature",
            SignRole::Initials => "LimareInitials",
        }
    }

    /// The box its artwork is fitted into at its own size, in points.
    fn fit(self) -> (f32, f32) {
        match self {
            SignRole::Signature => (150.0, 50.0),
            SignRole::Initials => (50.0, 30.0),
        }
    }
}

/// A mark for ticking boxes that aren't form fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    Tick,
    Cross,
    Dot,
}

impl Mark {
    pub const ALL: [Mark; 3] = [Mark::Tick, Mark::Cross, Mark::Dot];

    pub fn label(self) -> &'static str {
        match self {
            Mark::Tick => "Tick",
            Mark::Cross => "Cross",
            Mark::Dot => "Dot",
        }
    }

    /// The stamp's `/Name`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Mark::Tick => "LimareTick",
            Mark::Cross => "LimareCross",
            Mark::Dot => "LimareDot",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|m| m.label().eq_ignore_ascii_case(s.trim()))
    }
}

/// How a signature or initials are made.
#[derive(Clone, Copy, Debug)]
pub enum SignArt<'a> {
    /// Pen strokes, y down, in any units; `width` is the pen's width in the same units.
    /// Each stroke is smoothed into curves.
    Drawn {
        strokes: &'a [Vec<(f32, f32)>],
        width: f32,
    },
    /// A name in a handwriting style (see [`typed_signature`]).
    Typed { text: &'a str, style: SignStyle },
    /// A PNG, JPEG or TIFF picture. `crop` keeps only that part of it: its left, top,
    /// right and bottom as shares of the picture as it is seen (see [`picture_view`]), y
    /// down. `clear_paper` then makes the paper around the ink transparent and crops the
    /// empty margins.
    Picture {
        bytes: &'a [u8],
        clear_paper: bool,
        crop: Option<[f32; 4]>,
    },
}

/// What LimarePDF calls a fill-and-sign stamp, from its `/Name`; `None` for other stamps.
pub fn sign_label(name: &str) -> Option<&'static str> {
    Some(match name {
        "LimareSignature" => "Signature",
        "LimareInitials" => "Initials",
        _ => Mark::ALL.into_iter().find(|m| m.name() == name)?.label(),
    })
}

/// A name to sign with as it will be drawn: trimmed, with runs of spaces made single.
/// Fails when it is empty or has characters the handwriting style can't show, naming the
/// styles that can.
pub fn typed_signature(text: &str, style: SignStyle) -> Result<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        bail!("type a name to sign with");
    }
    let bad = unshown(&text, style)?;
    if !bad.is_empty() {
        let others: Vec<&str> = SignStyle::ALL
            .into_iter()
            .filter(|&s| s != style && unshown(&text, s).is_ok_and(|b| b.is_empty()))
            .map(SignStyle::label)
            .collect();
        let instead = match others.as_slice() {
            [] => String::new(),
            [one] => format!("{one} can show it, or "),
            [first @ .., last] => format!("{} and {last} can show it, or ", first.join(", ")),
        };
        bail!(
            "the {} handwriting can't show {bad:?}; {instead}draw your signature or use a picture \
             instead",
            style.label()
        );
    }
    Ok(text)
}

/// The characters of `text` the handwriting `style` can't show, each once.
fn unshown(text: &str, style: SignStyle) -> Result<String> {
    let font = FontRef::new(style.font()).map_err(|e| anyhow!("handwriting font: {e}"))?;
    let map = font.charmap();
    let mut bad = String::new();
    for c in text.chars() {
        if c != ' ' && map.map(c).is_none() && !bad.contains(c) {
            bad.push(c);
        }
    }
    Ok(bad)
}

/// A signature's or mark's drawing at its own size, ready to become a form XObject.
pub(crate) struct Art {
    /// Content stream operators in a `w` by `h` box, y up.
    ops: String,
    pub w: f32,
    pub h: f32,
    /// The date line uses Helvetica.
    font: bool,
    /// A picture, drawn as [`IMAGE`].
    image: Option<Raster>,
}

/// A number for a content stream: at most three decimals, no trailing zeros.
fn n(v: f32) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.into()
    }
}

/// A box that grows to hold points.
#[derive(Clone, Copy)]
struct Bounds([f32; 4]);

impl Bounds {
    const EMPTY: Bounds = Bounds([f32::MAX, f32::MAX, f32::MIN, f32::MIN]);

    fn add(&mut self, x: f32, y: f32, pad: f32) {
        let b = &mut self.0;
        b[0] = b[0].min(x - pad);
        b[1] = b[1].min(y - pad);
        b[2] = b[2].max(x + pad);
        b[3] = b[3].max(y + pad);
    }

    fn is_empty(&self) -> bool {
        self.0[2] <= self.0[0] || self.0[3] <= self.0[1]
    }
}

/// Glyph outlines as path operators, moved right by `dx`.
struct PathPen {
    ops: String,
    dx: f32,
    at: (f32, f32),
    bounds: Bounds,
}

impl PathPen {
    fn point(&mut self, x: f32, y: f32) -> String {
        let x = x + self.dx;
        self.bounds.add(x, y, 0.0);
        format!("{} {}", n(x), n(y))
    }
}

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.ops += &format!("{p} m\n");
        self.at = (x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.ops += &format!("{p} l\n");
        self.at = (x, y);
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        // The same curve as a cubic one.
        let (x0, y0) = self.at;
        let c1 = self.point(x0 + 2.0 / 3.0 * (cx - x0), y0 + 2.0 / 3.0 * (cy - y0));
        let c2 = self.point(x + 2.0 / 3.0 * (cx - x), y + 2.0 / 3.0 * (cy - y));
        let p = self.point(x, y);
        self.ops += &format!("{c1} {c2} {p} c\n");
        self.at = (x, y);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (c1, c2, p) = (self.point(cx0, cy0), self.point(cx1, cy1), self.point(x, y));
        self.ops += &format!("{c1} {c2} {p} c\n");
        self.at = (x, y);
    }

    fn close(&mut self) {
        self.ops += "h\n";
    }
}

/// A typed signature's outlines, filled, and their bounds (y up).
fn typed_ops(text: &str, style: SignStyle) -> Result<(String, Bounds)> {
    let text = typed_signature(text, style)?;
    let font = FontRef::new(style.font()).map_err(|e| anyhow!("handwriting font: {e}"))?;
    let (map, glyphs) = (font.charmap(), font.outline_glyphs());
    let size = Size::new(TYPE_SIZE);
    let metrics = font.glyph_metrics(size, LocationRef::default());
    let mut pen = PathPen {
        ops: String::new(),
        dx: 0.0,
        at: (0.0, 0.0),
        bounds: Bounds::EMPTY,
    };
    for c in text.chars() {
        let Some(gid) = map.map(c) else { continue };
        if let Some(g) = glyphs.get(gid) {
            g.draw(
                DrawSettings::unhinted(size, LocationRef::default()),
                &mut pen,
            )
            .map_err(|e| anyhow!("can't draw {c:?} in the handwriting font: {e}"))?;
        }
        pen.dx += metrics.advance_width(gid).unwrap_or(TYPE_SIZE / 2.0);
    }
    if pen.bounds.is_empty() {
        bail!("type a name to sign with");
    }
    Ok((pen.ops + "f\n", pen.bounds))
}

/// Curves through a stroke's points (Catmull–Rom as Bézier curves), y flipped so up is
/// up; one point makes a dot.
fn stroke_ops(points: &[(f32, f32)], width: f32, bounds: &mut Bounds) -> String {
    let pts: Vec<(f32, f32)> = points
        .iter()
        .take(MAX_POINTS)
        .filter(|p| p.0.is_finite() && p.1.is_finite())
        .map(|&(x, y)| (x, -y))
        .collect();
    let pts = simplify(&pts, width * 0.2);
    let Some(&(x0, y0)) = pts.first() else {
        return String::new();
    };
    for &(x, y) in &pts {
        bounds.add(x, y, width / 2.0);
    }
    let p = |(x, y): (f32, f32)| format!("{} {}", n(x), n(y));
    let mut s = format!("{} m\n", p((x0, y0)));
    if pts.len() == 1 {
        return s + &format!("{} l\n", p((x0, y0)));
    }
    let at = |i: isize| pts[i.clamp(0, pts.len() as isize - 1) as usize];
    for i in 0..pts.len() as isize - 1 {
        let (a, b, c, d) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let c1 = (b.0 + (c.0 - a.0) / 6.0, b.1 + (c.1 - a.1) / 6.0);
        let c2 = (c.0 - (d.0 - b.0) / 6.0, c.1 - (d.1 - b.1) / 6.0);
        s += &format!("{} {} {} c\n", p(c1), p(c2), p(c));
    }
    s
}

/// A picture's samples as 8-bit RGB with alpha, if it has a form that can be read so
/// (all but CMYK JPEGs).
fn rgb_alpha(r: &Raster) -> Result<(Vec<u8>, Option<Vec<u8>>)> {
    let (w, h) = (r.w as usize, r.h as usize);
    let (rgb, alpha) = match &r.pixels {
        Pixels::Jpeg {
            data, components, ..
        } if *components != 4 => {
            let img = image::load_from_memory_with_format(data, image::ImageFormat::Jpeg)
                .map_err(|e| anyhow!("cannot decode the picture: {e}"))?;
            (img.to_rgb8().into_raw(), None)
        }
        Pixels::Raw {
            color_space,
            bpc,
            data,
            alpha,
        } => {
            let rgb = match (*color_space, *bpc) {
                ("DeviceRGB", 8) => data.clone(),
                ("DeviceGray", 8) => data.iter().flat_map(|&v| [v, v, v]).collect(),
                // Rows padded to whole bytes, the first pixel in the high bit.
                ("DeviceGray", 1) => {
                    let row = w.div_ceil(8);
                    if data.len() < row * h {
                        bail!("the picture is shorter than its size says");
                    }
                    (0..w * h)
                        .map(|i| data[i / w * row + i % w / 8] >> (7 - i % w % 8) & 1)
                        .flat_map(|bit| [bit * 255; 3])
                        .collect()
                }
                ("DeviceCMYK", 8) => data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|&[c, m, y, k]| {
                        [c, m, y].map(|v| 255u8.saturating_sub(v.saturating_add(k)))
                    })
                    .collect(),
                _ => bail!("LimarePDF can't read this picture's colours"),
            };
            (rgb, alpha.clone())
        }
        _ => bail!("LimarePDF can't read this picture's colours (a CMYK JPEG)"),
    };
    if rgb.len() < w * h * 3 || alpha.as_ref().is_some_and(|a| a.len() < w * h) {
        bail!("the picture is shorter than its size says");
    }
    Ok((rgb, alpha))
}

/// The part of a picture inside `crop`: its left, top, right and bottom as shares of the
/// picture as it is seen (after its orientation), y down. At least one pixel is kept.
fn crop_picture(r: Raster, crop: [f32; 4]) -> Result<Raster> {
    let [u0, v0, u1, v1] = crop.map(|v| {
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        }
    });
    if u1 <= u0 || v1 <= v0 {
        bail!("the part of the picture to use is empty");
    }
    let (w, h) = (r.w as usize, r.h as usize);
    let (a, b) = (
        stored_point(r.orientation, u0, v0),
        stored_point(r.orientation, u1, v1),
    );
    let span = |p: f32, q: f32, n: usize| {
        let lo = ((p.min(q) * n as f32).floor() as usize).min(n - 1);
        let hi = ((p.max(q) * n as f32).ceil() as usize).clamp(lo + 1, n);
        (lo, hi)
    };
    let ((x0, x1), (y0, y1)) = (span(a.0, b.0, w), span(a.1, b.1, h));
    let (rgb, alpha) = rgb_alpha(&r)?;
    let mut data = Vec::with_capacity((x1 - x0) * (y1 - y0) * 3);
    let mut cut = alpha
        .as_ref()
        .map(|_| Vec::with_capacity((x1 - x0) * (y1 - y0)));
    for y in y0..y1 {
        data.extend_from_slice(&rgb[(y * w + x0) * 3..(y * w + x1) * 3]);
        if let (Some(c), Some(a)) = (&mut cut, &alpha) {
            c.extend_from_slice(&a[y * w + x0..y * w + x1]);
        }
    }
    Ok(Raster {
        w: (x1 - x0) as u32,
        h: (y1 - y0) as u32,
        pixels: Pixels::Raw {
            color_space: "DeviceRGB",
            bpc: 8,
            data,
            alpha: cut,
        },
        dpi: r.dpi,
        orientation: r.orientation,
    })
}

/// A PNG, JPEG or TIFF picture as it is seen (after its orientation), as RGBA samples at
/// most `max` pixels across and down: its width, height and samples. For choosing the part
/// of a picture that holds a signature ([`SignArt::Picture`]'s `crop`).
pub fn picture_view(bytes: &[u8], max: u32) -> Result<(u32, u32, Vec<u8>)> {
    let r = first(decode(bytes)?)?;
    let (rgb, alpha) = rgb_alpha(&r)?;
    let rgba: Vec<u8> = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .flat_map(|(i, &[r, g, b])| [r, g, b, alpha.as_ref().map_or(255, |a| a[i])])
        .collect();
    let img = image::RgbaImage::from_raw(r.w, r.h, rgba)
        .ok_or_else(|| anyhow!("the picture is shorter than its size says"))?;
    let mut img = image::DynamicImage::ImageRgba8(img);
    img.apply_orientation(r.orientation);
    if img.width() > max || img.height() > max {
        img = img.thumbnail(max, max);
    }
    Ok((img.width(), img.height(), img.into_rgba8().into_raw()))
}

/// Make the paper around a signature transparent and crop the empty margins: pixels much
/// darker than the paper around them are kept, with a soft edge ([`ink_alpha`]). A picture
/// that is already largely transparent is only cropped.
fn clear_paper(r: Raster) -> Result<Raster> {
    let (w, h) = (r.w as usize, r.h as usize);
    let (rgb, alpha) = rgb_alpha(&r)
        .map_err(|_| anyhow!("can't make this picture's paper transparent; keep its background"))?;
    let lum: Vec<u8> = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&[r, g, b]| ((299 * r as u32 + 587 * g as u32 + 114 * b as u32) / 1000) as u8)
        .collect();
    let see_through = alpha
        .as_ref()
        .map_or(0, |a| a.iter().filter(|&&v| v < 128).count());
    let alpha = if see_through * 5 > w * h {
        alpha.unwrap_or_default()
    } else {
        ink_alpha(&lum, w, h, alpha.as_deref())?
    };
    crop_to_ink(r, w, h, rgb, alpha)
}

/// How much darker than the paper a pixel must be, at least, to be ink.
const MIN_INK: f32 = 24.0;

/// How much of each pixel of a picture of a signature on paper is ink, from 0 (paper) to
/// 255, times its own opacity (`old`). The paper's shade is found around each pixel
/// ([`paper_shade`]), so shadows and uneven light count as paper; ink is what is clearly
/// darker than the paper around it, beyond the paper's own grain; and specks that can't
/// be writing are left out ([`keep_writing`]). Fails when no writing is left.
fn ink_alpha(lum: &[u8], w: usize, h: usize, old: Option<&[u8]>) -> Result<Vec<u8>> {
    let none =
        || anyhow!("LimarePDF couldn't tell a signature apart from the paper in the picture");
    let paper = paper_shade(lum, w, h);
    let dark: Vec<u8> = lum
        .iter()
        .zip(&paper)
        .map(|(&l, &p)| p.saturating_sub(l))
        .collect();
    // The paper's grain: ink is a small share of a signature's picture, so the usual
    // darkness and the usual distance from it are the paper's.
    let mut hist = [0usize; 256];
    for &d in &dark {
        hist[d as usize] += 1;
    }
    let usual = percentile(&hist, 0.5);
    let mut off = [0usize; 256];
    for &d in &dark {
        off[d.abs_diff(usual) as usize] += 1;
    }
    let grain = 1.4826 * percentile(&off, 0.5) as f32;
    let floor = (usual as f32 + 6.0 * grain).max(MIN_INK);
    // The ink's own darkness: most of what is darker than that.
    let mut inky = [0usize; 256];
    for (i, n) in hist.iter().enumerate() {
        if i as f32 > floor {
            inky[i] = *n;
        }
    }
    if inky.iter().all(|&n| n == 0) {
        return Err(none());
    }
    let ink = percentile(&inky, 0.9) as f32;
    // See-through up to a fifth of the way to the ink, solid from three fifths.
    let lo = floor.max(0.2 * ink);
    let hi = (0.6 * ink).max(lo + 1.0);
    let mut alpha: Vec<u8> = dark
        .iter()
        .map(|&d| ((d as f32 - lo) / (hi - lo)).clamp(0.0, 1.0))
        .map(|a| (a * 255.0).round() as u8)
        .collect();
    keep_writing(&mut alpha, w, h).ok_or_else(none)?;
    if let Some(old) = old {
        for (a, &o) in alpha.iter_mut().zip(old) {
            *a = ((*a as u32 * o as u32 + 127) / 255) as u8;
        }
    }
    Ok(alpha)
}

/// The shade at which `share` of the samples counted in `hist` are no darker.
fn percentile(hist: &[usize; 256], share: f32) -> u8 {
    let want = (hist.iter().sum::<usize>() as f32 * share) as usize;
    let mut sum = 0;
    (0..=255u8)
        .find(|&i| {
            sum += hist[i as usize];
            sum > want
        })
        .unwrap_or(255)
}

/// The paper's shade at each pixel of a grey picture: in blocks of about a 24th of the
/// picture, each block's light end (its 90th percentile, so writing over part of it
/// doesn't count), blended smoothly between the blocks' centres. A block much darker than
/// most of its neighbours (one covered by thick writing) takes their shade.
fn paper_shade(lum: &[u8], w: usize, h: usize) -> Vec<u8> {
    let b = (w.max(h) / 24).max(8);
    let (bw, bh) = (w.div_ceil(b), h.div_ceil(b));
    let mut level = vec![0f32; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let mut hist = [0usize; 256];
            for y in by * b..((by + 1) * b).min(h) {
                for &l in &lum[y * w + bx * b..y * w + ((bx + 1) * b).min(w)] {
                    hist[l as usize] += 1;
                }
            }
            level[by * bw + bx] = percentile(&hist, 0.9) as f32;
        }
    }
    let level: Vec<f32> = (0..bw * bh)
        .map(|i| {
            let (bx, by) = (i % bw, i / bw);
            let mut near: Vec<f32> = (by.saturating_sub(1)..(by + 2).min(bh))
                .flat_map(|y| (bx.saturating_sub(1)..(bx + 2).min(bw)).map(move |x| (x, y)))
                .map(|(x, y)| level[y * bw + x])
                .collect();
            near.sort_by(f32::total_cmp);
            let mid = near[near.len() / 2];
            if level[i] < mid - MIN_INK {
                mid
            } else {
                level[i]
            }
        })
        .collect();
    // Where a pixel is between the block centres around it: the lower one and how far on.
    let place = |p: usize, n: usize| {
        let g = ((p as f32 + 0.5) / b as f32 - 0.5).clamp(0.0, (n - 1) as f32);
        let lo = g.floor() as usize;
        (lo, (lo + 1).min(n - 1), g - lo as f32)
    };
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let (y0, y1, ty) = place(y, bh);
        for x in 0..w {
            let (x0, x1, tx) = place(x, bw);
            let at = |x: usize, y: usize| level[y * bw + x];
            let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
            let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
            out.push((top + (bottom - top) * ty).round() as u8);
        }
    }
    out
}

/// Clears the specks in `alpha` (a picture's ink) that can't be writing: pieces of a few
/// pixels, and small pieces away from the strokes (the pieces of at least a 20th of the
/// largest), farther than the strokes are tall. An i's dot stays. `None` when no piece is
/// big enough to be a stroke.
fn keep_writing(alpha: &mut [u8], w: usize, h: usize) -> Option<()> {
    // Each piece's number for its pixels (from 1), and each piece's size and box.
    let mut piece = vec![0u32; w * h];
    let mut pieces: Vec<(usize, [usize; 4])> = Vec::new();
    let mut todo = Vec::new();
    for start in 0..w * h {
        if alpha[start] == 0 || piece[start] != 0 {
            continue;
        }
        let n = pieces.len() as u32 + 1;
        piece[start] = n;
        todo.push(start);
        let (mut size, mut b) = (0, [usize::MAX, usize::MAX, 0, 0]);
        while let Some(i) = todo.pop() {
            let (x, y) = (i % w, i / w);
            size += 1;
            b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let j = ny * w + nx;
                    if alpha[j] != 0 && piece[j] == 0 {
                        piece[j] = n;
                        todo.push(j);
                    }
                }
            }
        }
        pieces.push((size, b));
    }
    let tiny = (w * h / 500_000).max(3);
    let largest = pieces.iter().map(|p| p.0).max().unwrap_or(0);
    if largest < 16 {
        return None;
    }
    let mut strokes = [usize::MAX, usize::MAX, 0, 0];
    for (_, b) in pieces.iter().filter(|p| p.0 * 20 >= largest) {
        strokes = [
            strokes[0].min(b[0]),
            strokes[1].min(b[1]),
            strokes[2].max(b[2]),
            strokes[3].max(b[3]),
        ];
    }
    let reach = (strokes[3] - strokes[1] + 1).max(8);
    let keep: Vec<bool> = pieces
        .iter()
        .map(|&(size, b)| {
            size >= tiny
                && b[0] <= strokes[2] + reach
                && b[2] + reach >= strokes[0]
                && b[1] <= strokes[3] + reach
                && b[3] + reach >= strokes[1]
        })
        .collect();
    for (a, &p) in alpha.iter_mut().zip(&piece) {
        if p != 0 && !keep[p as usize - 1] {
            *a = 0;
        }
    }
    Some(())
}

/// `r` cropped to where `alpha` shows anything, with a small margin, as RGB (`rgb`) with
/// that alpha.
fn crop_to_ink(r: Raster, w: usize, h: usize, rgb: Vec<u8>, alpha: Vec<u8>) -> Result<Raster> {
    let mut b = [usize::MAX, usize::MAX, 0, 0];
    for y in 0..h {
        for x in 0..w {
            if alpha[y * w + x] > 16 {
                b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
        }
    }
    if b[0] > b[2] {
        bail!("the picture has no signature to keep");
    }
    let m = 2;
    let (x0, y0) = (b[0].saturating_sub(m), b[1].saturating_sub(m));
    let (x1, y1) = ((b[2] + m).min(w - 1), (b[3] + m).min(h - 1));
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut data = Vec::with_capacity(cw * ch * 3);
    let mut a = Vec::with_capacity(cw * ch);
    for y in y0..=y1 {
        data.extend_from_slice(&rgb[(y * w + x0) * 3..(y * w + x1 + 1) * 3]);
        a.extend_from_slice(&alpha[y * w + x0..=y * w + x1]);
    }
    Ok(Raster {
        w: cw as u32,
        h: ch as u32,
        pixels: Pixels::Raw {
            color_space: "DeviceRGB",
            bpc: 8,
            data,
            alpha: Some(a),
        },
        dpi: r.dpi,
        orientation: r.orientation,
    })
}

/// A signature's artwork: fitted into its role's box, in `color`, with today's date under
/// it when `date`.
pub(crate) fn signature_art(
    art: &SignArt,
    role: SignRole,
    color: InkColor,
    date: bool,
) -> Result<Art> {
    let (ops, bounds, image) = match *art {
        SignArt::Drawn { strokes, width } => {
            if !(width > 0.0 && width.is_finite()) {
                bail!("the pen width must be above 0");
            }
            let mut b = Bounds::EMPTY;
            let paths: String = strokes
                .iter()
                .take(MAX_STROKES)
                .map(|s| stroke_ops(s, width, &mut b))
                .collect();
            if b.is_empty() {
                bail!("nothing was drawn");
            }
            let ops = format!("{} w 1 J 1 j\n{paths}S\n", n(width));
            (ops, b, None)
        }
        SignArt::Typed { text, style } => {
            let (ops, b) = typed_ops(text, style)?;
            (ops, b, None)
        }
        SignArt::Picture {
            bytes,
            clear_paper: clear,
            crop,
        } => {
            let r = first(decode(bytes)?)?;
            let r = match crop {
                Some(c) => crop_picture(r, c)?,
                None => r,
            };
            let r = if clear { clear_paper(r)? } else { r };
            let (pw, ph) = (r.w as f32, r.h as f32);
            let (pw, ph) = if swaps_axes(r.orientation) {
                (ph, pw)
            } else {
                (pw, ph)
            };
            let m = orient_matrix(r.orientation, pw, ph).map(n).join(" ");
            let ops = format!("q {m} cm /{IMAGE} Do Q\n");
            (ops, Bounds([0.0, 0.0, pw, ph]), Some(r))
        }
    };
    let [x0, y0, x1, y1] = bounds.0;
    let (fw, fh) = role.fit();
    let k = (fw / (x1 - x0)).min(fh / (y1 - y0));
    let (sw, sh) = ((x1 - x0) * k, (y1 - y0) * k);
    let line = date.then(|| {
        let [y, m, d] = local_today();
        format!("{y:04}-{m:02}-{d:02}")
    });
    let below = line
        .as_ref()
        .map_or(0.0, |_| DATE_SIZE * (CAP + DESCENT) + DATE_GAP);
    let w = sw.max(line.as_deref().map_or(0.0, |l| text_width(l, DATE_SIZE)));
    let [r, g, b] = color.rgb().map(n);
    let mut s = format!(
        "q {r} {g} {b} RG {r} {g} {b} rg\nq {k} 0 0 {k} {tx} {ty} cm\n{ops}Q\n",
        k = n(k),
        tx = n(-x0 * k),
        ty = n(below - y0 * k),
    );
    if let Some(l) = &line {
        s += &format!(
            "BT /{FONT} {DATE_SIZE} Tf 0 {} Td ({}) Tj ET\n",
            n(DATE_SIZE * DESCENT),
            escape(l)
        );
    }
    s += "Q\n";
    Ok(Art {
        ops: s,
        w,
        h: sh + below,
        font: line.is_some(),
        image,
    })
}

/// A mark's artwork, in a [`MARK`]-point box.
pub(crate) fn mark_art(mark: Mark, color: InkColor) -> Art {
    let [r, g, b] = color.rgb().map(n);
    let shape = match mark {
        Mark::Tick => "1.8 w 1 J 1 j 2 6.4 m 4.8 3 l 10 9.6 l S".to_string(),
        Mark::Cross => "1.8 w 1 J 2.5 2.5 m 9.5 9.5 l 2.5 9.5 m 9.5 2.5 l S".to_string(),
        Mark::Dot => {
            // A circle of radius 3 in the middle, as four quarter curves.
            let (c, r) = (MARK / 2.0, 3.0);
            let (hi, lo) = (n(c + r), n(c - r));
            let (kh, kl) = (n(c + r * KAPPA), n(c - r * KAPPA));
            let c = n(c);
            format!(
                "{hi} {c} m {hi} {kh} {kh} {hi} {c} {hi} c {kl} {hi} {lo} {kh} {lo} {c} c \
                 {lo} {kl} {kl} {lo} {c} {lo} c {kh} {lo} {hi} {kl} {hi} {c} c f"
            )
        }
    };
    Art {
        ops: format!("q {r} {g} {b} RG {r} {g} {b} rg {shape} Q\n"),
        w: MARK,
        h: MARK,
        font: false,
        image: None,
    }
}

impl Pdf {
    /// Artwork as a form XObject.
    pub(crate) fn add_art(&mut self, art: Art) -> ObjectId {
        let mut res = Dictionary::new();
        if art.font {
            let font = self.doc.add_object(dictionary! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
                "Encoding" => "WinAnsiEncoding",
            });
            res.set("Font", dictionary! { FONT => font });
        }
        if let Some(r) = art.image {
            let img = self.add_raster(r);
            res.set("XObject", dictionary! { IMAGE => img });
        }
        let form = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => nums(&[0.0, 0.0, art.w, art.h]),
            "Resources" => res,
        };
        self.doc.add_object(Stream::new(form, art.ops.into_bytes()))
    }

    /// Write `text` (a date, a name or other words) on page `page_index` as a fill-and-sign
    /// item: a text box without border or background, in Helvetica at [`SIGN_TEXT`]
    /// points, centred on (`x`, `y`) in displayed-page points. Returns its id.
    pub fn add_sign_text(
        &mut self,
        page_index: usize,
        (x, y): (f32, f32),
        text: &str,
        color: InkColor,
        author: &str,
    ) -> Result<(u32, u16)> {
        let text = text.trim();
        if text.is_empty() {
            bail!("there is no text to put down");
        }
        let w = text
            .lines()
            .map(|l| text_width(l, SIGN_TEXT))
            .fold(0.0, f32::max)
            + 6.0;
        let h = SIGN_TEXT * 1.25 * text.lines().count() as f32 + 6.0;
        let style = TextStyle {
            size: SIGN_TEXT,
            color: color.rgb(),
            fill: None,
            border: 0.0,
        };
        let rect = [x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0];
        let id = self.add_text_box(page_index, rect, text, &style, author)?;
        self.doc.get_dictionary_mut(id)?.set(ITEM_KEY, true);
        Ok(id)
    }

    /// How many fill-and-sign items (signatures, initials, marks and their text) are still
    /// annotations, so they can be moved; [`Pdf::finish_signing`] draws them in.
    pub fn sign_items(&self) -> usize {
        self.page_ids()
            .into_iter()
            .flat_map(|p| self.annot_refs(p))
            .filter(|&id| self.doc.get_dictionary(id).is_ok_and(is_sign_item))
            .count()
    }

    /// Draw every fill-and-sign item into its page, so it can no longer be moved or
    /// removed as a comment. Other annotations stay. Returns how many were drawn in.
    pub fn finish_signing(&mut self) -> Result<usize> {
        fn which(_: &Pdf, d: &Dictionary) -> Flat {
            if is_sign_item(d) {
                Flat::Draw
            } else {
                Flat::Keep
            }
        }
        let mut drawn = 0;
        for page in self.page_ids() {
            drawn += self.flatten_page(page, which, None, "LimareSign", &mut HashSet::new())?;
        }
        Ok(drawn)
    }

    /// Lock every fill-and-sign item in place (annotation flag Locked), for a digital
    /// signature on a document someone already signed: drawing them in would change page
    /// content, which earlier signatures don't allow. Returns how many were locked.
    pub(crate) fn lock_sign_items(&mut self) -> Result<usize> {
        let items: Vec<ObjectId> = self
            .page_ids()
            .into_iter()
            .flat_map(|p| self.annot_refs(p))
            .filter(|&id| self.doc.get_dictionary(id).is_ok_and(is_sign_item))
            .collect();
        for &id in &items {
            let d = self.doc.get_dictionary_mut(id)?;
            let flags = d.get(b"F").and_then(lopdf::Object::as_i64).unwrap_or(0);
            d.set("F", flags | 128);
        }
        Ok(items.len())
    }
}

/// The size of fill-and-sign text, in points.
pub const SIGN_TEXT: f32 = 11.0;
/// The entry LimarePDF puts on fill-and-sign text boxes, to know them from other text boxes.
const ITEM_KEY: &str = "LimareFillSign";

/// Whether an annotation is a fill-and-sign item: a signature, initials or mark stamp
/// (by its `/Name`), or text put down with them.
fn is_sign_item(d: &Dictionary) -> bool {
    let named = d
        .get(b"Name")
        .and_then(|o| o.as_name())
        .is_ok_and(|n| sign_label(&String::from_utf8_lossy(n)).is_some());
    let stamp = d
        .get(b"Subtype")
        .and_then(|o| o.as_name())
        .is_ok_and(|s| s == b"Stamp");
    (stamp && named) || d.has(ITEM_KEY.as_bytes())
}

/// How a date is written.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DateStyle {
    /// 2026-10-16
    #[default]
    Iso,
    /// 16 October 2026
    DayMonthYear,
    /// October 16, 2026
    MonthDayYear,
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

impl DateStyle {
    pub const ALL: [DateStyle; 3] = [
        DateStyle::Iso,
        DateStyle::DayMonthYear,
        DateStyle::MonthDayYear,
    ];

    /// Its name in the settings file.
    pub fn name(self) -> &'static str {
        match self {
            DateStyle::Iso => "iso",
            DateStyle::DayMonthYear => "day-month-year",
            DateStyle::MonthDayYear => "month-day-year",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.name() == s.trim())
    }

    /// A date written this way.
    pub fn format(self, y: u32, m: u32, d: u32) -> String {
        let month = MONTHS[(m.clamp(1, 12) - 1) as usize];
        match self {
            DateStyle::Iso => format!("{y:04}-{m:02}-{d:02}"),
            DateStyle::DayMonthYear => format!("{d} {month} {y}"),
            DateStyle::MonthDayYear => format!("{month} {d}, {y}"),
        }
    }

    /// Today's date (in this computer's time zone, as stamps write it) written this way.
    pub fn today(self) -> String {
        let [y, m, d] = local_today();
        self.format(y as u32, m as u32, d as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_names_are_tidied_and_checked_against_the_font() {
        for style in SignStyle::ALL {
            assert_eq!(typed_signature("  Jane   Doe ", style).unwrap(), "Jane Doe");
            assert!(typed_signature("José Müller", style).is_ok());
            assert!(typed_signature("   ", style).is_err());
            let e = typed_signature("李", style).unwrap_err().to_string();
            assert!(e.contains('李') && !e.contains("can show it"), "{e}");
        }
        // The note names the styles that have the character.
        let e = typed_signature("Юлия", SignStyle::Zeyada)
            .unwrap_err()
            .to_string();
        assert!(e.contains("; Caveat can show it, or draw"), "{e}");
        let e = typed_signature("Dvořák", SignStyle::HomemadeApple)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("Caveat, Cedarville Cursive and Zeyada can show it"),
            "{e}"
        );
    }

    #[test]
    fn typed_signatures_fit_the_signature_box() {
        let art = SignArt::Typed {
            text: "Jane Doe",
            style: SignStyle::Caveat,
        };
        let a = signature_art(&art, SignRole::Signature, InkColor::Black, false).unwrap();
        assert!(a.w <= 150.01 && a.h <= 50.01, "{} x {}", a.w, a.h);
        assert!(a.w > 149.0 || a.h > 49.0);
        assert!(a.ops.contains(" c\n") && a.ops.contains("f\n"));
        let i = signature_art(&art, SignRole::Initials, InkColor::Blue, true).unwrap();
        assert!(i.w < a.w && i.font);
    }

    #[test]
    fn drawn_strokes_become_curves_and_dots() {
        let strokes = vec![
            vec![(0.0, 0.0), (10.0, 5.0), (20.0, 0.0), (30.0, 8.0)],
            vec![(40.0, 4.0)],
        ];
        let art = SignArt::Drawn {
            strokes: &strokes,
            width: 2.0,
        };
        let a = signature_art(&art, SignRole::Signature, InkColor::Black, false).unwrap();
        assert!(a.ops.contains(" c\n"));
        // The dot: a line to where it starts.
        assert!(a.ops.contains("40 -4 m\n40 -4 l\n"), "{}", a.ops);
        let none: Vec<Vec<(f32, f32)>> = vec![vec![]];
        let art = SignArt::Drawn {
            strokes: &none,
            width: 2.0,
        };
        assert!(signature_art(&art, SignRole::Signature, InkColor::Black, false).is_err());
    }

    #[test]
    fn paper_turns_transparent_and_margins_go() {
        // White paper with a dark bar in the middle.
        let (w, h) = (40u32, 20u32);
        let mut data = vec![235u8; (w * h) as usize];
        for y in 8..12 {
            for x in 10..30 {
                data[(y * w + x) as usize] = 20;
            }
        }
        let r = Raster {
            w,
            h,
            pixels: Pixels::Raw {
                color_space: "DeviceGray",
                bpc: 8,
                data,
                alpha: None,
            },
            dpi: None,
            orientation: image::metadata::Orientation::NoTransforms,
        };
        let c = clear_paper(r).unwrap();
        assert_eq!((c.w, c.h), (24, 8));
        let Pixels::Raw { alpha: Some(a), .. } = &c.pixels else {
            panic!("no alpha");
        };
        assert_eq!(a[0], 0);
        assert_eq!(a[(3 * 24 + 12) as usize], 255);
    }

    /// A grey picture `w` by `h` of `paper` (by x and y), and a made-up signature drawn on it
    /// in ink of shade `ink` from (x, y): a wavy word about 300 by 50 pixels, a stroke under
    /// it and a dot over it (about 0.1% of a letter-size sheet at 150 dpi). Returns the
    /// samples and the box the ink takes up (left, top, right, bottom, inclusive).
    fn signed_sheet(
        (w, h): (usize, usize),
        mut paper: impl FnMut(usize, usize) -> u8,
        (x, y): (usize, usize),
        ink: u8,
    ) -> (Vec<u8>, [usize; 4]) {
        let mut data: Vec<u8> = (0..w * h).map(|i| paper(i % w, i / w)).collect();
        let mut b = [usize::MAX, usize::MAX, 0, 0];
        let mut dot = |cx: f32, cy: f32, r: f32| {
            let (x0, y0) = ((cx - r).floor() as usize, (cy - r).floor() as usize);
            for py in y0..=(cy + r).ceil() as usize {
                for px in x0..=(cx + r).ceil() as usize {
                    if (px as f32 - cx).hypot(py as f32 - cy) <= r {
                        data[py * w + px] = ink;
                        b = [b[0].min(px), b[1].min(py), b[2].max(px), b[3].max(py)];
                    }
                }
            }
        };
        let (x, y) = (x as f32, y as f32);
        for t in 0..600 {
            let s = t as f32 * 0.5;
            let wave = 18.0 * (s / 14.0).sin() + 6.0 * (s / 5.0).sin();
            dot(x + s, y + 25.0 + wave, 1.5);
        }
        for t in 0..520 {
            dot(x + 20.0 + t as f32 * 0.5, y + 60.0, 1.2);
        }
        dot(x + 150.0, y + 2.0, 2.0);
        (data, b)
    }

    fn gray(w: usize, h: usize, data: Vec<u8>, alpha: Option<Vec<u8>>) -> Raster {
        Raster {
            w: w as u32,
            h: h as u32,
            pixels: Pixels::Raw {
                color_space: "DeviceGray",
                bpc: 8,
                data,
                alpha,
            },
            dpi: None,
            orientation: image::metadata::Orientation::NoTransforms,
        }
    }

    /// Noise from -`k` to `k`, the same every run.
    fn noise(k: i32) -> impl FnMut() -> i32 {
        let mut s: u32 = 0x9e37_79b9;
        move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s % (2 * k as u32 + 1)) as i32 - k
        }
    }

    /// The size [`clear_paper`] crops a picture to whose ink takes up `b`: the ink and a
    /// margin of 2 pixels.
    fn cropped_to(b: [usize; 4]) -> (u32, u32) {
        ((b[2] - b[0] + 5) as u32, (b[3] - b[1] + 5) as u32)
    }

    #[test]
    fn a_small_signature_on_a_full_sheet_is_kept() {
        // A letter-size sheet at 150 dpi, signed near the bottom (A24's case).
        let (w, h) = (1275, 1650);
        let (data, b) = signed_sheet((w, h), |_, _| 240, (800, 1450), 30);
        let ink = data.iter().filter(|&&v| v < 128).count();
        assert!(ink * 1000 < w * h * 2, "{ink} ink pixels: not sparse");
        let full = clear_paper(gray(w, h, data.clone(), None)).unwrap();
        assert_eq!((full.w, full.h), cropped_to(b));
        // The same signature cut out first comes out the same.
        let (cw, ch) = (b[2] - b[0] + 41, b[3] - b[1] + 41);
        let cut: Vec<u8> = (0..cw * ch)
            .map(|i| data[(b[1] - 20 + i / cw) * w + b[0] - 20 + i % cw])
            .collect();
        let close = clear_paper(gray(cw, ch, cut, None)).unwrap();
        assert_eq!((close.w, close.h), (full.w, full.h));
        let (Pixels::Raw { alpha: Some(a), .. }, Pixels::Raw { alpha: Some(c), .. }) =
            (&full.pixels, &close.pixels)
        else {
            panic!("no alpha");
        };
        assert_eq!(a, c);
    }

    #[test]
    fn blank_or_noisy_paper_has_no_signature() {
        let (w, h) = (600, 400);
        let refused = |data: Vec<u8>| match clear_paper(gray(w, h, data, None)) {
            Ok(r) => panic!("kept a {} by {} signature", r.w, r.h),
            Err(e) => assert!(e.to_string().contains("signature"), "{e}"),
        };
        refused(vec![236; w * h]);
        let mut n = noise(14);
        refused((0..w * h).map(|_| (220 + n()) as u8).collect());
        // Lightly shaded from one side to the other, as a photo often is.
        refused((0..w * h).map(|i| (250 - (i % w) * 60 / w) as u8).collect());
    }

    #[test]
    fn grain_shading_and_specks_are_not_the_signature() {
        let (w, h) = (900, 600);
        // Grainy paper.
        let mut n = noise(14);
        let (data, b) = signed_sheet((w, h), |_, _| (215 + n()) as u8, (300, 300), 40);
        let r = clear_paper(gray(w, h, data, None)).unwrap();
        assert_eq!((r.w, r.h), cropped_to(b));
        // Paper shaded from 250 on the left to 170 on the right, signed on the light side.
        let shade = |x: usize, _| (250 - x * 80 / w) as u8;
        let (data, b) = signed_sheet((w, h), shade, (60, 200), 40);
        let r = clear_paper(gray(w, h, data, None)).unwrap();
        assert_eq!((r.w, r.h), cropped_to(b));
        // Cream paper with dust: a speck of 9 pixels in a far corner, and two of a pixel.
        let (mut data, b) = signed_sheet((w, h), |_, _| 205, (300, 300), 60);
        for i in 0..9 {
            data[(40 + i / 3) * w + 40 + i % 3] = 60;
        }
        for (x, y) in [(310, 290), (880, 590)] {
            data[y * w + x] = 60;
        }
        let r = clear_paper(gray(w, h, data, None)).unwrap();
        assert_eq!((r.w, r.h), cropped_to(b));
    }

    #[test]
    fn a_picture_already_see_through_is_only_cropped() {
        let (w, h) = (500, 300);
        let (data, b) = signed_sheet((w, h), |_, _| 255, (100, 100), 0);
        let alpha: Vec<u8> = data.iter().map(|&v| 255 - v).collect();
        // Its colour gives nothing away: the same black everywhere.
        let r = clear_paper(gray(w, h, vec![0; w * h], Some(alpha))).unwrap();
        assert_eq!((r.w, r.h), cropped_to(b));
    }

    #[test]
    fn a_part_of_the_picture_as_seen_is_used() {
        use image::metadata::Orientation::*;
        // Stored 3 across and 2 down, each pixel its own shade.
        let r = |orientation| Raster {
            orientation,
            ..gray(3, 2, vec![10, 20, 30, 40, 50, 60], None)
        };
        let samples = |r: &Raster| match &r.pixels {
            Pixels::Raw { data, .. } => data.iter().step_by(3).copied().collect::<Vec<_>>(),
            _ => panic!("not raw"),
        };
        let c = crop_picture(r(NoTransforms), [0.5, 0.5, 1.0, 1.0]).unwrap();
        assert_eq!((c.w, c.h, samples(&c)), (2, 1, vec![50, 60]));
        // Turned a quarter right, it is seen 2 across and 3 down: the top row seen is the
        // stored left column, its bottom first.
        let c = crop_picture(r(Rotate90), [0.0, 0.0, 1.0, 0.3]).unwrap();
        assert_eq!((c.w, c.h, samples(&c)), (1, 2, vec![10, 40]));
        assert_eq!(c.orientation, Rotate90);
        let c = crop_picture(r(Rotate270), [0.0, 0.0, 1.0, 0.3]).unwrap();
        assert_eq!((c.w, c.h, samples(&c)), (1, 2, vec![30, 60]));
        assert!(crop_picture(r(NoTransforms), [0.5, 0.2, 0.5, 1.0]).is_err());
        assert!(crop_picture(r(NoTransforms), [f32::NAN, 0.0, 1.0, 1.0]).is_ok());
    }

    #[test]
    fn one_bit_and_cmyk_pictures_are_read() {
        let raw = |color_space, bpc, w, h, data| Raster {
            w,
            h,
            pixels: Pixels::Raw {
                color_space,
                bpc,
                data,
                alpha: None,
            },
            dpi: None,
            orientation: image::metadata::Orientation::NoTransforms,
        };
        // 10 across: two bytes a row, white where a bit is set.
        let one = raw("DeviceGray", 1, 10, 2, vec![0b1000_0000, 0b0100_0000, 0, 0]);
        let (rgb, _) = rgb_alpha(&one).unwrap();
        let firsts: Vec<u8> = rgb.iter().step_by(3).copied().collect();
        assert_eq!(&firsts[..10], &[255, 0, 0, 0, 0, 0, 0, 0, 0, 255]);
        assert!(rgb_alpha(&raw("DeviceGray", 1, 10, 3, vec![0; 4])).is_err());
        let cmyk = raw("DeviceCMYK", 8, 2, 1, vec![0, 0, 0, 0, 0, 0, 0, 255]);
        assert_eq!(rgb_alpha(&cmyk).unwrap().0, vec![255, 255, 255, 0, 0, 0]);
    }

    #[test]
    fn a_picture_is_shown_upright_and_small() {
        let mut png = Vec::new();
        image::RgbImage::from_pixel(300, 200, image::Rgb([200, 10, 10]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (w, h, rgba) = picture_view(&png, 600).unwrap();
        assert_eq!((w, h), (300, 200));
        assert_eq!(&rgba[..4], &[200, 10, 10, 255]);
        let (w, h, rgba) = picture_view(&png, 60).unwrap();
        assert_eq!((w, h, rgba.len()), (60, 40, 60 * 40 * 4));
    }

    #[test]
    fn labels_come_from_names() {
        assert_eq!(sign_label("LimareSignature"), Some("Signature"));
        assert_eq!(sign_label("LimareCross"), Some("Cross"));
        assert_eq!(sign_label("Approved"), None);
        assert_eq!(Mark::from_name("TICK"), Some(Mark::Tick));
        assert_eq!(InkColor::from_name("Blue"), Some(InkColor::Blue));
        assert_eq!(SignStyle::from_number(3), Some(SignStyle::HomemadeApple));
        assert_eq!(SignStyle::from_number(5), None);
    }
}
