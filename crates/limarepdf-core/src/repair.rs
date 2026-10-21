//! Opening files whose cross-reference information is damaged: offsets that point at the
//! wrong bytes, a cross-reference stream that doesn't read, or no table at all. lopdf
//! trusts the table and then finds no pages, so the objects are found by scanning the
//! bytes instead, as other PDF apps do, and a rebuilt table is appended for lopdf to read.
//! A page tree that lopdf reads with no pages, for reasons other readers look past, is
//! mended too (see [`mend_page_tree`]).

use lopdf::{Dictionary, Document, Object};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::io::Cursor;
use std::ops::Range;

/// The highest object number the PDF format allows; a scanned header above it is noise.
const MAX_OBJECT: u32 = 8_388_607;
/// How many trailers, newest first, are tried for the catalog.
const MAX_TRAILERS: usize = 16;
/// The longest trailer dictionary read, in bytes.
const MAX_TRAILER_BYTES: usize = 64 * 1024;
/// The deepest page tree mended, as deep as page attributes are inherited from.
const MAX_PAGE_TREE_DEPTH: usize = 64;
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
    let scan = scan_objects(body);
    if scan.objects.is_empty() {
        return None;
    }
    let trailer = newest_trailer(body, &scan).unwrap_or_default();
    let objects = scan.objects;

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

/// Mends a page tree in which lopdf finds no pages although other readers do: a page or
/// page tree node written directly in a `/Kids` array is made an object of its own, and a
/// kid whose object isn't a page or a node takes another copy of that object in `bytes` that
/// is one (the file may give a second object the page's number, and lopdf keeps that one).
/// Returns whether anything changed.
pub(crate) fn mend_page_tree(doc: &mut Document, bytes: &[u8]) -> bool {
    let Some(root) = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Pages").ok())
        .and_then(|p| p.as_reference().ok())
    else {
        return false;
    };
    let start = bytes.windows(5).position(|w| w == b"%PDF-").unwrap_or(0);
    let body = &bytes[start..];
    let mut scan = None;
    let mut changed = false;
    let mut seen = HashSet::new();
    let mut todo = vec![(root, 0)];
    while let Some((node, depth)) = todo.pop() {
        if depth > MAX_PAGE_TREE_DEPTH || !seen.insert(node) {
            continue;
        }
        let kids = match doc.get_dictionary(node).and_then(|d| d.get(b"Kids")) {
            Ok(Object::Array(a)) => a.clone(),
            Ok(Object::Reference(r)) => match doc.get_object(*r) {
                Ok(Object::Array(a)) => a.clone(),
                _ => continue,
            },
            _ => continue,
        };
        let mut mended = kids.clone();
        for kid in &mut mended {
            let id = match kid {
                Object::Dictionary(d) if is_page_node(d) => {
                    let mut d = d.clone();
                    d.set("Parent", node);
                    doc.add_object(d)
                }
                Object::Reference(id) => {
                    let id = *id;
                    if !doc.get_dictionary(id).is_ok_and(is_page_node) {
                        let scan = scan.get_or_insert_with(|| scan_objects(body));
                        match page_node_copy(body, scan, id) {
                            Some(d) => {
                                doc.objects.insert(id, Object::Dictionary(d));
                                changed = true;
                            }
                            None => continue,
                        }
                    }
                    id
                }
                _ => continue,
            };
            *kid = Object::Reference(id);
            todo.push((id, depth + 1));
        }
        if mended != kids {
            if let Ok(d) = doc.get_dictionary_mut(node) {
                d.set("Kids", mended);
                changed = true;
            }
        }
    }
    changed
}

/// Whether `d` is a page or a page tree node.
fn is_page_node(d: &Dictionary) -> bool {
    d.has_type(b"Page") || d.has_type(b"Pages")
}

