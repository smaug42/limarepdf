#!/usr/bin/env bash
# Writes the README, CHANGELOG and SECURITY files the installers and packages (deb, rpm, MSI, dmg) carry,
# into target/package-docs/. The packages don't carry docs/, so every relative link points at this version's
# tag on GitHub instead: pages under blob/, images under raw/. The archives carry docs/ and the files as they are.
#
#   packaging/package-docs.sh
#
# Run from the repository root before building a package. The links work once the version is tagged.
set -euo pipefail

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
site="https://github.com/smaug42/limarepdf"
out=target/package-docs
mkdir -p "$out"
for f in README.md CHANGELOG.md SECURITY.md; do
    # A link target without a colon (no https:) that doesn't start with # is relative.
    sed -E \
        -e "s#(!\[[^]]*\])\(([^):#][^):]*)\)#\1($site/raw/v$version/\2)#g" \
        -e "s#\]\(([^):#][^):]*)\)#]($site/blob/v$version/\1)#g" \
        "$f" > "$out/$f"
done
