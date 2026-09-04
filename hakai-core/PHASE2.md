# Phase 2 — build & test walkthrough (part 1: decals)

Phase 2 is scoped in the plan as "decal, icon and sprite generators" — 1,646 Swift lines,
5–7 days. Rather than port all of that blind in one shot, it's split into testable chunks
like Phases 0–1 were. **This is chunk 1: the full decal set.** Icons and sprites follow in
a later round once this is confirmed working.

Ported from `DecalFactory.swift` + `DecalFactory+Weapons.swift`: bullet hole, scorch mark,
paint splat (all 8 colours), phaser hit, stamp print (10 texts, real glyph outlines from an
embedded font), termite bite, blood, saw cut, and sliver — on top of the crack from Phase 1.

Still headless — no window, no audio, no compositor.

## New: an embedded font

The stamp decal needs real text rendered as filled paths (no CoreText here). I downloaded
**Archivo Black** (OFL-licensed, from Google Fonts) and embedded it via `include_bytes!` —
see `assets/fonts/ATTRIBUTION.md`. `src/fonts.rs` reads glyph outlines out of it directly
with `ttf-parser`, with no system font lookup, so output is identical on every machine.

## Build & test

```bash
cd /mnt/mac/hakai-core
cargo test
```

**Two things in this chunk are flagged `RISK` in `src/decals.rs` and `src/fonts.rs`** —
search for `RISK` — both are my best-confidence reconstructions I couldn't check against a
compiler:

1. `radial_gradient_paint` — `tiny_skia::RadialGradient::new`'s exact parameter order (a
   two-circle gradient, start point+radius and end point+radius, mirroring Core Graphics'
   `drawRadialGradient`). Used by the bullet hole's crater, the scorch mark's burn
   gradient, and the phaser's glowing core. If these come out solid-coloured instead of
   graduated, or this doesn't compile, start here.
2. The glyph outline → path pipeline in `fonts.rs` (`ttf_parser::OutlineBuilder`,
   `Face::glyph_index`/`outline_glyph`/`glyph_hor_advance`/`units_per_em`) — I'm fairly
   confident in this one (it's ttf-parser's primary documented use case), but it's the one
   piece of this chunk touching a file format rather than just drawing calls.

Everything else (the shape helpers — `lobed_path`, `roughen`, `push_ellipse`,
`push_rounded_rect`, `stroke_rect` — and every individual decal generator) is the same kind
of `tiny-skia` drawing calls Phase 1 already proved out; I'm considerably more confident in
those.

As always: paste back the full `cargo test` output if anything fails, including warnings —
they're often more informative than the final error line for a dependency-resolution or
version-mismatch problem.

## Visual check

```bash
cargo run --example dump_decals
```

Writes one PNG per decal/variant to `target/dump/` (or `<dir>` if you pass one) — since
that path is on the shared mount, I can look at them directly afterward without you needing
to send anything. Expect, roughly:

- **bullet_hole-\*.png** — a dark ragged hole with a light crater glow around it
- **scorch-\*.png** — an irregular dark burnt patch, charred specks, a few light ash flecks
- **paint-{red,green,blue,…}.png** — a wet-looking blob with drips hanging below it, a
  visibly different hue per file
- **phaser-\*.png** — a small glowing white→yellow→orange→red core with radiating streaks
- **stamp-0 through stamp-9.png** — real words (VOID, REJECTED, PAID, …) inside one of
  three frame styles (double rect / oval / rounded rect), slightly roughened like real ink
- **bite-\*.png** — a small dark irregular blotch with a light rim
- **blood-\*.png** — a dark red blob with a few smaller droplets scattered around it
- **saw_cut-\*.png** — a narrow horizontal torn slit, wider in the middle, tapering at
  both ends
- **sliver-\*.png** — a small sharp-edged light-grey polygon

## What "done" looks like

- [ ] `cargo test` passes — expect ~30 tests (Phase 1's 22 plus this chunk's new ones:
      per-type "not blank" checks, stamp determinism/uniqueness, paint colour count/order,
      stamp text count, and that different paint colours actually look different)
- [ ] The PNGs above look like their description, not blank/solid-black squares
- [ ] The 10 stamp PNGs show real, readable words — this is the one output where "the
      code compiled and the test passed" doesn't guarantee "it looks right," since a
      layout bug could still produce garbled or overlapping glyphs that aren't literally
      blank

