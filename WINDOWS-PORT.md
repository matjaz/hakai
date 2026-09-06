# Hakai on Windows

Companion to `OMARCHY-PORT.md`. That document planned the move off macOS; this one plans
the move off Wayland, and it is a much smaller document because the expensive half of the
work is already done and already shipping.

## What this port actually is

Not a port. **A second binary crate against an unchanged core.**

The Linux port was a reimplementation of roughly half the Swift codebase. This one is not,
because the split it introduced — a platform-neutral `hakai-core` and a thin platform
binary — turns out to fall almost exactly on the Linux/Windows seam too. Measured against
the current tree (11,844 lines of Rust):

| | lines | on Windows |
|---|---:|---|
| `hakai-core/src` — nine tools, decals, icons, sprites, damage layer, colony, particles, HUD layout, credits, RNG, geometry, fonts, simulation | 7,566 | **unchanged** |
| `hakai-core/examples` — asset dumps, credits generator, simulation | 442 | **unchanged** |
| `hakai/src/main.rs` — wgpu pipelines, tile/sprite/HUD drawing | 998 | **unchanged** |
| `hakai/src/audio.rs` — `cpal` voice pool | 442 | **unchanged** |
| `hakai/src/text.rs` — `cosmic-text` HUD shaping | 144 | **unchanged** |
| `hakai/src/main.rs` — `State`: scene, tools, input decisions | 790 | input plumbing reattached |
| `hakai/src/capture.rs` — `BrightnessMap` | 128 | frame acquisition swapped, logic kept |
| `hakai/src/theme.rs` — Omarchy theme | 145 | `#[cfg]`-out, falls back to the built-in palette |
| `hakai/src/main.rs` — `main()` setup | 283 | mostly rewritten |
| `hakai/src/main.rs` — Wayland protocol `impl`s | 909 | **deleted, rewritten as Win32** |

**~9,600 lines — 80% of the codebase — cross the boundary untouched.** `hakai-core`'s
entire dependency set is `tiny-skia`, `ttf-parser`, `serde`, `serde_json`: four pure-Rust
crates with no OS surface at all. Nothing in it has ever known which compositor it was
running under.

Of `hakai`'s own dependencies, only seven are Linux-bound —
`smithay-client-toolkit`, `wayland-client`, `wayland-backend`, `calloop`,
`calloop-wayland-source`, `wayland-protocols`, `wayland-protocols-wlr`. Everything else
(`wgpu`, `cpal`, `cosmic-text`, `hound`, `tiny-skia`, `bytemuck`, `raw-window-handle`,
`pollster`, `log`, `env_logger`) is already cross-platform and already picks a Windows
backend by itself: `wgpu` → D3D12, `cpal` → WASAPI.

> Not verified by compilation. The measurements above are from reading the tree; nothing
> here has been cross-compiled, because the machine this was written on has no Rust
> toolchain. **`cargo check --target x86_64-pc-windows-msvc -p hakai-core` is the first
> command to run**, and it should either pass outright or fail somewhere small.

## Target stack

| Concern | Linux/Omarchy (shipping) | Windows (planned) |
|---|---|---|
| Overlay window | `wlr-layer-shell-unstable-v1`, layer `overlay` | `winit` + `WS_EX_TOPMOST`, DWM per-pixel alpha |
| Event loop | `calloop` + `calloop-wayland-source` | `winit`'s own loop |
| Rendering | `wgpu` 22 on Vulkan | `wgpu` 22 on D3D12 — **same code** |
| 2D raster | `tiny-skia` | `tiny-skia` — **same code** |
| Text | `ttf-parser` (baked) + `cosmic-text` (HUD) | **same code** |
| Screen capture | `zwlr_screencopy_v1` → `wl_shm` | DXGI Desktop Duplication → staging texture |
| Audio | `cpal` on ALSA/PipeWire | `cpal` on WASAPI — **same code** |
| Input | `xkbcommon` keysyms via sctk | `winit` `KeyCode` / `MouseButton` |
| Per-output scale | `wp_fractional_scale_v1` + `wp_viewporter` | Per-Monitor DPI v2, handled by `winit` |
| Theme | `omarchy-theme-color` | built-in palette (or the registry accent colour) |
| Packaging | `PKGBUILD`, AUR | portable `.zip`, or MSIX/Inno + Authenticode |

