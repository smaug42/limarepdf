//! Fill-and-sign artwork: signatures and initials (drawn with the pen, typed in a
//! handwriting font, or from a picture) and tick, cross and dot marks. [`crate::Stamp`]
//! puts them on pages as stamp annotations. Typed signatures are the font's glyph
//! outlines drawn as shapes, so no font goes into the PDF and they look the same in every
//! reader. None of this proves who signed: that is what digital signatures are for.

use crate::annot_write::simplify;
use crate::form_ap::nums;
use crate::form_flatten::Flat;
use crate::imgs::{decode, first, orient_matrix, swaps_axes, Pixels, Raster};
use crate::overlay::{escape, text_width, utc_now};
use crate::stamps::KAPPA;
use crate::{Pdf, Result, TextStyle};
use anyhow::{anyhow, bail};
use lopdf::{dictionary, Dictionary, ObjectId, Stream};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{FontRef, LocationRef, MetadataProvider, Size};
use std::collections::HashSet;

/// The handwriting fonts, under the SIL Open Font Licence (see docs/licence.md).
static CASUAL: &[u8] = include_bytes!("../assets/fonts/DancingScript.ttf");
static FORMAL: &[u8] = include_bytes!("../assets/fonts/GreatVibes-Regular.ttf");

/// The size typed signatures are drawn at before they are fitted, in points.
const TYPE_SIZE: f32 = 40.0;
/// Resource names in the artwork.
const FONT: &str = "MpeF";
const IMAGE: &str = "MpeIm";
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

/// A handwriting style for typed signatures.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignStyle {
    /// Dancing Script: casual, joined-up.
    Casual,
    /// Great Vibes: a formal script.
    Formal,
}

impl SignStyle {
    pub const ALL: [SignStyle; 2] = [SignStyle::Casual, SignStyle::Formal];

    pub fn label(self) -> &'static str {
        match self {
            SignStyle::Casual => "Casual",
            SignStyle::Formal => "Formal",
        }
    }

    /// Its number, 1 or 2, as the command line and the settings file give it.
    pub fn number(self) -> u8 {
        match self {
            SignStyle::Casual => 1,
            SignStyle::Formal => 2,
        }
    }

    pub fn from_number(n: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.number() == n)
    }

    fn font(self) -> &'static [u8] {
        match self {
            SignStyle::Casual => CASUAL,
            SignStyle::Formal => FORMAL,
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
            SignRole::Signature => "MpeSignature",
            SignRole::Initials => "MpeInitials",
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
            Mark::Tick => "MpeTick",
            Mark::Cross => "MpeCross",
            Mark::Dot => "MpeDot",
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
    /// A PNG, JPEG or TIFF picture. `clear_paper` makes the paper around the ink
    /// transparent and crops the empty margins.
    Picture { bytes: &'a [u8], clear_paper: bool },
}

/// What MPE calls a fill-and-sign stamp, from its `/Name`; `None` for other stamps.
pub fn sign_label(name: &str) -> Option<&'static str> {
    Some(match name {
        "MpeSignature" => "Signature",
        "MpeInitials" => "Initials",
        _ => Mark::ALL.into_iter().find(|m| m.name() == name)?.label(),
    })
}

/// A name to sign with as it will be drawn: trimmed, with runs of spaces made single.
/// Fails when it is empty or has characters the handwriting style can't show.
pub fn typed_signature(text: &str, style: SignStyle) -> Result<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        bail!("type a name to sign with");
    }
    let font = FontRef::new(style.font()).map_err(|e| anyhow!("handwriting font: {e}"))?;
    let map = font.charmap();
    let mut bad = String::new();
    for c in text.chars() {
        if c != ' ' && map.map(c).is_none() && !bad.contains(c) {
            bad.push(c);
        }
    }
    if !bad.is_empty() {
        bail!(
            "the {} handwriting can't show {bad:?}; draw your signature or use a picture instead",
            style.label().to_lowercase()
        );
    }
    Ok(text)
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

