#!/bin/sh
# Fills fuzz/corpus/<target> with starting inputs from the repository's own test files (made-up
# documents, the signature samples, test certificates and digital IDs, all committed) and a few
# content streams and printing-service answers. The fuzzer adds to these folders as it runs;
# they are git-ignored.
#
# Usage, from the repository: sh fuzz/seed.sh
set -eu
cd "$(dirname "$0")"
data=../crates/limarepdf-core/tests/data
signatures="$data/signatures/pyhanko $data/signatures/node-signpdf"
mkdir -p corpus/open corpus/readers corpus/content corpus/draw corpus/images \
    corpus/verify corpus/signature corpus/trustfiles corpus/pkcs12 corpus/ipp

for target in open readers; do
    # shellcheck disable=SC2086 # $signatures is two folders
    find "$data/paragraphs/madeup" $signatures -name '*.pdf' -size -200k \
        -exec cp {} "corpus/$target/" \;
done
# shellcheck disable=SC2086
find $signatures -name '*.pdf' -exec cp {} corpus/verify/ \;

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

# The rest is made by Python: each sample's signatures, as the `signature` target takes them; a
# trust list of test certificates; the digital IDs, each after the byte that picks its password;
# and answers as CUPS writes them.
python3 -I - "$data" <<'EOF'
import base64, hashlib, pathlib, re, struct, sys, textwrap

data = pathlib.Path(sys.argv[1])
corpus = pathlib.Path("corpus")

# `signature`: the first byte picks the /SubFilter, in fuzz_targets/signature.rs's order.
kinds = ["adbe.pkcs7.detached", "ETSI.CAdES.detached", "adbe.pkcs7.sha1", "ETSI.RFC3161",
         "adbe.x509.rsa_sha1"]
def der(b):
    """The DER value `b` starts with, without the zeros padding /Contents after it."""
    if len(b) < 2:
        return b
    k = b[1] & 0x7F if b[1] >= 0x80 else 0
    length = int.from_bytes(b[2:2 + k], "big") if k else b[1]
    return b[:2 + k + length]
n = 0
for pdf in sorted((data / "signatures").glob("*/*.pdf")):
    if pdf.parent.name not in ("pyhanko", "node-signpdf"):
        continue
    for body in re.findall(rb"\bobj\b(.*?)\bendobj\b", pdf.read_bytes(), re.S):
        kind = re.search(rb"/SubFilter\s*/([^\s/<>\[\]()]+)", body)
        contents = re.search(rb"/Contents\s*<([0-9A-Fa-f\s]*)>", body)
        if not (kind and contents) or kind.group(1).decode() not in kinds:
            continue
        value = der(bytes.fromhex(re.sub(rb"\s", b"", contents.group(1)).decode()))
        n += 1
        pick = kinds.index(kind.group(1).decode())
        (corpus / "signature" / f"{n}").write_bytes(bytes([pick]) + value)

# `trustfiles`: a list as LimarePDF writes it, with an authority, a signer and an entry it
# can't use.
def entry(role, name, label, added):
    der = (data / "signatures" / "keys" / name).read_bytes()
    fingerprint = ":".join(f"{b:02X}" for b in hashlib.sha256(der).digest())
    pem = "\n".join(textwrap.wrap(base64.b64encode(der).decode(), 64))
    return (f"\n[{role}]\nlabel = {label}\nadded = {added}\nsha256 = {fingerprint}\n"
            f"-----BEGIN CERTIFICATE-----\n{pem}\n-----END CERTIFICATE-----\n")
(corpus / "trustfiles" / "1").write_text(
    "# Certificates LimarePDF trusts when it checks digital signatures.\n"
    + entry("authority", "ca.crt.der", "Test CA", "2026-11-04")
    + entry("signer", "t-leaf.crt.der", "Test signer", "2026-11-11")
    + entry("authority", "t-leaf.crt.der", "Not an authority", "2026-11-11")
    + "\n[witness]\nlabel = an unknown kind\n")

# `pkcs12`: the byte before each ID picks its password, in fuzz_targets/pkcs12.rs's order.
for p12 in sorted((data / "signatures" / "ids").glob("*.p12")):
    pick = 1 if p12.stem.startswith("non-ascii") else 2 if p12.stem == "empty-password" else 0
    (corpus / "pkcs12" / p12.stem).write_bytes(bytes([pick]) + p12.read_bytes())

# `ipp`: answers to Get-Printer-Attributes and CUPS-Get-Printers (RFC 8010), as in core's
# ipp.rs tests.
def put(tag, name, value):
    return struct.pack(">BH", tag, len(name)) + name + struct.pack(">H", len(value)) + value
def integer(name, n):
    return put(0x21, name, struct.pack(">i", n))
def collection(name, members):
    out = put(0x34, name, b"")
    for member, values in members:
        out += put(0x4A, b"", member)
        for v in values:
            out += v
    return out + put(0x37, b"", b"")
def media_col(name, w, h, margins):
    size = collection(b"", [(b"x-dimension", [integer(b"", w)]),
                            (b"y-dimension", [integer(b"", h)])])
    sides = [b"media-top-margin", b"media-right-margin", b"media-bottom-margin",
             b"media-left-margin"]
    return collection(name, [(b"media-size", [size])]
                      + [(s, [integer(b"", m)]) for s, m in zip(sides, margins)])
def answer(printers):
    out = struct.pack(">BBHI", 1, 1, 0, 1) + b"\x01"
    out += put(0x47, b"attributes-charset", b"utf-8")
    out += put(0x48, b"attributes-natural-language", b"en")
    out += put(0x41, b"status-message", b"successful-ok")
    for attributes in printers:
        out += b"\x04" + b"".join(attributes)
    return out + b"\x03"
sizes = [put(0x44, b"media-supported", b"na_letter_8.5x11in"),
         put(0x44, b"", b"iso_a4_210x297mm"),
         put(0x44, b"", b"custom_min_3x5in"),
         put(0x44, b"", b"custom_max_8.5x14in"),
         put(0x44, b"media-default", b"iso_a4_210x297mm")]
database = [media_col(b"media-col-database", 21590, 27940, [423, 423, 1000, 423]),
            media_col(b"", 21000, 29700, [423, 423, 1000, 423])]
(corpus / "ipp" / "1").write_bytes(answer([sizes + database]))
(corpus / "ipp" / "2").write_bytes(answer([
    [put(0x42, b"printer-name", b"Office"), put(0x45, b"printer-uri-supported",
                                                  b"ipp://localhost/printers/Office")],
    [put(0x42, b"printer-name", b"Home"), put(0x23, b"printer-state", struct.pack(">i", 3))],
]))
print(f"{n} signatures, 1 trust list, "
      f"{len(list((corpus / 'pkcs12').iterdir()))} digital IDs, 2 printing answers")
EOF
echo "seeded: $(ls corpus/open | wc -l) PDFs, $n content streams, $(ls corpus/images | wc -l) pictures"
