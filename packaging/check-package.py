"""Checks an unpacked archive, package or installed folder: the programs, docs and licences are all there, and
every link in its Markdown files works. A relative link must reach a file in the folder (and a heading in it, for
#anchors); a link to this version's tag on GitHub (packaging/package-docs.sh writes them) must reach a file in this
repository. Other web links aren't followed.

THIRD-PARTY-LICENSES.html (packaging/package-notices.sh) must have a notice for each crate built into the programs
for the folder's targets, by name and version, and none for a crate that isn't. The crates built are the programs'
normal dependencies as `cargo tree` lists them (cargo metadata's graph also has optional dependencies whose features
are off), less the workspace's own. The notices for fonts and code that crates bundle must be there too.

Each program must be built for the targets' architectures, as its header says (ELF, PE, or Mach-O, universal or
not): a toolchain for another architecture, an emulated one say, builds programs that run but aren't the package's.
A Windows program must not import the Visual C++ runtime's DLLs, delay-loaded or not: Windows doesn't have them, so
a program needing them doesn't start on a computer that hasn't installed them (#324), though GitHub's runners have.

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
# The programs, and the crates whose dependencies are shipped in them.
PROGRAMS = ["limarepdf", "limarepdf-cli"]
SHIPPED = ["limarepdf-gui", "limarepdf-cli"]
# A target triple's architecture, by the machine number in each kind of program header.
ELF = {62: "x86_64", 183: "aarch64"}
PE = {0x8664: "x86_64", 0xAA64: "aarch64"}
MACHO = {0x01000007: "x86_64", 0x0100000C: "aarch64"}
# The Visual C++ runtime's DLLs, which Windows doesn't have (vcruntime140.dll, msvcp140.dll, ...). The Universal C
# runtime (ucrtbase.dll, api-ms-win-crt-*.dll) is part of Windows 10 and 11, and msvcrt.dll is Windows's own.
VC_RUNTIME = re.compile(r"(vcruntime|msvcp|concrt|vccorlib|vcamp|vcomp|mfc)\d", re.I)
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


def machines(head):
    """The architectures a program's first bytes say it is built for: ELF (Linux), PE (Windows), Mach-O, universal
    or not (macOS)."""

    def number(start, size, order="little"):
        return int.from_bytes(head[start : start + size], order)

    pe = number(0x3C, 4)
    if head[:4] == b"\x7fELF":
        found = [ELF.get(number(18, 2))]
    elif head[:2] == b"MZ" and head[pe : pe + 4] == b"PE\0\0":
        found = [PE.get(number(pe + 4, 2))]
    elif head[:4] == b"\xca\xfe\xba\xbe":  # universal: a count, then 20 bytes per architecture, big-endian
        found = [MACHO.get(number(8 + 20 * i, 4, "big")) for i in range(number(4, 4, "big"))]
    elif head[:4] == b"\xcf\xfa\xed\xfe":
        found = [MACHO.get(number(4, 4))]
    else:
        found = [None]
    return {f or "an unknown architecture" for f in found}


def architectures(programs, targets):
    """The problems with the programs' architectures, given each program's bytes (name: bytes) and the targets."""
    wanted = {t.split("-")[0] for t in targets}
    problems = []
    for name, head in sorted(programs.items()):
        got = machines(head)
        if got != wanted:
            problems.append(f"{name}: built for {' and '.join(sorted(got))}, not {' and '.join(sorted(wanted))}")
    return problems


def imports(data):
    """The DLLs a Windows program imports, delay-loaded ones too, as its import tables name them; none for another
    kind of program."""

    def number(start, size=4):
        return int.from_bytes(data[start : start + size], "little")

    pe = number(0x3C)
    if data[:2] != b"MZ" or data[pe : pe + 4] != b"PE\0\0":
        return []
    optional = pe + 24
    directories = optional + (112 if number(optional, 2) == 0x20B else 96)  # PE32+ or PE32
    sections = [optional + number(pe + 20, 2) + 40 * i for i in range(number(pe + 6, 2))]

    def offset(rva):
        """Where an address in memory is in the file, by the section holding it."""
        for s in sections:
            start, size = number(s + 12), number(s + 8) or number(s + 16)
            if start <= rva < start + size:
                return rva - start + number(s + 20)
        return None

    def name(rva):
        start = offset(rva)
        end = -1 if start is None else data.find(b"\0", start)
        return data[start:end].decode("ascii", "replace") if end >= 0 else "an unreadable name"

    names = []
    # The import table's entries are 20 bytes, the name's address at 12; the delay-load table's 32, at 4. Each ends
    # with an empty entry.
    for directory, size, at in ((1, 20, 12), (13, 32, 4)):
        rva = number(directories + 8 * directory)
        start = offset(rva) if rva else None
        while start is not None and any(data[start : start + size]):
            names.append(name(number(start + at)))
            start += size
    return names


def runtimes(programs):
    """The problems with the programs' DLLs, given each program's bytes (name: bytes): none may be the Visual C++
    runtime's."""
    problems = []
    for name, data in sorted(programs.items()):
        needs = sorted({d for d in imports(data) if VC_RUNTIME.match(d)}, key=str.lower)
        if needs:
            problems.append(f"{name}: needs the Visual C++ runtime ({', '.join(needs)}), which Windows doesn't have")
    return problems


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
    for name in NEEDED + PROGRAMS:
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
    # Read whole: a Windows program's import tables can be anywhere in it.
    programs = {
        p.name: p.read_bytes() for p in folder.rglob("*") if p.name.removesuffix(".exe") in PROGRAMS and p.is_file()
    }
    return problems + architectures(programs, targets) + runtimes(programs) + notices(files, built(targets))


