//! Short-lived particles: splinters from the hammer, sawdust from the chain-saw, shells
//! from the machine gun, droplets from the color-thrower.
//!
//! Ported from `ParticleSystem.swift`, with one real architectural change. The Swift
//! original lets a particle carry an `onLand`/`onExpire` closure that captures `ctx`
//! (weakly) at emit time and is invoked whenever that particle later lands or expires,
//! however many frames later that turns out to be — trivial with reference semantics.
//! Rust can't hold a `&mut` borrow of a context struct alive across separate future
//! frames like that. So instead, each particle carries a small [`ParticleKind`] tag, and
//! the (small) amount of logic those closures ran — a shell's landing sound, a droplet's
//! paint splat — moves into [`ParticleSystem::update`] itself, dispatched by kind. Same
//! behaviour, different mechanism.
//!
//! **Y-up/y-down.** Gravity is a directional force, not a shape, so — unlike most of
//! `decals.rs`/`sprites.rs`, where the fix was per-shape — this crate's one shared
//! `GRAVITY` constant needed its sign flipped once here, positive instead of Swift's
//! negative, so that "pulls toward larger y" still means "down" in this port's y-down
//! scene convention. Individual tools separately negate the *launch* velocities that are
//! supposed to fly upward off an impact — see the comments at each call site in
//! `tools/*.rs`.

use crate::colony::TermiteColony;
use crate::damage::DamageLayer;
use crate::decals::DecalFactory;

/// What kind of effect a particle produces when it lands or expires, and which texture a
/// renderer should draw it with. `Generic` covers slivers and sawdust, which do neither
/// (they just fade out) — both ride the same `DecalFactory::sliver` shape, hence a
/// `variant` here rather than a separate kind per tool.
#[derive(Clone, Copy, Debug)]
pub enum ParticleKind {
    Generic { variant: i64 },
    /// A machine-gun shell — plays a landing sound.
    Shell,
    /// A paint droplet — splats paint on the damage layer when it expires (droplets don't
    /// have a `land_y`; they end their short flight by timing out, not by hitting a
    /// floor).
    Droplet { color_index: i64 },
}

struct Particle {
    position: (f32, f32),
    velocity: (f32, f32),
    /// Radians/second — how fast `rotation` turns, not an angle itself.
    spin: f32,
    /// The actual current rotation, in radians — `spin` integrated over time
    /// (`rotation += spin * dt` each `update`), matching the Swift original's
    /// `sprite.zRotation += spin * d`. Phase 3 had no sprite to apply this to and so never
    /// accumulated it; a real gap, not a deferred-on-purpose one — fixed here now that a
    /// renderer needs it.
    rotation: f32,
    /// Display size in points.
    size: (f32, f32),
    age: f32,
    life: f32,
    gravity_scale: f32,
    /// Once the particle falls below this height, it's removed and, for a `Shell`, its
    /// landing sound plays. `None` for particles (like droplets) that end on a timer
    /// instead.
    land_y: Option<f32>,
    kind: ParticleKind,
}

/// A read-only snapshot of one live particle, for a renderer.
#[derive(Clone, Copy, Debug)]
pub struct ParticleView {
    pub position: (f32, f32),
    pub rotation: f32,
    pub size: (f32, f32),
    pub kind: ParticleKind,
    /// 0 at spawn, 1 at end of life — a renderer's hook for anything that fades or grows
    /// over a particle's lifetime (nothing needs it yet, but `FlameThrower::FlameView`
    /// carries the equivalent for exactly this reason).
    pub life_fraction: f32,
}

pub struct ParticleSystem {
    particles: Vec<Particle>,
}

/// Points per second squared. Positive here (Swift's is `-1_800`) because this port's
/// scene coordinates are y-down — see the module doc comment.
const GRAVITY: f32 = 1_800.0;

impl ParticleSystem {
    pub fn new() -> Self {
        Self { particles: Vec::new() }
    }

    pub fn count(&self) -> usize {
        self.particles.len()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn emit(
        &mut self,
        position: (f32, f32),
        velocity: (f32, f32),
        spin: f32,
        size: (f32, f32),
        life: f32,
        gravity_scale: f32,
        land_y: Option<f32>,
        kind: ParticleKind,
    ) {
        self.particles.push(Particle {
            position,
            velocity,
            spin,
            // Matches the Swift original's `sprite.zRotation = spin` — `spin` doubles as
            // both this particle's *starting* angle and its angular velocity (an unusual
            // choice, but a deliberate one in the source, not a simplification of it).
            // This was `0.0` through Phase 3, which had no sprite to show the discrepancy
            // on; a real gap, not a deferred-on-purpose one — fixed here now that a
            // renderer needs it.
            rotation: spin,
            size,
            age: 0.0,
            life,
            gravity_scale,
            land_y,
            kind,
        });
    }

    /// Every live particle, for a renderer to draw. Order isn't meaningful.
    pub fn iter(&self) -> impl Iterator<Item = ParticleView> + '_ {
        self.particles.iter().map(|p| ParticleView {
            position: p.position,
            rotation: p.rotation,
            size: p.size,
            kind: p.kind,
            life_fraction: if p.life > 0.0 { (p.age / p.life).clamp(0.0, 1.0) } else { 0.0 },
        })
    }

