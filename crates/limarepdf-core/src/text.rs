use crate::matrix::{self as mx, M};
use crate::objects::{span_boxes, Piece};
use crate::{words_text, Pdf, Result};
use lopdf::ObjectId;
use rayon::prelude::*;
use std::borrow::Cow;
use std::fmt;
use std::ops::Range;

/// How many characters of the page's text a snippet shows on each side of a match, at most.
pub const CONTEXT: usize = 40;

/// A match of a search: the page it starts on, the words around it, and where it is.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    /// 0-based page index. A phrase that runs across a page break starts on this page and
    /// goes on to the next.
    pub page: usize,
    pub snippet: Snippet,
    /// Where the match is: a box for each piece of each line it is on, in displayed-page
    /// points (y down), each with the page (0-based) it is on, in reading order.
    pub boxes: Vec<(usize, [f32; 4])>,
}

/// The words around a match, as the page has them: whole words, up to [`CONTEXT`]
/// characters before and after it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snippet {
    pub before: String,
    /// The match itself.
    pub found: String,
    pub after: String,
    /// Words were left out before `before`, or after `after`.
    pub cut_before: bool,
    pub cut_after: bool,
}

impl Snippet {
    /// The same snippet with at most `before` and `after` characters around the match, cut
    /// at words as the whole snippet is.
    pub fn around(&self, before: usize, after: usize) -> Snippet {
        let (b, cut_before) = tail(&self.before.chars().collect::<Vec<_>>(), before);
        let (a, cut_after) = head(&self.after.chars().collect::<Vec<_>>(), after);
        Snippet {
            before: b,
            found: self.found.clone(),
            after: a,
            cut_before: self.cut_before || cut_before,
            cut_after: self.cut_after || cut_after,
        }
    }
}

/// The words before the match, the match and the words after it, with "…" where words were
/// left out.
impl fmt::Display for Snippet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dots = |cut: bool| if cut { "…" } else { "" };
        write!(
            f,
            "{}{}{}{}{}",
            dots(self.cut_before),
            self.before,
            self.found,
            self.after,
            dots(self.cut_after)
        )
    }
}

/// How many matches a search found, in words: "No matches", "1 match", "3 matches".
pub fn match_count(n: usize) -> String {
    match n {
        0 => "No matches".into(),
        1 => "1 match".into(),
        n => format!("{n} matches"),
    }
}

/// The end of `text`, at most `n` characters, from the start of a word, and whether anything
/// was left out. A single word longer than `n` is cut inside.
fn tail(text: &[char], n: usize) -> (String, bool) {
    if text.len() <= n {
        return (text.iter().collect(), false);
    }
    let mut start = text.len() - n;
    if !text[start - 1].is_whitespace() {
        if let Some(k) = text[start..].iter().position(|c| c.is_whitespace()) {
            start += k;
        }
    }
    let kept = &text[start..];
    let lead = kept.iter().take_while(|c| c.is_whitespace()).count();
    (kept[lead..].iter().collect(), true)
}

/// The start of `text`, at most `n` characters, up to the end of a word, and whether anything
/// was left out. A single word longer than `n` is cut inside.
fn head(text: &[char], n: usize) -> (String, bool) {
    if text.len() <= n {
        return (text.iter().collect(), false);
    }
    let mut end = n;
    if !text[end].is_whitespace() {
        if let Some(k) = text[..end].iter().rposition(|c| c.is_whitespace()) {
            end = k;
        }
    }
    let kept = &text[..end];
    let trail = kept.iter().rev().take_while(|c| c.is_whitespace()).count();
    (kept[..kept.len() - trail].iter().collect(), true)
}

impl Pdf {
    /// Extract text of one page (0-based): its words in reading order, worked out from
    /// their positions, with spaces between words on a line and line breaks between
    /// lines. It is the same reading of the page that search highlights and text
    /// selection use. Empty when the page has no text (a scan) or cannot be read.
    pub fn page_text(&self, index: usize) -> String {
        self.page_words(index)
            .map(|w| words_text(&w))
            .unwrap_or_default()
    }

    /// Text of `pages` (0-based) in the order given, separated by form feeds as pdftotext
    /// does. Pages are extracted in parallel.
    pub fn pages_text(&self, pages: &[usize]) -> String {
        let texts: Vec<String> = pages.par_iter().map(|&p| self.page_text(p)).collect();
        texts.join("\u{c}")
    }

