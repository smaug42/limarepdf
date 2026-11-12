#!/usr/bin/env python3
"""Ignored-test list check (cycle plan WP-11, V-10): every ignored test (`#[ignore]`) is in
`tools/ignored-tests.toml` once, with its group, and every test there is still ignored.

Fails, for the repository's tracked `*.rs` files (comments and strings ignored), when:
- an ignored test isn't listed, so a new one must be given a group;
- a test is listed twice, or is listed but no longer ignored (or gone);
- an entry lacks a field, names a group the list doesn't describe, gives a test target cargo can't
  select (`--lib`, `--bin NAME`, `--test NAME`), or is a cross-check (G6) without the programs it needs.

The list also drives tools/run-cross-checks.sh, which runs group G6 by these exact names:

    python3 tools/check-ignored-tests.py [ROOT]       check the repository at ROOT (default: here)
    python3 tools/check-ignored-tests.py --list G6    each test of a group: crate, target and name, tab-separated
    python3 tools/check-ignored-tests.py --needs G6   the programs a group's tests run, one a line
    python3 tools/check-ignored-tests.py --self-test
"""
import collections
import functools
import importlib.util
import pathlib
import re
import subprocess
import sys
import tomllib

LIST = "tools/ignored-tests.toml"
FIELDS = {"name", "crate", "target", "file", "group", "needs"}
REQUIRED = FIELDS - {"needs"}
TARGET = re.compile(r"--lib|--(?:bin|test) [\w-]+")
IGNORE = re.compile(r"#\s*\[\s*ignore\b")
FN = re.compile(r"\bfn\s+(\w+)")