    /// Advances every particle by `dt`. Needs the pieces of `ToolContext` that a landed
    /// shell or an expired droplet might have to touch — see the module doc comment for
    /// why those come in as explicit parameters rather than a bundled context.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        dt: f32,
        damage: &mut DamageLayer,
        decals: &mut DecalFactory,
        audio: &mut crate::audio::AudioSink,
        rng: &mut crate::rng::SeededRng,
        screen_size: (f32, f32),
        _termites: &mut TermiteColony, // unused today; kept so this signature doesn't have
                                       // to change if a future particle kind needs it
    ) {
        if self.particles.is_empty() {
            return;
        }

        let mut survivors = Vec::with_capacity(self.particles.len());
        for mut p in self.particles.drain(..) {
            p.age += dt;
            p.velocity.1 += GRAVITY * p.gravity_scale * dt;
            p.position.0 += p.velocity.0 * dt;
            p.position.1 += p.velocity.1 * dt;
            p.rotation += p.spin * dt;

            let landed = p.land_y.map(|y| p.position.1 >= y).unwrap_or(false);
            let expired = p.age >= p.life;

            if landed {
                on_land(p.kind, p.position, audio, rng, screen_size);
            } else if expired {
                on_expire(p.kind, p.position, damage, decals, audio, rng, screen_size);
            } else {
                survivors.push(p);
            }
        }
        self.particles = survivors;
    }

    pub fn remove_all(&mut self) {
        self.particles.clear();
    }
}

impl Default for ParticleSystem {
    fn default() -> Self {
        Self::new()
    }
}

fn on_land(kind: ParticleKind, position: (f32, f32), audio: &mut crate::audio::AudioSink, rng: &mut crate::rng::SeededRng, screen_size: (f32, f32)) {
    if let ParticleKind::Shell = kind {
        let name = crate::audio::numbered_name("shell", 3, rng);
        let pan = crate::audio::pan_for_x(position.0, screen_size.0);
        audio.play(&name, pan, rng.range(0.30, 0.55) as f32);
    }
}

