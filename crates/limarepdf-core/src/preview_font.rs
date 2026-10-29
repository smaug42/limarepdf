//! Small OpenType fonts that draw typing in a PDF font's own glyphs while an edited line keeps
//! that font (ENH-31). Core records each glyph's outline from the embedded program with a skrifa
//! pen ([`Outline`]) and writes the glyphs into a font of their own ([`opentype`]), which the
//! app loads like an installed font. The advances are the PDF's widths, so the caret matches
//! what is written.
//!
//! The outlines are CFF, a TrueType program's quadratic segments kept as the cubics that draw
//! them exactly, with coordinates to 1/65536 of a unit. skrifa draws an unhinted CFF outline in
//! whole units, as FreeType does, so a coordinate can come back up to a unit (1/1000 em) off.

use skrifa::outline::OutlinePen;
use std::collections::HashSet;

/// One segment of a glyph's outline, in units of a 1000-unit em, y up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    /// A cubic Bézier: its two control points, then where it ends.
    Curve(f32, f32, f32, f32, f32, f32),
    Close,
}

/// A glyph's outline as a pen draws it; a quadratic segment is recorded as the cubic that
/// draws it exactly (TrueType outlines).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Outline(pub Vec<Seg>);

impl Outline {
    /// Where the pen is: where the last segment ends, or its contour's start after a close.
    fn pen(&self) -> (f32, f32) {
        let mut closed = false;
        for seg in self.0.iter().rev() {
            match *seg {
                Seg::Close => closed = true,
                Seg::Move(x, y) => return (x, y),
                Seg::Line(x, y) | Seg::Curve(_, _, _, _, x, y) if !closed => return (x, y),
                Seg::Line(..) | Seg::Curve(..) => {}
            }
        }
        (0.0, 0.0)
    }
}

impl OutlinePen for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(Seg::Move(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push(Seg::Line(x, y));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (x0, y0) = self.pen();
        // Two thirds of the way from an end to the quadratic's control point.
        let toward = |from: f32, to: f32| from + 2.0 / 3.0 * (to - from);
        self.0.push(Seg::Curve(
            toward(x0, cx0),
            toward(y0, cy0),
            toward(x, cx0),
            toward(y, cy0),
            x,
            y,
        ));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.push(Seg::Curve(cx0, cy0, cx1, cy1, x, y));
    }

    fn close(&mut self) {
        self.0.push(Seg::Close);
    }
}

/// A character of a preview font: its outline and advance width (1/1000 em).
#[derive(Clone, Debug)]
pub(crate) struct Glyph {
    pub c: char,
    pub advance: f32,
    pub outline: Outline,
}

/// A preview font's vertical metrics in 1/1000 em: ascent above the baseline (positive)
/// and descent below it (zero or negative).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Metrics {
    pub ascent: f32,
    pub descent: f32,
}

/// The font's units per em, those of a PDF's glyph widths.
const UNITS_PER_EM: u16 = 1000;

/// How far from the origin a coordinate may be, in units: the distance between two still fits
/// a charstring's 16-bit operands, and every coordinate the 16-bit fields of the tables.
const LIMIT: f32 = 16_000.0;

/// The most characters a font holds: a CFF font names its glyphs with 16-bit string ids, the
/// first 391 of which are the standard strings.
const MAX_CHARS: usize = 65_536 - 391;

/// The most runs of characters a format 4 cmap holds before its last segment, its length
/// being 16 bits.
const MAX_SEGMENTS: usize = (65_535 - 16) / 8 - 1;

/// An OpenType font with CFF outlines holding `glyphs`, each character mapped to a glyph of
/// its own (glyph 0 an empty `.notdef`), named `name`.
///
/// The glyphs follow `.notdef` in the order of their characters. A character given twice
/// keeps its first glyph, and beyond 65,145 characters (what CFF glyph names allow) the
/// first are kept. Coordinates are kept within 16 em of the origin, and segments with ones
/// that aren't numbers left out; advances are rounded to whole units up to 32,767, and are 0
/// when they aren't numbers.
pub(crate) fn opentype(name: &str, glyphs: &[Glyph], m: Metrics) -> Vec<u8> {
    let mut seen = HashSet::new();
    let mut kept: Vec<&Glyph> = glyphs
        .iter()
        .filter(|g| seen.insert(g.c))
        .take(MAX_CHARS)
        .collect();
    kept.sort_by_key(|g| g.c);
    let chars: Vec<char> = kept.iter().map(|g| g.c).collect();
    let drawn: Vec<Drawn> = std::iter::once(Drawn::notdef())
        .chain(kept.iter().map(|g| Drawn::new(g)))
        .collect();
    let bbox = drawn
        .iter()
        .filter_map(|d| d.bounds)
        .reduce(Bounds::union)
        .unwrap_or_default();
    let ps = postscript_name(name);
    let family: String = name.chars().take(255).collect();
    let family = if family.trim().is_empty() {
        ps.clone()
    } else {
        family
    };
    let ascent = units(m.ascent, 800.0).max(0);
    let descent = units(m.descent, -200.0).min(0);
    sfnt(vec![
        (*b"CFF ", cff(&ps, &chars, &drawn, bbox)),
        (*b"OS/2", os2(&chars, &drawn, ascent, descent, bbox)),
        (*b"cmap", cmap(&chars)),
        (*b"head", head(bbox)),
        (*b"hhea", hhea(&drawn, ascent, descent)),
        (*b"hmtx", hmtx(&drawn)),
        (*b"maxp", maxp(drawn.len())),
        (*b"name", names(&family, &ps)),
        (*b"post", post()),
    ])
}

/// A glyph as the font holds it.
struct Drawn {
    advance: u16,
    charstring: Vec<u8>,
    /// Its outline's bounds, none when it draws nothing.
    bounds: Option<Bounds>,
}

impl Drawn {
    fn new(g: &Glyph) -> Self {
        // Whole units, up to what a charstring's width can be.
        let advance = if g.advance.is_finite() {
            g.advance.round().clamp(0.0, 32_767.0) as u16
        } else {
            0
        };
        let (charstring, bounds) = Charstring::write(&g.outline, advance);
        Drawn {
            advance,
            charstring,
            bounds,
        }
    }

    fn notdef() -> Self {
        Drawn {
            advance: 0,
            charstring: vec![ENDCHAR],
            bounds: None,
        }
    }
}

/// An outline's bounds in whole units, rounded outward.
#[derive(Clone, Copy, Default)]
struct Bounds {
    x_min: i16,
    y_min: i16,
    x_max: i16,
    y_max: i16,
}

impl Bounds {
    fn union(self, o: Self) -> Self {
        Bounds {
            x_min: self.x_min.min(o.x_min),
            y_min: self.y_min.min(o.y_min),
            x_max: self.x_max.max(o.x_max),
            y_max: self.y_max.max(o.y_max),
        }
    }
}

/// A point in 16.16 units, each coordinate kept within [`LIMIT`]; none when one isn't a
/// number.
fn point(x: f32, y: f32) -> Option<[i32; 2]> {
    let fixed = |v: f32| {
        v.is_finite()
            .then(|| (f64::from(v.clamp(-LIMIT, LIMIT)) * 65_536.0).round() as i32)
    };
    Some([fixed(x)?, fixed(y)?])
}

