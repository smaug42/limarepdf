#!/usr/bin/env bash
# Exercise compare-cli's shell plumbing without building Rust or opening real documents.
set -euo pipefail
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
root=$scratch/tree
tools=$root/crates/limarepdf-core/tools
mkdir -p "$tools" "$root/crates/limarepdf-core/tests/data" "$scratch/bin" \
    "$root/target/ci-test" "$scratch/custom target/ci-test"
cp "$(dirname "$0")/compare-cli.sh" "$tools/compare-cli.sh"
touch "$root/crates/limarepdf-core/tests/data/sample.pdf" "$scratch/extra.pdf"
cat > "$scratch/before" <<'STUB'
#!/usr/bin/env bash
exit 0
STUB
cp "$scratch/before" "$root/target/ci-test/limarepdf-cli"
cat > "$scratch/custom target/ci-test/limarepdf-cli" <<'STUB'
#!/usr/bin/env bash
if [ "$1" = info ]; then exit "${AFTER_INFO_STATUS:-0}"; fi
exit 0
STUB
cat > "$scratch/bin/cargo" <<'STUB'
#!/usr/bin/env bash
printf '{"reason":"compiler-artifact","target":{"name":"limarepdf-cli","kind":["bin"]},"executable":"%s/ci-test/limarepdf-cli"}\n' "$CARGO_TARGET_DIR"
STUB
chmod +x "$scratch/bin/cargo" "$scratch/before" "$root/target/ci-test/limarepdf-cli" \
    "$scratch/custom target/ci-test/limarepdf-cli"
export PATH="$scratch/bin:$PATH" CARGO_TARGET_DIR="$scratch/custom target"

COMPARE_CLI_OUT="$scratch/same" bash "$tools/compare-cli.sh" "$scratch/before" > "$scratch/same.log"
# A folder the script made is emptied and used again.
COMPARE_CLI_OUT="$scratch/same" bash "$tools/compare-cli.sh" "$scratch/before" > "$scratch/same.log"
status=0
COMPARE_CLI_OUT="$scratch/different" AFTER_INFO_STATUS=7 \
    bash "$tools/compare-cli.sh" "$scratch/before" "$scratch/extra.pdf" > "$scratch/diff.log" 2>&1 || status=$?
test "$status" -eq 1
grep -q 'exit 7' "$scratch/different/info-diff.txt"
grep -qF "== $(realpath "$scratch/extra.pdf") with after" "$scratch/different/summary.txt"

mkdir "$scratch/existing"
echo keep > "$scratch/existing/marker"
if COMPARE_CLI_OUT="$scratch/existing" bash "$tools/compare-cli.sh" "$scratch/before" "$scratch/extra.pdf" > "$scratch/existing.log" 2>&1; then
    echo "Compare accepted an existing output directory" >&2
    exit 1
fi
grep -qx keep "$scratch/existing/marker"

cat > "$scratch/bin/diff" <<'STUB'
#!/usr/bin/env bash
exit 2
STUB
chmod +x "$scratch/bin/diff"
status=0
COMPARE_CLI_OUT="$scratch/broken-diff" bash "$tools/compare-cli.sh" "$scratch/before" > "$scratch/broken-diff.log" 2>&1 || status=$?
test "$status" -eq 2
grep -q 'Could not compare verify logs' "$scratch/broken-diff.log"

# A build that names no executable stops the comparison with 2, not 1.
printf '#!/usr/bin/env bash\n' > "$scratch/bin/cargo"
status=0
COMPARE_CLI_OUT="$scratch/no-cli" bash "$tools/compare-cli.sh" "$scratch/before" > "$scratch/no-cli.log" 2>&1 || status=$?
test "$status" -eq 2
grep -q 'did not report a limarepdf-cli executable' "$scratch/no-cli.log"
echo "compare-cli checks passed"
