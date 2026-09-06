//! Hakai on Windows — Phase 2: window, input, renderer.
//!
//! A transparent, borderless, always-on-top overlay (see Phase 0 / `WINDOWS-PLAN.md` for
//! why it needs `Dx12SwapchainKind::DxgiFromVisual` + `WS_EX_NOREDIRECTIONBITMAP`), a
//! `winit` event loop, keyboard and pointer events routed into `state::State` — the
//! platform-agnostic scene ported from the Linux binary — and one `render::render` call
//! per frame.
//!
//! Esc quits. `HAKAI_WINDOWED=1` runs it as a small ordinary window (for boxes whose
//! reported monitor size doesn't match the presentable area, e.g. some RDP sessions).

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
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

#[path = "../../hakai/src/audio.rs"]
mod audio;
#[path = "../../hakai/src/text.rs"]
mod text;
// The Wayland-free brightness grid + snapshot conversion — its producer is
// `duplication::DesktopDuplication` (DXGI Desktop Duplication).
#[path = "../../hakai/src/capture.rs"]
mod capture;

use state::State;
use text::TextRenderer;

fn build_wgpu(window: Arc<Window>) -> (wgpu::Surface<'static>, wgpu::Adapter, wgpu::Device, wgpu::Queue) {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    desc.backends = wgpu::Backends::DX12;
    desc.backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
    let instance = wgpu::Instance::new(desc);

    let surface = instance.create_surface(window).expect("create_surface");

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
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

    (surface, adapter, device, queue)
}

#[derive(Default)]
struct App {
    window: Option<Arc<Window>>,
    state: Option<State>,
    modifiers: ModifiersState,
    cursor: (f32, f32),
}

impl App {
    fn init(&mut self, event_loop: &ActiveEventLoop) {
        let monitor = event_loop.primary_monitor().or_else(|| event_loop.available_monitors().next());

        let mut attrs = Window::default_attributes()
            .with_title("Hakai")
            .with_transparent(true)
            .with_no_redirection_bitmap(true)
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop);

        if std::env::var("HAKAI_WINDOWED").is_ok() {
            // Physical pixels, not logical — so a box reporting a bogus DPI scale (some RDP
            // sessions claim 3x on a 1280x720 desktop) still gets a window that fits.
            attrs = attrs
                .with_decorations(true)
                .with_inner_size(winit::dpi::PhysicalSize::new(1100, 620));
        } else if let Some(m) = &monitor {
            attrs = attrs.with_position(m.position()).with_inner_size(m.size());
        }

        let window = Arc::new(event_loop.create_window(attrs).expect("create_window"));
        window.set_cursor_visible(false);
        let size = window.inner_size();
        log::info!("window {}x{}", size.width, size.height);

        let (surface, adapter, device, queue) = build_wgpu(window.clone());

        // Surface format: a non-sRGB one, so the already-final tile pixel bytes aren't
        // re-encoded on write.
        let caps = surface.get_capabilities(&adapter);
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

        let assets = render::Assets::build(
            device,
            queue,
            format,
            &mut icons,
            &mut sprites,
            &mut decals,
            &mut text,
            hud_colors,
            size.width as f32,
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
            pointer_inside: false,
            shift_held: false,
        };
        st.add_window_layer(surface, &adapter, size.width, size.height);
        // The initial tool label is already built for the hammer; make the palette flash
        // match a real selection so nothing looks half-initialised.
        st.select_tool(ToolId::Hammer);

        window.request_redraw();
        self.window = Some(window);
        self.state = Some(st);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.init(event_loop);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(st) = self.state.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

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
                    // ISO_Left_Tab equivalent, so the one branch covers both directions.
                    KeyCode::Tab => st.cycle_tool(if shift { -1 } else { 1 }),
                    KeyCode::ArrowUp => st.set_palette_visible(true),
                    KeyCode::ArrowDown => st.set_palette_visible(false),
                    KeyCode::KeyC => st.toggle_credits(),
                    KeyCode::KeyM => st.toggle_mode(),
                    _ => {}
                }
            }

            WindowEvent::CursorEntered { .. } => {
                st.pointer_inside = true;
            }
            WindowEvent::CursorLeft { .. } => {
                st.pointer_inside = false;
            }
            WindowEvent::CursorMoved { position, .. } => {
                // winit gives physical pixels, y down from the top-left — the scene's own
                // working unit here (scale 1.0). Flip nothing.
                self.cursor = (position.x as f32, position.y as f32);
                st.pointer_moved(self.cursor);
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                st.pointer_pressed(self.cursor);
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                st.pointer_released(self.cursor);
            }

            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let Some(layer) = st.layers.first_mut() {
                        if (layer.width, layer.height) != (size.width, size.height) {
                            layer.resize(size.width, size.height, &st.assets);
                        }
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                st.frame();
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
