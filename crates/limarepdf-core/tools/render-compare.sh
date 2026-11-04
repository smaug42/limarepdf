#!/bin/sh
# Compares LimarePDF's rendering with poppler's pdftoppm over public test PDFs and writes a
# report with side-by-side pictures of the pages that differ (docs/developer/testing.md). Downloads
# the files first if needed (tools/fetch-render-samples.sh). Needs pdftoppm (poppler-utils)
# and curl.
#
# Usage, from crates/limarepdf-core: sh tools/render-compare.sh [part of a file name]
# RENDER_COMPARE_DPI sets the resolution (96 by default).
set -eu
cd "$(dirname "$0")/.."
command -v pdftoppm >/dev/null 2>&1 || {
    echo "needs pdftoppm (poppler-utils)" >&2
    exit 1
}
# Files that couldn't be downloaded (status 2) are named and counted; the rest are compared.
sh tools/fetch-render-samples.sh || [ $? -eq 2 ]
RENDER_COMPARE_ONLY=${1:-} cargo test --release -p limarepdf-core --test render_compare \
    -- --ignored --nocapture