/// The first copy of object `id` in `b`, outside stream data, that is a page or a page tree
/// node.
fn page_node_copy(b: &[u8], scan: &Scan, id: lopdf::ObjectId) -> Option<Dictionary> {
    let mut i = 0;
    while i < b.len() {
        let line = i + b[i..]
            .iter()
            .take_while(|c| matches!(c, b' ' | b'\t'))
            .count();
        if object_header(&b[line..]) == Some(id) && !scan.in_stream(line) {
            let after = line + find(&b[line..], b"obj")? + 3;
            let at = after + b[after..].iter().take_while(|c| is_white(**c)).count();
            if let Some(d) = read_dictionary(&b[at..]).filter(is_page_node) {
                return Some(d);
            }
        }
        i = match b[i..].iter().position(|c| matches!(c, b'\r' | b'\n')) {
            Some(n) => i + n + 1,
            None => b.len(),
        };
    }
    None
}

/// What a scan of a file's bytes finds.
#[derive(Default)]
struct Scan {
    /// Every `N G obj` header that starts a line, by object number: where it starts and its
    /// generation. A later copy of a number wins, as in an appended revision.
    objects: BTreeMap<u32, (usize, u16)>,
    /// Where each stream's data is, from its `stream` keyword to the end of `endstream`.
    streams: Vec<Range<usize>>,
}

impl Scan {
    fn in_stream(&self, at: usize) -> bool {
        let i = self.streams.partition_point(|s| s.end <= at);
        self.streams.get(i).is_some_and(|s| s.contains(&at))
    }
}

/// Where every `N G obj <integer> endobj` whose header starts a line is, and its value, by
/// number and generation, in file order: the candidates for an indirect `/Length`. Copies
/// inside stream data are included, since where the streams end isn't known yet.
fn integer_objects(b: &[u8]) -> HashMap<(u32, u16), Vec<(usize, u64)>> {
    let mut out: HashMap<(u32, u16), Vec<(usize, u64)>> = HashMap::new();
    let mut i = 0;
    while i < b.len() {
        let line = i + b[i..]
            .iter()
            .take_while(|c| matches!(c, b' ' | b'\t'))
            .count();
        if let Some(id) = object_header(&b[line..]) {
            let rest = &b[line + find(&b[line..], b"obj").unwrap_or(0) + 3..];
            let rest = &rest[rest.iter().take_while(|c| is_white(**c)).count()..];
            let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
            let after = &rest[digits..];
            let space = after.iter().take_while(|c| is_white(**c)).count();
            if space > 0 && after[space..].starts_with(b"endobj") {
                let value = std::str::from_utf8(&rest[..digits]).ok();
                if let Some(v) = value.and_then(|v| v.parse().ok()) {
                    out.entry(id).or_default().push((line, v));
                }
            }
        }
        i = match b[i..].iter().position(|c| matches!(c, b'\r' | b'\n')) {
            Some(n) => i + n + 1,
            None => b.len(),
        };
    }
    out
}

