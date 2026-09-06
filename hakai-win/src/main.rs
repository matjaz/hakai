//! Hakai on Windows — the winit shell.
//!
//! Phase 0: a transparent, borderless, always-on-top overlay per monitor (needs
//! `Dx12SwapchainKind::DxgiFromVisual` + `WS_EX_NOREDIRECTIONBITMAP` — see
//! `WINDOWS-PLAN.md`). Phase 2: keyboard + pointer into `state::State`, the scene ported
//! from the Linux binary, and `render::render` per output per frame. Phase 4: DXGI
//! Desktop Duplication feeds the brightness map. Phase 5: one window per monitor, each
//! with its own Per-Monitor-DPI-v2 scale factor.
//!
//! Esc quits. `HAKAI_WINDOWED=1` forces a single small ordinary window (for boxes whose
//! reported monitor geometry doesn't match the presentable area, e.g. some RDP sessions).

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Window, WindowId, WindowLevel};

use hakai_core::audio::AudioSink;
use hakai_core::icons::ToolIcons;
use hakai_core::sprites::SpriteFactory;
use hakai_core::tools::ToolId;
use hakai_core::DecalFactory;

mod duplication;
mod render;
mod state;
mod theme;
mod win32;

#[path = "../../hakai/src/audio.rs"]
mod audio;

// The renderer, HUD text shaping, and the Wayland-free brightness grid + snapshot
// conversion all live in `hakai_core` now (behind its `render` feature). Only the
// producer of the brightness grid is ours — `duplication::DesktopDuplication` (DXGI
// Desktop Duplication).
use hakai_core::capture;
use hakai_core::render::text::TextRenderer;
use state::State;

fn build_instance() -> wgpu::Instance {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    desc.backends = wgpu::Backends::DX12;
    desc.backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
    wgpu::Instance::new(desc)
}

/// A borderless, transparent, always-on-top window covering exactly `monitor` (explicit
/// bounds, not `Fullscreen::Borderless` — the plan's preference, to stay off the
/// exclusive-mode / mode-switch paths). `WS_EX_NOREDIRECTIONBITMAP` is what makes the
/// DirectComposition presentation path actually transparent.
fn overlay_attributes(monitor: Option<&MonitorHandle>) -> winit::window::WindowAttributes {
    let mut attrs = Window::default_attributes()
        .with_title("Hakai")
        .with_transparent(true)
        .with_no_redirection_bitmap(true)
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

/// Posted into the event loop from the global-hotkey thread (`win32::spawn_quit_hotkey`).
#[derive(Debug, Clone, Copy)]
enum UserEvent {
    Quit,
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
        // is the one DXGI Desktop Duplication captures.
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
            // Before the wgpu surface (and its DComp swapchain) is built, so the final
            // window styles are in place first.
            win32::mark_non_occluding(&w);
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
        .expect("no D3D12 adapter");
        log::info!("adapter: {:?}", adapter.get_info());

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("hakai-win"),
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
            None => log::info!("paint palette: built-in default (Windows)"),
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
            duplication: duplication::DesktopDuplication::new(),
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

        self.instance = Some(instance);
        self.adapter = Some(adapter);
        self.state = Some(st);
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.windows.is_empty() {
            self.init(event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Quit => event_loop.exit(),
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
            WindowEvent::CloseRequested => event_loop.exit(),
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
                    KeyCode::Escape => event_loop.exit(),
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
                    // Windows reports Shift+Tab as Tab with the shift modifier — no
                    // ISO_Left_Tab equivalent, so one branch covers both directions.
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

    // Get the launching terminal out of the way before the overlay covers the screen —
    // while it's still the foreground window.
    win32::minimize_launcher();

    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    // Esc closes hakai from anywhere — even after Alt+Tabbing to another app. The hotkey
    // thread posts UserEvent::Quit back into the loop.
    let proxy = event_loop.create_proxy();
    win32::spawn_quit_hotkey(move || {
        let _ = proxy.send_event(UserEvent::Quit);
    });

    event_loop.run_app(&mut App::default()).expect("run_app");
}
