# Developer docs

How LimarePDF is built, tested, measured and released, for people working on it. How to use LimarePDF is in the
user docs, one folder up ([the list](../../README.md#documentation)).

- [Features and status](feature-status.md): every feature's status in detail, its limits, what doesn't work yet
  and what is planned.
- [How it is built](architecture.md): the crates and the libraries used, and how each part works, with the code
  it is in.
- [Testing and platforms](testing.md): the platforms, the automated tests, how each feature was checked, and the
  acceptance checks by hand with their results.
- [Performance](performance.md): measured times and memory, and how the desktop app stays responsive.
- [Releasing](releasing.md): the release steps, the advisory check a release waits for, what CI builds, the package
  repositories and the signing key. How security problems are reported is in the [security policy](../../SECURITY.md).
- [Ideas pool](ideas.md): features other PDF tools have, to pick from as LimarePDF grows.

Building from source is in [Install LimarePDF](../install.md#build-from-source).
