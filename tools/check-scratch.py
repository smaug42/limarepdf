#!/usr/bin/env python3
"""Accidental-capture guard (cycle plan S1-00, O-02): fails when tracked code pulls in a file
from a build folder, or when an audit's scratch file is tracked.

Flags, in every tracked `*.rs` file (comments ignored):
- an `include!`, `include_str!` or `include_bytes!`, called with `()`, `[]` or `{}`, whose argument,
  over any number of lines, names a `target` folder (as
  `include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/x.rs"))` did in d157ec3);
- a `#[path = "..."]` (or a raw string, `r"..."`, `r#"..."#`) naming a `target` folder;
and every tracked file whose name starts with `audit_`.

Includes from `env!("OUT_DIR")` don't name a `target` folder and pass. This catches mistakes,
not deliberate hiding: it is not a security boundary.

    python3 tools/check-scratch.py [ROOT]     check the repository at ROOT (default: here)
    python3 tools/check-scratch.py --self-test
"""
import pathlib
import re
import subprocess
import sys

INCLUDE = re.compile(r"\binclude(?:_str|_bytes)?\s*!\s*[(\[{]")
PATH_ATTR = re.compile(
    r"#\s*!?\s*\[\s*path\s*=\s*"
    r"(?:\"(?P<plain>(?:[^\"\\]|\\.)*)\"|r(?P<hashes>#*)\"(?P<raw>.*?)\"(?P=hashes))",
    re.S,
)
TARGET = re.compile(r"(?:^|[/\\\"])target(?:[/\\]|\"|$)")


def code_only(text, blank_strings=False):
    """`text` with comments blanked (newlines kept, so line numbers and offsets hold); string
    literals kept, or with `blank_strings` blanked too, but for their quotes."""
    out, i, n = [], 0, len(text)
    blank = (lambda s: s[0] + re.sub(r"[^\n]", " ", s[1:-1]) + s[-1]) if blank_strings else (lambda s: s)
    while i < n:
        c, nxt = text[i], text[i + 1 : i + 2]
        if c == "/" and nxt == "/":
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif c == "/" and nxt == "*":
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            out.append(re.sub(r"[^\n]", " ", text[i:j]))
            i = j
        elif c == "r" and re.match(r"r#*\"", text[i:]) and not re.match(r"\w", text[i - 1 : i] or " "):
            hashes = re.match(r"r(#*)\"", text[i:]).group(1)
            end = text.find('"' + hashes, i + len(hashes) + 2)
            j = n if end < 0 else end + 1 + len(hashes)
            out.append(blank(text[i:j]))
            i = j
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            out.append(blank(text[i : j + 1]))
            i = j + 1
        elif c == "'":
            # A character literal ('x', '\n', '"') or a lifetime ('a).
            m = re.match(r"'(?:\\.[^']*|[^\\'])'", text[i:])
            j = i + (m.end() if m else 1)
            out.append(text[i:j])
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


def call_args(code, start):
    """The text between the parenthesis opening at `start` and its match (strings skipped)."""
    depth, j, n = 0, start, len(code)
    while j < n:
        c = code[j]
        if c == '"':
            j += 1
            while j < n and code[j] != '"':
                j += 2 if code[j] == "\\" else 1
        elif c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
            if depth == 0:
                return code[start + 1 : j]
        j += 1
    return code[start + 1 :]


def findings_in(name, text):
    # Macros and attributes are found where neither comments nor strings are (`bare`); their
    # arguments are read from the same offsets with the strings kept (`code`).
    code, bare = code_only(text), code_only(text, blank_strings=True)
    found = []
    for m in INCLUDE.finditer(bare):
        if TARGET.search(call_args(code, m.end() - 1)):
            line = code.count("\n", 0, m.start()) + 1
            found.append(f"{name}:{line}: `{m.group(0).rstrip('([{').strip()}` reads from a target folder")
    for m in re.finditer(r"#\s*!?\s*\[\s*path\s*=", bare):
        attr = PATH_ATTR.match(code, m.start())
        path = attr and (attr.group("plain") if attr.group("plain") is not None else attr.group("raw"))
        if path and TARGET.search(path):
            line = code.count("\n", 0, m.start()) + 1
            found.append(f"{name}:{line}: `#[path]` points into a target folder")
    return found


def name_findings(paths):
    return [f"{p}: an audit's scratch file is tracked" for p in paths if pathlib.PurePath(p).name.startswith("audit_")]


def check(root):
    tracked = subprocess.run(
        ["git", "ls-files", "-z"], cwd=root, capture_output=True, check=True
    ).stdout.decode().split("\0")
    tracked = [p for p in tracked if p]
    found = name_findings(tracked)
    for p in tracked:
        if p.endswith(".rs"):
            found += findings_in(p, (root / p).read_text(encoding="utf-8", errors="replace"))
    return found


def self_test():
    here = pathlib.Path(__file__).resolve().parent / "tests" / "scratch"
    failures = []
    fixtures = sorted(here.glob("*.rs.fixture"))
    for f in fixtures:
        flagged = bool(findings_in(f.name, f.read_text(encoding="utf-8")))
        if f.name.startswith("flag_") and not flagged:
            failures.append(f"{f.name}: not flagged")
        if f.name.startswith("pass_") and flagged:
            failures.append(f"{f.name}: flagged")
    names = ["crates/limarepdf-core/tests/audit_x.rs", "crates/limarepdf-core/src/audit.rs", "docs/auditing.md"]
    if name_findings(names) != [f"{names[0]}: an audit's scratch file is tracked"]:
        failures.append(f"tracked names: got {name_findings(names)}")
    for f in failures:
        print(f"self-test: {f}")
    print(f"self-test: {len(fixtures)} fixtures and the name rule, {len(failures)} failed")
    return not failures


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(0 if self_test() else 1)
    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    found = check(root)
    for f in found:
        print(f)
    print(f"{len(found)} found")
    sys.exit(1 if found else 0)
