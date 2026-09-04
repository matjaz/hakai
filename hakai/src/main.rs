//! Hakai — Phase 4, chunk 2: real input, driving the Phase 3 tools.
//!
//! Builds on chunk 1 (the damage layer as real GPU textures, proven working) by replacing
//! its scripted single crack with actual Wayland pointer/keyboard input: motion, click and
//! drag drive whichever `hakai_core::tools::Tool` is currently selected (1–9 to switch,
//! `R` to clear, Tab to cycle), exactly the way `ToolSimulation` drove them in Phase 3 —
//! just from real input instead of a synthetic stroke.
//!
//! **Per-output game state, not shared.** Wayland routes pointer focus to one surface at a
//! time, but a standing flame or a walking termite has to keep evolving on every output
//! regardless of which one currently has focus. So each `GpuLayer` gets its own
//! `ParticleSystem`, `TermiteColony`, RNG and full set of `Tool` instances (matching how
//! Swift's `GameScene` — one per screen — owns all of this per-scene too) — sharing tool
//! instances across outputs would double-fire a tool's internal cooldown timers the moment
//! a second monitor exists, since every output's frame callback would call the *same*
//! tool's `update()`.
//!
//! See PHASE4.md for the walkthrough.

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_keyboard, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};

use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::{
    globals::registry_queue_init, protocol::wl_keyboard, protocol::wl_output,
    protocol::wl_pointer, protocol::wl_seat, protocol::wl_shm, protocol::wl_surface,
    Connection, Dispatch, Proxy, QueueHandle,
};

// `zwlr_screencopy_v1` is wlroots-specific — `smithay-client-toolkit` doesn't wrap it, so
// its raw generated bindings (a sibling crate to `wayland-client`, not part of `sctk`'s own
// reexports) are used directly. See the "Screen capture" module section.
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1};

// `wp_fractional_scale_v1`/`wp_viewporter` — generic (non-wlroots) protocols, so from the
// sibling `wayland-protocols` crate rather than the `-wlr` one above. See the "Fractional
// scale" module section.
use wayland_protocols::wp::fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};

use wgpu::util::DeviceExt;

use std::collections::HashMap;
use std::time::Instant;

use hakai_core::audio::AudioSink;
use hakai_core::colony::TermiteColony;
use hakai_core::hud::Hud;
use hakai_core::icons::ToolIcons;
use hakai_core::particles::{ParticleKind, ParticleSystem};
use hakai_core::sprites::SpriteFactory;
use hakai_core::tools::chain_saw::ChainSaw;
use hakai_core::tools::flame_thrower::FlameThrower;
use hakai_core::tools::hammer::Hammer;
use hakai_core::tools::machine_gun::MachineGun;
use hakai_core::tools::{Tool, ToolContext, ToolId};
use hakai_core::{DamageLayer, DecalFactory, SeededRng};

mod audio;
mod capture;
mod text;
mod theme;
use text::TextRenderer;

fn main() {
    // Defaults to `warn` when `RUST_LOG` isn't set at all — quiet for a normal launch
    // (from a keybind, an app launcher, or a packaged install), since every build/test
    // instruction during this project's own development explicitly set `RUST_LOG=info`
    // itself; a real end user never would, and got a wall of "uploaded N textures"/"HUD
    // panel built"-style diagnostic noise as a result (caught from an actual packaged
    // run, not assumed). `RUST_LOG=info`/`RUST_LOG=debug` still work exactly as before for
    // anyone who wants that output back.
    //
    // Plain `env_logger::init()` would let `wgpu_core`/`wgpu_hal`'s own `info`-level
    // internals (e.g. "Device::maintain: waiting for submission index N", logged on
    // essentially every frame) drown out this app's own logging at `RUST_LOG=info`.
    // Always capping those two at `warn` — appended after whatever `RUST_LOG` says, so it
    // wins regardless of ordering (`env_logger` matches the most specific directive for a
    // module, not just the last one) — keeps `RUST_LOG=info`/`RUST_LOG=debug` useful for
    // this app's own output without silencing genuine `wgpu` warnings/errors. The
    // tradeoff: debugging `wgpu` itself now needs editing this line, not just the
    // environment variable.
    let base_filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".to_string());
    env_logger::Builder::new().parse_filters(&format!("{base_filter},wgpu_core=warn,wgpu_hal=warn,naga=warn")).init();

    let conn = Connection::connect_to_env().expect(
        "could not connect to a Wayland compositor — this has to run inside a live \
         Hyprland session, not a TTY",
    );

    let (globals, mut event_queue) =
        registry_queue_init(&conn).expect("failed to initialize the wl_registry");
    let qh = event_queue.handle();

    let compositor_state =
        CompositorState::bind(&globals, &qh).expect("wl_compositor is not advertised");
    let layer_shell =
        LayerShell::bind(&globals, &qh).expect("zwlr_layer_shell_v1 is not advertised — is this really wlroots/Hyprland?");
    let output_state = OutputState::new(&globals, &qh);
    let seat_state = SeatState::new(&globals, &qh);

    // Phase 6: screen capture, for the brightness-driven impact sound — see the "Screen
    // capture" module section. Both optional, unlike the globals above: a non-wlroots
    // compositor (unlikely on Omarchy, but not impossible) or a `wl_shm`-less setup
    // (essentially impossible, but the type is `Option` either way) just means every
    // tool falls back to a random impact variant, same as this port's behaviour since
    // Phase 3 — not a reason to refuse to start at all, unlike `wl_compositor`/
    // `zwlr_layer_shell_v1` above.
    let screencopy_manager = globals
        .bind::<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, State, ()>(&qh, 1..=3, ())
        .inspect_err(|e| log::warn!("zwlr_screencopy_manager_v1 not available ({e}) — impact sounds will use a random variant"))
        .ok();
    let shm = Shm::bind(&globals, &qh).inspect_err(|e| log::warn!("wl_shm not available ({e}) — screen capture disabled")).ok();
    // One shared pool, sized generously up front (64MiB covers a 4K BGRA8 frame with
    // room to spare) rather than grown on demand — simpler, and this only ever holds one
    // in-flight capture buffer per output at a time.
    let shm_pool = shm.as_ref().and_then(|shm| SlotPool::new(64 * 1024 * 1024, shm).inspect_err(|e| log::warn!("failed to create the screen-capture SHM pool: {e}")).ok());

    // Phase 6, chunk 3: `wp_fractional_scale_v1` + `wp_viewporter`, for correct rendering
    // on a non-integer output scale — see the "Fractional scale" module section. Both
    // optional: without either, every output just stays at `scale: 1.0`, which is already
    // correct for the (more common) integer-scaled case.
    let fractional_scale_manager = globals
        .bind::<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, State, ()>(&qh, 1..=1, ())
        .inspect_err(|e| log::warn!("wp_fractional_scale_v1 not available ({e}) — fractional-scale outputs will render at integer scale instead"))
        .ok();
    let viewporter = globals.bind::<wp_viewporter::WpViewporter, State, ()>(&qh, 1..=1, ()).inspect_err(|e| log::warn!("wp_viewporter not available ({e})")).ok();

    // wgpu setup, shared across every output's layer surface. Explicitly Vulkan, not
    // `Instance::default()`'s auto-selection — that fell back to the GLES/EGL backend on
    // at least one real run, which then failed creating an EGL context outright
    // (`eglCreateContext` → `EGL_BAD_MATCH`). Vulkan via Mesa is what the plan always
    // targeted for Omarchy anyway.
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });

    // Audio is optional — `AudioEngine.swift`'s own doc comment: "if `AVAudioEngine`
    // fails to start, the app carries on normally." `AudioSink::new()` (no backend) is
    // the same genuine no-op it's always been, so a missing/failed audio device costs
    // nothing but sound.
    let audio_sink = match audio::CpalBackend::new() {
        Some(backend) => {
            log::info!("audio: cpal backend started");
            AudioSink::with_backend(Box::new(backend))
        }
        None => {
            log::warn!("audio: no backend available — running without sound");
            AudioSink::new()
        }
    };

    let mut state = State {
        registry_state: RegistryState::new(&globals),
        output_state,
        compositor_state,
        layer_shell,
        seat_state,
        keyboard: None,
        shift_held: false,
        pointer: None,
        pointer_focus: None,
        instance,
        adapter: None,
        device: None,
        queue: None,
        pipelines: None,
        sampler: None,
        decals: DecalFactory::new(),
        hud_colors: theme::HudColors::FALLBACK,
        audio: audio_sink,
        screencopy_manager,
        shm,
        shm_pool,
        fractional_scale_manager,
        viewporter,
        icons: ToolIcons::new(),
        icon_gpu: HashMap::new(),
        cursor_uniform_buffer: None,
        sprites: SpriteFactory::new(),
        termite_textures: Vec::new(),
        shell_texture: None,
        droplet_textures: Vec::new(),
        flame_textures: Vec::new(),
        sliver_textures: Vec::new(),
        flash_texture: None,
        text: TextRenderer::new(),
        hud_panel: None,
        hud_hint: None,
        hud_label: None,
        palette_panel: None,
        palette_cell_normal: None,
        palette_cell_selected: None,
        palette_icons: HashMap::new(),
        palette_digits: HashMap::new(),
        credits_panel: None,
        layers: Vec::new(),
        exit: false,
        qh: qh.clone(),
        conn: conn.clone(),
    };

    // Phase 7: paint the color-thrower/droplets with the active Omarchy theme's own
    // colours instead of the built-in default set — see `theme.rs`. Has to happen before
    // anything ever calls `paint_splat`/`droplet` (nothing has yet, this early in `main`),
    // since both cache by colour *index*, not colour value — see
    // `DecalFactory::set_paint_colors`'s own doc comment.
    match theme::read_paint_colors() {
        Some(colors) => {
            state.decals.set_paint_colors(colors);
            log::info!("paint palette: sourced from the active Omarchy theme");
        }
        None => log::info!("paint palette: using the built-in default (no Omarchy theme found)"),
    }
    match theme::read_hud_colors() {
        Some(colors) => {
            state.hud_colors = colors;
            log::info!("HUD colours: sourced from the active Omarchy theme");
        }
        None => log::info!("HUD colours: using the built-in default (no Omarchy theme found)"),
    }

    // Let the compositor tell us about the outputs it already has before we create
    // anything — a fresh connection has an empty output list until the first roundtrip.
    event_queue.roundtrip(&mut state).unwrap();

    let mut event_loop: EventLoop<State> =
        EventLoop::try_new().expect("failed to create the calloop event loop");
    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .expect("failed to insert the Wayland source into the event loop");

    for output in state.output_state.outputs().collect::<Vec<_>>() {
        state.spawn_layer_for_output(&qh, output);
    }

    log::info!(
        "hakai phase 4 chunk 1: {} layer surface(s) up — Esc to quit",
        state.layers.len()
    );

    while !state.exit {
        event_loop
            .dispatch(std::time::Duration::from_millis(16), &mut state)
            .expect("event loop dispatch failed");
    }

    log::info!("exiting — dropping every layer surface");
}

// ── Per-tile GPU resources ──────────────────────────────────────────────────────────────

/// One damage tile's GPU-side resources. `bind_group` references `texture` and a small
/// uniform buffer holding this tile's placement, already converted to clip space — see
/// `tile_ndc` — since that placement is static between resizes; only the texture's pixel
/// *contents* change per frame.
struct TileGpu {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

/// Must match `shader.wgsl`'s `Tile` struct exactly: two `vec2<f32>` back to back, no
/// padding needed since both are already 8-byte aligned.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TileUniform {
    ndc_origin: [f32; 2],
    ndc_size: [f32; 2],
}

/// This tile's placement in clip space. wgpu's NDC is y-up; this port's scene convention
/// (matching Wayland input and `tiny-skia`'s raster storage) is y-down — so `ndc_size.y`
/// comes out *negative* here, which is what makes the shader's unit quad (whose `y = 0`
/// corner is this tile's top edge, in our y-down sense) land at the larger NDC y without
/// needing any flip inside the shader itself. Same fix, same reason, as everywhere else
/// in this port that's crossed a y-up/y-down boundary — just done once, here, rather than
/// per shape.
/// `size_px` needn't be square — only tiles and icons happen to be; a termite's sprite
/// (34×19) isn't.
fn tile_ndc(origin_px: (f32, f32), size_px: (f32, f32), screen_px: (f32, f32)) -> TileUniform {
    TileUniform {
        ndc_origin: [-1.0 + 2.0 * origin_px.0 / screen_px.0, 1.0 - 2.0 * origin_px.1 / screen_px.1],
        ndc_size: [2.0 * size_px.0 / screen_px.0, -2.0 * size_px.1 / screen_px.1],
    }
}

fn create_tile_gpu(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, side_px: u32, ndc: TileUniform) -> TileGpu {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("damage-tile"),
        size: wgpu::Extent3d { width: side_px, height: side_px, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("tile-uniform"),
        contents: bytemuck::bytes_of(&ndc),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tile-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });

    TileGpu { texture, bind_group }
}

