//! PKCS#12 files (RFC 7292): the `.p12` / `.pfx` files a digital ID is kept in, a private
//! key with its certificates, protected by a password. Acrobat, Windows, macOS and
//! certificate authorities all use them. See docs/signatures.md, "Digital IDs".
//!
//! Read: the integrity check (MAC) by HMAC with SHA-1, SHA-224, SHA-256, SHA-384 or
//! SHA-512 over the PKCS#12 key derivation, or by PBMAC1 (RFC 9579); encryption by PBES2
//! (PBKDF2 with AES-CBC or 3DES-CBC) or the legacy PKCS#12 schemes (3DES, RC2); BER as
//! Windows writes it. A file without an integrity check, or protected by a public key
//! instead of a password, is refused.
//!
//! Write: PBES2 with PBKDF2-HMAC-SHA256 at 600,000 iterations and AES-256-CBC for the key
//! (and for the certificates when asked), and an HMAC-SHA256 integrity check over 1,200,000
//! iterations of the PKCS#12 derivation. One derivation step there is one SHA-256 block
//! where a PBKDF2 step is two, so guessing the password through the integrity check costs
//! as much as through the key.
//!
//! Keys, passwords and everything derived from them are wiped from memory after use.

use crate::asn1::{self, ctx, ctx_primitive, der, der_octets, der_oid, der_seq, der_uint, Tlv};
use crate::Result;
use anyhow::{anyhow, bail};
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, BlockModeEncrypt, InnerIvInit, KeyIvInit};
use der::asn1::ObjectIdentifier as Oid;
use getrandom::rand_core::TryRng;
use sha2::Digest;
use zeroize::{Zeroize, Zeroizing};

mod oid {
    use der::asn1::ObjectIdentifier as Oid;

    pub const DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.1");
    pub const SIGNED_DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.2");
    pub const ENVELOPED_DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.3");
    pub const ENCRYPTED_DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.6");

    pub const KEY_BAG: Oid = Oid::new_unwrap("1.2.840.113549.1.12.10.1.1");
    pub const SHROUDED_KEY_BAG: Oid = Oid::new_unwrap("1.2.840.113549.1.12.10.1.2");
    pub const CERT_BAG: Oid = Oid::new_unwrap("1.2.840.113549.1.12.10.1.3");
    pub const SAFE_CONTENTS_BAG: Oid = Oid::new_unwrap("1.2.840.113549.1.12.10.1.6");
    pub const X509_CERTIFICATE: Oid = Oid::new_unwrap("1.2.840.113549.1.9.22.1");
    pub const FRIENDLY_NAME: Oid = Oid::new_unwrap("1.2.840.113549.1.9.20");
    pub const LOCAL_KEY_ID: Oid = Oid::new_unwrap("1.2.840.113549.1.9.21");

    pub const PBE_SHA1_3DES: Oid = Oid::new_unwrap("1.2.840.113549.1.12.1.3");
    pub const PBE_SHA1_2DES: Oid = Oid::new_unwrap("1.2.840.113549.1.12.1.4");
    pub const PBE_SHA1_RC2_128: Oid = Oid::new_unwrap("1.2.840.113549.1.12.1.5");
    pub const PBE_SHA1_RC2_40: Oid = Oid::new_unwrap("1.2.840.113549.1.12.1.6");
    pub const PBES2: Oid = Oid::new_unwrap("1.2.840.113549.1.5.13");
    pub const PBKDF2: Oid = Oid::new_unwrap("1.2.840.113549.1.5.12");
    pub const PBMAC1: Oid = Oid::new_unwrap("1.2.840.113549.1.5.14");

    pub const SHA1: Oid = Oid::new_unwrap("1.3.14.3.2.26");
    pub const SHA224: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.4");
    pub const SHA256: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.1");
    pub const SHA384: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.2");
    pub const SHA512: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.3");
    pub const HMAC_SHA1: Oid = Oid::new_unwrap("1.2.840.113549.2.7");
    pub const HMAC_SHA224: Oid = Oid::new_unwrap("1.2.840.113549.2.8");
    pub const HMAC_SHA256: Oid = Oid::new_unwrap("1.2.840.113549.2.9");
    pub const HMAC_SHA384: Oid = Oid::new_unwrap("1.2.840.113549.2.10");
    pub const HMAC_SHA512: Oid = Oid::new_unwrap("1.2.840.113549.2.11");

    pub const AES128_CBC: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.1.2");
    pub const AES192_CBC: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.1.22");
    pub const AES256_CBC: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.1.42");
    pub const DES_EDE3_CBC: Oid = Oid::new_unwrap("1.2.840.113549.3.7");
}

/// The password is wrong, or the file was changed or damaged: PKCS#12 can't tell which.
#[derive(Debug, thiserror::Error)]
#[error("the password is wrong, or the file is damaged")]
pub struct WrongPassword;

/// What a PKCS#12 file holds.
pub(crate) struct Contents {
    /// The private keys (PKCS#8), wiped when dropped.
    pub keys: Vec<Zeroizing<Vec<u8>>>,
    /// The certificates as written.
    pub certs: Vec<Vec<u8>>,
    /// The name the file gives the ID, if any.
    pub friendly_name: Option<String>,
}

/// The largest file read: real ones are a few kilobytes.
const MAX_FILE: usize = 4 << 20;
/// The most items (bags) a file may hold.
const MAX_BAGS: usize = 256;
/// The most key-derivation iterations reading one file may take, all derivations together,
/// so a crafted file can't keep LimarePDF busy for minutes. LimarePDF's own files take 2,400,000.
const MAX_ITERATIONS: u64 = 20_000_000;
/// How deeply bags may hold other bags.
const MAX_NESTING: usize = 4;

/// PBKDF2 iterations for the key and certificates LimarePDF writes (OWASP's figure for
/// PBKDF2-HMAC-SHA256).
pub(crate) const KEY_ITERATIONS: u32 = 600_000;
/// PKCS#12 derivation iterations for the integrity check LimarePDF writes: twice the PBKDF2
/// figure, as each step costs half as much.
pub(crate) const MAC_ITERATIONS: u32 = 1_200_000;

