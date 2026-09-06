//! Hakai on Wayland — the smithay-client-toolkit shell.
//!
//! Everything below the compositor edge — the renderer, the per-output scene state, all
//! nine tools, particles, the termite colony, the HUD — lives in `hakai_core::render`
//! (`Scene` / `GpuLayer` / `Assets` / `render`), the same code `hakai-win` runs. This file
//! is the Wayland half: `wl_compositor` / `zwlr_layer_shell_v1` surfaces (one `WlLayer`
//! per `wl_output`, index-aligned with `scene.layers`), keyboard/pointer routed into the
//! `Scene`, `zwlr_screencopy_v1` feeding the brightness map, and `wp_fractional_scale_v1`
//! + `wp_viewporter` for crisp rendering on a non-integer output scale.
//!
//! The wgpu surface and the `Scene` are built lazily in `spawn_layer_for_output`, on the
//! first output — a device/format needs a real surface first. Esc quits.

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

use std::time::Instant;

use hakai_core::audio::AudioSink;
use hakai_core::icons::ToolIcons;
use hakai_core::render::text::TextRenderer;
use hakai_core::render::{Assets, Scene};
use hakai_core::sprites::SpriteFactory;
use hakai_core::tools::ToolId;
use hakai_core::{capture, DecalFactory};

mod audio;
mod theme;

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
    // wgpu 30: `InstanceDescriptor` is no longer `Default` — build it from the
    // env-aware constructor, then pin the backend (same pattern as `hakai-win`).
    let instance = wgpu::Instance::new({
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        desc.backends = wgpu::Backends::VULKAN;
        desc
    });

    // Audio is optional — `AudioEngine.swift`'s own doc comment: "if `AVAudioEngine`
    // fails to start, the app carries on normally." `AudioSink::new()` (no backend) is
    // the same genuine no-op it's always been, so a missing/failed audio device costs
    // nothing but sound. Held on `State` only until the first output's surface exists and
    // the `Scene` is built, which takes ownership of it.
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

    // Phase 7: the active Omarchy theme's paint palette + HUD chrome colours — read once,
    // here, and stashed until `Scene` is built (both feed into it). All-or-nothing: a
    // missing resolver or a malformed value falls back to the built-in defaults wholesale.
    let paint_colors = theme::read_paint_colors();
    log::info!(
        "paint palette: {}",
        if paint_colors.is_some() { "sourced from the active Omarchy theme" } else { "built-in default (no Omarchy theme found)" }
    );
    let hud_colors = match theme::read_hud_colors() {
        Some(c) => {
            log::info!("HUD colours: sourced from the active Omarchy theme");
            c
        }
        None => {
            log::info!("HUD colours: built-in default");
            theme::HudColors::FALLBACK
        }
    };

    let mut state = State {
        registry_state: RegistryState::new(&globals),
        output_state,
        compositor_state,
        layer_shell,
        seat_state,
        keyboard: None,
        pointer: None,
        pointer_focus: None,
        instance,
        adapter: None,
        scene: None,
        audio: Some(audio_sink),
        paint_colors,
        hud_colors,
        screencopy_manager,
        shm,
        shm_pool,
        fractional_scale_manager,
        viewporter,
        wl: Vec::new(),
        exit: false,
        qh: qh.clone(),
        conn: conn.clone(),
    };

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

    log::info!("hakai: {} layer surface(s) up — Esc to quit", state.wl.len());

    while !state.exit {
        event_loop
            .dispatch(std::time::Duration::from_millis(16), &mut state)
            .expect("event loop dispatch failed");
    }

    log::info!("exiting — dropping every layer surface");
}

// ── Per-output layer surface + its wgpu surface ─────────────────────────────────────────

struct WlLayer {
    /// The wlr layer surface covering this output. Its `wl_surface` is this layer's
    /// identity everywhere else in this file.
    layer: LayerSurface,
    /// This output's `wl_output` — needed only to request a capture of it
    /// (`zwlr_screencopy_manager_v1::capture_output`).
    output: wl_output::WlOutput,

