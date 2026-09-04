//! The termite colony.
//!
//! Ported from `TermiteColony.swift`. It lives outside the tools because it has to
//! survive a tool change — the original has an explicit note for this: `termites will
//! not be switched off`. Four tools reach into it: the termites drop them, the
//! flame-thrower and the stamp kill them, and the washer **leaves them alive** (`there
//! was found termite`).
//!
//! Like `ParticleSystem`, methods here take the pieces of `ToolContext` they need as
//! explicit parameters rather than a bundled context — see the note in `particles.rs` for
//! why: `TermiteColony` is itself a field *of* `ToolContext`, and Rust can't borrow a
//! struct's field mutably while also borrowing the whole struct.
//!
//! **Y-up/y-down.** The reflection off a screen edge (`heading = -heading`) is a vector
//! identity independent of which way y increases, so it needed no change — the only
//! genuinely convention-sensitive thing here (gravity, launch directions) lives in
//! `particles.rs`, not this file.

use crate::audio::AudioSink;
use crate::damage::DamageLayer;
use crate::decals::DecalFactory;
use crate::rng::SeededRng;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cause {
    /// The flame-thrower — `death of termites` → `dead termite`.
    Flame,
    /// The stamp — `squishy termite`.
    Squish,
}

struct Termite {
    position: (f32, f32),
    heading: f32,
    speed: f32,
    scale: f32,
    chew_in: f32,
    frame_in: f32,
    frame: i64,
}

/// A read-only snapshot of one termite, for a renderer — everything it needs to place and
/// pick a sprite frame, and nothing about the physics/timers driving it.
#[derive(Clone, Copy, Debug)]
pub struct TermiteView {
    pub position: (f32, f32),
    /// Radians — also the sprite's rotation, matching how the original orients the
    /// texture along the direction of travel.
    pub heading: f32,
    pub scale: f32,
    /// 0 or 1 — `SpriteFactory::termite`'s frame index.
    pub frame: i64,
}

pub struct TermiteColony {
    termites: Vec<Termite>,
    chew_sound_in: f32,
    death_sound_in: f32,
}

/// The upper bound. The original allows a lot of termites, but there has to be a ceiling
/// somewhere so the frame rate doesn't collapse.
pub const MAX_COUNT: usize = 500;

impl TermiteColony {
    pub fn new() -> Self {
        Self { termites: Vec::new(), chew_sound_in: 0.0, death_sound_in: 0.0 }
    }

    pub fn count(&self) -> usize {
        self.termites.len()
    }

