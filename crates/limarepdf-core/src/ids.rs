//! Digital IDs: a private key with its certificate, what a digital signature is made with.
//! LimarePDF makes self-signed IDs like Acrobat's own (RSA 3072-bit, SHA-256, valid five years)
//! and imports IDs from `.p12` / `.pfx` files made by Acrobat, Windows, macOS or a
//! certificate authority. Each ID is a PKCS#12 file in the `ids` folder of LimarePDF's settings,
//! protected by its password, which LimarePDF never stores. See docs/signatures.md, "Digital IDs".

use crate::cms_check::{self, Cert, CertInfo, DigestAlg};
use crate::pkcs12;
use crate::Result;
use anyhow::{anyhow, bail, Context};
use der::asn1::ObjectIdentifier as Oid;
use getrandom::rand_core::UnwrapErr;
use getrandom::SysRng;
use std::fmt;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub use crate::pkcs12::WrongPassword;

mod oid {
    use der::asn1::ObjectIdentifier as Oid;

    pub const RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.1");
    pub const RSA_PSS: Oid = Oid::new_unwrap("1.2.840.113549.1.1.10");
    pub const EC_PUBLIC_KEY: Oid = Oid::new_unwrap("1.2.840.10045.2.1");
    pub const P256: Oid = Oid::new_unwrap("1.2.840.10045.3.1.7");
    pub const P384: Oid = Oid::new_unwrap("1.3.132.0.34");
    pub const COMMON_NAME: Oid = Oid::new_unwrap("2.5.4.3");
    pub const ORGANISATION: Oid = Oid::new_unwrap("2.5.4.10");
    pub const EMAIL: Oid = Oid::new_unwrap("1.2.840.113549.1.9.1");
    pub const EMAIL_PROTECTION: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.3.4");
    /// Adobe Authentic Documents Trust, which Acrobat puts in its self-signed IDs.
    pub const ADOBE_AUTHENTIC_DOCUMENTS: Oid = Oid::new_unwrap("1.2.840.113583.1.1.5");
}

/// The key size of the IDs LimarePDF makes.
const RSA_BITS: usize = 3072;
/// The RSA key sizes LimarePDF signs with: below 2048 isn't safe, above 4096 Acrobat can't check.
const RSA_RANGE: std::ops::RangeInclusive<usize> = 2048..=4096;
/// The shortest password LimarePDF sets on an ID.
pub const MIN_PASSWORD: usize = 8;
/// The longest password LimarePDF accepts, in bytes.
const MAX_PASSWORD: usize = 1024;
/// The longest name or organisation (RFC 5280's limit for both).
const MAX_NAME: usize = 64;
/// The longest email address (RFC 5280's limit).
const MAX_EMAIL: usize = 255;
/// The most certificates kept with an ID besides its own.
const MAX_CHAIN: usize = 16;
/// The largest ID file read.
const MAX_FILE: u64 = 4 << 20;
/// What a key's signature is checked over to show it belongs to its certificate.
const KEY_CHECK: &[u8] = b"LimarePDF checks that this digital ID's key belongs to its certificate";

/// What a new self-signed ID says about its owner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewId {
    /// The owner's name, as signatures will show it.
    pub name: String,
    pub organisation: Option<String>,
    pub email: Option<String>,
}

impl NewId {
    /// Fails when the name, organisation or email address can't go in a certificate.
    pub fn check(&self) -> Result<()> {
        check_new(self).map(|_| ())
    }
}

/// A digital ID in the ID folder, as listed without its password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigitalId {
    /// How commands refer to it: the file's name without `.p12`.
    pub name: String,
    pub path: PathBuf,
    /// The ID's certificate: whose it is and how long it is valid.
    pub cert: CertInfo,
    /// Other certificates kept with it: the authorities above an ID a certificate
    /// authority issued.
    pub chain: Vec<CertInfo>,
    /// The one signing uses unless told otherwise: the last one created, imported or used.
    pub default: bool,
}

impl DigitalId {
    /// Whether the certificate is valid today, by this computer's clock. LimarePDF doesn't sign
    /// with an ID outside its validity period.
    pub fn valid_now(&self) -> bool {
        self.cert.valid_now()
    }
}

/// The IDs in the folder, and why files there aren't listed.
#[derive(Clone, Debug, Default)]
pub struct IdList {
    pub ids: Vec<DigitalId>,
    pub problems: Vec<String>,
}

/// A digital ID opened with its password: its key is in memory, wiped when dropped.
pub struct UnlockedId {
    pub cert: CertInfo,
    pub chain: Vec<CertInfo>,
    key: Key,
}