/// Scans `b` for objects. Stream data is skipped, so headers inside an uncompressed stream
/// (an attached PDF, say) don't count.
fn scan_objects(b: &[u8]) -> Scan {
    let lengths = integer_objects(b);
    let mut out = Scan::default();
    let mut header = None;
    let mut line_start = true;
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"stream")
            && !b[..i].ends_with(b"end")
            && matches!(b.get(i + 6), Some(b'\r' | b'\n'))
        {
            if let Some(end) = stream_end(b, header, i, &lengths) {
                out.streams.push(i..end);
                i = end;
                line_start = false;
                continue;
            }
        }
        if line_start && b[i].is_ascii_digit() {
            if let Some((num, generation)) = object_header(&b[i..]) {
                header = Some(i);
                if (1..=MAX_OBJECT).contains(&num) {
                    out.objects.insert(num, (i, generation));
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

/// Where the stream whose `stream` keyword is at `at` ends, just after its `endstream`,
/// given where its object's header is. Its `/Length` is used when `endstream` follows the
/// data there, as other PDF apps do: a direct number, or for an indirect one the latest of
/// `lengths`' copies of that object that fits and isn't part of the data, as it would be if
/// it were inside the data its value or another fitting copy's gives (an attached file can
/// hold a copy, to end the stream early). Else the stream ends at the first `endstream`.
/// `None` when there is no `endstream`, or the keyword is inside the object's dictionary
/// (in a string), so no stream starts there.
fn stream_end(
    b: &[u8],
    header: Option<usize>,
    at: usize,
    lengths: &HashMap<(u32, u16), Vec<(usize, u64)>>,
) -> Option<usize> {
    let dict = header.and_then(|h| {
        let obj = h + find(&b[h..at], b"obj")? + 3;
        let open = obj + b[obj..at].iter().position(|c| !is_white(*c))?;
        Some((open, open + dictionary_len(&b[open..])?))
    });
    if let Some((open, close)) = dict {
        if close > at {
            return None;
        }
        let eol = if b[at + 6..].starts_with(b"\r\n") {
            2
        } else {
            1
        };
        let data = at + 6 + eol;
        let ends_at = |n: u64| {
            let e = data
                .checked_add(usize::try_from(n).ok()?)
                .filter(|&e| e <= b.len())?;
            let e = e + b[e..].iter().take_while(|c| is_white(**c)).count();
            b[e..].starts_with(b"endstream").then_some(e)
        };
        let end = read_dictionary(&b[open..close]).and_then(|d| match d.get(b"Length").ok()? {
            Object::Integer(n) => ends_at(u64::try_from(*n).ok()?),
            Object::Reference(id) => {
                let fits: Vec<(usize, usize)> = lengths
                    .get(id)?
                    .iter()
                    .filter_map(|&(pos, n)| Some((pos, ends_at(n)?)))
                    .filter(|&(pos, end)| !(data..end).contains(&pos))
                    .collect();
                // Every copy's data starts at `data`: inside another's is inside the longest.
                let longest = fits.iter().map(|&(_, end)| end).max()?;
                fits.iter()
                    .rev()
                    .find(|&&(pos, _)| !(data..longest).contains(&pos))
                    .map(|&(_, end)| end)
            }
            _ => None,
        });
        if let Some(end) = end {
            return Some(end + 9);
        }
    }
    Some(at + 6 + find(&b[at + 6..], b"endstream")? + 9)
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
/// dictionary or a cross-reference stream's, leaving out any inside stream data.
fn newest_trailer(b: &[u8], scan: &Scan) -> Option<Dictionary> {
    let mut places: Vec<usize> = Vec::new();
    let mut at = 0;
    while let Some(i) = find(&b[at..], b"trailer") {
        let after = at + i + 7;
        if let Some(d) = b[after..].iter().position(|c| !is_white(*c)) {
            if !scan.in_stream(at + i) {
                places.push(after + d);
            }
        }
        at = after;
    }
    // A cross-reference stream's dictionary follows its object's header.
    let mut streams: Vec<usize> = Vec::new();
    let mut at = 0;
    while let Some(i) = find(&b[at..], b"/XRef") {
        let pos = at + i;
        let start = (!scan.in_stream(pos))
            .then(|| scan.objects.values().filter(|(o, _)| *o < pos).max())
            .flatten();
        if let Some(&(start, _)) = start {
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
pub(crate) fn write_object(out: &mut String, o: &Object, depth: usize) {
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
        assert_eq!(numbers(&scan_objects(b)), vec![1, 3]);
    }

    fn numbers(scan: &Scan) -> Vec<u32> {
        scan.objects.keys().copied().collect()
    }

    #[test]
    fn streams_end_by_their_length() {
        let b = b"1 0 obj\n<< /Length 18 >>\nstream\nendstream\n2 0 obj\nendstream\nendobj\n";
        let scan = scan_objects(b);
        assert_eq!(numbers(&scan), vec![1]);
        assert_eq!(scan.streams, vec![25..b.len() - 8]);
        // A length that doesn't reach an `endstream` isn't used, nor an indirect one with no
        // object holding it.
        for length in ["14", "16", "9 0 R", "-3"] {
            let b = format!("1 0 obj\n<< /Length {length} >>\nstream\nab\nendstream\n2 0 obj\n");
            assert_eq!(numbers(&scan_objects(b.as_bytes())), vec![1, 2]);
        }
    }

    #[test]
    fn streams_end_by_an_indirect_length() {
        // The length object comes after the stream; an earlier copy that doesn't fit is
        // passed over, and the latest that fits is used.
        let b = b"1 0 obj\n<< /Length 9 0 R >>\nstream\nendstream\n2 0 obj\nendstream\nendobj\n\
                  9 0 obj\n18\nendobj\n9 0 obj 3 endobj\n";
        assert_eq!(numbers(&scan_objects(b)), vec![1, 9]);
        let at = |s: &[u8]| b.windows(s.len()).position(|w| w == s).unwrap();
        assert_eq!(
            integer_objects(b).get(&(9, 0)),
            Some(&vec![(at(b"9 0 obj\n18"), 18), (at(b"9 0 obj 3"), 3)])
        );
        // A different generation isn't the same object.
        let b = b"1 0 obj\n<< /Length 9 1 R >>\nstream\nendstream\n2 0 obj\nendstream\nendobj\n\
                  9 0 obj\n18\nendobj\n";
        assert_eq!(numbers(&scan_objects(b)), vec![1, 2, 9]);
    }

    #[test]
    fn a_length_inside_the_stream_data_is_not_used() {
        // The data holds an `endstream` and then a later copy of the length object, which
        // would end the stream there and make the rest of the data objects.
        let data = "x\nendstream\n9 0 obj\n2\nendobj\n2 0 obj\n<< >>\nendobj";
        for before in [true, false] {
            let length = format!("9 0 obj\n{}\nendobj\n", data.len());
            let stream =
                format!("1 0 obj\n<< /Length 9 0 R >>\nstream\n{data}\nendstream\nendobj\n");
            let b = if before {
                length + &stream
            } else {
                stream + &length
            };
            let scan = scan_objects(b.as_bytes());
            assert_eq!(numbers(&scan), vec![1, 9], "length before: {before}");
            let at = b.find("stream\nx").unwrap();
            assert_eq!(scan.streams, vec![at..b.rfind("endstream").unwrap() + 9]);
        }
        // Nor one whose value would end the stream past itself: it would be data then too.
        let data = "x\n9 0 obj\n0000\nendobj";
        let b = format!(
            "1 0 obj\n<< /Length 9 0 R >>\nstream\n{data}\nendstream\nendobj\n9 0 obj\n{}\n\
             endobj\n2 0 obj\n<< /Length 3 >>\nstream\nabc\nendstream\nendobj\n",
            data.len()
        );
        let start = b.find("x\n").unwrap();
        let far = b.rfind("endstream").unwrap() - start;
        let b = b.replace("0000", &format!("{far:04}"));
        assert_eq!(numbers(&scan_objects(b.as_bytes())), vec![1, 2, 9]);
    }

    #[test]
    fn trailers_inside_streams_are_not_used() {
        let b = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n\
                  2 0 obj\n<< /Length 25 >>\nstream\ntrailer\n<< /Root 9 0 R >>\nendstream\n\
                  trailer\n<< /Root 1 0 R >>\n3 0 obj\n<< /Length 25 >>\nstream\n\
                  trailer\n<< /Root 9 0 R >>\nendstream\nendobj\n";
        let scan = scan_objects(b);
        let trailer = newest_trailer(b, &scan).unwrap();
        assert_eq!(
            trailer.get(b"Root").unwrap().as_reference().unwrap(),
            (1, 0)
        );
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
