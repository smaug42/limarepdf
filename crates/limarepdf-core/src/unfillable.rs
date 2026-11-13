//! Pictures whose stated size their data can't come near filling (PB-81, found by fuzzing).
//! hayro sizes its buffers from a picture's stated width and height, not from its data: a
//! few bytes stating 255,555,555 × 2 pixels make it ask for gigabytes before it finds the
//! data short. In the copies pages are drawn from, such a picture is swapped for a
//! one-pixel picture that can't be decoded, so hayro leaves it out and the renderer reports
//! it as it reports any picture it can't decode ([`crate::render::Missing::Picture`]). The
//! document keeps the picture as it is.
//!
//! So too tiling patterns hayro 0.9 can't tile (PB-11). It draws the pattern's cell into its
//! tile once for each copy the tile overlaps (about the cell's size over the step, each way),
//! and sizes the tile from the step or the area filled, whichever is less, in the pattern's
//! space, where a skew makes that area grow: a cell 100,000 steps wide is drawn ten billion
//! times, and a pattern skewed 80 to 1 on a Letter page asks for 13 GB at 72 dpi and, its
//! tile past the 65,532 pixels a side vello can snap to its tiles, panics at 96. The renderer
//! draws a page with a tiling pattern in passes of at most [`crate::render::PASS_SIDE`] a
//! side, which bounds what a tile can show of the pattern's space in its pixels at every
//! zoom; a pattern whose tile could still be past 65,532 pixels a side, or take more than 16
//! passes' pixels, is swapped for one whose tile draws a picture that can't be decoded,
//! reported the same way.
//!
//! The app takes a snapshot after every edit, and looking for inline images means decoding
//! every content stream, so streams found fine are remembered ([`Checked`]) and only new or
//! changed ones are looked at again.

use crate::matrix::{self as mx, M};
use crate::redact_img::{components, inline_stream};
use crate::render::MAX_REGION_SCALE;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Pictures stating more pixels than this are checked; smaller ones are drawn as they are,
/// however short their data (hayro fills in the rest).
const LARGE: u64 = 16 << 20;
/// A large picture whose data fills less than one part in this many of its stated size is
/// left out.
const PART: u64 = 8;
/// The widest a large fax picture is drawn (PB-11): hayro sizes each row from the width
/// alone, and a fax row of any width can take a few bits.
const MOST_FAX_COLUMNS: u64 = 1 << 17;
/// The most a content stream is decoded to, to look for inline images in it (or to redraw
/// it, [`crate::mesh_fill`]).
pub(crate) const MAX_CONTENT: usize = 1 << 30;
/// The most copies of a tiling pattern's cell drawn into its tile.
const MOST_COPIES: f64 = 10_000.0;
/// The most pixels a side of a tiling pattern's tile may have: past 65,532 the rasteriser
/// panics, less 2 hayro may round up.
const MOST_SIDE: f64 = 65_530.0;
/// The most pixels a tiling pattern's tile may have: 256 MiB of them, 16 of the renderer's
/// passes. Unskewed or turned a quarter, a tile is at most one; turned an eighth, two; skewed
/// 2 to 1, about 7.
const MOST_TILE: f64 = (1u64 << 26) as f64;
/// The most pixels a side of one of the renderer's passes on a page with a tiling pattern.
const PASS: f64 = crate::render::PASS_SIDE as f64;
/// Where a page is drawn: its own space, on one of the renderer's passes.
const ON_PAGE: Place = Place {
    base: mx::ID,
    pixels: (PASS, PASS),
};

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

/// Swap, in `doc`, every picture its data can't fill for one that can't be decoded, and
/// every tiling pattern hayro can't tile for one drawing such a picture (see the module
/// notes).
pub(crate) fn leave_out(doc: &mut Document) {
    let swaps = swaps(doc, &mut Checked::default());
    put(doc, swaps);
}

/// Put `swaps` ([`swaps`] of `doc` or of what it was copied from) in `doc`.
pub(crate) fn put(doc: &mut Document, swaps: Vec<(ObjectId, Object)>) {
    doc.objects.extend(swaps);
}

