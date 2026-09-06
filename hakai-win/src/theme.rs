//! Theme colours — the Windows build's stub.
//!
//! The Linux binary reads the active Omarchy theme via `omarchy-theme-color`. Windows has
//! no coherent system palette worth matching in the same sense (see WINDOWS-PORT.md's
//! non-goals), so this ships the built-in Tokyo-Night-ish fallback and nothing else. Both
//! reader functions return `None`; `main` then keeps `DecalFactory`'s built-in paint
//! palette and `HudColors::FALLBACK`, exactly as the Linux build does off Omarchy.

/// The active theme's HUD chrome colours. Same shape as the Linux `theme::HudColors` so
/// `render.rs` (ported verbatim) compiles unchanged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudColors {
    pub background: (u8, u8, u8),
    pub foreground: (u8, u8, u8),
    pub accent: (u8, u8, u8),
}

impl HudColors {
    /// Flat black panels, flat white text/borders, a light-blue accent — the pre-theming
    /// look, identical to the Linux binary's `HudColors::FALLBACK`.
    pub const FALLBACK: Self =
        Self { background: (0, 0, 0), foreground: (255, 255, 255), accent: (140, 217, 255) };
}

/// Always `None` on Windows — the caller keeps `DecalFactory::DEFAULT_PAINT_COLORS`.
pub fn read_paint_colors() -> Option<[(f32, f32, f32); 8]> {
    None
}

/// Always `None` on Windows — the caller keeps `HudColors::FALLBACK`.
pub fn read_hud_colors() -> Option<HudColors> {
    None
}
