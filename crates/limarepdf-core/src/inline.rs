//! Inline images (`BI … ID <data> EI`) cannot be round-tripped through lopdf's content parser, so they are
//! lifted out of the stream before parsing (replaced by a placeholder operator) and put back verbatim when
//! the operators are encoded again.

use lopdf::content::{Content, Operation};

const TAG: &str = "LIMAREINLINEIMG";

/// The inline images lifted out of one content stream.
#[derive(Debug)]
pub(crate) struct Inline {
    /// What every placeholder operator starts with: [`TAG`], followed by as many `K`s as it
    /// takes for it not to occur anywhere in the stream, so that no operator or text there
    /// can pass for a placeholder.
    tag: String,
    raws: Vec<Vec<u8>>,
}

impl Inline {
    /// The raw inline image (from `BI` to `EI`) a placeholder operator stands for.
    pub fn raw(&self, operator: &str) -> Option<&[u8]> {
        let code = operator.strip_prefix(self.tag.as_str())?;
        if code.is_empty() || !code.bytes().all(|b| (b'A'..=b'J').contains(&b)) {
            return None;
        }
        let digits: String = code.bytes().map(|b| (b - b'A' + b'0') as char).collect();
        self.raws
            .get(digits.parse::<usize>().ok()?)
            .map(Vec::as_slice)
    }

    /// `ops` as content stream bytes, each placeholder operator replaced by its raw inline
    /// image. Placeholders are recognised as operators only, never inside text.
    pub fn encode(&self, ops: Vec<Operation>) -> lopdf::Result<Vec<u8>> {
        if self.raws.is_empty() {
            return Content { operations: ops }.encode();
        }
        // Operators are separated by line breaks, as lopdf separates them.
        fn put(out: &mut Vec<u8>, bytes: &[u8]) {
            if !out.is_empty() {
                out.push(b'\n');
            }
            out.extend_from_slice(bytes);
        }
        let mut out = Vec::new();
        let mut run = Vec::new();
        for o in ops {
            match self.raw(&o.operator) {
                Some(raw) => {
                    if !run.is_empty() {
                        let operations = std::mem::take(&mut run);
                        put(&mut out, &Content { operations }.encode()?);
                    }
                    put(&mut out, raw);
                }
                None => run.push(o),
            }
        }
        if !run.is_empty() {
            put(&mut out, &Content { operations: run }.encode()?);
        }
        Ok(out)
    }
}

/// [`TAG`] with enough `K`s after it that it occurs nowhere in `d`: one more than the
/// longest run of them after any `TAG` there.
fn tag_for(d: &[u8]) -> String {
    let t = TAG.as_bytes();
    let longest = d
        .windows(t.len())
        .enumerate()
        .filter(|(_, w)| *w == t)
        .map(|(i, _)| d[i + t.len()..].iter().take_while(|&&b| b == b'K').count())
        .max();
    match longest {
        None => TAG.to_string(),
        Some(k) => format!("{TAG}{}", "K".repeat(k + 1)),
    }
}

