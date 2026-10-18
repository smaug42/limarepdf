//! Trust: the signers and certificate authorities the user trusts, kept in `trust.txt` in
//! LimarePDF's settings folder, and deciding whether a valid signature's certificate leads to one
//! of them. See docs/signatures.md, "Trusting signers".
//!
//! A signer is trusted when its own certificate is in the list, or when a chain of
//! certificates leads from it to a certificate authority in the list: each certificate
//! signed by the next one's key, each issuer allowed to issue certificates, and every one
//! valid at the signing time. The chain is built from the certificates the signature
//! carries and the document's validation data; those only help find the way, trust comes
//! from the list alone. Revocation isn't checked. Trust settings affect LimarePDF only.

use crate::cms_check::{self, Cert, CertInfo, DigestAlg, Fail};
use crate::Result;
use anyhow::{anyhow, bail, Context};
use chrono::{DateTime, NaiveDate, Utc};
use der::asn1::ObjectIdentifier as Oid;
use std::fmt;
use std::path::{Path, PathBuf};

/// What a certificate in the trust list is trusted for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustRole {
    /// Signatures made with this certificate.
    Signer,
    /// Certificates this authority issued, directly or through other authorities it
    /// allowed to issue them (and signatures made with its own certificate).
    Authority,
}

impl TrustRole {
    fn word(self) -> &'static str {
        match self {
            Self::Signer => "signer",
            Self::Authority => "authority",
        }
    }
}

impl fmt::Display for TrustRole {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Self::Signer => "a signer",
            Self::Authority => "a certificate authority",
        })
    }
}

/// A certificate the user trusts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustEntry {
    pub role: TrustRole,
    /// A name for people, by default the certificate's.
    pub label: String,
    /// When it was trusted, if known.
    pub added: Option<NaiveDate>,
    pub cert: CertInfo,
}

/// The certificates the user trusts. A file that can't be read gives an empty list; an
/// entry that can't be used is left out, with the reason in [`TrustList::problems`], and
/// kept as written when the list is saved.
#[derive(Clone, Debug, Default)]
pub struct TrustList {
    pub entries: Vec<TrustEntry>,
    /// Why entries in the file were left out.
    pub problems: Vec<String>,
    /// Those entries as written.
    kept: Vec<String>,
}

/// What [`TrustList::add`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Added {
    New,
    /// The certificate was already trusted, as this other role.
    RoleChanged(TrustRole),
}

/// The largest trust file read: far more than any real list.
const MAX_FILE: u64 = 16 << 20;

const HEADER: &str = "\
# Certificates LimarePDF trusts when it checks digital signatures. These settings affect LimarePDF only.
# Change them with `limarepdf-cli trust` or in the app. Each entry is [signer] or [authority], a label,
# the date it was added, the certificate's SHA-256 fingerprint, and the certificate.
";

impl TrustList {
    /// `trust.txt` in LimarePDF's settings folder; `None` when there is no settings folder.
    pub fn default_path() -> Option<PathBuf> {
        crate::settings_dir().map(|d| d.join("trust.txt"))
    }

