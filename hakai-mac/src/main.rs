//! Hakai on macOS — the winit shell.
//!
//! The same structure as hakai-win's `main.rs`: one borderless transparent overlay per
//! monitor, keyboard + pointer into `hakai_core::render::Scene`, `hakai_core::render::render`
//! per output per frame. The macOS-specific parts live in `macos.rs` (window level, Spaces,
//! full-screen frame — the Swift original's `OverlayWindow` recipe) and `duplication.rs`
//! (CoreGraphics capture for the brightness map). Metal's `CAMetalLayer` is transparent as
//! soon as it's non-opaque, so there's no equivalent of Windows' DirectComposition dance.
//!
//! Esc or ⌘Q quits. `HAKAI_WINDOWED=1` runs a single ordinary window instead.

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::window::{Window, WindowId, WindowLevel};

use hakai_core::audio::AudioSink;
use hakai_core::icons::ToolIcons;
use hakai_core::render::{Assets, Scene};
use hakai_core::sprites::SpriteFactory;
use hakai_core::tools::ToolId;
use hakai_core::DecalFactory;

mod duplication;
mod macos;
mod theme;

#[path = "../../hakai/src/audio.rs"]
mod audio;

// The renderer, HUD text shaping, scene state, and the Wayland-free brightness grid +
// snapshot conversion all live in `hakai_core` now (behind its `render` feature). Only the
// producer of the brightness grid is ours — `duplication::DesktopDuplication`
// (CGWindowListCreateImage).
use hakai_core::render::text::TextRenderer;

fn build_instance() -> wgpu::Instance {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    desc.backends = wgpu::Backends::METAL;
    wgpu::Instance::new(desc)
}

/// A borderless, transparent window covering `monitor`. Explicit bounds, not
/// `Fullscreen::Borderless` — on macOS that opens a new Space with nothing behind the
/// window, the opposite of an overlay. `macos::make_overlay` then lifts it above the menu
/// bar and snaps it to the screen's full frame.
fn overlay_attributes(monitor: Option<&MonitorHandle>) -> winit::window::WindowAttributes {
    let mut attrs = Window::default_attributes()
        .with_title("Hakai")
        .with_transparent(true)
        .with_decorations(false)
        .with_resizable(false)
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_visible(true);

    if std::env::var("HAKAI_WINDOWED").is_ok() {
        attrs = attrs
            .with_decorations(true)
            .with_inner_size(winit::dpi::PhysicalSize::new(1100, 620));
    } else if let Some(m) = monitor {
        attrs = attrs.with_position(m.position()).with_inner_size(m.size());
    }
    attrs
}

/// winit's `inner_size()` is physical pixels; the scene works in points. One divide, here.
fn logical_size(w: &Window) -> (u32, u32) {
    let scale = (w.scale_factor() as f32).max(0.01);
    let size = w.inner_size();
    (
        ((size.width as f32 / scale).round() as u32).max(1),
        ((size.height as f32 / scale).round() as u32).max(1),
    )
}

#[derive(Default)]
struct App {
    instance: Option<wgpu::Instance>,
    adapter: Option<wgpu::Adapter>,
    windows: Vec<Arc<Window>>,
    /// `WindowId` for each `scene.layers[i]`, same order — this binary's key→index map.
    window_ids: Vec<WindowId>,
    scene: Option<Scene>,

    /// Capture of the **primary** display — `None` without the Screen Recording
    /// permission; tools then fall back to a random impact-sound variant. A secondary
    /// monitor's brightness is sampled from the primary's capture, as on Windows.
    duplication: Option<duplication::DesktopDuplication>,
    /// `None` until the first capture, so the brightness map is populated on frame 1.
    last_capture: Option<Instant>,

    modifiers: ModifiersState,
    /// Last cursor position, in the focused layer's point space.
    cursor: (f32, f32),
}

