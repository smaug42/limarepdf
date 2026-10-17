#!/bin/sh
# Makes the keys and certificates the signature tests sign with (tests/signatures.rs), in
# tests/data/signatures/keys. They are made with OpenSSL, so the tests don't only check
# MPE against itself. The identities are made up; the keys are public and for tests only.
#
# Usage, from crates/mpe-core: sh tools/make-signature-keys.sh [all|trust]
# `trust` remakes only the certificate chains for tests/signature_trust.rs (t-*.der).
# Needs OpenSSL 3.4 or later (for -not_before and -not_after).
set -eu
what=${1:-all}
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

# A key kept only while the script runs ($tmp/NAME.pem), for certificate authorities the
# tests never sign with.
cakey() {
    openssl genpkey $2 -out "$tmp/$1.pem" 2>/dev/null
}

# The digest certificates are signed with.
md=sha256

# Writes NAME.crt.der: a certificate for KEY with SUBJECT and extensions EXT, valid from
# FROM to TO, signed by itself or by ISSUER (a certificate made above; its key is the one
# named ISSUER, or ISSUER_KEY when given).
cert() {
    # (sh has no local variables, hence names unlike the callers'.)
    name=$1 key=$2 subject=$3 ext=$4 nb=$5 na=$6 issuer=${7:-} issuer_key=${8:-${7:-}}
    printf '[req]\ndistinguished_name = dn\n[dn]\n[ext]\n%s\n' "$ext" >"$tmp/ext.cnf"
    if [ -z "$issuer" ]; then
        openssl req -x509 -new -key "$tmp/$key.pem" -subj "$subject" -not_before "$nb" \
            -not_after "$na" -config "$tmp/ext.cnf" -extensions ext "-$md" \
            -set_serial "0x$(openssl rand -hex 8)" -outform DER -out "$name.crt.der"
    else
        openssl req -new -key "$tmp/$key.pem" -subj "$subject" -config "$tmp/ext.cnf" -out "$tmp/$name.csr"
        openssl x509 -req -in "$tmp/$name.csr" -CA "$tmp/$issuer.crt.pem" -CAkey "$tmp/$issuer_key.pem" \
            -not_before "$nb" -not_after "$na" -extfile "$tmp/ext.cnf" -extensions ext "-$md" \
            -set_serial "0x$(openssl rand -hex 8)" -outform DER -out "$name.crt.der"
    fi
    openssl x509 -inform DER -in "$name.crt.der" -out "$tmp/$name.crt.pem"
}

signer='keyUsage = critical, digitalSignature, nonRepudiation
subjectKeyIdentifier = hash'
from=20260101000000Z
to=20360101000000Z

if [ "$what" = all ]; then

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
fi

# Chains for trust (tests/signature_trust.rs): t-root is the authority the tests trust.
# Signers use the t-leaf key; authorities have keys of their own that aren't kept.
root='basicConstraints = critical, CA:true
keyUsage = critical, keyCertSign, cRLSign
subjectKeyIdentifier = hash'
ca="$root
authorityKeyIdentifier = keyid"
leaf="$signer
authorityKeyIdentifier = keyid"
p256='-algorithm EC -pkeyopt ec_paramgen_curve:P-256'
rsa2048='-algorithm RSA -pkeyopt rsa_keygen_bits:2048'
old_from=20200101000000Z
old_to=20210101000000Z