/// A vertical metric in whole units; `otherwise` when it isn't a number.
fn units(v: f32, otherwise: f32) -> i16 {
    let v = if v.is_finite() { v } else { otherwise };
    v.clamp(-LIMIT, LIMIT).round() as i16
}

/// Type 2 charstring operators.
const RLINETO: u8 = 5;
const RRCURVETO: u8 = 8;
const ENDCHAR: u8 = 14;
const RMOVETO: u8 = 21;

/// The most operands a charstring's stack holds.
const MAX_STACK: usize = 48;

/// A glyph's Type 2 charstring as it is written, with the advance `hmtx` has as its width and
/// no hints.
struct Charstring {
    out: Vec<u8>,
    /// The width (16.16), written before the first operator's operands; none when it is 0,
    /// the Private DICT's defaultWidthX.
    width: Option<i32>,
    /// The operator whose operands (16.16) wait to be written with more of the same.
    op: u8,
    args: Vec<i32>,
    /// The charstring's current point: the last point written.
    at: [i32; 2],
    /// Where the pen is: as `at`, else where a move left it or a closed contour's start.
    pen: [i32; 2],
    /// The start of the contour being drawn.
    start: [i32; 2],
    /// Whether a contour has been begun with a moveto since the last move or close.
    open: bool,
    /// The extremes the outline reaches, in 16.16 units.
    min: [f64; 2],
    max: [f64; 2],
}

impl Charstring {
    /// The charstring drawing `outline` at `advance`, and the outline's bounds.
    fn write(outline: &Outline, advance: u16) -> (Vec<u8>, Option<Bounds>) {
        let mut cs = Charstring {
            out: Vec::new(),
            width: (advance != 0).then(|| i32::from(advance) << 16),
            op: 0,
            args: Vec::new(),
            at: [0; 2],
            pen: [0; 2],
            start: [0; 2],
            open: false,
            min: [f64::INFINITY; 2],
            max: [f64::NEG_INFINITY; 2],
        };
        for seg in &outline.0 {
            match *seg {
                Seg::Move(x, y) => {
                    if let Some(p) = point(x, y) {
                        cs.pen = p;
                        cs.start = p;
                        cs.open = false;
                    }
                }
                Seg::Line(x, y) => {
                    if let Some(p) = point(x, y) {
                        cs.begin();
                        cs.add(RLINETO, &[p[0] - cs.at[0], p[1] - cs.at[1]]);
                        cs.reach(p);
                    }
                }
                Seg::Curve(x0, y0, x1, y1, x, y) => {
                    if let (Some(a), Some(b), Some(p)) = (point(x0, y0), point(x1, y1), point(x, y))
                    {
                        cs.begin();
                        let at = cs.at;
                        for axis in 0..2 {
                            let coords = [at, a, b, p].map(|q| f64::from(q[axis]));
                            for v in turns(coords) {
                                cs.reach_axis(axis, v.round());
                            }
                        }
                        cs.add(
                            RRCURVETO,
                            &[
                                a[0] - at[0],
                                a[1] - at[1],
                                b[0] - a[0],
                                b[1] - a[1],
                                p[0] - b[0],
                                p[1] - b[1],
                            ],
                        );
                        cs.reach(p);
                    }
                }
                Seg::Close => {
                    if cs.open {
                        cs.open = false;
                        cs.pen = cs.start;
                    }
                }
            }
        }
        cs.flush();
        if let Some(width) = cs.width.take() {
            operand(&mut cs.out, width);
        }
        cs.out.push(ENDCHAR);
        let bounds = (cs.min[0] <= cs.max[0]).then(|| {
            let whole = |v: f64, round: fn(f64) -> f64| round(v / 65_536.0) as i16;
            Bounds {
                x_min: whole(cs.min[0], f64::floor),
                y_min: whole(cs.min[1], f64::floor),
                x_max: whole(cs.max[0], f64::ceil),
                y_max: whole(cs.max[1], f64::ceil),
            }
        });
        (cs.out, bounds)
    }

    /// Begins a contour where the pen is, unless one is open.
    fn begin(&mut self) {
        if !self.open {
            let pen = self.pen;
            self.flush();
            self.op = RMOVETO;
            self.args.extend(self.width.take());
            self.args
                .extend_from_slice(&[pen[0] - self.at[0], pen[1] - self.at[1]]);
            self.flush();
            self.at = pen;
            self.start = pen;
            self.open = true;
            self.reach(pen);
        }
    }

    /// Adds a segment's operands, to be written with the ones before while they are for the
    /// same operator and the stack has room.
    fn add(&mut self, op: u8, args: &[i32]) {
        if op != self.op || self.args.len() + args.len() > MAX_STACK {
            self.flush();
            self.op = op;
        }
        self.args.extend_from_slice(args);
    }

    /// Writes the waiting operands and their operator.
    fn flush(&mut self) {
        if !self.args.is_empty() {
            for &v in &self.args {
                operand(&mut self.out, v);
            }
            self.out.push(self.op);
            self.args.clear();
        }
    }

    /// Moves the current point and the pen to `p`, where a segment ends, which the outline
    /// reaches.
    fn reach(&mut self, p: [i32; 2]) {
        self.at = p;
        self.pen = p;
        for (axis, &v) in p.iter().enumerate() {
            self.reach_axis(axis, f64::from(v));
        }
    }

    fn reach_axis(&mut self, axis: usize, v: f64) {
        self.min[axis] = self.min[axis].min(v);
        self.max[axis] = self.max[axis].max(v);
    }
}

/// The values one coordinate of a cubic Bézier takes where it turns inside the curve, from
/// that coordinate of its four points.
fn turns(p: [f64; 4]) -> impl Iterator<Item = f64> {
    let (d0, d1, d2) = (p[1] - p[0], p[2] - p[1], p[3] - p[2]);
    // The derivative over 3 is a t² + b t + c.
    let (a, b, c) = (d0 - 2.0 * d1 + d2, 2.0 * (d1 - d0), d0);
    let roots = if a == 0.0 {
        [(b != 0.0).then(|| -c / b), None]
    } else {
        let discriminant = b * b - 4.0 * a * c;
        if discriminant < 0.0 {
            [None, None]
        } else {
            let s = discriminant.sqrt();
            [Some((-b + s) / (2.0 * a)), Some((-b - s) / (2.0 * a))]
        }
    };
    roots
        .into_iter()
        .flatten()
        .filter(|t| *t > 0.0 && *t < 1.0)
        .map(move |t| {
            let u = 1.0 - t;
            u * u * u * p[0] + 3.0 * u * u * t * p[1] + 3.0 * u * t * t * p[2] + t * t * t * p[3]
        })
}

/// Writes a charstring operand given in 16.16 units: a whole number in its shortest form,
/// else as 16.16 fixed.
fn operand(out: &mut Vec<u8>, v: i32) {
    if v & 0xFFFF == 0 {
        int(out, v >> 16);
    } else {
        out.push(255);
        out.extend_from_slice(&v.to_be_bytes());
    }
}

/// Writes a whole number in the forms a charstring and a DICT share, up to 16 bits.
fn int(out: &mut Vec<u8>, v: i32) {
    match v {
        -107..=107 => out.push((v + 139) as u8),
        108..=1131 => out.extend_from_slice(&[((v - 108) >> 8) as u8 + 247, (v - 108) as u8]),
        -1131..=-108 => out.extend_from_slice(&[((-v - 108) >> 8) as u8 + 251, (-v - 108) as u8]),
        _ => {
            out.push(28);
            out.extend_from_slice(&(v.clamp(-32_768, 32_767) as i16).to_be_bytes());
        }
    }
}