/// A picture's samples as 8-bit RGB with alpha, if it has a form that can be read so.
fn rgb_alpha(r: &Raster) -> Result<(Vec<u8>, Option<Vec<u8>>)> {
    let cant = || anyhow!("can't make this picture's paper transparent; keep its background");
    match &r.pixels {
        Pixels::Jpeg {
            data, components, ..
        } if *components != 4 => {
            let img = image::load_from_memory_with_format(data, image::ImageFormat::Jpeg)
                .map_err(|e| anyhow!("cannot decode the picture: {e}"))?;
            Ok((img.to_rgb8().into_raw(), None))
        }
        Pixels::Raw {
            color_space,
            bpc: 8,
            data,
            alpha,
        } => {
            let rgb = match *color_space {
                "DeviceRGB" => data.clone(),
                "DeviceGray" => data.iter().flat_map(|&v| [v, v, v]).collect(),
                _ => return Err(cant()),
            };
            Ok((rgb, alpha.clone()))
        }
        _ => Err(cant()),
    }
}

/// Make the paper around a signature transparent and crop the empty margins: pixels much
/// darker than the paper are kept, with a soft edge. A picture that is already largely
/// transparent is only cropped.
fn clear_paper(r: Raster) -> Result<Raster> {
    let (w, h) = (r.w as usize, r.h as usize);
    let (rgb, alpha) = rgb_alpha(&r)?;
    if rgb.len() < w * h * 3 {
        bail!("the picture is shorter than its size says");
    }
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
        // The paper is the usual shade (the median), the ink the darkest 1%.
        let mut hist = [0usize; 256];
        for &l in &lum {
            hist[l as usize] += 1;
        }
        let level = |share: f32| {
            let want = (lum.len() as f32 * share) as usize;
            let mut sum = 0;
            (0..256).find(|&i| {
                sum += hist[i];
                sum > want
            })
        };
        let (ink, paper) = (level(0.01).unwrap_or(0), level(0.5).unwrap_or(255));
        let range = paper.saturating_sub(ink) as f32;
        if range < 24.0 {
            bail!("the picture has no signature to keep: it is all much the same shade");
        }
        let (dark, light) = (ink as f32 + 0.4 * range, ink as f32 + 0.8 * range);
        lum.iter()
            .enumerate()
            .map(|(i, &l)| {
                let a = ((light - l as f32) / (light - dark)).clamp(0.0, 1.0);
                let old = alpha.as_ref().map_or(255.0, |v| v[i] as f32);
                (a * old).round() as u8
            })
            .collect()
    };
    // Crop to what is left, with a small margin.
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
        } => {
            let r = first(decode(bytes)?)?;
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
        let [y, m, d, ..] = utc_now();
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
            drawn += self.flatten_page(page, which, None, "MpeSign", &mut HashSet::new())?;
        }
        Ok(drawn)
    }
}

/// The size of fill-and-sign text, in points.
pub const SIGN_TEXT: f32 = 11.0;
/// The entry MPE puts on fill-and-sign text boxes, to know them from other text boxes.
const ITEM_KEY: &str = "MpeFillSign";

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

    /// Today's date (in UTC, as stamps write it) written this way.
    pub fn today(self) -> String {
        let [y, m, d, ..] = utc_now();
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
            assert!(e.contains('李'), "{e}");
        }
    }

    #[test]
    fn typed_signatures_fit_the_signature_box() {
        let art = SignArt::Typed {
            text: "Jane Doe",
            style: SignStyle::Casual,
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

    #[test]
    fn labels_come_from_names() {
        assert_eq!(sign_label("MpeSignature"), Some("Signature"));
        assert_eq!(sign_label("MpeCross"), Some("Cross"));
        assert_eq!(sign_label("Approved"), None);
        assert_eq!(Mark::from_name("TICK"), Some(Mark::Tick));
        assert_eq!(InkColor::from_name("Blue"), Some(InkColor::Blue));
        assert_eq!(SignStyle::from_number(2), Some(SignStyle::Formal));
    }
}