def self_test():
    """The crates built, from a workspace with each kind of dependency (tools/tests/package), the notices checked
    against them, the programs' architectures read from made-up headers, and the DLLs from made-up Windows
    programs."""
    ws = REPO / "tools" / "tests" / "package" / "ws"
    every = {("common", "1.0.0"), ("common", "2.0.0"), ("mid", "0.1.0"), ("derive-it", "0.1.0"), ("via-inner", "0.1.0")}
    linux = every | {("linux-only", "0.1.0")}
    macs = every | {("arm-mac", "0.1.0"), ("intel-mac", "0.1.0")}

    def page(crates, text=""):
        return "".join(f'<li data-crate="{n} {v}">' for n, v in sorted(crates)) + text

    foxit = {("hayro-font", "0.5.0")}
    cantarell = {("sctk-adwaita", "0.10.1")}

    def elf(machine):
        return b"\x7fELF" + bytes(14) + machine.to_bytes(2, "little")

    def pe(machine):
        return b"MZ" + bytes(0x3A) + (0x40).to_bytes(4, "little") + b"PE\0\0" + machine.to_bytes(2, "little")

    def macho(*cpus):
        if len(cpus) == 1:
            return b"\xcf\xfa\xed\xfe" + cpus[0].to_bytes(4, "little")
        entries = b"".join(c.to_bytes(4, "big") + bytes(16) for c in cpus)
        return b"\xca\xfe\xba\xbe" + len(cpus).to_bytes(4, "big") + entries

    def windows(imported, delayed=()):
        """A made-up x64 Windows program whose one section (at 0x1000 in memory, 0x200 in the file) holds its import
        table, its delay-load table, the DLLs' names and the one list of what it takes from each: the first
        function, by number."""

        def le(n, size=4):
            return n.to_bytes(size, "little")

        rva, at = 0x1000, 0x200
        delay = 20 * (len(imported) + 1)
        thunks = delay + 32 * (len(delayed) + 1)
        body = bytearray(thunks) + le(1 << 63 | 1, 8) + bytes(8)
        taken = le(rva + thunks)
        for i, dll in enumerate(imported):  # what it takes, the name's address, the addresses it fills in
            body[20 * i : 20 * i + 20] = taken + bytes(8) + le(rva + len(body)) + taken
            body += dll.encode() + b"\0"
        for i, dll in enumerate(delayed):  # addresses not pointers, the name's, the module's, filled in, taken
            body[delay + 32 * i : delay + 32 * i + 20] = le(1) + le(rva + len(body)) + bytes(4) + taken + taken
            body += dll.encode() + b"\0"
        directories = bytearray(16 * 8)  # each an address and a size
        directories[8:16] = le(rva) + le(delay)  # 1, the import table
        if delayed:
            directories[104:112] = le(rva + delay) + le(32 * (len(delayed) + 1))  # 13, the delay-load table
        optional = le(0x20B, 2) + bytes(106) + le(16) + directories  # PE32+: the directories' count at 108
        coff = le(0x8664, 2) + le(1, 2) + bytes(12) + le(len(optional), 2) + bytes(2)
        section = b".rdata\0\0" + le(len(body)) + le(rva) + le(len(body)) + le(at) + bytes(16)
        head = b"MZ" + bytes(0x3A) + le(0x40) + b"PE\0\0" + coff + optional + section
        return head + bytes(at - len(head)) + body

    linux_x86, linux_arm = ["x86_64-unknown-linux-gnu"], ["aarch64-unknown-linux-gnu"]
    windows_arm, both_macs = ["aarch64-pc-windows-msvc"], ["aarch64-apple-darwin", "x86_64-apple-darwin"]
    cases = [
        ("Linux programs for x86_64", lambda: architectures({"limarepdf": elf(62), "cli": elf(62)}, linux_x86), []),
        ("Linux programs for Arm64", lambda: architectures({"limarepdf": elf(183)}, linux_arm), []),
        (
            "an x86_64 program in an Arm64 package",
            lambda: architectures({"limarepdf": elf(183), "limarepdf-cli": elf(62)}, linux_arm),
            ["limarepdf-cli: built for x86_64, not aarch64"],
        ),
        ("Windows programs for Arm64", lambda: architectures({"limarepdf.exe": pe(0xAA64)}, windows_arm), []),
        (
            "an emulated toolchain's x64 program for Windows on Arm",
            lambda: architectures({"limarepdf.exe": pe(0x8664)}, windows_arm),
            ["limarepdf.exe: built for x86_64, not aarch64"],
        ),
        ("the universal Mac app", lambda: architectures({"limarepdf": macho(0x0100000C, 0x01000007)}, both_macs), []),
        (
            "a half missing from the universal Mac app",
            lambda: architectures({"limarepdf": macho(0x0100000C)}, both_macs),
            ["limarepdf: built for aarch64, not aarch64 and x86_64"],
        ),
        (
            "not a program",
            lambda: architectures({"limarepdf": b"#!/bin/sh\n"}, linux_x86),
            ["limarepdf: built for an unknown architecture, not x86_64"],
        ),
        (
            "a Windows program's DLLs, delay-loaded ones too",
            lambda: imports(windows(["KERNEL32.dll", "VCRUNTIME140.dll"], ["bcrypt.dll"])),
            ["KERNEL32.dll", "VCRUNTIME140.dll", "bcrypt.dll"],
        ),
        (
            "a Windows program needing the Visual C++ runtime",
            lambda: runtimes(
                {"limarepdf.exe": windows(["KERNEL32.dll", "VCRUNTIME140.dll", "api-ms-win-crt-heap-l1-1-0.dll"])}
            ),
            ["limarepdf.exe: needs the Visual C++ runtime (VCRUNTIME140.dll), which Windows doesn't have"],
        ),
        (
            "a Visual C++ runtime DLL delay-loaded",
            lambda: runtimes({"limarepdf-cli.exe": windows(["KERNEL32.dll"], ["msvcp140.dll"])}),
            ["limarepdf-cli.exe: needs the Visual C++ runtime (msvcp140.dll), which Windows doesn't have"],
        ),
        (
            "a Windows program needing only Windows's DLLs",
            lambda: runtimes(
                {
                    "limarepdf.exe": windows(
                        ["KERNEL32.dll", "ucrtbase.dll", "api-ms-win-crt-runtime-l1-1-0.dll", "msvcrt.dll"],
                        ["USER32.dll"],
                    )
                }
            ),
            [],
        ),
        ("a Linux program", lambda: runtimes({"limarepdf": elf(62)}), []),
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
