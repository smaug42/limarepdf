//! The saved-output checker (build plan, stage E step 2): what a saved file still holds of
//! something that must be gone, looked for the ways another reader could find it.
//!
//! - **The bytes as written,** before any repair or pruning: every form a marker can take in a
//!   string (literal, hex, UTF-16) in the raw file, and in every `stream` segment of the file
//!   decoded, whether any object still uses it or not.
//! - **The file's own cross-reference table,** checked entry by entry, so the structure read
//!   here is the one written and not one lopdf rebuilt by scanning.
//! - **Structure, explicitly:** form fields (values, defaults, rich text, inherited ones) and
//!   their appearances, actions and JavaScript wherever they are, optional content, pages and
//!   references to pages that aren't in the page tree.
//! - **Outside readers** (`pdftotext`, Ghostscript's `txtwrite`, which draws form fields, and
//!   `pdfinfo -js`), for the opt-in `cross_check` tests.

use limarepdf_core::Pdf;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::collections::HashSet;

/// Actions that run something, send data or open other files by themselves (as
/// `sanitise.rs` lists them).
const ACTIVE: [&[u8]; 6] = [
    b"JavaScript",
    b"Launch",
    b"SubmitForm",
    b"ImportData",
    b"Rendition",
    b"GoToE",
];

/// Deepest nesting of direct objects followed.
const MAX_DEPTH: usize = 64;

pub struct Saved {
    pub bytes: Vec<u8>,
    /// The file as its cross-reference table gives it.
    pub doc: Document,
    /// Every `stream` segment in the file, decoded, with its byte offset.
    segments: Vec<(usize, Vec<u8>)>,
    /// Segments whose filters the checker doesn't decode (pictures), with their offsets and
    /// filters: what a marker search can't see into.
    pub opaque: Vec<(usize, String)>,
}

impl Saved {
    /// `p` saved, as [`Pdf::save`] writes it.
    pub fn of(p: &mut Pdf) -> Self {
        Self::new(p.to_bytes().unwrap())
    }

    pub fn new(bytes: Vec<u8>) -> Self {
        check_xref(&bytes);
        let doc = Document::load_mem(&bytes).expect("the saved file loads");
        let (segments, opaque) = segments(&bytes);
        Self {
            bytes,
            doc,
            segments,
            opaque,
        }
    }

    /// Every place `needle` is in the file: in its raw bytes, in a decoded stream segment, or in
    /// a string of an object (escapes undone).
    pub fn places(&self, needle: &str) -> Vec<String> {
        let forms = forms(needle);
        let mut out = Vec::new();
        for f in &forms {
            for at in find_all(&self.bytes, f) {
                out.push(format!("raw bytes at {at}"));
            }
        }
        for (at, data) in &self.segments {
            if forms.iter().any(|f| contains(data, f)) {
                out.push(format!("stream at {at}, decoded"));
            }
        }
        for (id, o) in &self.doc.objects {
            let mut hit = false;
            each_string(o, 0, &mut |s| hit |= forms.iter().any(|f| contains(s, f)));
            if hit {
                out.push(format!("a string in object {} {}", id.0, id.1));
            }
        }
        out
    }

    /// Panics, listing every place, when any of `needles` is still in the file.
    pub fn assert_gone(&self, needles: &[&str]) {
        let found: Vec<String> = needles
            .iter()
            .flat_map(|n| self.places(n).into_iter().map(move |p| format!("{n}: {p}")))
            .collect();
        assert!(
            found.is_empty(),
            "still in the saved file:\n{}",
            found.join("\n")
        );
    }

    /// Panics when any of `needles` is no longer in the file (the controls).
    pub fn assert_kept(&self, needles: &[&str]) {
        for n in needles {
            assert!(
                !self.places(n).is_empty(),
                "{n} is gone from the saved file"
            );
        }
    }

