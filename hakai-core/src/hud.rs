//! The HUD's pure-logic state: what's visible, how far into its fade it is, and what a
//! transient toast currently says — everything a renderer needs to know *what* to show and
//! *when*, without touching text shaping, pixel layout, or the GPU.
//!
//! Ported from the HUD-related pieces of `ToolPaletteHUD.swift` (visibility, the
//! flash-if-hidden peek) and `GameScene.swift` (`flash(_:)`, the toast message). The tool
//! palette's *cell layout* and *hit-testing* (`ToolPaletteHUD.tool(at:)`) deliberately
//! aren't here — they depend on exact pixel geometry a renderer computes, the same reason
//! `icon_metadata`/`icon_variant_key` live in `hakai`'s `main.rs` rather than here.
//!
//! **Recomputed from elapsed time, not one-shot actions.** Every fade below is an
//! `SKAction` in the original; each becomes a small enum plus a clock advanced by
//! `advance(dt)`, the same move already made for `Hammer::cursor_rotation` and
//! `FlameThrower`'s standing flames — testable headlessly, no scene required.
//!
//! **Interrupting a fade restarts it from empty, not from wherever the previous fade had
//! gotten to.** `SKAction.fadeAlpha(to:duration:)` animates from whatever alpha the node
//! is *actually* at when it starts, which depends on exactly when the interrupting call
//! lands — not reproducible as a pure function of elapsed time without tracking a
//! continuous alpha value across every interruption. Simplified the same way this port
//! already simplified the machine gun's recoil kick if it's retriggered mid-animation:
//! correctness of the common case (fades essentially never overlap; a person would have to
//! hit ↑ and ↓ within 140ms of each other) over exact fidelity to a rare edge case.

/// How far into its fade the tool palette is, and why.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PaletteAnim {
    Hidden,
    Shown,
    /// Seconds since `set_visible(true)`.
    FadingIn(f32),
    /// Seconds since `set_visible(false)`.
    FadingOut(f32),
    /// Seconds since `flash_if_hidden()` — independent of `FadingIn`/`FadingOut`, matching
    /// `ToolPaletteHUD.swift`'s `flashIfHidden` animating the node directly rather than
    /// going through `isVisible`/`setVisible` at all.
    Flashing(f32),
}

const PALETTE_FADE: f32 = 0.14;
const FLASH_IN: f32 = 0.10;
const FLASH_HOLD: f32 = 0.9;
const FLASH_OUT: f32 = 0.3;
const FLASH_TOTAL: f32 = FLASH_IN + FLASH_HOLD + FLASH_OUT;

const TOAST_HOLD: f32 = 1.4;
const TOAST_FADE: f32 = 0.4;

const CREDITS_FADE: f32 = 0.12;

#[derive(Clone, Copy, Debug, PartialEq)]
enum CreditsAnim {
    Hidden,
    Shown,
    FadingIn(f32),
    FadingOut(f32),
}

pub struct Hud {
    /// Mirrors `ToolPaletteHUD.isVisible` — the *target* of the last explicit toggle, set
    /// synchronously (unlike the animation, which catches up over `PALETTE_FADE`). Hit
    /// testing (`ToolPaletteHUD.tool(at:)`) reads this directly, not the animation state —
    /// a palette mid-fade-in is already clickable in the original.
    palette_open: bool,
    palette_anim: PaletteAnim,

    toast: Option<String>,
    since_toast: f32,

    credits_open: bool,
    credits_anim: CreditsAnim,
}

impl Hud {
    pub fn new() -> Self {
        Self {
            palette_open: false,
            palette_anim: PaletteAnim::Hidden,
            toast: None,
            since_toast: 0.0,
            credits_open: false,
            credits_anim: CreditsAnim::Hidden,
        }
    }

    // MARK: - Palette

    pub fn palette_open(&self) -> bool {
        self.palette_open
    }

    /// `ToolPaletteHUD.swift`'s `setVisible(_:animated:)` (always animated here — nothing
    /// in this port's input handling ever wants the instant, non-animated variant Swift
    /// only used for the `--simulate`-style demo script).
    pub fn set_palette_visible(&mut self, visible: bool) {
        if self.palette_open == visible {
            return;
        }
        self.palette_open = visible;
        self.palette_anim = if visible { PaletteAnim::FadingIn(0.0) } else { PaletteAnim::FadingOut(0.0) };
    }

    pub fn toggle_palette(&mut self) {
        self.set_palette_visible(!self.palette_open);
    }

