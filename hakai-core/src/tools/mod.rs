//! The nine tools, and everything they share.
//!
//! Ported from `Tool.swift`. The indices `0: hammer` … `8: washer` are part of the
//! original's data model and determine keys 1–9 in the real app; the check suite (once
//! ported) verifies them.
//!
//! **No cursor here, deliberately.** The Swift `Tool` protocol has a `makeCursor` method
//! and each tool drives small `SKAction` animations (a hammer's knock, a chain-saw's
//! shake, a machine gun's recoil kick). None of that is game logic — nothing in
//! `ToolSimulation.swift`'s checks ever inspects cursor state, only the damage layer,
//! particle count and termite count — so it's pure presentation, deferred to Phase 4
//! (the renderer) rather than ported here. Everything that *is* logic (what gets stamped,
//! what gets emitted, what dies) is ported in full.

use crate::audio::AudioSink;
use crate::damage::DamageLayer;
use crate::decals::DecalFactory;
use crate::particles::ParticleSystem;
use crate::colony::TermiteColony;
use crate::rng::SeededRng;

pub mod chain_saw;
pub mod color_thrower;
pub mod flame_thrower;
pub mod hammer;
pub mod machine_gun;
pub mod phaser;
pub mod stamp;
pub mod termites;
pub mod washer;

pub use chain_saw::ChainSaw;
pub use color_thrower::ColorThrower;
pub use flame_thrower::FlameThrower;
pub use hammer::Hammer;
pub use machine_gun::MachineGun;
pub use phaser::Phaser;
pub use stamp::Stamp;
pub use termites::Termites;
pub use washer::Washer;

/// The indices match the original (`0: hammer` … `8: washing` in its symbol table), so
/// they also line up with keys 1–9.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolId {
    Hammer = 0,
    ChainSaw = 1,
    MachineGun = 2,
    FlameThrower = 3,
    ColorThrower = 4,
    Phaser = 5,
    Stamp = 6,
    Termites = 7,
    Washer = 8,
}

impl ToolId {
    pub const ALL: [ToolId; 9] = [
        ToolId::Hammer,
        ToolId::ChainSaw,
        ToolId::MachineGun,
        ToolId::FlameThrower,
        ToolId::ColorThrower,
        ToolId::Phaser,
        ToolId::Stamp,
        ToolId::Termites,
        ToolId::Washer,
    ];

    pub fn key_digit(self) -> i32 {
        self as i32 + 1
    }

    pub fn from_key_digit(digit: i32) -> Option<ToolId> {
        ToolId::ALL.into_iter().find(|t| t.key_digit() == digit)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ToolId::Hammer => "Hammer",
            ToolId::ChainSaw => "Chain-saw",
            ToolId::MachineGun => "Machine gun",
            ToolId::FlameThrower => "Flame-thrower",
            ToolId::ColorThrower => "Color-thrower",
            ToolId::Phaser => "Phaser",
            ToolId::Stamp => "Stamp",
            ToolId::Termites => "Termites",
            ToolId::Washer => "Washer",
        }
    }

    /// The single tool factory — used by both the (future) scene and the simulation, so
    /// the two can't drift apart.
    pub fn make_tool(self) -> Box<dyn Tool> {
        match self {
            ToolId::Hammer => Box::new(Hammer::new()),
            ToolId::ChainSaw => Box::new(ChainSaw::new()),
            ToolId::MachineGun => Box::new(MachineGun::new()),
            ToolId::FlameThrower => Box::new(FlameThrower::new()),
            ToolId::ColorThrower => Box::new(ColorThrower::new()),
            ToolId::Phaser => Box::new(Phaser::new()),
            ToolId::Stamp => Box::new(Stamp::new()),
            ToolId::Termites => Box::new(Termites::new()),
            ToolId::Washer => Box::new(Washer::new()),
        }
    }
}

