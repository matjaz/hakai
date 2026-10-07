//! The winit shell — the event loop, windows and input every winit-based binary shares.
//!
//! `hakai-win` and `hakai-mac` were the same ~350-line `main.rs` apart from a handful of
//! platform hooks, and the Linux binary's GNOME/X11 fallback would have been a third copy.
//! This is that `main.rs`, once: one borderless transparent window per monitor, a wgpu
//! device, the [`Scene`], keyboard and pointer routing, per-monitor scale factors, and the
//! periodic desktop capture. What genuinely differs per platform comes in through
//! [`Platform`]:
//!
//! - **Windows** — the DX12/DirectComposition instance, `WS_EX_NOREDIRECTIONBITMAP`, DXGI
//!   Desktop Duplication, the global Esc hotkey.
//! - **macOS** — the Metal instance, the screensaver window level, CoreGraphics capture,
//!   ⌘Q.
//! - **Linux (no wlr-layer-shell: GNOME, X11)** — Vulkan/GL, true fullscreen windows,
//!   no capture.
//!
//! Gated behind the crate's `shell` feature (which implies `render`).

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, Touch, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy, OwnedDisplayHandle};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId, WindowLevel};

pub use winit;

use crate::audio::AudioSink;
use crate::icons::ToolIcons;
use crate::render::text::TextRenderer;
use crate::render::{hud_bar_at, palette_tool_at, Assets, HudColors, HudText, Scene};
use crate::sprites::SpriteFactory;
use crate::tools::ToolId;
use crate::DecalFactory;

/// A desktop-capture backend: hands the newest frame, if there is one, to `f` as a strided
/// BGRA buffer `(bytes, width, height, stride)` and returns whether it did.
pub trait DesktopCapture {
    fn capture(&mut self, f: &mut dyn FnMut(&[u8], u32, u32, u32)) -> bool;
}

/// How each monitor's window covers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// Explicit position + size, no fullscreen state (Windows, macOS — fullscreen there
    /// means exclusive mode or a new Space, both the opposite of an overlay).
    Bounds,
    /// `Fullscreen::Borderless(monitor)` — the only way to cover the panel/top bar for an
    /// ordinary window on GNOME and most X11 window managers.
    Fullscreen,
}

/// Lets a platform end the event loop from another thread (Windows' global Esc hotkey).
#[derive(Clone)]
pub struct QuitHandle(EventLoopProxy<ShellEvent>);