impl fmt::Debug for UnlockedId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("UnlockedId")
            .field("cert", &self.cert.subject)
            .field("key", &self.key.describe())
            .finish_non_exhaustive()
    }
}

impl UnlockedId {
    /// The ID's file with its key and certificates, protected by `password`, for other
    /// apps (Acrobat, Windows, macOS). The certificates are encrypted too, as other apps
    /// do it.
    pub fn to_p12(&self, password: &str) -> Result<Vec<u8>> {
        if let Some(why) = password_problem(password) {
            bail!("{why}");
        }
        let name = self.cert.display_name().to_string();
        let chain: Vec<Vec<u8>> = self.chain.iter().map(|c| c.der.clone()).collect();
        let key = self.key.to_pkcs8()?;
        let file = pkcs12::write(&key, &self.cert.der, &chain, &name, password, true)?;
        // Read back what was written before handing it out.
        let back = read_id(&file, password)?;
        if back.cert.der != self.cert.der || back.key.to_pkcs8()? != key {
            bail!("the exported file didn't read back the same; nothing was written");
        }
        Ok(file)
    }

    /// The key, such as "RSA 3072-bit" or "ECDSA P-256".
    pub fn key_description(&self) -> String {
        self.key.describe()
    }

    /// The digest the key signs with: SHA-384 for P-384, SHA-256 otherwise.
    pub(crate) fn digest(&self) -> DigestAlg {
        self.key.digest()
    }

    pub(crate) fn is_rsa(&self) -> bool {
        matches!(self.key, Key::Rsa(_))
    }

    /// A signature over `data` (hashed with [`UnlockedId::digest`]): PKCS#1 v1.5 for RSA,
    /// DER-encoded ECDSA otherwise.
    pub(crate) fn sign(&self, data: &[u8]) -> Result<Vec<u8>> {
        self.key.sign(data)
    }
}

/// Why `password` can't protect an ID, if it can't.
pub fn password_problem(password: &str) -> Option<String> {
    if password.chars().count() < MIN_PASSWORD {
        Some(format!(
            "the password must be at least {MIN_PASSWORD} characters long"
        ))
    } else if password.len() > MAX_PASSWORD {
        Some("the password is too long".into())
    } else {
        None
    }
}

// ------------------------------------------------------------------ keys

/// A private key LimarePDF can sign with.
enum Key {
    Rsa(Box<rsa::RsaPrivateKey>),
    P256(p256::ecdsa::SigningKey),
    P384(p384::ecdsa::SigningKey),
}

fn rng() -> UnwrapErr<SysRng> {
    UnwrapErr(SysRng)
}

impl Key {
    /// Reads a PKCS#8 private key.
    fn from_pkcs8(der: &[u8]) -> Result<Self> {
        use crate::asn1::{self, Tlv};
        let unreadable = || anyhow!("the ID's private key can't be read");
        let info = Tlv::read_all(der).map_err(|_| unreadable())?;
        let kids = info
            .expect(asn1::SEQUENCE, "key")
            .and_then(|t| t.children())
            .map_err(|_| unreadable())?;
        let alg = kids
            .get(1)
            .and_then(|a| a.children().ok())
            .ok_or_else(unreadable)?;
        let alg_oid = alg
            .first()
            .and_then(|o| o.oid().ok())
            .ok_or_else(unreadable)?;
        match alg_oid {
            oid::RSA => {
                use rsa::pkcs8::DecodePrivateKey;
                use rsa::traits::PublicKeyParts;
                let k = rsa::RsaPrivateKey::from_pkcs8_der(der).map_err(|_| unreadable())?;
                k.validate()
                    .map_err(|_| anyhow!("the ID's RSA key is damaged"))?;
                let bits = k.n().bits_vartime() as usize;
                if !RSA_RANGE.contains(&bits) {
                    bail!(
                        "the ID has a {bits}-bit RSA key; LimarePDF signs with RSA keys of {} to {} \
                         bits (shorter ones aren't safe, longer ones Acrobat can't check)",
                        RSA_RANGE.start(),
                        RSA_RANGE.end()
                    );
                }
                Ok(Self::Rsa(Box::new(k)))
            }
            oid::RSA_PSS => bail!(
                "the ID's key allows only RSA-PSS signatures, which LimarePDF can't make yet"
            ),
            oid::EC_PUBLIC_KEY => {
                use p256::pkcs8::DecodePrivateKey;
                match alg.get(1).and_then(|c| c.oid().ok()) {
                    Some(oid::P256) => p256::ecdsa::SigningKey::from_pkcs8_der(der)
                        .map(Self::P256)
                        .map_err(|_| unreadable()),
                    Some(oid::P384) => p384::ecdsa::SigningKey::from_pkcs8_der(der)
                        .map(Self::P384)
                        .map_err(|_| unreadable()),
                    _ => bail!(
                        "the ID's key uses an elliptic curve LimarePDF can't sign with (it signs with \
                         RSA, P-256 and P-384 keys)"
                    ),
                }
            }
            o => bail!(
                "the ID's key is of a kind LimarePDF can't sign with ({o}); it signs with RSA, P-256 \
                 and P-384 keys"
            ),
        }
    }

