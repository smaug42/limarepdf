//! Paragraphs on a page (NEW-01; the paragraph design's section 4): the page's lines joined
//! into visual lines across text objects, grouped into blocks top to bottom, and each block
//! confident, edited as a paragraph, or not, with the reason.
//!
//! The rules and their constants are batch 44's prototype's
//! (`target/b44/probe/detect/detector.py`), with the fixes its errors called for: columns
//! matched across the cut's bands, a line split against its own word gaps, emphasis allowed
//! inside a paragraph, the gap's limit following the page's spacing, a number a list label
//! only with a gap after it, and the next word's fit measured against the paragraph's own
//! width. The opt-in check (`tests/paragraph_check.rs`) judges them on the answer sets.
//!
//! Positions are in points on the displayed page (y down); "sizes" are the lines' text
//! sizes.

use crate::objects::{ObjectKind, PageObject, Parsed};
use crate::own_font::{FontRefusal, PageFonts};
use crate::reading::gaps;
use crate::{Pdf, Result};
use lopdf::{Object, ObjectId};
use std::collections::{HashMap, HashSet};

// ---- 1. Regions: an XY cut of the lines' boxes, in median line heights.

/// An empty band across this tall separates blocks of rows (reading order uses 0.8).
const ROW_GAP: f32 = 0.5;
/// An empty strip down this wide separates columns...
const COL_GAP: f32 = 1.0;
/// ...when the text on each side is at least this wide.
const COL_WIDTH: f32 = 10.0;
/// Columns cut in different bands are one column when they overlap by this share of the
/// narrower...
const COL_SAME: f32 = 0.7;
/// ...and their widths differ by at most this share of the wider.
const COL_WIDTHS: f32 = 0.3;

// ---- 2. Visual lines, in sizes (the larger text's).

/// Baselines this close are one line.
const BASE_TOL: f32 = 0.25;
/// A gap along a line wider than this splits it in two...
const SPLIT_GAP: f32 = 1.5;
/// ...when it is also this many times the line's word gap (justified words drawn one by
/// one stand wide apart).
const SPLIT_WORDS: f32 = 2.5;
/// Text at most this size of the line's...
const SUP_SIZE: f32 = 0.85;
/// ...with its baseline this close is a raised or lowered piece of it...
const SUP_BASE: f32 = 0.6;
/// ...when it is at most this many sizes beyond the line's ends.
const SUP_REACH: f32 = 2.5;
/// A first piece of at most this many characters with no letters...
const LABEL_PIECE: usize = 4;
/// ...this many sizes clear of the next piece, is a list label...
const LABEL_GAP: f32 = 0.3;
/// ...and joins text this many sizes to its right.
const LABEL_REACH: f32 = 4.0;

// ---- 3. Joining a visual line to the block above it.

/// Dominant sizes agree within this share.
const SIZE_TOL: f32 = 0.05;
/// The baseline gap, in sizes, at least...
const GAP_MIN: f32 = 1.0;
/// ...and at most this, or the page's own spacing with [`GAP_MED_TOL`] to spare...
const GAP_MAX: f32 = 2.0;
/// ...which counts up to this.
const GAP_MAX_PAGE: f32 = 3.0;
/// The gap within this share of the block's median so far.
const GAP_MED_TOL: f32 = 0.15;
/// The lines overlap across this share of the narrower.
const OVERLAP_MIN: f32 = 0.5;
/// Edges agree within this many points...
const EDGE_PT: f32 = 1.5;
/// ...or this many sizes, whichever is more.
const EDGE_EM: f32 = 0.25;
/// A first-line indent, in sizes, at least...
const INDENT_MIN: f32 = 0.5;
/// ...and at most.
const INDENT_MAX: f32 = 4.0;
/// A list item's hanging indent, in sizes, at most.
const HANG_MAX: f32 = 6.0;
/// Two lines each this much in one font or colour, the two different: a break (a
/// heading's weight, an italic byline); less is emphasis inside a paragraph.
const STYLE_ALL: f32 = 0.8;
/// The next line's first word would have fitted at the end of this one with this many
/// sizes to spare: a hard break.
const FIT_SLACK: f32 = 0.5;
/// A word space, in sizes, for that test.
const SPACE_EM: f32 = 0.25;

// ---- 4. Confidence.

/// Every gap within this share of the block's median.
const CONF_GAP_TOL: f32 = 0.10;
/// Every line's aligned edge within this many points.
const CONF_EDGE: f32 = 1.0;
/// The block at least this many sizes wide.
const CONF_MIN_WIDTH: f32 = 8.0;
/// A line ending a sentence this many sizes short of the block's right edge, the next
/// starting upper-case, may end a paragraph.
const CONF_END_ROOM: f32 = 3.0;
/// The block at least this share of its column's width (cells, captions, side notes are
/// narrower).
const CONF_COL_SHARE: f32 = 0.5;
/// A centred or right-aligned block needs this many lines (two short centred lines are as
/// often two titles as one).
const CONF_CENTRED_LINES: usize = 3;
/// Type this many times the page's body size is a heading's.
const CONF_BIG: f32 = 1.6;
/// A break on the gap is clear when it is this share off the median.
const STRONG_GAP_DEV: f32 = 0.25;
/// A break on "the next word fits" is clear with this many sizes to spare.
const STRONG_FIT: f32 = 3.0;
/// Other objects and text keep this far out of the frame...
const OBJ_INSET: f32 = 0.5;
/// ...but a shape thinner than this that only touches its top or bottom (a rule) is
/// outside it.
const OBJ_THIN: f32 = 3.0;
/// A region where this share of visual lines have another on their baseline is a table or
/// a form.
const GRID_ROWMATES: f32 = 0.3;

/// A block of a page's text: a paragraph, or lines LimarePDF isn't sure are one.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// Its visual lines, top to bottom.
    pub lines: Vec<VisualLine>,
    pub frame: Frame,
    /// Why it isn't edited as a paragraph by default, or `None` when it is (confident).
    pub doubt: Option<Doubt>,
}

impl Block {
    /// Whether LimarePDF is sure enough to edit the block as a paragraph by default.
    pub fn confident(&self) -> bool {
        self.doubt.is_none()
    }

    /// The page's text objects (ids as in [`Pdf::page_objects`]) the block is made of, line
    /// by line.
    pub fn pieces(&self) -> impl Iterator<Item = usize> + '_ {
        self.lines.iter().flat_map(|l| l.pieces.iter().copied())
    }
}

/// A line as it reads on the page, drawn in one or more text objects.
#[derive(Clone, Debug, PartialEq)]
pub struct VisualLine {
    /// Its text objects (ids as in [`Pdf::page_objects`]), in content order.
    pub pieces: Vec<usize>,
    pub bbox: [f32; 4],
    /// Where its baseline is down the page.
    pub baseline: f32,
    /// Its text's size, the most of its characters'.
    pub size: f32,
}

/// Where a block's text sits: what a paragraph is rewrapped in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The left edge its lines share (the lines after the first).
    pub left: f32,
    /// The right edge: the justified edge, or the longest line's end.
    pub right: f32,
    /// The first line's baseline.
    pub baseline: f32,
    /// From one baseline to the next (the median); 0 for one line.
    pub spacing: f32,
    pub align: Align,
    /// How far the first line starts right of `left` (left of it when below 0, a hanging
    /// indent).
    pub indent: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Centre,
    /// Both edges agree on every line but the last.
    Justified,
}

