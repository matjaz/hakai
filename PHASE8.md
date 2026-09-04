# Phase 8 — build & run walkthrough (chunk 1: CREDITS.md)

Goal: a regenerated, checkable `CREDITS.md` at the repo root, covering both the bundled
sounds (unchanged in substance from the macOS build's own `tools/gen_credits.py`) and the
two embedded OFL fonts this port bundles that the macOS original never needed — the first
of Phase 8's several packaging deliverables (`PKGBUILD`, CI, the desktop entry, AUR
submission itself are still open).

## Why this exists now, not as a placeholder

Both crates' `assets/fonts/ATTRIBUTION.md` files have said, since the fonts were first
bundled back in the HUD/text-rendering work, "regenerate the credits/licence-gate check
... once Phase 8 packaging starts — this file is a placeholder for that mechanism, not a
replacement for it." This chunk is that mechanism. It matters for a real reason, not just
tidiness: 23 of the 35 bundled sounds are **CC BY 4.0**, which legally requires naming the
author *with the work being distributed* — a `CREDITS.md` nobody who installs a built
package ever sees doesn't satisfy that on its own (the in-app credits panel, `C`, is the
other half — both already existed; this chunk is the file-based half specifically).

## Ported to Rust, not kept as a third Python script

The macOS build's own `tools/gen_credits.py` was the obvious starting point — same
`manifest.json` schema, same grouping-by-licence logic — but this chunk reimplements it as
a new `hakai-core` example (`examples/gen_credits.rs`) rather than adapting the Python
script in place, for two reasons specific to this port:

- Phase 8's own CI is meant to be `cargo test`-only (see `OMARCHY-PORT.md`'s exit
  criterion) — a Python step would be one more thing CI needs installed and one more
  language a contributor needs to touch this file.
- `credits.rs` (the in-app acknowledgements panel) already reads the *identical*
  `assets/manifest.json` file. Two independent parsers of the same schema, one in Python
  and one in Rust, would be two places that could silently disagree about what a field
  means; one Rust parser reading the one bundled file removes that risk entirely, even
  though this generator doesn't literally share code with `credits.rs` (it duplicates the
  small `short_title` helper rather than widen that library crate's public API for a
  one-off tool — six lines, not worth the API surface).

