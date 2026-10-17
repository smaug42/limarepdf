//! A small reader for the BER and DER encodings (ITU-T X.690) that signatures use: just
//! enough to walk CMS structures exactly as they are written. Nothing is re-encoded, so
//! bytes that a signature covers are checked as the signer wrote them.

use crate::Result;
use anyhow::{anyhow, bail};
use chrono::{DateTime, FixedOffset, NaiveDate};
use der::asn1::ObjectIdentifier;

/// How deeply indefinite-length elements may nest before the input is refused.
const MAX_DEPTH: usize = 64;

pub(crate) const INTEGER: u8 = 0x02;
pub(crate) const BIT_STRING: u8 = 0x03;
pub(crate) const OCTET_STRING: u8 = 0x04;
pub(crate) const NULL: u8 = 0x05;
pub(crate) const OID: u8 = 0x06;
pub(crate) const SEQUENCE: u8 = 0x30;
pub(crate) const SET: u8 = 0x31;
pub(crate) const UTC_TIME: u8 = 0x17;
pub(crate) const GENERALIZED_TIME: u8 = 0x18;

/// Context-specific tag `n`, constructed (`[n]` holding other elements).
pub(crate) const fn ctx(n: u8) -> u8 {
    0xa0 | n
}

/// Context-specific tag `n`, primitive (`[n] IMPLICIT` over a plain value).
pub(crate) const fn ctx_primitive(n: u8) -> u8 {
    0x80 | n
}

/// One element: its tag, everything as written, and its contents.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tlv<'a> {
    pub tag: u8,
    /// The whole element as written: tag, length and contents.
    pub raw: &'a [u8],
    /// The contents. For an indefinite length, without the end-of-contents marker.
    pub body: &'a [u8],
    pub indefinite: bool,
}

impl<'a> Tlv<'a> {
    /// The first element in `b`. Bytes after it are ignored; see [`Tlv::read_all`].
    pub fn read(b: &'a [u8]) -> Result<Self> {
        read_at(b, 0)
    }

    /// The single element that makes up all of `b`.
    pub fn read_all(b: &'a [u8]) -> Result<Self> {
        let t = Self::read(b)?;
        if t.raw.len() != b.len() {
            bail!("unexpected data after the end");
        }
        Ok(t)
    }

    /// Fails unless the tag is `tag`.
    pub fn expect(self, tag: u8, what: &str) -> Result<Self> {
        if self.tag != tag {
            bail!("{what}: expected tag {tag:#04x}, found {:#04x}", self.tag);
        }
        Ok(self)
    }

    pub fn constructed(&self) -> bool {
        self.tag & 0x20 != 0
    }

