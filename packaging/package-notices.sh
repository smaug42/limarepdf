#!/usr/bin/env bash
# Writes THIRD-PARTY-LICENSES.html, the licences and notices of every crate built into the programs, into
# target/package-notices/, for the targets given (this computer's if none; the universal Mac app gives both of its
# targets). The settings are in about.toml, the page in packaging/about.hbs. Needs cargo-about, the version below:
#
#   cargo install cargo-about --locked --version 0.9.3 --features cli
#   packaging/package-notices.sh [target]...
#
# Run from the repository root before building a package. It stops when about.toml's licences differ from
# deny.toml's, and on any warning from cargo-about: that is how it reports a clarification whose file has changed,
# leaving the notice out (docs/developer/releasing.md).
set -euo pipefail

pin=0.9.3
out=target/package-notices
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) py=python ;; *) py=python3 ;; esac
# No earlier page left for a package to take when this run stops.
rm -f "$out/THIRD-PARTY-LICENSES.html"

have=$(cargo about --version 2>/dev/null || true)
if [ "$have" != "cargo-about $pin" ]; then
    echo "package-notices.sh: needs cargo-about $pin (found: ${have:-none})" >&2
    exit 1
fi
$py -I - <<'EOF'
import sys, tomllib
allow = tomllib.load(open("deny.toml", "rb"))["licenses"]["allow"]
accepted = tomllib.load(open("about.toml", "rb"))["accepted"]
if sorted(allow) != sorted(accepted):
    sys.exit(f"package-notices.sh: about.toml accepts {sorted(set(accepted) ^ set(allow))} differently from deny.toml")
EOF

targets=("$@")
[ ${#targets[@]} -gt 0 ] || targets=("$(rustc -vV | sed -n 's/^host: //p')")
args=()
for t in "${targets[@]}"; do
    args+=(--target "$t")
done
mkdir -p "$out"
log=$(mktemp)
trap 'rm -f "$log"' EXIT
# The crates' own files, read offline (--frozen) so the page depends on Cargo.lock alone.
cargo fetch --locked
status=0
cargo about generate --locked --frozen --workspace --fail -c about.toml "${args[@]}" \
    packaging/about.hbs -o "$out/THIRD-PARTY-LICENSES.html" 2> "$log" || status=$?
cat "$log" >&2
if [ "$status" -ne 0 ] || grep -q 'WARN\|ERROR' "$log"; then
    rm -f "$out/THIRD-PARTY-LICENSES.html"
    echo "package-notices.sh: cargo-about reported the problems above" >&2
    exit 1
fi
echo "$out/THIRD-PARTY-LICENSES.html: ${targets[*]}"