    /// The key as PKCS#8, wiped when dropped.
    fn to_pkcs8(&self) -> Result<Zeroizing<Vec<u8>>> {
        use rsa::pkcs8::EncodePrivateKey;
        let doc = match self {
            Self::Rsa(k) => k.to_pkcs8_der(),
            Self::P256(k) => k.to_pkcs8_der(),
            Self::P384(k) => k.to_pkcs8_der(),
        }
        .map_err(|e| anyhow!("encoding the key failed: {e}"))?;
        Ok(Zeroizing::new(doc.as_bytes().to_vec()))
    }

    fn describe(&self) -> String {
        match self {
            Self::Rsa(k) => {
                use rsa::traits::PublicKeyParts;
                format!("RSA {}-bit", k.n().bits_vartime())
            }
            Self::P256(_) => "ECDSA P-256".into(),
            Self::P384(_) => "ECDSA P-384".into(),
        }
    }

    fn digest(&self) -> DigestAlg {
        match self {
            Self::P384(_) => DigestAlg::Sha384,
            Self::Rsa(_) | Self::P256(_) => DigestAlg::Sha256,
        }
    }

    fn sign(&self, data: &[u8]) -> Result<Vec<u8>> {
        use p256::ecdsa::signature::Signer;
        Ok(match self {
            Self::Rsa(k) => {
                // Blinded, so the signing time doesn't depend on the key.
                let digest = DigestAlg::Sha256.hash(&[data]);
                k.sign_with_rng(
                    &mut rng(),
                    rsa::Pkcs1v15Sign::new::<sha2::Sha256>(),
                    &digest,
                )
                .map_err(|e| anyhow!("signing failed: {e}"))?
            }
            Self::P256(k) => {
                let s: p256::ecdsa::DerSignature = k
                    .try_sign(data)
                    .map_err(|e| anyhow!("signing failed: {e}"))?;
                s.as_bytes().to_vec()
            }
            Self::P384(k) => {
                let s: p384::ecdsa::DerSignature = k
                    .try_sign(data)
                    .map_err(|e| anyhow!("signing failed: {e}"))?;
                s.as_bytes().to_vec()
            }
        })
    }

    /// Whether `cert` is this key's certificate: a signature made with the key checks
    /// against it.
    fn belongs_to(&self, cert: &Cert) -> Result<bool> {
        let sig = self.sign(KEY_CHECK)?;
        Ok(cms_check::signed_by(cert, self.digest(), KEY_CHECK, &sig))
    }

    fn public_key_info(&self) -> Result<spki::SubjectPublicKeyInfoOwned> {
        let r = match self {
            Self::Rsa(k) => spki::SubjectPublicKeyInfoOwned::from_key(&k.to_public_key()),
            Self::P256(k) => spki::SubjectPublicKeyInfoOwned::from_key(k.verifying_key()),
            Self::P384(k) => spki::SubjectPublicKeyInfoOwned::from_key(k.verifying_key()),
        };
        r.map_err(|e| anyhow!("encoding the public key failed: {e}"))
    }
}

// ------------------------------------------------------------------ making IDs

/// Checks what a new ID says: a name, and optionally an organisation and email address.
fn check_new(new: &NewId) -> Result<(String, Option<String>, Option<String>)> {
    let text = |s: &str, what: &str| -> Result<String> {
        let s = s.trim();
        if s.chars().any(char::is_control) {
            bail!("the {what} can't hold line breaks or other control characters");
        }
        if s.chars().count() > MAX_NAME {
            bail!("the {what} can be at most {MAX_NAME} characters long");
        }
        Ok(s.to_string())
    };
    let name = text(&new.name, "name")?;
    if name.is_empty() {
        bail!("a digital ID needs a name");
    }
    let organisation = match new.organisation.as_deref().map(|o| text(o, "organisation")) {
        Some(Ok(o)) if o.is_empty() => None,
        Some(r) => Some(r?),
        None => None,
    };
    let email = match new.email.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(e) => {
            let ok = e.is_ascii()
                && e.len() <= MAX_EMAIL
                && !e
                    .chars()
                    .any(|c| c.is_ascii_whitespace() || c.is_ascii_control())
                && matches!(e.split_once('@'), Some((l, d)) if !l.is_empty() && !d.is_empty() && !d.contains('@'));
            if !ok {
                bail!("\"{e}\" isn't an email address LimarePDF can put in a certificate");
            }
            Some(e.to_string())
        }
    };
    Ok((name, organisation, email))
}

