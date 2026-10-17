//! Checking CMS signatures (RFC 5652) the way PDF signatures use them: the signer's
//! certificate, the digest of the signed bytes, the signed attributes exactly as written,
//! and RFC 3161 timestamps. Only the parts a check needs are decoded, so unusual extras
//! (embedded revocation data, attribute certificates) can't make a good signature fail.

use crate::asn1::{self, ctx, ctx_primitive, Tlv};
use crate::Result;
use anyhow::{anyhow, bail};
use chrono::{DateTime, FixedOffset, Utc};
use der::asn1::ObjectIdentifier as Oid;
use der::Decode;
use x509_cert::Certificate;

pub(crate) mod oid {
    use der::asn1::ObjectIdentifier as Oid;

    pub const DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.1");
    pub const SIGNED_DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.2");
    pub const TST_INFO: Oid = Oid::new_unwrap("1.2.840.113549.1.9.16.1.4");
    pub const CONTENT_TYPE: Oid = Oid::new_unwrap("1.2.840.113549.1.9.3");
    pub const MESSAGE_DIGEST: Oid = Oid::new_unwrap("1.2.840.113549.1.9.4");
    pub const SIGNING_TIME: Oid = Oid::new_unwrap("1.2.840.113549.1.9.5");
    pub const SIGNING_CERT: Oid = Oid::new_unwrap("1.2.840.113549.1.9.16.2.12");
    pub const SIGNING_CERT_V2: Oid = Oid::new_unwrap("1.2.840.113549.1.9.16.2.47");
    pub const TIMESTAMP_TOKEN: Oid = Oid::new_unwrap("1.2.840.113549.1.9.16.2.14");

    pub const MD5: Oid = Oid::new_unwrap("1.2.840.113549.2.5");
    pub const RIPEMD160: Oid = Oid::new_unwrap("1.3.36.3.2.1");
    pub const SHA1: Oid = Oid::new_unwrap("1.3.14.3.2.26");
    pub const SHA224: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.4");
    pub const SHA256: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.1");
    pub const SHA384: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.2");
    pub const SHA512: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.3");

    pub const RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.1");
    pub const MD5_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.4");
    pub const SHA1_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.5");
    pub const SHA224_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.14");
    pub const SHA256_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.11");
    pub const SHA384_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.12");
    pub const SHA512_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.13");
    pub const RSA_PSS: Oid = Oid::new_unwrap("1.2.840.113549.1.1.10");
    pub const MGF1: Oid = Oid::new_unwrap("1.2.840.113549.1.1.8");

    pub const EC_PUBLIC_KEY: Oid = Oid::new_unwrap("1.2.840.10045.2.1");
    pub const ECDSA_SHA1: Oid = Oid::new_unwrap("1.2.840.10045.4.1");
    pub const ECDSA_SHA224: Oid = Oid::new_unwrap("1.2.840.10045.4.3.1");
    pub const ECDSA_SHA256: Oid = Oid::new_unwrap("1.2.840.10045.4.3.2");
    pub const ECDSA_SHA384: Oid = Oid::new_unwrap("1.2.840.10045.4.3.3");
    pub const ECDSA_SHA512: Oid = Oid::new_unwrap("1.2.840.10045.4.3.4");
    pub const P256: Oid = Oid::new_unwrap("1.2.840.10045.3.1.7");
    pub const P384: Oid = Oid::new_unwrap("1.3.132.0.34");

    pub const KP_TIME_STAMPING: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.3.8");
    pub const COMMON_NAME: Oid = Oid::new_unwrap("2.5.4.3");
    pub const ORGANISATION: Oid = Oid::new_unwrap("2.5.4.10");
    pub const EMAIL: Oid = Oid::new_unwrap("1.2.840.113549.1.9.1");
}

/// Why a signature isn't valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Fail {
    /// MPE can tell it is not a genuine, intact signature.
    Broken(String),
    /// MPE can't tell: an unsupported algorithm, or data it can't read.
    CantCheck(String),
}

pub(crate) type Check<T> = std::result::Result<T, Fail>;

fn broken<T>(why: impl Into<String>) -> Check<T> {
    Err(Fail::Broken(why.into()))
}

fn cant<T>(why: impl Into<String>) -> Check<T> {
    Err(Fail::CantCheck(why.into()))
}

/// Unreadable signature data is reported as "can't check", with the reason.
fn malformed(e: anyhow::Error) -> Fail {
    Fail::CantCheck(format!("the signature data is malformed ({e})"))
}

/// A digest (hash) algorithm MPE checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigestAlg {
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

