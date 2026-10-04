//! Hakai on macOS.
//!
//! The window/event-loop/input shell is `hakai_core::shell`, shared with the Windows build
//! and the Linux GNOME/X11 fallback; this file is just the macOS hooks into it. The
//! macOS-specific parts live in `macos.rs` (window level, Spaces, full-screen frame — the
//! Swift original's `OverlayWindow` recipe) and `duplication.rs` (CoreGraphics capture
//! for the brightness map). Metal's `CAMetalLayer` is transparent as soon as it's
//! non-opaque, so there's no equivalent of Windows' DirectComposition dance.
//!
//! Esc or ⌘Q quits. `HAKAI_WINDOWED=1` runs a single ordinary window instead.

use std::sync::Arc;

use hakai_core::audio::AudioSink;
use hakai_core::render::HudColors;
use hakai_core::shell::winit::event_loop::OwnedDisplayHandle;
use hakai_core::shell::winit::keyboard::{KeyCode, ModifiersState};
use hakai_core::shell::winit::monitor::MonitorHandle;
use hakai_core::shell::winit::window::Window;
use hakai_core::shell::{DesktopCapture, Platform};

mod duplication;
mod macos;
mod theme;

struct Mac;

impl Platform for Mac {
    fn name(&self) -> &'static str {
        "macOS"
    }

    fn instance(&self, _display: OwnedDisplayHandle) -> wgpu::Instance {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        desc.backends = wgpu::Backends::METAL;
        wgpu::Instance::new(desc)
    }

    fn audio(&mut self) -> AudioSink {
        hakai_core::playback::sink()
    }

    /// Lifts each window above the menu bar and snaps it to the screen's full frame.
    fn window_created(&mut self, window: &Window, monitor: Option<&MonitorHandle>, windowed: bool) {
        macos::make_overlay(window, monitor, windowed);
    }

    fn started(&mut self, _windows: &[Arc<Window>], windowed: bool) {
        if !windowed {
            macos::take_over_screen();
        }
    }

    /// Captures everything below the primary monitor's overlay window.
    fn capture(&mut self, windows: &[Arc<Window>]) -> Option<Box<dyn DesktopCapture>> {
        let below = macos::window_number(&windows[0])?;
        Some(Box::new(duplication::DesktopDuplication::new(below)?))
    }

    fn paint_colors(&self) -> Option<[(f32, f32, f32); 8]> {
        theme::read_paint_colors()
    }

    fn hud_colors(&self) -> Option<HudColors> {
        theme::read_hud_colors()
    }

    fn cursor_position(&self, window: &Window) -> Option<(f64, f64)> {
        macos::cursor_position(window)
    }

    fn is_quit_key(&self, code: KeyCode, modifiers: ModifiersState) -> bool {
        code == KeyCode::KeyQ && modifiers.super_key()
    }
}

fn main() {
    let base = std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".to_string());
    env_logger::Builder::new()
        .parse_filters(&format!("{base},wgpu_core=warn,wgpu_hal=warn,naga=warn"))
        .init();

    hakai_core::shell::run(Mac);
}