    /// `Some` exactly while a `zwlr_screencopy_v1` request/event dance is in flight for
    /// this output. The three `capture_*` fields together are the per-output capture state,
    /// inferred from which are `Some`/`None` rather than a separate enum.
    capture_frame: Option<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1>,
    /// `(format, width, height, stride)` from the frame's own `buffer` event.
    capture_buffer_info: Option<(wl_shm::Format, u32, u32, u32)>,
    /// The SHM buffer the compositor writes the capture into.
    capture_buffer: Option<smithay_client_toolkit::shm::slot::Buffer>,
    /// When the last capture *completed* (successful or failed) — capture scheduling reads
    /// wall-clock elapsed against this rather than accumulating `dt`, so a request that
    /// never resolves can't wedge future captures.
    last_capture: Option<Instant>,

    /// `wp_fractional_scale_v1` object for this output (`None` if the global was missing) —
    /// its `PreferredScale` events drive `scene.layers[i].scale`.
    fractional_scale: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    /// `wp_viewporter` viewport — `set_destination` maps the oversized (points × scale)
    /// buffer back down to the surface's logical size. `None` if the global was missing.
    viewport: Option<wp_viewport::WpViewport>,
}

struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    layer_shell: LayerShell,
    seat_state: SeatState,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    /// Which output's surface the pointer is currently over — Wayland gives pointer focus
    /// to one surface at a time, unlike the exclusive keyboard grab every surface holds.
    pointer_focus: Option<wl_surface::WlSurface>,

    /// Explicitly Vulkan, not `Instance::default()`'s auto-selection — that fell back to
    /// the GLES/EGL backend on at least one real run and failed `eglCreateContext` outright.
    instance: wgpu::Instance,
    /// `None` until the first output's surface exists — created from it, then shared.
    adapter: Option<wgpu::Adapter>,
    /// The shared renderer + scene. `None` until the first output's layer surface is up
    /// (it needs a wgpu surface to get a device/format from); built once in
    /// `spawn_layer_for_output`, then every output's `GpuLayer` lives in `scene.layers`,
    /// index-aligned with `self.wl`.
    scene: Option<Scene>,
    /// Held only until `Scene` is built, which takes ownership.
    audio: Option<AudioSink>,
    /// The active Omarchy theme's paint palette (`None` = built-in default), stashed until
    /// `Scene` is built.
    paint_colors: Option<[(f32, f32, f32); 8]>,
    /// The active theme's HUD chrome colours, stashed until `Scene` is built.
    hud_colors: hakai_core::render::HudColors,

    /// `None` when `zwlr_screencopy_manager_v1` isn't advertised — every capture attempt is
    /// then skipped and every tool falls back to a random impact-sound variant.
    screencopy_manager: Option<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1>,
    shm: Option<Shm>,
    /// One shared pool for every output's capture buffers — sized generously once.
    shm_pool: Option<SlotPool>,

    /// `None` if `wp_fractional_scale_v1` isn't advertised — every output then stays at
    /// `scale: 1.0`, already correct for the (common) integer-scaled case.
    fractional_scale_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,

    /// One per output, index-aligned with `scene.layers`. The Wayland edge of a layer;
    /// its scene state is `scene.layers[i]`.
    wl: Vec<WlLayer>,
    exit: bool,
    qh: QueueHandle<Self>,
    conn: Connection,
}

impl State {
    /// Creates the wlr layer surface for `output`, its wgpu surface, and — on the first
    /// call — the shared `Scene` (which needs a device/format the first surface provides).
    /// Idempotent: the startup loop and `OutputHandler::new_output` can both reach here.
    fn spawn_layer_for_output(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if self.wl.iter().any(|w| w.output == output) {
            return;
        }

        let surface = self.compositor_state.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh, surface, Layer::Overlay, Some("hakai"), Some(&output),
        );
        // Cover the whole output, sit above everything, take the keyboard exclusively so
        // Esc always reaches us. Real size arrives in the first `configure`.
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.set_size(0, 0);
        layer.commit();

        let fractional_scale = self
            .fractional_scale_manager
            .as_ref()
            .map(|m| m.get_fractional_scale(layer.wl_surface(), qh, ()));
        let viewport = self.viewporter.as_ref().map(|v| v.get_viewport(layer.wl_surface(), qh, ()));

        // wgpu wants HasWindowHandle + HasDisplayHandle; Wayland's side of that is just the
        // wl_display / wl_surface pointers libwayland-client sees. See PHASE0.md.
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

