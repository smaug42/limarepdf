//! A tolerant reader for ToUnicode CMaps: the map from a font's character codes to the
//! text they stand for. Only the mappings (bfchar and bfrange) are read; the rest of
//! the CMap program is skipped, so maps laid out differently from the usual template
//! (as LibreOffice writes them, for one) still work.

use std::collections::HashMap;

/// Most codes one range may map, and most mappings kept, against hostile files.
const MAX_RANGE: u32 = 65_536;
const MAX_ENTRIES: usize = 1 << 20;

#[derive(Debug, Default)]
pub(crate) struct ToUnicode {
    map: HashMap<u32, String>,
}

impl ToUnicode {
    /// Read the mappings of a CMap program. `None` when it has none.
    pub fn parse(data: &[u8]) -> Option<Self> {
        let toks = tokens(data);
        let mut map = HashMap::new();
        let put = |map: &mut HashMap<u32, String>, code: u32, text: String| {
            if map.len() < MAX_ENTRIES {
                map.insert(code, text);
            }
        };
        let mut i = 0;
        while i < toks.len() {
            match &toks[i] {
                Tok::Word(w) if *w == b"beginbfchar" => {
                    i += 1;
                    while let (Some(Tok::Hex(src)), Some(Tok::Hex(dst))) =
                        (toks.get(i), toks.get(i + 1))
                    {
                        put(&mut map, code(src), text(&units(dst)));
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
                            // Consecutive codes: the last UTF-16 unit counts up.
                            Some(Tok::Hex(dst)) => {
                                let base = units(dst);
                                for (k, c) in (lo..=hi).enumerate() {
                                    let mut u = base.clone();
                                    if let Some(last) = u.last_mut() {
                                        *last = last.wrapping_add(k as u16);
                                    }
                                    put(&mut map, c, text(&u));
                                }
                                i += 3;
                            }
                            // One string per code.
                            Some(Tok::Open) => {
                                let mut j = i + 3;
                                let mut c = lo;
                                while let Some(Tok::Hex(dst)) = toks.get(j) {
                                    if c <= hi {
                                        put(&mut map, c, text(&units(dst)));
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
        (!map.is_empty()).then_some(Self { map })
    }

    /// The text a code stands for, if the map has it.
    pub fn get(&self, code: u32) -> Option<&str> {
        self.map.get(&code).map(String::as_str)
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

fn text(u: &[u16]) -> String {
    String::from_utf16_lossy(u)
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
}
