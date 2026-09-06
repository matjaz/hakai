//! The wgpu renderer — shared by `hakai` (Wayland) and `hakai-win` (Win32/DXGI).
//!
//! Everything platform-specific — creating the window, the event loop, the surface, and
//! the screen-capture backend — stays in each binary. What lives here is the part that was
//! genuinely identical between them once both moved to the same wgpu major: pipeline
//! construction, the per-tile / sprite / HUD GPU resource builders, the per-frame draw,
//! and HUD text shaping ([`text`]).
//!
//! Gated behind the crate's `render` feature so the headless default build (and its CI
//! job) never compiles wgpu or cosmic-text.

pub mod text;
pub mod theme;