impl QuitHandle {
    pub fn quit(&self) {
        let _ = self.0.send_event(ShellEvent::Quit);
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ShellEvent {
    Quit,
}

/// The platform hooks. Every method but [`Platform::instance`] and [`Platform::audio`] has
/// a do-nothing default.
pub trait Platform: 'static {
    /// Used in log lines and the wgpu device label.
    fn name(&self) -> &'static str;

    /// The wgpu instance — which backends, and any backend options. `display` is the
    /// event loop's display connection, which wgpu's GL backend needs up front (EGL on
    /// X11/Wayland); Vulkan, DX12 and Metal don't.
    fn instance(&self, display: OwnedDisplayHandle) -> wgpu::Instance;

    /// The audio sink, backend already attached (or silent).
    fn audio(&mut self) -> AudioSink;

    fn coverage(&self) -> Coverage {
        Coverage::Bounds
    }

    /// Adjusts the shared window attributes (transparent, undecorated, always on top, sized
    /// to the monitor) before the window is created.
    fn window_attributes(&self, attrs: WindowAttributes) -> WindowAttributes {
        attrs
    }

    /// Right after a window is created, before its wgpu surface is — for native styling
    /// winit doesn't expose. `monitor` is `None` in windowed mode.
    fn window_created(&mut self, _window: &Window, _monitor: Option<&MonitorHandle>, _windowed: bool) {}

    /// Once every window and the scene exist.
    fn started(&mut self, _windows: &[Arc<Window>], _windowed: bool) {}

    /// The desktop-capture backend for the brightness map and frozen mode, if there is
    /// one. `windows[0]` covers the primary monitor.
    fn capture(&mut self, _windows: &[Arc<Window>]) -> Option<Box<dyn DesktopCapture>> {
        None
    }

    /// Before the event loop starts — e.g. to hand a [`QuitHandle`] to a hotkey thread.
    fn event_loop_ready(&mut self, _quit: QuitHandle) {}

    /// The paint palette, in `DecalFactory::DEFAULT_PAINT_COLORS` order. `None` keeps it.
    fn paint_colors(&self) -> Option<[(f32, f32, f32); 8]> {
        None
    }

    /// The HUD colours. `None` keeps `HudColors::FALLBACK`.
    fn hud_colors(&self) -> Option<HudColors> {
        None
    }

    /// Where the pointer is right now, in `window`'s physical pixels from its top-left, or
    /// `None` if it isn't over `window` (or the platform can't tell). Asked once at
    /// startup: a window that appears under a pointer that isn't moving gets no motion
    /// event, so without this the tool cursor would wait for the first nudge.
    fn cursor_position(&self, _window: &Window) -> Option<(f64, f64)> {
        None
    }

    /// A platform quit chord on top of Esc (⌘Q on macOS, Back on Android).
    fn is_quit_key(&self, _code: KeyCode, _modifiers: ModifiersState) -> bool {
        false
    }

    /// The HUD's how-to text. Keyboard platforms keep the key list; touch-only ones explain
    /// the tap/long-press/drag controls instead.
    fn hud_text(&self) -> HudText {
        HudText::KEYBOARD
    }

    /// Adjusts the event loop before it's built (Android hands it the `AndroidApp`).
    fn configure_event_loop(&mut self, _builder: &mut EventLoopBuilder<ShellEvent>) {}

    /// Quit when the app is sent to the background (Android: its window and surfaces are
    /// gone until it's resumed, and a fresh start is simpler than rebuilding them).
    fn quit_when_suspended(&self) -> bool {
        false
    }
}

/// How long a press on the status bar has to last to count as a long press (credits).
const LONG_PRESS: f32 = 0.6;
/// Below this much finger travel (points), a touch on the credits is a tap, not a drag.
const TAP_SLOP: f32 = 10.0;
/// Points the credits scroll per wheel notch / arrow key.
const SCROLL_STEP: f32 = 48.0;

/// Minimum horizontal travel (points) for a swipe to switch tools, and how much more
/// horizontal than vertical it has to be.
const SWIPE_MIN: f32 = 60.0;
const SWIPE_DOMINANCE: f32 = 1.5;

/// The fingers on the screen and what they're doing together. The first finger drives
/// everything a single pointer would; a second one turns the gesture into a tool-switching
/// swipe; further fingers are ignored.
struct TouchState {
    /// The layer the gesture started on.
    idx: usize,
    role: TouchRole,
    /// (id, where it went down, where it is now) — the first entry is the primary finger.
    fingers: Vec<(u64, (f32, f32), (f32, f32))>,
}

impl TouchState {
    /// The average displacement of the fingers still down — a two-finger swipe's motion.
    fn mean_travel(&self) -> (f32, f32) {
        let n = self.fingers.len().max(1) as f32;
        let (dx, dy) = self.fingers.iter().fold((0.0, 0.0), |acc, (_, start, now)| {
            (acc.0 + now.0 - start.0, acc.1 + now.1 - start.1)
        });
        (dx / n, dy / n)
    }
}

/// `travel` as a tool step: a swipe left is the next tool, right the previous one (like
/// turning pages) — `None` unless it's long enough and clearly horizontal.
fn swipe_step(travel: (f32, f32)) -> Option<i32> {
    let (dx, dy) = travel;
    (dx.abs() >= SWIPE_MIN && dx.abs() >= SWIPE_DOMINANCE * dy.abs()).then_some(if dx < 0.0 { 1 } else { -1 })
}

/// What the gesture is doing.
#[derive(Clone, Copy)]
enum TouchRole {
    /// Using the active tool — press, drag, release, like the left mouse button.
    Tool,
    /// Held on the status bar: a swipe switches tools, a tap opens the palette, a long
    /// press the credits.
    Bar(Instant),
    /// Two fingers down: decided when the first of them lifts — a horizontal swipe
    /// switches tools, anything else does nothing. Tools never see this gesture, so the
    /// one-finger drag stays free for drawing.
    Swipe,
    /// On the open credits panel: a drag scrolls it, a tap closes it. `last_y` is the
    /// finger's previous height, `travel` how far it has moved in total.
    Credits { last_y: f32, travel: f32 },
    /// Already handled on touch-down (closing a panel, picking a tool) — swallow the rest.
    Consumed,
}

/// Runs the shell until the user quits. `HAKAI_WINDOWED=1` gives a single ordinary window
/// instead of the overlay.
pub fn run(mut platform: impl Platform) {
    let mut builder = EventLoop::<ShellEvent>::with_user_event();
    platform.configure_event_loop(&mut builder);
    let event_loop = match builder.build() {
        Ok(event_loop) => event_loop,
        Err(e) => {
            // On Linux: neither a Wayland nor an X11 display to open (a TTY, SSH).
            eprintln!("hakai: couldn't open a window ({e}) — it needs a graphical session.");
            std::process::exit(1);
        }
    };
    event_loop.set_control_flow(ControlFlow::Poll);
    platform.event_loop_ready(QuitHandle(event_loop.create_proxy()));
    let mut app = App {
        platform,
        windows: Vec::new(),
        window_ids: Vec::new(),
        scene: None,
        capture: None,
        last_capture: None,
        modifiers: ModifiersState::default(),
        cursor: (0.0, 0.0),
        touch: None,
        _gpu: None,
    };
    event_loop.run_app(&mut app).expect("run_app");
}

/// winit's `inner_size()` is physical pixels; the scene works in points. One divide, here.
fn logical_size(w: &Window) -> (u32, u32) {
    let scale = (w.scale_factor() as f32).max(0.01);
    let size = w.inner_size();
    (((size.width as f32 / scale).round() as u32).max(1), ((size.height as f32 / scale).round() as u32).max(1))
}

struct App<P: Platform> {
    platform: P,
    windows: Vec<Arc<Window>>,
    /// `WindowId` for each `scene.layers[i]`, same order.
    window_ids: Vec<WindowId>,
    scene: Option<Scene>,

