//! Font information needed to measure and decode text in page content.

use crate::std14;
use crate::tounicode::ToUnicode;
use hayro::hayro_interpret::hayro_cmap::{load_embedded, BfString, CMap, CMapName, CidFamily};
use lopdf::Encoding;
use lopdf::{Dictionary, Document, Object};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Helvetica,
    Times,
    Courier,
}

/// One of the 12 standard PDF fonts (no embedding needed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontChoice {
    pub family: Family,
    pub bold: bool,
    pub italic: bool,
}

impl Default for FontChoice {
    fn default() -> Self {
        Self {
            family: Family::Helvetica,
            bold: false,
            italic: false,
        }
    }
}

impl FontChoice {
    pub fn base_font(&self) -> &'static str {
        use Family::*;
        match (self.family, self.bold, self.italic) {
            (Helvetica, false, false) => "Helvetica",
            (Helvetica, true, false) => "Helvetica-Bold",
            (Helvetica, false, true) => "Helvetica-Oblique",
            (Helvetica, true, true) => "Helvetica-BoldOblique",
            (Times, false, false) => "Times-Roman",
            (Times, true, false) => "Times-Bold",
            (Times, false, true) => "Times-Italic",
            (Times, true, true) => "Times-BoldItalic",
            (Courier, false, false) => "Courier",
            (Courier, true, false) => "Courier-Bold",
            (Courier, false, true) => "Courier-Oblique",
            (Courier, true, true) => "Courier-BoldOblique",
        }
    }

    /// Best standard-font match for an arbitrary font name.
    pub fn from_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        let family = if n.contains("courier") || n.contains("mono") || n.contains("consolas") {
            Family::Courier
        } else if n.contains("times")
            || n.contains("serif") && !n.contains("sans")
            || n.contains("georgia")
            || n.contains("garamond")
            || n.contains("minion")
        {
            Family::Times
        } else {
            Family::Helvetica
        };
        Self {
            family,
            bold: n.contains("bold") || n.contains("black") || n.contains("heavy"),
            italic: n.contains("italic") || n.contains("oblique"),
        }
    }
}

/// Encode text for a WinAnsi simple font. Unrepresentable characters become `?`.
pub fn winansi_bytes(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| {
            let u = c as u32;
            match c {
                '€' => 0x80,
                '‚' => 0x82,
                '„' => 0x84,
                '…' => 0x85,
                '‘' => 0x91,
                '’' => 0x92,
                '“' => 0x93,
                '”' => 0x94,
                '•' => 0x95,
                '–' => 0x96,
                '—' => 0x97,
                '™' => 0x99,
                _ if (0x20..=0x7E).contains(&u) || (0xA0..=0xFF).contains(&u) => u as u8,
                _ => b'?',
            }
        })
        .collect()
}

/// The letters of a Latin ligature character (U+FB00..=U+FB06); other characters as
/// they are.
fn unligature(c: char) -> String {
    match c {
        '\u{FB00}' => "ff",
        '\u{FB01}' => "fi",
        '\u{FB02}' => "fl",
        '\u{FB03}' => "ffi",
        '\u{FB04}' => "ffl",
        '\u{FB05}' | '\u{FB06}' => "st",
        _ => return c.to_string(),
    }
    .to_string()
}

/// Largest ToUnicode CMap read, once decompressed.
const MAX_CMAP: usize = 16 << 20;
/// Largest CID (ISO 32000-1, Annex C: CIDs are at most 65,535).
const MAX_CID: u32 = 0xFFFF;
/// Most widths a `/W` array may set, counting repeats: four times every CID.
const MAX_W_STEPS: usize = 4 << 16;

pub(crate) struct FontCtx<'a> {
    pub type0: bool,
    pub base: String,
    enc: Option<Encoding<'a>>,
    /// The font's ToUnicode map, read by LimarePDF (lopdf rejects many that are laid out
    /// differently from the usual template). Consulted before `enc`.
    to_unicode: Option<ToUnicode>,
    /// A Type 0 font's CMap, from codes to CIDs: Identity, one of the predefined ones
    /// (hayro carries them), or the font's own. `None` when it can't be read.
    cmap: Option<Arc<CMap>>,
    /// The CMap's codespace, which says how many bytes each code takes (hayro-cmap reads it
    /// but keeps it to itself); empty when it can't be read.
    space: Vec<Space>,
    /// The CMap's codes are Unicode, as UTF-16 (`UniJIS-UCS2-H`, `UniGB-UTF16-V`, ...).
    utf16: bool,
    /// For a Type 0 font with no ToUnicode map: its character collection's map from CIDs
    /// to Unicode (`Adobe-Japan1-UCS2` and its kin), as pdf.js and hayro use.
    cid_text: Option<Arc<CMap>>,
    first: u32,
    widths: Vec<f32>,
    missing: f32,
    dw: f32,
    wmap: HashMap<u32, f32>,
    has_widths: bool,
    encoding_ok: bool,
    /// Glyph widths to 1/1000 em: 1, except for Type 3 fonts, whose widths are in their
    /// own glyph space (`/FontMatrix`).
    scale: f32,
    /// Whether [`FontCtx::codes`] splits strings the way the font does: always for simple
    /// fonts, and for Type 0 fonts whose CMap and its codespace were read or that are
    /// two-byte Unicode.
    pub codes_known: bool,
    /// A Type 0 font that writes downwards (its CMap's `/WMode` 1, or a predefined `-V`
    /// one). Its text is still measured as if it ran across.
    pub vertical: bool,
}