        if self.scene.is_none() {
            let adapter = pollster::block_on(self.instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&wgpu_surface),
                force_fallback_adapter: false,
                ..Default::default()
            }))
            .expect("no wgpu adapter compatible with the layer surface");

            // `adapter.limits()` (not the WebGL2 downlevel profile): a wgpu surface buffer
            // is one texture, and any 1440p/4K output at >=1x scale exceeds the downlevel
            // 2048 cap and fails `Surface::configure` outright. Real desktop GPUs report
            // 8192/16384 here, which is what `request_adapter` already confirmed we can ask.
            let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("hakai"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            }))
            .expect("failed to acquire a wgpu device");

            let caps = wgpu_surface.get_capabilities(&adapter);
            let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);

            let mut decals = DecalFactory::new();
            if let Some(colors) = self.paint_colors {
                decals.set_paint_colors(colors);
            }
            let mut icons = ToolIcons::new();
            let mut sprites = SpriteFactory::new();
            let mut text = TextRenderer::new();
            // `build_credits_pixmap` clamps its layout to ~960px, so this default width
            // produces an identical panel on any ordinary monitor — the real per-output
            // size isn't known until the first `configure`.
            let assets = Assets::build(
                device, queue, format, &mut icons, &mut sprites, &mut decals, &mut text, self.hud_colors, 1920.0,
            );
            let audio = self.audio.take().unwrap_or_default();
            let mut scene = Scene {
                assets, decals, icons, sprites, text, audio,
                layers: Vec::new(),
                focused_layer: None,
                shift_held: false,
            };
            scene.select_tool(ToolId::Hammer);
            self.scene = Some(scene);
            self.adapter = Some(adapter);
        }

        let adapter = self.adapter.as_ref().unwrap();
        let scene = self.scene.as_mut().unwrap();
        // Size 0x0 until the first `configure` — `add_layer` leaves it unconfigured and
        // `LayerShellHandler::configure` calls `resize_layer` with the real size.
        scene.add_layer(wgpu_surface, adapter, (0, 0), 1.0);

        self.wl.push(WlLayer {
            layer,
            output,
            capture_frame: None,
            capture_buffer_info: None,
            capture_buffer: None,
            last_capture: None,
            fractional_scale,
            viewport,
        });
    }

    /// Which `self.wl` / `scene.layers` index this layer surface is, by its `wl_surface`.
    fn layer_index(&self, surface: &wl_surface::WlSurface) -> Option<usize> {
        self.wl.iter().position(|w| w.layer.wl_surface() == surface)
    }

    /// Kicks off a fresh `zwlr_screencopy_v1` capture for output `index`, if one's due
    /// (`CAPTURE_INTERVAL` elapsed, or a just-frozen output still has no snapshot) and
    /// nothing's already in flight for it.
    fn maybe_start_capture(&mut self, index: usize) {
        let Some(manager) = self.screencopy_manager.clone() else { return };
        let Some(wl) = self.wl.get(index) else { return };
        if wl.capture_frame.is_some() {
            return;
        }
        let (frozen, has_snapshot) = self
            .scene
            .as_ref()
            .and_then(|s| s.layers.get(index))
            .map(|l| (l.frozen, l.snapshot_texture.is_some()))
            .unwrap_or((false, false));
        let due = if frozen {
            !has_snapshot
        } else {
            match self.wl[index].last_capture {
                None => true,
                Some(t) => t.elapsed().as_secs_f32() >= capture::CAPTURE_INTERVAL,
            }
        };
        if !due {
            return;
        }
        let frame = manager.capture_output(0, &self.wl[index].output, &self.qh, ());
        self.wl[index].capture_frame = Some(frame);
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
        let Some(index) = self.layer_index(surface) else { return };
        // `&mut self` — must run before `self.scene` is borrowed just below.
        self.maybe_start_capture(index);

        let show_cursor = self.pointer_focus.as_ref() == Some(surface);
        // Only the first output drives the shared audio gain-glide — see AudioEngine.swift.
        let drive_audio = index == 0;
        let Some(scene) = self.scene.as_mut() else { return };
        if !scene.layers.get(index).map(|l| l.configured).unwrap_or(false) {
            return;
        }
        scene.tick_layer(index, drive_audio, show_cursor);

        // Re-arm this output's frame callback and commit the surface.
        let wl_surface = self.wl[index].layer.wl_surface().clone();
        wl_surface.frame(&self.qh, surface.clone());
        wl_surface.commit();
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
        if let Some(index) = self.wl.iter().position(|w| &w.layer == layer) {
            self.wl.remove(index);
            if let Some(scene) = self.scene.as_mut() {
                scene.remove_layer(index);
            }
        }
        if self.wl.is_empty() {
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
        let Some(index) = self.wl.iter().position(|w| &w.layer == layer) else { return };
        log::debug!("configure: layer {index}, new_size {:?}", configure.new_size);

        // `configure.new_size` is in *points* — the scene keeps `width`/`height` in points
        // too (matching mouse / `DamageLayer` coords). Only the wgpu surface's own pixel
        // buffer (points × scale, computed inside `resize_layer`) needs `scale` at all.
        let (w, h) = configure.new_size;
        let (w, h) = (w.max(1), h.max(1));
        let scale = self
            .scene
            .as_ref()
            .and_then(|s| s.layers.get(index))
            .map(|l| l.scale)
            .unwrap_or(1.0);

        // Wayland's half of fractional scale: display the oversized buffer scaled back
        // down to the surface's logical (point) size.
        if let Some(viewport) = &self.wl[index].viewport {
            viewport.set_destination(w as i32, h as i32);
        }

        let wl_surface = self.wl[index].layer.wl_surface().clone();
        let show_cursor = self.pointer_focus.as_ref() == Some(&wl_surface);
        let Some(scene) = self.scene.as_mut() else { return };
        scene.resize_layer(index, (w, h), scale);

        // Prime the redraw loop: request the next frame callback and render once. Without
        // this first frame + callback request, nothing ever asks the compositor for a
        // frame callback, so `CompositorHandler::frame` never fires and every later change
        // is marked dirty but never drawn.
        wl_surface.frame(&self.qh, wl_surface.clone());
        scene.tick_layer(index, index == 0, show_cursor);
        wl_surface.commit();
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
        // Bound by keysym, not scancode — matches the plan's non-QWERTY note.
        log::debug!("key pressed: {:?}", event.keysym);
        if event.keysym == Keysym::Escape {
            log::info!("Esc pressed — quitting");
            self.exit = true;
            return;
        }
        let Some(scene) = self.scene.as_mut() else { return };
        match event.keysym {
            Keysym::_1 => scene.select_tool(ToolId::Hammer),
            Keysym::_2 => scene.select_tool(ToolId::ChainSaw),
            Keysym::_3 => scene.select_tool(ToolId::MachineGun),
            Keysym::_4 => scene.select_tool(ToolId::FlameThrower),
            Keysym::_5 => scene.select_tool(ToolId::ColorThrower),
            Keysym::_6 => scene.select_tool(ToolId::Phaser),
            Keysym::_7 => scene.select_tool(ToolId::Stamp),
            Keysym::_8 => scene.select_tool(ToolId::Termites),
            Keysym::_9 => scene.select_tool(ToolId::Washer),
            Keysym::r | Keysym::R => scene.erase_all(),
            // XKB remaps Shift+Tab to `ISO_Left_Tab` rather than `Tab`+shift.
            Keysym::Tab => scene.cycle_tool(if scene.shift_held { -1 } else { 1 }),
            Keysym::ISO_Left_Tab => scene.cycle_tool(-1),
            Keysym::Up => scene.set_palette_visible(true),
            Keysym::Down => scene.set_palette_visible(false),
            Keysym::c | Keysym::C => scene.toggle_credits(),
            Keysym::m | Keysym::M => scene.toggle_mode(),
            _ => {}
        }
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: smithay_client_toolkit::seat::keyboard::Modifiers, _: u32) {
        // RISK: `Modifiers::shift` is my best-confidence reading of this struct's field
        // name — if it doesn't compile, this is the one line to fix; `Modifiers` is
        // otherwise not used anywhere else in this file.
        if let Some(scene) = self.scene.as_mut() { scene.shift_held = modifiers.shift; }
    }
}