/// Writes a DICT offset or size in five bytes, whatever its value, so that a DICT's size
/// doesn't depend on the offsets in it.
fn offset(out: &mut Vec<u8>, v: usize) {
    out.push(29);
    out.extend_from_slice(&i32::try_from(v).unwrap_or(i32::MAX).to_be_bytes());
}

/// A CFF INDEX of `items`.
fn index<T: AsRef<[u8]>>(items: &[T]) -> Vec<u8> {
    let items = &items[..items.len().min(65_535)];
    let mut out = Vec::new();
    out.u16(items.len() as u16);
    if items.is_empty() {
        return out;
    }
    let last = 1 + items.iter().map(|i| i.as_ref().len()).sum::<usize>();
    let size = match last {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    };
    out.push(size as u8);
    let mut at = 1;
    let put = |out: &mut Vec<u8>, at: usize| {
        out.extend_from_slice(&u32::try_from(at).unwrap_or(u32::MAX).to_be_bytes()[4 - size..]);
    };
    put(&mut out, at);
    for item in items {
        at += item.as_ref().len();
        put(&mut out, at);
    }
    for item in items {
        out.extend_from_slice(item.as_ref());
    }
    out
}

/// A glyph's name: `uni0041` in the BMP, `u1D400` beyond.
fn glyph_name(c: char) -> String {
    let c = u32::from(c);
    if c <= 0xFFFF {
        format!("uni{c:04X}")
    } else {
        format!("u{c:X}")
    }
}

/// The `CFF ` table: one font, not CID-keyed, its glyphs named after their characters, with
/// no subroutines.
fn cff(ps: &str, chars: &[char], glyphs: &[Drawn], bbox: Bounds) -> Vec<u8> {
    let name_index = index(&[ps]);
    // The glyphs' names, string ids 391 on in glyph order.
    let names: Vec<String> = chars.iter().map(|&c| glyph_name(c)).collect();
    let strings = index(&names);
    let global_subrs = index::<&[u8]>(&[]);
    let charset = match chars.len().checked_sub(1) {
        // Format 2: one range of string ids from 391.
        Some(left) => {
            let mut charset = vec![2];
            charset.u16(391);
            charset.u16(left as u16);
            charset
        }
        // Format 0 with no glyphs but .notdef.
        None => vec![0],
    };
    let charstrings: Vec<&[u8]> = glyphs.iter().map(|g| &g.charstring[..]).collect();
    let charstrings = index(&charstrings);
    // defaultWidthX 0, nominalWidthX 0.
    let private = [139, 20, 139, 21];
    let top = |charset_at, charstrings_at, private_at| {
        let mut dict = Vec::new();
        for v in [bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max] {
            int(&mut dict, v.into());
        }
        dict.push(5); // FontBBox
        offset(&mut dict, charset_at);
        dict.push(15); // charset
        offset(&mut dict, charstrings_at);
        dict.push(17); // CharStrings
        offset(&mut dict, private.len());
        offset(&mut dict, private_at);
        dict.push(18); // Private
        dict
    };
    let top_size = index(&[top(0, 0, 0)]).len();
    let charset_at = 4 + name_index.len() + top_size + strings.len() + global_subrs.len();
    let charstrings_at = charset_at + charset.len();
    let private_at = charstrings_at + charstrings.len();
    // Version 1.0, a 4-byte header, 4-byte offsets.
    let mut out = vec![1, 0, 4, 4];
    for part in [
        name_index,
        index(&[top(charset_at, charstrings_at, private_at)]),
        strings,
        global_subrs,
        charset,
        charstrings,
    ] {
        out.extend_from_slice(&part);
    }
    out.extend_from_slice(&private);
    out
}

/// The `cmap` table for `chars`, sorted, glyph `i + 1` drawing `chars[i]`: format 4 under
/// (0, 3) and (3, 1), and format 12 with every character under (0, 4) and (3, 10) as well
/// when some are beyond U+FFFE or there are more runs of them than format 4 holds.
fn cmap(chars: &[char]) -> Vec<u8> {
    // Runs of consecutive characters, which consecutive glyphs draw: first, last, first glyph.
    let mut runs: Vec<[u32; 3]> = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        let c = u32::from(c);
        match runs.last_mut() {
            Some(run) if run[1] + 1 == c => run[1] = c,
            _ => runs.push([c, c, i as u32 + 1]),
        }
    }
    let bmp: Vec<[u32; 3]> = runs
        .iter()
        .filter(|r| r[0] < 0xFFFF)
        .map(|&[first, last, gid]| [first, last.min(0xFFFE), gid])
        .collect();
    let full = chars.last().is_some_and(|&c| u32::from(c) >= 0xFFFF) || bmp.len() > MAX_SEGMENTS;
    let format4 = format4(&bmp[..bmp.len().min(MAX_SEGMENTS)]);
    let records: &[(u16, u16, bool)] = if full {
        &[(0, 3, false), (0, 4, true), (3, 1, false), (3, 10, true)]
    } else {
        &[(0, 3, false), (3, 1, false)]
    };
    let format4_at = 4 + 8 * records.len() as u32;
    let format12_at = format4_at + format4.len() as u32;
    let mut t = Vec::new();
    t.u16(0);
    t.u16(records.len() as u16);
    for &(platform, encoding, every) in records {
        t.u16(platform);
        t.u16(encoding);
        t.u32(if every { format12_at } else { format4_at });
    }
    t.extend_from_slice(&format4);
    if full {
        t.u16(12);
        t.u16(0);
        t.u32(16 + 12 * runs.len() as u32);
        t.u32(0); // language
        t.u32(runs.len() as u32);
        for run in &runs {
            for v in run {
                t.u32(*v);
            }
        }
    }
    t
}

/// A format 4 cmap subtable of `runs` (first, last, first glyph), each a segment, then the
/// last segment for U+FFFF.
fn format4(runs: &[[u32; 3]]) -> Vec<u8> {
    let segments: Vec<[u16; 3]> = runs
        .iter()
        .map(|&[first, last, gid]| [first as u16, last as u16, gid.wrapping_sub(first) as u16])
        .chain([[0xFFFF, 0xFFFF, 1]])
        .collect();
    let n = segments.len() as u16;
    let selector = 15 - n.leading_zeros() as u16;
    let mut t = Vec::new();
    t.u16(4);
    t.u16(16 + 8 * n); // length
    t.u16(0); // language
    t.u16(2 * n);
    t.u16(2 << selector); // searchRange
    t.u16(selector);
    t.u16(2 * n - (2 << selector)); // rangeShift
    for s in &segments {
        t.u16(s[1]);
    }
    t.u16(0);
    for s in &segments {
        t.u16(s[0]);
    }
    for s in &segments {
        t.u16(s[2]); // idDelta
    }
    for _ in &segments {
        t.u16(0); // idRangeOffset
    }
    t
}