    /// Reads the list at `path`; a missing file is an empty list. Fails when the file
    /// can't be read or, on Unix, when other users could change it (anyone who can write
    /// it can make LimarePDF trust any signature).
    pub fn load(path: &Path) -> Result<Self> {
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o022 != 0 {
                bail!(
                    "{} can be changed by other users of this computer, so LimarePDF doesn't use it; \
                     make it private (chmod 600) to use it again",
                    path.display()
                );
            }
        }
        if meta.len() > MAX_FILE {
            bail!("{} is too large to be a trust list", path.display());
        }
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| anyhow!("{} isn't a text file", path.display()))?;
        Ok(Self::parse(&text))
    }

    /// Reads a list from the text of a trust file.
    pub fn parse(text: &str) -> Self {
        let mut list = Self::default();
        let mut blocks: Vec<Vec<&str>> = Vec::new();
        for line in text.lines() {
            if line.trim_start().starts_with('[') {
                blocks.push(vec![line]);
            } else if let Some(b) = blocks.last_mut() {
                b.push(line);
            }
        }
        for (i, block) in blocks.iter().enumerate() {
            let raw = block.join("\n").trim_end().to_string();
            match parse_entry(block) {
                Ok(e) => {
                    if let Some(first) =
                        list.entries.iter().find(|f| f.cert.sha256 == e.cert.sha256)
                    {
                        list.problems.push(format!(
                            "entry {} (\"{}\") repeats the certificate of \"{}\"; the first is used",
                            i + 1,
                            e.label,
                            first.label
                        ));
                        list.kept.push(raw);
                    } else {
                        list.entries.push(e);
                    }
                }
                Err(why) => {
                    list.problems
                        .push(format!("entry {} isn't used: {why}", i + 1));
                    list.kept.push(raw);
                }
            }
        }
        list
    }

    /// The text of the trust file.
    pub fn to_text(&self) -> String {
        let mut out = String::from(HEADER);
        for e in &self.entries {
            out.push('\n');
            out.push_str(&format!("[{}]\n", e.role.word()));
            out.push_str(&format!("label = {}\n", one_line(&e.label)));
            if let Some(d) = e.added {
                out.push_str(&format!("added = {}\n", d.format("%Y-%m-%d")));
            }
            out.push_str(&format!("sha256 = {}\n", e.cert.sha256));
            out.push_str(&pem(&e.cert.der));
        }
        for k in &self.kept {
            out.push('\n');
            out.push_str(k);
            out.push('\n');
        }
        out
    }

    /// Writes the list to `path`, replacing the file in one step, readable and writable by
    /// this user only on Unix.
    pub fn save(&self, path: &Path) -> Result<()> {
        crate::write_private(path, self.to_text().as_bytes())
    }

    /// Trusts the certificate `der` as `role`, labelled `label` (by default the
    /// certificate's name). A certificate authority must be allowed to issue certificates.
    /// A certificate already trusted as the other role changes role. Today's date is
    /// recorded.
    pub fn add(&mut self, der: &[u8], role: TrustRole, label: Option<&str>) -> Result<Added> {
        let today = chrono::Local::now().date_naive();
        let cert = Cert::parse(der)?;
        if role == TrustRole::Authority {
            if let Some(why) = authority_problem(&cert) {
                bail!("this certificate can't be trusted as a certificate authority: {why}");
            }
        }
        let info = cert.info();
        let label = label
            .map(one_line)
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| one_line(info.display_name()));
        match self
            .entries
            .iter_mut()
            .find(|e| e.cert.sha256 == info.sha256)
        {
            Some(e) if e.role == role => {
                bail!("\"{}\" is already trusted as {role}", e.label)
            }
            Some(e) => {
                let old = e.role;
                e.role = role;
                e.label = label;
                e.added = Some(today);
                Ok(Added::RoleChanged(old))
            }
            None => {
                self.entries.push(TrustEntry {
                    role,
                    label,
                    added: Some(today),
                    cert: info,
                });
                Ok(Added::New)
            }
        }
    }

    /// The position of the entry with this SHA-256 fingerprint (hex, with or without
    /// colons or spaces, any case).
    pub fn find(&self, sha256: &str) -> Option<usize> {
        let want = normal_hex(sha256);
        (!want.is_empty())
            .then(|| {
                self.entries
                    .iter()
                    .position(|e| normal_hex(&e.cert.sha256) == want)
            })
            .flatten()
    }

    /// Stops trusting the entry at `index` and returns it.
    pub fn remove(&mut self, index: usize) -> Option<TrustEntry> {
        (index < self.entries.len()).then(|| self.entries.remove(index))
    }
}

fn normal_hex(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, ':' | ' '))
        .collect::<String>()
        .to_ascii_uppercase()
}

/// `s` on one line: control characters (line breaks among them) become spaces.
fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

fn pem(der: &[u8]) -> String {
    der::pem::encode_string("CERTIFICATE", der::pem::LineEnding::LF, der)
        .expect("a certificate fits in PEM")
}

fn parse_entry(lines: &[&str]) -> std::result::Result<TrustEntry, String> {
    let head = lines[0].trim();
    let role = match head {
        "[signer]" => TrustRole::Signer,
        "[authority]" => TrustRole::Authority,
        other => return Err(format!("unknown kind of entry {other}")),
    };
    let (mut label, mut added, mut sha256) = (None, None, None);
    let mut pem_lines: Option<Vec<&str>> = None;
    let mut pem_done = None;
    for line in &lines[1..] {
        let t = line.trim();
        if let Some(p) = pem_lines.as_mut() {
            p.push(t);
            if t.starts_with("-----END ") {
                pem_done = pem_lines.take();
            }
            continue;
        }
        if t.starts_with("-----BEGIN ") {
            if pem_done.is_some() {
                return Err("it holds more than one certificate".into());
            }
            pem_lines = Some(vec![t]);
            continue;
        }
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let Some((k, v)) = t.split_once('=') else {
            return Err(format!("unreadable line \"{}\"", one_line(t)));
        };
        let v = v.trim();
        match k.trim() {
            "label" => label = Some(v.to_string()),
            "added" => {
                added = Some(
                    NaiveDate::parse_from_str(v, "%Y-%m-%d")
                        .map_err(|_| format!("unreadable date \"{}\"", one_line(v)))?,
                )
            }
            "sha256" => sha256 = Some(v.to_string()),
            _ => {}
        }
    }
    if pem_lines.is_some() {
        return Err("its certificate isn't complete".into());
    }
    let pem_text = pem_done.ok_or("it has no certificate")?.join("\n");
    let (kind, der) = der::pem::decode_vec(pem_text.as_bytes())
        .map_err(|e| format!("its certificate is unreadable ({e})"))?;
    if kind != "CERTIFICATE" {
        return Err(format!("it holds a {kind}, not a certificate"));
    }
    let cert = Cert::parse(&der).map_err(|e| e.to_string())?;
    let info = cert.info();
    if sha256.is_some_and(|f| normal_hex(&f) != normal_hex(&info.sha256)) {
        return Err("the fingerprint written doesn't match its certificate".into());
    }
    if role == TrustRole::Authority {
        if let Some(why) = authority_problem(&cert) {
            return Err(format!("it can't be a certificate authority: {why}"));
        }
    }
    Ok(TrustEntry {
        role,
        label: label
            .map(|l| one_line(&l))
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| one_line(info.display_name())),
        added,
        cert: info,
    })
}

