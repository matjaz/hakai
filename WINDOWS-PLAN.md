# Hakai on Windows — working plan

The companion to `WINDOWS-PORT.md`. That document is the analysis: what transfers, what
does not, and why. This one is the sequence of things to actually do, in order, with the
command to run and the fact that has to be true before moving on.

Written to be followed at a Windows machine with nothing installed on it.

**The shape of the work:** ~80% of the codebase (`hakai-core`, the wgpu draw code,
`audio.rs`, `text.rs`) is already cross-platform and needs no edits. What gets written is a
new binary crate holding a window, an event loop and a screen-capture backend. Estimated
7–9 days, with one caveat in Phase 0 that can move it.

---

## Before you start (~40 min)

```powershell
winget install Rustlang.Rustup
winget install Git.Git
winget install Microsoft.VisualStudio.2022.BuildTools
```

The Build Tools install must include **"Desktop development with C++"** — Rust's default
Windows toolchain is `x86_64-pc-windows-msvc` and links with MSVC. `rustup-init` will say
so if it is missing. Reboot or open a fresh terminal afterwards so `PATH` picks everything
up, then confirm:

```powershell
rustc -vV          # host: x86_64-pc-windows-msvc
cargo --version
```

Get the source across. A `git clone` from your own remote is cleaner than copying a working
tree, which drags `target/` and `.DS_Store` along:

```powershell
git clone <your-remote> hakai
cd hakai
```

Work on a branch from the start — Phase 7 is a refactor that touches the Linux binary, and
you want that reviewable separately:

```powershell
git switch -c windows
```

---

## Phase 0 — the decision (0.5–1 day)

**Everything in this plan rests on one unverified assumption: that `wgpu` can present a
per-pixel-alpha surface to a plain Win32 window.** On Wayland this is free. On Windows it
depends on what the D3D12 backend exposes, and nothing downstream is worth starting until
you know.

Do not build a window first. The cheapest possible version of this question comes first.

### 0a. Probe the alpha modes (~30 min)

A throwaway binary that creates a window, makes a `wgpu` surface on it, and prints one
line:

```rust
let caps = surface.get_capabilities(&adapter);
println!("alpha modes: {:?}", caps.alpha_modes);
println!("formats:     {:?}", caps.formats);
```

**The fact you need:** does `alpha_modes` contain `PreMultiplied` or `PostMultiplied`, or
only `Opaque`?

| Result | Meaning |
|---|---|
| `PreMultiplied` present | Route 1 works. Follow this plan as written |
| Only `Opaque` | The plain-HWND route cannot do per-pixel alpha. Go to *If Phase 0 fails* below |

Do this before writing anything else. It is thirty minutes and it decides the next week.

### 0b. The spike (~half a day)

Only if 0a came back positive. One transparent, borderless, always-on-top window covering
the primary monitor, with `wgpu` clearing to semi-transparent red and drawing one opaque
triangle, over a live desktop with Explorer visible behind it.

Window attributes (`winit` 0.30-era API — check names against whatever version resolves):

```rust
WindowAttributes::default()
    .with_transparent(true)
    .with_decorations(false)
    .with_window_level(WindowLevel::AlwaysOnTop)
    .with_inner_size(monitor.size())
    .with_position(monitor.position())
```

Prefer explicit monitor bounds over `Fullscreen::Borderless` — fullscreen modes on Windows
can trigger exclusive-mode paths and mode switches, which is not what an overlay wants.

Surface configuration is where the alpha actually happens:

```rust
config.alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;
```

and the clear colour must be **premultiplied** — `r: 0.5, g: 0.0, b: 0.0, a: 0.5`, not
`r: 1.0 … a: 0.5`. Getting this wrong looks exactly like the alpha not working, so rule it
out before concluding anything.

> **RISK.** `winit`'s `with_transparent(true)` on Windows historically routed through DWM
> blur-behind or `WS_EX_LAYERED`, and which one it uses has moved between releases. If the
> window comes up opaque despite a `PreMultiplied` surface, the next thing to try is
> setting the extended style yourself: get the `HWND` from `window.window_handle()`, then
> `SetWindowLongPtrW(hwnd, GWL_EXSTYLE, current | WS_EX_LAYERED)` and
> `SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA)` through the `windows` crate. Try
> this before giving up on Route 1.

**Exit criterion.** A translucent red sheet with a solid triangle floats above Explorer, at
60 fps, on the live desktop. Alt+Tab still switches away from it. Esc closes it cleanly.

Keep the spike. It becomes the skeleton of `hakai-win`.

### If Phase 0 fails

