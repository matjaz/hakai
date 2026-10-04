//! Hakai on macOS — the winit shell.
//!
//! The same structure as hakai-win's `main.rs`: one borderless transparent overlay per
//! monitor, keyboard + pointer into `state::State`, `render::render` per output per frame.
//! The macOS-specific parts live in `macos.rs` (window level, Spaces, full-screen frame —
//! the Swift original's `OverlayWindow` recipe) and `duplication.rs` (CoreGraphics
//! capture for the brightness map). Metal's `CAMetalLayer` is transparent as soon as
//! it's non-opaque, so there's no equivalent of Windows' DirectComposition dance.
//!
//! Esc or ⌘Q quits. `HAKAI_WINDOWED=1` runs a single ordinary window instead.

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::window::{Window, WindowId, WindowLevel};

use hakai_core::audio::AudioSink;
use hakai_core::icons::ToolIcons;
use hakai_core::sprites::SpriteFactory;
use hakai_core::tools::ToolId;
use hakai_core::DecalFactory;

mod duplication;
mod macos;

// The scene, the renderer and the (stubbed) theme are platform-agnostic and shared
// verbatim with the Windows build.
#[path = "../../hakai-win/src/render.rs"]
mod render;
#[path = "../../hakai-win/src/state.rs"]
mod state;
#[path = "../../hakai-win/src/theme.rs"]
mod theme;

#[path = "../../hakai/src/audio.rs"]
mod audio;
#[path = "../../hakai/src/text.rs"]
mod text;
// The Wayland-free brightness grid + snapshot conversion — its producer is
// `duplication::DesktopDuplication` (CGWindowListCreateImage on macOS).
#[path = "../../hakai/src/capture.rs"]
mod capture;

use state::State;
use text::TextRenderer;

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

#[derive(Default)]
struct App {
    instance: Option<wgpu::Instance>,
    adapter: Option<wgpu::Adapter>,
    windows: Vec<Arc<Window>>,
    state: Option<State>,
    modifiers: ModifiersState,
    /// Last cursor position, in the focused layer's point space.
    cursor: (f32, f32),
}

impl App {
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

        let w0 = &self.windows[0];
        let w0_points = w0.inner_size().width as f32 / w0.scale_factor() as f32;
        let assets = render::Assets::build(
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

        let mut st = State {
            assets,
            decals,
            icons,
            sprites,
            text,
            audio,
            layers: Vec::new(),
            duplication: macos::window_number(&self.windows[0]).and_then(duplication::DesktopDuplication::new),
            last_capture: None,
            focused_layer: None,
            shift_held: false,
        };

        // First layer reuses surface0; the rest make their own from the same instance.
        st.add_window_layer(
            self.windows[0].id(),
            surface0,
            &adapter,
            self.windows[0].inner_size().into(),
            self.windows[0].scale_factor() as f32,
        );
        for w in self.windows.iter().skip(1) {
            let surface = instance.create_surface(w.clone()).expect("create_surface");
            st.add_window_layer(w.id(), surface, &adapter, w.inner_size().into(), w.scale_factor() as f32);
        }
        st.select_tool(ToolId::Hammer);

        if !windowed {
            macos::take_over_screen();
        }

        self.instance = Some(instance);
        self.adapter = Some(adapter);
        self.state = Some(st);
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
        if let Some(st) = self.state.as_mut() {
            st.frame();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // Plain f32 copy — taken before `st` borrows `self` mutably.
        let win_scale = self.windows.iter().find(|w| w.id() == id).map(|w| w.scale_factor() as f32);
        let Some(st) = self.state.as_mut() else { return };

        match event {
            WindowEvent::CloseRequested => {
                log::info!("quit: window close requested");
                event_loop.exit();
            }
            WindowEvent::Destroyed => {
                st.remove_window_layer(id);
                self.windows.retain(|w| w.id() != id);
                if self.windows.is_empty() {
                    event_loop.exit();
                }
            }

            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                st.shift_held = self.modifiers.contains(ModifiersState::SHIFT);
            }

            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                let shift = self.modifiers.contains(ModifiersState::SHIFT);
                match code {
                    KeyCode::Escape => {
                        log::info!("quit: Esc");
                        event_loop.exit();
                    }
                    KeyCode::KeyQ if self.modifiers.super_key() => event_loop.exit(),
                    KeyCode::Digit1 => st.select_tool(ToolId::Hammer),
                    KeyCode::Digit2 => st.select_tool(ToolId::ChainSaw),
                    KeyCode::Digit3 => st.select_tool(ToolId::MachineGun),
                    KeyCode::Digit4 => st.select_tool(ToolId::FlameThrower),
                    KeyCode::Digit5 => st.select_tool(ToolId::ColorThrower),
                    KeyCode::Digit6 => st.select_tool(ToolId::Phaser),
                    KeyCode::Digit7 => st.select_tool(ToolId::Stamp),
                    KeyCode::Digit8 => st.select_tool(ToolId::Termites),
                    KeyCode::Digit9 => st.select_tool(ToolId::Washer),
                    KeyCode::KeyR => st.erase_all(),
                    // Shift+Tab arrives as Tab with the shift modifier — one branch
                    // covers both directions.
                    KeyCode::Tab => st.cycle_tool(if shift { -1 } else { 1 }),
                    KeyCode::ArrowUp => st.set_palette_visible(true),
                    KeyCode::ArrowDown => st.set_palette_visible(false),
                    KeyCode::KeyC => st.toggle_credits(),
                    KeyCode::KeyM => st.toggle_mode(),
                    _ => {}
                }
            }

            WindowEvent::CursorEntered { .. } => {
                st.focused_layer = st.layer_index(id);
            }
            WindowEvent::CursorLeft { .. } => {
                if st.focused_layer == st.layer_index(id) {
                    st.focused_layer = None;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let Some(idx) = st.layer_index(id) else { return };
                // winit gives physical pixels, y down from the top-left. The scene works
                // in points — divide by this output's scale once, here, and nowhere else.
                let scale = st.layers[idx].scale.max(0.01);
                self.cursor = (position.x as f32 / scale, position.y as f32 / scale);
                st.focused_layer = Some(idx);
                st.pointer_moved(idx, self.cursor);
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let Some(idx) = st.layer_index(id) {
                    st.pointer_pressed(idx, self.cursor);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                if let Some(idx) = st.layer_index(id) {
                    st.pointer_released(idx, self.cursor);
                }
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(idx) = st.layer_index(id) {
                    // The matching Resized event does the reconfigure; just record the
                    // new scale so it's current when that arrives.
                    st.layers[idx].scale = scale_factor as f32;
                }
            }
            WindowEvent::Resized(size) => {
                if size.width == 0 || size.height == 0 {
                    return;
                }
                let Some(idx) = st.layer_index(id) else { return };
                let scale = win_scale.unwrap_or(st.layers[idx].scale);
                st.layers[idx].resize((size.width, size.height), scale, &st.assets);
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
