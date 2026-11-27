#!/usr/bin/env python3
"""Test-program check: a crate that builds its integration tests as one program turns Cargo's
finding of them off (`autotests = false`), so a file in its `tests/` that no program takes in is
never built and its tests never run, silently.

Fails, for each `crates/*/Cargo.toml` with `autotests = false`, when:
- a `tests/NAME.rs`, or a folder's `tests/NAME/mod.rs`, is neither a `[[test]]`'s path nor a module
  (`mod NAME;`) of one: it is added to `tests/all.rs`, or given a `[[test]]` of its own;
- a `[[test]]`'s path isn't a file in `tests/`.

    python3 tools/check-test-programs.py [ROOT]     check the crates of the repository at ROOT
                                                    (default: here)
    python3 tools/check-test-programs.py --self-test
"""
import pathlib
import re
import sys
import tomllib

# A module declared in a program's root file: `mod x;`, `pub mod x;`, after any attributes. A
# comment's line starts with `//`, so it never matches.
MOD = re.compile(r"^\s*(?:pub(?:\([\w ]+\))?\s+)?mod\s+(\w+)\s*;", re.M)
TOP = re.compile(r"tests/[^/]+\.rs|tests/[^/]+/mod\.rs")


def findings(crates):
    """What's wrong, given each crate's folder → (its parsed Cargo.toml, its `tests/` files: path
    from the crate's folder → text)."""
    found = []
    for crate, (manifest, files) in sorted(crates.items()):
        if manifest.get("package", {}).get("autotests", True) is not False:
            continue
        built = set()
        for test in manifest.get("test", []):
            path = test.get("path", f"tests/{test.get('name')}.rs")
            if path not in files:
                found.append(f"{crate}/Cargo.toml: the test program `{path}` isn't there")
                continue
            built.add(path)
            for name in MOD.findall(files[path]):
                built |= {f"tests/{name}.rs", f"tests/{name}/mod.rs"}
        for path in sorted(files):
            if TOP.fullmatch(path) and path not in built:
                found.append(
                    f"{crate}/{path}: in no test program: make it a module of tests/all.rs, or a "
                    "[[test]] of its own in Cargo.toml"
                )
    return found


def crates_at(root):
    """Each crate under `root/crates`: its folder → (parsed Cargo.toml, its `tests/` files)."""
    crates = {}
    for manifest in sorted(root.glob("crates/*/Cargo.toml")):
        folder = manifest.parent
        with open(manifest, "rb") as f:
            parsed = tomllib.load(f)
        paths = [*folder.glob("tests/*.rs"), *folder.glob("tests/*/mod.rs")]
        files = {
            p.relative_to(folder).as_posix(): p.read_text(encoding="utf-8", errors="replace")
            for p in paths
        }
        crates[folder.relative_to(root).as_posix()] = (parsed, files)
    return crates


def self_test():
    off = {"package": {"autotests": False}}
    root = "//! The tests: mod in_a_comment;\nmod common;\n\n#[cfg(unix)]\nmod one;\npub mod two;\n"
    files = {
        "tests/all.rs": root,
        "tests/one.rs": "",
        "tests/two.rs": "mod nested;",
        "tests/alone.rs": "",
        "tests/common/mod.rs": "pub mod tool;",
        "tests/common/tool.rs": "",
        "tests/data/notes.rs": "",
        "tests/left_out.rs": "",
        "tests/kit/mod.rs": "",
        "tests/in_a_comment.rs": "",
    }
    programs = [{"name": "all", "path": "tests/all.rs"}, {"name": "alone"}, {"name": "gone"}]
    crates = {
        "crates/a": ({**off, "test": programs}, files),
        "crates/b": ({"package": {}}, {"tests/found_by_cargo.rs": ""}),
        "crates/c": ({**off}, {"tests/no_program.rs": ""}),
    }
    want = [
        "crates/a/Cargo.toml: the test program `tests/gone.rs` isn't there",
        "crates/a/tests/in_a_comment.rs: in no test program",
        "crates/a/tests/kit/mod.rs: in no test program",
        "crates/a/tests/left_out.rs: in no test program",
        "crates/c/tests/no_program.rs: in no test program",
    ]
    got = findings(crates)
    failures = [f"not found: {w}" for w in want if not any(g.startswith(w) for g in got)]
    failures += [f"unexpected: {g}" for g in got if not any(g.startswith(w) for w in want)]
    left_out = {"tests/left_out.rs", "tests/kit/mod.rs", "tests/in_a_comment.rs"}
    complete = {k: v for k, v in files.items() if k not in left_out}
    ok = {"crates/a": ({**off, "test": programs[:2]}, complete), "crates/b": crates["crates/b"]}
    failures += [f"a complete crate: {g}" for g in findings(ok)]
    for f in failures:
        print(f"self-test: {f}")
    print(f"self-test: {len(want)} findings and complete crates, {len(failures)} failed")
    return not failures


def main(args):
    if args == ["--self-test"]:
        return 0 if self_test() else 1
    root = pathlib.Path(args[0] if args else ".").resolve()
    found = findings(crates_at(root))
    for f in found:
        print(f)
    print(f"{len(found)} found")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