**Genuinely new content, not just a port: the Fonts section.** The macOS build drew every
piece of text with system fonts via CoreText, so `tools/gen_credits.py` never had anything
font-related to say. This port embeds two OFL font families of its own (Archivo Black for
the stamp decal's baked-in text, JetBrains Mono for the whole HUD) — `gen_credits.rs`'s
`FONTS` table is a structured transcription of both `assets/fonts/ATTRIBUTION.md` files,
kept in sync by hand (there are only ever two font families to update, so a JSON manifest
for this specifically would be more ceremony than the problem needs).

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
cargo run --example gen_credits
```

This writes `/mnt/mac/CREDITS.md` (the repo root, one level up from `hakai-core/` — see
the example's own `main` for why: no formal Cargo workspace, so it can't infer a
"workspace root" and takes an explicit relative path instead). Check the file it produces:
a Graphics section, a Fonts section (2 entries), and an Audio section grouped by licence
(CC BY 4.0 first, with the attribution table, then CC0, then Public domain) — 35 sounds
total, 23 requiring attribution.

Then check it's stable and checkable:

```bash
cargo run --example gen_credits -- --check
```

Should print `... is in sync (35 sounds, 2 fonts)` and exit 0, having made no changes —
this is the command Phase 8's eventual CI will run.

## What "done" looks like

- [ ] `hakai-core` builds and tests clean
- [ ] `cargo run --example gen_credits` writes a `CREDITS.md` that reads correctly —
      Graphics, Fonts (2), Audio (35 sounds, grouped by licence, CC BY entries showing
      author/source/modification-notice)
- [ ] `cargo run --example gen_credits -- --check` exits 0 immediately after, with no
      further changes to the file
- [ ] Nothing else regressed

Not yet build-tested — this is the first run of the new generator.

---

# Phase 8 — chunk 2: LICENSE + PKGBUILD + desktop entry

Goal: the rest of what an AUR package needs beyond the binary itself — a declared
license, dependency list, build/install steps, a `.desktop` entry, and the Hyprland
keybind snippet as installable documentation (not auto-applied — see below for why).

## Two decisions that weren't mine to make

Neither this repo nor the original macOS project had ever declared a license, and the
PKGBUILD's `source=`/`url=` fields need a real, fetchable git URL that doesn't exist yet
(this repo has no remote). Rather than pick either silently:

- **License: MIT**, the user's explicit choice. Added `license = "MIT"` to both crates'
  `Cargo.toml`, and a new repo-root `LICENSE` file — with a note that the license covers
  this project's own source only, not the separately-licensed bundled fonts (SIL OFL 1.1)
  and sounds (CC BY 4.0 / CC0 / Public Domain — see `CREDITS.md`, chunk 1).
- **Repo hosting: not decided yet.** `PKGBUILD`'s `url`/`source` fields are clearly-marked
  `TODO` placeholders (`https://github.com/TODO/hakai`) rather than a fabricated address —
  everything else (dependencies, `build()`/`package()` steps, installed file layout) is
  written and reviewable now regardless, and only those two lines need filling in once
  this repo has a real remote.

## `-git`, not a release-tag package

There's no tagged release, and the project's still under active development — a VCS
package (`pkgver()` computing `<Cargo.toml version>.r<commit count>.g<short hash>` from
git at build time, matching the standard `-git` AUR convention) is the right shape until
that changes, the same as most actively-developed Hyprland-ecosystem AUR packages.

## One real bug caught before it could bite a packager

Both crates' own `.cargo/config.toml` redirect `target-dir` to a fixed
`/home/matjaz/.cache/...` path — a genuinely useful workaround *for this project's own
dev setup* (building on a `/mnt`-mounted source directory corrupts cargo's crate metadata,
see `PHASE0.md`), but exactly the kind of thing that would silently break — or worse,
silently leak build artifacts to a hardcoded path outside `makepkg`'s own sandboxed
`$srcdir` — for anyone else building this package, since it assumes one specific user's
home directory. Rather than strip or conditionally patch that file (real risk to the
existing, working dev setup for no benefit), `PKGBUILD`'s `build()` exports
`CARGO_TARGET_DIR` — an environment variable, which overrides a `.cargo/config.toml`'s
`target-dir` per Cargo's own precedence rules — pointed at `$srcdir/target` instead. Same
dev-setup file, correctly ignored during packaging, no changes needed to either.

## The icon is a repurposed cursor icon, flagged as a placeholder

This project's own explicit non-goal is bundled bitmap assets (`OMARCHY-PORT.md`) — every
icon it ships is generated procedurally, including the ones a renderer needs, so a
`.desktop` entry's `Icon=` has nothing to point at until package build time generates one.
`PKGBUILD`'s `build()` runs the existing `hakai-core/examples/dump_icons.rs` (already used
for visual review during development, not new for this chunk) and installs the hammer
tool's icon — the single most recognizable piece of this toy — as `hakai.png`, to both
`/usr/share/pixmaps/` and the hicolor theme's `256x256/apps/` directory. A *dedicated*
app icon, distinct from a repurposed cursor icon (which was designed with an off-center
hotspot/pivot and cursor-sized padding, not centred like a launcher icon expects), is open
follow-up work — flagged here rather than silently shipped as if it were final.

## The Hyprland keybind snippet installs as documentation, not a dotfile edit

`packaging/hyprland-bindings.conf.example` — the same `SUPER SHIFT H` / `SUPER SHIFT ALT
H` snippet recorded in `OMARCHY-PORT.md` — installs to `/usr/share/doc/hakai-git/`, for a
user to copy into their own `~/.config/hypr/bindings.conf` by hand. A package silently
editing a user's own dotfiles on install is exactly the kind of thing that should never
happen without being asked.

## Build & run

Not yet build-tested — `makepkg` needs a real, publicly-fetchable `source=` URL to run at
all, which doesn't exist yet (see above). Once this repo has a remote:

```bash
# fill in packaging/PKGBUILD's url=/source= first
cd packaging && makepkg -si
```

## What "done" looks like

- [ ] `LICENSE` exists, matches the chosen license, and both `Cargo.toml`s declare it
- [ ] `PKGBUILD` builds cleanly with `makepkg` once `url=`/`source=` are filled in
- [ ] The installed binary runs, the `.desktop` entry appears in an app launcher with a
      real (if placeholder) icon, and `/usr/share/doc/hakai-git/` has the credits and the
      keybind snippet
- [ ] `makepkg`'s build never touches `~/.cache/hakai-target`/`~/.cache/hakai-core-target`
      (confirms the `CARGO_TARGET_DIR` override actually works, not just reads correctly)

---

Still open for Phase 8: filling in `PKGBUILD`'s `url=`/`source=` once this repo has a
public remote, CI (`cargo test` for both crates, `gen_credits --check`, plus a
headless-Hyprland smoke run), a dedicated (non-cursor) app icon, and the actual AUR
submission.