fn damaged() -> anyhow::Error {
    anyhow!("the file is damaged: it passed its integrity check but can't be decrypted")
}

// ------------------------------------------------------------------ hashes

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hash {
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    fn from_digest_oid(o: Oid) -> Option<Self> {
        Some(match o {
            oid::SHA1 => Self::Sha1,
            oid::SHA224 => Self::Sha224,
            oid::SHA256 => Self::Sha256,
            oid::SHA384 => Self::Sha384,
            oid::SHA512 => Self::Sha512,
            _ => return None,
        })
    }

    fn from_hmac_oid(o: Oid) -> Option<Self> {
        Some(match o {
            oid::HMAC_SHA1 => Self::Sha1,
            oid::HMAC_SHA224 => Self::Sha224,
            oid::HMAC_SHA256 => Self::Sha256,
            oid::HMAC_SHA384 => Self::Sha384,
            oid::HMAC_SHA512 => Self::Sha512,
            _ => return None,
        })
    }

    fn output_len(self) -> usize {
        match self {
            Self::Sha1 => 20,
            Self::Sha224 => 28,
            Self::Sha256 => 32,
            Self::Sha384 => 48,
            Self::Sha512 => 64,
        }
    }

    fn hmac(self, key: &[u8], data: &[u8]) -> Vec<u8> {
        fn run<M: hmac::Mac + hmac::KeyInit>(key: &[u8], data: &[u8]) -> Vec<u8> {
            let mut m = <M as hmac::KeyInit>::new_from_slice(key).expect("HMAC takes any key");
            m.update(data);
            m.finalize().into_bytes().to_vec()
        }
        use hmac::Hmac;
        match self {
            Self::Sha1 => run::<Hmac<sha1::Sha1>>(key, data),
            Self::Sha224 => run::<Hmac<sha2::Sha224>>(key, data),
            Self::Sha256 => run::<Hmac<sha2::Sha256>>(key, data),
            Self::Sha384 => run::<Hmac<sha2::Sha384>>(key, data),
            Self::Sha512 => run::<Hmac<sha2::Sha512>>(key, data),
        }
    }

    /// PBKDF2 with HMAC over this hash (RFC 8018), `n` bytes.
    fn pbkdf2(self, password: &[u8], salt: &[u8], iterations: u32, n: usize) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(vec![0; n]);
        let o = out.as_mut_slice();
        match self {
            Self::Sha1 => pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, salt, iterations, o),
            Self::Sha224 => pbkdf2::pbkdf2_hmac::<sha2::Sha224>(password, salt, iterations, o),
            Self::Sha256 => pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password, salt, iterations, o),
            Self::Sha384 => pbkdf2::pbkdf2_hmac::<sha2::Sha384>(password, salt, iterations, o),
            Self::Sha512 => pbkdf2::pbkdf2_hmac::<sha2::Sha512>(password, salt, iterations, o),
        }
        out
    }

    /// The PKCS#12 key derivation over this hash, `n` bytes.
    fn p12_kdf(
        self,
        password: &[u8],
        salt: &[u8],
        id: u8,
        iterations: u32,
        n: usize,
    ) -> Zeroizing<Vec<u8>> {
        match self {
            Self::Sha1 => p12_kdf::<sha1::Sha1>(64, password, salt, id, iterations, n),
            Self::Sha224 => p12_kdf::<sha2::Sha224>(64, password, salt, id, iterations, n),
            Self::Sha256 => p12_kdf::<sha2::Sha256>(64, password, salt, id, iterations, n),
            Self::Sha384 => p12_kdf::<sha2::Sha384>(128, password, salt, id, iterations, n),
            Self::Sha512 => p12_kdf::<sha2::Sha512>(128, password, salt, id, iterations, n),
        }
    }
}

/// Purposes of the PKCS#12 key derivation.
const KDF_KEY: u8 = 1;
const KDF_IV: u8 = 2;
const KDF_MAC: u8 = 3;

/// The PKCS#12 key derivation (RFC 7292 appendix B.2): `n` bytes for purpose `id` from
/// the password (in its BMPString form) and salt. `v` is the hash's block size in bytes.
fn p12_kdf<D: Digest>(
    v: usize,
    password: &[u8],
    salt: &[u8],
    id: u8,
    iterations: u32,
    n: usize,
) -> Zeroizing<Vec<u8>> {
    let u = <D as Digest>::output_size();
    // I: the salt, then the password, each repeated to a whole number of blocks.
    let stretched = |len: usize| v * len.div_ceil(v);
    let (s_len, p_len) = (stretched(salt.len()), stretched(password.len()));
    let mut i = Zeroizing::new(Vec::with_capacity(s_len + p_len));
    i.extend(salt.iter().cycle().take(s_len));
    i.extend(password.iter().cycle().take(p_len));
    let d = vec![id; v];
    let mut out = Zeroizing::new(Vec::with_capacity(n + u));
    let mut b = Zeroizing::new(vec![0u8; v]);
    loop {
        let mut a = D::new_with_prefix(&d).chain_update(&*i).finalize();
        for _ in 1..iterations {
            let next = D::digest(&a);
            a.as_mut_slice().zeroize();
            a = next;
        }
        let take = (n - out.len()).min(u);
        out.extend_from_slice(&a[..take]);
        for (k, x) in b.iter_mut().enumerate() {
            *x = a[k % u];
        }
        a.as_mut_slice().zeroize();
        if out.len() >= n {
            return out;
        }
        // Each block of I becomes (I_j + B + 1) mod 2^(8v).
        for block in i.chunks_mut(v) {
            let mut carry = 1u16;
            for k in (0..v).rev() {
                let s = u16::from(block[k]) + u16::from(b[k]) + carry;
                block[k] = s as u8;
                carry = s >> 8;
            }
        }
    }
}

