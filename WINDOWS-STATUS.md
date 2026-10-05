# Hakai on Windows — build status

Progress log for the Windows port. `WINDOWS-PORT.md` is the analysis, `WINDOWS-PLAN.md` is
the phased plan; this is what actually got built, what's confirmed, and what's left.

**Bottom line: the Windows build is functionally complete (phases 0–7) and shipping.**
It's a native transparent Direct3D 12 overlay with all nine tools, WASAPI audio,
brightness-driven impact sounds via DXGI Desktop Duplication, one window per monitor with
Per-Monitor DPI, and a portable `.zip` that runs on a clean machine — published on the
GitHub releases page since v1.0.0. Built and verified on real hardware, with most of the
runtime checklist below still pending a normal (non-RDP) display.

Since Phase 7 the port has shrunk twice more: the window/event-loop/input shell moved into
`hakai_core::shell` and the audio engine into `hakai_core::playback`, both shared with the
macOS and Android builds and the Linux GNOME/X11 fallback. `hakai-win` is now a `Platform`
impl on that shell plus its Windows-only glue.

Work happened on branch `windows`, then `wgpu-30-linux` → `render-lift` → `render-lift-wip`
for Phase 7 (PRs #2–#4); everything since is on `main`.

---

## What was done

| Phase | What | Commit | State |
|---|---|---|---|
| 0 | Spike: transparent, always-on-top, per-pixel-alpha overlay over the live desktop | `b441993` | ✅ verified |
| 1 | `hakai-core` builds + all tests pass on `x86_64-pc-windows-msvc` | — | ✅ verified; re-run by CI on every push |
| 2 | Window, winit event loop, keyboard/pointer, the wgpu renderer | `ecde42c` | ✅ verified |
| 3 | Audio — `cpal` on WASAPI, 35 bundled sounds | (in `ecde42c`) | ✅ verified |
| 4 | Screen capture — DXGI Desktop Duplication → `BrightnessMap` | `392cf38` | ✅ verified |
| 5 | One overlay window per monitor, Per-Monitor DPI v2 | `2d75786` | ✅ single-monitor; ⚠ multi-monitor unverified |
| 6 | Portable `.zip` + `windows-latest` CI job | `b31c4df` | ✅ verified |
| 7 | Renderer + scene lifted into `hakai_core::render` (both binaries) | PRs #2–#4 | ✅ CI green; `hakai` verified on Hyprland; `hakai-win` runtime unverified |
| — | Shell lifted into `hakai_core::shell`, audio into `hakai_core::playback` | on `main` | ✅ CI green; `hakai-win` runtime unverified |
| — | GitHub releases: `hakai-<version>-windows-x86_64.zip`, built by CI on `v*` tags | on `main` | ✅ v1.0.0–v1.3.0 published |

### The shape of the port

Almost everything is shared. `hakai-core` holds the whole engine — nine tools, damage
layer, decal/icon/sprite generators, termite colony, particles, HUD logic — and, behind
off-by-default features:

- `render` — the wgpu renderer, HUD text shaping, `BrightnessMap`, and the
  platform-agnostic `Scene` / `GpuLayer`
- `shell` — the winit shell: one window per monitor, the event loop, the `match` over
  `KeyCode`, pointer and touch routing into `Scene`, the capture tick
- `playback` — the `cpal` audio engine with all 35 sounds (`cpal` picks WASAPI)

`hakai-win/src/` is just the Windows edge (~530 lines):

| File | What |
|---|---|
| `main.rs` | `impl Platform for Win` — the DX12 / `DxgiFromVisual` instance, `WS_EX_NOREDIRECTIONBITMAP`, and the hooks into `win32.rs` and `duplication.rs` |
| `duplication.rs` | `DesktopDuplication` — DXGI Desktop Duplication producer for `BrightnessMap` |
| `theme.rs` | Windows stub — re-exports `hakai_core::render::HudColors`, both readers return `None` |
| `win32.rs` | Win32 escape hatches (non-occluding layered window, launcher-minimise, global Esc hotkey, cursor position) |

Plus `hakai-win/package.ps1` (release zip) and `hakai-win/README.txt` (end-user doc). The
binary is `hakai.exe`.

### Key technical decisions

- **wgpu 30, not 22.** Phase 0 found wgpu 22's D3D12 backend only offers
  `CompositeAlphaMode::Opaque` on a plain HWND — no transparency. wgpu 27+ added
  `Dx12SwapchainKind::DxgiFromVisual` (a DirectComposition presentation path) which does
  support it. The HWND also needs `WS_EX_NOREDIRECTIONBITMAP` (winit's
  `with_no_redirection_bitmap`) or the DComp visual composites over an opaque backing.
  The Windows side forced the bump; every other binary has since followed, so all of them
  build against wgpu 30.
- **Render, shell and audio code is shared.** The renderer + scene code that `hakai-win`
  used to carry a copy of (`render.rs` / `state.rs`) lives in `hakai_core::render`; the
  window and input handling that was `hakai-win/src/main.rs` lives in `hakai_core::shell`;
  `audio.rs`, once `#[path]`-included from the Linux crate, is `hakai_core::playback`.
  `hakai-win` keeps only its backend choice, native window styling and screen-capture
  backend.
- **No assets to bundle.** Every font and all 35 sounds are `include_bytes!`'d at compile
  time and nothing in `hakai-core` reads a file at runtime, so `hakai.exe` is fully
  self-contained and runs from anywhere. The plan's "move `hakai/assets/` to the repo
  root" step turned out unnecessary (the sounds live in `hakai-core/assets/`).
- **DPI the Linux way.** `GpuLayer.width/height` are points (logical); the wgpu surface
  buffer is `points × scale` (physical). NDC is a ratio, so every placement in
  `hakai_core::render::gpu` is identical either way. The one conversion is the pointer's
  physical position ÷ scale, done once in the shell.
- **Graceful degradation preserved.** `DesktopDuplication::new()` returns `None` on any
  failure (hybrid graphics, a Remote Desktop session, another duplicator holding the
  output); tools then fall back to a random impact-sound variant, exactly as on a
  compositor without `wlr-screencopy`.

---

## Verified on real hardware

The dev box is a **single-display Remote Desktop session that reports a bogus 3× DPI
scale** (a 3840×2160 "monitor" presented at ~1280×720), and synthetic keyboard input to
the overlay is flaky. So visual verification is partial. Confirmed by screenshot / log,
all of it **before** the Phase 7 and shell lifts:

- Transparent, always-on-top overlay composites per-pixel alpha over live windows, 50–60 fps
- **Hammer** cracks the desktop where you click; the crack lands where the click did
- **Color-thrower** splats paint; tool switching by digit key works
- The **3D hammer cursor** tracks the mouse and rotates on its knock animation
- The **HUD bar** and hint text render at the bottom of the screen
- **DPI**: the scene renders in point space into a supersampled buffer; cursor, cracks and
  HUD all align
- **Audio**: all 35 sounds load through WASAPI, 2ch @ 44.1 kHz
- **Desktop Duplication** activates; the full 3840×2160 desktop is captured every ~2 s;
  brightness-under-cursor tracks live desktop content (values move as windows redraw)
- **Portable zip** extracts into an empty directory and the binary runs from there with no
  toolchain

---

## Not yet verified — needs a normal monitor

Run `cargo run --release --manifest-path hakai-win/Cargo.toml` on a real Windows desktop
(ideally two monitors, ideally mixed DPI) and check. **The lifts re-open this list** — the
Windows binary now runs `hakai_core::render` and `hakai_core::shell` rather than its own
copies, and while that shared code is verified on Hyprland (render) and on macOS (render +
shell), no part of `hakai-win` has a recorded run since:

- [ ] The overlay still comes up at all — transparent, all layers, the scene renders
- [ ] All nine tools produce their effect (only hammer + color-thrower exercised so far)
- [ ] Chain-saw revs with movement; flame-thrower fires keep burning through a tool switch;
      termites survive a tool switch; the washer erases damage but not living termites
- [ ] `Tab` / `Shift+Tab` cycle forward / backward
- [ ] `↑` / `↓` open / close the tool palette; clicking a palette cell selects that tool
- [ ] `C` opens the credits panel; `R` clears; `M` freezes the view and the captured
      desktop snapshot renders as the frozen background
- [ ] Multi-monitor: one overlay per display, each at its own scale; the cursor crossing
      between monitors switches which overlay draws the tool cursor
- [ ] Unplugging a monitor while running doesn't panic (the shell drops that layer on
      `WindowEvent::Destroyed`)
