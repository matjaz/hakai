# Hakai for Android

A translucent activity: whatever it's launched over — the home screen, or another app —
stays visible behind it, and that's what you smash. Same nine tools as the desktop
builds — see the [main README](../README.md) for the game, and the
[releases page](https://github.com/matjaz/hakai/releases) for the `.apk` (arm64,
Android 8+).

`hakai-android` is a `cdylib` loaded by Android's `NativeActivity` (android-activity
glue) on the same `hakai_core::shell` as Windows and macOS. It renders with Vulkan
(GLES as the fallback) and plays sound through AAudio. The only Java is the Quick
Settings tile (`java/`).

Phones can't overlay other apps the way a desktop window can, but they don't need to
here. No special permission. It runs in either orientation — held upright, the tool
palette folds into rows — and touch replaces the keyboard:

- drag to use the current tool,
- swipe left/right with **two fingers** anywhere — or with one along the status bar — to
  switch to the next/previous tool (one finger stays free for drawing),
- tap the status bar for the tool palette, tap a tool to pick it,
- long-press the status bar for the credits, tap anywhere to close a panel,
- Back quits; so does leaving the app.

To smash another app rather than the home screen, start hakai from that app: add the
Hakai tile to Quick Settings once (pull the shade down twice, edit, drag *Hakai* in),
then in any app pull the shade down and tap it — hakai opens straight over that app.

## Install

Install the `.apk` from the [releases page](https://github.com/matjaz/hakai/releases)
(allow installs from your browser or file manager when Android asks).

## Building

Needs the Android SDK + NDK and a JDK — no Gradle. [`package.sh`](package.sh) assembles
the APK with the SDK's own tools (its header lists them), because cargo-apk can't
include the Quick Settings tile's Java:

```sh
rustup target add aarch64-linux-android
hakai-android/package.sh                     # → hakai-android/target/hakai-<ver>-android-arm64.apk
```

Releases are signed with one long-lived key so each APK installs over the last —
[`SIGNING.md`](SIGNING.md) covers making it and giving it to CI.

## Status

The Android build produces a signed arm64 APK, but hasn't yet run on a device — whether
the translucent window really shows the app or home screen behind it on every phone (and which
Vulkan/GLES drivers get the alpha right) is the open question.

## Releases

CI builds the arm64 APK on each push and keeps it as a workflow artifact for two weeks.
A `v*` tag that matches this crate's `version` (and the same version in `hakai-linux`,
`hakai-win`, and `hakai-mac`) publishes it. See
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml).
