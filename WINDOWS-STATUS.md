# Hakai on Windows — build status

Progress log for the Windows port. `WINDOWS-PORT.md` is the analysis, `WINDOWS-PLAN.md` is
the phased plan; this is what actually got built, what's confirmed, and what's left.

**Bottom line: the Windows build is functionally complete (phases 0–6).** It's a native
transparent Direct3D 12 overlay with all nine tools, WASAPI audio, brightness-driven
impact sounds via DXGI Desktop Duplication, one window per monitor with Per-Monitor DPI,
and a portable `.zip` that runs on a clean machine. Built and verified on real hardware,
with a few checks still pending a normal (non-RDP) display.

Work happened on branch `windows`.

---

## What was done

| Phase | What | Commit | State |
|---|---|---|---|
| 0 | Spike: transparent, always-on-top, per-pixel-alpha overlay over the live desktop | `b441993` | ✅ verified |
| 1 | `hakai-core` builds + all 128 tests pass on `x86_64-pc-windows-msvc` | — | ✅ verified |
| 2 | Window, winit event loop, keyboard/pointer, the wgpu renderer | `ecde42c` | ✅ verified |
| 3 | Audio — `cpal` on WASAPI, 35 bundled sounds | (in `ecde42c`) | ✅ verified |
| 4 | Screen capture — DXGI Desktop Duplication → `BrightnessMap` | `392cf38` | ✅ verified |
| 5 | One overlay window per monitor, Per-Monitor DPI v2 | `2d75786` | ✅ single-monitor; ⚠ multi-monitor unverified |
| 6 | Portable `.zip` + `windows-latest` CI job | `b31c4df` | ✅ verified |

### The shape of the port

~80% of the code crosses unchanged. `hakai-core` (nine tools, damage layer, decal/icon/
sprite generators, termite colony, particles, HUD/credits logic) is untouched. `audio.rs`,
`text.rs` and `capture.rs` are `#[path]`-included from `hakai/src/` **verbatim** — `cpal`
picks WASAPI, `cosmic-text` and the `BrightnessMap` need no changes.

New in `hakai-win/src/`:

| File | What |
|---|---|
| `main.rs` | winit shell: one overlay window per monitor, `match` over `KeyCode`, pointer → `State` |
| `render.rs` | the Linux binary's Phase 4 draw code (pipelines, tile/sprite helpers, HUD/palette/credits pixmap builders, the per-frame render) **ported to wgpu 30**; wgpu state bundled into one `Assets` struct |
| `state.rs` | `State` + `GpuLayer` with every Wayland type stripped; damage layer, nine tools, particles, colony, HUD logic, `advance`, tool switching — verbatim from `hakai/src/main.rs` |
| `duplication.rs` | `DesktopDuplication` — DXGI Desktop Duplication producer for `BrightnessMap` |
| `theme.rs` | Windows stub — ships the built-in palette (`HudColors::FALLBACK`), both readers return `None` |

Plus `hakai-win/package.ps1` (release zip) and `hakai-win/README.txt` (end-user doc).

### Key technical decisions

- **wgpu 30, not 22.** Phase 0 found wgpu 22's D3D12 backend only offers
  `CompositeAlphaMode::Opaque` on a plain HWND — no transparency. wgpu 27+ added
  `Dx12SwapchainKind::DxgiFromVisual` (a DirectComposition presentation path) which does
  support it. The HWND also needs `WS_EX_NOREDIRECTIONBITMAP` (winit's
  `with_no_redirection_bitmap`) or the DComp visual composites over an opaque backing.
  So `hakai-win` tracks modern wgpu while `hakai` (Linux) stays on pinned 22.
- **Render code is duplicated, not shared** (yet). The ~1000 lines of wgpu draw code can't
  be `#[path]`-included across two binaries on different wgpu majors. Unifying them into
  `hakai-core` behind a feature — and bumping the Linux binary to wgpu 30 — is Phase 7,
  deferred.
- **No assets to bundle.** Every font and all 35 sounds are `include_bytes!`'d at compile
  time (the Linux binary does this too) and nothing in `hakai-core` reads a file at
  runtime, so `hakai-win.exe` is fully self-contained — 13 MB, runs from anywhere. The
  plan's "move `hakai/assets/` to the repo root" step turned out unnecessary.
- **DPI the Linux way.** `GpuLayer.width/height` are points (logical); the wgpu surface
  buffer is `points × scale` (physical). NDC is a ratio, so every placement in `render.rs`
  is identical either way. The one conversion is `CursorMoved`'s physical position ÷ scale,
  done once in `main.rs`.