/// Whether `c` can be trusted as a certificate authority: it says it is one and may sign
/// certificates.
pub fn is_authority(c: &CertInfo) -> bool {
    Cert::parse(&c.der).is_ok_and(|c| authority_problem(&c).is_none())
}

/// Reads the certificates in a file: DER or PEM certificates, or a PKCS#7 certificate
/// bundle (`.p7c`, `.p7b`: what Acrobat's "Export" writes), DER or PEM.
pub fn read_certificates(bytes: &[u8]) -> Result<Vec<CertInfo>> {
    let text = std::str::from_utf8(bytes).ok();
    if let Some(t) = text.filter(|t| t.contains("-----BEGIN ")) {
        let mut out = Vec::new();
        let mut rest = t;
        while let Some(start) = rest.find("-----BEGIN ") {
            let after = &rest[start..];
            let label_end = after[11..]
                .find("-----")
                .ok_or_else(|| anyhow!("a PEM block isn't complete"))?;
            let label = &after[11..11 + label_end];
            let end_marker = format!("-----END {label}-----");
            let end = after
                .find(&end_marker)
                .ok_or_else(|| anyhow!("a PEM block isn't complete"))?
                + end_marker.len();
            let (kind, der) = der::pem::decode_vec(&after.as_bytes()[..end])
                .map_err(|e| anyhow!("unreadable PEM ({e})"))?;
            match kind {
                "CERTIFICATE" | "X509 CERTIFICATE" => out.push(Cert::parse(&der)?.info()),
                "PKCS7" | "CMS" => out.extend(bundle(&der)?),
                _ => {}
            }
            rest = &after[end..];
        }
        if out.is_empty() {
            bail!("the file holds no certificate");
        }
        return Ok(out);
    }
    if let Ok(c) = Cert::parse(bytes) {
        return Ok(vec![c.info()]);
    }
    bundle(bytes).map_err(|_| anyhow!("the file isn't a certificate (.cer, .crt, .pem or .p7c)"))
}

/// The certificates in a PKCS#7 / CMS SignedData bundle.
fn bundle(der: &[u8]) -> Result<Vec<CertInfo>> {
    let sd = cms_check::SignedData::parse(der)?;
    if sd.certs.is_empty() {
        bail!("the bundle holds no certificate");
    }
    Ok(sd.certs.iter().map(Cert::info).collect())
}

// ------------------------------------------------------------------ checking

/// Whether a valid signature's signer is someone the user trusts. Trust settings affect
/// LimarePDF only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trust {
    /// The signer's own certificate is in the trust list.
    Signer { label: String },
    /// A certificate authority in the trust list issued the signer's certificate. `chain`
    /// runs from the signer's certificate up to the authority's.
    Authority { label: String, chain: Vec<CertInfo> },
    /// Nothing in the trust list vouches for the signer, with why: the usual result for a
    /// self-made digital ID nobody has trusted yet.
    Unknown(String),
    /// The certificate is in the trust list, or leads to an authority there, but fails a
    /// check, with the reason: not trusted, and worth a closer look.
    Failed(String),
}

impl Trust {
    pub fn is_trusted(&self) -> bool {
        matches!(self, Self::Signer { .. } | Self::Authority { .. })
    }
}

/// What a certificate is checked as.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Purpose {
    Signing,
    Timestamping,
}

/// The longest chain followed, the trusted authority included.
const MAX_CHAIN: usize = 10;
/// The most certificate signatures checked for one signer, so crafted certificates can't
/// make the search run long.
const MAX_CHECKS: usize = 200;

mod ext {
    use der::asn1::ObjectIdentifier as Oid;
    pub const SUBJECT_KEY_ID: Oid = Oid::new_unwrap("2.5.29.14");
    pub const KEY_USAGE: Oid = Oid::new_unwrap("2.5.29.15");
    pub const SUBJECT_ALT_NAME: Oid = Oid::new_unwrap("2.5.29.17");
    pub const ISSUER_ALT_NAME: Oid = Oid::new_unwrap("2.5.29.18");
    pub const BASIC_CONSTRAINTS: Oid = Oid::new_unwrap("2.5.29.19");
    pub const NAME_CONSTRAINTS: Oid = Oid::new_unwrap("2.5.29.30");
    pub const CRL_POINTS: Oid = Oid::new_unwrap("2.5.29.31");
    pub const POLICIES: Oid = Oid::new_unwrap("2.5.29.32");
    pub const POLICY_MAPPINGS: Oid = Oid::new_unwrap("2.5.29.33");
    pub const AUTHORITY_KEY_ID: Oid = Oid::new_unwrap("2.5.29.35");
    pub const POLICY_CONSTRAINTS: Oid = Oid::new_unwrap("2.5.29.36");
    pub const EXT_KEY_USAGE: Oid = Oid::new_unwrap("2.5.29.37");
    pub const FRESHEST_CRL: Oid = Oid::new_unwrap("2.5.29.46");
    pub const INHIBIT_ANY_POLICY: Oid = Oid::new_unwrap("2.5.29.54");
    pub const AUTHORITY_INFO: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.1.1");
    pub const QC_STATEMENTS: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.1.3");
    pub const SUBJECT_INFO: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.1.11");
    pub const OCSP_NO_CHECK: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.48.1.5");