    /// The elements inside a constructed element, in order.
    pub fn children(&self) -> Result<Vec<Tlv<'a>>> {
        if !self.constructed() {
            bail!(
                "expected a constructed element, found tag {:#04x}",
                self.tag
            );
        }
        let mut out = Vec::new();
        let mut rest = self.body;
        while !rest.is_empty() {
            let t = Self::read(rest)?;
            rest = &rest[t.raw.len()..];
            out.push(t);
        }
        Ok(out)
    }

    /// An OBJECT IDENTIFIER's value.
    pub fn oid(&self) -> Result<ObjectIdentifier> {
        if self.tag != OID {
            bail!("expected an object identifier, found tag {:#04x}", self.tag);
        }
        ObjectIdentifier::from_bytes(self.body).map_err(|e| anyhow!("bad object identifier: {e}"))
    }

    /// An OCTET STRING's bytes. BER may split one into pieces (a constructed string);
    /// they are joined.
    pub fn octets(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.octets_into(&mut out, 0)?;
        Ok(out)
    }

    fn octets_into(&self, out: &mut Vec<u8>, depth: usize) -> Result<()> {
        match self.tag {
            OCTET_STRING => out.extend_from_slice(self.body),
            0x24 if depth < MAX_DEPTH => {
                for c in self.children()? {
                    if c.tag != OCTET_STRING && c.tag != 0x24 {
                        bail!("a split octet string holds tag {:#04x}", c.tag);
                    }
                    c.octets_into(out, depth + 1)?;
                }
            }
            t => bail!("expected an octet string, found tag {t:#04x}"),
        }
        Ok(())
    }

    /// A non-negative INTEGER that fits in 64 bits.
    pub fn uint(&self) -> Result<u64> {
        if self.tag != INTEGER {
            bail!("expected an integer, found tag {:#04x}", self.tag);
        }
        let b = self.body;
        if b.is_empty() || b[0] & 0x80 != 0 {
            bail!("expected a non-negative integer");
        }
        let b = if b.len() > 1 && b[0] == 0 { &b[1..] } else { b };
        if b.len() > 8 {
            bail!("integer too large");
        }
        Ok(b.iter().fold(0u64, |a, &x| (a << 8) | u64::from(x)))
    }

    /// A UTCTime or GeneralizedTime.
    pub fn time(&self) -> Result<DateTime<FixedOffset>> {
        let s = std::str::from_utf8(self.body).map_err(|_| anyhow!("time is not text"))?;
        match self.tag {
            UTC_TIME => parse_time(s, false),
            GENERALIZED_TIME => parse_time(s, true),
            t => bail!("expected a time, found tag {t:#04x}"),
        }
        .ok_or_else(|| anyhow!("unreadable time {s:?}"))
    }

    /// A string value (the kinds names and certificates use), if this is one.
    pub fn text(&self) -> Option<String> {
        match self.tag {
            // UTF8String, PrintableString, IA5String, VisibleString, NumericString
            0x0c | 0x13 | 0x16 | 0x1a | 0x12 => Some(String::from_utf8_lossy(self.body).into()),
            // TeletexString: in practice Latin-1.
            0x14 => Some(self.body.iter().map(|&b| char::from(b)).collect()),
            // BMPString: UTF-16BE.
            0x1e if self.body.len().is_multiple_of(2) => {
                let units = self
                    .body
                    .chunks(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]));
                Some(
                    char::decode_utf16(units)
                        .map(|c| c.unwrap_or('\u{FFFD}'))
                        .collect(),
                )
            }
            // UniversalString: UTF-32BE.
            0x1c if self.body.len().is_multiple_of(4) => Some(
                self.body
                    .chunks(4)
                    .map(|c| {
                        char::from_u32(u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
                            .unwrap_or('\u{FFFD}')
                    })
                    .collect(),
            ),
            _ => None,
        }
    }
}

fn read_at(b: &[u8], depth: usize) -> Result<Tlv<'_>> {
    if depth > MAX_DEPTH {
        bail!("nested too deeply");
    }
    let (&tag, &l0) = match b {
        [t, l, ..] => (t, l),
        _ => bail!("truncated"),
    };
    if tag & 0x1f == 0x1f {
        bail!("tag numbers above 30 are not used in signatures");
    }
    if tag == 0 {
        bail!("unexpected end-of-contents marker");
    }
    if l0 == 0x80 {
        if tag & 0x20 == 0 {
            bail!("indefinite length on a primitive element");
        }
        let mut pos = 2;
        loop {
            let rest = &b[pos..];
            if rest.starts_with(&[0, 0]) {
                return Ok(Tlv {
                    tag,
                    raw: &b[..pos + 2],
                    body: &b[2..pos],
                    indefinite: true,
                });
            }
            pos += read_at(rest, depth + 1)?.raw.len();
        }
    }
    let (len, header) = if l0 < 0x80 {
        (usize::from(l0), 2)
    } else {
        let n = usize::from(l0 & 0x7f);
        if n > 4 {
            bail!("length too long");
        }
        let bytes = b.get(2..2 + n).ok_or_else(|| anyhow!("truncated"))?;
        (
            bytes.iter().fold(0usize, |a, &x| (a << 8) | usize::from(x)),
            2 + n,
        )
    };
    let end = header
        .checked_add(len)
        .filter(|&e| e <= b.len())
        .ok_or_else(|| anyhow!("truncated"))?;
    Ok(Tlv {
        tag,
        raw: &b[..end],
        body: &b[header..end],
        indefinite: false,
    })
}

// ------------------------------------------------------------------ writing (DER)