/// The password as the PKCS#12 derivation takes it: BMPString (UTF-16, big-endian) with a
/// two-byte terminator.
fn bmp_password(password: &str) -> Zeroizing<Vec<u8>> {
    let mut out = Zeroizing::new(Vec::with_capacity(password.len() * 4 + 2));
    for unit in password.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    out
}

/// Iterations a file still may use, all derivations together.
struct Budget(u64);

impl Budget {
    /// An iteration count from the file, counted against the budget.
    fn take(&mut self, t: &Tlv) -> Result<u32> {
        let n = t.uint()?;
        if n == 0 {
            bail!("an iteration count of 0 isn't allowed");
        }
        if n > self.0 {
            bail!("the file asks for more password stretching than LimarePDF allows");
        }
        self.0 -= n;
        Ok(n as u32)
    }
}

/// An algorithm identifier: its OID and parameters (none when absent or NULL).
fn alg<'a>(t: &Tlv<'a>) -> Result<(Oid, Option<Tlv<'a>>)> {
    let kids = t.expect(asn1::SEQUENCE, "algorithm")?.children()?;
    match &kids[..] {
        [o] => Ok((o.oid()?, None)),
        [o, p] => Ok((o.oid()?, (p.tag != asn1::NULL).then_some(*p))),
        _ => bail!("an algorithm has {} parts", kids.len()),
    }
}

// ------------------------------------------------------------------ reading

/// Reads a PKCS#12 file with `password`. A wrong password (or a changed file) gives
/// [`WrongPassword`].
pub(crate) fn read(file: &[u8], password: &str) -> Result<Contents> {
    let not_p12 = || anyhow!("the file isn't a digital ID (a PKCS#12 .p12 or .pfx file)");
    if file.len() > MAX_FILE {
        bail!("the file is too large to be a digital ID");
    }
    let pfx = Tlv::read_all(file).map_err(|_| not_p12())?;
    if pfx.tag != asn1::SEQUENCE {
        return Err(not_p12());
    }
    let kids = pfx.children().map_err(|_| not_p12())?;
    let (version, auth_safe, mac) = match &kids[..] {
        [v, a] => (v, a, None),
        [v, a, m] => (v, a, Some(m)),
        _ => return Err(not_p12()),
    };
    if version.uint().map_err(|_| not_p12())? != 3 {
        bail!("this version of PKCS#12 isn't supported");
    }
    let (kind, content) = content_info(auth_safe).map_err(|_| not_p12())?;
    match kind {
        oid::DATA => {}
        oid::SIGNED_DATA => bail!(
            "the file is protected by a public key rather than a password, which LimarePDF doesn't \
             support"
        ),
        _ => return Err(not_p12()),
    }
    let auth = content.octets()?;
    let mac = mac.ok_or_else(|| {
        anyhow!("the file has no integrity check (MAC), so LimarePDF doesn't use it")
    })?;
    let mut budget = Budget(MAX_ITERATIONS);
    let bmp = check_mac(mac, &auth, password, &mut budget)?;

    let mut out = Contents {
        keys: Vec::new(),
        certs: Vec::new(),
        friendly_name: None,
    };
    let mut bags = 0;
    let safes = Tlv::read_all(&auth)?.expect(asn1::SEQUENCE, "contents")?;
    for safe in safes.children()? {
        let (kind, content) = content_info(&safe)?;
        let plain: Zeroizing<Vec<u8>> = match kind {
            oid::DATA => Zeroizing::new(content.octets()?),
            oid::ENCRYPTED_DATA => decrypt_data(&content, password, &bmp, &mut budget)?,
            oid::ENVELOPED_DATA => bail!(
                "part of the file is encrypted for a public key rather than with a password, \
                 which LimarePDF doesn't support"
            ),
            o => bail!("the file holds a kind of content LimarePDF doesn't know ({o})"),
        };
        read_bags(&plain, password, &bmp, &mut budget, &mut out, &mut bags, 0)?;
    }
    Ok(out)
}

/// A ContentInfo: its type and its content (inside the `[0] EXPLICIT`).
fn content_info<'a>(t: &Tlv<'a>) -> Result<(Oid, Tlv<'a>)> {
    let kids = t.expect(asn1::SEQUENCE, "content")?.children()?;
    match &kids[..] {
        [o, c] if c.tag == ctx(0) => Ok((o.oid()?, Tlv::read_all(c.body)?)),
        _ => bail!("malformed content"),
    }
}

