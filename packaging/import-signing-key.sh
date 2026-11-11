#!/usr/bin/env bash
# Imports the release signing subkey for a job that signs: ci.yml's release job at a version tag and
# packages.yml's sign job, with the release-signing environment's GPG_SIGNING_KEY; ci.yml's dry run
# with a key made for that run alone.
#
#   packaging/import-signing-key.sh <published key (.asc)>
#
# The published key is imported after the secret: the secret keeps the expiry dates it was exported
# with, and gpg takes the later dates a renewed published key carries, so renewing the key needs no
# new secret (docs/developer/releasing.md).
set -euo pipefail

if [ -z "${GPG_SIGNING_KEY:-}" ]; then
    echo "::error::The release-signing environment has no GPG_SIGNING_KEY secret"
    exit 1
fi
printf '%s\n' "$GPG_SIGNING_KEY" | gpg --batch --import
gpg --batch --import "$1"
