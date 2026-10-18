//! Opening files whose cross-reference information is damaged: offsets that point at the
//! wrong bytes, a cross-reference stream that doesn't read, or no table at all. lopdf
//! trusts the table and then finds no pages, so the objects are found by scanning the
//! bytes instead, as other PDF apps do, and a rebuilt table is appended for lopdf to read.

use lopdf::{Dictionary, Document, Object};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Cursor;

/// The highest object number the PDF format allows; a scanned header above it is noise.
const MAX_OBJECT: u32 = 8_388_607;
/// How many trailers, newest first, are tried for the catalog.
const MAX_TRAILERS: usize = 16;
/// The longest trailer dictionary read, in bytes.
const MAX_TRAILER_BYTES: usize = 64 * 1024;
/// The most bytes an object stream is unpacked to.
const MAX_OBJECT_STREAM: usize = 64 << 20;
/// Trailer entries carried into the rebuilt table. `/Encrypt` and `/ID` must be there when
/// lopdf reads the objects, since decrypting them needs both.
const TRAILER_KEYS: [&[u8]; 4] = [b"Root", b"Info", b"ID", b"Encrypt"];

/// `bytes` with a rebuilt cross-reference table appended, listing every object found by
/// scanning (a later copy of a number wins) under the newest trailer that names a catalog.
/// `None` when no objects are found. Objects inside object streams come with their
/// stream: lopdf takes the members of every object stream it reads.
pub(crate) fn rebuild(bytes: &[u8]) -> Option<Vec<u8>> {
    // lopdf reads from the header on, and its offsets count from there.
    let start = bytes.windows(5).position(|w| w == b"%PDF-")?;
    let body = &bytes[start..];
    let objects = scan_objects(body);
    if objects.is_empty() {
        return None;
    }
    let trailer = newest_trailer(body, &objects).unwrap_or_default();

    let mut out = bytes.to_vec();
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    let xref = out.len() - start;
    let mut table = String::from("xref\n0 1\n0000000000 65535 f \n");
    let mut run: Vec<(u32, usize, u16)> = Vec::new();
    let flush = |run: &mut Vec<(u32, usize, u16)>, table: &mut String| {
        if let Some(&(first, ..)) = run.first() {
            let _ = writeln!(table, "{first} {}", run.len());
            for (_, offset, generation) in run.drain(..) {
                let _ = write!(table, "{offset:010} {generation:05} n\r\n");
            }
        }
    };
    for (&num, &(offset, generation)) in &objects {
        if run.last().is_some_and(|&(last, ..)| last + 1 != num) {
            flush(&mut run, &mut table);
        }
        run.push((num, offset, generation));
    }
    flush(&mut run, &mut table);
    let size = objects.keys().next_back().map_or(1, |n| n + 1);
    let _ = write!(table, "trailer\n<< /Size {size}");
    for key in TRAILER_KEYS {
        if let Ok(v) = trailer.get(key) {
            table.push(' ');
            write_name(&mut table, key);
            table.push(' ');
            write_object(&mut table, v, 0);
        }
    }
    let _ = write!(table, " >>\nstartxref\n{xref}\n%%EOF\n");
    out.extend_from_slice(table.as_bytes());
    Some(out)
}

/// The catalog among `doc`'s objects when its trailer names none that is there: the
/// highest-numbered object of type Catalog that has a page tree.
pub(crate) fn find_catalog(doc: &Document) -> Option<lopdf::ObjectId> {
    doc.objects
        .iter()
        .rev()
        .find(|(_, o)| {
            o.as_dict()
                .is_ok_and(|d| d.has_type(b"Catalog") && d.get(b"Pages").is_ok())
        })
        .map(|(id, _)| *id)
}

/// Adds the members of `doc`'s object streams that lopdf couldn't read there, reading each
/// on its own: lopdf skips a member that starts with a comment, for one. An object already
/// in `doc` is kept.
pub(crate) fn unpack_object_streams(doc: &mut Document) {
    let mut found = Vec::new();
    for o in doc.objects.values() {
        let Object::Stream(s) = o else { continue };
        if !s.dict.has_type(b"ObjStm") {
            continue;
        }
        let Some(first) = s
            .dict
            .get(b"First")
            .and_then(Object::as_i64)
            .ok()
            .and_then(|f| usize::try_from(f).ok())
        else {
            continue;
        };
        let Ok(content) = s.get_plain_content_with_limit(MAX_OBJECT_STREAM) else {
            continue;
        };
        let Some(index) = content.get(..first) else {
            continue;
        };
        let numbers: Vec<usize> = index
            .split(|c| is_white(*c))
            .filter(|w| !w.is_empty())
            .map_while(|w| std::str::from_utf8(w).ok()?.parse().ok())
            .collect();
        let mut members: Vec<(usize, u32)> = numbers
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|&[num, at]| Some((first.checked_add(at)?, u32::try_from(num).ok()?)))
            .filter(|&(at, _)| at < content.len())
            .collect();
        members.sort_unstable();
        for (i, &(at, num)) in members.iter().enumerate() {
            if doc.objects.contains_key(&(num, 0)) {
                continue;
            }
            let end = members.get(i + 1).map_or(content.len(), |m| m.0);
            if let Some(o) = read_object(&content[at..end]) {
                found.push(((num, 0), o));
            }
        }
    }
    for (id, o) in found {
        doc.objects.entry(id).or_insert(o);
    }
    if let Some(&(top, _)) = doc.objects.keys().next_back() {
        doc.max_id = doc.max_id.max(top);
    }
}