    /// Case-insensitive text search across all pages, in parallel. Runs of whitespace,
    /// line breaks and page breaks included, match one space, and words hyphenated at a
    /// line or page end are found whole (see [`joints`]). Each page is read once, for the
    /// matches, their snippets and their boxes together (see [`TextIndex`]).
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let q = Query::new(query);
        if q.is_empty() {
            return Ok(vec![]);
        }
        let mut index = TextIndex::new(self);
        let n = index.page_count();
        index.read(self, 0..n, &|| false);
        Ok(index.find(&q, 0..n, 0).hits)
    }

    /// Where `query` occurs on a page (0-based): one entry per match, with a box for each
    /// line it is on, in displayed-page points (y down), like [`PageObject::bbox`]. Case
    /// is ignored, and any run of whitespace, a line break included, matches one space,
    /// so a phrase broken across lines is found, as is a word hyphenated at a line end.
    /// Visible and invisible (OCR) text both count: the same text [`Pdf::search`] reads.
    /// Only the page is looked at: a phrase going on to the next page isn't found here.
    ///
    /// [`PageObject::bbox`]: crate::PageObject::bbox
    pub fn find_text_matches(&self, page_index: usize, query: &str) -> Result<Vec<Vec<[f32; 4]>>> {
        let q = Query::new(query);
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let page = PageText::read(self, self.page_id_at(page_index)?)?;
        Ok(find_on(page_index, &page, None, &q, 0)
            .into_iter()
            .map(|f| f.hit.boxes.into_iter().map(|(_, b)| b).collect::<Vec<_>>())
            .filter(|b| !b.is_empty())
            .collect())
    }
}

/// A search query as matched: see [`fold_query`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query(Vec<char>);

impl Query {
    pub fn new(query: &str) -> Self {
        Query(fold_query(query))
    }

    /// Nothing to look for: the query was empty or only spaces.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Where a character of a [`PageText`] came from: its text line and its place in that
/// line. The spaces put between words have none.
type Place = Option<(u32, u32)>;

/// A page's text as search reads it, read once and searched for any query: the page's
/// words in reading order (see [`Pdf::page_words`]) with a space between them, folded (see
/// [`fold`]), with the original characters, for snippets, and where each one is, for the
/// boxes.
#[derive(Clone, Debug, Default)]
pub struct PageText {
    text: Vec<char>,
    orig: Vec<char>,
    from: Vec<Place>,
    /// The spaces that join lines.
    breaks: Vec<usize>,
    /// Each text line's pieces.
    lines: Vec<Vec<Piece>>,
    /// From page user space to displayed-page points.
    to_vis: M,
}

impl PageText {
    /// The text of the page with object `page`.
    fn read(pdf: &Pdf, page: ObjectId) -> Result<Self> {
        let (words, lines, to_vis) = pdf.search_parts(page)?;
        let mut t = PageText {
            lines,
            to_vis,
            ..Default::default()
        };
        for (i, (w, li, start)) in words.iter().enumerate() {
            if i > 0 {
                if words[i - 1].0.line != w.line {
                    t.breaks.push(t.text.len());
                }
                t.text.push(' ');
                t.orig.push(' ');
                t.from.push(None);
            }
            for (k, c) in w.text.chars().enumerate() {
                t.text.push(fold(c));
                t.orig.push(c);
                t.from.push(Some((*li as u32, (start + k) as u32)));
            }
        }
        Ok(t)
    }