/// Uploads every dirty tile's pixels to its texture, then clears the dirty set. Called at
/// the start of every `render()` — so a resize/rebuild plus the very next frame is enough
/// to get pixels on screen, no separate "upload once at setup" call needed anywhere else.
fn upload_dirty_tiles(queue: &wgpu::Queue, gpu: &mut GpuLayer) {
    let Some(damage) = &gpu.damage else { return };
    let dirty: Vec<usize> = damage.dirty_indices().collect();
    if !dirty.is_empty() {
        log::debug!("uploading {} dirty tile(s): {:?}", dirty.len(), dirty);
    }
    for i in dirty {
        let (Some((pixels, w, h)), Some(tile)) = (damage.tile_pixels(i), gpu.tiles.get(i)) else { continue };
        // RISK: `wgpu` renamed these copy-destination/layout types (`ImageCopyTexture` →
        // `TexelCopyTextureInfo`, `ImageDataLayout` → `TexelCopyBufferLayout`) at some
        // point in its history close to the version pinned here (`wgpu = "22"`). If this
        // doesn't compile, that rename is almost certainly why — try the `TexelCopy*`
        // names instead.
        queue.write_texture(
            wgpu::ImageCopyTexture { texture: &tile.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            pixels,
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }
    if let Some(damage) = &mut gpu.damage {
        damage.commit();
    }
}

// ── Cursor icons ─────────────────────────────────────────────────────────────────────────
//
// Rides the rotatable-sprite pipeline (`vs_sprite`/`fs_sprite`, a `RotatedSprite` uniform),
// not the axis-aligned tile one — the hammer's cursor genuinely rotates (its knock
// animation, see `Hammer::cursor_rotation`), and every other tool's icon is just the
// zero-rotation case of the same math (see `icon_metadata`'s pivot-defaults-to-hotspot
// comment), so there's no reason to keep two separate code paths for "icon" vs "icon that
// happens to spin." A tile's placement is static between resizes (baked into its own
// uniform buffer once), but the cursor moves (and now rotates) every frame, so all eleven
// icon variants' bind groups share *one* dynamic uniform buffer, rewritten via
// `write_buffer` right before each frame's single cursor draw. That's safe in a way
// reusing a buffer across *tile* uploads wouldn't have been: there's exactly one write and
// one read here, strictly ordered within a single frame's submission — not several
// different tiles' worth of data racing to land in the same buffer before any of their
// draws actually execute.

struct IconGpu {
    /// Kept alive for the bind group's texture view — never rewritten after creation,
    /// unlike a damage tile's texture, so nothing else in this file reads it again.
    #[allow(dead_code)]
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

fn create_icon_gpu(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, cursor_uniform: &wgpu::Buffer, pixmap: &tiny_skia::Pixmap) -> IconGpu {
    let (w, h) = (pixmap.width(), pixmap.height());
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("icon"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        pixmap.data(),
        wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("icon-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: cursor_uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });

    IconGpu { texture, bind_group }
}

// ── Termites ─────────────────────────────────────────────────────────────────────────────
//
// A termite's position/heading/walk-frame changes every frame, and there can be up to 500
// of them — nothing like a tile's static placement or the cursor's one-entity-per-frame
// case. Reusing a single dynamic uniform buffer across many termites drawn in the *same*
// submission would hit exactly the ordering problem tile uploads were designed around in
// the first place (all the `write_buffer` calls would land before any of the draws that
// were supposed to see them individually, so every termite would end up drawn with
// whichever termite's data was written last). So each termite instead gets a *fresh*
// uniform buffer and bind group, created and dropped within the same frame — simple and
// correct, at the cost of some avoidable per-frame allocation. Left as a known
// performance-only limitation to revisit if 200+ termites visibly stutters — correctness
// first, matching how every other chunk in this port has been sequenced.

/// Texture + view only, no bind group — unlike `IconGpu`, termite frames get a fresh bind
/// group per draw (see above), so there's no bind group to cache alongside the texture.
fn create_sprite_texture(device: &wgpu::Device, queue: &wgpu::Queue, pixmap: &tiny_skia::Pixmap) -> (wgpu::Texture, wgpu::TextureView) {
    let (w, h) = (pixmap.width(), pixmap.height());
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sprite"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        pixmap.data(),
        wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Like `create_sprite_texture`, but from a raw RGBA8 byte buffer directly rather than a
/// `tiny_skia::Pixmap` — for frozen mode's captured-snapshot background
/// (`capture::to_rgba`'s own output), which has no reason to exist as a `Pixmap` first
/// (no `tiny-skia` drawing ever touches it).
fn create_texture_from_rgba(device: &wgpu::Device, queue: &wgpu::Queue, rgba: &[u8], width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("snapshot"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        rgba,
        wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4 * width), rows_per_image: Some(height) },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

// ── HUD elements ─────────────────────────────────────────────────────────────────────────
//
// The HUD bar's panel and every label on it (tool name, hint, toast) share one shape: a
// small, non-rotating rectangle whose *pixel size* isn't known ahead of time the way an
// icon's or a sprite's is (a label's depends on the string it renders), so each gets its
// own dedicated uniform buffer + bind group — built once alongside the texture, rewritten
// via `write_buffer` every frame purely for placement, the same "write right before the
// one draw that reads it" safety already established for the cursor. Unlike the cursor,
// though, several of these are on screen in the *same* frame (the bar, the label and the
// hint all draw every frame the HUD is up), so they can't share one buffer the way the
// cursor's icon variants — never more than one drawn per frame — safely do.

struct HudGpu {
    /// Kept alive for the bind group's texture view — neither is read directly again
    /// after construction (the bind group already references the view), same as
    /// `IconGpu::texture`.
    #[allow(dead_code)]
    texture: wgpu::Texture,
    #[allow(dead_code)]
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    /// For a text element, the exact string this texture was rasterized from — the cache
    /// key `ensure_hud_text` compares against. Left empty for the (never-rebuilt) panel.
    source: String,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

fn create_hud_gpu(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, pixmap: &tiny_skia::Pixmap, source: String) -> HudGpu {
    let (width, height) = (pixmap.width(), pixmap.height());
    let (texture, view) = create_sprite_texture(device, queue, pixmap);
    let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hud-uniform"),
        size: std::mem::size_of::<TileUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("hud-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    HudGpu { texture, view, width, height, source, uniform_buffer, bind_group }
}

fn create_hud_text(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, text: &mut TextRenderer, s: &str, size_px: f32, bold: bool, color: [u8; 4]) -> Option<HudGpu> {
    let pixmap = text.rasterize(s, size_px, bold, color)?;
    Some(create_hud_gpu(device, queue, layout, sampler, &pixmap, s.to_string()))
}

/// Rebuilds `*cache` only if `s` differs from what it was last built from (or there's no
/// cache yet) — the common case, every frame a label's text hasn't changed, is then just a
/// string comparison, not a re-shape/re-rasterize/re-upload/re-bind-group. `s.is_empty()`
/// clears the cache instead of building a zero-size texture.
#[allow(clippy::too_many_arguments)]
fn ensure_hud_text(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, text: &mut TextRenderer, cache: &mut Option<HudGpu>, s: &str, size_px: f32, bold: bool, color: [u8; 4]) {
    if cache.as_ref().map(|c| c.source == s).unwrap_or(false) {
        return;
    }
    *cache = if s.is_empty() { None } else { create_hud_text(device, queue, layout, sampler, text, s, size_px, bold, color) };
}

/// Writes this element's placement into its own uniform buffer and issues its draw — the
/// tile/icon (axis-aligned) pipeline must already be bound. `anchor` is where in the
/// element's own box `origin_px` points to (`(0, 0)` top-left, `(0.5, 0.5)` centre,
/// `(1, 0.5)` right-centre, ...), matching `SKLabelNode`'s
/// `horizontalAlignmentMode`/`verticalAlignmentMode` — needed because, unlike every other
/// texture in this renderer, a HUD label's size depends on the string it shows, so callers
/// place it by a meaningful point (screen centre, a bar's right edge) rather than always
/// knowing its top-left corner themselves.
fn draw_hud_element(queue: &wgpu::Queue, pass: &mut wgpu::RenderPass<'_>, element: &HudGpu, anchor_px: (f32, f32), anchor: (f32, f32), screen_px: (f32, f32)) {
    let size_px = (element.width as f32, element.height as f32);
    let origin_px = (anchor_px.0 - anchor.0 * size_px.0, anchor_px.1 - anchor.1 * size_px.1);
    let ndc = tile_ndc(origin_px, size_px, screen_px);
    queue.write_buffer(&element.uniform_buffer, 0, bytemuck::bytes_of(&ndc));
    pass.set_bind_group(0, &element.bind_group, &[]);
    pass.draw(0..6, 0..1);
}

/// The toast's own texture — see the field doc comment on `GpuLayer::toast_gpu` for why
/// this is simpler than `HudGpu` (no persistent bind group).
struct ToastGpu {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    source: String,
}

/// Rebuilds `*cache` only if `s` differs from what it was last built from — the toast's
/// own version of `ensure_hud_text`.
fn ensure_toast_gpu(device: &wgpu::Device, queue: &wgpu::Queue, text: &mut TextRenderer, cache: &mut Option<ToastGpu>, s: &str, size_px: f32, color: [u8; 4]) {
    if cache.as_ref().map(|c| c.source == s).unwrap_or(false) {
        return;
    }
    *cache = if s.is_empty() {
        None
    } else {
        text.rasterize(s, size_px, false, color).map(|pixmap| {
            let (width, height) = (pixmap.width(), pixmap.height());
            let (texture, view) = create_sprite_texture(device, queue, &pixmap);
            ToastGpu { texture, view, width, height, source: s.to_string() }
        })
    };
}

/// A termite's own display size in points — `SpriteFactory::termite`'s 34×19 native
/// pixmap, scaled by the termite's own random per-spawn `scale` (see
/// `colony.rs::TermiteView`), matching `Termites.swift`'s
/// `CGSize(width: 34 * scale, height: 19 * scale)`.
const TERMITE_BASE_SIZE: (f32, f32) = (34.0, 19.0);

/// A standing flame's own display size in points, before its per-spawn `FlameView::scale`
/// — matches `FlameThrower.swift`'s `SKSpriteNode(size: CGSize(width: 58 * scale, height:
/// 100 * scale))` exactly.
const FLAME_BASE_SIZE: (f32, f32) = (58.0, 100.0);

/// A flame's anchor sits near its *base*, not its centre — `FlameThrower.swift` sets
/// `anchorPoint = CGPoint(x: 0.5, y: 0.10)` (SpriteKit y-up: 10% of the sprite's height
/// below the anchor, 90% above — "a flame grows upwards from its base"). In this port's
/// y-down pixel space that 90%-above-the-anchor region is the smaller-y side, so the
/// anchor sits `0.40 * height` *below* (larger y than) this quad's own centre — this is
/// how far to shift a flame's `FlameView::position` up (toward smaller y) to get the
/// centre `rotated_sprite_ndc` actually wants.
const FLAME_ANCHOR_TO_CENTER_Y: f32 = 0.40;

// ── HUD layout ───────────────────────────────────────────────────────────────────────────
//
// `GameScene.swift`'s `buildHUD`/`ToolPaletteHUD.swift`'s own constants, carried over
// directly — every position below is expressed the same way Swift's was (screen-relative,
// y measured from the *bottom* of the screen), converted to this port's y-down pixel space
// at the one call site that actually places each element, not here.

/// `buildHUD`'s `bar`: `SKShapeNode(rectOf: CGSize(width: 760, height: 34), cornerRadius:
/// 10)`. `HUD_PANEL_CORNER_RADIUS` isn't used yet — see `build_panel_pixmap`'s doc comment.
const HUD_BAR_SIZE: (f32, f32) = (760.0, 34.0);
#[allow(dead_code)]
const HUD_PANEL_CORNER_RADIUS: f32 = 10.0;
/// `bar.position = CGPoint(x: size.width / 2, y: 40)` — 40pt up from the bottom edge.
const HUD_BAR_BOTTOM_MARGIN: f32 = 40.0;
/// `toolLabel.position = CGPoint(x: -width/2 + 16, y: 0)` / `hint.position = CGPoint(x:
/// width/2 - 16, y: 0)`, both relative to the bar's own centre.
const HUD_BAR_PADDING: f32 = 16.0;
const HUD_LABEL_SIZE: f32 = 14.0;
const HUD_HINT_SIZE: f32 = 12.0;
/// `flashLabel.position = CGPoint(x: size.width / 2, y: 92)`.
const HUD_TOAST_BOTTOM_MARGIN: f32 = 92.0;
const HUD_TOAST_SIZE: f32 = 15.0;

// `M mode` restored now that `toggle_mode` actually exists — the original hint (from
// chunk 4g, before frozen mode was implemented) deliberately left it out.
const HUD_HINT_TEXT: &str = "1\u{2013}9 tool \u{b7} \u{2191}\u{2193} palette \u{b7} M mode \u{b7} C credits \u{b7} R clear \u{b7} Esc quit";

// ── Tool palette layout ──────────────────────────────────────────────────────────────────
//
// `ToolPaletteHUD.swift`'s own constants and layout math, carried over directly — see the
// "Tool palette" module section further down for why every element here draws through
// `draw_rotated_sprite` rather than `draw_hud_element`.

const PALETTE_CELL: f32 = 66.0;
const PALETTE_GAP: f32 = 8.0;
/// `ToolPaletteHUD.swift`'s `bottom` — 86pt up from the screen's bottom edge.
const PALETTE_BOTTOM_MARGIN: f32 = 86.0;
const PALETTE_ICON_SIZE: f32 = PALETTE_CELL - 12.0;
const PALETTE_DIGIT_SIZE: f32 = 10.0;
/// `nameLabel.position.y = bottom + cell + 18` (y-up).
const PALETTE_NAME_MARGIN: f32 = PALETTE_BOTTOM_MARGIN + PALETTE_CELL + 18.0;

/// The full row's width — a function of `ToolId::ALL.len()`, not a hardcoded number, so it
/// can never drift out of sync with `palette_cell_center`/the panel size below it.
fn palette_total_width() -> f32 {
    let count = ToolId::ALL.len() as f32;
    count * PALETTE_CELL + (count - 1.0) * PALETTE_GAP
}

fn palette_start_x(screen_width: f32) -> f32 {
    (screen_width - palette_total_width()) / 2.0
}

/// This cell's centre in pixels, y-down. `index` is the tool's position in `ToolId::ALL`.
fn palette_cell_center(index: usize, screen: (f32, f32)) -> (f32, f32) {
    let x = palette_start_x(screen.0) + index as f32 * (PALETTE_CELL + PALETTE_GAP) + PALETTE_CELL / 2.0;
    // The whole row shares one vertical centre — also the panel's own, since the panel is
    // just the row's bounding box plus a 12pt margin on every side.
    let y = screen.1 - PALETTE_BOTTOM_MARGIN - PALETTE_CELL / 2.0;
    (x, y)
}

/// The tool under `point`, or `None` — `ToolPaletteHUD.swift`'s `tool(at:)`, minus the
/// `isVisible` guard (the caller checks `hud.palette_open()` itself, since that's
/// per-output state this free function has no access to). The half-gap hit-test margin
/// (`insetBy(dx: -gap/2, dy: -gap/2)`) is carried over too — a click in the gap between two
/// cells still lands one of them, rather than nothing.
fn palette_tool_at(point: (f32, f32), screen: (f32, f32)) -> Option<ToolId> {
    let half = PALETTE_CELL / 2.0 + PALETTE_GAP / 2.0;
    for (i, id) in ToolId::ALL.into_iter().enumerate() {
        let (cx, cy) = palette_cell_center(i, screen);
        if (point.0 - cx).abs() <= half && (point.1 - cy).abs() <= half {
            return Some(id);
        }
    }
    None
}

/// This tool's palette icon pixmap — the same accessors `icon_metadata` uses for the
/// cursor, minus the hotspot/pivot/size (a palette icon is always drawn at
/// `PALETTE_ICON_SIZE`, centred in its cell, never rotated).
fn palette_icon_pixmap(icons: &mut ToolIcons, id: ToolId) -> &tiny_skia::Pixmap {
    match id {
        ToolId::Hammer => &icons.hammer().pixmap,
        ToolId::ChainSaw => &icons.chain_saw(false).pixmap,
        ToolId::MachineGun => &icons.machine_gun().pixmap,
        ToolId::FlameThrower => &icons.flame_thrower().pixmap,
        ToolId::ColorThrower => &icons.color_thrower().pixmap,
        ToolId::Phaser => &icons.phaser().pixmap,
        ToolId::Stamp => &icons.stamp(false).pixmap,
        ToolId::Termites => &icons.termite_hand().pixmap,
        ToolId::Washer => &icons.washer().pixmap,
    }
}

/// `theme::HudColors`' `(u8, u8, u8)` + an alpha → `tiny_skia::Color`. Every HUD/palette
/// panel below reads its fill/stroke colours through this rather than a literal
/// `Color::from_rgba8(0, 0, 0, ...)`/`(255, 255, 255, ...)`, so they track
/// `self.hud_colors` (the active Omarchy theme, or `HudColors::FALLBACK` off Omarchy)
/// instead of being hardcoded to black/white.
fn hud_rgba(color: (u8, u8, u8), alpha: u8) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(color.0, color.1, color.2, alpha)
}

/// Same idea as `hud_rgba`, for the `[u8; 4]` shape `TextRenderer::rasterize` wants.
fn hud_rgba_arr(color: (u8, u8, u8), alpha: u8) -> [u8; 4] {
    [color.0, color.1, color.2, alpha]
}

/// The HUD bar's backing panel. `GameScene.swift`'s `bar` is actually a *rounded* rect
/// (`cornerRadius: 10`) — `tiny-skia`'s `PathBuilder` has no rounded-rect primitive of its
/// own (the same reason `hakai_core::geometry::push_rounded_rect` exists, hand-building
/// one from cubic curves for `icons.rs`/`decals.rs`, both inside `hakai-core` and so not
/// reachable from this crate) — simplified to square corners for now rather than
/// duplicating that helper here. A real, minor visual gap from the Swift original, not an
/// oversight; worth revisiting once the rest of the HUD is confirmed working.
fn build_panel_pixmap(width_px: u32, height_px: u32, fill: tiny_skia::Color, stroke: Option<(tiny_skia::Color, f32)>) -> tiny_skia::Pixmap {
    let mut pixmap = tiny_skia::Pixmap::new(width_px, height_px).expect("nonzero HUD panel size");
    let (w, h) = (width_px as f32, height_px as f32);

    // `fill_path`, not `Pixmap::fill_rect` — matches the exact pattern
    // `hakai_core::sprites`'s own flat-rect shapes (the shell's case, the beam) already
    // use successfully, rather than a second, unverified tiny-skia convenience method.
    let mut fill_paint = tiny_skia::Paint::default();
    fill_paint.set_color(fill);
    fill_paint.anti_alias = true;
    if let Some(rect) = tiny_skia::Rect::from_ltrb(0.0, 0.0, w, h) {
        let path = tiny_skia::PathBuilder::from_rect(rect);
        pixmap.fill_path(&path, &fill_paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
    }

    // The stroke border, inset by half its own width so it doesn't get clipped at the
    // edge. `None` (an unselected palette cell's `strokeColor = .clear`) skips this
    // entirely rather than stroking with a transparent paint.
    if let Some((color, width)) = stroke {
        let mut stroke_paint = tiny_skia::Paint::default();
        stroke_paint.set_color(color);
        stroke_paint.anti_alias = true;
        let stroke_style = tiny_skia::Stroke { width, ..Default::default() };
        let inset = width / 2.0;
        if let Some(rect) = tiny_skia::Rect::from_ltrb(inset, inset, w - inset, h - inset) {
            let path = tiny_skia::PathBuilder::from_rect(rect);
            pixmap.stroke_path(&path, &stroke_paint, &stroke_style, tiny_skia::Transform::identity(), None);
        }
    }

    pixmap
}

// ── Credits panel ────────────────────────────────────────────────────────────────────────
//
// `CreditsPanel.swift` lays out dozens of individually-positioned `SKLabelNode`s onto one
// panel. This port composites the same content into a *single* pixmap instead — one
// texture, one draw, rather than one bind group per line — using `blit_over` to paste each
// line's own tightly-cropped `TextRenderer::rasterize` output onto the panel's background
// at the right offset. Built once (like the status bar/palette), from the first output's
// width; a real, minor simplification for multi-output setups whose screens differ in
// width, since `CreditsPanel.swift` builds a fresh one per scene. `hakai_core::credits`
// supplies the wrapped, colored lines; everything pixel-level (the font, the panel size,
// the actual layout) lives here, matching the split `build`'s own doc comment describes.

/// The monospaced font's actual per-character advance in pixels, measured by rasterizing a
/// reference string rather than reaching for font-metrics APIs `text.rs` doesn't expose —
/// `CreditsPanel.swift`'s own `body.maximumAdvancement.width` reasoning (a monospaced font
/// makes a character count an exact width budget), just measured empirically instead of
/// queried.
fn measure_char_width(text: &mut TextRenderer, size_px: f32) -> f32 {
    const REFERENCE: &str = "MMMMMMMMMM";
    // The color here is never seen — only the ink-bounds *width* this pixmap measures is
    // used, not its pixels — so this stays a plain literal rather than threading
    // `hud_colors` through just for that.
    text.rasterize(REFERENCE, size_px, false, [255, 255, 255, 255])
        .map(|p| p.width() as f32 / REFERENCE.chars().count() as f32)
        .unwrap_or(size_px * 0.6) // a typical monospace aspect ratio, if rasterizing ever somehow draws nothing
}

/// Alpha-composites `src` onto `dst` at `(x, y)`, "over" blending, clipped to `dst`'s own
/// bounds. Both are premultiplied-alpha RGBA — tiny-skia's native pixmap format, and what
/// `TextRenderer::rasterize` already produces — so this is just `dst = src + dst * (1 -
/// src.a)` per channel, no premultiply/unpremultiply conversion needed either direction.
fn blit_over(dst: &mut tiny_skia::Pixmap, src: &tiny_skia::Pixmap, x: i32, y: i32) {
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    let (sw, sh) = (src.width(), src.height());
    let dst_stride = dw as usize * 4;
    let src_stride = sw as usize * 4;
    let src_data = src.data();
    let dst_data = dst.data_mut();
    for sy in 0..sh {
        let dy = y + sy as i32;
        if dy < 0 || dy >= dh {
            continue;
        }
        for sx in 0..sw {
            let dx = x + sx as i32;
            if dx < 0 || dx >= dw {
                continue;
            }
            let si = sy as usize * src_stride + sx as usize * 4;
            let di = dy as usize * dst_stride + dx as usize * 4;
            let inv_a = 255 - src_data[si + 3] as u32;
            for c in 0..4 {
                let s = src_data[si + c] as u32;
                let d = dst_data[di + c] as u32;
                dst_data[di + c] = (s + (d * inv_a) / 255) as u8;
            }
        }
    }
}

const CREDITS_PADDING: f32 = 28.0;
const CREDITS_BODY_SIZE: f32 = 12.0;
const CREDITS_LINE_GAP: f32 = 5.0;

fn credits_text_color(color: hakai_core::credits::TextColor, hud_colors: &theme::HudColors) -> [u8; 4] {
    match color {
        hakai_core::credits::TextColor::White => hud_rgba_arr(hud_colors.foreground, 255),
        hakai_core::credits::TextColor::Dim => hud_rgba_arr(hud_colors.foreground, 158), // 62%
        hakai_core::credits::TextColor::Accent => hud_rgba_arr(hud_colors.accent, 255),
    }
}

/// Builds the whole credits panel as one pixmap, plus its own size in pixels (needed to
/// place it — a HUD label's own generalization, see `HudGpu`'s doc comment).
fn build_credits_pixmap(text: &mut TextRenderer, screen_width: f32, hud_colors: &theme::HudColors) -> tiny_skia::Pixmap {
    let char_width = measure_char_width(text, CREDITS_BODY_SIZE);
    // `min(960, screenSize.width - 120)` — `CreditsPanel.swift`'s own `maxWidth`.
    let max_width = 960.0_f32.min(screen_width - 120.0);
    let columns = (((max_width - CREDITS_PADDING * 2.0) / char_width).floor() as i64).max(40) as usize;

    let lines = hakai_core::credits::build(columns);

    let width = columns as f32 * char_width + CREDITS_PADDING * 2.0;
    let mut height = CREDITS_PADDING * 2.0;
    for line in &lines {
        height += line.gap_before + line.size + CREDITS_LINE_GAP;
    }

    let mut pixmap = build_panel_pixmap(
        width.ceil().max(1.0) as u32,
        height.ceil().max(1.0) as u32,
        hud_rgba(hud_colors.background, 224), // 88% — `panel.fillColor`
        Some((hud_rgba(hud_colors.foreground, 71), 1.0)), // 28% — `panel.strokeColor`
    );

    // Top-down, matching `CreditsPanel.swift`'s own `var y = frame.maxY - padding; y -=
    // gapBefore + size; ...; y -= lineGap` — just run the other way, since that's y-up
    // descending from the panel's top edge and this pixmap's own local space is already
    // y-down from that same top edge.
    let mut y = CREDITS_PADDING;
    for line in &lines {
        y += line.gap_before + line.size;
        if !line.text.is_empty() {
            let color = credits_text_color(line.color, hud_colors);
            if let Some(glyph_pixmap) = text.rasterize(&line.text, line.size, false, color) {
                // `verticalAlignmentMode = .baseline` in Swift, with `position.y = y` —
                // approximated here (this crate's rasterized pixmaps carry no baseline
                // metric of their own, by design — see `TextRenderer::rasterize`'s doc
                // comment) by treating `y` as roughly the line's own bottom edge.
                blit_over(&mut pixmap, &glyph_pixmap, CREDITS_PADDING as i32, (y - line.size) as i32);
            }
        }
        y += CREDITS_LINE_GAP;
    }

    pixmap
}

/// Which cached `ToolIcons` variant is on screen for a given tool/press state — matches
/// the keys `ToolIcons` itself caches under internally, not that it matters here (this
/// crate only uses these as `icon_gpu` HashMap keys).
fn icon_variant_key(active: ToolId, is_down: bool) -> &'static str {
    match active {
        ToolId::Hammer => "hammer",
        ToolId::ChainSaw => if is_down { "saw_cut" } else { "saw_idle" },
        ToolId::MachineGun => "machinegun",
        ToolId::FlameThrower => "flamethrower",
        ToolId::ColorThrower => "colorthrower",
        ToolId::Phaser => "phaser",
        ToolId::Stamp => if is_down { "stamp_down" } else { "stamp_up" },
        ToolId::Termites => "termites",
        ToolId::Washer => "washer",
    }
}

/// This tool's hotspot, pivot and recommended square display size in points — all
/// normalised 0..1, already y-down (see `icons.rs`). `ToolIcons`'s accessors cache
/// internally, so calling this every frame is cheap after the first call for each variant.
///
/// The pivot defaults to the hotspot when an icon doesn't report its own (every tool but
/// the hammer — see `icons.rs`'s `ToolIcon::pivot` doc comment) — matching
/// `Hammer.swift`'s own `icon.pivot ?? icon.hotspot`, and what makes it safe for the
/// renderer to always place the cursor by pivot+rotation (see the "Cursor icons" module
/// section) rather than branching on whether this particular icon has a real pivot: with
/// pivot == hotspot and rotation == 0, that placement reduces to exactly "put the hotspot
/// on the mouse," the same as every non-hammer tool always wanted.
fn icon_metadata(icons: &mut ToolIcons, active: ToolId, is_down: bool) -> ((f32, f32), (f32, f32), (f32, f32)) {
    let icon = match active {
        ToolId::Hammer => icons.hammer(),
        ToolId::ChainSaw => icons.chain_saw(is_down),
        ToolId::MachineGun => icons.machine_gun(),
        ToolId::FlameThrower => icons.flame_thrower(),
        ToolId::ColorThrower => icons.color_thrower(),
        ToolId::Phaser => icons.phaser(),
        ToolId::Stamp => icons.stamp(is_down),
        ToolId::Termites => icons.termite_hand(),
        ToolId::Washer => icons.washer(),
    };
    (icon.hotspot, icon.pivot.unwrap_or(icon.hotspot), icon.point_size)
}

/// The active tool's own cursor animation angle, if it has one — the hammer's knock, the
/// machine gun's recoil kick, the chain-saw's vibration. Every other tool has none, so
/// `0.0` (no rotation) is also the fallback if a downcast ever unexpectedly fails.
fn active_cursor_rotation(active: ToolId, tools: &HashMap<ToolId, Box<dyn Tool>>) -> f32 {
    match active {
        ToolId::Hammer => tools.get(&ToolId::Hammer).and_then(|t| t.as_any().downcast_ref::<Hammer>()).map(|h| h.cursor_rotation()).unwrap_or(0.0),
        ToolId::MachineGun => tools.get(&ToolId::MachineGun).and_then(|t| t.as_any().downcast_ref::<MachineGun>()).map(|g| g.cursor_rotation()).unwrap_or(0.0),
        ToolId::ChainSaw => tools.get(&ToolId::ChainSaw).and_then(|t| t.as_any().downcast_ref::<ChainSaw>()).map(|s| s.cursor_rotation()).unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Both the axis-aligned tile/icon pipeline (`vs_main`/`fs_main`) and the rotatable-sprite
/// pipeline (`vs_sprite`/`fs_sprite`) live in the same `shader.wgsl` module and are built
/// here together, sharing one shader-module compile rather than two.
struct Pipelines {
    tile: wgpu::RenderPipeline,
    tile_bind_group_layout: wgpu::BindGroupLayout,
    sprite: wgpu::RenderPipeline,
    sprite_bind_group_layout: wgpu::BindGroupLayout,
    /// Same `vs_sprite`/`fs_sprite` shader and bind-group layout as `sprite`, but with an
    /// additive blend state instead of `sprite`'s regular alpha blend — for standing
    /// flames only. Matches `FlameThrower.swift`'s `flame.blendMode = .add` ("fire glows,
    /// it does not cover"): two overlapping flames should brighten each other rather than
    /// one occluding the other the way a termite or a shell should.
    sprite_additive: wgpu::RenderPipeline,
}

fn uniform_texture_sampler_layout(device: &wgpu::Device, label: &str) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                // VERTEX | FRAGMENT, not just VERTEX: `fs_sprite` reads `sprite.alpha`
                // out of this same uniform (for a standing flame's fade-out) — `fs_main`
                // doesn't touch `tile`'s copy of this binding, but granting the fragment
                // stage access here too is harmless for it, and this layout helper is
                // shared between both the tile and the sprite bind-group layouts.
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
        ],
    })
}

fn create_pipelines(device: &wgpu::Device, format: wgpu::TextureFormat) -> Pipelines {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("hakai-shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
    });

    // Same bind-group *shape* (uniform, texture, sampler) for both, but kept as two
    // distinct layout objects — the uniform struct's actual size differs (`Tile` is 24
    // bytes, `RotatedSprite` is 32), and each pipeline validates against its own layout.
    let tile_bind_group_layout = uniform_texture_sampler_layout(device, "tile-bind-group-layout");
    let sprite_bind_group_layout = uniform_texture_sampler_layout(device, "sprite-bind-group-layout");

    // At `wgpu = "22"`, `entry_point` is a bare `&str`, not `Option<&str>` — confirmed by
    // the compiler, not a guess (a later wgpu did move it to `Option`; that rename just
    // hasn't landed at this pinned version).
    let tile_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("tile-pipeline-layout"),
        bind_group_layouts: &[&tile_bind_group_layout],
        push_constant_ranges: &[],
    });
    let tile = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("tile-pipeline"),
        layout: Some(&tile_pipeline_layout),
        vertex: wgpu::VertexState { module: &shader, entry_point: "vs_main", buffers: &[], compilation_options: wgpu::PipelineCompilationOptions::default() },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });

    let sprite_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sprite-pipeline-layout"),
        bind_group_layouts: &[&sprite_bind_group_layout],
        push_constant_ranges: &[],
    });
    let sprite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sprite-pipeline"),
        layout: Some(&sprite_pipeline_layout),
        vertex: wgpu::VertexState { module: &shader, entry_point: "vs_sprite", buffers: &[], compilation_options: wgpu::PipelineCompilationOptions::default() },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_sprite",
            targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });

    // The additive variant reuses `sprite`'s pipeline layout and shader entry points —
    // only the blend state differs. `SrcAlpha`/`One` on the color channel is the standard
    // "glow" additive blend (a fully-opaque source pixel adds its full color on top of
    // whatever's already there rather than replacing it); `One`/`One` on alpha just
    // accumulates coverage, which nothing here reads back since this is the last write to
    // this pass's only color attachment.
    let additive_blend = wgpu::BlendState {
        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
    };
    let sprite_additive = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sprite-additive-pipeline"),
        layout: Some(&sprite_pipeline_layout),
        vertex: wgpu::VertexState { module: &shader, entry_point: "vs_sprite", buffers: &[], compilation_options: wgpu::PipelineCompilationOptions::default() },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_sprite",
            targets: &[Some(wgpu::ColorTargetState { format, blend: Some(additive_blend), write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });

    Pipelines { tile, tile_bind_group_layout, sprite, sprite_bind_group_layout, sprite_additive }
}

