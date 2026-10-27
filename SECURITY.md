# Security policy

## Supported versions

Security fixes go into the next release. Only the latest release is supported: update to it before reporting, and
check whether the problem is still there.

## Reporting a vulnerability

Please report security problems privately, not as a public issue: on GitHub, open the repository's **Security** tab
and choose **Report a vulnerability**
([direct link](https://github.com/smaug42/limarepdf/security/advisories/new)). Only you and the maintainer can see
the report.

It helps to include:

- what an attacker could do, and what they need for it (a file you open, a document you sign, access to your
  computer);
- the steps to show it, with the version and system (☰ > Help > About, then Copy next to the version);
- a file that shows it, if one is needed. Make one up for the report; never send a private document.

## What happens next

LimarePDF is maintained by one person. You should get a first reply within a week. Once the problem is confirmed,
the fix goes into a new release, and a security advisory is published on GitHub with the release, crediting you if
you wish. Please keep the problem private until then.

## What counts

Report privately anything that lets a document or another person do what LimarePDF says can't be done, for example:

- a signature shown as valid on a document that was changed in a way it doesn't report;
- redaction or Remove hidden information leaving behind what it says it removed;
- password protection weaker than [Password protection](docs/security.md) describes, or a digital ID's key or
  password leaking;
- a document that makes LimarePDF run a program, open a link or website, or read or write a file without asking;
- release files that don't match their signatures or build attestations.

A crash, a hang, or a document that is drawn wrongly is an ordinary bug: report it in the
[issues](https://github.com/smaug42/limarepdf/issues), as [Troubleshooting](docs/troubleshooting.md#reporting-a-problem)
describes, unless it leads to one of the problems above.

Already known, so not needed as reports: the RSA timing advisory RUSTSEC-2023-0071, explained in
[Digital signatures](docs/signatures.md#security-notes), and the macOS and Windows builds not being code-signed
([Install](docs/install.md)). [Features and status](docs/developer/feature-status.md#what-doesnt-work-yet) lists what
doesn't work yet.

## How releases are protected

Every release waits for a check of all of LimarePDF's libraries against the RustSec database of known
vulnerabilities, and is built from exactly the versions in `Cargo.lock`. Each release file is GPG-signed and has a
build attestation; [Checking a download](docs/install.md#checking-a-download) shows how to check them.
