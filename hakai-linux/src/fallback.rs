//! The GNOME / X11 fallback — `hakai_core::shell` with Linux hooks.
//!
//! The native path (`main.rs`) needs `wlr-layer-shell`, which GNOME's Mutter doesn't
//! implement and an X11 session doesn't have at all. Here hakai runs the same winit shell
//! as the Windows and macOS builds instead: one borderless, transparent window per monitor,
//! made fullscreen so it covers GNOME's top bar and X11 panels too.
//!
//! What's lost against the native path: no exclusive keyboard grab (Alt+Tab and the Super
//! key still reach the desktop, as on Windows), no `wlr-screencopy` — so the impact sound
//! picks a random variant — and on X11 transparency needs a compositing window manager
//! (every mainstream desktop has one; a bare i3 without picom shows black instead).

use hakai_core::audio::AudioSink;
use hakai_core::render::HudColors;
use hakai_core::shell::winit::event_loop::OwnedDisplayHandle;
use hakai_core::shell::{Coverage, Platform};

use crate::theme;

struct Linux {
    /// No Wayland connection at all — winit will be on X11.
    x11: bool,
}

impl Platform for Linux {
    fn name(&self) -> &'static str {
        "Linux"
    }

    /// Wayland (GNOME): Vulkan, GL as a fallback. X11: GL only — a Vulkan swapchain on a
    /// 32-bit ARGB X11 window presented nothing at all under Mesa's software driver in
    /// testing, while GL (EGL on the same visual) drew the overlay with working alpha.
    /// `WGPU_BACKEND=gl` / `vulkan` overrides either, for troubleshooting a driver.
    fn instance(&self, display: OwnedDisplayHandle) -> wgpu::Instance {
        let default = if self.x11 { wgpu::Backends::GL } else { wgpu::Backends::VULKAN | wgpu::Backends::GL };
        let mut desc = wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display));
        desc.backends = wgpu::Backends::from_env().unwrap_or(default);
        wgpu::Instance::new(desc)
    }

    fn audio(&mut self) -> AudioSink {
        hakai_core::playback::sink()
    }

    fn coverage(&self) -> Coverage {
        Coverage::Fullscreen
    }

    fn paint_colors(&self) -> Option<[(f32, f32, f32); 8]> {
        theme::read_paint_colors()
    }

    fn hud_colors(&self) -> Option<HudColors> {
        theme::read_hud_colors()
    }
}

/// `x11`: there's no Wayland connection, so winit will open X11 windows.
pub fn run(reason: &str, x11: bool) {
    log::info!("{reason} — using the winit fallback (fullscreen transparent windows)");
    hakai_core::shell::run(Linux { x11 });
}
