# Install and build

## Download

Download an archive for your platform from the project's Releases page on GitHub: Linux (`.tar.gz`), Windows
(`.zip`), and macOS for Intel and for Apple Silicon (`.tar.gz`). `SHA256SUMS` lists their checksums
([checking a download](#checking-a-download)).

Each archive holds the desktop app (`mpe-gui`), the command-line tool (`mpe`), these docs, the changelog, and the
licences, including those of the bundled OCR model and font. The Linux archive also has a desktop entry and icon
([below](#menu-entry-and-icon-on-linux)). Unpack it anywhere; there is no installer.

The binaries are not code-signed, so the system warns the first time you run them. On macOS, run
`xattr -d com.apple.quarantine mpe mpe-gui` in the unpacked folder, or try to run it once, then choose Open Anyway
under System Settings > Privacy & Security. On Windows, choose More info, then Run anyway.

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

### Menu entry and icon on Linux

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