/// The `OS/2` table, version 4.
fn os2(chars: &[char], glyphs: &[Drawn], ascent: i16, descent: i16, bbox: Bounds) -> Vec<u8> {
    let widths: Vec<u64> = glyphs
        .iter()
        .map(|g| u64::from(g.advance))
        .filter(|&a| a > 0)
        .collect();
    let n = widths.len() as u64;
    let average = (widths.iter().sum::<u64>() + n / 2)
        .checked_div(n)
        .unwrap_or(0);
    let char_index = |c: Option<&char>| c.map_or(0, |&c| u32::from(c).min(0xFFFF) as u16);
    let win_ascent = ascent.max(bbox.y_max).max(0) as u16;
    let win_descent = (-i32::from(descent.min(bbox.y_min))).max(0) as u16;
    let mut t = Vec::with_capacity(96);
    t.u16(4);
    t.i16(average.min(32_767) as i16); // xAvgCharWidth
    t.u16(400); // usWeightClass: regular
    t.u16(5); // usWidthClass: medium
    t.u16(0); // fsType: installable

    // Subscripts' and superscripts' sizes and offsets, the strikeout's size and position.
    for v in [650, 600, 0, 75, 650, 600, 0, 350, 50, 250] {
        t.i16(v);
    }
    t.i16(0); // sFamilyClass
    t.extend_from_slice(&[0; 10]); // PANOSE: any
    t.extend_from_slice(&[0; 16]); // ulUnicodeRange1 to 4
    t.extend_from_slice(b"NONE"); // achVendID
    t.u16(0x00C0); // fsSelection: REGULAR, USE_TYPO_METRICS
    t.u16(char_index(chars.first()));
    t.u16(char_index(chars.last()));
    t.i16(ascent);
    t.i16(descent);
    t.i16(0); // sTypoLineGap
    t.u16(win_ascent);
    t.u16(win_descent);
    t.extend_from_slice(&[0; 8]); // ulCodePageRange1 and 2
    t.i16(0); // sxHeight
    t.i16(0); // sCapHeight
    t.u16(0); // usDefaultChar
    t.u16(0x20); // usBreakChar
    t.u16(0); // usMaxContext
    t
}

/// The `head` table; its `checkSumAdjustment` is set once the whole file is written.
fn head(bbox: Bounds) -> Vec<u8> {
    let mut t = Vec::with_capacity(54);
    t.u16(1);
    t.u16(0);
    t.u32(0x0001_0000); // fontRevision 1.0
    t.u32(0); // checkSumAdjustment
    t.u32(0x5F0F_3CF5); // magicNumber
    t.u16(0b11); // flags: baseline at y = 0, left side bearing at x = 0
    t.u16(UNITS_PER_EM);
    // Created and modified: 1 January 2026, in seconds from 1904, so the file is the same each
    // time (0 reads as a Unix time to fontTools).
    for _ in 0..2 {
        t.extend_from_slice(&3_850_070_400_i64.to_be_bytes());
    }
    for v in [bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max] {
        t.i16(v);
    }
    t.u16(0); // macStyle
    t.u16(8); // lowestRecPPEM
    t.i16(2); // fontDirectionHint
    t.i16(0); // indexToLocFormat
    t.i16(0); // glyphDataFormat
    t
}

/// The `hhea` table, with a metric for each glyph.
fn hhea(glyphs: &[Drawn], ascent: i16, descent: i16) -> Vec<u8> {
    let inked = || glyphs.iter().filter_map(|g| Some((g.advance, g.bounds?)));
    let min_lsb = inked().map(|(_, b)| b.x_min).min().unwrap_or(0);
    let min_rsb = inked()
        .map(|(a, b)| i32::from(a) - i32::from(b.x_max))
        .min()
        .unwrap_or(0)
        .clamp(-32_768, 32_767) as i16;
    let x_max_extent = inked().map(|(_, b)| b.x_max).max().unwrap_or(0);
    let mut t = Vec::with_capacity(36);
    t.u16(1);
    t.u16(0);
    t.i16(ascent);
    t.i16(descent);
    t.i16(0); // lineGap
    t.u16(glyphs.iter().map(|g| g.advance).max().unwrap_or(0));
    t.i16(min_lsb);
    t.i16(min_rsb);
    t.i16(x_max_extent);
    t.i16(1); // caretSlopeRise
    t.i16(0); // caretSlopeRun
    t.i16(0); // caretOffset
    t.extend_from_slice(&[0; 8]); // reserved
    t.i16(0); // metricDataFormat
    t.u16(glyphs.len() as u16); // numberOfHMetrics
    t
}

/// The `hmtx` table: each glyph's advance, and its outline's left edge as its left side
/// bearing.
fn hmtx(glyphs: &[Drawn]) -> Vec<u8> {
    let mut t = Vec::with_capacity(4 * glyphs.len());
    for g in glyphs {
        t.u16(g.advance);
        t.i16(g.bounds.map_or(0, |b| b.x_min));
    }
    t
}

/// The `maxp` table, version 0.5 as a CFF font's is.
fn maxp(glyphs: usize) -> Vec<u8> {
    let mut t = Vec::with_capacity(6);
    t.u32(0x0000_5000);
    t.u16(glyphs as u16);
    t
}

/// The `name` table, in Windows' Unicode (UTF-16) English names.
fn names(family: &str, ps: &str) -> Vec<u8> {
    let unique = format!("{ps};LimarePDF preview");
    let strings = [
        (1, family),
        (2, "Regular"),
        (3, unique.as_str()),
        (4, family),
        (5, "Version 1.000"),
        (6, ps),
    ];
    let mut t = Vec::new();
    t.u16(0);
    t.u16(strings.len() as u16);
    t.u16(6 + 12 * strings.len() as u16); // storageOffset
    let mut storage = Vec::new();
    for (id, s) in strings {
        let at = storage.len() as u16;
        for unit in s.encode_utf16() {
            storage.u16(unit);
        }
        t.u16(3); // Windows
        t.u16(1); // Unicode BMP
        t.u16(0x409); // English (United States)
        t.u16(id);
        t.u16(storage.len() as u16 - at);
        t.u16(at);
    }
    t.extend_from_slice(&storage);
    t
}

/// The `post` table, version 3.0: no glyph names (the CFF table has them).
fn post() -> Vec<u8> {
    let mut t = Vec::with_capacity(32);
    t.u32(0x0003_0000);
    t.u32(0); // italicAngle
    t.i16(-100); // underlinePosition
    t.i16(50); // underlineThickness
    t.extend_from_slice(&[0; 20]); // isFixedPitch, and the memory hints
    t
}

/// The PostScript name: `name`'s ASCII letters, digits and hyphens, at most 63 of them, else
/// "Preview".
fn postscript_name(name: &str) -> String {
    let ps: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(63)
        .collect();
    if ps.is_empty() {
        "Preview".to_owned()
    } else {
        ps
    }
}

/// The OpenType file of `tables` (tags and contents): sorted by tag, each 4-byte aligned, with
/// their checksums and the `head` table's `checkSumAdjustment`.
fn sfnt(mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|t| t.0);
    let n = tables.len() as u16;
    let selector = 15 - n.leading_zeros() as u16;
    let mut out = Vec::new();
    out.extend_from_slice(b"OTTO");
    out.u16(n);
    out.u16(16 << selector); // searchRange
    out.u16(selector);
    out.u16(16 * n - (16 << selector)); // rangeShift
    let mut at = 12 + 16 * tables.len();
    let mut head = None;
    for (tag, data) in &tables {
        out.extend_from_slice(tag);
        out.u32(checksum(data));
        out.u32(at as u32);
        out.u32(data.len() as u32);
        if tag == b"head" {
            head = Some(at);
        }
        at += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    if let Some(head) = head {
        let adjustment = 0xB1B0_AFBA_u32.wrapping_sub(checksum(&out));
        out[head + 8..head + 12].copy_from_slice(&adjustment.to_be_bytes());
    }
    out
}

