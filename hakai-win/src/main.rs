//! Hakai on Windows — the winit shell.
//!
//! Phase 0: a transparent, borderless, always-on-top overlay per monitor (needs
//! `Dx12SwapchainKind::DxgiFromVisual` + `WS_EX_NOREDIRECTIONBITMAP` — see
//! `WINDOWS-PLAN.md`). Phase 2: keyboard + pointer into `hakai_core::render::Scene`, and
//! `hakai_core::render::render` per output per frame. Phase 4: DXGI Desktop Duplication
//! feeds the brightness map. Phase 5: one window per monitor, each with its own
//! Per-Monitor-DPI-v2 scale factor.
//!
//! Everything below the window/event-loop/capture-backend line lives in
//! `hakai_core::render` now — the same code the Linux binary runs.
//!
//! Esc quits. `HAKAI_WINDOWED=1` forces a single small ordinary window (for boxes whose
//! reported monitor geometry doesn't match the presentable area, e.g. some RDP sessions).

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Window, WindowId, WindowLevel};

use hakai_core::audio::AudioSink;
use hakai_core::icons::ToolIcons;
use hakai_core::render::{Assets, Scene};
use hakai_core::sprites::SpriteFactory;
use hakai_core::tools::ToolId;
use hakai_core::DecalFactory;

mod duplication;
mod theme;
mod win32;

#[path = "../../hakai/src/audio.rs"]
mod audio;

// The renderer, HUD text shaping, scene state, and the Wayland-free brightness grid +
// snapshot conversion all live in `hakai_core` now (behind its `render` feature). Only the
// producer of the brightness grid is ours — `duplication::DesktopDuplication` (DXGI
// Desktop Duplication).
use hakai_core::render::text::TextRenderer;

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

    /// DXGI Desktop Duplication of the **primary** output — `None` when unavailable
    /// (hybrid graphics, RDP, another duplicator already running); tools then fall back to
    /// a random impact-sound variant. A secondary monitor's brightness is sampled from the
    /// primary's capture — no worse than the random fallback; per-output duplication later.
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

        self.duplication = duplication::DesktopDuplication::new();
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
                    // Windows reports Shift+Tab as Tab with the shift modifier — no
                    // ISO_Left_Tab equivalent, so one branch covers both directions.
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
