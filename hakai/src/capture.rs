//! Screen capture — `zwlr_screencopy_v1`, for the brightness-driven impact sound.
//!
//! `hakai_core::audio::smash_name` (ported since Phase 3) already picks a hollow/wooden
//! vs. glassy/metallic impact variant from `brightness: Option<u8>` — but every
//! `ToolContext` in this port has passed `None` for it, since there was nothing to sample
//! from before now, which falls back to a random variant instead of one actually driven
//! by what's under the cursor (`sound by color brightness below mouse cursor`, the
//! mechanic that makes the original's hammer "feel alive").
//!
//! **Coarse and periodic, not per-pixel and per-frame.** This only ever needs to answer
//! "is it dark or light under the cursor" for whichever tool asks at the moment of a
//! strike — not reproduce the desktop. [`BrightnessMap`] downsamples a captured frame to a
//! small grid (`GRID_COLS` × `GRID_ROWS`) once per capture, and a capture itself only
//! needs to happen every so often (`CAPTURE_INTERVAL`), not every render frame — the
//! desktop underneath rarely changes fast enough for that to matter, and it would be a lot
//! of needless `zwlr_screencopy_v1` traffic and SHM copying otherwise.
//!
//! **The actual `zwlr_screencopy_v1` request/event dance and the SHM buffer plumbing live
//! in `main.rs`**, alongside every other `Dispatch` impl `State` has, including the
//! per-output capture state itself (inferred from a couple of `Option` fields on
//! `GpuLayer` — `None` frame means idle, a frame but no buffer info means awaiting it,
//! both `Some` means a copy is in flight — rather than a separate enum here) — this
//! module is deliberately just the brightness grid, which doesn't need to know anything
//! about Wayland at all.

/// How often a fresh capture is requested per output, in seconds.
pub const CAPTURE_INTERVAL: f32 = 2.0;

const GRID_COLS: usize = 24;
const GRID_ROWS: usize = 16;

/// One output's most recent brightness sample grid. `sample` is the only thing a tool
/// ever needs from this — everything else here is what builds one from raw captured
/// pixels.
#[derive(Default)]
pub struct BrightnessMap {
    /// `GRID_COLS * GRID_ROWS` grayscale samples, row-major — empty until the first
    /// capture actually completes.
    grid: Vec<u8>,
    screen_width: f32,
    screen_height: f32,
}

impl BrightnessMap {
    /// The brightness (0..255) under `point` (screen pixels), or `None` before the first
    /// capture has completed — callers already treat a `None` brightness as "pick a
    /// random impact variant instead," the same fallback this port has used since Phase 3
    /// for "no screen-capture permission" in the Swift original.
    pub fn sample(&self, point: (f32, f32)) -> Option<u8> {
        if self.grid.is_empty() || self.screen_width <= 0.0 || self.screen_height <= 0.0 {
            return None;
        }
        let col = ((point.0 / self.screen_width) * GRID_COLS as f32).floor().clamp(0.0, GRID_COLS as f32 - 1.0) as usize;
        let row = ((point.1 / self.screen_height) * GRID_ROWS as f32).floor().clamp(0.0, GRID_ROWS as f32 - 1.0) as usize;
        self.grid.get(row * GRID_COLS + col).copied()
    }

    /// Rebuilds the grid from one freshly captured frame's raw pixels — `bytes` is a
    /// tightly packed `stride`-wide (bytes per row) buffer, `argb8888`/`xrgb8888` (the
    /// format `main.rs` requests): 4 bytes per pixel, blue/green/red/alpha in that byte
    /// order (`wl_shm`'s `Argb8888`/`Xrgb8888`, little-endian). Averages every source
    /// pixel that falls into each grid cell — cheap enough at `GRID_COLS × GRID_ROWS`
    /// even over a 4K frame, and a lot less noise-sensitive than nearest-neighbor
    /// sampling would be for something this coarse.
    pub fn update(&mut self, bytes: &[u8], width: u32, height: u32, stride: u32) {
        if width == 0 || height == 0 || stride == 0 {
            return;
        }
        let mut sums = vec![0u32; GRID_COLS * GRID_ROWS];
        let mut counts = vec![0u32; GRID_COLS * GRID_ROWS];

        for y in 0..height {
            let row_start = (y * stride) as usize;
            if row_start + (width as usize) * 4 > bytes.len() {
                break; // a short/corrupt buffer — use whatever rows did fit
            }
            let cell_row = ((y * GRID_ROWS as u32) / height).min(GRID_ROWS as u32 - 1) as usize;
            for x in 0..width {
                let px = row_start + (x * 4) as usize;
                let (b, g, r) = (bytes[px] as u32, bytes[px + 1] as u32, bytes[px + 2] as u32);
                // Perceptual luma (Rec. 601), not a flat average — matches how
                // `hakai_core::decals`/`icons` already reason about "dark vs light" for
                // deciding a highlight's own contrast direction elsewhere in this port.
                let luma = (r * 299 + g * 587 + b * 114) / 1000;
                let cell_col = ((x * GRID_COLS as u32) / width).min(GRID_COLS as u32 - 1) as usize;
                let idx = cell_row * GRID_COLS + cell_col;
                sums[idx] += luma;
                counts[idx] += 1;
            }
        }

        self.grid = sums
            .iter()
            .zip(counts.iter())
            .map(|(&sum, &count)| if count > 0 { (sum / count).min(255) as u8 } else { 0 })
            .collect();
        self.screen_width = width as f32;
        self.screen_height = height as f32;
    }
}

/// Converts one freshly captured frame's raw pixels into a tightly packed, fully-opaque
/// RGBA8 buffer — for frozen mode's background (`DisplayMode.frozen` in the Swift
/// original: a captured snapshot shown as the overlay's own background, so the *real*
/// desktop can keep changing underneath — other windows moving, redrawing — without
/// disturbing what the user sees themselves smashing). Same source byte layout as
/// `BrightnessMap::update` (BGRA, `wl_shm`'s `Argb8888`/`Xrgb8888`), swapped to RGBA —
/// what `wgpu`'s `Rgba8Unorm` (every other texture in this renderer) expects — and with
/// alpha forced to fully opaque regardless of the source's own alpha channel: this is
/// meant to fully replace what's behind the overlay, not blend with it.
pub fn to_rgba(bytes: &[u8], width: u32, height: u32, stride: u32) -> Vec<u8> {
    let mut out = vec![0u8; (width as usize) * (height as usize) * 4];
    for y in 0..height {
        let row_start = (y * stride) as usize;
        if row_start + (width as usize) * 4 > bytes.len() {
            break; // a short/corrupt buffer — leave the remaining rows black, not garbage
        }
        for x in 0..width {
            let src = row_start + (x * 4) as usize;
            let dst = ((y * width + x) * 4) as usize;
            out[dst] = bytes[src + 2]; // R
            out[dst + 1] = bytes[src + 1]; // G
            out[dst + 2] = bytes[src]; // B
            out[dst + 3] = 255; // opaque, regardless of the source's own alpha
        }
    }
    out
}
