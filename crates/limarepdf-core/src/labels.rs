//! Page labels: the page numbers a document shows, such as i, ii, 1, 2 or A-1.

use crate::doc::{decode_text, encode_text};
use crate::{Pdf, Result};
use anyhow::bail;
use lopdf::{Dictionary, Object, ObjectId, StringFormat};
use std::collections::{HashMap, HashSet};

/// Limits that keep a malformed or hostile number tree from running away.
const MAX_NODES: usize = 10_000;
const MAX_DEPTH: usize = 32;

/// How a label's number is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LabelStyle {
    Decimal,
    UpperRoman,
    LowerRoman,
    UpperLetters,
    LowerLetters,
}

impl LabelStyle {
    pub const ALL: [LabelStyle; 5] = [
        LabelStyle::Decimal,
        LabelStyle::LowerRoman,
        LabelStyle::UpperRoman,
        LabelStyle::LowerLetters,
        LabelStyle::UpperLetters,
    ];

    /// A sample of the style for menus.
    pub fn name(self) -> &'static str {
        match self {
            LabelStyle::Decimal => "1, 2, 3",
            LabelStyle::UpperRoman => "I, II, III",
            LabelStyle::LowerRoman => "i, ii, iii",
            LabelStyle::UpperLetters => "A, B, C",
            LabelStyle::LowerLetters => "a, b, c",
        }
    }

    fn code(self) -> &'static str {
        match self {
            LabelStyle::Decimal => "D",
            LabelStyle::UpperRoman => "R",
            LabelStyle::LowerRoman => "r",
            LabelStyle::UpperLetters => "A",
            LabelStyle::LowerLetters => "a",
        }
    }

    fn from_code(c: &[u8]) -> Option<Self> {
        Some(match c {
            b"D" => LabelStyle::Decimal,
            b"R" => LabelStyle::UpperRoman,
            b"r" => LabelStyle::LowerRoman,
            b"A" => LabelStyle::UpperLetters,
            b"a" => LabelStyle::LowerLetters,
            _ => return None,
        })
    }

    /// Write `n` (1 or more) in this style. Roman numbers past 4999 and letters past ZZZZZ… (100 letters) are
    /// written as plain numbers, so a hostile start number can't make huge labels.
    pub fn format(self, n: u32) -> String {
        let n = n.max(1);
        match self {
            LabelStyle::Decimal => n.to_string(),
            LabelStyle::UpperRoman | LabelStyle::LowerRoman if n < 5000 => {
                let r = roman(n);
                if self == LabelStyle::LowerRoman {
                    r.to_lowercase()
                } else {
                    r
                }
            }
            LabelStyle::UpperLetters | LabelStyle::LowerLetters if n <= 26 * 100 => {
                let base = if self == LabelStyle::UpperLetters {
                    b'A'
                } else {
                    b'a'
                };
                let c = (base + ((n - 1) % 26) as u8) as char;
                c.to_string().repeat(((n - 1) / 26 + 1) as usize)
            }
            _ => n.to_string(),
        }
    }
}

fn roman(mut n: u32) -> String {
    const PARTS: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (v, s) in PARTS {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}

/// Numbering from one page on, until the next range starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelRange {
    /// The range's first page (zero-based).
    pub page: usize,
    /// `None`: every page in the range is labelled with the prefix alone.
    pub style: Option<LabelStyle>,
    pub prefix: String,
    /// The number of the range's first page, 1 or more.
    pub first: u32,
}

impl LabelRange {
    /// Whether `next` is this range carried on `len` pages later, so it adds nothing.
    fn continues_as(&self, len: usize, next: &LabelRange) -> bool {
        let carried = u32::try_from(len)
            .ok()
            .and_then(|l| self.first.checked_add(l));
        self.style == next.style
            && self.prefix == next.prefix
            && (self.style.is_none() || carried == Some(next.first))
    }
}