`alpha_modes` is `Opaque`-only *and* the `WS_EX_LAYERED` fallback does not help. Two ways
forward, in order of preference:

1. **DirectComposition.** `DCompositionCreateDevice`, a composition swapchain with
   `DXGI_ALPHA_MODE_PREMULTIPLIED`, visual tree built by hand, and `wgpu` rendering into a
   texture you present yourself. Correct and fast, but `wgpu` does not expose composition
   swapchains, so this means raw DXGI/D3D12 interop carried for the life of the project.
   **Add 3–5 days to Phase 2** and reconsider whether the port is worth it.
2. **`WS_EX_LAYERED` + `UpdateLayeredWindow`.** True per-pixel alpha everywhere, at the
   cost of a full-framebuffer CPU blit every frame. Fine at 1080p, will not hold 60 fps at
   4K. Acceptable only as a shipped fallback, not as the primary path.

Either way: **stop and re-plan rather than pushing on.** The rest of this plan assumes
Route 1.

---

## Phase 1 — the core, on Windows (0.5 day)

Prove the 80% claim on the machine that has to run it, before building anything on top.

**There is no Cargo workspace here.** The crates are siblings joined by a path dependency,
a deliberate choice recorded in `hakai/Cargo.toml`. So `-p hakai-core` does not work —
every command needs `--manifest-path`, exactly as `.github/workflows/ci.yml` already does:

```powershell
cargo test --manifest-path hakai-core/Cargo.toml --locked
cargo run  --manifest-path hakai-core/Cargo.toml --example simulate --locked
cargo run  --manifest-path hakai-core/Cargo.toml --example dump_decals --locked
cargo run  --manifest-path hakai-core/Cargo.toml --example dump_icons --locked
cargo run  --manifest-path hakai-core/Cargo.toml --example dump_sprites --locked
```

`hakai-core` depends only on `tiny-skia`, `ttf-parser`, `serde` and `serde_json` — four
pure-Rust crates with no OS surface — and carries **128 `#[test]` functions**. This should
pass on the first try. If it does not, the failure is small and local, and far better found
now than in Phase 2.

**Exit criterion.** All core tests green. The simulation prints its coverage table and
passes every rule-of-the-original. The dumped decal, icon and sprite sheets are visually
identical to the ones the Linux build produces.

That last check is worth doing properly: `tiny-skia` is deterministic CPU raster, so the
PNGs should be **byte-identical** across platforms, not merely similar. Compare hashes. If
they differ, something platform-dependent has crept into the generators and you want to
know before it is buried under a renderer.

---

## Phase 2 — window, input, renderer (2–3 days)

The bulk of the work. Create the new binary crate:

```
hakai-core/     unchanged
hakai/          the Wayland binary, unchanged
hakai-win/      new
```

A third sibling crate keeps the existing no-workspace arrangement. That choice was made to
avoid a shared `Cargo.lock` and a shared target directory between two independently working
setups; with a third crate the bookkeeping gets more tedious but the reasoning still holds.
Introducing a real `[workspace]` is defensible now — just do it as its own commit, not
folded into the port.

### Sharing the code without duplicating it yet

`hakai-win` needs four things that currently live in `hakai/src`: the wgpu pipelines and
draw functions (`main.rs` 283–1280), `State` (`main.rs` 1280–2069), `audio.rs` and
`text.rs`. Phase 7 lifts these into `hakai-core` properly. Until then, do **not** copy them
— point at them:

```rust
#[path = "../../hakai/src/audio.rs"] mod audio;
#[path = "../../hakai/src/text.rs"]  mod text;
```

It is a hack, it keeps the two binaries honest while you work, and it makes Phase 7 a move
rather than a merge. The wgpu and `State` code is currently inside `hakai/src/main.rs` and
cannot be `#[path]`-included as-is; extract those two regions into `hakai/src/render.rs`
and `hakai/src/state.rs` first, as a pure move with no behaviour change, and commit that
on its own so the diff is reviewable.

### The order to build it in

1. **Window and event loop.** Extend the Phase 0 spike: one window on the primary monitor,
   `winit`'s `ApplicationHandler`, redraw driven by `RedrawRequested`.
2. **Renderer.** Wire in `render.rs` unchanged. Damage tiles, sprite atlas, particles,
   cursor, HUD, in the existing z-bands. This is the moment the app should suddenly look
   like itself.
