# LimarePDF

LimarePDF is a PDF viewer and editor for Linux, Windows and macOS, written entirely in Rust with no C or C++ dependencies.
It organises, edits, annotates, fills in, signs, stamps, redacts, secures, scans and prints PDFs, from a desktop app or the command
line.
This is a from-scratch implementation (with AI assistance): it contains no other product's code, strings, stamps, fonts or other assets.

It runs on Windows 10 and 11 (x64), macOS 11 or later (Apple Silicon and Intel), and 64-bit Linux with glibc 2.39 or
later. Support for each is provisional until the release checks by hand have passed there ([requirements](docs/install.md#requirements)).

![The LimarePDF window: the commands under the document's tab, page thumbnails on the left, a report with a highlight, a note, a pen tick and an arrow with a text box in the middle, and the inspector on the right](docs/images/limarepdf.png)

## Documentation

**[The user guide](docs/README.md)** takes you from installing LimarePDF to using every part of it. New to
LimarePDF? Start here:

1. [Install LimarePDF](docs/install.md)
2. [First steps](docs/first-steps.md)
3. [Features](docs/features.md): everything LimarePDF can do

Then go to the [everyday tasks](docs/README.md#everyday-tasks), the [desktop app](docs/gui.md) and
[command-line](docs/cli.md) references, or [Troubleshooting](docs/troubleshooting.md) when something goes wrong.

**For developers** ([all developer docs](docs/developer/README.md))

- [Features and status](docs/developer/feature-status.md): what works, its limits, and what doesn't yet
- [How it is built](docs/developer/architecture.md)
- [Testing and platforms](docs/developer/testing.md)
- [Performance](docs/developer/performance.md)
- [Releasing](docs/developer/releasing.md)
- [Ideas pool](docs/developer/ideas.md)

**[Changelog](CHANGELOG.md)** · **[Licence](docs/licence.md)** · **[Reporting a security problem](SECURITY.md)**
