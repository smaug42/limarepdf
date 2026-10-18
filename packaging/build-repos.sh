#!/usr/bin/env bash
# Builds the apt and dnf repositories served on GitHub Pages.
#
#   packaging/build-repos.sh <folder with .deb and .rpm files> <site folder>
#
# Needs apt-ftparchive (apt-utils), createrepo_c and gpg with the signing key imported;
# GPG_PASSPHRASE is used if set. The site gets apt/ (a flat repository), rpm/ (with
# limarepdf.repo), the public key and the index page from packaging/pages/.
set -euo pipefail

src=$1
site=$2
here=$(cd "$(dirname "$0")" && pwd)
url=${LIMAREPDF_SITE_URL:-https://smaug42.github.io/limarepdf}

sign() {
    if [ -n "${GPG_PASSPHRASE:-}" ]; then
        gpg --batch --yes --pinentry-mode loopback --passphrase "$GPG_PASSPHRASE" "$@"
    else
        gpg --batch --yes "$@"
    fi
}

mkdir -p "$site/apt" "$site/rpm"
cp "$here/../docs/limarepdf-signing-key.asc" "$site/"
cp "$here"/pages/* "$site/"

# apt: a flat repository, used with "Suites: ./" in limarepdf.sources.
cp "$src"/*.deb "$site/apt/"
(
    cd "$site/apt"
    apt-ftparchive packages . > Packages
    gzip -k9 Packages
    apt-ftparchive \
        -o APT::FTPArchive::Release::Origin=LimarePDF \
        -o APT::FTPArchive::Release::Label=LimarePDF \
        -o APT::FTPArchive::Release::Suite=stable \
        -o APT::FTPArchive::Release::Architectures=amd64 \
        release . > ../Release.tmp
    mv ../Release.tmp Release
    sign --clearsign --output InRelease Release
    sign --armor --detach-sign --output Release.gpg Release
)
cat > "$site/apt/limarepdf.sources" <<EOF
Types: deb
URIs: $url/apt/
Suites: ./
Signed-By: /etc/apt/keyrings/limarepdf.asc
EOF

# dnf and zypper: repodata signed, and each rpm carries its own signature.
cp "$src"/*.rpm "$site/rpm/"
createrepo_c --quiet "$site/rpm"
sign --armor --detach-sign --output "$site/rpm/repodata/repomd.xml.asc" "$site/rpm/repodata/repomd.xml"
cat > "$site/rpm/limarepdf.repo" <<EOF
[limarepdf]
name=LimarePDF
baseurl=$url/rpm/
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=$url/limarepdf-signing-key.asc
EOF
