//! Checking the digital signatures in a PDF: finding them, the byte-range rules that stop
//! a signature being moved, wrapped or borrowed, and the CMS checks in
//! [`crate::cms_check`]. See docs/signatures.md.
//!
//! Everything about a signature (its byte range, reason, certification) is read from the
//! version of the document it signed, never from later additions, and the signature
//! value must sit inside that version's signature dictionary.

use crate::cms_check::{self, oid, Cert, CertInfo, DigestAlg, Fail, SignedData};
use crate::forms::FieldKind;
use crate::sig_changes::{self, Change};
use crate::trust::{self, Purpose, Trust, TrustList};
use crate::{Pdf, Result};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use lopdf::xref::XrefEntry;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::HashMap;
use std::fmt;

/// The digital signatures in a document, as checked by [`verify_signatures`].
#[derive(Clone, Debug)]
pub struct SignatureReport {
    /// In signing order: the signature covering the least of the file first.
    pub signatures: Vec<DigitalSignature>,
    /// Signature fields that hold no signature yet.
    pub empty_fields: Vec<EmptySignatureField>,
    /// The size of the checked file in bytes.
    pub file_len: usize,
    /// Certificates in the document's validation data (`/DSS`), which help build chains
    /// to a trusted authority but are never trusted for being there.
    pub validation_certs: Vec<CertInfo>,
}

impl SignatureReport {
    /// Decides, for every valid signature and its embedded timestamp, whether the signer
    /// is someone `list` trusts ([`DigitalSignature::trust`], [`Timestamp::trust`]).
    /// [`verify_signatures`] does this already; call it again when the list changes.
    pub fn apply_trust(&mut self, list: &TrustList) {
        let parse = |certs: &[CertInfo]| -> Vec<Cert> {
            certs
                .iter()
                .filter_map(|c| Cert::parse(&c.der).ok())
                .collect()
        };
        let extra = parse(&self.validation_certs);
        let judge = |signer: &CertInfo, carried: &[CertInfo], at: DateTime<Utc>, p: Purpose| {
            let Ok(signer) = Cert::parse(&signer.der) else {
                return Trust::Unknown("the signer's certificate can't be read".into());
            };
            let mut pool = parse(carried);
            pool.extend(extra.iter().cloned());
            trust::evaluate(list, &signer, &pool, at, p)
        };
        let now = Utc::now();
        for s in &mut self.signatures {
            s.trust = None;
            if s.status != SigStatus::Valid {
                continue;
            }
            if let Some(ts) = &mut s.timestamp {
                let at = ts.time.with_timezone(&Utc);
                ts.trust = Some(judge(
                    &ts.authority,
                    &ts.certificates,
                    at,
                    Purpose::Timestamping,
                ));
            }
            let Some(signer) = &s.signer else {
                continue;
            };
            let purpose = match s.kind {
                SigKind::DocumentTimestamp => Purpose::Timestamping,
                _ => Purpose::Signing,
            };
            // Certificates are checked at the signing time; with none given, now.
            let at = s.time.map_or(now, |t| t.time.with_timezone(&Utc));
            s.trust = Some(judge(signer, &s.certificates, at, purpose));
        }
    }

    /// True when the document has signatures and every one checked as valid with a
    /// signer the trust list vouches for.
    pub fn all_trusted(&self) -> bool {
        !self.signatures.is_empty()
            && self
                .signatures
                .iter()
                .all(|s| s.trust.as_ref().is_some_and(Trust::is_trusted))
    }

    /// The certification signature, if the document is certified, with what it allows.
    pub fn certification(&self) -> Option<(&DigitalSignature, Permissions)> {
        self.signatures.iter().find_map(|s| match s.kind {
            SigKind::Certification(p) => Some((s, p)),
            _ => None,
        })
    }

    /// True when every signature checked as valid (also when there are none).
    pub fn all_valid(&self) -> bool {
        self.signatures.iter().all(|s| s.status == SigStatus::Valid)
    }

    /// True when every signature checked as valid and nothing changed after any of them
    /// that its signer didn't allow (also when there are no signatures).
    pub fn all_ok(&self) -> bool {
        self.all_valid() && self.signatures.iter().all(|s| s.changes_allowed())
    }
}

impl DigitalSignature {
    /// True when every change after signing is one the signer allowed (also when
    /// nothing changed).
    pub fn changes_allowed(&self) -> bool {
        self.changes.iter().all(Change::allowed)
    }
}

/// One signature and what checking it found.
#[derive(Clone, Debug)]
pub struct DigitalSignature {
    /// The fully qualified names of the fields holding it: one, unless the file shares one
    /// signature between fields (see [`SigWarning::SharedByFields`]).
    pub fields: Vec<String>,
    /// The page of its first widget (zero-based), and the widget's box in displayed-page
    /// points (y down). An invisible signature has a zero-sized box.
    pub page: Option<usize>,
    pub bbox: Option<[f32; 4]>,
    pub kind: SigKind,
    /// The signature format (`/SubFilter`), such as `adbe.pkcs7.detached`.
    pub sub_filter: String,
    pub status: SigStatus,
    /// The signed byte ranges `[0, b, c, d]`: bytes `0..b` and `c..c+d`, around the
    /// signature value. `None` when they couldn't be read.
    pub byte_range: Option<[usize; 4]>,
    /// True when the signature covers the whole file; false when something was added
    /// after signing (a later revision), or when the range couldn't be read.
    pub covers_whole_file: bool,
    /// The length of the version this signature signed (the file from its start to the
    /// end of the byte range), when the range follows the rules: View signed version.
    pub signed_len: Option<usize>,
    /// What changed after signing, each with whether the signer allowed it: empty when
    /// nothing did. Disallowed changes come first.
    pub changes: Vec<Change>,
    /// The signer's certificate (the timestamp authority's, for a document timestamp).
    pub signer: Option<CertInfo>,
    /// Every certificate the signature carries, the signer's among them.
    pub certificates: Vec<CertInfo>,
    /// Whether the signer is someone the trust list vouches for; `None` unless the
    /// signature checked as valid.
    pub trust: Option<Trust>,
    /// The signer's name, reason, location and contact details as the signature
    /// dictionary gives them: claims by the signer, not checked by anyone.
    pub name: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub contact: Option<String>,
    /// When it was signed, and where that time comes from.
    pub time: Option<SigningTime>,
    /// A timestamp from a timestamp authority embedded in the signature, if valid.
    pub timestamp: Option<Timestamp>,
    /// The digest algorithm, such as "SHA-256".
    pub digest: Option<&'static str>,
    /// The signature algorithm, such as "RSA PKCS#1 v1.5".
    pub algorithm: Option<&'static str>,
    /// Fields this signature locked when it was signed (`FieldMDP`).
    pub locks: Option<FieldLock>,
    pub warnings: Vec<SigWarning>,
}

