#!/bin/sh
# Fills fuzz/corpus/<target> with starting inputs from the repository's own test files (made-up
# documents and the signature samples, all committed) and a few content streams. The fuzzer
# adds to these folders as it runs; they are git-ignored.
#
# Usage, from the repository: sh fuzz/seed.sh
set -eu
cd "$(dirname "$0")"
data=../crates/limarepdf-core/tests/data
mkdir -p corpus/open corpus/readers corpus/content corpus/draw corpus/images

for target in open readers; do
    find "$data/paragraphs/madeup" "$data/signatures/pyhanko" "$data/signatures/node-signpdf" \
        -name '*.pdf' -size -200k -exec cp {} "corpus/$target/" \;
done

# Content streams: text in the page's font, a path, colours, the page's picture, a saved
# state, an inline image and marked content.
n=0
for c in \
    'BT /F1 12 Tf 20 150 Td (Hello, world) Tj ET' \
    'BT /F1 10 Tf 12 TL 20 180 Td [(Ke) 20 (rn) -300 (ing)] TJ T* (next line) Tj ET' \
    '0.5 g 20 20 100 50 re f 1 0 0 RG 2 w 20 100 m 180 120 l S' \
    'q 50 0 0 50 75 75 cm /Im1 Do Q' \
    'q 1 0 0 1 10 10 cm 0 0 1 rg 0 0 m 50 0 l 50 50 l h f Q' \
    'BI /W 2 /H 2 /CS /G /BPC 8 ID \0000\0100\0200\0377 EI' \
    '/P <</MCID 0>> BDC BT /F1 9 Tf 3 Tr 20 40 Td (invisible) Tj ET EMC'; do
    n=$((n + 1))
    printf '%b' "$c" > "corpus/content/$n"
    cp "corpus/content/$n" "corpus/draw/$n"
done

cp "$data/images/cmyk-cyan-red.jpg" corpus/images/
cp ../crates/limarepdf-gui/assets/maple-leaf.png corpus/images/
echo "seeded: $(ls corpus/open | wc -l) PDFs, $n content streams, $(ls corpus/images | wc -l) pictures"
