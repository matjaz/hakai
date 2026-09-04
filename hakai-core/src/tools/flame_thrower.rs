//! Tool 3 — the flame-thrower.
//!
//! Ported from `FlameThrower.swift`, minus the flying-flame frame animation and the
//! standing flame's spread/fade-out sprite scaling — pure presentation, see the `tools`
//! module doc comment. What's gameplay (a standing flame's lifetime, drift, termite kill,
//! and the scorch mark it leaves when it burns out) is driven by `dt` here exactly as it
//! is in the Swift original — deliberately not through any animation system, since the
//! scorch mark is a damage-layer change and has to be verifiable headlessly.
//!
//! The flame is the only tool that **kills termites**, including ones that wander into a
//! standing flame on their own.

use crate::colony::Cause;
use crate::decals::DecalFactory;

use super::{Tool, ToolContext, ToolId};

struct Flame {
    position: (f32, f32),
    age: f32,
    life: f32,
    drift: (f32, f32),
    /// This flame's own random size multiplier, `ctx.rng.range(0.62, 1.1)` at spawn —
    /// matching the Swift original's `SKSpriteNode(size: CGSize(width: 58 * scale, height:
    /// 100 * scale))`. Stored per-flame (not derived) for the same reason `life_fraction`
    /// is: a renderer needs it, and it's fixed at spawn, not recomputable from `position`
    /// or `age` alone.
    scale: f32,
}

/// A read-only snapshot of one standing flame, for a renderer.
#[derive(Clone, Copy, Debug)]
pub struct FlameView {
    pub position: (f32, f32),
    /// 0 at spawn, 1 at end of life. A renderer derives the spread scale and the
    /// fade-out alpha from this, matching the Swift original's `xScale = 1 + t * 0.35`
    /// and `alpha = t > 0.8 ? max(0, (1 - t) / 0.2) : 1` — dropped as "pure presentation"
    /// back when this tool was first ported (Phase 3, no renderer yet), recoverable now.
    pub life_fraction: f32,
    /// Seconds since this flame was spawned — a renderer uses this to pick a walk-style
    /// animation frame (matching the Swift original's `timePerFrame: 0.07` action) without
    /// needing its own separate per-flame clock.
    pub age: f32,
    /// This flame's spawn-time size multiplier — see `Flame::scale`.
    pub scale: f32,
}

pub struct FlameThrower {
    since_emit: f32,
    burning: bool,
    flames: Vec<Flame>,
}

impl FlameThrower {
    const EMIT_INTERVAL: f32 = 0.05;
    /// The radius within which the flame kills termites.
    const KILL_RADIUS: f32 = 52.0;

    pub fn new() -> Self {
        Self { since_emit: 0.0, burning: false, flames: Vec::new() }
    }

    /// For checks and diagnostics.
    pub fn active_flame_count(&self) -> usize {
        self.flames.len()
    }

    /// Every standing flame, for a renderer to draw.
    pub fn flames(&self) -> impl Iterator<Item = FlameView> + '_ {
        self.flames.iter().map(|f| FlameView {
            position: f.position,
            life_fraction: if f.life > 0.0 { (f.age / f.life).clamp(0.0, 1.0) } else { 0.0 },
            age: f.age,
            scale: f.scale,
        })
    }

    fn spawn_flame(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        let drift_x = ctx.rng.jitter(28.0) as f32;
        // Swift's y range (2..14) is entirely positive — always drifts upward in its y-up
        // scene (flames/heat rise). Negated here for this port's y-down convention.
        let drift_y = -(ctx.rng.range(2.0, 14.0) as f32);
        let position = (point.0 + ctx.rng.jitter(26.0) as f32, point.1 + ctx.rng.jitter(16.0) as f32);
        let life = ctx.rng.range(1.0, 2.1) as f32;
        let scale = ctx.rng.range(0.62, 1.1) as f32;
        self.flames.push(Flame { position, age: 0.0, life, drift: (drift_x, drift_y), scale });
    }

    /// Standing flames keep burning, spread sideways, kill termites and leave a scorch
    /// mark when they go out — even after the user releases the button.
    fn advance_flames(&mut self, dt: f32, ctx: &mut ToolContext) {
        if self.flames.is_empty() {
            return;
        }
        let mut survivors = Vec::with_capacity(self.flames.len());
        for mut flame in self.flames.drain(..) {
            flame.age += dt;
            flame.position.0 += flame.drift.0 * dt;
            flame.position.1 += flame.drift.1 * dt;

            if !ctx.termites.is_empty() {
                ctx.termites.kill(flame.position, Self::KILL_RADIUS * 0.7, Cause::Flame, ctx.damage, ctx.decals, ctx.audio, ctx.rng, ctx.screen_size);
            }

            if flame.age >= flame.life {
                stamp_scorch(flame.position, ctx);
            } else {
                survivors.push(flame);
            }
        }
        self.flames = survivors;
    }
}

impl Default for FlameThrower {
    fn default() -> Self {
        Self::new()
    }
}

