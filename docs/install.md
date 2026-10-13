# Install and build

## Download

Download an archive for your platform from the project's Releases page on GitHub: Linux (`.tar.gz`), Windows
(`.zip`), and macOS for Intel and for Apple Silicon (`.tar.gz`). `SHA256SUMS` lists their checksums.

Each archive holds the desktop app (`mpe-gui`), the command-line tool (`mpe`), these docs, the changelog, and the
licences, including those of the bundled OCR model and font. Unpack it anywhere; there is no installer.

The binaries are not code-signed, so the system warns the first time you run them. On macOS, right-click the app and
choose Open, or run `xattr -d com.apple.quarantine mpe mpe-gui` in the unpacked folder. On Windows, choose
More info, then Run anyway.

Between releases, each run of the project's continuous integration on GitHub Actions builds the same archives; they
are under the run's artifacts (this needs a GitHub sign-in).

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