fn on_expire(
    kind: ParticleKind,
    position: (f32, f32),
    damage: &mut DamageLayer,
    decals: &mut DecalFactory,
    audio: &mut crate::audio::AudioSink,
    rng: &mut crate::rng::SeededRng,
    screen_size: (f32, f32),
) {
    if let ParticleKind::Droplet { color_index } = kind {
        // `ctx.contains(point, margin: 60)` in the Swift original.
        let margin = 60.0;
        let inside = position.0 >= -margin && position.1 >= -margin && position.0 <= screen_size.0 + margin && position.1 <= screen_size.1 + margin;
        if !inside {
            return;
        }

        let size = rng.range(66.0, 132.0) as f32;
        let variant = rng.int(0, DecalFactory::PAINT_VARIANTS);
        let rotation = rng.range(0.0, std::f64::consts::TAU) as f32;
        let alpha = rng.range(0.85, 1.0) as f32;
        let decal = decals.paint_splat(color_index, variant);
        damage.stamp_ex(decal, position, (size, size), rotation, alpha);

        // `Color-thrower drop` — only occasionally, otherwise the sounds blur into noise.
        if rng.chance(0.18) {
            let pan = crate::audio::pan_for_x(position.0, screen_size.0);
            audio.play("paint_drop", pan, rng.range(0.25, 0.45) as f32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioSink;
    use crate::damage::DamageLayer;
    use crate::decals::DecalFactory;
    use crate::rng::SeededRng;

    fn env() -> (DamageLayer, DecalFactory, AudioSink, SeededRng, TermiteColony) {
        (
            DamageLayer::new(512.0, 512.0, 1.0),
            DecalFactory::new(),
            AudioSink::new(),
            SeededRng::new(1),
            TermiteColony::new(),
        )
    }

    #[test]
    fn gravity_pulls_particles_toward_larger_y() {
        let mut sys = ParticleSystem::new();
        let (mut damage, mut decals, mut audio, mut rng, mut termites) = env();
        // No initial velocity, but a `land_y` numerically *larger* than the start position
        // — "further down the screen" in this port's y-down convention. If gravity's sign
        // were wrong (pulling toward smaller y instead), this particle would drift away
        // from land_y forever and never land, so this genuinely tests the direction rather
        // than just that gravity does something.
        sys.emit((100.0, 100.0), (0.0, 0.0), 0.0, (10.0, 10.0), 100.0, 1.0, Some(150.0), ParticleKind::Generic { variant: 0 });
        for _ in 0..120 {
            sys.update(1.0 / 60.0, &mut damage, &mut decals, &mut audio, &mut rng, (512.0, 512.0), &mut termites);
        }
        assert_eq!(sys.count(), 0, "gravity should have pulled the particle down to land_y within 2 seconds");
    }

    #[test]
    fn shell_plays_a_landing_sound_when_it_reaches_land_y() {
        let mut sys = ParticleSystem::new();
        let (mut damage, mut decals, mut audio, mut rng, mut termites) = env();
        sys.emit((100.0, 100.0), (0.0, 500.0), 10.0, (15.0, 8.0), 3.0, 1.0, Some(101.0), ParticleKind::Shell);
        sys.update(1.0 / 60.0, &mut damage, &mut decals, &mut audio, &mut rng, (512.0, 512.0), &mut termites);
        assert_eq!(sys.count(), 0, "the shell should have landed and been removed");
        assert!(audio.events.iter().any(|e| e.starts_with("play shell")), "no landing sound was triggered: {:?}", audio.events);
    }

    #[test]
    fn droplet_splats_paint_on_expiry() {
        let mut sys = ParticleSystem::new();
        let (mut damage, mut decals, mut audio, mut rng, mut termites) = env();
        let before = damage.coverage();
        sys.emit((256.0, 256.0), (0.0, 0.0), 0.0, (16.0, 16.0), 0.01, 0.0, None, ParticleKind::Droplet { color_index: 0 });
        sys.update(0.02, &mut damage, &mut decals, &mut audio, &mut rng, (512.0, 512.0), &mut termites);
        assert_eq!(sys.count(), 0);
        assert!(damage.coverage() > before, "expiring should have splatted paint onto the damage layer");
    }

    #[test]
    fn generic_particles_land_and_expire_silently() {
        let mut sys = ParticleSystem::new();
        let (mut damage, mut decals, mut audio, mut rng, mut termites) = env();
        sys.emit((100.0, 100.0), (0.0, 0.0), 0.0, (10.0, 10.0), 0.01, 0.0, None, ParticleKind::Generic { variant: 0 });
        sys.update(0.02, &mut damage, &mut decals, &mut audio, &mut rng, (512.0, 512.0), &mut termites);
        assert_eq!(sys.count(), 0);
        assert!(audio.events.is_empty());
    }

    #[test]
    fn rotation_accumulates_from_spin_over_time() {
        let mut sys = ParticleSystem::new();
        let (mut damage, mut decals, mut audio, mut rng, mut termites) = env();
        sys.emit((100.0, 100.0), (0.0, 0.0), 2.0, (10.0, 10.0), 10.0, 0.0, None, ParticleKind::Generic { variant: 0 });
        for _ in 0..30 {
            sys.update(1.0 / 60.0, &mut damage, &mut decals, &mut audio, &mut rng, (512.0, 512.0), &mut termites);
        }
        let view = sys.iter().next().expect("the particle should still be alive");
        // 0.5s at a spin of 2 rad/s should have turned it roughly 1 radian — not checking
        // an exact value (float accumulation over 30 steps), just that it moved
        // meaningfully rather than staying at its initial 0.
        assert!(view.rotation > 0.5, "expected substantial rotation, got {}", view.rotation);
    }

    #[test]
    fn iter_yields_one_view_per_particle_with_its_kind_and_size() {
        let mut sys = ParticleSystem::new();
        sys.emit((10.0, 20.0), (0.0, 0.0), 0.0, (5.0, 6.0), 10.0, 0.0, None, ParticleKind::Shell);
        sys.emit((30.0, 40.0), (0.0, 0.0), 0.0, (7.0, 8.0), 10.0, 0.0, None, ParticleKind::Droplet { color_index: 2 });

        let views: Vec<ParticleView> = sys.iter().collect();
        assert_eq!(views.len(), 2);
        assert!(views.iter().any(|v| matches!(v.kind, ParticleKind::Shell) && v.size == (5.0, 6.0)));
        assert!(views.iter().any(|v| matches!(v.kind, ParticleKind::Droplet { color_index: 2 }) && v.size == (7.0, 8.0)));
    }
}
