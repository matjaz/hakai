# Hakai for Linux

The `hakai` command: a native overlay on Wayland, with a fullscreen-window fallback on
GNOME and X11. Same nine tools as everywhere else — see the
[main README](../README.md) for what the game does, and the
[releases page](https://github.com/matjaz/hakai/releases) to download it.

## Install

Runs on any Wayland or X11 desktop, two ways:

- **Native overlay** — a `wlr-layer-shell` surface with an exclusive keyboard grab, on
  Hyprland (the one it's built and tested on), Sway, river, labwc, Wayfire, niri, KDE
  Plasma 6 and COSMIC. The brightness-driven impact sound reads the screen with
  `wlr-screencopy` where there is one (wlroots compositors, Hyprland, niri). On Hyprland
  the compositor's own `SUPER` binds still work underneath the grab.
- **Fallback** — on GNOME (no layer-shell) and in X11 sessions, hakai opens ordinary
  fullscreen transparent windows instead, one per monitor. Alt+Tab and the Super key
  still reach the desktop, the impact sound picks a random variant, and on X11
  transparency needs a compositing window manager (every mainstream desktop has one;
  bare i3 without picom shows black).

It picks automatically. `HAKAI_BACKEND=winit` forces the fallback, and `WGPU_BACKEND=gl`
or `vulkan` pins the graphics backend if a driver misbehaves (the fallback uses Vulkan on
Wayland and GL on X11 by default).

One binary per architecture serves every distro — it's built on Ubuntu 22.04, so it needs
glibc 2.35+ (Ubuntu 22.04+, Debian 12+, Fedora 36+). From the
[releases page](https://github.com/matjaz/hakai/releases):

```bash
sudo apt install ./hakai_<version>-1_amd64.deb        # Debian, Ubuntu, Mint, …
sudo dnf install ./hakai-<version>-1.x86_64.rpm       # Fedora, openSUSE (zypper), …
```

(`arm64`/`aarch64` for ARM.) Or the tarball, `hakai-<version>-linux-<arch>.tar.gz` —
the binary, a `.desktop` entry and its icon: `install -Dm755 hakai ~/.local/bin/hakai`,
plus `hakai.desktop` → `~/.local/share/applications/` and `hakai.png` →
`~/.local/share/icons/hicolor/256x256/apps/` for a launcher entry. Either way it needs
`libxkbcommon`, ALSA's `libasound`, `libwayland-client` and a Vulkan driver at runtime (plus
EGL for the X11 fallback) — the packages pull them in.

On Arch / Omarchy, it's not yet on the AUR (registration is currently locked down
repo-wide after a wave of malicious package uploads in mid-2026 — nothing to do with this
package specifically; submission will follow once that reopens). Until then, install
directly with `makepkg`, using the `PKGBUILD` already in this repo — confirmed working
end to end on real QEMU/aarch64 Omarchy hardware:

```bash
git clone https://github.com/matjaz/hakai.git
cd hakai/packaging
makepkg -si
```

This builds the Linux binary and `hakai-core`, generates the app icon procedurally (no
bundled bitmap assets — the `app_icon` example in
[`hakai-core`](../hakai-core/README.md)), and installs the binary, `.desktop` entry,
icon, license, and docs via `pacman`, so it's a real tracked package
(`pacman -Q hakai-git`), not a stray binary. `pkgrel`, dependencies, and install paths
are all in [`packaging/PKGBUILD`](../packaging/PKGBUILD).

If `makepkg` is invoked from a network-mounted checkout of this repo (e.g. developing
over a shared/virtiofs mount), set `BUILDDIR` somewhere local first — network mounts have
caused real build failures here ([`PHASE0.md`](PHASE0.md), [`PHASE8.md`](../PHASE8.md)):

```bash
BUILDDIR="$HOME/.cache/hakai-git-build" makepkg -si
```

### Hyprland keybind

Not applied automatically — see
[`packaging/hyprland-bindings.conf.example`](../packaging/hyprland-bindings.conf.example)
for both the current Lua form and the classic hyprlang one, depending on your Hyprland
version. On a current Omarchy install (Hyprland 0.55+, Lua config), add this to
`~/.config/hypr/bindings.lua` — not `bindings.conf`, which nothing loads once you're on
Lua — then `hyprctl reload`:

```lua
o.bind("SUPER + SHIFT + H", "Destroy the desktop", "uwsm app -- hakai")
o.bind("SUPER + SHIFT + ALT + H", "Stop destroying it", "pkill -x hakai")
```

## Omarchy

The color-thrower's palette and the whole HUD (status bar, tool palette, credits panel)
read their colours from your active Omarchy theme (`omarchy-theme-color`), falling back
to a built-in palette cleanly if that's not available — read once at startup, so
`omarchy-theme-set` while `hakai` is already running needs a restart to take effect.

## Building

```bash
cd hakai-linux
cargo build --release
cargo run --release
```

`hakai-linux/package.sh` builds the release binary and wraps it as a tarball, `.deb`, and
`.rpm` (the last two need [nfpm](https://nfpm.goreleaser.com)). The headless core tests
on any OS — see [`hakai-core`](../hakai-core/README.md).

Wayland, xkbcommon, ALSA, and Vulkan development headers are required to compile
(`libwayland-dev`, `libxkbcommon-dev`, `libasound2-dev`, `libvulkan-dev`, `pkg-config`
on Debian/Ubuntu). The binary links `libxkbcommon` and `libasound` directly;
`libwayland-client`, the Vulkan loader, and (for the GNOME/X11 fallback) the X11
libraries and EGL are dlopen'd at runtime.

`.cargo/config.toml` sends build output to `/home/matjaz/.cache/hakai-target` so a
shared or network checkout doesn't hold the `target/` directory. Override with
`CARGO_TARGET_DIR`, which is what `package.sh` does.

## Status

Core development (rendering, input, audio, screen capture, fractional scaling, Omarchy
theming) is done and confirmed working on real Hyprland hardware. Packaging is mostly
done — `PKGBUILD`, CI, and a real `makepkg -si` install are all confirmed working — with
AUR submission itself on hold until the AUR's own registration lockdown lifts. See
`PHASE8.md` at the repo root for the packaging writeup.

The GNOME/X11 fallback is tested in headless sessions (Weston without layer-shell,
Xvfb + openbox; Mesa's software renderer) — window, fullscreen, tools, HUD and input all
work there. It hasn't yet been run on a real GNOME or X11 desktop with a GPU.

## Releases

CI builds `.deb`, `.rpm`, and a tarball for x86_64 and aarch64 (one build each, wrapped
three ways by nfpm) and keeps them as workflow artifacts for two weeks. A `v*` tag that
matches this crate's `version` (`v1.3.0` ↔ `1.3.0` in `Cargo.toml`, and the same version
in `hakai-win`, `hakai-mac`, and `hakai-android`) publishes those artifacts, plus
`SHA256SUMS`, as a GitHub release without rebuilding. See
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml).