    /// Every form field node (terminal or not, in the order of the field tree), with what it
    /// holds.
    pub fn fields(&self) -> Vec<FieldState> {
        let mut out = Vec::new();
        let fields = self
            .doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"AcroForm").ok())
            .and_then(|o| self.dict(o))
            .and_then(|f| f.get(b"Fields").ok())
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok().cloned())
            .unwrap_or_default();
        let mut seen = HashSet::new();
        for f in &fields {
            if let Ok(id) = f.as_reference() {
                self.walk_field(id, "", &mut seen, &mut out, 0);
            }
        }
        out
    }

    fn walk_field(
        &self,
        id: ObjectId,
        parent: &str,
        seen: &mut HashSet<ObjectId>,
        out: &mut Vec<FieldState>,
        depth: usize,
    ) {
        assert!(
            depth < MAX_DEPTH,
            "the field tree is deeper than the checker follows"
        );
        if !seen.insert(id) {
            return;
        }
        let Ok(d) = self.doc.get_dictionary(id) else {
            return;
        };
        let name = match d.get(b"T").ok().and_then(|o| o.as_str().ok()) {
            Some(t) if parent.is_empty() => String::from_utf8_lossy(t).into_owned(),
            Some(t) => format!("{parent}.{}", String::from_utf8_lossy(t)),
            None => parent.to_string(),
        };
        let mut state = FieldState {
            name: name.clone(),
            values: Vec::new(),
            appearance: String::new(),
        };
        for k in [&b"V"[..], b"DV", b"RV", b"I"] {
            if let Ok(v) = d.get(k) {
                let v = self.doc.dereference(v).map_or(v, |(_, o)| o);
                // A button's Off is no value.
                if v.as_name().ok() != Some(b"Off") {
                    state
                        .values
                        .push(format!("/{} {v:?}", String::from_utf8_lossy(k)));
                }
            }
        }
        let kids: Vec<ObjectId> = d
            .get(b"Kids")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .map(|a| a.iter().filter_map(|k| k.as_reference().ok()).collect())
            .unwrap_or_default();
        let mut widgets: Vec<ObjectId> = kids
            .iter()
            .copied()
            .filter(|k| {
                self.doc
                    .get_dictionary(*k)
                    .is_ok_and(|k| !k.has(b"T") && k.has(b"Rect"))
            })
            .collect();
        if d.has(b"Rect") {
            widgets.push(id);
        }
        for w in widgets {
            if let Ok(w) = self.doc.get_dictionary(w) {
                if let Some(state_name) = w.get(b"AS").ok().and_then(|o| o.as_name().ok()) {
                    if state_name != b"Off" {
                        state
                            .values
                            .push(format!("/AS {}", String::from_utf8_lossy(state_name)));
                    }
                }
                let ap = w.get(b"AP").ok().and_then(|o| self.dict(o));
                for (_, o) in ap.iter().flat_map(|ap| ap.iter()) {
                    self.each_stream(o, &mut |s| {
                        state
                            .appearance
                            .push_str(&String::from_utf8_lossy(&decoded(s)));
                    });
                }
            }
        }
        out.push(state);
        for k in kids {
            if self
                .doc
                .get_dictionary(k)
                .is_ok_and(|k| k.has(b"T") || !k.has(b"Rect"))
            {
                self.walk_field(k, &name, seen, out, depth + 1);
            }
        }
    }

    /// Every action that runs something by itself, every `/JS` and every set of additional
    /// actions (`/AA`), anywhere in the file, direct or not; and the document's JavaScript.
    pub fn actions(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (id, o) in &self.doc.objects {
            let at = format!("object {} {}", id.0, id.1);
            each_dict(o, &at, 0, &mut |path, d| {
                let s = d.get(b"S").and_then(|o| o.as_name()).unwrap_or(b"");
                if ACTIVE.contains(&s) {
                    out.push(format!("{path}: /S /{}", String::from_utf8_lossy(s)));
                } else if d.has(b"JS") {
                    out.push(format!("{path}: /JS"));
                }
                if d.has(b"AA") {
                    out.push(format!("{path}: /AA"));
                }
            });
        }
        let js = self
            .doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"Names").ok())
            .and_then(|o| self.dict(o))
            .is_some_and(|n| n.has(b"JavaScript"));
        if js {
            out.push("the catalog's /Names /JavaScript".into());
        }
        out
    }

    /// Streams and annotations in a layer (optional content group) that isn't in the layer
    /// list or is switched off, and resources naming such layers.
    pub fn hidden_layer_users(&self) -> Vec<String> {
        let props = self
            .doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"OCProperties").ok())
            .and_then(|o| self.dict(o));
        let refs = |o: Option<&Object>| -> HashSet<ObjectId> {
            o.and_then(|o| self.doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_array().ok())
                .map(|a| a.iter().filter_map(|x| x.as_reference().ok()).collect())
                .unwrap_or_default()
        };
        let listed = refs(props.and_then(|p| p.get(b"OCGs").ok()));
        let d = props
            .and_then(|p| p.get(b"D").ok())
            .and_then(|o| self.dict(o));
        let off = refs(d.and_then(|d| d.get(b"OFF").ok()));
        let base_off = d
            .and_then(|d| d.get(b"BaseState").ok())
            .and_then(|o| o.as_name().ok())
            == Some(b"OFF");
        let on = refs(d.and_then(|d| d.get(b"ON").ok()));
        let hidden = |id: &ObjectId| {
            let is_group = self
                .doc
                .get_dictionary(*id)
                .is_ok_and(|g| g.get(b"Type").and_then(|o| o.as_name()).ok() == Some(b"OCG"));
            is_group && (!listed.contains(id) || off.contains(id) || (base_off && !on.contains(id)))
        };
        let mut out = Vec::new();
        for (id, o) in &self.doc.objects {
            let at = format!("object {} {}", id.0, id.1);
            each_dict(o, &at, 0, &mut |path, d| {
                for (k, v) in d.iter() {
                    let named = match v {
                        Object::Reference(r) => vec![*r],
                        Object::Dictionary(m) => m
                            .get(b"OCGs")
                            .map(|g| match g {
                                Object::Reference(r) => vec![*r],
                                Object::Array(a) => {
                                    a.iter().filter_map(|x| x.as_reference().ok()).collect()
                                }
                                _ => Vec::new(),
                            })
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    };
                    if named.iter().any(hidden) {
                        out.push(format!("{path} /{}", String::from_utf8_lossy(k)));
                    }
                }
            });
        }
        out
    }

    /// Page objects that aren't in the page tree, and references to them or to objects the
    /// file doesn't have.
    pub fn stray_pages(&self) -> Vec<String> {
        let pages: HashSet<ObjectId> = self.doc.get_pages().into_values().collect();
        let mut out = Vec::new();
        for (id, o) in &self.doc.objects {
            let is_page = o
                .as_dict()
                .is_ok_and(|d| d.get(b"Type").and_then(|t| t.as_name()).ok() == Some(b"Page"));
            if is_page && !pages.contains(id) {
                out.push(format!(
                    "page object {} {} is not in the page tree",
                    id.0, id.1
                ));
            }
            let mut missing = Vec::new();
            each_reference(o, 0, &mut |r| {
                if !self.doc.objects.contains_key(&r) {
                    missing.push(r);
                }
            });
            for r in missing {
                out.push(format!(
                    "object {} {} refers to {} {}, which isn't in the file",
                    id.0, id.1, r.0, r.1
                ));
            }
        }
        out
    }

    /// The objects nothing reachable from the trailer refers to (the cross-reference stream
    /// aside).
    pub fn unreachable(&self) -> Vec<ObjectId> {
        let mut seen = HashSet::new();
        let mut todo: Vec<ObjectId> = Vec::new();
        each_reference(&Object::Dictionary(self.doc.trailer.clone()), 0, &mut |r| {
            todo.push(r)
        });
        while let Some(id) = todo.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(o) = self.doc.objects.get(&id) {
                each_reference(o, 0, &mut |r| todo.push(r));
            }
        }
        let xref = |o: &Object| {
            o.as_stream()
                .is_ok_and(|s| s.dict.get(b"Type").and_then(|t| t.as_name()).ok() == Some(b"XRef"))
        };
        let mut out: Vec<ObjectId> = self
            .doc
            .objects
            .iter()
            .filter(|(id, o)| !seen.contains(id) && !xref(o))
            .map(|(id, _)| *id)
            .collect();
        out.sort();
        out
    }

    /// The bookmark titles, in order, each with whether it has a destination or action.
    pub fn bookmarks(&self) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        let first = self
            .doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"Outlines").ok())
            .and_then(|o| self.dict(o))
            .and_then(|d| d.get(b"First").ok())
            .and_then(|o| o.as_reference().ok());
        let mut todo: Vec<ObjectId> = first.into_iter().collect();
        let mut seen = HashSet::new();
        while let Some(id) = todo.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Ok(d) = self.doc.get_dictionary(id) else {
                continue;
            };
            let title = d
                .get(b"Title")
                .ok()
                .and_then(|o| o.as_str().ok())
                .map(|t| String::from_utf8_lossy(t).into_owned())
                .unwrap_or_default();
            out.push((title, d.has(b"Dest") || d.has(b"A")));
            // Children before the next sibling, so the order is the reading order.
            for k in [&b"Next"[..], b"First"] {
                if let Ok(r) = d.get(k).and_then(|o| o.as_reference()) {
                    todo.push(r);
                }
            }
        }
        out
    }

    fn dict<'a>(&'a self, o: &'a Object) -> Option<&'a Dictionary> {
        self.doc.dereference(o).ok()?.1.as_dict().ok()
    }

    /// `f` for each stream `o` is or holds (an appearance dictionary of states), by reference
    /// or direct.
    fn each_stream(&self, o: &Object, f: &mut dyn FnMut(&Stream)) {
        match self.doc.dereference(o).map(|(_, o)| o) {
            Ok(Object::Stream(s)) => f(s),
            Ok(Object::Dictionary(d)) => {
                for (_, v) in d.iter() {
                    if let Ok(Object::Stream(s)) = self.doc.dereference(v).map(|(_, o)| o) {
                        f(s);
                    }
                }
            }
            _ => {}
        }
    }
}

