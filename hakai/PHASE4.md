# Phase 4 — build & run walkthrough (chunk 1: the damage layer as real GPU textures)

Goal: replace Phase 0's flat translucent-red clear with an actual render pipeline — one
`wgpu::Texture` per damage tile, uploaded from `hakai_core::DamageLayer`'s CPU-side
`tiny-skia` pixels, drawn as a textured quad per tile. No real mouse input yet (next
chunk) — to have something recognisable to check, each output stamps one scripted crack
decal, dead centre, the moment it's configured.

This is the largest single block of genuinely new, unverified surface since Phase 0 —
expect more rounds of fixes than recent chunks, and expect to actually judge this one by
eye rather than by test output, since there's no way to headlessly verify "does this
actually render right."

## What changed structurally

`hakai` now depends on `hakai-core` via a relative path (`hakai-core = { path =
"../hakai-core" }` in `Cargo.toml`) — not a formal Cargo workspace, just a plain
cross-crate dependency. Simpler, and lower-risk to two already-working, separately-tested
setups than merging Cargo.locks / target-dirs would have been.

`hakai-core`'s `DamageLayer` gained a few accessors a renderer needs that nothing in
Phases 1–3 required: `tile_pixels(index)` (raw RGBA8 bytes + dimensions, ready to upload),
`dirty_indices()` (which tiles changed since the last `commit()`), `scale()`, and
`tile_origin()` was made `pub`. All four are covered by new `cargo test` cases — those
should already be green regardless of how the GPU side goes.

## RISK — five things flagged in the code, ranked by how likely each is to bite

1. **`queue.write_texture`'s destination-type names** (`main.rs`, `upload_dirty_tiles`).
   wgpu renamed `ImageCopyTexture`/`ImageDataLayout` to `TexelCopyTextureInfo`/
   `TexelCopyBufferLayout` at some point in its history close to `wgpu = "22"`, the
   version pinned here. If this doesn't compile, try the `TexelCopy*` names.
2. **`entry_point`'s type** in `create_pipeline` — `Option<&str>` here
   (`Some("vs_main")`), which might need to be a bare `&str` (`"vs_main"`) depending on
   exactly which `wgpu` 22.x patch resolves.
3. **The NDC/y-flip math** in `tile_ndc()` — wgpu's clip space is y-up; this port's scene
   convention is y-down (same underlying issue as `decals.rs`/`icons.rs`/`particles.rs`,
   now in the renderer). Should be provably correct by the same reasoning that worked for
   the icon rotations and the damage-layer stamp rotation, but this is the *first* time
   that reasoning has been applied to an actual GPU clip-space transform rather than a
   `tiny-skia` one, so treat it as unverified until you've actually looked at the screen.
4. **The non-sRGB swapchain format choice** (`configure`, `!f.is_srgb()`) — deliberate,
   not a guess: the tile textures are plain `Rgba8Unorm` with no gamma handling in the
   shader, so writing to an sRGB view would double up the encoding. If colours look
   washed out or oddly dark, this pairing (`Rgba8Unorm` tiles → non-sRGB swapchain) is the
   first thing to check.
5. Everything else (bind group layout, pipeline layout, sampler, the shader's own vertex
   generation) is textbook `wgpu` — much higher confidence than the four above.

## Build & run

```bash
cd /mnt/mac/hakai
cargo build
```

Fix whatever `cargo build` reports — likely candidates are #1 and #2 above. Once it
builds:

```bash
cargo run
```

**Expected:** on every monitor, a large crack decal (starburst arms, a dark impact hole,
concentric rings) centred on the screen, over your real desktop, correctly scaled and
*not* mirrored, rotated 90° off, or oddly stretched. If you see nothing but a blank/clear
overlay, or something clearly wrong (crack in the corner instead of centred, colours
inverted, only a partial shape) — that's #3 or #4 above; screenshot or describe what you
actually see and I'll fix it against that, the same way we debugged Phase 0's raw-pointer
bridge.

Esc quits, same as Phase 0.

## What "done" looks like

- [ ] `cargo build` succeeds (after whatever fixes #1/#2 need)
- [ ] `cargo run` shows one correctly-centred, correctly-scaled crack decal per monitor,
      over the real desktop
- [ ] The crack looks like the same shape `dump_decals`/`crack-0.png` produced back in
      Phase 2 — same starburst structure, not distorted
- [ ] Esc still quits cleanly

If that holds, this chunk is done and the next one wires up real mouse/keyboard input so
the tools from Phase 3 actually run against your input instead of a scripted stamp.

---

# Phase 4 — chunk 2: real input, driving the Phase 3 tools

Wayland pointer motion/press/release and keyboard tool-switching now drive the nine tools
from Phase 3, exactly the way `ToolSimulation` drove them synthetically — just from your
actual mouse instead of a scripted stroke. The scripted crack from chunk 1 is gone; the
overlay starts blank.

**Controls**: `1`–`9` select a tool, `R` clears the desktop, `Tab` cycles to the next
tool, `Esc` quits (unchanged). Click and drag with the left button to use whichever
tool's active — same as the real app's design.

## The one new architectural point

Each output now owns its **own** full set of tool instances, particle system, termite
colony and RNG — not shared across outputs. Wayland gives pointer focus to one surface at
a time, but a standing flame or a walking termite has to keep evolving on every output
regardless of focus, and every output's `frame()` callback calls that output's active
tool's `update()` every frame — if tools were shared, a second monitor would make every
tool's internal cooldown/repeat timer fire twice as often. Keyboard tool-switches still
broadcast to every output in lockstep (`select_tool`), so they all agree on *which* tool
is active even though each has its own instance of it.

## RISK

One new area, on top of chunk 1's four: **`PointerHandler`'s event shape** —
`PointerEvent { surface, position, kind }` and `PointerEventKind`'s variants (`Enter`,
`Leave`, `Motion`, `Press`, `Release`, plus `Axis` not handled here) are my
best-confidence reading of `smithay_client_toolkit::seat::pointer` at this pinned SCTK
version, not verified against a compiler. If `cargo build` complains inside
`impl PointerHandler for State`, that's the shape to check against the real error.

