//! HUD text — rasterizes a single line of arbitrary UTF-8 text into a `tiny_skia::Pixmap`,
//! for upload as a GPU texture the same way an icon or a sprite already is.
//!
//! Unlike `hakai_core::fonts`'s stamp decal (which traces glyph outlines by hand with
//! `ttf-parser`, workable because it only ever draws a handful of known short strings),
//! HUD text is arbitrary — tool names, a licence's modification notice, credits authors —
//! so this uses `cosmic-text` for real shaping and rasterization instead. `tiny-skia` never
//! sees a glyph; `cosmic-text`'s own `SwashCache` rasterizes each one, and this module just
//! composites the coverage `cosmic-text` hands back into a plain RGBA buffer.
//!
//! RISK, more than anywhere else in this port: `cosmic-text`'s convenience API
//! (`FontSystem`'s constructors, `Buffer::draw`'s per-pixel callback shape, `Color`'s
//! accessors) has shifted across its `0.1x` releases, and I have no way to check the exact
//! shape for whatever version actually resolves here — everything below is my
//! best-confidence reading, not a verified one. If this doesn't compile, this file (not
//! the rest of the HUD) is where to start.

use cosmic_text::{Attrs, Buffer, Color as CosmicColor, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};

const REGULAR: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf");

/// The one family name both embedded font files declare internally — referenced by name
/// (`Family::Name`), not `Family::Monospace`, so this never depends on `fontdb` guessing a
/// generic-family alias for a font it only just loaded from bytes with nothing else in the
/// database to compare against.
const FAMILY: &str = "JetBrains Mono";

pub struct TextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl TextRenderer {
    pub fn new() -> Self {
        // RISK: `FontSystem::new_with_locale_and_db` is my best-confidence reading of how
        // to hand `FontSystem` a `fontdb::Database` of *only* our own embedded fonts,
        // rather than `FontSystem::new()`'s default of scanning the host's installed
        // system fonts — deliberate, matching this whole port's preference for
        // deterministic, embedded assets over a system font lookup (see
        // `hakai_core::fonts`'s own doc comment).
        let mut db = cosmic_text::fontdb::Database::new();
        db.load_font_data(REGULAR.to_vec());
        db.load_font_data(BOLD.to_vec());
        let font_system = FontSystem::new_with_locale_and_db("en-US".to_string(), db);
        Self { font_system, swash_cache: SwashCache::new() }
    }

    /// Rasterizes one line of `text` at `size_px`, tightly cropped to the actual ink
    /// (every drawn pixel's bounding box) rather than to some guessed line-height canvas —
    /// `None` for empty text, or text that turned out to draw nothing (e.g. all
    /// whitespace). Tight cropping isn't just an optimization here: it's what makes the
    /// returned pixmap's own vertical *centre* a reliable stand-in for the text's visual
    /// centre, which `main.rs` leans on to vertically centre HUD labels the same way
    /// `SKLabelNode.verticalAlignmentMode = .center` did in the Swift original — no font
    /// ascent/descent metrics need to leave this module for that to work.
    ///
    /// `color` is straight (non-premultiplied) RGBA; the returned pixmap's bytes are
    /// premultiplied, matching what every other texture in this renderer (icons, sprites,
    /// decals) already is.
    ///
    /// Shapes the line twice: a first pass measures the ink's bounding box without
    /// allocating a pixmap at all, then a second pass draws into a pixmap sized to exactly
    /// that box. Cheap for the short strings the HUD ever draws, and only run when a
    /// label's text actually changes (a tool switch, a new toast), not every frame — see
    /// `HudGpu`'s cache (`ensure_hud_text`/`ensure_toast_gpu`) in `main.rs`.
    pub fn rasterize(&mut self, text: &str, size_px: f32, bold: bool, color: [u8; 4]) -> Option<tiny_skia::Pixmap> {
        if text.is_empty() {
            return None;
        }

        let metrics = Metrics::new(size_px, size_px * 1.35);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        // Wide and tall enough that nothing in practice wraps or clips before the
        // measuring pass below crops to the real ink.
        buffer.set_size(&mut self.font_system, Some(8192.0), Some(size_px * 3.0));

        let weight = if bold { Weight::BOLD } else { Weight::NORMAL };
        let attrs = Attrs::new().family(Family::Name(FAMILY)).weight(weight);
        buffer.set_text(&mut self.font_system, text, attrs, Shaping::Advanced);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let text_color = CosmicColor::rgba(color[0], color[1], color[2], color[3]);

        // Pass 1: measure. `min_x`/`min_y` start at `i32::MAX` as "nothing seen yet"
        // sentinels — if they're still there after `draw`, no pixel was ever covered.
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        buffer.draw(&mut self.font_system, &mut self.swash_cache, text_color, |x, y, pw, ph, c| {
            if c.a() == 0 || pw == 0 || ph == 0 {
                return;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + pw as i32);
            max_y = max_y.max(y + ph as i32);
        });
        if min_x > max_x || min_y > max_y {
            return None; // nothing was actually drawn (e.g. an all-whitespace string)
        }

        let w = (max_x - min_x).max(1) as u32;
        let h = (max_y - min_y).max(1) as u32;
        let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
        let stride = w as usize * 4;
        let data = pixmap.data_mut();

        // Pass 2: draw, offset so the ink's own top-left lands at (0, 0).
        buffer.draw(&mut self.font_system, &mut self.swash_cache, text_color, |x, y, pw, ph, c| {
            if c.a() == 0 {
                return;
            }
            for dy in 0..ph {
                let py = y - min_y + dy as i32;
                if py < 0 || py as u32 >= h {
                    continue;
                }
                for dx in 0..pw {
                    let px = x - min_x + dx as i32;
                    if px < 0 || px as u32 >= w {
                        continue;
                    }
                    let idx = py as usize * stride + px as usize * 4;
                    // `c` is straight alpha; tiny-skia's `Pixmap` bytes are premultiplied
                    // — same conversion `sprites.rs`/`decals.rs` get for free from
                    // `tiny_skia::Paint`, done by hand here since nothing routes this
                    // through tiny-skia's own path-fill machinery.
                    let a = c.a() as u32;
                    data[idx] = ((c.r() as u32 * a) / 255) as u8;
                    data[idx + 1] = ((c.g() as u32 * a) / 255) as u8;
                    data[idx + 2] = ((c.b() as u32 * a) / 255) as u8;
                    data[idx + 3] = c.a();
                }
            }
        });

        Some(pixmap)
    }
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}
