//! Hakai's headless core.
//!
//! The default build has no window, no audio backend, no compositor — everything here
//! builds and runs with plain `cargo test` on any OS. See `../PHASE1.md`, `../PHASE2.md`,
//! `../PHASE3.md` for the build/test walkthroughs and `OMARCHY-PORT.md` (in the
//! desktop-destroyer repo) for how each phase fits the overall plan.
//!
//! The optional `render` feature adds the wgpu renderer and HUD text shaping (the
//! [`render`] module) — the code both binaries used to carry a copy of. It pulls in wgpu
//! and cosmic-text, so it stays off unless a binary asks for it.

pub mod audio;
pub mod capture;
pub mod colony;
pub mod credits;
pub mod damage;
pub mod decals;
pub mod fonts;
mod geometry;
pub mod hud;
pub mod icons;
pub mod particles;
#[cfg(feature = "render")]
pub mod render;
pub mod rng;
pub mod simulation;
pub mod sprites;
pub mod tools;

pub use colony::TermiteColony;
pub use damage::{DamageLayer, Rect};
pub use decals::DecalFactory;
pub use particles::ParticleSystem;
pub use rng::SeededRng;
