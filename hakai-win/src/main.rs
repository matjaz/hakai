//! Hakai on Windows — Phase 0 spike (see `WINDOWS-PLAN.md`).
//!
//! One transparent, borderless, always-on-top window covering the primary monitor, with a
//! `wgpu` D3D12 surface clearing to a premultiplied semi-transparent red and drawing one
//! opaque green triangle, over the live desktop. It exists to settle the single question
//! the whole port estimate rests on:
//!
//!   **Can `wgpu` present a per-pixel-alpha surface to a Win32 window over the live
//!   desktop, at 60 fps, without trapping the user?**
//!
//! ## What Phase 0 found
//!
//! - `wgpu` 22 (what the Linux binary pins) — D3D12 offers `alpha_modes: [Opaque]` only on
//!   a plain HWND. No per-pixel alpha. This is the plan's "If Phase 0 fails" case.
//! - `wgpu` 27+ added `Dx12SwapchainKind::DxgiFromVisual` — a DXGI swapchain fed through a
//!   `DirectComposition` visual. With it, `alpha_modes` gains `PreMultiplied`.
//! - The HWND must also carry `WS_EX_NOREDIRECTIONBITMAP` (`with_no_redirection_bitmap`),
//!   or the DComp visual composites over the window's opaque GDI redirection surface and
//!   the result is opaque anyway.
//! - With both in place: real per-pixel alpha over the live desktop, 50+ fps, Alt+Tab and
//!   Esc both work. **Route 1 is viable** — at the cost of moving the project off wgpu 22.
//!
//! Esc quits. `HAKAI_WINDOWED=1` runs it as a small ordinary window (useful on boxes whose
//! reported monitor size doesn't match the presentable area, e.g. some RDP sessions).

use std::sync::Arc;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Window, WindowId, WindowLevel};

// ── Win32 layered-window fallback ────────────────────────────────────────────────────────

/// The plan's RISK mitigation: if the surface still comes up `Opaque`, force the extended
/// style ourselves. `LWA_ALPHA` with alpha 255 leaves the per-pixel alpha the swapchain
/// writes intact rather than applying a uniform window-wide alpha — it just flips the
/// window into the layered-composition path. Not needed once `DxgiFromVisual` +
/// `WS_EX_NOREDIRECTIONBITMAP` are in play, but kept as the documented last resort.
fn force_layered(window: &Window) {
    use windows::Win32::Foundation::{COLORREF, HWND};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA,
        WS_EX_LAYERED,
    };

    let RawWindowHandle::Win32(handle) = window.window_handle().expect("window handle").as_raw()
    else {
        log::warn!("not a Win32 window handle — skipping the layered-window fallback");
        return;
    };

    let hwnd = HWND(handle.hwnd.get() as *mut _);
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
    }
    log::info!("applied WS_EX_LAYERED + SetLayeredWindowAttributes fallback");
}

// ── GPU state ────────────────────────────────────────────────────────────────────────────

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
}

impl Gpu {
    fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();

        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        desc.backends = wgpu::Backends::DX12;
        // The DirectComposition presentation path — the reason this spike uses modern wgpu
        // rather than the Linux binary's pinned 22. Without it, D3D12 is `Opaque` only.
        desc.backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
        let instance = wgpu::Instance::new(desc);

        let surface = instance.create_surface(window.clone()).expect("create_surface");

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no D3D12 adapter");
        log::info!("adapter: {:?}", adapter.get_info());

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("hakai-win spike"),
            required_features: wgpu::Features::empty(),
            // The real thing draws full-screen damage tiles — downlevel_defaults caps the
            // max texture size at 2048 and panics configuring anything bigger.
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        }))
        .expect("request_device");

        let caps = surface.get_capabilities(&adapter);
        // ── THE FACT PHASE 0 EXISTS TO ESTABLISH ──────────────────────────────────────────
        log::warn!("alpha modes: {:?}", caps.alpha_modes);
        log::warn!("formats:     {:?}", caps.formats);

        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            log::warn!(">>> PreMultiplied available — Route 1 works");
            wgpu::CompositeAlphaMode::PreMultiplied
        } else if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PostMultiplied) {
            log::warn!(">>> only PostMultiplied — usable, but clear colours must NOT be premultiplied");
            wgpu::CompositeAlphaMode::PostMultiplied
        } else {
            log::error!(">>> Opaque only — see 'If Phase 0 fails' in WINDOWS-PLAN.md");
            wgpu::CompositeAlphaMode::Opaque
        };

        // A straight (non-sRGB) format so the premultiplied clear colour lands on screen
        // with the numbers we wrote, not gamma-shifted.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| *f == wgpu::TextureFormat::Bgra8Unorm)
            .unwrap_or(caps.formats[0]);
        log::info!("format {format:?}, alpha {alpha_mode:?}");

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spike shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spike layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spike pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self { surface, device, queue, config, pipeline }
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    fn render(&mut self) {
        use wgpu::CurrentSurfaceTexture as C;
        let frame = match self.surface.get_current_texture() {
            C::Success(f) | C::Suboptimal(f) => f,
            C::Outdated | C::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            other => {
                log::warn!("get_current_texture: {other:?} — skipping frame");
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spike pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // PREMULTIPLIED semi-transparent red: rgb already * a. Getting this
                        // wrong looks exactly like the alpha not working.
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.35, g: 0.0, b: 0.0, a: 0.35 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }
}

const SHADER: &str = r#"
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i) - 1) * 0.9;
    let y = f32(i32(i & 1u) * 2 - 1) * 0.9;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.1, 0.8, 0.2, 1.0); // opaque green
}
"#;

// ── App ──────────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct App {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let monitor = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next());
        if let Some(m) = &monitor {
            log::warn!("monitor {:?}: pos {:?} size {:?} scale {}", m.name(), m.position(), m.size(), m.scale_factor());
        }

        // WS_EX_NOREDIRECTIONBITMAP: drop the window's own GDI redirection surface so the
        // only thing composited for this HWND is wgpu's DirectComposition visual — that's
        // what makes DxgiFromVisual actually transparent to the windows behind it rather
        // than compositing over an opaque backing. winit skips its own DWM blur-behind
        // transparency hack when this is set alongside with_transparent (window.rs:1232).
        let mut attrs = Window::default_attributes()
            .with_title("hakai-win spike")
            .with_transparent(true)
            .with_no_redirection_bitmap(true)
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop);

        if std::env::var("HAKAI_WINDOWED").is_ok() {
            attrs = attrs
                .with_decorations(true)
                .with_position(winit::dpi::LogicalPosition::new(80.0, 80.0))
                .with_inner_size(winit::dpi::LogicalSize::new(640.0, 400.0));
        } else if let Some(m) = &monitor {
            // Explicit monitor bounds rather than Fullscreen::Borderless — the plan's
            // preference, to stay off the exclusive-mode / mode-switch paths.
            attrs = attrs.with_position(m.position()).with_inner_size(m.size());
        }

        let window = Arc::new(event_loop.create_window(attrs).expect("create_window"));
        window.set_cursor_visible(false);
        log::warn!("window inner_size: {:?}", window.inner_size());

        let gpu = Gpu::new(window.clone());
        if gpu.config.alpha_mode == wgpu::CompositeAlphaMode::Opaque {
            force_layered(&window);
        }

        window.request_redraw();
        self.window = Some(window);
        self.gpu = Some(gpu);
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                event_loop.exit()
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.render();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let base = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    env_logger::Builder::new().parse_filters(&base).init();

    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut App::default()).expect("run_app");
}