fn f(doc: &Document, o: &Object) -> Option<f32> {
    doc.dereference(o).ok()?.1.as_float().ok()
}

/// The character collection a CID font's `/CIDSystemInfo` names (`Adobe-Japan1`, ...).
fn system_family(doc: &Document, desc: &Dictionary) -> Option<CidFamily> {
    let (_, info) = doc.dereference(desc.get(b"CIDSystemInfo").ok()?).ok()?;
    let info = info.as_dict().ok()?;
    let s = |k: &[u8]| {
        let (_, o) = doc.dereference(info.get(k).ok()?).ok()?;
        o.as_str().ok().map(<[u8]>::to_vec)
    };
    Some(CidFamily::from_registry_ordering(
        &s(b"Registry")?,
        &s(b"Ordering")?,
    ))
}

/// A predefined CMap by name (`Identity-H`, `UniJIS-UCS2-H`, `90ms-RKSJ-H`, ...). Each is
/// read once and kept: there are fewer than 200, and reading the large ones for every font
/// on every page made text of CJK documents several times slower.
fn predefined(name: CMapName) -> Option<Arc<CMap>> {
    /// Each CMap read so far by name; `None` for one that couldn't be.
    type Read = HashMap<Vec<u8>, Option<Arc<CMap>>>;
    static READ: LazyLock<Mutex<Read>> = LazyLock::new(Default::default);
    if let CMapName::Custom(_) = name {
        return None;
    }
    let mut read = READ.lock().unwrap_or_else(|e| e.into_inner());
    let entry = read.entry(name.to_bytes().to_vec()).or_insert_with(|| {
        match name {
            CMapName::IdentityH => Some(CMap::identity_h()),
            CMapName::IdentityV => Some(CMap::identity_v()),
            n => CMap::parse(load_embedded(n)?, load_embedded),
        }
        .map(Arc::new)
    });
    entry.clone()
}

/// A codespace range of a CMap: codes of `n` bytes, each byte between `low`'s and `high`'s at
/// its place.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Space {
    n: usize,
    low: [u8; 4],
    high: [u8; 4],
}

impl Space {
    /// Whether `bytes`, at most `n` of them, are the start of a code in this range.
    fn starts(&self, bytes: &[u8]) -> bool {
        bytes.len() <= self.n
            && bytes
                .iter()
                .enumerate()
                .all(|(i, b)| (self.low[i]..=self.high[i]).contains(b))
    }
}

/// Most codespace ranges read for one CMap, those of the CMaps it uses included. The
/// predefined ones have a few; more makes the codespace count as unread.
const MAX_SPACES: usize = 256;
/// Most CMaps followed through `usecmap`, as hayro-cmap allows.
const MAX_USECMAP: u32 = 16;

/// `bytes` split into character codes of the lengths `len` gives, each with its bytes.
/// The flag marks the single-byte space (word spacing applies).
fn split(bytes: &[u8], len: impl Fn(&[u8]) -> usize) -> Vec<(u32, bool, &[u8])> {
    let mut out = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let (raw, tail) = rest.split_at(len(rest).clamp(1, rest.len()));
        let code = raw.iter().fold(0, |c, &b| c << 8 | b as u32);
        out.push((code, raw == b" ", raw));
        rest = tail;
    }
    out
}

/// How many bytes the code `bytes` starts with takes in CMap `m` with codespace `space`: as
/// many as `m` maps it with, else the length of the codespace range it is in. One that only
/// starts like the codes of a range takes that range's length; one that starts like none,
/// one byte. Readers differ here: poppler goes by the mappings alone, pdf.js by the
/// codespace alone (the standard's way), hayro by the mappings or one byte. This agrees
/// with them where they agree with each other, and with poppler and pdf.js on codes the
/// CMap leaves unmapped.
fn space_len(m: &CMap, space: &[Space], bytes: &[u8]) -> usize {
    let mut code = 0;
    for n in 1..=bytes.len().min(4) {
        code = code << 8 | bytes[n - 1] as u32;
        let start = &bytes[..n];
        let mapped = m.lookup_cid_code(code, n as u8).is_some();
        let whole = space.iter().any(|s| s.n == n && s.starts(start));
        if mapped || whole || !space.iter().any(|s| s.n > n && s.starts(start)) {
            return n;
        }
    }
    bytes.len().min(4)
}