/// What kind of signature it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigKind {
    /// An ordinary signature.
    Approval,
    /// The author's certification, which limits later changes.
    Certification(Permissions),
    /// A document timestamp (RFC 3161): a timestamp authority's proof that the document
    /// existed then, with no person's signature.
    DocumentTimestamp,
}

/// What a certified document allows after certification (the DocMDP `/P` value), from
/// least to most.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Permissions {
    /// No changes.
    NoChanges,
    /// Filling in forms and signing.
    FormFilling,
    /// Filling in forms, signing and comments.
    FormFillingAndComments,
}

/// The result of checking one signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SigStatus {
    /// The signed part is unchanged since this key signed it, and the signature follows
    /// the PDF rules. Who owns the key is a separate question (trust).
    Valid,
    /// Not a genuine, intact signature, with the reason.
    Broken(String),
    /// LimarePDF can't check it, with the reason: an unsupported algorithm or unreadable data.
    CantCheck(String),
}

/// A signing time and where it comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SigningTime {
    pub time: DateTime<FixedOffset>,
    pub source: TimeSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeSource {
    /// A timestamp authority's timestamp, checked.
    Timestamp,
    /// The signer's computer clock, from the signature itself.
    SignerClock,
    /// The signer's computer clock, from the signature dictionary (`/M`), which the
    /// signature covers but which is only a claim.
    Dictionary,
}

/// A checked timestamp from a timestamp authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timestamp {
    pub time: DateTime<FixedOffset>,
    pub authority: CertInfo,
    /// Every certificate the timestamp carries, the authority's among them.
    pub certificates: Vec<CertInfo>,
    /// Whether the trust list vouches for the timestamp authority; `None` until judged.
    pub trust: Option<Trust>,
}

/// Fields locked by a signature (or to be locked when an empty field is signed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldLock {
    pub action: LockAction,
    /// The fields named, for [`LockAction::Include`] and [`LockAction::Exclude`].
    pub fields: Vec<String>,
    /// What the whole document allows after signing (`/P`, PDF 2.0; Acrobat's "Lock
    /// document after signing" sets no changes).
    pub permissions: Option<Permissions>,
}

impl FieldLock {
    /// Whether the field with this fully qualified name is locked (a field named in the
    /// lock also covers the fields under it).
    pub fn locks(&self, field: &str) -> bool {
        let named = self.fields.iter().any(|f| {
            field == f
                || field
                    .strip_prefix(f.as_str())
                    .is_some_and(|rest| rest.starts_with('.'))
        });
        match self.action {
            LockAction::All => true,
            LockAction::Include => named,
            LockAction::Exclude => !named,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockAction {
    /// Every field.
    All,
    /// Only the named fields.
    Include,
    /// Every field except the named ones.
    Exclude,
}

/// A signature field that holds no signature yet.
#[derive(Clone, Debug, PartialEq)]
pub struct EmptySignatureField {
    pub name: String,
    pub page: Option<usize>,
    pub bbox: Option<[f32; 4]>,
    /// Fields that signing this one locks (`/Lock`).
    pub lock: Option<FieldLock>,
}

/// Something worth knowing about a signature that doesn't make it invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SigWarning {
    /// The signature relies on a digest that is no longer safe against forgery (SHA-1).
    WeakDigest(&'static str),
    /// An RSA key shorter than 2048 bits.
    ShortKey(usize),
    /// The signer's certificate had expired or wasn't valid yet at the signing time.
    CertificateNotValidAtSigning,
    /// The certificate restricts its key to uses other than signing.
    KeyNotForSigning,
    /// The timestamp authority's certificate isn't marked for timestamping.
    NotATimestampAuthority,
    /// An embedded timestamp couldn't be confirmed, with the reason; the signing time
    /// then comes from the signer's computer.
    TimestampProblem(String),
    /// Bytes other than zeros follow the signature value in its placeholder.
    DataAfterSignature,
    /// The same signature is used by more than one field.
    SharedByFields,
    /// The signature's dictionary (reason, name, time) was replaced after signing; the
    /// signed values are shown.
    DetailsChangedAfterSigning,
    /// A certification that isn't the document's first signature, against the rules.
    CertificationNotFirst,
    /// A certification the document's permissions (`/Perms /DocMDP`) don't point to.
    CertificationNotRegistered,
}

impl fmt::Display for SigWarning {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::WeakDigest(d) => write!(f, "uses {d}, which is no longer safe against forgery"),
            Self::ShortKey(b) => write!(
                f,
                "uses a short {b}-bit RSA key; 2048 bits is the minimum considered safe"
            ),
            Self::CertificateNotValidAtSigning => {
                write!(
                    f,
                    "the signer's certificate wasn't valid at the signing time"
                )
            }
            Self::KeyNotForSigning => write!(f, "the signer's certificate isn't meant for signing"),
            Self::NotATimestampAuthority => {
                write!(
                    f,
                    "the timestamp authority's certificate isn't meant for timestamps"
                )
            }
            Self::TimestampProblem(why) => write!(f, "the embedded timestamp can't be used: {why}"),
            Self::DataAfterSignature => write!(f, "extra data follows the signature value"),
            Self::SharedByFields => write!(f, "the same signature is used by more than one field"),
            Self::DetailsChangedAfterSigning => write!(
                f,
                "the signature's details were replaced after signing; the signed ones are shown"
            ),
            Self::CertificationNotFirst => {
                write!(
                    f,
                    "this certification isn't the first signature, as the rules require"
                )
            }
            Self::CertificationNotRegistered => {
                write!(
                    f,
                    "the document's permissions don't point to this certification"
                )
            }
        }
    }
}

impl fmt::Display for Permissions {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Self::NoChanges => "no changes allowed",
            Self::FormFilling => "form filling and signing allowed",
            Self::FormFillingAndComments => "form filling, signing and comments allowed",
        })
    }
}