key t-leaf "$p256"
cakey t-root "$rsa2048"
cert t-root t-root '/CN=Trust Test Root/O=MPE Tests' "$root" $from $to
# Root, intermediate (allowed no authorities below it), signer.
cakey t-inter "$p256"
cert t-inter t-inter '/CN=Trust Test Intermediate/O=MPE Tests' "$ca
basicConstraints = critical, CA:true, pathlen:0" $from $to t-root
cert t-leaf t-leaf '/CN=Trust Test Signer/O=MPE Tests' "$leaf" $from $to t-inter
cert t-leaf-direct t-leaf '/CN=Trust Test Direct Signer/O=MPE Tests' "$leaf" $from $to t-root
cert t-leaf-email t-leaf '/CN=Trust Test Email Signer/O=MPE Tests' "$leaf
extendedKeyUsage = emailProtection" $from $to t-inter
cert t-leaf-server t-leaf '/CN=Trust Test Server/O=MPE Tests' "$leaf
extendedKeyUsage = serverAuth" $from $to t-inter
cert t-leaf-critical t-leaf '/CN=Trust Test Unknown Extension/O=MPE Tests' "$leaf
1.2.3.4 = critical, ASN1:NULL" $from $to t-inter
cert t-leaf-expired t-leaf '/CN=Trust Test Expired Signer/O=MPE Tests' "$leaf" $old_from $old_to t-inter
# Issuers that may not issue certificates: not a certificate authority, or a key not for
# signing certificates.
cakey t-nonca "$p256"
cert t-nonca t-nonca '/CN=Trust Test Not An Authority/O=MPE Tests' 'basicConstraints = critical, CA:false
keyUsage = critical, keyCertSign, digitalSignature
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid' $from $to t-root
cert t-leaf-nonca t-leaf '/CN=Trust Test Signer Under Non-Authority/O=MPE Tests' "$leaf" $from $to t-nonca
cakey t-nocertsign "$p256"
cert t-nocertsign t-nocertsign '/CN=Trust Test No Cert Sign/O=MPE Tests' 'basicConstraints = critical, CA:true
keyUsage = critical, digitalSignature
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid' $from $to t-root
cert t-leaf-nocertsign t-leaf '/CN=Trust Test Signer Under No Cert Sign/O=MPE Tests' "$leaf" $from $to t-nocertsign
# An authority below t-inter, which allows none.
cakey t-sub "$p256"
cert t-sub t-sub '/CN=Trust Test Sub Authority/O=MPE Tests' "$ca" $from $to t-inter
cert t-leaf-sub t-leaf '/CN=Trust Test Signer Under Sub/O=MPE Tests' "$leaf" $from $to t-sub
# An intermediate that expired before the signing time.
cakey t-inter-expired "$p256"
cert t-inter-expired t-inter-expired '/CN=Trust Test Expired Intermediate/O=MPE Tests' "$ca" $old_from $old_to t-root
cert t-leaf-under-expired t-leaf '/CN=Trust Test Signer Under Expired/O=MPE Tests' "$leaf" $from $to t-inter-expired
# Name constraints, which MPE doesn't check.
cakey t-nc "$p256"
cert t-nc t-nc '/CN=Trust Test Name Constraints/O=MPE Tests' "$ca
nameConstraints = critical, permitted;email:.example.com" $from $to t-root
cert t-leaf-nc t-leaf '/CN=Trust Test Signer Under Name Constraints/O=MPE Tests' "$leaf" $from $to t-nc
# Policy mappings: allowed, except to or from any policy.
cakey t-pm "$p256"
cert t-pm t-pm '/CN=Trust Test Policy Mapping/O=MPE Tests' "$ca
policyMappings = critical, 1.2.3.4:1.2.3.5" $from $to t-root
cert t-leaf-pm t-leaf '/CN=Trust Test Signer Under Policy Mapping/O=MPE Tests' "$leaf" $from $to t-pm
cakey t-pm-any "$p256"
cert t-pm-any t-pm-any '/CN=Trust Test Any Policy Mapping/O=MPE Tests' "$ca
policyMappings = critical, 2.5.29.32.0:1.2.3.5" $from $to t-root
cert t-leaf-pm-any t-leaf '/CN=Trust Test Signer Under Any Policy Mapping/O=MPE Tests' "$leaf" $from $to t-pm-any
# Required policies (policy constraints), as qualified-signature authorities use: the
# chain needs a policy running through every certificate.
cakey t-pc "$p256"
cert t-pc t-pc '/CN=Trust Test Policy Required/O=MPE Tests' "$ca
certificatePolicies = 1.2.3.4
policyConstraints = requireExplicitPolicy:0" $from $to t-root
cert t-leaf-pc t-leaf '/CN=Trust Test Signer With Policy/O=MPE Tests' "$leaf
certificatePolicies = 1.2.3.4" $from $to t-pc
cert t-leaf-pc-other t-leaf '/CN=Trust Test Signer With Other Policy/O=MPE Tests' "$leaf
certificatePolicies = 1.2.3.9" $from $to t-pc
cert t-leaf-pc-none t-leaf '/CN=Trust Test Signer Without Policy/O=MPE Tests' "$leaf" $from $to t-pc
# The same with a mapping: 1.2.3.4 above is 1.2.3.5 below.
cakey t-pc-map "$p256"
cert t-pc-map t-pc-map '/CN=Trust Test Policy Mapped/O=MPE Tests' "$ca
certificatePolicies = 1.2.3.4
policyConstraints = requireExplicitPolicy:0
policyMappings = 1.2.3.4:1.2.3.5" $from $to t-root
cert t-leaf-pc-mapped t-leaf '/CN=Trust Test Signer With Mapped Policy/O=MPE Tests' "$leaf
certificatePolicies = 1.2.3.5" $from $to t-pc-map
cert t-leaf-pc-unmapped t-leaf '/CN=Trust Test Signer With Unmapped Policy/O=MPE Tests' "$leaf
certificatePolicies = 1.2.3.4" $from $to t-pc-map
# Any policy, but not below this authority (inhibitAnyPolicy 0).
cakey t-pc-any "$p256"
cert t-pc-any t-pc-any '/CN=Trust Test Any Policy Inhibited/O=MPE Tests' "$ca
certificatePolicies = 2.5.29.32.0
policyConstraints = requireExplicitPolicy:0
inhibitAnyPolicy = 0" $from $to t-root
cert t-leaf-pc-any t-leaf '/CN=Trust Test Signer With Any Policy/O=MPE Tests' "$leaf
certificatePolicies = 2.5.29.32.0" $from $to t-pc-any
cert t-leaf-pc-named t-leaf '/CN=Trust Test Signer With Named Policy/O=MPE Tests' "$leaf
certificatePolicies = 1.2.3.4" $from $to t-pc-any
# A short RSA key and SHA-1 on the way to the root.
cakey t-weak "-algorithm RSA -pkeyopt rsa_keygen_bits:1024"
cert t-weak t-weak '/CN=Trust Test Weak Key/O=MPE Tests' "$ca" $from $to t-root
cert t-leaf-weak t-leaf '/CN=Trust Test Signer Under Weak Key/O=MPE Tests' "$leaf" $from $to t-weak
cakey t-sha1 "$p256"
md=sha1
cert t-sha1 t-sha1 '/CN=Trust Test SHA-1 Intermediate/O=MPE Tests' "$ca" $from $to t-root
md=sha256
cert t-leaf-sha1 t-leaf '/CN=Trust Test Signer Under SHA-1/O=MPE Tests' "$leaf" $from $to t-sha1
# A forgery: another key calling itself Trust Test Root, and a signer it "issued" with no
# key identifier to tell the two roots apart (OpenSSL adds one unless told not to).
cakey t-evil "$rsa2048"
cert t-evil t-evil '/CN=Trust Test Root/O=MPE Tests' "$root" $from $to
cert t-leaf-evil t-leaf '/CN=Trust Test Forged Signer/O=MPE Tests' "$signer
authorityKeyIdentifier = none" $from $to t-evil
cert t-leaf-evil-keyid t-leaf '/CN=Trust Test Forged Signer With Key Id/O=MPE Tests' "$leaf" $from $to t-evil
# A self-made ID with the same name as t-leaf-direct.
cert t-lookalike t-leaf '/CN=Trust Test Direct Signer/O=MPE Tests' "$signer" $from $to
# Two authorities that issued each other's certificates, and a signer under one: a loop.
cakey t-loop-a "$p256"
cakey t-loop-b "$p256"
cert t-loop-a-self t-loop-a '/CN=Trust Test Loop A/O=MPE Tests' "$root" $from $to
cert t-loop-b-self t-loop-b '/CN=Trust Test Loop B/O=MPE Tests' "$root" $from $to
cert t-loop-a t-loop-a '/CN=Trust Test Loop A/O=MPE Tests' "$ca" $from $to t-loop-b-self t-loop-b
cert t-loop-b t-loop-b '/CN=Trust Test Loop B/O=MPE Tests' "$ca" $from $to t-loop-a-self t-loop-a
cert t-leaf-loop t-leaf '/CN=Trust Test Signer In Loop/O=MPE Tests' "$leaf" $from $to t-loop-a-self t-loop-a
rm t-loop-a-self.crt.der t-loop-b-self.crt.der
# A chain longer than MPE follows: t-root, ten authorities, the signer.
cakey t-deep "$p256"
up=t-root up_key=t-root
for n in 1 2 3 4 5 6 7 8 9 10; do
    cert t-deep-$n t-deep "/CN=Trust Test Deep $n/O=MPE Tests" "$ca" $from $to $up $up_key
    up=t-deep-$n up_key=t-deep
done
cert t-leaf-deep t-leaf '/CN=Trust Test Deep Signer/O=MPE Tests' "$leaf" $from $to $up $up_key