/// What [`leave_out`] would put in `doc`: for a picture object or a tiling pattern, the
/// picture or pattern; for a content stream drawing an inline image, the stream (decoded,
/// the picture swapped). With them, what [`crate::mesh_fill`] redraws. Streams in `checked`
/// are passed over, and those found fine added to it.
pub(crate) fn swaps(doc: &Document, checked: &mut Checked) -> Vec<(ObjectId, Object)> {
    let mut out = BTreeMap::new();
    let mut contents = BTreeSet::new();
    let pages: Vec<ObjectId> = crate::doc::page_tree(doc).collect();
    let mut kept = BTreeSet::new();
    for (&id, o) in &doc.objects {
        match o {
            Object::Stream(s) => match s.dict.get(b"Subtype").and_then(Object::as_name) {
                Ok(b"Image")
                    if large(doc, &s.dict) && !checked.fine(id, s, || !unfillable(doc, s)) =>
                {
                    out.insert(id, Object::Stream(broken()));
                }
                Ok(b"Form") => {
                    contents.insert(id);
                }
                _ if matches!(s.dict.get(b"PatternType").and_then(Object::as_i64), Ok(1)) => {
                    if tile(doc, &s.dict, ON_PAGE).is_some_and(|t| t.too_big()) {
                        out.insert(id, Object::Stream(broken_pattern()));
                    } else {
                        kept.insert(id);
                        contents.insert(id);
                    }
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
    if !kept.is_empty() {
        let nested = Nested::new(doc, &kept).walk(pages.iter());
        for id in nested.unwrap_or(kept) {
            contents.remove(&id);
            out.insert(id, Object::Stream(broken_pattern()));
        }
    }
    for page in pages {
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
        out.extend(swap.map(|s| (id, Object::Stream(s))));
    }
    crate::mesh_fill::draw_as_shadings(doc, &mut out);
    out.into_iter().collect()
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

/// What stands in for a tiling pattern left out: one whose one-unit tile draws [`broken`].
fn broken_pattern() -> Stream {
    let unit = || Object::Array(vec![0.into(), 0.into(), 1.into(), 1.into()]);
    Stream::new(
        dictionary! {
            "Type" => "Pattern",
            "PatternType" => 1,
            "PaintType" => 1,
            "TilingType" => 1,
            "BBox" => unit(),
            "XStep" => 1,
            "YStep" => 1,
            "Resources" => dictionary! {},
        },
        BROKEN_INLINE.to_vec(),
    )
}

/// What `key` holds in page `id`, or in the nearest page tree node above it that has it.
fn inherited<'a>(doc: &'a Document, id: ObjectId, key: &[u8]) -> Option<&'a Object> {
    inherited_at(doc, id, key).map(|(_, o)| o)
}

/// [`inherited`], with the page or node holding it.
pub(crate) fn inherited_at<'a>(
    doc: &'a Document,
    id: ObjectId,
    key: &[u8],
) -> Option<(ObjectId, &'a Object)> {
    let mut at = id;
    for _ in 0..64 {
        let d = doc.get_dictionary(at).ok()?;
        if let Ok(o) = d.get(key) {
            return Some((at, o));
        }
        at = d.get(b"Parent").and_then(Object::as_reference).ok()?;
    }
    None
}

/// The most work [`Nested`] does, in drawings looked into and content operators gone
/// through in drawings other than pages; past it every tiling pattern is left out.
const MOST_WORK: u64 = 1 << 24;
/// How deep drawings in drawings are looked into: deeper than hayro draws them (50).
const MOST_DEPTH: usize = 64;

/// A walk of what the pages draw, for [`swaps`]: a tiling pattern in a form, an annotation's
/// appearance, a soft mask or a Type 3 glyph is drawn with their matrices too, as hayro
/// draws it, and they can skew it where its own matrix doesn't; one in another pattern's
/// cell is drawn on that pattern's tile, which can be longer than the page. The walk
/// follows `q`, `Q`, `cm`, `gs`, `Do`, `BT`, `Tm`, `Tf`, `Tz` and the text Type 3 fonts
/// show, and reads a drawing's content only when something it draws holds a tiling pattern.
/// Each pattern is reckoned against the whole of a pass or the tile it is drawn on, though
/// hayro may draw it in less; every state of an appearance, and hidden annotations, are
/// looked into too.
struct Nested<'a> {
    doc: &'a Document,
    /// The tiling patterns to check: those their own matrix doesn't leave out.
    kept: &'a BTreeSet<ObjectId>,
    /// Drawings looked into, by where, and how deep.
    seen: HashMap<(ObjectId, [u32; 4], [u64; 2]), usize>,
    /// Whether a drawing with resources of its own holds a tiling pattern, in them or in
    /// what it draws.
    holds: HashMap<ObjectId, bool>,
    /// The drawings [`Self::holds`] is finding that for, and whether one of them was met in
    /// what it draws, so that what was found there may be short.
    busy: HashSet<ObjectId>,
    cut: bool,
    ops: HashMap<ObjectId, Option<Rc<Vec<Operation>>>>,
    work: u64,
    out: BTreeSet<ObjectId>,
}

/// A graphics state, as far as [`Nested`] needs it: the transform, and the font, its size
/// and its horizontal scale.
#[derive(Clone)]
struct State<'o> {
    ctm: M,
    font: Option<&'o [u8]>,
    size: f32,
    scale: f32,
}

