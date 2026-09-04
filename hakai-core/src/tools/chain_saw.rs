//! Tool 1 — the chain-saw.
//!
//! Ported from `ChainSaw.swift`. Two looping sounds (idle, cutting) and a continuous cut
//! along the drag path, not just under the click. The cursor shake/texture-swap animation
//! — initially left out, see the `tools` module doc comment — was recovered once a
//! renderer existed to feed it, the same way `Hammer::cursor_rotation` was.
//!
//! **Revving is driven by movement, not by the mouse button.** `ChainSaw.swift`'s
//! `advanceRevving` tracks how long it's been since the mouse last moved more than
//! `MOVEMENT_THRESHOLD` in a frame, and `isRevving` — used for *both* the idle/cutting
//! sound crossfade and the cursor's shake amplitude — is just "was that recently enough."
//! It doesn't check whether the button is held at all: waving the saw around while
//! selected, not clicking, still revs it. So `update()` below always runs this
//! (unconditionally, not gated on `is_down`), matching the source rather than the
//! simpler (and wrong) "loud while held" rule an earlier pass here had.

use crate::decals::DecalFactory;
use crate::particles::ParticleKind;

use super::{along_path, Tool, ToolContext, ToolId};

pub struct ChainSaw {
    last_point: Option<(f32, f32)>,
    cutting: bool,
    /// The mouse position as of the last `update()` call — for detecting movement.
    last_mouse: Option<(f32, f32)>,
    /// Seconds since the mouse last moved more than `MOVEMENT_THRESHOLD` in one frame.
    /// Starts at infinity (matching Swift's `.infinity`) so a freshly selected saw idles
    /// rather than revving before it's ever seen any movement at all.
    since_movement: f32,
    /// A phase accumulator for the cursor's vibration — `ChainSaw.swift`'s `shakePhase`.
    shake_phase: f32,
}

impl ChainSaw {
    /// Smaller than the decal's width so segments overlap into a continuous line.
    const CUT_SPACING: f32 = 15.0;
    const IDLE_VOLUME: f32 = 0.28;
    const CUT_VOLUME: f32 = 0.80;
    /// How long the saw keeps revving after the mouse stops moving.
    const REV_HOLD_AFTER_MOVEMENT: f32 = 1.0;
    /// Movement below this in a frame is jitter, not a stroke.
    const MOVEMENT_THRESHOLD: f32 = 0.5;

    pub fn new() -> Self {
        Self { last_point: None, cutting: false, last_mouse: None, since_movement: f32::INFINITY, shake_phase: 0.0 }
    }

    /// Whether the saw is revving rather than idling — exposed (like `Hammer`'s
    /// `cursor_rotation`) so the check suite, and eventually a renderer, can read it
    /// without a running audio engine or scene.
    pub fn is_revving(&self) -> bool {
        self.since_movement < Self::REV_HOLD_AFTER_MOVEMENT
    }

    fn advance_revving(&mut self, dt: f32, mouse: (f32, f32)) {
        if let Some(last) = self.last_mouse {
            let (dx, dy) = (mouse.0 - last.0, mouse.1 - last.1);
            if dx.hypot(dy) > Self::MOVEMENT_THRESHOLD {
                self.since_movement = 0.0;
            } else {
                self.since_movement += dt;
            }
        }
        self.last_mouse = Some(mouse);
    }

    /// The cursor's current vibration angle — `ChainSaw.swift`'s
    /// `sin(shakePhase) * (isRevving ? 0.030 : 0.010)`, `shakePhase` advanced in `update`.
    pub fn cursor_rotation(&self) -> f32 {
        let amplitude = if self.is_revving() { 0.030 } else { 0.010 };
        let swift_rotation = self.shake_phase.sin() * amplitude;
        // Same y-up (`zRotation`) → y-down flip as `Hammer::cursor_rotation` — see there
        // for why: this is a raw SpriteKit angle, and this port's cursor rotation runs in
        // y-down pixel space.
        -swift_rotation
    }
}

