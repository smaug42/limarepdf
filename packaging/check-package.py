"""Checks an unpacked archive, package or installed folder: the programs, docs and licences are all there, and
every link in its Markdown files works. A relative link must reach a file in the folder (and a heading in it, for
#anchors); a link to this version's tag on GitHub (packaging/package-docs.sh writes them) must reach a file in this
repository. Other web links aren't followed.

THIRD-PARTY-LICENSES.html (packaging/package-notices.sh) must have a notice for each crate built into the programs
for the folder's targets, by name and version, and none for a crate that isn't. The crates built are the programs'
normal dependencies as `cargo tree` lists them (cargo metadata's graph also has optional dependencies whose features
are off), less the workspace's own. The notices for fonts and code that crates bundle must be there too.

    python3 packaging/check-package.py [--target TRIPLE]... <folder>
    python3 packaging/check-package.py --self-test

The target is this computer's unless given; the universal Mac app gives both of its targets. Run from anywhere; it
reads the version and the repository from where it is kept. Exits 1 with each problem listed.
"""

import argparse
import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
SITE = "https://github.com/smaug42/limarepdf"
NOTICES = "THIRD-PARTY-LICENSES.html"
NEEDED = [
    "README.md",
    "CHANGELOG.md",
    "SECURITY.md",
    "LICENSE-MIT",
    "LICENSE-APACHE",
    "OCR-MODEL.md",
    "FONT-LICENSE-Inter.txt",
    "FONT-LICENSE-Phosphor.txt",
    "FONT-LICENSE-Liberation.txt",
    "FONT-LICENSE-Caveat.txt",
    "FONT-LICENSE-CedarvilleCursive.txt",
    "FONT-LICENSE-HomemadeApple.txt",
    "FONT-LICENSE-Zeyada.txt",
    "DICTIONARY-LICENSE-English.txt",
    "DICTIONARY-LICENSE-French.txt",
    "LICENSE-MPL-2.0.txt",
    NOTICES,
]
# The programs whose dependencies are shipped.
SHIPPED = ["limarepdf-gui", "limarepdf-cli"]
# What a crate's bundled fonts or code need in the folder when the crate is built: (crate, file, text). cargo-about
# finds most of these only through about.toml's clarifications, which drop a notice with no more than a warning
# when the crate's file changes.
BUNDLED = [
    ("hayro-font", NOTICES, "PDFium Authors"),  # Foxit's standard fonts
    ("epaint_default_fonts", NOTICES, "Source Foundry"),  # Hack
    ("epaint_default_fonts", NOTICES, "John Slegers"),  # egui-icons, cut from emoji-icon-font
    ("epaint_default_fonts", NOTICES, "UBUNTU FONT LICENCE"),  # Ubuntu Light
    ("subsetter", NOTICES, "Just van Rossum"),  # fonttools' code, in its NOTICE
    ("sctk-adwaita", "FONT-LICENSE-Cantarell.txt", "Understanding Limited"),  # Cantarell, Wayland title bars
]
LINK = re.compile(r"\]\(([^)\s]+)\)")


def version():
    text = (REPO / "Cargo.toml").read_text(encoding="utf-8")
    return re.search(r'^version = "(.*)"', text, re.M).group(1)


def slug(heading):
    h = re.sub(r"`", "", heading.strip().lower())
    h = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", h)
    h = re.sub(r"[^\w\- ]", "", h)
    return h.replace(" ", "-")


def anchors(path):
    """GitHub's anchors for a Markdown file's headings (a repeated heading gets -1, -2, ...)."""
    out, seen, fence = set(), {}, False
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("```"):
            fence = not fence
        m = None if fence else re.match(r"#{1,6} (.*)", line)
        if m:
            s = slug(m.group(1))
            n = seen.get(s, 0)
            seen[s] = n + 1
            out.add(s if n == 0 else f"{s}-{n}")
    return out


def reaches(target, anchor):
    """Why a link's target or anchor doesn't exist, or None."""
    if not target.exists():
        return "no such file"
    if anchor and target.suffix == ".md" and anchor not in anchors(target):
        return f"no heading #{anchor}"
    return None


def host():
    out = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, check=True).stdout
    return re.search(r"^host: (\S+)", out, re.M).group(1)


def built(targets, root=REPO, programs=SHIPPED):
    """The crates, as (name, version), built into the programs for any of the targets: each line of `cargo tree`
    is a crate, its version and marks in brackets, one of them the path of a local crate; a path in the workspace
    is one of its own."""
    crates = set()
    for target in targets:
        command = ["cargo", "tree", "--locked", "-e", "normal", "--target", target, "-f", "{p}", "--prefix", "none"]
        for p in programs:
            command += ["-p", p]
        run = subprocess.run(command, cwd=root, capture_output=True, text=True)
        if run.returncode:
            sys.exit(f"{' '.join(command)} failed:\n{run.stderr}")
        for line in run.stdout.splitlines():
            m = re.match(r"(\S+) v(\S+)(.*)", line)
            if not m:
                continue
            marks = [pathlib.Path(s) for s in re.findall(r"\(([^)]*)\)", m.group(3))]
            if not any(p.is_absolute() and p.resolve().is_relative_to(root.resolve()) for p in marks):
                crates.add((m.group(1), m.group(2)))
    return crates


