use crate::{Pdf, Result};
use rayon::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// 0-based page index
    pub page: usize,
    pub snippet: String,
}

impl Pdf {
    /// Extract text of one page (0-based). Returns an empty string when the page
    /// has no extractable text (scanned page, unsupported font encoding).
    pub fn page_text(&self, index: usize) -> String {
        self.doc
            .extract_text(&[index as u32 + 1])
            .unwrap_or_default()
    }

    /// Case-insensitive text search across all pages, in parallel.
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let q = query.trim().to_lowercase();
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