/// Checks every digital signature in `bytes` (a whole PDF file), and whether `trust`
/// vouches for each signer. `password` opens an encrypted file. Fails only when the file
/// can't be opened at all; each signature's problems are in its
/// [`DigitalSignature::status`].
pub fn verify_signatures(
    bytes: &[u8],
    password: Option<&str>,
    trust: &TrustList,
) -> Result<SignatureReport> {
    let pdf = Pdf::from_bytes(bytes, password)?;
    let mut groups: Vec<(Place, Vec<(crate::Field, ObjectId)>)> = Vec::new();
    let mut empty_fields = Vec::new();
    for (field, node) in pdf.form_nodes() {
        if field.kind != FieldKind::Signature {
            continue;
        }
        let d = pdf.doc.get_dictionary(node.id).ok();
        let place = match d.and_then(|d| d.get(b"V").ok()) {
            None | Some(Object::Null) => {
                let w = field.widgets.first();
                empty_fields.push(EmptySignatureField {
                    lock: d
                        .and_then(|d| d.get(b"Lock").ok())
                        .and_then(|l| field_lock(&pdf, l)),
                    page: w.map(|w| w.page),
                    bbox: w.map(|w| w.bbox),
                    name: field.name,
                });
                continue;
            }
            Some(Object::Reference(id)) => Place::Indirect(*id),
            Some(Object::Dictionary(_)) => Place::Direct(node.id),
            Some(_) => Place::NotADictionary(node.id),
        };
        match groups.iter_mut().find(|(p, _)| *p == place) {
            Some((_, fields)) => fields.push((field, node.id)),
            None => groups.push((place, vec![(field, node.id)])),
        }
    }
    let mut versions = Versions {
        bytes,
        password,
        whole: &pdf,
        cache: HashMap::new(),
    };
    let mut signatures: Vec<DigitalSignature> = groups
        .into_iter()
        .map(|(place, fields)| check_one(&pdf, &mut versions, place, fields))
        .collect();
    signatures.sort_by_key(|s| match s.byte_range {
        Some([_, _, c, d]) => (false, c + d),
        None => (true, 0),
    });
    certification_rules(&pdf, &mut signatures);

    // What changed after each signature. Any certification limits every signature's
    // later changes; when more than one claims to certify, the strictest counts.
    let certified = signatures
        .iter()
        .filter_map(|s| match s.kind {
            SigKind::Certification(p) => Some(p),
            _ => None,
        })
        .min();
    let which: Vec<usize> = (0..signatures.len())
        .filter(|&i| signatures[i].signed_len.is_some())
        .collect();
    let inputs: Vec<sig_changes::Signed> = which
        .iter()
        .map(|&i| sig_changes::Signed {
            end: signatures[i].signed_len.unwrap_or_default(),
            locks: signatures[i].locks.as_ref(),
        })
        .collect();
    let found = sig_changes::analyse(&mut versions, &inputs, certified);
    for (i, changes) in which.into_iter().zip(found) {
        signatures[i].changes = changes;
    }
    let mut report = SignatureReport {
        signatures,
        empty_fields,
        file_len: bytes.len(),
        validation_certs: validation_certs(&pdf),
    };
    report.apply_trust(trust);
    Ok(report)
}

/// The most certificates read from the document's validation data.
const MAX_VALIDATION_CERTS: usize = 1000;

/// The certificates in the document's validation data (`/DSS /Certs`); unreadable ones
/// are left out.
fn validation_certs(pdf: &Pdf) -> Vec<CertInfo> {
    let resolve = |o: &Object| pdf.doc.dereference(o).ok().map(|(_, o)| o.clone());
    let certs = pdf
        .doc
        .catalog()
        .ok()
        .and_then(|c| resolve(c.get(b"DSS").ok()?))
        .and_then(|d| resolve(d.as_dict().ok()?.get(b"Certs").ok()?));
    let Some(Object::Array(certs)) = certs else {
        return Vec::new();
    };
    certs
        .iter()
        .take(MAX_VALIDATION_CERTS)
        .filter_map(|o| {
            let Object::Stream(s) = resolve(o)? else {
                return None;
            };
            let data = s.get_plain_content_with_limit(1 << 20).ok()?;
            Cert::parse(&data).ok().map(|c| c.info())
        })
        .collect()
}

/// Where a signature dictionary is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// An object of its own.
    Indirect(ObjectId),
    /// Written inside the field's dictionary (the field's id).
    Direct(ObjectId),
    /// The field's `/V` isn't a dictionary (the field's id).
    NotADictionary(ObjectId),
}

/// Versions of the document (the file up to the end of a signature's range, or of a
/// revision), opened once each.
pub(crate) struct Versions<'a> {
    pub(crate) bytes: &'a [u8],
    password: Option<&'a str>,
    /// The whole file, already open.
    whole: &'a Pdf,
    cache: HashMap<usize, Option<Pdf>>,
}

/// The most earlier versions opened while checking one file, and their combined size, so a
/// file crafted with very many signed versions can't use up the computer's memory.
const MAX_VERSIONS: usize = 100;
const MAX_VERSION_BYTES: usize = 1 << 30;

impl Versions<'_> {
    /// Whether the version ending at `end` is open or may be opened within the limits.
    fn allowed(&self, end: usize) -> bool {
        end == self.bytes.len()
            || self.cache.contains_key(&end)
            || (self.cache.len() < MAX_VERSIONS
                && self.cache.keys().sum::<usize>().saturating_add(end) <= MAX_VERSION_BYTES)
    }

    /// Opens the version ending at `end`, if not open yet and within the limits.
    pub(crate) fn load(&mut self, end: usize) {
        let (bytes, password) = (self.bytes, self.password);
        if end < bytes.len() && self.allowed(end) {
            self.cache
                .entry(end)
                .or_insert_with(|| Pdf::from_bytes(&bytes[..end], password).ok());
        }
    }

    /// The version ending at `end`, if it is open and readable.
    pub(crate) fn peek(&self, end: usize) -> Option<&Pdf> {
        if end == self.bytes.len() {
            return Some(self.whole);
        }
        self.cache.get(&end)?.as_ref()
    }

    fn get(&mut self, end: usize) -> Option<&Pdf> {
        self.load(end);
        self.peek(end)
    }
}

