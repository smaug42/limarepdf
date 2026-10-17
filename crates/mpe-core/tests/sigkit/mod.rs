//! Making signed test documents. CMS is built by hand with a tiny DER writer, so tests can
//! make every malformed shape an attacker might; lopdf appends the signature fields. The
//! keys and certificates are OpenSSL-made (tools/make-signature-keys.sh).

#![allow(dead_code)]

use lopdf::{dictionary, Dictionary, Document, IncrementalDocument, Object, StringFormat};
use mpe_core::Pdf;
use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/signatures")
}

// ------------------------------------------------------------------ keys

pub enum Key {
    Rsa(rsa::RsaPrivateKey),
    P256(p256::ecdsa::SigningKey),
    P384(p384::ecdsa::SigningKey),
}

/// A key and the certificate (DER) that goes with it.
pub struct Id {
    pub key: Key,
    pub cert: Vec<u8>,
}

/// The test ID `name`: keys/NAME.key.der and keys/NAME.crt.der.
pub fn id(name: &str) -> Id {
    id_with_cert(name, name)
}

/// Key `key` with certificate `cert` (which may be for another key).
pub fn id_with_cert(key: &str, cert: &str) -> Id {
    let dir = data_dir().join("keys");
    let der = std::fs::read(dir.join(format!("{key}.key.der"))).unwrap();
    let key = key_from_pkcs8(&der);
    let cert = std::fs::read(dir.join(format!("{cert}.crt.der"))).unwrap();
    Id { key, cert }
}

/// A PKCS#8 private key (RSA, P-256 or P-384).
pub fn key_from_pkcs8(der: &[u8]) -> Key {
    if let Ok(k) = <rsa::RsaPrivateKey as rsa::pkcs8::DecodePrivateKey>::from_pkcs8_der(der) {
        Key::Rsa(k)
    } else if let Ok(k) =
        <p256::ecdsa::SigningKey as p256::pkcs8::DecodePrivateKey>::from_pkcs8_der(der)
    {
        Key::P256(k)
    } else {
        Key::P384(
            <p384::ecdsa::SigningKey as p384::pkcs8::DecodePrivateKey>::from_pkcs8_der(der)
                .unwrap(),
        )
    }
}

/// The test timestamp authority (its certificate is for the RSA-2048 key).
pub fn tsa() -> Id {
    id_with_cert("rsa2048", "tsa")
}

pub fn cert_der(name: &str) -> Vec<u8> {
    std::fs::read(data_dir().join("keys").join(format!("{name}.crt.der"))).unwrap()
}

// ------------------------------------------------------------------ DER

pub fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let n = content.len();
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let bytes: Vec<u8> = n
            .to_be_bytes()
            .into_iter()
            .skip_while(|&b| b == 0)
            .collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(content);
    out
}

/// The same element with an indefinite length (BER), as some signers write.
pub fn indefinite(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag, 0x80];
    out.extend_from_slice(content);
    out.extend([0, 0]);
    out
}

pub fn seq(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

/// A SET with its elements in the order given (DER would sort them).
pub fn set(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x31, &parts.concat())
}

/// A SET in DER order.
pub fn der_set(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut sorted = parts.to_vec();
    sorted.sort_by(|a, b| {
        let n = a.len().max(b.len());
        let pad = |v: &Vec<u8>| {
            let mut v = v.clone();
            v.resize(n, 0);
            v
        };
        pad(a).cmp(&pad(b))
    });
    tlv(0x31, &sorted.concat())
}

pub fn ctx(n: u8, content: &[u8]) -> Vec<u8> {
    tlv(0xa0 | n, content)
}

pub fn oid(dotted: &str) -> Vec<u8> {
    tlv(
        0x06,
        der::asn1::ObjectIdentifier::new_unwrap(dotted).as_bytes(),
    )
}

pub fn octets(b: &[u8]) -> Vec<u8> {
    tlv(0x04, b)
}

pub fn int(n: u64) -> Vec<u8> {
    let mut b: Vec<u8> = n
        .to_be_bytes()
        .into_iter()
        .skip_while(|&x| x == 0)
        .collect();
    if b.is_empty() || b[0] & 0x80 != 0 {
        b.insert(0, 0);
    }
    tlv(0x02, &b)
}

pub fn null() -> Vec<u8> {
    vec![0x05, 0x00]
}

pub fn utc_time(s: &str) -> Vec<u8> {
    tlv(0x17, s.as_bytes())
}

pub fn gen_time(s: &str) -> Vec<u8> {
    tlv(0x18, s.as_bytes())
}

pub const DATA: &str = "1.2.840.113549.1.7.1";
pub const SIGNED_DATA: &str = "1.2.840.113549.1.7.2";
pub const TST_INFO: &str = "1.2.840.113549.1.9.16.1.4";
pub const CONTENT_TYPE: &str = "1.2.840.113549.1.9.3";
pub const MESSAGE_DIGEST: &str = "1.2.840.113549.1.9.4";
pub const SIGNING_TIME: &str = "1.2.840.113549.1.9.5";
pub const SIGNING_CERT_V2: &str = "1.2.840.113549.1.9.16.2.47";
pub const TIMESTAMP_TOKEN: &str = "1.2.840.113549.1.9.16.2.14";

