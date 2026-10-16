#!/usr/bin/env bash
# Makes MPE-<version>.dmg holding a universal MPE.app (Apple Silicon and Intel), on macOS.
#
#   packaging/macos/make-dmg.sh <output folder>
#
# Run from the repository root after building both targets:
#   cargo build --release --target aarch64-apple-darwin --target x86_64-apple-darwin
# The app is signed ad hoc only (no Apple Developer ID), so Gatekeeper asks once.
set -euo pipefail

out=$1
here=$(cd "$(dirname "$0")" && pwd)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
work=$(mktemp -d)
app="$work/stage/MPE.app"

mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
for bin in mpe-gui mpe; do
    lipo -create -output "$app/Contents/MacOS/$bin" \
        "target/aarch64-apple-darwin/release/$bin" "target/x86_64-apple-darwin/release/$bin"
done
sed "s/VERSION/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"

# The icon: every size iconutil wants up to the source's 256 px.
iconset="$work/mpe.iconset"
mkdir "$iconset"
for size in 16 32 128 256; do
    sips -z $size $size docs/images/icon.png --out "$iconset/icon_${size}x${size}.png" > /dev/null
done
for size in 16 32 128; do
    sips -z $((size * 2)) $((size * 2)) docs/images/icon.png --out "$iconset/icon_${size}x${size}@2x.png" > /dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/mpe.icns"

# Docs and licences, as in the archives.
res="$app/Contents/Resources"
cp README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE "$res/"
cp crates/mpe-core/models/README.md "$res/OCR-MODEL.md"
cp crates/mpe-gui/assets/fonts/OFL.txt "$res/FONT-LICENSE-Inter.txt"
for f in OFL-Caveat OFL-CedarvilleCursive LICENSE-HomemadeApple OFL-Zeyada; do
  cp "crates/mpe-core/assets/fonts/$f.txt" "$res/FONT-LICENSE-${f#*-}.txt"
done

# lipo drops the linker's ad hoc signatures; Apple Silicon won't run unsigned code.
codesign --force --sign - "$app/Contents/MacOS/mpe"
codesign --force --sign - "$app"
codesign --verify --strict "$app"

ln -s /Applications "$work/stage/Applications"
mkdir -p "$out"
hdiutil create -volname "MPE $version" -srcfolder "$work/stage" -fs HFS+ -format UDZO -ov \
    "$out/MPE-$version.dmg"
rm -rf "$work"