fn check_one(
    pdf: &Pdf,
    versions: &mut Versions,
    place: Place,
    fields: Vec<(crate::Field, ObjectId)>,
) -> DigitalSignature {
    let nodes: Vec<ObjectId> = fields.iter().map(|(_, id)| *id).collect();
    let fields: Vec<crate::Field> = fields.into_iter().map(|(f, _)| f).collect();
    let first_widget = fields.iter().find_map(|f| f.widgets.first());
    let mut s = DigitalSignature {
        fields: fields.iter().map(|f| f.name.clone()).collect(),
        page: first_widget.map(|w| w.page),
        bbox: first_widget.map(|w| w.bbox),
        kind: SigKind::Approval,
        sub_filter: String::new(),
        status: SigStatus::Valid,
        byte_range: None,
        covers_whole_file: false,
        signed_len: None,
        changes: Vec::new(),
        signer: None,
        certificates: Vec::new(),
        trust: None,
        name: None,
        reason: None,
        location: None,
        contact: None,
        time: None,
        timestamp: None,
        digest: None,
        algorithm: None,
        locks: None,
        warnings: Vec::new(),
    };
    if fields.len() > 1 {
        s.warnings.push(SigWarning::SharedByFields);
    }
    if let Err(f) = check_into(pdf, versions, place, &nodes, &mut s) {
        s.status = match f {
            Fail::Broken(why) => SigStatus::Broken(why),
            Fail::CantCheck(why) => SigStatus::CantCheck(why),
        };
    }
    s
}

/// The signature dictionary at `place` in `pdf`.
fn sig_dict(pdf: &Pdf, place: Place) -> Option<&Dictionary> {
    match place {
        Place::Indirect(id) => pdf.doc.get_dictionary(id).ok(),
        Place::Direct(field) => pdf
            .doc
            .get_dictionary(field)
            .ok()?
            .get(b"V")
            .ok()?
            .as_dict()
            .ok(),
        Place::NotADictionary(_) => None,
    }
}

fn broken<T>(why: impl Into<String>) -> cms_check::Check<T> {
    Err(Fail::Broken(why.into()))
}

fn cant<T>(why: impl Into<String>) -> cms_check::Check<T> {
    Err(Fail::CantCheck(why.into()))
}

const COPIED: &str =
    "the signature's dictionary was added after the version it claims to sign (a copied signature)";
const MOVED: &str =
    "the signature value isn't inside this signature's dictionary (a copied or moved signature)";

