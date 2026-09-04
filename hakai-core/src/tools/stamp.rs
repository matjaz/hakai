//! Tool 6 — the stamp.
//!
//! Ported from `Stamp.swift`, minus the up/down cursor texture swap — pure presentation,
//! see the `tools` module doc comment. Alongside the flame-thrower, the second tool that
//! kills termites — by squashing them.

use crate::colony::Cause;
use crate::decals::DecalFactory;

use super::{Tool, ToolContext, ToolId};

pub struct Stamp {
    since_stamp: f32,
}

impl Stamp {
    const SOUND_VARIANTS: i64 = 2;
    const SQUISH_RADIUS: f32 = 92.0;
    /// The rate while the button is held.
    const REPEAT_INTERVAL: f32 = 0.30;

    pub fn new() -> Self {
        Self { since_stamp: 0.0 }
    }

    fn press(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        self.since_stamp = 0.0;

        // The print is only slightly tilted — a stamp is pressed down, not spun.
        let width = ctx.rng.range(200.0, 265.0) as f32;
        let variant = ctx.rng.int(0, DecalFactory::STAMP_VARIANTS);
        let rotation = ctx.rng.jitter(0.32) as f32;
        let alpha = ctx.rng.range(0.80, 1.0) as f32;
        let decal = ctx.decals.stamp_print(variant);
        ctx.damage.stamp_ex(decal, point, (width, width), rotation, alpha);

        let idx = ctx.rng.int(1, Self::SOUND_VARIANTS + 1);
        let pan = ctx.pan(point);
        let volume = ctx.rng.range(0.7, 0.95) as f32;
        ctx.audio.play(&format!("stamp{idx}"), pan, volume);

        // Whatever is under the stamp gets squashed.
        ctx.termites.kill(point, Self::SQUISH_RADIUS, Cause::Squish, ctx.damage, ctx.decals, ctx.audio, ctx.rng, ctx.screen_size);
    }
}

impl Default for Stamp {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for Stamp {
    fn id(&self) -> ToolId {
        ToolId::Stamp
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn mouse_down(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        self.press(point, ctx);
    }

    fn update(&mut self, dt: f32, mouse: (f32, f32), is_down: bool, ctx: &mut ToolContext) {
        if !is_down {
            return;
        }
        self.since_stamp += dt;
        if self.since_stamp >= Self::REPEAT_INTERVAL {
            self.press(mouse, ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioSink;
    use crate::colony::TermiteColony;
    use crate::damage::DamageLayer;
    use crate::particles::ParticleSystem;
    use crate::rng::SeededRng;

    struct Env {
        damage: DamageLayer,
        decals: DecalFactory,
        particles: ParticleSystem,
        termites: TermiteColony,
        audio: AudioSink,
        rng: SeededRng,
    }
    impl Env {
        fn new(seed: u64) -> Self {
            Self {
                damage: DamageLayer::new(1024.0, 768.0, 1.0),
                decals: DecalFactory::new(),
                particles: ParticleSystem::new(),
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
    fn a_press_marks_the_damage_layer() {
        let mut env = Env::new(1);
        let mut stamp = Stamp::new();
        let before = env.damage.coverage();
        stamp.mouse_down((400.0, 300.0), &mut env.ctx());
        assert!(env.damage.coverage() > before);
    }

    #[test]
    fn a_press_squishes_nearby_termites() {
        let mut env = Env::new(2);
        for _ in 0..12 {
            env.termites.spawn((400.0, 300.0), &mut env.rng);
        }
        let before = env.termites.count();
        let mut stamp = Stamp::new();
        stamp.mouse_down((400.0, 300.0), &mut env.ctx());
        assert!(env.termites.count() < before);
    }
}