/// Why a block isn't edited as a paragraph by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Doubt {
    /// A line on its own.
    OneLine,
    /// Two centred or right-aligned lines, as often two titles as one paragraph.
    ShortCentred,
    /// A line that doesn't run upright left to right.
    Turned,
    /// In a table or a form (lines side by side, labels, dot leaders).
    Table,
    /// The line above or below may belong with it.
    WeakBreak,
    /// Its lines are spaced unevenly.
    UnevenGaps,
    /// Its lines' edges don't line up.
    UnevenEdges,
    /// Too narrow for its type, or for its column.
    Narrow,
    /// No line reads on into the next (a list of names, say).
    NoReadOn,
    /// A line may end a paragraph within it.
    MayEnd,
    /// Type large enough for a heading.
    LargeType,
    /// Other text in its frame.
    TextInTheWay,
    /// A picture in its frame.
    PictureInTheWay,
    /// A drawn shape in its frame.
    ShapeInTheWay,
    /// Its font sends new text to another face (a Type 3 font, say, or one whose program
    /// can't be used).
    Font,
}

/// What [`Pdf::page_blocks_by`] reads besides the lines, for measuring detection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Detect {
    /// Lines whose marked content belongs to different block elements of the document's
    /// tags are never joined.
    pub tags: bool,
    /// A block whose font can't take new text isn't confident ([`Doubt::Font`]).
    pub fonts: bool,
}

impl Default for Detect {
    fn default() -> Self {
        Self {
            tags: true,
            fonts: true,
        }
    }
}

impl Pdf {
    /// The blocks of text on page `page_index`, read once: each paragraph LimarePDF finds,
    /// confident or not, and the lines on their own. Hidden text, vertical writing and text
    /// in form XObjects is in none.
    pub fn page_blocks(&self, page_index: usize) -> Result<Vec<Block>> {
        self.page_blocks_by(page_index, Detect::default())
    }

    /// [`Pdf::page_blocks`] reading the tags and fonts as `how` says.
    pub fn page_blocks_by(&self, page_index: usize, how: Detect) -> Result<Vec<Block>> {
        let page = self.page_id_at(page_index)?;
        let parsed = self.parse_page(page_index)?;
        let objs = self.objects_of(page, &parsed)?;
        let tags = match how.tags {
            true => self.line_elements(page, &parsed),
            false => vec![None; objs.len()],
        };
        let fonts = match how.fonts {
            true => self.line_main_fonts(page, &parsed),
            false => vec![None; objs.len()],
        };
        Ok(blocks(&objs, &tags, &fonts))
    }

    /// For each of the page's objects, the block element of the document's tags (a
    /// paragraph, heading, list item, table cell and the like) its marked content belongs
    /// to, if it is text that has one.
    fn line_elements(&self, page: ObjectId, parsed: &Parsed) -> Vec<Option<ObjectId>> {
        let mut out = vec![None; parsed.objs.len()];
        let list = match self.struct_parents(page) {
            Ok(list) if !list.is_empty() => list,
            _ => return out,
        };
        let res = self.resolved_dict(self.inherited(page, b"Resources").as_ref());
        let props = self.resolved_dict(res.get(b"Properties").ok());
        // The marked content each operator is in, innermost first.
        let mut stack: Vec<Option<i64>> = Vec::new();
        let mut mcid_at: Vec<Option<i64>> = Vec::with_capacity(parsed.ops.len());
        for op in &parsed.ops {
            match op.operator.as_str() {
                "BDC" => {
                    let dict = match op.operands.get(1) {
                        Some(Object::Dictionary(d)) => Some(d.clone()),
                        Some(Object::Name(n)) => Some(self.resolved_dict(props.get(n).ok())),
                        _ => None,
                    };
                    stack.push(dict.and_then(|d| d.get(b"MCID").and_then(|o| o.as_i64()).ok()));
                }
                "BMC" => stack.push(None),
                "EMC" => {
                    stack.pop();
                }
                _ => {}
            }
            mcid_at.push(stack.iter().rev().find_map(|m| *m));
        }
        let roles = self.role_map();
        let mut known: HashMap<ObjectId, Option<ObjectId>> = HashMap::new();
        for (slot, o) in out.iter_mut().zip(&parsed.objs) {
            let Some(t) = &o.text else { continue };
            let mcid = t.runs.iter().find_map(|r| *mcid_at.get(r.op)?);
            let elem = mcid
                .and_then(|m| usize::try_from(m).ok())
                .and_then(|m| list.get(m))
                .and_then(|o| o.as_reference().ok());
            *slot = elem.and_then(|e| {
                *known
                    .entry(e)
                    .or_insert_with(|| self.block_element(e, &roles))
            });
        }
        out
    }

    /// The document's tag names mapped to standard ones (`/RoleMap`).
    fn role_map(&self) -> HashMap<Vec<u8>, Vec<u8>> {
        let root = (self.doc.catalog().ok())
            .and_then(|c| c.get(b"StructTreeRoot").ok())
            .map(|o| self.resolved_dict(Some(o)));
        let map = root.map(|r| self.resolved_dict(r.get(b"RoleMap").ok()));
        (map.iter().flat_map(|m| m.iter()))
            .filter_map(|(k, v)| Some((k.clone(), v.as_name().ok()?.to_vec())))
            .collect()
    }

    /// The nearest block element at or above structure element `elem`, if there is one
    /// before the tree's root or a grouping element (a section, a list, a table).
    fn block_element(&self, elem: ObjectId, roles: &HashMap<Vec<u8>, Vec<u8>>) -> Option<ObjectId> {
        const BLOCKS: [&[u8]; 18] = [
            b"P",
            b"H",
            b"H1",
            b"H2",
            b"H3",
            b"H4",
            b"H5",
            b"H6",
            b"LBody",
            b"Lbl",
            b"TD",
            b"TH",
            b"Caption",
            b"Note",
            b"FENote",
            b"BlockQuote",
            b"TOCI",
            b"Title",
        ];
        let mut seen = HashSet::new();
        let mut cur = Some(elem);
        while let Some(id) = cur.filter(|&id| seen.insert(id) && seen.len() <= 64) {
            let d = self.doc.get_dictionary(id).ok()?;
            let mut kind = d.get(b"S").and_then(|o| o.as_name()).ok()?.to_vec();
            for _ in 0..8 {
                match roles.get(&kind) {
                    Some(to) if !BLOCKS.contains(&kind.as_slice()) => kind = to.clone(),
                    _ => break,
                }
            }
            if BLOCKS.contains(&kind.as_slice()) {
                return Some(id);
            }
            cur = d.get(b"P").and_then(|o| o.as_reference()).ok();
        }
        None
    }

    /// For each of the page's objects that is text, its font with the most of its
    /// characters (untagged) and whether new text is written in its face: its own program
    /// takes it, or it is one of LimarePDF's own (which grow), or it isn't embedded (the
    /// reader draws it in a face of its own, and new text goes to the face of that name or
    /// the standard one in its place). A Type 3 font, a CMap other than Identity, vertical
    /// writing or a program that can't be used send new text to another face.
    fn line_main_fonts(&self, page: ObjectId, parsed: &Parsed) -> Vec<Option<(String, bool)>> {
        let res = self.font_resources(page);
        let mut read = PageFonts::new(&self.doc, &res);
        let lines = parsed.objs.iter().map(|o| o.text.as_ref());
        lines
            .map(|t| {
                let t = t?;
                let fonts = read.of(parsed, t);
                let (s, f) =
                    (t.spans.iter().zip(&fonts)).max_by_key(|(s, _)| s.chars.1 - s.chars.0)?;
                let other_face = matches!(
                    f.refused(),
                    Some(
                        FontRefusal::Type3
                            | FontRefusal::Cmap
                            | FontRefusal::Vertical
                            | FontRefusal::Unreadable
                    )
                );
                Some((untagged(&s.base).to_string(), !other_face))
            })
            .collect()
    }
}