    // Extended key usages.
    pub const ANY_EKU: Oid = Oid::new_unwrap("2.5.29.37.0");
    pub const CODE_SIGNING: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.3.3");
    pub const EMAIL_PROTECTION: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.3.4");
    pub const DOCUMENT_SIGNING: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.3.36");
    pub const ADOBE_AUTHENTIC_DOCUMENTS: Oid = Oid::new_unwrap("1.2.840.113583.1.1.5");
    pub const MS_DOCUMENT_SIGNING: Oid = Oid::new_unwrap("1.3.6.1.4.1.311.10.3.12");
    pub const TIME_STAMPING: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.3.8");
}

/// Extensions LimarePDF understands well enough to accept when marked critical.
const KNOWN: [Oid; 17] = [
    ext::SUBJECT_KEY_ID,
    ext::KEY_USAGE,
    ext::SUBJECT_ALT_NAME,
    ext::ISSUER_ALT_NAME,
    ext::BASIC_CONSTRAINTS,
    ext::CRL_POINTS,
    // Policies are processed as RFC 5280 6.1 says (`policy_problem`).
    ext::POLICIES,
    ext::POLICY_MAPPINGS,
    ext::POLICY_CONSTRAINTS,
    ext::AUTHORITY_KEY_ID,
    ext::EXT_KEY_USAGE,
    ext::FRESHEST_CRL,
    ext::INHIBIT_ANY_POLICY,
    ext::AUTHORITY_INFO,
    ext::QC_STATEMENTS,
    ext::SUBJECT_INFO,
    ext::OCSP_NO_CHECK,
];

/// Extended key usages that allow signing documents: Acrobat's (email protection, code
/// signing, any, Adobe Authentic Documents) and the two made for document signing.
const SIGNING_USES: [Oid; 6] = [
    ext::EMAIL_PROTECTION,
    ext::CODE_SIGNING,
    ext::ANY_EKU,
    ext::ADOBE_AUTHENTIC_DOCUMENTS,
    ext::DOCUMENT_SIGNING,
    ext::MS_DOCUMENT_SIGNING,
];

fn name_of(c: &Cert) -> String {
    let i = c.info();
    format!("\"{}\"", i.display_name())
}

fn date(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d").to_string()
}

/// Problems any certificate in a chain may have: not valid at `at`, a repeated extension,
/// a critical extension LimarePDF doesn't know, or constraints LimarePDF doesn't check.
fn cert_problem(c: &Cert, at: DateTime<Utc>) -> Option<String> {
    let info = c.info();
    if at < info.not_before || at > info.not_after {
        return Some(format!(
            "the certificate of {} wasn't valid at the signing time, {} (valid {} to {})",
            name_of(c),
            date(at),
            date(info.not_before),
            date(info.not_after)
        ));
    }
    if let Some(bits) = cms_check::rsa_bits(c).filter(|&b| b < 2048) {
        return Some(format!(
            "the certificate of {} has a {bits}-bit RSA key, too short to be safe",
            name_of(c)
        ));
    }
    let exts = c
        .cert
        .tbs_certificate()
        .extensions()
        .cloned()
        .unwrap_or_default();
    for (i, e) in exts.iter().enumerate() {
        if exts[..i].iter().any(|p| p.extn_id == e.extn_id) {
            return Some(format!(
                "the certificate of {} repeats an extension ({}), which isn't allowed",
                name_of(c),
                e.extn_id
            ));
        }
        if e.extn_id == ext::NAME_CONSTRAINTS {
            return Some(format!(
                "the certificate of {} sets name constraints, which LimarePDF doesn't check",
                name_of(c)
            ));
        }
        if e.extn_id == ext::POLICY_MAPPINGS {
            // Mapping to or from any policy is forbidden outright (RFC 5280 6.1.4 (a)); the
            // rest of policy processing is in `policy_problem`.
            use x509_cert::ext::pkix::PolicyMappings;
            let any = Oid::new_unwrap("2.5.29.32.0");
            let ok = c.extension::<PolicyMappings>().is_some_and(|m| {
                m.0.iter()
                    .all(|p| p.issuer_domain_policy != any && p.subject_domain_policy != any)
            });
            if !ok {
                return Some(format!(
                    "the certificate of {} has policy mappings that aren't allowed",
                    name_of(c)
                ));
            }
        }
        if e.critical && !KNOWN.contains(&e.extn_id) {
            return Some(format!(
                "the certificate of {} has a critical extension LimarePDF doesn't know ({})",
                name_of(c),
                e.extn_id
            ));
        }
    }
    // Limits that can't be read must not read as no limits.
    use x509_cert::ext::pkix::{BasicConstraints, ExtendedKeyUsage, KeyUsage};
    for (o, readable) in [
        (ext::KEY_USAGE, c.extension::<KeyUsage>().is_some()),
        (
            ext::EXT_KEY_USAGE,
            c.extension::<ExtendedKeyUsage>().is_some(),
        ),
        (
            ext::BASIC_CONSTRAINTS,
            c.extension::<BasicConstraints>().is_some(),
        ),
    ] {
        if !readable && raw_extension(c, o).is_some() {
            return Some(format!(
                "the certificate of {} has an extension LimarePDF can't read ({o})",
                name_of(c)
            ));
        }
    }
    None
}