/// Where a drawing is drawn: the matrix from its space to the canvas, in the units of a page
/// at 72 dpi, and the most pixels the canvas (one of the renderer's passes, or a pattern's
/// tile) can have across and up, at any zoom.
#[derive(Clone, Copy)]
struct Place {
    base: M,
    pixels: (f64, f64),
}

impl Place {
    /// Drawing `id` here, by its matrix's scale and skew and the canvas, by their bits.
    fn key(self, id: ObjectId) -> (ObjectId, [u32; 4], [u64; 2]) {
        let [a, b, c, d, ..] = self.base;
        (
            id,
            [a, b, c, d].map(f32::to_bits),
            [self.pixels.0.to_bits(), self.pixels.1.to_bits()],
        )
    }

    /// A space drawn from this one by matrix `m`.
    fn then(self, m: M) -> Place {
        Place {
            base: mx::mul(m, self.base),
            ..self
        }
    }
}

impl<'a> Nested<'a> {
    fn new(doc: &'a Document, kept: &'a BTreeSet<ObjectId>) -> Self {
        Nested {
            doc,
            kept,
            seen: HashMap::new(),
            holds: HashMap::new(),
            busy: HashSet::new(),
            cut: false,
            ops: HashMap::new(),
            work: 0,
            out: BTreeSet::new(),
        }
    }

    /// Whether drawing `id` is reached at `at` for the first time, or less deep than before.
    fn first(&mut self, id: ObjectId, at: Place, depth: usize) -> bool {
        let key = at.key(id);
        if self.seen.get(&key).is_some_and(|&d| d <= depth) {
            return false;
        }
        self.seen.insert(key, depth);
        true
    }

