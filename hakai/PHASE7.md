# Phase 7 — build & run walkthrough (chunk 1: the color-thrower's paint palette)

Goal: the color-thrower's 8 splat/droplet colours come from the active Omarchy theme
instead of a fixed built-in set — "your paint is your theme," the first of Phase 7's two
Omarchy-specific pieces (the second, still open, is the HUD/palette chrome).

## The plan doc was stale — checked before writing anything

`OMARCHY-PORT.md`'s original Phase 7 plan assumed the palette lived in
`alacritty.toml`'s ANSI 0–7 colour table. Checked directly against the live
`omacom/omarchy` repo (`gh api`, not a guess) before writing any code, since this is
exactly the kind of "unversioned, moves between releases" risk the plan itself flagged.
Both assumptions turned out wrong:

- The file isn't `alacritty.toml` any more. Omarchy's own canonical palette file is
  `~/.local/state/omarchy/current/theme/colors.toml`, a flat `key = "value"` TOML, not a
  nested ANSI table — confirmed by reading `bin/omarchy-theme-color`'s source, which reads
  from exactly that path.
- Raw ANSI 0–7 (which includes black and white) is a worse fit for 8 *paint* colours than
  what Omarchy actually offers: `colors.toml` has named hues — `red`, `orange`, `yellow`,
  `green`, `cyan`, `blue`, `magenta`, `brown` — a near-exact match for
  `DecalFactory::DEFAULT_PAINT_COLORS`' own `red/green/blue/yellow/purple/cyan/orange/pink`
  (`purple`↔`magenta` is already an alias in Omarchy's own resolver).

**The real decision, though, wasn't the file path — it's how much of Omarchy's own
resolution logic to reproduce.** Not every theme defines every key. Omarchy's
`omarchy-theme-color` script — the one every first-party consumer (Waybar, tmux, GNOME,
theme previews) goes through, not something informal or half-maintained — carries a real
amount of fallback logic on top of the raw file: legacy `colorN` ANSI aliasing, deriving
missing `bright_*`/`orange`/`brown` shades by mixing colours when a theme doesn't define
them directly, light/dark mode detection by background luminance. Reimplementing that by
hand in Rust would both be a meaningful amount of logic to port correctly and would drift
out of sync the moment Omarchy's own script changes it again — exactly the kind of
ongoing risk the original plan called out. Presented as an explicit choice rather than
assumed: shell out to `omarchy-theme-color` (gets Omarchy's real resolved colours for
free, stays correct across Omarchy version changes, at the cost of a runtime dependency on
that script being on `PATH`) vs. parse `colors.toml` directly (no subprocess, but
re-derives that fallback cascade by hand and risks drifting from it). **Chosen: shell
out** — see `hakai/src/theme.rs`'s own module doc comment for the full reasoning.

## What it does

`hakai/src/theme.rs`: `read_paint_colors()` runs `omarchy-theme-color <key>` once per
colour, for 8 keys in `DecalFactory::DEFAULT_PAINT_COLORS`' own order
(`red green blue yellow purple cyan orange bright_magenta` — `bright_magenta` standing in
for "pink", a hue Omarchy has no name of its own for), parses each `#rrggbb` line it
prints into the `0.0..=1.0` float triples this port already uses everywhere else.
Deliberately all-or-nothing: if the binary isn't on `PATH`, any single query fails, or any
line isn't well-formed hex, the whole read comes back `None` rather than a
partially-themed palette. `main()` calls this once at startup, right after `State` is
constructed and before anything can possibly have cached a paint decal yet, and either
calls the new `DecalFactory::set_paint_colors` or leaves the built-in default in place,
logging which happened.

**`hakai-core` stays theme-agnostic, deliberately** — same shape as `AudioBackend` being
injected from `hakai` rather than known about inside `hakai-core` itself. `DecalFactory`
gained a `paint_colors: [(f32, f32, f32); 8]` field (defaulting to
`DEFAULT_PAINT_COLORS`' own RGB triples), a `paint_colors()` getter and a
`set_paint_colors()` setter; `paint_splat` reads from the instance field instead of the
old `Self::PAINT_COLORS` constant (renamed `DEFAULT_PAINT_COLORS`, kept as the fallback
and for its names, still used in tests). `SpriteFactory::droplet` — a completely separate
struct with no reference to `DecalFactory` — now takes the 8 colours as a parameter
instead of reading the constant directly, so a flying droplet always matches the splat
it'll leave: both `main.rs` call sites read from the same `self.decals.paint_colors()`.

**One real caching hazard, documented rather than silently risked.** Both
`DecalFactory::paint_splat` and `SpriteFactory::droplet` cache by colour *index* alone
(`"paint{c}_{v}"`, `"drop{i}"`) — neither the colour value nor a "this is themed now"
epoch is part of the key. Calling `set_paint_colors` after either has already cached an
index would leave that index visibly stuck on its old colour. Not a problem *yet* since
this chunk only ever calls it once, at startup, before either cache is ever touched — but
flagged explicitly in both the setter's doc comment and here, since it becomes a real bug
the moment anything tries live theme-switch support (Omarchy's `omarchy-theme-set` while
`hakai` is already running) — deliberately out of scope for this chunk, not forgotten.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

New tests: `a_fresh_factory_starts_with_the_default_paint_colors`,
`set_paint_colors_overrides_paint_splat` (`decals.rs`), plus `sprites.rs`'s existing
droplet tests updated for the new `colors` parameter and a new
`droplet_reflects_whatever_colors_its_caller_passes`.

```bash
cd /mnt/mac/hakai
cargo build && cargo test && RUST_LOG=info cargo run
```

New tests in `theme.rs`: hex-colour parsing, malformed input rejection, the 8-key order
constant. Look for one of these two lines in the log right after startup:

```
paint palette: sourced from the active Omarchy theme
paint palette: using the built-in default (no Omarchy theme found)
```

If it says "sourced from the active Omarchy theme": switch to the color-thrower (`5`) and
throw some paint — the splats and flying droplets should visibly use your current Omarchy
theme's colours, not the fixed built-in set. Run `omarchy-theme-set <another-theme>`,
*restart* `hakai` (live reload isn't in scope for this chunk — see above), and throw paint
again — the palette should now match the new theme instead.

## What "done" looks like

- [ ] `hakai-core` and `hakai` both build and test clean
- [ ] The startup log shows one of the two palette-source lines above
- [ ] On an Omarchy box: the color-thrower's splats and flying droplets visibly match the
      active theme's colours, and change after `omarchy-theme-set` + a restart
- [ ] Off an Omarchy box (or with `omarchy-theme-color` unavailable): falls back to the
      built-in palette cleanly, no error, no partially-themed colours
- [ ] Nothing else regressed

**Confirmed working** on a real run ("works") — one unrelated crash found and fixed along
the way, not a chunk-1 regression: `Surface::configure: ... width and height must be
within the maximum supported texture size (2048)`, on an output whose buffer size (points
× `scale`) came out to `2494×1566` pixels. Root cause dates back to Phase 0:
`adapter.request_device` asked for `wgpu::Limits::downlevel_defaults()` — the WebGL2-safe
profile, capped at 2048×2048 textures, appropriate for "this has to also run in a
browser," not for a native Vulkan app. It never surfaced before this session because no
prior test output's buffer size (points × scale) had exceeded 2048 in either dimension.
Fixed by requesting `adapter.limits()` instead — whatever the real GPU actually reports
(typically 8192 or 16384 on a real Vulkan driver) — exactly as safe to request as the
downlevel profile was, since it's what `request_adapter` already established this adapter
supports. See `hakai/PHASE0.md`'s own device-setup section for where this originated.

Palette itself confirmed correctly themed and reflecting the active Omarchy theme.

---

# Phase 7 — chunk 2: HUD/palette chrome

Goal: "HUD in the system's voice" — the status bar, tool label, hint text, tool palette
and credits panel read their colours from the same theme source chunk 1 wired up for the
paint palette, instead of the fixed black/white this port has used since Phase 4.

## What it does

Extended `theme.rs` with `HudColors { background, foreground, accent }` (`u8` triples,
not the `0.0..=1.0` floats `read_paint_colors` uses — everything here hands straight to
`tiny_skia::Color::from_rgba8`/`TextRenderer::rasterize`) and `read_hud_colors()`, reading
Omarchy's own `background`/`foreground`/`accent` keys through the same
`omarchy-theme-color` call as chunk 1, with the same all-or-nothing `Option` shape. `main`
reads it once at startup, right alongside the paint palette read, into a new
`State::hud_colors` field — `HudColors::FALLBACK` (flat black/white/the credits panel's
old hand-picked light blue) if Omarchy isn't available, so behaviour off-Omarchy is
pixel-identical to every prior phase.

**Every HUD colour that used to be a literal `Color::from_rgba8(0, 0, 0, ...)` or
`(255, 255, 255, ...)` now reads through `hud_rgba`/`hud_rgba_arr` (two small helpers next
to `build_panel_pixmap`) instead** — the status bar panel, its hint/label text, the tool
palette's panel and both cell states, the palette digits, the toast, and the credits
panel's fill/stroke/text (`credits_text_color` now takes `&HudColors` instead of hardcoding
its own light blue for `TextColor::Accent`). Every alpha value is untouched — only the hue
changes, so the existing contrast/legibility design (a mostly-opaque dark bar, a subtle
border, brighter text) survives regardless of what a theme's actual background/foreground
luminance is.

**One deliberate exception: the selected palette cell reads `accent`, not
`foreground`.** Every other element reuses `background`/`foreground` (the panel is *a*
panel, the text is *text*, whatever hue the theme gives those roles) — but the selected
cell is the one HUD element whose entire job is to draw the eye, so it's the one place
this chunk reaches for the theme's actual accent colour instead. Concretely: switching
tools now highlights the palette with your Omarchy accent color, not a generic white glow.

**Threading, not restructuring.** `render()` (a free-standing associated function, not a
`&self` method, so it can't just reach for `self.hud_colors`) gained one more parameter,
`hud_colors: &theme::HudColors`, passed by both call sites. Everything that builds a
pixmap once at startup (the HUD/palette/credits block inside `configure()`) reads
`self.hud_colors` directly instead, since it *does* run as a `&mut self` method.

**Not themed, deliberately:** the reference-string colour `measure_char_width` rasterizes
to measure ink-bounds width — never actually displayed, so threading `HudColors` through
just for that would be pure noise.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

(unchanged — this chunk touched only `hakai`)

```bash
cd /mnt/mac/hakai
cargo build && cargo test && RUST_LOG=info cargo run
```

Look for `HUD colours: sourced from the active Omarchy theme` right after the paint
palette's own startup log line. The status bar, hint text, tool label, palette panel and
credits panel should all pick up your theme's background/foreground hues instead of flat
black/white, and switching tools (`1`–`9`) should highlight the palette cell in your
theme's accent colour. Run `omarchy-theme-set <another-theme>`, restart `hakai` (same
"read once at startup" limitation chunk 1 has — live reload is still out of scope), and
check the HUD picks up the new theme too.

## What "done" looks like

- [ ] `hakai-core` and `hakai` both build and test clean
- [ ] The startup log shows the HUD-colours line alongside the paint-palette one
- [ ] On an Omarchy box: HUD/palette/credits chrome visibly uses the active theme's
      colours, and the selected palette cell highlights in the theme's accent
- [ ] Off an Omarchy box: falls back to the original flat black/white/light-blue look,
      pixel-identical to before this chunk
- [ ] Nothing else regressed (paint palette, frozen mode, audio, fractional scale)

**Confirmed working** on a real run ("works") — HUD/palette/credits chrome themed
correctly, selected-tool accent highlight visible, no regressions.

---

Phase 7 is complete: the color-thrower's palette (chunk 1) and the HUD/palette/credits
chrome (chunk 2) both read from the active Omarchy theme, and the keybind snippet
(`SUPER SHIFT H` / `SUPER SHIFT ALT H` — settled earlier after `SUPER SHIFT D` turned out
to collide with Omarchy's own LazyDocker default) is recorded in `OMARCHY-PORT.md`. One
deliberate, explicitly-flagged scope limit carried by both chunks: theme colours are read
once at `hakai` startup, not live-reloaded while running — `omarchy-theme-set` while
`hakai` is already open needs a restart to pick up. Left as a known follow-up rather than
built now, since it isn't required by the plan's own exit criterion as written and would
need real design work (both `DecalFactory::paint_splat` and `SpriteFactory::droplet` cache
by colour *index*, not value — see `set_paint_colors`'s doc comment — so a live reload
would need real cache invalidation, not just re-calling the setter).
