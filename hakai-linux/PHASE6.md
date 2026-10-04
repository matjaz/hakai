# Phase 6 — build & run walkthrough (chunk 1: screen capture, the brightness map)

Goal: `zwlr_screencopy_v1` screen capture, feeding `hakai_core::audio::smash_name`'s
`brightness: Option<u8>` parameter — ported since Phase 3, but every `ToolContext` in this
port has passed `None` for it until now, since there was nothing to sample from. `None`
falls back to a random impact-sound variant instead of the real "sound by color brightness
below mouse cursor" mechanic (`Hammer.swift`'s own description) that makes the original's
hammer feel alive — a dark surface sounds hollow/wooden, a light one glassy/metallic.

**This is the highest-risk single chunk in the whole port since Phase 0's raw Wayland
pointer bridging.** Everything else that's touched a new protocol or crate so far has
either been well-documented (`cpal`) or had a stable, wrapped API to lean on
(`smithay-client-toolkit`'s own compositor/output/seat/layer-shell modules).
`zwlr_screencopy_v1` is wlroots-specific — `sctk` doesn't wrap it — so this chunk drives
raw generated bindings (`wayland-protocols-wlr`) directly, the same low-level `Dispatch`
machinery Phase 0 first proved out for layer-shell itself. Every request/event signature
used here (`capture_output`, `copy`, the `Event` enum's exact shape) was checked against
`docs.rs`'s real, current pages before being written — not just recalled — specifically
because this class of API (thin, auto-generated wayland-scanner bindings) has essentially
no room for a "close enough" guess the way a hand-written convenience API might.

## What it does

Every `CAPTURE_INTERVAL` (2 seconds) per output: request a capture
(`ZwlrScreencopyManagerV1::capture_output`), receive a `buffer` event describing the
format/size the compositor wants to write into, create a matching SHM buffer
(`smithay-client-toolkit`'s `shm`/`slot` modules — the one piece of this chunk that *is*
a wrapped, higher-level API), `copy` into it, and on `ready` downsample the raw pixels
into a coarse 24×16 brightness grid (`capture.rs`'s `BrightnessMap`) using perceptual luma
(Rec. 601), not a flat RGB average. Every `ToolContext` construction site now samples that
grid at the cursor's position instead of passing `None`.

**Coarse and periodic on purpose.** A tool only ever needs "is it dark or light right
here," not the actual desktop image — averaging into a 24×16 grid, once every 2 seconds
per output, is a lot less `zwlr_screencopy_v1` traffic and SHM copying than sampling
continuously, and the desktop underneath rarely changes fast enough for staleness to
matter for this specific use.

**Per-output capture state is three `Option` fields on `GpuLayer`, not a separate enum.**
`capture_frame`/`capture_buffer_info`/`capture_buffer` — `None`/`None`/`None` is idle;
`Some`/`None`/`None` is waiting on `buffer_done`; `Some`/`Some`/`Some` is a copy in
flight. `capture.rs` itself only holds the brightness grid, deliberately Wayland-agnostic
(see its own module doc comment).

**Everything's optional, gracefully.** No `zwlr_screencopy_manager_v1` (a non-wlroots
compositor), no `wl_shm` (essentially impossible, but still handled), a failed SHM pool,
a failed individual capture — none of these are treated as fatal. Every path just leaves
`brightness` sampling at `None` for whichever output/moment is affected, falling back to
this port's existing "no screen-capture permission" random-variant behavior, unchanged
since Phase 3.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

(unchanged — this chunk touched only `hakai`)

```bash
cd /mnt/mac/hakai
cargo build && RUST_LOG=info cargo run
```

If it doesn't compile, paste back the *first* error in full — given how much of this
chunk is thin generated bindings, don't assume the fix is obvious from the message alone.
If it does build: switch to the hammer (`1`) and strike over a light part of the desktop
(a bright wallpaper area, a light window), then a dark part — the impact sound should
audibly differ (glassy/metallic vs. hollow/wooden) rather than sounding randomly picked
each time.

## What "done" looks like

- [ ] `hakai` builds with `wayland-protocols-wlr` in the dependency tree
- [ ] No capture-related errors/warnings spamming the log at `RUST_LOG=info`
- [ ] The hammer's impact sound audibly tracks light vs. dark areas of the real desktop,
      not just picking randomly
- [ ] Nothing else regressed

**Confirmed working** on a real run ("works") — one fix needed, a genuine borrow-checker
catch rather than an API-shape mistake: `CompositorHandler::frame` already borrowed
`self.device`/`self.queue`/... immutably at its top (kept alive for the rest of the
function, for `render`), and `maybe_start_capture`'s `&mut self` can't overlap that.
Every `zwlr_screencopy_v1`/SHM API call itself — the part actually checked against live
docs beforehand — compiled on the first try.

---

# Phase 6 — chunk 2: frozen mode

`M` toggles `DisplayMode.frozen` — a captured snapshot shown as the overlay's own opaque
background instead of true transparency, so the real desktop underneath (other windows
moving, redrawing) can keep changing without disturbing what the user sees themselves
smashing. Builds directly on chunk 1's capture plumbing rather than anything new on the
Wayland side.

**A second use for one capture.** Entering frozen mode doesn't start a *different* kind
of capture — the same `zwlr_screencopy_v1` request that already feeds `BrightnessMap`
now also (only while frozen, so live mode isn't stuck constantly uploading full-res
frames it never shows) converts to a fully opaque RGBA8 buffer (`capture::to_rgba` —
BGRA→RGBA, alpha forced to 255) and uploads it as a texture.

**Frozen doesn't mean "keeps refreshing."** The obvious version of `maybe_start_capture`
— "due" once `since_last_capture >= CAPTURE_INTERVAL`, unconditionally — would have kept
the "frozen" snapshot drifting to a new moment every 2 seconds, which isn't frozen at
all. Fixed by splitting the "due" check itself: live mode keeps the periodic cadence
(for the brightness map); frozen mode is only ever due *once*, right when
`snapshot_texture` is still `None` — the first capture after entering frozen mode, and
never again until leaving and re-entering.

**Drawn through the existing rotated-sprite pipeline, not a new one.** The snapshot is
always axis-aligned and always fully opaque, which would normally point at the plain
tile pipeline — but that would have meant threading a `tile_bind_group_layout` parameter
through `render()`'s already-long signature for exactly one caller. `draw_rotated_sprite`
at `rotation: 0.0, alpha: 1.0` does the identical thing with machinery `render()` already
has in scope.

**The hint text's `M mode` is back.** It was deliberately dropped back in chunk 4g,
before `toggle_mode` existed to bind it to anything.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

(unchanged — this chunk touched only `hakai`)

```bash
cd /mnt/mac/hakai
cargo build && RUST_LOG=info cargo run
```

Press `M` — within a couple of seconds (the first capture after toggling) the desktop
should visibly "lock" to whatever it looked like at that instant, even if something else
on screen keeps moving underneath. Damage/tools should still work normally on top of it.
Press `M` again to return to the live, transparent view.

## What "done" looks like

- [ ] `hakai` builds
- [ ] `M` freezes the view to a static snapshot, `M` again returns to live
- [ ] The frozen snapshot doesn't visibly keep updating on its own
- [ ] Tools still work normally while frozen
- [ ] Nothing else regressed

If that holds, this chunk is done. Still open: `wp_fractional_scale_v1` (per-output
scale, for a HiDPI/fractional output — `DamageLayer::new`'s `scale` parameter is already
there, still hardcoded to `1.0`).

**Confirmed working** on a real run ("works") — froze/unfroze cleanly, tools kept
working on top of the snapshot, no regressions.

---

# Phase 6 — chunk 3: fractional scale

`wp_fractional_scale_v1` (staging) + `wp_viewporter` (stable) — correct, crisp rendering
on a non-integer output scale (1.25x, 1.5x, ...), rather than the `scale` hardcoded to
`1.0` every prior chunk shipped with. Both raw generated `Dispatch` bindings, the same
low-level pattern chunk 1's `zwlr_screencopy_v1` work established — neither protocol is
wrapped by `smithay-client-toolkit`.

**The one design decision that mattered here, and the one this chunk almost got wrong.**
The first pass made `gpu.width`/`gpu.height` themselves real pixels
(`round(points * scale)`), with a separate `logical_width`/`logical_height` pair for
points, and scaled every mouse position up to match. That's backwards for this codebase:
`hakai_core::damage::DamageLayer` (confirmed by reading its own source, not assumed) is
built expecting **points**, throughout — `stamp`/`erase`/`fill_circle` positions, tile
placement, everything. Caught before ever reaching the user's compiler, by re-reading
`damage.rs` directly rather than trusting the first design.

The corrected design is simpler than the wrong one: `gpu.width`/`gpu.height` stay in
points, completely unchanged from every phase before this one — no second pair of fields
needed. The *only* place a fractional scale needs to multiply in at all is the wgpu
surface's own literal pixel buffer size (`buffer_width`/`buffer_height`, computed locally,
never stored) — because NDC placement is a **ratio** (`origin / screen_size`), it comes
out identical whether both sides are expressed in points or pixels, as long as they agree.
The only other place scale matters is each damage tile's own *texture resolution*
(`side_px`) — genuinely higher-resolution on a >1x output, so the render doesn't blur.

**`wp_viewporter` is the other half of the trick.** `wp_fractional_scale_v1` only ever
*reports* a preferred scale — actually rendering at a different pixel size than the
surface's logical size and having the compositor scale between them needs
`wp_viewport::set_destination`, told (in points) what size the higher-resolution buffer
should be displayed at.

**Two trigger points for the same rebuild, on purpose.** A size change
(`LayerSurfaceConfigure`) and a scale change (`PreferredScale`) both need the damage/tile
grid rebuilt and the wgpu surface reconfigured, but they're different enough in what they
touch (a scale change never moves `gpu.width`/`gpu.height`, since those are points; a size
change never needs `PreferredScale`'s scale factor) that duplicating the ~20 lines in the
`PreferredScale` handler was less risky than restructuring `configure()`'s already-large
`&mut GpuLayer`-based body to share it.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

(unchanged — this chunk touched only `hakai`)

```bash
cd /mnt/mac/hakai
cargo build && RUST_LOG=info cargo run
```

If your monitor isn't fractionally scaled, this chunk should be invisible — everything
should look and behave exactly as before (scale stays `1.0`, same as always). If it *is*
fractionally scaled (check `hyprctl monitors` for a non-integer `scale`), the render
should now be crisp rather than blurry, tools/damage/HUD should land exactly under the
real cursor, and resizing/moving between outputs of different scales shouldn't misplace
anything.

## What "done" looks like

- [x] `hakai` builds with `wayland-protocols` in the dependency tree
- [x] On an integer-scaled output: no visible change at all from before this chunk
- [x] On a fractionally-scaled output: crisp render, cursor/tool/damage line up exactly
      with the real pointer position
- [x] Nothing else regressed (frozen mode, brightness-driven impact sounds, HUD, palette)

**Confirmed working** on a real run, twice: first at whatever integer scale was already
active ("works", clean on the first try — the design bug from the write-up above was
caught before ever reaching the compiler), then at a genuine fractional scale. Hyprland's
newer Lua config API dropped `hyprctl keyword monitor ...` (`keyword can't work with
non-legacy parsers. Use eval.`) — set live instead via
`hyprctl eval 'hl.monitor({ output = "eDP-0", mode = "preferred", position = "auto", scale = 1.25 })'`.
At a real 1.25x: "looks crisp" — no blur, no upscale artifacts, `wp_viewport`'s
destination/buffer split doing its job.

---

Phase 6 is complete: brightness-driven impact sounds (chunk 1), frozen mode (chunk 2),
and fractional-scale rendering (chunk 3) are all confirmed working. Nothing else known to
be open for this phase.
