#!/usr/bin/env bash
# Compares this tree's limarepdf-cli with an earlier build over every PDF in tests/data (the
# committed ones, and the render corpus and dss/ samples when fetched), to see what a change does
# to real files (docs/developer/testing.md). Builds this tree's CLI (ci-test profile, two jobs) and
# takes the executable Cargo reports, then for each file runs `verify` with both and diffs the
# outputs and exit codes, counts the exit codes of the signed files, and runs `info` with both,
# comparing the notes on stderr and the exit codes and timing each pass. PDFs given after the base
# CLI (crafted attack files, say) are compared too, and shown in full.
#
# Usage, from anywhere: bash crates/limarepdf-core/tools/compare-cli.sh BASE-CLI [PDF...]
# For BASE-CLI, build the CLI before the change (`cargo build --profile ci-test -p
# limarepdf-cli`) and copy it out of the target folder. The outputs and summary.txt go to
# target/tmp/compare-cli (COMPARE_CLI_OUT), emptied first if this script made it; a folder it
# didn't make is refused. Exits 0 when nothing differs, 1 when something does, and 2 when the
# comparison couldn't be made.
set -Eeuo pipefail
# A step that fails exits 2: 1 is for differences found.
trap 'exit 2' ERR
base=$(realpath "${1:?usage: compare-cli.sh BASE-CLI [PDF...]}")
shift
extra=()
for f in "$@"; do extra+=("$(realpath "$f")"); done
cd "$(dirname "$0")/../../.."
out=${COMPARE_CLI_OUT:-target/tmp/compare-cli}
# Only a folder this script made, with its marker, is emptied.
if [ -e "$out" ]; then
    if [ ! -f "$out/.compare-cli" ]; then
        echo "compare-cli didn't make $out and won't empty it" >&2
        exit 2
    fi
    rm -rf "$out"
fi
mkdir -p "$out"/verify-before "$out"/verify-after "$out"/info-before "$out"/info-after
touch "$out/.compare-cli"

cargo build -j2 --profile ci-test -p limarepdf-cli --message-format=json-render-diagnostics \
    > "$out/build.jsonl" 2> "$out/build.log" || { tail "$out/build.log" >&2; exit 2; }
# Cargo reports the executable path even when the target directory or target triple is configured.
built=$(python3 -I - "$out/build.jsonl" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    for line in stream:
        artifact = json.loads(line)
        if (artifact["reason"] == "compiler-artifact"
                and artifact["target"]["name"] == "limarepdf-cli"
                and "bin" in artifact["target"]["kind"] and artifact["executable"]):
            print(artifact["executable"])
            break
    else:
        sys.exit("Cargo did not report a limarepdf-cli executable")
PY
)
cp "$built" "$out/cli-after"
after=$out/cli-after
if [ ! -x "$base" ] || [ ! -x "$after" ]; then
    echo "compare-cli needs executable before and after binaries" >&2
    exit 2
fi
{
    find crates/limarepdf-core/tests/data -name '*.pdf'
    if [ "$#" -gt 0 ]; then printf '%s\n' "${extra[@]}"; fi
} | sort -u > "$out/all.txt"
if [ ! -s "$out/all.txt" ]; then
    echo "compare-cli found no PDFs to compare" >&2
    exit 2
fi

# Each file's output is kept under its path with `/` as `_`.
verify() { # cli outdir
    local f o status
    while IFS= read -r f; do
        o="$2/$(printf '%s' "$f" | tr / _).txt"
        status=0
        timeout 60 "$1" verify "$f" < /dev/null > "$o" 2>&1 || status=$?
        echo "exit $status" >> "$o"
    done < "$out/all.txt"
}
verify "$base" "$out/verify-before"
verify "$after" "$out/verify-after"
# grep returns 1 if none of the files named a signature.
grep -l -E ': [0-9]+ digital signatures?$' "$out"/verify-before/*.txt > "$out/signed-paths.txt" ||
    [ "$?" -eq 1 ]
while IFS= read -r p; do basename "$p"; done < "$out/signed-paths.txt" > "$out/signed.txt"

info() { # cli outdir pass
    local f o status t0
    t0=$(date +%s.%N)
    while IFS= read -r f; do
        o="$2/$(printf '%s' "$f" | tr / _).txt"
        status=0
        timeout 60 "$1" info "$f" < /dev/null > /dev/null 2> "$out/info-stderr.txt" || status=$?
        # The notes once, from the second pass, and each pass's exit code.
        if [ "$3" = 2 ]; then
            grep -v '^[[:space:]]*$' "$out/info-stderr.txt" >> "$o" || [ "$?" -eq 1 ]
        fi
        echo "pass $3: exit $status" >> "$o"
    done < "$out/all.txt"
    info_seconds=$(awk -v a="$t0" -v b="$(date +%s.%N)" 'BEGIN { printf "%.1f", b - a }')
}
# Twice each, alternating, so the file cache is warm for both.
info "$base" "$out/info-before" 1
tb1=$info_seconds
info "$after" "$out/info-after" 1
ta1=$info_seconds
info "$base" "$out/info-before" 2
tb2=$info_seconds
info "$after" "$out/info-after" 2
ta2=$info_seconds

changed=0
for command in verify info; do
    if diff -r "$out/$command-before" "$out/$command-after" > "$out/$command-diff.txt"; then
        continue
    else
        status=$?
    fi
    if [ "$status" -ne 1 ]; then
        echo "Could not compare $command logs in $out" >&2
        exit 2
    fi
    changed=1
done

{
    echo "$(wc -l < "$out/all.txt") PDFs compared; before: $base"
    echo "verify: files whose output names a signature (before): $(wc -l < "$out/signed.txt")"
    echo "verify: diff before after: $(wc -l < "$out/verify-diff.txt") lines"
    echo "verify: exit codes after, signed files:"
    while IFS= read -r o; do tail -1 "$out/verify-after/$o"; done < "$out/signed.txt" | sort | uniq -c
    echo "info: seconds, before: $tb1 $tb2, after: $ta1 $ta2"
    note_count() { awk '/^note/ { n++ } END { print n+0 }' "$1"/*.txt; }
    echo "info: notes before: $(note_count "$out/info-before"), after: $(note_count "$out/info-after")"
    echo "info: diff before after: $(wc -l < "$out/info-diff.txt") lines"
    cat "$out/verify-diff.txt" "$out/info-diff.txt"
    # The extra PDFs in full: a failure both builds share shows no difference.
    if [ "$#" -gt 0 ]; then
        for f in "${extra[@]}"; do
            o=$(printf '%s' "$f" | tr / _).txt
            for side in before after; do
                echo "== $f with $side"
                cat "$out/verify-$side/$o" "$out/info-$side/$o"
            done
        done
    fi
} > "$out/summary.txt"
cat "$out/summary.txt"
echo "Reports: $out"
exit "$changed"