impl App {
    fn layer_index(&self, id: WindowId) -> Option<usize> {
        self.window_ids.iter().position(|w| *w == id)
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) {
        // Primary monitor first, then the rest — so layer 0 is the primary output, which
        // is the one the desktop capture reads.
        let mut monitors: Vec<MonitorHandle> = Vec::new();
        if let Some(p) = event_loop.primary_monitor() {
            monitors.push(p.clone());
            monitors.extend(event_loop.available_monitors().filter(|m| *m != p));
        } else {
            monitors.extend(event_loop.available_monitors());
        }
        let windowed = std::env::var("HAKAI_WINDOWED").is_ok() || monitors.is_empty();

        let make_windows: Vec<Option<MonitorHandle>> = if windowed {
            vec![None]
        } else {
            monitors.iter().cloned().map(Some).collect()
        };

        for m in &make_windows {
            let w = Arc::new(event_loop.create_window(overlay_attributes(m.as_ref())).expect("create_window"));
            w.set_cursor_visible(false);
            macos::make_overlay(&w, m.as_ref(), windowed);
            log::info!("window {:?}: {:?} @ scale {}", w.id(), w.inner_size(), w.scale_factor());
            self.windows.push(w);
        }

        let instance = build_instance();
        let surface0 = instance.create_surface(self.windows[0].clone()).expect("create_surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface0),
            ..Default::default()
        }))
        .expect("no Metal adapter");
        log::info!("adapter: {:?}", adapter.get_info());

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("hakai-mac"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        }))
        .expect("request_device");

        let caps = surface0.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);

        let mut decals = DecalFactory::new();
        match theme::read_paint_colors() {
            Some(colors) => decals.set_paint_colors(colors),
            None => log::info!("paint palette: built-in default (macOS)"),
        }
        let hud_colors = theme::read_hud_colors().unwrap_or(theme::HudColors::FALLBACK);

        let mut icons = ToolIcons::new();
        let mut sprites = SpriteFactory::new();
        let mut text = TextRenderer::new();

        let w0_points = logical_size(&self.windows[0]).0 as f32;
        let assets = Assets::build(
            device, queue, format, &mut icons, &mut sprites, &mut decals, &mut text, hud_colors, w0_points,
        );

        let audio = match audio::CpalBackend::new() {
            Some(backend) => {
                log::info!("audio: cpal backend started");
                AudioSink::with_backend(Box::new(backend))
            }
            None => {
                log::warn!("audio: no backend — running silent");
                AudioSink::new()
            }
        };

        let mut scene = Scene {
            assets,
            decals,
            icons,
            sprites,
            text,
            audio,
            layers: Vec::new(),
            focused_layer: None,
            shift_held: false,
        };

        // First layer reuses surface0; the rest make their own from the same instance.
        scene.add_layer(surface0, &adapter, logical_size(&self.windows[0]), self.windows[0].scale_factor() as f32);
        self.window_ids.push(self.windows[0].id());
        for w in self.windows.iter().skip(1) {
            let surface = instance.create_surface(w.clone()).expect("create_surface");
            scene.add_layer(surface, &adapter, logical_size(w), w.scale_factor() as f32);
            self.window_ids.push(w.id());
        }
        scene.select_tool(ToolId::Hammer);

        self.duplication = macos::window_number(&self.windows[0]).and_then(duplication::DesktopDuplication::new);
        if !windowed {
            macos::take_over_screen();
        }
        self.instance = Some(instance);
        self.adapter = Some(adapter);
        self.scene = Some(scene);
    }

    /// Requests a fresh desktop capture if one is due (every `CAPTURE_INTERVAL`, or the
    /// moment a frozen output still lacks its snapshot) and feeds it to every output.
    fn capture_tick(&mut self) {
        if self.duplication.is_none() || self.scene.is_none() {
            return;
        }
        let now = Instant::now();
        let due = {
            let scene = self.scene.as_ref().unwrap();
            match self.last_capture {
                None => true,
                Some(t) => {
                    scene.wants_snapshot()
                        || now.duration_since(t).as_secs_f32() >= hakai_core::capture::CAPTURE_INTERVAL
                }
            }
        };
        if !due {
            return;
        }
        self.last_capture = Some(now);

        let scene = self.scene.as_mut().unwrap();
        let dupl = self.duplication.as_mut().unwrap();
        let got = dupl.capture(|bytes, w, h, stride| {
            scene.feed_capture_all(bytes, w, h, stride);
            (w, h)
        });
        if let Some((w, h)) = got {
            log::trace!("desktop capture {w}x{h}");
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.windows.is_empty() {
            self.init(event_loop);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Drive the whole scene from here rather than per-window RedrawRequested: capture,
        // audio and advance must run once per frame total, and `get_current_texture`'s
        // vsync wait inside `render` self-throttles the loop to the refresh rate.
        self.capture_tick();
        if let Some(scene) = self.scene.as_mut() {
            scene.frame();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // Plain copies taken before `scene` borrows `self` mutably.
        let win_scale = self.windows.iter().find(|w| w.id() == id).map(|w| w.scale_factor() as f32);
        let idx = self.layer_index(id);
        let Some(scene) = self.scene.as_mut() else { return };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Destroyed => {
                if let Some(idx) = idx {
                    scene.remove_layer(idx);
                    self.window_ids.remove(idx);
                }
                self.windows.retain(|w| w.id() != id);
                if self.windows.is_empty() {
                    event_loop.exit();
                }
            }

            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                scene.shift_held = self.modifiers.contains(ModifiersState::SHIFT);
            }

            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                let shift = self.modifiers.contains(ModifiersState::SHIFT);
                match code {
                    KeyCode::Escape => event_loop.exit(),
                    KeyCode::KeyQ if self.modifiers.super_key() => event_loop.exit(),
                    KeyCode::Digit1 => scene.select_tool(ToolId::Hammer),
                    KeyCode::Digit2 => scene.select_tool(ToolId::ChainSaw),
                    KeyCode::Digit3 => scene.select_tool(ToolId::MachineGun),
                    KeyCode::Digit4 => scene.select_tool(ToolId::FlameThrower),
                    KeyCode::Digit5 => scene.select_tool(ToolId::ColorThrower),
                    KeyCode::Digit6 => scene.select_tool(ToolId::Phaser),
                    KeyCode::Digit7 => scene.select_tool(ToolId::Stamp),
                    KeyCode::Digit8 => scene.select_tool(ToolId::Termites),
                    KeyCode::Digit9 => scene.select_tool(ToolId::Washer),
                    KeyCode::KeyR => scene.erase_all(),
                    // Shift+Tab arrives as Tab with the shift modifier — one branch
                    // covers both directions.
                    KeyCode::Tab => scene.cycle_tool(if shift { -1 } else { 1 }),
                    KeyCode::ArrowUp => scene.set_palette_visible(true),
                    KeyCode::ArrowDown => scene.set_palette_visible(false),
                    KeyCode::KeyC => scene.toggle_credits(),
                    KeyCode::KeyM => scene.toggle_mode(),
                    _ => {}
                }
            }

            WindowEvent::CursorEntered { .. } => {
                scene.focused_layer = idx;
            }
            WindowEvent::CursorLeft { .. } => {
                if scene.focused_layer == idx {
                    scene.focused_layer = None;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let Some(idx) = idx else { return };
                let scale = scene.layers[idx].scale.max(0.01);
                self.cursor = (position.x as f32 / scale, position.y as f32 / scale);
                scene.focused_layer = Some(idx);
                scene.pointer_moved(idx, self.cursor);
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let Some(idx) = idx {
                    scene.pointer_pressed(idx, self.cursor);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                if let Some(idx) = idx {
                    scene.pointer_released(idx, self.cursor);
                }
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(idx) = idx {
                    // The matching Resized event does the reconfigure; just record the
                    // new scale so it's current when that arrives.
                    scene.layers[idx].scale = scale_factor as f32;
                }
            }
            WindowEvent::Resized(size) => {
                if size.width == 0 || size.height == 0 {
                    return;
                }
                let Some(idx) = idx else { return };
                let scale = win_scale.unwrap_or(scene.layers[idx].scale).max(0.01);
                let logical = (
                    ((size.width as f32 / scale).round() as u32).max(1),
                    ((size.height as f32 / scale).round() as u32).max(1),
                );
                // Split borrow: `resize` needs `&layers[idx]` and `&assets` at once.
                let assets = &scene.assets;
                scene.layers[idx].resize(logical, scale, assets);
            }

            _ => {}
        }
    }
}

fn main() {
    let base = std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".to_string());
    env_logger::Builder::new()
        .parse_filters(&format!("{base},wgpu_core=warn,wgpu_hal=warn,naga=warn"))
        .init();

    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut App::default()).expect("run_app");
}
