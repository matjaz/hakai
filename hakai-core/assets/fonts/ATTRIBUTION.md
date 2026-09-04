# Bundled fonts

## Archivo Black

- File: `ArchivoBlack-Regular.ttf`
- Source: Google Fonts, `https://fonts.gstatic.com/s/archivoblack/v23/HTxqL289NzCGg4MzN6KJ7eW6OYs.ttf`
- License: SIL Open Font License 1.1 — full text in `ArchivoBlack-OFL.txt`
- Copyright 2017 The Archivo Black Project Authors (https://github.com/Omnibus-Type/ArchivoBlack)
- Used for: the stamp decal's text (`DecalFactory::stamp_print`), standing in for the
  macOS build's `NSFont.systemFont(weight: .black)`. A heavy, condensed sans matches the
  original's rubber-stamp lettering better than a system UI font would anyway.

Regenerate the credits/licence-gate check the macOS build has (`tools/gen_credits.py`,
`make verify-licenses`) once Phase 8 packaging starts — this file is a placeholder for that
mechanism, not a replacement for it.