    /// About how much memory it holds, in bytes.
    fn bytes(&self) -> usize {
        use std::mem::size_of;
        let pieces: usize = self
            .lines
            .iter()
            .flatten()
            .map(|p| size_of::<Piece>() + p.spans.len() * size_of::<(f32, f32)>())
            .sum();
        size_of::<Self>()
            + self.text.len() * (2 * size_of::<char>() + size_of::<Place>())
            + self.breaks.len() * size_of::<usize>()
            + self.lines.len() * size_of::<Vec<Piece>>()
            + pieces
    }
}

/// A match found on a page: where it starts in the page's text, how many characters of the
/// next page's text it takes (a phrase across the page break), and the hit.
struct Found {
    start: usize,
    spill: usize,
    hit: SearchHit,
}

/// The matches of `q` that start on page `page`, whose text is `here`, at or after its
/// character `from`; a phrase running across the page break goes on into `next`, the next
/// page's text.
fn find_on(
    page: usize,
    here: &PageText,
    next: Option<&PageText>,
    q: &Query,
    from: usize,
) -> Vec<Found> {
    let n = here.text.len();
    // As much of the next page as a match may run into: the query's length, with room for
    // the hyphens and line breaks a match passes over.
    let head_len = match next {
        Some(t) if n > 0 => t.text.len().min(3 * q.0.len() + 4),
        _ => 0,
    };
    let next = next.filter(|_| head_len > 0);
    let (mut text, mut breaks) = (
        Cow::Borrowed(&here.text[..]),
        Cow::Borrowed(&here.breaks[..]),
    );
    if let Some(t) = next {
        // The page break joins the pages as a line break does.
        let mut joined = Vec::with_capacity(n + 1 + head_len);
        joined.extend_from_slice(&here.text);
        joined.push(' ');
        joined.extend_from_slice(&t.text[..head_len]);
        let mut b = here.breaks.clone();
        b.push(n);
        b.extend(
            t.breaks
                .iter()
                .filter(|&&k| k < head_len)
                .map(|k| k + n + 1),
        );
        (text, breaks) = (Cow::Owned(joined), Cow::Owned(b));
    }
    let skip = joints(&text, &breaks);
    // Each character's page and place; the space at the page break has none.
    let place = |i: usize| -> Option<(usize, (u32, u32))> {
        if i < n {
            here.from[i].map(|f| (page, f))
        } else {
            next.and_then(|t| *t.from.get(i.checked_sub(n + 1)?)?)
                .map(|f| (page + 1, f))
        }
    };
    let text_of = |p: usize| if p == page { Some(here) } else { next };
    find_all(&text, &skip, &q.0, from, n)
        .into_iter()
        .map(|(s, e)| {
            let spill = e.saturating_sub(n + 1);
            // The characters of the match on each text line it touches; a run on one line
            // includes the spaces between its words.
            let mut parts: Vec<(usize, u32, u32, u32)> = Vec::new();
            for (pg, (li, ci)) in (s..e).filter_map(place) {
                match parts.last_mut() {
                    Some(p) if p.0 == pg && p.1 == li && ci >= p.3 => p.3 = ci + 1,
                    _ => parts.push((pg, li, ci, ci + 1)),
                }
            }
            let boxes = parts
                .iter()
                .filter_map(|&(pg, li, a, b)| Some((pg, text_of(pg)?, li, a, b)))
                .flat_map(|(pg, t, li, a, b)| {
                    span_boxes(&t.lines[li as usize], a as usize, b as usize)
                        .into_iter()
                        .map(move |bb| (pg, mx::transform_rect(t.to_vis, bb)))
                })
                .collect();
            let (before, cut_before) = tail(&here.orig[..s], CONTEXT);
            let mut found: String = here.orig[s..e.min(n)].iter().collect();
            let (after, cut_after) = match next.filter(|_| e > n) {
                Some(t) => {
                    found.push(' ');
                    found.extend(&t.orig[..spill]);
                    head(&t.orig[spill..], CONTEXT)
                }
                None => head(&here.orig[e..], CONTEXT),
            };
            Found {
                start: s,
                spill,
                hit: SearchHit {
                    page,
                    snippet: Snippet {
                        before,
                        found,
                        after,
                        cut_before,
                        cut_after,
                    },
                    boxes,
                },
            }
        })
        .collect()
}

/// The search text of a document's pages ([`PageText`]), read as it is needed and kept for
/// the queries after, so each page is read once until it changes.
#[derive(Clone, Debug, Default)]
pub struct TextIndex {
    ids: Vec<ObjectId>,
    pages: Vec<Option<PageText>>,
}

/// What [`TextIndex::find`] found.
#[derive(Clone, Debug, Default)]
pub struct Matches {
    pub hits: Vec<SearchHit>,
    /// How many characters of the next page's text the last page's last match takes: where
    /// looking goes on, on the next page.
    pub spill: usize,
}

impl TextIndex {
    /// An index of `pdf`'s pages, none of them read yet.
    pub fn new(pdf: &Pdf) -> Self {
        let ids = pdf.page_ids();
        TextIndex {
            pages: vec![None; ids.len()],
            ids,
        }
    }

