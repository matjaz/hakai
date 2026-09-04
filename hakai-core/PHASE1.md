# Phase 1 — build & test walkthrough

Goal: `SeededRng` (SplitMix64), the tiled damage layer on `tiny-skia` pixmaps, and one
decal (the hammer crack) end to end — deterministic, unit-tested, headless. No window, no
audio, no compositor. See `OMARCHY-PORT.md` in the desktop-destroyer repo for why this
phase is scoped the way it is.

Unlike Phase 0, **this doesn't need Hyprland running** — it's plain, portable Rust, so
`cargo test` works the same whether you run it on the Omarchy box or anywhere else. Still
building it there since that's where the toolchain already lives; nothing here is
Wayland-specific.

## Build & test

```bash
cd ~/omarchy/hakai-core
cargo test
```

**Expect green.** Unlike Phase 0, there's no unverifiable OS-glue in this crate — it's
ordinary `tiny-skia` drawing calls, and I'm considerably more confident in this API surface
than in Phase 0's raw Wayland-pointer bridging. That said, I still can't compile it myself,
so treat any failure the same way as Phase 0: paste back the full `cargo test` output and
I'll fix it against the real error.

One thing genuinely flagged as uncertain in the code itself (search for `RISK` in
`src/damage.rs`): the exact way `tiny_skia::Pixmap::draw_pixmap` combines its `transform`
argument with its integer `(x, y)` argument. `stamp` sidesteps the ambiguity by always
passing `x=0, y=0` and folding all placement and scaling into `transform` — if a decal test
fails specifically on *coverage numbers* (not a compile error), that function is the first
place to look.

## What "done" looks like

- [ ] `cargo test` passes — expect roughly 20 tests: RNG determinism/bounds, tile geometry
      (5×3 grid, partial tile, one/two/four-tile intersections, edge and off-screen
      clamping), drawing/erase/commit behaviour, and crack decal determinism
- [ ] `cargo run --example dump_decals` writes 8 PNGs to `target/dump/` — open a couple and
      eyeball them: each should look like a starburst crack with a dark impact hole and a
      light rim, readable-looking rather than a blank or solid-black square
- [ ] The 8 crack variants are visibly different from each other, not near-duplicates

If all three hold, Phase 1 is done. Phase 2 (the bulk of the mechanical work — bullet
holes, scorch marks, paint splats, phaser hits, stamps, bites, blood, saw cuts, slivers,
plus the icon and sprite generators) builds directly on this crate's `DecalFactory` and
`DamageLayer`, so it's worth actually looking at the PNGs before moving on, not just
trusting green tests — a decal can pass every test and still look wrong.

## Debugging a failed coverage test

If `erasing_reduces_coverage` or `erase_all_clears_everything` fails while everything else
passes, that isolates the problem to `stamp`'s placement/scaling math rather than to the
crack generator itself (the crack's own determinism tests would fail first if it were
broken). Dump the intermediate state to a PNG to see it directly:

```rust
// temporarily, inside the failing test
layer.snapshot().save_png("/tmp/hakai-debug.png").unwrap();
```

then pull `/tmp/hakai-debug.png` and look at where the crack actually landed.
