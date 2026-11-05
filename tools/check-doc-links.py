#!/usr/bin/env python3
"""Link check for the repository's Markdown (cycle plan S1-00): every relative link and image
must name a file that exists, and a `#fragment` into a Markdown file must name one of its
headings (GitHub's anchors). Web links aren't fetched. The only link checker the plan names.

Checks every tracked `*.md` file but this tool's own test fixtures.

    python3 tools/check-doc-links.py [ROOT]     check the repository at ROOT (default: here)
    python3 tools/check-doc-links.py --self-test
"""
import pathlib
import re
import subprocess
import sys

LINK = re.compile(r"\]\(\s*<([^>]+)>[^)]*\)|\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)|<img [^>]*src=\"([^\"]+)\"")
FIXTURES = "tools/tests/"


def slug(heading):
    """GitHub's anchor for a heading."""
    h = re.sub(r"`", "", heading.strip().lower())
    h = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", h)
    h = re.sub(r"[^\w\- ]", "", h)
    return h.replace(" ", "-")


def without_code(text):
    """`text` without fenced code blocks and inline code spans."""
    text = re.sub(r"^(```|~~~).*?^\1[^\n]*$", "", text, flags=re.S | re.M)
    return re.sub(r"`[^`\n]*`", "", text)


def anchors(path):
    out, seen, fence = set(), {}, None
    for line in path.read_text(encoding="utf-8").splitlines():
        m = re.match(r"(```|~~~)", line)
        if m:
            fence = None if fence == m.group(1) else (fence or m.group(1))
            continue
        if fence:
            continue
        m = re.match(r"#{1,6} (.*?)\s*#*\s*$", line)
        if m:
            s = slug(m.group(1))
            n = seen.get(s, 0)
            seen[s] = n + 1
            out.add(s if n == 0 else f"{s}-{n}")
    out.update(re.findall(r"<a\s+(?:name|id)=\"([^\"]+)\"", path.read_text(encoding="utf-8")))
    return out


def problems(path, label):
    found = []
    for m in LINK.finditer(without_code(path.read_text(encoding="utf-8"))):
        link = m.group(1) or m.group(2) or m.group(3)
        if re.match(r"[a-zA-Z][a-zA-Z+.-]*:", link):
            continue
        target_part, _, frag = link.partition("#")
        target = (path.parent / target_part).resolve() if target_part else path
        if not target.exists():
            found.append(f"{label}: missing file {link}")
        elif frag and target.suffix == ".md" and frag not in anchors(target):
            found.append(f"{label}: missing anchor {link}")
    return found


def check(root):
    tracked = subprocess.run(
        ["git", "ls-files", "-z", "*.md"], cwd=root, capture_output=True, check=True
    ).stdout.decode().split("\0")
    found = []
    for p in sorted(p for p in tracked if p and not p.startswith(FIXTURES)):
        found += problems(root / p, p)
    return found


def self_test():
    here = pathlib.Path(__file__).resolve().parent / "tests" / "links"
    expected = {
        "good.md": [],
        "missing_file.md": ["missing_file.md: missing file gone.md"],
        "missing_anchor.md": ["missing_anchor.md: missing anchor other.md#no-such-heading"],
    }
    failures = []
    for name, want in expected.items():
        got = problems(here / name, name)
        if got != want:
            failures.append(f"{name}: expected {want}, got {got}")
    for f in failures:
        print(f"self-test: {f}")
    print(f"self-test: {len(expected)} fixtures, {len(failures)} failed")
    return not failures


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(0 if self_test() else 1)
    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    found = check(root)
    for f in found:
        print(f)
    print(f"{len(found)} broken")
    sys.exit(1 if found else 0)