// ------------------------------------------------------------------ digests

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    pub fn oid(self) -> &'static str {
        match self {
            Hash::Sha1 => "1.3.14.3.2.26",
            Hash::Sha256 => "2.16.840.1.101.3.4.2.1",
            Hash::Sha384 => "2.16.840.1.101.3.4.2.2",
            Hash::Sha512 => "2.16.840.1.101.3.4.2.3",
        }
    }

    pub fn of(self, parts: &[&[u8]]) -> Vec<u8> {
        fn run<D: sha2::Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut h = D::new();
            parts.iter().for_each(|p| h.update(p));
            h.finalize().to_vec()
        }
        match self {
            Hash::Sha1 => run::<sha1::Sha1>(parts),
            Hash::Sha256 => run::<sha2::Sha256>(parts),
            Hash::Sha384 => run::<sha2::Sha384>(parts),
            Hash::Sha512 => run::<sha2::Sha512>(parts),
        }
    }

    pub fn alg(self) -> Vec<u8> {
        seq(&[&oid(self.oid()), &null()])
    }
}

/// Signs the digest `hash` (made with `h`) with `key`: PKCS#1 v1.5 or, with `pss`, PSS
/// with a salt as long as the digest; ECDSA for EC keys. Returns the value and the
/// signature algorithm identifier.
pub fn sign_hash(key: &Key, h: Hash, hash: &[u8], pss: bool) -> (Vec<u8>, Vec<u8>) {
    match key {
        Key::Rsa(k) if pss => {
            let mut rng = rsa::rand_core::UnwrapErr(getrandom::SysRng);
            let salt = hash.len();
            let sig = match h {
                Hash::Sha1 => {
                    k.sign_with_rng(&mut rng, rsa::Pss::<sha1::Sha1>::new_with_salt(salt), hash)
                }
                Hash::Sha256 => k.sign_with_rng(
                    &mut rng,
                    rsa::Pss::<sha2::Sha256>::new_with_salt(salt),
                    hash,
                ),
                Hash::Sha384 => k.sign_with_rng(
                    &mut rng,
                    rsa::Pss::<sha2::Sha384>::new_with_salt(salt),
                    hash,
                ),
                Hash::Sha512 => k.sign_with_rng(
                    &mut rng,
                    rsa::Pss::<sha2::Sha512>::new_with_salt(salt),
                    hash,
                ),
            }
            .unwrap();
            let params = seq(&[
                &ctx(0, &h.alg()),
                &ctx(1, &seq(&[&oid("1.2.840.113549.1.1.8"), &h.alg()])),
                &ctx(2, &int(salt as u64)),
            ]);
            (sig, seq(&[&oid("1.2.840.113549.1.1.10"), &params]))
        }
        Key::Rsa(k) => {
            let scheme = match h {
                Hash::Sha1 => rsa::Pkcs1v15Sign::new::<sha1::Sha1>(),
                Hash::Sha256 => rsa::Pkcs1v15Sign::new::<sha2::Sha256>(),
                Hash::Sha384 => rsa::Pkcs1v15Sign::new::<sha2::Sha384>(),
                Hash::Sha512 => rsa::Pkcs1v15Sign::new::<sha2::Sha512>(),
            };
            (
                k.sign(scheme, hash).unwrap(),
                seq(&[&oid("1.2.840.113549.1.1.1"), &null()]),
            )
        }
        Key::P256(k) => {
            use p256::ecdsa::signature::hazmat::PrehashSigner;
            let s: p256::ecdsa::Signature = k.sign_prehash(hash).unwrap();
            (s.to_der().as_bytes().to_vec(), seq(&[&oid(ecdsa_oid(h))]))
        }
        Key::P384(k) => {
            use p384::ecdsa::signature::hazmat::PrehashSigner;
            let s: p384::ecdsa::Signature = k.sign_prehash(hash).unwrap();
            (s.to_der().as_bytes().to_vec(), seq(&[&oid(ecdsa_oid(h))]))
        }
    }
}

fn ecdsa_oid(h: Hash) -> &'static str {
    match h {
        Hash::Sha1 => "1.2.840.10045.4.1",
        Hash::Sha256 => "1.2.840.10045.4.3.2",
        Hash::Sha384 => "1.2.840.10045.4.3.3",
        Hash::Sha512 => "1.2.840.10045.4.3.4",
    }
}

