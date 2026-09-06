# Bundled fonts

Both families live here in `hakai-core` — the stamp decal (`src/decals.rs`) needs Archivo
Black, and the HUD renderer (`src/render/text.rs`, behind the `render` feature) needs
JetBrains Mono. Each binary picks them up transitively; nothing bundles a font of its own.

## Archivo Black

- File: `ArchivoBlack-Regular.ttf`
- Source: Google Fonts, `https://fonts.gstatic.com/s/archivoblack/v23/HTxqL289NzCGg4MzN6KJ7eW6OYs.ttf`
- License: SIL Open Font License 1.1 — full text in `ArchivoBlack-OFL.txt`
- Copyright 2017 The Archivo Black Project Authors (https://github.com/Omnibus-Type/ArchivoBlack)
- Used for: the stamp decal's text (`DecalFactory::stamp_print`), standing in for the
  macOS build's `NSFont.systemFont(weight: .black)`. A heavy, condensed sans matches the
  original's rubber-stamp lettering better than a system UI font would anyway.

## JetBrains Mono

- Files: `JetBrainsMono-Regular.ttf`, `JetBrainsMono-Bold.ttf`
- Source: Google Fonts,
  `https://fonts.gstatic.com/s/jetbrainsmono/v24/tDbY2o-flEEny0FZhsfKu5WU4zr3E_BX0PnT8RD8yKxjPQ.ttf`
  (regular) and
  `https://fonts.gstatic.com/s/jetbrainsmono/v24/tDbY2o-flEEny0FZhsfKu5WU4zr3E_BX0PnT8RD8L6tjPQ.ttf`
  (bold)
- License: SIL Open Font License 1.1 — full text in `JetBrainsMono-OFL.txt`
- Copyright 2020 The JetBrains Mono Project Authors (https://github.com/JetBrains/JetBrainsMono)
- Used for: the whole HUD — the tool-name label, the toast, the palette's digits and
  tool-name readout, and the credits panel — standing in for the macOS build's
  `NSFont.monospacedSystemFont`. Two weights (regular/bold) rather than Archivo Black's
  single weight, since the HUD actually varies weight (the palette's bold key-digits vs.
  its regular body text) where the stamp decal never did.

Both entries are transcribed into `../../examples/gen_credits.rs`'s `FONTS` table, which
regenerates the repo-root `CREDITS.md` — the actual credits/licence-gate mechanism the
macOS build has as `tools/gen_credits.py` / `make verify-licenses`, ported to Rust for
Phase 8. Keep the two in sync by hand if this file ever changes.