/// Matches `shader.wgsl`'s `RotatedSprite` struct: four `vec2<f32>` corners (32 bytes)
/// plus an `alpha` multiplier. `_pad` isn't read anywhere — it exists only so this
/// struct's Rust byte layout (repr(C), natural size 36) can't come out smaller than
/// what WGSL's own uniform-address-space layout rules compute for `RotatedSprite`
/// (alignment 8, from its `vec2<f32>` members, rounds the struct's size *up* to 40) —
/// a buffer backed by too few bytes for what the shader expects to read is UB, so this
/// pads out to the same 40 defensively rather than relying on the two languages'
/// independent rounding rules to agree by coincidence.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RotatedSpriteUniform {
    top_left: [f32; 2],
    top_right: [f32; 2],
    bottom_left: [f32; 2],
    bottom_right: [f32; 2],
    alpha: f32,
    _pad: f32,
}

/// A rotated sprite's four corners, computed directly in clip space, plus an opacity
/// multiplier passed straight through to `RotatedSpriteUniform::alpha` — see the comment
/// there. Rotation happens here in *pixel* space (x and y are the same unit — one pixel is
/// one pixel either way), not in NDC, where they generally aren't (a screen's width and
/// height differ) — doing it here means the shader never has to know or correct for the
/// screen's aspect ratio. See the comment above `RotatedSprite` in `shader.wgsl`.
fn rotated_sprite_ndc(center_px: (f32, f32), size_px: (f32, f32), rotation: f32, screen_px: (f32, f32), alpha: f32) -> RotatedSpriteUniform {
    let (hw, hh) = (size_px.0 / 2.0, size_px.1 / 2.0);
    let (cos_r, sin_r) = (rotation.cos(), rotation.sin());
    let to_ndc = |local: (f32, f32)| -> [f32; 2] {
        let rotated = (local.0 * cos_r - local.1 * sin_r, local.0 * sin_r + local.1 * cos_r);
        let world_px = (center_px.0 + rotated.0, center_px.1 + rotated.1);
        [-1.0 + 2.0 * world_px.0 / screen_px.0, 1.0 - 2.0 * world_px.1 / screen_px.1]
    };
    RotatedSpriteUniform {
        top_left: to_ndc((-hw, -hh)),
        top_right: to_ndc((hw, -hh)),
        bottom_left: to_ndc((-hw, hh)),
        bottom_right: to_ndc((hw, hh)),
        alpha,
        _pad: 0.0,
    }
}