/// The OpenType checksum of `data`: the sum of its big-endian 32-bit words, the last padded
/// with zeros.
fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0, |sum: u32, chunk| {
        let mut word = [0; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

/// Big-endian numbers, as the tables hold them.
trait Put {
    fn u16(&mut self, v: u16);
    fn i16(&mut self, v: i16);
    fn u32(&mut self, v: u32);
}

impl Put for Vec<u8> {
    fn u16(&mut self, v: u16) {
        self.extend_from_slice(&v.to_be_bytes());
    }

    fn i16(&mut self, v: i16) {
        self.extend_from_slice(&v.to_be_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.extend_from_slice(&v.to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrifa::instance::{LocationRef, Size};
    use skrifa::outline::DrawSettings;
    use skrifa::raw::ps::cff::CffFontRef;
    use skrifa::raw::tables::cmap::PlatformId;
    use skrifa::raw::types::{Tag, Version16Dot16};
    use skrifa::raw::TableProvider;
    use skrifa::string::StringId;
    use skrifa::{FontRef, GlyphId, MetadataProvider};
    use std::fmt::Debug;
    use std::time::Instant;
    use Seg::{Close, Curve, Line, Move};

    const SOURCE_SERIF: &[u8] = include_bytes!("../tests/data/fonts/SourceSerif4-Regular.otf");
    const LIBERATION_SANS: &[u8] = include_bytes!("../assets/fonts/LiberationSans-Regular.ttf");
    const SYMBOLIC: &[u8] = include_bytes!("../tests/data/fonts/LimareSymbolic.ttf");

    const METRICS: Metrics = Metrics {
        ascent: 750.0,
        descent: -250.0,
    };

    /// The step of a 16.16 coordinate.
    const STEP: f32 = 1.0 / 65_536.0;

    fn glyph(c: char, advance: f32, segs: &[Seg]) -> Glyph {
        Glyph {
            c,
            advance,
            outline: Outline(segs.to_vec()),
        }
    }

    /// The outline skrifa draws for `gid`, unhinted at `size`, recorded through [`Outline`].
    fn skrifa_draws(font: &FontRef, gid: GlyphId, size: Size) -> Outline {
        let mut pen = Outline::default();
        font.outline_glyphs()
            .get(gid)
            .expect("an outline")
            .draw(
                DrawSettings::unhinted(size, LocationRef::default()),
                &mut pen,
            )
            .expect("drawn");
        pen
    }

    /// The outline `gid`'s charstring draws as read-fonts evaluates it, in 16.16 units
    /// (without the rounding to whole units of skrifa's unhinted drawing), and its width.
    fn charstring_draws(font: &FontRef, gid: GlyphId) -> (Outline, f32) {
        let data = font.table_data(Tag::new(b"CFF ")).expect("a CFF table");
        let cff = CffFontRef::new(data.as_bytes(), 0, Some(1000)).expect("a CFF font");
        let subfont = cff
            .subfont(cff.subfont_index(gid).unwrap_or(0), &[])
            .expect("a subfont");
        let mut pen = Outline::default();
        let width = cff
            .evaluate_charstring(&subfont, gid, &[], &mut pen)
            .expect("evaluated")
            .expect("a width");
        (pen, width.to_f32())
    }

    /// `outline` with its coordinates as skrifa draws a CFF outline unhinted and unscaled,
    /// in whole units as FreeType does: rounded to 1/64 of a unit, then down.
    fn as_skrifa_draws(outline: &Outline) -> Outline {
        let unit = |v: f32| {
            let product = (f64::from(v) * 65_536.0).round() as i64 * 1024;
            (((product + 0x8000 - i64::from(product < 0)) >> 16) >> 10) as f32
        };
        Outline(
            outline
                .0
                .iter()
                .map(|seg| match *seg {
                    Move(x, y) => Move(unit(x), unit(y)),
                    Line(x, y) => Line(unit(x), unit(y)),
                    Curve(a, b, c, d, x, y) => {
                        Curve(unit(a), unit(b), unit(c), unit(d), unit(x), unit(y))
                    }
                    Close => Close,
                })
                .collect(),
        )
    }

    /// `outline` as skrifa draws it back: each contour closed, without contours that draw
    /// nothing, lines to where the pen already is, or a last line back to the contour's start.
    fn normalised(outline: &Outline) -> Vec<Seg> {
        fn end(contour: &mut Vec<Seg>, out: &mut Vec<Seg>, start: (f32, f32)) {
            if contour.last() == Some(&Line(start.0, start.1)) {
                contour.pop();
            }
            if contour.len() > 1 {
                out.append(contour);
                out.push(Close);
            }
            contour.clear();
        }
        let (mut out, mut contour) = (Vec::new(), Vec::new());
        let (mut start, mut at) = ((0.0, 0.0), (0.0, 0.0));
        for &seg in &outline.0 {
            match seg {
                Move(x, y) => {
                    end(&mut contour, &mut out, start);
                    contour.push(seg);
                    (start, at) = ((x, y), (x, y));
                }
                Line(x, y) => {
                    if (x, y) != at {
                        contour.push(seg);
                    }
                    at = (x, y);
                }
                Curve(_, _, _, _, x, y) => {
                    contour.push(seg);
                    at = (x, y);
                }
                Close => {
                    end(&mut contour, &mut out, start);
                    at = start;
                }
            }
        }
        end(&mut contour, &mut out, start);
        out
    }

    /// Asserts `got` draws `want`'s segments, each coordinate within `tolerance`.
    fn assert_draws(got: &Outline, want: &Outline, tolerance: f32, what: impl Debug) {
        fn parts(seg: Seg) -> (u8, Vec<f32>) {
            match seg {
                Move(x, y) => (0, vec![x, y]),
                Line(x, y) => (1, vec![x, y]),
                Curve(a, b, c, d, x, y) => (2, vec![a, b, c, d, x, y]),
                Close => (3, Vec::new()),
            }
        }
        let (got, want) = (normalised(got), normalised(want));
        assert_eq!(
            got.len(),
            want.len(),
            "{what:?}: {got:?} drawn for {want:?}"
        );
        for (&g, &w) in got.iter().zip(&want) {
            let ((g_kind, g_coords), (w_kind, w_coords)) = (parts(g), parts(w));
            let near = g_coords
                .iter()
                .zip(&w_coords)
                .all(|(a, b)| (a - b).abs() <= tolerance);
            assert!(g_kind == w_kind && near, "{what:?}: {g:?} drawn for {w:?}");
        }
    }

    /// The OpenType checksum, summed byte by byte.
    fn sum(data: &[u8]) -> u32 {
        data.iter().enumerate().fold(0, |sum: u32, (i, &b)| {
            sum.wrapping_add(u32::from(b) << (24 - 8 * (i % 4)))
        })
    }

    /// Checks `font`'s table directory (sorted by tag; searchRange, entrySelector and
    /// rangeShift), that each table starts 4-byte aligned without overlapping another, is
    /// padded with zeros and has its checksum, and the whole file's checksum; returns the tags.
    fn check_tables(font: &[u8]) -> Vec<[u8; 4]> {
        let u16_at = |at: usize| usize::from(u16::from_be_bytes([font[at], font[at + 1]]));
        let u32_at = |at: usize| u32::from_be_bytes(font[at..at + 4].try_into().unwrap());
        assert_eq!(&font[..4], b"OTTO");
        let n = u16_at(4);
        let power = 1 << n.ilog2();
        assert_eq!(u16_at(6), 16 * power, "searchRange");
        assert_eq!(1 << u16_at(8), power, "entrySelector");
        assert_eq!(u16_at(10), 16 * (n - power), "rangeShift");
        let (mut tags, mut spans) = (Vec::new(), Vec::new());
        for record in (12..12 + 16 * n).step_by(16) {
            let tag: [u8; 4] = font[record..record + 4].try_into().unwrap();
            let name = String::from_utf8_lossy(&tag).into_owned();
            let at = u32_at(record + 8) as usize;
            let len = u32_at(record + 12) as usize;
            let end = at + len.next_multiple_of(4);
            assert_eq!(at % 4, 0, "{name} aligned");
            assert!(font[at + len..end].iter().all(|&b| b == 0), "{name} padded");
            let mut table = font[at..end].to_vec();
            if &tag == b"head" {
                table[8..12].fill(0);
            }
            assert_eq!(sum(&table), u32_at(record + 4), "{name} checksum");
            tags.push(tag);
            spans.push((at, end));
        }
        assert!(tags.windows(2).all(|w| w[0] < w[1]), "sorted by tag");
        spans.sort();
        assert_eq!(spans[0].0, 12 + 16 * n);
        assert!(spans.windows(2).all(|w| w[0].1 <= w[1].0), "no overlaps");
        assert_eq!(spans[spans.len() - 1].1, font.len());
        assert_eq!(sum(font), 0xB1B0_AFBA, "checkSumAdjustment");
        tags
    }

    /// The glyphs for the characters of `program` that `keep` keeps, recorded through
    /// [`Outline`] as skrifa draws them at `size` (every glyph of the program is drawn), with
    /// their advances at that size rounded.
    fn recorded(program: &[u8], size: Size, keep: impl Fn(char) -> bool) -> Vec<Glyph> {
        let font = FontRef::new(program).unwrap();
        let drawn: Vec<Outline> = (0..font.maxp().unwrap().num_glyphs())
            .map(|gid| skrifa_draws(&font, GlyphId::new(gid.into()), size))
            .collect();
        let widths = font.glyph_metrics(size, LocationRef::default());
        font.charmap()
            .mappings()
            .filter_map(|(c, gid)| {
                let c = char::from_u32(c).filter(|&c| keep(c))?;
                Some(Glyph {
                    c,
                    advance: widths.advance_width(gid)?.round(),
                    outline: drawn.get(gid.to_u32() as usize)?.clone(),
                })
            })
            .collect()
    }

    /// Checks that each of `glyphs` draws back from `font`, made of them, as given and at its
    /// advance.
    fn assert_draws_back(font: &[u8], glyphs: &[Glyph]) {
        check_tables(font);
        let font = FontRef::new(font).expect("an OpenType font");
        let charmap = font.charmap();
        let advances = font.glyph_metrics(Size::unscaled(), LocationRef::default());
        let at_1000 = font.glyph_metrics(Size::new(1000.0), LocationRef::default());
        for g in glyphs {
            let gid = charmap.map(g.c).expect("mapped");
            assert_ne!(gid, GlyphId::NOTDEF);
            assert_eq!(advances.advance_width(gid), Some(g.advance), "{:?}", g.c);
            assert_eq!(at_1000.advance_width(gid), Some(g.advance), "{:?}", g.c);
            let (exact, width) = charstring_draws(&font, gid);
            assert_draws(&exact, &g.outline, STEP, g.c);
            // The charstring's width is the advance too.
            assert_eq!(width, g.advance, "{:?}", g.c);
            let unscaled = skrifa_draws(&font, gid, Size::unscaled());
            assert_draws(&unscaled, &as_skrifa_draws(&g.outline), 0.0, g.c);
        }
    }

    #[test]
    fn each_character_maps_to_a_glyph_drawing_its_outline_at_its_advance() {
        let mut quadratic = Outline::default();
        quadratic.move_to(0.0, 0.0);
        quadratic.quad_to(250.0, 700.0, 500.0, 0.0);
        quadratic.quad_to(250.0, 301.0, 0.0, 0.0);
        quadratic.close();
        let square = [
            Move(100.0, 0.0),
            Line(500.0, 0.0),
            Line(500.0, 400.0),
            Line(100.0, 400.0),
            Close,
        ];
        let arch = [
            Move(50.0, -10.5),
            Curve(50.0, 650.25, 450.0, 650.25, 450.0, -10.5),
            Line(250.0, -200.0),
            Close,
        ];
        let bar = [
            Move(50.0, 300.0),
            Line(950.0, 300.0),
            Line(950.0, 360.0),
            Line(50.0, 360.0),
            Close,
        ];
        let given = [
            glyph('\u{4E00}', 1000.0, &bar),
            glyph('A', 600.0, &square),
            glyph(' ', 250.0, &[]),
            glyph(
                '\u{1D400}',
                700.0,
                &[Move(0.0, 0.0), Line(300.0, 700.0), Line(600.0, 0.0)],
            ),
            Glyph {
                c: 'B',
                advance: 520.0,
                outline: quadratic,
            },
            glyph('C', 500.0, &arch),
            // Given again: the first is kept.
            glyph('A', 999.0, &bar),
        ];
        let bytes = opentype("Limare Preview", &given, METRICS);
        assert_draws_back(&bytes, &given[..6]);
        let font = FontRef::new(&bytes).unwrap();
        let head = font.head().unwrap();
        assert_eq!(
            (head.units_per_em(), head.magic_number()),
            (1000, 0x5F0F_3CF5)
        );
        assert_eq!(head.index_to_loc_format(), 0);
        assert_eq!(
            (head.x_min(), head.y_min(), head.x_max(), head.y_max()),
            (0, -200, 950, 700)
        );
        let maxp = font.maxp().unwrap();
        assert_eq!(
            (maxp.version(), maxp.num_glyphs()),
            (Version16Dot16::VERSION_0_5, 7)
        );
        assert_eq!(font.post().unwrap().version(), Version16Dot16::VERSION_3_0);
        let os2 = font.os2().unwrap();
        assert_eq!(
            (os2.version(), os2.us_weight_class(), os2.fs_type()),
            (4, 400, 0)
        );
        assert_eq!(
            (os2.us_first_char_index(), os2.us_last_char_index()),
            (0x20, 0xFFFF)
        );
        let hhea = font.hhea().unwrap();
        assert_eq!(hhea.number_of_h_metrics(), 7);
        assert_eq!(hhea.advance_width_max().to_u16(), 1000);
        assert_eq!(hhea.x_max_extent().to_i16(), 950);
        let name = |id| {
            font.localized_strings(id)
                .english_or_first()
                .map(|s| s.to_string())
        };
        assert_eq!(
            name(StringId::FAMILY_NAME).as_deref(),
            Some("Limare Preview")
        );
        assert_eq!(name(StringId::FULL_NAME).as_deref(), Some("Limare Preview"));
        assert_eq!(name(StringId::SUBFAMILY_NAME).as_deref(), Some("Regular"));
        assert_eq!(
            name(StringId::POSTSCRIPT_NAME).as_deref(),
            Some("LimarePreview")
        );
        // What egui lays lines out with.
        let metrics = font.metrics(Size::unscaled(), LocationRef::default());
        assert_eq!(
            (
                metrics.units_per_em,
                metrics.ascent,
                metrics.descent,
                metrics.leading
            ),
            (1000, 750.0, -250.0, 0.0)
        );
        let charmap = font.charmap();
        assert_eq!(charmap.map('Z'), None);
        // Glyphs follow their characters' order.
        let gids: Vec<u32> = [' ', 'A', 'B', 'C', '\u{4E00}', '\u{1D400}']
            .map(|c| charmap.map(c).unwrap().to_u32())
            .to_vec();
        assert_eq!(gids, [1, 2, 3, 4, 5, 6]);
        let names = font.glyph_names();
        let named = |gid: u32| names.get(GlyphId::new(gid)).unwrap().as_str().to_owned();
        assert_eq!(
            [named(0), named(2), named(5), named(6)],
            [".notdef", "uni0041", "uni4E00", "u1D400"]
        );
        // The square of whole units is drawn back exactly by skrifa unscaled too.
        let a = charmap.map('A').unwrap();
        assert_eq!(
            normalised(&skrifa_draws(&font, a, Size::unscaled())),
            square
        );
        // An empty glyph's left side bearing is 0, others' their left edge.
        let lsb = |c| font.hmtx().unwrap().side_bearing(charmap.map(c).unwrap());
        assert_eq!(
            [lsb(' '), lsb('A'), lsb('C')],
            [Some(0), Some(100), Some(50)]
        );
    }

    #[test]
    fn a_quadratic_segment_is_recorded_as_the_cubic_that_draws_it() {
        let mut outline = Outline::default();
        outline.move_to(10.0, 20.0);
        outline.quad_to(100.0, 200.0, 190.0, 20.0);
        outline.close();
        // After a close, the pen is back at the contour's start.
        outline.quad_to(-50.0, 80.0, 30.0, 40.0);
        let bezier = |p: &[(f32, f32)], t: f32| {
            let u = 1.0 - t;
            let weights = match p.len() {
                3 => vec![u * u, 2.0 * u * t, t * t],
                _ => vec![u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t],
            };
            p.iter()
                .zip(weights)
                .fold((0.0, 0.0), |(x, y), (q, w)| (x + w * q.0, y + w * q.1))
        };
        let quadratics = [
            [(10.0, 20.0), (100.0, 200.0), (190.0, 20.0)],
            [(10.0, 20.0), (-50.0, 80.0), (30.0, 40.0)],
        ];
        for (seg, quadratic) in [outline.0[1], outline.0[3]].into_iter().zip(quadratics) {
            let Curve(x0, y0, x1, y1, x, y) = seg else {
                panic!("{seg:?} for a quadratic")
            };
            let cubic = [quadratic[0], (x0, y0), (x1, y1), (x, y)];
            for t in [0.0, 0.1, 0.25, 0.5, 0.8, 1.0] {
                let (a, b) = (bezier(&quadratic, t), bezier(&cubic, t));
                assert!(
                    (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3,
                    "{a:?} {b:?}"
                );
            }
        }
    }

    #[test]
    fn glyphs_recorded_from_cff_and_truetype_programs_draw_back_as_recorded() {
        // Source Serif 4 is CFF with 1000 units per em; Liberation Sans and Limare Symbolic are
        // TrueType, recorded at 1000 ppem for 1/1000 em, their quadratics becoming cubics.
        for (program, size) in [
            (SOURCE_SERIF, Size::unscaled()),
            (LIBERATION_SANS, Size::new(1000.0)),
            (SYMBOLIC, Size::new(1000.0)),
        ] {
            let glyphs = recorded(program, size, |_| true);
            assert!(glyphs.len() > 5);
            assert_draws_back(&opentype("Recorded", &glyphs, METRICS), &glyphs);
        }
    }

    #[test]
    fn tables_are_sorted_aligned_and_checksummed() {
        let latin = recorded(SOURCE_SERIF, Size::unscaled(), |c| c <= 'ÿ');
        for glyphs in [&[][..], &latin[..], &[glyph('\u{1D400}', 500.0, &[])][..]] {
            let tags = check_tables(&opentype("Tables", glyphs, METRICS));
            let want = [
                *b"CFF ", *b"OS/2", *b"cmap", *b"head", *b"hhea", *b"hmtx", *b"maxp", *b"name",
                *b"post",
            ];
            assert_eq!(tags, want);
        }
    }

    #[test]
    fn a_preview_font_takes_little_room() {
        let latin = recorded(SOURCE_SERIF, Size::unscaled(), |c| c <= 'ÿ');
        let all = recorded(SOURCE_SERIF, Size::unscaled(), |_| true);
        for glyphs in [latin, all] {
            let bytes = opentype("Source Serif 4", &glyphs, METRICS).len();
            println!(
                "Source Serif 4: {} characters in {bytes} bytes, {} a character",
                glyphs.len(),
                bytes / glyphs.len()
            );
            assert!(bytes / glyphs.len() < 200, "{bytes} bytes");
        }
    }

    #[test]
    fn a_font_without_glyphs_still_reads() {
        let bytes = opentype("", &[], METRICS);
        check_tables(&bytes);
        let font = FontRef::new(&bytes).unwrap();
        assert_eq!(font.maxp().unwrap().num_glyphs(), 1);
        assert_eq!(font.charmap().map('A'), None);
        assert_eq!(
            font.glyph_names().get(GlyphId::NOTDEF).unwrap().as_str(),
            ".notdef"
        );
        assert!(skrifa_draws(&font, GlyphId::NOTDEF, Size::unscaled())
            .0
            .is_empty());
        let family = font
            .localized_strings(StringId::FAMILY_NAME)
            .english_or_first();
        assert_eq!(family.unwrap().to_string(), "Preview");
        let os2 = font.os2().unwrap();
        assert_eq!(
            (os2.us_first_char_index(), os2.us_last_char_index()),
            (0, 0)
        );
    }

    #[test]
    fn an_empty_glyph_draws_nothing_at_its_advance() {
        let space = [glyph(' ', 250.0, &[Move(10.0, 10.0), Close])];
        let bytes = opentype("Space", &space, METRICS);
        assert_draws_back(&bytes, &space);
        let font = FontRef::new(&bytes).unwrap();
        let gid = font.charmap().map(' ').unwrap();
        assert!(skrifa_draws(&font, gid, Size::unscaled()).0.is_empty());
        let head = font.head().unwrap();
        assert_eq!(
            (head.x_min(), head.y_min(), head.x_max(), head.y_max()),
            (0, 0, 0, 0)
        );
        // A move that draws nothing isn't written: the charstring is its width (250, two
        // bytes) and endchar; .notdef, of width 0, just endchar.
        let data = font.table_data(Tag::new(b"CFF ")).unwrap();
        let cff = CffFontRef::new(data.as_bytes(), 0, Some(1000)).unwrap();
        let charstring = |gid: u32| cff.charstrings().get(gid as usize).unwrap();
        assert_eq!(charstring(gid.to_u32()), [247, 142, ENDCHAR]);
        assert_eq!(charstring(0), [ENDCHAR]);
    }

    #[test]
    fn five_thousand_glyphs_take_well_under_a_second() {
        // Each about as much as a Chinese character: twelve contours of lines and curves.
        let glyphs: Vec<Glyph> = (0..5000)
            .map(|i| {
                let mut outline = Outline::default();
                for k in 0..12 {
                    let (x, y) = (
                        (k % 4) as f32 * 240.0 + 20.0,
                        (k / 4) as f32 * 300.0 - 100.0,
                    );
                    let s = (i % 97) as f32 * 0.37;
                    outline.move_to(x, y);
                    outline.line_to(x + 200.0, y + s);
                    outline.curve_to(
                        x + 220.0,
                        y + 50.0,
                        x + 220.0,
                        y + 150.0,
                        x + 200.0,
                        y + 200.0,
                    );
                    outline.line_to(x + 100.5, y + 210.0);
                    outline.quad_to(x - 30.0, y + 150.0, x, y + 100.0);
                    outline.line_to(x + s, y + 50.0);
                    outline.close();
                }
                let c = char::from_u32(0x3400 + 3 * i).unwrap();
                Glyph {
                    c,
                    advance: 1000.0,
                    outline,
                }
            })
            .collect();
        let start = Instant::now();
        let bytes = opentype("Many", &glyphs, METRICS);
        let took = start.elapsed();
        println!(
            "{} glyphs written in {took:?}, {} bytes",
            glyphs.len(),
            bytes.len()
        );
        assert!(took.as_secs_f64() < 1.0, "{took:?}");
        let font = FontRef::new(&bytes).unwrap();
        assert_eq!(font.maxp().unwrap().num_glyphs(), 5001);
        let sample: Vec<Glyph> = glyphs.iter().step_by(97).cloned().collect();
        assert_draws_back(&bytes, &sample);
    }

    #[test]
    fn coordinates_that_are_not_numbers_are_left_out_and_huge_ones_kept_within_16_em() {
        let odd = glyph(
            'X',
            f32::NAN,
            &[
                Move(f32::NAN, 0.0),
                Move(0.0, 0.0),
                Line(f32::INFINITY, 5.0),
                Line(1e9, 1e9),
                Curve(0.0, f32::NAN, 1.0, 1.0, 2.0, 2.0),
                Line(-1e30, f32::MIN),
                Close,
            ],
        );
        let metrics = Metrics {
            ascent: f32::NAN,
            descent: f32::INFINITY,
        };
        let bytes = opentype("Odd", &[odd, glyph('Y', f32::INFINITY, &[])], metrics);
        check_tables(&bytes);
        let font = FontRef::new(&bytes).unwrap();
        let charmap = font.charmap();
        let (x, y) = (charmap.map('X').unwrap(), charmap.map('Y').unwrap());
        let advances = font.glyph_metrics(Size::unscaled(), LocationRef::default());
        assert_eq!(
            (advances.advance_width(x), advances.advance_width(y)),
            (Some(0.0), Some(0.0))
        );
        let want = [
            Move(0.0, 0.0),
            Line(16_000.0, 16_000.0),
            Line(-16_000.0, -16_000.0),
            Close,
        ];
        assert_eq!(normalised(&skrifa_draws(&font, x, Size::unscaled())), want);
        let metrics = font.metrics(Size::unscaled(), LocationRef::default());
        assert_eq!((metrics.ascent, metrics.descent), (800.0, -200.0));
        let head = font.head().unwrap();
        assert_eq!(
            (head.x_min(), head.y_min(), head.x_max(), head.y_max()),
            (-16_000, -16_000, 16_000, 16_000)
        );
    }

    #[test]
    fn beyond_the_most_characters_the_first_given_are_kept() {
        let chars: Vec<char> = (0x20..)
            .filter_map(char::from_u32)
            .take(MAX_CHARS + 10)
            .collect();
        // Given from the highest down, so the lowest ten are left out.
        let glyphs: Vec<Glyph> = chars.iter().rev().map(|&c| glyph(c, 500.0, &[])).collect();
        let bytes = opentype("Full", &glyphs, METRICS);
        check_tables(&bytes);
        let font = FontRef::new(&bytes).unwrap();
        assert_eq!(
            usize::from(font.maxp().unwrap().num_glyphs()),
            MAX_CHARS + 1
        );
        let charmap = font.charmap();
        assert!(chars[..10].iter().all(|&c| charmap.map(c).is_none()));
        assert!(chars[10..].iter().all(|&c| charmap.map(c).is_some()));
        assert!(charmap.map('\u{FFFF}').is_some());
        let names = font.glyph_names();
        let last = *chars.last().unwrap();
        let named = names.get(charmap.map(last).unwrap()).unwrap();
        assert_eq!(named.as_str(), glyph_name(last));
    }

    #[test]
    fn characters_too_scattered_for_format_4_are_all_mapped_by_format_12() {
        let chars: Vec<char> = (0..20_000)
            .map(|i| char::from_u32(0x100 + 2 * i).unwrap())
            .collect();
        let glyphs: Vec<Glyph> = chars.iter().map(|&c| glyph(c, 500.0, &[])).collect();
        let bytes = opentype("Scattered", &glyphs, METRICS);
        check_tables(&bytes);
        let font = FontRef::new(&bytes).unwrap();
        let cmap = font.cmap().unwrap();
        let records: Vec<(PlatformId, u16)> = cmap
            .encoding_records()
            .iter()
            .map(|r| (r.platform_id(), r.encoding_id()))
            .collect();
        assert_eq!(
            records,
            [
                (PlatformId::Unicode, 3),
                (PlatformId::Unicode, 4),
                (PlatformId::Windows, 1),
                (PlatformId::Windows, 10)
            ]
        );
        let charmap = font.charmap();
        for (i, &c) in chars.iter().enumerate() {
            assert_eq!(charmap.map(c), Some(GlyphId::new(i as u32 + 1)));
            assert_eq!(charmap.map(char::from_u32(u32::from(c) + 1).unwrap()), None);
        }
        // Characters in the BMP alone, in few runs, need only format 4.
        let few = opentype("Few", &glyphs[..100], METRICS);
        let few = FontRef::new(&few).unwrap();
        assert_eq!(few.cmap().unwrap().encoding_records().len(), 2);
        // Format 4's last segment maps U+FFFF to .notdef.
        assert_eq!(few.charmap().map('\u{FFFF}'), None);
    }

    #[test]
    fn postscript_names_keep_ascii_letters_digits_and_hyphens() {
        assert_eq!(
            postscript_name("ABCDEF+Times New Roman,Bold"),
            "ABCDEFTimesNewRomanBold"
        );
        assert_eq!(postscript_name("Source-Serif 4"), "Source-Serif4");
        assert_eq!(postscript_name(""), "Preview");
        assert_eq!(postscript_name("明朝"), "Preview");
        assert_eq!(postscript_name(&"x".repeat(100)), "x".repeat(63));
        // The family keeps the name as given.
        let bytes = opentype("Åsa 明朝", &[], METRICS);
        let font = FontRef::new(&bytes).unwrap();
        let name = |id| {
            font.localized_strings(id)
                .english_or_first()
                .unwrap()
                .to_string()
        };
        assert_eq!(name(StringId::FAMILY_NAME), "Åsa 明朝");
        assert_eq!(name(StringId::POSTSCRIPT_NAME), "sa");
    }
}