impl DigestAlg {
    fn from_oid(o: Oid) -> Check<Self> {
        Ok(match o {
            oid::SHA1 => Self::Sha1,
            oid::SHA224 => Self::Sha224,
            oid::SHA256 => Self::Sha256,
            oid::SHA384 => Self::Sha384,
            oid::SHA512 => Self::Sha512,
            // Some signers name the signature algorithm where the digest algorithm belongs.
            // Its digest is unambiguous, so it is accepted, as DSS and poppler do.
            oid::SHA1_RSA | oid::ECDSA_SHA1 => Self::Sha1,
            oid::SHA224_RSA | oid::ECDSA_SHA224 => Self::Sha224,
            oid::SHA256_RSA | oid::ECDSA_SHA256 => Self::Sha256,
            oid::SHA384_RSA | oid::ECDSA_SHA384 => Self::Sha384,
            oid::SHA512_RSA | oid::ECDSA_SHA512 => Self::Sha512,
            oid::MD5 => return cant("it uses MD5, which is broken and not checked"),
            oid::RIPEMD160 => return cant("it uses RIPEMD-160, which MPE doesn't support"),
            o => return cant(format!("it uses a digest algorithm MPE doesn't know ({o})")),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Sha1 => "SHA-1",
            Self::Sha224 => "SHA-224",
            Self::Sha256 => "SHA-256",
            Self::Sha384 => "SHA-384",
            Self::Sha512 => "SHA-512",
        }
    }

    /// The digest of `parts` joined.
    pub(crate) fn hash(self, parts: &[&[u8]]) -> Vec<u8> {
        fn run<D: sha2::Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut h = D::new();
            for p in parts {
                h.update(p);
            }
            h.finalize().to_vec()
        }
        match self {
            Self::Sha1 => run::<sha1::Sha1>(parts),
            Self::Sha224 => run::<sha2::Sha224>(parts),
            Self::Sha256 => run::<sha2::Sha256>(parts),
            Self::Sha384 => run::<sha2::Sha384>(parts),
            Self::Sha512 => run::<sha2::Sha512>(parts),
        }
    }
}

/// A certificate as it appears in a signature.
#[derive(Clone)]
pub(crate) struct Cert {
    /// The encoding as written, which fingerprints and certificate hashes cover.
    pub raw: Vec<u8>,
    pub cert: Certificate,
}

impl Cert {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let cert =
            Certificate::from_der(raw).map_err(|e| anyhow!("unreadable certificate: {e}"))?;
        Ok(Self {
            raw: raw.to_vec(),
            cert,
        })
    }

    fn not_before(&self) -> DateTime<Utc> {
        unix_time(self.cert.tbs_certificate().validity().not_before)
    }

    fn not_after(&self) -> DateTime<Utc> {
        unix_time(self.cert.tbs_certificate().validity().not_after)
    }

    /// Whether the certificate was valid at `t`.
    pub fn valid_at(&self, t: DateTime<FixedOffset>) -> bool {
        let t = t.with_timezone(&Utc);
        self.not_before() <= t && t <= self.not_after()
    }

    /// The extension of type `T`; `None` when absent, unreadable or repeated.
    pub(crate) fn extension<'a, T: Decode<'a> + der::oid::AssociatedOid>(&'a self) -> Option<T> {
        self.cert
            .tbs_certificate()
            .get_extension::<T>()
            .ok()
            .flatten()
            .map(|(_, v)| v)
    }

    /// False when the certificate limits its key to uses that don't include signing.
    pub fn key_usage_allows_signing(&self) -> bool {
        use x509_cert::ext::pkix::KeyUsage;
        self.extension::<KeyUsage>()
            .is_none_or(|k| k.digital_signature() || k.non_repudiation())
    }

    /// Whether the certificate is meant for a timestamp authority.
    pub fn for_timestamping(&self) -> bool {
        use x509_cert::ext::pkix::ExtendedKeyUsage;
        self.extension::<ExtendedKeyUsage>()
            .is_some_and(|e| e.0.contains(&oid::KP_TIME_STAMPING))
    }

    pub(crate) fn subject_key_id(&self) -> Option<Vec<u8>> {
        use x509_cert::ext::pkix::SubjectKeyIdentifier;
        self.extension::<SubjectKeyIdentifier>()
            .map(|k| k.0.as_bytes().to_vec())
    }

    pub fn info(&self) -> CertInfo {
        let tbs = self.cert.tbs_certificate();
        let subject = tbs.subject();
        let issuer = tbs.issuer();
        let email = name_part(subject, oid::EMAIL).or_else(|| {
            use x509_cert::ext::pkix::name::GeneralName;
            use x509_cert::ext::pkix::SubjectAltName;
            self.extension::<SubjectAltName>()?
                .0
                .into_iter()
                .find_map(|n| match n {
                    GeneralName::Rfc822Name(s) => Some(s.to_string()),
                    _ => None,
                })
        });
        let (key, key_bits) = match PublicKey::from_cert(self) {
            Ok(k) => (k.describe(), k.bits()),
            Err(_) => (
                format!(
                    "unsupported ({})",
                    tbs.subject_public_key_info().algorithm.oid
                ),
                0,
            ),
        };
        CertInfo {
            subject: subject.to_string(),
            common_name: name_part(subject, oid::COMMON_NAME),
            organisation: name_part(subject, oid::ORGANISATION),
            email,
            issuer: issuer.to_string(),
            issuer_common_name: name_part(issuer, oid::COMMON_NAME),
            serial: hex_colons(tbs.serial_number().as_bytes()),
            not_before: self.not_before(),
            not_after: self.not_after(),
            sha256: hex_colons(&DigestAlg::Sha256.hash(&[&self.raw])),
            self_issued: subject == issuer,
            key,
            key_bits,
            der: self.raw.clone(),
        }
    }
}

