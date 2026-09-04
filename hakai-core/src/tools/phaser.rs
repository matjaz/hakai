//! Tool 5 — the phaser.
//!
//! Ported from `Phaser.swift`, minus the muzzle flash and expanding energy ring — pure
//! presentation (`ctx.effects`, never touching gameplay state), see the `tools` module
//! doc comment. It fires into the point under the cursor, so there's no beam to draw.

use crate::decals::DecalFactory;

use super::{Tool, ToolContext, ToolId};

pub struct Phaser {
    since_shot: f32,
}

impl Phaser {
    const SOUND_VARIANTS: i64 = 2;
    /// The minimum time between shots even while the button is held — the phaser isn't a
    /// machine gun.
    const COOLDOWN: f32 = 0.22;

    pub fn new() -> Self {
        Self { since_shot: Self::COOLDOWN }
    }

    fn fire(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        self.since_shot = 0.0;

        let size = ctx.rng.range(88.0, 142.0) as f32;
        let variant = ctx.rng.int(0, DecalFactory::PHASER_VARIANTS);
        let rotation = ctx.rng.range(0.0, std::f64::consts::TAU) as f32;
        let decal = ctx.decals.phaser_hit(variant);
        ctx.damage.stamp_ex(decal, point, (size, size), rotation, 1.0);

        let idx = ctx.rng.int(1, Self::SOUND_VARIANTS + 1);
        let pan = ctx.pan(point);
        let volume = ctx.rng.range(0.7, 0.95) as f32;
        ctx.audio.play(&format!("phaser{idx}"), pan, volume);
    }
}

impl Default for Phaser {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for Phaser {
    fn id(&self) -> ToolId {
        ToolId::Phaser
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn mouse_down(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        self.fire(point, ctx);
    }

    fn update(&mut self, dt: f32, mouse: (f32, f32), is_down: bool, ctx: &mut ToolContext) {
        self.since_shot += dt;
        if !is_down || self.since_shot < Self::COOLDOWN {
            return;
        }
        self.fire(mouse, ctx);
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
    fn a_shot_marks_the_damage_layer() {
        let mut env = Env::new(1);
        let mut phaser = Phaser::new();
        let before = env.damage.coverage();
        phaser.mouse_down((400.0, 300.0), &mut env.ctx());
        assert!(env.damage.coverage() > before);
    }

    #[test]
    fn the_cooldown_limits_the_fire_rate() {
        let mut env = Env::new(2);
        let mut phaser = Phaser::new();
        phaser.mouse_down((400.0, 300.0), &mut env.ctx());
        let after_first = env.damage.coverage();
        phaser.update(0.05, (400.0, 300.0), true, &mut env.ctx()); // well under 0.22s cooldown
        assert_eq!(env.damage.coverage(), after_first, "a shot within the cooldown shouldn't fire");
        phaser.update(0.20, (400.0, 300.0), true, &mut env.ctx()); // 0.25s total, past cooldown
        assert!(env.damage.coverage() > after_first);
    }
}