- [ ] SmartScreen behaviour on a genuine download (Mark-of-the-Web) — one "More info → Run
      anyway", per `WINDOWS-PORT.md`
- [ ] A YouTube (or other hardware-accelerated) video behind the overlay keeps playing and
      stays visible — the `WS_EX_LAYERED` + alpha-254 mark (`win32::mark_non_occluding`) is
      the standard fix for browser occlusion-throttling, but couldn't be checked on the dev
      box (GDI/PrintWindow can't capture hardware video planes). `HAKAI_NO_LAYERED=1` backs
      it out.
- [ ] Launching from a terminal minimises that terminal (`win32::minimize_launcher`);
      double-clicking from Explorer minimises nothing. `HAKAI_KEEP_TERMINAL=1` keeps it.
- [x] Esc is a global hotkey (`win32::spawn_quit_hotkey`) — closes hakai even after
      Alt+Tabbing to another app. Verified. Reserves Esc system-wide while hakai runs;
      `HAKAI_NO_GLOBAL_ESC=1` makes it app-local.

Two exit criteria from `WINDOWS-PLAN.md` also have no recorded result: the dumped decal /
icon / sprite PNGs being byte-identical to the Linux ones (Phase 1), and how the mixer
behaves on a 48 kHz shared-mode WASAPI device (Phase 3 — the verified run was 44.1 kHz).

If the overlay renders at the wrong size on any box, `HAKAI_WINDOWED=1` runs it as a plain
window as a fallback.

---

## What's left

### 1. Verify on real hardware (above) — do this first

Everything else is polish; this is the actual "is it done" check.

### 2. Per-monitor Desktop Duplication (small) — not implemented

One `DesktopDuplication` captures the primary output (`EnumOutputs(0)`) and the shell feeds
that one capture to every layer. On a multi-monitor setup a secondary monitor's impact
sounds — and its `M` frozen background — come from the primary's desktop. Fix: one
`DesktopDuplication` per output (`IDXGIAdapter::EnumOutputs(i)`), matched to layers by
monitor, which also means the shell's `DesktopCapture` hook has to become per-window. Not
urgent — for the sounds, the current behaviour is no worse than the random fallback.

### 3. Runtime monitor hot-plug add (small) — not implemented

A monitor **removed** while running is handled (`WindowEvent::Destroyed` → drop the layer).
A monitor **added** while running isn't — you'd need to restart. The window list is built
once, in `hakai_core::shell`'s `init`. Fix: rescan `available_monitors()` periodically in
`about_to_wait` and create a window for any new one — which would fix macOS and the Linux
GNOME/X11 fallback at the same time, since they run the same shell.

### 4. Distribution — done, unsigned

- Releases are cut by CI: pushing a `v*` tag publishes
  `hakai-<version>-windows-x86_64.zip` (built by `hakai-win/package.ps1`) alongside the
  other platforms' artifacts and `SHA256SUMS`.
- Signing: shipped unsigned, deliberately. Per `WINDOWS-PORT.md` — an OV certificate
  (~200–400 EUR/yr) still raises SmartScreen until reputation accumulates; only an EV
  certificate (~350–600 EUR/yr + hardware token) removes it on day one, which costs more
  per year than the project is worth. The `.zip` sidesteps Mark-of-the-Web if it arrives
  by any route that doesn't set it.
- No MSIX / Store packaging — the Store sandbox can't host a full-screen input-swallowing
  framebuffer-reading overlay (`WINDOWS-PORT.md` non-goals).

### 5. Housekeeping

- The `HAKAI_WINDOWED` escape hatch now lives in the shared shell (it works on every
  winit-shell platform); `HAKAI_NO_LAYERED`, `HAKAI_KEEP_TERMINAL` and
  `HAKAI_NO_GLOBAL_ESC` are Windows-only, in `win32.rs`.
- The `windows` crate features list in `hakai-win/Cargo.toml` is minimal; add more only as
  needed.