/// What a certificate says about its holder, for showing to people.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertInfo {
    /// The full subject name (RFC 4514 form, such as `CN=Jane Doe,O=Example Ltd`).
    pub subject: String,
    pub common_name: Option<String>,
    pub organisation: Option<String>,
    pub email: Option<String>,
    pub issuer: String,
    pub issuer_common_name: Option<String>,
    /// The serial number in hex, bytes separated by colons.
    pub serial: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    /// The SHA-256 fingerprint of the certificate, hex bytes separated by colons: what
    /// people compare to know it's the same certificate.
    pub sha256: String,
    /// Issued by itself (subject and issuer are the same): a self-made ID.
    pub self_issued: bool,
    /// The key, such as "RSA 3072-bit" or "ECDSA P-256".
    pub key: String,
    /// The key's size in bits (0 when unsupported).
    pub key_bits: usize,
    /// The certificate as written.
    pub der: Vec<u8>,
}

impl CertInfo {
    /// The best short name: the common name, else the organisation, else the email,
    /// else the full subject.
    pub fn display_name(&self) -> &str {
        self.common_name
            .as_deref()
            .or(self.organisation.as_deref())
            .or(self.email.as_deref())
            .unwrap_or(&self.subject)
    }

    /// Whether the certificate is valid today, by this computer's clock.
    pub fn valid_now(&self) -> bool {
        let now = Utc::now();
        self.not_before <= now && now <= self.not_after
    }
}

fn unix_time(t: x509_cert::time::Time) -> DateTime<Utc> {
    let d = t.to_unix_duration();
    DateTime::from_timestamp(d.as_secs() as i64, 0).unwrap_or_default()
}

fn hex_colons(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// The first value of the name attribute `o` in `name`, as text.
fn name_part(name: &x509_cert::name::Name, o: Oid) -> Option<String> {
    name.iter()
        .find(|a| a.oid == o)
        .and_then(|a| Tlv::read(&der::Encode::to_der(&a.value).ok()?).ok()?.text())
}

/// A signer's public key.
enum PublicKey {
    Rsa(rsa::RsaPublicKey),
    P256(p256::ecdsa::VerifyingKey),
    P384(p384::ecdsa::VerifyingKey),
}

impl PublicKey {
    fn from_cert(c: &Cert) -> Check<Self> {
        let spki = c.cert.tbs_certificate().subject_public_key_info();
        let bits = spki
            .subject_public_key
            .as_bytes()
            .ok_or_else(|| Fail::CantCheck("the certificate's key is malformed".into()))?;
        match spki.algorithm.oid {
            oid::RSA | oid::RSA_PSS => {
                use pkcs1::DecodeRsaPublicKey;
                rsa::RsaPublicKey::from_pkcs1_der(bits)
                    .map(Self::Rsa)
                    .map_err(|e| {
                        Fail::CantCheck(format!("the certificate's RSA key can't be used ({e})"))
                    })
            }
            oid::EC_PUBLIC_KEY => {
                let curve = spki
                    .algorithm
                    .parameters
                    .as_ref()
                    .and_then(|p| p.decode_as::<Oid>().ok());
                let bad =
                    |e| Fail::CantCheck(format!("the certificate's EC key can't be used ({e})"));
                match curve {
                    Some(oid::P256) => p256::ecdsa::VerifyingKey::from_sec1_bytes(bits)
                        .map(Self::P256)
                        .map_err(bad),
                    Some(oid::P384) => p384::ecdsa::VerifyingKey::from_sec1_bytes(bits)
                        .map(Self::P384)
                        .map_err(bad),
                    Some(c) => cant(format!(
                        "the signer's key uses an elliptic curve MPE doesn't support ({c})"
                    )),
                    None => cant("the certificate's EC key names no curve"),
                }
            }
            o => cant(format!("the signer's key type isn't supported ({o})")),
        }
    }

    fn bits(&self) -> usize {
        use rsa::traits::PublicKeyParts;
        match self {
            Self::Rsa(k) => k.n().bits_vartime() as usize,
            Self::P256(_) => 256,
            Self::P384(_) => 384,
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Rsa(_) => format!("RSA {}-bit", self.bits()),
            Self::P256(_) => "ECDSA P-256".into(),
            Self::P384(_) => "ECDSA P-384".into(),
        }
    }
}

/// An algorithm identifier: the algorithm and its parameters, if any.
struct AlgId<'a> {
    oid: Oid,
    params: Option<Tlv<'a>>,
}

impl<'a> AlgId<'a> {
    fn parse(t: Tlv<'a>) -> Result<Self> {
        let kids = t.expect(asn1::SEQUENCE, "algorithm")?.children()?;
        match &kids[..] {
            [o] => Ok(Self {
                oid: o.oid()?,
                params: None,
            }),
            [o, p] => Ok(Self {
                oid: o.oid()?,
                params: (p.tag != asn1::NULL).then_some(*p),
            }),
            _ => bail!("algorithm has {} parts", kids.len()),
        }
    }
}

/// How the signature value is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scheme {
    RsaPkcs1,
    RsaPss { salt: usize },
    Ecdsa,
}

