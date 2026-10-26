"""Checks an unpacked archive, package or installed folder: the programs, docs and licences are all there, and
every link in its Markdown files works. A relative link must reach a file in the folder (and a heading in it, for
#anchors); a link to this version's tag on GitHub (packaging/package-docs.sh writes them) must reach a file in this
repository. Other web links aren't followed.

    python3 packaging/check-package.py <folder>

Run from anywhere; it reads the version and the repository from where it is kept. Exits 1 with each problem listed.
"""

import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
SITE = "https://github.com/smaug42/limarepdf"
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


def check(folder):
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
    return problems


if __name__ == "__main__":
    folder = pathlib.Path(sys.argv[1])
    problems = check(folder)
    for p in problems:
        print(p)
    print(f"{folder}: {len(problems)} problem(s)")
    sys.exit(1 if problems else 0)
