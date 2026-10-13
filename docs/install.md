# Install and build

## Download

There are no packaged releases yet. Each run of the project's continuous integration on GitHub Actions builds
archives for Linux, Windows and macOS (Intel and Apple Silicon); download them from the run's artifacts (this needs a
GitHub sign-in).

Each archive holds the desktop app (`mpe-gui`), the command-line tool (`mpe`), the README and licence files, and the
licences of the bundled OCR model and font.

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
