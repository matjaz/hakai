#!/usr/bin/env bash
#
# Builds hakai-mac and assembles the release DMG.
#
#   hakai-mac/package.sh
#
# Output: hakai-mac/target/hakai-mac-<version>-universal-macos.dmg — Hakai.app (one
# universal arm64 + x86_64 binary; fonts and all 35 sounds are compiled in) next to an
# /Applications link, README.txt, LICENSE.txt and CREDITS.md.
#
# Signing: ad-hoc by default, which runs fine on this Mac but which Gatekeeper refuses on
# any other until the user clears it (see README.txt). With a Developer ID certificate,
# set SIGN_IDENTITY="Developer ID Application: …" for a hardened-runtime signature, and
# NOTARY_PROFILE=<keychain profile> to notarise and staple the DMG as well.

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

MANIFEST=hakai-mac/Cargo.toml
TARGET_DIR=hakai-mac/target
VERSION="$(sed -nE 's/^version *= *"([^"]+)".*/\1/p' "$MANIFEST" | head -1)"
[ -n "$VERSION" ] || { echo "couldn't read version from $MANIFEST" >&2; exit 1; }
NAME="hakai-mac-$VERSION-universal-macos"
STAGE="$TARGET_DIR/$NAME"
APP="$STAGE/Hakai.app"
DMG="$TARGET_DIR/$NAME.dmg"
IDENTITY="${SIGN_IDENTITY:--}"

# Matches LSMinimumSystemVersion in Info.plist.
export MACOSX_DEPLOYMENT_TARGET=11.0

for t in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --locked --manifest-path "$MANIFEST" --target "$t"
done

rm -rf "$STAGE" "$DMG"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

lipo -create -output "$APP/Contents/MacOS/hakai-mac" \
    "$TARGET_DIR/aarch64-apple-darwin/release/hakai-mac" \
    "$TARGET_DIR/x86_64-apple-darwin/release/hakai-mac"
sed "s/@VERSION@/$VERSION/g" hakai-mac/Info.plist > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

# The icon is procedural (no bitmap assets in the repo): render the 1024 px master, let
# sips/iconutil make every size of the .icns from it.
ICONSET="$TARGET_DIR/AppIcon.iconset"
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
# --target = the host triple, so the example reuses that arch's dependency build above
# instead of compiling everything a third time in the plain target/release dir.
HOST="$(rustc -vV | sed -n 's/^host: //p')"
cargo run --release --locked --manifest-path "$MANIFEST" --target "$HOST" --example app_icon -- "$TARGET_DIR/app_icon.png"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$TARGET_DIR/app_icon.png" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) "$TARGET_DIR/app_icon.png" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$ICONSET"

xattr -cr "$APP"
if [ "$IDENTITY" = "-" ]; then
    codesign --force --sign - "$APP"
else
    codesign --force --sign "$IDENTITY" --options runtime --timestamp "$APP"
fi
codesign --verify --strict "$APP"

cp hakai-mac/README.txt "$STAGE/README.txt"
cp LICENSE "$STAGE/LICENSE.txt"
cp CREDITS.md "$STAGE/"
ln -s /Applications "$STAGE/Applications"

# hdiutil intermittently fails with "Resource busy" on CI runners — retry a few times.
for attempt in 1 2 3; do
    hdiutil create -volname "Hakai" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG" >/dev/null && break
    [ "$attempt" = 3 ] && { echo "hdiutil create failed" >&2; exit 1; }
    sleep 5
done
if [ "$IDENTITY" != "-" ]; then
    codesign --force --sign "$IDENTITY" --timestamp "$DMG"
    if [ -n "${NOTARY_PROFILE:-}" ]; then
        xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
        xcrun stapler staple "$DMG"
    fi
fi
rm -rf "$STAGE"

echo "-> $DMG  ($(du -h "$DMG" | cut -f1))"
