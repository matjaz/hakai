//! Theme colours — the Windows build's stub.
//!
//! The Linux binary reads the active Omarchy theme via `omarchy-theme-color`. Windows has
//! no coherent system palette worth matching in the same sense (see WINDOWS-PORT.md's
//! non-goals), so this ships the built-in Tokyo-Night-ish fallback and nothing else. Both
//! reader functions return `None`; `main` then keeps `DecalFactory`'s built-in paint
//! palette and `HudColors::FALLBACK`, exactly as the Linux build does off Omarchy.

/// Re-exported from `hakai_core` so the renderer (also there) and both binaries agree on
/// one type.
pub use hakai_core::render::theme::HudColors;

/// Always `None` on Windows — the caller keeps `DecalFactory::DEFAULT_PAINT_COLORS`.
pub fn read_paint_colors() -> Option<[(f32, f32, f32); 8]> {
    None
}

/// Always `None` on Windows — the caller keeps `HudColors::FALLBACK`.
pub fn read_hud_colors() -> Option<HudColors> {
    None
}
