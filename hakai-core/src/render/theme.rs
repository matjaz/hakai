//! HUD chrome colours — the small palette the renderer paints panels, text and accents
//! with. Just the data; each binary's own `theme.rs` does the *reading* (the Linux build
//! shells out to `omarchy-theme-color`, the Windows build has no system palette worth
//! matching and always returns `None`) and hands back an `Option<HudColors>`.

/// Panel background, foreground (text / borders) and accent, each `(r, g, b)` 0–255.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HudColors {
    pub background: (u8, u8, u8),
    pub foreground: (u8, u8, u8),
    pub accent: (u8, u8, u8),
}

impl HudColors {
    /// Flat black panels, flat white text/borders, a light-blue accent — the pre-theming
    /// look, used whenever a binary's theme reader returns `None`.
    pub const FALLBACK: Self =
        Self { background: (0, 0, 0), foreground: (255, 255, 255), accent: (140, 217, 255) };
}