    capture: Option<Box<dyn DesktopCapture>>,
    /// `None` until the first capture, so the brightness map is populated on frame 1.
    last_capture: Option<Instant>,

    modifiers: ModifiersState,
    /// Last cursor position, in the focused layer's point space.
    cursor: (f32, f32),
    /// The fingers currently down, if any.
    touch: Option<TouchState>,
    /// Kept alive for the surfaces made from them.
    _gpu: Option<(wgpu::Instance, wgpu::Adapter)>,
}

impl<P: Platform> App<P> {
    fn layer_index(&self, id: WindowId) -> Option<usize> {
        self.window_ids.iter().position(|w| *w == id)
    }

    fn overlay_attributes(&self, monitor: Option<&MonitorHandle>, windowed: bool) -> WindowAttributes {
        let mut attrs = Window::default_attributes()
            .with_title("Hakai")
            .with_transparent(true)
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_visible(true);
        if windowed {
            attrs = attrs.with_decorations(true).with_inner_size(winit::dpi::PhysicalSize::new(1100, 620));
        } else if let Some(m) = monitor {
            attrs = match self.platform.coverage() {
                Coverage::Bounds => attrs.with_position(m.position()).with_inner_size(m.size()),
                // Resizable, and already monitor-sized: on X11 a non-resizable window pins
                // min = max size in its WM hints, and window managers then refuse to
                // fullscreen it (openbox left it at winit's default 800×600).
                Coverage::Fullscreen => attrs
                    .with_resizable(true)
                    .with_position(m.position())
                    .with_inner_size(m.size())
                    .with_fullscreen(Some(Fullscreen::Borderless(Some(m.clone())))),
            };
        }
        self.platform.window_attributes(attrs)
    }

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
        let make_windows: Vec<Option<MonitorHandle>> =
            if windowed { vec![None] } else { monitors.iter().cloned().map(Some).collect() };

