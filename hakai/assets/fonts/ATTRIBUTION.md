# Bundled fonts

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
  `NSFont.monospacedSystemFont`. Two weights (regular/bold) rather than hakai-core's single
  Archivo Black, since the HUD actually varies weight (the palette's bold key-digits vs.
  its regular body text) where the stamp decal never did.

Regenerate the credits/licence-gate check the macOS build has (`tools/gen_credits.py`,
`make verify-licenses`) once Phase 8 packaging starts — this file is a placeholder for that
mechanism, not a replacement for it.