/// Why `c` can't be a certificate authority, if it can't: it must say it is one (basic
/// constraints) and, if it limits its key's uses, allow signing certificates.
fn authority_problem(c: &Cert) -> Option<String> {
    use x509_cert::ext::pkix::{BasicConstraints, KeyUsage};
    if !c.extension::<BasicConstraints>().is_some_and(|b| b.ca) {
        return Some(format!(
            "the certificate of {} doesn't say it is a certificate authority",
            name_of(c)
        ));
    }
    if c.extension::<KeyUsage>()
        .is_some_and(|k| !k.key_cert_sign())
    {
        return Some(format!(
            "the certificate of {} doesn't allow its key to sign certificates",
            name_of(c)
        ));
    }
    None
}

/// Why the signer's certificate `c` can't be trusted for `purpose` at `at`, if it can't.
fn leaf_problem(c: &Cert, at: DateTime<Utc>, purpose: Purpose) -> Option<String> {
    use x509_cert::ext::pkix::ExtendedKeyUsage;
    if let Some(p) = cert_problem(c, at) {
        return Some(p);
    }
    let uses = c.extension::<ExtendedKeyUsage>().map(|e| e.0);
    match purpose {
        Purpose::Signing => {
            if !c.key_usage_allows_signing() {
                return Some("the signer's certificate doesn't allow its key to sign".into());
            }
            if uses.is_some_and(|u| !u.iter().any(|o| SIGNING_USES.contains(o))) {
                return Some(
                    "the signer's certificate is meant for other uses than signing documents"
                        .into(),
                );
            }
        }
        Purpose::Timestamping => {
            if !uses.is_some_and(|u| u.contains(&ext::TIME_STAMPING)) {
                return Some(
                    "the timestamp authority's certificate isn't meant for timestamps".into(),
                );
            }
        }
    }
    None
}

/// Why a digital ID with the certificate `c` can't sign, if it can't: the checks a signer's
/// certificate gets when its signatures are checked, apart from the dates (an expired ID
/// can still be kept and exported).
pub(crate) fn id_problem(c: &Cert) -> Option<String> {
    // The start of the validity period passes the date check.
    let at = c.info().not_before;
    leaf_problem(c, at, Purpose::Signing)
}

/// A name in the two forms names are compared in: as encoded, and as text ignoring case.
struct NameKey {
    der: Option<Vec<u8>>,
    folded: String,
}

impl NameKey {
    fn of(n: &x509_cert::name::Name) -> Self {
        use der::Encode;
        Self {
            der: n.to_der().ok(),
            folded: n.to_string().to_lowercase(),
        }
    }

    fn matches(&self, o: &NameKey) -> bool {
        (self.der.is_some() && self.der == o.der) || self.folded == o.folded
    }
}

fn same_name(a: &x509_cert::name::Name, b: &x509_cert::name::Name) -> bool {
    NameKey::of(a).matches(&NameKey::of(b))
}

fn self_issued(c: &Cert) -> bool {
    let t = c.cert.tbs_certificate();
    same_name(t.subject(), t.issuer())
}

/// Whether `issuer` could have issued `c`, given that its subject is `c`'s issuer: their
/// key identifiers don't contradict each other.
fn key_ids_agree(issuer: &Cert, c: &Cert) -> bool {
    use x509_cert::ext::pkix::AuthorityKeyIdentifier;
    let want = c
        .extension::<AuthorityKeyIdentifier>()
        .and_then(|a| a.key_identifier)
        .map(|k| k.as_bytes().to_vec());
    match (want, issuer.subject_key_id()) {
        (Some(w), Some(have)) => w == have,
        _ => true,
    }
}

/// The most policies one certificate may list, and the most policy-tree nodes kept at
/// one depth, so crafted certificates can't make policy processing run long.
const MAX_POLICIES: usize = 64;
const MAX_NODES: usize = 1024;

/// A node of RFC 5280's valid policy tree, at the depth being processed. Only the deepest
/// level is kept: with any policy acceptable to LimarePDF (the user-initial-policy-set is
/// anyPolicy), whether the tree ends up empty depends on that level alone.
#[derive(Clone, PartialEq, Eq)]
struct PolicyNode {
    valid: Oid,
    expected: Vec<Oid>,
    /// The node it hangs from, in the level above.
    parent: usize,
}

/// The value of the extension `o` in `c`, as written.
fn raw_extension(c: &Cert, o: Oid) -> Option<Vec<u8>> {
    let exts = c.cert.tbs_certificate().extensions()?;
    exts.iter()
        .find(|e| e.extn_id == o)
        .map(|e| e.extn_value.as_bytes().to_vec())
}

