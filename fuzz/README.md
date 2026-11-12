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
| `verify` | Any bytes opened as a PDF and its signatures checked as the app checks them, then against a trust list of the test certificates |
| `signature` | Any bytes as a signature's `/Contents`, in a one-page PDF signed over the rest; the first byte picks the `/SubFilter` (CMS, a timestamp token, or PKCS#1 beside a test certificate) |
| `trustfiles` | Any text read as a trust list (`trust.txt`), saved and read again: the same certificates trusted, in the same roles |
| `pkcs12` | Any bytes read as a digital ID file (PKCS#12), with the test IDs' passwords; the first byte picks one |
| `ipp` | Any bytes read as the printing service's answer (IPP), and its paper sizes and margins as printing reads them |

`pkcs12` and `ipp` reach core's readers through `fuzz_seam` (core's `lib.rs`), built only with `--cfg fuzzing`, as
`cargo fuzz` builds; never in the app. Under it a digital ID's key derivation stops at 10,000 iterations (the test
IDs take 6,144 at most), so each run stays short; the integrity check is never skipped, so the fuzzer reaches what
lies past it through the test IDs as they are.

## Running

Once, install a nightly toolchain and cargo-fuzz:

```
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
```

From the repository, seed the corpus from the repository's own test files (made-up documents, the signature
samples, test certificates and digital IDs, and a few content streams and printing-service answers; `seed.sh` needs
Python 3), then run a target for a few minutes under a memory limit:

```
sh fuzz/seed.sh
cargo +nightly fuzz run open -- -max_total_time=300 -rss_limit_mb=2048 -timeout=20 -max_len=262144
```

`cargo +nightly fuzz list` lists the targets. A failure is saved in `fuzz/artifacts/<target>/`; run it again with
`cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>`, and shrink it with `cargo +nightly fuzz tmin`.
The corpus and the artifacts are git-ignored: a failure found becomes a test in the crate it concerns, written to
fail first, as any bug's does, and the input itself is never committed unless it is made up. Problems in hayro or
lopdf are recorded in the backlog (PB items), as other upstream problems are.

CI's `fuzz` workflow (`.github/workflows/fuzz.yml`) runs every target for five minutes with these limits, weekly and
by hand from the Actions tab. It is never a required check: a failure is looked at, its input is kept as the run's
artifact, and it becomes a test as above. Recorded runs are in [testing.md](../docs/developer/testing.md#automated-tests).