/// Like `rotated_sprite_ndc`, but rotates around an arbitrary point *within* the sprite
/// (`pivot_local`, in pixels measured from the sprite's own top-left) instead of its
/// centre. `rotated_sprite_ndc` is really just this with `pivot_local` fixed at
/// `size_px / 2` — kept as its own simpler function since every caller except the cursor
/// only ever wants that case.
///
/// `pivot_world` is where `pivot_local` itself should land on screen — for the cursor,
/// that's `Hammer.swift`'s own derivation: `mouse + (pivot - hotspot) * point_size` (see
/// the call site), which is what makes the *hotspot*, not the pivot, land exactly on the
/// mouse at zero rotation.
fn rotated_sprite_ndc_pivot(pivot_world: (f32, f32), pivot_local: (f32, f32), size_px: (f32, f32), rotation: f32, screen_px: (f32, f32), alpha: f32) -> RotatedSpriteUniform {
    let (cos_r, sin_r) = (rotation.cos(), rotation.sin());
    let to_ndc = |corner_local: (f32, f32)| -> [f32; 2] {
        let rel = (corner_local.0 - pivot_local.0, corner_local.1 - pivot_local.1);
        let rotated = (rel.0 * cos_r - rel.1 * sin_r, rel.0 * sin_r + rel.1 * cos_r);
        let world_px = (pivot_world.0 + rotated.0, pivot_world.1 + rotated.1);
        [-1.0 + 2.0 * world_px.0 / screen_px.0, 1.0 - 2.0 * world_px.1 / screen_px.1]
    };
    RotatedSpriteUniform {
        top_left: to_ndc((0.0, 0.0)),
        top_right: to_ndc((size_px.0, 0.0)),
        bottom_left: to_ndc((0.0, size_px.1)),
        bottom_right: to_ndc((size_px.0, size_px.1)),
        alpha,
        _pad: 0.0,
    }
}

/// Builds a fresh uniform buffer + bind group for one rotated-sprite draw and issues it —
/// the "fresh buffer/bind group per instance" pattern termites established, factored out
/// once particles and flames started needing it too. See the RISK note where this is
/// called from `State::render` for why creating-and-dropping these within a single
/// render-pass command list is trusted to be safe.
fn draw_rotated_sprite(
    device: &wgpu::Device,
    pass: &mut wgpu::RenderPass<'_>,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    texture_view: &wgpu::TextureView,
    ndc: &RotatedSpriteUniform,
    label: &str,
) {
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::bytes_of(ndc),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(texture_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..6, 0..1);
}

// ── Per-output layer surface + its wgpu surface ─────────────────────────────────────────

struct GpuLayer {
    layer: LayerSurface,
    wgpu_surface: wgpu::Surface<'static>,
    /// This output's surface size in *points* (logical units) — matching mouse
    /// coordinates, `hakai_core::damage::DamageLayer`, and every NDC placement call in
    /// this file, unchanged since Phase 0. NOT the `wgpu` surface's own pixel buffer
    /// size, which on a fractionally-scaled output is larger — see the "Fractional
    /// scale" module section for why keeping this in points, and computing the real
    /// pixel size only locally where `wgpu::SurfaceConfiguration` needs it, is correct.
    width: u32,
    height: u32,
    configured: bool,
    damage: Option<DamageLayer>,
    tiles: Vec<TileGpu>,

    // Per-output game state — see the module doc comment for why this isn't shared.
    // `audio` is deliberately *not* here, unlike particles/termites/rng — it lives on
    // `State` instead, shared across every output. Matches `AudioEngine.swift`, which is
    // one instance the app hands to every `GameScene`, not one per screen: a real audio
    // device is physically singular (two independent per-output engines would mean two
    // overlapping output streams fighting over the same speakers), and Swift's own "only
    // the first scene drives its update loop" comment only makes sense if the loop state
    // being updated is shared in the first place.
    particles: ParticleSystem,
    termites: TermiteColony,
    rng: SeededRng,
    tools: HashMap<ToolId, Box<dyn Tool>>,
    active_tool: ToolId,
    mouse: (f32, f32),
    is_down: bool,
    last_frame_time: Option<Instant>,

    /// Per-output HUD visibility/fade state — see `hakai_core::hud`'s module doc comment
    /// for why this is pure logic, separate from any of its actual rendering below. Each
    /// output gets its own, same reasoning as `particles`/`termites`: a toast triggered by
    /// (say) `eraseAll` fires on every scene independently in the Swift original too
    /// (`AppDelegate.swift`'s `onEraseAll` loops over every scene).
    hud: Hud,
    /// The toast's own GPU texture, rebuilt only when `hud.toast()`'s text changes —
    /// `None` whenever there's no toast live at all. A plain texture, not a `HudGpu`: the
    /// toast draws via the rotated-sprite pipeline for its alpha fade (see
    /// `draw_rotated_sprite`), which already builds a fresh buffer/bind group every frame,
    /// so there's no persistent bind group worth keeping here.
    toast_gpu: Option<ToastGpu>,

    /// This output's own `wl_output` — needed to request a capture of it
    /// (`zwlr_screencopy_manager_v1::capture_output`). Not used for anything else; every
    /// other per-output identity check in this file goes through the layer's `wl_surface`
    /// instead.
    output: wl_output::WlOutput,
    /// This output's most recent brightness sample grid — see the "Screen capture" module
    /// section. Starts empty (`sample` always returns `None` then), same as this port's
    /// existing "no screen-capture permission" fallback since Phase 3.
    brightness: capture::BrightnessMap,
    /// Seconds since the last *completed* capture (successful or failed) — ticks in
    /// `State::advance` regardless of what `capture_frame` is doing, so a request that
    /// never resolves can't wedge captures from ever being retried.
    since_last_capture: f32,
    /// `Some` exactly while a `zwlr_screencopy_v1` request/event dance is in flight for
    /// this output — `None` means idle. The three capture fields together are this
    /// port's per-output capture *state*, inferred from which are `Some`/`None` rather
    /// than a separate enum (see `capture.rs`'s own doc comment for why that lives there
    /// and not here).
    capture_frame: Option<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1>,
    /// `(format, width, height, stride)` from the frame's own `buffer` event — set once
    /// `buffer_done` fires, from whichever `buffer` event was seen (last write wins; see
    /// the `Buffer` event's own handling for why that's an accepted simplification here).
    capture_buffer_info: Option<(wl_shm::Format, u32, u32, u32)>,
    /// The SHM buffer the compositor is (or just finished) writing the capture into.
    capture_buffer: Option<smithay_client_toolkit::shm::slot::Buffer>,

    /// `DisplayMode.frozen` in the Swift original — `M` toggles this on every output at
    /// once (see `State::toggle_mode`). While frozen, this output draws `snapshot_texture`
    /// as its own opaque background instead of leaving the surface transparent, so the
    /// real desktop underneath can keep changing (other windows redrawing, moving)
    /// without disturbing what the user sees themselves smashing.
    frozen: bool,
    /// The most recent captured frame, converted for display (`capture::to_rgba`) — built
    /// only while `frozen` (a full-resolution upload every `CAPTURE_INTERVAL` would be
    /// wasted bandwidth in live mode, where nothing ever shows it). `None` until the
    /// first capture completes after entering frozen mode.
    snapshot_texture: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,

    /// The compositor's preferred scale for this output (`preferred_scale / 120.0`) —
    /// `1.0` until a `wp_fractional_scale_v1` event says otherwise, which is also the
    /// permanent value on an integer-scaled output or when the protocol isn't available
    /// at all. `width`/`height` above stay in points regardless of this value — see the
    /// "Fractional scale" module section for why only the wgpu surface's own pixel buffer
    /// size needs to multiply by it.
    scale: f32,
    /// `None` if either global wasn't available at startup (see `State`'s own fields) —
    /// this output just never rescales its buffer for a fractional factor in that case,
    /// same as before this chunk existed.
    fractional_scale: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    viewport: Option<wp_viewport::WpViewport>,
}

struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    layer_shell: LayerShell,
    seat_state: SeatState,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// Updated by `update_modifiers`, read by `press_key`'s `Tab` handler — Wayland
    /// delivers modifier state as its own separate event, not bundled into each key press,
    /// so this is the only way to know "is shift down" at the moment Tab actually fires.
    /// `InputRouter.swift`'s own `case 48: onCycleTool?(event.modifierFlags.contains(.shift)
    /// ? -1 : 1)`.
    shift_held: bool,
    pointer: Option<wl_pointer::WlPointer>,
    /// Which output's surface the pointer is currently over — Wayland gives pointer focus
    /// to one surface at a time, unlike the exclusive keyboard grab every surface holds.
    pointer_focus: Option<wl_surface::WlSurface>,

    instance: wgpu::Instance,
    adapter: Option<wgpu::Adapter>,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    pipelines: Option<Pipelines>,
    sampler: Option<wgpu::Sampler>,
    /// Shared across outputs — decals are generated once and cached, same as the real app
    /// will do (see `AppDelegate.swift`'s shared `DecalFactory`).
    decals: DecalFactory,
    /// The active Omarchy theme's HUD chrome colours (panel backgrounds, text/borders, the
    /// selected-palette-cell/credits accent) — `theme::HudColors::FALLBACK` until `main`
    /// overwrites it, same "read once at startup" timing as `decals`' paint palette and
    /// for the same reason: every HUD/palette/credits pixmap below is built once and
    /// cached, so this has to be settled before any of them are.
    hud_colors: theme::HudColors,
    /// Also shared, unlike `GpuLayer`'s other per-output game state — see the comment on
    /// `GpuLayer` itself for why audio in particular has to be.
    audio: AudioSink,
    /// Also shared — cursor icons don't depend on anything per-output either.
    icons: ToolIcons,
    /// One GPU texture + bind group per icon variant (11: 9 tools, plus the chain-saw's
    /// cutting and the stamp's pressed variants) — created once, alongside the pipeline.
    icon_gpu: HashMap<&'static str, IconGpu>,
    /// The one dynamic uniform buffer every icon variant's bind group shares — see the
    /// "Cursor icons" module section for why reusing it across frames is safe.
    cursor_uniform_buffer: Option<wgpu::Buffer>,
    /// Shared, like `decals`/`icons`.
    sprites: SpriteFactory,
    /// The two termite walk-frame textures — a termite's own position/heading/frame
    /// changes constantly (up to 500 of them), so unlike an icon it gets a *fresh*
    /// per-termite, per-frame uniform buffer and bind group rather than sharing one
    /// dynamic buffer the way the single cursor does — see the "Termites" module section.
    termite_textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// A machine-gun shell — one texture, reused (fresh buffer/bind group) for every live
    /// shell particle, the same way termites are.
    shell_texture: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// One texture per `self.decals.paint_colors()` entry, indexed by a droplet particle's
    /// own `color_index` — built from whatever the active Omarchy theme (or the built-in
    /// default) resolved to at startup, see `theme.rs`.
    droplet_textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// The four standing-flame walk-frame textures — `SpriteFactory::FLAME_FRAMES` many,
    /// cycled per-flame by its own age (see `FlameView::age`), matching the Swift
    /// original's `timePerFrame: 0.07` looping action.
    flame_textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// One texture per `DecalFactory::SLIVER_VARIANTS` entry — the hammer's slivers and
    /// the chain-saw's sawdust both ride `ParticleKind::Generic { variant }`, indexed into
    /// this the same way `droplet_textures` is indexed by a droplet's `color_index`.
    sliver_textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// The machine gun's muzzle flash — one texture, reused (fresh buffer/bind group,
    /// additive blend) for every live `MachineGun::FlashView`.
    flash_texture: Option<(wgpu::Texture, wgpu::TextureView)>,

    /// Shared — glyph shaping/rasterization has no per-output state, same as `icons`.
    text: TextRenderer,
    /// The HUD bar's backing panel — built once (its pixels never change; only its
    /// placement, per output, at draw time). `None` until the first `configure()`.
    hud_panel: Option<HudGpu>,
    /// The hint text (`"1–9 tool · ...`) — also built once; it never changes at runtime.
    hud_hint: Option<HudGpu>,
    /// The tool-name label (`"1 · Hammer"`) — rebuilt only when the active tool changes.
    /// Shared across outputs, unlike the toast: `select_tool` keeps every output's
    /// `active_tool` in lockstep already (see its own doc comment), so there's never a
    /// frame where two outputs would need to show different text here.
    hud_label: Option<HudGpu>,

    /// The tool palette's backing panel — built once, sized to fit exactly
    /// `ToolId::ALL.len()` cells (see `palette_total_width`). `(texture, view, width_px,
    /// height_px)`, not a `HudGpu`: the whole palette fades via `hud.palette_alpha()`, so
    /// every element in it draws through `draw_rotated_sprite` (alpha-capable, and already
    /// builds a fresh buffer/bind group per draw) instead — see the "Tool palette" module
    /// section.
    palette_panel: Option<(wgpu::Texture, wgpu::TextureView, f32, f32)>,
    /// A cell's background in its two states — reused across all nine cells; which one a
    /// given cell draws is decided at draw time, not baked into nine separate textures.
    palette_cell_normal: Option<(wgpu::Texture, wgpu::TextureView)>,
    palette_cell_selected: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// One icon texture per tool — built once from the same `ToolIcons` pixmaps the cursor
    /// already uses, but as their own textures rather than reusing the cursor's `IconGpu`
    /// (which doesn't expose its view, only a bind group already wired to the cursor's own
    /// dynamic uniform buffer).
    palette_icons: HashMap<ToolId, (wgpu::Texture, wgpu::TextureView)>,
    /// One digit texture ("1".."9") per tool, plus its own pixel size (text, so not fixed
    /// ahead of time — see `HudGpu`'s own doc comment for why).
    palette_digits: HashMap<ToolId, (wgpu::Texture, wgpu::TextureView, f32, f32)>,

    /// The whole acknowledgements panel, composited into one texture — see the "Credits
    /// panel" module section. `(texture, view, width_px, height_px)`, built once from the
    /// first output's width.
    credits_panel: Option<(wgpu::Texture, wgpu::TextureView, f32, f32)>,

    /// `None` when `zwlr_screencopy_manager_v1` isn't advertised at all — a real
    /// possibility on a non-wlroots compositor — in which case every capture attempt is
    /// just skipped and every tool falls back to a random impact-sound variant, same as
    /// this port's behaviour has been since Phase 3.
    screencopy_manager: Option<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1>,
    /// Also `None`-able for the same reason (`wl_shm` is near-universal, but not
    /// guaranteed).
    shm: Option<Shm>,
    /// One shared pool for every output's capture buffers — sized once, generously, at
    /// startup (see where it's created) rather than grown on demand.
    shm_pool: Option<SlotPool>,

    /// `None` if `wp_fractional_scale_v1` isn't advertised — every output then just stays
    /// at `scale: 1.0` (already correct for any integer-scaled output, which is the more
    /// common case) rather than ever getting per-output fractional-scale objects at all.
    fractional_scale_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    /// Also `None`-able — `wp_viewporter` is stable and near-universal on wlroots
    /// compositors, but not guaranteed either.
    viewporter: Option<wp_viewporter::WpViewporter>,

    layers: Vec<GpuLayer>,
    exit: bool,
    qh: QueueHandle<Self>,
    conn: Connection,
}

impl State {
    fn spawn_layer_for_output(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let surface = self.compositor_state.create_surface(qh);

        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("hakai"),
            Some(&output),
        );

        // The whole point of the exercise: cover the entire output, sit above
        // everything (including bars), and take the keyboard exclusively so Esc is
        // guaranteed to reach us regardless of what else is running.
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        // Real size arrives in the first `configure` event below; a size of (0, 0) here
        // tells the compositor "you decide," which is what we want for a full-output layer.
        layer.set_size(0, 0);
        layer.commit();

        // Opts this output into fractional-scale awareness, if the protocol's available
        // — see the "Fractional scale" module section. `None`/`None` (either global
        // missing) just means `configure()` never sees a `PreferredScale` event, so
        // `scale` stays `1.0` for this output permanently.
        let fractional_scale = self
            .fractional_scale_manager
            .as_ref()
            .map(|manager| manager.get_fractional_scale(layer.wl_surface(), qh, ()));
        let viewport = self.viewporter.as_ref().map(|viewporter| viewporter.get_viewport(layer.wl_surface(), qh, ()));

        // ── RISK: raw Wayland pointers for wgpu's raw-window-handle bridge ──────────────
        // wgpu wants something implementing HasWindowHandle + HasDisplayHandle. Wayland's
        // side of that is just the wl_display and wl_surface pointers as libwayland-client
        // sees them — which is exactly what `Connection::backend().display_ptr()` and
        // `Proxy::id().as_ptr()` hand back. If this section doesn't compile against your
        // installed `wayland-backend`, that's the one to fix first — see PHASE0.md.
        let display_ptr = self.conn.backend().display_ptr();
        let surface_ptr = layer.wl_surface().id().as_ptr();

        let raw = RawWaylandHandle {
            display: display_ptr as *mut _,
            surface: surface_ptr as *mut _,
        };

        let wgpu_surface = unsafe {
            std::mem::transmute::<wgpu::Surface<'_>, wgpu::Surface<'static>>(
                self.instance
                    .create_surface(raw)
                    .expect("failed to create a wgpu surface from the layer's wl_surface"),
            )
        };