    /// A brief glimpse of the palette when the tool is changed from the keyboard — it
    /// confirms what's selected without having to open the palette by hand.
    /// `ToolPaletteHUD.swift`'s `flashIfHidden`.
    pub fn flash_palette(&mut self) {
        if self.palette_open {
            return;
        }
        self.palette_anim = PaletteAnim::Flashing(0.0);
    }

    /// The palette's current opacity, 0..1 — a renderer skips drawing it entirely at 0.
    pub fn palette_alpha(&self) -> f32 {
        match self.palette_anim {
            PaletteAnim::Hidden => 0.0,
            PaletteAnim::Shown => 1.0,
            PaletteAnim::FadingIn(t) => (t / PALETTE_FADE).clamp(0.0, 1.0),
            PaletteAnim::FadingOut(t) => 1.0 - (t / PALETTE_FADE).clamp(0.0, 1.0),
            PaletteAnim::Flashing(t) => {
                if t < FLASH_IN {
                    t / FLASH_IN
                } else if t < FLASH_IN + FLASH_HOLD {
                    1.0
                } else {
                    (1.0 - (t - FLASH_IN - FLASH_HOLD) / FLASH_OUT).clamp(0.0, 1.0)
                }
            }
        }
    }

    // MARK: - Toast

    /// `GameScene.swift`'s `flash(_:)` — an unfortunate name clash with the palette's own
    /// "flash," which is why this crate calls the message one a toast instead.
    pub fn show_toast(&mut self, message: impl Into<String>) {
        self.toast = Some(message.into());
        self.since_toast = 0.0;
    }

    /// The current toast text, if any is still visible (`palette_alpha`-style callers
    /// should treat `None` as "don't draw").
    pub fn toast(&self) -> Option<(&str, f32)> {
        let text = self.toast.as_deref()?;
        let alpha = if self.since_toast < TOAST_HOLD {
            1.0
        } else {
            (1.0 - (self.since_toast - TOAST_HOLD) / TOAST_FADE).clamp(0.0, 1.0)
        };
        if alpha <= 0.0 {
            return None;
        }
        Some((text, alpha))
    }

    // MARK: - Credits

    pub fn credits_open(&self) -> bool {
        self.credits_open
    }

    pub fn toggle_credits(&mut self) {
        self.credits_open = !self.credits_open;
        self.credits_anim = if self.credits_open { CreditsAnim::FadingIn(0.0) } else { CreditsAnim::FadingOut(0.0) };
    }

    pub fn credits_alpha(&self) -> f32 {
        match self.credits_anim {
            CreditsAnim::Hidden => 0.0,
            CreditsAnim::Shown => 1.0,
            CreditsAnim::FadingIn(t) => (t / CREDITS_FADE).clamp(0.0, 1.0),
            CreditsAnim::FadingOut(t) => 1.0 - (t / CREDITS_FADE).clamp(0.0, 1.0),
        }
    }

    // MARK: - Per-frame

    /// Advances every clock above by `dt`. Call once per frame regardless of visibility —
    /// a hidden/fully-faded element just has nothing left to advance.
    pub fn advance(&mut self, dt: f32) {
        self.palette_anim = match self.palette_anim {
            PaletteAnim::FadingIn(t) => {
                let t = t + dt;
                if t >= PALETTE_FADE { PaletteAnim::Shown } else { PaletteAnim::FadingIn(t) }
            }
            PaletteAnim::FadingOut(t) => {
                let t = t + dt;
                if t >= PALETTE_FADE { PaletteAnim::Hidden } else { PaletteAnim::FadingOut(t) }
            }
            PaletteAnim::Flashing(t) => {
                let t = t + dt;
                if t >= FLASH_TOTAL { PaletteAnim::Hidden } else { PaletteAnim::Flashing(t) }
            }
            other => other,
        };

        self.credits_anim = match self.credits_anim {
            CreditsAnim::FadingIn(t) => {
                let t = t + dt;
                if t >= CREDITS_FADE { CreditsAnim::Shown } else { CreditsAnim::FadingIn(t) }
            }
            CreditsAnim::FadingOut(t) => {
                let t = t + dt;
                if t >= CREDITS_FADE { CreditsAnim::Hidden } else { CreditsAnim::FadingOut(t) }
            }
            other => other,
        };

        if self.toast.is_some() {
            self.since_toast += dt;
            if self.since_toast >= TOAST_HOLD + TOAST_FADE {
                self.toast = None;
            }
        }
    }
}

impl Default for Hud {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_hud_is_fully_hidden() {
        let hud = Hud::new();
        assert!(!hud.palette_open());
        assert_eq!(hud.palette_alpha(), 0.0);
        assert!(!hud.credits_open());
        assert_eq!(hud.credits_alpha(), 0.0);
        assert!(hud.toast().is_none());
    }