/// The Linux input-event code for the left mouse button — same value AppKit implicitly
/// used for `mouseDown` in the macOS build; Wayland just makes it an explicit number.
const BTN_LEFT: u32 = 0x110;

impl PointerHandler for State {
    fn pointer_frame(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, pointer: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            let surface = event.surface.clone();
            // Wayland reports pointer positions in a surface's logical units (points) — the
            // scene's working unit — so no `scale` multiply here.
            let point = (event.position.0 as f32, event.position.1 as f32);
            log::debug!("pointer event: {:?} @ {:?}", event.kind, point);

            let Some(index) = self.layer_index(&surface) else { continue };
            let Some(scene) = self.scene.as_mut() else { continue };

            match event.kind {
                PointerEventKind::Enter { serial } => {
                    self.pointer_focus = Some(surface.clone());
                    // Hide the compositor's own cursor — this overlay draws its own.
                    pointer.set_cursor(serial, None, 0, 0);
                    // Place the tool cursor under the pointer immediately, not on the next
                    // Motion.
                    if let Some(l) = scene.layers.get_mut(index) {
                        l.mouse = point;
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if self.pointer_focus.as_ref() == Some(&surface) {
                        self.pointer_focus = None;
                    }
                }
                PointerEventKind::Motion { .. } => {
                    scene.pointer_moved(index, point);
                }
                PointerEventKind::Press { button, .. } => {
                    if button != BTN_LEFT {
                        continue;
                    }
                    scene.pointer_pressed(index, point);
                }
                PointerEventKind::Release { button, .. } => {
                    if button != BTN_LEFT {
                        continue;
                    }
                    scene.pointer_released(index, point);
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
        // The frame object identifies which output started it, by matching `capture_frame`.
        let Some(index) = state.wl.iter().position(|w| w.capture_frame.as_ref() == Some(proxy)) else { return };

        match event {
            zwlr_screencopy_frame_v1::Event::Buffer { format, width, height, stride } => {
                // A frame can send several `Buffer` events (format/size options); keep the
                // last — a brightness sample doesn't care which SHM format it reads.
                if let Ok(format) = format.into_result() {
                    state.wl[index].capture_buffer_info = Some((format, width, height, stride));
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                let Some((format, width, height, stride)) = state.wl[index].capture_buffer_info else { return };
                let Some(pool) = state.shm_pool.as_mut() else { return };
                match pool.create_buffer(width as i32, height as i32, stride as i32, format) {
                    Ok((buffer, _canvas)) => {
                        proxy.copy(buffer.wl_buffer());
                        state.wl[index].capture_buffer = Some(buffer);
                    }
                    Err(e) => {
                        log::warn!("failed to create an SHM buffer for screen capture: {e}");
                        state.wl[index].capture_frame = None;
                        state.wl[index].capture_buffer_info = None;
                    }
                }
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                if let (Some(pool), Some(buffer), Some((_, width, height, stride))) = (
                    state.shm_pool.as_mut(),
                    state.wl[index].capture_buffer.as_ref(),
                    state.wl[index].capture_buffer_info,
                ) {
                    if let Some(bytes) = buffer.canvas(pool) {
                        if let Some(scene) = state.scene.as_mut() {
                            scene.feed_layer_capture(index, bytes, width, height, stride);
                        }
                    }
                }
                state.wl[index].capture_frame = None;
                state.wl[index].capture_buffer_info = None;
                state.wl[index].capture_buffer = None;
                state.wl[index].last_capture = Some(Instant::now());
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                log::debug!("screen capture failed for one output — retry in {}s", capture::CAPTURE_INTERVAL);
                state.wl[index].capture_frame = None;
                state.wl[index].capture_buffer_info = None;
                state.wl[index].capture_buffer = None;
                state.wl[index].last_capture = Some(Instant::now());
            }
            _ => {}
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
        let Some(index) = state.wl.iter().position(|w| w.fractional_scale.as_ref() == Some(proxy)) else { return };
        let new_scale = scale as f32 / 120.0;
        log::info!("output {index} scale -> {new_scale:.3}");

        let Some(scene) = state.scene.as_mut() else { return };
        let Some(layer) = scene.layers.get_mut(index) else { return };
        if !layer.configured {
            // Before the first `configure` — just record it; `configure` rebuilds at the
            // real size using this scale. The viewport destination (points) is unchanged
            // by a scale-only event, so no `set_destination` here.
            layer.scale = new_scale;
            return;
        }
        let (w, h) = (layer.width, layer.height);
        scene.resize_layer(index, (w, h), new_scale);
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
