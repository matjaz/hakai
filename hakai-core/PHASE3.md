# Phase 3 — build & test walkthrough (chunk 1: foundation)

Phase 3 is "all nine tools, the colony, the particle system" — the phase where the port
becomes logically complete. Like Phases 1–2, it's split into testable chunks. **This is
chunk 1: the foundation everything else builds on** — `AudioSink`, `ParticleSystem`,
`TermiteColony`, the `Tool` trait, and one tool (`Hammer`) to prove the design end to end.
The other 8 tools and the simulation harness follow in later chunks.

Still headless — no window, no audio device, no compositor.

## Two real architectural problems, not just porting

Swift's tools do two things Rust can't do directly:

1. **`ctx.termites.kill(..., ctx: ctx)`** — a method on `ctx.termites` takes the *whole*
   `ctx` (which contains `termites` itself) as an argument. Trivial with Swift's reference
   semantics; in Rust, a struct can't be mutably borrowed as one of its own method's
   arguments. Fixed by decomposing: `TermiteColony`/`ParticleSystem` methods take exactly
   the disjoint pieces they need (`damage: &mut DamageLayer`, `rng: &mut SeededRng`, …) as
   separate parameters, and every call site writes `ctx.termites.kill(point, radius,
   cause, ctx.damage, ctx.decals, &mut ctx.audio, &mut ctx.rng, ...)` rather than passing
   `ctx` whole — the same disjoint-field-borrow pattern used since Phase 0's wgpu surface
   setup, just with more fields this time.

2. **Particle `onLand`/`onExpire` closures capture `ctx` weakly at emit time**, called back
   whenever that particle lands or expires — possibly many frames later. Rust can't stash
   a `&mut` borrow in a closure for that long. Fixed differently: each particle carries a
   [`ParticleKind`] tag (`Generic`, `Shell`, `Droplet { color_index }`) instead of a
   closure, and the small amount of logic those closures ran (a shell's landing sound, a
   droplet's paint splat) moved into `ParticleSystem::update` itself, dispatched by kind.
   Same behaviour, no stored closures.

## A capability gap, found and fixed before it blocked everything

Phase 1's `DamageLayer::stamp` never got a `rotation` parameter — deferred to "Phase 2,"
which then didn't need it (decals were dumped as standalone PNGs). Nearly every Phase 3
tool call needs one (crack rotation, cut angle, bullet hole, stamp print, bite, blood, …),
so it's added now: `stamp_ex(decal, center, size, rotation, alpha)`, with the old 3-arg
`stamp()` kept as a `rotation: 0, alpha: 1` convenience so every existing Phase 1/2 test
still compiles unchanged.

**Flagged `RISK` in `damage.rs`**: the rotation composition uses `Transform`'s `pre_rotate`
/ `pre_scale` / `pre_translate` builder methods, chained in the same "pre_X = happens
before" order that worked for Core Graphics' `.rotated(by:)` in the icon port (Phase 2).
I'm fairly confident in the *semantics* (that ordering convention is close to universal in
graphics APIs) but haven't seen these specific three methods confirmed by a compiler. At
rotation=0 the new formula is provably identical to the old, tested one (I checked by
hand); the untested part is purely what happens once `rotation != 0`. There's a test for
this — see below — that would actually fail if the composition order were wrong, not just
one that checks "something happened."

## Y-up/y-down, one more time — this time in physics, not shapes

