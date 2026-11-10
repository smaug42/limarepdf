#!/usr/bin/env bash
# Signs a release's files: ci.yml's release job runs it at a version tag with the release key, and
# its dry run rehearses it on a copy with a key made for that run alone.
#
#   packaging/sign-release.sh <folder with the files> <public key (.asc) to check against>
#
# Needs gpg with the signing key imported, and rpm; GPG_PASSPHRASE is used if set. Each .rpm gets its
# own signature; then SHA256SUMS, and a detached signature (.asc) for it and for every file. Every
# signature is checked against the public key given (at a tag, the published one in docs/), as rpm,
# dnf and gpgv will.
set -euo pipefail

dir=$1
pub=$(realpath "$2")
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

gpg --dearmor < "$pub" > "$tmp/keyring.gpg"
(umask 077; printf '%s' "${GPG_PASSPHRASE:-}" > "$tmp/pass")
key=$(gpg --list-secret-keys --with-colons | awk -F: '/^fpr/ {print $10; exit}')

# Ubuntu's rpm looks for gpg2, which isn't installed.
rpmsign --addsign --define "__gpg /usr/bin/gpg" --define "_gpg_name $key" \
    --define "_gpg_sign_cmd_extra_args --batch --pinentry-mode loopback --passphrase-file $tmp/pass" \
    "$dir"/*.rpm
rpmkeys --dbpath "$tmp/rpmdb" --import "$pub"
for f in "$dir"/*.rpm; do
    rpmkeys --dbpath "$tmp/rpmdb" --checksig "$f" | tee "$tmp/rpmcheck"
    grep -q "signatures OK" "$tmp/rpmcheck"
done

cd "$dir"
sha256sum * > SHA256SUMS
for f in *; do
    gpg --batch --yes --pinentry-mode loopback --passphrase-file "$tmp/pass" --armor --detach-sign "$f"
    gpgv --keyring "$tmp/keyring.gpg" "$f.asc" "$f"
done
