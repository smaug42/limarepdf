# Releasing

How a release is made, for maintainers.

## Steps

1. On a branch from `main`:
   - set the new version in `Cargo.toml` (`[workspace.package]`), and run a build so `Cargo.lock` follows;
   - update the status line in `docs/developer/feature-status.md` if it names the version;
   - in `CHANGELOG.md`, rename the `## [Unreleased]` section to `## [x.y.z] - date`. Changes are added to
     Unreleased as they are made;
   - check the signature crates listed in [How it is built](architecture.md#signature-crates) are at their latest
     versions, and update the table's versions;
   - run `cargo deny check` (`cargo install cargo-deny --locked --version` with the version `ci.yml`'s `deny` job
     sets) and read any new advisory or licence it reports. An advisory set aside in `deny.toml` whose crate has
     gone or been fixed is reported as no longer needed: remove it. Move the `deny` job's cargo-deny version to the
     latest release here, after reading its changelog for changes to `deny.toml`'s settings;
   - take compatible dependency updates (`cargo update`, then the signing cross-checks with
     `-- --ignored cross_check`); move to a new major version only as planned work. After a hayro upgrade, run the
     two ignored transparency group tests (`non_isolated_group_blends_against_page_backdrop`,
     `knockout_group_replaces_earlier_objects`); after an egui upgrade, check the workarounds that rely on egui's
     internal ids, `dialogs::slider`'s number box, and whether winit now takes drops on Wayland, compare eframe's
     default features with the ones the workspace lists (a new default is taken only by choice), and keep
     `limarepdf-gui`'s winit entry at the winit version eframe uses; after a winit
     upgrade, whether it now handles Finder's "open documents" event or lets an app set its own delegate, either
     of which changes `macos.rs` (see [How it is built](architecture.md), BUG-28);
   - after the updates, run `packaging/package-notices.sh` for each platform's targets (with the cargo-about
     version it names) and read what it reports ([Third-party licences](#third-party-licences)). A clarification in
     `about.toml` whose file changed is reported as a checksum mismatch: read the new file, then update that
     clarification's licence and checksum. A crate that brings new fonts or code under another licence needs a
     clarification there and a line in `check-package.py`'s `BUNDLED`, and its fonts named in the About window's
     credits and in licence.md. Move cargo-about to its latest release here, after reading its changelog: the
     version is set in `package-notices.sh`, in `ci.yml`'s two Install cargo-about steps, and in
     `packaging/arch/limarepdf/PKGBUILD` (`_about`, with the crate's checksum: `curl -fsSL
     https://static.crates.io/crates/cargo-about/cargo-about-<version>.crate | sha256sum`);
   - move the AppImage's runtime (`AppImage/type2-runtime`) and `appimagetool` (`AppImage/appimagetool`) to their
     latest numbered releases (not `continuous`) in `packaging/linux/make-appimage.sh`, after reading their
     changes, with each file's SHA-256 for both architectures from GitHub: `gh release view <tag> -R
     AppImage/type2-runtime --json assets --jq '.assets[] | [.name, .digest] | @tsv'`, and the same for
     appimagetool ([AppImage](#appimage));
   - merge or close Dependabot's open pull requests for the workflows' actions ([Pinned
     actions](#pinned-actions)), and check that `dtolnay/rust-toolchain` is at its `stable` branch's latest commit;
   - check that the user guide's screenshots (`docs/images/`) still show the app as it is, and retake any that
     don't: a 1000×660 window in the light theme under a virtual X server, made-up documents and digital IDs only,
     cut down to the part the page is about. `tools/docshots/docshots.sh` takes `comments`, `fill-and-sign`,
     `forms`, `inspector`, `organise`, `redaction` and `signatures-panel` this way (its header says how).
2. Open a pull request. CI builds, tests and packages every platform; check the archives in the run's artifacts.
   Merge when it is green.
3. On `main`, after pulling the merge, make a signed tag and push it:

   ```
   git tag -s vX.Y.Z -m "LimarePDF X.Y.Z"
   git push origin vX.Y.Z
   ```

   The merge starts a CI run on `main` too. There is no need to wait for it: the tag run doesn't depend on it, and
   the two run side by side. The `main` run is still worth having, because it refreshes the build cache that pull
   requests and tags start from.

4. CI runs again for the tag. Its `release` job starts only once the `deny` job and every build have passed, so a
   new advisory published since the pull request stops the release: fix it (an update, or an `ignore` entry with
   its reason in `deny.toml`, and in `docs/signatures-in-depth.md` for a signing crate, when it doesn't apply), then delete the tag
   (`git push --delete origin vX.Y.Z`, `git tag -d vX.Y.Z`) and tag the fixed `main` again. The job then waits for
   approval, since the signing key is only in the `release-signing` environment ([Signing key](#signing-key)).
   Nothing else links to the approval: open the run itself (the repository's Actions tab, the `ci` run for the tag),
   where a box says it is waiting for review; choose Review deployments, tick `release-signing`, then Approve and
   deploy. From a terminal instead, with the run's number:

   ```
   gh api repos/smaug42/limarepdf/actions/runs/<run>/pending_deployments --jq '.[].environment.id'
   gh api -X POST repos/smaug42/limarepdf/actions/runs/<run>/pending_deployments -F 'environment_ids[]=<id>' -f state=approved -f comment=X.Y.Z
   ```

   Before signing anything, the job checks that the tag matches `Cargo.toml`, takes the release notes from the
   matching `CHANGELOG.md` section, and ends them with a link comparing the tag with the previous version tag. It
   then signs and creates a draft GitHub Release with the archives, the source archive, the Linux packages and
   AppImages, the Arch Linux package (x86_64, built from source), the MSIs and the dmg, `SHA256SUMS`, and a GPG
   signature (`.asc`) for each, and records a build attestation for every file.
5. On GitHub, open the draft under Releases, choose Edit, check the notes and files, and publish it. Publishing
   starts the `packages` workflow, which updates the apt and dnf repositories ([below](#package-repositories)); its
   `sign` job waits for approval in the same way, on the `packages` run the release started.
6. Update winget and Scoop ([winget and Scoop](#winget-and-scoop)): with the clones in `win/winget-pkgs` and
   `win/scoop-limarepdf`, run `bash packaging/windows/update.sh X.Y.Z win`. It checks the published `SHA256SUMS`
   against the release key, downloads the MSIs to read their codes, and writes winget's three manifests and Scoop's
   `bucket/limarepdf.json`. In the bucket, read the change, commit it with the message `limarepdf: Update to version
   X.Y.Z` and push. In winget-pkgs, on a new branch from the fork's updated `master`, commit the new folder with the
   message `New version: ClaytonCornell.LimarePDF version X.Y.Z`, push it and open a pull request to
   `microsoft/winget-pkgs`; its checks and a moderator's review take a day or more.

Do not create the release on GitHub by hand, or the tag from GitHub's release form: the `release` job then finds a
release already there and fails.

## What CI builds

`.github/workflows/ci.yml` runs on pull requests, pushes to `main`, and `v*` tags. Each platform job runs the tests
in the `ci-test` profile (`Cargo.toml`: optimised with overflow checks, as released, but without link-time
optimisation, which every test executable would otherwise go through on its own), then builds the binaries with
`--release` and packages (both with `--locked`: built from `Cargo.lock` exactly, the versions the `deny` job
checked; a `Cargo.lock` out of date with `Cargo.toml` fails the job instead of being updated on the runner)
`limarepdf-<version>-<platform>.tar.gz` (`.zip` on Windows), holding the binaries, the README, `CHANGELOG.md`, `SECURITY.md`, `docs/` and
the licences. The archives are made on the runner because Actions artifacts lose Unix file permissions; on the
Actions page each artifact therefore downloads as a zip with the archive inside. The Mac builds target macOS 11
(`MACOSX_DEPLOYMENT_TARGET` at the top of the workflow; Rust's own default for Intel is 10.12).

Each job keeps its own build cache (`Swatinem/rust-cache`), and only runs on `main` save one (`save-if`). Pull
requests and tags restore `main`'s, so a pull request's caches can't push `main`'s out of the repository's 10 GB,
which left the next run building every crate again (an hour or more). A pull request that changes `Cargo.lock`
builds the crates it changes on each push.

There are six platform jobs: Linux and Windows on x86_64 and on Arm64 (`ubuntu-24.04-arm`, `windows-11-arm`), and
the two Macs. Each builds natively, on a runner of its own architecture, and gives its target triple (the matrix's
`target`) to `check-package.py`, which checks that the programs' headers name that architecture: a runner's
toolchain emulating its other architecture would build programs that run there, but aren't the package's.

The Windows programs carry the C runtime (`+crt-static` in `.cargo/config.toml`, which a `RUSTFLAGS` set in the
environment replaces): the Visual C++ runtime's DLLs aren't part of Windows, so programs needing them don't start on
a computer that hasn't installed them (#324). GitHub's Windows runners have them, so no install or run there would
show it; `check-package.py` reads the programs' DLLs instead.

The packages and installers (deb, rpm, MSI, dmg) don't carry `docs/`: `packaging/package-docs.sh` writes their
README, changelog and security notes into `target/package-docs/` with every relative link pointed at the version's
tag on GitHub (pages under `blob/`, images under `raw/`), so those links work once the version is tagged.
`packaging/check-package.py [--target TRIPLE]... <folder>` checks a layout as installed: the programs, built for the
targets' architectures (each one's ELF, PE or Mach-O header says, universal or not) and, on Windows, importing none
of the Visual C++ runtime's DLLs (`VCRUNTIME140.dll` and the like, delay-loaded or not), the README, changelog,
security notes, OCR model notes and every licence (the font ones and the crates' included) there, every relative
link reaching a file and heading in the folder, every link to the version's tag reaching one in the repository, and
the crates' licences matching the crates built ([below](#third-party-licences)). CI runs it on each archive before
packing it, on the deb, rpm, AppImage and Arch packages unpacked, on the MSI installed, and on the app in the mounted
dmg; the logs list each package's files.

The Linux jobs also make `limarepdf_<version>-1_amd64.deb` and `limarepdf-<version>-1.x86_64.rpm` (on Arm,
`limarepdf_<version>-1_arm64.deb` and `limarepdf-<version>-1.aarch64.rpm`) with `cargo-deb` and
`cargo-generate-rpm`, from the `[package.metadata.deb]` and `[package.metadata.generate-rpm]` sections of
`crates/limarepdf-gui/Cargo.toml`. Their dependency lists, and the Arch packages' `depends`, name the libraries
limarepdf loads at run time; check them when eframe or rfd changes (`strings target/release/limarepdf | grep
'\.so'` lists them). The rpm's glibc version
(`libc.so.6(GLIBC_2.39)(64bit)`) is written by hand, since the automatic scan is off; each job fails when the
binaries need a different one. That version is the floor install.md names. The binaries need the glibc of the
Ubuntu they were built on, so the Linux jobs run on `ubuntu-24.04` and `ubuntu-24.04-arm` (glibc 2.39), not
`ubuntu-latest`, whose move to Ubuntu 26.04 raised the floor to 2.43 for 0.14.0 (for a few float functions alone).
Raising the floor is a decision, not a fix for this check: when GitHub retires the `ubuntu-24.04` runners, or a
library comes to need a newer glibc, change the runners, the rpm's version, the `linux-install` job's images and the
floor in install.md, the README, feature-status.md, testing.md and `packaging/pages/index.html` together. The deb's
comes from `dpkg-shlibdeps`.

The `linux-install` job installs the deb and the rpm of each architecture, each in a clean container on a runner of
that architecture, on the oldest systems install.md names (Ubuntu 24.04, Debian 13, AlmaLinux 10, openSUSE Leap 16)
and on a current Fedora, with `packaging/check-linux-install.sh`: the system's package manager installs the package
with the libraries it depends on from the system's own repositories (recommended packages left out), `ldd` finds
every library and glibc version both programs need, `limarepdf-cli --version` runs, and removing the package leaves
neither program behind. Before removing it, the script unpacks the AppImage ([below](#appimage)) there, checks with
`ldd` that both programs load on the libraries the package brought (those a desktop has), and runs `limarepdf-cli
--version` through the AppImage with `APPIMAGE_EXTRACT_AND_RUN=1`, since containers have no FUSE. The release and
dry-run jobs wait for it. To run it locally, with the packages and the AppImage in `dist/`:
`podman run --rm -v "$PWD:/w:ro" -w /w docker.io/library/ubuntu:24.04 bash packaging/check-linux-install.sh dist`,
and the same with each of the job's other images.

The `macos-dmg` job builds both Mac targets on one runner and runs `packaging/macos/make-dmg.sh`: it joins each
binary with `lipo`, makes `LimarePDF.app` from `packaging/macos/Info.plist` (which registers LimarePDF for PDFs) and an icon made
with `iconutil`, signs it ad hoc, and wraps it in `LimarePDF-<version>.dmg` with `hdiutil`. It then mounts the dmg,
checks the app's files and that both halves of each binary say macOS 11.

The Windows jobs also build `limarepdf-<version>-x86_64.msi`, and on Arm `limarepdf-<version>-arm64.msi` (`-arch
arm64`), from `packaging/windows/limarepdf.wxs` with WiX 5 (a .NET tool, pinned because later versions add an EULA
step) and its UI extension, `WixToolset.UI.wixext` (the same version), for the installer screens. Its
`UpgradeCode` must never change; both architectures share it, and `MajorUpgrade` replaces a LimarePDF of the same
version too (`AllowSameVersionUpgrades`), so the Arm64 installer takes the place of an x64 LimarePDF rather than
installing a second one beside it. The folder page is the installer's own (`LimareDirDlg`, written for LimarePDF
rather than copied from WiX, whose source is under the MS-RL licence), laid out like the extension's pages and
using its strings, with the desktop shortcut checkbox; two `Publish` lines route the extension's Welcome and Ready
pages to it. The checkbox sets `DESKTOPSHORTCUT`, which can also be given to `msiexec`; the shortcut's registry
value records the choice, and two property actions, run once before the screens (or before installing, without
them), keep it at an upgrade unless `DESKTOPSHORTCUT` is given, and turn `DESKTOPSHORTCUT=0` into an unticked box.
The job also builds the same installer as versions 255.0.0 and 255.0.1 and then installs on the runner without the
screens: the files, the Start menu shortcut and the installed folder's check; `limarepdf.exe`'s name in its version
information (`FileDescription`), which `build.rs` embeds with the icon and a failed resource compile leaves out with
only a build warning; uninstalling; the desktop shortcut asked for, kept by an upgrade and removed by one with
`DESKTOPSHORTCUT=0`; and uninstalling again, leaving nothing. On Arm, which runs x64 programs too, it then installs
an x64 MSI of the same version and the Arm64 one over it, and expects one LimarePDF installed, and none after
uninstalling. The screens themselves are only checked by hand (testing.md's platform smoke checks).

After the release build, the platform jobs and `macos-dmg` check that neither program has core's test seam (the
`test-seams` feature, which only the app's and the CLI's tests turn on: architecture.md's "atomic.rs" paragraph),
by looking for its variable's name, `LIMAREPDF_TEST_SYNC_FAILS`, in the binaries.

The `deny` job runs `cargo deny check` with `deny.toml` on every crate in `Cargo.lock`, for every platform: known
advisories (RustSec; those set aside are listed in `deny.toml` with their reasons), licences (an allow list matching
what the packages carry; a new licence fails until it is looked at), crates from crates.io only, and two versions of
one crate (reported, not refused). The workspace's crates are `publish = false`, which lets them use each other by
path.

The `lint` job runs `cargo fmt --all --check`, then `cargo clippy --workspace --all-targets --locked -- -D warnings`
for `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-gnu` and `aarch64-apple-darwin` in turn, reporting every
platform that fails. Clippy checks without linking, so one Linux runner covers the code of all three; the Windows GNU
target lints the same code as the MSVC build, since LimarePDF's code has no `cfg` on the toolchain kind, and one Mac
target is enough while none is on the architecture (a `target_env` or `target_arch` check added to LimarePDF's code
means adding that target here). The same commands run locally once the two targets are added with `rustup target
add`. Clippy comes with the stable toolchain, so a new Rust release can bring new lints: fix them in their own
change. The release job doesn't wait for it, since a lint doesn't change what is built; pull requests show it.

The `release` job runs only for a version tag, in the `release-signing` environment, once the maintainer approves
it. After its checks of the tag and the notes, `packaging/import-signing-key.sh` imports the signing subkey (below)
and `packaging/sign-release.sh` signs the rpms, writes `SHA256SUMS`, and signs it and every file with that subkey,
checking each signature against `docs/limarepdf-signing-key.asc` as rpm and gpgv will. The job then records build
provenance with `actions/attest-build-provenance`, which users check with `gh attestation verify`. A tag run whose
environment has no `GPG_SIGNING_KEY` fails at the signing step, with nothing signed or released.

### AppImage

After the deb and rpm, the Linux jobs make `LimarePDF-<version>-x86_64.AppImage` (on Arm,
`LimarePDF-<version>-aarch64.AppImage`) from the archive with `packaging/linux/make-appimage.sh`. The AppImage holds
the programs in `usr/bin`, the desktop entry and icon where a package puts them (and at the top, as an AppImage has
them), the rest of the archive in `usr/share/doc/limarepdf`, and `packaging/linux/AppRun`. AppRun starts
`limarepdf-cli` when the AppImage is started through a link of that name or given it as its first argument, and
`limarepdf` otherwise. The runtime, the program at the front of the file that mounts the rest, is AppImage's static
one: it needs no library, libfuse2 included, only `fusermount3` to mount itself, or nothing with
`--appimage-extract-and-run`. The script downloads it and `appimagetool` at set versions, checking each by its
SHA-256 (step 1 moves them). Nothing else is bundled: the programs load glibc and the desktop's libraries from the
system, as the packages' do, so the AppImage has the same glibc floor. `appimagetool` runs `desktop-file-validate`
on the desktop entry when it is installed, as the job installs it.

`packaging/linux/check-appimage.sh` then checks that the runtime is static and built for the job's architecture, the
files unpacked with `check-package.py`, AppRun with stand-ins for the programs (which one starts, by the name and by
the first argument, with the arguments passed on unchanged), and `limarepdf-cli --version` through the AppImage
both ways, with FUSE (`fuse3`). The `linux-install` job then runs the AppImage on each system
([above](#what-ci-builds)). To make and check one locally, on Linux with FUSE, from the archive in `dist/`:

```
bash packaging/linux/make-appimage.sh dist/limarepdf-<version>-linux-x86_64.tar.gz dist
bash packaging/linux/check-appimage.sh dist/LimarePDF-<version>-x86_64.AppImage x86_64-unknown-linux-gnu
```

### Arch Linux packages

`packaging/arch/` holds the PKGBUILDs of two packages written for the AUR, each for x86_64 and aarch64:
`limarepdf`, built from the release's source archive, and `limarepdf-bin`, from its Linux archives. Both install
what the release archive holds: the programs in `/usr/bin`, the desktop entry and icon, the licences in
`/usr/share/licenses/<package>`, and the rest in `/usr/share/doc/limarepdf`, with links there to the two licences
the guide links to. `limarepdf` makes that archive with `packaging/package-archive.sh`, as the Linux jobs do, with
cargo-about built at the version it names. makepkg's own link-time optimisation is off there (`!lto`): it builds C
code (ring's, in cargo-about) as GCC LTO objects that Rust's linker can't read, and the release profile does Rust's
own. Each package checks the release files' signatures against the release key (`validpgpkeys`).

The PKGBUILDs here carry a placeholder version and checksums (`SKIP`). `packaging/arch/update.sh [--pkgrel N]
<version> <out> [<dist>]` writes the AUR's copies into `<out>/limarepdf` and `<out>/limarepdf-bin`, each with the
version, the checksums and its `.SRCINFO`: for a release, from its `SHA256SUMS` once the signature checks against
`docs/limarepdf-signing-key.asc`; given `<dist>`, from the files there. `--pkgrel N` is for a changed PKGBUILD of a
version already on the AUR (2, then 3).

The `arch-source` job makes the release's source archive, `limarepdf-<version>-source.tar.gz` (`git archive`), and
builds `limarepdf` from it in a current `archlinux:base-devel` container while the platforms build. Both go in the
release, the package as `limarepdf-<version>-1-x86_64.pkg.tar.zst` for users who would rather not build it, and the
package to the `arch` job's checks too. After the builds, the `arch` job builds `limarepdf-bin` for
both architectures from the Linux archives, and one with pkgrel 0.1 to upgrade from, then checks the three packages:
namcap (an error fails it; its warnings that the libraries the app loads at run time look unneeded are expected,
and that `limarepdf-bin` should say `$CARCH` for the x86_64 in the archive's name, which is arm64 on aarch64),
every file of the release archive in its place and nothing else, pacman's modes, no test seam, and
`check-package.py`. On x86_64 it then installs the earlier `limarepdf-bin`, upgrades it, replaces it with
`limarepdf` and removes that. After each step: `pacman -Qkk`, the command line rotating a document, and the app's
window open on it under X11 (Xvfb, found by its title) and Wayland (headless Weston: the title set and the window's
surface drawn, read from `WAYLAND_DEBUG`); after removing, nothing left, and the user's files unchanged throughout.
The Arm64 package is built and checked but not installed, as there is no official Arch Linux image for Arm64
(testing.md's platforms). Both jobs run `packaging/arch/check-arch.sh`, which signs the files with a key made for the
run, and the release and dry-run jobs wait for `arch`. To run it locally, with the linux-x86_64 and linux-arm64
archives in `dist/`:

```
git archive --prefix="limarepdf-<version>/" HEAD | gzip -n > dist/limarepdf-<version>-source.tar.gz
podman run --rm -v "$PWD:/w:ro" -v "$PWD/dist:/dist" -w /w docker.io/library/archlinux:base-devel bash packaging/arch/check-arch.sh source /dist
podman run --rm -v "$PWD:/w:ro" -w /w docker.io/library/archlinux:base-devel bash packaging/arch/check-arch.sh dist
```

The packages aren't on the AUR: it isn't taking new accounts for now (found 2026-11-25), so the release's x86_64
`limarepdf` is how users install them, with `pacman -U` (#319). When it takes new accounts again, set up once: an
account on aur.archlinux.org with the maintainer's SSH key, then `git clone
ssh://aur@aur.archlinux.org/limarepdf.git aur/limarepdf`, and the same for `limarepdf-bin`. A name not yet taken
clones empty, and the first push makes the package. Then, after each release is published, on Arch Linux: `bash
packaging/arch/update.sh X.Y.Z aur` checks the published `SHA256SUMS` against the release key and writes each
package's `PKGBUILD` and `.SRCINFO`; read the changes (`git -C aur/limarepdf diff`), then in each repository commit
both files with the message `X.Y.Z` and push. After the first push, this becomes a release step again, and
install.md's Arch Linux section names the AUR.

### winget and Scoop

`packaging/windows/` holds the templates of winget's three manifests for `ClaytonCornell.LimarePDF` (`winget/`:
version, installer and the en-US locale), which install the x64 and Arm64 MSIs, and of Scoop's manifest
(`scoop/limarepdf.json`), which unpacks the Windows zip of the computer's architecture. Each fill-in is written
`@NAME@`. `packaging/windows/update.sh <version> <out> [<dist>]` fills them in: the version, the checksums (for a
release, from its `SHA256SUMS` once the signature checks against `docs/limarepdf-signing-key.asc`; given `<dist>`,
from the files there), and each MSI's product code with the upgrade code, publisher and name they share, read with
msitools' `msiinfo`. It fails when an MSI doesn't match `SHA256SUMS`, isn't for the version, or the two differ in
those, and when a fill-in is left. It writes winget's into
`<out>/winget-pkgs/manifests/c/ClaytonCornell/LimarePDF/<version>/` and Scoop's into
`<out>/scoop-limarepdf/bucket/limarepdf.json`. The manifests name no dependencies: the programs need nothing Windows
doesn't have. 0.14.1's installer manifest names the Visual C++ runtime (`Microsoft.VCRedist.2015+.x64` and `.arm64`,
added by hand), which its programs needed (#324).

After the builds, the `winget-scoop` job writes the manifests from the MSIs and zips, and `winget-scoop-check` checks
them on Windows x64 and Arm64 with `packaging/windows/check-manifests.ps1 <manifests> <dist>`. Every download address
in them must be the release's, for one of the four files; copies then point at the files served on the runner
(Python's `http.server`) in place of the release. winget validates its manifests as written, installs LimarePDF from
the copies and removes it again; Scoop then installs it from its copy and removes it. After each install, the
programs are the same as in the architecture's zip and `limarepdf-cli --version` names the version (through Scoop's
shim, for Scoop), with Scoop's Start menu shortcut there; after each removal, nothing is left. A runner's winget
older than 1.28, which can't read the manifests' schema, or none (the Arm64 runner's) is replaced by the release
the script names, and Scoop is installed with its installer at the commit the script names, each checked against
the checksum written there; update them there by hand. The release and dry-run jobs wait for
`winget-scoop-check`. To run it on a Windows computer, as an administrator, with the Windows files in `dist\` and
the manifests written from them (`update.sh` on Linux): `pwsh packaging/windows/check-manifests.ps1 manifests dist`.
It installs LimarePDF and removes it, and installs Scoop.

Set up once: a repository `smaug42/scoop-limarepdf` holding `bucket/limarepdf.json` and a README with the two
commands in install.md, cloned into `win/scoop-limarepdf`; and a fork of `microsoft/winget-pkgs`, which is large,
cloned sparse:

```
gh repo fork microsoft/winget-pkgs --clone=false
git clone --filter=blob:none --sparse https://github.com/smaug42/winget-pkgs win/winget-pkgs
git -C win/winget-pkgs sparse-checkout set manifests/c/ClaytonCornell
```

Before each version, bring the fork up to date (`gh repo sync smaug42/winget-pkgs`, then `git -C win/winget-pkgs
pull`). The first pull request asks for Microsoft's contributor licence agreement to be signed. Once the first
version is in winget (Scoop's bucket has had it since 0.14.1), remove install.md's "Not in winget yet" line.

### Third-party licences

Every package carries `THIRD-PARTY-LICENSES.html`, the licences of the crates built into its programs.
`packaging/package-notices.sh [target]...` writes it into `target/package-notices/` with cargo-about, at the
version the script names (`ci.yml` installs the same one), from `about.toml` and the template
`packaging/about.hbs`. It covers the targets given, the computer's own if none (the dmg gives both Mac targets),
leaving out build and dev dependencies and the workspace's own crates. It stops, leaving no page, when
`about.toml`'s accepted licences differ from `deny.toml`'s allow list, or when cargo-about warns.

`about.toml`'s clarifications add what cargo-about can't find by itself: the fonts that hayro-font and
epaint_default_fonts bundle, and the code notices in subsetter's NOTICE, each file pinned by its SHA-256. When a
new version of the crate changes one of those files, cargo-about only warns and leaves the notice out, which is why
the script stops on a warning. sctk-adwaita's Cantarell font, which draws the window title bars on Linux under
Wayland, comes with no licence file in the crate (its notice is in the font), so the Linux layouts carry
`packaging/linux/FONT-LICENSE-Cantarell.txt` instead.

`check-package.py` compares the page with the crates `cargo tree -e normal` lists for the folder's targets
(`--target`, the computer's own if none; both Macs for the dmg), less the workspace's own. There must be a notice
for each crate built, by name and version, and none for a crate that isn't. The notices for bundled fonts and code
(its `BUNDLED` list) must be there for each crate that bundles them. `check-package.py --self-test`, which the
`lint` job runs, checks the crate list against a small workspace in `tools/tests/package/` with each kind of
dependency, and checks the notices against made-up pages. The Linux packages and the MSI take the page from the
Package step; to build them locally, run `package-notices.sh` first, as with `package-docs.sh`.

### Dry run

To try the release job without making a release, start the workflow by hand (Actions > ci > Run workflow) on a
branch with "dry_run" ticked. The `dry-run` job runs instead of `release`, with no secrets and no approval, so it
signs and attests nothing for real:

- `packaging/check-signing-key.sh` fails it if the published key (`docs/limarepdf-signing-key.asc`) can't sign for
  another 60 days, when its primary key, or every signing subkey, expires or has been revoked by then
  ([renew it](#signing-key));
- the signing is rehearsed on a copy of the files with the release job's own scripts: a key made for the run alone
  is imported with `packaging/import-signing-key.sh`, as at a tag, and `packaging/sign-release.sh` signs with it,
  checking each signature against it;
- the files themselves get `SHA256SUMS` only, and are uploaded unsigned as the `release-dry-run-unsigned` artifact,
  with a notice on the run and its summary saying so.

Like a release, it waits for the `deny` job: the run's graph shows `dry-run` after `deny`, `build`, `macos-dmg`, `linux-install`, `arch` and `winget-scoop-check`,
and a failed `deny` skips it. What it can't try is the release key itself in CI (the environment's secrets and the
approval): a tag run, or `packages.yml`'s `sign` job, is the first to use them.

### Pinned actions

Every action in the workflows is pinned by the full SHA of a commit, with its version in a comment
(`actions/checkout@3d3c42e… # v7.0.1`), so a moved or replaced tag can't change what runs. Dependabot
(`.github/dependabot.yml`) checks them weekly and opens one pull request for all the updates; Cargo updates aren't
part of it, since they come with each release's preparation (step 1) and some crates are pinned on purpose.
`dtolnay/rust-toolchain` has no releases. It is pinned to its `stable` branch's latest commit
(`gh api repos/dtolnay/rust-toolchain/commits/stable --jq .sha`), which is where Dependabot moves every pin of it,
whatever the comment says. So every step that uses it names its toolchain (`toolchain: stable`, or `nightly` in
`fuzz.yml`) rather than taking the pinned branch's default, and a moved pin can't change what is installed. A new
action is added pinned the same way: `gh api repos/OWNER/REPO/commits/vX.Y.Z --jq .sha`.

## Package repositories

`.github/workflows/packages.yml` runs when a release is published. Its `build` job downloads the `.deb` and `.rpm`
files of the last three published releases, builds a flat apt repository and a dnf repository with
`packaging/build-repos.sh` (each holding both architectures' packages, from which apt and dnf pick the system's
own; the apt one's `Release` names the architectures of its debs), signed with a key made for the run alone, and
checks the signatures against that key.
Its `sign` job imports the release key with `packaging/import-signing-key.sh`, builds them again from the same
packages, signed with that key (`InRelease`, `Release.gpg`, `repomd.xml.asc`), and checks the signatures against
`docs/limarepdf-signing-key.asc`; like the tag's `release` job, it runs in the `release-signing` environment and
waits for the maintainer's approval (Review deployments on the run). The `deploy` job then deploys them to GitHub
Pages at https://smaug42.github.io/limarepdf/ with the page in `packaging/pages/`.

Set up once: Settings > Pages > Source: GitHub Actions; then, under Settings > Environments > github-pages, add a
deployment tag rule `v*`, since a release run's ref is its tag.

Started by hand (Actions > packages > Run workflow), it runs the `build` job alone unless "deploy" is ticked; pull
requests that change `packaging/` or the workflow run it the same way, as a check that needs no secrets. (Starting it
by hand needs the workflow on `main`.) Only runs that deploy wait for one another, one at a time and never cancelled,
so a pull request's run can't take the place of a deploy waiting for approval. With no published release that has
packages, the `build` job fails. To try the script locally, run it in an `ubuntu:24.04` container with `apt-utils`,
`createrepo-c` and `gnupg`, a throwaway key, and `LIMAREPDF_SITE_URL=file:///site`.

## Signing key

Releases are signed with a GPG key used only for LimarePDF: an ed25519 primary key that only certifies, kept offline,
and a signing subkey, the only part CI holds. The subkey expires after a year and the primary after three, so both
need renewing in time; a dry run fails 60 days before either can no longer sign.

CI holds the subkey and its passphrase as secrets of the `release-signing` environment, never as repository secrets:
a repository secret reaches every job of every workflow, while an environment's secrets reach only a job that names
the environment, and only once the maintainer approves that run. Two jobs name it: the tag's `release` job in
`ci.yml` and the `sign` job in `packages.yml`. Neither runs for a pull request, so a pull request (from a fork,
Dependabot or a branch) never gets the key. `tools/check-workflows.py`, which the `lint` job runs, fails when a
workflow names the signing secrets outside such a job; it is an early warning, and the environment is the
protection.

Setting up the environment (once, on GitHub): Settings > Environments > New environment, named `release-signing`.
Under Deployment protection rules, tick Required reviewers and add the maintainer; leave Prevent self-review
unticked, since the maintainer approves their own tag's run, and untick Allow administrators to bypass configured
protection rules. Under Deployment branches and tags, choose Selected branches and tags and add the branch `main`
and the tag `v*`.

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
gpg --armor --export-secret-subkeys SUB! | gh secret set GPG_SIGNING_KEY --env release-signing
gh secret set GPG_PASSPHRASE --env release-signing          # type the key's passphrase
```

Without `gh`, add the two secrets on GitHub under Settings > Environments > release-signing > Add environment
secret, pasting the subkey export (copied with `gpg --armor --export-secret-subkeys SUB! | wl-copy`, so it never
touches the disk) and the passphrase. `gh secret list --env release-signing` then lists both, and `gh secret list`
(the repository's) neither. A `GPG_` secret found there, such as one from before the environment existed, is
deleted with `gh secret delete NAME` once the environment has its own copy, never before.

Then upload the public key at keys.openpgp.org (it emails a confirmation link), put the primary fingerprint in
`docs/install.md`, and keep a backup of the whole key (`gpg --armor --export-secret-keys FPR`) and its revocation
certificate (`~/.gnupg/openpgp-revocs.d/FPR.rev`) off the computer, for example on a USB stick kept elsewhere.

Renewing: `gpg --quick-set-expire FPR 3y` for the primary, `gpg --quick-set-expire FPR 1y SUB` for the subkey, then
export `docs/limarepdf-signing-key.asc` again, commit it before the next tag, and re-upload it to keys.openpgp.org.
CI's secret needs no change: the copy it holds keeps the expiry dates it was exported with, but the jobs that sign
import the published key after it (`packaging/import-signing-key.sh`), and gpg takes the renewed dates from the key
in the commit the job runs on.

If the subkey leaks, revoke it (`gpg --edit-key FPR`, `key 1`, `revkey`), add a new one, and replace both the
published key and the environment's secret.

## To do

- **Code signing.** The macOS and Windows binaries are not code-signed or notarised, so Gatekeeper and SmartScreen
  warn on first launch (`docs/install.md` tells users how to proceed). Signing needs an Apple Developer account for
  macOS and a code-signing certificate for Windows, and signing steps in CI. GPG keys can't do either. Until then,
  the GPG-signed checksums and the build attestations are how users check those downloads.