// ------------------------------------------------------------------ CMS

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Md {
    Right,
    Missing,
    Wrong,
    Twice,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ess {
    None,
    Right,
    /// Names another certificate.
    Wrong,
}

/// How to build a CMS SignedData; `Default` is an ordinary Acrobat-style signature.
#[derive(Clone)]
pub struct Cms {
    pub hash: Hash,
    pub pss: bool,
    pub signed_attrs: bool,
    /// The content-type attribute's value; `None` leaves the attribute out.
    pub content_type: Option<&'static str>,
    pub message_digest: Md,
    pub signing_time: Option<&'static str>,
    pub ess: Ess,
    /// Write the signed attributes in an order DER wouldn't (the signature covers them
    /// as written).
    pub unsorted: bool,
    pub include_cert: bool,
    pub extra_certs: Vec<Vec<u8>>,
    /// Name the signer by subject key identifier instead of issuer and serial number.
    pub ski: bool,
    pub second_signer: bool,
    pub econtent_type: &'static str,
    /// Content held in the signature itself.
    pub econtent: Option<Vec<u8>>,
    /// The digest algorithm identifier to write instead of the real one.
    pub digest_alg: Option<Vec<u8>>,
    /// The signature algorithm identifier to write instead of the real one.
    pub sig_alg: Option<Vec<u8>>,
    /// An embedded timestamp from [`tsa`] at this GeneralizedTime; with the flag set, for
    /// other data than the signature value.
    pub timestamp: Option<(&'static str, bool)>,
    /// Indefinite lengths on the outer structures (BER).
    pub ber: bool,
    /// Flip a bit of the signature value.
    pub corrupt: bool,
}

impl Default for Cms {
    fn default() -> Self {
        Self {
            hash: Hash::Sha256,
            pss: false,
            signed_attrs: true,
            content_type: Some(DATA),
            message_digest: Md::Right,
            signing_time: Some("260601120000Z"),
            ess: Ess::Right,
            unsorted: false,
            include_cert: true,
            extra_certs: Vec::new(),
            ski: false,
            second_signer: false,
            econtent_type: DATA,
            econtent: None,
            digest_alg: None,
            sig_alg: None,
            timestamp: None,
            ber: false,
            corrupt: false,
        }
    }
}

fn attr(o: &str, value: &[u8]) -> Vec<u8> {
    seq(&[&oid(o), &set(&[value])])
}

/// The issuer and serial number, and the subject key identifier, of a certificate.
fn cert_ids(cert: &[u8]) -> (Vec<u8>, Vec<u8>) {
    use der::{Decode, Encode};
    let c = x509_cert::Certificate::from_der(cert).unwrap();
    let tbs = c.tbs_certificate();
    let isn = seq(&[
        &tbs.issuer().to_der().unwrap(),
        &tbs.serial_number().to_der().unwrap(),
    ]);
    let ski = tbs
        .get_extension::<x509_cert::ext::pkix::SubjectKeyIdentifier>()
        .unwrap()
        .map(|(_, k)| k.0.as_bytes().to_vec())
        .unwrap_or_default();
    (isn, ski)
}

fn signer_info(signer: &Id, message: &[&[u8]], o: &Cms) -> Vec<u8> {
    let (isn, ski) = cert_ids(&signer.cert);
    let digest = o.hash.of(message);
    let mut attrs: Vec<Vec<u8>> = Vec::new();
    if let Some(ct) = o.content_type {
        attrs.push(attr(CONTENT_TYPE, &oid(ct)));
    }
    if let Some(t) = o.signing_time {
        attrs.push(attr(SIGNING_TIME, &utc_time(t)));
    }
    match o.message_digest {
        Md::Right => attrs.push(attr(MESSAGE_DIGEST, &octets(&digest))),
        Md::Missing => {}
        Md::Wrong => attrs.push(attr(MESSAGE_DIGEST, &octets(&vec![0x5a; digest.len()]))),
        Md::Twice => {
            attrs.push(attr(MESSAGE_DIGEST, &octets(&digest)));
            attrs.push(attr(MESSAGE_DIGEST, &octets(&vec![0x5a; digest.len()])));
        }
    }
    let ess_cert = match o.ess {
        Ess::None => None,
        Ess::Right => Some(signer.cert.clone()),
        Ess::Wrong => Some(cert_der("rsa3072"))
            .filter(|c| *c != signer.cert)
            .or(Some(cert_der("p256"))),
    };
    if let Some(c) = ess_cert {
        let id = seq(&[&octets(&Hash::Sha256.of(&[&c]))]);
        attrs.push(attr(SIGNING_CERT_V2, &seq(&[&seq(&[&id])])));
    }
    let signed_set = if o.unsorted {
        // Reverse DER order: messageDigest before contentType and so on.
        let mut v = attrs.clone();
        v.sort();
        v.reverse();
        set(&v.iter().map(Vec::as_slice).collect::<Vec<_>>())
    } else {
        der_set(&attrs)
    };
    let (to_sign, signed_attrs) = if o.signed_attrs {
        let mut as_ctx = signed_set.clone();
        as_ctx[0] = 0xa0;
        (vec![signed_set], Some(as_ctx))
    } else {
        (message.iter().map(|p| p.to_vec()).collect(), None)
    };
    let parts: Vec<&[u8]> = to_sign.iter().map(Vec::as_slice).collect();
    let (mut value, sig_alg) = sign_hash(&signer.key, o.hash, &o.hash.of(&parts), o.pss);
    if o.corrupt {
        let n = value.len() / 2;
        value[n] ^= 1;
    }
    let mut unsigned = None;
    if let Some((time, wrong)) = o.timestamp {
        let data: &[u8] = if wrong { b"something else" } else { &value };
        let token = timestamp_token(&tsa(), &[data], time, Hash::Sha256);
        unsigned = Some(ctx(1, &attr(TIMESTAMP_TOKEN, &token)));
    }
    let sid = if o.ski { tlv(0x80, &ski) } else { isn };
    let mut parts: Vec<Vec<u8>> = vec![
        int(if o.ski { 3 } else { 1 }),
        sid,
        o.digest_alg.clone().unwrap_or_else(|| o.hash.alg()),
    ];
    parts.extend(signed_attrs);
    parts.push(o.sig_alg.clone().unwrap_or(sig_alg));
    parts.push(octets(&value));
    parts.extend(unsigned);
    seq(&parts.iter().map(Vec::as_slice).collect::<Vec<_>>())
}

/// A ContentInfo with SignedData by `signer` over `message` (in pieces).
pub fn cms(signer: &Id, message: &[&[u8]], o: &Cms) -> Vec<u8> {
    let mut infos = vec![signer_info(signer, message, o)];
    if o.second_signer {
        infos.push(signer_info(&id("p256"), message, o));
    }
    let mut certs: Vec<Vec<u8>> = Vec::new();
    if o.include_cert {
        certs.push(signer.cert.clone());
    }
    if o.second_signer {
        certs.push(cert_der("p256"));
    }
    certs.extend(o.extra_certs.iter().cloned());
    let encap = match &o.econtent {
        Some(e) => seq(&[&oid(o.econtent_type), &ctx(0, &octets(e))]),
        None => seq(&[&oid(o.econtent_type)]),
    };
    let mut sd: Vec<Vec<u8>> = vec![int(1), set(&[&o.hash.alg()]), encap];
    if !certs.is_empty() {
        sd.push(ctx(0, &certs.concat()));
    }
    sd.push(set(&infos.iter().map(Vec::as_slice).collect::<Vec<_>>()));
    let sd: Vec<&[u8]> = sd.iter().map(Vec::as_slice).collect();
    if o.ber {
        let inner = indefinite(0x30, &sd.concat());
        indefinite(0x30, &[oid(SIGNED_DATA), indefinite(0xa0, &inner)].concat())
    } else {
        seq(&[&oid(SIGNED_DATA), &ctx(0, &seq(&sd))])
    }
}

// ------------------------------------------------------------------ certificates

/// A certificate extension: its OID, whether it's critical, and its value (DER).
pub fn ext(o: &str, critical: bool, value: &[u8]) -> Vec<u8> {
    if critical {
        seq(&[&oid(o), &[0x01, 0x01, 0xff], &octets(value)])
    } else {
        seq(&[&oid(o), &octets(value)])
    }
}

/// keyUsage digitalSignature + nonRepudiation, critical.
pub fn ext_signing_key_usage() -> Vec<u8> {
    ext("2.5.29.15", true, &[0x03, 0x02, 0x06, 0xc0])
}

/// The subject key identifier of a certificate, as an authority key identifier extension.
pub fn ext_authority_key_id(issuer_cert: &[u8]) -> Vec<u8> {
    let (_, ski) = cert_ids(issuer_cert);
    ext("2.5.29.35", false, &seq(&[&tlv(0x80, &ski)]))
}

pub const SHA256_RSA: &str = "1.2.840.113549.1.1.11";

/// How [`make_cert`] builds a certificate by hand.
pub struct CertSpec<'a> {
    /// The certificate whose public key the new one carries.
    pub key_of: &'a [u8],
    pub cn: &'a str,
    pub not_before: &'a str,
    pub not_after: &'a str,
    pub extensions: Vec<Vec<u8>>,
    /// The signature algorithm written in the contents; by default the one outside.
    pub inner_alg: Option<Vec<u8>>,
}

/// A certificate issued by `issuer` (an RSA key and its certificate), signed with
/// SHA-256, built by hand so tests can make shapes OpenSSL won't.
pub fn make_cert(issuer: &Id, s: &CertSpec) -> Vec<u8> {
    use der::{Decode, Encode};
    let parse = |d: &[u8]| x509_cert::Certificate::from_der(d).unwrap();
    let issuer_name = parse(&issuer.cert)
        .tbs_certificate()
        .subject()
        .to_der()
        .unwrap();
    let spki = parse(s.key_of)
        .tbs_certificate()
        .subject_public_key_info()
        .to_der()
        .unwrap();
    let alg = seq(&[&oid(SHA256_RSA), &null()]);
    let subject = seq(&[&set(&[&seq(&[
        &oid("2.5.4.3"),
        &tlv(0x0c, s.cn.as_bytes()),
    ])])]);
    let tbs = seq(&[
        &ctx(0, &int(2)),
        &int(0x1234_5678),
        s.inner_alg.as_ref().unwrap_or(&alg),
        &issuer_name,
        &seq(&[&utc_time(s.not_before), &utc_time(s.not_after)]),
        &subject,
        &spki,
        &ctx(
            3,
            &seq(&s.extensions.iter().map(Vec::as_slice).collect::<Vec<_>>()),
        ),
    ]);
    let Key::Rsa(k) = &issuer.key else {
        panic!("make_cert signs with RSA keys");
    };
    let value = k
        .sign(
            rsa::Pkcs1v15Sign::new::<sha2::Sha256>(),
            &Hash::Sha256.of(&[&tbs]),
        )
        .unwrap();
    seq(&[&tbs, &alg, &tlv(0x03, &[&[0u8][..], &value].concat())])
}

/// A timestamp token (RFC 3161) from `tsa` over `data` at `time` (GeneralizedTime).
pub fn timestamp_token(tsa: &Id, data: &[&[u8]], time: &str, h: Hash) -> Vec<u8> {
    let info = seq(&[
        &int(1),
        &oid("1.2.3.4.5"),
        &seq(&[&h.alg(), &octets(&h.of(data))]),
        &int(42),
        &gen_time(time),
    ]);
    cms(
        tsa,
        &[&info],
        &Cms {
            content_type: Some(TST_INFO),
            econtent_type: TST_INFO,
            econtent: Some(info.clone()),
            signing_time: None,
            ..Cms::default()
        },
    )
}

/// A bare RSA PKCS#1 v1.5 signature in an OCTET STRING, for `adbe.x509.rsa_sha1`.
pub fn rsa_sha1_value(signer: &Id, message: &[&[u8]], h: Hash) -> Vec<u8> {
    octets(&sign_hash(&signer.key, h, &h.of(message), false).0)
}

// ------------------------------------------------------------------ PDF

/// A one-page document to sign.
pub fn base_pdf() -> Vec<u8> {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    p.snapshot().unwrap()
}

/// How the signature field and dictionary are written.
#[derive(Clone)]
pub struct Field {
    pub name: &'static str,
    pub sub_filter: &'static str,
    /// `/Type /DocTimeStamp` instead of `/Sig`.
    pub doc_timestamp: bool,
    /// The signature dictionary written inside the field instead of as an object.
    pub direct: bool,
    /// Bytes of placeholder for the signature value.
    pub room: usize,
    /// `/Reference` entries (DocMDP, FieldMDP).
    pub reference: Option<Object>,
    /// Point the catalog's `/Perms /DocMDP` at this signature.
    pub perms: bool,
    /// A second field sharing the signature dictionary.
    pub share_with: Option<&'static str>,
    pub m: Option<&'static str>,
    pub reason: Option<&'static str>,
    /// `/Cert` (for `adbe.x509.rsa_sha1`).
    pub cert: Option<Vec<u8>>,
    /// A zero-sized widget, as for invisible signatures and document timestamps.
    pub invisible: bool,
    /// Sign the existing empty signature field of this name instead of adding one.
    pub existing: bool,
}

impl Default for Field {
    fn default() -> Self {
        Self {
            name: "Signature1",
            sub_filter: "adbe.pkcs7.detached",
            doc_timestamp: false,
            direct: false,
            room: 8192,
            reference: None,
            perms: false,
            share_with: None,
            m: Some("D:20260601120000+02'00'"),
            reason: Some("Testing"),
            cert: None,
            invisible: false,
            existing: false,
        }
    }
}

const BR_PLACEHOLDER: i64 = 9_999_999_999;

/// `input` with an appended revision holding a signature field, its value made by `make`
/// from the signed byte ranges.
pub fn sign_with(input: &[u8], f: &Field, make: impl FnOnce(&[&[u8]]) -> Vec<u8>) -> Vec<u8> {
    let doc = Document::load_mem(input).unwrap();
    let mut inc = IncrementalDocument::create_from(input.to_vec(), doc);
    let prev = inc.get_prev_documents();
    let root_id = prev.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let page_id = *prev.get_pages().get(&1).unwrap();
    let mut catalog = prev.get_object(root_id).unwrap().as_dict().unwrap().clone();
    let mut page = prev.get_object(page_id).unwrap().as_dict().unwrap().clone();
    let acro = catalog
        .get(b"AcroForm")
        .ok()
        .map(|o| match o {
            Object::Reference(id) => prev.get_object(*id).unwrap().as_dict().unwrap().clone(),
            o => o.as_dict().unwrap().clone(),
        })
        .unwrap_or_default();

    let mut sig = dictionary! {
        "Type" => if f.doc_timestamp { "DocTimeStamp" } else { "Sig" },
        "Filter" => "Adobe.PPKLite",
        "SubFilter" => Object::Name(f.sub_filter.as_bytes().to_vec()),
        "ByteRange" => vec![0.into(), BR_PLACEHOLDER.into(), BR_PLACEHOLDER.into(), BR_PLACEHOLDER.into()],
        "Contents" => Object::String(vec![0; f.room], StringFormat::Hexadecimal),
    };
    if let Some(m) = f.m {
        sig.set("M", Object::string_literal(m));
    }
    if let Some(r) = f.reason {
        sig.set("Reason", Object::string_literal(r));
    }
    if let Some(r) = &f.reference {
        sig.set("Reference", r.clone());
    }
    if let Some(c) = &f.cert {
        sig.set("Cert", Object::String(c.clone(), StringFormat::Hexadecimal));
    }
    let existing = f.existing.then(|| {
        prev.objects
            .iter()
            .filter_map(|(id, o)| Some((*id, o.as_dict().ok()?.clone())))
            .find(|(_, d)| {
                d.get(b"T")
                    .and_then(Object::as_str)
                    .is_ok_and(|t| t == f.name.as_bytes())
            })
            .expect("no such field")
    });
    let new = &mut inc.new_document;
    let sig_obj: Object = if f.direct {
        Object::Dictionary(sig)
    } else {
        new.add_object(sig).into()
    };
    if let Some((id, mut d)) = existing {
        // Sign the empty field already in the document.
        d.set("V", sig_obj);
        new.set_object(id, Object::Dictionary(d));
        let mut out = Vec::new();
        inc.save_to(&mut out).unwrap();
        return fill(out, f.room, make);
    }
    let rect: Vec<Object> = if f.invisible {
        vec![0.into(), 0.into(), 0.into(), 0.into()]
    } else {
        vec![20.into(), 20.into(), 120.into(), 60.into()]
    };
    let field = |name: &str, v: Object| {
        dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Sig",
            "T" => Object::string_literal(name),
            "V" => v,
            "F" => 132,
            "Rect" => rect.clone(),
            "P" => page_id,
        }
    };
    let mut ids = vec![new.add_object(field(f.name, sig_obj.clone()))];
    if let Some(other) = f.share_with {
        ids.push(new.add_object(field(other, sig_obj.clone())));
    }

    let mut annots = page
        .get(b"Annots")
        .ok()
        .and_then(|a| a.as_array().ok())
        .cloned()
        .unwrap_or_default();
    annots.extend(ids.iter().map(|&i| Object::from(i)));
    page.set("Annots", annots);
    inc.new_document
        .set_object(page_id, Object::Dictionary(page));
    let mut acro: Dictionary = acro;
    let mut fields = acro
        .get(b"Fields")
        .ok()
        .and_then(|a| a.as_array().ok())
        .cloned()
        .unwrap_or_default();
    fields.extend(ids.iter().map(|&i| Object::from(i)));
    acro.set("Fields", fields);
    acro.set("SigFlags", 3);
    catalog.set("AcroForm", acro);
    if f.perms {
        catalog.set("Perms", dictionary! { "DocMDP" => sig_obj });
    }
    inc.new_document
        .set_object(root_id, Object::Dictionary(catalog));

    let mut out = Vec::new();
    inc.save_to(&mut out).unwrap();
    fill(out, f.room, make)
}

