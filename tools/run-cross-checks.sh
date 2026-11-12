#!/usr/bin/env bash
# Runs the cross-checks, group G6 of tools/ignored-tests.toml, by their exact names: Poppler,
# Ghostscript and OpenSSL read what LimarePDF wrote (cycle plan WP-11, V-10). CI's cross-check
# job runs it; so can you, with them installed.
#
#     bash tools/run-cross-checks.sh
#
# Each program the tests need is looked for first, and a missing one fails the run by name, so no
# test finds it missing and checks nothing. Then each test target's cross-checks run in one
# `cargo test` with --exact, so a test whose name only contains `cross_check` (the CUPS tests,
# group G5) never runs. Last, the tests libtest says ran and passed must be the list's, no more and
# no fewer; they're kept in target/cross-checks/ran.txt.
set -euo pipefail
cd "$(dirname "$0")/.."

list() { python3 -I tools/check-ignored-tests.py "$@"; }

missing=""
for program in $(list --needs G6); do
  command -v "$program" > /dev/null || missing="$missing $program"
done
if [ -n "$missing" ]; then
  echo "::error::not installed:$missing"; exit 1
fi

out=target/cross-checks
mkdir -p "$out"
list --list G6 | sort > "$out/listed.txt"
: > "$out/ran.txt"
failed=""
while IFS=$'\t' read -r crate target; do
  read -ra select <<< "$target"
  mapfile -t names < <(awk -F'\t' -v c="$crate" -v t="$target" '$1 == c && $2 == t {print $3}' "$out/listed.txt")
  echo "::group::$crate $target: ${#names[@]}"
  cargo test -p "$crate" "${select[@]}" --profile ci-test --locked -- --ignored --exact "${names[@]}" \
    2>&1 | tee "$out/log.txt" || failed="$failed $crate $target;"
  echo "::endgroup::"
  awk -v c="$crate" -v t="$target" '/^test .* \.\.\. ok$/ {print c "\t" t "\t" $2}' "$out/log.txt" >> "$out/ran.txt"
done < <(cut -f1,2 "$out/listed.txt" | sort -u)

sort -o "$out/ran.txt" "$out/ran.txt"
if [ -n "$failed" ]; then
  echo "::error::cross-checks failed in:$failed"; exit 1
fi
if ! diff "$out/listed.txt" "$out/ran.txt"; then
  echo "::error::the cross-checks that ran (>) aren't those listed in tools/ignored-tests.toml (<)"; exit 1
fi
echo "$(wc -l < "$out/ran.txt") cross-checks ran and passed, each listed in tools/ignored-tests.toml:"
cat "$out/ran.txt"
