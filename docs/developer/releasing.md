# Releasing

How a release is made, for maintainers.

## Steps

1. On a branch from `main`:
   - set the new version in `Cargo.toml` (`[workspace.package]`), and run a build so `Cargo.lock` follows;
   - update the status line in `docs/features.md` if it names the version;
   - in `CHANGELOG.md`, rename the `## [Unreleased]` section to `## [x.y.z] - date`. Changes are added to
     Unreleased as they are made;
   - check the signature crates listed in [How it is built](architecture.md#signature-crates): if a pinned
     pre-release (`rsa`, `pkcs1`) has a final version, move to it; if RUSTSEC-2023-0071 has a fixed `rsa`, take it
     and remove the `--ignore` from the `audit` job in `ci.yml`; read any new advisory the `audit` job reports;
   - take compatible dependency updates (`cargo update`, then the signing cross-checks with
     `-- --ignored cross_check`); move to a new major version only as planned work. After a hayro upgrade, run the
     two ignored transparency group tests (`non_isolated_group_blends_against_page_backdrop`,
     `knockout_group_replaces_earlier_objects`); after an egui upgrade, check the workarounds that rely on egui's
     internal ids, `dialogs::slider`'s number box, and whether winit now takes drops on Wayland, compare eframe's
     default features with the ones the workspace lists (a new default is taken only by choice), and keep
     `limarepdf-gui`'s winit entry at the winit version eframe uses; after a winit
     upgrade, whether it now handles Finder's "open documents" event or lets an app set its own delegate, either
     of which changes `macos.rs` (see [How it is built](architecture.md), BUG-28);
   - check that the user guide's screenshots (`docs/images/`) still show the app as it is, and retake any that
     don't: a 1000×660 window in the light theme under a virtual X server, made-up documents and digital IDs only,
     cut down to the part the page is about.
2. Open a pull request. CI builds, tests and packages all four platforms; check the archives in the run's artifacts.
   Merge when it is green.
3. On `main`, after pulling the merge, make a signed tag and push it:

   ```
   git tag -s vX.Y.Z -m "LimarePDF X.Y.Z"
   git push origin vX.Y.Z
   ```

   The merge starts a CI run on `main` too. There is no need to wait for it: the tag run doesn't depend on it, and
   the two run side by side. The `main` run is still worth having, because it refreshes the build cache that pull
   requests and tags start from.

4. CI runs again for the tag. Its `release` job starts only once the `audit` job and every build have passed, so a
   new advisory published since the pull request stops the release: fix it (an update, or an `--ignore` with its
   reason in `ci.yml` and `docs/signatures.md` when it doesn't apply), then delete the tag
   (`git push --delete origin vX.Y.Z`, `git tag -d vX.Y.Z`) and tag the fixed `main` again. It checks that the tag
   matches `Cargo.toml`, takes the release notes
   from the matching `CHANGELOG.md` section, ends them with a link comparing the tag with the previous version tag,
   and creates a draft GitHub Release with the four archives, `SHA256SUMS`, and a GPG signature (`.asc`) for each.
   It also records a build attestation for every file.
5. On GitHub, open the draft under Releases, choose Edit, check the notes and files, and publish it. Publishing
   starts the `packages` workflow, which updates the apt and dnf repositories ([below](#package-repositories)).

Do not create the release on GitHub by hand, or the tag from GitHub's release form: the `release` job then finds a
release already there and fails.

## What CI builds

`.github/workflows/ci.yml` runs on pull requests, pushes to `main`, and `v*` tags. Each platform job runs the tests
in the `ci-test` profile (`Cargo.toml`: optimised with overflow checks, as released, but without link-time
optimisation, which every test executable would otherwise go through on its own), then builds the binaries with
`--release` and packages (both with `--locked`: built from `Cargo.lock` exactly, the versions the `audit` job
checked; a `Cargo.lock` out of date with `Cargo.toml` fails the job instead of being updated on the runner)
`limarepdf-<version>-<platform>.tar.gz` (`.zip` on Windows), holding the binaries, the README, `CHANGELOG.md`, `SECURITY.md`, `docs/` and
the licences. The archives are made on the runner because Actions artifacts lose Unix file permissions; on the
Actions page each artifact therefore downloads as a zip with the archive inside. The Mac builds target macOS 11
(`MACOSX_DEPLOYMENT_TARGET` at the top of the workflow; Rust's own default for Intel is 10.12).

The packages and installers (deb, rpm, MSI, dmg) don't carry `docs/`: `packaging/package-docs.sh` writes their
README, changelog and security notes into `target/package-docs/` with every relative link pointed at the version's
tag on GitHub (pages under `blob/`, images under `raw/`), so those links work once the version is tagged.
`packaging/check-package.py <folder>` checks a layout as installed: the programs, the README, changelog, security
notes, OCR model notes and every licence (the font ones included) there, every relative link reaching a file and
heading in the folder, and every link to the version's tag reaching one in the repository. CI runs it on each
archive before packing it, on the deb and rpm unpacked, on the MSI installed, and on the app in the mounted dmg;
the logs list each package's files.

The Linux job also makes `limarepdf_<version>-1_amd64.deb` and `limarepdf-<version>-1.x86_64.rpm` with `cargo-deb` and
`cargo-generate-rpm`, from the `[package.metadata.deb]` and `[package.metadata.generate-rpm]` sections of
`crates/limarepdf-gui/Cargo.toml`. Their dependency lists name the libraries limarepdf loads at run time; check them when
eframe or rfd changes (`strings target/release/limarepdf | grep '\.so'` lists them). The rpm's glibc version
(`libc.so.6(GLIBC_2.39)(64bit)`) is written by hand, since the automatic scan is off; the job fails, naming the
version to set, when the binaries need a different one (`ubuntu-latest` moving to a newer Ubuntu would do it:
then update the floor in install.md, the README and features.md too). The deb's comes from `dpkg-shlibdeps`. A quick install test:
`podman run --rm -v $PWD:/p:Z debian:stable bash -c 'apt-get update && apt-get install -y /p/limarepdf_*.deb && limarepdf-cli --version'`
(and the same with `fedora` and `dnf install -y /p/*.rpm`).

The `macos-dmg` job builds both Mac targets on one runner and runs `packaging/macos/make-dmg.sh`: it joins each
binary with `lipo`, makes `LimarePDF.app` from `packaging/macos/Info.plist` (which registers LimarePDF for PDFs) and an icon made
with `iconutil`, signs it ad hoc, and wraps it in `LimarePDF-<version>.dmg` with `hdiutil`. It then mounts the dmg,
checks the app's files and that both halves of each binary say macOS 11.

The Windows job also builds `limarepdf-<version>-x86_64.msi` from `packaging/windows/limarepdf.wxs` with WiX 5 (a .NET tool,
pinned because later versions add an EULA step) and its UI extension, `WixToolset.UI.wixext` (the same version), for
the installer screens. Its `UpgradeCode` must never change. The folder page is the installer's own (`LimareDirDlg`,
written for LimarePDF rather than copied from WiX, whose source is under the MS-RL licence), laid out like the
extension's pages and using its strings, with the desktop shortcut checkbox; two `Publish` lines route the
extension's Welcome and Ready pages to it. The checkbox sets `DESKTOPSHORTCUT`, which can also be given to
`msiexec`; the shortcut's registry value records the choice, and two property actions, run once before the
screens (or before installing, without them), keep it at an upgrade unless `DESKTOPSHORTCUT` is given, and turn
`DESKTOPSHORTCUT=0` into an unticked box. The job also builds the same installer as versions 255.0.0 and 255.0.1
and then installs on the runner without the screens: the files, the Start menu shortcut and the installed folder's
check; uninstalling; the desktop shortcut asked for, kept by an upgrade and removed by one with
`DESKTOPSHORTCUT=0`; and uninstalling again, leaving nothing. The screens themselves are only checked by hand
(testing.md's platform smoke checks).

The `release` job writes `SHA256SUMS`, signs it and every file with the signing subkey (below), checks each signature
against `docs/limarepdf-signing-key.asc`, and records build provenance with `actions/attest-build-provenance`, which
users check with `gh attestation verify`.

### Dry run

To try the release job without making a release, start the workflow by hand (Actions > ci > Run workflow) on a
branch with "dry_run" ticked. It builds, signs and attests as a release would, then uploads the files as the
`release-dry-run` artifact instead of creating a release. Like a release, it waits for the `audit` job: the run's
graph shows `release` after `audit`, `build` and `macos-dmg`, and a failed audit skips it. Its attestations are real but harmless: they name the
branch, not a tag.

## Package repositories

`.github/workflows/packages.yml` runs when a release is published. It downloads the `.deb` and `.rpm` files of the
last three published releases, builds a flat apt repository and a dnf repository with
`packaging/build-repos.sh`, signs them (`InRelease`, `Release.gpg`, `repomd.xml.asc`), checks the signatures against
`docs/limarepdf-signing-key.asc`, and deploys them to GitHub Pages at https://smaug42.github.io/limarepdf/ with the page in
`packaging/pages/`.

Set up once: Settings > Pages > Source: GitHub Actions; then, under Settings > Environments > github-pages, add a
deployment tag rule `v*`, since a release run's ref is its tag.

Started by hand (Actions > packages > Run workflow), it builds the site without deploying it unless "deploy" is
ticked; pull requests that change `packaging/` or the workflow build it the same way, as a check. (Starting it by hand
needs the workflow on `main`.) If no published release has packages yet, it takes them from the latest successful `release-dry-run` of
`ci.yml` on the chosen branch. To try the script locally, run it in an `ubuntu:24.04` container with `apt-utils`,
`createrepo-c` and `gnupg`, a throwaway key, and `LIMAREPDF_SITE_URL=file:///site`.

## Signing key

Releases are signed with a GPG key used only for LimarePDF: an ed25519 primary key that only certifies, kept offline,
and a signing subkey, the only part CI holds. The subkey expires after a year and the primary after three, so both
need renewing in time.

Making the key (once, on the maintainer's computer). `FPR` and `SUB` below stand for fingerprints: the 40-character
hex line that `gpg --list-keys` prints under the `pub ... [C]` line (the primary key, `FPR`) and under the
`sub ... [S]` line (the signing subkey, `SUB`). Paste them without spaces.

```
gpg --quick-gen-key "LimarePDF release signing <email>" ed25519 cert 3y
gpg --list-keys --with-subkey-fingerprints "LimarePDF release signing"   # note the primary fingerprint, FPR
gpg --quick-add-key FPR ed25519 sign 1y
gpg --list-keys --with-subkey-fingerprints FPR                     # note the signing subkey's fingerprint, SUB
```

Publishing the public key and giving CI the subkey (`gh` is the GitHub CLI; the `!` exports that subkey alone):

```
gpg --armor --export FPR > docs/limarepdf-signing-key.asc
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
export `docs/limarepdf-signing-key.asc` again and re-upload it to keys.openpgp.org. The secret in CI does not change.

If the subkey leaks, revoke it (`gpg --edit-key FPR`, `key 1`, `revkey`), add a new one, and replace both the
published key and the secret.

## To do

- **Code signing.** The macOS and Windows binaries are not code-signed or notarised, so Gatekeeper and SmartScreen
  warn on first launch (`docs/install.md` tells users how to proceed). Signing needs an Apple Developer account for
  macOS and a code-signing certificate for Windows, and signing steps in CI. GPG keys can't do either. Until then,
  the GPG-signed checksums and the build attestations are how users check those downloads.