impl Pdf {
    /// The page label ranges, sorted by page; empty when the document has none.
    pub fn label_ranges(&self) -> Vec<LabelRange> {
        let n = self.page_count();
        let mut nums = Vec::new();
        if let Some(root) = self
            .doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"PageLabels").ok())
        {
            let mut seen = HashSet::new();
            self.walk_numbers(root, 0, &mut seen, &mut nums);
        }
        let mut ranges: Vec<LabelRange> = nums
            .into_iter()
            .filter_map(|(k, d)| {
                let page = usize::try_from(k).ok().filter(|&p| p < n)?;
                let style = d
                    .get(b"S")
                    .ok()
                    .and_then(|o| o.as_name().ok())
                    .and_then(LabelStyle::from_code);
                let prefix = d
                    .get(b"P")
                    .ok()
                    .and_then(|o| o.as_str().ok())
                    .map(decode_text)
                    .unwrap_or_default();
                let first = d
                    .get(b"St")
                    .ok()
                    .and_then(|o| o.as_i64().ok())
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(1)
                    .max(1);
                Some(LabelRange {
                    page,
                    style,
                    prefix,
                    first,
                })
            })
            .collect();
        ranges.sort_by_key(|r| r.page);
        ranges.dedup_by_key(|r| r.page);
        ranges
    }

    /// Every page's label, or `None` when the document has no page labels. Pages before the first range are
    /// numbered plainly from 1.
    pub fn page_labels(&self) -> Option<Vec<String>> {
        let ranges = self.label_ranges();
        if ranges.is_empty() {
            return None;
        }
        Some(labels_of(&ranges, self.page_count()))
    }

    /// Replace the page labels. Pages before the first range are numbered plainly from 1; no ranges removes the
    /// labels.
    pub fn set_label_ranges(&mut self, ranges: &[LabelRange]) -> Result<()> {
        let n = self.page_count();
        if let Some(r) = ranges.iter().find(|r| r.page >= n) {
            bail!("page {} does not exist (document has {n})", r.page + 1);
        }
        let mut ranges = ranges.to_vec();
        ranges.sort_by_key(|r| r.page);
        // The last range given for a page wins.
        ranges.reverse();
        ranges.dedup_by_key(|r| r.page);
        ranges.reverse();
        self.write_ranges(ranges)
    }

    /// Number the pages from `range.page` on as it says, up to the next range that already starts after it.
    pub fn set_label_range(&mut self, range: LabelRange) -> Result<()> {
        let mut ranges = self.label_ranges();
        ranges.retain(|r| r.page != range.page);
        ranges.push(range);
        self.set_label_ranges(&ranges)
    }

    /// Write `ranges` (sorted, one per page, all within the document), dropping ranges that only carry on the one
    /// before and adding a plain range at the start when needed.
    fn write_ranges(&mut self, ranges: Vec<LabelRange>) -> Result<()> {
        if ranges.is_empty() {
            self.doc.catalog_mut()?.remove(b"PageLabels");
            return Ok(());
        }
        let mut kept: Vec<LabelRange> = Vec::new();
        if ranges[0].page > 0 {
            kept.push(LabelRange {
                page: 0,
                style: Some(LabelStyle::Decimal),
                prefix: String::new(),
                first: 1,
            });
        }
        for r in ranges {
            match kept.last() {
                Some(prev) if prev.continues_as(r.page - prev.page, &r) => {}
                _ => kept.push(r),
            }
        }
        let mut nums = Vec::new();
        for r in kept {
            let mut d = Dictionary::new();
            if let Some(s) = r.style {
                d.set("S", Object::Name(s.code().into()));
            }
            if !r.prefix.is_empty() {
                d.set(
                    "P",
                    Object::String(encode_text(&r.prefix), StringFormat::Literal),
                );
            }
            if r.first != 1 {
                d.set("St", r.first as i64);
            }
            nums.push(Object::Integer(r.page as i64));
            nums.push(Object::Dictionary(d));
        }
        let tree = self.doc.add_object(lopdf::dictionary! { "Nums" => nums });
        self.doc.catalog_mut()?.set("PageLabels", tree);
        Ok(())
    }

    /// Each page's numbering as (style, prefix, number), keyed by page object, or `None` without page labels.
    /// [`Pdf::relabel`] uses it to keep labels with their pages when pages move.
    pub(crate) fn label_specs(&self) -> Option<HashMap<ObjectId, Spec>> {
        let ranges = self.label_ranges();
        if ranges.is_empty() {
            return None;
        }
        let specs = specs_of(&ranges, self.page_count());
        Some(self.page_ids().into_iter().zip(specs).collect())
    }

    /// Label the pages, now in the order `ids`, as `specs` says each was labelled. New pages carry on the
    /// numbering of the page before them.
    pub(crate) fn relabel(
        &mut self,
        ids: &[ObjectId],
        specs: &HashMap<ObjectId, Spec>,
    ) -> Result<()> {
        let mut ranges: Vec<LabelRange> = Vec::new();
        let mut prev: Option<Spec> = None;
        for (page, id) in ids.iter().enumerate() {
            let carried = prev.clone().map(|(s, p, n)| (s, p, n.saturating_add(1)));
            let spec = specs.get(id).cloned().or(carried.clone()).unwrap_or((
                Some(LabelStyle::Decimal),
                String::new(),
                1,
            ));
            let same = carried.as_ref().is_some_and(|c| {
                c.0 == spec.0 && c.1 == spec.1 && (spec.0.is_none() || c.2 == spec.2)
            });
            if !same {
                ranges.push(LabelRange {
                    page,
                    style: spec.0,
                    prefix: spec.1.clone(),
                    first: spec.2,
                });
            }
            prev = Some(spec);
        }
        self.write_ranges(ranges)
    }

    fn walk_numbers(
        &self,
        node: &Object,
        depth: usize,
        seen: &mut HashSet<ObjectId>,
        out: &mut Vec<(i64, Dictionary)>,
    ) {
        if depth > MAX_DEPTH || seen.len() > MAX_NODES {
            return;
        }
        if let Object::Reference(id) = node {
            if !seen.insert(*id) {
                return;
            }
        }
        let Ok((_, Object::Dictionary(d))) = self.doc.dereference(node) else {
            return;
        };
        if let Ok(Object::Array(nums)) = d
            .get(b"Nums")
            .and_then(|o| self.doc.dereference(o).map(|(_, o)| o))
        {
            for pair in nums.chunks(2) {
                if let [k, v] = pair {
                    let dict = self
                        .doc
                        .dereference(v)
                        .ok()
                        .and_then(|(_, o)| o.as_dict().ok());
                    if let (Ok(k), Some(dict)) = (k.as_i64(), dict) {
                        out.push((k, dict.clone()));
                    }
                }
            }
        }
        if let Ok(Object::Array(kids)) = d
            .get(b"Kids")
            .and_then(|o| self.doc.dereference(o).map(|(_, o)| o))
        {
            for kid in kids {
                self.walk_numbers(kid, depth + 1, seen, out);
            }
        }
    }
}