/// How many bytes hayro draws the code `bytes` starts with as: the first length the CMap
/// maps. `None` when it maps none (hayro then draws one byte).
fn drawn_len(m: &CMap, bytes: &[u8]) -> Option<usize> {
    let mut code = 0;
    for (n, &b) in bytes.iter().take(4).enumerate() {
        code = code << 8 | b as u32;
        if m.lookup_cid_code(code, n as u8 + 1).is_some() {
            return Some(n + 1);
        }
    }
    None
}

/// The codespace of CMap `data`, with those of the CMaps it uses (`usecmap`), `depth` deep
/// in them. `data` is a CMap's text, or one of hayro-cmap's predefined CMaps in its own
/// binary form. Empty when none can be read.
fn codespace(data: &[u8], depth: u32) -> Vec<Space> {
    let read = if data.starts_with(b"bcmap") {
        binary_codespace(data, depth)
    } else {
        text_codespace(data, depth)
    };
    read.filter(|s| s.len() <= MAX_SPACES).unwrap_or_default()
}

/// The codespace of the predefined CMap `name` uses.
fn used_codespace(name: &[u8], depth: u32) -> Option<Vec<Space>> {
    if depth >= MAX_USECMAP {
        return None;
    }
    Some(codespace(
        load_embedded(CMapName::from_bytes(name))?,
        depth + 1,
    ))
}

/// A codespace range from its lowest and highest codes, which must be as long, one to four
/// bytes.
fn space_range(low: &[u8], high: &[u8]) -> Option<Space> {
    if low.len() != high.len() || !(1..=4).contains(&low.len()) {
        return None;
    }
    let mut s = Space {
        n: low.len(),
        low: [0; 4],
        high: [0; 4],
    };
    s.low[..s.n].copy_from_slice(low);
    s.high[..s.n].copy_from_slice(high);
    Some(s)
}