/// One element in DER: `tag`, the length of `body`, then `body`.
pub(crate) fn der(tag: u8, body: &[u8]) -> Vec<u8> {
    let n = body.len();
    let mut out = Vec::with_capacity(n + 6);
    out.push(tag);
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let bytes = n.to_be_bytes();
        let skip = bytes.iter().take_while(|&&b| b == 0).count();
        out.push(0x80 | (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
    out.extend_from_slice(body);
    out
}

/// A SEQUENCE of elements already encoded.
pub(crate) fn der_seq(parts: &[&[u8]]) -> Vec<u8> {
    der(SEQUENCE, &parts.concat())
}

/// A SET OF elements already encoded, in the order DER requires.
pub(crate) fn der_set(mut items: Vec<Vec<u8>>) -> Vec<u8> {
    items.sort();
    der(SET, &items.concat())
}

pub(crate) fn der_uint(n: u64) -> Vec<u8> {
    let bytes = n.to_be_bytes();
    let skip = bytes.iter().take_while(|&&b| b == 0).count().min(7);
    let mut body = Vec::with_capacity(9);
    if bytes[skip] & 0x80 != 0 {
        body.push(0);
    }
    body.extend_from_slice(&bytes[skip..]);
    der(INTEGER, &body)
}

pub(crate) fn der_oid(o: &ObjectIdentifier) -> Vec<u8> {
    der(OID, o.as_bytes())
}

pub(crate) fn der_octets(b: &[u8]) -> Vec<u8> {
    der(OCTET_STRING, b)
}

/// UTCTime (`YYMMDDhhmm[ss]` + zone) or GeneralizedTime (`YYYYMMDDhhmmss[.f]` + zone),
/// the zone being `Z` or `+hhmm`/`-hhmm`. Times without a zone are refused: their meaning
/// depends on where they were written.
fn parse_time(s: &str, generalized: bool) -> Option<DateTime<FixedOffset>> {
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let (date, rest) = if generalized {
        let y: i32 = s.get(..4).filter(|d| digits(d))?.parse().ok()?;
        (y, s.get(4..)?)
    } else {
        let y: i32 = s.get(..2).filter(|d| digits(d))?.parse().ok()?;
        (if y >= 50 { 1900 + y } else { 2000 + y }, s.get(2..)?)
    };
    let two =
        |s: &str, i: usize| -> Option<u32> { s.get(i..i + 2).filter(|d| digits(d))?.parse().ok() };
    let (mo, d, h, mi) = (two(rest, 0)?, two(rest, 2)?, two(rest, 4)?, two(rest, 6)?);
    let mut rest = rest.get(8..)?;
    let mut sec = 0;
    if let Some(s) = two(rest, 0) {
        sec = s;
        rest = &rest[2..];
    } else if generalized {
        return None;
    }
    let mut nanos = 0;
    if generalized {
        if let Some(f) = rest.strip_prefix('.').or_else(|| rest.strip_prefix(',')) {
            let n = f.bytes().take_while(u8::is_ascii_digit).count();
            if n == 0 {
                return None;
            }
            let frac = &f[..n.min(9)];
            nanos = frac.parse::<u32>().ok()? * 10u32.pow(9 - frac.len() as u32);
            rest = &f[n..];
        }
    }
    let offset = match rest {
        "Z" => 0,
        z if z.len() == 5 && (z.starts_with('+') || z.starts_with('-')) => {
            let (oh, om) = (two(z, 1)?, two(z, 3)?);
            if oh > 23 || om > 59 {
                return None;
            }
            let secs = (oh * 3600 + om * 60) as i32;
            if z.starts_with('-') {
                -secs
            } else {
                secs
            }
        }
        _ => return None,
    };
    let naive = NaiveDate::from_ymd_opt(date, mo, d)?.and_hms_nano_opt(h, mi, sec, nanos)?;
    let tz = FixedOffset::east_opt(offset)?;
    naive.and_local_timezone(tz).single()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_definite_and_indefinite_lengths() {
        // SEQUENCE { INTEGER 5, OCTET STRING "ab" }, then the same with indefinite length.
        let def = [0x30, 0x07, 0x02, 0x01, 0x05, 0x04, 0x02, b'a', b'b'];
        let t = Tlv::read_all(&def).unwrap();
        let kids = t.children().unwrap();
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[1].octets().unwrap(), b"ab");
        let indef = [0x30, 0x80, 0x02, 0x01, 0x05, 0x04, 0x02, b'a', b'b', 0, 0];
        let t = Tlv::read_all(&indef).unwrap();
        assert!(t.indefinite);
        assert_eq!(t.children().unwrap().len(), 2);
        // A split octet string.
        let split = [0x24, 0x80, 0x04, 0x01, b'a', 0x04, 0x01, b'b', 0, 0];
        assert_eq!(Tlv::read_all(&split).unwrap().octets().unwrap(), b"ab");
    }

    #[test]
    fn refuses_malformed_input() {
        for bad in [
            &[][..],
            &[0x30],
            &[0x30, 0x05, 0x02],                   // runs past the end
            &[0x30, 0x85, 1, 0, 0, 0, 0],          // length of length too long
            &[0x04, 0x80, 0, 0],                   // indefinite primitive
            &[0x30, 0x80, 0x02, 0x01, 0x05],       // no end-of-contents
            &[0x1f, 0x01, 0x00],                   // high tag number
            &[0x30, 0x84, 0xff, 0xff, 0xff, 0xff], // huge length
        ] {
            assert!(Tlv::read(bad).is_err(), "{bad:?}");
        }
        // Deep nesting of indefinite lengths is refused, not a stack overflow.
        let mut deep = Vec::new();
        for _ in 0..10_000 {
            deep.extend([0x30, 0x80]);
        }
        assert!(Tlv::read(&deep).is_err());
    }

    #[test]
    fn writes_der() {
        assert_eq!(der(OCTET_STRING, b"ab"), [0x04, 0x02, b'a', b'b']);
        let long = der(OCTET_STRING, &[7; 300]);
        assert_eq!(&long[..4], [0x04, 0x82, 0x01, 0x2c]);
        assert_eq!(Tlv::read_all(&long).unwrap().body.len(), 300);
        for (n, enc) in [
            (0u64, &[0x02, 0x01, 0x00][..]),
            (3, &[0x02, 0x01, 0x03]),
            (128, &[0x02, 0x02, 0x00, 0x80]),
            (600_000, &[0x02, 0x03, 0x09, 0x27, 0xc0]),
            (
                u64::MAX,
                &[0x02, 0x09, 0, 255, 255, 255, 255, 255, 255, 255, 255],
            ),
        ] {
            assert_eq!(der_uint(n), enc, "{n}");
            assert_eq!(Tlv::read_all(enc).unwrap().uint().unwrap(), n);
        }
        assert!(Tlv::read_all(&[0x02, 0x01, 0xff]).unwrap().uint().is_err());
        // SET OF is sorted.
        let s = der_set(vec![vec![0x04, 0x01, 2], vec![0x04, 0x01, 1]]);
        assert_eq!(s, [0x31, 0x06, 0x04, 0x01, 1, 0x04, 0x01, 2]);
    }

    #[test]
    fn parses_times() {
        let t = parse_time("261016142305Z", false).unwrap();
        assert_eq!(t.to_rfc3339(), "2026-10-16T14:23:05+00:00");
        let t = parse_time("20261016142305.25-0400", true).unwrap();
        assert_eq!(t.to_rfc3339(), "2026-10-16T14:23:05.250-04:00");
        assert_eq!(
            parse_time("4912312359Z", false)
                .unwrap()
                .format("%Y")
                .to_string(),
            "2049"
        );
        assert_eq!(
            parse_time("5001010000Z", false)
                .unwrap()
                .format("%Y")
                .to_string(),
            "1950"
        );
        for bad in [
            "20261016142305",
            "261316142305Z",
            "2610161423Z5",
            "20261016142305.Z",
        ] {
            assert!(parse_time(bad, bad.len() > 13).is_none(), "{bad}");
        }
        // Text after the minutes that isn't ASCII is refused, not a panic.
        for bad in ["2610161423aé", "202610161423é", "2610161423\u{e9}Z"] {
            assert!(parse_time(bad, false).is_none(), "{bad}");
            assert!(parse_time(bad, true).is_none(), "{bad}");
        }
    }
}