Same underlying issue as `decals.rs`/`sprites.rs`, new context: particle *launch
velocities*. Gravity itself only needed one sign flip, in `particles.rs`'s shared
`GRAVITY` constant (positive here; Swift's is `-1_800`). Individual tools separately negate
velocities that are supposed to fly *upward* off an impact — Hammer's sliver velocity is
the first instance (`tools/hammer.rs`); chain-saw sawdust, machine-gun shell ejection, and
flame drift will need the same treatment when those tools are ported in later chunks.
Applied proactively this time, per Phase 2's precedent, with a comment at each site.

## Build & test

```bash
cd /mnt/mac/hakai-core
cargo test
```

Expect green — around 20 new tests across `audio.rs`, `particles.rs`, `colony.rs`,
`tools/mod.rs`, `tools/hammer.rs`, plus one new one in `damage.rs` for the rotation
transform. If `damage.rs`'s
`a_rotated_stamp_lands_near_its_centre_not_near_the_origin` fails specifically, that's the
`pre_rotate`/`pre_scale`/`pre_translate` chain — see the RISK note above.

## What "done" looks like

- [ ] `cargo test` passes
- [ ] The Hammer tests in particular are worth reading if anything fails there: they
      exercise the exact self-referential-borrow pattern (tool → `ctx.termites`/
      `ctx.particles` → back through `ctx.rng`/`ctx.damage`) that every other tool will
      need — if this compiles and passes, the foundation is proven and the remaining 8
      tools are comparatively mechanical

If that holds, tell me and I'll start on the next chunk of tools.

---

# Phase 3 — chunk 2: the remaining 8 tools

Chain-saw, machine gun, flame-thrower, color-thrower, phaser, stamp, termites (the tool —
distinct from `TermiteColony`), washer, plus `ToolId::make_tool()`. All the same patterns
as Hammer, applied to the rest: disjoint-field `ctx` decomposition, `ParticleKind` instead
of stored closures, launch-velocity y-flips where a tool has a real up/down bias.

Two fixes applied proactively before writing any of this, from chunk 1's lessons:

- `AudioSink::set_loop` was missing the `volume` parameter Swift's version has — the
  chain-saw's idle/cutting crossfade needs to set a loop's volume to 0, not just its pan.
- `ToolContext::along_path` (used by the chain-saw's cut and the washer's erase, both of
  which act continuously along a drag) turned out to never actually use `self` — kept as a
  method it would have created exactly the borrow conflict this phase keeps navigating
  (`ctx.along_path(...)` borrowing `ctx` for the method call while its own closure needs
  `ctx` mutably), so it's a free function instead. Every closure passed to it explicitly
  reborrows `ctx` as `&mut *ctx` rather than relying on capture inference.

```bash
cd /mnt/mac/hakai-core
cargo test
```

Nothing new flagged `RISK` here — this chunk is entirely the same kind of `tiny-skia`/
borrow-pattern work chunk 1 already proved out, just applied eight more times.

---

# Phase 3 — chunk 3: the simulation harness (Phase 3's exit criterion)

Ported from `ToolSimulation.swift`: a `Rig` (owns a `DamageLayer` + everything else a
`ToolContext` borrows from), `drive()` (a synthetic stroke — mouse down, dragged, updated,
released, then settled), `run()` (drives all nine tools and reports coverage/particles/
termites per tool), and `check_interactions()` (the five rules).

```bash
cd /mnt/mac/hakai-core
cargo test
cargo run --example simulate
```

`cargo run --example simulate` is this port's `hakai --simulate` — expect a coverage table
for all nine tools followed by five `OK` lines, one per rule. This is also runnable as a
plain diagnostic any time, not just as a test: it exits non-zero and prints `FAIL` lines if
anything regresses.

## What "done" looks like — and the end of Phase 3

- [ ] `cargo test` passes
- [ ] `cargo run --example simulate` prints a coverage percentage for all nine tools —
      zero only for the washer, which is expected (it *reduces* coverage; that's checked
      separately via its note and rule #4, not via its own coverage number being nonzero)
- [ ] All five interaction rules print `OK`

If that holds, **Phase 3 is done** — all nine tools, the termite colony, the particle
system, and headless verification of the rules of the original, matching the plan's exit
criterion exactly. Phase 4 (the renderer — wgpu, damage tiles as GPU textures, the sprite
atlas, the HUD) is next, and is the first phase that needs the Omarchy machine for more
than just running `cargo test`/`cargo run --example` — it needs a live Hyprland session to
actually watch anything render.