fn find(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    hay.windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle)
        .map(|(i, _)| i)
        .collect()
}

/// Patches the byte range and writes the signature value into the placeholder.
fn fill(mut out: Vec<u8>, room: usize, make: impl FnOnce(&[&[u8]]) -> Vec<u8>) -> Vec<u8> {
    let zeros = format!("<{}>", "00".repeat(room));
    let [b] = find(&out, zeros.as_bytes())[..] else {
        panic!("placeholder not found once");
    };
    let c = b + zeros.len();
    let br = format!("{BR_PLACEHOLDER} {BR_PLACEHOLDER} {BR_PLACEHOLDER}");
    let [p] = find(&out, br.as_bytes())[..] else {
        panic!("byte range placeholder not found once");
    };
    let real = format!("{b} {c} {}", out.len() - c);
    out[p..p + br.len()].copy_from_slice(format!("{real:<width$}", width = br.len()).as_bytes());
    let value = make(&[&out[..b], &out[c..]]);
    assert!(
        value.len() <= room,
        "signature value too big for the placeholder"
    );
    let hex: String = value.iter().map(|x| format!("{x:02X}")).collect();
    out[b + 1..b + 1 + hex.len()].copy_from_slice(hex.as_bytes());
    out
}

/// Signs `input` with `signer` and options `o` in a field made as `f`.
pub fn sign(input: &[u8], signer: &Id, f: &Field, o: &Cms) -> Vec<u8> {
    sign_with(input, f, |ranges| cms(signer, ranges, o))
}

