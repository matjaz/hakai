//! The wgpu renderer — shared by `hakai` (Wayland) and `hakai-win` (Win32/DXGI).
//!
//! Everything platform-specific — creating the window, the event loop, the surface, and
//! the screen-capture backend — stays in each binary. What lives here is the part that was
//! genuinely identical between them once both moved to the same wgpu major:
//!
//! - [`gpu`] — pipeline construction, the per-tile / sprite / HUD GPU resource builders,
//!   the shared [`Assets`], and the per-frame draw ([`render`]).
//! - [`scene`] — the per-output [`GpuLayer`] and the [`Scene`] that owns them, plus
//!   `advance` / `frame` / tool-switching / pointer input.
//! - [`text`] — HUD text shaping (cosmic-text).
//! - [`theme`] — the [`HudColors`] the renderer paints with (each binary reads its own).
//!
//! Gated behind the crate's `render` feature so the headless default build (and its CI
//! job) never compiles wgpu or cosmic-text.

pub mod gpu;
pub mod scene;
pub mod text;
pub mod theme;

pub use gpu::{palette_tool_at, render, Assets, Pipelines};
pub use scene::{GpuLayer, Scene};
pub use theme::HudColors;
