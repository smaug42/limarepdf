//! Drawing form fields' appearance streams: the small drawings that show a field's value.
//! This module lays out and draws; `form_write` picks the font and puts the result in
//! the file, the text written by a pen made for the lines laid out (`layout.rs`).

use crate::fonts::{winansi_byte, winansi_bytes};
use crate::layout::{break_lines, break_ranges, shown, Setter};
use crate::FontChoice;
use lopdf::content::{Content, Operation};
use lopdf::{Object, StringFormat};
use std::ops::Range;

/// Largest and smallest size picked for text whose size is "auto" (`0 Tf`).
const AUTO_MAX: f32 = 12.0;
const AUTO_MIN: f32 = 4.0;
/// Space between the border and the text, in points.
const PAD: f32 = 2.0;
/// Line height as a multiple of the font size.
const LEADING: f32 = 1.15;
/// The highlight behind chosen options in a list box (Acrobat's colour).
const HIGHLIGHT: [f32; 3] = [0.6, 0.757, 0.855];

/// How text is measured in the appearance's font.
pub(crate) enum Metrics {
    /// A font with one byte per character (WinAnsi, or ZapfDingbats' own codes): its
    /// widths by code, in 1/1000 em. Written as it is: the field's own font, kept.
    Bytes(Box<[f32; 256]>),
    /// Set by the text pipeline: a standard font or its embedded face, or an installed
    /// family's, and the fonts it falls back to.
    Set(Setter),
}

impl Metrics {
    /// Width of text written in one-byte codes at `size`.
    fn byte_width(&self, bytes: &[u8], size: f32) -> f32 {
        match self {
            Metrics::Bytes(w) => bytes.iter().map(|&b| w[b as usize]).sum::<f32>() * size / 1000.0,
            Metrics::Set(_) => 0.0,
        }
    }

    pub(crate) fn text_width(&self, s: &str, size: f32) -> f32 {
        match self {
            Metrics::Bytes(_) => self.byte_width(&winansi_bytes(s), size),
            Metrics::Set(set) => set.width(s, size),
        }
    }

    /// `text` in lines no wider than `max` at `size`, broken as every wrapper breaks them
    /// (`layout.rs`).
    pub(crate) fn lines(&self, text: &str, max: f32, size: f32) -> Vec<String> {
        break_lines(text, max, |l| self.text_width(l, size))
    }
}

/// A field's own font, the one its `/DA` names, as [`Metrics::Bytes`] writes in it: its
/// widths by code, whether its encoding is WinAnsi, and whether LimarePDF can write in it.
#[derive(Clone, Debug)]
pub(crate) struct OwnFont {
    pub widths: Box<[f32; 256]>,
    pub winansi: bool,
    pub reusable: bool,
}

impl OwnFont {
    /// Whether `text` is written in it as it is: each character has a code in it, with a
    /// width. A zero width is how some makers (LibreOffice, for one) mark a character the
    /// font in the file doesn't have.
    pub(crate) fn takes(&self, text: &str) -> bool {
        self.reusable
            && (text.is_ascii() || self.winansi)
            && (text.chars().filter(|c| !c.is_control())).all(|c| {
                winansi_byte(c).is_some_and(|b| b <= b' ' || self.widths[b as usize] > 0.0)
            })
    }
}

/// How text is set in a box LimarePDF writes it in, a text box's or a multi-line form
/// field's ([`BoxSetting::text_box`], [`crate::Pdf::field_setting`]): to lay text out while
/// it is typed as it will be written.
#[derive(Clone, Debug)]
pub struct BoxSetting {
    look: Look,
    /// A field's own font, which writes the text when it takes it.
    own: Option<OwnFont>,
    /// The font the text is set in otherwise, falling back where it lacks a character.
    font: FontChoice,
}

impl BoxSetting {
    /// A text box `width` points wide as its text reads, with a border `border` points
    /// wide, its text `size` points: set in Helvetica, as text boxes are.
    pub fn text_box(width: f32, border: f32, size: f32) -> Self {
        let border = (border > 0.0).then(|| Border {
            color: vec![0.0],
            width: border,
            style: b'S',
        });
        let look = Look {
            w: width,
            h: f32::INFINITY,
            bg: None,
            border,
            font: Vec::new(),
            size,
            color: None,
            q: 0,
        };
        Self {
            look,
            own: None,
            font: FontChoice::default(),
        }
    }