fn check_into(
    pdf: &Pdf,
    versions: &mut Versions,
    place: Place,
    nodes: &[ObjectId],
    s: &mut DigitalSignature,
) -> cms_check::Check<()> {
    let bytes = versions.bytes;
    let Some(now) = sig_dict(pdf, place) else {
        return cant("the field's signature isn't a signature dictionary");
    };
    s.sub_filter = name(pdf, now, b"SubFilter").unwrap_or_default();

    // The byte range, from the dictionary as the file now has it; everything is checked
    // against the signed version below.
    let [_, b, c, d] = byte_range(pdf, now, bytes.len())?;
    s.byte_range = Some([0, b, c, d]);
    let end = c + d;
    s.covers_whole_file = end == bytes.len();
    let Some(value) = gap_value(&bytes[b..c]) else {
        return broken("something other than the signature value sits between the signed ranges");
    };
    if !ends_revision(&bytes[..end]) {
        return broken("the signed part doesn't end where a version of the document ends");
    }

    // The version this signature signed, and the signature dictionary in it.
    if !versions.allowed(end) {
        return cant(format!(
            "the document has more signed versions than LimarePDF checks (at most {MAX_VERSIONS}, \
             {} MB together)",
            MAX_VERSION_BYTES >> 20
        ));
    }
    let Some(signed) = versions.get(end) else {
        return cant("LimarePDF can't read the version of the document this signature covers");
    };
    // What LimarePDF shows of the signed version must be what was signed in every app.
    if let Some(why) = &signed.differs {
        return cant(format!(
            "other PDF apps may read the version this signature covers differently from LimarePDF \
             ({why})"
        ));
    }
    let container = match place {
        Place::Indirect(id) | Place::Direct(id) | Place::NotADictionary(id) => id,
    };
    let offset = match signed.doc.reference_table.entries.get(&container.0) {
        Some(XrefEntry::Normal { offset, generation }) if *generation == container.1 => {
            *offset as usize
        }
        Some(XrefEntry::Compressed { .. }) => return broken(
            "the signature's dictionary is compressed, so its value can't be where the range says",
        ),
        _ => return broken(COPIED),
    };
    let direct = matches!(place, Place::Direct(_));
    match value_in_dictionary(&bytes[..end], offset, container, direct, b, c) {
        Located::Yes => {}
        Located::NoHeader => {
            return cant(
                "LimarePDF can't find the signature's dictionary where the file says it is",
            )
        }
        Located::No => return broken(MOVED),
    }
    let Some(dict) = sig_dict(signed, place) else {
        return cant("LimarePDF can't read the signature dictionary in the signed version");
    };
    if byte_range(signed, dict, end).ok() != Some([0, b, c, d]) {
        return broken("the byte range was changed after signing");
    }
    if dict != now {
        s.warnings.push(SigWarning::DetailsChangedAfterSigning);
    }
    s.signed_len = Some(end);

    // Everything shown comes from the signed dictionary.
    let text = |key: &[u8]| dict.get(key).ok().and_then(|o| signed.text_value(o));
    s.sub_filter = name(signed, dict, b"SubFilter").unwrap_or_default();
    s.name = text(b"Name");
    s.reason = text(b"Reason");
    s.location = text(b"Location");
    s.contact = text(b"ContactInfo");
    let dict_time = text(b"M").and_then(|m| parse_pdf_date(&m));
    let (kind, locks) = references(signed, dict);
    s.kind = kind;
    // A signature should repeat its field's lock; if it doesn't, the field's counts.
    s.locks = locks.or_else(|| {
        nodes.iter().find_map(|&n| {
            let l = signed.doc.get_dictionary(n).ok()?.get(b"Lock").ok()?;
            field_lock(signed, l)
        })
    });
    if name(signed, dict, b"Type").as_deref() == Some("DocTimeStamp")
        || s.sub_filter == "ETSI.RFC3161"
    {
        s.kind = SigKind::DocumentTimestamp;
    }

    // The signature value: one DER element, then zero padding.
    let used = crate::asn1::Tlv::read(&value)
        .map_err(|e| Fail::CantCheck(format!("the signature data is malformed ({e})")))?
        .raw
        .len();
    if value[used..].iter().any(|&x| x != 0) {
        s.warnings.push(SigWarning::DataAfterSignature);
    }
    let value = &value[..used];
    let ranges: [&[u8]; 2] = [&bytes[..b], &bytes[c..end]];

    let mut stamp_token = None;
    let mut claimed = None;
    let (signer, certs) = match s.sub_filter.as_str() {
        "adbe.pkcs7.detached" | "ETSI.CAdES.detached" | "adbe.pkcs7.sha1" => {
            let sd = SignedData::parse(value)
                .map_err(|e| Fail::CantCheck(format!("the signature data is malformed ({e})")))?;
            if sd.econtent_type != oid::DATA {
                return broken("the signature is for another kind of content");
            }
            let signed_part = if s.sub_filter == "adbe.pkcs7.sha1" {
                // The signature covers the SHA-1 digest of the ranges, held inside it.
                let Some(e) = &sd.econtent else {
                    return broken("the signature doesn't hold the document's digest");
                };
                if *e != DigestAlg::Sha1.hash(&ranges) {
                    return broken(
                        "the signed part was changed: its digest doesn't match the signature",
                    );
                }
                s.warnings.push(SigWarning::WeakDigest("SHA-1"));
                vec![e.as_slice()]
            } else {
                // Detached: the content isn't in the signature. Some signers put the signed
                // bytes in anyway; that is only accepted when they are exactly the ranges.
                if let Some(e) = &sd.econtent {
                    if e.len() != b + d || e[..b] != *ranges[0] || e[b..] != *ranges[1] {
                        return broken("the signature holds content other than this document");
                    }
                }
                ranges.to_vec()
            };
            let signer = cms_check::check_signer(&sd, &signed_part, oid::DATA)?;
            s.digest = Some(signer.digest.name());
            s.algorithm = Some(signer.scheme);
            if signer.digest == DigestAlg::Sha1
                && !s.warnings.contains(&SigWarning::WeakDigest("SHA-1"))
            {
                s.warnings.push(SigWarning::WeakDigest("SHA-1"));
            }
            claimed = signer.claimed_time;
            stamp_token = signer.timestamp_token.map(|t| (t, signer.value));
            (sd.certs[signer.cert].clone(), sd.certs)
        }
        "adbe.x509.rsa_sha1" => {
            let certs: Vec<Cert> = match dict.get(b"Cert").ok().map(|o| signed.doc.dereference(o)) {
                Some(Ok((_, Object::String(c, _)))) => Cert::parse(c).into_iter().collect(),
                Some(Ok((_, Object::Array(a)))) => a
                    .iter()
                    .filter_map(|c| signed.string_bytes(c))
                    .filter_map(|c| Cert::parse(&c).ok())
                    .collect(),
                _ => Vec::new(),
            };
            let Some(cert) = certs.first().cloned() else {
                return cant("the signer's certificate (/Cert) is missing or unreadable");
            };
            let digest = cms_check::check_rsa_sha1(value, &cert, &ranges)?;
            s.digest = Some(digest.name());
            s.algorithm = Some("RSA PKCS#1 v1.5");
            if digest == DigestAlg::Sha1 {
                s.warnings.push(SigWarning::WeakDigest("SHA-1"));
            }
            (cert, certs)
        }
        "ETSI.RFC3161" => {
            let stamp = cms_check::check_timestamp(value, &ranges)?;
            s.algorithm = Some("RFC 3161 timestamp");
            if !stamp.authority.for_timestamping() {
                s.warnings.push(SigWarning::NotATimestampAuthority);
            }
            s.time = Some(SigningTime {
                time: stamp.time,
                source: TimeSource::Timestamp,
            });
            // Every certificate the token carries, so that a chain through intermediate
            // authorities can be built; carried ones are never trusted for being there.
            (stamp.authority, stamp.certs)
        }
        "" => return cant("the signature doesn't say what kind it is (no /SubFilter)"),
        other => {
            return cant(format!(
                "LimarePDF doesn't know this kind of signature ({other})"
            ))
        }
    };

    // An embedded timestamp covers the signature value.
    if let Some((token, sig_value)) = stamp_token {
        match cms_check::check_timestamp(&token, &[&sig_value]) {
            Ok(st) => {
                if !st.authority.for_timestamping() {
                    s.warnings.push(SigWarning::NotATimestampAuthority);
                }
                if !st.authority.valid_at(st.time) {
                    s.warnings.push(SigWarning::TimestampProblem(
                        "the timestamp authority's certificate wasn't valid at that time".into(),
                    ));
                }
                s.timestamp = Some(Timestamp {
                    time: st.time,
                    authority: st.authority.info(),
                    certificates: st.certs.iter().map(Cert::info).collect(),
                    trust: None,
                });
            }
            Err(Fail::Broken(why) | Fail::CantCheck(why)) => {
                s.warnings.push(SigWarning::TimestampProblem(why));
            }
        }
    }
    if s.time.is_none() {
        s.time = s
            .timestamp
            .as_ref()
            .map(|t| SigningTime {
                time: t.time,
                source: TimeSource::Timestamp,
            })
            .or(claimed.map(|time| SigningTime {
                time,
                source: TimeSource::SignerClock,
            }))
            .or(dict_time.map(|time| SigningTime {
                time,
                source: TimeSource::Dictionary,
            }));
    }

    if let Some(bits) = cms_check::rsa_bits(&signer).filter(|&b| b < 2048) {
        s.warnings.push(SigWarning::ShortKey(bits));
    }
    if s.time.is_some_and(|t| !signer.valid_at(t.time)) {
        s.warnings.push(SigWarning::CertificateNotValidAtSigning);
    }
    if s.kind != SigKind::DocumentTimestamp && !signer.key_usage_allows_signing() {
        s.warnings.push(SigWarning::KeyNotForSigning);
    }
    s.signer = Some(signer.info());
    s.certificates = certs.iter().map(Cert::info).collect();
    Ok(())
}