3. **Keyboard.** The existing handler is a flat `match` over 20 keysyms. It becomes a flat
   `match` over `winit::keyboard::KeyCode`. The decisions port verbatim; only the names
   change:

   | Wayland | winit |
   |---|---|
   | `Keysym::Escape` | `KeyCode::Escape` |
   | `Keysym::_1` … `_9` | `KeyCode::Digit1` … `Digit9` |
   | `Keysym::r`/`R`, `c`/`C`, `m`/`M` | `KeyCode::KeyR`, `KeyC`, `KeyM` |
   | `Keysym::Tab` + `shift_held` | `KeyCode::Tab` + `ModifiersState::SHIFT` |
   | `Keysym::ISO_Left_Tab` | *(no equivalent — Windows reports Shift+Tab as Tab)* |
   | `Keysym::Up` / `Down` | `KeyCode::ArrowUp` / `ArrowDown` |

   Use `KeyCode` (physical) rather than the logical key, so digits work on non-QWERTY
   layouts — the same reasoning the Wayland handler records for keysyms. `ISO_Left_Tab`
   simply disappears: on Windows, Shift+Tab arrives as `Tab` with the shift modifier, which
   the existing `shift_held` branch already handles.
4. **Pointer.** `CursorMoved`, `MouseInput`. Note the coordinate origin: `winit` gives
   physical pixels with **y down from the top-left**, and the scene works bottom-left up.
   Flip once, at the boundary, and never again — a flip applied in two places is the
   classic way to lose an afternoon here.
5. **Cursor.** `window.set_cursor_visible(false)`. Do **not** call `ClipCursor` or grab the
   pointer; nothing should be able to trap the user.

**Exit criterion.** All nine tools usable with mouse and keyboard. Palette opens and
clicking it selects a tool. HUD, credits panel and toasts render. `R` clears, `Esc` quits.
Fire keeps burning through a tool change. It is, at this point, the actual application.

---

## Phase 3 — audio (0.5 day)

`audio.rs` compiles against `cpal`'s WASAPI host with no source changes expected. This is a
rebuild, not a port.

**Exit criterion.** All 35 sounds load. Stereo panning follows the mouse's X position. Loop
voices glide rather than click on tool switches — the chain-saw is the sensitive one, since
its two loops crossfade purely by gain.

> WASAPI's default output can be a shared-mode 48 kHz device while the bundled WAVs are
> 44.1 kHz mono. `cpal` will report the device's actual sample rate; check whether the
> existing mixer resamples or assumes. On PipeWire this is handled upstream and may never
> have been exercised.

---

## Phase 4 — screen capture (1.5 days)

Replace `zwlr_screencopy_v1` with DXGI Desktop Duplication. `BrightnessMap` itself needs no
changes — `update(bytes, width, height, stride)` already takes exactly what a mapped
staging texture is.

Through the `windows` crate:

1. `D3D11CreateDevice`
2. `IDXGIOutput1::DuplicateOutput`
3. `AcquireNextFrame` → `IDXGIResource` → `ID3D11Texture2D`
4. `CopyResource` into a `D3D11_USAGE_STAGING` texture with `CPU_ACCESS_READ`
5. `Map` → `D3D11_MAPPED_SUBRESOURCE` → `pData` + `RowPitch` as `bytes` + `stride`
6. `Unmap`, `ReleaseFrame`

**No permission prompt of any kind** — the whole TCC apparatus from the macOS original has
no counterpart here, exactly as on Linux. The brightness-driven `smash1`–`smash8` mechanic
works from first launch instead of degrading to random.

Two things to get right:

* **Pixel order is BGRA**, not RGBA. If dark surfaces start sounding like bright ones,
  this is why.
* **Keep the failure path.** Duplication genuinely fails: `DXGI_ERROR_UNSUPPORTED` on some
  hybrid-graphics laptops, `DXGI_ERROR_ACCESS_LOST` on every resolution change, secure-desktop
  transition and session switch. The existing `Option<u8>` fallback covers all of it — the
  app degrades to random impact variants, which is exactly what it does on a compositor
  that lacks the Wayland protocol. Do not turn these into panics.

**Exit criterion.** The impact sound audibly differs over a dark region versus a light one.
Frozen mode shows the captured desktop. Locking and unlocking the machine does not crash it.

---

## Phase 5 — multiple monitors and DPI (1 day)

One `winit` window per monitor, mirroring the current one-layer-surface-per-`wl_output`
structure — each output already owns its own damage layer and scale factor.

Prefer this over a single window spanning the virtual desktop: a monitor placed left of or
above the primary gives negative virtual-desktop coordinates, and that is an easy source of
off-by-a-screen bugs.

