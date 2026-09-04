//! Tool 0 — the hammer.
//!
//! Ported from `Hammer.swift`. The cursor swing animation (`cursor_rotation`) was
//! initially left out too — see the `tools` module doc comment for why every tool's
//! `SKAction` animations started out of scope — but recovered once a renderer existed to
//! feed, the same way `FlameThrower`'s `flames()`/`FlameView` were. Eight crack variants,
//! eight sound variants chosen by the brightness of the surface, and splinters that fly
//! off the impact.

use crate::decals::DecalFactory;
use crate::particles::ParticleKind;

use super::{Tool, ToolContext, ToolId};

pub struct Hammer {
    /// Repeated strikes while the button is held.
    since_last_hit: f32,
    last_hit_point: (f32, f32),
    /// Seconds since the last strike, for `cursor_rotation` — deliberately a *separate*
    /// clock from `since_last_hit`, which stops advancing while the button is up (it only
    /// exists to gate auto-repeat). The knock-animation clock has to keep advancing
    /// regardless, so a single click's cursor still lifts back out of the impact pose
    /// instead of freezing there the instant the button is released.
    since_strike: f32,
}

impl Hammer {
    const REPEAT_INTERVAL: f32 = 0.22;
    const DRAG_THRESHOLD: f32 = 70.0;
    /// `duration: 0.16` in `Hammer.swift`'s `animateKnock`. (A previous pass here tried
    /// slowing this down and easing it differently in response to "still looks rough"
    /// feedback — that was the wrong fix: the real bug was the rotation *direction* being
    /// mirrored, below, not the timing. Reverted to the literal Swift value once the real
    /// bug was found, rather than stacking a second, unrelated deviation on top of it.)
    const KNOCK_DURATION: f32 = 0.16;
    /// The angle of the raised hammer at rest — `Hammer.swift`'s `raisedRotation`.
    const RAISED_ROTATION: f32 = -0.42;
    /// The angle at which the striking face is exactly on the cursor —
    /// `Hammer.swift`'s `impactRotation`.
    const IMPACT_ROTATION: f32 = 0.0;

    pub fn new() -> Self {
        // `since_strike` starts already-finished (never struck yet), so a fresh hammer
        // renders at rest (`RAISED_ROTATION`) rather than mid-swing.
        Self { since_last_hit: 0.0, last_hit_point: (0.0, 0.0), since_strike: f32::INFINITY }
    }

    /// The cursor's current swing angle: snaps to `IMPACT_ROTATION` on every strike, then
    /// eases back out to `RAISED_ROTATION` over `KNOCK_DURATION` — `Hammer.swift`'s
    /// `animateKnock`, recomputed as a pure function of elapsed time on every call instead
    /// of run once as a one-shot `SKAction`, so this (like everything else in this crate)
    /// stays testable headlessly rather than needing a live scene to observe.
    pub fn cursor_rotation(&self) -> f32 {
        let t = if self.since_strike >= Self::KNOCK_DURATION { 1.0 } else { (self.since_strike / Self::KNOCK_DURATION).clamp(0.0, 1.0) };
        // Quadratic ease-out, matching `SKAction`'s `.timingMode = .easeOut`.
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        let swift_rotation = Self::IMPACT_ROTATION + (Self::RAISED_ROTATION - Self::IMPACT_ROTATION) * eased;

        // `RAISED_ROTATION`/`IMPACT_ROTATION` are `Hammer.swift`'s own `zRotation`
        // values — SpriteKit's y-up, counter-clockwise-positive convention. Handing that
        // same numeric angle to a rotation computed in this port's y-down pixel space
        // (`rotated_sprite_ndc_pivot` in `main.rs`) mirrors it vertically: with an
        // off-centre pivot (the grip, not the icon's middle — see `icons.rs`'s hammer
        // pivot), a mirrored rotation doesn't just run backwards, it sends the head
        // through a visually wrong arc entirely, which is what actually made this "look
        // rough" — not the timing. Same class of fix as everywhere else in this crate
        // that crosses this boundary (particle gravity/launch velocities, decal drips,
        // the termite's own facing direction) — negated once, here, rather than at every
        // call site that reads `cursor_rotation()`.
        -swift_rotation
    }

