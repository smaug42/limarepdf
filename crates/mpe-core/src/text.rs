use crate::{words_text, Pdf, Result};
use rayon::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// 0-based page index
    pub page: usize,
    pub snippet: String,
}

impl Pdf {
    /// Extract text of one page (0-based): its words in content order, which is usually
    /// reading order, with spaces between words on a line and line breaks between
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

    /// Case-insensitive text search across all pages, in parallel.
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        // Runs of whitespace match one space, as in the page text below.
        let q = query
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if q.is_empty() {
            return Ok(vec![]);
        }
        let n = self.page_count();
        let mut hits: Vec<SearchHit> = (0..n)
            .into_par_iter()
            .flat_map_iter(|p| {
                let text = self.page_text(p);
                let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                let lower = flat.to_lowercase();
                let mut out = Vec::new();
                let mut from = 0;
                // Only trust byte offsets when lowercasing preserved lengths.
                let same_len = lower.len() == flat.len();
                while let Some(pos) = lower[from..].find(&q) {
                    let at = from + pos;
                    let snippet = if same_len {
                        let s = floor_boundary(&flat, at.saturating_sub(30));
                        let e = floor_boundary(&flat, (at + q.len() + 30).min(flat.len()));
                        flat[s..e].to_string()
                    } else {
                        flat.chars().take(80).collect()
                    };
                    out.push(SearchHit { page: p, snippet });
                    from = at + q.len().max(1);
                }
                out.into_iter()
            })
            .collect();
        hits.sort_by_key(|h| h.page);
        Ok(hits)
    }
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
