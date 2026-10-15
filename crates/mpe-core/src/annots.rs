//! Annotations: comments, notes and markup on pages. Reading them, removing them, and
//! flattening them into the page content. Links, form widgets and popups have their own
//! code and are not listed here.

use crate::annot_ap::DRAWN;
use crate::doc::decode_text;
use crate::form_flatten::Flat;
use crate::form_write::parse_da;
use crate::matrix as mx;
use crate::{Pdf, Result};
use anyhow::anyhow;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::HashSet;

/// Limit per page that keeps a malformed or hostile file from running away.
const MAX_ANNOTS: usize = 10_000;

/// Annotation flags (`/F`).
const HIDDEN: i64 = 1 << 1;
pub(crate) const NO_VIEW: i64 = 1 << 5;
const READ_ONLY: i64 = 1 << 6;
const LOCKED: i64 = 1 << 7;

/// What kind of annotation it is (its `/Subtype`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AnnotKind {
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
    /// A sticky note (`/Text`).
    Note,
    /// Text written on the page (`/FreeText`).
    TextBox,
    Square,
    Circle,
    Line,
    Polygon,
    PolyLine,
    /// Freehand drawing (`/Ink`).
    Ink,
    Stamp,
    /// Any other subtype, as the file names it.
    Other(String),
}

impl AnnotKind {
    /// The kind for a subtype; `None` for links, form widgets and popups.
    fn of(subtype: &[u8]) -> Option<Self> {
        Some(match subtype {
            b"Link" | b"Widget" | b"Popup" => return None,
            b"Highlight" => Self::Highlight,
            b"Underline" => Self::Underline,
            b"StrikeOut" => Self::StrikeOut,
            b"Squiggly" => Self::Squiggly,
            b"Text" => Self::Note,
            b"FreeText" => Self::TextBox,
            b"Square" => Self::Square,
            b"Circle" => Self::Circle,
            b"Line" => Self::Line,
            b"Polygon" => Self::Polygon,
            b"PolyLine" => Self::PolyLine,
            b"Ink" => Self::Ink,
            b"Stamp" => Self::Stamp,
            other => Self::Other(String::from_utf8_lossy(other).into_owned()),
        })
    }

    /// A short lowercase name, as the command line takes and shows it.
    pub fn name(&self) -> &str {
        match self {
            Self::Highlight => "highlight",
            Self::Underline => "underline",
            Self::StrikeOut => "strikeout",
            Self::Squiggly => "squiggly",
            Self::Note => "note",
            Self::TextBox => "textbox",
            Self::Square => "square",
            Self::Circle => "circle",
            Self::Line => "line",
            Self::Polygon => "polygon",
            Self::PolyLine => "polyline",
            Self::Ink => "ink",
            Self::Stamp => "stamp",
            Self::Other(s) => s,
        }
    }

    /// The kind for a name from [`AnnotKind::name`] (any case). A subtype MPE doesn't
    /// list, such as `FileAttachment`, is taken as it is.
    pub fn parse(s: &str) -> Self {
        let known = [
            Self::Highlight,
            Self::Underline,
            Self::StrikeOut,
            Self::Squiggly,
            Self::Note,
            Self::TextBox,
            Self::Square,
            Self::Circle,
            Self::Line,
            Self::Polygon,
            Self::PolyLine,
            Self::Ink,
            Self::Stamp,
        ];
        let lower = s.to_ascii_lowercase();
        known
            .into_iter()
            .find(|k| k.name() == lower)
            .unwrap_or_else(|| Self::Other(s.to_string()))
    }

    /// A label for people: "Highlight", "Note", "Text box".
    pub fn label(&self) -> String {
        match self {
            Self::StrikeOut => "Strike-out".into(),
            Self::TextBox => "Text box".into(),
            Self::PolyLine => "Polyline".into(),
            Self::Ink => "Drawing".into(),
            Self::Other(s) => s.clone(),
            k => {
                let n = k.name();
                n[..1].to_ascii_uppercase() + &n[1..]
            }
        }
    }
}