- **Graceful degradation preserved.** `DesktopDuplication::new()` returns `None` on any
  failure (hybrid graphics, a Remote Desktop session, another duplicator holding the
  output); tools then fall back to a random impact-sound variant, exactly as on a
  compositor without `wlr-screencopy`.

---

## Verified on real hardware

The dev box is a **single-display Remote Desktop session that reports a bogus 3× DPI
scale** (a 3840×2160 "monitor" presented at ~1280×720), and synthetic keyboard input to
the overlay is flaky. So visual verification is partial. Confirmed by screenshot / log:

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
- **Portable zip** extracts into an empty directory and `hakai-win.exe` runs from there
  with no toolchain

---

## Not yet verified — needs a normal monitor

Run `cargo run --release --manifest-path hakai-win/Cargo.toml` on a real Windows desktop
(ideally two monitors, ideally mixed DPI) and check:

- [ ] All nine tools produce their effect (only hammer + color-thrower exercised so far)
- [ ] Chain-saw revs with movement; flame-thrower fires keep burning through a tool switch;
      termites survive a tool switch; the washer erases damage but not living termites
- [ ] `Tab` / `Shift+Tab` cycle forward / backward
- [ ] `↑` / `↓` open / close the tool palette; clicking a palette cell selects that tool
- [ ] `C` opens the credits panel; `R` clears; `M` freezes the view and the captured
      desktop snapshot renders as the frozen background
- [ ] Multi-monitor: one overlay per display, each at its own scale; the cursor crossing
      between monitors switches which overlay draws the tool cursor
- [ ] Unplugging a monitor while running doesn't panic (the code drops that layer on
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

If the overlay renders at the wrong size on any box, `HAKAI_WINDOWED=1` runs it as a plain
window as a fallback.

---

## Next steps

### 1. Verify on real hardware (above) — do this first

Everything else is polish; this is the actual "is it done" check.

### 2. Phase 7 — unify the render code (optional, ~1 day)

`hakai-win` and `hakai` share ~1,600 lines through the `#[path]` includes plus the
duplicated `render.rs` / `state.rs`. Finish the job: lift `render.rs`, `state.rs`,
`audio.rs` and `text.rs` into `hakai-core` behind a `render` feature, leaving each binary
with only its window, its event loop and its capture backend. This **requires bumping the
Linux binary to wgpu 30** — do that as its own commit, verify the Linux build still passes
CI (that's the check that the wgpu bump was mechanical), then do the lift.

Deferred by choice for now — keeping the Linux binary on its known-good wgpu 22 while the
Windows side stabilises.

### 3. Per-monitor Desktop Duplication (small)

Right now one `DesktopDuplication` captures the primary output and every layer samples its
brightness from that. On a multi-monitor setup a secondary monitor's impact sounds are
driven by the wrong desktop. Fix: one `DesktopDuplication` per output
(`IDXGIAdapter::EnumOutputs(i)`), matched to layers by monitor. Not urgent — the current
behaviour is no worse than the random fallback.

### 4. Runtime monitor hot-plug add (small)

A monitor **removed** while running is handled (`WindowEvent::Destroyed` → drop the layer).
A monitor **added** while running isn't — you'd need to restart. Fix: rescan
`available_monitors()` periodically in `about_to_wait` and create a window for any new one.

### 5. Distribution

- Cut a GitHub release, attach `hakai-win-<version>-x86_64-windows.zip` (built by
  `hakai-win/package.ps1`).
- Signing: ship unsigned. Per `WINDOWS-PORT.md` — an OV certificate (~200–400 EUR/yr) still
  raises SmartScreen until reputation accumulates; only an EV certificate (~350–600 EUR/yr
  + hardware token) removes it on day one, which costs more per year than the project is
  worth. The `.zip` sidesteps Mark-of-the-Web if it arrives by any route that doesn't set
  it.
- No MSIX / Store packaging — the Store sandbox can't host a full-screen input-swallowing
  framebuffer-reading overlay (`WINDOWS-PORT.md` non-goals).

### 6. Housekeeping

- `main.rs` still has an `HAKAI_WINDOWED` escape hatch and some `log::info!` startup noise
  — fine to keep, but trim if it bothers you.
- The `windows` crate features list in `hakai-win/Cargo.toml` is minimal; add more only as
  needed.