/// RFC 5280 certificate policy processing (6.1.2 to 6.1.5) for `path`: the certificates
/// from the one the trusted authority issued down to the signer. LimarePDF accepts any policy
/// and requires none itself, so this can only fail when a certificate requires an
/// explicit policy (policy constraints) and none runs through the whole path. The trusted
/// authority's own certificate isn't part of the path, as the RFC says.
fn policy_problem(path: &[&Cert]) -> Option<String> {
    use x509_cert::ext::pkix::{CertificatePolicies, PolicyConstraints, PolicyMappings};
    let any = Oid::new_unwrap("2.5.29.32.0");
    let n = path.len();
    let (mut explicit, mut inhibit_any, mut mapping) = (n + 1, n + 1, n + 1);
    // The deepest level of the tree; `None` once the tree is empty (NULL).
    let mut level: Option<Vec<PolicyNode>> = Some(vec![PolicyNode {
        valid: any,
        expected: vec![any],
        parent: 0,
    }]);
    let unreadable = |c: &Cert, what: &str| {
        Some(format!(
            "the certificate of {} has {what} LimarePDF can't read",
            name_of(c)
        ))
    };
    for (i, c) in path.iter().enumerate() {
        let i = i + 1;
        let last = i == n;
        let self_iss = self_issued(c);
        let has = |o: Oid| raw_extension(c, o).is_some();
        // (d) and (e): the policies this certificate lists.
        let policies: Option<Vec<Oid>> = if has(ext::POLICIES) {
            let Some(p) = c.extension::<CertificatePolicies>() else {
                return unreadable(c, "certificate policies");
            };
            if p.0.len() > MAX_POLICIES {
                return Some(format!(
                    "the certificate of {} lists too many policies",
                    name_of(c)
                ));
            }
            Some(p.0.iter().map(|p| p.policy_identifier).collect())
        } else {
            None
        };
        level = match (level.take(), policies) {
            (Some(prev), Some(listed)) => {
                let mut next: Vec<PolicyNode> = Vec::new();
                let push = |next: &mut Vec<PolicyNode>, node: PolicyNode| {
                    if next.len() < MAX_NODES && !next.contains(&node) {
                        next.push(node);
                    }
                };
                for &p in listed.iter().filter(|&&p| p != any) {
                    let matched: Vec<usize> = (0..prev.len())
                        .filter(|&k| prev[k].expected.contains(&p))
                        .collect();
                    let parents = if matched.is_empty() {
                        (0..prev.len()).filter(|&k| prev[k].valid == any).collect()
                    } else {
                        matched
                    };
                    for parent in parents {
                        push(
                            &mut next,
                            PolicyNode {
                                valid: p,
                                expected: vec![p],
                                parent,
                            },
                        );
                    }
                }
                if listed.contains(&any) && (inhibit_any > 0 || (!last && self_iss)) {
                    for (k, node) in prev.iter().enumerate() {
                        for &v in &node.expected {
                            let taken = next.iter().any(|c| c.parent == k && c.valid == v);
                            if !taken {
                                push(
                                    &mut next,
                                    PolicyNode {
                                        valid: v,
                                        expected: vec![v],
                                        parent: k,
                                    },
                                );
                            }
                        }
                    }
                }
                (!next.is_empty()).then_some(next)
            }
            _ => None,
        };
        // (f)
        if explicit == 0 && level.is_none() {
            return Some(policy_failure(c));
        }
        if last {
            break;
        }
        // 6.1.4: preparing for the next certificate.
        if has(ext::POLICY_MAPPINGS) {
            let Some(maps) = c.extension::<PolicyMappings>() else {
                return unreadable(c, "policy mappings");
            };
            let mut ids: Vec<Oid> = maps.0.iter().map(|m| m.issuer_domain_policy).collect();
            ids.dedup();
            for id in ids {
                let to: Vec<Oid> = maps
                    .0
                    .iter()
                    .filter(|m| m.issuer_domain_policy == id)
                    .map(|m| m.subject_domain_policy)
                    .collect();
                let Some(nodes) = level.as_mut() else { break };
                if mapping > 0 {
                    let mut found = false;
                    for node in nodes.iter_mut().filter(|n| n.valid == id) {
                        node.expected = to.clone();
                        found = true;
                    }
                    if !found {
                        if let Some(a) = nodes.iter().find(|n| n.valid == any).cloned() {
                            if nodes.len() < MAX_NODES {
                                nodes.push(PolicyNode {
                                    valid: id,
                                    expected: to,
                                    parent: a.parent,
                                });
                            }
                        }
                    }
                } else {
                    nodes.retain(|n| n.valid != id);
                    if nodes.is_empty() {
                        level = None;
                    }
                }
            }
        }
        // (h)
        if !self_iss {
            explicit = explicit.saturating_sub(1);
            mapping = mapping.saturating_sub(1);
            inhibit_any = inhibit_any.saturating_sub(1);
        }
        // (i)
        if has(ext::POLICY_CONSTRAINTS) {
            let Some(pc) = c.extension::<PolicyConstraints>() else {
                return unreadable(c, "policy constraints");
            };
            if let Some(r) = pc.require_explicit_policy {
                explicit = explicit.min(r as usize);
            }
            if let Some(m) = pc.inhibit_policy_mapping {
                mapping = mapping.min(m as usize);
            }
        }
        // (j)
        if let Some(v) = raw_extension(c, ext::INHIBIT_ANY_POLICY) {
            let skip = crate::asn1::Tlv::read_all(&v)
                .and_then(|t| t.expect(crate::asn1::INTEGER, "skip certificates"))
                .ok()
                .filter(|t| t.body.len() <= 4 && t.body.first().is_some_and(|b| b & 0x80 == 0))
                .map(|t| {
                    t.body
                        .iter()
                        .fold(0usize, |a, &b| (a << 8) | usize::from(b))
                });
            let Some(skip) = skip else {
                return unreadable(c, "an any-policy limit");
            };
            inhibit_any = inhibit_any.min(skip);
        }
    }
    // 6.1.5 (a), (b): the signer's own constraints.
    explicit = explicit.saturating_sub(1);
    let signer = path.last()?;
    if raw_extension(signer, ext::POLICY_CONSTRAINTS).is_some() {
        match signer.extension::<PolicyConstraints>() {
            Some(pc) if pc.require_explicit_policy == Some(0) => explicit = 0,
            Some(_) => {}
            None => return unreadable(signer, "policy constraints"),
        }
    }
    (explicit == 0 && level.is_none()).then(|| policy_failure(signer))
}