fn stamp_scorch(point: (f32, f32), ctx: &mut ToolContext) {
    let size = ctx.rng.range(72.0, 128.0) as f32;
    let variant = ctx.rng.int(0, DecalFactory::SCORCH_VARIANTS);
    let rotation = ctx.rng.range(0.0, std::f64::consts::TAU) as f32;
    let alpha = ctx.rng.range(0.55, 0.9) as f32;
    let decal = ctx.decals.scorch(variant);
    ctx.damage.stamp_ex(decal, point, (size, size), rotation, alpha);
}

impl Tool for FlameThrower {
    fn id(&self) -> ToolId {
        ToolId::FlameThrower
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn mouse_down(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        self.burning = true;
        self.since_emit = Self::EMIT_INTERVAL;
        let pan = ctx.pan(point);
        ctx.audio.play("flame_begin", pan, 0.85);
        ctx.audio.start_loop("flame_loop", "flame", 0.7, pan);
    }

    fn mouse_up(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        if !self.burning {
            return;
        }
        self.burning = false;
        // The loop fades out with a delay so it overlaps the end sound — as in the original.
        ctx.audio.stop_loop("flame", 0.22);
        let pan = ctx.pan(point);
        ctx.audio.play("flame_end", pan, 0.8);
    }

    fn update(&mut self, dt: f32, mouse: (f32, f32), _is_down: bool, ctx: &mut ToolContext) {
        if self.burning {
            self.since_emit += dt;
            if self.since_emit >= Self::EMIT_INTERVAL {
                self.since_emit = 0.0;
                self.spawn_flame(mouse, ctx);
            }
            let pan = ctx.pan(mouse);
            ctx.audio.set_loop("flame", None, Some(pan));
            ctx.termites.kill(mouse, Self::KILL_RADIUS, Cause::Flame, ctx.damage, ctx.decals, ctx.audio, ctx.rng, ctx.screen_size);
        }

        self.advance_flames(dt, ctx);
    }

    fn deactivate(&mut self, ctx: &mut ToolContext) {
        if self.burning {
            self.burning = false;
            ctx.audio.stop_loop("flame", 0.15);
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
    fn burning_spawns_standing_flames() {
        let mut env = Env::new(1);
        let mut ft = FlameThrower::new();
        ft.mouse_down((400.0, 300.0), &mut env.ctx());
        for _ in 0..10 {
            ft.update(1.0 / 60.0, (400.0, 300.0), true, &mut env.ctx());
        }
        assert!(ft.active_flame_count() > 0);
    }

    #[test]
    fn flames_leave_a_scorch_mark_when_they_burn_out() {
        let mut env = Env::new(2);
        let mut ft = FlameThrower::new();
        ft.mouse_down((400.0, 300.0), &mut env.ctx());
        ft.update(1.0 / 60.0, (400.0, 300.0), true, &mut env.ctx()); // one flame spawned
        ft.mouse_up((400.0, 300.0), &mut env.ctx());
        let before = env.damage.coverage();
        // A flame's life is up to 2.1s — burn well past that.
        for _ in 0..(3 * 60) {
            ft.update(1.0 / 60.0, (400.0, 300.0), false, &mut env.ctx());
        }
        assert_eq!(ft.active_flame_count(), 0, "every flame should have gone out by now");
        assert!(env.damage.coverage() > before, "burning out should leave a scorch mark");
    }

    #[test]
    fn burning_kills_nearby_termites() {
        let mut env = Env::new(3);
        for _ in 0..12 {
            env.termites.spawn((400.0, 300.0), &mut env.rng);
        }
        let before = env.termites.count();
        let mut ft = FlameThrower::new();
        ft.mouse_down((400.0, 300.0), &mut env.ctx());
        for _ in 0..30 {
            ft.update(1.0 / 60.0, (400.0, 300.0), true, &mut env.ctx());
        }
        assert!(env.termites.count() < before);
    }

    #[test]
    fn flames_reports_a_position_and_a_life_fraction_that_grows() {
        let mut env = Env::new(4);
        let mut ft = FlameThrower::new();
        ft.mouse_down((400.0, 300.0), &mut env.ctx());
        ft.update(1.0 / 60.0, (400.0, 300.0), true, &mut env.ctx()); // spawns one flame

        let first: Vec<FlameView> = ft.flames().collect();
        assert_eq!(first.len(), 1);
        // Not exactly 0.0: `update()` spawns the flame and then unconditionally runs
        // `advance_flames` within that same call (matching Swift's own ordering), so by
        // the time it's observable the flame is already one `dt` old.
        let just_spawned = first[0].life_fraction;
        assert!((0.0..0.05).contains(&just_spawned), "a freshly spawned flame should be near the very start of its life, got {just_spawned}");

        for _ in 0..20 {
            ft.update(1.0 / 60.0, (400.0, 300.0), true, &mut env.ctx());
        }
        let later: Vec<FlameView> = ft.flames().collect();
        assert!(later.iter().any(|f| f.life_fraction > just_spawned), "life_fraction should have advanced further");
    }
}