impl Scheme {
    fn name(self) -> &'static str {
        match self {
            Self::RsaPkcs1 => "RSA PKCS#1 v1.5",
            Self::RsaPss { .. } => "RSA-PSS",
            Self::Ecdsa => "ECDSA",
        }
    }
}

/// The scheme and digest a signature algorithm names; `digest` is the signer's digest
/// algorithm, used when the signature algorithm names none.
fn scheme(alg: &AlgId, digest: DigestAlg) -> Check<(Scheme, DigestAlg)> {
    use DigestAlg::*;
    Ok(match alg.oid {
        oid::RSA => (Scheme::RsaPkcs1, digest),
        oid::SHA1_RSA => (Scheme::RsaPkcs1, Sha1),
        oid::SHA224_RSA => (Scheme::RsaPkcs1, Sha224),
        oid::SHA256_RSA => (Scheme::RsaPkcs1, Sha256),
        oid::SHA384_RSA => (Scheme::RsaPkcs1, Sha384),
        oid::SHA512_RSA => (Scheme::RsaPkcs1, Sha512),
        oid::RSA_PSS => pss_params(alg.params).map_err(malformed)??,
        oid::EC_PUBLIC_KEY => (Scheme::Ecdsa, digest),
        oid::ECDSA_SHA1 => (Scheme::Ecdsa, Sha1),
        oid::ECDSA_SHA224 => (Scheme::Ecdsa, Sha224),
        oid::ECDSA_SHA256 => (Scheme::Ecdsa, Sha256),
        oid::ECDSA_SHA384 => (Scheme::Ecdsa, Sha384),
        oid::ECDSA_SHA512 => (Scheme::Ecdsa, Sha512),
        oid::MD5_RSA => return cant("it uses MD5, which is broken and not checked"),
        o => {
            return cant(format!(
                "it uses a signature algorithm MPE doesn't know ({o})"
            ))
        }
    })
}

/// RSASSA-PSS parameters (RFC 4055): the digest (default SHA-1), MGF1 with the same
/// digest, the salt length (default 20) and trailer field 1.
fn pss_params(p: Option<Tlv>) -> Result<Check<(Scheme, DigestAlg)>> {
    let mut digest = oid::SHA1;
    let mut mgf_digest = digest;
    let mut salt = 20usize;
    for f in p.map(|p| p.children()).transpose()?.unwrap_or_default() {
        let inner = Tlv::read_all(f.body)?;
        match f.tag {
            0xa0 => digest = AlgId::parse(inner)?.oid,
            0xa1 => {
                let mgf = AlgId::parse(inner)?;
                if mgf.oid != oid::MGF1 {
                    return Ok(cant("it uses an RSA-PSS mask function MPE doesn't support"));
                }
                let p = mgf.params.ok_or_else(|| anyhow!("MGF1 without a digest"))?;
                mgf_digest = AlgId::parse(p)?.oid;
            }
            0xa2 => {
                let n = inner.expect(asn1::INTEGER, "salt length")?.body;
                if n.len() > 2 || n.first().is_some_and(|b| b & 0x80 != 0) {
                    bail!("bad salt length");
                }
                salt = n.iter().fold(0, |a, &b| (a << 8) | usize::from(b));
            }
            0xa3 => {
                if inner.expect(asn1::INTEGER, "trailer")?.body != [1] {
                    return Ok(cant("it uses an RSA-PSS trailer MPE doesn't support"));
                }
            }
            t => bail!("unexpected RSA-PSS parameter {t:#04x}"),
        }
    }
    if mgf_digest != digest {
        return Ok(cant(
            "its RSA-PSS mask uses a different digest, which MPE doesn't support",
        ));
    }
    Ok(DigestAlg::from_oid(digest).map(|d| (Scheme::RsaPss { salt }, d)))
}

/// Checks `sig` over `signed` with `key`.
fn verify_value(
    key: &PublicKey,
    scheme: Scheme,
    digest: DigestAlg,
    signed: &[&[u8]],
    sig: &[u8],
) -> Check<()> {
    let prehash = digest.hash(signed);
    let no_match =
        || Fail::Broken("the signature value doesn't match the signer's certificate".into());
    match (key, scheme) {
        (PublicKey::Rsa(k), Scheme::RsaPkcs1 | Scheme::RsaPss { .. }) => {
            use rsa::traits::PublicKeyParts;
            // Some signers drop leading zero bytes; put them back.
            let size = k.size();
            if sig.len() > size {
                return Err(no_match());
            }
            let mut padded = vec![0; size - sig.len()];
            padded.extend_from_slice(sig);
            let r = match scheme {
                Scheme::RsaPss { salt } => match digest {
                    DigestAlg::Sha1 => k.verify(
                        rsa::Pss::<sha1::Sha1>::new_with_salt(salt),
                        &prehash,
                        &padded,
                    ),
                    DigestAlg::Sha224 => k.verify(
                        rsa::Pss::<sha2::Sha224>::new_with_salt(salt),
                        &prehash,
                        &padded,
                    ),
                    DigestAlg::Sha256 => k.verify(
                        rsa::Pss::<sha2::Sha256>::new_with_salt(salt),
                        &prehash,
                        &padded,
                    ),
                    DigestAlg::Sha384 => k.verify(
                        rsa::Pss::<sha2::Sha384>::new_with_salt(salt),
                        &prehash,
                        &padded,
                    ),
                    DigestAlg::Sha512 => k.verify(
                        rsa::Pss::<sha2::Sha512>::new_with_salt(salt),
                        &prehash,
                        &padded,
                    ),
                },
                _ => k.verify(pkcs1_scheme(digest), &prehash, &padded),
            };
            r.map_err(|_| no_match())
        }
        (PublicKey::P256(k), Scheme::Ecdsa) => {
            use p256::ecdsa::signature::hazmat::PrehashVerifier;
            let s = p256::ecdsa::DerSignature::from_bytes(sig).map_err(|_| no_match())?;
            k.verify_prehash(&prehash, &s).map_err(|_| no_match())
        }
        (PublicKey::P384(k), Scheme::Ecdsa) => {
            use p384::ecdsa::signature::hazmat::PrehashVerifier;
            let s = p384::ecdsa::DerSignature::from_bytes(sig).map_err(|_| no_match())?;
            k.verify_prehash(&prehash, &s).map_err(|_| no_match())
        }
        _ => broken("the signature algorithm doesn't fit the signer's key"),
    }
}

