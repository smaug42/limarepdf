# Releasing

How a release is made, for maintainers.

## Steps

1. On a branch from `main`:
   - set the new version in `Cargo.toml` (`[workspace.package]`), and run a build so `Cargo.lock` follows;
   - update the status line in `docs/features.md` if it names the version;
   - add a section to `CHANGELOG.md`, headed `## [x.y.z] - date`.
2. Open a pull request. CI builds, tests and packages all four platforms; check the archives in the run's artifacts.
   Merge when it is green.
3. On `main`, after pulling the merge, make a signed tag and push it:

   ```
   git tag -s vX.Y.Z -m "mpe X.Y.Z"
   git push origin vX.Y.Z
   ```

4. CI runs again for the tag. Its `release` job checks that the tag matches `Cargo.toml`, takes the release notes
   from the matching `CHANGELOG.md` section, and creates a draft GitHub Release with the four archives and
   `SHA256SUMS`.
5. On GitHub, open the draft under Releases, choose Edit, check the notes and files, and publish it.

Do not create the release on GitHub by hand, or the tag from GitHub's release form: the `release` job then finds a
release already there and fails.

## What CI builds

`.github/workflows/ci.yml` runs on pull requests, pushes to `main`, and `v*` tags. Each platform job packages
`mpe-<version>-<platform>.tar.gz` (`.zip` on Windows), holding the binaries, the README, `CHANGELOG.md`, `docs/` and
the licences. The archives are made on the runner because Actions artifacts lose Unix file permissions; on the
Actions page each artifact therefore downloads as a zip with the archive inside.

## To do

- **Code signing.** The macOS and Windows binaries are not code-signed or notarised, so Gatekeeper and SmartScreen
  warn on first launch (`docs/install.md` tells users how to proceed). Signing needs an Apple Developer account for
  macOS and a code-signing certificate for Windows, and signing steps in CI.
- **Comparison link.** Release notes could end with a link comparing the tag with the previous one; today that is
  added by hand on the release page.