/// An ordinary RSA-3072 signature on the base document.
pub fn signed() -> Vec<u8> {
    sign(
        &base_pdf(),
        &id("rsa3072"),
        &Field::default(),
        &Cms::default(),
    )
}

/// The signed byte range `[0, b, c, d]` of the last signature in `bytes`.
pub fn last_range(bytes: &[u8]) -> [usize; 4] {
    let at = find(bytes, b"/ByteRange");
    let p = *at.last().unwrap();
    let open = p + bytes[p..].iter().position(|&x| x == b'[').unwrap();
    let close = open + bytes[open..].iter().position(|&x| x == b']').unwrap();
    let nums: Vec<usize> = std::str::from_utf8(&bytes[open + 1..close])
        .unwrap()
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect();
    [nums[0], nums[1], nums[2], nums[3]]
}

/// Replaces the one occurrence of `from` with `to` (same length, so offsets hold).
pub fn replace_once(bytes: &mut [u8], from: &[u8], to: &[u8]) {
    assert_eq!(from.len(), to.len());
    let [p] = find(bytes, from)[..] else {
        panic!("{:?} not found once", String::from_utf8_lossy(from));
    };
    bytes[p..p + to.len()].copy_from_slice(to);
}

/// Appends an incremental update in which `change` edits the document (start from
/// `inc.get_prev_documents()`, write to `inc.new_document`).
pub fn append_update(bytes: &[u8], change: impl FnOnce(&mut IncrementalDocument)) -> Vec<u8> {
    let doc = Document::load_mem(bytes).unwrap();
    let mut inc = IncrementalDocument::create_from(bytes.to_vec(), doc);
    change(&mut inc);
    let mut out = Vec::new();
    inc.save_to(&mut out).unwrap();
    out
}