    /// A field's widget that looks as `look` says, its text written in `own` when that takes
    /// it, else set in `font`.
    pub(crate) fn field(look: Look, own: Option<OwnFont>, font: FontChoice) -> Self {
        Self { look, own, font }
    }

    /// `text` as it is written in the box, in a document whose language is `lang` (its
    /// `/Lang`, [`crate::Pdf::language`]): its size in points (a field fitting its text to
    /// its box gets the size that fits) and each line's bytes in `text`, the spaces it ends
    /// with among them, a line break between lines not.
    pub fn lines(&self, text: &str, lang: Option<&str>) -> (f32, Vec<Range<usize>>) {
        let m = match &self.own {
            Some(own) if own.takes(text) => Metrics::Bytes(own.widths.clone()),
            _ => Metrics::Set(Setter::new(self.font, text, lang)),
        };
        wrapped(&self.look, text, &m)
    }

    /// How far in from the box's sides and top its text is, in points: its border and the
    /// padding inside it. The first line's baseline is 0.8 times the size below that.
    pub fn margin(&self) -> f32 {
        self.look.inset() + PAD
    }

    /// How far apart its lines' baselines are, as a multiple of its text's size.
    pub fn leading(&self) -> f32 {
        LEADING
    }
}

/// An appearance as [`draw`] lays it out: everything but its text's lines, which are
/// written with the appearance's font ([`Drawn::content`]).
pub(crate) struct Drawn {
    /// What goes before the text: the background, the border, and the text's clip.
    before: Vec<Operation>,
    /// The text's lines: what each shows, and where its baseline starts.
    pub lines: Vec<(String, f32, f32)>,
    /// The text's size.
    pub size: f32,
    /// Its font's name in the resources, and its colour.
    font: Vec<u8>,
    color: Option<Operation>,
    /// Whether it shows text (a mark, or only the background and border, don't).
    text: bool,
}

impl Drawn {
    /// The appearance's content, each line written by `write` (from its number and text,
    /// with the font, size and colour set and the text matrix at its start).
    pub(crate) fn content(&self, write: impl Fn(usize, &str) -> Vec<Operation>) -> Vec<u8> {
        let mut ops = self.before.clone();
        if !self.text {
            return encode(ops);
        }
        let font = Object::Name(self.font.clone());
        ops.push(op("BT", vec![]));
        ops.push(op("Tf", vec![font, nums(&[self.size])[0].clone()]));
        ops.extend(self.color.clone());
        for (i, (s, x, y)) in self.lines.iter().enumerate() {
            ops.push(op("Tm", nums(&[1.0, 0.0, 0.0, 1.0, *x, *y])));
            ops.extend(write(i, s));
        }
        ops.push(op("ET", vec![]));
        ops.push(op("Q", vec![]));
        ops.push(op("EMC", vec![]));
        encode(ops)
    }

    /// [`Drawn::content`] with each line in one-byte codes, as [`Metrics::Bytes`] writes.
    pub(crate) fn bytes_content(&self) -> Vec<u8> {
        self.content(|_, s| vec![op("Tj", vec![bytes_obj(winansi_bytes(s))])])
    }
}

/// A widget's border.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Border {
    pub color: Vec<f32>,
    pub width: f32,
    /// The `/BS /S` style: b'S' solid, b'D' dashed, b'B' beveled, b'I' inset,
    /// b'U' underline. Beveled and inset are drawn solid, with their wider padding.
    pub style: u8,
}

/// How a widget looks, apart from its value.
#[derive(Clone, Debug)]
pub(crate) struct Look {
    /// The appearance's width and height (a widget turned by 90 or 270 degrees has its
    /// rectangle's sides swapped).
    pub w: f32,
    pub h: f32,
    pub bg: Option<Vec<f32>>,
    pub border: Option<Border>,
    /// The font's name in the appearance's resources, and its size; 0 means "auto".
    pub font: Vec<u8>,
    pub size: f32,
    /// The text colour: a `g`, `rg` or `k` operation, from the field's `/DA`.
    pub color: Option<Operation>,
    /// Alignment: 0 left, 1 centred, 2 right.
    pub q: i64,
}