## Port surface, subsystem by subsystem

| Subsystem | Lines | Verdict |
|---|---:|---|
| `hakai-core` in its entirety | 8,008 | **Verbatim.** Add `x86_64-pc-windows-msvc` to CI and stop thinking about it |
| wgpu pipelines and draw calls (`main.rs` 283–1280) | 998 | **Verbatim.** 13 Wayland mentions in 998 lines, all in surface configuration |
| `audio.rs` | 442 | **Verbatim.** Zero Wayland mentions; `cpal` changes host backend, not API |
| `text.rs` | 144 | **Verbatim.** Zero Wayland mentions |
| `State` (`main.rs` 1280–2069) | 790 | **Mechanical.** Scene, tool dispatch, palette hit-testing and mode switching are compositor-agnostic; only the calls *into* it are re-sourced |
| `capture.rs` | 128 | **Mechanical.** `BrightnessMap::update(bytes, width, height, stride)` already takes a raw strided buffer — Desktop Duplication hands over exactly that. Only the producer changes |
| `theme.rs` | 145 | **Delete or gate.** It shells out to `omarchy-theme-color` and already returns `Option`, so on Windows it degrades to the built-in palette on its own. Gate it behind `#[cfg(target_os = "linux")]` rather than leave a guaranteed-failing `Command::new` in the startup path |
| Keyboard/pointer handlers (inside `main.rs` 2069–2977) | ~200 | **Mechanical.** The dispatch table is a flat `match` on 20 keysyms; it becomes a flat `match` on 20 `winit::keyboard::KeyCode`s. The *decisions* — which key selects which tool, Shift+Tab cycling backwards — port verbatim |
| Layer shell, seat, output, shm, screencopy `Dispatch` impls | ~709 | **Rewrite.** This is the whole of the genuinely Linux-specific code |
| `main()` setup (`main.rs` 1–283) | 283 | **Rewrite.** Connection, globals, registry, one layer surface per output → `winit` window construction |

## The hard problems

### 1. A transparent window above everything — *the one that can move the estimate*

*Was: `wlr-layer-shell-unstable-v1`, layer `overlay`, `exclusive_zone = -1`.*

Windows has no equivalent primitive, and this is the single item that decides whether the
port takes eight days or twenty. There are three routes and they are not equally good:

1. **`winit` with `with_transparent(true)` + `WS_EX_TOPMOST`.** Transparency goes through
   DWM, `wgpu` renders into an ordinary HWND swapchain with an alpha-capable format. If
   this works it is a few hundred lines total and the rest of the plan holds.
2. **DirectComposition.** `DCompositionCreateDevice`, a composition swapchain with
   `DXGI_ALPHA_MODE_PREMULTIPLIED`, the window's visual tree built by hand. Correct and
   fast, but `wgpu` does not expose composition-swapchain creation, so this means dropping
   to raw DXGI/D3D12 and handing `wgpu` an external texture — or bypassing `wgpu` for
   presentation entirely. Costly.
3. **`WS_EX_LAYERED` + `UpdateLayeredWindow`.** True per-pixel alpha, universally supported,
   and a CPU blit of the whole framebuffer every frame. At 4K that is not going to hold
   60 fps. Fallback only.

**Prototype route 1 before anything else** — an empty transparent always-on-top window with
one `wgpu` triangle over the live desktop. Half a day, and it settles the plan.

Second-order behaviours, none of them fatal for a toy: the taskbar can surface above a
topmost window in some configurations; the UAC secure desktop hides it entirely;
exclusive-fullscreen applications take the display. There is no way to sit above *other*
topmost windows, so a pinned always-on-top window will occlude part of the canvas.

The escape hatch stays as good as it is on Linux — Windows dispatches Alt+Tab, Win, and
Ctrl+Alt+Del ahead of the foreground window, so the overlay can never trap the user. Hide
the cursor with `Window::set_cursor_visible(false)`. Do not call `ClipCursor`.