/// Appends a revision written by hand, for shapes lopdf won't write: `objects` (number
/// and body, generation 0), then a cross-reference table listing them and `frees`, with
/// `moved` overriding where an entry points (number, offset), and `trailer` added to the
/// trailer. `/Prev` points to the last revision.
pub fn append_raw(
    bytes: &[u8],
    objects: &[(u32, &[u8])],
    frees: &[u32],
    moved: &[(u32, usize)],
    trailer: &str,
) -> Vec<u8> {
    let doc = Document::load_mem(bytes).unwrap();
    let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let tail = String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(40)..]).into_owned();
    let prev: usize = tail
        .split("startxref")
        .last()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let mut out = bytes.to_vec();
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    let mut entries: std::collections::BTreeMap<u32, Option<usize>> = Default::default();
    for (num, body) in objects {
        entries.insert(*num, Some(out.len()));
        out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    for f in frees {
        entries.insert(*f, None);
    }
    for (num, at) in moved {
        entries.insert(*num, Some(*at));
    }
    let size = entries.keys().copied().max().unwrap_or(0).max(doc.max_id) + 1;
    let xref = out.len();
    out.extend_from_slice(b"xref\n");
    for (num, e) in &entries {
        let line = match e {
            Some(at) => format!("{num} 1\n{at:010} 00000 n \n"),
            None => format!("{num} 1\n0000000000 00001 f \n"),
        };
        out.extend_from_slice(line.as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {size} /Root {} {} R /Prev {prev} {trailer} >>\nstartxref\n{xref}\n%%EOF\n",
            root.0, root.1
        )
        .as_bytes(),
    );
    out
}

