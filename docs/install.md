# Install and build

## Download

On Linux, the [packages](#linux-packages) are the easiest way, on macOS the [app](#macos-app), and on Windows the
[installer](#windows-installer). Otherwise, download an archive for your platform from
the project's Releases page on GitHub: Linux (`.tar.gz`), Windows
(`.zip`), and macOS for Intel and for Apple Silicon (`.tar.gz`). `SHA256SUMS` lists their checksums
([checking a download](#checking-a-download)).

Each archive holds the desktop app (`mpe-gui`), the command-line tool (`mpe`), these docs, the changelog, and the
licences, including those of the bundled OCR model and font. The Linux archive also has a desktop entry and icon
([below](#menu-entry-and-icon-on-linux)). Unpack it anywhere; there is no installer.

The binaries are not code-signed, so the system warns the first time you run them. On macOS, run
`xattr -d com.apple.quarantine mpe mpe-gui` in the unpacked folder, or try to run it once, then choose Open Anyway
under System Settings > Privacy & Security. On Windows, choose More info, then Run anyway.

### macOS app

`MPE-<version>.dmg` holds `MPE.app`, one app for both Apple Silicon and Intel Macs (macOS 11 or later). Open the dmg
and drag MPE to Applications. MPE is not signed with an Apple Developer ID, so the first launch says Apple cannot
check it: choose Done, then open System Settings > Privacy & Security, choose Open Anyway next to MPE, and confirm.
MPE then appears under Open With for PDF files.

The command-line tool is inside the app; to use it from a terminal:

```
sudo ln -s /Applications/MPE.app/Contents/MacOS/mpe /usr/local/bin/mpe
```

### Windows installer

`mpe-<version>-x86_64.msi` opens an installer that lets you choose the folder (`Program Files\MPE` by default). It installs MPE for all
users, adds it to the Start menu and to Open
with for PDF files, and puts `mpe` on the `PATH` (open a new terminal to use it). To make MPE the default PDF app,
choose it in Settings > Apps > Default apps. Installing a newer version replaces the old one; uninstall from
Settings > Apps. The installer is not code-signed, so SmartScreen warns: choose More info, then Run anyway.

### Checking a download

Every release file has a GPG signature beside it (`.asc`), and `SHA256SUMS` is signed too. The signing key is
[`mpe-signing-key.asc`](mpe-signing-key.asc), also on keys.openpgp.org, with the fingerprint
`0CF9 7E88 BAFE E9B6 38D2  14CB EB40 041A 7A8D C801`. To check a file:

```
gpg --import mpe-signing-key.asc
gpg --verify SHA256SUMS.asc SHA256SUMS
sha256sum --check --ignore-missing SHA256SUMS
```

Each file also has a build attestation recording that it was built by this project's GitHub Actions from a given
commit. With the GitHub CLI: `gh attestation verify <file> --repo smaug42/mpe`.

Between releases, each run of the project's continuous integration on GitHub Actions builds the same archives; they
are under the run's artifacts (this needs a GitHub sign-in).

### Linux packages

The easiest way is MPE's package repository, which brings updates with the rest of your system. It is signed with
the key [below](#checking-a-download).

Debian and Ubuntu:

```
sudo install -d /etc/apt/keyrings
sudo curl -fsSLo /etc/apt/keyrings/mpe.asc https://smaug42.github.io/mpe/mpe-signing-key.asc
sudo curl -fsSLo /etc/apt/sources.list.d/mpe.sources https://smaug42.github.io/mpe/apt/mpe.sources
sudo apt update && sudo apt install mpe
```

Fedora: `sudo dnf config-manager addrepo --from-repofile=https://smaug42.github.io/mpe/rpm/mpe.repo`, then
`sudo dnf install mpe`. openSUSE: `sudo zypper addrepo https://smaug42.github.io/mpe/rpm/mpe.repo`, then
`sudo zypper install mpe`.

Each release also has a `.deb` (Debian 13, Ubuntu 24.04 and later) and an `.rpm` (Fedora, openSUSE), built on Ubuntu
24.04, so older systems need the archive's binaries or a build from source. They install `mpe-gui` and `mpe` in
`/usr/bin`, with the menu entry, icon and docs, and pull in the libraries the app needs.

```
sudo apt install ./mpe_<version>-1_amd64.deb
sudo dnf install ./mpe-<version>-1.x86_64.rpm
```

The `.rpm` is signed with the key [below](#checking-a-download); to have rpm check it, import the key first with
`sudo rpm --import mpe-signing-key.asc`.

### Menu entry and icon on Linux

The packages above set this up. For the archive:

The window shows MPE's icon on X11, Windows and macOS without any setup. Wayland desktops (GNOME, KDE Plasma and
others) take a window's icon from its desktop entry instead, so without one the window has a generic icon. The Linux
archive includes `mpe.desktop` and `mpe.png`; to install them for your user, run in the unpacked folder:

```
install -Dm644 mpe.desktop ~/.local/share/applications/mpe.desktop
install -Dm644 mpe.png ~/.local/share/icons/hicolor/256x256/apps/mpe.png
```

The entry runs `mpe-gui`, so put `mpe-gui` on your `PATH` (for example in `~/.local/bin`), or change the `Exec=` line
to its full path. MPE also appears in your applications menu and in the list of apps that can open PDF files. When
building from source, the files are `crates/mpe-gui/assets/mpe.desktop` and `docs/images/icon.png`.

## Build from source

You need Rust 1.95 or newer.

```
cargo build --release
# the binaries: target/release/mpe-gui and target/release/mpe
cargo test --workspace
```

## Requirements

- **Linux:** the desktop app needs a working OpenGL driver and an X11 or Wayland session. Nothing else is needed.
- **Windows, macOS:** nothing beyond the binaries.
- **Printing:** CUPS on Linux and macOS (macOS includes it).
