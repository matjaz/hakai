# Credits and licences

This file is **generated** — from `hakai-core/assets/manifest.json` and this file's own font table — by `cargo run --example gen_credits` (run from `hakai-core/`). Do not edit it by hand.

## Graphics

Every decal, tool icon, sprite and cursor is **generated procedurally at runtime** (`hakai-core/src/decals.rs`, `icons.rs`, `sprites.rs`) — there are no external image assets, so there is nothing here to attribute.

> The same sound/font information below is also shown inside the app itself — press **C** while it runs — because a file in a source repository doesn't reach someone who installs a build. Both are built from the same `manifest.json` this file reads.

## Fonts

### Archivo Black

- Files: `hakai-core/assets/fonts/ArchivoBlack-Regular.ttf`
- Source: Google Fonts — https://fonts.gstatic.com/s/archivoblack/v23/HTxqL289NzCGg4MzN6KJ7eW6OYs.ttf
- License: SIL Open Font License 1.1 — full text in `hakai-core/assets/fonts/ArchivoBlack-OFL.txt`
- Copyright 2017 The Archivo Black Project Authors (github.com/Omnibus-Type/ArchivoBlack)
- Used for: the stamp decal's baked-in text (`DecalFactory::stamp_print`), standing in for the macOS build's `NSFont.systemFont(weight: .black)`

### JetBrains Mono

- Files: `hakai/assets/fonts/JetBrainsMono-Regular.ttf`, `hakai/assets/fonts/JetBrainsMono-Bold.ttf`
- Source: Google Fonts — https://fonts.gstatic.com/s/jetbrainsmono/v24/tDbY2o-flEEny0FZhsfKu5WU4zr3E_BX0PnT8RD8yKxjPQ.ttf (regular), .../tDbY2o-flEEny0FZhsfKu5WU4zr3E_BX0PnT8RD8L6tjPQ.ttf (bold)
- License: SIL Open Font License 1.1 — full text in `hakai/assets/fonts/JetBrainsMono-OFL.txt`
- Copyright 2020 The JetBrains Mono Project Authors (github.com/JetBrains/JetBrainsMono)
- Used for: the whole HUD — tool-name label, toast, palette digits/readout, credits panel — standing in for the macOS build's `NSFont.monospacedSystemFont`

## Audio

35 sounds in total. All are mono 44.1 kHz 16-bit WAV.

**23 sounds require the author to be named** (CC BY). The credits are below.

### CC BY 4.0 — <https://creativecommons.org/licenses/by/4.0/>

This licence **requires the author to be named**. The credit stays for as long as these sounds are distributed with the app.

