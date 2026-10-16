#!/bin/sh
# Makes the keys and certificates the signature tests sign with (tests/signatures.rs), in
# tests/data/signatures/keys. They are made with OpenSSL, so the tests don't only check
# MPE against itself. The identities are made up; the keys are public and for tests only.
#
# Usage, from crates/mpe-core: sh tools/make-signature-keys.sh
# Needs OpenSSL 3.4 or later (for -not_before and -not_after).
set -eu
out=tests/data/signatures/keys
mkdir -p "$out"
cd "$out"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Writes NAME.key.der (PKCS#8) from an OpenSSL key specification.
key() {
    openssl genpkey $2 -out "$tmp/$1.pem" 2>/dev/null
    openssl pkcs8 -topk8 -nocrypt -in "$tmp/$1.pem" -outform DER -out "$1.key.der"
}

# Writes NAME.crt.der: a certificate for KEY with SUBJECT and extensions EXT, valid from
# FROM to TO, signed by itself or by ISSUER (a name used with key/cert above).
cert() {
    # (sh has no local variables, hence names unlike the callers'.)
    name=$1 key=$2 subject=$3 ext=$4 nb=$5 na=$6 issuer=${7:-}
    printf '[req]\ndistinguished_name = dn\n[dn]\n[ext]\n%s\n' "$ext" >"$tmp/ext.cnf"
    if [ -z "$issuer" ]; then
        openssl req -x509 -new -key "$tmp/$key.pem" -subj "$subject" -not_before "$nb" \
            -not_after "$na" -config "$tmp/ext.cnf" -extensions ext \
            -set_serial "0x$(openssl rand -hex 8)" -outform DER -out "$name.crt.der"
    else
        openssl req -new -key "$tmp/$key.pem" -subj "$subject" -config "$tmp/ext.cnf" -out "$tmp/$name.csr"
        openssl x509 -req -in "$tmp/$name.csr" -CA "$tmp/$issuer.crt.pem" -CAkey "$tmp/$issuer.pem" \
            -not_before "$nb" -not_after "$na" -extfile "$tmp/ext.cnf" -extensions ext \
            -set_serial "0x$(openssl rand -hex 8)" -outform DER -out "$name.crt.der"
    fi
    openssl x509 -inform DER -in "$name.crt.der" -out "$tmp/$name.crt.pem"
}

signer='keyUsage = critical, digitalSignature, nonRepudiation
subjectKeyIdentifier = hash'
from=20260101000000Z
to=20360101000000Z

key rsa3072 '-algorithm RSA -pkeyopt rsa_keygen_bits:3072'
key rsa2048 '-algorithm RSA -pkeyopt rsa_keygen_bits:2048'
key rsa1024 '-algorithm RSA -pkeyopt rsa_keygen_bits:1024'
key p256 '-algorithm EC -pkeyopt ec_paramgen_curve:P-256'
key p384 '-algorithm EC -pkeyopt ec_paramgen_curve:P-384'
key p521 '-algorithm EC -pkeyopt ec_paramgen_curve:P-521'

cert rsa3072 rsa3072 '/CN=Test Signer RSA-3072/O=MPE Tests/emailAddress=rsa3072@example.com' "$signer" $from $to
cert rsa2048 rsa2048 '/CN=Test Signer RSA-2048/O=MPE Tests' "$signer" $from $to
cert rsa1024 rsa1024 '/CN=Test Signer RSA-1024/O=MPE Tests' "$signer" $from $to
cert p256 p256 '/CN=Test Signer P-256/O=MPE Tests' "$signer" $from $to
cert p384 p384 '/CN=Test Signer P-384/O=MPE Tests' "$signer" $from $to
cert p521 p521 '/CN=Test Signer P-521/O=MPE Tests' "$signer" $from $to
# Same key as rsa2048: expired in 2021, and one whose key isn't for signing.
cert expired rsa2048 '/CN=Test Signer Expired/O=MPE Tests' "$signer" 20200101000000Z 20210101000000Z
cert keyenc rsa2048 '/CN=Test Signer Key Encipherment/O=MPE Tests' \
    'keyUsage = critical, keyEncipherment' $from $to
# A timestamp authority.
cert tsa rsa2048 '/CN=Test Timestamp Authority/O=MPE Tests' \
    'keyUsage = critical, digitalSignature
extendedKeyUsage = critical, timeStamping' $from $to
# A certificate authority and a signer it issued.
key ca '-algorithm RSA -pkeyopt rsa_keygen_bits:2048'
cert ca ca '/CN=Test Root CA/O=MPE Tests' 'basicConstraints = critical, CA:true
keyUsage = critical, keyCertSign, cRLSign
subjectKeyIdentifier = hash' $from $to
cert issued p256 '/CN=Test Signer Issued/O=MPE Tests' "$signer
authorityKeyIdentifier = keyid" $from $to ca
