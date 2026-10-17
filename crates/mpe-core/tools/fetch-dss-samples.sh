#!/bin/sh
# Downloads signed sample PDFs from the EU's DSS project (github.com/esig/dss) into
# tests/data/signatures/dss, for the opt-in test `cargo test -p mpe-core --test signatures
# -- --ignored dss`. They are not committed: they are LGPL-2.1, and many hold real
# people's names and certificates. The folder is git-ignored.
#
# Usage, from crates/mpe-core: sh tools/fetch-dss-samples.sh
set -eu
commit=9f3af8f2ca672b8948e0a3ee8687e53adf9f5c33
base=https://raw.githubusercontent.com/esig/dss/$commit/dss-pades/src/test/resources/validation
out=tests/data/signatures/dss
mkdir -p "$out"
for f in \
    adbe_crl_signed.pdf adbe_ocsp_signed.pdf BadEncodedCMS.pdf doc-firmado.pdf \
    hello_signed_INCSAVE_signed.pdf hello_signed_INCSAVE_signed_EDITED.pdf \
    malformed-pades.pdf malformed-rsa-digestinfo.pdf modified_after_signature.pdf \
    pades-5-signatures-and-1-document-timestamp.pdf \
    pades-alter-signature-appearance-modify-stream.pdf pades-bes.pdf \
    pades-bes-no-certificates.pdf pades-enveloping-cms.pdf pades-enveloping-other-pdf.pdf \
    pades-signed-annot-added.pdf pades-signed-filled-form.pdf \
    pades-spoofing-replaced-reason.pdf pades-unsupported-signature-algorithm.pdf \
    pdf-byterange-overlap.pdf pdf-double-signer-info.pdf pdf-eof.pdf \
    pdf-signed-added-page.pdf pdf-signed-corrupted.pdf pdf-signed-original.pdf \
    pdf-signed-out-of-byteRange.pdf pdf-spoofing-attack.pdf pkcs7.pdf pkcs7_sha1.pdf \
    pkcs7-no-message-digest.pdf signature-referenced-from-multiple-fields.pdf \
    Signature-P-PK7-2.pdf wrong-digest-algo.pdf \
    dss-2236/hide.pdf dss-2236/replace.pdf \
    dss-2554/certified-changes-permitted.pdf dss-2554/certified-no-change-permitted.pdf \
    dss-2554/fieldmdp-include.pdf \
    "dss-818/Signature-P-IT_ADO-1 (HASH_FAILURE) (ECDSA).pdf" \
    dss-PLAIN-ECDSA/TeleSec_PKS_eIDAS_QES_CA_1-baseline-b.pdf; do
    url=$base/$(printf '%s' "$f" | sed 's/ /%20/g; s/(/%28/g; s/)/%29/g')
    curl -sfL -o "$out/$(basename "$f")" "$url" || echo "failed: $f" >&2
done
echo "$(ls "$out" | wc -l) files in $out"