        for m in &make_windows {
            let attrs = self.overlay_attributes(m.as_ref(), windowed);
            let w = Arc::new(event_loop.create_window(attrs).expect("create_window"));
            // Platform styling first: on macOS it re-frames the window, which would reset
            // the cursor rects winit hides the cursor with.
            self.platform.window_created(&w, m.as_ref(), windowed);
            w.set_cursor_visible(false);
            log::info!("window {:?}: {:?} @ scale {}", w.id(), w.inner_size(), w.scale_factor());
            self.windows.push(w);
        }

        let instance = self.platform.instance(event_loop.owned_display_handle());
        let surface0 = instance.create_surface(self.windows[0].clone()).expect("create_surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface0),
            ..Default::default()
        }))
        .expect("no wgpu adapter compatible with the window");
        log::info!("adapter: {:?}", adapter.get_info());

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some(self.platform.name()),
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
        match self.platform.paint_colors() {
            Some(colors) => decals.set_paint_colors(colors),
            None => log::info!("paint palette: built-in default ({})", self.platform.name()),
        }
        let hud_colors = self.platform.hud_colors().unwrap_or(HudColors::FALLBACK);

        let mut icons = ToolIcons::new();
        let mut sprites = SpriteFactory::new();
        let mut text = TextRenderer::new();
        // The credits panel is built once; sizing it for the narrower side means it still
        // fits across when a phone is turned upright (it scrolls vertically anyway).
        let (w0, h0) = logical_size(&self.windows[0]);
        let w0_points = w0.min(h0) as f32;
        let assets = Assets::build(
            device,
            queue,
            format,
            &mut icons,
            &mut sprites,
            &mut decals,
            &mut text,
            hud_colors,
            w0_points,
            self.platform.hud_text(),
        );

        let audio = self.platform.audio();
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

        // Put the tool cursor under the real pointer from the first frame.
        for (i, w) in self.windows.iter().enumerate() {
            if let Some((x, y)) = self.platform.cursor_position(w) {
                let scale = scene.layers[i].scale.max(0.01);
                self.cursor = (x as f32 / scale, y as f32 / scale);
                scene.layers[i].mouse = self.cursor;
                scene.focused_layer = Some(i);
                log::info!("pointer starts at {:?} (points) on layer {i}", self.cursor);
                break;
            }
        }

        self.capture = self.platform.capture(&self.windows);
        self.platform.started(&self.windows, windowed);
        self._gpu = Some((instance, adapter));
        self.scene = Some(scene);
    }

    /// Touch input — the only input on a phone, and a bonus on touchscreen laptops. The
    /// first finger down acts as the left mouse button, plus the controls a keyboard would
    /// otherwise provide: tap the status bar for the palette, long-press it for credits,
    /// tap a palette tool to pick it, tap anywhere to close an open panel.
    fn touch_event(&mut self, idx: usize, touch: Touch) {
        let Some(scene) = self.scene.as_mut() else { return };
        let scale = scene.layers[idx].scale.max(0.01);
        let p = (touch.location.x as f32 / scale, touch.location.y as f32 / scale);

        match touch.phase {
            TouchPhase::Started => {
                if let Some(state) = self.touch.as_mut() {
                    // A second finger on a tool stroke or the status bar: it's a swipe. End
                    // the stroke where it is, so nothing more gets drawn.
                    let joins = state.idx == idx
                        && state.fingers.len() == 1
                        && matches!(state.role, TouchRole::Tool | TouchRole::Bar(_));
                    if joins {
                        if matches!(state.role, TouchRole::Tool) {
                            scene.pointer_released(idx, self.cursor);
                        }
                        state.role = TouchRole::Swipe;
                        state.fingers.push((touch.id, p, p));
                    }
                    return;
                }
                self.cursor = p;
                scene.focused_layer = Some(idx);
                let layer = &scene.layers[idx];
                let screen = (layer.width as f32, layer.height as f32);
                let role = if layer.hud.credits_open() {
                    TouchRole::Credits { last_y: p.1, travel: 0.0 }
                } else if layer.hud.palette_open() {
                    if let Some(tool) = palette_tool_at(p, screen) {
                        scene.select_tool(tool);
                    }
                    scene.set_palette_visible(false);
                    TouchRole::Consumed
                } else if hud_bar_at(p, screen) {
                    TouchRole::Bar(Instant::now())
                } else {
                    scene.pointer_moved(idx, p);
                    scene.pointer_pressed(idx, p);
                    TouchRole::Tool
                };
                self.touch = Some(TouchState { idx, role, fingers: vec![(touch.id, p, p)] });
            }
            TouchPhase::Moved => {
                let Some(state) = self.touch.as_mut() else { return };
                if state.idx != idx {
                    return;
                }
                let Some(finger) = state.fingers.iter().position(|f| f.0 == touch.id) else { return };
                state.fingers[finger].2 = p;
                if finger != 0 {
                    return;
                }
                match &mut state.role {
                    TouchRole::Tool => {
                        self.cursor = p;
                        scene.pointer_moved(idx, p);
                    }
                    // Content follows the finger: dragging up scrolls further down.
                    TouchRole::Credits { last_y, travel } => {
                        let delta = *last_y - p.1;
                        scene.scroll_credits(idx, delta);
                        *travel += delta.abs();
                        *last_y = p.1;
                    }
                    _ => {}
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let Some(state) = self.touch.as_mut() else { return };
                if state.idx != idx {
                    return;
                }
                let Some(finger) = state.fingers.iter().position(|f| f.0 == touch.id) else { return };
                state.fingers[finger].2 = p;
                match state.role {
                    TouchRole::Swipe => {
                        // Decided on the first lift; the other finger(s) just get swallowed.
                        if let Some(step) = swipe_step(state.mean_travel()) {
                            scene.cycle_tool(step);
                        }
                        state.role = TouchRole::Consumed;
                    }
                    _ if finger != 0 => {}
                    TouchRole::Tool => scene.pointer_released(idx, p),
                    TouchRole::Bar(since) => {
                        let (_, start, _) = state.fingers[0];
                        if let Some(step) = swipe_step((p.0 - start.0, p.1 - start.1)) {
                            scene.cycle_tool(step);
                        } else if since.elapsed().as_secs_f32() >= LONG_PRESS {
                            scene.toggle_credits();
                        } else {
                            scene.set_palette_visible(true);
                        }
                    }
                    TouchRole::Credits { travel, .. } if travel < TAP_SLOP => scene.toggle_credits(),
                    TouchRole::Credits { .. } | TouchRole::Consumed => {}
                }
                // A primary finger that's done ends its role; the rest of a swipe waits for
                // every finger to lift.
                if finger == 0 && !matches!(state.role, TouchRole::Consumed) {
                    state.role = TouchRole::Consumed;
                }
                state.fingers.remove(finger);
                if state.fingers.is_empty() {
                    self.touch = None;
                }
            }
        }
    }

    /// Requests a fresh desktop capture if one is due (every `CAPTURE_INTERVAL`, or the
    /// moment a frozen output still lacks its snapshot) and feeds it to every output.
    fn capture_tick(&mut self) {
        let (Some(capture), Some(scene)) = (self.capture.as_mut(), self.scene.as_mut()) else { return };
        let now = Instant::now();
        let due = match self.last_capture {
            None => true,
            Some(t) => {
                scene.wants_snapshot() || now.duration_since(t).as_secs_f32() >= crate::capture::CAPTURE_INTERVAL
            }
        };
        if !due {
            return;
        }
        self.last_capture = Some(now);
        capture.capture(&mut |bytes, w, h, stride| {
            scene.feed_capture_all(bytes, w, h, stride);
            log::trace!("desktop capture {w}x{h}");
        });
    }
}