@functools.cache
def scratch_check():
    spec = importlib.util.spec_from_file_location(
        "check_scratch", pathlib.Path(__file__).resolve().with_name("check-scratch.py")
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def code_only(text):
    """`text` with comments and string contents blanked: check-scratch.py's reader, shared."""
    return scratch_check().code_only(text, blank_strings=True)


def ignored_in(path, text, blank=code_only):
    """Each ignored test in one file: (path, function name, line)."""
    bare = blank(text)
    found = []
    for m in IGNORE.finditer(bare):
        f = FN.search(bare, m.end())
        if f:
            found.append((path, f.group(1), bare.count("\n", 0, m.start()) + 1))
    return found


def findings(sources, listed, blank=code_only):
    """What's wrong, given `sources` (path → text) and the parsed list."""
    found = []
    groups = listed.get("groups", {})
    tests = listed.get("test", [])
    for i, t in enumerate(tests):
        where = f"{LIST}: test {i + 1} ({t.get('name', 'no name')})"
        missing = REQUIRED - t.keys()
        if missing:
            found.append(f"{where}: no {', '.join(sorted(missing))}")
            continue
        if t.keys() - FIELDS:
            found.append(f"{where}: unknown {', '.join(sorted(t.keys() - FIELDS))}")
        if t["group"] not in groups:
            found.append(f"{where}: group {t['group']} isn't described under [groups]")
        if not TARGET.fullmatch(t["target"]):
            found.append(f"{where}: target `{t['target']}` isn't --lib, --bin NAME or --test NAME")
        elif t["target"].startswith("--test ") and t["file"] != f"crates/{t['crate']}/tests/{t['target'][7:]}.rs":
            found.append(f"{where}: {t['file']} isn't the file of `{t['target']}` in {t['crate']}")
        if not t["file"].startswith(f"crates/{t['crate']}/"):
            found.append(f"{where}: {t['file']} isn't in {t['crate']}")
        if t["group"] == "G6" and not t.get("needs"):
            found.append(f"{where}: a cross-check (G6) needs `needs`, the programs it runs")
    complete = {}
    for t in tests:
        if REQUIRED <= t.keys():
            complete.setdefault((t["crate"], t["target"], t["name"]), []).append(t)
    for (crate, target, name), same in sorted(complete.items()):
        if len(same) > 1:
            found.append(f"{LIST}: {crate} {target} {name} is listed {len(same)} times")
    # By file and function name, as the files are read: a test inside a module is listed by its
    # whole name (`tests::name`).
    there = collections.Counter((s[0]["file"], s[0]["name"].rsplit("::", 1)[-1]) for s in complete.values())
    ignored = [i for path, text in sorted(sources.items()) for i in ignored_in(path, text, blank)]
    counted = collections.Counter((path, name) for path, name, _ in ignored)
    for path, name, line in ignored:
        if counted[(path, name)] > there[(path, name)]:
            found.append(f"{path}:{line}: `{name}` is ignored but isn't in {LIST}: give it a group there")
    for path, name in sorted(there - counted):
        found.append(f"{LIST}: `{name}` in {path} is listed but isn't an ignored test there")
    return found


def tracked_sources(root):
    files = subprocess.run(["git", "ls-files", "-z"], cwd=root, capture_output=True, check=True)
    paths = [p for p in files.stdout.decode().split("\0") if p.endswith(".rs")]
    return {p: (root / p).read_text(encoding="utf-8", errors="replace") for p in paths}


def read_list(root):
    """The parsed list; with no list, an empty one, so every ignored test is reported."""
    if not (root / LIST).exists():
        return {}
    with open(root / LIST, "rb") as f:
        return tomllib.load(f)


def self_test():
    source = """
#[test]
#[ignore = "needs pdftotext"]
fn listed() {}
// #[ignore] in a comment, and one in a string, aren't tests
fn not_ignored() { let s = "#[ignore]"; }
#[ignore]
#[test]
fn unlisted() {}
mod a { #[test] #[ignore] fn twice_named() {} }
mod b { #[test] #[ignore] fn twice_named() {} }
"""
    sources = {"crates/c/tests/t.rs": source}
    entry = {"crate": "c", "target": "--test t", "file": "crates/c/tests/t.rs", "group": "G6"}
    listed = {
        "groups": {"G6": "cross-checks", "G2": "children"},
        "test": [
            {**entry, "name": "listed", "needs": ["pdftotext"]},
            {**entry, "name": "a::twice_named", "group": "G2"},
            {**entry, "name": "b::twice_named", "group": "G2"},
            {**entry, "name": "gone", "group": "G2"},
            {**entry, "name": "listed", "needs": ["pdftotext"]},
            {**entry, "name": "no_needs"},
            {**entry, "name": "odd", "group": "G9", "target": "--tests t"},
            {**entry, "name": "elsewhere", "group": "G2", "file": "crates/c/tests/u.rs"},
            {"name": "short"},
        ],
    }
    want = [
        "test 9 (short): no crate, file, group, target",
        "test 6 (no_needs): a cross-check (G6) needs `needs`",
        "test 7 (odd): group G9 isn't described",
        "test 7 (odd): target `--tests t` isn't",
        "test 8 (elsewhere): crates/c/tests/u.rs isn't the file of `--test t`",
        "c --test t listed is listed 2 times",
        "crates/c/tests/t.rs:7: `unlisted` is ignored but isn't in",
        "`gone` in crates/c/tests/t.rs is listed but isn't an ignored test there",
        "`no_needs` in crates/c/tests/t.rs is listed but isn't",
        "`odd` in crates/c/tests/t.rs is listed but isn't",
        "`elsewhere` in crates/c/tests/u.rs is listed but isn't",
    ]
    got = findings(sources, listed)
    failures = [f"not found: {w}" for w in want if not any(w in g for g in got)]
    failures += [f"unexpected: {g}" for g in got if not any(w in g for w in want)]
    ok = {"groups": listed["groups"], "test": listed["test"][:3] + [{**entry, "name": "unlisted", "group": "G2"}]}
    failures += [f"a complete list: {g}" for g in findings(sources, ok)]
    for f in failures:
        print(f"self-test: {f}")
    print(f"self-test: {len(want)} findings and a complete list, {len(failures)} failed")
    return not failures


def main(args):
    if args == ["--self-test"]:
        return 0 if self_test() else 1
    if len(args) == 2 and args[0] in ("--list", "--needs"):
        tests = [t for t in read_list(pathlib.Path(".")).get("test", []) if t.get("group") == args[1]]
        if args[0] == "--list":
            for t in tests:
                print(f"{t['crate']}\t{t['target']}\t{t['name']}")
        else:
            print("\n".join(sorted({n for t in tests for n in t.get("needs", [])})))
        return 0
    root = pathlib.Path(args[0] if args else ".").resolve()
    found = findings(tracked_sources(root), read_list(root))
    for f in found:
        print(f)
    print(f"{len(found)} found")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
