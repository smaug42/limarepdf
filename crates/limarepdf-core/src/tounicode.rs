//! A tolerant reader for ToUnicode CMaps: the map from a font's character codes to the
//! text they stand for. Only the mappings (bfchar and bfrange) are read; the rest of
//! the CMap program is skipped, so maps laid out differently from the usual template
//! (as LibreOffice writes them, for one) still work.

use std::collections::HashMap;

/// Most codes one range may map, and most mappings read, against hostile files.
const MAX_RANGE: u32 = 65_536;
const MAX_ENTRIES: usize = 1 << 20;
/// Most bytes the mappings may take, each counted with its entry's own size, and the
/// longest destination a CMap may have (the PDF specification's limit, 512 bytes).
const MAX_BYTES: usize = 32 << 20;
const MAX_DST: usize = 512;
const ENTRY: usize = std::mem::size_of::<(u32, String)>();

#[derive(Debug, Default)]
pub(crate) struct ToUnicode {
    map: HashMap<u32, String>,
}

impl ToUnicode {
    /// Read the mappings of a CMap program. `None` when it has none.
    pub fn parse(data: &[u8]) -> Option<Self> {
        let toks = tokens(data);
        let mut m = Mappings::default();
        let mut i = 0;
        'read: while i < toks.len() {
            match &toks[i] {
                Tok::Word(w) if *w == b"beginbfchar" => {
                    i += 1;
                    while let (Some(Tok::Hex(src)), Some(Tok::Hex(dst))) =
                        (toks.get(i), toks.get(i + 1))
                    {
                        if !m.put(code(src), &units(dst)) {
                            break 'read;
                        }
                        i += 2;
                    }
                }
                Tok::Word(w) if *w == b"beginbfrange" => {
                    i += 1;
                    while let (Some(Tok::Hex(lo)), Some(Tok::Hex(hi))) =
                        (toks.get(i), toks.get(i + 1))
                    {
                        let (lo, hi) = (code(lo), code(hi));
                        let hi = hi.min(lo.saturating_add(MAX_RANGE - 1));
                        match toks.get(i + 2) {
                            // Consecutive codes: the last UTF-16 unit counts up. A destination
                            // too long skips the whole range.
                            Some(Tok::Hex(dst)) => {
                                let base = units(dst);
                                if base.len() <= MAX_DST / 2 {
                                    for (k, c) in (lo..=hi).enumerate() {
                                        let mut u = base.clone();
                                        if let Some(last) = u.last_mut() {
                                            *last = last.wrapping_add(k as u16);
                                        }
                                        if !m.put(c, &u) {
                                            break 'read;
                                        }
                                    }
                                }
                                i += 3;
                            }
                            // One string per code.
                            Some(Tok::Open) => {
                                let mut j = i + 3;
                                let mut c = lo;
                                while let Some(Tok::Hex(dst)) = toks.get(j) {
                                    if c <= hi && !m.put(c, &units(dst)) {
                                        break 'read;
                                    }
                                    c = c.saturating_add(1);
                                    j += 1;
                                }
                                if matches!(toks.get(j), Some(Tok::Close)) {
                                    j += 1;
                                }
                                i = j;
                            }
                            _ => break,
                        }
                    }
                }
                _ => i += 1,
            }
        }
        let map = m.map;
        (!map.is_empty()).then_some(Self { map })
    }

    /// The text a code stands for, if the map has it.
    pub fn get(&self, code: u32) -> Option<&str> {
        self.map.get(&code).map(String::as_str)
    }
}

/// The mappings read so far, within `MAX_ENTRIES` and `MAX_BYTES`.
#[derive(Default)]
struct Mappings {
    map: HashMap<u32, String>,
    read: usize,
    bytes: usize,
}