impl Default for ChainSaw {
    fn default() -> Self {
        Self::new()
    }
}

fn cut(point: (f32, f32), angle: f32, ctx: &mut ToolContext) {
    let variant = ctx.rng.int(0, DecalFactory::SAW_CUT_VARIANTS);
    let length = ctx.rng.range(70.0, 100.0) as f32;
    let alpha = ctx.rng.range(0.8, 1.0) as f32;
    let decal = ctx.decals.saw_cut(variant);
    ctx.damage.stamp_ex(decal, point, (length, length * 0.375), angle, alpha);

    // Sawdust from the cut.
    if ctx.rng.chance(0.35) {
        let variant = ctx.rng.int(0, DecalFactory::SLIVER_VARIANTS);
        let spread = angle + std::f32::consts::FRAC_PI_2 + ctx.rng.jitter(0.8) as f32;
        let speed = ctx.rng.range(90.0, 240.0) as f32;
        let s = ctx.rng.range(5.0, 10.0) as f32;
        let spin = ctx.rng.jitter(11.0) as f32;
        let life = ctx.rng.range(0.35, 0.75) as f32;
        // `y: abs(sin(spread))*speed + 90` in Swift always launches upward (positive y,
        // y-up) — negated here for this port's y-down convention, same as the hammer.
        let velocity = (spread.cos() * speed, -(spread.sin().abs() * speed + 90.0));
        ctx.particles.emit(point, velocity, spin, (s, s), life, 1.0, None, ParticleKind::Generic { variant });
    }
}