/// [`codespace`] of a CMap's text: the hexadecimal pairs between `begincodespacerange` and
/// `endcodespacerange`. Comments, strings and dictionaries are passed over.
fn text_codespace(data: &[u8], depth: u32) -> Option<Vec<Space>> {
    let regular = |c: u8| !c.is_ascii_whitespace() && !b"()<>[]{}/%".contains(&c);
    let mut out = Vec::new();
    let mut inside = false;
    let mut low: Option<Vec<u8>> = None;
    let mut name: &[u8] = &[];
    let mut i = 0;
    while i < data.len() {
        let c = data[i];
        let word = |i: usize| -> usize {
            let n = data[i..].iter().position(|&c| !regular(c));
            i + n.unwrap_or(data.len() - i)
        };
        match c {
            b'%' => {
                while i < data.len() && !matches!(data[i], b'\r' | b'\n') {
                    i += 1;
                }
            }
            b'(' => {
                // A string, with nested brackets and escapes.
                let mut level = 0usize;
                while i < data.len() {
                    match data[i] {
                        b'\\' => i += 1,
                        b'(' => level += 1,
                        b')' => level -= 1,
                        _ => {}
                    }
                    i += 1;
                    if level == 0 {
                        break;
                    }
                }
            }
            b'<' if data.get(i + 1) == Some(&b'<') => i += 2,
            b'<' => {
                let end = i + data[i..].iter().position(|&c| c == b'>')?;
                let digits: Vec<u8> = data[i + 1..end]
                    .iter()
                    .filter(|c| !c.is_ascii_whitespace())
                    .copied()
                    .collect();
                i = end + 1;
                if !inside {
                    continue;
                }
                if digits.len() % 2 == 1 {
                    return None;
                }
                let bytes = digits
                    .chunks(2)
                    .map(|d| u8::from_str_radix(std::str::from_utf8(d).ok()?, 16).ok())
                    .collect::<Option<Vec<u8>>>()?;
                match low.take() {
                    None => low = Some(bytes),
                    Some(l) => out.push(space_range(&l, &bytes)?),
                }
                if out.len() > MAX_SPACES {
                    return None;
                }
            }
            b'/' => {
                let end = word(i + 1);
                name = &data[i + 1..end];
                i = end;
            }
            c if regular(c) => {
                let end = word(i);
                match &data[i..end] {
                    b"begincodespacerange" => inside = true,
                    b"endcodespacerange" if low.is_some() => return None,
                    b"endcodespacerange" => inside = false,
                    b"usecmap" => out.extend(used_codespace(name, depth)?),
                    _ => {}
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    Some(out)
}

/// [`codespace`] of one of hayro-cmap's predefined CMaps, in its binary form (its own, as
/// hayro-cmap 0.1 reads it): a 10-byte header (`bcmap`, a version, the length), then
/// segments of a type byte, a 4-byte length counting these 5 bytes, and the data. A
/// codespace segment is a count, then each range as its length in bytes and its lowest and
/// highest codes; a `usecmap` segment is the used CMap's name.
fn binary_codespace(data: &[u8], depth: u32) -> Option<Vec<Space>> {
    const CODESPACE: u8 = 0x0C;
    const USECMAP: u8 = 0x09;
    let be32 = |at: usize| -> Option<usize> {
        let b: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
        Some(u32::from_be_bytes(b) as usize)
    };
    let end = be32(6)?.min(data.len());
    let mut out = Vec::new();
    let mut at = 10;
    while at < end {
        let kind = data[at];
        let len = be32(at + 1)?;
        let seg = data.get(at + 5..at.checked_add(len)?)?;
        match kind {
            CODESPACE => {
                let (&count, mut rest) = seg.split_first()?;
                for _ in 0..count {
                    let (&n, tail) = rest.split_first()?;
                    let n = n as usize;
                    let (low, tail) = tail.split_at_checked(n)?;
                    let (high, tail) = tail.split_at_checked(n)?;
                    out.push(space_range(low, high)?);
                    rest = tail;
                }
            }
            USECMAP => out.extend(used_codespace(seg, depth)?),
            _ => {}
        }
        at += len.max(5);
    }
    Some(out)
}

impl<'a> FontCtx<'a> {
    pub fn new(doc: &'a Document, dict: &'a Dictionary) -> Self {
        let name = |d: &Dictionary, k: &[u8]| -> Option<String> {
            let o = d.get(k).ok()?;
            let (_, o) = doc.dereference(o).ok()?;
            o.as_name()
                .ok()
                .map(|n| String::from_utf8_lossy(n).into_owned())
        };
        let type0 = name(dict, b"Subtype").as_deref() == Some("Type0");
        let base = name(dict, b"BaseFont").unwrap_or_default();
        let enc_name = name(dict, b"Encoding");
        let has_encoding = dict.get(b"Encoding").is_ok();
        let encoding_ok = match &enc_name {
            Some(n) => matches!(
                n.as_str(),
                "WinAnsiEncoding" | "StandardEncoding" | "MacRomanEncoding"
            ),
            None => !has_encoding, // no /Encoding: the font's built-in (Standard for the base 14)
        };
        let to_unicode = dict
            .get(b"ToUnicode")
            .ok()
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_stream().ok())
            .and_then(|s| s.get_plain_content_with_limit(MAX_CMAP).ok())
            .and_then(|data| ToUnicode::parse(&data));
        let mut ctx = Self {
            type0,
            base,
            enc: dict.get_font_encoding(doc).ok(),
            to_unicode,
            cmap: None,
            space: Vec::new(),
            utf16: false,
            cid_text: None,
            first: 0,
            widths: Vec::new(),
            missing: 0.0,
            dw: 1000.0,
            wmap: HashMap::new(),
            has_widths: false,
            encoding_ok,
            scale: 1.0,
            codes_known: !type0
                || enc_name.as_deref().is_some_and(|n| {
                    n.starts_with("Identity-") || n.contains("UCS2") || n.contains("UTF16")
                }),
            vertical: type0
                && match &enc_name {
                    Some(n) => n.ends_with("-V"),
                    None => {
                        dict.get(b"Encoding")
                            .ok()
                            .and_then(|o| doc.dereference(o).ok())
                            .and_then(|(_, o)| o.as_stream().ok())
                            .and_then(|s| f(doc, s.dict.get(b"WMode").ok()?))
                            == Some(1.0)
                    }
                },
        };
        if name(dict, b"Subtype").as_deref() == Some("Type3") {
            let fm = dict
                .get(b"FontMatrix")
                .ok()
                .and_then(|o| doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_array().ok())
                .and_then(|a| a.first())
                .and_then(|o| f(doc, o));
            if let Some(a) = fm.filter(|a| a.is_finite() && *a != 0.0) {
                ctx.scale = a * 1000.0;
            }
        }
        if type0 {
            let desc = dict
                .get(b"DescendantFonts")
                .ok()
                .and_then(|o| doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_array().ok())
                .and_then(|a| a.first())
                .and_then(|o| doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_dict().ok());
            if let Some(d) = desc {
                if let Some(v) = d.get(b"DW").ok().and_then(|o| f(doc, o)) {
                    ctx.dw = v;
                }
                if let Some(w) = d
                    .get(b"W")
                    .ok()
                    .and_then(|o| doc.dereference(o).ok())
                    .and_then(|(_, o)| o.as_array().ok())
                {
                    ctx.parse_w(doc, w);
                }
            }
            (ctx.cmap, ctx.space) = match dict
                .get(b"Encoding")
                .ok()
                .and_then(|o| doc.dereference(o).ok())
            {
                Some((_, Object::Name(n))) => {
                    let name = CMapName::from_bytes(n);
                    let space = load_embedded(name).map_or_else(Vec::new, |d| codespace(d, 0));
                    (predefined(name), space)
                }
                Some((_, Object::Stream(s))) => match s.get_plain_content_with_limit(MAX_CMAP) {
                    Ok(data) => (
                        CMap::parse(&data, load_embedded).map(Arc::new),
                        codespace(&data, 0),
                    ),
                    Err(_) => (None, Vec::new()),
                },
                _ => (None, Vec::new()),
            };
            ctx.codes_known |= ctx.cmap.is_some() && !ctx.space.is_empty();
            ctx.utf16 = enc_name.as_deref().is_some_and(|n| {
                n.starts_with("Uni") && (n.contains("-UCS2-") || n.contains("-UTF16-"))
            });
            if ctx.to_unicode.is_none() && !ctx.utf16 {
                let family = desc.and_then(|d| system_family(doc, d)).or_else(|| {
                    let cc = ctx.cmap.as_ref()?.metadata().character_collection.as_ref();
                    cc.map(|c| c.family.clone())
                });
                ctx.cid_text = family.and_then(|f| predefined(f.ucs2_cmap()?));
            }
        } else {
            ctx.first = dict
                .get(b"FirstChar")
                .ok()
                .and_then(|o| f(doc, o))
                .unwrap_or(0.0) as u32;
            if let Some(a) = dict
                .get(b"Widths")
                .ok()
                .and_then(|o| doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_array().ok())
            {
                ctx.widths = a.iter().map(|o| f(doc, o).unwrap_or(0.0)).collect();
                ctx.has_widths = !ctx.widths.is_empty();
            }
            if let Some(fd) = dict
                .get(b"FontDescriptor")
                .ok()
                .and_then(|o| doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_dict().ok())
            {
                ctx.missing = fd
                    .get(b"MissingWidth")
                    .ok()
                    .and_then(|o| f(doc, o))
                    .unwrap_or(0.0);
            }
        }
        ctx
    }

    /// Read a CID font's `/W` array into `wmap`. CIDs past [`MAX_CID`] are left out, so
    /// the map stays within 65,536 entries, and the work within [`MAX_W_STEPS`].
    fn parse_w(&mut self, doc: &Document, w: &[Object]) {
        let mut left = MAX_W_STEPS;
        let mut i = 0;
        while i < w.len() && left > 0 {
            let Some(c1) = f(doc, &w[i]) else { break };
            // Saturates: negative and NaN are 0, huge is past MAX_CID.
            let c1 = c1 as u32;
            match w
                .get(i + 1)
                .and_then(|o| doc.dereference(o).ok())
                .map(|(_, o)| o)
            {
                Some(Object::Array(a)) => {
                    let room = (MAX_CID + 1).saturating_sub(c1) as usize;
                    for (k, o) in a.iter().take(room.min(left)).enumerate() {
                        left -= 1;
                        if let Some(v) = f(doc, o) {
                            self.wmap.insert(c1 + k as u32, v);
                        }
                    }
                    i += 2;
                }
                Some(_) => {
                    let (Some(c2), Some(v)) = (
                        w.get(i + 1).and_then(|o| f(doc, o)),
                        w.get(i + 2).and_then(|o| f(doc, o)),
                    ) else {
                        break;
                    };
                    for c in (c1..=(c2 as u32).min(MAX_CID)).take(left) {
                        left -= 1;
                        self.wmap.insert(c, v);
                    }
                    i += 3;
                }
                None => break,
            }
        }
    }

    /// The CID a Type 0 font's code of `len` bytes stands for. Without a CMap, the code
    /// itself.
    fn cid(&self, code: u32, len: usize) -> Option<u32> {
        match &self.cmap {
            Some(m) => m.lookup_cid_code(code, u8::try_from(len).ok()?),
            None => Some(code),
        }
    }

    /// Glyph width in 1/1000 em of character code `code`, `len` bytes long.
    pub fn width(&self, code: u32, len: usize) -> f32 {
        if self.type0 {
            let w = self.cid(code, len).and_then(|c| self.wmap.get(&c));
            return w.copied().unwrap_or(self.dw);
        }
        if self.has_widths {
            return code
                .checked_sub(self.first)
                .and_then(|i| self.widths.get(i as usize))
                .copied()
                .unwrap_or(self.missing)
                * self.scale;
        }
        // No widths in the document: a standard font. Symbol and ZapfDingbats are
        // measured by code, the others by the character shown.
        if let Some(w) = std14::symbolic_width(&self.base, code) {
            return w;
        }
        let c = self.decode_raw(code, &[code as u8]).chars().next();
        c.and_then(|c| std14::width(&self.base, c)).unwrap_or(556.0)
    }

    /// Split a string into character codes, each with its bytes. The flag marks the
    /// single-byte space (word spacing applies).
    pub fn codes<'s>(&self, bytes: &'s [u8]) -> Vec<(u32, bool, &'s [u8])> {
        split(bytes, |rest| self.code_len(rest))
    }

    /// Split a string into character codes as LimarePDF draws it (hayro splits by the
    /// CMap's mappings alone), as [`FontCtx::codes`] does.
    pub fn drawn_codes<'s>(&self, bytes: &'s [u8]) -> Vec<(u32, bool, &'s [u8])> {
        match self.cmap.as_ref().filter(|_| self.type0 && !self.utf16) {
            Some(m) => split(bytes, |rest| drawn_len(m, rest).unwrap_or(1)),
            None => self.codes(bytes),
        }
    }

    /// Whether LimarePDF draws string `bytes` with the characters where
    /// [`FontCtx::codes`] reads them. hayro, which draws, takes a code the CMap doesn't map
    /// as one byte, whatever the codespace says, so the characters after it are drawn
    /// elsewhere. Where the two agree, poppler and pdf.js read the string the same way too,
    /// unless the CMap breaks its own codespace.
    pub fn drawn_alike(&self, bytes: &[u8]) -> bool {
        let Some(m) = self.cmap.as_ref() else {
            return true;
        };
        if !self.type0 || self.utf16 || self.space.is_empty() {
            return true;
        }
        let mut rest = bytes;
        while !rest.is_empty() {
            let n = space_len(m, &self.space, rest);
            if drawn_len(m, rest) != Some(n) {
                return false;
            }
            rest = &rest[n..];
        }
        true
    }

    /// How many bytes the code `bytes` starts with takes: one in a simple font; in a
    /// Type 0 font, two, or four for a UTF-16 surrogate pair, or as the CMap and its
    /// codespace say ([`space_len`]). Without a codespace, as many as the CMap maps, or one
    /// (as hayro does).
    fn code_len(&self, bytes: &[u8]) -> usize {
        if !self.type0 {
            return 1;
        }
        match &self.cmap {
            Some(m) if !self.utf16 && !self.space.is_empty() => space_len(m, &self.space, bytes),
            Some(m) if !self.utf16 => drawn_len(m, bytes).unwrap_or(1),
            _ if self.utf16 && bytes.len() >= 4 && (0xD8..=0xDB).contains(&bytes[0]) => 4,
            _ => bytes.len().min(2),
        }
    }

    /// The text one character code stands for: from the ToUnicode map, else from the
    /// encoding. `raw` is the code's bytes in the string. Ligatures come out as their
    /// letters ("ﬁ" as "fi"), so that searches find the words they are in.
    pub fn decode_code(&self, code: u32, raw: &[u8]) -> String {
        let s = self.decode_raw(code, raw);
        if s.chars().any(|c| ('\u{FB00}'..='\u{FB06}').contains(&c)) {
            return s.chars().map(unligature).collect();
        }
        s
    }

    fn decode_raw(&self, code: u32, raw: &[u8]) -> String {
        if let Some(s) = self.to_unicode.as_ref().and_then(|m| m.get(code)) {
            return s.to_owned();
        }
        if self.utf16 {
            let units: Vec<u16> = raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| u16::from_be_bytes(c))
                .collect();
            let mut s = String::from_utf16_lossy(&units);
            if raw.len() % 2 == 1 {
                s.push('\u{FFFD}');
            }
            return s;
        }
        if let Some(m) = &self.cid_text {
            match self
                .cid(code, raw.len())
                .and_then(|c| m.lookup_bf_string(c))
            {
                Some(BfString::Char(c)) => return c.to_string(),
                Some(BfString::String(s)) => return s,
                None => {}
            }
        }
        if let Some(e) = &self.enc {
            if let Ok(s) = e.bytes_to_string(raw) {
                if !s.is_empty() || raw.is_empty() {
                    return s;
                }
            }
        }
        if self.type0 {
            return "\u{FFFD}".repeat(raw.len() / 2);
        }
        raw.iter().map(|&b| b as char).collect()
    }

    /// True when text can be re-encoded for this very font with plain ASCII:
    /// a simple, non-subset font with a standard encoding.
    pub fn reusable(&self) -> bool {
        let subset = self.base.len() > 7
            && self.base.as_bytes()[6] == b'+'
            && self.base[..6].chars().all(|c| c.is_ascii_uppercase());
        !self.type0
            && !subset
            && self.encoding_ok
            && !self.base.is_empty()
            && !self.base.contains('+')
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    /// A Type 0 font whose descendant has `/W` `w` and `/DW` 500.
    fn widths(w: Vec<Object>) -> (Document, Dictionary) {
        let mut doc = Document::with_version("1.7");
        let desc = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType2", "DW" => 500, "W" => w,
        });
        let font = dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "F",
            "Encoding" => "Identity-H", "DescendantFonts" => vec![desc.into()],
        };
        (doc, font)
    }

    #[test]
    fn cid_widths_are_read() {
        let (doc, font) = widths(vec![
            10.into(),
            vec![600.into(), 700.into()].into(),
            20.into(),
            22.into(),
            300.into(),
        ]);
        let ctx = FontCtx::new(&doc, &font);
        let got: Vec<f32> = [9, 10, 11, 12, 20, 22, 23]
            .map(|c| ctx.width(c, 2))
            .to_vec();
        assert_eq!(got, [500.0, 600.0, 700.0, 500.0, 300.0, 300.0, 500.0]);
    }

    #[test]
    fn cid_widths_past_the_largest_cid_are_left_out() {
        // A start past u32 (saturates, no overflow), ranges over every CID many times
        // and a list running past the last CID.
        let mut w = vec![
            1e12.into(),
            vec![600.into(), 700.into()].into(),
            65_535.into(),
            vec![800.into(), 900.into()].into(),
        ];
        for _ in 0..1000 {
            w.extend([0.into(), 4_000_000_000i64.into(), 400.into()]);
        }
        let (doc, font) = widths(w);
        let ctx = FontCtx::new(&doc, &font);
        // Set by the list, then by the first ranges, before the work limit stops them.
        assert_eq!(ctx.width(65_535, 2), 400.0);
        assert_eq!(ctx.width(u32::MAX, 4), 500.0);
        assert!(ctx.wmap.len() <= MAX_CID as usize + 1, "{}", ctx.wmap.len());
        assert!(ctx.wmap.keys().all(|&c| c <= MAX_CID));
    }

    #[test]
    fn vertical_writing_is_told_by_the_cmap() {
        let (mut doc, mut font) = widths(vec![]);
        let vertical = |doc: &Document, font: &Dictionary| FontCtx::new(doc, font).vertical;
        assert!(!vertical(&doc, &font));
        font.set("Encoding", "UniJIS-UCS2-V");
        assert!(vertical(&doc, &font));
        for (mode, down) in [(0, false), (1, true)] {
            let cmap = doc.add_object(lopdf::Stream::new(
                dictionary! { "Type" => "CMap", "WMode" => mode },
                Vec::new(),
            ));
            font.set("Encoding", cmap);
            assert_eq!(vertical(&doc, &font), down, "WMode {mode}");
        }
        // Only Type 0 fonts write downwards.
        let simple = dictionary! { "Subtype" => "Type1", "Encoding" => "Identity-V" };
        assert!(!vertical(&doc, &simple));
    }

    fn range(low: &[u8], high: &[u8]) -> Space {
        space_range(low, high).unwrap()
    }

    #[test]
    fn every_predefined_cmap_has_a_codespace() {
        let names = "83pv-RKSJ-H 90ms-RKSJ-H 90ms-RKSJ-V 90msp-RKSJ-H 90msp-RKSJ-V \
            90pv-RKSJ-H Add-RKSJ-H Add-RKSJ-V B5pc-H B5pc-V CNS-EUC-H CNS-EUC-V ETen-B5-H \
            ETen-B5-V ETenms-B5-H ETenms-B5-V EUC-H EUC-V Ext-RKSJ-H Ext-RKSJ-V GB-EUC-H \
            GB-EUC-V GBK-EUC-H GBK-EUC-V GBK2K-H GBK2K-V GBKp-EUC-H GBKp-EUC-V GBpc-EUC-H \
            GBpc-EUC-V H HKscs-B5-H HKscs-B5-V Identity-H Identity-V KSC-EUC-H KSC-EUC-V \
            KSCms-UHC-H KSCms-UHC-HW-H KSCms-UHC-HW-V KSCms-UHC-V KSCpc-EUC-H UniCNS-UCS2-H \
            UniCNS-UCS2-V UniCNS-UTF16-H UniCNS-UTF16-V UniGB-UCS2-H UniGB-UCS2-V \
            UniGB-UTF16-H UniGB-UTF16-V UniJIS-UCS2-H UniJIS-UCS2-HW-H UniJIS-UCS2-HW-V \
            UniJIS-UCS2-V UniJIS-UTF16-H UniJIS-UTF16-V UniKS-UCS2-H UniKS-UCS2-V \
            UniKS-UTF16-H UniKS-UTF16-V V";
        let none: Vec<&str> = names
            .split_whitespace()
            .filter(|n| {
                used_codespace(n.as_bytes(), 0)
                    .unwrap_or_default()
                    .is_empty()
            })
            .collect();
        assert!(none.is_empty(), "{none:?}");
        // Shift-JIS: one byte, two from 0x8140, one for half-width katakana, two again.
        let sjis = used_codespace(b"90ms-RKSJ-H", 0).unwrap();
        assert!(
            sjis.contains(&range(&[0x81, 0x40], &[0x9F, 0xFC])),
            "{sjis:?}"
        );
        let m = predefined(CMapName::from_bytes(b"90ms-RKSJ-H")).unwrap();
        assert_eq!(space_len(&m, &sjis, b"A"), 1);
        assert_eq!(space_len(&m, &sjis, &[0x93, 0x8C]), 2);
        // Vertical uses horizontal's.
        assert_eq!(used_codespace(b"90ms-RKSJ-V", 0).unwrap(), sjis);
        let identity = used_codespace(b"Identity-H", 0).unwrap();
        assert_eq!(identity, [range(&[0, 0], &[0xFF, 0xFF])]);
    }

    #[test]
    fn a_cmap_s_own_codespace_is_read() {
        let text =
            b"%!PS-Adobe-3.0 Resource-CMap\n% begincodespacerange <00> <FF> (in a comment)\n\
            /CIDSystemInfo << /Registry (Adobe) /Ordering (begincodespacerange <) >> def\n\
            2 begincodespacerange <00> <80> < 81 40 > <9F FC> endcodespacerange\n\
            1 begincidchar <41> 34 endcidchar";
        let got = codespace(text, 0);
        assert_eq!(
            got,
            [range(&[0], &[0x80]), range(&[0x81, 0x40], &[0x9F, 0xFC])]
        );
        // With the ranges of the predefined CMap it uses.
        let used = codespace(
            b"/90ms-RKSJ-H usecmap 1 begincodespacerange <F040> <F9FC> endcodespacerange",
            0,
        );
        let sjis = used_codespace(b"90ms-RKSJ-H", 0).unwrap();
        assert_eq!(used.len(), sjis.len() + 1);
        assert!(used.contains(&range(&[0xF0, 0x40], &[0xF9, 0xFC])));
        // Ranges that can't be read leave no codespace: lengths that differ, five bytes,
        // a range left open, a CMap that isn't known.
        for bad in [
            &b"1 begincodespacerange <00> <FFFF> endcodespacerange"[..],
            b"1 begincodespacerange <0000000000> <FFFFFFFFFF> endcodespacerange",
            b"1 begincodespacerange <00> endcodespacerange",
            b"/Unknown-H usecmap",
        ] {
            assert!(
                codespace(bad, 0).is_empty(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        assert!(codespace(b"1 begincidchar <41> 34 endcidchar", 0).is_empty());
    }

    #[test]
    fn codes_take_their_mapped_length_else_that_of_their_codespace_range() {
        let text = b"2 begincodespacerange <00> <80> <8140> <9FFC> endcodespacerange \
            3 begincidchar <41> 34 <8141> 36 <85> 37 endcidchar";
        let m = CMap::parse(text, load_embedded).unwrap();
        let space = codespace(text, 0);
        let len = |b: &[u8]| space_len(&m, &space, b);
        // In a range, mapped or not.
        assert_eq!(len(&[0x41, 0x42]), 1);
        assert_eq!(len(&[0x42, 0x41]), 1);
        assert_eq!(len(&[0x81, 0x41, 0x41]), 2);
        assert_eq!(len(&[0x81, 0x50, 0x41]), 2);
        // Starting like a two-byte code: two bytes.
        assert_eq!(len(&[0x81, 0x20, 0x41]), 2);
        // Starting like none: one byte. Mapped outside the codespace: as mapped.
        assert_eq!(len(&[0xF0, 0x41]), 1);
        assert_eq!(len(&[0x85, 0x41]), 1);
        assert_eq!(len(&[0x86, 0x41]), 2);
        // Cut off: what is left.
        assert_eq!(len(&[0x81]), 1);
    }
}