/// The subject name of a new ID: emailAddress and O when given, then CN, so that it reads
/// `CN=…,O=…,EMAIL=…` in the usual text form (which lists the parts last first). Built from
/// its parts, so commas or equals signs in a name stay part of the name.
fn subject_name(
    name: &str,
    org: Option<&str>,
    email: Option<&str>,
) -> Result<x509_cert::name::Name> {
    use crate::asn1::{der, der_oid, der_seq, der_set};
    use der::Decode;
    let rdn =
        |o: &Oid, tag: u8, v: &str| der_set(vec![der_seq(&[&der_oid(o), &der(tag, v.as_bytes())])]);
    // UTF8String for names; emailAddress is an IA5String.
    let mut rdns = Vec::new();
    if let Some(e) = email {
        rdns.push(rdn(&oid::EMAIL, 0x16, e));
    }
    if let Some(o) = org {
        rdns.push(rdn(&oid::ORGANISATION, 0x0c, o));
    }
    rdns.push(rdn(&oid::COMMON_NAME, 0x0c, name));
    let encoded = crate::asn1::der(crate::asn1::SEQUENCE, &rdns.concat());
    x509_cert::name::Name::from_der(&encoded).map_err(|e| anyhow!("the name can't be encoded: {e}"))
}

/// The extensions of a self-made ID, as Acrobat's: the key's identifier, key usage for
/// signing (critical), extended key usage for email protection and Adobe's authentic
/// documents, and the email address as an alternative name when given.
struct SelfSigned {
    subject: x509_cert::name::Name,
    email: Option<String>,
}

impl x509_cert::builder::profile::BuilderProfile for SelfSigned {
    fn get_issuer(&self, subject: &x509_cert::name::Name) -> x509_cert::name::Name {
        subject.clone()
    }

    fn get_subject(&self) -> x509_cert::name::Name {
        self.subject.clone()
    }

    fn build_extensions(
        &self,
        spk: spki::SubjectPublicKeyInfoRef<'_>,
        _issuer_spk: spki::SubjectPublicKeyInfoRef<'_>,
        tbs: &x509_cert::certificate::TbsCertificate,
    ) -> x509_cert::builder::Result<Vec<x509_cert::ext::Extension>> {
        use x509_cert::ext::pkix::name::GeneralName;
        use x509_cert::ext::pkix::{
            ExtendedKeyUsage, KeyUsage, KeyUsages, SubjectAltName, SubjectKeyIdentifier,
        };
        use x509_cert::ext::ToExtension;
        let subject = tbs.subject();
        let mut exts = Vec::new();
        let ski = SubjectKeyIdentifier::try_from(spk)?;
        exts.push(ski.to_extension(subject, &exts)?);
        let ku = KeyUsage(KeyUsages::DigitalSignature | KeyUsages::NonRepudiation);
        exts.push(ku.to_extension(subject, &exts)?);
        let eku = ExtendedKeyUsage(vec![oid::EMAIL_PROTECTION, oid::ADOBE_AUTHENTIC_DOCUMENTS]);
        exts.push(eku.to_extension(subject, &exts)?);
        if let Some(e) = &self.email {
            let san = SubjectAltName(vec![GeneralName::Rfc822Name(der::asn1::Ia5String::new(e)?)]);
            exts.push(san.to_extension(subject, &exts)?);
        }
        Ok(exts)
    }
}

/// A self-signed certificate for `key`, valid from now for five years.
fn self_signed(key: &Key, new: &NewId) -> Result<Vec<u8>> {
    use der::Encode;
    use x509_cert::builder::{Builder, CertificateBuilder};
    let (name, org, email) = check_new(new)?;
    let subject = subject_name(&name, org.as_deref(), email.as_deref())?;
    let now = chrono::Utc::now();
    let then = now
        .checked_add_months(chrono::Months::new(60))
        .ok_or_else(|| anyhow!("the date is out of range"))?;
    let years = (then - now)
        .to_std()
        .map_err(|_| anyhow!("the date is out of range"))?;
    let validity = x509_cert::time::Validity::from_now(years)
        .map_err(|e| anyhow!("the validity period can't be encoded: {e}"))?;
    let serial = x509_cert::serial_number::SerialNumber::generate(&mut rng());
    let profile = SelfSigned { subject, email };
    let builder = CertificateBuilder::new(profile, serial, validity, key.public_key_info()?)
        .map_err(|e| anyhow!("making the certificate failed: {e}"))?;
    let cert = match key {
        Key::Rsa(k) => {
            let signer = rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new((**k).clone());
            builder.build::<_, rsa::pkcs1v15::Signature>(&signer)
        }
        Key::P256(k) => builder.build::<_, p256::ecdsa::DerSignature>(k),
        Key::P384(k) => builder.build::<_, p384::ecdsa::DerSignature>(k),
    }
    .map_err(|e| anyhow!("making the certificate failed: {e}"))?;
    cert.to_der()
        .map_err(|e| anyhow!("encoding the certificate failed: {e}"))
}

