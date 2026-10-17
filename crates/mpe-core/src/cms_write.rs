//! Making the CMS signature (RFC 5652 SignedData, detached) that goes into a PDF
//! signature's `/Contents`, as Acrobat writes it for `adbe.pkcs7.detached`: the signer's
//! certificate and the authorities above it, and signed attributes for the content type,
//! the signing time, the digest of the signed bytes and the signer's certificate (ESS
//! signing-certificate-v2, which stops another certificate for the same key being put in
//! its place). Written with the DER helpers in [`crate::asn1`].

use crate::asn1::{der, der_octets, der_oid, der_seq, der_set, der_uint, SEQUENCE};
use crate::cms_check::{oid, Cert, DigestAlg};
use crate::ids::UnlockedId;
use crate::Result;
use anyhow::{anyhow, bail};
use chrono::{DateTime, Datelike, Utc};
use der::asn1::ObjectIdentifier as Oid;
use der::Encode;

/// The SignedData for a PDF signature by `id` over `ranges` (the signed byte ranges), made
/// at `time`.
pub(crate) fn signed_data(
    id: &UnlockedId,
    ranges: &[&[u8]],
    time: DateTime<Utc>,
) -> Result<Vec<u8>> {
    let digest = id.digest();
    let digest_alg = der_seq(&[&der_oid(&digest_oid(digest)?)]);
    let cert = Cert::parse(&id.cert.der)?;
    let tbs = cert.cert.tbs_certificate();
    let encode = |r: der::Result<Vec<u8>>| {
        r.map_err(|e| anyhow!("encoding the certificate's names failed: {e}"))
    };
    let issuer_serial = der_seq(&[
        &encode(tbs.issuer().to_der())?,
        &encode(tbs.serial_number().to_der())?,
    ]);

    let attr = |o: &Oid, value: Vec<u8>| der_seq(&[&der_oid(o), &der_set(vec![value])]);
    // ESSCertIDv2 with the default hash algorithm, SHA-256, left out as DER requires.
    let cert_id = der_seq(&[&der_octets(&DigestAlg::Sha256.hash(&[&id.cert.der]))]);
    let attrs = der_set(vec![
        attr(&oid::CONTENT_TYPE, der_oid(&oid::DATA)),
        attr(&oid::SIGNING_TIME, asn1_time(time)),
        attr(&oid::MESSAGE_DIGEST, der_octets(&digest.hash(ranges))),
        attr(&oid::SIGNING_CERT_V2, der_seq(&[&der_seq(&[&cert_id])])),
    ]);
    // The signature covers the attributes as a SET; in the SignerInfo they are [0].
    let value = id.sign(&attrs)?;
    let mut signed_attrs = attrs;
    signed_attrs[0] = 0xa0;

    let sig_alg = if id.is_rsa() {
        // As Acrobat writes it: rsaEncryption, with the digest named separately.
        der_seq(&[&der_oid(&oid::RSA), &[0x05, 0x00]])
    } else {
        let o = match digest {
            DigestAlg::Sha384 => oid::ECDSA_SHA384,
            _ => oid::ECDSA_SHA256,
        };
        der_seq(&[&der_oid(&o)])
    };
    let signer_info = der_seq(&[
        &der_uint(1),
        &issuer_serial,
        &digest_alg,
        &signed_attrs,
        &sig_alg,
        &der_octets(&value),
    ]);
    let mut certs = id.cert.der.clone();
    for c in &id.chain {
        certs.extend_from_slice(&c.der);
    }
    let sd = der_seq(&[
        &der_uint(1),
        &der_set(vec![digest_alg]),
        &der_seq(&[&der_oid(&oid::DATA)]),
        &der(0xa0, &certs),
        &der_set(vec![signer_info]),
    ]);
    Ok(der(
        SEQUENCE,
        &[der_oid(&oid::SIGNED_DATA), der(0xa0, &sd)].concat(),
    ))
}

fn digest_oid(d: DigestAlg) -> Result<Oid> {
    Ok(match d {
        DigestAlg::Sha256 => oid::SHA256,
        DigestAlg::Sha384 => oid::SHA384,
        DigestAlg::Sha512 => oid::SHA512,
        other => bail!("MPE doesn't sign with {}", other.name()),
    })
}

/// A time as CMS writes it: UTCTime through 2049, GeneralizedTime after (RFC 5652 11.3).
fn asn1_time(t: DateTime<Utc>) -> Vec<u8> {
    if (1950..2050).contains(&t.year()) {
        der(0x17, t.format("%y%m%d%H%M%SZ").to_string().as_bytes())
    } else {
        der(0x18, t.format("%Y%m%d%H%M%SZ").to_string().as_bytes())
    }
}