        if self.adapter.is_none() {
            let adapter = pollster::block_on(self.instance.request_adapter(
                &wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: Some(&wgpu_surface),
                    force_fallback_adapter: false,
                },
            ))
            .expect("no wgpu adapter compatible with the layer surface");

            // `Limits::downlevel_defaults()` is the WebGL2-safe profile — capped at a
            // 2048×2048 max texture size, since that's what has to work on the lowest
            // common denominator GPU a browser might hand WebGL. This is a native Vulkan
            // app with no such constraint, and that cap is real: a wgpu surface's own
            // pixel buffer is one giant texture, so any output whose buffer size (points
            // × scale) exceeds 2048 in either dimension — an ordinary 1440p/4K monitor at
            // 1x, or a more modest one at a >1x fractional/integer scale — hit
            // `Surface::configure`'s validation error outright. `adapter.limits()` asks
            // for whatever this specific GPU actually supports instead (real Vulkan
            // drivers report 8192 or 16384), which is exactly as safe to request as the
            // downlevel profile was (it's what `request_adapter` already told us this
            // adapter can do) and removes the cap entirely on any real desktop GPU.
            let (device, queue) = pollster::block_on(adapter.request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("hakai"),
                    required_features: wgpu::Features::empty(),
                    required_limits: adapter.limits(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            ))
            .expect("failed to acquire a wgpu device");

            self.adapter = Some(adapter);
            self.device = Some(device);
            self.queue = Some(queue);
        }

        // A per-output seed. Not cryptographic-grade randomness (no `rand` dependency
        // pulled in just for this) — nanosecond-resolution wall clock time is entropy
        // enough for "the crack pattern looks different each run," which is all this
        // needs.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1)
            .wrapping_add(self.layers.len() as u64 + 1);

        self.layers.push(GpuLayer {
            layer,
            wgpu_surface,
            width: 0,
            height: 0,
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
            output,
            brightness: capture::BrightnessMap::default(),
            // Already due — so the first capture fires on this output's very first
            // `advance()` call rather than waiting a full `CAPTURE_INTERVAL` after
            // startup for the first real brightness sample.
            since_last_capture: capture::CAPTURE_INTERVAL,
            capture_frame: None,
            capture_buffer_info: None,
            capture_buffer: None,
            frozen: false,
            snapshot_texture: None,
            scale: 1.0,
            fractional_scale,
            viewport,
        });
    }

    // MARK: - Tool switching and global commands

    /// Broadcasts a tool switch to every output — matches Swift's `select(_:)` being
    /// called on every scene. Each output still owns its own instance of the *target*
    /// tool (with its own state/timers); only which one is active changes here.
    fn select_tool(&mut self, id: ToolId) {
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
                    // ^ `gpu` here is the same `&mut GpuLayer` `damage`/`particles`/etc.
                    // above already borrow from — calling `.sample(gpu.mouse)` reads two
                    // of its fields (`brightness`, `mouse`) without needing a whole
                    // separate `&gpu`, so this doesn't add a new borrow-checker wrinkle.
                };
                tool.deactivate(&mut ctx);
            }
            gpu.active_tool = id;
        }

        // `GameScene.swift`'s `toolLabel.text = "\(id.keyDigit) · \(id.displayName)"`.
        // `ensure_hud_text` no-ops on its own if this exact string is already what's
        // cached, so it's safe (and simplest) to just call this unconditionally rather
        // than tracking "did the tool actually change" separately.
        if let (Some(device), Some(queue), Some(pipelines), Some(sampler)) = (&self.device, &self.queue, &self.pipelines, &self.sampler) {
            let text = format!("{} \u{b7} {}", id.key_digit(), id.display_name());
            ensure_hud_text(device, queue, &pipelines.tile_bind_group_layout, sampler, &mut self.text, &mut self.hud_label, &text, HUD_LABEL_SIZE, false, hud_rgba_arr(self.hud_colors.foreground, 255));
        }
        for gpu in &mut self.layers {
            gpu.hud.flash_palette();
        }
    }

    fn cycle_tool(&mut self, direction: i32) {
        // Every output tracks the same active tool in lockstep (see `select_tool`), so
        // the first one is as good a reference point as any.
        let Some(current) = self.layers.first().map(|g| g.active_tool) else { return };
        let idx = ToolId::ALL.iter().position(|&t| t == current).unwrap_or(0) as i32;
        let next = (idx + direction).rem_euclid(ToolId::ALL.len() as i32) as usize;
        self.select_tool(ToolId::ALL[next]);
    }

    fn erase_all(&mut self) {
        for gpu in &mut self.layers {
            if let Some(damage) = gpu.damage.as_mut() {
                damage.erase_all();
            }
            gpu.hud.show_toast("Desktop cleaned");
        }
    }

    /// `AppDelegate.swift`'s `input.onTogglePalette = { visible in forEachScene { ... } }`.
    fn set_palette_visible(&mut self, visible: bool) {
        for gpu in &mut self.layers {
            gpu.hud.set_palette_visible(visible);
        }
    }

    /// `AppDelegate.swift`'s `input.onToggleCredits = { forEachScene { $0.toggleCredits() } }`.
    fn toggle_credits(&mut self) {
        for gpu in &mut self.layers {
            gpu.hud.toggle_credits();
        }
    }

    /// `M` — `AppDelegate.swift`'s `toggleMode()`. Freezes/unfreezes every output at once.
    /// Entering frozen mode marks a capture as immediately due (rather than waiting up to
    /// `CAPTURE_INTERVAL` for the next scheduled one — `maybe_start_capture` picks this up
    /// on the very next frame), so there's something to show right away instead of a
    /// transparent gap; leaving it frees the now-unused snapshot texture and goes back to
    /// true transparency.
    fn toggle_mode(&mut self) {
        for gpu in &mut self.layers {
            gpu.frozen = !gpu.frozen;
            if gpu.frozen {
                gpu.since_last_capture = capture::CAPTURE_INTERVAL;
            } else {
                gpu.snapshot_texture = None;
            }
        }
    }

    /// Drives this output's active tool, particles and termites forward by `dt` — the
    /// per-output equivalent of `ToolSimulation.drive()`'s per-step loop, just fed by real
    /// input instead of a synthetic stroke. A plain associated function taking `decals`,
    /// `audio` and `gpu` as separate parameters, not `&mut self` — same reasoning as
    /// `render` below: `gpu` is already a mutable borrow of one of `self.layers`'
    /// elements wherever this is called from, and a method here would conflict with that
    /// even though `self.decals`/`self.audio` and `self.layers` are genuinely disjoint
    /// fields. `audio` is shared (unlike `decals`, it isn't *also* passed here because
    /// it's stateless-and-cached — see the comment on `GpuLayer` for why it has to be),
    /// but nothing here calls `audio.update(dt)`: this runs once per *output*, and that
    /// must run once per frame total — see `AudioBackend::update`'s own doc comment. The
    /// caller is responsible for that, from exactly one output's frame callback.
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

        // Standing flames keep burning down — drifting, spreading, fading, killing
        // termites and leaving a scorch mark when they go out — no matter which tool is
        // active, the same "always updates" treatment `termites`/`particles` get just
        // below. A deliberate departure from the Swift original, where a flame's whole
        // `advanceFlames` step (unlike the termite colony's own update, which has its own
        // explicit "always updates, not only while the termite tool is selected"
        // treatment there too) only ever runs from `FlameThrower.update`, itself only
        // called while the flame-thrower is the active tool — verified by reading
        // `GameScene.swift` directly, not assumed. Freezing a fire mid-burn because the
        // player picked up a different tool reads as a bug to a player even though it's
        // just an oversight in the 2,000-line original this was ported from, so this port
        // fixes it rather than reproducing it. Calling `FlameThrower::update` again here
        // even when it *was* just called above (i.e. it's also the active tool) is
        // guarded by the `active != FlameThrower` check below — safe either way, since
        // `deactivate` always resets `burning` to `false` before this could ever run a
        // second time for an inactive tool, so the emit-a-new-flame branch inside never
        // double-fires — but there's no reason to call it twice in the same frame.
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
        // Actually *starting* the next capture (once this reaches `CAPTURE_INTERVAL`)
        // needs `self.screencopy_manager`/`self.qh`, which this plain associated function
        // doesn't have — see `State::maybe_start_capture`, called right alongside this
        // from `CompositorHandler::frame`.
        gpu.since_last_capture += dt;
    }

    /// Kicks off a fresh `zwlr_screencopy_v1` capture for `self.layers[index]`, if one's
    /// actually due (`since_last_capture >= CAPTURE_INTERVAL`) and nothing's already in
    /// flight for it. A `&mut self` method, unlike `advance` — it needs
    /// `self.screencopy_manager`/`self.qh` alongside `self.layers[index]`, and indexing
    /// (rather than an already-borrowed `&mut GpuLayer`) is what keeps those disjoint-field
    /// borrows straightforward here.
    fn maybe_start_capture(&mut self, index: usize) {
        let Some(manager) = &self.screencopy_manager else { return };
        let gpu = &mut self.layers[index];
        if gpu.capture_frame.is_some() {
            return;
        }
        // Once frozen, the snapshot is meant to stay put — only the *first* capture after
        // entering frozen mode is wanted (to actually have something to show), not a
        // continuing refresh every `CAPTURE_INTERVAL`, which would defeat "frozen." Live
        // mode keeps periodically recapturing for the brightness map, same as before.
        let due = if gpu.frozen { gpu.snapshot_texture.is_none() } else { gpu.since_last_capture >= capture::CAPTURE_INTERVAL };
        if !due {
            return;
        }
        gpu.since_last_capture = 0.0;
        // `overlay_cursor: 0` — don't composite a cursor into the capture (confirmed
        // against `capture_output`'s real signature, not a guess — the parameter's
        // actually named `overlay_cursor`, not the `overwrite_cursor` the protocol XML's
        // own wording might suggest). This
        // overlay hides the real system cursor entirely (`set_cursor(serial, None, 0,
        // 0)`) and draws its own, so there's never a "real" cursor to include anyway.
        let frame = manager.capture_output(0, &gpu.output, &self.qh, ());
        gpu.capture_frame = Some(frame);
    }

    /// A plain associated function, deliberately not `&self` — it's called from places
    /// that are already holding a mutable borrow of one of `self`'s fields (`configure`
    /// holds `self.layers.iter_mut()`), and a `&self` method there would conflict with
    /// that borrow even though the two don't actually overlap.
    #[allow(clippy::too_many_arguments)]
    fn render(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &wgpu::RenderPipeline,
        sprite_pipeline: &wgpu::RenderPipeline,
        sprite_additive_pipeline: &wgpu::RenderPipeline,
        sprite_bind_group_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        icons: &mut ToolIcons,
        icon_gpu: &HashMap<&'static str, IconGpu>,
        cursor_uniform: &wgpu::Buffer,
        termite_textures: &[(wgpu::Texture, wgpu::TextureView)],
        shell_texture: &Option<(wgpu::Texture, wgpu::TextureView)>,
        droplet_textures: &[(wgpu::Texture, wgpu::TextureView)],
        flame_textures: &[(wgpu::Texture, wgpu::TextureView)],
        sliver_textures: &[(wgpu::Texture, wgpu::TextureView)],
        flash_texture: &Option<(wgpu::Texture, wgpu::TextureView)>,
        text: &mut TextRenderer,
        hud_panel: &Option<HudGpu>,
        hud_hint: &Option<HudGpu>,
        hud_label: &Option<HudGpu>,
        palette_panel: &Option<(wgpu::Texture, wgpu::TextureView, f32, f32)>,
        palette_cell_normal: &Option<(wgpu::Texture, wgpu::TextureView)>,
        palette_cell_selected: &Option<(wgpu::Texture, wgpu::TextureView)>,
        palette_icons: &HashMap<ToolId, (wgpu::Texture, wgpu::TextureView)>,
        palette_digits: &HashMap<ToolId, (wgpu::Texture, wgpu::TextureView, f32, f32)>,
        credits_panel: &Option<(wgpu::Texture, wgpu::TextureView, f32, f32)>,
        hud_colors: &theme::HudColors,
        show_cursor: bool,
        gpu: &mut GpuLayer,
    ) {
        upload_dirty_tiles(queue, gpu);

        // The cursor's placement (and, for the hammer, rotation) changes every frame —
        // written into the shared dynamic uniform buffer right before the one draw that
        // reads it this frame. See the "Cursor icons" module section for why that's safe.
        let variant_key = icon_variant_key(gpu.active_tool, gpu.is_down);
        if show_cursor && icon_gpu.contains_key(variant_key) {
            let (hotspot, pivot, point_size) = icon_metadata(icons, gpu.active_tool, gpu.is_down);
            // Every non-animating tool's `pivot == hotspot` (see `icon_metadata`), which
            // makes the `pivot_world`/`pivot_local` math below reduce to exactly the old
            // hotspot-on-the-mouse placement regardless of what `rotation` is, so
            // `active_cursor_rotation` is the only tool-specific branch needed here.
            let rotation = active_cursor_rotation(gpu.active_tool, &gpu.tools);
            let pivot_world = (gpu.mouse.0 + (pivot.0 - hotspot.0) * point_size.0, gpu.mouse.1 + (pivot.1 - hotspot.1) * point_size.1);
            let pivot_local = (pivot.0 * point_size.0, pivot.1 * point_size.1);
            let ndc = rotated_sprite_ndc_pivot(pivot_world, pivot_local, point_size, rotation, (gpu.width as f32, gpu.height as f32), 1.0);
            queue.write_buffer(cursor_uniform, 0, bytemuck::bytes_of(&ndc));
        }

        let Ok(frame) = gpu.wgpu_surface.get_current_texture() else { return };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hakai-frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            // Frozen mode's background — the captured snapshot, drawn first (so
            // everything else layers on top of it, same as the real, live desktop
            // otherwise shows through transparency). Axis-aligned and always fully
            // opaque, but drawn via the rotated-sprite pipeline (rotation `0.0`, alpha
            // `1.0`) rather than adding a dedicated tile-pipeline "draw one fresh texture"
            // helper solely for this one caller — `draw_rotated_sprite` already does
            // exactly this otherwise.
            if let Some((_, view, width, height)) = &gpu.snapshot_texture {
                // `screen_px` isn't in scope yet this early in the function — computed
                // fresh here rather than hoisting its later definition up just for this.
                let screen = (gpu.width as f32, gpu.height as f32);
                pass.set_pipeline(sprite_pipeline);
                let center = (screen.0 / 2.0, screen.1 / 2.0);
                let ndc = rotated_sprite_ndc(center, (*width as f32, *height as f32), 0.0, screen, 1.0);
                draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, view, &ndc, "snapshot");
            }

            pass.set_pipeline(pipeline);
            for tile in &gpu.tiles {
                pass.set_bind_group(0, &tile.bind_group, &[]);
                pass.draw(0..6, 0..1);
            }

            // Shells, paint droplets and slivers/sawdust — live particles, drawn above
            // the damage layer. Same fresh-buffer-per-instance treatment as termites
            // below (see the RISK note on that loop; it applies here too), just driven
            // by `gpu.particles` instead of `gpu.termites`.
            pass.set_pipeline(sprite_pipeline);
            for particle in gpu.particles.iter() {
                let tex_view = match particle.kind {
                    ParticleKind::Shell => shell_texture.as_ref().map(|(_, v)| v),
                    ParticleKind::Droplet { color_index } => {
                        let i = (color_index.rem_euclid(droplet_textures.len().max(1) as i64)) as usize;
                        droplet_textures.get(i).map(|(_, v)| v)
                    }
                    ParticleKind::Generic { variant } => {
                        let i = (variant.rem_euclid(sliver_textures.len().max(1) as i64)) as usize;
                        sliver_textures.get(i).map(|(_, v)| v)
                    }
                };
                let Some(tex_view) = tex_view else { continue };
                let ndc = rotated_sprite_ndc(particle.position, particle.size, particle.rotation, (gpu.width as f32, gpu.height as f32), 1.0);
                draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, tex_view, &ndc, "particle");
            }

            // Standing flames — additive blending (see `Pipelines::sprite_additive`), so
            // switch pipelines rather than reusing `sprite_pipeline` above.
            if let Some(flame_thrower) = gpu.tools.get(&ToolId::FlameThrower).and_then(|t| t.as_any().downcast_ref::<FlameThrower>()) {
                pass.set_pipeline(sprite_additive_pipeline);
                for flame in flame_thrower.flames() {
                    if flame_textures.is_empty() {
                        break;
                    }
                    // `timePerFrame: 0.07` in the Swift original — a plain animation
                    // action there; derived from the flame's own age here. Safe to rely on
                    // `flame.age` always advancing now — see `State::advance`'s own
                    // comment on why `FlameThrower::update` runs every frame regardless of
                    // which tool is active, unlike every other tool.
                    let frame = ((flame.age / 0.07) as i64).rem_euclid(flame_textures.len() as i64) as usize;
                    let Some((_, tex_view)) = flame_textures.get(frame) else { continue };
                    let size_px = (FLAME_BASE_SIZE.0 * flame.scale, FLAME_BASE_SIZE.1 * flame.scale);
                    // See `FLAME_ANCHOR_TO_CENTER_Y`'s doc comment: shift the centre this
                    // quad is built around up from the flame's own (anchor) position.
                    let center = (flame.position.0, flame.position.1 - FLAME_ANCHOR_TO_CENTER_Y * size_px.1);
                    let alpha = if flame.life_fraction > 0.8 { ((1.0 - flame.life_fraction) / 0.2).max(0.0) } else { 1.0 };
                    let ndc = rotated_sprite_ndc(center, size_px, 0.0, (gpu.width as f32, gpu.height as f32), alpha);
                    draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, tex_view, &ndc, "flame");
                }
            }

            // Machine-gun muzzle flashes — also additive (`MachineGun.swift`'s
            // `flash.blendMode = .add`), also tool-owned rather than a `ParticleSystem`
            // particle — see the module doc comment in `machine_gun.rs`.
            if let Some(gun) = gpu.tools.get(&ToolId::MachineGun).and_then(|t| t.as_any().downcast_ref::<MachineGun>()) {
                if let Some((_, tex_view)) = flash_texture {
                    pass.set_pipeline(sprite_additive_pipeline);
                    for flash in gun.flashes() {
                        // 1 → 1.5 scale, 1 → 0 alpha, both over the flash's whole (very
                        // short) life — `MachineGun.swift`'s `.group([.scale(to: 1.5,
                        // duration: 0.09), .fadeOut(withDuration: 0.09)])`.
                        let side = flash.size * (1.0 + 0.5 * flash.life_fraction);
                        let alpha = (1.0 - flash.life_fraction).max(0.0);
                        // `flash.rotation` is a uniformly random full-circle angle with no
                        // particular "up" — unlike `Hammer`/`MachineGun::cursor_rotation`,
                        // `sprites.flash()`'s shape (a radial glow plus a 4-point star, 90°
                        // rotationally symmetric) looks identical either way, so this is
                        // the one Swift-sourced rotation in this port that doesn't need
                        // the y-up→y-down negation — not an oversight, a deliberate no-op.
                        let ndc = rotated_sprite_ndc(flash.position, (side, side), flash.rotation, (gpu.width as f32, gpu.height as f32), alpha);
                        draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, tex_view, &ndc, "flash");
                    }
                }
            }

            // Termites — above the damage layer, below the cursor. See the "Termites"
            // module section for why each one gets a fresh buffer + bind group rather
            // than sharing one dynamic buffer the way the cursor does.
            //
            // RISK: `buffer`/`bind_group` below (inside `draw_rotated_sprite`) are
            // created and go out of scope again within the same loop iteration, before
            // `queue.submit()` at the end of this function actually runs. I'm relying on
            // `wgpu`'s resource handles being internally reference-counted (so
            // `set_bind_group` captures its own handle to the GPU-side resource,
            // independent of the Rust value's lifetime) — the same assumption already
            // relied on implicitly for `TextureView`s outliving the `Texture` they're
            // built from elsewhere in this file. If this doesn't compile, or compiles but
            // termites/particles/flames render as garbage/nothing, this is the first
            // place to look.
            pass.set_pipeline(sprite_pipeline);
            for termite in gpu.termites.iter() {
                let frame = (termite.frame.max(0) as usize) % termite_textures.len().max(1);
                let Some((_, tex_view)) = termite_textures.get(frame) else { continue };
                let size_px = (TERMITE_BASE_SIZE.0 * termite.scale, TERMITE_BASE_SIZE.1 * termite.scale);
                // `SpriteFactory::termite`'s head art is on the sprite's own *left* (see
                // `colony.rs`'s bite-position comment) — local "forward" is -x, not +x —
                // so facing `heading` means rotating by `heading + π`, not `heading`
                // itself: rotating local -x by that angle lands on
                // (cos(heading), sin(heading)), i.e. pointing along heading. Worked
                // through by hand rather than assumed; see `rotated_sprite_ndc`'s doc
                // comment for why the rotation itself happens in pixel space, not here.
                let rotation = termite.heading + std::f32::consts::PI;
                let ndc = rotated_sprite_ndc(termite.position, size_px, rotation, (gpu.width as f32, gpu.height as f32), 1.0);
                draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, tex_view, &ndc, "termite");
            }

            // Drawn last — above everything, never occluded. The rotatable-sprite
            // pipeline, not `pipeline` (tiles) — see the "Cursor icons" module section.
            if show_cursor {
                if let Some(icon) = icon_gpu.get(variant_key) {
                    pass.set_pipeline(sprite_pipeline);
                    pass.set_bind_group(0, &icon.bind_group, &[]);
                    pass.draw(0..6, 0..1);
                }
            }

            // The HUD — above even the cursor (`GameScene.swift`'s `Z.hud` is the highest
            // z-index there is, above `Z.cursor`). The bar/label/hint are always visible
            // (Swift never hides `buildHUD`'s own nodes); only the toast fades, which is
            // why it alone rides the alpha-capable rotated-sprite pipeline instead.
            pass.set_pipeline(pipeline);
            let screen_px = (gpu.width as f32, gpu.height as f32);
            let bar_center = (screen_px.0 / 2.0, screen_px.1 - HUD_BAR_BOTTOM_MARGIN);
            if let Some(panel) = hud_panel {
                draw_hud_element(queue, &mut pass, panel, bar_center, (0.5, 0.5), screen_px);
            }
            if let Some(label) = hud_label {
                let anchor = (bar_center.0 - HUD_BAR_SIZE.0 / 2.0 + HUD_BAR_PADDING, bar_center.1);
                draw_hud_element(queue, &mut pass, label, anchor, (0.0, 0.5), screen_px);
            }
            if let Some(hint) = hud_hint {
                let anchor = (bar_center.0 + HUD_BAR_SIZE.0 / 2.0 - HUD_BAR_PADDING, bar_center.1);
                draw_hud_element(queue, &mut pass, hint, anchor, (1.0, 0.5), screen_px);
            }

            if let Some((toast_text, alpha)) = gpu.hud.toast() {
                let toast_text = toast_text.to_string(); // ends the borrow of `gpu.hud` before touching `gpu.toast_gpu`
                ensure_toast_gpu(device, queue, text, &mut gpu.toast_gpu, &toast_text, HUD_TOAST_SIZE, hud_rgba_arr(hud_colors.foreground, 255));
                if let Some(toast) = &gpu.toast_gpu {
                    pass.set_pipeline(sprite_pipeline);
                    let center = (screen_px.0 / 2.0, screen_px.1 - HUD_TOAST_BOTTOM_MARGIN);
                    let ndc = rotated_sprite_ndc(center, (toast.width as f32, toast.height as f32), 0.0, screen_px, alpha);
                    draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, &toast.view, &ndc, "toast");
                }
            } else if gpu.toast_gpu.is_some() {
                gpu.toast_gpu = None; // free the texture once the toast is fully gone
            }

            // The tool palette — drawn after the always-on bar, matching
            // `GameScene.swift`'s child order (`palette.node` added after `buildHUD`'s
            // bar). Every element rides the alpha-capable rotated-sprite pipeline
            // (rotation always 0) since the whole palette fades via `hud.palette_alpha()`.
            let palette_alpha = gpu.hud.palette_alpha();
            if palette_alpha > 0.0 {
                pass.set_pipeline(sprite_pipeline);
                if let Some((_, view, pw, ph)) = palette_panel {
                    let center = (screen_px.0 / 2.0, screen_px.1 - PALETTE_BOTTOM_MARGIN - PALETTE_CELL / 2.0);
                    let ndc = rotated_sprite_ndc(center, (*pw, *ph), 0.0, screen_px, palette_alpha);
                    draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, view, &ndc, "palette-panel");
                }
                for (i, id) in ToolId::ALL.into_iter().enumerate() {
                    let center = palette_cell_center(i, screen_px);
                    let cell_bg = if id == gpu.active_tool { palette_cell_selected } else { palette_cell_normal };
                    if let Some((_, view)) = cell_bg {
                        let ndc = rotated_sprite_ndc(center, (PALETTE_CELL, PALETTE_CELL), 0.0, screen_px, palette_alpha);
                        draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, view, &ndc, "palette-cell");
                    }
                    if let Some((_, view)) = palette_icons.get(&id) {
                        // `sprite.position.y = frame.midY + 4` (y-up) — 4pt visually
                        // *above* the cell's own centre, so `-4` here (y-down).
                        let icon_center = (center.0, center.1 - 4.0);
                        let ndc = rotated_sprite_ndc(icon_center, (PALETTE_ICON_SIZE, PALETTE_ICON_SIZE), 0.0, screen_px, palette_alpha);
                        draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, view, &ndc, "palette-icon");
                    }
                    if let Some((_, view, dw, dh)) = palette_digits.get(&id) {
                        // `digit.position.y = frame.minY + 8` (y-up) — 8pt above the
                        // cell's bottom edge; that edge is `center.1 + CELL/2` (y-down).
                        let digit_center = (center.0, center.1 + PALETTE_CELL / 2.0 - 8.0);
                        let ndc = rotated_sprite_ndc(digit_center, (*dw, *dh), 0.0, screen_px, palette_alpha);
                        draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, view, &ndc, "palette-digit");
                    }
                }
                // The palette's own name label — reuses `hud_label`'s texture (identical
                // text, "N · ToolName") rather than building a second, near-identical one
                // at a slightly different size; `ToolPaletteHUD.swift`'s own `nameLabel`
                // (13pt semibold) and `buildHUD`'s `toolLabel` (14pt medium) differ only
                // slightly anyway.
                if let Some(label) = hud_label {
                    let center = (screen_px.0 / 2.0, screen_px.1 - PALETTE_NAME_MARGIN);
                    let ndc = rotated_sprite_ndc(center, (label.width as f32, label.height as f32), 0.0, screen_px, palette_alpha);
                    draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, &label.view, &ndc, "palette-name");
                }
            }

            // The credits panel — drawn last of all (`credits.node.zPosition = 10`,
            // "above the palette, within the HUD layer", in `AppDelegate.swift`), and
            // screen-centred, unlike everything else in the HUD.
            let credits_alpha = gpu.hud.credits_alpha();
            if credits_alpha > 0.0 {
                if let Some((_, view, w, h)) = credits_panel {
                    pass.set_pipeline(sprite_pipeline);
                    let center = (screen_px.0 / 2.0, screen_px.1 / 2.0);
                    let ndc = rotated_sprite_ndc(center, (*w, *h), 0.0, screen_px, credits_alpha);
                    draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, view, &ndc, "credits-panel");
                }
            }
        }
        queue.submit(Some(encoder.finish()));
        frame.present();
    }
}