/// The object id of the signature dictionary of the field named `name`.
pub fn sig_id(bytes: &[u8], name: &str) -> lopdf::ObjectId {
    let doc = Document::load_mem(bytes).unwrap();
    doc.objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .find(|d| {
            d.get(b"T")
                .and_then(Object::as_str)
                .is_ok_and(|t| t == name.as_bytes())
        })
        .and_then(|d| d.get(b"V").ok()?.as_reference().ok())
        .unwrap()
}

// ------------------------------------------------------------------ hidden objects

/// How [`hidden_payment`] hides object 5 from PDF apps following the specification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hide {
    /// Not hidden: every app shows "Pay one hundred".
    Not,
    /// A later revision's cross-reference table frees it.
    TableFree,
    /// A later revision's cross-reference stream frees it.
    StreamFree,
    /// It is only in an object stream, and its entry is free.
    StrayFree,
    /// It is only in an object stream, with no entry at all.
    StrayUnlisted,
    /// The other way round: only the stream of a hybrid revision that isn't the newest
    /// lists it, which those apps read and lopdf doesn't.
    OlderHybrid,
}

pub const HIDES: [Hide; 5] = [
    Hide::TableFree,
    Hide::StreamFree,
    Hide::StrayFree,
    Hide::StrayUnlisted,
    Hide::OlderHybrid,
];

/// A one-page contract with an empty signature field "Sign here". The page says "Pay one
/// million"; object 5, its XObject resources, names a form that covers that with "Pay one
/// hundred". `hide` makes apps following the specification read object 5 as deleted or
/// missing, so they show "Pay one million" while lopdf on its own shows "Pay one
/// hundred". With [`Hide::OlderHybrid`] those apps show "Pay one hundred" and lopdf misses
/// object 5.
pub fn hidden_payment(hide: Hide) -> Vec<u8> {
    let mut w = Writer::new();
    w.obj(
        1,
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [7 0 R] >> >>",
    );
    w.obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    w.obj(
        3,
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 \
         6 0 R >> /XObject 5 0 R >> /Contents 4 0 R /Annots [7 0 R] >>",
    );
    w.stream(
        4,
        "",
        b"BT /F1 12 Tf 20 150 Td (Pay one million) Tj ET q /Cover Do Q",
    );
    let cover = "<< /Cover 8 0 R >>";
    let stray = matches!(hide, Hide::StrayFree | Hide::StrayUnlisted);
    if stray {
        let head = "5 0 ";
        w.stream(
            9,
            &format!("/Type /ObjStm /N 1 /First {}", head.len()),
            format!("{head}{cover}").as_bytes(),
        );
    } else {
        w.obj(5, cover);
    }
    w.obj(6, FONT);
    w.obj(
        7,
        "<< /FT /Sig /T (Sign here) /Type /Annot /Subtype /Widget /Rect [20 20 220 70] /P 3 \
         0 R /F 4 >>",
    );
    w.stream(
        8,
        "/Type /XObject /Subtype /Form /BBox [0 0 300 200] /Resources << /Font << /F1 6 0 R \
         >> >>",
        b"1 1 1 rg 0 140 300 30 re f 0 0 0 rg BT /F1 12 Tf 20 120 Td (Pay one hundred) Tj ET",
    );
    let root = "/Root 1 0 R";
    match hide {
        Hide::Not | Hide::TableFree => {
            let rows = w.rows(&[1, 2, 3, 4, 5, 6, 7, 8]);
            w.table(&rows, root);
            if hide == Hide::Not {
                return w.out;
            }
            append_raw(&w.out, &[], &[5], &[], "")
        }
        Hide::StreamFree => {
            let mut rows = w.rows(&[1, 2, 3, 4, 5, 6, 7, 8]);
            rows.push((0, Row::Free(65535)));
            rows.push((10, Row::At(w.out.len())));
            let first = w.xref_stream(10, &rows, &format!("{root} /Size 11"));
            let rows = [(5, Row::Free(1)), (11, Row::At(w.out.len()))];
            w.xref_stream(11, &rows, &format!("{root} /Size 12 /Prev {first}"));
            w.out
        }
        Hide::StrayFree | Hide::StrayUnlisted => {
            let mut rows = w.rows(&[1, 2, 3, 4, 6, 7, 8, 9]);
            if hide == Hide::StrayFree {
                rows.push((5, Row::Free(1)));
            }
            w.table(&rows, root);
            w.out
        }
        Hide::OlderHybrid => {
            let stm = w.out.len();
            let five = [(5, Row::At(w.at[&5]))];
            w.xref_stream_object(10, &five, "/Size 11");
            let mut rows = w.rows(&[1, 2, 3, 4, 6, 7, 8]);
            rows.push((10, Row::At(stm)));
            w.table(&rows, &format!("{root} /XRefStm {stm}"));
            append_raw(&w.out, &[(6, FONT.as_bytes())], &[], &[], "")
        }
    }
}

