use crate::{words_text, Pdf, Result};
use rayon::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// 0-based page index
    pub page: usize,
    pub snippet: String,
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
    /// line breaks included, match one space, and words hyphenated at a line end are
    /// found whole (see [`joints`]).
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let q = fold_query(query);
        if q.is_empty() {
            return Ok(vec![]);
        }
        let n = self.page_count();
        let mut hits: Vec<SearchHit> = (0..n)
            .into_par_iter()
            .flat_map_iter(|p| {
                let words = self.page_words(p).unwrap_or_default();
                let (text, breaks, orig) = search_text(&words);
                let skip = joints(&text, &breaks);
                find_all(&text, &skip, &q)
                    .into_iter()
                    .map(|(s, e)| SearchHit {
                        page: p,
                        snippet: orig[s.saturating_sub(30)..(e + 30).min(orig.len())]
                            .iter()
                            .collect(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        hits.sort_by_key(|h| h.page);
        Ok(hits)
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
/// ranges of `text`; characters marked in `skip` are passed over when they don't match.
pub(crate) fn find_all(text: &[char], skip: &[bool], q: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if q.is_empty() {
        return out;
    }
    let mut i = 0;
    while i < text.len() {
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

/// The text of a run of words for searching, folded, with the line-joining spaces' indices
/// and, for each character, the original character (for snippets).
fn search_text(words: &[crate::PageWord]) -> (Vec<char>, Vec<usize>, Vec<char>) {
    let (mut text, mut breaks, mut orig) = (Vec::new(), Vec::new(), Vec::new());
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            if words[i - 1].line != w.line {
                breaks.push(text.len());
            }
            text.push(' ');
            orig.push(' ');
        }
        for c in w.text.chars() {
            text.push(fold(c));
            orig.push(c);
        }
    }
    (text, breaks, orig)
}
