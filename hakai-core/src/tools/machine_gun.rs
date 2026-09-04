//! Tool 2 — the machine gun.
//!
//! Ported from `MachineGun.swift`. Three audio layers: the shot itself, a periodic
//! reverberation under a burst, and the ejected shell's landing sound (via
//! `ParticleKind::Shell`, `particles.rs`). The muzzle flash and the recoil-kick cursor
//! animation — initially left out (pure presentation, see the `tools` module doc comment)
//! — were recovered once a renderer existed to feed them, the same way `Hammer`'s knock
//! was.
//!
//! **The flash isn't a `ParticleSystem` particle.** `MachineGun.swift`'s `spawnFlash`
//! creates a raw `SKSpriteNode` with its own one-shot scale/fade action, never going
//! through `ParticleSystem.emit` — matching that, `flashes` here is tool-owned, like
//! `FlameThrower`'s standing flames, not `ToolContext`-owned like real particles.

use crate::decals::DecalFactory;
use crate::particles::ParticleKind;

use super::{Tool, ToolContext, ToolId};

/// A brief muzzle flash at the point of impact — position, a fixed rotation (SpriteKit
/// `zRotation`, set once at spawn and never changed) and its own spawn-time size
/// (`MachineGun.swift`'s `side = range(34, 58)`, chosen fresh per flash) are all it needs;
/// a renderer derives its grow/fade from `FlashView::life_fraction`.
struct Flash {
    position: (f32, f32),
    rotation: f32,
    size: f32,
    age: f32,
}

/// A read-only snapshot of one live muzzle flash, for a renderer.
#[derive(Clone, Copy, Debug)]
pub struct FlashView {
    pub position: (f32, f32),
    pub rotation: f32,
    /// This flash's spawn-time size (points, square) — the *starting* size; a renderer
    /// grows it via `life_fraction`, matching `MachineGun.swift`'s `.scale(to: 1.5)`.
    pub size: f32,
    /// 0 at spawn, 1 at end of life — `MachineGun.swift`'s flash scales from 1 to 1.5 and
    /// fades out over its whole (very short) life, both driven by this.
    pub life_fraction: f32,
}

pub struct MachineGun {
    since_shot: f32,
    since_reverb: f32,
    /// Seconds since the last shot — drives `cursor_rotation`'s recoil kick.
    since_kick: f32,
    flashes: Vec<Flash>,
}

impl MachineGun {
    const FIRE_INTERVAL: f32 = 0.085;
    const REVERB_INTERVAL: f32 = 0.55;
    const SHOT_VARIANTS: i64 = 6;
    /// `.rotate(toAngle: -0.075, duration: 0.025, ...)` in `MachineGun.swift`'s `kick`.
    const KICK_OUT_DURATION: f32 = 0.025;
    /// `.rotate(toAngle: 0, duration: 0.055, ...)` — the return leg.
    const KICK_BACK_DURATION: f32 = 0.055;
    const KICK_ANGLE: f32 = -0.075;
    /// `MachineGun.swift`'s `spawnFlash`: `.group([.scale(to: 1.5, duration: 0.09),
    /// .fadeOut(withDuration: 0.09)])`.
    const FLASH_LIFE: f32 = 0.09;

    pub fn new() -> Self {
        Self { since_shot: Self::FIRE_INTERVAL, since_reverb: Self::REVERB_INTERVAL, since_kick: f32::INFINITY, flashes: Vec::new() }
    }