/// A name entry as text.
fn name(pdf: &Pdf, d: &Dictionary, key: &[u8]) -> Option<String> {
    let o = pdf.doc.dereference(d.get(key).ok()?).ok()?.1;
    Some(String::from_utf8_lossy(o.as_name().ok()?).into_owned())
}

/// The `/ByteRange`, checked: four whole numbers `[0 b c d]` with `0 < b < c`, inside a
/// file of `len` bytes.
fn byte_range(pdf: &Pdf, d: &Dictionary, len: usize) -> cms_check::Check<[usize; 4]> {
    let arr = d
        .get(b"ByteRange")
        .ok()
        .and_then(|o| pdf.doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_array().ok());
    let Some(arr) = arr else {
        return broken("the signature has no byte range");
    };
    let nums: Option<Vec<i64>> = arr.iter().map(|o| o.as_i64().ok()).collect();
    let Some(&[a, b, c, d]) = nums.as_deref() else {
        return broken("the byte range must be exactly four whole numbers");
    };
    if a != 0 {
        return broken("the signed part doesn't start at the beginning of the file");
    }
    let (Ok(b), Ok(c), Ok(d)) = (usize::try_from(b), usize::try_from(c), usize::try_from(d)) else {
        return broken("the byte range holds negative numbers");
    };
    if b == 0 || c <= b {
        return broken("the byte range is out of order");
    }
    if c.checked_add(d).is_none_or(|e| e > len) {
        return broken("the byte range runs past the end of the file");
    }
    Ok([0, b, c, d])
}

pub(crate) fn is_white(ch: u8) -> bool {
    matches!(ch, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'\0')
}

