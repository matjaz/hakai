//! Tool 8 — the washer.
//!
//! Ported from `Washer.swift`, minus the spray-mist sprite — pure presentation (it's the
//! *only* thing `Washer.update` did in Swift, so this port has no `update` override at
//! all). The only tool that repairs: it erases along the stroke.
//!
//! It does **not** remove living termites — this reading treats a termite as blocking
//! erasure underneath it, so termites have to be burned or squished first before the
//! desktop under them can be cleaned. See `TermiteColony::has_living`.

use super::{along_path, Tool, ToolContext, ToolId};

pub struct Washer {
    last_point: Option<(f32, f32)>,
    washing: bool,
    /// How many spots this stroke skipped because of termites — not shown anywhere yet,
    /// but useful for checks and while debugging.
    blocked_by_termites: usize,
}

impl Washer {
    const ERASE_RADIUS: f32 = 36.0;
    const ERASE_SPACING: f32 = 14.0;
    /// The radius within which a living termite prevents erasing.
    const TERMITE_BLOCK_RADIUS: f32 = 30.0;

    pub fn new() -> Self {
        Self { last_point: None, washing: false, blocked_by_termites: 0 }
    }

    pub fn blocked_by_termites(&self) -> usize {
        self.blocked_by_termites
    }
}

impl Default for Washer {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns whether this spot was blocked by a living termite (and so was *not* erased).
fn wipe(point: (f32, f32), ctx: &mut ToolContext) -> bool {
    if ctx.termites.has_living(point, Washer::TERMITE_BLOCK_RADIUS) {
        return true;
    }
    ctx.damage.erase(point, Washer::ERASE_RADIUS);
    false
}

impl Tool for Washer {
    fn id(&self) -> ToolId {
        ToolId::Washer
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn mouse_down(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        self.washing = true;
        self.last_point = Some(point);
        self.blocked_by_termites = 0;
        let pan = ctx.pan(point);
        ctx.audio.play("wash_start", pan, 0.6);
        ctx.audio.start_loop("wash_loop", "wash", 0.55, pan);
        if wipe(point, ctx) {
            self.blocked_by_termites += 1;
        }
    }

    fn mouse_dragged(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        if !self.washing {
            return;
        }
        let from = self.last_point;
        self.last_point = Some(point);
        let Some(from) = from else { return };

        // A plain local counter, not `self.blocked_by_termites` directly, so the closure
        // below doesn't need to capture `self` at all — only `ctx` (reborrowed, same
        // reasoning as the chain-saw's cut-along-path).
        let mut blocked_this_stroke = 0;
        along_path(from, point, Self::ERASE_SPACING, |p, _angle| {
            if wipe(p, &mut *ctx) {
                blocked_this_stroke += 1;
            }
        });
        self.blocked_by_termites += blocked_this_stroke;

        let pan = ctx.pan(point);
        ctx.audio.set_loop("wash", None, Some(pan));
    }

    fn mouse_up(&mut self, _point: (f32, f32), ctx: &mut ToolContext) {
        self.washing = false;
        self.last_point = None;
        ctx.audio.stop_loop("wash", 0.20);
    }

    fn deactivate(&mut self, ctx: &mut ToolContext) {
        self.washing = false;
        self.last_point = None;
        ctx.audio.stop_loop("wash", 0.15);
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
    use crate::tools::Hammer;

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
    fn washing_reduces_coverage_left_by_another_tool() {
        let mut env = Env::new(1);
        let mut hammer = Hammer::new();
        hammer.mouse_down((400.0, 300.0), &mut env.ctx());
        let before = env.damage.coverage();
        assert!(before > 0.0);

        let mut washer = Washer::new();
        washer.mouse_down((400.0, 300.0), &mut env.ctx());
        assert!(env.damage.coverage() < before);
    }

    #[test]
    fn a_living_termite_blocks_erasing_underneath_it() {
        let mut env = Env::new(2);
        let mut hammer = Hammer::new();
        hammer.mouse_down((400.0, 300.0), &mut env.ctx());
        env.termites.spawn((400.0, 300.0), &mut env.rng);
        let before = env.damage.coverage();

        let mut washer = Washer::new();
        washer.mouse_down((400.0, 300.0), &mut env.ctx());

        assert_eq!(env.damage.coverage(), before, "erasing under a living termite should be blocked");
        assert!(washer.blocked_by_termites() > 0);
        assert_eq!(env.termites.count(), 1, "the washer must not remove the termite either");
    }

    #[test]
    fn dragging_erases_continuously_along_the_stroke() {
        let mut env = Env::new(3);
        let mut hammer = Hammer::new();
        for x in [200.0, 400.0, 600.0] {
            hammer.mouse_down((x, 300.0), &mut env.ctx());
        }
        let before = env.damage.coverage();

        let mut washer = Washer::new();
        washer.mouse_down((150.0, 300.0), &mut env.ctx());
        washer.mouse_dragged((650.0, 300.0), &mut env.ctx());
        assert!(env.damage.coverage() < before * 0.5, "a long wipe should remove most of what the hammer left");
    }
}
