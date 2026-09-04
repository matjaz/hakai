//! Omarchy theme integration — reading the active theme's paint palette.
//!
//! Phase 7. `hakai_core` stays theme-agnostic by design (see
//! `DecalFactory::set_paint_colors`'s own doc comment) — this module is the one piece
//! that actually knows Omarchy exists, and hands its result in as plain data.
//!
//! **Why this shells out instead of parsing `colors.toml` directly.** The plan's original
//! assumption — an ANSI 0–7 table inside `alacritty.toml` — turned out stale: checked
//! against the live `omacom/omarchy` repo directly rather than trusted, the real file on
//! the version this was written against is
//! `~/.local/state/omarchy/current/theme/colors.toml`, a flat `key = "value"` palette
//! (`red`, `orange`, `yellow`, `green`, `cyan`, `blue`, `magenta`, `brown`, ...) — but not
//! every theme defines every key. Omarchy's own `bin/omarchy-theme-color` script carries a
//! real amount of fallback logic on top of that raw file (legacy `colorN` aliasing,
//! deriving missing `bright_*`/`orange`/`brown` shades by mixing colours, light/dark mode
//! detection by background luminance) that every first-party Omarchy consumer (Waybar,
//! tmux, GNOME, theme previews) goes through rather than re-deriving by hand. Shelling out
//! to it means this port gets exactly what those consumers get, and stays correct if
//! Omarchy's resolution logic changes again — at the cost of a runtime dependency on that
//! script being on `PATH`, accepted explicitly (see `OMARCHY-PORT.md`'s Phase 7 writeup).

use std::process::Command;

/// Order matches `hakai_core::decals::DecalFactory::DEFAULT_PAINT_COLORS` exactly: red,
/// green, blue, yellow, purple, cyan, orange, pink. Omarchy has no "pink" concept of its
/// own — `bright_magenta`, a lighter/warmer magenta, stands in for it; every other slot
/// has a direct Omarchy name (`purple` itself resolves via Omarchy's own
/// `magenta`↔`purple` alias, so either name works — `purple` is used here since it's the
/// one that also appears as a first-class key in older/simpler themes).
const PAINT_COLOR_KEYS: [&str; 8] = ["red", "green", "blue", "yellow", "purple", "cyan", "orange", "bright_magenta"];

/// Reads the active Omarchy theme's 8 paint colours via `omarchy-theme-color`, in
/// `DecalFactory::DEFAULT_PAINT_COLORS` order. `None` if the tool isn't on `PATH` (a
/// non-Omarchy system, or a build tested off-box), any single query fails, or any value it
/// prints isn't a well-formed `#rrggbb` hex colour — deliberately all-or-nothing, so a
/// caller never ends up with e.g. 7 themed colours and one built-in default sitting oddly
/// next to them. The caller (`main`) falls back to `DecalFactory`'s own built-in default
/// palette in that case, the same "not available, degrade gracefully" shape this port's
/// other optional integrations (screen capture, fractional scale) have used since earlier
/// phases.
pub fn read_paint_colors() -> Option<[(f32, f32, f32); 8]> {
    let mut colors = [(0.0, 0.0, 0.0); 8];
    for (i, key) in PAINT_COLOR_KEYS.iter().enumerate() {
        colors[i] = query_color(key)?;
    }
    Some(colors)
}

/// The active Omarchy theme's HUD chrome colours — panel backgrounds, borders/text, and
/// the accent used for the selected palette cell and the credits panel's "Accent" text
/// colour. `u8` per channel (not the `0.0..=1.0` floats `read_paint_colors` uses): every
/// call site hands these straight to `tiny_skia::Color::from_rgba8`/
/// `TextRenderer::rasterize`, which both want `u8`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudColors {
    pub background: (u8, u8, u8),
    pub foreground: (u8, u8, u8),
    pub accent: (u8, u8, u8),
}

impl HudColors {
    /// What every HUD element already looked like before this chunk — flat black panels,
    /// flat white text/borders, and the credits panel's own hand-picked light blue for its
    /// "Accent" text colour (`(0.55, 0.85, 1.0)`, unchanged in meaning here). Used whenever
    /// `read_hud_colors` can't get a real answer, so behaviour off Omarchy (or if the
    /// resolver script is ever missing) is unchanged from every prior phase.
    pub const FALLBACK: Self = Self { background: (0, 0, 0), foreground: (255, 255, 255), accent: (140, 217, 255) };
}

/// Reads `background`/`foreground`/`accent` from the active Omarchy theme, the same
/// all-or-nothing shape as `read_paint_colors` — `None` on anything short of all three
/// resolving to well-formed hex colours, so the caller falls back to `HudColors::FALLBACK`
/// wholesale rather than mixing themed and default chrome.
pub fn read_hud_colors() -> Option<HudColors> {
    Some(HudColors { background: query_color_u8("background")?, foreground: query_color_u8("foreground")?, accent: query_color_u8("accent")? })
}

fn query_color_u8(key: &str) -> Option<(u8, u8, u8)> {
    let (r, g, b) = query_color(key)?;
    Some(((r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8))
}

fn query_color(key: &str) -> Option<(f32, f32, f32)> {
    let output = Command::new("omarchy-theme-color").arg(key).output().inspect_err(|e| log::warn!("omarchy-theme-color not runnable ({e}) — using the built-in paint palette")).ok()?;
    if !output.status.success() {
        log::warn!("omarchy-theme-color {key} exited with {} — using the built-in paint palette", output.status);
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let color = parse_hex_color(text.trim());
    if color.is_none() {
        log::warn!("omarchy-theme-color {key} printed {text:?}, not a #rrggbb colour — using the built-in paint palette");
    }
    color
}

/// `#rrggbb` (the format every colour `omarchy-theme-color` prints uses) → the
/// `0.0..=1.0` float triples `DecalFactory::DEFAULT_PAINT_COLORS` already uses. No gamma
/// correction here, matching every other hand-picked colour constant already in this port.
fn parse_hex_color(text: &str) -> Option<(f32, f32, f32)> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_well_formed_hex_color() {
        assert_eq!(parse_hex_color("#f7768e"), Some((0xf7 as f32 / 255.0, 0x76 as f32 / 255.0, 0x8e as f32 / 255.0)));
    }

    #[test]
    fn parses_lowercase_and_uppercase_hex_digits() {
        assert_eq!(parse_hex_color("#AABBCC"), parse_hex_color("#aabbcc"));
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(parse_hex_color("f7768e"), None); // missing '#'
        assert_eq!(parse_hex_color("#f7768"), None); // too short
        assert_eq!(parse_hex_color("#f7768ez"), None); // too long
        assert_eq!(parse_hex_color("#gggggg"), None); // not hex digits
        assert_eq!(parse_hex_color(""), None);
    }

    #[test]
    fn eight_keys_in_decal_factory_order() {
        assert_eq!(PAINT_COLOR_KEYS.len(), 8);
    }

    #[test]
    fn fallback_hud_colors_match_the_pre_theming_hardcoded_look() {
        assert_eq!(HudColors::FALLBACK.background, (0, 0, 0));
        assert_eq!(HudColors::FALLBACK.foreground, (255, 255, 255));
        assert_eq!(HudColors::FALLBACK.accent, (140, 217, 255));
    }
}
