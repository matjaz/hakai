//! Headless tool simulation.
//!
//! Ported from `ToolSimulation.swift`. Every tool is driven without a window: synthetic
//! `mouse_down`/`mouse_dragged`/`update(dt)`/`mouse_up`, after which how much of the
//! damage layer actually got covered is measured.
//!
//! Why this matters: without it, the only thing verifiable is that the tools compile.
//! With it, every tool is verified to **actually change the damage layer**, and the
//! interactions between them — the flame kills termites, the stamp squishes them, the
//! washer leaves them — are verified to actually hold.

use crate::colony::TermiteColony;
use crate::damage::DamageLayer;
use crate::decals::DecalFactory;
use crate::particles::ParticleSystem;
use crate::rng::SeededRng;
use crate::tools::{FlameThrower, Hammer, Stamp, Termites, Tool, ToolContext, ToolId, Washer};

const CANVAS: (f32, f32) = (1_280.0, 720.0);
const SCALE: f32 = 2.0;
const STEP: f32 = 1.0 / 60.0;

pub struct SimResult {
    pub tool: ToolId,
    pub coverage: f32,
    pub particles: usize,
    pub termites: usize,
    pub note: String,
}

pub struct InteractionCheck {
    pub rule: &'static str,
    pub passed: bool,
    pub detail: String,
}

/// Everything needed for one run.
pub struct Rig {
    pub damage: DamageLayer,
    pub decals: DecalFactory,
    pub particles: ParticleSystem,
    pub termites: TermiteColony,
    pub audio: crate::audio::AudioSink,
    pub rng: SeededRng,
    pub screen_size: (f32, f32),
}

impl Rig {
    pub fn new(seed: u64) -> Self {
        Self {
            damage: DamageLayer::new(CANVAS.0, CANVAS.1, SCALE),
            decals: DecalFactory::new(),
            particles: ParticleSystem::new(),
            termites: TermiteColony::new(),
            audio: crate::audio::AudioSink::new(),
            rng: SeededRng::new(seed),
            screen_size: CANVAS,
        }
    }

    pub fn ctx(&mut self) -> ToolContext<'_> {
        ToolContext {
            damage: &mut self.damage,
            decals: &mut self.decals,
            particles: &mut self.particles,
            termites: &mut self.termites,
            audio: &mut self.audio,
            screen_size: self.screen_size,
            rng: &mut self.rng,
            brightness: Some(128),
        }
    }

    fn advance(&mut self, dt: f32) {
        self.particles.update(dt, &mut self.damage, &mut self.decals, &mut self.audio, &mut self.rng, self.screen_size, &mut self.termites);
        self.termites.update(dt, &mut self.damage, &mut self.decals, &mut self.audio, &mut self.rng, self.screen_size);
    }
}

/// A stroke across the middle of the canvas.
fn drive(tool: &mut dyn Tool, rig: &mut Rig, from: (f32, f32), to: (f32, f32), drag_steps: i32, settle_steps: i32) {
    tool.mouse_down(from, &mut rig.ctx());

    let mut mouse = from;
    for i in 1..=drag_steps {
        let t = i as f32 / drag_steps as f32;
        mouse = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        tool.mouse_dragged(mouse, &mut rig.ctx());
        tool.update(STEP, mouse, true, &mut rig.ctx());
        rig.advance(STEP);
    }

    tool.mouse_up(mouse, &mut rig.ctx());

    // Settling: flames burn out, shells land, droplets splat.
    for _ in 0..settle_steps {
        tool.update(STEP, mouse, false, &mut rig.ctx());
        rig.advance(STEP);
    }
}

// MARK: - Run

pub fn run() -> Vec<SimResult> {
    let mut results = Vec::with_capacity(9);
    let a = (220.0, 250.0);
    let b = (1_060.0, 470.0);

    for id in ToolId::ALL {
        let mut rig = Rig::new(0x51_0000u64.wrapping_add(id as u64));

        match id {
            ToolId::Washer => {
                // The washer needs something to erase, so the hammer covers the canvas
                // first; and one termite, so the erase block can be verified.
                let mut hammer = Hammer::new();
                for i in 0..=20 {
                    let t = i as f32 / 20.0;
                    let p = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                    hammer.mouse_down(p, &mut rig.ctx());
                }
                let before = rig.damage.coverage();
                rig.termites.spawn((640.0, 360.0), &mut rig.rng);

                let mut washer = Washer::new();
                drive(&mut washer, &mut rig, a, b, 48, 20);
                let after = rig.damage.coverage();
                let note = format!(
                    "coverage {:.1}% → {:.1}%, blocked by termites: {}, termites surviving: {}",
                    before * 100.0,
                    after * 100.0,
                    washer.blocked_by_termites(),
                    rig.termites.count()
                );
                results.push(SimResult { tool: id, coverage: after, particles: rig.particles.count(), termites: rig.termites.count(), note });
            }
            ToolId::FlameThrower => {
                let mut flame = FlameThrower::new();
                drive(&mut flame, &mut rig, a, b, 48, 200);
                let note = format!("flames left at the end: {}", flame.active_flame_count());
                results.push(SimResult { tool: id, coverage: rig.damage.coverage(), particles: rig.particles.count(), termites: rig.termites.count(), note });
            }
            _ => {
                let mut tool = id.make_tool();
                drive(tool.as_mut(), &mut rig, a, b, 48, 200);
                results.push(SimResult { tool: id, coverage: rig.damage.coverage(), particles: rig.particles.count(), termites: rig.termites.count(), note: String::new() });
            }
        }
    }

    results
}