impl<P: Platform> ApplicationHandler<ShellEvent> for App<P> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.windows.is_empty() {
            self.init(event_loop);
        }
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        if self.platform.quit_when_suspended() {
            event_loop.exit();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ShellEvent) {
        match event {
            ShellEvent::Quit => event_loop.exit(),
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
                // Android's Back button arrives with no physical key code, only this.
                if event.logical_key == Key::Named(NamedKey::BrowserBack) {
                    event_loop.exit();
                    return;
                }
                let PhysicalKey::Code(code) = event.physical_key else { return };
                if code == KeyCode::Escape || self.platform.is_quit_key(code, self.modifiers) {
                    event_loop.exit();
                    return;
                }
                let shift = self.modifiers.contains(ModifiersState::SHIFT);
                // With the credits open, the arrows and paging keys scroll them.
                if let Some(i) = idx.filter(|&i| scene.layers[i].hud.credits_open()) {
                    let page = scene.layers[i].height as f32 * 0.8;
                    let delta = match code {
                        KeyCode::ArrowUp => Some(-SCROLL_STEP),
                        KeyCode::ArrowDown => Some(SCROLL_STEP),
                        KeyCode::PageUp => Some(-page),
                        KeyCode::PageDown | KeyCode::Space => Some(page),
                        KeyCode::Home => Some(f32::MIN),
                        KeyCode::End => Some(f32::MAX),
                        _ => None,
                    };
                    if let Some(delta) = delta {
                        scene.scroll_credits(i, delta);
                        return;
                    }
                }
                match code {
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
                    // Shift+Tab arrives as Tab with the shift modifier on every winit
                    // backend — one branch covers both directions.
                    KeyCode::Tab => scene.cycle_tool(if shift { -1 } else { 1 }),
                    KeyCode::ArrowUp => scene.set_palette_visible(true),
                    KeyCode::ArrowDown => scene.set_palette_visible(false),
                    KeyCode::KeyC => scene.toggle_credits(),
                    KeyCode::KeyM => scene.toggle_mode(),
                    _ => {}
                }
            }

            // Entering doesn't show the tool cursor yet: winit's CursorEntered carries no
            // position, and drawing at the layer's last-known one put it at (0, 0) — the
            // top-left corner — until the first move. CursorMoved (which follows right
            // away when the pointer really moves in) is what focuses the layer.
            WindowEvent::CursorEntered { .. } => {}
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

            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(i) = idx.filter(|&i| scene.layers[i].hud.credits_open()) {
                    let points = match delta {
                        MouseScrollDelta::LineDelta(_, y) => -y * SCROLL_STEP,
                        MouseScrollDelta::PixelDelta(p) => -(p.y as f32) / scene.layers[i].scale.max(0.01),
                    };
                    scene.scroll_credits(i, points);
                }
            }

            WindowEvent::Touch(touch) => {
                if let Some(idx) = idx {
                    self.touch_event(idx, touch);
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
                let logical =
                    (((size.width as f32 / scale).round() as u32).max(1), ((size.height as f32 / scale).round() as u32).max(1));
                // Split borrow: `resize` needs `&layers[idx]` and `&assets` at once.
                let assets = &scene.assets;
                scene.layers[idx].resize(logical, scale, assets);
            }

            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_horizontal_swipe_steps_through_the_tools() {
        assert_eq!(swipe_step((-120.0, 10.0)), Some(1), "swiping left is the next tool");
        assert_eq!(swipe_step((120.0, -10.0)), Some(-1), "swiping right is the previous tool");
    }

    #[test]
    fn short_or_diagonal_movement_is_not_a_swipe() {
        assert_eq!(swipe_step((-30.0, 0.0)), None, "too short");
        assert_eq!(swipe_step((-80.0, 70.0)), None, "too diagonal");
        assert_eq!(swipe_step((0.0, -200.0)), None, "vertical");
    }

    #[test]
    fn a_two_finger_swipe_is_the_mean_of_both_fingers() {
        let state = TouchState {
            idx: 0,
            role: TouchRole::Swipe,
            fingers: vec![(1, (300.0, 200.0), (180.0, 210.0)), (2, (300.0, 300.0), (200.0, 300.0))],
        };
        assert_eq!(state.mean_travel(), (-110.0, 5.0));
        assert_eq!(swipe_step(state.mean_travel()), Some(1));
    }
}