    /// Bring the index up to date after `pdf` changed: the pages in `changed` (0-based) are
    /// read again when they are next needed, or every page is when `changed` is `None` or
    /// the document's pages are no longer the same.
    pub fn update(&mut self, pdf: &Pdf, changed: Option<&[usize]>) {
        let ids = pdf.page_ids();
        match changed {
            Some(pages) if ids == self.ids => {
                for &p in pages {
                    if let Some(t) = self.pages.get_mut(p) {
                        *t = None;
                    }
                }
            }
            _ => *self = TextIndex::new(pdf),
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Whether page `page` (0-based) is read.
    pub fn has(&self, page: usize) -> bool {
        self.pages.get(page).is_some_and(Option::is_some)
    }

    /// Read the pages in `pages` (0-based) that aren't read yet, in parallel, from `pdf`, the
    /// document the index is of. A page that can't be read counts as having no text, as for
    /// [`Pdf::page_text`]. `stop` is asked before each page; once it says to stop, the pages
    /// left aren't read (those read by then are kept) and false is returned.
    pub fn read(
        &mut self,
        pdf: &Pdf,
        pages: Range<usize>,
        stop: &(dyn Fn() -> bool + Sync),
    ) -> bool {
        let end = pages.end.min(self.pages.len());
        let start = pages.start.min(end);
        let (ids, slots) = (&self.ids, &mut self.pages[start..end]);
        slots.par_iter_mut().enumerate().for_each(|(k, slot)| {
            if slot.is_none() && !stop() {
                *slot = Some(PageText::read(pdf, ids[start + k]).unwrap_or_default());
            }
        });
        self.pages[start..end].iter().all(Option::is_some)
    }

    /// Let go of the pages in `pages` (0-based): they are read again when next needed.
    pub fn forget(&mut self, pages: Range<usize>) {
        let end = pages.end.min(self.pages.len());
        for t in &mut self.pages[pages.start.min(end)..end] {
            *t = None;
        }
    }

    /// About how much memory the pages read hold, in bytes.
    pub fn bytes(&self) -> usize {
        self.pages.iter().flatten().map(PageText::bytes).sum()
    }

    /// The matches of `q` that start on the pages in `pages` (0-based), in order. Those pages,
    /// and the one after the last for a phrase across the page break, are to be read first
    /// ([`TextIndex::read`]); a page that isn't is passed over as if it had no text. `from`
    /// is where looking starts on the first page: the [`Matches::spill`] of the matches on
    /// the page before, so that a phrase across the page break isn't found twice.
    pub fn find(&self, q: &Query, pages: Range<usize>, from: usize) -> Matches {
        let empty = PageText::default();
        let text = |p: usize| self.pages.get(p).and_then(Option::as_ref);
        let end = pages.end.min(self.pages.len());
        let start = pages.start.min(end);
        let on =
            |p: usize, from: usize| find_on(p, text(p).unwrap_or(&empty), text(p + 1), q, from);
        let mut found: Vec<Vec<Found>> = (start..end)
            .into_par_iter()
            .map(|p| on(p, if p == start { from } else { 0 }))
            .collect();
        // A phrase running on to the next page takes its first characters, so that page's
        // matches start after them. A match found there among them is looked for again.
        let (mut spill, mut hits) = (0, Vec::new());
        for (k, p) in (start..end).enumerate() {
            if spill > 0 && found[k].first().is_some_and(|f| f.start < spill) {
                found[k] = on(p, spill);
            }
            spill = found[k].last().map_or(0, |f| f.spill);
            hits.extend(std::mem::take(&mut found[k]).into_iter().map(|f| f.hit));
        }
        Matches { hits, spill }
    }
}

/// One character as search compares it: lowercased.
pub(crate) fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// A search query as matched: whitespace runs as one space, lowercased.
pub(crate) fn fold_query(query: &str) -> Vec<char> {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .map(fold)
        .collect()
}

/// Which characters of folded page text a match may skip: soft hyphens, and a hyphen at
/// a line end together with the line break after it, when letters or digits sit on both
/// sides ("infor-" + "mation"). `breaks` are the indices of the spaces that join lines.
/// Matching the hyphen is still allowed, so "self-employed" finds "self-" + "employed".
pub(crate) fn joints(text: &[char], breaks: &[usize]) -> Vec<bool> {
    let mut skip: Vec<bool> = text.iter().map(|&c| c == '\u{ad}').collect();
    for &k in breaks {
        if k >= 2
            && k + 1 < text.len()
            && matches!(text[k - 1], '-' | '\u{ad}' | '\u{2010}')
            && text[k - 2].is_alphanumeric()
            && text[k + 1].is_alphanumeric()
        {
            skip[k - 1] = true;
            skip[k] = true;
        }
    }
    skip
}

/// Where the folded query `q` occurs in folded text, as non-overlapping `start..end`
/// ranges of `text` starting at or after `from` and before `until` (a match may end after
/// it); characters marked in `skip` are passed over when they don't match.
pub(crate) fn find_all(
    text: &[char],
    skip: &[bool],
    q: &[char],
    from: usize,
    until: usize,
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if q.is_empty() {
        return out;
    }
    let until = until.min(text.len());
    let mut i = from;
    while i < until {
        let (mut t, mut k) = (i, 0);
        while k < q.len() && t < text.len() {
            if text[t] == q[k] {
                k += 1;
            } else if !(skip[t] && t > i) {
                break;
            }
            t += 1;
        }
        if k == q.len() {
            out.push((i, t));
            i = t;
        } else {
            i += 1;
        }
    }
    out
}