### 2. Reading the desktop's pixels

*Was: `zwlr_screencopy_v1` → `wl_shm`.*

**Answer:** DXGI Desktop Duplication — `IDXGIOutput1::DuplicateOutput`,
`AcquireNextFrame`, copy to a `D3D11_USAGE_STAGING` texture, `Map`, hand the pointer to
`BrightnessMap::update`. Roughly 150–250 lines through the `windows` crate.

As on Linux, **there is no permission prompt** — the entire TCC/Screen-Recording apparatus
that shaped the macOS original has no counterpart here either. The brightness-driven
`smash1`–`smash8` mechanic works from first launch rather than degrading to random.

Keep the graceful-degradation path anyway. Duplication fails in real situations —
`DXGI_ERROR_UNSUPPORTED` on some hybrid-graphics setups, an access loss on every resolution
change or session switch — and the existing `Option<u8>` fallback covers all of them for
free.

`BrightnessMap` itself needs no changes: it already accepts a strided byte buffer, which is
exactly what a mapped staging texture is. Watch the pixel order — Duplication gives BGRA.

### 3. Erasing what was already drawn

*Was: `tiny_skia::BlendMode::Clear`.*

**No problem at all.** The washer drove the entire damage-layer design, and that design
lives in `hakai-core/src/damage.rs` on `tiny-skia` pixmaps, on the CPU, identically on
every platform. Nothing to solve, and worth writing down precisely because it was the
hardest constraint in both previous ports.

### 4. Multiple monitors and fractional scale

*Was: one layer surface per `wl_output`, `wp_fractional_scale_v1`.*

**Answer:** one `winit` window per monitor, or a single window spanning the virtual desktop
rectangle. Prefer one per monitor — it matches the existing structure, where each output
already owns its own damage layer and scale factor, and it avoids the negative coordinates
a virtual-desktop-spanning window inherits when a secondary monitor sits left of or above
the primary.

`winit` reports Per-Monitor DPI v2 scale factors and forwards `ScaleFactorChanged`, so the
existing per-output scale handling maps over directly. Hot-plug arrives as
window/monitor events rather than `wl_output` add/remove.

## Phases

Ordered so the one item that can invalidate the estimate is settled on day one, and so
every phase after that has a working thing to run. Estimates are focused engineering days.

| # | Phase | Exit criterion | Est. |
|---|---|---|---:|
| 0 | **Spike: transparent topmost window.** `winit` + `wgpu`, one triangle, over the live desktop. Nothing else. This decides between routes 1, 2 and 3 above. | A translucent triangle floats over Explorer at 60 fps; Esc quits clean | 0.5–1 d |
| 1 | **Core builds for Windows.** `cargo check --target x86_64-pc-windows-msvc -p hakai-core`, then the examples. No window involved. | `cargo test -p hakai-core` and `--simulate` green on Windows | 0.5 d |
| 2 | **Window, input, renderer.** New `main()`, `winit` event loop, the 20-arm key `match` re-pointed at `KeyCode`, pointer events into the existing `State`. The 998 wgpu lines reattached unchanged. | All nine tools usable with mouse and keyboard, HUD and palette drawn | 2–3 d |
| 3 | **Audio.** `audio.rs` compiled against `cpal`'s WASAPI host. Expected to be a rebuild, not a rewrite. | 35 sounds load; stereo panning and loop gliding audibly correct | 0.5 d |
| 4 | **Capture.** Desktop Duplication into `BrightnessMap`; frozen mode background. | Impact sound differs over a dark vs. a light region; frozen mode shows the desktop | 1.5 d |
| 5 | **Multi-monitor and DPI.** One window per monitor, scale changes, hot-plug. | Correct on a 1× + 1.5× pair; unplugging a monitor does not panic | 1 d |
| 6 | **Package.** Portable `.zip` first — no installer, no certificate, runs from any folder. Then optionally an installer and signing. | Extracts and runs on a clean Windows box with no toolchain | 1–1.5 d |

**Total: 7–9 days** to a working, distributable Windows build, assuming phase 0 lands on
route 1. If it forces route 2 (DirectComposition), add 3–5 days to phase 2 and expect to
carry raw DXGI interop code for the life of the project.

