#!/usr/bin/env bash
#
# Builds hakai-android and assembles the release APK.
#
#   hakai-android/package.sh
#
# Output: hakai-android/target/hakai-<version>-android-arm64.apk — one arm64-v8a APK
# (every phone of the last decade); fonts and all 35 sounds are compiled in.
#
# Needs: the Android SDK (ANDROID_HOME, with a platform ≤ the NDK's max — android-35 for
# NDK r29 — and build-tools), the NDK (ANDROID_NDK_ROOT), a JDK (JAVA_HOME, for keytool
# and apksigner), the aarch64-linux-android Rust target and cargo-apk.
#
# Signing: with ANDROID_KEYSTORE (+ ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS) the APK
# is signed with that key — keep it, Android only installs updates signed by the same
# key. Without it cargo-apk signs with the local debug key, fine for testing.

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

MANIFEST=hakai-android/Cargo.toml
TARGET_DIR=hakai-android/target
VERSION="$(sed -nE 's/^version *= *"([^"]+)".*/\1/p' "$MANIFEST" | head -1)"
[ -n "$VERSION" ] || { echo "couldn't read version from $MANIFEST" >&2; exit 1; }
APK="$TARGET_DIR/hakai-$VERSION-android-arm64.apk"

# Launcher icons, one per density, rendered procedurally (hakai ships no bitmap assets).
RES=hakai-android/res
rm -rf "$RES"
for pair in mdpi:48 hdpi:72 xhdpi:96 xxhdpi:144 xxxhdpi:192; do
    mkdir -p "$RES/mipmap-${pair%%:*}"
    cargo run -q --release --locked --manifest-path hakai-core/Cargo.toml --example app_icon -- \
        "$RES/mipmap-${pair%%:*}/ic_launcher.png" "${pair##*:}" >/dev/null
done

if [ -n "${ANDROID_KEYSTORE:-}" ]; then
    export CARGO_APK_RELEASE_KEYSTORE="$ANDROID_KEYSTORE"
    export CARGO_APK_RELEASE_KEYSTORE_PASSWORD="${ANDROID_KEYSTORE_PASSWORD:?}"
else
    # The standard Android debug key (what Android Studio uses too), made on first use.
    DEBUG_KS="$HOME/.android/debug.keystore"
    if [ ! -f "$DEBUG_KS" ]; then
        mkdir -p "$HOME/.android"
        keytool -genkeypair -keystore "$DEBUG_KS" -storepass android -keypass android \
            -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 \
            -dname "CN=Android Debug,O=Android,C=US" >/dev/null
    fi
    echo "note: no ANDROID_KEYSTORE — signing with the debug key ($DEBUG_KS)"
    export CARGO_APK_RELEASE_KEYSTORE="$DEBUG_KS"
    export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android
fi

(cd hakai-android && cargo apk build --release --lib)

rm -f "$APK"
cp "$TARGET_DIR/release/apk/hakai.apk" "$APK"
echo "-> $APK  ($(du -h "$APK" | cut -f1))"