/// A font name without its subset tag (`ABCDEF+`).
fn untagged(name: &str) -> &str {
    match name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => name,
    }
}

/// A page's line (text object) as detection reads it.
struct Line<'a> {
    o: &'a PageObject,
    base: f32,
    size: f32,
    text: &'a str,
    tag: Option<ObjectId>,
    font: Option<&'a (String, bool)>,
}

/// A visual line while blocks are found.
struct VLine {
    /// Indices into the lines, left to right.
    members: Vec<usize>,
    x0: f32,
    x1: f32,
    top: f32,
    bottom: f32,
    /// The baseline of its longest member.
    y: f32,
    size: f32,
    fill: Option<[u8; 3]>,
    fill_share: f32,
    font: String,
    font_share: f32,
    upright: bool,
    text: String,
    label: bool,
    hyphen_end: bool,
    first_word: f32,
    cjk_start: bool,
    region: usize,
    /// Its column (columns matched across bands are one), and the column's extent.
    col: usize,
    col_extent: (f32, f32),
    grid: bool,
    tags: Vec<ObjectId>,
}

impl VLine {
    fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    fn cx(&self) -> f32 {
        (self.x0 + self.x1) / 2.0
    }
}

/// Why a line didn't join the block above it, and by how much where it matters.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Why {
    /// Nothing above it.
    None,
    Size,
    Fill,
    Font,
    Gap,
    /// The gap off the block's median, by this share of it.
    GapVsMedian(f32),
    Overlap,
    Tags,
    Label,
    Indent,
    Edges,
    /// Its first word would have fitted on the line above, this many sizes to spare.
    NextWordFits(f32),
    NotUpright,
    /// Under a block's middle line, not its last.
    UnderMiddle,
    /// An empty band across the column lies between them.
    Band,
}

impl Why {
    /// Whether the break is clear: a weak one (edges a little off, a gap a little off the
    /// median, a next word that only just fitted) leaves both blocks uncertain.
    fn strong(self) -> bool {
        match self {
            Why::Edges | Why::UnderMiddle => false,
            Why::GapVsMedian(m) => m >= STRONG_GAP_DEV,
            Why::NextWordFits(m) => m >= STRONG_FIT,
            _ => true,
        }
    }
}

const LEFT: u8 = 1;
const RIGHT: u8 = 2;
const CENTRE: u8 = 4;

/// A block while blocks are found.
struct Group {
    lines: Vec<usize>,
    gaps: Vec<f32>,
    /// The left edge of the lines after the first.
    left: f32,
    right: f32,
    /// The alignments its lines agree with so far.
    hyp: u8,
    top: Why,
    bottom: Why,
    indent: bool,
}

impl Group {
    fn new(v: &VLine, at: usize, top: Why) -> Self {
        Self {
            lines: vec![at],
            gaps: Vec::new(),
            left: v.x0,
            right: v.x1,
            hyp: LEFT | RIGHT | CENTRE,
            top,
            bottom: Why::None,
            indent: false,
        }
    }

    fn median_gap(&self) -> Option<f32> {
        median(&self.gaps)
    }

    /// The alignment taken: the first it still agrees with, of left, right and centre.
    fn align(&self) -> u8 {
        [LEFT, RIGHT, CENTRE]
            .into_iter()
            .find(|a| self.hyp & a != 0)
            .unwrap_or(LEFT)
    }
}

fn median(v: &[f32]) -> Option<f32> {
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    let n = s.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(s[n / 2]),
        _ => Some((s[n / 2 - 1] + s[n / 2]) / 2.0),
    }
}

