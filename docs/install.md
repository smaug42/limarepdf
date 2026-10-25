[User guide](README.md) › Getting started

# Install LimarePDF

LimarePDF comes as two programs: the desktop app (`limarepdf`) and the command-line tool (`limarepdf-cli`). Every
way of installing below gives you both. Downloads are on the project's
[Releases page](https://github.com/smaug42/limarepdf/releases) on GitHub.

Pick your system:

- **Windows:** the [installer](#windows-installer).
- **macOS:** the [app](#macos-app).
- **Linux:** the [package repository](#linux-packages), which keeps LimarePDF updated with the rest of your system.
- **Without installing**, on any of them: an [archive](#archives) you unpack anywhere.

## Requirements

- **Windows:** Windows 10 or 11, 64-bit (x64).
- **macOS:** macOS 11 (Big Sur) or later, on Apple Silicon or Intel.
- **Linux:** 64-bit (x86_64) with glibc 2.39 or later, as in Ubuntu 24.04, Debian 13, Fedora 40, current Arch
  Linux and later; a working OpenGL graphics driver; and an X11 or Wayland desktop session. On older systems, you
  can [build LimarePDF from source](#build-from-source). If you'd like packages for an older system,
  [say so](troubleshooting.md#reporting-a-problem): they could be made if enough people need them.
- **Printing:** CUPS on Linux and macOS (macOS includes it).

Support for all three is **provisional** for now. Every change is built and its automated tests run on each of
them, but the release checks by hand haven't all passed on each yet: installing and uninstalling, opening a PDF
from the file manager, editing and saving, recovery after a crash, printing, and display scaling. They are run
before LimarePDF 1.0. So far:

| System | Checked by hand |
| --- | --- |
| Linux | Used every day; printing on paper checked; not yet the whole set of release checks |
| Windows | Installing and starting (0.5); the print dialog |
| macOS | None yet |

## Windows installer

Download `limarepdf-<version>-x86_64.msi` and open it. The installer lets you choose the folder
(`Program Files\LimarePDF` by default) and, under it, whether to create a desktop shortcut (not ticked unless you
tick it). It installs LimarePDF for all users, adds it to the Start menu and to Open with for PDF files, and puts
`limarepdf-cli` on the `PATH` (open a new terminal to use it).

For an install without the screens, `msiexec /i limarepdf-<version>-x86_64.msi /qn` installs with no desktop
shortcut; add `DESKTOPSHORTCUT=1` for one. Installing a newer version keeps the shortcut choice made before, unless
you give `DESKTOPSHORTCUT` again (`DESKTOPSHORTCUT=0` removes the shortcut).

The installer is not code-signed yet, so SmartScreen warns: choose More info, then Run anyway.

To make LimarePDF the default PDF app, choose it in Settings > Apps > Default apps. Installing a newer version
replaces the old one; uninstall from Settings > Apps.

## macOS app

Download `LimarePDF-<version>.dmg`, open it and drag LimarePDF to Applications. It is one app for both Apple Silicon
and Intel Macs. LimarePDF then appears under Open With for PDF files, and a PDF opened that way, or dropped on its
Dock icon, opens in LimarePDF.

LimarePDF is not signed with an Apple Developer ID yet, so the first start says Apple cannot check it: choose Done,
then open System Settings > Privacy & Security, choose Open Anyway next to LimarePDF, and confirm.

The command-line tool is inside the app; to use it from a terminal:

```
sudo ln -s /Applications/LimarePDF.app/Contents/MacOS/limarepdf-cli /usr/local/bin/limarepdf-cli
```

## Linux packages

The easiest way is LimarePDF's package repository, which brings updates with the rest of your system. It is signed
with the project's key (see [Checking a download](#checking-a-download)).

Debian and Ubuntu:

```
sudo install -d /etc/apt/keyrings
sudo curl -fsSLo /etc/apt/keyrings/limarepdf.asc https://smaug42.github.io/limarepdf/limarepdf-signing-key.asc
sudo curl -fsSLo /etc/apt/sources.list.d/limarepdf.sources https://smaug42.github.io/limarepdf/apt/limarepdf.sources
sudo apt update && sudo apt install limarepdf
```

Fedora: `sudo dnf config-manager addrepo --from-repofile=https://smaug42.github.io/limarepdf/rpm/limarepdf.repo`, then
`sudo dnf install limarepdf`. openSUSE: `sudo zypper addrepo https://smaug42.github.io/limarepdf/rpm/limarepdf.repo`, then
`sudo zypper install limarepdf`.

Each release also has a `.deb` (Debian 13, Ubuntu 24.04 and later) and an `.rpm` (Fedora 40 and later, openSUSE
Tumbleweed) to download and install by hand. Like the archives, they need glibc 2.39 or later; on older
systems, [build from source](#build-from-source). They install `limarepdf` and `limarepdf-cli` in `/usr/bin`, with
the menu entry, the icon, the licences and a README that links to this guide online, and pull in the libraries the
app needs.

```
sudo apt install ./limarepdf_<version>-1_amd64.deb
sudo dnf install ./limarepdf-<version>-1.x86_64.rpm
```

The `.rpm` is signed with the project's key; to have rpm check it, import the key first with
`sudo rpm --import limarepdf-signing-key.asc`.

## Archives

To use LimarePDF without installing it, download the archive for your system: Linux (`.tar.gz`), Windows (`.zip`),
or macOS for Intel or for Apple Silicon (`.tar.gz`). Unpack it anywhere and run `limarepdf` from there. Each
archive holds the desktop app, the command-line tool, these docs, the changelog, and the licences, including those
of the bundled text recognition model and fonts.

The programs are not code-signed yet, so the system warns the first time you run them. On Windows, choose More
info, then Run anyway. On macOS, run `xattr -d com.apple.quarantine limarepdf-cli limarepdf` in the unpacked
folder, or try to run it once, then choose Open Anyway under System Settings > Privacy & Security.

### Menu entry and icon on Linux

The packages set this up; for the archive, do it yourself. Without a menu entry, Wayland desktops (GNOME, KDE
Plasma and others) show the window with a generic icon, since they take a window's icon from its entry. The Linux
archive includes `io.github.smaug42.limarepdf.desktop` and `io.github.smaug42.limarepdf.png`; to install them for
your user, run in the unpacked folder:

```
install -Dm644 io.github.smaug42.limarepdf.desktop ~/.local/share/applications/io.github.smaug42.limarepdf.desktop
install -Dm644 io.github.smaug42.limarepdf.png ~/.local/share/icons/hicolor/256x256/apps/io.github.smaug42.limarepdf.png
```

The entry runs `limarepdf`, so put `limarepdf` on your `PATH` (for example in `~/.local/bin`), or change the `Exec=` line
to its full path. LimarePDF then appears in your applications menu and in the list of apps that can open PDF files.
When building from source, the files are `crates/limarepdf-gui/assets/io.github.smaug42.limarepdf.desktop` and
`docs/images/icon.png`.

## Start LimarePDF

Start it from your applications menu, or with `limarepdf` in a terminal. If the system warns about it, see above
for your system, or [Troubleshooting](troubleshooting.md#the-system-warns-about-the-app-the-first-time). Then go
on to [First steps](first-steps.md).

## Checking a download

To be sure a download really comes from this project and wasn't changed on the way, check it. Every release file
has a GPG signature beside it (`.asc`), and `SHA256SUMS`, which lists every file's checksum, is signed too. The
signing key is [`limarepdf-signing-key.asc`](limarepdf-signing-key.asc), also on keys.openpgp.org, with the
fingerprint `0CF9 7E88 BAFE E9B6 38D2  14CB EB40 041A 7A8D C801`. To check a file:

```
gpg --import limarepdf-signing-key.asc
gpg --verify SHA256SUMS.asc SHA256SUMS
sha256sum --check --ignore-missing SHA256SUMS
```

Each file also has a build attestation recording that it was built by this project's GitHub Actions from a given
commit. With the GitHub CLI: `gh attestation verify <file> --repo smaug42/limarepdf`.

Between releases, each run of the project's continuous integration on GitHub Actions builds the same archives; they
are under the run's artifacts (this needs a GitHub sign-in).

## Build from source

You need Rust 1.95 or newer.

```
cargo build --release --locked
# the binaries: target/release/limarepdf and target/release/limarepdf-cli
cargo test --workspace
```

`--locked` builds with exactly the library versions in `Cargo.lock`, the ones the releases are built and checked
with.

More for developers is in the [developer docs](developer/README.md).

---

[← User guide](README.md) · [First steps →](first-steps.md)
