//! Glyph outline extraction for baked-in decal text.
//!
//! The stamp decal (`VOID`, `REJECTED`, `PAID`, …) needs real text rendered as filled
//! paths, the same way the macOS build uses `CTLineDraw` into a `CGContext`. There's no
//! CoreText here, so this reads glyph outlines directly out of an embedded font with
//! `ttf-parser` and builds a `tiny_skia::Path` from them — no system font lookup, so the
//! output is identical on every machine, which matters for the determinism tests.
//!
//! Deliberately not a general text layout engine: no shaping, no kerning tables, no
//! bidi — just per-glyph outlines at a fixed advance plus a manual letter-spacing bump,
//! which is all ten short, all-caps, Latin-only stamp texts need.

use tiny_skia::{Path, PathBuilder};

/// Forwards `ttf-parser`'s outline callbacks straight into a `tiny_skia::PathBuilder`,
/// applying this glyph's pen position and font-to-pixel scale as it goes. Font units have
/// +y pointing up; pixmaps have +y pointing down, hence the sign flip on every y.
struct GlyphPathBuilder<'a> {
    pb: &'a mut PathBuilder,
    pen_x: f32,
    baseline_y: f32,
    scale: f32,
}

impl GlyphPathBuilder<'_> {
    fn tx(&self, x: f32) -> f32 {
        self.pen_x + x * self.scale
    }
    fn ty(&self, y: f32) -> f32 {
        self.baseline_y - y * self.scale
    }
}

impl ttf_parser::OutlineBuilder for GlyphPathBuilder<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pb.move_to(self.tx(x), self.ty(y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.pb.line_to(self.tx(x), self.ty(y));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.pb.quad_to(self.tx(x1), self.ty(y1), self.tx(x), self.ty(y));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.pb
            .cubic_to(self.tx(x1), self.ty(y1), self.tx(x2), self.ty(y2), self.tx(x), self.ty(y));
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

pub struct StampFont<'a> {
    face: ttf_parser::Face<'a>,
}

impl<'a> StampFont<'a> {
    pub fn new(data: &'a [u8]) -> Option<Self> {
        Some(Self {
            face: ttf_parser::Face::parse(data, 0).ok()?,
        })
    }

    /// Lays out `text` as one merged path, upper-left-ish origin: baseline at y=0, first
    /// glyph's left edge at x=0. `kern` is extra letter-spacing in px between glyphs,
    /// mirroring the original's `.kern` attribute (real fonts don't need this much
    /// spacing at body sizes, but a rubber-stamp look wants it loose).
    ///
    /// Returns the path plus its tight bounding box `(x, y, w, h)` in that same space —
    /// good enough to centre the text the way `CTLineGetBoundsWithOptions` did, even
    /// though it isn't computed the same way under the hood.
    pub fn layout(&self, text: &str, font_size: f32, kern: f32) -> Option<(Path, (f32, f32, f32, f32))> {
        let upm = self.face.units_per_em() as f32;
        if upm <= 0.0 {
            return None;
        }
        let scale = font_size / upm;

        let mut pb = PathBuilder::new();
        let mut pen_x: f32 = 0.0;
        let (mut min_x, mut min_y, mut max_x, mut max_y) =
            (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut any = false;

        for ch in text.chars() {
            let Some(gid) = self.face.glyph_index(ch) else {
                pen_x += font_size * 0.3; // a rough space width for anything not in the font
                continue;
            };
            let advance = self.face.glyph_hor_advance(gid).unwrap_or(0) as f32 * scale;

            let mut builder = GlyphPathBuilder {
                pb: &mut pb,
                pen_x,
                baseline_y: 0.0,
                scale,
            };
            if let Some(bbox) = self.face.outline_glyph(gid, &mut builder) {
                any = true;
                min_x = min_x.min(pen_x + bbox.x_min as f32 * scale);
                max_x = max_x.max(pen_x + bbox.x_max as f32 * scale);
                min_y = min_y.min(-(bbox.y_max as f32) * scale);
                max_y = max_y.max(-(bbox.y_min as f32) * scale);
            }
            pen_x += advance + kern;
        }

        if !any {
            return None;
        }
        let path = pb.finish()?;
        Some((path, (min_x, min_y, (max_x - min_x).max(0.0), (max_y - min_y).max(0.0))))
    }
}