/// A minimal HasWindowHandle/HasDisplayHandle bridge from raw Wayland pointers.
///
/// Both pointers outlive this struct in practice (the connection and the layer surface
/// are both kept alive in `State` for as long as `GpuLayer` exists), but nothing here
/// enforces that at the type level — this is spike code, not the real renderer.
struct RawWaylandHandle {
    display: *mut std::ffi::c_void,
    surface: *mut std::ffi::c_void,
}

// SAFETY: these are opaque libwayland-client pointers, not thread-confined Rust data;
// wgpu only ever reads them on the thread that already owns the Wayland connection here.
unsafe impl Send for RawWaylandHandle {}
unsafe impl Sync for RawWaylandHandle {}

impl HasDisplayHandle for RawWaylandHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let ptr = std::ptr::NonNull::new(self.display).ok_or(HandleError::Unavailable)?;
        let handle = WaylandDisplayHandle::new(ptr);
        Ok(unsafe { DisplayHandle::borrow_raw(RawDisplayHandle::Wayland(handle)) })
    }
}

impl HasWindowHandle for RawWaylandHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let ptr = std::ptr::NonNull::new(self.surface).ok_or(HandleError::Unavailable)?;
        let handle = WaylandWindowHandle::new(ptr);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Wayland(handle)) })
    }
}

// ── SCTK plumbing ────────────────────────────────────────────────────────────────────────

impl CompositorHandler for State {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        // Ahead of everything else below, which borrows several of `self`'s fields
        // immutably (`device`/`queue`/... — kept alive for the rest of this function, for
        // `render`) — `maybe_start_capture` needs a genuine `&mut self`, which can't
        // overlap those, so it has to run before they're taken, not interleaved with the
        // rest of this function's own per-output lookup below.
        if let Some(index) = self.layers.iter().position(|l| l.layer.wl_surface() == surface) {
            self.maybe_start_capture(index);
        }