fn policy_failure(c: &Cert) -> String {
    format!(
        "the chain requires a certificate policy, and no policy runs through every \
         certificate down to {}",
        name_of(c)
    )
}

/// A self-issued certificate `c` with the name of the trusted certificate `label`.
fn lookalike_problem(c: &Cert, label: &str) -> String {
    format!(
        "{} has the same name as \"{label}\", which you trust, but is a different \
         certificate: someone else's, or a replacement you haven't trusted",
        name_of(c)
    )
}

struct Search<'a> {
    /// Every entry in the trust list, with its certificate.
    entries: &'a [(&'a TrustEntry, Cert)],
    /// The certificate authorities among them.
    anchors: &'a [(&'a TrustEntry, Cert)],
    /// The subjects of the anchors, then of the pool, in order.
    subjects: &'a [NameKey],
    pool: &'a [Cert],
    at: DateTime<Utc>,
    checks: usize,
    /// The first problem found on a way to a trusted authority.
    failed: Option<String>,
    /// Why the longest way found led nowhere trusted, and how long it was.
    unknown: Option<(usize, String)>,
}

impl Search<'_> {
    fn dead_end(&mut self, depth: usize, why: String) {
        if self.unknown.as_ref().is_none_or(|(d, _)| depth > *d) {
            self.unknown = Some((depth, why));
        }
    }

    /// Follows issuers up from the last certificate of `path`. `taint` is the first problem
    /// on the way so far: a way with one can't give trust, but reaching a trusted authority
    /// through it is reported. Returns the trusted authority's entry when a clean way
    /// reaches one.
    fn walk(&mut self, path: &mut Vec<Cert>, taint: Option<String>) -> Option<String> {
        let current = path.last().expect("a path holds the signer").clone();
        if path.len() >= MAX_CHAIN {
            self.dead_end(
                path.len(),
                "its chain of certificates is too long to follow".into(),
            );
            return None;
        }
        let (anchors, pool, subjects) = (self.anchors, self.pool, self.subjects);
        let wanted = NameKey::of(current.cert.tbs_certificate().issuer());
        // Anchors first, then the rest; `subjects` holds the names in that order.
        let candidates: Vec<(Option<&str>, &Cert)> = anchors
            .iter()
            .map(|(e, c)| (Some(e.label.as_str()), c))
            .chain(pool.iter().map(|c| (None, c)))
            .zip(subjects)
            .filter(|(_, name)| name.matches(&wanted))
            .map(|(candidate, _)| candidate)
            .filter(|(_, c)| key_ids_agree(c, &current))
            .filter(|(_, c)| !path.iter().any(|p| p.raw == c.raw))
            .collect();
        if candidates.is_empty() {
            let tbs = current.cert.tbs_certificate();
            let lookalike = |name: &x509_cert::name::Name| {
                self.entries
                    .iter()
                    .find(|(_, c)| {
                        c.raw != current.raw && same_name(c.cert.tbs_certificate().subject(), name)
                    })
                    .map(|(e, _)| e.label.clone())
            };
            if self_issued(&current) {
                if let Some(label) = lookalike(tbs.subject()) {
                    self.failed
                        .get_or_insert(lookalike_problem(&current, &label));
                }
            } else if let Some(label) = lookalike(tbs.issuer()) {
                self.dead_end(
                    path.len(),
                    format!(
                        "{} names \"{label}\", which you trust, as its issuer, but a different \
                         certificate of that name issued it",
                        name_of(&current)
                    ),
                );
                return None;
            }
            let why = if self_issued(&current) {
                if path.len() == 1 {
                    "it's a self-made digital ID you haven't trusted".into()
                } else {
                    format!(
                        "it was issued by {}, which you haven't trusted",
                        name_of(&current)
                    )
                }
            } else {
                let issuer = current
                    .info()
                    .issuer_common_name
                    .unwrap_or_else(|| current.info().issuer);
                format!(
                    "LimarePDF doesn't have the certificate of \"{issuer}\", which issued {}, and \
                     nothing you trust vouches for it",
                    name_of(&current)
                )
            };
            self.dead_end(path.len(), why);
            return None;
        }
        for (anchor, issuer) in candidates {
            if self.checks >= MAX_CHECKS {
                self.dead_end(
                    usize::MAX,
                    "it carries too many certificates to check".into(),
                );
                return None;
            }
            self.checks += 1;
            let digest = match cms_check::check_cert_signature(&current, issuer) {
                Ok(d) => d,
                Err(Fail::Broken(why) | Fail::CantCheck(why)) => {
                    // Not a link at all; only worth reporting when it names a trusted
                    // authority, as a forged certificate would.
                    if let Some(label) = &anchor {
                        self.failed.get_or_insert(if self_issued(&current) {
                            lookalike_problem(&current, label)
                        } else {
                            format!(
                                "{} names \"{label}\", which you trust, as its issuer, but {why}",
                                name_of(&current)
                            )
                        });
                    }
                    continue;
                }
            };
            let mut problem = taint.clone();
            if digest == DigestAlg::Sha1 {
                problem.get_or_insert(format!(
                    "the certificate of {} is signed with SHA-1, which is no longer safe \
                     against forgery",
                    name_of(&current)
                ));
            }
            if let Some(p) = authority_problem(issuer).or_else(|| cert_problem(issuer, self.at)) {
                problem.get_or_insert(p);
            }
            {
                use x509_cert::ext::pkix::BasicConstraints;
                // Certificate authorities between this issuer and the signer, not counting
                // self-issued ones (RFC 5280 4.2.1.9).
                let below = path[1..].iter().filter(|c| !self_issued(c)).count();
                if let Some(n) = issuer
                    .extension::<BasicConstraints>()
                    .and_then(|b| b.path_len_constraint)
                {
                    if below > usize::from(n) {
                        problem.get_or_insert(format!(
                            "{} allows at most {n} authorities below it, and this chain has {below}",
                            name_of(issuer)
                        ));
                    }
                }
            }
            if let Some(label) = anchor {
                let problem = problem.or_else(|| {
                    let from_anchor: Vec<&Cert> = path.iter().rev().collect();
                    policy_problem(&from_anchor)
                });
                match problem {
                    None => {
                        path.push(issuer.clone());
                        return Some(label.to_string());
                    }
                    Some(p) => {
                        self.failed.get_or_insert(format!(
                            "it was issued under \"{label}\", which you trust, but {p}"
                        ));
                        continue;
                    }
                }
            }
            path.push(issuer.clone());
            if let Some(found) = self.walk(path, problem) {
                return Some(found);
            }
            path.pop();
        }
        None
    }
}

