//! Scene state and per-frame simulation — everything both binaries share once their
//! windowing and screen-capture backends are set aside: the per-output `GpuLayer` (damage
//! layer, nine tool instances, particles, termite colony, HUD state, brightness map, a
//! wgpu surface), the shared [`Assets`], and the `advance` / `frame` / tool-switching /
//! pointer-input logic.
//!
//! One `GpuLayer` per output — a `wl_output` on Wayland, a monitor's `Window` on Windows.
//! The binary owns the mapping from its own platform key to a layer index; `Scene` just
//! keeps `layers: Vec<GpuLayer>`. Screen capture is likewise the binary's job — it calls
//! [`Scene::feed_layer_capture`] / [`Scene::feed_capture_all`] with raw frame bytes when
//! its backend produces one.

use std::collections::HashMap;
use std::time::Instant;

use crate::audio::AudioSink;
use crate::colony::TermiteColony;
use crate::hud::Hud;
use crate::icons::ToolIcons;
use crate::particles::ParticleSystem;
use crate::sprites::SpriteFactory;
use crate::tools::{Tool, ToolContext, ToolId};
use crate::{DamageLayer, DecalFactory, SeededRng};

use crate::capture::BrightnessMap;
use super::gpu::{Assets, ToastGpu, TileGpu};
use super::text::TextRenderer;

/// One output. `wgpu_surface`/`config` are the platform edge the binary hands in; every
/// other field is per-output scene state. The binary keys layers however it likes (a
/// `wl_surface`, a winit `WindowId`) and tracks the key→index mapping itself.
pub struct GpuLayer {
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

    /// A resize or scale change. `logical` is the output's size in **points** (Wayland
    /// logical coords, or winit's `inner_size() / scale_factor()`); the wgpu surface buffer
    /// is sized `points * scale`. Rebuilds the damage layer and GPU tiles at the new scale.
    pub fn resize(&mut self, logical: (u32, u32), scale: f32, assets: &Assets) {
        self.width = logical.0.max(1);
        self.height = logical.1.max(1);
        self.scale = if scale > 0.0 { scale } else { 1.0 };
        let (bw, bh) = self.buffer_px();
        self.config.width = bw;
        self.config.height = bh;
        self.wgpu_surface.configure(&assets.device, &self.config);
        assets.build_damage(self);
        self.configured = true;
    }
}

pub struct Scene {
    pub assets: Assets,
    pub decals: DecalFactory,
    pub icons: ToolIcons,
    /// Kept for asset rebuilds on a scale/monitor change; unused so far.
    #[allow(dead_code)]
    pub sprites: SpriteFactory,
    pub text: TextRenderer,
    pub audio: AudioSink,
    pub layers: Vec<GpuLayer>,

    /// Which layer the cursor is currently over — the binary sets this from its own
    /// per-output pointer focus (`wl_pointer` enter/leave, winit `CursorEntered`/`Left`).
    /// Gates cursor drawing to that output.
    pub focused_layer: Option<usize>,
    pub shift_held: bool,
}

impl Scene {
    /// Adds a `GpuLayer` for a surface the binary just created, and returns its index.
    /// `logical` is the output's size in points; `scale` its DPI factor.
    pub fn add_layer(
        &mut self,
        wgpu_surface: wgpu::Surface<'static>,
        adapter: &wgpu::Adapter,
        logical: (u32, u32),
        scale: f32,
    ) -> usize {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let width = logical.0.max(1);
        let height = logical.1.max(1);

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
            width: ((width as f32 * scale).round() as u32).max(1),
            height: ((height as f32 * scale).round() as u32).max(1),
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
        // The Wayland binary adds a layer before its first `configure` event, so its size
        // is still 0×0 here — leave it unconfigured; `resize_layer` finishes it when the
        // real size arrives. The Windows binary always has a real size and is done now.
        if width > 1 && height > 1 {
            self.assets.build_damage(&mut layer);
            layer.configured = true;
        }
        self.layers.push(layer);
        self.layers.len() - 1
    }