// ------------------------------------------------------------------ reading IDs

/// An ID file's contents, opened with its password.
struct Opened {
    key: Key,
    cert: CertInfo,
    chain: Vec<CertInfo>,
    friendly_name: Option<String>,
}

/// Opens a PKCS#12 file: exactly one private key, its certificate (found by checking a
/// signature, not by labels), and the other certificates as its chain.
fn read_id(file: &[u8], password: &str) -> Result<Opened> {
    let c = pkcs12::read(file, password)?;
    let key = match c.keys.len() {
        0 => bail!(
            "the file holds no private key, so it can't sign; a certificate on its own can be \
             trusted with `limarepdf-cli trust add` instead"
        ),
        1 => Key::from_pkcs8(&c.keys[0])?,
        n => bail!("the file holds {n} private keys; LimarePDF takes one digital ID per file"),
    };
    let mut certs: Vec<Cert> = Vec::new();
    for raw in &c.certs {
        let cert = Cert::parse(raw).context("a certificate in the file can't be read")?;
        if !certs.iter().any(|k| k.raw == cert.raw) {
            certs.push(cert);
        }
    }
    let mut own = None;
    for (i, cert) in certs.iter().enumerate() {
        if key.belongs_to(cert)? {
            own = Some(i);
            break;
        }
    }
    let own = own.ok_or_else(|| {
        anyhow!("the file has no certificate for its private key, so it can't be used to sign")
    })?;
    let cert = certs.remove(own);
    if certs.len() > MAX_CHAIN {
        bail!("the file holds too many certificates ({})", certs.len() + 1);
    }
    Ok(Opened {
        key,
        cert: cert.info(),
        chain: certs.iter().map(Cert::info).collect(),
        friendly_name: c.friendly_name,
    })
}

/// Why LimarePDF can't sign with the certificate `c`, if it can't. LimarePDF signs RSA with
/// PKCS#1 v1.5, which a key limited to RSA-PSS mustn't make.
fn signing_problem(c: &CertInfo) -> Option<String> {
    match Cert::parse(&c.der) {
        Ok(cert) if cert.rsa_pss_only() => Some(
            "its certificate allows only RSA-PSS signatures with its key, which LimarePDF \
             can't make yet"
                .into(),
        ),
        Ok(cert) => crate::trust::id_problem(&cert),
        Err(e) => Some(format!("{e:#}")),
    }
}

// ------------------------------------------------------------------ the ID folder

/// The folder of digital IDs: one PKCS#12 file per ID, private to this user on Unix.
#[derive(Clone, Debug)]
pub struct IdStore {
    dir: PathBuf,
}

/// The file naming the default ID.
const DEFAULT_FILE: &str = "default.txt";