/// Everything a tool needs from the world, minus the cursor/rendering pieces — see the
/// module doc comment. Bundles several independently-borrowed pieces of a `Rig`
/// (`simulation.rs` in Phase 3; the real app's scene, later) rather than owning anything
/// itself.
pub struct ToolContext<'a> {
    pub damage: &'a mut DamageLayer,
    pub decals: &'a mut DecalFactory,
    pub particles: &'a mut ParticleSystem,
    pub termites: &'a mut TermiteColony,
    pub audio: &'a mut AudioSink,
    pub screen_size: (f32, f32),
    pub rng: &'a mut SeededRng,
    /// The desktop's brightness under a point, 0–255. A plain constant for now rather
    /// than the Swift original's per-point closure — real per-pixel sampling is a Phase 6
    /// concern (`wlr-screencopy`), and every current caller of this (the simulation rig)
    /// only ever returns the same constant regardless of point anyway.
    pub brightness: Option<u8>,
}

impl ToolContext<'_> {
    /// The stereo position for a sound triggered at a given point (`sound stereobase`).
    pub fn pan(&self, point: (f32, f32)) -> f32 {
        crate::audio::pan_for_x(point.0, self.screen_size.0)
    }

    /// Whether a point is inside the screen (with a margin) — tools must not act outside.
    pub fn contains(&self, point: (f32, f32), margin: f32) -> bool {
        point.0 >= -margin && point.1 >= -margin && point.0 <= self.screen_size.0 + margin && point.1 <= self.screen_size.1 + margin
    }
}

/// Evenly spaced points along a segment, together with the direction angle. Used by the
/// saw (which draws the cut) and the washer (which erases) — both have to act
/// continuously along the stroke, not just at its end point.
///
/// A free function, not a `ToolContext` method — it never actually needs `self`, and
/// giving it one would create exactly the closure-capture conflict the rest of this phase
/// has been navigating: `ctx.along_path(..., |p, a| ctx.damage.stamp(...))` would need to
/// borrow `ctx` immutably for the method call while its own closure needs `ctx` mutably.
pub fn along_path(from: (f32, f32), to: (f32, f32), spacing: f32, mut body: impl FnMut((f32, f32), f32)) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let distance = dx.hypot(dy);
    let angle = dy.atan2(dx);
    if spacing <= 0.0 {
        return;
    }
    let steps = ((distance / spacing) as i64).max(1);
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        body((from.0 + dx * t, from.1 + dy * t), angle);
    }
}

pub trait Tool {
    fn id(&self) -> ToolId;

    /// A standard downcast hook — `{ self }` in every impl. Exists so a renderer can get
    /// from `gpu.tools`'s type-erased `Box<dyn Tool>` back to a *specific* tool's own
    /// state it needs to draw (right now: `FlameThrower`'s standing flames, which are
    /// tool-owned, unlike `TermiteColony`/`ParticleSystem` which are `ToolContext`-owned
    /// specifically so nothing needs this trick for *them*).
    fn as_any(&self) -> &dyn std::any::Any;

    fn mouse_down(&mut self, point: (f32, f32), ctx: &mut ToolContext);
    fn mouse_dragged(&mut self, _point: (f32, f32), _ctx: &mut ToolContext) {}
    fn mouse_up(&mut self, _point: (f32, f32), _ctx: &mut ToolContext) {}

    /// For continuous tools (machine gun, flame, saw). `dt` in seconds.
    fn update(&mut self, _dt: f32, _mouse: (f32, f32), _is_down: bool, _ctx: &mut ToolContext) {}

    /// Called when switching away from this tool — stop loops, tidy up state. The
    /// termites deliberately do not tidy up (`termites will not be switched off`).
    fn deactivate(&mut self, _ctx: &mut ToolContext) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_digits_are_1_through_9_in_order() {
        for (i, id) in ToolId::ALL.iter().enumerate() {
            assert_eq!(id.key_digit(), i as i32 + 1);
        }
    }

    #[test]
    fn from_key_digit_round_trips() {
        for id in ToolId::ALL {
            assert_eq!(ToolId::from_key_digit(id.key_digit()), Some(id));
        }
        assert_eq!(ToolId::from_key_digit(0), None);
        assert_eq!(ToolId::from_key_digit(10), None);
    }

    #[test]
    fn make_tool_produces_a_tool_reporting_its_own_id() {
        for id in ToolId::ALL {
            assert_eq!(id.make_tool().id(), id);
        }
    }
}
