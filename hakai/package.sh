#!/usr/bin/env bash
#
# Builds hakai (the Linux binary) and assembles the release tarball.
#
#   hakai/package.sh
#
# Output: hakai/target/hakai-<version>-<arch>-linux.tar.gz — the stripped binary (fonts
# and all 35 sounds are compiled in), hakai.desktop + a 256 px icon, the Hyprland keybind
# example, README.md, LICENSE and CREDITS.md. Arch users can still build the -git package
# from packaging/PKGBUILD instead.
#
# Run from anywhere; cargo is invoked from the repo root, so hakai/.cargo/config.toml's
# dev-machine target-dir override doesn't apply — output lands in hakai/target, or in
# $CARGO_TARGET_DIR when set (e.g. to keep the build off a shared mount).

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

MANIFEST=hakai/Cargo.toml
TARGET_DIR="${CARGO_TARGET_DIR:-hakai/target}"
VERSION="$(sed -nE 's/^version *= *"([^"]+)".*/\1/p' "$MANIFEST" | head -1)"
[ -n "$VERSION" ] || { echo "couldn't read version from $MANIFEST" >&2; exit 1; }
NAME="hakai-$VERSION-$(uname -m)-linux"
STAGE="$TARGET_DIR/$NAME"
TARBALL="$TARGET_DIR/$NAME.tar.gz"

cargo build --release --locked --manifest-path "$MANIFEST" --target-dir "$TARGET_DIR"

rm -rf "$STAGE" "$TARBALL"
mkdir -p "$STAGE"
cargo run --release --locked --manifest-path "$MANIFEST" --target-dir "$TARGET_DIR" \
    --example app_icon -- "$STAGE/hakai.png" 256

cp "$TARGET_DIR/release/hakai" "$STAGE/"
cp packaging/hakai.desktop packaging/hyprland-bindings.conf.example "$STAGE/"
cp README.md LICENSE CREDITS.md "$STAGE/"

tar -C "$TARGET_DIR" -czf "$TARBALL" "$NAME"
rm -rf "$STAGE"

echo "-> $TARBALL  ($(du -h "$TARBALL" | cut -f1))"
