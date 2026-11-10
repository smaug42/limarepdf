#!/usr/bin/env python3
"""Release-signing guard (cycle plan S1-19, M-16): fails when a workflow can hand the release signing
secrets to a job that isn't bound to the protected `release-signing` environment, or that a pull
request can start.

A signing reference is `secrets.GPG_...` (or `secrets['GPG_...']`, in any case), `toJSON(secrets)` or
`secrets: inherit`, anywhere but in a YAML comment (a shell comment in a `run:` block counts: Actions
fills in expressions before the shell sees them). Flagged, in `.github/workflows/*.yml` and `*.yaml`:
- a reference outside the jobs (a workflow-wide `env:`, say), which every job sees;
- a reference in a job whose `environment` isn't `release-signing`;
- a reference in a job of a workflow a pull request can start (`pull_request`, `pull_request_target`,
  `workflow_run`), unless the job's `if:` holds only for a release: each of its `||` branches needs,
  among its `&&` terms, `github.event_name == 'release'`, `startsWith(github.ref, 'refs/tags/...')` or a
  manual run's input (`inputs.x`, `github.event.inputs.x`), none of which a pull request has.

This is an early warning, not the protection: a change to a workflow can change this check too. The
protection is the environment (a required reviewer; `v*` tags and `main` only) holding the only copy of
the secrets.

    python3 tools/check-workflows.py [ROOT]     check the workflows of the repository at ROOT (default: here)
    python3 tools/check-workflows.py --self-test
"""
import pathlib
import re
import sys

ENVIRONMENT = "release-signing"
PR_EVENTS = {"pull_request", "pull_request_target", "workflow_run"}
REFERENCE = re.compile(r"\bsecrets\s*(?:\.\s*GPG_|\[\s*['\"]GPG_)|\btoJSON\s*\(\s*secrets\s*\)", re.I)
INHERIT = re.compile(r"^\s*secrets\s*:\s*inherit\s*$")
KEY = re.compile(r"^\s*(?:-\s+)?(['\"]?)([\w-]+)\1\s*:(?:\s+(.*))?$")
BLOCK = re.compile(r":\s*[|>][-+0-9]*\s*$")
RELEASE_ONLY = [
    re.compile(r"github\.event_name\s*==\s*'release'", re.I),
    re.compile(r"startsWith\(\s*github\.ref\s*,\s*'refs/tags/[^']*'\s*\)", re.I),
    re.compile(r"(?:github\.event\.)?inputs\.[A-Za-z_][\w-]*(?:\s*==\s*true)?", re.I),
]


def uncomment(line):
    """`line` without its YAML comment: a `#` at its start or after a space, outside quotes."""
    quote, i, n = None, 0, len(line)
    while i < n:
        c = line[i]
        if quote:
            if quote == '"' and c == "\\":
                i += 1
            elif c == quote:
                quote = None
        elif c in "'\"" and (i == 0 or line[i - 1] in " \t[{,:"):
            quote = c
        elif c == "#" and (i == 0 or line[i - 1] in " \t"):
            return line[:i].rstrip()
        i += 1
    return line


def lines_of(text):
    """Each line as (number, indent, code, structural): `code` is what Actions reads (a YAML comment
    removed; a block scalar's lines whole), and `structural` is False for blank lines and the lines of
    a block scalar (a `run: |`, say)."""
    out, block = [], None
    for number, raw in enumerate(text.splitlines(), 1):
        indent = len(raw) - len(raw.lstrip(" "))
        if block is not None:
            if not raw.strip() or indent > block:
                out.append((number, indent, raw, False))
                continue
            block = None
        code = uncomment(raw)
        if not code.strip():
            out.append((number, indent, code, False))
            continue
        if BLOCK.search(code):
            # Its lines are those indented past the key (past `run` in `- run: |`).
            block = len(re.match(r"\s*(?:-\s+)*", code).group(0))
        out.append((number, indent, code, True))
    return out


def body(lines, i):
    """The index past the lines under line `i`: up to the next structural line indented no deeper."""
    j = i + 1
    while j < len(lines) and not (lines[j][3] and lines[j][1] <= lines[i][1]):
        j += 1
    return j


def children(lines, i, end):
    """The structural lines directly under line `i` (up to `end`), as indexes."""
    inner = [k for k in range(i + 1, end) if lines[k][3]]
    return [k for k in inner if lines[k][1] == lines[inner[0]][1]] if inner else []


def unquote(value):
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
        return value[1:-1]
    return value


def value_of(lines, i):
    """Line `i`'s value: the text after its key and every line under it, joined."""
    m = KEY.match(lines[i][2])
    inline = (m and m.group(3) or "").strip()
    parts = [] if re.fullmatch(r"[|>][-+0-9]*", inline) else [inline]
    parts += [lines[k][2].strip() for k in range(i + 1, body(lines, i)) if lines[k][2].strip()]
    return unquote(" ".join(p for p in parts if p))


