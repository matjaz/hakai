# hakai-core

The shared library: damage layer, procedural decals, icons and sprites, all nine tools,
the termite colony, particles, and the HUD. Headless by default — no window, no audio,
no compositor — so it builds and tests on any OS.

The game itself is described in the [main README](../README.md). This crate is what the
platform binaries draw with.

```bash
cd hakai-core
cargo test
```

`cargo run --example simulate` drives all nine tools headlessly and checks the
interaction rules. `cargo run --example gen_credits -- --check` confirms `CREDITS.md`
matches the bundled fonts and sounds.

## Features

Off by default, so `cargo test` stays light:

| Feature | What it adds |
|---|---|
| `render` | wgpu renderer, HUD text, and the per-output scene |
| `playback` | cpal audio engine and the 35 bundled sounds |
| `shell` | winit window, event loop, and input (implies `render`) |

Windows, macOS, and the Linux GNOME/X11 fallback are a `Platform` impl on `shell`. The
Linux binary keeps its own Wayland event loop for the layer-shell overlay and uses
`shell` for the fallback. All three binaries build against wgpu 30: the Windows side
needed 27+ so a per-pixel-alpha surface can present through DirectComposition, and Linux
and macOS followed. The renderer lives in `hakai_core::render`.

Not a formal Cargo workspace, deliberately. Each binary depends on this crate via a
plain relative path. Each platform crate has its own `PHASE*.md` walkthrough;
`PHASE8.md` at the repo root covers packaging.

## The other crates

```
hakai-linux/    the Linux binary — see its README
hakai-win/      the Windows binary
hakai-mac/      the macOS binary
hakai-android/  the Android library
packaging/      PKGBUILD, .desktop entry, Hyprland keybind snippet
```

`.cargo/config.toml` sends build output to `/home/matjaz/.cache/hakai-core-target`.
Override with `CARGO_TARGET_DIR`.

## Releases

CI builds and packages every platform on each push and keeps the artifacts for two
weeks. Pushing a `v*` tag publishes those same artifacts (plus `SHA256SUMS`) as a GitHub
release, without rebuilding. The tag has to match the version in `hakai-linux`,
`hakai-win`, `hakai-mac`, and `hakai-android` (`v1.3.0` ↔ `1.3.0`), or the release job
refuses.

```sh
git tag v1.3.0 && git push origin v1.3.0
```

How each platform is wrapped: [Linux](../hakai-linux/README.md),
[Windows](../hakai-win/README.txt), [macOS](../hakai-mac/README.txt),
[Android](../hakai-android/README.md). The workflow is
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml).