/// Checks the integrity check (MAC) over `data`; returns the password's BMPString form
/// that passed, for the legacy encryptions.
fn check_mac(
    mac: &Tlv,
    data: &[u8],
    password: &str,
    budget: &mut Budget,
) -> Result<Zeroizing<Vec<u8>>> {
    use subtle::ConstantTimeEq;
    let kids = mac.expect(asn1::SEQUENCE, "integrity check")?.children()?;
    let (digest_info, salt, iterations) = match &kids[..] {
        [d, s] => (d, s, None),
        [d, s, i] => (d, s, Some(i)),
        _ => bail!("malformed integrity check"),
    };
    let di = digest_info
        .expect(asn1::SEQUENCE, "integrity check")?
        .children()?;
    let [algorithm, expected] = &di[..] else {
        bail!("malformed integrity check");
    };
    let expected = expected.octets()?;
    let (alg_oid, params) = alg(algorithm)?;
    if alg_oid == oid::PBMAC1 {
        // RFC 9579: PBKDF2 over the password as UTF-8 gives the HMAC key; the salt and
        // iterations fields of the integrity check aren't used.
        let p = params.ok_or_else(|| anyhow!("PBMAC1 without parameters"))?;
        let parts = p.expect(asn1::SEQUENCE, "PBMAC1")?.children()?;
        let [kdf, scheme] = &parts[..] else {
            bail!("malformed PBMAC1 parameters");
        };
        let kdf = Pbkdf2::parse(kdf, budget)?;
        let (scheme, _) = alg(scheme)?;
        let hash = Hash::from_hmac_oid(scheme).ok_or_else(|| {
            anyhow!("the file's integrity check uses an unsupported HMAC ({scheme})")
        })?;
        let len = kdf
            .key_len
            .ok_or_else(|| anyhow!("PBMAC1 without a key length"))?;
        if !(20..=64).contains(&len) {
            bail!("PBMAC1 with an unusable key length ({len})");
        }
        let key = kdf.derive(password.as_bytes(), len);
        if !bool::from(hash.hmac(&key, data).ct_eq(&expected)) {
            return Err(WrongPassword.into());
        }
        return Ok(bmp_password(password));
    }
    let hash = Hash::from_digest_oid(alg_oid).ok_or_else(|| {
        anyhow!("the file's integrity check uses an unsupported digest ({alg_oid})")
    })?;
    let salt = salt.octets()?;
    let count = iterations;
    let iterations = match count {
        Some(i) => budget.take(i)?,
        None => 1,
    };
    // An empty password has two forms in use: just the terminator, or nothing at all.
    // Trying both costs twice.
    let mut forms = vec![bmp_password(password)];
    if password.is_empty() {
        if let Some(i) = count {
            budget.take(i)?;
        }
        forms.push(Zeroizing::new(Vec::new()));
    }
    for form in forms {
        let key = hash.p12_kdf(&form, &salt, KDF_MAC, iterations, hash.output_len());
        if bool::from(hash.hmac(&key, data).ct_eq(&expected)) {
            return Ok(form);
        }
    }
    Err(WrongPassword.into())
}

/// PBKDF2 parameters (RFC 8018 appendix A.2).
struct Pbkdf2 {
    salt: Vec<u8>,
    iterations: u32,
    key_len: Option<usize>,
    prf: Hash,
}

impl Pbkdf2 {
    /// From an algorithm identifier naming PBKDF2.
    fn parse(t: &Tlv, budget: &mut Budget) -> Result<Self> {
        let (o, params) = alg(t)?;
        if o != oid::PBKDF2 {
            bail!("the file uses a password function LimarePDF doesn't support ({o})");
        }
        let p = params.ok_or_else(|| anyhow!("PBKDF2 without parameters"))?;
        let kids = p.expect(asn1::SEQUENCE, "PBKDF2")?.children()?;
        let mut it = kids.iter();
        let salt = it
            .next()
            .ok_or_else(|| anyhow!("PBKDF2 without a salt"))?
            .octets()
            .map_err(|_| anyhow!("PBKDF2 with an unsupported kind of salt"))?;
        let iterations = it
            .next()
            .ok_or_else(|| anyhow!("PBKDF2 without iterations"))?;
        let iterations = budget.take(iterations)?;
        let mut key_len = None;
        let mut prf = Hash::Sha1;
        for k in it {
            match k.tag {
                asn1::INTEGER if key_len.is_none() => {
                    key_len = Some(usize::try_from(k.uint()?).unwrap_or(usize::MAX))
                }
                asn1::SEQUENCE => {
                    let (o, _) = alg(k)?;
                    prf = Hash::from_hmac_oid(o).ok_or_else(|| {
                        anyhow!("PBKDF2 with a function LimarePDF doesn't support ({o})")
                    })?;
                }
                _ => bail!("malformed PBKDF2 parameters"),
            }
        }
        Ok(Self {
            salt,
            iterations,
            key_len,
            prf,
        })
    }

    fn derive(&self, password: &[u8], n: usize) -> Zeroizing<Vec<u8>> {
        self.prf.pbkdf2(password, &self.salt, self.iterations, n)
    }
}

/// An EncryptedData content: decrypts it.
fn decrypt_data(
    t: &Tlv,
    password: &str,
    bmp: &[u8],
    budget: &mut Budget,
) -> Result<Zeroizing<Vec<u8>>> {
    let kids = t.expect(asn1::SEQUENCE, "encrypted data")?.children()?;
    let eci = kids
        .get(1)
        .ok_or_else(|| anyhow!("malformed encrypted data"))?
        .expect(asn1::SEQUENCE, "encrypted data")?
        .children()?;
    let (algorithm, content) = match &eci[..] {
        [_, a, c, ..] => (a, c),
        _ => bail!("encrypted data without content"),
    };
    // `[0] IMPLICIT OCTET STRING`: whole, or split into pieces (BER).
    let data = match content.tag {
        t if t == ctx_primitive(0) => content.body.to_vec(),
        t if t == ctx(0) => {
            let mut out = Vec::new();
            for piece in content.children()? {
                out.extend(piece.octets()?);
            }
            out
        }
        _ => bail!("malformed encrypted data"),
    };
    decrypt(algorithm, &data, password, bmp, budget)
}