def closing(e, start):
    """The index of the parenthesis closing the one at `start` (quotes skipped), or -1."""
    depth, quote = 0, False
    for j in range(start, len(e)):
        c = e[j]
        if c == "'":
            quote = not quote
        elif not quote and c == "(":
            depth += 1
        elif not quote and c == ")":
            depth -= 1
            if depth == 0:
                return j
    return -1


def unwrap(e):
    e = e.strip()
    m = re.fullmatch(r"\$\{\{(.*)\}\}", e, re.S)
    e = m.group(1).strip() if m else e
    while e.startswith("(") and closing(e, 0) == len(e) - 1:
        e = e[1:-1].strip()
    return e


def split_top(e, op):
    """`e` split at each `op` outside parentheses and quotes."""
    parts, depth, quote, last, j = [], 0, False, 0, 0
    while j < len(e):
        c = e[j]
        if c == "'":
            quote = not quote
        elif not quote and c == "(":
            depth += 1
        elif not quote and c == ")":
            depth -= 1
        elif not quote and depth == 0 and e.startswith(op, j):
            parts.append(e[last:j])
            last = j = j + len(op)
            continue
        j += 1
    return parts + [e[last:]]


def release_only(expr):
    """Whether an `if:` expression can hold only for a release (never for a pull request)."""
    e = unwrap(expr)
    branches = split_top(e, "||")
    if len(branches) > 1:
        return all(release_only(b) for b in branches)
    terms = split_top(e, "&&")
    if len(terms) > 1:
        return any(release_only(t) for t in terms)
    return any(p.fullmatch(e) for p in RELEASE_ONLY)


def events(lines, i, end):
    m = KEY.match(lines[i][2])
    inline = (m.group(3) or "").strip()
    if inline:
        return set(re.findall(r"[\w-]+", inline))
    return {KEY.match(lines[k][2]).group(2) if KEY.match(lines[k][2])
            else lines[k][2].strip().lstrip("-").strip().strip("'\"") for k in children(lines, i, end)}


def environment(lines, i):
    """A job's `environment`: its name whether written as a name, a mapping or a flow mapping."""
    end = body(lines, i)
    value = value_of(lines, i)
    m = re.match(r"\{.*\bname\s*:\s*([^,}]+)", value)
    if m:
        return unquote(m.group(1))
    for k in children(lines, i, end):
        km = KEY.match(lines[k][2])
        if km and km.group(2) == "name":
            return unquote(km.group(3) or "")
    return value


def findings_in(name, text):
    lines = lines_of(text)
    tops = [k for k, l in enumerate(lines) if l[3] and l[1] == 0 and KEY.match(l[2])]
    triggers, jobs = set(), []
    for k in tops:
        key, end = KEY.match(lines[k][2]).group(2), body(lines, k)
        if key == "on":
            triggers = events(lines, k, end)
        elif key == "jobs":
            for j in children(lines, k, end):
                jm, jend = KEY.match(lines[j][2]), body(lines, j)
                keys = {KEY.match(lines[c][2]).group(2): c for c in children(lines, j, jend) if KEY.match(lines[c][2])}
                jobs.append((jm.group(2) if jm else "?", j, jend, keys))
    from_pr = bool(triggers & PR_EVENTS)
    found = []
    for k, (number, _, code, _) in enumerate(lines):
        if not (REFERENCE.search(code) or INHERIT.match(code)):
            continue
        job = next((jb for jb in jobs if jb[1] <= k < jb[2]), None)
        if job is None:
            found.append(f"{name}:{number}: the signing secrets are named outside a job, where every job sees them")
            continue
        job_name, _, _, keys = job
        env = environment(lines, keys["environment"]) if "environment" in keys else ""
        if env != ENVIRONMENT:
            found.append(f"{name}:{number}: job `{job_name}` uses the signing secrets without `environment: {ENVIRONMENT}`")
        elif from_pr and not ("if" in keys and release_only(value_of(lines, keys["if"]))):
            found.append(
                f"{name}:{number}: job `{job_name}` uses the signing secrets, and a pull request can start it "
                "(its `if:` doesn't hold only for a release)"
            )
    return found


def check(root):
    found = []
    folder = root / ".github" / "workflows"
    for p in sorted([*folder.glob("*.yml"), *folder.glob("*.yaml")]):
        found += findings_in(p.relative_to(root).as_posix(), p.read_text(encoding="utf-8"))
    return found


def self_test():
    here = pathlib.Path(__file__).resolve().parent / "tests" / "workflows"
    failures = []
    fixtures = sorted(here.glob("*.yml.fixture"))
    for f in fixtures:
        flagged = bool(findings_in(f.name, f.read_text(encoding="utf-8")))
        if f.name.startswith("flag_") and not flagged:
            failures.append(f"{f.name}: not flagged")
        if f.name.startswith("pass_") and flagged:
            failures.append(f"{f.name}: flagged")
    for f in failures:
        print(f"self-test: {f}")
    print(f"self-test: {len(fixtures)} fixtures, {len(failures)} failed")
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
