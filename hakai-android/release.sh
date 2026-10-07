#!/usr/bin/env bash
#
# Builds a release-signed APK on this machine — the same as CI does with the
# ANDROID_KEYSTORE_* secrets, for testing a release build on a phone before tagging.
#
#   hakai-android/release.sh                       # build, signed with ~/hakai-release.keystore
#   hakai-android/release.sh --install             # …and install it on the connected phone
#   hakai-android/release.sh --save-password       # remember the password in the login Keychain
#   hakai-android/release.sh --keystore <path>     # a keystore somewhere else
#
# The password comes from the macOS login Keychain (item "hakai-android-keystore", saved
# by --save-password), else $HAKAI_KEYSTORE_PASSWORD, else it's asked for — never on the
# command line, so it stays out of shell history and `ps`.
#
# Making the keystore in the first place: hakai-android/SIGNING.md.

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KEYSTORE="$HOME/hakai-release.keystore"
KEYCHAIN_ITEM=hakai-android-keystore
INSTALL=0
SAVE=0

while [ $# -gt 0 ]; do
    case "$1" in
        --install) INSTALL=1 ;;
        --save-password) SAVE=1 ;;
        --keystore) KEYSTORE="${2:?--keystore needs a path}"; shift ;;
        -h|--help) sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1 (see --help)" >&2; exit 1 ;;
    esac
    shift
done

[ -f "$KEYSTORE" ] || { echo "no keystore at $KEYSTORE — see hakai-android/SIGNING.md, or pass --keystore" >&2; exit 1; }

# --- toolchain: whatever's already set, else this Mac's usual places --------------------

if [ -z "${JAVA_HOME:-}" ] && [ -d /opt/homebrew/opt/openjdk@17 ]; then
    export JAVA_HOME=/opt/homebrew/opt/openjdk@17
fi
[ -n "${JAVA_HOME:-}" ] && export PATH="$JAVA_HOME/bin:$PATH"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
if [ -z "${ANDROID_NDK_ROOT:-}" ]; then
    ANDROID_NDK_ROOT="$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)"
    export ANDROID_NDK_ROOT
fi
[ -d "$ANDROID_HOME" ] || { echo "no Android SDK at $ANDROID_HOME (set ANDROID_HOME)" >&2; exit 1; }
[ -n "$ANDROID_NDK_ROOT" ] || { echo "no NDK under $ANDROID_HOME/ndk (set ANDROID_NDK_ROOT)" >&2; exit 1; }
command -v keytool >/dev/null || { echo "no JDK on PATH (set JAVA_HOME)" >&2; exit 1; }

# --- the password ------------------------------------------------------------------------

PASSWORD=""
if command -v security >/dev/null; then
    PASSWORD="$(security find-generic-password -a "$USER" -s "$KEYCHAIN_ITEM" -w 2>/dev/null || true)"
fi
PASSWORD="${PASSWORD:-${HAKAI_KEYSTORE_PASSWORD:-}}"
if [ -z "$PASSWORD" ]; then
    read -r -s -p "Password for $KEYSTORE: " PASSWORD
    echo
fi
# Fail early on a wrong password rather than after a full build.
if ! KS_PASS="$PASSWORD" keytool -list -keystore "$KEYSTORE" -storepass:env KS_PASS >/dev/null 2>&1; then
    echo "wrong password for $KEYSTORE" >&2
    exit 1
fi
if [ "$SAVE" = 1 ]; then
    # A bare -w makes `security` prompt for the value itself, so the password never
    # becomes a command-line argument. -U replaces an existing item.
    echo "Saving to the login Keychain as \"$KEYCHAIN_ITEM\" — enter the same password again:"
    security add-generic-password -U -a "$USER" -s "$KEYCHAIN_ITEM" -w
fi

# --- build -------------------------------------------------------------------------------

ANDROID_KEYSTORE="$KEYSTORE" ANDROID_KEYSTORE_PASSWORD="$PASSWORD" "$REPO/hakai-android/package.sh"

VERSION="$(sed -nE 's/^version *= *"([^"]+)".*/\1/p' "$REPO/hakai-android/Cargo.toml" | head -1)"
APK="$REPO/hakai-android/target/hakai-$VERSION-android-arm64.apk"
BUILD_TOOLS="$(ls -d "$ANDROID_HOME"/build-tools/* | sort -V | tail -1)"
echo
"$BUILD_TOOLS/apksigner" verify --print-certs "$APK" | grep -E "DN:|SHA-256"
echo "(compare with: keytool -list -v -keystore $KEYSTORE | grep SHA256)"

# --- install -----------------------------------------------------------------------------

if [ "$INSTALL" = 1 ]; then
    ADB="$ANDROID_HOME/platform-tools/adb"
    if ! "$ADB" get-state >/dev/null 2>&1; then
        echo "no phone connected (USB debugging on, and this computer allowed?)" >&2
        exit 1
    fi
    # -r: replace an installed copy — works when it was signed with this same key.
    if ! "$ADB" install -r "$APK"; then
        echo "install failed — if the installed hakai was signed with a different (debug) key," >&2
        echo "uninstall it once: $ADB uninstall si.lipus.hakai" >&2
        exit 1
    fi
fi
