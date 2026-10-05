#!/usr/bin/env bash
#
# Builds hakai-linux (the `hakai` command) and assembles the release tarball.
#
#   hakai-linux/package.sh
#
# Output, all from one cargo build:
#
#   hakai-linux/target/hakai-<version>-linux-<arch>.tar.gz — the stripped binary (fonts and all
#       35 sounds are compiled in), hakai.desktop + a 256 px icon, the Hyprland keybind
#       example, README.md, LINUX.md, LICENSE and CREDITS.md
#   hakai-linux/target/hakai_<version>-1_<amd64|arm64>.deb  — Debian, Ubuntu, Mint, …
#   hakai-linux/target/hakai-<version>-1.<x86_64|aarch64>.rpm — Fedora, openSUSE, …
#
# The .deb/.rpm are the same binary wrapped by nfpm (https://nfpm.goreleaser.com) — only
# the format and the distro's dependency names differ. They're skipped, with a note, when
# nfpm isn't on PATH; CI installs it. Arch users can still build the -git package from
# packaging/PKGBUILD instead.
#
# One binary serves every distro: glibc is backwards-compatible, so building on an old
# base (CI uses Ubuntu 22.04, glibc 2.35) is what sets the floor. The binary links only
# libxkbcommon and libasound directly; libwayland-client, the Vulkan loader and (for the
# GNOME/X11 fallback) the X11 libraries and EGL are dlopen'd at runtime.
#
# Run from anywhere; cargo is invoked from the repo root, so hakai-linux/.cargo/config.toml's
# dev-machine target-dir override doesn't apply — output lands in hakai-linux/target, or in
# $CARGO_TARGET_DIR when set (e.g. to keep the build off a shared mount).

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

MANIFEST=hakai-linux/Cargo.toml
TARGET_DIR="${CARGO_TARGET_DIR:-hakai-linux/target}"
VERSION="$(sed -nE 's/^version *= *"([^"]+)".*/\1/p' "$MANIFEST" | head -1)"
[ -n "$VERSION" ] || { echo "couldn't read version from $MANIFEST" >&2; exit 1; }
NAME="hakai-$VERSION-linux-$(uname -m)"
STAGE="$TARGET_DIR/$NAME"
TARBALL="$TARGET_DIR/$NAME.tar.gz"

cargo build --release --locked --manifest-path "$MANIFEST" --target-dir "$TARGET_DIR"

rm -rf "$STAGE" "$TARBALL" "$TARGET_DIR"/hakai_*.deb "$TARGET_DIR"/hakai-*.rpm
mkdir -p "$STAGE"
cargo run --release --locked --manifest-path "$MANIFEST" --target-dir "$TARGET_DIR" \
    --example app_icon -- "$STAGE/hakai.png" 256

cp "$TARGET_DIR/release/hakai" "$STAGE/"
cp packaging/hakai.desktop packaging/hyprland-bindings.conf.example "$STAGE/"
cp README.md LICENSE CREDITS.md "$STAGE/"
cp hakai-linux/README.md "$STAGE/LINUX.md"

tar -C "$TARGET_DIR" -czf "$TARBALL" "$NAME"
echo "-> $TARBALL  ($(du -h "$TARBALL" | cut -f1))"

if command -v nfpm >/dev/null; then
    case "$(uname -m)" in
        x86_64)  GOARCH=amd64 ;;
        aarch64) GOARCH=arm64 ;;
        *) echo "no nfpm arch mapping for $(uname -m)" >&2; exit 1 ;;
    esac
    # Generated rather than checked in, so the paths and version need no templating.
    cat > "$STAGE/nfpm.yaml" <<YAML
name: hakai
arch: $GOARCH
platform: linux
version: $VERSION
release: 1
section: games
priority: optional
maintainer: Matjaz Lipus
vendor: Hakai
homepage: https://github.com/matjaz/hakai
license: MIT
description: |-
  Smash, burn and repaint your live desktop.
  A transparent overlay with nine tools (hammer, chain-saw, machine gun,
  flame-thrower, paint, phaser, stamps, termites, washer) over your real
  desktop. Runs on any Wayland or X11 desktop: a native layer-shell overlay on
  Hyprland, Sway, KDE Plasma, river, labwc, Wayfire, niri and COSMIC, and
  fullscreen transparent windows on GNOME and X11.
contents:
  - src: $STAGE/hakai
    dst: /usr/bin/hakai
    file_info: { mode: 0755 }
  - src: $STAGE/hakai.desktop
    dst: /usr/share/applications/hakai.desktop
  - src: $STAGE/hakai.png
    dst: /usr/share/icons/hicolor/256x256/apps/hakai.png
  - src: $STAGE/README.md
    dst: /usr/share/doc/hakai/README.md
  - src: $STAGE/LINUX.md
    dst: /usr/share/doc/hakai/LINUX.md
  - src: $STAGE/CREDITS.md
    dst: /usr/share/doc/hakai/CREDITS.md
  - src: $STAGE/hyprland-bindings.conf.example
    dst: /usr/share/doc/hakai/hyprland-bindings.conf.example
  - src: $STAGE/LICENSE
    dst: /usr/share/licenses/hakai/LICENSE
overrides:
  deb:
    depends:
      - libc6 (>= 2.35)
      - libxkbcommon0
      - libasound2t64 | libasound2
      - libwayland-client0
      - libvulkan1
    recommends:
      - mesa-vulkan-drivers
      # The GNOME/X11 fallback: EGL for wgpu's GL backend on X11, xkbcommon's X11 glue.
      - libegl1
      - libxkbcommon-x11-0
  rpm:
    depends:
      - libxkbcommon
      - alsa-lib
      - libwayland-client
      - vulkan-loader
    recommends:
      - mesa-vulkan-drivers
      - libglvnd-egl
      - libxkbcommon-x11
YAML
    for fmt in deb rpm; do
        nfpm package --config "$STAGE/nfpm.yaml" --packager "$fmt" --target "$TARGET_DIR/" >/dev/null
    done
    for f in "$TARGET_DIR"/hakai_"$VERSION"-1_"$GOARCH".deb "$TARGET_DIR"/hakai-"$VERSION"-1.*.rpm; do
        echo "-> $f  ($(du -h "$f" | cut -f1))"
    done
else
    echo "note: nfpm not found — skipping .deb/.rpm (https://nfpm.goreleaser.com/install/)"
fi

rm -rf "$STAGE"