Keysym matching (`Keysym::_1`..`Keysym::_9`, `Keysym::r`) is lower-confidence than
`Keysym::Escape` was in chunk 0 (that one's already proven working) but the same *kind*
of risk, not a new one.

## Build & run

```bash
cd /mnt/mac/hakai
cargo build
```

Fix whatever comes up (`PointerHandler`'s shape is the most likely candidate). Then:

```bash
cargo run
```

**Expected**: a blank transparent overlay on every monitor, cursor hidden. Try each of:

- Click (Hammer, the default) → a crack appears under the cursor, roughly centred on
  where you clicked
- Click-and-drag with the Hammer → a new crack appears every time you've dragged far
  enough (≈70pt) since the last one
- Press `2` (chain-saw), click and drag → a continuous cut follows the drag path, not
  just a mark at the start and end
- Press `8` (termites), click a few times → small bugs appear and wander on their own
  even without further input
- Press `4` (flame-thrower), hold the click, release → flame-shaped marks (scorch, once
  Phase 2's decals are what's landing) appear at/near the cursor and keep evolving briefly
  after release
- Press `9` (washer), drag over existing damage → it erases
- Press `R` → the whole desktop clears
- Press `Tab` a few times → the active tool visibly changes (different mark shapes/sizes)
- `Esc` → quits cleanly

## What "done" looks like

- [ ] `cargo build` succeeds
- [ ] Every control above does what it says
- [ ] Marks land under the actual cursor position, not offset or on the wrong monitor
- [ ] With two monitors (if you have them): termites/flames on one screen keep animating
      while you're actively clicking on the other — confirms the per-output state design
      actually works, not just compiles

If that holds, chunk 2 is done. Remaining Phase 4 work: the sprite atlas (termites,
flames, shells, droplets — currently invisible even though the *particles* and *termite
positions* are already being tracked correctly, since only damage-layer tiles render so
far), the cursor icon, and the HUD.

---

# Phase 4 — chunk 3: the cursor icon

The current tool's icon (`hakai_core::icons::ToolIcons`) now renders at the cursor,
hotspot-aligned, hidden entirely unless that output actually has pointer focus. The
chain-saw and stamp swap to their alternate ("cutting"/"pressed") icon while the button is
held — an instant swap, not the original's animated transition, same scope trade-off as
everywhere else "presentation" has been deliberately simplified in this port. The hammer
always shows its baked-in resting angle — no swing-on-strike animation yet.

**Lower risk than chunks 1–2**: this reuses the exact same shader, pipeline and
bind-group layout already proven for damage tiles (the uniform shape is identical — one
origin, one size, in clip space). The only genuinely new piece is a *dynamic* uniform
buffer, rewritten via `write_buffer` every frame since the cursor moves and a tile's
placement doesn't — safe to reuse because there's exactly one write and one read per
frame, not several tiles' worth of data racing into the same buffer before any of their
draws run (the concern that would have mattered).

## Build & run

```bash
cd /mnt/mac/hakai
cargo build && RUST_LOG=info cargo run
```

Expect an `uploaded 11 cursor icon variants` log line near startup. Then:

- The current tool's icon should follow your mouse, its hotspot (not its centre) tracking
  the cursor position — e.g. the hammer's striking face, not the middle of its bounding
  box, should sit exactly where clicks land
- Switching tools (`1`–`9`) should visibly change which icon follows the mouse
- Holding the chain-saw's button down should swap it to the "cutting" icon; releasing
  swaps it back. Same for the stamp's "pressed" icon
- With two monitors: the icon should only appear on whichever one currently has the
  pointer, not both

## What "done" looks like

- [ ] `cargo build` succeeds
- [ ] All of the above holds
- [ ] The icon's hotspot alignment is visibly correct, not offset from where marks
      actually land

If that holds, this chunk is done. Remaining Phase 4 work: the sprite atlas (termites,
flames, shells, droplets — tracked correctly, still invisible) and the HUD.

---

# Phase 4 — chunk 4: termites (first of the sprite atlas)

Termites now render — walking, wandering, chewing, all as before, just visible now. This
is chunk 1 of "the sprite atlas"; flames and particles (shells, droplets, slivers) are a
follow-up, since `ParticleKind::Generic` particles currently have no stored texture
reference at all (Phase 3 never needed one) — real design work for its own round, not
bolted on here.

## Two real gaps found and fixed in `hakai-core` first

Phase 3 exposed only *counts* (`TermiteColony::count()`, `ParticleSystem::count()`,
`FlameThrower::active_flame_count()`) — there was no renderer yet, so no reason to expose
positions. Fixed by adding `TermiteColony::iter() -> impl Iterator<Item = TermiteView>`,
`ParticleSystem::iter() -> impl Iterator<Item = ParticleView>`, and
`FlameThrower::flames() -> impl Iterator<Item = FlameView>`.

Along the way, a second real gap: `Particle::spin` (radians/second) was stored but never
integrated into an actual rotation angle — Swift accumulates `sprite.zRotation += spin *
dt` every frame; this port's `update()` only ever advanced position and age. Fixed by
adding a `rotation: f32` field, accumulated the same way. Covered by a new test
(`rotation_accumulates_from_spin_over_time`).

## Data flow — different from tiles and the cursor, on purpose

A termite's position/heading/frame changes every frame, and there can be up to 500 of
them at once — sharing *one* dynamic uniform buffer across many termites drawn in the same
submission would hit exactly the ordering problem tile uploads were built around avoiding
in the first place (every `write_buffer` call would land before any of the draws that
were supposed to see it individually, so every termite would render with whichever one's
data was written last). So each termite gets a **fresh** uniform buffer and bind group,
created and dropped within the same frame. Simple and correct; not fast — flagged in the
code as a known, deliberate performance trade-off to revisit if 200+ termites visibly
stutters, not a correctness concern for this chunk.

**No rotation yet.** Termites always render at their sprite's baked-in orientation,
regardless of which way they're actually walking — the shared `Tile` shader/uniform is
axis-aligned only, and adding rotation would mean extending it, which nothing else
(tiles, icons) currently needs. Noted, not hidden — same treatment as every other
deliberate scope cut in this port.

## RISK

One thing flagged in the code, more of a "watch for this" than a strong doubt: the
per-termite buffer and bind group are dropped at the end of each loop iteration, before
`queue.submit()` runs. This relies on `wgpu`'s resource handles being internally
reference-counted — the same assumption this file already leans on for `TextureView`s
outliving the `Texture` they're built from. If termites don't compile, or compile but
render as garbage, this is the first place to check.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

(covers the three new accessors — should be a clean, low-risk pass on its own, worth
confirming before the GPU build)

```bash
cd /mnt/mac/hakai
cargo build && RUST_LOG=info cargo run
```

Drop some termites (`8`, then click a few times) and watch them wander. Everything from
chunk 2 (they persist across tool switches, get killed by flame/stamp, blocked-not-killed
by the washer) should look the same as before — just visible now.

## What "done" looks like

- [ ] `hakai-core`'s `cargo test` passes
- [ ] `hakai`'s `cargo build` succeeds
- [ ] Termites are visible, positioned correctly (roughly where they were dropped, then
      wandering), and their 2-frame walk animation is visible up close
- [ ] Dropping many termites (try 30–50) doesn't visibly stutter — if it does, that's the
      per-frame-allocation trade-off noted above, worth flagging back rather than treating
      as expected

If that holds, this chunk is done. Remaining sprite atlas work: flames and particles.

---

# Phase 4 — chunk 4b: termite rotation

Feedback from the real run: termites were visible and correctly positioned, but read as
"sliding" rather than walking — the underlying path does meander (`heading` random-walks
every frame, same formula as Swift), but since the sprite itself never rotated to face
that heading, the wiggle wasn't visually legible. Fixed by actually rotating the termite
sprite to face its heading, which also let the bite-mark fix from earlier become simpler
and more correct: instead of a fixed left-offset (which only reads as "the head" while the
sprite is unrotated), it's now `head_dist * (cos(heading), sin(heading))` — the head's
actual position now that the sprite turns with it.

**New GPU surface, reasoned through by hand rather than assumed**: rotating in clip space
(NDC) would shear a sprite, since a screen's width and height generally differ — a
rotation matrix applied directly to NDC coordinates doesn't know about that anisotropy
unless corrected for it. So rotation happens in *pixel* space instead (`rotated_sprite_ndc`
in `main.rs`), where x and y are the same unit, and only the four already-rotated corners
get converted to clip space afterward — the shader (`vs_sprite` in `shader.wgsl`) never
does any rotation math at all, just picks one of four precomputed corners per vertex.

This needed a **second pipeline**: tiles/icons stay on the original axis-aligned
`vs_main`/`fs_main` (a `Tile` uniform: origin + size), termites move to the new
`vs_sprite`/`fs_sprite` (a `RotatedSprite` uniform: four corners). Both live in one shader
module, compiled once, in `create_pipelines`. The rotation direction itself needed care:
`SpriteFactory::termite`'s head art sits on the sprite's own *left* (local -x), so facing
`heading` means rotating by `heading + π`, not `heading` — worked out algebraically (see
the comment at the call site) rather than guessed.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

```bash
cd /mnt/mac/hakai
cargo build && RUST_LOG=info cargo run
```

Drop termites and watch them for a few seconds — they should visibly turn as they wander,
not just slide, and the chewing mark should track the head (now the point the sprite is
actually facing) as it turns.

## What "done" looks like

- [ ] `hakai-core`'s tests pass, `hakai` builds
- [ ] Termites visibly rotate to face their direction of travel as they wander
- [ ] The bite mark still lands at the head, now correctly through turns too — not just
      when facing the original fixed left direction
- [ ] Nothing else (tiles, cursor icons) regressed — they're still axis-aligned and
      unaffected, since they stayed on the original pipeline

If that holds, this chunk is done. Remaining sprite atlas work: flames and particles.

---

# Phase 4 — chunk 4c: machine-gun shells and standing flames

Feedback from the real run: "missing machine gun empty shells falling. also missing
flame. the damage shadow is there." — the scorch mark a flame leaves behind (a damage
tile decal) was rendering correctly since chunk 1; the flame itself, and the shells
`ParticleSystem` was already tracking headlessly since Phase 3, had nothing drawing them.

**The trait-object hurdle.** Termites and particles are `ToolContext`-owned, so the
renderer already has a direct `&ParticleSystem`/`&TermiteColony` to iterate. A standing
flame isn't — `FlameThrower` owns its own flame list, matching the Swift original, and the
renderer only ever sees it as `gpu.tools`'s type-erased `Box<dyn Tool>`. Fixed with the
standard downcast hook: `Tool::as_any(&self) -> &dyn std::any::Any { self }`, added as a
non-default trait method (so every one of the nine tools has to implement it — a
deliberate compile-time nudge, not an oversight) — `gpu.tools.get(&ToolId::FlameThrower)
.and_then(|t| t.as_any().downcast_ref::<FlameThrower>())` gets back a concrete
`&FlameThrower` to call its `flames()` accessor on.

**Shells and paint droplets** both ride `ParticleSystem::iter()`'s existing `ParticleView`
(position, rotation, size, kind) — `ParticleKind::Shell` picks the one shared shell
texture, `ParticleKind::Droplet { color_index }` indexes into eight droplet textures (one
per `DecalFactory::PAINT_COLORS` entry). `ParticleKind::Generic` (hammer slivers,
chain-saw sawdust) still has no texture of its own — explicitly still deferred, not fixed
here.

**Flames needed two pieces of data `FlameView` didn't carry yet**: a per-flame spawn-time
size multiplier (`scale`, matching Swift's `ctx.rng.range(0.62, 1.1)`) and raw `age` (to
drive the 0.07s-per-frame walk animation the Swift original ran as an `SKAction`). Both
added to `hakai-core`'s `Flame`/`FlameView` — small, deliberate extensions, the same move
already made once for `life_fraction` back when this tool was first ported with no
renderer to feed.

**Additive blending, a genuinely new GPU concept this chunk.** `FlameThrower.swift` sets
`flame.blendMode = .add` — "fire glows, it does not cover." The existing `sprite` pipeline
uses regular alpha blending (`SrcAlpha`/`OneMinusSrcAlpha`), which is correct for termites
and shells (opaque-ish objects that should occlude each other) but wrong for flames
(overlapping flames should brighten, not cut into each other). Rather than branch on blend
mode per-draw (blend state is baked into a `wgpu::RenderPipeline` at creation, immutable
after that), added a third pipeline, `sprite_additive` — same `vs_sprite`/`fs_sprite`
shader entry points and bind-group layout as `sprite`, differing only in blend state
(`SrcAlpha`/`One` on color, `One`/`One` on alpha). `State::render` switches to it just for
the flame-drawing loop and switches back to the regular `sprite` pipeline for termites.

**A per-instance alpha, added to `RotatedSprite`.** Flames fade out over their last 20% of
life (`alpha = t > 0.8 ? max(0, (1 - t) / 0.2) : 1` in Swift) — a number with nothing to do
with a quad's placement, so it doesn't belong among the four corners `RotatedSprite`
already carried. Added an `alpha: f32` field to both the WGSL struct and its Rust
counterpart; `fs_sprite` now multiplies the sampled texture's alpha channel by it.
Everything that isn't a flame (termites, shells, droplets) just passes `1.0`. The Rust
struct pads out to 40 bytes explicitly (`_pad: f32`) rather than trusting Rust's own
`repr(C)` rounding (36, natural alignment 4) to happen to match what WGSL's uniform
address-space layout rules compute for the same struct (40, alignment 8, from its
`vec2<f32>` members) — a buffer backed by fewer bytes than the shader expects to read
would be undefined behavior, so this pads to the larger of the two rather than relying on
the two languages' rounding agreeing by coincidence.

**A flame's anchor isn't its centre.** `FlameThrower.swift`'s sprite has `anchorPoint =
CGPoint(x: 0.5, y: 0.10)` — a flame grows upward from its base, so 90% of its height sits
above the spawn point, not centred on it. `rotated_sprite_ndc` only knows how to place a
quad by its own centre, so `main.rs` shifts the centre up by `0.40 * height` before calling
it (worked out by hand from the anchor fraction — see `FLAME_ANCHOR_TO_CENTER_Y`'s doc
comment) rather than teaching the shared placement function about arbitrary anchors for
just this one caller.

**Not ported**: the flame's small `zRotation` jitter (`ctx.rng.jitter(0.16)`, a few degrees
either way) — that random value isn't stored anywhere retrievable from `FlameView` today,
and the effect is minor enough not to be worth adding a third field for on its own. Flagged
here rather than silently dropped.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

(covers the two new `Flame`/`FlameView` fields and the `as_any` additions across all nine
tools — should be a clean, low-risk pass; no behavior changed, only new accessors)

```bash
cd /mnt/mac/hakai
cargo build && RUST_LOG=info cargo run
```

Switch to the machine gun (`3`) and hold the mouse down — brass shells should eject and
fall, each with a visible highlight, landing with a sound. Switch to the flame-thrower
(`4`) and hold — standing flames should appear where the cursor is, glow (brighten where
they overlap rather than occlude), flicker through their walk frames, spread/fade near the
end of their life, and leave a scorch mark when they go out. Switch to the color-thrower
(`5`) and throw paint — flying droplets should now be visible in flight, not just their
eventual splat.

## What "done" looks like

- [ ] `hakai-core`'s tests pass, `hakai` builds
- [ ] Machine-gun shells are visible falling and landing
- [ ] Standing flames are visible, glow additively, animate, and fade out near the end of
      their life
- [ ] Paint droplets are visible in flight before they splat
- [ ] Nothing else (tiles, termites, cursor icons) regressed

If that holds, this chunk is done. Remaining sprite atlas work: `ParticleKind::Generic`
(hammer slivers, chain-saw sawdust) still has no texture; the HUD (tool label, hint text,
`cosmic-text`) is the last piece of Phase 4 after that.

**Follow-up, found during Phase 6 — flames should keep burning down after a tool switch.**
User feedback in two rounds: first "flame does not glow when i change to other tools",
then, once the animation was fixed, "it animates, but doesn't move and fades."

The frame-cycling calculation above (`flame.age`) freezes once you switch away from the
flame-thrower, because `advance_flames` — the only thing that advances `flame.age` — only
ever ran from `FlameThrower::update`, itself only called on the *active* tool (matching
`GameScene.swift`'s `active?.update(...)` exactly). Reading `FlameThrower.swift` closely
explained why the original doesn't visibly freeze the same way: its walk-frame cycling
isn't part of `advanceFlames` at all — it's a plain
`flame.run(.repeatForever(.animate(with: frames, timePerFrame: 0.07)))` `SKAction`
attached directly to the flame's own sprite node at spawn, which SpriteKit's renderer keeps
running every frame regardless of which tool is active, decoupled from `FlameThrower.update`
entirely. A first pass fixed only that: a new `GpuLayer.render_clock`, advanced
unconditionally every frame, driving the frame-index calculation in place of `flame.age`.

That surfaced the second round of feedback: a flame's actual *life* — drift, spread,
fade-out, expiry, the scorch mark it leaves — genuinely does freeze in the real macOS app
too while another tool is selected (verified directly in `GameScene.swift`: termites get
an explicit, commented always-on `termites.update(dt:...)` there — *"The colony updates
always, not only while the termite tool is selected"* — but flames get no such exception).
Presented as an explicit choice rather than assumed: match the original's freeze, or give
flames the same always-on treatment termites already have. Chosen: **always burn down**,
a deliberate departure from the original, since a fire pausing mid-burn because the player
picked a different tool reads as a bug to a player even though it's a faithful port of
what's arguably just an oversight in the original.

Implemented by calling `FlameThrower::update` a second time from `State::advance` whenever
it *isn't* the active tool (the active-tool branch already calls it once) — safe because
`deactivate` always resets `burning` to `false` first, so the emit-a-new-flame branch
inside never fires from this second call; only `advance_flames` (called unconditionally at
the bottom of `update` either way) actually does anything. This also let the `render_clock`
workaround be reverted: since `flame.age` now always advances, the frame-cycling
calculation went back to using it directly, restoring each flame's own independent
spawn-relative animation phase (closer to Swift's per-node `SKAction` than the shared-clock
version was) as a side effect, not just a fix for the freeze.

**Confirmed working** on a real run ("works") — flames now flicker, drift, spread, fade
out and leave a scorch mark no matter which tool is active.

---

# Phase 4 — chunk 4d: the hammer's knock animation

Feedback from the real run: "missing hammer animation." The `tools` module doc comment
had flagged this from the start — every tool's `SKAction`-driven cursor animation (the
hammer's knock, the chain-saw's shake, the machine gun's recoil) was deliberately left
out of `hakai-core` back when there was no renderer to feed, on the grounds that it's pure
presentation. This chunk recovers the hammer's.

**Recomputed from elapsed time, not run once.** `Hammer.swift`'s `animateKnock` runs a
one-shot `SKAction` (`rotate(toAngle:duration:shortestUnitArc:)`, ease-out) on strike.
Rather than any kind of imperative "animation system," `hakai-core::Hammer` got a new
`since_strike: f32` clock and a pure `cursor_rotation(&self) -> f32` method: snap to
`IMPACT_ROTATION` (0) when `since_strike` is reset to 0 on every strike, ease back out to
`RAISED_ROTATION` (-0.42) as `since_strike` climbs past `KNOCK_DURATION` (0.16s), clamp
there once it's past. Fully deterministic and headlessly testable, same as everything else
in this crate — no scene, no timer, no action queue.

**A second, independent clock — not a reuse of the existing repeat-strike timer.**
`since_last_hit` already existed (gates the hold-to-repeat mechanic) but deliberately
*stops* advancing while the mouse button is up — reusing it for the knock animation would
have frozen a single click's cursor mid-swing forever the instant the button came back up,
since the animation would never get the chance to finish. `since_strike` advances
unconditionally in `update()`, before the `is_down` check.

**The cursor's placement architecture had to generalize from a fixed hotspot to an
arbitrary rotating pivot.** `Hammer.swift`'s cursor is a container node at the mouse, with
the hammer sprite inside anchored at its *grip* (`pivot`), offset so that at zero rotation
the *striking face* (`hotspot`) lands exactly on the mouse — the handle holds still while
the head swings. `hakai-core::icons::ToolIcon` already carried `pivot: Option<(f32, f32)>`
from Phase 1 (`None` for every tool but the hammer), but the renderer had only ever used
`hotspot`, and only ever drawn cursors axis-aligned via the tile pipeline. Fixed by moving
the cursor onto the rotatable-sprite pipeline unconditionally (the zero-rotation,
pivot-equals-hotspot case other tools always hit is identical to the old axis-aligned
placement — worked through algebraically, not assumed) and adding
`rotated_sprite_ndc_pivot`, a generalization of the termite/particle/flame placement
function (`rotated_sprite_ndc`) that rotates around an arbitrary point within the sprite
instead of always its centre.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

(covers `Hammer::cursor_rotation`'s snap-then-ease behavior, including that it keeps
animating — and finishes — after the button is released)

```bash
cd /mnt/mac/hakai
cargo build && RUST_LOG=info cargo run
```

Select the hammer (`1`) and click. The head should snap onto the impact instantly (in the
same frame as the crack and the sound) and then visibly lift back out over the next beat,
with the handle staying roughly put while the head travels the arc — not the whole icon
just rotating around its own centre.

## What "done" looks like

- [ ] `hakai-core`'s tests pass, `hakai` builds
- [ ] The hammer visibly knocks on every strike and eases back to its raised pose
- [ ] The knock finishes even after a single quick click (button already released)
- [ ] Every other tool's cursor is unaffected — still lands its hotspot exactly on the
      mouse, doesn't rotate

If that holds, this chunk is done. The chain-saw's shake and the machine gun's recoil are
the same kind of gap, not yet picked up. Remaining Phase 4 work otherwise unchanged:
`ParticleKind::Generic`, then the HUD.

**Follow-up, same chunk — the real bug.** Real-run feedback was "not really [natural]...
should move more naturally, longer movement," which the first response to (doubling
`KNOCK_DURATION`, a cubic ease-out) didn't actually fix — "still rough" — because timing
was never the problem. `RAISED_ROTATION`/`IMPACT_ROTATION` are `Hammer.swift`'s own
`zRotation` values, ported as raw numbers — but `zRotation` is SpriteKit's y-up,
counter-clockwise-positive convention, and this port's cursor rotation
(`rotated_sprite_ndc_pivot`) runs in y-down pixel space. Feeding the same numeric angle
into a y-down rotation mirrors it vertically — for a plain centred rotation that would
just look "backwards," but the hammer rotates around an *off-centre* pivot (the grip, not
the icon's middle), so a mirrored rotation swings the head through a visually wrong arc
entirely. That's what "rough" actually was. Same class of bug this whole port has hit
repeatedly (particle gravity/launch velocities, decal drips, the termite's own facing
direction) — fixed the same way, negating once at the boundary
(`Hammer::cursor_rotation()`) rather than at every caller — and reverted the speculative
timing changes back to the literal Swift values (`0.16`s, quadratic ease-out) once the
actual bug was found, rather than leaving an unrelated deviation stacked on top of it.

---

# Phase 4 — chunk 4e: finishing the sprite atlas — slivers, machine-gun kick/flash, chain-saw shake

Closes out the three items left over from chunk 4c/4d: `ParticleKind::Generic`'s missing
texture, and the chain-saw's/machine gun's own cursor animations (flagged as deferred back
in chunks 4c and 4d, same as the hammer's knock originally was).

**A real bug found along the way, independent of any of the above.** `ParticleSystem::emit`
initialized every particle's `rotation` to `0.0`; `ParticleSystem.swift`'s original
actually does `sprite.zRotation = spin` — `spin` doubles as *both* a particle's starting
angle and its angular velocity, an unusual but deliberate choice in the source. Phase 3
had no sprite to show the discrepancy on, so it went unnoticed until this chunk needed
particles to render correctly from spawn. Fixed in `particles.rs`, affecting every
particle kind at once (slivers, sawdust, shells, droplets), not just the ones this chunk
was nominally about.

**`ParticleKind::Generic` gained a `variant: i64`.** Both the hammer's slivers and the
chain-saw's sawdust ride the same `DecalFactory::sliver(variant)` shape in the Swift
original — a decal generator doing double duty as a live-particle texture, the same way
`DecalFactory::paint_splat` already did for droplets. Six sliver textures, cached once
like every other sprite atlas entry.

**The chain-saw's shake needed its underlying gameplay logic, not just presentation.**
`ChainSaw.swift`'s `isRevving` — which drives *both* the idle/cutting sound crossfade and
the cursor's shake amplitude — is computed from mouse *movement* (`advanceRevving`, a
1-second hold after the last frame that moved more than 0.5pt), not from whether the
button is held. The existing `hakai-core` port had simplified this to a button-gated rule,
which happened to look similar in the common case (you're usually moving while dragging)
but wasn't actually what the source does — waving the saw around without clicking should
still rev it. Ported the movement-tracking faithfully rather than inventing a
shake-while-cutting shortcut to match the existing (wrong) simplification.

**The machine gun's muzzle flash isn't a `ParticleSystem` particle, deliberately.**
`MachineGun.swift`'s `spawnFlash` builds a raw `SKSpriteNode` with its own one-shot
scale/fade action, never going through `ParticleSystem.emit` at all. Matched that
structurally: `MachineGun` now owns a small `flashes: Vec<Flash>` list with a
`FlashView`/`flashes()` accessor, exactly like `FlameThrower`'s standing flames — not
`ToolContext`-owned, reached by the same `as_any` downcast. Drawn additively (`.blendMode
= .add`, "fire glows, it does not cover" — same reasoning as the flame), scaling 1→1.5 and
fading 1→0 over its 0.09s life, both driven by a `life_fraction` the same shape as
`FlameView`'s.

**The recoil kick and the shake both needed the same y-up→y-down fix already found for the
hammer.** `MachineGun::cursor_rotation` and `ChainSaw::cursor_rotation` are both raw
`zRotation` values from their respective Swift sources, so both get the same negation at
their own boundary this chunk found necessary for the hammer — applied proactively this
time, not after a second round of "still rough" feedback. (The flash's own random
rotation is the one exception: a uniformly random full-circle angle on a 90°-symmetric
shape looks identical mirrored or not, so it's passed through unflipped, with a comment
explaining why rather than silently doing nothing.)

**Cursor rotation dispatch generalized.** The inline `if active_tool == Hammer {...}
else {0}` from chunk 4d became `active_cursor_rotation`, matching on all three animating
tools (hammer, machine gun, chain-saw) and downcasting to each's own concrete type —
still `0.0` (no rotation) for the other six, and still safe by construction: every
non-animating tool's `pivot == hotspot`, so the placement math is a no-op regardless of
what `rotation` evaluates to.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

(covers the rotation-init fix, the three tools' new animation/revving state, and the
`Generic` variant plumbing — a good-sized pass, worth reading the full `cargo test`
output rather than just the pass count)

```bash
cd /mnt/mac/hakai
cargo build && RUST_LOG=info cargo run
```

Hammer (`1`): unaffected by this chunk, should still look like chunk 4d left it. Chain-saw
(`2`): select it and just *move the mouse around* without clicking — the cursor should
start vibrating noticeably; stop moving and it should settle to a faint idle tremor after
about a second. Machine gun (`3`): hold the trigger — each shot should kick the cursor and
flash briefly at the impact point, brightening rather than just appearing. Hammer/chain-saw
sparks should now be visible tumbling splinters, not invisible.

## What "done" looks like

- [ ] `hakai-core`'s tests pass, `hakai` builds
- [ ] Hammer slivers and chain-saw sawdust are visible as small tumbling splinter sprites
- [ ] The chain-saw cursor vibrates on movement alone (no click needed), settling after
      ~1s of stillness
- [ ] The machine gun's cursor kicks on every shot and a brief additive flash appears at
      each impact
- [ ] Nothing else (tiles, termites, flames, shells, droplets, the hammer) regressed

If that holds, all of Phase 4's sprite/cursor work is done except the HUD (tool label,
hint text, `cosmic-text`) — the last piece of Phase 4.

**Confirmed working** on a real run ("working").

---

# Phase 4 — chunk 4f: the HUD, part 1 — pure-logic state (`hakai-core`)

The Swift HUD turned out to be four pieces, not one: a top tool-name label, a transient
toast (e.g. "Desktop cleaned"), a full clickable 9-cell tool palette (↑/↓ to open, click a
cell to select), and a separate acknowledgements/credits panel (`C` to toggle) — the last
one exists to satisfy a real licence obligation (23 of the 35 bundled sounds are CC BY
4.0, which requires attribution reachable *from inside the running app*, not just a
`CREDITS.md` in the repo). User chose to build all four now rather than a smaller slice.

This chunk is the part that doesn't touch a GPU or a font: two new `hakai-core` modules.

**`hud.rs`** — visibility and fade timing for the palette, the toast, and the credits
panel, ported from `ToolPaletteHUD.swift`'s `isVisible`/`setVisible`/`flashIfHidden` and
`GameScene.swift`'s `flash(_:)`. Same move as `Hammer::cursor_rotation`: every `SKAction`
fade becomes a small enum plus a clock advanced by `advance(dt)`, recomputed as a pure
function of elapsed time rather than run once. One deliberate simplification, flagged in
the module doc comment: `SKAction.fadeAlpha` animates from whatever alpha a node is
*actually* at when a new fade interrupts an old one; reproducing that exactly needs a
continuous alpha tracked across every interruption, so this always restarts a fade from
empty instead — the same trade this port already made for the machine gun's recoil kick,
and just as unlikely to matter (a person would have to hit ↑ and ↓ within 140ms).

**`credits.rs`** — the acknowledgements text itself, ported from `CreditsPanel.swift`'s
`build(bank:columns:)`. Reads a *new* bundled file, `hakai-core/assets/manifest.json` —
copied straight from the macOS app's `Resources/manifest.json` (same 35-sound metadata:
license, author, source, modification notice), embedded via `include_str!`. This only
needs the manifest's metadata, not real audio playback, so it doesn't have to wait for
Phase 5 — and Phase 5's real `SoundBank` can read the very same bundled file later without
the two drifting apart. `serde`/`serde_json` are new dependencies, added for exactly this
one parse. The text-wrapping logic (`packed`/`paragraph`, license grouping, the
`shortTitle` cleanup) is a straight port; the one adaptation is that `columns` (the
monospaced-font character budget) comes in as a parameter rather than being computed here
— `CreditsPanel.swift` computes it from `NSFont.maximumAdvancement.width` before calling
`build`, and the equivalent (an actual embedded font's metrics) doesn't exist in this crate
— it'll be `hakai`'s job, in part 2.

Neither module has any rendering, font, or GPU code yet — that, plus the actual key
bindings (↑/↓/C) and the palette's click hit-testing, is part 2, in `hakai`'s `main.rs`,
once `cosmic-text` is in the picture.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

Nothing to run yet — `hakai` doesn't reference either new module. This is purely to
de-risk the logic (and the manifest parse) before touching anything GPU/font-related,
which can't be verified without a real run.

## What "done" looks like

- [ ] `hakai-core`'s tests pass, including `credits.rs`'s manifest-parsing and
      column-wrapping tests and `hud.rs`'s fade-timing tests
- [ ] `cargo build` on `hakai-core` alone succeeds with the new `serde`/`serde_json`
      dependencies

**Confirmed working** on a real run ("tests pass").

---

# Phase 4 — chunk 4g: the HUD, part 2 — cosmic-text and the status bar

Wires part 1's `hakai-core` state to actual pixels: a new `text.rs` in `hakai` (the
renderer), and the always-on status bar — the panel, the tool-name label, the hint text,
and the toast (`"Desktop cleaned"` on `R`). The clickable 9-cell palette and the credits
panel are still one more chunk out; this one exists specifically to prove `cosmic-text`
works in isolation before building either on top of it.

**This is the highest-risk single piece of the whole port so far — more than the raw
Wayland pointer bridging in Phase 0.** Everything else new in this port has been either a
stable, well-documented API (`wgpu`, `smithay-client-toolkit`) or a small enough surface to
verify by reading `hakai-core`'s own already-working use of it (`tiny-skia`, `ttf-parser`).
`cosmic-text`'s convenience API — `FontSystem`'s constructors, `Buffer::draw`'s per-pixel
callback shape, `Color`'s accessors — has genuinely shifted across its `0.1x` releases, and
there's no way from here to check the exact shape for whatever version resolves. `text.rs`
is written at my best-confidence reading of it, flagged accordingly, and is very much
expected to need at least one round of "read the real compiler error, fix the real API" —
same discipline as everywhere else in this port that carried real uncertainty, just with
higher odds of actually needing it this time.

**Two embedded fonts, matching `hakai-core`'s own precedent.** `Hammer.swift`'s
`ArchivoBlack-Regular.ttf` (embedded font, OFL, `assets/fonts/` + `ATTRIBUTION.md`) already
established the pattern; `hakai/assets/fonts/` now has `JetBrainsMono-Regular.ttf` and
`JetBrainsMono-Bold.ttf` (also OFL, from Google Fonts) the same way, standing in for the
Swift original's `NSFont.monospacedSystemFont`. Two weights, not one, since the HUD
actually varies weight where the stamp decal never did — though for now every label here
uses the regular weight only (`Hammer.swift`'s own HUD text is all `.medium`, a weight
this port didn't bother sourcing a third file for; `Weight::BOLD` is wired and ready for
the palette's key-digits in the next chunk).

**A GPU element whose pixel size isn't known ahead of time.** Every other texture in this
renderer (a tile, an icon, a sprite) has a size fixed before it's ever drawn; a text
label's depends on the string. `HudGpu` carries its own `width`/`height` alongside the
texture for exactly this reason, and `draw_hud_element` places it by an *anchor point*
(`(0, 0.5)` for left-aligned-and-vertically-centred, ...) rather than a raw origin — the
same thing `SKLabelNode`'s `horizontalAlignmentMode`/`verticalAlignmentMode` did in Swift.
Getting "vertically centred" to actually mean something without carrying font metrics out
of `text.rs` took a specific design choice: `TextRenderer::rasterize` shapes each line
*twice* — once just to measure the bounding box of every pixel `cosmic-text` actually
drew, once more into a pixmap cropped to exactly that box — so a rasterized label's own
pixmap centre reliably *is* its visual centre, by construction.

**Cached, not rebuilt every frame.** A label's text changes rarely (a tool switch, a new
toast) — `ensure_hud_text`/`ensure_toast_gpu` compare against the string a cached texture
was last built from and skip the whole shape/rasterize/upload/bind-group cycle when
nothing's changed, the same "rebuild only when the input state actually differs" discipline
already used for cursor icon variants.

**The bar/label/hint are always visible; only the toast fades.** `GameScene.swift` never
hides `buildHUD`'s own nodes — only the separate `ToolPaletteHUD` and `CreditsPanel` panels
toggle. So the bar/label/hint stay on the plain tile pipeline (no alpha needed), and only
the toast — which genuinely fades via `hud.toast()`'s `life_fraction` — moved onto the
alpha-capable rotated-sprite pipeline instead (at zero rotation), rather than adding an
unused alpha field to `TileUniform`/`fs_main` for everyone else's sake.

**One deliberate simplification, flagged for later:** the bar is drawn as a plain rect,
not `Hammer.swift`'s `SKShapeNode(cornerRadius: 10)` rounded one — tiny-skia's
`PathBuilder` has no rounded-rect primitive, and the helper that already solves this
(`hakai_core::geometry::push_rounded_rect`) isn't reachable from outside `hakai-core`. A
real, minor visual gap, not an oversight.

**One real fix to something part 1 got wrong:** `spawn_layer_for_output` sets a fresh
`GpuLayer`'s `active_tool` to `Hammer` directly, never through `select_tool` — so nothing
would otherwise have built the tool-name label before the user's first tool switch. Built
once, explicitly, alongside the panel and hint, using the known startup tool.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

(no changes since part 1 — this is just confirming nothing regressed)

```bash
cd /mnt/mac/hakai
cargo build && RUST_LOG=info cargo run
```

If it doesn't compile, paste back the *first* error — with `cosmic-text` in the mix now,
don't assume the fix is obvious from the message alone; `text.rs` is where to look first.
If it does: a dark rounded... well, square-cornered bar should sit near the bottom of the
screen, reading "1 · Hammer" on the left and the hint text on the right. Switch tools
(`1`–`9`, Tab) and the label should update. Press `R` to clear — a "Desktop cleaned" toast
should appear above the bar and fade out after about 1.8s.

## What "done" looks like

- [ ] `hakai` builds with `cosmic-text` in the dependency tree
- [ ] The status bar is visible: panel, "N · ToolName" label, hint text
- [ ] The label updates on every tool switch
- [ ] `R` shows a fading "Desktop cleaned" toast
- [ ] Text is legible — right font, roughly the right size, not garbled or mis-cropped
      (a `cosmic-text` API mismatch that happens to still compile would most likely show up
      here, not as a build failure)

If that holds, this chunk is done. Remaining Phase 4 work: the clickable 9-cell palette
(↑/↓ to open, click to select) and the credits panel (`C` to toggle) — both build directly
on this chunk's `HudGpu`/`ensure_hud_text` machinery, now that it's proven working.

**Confirmed working** on a real run ("works") — status bar, tool-name label, hint text and
the toast all render correctly. `cosmic-text` needed no further fixes beyond the one
`credits.rs` test that had asserted an invariant the content never actually promised (a
fixed line ported straight from `CreditsPanel.swift` that neither app wraps).

---

# Phase 4 — chunk 4h: the clickable tool palette

The headline HUD feature: `↑`/`↓` opens and closes a 9-cell grid (icon + key digit per
tool), a click on a cell selects that tool, and any keyboard tool switch briefly flashes it
open even while closed. The credits panel (`C` to toggle) is still one more chunk out —
scoped out on purpose to keep this one reviewable on its own, the same split as the HUD's
"part 1 / part 2" before it.

**Everything here rides the alpha-capable rotated-sprite pipeline, at zero rotation.**
Unlike the always-on status bar (chunk 4g), the *whole* palette fades in and out via
`hud.palette_alpha()` — so instead of `HudGpu`/`draw_hud_element` (built for the bar's
"never fades" case), every element here — panel, all nine cell backgrounds, icons and
digits, even the reused name label — draws through the same `draw_rotated_sprite` that
already serves termites, particles and flames: a fresh buffer/bind group per draw, alpha
built in, rotation just always `0.0`.

**Two cell-background textures, not eighteen.** A cell's fill/stroke only ever has two
states (selected/not) — `ToolPaletteHUD.swift`'s own `highlight(_:)` just swaps colors on
the same shape. Built once, reused across all nine cells by choosing which texture a given
cell's draw call points at, keyed off `id == gpu.active_tool` at draw time — no per-cell
texture, no rebuild on selection change.

**One simplification beyond square corners (already flagged for the bar):** a palette
digit renders in a single fixed dim color always, rather than switching to a brighter
white when its cell is selected (`ToolPaletteHUD.swift`'s `digit.fontColor` does). The
selected cell's own highlighted background+stroke already carries that signal visually;
this avoids needing *two* cached textures per digit (18 total) for a distinction the cell
background already makes.

**The name label is reused, not rebuilt.** `ToolPaletteHUD.swift`'s own `nameLabel` shows
exactly the same text (`"N · ToolName"`) as `buildHUD`'s `toolLabel`, just at a slightly
different size (13pt semibold vs. 14pt medium) — close enough that this port draws the
*same* `hud_label` texture at both places rather than shaping and caching a second,
near-identical one.

**Click-through required re-borrowing `gpu`, not fighting the borrow checker.** The
pointer handler already held `gpu: &mut GpuLayer` (borrowed from `self.layers.iter_mut()`)
when the palette-hit check was added; calling `self.select_tool(id)` — which itself
iterates `&mut self.layers` — while that borrow was theoretically still live would
conflict. Simplest fix, not the cleverest one: re-run `self.layers.iter_mut().find(...)`
after the `select_tool` call, at the small, one-time cost of a second short lookup, rather
than relying on subtler reasoning about exactly where NLL considers the first borrow to
end.

**`palette_tool_at`'s hit test doesn't check `hud.palette_open()` itself** — deliberately;
that's per-`GpuLayer` state this free geometry function has no access to, so the pointer
handler checks it and only calls `palette_tool_at` at all when the palette is actually
open, mirroring `ToolPaletteHUD.swift`'s own `tool(at:)` guard structurally rather than
literally.

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

Press `↑` — a 9-cell grid should fade in above the status bar, the active tool's cell
visibly highlighted. Click a different cell — that tool should become active (cursor
changes, status bar label updates) without needing to also press a number key. Press `↓`
to close it, or switch tools with `1`–`9`/Tab while it's closed — it should briefly flash
open and fade back out on its own.

## What "done" looks like

- [ ] `hakai` builds
- [ ] `↑`/`↓` opens/closes the palette with a visible fade
- [ ] The active tool's cell is visibly highlighted (brighter fill, an outline)
- [ ] Clicking a cell switches to that tool
- [ ] A keyboard tool switch flashes the palette open briefly even while it's closed
- [ ] Nothing else (status bar, cursor, damage) regressed

If that holds, this chunk is done. The credits panel (`C` to toggle) is the last piece of
the HUD, and the last piece of Phase 4 altogether.

**Confirmed working** on a real run ("works"), plus two follow-ups:

- **Shift+Tab.** Tab only ever cycled forward — `InputRouter.swift`'s own binding is
  `event.modifierFlags.contains(.shift) ? -1 : 1`, and Shift+Tab wasn't wired at all.
  Wayland delivers modifier state as its own separate event (`wl_keyboard.modifiers`), not
  bundled into each key press, so this needed a new `State.shift_held` field, kept current
  by `update_modifiers` and read by `press_key`'s `Tab` case. That alone wasn't enough,
  either: XKB's standard keymap remaps Shift+Tab to a *different* keysym entirely
  (`ISO_Left_Tab`), so `Keysym::Tab`'s own match arm never even fired for it — a second,
  explicit `Keysym::ISO_Left_Tab` arm was needed alongside `shift_held`, not instead of it.

- **The cursor's position on startup.** `PointerEventKind::Enter` carries the pointer's
  real surface-local position (per the `wl_pointer` protocol), exactly like `Motion` does
  — but the handler only ever used it to set `pointer_focus`/hide the system cursor, never
  writing it into `gpu.mouse`. So a fresh overlay, mapped under an already-stationary
  mouse, drew its tool icon at `GpuLayer`'s construction-time default `(0, 0)` until the
  *next* `Motion` event corrected it — invisible while the mouse was moving (which is most
  of the time), but wrong for that first static frame. One line: write `event.position`
  into `gpu.mouse` in the `Enter` handler too. (A first report of "wrong position" after
  this fix turned out to be an un-rebuilt binary, not a second bug — confirmed working on
  an actual rebuild.)

---

# Phase 4 — chunk 4i: the credits panel — the last piece of Phase 4

`C` toggles a fading acknowledgements screen — the licence-obligation-driven panel
`CreditsPanel.swift` exists for (23 of the 35 bundled sounds are CC BY 4.0, which requires
attribution reachable *from inside the running app*). With this, every piece of the plan's
Phase 4 exit criterion ("Damage tiles, sprite atlas, particles, cursor, HUD... in the same
seven z-bands") is in place.

**One texture, not dozens of bind groups.** `CreditsPanel.swift` lays out potentially 40+
individually-positioned `SKLabelNode`s onto one panel. Rather than the palette's approach
(nine cells, each cheap enough to draw as several small textures), this composites the
*whole* panel — background plus every line — into one pixmap up front, via a new
`blit_over` (straightforward "over" alpha-compositing, no different from what
`tiny-skia`'s own path-fill machinery already does internally, just done by hand here since
nothing routes two independently-rasterized pixmaps' pixels together otherwise). One
texture, one draw per frame, however many lines the credits actually have.

**A real font-metrics measurement, not a guess.** `CreditsPanel.swift`'s column budget
comes from `NSFont.maximumAdvancement.width` — a real API this crate doesn't have access
to through `cosmic-text`'s surface as used so far. `measure_char_width` gets the same
number empirically instead: rasterize a reference string of a known length, divide its
pixel width by that length. Cheap, run once (this chunk's panel is built once, like the
palette), and doesn't need to know anything about `cosmic-text`'s metrics API at all.

**Baseline alignment is approximated, not exact.** `TextRenderer::rasterize` deliberately
returns a pixmap tightly cropped to its own ink (see its own doc comment — that's what
makes a rasterized label's centre a reliable *visual* centre for the HUD's other,
single-line elements), which means it carries no baseline metric a multi-line block's
careful vertical rhythm would want. Each line is blitted with its top approximated as `y -
line.size`, close enough for a body-text block; a real typographic gap from the original,
flagged rather than silently accepted as exact.

**Modal, not bounds-checked.** `CreditsPanel.swift`'s own comment implies its click-swallow
only covers its own frame; this port swallows *every* click while the credits panel is
open, full stop — simpler, and arguably the behavior a modal acknowledgements screen should
have regardless of what the original does. Got the ordering right on the second try:
`is_down` has to stay `false` when a click is swallowed, not just skip delivering that one
`mouse_down` — otherwise every `Motion` event until release would still reach the active
tool and draw on the desktop underneath the panel the user thinks they're just looking at.

**Built once, from the first output's width — a real, minor simplification** for
multi-monitor setups whose screens differ in width, flagged the same way the palette's
square corners were: `CreditsPanel.swift` builds a fresh, correctly-sized panel per scene,
since its `max_width` genuinely depends on `screenSize.width`.

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

Press `C` — a large panel should fade in, centred on screen, with real acknowledgements
text: a title, a GRAPHICS section, an AUDIO section grouped by licence (CC BY 4.0 first,
with author names and a packed list of source titles), a THE ORIGINAL section, and "Press
C to close" at the bottom. Try clicking through it — nothing on the desktop underneath
should react. Press `C` again to close.

## What "done" looks like

- [ ] `hakai` builds
- [ ] `C` opens/closes the credits panel with a visible fade
- [ ] The text is legible, correctly wrapped, and grouped by licence
- [ ] Clicking anywhere while it's open doesn't affect the desktop underneath
- [ ] Nothing else (status bar, palette, cursor, damage) regressed

If that holds, **Phase 4 is done** — every exit criterion in the original plan (damage
tiles, sprite atlas, particles, cursor, HUD) is met. `ParticleKind::Generic`'s texture, the
hammer/machine-gun/chain-saw cursor animations, and the full HUD (status bar, toast,
palette, credits) all shipped across this phase's chunks. Phase 5 (real audio, `cpal`) is
next, per OMARCHY-PORT.md.
