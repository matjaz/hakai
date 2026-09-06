//! Scene state and per-frame simulation — the platform-agnostic half of the Linux
//! binary's `State`, with every Wayland type (`RegistryState`, `LayerSurface`,
//! `zwlr_screencopy_*`, `wp_fractional_scale_*`) stripped out. What's left — the damage
//! layer, the nine tool instances, particles, the termite colony, HUD logic, tool
//! switching, and `advance` — is unchanged from `hakai/src/main.rs`.
//!
//! One window per monitor, one `GpuLayer` each (Phase 5), mirroring the Linux binary's
//! one-layer-surface-per-`wl_output`. Every `GpuLayer` owns its own damage layer, RNG,
//! tool set and scale factor.

use std::collections::HashMap;
use std::time::Instant;

use winit::window::WindowId;

use hakai_core::audio::AudioSink;
use hakai_core::colony::TermiteColony;
use hakai_core::hud::Hud;
use hakai_core::icons::ToolIcons;
use hakai_core::particles::ParticleSystem;
use hakai_core::sprites::SpriteFactory;
use hakai_core::tools::{Tool, ToolContext, ToolId};
use hakai_core::{DamageLayer, DecalFactory, SeededRng};

use crate::capture::BrightnessMap;
use crate::render::{Assets, ToastGpu, TileGpu};
use crate::text::TextRenderer;

/// One output — one monitor's window. `wgpu_surface`/`config`/`window_id` are the platform
/// edge; every other field is the same per-output game state the Linux `GpuLayer` carries.
pub struct GpuLayer {
    pub window_id: WindowId,
    pub wgpu_surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,

    /// This output's size in **points** (logical units) — the scene's working unit,
    /// matching mouse coordinates and `DamageLayer`, exactly as the Linux binary keeps
    /// `gpu.width`/`gpu.height`. The wgpu surface's own pixel buffer is `points * scale`
    /// (see `config`), computed only where it's needed. NDC is a ratio, so every placement
    /// in `render.rs` comes out identical whether it's expressed in points or pixels.
    pub width: u32,
    pub height: u32,
    /// Per-Monitor DPI v2 scale factor from winit (`1.0`, `1.25`, `1.5`, ...).
    pub scale: f32,
    pub configured: bool,

    pub damage: Option<DamageLayer>,
    pub tiles: Vec<TileGpu>,

    pub particles: ParticleSystem,
    pub termites: TermiteColony,
    pub rng: SeededRng,
    pub tools: HashMap<ToolId, Box<dyn Tool>>,
    pub active_tool: ToolId,
    pub mouse: (f32, f32),
    pub is_down: bool,
    pub last_frame_time: Option<Instant>,

    pub hud: Hud,
    pub toast_gpu: Option<ToastGpu>,

    pub brightness: BrightnessMap,
    pub frozen: bool,
    pub snapshot_texture: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
}

impl GpuLayer {
    fn buffer_px(&self) -> (u32, u32) {
        (
            ((self.width as f32 * self.scale).round() as u32).max(1),
            ((self.height as f32 * self.scale).round() as u32).max(1),
        )
    }

    /// Re-runs `surface.configure` with the stored config — after a `Lost`/`Outdated`
    /// acquire. Kept infallible: a genuinely gone surface (an unplugged monitor) just
    /// keeps failing `get_current_texture` on the next frame, which `render` also tolerates.
    pub fn reconfigure_surface(&self, device: &wgpu::Device) {
        self.wgpu_surface.configure(device, &self.config);
    }

    /// A resize or scale change: `physical` is winit's `inner_size()` in pixels, `scale`
    /// its `scale_factor()`. Points = physical / scale; the surface buffer stays physical.
    pub fn resize(&mut self, physical: (u32, u32), scale: f32, assets: &Assets) {
        self.scale = if scale > 0.0 { scale } else { 1.0 };
        self.width = ((physical.0 as f32 / self.scale).round() as u32).max(1);
        self.height = ((physical.1 as f32 / self.scale).round() as u32).max(1);
        let (bw, bh) = self.buffer_px();
        self.config.width = bw;
        self.config.height = bh;
        self.wgpu_surface.configure(&assets.device, &self.config);
        assets.build_damage(self);
        self.configured = true;
    }
}