| Sound | Author | Source |
|-------|--------|--------|
| `paint_drop` | Gravity Sound | [Drip](https://commons.wikimedia.org/wiki/File:Drip_(Gravity_Sound).wav) |
| `paint_shoot` | Gravity Sound | [Sprinkler hit 4](https://commons.wikimedia.org/wiki/File:Sprinkler_hit_4_(Gravity_Sound).wav) |
| `phaser1` | Gravity Sound | [Laser 1](https://commons.wikimedia.org/wiki/File:Laser_1_(Gravity_Sound).wav) |
| `phaser2` | Gravity Sound | [Laser 8](https://commons.wikimedia.org/wiki/File:Laser_8_(Gravity_Sound).wav) |
| `shell1` | Gravity Sound | [Coins 3](https://commons.wikimedia.org/wiki/File:Coins_3_(Gravity_Sound).mp3) |
| `shell2` | Gravity Sound | [Pebbles dropped on metal frame](https://commons.wikimedia.org/wiki/File:Pebbles_dropped_on_metal_frame_(Gravity_Sound).wav) |
| `shell3` | Gravity Sound | [Dropping spoon](https://commons.wikimedia.org/wiki/File:Dropping_spoon_(Gravity_Sound).wav) |
| `smash1` | Gravity Sound | [Hollow log 2](https://commons.wikimedia.org/wiki/File:Hollow_log_2_(Gravity_Sound).wav) |
| `smash2` | Gravity Sound | [Hollow hit 2](https://commons.wikimedia.org/wiki/File:Hollow_hit_2_(Gravity_Sound).wav) |
| `smash3` | Gravity Sound | [Hollow stick hit 2](https://commons.wikimedia.org/wiki/File:Hollow_stick_hit_2_(Gravity_Sound).wav) |
| `smash4` | Gravity Sound | [Knock on bin 2](https://commons.wikimedia.org/wiki/File:Knock_on_bin_2_(Gravity_Sound).wav) |
| `smash5` | Gravity Sound | [Crash hit 2](https://commons.wikimedia.org/wiki/File:Crash_hit_2_(Gravity_Sound).wav) |
| `smash6` | Gravity Sound | [Rocks hitting together](https://commons.wikimedia.org/wiki/File:Rocks_hitting_together_(Gravity_Sound).wav) |
| `smash7` | Gravity Sound | [Metal pole hit 7](https://commons.wikimedia.org/wiki/File:Metal_pole_hit_7_(Gravity_Sound).wav) |
| `smash8` | Gravity Sound | [Glass breaking 3](https://commons.wikimedia.org/wiki/File:Glass_breaking_3_(Gravity_Sound).wav) |
| `stamp1` | Gravity Sound | [Wump](https://commons.wikimedia.org/wiki/File:Wump_(Gravity_Sound).wav) |
| `stamp2` | Gravity Sound | [Kick 5](https://commons.wikimedia.org/wiki/File:Kick_5_(Gravity_Sound).wav) |
| `termite_chew` | Gravity Sound | [Big bite](https://commons.wikimedia.org/wiki/File:Big_bite_(Gravity_Sound).wav) |
| `termite_crunch` | Gravity Sound | [Crunch 3](https://commons.wikimedia.org/wiki/File:Crunch_3_(Gravity_Sound).wav) |
| `termite_dead` | Gravity Sound | [Stab Damage](https://commons.wikimedia.org/wiki/File:Stab_Damage_(Gravity_Sound).mp3) |
| `termite_squish` | Gravity Sound | [Punch Flesh Damage](https://commons.wikimedia.org/wiki/File:Punch_Flesh_Damage_(Gravity_Sound).mp3) |
| `wash_loop` | Gravity Sound | [River flowing](https://commons.wikimedia.org/wiki/File:River_flowing_(Gravity_Sound).mp3) |
| `wash_start` | Gravity Sound | [Sprinkler hit](https://commons.wikimedia.org/wiki/File:Sprinkler_hit_(Gravity_Sound).wav) |

**Changes made to the originals.** Every file above was processed; none is a verbatim copy:

* converted to mono 44.1 kHz 16-bit, trimmed to the loudest excerpt, closed into a seamless loop with a crossfade, peak-normalised
* converted to mono 44.1 kHz 16-bit, trimmed to the loudest excerpt, faded and peak-normalised

### CC0 — <https://creativecommons.org/publicdomain/zero/1.0/>

| Sound | Author | Source |
|-------|--------|--------|
| `flame_begin` | Desktop Destroyer (procedural synthesis) | `tools/synth_sounds.py` |
| `flame_end` | Desktop Destroyer (procedural synthesis) | `tools/synth_sounds.py` |
| `flame_loop` | Desktop Destroyer (procedural synthesis) | `tools/synth_sounds.py` |
| `mg_reverb` | Desktop Destroyer (procedural synthesis) | `tools/synth_sounds.py` |
| `mg_shot1` | Desktop Destroyer (procedural synthesis) | `tools/synth_sounds.py` |
| `mg_shot2` | Desktop Destroyer (procedural synthesis) | `tools/synth_sounds.py` |
| `mg_shot3` | Desktop Destroyer (procedural synthesis) | `tools/synth_sounds.py` |
| `mg_shot4` | Desktop Destroyer (procedural synthesis) | `tools/synth_sounds.py` |
| `mg_shot5` | Desktop Destroyer (procedural synthesis) | `tools/synth_sounds.py` |
| `mg_shot6` | Desktop Destroyer (procedural synthesis) | `tools/synth_sounds.py` |

### Public domain — <https://en.wikipedia.org/wiki/Public_domain>

| Sound | Author | Source |
|-------|--------|--------|
| `saw_cut` | ezwa | [Chainsaw 3](https://commons.wikimedia.org/wiki/File:Chainsaw_3.ogg) |
| `saw_idle` | ezwa | [Chainsaw 10](https://commons.wikimedia.org/wiki/File:Chainsaw_10.ogg) |

**Changes made to the originals.** Every file above was processed; none is a verbatim copy:

* converted to mono 44.1 kHz 16-bit, trimmed to the loudest excerpt, closed into a seamless loop with a crossfade, peak-normalised

## The original application

Desktop Destroyer / Desktop Games was written by **Miroslav Němeček** (<http://www.breatharian.eu/Petr/en/program/misc.htm>). This project is an independent Linux/Wayland reimplementation, ported in turn from an independent macOS implementation of the same idea. **No asset from the original is included in this repository** — its sounds and bitmaps live in a proprietary `PET` container inside `DESKTOP.EXE`.