/// One annotation on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Annot {
    /// The annotation's object in the file; stays the same while the document is open.
    pub id: (u32, u16),
    /// The page it is on (zero-based).
    pub page: usize,
    pub kind: AnnotKind,
    /// Its area in displayed-page points (y down), like [`crate::PageObject::bbox`].
    pub bbox: [f32; 4],
    /// For text markup, the box around each marked line, in the same points; empty for
    /// other kinds.
    pub quads: Vec<[f32; 4]>,
    /// Its colour as RGB (grey and CMYK are converted), if it has one.
    pub color: Option<[f32; 3]>,
    /// 0 (invisible) to 1 (opaque).
    pub opacity: f32,
    /// The comment.
    pub contents: String,
    /// Who made it (`/T`).
    pub author: String,
    pub subject: String,
    /// When it was last changed, as `YYYY-MM-DD HH:MM`, or empty when unknown.
    pub modified: String,
    /// The annotation flags (`/F`).
    pub flags: i64,
    /// The comment this one replies to (`/IRT`).
    pub reply_to: Option<(u32, u16)>,
    /// Whether the file has a drawing for it. MPE draws the common kinds itself when
    /// not, on screen and when printing.
    pub has_appearance: bool,
    /// For text boxes, how the text is set; `None` for other kinds.
    pub text_style: Option<TextStyle>,
    /// For rectangles, ellipses, lines and drawings, how they are painted; `None` for
    /// other kinds.
    pub shape: Option<ShapeLook>,
    /// For lines, polylines and drawings, their points in displayed-page points, one list
    /// per stroke; empty for other kinds.
    pub paths: Vec<Vec<(f32, f32)>>,
}

/// How a rectangle, ellipse, line or drawing is painted. Its line's colour is the
/// annotation's colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeLook {
    /// The inside of a rectangle or ellipse (none when transparent).
    pub fill: Option<[f32; 3]>,
    /// The line's width in points.
    pub width: f32,
    /// A line has an arrowhead at its end.
    pub arrow: bool,
}

impl Default for ShapeLook {
    fn default() -> Self {
        Self {
            fill: None,
            width: 1.0,
            arrow: false,
        }
    }
}

/// How a text box's text is set: Helvetica at `size` points in `color`, on `fill` (none
/// when transparent), with a border `border` points wide in the text colour (0 for none).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub color: [f32; 3],
    pub fill: Option<[f32; 3]>,
    pub border: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: 12.0,
            color: [0.0; 3],
            fill: None,
            border: 0.0,
        }
    }
}

impl Annot {
    pub fn hidden(&self) -> bool {
        self.flags & HIDDEN != 0
    }

    /// Whether it is shown on the page: not hidden and not a reply, which is listed with
    /// the comment it answers.
    pub fn on_page(&self) -> bool {
        self.flags & (HIDDEN | NO_VIEW) == 0 && self.reply_to.is_none()
    }

    /// Locked or read-only: it may not be moved, changed or deleted in the app.
    pub fn locked(&self) -> bool {
        self.flags & (LOCKED | READ_ONLY) != 0
    }
}

impl Pdf {
    /// The annotations on every page, page by page in the file's order.
    pub fn annotations(&self) -> Vec<Annot> {
        (0..self.page_count())
            .flat_map(|i| self.page_annotations(i).unwrap_or_default())
            .collect()
    }

    /// The annotations on one page (zero-based), in the file's order (the order they are
    /// drawn in).
    pub fn page_annotations(&self, page_index: usize) -> Result<Vec<Annot>> {
        let page = self.page_id_at(page_index)?;
        let (vd, _, _) = self.vis_down(page);
        let to_vis = mx::inv(vd).ok_or_else(|| anyhow!("degenerate page transform"))?;
        Ok(self
            .annot_refs(page)
            .into_iter()
            .filter_map(|id| self.annot(id, page_index, to_vis))
            .collect())
    }

