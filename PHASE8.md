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
- **Repo hosting: `https://github.com/matjaz/hakai`**, the user's explicit choice, filled
  into `PKGBUILD`'s `url`/`source` (previously clearly-marked `TODO` placeholders rather
  than a fabricated address) and both crates' `Cargo.toml` `repository` fields. Still
  untested end to end with `makepkg` — the repo has to actually be pushed there first.

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

Pushed to `github.com/matjaz/hakai` (`main`, commit `b69276b` at push time). **The repo is
currently private** — `git remote add` used SSH (matching the account's own `gh` protocol
config), which works fine for pushing, but `makepkg`'s anonymous `git+https://...` fetch in
`source=` (and, later, actual AUR builders) needs public read access. Until the repo goes
public, either test locally with a manually-adjusted SSH `source=` URL, or flip visibility
first:

```bash
gh repo edit matjaz/hakai --visibility public   # when ready
cd packaging && makepkg -si
```

## What "done" looks like

- [x] `LICENSE` exists, matches the chosen license, and both `Cargo.toml`s declare it
- [x] `PKGBUILD`'s `url=`/`source=` point at the real repo
- [x] The repo is pushed to `https://github.com/matjaz/hakai` and public
- [x] `PKGBUILD` builds and installs cleanly with `makepkg -si`
- [x] The installed binary runs (`pacman -Q hakai-git`, `which hakai`, `hakai` all
      confirmed on real QEMU/aarch64 Omarchy hardware)
- [ ] The `.desktop` entry's icon actually shows correctly in an app launcher (a pacman
      hook failed regenerating the icon cache — see below; not yet re-verified since)

---

# Phase 8 — chunk 3: the real `makepkg` run — three real bugs, one non-bug

Goal: actually run `makepkg -si` end to end against the pushed repo, not just review the
`PKGBUILD` by reading it. Found three real, worth-fixing bugs and one environment quirk
that looked alarming but wasn't — recorded here in the order they surfaced, since each
one only became visible after the previous one was fixed.

**1. `arch=('x86_64')` — missing `aarch64`.** First error, immediately:
`hakai-git is not available for the 'aarch64' architecture`. This project's own dev/test
box is QEMU/aarch64 (confirmed directly, not assumed), and neither crate has any
architecture-specific code — pure Rust throughout. Added `aarch64` to `arch=()`.

**2. `CARGO_TARGET_DIR="$srcdir/target"` — still on the network mount.** Next error:
`invalid metadata files for crate regex_automata` — the *exact* bug `PHASE0.md` already
documents (a `/mnt`-mounted source directory corrupts cargo's own crate metadata), just a
different crate name than the original `ash` instance. The irony: this override existed
specifically to avoid picking up each crate's own `.cargo/config.toml` (which redirects
`target-dir` to a hardcoded `/home/matjaz/.cache/...` path — wrong for a package build,
since it assumes one specific user), but pointing it at `$srcdir/target` put it right back
on the same mount when `makepkg` itself runs from a mounted `packaging/` directory —
`$srcdir` is `$BUILDDIR/src`, and `$BUILDDIR` defaults to wherever the `PKGBUILD` lives.
Fixed: `CARGO_TARGET_DIR="$HOME/.cache/hakai-git-build-target"` — `$HOME`, not a hardcoded
username, and a real property (off *any* mount, not just "not `$srcdir`") rather than a
coincidence.

**3. `fakeroot`/`chmod` permission denied on `$pkgdir` — a test-environment quirk, not a
`PKGBUILD` bug.** Next error, once the build itself succeeded: `package()`'s `fakeroot`
step failed with `chmod: changing permissions of '.../packaging/pkg': Permission denied`.
Same underlying cause as bug 2 (the network mount doesn't support real chmod/chown
semantics) but a different symptom, and — importantly — **not something to fix in the
`PKGBUILD` itself**: a real `yay -S hakai-git` install builds entirely inside the AUR
helper's own local cache directory (e.g. `~/.cache/yay/hakai-git/`), never touching this
mount at all, so no real end user would ever hit this. It's purely an artifact of testing
`makepkg` directly from this repo's own mounted dev checkout. Worked around for testing
only, via `makepkg`'s own `BUILDDIR` env var (relocates `src`/`pkg` off the mount
entirely): `BUILDDIR="$HOME/.cache/hakai-git-build" makepkg -si`.

