# Phase 0 — build & run walkthrough

Goal: prove `wgpu` can render onto a `wlr-layer-shell` surface, on every output, with a
hidden cursor and a clean Esc-to-quit. That's it — no damage layer, no tools, no audio.
See `OMARCHY-PORT.md` in the desktop-destroyer repo for why this specific step is first.

This has to run **inside a live Hyprland session** — not a TTY, not SSH without a
`WAYLAND_DISPLAY` forwarded, not a nested Xorg session.

## 1. System dependencies (Arch/Omarchy)

Omarchy ships Hyprland, Mesa and Wayland already; what's usually missing for a from-scratch
Rust/Wayland dev setup is:

```bash
sudo pacman -S --needed base-devel rustup pkgconf libxkbcommon wayland wayland-protocols
rustup default stable
```

Vulkan should already be present (Omarchy needs it for Hyprland itself), but if `cargo
build` later complains about a missing Vulkan loader:

```bash
sudo pacman -S --needed vulkan-icd-loader mesa
vulkaninfo --summary   # sanity check — should list your GPU, not error out
```

## 2. Build

```bash
cd ~/omarchy/hakai
cargo build
```

**Expect this not to compile clean on the first try.** The one part of `src/main.rs` I
could not verify without a compiler is marked `RISK` in a comment — it hand-extracts raw
`wl_display`/`wl_surface` pointers to satisfy `wgpu`'s `raw-window-handle` requirement,
using methods (`Connection::backend().display_ptr()`, `Proxy::id().as_ptr()`) that exist in
the crate versions I targeted but that I'm recalling, not compiling against. If those
specific names have moved, the compiler error will point straight at that block.

**What to send back if it fails:** the full `cargo build` output, not just the last error —
version-mismatch errors often show the *real* problem several lines above the final one.
Paste it back and I'll patch `main.rs`/`Cargo.toml` against it.

Other likely first-build friction, roughly in order of likelihood:

- **A `smithay-client-toolkit` trait method signature is slightly off** (an added/removed
  parameter on a `*Handler` trait). Fix: match the compiler's expected signature — these
  traits change shape between sctk minor versions more often than anything else here.
- **`wayland-client` resolves to two different versions** (one direct, one via sctk),
  so the `client_system` feature doesn't reach the copy sctk actually uses. Fix: run
  `cargo tree -i wayland-client` and pin the direct dependency's version to match exactly.
- **`wgpu = "22"` has moved on and some struct gained/lost a field** (`wgpu::SurfaceConfiguration`
  and `wgpu::DeviceDescriptor` are the two most churn-prone structs in wgpu's history). Fix:
  add or remove fields per the compiler message; the crate's own docs.rs page for your
  resolved version is the fastest reference.

None of this is a sign the approach is wrong — it's exactly the "small round of fixes"
flagged in `OMARCHY-PORT.md`'s Phase 0 entry.

## 3. Run

```bash
RUST_LOG=info cargo run
```

**Expected:** a translucent red layer over every connected output, above Waybar and
everything else, cursor hidden, keyboard exclusively grabbed. The log line
`hakai phase 0: N layer surface(s) up` tells you it saw all your outputs.

Press **Esc** — it should quit immediately and cleanly, with nothing left running
(`pgrep hakai` should come back empty).

## 4. What "done" looks like

Check off against the Phase 0 exit criterion in `OMARCHY-PORT.md`:

- [ ] Red overlay appears on **every** connected monitor, not just the first
- [ ] It's above Waybar/any bar, above other windows — nothing shows through except itself
- [ ] Cursor is invisible over the overlay
- [ ] `SUPER`-anything still fires (try opening the Omarchy menu) — the compositor bind
      still reaches Hyprland even though the surface holds an exclusive keyboard grab
- [ ] Esc quits instantly, no hang, no leftover process
- [ ] Unplugging/replugging a monitor (or toggling one off in `hyprctl`) doesn't crash it —
      not required to *handle* it gracefully yet, just not to panic

If all of those hold, Phase 0 is done and we move to Phase 1 (the headless raster core,
which — unlike this — can be built and tested back on macOS with plain `cargo test`).

## 5. If Vulkan itself is the problem

If `cargo run` gets past compiling but panics on `request_adapter`/`request_device` rather
than a Wayland error, that's a Mesa/Vulkan issue, not a Wayland one — worth isolating from
the Wayland-specific code with:

```bash
cargo run --example vulkan-check 2>/dev/null || vulkaninfo --summary
```

(there's no `vulkan-check` example in this crate yet — `vulkaninfo` alone is enough to tell
whether the GPU/driver side is sound before blaming `main.rs`).

## 6. Follow-up, found during Phase 7 — `request_device` under-asked for texture limits

`request_device` originally passed `wgpu::Limits::downlevel_defaults()` — the WebGL2-safe
profile, capped at a 2048×2048 max texture size, appropriate for "this also has to run
inside a browser," which this native Vulkan app never needed to care about. Never caught
here because every output tested through Phases 0–6 happened to have a buffer size (points
× `scale`) under 2048 in both dimensions; Phase 7 hit an output where `scale` pushed it
over, and `Surface::configure` failed its own validation outright (a wgpu surface's pixel
buffer is one texture, so this cap applies to the *whole output*, not just individual
sprites). Fixed by requesting `adapter.limits()` instead of the downlevel profile — exactly
as safe to request (it's what `request_adapter` already established this specific adapter
supports) and removes the cap entirely on any real Vulkan driver (typically 8192 or
16384). See `hakai/PHASE7.md` for the full writeup of where this surfaced.