fn ws(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}
fn delim(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// End of the string / hex string / name / comment / regular token starting at `i` (which is not whitespace).
fn token_end(d: &[u8], i: usize) -> usize {
    let n = d.len();
    match d[i] {
        b'(' => {
            let (mut depth, mut j) = (0i32, i);
            while j < n {
                match d[j] {
                    b'\\' => j += 1,
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            return (j + 1).min(n);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            n
        }
        b'<' if d.get(i + 1) == Some(&b'<') => i + 2,
        b'>' if d.get(i + 1) == Some(&b'>') => i + 2,
        b'<' => d[i..]
            .iter()
            .position(|&c| c == b'>')
            .map(|p| i + p + 1)
            .unwrap_or(n),
        b'%' => d[i..]
            .iter()
            .position(|&c| c == b'\n' || c == b'\r')
            .map(|p| i + p)
            .unwrap_or(n),
        b'/' => {
            let mut j = i + 1;
            while j < n && !ws(d[j]) && !delim(d[j]) {
                j += 1;
            }
            j
        }
        b')' | b'>' | b'[' | b']' | b'{' | b'}' => i + 1,
        _ => {
            let mut j = i;
            while j < n && !ws(d[j]) && !delim(d[j]) {
                j += 1;
            }
            j
        }
    }
}

fn num_after(dict: &[u8], keys: &[&str]) -> Option<usize> {
    let mut i = 0;
    let mut toks: Vec<&[u8]> = vec![];
    while i < dict.len() {
        if ws(dict[i]) {
            i += 1;
            continue;
        }
        let e = token_end(dict, i).max(i + 1);
        toks.push(&dict[i..e.min(dict.len())]);
        i = e;
    }
    for w in toks.windows(2) {
        if keys.iter().any(|k| w[0] == format!("/{k}").as_bytes()) {
            return std::str::from_utf8(w[1]).ok()?.parse().ok();
        }
    }
    None
}

fn dict_has(dict: &[u8], keys: &[&str]) -> bool {
    let mut i = 0;
    while i < dict.len() {
        if ws(dict[i]) {
            i += 1;
            continue;
        }
        let e = token_end(dict, i).max(i + 1);
        if keys
            .iter()
            .any(|k| &dict[i..e.min(dict.len())] == format!("/{k}").as_bytes())
        {
            return true;
        }
        i = e;
    }
    false
}

fn name_after(dict: &[u8], keys: &[&str]) -> Option<String> {
    let mut i = 0;
    let mut toks: Vec<String> = vec![];
    while i < dict.len() {
        if ws(dict[i]) {
            i += 1;
            continue;
        }
        let e = token_end(dict, i).max(i + 1);
        toks.push(String::from_utf8_lossy(&dict[i..e.min(dict.len())]).into_owned());
        i = e;
    }
    toks.windows(2)
        .find(|w| keys.iter().any(|k| w[0] == format!("/{k}")))
        .map(|w| w[1].clone())
}

fn find_ei(d: &[u8], from: usize) -> Option<usize> {
    let n = d.len();
    let mut p = from;
    while p + 2 < n + 1 {
        if p + 2 <= n
            && ws(d[p.saturating_sub(0)])
            && p + 3 <= n
            && &d[p + 1..p + 3] == b"EI"
            && (p + 3 == n || ws(d[p + 3]))
        {
            let tail = &d[(p + 3).min(n)..(p + 3 + 16).min(n)];
            if tail.iter().all(|&c| c < 128 && (c >= 32 || ws(c))) {
                return Some(p + 3);
            }
        }
        p += 1;
    }
    None
}

/// Replace every inline image by a placeholder operator. Returns the new stream and the raw images.
pub(crate) fn extract(d: &[u8]) -> Result<(Vec<u8>, Inline), String> {
    let n = d.len();
    let mut out = Vec::with_capacity(n);
    let tag = tag_for(d);
    let mut raws = vec![];
    let mut i = 0;
    while i < n {
        if ws(d[i]) {
            out.push(d[i]);
            i += 1;
            continue;
        }
        let e = token_end(d, i).max(i + 1).min(n);
        if &d[i..e] != b"BI" || (i > 0 && !ws(d[i - 1]) && !delim(d[i - 1])) {
            out.extend_from_slice(&d[i..e]);
            i = e;
            continue;
        }
        // inline image: find the ID token
        let mut j = e;
        let id_at = loop {
            while j < n && ws(d[j]) {
                j += 1;
            }
            if j >= n {
                return Err("inline image without ID".into());
            }
            let te = token_end(d, j).max(j + 1).min(n);
            if &d[j..te] == b"ID" {
                break j;
            }
            j = te;
        };
        let dict = &d[e..id_at];
        let mut d0 = id_at + 2;
        if d0 < n && d[d0] == b'\r' && d.get(d0 + 1) == Some(&b'\n') {
            d0 += 2;
        } else if d0 < n && ws(d[d0]) {
            d0 += 1;
        }
        // exact length when unfiltered and computable
        let mut end = None;
        if !dict_has(dict, &["F", "Filter"]) {
            let (w, h) = (
                num_after(dict, &["W", "Width"]),
                num_after(dict, &["H", "Height"]),
            );
            let mask = name_after(dict, &["IM", "ImageMask"])
                .map(|v| v == "true")
                .unwrap_or(false);
            let bpc = if mask {
                Some(1)
            } else {
                num_after(dict, &["BPC", "BitsPerComponent"])
            };
            let comps = if mask {
                Some(1)
            } else {
                match name_after(dict, &["CS", "ColorSpace"]).as_deref() {
                    Some("/G" | "/DeviceGray" | "/CalGray") => Some(1),
                    Some("/RGB" | "/DeviceRGB" | "/CalRGB") => Some(3),
                    Some("/CMYK" | "/DeviceCMYK") => Some(4),
                    Some("[") => Some(1),
                    _ => None,
                }
            };
            if let (Some(w), Some(h), Some(b), Some(c)) = (w, h, bpc, comps) {
                let len = h * (w * c * b).div_ceil(8);
                let mut k = d0 + len;
                if k <= n {
                    while k < n && ws(d[k]) {
                        k += 1;
                    }
                    if k + 2 <= n
                        && &d[k..k + 2] == b"EI"
                        && (k + 2 == n || ws(d[k + 2]) || delim(d[k + 2]))
                    {
                        end = Some(k + 2);
                    }
                }
            }
        }
        let end = match end.or_else(|| find_ei(d, d0.saturating_sub(1))) {
            Some(x) => x,
            None => return Err("inline image without EI".into()),
        };
        raws.push(d[i..end].to_vec());
        let code: String = (raws.len() - 1)
            .to_string()
            .bytes()
            .map(|b| (b - b'0' + b'A') as char)
            .collect();
        out.extend_from_slice(format!("\n{tag}{code}\n").as_bytes());
        i = end;
    }
    Ok((out, Inline { tag, raws }))
}

/// A raw inline image's dictionary entries (the text between `BI` and `ID`) and its data
/// (between `ID` and `EI`, without the whitespace that ends it).
pub(crate) fn split(raw: &[u8]) -> Option<(&[u8], &[u8])> {
    let n = raw.len();
    let mut j = 2.min(n);
    let id_at = loop {
        while j < n && ws(raw[j]) {
            j += 1;
        }
        if j >= n {
            return None;
        }
        let te = token_end(raw, j).max(j + 1).min(n);
        if &raw[j..te] == b"ID" {
            break j;
        }
        j = te;
    };
    let mut d0 = id_at + 2;
    if raw.get(d0) == Some(&b'\r') && raw.get(d0 + 1) == Some(&b'\n') {
        d0 += 2;
    } else if d0 < n && ws(raw[d0]) {
        d0 += 1;
    }
    let mut end = n.checked_sub(2).filter(|&e| raw[e..] == *b"EI")?;
    if end > d0 && ws(raw[end - 1]) {
        end -= 1;
    }
    Some((&raw[2..id_at], &raw[d0.min(end)..end]))
}