impl Tool for ChainSaw {
    fn id(&self) -> ToolId {
        ToolId::ChainSaw
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn mouse_down(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        self.cutting = true;
        self.last_point = Some(point);
        let angle = ctx.rng.range(0.0, std::f64::consts::TAU) as f32;
        cut(point, angle, ctx);
    }

    fn mouse_dragged(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        if !self.cutting {
            return;
        }
        let from = self.last_point;
        self.last_point = Some(point);
        let Some(from) = from else { return };

        // `&mut *ctx` (an explicit reborrow), not `ctx`: this closure runs once per step
        // along the path, and passing `ctx` itself would move it out of the closure's
        // captured state on the first call, which the compiler needs to be told not to
        // do.
        along_path(from, point, Self::CUT_SPACING, |p, angle| cut(p, angle, &mut *ctx));
    }

    fn mouse_up(&mut self, _point: (f32, f32), _ctx: &mut ToolContext) {
        self.cutting = false;
        self.last_point = None;
    }

    fn update(&mut self, dt: f32, mouse: (f32, f32), _is_down: bool, ctx: &mut ToolContext) {
        // Unconditional — not gated on `is_down`. See the module doc comment: revving
        // (and so both loops' crossfade, and the cursor's shake amplitude) follows mouse
        // *movement*, not the button.
        self.advance_revving(dt, mouse);
        let revving = self.is_revving();

        // Both loops run for as long as the tool is selected and are crossfaded by gain;
        // `start_loop` only updates the target once a loop already exists, so calling it
        // every frame is safe and the ~100ms glide (once Phase 5 wires up real audio)
        // does the blend in both directions.
        let pan = ctx.pan(mouse);
        ctx.audio.start_loop("saw_idle", "saw_idle", if revving { 0.0 } else { Self::IDLE_VOLUME }, pan);
        ctx.audio.start_loop("saw_cut", "saw_cut", if revving { Self::CUT_VOLUME } else { 0.0 }, pan);

        // Vibration follows the sound: a revving saw shakes, an idling one barely.
        self.shake_phase += dt * if revving { 46.0 } else { 17.0 };
    }

    fn deactivate(&mut self, ctx: &mut ToolContext) {
        self.cutting = false;
        self.last_point = None;
        self.last_mouse = None;
        self.since_movement = f32::INFINITY;
        ctx.audio.stop_loop("saw_idle", 0.18);
        ctx.audio.stop_loop("saw_cut", 0.10);
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
    fn mouse_down_makes_one_cut() {
        let mut env = Env::new(1);
        let mut saw = ChainSaw::new();
        let before = env.damage.coverage();
        saw.mouse_down((400.0, 300.0), &mut env.ctx());
        assert!(env.damage.coverage() > before);
    }

    #[test]
    fn dragging_cuts_continuously_along_the_stroke() {
        let mut env = Env::new(2);
        let mut saw = ChainSaw::new();
        saw.mouse_down((100.0, 300.0), &mut env.ctx());
        let after_down = env.damage.coverage();
        saw.mouse_dragged((700.0, 300.0), &mut env.ctx()); // a long drag, many cut segments
        assert!(env.damage.coverage() > after_down * 2.0, "a long drag should cut much more than a single click");
    }

    #[test]
    fn dragging_without_mouse_down_does_nothing() {
        let mut env = Env::new(3);
        let mut saw = ChainSaw::new();
        let before = env.damage.coverage();
        saw.mouse_dragged((700.0, 300.0), &mut env.ctx());
        assert_eq!(env.damage.coverage(), before);
    }

    #[test]
    fn mouse_up_stops_cutting_state() {
        let mut env = Env::new(4);
        let mut saw = ChainSaw::new();
        saw.mouse_down((100.0, 300.0), &mut env.ctx());
        saw.mouse_up((100.0, 300.0), &mut env.ctx());
        let before = env.damage.coverage();
        saw.mouse_dragged((700.0, 300.0), &mut env.ctx());
        assert_eq!(env.damage.coverage(), before, "dragging after mouse_up shouldn't cut");
    }

    #[test]
    fn a_fresh_saw_is_not_revving() {
        let saw = ChainSaw::new();
        assert!(!saw.is_revving(), "a saw that has never seen movement shouldn't be revving");
    }

    #[test]
    fn movement_starts_revving_without_needing_the_button_held() {
        let mut env = Env::new(5);
        let mut saw = ChainSaw::new();
        saw.update(1.0 / 60.0, (100.0, 100.0), false, &mut env.ctx()); // establishes last_mouse
        saw.update(1.0 / 60.0, (200.0, 100.0), false, &mut env.ctx()); // a real jump, well over the threshold
        assert!(saw.is_revving(), "moving the mouse should start revving even with the button up");
    }

    #[test]
    fn revving_stops_after_the_hold_period_once_movement_stops() {
        let mut env = Env::new(6);
        let mut saw = ChainSaw::new();
        saw.update(1.0 / 60.0, (100.0, 100.0), false, &mut env.ctx());
        saw.update(1.0 / 60.0, (200.0, 100.0), false, &mut env.ctx());
        assert!(saw.is_revving());
        // Sit still for well over `REV_HOLD_AFTER_MOVEMENT`.
        for _ in 0..90 {
            saw.update(1.0 / 60.0, (200.0, 100.0), false, &mut env.ctx());
        }
        assert!(!saw.is_revving(), "revving should stop once movement has stopped for long enough");
    }

    #[test]
    fn the_cursor_shakes_more_while_revving() {
        let mut env = Env::new(7);
        let mut saw = ChainSaw::new();
        saw.update(1.0 / 60.0, (100.0, 100.0), false, &mut env.ctx());
        saw.update(1.0 / 60.0, (400.0, 100.0), false, &mut env.ctx());
        assert!(saw.is_revving());
        let mut revving_peak: f32 = 0.0;
        for _ in 0..40 {
            saw.update(1.0 / 60.0, (400.0, 100.0), false, &mut env.ctx());
            revving_peak = revving_peak.max(saw.cursor_rotation().abs());
        }
        assert!(revving_peak > 0.02, "the shake amplitude while revving should reach close to its 0.030 rad peak, got {revving_peak}");
    }
}
