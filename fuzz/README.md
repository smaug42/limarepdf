# Fuzzing

Fuzz targets for LimarePDF's readers (REL-12), run with [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz)
and libFuzzer. This folder is a workspace of its own: nothing here is built by the main workspace, CI or a release.
libFuzzer is C++, used only here, for development.

| Target | What it fuzzes |
| --- | --- |
| `open` | Any bytes opened as a PDF (`Pdf::from_bytes`, repair included), and a document that opens saved again |
| `content` | A page's content stream, read for its text, editable objects and paragraphs |
| `draw` | The same, drawn as the app draws a page, on a 200-point page so the picture stays small |
| `readers` | Any bytes opened as a PDF, then its annotations, form fields, bookmarks and first pages' text read |
| `images` | Any bytes imported as a picture page (PNG, JPEG, TIFF) |

Signature checking comes later (the build plan's REL-12).

## Running

Once, install a nightly toolchain and cargo-fuzz:

```
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
```

From the repository, seed the corpus from the repository's own test files, then run a target for a few minutes
under a memory limit:

```
sh fuzz/seed.sh
cargo +nightly fuzz run open -- -max_total_time=300 -rss_limit_mb=2048 -timeout=20 -max_len=262144
```

`cargo +nightly fuzz list` lists the targets. A failure is saved in `fuzz/artifacts/<target>/`; run it again with
`cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>`, and shrink it with `cargo +nightly fuzz tmin`.
The corpus and the artifacts are git-ignored: a failure found becomes a test in the crate it concerns, written to
fail first, as any bug's does, and the input itself is never committed unless it is made up. Problems in hayro or
lopdf are recorded in the backlog (PB items), as other upstream problems are.

Fuzzing runs here only for now, minutes per target; a scheduled CI job may follow.
