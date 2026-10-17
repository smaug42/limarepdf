//! What changed in a signed document after each signature, and whether the signer allowed
//! it: Acrobat's allowed-changes analysis. See docs/signatures.md.
//!
//! A change made after signing is appended to the file as a new revision. For each
//! signature, the revisions after it are read with MPE's own strict reader of
//! cross-reference sections, because lopdf is lenient in ways that would hide changes (it
//! ignores free entries, corrects bad offsets and rebuilds broken tables). lopdf is used
//! only for object contents, after checking that it reads the same objects.
//!
//! Each version a later signature signed is compared with the one before it, so a change
//! made and undone between two signatures still counts. Every object that differs is
//! judged by each role it plays in the newer version (page content, comment, form field,
//! signature, ...); a change no role allows is disallowed, and so is anything MPE doesn't
//! recognise. Links between structures (a bookmark pointing to a page, a tag pointing to an
//! annotation) don't make the target part of the linking structure.

use crate::sig_verify::{is_white, FieldLock, Lexer, Permissions, Tok, Versions};
use crate::Pdf;
use lopdf::xref::XrefEntry;
use lopdf::{Dictionary, Object, ObjectId, Stream, StringFormat};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

// ---------------------------------------------------------------- report types

/// Something that changed after a signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    /// The page it is on (zero-based), where it has one.
    pub page: Option<usize>,
    /// Why the signer didn't allow it; `None` when allowed.
    pub refused: Option<Refusal>,
}

impl Change {
    pub fn allowed(&self) -> bool {
        self.refused.is_none()
    }
}

/// The kinds of change, following Acrobat's list.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeKind {
    /// A form field's value (and its appearance) changed.
    FieldFilled(String),
    /// An empty signature field was signed.
    Signed(String),
    /// A document timestamp was added.
    DocumentTimestamp(String),
    /// A signature field was added (signed in the same step or not).
    SignatureFieldAdded(String),
    CommentAdded,
    CommentChanged,
    CommentRemoved,
    /// Form-wide settings used for filling in (fonts, default appearance).
    FormSettings,
    /// Appearances stored for comment icons (Acrobat keeps them in the `/Names /AP` tree).
    CommentAppearances,
    /// Certificates and revocation information for long-term validation (`/DSS`).
    ValidationData,
    /// Adobe Reader usage rights removed (as Acrobat does when it saves).
    UsageRightsRemoved,
    /// The document information or XMP metadata.
    Metadata,
    // Never allowed after signing:
    PageContent,
    /// Pages added, removed or reordered.
    PagesChanged,
    /// A link or another annotation that isn't a comment.
    AnnotationChanged,
    FieldAdded(String),
    FieldRemoved(String),
    /// Anything about a field other than its value: position, type, flags, actions.
    FieldChanged(String),
    /// A signature, its dictionary or its appearance changed, or it was removed.
    SignatureChanged(String),
    /// Actions or scripts.
    Actions,
    /// Document-level structure, with what changed.
    DocumentChanged(String),
    /// A change MPE doesn't recognise, with where.
    Unrecognised(String),
    /// The appended revisions aren't well formed, with why.
    Malformed(String),
}

/// Why a change isn't allowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Never allowed after signing.
    Never,
    /// The document's certification allows less.
    Certification(Permissions),
    /// This signature locked the field, or limited changes to the document.
    Locked,
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::FieldFilled(n) => write!(f, "form field \"{n}\" filled in"),
            Self::Signed(n) => write!(f, "signed in field \"{n}\""),
            Self::DocumentTimestamp(_) => write!(f, "document timestamp added"),
            Self::SignatureFieldAdded(n) => write!(f, "signature field \"{n}\" added"),
            Self::CommentAdded => write!(f, "comment added"),
            Self::CommentChanged => write!(f, "comment changed"),
            Self::CommentRemoved => write!(f, "comment removed"),
            Self::FormSettings => write!(f, "form settings for filling in changed"),
            Self::CommentAppearances => write!(f, "appearances for comments stored"),
            Self::ValidationData => write!(f, "validation data added"),
            Self::UsageRightsRemoved => write!(f, "Adobe Reader usage rights removed"),
            Self::Metadata => write!(f, "document information changed"),
            Self::PageContent => write!(f, "page content changed"),
            Self::PagesChanged => write!(f, "pages added, removed or reordered"),
            Self::AnnotationChanged => {
                write!(f, "a link or other annotation that isn't a comment changed")
            }
            Self::FieldAdded(n) => write!(f, "form field \"{n}\" added"),
            Self::FieldRemoved(n) => write!(f, "form field \"{n}\" removed"),
            Self::FieldChanged(n) => write!(f, "form field \"{n}\" changed beyond its value"),
            Self::SignatureChanged(n) => {
                write!(f, "the signature in field \"{n}\" changed or was removed")
            }
            Self::Actions => write!(f, "actions or scripts added or changed"),
            Self::DocumentChanged(w) => write!(f, "document structure changed ({w})"),
            Self::Unrecognised(w) => write!(f, "a change MPE doesn't recognise ({w})"),
            Self::Malformed(w) => write!(f, "the later additions are malformed: {w}"),
        }
    }
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.kind)?;
        if let Some(p) = self.page {
            write!(f, " on page {}", p + 1)?;
        }
        Ok(())
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Self::Never => "never allowed after signing",
            Self::Certification(Permissions::NoChanges) => "the certification allows no changes",
            Self::Certification(Permissions::FormFilling) => {
                "the certification allows only form filling and signing"
            }
            Self::Certification(Permissions::FormFillingAndComments) => {
                "the certification allows only form filling, signing and comments"
            }
            Self::Locked => "this signature locked it",
        })
    }
}

// ---------------------------------------------------------------- entry point

/// What one signature needs for the analysis.
pub(crate) struct Signed<'a> {
    /// Where its signed version ends.
    pub end: usize,
    pub locks: Option<&'a FieldLock>,
}

/// The changes after each signature in `sigs`, judged against the certification
/// `certified` (the strictest level any certification in the document claims) and each
/// signature's own locks. `versions` opens the signed versions.
pub(crate) fn analyse(
    versions: &mut Versions,
    sigs: &[Signed],
    certified: Option<Permissions>,
) -> Vec<Vec<Change>> {
    let bytes = versions.bytes;
    let ends: Vec<usize> = sigs.iter().map(|s| s.end).collect();
    let mut cache: HashMap<(usize, usize), Vec<Seen>> = HashMap::new();
    sigs.iter()
        .map(|s| {
            let seen = changes_after(versions, bytes, s.end, &ends, &mut cache);
            let mut out: Vec<Change> = Vec::new();
            for x in seen {
                let c = Change {
                    refused: refusal(&x.need, certified, s.locks),
                    kind: x.kind,
                    page: x.page,
                };
                if !out.contains(&c) {
                    out.push(c);
                }
            }
            out.sort_by(|a, b| (a.allowed(), &a.kind, a.page).cmp(&(b.allowed(), &b.kind, b.page)));
            out
        })
        .collect()
}

/// What a change needs to be allowed.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Need {
    Never,
    /// Always allowed: validation data, metadata.
    Nothing,
    /// Form filling, of this field (`None`: form-wide settings).
    Fill(Option<String>),
    /// Signing in `field`. `new_visible` is set for a field added after signing: whether it
    /// shows on the page. A document timestamp needs no form-filling permission.
    Sign {
        field: String,
        new_visible: Option<bool>,
        timestamp: bool,
    },
    Comment,
}

/// A change found, before judging it for a particular signature.
#[derive(Clone, Debug)]
struct Seen {
    kind: ChangeKind,
    page: Option<usize>,
    need: Need,
}

fn seen(kind: ChangeKind, page: Option<usize>, need: Need) -> Seen {
    Seen { kind, page, need }
}

fn never(kind: ChangeKind, page: Option<usize>) -> Seen {
    seen(kind, page, Need::Never)
}

fn allowed_at(need: &Need, p: Permissions) -> bool {
    match need {
        Need::Never => false,
        Need::Nothing => true,
        Need::Fill(_) => p >= Permissions::FormFilling,
        Need::Sign {
            new_visible,
            timestamp,
            ..
        } => (*timestamp || p >= Permissions::FormFilling) && *new_visible != Some(true),
        Need::Comment => p >= Permissions::FormFillingAndComments,
    }
}

fn refusal(
    need: &Need,
    certified: Option<Permissions>,
    lock: Option<&FieldLock>,
) -> Option<Refusal> {
    if *need == Need::Never {
        return Some(Refusal::Never);
    }
    if let Some(p) = certified.filter(|&p| !allowed_at(need, p)) {
        return Some(Refusal::Certification(p));
    }
    let lock = lock?;
    if lock.permissions.is_some_and(|p| !allowed_at(need, p)) {
        return Some(Refusal::Locked);
    }
    let field = match need {
        Need::Fill(Some(f)) => f,
        Need::Sign {
            field,
            timestamp: false,
            ..
        } => field,
        _ => return None,
    };
    lock.locks(field).then_some(Refusal::Locked)
}