// MARK: - Interactions between tools

/// Verifies the rules of the original that can't be seen in any single image.
pub fn check_interactions() -> Vec<InteractionCheck> {
    let mut out = Vec::with_capacity(5);

    // 1. Termites survive a tool change (`termites will not be switched off`).
    {
        let mut rig = Rig::new(1);
        let mut termites = Termites::new();
        termites.mouse_down((400.0, 360.0), &mut rig.ctx());
        let after = rig.termites.count();
        termites.deactivate(&mut rig.ctx());
        let mut hammer = Hammer::new();
        hammer.mouse_down((800.0, 360.0), &mut rig.ctx());
        out.push(InteractionCheck {
            rule: "termites survive a tool change",
            passed: after > 0 && rig.termites.count() == after,
            detail: format!("{after} dropped → {} after switching", rig.termites.count()),
        });
    }

    // 2. The flame-thrower kills termites (`death of termites`).
    {
        let mut rig = Rig::new(2);
        for _ in 0..12 {
            rig.termites.spawn((640.0, 360.0), &mut rig.rng);
        }
        let before = rig.termites.count();
        let mut flame = FlameThrower::new();
        flame.mouse_down((640.0, 360.0), &mut rig.ctx());
        for _ in 0..30 {
            flame.update(STEP, (640.0, 360.0), true, &mut rig.ctx());
        }
        out.push(InteractionCheck { rule: "the flame-thrower kills termites", passed: rig.termites.count() < before, detail: format!("{before} → {}", rig.termites.count()) });
    }

    // 3. The stamp squishes termites (`squishy termite`).
    {
        let mut rig = Rig::new(3);
        for _ in 0..12 {
            rig.termites.spawn((640.0, 360.0), &mut rig.rng);
        }
        let before = rig.termites.count();
        let mut stamp = Stamp::new();
        stamp.mouse_down((640.0, 360.0), &mut rig.ctx());
        out.push(InteractionCheck { rule: "the stamp squishes termites", passed: rig.termites.count() < before, detail: format!("{before} → {}", rig.termites.count()) });
    }

    // 4. The washer does NOT remove termites (`there was found termite`).
    {
        let mut rig = Rig::new(4);
        for _ in 0..12 {
            rig.termites.spawn((640.0, 360.0), &mut rig.rng);
        }
        let before = rig.termites.count();
        let mut washer = Washer::new();
        washer.mouse_down((640.0, 360.0), &mut rig.ctx());
        for i in 0..40 {
            washer.mouse_dragged((600.0 + i as f32 * 2.0, 360.0), &mut rig.ctx());
        }
        out.push(InteractionCheck {
            rule: "the washer does not remove termites",
            passed: rig.termites.count() == before && washer.blocked_by_termites() > 0,
            detail: format!("{before} → {}, blocked: {}", rig.termites.count(), washer.blocked_by_termites()),
        });
    }

    // 5. Surface brightness selects the impact variant.
    {
        let mut rng = SeededRng::new(5);
        let dark = crate::audio::smash_name(Some(10), &mut rng);
        let bright = crate::audio::smash_name(Some(245), &mut rng);
        out.push(InteractionCheck { rule: "surface brightness selects the impact variant", passed: dark == "smash1" && bright == "smash8", detail: format!("dark → {dark}, light → {bright}") });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_changes_the_damage_layer_except_the_washer() {
        for r in run() {
            if r.tool == ToolId::Washer {
                continue; // the washer erases — see its own note about before/after coverage
            }
            assert!(r.coverage > 0.001, "{} ({:?}) did not change the damage layer", r.tool.display_name(), r.tool);
        }
    }

    #[test]
    fn the_washers_result_reflects_its_special_setup() {
        let results = run();
        let washer = results.iter().find(|r| r.tool == ToolId::Washer).expect("washer result");
        // The rig spawns exactly one termite before driving the washer — it should have
        // survived (the washer never kills termites; that's rule #4 in
        // `check_interactions`), and its blood/blocking wouldn't show up here if the
        // special-cased setup in `run()` hadn't actually executed.
        assert_eq!(washer.termites, 1, "the seeded termite should have survived the wash");
        assert!(!washer.note.is_empty());
    }

    #[test]
    fn all_five_interaction_rules_pass() {
        for check in check_interactions() {
            assert!(check.passed, "rule failed: {} — {}", check.rule, check.detail);
        }
    }
}