fn pkcs1_scheme(d: DigestAlg) -> rsa::Pkcs1v15Sign {
    match d {
        DigestAlg::Sha1 => rsa::Pkcs1v15Sign::new::<sha1::Sha1>(),
        DigestAlg::Sha224 => rsa::Pkcs1v15Sign::new::<sha2::Sha224>(),
        DigestAlg::Sha256 => rsa::Pkcs1v15Sign::new::<sha2::Sha256>(),
        DigestAlg::Sha384 => rsa::Pkcs1v15Sign::new::<sha2::Sha384>(),
        DigestAlg::Sha512 => rsa::Pkcs1v15Sign::new::<sha2::Sha512>(),
    }
}

/// A CMS SignedData, as far as checking needs.
pub(crate) struct SignedData<'a> {
    pub econtent_type: Oid,
    /// The encapsulated content, when the signature holds it.
    pub econtent: Option<Vec<u8>>,
    pub certs: Vec<Cert>,
    /// Why certificates that couldn't be read were left out.
    pub unreadable_certs: Vec<String>,
    signers: Vec<Tlv<'a>>,
}

impl<'a> SignedData<'a> {
    /// Reads a ContentInfo holding SignedData. Bytes after it are not allowed here; the
    /// caller trims a PDF's zero padding first.
    pub fn parse(blob: &'a [u8]) -> Result<Self> {
        let ci = Tlv::read_all(blob)?.expect(asn1::SEQUENCE, "ContentInfo")?;
        let kids = ci.children()?;
        let [ct, content] = &kids[..] else {
            bail!("ContentInfo has {} parts", kids.len());
        };
        if ct.oid()? != oid::SIGNED_DATA {
            bail!("not SignedData but {}", ct.oid()?);
        }
        let sd = Tlv::read_all(content.expect(ctx(0), "content")?.body)?
            .expect(asn1::SEQUENCE, "SignedData")?;
        let parts = sd.children()?;
        let mut it = parts.into_iter().peekable();
        it.next()
            .ok_or_else(|| anyhow!("no version"))?
            .expect(asn1::INTEGER, "version")?;
        it.next()
            .ok_or_else(|| anyhow!("no digest algorithms"))?
            .expect(asn1::SET, "digest algorithms")?;
        let encap = it
            .next()
            .ok_or_else(|| anyhow!("no content info"))?
            .expect(asn1::SEQUENCE, "encapsulated content")?
            .children()?;
        let (econtent_type, econtent) = match &encap[..] {
            [t] => (t.oid()?, None),
            [t, c] => (
                t.oid()?,
                Some(Tlv::read_all(c.expect(ctx(0), "content")?.body)?.octets()?),
            ),
            _ => bail!("encapsulated content has {} parts", encap.len()),
        };
        let mut certs = Vec::new();
        let mut unreadable_certs = Vec::new();
        if let Some(set) = it.next_if(|t| t.tag == ctx(0)) {
            // Other kinds of certificate (attribute certificates) are tagged; only plain
            // X.509 certificates are used.
            for c in set
                .children()?
                .into_iter()
                .filter(|c| c.tag == asn1::SEQUENCE)
            {
                match Cert::parse(c.raw) {
                    Ok(c) => certs.push(c),
                    Err(e) => unreadable_certs.push(e.to_string()),
                }
            }
        }
        // Revocation data: not needed for checking.
        it.next_if(|t| t.tag == ctx(1));
        let signers = it
            .next()
            .ok_or_else(|| anyhow!("no signer infos"))?
            .expect(asn1::SET, "signer infos")?
            .children()?;
        if it.next().is_some() {
            bail!("unexpected data after the signer infos");
        }
        Ok(Self {
            econtent_type,
            econtent,
            certs,
            unreadable_certs,
            signers,
        })
    }
}

/// One SignerInfo.
struct SignerInfo<'a> {
    sid: Sid<'a>,
    digest: AlgId<'a>,
    signed_attrs: Option<Tlv<'a>>,
    sig_alg: AlgId<'a>,
    signature: Vec<u8>,
    unsigned_attrs: Option<Tlv<'a>>,
}