fn is_delim(ch: u8) -> bool {
    matches!(
        ch,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// The bytes of the hex string that must make up the whole gap between the signed
/// ranges: `<`, hex digits (white space allowed, as in any PDF hex string), `>`. Anything
/// else in the gap is unsigned content and refused.
fn gap_value(gap: &[u8]) -> Option<Vec<u8>> {
    let inner = gap.strip_prefix(b"<")?.strip_suffix(b">")?;
    let mut out = Vec::with_capacity(inner.len() / 2);
    let mut high = None;
    for &ch in inner.iter().filter(|&&ch| !is_white(ch)) {
        let v = (ch as char).to_digit(16)? as u8;
        match high.take() {
            None => high = Some(v),
            Some(h) => out.push((h << 4) | v),
        }
    }
    if let Some(h) = high {
        out.push(h << 4);
    }
    Some(out)
}

/// Whether `prefix` ends where a version of the document ends: at `%%EOF`, perhaps
/// followed by an end of line.
fn ends_revision(prefix: &[u8]) -> bool {
    let n = prefix.iter().rev().take_while(|&&ch| is_white(ch)).count();
    prefix[..prefix.len() - n].ends_with(b"%%EOF")
}

enum Located {
    Yes,
    No,
    /// The object `id` doesn't start where the cross-reference table says.
    NoHeader,
}

/// Whether the hex string at `b..c` is the `/Contents` value of the signature dictionary
/// in object `id`, starting at `offset` of `bytes`: the object's own dictionary, or, when
/// `direct`, the dictionary under its `/V`. This ties the value to its dictionary by
/// position, so a copied dictionary can't borrow another signature's value.
fn value_in_dictionary(
    bytes: &[u8],
    offset: usize,
    id: ObjectId,
    direct: bool,
    b: usize,
    c: usize,
) -> Located {
    let mut lx = Lexer { bytes, pos: offset };
    // `N G obj`, perhaps after white space.
    let number = |t: Option<(usize, Tok, usize)>| match t {
        Some((_, Tok::Word(w), _)) => std::str::from_utf8(w).ok()?.parse::<u32>().ok(),
        _ => None,
    };
    let (n, g) = (number(lx.next()), number(lx.next()));
    let obj = matches!(lx.next(), Some((_, Tok::Word(b"obj"), _)));
    if !obj || n != Some(id.0) || g != Some(u32::from(id.1)) {
        return Located::NoHeader;
    }
    // For each open dictionary or array: whether it's a dictionary, and the name before it.
    let mut frames: Vec<(bool, Option<&[u8]>)> = Vec::new();
    let mut prev_name: Option<&[u8]> = None;
    while let Some((start, tok, end)) = lx.next() {
        if start > b || (start < b && end > b) {
            return Located::No;
        }
        if frames.is_empty() && !matches!(tok, Tok::DictOpen) {
            return Located::No;
        }
        match tok {
            Tok::Hex if start == b => {
                let shape = match frames[..] {
                    [(true, None)] => !direct,
                    [(true, None), (true, Some(b"V"))] => direct,
                    _ => false,
                };
                return if end == c && shape && prev_name == Some(b"Contents") {
                    Located::Yes
                } else {
                    Located::No
                };
            }
            Tok::DictOpen | Tok::ArrayOpen => {
                frames.push((matches!(tok, Tok::DictOpen), prev_name));
                prev_name = None;
            }
            Tok::DictClose | Tok::ArrayClose => {
                frames.pop();
                if frames.is_empty() {
                    return Located::No;
                }
                prev_name = None;
            }
            Tok::Name(n) => prev_name = Some(n),
            _ => prev_name = None,
        }
    }
    Located::No
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tok<'a> {
    DictOpen,
    DictClose,
    ArrayOpen,
    ArrayClose,
    Name(&'a [u8]),
    Hex,
    Literal,
    Word(&'a [u8]),
}

/// Just enough of a PDF tokenizer to walk one object's dictionary.
#[derive(Clone)]
pub(crate) struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub(crate) fn new(bytes: &'a [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }

    /// Where the next token starts looking.
    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    /// The next token with its start and end, or `None` at the end or on malformed input.
    #[allow(clippy::should_implement_trait)]
    pub(crate) fn next(&mut self) -> Option<(usize, Tok<'a>, usize)> {
        let b = self.bytes;
        loop {
            while self.pos < b.len() && is_white(b[self.pos]) {
                self.pos += 1;
            }
            if b.get(self.pos) == Some(&b'%') {
                while self.pos < b.len() && !matches!(b[self.pos], b'\n' | b'\r') {
                    self.pos += 1;
                }
                continue;
            }
            break;
        }
        let start = self.pos;
        let tok = match *b.get(start)? {
            b'<' if b.get(start + 1) == Some(&b'<') => {
                self.pos += 2;
                Tok::DictOpen
            }
            b'>' if b.get(start + 1) == Some(&b'>') => {
                self.pos += 2;
                Tok::DictClose
            }
            b'<' => {
                let len = b[start..].iter().position(|&ch| ch == b'>')?;
                if !b[start + 1..start + len]
                    .iter()
                    .all(|&ch| ch.is_ascii_hexdigit() || is_white(ch))
                {
                    return None;
                }
                self.pos = start + len + 1;
                Tok::Hex
            }
            b'[' => {
                self.pos += 1;
                Tok::ArrayOpen
            }
            b']' => {
                self.pos += 1;
                Tok::ArrayClose
            }
            b'(' => {
                let mut depth = 0usize;
                let mut i = start;
                loop {
                    match *b.get(i)? {
                        b'\\' => i += 1,
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                self.pos = i + 1;
                Tok::Literal
            }
            b'/' => {
                let n = b[start + 1..]
                    .iter()
                    .take_while(|&&ch| !is_white(ch) && !is_delim(ch))
                    .count();
                self.pos = start + 1 + n;
                Tok::Name(&b[start + 1..self.pos])
            }
            b')' | b'>' => return None,
            b'{' | b'}' => {
                self.pos += 1;
                Tok::Word(&b[start..self.pos])
            }
            _ => {
                let n = b[start..]
                    .iter()
                    .take_while(|&&ch| !is_white(ch) && !is_delim(ch))
                    .count();
                self.pos = start + n;
                Tok::Word(&b[start..self.pos])
            }
        };
        Some((start, tok, self.pos))
    }
}

/// The kind of signature and its field locks, from the signature dictionary's
/// `/Reference` entries (DocMDP for certification, FieldMDP for locks).
fn references(pdf: &Pdf, d: &Dictionary) -> (SigKind, Option<FieldLock>) {
    let mut kind = SigKind::Approval;
    let mut locks = None;
    let refs = d
        .get(b"Reference")
        .ok()
        .and_then(|o| pdf.doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_array().ok());
    for r in refs.into_iter().flatten() {
        let Some(r) = pdf
            .doc
            .dereference(r)
            .ok()
            .and_then(|(_, o)| o.as_dict().ok())
        else {
            continue;
        };
        let params = r
            .get(b"TransformParams")
            .ok()
            .and_then(|o| pdf.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok());
        match name(pdf, r, b"TransformMethod").as_deref() {
            Some("DocMDP") => {
                let p = params
                    .and_then(|p| p.get(b"P").ok())
                    .and_then(|o| pdf.int_value(o))
                    .unwrap_or(2);
                kind = SigKind::Certification(match p {
                    1 => Permissions::NoChanges,
                    3 => Permissions::FormFillingAndComments,
                    _ => Permissions::FormFilling,
                });
            }
            Some("FieldMDP") => {
                if let Some(p) = params {
                    locks = lock_from(pdf, p);
                }
            }
            _ => {}
        }
    }
    (kind, locks)
}

/// A field lock from a `/Lock` dictionary (or FieldMDP parameters, which have the same
/// `/Action` and `/Fields`).
fn field_lock(pdf: &Pdf, o: &Object) -> Option<FieldLock> {
    lock_from(pdf, pdf.doc.dereference(o).ok()?.1.as_dict().ok()?)
}

fn lock_from(pdf: &Pdf, d: &Dictionary) -> Option<FieldLock> {
    let action = match name(pdf, d, b"Action")?.as_str() {
        "All" => LockAction::All,
        "Include" => LockAction::Include,
        "Exclude" => LockAction::Exclude,
        _ => return None,
    };
    let fields = d
        .get(b"Fields")
        .ok()
        .and_then(|o| pdf.doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_array().ok())
        .map(|a| a.iter().filter_map(|f| pdf.text_value(f)).collect())
        .unwrap_or_default();
    let permissions = d
        .get(b"P")
        .ok()
        .and_then(|o| pdf.int_value(o))
        .and_then(|p| match p {
            1 => Some(Permissions::NoChanges),
            2 => Some(Permissions::FormFilling),
            3 => Some(Permissions::FormFillingAndComments),
            _ => None,
        });
    Some(FieldLock {
        action,
        fields,
        permissions,
    })
}

/// Certification rules across signatures: it must be the first signature, and the
/// document's `/Perms /DocMDP` must point to it.
fn certification_rules(pdf: &Pdf, sigs: &mut [DigitalSignature]) {
    let registered = pdf
        .doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Perms").ok())
        .and_then(|o| pdf.doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok())
        .and_then(|p| p.get(b"DocMDP").ok())
        .and_then(|o| o.as_reference().ok());
    let registered_fields: Vec<String> = registered
        .map(|id| {
            pdf.form_nodes()
                .into_iter()
                .filter(|(_, n)| {
                    pdf.doc
                        .get_dictionary(n.id)
                        .ok()
                        .and_then(|d| d.get(b"V").ok())
                        .and_then(|v| v.as_reference().ok())
                        == Some(id)
                })
                .map(|(f, _)| f.name)
                .collect()
        })
        .unwrap_or_default();
    let first_end = sigs
        .iter()
        .find_map(|s| s.byte_range.map(|[_, _, c, d]| c + d));
    for s in sigs.iter_mut() {
        if !matches!(s.kind, SigKind::Certification(_)) {
            continue;
        }
        if s.byte_range.map(|[_, _, c, d]| c + d) != first_end {
            s.warnings.push(SigWarning::CertificationNotFirst);
        }
        if !s.fields.iter().any(|f| registered_fields.contains(f)) {
            s.warnings.push(SigWarning::CertificationNotRegistered);
        }
    }
}

/// A PDF date, `D:YYYYMMDDHHmmSSOHH'mm'`, where everything after the year is optional.
/// Without a time zone, UTC is assumed.
pub(crate) fn parse_pdf_date(s: &str) -> Option<DateTime<FixedOffset>> {
    let s = s.trim();
    let s = s.strip_prefix("D:").unwrap_or(s);
    let digits = s.bytes().take(14).take_while(u8::is_ascii_digit).count();
    // Each part is present in full or, from some part on, left out.
    let num = |i: usize, n: usize, default: u32| -> Option<u32> {
        match i + n <= digits {
            true => s[i..i + n].parse().ok(),
            false if i >= digits => Some(default),
            false => None,
        }
    };
    if digits < 4 {
        return None;
    }
    let year = num(0, 4, 0)? as i32;
    let (mo, d, h, mi, sec) = (
        num(4, 2, 1)?,
        num(6, 2, 1)?,
        num(8, 2, 0)?,
        num(10, 2, 0)?,
        num(12, 2, 0)?,
    );
    let zone = s.get(digits..).unwrap_or("");
    let offset = match zone.as_bytes().first() {
        None | Some(b'Z') => 0,
        Some(&sign @ (b'+' | b'-')) => {
            let z: String = zone[1..].chars().filter(char::is_ascii_digit).collect();
            let oh: i32 = z.get(..2).unwrap_or("0").parse().ok()?;
            let om: i32 = z.get(2..4).unwrap_or("0").parse().ok()?;
            let secs = oh * 3600 + om * 60;
            if sign == b'-' {
                -secs
            } else {
                secs
            }
        }
        _ => return None,
    };
    let naive = NaiveDate::from_ymd_opt(year, mo, d)?.and_hms_opt(h, mi, sec)?;
    naive
        .and_local_timezone(FixedOffset::east_opt(offset)?)
        .single()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_dates() {
        let t = parse_pdf_date("D:20261016142305-04'00'").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-10-16T14:23:05-04:00");
        let t = parse_pdf_date("D:20261016142305Z").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-10-16T14:23:05+00:00");
        assert_eq!(
            parse_pdf_date("D:2026").unwrap().to_rfc3339(),
            "2026-01-01T00:00:00+00:00"
        );
        assert_eq!(
            parse_pdf_date("D:202610161423+0530").unwrap().to_rfc3339(),
            "2026-10-16T14:23:00+05:30"
        );
        for bad in [
            "",
            "D:",
            "D:20",
            "D:2026x",
            "D:20261316",
            "D:20261016142305?",
        ] {
            assert!(parse_pdf_date(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn gap_must_be_one_hex_string() {
        assert_eq!(gap_value(b"<3082 01ff>").unwrap(), [0x30, 0x82, 0x01, 0xff]);
        assert_eq!(gap_value(b"<abc>").unwrap(), [0xab, 0xc0]);
        for bad in [
            &b""[..],
            b"<",
            b"<>x",
            b" <00>",
            b"<00> ",
            b"<0g>",
            b"<00>>",
            b"(00)",
        ] {
            assert!(gap_value(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn revision_ends() {
        assert!(ends_revision(b"...%%EOF"));
        assert!(ends_revision(b"...%%EOF\r\n"));
        assert!(!ends_revision(b"...%%EOF\r\nxref"));
        assert!(!ends_revision(b"...endobj\n"));
    }

    #[test]
    fn value_must_be_this_dictionarys_contents() {
        fn located(
            file: &[u8],
            offset: usize,
            id: u32,
            direct: bool,
            b: usize,
            c: usize,
        ) -> &'static str {
            match value_in_dictionary(file, offset, (id, 0), direct, b, c) {
                Located::Yes => "yes",
                Located::No => "no",
                Located::NoHeader => "no header",
            }
        }
        let file = b"5 0 obj\n<</Type/Sig/Reason(a (nested) one\\))/Contents <00ff>/ByteRange[0 1 2 3]>>\nendobj";
        let b = file.windows(6).position(|w| w == b"<00ff>").unwrap();
        let c = b + 6;
        assert_eq!(located(file, 0, 5, false, b, c), "yes");
        assert_eq!(located(file, 0, 5, true, b, c), "no");
        assert_eq!(located(file, 0, 5, false, b, c - 1), "no");
        assert_eq!(located(file, 3, 5, false, b, c), "no header");
        // Another object's header where this one should start.
        assert_eq!(located(file, 0, 6, false, b, c), "no header");
        // The value under another key, or inside a nested dictionary.
        let file = b"5 0 obj <</Other <00ff> /Contents <11>>> endobj";
        let b = file.windows(6).position(|w| w == b"<00ff>").unwrap();
        assert_eq!(located(file, 0, 5, false, b, b + 6), "no");
        let file = b"5 0 obj <</X <</Contents <00ff>>> >> endobj";
        let b = file.windows(6).position(|w| w == b"<00ff>").unwrap();
        assert_eq!(located(file, 0, 5, false, b, b + 6), "no");
        // A field with its signature dictionary written inside, under /V.
        let file = b"7 0 obj <</FT/Sig/V <</Contents <00ff>>> >> endobj";
        let b = file.windows(6).position(|w| w == b"<00ff>").unwrap();
        assert_eq!(located(file, 0, 7, true, b, b + 6), "yes");
        // The object ends before the value: someone else's value.
        let file = b"5 0 obj <</Type/Sig>> endobj 6 0 obj <</Contents <00ff>>> endobj";
        let b = file.windows(6).position(|w| w == b"<00ff>").unwrap();
        assert_eq!(located(file, 0, 5, false, b, b + 6), "no");
    }
}
