#!/bin/sh
# Downloads the public test PDFs listed in tools/render-samples.txt into tests/data/render,
# for the opt-in poppler comparison (tools/render-compare.sh). They are not committed:
# they come from bug reports with no stated licence, and the set is about 400 MB. The
# folder is git-ignored. Files already there with the right SHA-256 are kept; a file whose
# download doesn't match its SHA-256 is deleted and reported.
#
# Usage, from crates/limarepdf-core: sh tools/fetch-render-samples.sh
set -eu
cd "$(dirname "$0")/.."
list=tools/render-samples.txt
out=tests/data/render
pdfjs=https://raw.githubusercontent.com/mozilla/pdf.js/157d30e833e5afbd282db5b1280aae4843f55c0a/test/pdfs
mirror=https://hayro-assets.dev

if command -v sha256sum >/dev/null 2>&1; then
    sha() { sha256sum "$1" | cut -d' ' -f1; }
else
    sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

# Prints "url path sha256" for each listed file.
files() {
    grep -v '^#' "$list" | while read -r src name sum _; do
        [ -n "$src" ] || continue
        case $src in
            pdfjs) echo "$pdfjs/$name.pdf $out/pdfjs/$name.pdf $sum" ;;
            hayro-pdfjs) echo "$mirror/pdfjs/$name.pdf $out/pdfjs/$name.pdf $sum" ;;
            hayro-pdfbox) echo "$mirror/pdfbox/$name.pdf $out/pdfbox/$name.pdf $sum" ;;
            *) echo "unknown source $src" >&2; exit 1 ;;
        esac
    done
}

mkdir -p "$out/pdfjs" "$out/pdfbox"
cfg=$(mktemp)
trap 'rm -f "$cfg"' EXIT
files | while read -r url path sum; do
    if [ ! -f "$path" ] || [ "$(sha "$path")" != "$sum" ]; then
        printf 'url = "%s"\noutput = "%s"\n' "$url" "$path" >>"$cfg"
    fi
done
if [ -s "$cfg" ]; then
    echo "downloading $(grep -c '^url' "$cfg") files"
    curl -sfL --retry 2 --parallel --parallel-max 8 -K "$cfg" || true
fi

files | while read -r url path sum; do
    if [ ! -f "$path" ] || [ "$(sha "$path")" != "$sum" ]; then
        rm -f "$path"
        echo "failed: $url" >&2
    fi
done
echo "$(find "$out" -name '*.pdf' | wc -l) files in $out"
