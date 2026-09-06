//! Scene state and per-frame simulation — the platform-agnostic half of the Linux
//! binary's `State`, with every Wayland type (`RegistryState`, `LayerSurface`,
//! `zwlr_screencopy_*`, `wp_fractional_scale_*`) stripped out. What's left — the damage
//! layer, the nine tool instances, particles, the termite colony, HUD logic, tool
//! switching, and `advance` — is unchanged from `hakai/src/main.rs`.
//!
//! One window == one `GpuLayer` for now. The `Vec<GpuLayer>` shape is kept so Phase 5
//! (multi-monitor) is an additive change rather than a restructure.

use std::collections::HashMap;
use std::time::Instant;

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

/// One output — here, one window. `wgpu_surface`/`config` are the platform edge; every
/// other field is the same per-output game state the Linux `GpuLayer` carries.
pub struct GpuLayer {
    pub wgpu_surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,

    /// Surface size in the scene's working unit. On Windows that's physical pixels with
    /// `scale == 1.0` for now — winit reports physical pixels for both the window size and
    /// pointer events, and `DamageLayer` treats its inputs as one consistent unit, so
    /// nothing needs a conversion until Phase 5 wires up Per-Monitor DPI properly.
    pub width: u32,
    pub height: u32,
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
    /// Re-runs `surface.configure` with the stored config — after a `Lost`/`Outdated`
    /// acquire, or a resize (which updates the config first).
    pub fn reconfigure_surface(&self, device: &wgpu::Device) {
        self.wgpu_surface.configure(device, &self.config);
    }

    /// A window resize: new pixel size, reconfigure the surface, rebuild the damage grid.
    pub fn resize(&mut self, width: u32, height: u32, assets: &Assets) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.config.width = self.width;
        self.config.height = self.height;
        self.wgpu_surface.configure(&assets.device, &self.config);
        assets.build_damage(self);
        self.configured = true;
    }
}

pub struct State {
    pub assets: Assets,
    pub decals: DecalFactory,
    pub icons: ToolIcons,
    /// Kept for asset rebuilds on a scale/monitor change (Phase 5); unused so far.
    #[allow(dead_code)]
    pub sprites: SpriteFactory,
    pub text: TextRenderer,
    pub audio: AudioSink,
    pub layers: Vec<GpuLayer>,

    /// Whether the cursor is inside the window — the Windows equivalent of the Linux
    /// binary's `pointer_focus` (which output the pointer is over). Gates cursor drawing.
    pub pointer_inside: bool,
    pub shift_held: bool,
}

impl State {
    /// Creates the single `GpuLayer` for a freshly-made window surface at `(width,
    /// height)` physical pixels, configures it, and builds its damage grid.
    pub fn add_window_layer(&mut self, wgpu_surface: wgpu::Surface<'static>, adapter: &wgpu::Adapter, width: u32, height: u32) {
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
            width: width.max(1),
            height: height.max(1),
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
            width: width.max(1),
            height: height.max(1),
            scale: 1.0,
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

    /// One real frame: advance every output, tick audio once, render every output.
    pub fn frame(&mut self) {
        let now = Instant::now();
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
            if i == 0 {
                self.audio.update(dt);
            }
            let show_cursor = self.pointer_inside && i == 0;
            crate::render::render(&self.assets, &mut self.text, &mut self.icons, show_cursor, &mut self.layers[i]);
        }
    }

    // ── Input ────────────────────────────────────────────────────────────────────────────

    pub fn pointer_moved(&mut self, point: (f32, f32)) {
        let Some(gpu) = self.layers.first_mut() else { return };
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

    pub fn pointer_pressed(&mut self, point: (f32, f32)) {
        let Some(gpu) = self.layers.first_mut() else { return };
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
        if self.layers[0].hud.credits_open() {
            return;
        }

        let gpu = &mut self.layers[0];
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

    pub fn pointer_released(&mut self, point: (f32, f32)) {
        let Some(gpu) = self.layers.first_mut() else { return };
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