pub struct State {
    pub assets: Assets,
    pub decals: DecalFactory,
    pub icons: ToolIcons,
    /// Kept for asset rebuilds on a scale/monitor change; unused so far.
    #[allow(dead_code)]
    pub sprites: SpriteFactory,
    pub text: TextRenderer,
    pub audio: AudioSink,
    pub layers: Vec<GpuLayer>,

    /// DXGI Desktop Duplication of the **primary** output — `None` when it isn't available
    /// (hybrid graphics, a Remote Desktop session, another duplicator already running).
    /// Every tool then falls back to a random impact-sound variant. On a multi-monitor
    /// setup a secondary output's brightness is sampled from the primary's capture — not
    /// accurate, but no worse than the random fallback, and per-output duplication is a
    /// later refinement.
    pub duplication: Option<crate::duplication::DesktopDuplication>,
    /// `None` until the first capture — so the brightness map is populated on the very
    /// first frame rather than after a full `CAPTURE_INTERVAL`.
    pub last_capture: Option<Instant>,

    /// Which layer the cursor is currently over (Wayland's per-surface pointer focus, here
    /// tracked from `CursorEntered`/`CursorLeft`). Gates cursor drawing to that output.
    pub focused_layer: Option<usize>,
    pub shift_held: bool,
}

impl State {
    pub fn layer_index(&self, id: WindowId) -> Option<usize> {
        self.layers.iter().position(|l| l.window_id == id)
    }