    /// The tiling patterns hayro can't tile as `pages` and their annotations draw them;
    /// `None` when the walk ran out of work.
    fn walk<'p>(mut self, pages: impl Iterator<Item = &'p ObjectId>) -> Option<BTreeSet<ObjectId>> {
        for &id in pages {
            let res = self.dict(inherited(self.doc, id, b"Resources"));
            self.look(id, true, res, ON_PAGE, 0);
            let annots = (self.doc.get_dictionary(id).ok())
                .and_then(|p| self.doc.dereference(p.get(b"Annots").ok()?).ok())
                .and_then(|(_, a)| a.as_array().ok());
            for a in annots.into_iter().flatten() {
                let Some(a) = self.dict(Some(a)) else {
                    continue;
                };
                self.appearance(a, res);
            }
        }
        (self.work <= MOST_WORK).then_some(self.out)
    }

    fn dict(&self, o: Option<&'a Object>) -> Option<&'a Dictionary> {
        self.doc.dereference(o?).ok()?.1.as_dict().ok()
    }

    fn matrix(&self, d: &Dictionary) -> M {
        let m = d.get(b"Matrix").ok().and_then(|m| numbers(self.doc, m));
        match m.as_deref() {
            Some(&[a, b, c, d, e, f]) => [a, b, c, d, e, f].map(|n| n as f32),
            _ => mx::ID,
        }
    }

    /// The tiling pattern `o` refers to.
    fn tiling(&self, o: &Object) -> Option<(ObjectId, &'a Stream)> {
        let id = o.as_reference().ok()?;
        let s = self.doc.get_object(id).and_then(Object::as_stream).ok()?;
        let tiling = matches!(s.dict.get(b"PatternType").and_then(Object::as_i64), Ok(1));
        tiling.then_some((id, s))
    }

    /// The forms, Type 3 fonts and soft masks resources `res` give that have resources of
    /// their own, with them (a Type 3 font that isn't an object of its own without an id).
    fn drawings(&self, res: &'a Dictionary) -> Vec<(Option<ObjectId>, &'a Dictionary)> {
        let mut out = Vec::new();
        let entries = |key: &[u8]| {
            self.dict(res.get(key).ok())
                .into_iter()
                .flat_map(|d| d.iter())
        };
        for (_, o) in entries(b"XObject") {
            out.extend(self.form(o).and_then(|id| Some((Some(id), self.own(id)?))));
        }
        for (_, o) in entries(b"Font") {
            let own = self
                .type3(o)
                .and_then(|f| self.dict(f.get(b"Resources").ok()));
            out.extend(own.map(|own| (o.as_reference().ok(), own)));
        }
        for (_, o) in entries(b"ExtGState") {
            out.extend(
                self.soft_mask(o)
                    .and_then(|id| Some((Some(id), self.own(id)?))),
            );
        }
        out
    }

    /// The stream `o` refers to.
    fn stream(&self, o: &Object) -> Option<(ObjectId, &'a Stream)> {
        let id = o.as_reference().ok()?;
        Some((
            id,
            self.doc.get_object(id).and_then(Object::as_stream).ok()?,
        ))
    }

    /// The form `o` refers to.
    fn form(&self, o: &Object) -> Option<ObjectId> {
        let (id, s) = self.stream(o)?;
        let form = matches!(
            s.dict.get(b"Subtype").and_then(Object::as_name),
            Ok(b"Form")
        );
        form.then_some(id)
    }

    /// The Type 3 font `o` is or refers to.
    fn type3(&self, o: &'a Object) -> Option<&'a Dictionary> {
        let f = self.dict(Some(o))?;
        matches!(f.get(b"Subtype").and_then(Object::as_name), Ok(b"Type3")).then_some(f)
    }

    /// The soft mask's group in graphics state `o`: hayro draws any stream there as a form.
    fn soft_mask(&self, o: &'a Object) -> Option<ObjectId> {
        let mask = self.dict(self.dict(Some(o))?.get(b"SMask").ok())?;
        Some(self.stream(mask.get(b"G").ok()?)?.0)
    }

    /// Stream `id`'s own resources.
    fn own(&self, id: ObjectId) -> Option<&'a Dictionary> {
        let s = self.doc.get_object(id).and_then(Object::as_stream).ok()?;
        self.dict(s.dict.get(b"Resources").ok())
    }

    /// Whether `res` hold a tiling pattern, or something they give draws one.
    fn holds(&mut self, res: Option<&'a Dictionary>, depth: usize) -> bool {
        let Some(res) = res else { return false };
        let patterns = self.dict(res.get(b"Pattern").ok());
        patterns.is_some_and(|p| p.iter().any(|(_, o)| self.tiling(o).is_some()))
            || self.deeper(res, depth)
    }

    /// Whether a form, Type 3 font or soft mask `res` give draws a tiling pattern. One
    /// without resources of its own draws with `res`, whose patterns [`Self::holds`] counts.
    /// Too deep or past the work to find out, as if one did: the walk stops there anyway.
    fn deeper(&mut self, res: &'a Dictionary, depth: usize) -> bool {
        if depth >= MOST_DEPTH || self.work > MOST_WORK {
            return true;
        }
        self.drawings(res).into_iter().any(|(id, own)| {
            self.work += 1;
            let Some(id) = id else {
                return self.holds(Some(own), depth + 1);
            };
            if let Some(&h) = self.holds.get(&id) {
                return h;
            }
            if !self.busy.insert(id) {
                // Drawn in itself: what it holds is being found.
                self.cut = true;
                return false;
            }
            let outer = std::mem::take(&mut self.cut);
            let h = self.holds(Some(own), depth + 1);
            self.busy.remove(&id);
            if h || !self.cut {
                self.holds.insert(id, h);
            }
            self.cut |= outer;
            h
        })
    }

    /// Drawing `id`'s content operators (a page's when `page`), without inline images.
    fn ops(&mut self, id: ObjectId, page: bool) -> Option<Rc<Vec<Operation>>> {
        if let Some(ops) = self.ops.get(&id) {
            return ops.clone();
        }
        let data = if page {
            self.doc.get_page_content_with_limit(id, MAX_CONTENT).ok()
        } else {
            (self.doc.get_object(id).and_then(Object::as_stream))
                .and_then(|s| s.get_plain_content_with_limit(MAX_CONTENT))
                .ok()
        };
        let ops = data.and_then(|d| {
            let (d, _) = crate::inline::extract(&d).ok()?;
            Some(Rc::new(Content::decode(&d).ok()?.operations))
        });
        self.ops.insert(id, ops.clone());
        ops
    }

    /// Look into drawing `id` (a page when `page`), drawn with resources `res` at `at`: its
    /// tiling patterns checked and their cells looked into, and, when something it draws
    /// holds a tiling pattern, its content.
    fn look(
        &mut self,
        id: ObjectId,
        page: bool,
        res: Option<&'a Dictionary>,
        at: Place,
        depth: usize,
    ) {
        self.work += 1;
        if depth > MOST_DEPTH || self.work > MOST_WORK || !self.first(id, at, depth) {
            return;
        }
        let Some(res) = res else { return };
        let patterns = self.dict(res.get(b"Pattern").ok());
        for (_, o) in patterns.into_iter().flat_map(|p| p.iter()) {
            let Some((p, s)) = self.tiling(o).filter(|(p, _)| self.kept.contains(p)) else {
                continue;
            };
            let Some(t) = tile(self.doc, &s.dict, at) else {
                continue;
            };
            if t.too_big() {
                self.out.insert(p);
                continue;
            }
            let own = self.dict(s.dict.get(b"Resources").ok());
            if self.holds(own, depth + 1) {
                // hayro draws the cell on the tile, by the pattern's lengths across and up.
                let (sx, sy) = (t.scale.0 as f32, t.scale.1 as f32);
                let cell = Place {
                    base: [sx, 0.0, 0.0, sy, 0.0, 0.0],
                    pixels: t.sides,
                };
                self.look(p, false, own, cell, depth + 1);
            }
        }
        if !self.deeper(res, depth) {
            return;
        }
        let Some(ops) = self.ops(id, page) else {
            return;
        };
        if !page {
            self.work += ops.len() as u64;
        }
        let [xobjects, states, fonts] =
            [&b"XObject"[..], b"ExtGState", b"Font"].map(|key| self.dict(res.get(key).ok()));
        let entry = |d: Option<&'a Dictionary>, name: &[u8]| d?.get(name).ok();
        let mut g = State {
            ctm: mx::ID,
            font: None,
            size: 0.0,
            scale: 1.0,
        };
        let (mut stack, mut tm) = (Vec::new(), mx::ID);
        for op in ops.iter() {
            let nums: Vec<f32> = op
                .operands
                .iter()
                .filter_map(|o| o.as_float().ok())
                .collect();
            match (op.operator.as_str(), &nums[..], op.operands.first()) {
                ("q", ..) => stack.push(g.clone()),
                ("Q", ..) => g = stack.pop().unwrap_or(g),
                ("cm", &[a, b, c, d, e, f], _) => g.ctm = mx::mul([a, b, c, d, e, f], g.ctm),
                ("BT", ..) => tm = mx::ID,
                ("Tm", &[a, b, c, d, e, f], _) => tm = [a, b, c, d, e, f],
                ("Tf", _, Some(name)) => {
                    g.font = name.as_name().ok();
                    g.size = op
                        .operands
                        .get(1)
                        .and_then(|o| o.as_float().ok())
                        .unwrap_or(0.0);
                }
                ("Tz", &[s], _) => g.scale = s / 100.0,
                ("Do", _, Some(Object::Name(name))) => {
                    if let Some(form) = entry(xobjects, name).and_then(|o| self.form(o)) {
                        self.draw_form(form, res, at.then(g.ctm), depth);
                    }
                }
                ("gs", _, Some(Object::Name(name))) => {
                    if let Some(group) = entry(states, name).and_then(|o| self.soft_mask(o)) {
                        self.draw_form(group, res, at.then(g.ctm), depth);
                    }
                }
                ("Tj" | "TJ" | "'" | "\"", ..) => {
                    let Some(o) = g.font.and_then(|f| entry(fonts, f)) else {
                        continue;
                    };
                    let Some(f) = self.type3(o) else { continue };
                    let fm = f.get(b"FontMatrix").ok().and_then(|m| numbers(self.doc, m));
                    let fm = match fm.as_deref() {
                        Some(&[a, b, c, d, e, f]) => [a, b, c, d, e, f].map(|n| n as f32),
                        _ => [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
                    };
                    let size = [g.size * g.scale, 0.0, 0.0, g.size, 0.0, 0.0];
                    let glyph = at.then(g.ctm).then(tm).then(size).then(fm);
                    // Each glyph is looked into once at each size and slant.
                    if o.as_reference()
                        .is_ok_and(|f| !self.first(f, glyph, depth + 1))
                    {
                        continue;
                    }
                    let own = self.dict(f.get(b"Resources").ok()).or(Some(res));
                    let procs = self.dict(f.get(b"CharProcs").ok());
                    for (_, p) in procs.into_iter().flat_map(|p| p.iter()) {
                        if let Ok(p) = p.as_reference() {
                            self.look(p, false, own, glyph, depth + 1);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Look into form `id` drawn at `at` in a drawing with resources `res`.
    fn draw_form(&mut self, id: ObjectId, res: &'a Dictionary, at: Place, depth: usize) {
        let Ok(s) = self.doc.get_object(id).and_then(Object::as_stream) else {
            return;
        };
        let own = self.dict(s.dict.get(b"Resources").ok()).or(Some(res));
        self.look(id, false, own, at.then(self.matrix(&s.dict)), depth + 1);
    }

    /// Look into annotation `a`'s normal appearances, on a page with resources `res`: each
    /// stream drawn by its matrix, then scaled to the annotation's rectangle (hayro's
    /// `interpret_annotation`).
    fn appearance(&mut self, a: &'a Dictionary, res: Option<&'a Dictionary>) {
        let rect = a.get(b"Rect").ok().and_then(|r| numbers(self.doc, r));
        let Some(&[r0, r1, r2, r3]) = rect.as_deref().and_then(|r| r.get(..4)) else {
            return;
        };
        let Some(normal) = self
            .dict(a.get(b"AP").ok())
            .and_then(|ap| ap.get(b"N").ok())
        else {
            return;
        };
        let states: Vec<&Object> = match (self.stream(normal), self.dict(Some(normal))) {
            (None, Some(states)) => states.iter().map(|(_, s)| s).collect(),
            _ => vec![normal],
        };
        let streams: Vec<_> = states.into_iter().filter_map(|s| self.stream(s)).collect();
        for (id, s) in streams {
            let bbox = s.dict.get(b"BBox").ok().and_then(|b| numbers(self.doc, b));
            let Some(&[b0, b1, b2, b3]) = bbox.as_deref().and_then(|b| b.get(..4)) else {
                continue;
            };
            let m = self.matrix(&s.dict);
            let t = mx::transform_rect(m, [b0, b1, b2, b3].map(|n| n as f32));
            let sx = (r2 - r0).abs() as f32 / (t[2] - t[0]);
            let sy = (r3 - r1).abs() as f32 / (t[3] - t[1]);
            if sx.is_finite() && sy.is_finite() {
                let own = self.dict(s.dict.get(b"Resources").ok()).or(res);
                let base = mx::mul(m, [sx, 0.0, 0.0, sy, 0.0, 0.0]);
                self.look(id, false, own, Place { base, ..ON_PAGE }, 1);
            }
        }
    }
}

/// How hayro would tile a tiling pattern.
struct Tile {
    /// How many times the cell is drawn into the tile.
    copies: f64,
    /// The most pixels the tile can have across and up, at any zoom.
    sides: (f64, f64),
    /// How long a unit of the pattern's space is, across and up, on the tile, in the units
    /// of a page at 72 dpi.
    scale: (f64, f64),
}

impl Tile {
    /// Whether hayro would draw the cell into the tile more than [`MOST_COPIES`] times, or
    /// make a side of the tile more than [`MOST_SIDE`] or the tile more than [`MOST_TILE`],
    /// at some zoom.
    fn too_big(&self) -> bool {
        let (w, h) = self.sides;
        self.copies > MOST_COPIES || w > MOST_SIDE || h > MOST_SIDE || w * h > MOST_TILE
    }
}

/// How hayro would tile tiling pattern `d` drawn at `at`, filling all the canvas, at a zoom
/// of up to [`MAX_REGION_SCALE`]; `None` for a pattern hayro doesn't draw (its steps near 0,
/// its cell without area, its matrix flat).
fn tile(doc: &Document, d: &Dictionary, at: Place) -> Option<Tile> {
    let num = |o: &Object| Some(f64::from(doc.dereference(o).ok()?.1.as_float().ok()?));
    let bbox = numbers(doc, d.get(b"BBox").ok()?).filter(|b| b.len() >= 4)?;
    let (cw, ch) = ((bbox[2] - bbox[0]).abs(), (bbox[3] - bbox[1]).abs());
    let xs = num(d.get(b"XStep").ok()?)?.abs();
    let ys = num(d.get(b"YStep").ok()?)?.abs();
    // hayro's nearly 0.
    let least = 1.0 / 256.0;
    if xs <= least || ys <= least || cw * ch == 0.0 {
        return None;
    }
    let m = d.get(b"Matrix").ok().and_then(|m| numbers(doc, m));
    let [a, b, c, dd] = match m.as_deref() {
        Some(&[a, b, c, dd, _, _]) => [a, b, c, dd],
        _ => [1.0, 0.0, 0.0, 1.0],
    };
    // The pattern's matrix, then the base's.
    let [ba, bb, bc, bd] = [at.base[0], at.base[1], at.base[2], at.base[3]].map(f64::from);
    let [a, b, c, dd] = [
        a * ba + b * bc,
        a * bb + b * bd,
        c * ba + dd * bc,
        c * bb + dd * bd,
    ];
    let det = (a * dd - b * c).abs();
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    // hayro makes the tile a step, or as much of the pattern's space as the canvas shows,
    // whichever is less, each way. At zoom z the canvas shows the pattern's space this far
    // over z: the tile's pixels, those over a unit's length on the canvas, are the same at
    // every zoom, and a step's grow with it.
    let (w, h) = at.pixels;
    let (seen_x, seen_y) = (
        (dd.abs() * w + c.abs() * h) / det,
        (b.abs() * w + a.abs() * h) / det,
    );
    let scale = (a.hypot(b), c.hypot(dd));
    let zoom = f64::from(MAX_REGION_SCALE);
    Some(Tile {
        copies: (cw / xs + 2.0) * (ch / ys + 2.0),
        sides: (
            scale.0 * seen_x.min(xs * zoom),
            scale.1 * seen_y.min(ys * zoom),
        ),
        scale,
    })
}

/// The numbers in array `o`, if it is one of numbers only.
pub(crate) fn numbers(doc: &Document, o: &Object) -> Option<Vec<f64>> {
    let (_, a) = doc.dereference(o).ok()?;
    a.as_array()
        .ok()?
        .iter()
        .map(|n| Some(f64::from(doc.dereference(n).ok()?.1.as_float().ok()?)))
        .collect()
}

pub(crate) fn int(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<i64> {
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
/// JBIG2), or that LimarePDF can't decode, is left to hayro. A fax picture's rows are bounded
/// by its data (each takes at least a bit) but its width isn't, so a large one wider than
/// [`MOST_FAX_COLUMNS`] is left out whatever its data.
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
        let columns = parms.and_then(|p| int(doc, p, b"Columns")).unwrap_or(0);
        if w.max(u64::try_from(columns).unwrap_or(0)) > MOST_FAX_COLUMNS {
            return true;
        }
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