/// Whether `signer` (with the other certificates in `pool` to build a chain from) is
/// trusted for `purpose` at `at`, the signing time.
pub(crate) fn evaluate(
    list: &TrustList,
    signer: &Cert,
    pool: &[Cert],
    at: DateTime<Utc>,
    purpose: Purpose,
) -> Trust {
    let leaf = leaf_problem(signer, at, purpose);
    let fp = signer.info().sha256;
    if let Some(e) = list.entries.iter().find(|e| e.cert.sha256 == fp) {
        return match leaf {
            None => Trust::Signer {
                label: e.label.clone(),
            },
            Some(p) => Trust::Failed(format!("you trust \"{}\", but {p}", e.label)),
        };
    }
    let entries: Vec<(&TrustEntry, Cert)> = list
        .entries
        .iter()
        .filter_map(|e| Some((e, Cert::parse(&e.cert.der).ok()?)))
        .collect();
    let anchors: Vec<(&TrustEntry, Cert)> = entries
        .iter()
        .filter(|(e, _)| e.role == TrustRole::Authority)
        .cloned()
        .collect();
    // The pool without repeats or anchors (an anchor is used as the trusted entry).
    let mut unique: Vec<Cert> = Vec::new();
    for c in pool {
        let known = |u: &Cert| u.raw == c.raw;
        if !unique.iter().any(known) && !anchors.iter().any(|(_, a)| known(a)) {
            unique.push(c.clone());
        }
    }
    let subjects: Vec<NameKey> = anchors
        .iter()
        .map(|(_, c)| c)
        .chain(&unique)
        .map(|c| NameKey::of(c.cert.tbs_certificate().subject()))
        .collect();
    let mut s = Search {
        entries: &entries,
        anchors: &anchors,
        subjects: &subjects,
        pool: &unique,
        at,
        checks: 0,
        failed: None,
        unknown: None,
    };
    let mut path = vec![signer.clone()];
    match s.walk(&mut path, leaf) {
        Some(label) => Trust::Authority {
            label,
            chain: path.iter().map(Cert::info).collect(),
        },
        None => match s.failed {
            Some(why) => Trust::Failed(why),
            None => Trust::Unknown(
                s.unknown
                    .map(|(_, why)| why)
                    .unwrap_or_else(|| "nothing you trust vouches for it".into()),
            ),
        },
    }
}