impl IdStore {
    /// The `ids` folder in LimarePDF's settings folder; `None` when there is no settings folder.
    pub fn default_dir() -> Option<PathBuf> {
        crate::settings_dir().map(|d| d.join("ids"))
    }

    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The IDs in the folder, by name. A missing folder has none.
    pub fn list(&self) -> IdList {
        let mut list = IdList::default();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return list,
            Err(e) => {
                list.problems
                    .push(format!("{} can't be read: {e}", self.dir.display()));
                return list;
            }
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("p12")))
            .collect();
        paths.sort();
        let default = self.default_name();
        for path in paths {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            match self.peek(&path) {
                Ok((cert, chain)) => {
                    #[cfg(unix)]
                    if let Ok(m) = std::fs::metadata(&path) {
                        use std::os::unix::fs::PermissionsExt;
                        if m.permissions().mode() & 0o077 != 0 {
                            list.problems.push(format!(
                                "{} can be read by other users of this computer; make it \
                                 private (chmod 600)",
                                path.display()
                            ));
                        }
                    }
                    list.ids.push(DigitalId {
                        default: default.as_deref() == Some(name.as_str()),
                        name,
                        path,
                        cert,
                        chain,
                    });
                }
                Err(e) => list
                    .problems
                    .push(format!("{} isn't listed: {e:#}", path.display())),
            }
        }
        list.ids.sort_by(|a, b| a.name.cmp(&b.name));
        if list.ids.len() == 1 {
            list.ids[0].default = true;
        }
        list
    }

    /// The certificates of an ID file, read without its password.
    fn peek(&self, path: &Path) -> Result<(CertInfo, Vec<CertInfo>)> {
        let bytes = read_file(path)?;
        let (certs, _) = pkcs12::peek(&bytes).map_err(|_| {
            anyhow!("it isn't a digital ID file LimarePDF wrote; add IDs with `limarepdf-cli id import`")
        })?;
        let mut certs = certs.iter().map(|c| Cert::parse(c).map(|c| c.info()));
        let cert = certs.next().ok_or_else(|| {
            anyhow!("it isn't a digital ID file LimarePDF wrote; add IDs with `limarepdf-cli id import`")
        })??;
        Ok((cert, certs.collect::<Result<_>>()?))
    }

    fn default_name(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.dir.join(DEFAULT_FILE)).ok()?;
        let name = text.trim();
        (!name.is_empty()).then(|| name.to_string())
    }

    /// Makes `id` the one signing uses unless told otherwise.
    pub fn set_default(&self, id: &DigitalId) -> Result<()> {
        crate::write_private(
            &self.dir.join(DEFAULT_FILE),
            format!("{}\n", id.name).as_bytes(),
        )
    }

    /// The ID `which` names: its number in [`IdStore::list`] (from 1), its name, the name
    /// in its certificate (any case, when only one ID has it) or its SHA-256 fingerprint.
    pub fn find(&self, which: &str) -> Result<DigitalId> {
        let list = self.list();
        let which = which.trim();
        if list.ids.is_empty() {
            bail!("there are no digital IDs yet; make one with `limarepdf-cli id create`");
        }
        if let Ok(n) = which.parse::<usize>() {
            return n
                .checked_sub(1)
                .and_then(|i| list.ids.get(i))
                .cloned()
                .ok_or_else(|| {
                    anyhow!("there is no digital ID {n}; there are {}", list.ids.len())
                });
        }
        if let Some(id) = list.ids.iter().find(|i| i.name.eq_ignore_ascii_case(which)) {
            return Ok(id.clone());
        }
        let lower = which.to_lowercase();
        let by_cert: Vec<&DigitalId> = list
            .ids
            .iter()
            .filter(|i| i.cert.display_name().to_lowercase() == lower)
            .collect();
        match by_cert[..] {
            [one] => return Ok(one.clone()),
            [_, _, ..] => bail!(
                "several digital IDs are called \"{which}\"; choose one by its number or name \
                 in `limarepdf-cli id list`"
            ),
            [] => {}
        }
        let hex = |s: &str| -> String {
            s.chars()
                .filter(|c| c.is_ascii_hexdigit())
                .collect::<String>()
                .to_ascii_uppercase()
        };
        let want = hex(which);
        if want.len() == 64 {
            if let Some(id) = list.ids.iter().find(|i| hex(&i.cert.sha256) == want) {
                return Ok(id.clone());
            }
        }
        bail!("there is no digital ID \"{which}\"; `limarepdf-cli id list` lists them")
    }

    /// The default ID: the one marked, or the only one.
    pub fn default_id(&self) -> Option<DigitalId> {
        self.list().ids.into_iter().find(|i| i.default)
    }

    /// Makes a new self-signed ID (RSA 3072-bit, valid five years), protected by
    /// `password`, and makes it the default. Takes a few seconds: run it off the UI
    /// thread.
    pub fn create(&self, new: &NewId, password: &str) -> Result<DigitalId> {
        if let Some(why) = password_problem(password) {
            bail!("{why}");
        }
        check_new(new)?;
        let key = Key::Rsa(Box::new(
            rsa::RsaPrivateKey::new(&mut rng(), RSA_BITS)
                .map_err(|e| anyhow!("making the key failed: {e}"))?,
        ));
        let cert = Cert::parse(&self_signed(&key, new)?)?;
        if !key.belongs_to(&cert)? {
            bail!("the new certificate doesn't check against its key; nothing was saved");
        }
        let info = cert.info();
        // LimarePDF's own checks must accept the IDs it makes.
        if let Some(why) = signing_problem(&info) {
            bail!("the new certificate isn't usable for signing ({why}); nothing was saved");
        }
        let name = info.display_name().to_string();
        self.store(&key, &info, &[], &name, &name, password)
    }

    /// Imports the ID in a `.p12` / `.pfx` file opened with `password`, keeping it under
    /// the same password with LimarePDF's own encryption, named `name` (by default the name in
    /// its certificate), and makes it the default. Refused when LimarePDF couldn't sign with it;
    /// an expired ID is imported (and can be exported), but LimarePDF won't sign with it.
    pub fn import(&self, file: &[u8], password: &str, name: Option<&str>) -> Result<DigitalId> {
        if password.len() > MAX_PASSWORD {
            bail!("the password is too long");
        }
        let opened = read_id(file, password)?;
        if let Some(why) = signing_problem(&opened.cert) {
            bail!("LimarePDF can't sign with this ID: {why}");
        }
        if let Some(have) = self
            .list()
            .ids
            .into_iter()
            .find(|i| i.cert.sha256 == opened.cert.sha256)
        {
            bail!(
                "this digital ID is already in LimarePDF as \"{}\" ({})",
                have.cert.display_name(),
                have.name
            );
        }
        let label = opened
            .friendly_name
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| opened.cert.display_name().to_string());
        let base = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(opened.cert.display_name())
            .to_string();
        let chain: Vec<CertInfo> = opened.chain.clone();
        self.store(&opened.key, &opened.cert, &chain, &base, &label, password)
    }

    /// Writes a new ID file named after `base`, checks it reads back, and makes it the
    /// default.
    fn store(
        &self,
        key: &Key,
        cert: &CertInfo,
        chain: &[CertInfo],
        base: &str,
        label: &str,
        password: &str,
    ) -> Result<DigitalId> {
        let chain_der: Vec<Vec<u8>> = chain.iter().map(|c| c.der.clone()).collect();
        let pkcs8 = key.to_pkcs8()?;
        let file = pkcs12::write(&pkcs8, &cert.der, &chain_der, label, password, false)?;
        // Never keep a file that doesn't open again with the same password.
        let back = read_id(&file, password)?;
        if back.cert.der != cert.der || back.key.to_pkcs8()? != pkcs8 {
            bail!("the ID file didn't read back the same; nothing was saved");
        }
        let name = self.free_name(base);
        let path = self.dir.join(format!("{name}.p12"));
        crate::write_private(&path, &file)?;
        let id = DigitalId {
            name,
            path,
            cert: cert.clone(),
            chain: chain.to_vec(),
            default: true,
        };
        self.set_default(&id)?;
        Ok(id)
    }

    /// A file name for a new ID from `base` that no ID uses yet.
    fn free_name(&self, base: &str) -> String {
        let stem = file_stem(base);
        let taken = |n: &str| self.dir.join(format!("{n}.p12")).exists();
        if !taken(&stem) {
            return stem;
        }
        (2..)
            .map(|i| format!("{stem}-{i}"))
            .find(|n| !taken(n))
            .expect("some number is free")
    }

    /// Opens `id` with its password, for signing or exporting. Refused, like an import,
    /// when LimarePDF couldn't sign with it (a file put in the folder by hand isn't checked
    /// otherwise).
    pub fn unlock(&self, id: &DigitalId, password: &str) -> Result<UnlockedId> {
        let bytes = read_file(&id.path)?;
        let opened = read_id(&bytes, password)?;
        if let Some(why) = signing_problem(&opened.cert) {
            bail!("LimarePDF can't sign with this ID: {why}");
        }
        Ok(UnlockedId {
            cert: opened.cert,
            chain: opened.chain,
            key: opened.key,
        })
    }

    /// Deletes `id`'s file. This can't be undone: without a copy (exported `.p12`), the ID
    /// can't sign again.
    pub fn remove(&self, id: &DigitalId) -> Result<()> {
        std::fs::remove_file(&id.path)
            .with_context(|| format!("removing {}", id.path.display()))?;
        if self.default_name().as_deref() == Some(id.name.as_str()) {
            let _ = std::fs::remove_file(self.dir.join(DEFAULT_FILE));
        }
        Ok(())
    }
}

