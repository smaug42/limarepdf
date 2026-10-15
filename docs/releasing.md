# Releasing

How a release is made, for maintainers.

## Steps

1. On a branch from `main`:
   - set the new version in `Cargo.toml` (`[workspace.package]`), and run a build so `Cargo.lock` follows;
   - update the status line in `docs/features.md` if it names the version;
   - in `CHANGELOG.md`, rename the `## [Unreleased]` section to `## [x.y.z] - date`. Changes are added to
     Unreleased as they are made.
2. Open a pull request. CI builds, tests and packages all four platforms; check the archives in the run's artifacts.
   Merge when it is green.
3. On `main`, after pulling the merge, make a signed tag and push it:

   ```
   git tag -s vX.Y.Z -m "mpe X.Y.Z"
   git push origin vX.Y.Z
   ```

4. CI runs again for the tag. Its `release` job checks that the tag matches `Cargo.toml`, takes the release notes
   from the matching `CHANGELOG.md` section, ends them with a link comparing the tag with the previous version tag,
   and creates a draft GitHub Release with the four archives, `SHA256SUMS`, and a GPG signature (`.asc`) for each.
   It also records a build attestation for every file.
5. On GitHub, open the draft under Releases, choose Edit, check the notes and files, and publish it.

Do not create the release on GitHub by hand, or the tag from GitHub's release form: the `release` job then finds a
release already there and fails.

## What CI builds

`.github/workflows/ci.yml` runs on pull requests, pushes to `main`, and `v*` tags. Each platform job packages
`mpe-<version>-<platform>.tar.gz` (`.zip` on Windows), holding the binaries, the README, `CHANGELOG.md`, `docs/` and
the licences. The archives are made on the runner because Actions artifacts lose Unix file permissions; on the
Actions page each artifact therefore downloads as a zip with the archive inside.

The `release` job writes `SHA256SUMS`, signs it and every file with the signing subkey (below), checks each signature
against `docs/mpe-signing-key.asc`, and records build provenance with `actions/attest-build-provenance`, which
users check with `gh attestation verify`.

### Dry run

To try the release job without making a release, start the workflow by hand (Actions > ci > Run workflow) on a
branch with "dry_run" ticked. It builds, signs and attests as a release would, then uploads the files as the
`release-dry-run` artifact instead of creating a release. Its attestations are real but harmless: they name the
branch, not a tag.

## Signing key

Releases are signed with a GPG key used only for MPE: an ed25519 primary key that only certifies, kept offline,
and a signing subkey, the only part CI holds. The subkey expires after a year and the primary after three, so both
need renewing in time.

Making the key (once, on the maintainer's computer). `FPR` and `SUB` below stand for fingerprints: the 40-character
hex line that `gpg --list-keys` prints under the `pub ... [C]` line (the primary key, `FPR`) and under the
`sub ... [S]` line (the signing subkey, `SUB`). Paste them without spaces.

```
gpg --quick-gen-key "MPE release signing <email>" ed25519 cert 3y
gpg --list-keys --with-subkey-fingerprints "MPE release signing"   # note the primary fingerprint, FPR
gpg --quick-add-key FPR ed25519 sign 1y
gpg --list-keys --with-subkey-fingerprints FPR                     # note the signing subkey's fingerprint, SUB
```

Publishing the public key and giving CI the subkey (`gh` is the GitHub CLI; the `!` exports that subkey alone):

```
gpg --armor --export FPR > docs/mpe-signing-key.asc
gpg --armor --export-secret-subkeys SUB! | gh secret set GPG_SIGNING_KEY
gh secret set GPG_PASSPHRASE          # type the key's passphrase
```

Without `gh`, add the two secrets on GitHub under Settings > Secrets and variables > Actions > New repository
secret, pasting the subkey export (copied with `gpg --armor --export-secret-subkeys SUB! | wl-copy`, so it never
touches the disk) and the passphrase.

Then upload the public key at keys.openpgp.org (it emails a confirmation link), put the primary fingerprint in
`docs/install.md`, and keep a backup of the whole key (`gpg --armor --export-secret-keys FPR`) and its revocation
certificate (`~/.gnupg/openpgp-revocs.d/FPR.rev`) off the computer, for example on a USB stick kept elsewhere.

Renewing: `gpg --quick-set-expire FPR 3y` for the primary, `gpg --quick-set-expire FPR 1y SUB` for the subkey, then
export `docs/mpe-signing-key.asc` again and re-upload it to keys.openpgp.org. The secret in CI does not change.

If the subkey leaks, revoke it (`gpg --edit-key FPR`, `key 1`, `revkey`), add a new one, and replace both the
published key and the secret.

## To do

- **Code signing.** The macOS and Windows binaries are not code-signed or notarised, so Gatekeeper and SmartScreen
  warn on first launch (`docs/install.md` tells users how to proceed). Signing needs an Apple Developer account for
  macOS and a code-signing certificate for Windows, and signing steps in CI. GPG keys can't do either. Until then,
  the GPG-signed checksums and the build attestations are how users check those downloads.