enum Sid<'a> {
    IssuerSerial { issuer: Tlv<'a>, serial: Tlv<'a> },
    KeyId(Vec<u8>),
}

impl<'a> SignerInfo<'a> {
    fn parse(t: Tlv<'a>) -> Result<Self> {
        let parts = t.expect(asn1::SEQUENCE, "SignerInfo")?.children()?;
        let mut it = parts.into_iter().peekable();
        let mut next = |what: &str| it.next().ok_or_else(|| anyhow!("SignerInfo: no {what}"));
        next("version")?.expect(asn1::INTEGER, "version")?;
        let sid = next("signer")?;
        let sid = match sid.tag {
            asn1::SEQUENCE => match &sid.children()?[..] {
                [issuer, serial] => Sid::IssuerSerial {
                    issuer: issuer.expect(asn1::SEQUENCE, "issuer")?,
                    serial: serial.expect(asn1::INTEGER, "serial")?,
                },
                _ => bail!("malformed issuer and serial number"),
            },
            t if t == ctx_primitive(0) => Sid::KeyId(sid.body.to_vec()),
            t => bail!("unexpected signer identifier tag {t:#04x}"),
        };
        let digest = AlgId::parse(it.next().ok_or_else(|| anyhow!("SignerInfo: no digest"))?)?;
        let signed_attrs = it.next_if(|t| t.tag == ctx(0));
        let sig_alg = AlgId::parse(
            it.next()
                .ok_or_else(|| anyhow!("SignerInfo: no algorithm"))?,
        )?;
        let signature = it
            .next()
            .ok_or_else(|| anyhow!("SignerInfo: no signature"))?
            .octets()?;
        let unsigned_attrs = it.next_if(|t| t.tag == ctx(1));
        if it.next().is_some() {
            bail!("unexpected data in SignerInfo");
        }
        Ok(Self {
            sid,
            digest,
            signed_attrs,
            sig_alg,
            signature,
            unsigned_attrs,
        })
    }

    /// The position in `certs` of the certificate this signer names.
    fn find_cert(&self, certs: &[Cert]) -> Option<usize> {
        certs.iter().position(|c| {
            let tbs = c.cert.tbs_certificate();
            match &self.sid {
                Sid::IssuerSerial { issuer, serial } => {
                    let same_serial =
                        strip_zeros(serial.body) == strip_zeros(tbs.serial_number().as_bytes());
                    let same_issuer = der::Encode::to_der(tbs.issuer())
                        .is_ok_and(|d| d == issuer.raw)
                        || x509_cert::name::Name::from_der(issuer.raw).is_ok_and(|n| {
                            n.to_string().to_lowercase() == tbs.issuer().to_string().to_lowercase()
                        });
                    same_serial && same_issuer
                }
                Sid::KeyId(k) => c.subject_key_id().as_ref() == Some(k),
            }
        })
    }
}

fn strip_zeros(b: &[u8]) -> &[u8] {
    let n = b
        .iter()
        .take_while(|&&x| x == 0)
        .count()
        .min(b.len().saturating_sub(1));
    &b[n..]
}

/// Attributes: (type, values).
fn attributes<'a>(t: &Tlv<'a>) -> Result<Vec<(Oid, Vec<Tlv<'a>>)>> {
    t.children()?
        .into_iter()
        .map(|a| {
            let kids = a.expect(asn1::SEQUENCE, "attribute")?.children()?;
            let [o, vals] = &kids[..] else {
                bail!("attribute has {} parts", kids.len());
            };
            Ok((
                o.oid()?,
                vals.expect(asn1::SET, "attribute values")?.children()?,
            ))
        })
        .collect()
}

/// The single value of attribute `o`: `None` when absent, an error when repeated or
/// holding several values (which could make checkers disagree about what was signed).
fn single<'a>(attrs: &[(Oid, Vec<Tlv<'a>>)], o: Oid, what: &str) -> Check<Option<Tlv<'a>>> {
    let found: Vec<_> = attrs.iter().filter(|(t, _)| *t == o).collect();
    match found[..] {
        [] => Ok(None),
        [(_, vals)] if vals.len() == 1 => Ok(Some(vals[0])),
        _ => broken(format!("the signature lists its {what} more than once")),
    }
}

/// A checked signer.
pub(crate) struct Signed {
    /// Index of the signer's certificate in [`SignedData::certs`].
    pub cert: usize,
    pub digest: DigestAlg,
    /// Such as "RSA PKCS#1 v1.5".
    pub scheme: &'static str,
    /// The signing time the signer claims (the signingTime attribute).
    pub claimed_time: Option<DateTime<FixedOffset>>,
    /// The signature value, which an embedded timestamp covers.
    pub value: Vec<u8>,
    /// The embedded timestamp token (RFC 3161), as written.
    pub timestamp_token: Option<Vec<u8>>,
}