`winit` reports Per-Monitor DPI v2 scale factors and forwards `ScaleFactorChanged`, so the
existing per-output scale handling maps over. Monitor hot-plug arrives as window/monitor
events rather than `wl_output` add and remove.

**Exit criterion.** Correct on a 1× + 1.5× pair, with the cursor crossing between them.
Unplugging a monitor while running does not panic.

---

## Phase 6 — package (1–1.5 days)

Ship a **portable `.zip`** first: the stripped release binary plus `assets/`, extracted
anywhere, no installer, no certificate.

```powershell
cargo build --release --manifest-path hakai-win/Cargo.toml --locked
```

**The bundled assets live in `hakai/assets/`**, inside the Linux binary's crate rather than
at the repo root — 35 WAVs plus `manifest.json`. `hakai-win` needs them too, so decide
early: move `assets/` up to the repo root and point both crates at it, or keep one copy and
reference it across. Moving it is cleaner and is a good thing to do in Phase 2 rather than
discovering it at packaging time.

**Exit criterion.** The zip extracts and runs on a clean Windows box with no Rust toolchain
and no Visual Studio.

On signing — see `WINDOWS-PORT.md` for the full reasoning, but the short version: an
unsigned download raises SmartScreen once, an OV certificate (~200–400 EUR/yr) still raises
it until reputation accumulates, and only an EV certificate (~350–600 EUR/yr plus a hardware
token) removes it on day one. For a toy that draws on your screen, ship it unsigned.

---

## Phase 7 — the lift — ✅ done (PRs #2–#4)

Both binaries were bumped to wgpu 30 first (the Linux side was still on 22), then the code
was lifted into `hakai-core` behind an off-by-default `render` feature:

- `hakai_core::render::{gpu, scene, text, theme}` + `capture` + `shader.wgsl` + the
  JetBrains Mono fonts. `Scene` / `GpuLayer` are platform-generic (no winit/Wayland types);
  each binary feeds screen-capture bytes in via `Scene::feed_layer_capture`.
- `hakai-win` deleted `render.rs` / `state.rs`; `hakai` deleted ~900 lines of inline render
  helpers and its own `GpuLayer` / render / `advance` / tool-switching code. Each binary is
  now window + event loop + capture backend.

Deltas from the plan above: `audio.rs` stayed `#[path]`-included (it's tiny and already
worked); the extracted-first `hakai/src/render.rs` / `state.rs` step was skipped — the
Windows port's own `render.rs` / `state.rs` were the clean module form and moved
near-verbatim, and `hakai`'s inline version was deleted rather than reconciled.

**Exit criterion — met.** Both binaries build from the shared core; `hakai-win/src` is
~815 lines (`main.rs` 408, the rest DXGI + Win32 glue), and `hakai/src/main.rs` dropped
from ~2,980 to ~900. All three CI jobs green, including the Linux `hakai` build — the check
that the lift was a move, not a rewrite. `hakai` verified running on Hyprland.

---

## CI

Add a `windows-latest` job running
`cargo test --manifest-path hakai-core/Cargo.toml --locked` and
`cargo build --manifest-path hakai-win/Cargo.toml --locked`, mirroring the existing
`ubuntu-latest` jobs in `.github/workflows/ci.yml`. It cannot verify anything visual — GitHub's runners have no
interactive desktop session, so the window and capture code can be built but never
exercised — but it keeps the core honest and catches Windows-only compile breakage on the
Linux side of development.

---

## Known Windows behaviours (none fatal)

| Behaviour | Consequence |
|---|---|
| The taskbar can surface above a topmost window | A strip of the canvas is occluded |
| The UAC secure desktop hides everything | The overlay vanishes during a UAC prompt and returns after |
| Exclusive-fullscreen apps take the display | The overlay is not visible over them |
| Other always-on-top windows | Cannot be drawn over; there is no level above `HWND_TOPMOST` |
| Win+D / Show Desktop | May minimise or hide the overlay |

All acceptable for a desktop toy. None of them trap the user, which is the property that
actually matters: Windows dispatches Alt+Tab, Win and Ctrl+Alt+Del ahead of the foreground
window, so there is always a way out even before Esc.

---

## Definition of done

* All nine tools work with mouse and keyboard, on multiple monitors at mixed DPI.
* Impact sounds follow the brightness of the surface underneath.
* Sound pans with the mouse's X position.
* Termites survive a tool change; fire keeps burning through one.
* The washer erases damage but not living termites.
* `cargo test --manifest-path hakai-core/Cargo.toml` green on Windows; dumped assets
  byte-identical to Linux.
* A portable zip runs on a clean machine.
* Esc quits; Alt+Tab works throughout.