If all of that holds, tell me and I'll mark this chunk done in `OMARCHY-PORT.md` and start
on icons + sprites.

---

# Phase 2, chunk 2: icons

Ported from `IconBuilder.swift` + `ToolIcons.swift` + `ToolIcons+Weapons.swift` — all 9
tool icons (hammer, chain-saw idle/cutting, machine gun, flame-thrower, color-thrower,
phaser, stamp up/pressed, termite hand, washer), each with its hotspot and, for the
hammer, its separate rotation pivot.

## The y-up/y-down issue, handled centrally this time

Phase 2 chunk 1's real bug (paint splat drips pointing up) was a Core Graphics-is-y-up vs
tiny-skia-is-y-down mismatch, fixed by hand at each of the two affected call sites. Icons
have the same underlying issue at far more call sites — every icon's coordinates encode an
absolute up/down orientation (a chain-saw bar pointing down-left, a hammer head at the
bottom, …) — so this time the flip happens in exactly one place,
`IconBuilder::flip` (see the module doc comment at the top of `src/icons.rs`), rather than
being re-derived by hand per shape. Every icon's coordinates are transcribed from Swift
untouched; only the final screen-space emission is different.

**One spot could not go through that central mechanism**: the chain-saw's cutting-mode
sawdust particles compute a perpendicular offset (`nx`) from the bar's own already-flipped
direction vector, which flips the sign of that one perpendicular component relative to
Swift's — see the comment at that call site in `make_chain_saw`. The practical effect is
at most a mirrored curve to the sawdust spray (still visibly flying from the right place,
just possibly bowing the other way) — flagged rather than fixed, since chasing it further
didn't seem worth it for something this minor. Worth a glance in the PNG dump, not worth
worrying about if it looks basically right.

## Build & test

```bash
cd /mnt/mac/hakai-core
cargo test
```

Nothing in this chunk needed a `RISK` flag — no new external API surface, just more of the
same `tiny-skia` drawing calls Phase 1 and chunk 1 already proved out, plus the geometry
helpers factored into `src/geometry.rs` (shared with `decals.rs` now — `push_ellipse` and
`push_rounded_rect` moved there unchanged, plus two new ones: `push_rounded_rect_mapped`
for the hammer's rotated head/face, and `push_capsule`, standing in for Core Graphics'
`CGPath.copy(strokingWithWidth:)`, which tiny-skia has no public equivalent for).

## Visual check

```bash
cargo run --example dump_icons
```

Writes one PNG per icon to `target/dump/`, prefixed `icon-`. Expect:

- **icon-1-hammer** — a hammer head at roughly a -44° angle (handle down-right), claw at
  the top, striking face at the bottom-left
- **icon-2-chainsaw-idle / -cutting** — a bar with visible teeth down both edges, an
  orange engine housing, two handles; the cutting version additionally shows a small
  scatter of tan sawdust flecks near the bar's tip
- **icon-3-machinegun** — barrel, cooling slots, a forward-tilted magazine, wood stock
- **icon-4-flamethrower** — a red fuel tank, a flared nozzle with a small permanently-lit
  pilot flame (orange dot) above it
- **icon-5-colorthrower** — a blue gun body with a translucent paint canister on top
- **icon-6-phaser** — a sleek gunmetal body, a glowing cyan-white emitter ring, a cyan
  light strip along the top
- **icon-7-stamp-up / -down** — a wood-handled rubber stamp; the "down" version's knob and
  stem sit visibly lower (pressed toward the rubber face) than "up"
- **icon-8-termites** — a hand pinching a termite (oval body, two mandibles, six legs)
- **icon-9-washer** — a spray bottle with a nozzle, a visible liquid fill line, and a label

The printed hotspot coordinates are worth a glance too — every one should be a plausible
point near the tool's business end (e.g. the hammer's near the striking face, the
machine gun's near the muzzle), and the hammer should be the only line printing
"has pivot".

## What "done" looks like

- [ ] `cargo test` passes — expect Phase 1+chunk 1's ~28 plus this chunk's new ones
      (~35 total): every icon non-blank, every hotspot inside `[0,1]`, only the hammer has
      a pivot and it's meaningfully different from its hotspot, `icon_for` agrees with the
      direct accessors, idle vs cutting and stamp up vs down are visibly different