/// A form field node in the saved file.
#[derive(Debug)]
pub struct FieldState {
    pub name: String,
    /// The value entries it holds itself (`/V`, `/DV`, `/RV`, `/I`, and its widgets' `/AS`),
    /// leaving out a button's Off.
    pub values: Vec<String>,
    /// Its widgets' appearance streams, decoded, as text.
    pub appearance: String,
}

/// The ways `needle` can be written in a PDF string: as it is, in hex, and in UTF-16 (big
/// endian, as text strings are), plain and in hex.
fn forms(needle: &str) -> Vec<Vec<u8>> {
    let plain = needle.as_bytes().to_vec();
    let utf16: Vec<u8> = needle
        .encode_utf16()
        .flat_map(|u| u.to_be_bytes())
        .collect();
    let hex = |b: &[u8], upper: bool| -> Vec<u8> {
        b.iter()
            .flat_map(|x| {
                if upper {
                    format!("{x:02X}")
                } else {
                    format!("{x:02x}")
                }
                .into_bytes()
            })
            .collect()
    };
    vec![
        hex(&plain, false),
        hex(&plain, true),
        hex(&utf16, false),
        hex(&utf16, true),
        utf16,
        plain,
    ]
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

fn find_all(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    hay.windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle)
        .map(|(i, _)| i)
        .collect()
}

fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// A stream's data, decoded where lopdf can, else as stored.
fn decoded(s: &Stream) -> Vec<u8> {
    s.decompressed_content()
        .unwrap_or_else(|_| s.content.clone())
}

/// Every `stream` segment in the file by its keywords alone, reachable or not: the data up to
/// the next `endstream` (more than `/Length` says, never less), decoded when its dictionary
/// names only FlateDecode. Segments with other filters are listed as opaque. A segment that
/// says FlateDecode and doesn't decode is a failure: the checker can't see into it.
#[allow(clippy::type_complexity)]
fn segments(b: &[u8]) -> (Vec<(usize, Vec<u8>)>, Vec<(usize, String)>) {
    let mut out = Vec::new();
    let mut opaque = Vec::new();
    let mut i = 0;
    while let Some(k) = find_from(b, b"stream", i) {
        i = k + 6;
        if k >= 3 && &b[k - 3..k] == b"end" {
            continue;
        }
        let start = match b.get(i..) {
            Some([b'\r', b'\n', ..]) => i + 2,
            Some([b'\n' | b'\r', ..]) => i + 1,
            _ => continue,
        };
        let end = find_from(b, b"endstream", start).unwrap_or(b.len());
        let data = &b[start..end];
        let head = b[..k]
            .windows(3)
            .rposition(|w| w == b"obj")
            .map_or(&b[..k], |p| &b[p..k]);
        let head = String::from_utf8_lossy(head);
        let filters: Vec<&str> = [
            "FlateDecode",
            "DCTDecode",
            "JPXDecode",
            "CCITTFaxDecode",
            "JBIG2Decode",
            "LZWDecode",
            "ASCIIHexDecode",
            "ASCII85Decode",
            "RunLengthDecode",
            "Crypt",
        ]
        .into_iter()
        .filter(|f| head.contains(&format!("/{f}")))
        .collect();
        match filters.as_slice() {
            [] => out.push((k, data.to_vec())),
            ["FlateDecode"] => out.push((k, inflate(k, data))),
            other => opaque.push((k, other.join(" "))),
        }
        i = end;
    }
    (out, opaque)
}

