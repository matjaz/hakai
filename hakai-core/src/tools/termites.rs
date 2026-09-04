//! Tool 7 — the termites.
//!
//! Ported from `Termites.swift`, minus the drop-nudge cursor animation. The tool only
//! drops termites; everything else lives in `TermiteColony` (`colony.rs`), because
//! termites have to survive a tool change (`termites will not be switched off`).
//!
//! Only the flame-thrower and the stamp kill them. The washer doesn't remove them either
//! — see `washer.rs`.

use super::{Tool, ToolContext, ToolId};

pub struct Termites {
    since_drop: f32,
    last_drop: (f32, f32),
}

impl Termites {
    const DROP_INTERVAL: f32 = 0.11;
    /// The stroke distance that triggers another drop before the rate interval elapses.
    const DRAG_THRESHOLD: f32 = 24.0;

    pub fn new() -> Self {
        Self { since_drop: 0.0, last_drop: (0.0, 0.0) }
    }

    fn drop(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        self.since_drop = 0.0;
        self.last_drop = point;

        let before = ctx.termites.count();
        for _ in 0..ctx.rng.int(1, 3) {
            let p = (point.0 + ctx.rng.jitter(14.0) as f32, point.1 + ctx.rng.jitter(14.0) as f32);
            ctx.termites.spawn(p, ctx.rng);
        }

        // If the colony is full, don't pretend anything happened.
        if ctx.termites.count() <= before {
            return;
        }
        // A short "plop" on release; the chewing sound itself is driven by the colony.
        if ctx.rng.chance(0.5) {
            let pan = ctx.pan(point);
            let volume = ctx.rng.range(0.3, 0.5) as f32;
            ctx.audio.play("termite_chew", pan, volume);
        }
    }
}

impl Default for Termites {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for Termites {
    fn id(&self) -> ToolId {
        ToolId::Termites
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn mouse_down(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        self.drop(point, ctx);
    }

    fn mouse_dragged(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        let (dx, dy) = (point.0 - self.last_drop.0, point.1 - self.last_drop.1);
        if dx.hypot(dy) >= Self::DRAG_THRESHOLD {
            self.drop(point, ctx);
        }
    }

    fn update(&mut self, dt: f32, mouse: (f32, f32), is_down: bool, ctx: &mut ToolContext) {
        if !is_down {
            return;
        }
        self.since_drop += dt;
        if self.since_drop >= Self::DROP_INTERVAL {
            self.drop(mouse, ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioSink;
    use crate::colony::TermiteColony;
    use crate::damage::DamageLayer;
    use crate::decals::DecalFactory;
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
    fn mouse_down_drops_termites() {
        let mut env = Env::new(1);
        let mut tool = Termites::new();
        tool.mouse_down((400.0, 300.0), &mut env.ctx());
        assert!(env.termites.count() >= 1);
    }

    #[test]
    fn a_full_colony_drops_nothing_further() {
        let mut env = Env::new(2);
        for _ in 0..crate::colony::MAX_COUNT {
            env.termites.spawn((10.0, 10.0), &mut env.rng);
        }
        let mut tool = Termites::new();
        tool.mouse_down((400.0, 300.0), &mut env.ctx());
        assert_eq!(env.termites.count(), crate::colony::MAX_COUNT);
    }

    #[test]
    fn dragging_past_the_threshold_drops_again() {
        let mut env = Env::new(3);
        let mut tool = Termites::new();
        tool.mouse_down((400.0, 300.0), &mut env.ctx());
        let after_first = env.termites.count();
        tool.mouse_dragged((450.0, 300.0), &mut env.ctx()); // 50pt, over the 24pt threshold
        assert!(env.termites.count() > after_first);
    }
}