fn read_file(path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if meta.len() > MAX_FILE {
        bail!("{} is too large to be a digital ID", path.display());
    }
    std::fs::read(path).with_context(|| format!("reading {}", path.display()))
}

/// A file name from an ID's name: letters and digits in lower case, other runs of
/// characters as one `-`, at most 40 characters; never empty, all digits (those would read
/// as list numbers) or a name Windows reserves.
fn file_stem(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
        } else if !s.is_empty() && !s.ends_with('-') {
            s.push('-');
        }
        if s.chars().count() >= 40 {
            break;
        }
    }
    let s = s.trim_end_matches('-').to_string();
    const RESERVED: [&str; 22] = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    if s.is_empty() {
        "id".into()
    } else if s.chars().all(|c| c.is_ascii_digit()) || RESERVED.contains(&s.as_str()) {
        format!("id-{s}")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stems() {
        assert_eq!(file_stem("Jane Doe"), "jane-doe");
        assert_eq!(file_stem("  Dr. José Álvarez, PhD "), "dr-josé-álvarez-phd");
        assert_eq!(file_stem("../../etc/passwd"), "etc-passwd");
        assert_eq!(file_stem("C:\\Windows"), "c-windows");
        assert_eq!(file_stem("???"), "id");
        assert_eq!(file_stem("2024"), "id-2024");
        assert_eq!(file_stem("CON"), "id-con");
        assert_eq!(file_stem(&"a".repeat(100)).len(), 40);
    }

    #[test]
    fn new_id_checks() {
        let ok = |n: &str, o: Option<&str>, e: Option<&str>| {
            check_new(&NewId {
                name: n.into(),
                organisation: o.map(Into::into),
                email: e.map(Into::into),
            })
        };
        assert_eq!(
            ok(" Jane Doe ", Some(""), Some(" jane@example.com ")).unwrap(),
            ("Jane Doe".into(), None, Some("jane@example.com".into()))
        );
        assert!(ok("", None, None).is_err());
        assert!(ok("Jane\nDoe", None, None).is_err());
        assert!(ok(&"x".repeat(65), None, None).is_err());
        assert!(ok("Jane", Some(&"x".repeat(65)), None).is_err());
        for bad in [
            "jane",
            "jane@",
            "@example.com",
            "jane doe@example.com",
            "jäne@example.com",
            "a@b@c",
        ] {
            assert!(ok("Jane", None, Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn subject_keeps_special_characters_in_the_name() {
        let n = subject_name("Doe, Jane=CN=Evil", Some("Ex+ample"), Some("j@example.com")).unwrap();
        let rdns: Vec<_> = n.iter_rdn().collect();
        assert_eq!(rdns.len(), 3);
        let cn = n.iter().last().unwrap();
        assert_eq!(cn.oid, oid::COMMON_NAME);
        assert_eq!(cn.value.value(), b"Doe, Jane=CN=Evil");
    }

    /// LimarePDF signs RSA with PKCS#1 v1.5 only, so an ID limited to RSA-PSS (RFC 4055)
    /// is refused, whether only its certificate says so or its key too (as OpenSSL writes
    /// them).
    #[test]
    fn rsa_pss_only_ids_are_refused() {
        use rsa::pkcs8::DecodePrivateKey;
        use x509_cert::builder::{Builder, CertificateBuilder};
        let keys = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/signatures/keys");
        let der = std::fs::read(keys.join("rsa2048.key.der")).unwrap();
        let rsa = rsa::RsaPrivateKey::from_pkcs8_der(&der).unwrap();
        let key = Key::Rsa(Box::new(rsa.clone()));
        let mut spki = key.public_key_info().unwrap();
        spki.algorithm = spki::AlgorithmIdentifierOwned {
            oid: Oid::new_unwrap("1.2.840.113549.1.1.10"),
            parameters: None,
        };
        let profile = SelfSigned {
            subject: subject_name("PSS Only", None, None).unwrap(),
            email: None,
        };
        let validity =
            x509_cert::time::Validity::from_now(std::time::Duration::from_secs(86_400)).unwrap();
        let serial = x509_cert::serial_number::SerialNumber::generate(&mut rng());
        let signer = rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new(rsa);
        let cert = CertificateBuilder::new(profile, serial, validity, spki)
            .unwrap()
            .build::<_, rsa::pkcs1v15::Signature>(&signer)
            .unwrap();
        let cert = der::Encode::to_der(&cert).unwrap();
        let password = "pss-test-password";
        let file = pkcs12::write(
            &key.to_pkcs8().unwrap(),
            &cert,
            &[],
            "PSS Only",
            password,
            false,
        )
        .unwrap();
        let dir = std::env::temp_dir().join(format!("limarepdf-pss-id-{}", std::process::id()));
        let err = IdStore::new(&dir)
            .import(&file, password, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("allows only RSA-PSS signatures"), "{err}");
        assert!(!dir.exists(), "nothing is written for a refused ID");

        // The key itself marked RSA-PSS-only.
        let rsa_oid = [
            0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01,
        ];
        let mut pss_key = der.clone();
        let at = pss_key.windows(11).position(|w| w == rsa_oid).unwrap();
        pss_key[at + 10] = 0x0a;
        let err = Key::from_pkcs8(&pss_key).err().unwrap().to_string();
        assert!(err.contains("allows only RSA-PSS signatures"), "{err}");
    }

    #[test]
    fn passwords() {
        assert!(password_problem("1234567").is_some());
        assert!(password_problem("12345678").is_none());
        assert!(password_problem("pässwörd").is_none());
        assert!(password_problem(&"x".repeat(2000)).is_some());
    }
}