    /// A resize or scale change for one output. `logical` is its size in points; the wgpu
    /// buffer is sized `points * scale`. Rebuilds that layer's damage/tiles at the new
    /// scale. (Splits the `&mut layers[i]` / `&assets` borrow so callers don't have to.)
    pub fn resize_layer(&mut self, index: usize, logical: (u32, u32), scale: f32) {
        let assets = &self.assets;
        if let Some(layer) = self.layers.get_mut(index) {
            layer.resize(logical, scale, assets);
        }
    }

    /// Drops the layer at `index` (a monitor unplugged, its window destroyed) — leaves the
    /// rest running. The binary must fix up its own key→index mapping to match.
    pub fn remove_layer(&mut self, index: usize) {
        if index >= self.layers.len() {
            return;
        }
        self.layers.remove(index);
        match self.focused_layer {
            Some(f) if f == index => self.focused_layer = None,
            Some(f) if f > index => self.focused_layer = Some(f - 1),
            _ => {}
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
    /// from the Linux binary's `Scene::advance`, minus the screen-capture bookkeeping.
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

    /// Feeds one freshly captured desktop frame into a single output's brightness map (and
    /// its frozen-mode snapshot texture, if it's frozen and doesn't have one yet). Raw
    /// bytes are BGRA, `stride` bytes per row — see [`crate::capture`]. Used by the Wayland
    /// binary, whose `zwlr_screencopy_v1` captures are per-`wl_output`.
    pub fn feed_layer_capture(&mut self, index: usize, bytes: &[u8], w: u32, h: u32, stride: u32) {
        let assets = &self.assets;
        let Some(layer) = self.layers.get_mut(index) else { return };
        layer.brightness.update(bytes, w, h, stride);
        if layer.frozen && layer.snapshot_texture.is_none() {
            let rgba = crate::capture::to_rgba(bytes, w, h, stride);
            layer.snapshot_texture = Some(super::gpu::upload_snapshot(assets, &rgba, w, h));
        }
    }

    /// Feeds one captured desktop frame into **every** output — used by the Windows binary,
    /// whose single DXGI duplicator covers the primary output and stands in for the rest.
    pub fn feed_capture_all(&mut self, bytes: &[u8], w: u32, h: u32, stride: u32) {
        for i in 0..self.layers.len() {
            self.feed_layer_capture(i, bytes, w, h, stride);
        }
    }

    /// Whether any frozen output is still waiting for its snapshot — a binary can use this
    /// to trigger an off-schedule capture the moment `M` is pressed.
    pub fn wants_snapshot(&self) -> bool {
        self.layers.iter().any(|l| l.frozen && l.snapshot_texture.is_none())
    }

    /// Advance + render one output. `drive_audio` ticks the shared audio gain-glide (must
    /// be true for exactly one output per real frame — see `AudioBackend::update`). `dt` is
    /// measured here from the layer's own `last_frame_time`, clamped so a stalled frame
    /// can't hand a tool a huge step. A no-op until the layer is `configured`.
    ///
    /// The Wayland binary calls this per `wl_surface` frame callback; the Windows binary
    /// loops it from `frame`.
    pub fn tick_layer(&mut self, index: usize, drive_audio: bool, show_cursor: bool) {
        let Some(gpu) = self.layers.get_mut(index) else { return };
        if !gpu.configured {
            return;
        }
        let now = Instant::now();
        let dt = gpu.last_frame_time.map(|t| (now - t).as_secs_f32()).unwrap_or(1.0 / 60.0).min(0.1);
        gpu.last_frame_time = Some(now);

        Scene::advance(&mut self.decals, &mut self.audio, &mut self.layers[index], dt);
        if drive_audio {
            self.audio.update(dt);
        }
        super::gpu::render(&self.assets, &mut self.text, &mut self.icons, show_cursor, &mut self.layers[index]);
    }

    /// One real frame across every output — the Windows binary's driver. The Wayland binary
    /// drives `tick_layer` per surface instead. Screen capture is fed separately (see
    /// `feed_*_capture`), before this.
    pub fn frame(&mut self) {
        for i in 0..self.layers.len() {
            let show_cursor = self.focused_layer == Some(i);
            self.tick_layer(i, i == 0, show_cursor);
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
            super::gpu::palette_tool_at(point, screen)
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