    fn strike(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        self.since_last_hit = 0.0;
        self.since_strike = 0.0;
        self.last_hit_point = point;

        // `sound by color brightness below mouse cursor` — the impact variant follows the
        // brightness of the surface, not chance. This is the mechanic that makes the
        // original sound "alive".
        let smash = crate::audio::smash_name(ctx.brightness, ctx.rng);
        let pan = ctx.pan(point);
        let volume = ctx.rng.range(0.85, 1.0) as f32;
        ctx.audio.play(smash, pan, volume);

        let variant = ctx.rng.int(0, DecalFactory::CRACK_VARIANTS);
        let side = ctx.rng.range(140.0, 205.0) as f32;
        let rotation = ctx.rng.range(0.0, std::f64::consts::TAU) as f32;
        let alpha = ctx.rng.range(0.85, 1.0) as f32;
        let decal = ctx.decals.crack(variant);
        ctx.damage.stamp_ex(decal, point, (side, side), rotation, alpha);

        self.emit_slivers(point, ctx);
    }

    /// The splinters that fly off the impact.
    fn emit_slivers(&self, point: (f32, f32), ctx: &mut ToolContext) {
        let count = ctx.rng.int(4, 9);
        for _ in 0..count {
            let variant = ctx.rng.int(0, DecalFactory::SLIVER_VARIANTS);
            let angle = ctx.rng.range(0.0, std::f64::consts::TAU) as f32;
            let speed = ctx.rng.range(140.0, 430.0) as f32;
            let s = ctx.rng.range(7.0, 15.0) as f32;
            let spin = ctx.rng.jitter(9.0) as f32;
            let life = ctx.rng.range(0.5, 1.1) as f32;
            // `y: abs(sin(angle)) * speed + 120` in the Swift original always launches
            // upward (positive y in its y-up scene) — negated here to mean the same
            // "upward" in this port's y-down convention. See `particles.rs`'s doc comment.
            let velocity = (angle.cos() * speed, -(angle.sin().abs() * speed + 120.0));
            ctx.particles.emit(point, velocity, spin, (s, s), life, 1.0, None, ParticleKind::Generic { variant });
        }
    }
}