    #[test]
    fn setting_the_palette_visible_fades_it_in_over_time() {
        let mut hud = Hud::new();
        hud.set_palette_visible(true);
        assert!(hud.palette_open());
        assert_eq!(hud.palette_alpha(), 0.0, "should start the fade at 0, not jump straight to 1");
        for _ in 0..20 {
            hud.advance(1.0 / 60.0);
        }
        assert_eq!(hud.palette_alpha(), 1.0, "well past the fade duration, should be fully shown");
    }

    #[test]
    fn hiding_the_palette_fades_it_back_out() {
        let mut hud = Hud::new();
        hud.set_palette_visible(true);
        for _ in 0..20 {
            hud.advance(1.0 / 60.0);
        }
        hud.set_palette_visible(false);
        assert!(!hud.palette_open());
        assert_eq!(hud.palette_alpha(), 1.0, "should start fading out from fully visible");
        for _ in 0..20 {
            hud.advance(1.0 / 60.0);
        }
        assert_eq!(hud.palette_alpha(), 0.0);
    }

    #[test]
    fn toggle_flips_the_current_state() {
        let mut hud = Hud::new();
        hud.toggle_palette();
        assert!(hud.palette_open());
        hud.toggle_palette();
        assert!(!hud.palette_open());
    }

    #[test]
    fn setting_the_same_visibility_twice_does_nothing() {
        let mut hud = Hud::new();
        hud.set_palette_visible(true);
        for _ in 0..20 {
            hud.advance(1.0 / 60.0);
        }
        assert_eq!(hud.palette_alpha(), 1.0);
        hud.set_palette_visible(true); // already open — should be a no-op, not restart the fade
        assert_eq!(hud.palette_alpha(), 1.0);
    }

    #[test]
    fn flashing_while_hidden_shows_then_hides_the_palette_without_opening_it() {
        let mut hud = Hud::new();
        hud.flash_palette();
        assert!(!hud.palette_open(), "a flash shouldn't make the palette actually open (clickable)");
        for _ in 0..6 {
            hud.advance(1.0 / 60.0);
        }
        assert!(hud.palette_alpha() > 0.0, "should be visible partway through the flash");

        for _ in 0..90 {
            hud.advance(1.0 / 60.0);
        }
        assert_eq!(hud.palette_alpha(), 0.0, "should have faded back out once the flash finishes");
    }

    #[test]
    fn flashing_while_already_open_does_nothing() {
        let mut hud = Hud::new();
        hud.set_palette_visible(true);
        for _ in 0..20 {
            hud.advance(1.0 / 60.0);
        }
        hud.flash_palette();
        assert_eq!(hud.palette_alpha(), 1.0, "flashing an already-open palette shouldn't restart or disturb it");
    }

    #[test]
    fn a_toast_is_visible_then_fades_then_clears() {
        let mut hud = Hud::new();
        hud.show_toast("Desktop cleaned");
        let (text, alpha) = hud.toast().unwrap();
        assert_eq!(text, "Desktop cleaned");
        assert_eq!(alpha, 1.0);

        for _ in 0..(90) {
            // 1.5s — past TOAST_HOLD, partway into the fade
            hud.advance(1.0 / 60.0);
        }
        let (_, alpha) = hud.toast().expect("should still be fading out, not gone yet");
        assert!(alpha < 1.0 && alpha > 0.0);

        for _ in 0..90 {
            hud.advance(1.0 / 60.0);
        }
        assert!(hud.toast().is_none(), "should be fully gone well past the hold+fade");
    }

    #[test]
    fn showing_a_new_toast_restarts_the_timer() {
        let mut hud = Hud::new();
        hud.show_toast("first");
        for _ in 0..90 {
            hud.advance(1.0 / 60.0);
        }
        hud.show_toast("second");
        let (text, alpha) = hud.toast().unwrap();
        assert_eq!(text, "second");
        assert_eq!(alpha, 1.0, "a fresh toast should start fully visible even if the previous one was fading");
    }

    #[test]
    fn credits_toggle_fades_in_and_out() {
        let mut hud = Hud::new();
        hud.toggle_credits();
        assert!(hud.credits_open());
        assert_eq!(hud.credits_alpha(), 0.0);
        for _ in 0..20 {
            hud.advance(1.0 / 60.0);
        }
        assert_eq!(hud.credits_alpha(), 1.0);

        hud.toggle_credits();
        assert!(!hud.credits_open());
        for _ in 0..20 {
            hud.advance(1.0 / 60.0);
        }
        assert_eq!(hud.credits_alpha(), 0.0);
    }
}