**4. Default log level too verbose for a real launch — a real bug, found on the actual
installed binary.** Once installed and run directly (not via `RUST_LOG=info cargo run`,
which is how every single build/test instruction during this project's own development
invoked it), `hakai` printed a wall of `info`-level diagnostic noise ("uploaded N
textures", "HUD panel built", etc.) — exactly the output that was genuinely useful while
building this port, but wrong as a *default* for a normal user launching a packaged app
from a keybind or launcher. Fixed in `hakai/src/main.rs`: the fallback when `RUST_LOG`
isn't set at all is now `warn`, not `info`. `RUST_LOG=info`/`RUST_LOG=debug` still work
exactly as before for anyone who wants that output back.

**Also found, likely benign, not chased further:** `gtk-update-icon-cache: The generated
cache was invalid` from a pacman hook during install. This is standard system machinery
(triggered by any package touching `/usr/share/icons/hicolor/`, not anything in this
package's own `PKGBUILD`) and a known, often-harmless quirk in VM/container environments.
The package itself installed and ran correctly regardless — worth a second look only if
the `.desktop` entry's icon actually fails to show up in a real app launcher, not chased
purely on the hook's own error text.

## What "done" looks like (chunk 3)

- [x] `arch=()` includes `aarch64`
- [x] `CARGO_TARGET_DIR` no longer resolves onto a network mount
- [x] A full `makepkg -si` run succeeds, package installs, `hakai` runs
- [x] Default log level is quiet (`warn`) for a normal launch
- [ ] The `.desktop` entry's icon renders correctly in a real app launcher (pending
      re-verification after the icon-cache hook failure above)
- [x] `Super Shift H` / `Super Shift Alt H` — confirmed live ("works!"); see chunk 4 below
      for a real config-format surprise found getting there

---

# Phase 8 — chunk 4: the Hyprland keybind, on a Lua config — a repeat mistake, caught

Goal: actually verify `Super Shift H` / `Super Shift Alt H` live, not just ship a snippet
and assume it works. It didn't, the first time — and the reason was a mistake this
project had already made once before in this same conversation, just in a different spot.

**What went wrong.** `packaging/hyprland-bindings.conf.example` shipped only the classic
hyprlang form (`bindd = SUPER SHIFT, H, ...`, for `~/.config/hypr/bindings.conf`).
Appending it and running `hyprctl reload` did nothing — no error, just silently no bind.
This Omarchy install is on Hyprland 0.55+'s **Lua** config, the exact same fact that broke
a plain `hyprctl keyword monitor` call earlier in Phase 6/7's fractional-scale work (see
`hakai/PHASE6.md`) — and should have been remembered and accounted for here the first
time, not rediscovered the hard way a second time on the same box.

**The real file and the real API.** `~/.config/hypr/hyprland.lua` `require()`s
`~/.config/hypr/bindings.lua` (via `require("hypr.bindings")`) — `bindings.conf` isn't
loaded by anything once you're on Lua. And Omarchy doesn't expose the raw Hyprland Lua
API (`hl.bind`/`hl.dsp.exec_cmd`) directly for this — it wraps it in its own
`o.bind(keys, description, command)`, confirmed by reading the user's actual
`bindings.lua` (which ships commented-out examples in exactly this form) rather than
guessing from Hyprland's own upstream docs, which only document the raw `hl.*` API.

**Fixed in two places**: the correct bind lines
(`o.bind("SUPER + SHIFT + H", "Destroy the desktop", "uwsm app -- hakai")`, and the `ALT`
kill variant) added to the user's real `bindings.lua` — confirmed working live ("works!").
And, so the next person installing this package doesn't hit the exact same dead end:
`packaging/hyprland-bindings.conf.example` now leads with the Lua/`o.bind` form and keeps
the classic hyprlang form as a clearly-labeled fallback for older, pre-Lua Omarchy
installs — both `README.md` and `OMARCHY-PORT.md`'s own "What makes it Omarchy" section
updated to match.

## What "done" looks like (chunk 4)

- [x] `packaging/hyprland-bindings.conf.example` covers both config formats, correctly
- [x] `README.md` and `OMARCHY-PORT.md` updated to match
- [x] `Super Shift H` launches `hakai`, `Super Shift Alt H` kills it — confirmed live

---

# Phase 8 — chunk 5: CI

Goal: a GitHub Actions workflow (`.github/workflows/ci.yml`) — the one piece of Phase 8
that runs on GitHub's own infrastructure rather than needing the user's Omarchy box, so
it can actually be verified directly rather than handed off.

Two jobs, deliberately scoped differently by confidence:

**`hakai-core`** — high confidence. This crate is headless by design (no Wayland, no
audio, no compositor — see its own crate doc comment), so it runs on a bare
`ubuntu-latest` runner with nothing extra installed: the same `cargo test` /
`cargo run --example gen_credits -- --check` invocations already used throughout local
development, unchanged.

**`hakai-build`** — build-only, lower confidence, honestly scoped. `hakai` needs real
Wayland/xkbcommon/ALSA/Vulkan dev headers to *compile* at all (`smithay-client-toolkit`/
`wayland-backend`/`cpal` all link against or probe for them via their own `build.rs`, even
though the actual libraries are `dlopen`'d at runtime, not link-time) — installed via
`apt-get`. This only confirms the binary *compiles*, since there's no real compositor for
it to run against in CI.

**Explicitly not attempted: the "headless-Hyprland smoke run" from `OMARCHY-PORT.md`'s own
Phase 8 exit criterion.** That's a genuinely harder problem — a nested or virtual
compositor actually accepting `hakai`'s `wlr-layer-shell` surface, not just a `cargo
build` — and guessing at a CI setup for it without being able to test it directly would be
exactly the kind of unverified risk this project has consistently avoided elsewhere.
Flagged as real, deferred work, not silently dropped from scope.

## Verified how

Unlike every other Phase 8 chunk, this one runs on GitHub's own infrastructure — pushed,
then checked directly via `gh run list`/`gh run view` rather than handed to the user's
Omarchy box, since there's nothing Omarchy-specific about compiling.

## What "done" looks like

- [ ] `hakai-core`'s job passes (test + `gen_credits --check`)
- [ ] `hakai-build`'s job passes (`cargo build --release`, dev headers installed)
- [ ] Neither job was guessed at without being able to see its actual result

---

Still open for Phase 8: verifying the app launcher icon shows up correctly, a headless
compositor smoke-test setup for CI (deferred, see chunk 5 above), a dedicated (non-cursor)
app icon, and the actual AUR submission.