    /// Every living termite, for a renderer to draw. Order is whatever `Vec` order they
    /// happen to be stored in — not meaningful, not stable across a kill (which uses
    /// swap-free removal via rebuilding a survivors list).
    pub fn iter(&self) -> impl Iterator<Item = TermiteView> + '_ {
        self.termites.iter().map(|t| TermiteView { position: t.position, heading: t.heading, scale: t.scale, frame: t.frame })
    }
    pub fn is_empty(&self) -> bool {
        self.termites.is_empty()
    }

    // MARK: - Dropping

    pub fn spawn(&mut self, point: (f32, f32), rng: &mut SeededRng) {
        if self.termites.len() >= MAX_COUNT {
            return;
        }
        let heading = rng.range(0.0, std::f64::consts::TAU) as f32;
        let scale = rng.range(0.85, 1.25) as f32;
        let speed = rng.range(26.0, 58.0) as f32;
        self.termites.push(Termite {
            position: point,
            heading,
            speed,
            scale,
            chew_in: 0.0,
            frame_in: 0.0,
            frame: 0,
        });
    }

    // MARK: - Update

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        dt: f32,
        damage: &mut DamageLayer,
        decals: &mut DecalFactory,
        audio: &mut AudioSink,
        rng: &mut SeededRng,
        screen_size: (f32, f32),
    ) {
        self.chew_sound_in -= dt;
        self.death_sound_in -= dt;
        if self.termites.is_empty() {
            return;
        }

        let mut chewed_this_frame = false;
        let mut chew_anchor: Option<(f32, f32)> = None;

        for t in &mut self.termites {
            // Random walk: the heading meanders slowly rather than jumping around.
            t.heading += rng.jitter(2.2) as f32 * dt;
            let mut p = (t.position.0 + t.heading.cos() * t.speed * dt, t.position.1 + t.heading.sin() * t.speed * dt);

            // Bounce off the screen edges. The reflection formulas are the same
            // regardless of which way y increases — see the module doc comment.
            if p.0 < 6.0 || p.0 > screen_size.0 - 6.0 {
                t.heading = std::f32::consts::PI - t.heading;
                p.0 = p.0.clamp(6.0, screen_size.0 - 6.0);
            }
            if p.1 < 6.0 || p.1 > screen_size.1 - 6.0 {
                t.heading = -t.heading;
                p.1 = p.1.clamp(6.0, screen_size.1 - 6.0);
            }
            t.position = p;

            // Walking animation (frame index only — nothing in Phase 3 renders it).
            t.frame_in -= dt;
            if t.frame_in <= 0.0 {
                t.frame_in = 0.09;
                t.frame = (t.frame + 1) % 2;
            }

            // Chewing: a trail through the damage layer. Lands at the termite's *head*,
            // not its geometric centre — `SpriteFactory::termite`'s head ellipse sits at
            // 16% from the sprite's left edge, not 50%, and the renderer rotates the
            // sprite to face `heading` (its local "forward" being -x, since the head art
            // is on the sprite's left) — so in world space the head sits `head_dist`
            // ahead of centre, along `heading` itself. `34.0` is the sprite's display
            // width before the termite's own per-spawn `t.scale`.
            t.chew_in -= dt;
            if t.chew_in <= 0.0 {
                t.chew_in = rng.range(0.06, 0.14) as f32;
                let size = rng.range(11.0, 18.0) as f32;
                let variant = rng.int(0, 4);
                let alpha = rng.range(0.6, 0.95) as f32;
                let decal = decals.bite(variant);
                let head_dist = (0.5 - 0.16) * 34.0 * t.scale;
                let bite_at = (p.0 + head_dist * t.heading.cos(), p.1 + head_dist * t.heading.sin());
                damage.stamp_ex(decal, bite_at, (size, size), t.heading, alpha);
                chewed_this_frame = true;
                chew_anchor = Some(p);
            }
        }

        if chewed_this_frame && self.chew_sound_in <= 0.0 {
            self.chew_sound_in = 0.16;
            if let Some(anchor) = chew_anchor {
                let name = if rng.chance(0.6) { "termite_chew" } else { "termite_crunch" };
                let pan = crate::audio::pan_for_x(anchor.0, screen_size.0);
                audio.play(name, pan, 0.42);
            }
        }
    }

    // MARK: - Death

    /// Kills the termites inside a circle. Returns how many died.
    #[allow(clippy::too_many_arguments)]
    pub fn kill(
        &mut self,
        point: (f32, f32),
        radius: f32,
        cause: Cause,
        damage: &mut DamageLayer,
        decals: &mut DecalFactory,
        audio: &mut AudioSink,
        rng: &mut SeededRng,
        screen_size: (f32, f32),
    ) -> usize {
        if self.termites.is_empty() {
            return 0;
        }

        let mut survivors = Vec::with_capacity(self.termites.len());
        let mut killed = 0usize;

        for t in self.termites.drain(..) {
            let (dx, dy) = (t.position.0 - point.0, t.position.1 - point.1);
            let dist = dx.hypot(dy);
            if dist <= radius {
                killed += 1;
                // A trail of blood — `death of termites`.
                let size = rng.range(26.0, 46.0) as f32;
                let variant = rng.int(0, DecalFactory::BLOOD_VARIANTS);
                let rotation = rng.range(0.0, std::f64::consts::TAU) as f32;
                let alpha = rng.range(0.75, 1.0) as f32;
                let decal = decals.blood(variant);
                damage.stamp_ex(decal, t.position, (size, size), rotation, alpha);
            } else {
                survivors.push(t);
            }
        }
        self.termites = survivors;

        if killed > 0 && self.death_sound_in <= 0.0 {
            self.death_sound_in = 0.10;
            let name = if cause == Cause::Squish { "termite_squish" } else { "termite_dead" };
            let pan = crate::audio::pan_for_x(point.0, screen_size.0);
            audio.play(name, pan, 0.75);
        }
        killed
    }

    /// Whether there's still a living termite inside the circle. The washer checks this
    /// before erasing: in the original the water jet removes the traces but not the
    /// living ants (`there was found termite`).
    pub fn has_living(&self, point: (f32, f32), radius: f32) -> bool {
        self.termites.iter().any(|t| {
            let dx = t.position.0 - point.0;
            let dy = t.position.1 - point.1;
            (dx * dx + dy * dy).sqrt() <= radius
        })
    }

    pub fn remove_all(&mut self) {
        self.termites.clear();
    }
}