/// A page's numbering: style, prefix and number.
pub(crate) type Spec = (Option<LabelStyle>, String, u32);

/// Each of `n` pages' numbering under `ranges` (sorted by page). Pages before the first range are numbered plainly.
fn specs_of(ranges: &[LabelRange], n: usize) -> Vec<Spec> {
    let mut out = Vec::with_capacity(n);
    let mut k = 0;
    for i in 0..n {
        while ranges.get(k).is_some_and(|r| r.page <= i) {
            k += 1;
        }
        let (start, spec) = match k.checked_sub(1).map(|k| &ranges[k]) {
            Some(r) => (r.page, (r.style, r.prefix.clone(), r.first)),
            None => (0, (Some(LabelStyle::Decimal), String::new(), 1)),
        };
        let off = u32::try_from(i - start).unwrap_or(u32::MAX);
        out.push((spec.0, spec.1, spec.2.saturating_add(off)));
    }
    out
}

fn labels_of(ranges: &[LabelRange], n: usize) -> Vec<String> {
    specs_of(ranges, n)
        .into_iter()
        .map(|(style, prefix, n)| match style {
            Some(s) => format!("{prefix}{}", s.format(n)),
            None => prefix,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles() {
        use LabelStyle::*;
        assert_eq!(LowerRoman.format(14), "xiv");
        assert_eq!(UpperRoman.format(1994), "MCMXCIV");
        assert_eq!(UpperRoman.format(9000), "9000");
        assert_eq!(UpperLetters.format(1), "A");
        assert_eq!(LowerLetters.format(28), "bb");
        assert_eq!(UpperLetters.format(5000), "5000");
        assert_eq!(Decimal.format(0), "1");
    }
}