/// Decrypts `data` encrypted as `algorithm` says.
fn decrypt(
    algorithm: &Tlv,
    data: &[u8],
    password: &str,
    bmp: &[u8],
    budget: &mut Budget,
) -> Result<Zeroizing<Vec<u8>>> {
    let (o, params) = alg(algorithm)?;
    let mut buf = Zeroizing::new(data.to_vec());
    let len = if o == oid::PBES2 {
        let p = params.ok_or_else(|| anyhow!("PBES2 without parameters"))?;
        let kids = p.expect(asn1::SEQUENCE, "PBES2")?.children()?;
        let [kdf, scheme] = &kids[..] else {
            bail!("malformed PBES2 parameters");
        };
        let kdf = Pbkdf2::parse(kdf, budget)?;
        let (cipher, iv) = alg(scheme)?;
        let iv = iv
            .ok_or_else(|| anyhow!("encryption without an IV"))?
            .octets()?;
        let key_len = match cipher {
            oid::AES128_CBC => 16,
            oid::AES192_CBC => 24,
            oid::AES256_CBC => 32,
            oid::DES_EDE3_CBC => 24,
            o => bail!("the file uses an encryption LimarePDF doesn't support ({o})"),
        };
        if kdf.key_len.is_some_and(|l| l != key_len) {
            bail!("malformed PBES2 parameters (key length)");
        }
        // PBES2 takes the password as UTF-8 (what OpenSSL and Windows do).
        let key = kdf.derive(password.as_bytes(), key_len);
        let r = match cipher {
            oid::AES128_CBC => cbc::Decryptor::<aes::Aes128>::new_from_slices(&key, &iv)
                .map(|d| d.decrypt_padded::<Pkcs7>(&mut buf).map(<[u8]>::len)),
            oid::AES192_CBC => cbc::Decryptor::<aes::Aes192>::new_from_slices(&key, &iv)
                .map(|d| d.decrypt_padded::<Pkcs7>(&mut buf).map(<[u8]>::len)),
            oid::AES256_CBC => cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, &iv)
                .map(|d| d.decrypt_padded::<Pkcs7>(&mut buf).map(<[u8]>::len)),
            _ => cbc::Decryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
                .map(|d| d.decrypt_padded::<Pkcs7>(&mut buf).map(<[u8]>::len)),
        };
        r.map_err(|_| anyhow!("malformed encryption parameters (IV)"))?
            .map_err(|_| damaged())?
    } else {
        // The legacy PKCS#12 schemes: SHA-1 derivation, then 3DES or RC2.
        let p = params.ok_or_else(|| anyhow!("encryption without parameters"))?;
        let kids = p
            .expect(asn1::SEQUENCE, "encryption parameters")?
            .children()?;
        let [salt, iterations] = &kids[..] else {
            bail!("malformed encryption parameters");
        };
        let salt = salt.octets()?;
        let iterations = budget.take(iterations)?;
        let key_len = match o {
            oid::PBE_SHA1_3DES => 24,
            oid::PBE_SHA1_2DES | oid::PBE_SHA1_RC2_128 => 16,
            oid::PBE_SHA1_RC2_40 => 5,
            o => bail!("the file uses an encryption LimarePDF doesn't support ({o})"),
        };
        let key = Hash::Sha1.p12_kdf(bmp, &salt, KDF_KEY, iterations, key_len);
        let iv = Hash::Sha1.p12_kdf(bmp, &salt, KDF_IV, iterations, 8);
        let r = match o {
            oid::PBE_SHA1_3DES => cbc::Decryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
                .map(|d| d.decrypt_padded::<Pkcs7>(&mut buf).map(<[u8]>::len)),
            oid::PBE_SHA1_2DES => cbc::Decryptor::<des::TdesEde2>::new_from_slices(&key, &iv)
                .map(|d| d.decrypt_padded::<Pkcs7>(&mut buf).map(<[u8]>::len)),
            _ => {
                let rc2 = rc2::Rc2::new_with_eff_key_len(&key, key_len * 8);
                let iv: &[u8; 8] = iv.as_slice().try_into().expect("8 bytes");
                Ok(cbc::Decryptor::<rc2::Rc2>::inner_iv_init(rc2, iv.into())
                    .decrypt_padded::<Pkcs7>(&mut buf)
                    .map(<[u8]>::len))
            }
        };
        r.map_err(|_| anyhow!("malformed encryption parameters"))?
            .map_err(|_| damaged())?
    };
    buf.truncate(len);
    Ok(buf)
}

/// Reads the bags in a SafeContents.
fn read_bags(
    safe: &[u8],
    password: &str,
    bmp: &[u8],
    budget: &mut Budget,
    out: &mut Contents,
    count: &mut usize,
    depth: usize,
) -> Result<()> {
    if depth > MAX_NESTING {
        bail!("the file nests its contents too deeply");
    }
    let list = Tlv::read_all(safe)
        .and_then(|t| t.expect(asn1::SEQUENCE, "contents"))
        .map_err(|_| damaged())?;
    for bag in list.children()? {
        *count += 1;
        if *count > MAX_BAGS {
            bail!("the file holds too many items to be a digital ID");
        }
        let kids = bag.expect(asn1::SEQUENCE, "item")?.children()?;
        let (id, value, attrs) = match &kids[..] {
            [i, v] => (i, v, None),
            [i, v, a] => (i, v, Some(a)),
            _ => bail!("malformed item"),
        };
        if value.tag != ctx(0) {
            bail!("malformed item");
        }
        let value = Tlv::read_all(value.body)?;
        let name = attrs.map(friendly_name).transpose()?.flatten();
        match id.oid()? {
            oid::CERT_BAG => {
                let parts = value.expect(asn1::SEQUENCE, "certificate")?.children()?;
                let [kind, cert] = &parts[..] else {
                    bail!("malformed certificate item");
                };
                // Other kinds of certificate (SDSI) aren't used for signing.
                if kind.oid()? == oid::X509_CERTIFICATE && cert.tag == ctx(0) {
                    out.certs.push(Tlv::read_all(cert.body)?.octets()?);
                    if out.friendly_name.is_none() {
                        out.friendly_name = name;
                    }
                }
            }
            oid::SHROUDED_KEY_BAG => {
                let parts = value.expect(asn1::SEQUENCE, "key")?.children()?;
                let [algorithm, data] = &parts[..] else {
                    bail!("malformed key item");
                };
                out.keys
                    .push(decrypt(algorithm, &data.octets()?, password, bmp, budget)?);
                if name.is_some() {
                    out.friendly_name = name;
                }
            }
            oid::KEY_BAG => {
                out.keys.push(Zeroizing::new(value.raw.to_vec()));
                if name.is_some() {
                    out.friendly_name = name;
                }
            }
            oid::SAFE_CONTENTS_BAG => {
                read_bags(value.raw, password, bmp, budget, out, count, depth + 1)?
            }
            // Revocation lists and secrets aren't needed.
            _ => {}
        }
    }
    Ok(())
}