def notices(files, crates):
    """The problems with the notices, given the folder's notice files (name: text) and the crates built."""
    problems = []
    # Without the file, there is nothing to compare (NEEDED reports it missing).
    if NOTICES in files:
        listed = {tuple(c.split(" ", 1)) for c in re.findall(r'data-crate="([^"]+)"', files[NOTICES])}
        problems += [f"{NOTICES}: no notice for {n} {v}" for n, v in sorted(crates - listed)]
        problems += [f"{NOTICES}: a notice for {n} {v}, which isn't built" for n, v in sorted(listed - crates)]
    names = {n for n, _ in crates}
    for crate, file, text in BUNDLED:
        if crate not in names or file == NOTICES and file not in files:
            continue
        if file not in files:
            problems.append(f"missing: {file}, for {crate}'s bundled works")
        elif text not in files[file]:
            problems.append(f"{file}: no {text!r}, for {crate}'s bundled works")
    return problems


def check(folder, targets):
    problems = []
    names = {p.name for p in folder.rglob("*") if p.is_file()}
    for name in NEEDED + ["limarepdf", "limarepdf-cli"]:
        if name not in names and name + ".exe" not in names:
            problems.append(f"missing: {name}")
    online = f"{SITE}/(?:blob|raw)/v{re.escape(version())}/"
    for md in sorted(folder.rglob("*.md")):
        in_fence = False
        for number, line in enumerate(md.read_text(encoding="utf-8").splitlines(), 1):
            if line.startswith("```"):
                in_fence = not in_fence
            if in_fence:
                continue
            for link in LINK.findall(line):
                path, _, anchor = link.partition("#")
                if re.match(online, link):
                    target = REPO / re.sub(online, "", path)
                elif re.match(r"[a-z]+:", link):
                    continue
                elif not path:
                    target = md
                else:
                    target = md.parent / path
                why = reaches(target, anchor)
                if why:
                    problems.append(f"{md.relative_to(folder)}:{number}: {link}: {why}")
    wanted = {NOTICES} | {file for _, file, _ in BUNDLED}
    files = {p.name: p.read_text(encoding="utf-8") for p in folder.rglob("*") if p.name in wanted and p.is_file()}
    return problems + notices(files, built(targets))


def self_test():
    """The crates built, from a workspace with each kind of dependency (tools/tests/package), and the notices
    checked against them."""
    ws = REPO / "tools" / "tests" / "package" / "ws"
    every = {("common", "1.0.0"), ("common", "2.0.0"), ("mid", "0.1.0"), ("derive-it", "0.1.0"), ("via-inner", "0.1.0")}
    linux = every | {("linux-only", "0.1.0")}
    macs = every | {("arm-mac", "0.1.0"), ("intel-mac", "0.1.0")}

    def page(crates, text=""):
        return "".join(f'<li data-crate="{n} {v}">' for n, v in sorted(crates)) + text

    foxit = {("hayro-font", "0.5.0")}
    cantarell = {("sctk-adwaita", "0.10.1")}
    cases = [
        ("Linux's crates", lambda: built(["x86_64-unknown-linux-gnu"], ws, ["app"]), linux),
        ("both Macs' crates", lambda: built(["aarch64-apple-darwin", "x86_64-apple-darwin"], ws, ["app"]), macs),
        ("every notice", lambda: notices({NOTICES: page(linux)}, linux), []),
        (
            "a version's notice missing",
            lambda: notices({NOTICES: page(linux - {("common", "2.0.0")})}, linux),
            [f"{NOTICES}: no notice for common 2.0.0"],
        ),
        (
            "a notice for a crate not built",
            lambda: notices({NOTICES: page(linux | {("arm-mac", "0.1.0")})}, linux),
            [f"{NOTICES}: a notice for arm-mac 0.1.0, which isn't built"],
        ),
        ("a bundled font's notice", lambda: notices({NOTICES: page(foxit, "PDFium Authors")}, foxit), []),
        (
            "a bundled font's notice missing",
            lambda: notices({NOTICES: page(foxit)}, foxit),
            [f"{NOTICES}: no 'PDFium Authors', for hayro-font's bundled works"],
        ),
        (
            "a bundled font's file missing",
            lambda: notices({NOTICES: page(cantarell)}, cantarell),
            ["missing: FONT-LICENSE-Cantarell.txt, for sctk-adwaita's bundled works"],
        ),
        (
            "no notices at all",
            lambda: notices({}, foxit | cantarell),
            ["missing: FONT-LICENSE-Cantarell.txt, for sctk-adwaita's bundled works"],
        ),
    ]
    failures = []
    for name, run, want in cases:
        got = run()
        if got != want:
            failures.append(f"{name}: expected {sorted(want)}, got {sorted(got)}")
    for f in failures:
        print(f"self-test: {f}")
    print(f"self-test: {len(cases)} cases, {len(failures)} failed")
    return not failures


if __name__ == "__main__":
    args = argparse.ArgumentParser()
    args.add_argument("--self-test", action="store_true")
    args.add_argument("--target", action="append")
    args.add_argument("folder", nargs="?", type=pathlib.Path)
    a = args.parse_args()
    if a.self_test:
        sys.exit(0 if self_test() else 1)
    if a.folder is None:
        args.error("a folder to check is needed")
    problems = check(a.folder, a.target or [host()])
    for p in problems:
        print(p)
    print(f"{a.folder}: {len(problems)} problem(s)")
    sys.exit(1 if problems else 0)
