//! Hakai on Windows.
//!
//! Phase 0: a transparent, borderless, always-on-top overlay per monitor (needs
//! `Dx12SwapchainKind::DxgiFromVisual` + `WS_EX_NOREDIRECTIONBITMAP` — see
//! `WINDOWS-PLAN.md`). Phase 2: keyboard + pointer into `hakai_core::render::Scene`.
//! Phase 4: DXGI Desktop Duplication feeds the brightness map. Phase 5: one window per
//! monitor, each with its own Per-Monitor-DPI-v2 scale factor.
//!
//! The window/event-loop/input shell is `hakai_core::shell`, shared with the macOS build
//! and the Linux GNOME/X11 fallback; this file is just the Windows hooks into it.
//!
//! Esc quits — from anywhere, via a global hotkey (`HAKAI_NO_GLOBAL_ESC=1` makes it
//! app-local). `HAKAI_WINDOWED=1` forces a single small ordinary window (for boxes whose
//! reported monitor geometry doesn't match the presentable area, e.g. some RDP sessions).

use std::sync::Arc;

use hakai_core::audio::AudioSink;
use hakai_core::render::HudColors;
use hakai_core::shell::winit::event_loop::OwnedDisplayHandle;
use hakai_core::shell::winit::monitor::MonitorHandle;
use hakai_core::shell::winit::platform::windows::WindowAttributesExtWindows;
use hakai_core::shell::winit::window::{Window, WindowAttributes};
use hakai_core::shell::{DesktopCapture, Platform, QuitHandle};

mod duplication;
mod theme;
mod win32;

struct Win;

impl Platform for Win {
    fn name(&self) -> &'static str {
        "Windows"
    }

    fn instance(&self, _display: OwnedDisplayHandle) -> wgpu::Instance {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        desc.backends = wgpu::Backends::DX12;
        desc.backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
        wgpu::Instance::new(desc)
    }

    fn audio(&mut self) -> AudioSink {
        hakai_core::playback::sink()
    }

    /// `WS_EX_NOREDIRECTIONBITMAP` is what makes the DirectComposition presentation path
    /// actually transparent.
    fn window_attributes(&self, attrs: WindowAttributes) -> WindowAttributes {
        attrs.with_no_redirection_bitmap(true)
    }

    /// Before the wgpu surface (and its DComp swapchain) is built, so the final window
    /// styles are in place first.
    fn window_created(&mut self, window: &Window, _monitor: Option<&MonitorHandle>, _windowed: bool) {
        win32::mark_non_occluding(window);
    }

    /// DXGI Desktop Duplication of the **primary** output — `None` when unavailable
    /// (hybrid graphics, RDP, another duplicator already running); tools then fall back to
    /// a random impact-sound variant. A secondary monitor's brightness is sampled from the
    /// primary's capture — no worse than the random fallback; per-output duplication later.
    fn capture(&mut self, _windows: &[Arc<Window>]) -> Option<Box<dyn DesktopCapture>> {
        Some(Box::new(duplication::DesktopDuplication::new()?))
    }

    /// Esc closes hakai from anywhere — even after Alt+Tabbing to another app.
    fn event_loop_ready(&mut self, quit: QuitHandle) {
        win32::spawn_quit_hotkey(move || quit.quit());
    }

    fn cursor_position(&self, window: &Window) -> Option<(f64, f64)> {
        win32::cursor_position(window)
    }

    fn paint_colors(&self) -> Option<[(f32, f32, f32); 8]> {
        theme::read_paint_colors()
    }

    fn hud_colors(&self) -> Option<HudColors> {
        theme::read_hud_colors()
    }
}

fn main() {
    let base = std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".to_string());
    env_logger::Builder::new()
        .parse_filters(&format!("{base},wgpu_core=warn,wgpu_hal=warn,naga=warn"))
        .init();

    // Get the launching terminal out of the way before the overlay covers the screen —
    // while it's still the foreground window.
    win32::minimize_launcher();

    hakai_core::shell::run(Win);
}