## Structure

Add a third crate rather than `#[cfg]`-ing the existing binary into knots:

```
hakai-core/        unchanged, now built for three targets
hakai/             the Wayland binary, unchanged
hakai-win/         the Win32 binary
```

The awkwardness is the ~1,600 lines the two binaries would share — wgpu pipelines,
`audio.rs`, `text.rs`, `State`. Two options, and the second is better:

* **Duplicate them.** Fast, and they immediately drift.
* **Lift them into `hakai-core`** behind a `render` feature, leaving each binary with only
  its window, its event loop and its capture backend — a few hundred lines each. This is
  the same move that made the Windows port cheap in the first place, applied once more.
  `wgpu` and `cpal` are cross-platform, so nothing about them belongs in a
  platform-specific crate; only their *host* is platform-specific, and that is chosen at
  runtime rather than in the source.

**Done** (Phase 7, PRs #2–#4): the duplicate-first shortcut was taken to ship the port,
then the second option was carried out — `hakai_core::render` now owns the renderer,
`Scene` / `GpuLayer`, HUD text and `HudColors`; `audio.rs` alone stayed `#[path]`-included.

Prefer the second. It shrinks `hakai-win` to roughly the size of the problem it actually
represents.

## Signing and distribution

Same shape of problem as the macOS Developer ID, with worse economics.

SmartScreen is Gatekeeper's counterpart: an unsigned binary downloaded from the web raises
*"Windows protected your PC"*, and unlike Gatekeeper there is no per-file attribute a user
can clear from a terminal — they click *More info* → *Run anyway*.

* **Unsigned, portable `.zip`.** Free. Users see the SmartScreen prompt once, or avoid it
  entirely if the file arrives by any route that does not set the Mark of the Web.
* **OV Authenticode certificate**, ~200–400 EUR/year. Signs the binary, but a *new* OV
  certificate has no SmartScreen reputation, so the prompt persists until enough installs
  accumulate. This is the trap: you can pay and still be warned about.
* **EV Authenticode certificate**, ~350–600 EUR/year plus a hardware token. Immediate
  SmartScreen reputation. The only option that genuinely removes the warning on day one.

Recommendation: ship the portable `.zip` unsigned. This is a toy that draws on your
screen; the audience that wants it will get past one dialog, and an EV certificate costs
more per year than the entire project is worth to anyone.

## Risks

| Risk | Mitigation |
|---|---|
| Route 1 transparency does not work with a `wgpu` swapchain | Phase 0 exists precisely to find this out before anything is built on it. Route 2 is known-correct but expensive |
| `wgpu`'s D3D12 backend behaves differently from Vulkan for the per-tile blend | The damage layer composites on the CPU in `tiny-skia`; the GPU only blits premultiplied textures. Small blast radius |
| Desktop Duplication unavailable on hybrid graphics | Already covered by the existing `Option<u8>` fallback — the app degrades to random impact variants exactly as it does without the Wayland protocol |
| Shared code drifts between the two binaries | Lift it into `hakai-core` (see Structure) rather than duplicating |
| `cosmic-text` font discovery differs on Windows | Already a flagged risk on Linux. The fonts are embedded rather than looked up system-wide, which is what makes output identical everywhere — keep it that way |
| CI cost of a third target | `cargo check` for the Windows target on every push is cheap; a full `windows-latest` job only on tags |

## Non-goals

* **No Windows-native theming.** `theme.rs` exists because Omarchy has a coherent system
  palette worth matching. Windows does not, in the same sense — ship the built-in Tokyo
  Night palette and stop there.
* **No MSIX/Store packaging.** The Store requires the same sandboxing this app cannot
  accept, for exactly the reasons `OMARCHY-PORT.md`'s macOS predecessor gave up on the
  App Store: a full-screen input-swallowing overlay that reads the framebuffer.
* **No Windows 10 support target.** Build against Windows 11, note that Desktop Duplication
  and DWM transparency both exist on 10 and it will probably work there, and do not test it.
* **No ARM64 build** until someone asks.