    /// Remove the annotations `pick` chooses, with the replies to them and their popups.
    /// Locked ones are removed too; the app checks [`Annot::locked`] first. Returns how
    /// many were removed, replies included.
    pub fn remove_annotations(&mut self, pick: impl Fn(&Annot) -> bool) -> Result<usize> {
        let all = self.annotations();
        let mut gone: HashSet<ObjectId> = all.iter().filter(|a| pick(a)).map(|a| a.id).collect();
        loop {
            let before = gone.len();
            for a in &all {
                if a.reply_to.is_some_and(|r| gone.contains(&r)) {
                    gone.insert(a.id);
                }
            }
            if gone.len() == before {
                break;
            }
        }
        if gone.is_empty() {
            return Ok(0);
        }
        for page in self.page_ids() {
            let Some(refs) = self.annots_array(page) else {
                continue;
            };
            let kept: Vec<Object> = refs
                .iter()
                .filter(|o| {
                    o.as_reference()
                        .map_or(true, |id| !gone.contains(&id) && !self.popup_of(id, &gone))
                })
                .cloned()
                .collect();
            if kept.len() < refs.len() {
                let pd = self.doc.get_dictionary_mut(page)?;
                if kept.is_empty() {
                    pd.remove(b"Annots");
                } else {
                    pd.set("Annots", kept);
                }
            }
        }
        Ok(gone.len())
    }

    /// Draw the annotations into the pages' content as they look now and remove them, so
    /// they can no longer be changed or removed. `pages` (zero-based) limits this to some
    /// pages. Links, form fields, file attachments, media and redaction marks are kept;
    /// replies and popups go without being drawn, as do hidden annotations. Annotations
    /// without a drawing get MPE's first. Returns how many were drawn.
    pub fn flatten_annotations(&mut self, pages: Option<&[usize]>) -> Result<usize> {
        let ids = self.page_ids();
        let pages = self.page_list(pages)?;
        let on: Vec<ObjectId> = pages.iter().map(|&i| ids[i]).collect();
        let missing = self.missing_appearances(&on);
        self.draw_annotations(&missing);
        let mut drawn = 0;
        for page in on {
            drawn += self.flatten_page(page, flat_annot, None, "MpeAnnot", &mut HashSet::new())?;
        }
        Ok(drawn)
    }

    /// The annotations on `pages` that MPE can draw and that have no drawing of their own
    /// (no `/AP /N`). Hidden ones and replies are left out.
    pub(crate) fn missing_appearances(&self, pages: &[ObjectId]) -> Vec<ObjectId> {
        let mut out = Vec::new();
        for &page in pages {
            for id in self.annot_refs(page) {
                let Ok(d) = self.doc.get_dictionary(id) else {
                    continue;
                };
                let drawn = d
                    .get(b"Subtype")
                    .and_then(|o| o.as_name())
                    .is_ok_and(|s| DRAWN.contains(&s));
                let has_ap = d
                    .get(b"AP")
                    .ok()
                    .and_then(|o| self.dict_of(o))
                    .is_some_and(|ap| ap.has(b"N"));
                // Replies are listed, not drawn.
                let shown = self.flags(d) & (HIDDEN | NO_VIEW) == 0 && !d.has(b"IRT");
                if drawn && !has_ap && shown {
                    out.push(id);
                }
            }
        }
        out
    }

    /// The page's `/Annots` array, resolved.
    fn annots_array(&self, page: ObjectId) -> Option<Vec<Object>> {
        self.doc
            .get_dictionary(page)
            .ok()?
            .get(b"Annots")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .cloned()
    }

    /// The annotations a page refers to (direct dictionaries, which the format doesn't
    /// allow, are skipped).
    pub(crate) fn annot_refs(&self, page: ObjectId) -> Vec<ObjectId> {
        self.annots_array(page)
            .unwrap_or_default()
            .iter()
            .take(MAX_ANNOTS)
            .filter_map(|o| o.as_reference().ok())
            .collect()
    }

    fn flags(&self, d: &Dictionary) -> i64 {
        d.get(b"F")
            .ok()
            .and_then(|o| self.int_value(o))
            .unwrap_or(0)
    }

