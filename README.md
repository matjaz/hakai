# Hakai (破壊)

A native Wayland/Hyprland desktop-destruction toy for **Omarchy** — smash, burn, shoot,
paint and squish your live desktop with nine tools, then watch it wander off on its own
(termites) or wipe it clean again (the washer).

Hakai is Japanese for *destruction*. It's a Rust/`wgpu`/Hyprland port of
[Desktop Destroyer](http://www.breatharian.eu/Petr/en/program/misc.htm), a macOS
SpriteKit/AppKit toy by Miroslav Němeček — reimplemented independently for Linux, not a
code fork. See [`CREDITS.md`](CREDITS.md) for the full sound/font attribution.

## What it does

A full-screen, always-on-top, transparent overlay sits above your real desktop and takes
an exclusive keyboard grab (`Esc` always gets you out, and Hyprland still dispatches its
own compositor binds — `SUPER`-anything — underneath it regardless). Nine tools, each a
faithful port of the original's own behaviour:

| Key | Tool | What it does |
|---|---|---|
| `1` | Hammer | Cracks the desktop where you strike, with a satisfying knock animation |
| `2` | Chain-saw | Cuts continuously while dragged; revs up with movement, not just the button |
| `3` | Machine gun | Punches bullet holes, ejects shells, flashes on fire |
| `4` | Flame-thrower | Leaves standing fires that spread, flicker, and burn out into scorch marks — and keep doing all of that even after you switch tools |
| `5` | Color-thrower | Splats paint in your active Omarchy theme's own colours |
| `6` | Phaser | A sustained beam |
| `7` | Stamp | Stamps a random bureaucratic verdict (`REJECTED`, `APPROVED`, `TOP SECRET`, ...) |
| `8` | Termites | Releases bugs that wander the desktop and eat it, one bite at a time |
| `9` | Washer | The only repair tool — wipes damage away along the stroke (blocked by living termites) |

Plus: `Tab` / `Shift+Tab` cycles tools, `↑`/`↓` opens/closes the tool palette, `M` freezes
the view to a snapshot (so the *real* desktop underneath can keep changing without
disturbing what you're smashing), `C` opens the credits panel, `R` clears everything.

## Building and running

Needs a real Hyprland session (Wayland, `wlr-layer-shell`) — this won't build a usable
window on X11, GNOME, or KDE, and there's no plan to support them (see the non-goals in
the project's own port-planning notes).

```bash
cd hakai
cargo build --release
cargo run --release
```

`hakai-core` — the headless raster/game-logic core (damage layer, decal/icon/sprite
generators, all nine tools, the termite colony, particle system) — builds and tests on
any OS, no compositor needed:

```bash
cd hakai-core
cargo test
```

A suggested Hyprland keybind (not applied automatically — see
[`packaging/hyprland-bindings.conf.example`](packaging/hyprland-bindings.conf.example) for
both the current Lua form and the classic hyprlang one, depending on your Hyprland
version). On a current Omarchy install (Hyprland 0.55+, Lua config), add this to
`~/.config/hypr/bindings.lua` — not `bindings.conf`, which nothing loads once you're on
Lua — then `hyprctl reload`:

```lua
o.bind("SUPER + SHIFT + H", "Destroy the desktop", "uwsm app -- hakai")
o.bind("SUPER + SHIFT + ALT + H", "Stop destroying it", "pkill -x hakai")
```

## Layout

```
hakai/         the binary — wlr-layer-shell + wgpu rendering, Wayland input, cpal audio,
               screen capture, Omarchy theme integration
hakai-core/    headless library — everything that doesn't need a window: the tiled damage
               layer, procedural decal/icon/sprite generators, all nine tools, the termite
               colony, particle system, HUD/credits logic
packaging/     PKGBUILD (AUR, -git), .desktop entry, Hyprland keybind snippet
```

Not a formal Cargo workspace, deliberately — `hakai` depends on `hakai-core` via a plain
relative path instead. Each crate has its own `PHASE*.md` walkthrough documenting how it
was built, chunk by chunk, including every real bug found along the way and how it was
fixed; `PHASE8.md` at the repo root covers packaging, the one phase that's inherently
cross-cutting.

## Omarchy integration

The color-thrower's palette and the whole HUD (status bar, tool palette, credits panel)
read their colours from your active Omarchy theme (`omarchy-theme-color`), falling back
to a built-in palette cleanly if that's not available — read once at startup, so
`omarchy-theme-set` while `hakai` is already running needs a restart to take effect.

## Status

Core development (rendering, input, audio, screen capture, fractional scaling, Omarchy
theming) is done and confirmed working on real Hyprland hardware. Packaging (`PKGBUILD`,
CI, AUR submission) is in progress — see `PHASE8.md`.

## License

MIT — see [`LICENSE`](LICENSE). That covers this repository's own source only; the
bundled fonts (SIL OFL 1.1) and sounds (a mix of CC BY 4.0 / CC0 / Public Domain) are
separately licensed — see [`CREDITS.md`](CREDITS.md) for the full per-file breakdown.
Behaviour is derived from Desktop Destroyer by Miroslav Němeček; no code or asset from the
original is included here.