/// The friendly name among an item's attributes.
fn friendly_name(attrs: &Tlv) -> Result<Option<String>> {
    for a in attrs.expect(asn1::SET, "attributes")?.children()? {
        let parts = a.expect(asn1::SEQUENCE, "attribute")?.children()?;
        if let [o, values] = &parts[..] {
            if o.oid()? == oid::FRIENDLY_NAME {
                let first = values.children()?.into_iter().next();
                return Ok(first.and_then(|v| v.text()).map(|s| s.trim().to_string()));
            }
        }
    }
    Ok(None)
}

/// The certificates in the parts of `file` that aren't encrypted, read without the
/// password and so without the integrity check: for showing a list only. The certificate
/// marked as the key's comes first. Also the friendly name.
pub(crate) fn peek(file: &[u8]) -> Result<(Vec<Vec<u8>>, Option<String>)> {
    if file.len() > MAX_FILE {
        bail!("the file is too large to be a digital ID");
    }
    let pfx = Tlv::read_all(file)?.expect(asn1::SEQUENCE, "PKCS#12")?;
    let kids = pfx.children()?;
    let auth = content_info(kids.get(1).ok_or_else(|| anyhow!("not a PKCS#12 file"))?)?;
    if auth.0 != oid::DATA {
        bail!("not a password-protected PKCS#12 file");
    }
    let auth = auth.1.octets()?;
    let mut certs: Vec<(bool, Vec<u8>)> = Vec::new();
    let mut name = None;
    let safes = Tlv::read_all(&auth)?.expect(asn1::SEQUENCE, "contents")?;
    for safe in safes.children()? {
        let (kind, content) = content_info(&safe)?;
        if kind != oid::DATA {
            continue;
        }
        let plain = content.octets()?;
        let bags = Tlv::read_all(&plain)?.children()?;
        for bag in bags.into_iter().take(MAX_BAGS) {
            let kids = bag.children()?;
            let (Some(id), Some(value)) = (kids.first(), kids.get(1)) else {
                continue;
            };
            if id.oid()? != oid::CERT_BAG {
                continue;
            }
            let parts = Tlv::read_all(value.body)?.children()?;
            let [kind, cert] = &parts[..] else { continue };
            if kind.oid()? != oid::X509_CERTIFICATE {
                continue;
            }
            let mut has_key = false;
            if let Some(attrs) = kids.get(2) {
                for a in attrs.children()? {
                    let p = a.children()?;
                    if p.first().and_then(|o| o.oid().ok()) == Some(oid::LOCAL_KEY_ID) {
                        has_key = true;
                    }
                }
                if name.is_none() {
                    name = friendly_name(attrs)?;
                }
            }
            certs.push((has_key, Tlv::read_all(cert.body)?.octets()?));
        }
    }
    certs.sort_by_key(|(has_key, _)| !has_key);
    Ok((certs.into_iter().map(|(_, c)| c).collect(), name))
}

// ------------------------------------------------------------------ writing

/// Writes a PKCS#12 file holding the private key `key` (PKCS#8), its certificate `cert`
/// and other certificates `chain`, named `name`, protected by `password`. The certificates
/// are encrypted too when `encrypt_certs` is set; otherwise anyone can read them (they are
/// public), which lets LimarePDF list IDs without their passwords.
pub(crate) fn write(
    key: &[u8],
    cert: &[u8],
    chain: &[Vec<u8>],
    name: &str,
    password: &str,
    encrypt_certs: bool,
) -> Result<Vec<u8>> {
    let local_id = der_octets(&sha1::Sha1::digest(cert));
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    let bmp_name: Vec<u8> = name.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let attrs = asn1::der_set(vec![
        der_seq(&[
            &der_oid(&oid::FRIENDLY_NAME),
            &asn1::der_set(vec![der(0x1e, &bmp_name)]),
        ]),
        der_seq(&[&der_oid(&oid::LOCAL_KEY_ID), &asn1::der_set(vec![local_id])]),
    ]);
    let cert_bag = |c: &[u8], attrs: Option<&[u8]>| {
        let value = der_seq(&[
            &der_oid(&oid::X509_CERTIFICATE),
            &der(ctx(0), &der_octets(c)),
        ]);
        der_seq(&[
            &der_oid(&oid::CERT_BAG),
            &der(ctx(0), &value),
            attrs.unwrap_or_default(),
        ])
    };
    let mut bags = vec![cert_bag(cert, Some(&attrs))];
    bags.extend(chain.iter().map(|c| cert_bag(c, None)));
    let cert_safe = der(asn1::SEQUENCE, &bags.concat());
    let certs_info = if encrypt_certs {
        let (algorithm, data) = encrypt(password, &cert_safe)?;
        let eci = der_seq(&[
            &der_oid(&oid::DATA),
            &algorithm,
            &der(ctx_primitive(0), &data),
        ]);
        let ed = der_seq(&[&der_uint(0), &eci]);
        der_seq(&[&der_oid(&oid::ENCRYPTED_DATA), &der(ctx(0), &ed)])
    } else {
        der_seq(&[&der_oid(&oid::DATA), &der(ctx(0), &der_octets(&cert_safe))])
    };

    let (algorithm, data) = encrypt(password, key)?;
    let epki = der_seq(&[&algorithm, &der_octets(&data)]);
    let key_bag = der_seq(&[
        &der_oid(&oid::SHROUDED_KEY_BAG),
        &der(ctx(0), &epki),
        &attrs,
    ]);
    let key_safe = der_seq(&[&key_bag]);
    let key_info = der_seq(&[&der_oid(&oid::DATA), &der(ctx(0), &der_octets(&key_safe))]);

    let auth = der_seq(&[&certs_info, &key_info]);
    let salt = random::<16>()?;
    let mac_key = Hash::Sha256.p12_kdf(&bmp_password(password), &salt, KDF_MAC, MAC_ITERATIONS, 32);
    let mac = Hash::Sha256.hmac(&mac_key, &auth);
    let mac_data = der_seq(&[
        &der_seq(&[
            &der_seq(&[&der_oid(&oid::SHA256), &der(asn1::NULL, &[])]),
            &der_octets(&mac),
        ]),
        &der_octets(&salt),
        &der_uint(u64::from(MAC_ITERATIONS)),
    ]);
    Ok(der_seq(&[
        &der_uint(3),
        &der_seq(&[&der_oid(&oid::DATA), &der(ctx(0), &der_octets(&auth))]),
        &mac_data,
    ]))
}