impl Default for Hammer {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for Hammer {
    fn id(&self) -> ToolId {
        ToolId::Hammer
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn mouse_down(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        self.strike(point, ctx);
    }

    fn mouse_dragged(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        let (dx, dy) = (point.0 - self.last_hit_point.0, point.1 - self.last_hit_point.1);
        if dx.hypot(dy) >= Self::DRAG_THRESHOLD {
            self.strike(point, ctx);
        }
    }

    fn update(&mut self, dt: f32, mouse: (f32, f32), is_down: bool, ctx: &mut ToolContext) {
        // Always advances — unlike `since_last_hit` below, this drives `cursor_rotation`,
        // which has to keep animating (and finish) after the button comes back up.
        self.since_strike += dt;

        if !is_down {
            return;
        }
        self.since_last_hit += dt;
        if self.since_last_hit >= Self::REPEAT_INTERVAL {
            self.strike(mouse, ctx);
        }
    }

    fn deactivate(&mut self, _ctx: &mut ToolContext) {
        self.since_last_hit = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioSink;
    use crate::colony::TermiteColony;
    use crate::damage::DamageLayer;
    use crate::rng::SeededRng;

    struct Env {
        damage: DamageLayer,
        decals: DecalFactory,
        particles: crate::particles::ParticleSystem,
        termites: TermiteColony,
        audio: AudioSink,
        rng: SeededRng,
    }
    impl Env {
        fn new(seed: u64) -> Self {
            Self {
                damage: DamageLayer::new(1024.0, 768.0, 1.0),
                decals: DecalFactory::new(),
                particles: crate::particles::ParticleSystem::new(),
                termites: TermiteColony::new(),
                audio: AudioSink::new(),
                rng: SeededRng::new(seed),
            }
        }
        fn ctx(&mut self) -> ToolContext<'_> {
            ToolContext {
                damage: &mut self.damage,
                decals: &mut self.decals,
                particles: &mut self.particles,
                termites: &mut self.termites,
                audio: &mut self.audio,
                screen_size: (1024.0, 768.0),
                rng: &mut self.rng,
                brightness: Some(128),
            }
        }
    }

    #[test]
    fn a_strike_marks_the_damage_layer_and_emits_slivers() {
        let mut env = Env::new(1);
        let mut hammer = Hammer::new();
        let before = env.damage.coverage();
        hammer.mouse_down((400.0, 300.0), &mut env.ctx());
        assert!(env.damage.coverage() > before, "a strike should crack the desktop");
        assert!(env.particles.count() > 0, "a strike should emit slivers");
    }

    #[test]
    fn dragging_below_the_threshold_does_not_strike_again() {
        let mut env = Env::new(2);
        let mut hammer = Hammer::new();
        hammer.mouse_down((400.0, 300.0), &mut env.ctx());
        let after_first = env.particles.count();
        hammer.mouse_dragged((410.0, 300.0), &mut env.ctx()); // well under the 70pt threshold
        assert_eq!(env.particles.count(), after_first, "a small drag shouldn't trigger another strike");
    }

    #[test]
    fn dragging_past_the_threshold_strikes_again() {
        let mut env = Env::new(3);
        let mut hammer = Hammer::new();
        hammer.mouse_down((400.0, 300.0), &mut env.ctx());
        let after_first = env.particles.count();
        hammer.mouse_dragged((500.0, 300.0), &mut env.ctx()); // 100pt, over the threshold
        assert!(env.particles.count() > after_first, "a large drag should trigger another strike");
    }

    #[test]
    fn holding_the_button_repeats_at_the_interval() {
        let mut env = Env::new(4);
        let mut hammer = Hammer::new();
        hammer.mouse_down((400.0, 300.0), &mut env.ctx());
        let after_first = env.particles.count();
        // Just under the repeat interval: no second strike yet.
        hammer.update(0.20, (400.0, 300.0), true, &mut env.ctx());
        assert_eq!(env.particles.count(), after_first);
        // Past it: a second strike.
        hammer.update(0.05, (400.0, 300.0), true, &mut env.ctx());
        assert!(env.particles.count() > after_first);
    }

    #[test]
    fn cursor_rotation_starts_at_rest() {
        let hammer = Hammer::new();
        // `cursor_rotation()` returns `-RAISED_ROTATION`/`-IMPACT_ROTATION`, not the raw
        // Swift constants directly — see its doc comment for the y-up/y-down flip.
        assert_eq!(hammer.cursor_rotation(), -Hammer::RAISED_ROTATION, "a fresh hammer should render in its resting pose, not mid-swing");
    }

    #[test]
    fn a_strike_snaps_the_cursor_to_impact_then_it_eases_back_even_after_release() {
        let mut env = Env::new(6);
        let mut hammer = Hammer::new();
        hammer.mouse_down((400.0, 300.0), &mut env.ctx());
        assert_eq!(hammer.cursor_rotation(), -Hammer::IMPACT_ROTATION, "the cursor should snap onto the strike instantly");

        // Advancing with the button *up* — the knock animation still has to finish, since
        // `since_strike` (unlike `since_last_hit`) isn't gated on `is_down`.
        hammer.update(Hammer::KNOCK_DURATION / 2.0, (400.0, 300.0), false, &mut env.ctx());
        let halfway = hammer.cursor_rotation();
        assert!(
            halfway > -Hammer::IMPACT_ROTATION && halfway < -Hammer::RAISED_ROTATION,
            "halfway through the knock, the cursor should be partway between impact and raised, got {halfway}"
        );

        hammer.update(1.0, (400.0, 300.0), false, &mut env.ctx());
        assert_eq!(hammer.cursor_rotation(), -Hammer::RAISED_ROTATION, "well past the knock duration, the cursor should be back at rest");
    }

    #[test]
    fn releasing_the_button_stops_the_repeat() {
        let mut env = Env::new(5);
        let mut hammer = Hammer::new();
        hammer.mouse_down((400.0, 300.0), &mut env.ctx());
        let after_first = env.particles.count();
        hammer.update(1.0, (400.0, 300.0), false, &mut env.ctx());
        assert_eq!(env.particles.count(), after_first, "update() with isDown=false must not strike");
    }
}