/// What a widget shows.
pub(crate) enum Body<'a> {
    /// One line: a single-line text field, or a drop-down's choice.
    Line(&'a str),
    /// A multi-line text field, wrapped to the width.
    Lines(&'a str),
    /// A comb field: one character in each of `n` equal cells.
    Comb(&'a str, usize),
    /// A list box: its options' text from `top`, with the `chosen` ones highlighted.
    List {
        items: &'a [String],
        chosen: &'a [usize],
        top: usize,
    },
    /// A checkbox or radio button's mark: one ZapfDingbats character.
    Mark(u8),
    /// Only the background and border.
    Empty,
}

impl Look {
    /// The space taken by the border on each side.
    fn inset(&self) -> f32 {
        match &self.border {
            Some(b) if matches!(b.style, b'B' | b'I') => 2.0 * b.width,
            Some(b) => b.width,
            None => 0.0,
        }
    }
}

/// The appearance for `body`, laid out: its text's lines are written by [`Drawn::content`].
pub(crate) fn draw(look: &Look, body: &Body, m: &Metrics) -> Drawn {
    let mut ops = Vec::new();
    let (w, h) = (look.w, look.h);
    if let Some(bg) = look.bg.as_ref().and_then(|c| fill_color(c)) {
        ops.push(op("q", vec![]));
        ops.push(bg);
        ops.push(op("re", nums(&[0.0, 0.0, w, h])));
        ops.push(op("f", vec![]));
        ops.push(op("Q", vec![]));
    }
    if let Some(b) = look.border.as_ref().filter(|b| b.width > 0.0) {
        if let Some(c) = stroke_color(&b.color) {
            let bw = b.width;
            ops.push(op("q", vec![]));
            ops.push(c);
            ops.push(op("w", nums(&[bw])));
            if b.style == b'D' {
                ops.push(op("d", vec![vec![Object::Integer(3)].into(), 0.into()]));
            }
            if b.style == b'U' {
                ops.push(op("m", nums(&[0.0, bw / 2.0])));
                ops.push(op("l", nums(&[w, bw / 2.0])));
            } else {
                ops.push(op("re", nums(&[bw / 2.0, bw / 2.0, w - bw, h - bw])));
            }
            ops.push(op("S", vec![]));
            ops.push(op("Q", vec![]));
        }
    }
    let alone = |before| Drawn {
        before,
        lines: Vec::new(),
        size: 0.0,
        font: look.font.clone(),
        color: None,
        text: false,
    };
    if matches!(body, Body::Empty) {
        return alone(ops);
    }
    let inset = look.inset();
    let (iw, ih) = ((w - 2.0 * inset).max(0.0), (h - 2.0 * inset).max(0.0));
    if let Body::Mark(c) = body {
        let size = if look.size > 0.0 {
            look.size
        } else {
            0.8 * iw.min(ih)
        };
        let x = (w - m.byte_width(&[*c], size)) / 2.0;
        // ZapfDingbats marks stand about 0.7 em tall on the baseline.
        let y = (h - 0.7 * size) / 2.0;
        ops.push(op("q", vec![]));
        ops.push(op("BT", vec![]));
        ops.push(font_op(look, size));
        ops.extend(look.color.clone());
        ops.push(op("Td", nums(&[x, y])));
        ops.push(op("Tj", vec![bytes_obj(vec![*c])]));
        ops.push(op("ET", vec![]));
        ops.push(op("Q", vec![]));
        return alone(ops);
    }
    ops.push(op("BMC", vec![Object::Name(b"Tx".to_vec())]));
    ops.push(op("q", vec![]));
    ops.push(op("re", nums(&[inset, inset, iw, ih])));
    ops.push(op("W", vec![]));
    ops.push(op("n", vec![]));
    let avail = iw - 2.0 * PAD;
    // Each line: its text and its baseline's x and y.
    let mut lines: Vec<(String, f32, f32)> = Vec::new();
    let size;
    match body {
        Body::Line(text) => {
            size = auto(look.size, || line_size(ih, avail, m.text_width(text, 1.0)));
            let x = align(look.q, inset + PAD, avail, m.text_width(text, size));
            lines.push((text.to_string(), x, centred_baseline(h, size)));
        }
        Body::Comb(text, n) => {
            let cell = iw / (*n).max(1) as f32;
            let widest = text.chars().map(|c| m.text_width(&c.to_string(), 1.0));
            let widest = widest.fold(0.0, f32::max);
            size = auto(look.size, || line_size(ih, cell, widest));
            let y = centred_baseline(h, size);
            for (i, c) in text.chars().take(*n).enumerate() {
                let c = c.to_string();
                let x = inset + i as f32 * cell + (cell - m.text_width(&c, size)) / 2.0;
                lines.push((c, x, y));
            }
        }
        Body::Lines(text) => {
            let laid;
            (size, laid) = wrapped(look, text, m);
            // The first baseline sits one ascent (about 0.8 em) below the padding.
            let mut y = h - inset - PAD - 0.8 * size;
            for l in laid.into_iter().map(|line| shown(&text[line])) {
                let x = align(look.q, inset + PAD, avail, m.text_width(&l, size));
                lines.push((l, x, y));
                y -= LEADING * size;
            }
        }
        Body::List { items, chosen, top } => {
            size = auto(look.size, || AUTO_MAX);
            let lh = LEADING * size;
            let mut row_top = h - inset;
            for (i, item) in items.iter().enumerate().skip(*top) {
                if row_top < inset {
                    break;
                }
                if chosen.contains(&i) {
                    ops.push(op("q", vec![]));
                    ops.push(fill_color(&HIGHLIGHT).unwrap());
                    ops.push(op("re", nums(&[inset, row_top - lh, iw, lh])));
                    ops.push(op("f", vec![]));
                    ops.push(op("Q", vec![]));
                }
                let x = align(look.q, inset + PAD, avail, m.text_width(item, size));
                lines.push((item.clone(), x, centred_baseline(lh, size) + row_top - lh));
                row_top -= lh;
            }
        }
        Body::Mark(_) | Body::Empty => unreachable!(),
    }
    Drawn {
        before: ops,
        lines,
        size,
        font: look.font.clone(),
        color: look.color.clone(),
        text: true,
    }
}

/// A multi-line text's size in `look` (its own, or when it asks for "auto" the largest from
/// 12 points down by half a point whose lines fit the box's height) and each of its lines'
/// bytes in `text`, as [`draw`] lays it out.
fn wrapped(look: &Look, text: &str, m: &Metrics) -> (f32, Vec<Range<usize>>) {
    let inset = look.inset();
    let iw = (look.w - 2.0 * inset).max(0.0);
    let ih = (look.h - 2.0 * inset).max(0.0);
    let lines = |size: f32| break_ranges(text, iw - 2.0 * PAD, |l| m.text_width(l, size));
    let size = auto(look.size, || {
        let mut s = AUTO_MAX;
        while s > AUTO_MIN && lines(s).len() as f32 * LEADING * s > ih - 2.0 * PAD {
            s -= 0.5;
        }
        s
    });
    (size, lines(size))
}

/// The size to use: `size`, or the computed one when it is 0 (auto).
fn auto(size: f32, f: impl FnOnce() -> f32) -> f32 {
    if size > 0.0 {
        size
    } else {
        f().clamp(AUTO_MIN, AUTO_MAX)
    }
}

/// An auto size for one line: as tall as the box allows, but no wider than `avail`.
/// `units` is the text's width at size 1.
fn line_size(ih: f32, avail: f32, units: f32) -> f32 {
    let by_height = (ih - 2.0) / LEADING;
    if units > 0.0 {
        by_height.min(avail / units)
    } else {
        by_height
    }
}

/// The baseline that centres text of `size` in a box of height `h`.
fn centred_baseline(h: f32, size: f32) -> f32 {
    (h - size) / 2.0 + 0.22 * size
}

/// The x where text of width `tw` starts, aligned in `avail` from `x0`.
fn align(q: i64, x0: f32, avail: f32, tw: f32) -> f32 {
    match q {
        1 => x0 + (avail - tw) / 2.0,
        2 => x0 + avail - tw,
        _ => x0,
    }
}

/// The height a box `w` wide, with a solid border `border` wide, needs to show all of
/// `text` in lines at `size`, as [`draw`] lays them out.
pub(crate) fn lines_height(text: &str, w: f32, size: f32, border: f32, m: &Metrics) -> f32 {
    let avail = (w - 2.0 * border).max(0.0) - 2.0 * PAD;
    let n = m.lines(text, avail, size).len().max(1) as f32;
    // The first line's ascent, the lines below it, and the last line's descent.
    2.0 * (border + PAD) + 0.8 * size + (n - 1.0) * LEADING * size + 0.25 * size
}

pub(crate) fn op(name: &str, operands: Vec<Object>) -> Operation {
    Operation::new(name, operands)
}

pub(crate) fn nums(v: &[f32]) -> Vec<Object> {
    v.iter()
        .map(|&x| Object::Real((x * 1000.0).round() / 1000.0))
        .collect()
}

fn bytes_obj(b: Vec<u8>) -> Object {
    Object::String(b, StringFormat::Literal)
}

fn font_op(look: &Look, size: f32) -> Operation {
    op(
        "Tf",
        vec![Object::Name(look.font.clone()), nums(&[size])[0].clone()],
    )
}

/// A fill colour operation for a colour array of 1 (grey), 3 (RGB) or 4 (CMYK) numbers.
pub(crate) fn fill_color(c: &[f32]) -> Option<Operation> {
    let name = match c.len() {
        1 => "g",
        3 => "rg",
        4 => "k",
        _ => return None,
    };
    Some(op(name, nums(c)))
}

pub(crate) fn stroke_color(c: &[f32]) -> Option<Operation> {
    let name = match c.len() {
        1 => "G",
        3 => "RG",
        4 => "K",
        _ => return None,
    };
    Some(op(name, nums(c)))
}

pub(crate) fn encode(ops: Vec<Operation>) -> Vec<u8> {
    Content { operations: ops }.encode().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character 500 units wide.
    fn even() -> Metrics {
        Metrics::Bytes(Box::new([500.0; 256]))
    }

    fn look(w: f32, h: f32, size: f32) -> Look {
        Look {
            w,
            h,
            bg: None,
            border: None,
            font: b"Helv".to_vec(),
            size,
            color: None,
            q: 0,
        }
    }

    /// The text-showing operations of a content stream: (text, x, y, size).
    fn shown(content: &[u8]) -> Vec<(String, f32, f32, f32)> {
        let ops = Content::decode(content).unwrap().operations;
        let mut size = 0.0;
        let (mut x, mut y) = (0.0, 0.0);
        let mut out = Vec::new();
        for o in ops {
            let f = |i: usize| o.operands[i].as_float().unwrap();
            match o.operator.as_str() {
                "Tf" => size = f(1),
                "Tm" => (x, y) = (f(4), f(5)),
                "Td" => (x, y) = (f(0), f(1)),
                "Tj" => {
                    let s = String::from_utf8_lossy(o.operands[0].as_str().unwrap()).into_owned();
                    out.push((s, x, y, size));
                }
                _ => {}
            }
        }
        out
    }

    /// The content for `body`, its text in one-byte codes.
    fn drawn(look: &Look, body: &Body, m: &Metrics) -> Vec<u8> {
        draw(look, body, m).bytes_content()
    }

    #[test]
    fn wraps_at_spaces_and_breaks_long_words() {
        // At size 10 each character is 5 points: 20 points take 4 characters.
        let m = even();
        assert_eq!(m.lines("ab cd ef", 25.0, 10.0), ["ab cd", "ef"]);
        assert_eq!(m.lines("abcdefghij", 20.0, 10.0), ["abcd", "efgh", "ij"]);
        assert_eq!(m.lines("a\r\n\nb", 20.0, 10.0), ["a", "", "b"]);
        assert_eq!(m.lines("", 20.0, 10.0), [""]);
        // After a hyphen, as Unicode's line-breaking rule allows.
        assert_eq!(m.lines("well-known", 25.0, 10.0), ["well-", "known"]);
    }

    #[test]
    fn auto_size_fits_the_box() {
        let m = even();
        // Tall box, short text: capped at 12.
        let s = shown(&drawn(&look(200.0, 40.0, 0.0), &Body::Line("abc"), &m));
        assert_eq!(s[0].3, 12.0);
        // Long text: shrunk to the width (100 chars × 0.5 em in 196 points ≈ 3.9, so 4).
        let long = "x".repeat(100);
        let s = shown(&drawn(&look(200.0, 40.0, 0.0), &Body::Line(&long), &m));
        assert_eq!(s[0].3, 4.0);
        // A fixed size is kept.
        let s = shown(&drawn(&look(200.0, 40.0, 9.0), &Body::Line(&long), &m));
        assert_eq!(s[0].3, 9.0);
    }

    #[test]
    fn aligns_left_centre_and_right() {
        let m = even();
        let mut l = look(100.0, 20.0, 10.0);
        // "abcd" is 20 points wide; the text area runs from 2 to 98.
        let x = |l: &Look| shown(&drawn(l, &Body::Line("abcd"), &m))[0].1;
        assert_eq!(x(&l), 2.0);
        l.q = 1;
        assert_eq!(x(&l), 40.0);
        l.q = 2;
        assert_eq!(x(&l), 78.0);
    }

    #[test]
    fn comb_puts_one_character_in_each_cell() {
        let m = even();
        let s = shown(&drawn(&look(100.0, 20.0, 10.0), &Body::Comb("abc", 5), &m));
        // Cells are 20 wide; each 5-point character is centred in its own.
        let xs: Vec<f32> = s.iter().map(|t| t.1).collect();
        assert_eq!(xs, [7.5, 27.5, 47.5]);
    }

    #[test]
    fn multiline_text_runs_down_from_the_top() {
        let m = even();
        let s = shown(&drawn(
            &look(100.0, 60.0, 10.0),
            &Body::Lines("one\ntwo"),
            &m,
        ));
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].0, "one");
        assert!(s[0].2 > s[1].2 && s[0].2 < 60.0 - 2.0 - 7.0);
        assert!((s[0].2 - s[1].2 - 11.5).abs() < 0.01);
        // Auto size shrinks until all the lines fit.
        let many = "a\n".repeat(5);
        let s = shown(&drawn(&look(100.0, 60.0, 0.0), &Body::Lines(&many), &m));
        let size = s[0].3;
        assert!(size < 12.0);
        assert!(6.0 * LEADING * size <= 60.0 - 2.0 * PAD);
        // Too many lines for the smallest size: stays at the smallest, and is clipped.
        let lots = "a\n".repeat(40);
        let s = shown(&drawn(&look(100.0, 60.0, 0.0), &Body::Lines(&lots), &m));
        assert_eq!(s[0].3, AUTO_MIN);
    }

    #[test]
    fn list_highlights_the_chosen_option() {
        let m = even();
        let items = ["a".to_string(), "b".to_string(), "c".to_string()];
        let c = drawn(
            &look(100.0, 60.0, 10.0),
            &Body::List {
                items: &items,
                chosen: &[1],
                top: 0,
            },
            &m,
        );
        let ops = Content::decode(&c).unwrap().operations;
        let fills = ops.iter().filter(|o| o.operator == "f").count();
        assert_eq!(fills, 1);
        assert_eq!(shown(&c).len(), 3);
    }

    #[test]
    fn draws_background_border_and_turns_text_into_winansi() {
        let m = even();
        let mut l = look(100.0, 20.0, 10.0);
        l.bg = Some(vec![1.0]);
        l.border = Some(Border {
            color: vec![0.0, 0.0, 1.0],
            width: 1.0,
            style: b'S',
        });
        let c = drawn(&l, &Body::Line("café"), &m);
        let ops = Content::decode(&c).unwrap().operations;
        let names: Vec<&str> = ops.iter().map(|o| o.operator.as_str()).collect();
        assert!(names.starts_with(&["q", "g", "re", "f", "Q", "q", "RG", "w", "re", "S", "Q"]));
        let tj = ops.iter().find(|o| o.operator == "Tj").unwrap();
        assert_eq!(tj.operands[0].as_str().unwrap(), b"caf\xe9");
        // The text starts inside the border and padding.
        assert_eq!(shown(&c)[0].1, 3.0);
    }
}