/// Encrypts `plain` with PBES2 (PBKDF2-HMAC-SHA256, AES-256-CBC); returns the algorithm
/// identifier and the encrypted bytes.
fn encrypt(password: &str, plain: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let salt = random::<16>()?;
    let iv = random::<16>()?;
    let key = Hash::Sha256.pbkdf2(password.as_bytes(), &salt, KEY_ITERATIONS, 32);
    let data = cbc::Encryptor::<aes::Aes256>::new_from_slices(&key, &iv)
        .map_err(|_| anyhow!("AES-256 key setup failed"))?
        .encrypt_padded_vec::<Pkcs7>(plain);
    let kdf = der_seq(&[
        &der_oid(&oid::PBKDF2),
        &der_seq(&[
            &der_octets(&salt),
            &der_uint(u64::from(KEY_ITERATIONS)),
            &der_seq(&[&der_oid(&oid::HMAC_SHA256), &der(asn1::NULL, &[])]),
        ]),
    ]);
    let scheme = der_seq(&[&der_oid(&oid::AES256_CBC), &der_octets(&iv)]);
    let algorithm = der_seq(&[&der_oid(&oid::PBES2), &der_seq(&[&kdf, &scheme])]);
    Ok((algorithm, data))
}

/// `N` bytes from the operating system's random source.
pub(crate) fn random<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0; N];
    getrandom::SysRng
        .try_fill_bytes(&mut b)
        .map_err(|e| anyhow!("the system's random number source failed: {e}"))?;
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kdf_matches_rfc_7292_vectors() {
        // Known answers for the PKCS#12 derivation with SHA-1 (as used by OpenSSL's tests
        // and Bouncy Castle's PKCS12ParametersGeneratorTest): password "smeg", salt
        // 0A58CF64530D823F, one iteration.
        let pass = bmp_password("smeg");
        let salt = [0x0a, 0x58, 0xcf, 0x64, 0x53, 0x0d, 0x82, 0x3f];
        let key = Hash::Sha1.p12_kdf(&pass, &salt, KDF_KEY, 1, 24);
        assert_eq!(
            hex(&key),
            "8aaae6297b6cb04642ab5b077851284eb7128f1a2a7fbca3"
        );
        let iv = Hash::Sha1.p12_kdf(&pass, &salt, KDF_IV, 1, 8);
        assert_eq!(hex(&iv), "79993dfe048d3b76");
        // "queeg", salt 05DEC959ACFF72F7, 1000 iterations.
        let pass = bmp_password("queeg");
        let salt = [0x05, 0xde, 0xc9, 0x59, 0xac, 0xff, 0x72, 0xf7];
        let key = Hash::Sha1.p12_kdf(&pass, &salt, KDF_KEY, 1000, 24);
        assert_eq!(
            hex(&key),
            "ed2034e36328830ff09df1e1a07dd357185dac0d4f9eb3d4"
        );
        let iv = Hash::Sha1.p12_kdf(&pass, &salt, KDF_IV, 1000, 8);
        assert_eq!(hex(&iv), "11dedad7758d4860");
        // A MAC key: "smeg", salt 3D83C0E4546AC140, one iteration.
        let pass = bmp_password("smeg");
        let salt = [0x3d, 0x83, 0xc0, 0xe4, 0x54, 0x6a, 0xc1, 0x40];
        let mac = Hash::Sha1.p12_kdf(&pass, &salt, KDF_MAC, 1, 20);
        assert_eq!(hex(&mac), "8d967d88f6caa9d714800ab3d48051d63f73a312");
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// BER as Windows writes it: indefinite lengths throughout, and octet strings split
    /// into pieces. Made from an OpenSSL file (legacy encryption) by re-encoding it, with
    /// the integrity check redone over the re-encoded contents.
    #[test]
    fn reads_ber() {
        fn indefinite(tag: u8, parts: &[Vec<u8>]) -> Vec<u8> {
            let mut out = vec![tag, 0x80];
            parts.iter().for_each(|p| out.extend(p));
            out.extend([0, 0]);
            out
        }
        fn pieces(tag: u8, body: &[u8]) -> Vec<u8> {
            let chunks: Vec<Vec<u8>> = body.chunks(40).map(der_octets).collect();
            indefinite(tag, &chunks)
        }
        // Every constructed element indefinite; long octet strings (also the encrypted
        // contents' `[0]`) in pieces.
        fn ber(t: &Tlv) -> Vec<u8> {
            match t.tag {
                asn1::OCTET_STRING if t.body.len() > 40 => pieces(0x24, t.body),
                0x80 if t.body.len() > 40 => pieces(ctx(0), t.body),
                _ if t.constructed() => {
                    let kids: Vec<Vec<u8>> = t.children().unwrap().iter().map(ber).collect();
                    indefinite(t.tag, &kids)
                }
                _ => t.raw.to_vec(),
            }
        }
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/signatures/ids");
        let file = std::fs::read(format!("{dir}/legacy.p12")).unwrap();
        let password = "mpe-test-password";
        let want = read(&file, password).unwrap();

        let pfx = Tlv::read_all(&file).unwrap().children().unwrap();
        let (_, content) = content_info(&pfx[1]).unwrap();
        let auth = ber(&Tlv::read_all(&content.octets().unwrap()).unwrap());
        let mac = pfx[2].children().unwrap();
        let salt = mac[1].octets().unwrap();
        let iterations = mac[2].uint().unwrap() as u32;
        let key = Hash::Sha1.p12_kdf(&bmp_password(password), &salt, KDF_MAC, iterations, 20);
        let mac = der_seq(&[
            &der_seq(&[
                &der_seq(&[&der_oid(&oid::SHA1), &der(asn1::NULL, &[])]),
                &der_octets(&Hash::Sha1.hmac(&key, &auth)),
            ]),
            &der_octets(&salt),
            &der_uint(u64::from(iterations)),
        ]);
        let content = indefinite(ctx(0), &[pieces(0x24, &auth)]);
        let auth_safe = indefinite(asn1::SEQUENCE, &[der_oid(&oid::DATA), content]);
        let windows = indefinite(asn1::SEQUENCE, &[der_uint(3), auth_safe, mac]);
        assert_ne!(windows, file);

        let got = read(&windows, password).unwrap();
        assert_eq!(got.certs, want.certs);
        assert_eq!(got.keys.len(), 1);
        assert_eq!(&got.keys[0][..], &want.keys[0][..]);
        let err = read(&windows, "wrong").err().unwrap();
        assert!(err.downcast_ref::<WrongPassword>().is_some());
    }

    /// A file around `auth` (an AuthenticatedSafe) with an HMAC-SHA1 integrity check of
    /// `iterations` for `password`.
    fn wrap(auth: &[u8], password: &str, iterations: u64) -> Vec<u8> {
        let salt = [1u8; 8];
        let mac = if iterations <= 10_000 {
            let pass = bmp_password(password);
            let key = Hash::Sha1.p12_kdf(&pass, &salt, KDF_MAC, iterations as u32, 20);
            Hash::Sha1.hmac(&key, auth)
        } else {
            vec![0; 20]
        };
        der_seq(&[
            &der_uint(3),
            &der_seq(&[&der_oid(&oid::DATA), &der(ctx(0), &der_octets(auth))]),
            &der_seq(&[
                &der_seq(&[
                    &der_seq(&[&der_oid(&oid::SHA1), &der(asn1::NULL, &[])]),
                    &der_octets(&mac),
                ]),
                &der_octets(&salt),
                &der_uint(iterations),
            ]),
        ])
    }

    /// Plain contents (`SafeContents`) as the only part of an AuthenticatedSafe.
    fn plain(safe: &[u8]) -> Vec<u8> {
        let info = der_seq(&[&der_oid(&oid::DATA), &der(ctx(0), &der_octets(safe))]);
        der_seq(&[&info])
    }

    #[test]
    fn refuses_costly_files() {
        let safe = der_seq(&[]);
        // A huge or zero iteration count is refused before any work.
        let start = std::time::Instant::now();
        for n in [4_000_000_000u64, 20_000_001, u64::MAX] {
            let e = read(&wrap(&plain(&safe), "", n), "").err().unwrap();
            assert!(format!("{e}").contains("more password stretching"), "{e}");
        }
        assert!(start.elapsed().as_secs() < 2);
        let e = read(&wrap(&plain(&safe), "", 0), "").err().unwrap();
        assert!(format!("{e}").contains("iteration count of 0"), "{e}");
        // An empty file is fine (no key: the ID layer refuses it).
        let empty = read(&wrap(&plain(&safe), "", 1), "").unwrap();
        assert!(empty.keys.is_empty());

        // Too many items.
        // Secret bags, which LimarePDF skips.
        let secret = Oid::new_unwrap("1.2.840.113549.1.12.10.1.5");
        let bag = der_seq(&[&der_oid(&secret), &der(ctx(0), &der_seq(&[]))]);
        assert!(read(&wrap(&plain(&der_seq(&[&bag])), "", 1), "").is_ok());
        let many = der(asn1::SEQUENCE, &vec![bag.clone(); MAX_BAGS + 1].concat());
        let e = read(&wrap(&plain(&many), "", 1), "").err().unwrap();
        assert!(format!("{e}").contains("too many items"), "{e}");
        // Items nested too deeply.
        let mut nested = der_seq(&[]);
        for _ in 0..=MAX_NESTING + 1 {
            let b = der_seq(&[&der_oid(&oid::SAFE_CONTENTS_BAG), &der(ctx(0), &nested)]);
            nested = der_seq(&[&b]);
        }
        let e = read(&wrap(&plain(&nested), "", 1), "").err().unwrap();
        assert!(format!("{e}").contains("too deeply"), "{e}");
        // Public-key modes are refused with the reason.
        let signed = der_seq(&[
            &der_uint(3),
            &der_seq(&[&der_oid(&oid::SIGNED_DATA), &der(ctx(0), &der_seq(&[]))]),
        ]);
        let e = read(&signed, "").err().unwrap();
        assert!(format!("{e}").contains("public key"), "{e}");
    }

    /// A small file written and read back with a non-ASCII password, and listed without it.
    #[test]
    fn writes_and_reads_back() {
        let key = b"not really a key, but bytes to protect".to_vec();
        let cert = der_seq(&[&der_octets(b"certificate")]);
        let chain = vec![der_seq(&[&der_octets(b"issuer")])];
        for encrypt_certs in [false, true] {
            let password = "pässwörd 🙂";
            let file = write(&key, &cert, &chain, "Jane Doe", password, encrypt_certs).unwrap();
            let c = read(&file, password).unwrap();
            assert_eq!(c.keys.len(), 1);
            assert_eq!(&c.keys[0][..], &key[..]);
            assert_eq!(c.certs, [cert.clone(), chain[0].clone()]);
            assert_eq!(c.friendly_name.as_deref(), Some("Jane Doe"));
            let err = read(&file, "passwörd 🙂").err().unwrap();
            assert!(err.downcast_ref::<WrongPassword>().is_some(), "{err}");
            let (peeked, name) = peek(&file).unwrap();
            if encrypt_certs {
                assert!(peeked.is_empty());
            } else {
                assert_eq!(peeked, [cert.clone(), chain[0].clone()]);
                assert_eq!(name.as_deref(), Some("Jane Doe"));
            }
        }
    }
}