- [ ] All 11 PNGs (9 tools, +1 each for the chain-saw and stamp's second state) look like
      their description above — each icon recognisable as its tool, not a jumbled or
      upside-down silhouette
- [ ] The hammer in particular looks like a hammer at an angle, not flipped or mirrored —
      it's the one icon with genuinely custom rotation math, so it's the most likely place
      for a sign error to hide

If that holds, tell me and I'll mark chunk 2 done — sprites (`SpriteFactory.swift`, ~314
lines: flames, termites, paint droplets, shells, sparks) are the last piece of Phase 2.

---

# Phase 2, chunk 3: sprites (the last piece of Phase 2)

Ported from `SpriteFactory.swift`: the flying flame, the standing flame, the termite (2
walk frames), paint droplets (all 8 colours), the ejected shell, the impact flash, the
phaser beam segment, and the washer's spray droplets.

This time the y-up/y-down fix (see chunk 1 and 2's notes) was applied proactively rather
than found after the fact — worked out which shapes have real up/down meaning before
writing them: the flame's teardrop (tip vs base) and the droplet/shell highlights needed
it; the termite, flash, beam and spray turned out to be vertically symmetric and so are
unaffected regardless (each has a one-line note in `sprites.rs` explaining which and why).

**One quirk ported as-is, not fixed**: the termite's leg-drawing loop in the Swift source
draws a line to *both* of its two possible endpoints regardless of the `up`/`frame`
condition that's supposed to select between them — so the two walk frames currently render
identical legs. This is a property of the source being ported, not something introduced
here; flagged in a comment in `make_termite` rather than silently corrected, since fixing
behaviour wasn't asked for and it's not clear it's actually a bug rather than an
intentional (if oddly-written) "always show both" choice.

## Build & test

```bash
cd /mnt/mac/hakai-core
cargo test
```

One thing worth a note, not quite a `RISK` flag since I'm fairly confident: `beam` uses
`tiny_skia::LinearGradient::new` for the first time in this port (everything else so far
used `RadialGradient`, whose real signature — `(start, end, radius, stops, mode,
transform)` — came out different from my first guess back in chunk 1). I've written
`LinearGradient::new(start, end, stops, mode, transform)` — the same 5 arguments minus the
radius, by analogy — but haven't seen it confirmed by a compiler. If `make_beam` doesn't
compile, that's the signature to check against the error.

## Visual check

```bash
cargo run --example dump_sprites
```

Writes one PNG per sprite/variant to `target/dump/`, prefixed `sprite-`. These are small
(the largest is 84×148) — expect:

- **flame-0..3 / standing_flame-0..3** — a teardrop shape, wide base narrowing to a point,
  layered red→orange→yellow-white, a few small bright specks near the tip. The four
  frames should look like flickering variations on the same shape, not four unrelated
  images. `standing_flame` should look like a taller, slightly more stretched version
- **termite-0 / termite-1** — a small bug: oval abdomen, smaller thorax, a head with two
  mandible lines, six visible leg strokes. The two frames will look identical per the
  quirk noted above — that's expected, not a rendering failure
- **droplet-0..7** — a small filled circle in each of the 8 paint colours, a dark rim, and
  a white highlight toward the upper-left
- **shell** — a small brass rounded-rect capsule (a bullet casing on its side), a darker
  rim on the left end, a light highlight line along the upper portion
- **flash** — a small radiating glow (white-yellow core fading to transparent orange) with
  a faint four-pointed star through it
- **beam** — a short vertical gradient strip: transparent at both ends, cyan glow, a solid
  white core in the middle
- **spray-0..2** — a scatter of small translucent light-blue droplets, no two frames
  identical

## What "done" looks like — and the end of Phase 2

- [ ] `cargo test` passes — expect chunk 2's ~35 plus this chunk's new ones (~43 total):
      every sprite type non-blank, flame frames deterministic and mutually distinct,
      flame vs standing_flame report the right pixel dimensions, droplet colours differ,
      negative/oversized indices wrap, spray frames differ
- [ ] The 21 PNGs above look like their descriptions — the flame in particular should
      unambiguously read as fire, tapering to a point at the top, not a blob or something
      pointing sideways/down

If that holds, **all of Phase 2 is done** — the full decal, icon and sprite generator set,
matching the plan's exit criterion (`hakai --dump-assets`-equivalent output for visual
review). Phase 3 (tools, termites, the simulation harness) is next, and is the phase where
this port becomes logically complete — everything after it is presentation (rendering,
audio, window/input).
