//! Font information needed to measure and decode text in page content.

use crate::overlay::HELV;
use lopdf::Encoding;
use lopdf::{Dictionary, Document, Object};
use std::collections::HashMap;

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

pub(crate) struct FontCtx<'a> {
    pub type0: bool,
    pub base: String,
    enc: Option<Encoding<'a>>,
    first: u32,
    widths: Vec<f32>,
    missing: f32,
    dw: f32,
    wmap: HashMap<u32, f32>,
    has_widths: bool,
    encoding_ok: bool,
}

fn f(doc: &Document, o: &Object) -> Option<f32> {
    doc.dereference(o).ok()?.1.as_float().ok()
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
        let mut ctx = Self {
            type0,
            base,
            enc: dict.get_font_encoding(doc).ok(),
            first: 0,
            widths: Vec::new(),
            missing: 0.0,
            dw: 1000.0,
            wmap: HashMap::new(),
            has_widths: false,
            encoding_ok,
        };
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

    fn parse_w(&mut self, doc: &Document, w: &[Object]) {
        let mut i = 0;
        while i < w.len() {
            let Some(c1) = f(doc, &w[i]) else { break };
            match w
                .get(i + 1)
                .and_then(|o| doc.dereference(o).ok())
                .map(|(_, o)| o)
            {
                Some(Object::Array(a)) => {
                    for (k, o) in a.iter().enumerate() {
                        if let Some(v) = f(doc, o) {
                            self.wmap.insert(c1 as u32 + k as u32, v);
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
                    for c in (c1 as u32)..=(c2 as u32).min(c1 as u32 + 70_000) {
                        self.wmap.insert(c, v);
                    }
                    i += 3;
                }
                None => break,
            }
        }
    }

    /// Glyph width in 1/1000 em.
    pub fn width(&self, code: u32) -> f32 {
        if self.type0 {
            return self.wmap.get(&code).copied().unwrap_or(self.dw);
        }
        if self.has_widths {
            return code
                .checked_sub(self.first)
                .and_then(|i| self.widths.get(i as usize))
                .copied()
                .unwrap_or(self.missing);
        }
        let b = self.base.to_ascii_lowercase();
        if b.contains("courier") {
            600.0
        } else if (32..=126).contains(&code) {
            let w = HELV[(code - 32) as usize] as f32;
            if b.contains("times") {
                w * 0.9
            } else {
                w
            }
        } else {
            556.0
        }
    }

    /// Split a string into character codes. The flag marks the single-byte space (word spacing applies).
    pub fn codes(&self, bytes: &[u8]) -> Vec<(u32, bool)> {
        if self.type0 {
            bytes
                .chunks(2)
                .map(|c| {
                    (
                        if c.len() == 2 {
                            (c[0] as u32) << 8 | c[1] as u32
                        } else {
                            c[0] as u32
                        },
                        false,
                    )
                })
                .collect()
        } else {
            bytes.iter().map(|&b| (b as u32, b == 32)).collect()
        }
    }

    pub fn decode(&self, bytes: &[u8]) -> String {
        if let Some(e) = &self.enc {
            if let Ok(s) = e.bytes_to_string(bytes) {
                if !s.is_empty() || bytes.is_empty() {
                    return s;
                }
            }
        }
        if self.type0 {
            return "\u{FFFD}".repeat(bytes.len() / 2);
        }
        bytes.iter().map(|&b| b as char).collect()
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