    /// Whether `id` is a popup belonging to one of `gone`.
    fn popup_of(&self, id: ObjectId, gone: &HashSet<ObjectId>) -> bool {
        let Ok(d) = self.doc.get_dictionary(id) else {
            return false;
        };
        d.get(b"Subtype").and_then(|o| o.as_name()).ok() == Some(b"Popup")
            && d.get(b"Parent")
                .and_then(|o| o.as_reference())
                .is_ok_and(|p| gone.contains(&p))
    }

    fn annot(&self, id: ObjectId, page: usize, to_vis: mx::M) -> Option<Annot> {
        let d = self.doc.get_dictionary(id).ok()?;
        let kind = AnnotKind::of(d.get(b"Subtype").ok()?.as_name().ok()?)?;
        let rect = self.rect(d.get(b"Rect").ok()?)?;
        let text = |key: &[u8]| {
            d.get(key)
                .ok()
                .and_then(|o| self.text_value(o))
                .unwrap_or_default()
        };
        let date = |key: &[u8]| {
            d.get(key)
                .ok()
                .and_then(|o| self.string_bytes(o))
                .and_then(|b| pdf_date(&decode_text(&b)))
        };
        let markup = matches!(
            kind,
            AnnotKind::Highlight
                | AnnotKind::Underline
                | AnnotKind::StrikeOut
                | AnnotKind::Squiggly
        );
        let quads = if markup {
            self.numbers(d.get(b"QuadPoints").ok())
                .unwrap_or_default()
                .as_chunks::<8>()
                .0
                .iter()
                .map(|q| {
                    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                    for &[x, y] in q.as_chunks::<2>().0 {
                        let (x, y) = mx::apply(to_vis, x, y);
                        (b[0], b[1]) = (b[0].min(x), b[1].min(y));
                        (b[2], b[3]) = (b[2].max(x), b[3].max(y));
                    }
                    b
                })
                .collect()
        } else {
            Vec::new()
        };
        let text_style = (kind == AnnotKind::TextBox).then(|| {
            let da = d
                .get(b"DA")
                .ok()
                .and_then(|o| self.string_bytes(o))
                .unwrap_or_default();
            let (_, size, color) = parse_da(&da);
            let color: Vec<f32> = color
                .map(|o| {
                    o.operands
                        .iter()
                        .filter_map(|x| x.as_float().ok())
                        .collect()
                })
                .unwrap_or_default();
            TextStyle {
                size: if size > 0.0 { size } else { 12.0 },
                color: rgb(&color),
                fill: self.colour(d, b"C").map(|c| rgb(&c)),
                border: self.border(d).0,
            }
        });
        let shape = matches!(
            kind,
            AnnotKind::Square | AnnotKind::Circle | AnnotKind::Line | AnnotKind::Ink
        )
        .then(|| ShapeLook {
            fill: matches!(kind, AnnotKind::Square | AnnotKind::Circle)
                .then(|| self.colour(d, b"IC").map(|c| rgb(&c)))
                .flatten(),
            width: self.border(d).0,
            arrow: self.line_ends(d).iter().any(|e| e.as_slice() != b"None"),
        });
        let to_shown = |v: Vec<f32>| -> Vec<(f32, f32)> {
            v.as_chunks::<2>()
                .0
                .iter()
                .map(|&[x, y]| mx::apply(to_vis, x, y))
                .collect()
        };
        let paths = match kind {
            AnnotKind::Line => self.numbers(d.get(b"L").ok()).into_iter().collect(),
            AnnotKind::PolyLine => self.numbers(d.get(b"Vertices").ok()).into_iter().collect(),
            AnnotKind::Ink => d
                .get(b"InkList")
                .ok()
                .and_then(|o| self.doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_array().ok())
                .map(|list| {
                    list.iter()
                        .take(MAX_ANNOTS)
                        .filter_map(|s| self.numbers(Some(s)))
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        }
        .into_iter()
        .map(to_shown)
        .collect();
        Some(Annot {
            id,
            page,
            kind,
            bbox: mx::transform_rect(to_vis, rect),
            quads,
            color: self.colour(d, b"C").map(|c| rgb(&c)),
            opacity: d
                .get(b"CA")
                .ok()
                .and_then(|o| self.num(o))
                .map_or(1.0, |a| a.clamp(0.0, 1.0)),
            contents: text(b"Contents"),
            author: text(b"T"),
            subject: text(b"Subj"),
            modified: date(b"M")
                .or_else(|| date(b"CreationDate"))
                .unwrap_or_default(),
            flags: self.flags(d),
            reply_to: d.get(b"IRT").and_then(|o| o.as_reference()).ok(),
            has_appearance: d
                .get(b"AP")
                .ok()
                .and_then(|o| self.dict_of(o))
                .is_some_and(|ap| ap.has(b"N")),
            text_style,
            shape,
            paths,
        })
    }
}

/// What flattening does with each annotation: draw the marks, drop replies and the
/// popups of drawn annotations, keep the rest.
fn flat_annot(pdf: &Pdf, d: &Dictionary) -> Flat {
    fn sub(d: &Dictionary) -> Option<&[u8]> {
        d.get(b"Subtype").and_then(|o| o.as_name()).ok()
    }
    let marks = |s: Option<&[u8]>| {
        s.is_some_and(|s| DRAWN.contains(&s) || matches!(s, b"Stamp" | b"Caret"))
    };
    match sub(d) {
        Some(b"Popup") => {
            let parent = d
                .get(b"Parent")
                .and_then(|o| o.as_reference())
                .ok()
                .and_then(|p| pdf.doc.get_dictionary(p).ok());
            if parent.is_some_and(|p| marks(sub(p))) {
                Flat::Drop
            } else {
                Flat::Keep
            }
        }
        s if marks(s) && d.has(b"IRT") => Flat::Drop,
        s if marks(s) => Flat::Draw,
        _ => Flat::Keep,
    }
}

/// A colour as RGB: grey and CMYK are converted simply.
fn rgb(c: &[f32]) -> [f32; 3] {
    match *c {
        [g] => [g; 3],
        [c, m, y, k] => [
            (1.0 - c) * (1.0 - k),
            (1.0 - m) * (1.0 - k),
            (1.0 - y) * (1.0 - k),
        ],
        [r, g, b, ..] => [r, g, b],
        [] | [_, _] => [0.0; 3],
    }
}

/// A PDF date (`D:YYYYMMDDHHmmSS…`) as `YYYY-MM-DD HH:MM`; the time is left out when
/// the date has none.
fn pdf_date(s: &str) -> Option<String> {
    let s = s.strip_prefix("D:").unwrap_or(s);
    let digits: String = s
        .chars()
        .take_while(char::is_ascii_digit)
        .take(12)
        .collect();
    if digits.len() < 8 {
        return None;
    }
    let mut out = format!("{}-{}-{}", &digits[..4], &digits[4..6], &digits[6..8]);
    if digits.len() >= 12 {
        out += &format!(" {}:{}", &digits[8..10], &digits[10..12]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(
            pdf_date("D:20240305142233+01'00'").as_deref(),
            Some("2024-03-05 14:22")
        );
        assert_eq!(pdf_date("20240305").as_deref(), Some("2024-03-05"));
        assert_eq!(pdf_date("D:2024"), None);
    }

    #[test]
    fn kinds_round_trip_by_name() {
        for k in [
            AnnotKind::Highlight,
            AnnotKind::Note,
            AnnotKind::TextBox,
            AnnotKind::Ink,
        ] {
            assert_eq!(AnnotKind::parse(k.name()), k);
        }
        assert_eq!(AnnotKind::parse("HIGHLIGHT"), AnnotKind::Highlight);
        assert_eq!(
            AnnotKind::parse("FileAttachment"),
            AnnotKind::Other("FileAttachment".into())
        );
        assert_eq!(AnnotKind::TextBox.label(), "Text box");
        assert_eq!(AnnotKind::Highlight.label(), "Highlight");
    }

    #[test]
    fn colours_become_rgb() {
        assert_eq!(rgb(&[0.5]), [0.5; 3]);
        assert_eq!(rgb(&[0.0, 0.0, 1.0, 0.0]), [1.0, 1.0, 0.0]);
    }
}