    /// The cursor's current recoil angle: kicks out to `KICK_ANGLE` over
    /// `KICK_OUT_DURATION`, then linearly back to 0 over `KICK_BACK_DURATION` —
    /// `MachineGun.swift`'s `kick()`, recomputed as a pure function of elapsed time
    /// instead of a one-shot `SKAction` sequence, same as `Hammer::cursor_rotation`. Both
    /// `SKAction.rotate` calls there have no `.timingMode` set, i.e. plain linear, so no
    /// easing here either.
    pub fn cursor_rotation(&self) -> f32 {
        let swift_rotation = if self.since_kick < Self::KICK_OUT_DURATION {
            Self::KICK_ANGLE * (self.since_kick / Self::KICK_OUT_DURATION)
        } else if self.since_kick < Self::KICK_OUT_DURATION + Self::KICK_BACK_DURATION {
            let t = (self.since_kick - Self::KICK_OUT_DURATION) / Self::KICK_BACK_DURATION;
            Self::KICK_ANGLE * (1.0 - t)
        } else {
            0.0
        };
        // Same y-up (`zRotation`) → y-down flip as `Hammer::cursor_rotation` — see there.
        -swift_rotation
    }

    /// Every live muzzle flash, for a renderer to draw.
    pub fn flashes(&self) -> impl Iterator<Item = FlashView> + '_ {
        self.flashes.iter().map(|f| FlashView {
            position: f.position,
            rotation: f.rotation,
            size: f.size,
            life_fraction: (f.age / Self::FLASH_LIFE).clamp(0.0, 1.0),
        })
    }

    fn spawn_flash(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        let rotation = ctx.rng.range(0.0, std::f64::consts::TAU) as f32;
        let size = ctx.rng.range(34.0, 58.0) as f32;
        self.flashes.push(Flash { position: point, rotation, size, age: 0.0 });
    }

    fn advance_flashes(&mut self, dt: f32) {
        for f in &mut self.flashes {
            f.age += dt;
        }
        self.flashes.retain(|f| f.age < Self::FLASH_LIFE);
    }

    fn fire(&mut self, point: (f32, f32), ctx: &mut ToolContext) {
        // Spread: a machine gun doesn't hit exactly the same spot twice.
        let spread = ctx.rng.range(0.0, 14.0) as f32;
        let angle = ctx.rng.range(0.0, std::f64::consts::TAU) as f32;
        let hit = (point.0 + angle.cos() * spread, point.1 + angle.sin() * spread);

        let size = ctx.rng.range(56.0, 88.0) as f32;
        let variant = ctx.rng.int(0, DecalFactory::BULLET_HOLE_VARIANTS);
        let rotation = ctx.rng.range(0.0, std::f64::consts::TAU) as f32;
        let decal = ctx.decals.bullet_hole(variant);
        ctx.damage.stamp_ex(decal, hit, (size, size), rotation, 1.0);

        let shot_index = ctx.rng.int(1, MachineGun::SHOT_VARIANTS + 1);
        let pan = ctx.pan(hit);
        let volume = ctx.rng.range(0.8, 1.0) as f32;
        ctx.audio.play(&format!("mg_shot{shot_index}"), pan, volume);

        self.spawn_flash(hit, ctx);
        eject_shell(point, ctx);
        self.since_kick = 0.0;
    }
}

impl Default for MachineGun {
    fn default() -> Self {
        Self::new()
    }
}

/// The shell falls and rings when it lands — handled by `ParticleSystem` via
/// `ParticleKind::Shell`, not here; see `particles.rs`'s `on_land`.
fn eject_shell(point: (f32, f32), ctx: &mut ToolContext) {
    // `max(10, point.y - range(110, 280))` in the Swift original: the shell lands *below*
    // where it was ejected (subtracting moves toward y=0, the bottom, in its y-up scene),
    // clamped so it doesn't fall through the bottom edge. In this port's y-down
    // convention, "below" is *larger* y, and the "don't fall through the edge" clamp is
    // against `screen_size.1`, not 0.
    let land_y = (point.1 + ctx.rng.range(110.0, 280.0) as f32).min(ctx.screen_size.1 - 10.0);
    let start = (point.0 + ctx.rng.range(18.0, 34.0) as f32, point.1 + ctx.rng.range(4.0, 16.0) as f32);
    let vx = ctx.rng.range(80.0, 220.0) as f32;
    // Swift's y range (180..340) is entirely positive — always launches upward in its
    // y-up scene. Negated here for the same reason as the hammer's sliver velocity.
    let vy = -(ctx.rng.range(180.0, 340.0) as f32);
    let spin = ctx.rng.range(9.0, 20.0) as f32;
    ctx.particles.emit(start, (vx, vy), spin, (15.0, 8.0), 3.0, 1.0, Some(land_y), ParticleKind::Shell);
}