        let (Some(device), Some(queue), Some(pipelines), Some(sampler), Some(cursor_uniform)) =
            (&self.device, &self.queue, &self.pipelines, &self.sampler, &self.cursor_uniform_buffer)
        else {
            return;
        };
        let show_cursor = self.pointer_focus.as_ref() == Some(surface);
        // `AudioEngine.swift`'s "only the first scene drives its update loop" — the loop
        // gain-glide is shared state (see `AudioBackend::update`'s doc comment), so it
        // must only ever advance once per real frame, not once per output's own frame
        // callback. Computed before `gpu` borrows `self.layers` mutably below.
        let is_audio_driving_output = self.layers.first().map(|l| l.layer.wl_surface()) == Some(surface);
        if let Some(index) = self.layers.iter().position(|l| l.layer.wl_surface() == surface) {
            let gpu = &mut self.layers[index];
            if gpu.configured {
                let now = Instant::now();
                // Clamped so a stalled frame (a compositor hiccup, a debugger pause)
                // doesn't hand a tool a huge `dt` and make it think a lot of time passed
                // instantly — Swift gets this from SpriteKit's own frame timing; here we
                // own the clock, so the clamp is ours to add.
                let dt = gpu.last_frame_time.map(|t| (now - t).as_secs_f32()).unwrap_or(1.0 / 60.0).min(0.1);
                gpu.last_frame_time = Some(now);

                State::advance(&mut self.decals, &mut self.audio, gpu, dt);
                if is_audio_driving_output {
                    self.audio.update(dt);
                }
                State::render(
                    device,
                    queue,
                    &pipelines.tile,
                    &pipelines.sprite,
                    &pipelines.sprite_additive,
                    &pipelines.sprite_bind_group_layout,
                    sampler,
                    &mut self.icons,
                    &self.icon_gpu,
                    cursor_uniform,
                    &self.termite_textures,
                    &self.shell_texture,
                    &self.droplet_textures,
                    &self.flame_textures,
                    &self.sliver_textures,
                    &self.flash_texture,
                    &mut self.text,
                    &self.hud_panel,
                    &self.hud_hint,
                    &self.hud_label,
                    &self.palette_panel,
                    &self.palette_cell_normal,
                    &self.palette_cell_selected,
                    &self.palette_icons,
                    &self.palette_digits,
                    &self.credits_panel,
                    &self.hud_colors,
                    show_cursor,
                    gpu,
                );
                gpu.layer.wl_surface().frame(&self.qh, surface.clone());
                gpu.layer.wl_surface().commit();
            }
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.spawn_layer_for_output(qh, output);
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.layers.retain(|l| &l.layer != layer);
        if self.layers.is_empty() {
            self.exit = true;
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(gpu) = self.layers.iter_mut().find(|l| &l.layer == layer) else { return };
        let (Some(device), Some(queue)) = (&self.device, &self.queue) else { return };

        // `configure.new_size` is in *points* — `gpu.width`/`gpu.height` stay in points
        // too, exactly as they've always been (matching mouse/damage coordinates, which
        // `hakai_core::damage::DamageLayer` also treats as points — confirmed by reading
        // its own source, not assumed). Only the `wgpu` surface's *actual pixel buffer*
        // (`buffer_width`/`buffer_height`, computed just below, local to this function —
        // not stored anywhere) needs to know about `gpu.scale` at all: NDC is a *ratio*
        // (`origin / screen_size`), so it comes out identical whether both sides of that
        // ratio are expressed in points or in pixels, as long as they agree — which is
        // exactly why nothing else in this file needs to change for fractional scale to
        // render crisply. See the "Fractional scale" module section.
        let (w, h) = configure.new_size;
        gpu.width = w.max(1);
        gpu.height = h.max(1);
        let buffer_width = ((gpu.width as f32) * gpu.scale).round().max(1.0) as u32;
        let buffer_height = ((gpu.height as f32) * gpu.scale).round().max(1.0) as u32;
        if let Some(viewport) = &gpu.viewport {
            // Tells the compositor: the buffer about to be configured at the *pixel* size
            // above should be displayed scaled down to this *point* size — the other half
            // of what makes rendering at a higher, crisper pixel density than the
            // surface's own logical size actually work.
            viewport.set_destination(gpu.width as i32, gpu.height as i32);
        }

        let caps = gpu.wgpu_surface.get_capabilities(self.adapter.as_ref().unwrap());
        // Prefer a *non*-sRGB swapchain format: the tile textures are plain Rgba8Unorm
        // with no gamma handling anywhere in the shader, so writing that straight to an
        // sRGB view would apply an unwanted linear→sRGB re-encode on top of pixel bytes
        // that are already final. Simplest to just avoid sRGB entirely on both ends.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);

        gpu.wgpu_surface.configure(
            device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: buffer_width,
                height: buffer_height,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: caps
                    .alpha_modes
                    .iter()
                    .copied()
                    .find(|m| *m == wgpu::CompositeAlphaMode::PreMultiplied)
                    .unwrap_or(caps.alpha_modes[0]),
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );

        if self.pipelines.is_none() {
            let pipelines = create_pipelines(device, format);
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("tile-sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });

            let cursor_uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cursor-uniform"),
                // `RotatedSpriteUniform`, not `TileUniform` — see the "Cursor icons"
                // module section: the cursor now rides the rotatable-sprite pipeline.
                size: std::mem::size_of::<RotatedSpriteUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            // All eleven icon variants, uploaded once. Each call below borrows
            // `self.icons` mutably and releases it again by the end of its own
            // statement, so they can run one after another even though `self.icons`
            // itself is a single shared cache. On the rotatable-sprite layout, not the
            // tile one — see the "Cursor icons" module section.
            let layout = &pipelines.sprite_bind_group_layout;
            let mut icon_gpu = HashMap::new();
            icon_gpu.insert("hammer", create_icon_gpu(device, queue, layout, &sampler, &cursor_uniform, &self.icons.hammer().pixmap));
            icon_gpu.insert("saw_idle", create_icon_gpu(device, queue, layout, &sampler, &cursor_uniform, &self.icons.chain_saw(false).pixmap));
            icon_gpu.insert("saw_cut", create_icon_gpu(device, queue, layout, &sampler, &cursor_uniform, &self.icons.chain_saw(true).pixmap));
            icon_gpu.insert("machinegun", create_icon_gpu(device, queue, layout, &sampler, &cursor_uniform, &self.icons.machine_gun().pixmap));
            icon_gpu.insert("flamethrower", create_icon_gpu(device, queue, layout, &sampler, &cursor_uniform, &self.icons.flame_thrower().pixmap));
            icon_gpu.insert("colorthrower", create_icon_gpu(device, queue, layout, &sampler, &cursor_uniform, &self.icons.color_thrower().pixmap));
            icon_gpu.insert("phaser", create_icon_gpu(device, queue, layout, &sampler, &cursor_uniform, &self.icons.phaser().pixmap));
            icon_gpu.insert("stamp_up", create_icon_gpu(device, queue, layout, &sampler, &cursor_uniform, &self.icons.stamp(false).pixmap));
            icon_gpu.insert("stamp_down", create_icon_gpu(device, queue, layout, &sampler, &cursor_uniform, &self.icons.stamp(true).pixmap));
            icon_gpu.insert("termites", create_icon_gpu(device, queue, layout, &sampler, &cursor_uniform, &self.icons.termite_hand().pixmap));
            icon_gpu.insert("washer", create_icon_gpu(device, queue, layout, &sampler, &cursor_uniform, &self.icons.washer().pixmap));
            log::info!("uploaded {} cursor icon variants", icon_gpu.len());

            let termite_textures = vec![
                create_sprite_texture(device, queue, self.sprites.termite(0)),
                create_sprite_texture(device, queue, self.sprites.termite(1)),
            ];

            // Shells, paint droplets and standing-flame frames — same "texture + view
            // only, fresh bind group per draw" treatment as termites, for the same reason
            // (many live at once, all with independently changing placement).
            let shell_texture = create_sprite_texture(device, queue, self.sprites.shell());
            // Read once, outside the closure: `self.decals.paint_colors()` returns a plain
            // `Copy` value, so there's no live borrow of `self.decals` left for the
            // closure below (which needs `self.sprites` instead) to conflict with.
            let paint_colors = self.decals.paint_colors();
            let droplet_textures: Vec<_> = (0..DecalFactory::DEFAULT_PAINT_COLORS.len() as i64)
                .map(|i| create_sprite_texture(device, queue, self.sprites.droplet(i, &paint_colors)))
                .collect();
            let flame_textures: Vec<_> = (0..SpriteFactory::FLAME_FRAMES)
                .map(|f| create_sprite_texture(device, queue, self.sprites.standing_flame(f)))
                .collect();
            // Slivers come from `self.decals` (`DecalFactory`), not `self.sprites` — the
            // same generator the hammer/chain-saw use to *stamp* a sliver decal onto the
            // damage layer doubles as the texture for the live particle, matching
            // `Hammer.swift`/`ChainSaw.swift`'s own `ctx.decals.sliver(variant:)` reuse.
            let sliver_textures: Vec<_> = (0..DecalFactory::SLIVER_VARIANTS)
                .map(|v| create_sprite_texture(device, queue, self.decals.sliver(v)))
                .collect();
            let flash_texture = create_sprite_texture(device, queue, self.sprites.flash());
            log::info!(
                "uploaded shell/droplet/flame/sliver/flash textures: 1 shell, {} droplet colors, {} flame frames, {} sliver variants, 1 flash",
                droplet_textures.len(),
                flame_textures.len(),
                sliver_textures.len()
            );

            // The HUD bar's panel and its (static) hint text — built once here, same as
            // the icon variants above. The tool-name label isn't built here: it depends
            // on `active_tool`, so it's built here too, using `ToolId::Hammer` — every
            // `GpuLayer` starts on the hammer already (see `spawn_layer_for_output`), it
            // just never actually goes through `select_tool` to get there, so nothing
            // else would otherwise build this label before the user's first tool switch.
            let hud_panel_pixmap = build_panel_pixmap(
                HUD_BAR_SIZE.0 as u32,
                HUD_BAR_SIZE.1 as u32,
                hud_rgba(self.hud_colors.background, 158), // 62% — `bar.fillColor`
                Some((hud_rgba(self.hud_colors.foreground, 56), 1.0)), // 22% — `bar.strokeColor`, `lineWidth: 1`
            );
            let hud_panel = create_hud_gpu(device, queue, &pipelines.tile_bind_group_layout, &sampler, &hud_panel_pixmap, String::new());
            let hud_hint = create_hud_text(device, queue, &pipelines.tile_bind_group_layout, &sampler, &mut self.text, HUD_HINT_TEXT, HUD_HINT_SIZE, false, hud_rgba_arr(self.hud_colors.foreground, 153));
            let initial_label = format!("{} \u{b7} {}", ToolId::Hammer.key_digit(), ToolId::Hammer.display_name());
            let hud_label = create_hud_text(device, queue, &pipelines.tile_bind_group_layout, &sampler, &mut self.text, &initial_label, HUD_LABEL_SIZE, false, hud_rgba_arr(self.hud_colors.foreground, 255));
            log::info!("HUD panel and hint text built ({}x{})", hud_panel.width, hud_panel.height);

            // The tool palette — panel, the two cell-background states, and every tool's
            // icon and digit. All built once here; see the "Tool palette" module section
            // for why these are plain `(Texture, TextureView[, w, h])` tuples rather than
            // `HudGpu`s.
            let palette_panel_pixmap = build_panel_pixmap(
                (palette_total_width() + 24.0) as u32,
                (PALETTE_CELL + 24.0) as u32,
                hud_rgba(self.hud_colors.background, 168), // 66% — `panel.fillColor`
                Some((hud_rgba(self.hud_colors.foreground, 56), 1.0)), // 22% — `panel.strokeColor`
            );
            let (palette_panel_texture, palette_panel_view) = create_sprite_texture(device, queue, &palette_panel_pixmap);
            let palette_panel = Some((palette_panel_texture, palette_panel_view, palette_panel_pixmap.width() as f32, palette_panel_pixmap.height() as f32));

            let cell_normal_pixmap = build_panel_pixmap(PALETTE_CELL as u32, PALETTE_CELL as u32, hud_rgba(self.hud_colors.foreground, 18), None);
            let palette_cell_normal = Some(create_sprite_texture(device, queue, &cell_normal_pixmap));
            // The selected cell is the one place this chunk reaches for `accent` rather
            // than `foreground`/`background` — the clearest single "this is yours" signal
            // the HUD has, since it's the one element that's supposed to draw the eye.
            let cell_selected_pixmap = build_panel_pixmap(
                PALETTE_CELL as u32,
                PALETTE_CELL as u32,
                hud_rgba(self.hud_colors.accent, 66),
                Some((hud_rgba(self.hud_colors.accent, 191), 1.5)),
            );
            let palette_cell_selected = Some(create_sprite_texture(device, queue, &cell_selected_pixmap));

            let mut palette_icons = HashMap::new();
            let mut palette_digits = HashMap::new();
            for id in ToolId::ALL {
                let icon_texture = create_sprite_texture(device, queue, palette_icon_pixmap(&mut self.icons, id));
                palette_icons.insert(id, icon_texture);

                let digit = format!("{}", id.key_digit());
                if let Some(pixmap) = self.text.rasterize(&digit, PALETTE_DIGIT_SIZE, true, hud_rgba_arr(self.hud_colors.foreground, 140)) {
                    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
                    let (texture, view) = create_sprite_texture(device, queue, &pixmap);
                    palette_digits.insert(id, (texture, view, w, h));
                }
            }
            log::info!("tool palette built: panel, 2 cell states, {} icons, {} digits", palette_icons.len(), palette_digits.len());

            // The credits panel — built once, from this (the first) output's width; see
            // the "Credits panel" module section for why that's an accepted simplification
            // rather than a per-output rebuild.
            let credits_pixmap = build_credits_pixmap(&mut self.text, gpu.width as f32, &self.hud_colors);
            let (credits_w, credits_h) = (credits_pixmap.width() as f32, credits_pixmap.height() as f32);
            let (credits_texture, credits_view) = create_sprite_texture(device, queue, &credits_pixmap);
            let credits_panel = Some((credits_texture, credits_view, credits_w, credits_h));
            log::info!("credits panel built ({credits_w}x{credits_h})");

            self.pipelines = Some(pipelines);
            self.sampler = Some(sampler);
            self.cursor_uniform_buffer = Some(cursor_uniform);
            self.icon_gpu = icon_gpu;
            self.termite_textures = termite_textures;
            self.shell_texture = Some(shell_texture);
            self.droplet_textures = droplet_textures;
            self.flame_textures = flame_textures;
            self.sliver_textures = sliver_textures;
            self.flash_texture = Some(flash_texture);
            self.hud_panel = Some(hud_panel);
            self.hud_hint = hud_hint;
            self.hud_label = hud_label;
            self.palette_panel = palette_panel;
            self.palette_cell_normal = palette_cell_normal;
            self.palette_cell_selected = palette_cell_selected;
            self.palette_icons = palette_icons;
            self.palette_digits = palette_digits;
            self.credits_panel = credits_panel;
        }

        // (Re)build this output's damage layer and its GPU tiles at the new size — points
        // (`gpu.width`/`gpu.height`), matching `DamageLayer::new`'s own expectation.
        let mut damage = DamageLayer::new(gpu.width as f32, gpu.height as f32, gpu.scale);
        // Every tile's pixels are already correct (freshly allocated, fully transparent),
        // but its GPU texture's memory isn't until at least one upload has happened — see
        // `DamageLayer::mark_all_dirty`'s doc comment. Without this, a fresh layer would
        // render as uninitialized VRAM garbage instead of a clean blank overlay.
        damage.mark_all_dirty();

        let (cols, rows) = damage.grid_size();
        // `side_px` (pixels, via `damage.scale()`) sizes the tile's actual GPU *texture* —
        // that genuinely needs to be higher-resolution for a crisp fractional-scale
        // render. Its NDC *placement* below is a different concern entirely and stays in
        // points throughout (`origin`, `DamageLayer::TILE_SIDE`, `gpu.width`/`gpu.height`
        // all points) — an NDC ratio comes out identical either way (see this function's
        // own comment on `buffer_width`/`buffer_height` above for why), so there's no
        // reason to convert it to pixels just to convert straight back via division.
        let side_px = (DamageLayer::TILE_SIDE * damage.scale()) as u32;
        let mut tiles = Vec::with_capacity(cols * rows);
        for i in 0..(cols * rows) {
            let origin = damage.tile_origin(i);
            let ndc = tile_ndc(origin, (DamageLayer::TILE_SIDE, DamageLayer::TILE_SIDE), (gpu.width as f32, gpu.height as f32));
            tiles.push(create_tile_gpu(
                device,
                &self.pipelines.as_ref().unwrap().tile_bind_group_layout,
                self.sampler.as_ref().unwrap(),
                side_px,
                ndc,
            ));
        }

        gpu.damage = Some(damage);
        gpu.tiles = tiles;
        gpu.configured = true;

        // Requesting the *next* frame callback here — not just in `frame()` itself — is
        // what actually starts the continuous redraw loop. Without this, `frame()` never
        // fires at all after this first render: nothing else in the program ever asks the
        // compositor for a frame callback, so every later change (a stamped crack, a
        // moving termite) gets marked dirty correctly but is never actually uploaded or
        // drawn. Chunk 1 didn't need this — it rendered one static scripted crack and
        // never had to update again.
        layer.wl_surface().frame(&self.qh, layer.wl_surface().clone());
        let show_cursor = self.pointer_focus.as_ref() == Some(layer.wl_surface());
        let pipelines = self.pipelines.as_ref().unwrap();
        State::render(
            device,
            queue,
            &pipelines.tile,
            &pipelines.sprite,
            &pipelines.sprite_additive,
            &pipelines.sprite_bind_group_layout,
            self.sampler.as_ref().unwrap(),
            &mut self.icons,
            &self.icon_gpu,
            self.cursor_uniform_buffer.as_ref().unwrap(),
            &self.termite_textures,
            &self.shell_texture,
            &self.droplet_textures,
            &self.flame_textures,
            &self.sliver_textures,
            &self.flash_texture,
            &mut self.text,
            &self.hud_panel,
            &self.hud_hint,
            &self.hud_label,
            &self.palette_panel,
            &self.palette_cell_normal,
            &self.palette_cell_selected,
            &self.palette_icons,
            &self.palette_digits,
            &self.credits_panel,
            &self.hud_colors,
            show_cursor,
            gpu,
        );
        layer.wl_surface().commit();
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = Some(self.seat_state.get_keyboard(qh, &seat, None).unwrap());
            log::info!("keyboard capability bound");
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = Some(self.seat_state.get_pointer(qh, &seat).unwrap());
            log::info!("pointer capability bound");
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, _: Capability) {}
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for State {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}

    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        // RISK: bound by keysym, not scancode, deliberately — matches the plan's own
        // note about non-QWERTY layouts. `Keysym::_1`..`Keysym::_9` (the leading
        // underscore because a bare `1` isn't a valid Rust identifier) and `Keysym::r`
        // are my best-confidence reading of xkbcommon-rs's naming; if any of these don't
        // compile or don't fire, that's the mapping to check first.
        log::debug!("key pressed: {:?}", event.keysym);
        match event.keysym {
            Keysym::Escape => {
                log::info!("Esc pressed — quitting");
                self.exit = true;
            }
            Keysym::_1 => self.select_tool(ToolId::Hammer),
            Keysym::_2 => self.select_tool(ToolId::ChainSaw),
            Keysym::_3 => self.select_tool(ToolId::MachineGun),
            Keysym::_4 => self.select_tool(ToolId::FlameThrower),
            Keysym::_5 => self.select_tool(ToolId::ColorThrower),
            Keysym::_6 => self.select_tool(ToolId::Phaser),
            Keysym::_7 => self.select_tool(ToolId::Stamp),
            Keysym::_8 => self.select_tool(ToolId::Termites),
            Keysym::_9 => self.select_tool(ToolId::Washer),
            Keysym::r | Keysym::R => self.erase_all(),
            // Shift+Tab often doesn't arrive as `Tab` with a shift modifier at all — XKB's
            // standard keymap remaps it to a *different* keysym, `ISO_Left_Tab`, the
            // moment shift is held (this is why `shift_held` alone wasn't enough: the
            // `Keysym::Tab` arm below never even matched on Shift+Tab, `shift_held` or
            // not). Both are handled explicitly rather than relying on `shift_held` for
            // this one key — `ISO_Left_Tab` only ever shows up already-shifted, so there's
            // nothing left for `shift_held` to disambiguate for it.
            Keysym::Tab => self.cycle_tool(if self.shift_held { -1 } else { 1 }),
            Keysym::ISO_Left_Tab => self.cycle_tool(-1),
            // RISK: `Keysym::Up`/`Keysym::Down` are my best-confidence reading of
            // xkbcommon-rs's arrow-key names (mirroring X11's `XK_Up`/`XK_Down`) — same
            // confidence level as the digit keysyms above.
            Keysym::Up => self.set_palette_visible(true),
            Keysym::Down => self.set_palette_visible(false),
            Keysym::c | Keysym::C => self.toggle_credits(),
            Keysym::m | Keysym::M => self.toggle_mode(),
            _ => {}
        }
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: smithay_client_toolkit::seat::keyboard::Modifiers, _: u32) {
        // RISK: `Modifiers::shift` is my best-confidence reading of this struct's field
        // name — if it doesn't compile, this is the one line to fix; `Modifiers` is
        // otherwise not used anywhere else in this file.
        self.shift_held = modifiers.shift;
    }
}

/// The Linux input-event code for the left mouse button — same value AppKit implicitly
/// used for `mouseDown` in the macOS build; Wayland just makes it an explicit number.
const BTN_LEFT: u32 = 0x110;

impl PointerHandler for State {
    // RISK: `PointerEvent { surface, position, kind }` and `PointerEventKind`'s variants
    // (`Enter { serial }`, `Leave { serial }`, `Motion { time }`, `Press { time, button,
    // serial }`, `Release { time, button, serial }`, plus `Axis { .. }` not handled here)
    // are my best-confidence reading of `smithay_client_toolkit::seat::pointer` at this
    // pinned SCTK version — the one part of this handler not verified against a compiler.
    fn pointer_frame(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, pointer: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            let surface = event.surface.clone();
            // Wayland always reports pointer positions in a surface's *logical* units
            // (points) — the same units `gpu.width`/`gpu.height`, damage tiles, and
            // HUD/palette layout all use throughout this file (see the "Fractional scale"
            // module section), so this is used directly, with no `gpu.scale` multiply.
            let point = (event.position.0 as f32, event.position.1 as f32);
            log::debug!("pointer event: {:?} @ {:?}", event.kind, point);

            match event.kind {
                PointerEventKind::Enter { serial } => {
                    log::debug!("pointer entered a layer surface");
                    self.pointer_focus = Some(surface.clone());
                    // Hide the compositor's own cursor — this overlay draws its own.
                    pointer.set_cursor(serial, None, 0, 0);
                    // `event.position` on `Enter` is the pointer's actual surface-local
                    // position at the moment it entered (per the `wl_pointer` protocol),
                    // not a placeholder — without writing it into `gpu.mouse` here, the
                    // tool icon stayed wherever `mouse` last was (initially `(0, 0)`, from
                    // `GpuLayer`'s own construction) until the *next* `Motion` event,
                    // rather than appearing under the real cursor immediately on startup
                    // or whenever the pointer re-enters after having left.
                    if let Some(gpu) = self.layers.iter_mut().find(|l| l.layer.wl_surface() == &surface) {
                        gpu.mouse = point;
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if self.pointer_focus.as_ref() == Some(&surface) {
                        self.pointer_focus = None;
                    }
                }
                PointerEventKind::Motion { .. } => {
                    let Some(gpu) = self.layers.iter_mut().find(|l| l.layer.wl_surface() == &surface) else { continue };
                    gpu.mouse = point;
                    if !gpu.is_down {
                        continue;
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
                PointerEventKind::Press { button, .. } => {
                    log::debug!("press: button=0x{button:x} (BTN_LEFT is 0x{BTN_LEFT:x})");
                    if button != BTN_LEFT {
                        continue;
                    }
                    let Some(gpu) = self.layers.iter_mut().find(|l| l.layer.wl_surface() == &surface) else {
                        log::warn!("press on a surface with no matching GpuLayer");
                        continue;
                    };
                    gpu.mouse = point;

                    // The palette and the credits panel, while open, swallow the click
                    // instead of delivering it to the active tool — checked, and `is_down`
                    // deliberately left `false`, *before* any tool dispatch below. Setting
                    // `is_down` regardless and swallowing only the immediate `mouse_down`
                    // would still leave it `true` for every `Motion` event that follows
                    // (until release), which would stamp the desktop underneath a modal
                    // panel the user thinks they're just clicking on.
                    let screen = (gpu.width as f32, gpu.height as f32);
                    let palette_hit = if gpu.hud.palette_open() { palette_tool_at(point, screen) } else { None };
                    if let Some(id) = palette_hit {
                        self.select_tool(id);
                        continue;
                    }
                    // `CreditsPanel.swift`'s own comment ("so the scene can swallow clicks
                    // on it") suggests the original only checks its own bounds; swallowing
                    // every click while it's open is simpler and, for a modal
                    // acknowledgements screen, the more expected behavior anyway.
                    if gpu.hud.credits_open() {
                        continue;
                    }

                    let Some(gpu) = self.layers.iter_mut().find(|l| l.layer.wl_surface() == &surface) else { continue };
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
                        log::debug!("mouse_down delivered to {:?} at {:?}, dirty tiles now: {}", active, point, ctx.damage.dirty_count());
                    } else {
                        log::warn!("no tool/damage available for {:?} on press", active);
                    }
                }
                PointerEventKind::Release { button, .. } => {
                    if button != BTN_LEFT {
                        continue;
                    }
                    let Some(gpu) = self.layers.iter_mut().find(|l| l.layer.wl_surface() == &surface) else { continue };
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
                _ => {}
            }
        }
    }
}

// ── Screen capture ───────────────────────────────────────────────────────────────────────
//
// `zwlr_screencopy_v1`'s own request/event dance: `capture_output` (issued from
// `State::maybe_start_capture`) returns a `ZwlrScreencopyFrameV1` that fires `buffer`
// (possibly several times — different format/size options) then `buffer_done`, at which
// point an SHM buffer matching whatever was advertised is created and `copy`'d into; the
// frame then fires either `ready` (success — read the buffer) or `failed`. `flags`,
// `damage` and `linux_dmabuf` aren't used by this port at all: no cursor overlay decisions
// depend on `flags`, no incremental redraw depends on `damage` (a whole fresh capture is
// cheap enough at this cadence — `CAPTURE_INTERVAL` — not to need it), and `linux_dmabuf`
// is the GPU-buffer alternative to the SHM path actually used here.

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        // Only ever called from `wl_shm`/`wl_buffer` event dispatch, which only exists on
        // this queue at all because `Shm::bind` already succeeded at startup (see `main`)
        // — a `None` here would mean that binding somehow vanished afterward, which
        // shouldn't be possible.
        self.shm.as_mut().expect("ShmHandler::shm_state called, but wl_shm was never bound")
    }
}

impl Dispatch<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
        _event: zwlr_screencopy_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // This interface has no events at all — it only ever makes requests
        // (`capture_output`/`capture_output_region`/`destroy`). A `Dispatch` impl still
        // has to exist for `wayland-client` to let the type be used at all.
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Identified by comparing the frame object's own identity against whichever
        // `GpuLayer` started it, rather than threading a `wl_output` (or an index) through
        // as this `Dispatch`'s user data — one fewer thing that could drift out of sync
        // with `capture_frame` itself.
        let Some(gpu) = state.layers.iter_mut().find(|l| l.capture_frame.as_ref() == Some(proxy)) else { return };