const FONT: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";

/// A cross-reference entry for [`Writer`].
#[derive(Clone, Copy)]
pub enum Row {
    /// Free, with the generation a reuse would take.
    Free(u16),
    At(usize),
}

/// Writes a PDF by hand, one object at a time.
struct Writer {
    out: Vec<u8>,
    at: std::collections::BTreeMap<u32, usize>,
}

impl Writer {
    fn new() -> Self {
        Self {
            out: b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec(),
            at: Default::default(),
        }
    }

    fn obj(&mut self, num: u32, body: &str) {
        self.at.insert(num, self.out.len());
        self.out
            .extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
    }

    fn stream(&mut self, num: u32, dict: &str, data: &[u8]) {
        self.at.insert(num, self.out.len());
        let head = format!("{num} 0 obj\n<< {dict} /Length {} >>\nstream\n", data.len());
        self.out.extend_from_slice(head.as_bytes());
        self.out.extend_from_slice(data);
        self.out.extend_from_slice(b"\nendstream\nendobj\n");
    }

    /// Entries for objects already written.
    fn rows(&self, nums: &[u32]) -> Vec<(u32, Row)> {
        nums.iter().map(|n| (*n, Row::At(self.at[n]))).collect()
    }

    fn finish(&mut self, xref: usize) {
        self.out
            .extend_from_slice(format!("startxref\n{xref}\n%%EOF\n").as_bytes());
    }

    /// A cross-reference table (with the head of the free list) and its trailer.
    fn table(&mut self, rows: &[(u32, Row)], trailer: &str) {
        let mut rows = rows.to_vec();
        rows.push((0, Row::Free(65535)));
        rows.sort_by_key(|r| r.0);
        let size = rows.last().unwrap().0 + 1;
        let xref = self.out.len();
        self.out.extend_from_slice(b"xref\n");
        let mut i = 0;
        while i < rows.len() {
            let mut j = i + 1;
            while j < rows.len() && rows[j].0 == rows[j - 1].0 + 1 {
                j += 1;
            }
            self.out
                .extend_from_slice(format!("{} {}\n", rows[i].0, j - i).as_bytes());
            for (_, r) in &rows[i..j] {
                let line = match r {
                    Row::Free(g) => format!("0000000000 {g:05} f \n"),
                    Row::At(at) => format!("{at:010} 00000 n \n"),
                };
                self.out.extend_from_slice(line.as_bytes());
            }
            i = j;
        }
        self.out
            .extend_from_slice(format!("trailer\n<< /Size {size} {trailer} >>\n").as_bytes());
        self.finish(xref);
    }

    /// A cross-reference stream as a section of its own; returns where it starts.
    fn xref_stream(&mut self, num: u32, rows: &[(u32, Row)], dict: &str) -> usize {
        let at = self.out.len();
        self.xref_stream_object(num, rows, dict);
        self.finish(at);
        at
    }

    /// A cross-reference stream object (`/W [1 4 2]`, uncompressed).
    fn xref_stream_object(&mut self, num: u32, rows: &[(u32, Row)], dict: &str) {
        let mut rows = rows.to_vec();
        rows.sort_by_key(|r| r.0);
        let mut index = Vec::new();
        let mut data = Vec::new();
        for (k, (n, r)) in rows.iter().enumerate() {
            if k == 0 || *n != rows[k - 1].0 + 1 {
                index.extend([*n, 0]);
            }
            *index.last_mut().unwrap() += 1;
            let (kind, f1, f2) = match r {
                Row::Free(g) => (0u8, 0u32, *g),
                Row::At(at) => (1, u32::try_from(*at).unwrap(), 0),
            };
            data.push(kind);
            data.extend_from_slice(&f1.to_be_bytes());
            data.extend_from_slice(&f2.to_be_bytes());
        }
        let index: Vec<String> = index.iter().map(u32::to_string).collect();
        self.stream(
            num,
            &format!("/Type /XRef /W [1 4 2] /Index [{}] {dict}", index.join(" ")),
            &data,
        );
    }
}