/// Every `N G obj` header that starts a line, by object number: where it starts and its
/// generation. A later copy of a number wins, as in an appended revision. Stream data is
/// skipped, so headers inside an uncompressed stream don't count.
fn scan_objects(b: &[u8]) -> BTreeMap<u32, (usize, u16)> {
    let mut out = BTreeMap::new();
    let mut line_start = true;
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"stream")
            && !b[..i].ends_with(b"end")
            && matches!(b.get(i + 6), Some(b'\r' | b'\n'))
        {
            if let Some(end) = find(&b[i + 6..], b"endstream") {
                i += 6 + end + 9;
                line_start = false;
                continue;
            }
        }
        if line_start && b[i].is_ascii_digit() {
            if let Some((num, generation)) = object_header(&b[i..]) {
                if (1..=MAX_OBJECT).contains(&num) {
                    out.insert(num, (i, generation));
                }
            }
        }
        match b[i] {
            b'\r' | b'\n' => line_start = true,
            b' ' | b'\t' | b'\x0c' | b'\0' => {}
            _ => line_start = false,
        }
        i += 1;
    }
    out
}

/// The object number and generation of an `N G obj` header at the start of `b`.
fn object_header(b: &[u8]) -> Option<(u32, u16)> {
    fn number(b: &[u8], max: usize) -> Option<(&[u8], &[u8])> {
        let n = b.iter().take_while(|c| c.is_ascii_digit()).count();
        (n > 0 && n <= max).then(|| b.split_at(n))
    }
    fn space(b: &[u8]) -> Option<&[u8]> {
        let n = b.iter().take_while(|c| is_white(**c)).count();
        (n > 0).then(|| &b[n..])
    }
    let (num, rest) = number(b, 10)?;
    let (generation, rest) = number(space(rest)?, 5)?;
    let rest = space(rest)?.strip_prefix(b"obj")?;
    if rest.first().is_some_and(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let num = std::str::from_utf8(num).ok()?.parse().ok()?;
    let generation = std::str::from_utf8(generation).ok()?.parse().ok()?;
    Some((num, generation))
}

/// The newest trailer whose `/Root` is an indirect reference: a table's `trailer`
/// dictionary or a cross-reference stream's.
fn newest_trailer(b: &[u8], objects: &BTreeMap<u32, (usize, u16)>) -> Option<Dictionary> {
    let mut places: Vec<usize> = Vec::new();
    let mut at = 0;
    while let Some(i) = find(&b[at..], b"trailer") {
        let after = at + i + 7;
        if let Some(d) = b[after..].iter().position(|c| !is_white(*c)) {
            places.push(after + d);
        }
        at = after;
    }
    // A cross-reference stream's dictionary follows its object's header.
    let mut streams: Vec<usize> = Vec::new();
    let mut at = 0;
    while let Some(i) = find(&b[at..], b"/XRef") {
        let pos = at + i;
        if let Some(&(start, _)) = objects.values().filter(|(o, _)| *o < pos).max() {
            if let Some(d) = find(&b[start..pos], b"<<") {
                streams.push(start + d);
            }
        }
        at = pos + 5;
    }
    places.extend(streams);
    places.sort_unstable();
    places.dedup();
    places
        .iter()
        .rev()
        .take(MAX_TRAILERS)
        .filter_map(|&p| read_dictionary(&b[p..]))
        .find(|d| d.get(b"Root").and_then(Object::as_reference).is_ok())
}

/// Reads the dictionary at the start of `b`.
fn read_dictionary(b: &[u8]) -> Option<Dictionary> {
    match read_object(&b[..dictionary_len(b)?])? {
        Object::Dictionary(d) => Some(d),
        _ => None,
    }
}

/// Reads `b` as one object (a stream's data can't be reached), by having lopdf read it as
/// the one object of a small file.
fn read_object(b: &[u8]) -> Option<Object> {
    let mut doc = b"%PDF-1.7\n".to_vec();
    let at = doc.len();
    doc.extend_from_slice(b"1 0 obj\n");
    doc.extend_from_slice(b);
    doc.extend_from_slice(b"\nendobj\n");
    let xref = doc.len();
    doc.extend_from_slice(
        format!(
            "xref\n0 2\n0000000000 65535 f\r\n{at:010} 00000 n\r\ntrailer\n<< /Size 2 >>\n\
             startxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    match Document::load_from(Cursor::new(doc))
        .ok()?
        .objects
        .remove(&(1, 0))?
    {
        Object::Stream(s) => Some(Object::Dictionary(s.dict)),
        o => Some(o),
    }
}

/// The length of the dictionary at the start of `b`, from `<<` to its matching `>>`,
/// stepping over strings and comments.
fn dictionary_len(b: &[u8]) -> Option<usize> {
    if !b.starts_with(b"<<") {
        return None;
    }
    let mut depth = 0usize;
    let mut i = 0;
    while i < b.len().min(MAX_TRAILER_BYTES) {
        match b[i] {
            b'<' if b.get(i + 1) == Some(&b'<') => {
                depth += 1;
                i += 2;
            }
            b'>' if b.get(i + 1) == Some(&b'>') => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'<' => i += b[i..].iter().position(|&c| c == b'>')? + 1,
            b'(' => {
                let mut nest = 0usize;
                while i < b.len() {
                    match b[i] {
                        b'\\' => i += 1,
                        b'(' => nest += 1,
                        b')' => {
                            nest -= 1;
                            if nest == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
            }
            b'%' => {
                while i < b.len() && !matches!(b[i], b'\r' | b'\n') {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    None
}

/// Writes `o` in PDF syntax. Strings are written in hexadecimal, which needs no escaping.
fn write_object(out: &mut String, o: &Object, depth: usize) {
    if depth > 32 {
        out.push_str("null");
        return;
    }
    match o {
        Object::Null => out.push_str("null"),
        Object::Boolean(v) => out.push_str(if *v { "true" } else { "false" }),
        Object::Integer(v) => {
            let _ = write!(out, "{v}");
        }
        Object::Real(v) => {
            let _ = write!(out, "{v}");
        }
        Object::Name(n) => write_name(out, n),
        Object::String(s, _) => {
            out.push('<');
            for c in s {
                let _ = write!(out, "{c:02X}");
            }
            out.push('>');
        }
        Object::Array(a) => {
            out.push('[');
            for (i, v) in a.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                write_object(out, v, depth + 1);
            }
            out.push(']');
        }
        Object::Dictionary(d) => {
            out.push_str("<<");
            for (k, v) in d.iter() {
                write_name(out, k);
                out.push(' ');
                write_object(out, v, depth + 1);
            }
            out.push_str(">>");
        }
        Object::Reference((n, g)) => {
            let _ = write!(out, "{n} {g} R");
        }
        // A stream can't be a trailer value.
        Object::Stream(_) => out.push_str("null"),
    }
}

/// Writes a name, with `#xx` for bytes that can't stand in a name as they are.
fn write_name(out: &mut String, n: &[u8]) {
    out.push('/');
    for &c in n {
        if c.is_ascii_graphic() && !b"#%()/<>[]{}".contains(&c) {
            out.push(c as char);
        } else {
            let _ = write!(out, "#{c:02X}");
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn is_white(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionaries_end_at_their_own_brackets() {
        let b = b"<< /A (a >> b \\) c) /B <414243> /C << /D 1 >> % >>\n>> trailing";
        assert_eq!(dictionary_len(b), Some(b.len() - " trailing".len()));
        assert_eq!(dictionary_len(b"<< /A 1"), None);
    }

    #[test]
    fn headers_need_whole_tokens() {
        assert_eq!(object_header(b"12 0 obj\n<<"), Some((12, 0)));
        assert_eq!(object_header(b"12 0 obj<<"), Some((12, 0)));
        assert_eq!(object_header(b"12 0 objx"), None);
        assert_eq!(object_header(b"12 obj"), None);
    }

    #[test]
    fn objects_inside_stream_data_are_skipped() {
        let b = b"%PDF-1.7\n1 0 obj\n<< /Length 9 >>\nstream\n2 0 obj\nendstream\nendobj\n3 0 obj\n1\nendobj\n";
        let found = scan_objects(b);
        assert_eq!(found.keys().copied().collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn trailer_values_are_written_back_readably() {
        let mut d = Dictionary::new();
        d.set("Root", Object::Reference((4, 0)));
        d.set(
            "ID",
            vec![
                Object::string_literal(b"a)b".to_vec()),
                Object::Name(b"x y#".to_vec()),
            ],
        );
        let mut s = String::new();
        write_object(&mut s, &Object::Dictionary(d.clone()), 0);
        let back = read_dictionary(s.as_bytes()).unwrap();
        assert_eq!(back.get(b"Root").unwrap(), d.get(b"Root").unwrap());
        let id = back.get(b"ID").unwrap().as_array().unwrap();
        assert_eq!(id[0].as_str().unwrap(), b"a)b");
        assert_eq!(id[1].as_name().unwrap(), b"x y#");
    }
}