/// Checks the one signer of `sd` over `message` (in pieces), whose content type must be
/// `content_type`.
pub(crate) fn check_signer(sd: &SignedData, message: &[&[u8]], content_type: Oid) -> Check<Signed> {
    let si = match &sd.signers[..] {
        [one] => SignerInfo::parse(*one).map_err(malformed)?,
        [] => return cant("the signature has no signer"),
        many => {
            return cant(format!(
                "the signature has {} signers; a PDF signature must have exactly one",
                many.len()
            ))
        }
    };
    let Some(cert_index) = si.find_cert(&sd.certs) else {
        return match sd.unreadable_certs.first() {
            Some(e) => cant(format!("the signer's certificate can't be read ({e})")),
            None => cant("the signer's certificate isn't in the signature"),
        };
    };
    let cert = &sd.certs[cert_index];
    let key = PublicKey::from_cert(cert)?;
    let digest = DigestAlg::from_oid(si.digest.oid)?;
    let (scheme, sig_digest) = scheme(&si.sig_alg, digest)?;
    let mut claimed_time = None;
    match &si.signed_attrs {
        Some(attrs_tlv) => {
            let attrs = attributes(attrs_tlv).map_err(malformed)?;
            let ct = single(&attrs, oid::CONTENT_TYPE, "content type")?
                .ok_or_else(|| {
                    Fail::Broken("the signature doesn't say what it signs (no content type)".into())
                })?
                .oid()
                .map_err(malformed)?;
            if ct != content_type {
                return broken(format!(
                    "the signature is for another kind of content ({ct})"
                ));
            }
            let md = single(&attrs, oid::MESSAGE_DIGEST, "document digest")?
                .ok_or_else(|| {
                    Fail::Broken("the signature doesn't hold the digest of what it signs".into())
                })?
                .octets()
                .map_err(malformed)?;
            if md != digest.hash(message) {
                return broken(
                    "the signed part was changed: its digest doesn't match the signature",
                );
            }
            if let Some(t) = single(&attrs, oid::SIGNING_TIME, "signing time")? {
                claimed_time = t.time().ok();
            }
            if let Some(v) = single(&attrs, oid::SIGNING_CERT_V2, "signing certificate")? {
                check_cert_ref(v, cert, true)?;
            }
            if let Some(v) = single(&attrs, oid::SIGNING_CERT, "signing certificate")? {
                check_cert_ref(v, cert, false)?;
            }
            if attrs_tlv.indefinite {
                return cant(
                    "the signed attributes aren't DER-encoded, so what was signed is unclear",
                );
            }
            // The signature covers the attributes as a SET (RFC 5652 5.4): as written,
            // with the [0] tag replaced, never re-encoded.
            let mut signed = attrs_tlv.raw.to_vec();
            signed[0] = asn1::SET;
            verify_value(&key, scheme, sig_digest, &[&signed], &si.signature)?;
        }
        None => {
            if content_type != oid::DATA {
                return broken(
                    "the signature has no signed attributes, which this kind of content needs",
                );
            }
            verify_value(&key, scheme, sig_digest, message, &si.signature)?;
        }
    }
    let timestamp_token = match &si.unsigned_attrs {
        Some(u) => {
            let attrs = attributes(u).map_err(malformed)?;
            attrs
                .iter()
                .find(|(o, _)| *o == oid::TIMESTAMP_TOKEN)
                .and_then(|(_, v)| v.first())
                .map(|t| t.raw.to_vec())
        }
        None => None,
    };
    Ok(Signed {
        cert: cert_index,
        digest,
        scheme: scheme.name(),
        claimed_time,
        value: si.signature,
        timestamp_token,
    })
}

/// Checks an ESS signing-certificate attribute (RFC 5035; `v2`, or the SHA-1 original of
/// RFC 2634): the first certificate it names must be the signer's. This stops someone
/// swapping in another certificate for the same key.
fn check_cert_ref(v: Tlv, cert: &Cert, v2: bool) -> Check<()> {
    let first = (|| -> Result<(DigestAlg, Vec<u8>)> {
        let certs = v
            .expect(asn1::SEQUENCE, "signing certificate")?
            .children()?;
        let list = certs.first().ok_or_else(|| anyhow!("empty"))?.children()?;
        let id = list
            .first()
            .ok_or_else(|| anyhow!("no certificates"))?
            .children()?;
        let mut it = id.iter();
        let mut alg = DigestAlg::Sha1;
        if v2 {
            alg = DigestAlg::Sha256;
            if let Some(a) = it.clone().next().filter(|a| a.tag == asn1::SEQUENCE) {
                alg = DigestAlg::from_oid(AlgId::parse(*a)?.oid)
                    .map_err(|_| anyhow!("unsupported digest"))?;
                it.next();
            }
        }
        let hash = it.next().ok_or_else(|| anyhow!("no hash"))?.octets()?;
        Ok((alg, hash))
    })();
    let (alg, hash) = first.map_err(|e| {
        Fail::CantCheck(format!(
            "the signature's certificate reference is malformed ({e})"
        ))
    })?;
    if hash != alg.hash(&[&cert.raw]) {
        return broken("the signature names a different certificate from the one it carries");
    }
    Ok(())
}

/// A checked timestamp (RFC 3161).
pub(crate) struct Stamp {
    pub time: DateTime<FixedOffset>,
    pub authority: Cert,
    /// Every certificate the timestamp carries, the authority's among them.
    pub certs: Vec<Cert>,
}