/// The Flate (zlib) segment at byte `at`, decoded to its end with flate2 itself. lopdf gives
/// what decodes up to a fault rather than an error, so it isn't used here. A wrong checksum
/// is read past, as other readers do; data that stops short or is damaged is a failure.
fn inflate(at: usize, z: &[u8]) -> Vec<u8> {
    use flate2::{Decompress, FlushDecompress, Status};
    assert!(
        z.len() >= 2 && z[0] & 0x0f == 8 && (u16::from(z[0]) << 8 | u16::from(z[1])) % 31 == 0,
        "the stream at byte {at} has no zlib header"
    );
    let body = &z[2..];
    let mut d = Decompress::new(false);
    let mut out = Vec::with_capacity(body.len() * 4 + 1024);
    loop {
        out.reserve(out.len().max(1024));
        let (used, made) = (d.total_in(), d.total_out());
        let status = d
            .decompress_vec(&body[used as usize..], &mut out, FlushDecompress::None)
            .unwrap_or_else(|e| panic!("the stream at byte {at} doesn't decode: {e}"));
        if status == Status::StreamEnd {
            return out;
        }
        assert!(
            d.total_in() != used || d.total_out() != made,
            "the stream at byte {at} stops short"
        );
    }
}

/// Check the file's last cross-reference section, a table or an uncompressed stream:
/// `startxref` points at it, and every entry in use points at its object's `N G obj`. Then
/// lopdf read the file by it, not by a scan.
fn check_xref(b: &[u8]) {
    let s = b
        .windows(9)
        .rposition(|w| w == b"startxref")
        .expect("the file has startxref");
    let num = |at: usize| -> (usize, usize) {
        let mut i = at;
        while b[i].is_ascii_whitespace() {
            i += 1;
        }
        let j = i + b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        (std::str::from_utf8(&b[i..j]).unwrap().parse().unwrap(), j)
    };
    let points_at = |offset: usize, n: usize, gen: usize| {
        let head = format!("{n} {gen} obj");
        assert!(
            b[offset..].starts_with(head.as_bytes()),
            "the cross-reference entry for {head} points elsewhere"
        );
    };
    let (at, _) = num(s + 9);
    if !b[at..].starts_with(b"xref") {
        // A cross-reference stream, as lopdf writes it: no filter, `/W`, `/Index`.
        let (n, j) = num(at);
        let (gen, _) = num(j);
        points_at(at, n, gen);
        let data = find_from(b, b"stream", at).expect("the cross-reference stream's data");
        let dict = String::from_utf8_lossy(&b[at..data]);
        assert!(
            !dict.contains("/Filter"),
            "the checker reads only uncompressed cross-reference streams"
        );
        let ints = |key: &str| -> Vec<usize> {
            let from = dict.find(key).unwrap_or_else(|| panic!("no {key}")) + key.len();
            let open = from + dict[from..].find('[').unwrap();
            let close = open + dict[open..].find(']').unwrap();
            dict[open + 1..close]
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect()
        };
        let w = ints("/W");
        let index = ints("/Index");
        let mut i = data + 6;
        i += if b[i] == b'\r' { 2 } else { 1 };
        let field = |at: usize, len: usize| -> usize {
            b[at..at + len]
                .iter()
                .fold(0, |v, &x| (v << 8) | usize::from(x))
        };
        for pair in index.chunks(2) {
            for k in 0..pair[1] {
                let kind = if w[0] == 0 { 1 } else { field(i, w[0]) };
                let offset = field(i + w[0], w[1]);
                let gen = field(i + w[0] + w[1], w[2]);
                assert_ne!(kind, 2, "the checker doesn't read object streams");
                if kind == 1 {
                    points_at(offset, pair[0] + k, gen);
                }
                i += w.iter().sum::<usize>();
            }
        }
        return;
    }
    let mut i = at + 4;
    loop {
        while b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b[i..].starts_with(b"trailer") {
            break;
        }
        let (first, j) = num(i);
        let (count, j) = num(j);
        i = j;
        while b[i] != b'\n' {
            i += 1;
        }
        i += 1;
        for n in 0..count {
            let e = &b[i..i + 20];
            if e[17] == b'n' {
                let offset: usize = std::str::from_utf8(&e[..10]).unwrap().parse().unwrap();
                let gen: usize = std::str::from_utf8(&e[11..16]).unwrap().parse().unwrap();
                points_at(offset, first + n, gen);
            }
            i += 20;
        }
    }
}