/// The changes after the version ending at `end`, version by version: each later
/// signature's signed version is compared with the one before it, and the whole file with
/// the last.
fn changes_after(
    versions: &mut Versions,
    bytes: &[u8],
    end: usize,
    sig_ends: &[usize],
    cache: &mut HashMap<(usize, usize), Vec<Seen>>,
) -> Vec<Seen> {
    if bytes[end..].iter().all(|&c| is_white(c)) {
        return Vec::new();
    }
    let malformed = |why: String| vec![never(ChangeKind::Malformed(why), None)];
    let sections = match read_chain(bytes, end) {
        Ok(s) => s,
        Err(why) => return malformed(why),
    };
    // The versions to compare: this one, each later signed version that is one of the
    // revisions read, and the whole file.
    let mut anchors = vec![end];
    let mut later: Vec<usize> = sig_ends
        .iter()
        .copied()
        .filter(|&e| e > end && e < bytes.len())
        .filter(|&e| {
            sections
                .iter()
                .any(|s| s.end <= e && bytes[s.end..e].iter().all(|&c| is_white(c)))
        })
        .collect();
    later.sort_unstable();
    later.dedup();
    anchors.extend(later);
    anchors.push(bytes.len());

    let mut out = Vec::new();
    for w in anchors.windows(2) {
        let (a, b) = (w[0], w[1]);
        // Newest first, as read.
        let between: Vec<&Section> = sections
            .iter()
            .filter(|s| s.offset >= a && s.end <= b)
            .collect();
        if between.is_empty() {
            continue;
        }
        if let Some(found) = cache.get(&(a, b)) {
            out.extend(found.iter().cloned());
            continue;
        }
        let before: Vec<&Section> = sections
            .iter()
            .filter(|s| s.offset >= end && s.end <= a)
            .collect();
        let found = compare_versions(versions, end, a, b, &before, &between);
        let stop = found
            .iter()
            .any(|s| matches!(s.kind, ChangeKind::Malformed(_)));
        out.extend(found.iter().cloned());
        cache.insert((a, b), found);
        if stop {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------- cross-reference sections

/// The most cross-reference sections read after a signature; more is refused as
/// malformed rather than spending unbounded time.
const MAX_SECTIONS: usize = 2000;
/// The largest decoded cross-reference stream accepted (bytes).
const MAX_XREF_STREAM: usize = 64 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Free,
    InUse { offset: usize, gen: u16 },
    Compressed { container: u32, index: u32 },
}

/// One revision appended after a signature: its cross-reference section.
#[derive(Debug)]
struct Section {
    /// Where the section starts, and where its revision ends (just after `%%EOF` and
    /// its end of line).
    offset: usize,
    end: usize,
    /// Its entries in precedence order: for a number listed twice, the first counts.
    entries: Vec<(u32, Entry)>,
    /// Where a hybrid revision's cross-reference stream starts.
    stream_at: Option<usize>,
}

/// The `startxref` value of the revision `prefix` ends with (`startxref N %%EOF`, perhaps
/// followed by white space).
fn last_startxref(prefix: &[u8]) -> Option<usize> {
    let t = trim_white_end(prefix).strip_suffix(b"%%EOF")?;
    let t = trim_white_end(t);
    let digits = t.iter().rev().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 15 {
        return None;
    }
    let n = std::str::from_utf8(&t[t.len() - digits..])
        .ok()?
        .parse()
        .ok()?;
    trim_white_end(&t[..t.len() - digits])
        .ends_with(b"startxref")
        .then_some(n)
}

fn trim_white_end(b: &[u8]) -> &[u8] {
    let n = b.iter().rev().take_while(|&&c| is_white(c)).count();
    &b[..b.len() - n]
}

/// The cross-reference sections appended after the version ending at `signed_end`,
/// newest first, read from the end of the file along the `/Prev` chain, which must lead
/// back to that version's own section.
fn read_chain(bytes: &[u8], signed_end: usize) -> Result<Vec<Section>, String> {
    let signed_xref = last_startxref(&bytes[..signed_end])
        .ok_or("the signed version doesn't end with a cross-reference pointer")?;
    let mut at = last_startxref(bytes).ok_or(
        "the file doesn't end with a complete revision (data after the last end-of-file \
         marker, or no cross-reference pointer)",
    )?;
    let mut sections: Vec<Section> = Vec::new();
    let mut seen = HashSet::new();
    loop {
        if at < signed_end {
            if at != signed_xref {
                return Err("a later revision doesn't build on the signed version".into());
            }
            return Ok(sections);
        }
        if !seen.insert(at) || sections.len() >= MAX_SECTIONS {
            return Err("the chain of revisions loops or is too long".into());
        }
        let read = read_section(bytes, at)?;
        let end = revision_end(bytes, read.after, at)
            .ok_or_else(|| format!("the revision at byte {at} isn't closed properly"))?;
        if let Some(newer) = sections.last() {
            if end > newer.offset {
                return Err("revisions overlap".into());
            }
        } else if end < bytes.len() && !bytes[end..].iter().all(|&c| is_white(c)) {
            return Err("data follows the last revision".into());
        }
        let mut entries = read.entries;
        // A hybrid file: a cross-reference stream adds to the table, ranking below it.
        if let Some(stm) = read.xref_stm {
            if stm < signed_end || stm >= at {
                return Err("a later revision points back into earlier data".into());
            }
            let extra = read_section(bytes, stm)?;
            if extra.kind != SectionKind::Stream {
                return Err("a hybrid revision's stream isn't a cross-reference stream".into());
            }
            entries.extend(extra.entries);
        }
        sections.push(Section {
            offset: at,
            end,
            entries,
            stream_at: read.xref_stm,
        });
        at = read
            .prev
            .ok_or("a later revision replaces the whole cross-reference instead of adding to it")?;
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SectionKind {
    Table,
    Stream,
}

struct ReadSection {
    kind: SectionKind,
    entries: Vec<(u32, Entry)>,
    prev: Option<usize>,
    xref_stm: Option<usize>,
    /// Where the section (with its trailer, or its object) ends.
    after: usize,
}

fn read_section(bytes: &[u8], at: usize) -> Result<ReadSection, String> {
    let bad = || format!("the cross-reference section at byte {at} is malformed");
    let mut lx = Lexer::new(bytes, at);
    let (start, tok, _) = lx.next().ok_or_else(bad)?;
    if !bytes[at..start].iter().all(|&c| is_white(c)) {
        return Err(bad());
    }
    match tok {
        Tok::Word(b"xref") => read_table(bytes, lx).ok_or_else(bad),
        Tok::Word(_) => read_xref_stream(bytes, Lexer::new(bytes, at)).ok_or_else(bad),
        _ => Err(bad()),
    }
}

fn word_num<T: std::str::FromStr>(t: Option<(usize, Tok, usize)>) -> Option<T> {
    match t? {
        (_, Tok::Word(w), _) if w.iter().all(u8::is_ascii_digit) => {
            std::str::from_utf8(w).ok()?.parse().ok()
        }
        _ => None,
    }
}

fn read_table(bytes: &[u8], mut lx: Lexer) -> Option<ReadSection> {
    let mut entries = Vec::new();
    loop {
        let mut look = lx.clone();
        if let Some((_, Tok::Word(b"trailer"), _)) = look.next() {
            lx = look;
            break;
        }
        let first: u32 = word_num(lx.next())?;
        let count: usize = word_num(lx.next())?;
        if count > bytes.len() / 18 {
            return None;
        }
        for i in 0..count {
            let num = first.checked_add(u32::try_from(i).ok()?)?;
            let offset: usize = word_num(lx.next())?;
            let gen: u32 = word_num(lx.next())?;
            let e = match lx.next()? {
                (_, Tok::Word(b"n"), _) => Entry::InUse {
                    offset,
                    gen: u16::try_from(gen).ok()?,
                },
                (_, Tok::Word(b"f"), _) => Entry::Free,
                _ => return None,
            };
            entries.push((num, e));
        }
    }
    let trailer = match read_object(&mut lx, 0)? {
        Object::Dictionary(d) => d,
        _ => return None,
    };
    Some(ReadSection {
        kind: SectionKind::Table,
        entries,
        prev: direct_offset(&trailer, b"Prev")?,
        xref_stm: direct_offset(&trailer, b"XRefStm")?,
        after: lx.pos(),
    })
}

/// An optional offset entry: absent is fine, present must be a whole number.
fn direct_offset(d: &Dictionary, key: &[u8]) -> Option<Option<usize>> {
    match d.get(key) {
        Err(_) => Some(None),
        Ok(Object::Integer(n)) => Some(Some(usize::try_from(*n).ok()?)),
        Ok(_) => None,
    }
}

fn read_xref_stream(bytes: &[u8], mut lx: Lexer) -> Option<ReadSection> {
    let _: u32 = word_num(lx.next())?;
    let _: u16 = word_num(lx.next())?;
    if !matches!(lx.next()?, (_, Tok::Word(b"obj"), _)) {
        return None;
    }
    let dict = match read_object(&mut lx, 0)? {
        Object::Dictionary(d) => d,
        _ => return None,
    };
    let (_, Tok::Word(b"stream"), mut start) = lx.next()? else {
        return None;
    };
    start += match bytes.get(start..start + 2)? {
        b"\r\n" => 2,
        [b'\n', _] => 1,
        _ => return None,
    };
    let len = usize::try_from(dict.get(b"Length").ok()?.as_i64().ok()?).ok()?;
    let data = bytes.get(start..start.checked_add(len)?)?;
    let mut tail = Lexer::new(bytes, start + len);
    if !matches!(tail.next()?, (_, Tok::Word(b"endstream"), _))
        || !matches!(tail.next()?, (_, Tok::Word(b"endobj"), _))
    {
        return None;
    }
    if dict.get(b"Type").ok()?.as_name().ok()? != b"XRef" {
        return None;
    }
    let decoded = Stream::new(dict.clone(), data.to_vec())
        .decompressed_content_with_limit(MAX_XREF_STREAM)
        .ok()?;
    let ints = |key: &[u8]| -> Option<Vec<i64>> {
        dict.get(key)
            .ok()?
            .as_array()
            .ok()?
            .iter()
            .map(|o| o.as_i64().ok())
            .collect()
    };
    let w: Vec<usize> = ints(b"W")?
        .into_iter()
        .map(|x| usize::try_from(x).ok().filter(|&x| x <= 8))
        .collect::<Option<_>>()?;
    let [w0, w1, w2] = w[..] else {
        return None;
    };
    let row = w0 + w1 + w2;
    if row == 0 {
        return None;
    }
    let size = dict.get(b"Size").ok()?.as_i64().ok()?;
    let index = match dict.get(b"Index") {
        Ok(_) => ints(b"Index")?,
        Err(_) => vec![0, size],
    };
    if index.len() % 2 != 0 {
        return None;
    }
    let field = |b: &[u8]| b.iter().fold(0u64, |a, &x| (a << 8) | u64::from(x));
    let mut entries = Vec::new();
    let mut pos = 0;
    for pair in index.chunks(2) {
        let first = u32::try_from(pair[0]).ok()?;
        let count = usize::try_from(pair[1]).ok()?;
        if count > decoded.len() / row {
            return None;
        }
        for i in 0..count {
            let r = decoded.get(pos..pos + row)?;
            pos += row;
            let kind = if w0 == 0 { 1 } else { field(&r[..w0]) };
            let (f1, f2) = (field(&r[w0..w0 + w1]), field(&r[w0 + w1..]));
            let num = first.checked_add(u32::try_from(i).ok()?)?;
            let e = match kind {
                0 => Entry::Free,
                1 => Entry::InUse {
                    offset: usize::try_from(f1).ok()?,
                    gen: u16::try_from(f2).ok()?,
                },
                2 => Entry::Compressed {
                    container: u32::try_from(f1).ok()?,
                    index: u32::try_from(f2).ok()?,
                },
                _ => return None,
            };
            entries.push((num, e));
        }
    }
    Some(ReadSection {
        kind: SectionKind::Stream,
        entries,
        prev: direct_offset(&dict, b"Prev")?,
        xref_stm: None,
        after: tail.pos(),
    })
}

/// Where the revision whose section starts at `at` ends: after the section (ending at
/// `after`) come `startxref`, `at` and `%%EOF`.
fn revision_end(bytes: &[u8], after: usize, at: usize) -> Option<usize> {
    let mut lx = Lexer::new(bytes, after);
    if !matches!(lx.next()?, (_, Tok::Word(b"startxref"), _)) || word_num::<usize>(lx.next())? != at
    {
        return None;
    }
    let mut p = lx.pos();
    while bytes.get(p).is_some_and(|&c| is_white(c)) {
        p += 1;
    }
    let mut end = p + bytes.get(p..)?.strip_prefix(b"%%EOF").map(|_| 5)?;
    if bytes.get(end..end + 2) == Some(b"\r\n") {
        end += 2;
    } else if matches!(bytes.get(end), Some(b'\n' | b'\r')) {
        end += 1;
    }
    Some(end)
}

/// One direct object, enough for cross-reference dictionaries: numbers, names (with
/// `#xx` escapes), references, arrays, dictionaries. Strings are kept empty.
fn read_object(lx: &mut Lexer, depth: usize) -> Option<Object> {
    if depth > 32 {
        return None;
    }
    let (_, tok, _) = lx.next()?;
    Some(match tok {
        Tok::DictOpen => {
            let mut d = Dictionary::new();
            loop {
                match lx.next()? {
                    (_, Tok::DictClose, _) => break,
                    (_, Tok::Name(k), _) => {
                        let v = read_object(lx, depth + 1)?;
                        d.set(decode_name(k)?, v);
                    }
                    _ => return None,
                }
            }
            Object::Dictionary(d)
        }
        Tok::ArrayOpen => {
            let mut a = Vec::new();
            loop {
                let mut look = lx.clone();
                if let (_, Tok::ArrayClose, _) = look.next()? {
                    *lx = look;
                    break;
                }
                a.push(read_object(lx, depth + 1)?);
            }
            Object::Array(a)
        }
        Tok::Name(n) => Object::Name(decode_name(n)?),
        Tok::Hex | Tok::Literal => Object::String(Vec::new(), StringFormat::Literal),
        Tok::Word(b"true") => Object::Boolean(true),
        Tok::Word(b"false") => Object::Boolean(false),
        Tok::Word(b"null") => Object::Null,
        Tok::Word(w) => {
            let s = std::str::from_utf8(w).ok()?;
            match s.parse::<i64>() {
                Ok(n) => {
                    // Perhaps `N G R`.
                    let mut look = lx.clone();
                    let gen: Option<u16> = word_num(look.next());
                    let r = matches!(look.next(), Some((_, Tok::Word(b"R"), _)));
                    match (gen, r, u32::try_from(n)) {
                        (Some(g), true, Ok(num)) => {
                            *lx = look;
                            Object::Reference((num, g))
                        }
                        _ => Object::Integer(n),
                    }
                }
                Err(_) => Object::Real(s.parse().ok()?),
            }
        }
        Tok::DictClose | Tok::ArrayClose => return None,
    })
}

fn decode_name(n: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(n.len());
    let mut i = 0;
    while i < n.len() {
        if n[i] == b'#' {
            let h = std::str::from_utf8(n.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(n[i]);
            i += 1;
        }
    }
    Some(out)
}

/// Whether object `id` starts at `offset`: `N G obj`.
fn header_at(bytes: &[u8], offset: usize, id: ObjectId) -> bool {
    let mut lx = Lexer::new(bytes, offset);
    let first = lx.next();
    if !first.is_some_and(|(s, _, _)| bytes[offset..s].iter().all(|&c| is_white(c))) {
        return false;
    }
    word_num::<u32>(first) == Some(id.0)
        && word_num::<u16>(lx.next()) == Some(id.1)
        && matches!(lx.next(), Some((_, Tok::Word(b"obj"), _)))
}

// ---------------------------------------------------------------- one step

/// A version of the document: lopdf's reading, with the objects MPE's reader found freed
/// hidden (lopdf ignores free entries and would still show them).
struct Version<'a> {
    pdf: &'a Pdf,
    freed: HashSet<u32>,
}

impl Version<'_> {
    fn get(&self, id: ObjectId) -> Option<&Object> {
        if self.freed.contains(&id.0) {
            return None;
        }
        self.pdf.doc.objects.get(&id)
    }

    /// `o`, following references (a few steps at most).
    fn resolve<'b>(&'b self, mut o: &'b Object) -> Option<&'b Object> {
        for _ in 0..8 {
            match o {
                Object::Reference(id) => o = self.get(*id)?,
                o => return Some(o),
            }
        }
        None
    }

    fn dict<'b>(&'b self, o: &'b Object) -> Option<&'b Dictionary> {
        match self.resolve(o)? {
            Object::Dictionary(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    fn dict_of(&self, id: ObjectId) -> Option<&Dictionary> {
        match self.get(id)? {
            Object::Dictionary(d) => Some(d),
            _ => None,
        }
    }

    fn array<'b>(&'b self, o: &'b Object) -> Option<&'b Vec<Object>> {
        self.resolve(o)?.as_array().ok()
    }

    fn name<'b>(&'b self, d: &'b Dictionary, key: &[u8]) -> Option<&'b [u8]> {
        self.resolve(d.get(key).ok()?)?.as_name().ok()
    }

    /// The ids lopdf holds for object number `num`.
    fn ids(&self, num: u32) -> Vec<ObjectId> {
        if self.freed.contains(&num) {
            return Vec::new();
        }
        self.pdf
            .doc
            .objects
            .range((num, 0)..=(num, u16::MAX))
            .map(|(id, _)| *id)
            .collect()
    }
}

/// Compares the version ending at `a` with the one ending at `b`. `before` are the
/// sections between the signature's own version (ending at `sig_end`) and `a`; `between`
/// those between `a` and `b`; both newest first.
fn compare_versions(
    versions: &mut Versions,
    sig_end: usize,
    a: usize,
    b: usize,
    before: &[&Section],
    between: &[&Section],
) -> Vec<Seen> {
    let malformed = |why: String| vec![never(ChangeKind::Malformed(why), None)];
    let bytes = versions.bytes;
    // Which objects the sections change, and which they free, newest entry first.
    let effective = |secs: &[&Section]| {
        let mut m: HashMap<u32, Entry> = HashMap::new();
        for s in secs {
            for &(num, e) in &s.entries {
                if num != 0 {
                    m.entry(num).or_insert(e);
                }
            }
        }
        m
    };
    let freed = |m: &HashMap<u32, Entry>| -> HashSet<u32> {
        m.iter()
            .filter(|(_, e)| **e == Entry::Free)
            .map(|(n, _)| *n)
            .collect()
    };
    let newest = effective(between);
    // Frees before `a`, unless `between` reuses the number.
    let old_freed = freed(&effective(before));
    let new_freed: HashSet<u32> = {
        let mut all: Vec<&Section> = between.to_vec();
        all.extend(before.iter().copied());
        freed(&effective(&all))
    };
    versions.load(a);
    versions.load(b);
    let (Some(old_pdf), Some(new_pdf)) = (versions.peek(a), versions.peek(b)) else {
        return malformed(if a == sig_end {
            "MPE can't read the document as it is after signing".into()
        } else {
            "MPE can't read a later signed version".into()
        });
    };
    let old = Version {
        pdf: old_pdf,
        freed: old_freed,
    };
    let new = Version {
        pdf: new_pdf,
        freed: new_freed,
    };

    // lopdf must read the same objects MPE's reader finds.
    let old_table = &old.pdf.doc.reference_table.entries;
    let new_table = &new.pdf.doc.reference_table.entries;
    // Cross-reference streams are objects too, which lopdf may not keep.
    let section_starts: HashSet<usize> = between
        .iter()
        .flat_map(|s| [Some(s.offset), s.stream_at])
        .flatten()
        .collect();
    let mut changed: HashSet<u32> = HashSet::new();
    for (&num, &e) in &newest {
        changed.insert(num);
        let ok = match e {
            Entry::Free => true,
            Entry::InUse { offset, gen } => {
                if offset < a || offset >= b {
                    return malformed(format!(
                        "object {num} points outside the revision that changes it"
                    ));
                }
                if !header_at(bytes, offset, (num, gen)) {
                    return malformed(format!(
                        "object {num} isn't where its cross-reference entry says"
                    ));
                }
                matches!(new_table.get(&num), Some(XrefEntry::Normal { offset: o, generation: g })
                    if *o as usize == offset && *g == gen)
                    && (section_starts.contains(&offset)
                        || new.pdf.doc.objects.contains_key(&(num, gen)))
            }
            Entry::Compressed { container, index } => {
                let fresh = matches!(newest.get(&container), Some(Entry::InUse { .. }));
                if !fresh {
                    return malformed(format!(
                        "object {num} is stored in an object stream from before"
                    ));
                }
                matches!(new_table.get(&num), Some(XrefEntry::Compressed { container: c, index: i })
                    if *c == container && u32::from(*i) == index)
                    && new.pdf.doc.objects.contains_key(&(num, 0))
            }
        };
        if !ok {
            return malformed(format!("MPE can't read object {num} reliably"));
        }
    }
    // Everything else reads as before, except objects held in a rewritten object stream.
    let nums: HashSet<u32> = old_table.keys().chain(new_table.keys()).copied().collect();
    for num in nums {
        if newest.contains_key(&num) {
            continue;
        }
        let (o, n) = (old_table.get(&num), new_table.get(&num));
        if let Some(XrefEntry::Compressed { container, .. }) = o {
            if newest.contains_key(container) {
                changed.insert(num);
                continue;
            }
        }
        let same = match (o, n) {
            (None, None) => true,
            (Some(x), Some(y)) => same_entry(x, y),
            _ => false,
        };
        if !same {
            return malformed(format!("MPE can't read object {num} reliably"));
        }
    }
    let ids = |v: &Version| -> HashSet<ObjectId> { v.pdf.doc.objects.keys().copied().collect() };
    for id in ids(&old).union(&ids(&new)) {
        if changed.contains(&id.0) {
            continue;
        }
        let same = match (old.pdf.doc.objects.get(id), new.pdf.doc.objects.get(id)) {
            (Some(x), Some(y)) => same_object(x, y),
            (None, None) => true,
            _ => false,
        };
        if !same {
            return malformed(format!("MPE can't read object {} reliably", id.0));
        }
    }

    let (old_roles, new_roles) = match (Roles::build(&old), Roles::build(&new)) {
        (Ok(o), Ok(n)) => (o, n),
        (Err(why), _) | (_, Err(why)) => return malformed(why),
    };
    let mut j = Judge {
        old: &old,
        new: &new,
        or: &old_roles,
        nr: &new_roles,
        out: Vec::new(),
    };
    j.trailer();
    j.pages();
    j.annotations();
    j.fields();
    let mut changed: Vec<u32> = changed.into_iter().collect();
    changed.sort_unstable();
    for num in changed {
        let mut ids: Vec<ObjectId> = old.ids(num);
        ids.extend(new.ids(num));
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            j.object(id);
        }
    }
    j.out
}

fn same_entry(a: &XrefEntry, b: &XrefEntry) -> bool {
    match (a, b) {
        (
            XrefEntry::Normal {
                offset: o1,
                generation: g1,
            },
            XrefEntry::Normal {
                offset: o2,
                generation: g2,
            },
        ) => o1 == o2 && g1 == g2,
        (
            XrefEntry::Compressed {
                container: c1,
                index: i1,
            },
            XrefEntry::Compressed {
                container: c2,
                index: i2,
            },
        ) => c1 == c2 && i1 == i2,
        (XrefEntry::Free, XrefEntry::Free) | (XrefEntry::UnusableFree, XrefEntry::UnusableFree) => {
            true
        }
        _ => false,
    }
}

/// Objects equal in value: how a string is written (literal or hex) and where a stream
/// sits in the file don't count. Acrobat rewrites strings in its own way when saving.
fn same_object(a: &Object, b: &Object) -> bool {
    same_at(a, b, 0)
}

fn same_at(a: &Object, b: &Object, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    match (a, b) {
        (Object::String(x, _), Object::String(y, _)) => x == y,
        (Object::Array(x), Object::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_at(p, q, depth + 1))
        }
        (Object::Dictionary(x), Object::Dictionary(y)) => same_dict(x, y, depth),
        (Object::Stream(x), Object::Stream(y)) => {
            x.content == y.content && same_dict(&x.dict, &y.dict, depth)
        }
        _ => a == b,
    }
}

fn same_dict(x: &Dictionary, y: &Dictionary, depth: usize) -> bool {
    x.len() == y.len()
        && x.iter()
            .all(|(k, v)| y.get(k).is_ok_and(|w| same_at(v, w, depth + 1)))
}

fn same_value(a: Option<&Object>, b: Option<&Object>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => same_object(x, y),
        (None, None) => true,
        _ => false,
    }
}

// ---------------------------------------------------------------- roles

/// The roles an object plays in a version of the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Catalog,
    Info,
    Encrypt,
    Perms,
    Metadata,
    Dss,
    Extensions,
    /// The document's name trees (`/Names`), and the comment icon appearances in it.
    Names,
    NamedAppearance,
    /// Any other document-level structure.
    Other,
    /// An action or script.
    Action,
    /// A page or a node of the page tree.
    PageNode,
    /// An indirect `/Kids` array of the page tree.
    PagesArray,
    /// An indirect `/Annots` array.
    AnnotsArray,
    /// Anything a page draws or uses: contents, resources, fonts, images.
    PageContent,
    /// A comment (a markup annotation or a popup), and anything under it.
    Comment,
    CommentPart,
    /// A link or another annotation that isn't a comment, and anything under it.
    OtherAnnot,
    OtherAnnotPart,
    /// A widget annotation that is also a form field.
    WidgetAnnot,
    /// A form field other than a signature field, and anything under it (appearances).
    Field,
    FieldPart,
    SigField,
    SigFieldPart,
    /// A signature field's lock and seed value.
    FieldMeta,
    /// An indirect `/Fields` or `/Kids` array of the form.
    FieldsArray,
    /// A signature dictionary, and anything under it.
    SigDict,
    SigPart,
    AcroForm,
    /// Form-wide settings: default resources.
    AcroFormPart,
}

/// A form field node.
#[derive(Clone, Debug)]
struct FieldInfo {
    name: String,
    parent: Option<ObjectId>,
    sig: bool,
    /// For a signature field: it holds a signature.
    signed: bool,
    /// The signature it holds is a document timestamp.
    timestamp: bool,
    /// It has a widget with an area that isn't hidden.
    visible: bool,
}

/// The most nodes walked in the page tree or the form, and the most steps assigning
/// roles; beyond that the document is refused as too complex to check.
const MAX_NODES: usize = 500_000;
const MAX_STEPS: usize = 4_000_000;

/// Keys that link to an annotation or field elsewhere without containing it.
const LINK_KEYS: [&[u8]; 12] = [
    b"P", b"Parent", b"IRT", b"Popup", b"Obj", b"Pg", b"T", b"AN", b"Dest", b"D", b"Fields", b"CO",
];

struct Roles {
    of: HashMap<ObjectId, Vec<(Kind, ObjectId)>>,
    catalog: ObjectId,
    /// Leaf pages in order, every page tree node, and each leaf's index.
    pages: Vec<ObjectId>,
    page_nodes: HashSet<ObjectId>,
    page_index: HashMap<ObjectId, usize>,
    /// Each leaf page's `/Annots` entries.
    page_annots: HashMap<ObjectId, Vec<Object>>,
    /// Indirect annotations and the page listing them.
    annots: HashMap<ObjectId, usize>,
    fields: BTreeMap<ObjectId, FieldInfo>,
}

impl Roles {
    fn build(v: &Version) -> Result<Self, String> {
        let too_complex = || "the document is too complex to check".to_string();
        let catalog = v
            .pdf
            .doc
            .trailer
            .get(b"Root")
            .ok()
            .and_then(|o| o.as_reference().ok())
            .filter(|&id| v.dict_of(id).is_some())
            .ok_or("a version has no document catalog")?;
        let cat = v
            .dict_of(catalog)
            .ok_or("a version has no document catalog")?;
        let mut r = Roles {
            of: HashMap::new(),
            catalog,
            pages: Vec::new(),
            page_nodes: HashSet::new(),
            page_index: HashMap::new(),
            page_annots: HashMap::new(),
            annots: HashMap::new(),
            fields: BTreeMap::new(),
        };

        // The page tree.
        let mut stack: Vec<(ObjectId, usize)> = cat
            .get(b"Pages")
            .ok()
            .and_then(|o| o.as_reference().ok())
            .map(|id| vec![(id, 0)])
            .unwrap_or_default();
        while let Some((id, depth)) = stack.pop() {
            if depth > 64 || !r.page_nodes.insert(id) {
                continue;
            }
            if r.page_nodes.len() > MAX_NODES {
                return Err(too_complex());
            }
            let Some(d) = v.dict_of(id) else {
                continue;
            };
            let kids = d.get(b"Kids").ok().and_then(|k| v.array(k));
            match kids {
                Some(kids) if v.name(d, b"Type") != Some(b"Page") => {
                    for k in kids.iter().rev() {
                        if let Ok(k) = k.as_reference() {
                            stack.push((k, depth + 1));
                        }
                    }
                }
                _ => {
                    r.page_index.entry(id).or_insert(r.pages.len());
                    r.pages.push(id);
                }
            }
        }
        for (i, &page) in r.pages.iter().enumerate() {
            let entries = v
                .dict_of(page)
                .and_then(|d| d.get(b"Annots").ok())
                .and_then(|a| v.array(a))
                .cloned()
                .unwrap_or_default();
            for e in &entries {
                if let Ok(id) = e.as_reference() {
                    r.annots.entry(id).or_insert(i);
                }
            }
            r.page_annots.insert(page, entries);
        }

        // The form's fields.
        let form = cat.get(b"AcroForm").ok().and_then(|o| v.dict(o));
        let roots = form
            .and_then(|f| f.get(b"Fields").ok())
            .and_then(|o| v.array(o))
            .cloned()
            .unwrap_or_default();
        // (field, parent, depth); a parent is recorded before its kids.
        let mut stack: Vec<(ObjectId, Option<ObjectId>, usize)> = roots
            .iter()
            .rev()
            .filter_map(|o| o.as_reference().ok())
            .map(|id| (id, None, 0))
            .collect();
        while let Some((id, parent, depth)) = stack.pop() {
            if depth > 32 || r.fields.contains_key(&id) {
                continue;
            }
            if r.fields.len() > MAX_NODES {
                return Err(too_complex());
            }
            let Some(d) = v.dict_of(id) else {
                continue;
            };
            let up = parent.and_then(|p| r.fields.get(&p));
            let partial = d.get(b"T").ok().and_then(|t| v.pdf.text_value(t));
            let name = match (partial, up) {
                (Some(t), Some(p)) if !p.name.is_empty() => format!("{}.{t}", p.name),
                (Some(t), _) => t,
                (None, p) => p.map(|p| p.name.clone()).unwrap_or_default(),
            };
            let sig = match v.name(d, b"FT") {
                Some(ft) => ft == b"Sig",
                None => up.is_some_and(|p| p.sig),
            };
            let value = d.get(b"V").ok().and_then(|o| v.dict(o));
            let timestamp = value.is_some_and(|s| {
                v.name(s, b"Type") == Some(b"DocTimeStamp")
                    || v.name(s, b"SubFilter") == Some(b"ETSI.RFC3161")
            });
            r.fields.insert(
                id,
                FieldInfo {
                    name,
                    parent,
                    sig,
                    signed: sig && value.is_some(),
                    timestamp: sig && timestamp,
                    visible: widget_visible(v, d),
                },
            );
            if let Some(kids) = d.get(b"Kids").ok().and_then(|k| v.array(k)) {
                for k in kids.iter().rev() {
                    if let Ok(k) = k.as_reference() {
                        stack.push((k, Some(id), depth + 1));
                    }
                }
            }
        }

        // Roles.
        let mut w = Walker {
            v,
            r: &mut r,
            seen: HashSet::new(),
            todo: Vec::new(),
            steps: 0,
        };
        w.push(catalog, Kind::Catalog, catalog);
        for (key, kind) in [(&b"Info"[..], Kind::Info), (b"Encrypt", Kind::Encrypt)] {
            if let Ok(id) = v.pdf.doc.trailer.get(key).and_then(|o| o.as_reference()) {
                w.push(id, kind, id);
            }
        }
        if let Some(Ok(id)) = cat.get(b"AcroForm").ok().map(|o| o.as_reference()) {
            w.push(id, Kind::AcroForm, id);
        }
        let nodes: Vec<ObjectId> = w.r.page_nodes.iter().copied().collect();
        for id in nodes {
            w.push(id, Kind::PageNode, id);
        }
        let fields: Vec<(ObjectId, bool)> = w.r.fields.iter().map(|(id, f)| (*id, f.sig)).collect();
        for (id, sig) in fields {
            w.push(id, if sig { Kind::SigField } else { Kind::Field }, id);
        }
        let annots: Vec<ObjectId> = w.r.annots.keys().copied().collect();
        for id in annots {
            let kind = if w.r.fields.contains_key(&id) {
                Kind::WidgetAnnot
            } else {
                match annot_class(v, v.get(id)) {
                    AnnotClass::Comment => Kind::Comment,
                    _ => Kind::OtherAnnot,
                }
            };
            w.push(id, kind, id);
        }
        w.run()?;
        Ok(r)
    }

    fn kinds(&self, id: ObjectId) -> &[(Kind, ObjectId)] {
        self.of.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    fn page_of(&self, node: ObjectId) -> Option<usize> {
        self.page_index.get(&node).copied()
    }
}

/// Whether a field's widget (the field itself, or any of its kids) shows on the page: a
/// rectangle with an area, not hidden.
fn widget_visible(v: &Version, d: &Dictionary) -> bool {
    let one = |d: &Dictionary| {
        let rect: Option<Vec<f32>> = d.get(b"Rect").ok().and_then(|r| v.array(r)).and_then(|a| {
            a.iter()
                .map(|x| match v.resolve(x)? {
                    Object::Integer(i) => Some(*i as f32),
                    Object::Real(f) => Some(*f),
                    _ => None,
                })
                .collect()
        });
        let area = matches!(rect.as_deref(), Some(&[x0, y0, x1, y1])
            if (x1 - x0).abs() > 0.0 && (y1 - y0).abs() > 0.0);
        let flags = d
            .get(b"F")
            .ok()
            .and_then(|f| v.resolve(f)?.as_i64().ok())
            .unwrap_or(0);
        // Hidden (2) or NoView (32).
        area && flags & (2 | 32) == 0
    };
    one(d)
        || d.get(b"Kids")
            .ok()
            .and_then(|k| v.array(k))
            .is_some_and(|kids| kids.iter().filter_map(|k| v.dict(k)).any(one))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AnnotClass {
    /// A markup annotation or a popup.
    Comment,
    Widget,
    /// A link or another annotation that isn't a comment, or not an annotation at all.
    Other,
}

const COMMENT_SUBTYPES: [&[u8]; 18] = [
    b"Text",
    b"FreeText",
    b"Line",
    b"Square",
    b"Circle",
    b"Polygon",
    b"PolyLine",
    b"Highlight",
    b"Underline",
    b"Squiggly",
    b"StrikeOut",
    b"Stamp",
    b"Caret",
    b"Ink",
    b"FileAttachment",
    b"Sound",
    b"Redact",
    b"Popup",
];

fn annot_class(v: &Version, o: Option<&Object>) -> AnnotClass {
    let Some(Object::Dictionary(d)) = o.and_then(|o| v.resolve(o)) else {
        return AnnotClass::Other;
    };
    match v.name(d, b"Subtype") {
        Some(b"Widget") => AnnotClass::Widget,
        Some(s) if COMMENT_SUBTYPES.contains(&s) => AnnotClass::Comment,
        _ => AnnotClass::Other,
    }
}

/// Assigns roles: each object reached gets the kind of the structure reaching it.
struct Walker<'v, 'r> {
    v: &'v Version<'v>,
    r: &'r mut Roles,
    seen: HashSet<(ObjectId, Kind, ObjectId)>,
    todo: Vec<(ObjectId, Kind, ObjectId)>,
    steps: usize,
}

impl Walker<'_, '_> {
    fn push(&mut self, id: ObjectId, kind: Kind, owner: ObjectId) {
        if self.seen.insert((id, kind, owner)) {
            self.r.of.entry(id).or_default().push((kind, owner));
            self.todo.push((id, kind, owner));
        }
    }

    fn run(&mut self) -> Result<(), String> {
        while let Some((id, kind, owner)) = self.todo.pop() {
            self.steps += 1;
            if self.steps > MAX_STEPS {
                return Err("the document is too complex to check".into());
            }
            let v = self.v;
            let Some(obj) = v.get(id) else {
                continue;
            };
            let dict = match obj {
                Object::Dictionary(d) => Some(d),
                Object::Stream(s) => Some(&s.dict),
                _ => None,
            };
            match (kind, dict) {
                (Kind::Catalog, Some(d)) => {
                    for (k, val) in d.iter() {
                        let nk = match k.as_slice() {
                            b"Pages" => continue,
                            b"Names" => {
                                match val {
                                    Object::Reference(r) => self.push(*r, Kind::Names, *r),
                                    Object::Dictionary(n) => self.names(n, owner),
                                    _ => {}
                                }
                                continue;
                            }
                            b"AcroForm" => Kind::AcroForm,
                            b"DSS" => Kind::Dss,
                            b"Metadata" => Kind::Metadata,
                            b"Extensions" => Kind::Extensions,
                            b"Perms" => Kind::Perms,
                            _ => Kind::Other,
                        };
                        self.walk_entry(val, nk, owner, k);
                    }
                }
                (Kind::PageNode, Some(d)) => {
                    for (k, val) in d.iter() {
                        match k.as_slice() {
                            b"Parent" => {}
                            b"Kids" => {
                                if let Ok(r) = val.as_reference() {
                                    self.push(r, Kind::PagesArray, id);
                                }
                            }
                            b"Annots" => {
                                if let Ok(r) = val.as_reference() {
                                    self.push(r, Kind::AnnotsArray, id);
                                }
                                // Annotations written inside the array; the others are
                                // pushed as annotations already.
                                for e in v.array(val).into_iter().flatten() {
                                    if let Object::Dictionary(_) = e {
                                        let nk = match annot_class(v, Some(e)) {
                                            AnnotClass::Comment => Kind::CommentPart,
                                            _ => Kind::OtherAnnotPart,
                                        };
                                        self.walk(e, nk, id, Some(k.as_slice()));
                                    }
                                }
                            }
                            _ => self.walk_entry(val, Kind::PageContent, id, k),
                        }
                    }
                }
                (Kind::Comment | Kind::OtherAnnot, Some(d)) => {
                    let part = if kind == Kind::Comment {
                        Kind::CommentPart
                    } else {
                        Kind::OtherAnnotPart
                    };
                    for (k, val) in d.iter() {
                        match k.as_slice() {
                            b"P" | b"Parent" | b"IRT" => {}
                            b"Popup" => {
                                if let Ok(r) = val.as_reference() {
                                    if !self.r.fields.contains_key(&r) {
                                        self.push(r, kind, r);
                                    }
                                }
                            }
                            _ => self.walk_entry(val, part, id, k),
                        }
                    }
                }
                (Kind::Field | Kind::SigField, Some(d)) => {
                    let part = if kind == Kind::Field {
                        Kind::FieldPart
                    } else {
                        Kind::SigFieldPart
                    };
                    for (k, val) in d.iter() {
                        match k.as_slice() {
                            b"Parent" | b"P" => {}
                            b"Kids" => {
                                if let Ok(r) = val.as_reference() {
                                    self.push(r, Kind::FieldsArray, id);
                                }
                            }
                            b"V" if kind == Kind::SigField => match val {
                                Object::Reference(r) => self.push(*r, Kind::SigDict, id),
                                _ => self.walk(val, Kind::SigPart, id, Some(k.as_slice())),
                            },
                            b"Lock" | b"SV" => self.walk_entry(val, Kind::FieldMeta, id, k),
                            _ => self.walk_entry(val, part, id, k),
                        }
                    }
                }
                (Kind::AcroForm, Some(d)) => {
                    for (k, val) in d.iter() {
                        match k.as_slice() {
                            b"Fields" => {
                                if let Ok(r) = val.as_reference() {
                                    self.push(r, Kind::FieldsArray, id);
                                }
                            }
                            b"CO" => {}
                            b"XFA" => self.walk_entry(val, Kind::Other, id, k),
                            _ => self.walk_entry(val, Kind::AcroFormPart, id, k),
                        }
                    }
                }
                (Kind::Names, Some(d)) => self.names(d, id),
                (
                    Kind::WidgetAnnot | Kind::PagesArray | Kind::AnnotsArray | Kind::FieldsArray,
                    _,
                ) => {}
                _ => self.walk(obj, kind, owner, None),
            }
        }
        Ok(())
    }

    /// The name trees: comment icon appearances, and the rest (destinations, scripts,
    /// embedded files) as document structure.
    fn names(&mut self, d: &Dictionary, owner: ObjectId) {
        for (k, val) in d.iter() {
            let kind = if k == b"AP" {
                Kind::NamedAppearance
            } else {
                Kind::Other
            };
            self.walk_entry(val, kind, owner, k);
        }
    }

    /// One dictionary entry: actions and scripts get their own kind.
    fn walk_entry(&mut self, val: &Object, kind: Kind, owner: ObjectId, key: &[u8]) {
        self.walk(val, child_kind(kind, key), owner, Some(key));
    }

    /// The references in `val` (a value of `kind`, under dictionary key `key`).
    fn walk(&mut self, val: &Object, kind: Kind, owner: ObjectId, key: Option<&[u8]>) {
        match val {
            Object::Reference(r) => {
                let r = *r;
                if self.r.page_nodes.contains(&r) {
                    return;
                }
                // A signature's reference to what it covers (the catalog, the form).
                if matches!(kind, Kind::SigDict | Kind::SigPart | Kind::Perms)
                    && key == Some(b"Data")
                {
                    return;
                }
                if key.is_some_and(|k| LINK_KEYS.contains(&k))
                    && (self.r.annots.contains_key(&r) || self.r.fields.contains_key(&r))
                {
                    return;
                }
                let kind = if kind == Kind::SigDict {
                    Kind::SigPart
                } else {
                    kind
                };
                self.push(r, kind, owner);
            }
            Object::Array(a) => {
                for e in a {
                    self.walk(e, kind, owner, key);
                }
            }
            Object::Dictionary(d) => {
                for (k, v) in d.iter() {
                    self.walk_entry(v, kind, owner, k);
                }
            }
            Object::Stream(s) => {
                for (k, v) in s.dict.iter() {
                    self.walk_entry(v, kind, owner, k);
                }
            }
            _ => {}
        }
    }
}

fn child_kind(kind: Kind, key: &[u8]) -> Kind {
    match key {
        b"A" | b"AA" | b"JS" | b"OpenAction" => Kind::Action,
        _ if kind == Kind::SigDict => Kind::SigPart,
        _ => kind,
    }
}

// ---------------------------------------------------------------- judging

struct Judge<'a> {
    old: &'a Version<'a>,
    new: &'a Version<'a>,
    or: &'a Roles,
    nr: &'a Roles,
    out: Vec<Seen>,
}

/// Field dictionary entries filling in may change: the value, its appearance, the
/// appearance state, the rich-text value, the selected options, the modification date.
const FILL_KEYS: [&[u8]; 6] = [b"V", b"AP", b"AS", b"RV", b"I", b"M"];
/// Signature field entries signing may change.
const SIGN_KEYS: [&[u8]; 4] = [b"V", b"AP", b"AS", b"M"];

impl Judge<'_> {
    fn push(&mut self, s: Seen) {
        self.out.push(s);
    }

    /// The keys whose values differ between two dictionaries.
    fn differing(a: &Dictionary, b: &Dictionary) -> Vec<Vec<u8>> {
        let mut keys: Vec<Vec<u8>> = a
            .iter()
            .map(|(k, _)| k.clone())
            .chain(b.iter().map(|(k, _)| k.clone()))
            .collect();
        keys.sort();
        keys.dedup();
        keys.into_iter()
            .filter(|k| !same_value(a.get(k).ok(), b.get(k).ok()))
            .collect()
    }

    fn trailer(&mut self) {
        let (o, n) = (&self.old.pdf.doc.trailer, &self.new.pdf.doc.trailer);
        if self.or.catalog != self.nr.catalog {
            self.push(never(
                ChangeKind::DocumentChanged("the document catalog was replaced".into()),
                None,
            ));
        }
        if !same_value(o.get(b"Encrypt").ok(), n.get(b"Encrypt").ok()) {
            self.push(never(
                ChangeKind::DocumentChanged("encryption".into()),
                None,
            ));
        }
        if !same_value(o.get(b"Info").ok(), n.get(b"Info").ok()) {
            self.push(seen(ChangeKind::Metadata, None, Need::Nothing));
        }
    }

    fn pages(&mut self) {
        if self.or.pages != self.nr.pages {
            self.push(never(ChangeKind::PagesChanged, None));
        }
    }

    /// Annotations added to, removed from or reordered on each page.
    fn annotations(&mut self) {
        let (old, new) = (self.old, self.new);
        for (i, page) in self.nr.pages.iter().enumerate() {
            let Some(before) = self.or.page_annots.get(page) else {
                continue;
            };
            let after = &self.nr.page_annots[page];
            let page = Some(i);
            let has = |list: &[Object], e: &Object| list.iter().any(|x| same_object(x, e));
            for e in before.iter().filter(|e| !has(after, e)) {
                match annot_class(old, Some(e)) {
                    AnnotClass::Comment => {
                        self.push(seen(ChangeKind::CommentRemoved, page, Need::Comment))
                    }
                    AnnotClass::Widget => {
                        let id = e.as_reference().ok();
                        let field = id.and_then(|id| self.or.fields.get(&id));
                        let kept = id.is_some_and(|id| self.nr.fields.contains_key(&id));
                        match field {
                            // Taken off the page but still a field.
                            Some(f) if kept && f.signed => {
                                self.push(never(ChangeKind::SignatureChanged(f.name.clone()), page))
                            }
                            Some(f) if kept => {
                                self.push(never(ChangeKind::FieldChanged(f.name.clone()), page))
                            }
                            // A field removed is reported with the fields.
                            Some(_) => {}
                            None => self.push(never(ChangeKind::AnnotationChanged, page)),
                        }
                    }
                    AnnotClass::Other => self.push(never(ChangeKind::AnnotationChanged, page)),
                }
            }
            for e in after.iter().filter(|e| !has(before, e)) {
                match annot_class(new, Some(e)) {
                    AnnotClass::Comment => {
                        let d = new.dict(e);
                        if d.is_some_and(|d| d.has(b"A") || d.has(b"AA")) {
                            self.push(never(ChangeKind::Actions, page));
                        } else {
                            self.push(seen(ChangeKind::CommentAdded, page, Need::Comment));
                        }
                    }
                    AnnotClass::Widget => {
                        let id = e.as_reference().ok();
                        match id.and_then(|id| self.nr.fields.get(&id).map(|f| (id, f))) {
                            // A widget of a field added in this step: reported with the
                            // fields. A widget added to a field from before isn't allowed.
                            Some((id, f)) => {
                                let root = self.field_root(id);
                                if self.or.fields.contains_key(&root) {
                                    self.push(never(
                                        ChangeKind::FieldChanged(f.name.clone()),
                                        page,
                                    ));
                                }
                            }
                            None => self.push(never(ChangeKind::AnnotationChanged, page)),
                        }
                    }
                    AnnotClass::Other => self.push(never(ChangeKind::AnnotationChanged, page)),
                }
            }
            // Entries kept must keep their order (it decides what covers what).
            let kept_before: Vec<&Object> = before.iter().filter(|e| has(after, e)).collect();
            let kept_after: Vec<&Object> = after.iter().filter(|e| has(before, e)).collect();
            let same_order = kept_before.len() == kept_after.len()
                && kept_before
                    .iter()
                    .zip(&kept_after)
                    .all(|(x, y)| same_object(x, y));
            if !same_order {
                if kept_after
                    .iter()
                    .all(|e| annot_class(new, Some(e)) == AnnotClass::Comment)
                {
                    self.push(seen(ChangeKind::CommentChanged, page, Need::Comment));
                } else {
                    self.push(never(ChangeKind::AnnotationChanged, page));
                }
            }
        }
    }

    /// The top of the field tree `id` is in, in the newer version.
    fn field_root(&self, mut id: ObjectId) -> ObjectId {
        for _ in 0..64 {
            match self.nr.fields.get(&id).and_then(|f| f.parent) {
                Some(p) => id = p,
                None => break,
            }
        }
        id
    }

    /// Fields added, removed or signed.
    fn fields(&mut self) {
        for (id, f) in &self.or.fields {
            if !self.nr.fields.contains_key(id) {
                let kind = if f.signed {
                    ChangeKind::SignatureChanged(f.name.clone())
                } else {
                    ChangeKind::FieldRemoved(f.name.clone())
                };
                self.push(never(kind, None));
            }
        }
        for (id, f) in &self.nr.fields {
            match self.or.fields.get(id) {
                Some(o) => {
                    if o.signed && (!f.signed || o.name != f.name) {
                        self.push(never(ChangeKind::SignatureChanged(o.name.clone()), None));
                    } else if o.sig && f.sig && !o.signed && f.signed {
                        self.push(sign_seen(f, None));
                    }
                }
                None => match f.parent {
                    // Part of a field added in this step.
                    Some(p) if !self.or.fields.contains_key(&p) => {}
                    Some(p) => {
                        let name = self.nr.fields[&p].name.clone();
                        self.push(never(ChangeKind::FieldChanged(name), None));
                    }
                    None if f.sig => {
                        // The whole new field: whether anything in it is signed or shows.
                        let tree: Vec<&FieldInfo> = self
                            .nr
                            .fields
                            .iter()
                            .filter(|(i, _)| self.field_root(**i) == *id)
                            .map(|(_, f)| f)
                            .collect();
                        let visible = tree.iter().any(|f| f.visible);
                        match tree.iter().find(|f| f.signed) {
                            Some(s) => self.push(sign_seen(s, Some(visible))),
                            None => self.push(seen(
                                ChangeKind::SignatureFieldAdded(f.name.clone()),
                                None,
                                Need::Sign {
                                    field: f.name.clone(),
                                    new_visible: Some(visible),
                                    timestamp: false,
                                },
                            )),
                        }
                    }
                    None => self.push(never(ChangeKind::FieldAdded(f.name.clone()), None)),
                },
            }
        }
    }

    /// One object that differs between the versions, judged by every role it plays in the
    /// newer one.
    fn object(&mut self, id: ObjectId) {
        let (o, n) = (self.old.get(id), self.new.get(id));
        if same_value(o, n) {
            return;
        }
        let kinds = self.nr.kinds(id).to_vec();
        let what = || format!("object {}", id.0);
        let (o, n) = match (o, n) {
            (None, Some(_)) => {
                // New: judged where it is used, unless the earlier version already used it.
                if !self.or.kinds(id).is_empty() {
                    self.push(never(
                        ChangeKind::Unrecognised(format!(
                            "{}, missing from the signed version but used by it, was added",
                            what()
                        )),
                        None,
                    ));
                }
                return;
            }
            (Some(_), None) => {
                // Deleted: fine when nothing uses it any more.
                if !kinds.is_empty() {
                    self.push(never(
                        ChangeKind::Unrecognised(format!(
                            "{} was deleted but is still used",
                            what()
                        )),
                        None,
                    ));
                }
                return;
            }
            (Some(o), Some(n)) => (o, n),
            (None, None) => return,
        };
        for (kind, owner) in kinds {
            self.judge(kind, owner, id, o, n);
        }
    }

    fn field_name(&self, owner: ObjectId) -> String {
        self.nr
            .fields
            .get(&owner)
            .or_else(|| self.or.fields.get(&owner))
            .map(|f| f.name.clone())
            .unwrap_or_default()
    }

    fn judge(&mut self, kind: Kind, owner: ObjectId, id: ObjectId, o: &Object, n: &Object) {
        let page = (self.nr.annots.get(&owner).copied()).or(self.nr.page_of(owner));
        let dicts = match (o, n) {
            (Object::Dictionary(a), Object::Dictionary(b)) => Some((a, b)),
            _ => None,
        };
        match kind {
            Kind::Catalog => match dicts {
                Some((a, b)) => self.catalog(a, b),
                None => self.push(never(ChangeKind::Unrecognised("the catalog".into()), None)),
            },
            Kind::AcroForm => match dicts {
                Some((a, b)) => self.acroform(a, b),
                None => self.push(never(ChangeKind::Unrecognised("the form".into()), None)),
            },
            Kind::Info | Kind::Metadata => {
                self.push(seen(ChangeKind::Metadata, None, Need::Nothing))
            }
            Kind::Dss | Kind::Extensions => {
                self.push(seen(ChangeKind::ValidationData, None, Need::Nothing))
            }
            Kind::AcroFormPart => self.push(seen(ChangeKind::FormSettings, None, Need::Fill(None))),
            Kind::PageNode => {
                let keys = match dicts {
                    Some((a, b)) => Self::differing(a, b),
                    None => vec![b"Type".to_vec()],
                };
                for k in keys {
                    match k.as_slice() {
                        // Compared annotation by annotation.
                        b"Annots" => {}
                        // iText marks the pages it saves with its version; no reader uses it.
                        b"ITXT"
                            if dicts.is_some_and(|(_, b)| {
                                matches!(b.get(b"ITXT"), Ok(Object::String(..)) | Err(_))
                            }) =>
                        {
                            self.push(seen(ChangeKind::Metadata, None, Need::Nothing))
                        }
                        b"Kids" | b"Count" | b"Parent" => {
                            self.push(never(ChangeKind::PagesChanged, None))
                        }
                        _ => self.push(never(ChangeKind::PageContent, self.nr.page_of(id))),
                    }
                }
            }
            Kind::PagesArray => self.push(never(ChangeKind::PagesChanged, None)),
            // Compared entry by entry.
            Kind::AnnotsArray | Kind::FieldsArray | Kind::WidgetAnnot => {}
            Kind::PageContent => self.push(never(ChangeKind::PageContent, page)),
            Kind::Comment => {
                let comments = annot_class(self.old, Some(o)) == AnnotClass::Comment
                    && annot_class(self.new, Some(n)) == AnnotClass::Comment;
                let actions = dicts.is_some_and(|(a, b)| {
                    !same_value(a.get(b"A").ok(), b.get(b"A").ok())
                        || !same_value(a.get(b"AA").ok(), b.get(b"AA").ok())
                });
                if !comments {
                    self.push(never(ChangeKind::AnnotationChanged, page));
                } else if actions {
                    self.push(never(ChangeKind::Actions, page));
                } else {
                    self.push(seen(ChangeKind::CommentChanged, page, Need::Comment));
                }
            }
            Kind::CommentPart => self.push(seen(ChangeKind::CommentChanged, page, Need::Comment)),
            Kind::OtherAnnot | Kind::OtherAnnotPart => {
                self.push(never(ChangeKind::AnnotationChanged, page))
            }
            Kind::Field => {
                let name = self.field_name(owner);
                let keys = match dicts {
                    Some((a, b)) => Self::differing(a, b),
                    None => vec![b"Type".to_vec()],
                };
                let mut filled = false;
                for k in keys {
                    if FILL_KEYS.contains(&k.as_slice()) {
                        filled = true;
                    } else if k != b"Kids" {
                        self.push(never(ChangeKind::FieldChanged(name.clone()), page));
                    }
                }
                if filled {
                    self.push(fill_seen(&name, page));
                }
            }
            Kind::FieldPart => {
                let name = self.field_name(owner);
                self.push(fill_seen(&name, page));
            }
            Kind::SigField => {
                let name = self.field_name(owner);
                let was_signed = self.or.fields.get(&owner).is_some_and(|f| f.signed);
                let keys = match dicts {
                    Some((a, b)) => Self::differing(a, b),
                    None => vec![b"Type".to_vec()],
                };
                let is_signed = self.nr.fields.get(&owner).is_some_and(|f| f.signed);
                for k in keys {
                    if was_signed {
                        self.push(never(ChangeKind::SignatureChanged(name.clone()), page));
                    } else if k == b"F"
                        && is_signed
                        && dicts.is_some_and(|(a, b)| only_locked(a, b))
                    {
                        // Acrobat locks the widget it signs.
                    } else if !SIGN_KEYS.contains(&k.as_slice()) && k != b"Kids" {
                        self.push(never(ChangeKind::FieldChanged(name.clone()), page));
                    } else if !is_signed {
                        // Changed, but not signed.
                        self.push(never(ChangeKind::FieldChanged(name.clone()), page));
                    }
                }
            }
            Kind::SigFieldPart => {
                let name = self.field_name(owner);
                let was_signed = self.or.fields.get(&owner).is_some_and(|f| f.signed);
                let is_signed = self.nr.fields.get(&owner).is_some_and(|f| f.signed);
                if was_signed {
                    self.push(never(ChangeKind::SignatureChanged(name), page));
                } else if !is_signed {
                    self.push(never(ChangeKind::FieldChanged(name), page));
                }
                // Otherwise part of signing the field, reported with the fields.
            }
            Kind::SigDict | Kind::SigPart => {
                let name = self.field_name(owner);
                self.push(never(ChangeKind::SignatureChanged(name), page));
            }
            Kind::FieldMeta => {
                let name = self.field_name(owner);
                self.push(never(ChangeKind::FieldChanged(name), page));
            }
            Kind::Action => self.push(never(ChangeKind::Actions, page)),
            Kind::Perms => match dicts {
                Some((a, b)) => self.perms(a, b),
                None => self.push(never(
                    ChangeKind::DocumentChanged("the document's permissions".into()),
                    None,
                )),
            },
            Kind::Names => match dicts {
                Some((a, b)) => self.names(a, b),
                None => self.push(never(ChangeKind::DocumentChanged("/Names".into()), None)),
            },
            Kind::NamedAppearance => {
                self.push(seen(ChangeKind::CommentAppearances, None, Need::Comment))
            }
            Kind::Encrypt => self.push(never(
                ChangeKind::DocumentChanged("encryption".into()),
                None,
            )),
            Kind::Other => self.push(never(
                ChangeKind::DocumentChanged(format!("object {}", id.0)),
                None,
            )),
        }
    }

    fn catalog(&mut self, a: &Dictionary, b: &Dictionary) {
        for k in Self::differing(a, b) {
            match k.as_slice() {
                b"AcroForm" => {
                    let empty = Dictionary::new();
                    let fa = a.get(b"AcroForm").ok().and_then(|o| self.old.dict(o));
                    let fb = b.get(b"AcroForm").ok().and_then(|o| self.new.dict(o));
                    match (fa, fb) {
                        (_, None) => self.push(never(
                            ChangeKind::DocumentChanged("the form was removed".into()),
                            None,
                        )),
                        (fa, Some(fb)) => self.acroform(fa.unwrap_or(&empty), fb),
                    }
                }
                b"DSS" | b"Extensions" => {
                    self.push(seen(ChangeKind::ValidationData, None, Need::Nothing))
                }
                b"Metadata" | b"Version" => {
                    self.push(seen(ChangeKind::Metadata, None, Need::Nothing))
                }
                b"Pages" => self.push(never(ChangeKind::PagesChanged, None)),
                b"OpenAction" | b"AA" => self.push(never(ChangeKind::Actions, None)),
                b"Names" | b"Perms" => {
                    let empty = Dictionary::new();
                    let da = a.get(&k).ok().and_then(|o| self.old.dict(o));
                    let db = b.get(&k).ok().and_then(|o| self.new.dict(o));
                    let (da, db) = (da.unwrap_or(&empty), db.unwrap_or(&empty));
                    if k == b"Names" {
                        self.names(da, db);
                    } else {
                        self.perms(da, db);
                    }
                }
                other => self.push(never(
                    ChangeKind::DocumentChanged(format!("/{}", String::from_utf8_lossy(other))),
                    None,
                )),
            }
        }
    }

    /// The name trees: only the comment icon appearances may change.
    fn names(&mut self, a: &Dictionary, b: &Dictionary) {
        for k in Self::differing(a, b) {
            if k == b"AP" {
                self.push(seen(ChangeKind::CommentAppearances, None, Need::Comment));
            } else {
                self.push(never(
                    ChangeKind::DocumentChanged(format!("/Names /{}", String::from_utf8_lossy(&k))),
                    None,
                ));
            }
        }
    }

    /// The document's permissions: Reader usage rights may be removed; nothing else may
    /// change.
    fn perms(&mut self, a: &Dictionary, b: &Dictionary) {
        for k in Self::differing(a, b) {
            if matches!(k.as_slice(), b"UR3" | b"UR") && !b.has(&k) {
                self.push(seen(ChangeKind::UsageRightsRemoved, None, Need::Nothing));
            } else {
                self.push(never(
                    ChangeKind::DocumentChanged("the document's permissions".into()),
                    None,
                ));
            }
        }
    }

    /// The form dictionary: fields are compared one by one; signature flags change with
    /// signing; fonts and defaults for filling in may change with form filling.
    fn acroform(&mut self, a: &Dictionary, b: &Dictionary) {
        for k in Self::differing(a, b) {
            match k.as_slice() {
                b"Fields" | b"SigFlags" => {}
                b"DR" | b"DA" | b"Q" => {
                    self.push(seen(ChangeKind::FormSettings, None, Need::Fill(None)))
                }
                // Turned off, it asks nothing; turned on, every field would be redrawn
                // from its value, which can change what the page shows.
                b"NeedAppearances"
                    if !b
                        .get(b"NeedAppearances")
                        .ok()
                        .and_then(|o| self.new.resolve(o))
                        .is_some_and(|o| o.as_bool().unwrap_or(true)) =>
                {
                    self.push(seen(ChangeKind::FormSettings, None, Need::Fill(None)))
                }
                b"CO" | b"NeedAppearances" => self.push(never(
                    ChangeKind::DocumentChanged(format!("form /{}", String::from_utf8_lossy(&k))),
                    None,
                )),
                other => self.push(never(
                    ChangeKind::DocumentChanged(format!(
                        "form /{}",
                        String::from_utf8_lossy(other)
                    )),
                    None,
                )),
            }
        }
    }
}

/// Whether the annotation flags only gained Locked (128) or LockedContents (512).
fn only_locked(a: &Dictionary, b: &Dictionary) -> bool {
    let flags = |d: &Dictionary| match d.get(b"F") {
        Ok(Object::Integer(f)) => Some(*f),
        Err(_) => Some(0),
        Ok(_) => None,
    };
    match (flags(a), flags(b)) {
        (Some(x), Some(y)) => x & !y == 0 && (y & !x) & !(128 | 512) == 0,
        _ => false,
    }
}

fn fill_seen(name: &str, page: Option<usize>) -> Seen {
    seen(
        ChangeKind::FieldFilled(name.to_string()),
        page,
        Need::Fill(Some(name.to_string())),
    )
}

/// A field signed (`new_visible` set when the field was added in the same step).
fn sign_seen(f: &FieldInfo, new_visible: Option<bool>) -> Seen {
    let kind = if f.timestamp {
        ChangeKind::DocumentTimestamp(f.name.clone())
    } else {
        ChangeKind::Signed(f.name.clone())
    };
    seen(
        kind,
        None,
        Need::Sign {
            field: f.name.clone(),
            new_visible,
            timestamp: f.timestamp,
        },
    )
}