/// Checks a timestamp token over `data` (in pieces): the authority's signature over its
/// TSTInfo, and that the TSTInfo's imprint is the digest of `data`.
pub(crate) fn check_timestamp(token: &[u8], data: &[&[u8]]) -> Check<Stamp> {
    let sd = SignedData::parse(token).map_err(malformed)?;
    if sd.econtent_type != oid::TST_INFO {
        return cant("the timestamp's content isn't timestamp information");
    }
    let Some(info) = &sd.econtent else {
        return cant("the timestamp holds no timestamp information");
    };
    let signed = check_signer(&sd, &[info], oid::TST_INFO)?;
    let (alg, imprint, time) = (|| -> Result<(Oid, Vec<u8>, DateTime<FixedOffset>)> {
        let kids = Tlv::read_all(info)?
            .expect(asn1::SEQUENCE, "TSTInfo")?
            .children()?;
        let [_version, _policy, imprint, _serial, gen_time, ..] = &kids[..] else {
            bail!("TSTInfo is too short");
        };
        let mi = imprint
            .expect(asn1::SEQUENCE, "message imprint")?
            .children()?;
        let [alg, hash] = &mi[..] else {
            bail!("malformed message imprint");
        };
        let time = gen_time.expect(asn1::GENERALIZED_TIME, "time")?.time()?;
        Ok((AlgId::parse(*alg)?.oid, hash.octets()?, time))
    })()
    .map_err(malformed)?;
    if imprint != DigestAlg::from_oid(alg)?.hash(data) {
        return broken("the timestamp is for different data");
    }
    Ok(Stamp {
        time,
        authority: sd.certs[signed.cert].clone(),
        certs: sd.certs,
    })
}

/// Checks an `adbe.x509.rsa_sha1` signature: a bare RSA PKCS#1 v1.5 signature (in a DER
/// OCTET STRING) over the digest of `data`, by the first certificate in `/Cert`. The
/// digest is SHA-1, or since PDF 1.6 SHA-256, SHA-384 or SHA-512; whichever the signature
/// holds is used. Returns the digest used.
pub(crate) fn check_rsa_sha1(contents: &[u8], cert: &Cert, data: &[&[u8]]) -> Check<DigestAlg> {
    let sig = Tlv::read_all(contents)
        .and_then(|t| t.octets())
        .map_err(malformed)?;
    let key = PublicKey::from_cert(cert)?;
    if !matches!(key, PublicKey::Rsa(_)) {
        return broken("this signature type needs an RSA key");
    }
    for d in [
        DigestAlg::Sha1,
        DigestAlg::Sha256,
        DigestAlg::Sha384,
        DigestAlg::Sha512,
    ] {
        if verify_value(&key, Scheme::RsaPkcs1, d, data, &sig).is_ok() {
            return Ok(d);
        }
    }
    broken("the signature doesn't match the signed part and the signer's certificate")
}

/// Checks that `issuer`'s key made `cert`'s signature, over the certificate's contents as
/// written (never re-encoded). The certificate's two copies of its signature algorithm must
/// agree, as RFC 5280 requires. Returns the digest the signature relies on.
pub(crate) fn check_cert_signature(cert: &Cert, issuer: &Cert) -> Check<DigestAlg> {
    let (tbs, alg, value) = (|| -> Result<(&[u8], AlgId, Vec<u8>)> {
        let kids = Tlv::read_all(&cert.raw)?
            .expect(asn1::SEQUENCE, "certificate")?
            .children()?;
        let [tbs, alg, sig] = &kids[..] else {
            bail!("the certificate has {} parts", kids.len());
        };
        let inner = tbs
            .expect(asn1::SEQUENCE, "certificate contents")?
            .children()?;
        // An optional [0] version, the serial number, then the signature algorithm.
        let at = usize::from(inner.first().is_some_and(|t| t.tag == ctx(0))) + 1;
        let inner_alg = inner
            .get(at)
            .ok_or_else(|| anyhow!("no signature algorithm"))?;
        if inner_alg.raw != alg.raw {
            bail!("its two signature algorithms differ");
        }
        let sig = sig.expect(asn1::BIT_STRING, "signature")?;
        let [0, value @ ..] = sig.body else {
            bail!("malformed signature bits");
        };
        Ok((tbs.raw, AlgId::parse(*alg)?, value.to_vec()))
    })()
    .map_err(|e| Fail::CantCheck(format!("the certificate is malformed ({e})")))?;
    if matches!(alg.oid, oid::RSA | oid::EC_PUBLIC_KEY) {
        return cant("the certificate's signature algorithm names no digest");
    }
    let key = PublicKey::from_cert(issuer)?;
    let (scheme, digest) = scheme(&alg, DigestAlg::Sha256)?;
    verify_value(&key, scheme, digest, &[tbs], &value).map_err(|_| {
        Fail::Broken("the issuer's signature on the certificate doesn't match".into())
    })?;
    Ok(digest)
}

/// The size in bits of a certificate's RSA key; `None` for other keys.
pub(crate) fn rsa_bits(cert: &Cert) -> Option<usize> {
    match PublicKey::from_cert(cert) {
        Ok(k @ PublicKey::Rsa(_)) => Some(k.bits()),
        _ => None,
    }
}