/// `f` for every string in `o`, direct values included.
fn each_string(o: &Object, depth: usize, f: &mut dyn FnMut(&[u8])) {
    assert!(
        depth < MAX_DEPTH,
        "objects nest deeper than the checker follows"
    );
    match o {
        Object::String(s, _) => f(s),
        Object::Array(a) => a.iter().for_each(|x| each_string(x, depth + 1, f)),
        Object::Dictionary(d) => d.iter().for_each(|(_, v)| each_string(v, depth + 1, f)),
        Object::Stream(s) => s
            .dict
            .iter()
            .for_each(|(_, v)| each_string(v, depth + 1, f)),
        _ => {}
    }
}

/// `f` for every reference in `o`.
fn each_reference(o: &Object, depth: usize, f: &mut dyn FnMut(ObjectId)) {
    assert!(
        depth < MAX_DEPTH,
        "objects nest deeper than the checker follows"
    );
    match o {
        Object::Reference(r) => f(*r),
        Object::Array(a) => a.iter().for_each(|x| each_reference(x, depth + 1, f)),
        Object::Dictionary(d) => d.iter().for_each(|(_, v)| each_reference(v, depth + 1, f)),
        Object::Stream(s) => s
            .dict
            .iter()
            .for_each(|(_, v)| each_reference(v, depth + 1, f)),
        _ => {}
    }
}

/// `f` for every dictionary in `o` (a stream's included), direct ones inside it too, with a
/// path to it.
fn each_dict(o: &Object, path: &str, depth: usize, f: &mut dyn FnMut(&str, &Dictionary)) {
    assert!(
        depth < MAX_DEPTH,
        "objects nest deeper than the checker follows"
    );
    let d = match o {
        Object::Dictionary(d) => d,
        Object::Stream(s) => &s.dict,
        Object::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                each_dict(x, &format!("{path} [{i}]"), depth + 1, f);
            }
            return;
        }
        _ => return,
    };
    f(path, d);
    for (k, v) in d.iter() {
        let p = format!("{path} /{}", String::from_utf8_lossy(k));
        each_dict(v, &p, depth + 1, f);
    }
}

/// Outside readers, for the opt-in `cross_check` tests.
pub mod tools {
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    /// The standard output of `tool` run on `bytes` written to a file; `{}` in `args` is the
    /// file's path.
    pub fn run(tool: &str, args: &[&str], bytes: &[u8]) -> String {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("limarepdf-saved-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("saved.pdf");
        std::fs::write(&path, bytes).unwrap();
        let p = path.to_str().unwrap();
        let args: Vec<&str> = args
            .iter()
            .map(|a| if *a == "{}" { p } else { a })
            .collect();
        let out = Command::new(tool)
            .args(&args)
            .output()
            .unwrap_or_else(|e| panic!("{tool} installed: {e}"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            out.status.success(),
            "{tool} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// The page text as poppler reads it.
    pub fn pdftotext(bytes: &[u8]) -> String {
        run("pdftotext", &["{}", "-"], bytes)
    }

    /// The text Ghostscript draws, form fields and annotations included.
    pub fn gs_text(bytes: &[u8]) -> String {
        run(
            "gs",
            &[
                "-q",
                "-dNOPAUSE",
                "-dBATCH",
                "-dSAFER",
                "-sDEVICE=txtwrite",
                "-sOutputFile=-",
                "{}",
            ],
            bytes,
        )
    }

    /// The JavaScript poppler finds in the document.
    pub fn pdfinfo_js(bytes: &[u8]) -> String {
        run("pdfinfo", &["-js", "{}"], bytes)
    }
}