impl Tool for MachineGun {
    fn id(&self) -> ToolId {
        ToolId::MachineGun
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn mouse_down(&mut self, _point: (f32, f32), _ctx: &mut ToolContext) {
        self.since_shot = Self::FIRE_INTERVAL;
        self.since_reverb = Self::REVERB_INTERVAL;
    }

    fn update(&mut self, dt: f32, mouse: (f32, f32), is_down: bool, ctx: &mut ToolContext) {
        // Advances (and finishes) even after the trigger's released — same reasoning as
        // `Hammer::since_strike`.
        self.since_kick += dt;
        self.advance_flashes(dt);

        if !is_down {
            return;
        }
        self.since_shot += dt;
        self.since_reverb += dt;

        if self.since_shot >= Self::FIRE_INTERVAL {
            self.since_shot = 0.0;
            self.fire(mouse, ctx);
        }
        if self.since_reverb >= Self::REVERB_INTERVAL {
            self.since_reverb = 0.0;
            let pan = ctx.pan(mouse);
            ctx.audio.play("mg_reverb", pan, 0.5);
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
    fn holding_the_trigger_fires_repeatedly() {
        let mut env = Env::new(1);
        let mut gun = MachineGun::new();
        gun.mouse_down((400.0, 300.0), &mut env.ctx());
        let before = env.damage.coverage();
        for _ in 0..60 {
            // 1 second at 60fps — well over the 0.085s fire interval, several shots
            gun.update(1.0 / 60.0, (400.0, 300.0), true, &mut env.ctx());
        }
        assert!(env.damage.coverage() > before);
        assert!(env.particles.count() > 1, "a burst should eject more than one shell: {}", env.particles.count());
    }

    #[test]
    fn releasing_the_trigger_stops_firing() {
        let mut env = Env::new(2);
        let mut gun = MachineGun::new();
        gun.mouse_down((400.0, 300.0), &mut env.ctx());
        gun.update(1.0 / 60.0, (400.0, 300.0), true, &mut env.ctx());
        let after_one_shot = env.damage.coverage();
        for _ in 0..30 {
            gun.update(1.0 / 60.0, (400.0, 300.0), false, &mut env.ctx());
        }
        assert_eq!(env.damage.coverage(), after_one_shot, "update() with isDown=false must not fire");
    }

    #[test]
    fn a_fresh_gun_has_no_kick_and_no_flashes() {
        let gun = MachineGun::new();
        assert_eq!(gun.cursor_rotation(), 0.0);
        assert_eq!(gun.flashes().count(), 0);
    }

    #[test]
    fn firing_kicks_the_cursor_and_spawns_a_flash() {
        let mut env = Env::new(3);
        let mut gun = MachineGun::new();
        gun.mouse_down((400.0, 300.0), &mut env.ctx());
        gun.update(1.0 / 60.0, (400.0, 300.0), true, &mut env.ctx()); // one shot fires
        assert_eq!(gun.flashes().count(), 1);

        // `since_kick` reads exactly 0 the instant a shot fires (indistinguishable from
        // "at rest," since the kick starts *and* ends at rotation 0) — one more frame,
        // still well inside the kick-out leg, is what actually shows movement.
        gun.update(1.0 / 60.0, (400.0, 300.0), false, &mut env.ctx());
        assert_ne!(gun.cursor_rotation(), 0.0, "the cursor should be mid-kick shortly after a shot");

        // Well past both the kick and the flash's life.
        for _ in 0..30 {
            gun.update(1.0 / 60.0, (400.0, 300.0), false, &mut env.ctx());
        }
        assert_eq!(gun.cursor_rotation(), 0.0, "the kick should have finished");
        assert_eq!(gun.flashes().count(), 0, "the flash should have expired");
    }
}