impl Default for TermiteColony {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decals::DecalFactory;

    fn env() -> (DamageLayer, DecalFactory, AudioSink, SeededRng) {
        (DamageLayer::new(1024.0, 1024.0, 1.0), DecalFactory::new(), AudioSink::new(), SeededRng::new(1))
    }

    #[test]
    fn spawn_increases_count_up_to_the_cap() {
        let mut colony = TermiteColony::new();
        let mut rng = SeededRng::new(1);
        for _ in 0..MAX_COUNT + 10 {
            colony.spawn((100.0, 100.0), &mut rng);
        }
        assert_eq!(colony.count(), MAX_COUNT);
    }

    #[test]
    fn kill_near_a_point_removes_only_nearby_termites() {
        let mut colony = TermiteColony::new();
        let mut rng = SeededRng::new(2);
        colony.spawn((100.0, 100.0), &mut rng);
        colony.spawn((900.0, 900.0), &mut rng);
        let (mut damage, mut decals, mut audio, mut rng) = env();
        let killed = colony.kill((100.0, 100.0), 50.0, Cause::Flame, &mut damage, &mut decals, &mut audio, &mut rng, (1024.0, 1024.0));
        assert_eq!(killed, 1);
        assert_eq!(colony.count(), 1);
        assert!(!colony.has_living((100.0, 100.0), 50.0));
        assert!(colony.has_living((900.0, 900.0), 50.0));
    }

    #[test]
    fn kill_leaves_a_blood_trail() {
        let mut colony = TermiteColony::new();
        let mut rng = SeededRng::new(3);
        colony.spawn((512.0, 512.0), &mut rng);
        let (mut damage, mut decals, mut audio, mut rng) = env();
        let before = damage.coverage();
        colony.kill((512.0, 512.0), 50.0, Cause::Squish, &mut damage, &mut decals, &mut audio, &mut rng, (1024.0, 1024.0));
        assert!(damage.coverage() > before);
    }

    #[test]
    fn update_moves_termites_and_chews() {
        let mut colony = TermiteColony::new();
        let mut spawn_rng = SeededRng::new(4);
        colony.spawn((512.0, 512.0), &mut spawn_rng);
        let (mut damage, mut decals, mut audio, mut rng) = env();
        let before = damage.coverage();
        for _ in 0..30 {
            colony.update(1.0 / 60.0, &mut damage, &mut decals, &mut audio, &mut rng, (1024.0, 1024.0));
        }
        assert!(damage.coverage() > before, "half a second of walking should have chewed at least one bite");
    }

    #[test]
    fn remove_all_empties_the_colony() {
        let mut colony = TermiteColony::new();
        let mut rng = SeededRng::new(5);
        for _ in 0..5 {
            colony.spawn((10.0, 10.0), &mut rng);
        }
        colony.remove_all();
        assert!(colony.is_empty());
    }

    #[test]
    fn iter_yields_one_view_per_termite_at_its_current_position() {
        let mut colony = TermiteColony::new();
        let mut rng = SeededRng::new(6);
        colony.spawn((100.0, 200.0), &mut rng);
        colony.spawn((300.0, 400.0), &mut rng);

        let views: Vec<TermiteView> = colony.iter().collect();
        assert_eq!(views.len(), 2);
        let positions: std::collections::HashSet<(i32, i32)> = views.iter().map(|v| (v.position.0 as i32, v.position.1 as i32)).collect();
        assert!(positions.contains(&(100, 200)));
        assert!(positions.contains(&(300, 400)));
    }
}