        match event {
            zwlr_screencopy_frame_v1::Event::Buffer { format, width, height, stride } => {
                // RISK: `format` arrives as a `WEnum<wl_shm::Format>` (a value that might
                // not be a format this binding's generated enum recognizes) — my
                // best-confidence reading of `wayland-scanner`'s usual codegen for
                // enum-typed event fields, not verified against a compiler. A frame can
                // send several `Buffer` events (different format/size options); this
                // simplification just keeps whichever arrived last rather than picking
                // the best one — fine for a brightness sample, which doesn't care which
                // SHM format it reads back through.
                if let Ok(format) = format.into_result() {
                    gpu.capture_buffer_info = Some((format, width, height, stride));
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                let Some((format, width, height, stride)) = gpu.capture_buffer_info else { return };
                let Some(pool) = &mut state.shm_pool else { return };
                match pool.create_buffer(width as i32, height as i32, stride as i32, format) {
                    Ok((buffer, _canvas)) => {
                        // `_canvas` is this buffer's *writable* view at creation time —
                        // unused here since this buffer receives the compositor's own
                        // `copy` write, not ours; its contents are read back through
                        // `Buffer::canvas` again once `ready` fires instead.
                        proxy.copy(buffer.wl_buffer());
                        gpu.capture_buffer = Some(buffer);
                    }
                    Err(e) => {
                        log::warn!("failed to create an SHM buffer for screen capture: {e}");
                        gpu.capture_frame = None;
                        gpu.capture_buffer_info = None;
                    }
                }
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                if let (Some(pool), Some(buffer), Some((_, width, height, stride))) = (&mut state.shm_pool, &gpu.capture_buffer, gpu.capture_buffer_info) {
                    if let Some(bytes) = buffer.canvas(pool) {
                        gpu.brightness.update(bytes, width, height, stride);
                        // The full-resolution RGBA conversion + texture upload only
                        // happens while frozen — nothing ever displays it otherwise, so
                        // doing this every `CAPTURE_INTERVAL` in live mode too would just
                        // be wasted bandwidth (a 4K frame is tens of MB).
                        if gpu.frozen {
                            if let (Some(device), Some(queue)) = (&state.device, &state.queue) {
                                let rgba = capture::to_rgba(bytes, width, height, stride);
                                let (texture, view) = create_texture_from_rgba(device, queue, &rgba, width, height);
                                gpu.snapshot_texture = Some((texture, view, width, height));
                            }
                        }
                    }
                }
                gpu.capture_frame = None;
                gpu.capture_buffer_info = None;
                gpu.capture_buffer = None;
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                log::debug!("screen capture failed for one output — will retry in {}s", capture::CAPTURE_INTERVAL);
                gpu.capture_frame = None;
                gpu.capture_buffer_info = None;
                gpu.capture_buffer = None;
            }
            _ => {} // `Flags`/`Damage`/`LinuxDmabuf` — see the module section comment
        }
    }
}

// ── Fractional scale ─────────────────────────────────────────────────────────────────────
//
// `wp_fractional_scale_v1` reports a *logical* scale factor (`PreferredScale`,
// numerator/120); `wp_viewporter` is the other half — it's what actually lets a client
// render its buffer at a *different* pixel size than the surface's own logical size and
// have the compositor scale between them (`WpViewport::set_destination`).
//
// **`gpu.width`/`gpu.height` stay in points, unchanged from every prior phase of this
// port.** `layer_shell`'s `configure()` reports logical (point) size, Wayland pointer
// coordinates arrive in the same logical units, and `hakai_core::damage::DamageLayer` — the
// module actually responsible for where a stamp/tile/decal lands — is built to take points
// too (`scale` is purely an internal detail it uses to size its own tile *textures*).
// Keeping `gpu.width`/`gpu.height`/mouse position/`DamageLayer` all in points means none of
// that code needs to know a fractional scale exists at all. The only place scale actually
// has to be multiplied in is the one spot that genuinely is pixels: the literal
// `width`/`height` fields of `wgpu::SurfaceConfiguration`, computed locally as
// `buffer_width`/`buffer_height` right before each `configure()` call and never stored,
// since a real pixel buffer is what makes the render crisp on a >1x output.
//
// This falls out of NDC being a scale-invariant *ratio* (`origin / screen_size`) rather
// than an absolute unit: as long as an origin and the screen size it's divided by are both
// in the same unit — both points, here — the resulting clip-space position comes out
// identical to computing it in pixels and dividing by the pixel screen size instead. So
// nothing downstream of `tile_ndc`/`rotated_sprite_ndc` needs to change for scale at all;
// only the *texture resolution* a tile is rasterized at (`side_px`, pixels) benefits from
// knowing the scale, to avoid a blurry upscale on a >1x output.

impl Dispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, ()> for State {
    fn event(_: &mut Self, _: &wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, _: wp_fractional_scale_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        // Request-only interface — no events.
    }
}

impl Dispatch<wp_viewporter::WpViewporter, ()> for State {
    fn event(_: &mut Self, _: &wp_viewporter::WpViewporter, _: wp_viewporter::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        // Request-only interface — no events.
    }
}

impl Dispatch<wp_viewport::WpViewport, ()> for State {
    fn event(_: &mut Self, _: &wp_viewport::WpViewport, _: wp_viewport::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        // Request-only interface — no events.
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let wp_fractional_scale_v1::Event::PreferredScale { scale } = event else { return };
        let Some(index) = state.layers.iter().position(|l| l.fractional_scale.as_ref() == Some(proxy)) else { return };

        state.layers[index].scale = scale as f32 / 120.0;
        log::info!("output scale changed to {:.3}", state.layers[index].scale);

        // A pure scale change (`gpu.width`/`gpu.height`, points, are untouched by it —
        // see the module comment above) still needs: (1) the wgpu surface reconfigured at
        // the new pixel buffer size, so the render stays crisp, and (2) the damage/tile
        // grid rebuilt, so each tile's *texture* gets rasterized at the new pixel
        // resolution rather than staying blurry (upscaled) or wasteful (still
        // high-resolution after a scale-down). NDC placement itself doesn't need to
        // change at all (see the module comment), which is why this doesn't duplicate
        // `configure()`'s size-change branch outright — `WpViewport::set_destination`
        // also doesn't need re-calling here, since the destination stays this surface's
        // points size, which a scale-only event never changes.
        //
        // Duplicated rather than factored into a shared method, to avoid restructuring
        // `configure()`'s already-large, `&mut GpuLayer`-reference-based body just to
        // share this much smaller handler's logic. If it never runs (the `else { return
        // }` below), that's because this fired before `self.pipelines`/`self.sampler`
        // exist yet — shouldn't be possible in practice (a surface has to already be
        // mapped, hence configured once, before it can report a preferred scale at all),
        // but if it somehow did, the *next* `configure()` call still picks up the
        // already-updated `scale` above. `_queue` isn't used directly below
        // (`create_tile_gpu` doesn't need it) — kept in this guard anyway as part of the
        // same "is wgpu actually set up yet" readiness check the other four fields are.
        let (Some(device), Some(_queue), Some(adapter), Some(pipelines), Some(sampler)) =
            (&state.device, &state.queue, &state.adapter, &state.pipelines, &state.sampler)
        else {
            return;
        };
        let gpu = &mut state.layers[index];

        let buffer_width = ((gpu.width as f32) * gpu.scale).round().max(1.0) as u32;
        let buffer_height = ((gpu.height as f32) * gpu.scale).round().max(1.0) as u32;
        let caps = gpu.wgpu_surface.get_capabilities(adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        gpu.wgpu_surface.configure(
            device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: buffer_width,
                height: buffer_height,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: caps.alpha_modes.iter().copied().find(|m| *m == wgpu::CompositeAlphaMode::PreMultiplied).unwrap_or(caps.alpha_modes[0]),
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );

        let mut damage = DamageLayer::new(gpu.width as f32, gpu.height as f32, gpu.scale);
        damage.mark_all_dirty();
        let (cols, rows) = damage.grid_size();
        // `side_px` (pixels) sizes each tile's GPU texture at the new scale; the NDC
        // placement stays in points — see this section's module comment.
        let side_px = (DamageLayer::TILE_SIDE * damage.scale()) as u32;
        let mut tiles = Vec::with_capacity(cols * rows);
        for i in 0..(cols * rows) {
            let origin = damage.tile_origin(i);
            let ndc = tile_ndc(origin, (DamageLayer::TILE_SIDE, DamageLayer::TILE_SIDE), (gpu.width as f32, gpu.height as f32));
            tiles.push(create_tile_gpu(device, &pipelines.tile_bind_group_layout, sampler, side_px, ndc));
        }
        gpu.damage = Some(damage);
        gpu.tiles = tiles;
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(State);
delegate_output!(State);
delegate_seat!(State);
delegate_keyboard!(State);
delegate_pointer!(State);
delegate_layer!(State);
delegate_shm!(State);
delegate_registry!(State);