/// The blocks of a page whose objects are `objs`, with each object's tag element (`tags`)
/// and main font (`fonts`) where those are read.
fn blocks(
    objs: &[PageObject],
    tags: &[Option<ObjectId>],
    fonts: &[Option<(String, bool)>],
) -> Vec<Block> {
    let lines: Vec<Line> = (objs.iter().enumerate())
        .filter(|(_, o)| o.kind == ObjectKind::Text && !o.hidden && !o.vertical)
        .filter_map(|(i, o)| {
            let text = o.text.as_deref()?.trim();
            let base = o.baseline?.1;
            let size = o.font_size.filter(|s| *s > 0.0)?;
            (!text.is_empty()).then_some(Line {
                o,
                base,
                size,
                text,
                tag: tags.get(i).copied().flatten(),
                font: fonts.get(i).and_then(Option::as_ref),
            })
        })
        .collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let boxes: Vec<[f32; 4]> = lines.iter().map(|l| l.o.bbox).collect();
    let mut vlines: Vec<VLine> = Vec::new();
    for (ri, (idx, col, extent)) in regions(&boxes).into_iter().enumerate() {
        let mut vls = visual_lines(&lines, &idx);
        let mates = (0..vls.len())
            .filter(|&a| {
                (0..vls.len()).any(|b| {
                    let size = vls[a].size.max(vls[b].size);
                    a != b && (vls[a].y - vls[b].y).abs() < BASE_TOL * size
                })
            })
            .count();
        let grid = vls.len() >= 2 && mates as f32 >= GRID_ROWMATES * vls.len() as f32;
        for v in &mut vls {
            v.region = ri;
            v.col = col;
            v.col_extent = extent;
            v.grid = grid;
        }
        vlines.extend(vls);
    }
    let page_gap = page_spacing(&vlines);
    let others: Vec<&PageObject> = (objs.iter())
        .filter(|o| o.kind != ObjectKind::Text)
        .collect();
    // Pictures, forms and solid shapes, which text can be wrapped round.
    let beside: Vec<[f32; 4]> = (others.iter())
        .map(|o| o.bbox)
        .filter(|b| b[2] - b[0] > OBJ_THIN && b[3] - b[1] > OBJ_THIN)
        .collect();
    let mut by_col: Vec<(usize, Vec<usize>)> = Vec::new();
    for (k, v) in vlines.iter().enumerate() {
        match by_col.iter_mut().find(|(c, _)| *c == v.col) {
            Some((_, list)) => list.push(k),
            None => by_col.push((v.col, vec![k])),
        }
    }
    let mut groups: Vec<Group> = Vec::new();
    for (_, list) in by_col {
        groups.extend(group(&vlines, list, page_gap, &beside));
    }
    // The page's body size: the most common size by characters.
    let mut sizes: HashMap<i32, usize> = HashMap::new();
    for v in &vlines {
        *sizes.entry((v.size * 10.0).round() as i32).or_default() += v.text.chars().count();
    }
    let body = (sizes.into_iter().max_by_key(|&(s, n)| (n, s))).map(|(s, _)| s as f32 / 10.0);
    let mut out: Vec<Block> = (groups.iter())
        .map(|g| {
            let doubt = doubt(g, &vlines, &lines, &others, body);
            let frame = frame(g, &vlines);
            let lines = (g.lines.iter())
                .map(|&k| {
                    let v = &vlines[k];
                    let mut pieces: Vec<usize> = v.members.iter().map(|&m| lines[m].o.id).collect();
                    pieces.sort_unstable();
                    VisualLine {
                        pieces,
                        bbox: [v.x0, v.top, v.x1, v.bottom],
                        baseline: v.y,
                        size: v.size,
                    }
                })
                .collect();
            Block {
                lines,
                frame,
                doubt,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        let (p, q) = (&a.lines[0].bbox, &b.lines[0].bbox);
        p[1].total_cmp(&q[1]).then(p[0].total_cmp(&q[0]))
    });
    out
}

/// The leaf regions of an XY cut of `boxes`, in reading order: each as the indices of its
/// boxes, its column and the column's extent. A region's column is the one the nearest cut
/// down the page above it made (the whole page when none did); columns cut in different
/// bands are matched into one where they line up, as columns of lines on a common baseline
/// are cut one band per line.
fn regions(boxes: &[[f32; 4]]) -> Vec<(Vec<usize>, usize, (f32, f32))> {
    let mut hs: Vec<f32> = boxes
        .iter()
        .map(|b| b[3] - b[1])
        .filter(|h| *h > 0.0)
        .collect();
    hs.sort_by(f32::total_cmp);
    let h = hs.get(hs.len() / 2).copied().unwrap_or(10.0);
    let extent = |idx: &[usize]| {
        (idx.iter()).fold((f32::MAX, f32::MIN), |(l, r), &i| {
            (l.min(boxes[i][0]), r.max(boxes[i][2]))
        })
    };
    // Columns made so far: their extent, and whether a cut down the page made them.
    let mut cols: Vec<((f32, f32), bool)> = Vec::new();
    let mut out = Vec::new();
    // Regions still to cut, the next on top, each with its column.
    let mut todo: Vec<(Vec<usize>, Option<usize>)> = vec![((0..boxes.len()).collect(), None)];
    while let Some((idx, col)) = todo.pop() {
        let col = col.unwrap_or_else(|| {
            cols.push((extent(&idx), false));
            cols.len() - 1
        });
        if idx.len() < 2 {
            out.push((idx, col));
            continue;
        }
        let at: Vec<f32> = (gaps(boxes, &idx, 1).into_iter())
            .filter(|g| g.1 - g.0 >= ROW_GAP * h)
            .map(|g| (g.0 + g.1) / 2.0)
            .collect();
        if !at.is_empty() {
            let mut parts = vec![Vec::new(); at.len() + 1];
            for i in idx {
                parts[at.partition_point(|&y| y <= boxes[i][1])].push(i);
            }
            todo.extend(
                parts
                    .into_iter()
                    .rev()
                    .filter(|p| !p.is_empty())
                    .map(|p| (p, Some(col))),
            );
            continue;
        }
        let (left, right) = extent(&idx);
        let wider = |a: &&(f32, f32), b: &&(f32, f32)| (a.1 - a.0).total_cmp(&(b.1 - b.0));
        let down = gaps(boxes, &idx, 0);
        let cut = (down.iter())
            .filter(|g| {
                g.1 - g.0 >= COL_GAP * h
                    && g.0 - left >= COL_WIDTH * h
                    && right - g.1 >= COL_WIDTH * h
            })
            .max_by(wider)
            .map(|g| (g.0 + g.1) / 2.0);
        match cut {
            Some(x) => {
                let (a, b): (Vec<usize>, Vec<usize>) =
                    idx.into_iter().partition(|&i| boxes[i][0] < x);
                for side in [b, a] {
                    cols.push((extent(&side), true));
                    todo.push((side, Some(cols.len() - 1)));
                }
            }
            None => out.push((idx, col)),
        }
    }
    let lines_up = |(l0, r0): (f32, f32), (l1, r1): (f32, f32)| {
        let (w0, w1) = (r0 - l0, r1 - l1);
        let overlap = r0.min(r1) - l0.max(l1);
        overlap >= COL_SAME * w0.min(w1) && (w0 - w1).abs() <= COL_WIDTHS * w0.max(w1)
    };
    // Columns cut down the page that line up are one: each takes the first of its kind.
    let mut same: Vec<usize> = (0..cols.len()).collect();
    for a in 0..cols.len() {
        for b in 0..a {
            if cols[a].1 && cols[b].1 && lines_up(cols[a].0, cols[b].0) {
                same[a] = same[b];
                break;
            }
        }
    }
    (out.into_iter())
        .map(|(idx, col)| {
            let c = same[col];
            (idx, c, cols[c].0)
        })
        .collect()
}

/// The lines `idx` of a region as visual lines: lines on one baseline (and smaller raised or
/// lowered pieces beside them) together, split at wide gaps.
fn visual_lines(lines: &[Line], idx: &[usize]) -> Vec<VLine> {
    let mut order = idx.to_vec();
    order.sort_by(|&a, &b| {
        (lines[b].size.total_cmp(&lines[a].size)).then(lines[a].base.total_cmp(&lines[b].base))
    });
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for i in order {
        let l = &lines[i];
        let row = rows.iter_mut().find(|row| {
            let r = &lines[row[0]];
            let dy = (l.base - r.base).abs();
            if dy < BASE_TOL * l.size.max(r.size) {
                return true;
            }
            if l.size <= SUP_SIZE * r.size && dy < SUP_BASE * r.size {
                let x0 = row
                    .iter()
                    .map(|&m| lines[m].o.bbox[0])
                    .fold(f32::MAX, f32::min);
                let x1 = row
                    .iter()
                    .map(|&m| lines[m].o.bbox[2])
                    .fold(f32::MIN, f32::max);
                let reach = SUP_REACH * r.size;
                return l.o.bbox[0] < x1 + reach && l.o.bbox[2] > x0 - reach;
            }
            false
        });
        match row {
            Some(row) => row.push(i),
            None => rows.push(vec![i]),
        }
    }
    let mut out = Vec::new();
    for mut row in rows {
        row.sort_by(|&a, &b| lines[a].o.bbox[0].total_cmp(&lines[b].o.bbox[0]));
        let word = word_gap(lines, &row);
        let mut group = vec![row[0]];
        let mut end = lines[row[0]].o.bbox[2];
        for &i in &row[1..] {
            let size = group
                .iter()
                .map(|&m| lines[m].size)
                .fold(lines[i].size, f32::max);
            let first = lines[group[0]].text;
            let label = group.len() == 1
                && first.chars().count() <= LABEL_PIECE
                && !first.chars().any(char::is_alphabetic);
            let gap = lines[i].o.bbox[0] - end;
            let split = match label {
                true => gap > LABEL_REACH * size,
                false => gap > SPLIT_GAP * size && word.is_none_or(|w| gap > SPLIT_WORDS * w),
            };
            if split {
                out.push(vline(lines, std::mem::take(&mut group)));
            }
            group.push(i);
            end = end.max(lines[i].o.bbox[2]);
        }
        out.push(vline(lines, group));
    }
    out
}

/// A row's word gap: the median of the spaces inside its lines, which a justified line
/// stretches as it does the gaps between them; with none (each word drawn on its own), the
/// lower median of the gaps between its lines, when there are three or more (fewer say
/// nothing of its words: a label and its value).
fn word_gap(lines: &[Line], row: &[usize]) -> Option<f32> {
    let mut spaces: Vec<f32> = Vec::new();
    for &i in row {
        let o = lines[i].o;
        let chars: Vec<char> = o.text.as_deref().unwrap_or("").chars().collect();
        for (k, c) in chars.iter().enumerate() {
            if *c == ' ' && k > 0 && k + 1 < chars.len() {
                if let (Some(a), Some(b)) = (o.stops.get(k), o.stops.get(k + 1)) {
                    spaces.push(b - a);
                }
            }
        }
    }
    if spaces.is_empty() {
        for w in row.windows(2) {
            let (a, b) = (&lines[w[0]].o.bbox, &lines[w[1]].o.bbox);
            if b[0] > a[2] {
                spaces.push(b[0] - a[2]);
            }
        }
        if spaces.len() < 3 {
            return None;
        }
    }
    spaces.sort_by(f32::total_cmp);
    spaces.get((spaces.len().max(1) - 1) / 2).copied()
}

/// A visual line of `members` (indices into `lines`, left to right).
fn vline(lines: &[Line], members: Vec<usize>) -> VLine {
    let ms = || members.iter().map(|&m| &lines[m]);
    let x0 = ms().map(|l| l.o.bbox[0]).fold(f32::MAX, f32::min);
    let x1 = ms().map(|l| l.o.bbox[2]).fold(f32::MIN, f32::max);
    let top = ms().map(|l| l.o.bbox[1]).fold(f32::MAX, f32::min);
    let bottom = ms().map(|l| l.o.bbox[3]).fold(f32::MIN, f32::max);
    // Dominant size, fill and font by characters (not spaces).
    let mut size_w: HashMap<i32, usize> = HashMap::new();
    let mut fill_w: HashMap<Option<[u8; 3]>, usize> = HashMap::new();
    let mut font_w: HashMap<&str, usize> = HashMap::new();
    let mut total = 0;
    for l in ms() {
        let chars: Vec<char> = l.o.text.as_deref().unwrap_or("").chars().collect();
        for s in &l.o.spans {
            let part = chars.get(s.chars.0..s.chars.1).unwrap_or(&[]);
            let n = part.iter().filter(|c| !c.is_whitespace()).count();
            if n == 0 {
                continue;
            }
            *size_w
                .entry((s.font_size * 10.0).round() as i32)
                .or_default() += n;
            *fill_w.entry(s.fill.map(colour)).or_default() += n;
            *font_w.entry(untagged(&s.font_name)).or_default() += n;
            total += n;
        }
    }
    let most = |m: &HashMap<i32, usize>| m.iter().max_by_key(|&(k, n)| (*n, *k)).map(|(k, _)| *k);
    let size = most(&size_w).map_or_else(
        || ms().map(|l| l.size).fold(0.0, f32::max),
        |s| s as f32 / 10.0,
    );
    let (fill, fill_n) = (fill_w.iter())
        .max_by_key(|&(k, n)| (*n, *k))
        .map_or((None, 0), |(k, n)| (*k, *n));
    let (font, font_n) = (font_w.iter())
        .max_by_key(|&(k, n)| (*n, *k))
        .map_or(("", 0), |(k, n)| (*k, *n));
    let share = |n: usize| {
        if total > 0 {
            n as f32 / total as f32
        } else {
            0.0
        }
    };
    let main = ms()
        .max_by_key(|l| l.text.chars().count())
        .expect("a member");
    let text = ms().map(|l| l.text).collect::<Vec<_>>().join(" ");
    let first = &lines[members[0]];
    let mut label = label_at(&text).is_some_and(|(_, sure, gap)| sure || gap);
    if members.len() > 1 {
        let next = &lines[members[1]];
        let t0 = first.text;
        if t0.chars().count() <= LABEL_PIECE
            && !t0.chars().any(char::is_alphabetic)
            && next.o.bbox[0] - first.o.bbox[2] >= LABEL_GAP * size
        {
            label = true;
        }
    }
    // A label that could be a word's start needs a gap, which may be inside the first line:
    // a space wider than its others.
    if !label {
        if let Some((n, false, false)) = label_at(first.text) {
            label = wide_space_after(first.o, n);
        }
    }
    let first_word = first_word_width(first.o);
    let cjk_start = first.text.chars().next().is_some_and(cjk);
    VLine {
        x0,
        x1,
        top,
        bottom,
        y: main.base,
        size,
        fill,
        fill_share: share(fill_n),
        font: font.to_string(),
        font_share: share(font_n),
        upright: ms().all(|l| l.o.upright),
        hyphen_end: text.ends_with(['-', '\u{AD}', '\u{2010}']),
        text,
        label,
        first_word,
        cjk_start,
        region: 0,
        col: 0,
        col_extent: (x0, x1),
        grid: false,
        tags: {
            let mut t: Vec<ObjectId> = ms().filter_map(|l| l.tag).collect();
            t.sort_unstable();
            t.dedup();
            t
        },
        members,
    }
}

/// A fill colour to compare by, in 1/255ths.
fn colour(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Whether `c` is Chinese, Japanese or Korean (or their punctuation and full-width forms).
fn cjk(c: char) -> bool {
    matches!(c, '\u{2E80}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}' | '\u{F900}'..='\u{FAFF}' | '\u{FF00}'..='\u{FFEF}')
}

/// The width of the first word of line `o`, from where its characters are.
fn first_word_width(o: &PageObject) -> f32 {
    let chars: Vec<char> = o.text.as_deref().unwrap_or("").chars().collect();
    let Some(start) = chars.iter().position(|c| !c.is_whitespace()) else {
        return 0.0;
    };
    let end = match cjk(chars[start]) {
        true => start + 1,
        false => (start..chars.len())
            .find(|&k| chars[k].is_whitespace())
            .unwrap_or(chars.len()),
    };
    match (o.stops.get(start), o.stops.get(end)) {
        (Some(a), Some(b)) => b - a,
        _ => {
            let n = chars.len().max(1) as f32;
            (o.bbox[2] - o.bbox[0]) * (end - start) as f32 / n
        }
    }
}

/// Whether the space after the first `n` characters (a label) of line `o`, past its
/// leading spaces, is wider than its other spaces: a gap after a numbered label.
fn wide_space_after(o: &PageObject, n: usize) -> bool {
    let chars: Vec<char> = o.text.as_deref().unwrap_or("").chars().collect();
    let lead = chars.iter().take_while(|c| c.is_whitespace()).count();
    let at = lead + n;
    let width = |k: usize| Some(o.stops.get(k + 1)? - o.stops.get(k)?);
    let Some(after) = (at < chars.len() && chars[at] == ' ')
        .then(|| width(at))
        .flatten()
    else {
        return false;
    };
    let mut spaces: Vec<f32> = (at + 1..chars.len())
        .filter(|&k| chars[k] == ' ')
        .filter_map(width)
        .collect();
    spaces.sort_by(f32::total_cmp);
    let usual = spaces.get(spaces.len() / 2).copied();
    usual.is_some_and(|u| after > 1.5 * u) && after >= LABEL_GAP * o.font_size.unwrap_or(10.0)
}

/// A list label at the start of `text`: its length in characters, whether it is surely one
/// rather than a word's start, and whether a gap follows it. Sure: bullets and dashes
/// followed by a space, numbers like "1.", "2)" and "1.2.", circled numbers, and Chinese and
/// Japanese counting ("（一）", "一、"). Not sure, as text can start so: "1.2" ("2.2
/// million"), "(3)" ("(88) per cent"), "a)", "iv.".
fn label_at(text: &str) -> Option<(usize, bool, bool)> {
    let t = text.trim_start();
    let chars: Vec<char> = t.chars().collect();
    let first = *chars.first()?;
    let space_at = |k: usize| chars.get(k).is_some_and(|c| c.is_whitespace());
    // A gap: two spaces or more, or a tab.
    let gap_at = |k: usize| chars.get(k) == Some(&'\t') || (space_at(k) && space_at(k + 1));
    if "•·◦▪▫‣⁃●○■□►▸–—-*".contains(first) && space_at(1) {
        return Some((1, true, true));
    }
    if ('\u{2460}'..='\u{24FF}').contains(&first) || ('\u{2776}'..='\u{2793}').contains(&first) {
        return Some((1, true, true));
    }
    const HAN: &str = "一二三四五六七八九十";
    let han_or_digit = |c: char| HAN.contains(c) || c.is_ascii_digit();
    if "（(".contains(first) {
        let n = chars[1..].iter().take_while(|c| han_or_digit(**c)).count();
        let close = chars.get(1 + n).copied();
        // "(3)" is as often text ("(88) per cent"): the numbers below take it.
        let ascii =
            first == '(' && close == Some(')') && chars[1..1 + n].iter().all(char::is_ascii_digit);
        if (1..=3).contains(&n) && close.is_some_and(|c| ")）".contains(c)) && !ascii {
            return Some((n + 2, true, true));
        }
    }
    let n = chars.iter().take_while(|c| han_or_digit(**c)).count();
    if (1..=3).contains(&n) && chars.get(n).is_some_and(|c| "、．".contains(*c)) {
        return Some((n + 1, true, true));
    }
    // Numbers and letters: "1." "1)" "1.2" "1.2." "a." "a)" "A)" "iv." "IV)", optionally
    // in brackets, then a space.
    let open = usize::from(first == '(');
    let rest = &chars[open..];
    let digits = |from: usize| {
        rest[from..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count()
    };
    let mut k = digits(0);
    let mut sure = false;
    let numbered = if (1..=3).contains(&k) {
        let mut parts = 1;
        while rest.get(k) == Some(&'.') && (1..=3).contains(&digits(k + 1)) {
            k += 1 + digits(k + 1);
            parts += 1;
        }
        match rest.get(k) {
            Some('.') | Some(')') => {
                k += 1;
                sure = open == 0;
                true
            }
            _ => parts > 1,
        }
    } else {
        let roman_lower = rest.iter().take_while(|c| "ivxlc".contains(**c)).count();
        let roman_upper = rest.iter().take_while(|c| "IVXLC".contains(**c)).count();
        let one = |c: char| matches!(rest.get(1), Some(p) if *p == c);
        let lower = rest.first().is_some_and(|c| c.is_ascii_lowercase()) && (one('.') || one(')'));
        let upper = rest.first().is_some_and(|c| c.is_ascii_uppercase()) && one(')');
        if lower || upper {
            k = 2;
            true
        } else if (1..=5).contains(&roman_lower) && matches!(rest.get(roman_lower), Some('.' | ')'))
        {
            k = roman_lower + 1;
            true
        } else if (1..=5).contains(&roman_upper) && rest.get(roman_upper) == Some(&')') {
            k = roman_upper + 1;
            true
        } else {
            false
        }
    };
    let k = k + open;
    (numbered && space_at(k)).then(|| (k, sure, gap_at(k)))
}

/// The page's own line spacing, in sizes: the most common gap from a visual line to the
/// one below it in its column at the same size, where it is more than [`GAP_MAX`] (double
/// spacing), up to [`GAP_MAX_PAGE`].
fn page_spacing(vlines: &[VLine]) -> f32 {
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for (k, w) in vlines.iter().enumerate() {
        let above = (vlines.iter().enumerate())
            .filter(|&(j, u)| j != k && u.col == w.col && u.y < w.y - BASE_TOL * w.size)
            .filter(|(_, u)| u.x1.min(w.x1) - u.x0.max(w.x0) > 0.0)
            .max_by(|a, b| a.1.y.total_cmp(&b.1.y));
        if let Some((_, u)) = above.filter(|(_, u)| (u.size - w.size).abs() <= SIZE_TOL * w.size) {
            let sizes = (w.y - u.y) / w.size;
            if sizes > GAP_MIN && sizes <= GAP_MAX_PAGE * 1.2 {
                *counts.entry((sizes * 20.0).round() as i32).or_default() += 1;
            }
        }
    }
    let mode = (counts.into_iter().max_by_key(|&(s, n)| (n, -s))).map(|(s, _)| s as f32 / 20.0);
    mode.map_or(GAP_MAX, |m| {
        (m * (1.0 + GAP_MED_TOL)).clamp(GAP_MAX, GAP_MAX_PAGE)
    })
}

/// The edge tolerance for text of `size`.
fn edge_tol(size: f32) -> f32 {
    EDGE_PT.max(EDGE_EM * size)
}

/// Which alignments line `w` agrees with against block `b`'s lines, and whether it makes the
/// block's first line an indent (a first-line or hanging one) when it is the second line.
fn edge_checks(b: &Group, w: &VLine, vl: &[VLine]) -> (u8, bool) {
    let v = &vl[*b.lines.last().expect("a line")];
    let tol = edge_tol(v.size);
    let (mut agree, mut indent) = (0, false);
    if b.lines.len() == 1 {
        let out_by = v.x0 - w.x0;
        if out_by.abs() <= tol {
            agree |= LEFT;
        } else if (INDENT_MIN * v.size..=INDENT_MAX * v.size).contains(&out_by) {
            agree |= LEFT;
            indent = true;
        } else if (v.label && -out_by > 0.0 && -out_by <= HANG_MAX * v.size)
            || (INDENT_MIN * v.size..=INDENT_MAX * v.size).contains(&-out_by)
        {
            // A hanging indent: a list item's after its label, a reference's after its
            // first line.
            agree |= LEFT;
            indent = true;
        }
        if (w.x1 - v.x1).abs() <= tol {
            agree |= RIGHT;
        }
        if (w.cx() - v.cx()).abs() <= EDGE_PT {
            agree |= CENTRE;
        }
    } else {
        if (w.x0 - b.left).abs() <= tol {
            agree |= LEFT;
        }
        let rights: Vec<f32> = b.lines.iter().map(|&k| vl[k].x1).collect();
        if median(&rights).is_some_and(|r| (w.x1 - r).abs() <= tol) {
            agree |= RIGHT;
        }
        let mids: Vec<f32> = b.lines.iter().map(|&k| vl[k].cx()).collect();
        if median(&mids).is_some_and(|c| (w.cx() - c).abs() <= EDGE_PT) {
            agree |= CENTRE;
        }
    }
    (agree, indent)
}

/// Whether visual line `w` continues block `b`: the join's gap, alignments and indent, or
/// why not. `beside` are the page's pictures and solid shapes, which can stop a line short.
fn try_join(
    b: &Group,
    w: &VLine,
    vl: &[VLine],
    page_gap: f32,
    beside: &[[f32; 4]],
) -> std::result::Result<(f32, u8, bool), Why> {
    let v = &vl[*b.lines.last().expect("a line")];
    let size = v.size;
    if !(v.upright && w.upright) {
        return Err(Why::NotUpright);
    }
    if (w.size - v.size).abs() > SIZE_TOL * v.size {
        return Err(Why::Size);
    }
    if w.fill != v.fill && v.fill_share >= STYLE_ALL && w.fill_share >= STYLE_ALL {
        return Err(Why::Fill);
    }
    if v.font != w.font && v.font_share >= STYLE_ALL && w.font_share >= STYLE_ALL {
        return Err(Why::Font);
    }
    let gap = w.y - v.y;
    if !(GAP_MIN * size..=page_gap * size).contains(&gap) {
        return Err(Why::Gap);
    }
    if let Some(med) = b.median_gap() {
        if (gap - med).abs() > GAP_MED_TOL * med {
            return Err(Why::GapVsMedian((gap - med).abs() / med));
        }
    }
    if v.x1.min(w.x1) - v.x0.max(w.x0) < OVERLAP_MIN * v.width().min(w.width()) {
        return Err(Why::Overlap);
    }
    if !v.tags.is_empty() && !w.tags.is_empty() && !v.tags.iter().any(|t| w.tags.contains(t)) {
        return Err(Why::Tags);
    }
    if w.label {
        return Err(Why::Label);
    }
    let (agree, indent) = edge_checks(b, w, vl);
    let hyp = b.hyp & agree;
    if hyp == 0 {
        if b.hyp & LEFT != 0 && b.lines.len() >= 2 && w.x0 - b.left >= INDENT_MIN * size {
            return Err(Why::Indent);
        }
        return Err(Why::Edges);
    }
    // Would `w`'s first word have fitted at the end of `v`? Then `v` ended with a hard
    // break. The room is measured against the paragraph's own width, up to a picture
    // beside `v` (text wrapped round it), for the alignment that leaves the least of it.
    if !v.hyphen_end {
        let space = if w.cjk_start { 0.0 } else { SPACE_EM * size };
        let first = &vl[b.lines[0]];
        let level = || beside.iter().filter(|o| o[1] < v.bottom && o[3] > v.top);
        let mut rooms = Vec::new();
        if hyp & LEFT != 0 {
            let stop = level()
                .map(|o| o[0])
                .filter(|&x| x >= v.x1 - 1.0)
                .fold(f32::MAX, f32::min);
            rooms.push(b.right.max(w.x1).min(stop) - v.x1);
        }
        if hyp & RIGHT != 0 {
            let stop = level()
                .map(|o| o[2])
                .filter(|&x| x <= v.x0 + 1.0)
                .fold(f32::MIN, f32::max);
            rooms.push(v.x0 - first.x0.min(w.x0).max(stop));
        }
        if hyp & CENTRE != 0 {
            rooms.push((b.right - b.left).max(w.width()) - v.width());
        }
        let room = rooms.into_iter().fold(f32::MAX, f32::min);
        if space + w.first_word <= room - FIT_SLACK * size {
            return Err(Why::NextWordFits((room - space - w.first_word) / size));
        }
    }
    Ok((gap, hyp, indent))
}

/// A clear reason for a break between block `b` and line `w` under it, if there is one,
/// whichever check failed first: a change of size, colour or font, a list label, a gap
/// out of a paragraph's range, or an indent under a left-aligned block.
fn strong_signal(b: &Group, w: &VLine, vl: &[VLine], page_gap: f32) -> Option<Why> {
    let v = &vl[*b.lines.last().expect("a line")];
    if (w.size - v.size).abs() > SIZE_TOL * v.size {
        return Some(Why::Size);
    }
    if w.fill != v.fill && v.fill_share >= STYLE_ALL && w.fill_share >= STYLE_ALL {
        return Some(Why::Fill);
    }
    if v.font != w.font && v.font_share >= STYLE_ALL && w.font_share >= STYLE_ALL {
        return Some(Why::Font);
    }
    if w.label {
        return Some(Why::Label);
    }
    let gap = w.y - v.y;
    if !(GAP_MIN * v.size..=page_gap * v.size).contains(&gap) {
        return Some(Why::Gap);
    }
    if b.lines.len() >= 2 && b.align() == LEFT && w.x0 - b.left >= INDENT_MIN * v.size {
        return Some(Why::Indent);
    }
    None
}

/// A column's visual lines `list` (indices into `vl`) as blocks: top to bottom, each line
/// joins the block of the nearest line above it that overlaps it, when that line is the
/// block's last and the join's rules pass; else it starts a block.
fn group(vl: &[VLine], mut list: Vec<usize>, page_gap: f32, beside: &[[f32; 4]]) -> Vec<Group> {
    list.sort_by(|&a, &b| {
        vl[a]
            .y
            .total_cmp(&vl[b].y)
            .then(vl[a].x0.total_cmp(&vl[b].x0))
    });
    let mut groups: Vec<Group> = Vec::new();
    let mut owner: HashMap<usize, usize> = HashMap::new();
    for (k, &wi) in list.iter().enumerate() {
        let w = &vl[wi];
        let above = list[..k].iter().rev().copied().find(|&ui| {
            let u = &vl[ui];
            u.y < w.y - BASE_TOL * w.size && u.x1.min(w.x1) - u.x0.max(w.x0) > 0.0
        });
        let mut why = Why::None;
        if let Some(ui) = above {
            let g = owner[&ui];
            if groups[g].lines.last() == Some(&ui) {
                match try_join(&groups[g], w, vl, page_gap, beside) {
                    Ok((gap, hyp, indent)) => {
                        let b = &mut groups[g];
                        if b.lines.len() == 1 {
                            b.left = if indent { w.x0 } else { b.left.min(w.x0) };
                            b.indent = indent;
                        }
                        b.hyp = hyp;
                        b.gaps.push(gap);
                        b.lines.push(wi);
                        b.right = b.right.max(w.x1);
                        owner.insert(wi, g);
                        continue;
                    }
                    Err(reason) => {
                        why = reason;
                        if !why.strong() {
                            if let Some(also) = strong_signal(&groups[g], w, vl, page_gap) {
                                why = also;
                            }
                        }
                        if vl[ui].region != w.region {
                            why = Why::Band;
                        }
                        groups[g].bottom = why;
                    }
                }
            } else {
                why = Why::UnderMiddle;
            }
        }
        owner.insert(wi, groups.len());
        groups.push(Group::new(w, wi, why));
    }
    groups
}

/// Why block `b` isn't confident, if it isn't. `body` is the page's body size; `others`
/// its pictures, forms and shapes.
fn doubt(
    b: &Group,
    vl: &[VLine],
    lines: &[Line],
    others: &[&PageObject],
    body: Option<f32>,
) -> Option<Doubt> {
    let ls: Vec<&VLine> = b.lines.iter().map(|&k| &vl[k]).collect();
    if ls.len() < 2 {
        return Some(Doubt::OneLine);
    }
    let align = b.align();
    if align != LEFT && ls.len() < CONF_CENTRED_LINES {
        return Some(Doubt::ShortCentred);
    }
    if !ls.iter().all(|v| v.upright) {
        return Some(Doubt::Turned);
    }
    if b.lines.iter().any(|&k| vl[k].grid) {
        return Some(Doubt::Table);
    }
    if !b.top.strong() || !b.bottom.strong() {
        return Some(Doubt::WeakBreak);
    }
    let med = b.median_gap().unwrap_or(0.0);
    if b.gaps.iter().any(|g| (g - med).abs() > CONF_GAP_TOL * med) {
        return Some(Doubt::UnevenGaps);
    }
    let mids: Vec<f32> = ls.iter().map(|v| v.cx()).collect();
    let mid = median(&mids).unwrap_or(0.0);
    for (k, v) in ls.iter().enumerate() {
        if k == 0 && b.indent {
            continue;
        }
        let d = match align {
            LEFT => (v.x0 - b.left).abs(),
            RIGHT => (v.x1 - b.right).abs(),
            _ => (v.cx() - mid).abs(),
        };
        if d > CONF_EDGE {
            return Some(Doubt::UnevenEdges);
        }
    }
    let x0 = ls.iter().map(|v| v.x0).fold(f32::MAX, f32::min);
    let x1 = ls.iter().map(|v| v.x1).fold(f32::MIN, f32::max);
    let width = x1 - x0;
    if width < CONF_MIN_WIDTH * ls[0].size {
        return Some(Doubt::Narrow);
    }
    if ls[..ls.len() - 1]
        .iter()
        .any(|v| v.text.trim_end().ends_with(':'))
    {
        return Some(Doubt::Table);
    }
    if ls.iter().any(|v| leaders(&v.text)) {
        return Some(Doubt::Table);
    }
    if !continued(&ls) {
        return Some(Doubt::NoReadOn);
    }
    for pair in ls.windows(2) {
        let (v, w) = (pair[0], pair[1]);
        let first = w.text.chars().find(|c| c.is_alphabetic());
        if sentence_end(&v.text)
            && first.is_some_and(char::is_uppercase)
            && b.right - v.x1 >= CONF_END_ROOM * v.size
        {
            return Some(Doubt::MayEnd);
        }
    }
    let (cl, cr) = ls[0].col_extent;
    if width < CONF_COL_SHARE * (cr - cl) {
        return Some(Doubt::Narrow);
    }
    if body.is_some_and(|s| ls[0].size > CONF_BIG * s) {
        return Some(Doubt::LargeType);
    }
    let frame = [x0, ls[0].top, x1, ls[ls.len() - 1].bottom];
    let mine: HashSet<usize> = b.lines.iter().copied().collect();
    for (k, u) in vl.iter().enumerate() {
        if !mine.contains(&k) && intersects([u.x0, u.top, u.x1, u.bottom], frame, OBJ_INSET) {
            return Some(Doubt::TextInTheWay);
        }
    }
    for o in others {
        let bb = o.bbox;
        if !intersects(bb, frame, OBJ_INSET) {
            continue;
        }
        let contains = bb[0] <= frame[0] + 0.5
            && bb[1] <= frame[1] + 0.5
            && bb[2] >= frame[2] - 0.5
            && bb[3] >= frame[3] - 0.5;
        let thin = bb[3] - bb[1] < OBJ_THIN;
        let rule = thin && (bb[3] <= frame[1] + OBJ_THIN || bb[1] >= frame[3] - OBJ_THIN);
        if contains || rule {
            continue;
        }
        return Some(match o.kind {
            ObjectKind::Path => Doubt::ShapeInTheWay,
            _ => Doubt::PictureInTheWay,
        });
    }
    // Its main font: the one with the most of its characters.
    let mut fonts: HashMap<&str, (usize, bool)> = HashMap::new();
    for v in &ls {
        for &m in &v.members {
            if let Some((name, takes)) = lines[m].font {
                let e = fonts.entry(name.as_str()).or_insert((0, *takes));
                e.0 += lines[m].text.chars().count();
            }
        }
    }
    let main = fonts.into_iter().max_by_key(|&(name, (n, _))| (n, name));
    if main.is_some_and(|(_, (_, takes))| !takes) {
        return Some(Doubt::Font);
    }
    None
}

fn intersects(a: [f32; 4], b: [f32; 4], inset: f32) -> bool {
    a[0] + inset < b[2] && b[0] + inset < a[2] && a[1] + inset < b[3] && b[1] + inset < a[3]
}

/// Dot leaders (a contents line) or blanks to fill in: four dots or more (a space allowed
/// between), two ellipses, or four underscores.
fn leaders(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let run = |k: usize, c: char, gap: bool| {
        let mut n = 0;
        let mut j = k;
        while chars.get(j) == Some(&c) {
            n += 1;
            j += 1;
            if gap && chars.get(j) == Some(&' ') && chars.get(j + 1) == Some(&c) {
                j += 1;
            }
        }
        n
    };
    (0..chars.len())
        .any(|k| run(k, '.', true) >= 4 || run(k, '…', false) >= 2 || run(k, '_', false) >= 4)
}

/// Whether `text` ends a sentence: a full stop, question or exclamation mark or colon (or
/// their Chinese and Japanese forms), then closing quotes or brackets.
fn sentence_end(text: &str) -> bool {
    let t = text
        .trim_end()
        .trim_end_matches(['"', '\'', '»', '”', '’', ')', ']']);
    t.ends_with(['.', '!', '?', ':', '。', '！', '？'])
}

/// Whether some line of a block plainly reads on into the next: the next starts with a
/// lower-case letter (or a script without case, or no letter), or this one ends in a
/// hyphen, semicolon or comma, or in a lower-case word with no full stop. A list of names
/// or times, one per line, has none.
fn continued(ls: &[&VLine]) -> bool {
    ls.windows(2).any(|pair| {
        let (v, w) = (pair[0], pair[1]);
        let first = w.text.chars().find(|c| c.is_alphabetic());
        let caseless = |c: char| !c.is_uppercase() && !c.is_lowercase();
        if first.is_none_or(|c| caseless(c) || c.is_lowercase()) {
            return true;
        }
        let end = v.text.trim_end();
        if end.ends_with(['-', '\u{AD}', '\u{2010}', ';', ',']) {
            return true;
        }
        let last_word = end
            .split(|c: char| !c.is_alphabetic())
            .rfind(|w| !w.is_empty());
        // A full stop (or ! ? :) among what follows the last letter or digit.
        let after = end.chars().rev().take_while(|c| !c.is_alphanumeric());
        let stops = after.into_iter().any(|c| ".!?:".contains(c));
        last_word.is_some_and(|w| w.starts_with(char::is_lowercase)) && !stops
    })
}

/// The frame of block `b`: what it is rewrapped in (the paragraph design's 6.1).
fn frame(b: &Group, vl: &[VLine]) -> Frame {
    let ls: Vec<&VLine> = b.lines.iter().map(|&k| &vl[k]).collect();
    let first = ls[0];
    let right = ls.iter().map(|v| v.x1).fold(f32::MIN, f32::max);
    let left = if ls.len() > 1 { b.left } else { first.x0 };
    let tol = edge_tol(first.size);
    let align = match b.align() {
        LEFT if ls.len() >= 2
            && ls[..ls.len() - 1]
                .iter()
                .all(|v| (v.x1 - right).abs() <= tol) =>
        {
            Align::Justified
        }
        RIGHT => Align::Right,
        CENTRE => Align::Centre,
        _ => Align::Left,
    };
    Frame {
        left,
        right,
        baseline: first.y,
        spacing: b.median_gap().unwrap_or(0.0),
        align,
        indent: first.x0 - left,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_bullets_numbers_and_counting() {
        assert_eq!(label_at("• Apples"), Some((1, true, true)));
        assert_eq!(label_at("1. Apples"), Some((2, true, false)));
        assert_eq!(label_at("1.  Apples"), Some((2, true, true)));
        assert_eq!(label_at("1.2. Apples"), Some((4, true, false)));
        assert_eq!(label_at("(a) Apples"), Some((3, false, false)));
        assert_eq!(label_at("(3) Apples"), Some((3, false, false)));
        assert_eq!(label_at("iv. Apples"), Some((3, false, false)));
        assert_eq!(label_at("2.2 million"), Some((3, false, false)));
        assert_eq!(label_at("（一）项目"), Some((3, true, true)));
        assert_eq!(label_at("一、项目"), Some((2, true, true)));
        assert_eq!(label_at("Apples"), None);
        assert_eq!(label_at("10 apples"), None);
        assert_eq!(label_at("I think"), None);
    }

    #[test]
    fn leaders_and_sentence_ends() {
        assert!(leaders("Introduction . . . . 4"));
        assert!(leaders("Name ________"));
        assert!(!leaders("Wait... for it"));
        assert!(sentence_end("It ended.”"));
        assert!(sentence_end("Done!"));
        assert!(!sentence_end("and then"));
    }
}