    /// Adds a `GpuLayer` for a freshly-made window surface. `physical` is the window's
    /// `inner_size()` in pixels, `scale` its `scale_factor()`.
    pub fn add_window_layer(
        &mut self,
        window_id: WindowId,
        wgpu_surface: wgpu::Surface<'static>,
        adapter: &wgpu::Adapter,
        physical: (u32, u32),
        scale: f32,
    ) {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let width = ((physical.0 as f32 / scale).round() as u32).max(1);
        let height = ((physical.1 as f32 / scale).round() as u32).max(1);

        let caps = wgpu_surface.get_capabilities(adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let alpha_mode = caps
            .alpha_modes
            .iter()
            .copied()
            .find(|m| *m == wgpu::CompositeAlphaMode::PreMultiplied)
            .unwrap_or(caps.alpha_modes[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: physical.0.max(1),
            height: physical.1.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: vec![],
        };
        wgpu_surface.configure(&self.assets.device, &config);

        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1)
            .wrapping_add(self.layers.len() as u64 + 1);

        let mut layer = GpuLayer {
            window_id,
            wgpu_surface,
            config,
            width,
            height,
            scale,
            configured: false,
            damage: None,
            tiles: Vec::new(),
            particles: ParticleSystem::new(),
            termites: TermiteColony::new(),
            rng: SeededRng::new(seed),
            tools: ToolId::ALL.into_iter().map(|id| (id, id.make_tool())).collect(),
            active_tool: ToolId::Hammer,
            mouse: (0.0, 0.0),
            is_down: false,
            last_frame_time: None,
            hud: Hud::new(),
            toast_gpu: None,
            brightness: BrightnessMap::default(),
            frozen: false,
            snapshot_texture: None,
        };
        self.assets.build_damage(&mut layer);
        layer.configured = true;
        self.layers.push(layer);
    }

    /// Drops the layer for a window that's gone (a monitor unplugged, its window
    /// destroyed) — leaves the rest running.
    pub fn remove_window_layer(&mut self, id: WindowId) {
        self.layers.retain(|l| l.window_id != id);
        if let Some(f) = self.focused_layer {
            if f >= self.layers.len() {
                self.focused_layer = None;
            }
        }
    }

    // ── Tool switching and global commands (verbatim from the Linux binary) ────────────────

    pub fn select_tool(&mut self, id: ToolId) {
        for gpu in &mut self.layers {
            if gpu.active_tool == id {
                continue;
            }
            let previous = gpu.active_tool;
            if let (Some(tool), Some(damage)) = (gpu.tools.get_mut(&previous), gpu.damage.as_mut()) {
                let mut ctx = ToolContext {
                    damage,
                    decals: &mut self.decals,
                    particles: &mut gpu.particles,
                    termites: &mut gpu.termites,
                    audio: &mut self.audio,
                    screen_size: (gpu.width as f32, gpu.height as f32),
                    rng: &mut gpu.rng,
                    brightness: gpu.brightness.sample(gpu.mouse),
                };
                tool.deactivate(&mut ctx);
            }
            gpu.active_tool = id;
        }
        self.assets.refresh_tool_label(&mut self.text, id);
        for gpu in &mut self.layers {
            gpu.hud.flash_palette();
        }
    }

    pub fn cycle_tool(&mut self, direction: i32) {
        let Some(current) = self.layers.first().map(|g| g.active_tool) else { return };
        let idx = ToolId::ALL.iter().position(|&t| t == current).unwrap_or(0) as i32;
        let next = (idx + direction).rem_euclid(ToolId::ALL.len() as i32) as usize;
        self.select_tool(ToolId::ALL[next]);
    }

    pub fn erase_all(&mut self) {
        for gpu in &mut self.layers {
            if let Some(damage) = gpu.damage.as_mut() {
                damage.erase_all();
            }
            gpu.hud.show_toast("Desktop cleaned");
        }
    }

    pub fn set_palette_visible(&mut self, visible: bool) {
        for gpu in &mut self.layers {
            gpu.hud.set_palette_visible(visible);
        }
    }

    pub fn toggle_credits(&mut self) {
        for gpu in &mut self.layers {
            gpu.hud.toggle_credits();
        }
    }

    pub fn toggle_mode(&mut self) {
        for gpu in &mut self.layers {
            gpu.frozen = !gpu.frozen;
            if !gpu.frozen {
                gpu.snapshot_texture = None;
            }
        }
    }

    /// Drives one output's active tool, particles and termites forward by `dt`. Verbatim
    /// from the Linux binary's `State::advance`, minus the screen-capture bookkeeping.
    fn advance(decals: &mut DecalFactory, audio: &mut AudioSink, gpu: &mut GpuLayer, dt: f32) {
        let active = gpu.active_tool;
        let mouse = gpu.mouse;
        let is_down = gpu.is_down;

        if let (Some(tool), Some(damage)) = (gpu.tools.get_mut(&active), gpu.damage.as_mut()) {
            let mut ctx = ToolContext {
                damage,
                decals,
                particles: &mut gpu.particles,
                termites: &mut gpu.termites,
                audio: &mut *audio,
                screen_size: (gpu.width as f32, gpu.height as f32),
                rng: &mut gpu.rng,
                brightness: gpu.brightness.sample(mouse),
            };
            tool.update(dt, mouse, is_down, &mut ctx);
        }

        if active != ToolId::FlameThrower {
            if let (Some(tool), Some(damage)) = (gpu.tools.get_mut(&ToolId::FlameThrower), gpu.damage.as_mut()) {
                let mut ctx = ToolContext {
                    damage,
                    decals,
                    particles: &mut gpu.particles,
                    termites: &mut gpu.termites,
                    audio: &mut *audio,
                    screen_size: (gpu.width as f32, gpu.height as f32),
                    rng: &mut gpu.rng,
                    brightness: gpu.brightness.sample(mouse),
                };
                tool.update(dt, mouse, is_down, &mut ctx);
            }
        }

        let screen_size = (gpu.width as f32, gpu.height as f32);
        if let Some(damage) = gpu.damage.as_mut() {
            gpu.particles.update(dt, damage, decals, &mut *audio, &mut gpu.rng, screen_size, &mut gpu.termites);
        }
        if let Some(damage) = gpu.damage.as_mut() {
            gpu.termites.update(dt, damage, decals, &mut *audio, &mut gpu.rng, screen_size);
        }

        gpu.hud.advance(dt);
    }

    /// Requests a fresh desktop capture if one is due (every `CAPTURE_INTERVAL`, or
    /// immediately when a frozen output still has no snapshot), and feeds it to every
    /// output's brightness map. A no-op when duplication isn't available.
    fn capture_tick(&mut self, now: Instant) {
        let Some(dupl) = self.duplication.as_mut() else { return };
        let want_snapshot = self.layers.iter().any(|l| l.frozen && l.snapshot_texture.is_none());
        let due = match self.last_capture {
            None => true,
            Some(t) => want_snapshot || now.duration_since(t).as_secs_f32() >= crate::capture::CAPTURE_INTERVAL,
        };
        if !due {
            return;
        }
        self.last_capture = Some(now);

        let layers = &mut self.layers;
        let assets = &self.assets;
        let got = dupl.capture(|bytes, w, h, stride| {
            for layer in layers.iter_mut() {
                layer.brightness.update(bytes, w, h, stride);
                if layer.frozen && layer.snapshot_texture.is_none() {
                    let rgba = crate::capture::to_rgba(bytes, w, h, stride);
                    layer.snapshot_texture = Some(crate::render::upload_snapshot(assets, &rgba, w, h));
                }
            }
            (w, h)
        });
        if let Some((w, h)) = got {
            log::trace!("desktop capture {w}x{h}");
        }
    }

    /// One real frame: advance every output, tick audio once, render every output.
    pub fn frame(&mut self) {
        let now = Instant::now();
        self.capture_tick(now);
        for i in 0..self.layers.len() {
            if !self.layers[i].configured {
                continue;
            }
            let dt = {
                let gpu = &mut self.layers[i];
                let dt = gpu.last_frame_time.map(|t| (now - t).as_secs_f32()).unwrap_or(1.0 / 60.0).min(0.1);
                gpu.last_frame_time = Some(now);
                dt
            };
            State::advance(&mut self.decals, &mut self.audio, &mut self.layers[i], dt);
            // The audio gain-glide must advance once per real frame total, not once per
            // output — drive it from the first output only, matching AudioEngine.swift.
            if i == 0 {
                self.audio.update(dt);
            }
            let show_cursor = self.focused_layer == Some(i);
            crate::render::render(&self.assets, &mut self.text, &mut self.icons, show_cursor, &mut self.layers[i]);
        }
    }

    // ── Input (per-output; `idx` is the layer the event landed on) ────────────────────────

    pub fn pointer_moved(&mut self, idx: usize, point: (f32, f32)) {
        let Some(gpu) = self.layers.get_mut(idx) else { return };
        gpu.mouse = point;
        if !gpu.is_down {
            return;
        }
        let active = gpu.active_tool;
        if let (Some(tool), Some(damage)) = (gpu.tools.get_mut(&active), gpu.damage.as_mut()) {
            let mut ctx = ToolContext {
                damage,
                decals: &mut self.decals,
                particles: &mut gpu.particles,
                termites: &mut gpu.termites,
                audio: &mut self.audio,
                screen_size: (gpu.width as f32, gpu.height as f32),
                rng: &mut gpu.rng,
                brightness: gpu.brightness.sample(point),
            };
            tool.mouse_dragged(point, &mut ctx);
        }
    }

    pub fn pointer_pressed(&mut self, idx: usize, point: (f32, f32)) {
        let Some(gpu) = self.layers.get_mut(idx) else { return };
        gpu.mouse = point;

        let screen = (gpu.width as f32, gpu.height as f32);
        let palette_hit = if gpu.hud.palette_open() {
            crate::render::palette_tool_at(point, screen)
        } else {
            None
        };
        if let Some(id) = palette_hit {
            self.select_tool(id);
            return;
        }
        if self.layers[idx].hud.credits_open() {
            return;
        }

        let gpu = &mut self.layers[idx];
        gpu.is_down = true;
        let active = gpu.active_tool;
        if let (Some(tool), Some(damage)) = (gpu.tools.get_mut(&active), gpu.damage.as_mut()) {
            let mut ctx = ToolContext {
                damage,
                decals: &mut self.decals,
                particles: &mut gpu.particles,
                termites: &mut gpu.termites,
                audio: &mut self.audio,
                screen_size: (gpu.width as f32, gpu.height as f32),
                rng: &mut gpu.rng,
                brightness: gpu.brightness.sample(point),
            };
            tool.mouse_down(point, &mut ctx);
        }
    }

    pub fn pointer_released(&mut self, idx: usize, point: (f32, f32)) {
        let Some(gpu) = self.layers.get_mut(idx) else { return };
        gpu.mouse = point;
        gpu.is_down = false;
        let active = gpu.active_tool;
        if let (Some(tool), Some(damage)) = (gpu.tools.get_mut(&active), gpu.damage.as_mut()) {
            let mut ctx = ToolContext {
                damage,
                decals: &mut self.decals,
                particles: &mut gpu.particles,
                termites: &mut gpu.termites,
                audio: &mut self.audio,
                screen_size: (gpu.width as f32, gpu.height as f32),
                rng: &mut gpu.rng,
                brightness: gpu.brightness.sample(point),
            };
            tool.mouse_up(point, &mut ctx);
        }
    }
}