impl Mappings {
    /// Maps `code` to the text of the UTF-16 units `dst`, unless it is longer than a CMap
    /// allows. False once a limit is reached: nothing more is read.
    fn put(&mut self, code: u32, dst: &[u16]) -> bool {
        if self.read == MAX_ENTRIES {
            return false;
        }
        self.read += 1;
        if dst.len() > MAX_DST / 2 {
            return true;
        }
        let text = text(dst);
        let cost = ENTRY + text.len();
        if self.bytes + cost > MAX_BYTES {
            return false;
        }
        self.bytes += cost;
        self.map.insert(code, text);
        true
    }
}

enum Tok<'a> {
    Hex(Vec<u8>),
    Word(&'a [u8]),
    Open,
    Close,
}

fn is_delim(b: u8) -> bool {
    b.is_ascii_whitespace() || b"<>[]()/%{}\0".contains(&b)
}

/// Split a CMap program into hex strings, words (names, numbers, operators) and array
/// brackets. Dictionaries, literal strings and comments are skipped.
fn tokens(data: &[u8]) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        match data[i] {
            b'%' => {
                while i < data.len() && data[i] != b'\n' && data[i] != b'\r' {
                    i += 1;
                }
            }
            b'<' if data.get(i + 1) == Some(&b'<') => i += 2,
            b'>' if data.get(i + 1) == Some(&b'>') => i += 2,
            b'<' => {
                let end = data[i + 1..]
                    .iter()
                    .position(|&b| b == b'>')
                    .map_or(data.len(), |p| i + 1 + p);
                out.push(Tok::Hex(hex(&data[i + 1..end])));
                i = end + 1;
            }
            b'[' => {
                out.push(Tok::Open);
                i += 1;
            }
            b']' => {
                out.push(Tok::Close);
                i += 1;
            }
            b'(' => {
                let mut depth = 0;
                while i < data.len() {
                    match data[i] {
                        b'\\' => i += 1,
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                i += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            b if is_delim(b) && b != b'/' => i += 1,
            _ => {
                let start = i;
                i += 1;
                while i < data.len() && !is_delim(data[i]) {
                    i += 1;
                }
                out.push(Tok::Word(&data[start..i]));
            }
        }
    }
    out
}

/// The bytes of a hex string; spaces are ignored and an odd last digit is followed by 0.
fn hex(s: &[u8]) -> Vec<u8> {
    let digits: Vec<u8> = s
        .iter()
        .filter_map(|&c| (c as char).to_digit(16).map(|d| d as u8))
        .collect();
    digits
        .chunks(2)
        .map(|p| p[0] << 4 | p.get(1).copied().unwrap_or(0))
        .collect()
}

/// A code's value, big-endian.
fn code(b: &[u8]) -> u32 {
    b.iter().take(4).fold(0, |v, &x| v << 8 | x as u32)
}

/// UTF-16BE units; a lone byte is one unit.
fn units(b: &[u8]) -> Vec<u16> {
    if b.len() == 1 {
        return vec![b[0] as u16];
    }
    b.chunks(2)
        .map(|p| (p[0] as u16) << 8 | p.get(1).copied().unwrap_or(0) as u16)
        .collect()
}

/// The text of UTF-16 units, a CJK radical read as the ideograph it looks like (PB-57,
/// `radicals.rs`).
fn text(u: &[u16]) -> String {
    (String::from_utf16_lossy(u).chars())
        .map(crate::radicals::ideograph)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_usual_template() {
        let m = ToUnicode::parse(
            b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
              /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
              /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
              1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n\
              2 beginbfchar\n<0003> <0020>\n<0011> <00660069>\nendbfchar\n\
              2 beginbfrange\n<0024> <0026> <0041>\n<0030> <0031> [<03B1> <D835DC00>]\nendbfrange\n\
              endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
        )
        .unwrap();
        assert_eq!(m.get(3), Some(" "));
        assert_eq!(m.get(0x11), Some("fi"));
        assert_eq!(m.get(0x25), Some("B"));
        assert_eq!(m.get(0x26), Some("C"));
        assert_eq!(m.get(0x30), Some("α"));
        assert_eq!(m.get(0x31), Some("\u{1D400}"));
        assert_eq!(m.get(0x27), None);
    }

    #[test]
    fn reads_compact_layouts() {
        // As LibreOffice writes it: no spaces before << or between names.
        let m = ToUnicode::parse(
            b"/CIDInit/ProcSet findresource begin\n12 dict begin\nbegincmap\n\
              /CIDSystemInfo<<\n/Registry (Adobe)\n/Ordering (UCS)\n/Supplement 0\n>> def\n\
              /CMapName/Adobe-Identity-UCS def\n/CMapType 2 def\n\
              1 begincodespacerange\n<00> <FF>\nendcodespacerange\n\
              2 beginbfchar\n<01> <25CF>\n<02><25CB>\nendbfchar\n\
              endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
        )
        .unwrap();
        assert_eq!(m.get(1), Some("\u{25CF}"));
        assert_eq!(m.get(2), Some("\u{25CB}"));
    }

    #[test]
    fn rejects_empty_and_bounds_ranges() {
        assert!(ToUnicode::parse(b"begincmap endcmap").is_none());
        assert!(ToUnicode::parse(b"").is_none());
        let huge = b"1 beginbfrange <00000000> <FFFFFFFF> <0041> endbfrange";
        let m = ToUnicode::parse(huge).unwrap();
        assert_eq!(m.map.len(), MAX_RANGE as usize);
    }

    /// The bytes the map holds, counting each entry's own size.
    fn held(m: &ToUnicode) -> usize {
        m.map.values().map(|t| ENTRY + t.len()).sum()
    }

    #[test]
    fn destinations_longer_than_a_cmap_allows_are_skipped() {
        // A map of about 4 KB, one range to 1,000 letters, asked for 64 MiB of text (E8).
        let long = format!(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
             1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n\
             1 beginbfrange\n<0000> <FFFF> <{}>\nendbfrange\nendcmap\nend\nend\n",
            "0041".repeat(1000)
        );
        assert!(long.len() < 4200);
        assert!(ToUnicode::parse(long.as_bytes()).is_none());
        // 512 bytes is the longest a destination may be; one past it is skipped.
        let at = |n: usize| format!("<{}>", "0041".repeat(n));
        let m = ToUnicode::parse(
            format!(
                "3 beginbfchar <01> {} <02> {} <03> <0042> endbfchar\n\
                 1 beginbfrange <10> <11> [{} {}] endbfrange",
                at(256),
                at(257),
                at(257),
                at(1)
            )
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(m.get(1).map(str::len), Some(256));
        assert_eq!(m.get(2), None);
        assert_eq!(m.get(3), Some("B"));
        assert_eq!(m.get(0x10), None);
        assert_eq!(m.get(0x11), Some("A"));
    }

    #[test]
    fn bounds_the_bytes_mapped() {
        // Sixteen ranges of 65,536 four-byte codes, each to 256 letters: 256 MiB of text.
        let ranges: String = (0..16)
            .map(|r| format!("<{r:04X}0000> <{r:04X}FFFF> <{}>\n", "0041".repeat(256)))
            .collect();
        let m =
            ToUnicode::parse(format!("16 beginbfrange\n{ranges}endbfrange").as_bytes()).unwrap();
        assert!(held(&m) <= MAX_BYTES, "{} bytes", held(&m));
        assert!(held(&m) > MAX_BYTES - 1024, "{} bytes", held(&m));
        assert_eq!(m.get(0).map(str::len), Some(256));
    }

    #[test]
    fn bounds_the_work_of_ranges_that_overlap() {
        // Each range maps the same 65,536 codes again, so the map never fills: a 16 MiB map
        // of them is about 52 billion codes to work through. Reading stops after
        // `MAX_ENTRIES` codes, so a later mapping is never reached.
        let ranges = "<0000> <FFFF> <0041>\n".repeat(64);
        let m = ToUnicode::parse(
            format!("64 beginbfrange\n{ranges}endbfrange\n1 beginbfchar <0000> <0042> endbfchar")
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(m.get(0), Some("A"));
        assert_eq!(m.map.len(), MAX_RANGE as usize);
    }
}
