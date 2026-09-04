//! Tool 4 — the color-thrower.
//!
//! Ported from `ColorThrower.swift`. The least destructive tool: instead of holes it
//! leaves paint splats. The splat itself, and the occasional `paint_drop` sound, happen
//! when a droplet particle expires — see `ParticleKind::Droplet` in `particles.rs`, which
//! is where `splat()`'s logic actually lives now.

use crate::decals::DecalFactory;
use crate::particles::ParticleKind;

use super::{Tool, ToolContext, ToolId};

pub struct ColorThrower {
    since_emit: f32,
    since_shoot_sound: f32,
}

impl ColorThrower {
    const EMIT_INTERVAL: f32 = 0.045;
    /// A droplet's flight time. Short so the response is immediate, but long enough for a
    /// visible arc.
    const FLIGHT_TIME: f32 = 0.17;

    pub fn new() -> Self {
        Self { since_emit: 0.0, since_shoot_sound: 0.0 }
    }
}

impl Default for ColorThrower {
    fn default() -> Self {
        Self::new()
    }
}

fn emit(origin: (f32, f32), ctx: &mut ToolContext) {
    let color_index = ctx.rng.int(0, DecalFactory::DEFAULT_PAINT_COLORS.len() as i64);

    for _ in 0..ctx.rng.int(1, 3) {
        // A target near the cursor — the jet scatters, it doesn't hit one point.
        // Omnidirectional (the angle covers the full circle), so unlike this port's other
        // particle-launch velocities, this one needs no y-flip correction.
        let angle = ctx.rng.range(0.0, std::f64::consts::TAU) as f32;
        let distance = ctx.rng.range(18.0, 120.0) as f32;
        let target = (origin.0 + angle.cos() * distance, origin.1 + angle.sin() * distance);

        let velocity = ((target.0 - origin.0) / ColorThrower::FLIGHT_TIME, (target.1 - origin.1) / ColorThrower::FLIGHT_TIME);
        let side = ctx.rng.range(11.0, 20.0) as f32;
        let spin = ctx.rng.jitter(3.0) as f32;

        // Slight gravity: the droplet flies in an arc, not a straight line.
        ctx.particles.emit(origin, velocity, spin, (side, side), ColorThrower::FLIGHT_TIME, 0.22, None, ParticleKind::Droplet { color_index });
    }
}

impl Tool for ColorThrower {
    fn id(&self) -> ToolId {
        ToolId::ColorThrower
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn mouse_down(&mut self, _point: (f32, f32), _ctx: &mut ToolContext) {
        self.since_emit = Self::EMIT_INTERVAL;
        self.since_shoot_sound = 1.0;
    }

    fn update(&mut self, dt: f32, mouse: (f32, f32), is_down: bool, ctx: &mut ToolContext) {
        if !is_down {
            return;
        }
        self.since_emit += dt;
        self.since_shoot_sound += dt;

        if self.since_emit >= Self::EMIT_INTERVAL {
            self.since_emit = 0.0;
            emit(mouse, ctx);
        }
        // The jet sound is loop-like in character; repeated less often than the droplets.
        if self.since_shoot_sound >= 0.20 {
            self.since_shoot_sound = 0.0;
            let pan = ctx.pan(mouse);
            let volume = ctx.rng.range(0.35, 0.55) as f32;
            ctx.audio.play("paint_shoot", pan, volume);
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
    fn holding_the_button_emits_droplets_that_eventually_splat() {
        let mut env = Env::new(1);
        let mut ct = ColorThrower::new();
        ct.mouse_down((400.0, 300.0), &mut env.ctx());
        for _ in 0..10 {
            ct.update(1.0 / 60.0, (400.0, 300.0), true, &mut env.ctx());
        }
        assert!(env.particles.count() > 0, "droplets should be in flight");

        let before = env.damage.coverage();
        // Flight time is 0.17s — let everything land.
        for _ in 0..30 {
            env.particles.update(1.0 / 60.0, &mut env.damage, &mut env.decals, &mut env.audio, &mut env.rng, (1024.0, 768.0), &mut env.termites);
        }
        assert!(env.damage.coverage() > before, "expired droplets should have splatted paint");
    }

    #[test]
    fn releasing_the_button_stops_emitting() {
        let mut env = Env::new(2);
        let mut ct = ColorThrower::new();
        ct.mouse_down((400.0, 300.0), &mut env.ctx());
        ct.update(1.0 / 60.0, (400.0, 300.0), true, &mut env.ctx());
        let after_first = env.particles.count();
        for _ in 0..10 {
            ct.update(1.0 / 60.0, (400.0, 300.0), false, &mut env.ctx());
        }
        assert_eq!(env.particles.count(), after_first);
    }
}
