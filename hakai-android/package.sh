#!/usr/bin/env bash
#
# Builds hakai-android and assembles the release APK.
#
#   hakai-android/package.sh
#
# Output: hakai-android/target/hakai-<version>-android-arm64.apk — one arm64-v8a APK
# (every phone of the last decade); fonts and all 35 sounds are compiled in.
#
# The APK is put together by hand from the SDK's own tools rather than by cargo-apk, which
# can't include Java code — and the Quick Settings tile (java/) is Java:
#
#   cargo (NDK clang as linker)  → libhakai.so
#   javac + d8                   → classes.dex   (HakaiTileService)
#   aapt2 compile + link         → manifest, launcher icons, tile icon
#   zip, zipalign, apksigner     → the signed APK
#
# Needs: the Android SDK (ANDROID_HOME: build-tools, and platform android-35), the NDK
# (ANDROID_NDK_ROOT), a JDK (javac, keytool on PATH) and the aarch64-linux-android Rust
# target.
#
# Signing: with ANDROID_KEYSTORE (+ ANDROID_KEYSTORE_PASSWORD) the APK is signed with that
# key — keep it, Android only installs updates signed by the same key. Without it the
# local debug key is used, fine for testing. Making the release key and handing it to CI:
# hakai-android/SIGNING.md.

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

CRATE=hakai-android
MANIFEST=$CRATE/Cargo.toml
TARGET_DIR=$CRATE/target
TRIPLE=aarch64-linux-android
MIN_SDK=26        # AAudio, cpal's Android backend
TARGET_SDK=35     # the newest platform NDK r27–r29 accept
VERSION="$(sed -nE 's/^version *= *"([^"]+)".*/\1/p' "$MANIFEST" | head -1)"
[ -n "$VERSION" ] || { echo "couldn't read version from $MANIFEST" >&2; exit 1; }
# cargo-apk's formula (which the 1.3.0 APK shipped with), kept so updates keep installing:
# 1 << 24 | major << 16 | minor << 8 | patch.
IFS=. read -r MAJOR MINOR PATCH <<<"${VERSION%%[-+]*}"
VERSION_CODE=$(( (1 << 24) | (MAJOR << 16) | (MINOR << 8) | PATCH ))
APK="$TARGET_DIR/hakai-$VERSION-android-arm64.apk"
BUILD="$TARGET_DIR/apk-build"

: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
: "${ANDROID_NDK_ROOT:?set ANDROID_NDK_ROOT to the NDK}"
BUILD_TOOLS="$(ls -d "$ANDROID_HOME"/build-tools/* | sort -V | tail -1)"
ANDROID_JAR="$ANDROID_HOME/platforms/android-$TARGET_SDK/android.jar"
[ -f "$ANDROID_JAR" ] || { echo "missing $ANDROID_JAR — sdkmanager \"platforms;android-$TARGET_SDK\"" >&2; exit 1; }

# --- 1. libhakai.so ---------------------------------------------------------------------

case "$(uname -s)" in
    Darwin) HOST_TAG=darwin-x86_64 ;;   # the NDK's macOS toolchain is universal, named this
    Linux)  HOST_TAG=linux-x86_64 ;;
    *) echo "unsupported build host $(uname -s)" >&2; exit 1 ;;
esac
NDK_BIN="$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/$HOST_TAG/bin"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$NDK_BIN/$TRIPLE$MIN_SDK-clang"
export CC_aarch64_linux_android="$NDK_BIN/$TRIPLE$MIN_SDK-clang"
export AR_aarch64_linux_android="$NDK_BIN/llvm-ar"
cargo build --release --locked --manifest-path "$MANIFEST" --target "$TRIPLE"
LIB="$TARGET_DIR/$TRIPLE/release/libhakai.so"

# --- 2. classes.dex ---------------------------------------------------------------------

rm -rf "$BUILD" && mkdir -p "$BUILD/classes" "$BUILD/dex" "$BUILD/res"
javac -source 11 -target 11 -nowarn -Xlint:-options -classpath "$ANDROID_JAR" -d "$BUILD/classes" \
    $(find "$CRATE/java" -name '*.java')
"$BUILD_TOOLS/d8" --release --min-api "$MIN_SDK" --lib "$ANDROID_JAR" --output "$BUILD/dex" \
    $(find "$BUILD/classes" -name '*.class')

# --- 3. resources + manifest ------------------------------------------------------------

# Launcher icons, one per density, rendered procedurally (hakai ships no bitmap assets);
# the tile icon is a checked-in vector.
RES="$BUILD/res-src"
cp -R "$CRATE/res-static" "$RES"
for pair in mdpi:48 hdpi:72 xhdpi:96 xxhdpi:144 xxxhdpi:192; do
    mkdir -p "$RES/mipmap-${pair%%:*}"
    cargo run -q --release --locked --manifest-path hakai-core/Cargo.toml --example app_icon -- \
        "$RES/mipmap-${pair%%:*}/ic_launcher.png" "${pair##*:}" >/dev/null
done
"$BUILD_TOOLS/aapt2" compile --dir "$RES" -o "$BUILD/res/res.zip"
"$BUILD_TOOLS/aapt2" link -o "$BUILD/unsigned.apk" -I "$ANDROID_JAR" \
    --manifest "$CRATE/AndroidManifest.xml" \
    --min-sdk-version "$MIN_SDK" --target-sdk-version "$TARGET_SDK" \
    --version-code "$VERSION_CODE" --version-name "$VERSION" \
    "$BUILD/res/res.zip"

# --- 4. assemble, align, sign -----------------------------------------------------------

mkdir -p "$BUILD/apk/lib/arm64-v8a"
cp "$BUILD/dex/classes.dex" "$BUILD/apk/"
cp "$LIB" "$BUILD/apk/lib/arm64-v8a/"
(cd "$BUILD/apk" && zip -qr ../unsigned.apk classes.dex lib)
"$BUILD_TOOLS/zipalign" -f 4 "$BUILD/unsigned.apk" "$BUILD/aligned.apk"

if [ -n "${ANDROID_KEYSTORE:-}" ]; then
    KEYSTORE="$ANDROID_KEYSTORE"
    : "${ANDROID_KEYSTORE_PASSWORD:?set ANDROID_KEYSTORE_PASSWORD for $ANDROID_KEYSTORE}"
    export HAKAI_KS_PASS="$ANDROID_KEYSTORE_PASSWORD"
else
    # The standard Android debug key (what Android Studio uses too), made on first use.
    KEYSTORE="$HOME/.android/debug.keystore"
    if [ ! -f "$KEYSTORE" ]; then
        mkdir -p "$HOME/.android"
        keytool -genkeypair -keystore "$KEYSTORE" -storepass android -keypass android \
            -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 \
            -dname "CN=Android Debug,O=Android,C=US" >/dev/null
    fi
    echo "note: no ANDROID_KEYSTORE — signing with the debug key ($KEYSTORE)"
    export HAKAI_KS_PASS=android
fi
rm -f "$APK"
"$BUILD_TOOLS/apksigner" sign --ks "$KEYSTORE" --ks-pass env:HAKAI_KS_PASS --key-pass env:HAKAI_KS_PASS \
    --out "$APK" "$BUILD/aligned.apk"
"$BUILD_TOOLS/apksigner" verify "$APK"

echo "-> $APK  ($(du -h "$APK" | cut -f1))"
