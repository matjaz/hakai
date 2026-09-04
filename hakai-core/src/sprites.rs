//! Sprites for live objects: flames, termites, paint droplets, shells, sparks.
//!
//! Ported from `SpriteFactory.swift`. Separate from `decals.rs` because these back live
//! scene objects rather than marks baked into the damage layer — a distinction that
//! matters once there's a renderer to feed (Phase 4), not yet.
//!
//! **Y-up/y-down.** Most of these shapes are vertically symmetric (the termite, the
//! flash, the beam's palindromic gradient stops, the spray's uniform scatter) and so are
//! unaffected by Core-Graphics-y-up vs tiny-skia-y-down, the issue documented at length in
//! `decals.rs` and `icons.rs`. Three spots do have real up/down meaning and are fixed
//! inline where they occur: the flame's teardrop shape (tip vs base), the paint droplet's
//! highlight position, and the shell's highlight line.

use std::collections::HashMap;

use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LinearGradient, Paint, PathBuilder, Pixmap, Point,
    RadialGradient, Rect as SkRect, SpreadMode, Stroke, Transform,
};

use crate::geometry::{push_ellipse, push_rounded_rect};
use crate::rng::SeededRng;

// `DecalFactory` isn't imported at module level any more — `droplet` below takes its
// caller's colours as a plain parameter instead of reaching for
// `DecalFactory::DEFAULT_PAINT_COLORS` directly (see `droplet`'s own doc comment), so the
// only remaining use is in this file's own tests, imported there via `use super::*`.

pub struct SpriteFactory {
    cache: HashMap<String, Pixmap>,
}

impl SpriteFactory {
    pub const FLAME_FRAMES: i64 = 4;
    pub const TERMITE_FRAMES: i64 = 2;
    pub const SPRAY_FRAMES: i64 = 3;

    pub fn new() -> Self {
        Self { cache: HashMap::new() }
    }

    fn cached(&mut self, key: String, build: impl FnOnce() -> Pixmap) -> &Pixmap {
        self.cache.entry(key).or_insert_with(build)
    }

    /// The flying flame — directed and narrower.
    pub fn flame(&mut self, frame: i64) -> &Pixmap {
        let f = frame.rem_euclid(Self::FLAME_FRAMES);
        self.cached(format!("flame{f}"), move || make_flame(0xF1A3_0000u64.wrapping_add(f as u64), 72, 108))
    }

    /// The standing flame — taller, burning in place.
    pub fn standing_flame(&mut self, frame: i64) -> &Pixmap {
        let f = frame.rem_euclid(Self::FLAME_FRAMES);
        self.cached(format!("standflame{f}"), move || make_flame(0x57A4_0000u64.wrapping_add(f as u64), 84, 148))
    }

    pub fn termite(&mut self, frame: i64) -> &Pixmap {
        let f = frame.rem_euclid(Self::TERMITE_FRAMES);
        self.cached(format!("termite{f}"), move || make_termite(f, 44, 24))
    }

    /// `colors` is the live paint palette — `DecalFactory::paint_colors()`, so a flying
    /// droplet always matches the splat it'll leave (both read from the same source; see
    /// `DecalFactory::set_paint_colors`'s doc comment for why a caller has to own that
    /// timing, not this cache). Not read from `DecalFactory` directly: this struct has no
    /// reference to one, and taking one just for this single field would be a bigger
    /// change than passing the eight colours in by value.
    pub fn droplet(&mut self, color_index: i64, colors: &[(f32, f32, f32); 8]) -> &Pixmap {
        let i = color_index.rem_euclid(colors.len() as i64);
        let (r, g, b) = colors[i as usize];
        self.cached(format!("drop{i}"), move || make_droplet((r, g, b), 28))
    }

    /// An ejected shell.
    pub fn shell(&mut self) -> &Pixmap {
        self.cached("shell".to_string(), || make_shell(22, 12))
    }

    /// A brief flash on impact.
    pub fn flash(&mut self) -> &Pixmap {
        self.cached("flash".to_string(), || make_flash(72))
    }

    /// A horizontal segment of the phaser beam; the scene stretches it along the shot.
    pub fn beam(&mut self) -> &Pixmap {
        self.cached("beam".to_string(), || make_beam(64, 24))
    }

    pub fn spray(&mut self, frame: i64) -> &Pixmap {
        let f = frame.rem_euclid(Self::SPRAY_FRAMES);
        self.cached(format!("spray{f}"), move || make_spray(0x5B7A_0000u64.wrapping_add(f as u64), 56))
    }
}

impl Default for SpriteFactory {
    fn default() -> Self {
        Self::new()
    }
}

fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgba8(to_u8(r), to_u8(g), to_u8(b), to_u8(a))
}

fn stop(pos: f64, color: Color) -> GradientStop {
    GradientStop::new(pos as f32, color)
}

fn radial_gradient_paint(center: (f32, f32), radius: f32, stops: Vec<GradientStop>) -> Paint<'static> {
    let mut paint = Paint::default();
    let pt = Point::from_xy(center.0, center.1);
    if let Some(shader) = RadialGradient::new(pt, pt, radius.max(0.01), stops, SpreadMode::Pad, Transform::identity()) {
        paint.shader = shader;
    }
    paint.anti_alias = true;
    paint
}

fn fill(pixmap: &mut Pixmap, path: &tiny_skia::Path, color: Color) {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    pixmap.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn stroke(pixmap: &mut Pixmap, path: &tiny_skia::Path, color: Color, width: f32, round_cap: bool) {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    let stroke = Stroke {
        width,
        line_cap: if round_cap { LineCap::Round } else { LineCap::Butt },
        ..Default::default()
    };
    pixmap.stroke_path(path, &paint, &stroke, Transform::identity(), None);
}

// MARK: - Flame

fn make_flame(seed: u64, w: u32, h: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(w, h).expect("could not allocate the flame pixmap");
    let mut rng = SeededRng::new(seed);
    let width = w as f32;
    let height = h as f32;
    let cx = width / 2.0;

    // A flame gets its shape from three nested teardrops — an outer red one, a middle
    // orange one and a yellow-white core. Each is narrower and lower than the last.
    //
    // y-up in the original (the wide base near the bottom, the tip near the top); flipped
    // here with a single `height - Y` on every literal Y, rather than per curve — same
    // fix as the paint splat in `decals.rs`, applied proactively this time.
    let teardrop = |height_factor: f32, width_factor: f32, wobble: f64, rng: &mut SeededRng| -> Option<tiny_skia::Path> {
        let top = height * height_factor;
        let half_w = width * 0.5 * width_factor;
        let base_y = height * 0.04;
        let fy = |y: f32| height - y;

        let tip_x = cx + rng.jitter(wobble) as f32 * width;
        let mut pb = PathBuilder::new();
        pb.move_to(cx - half_w, fy(base_y));
        pb.cubic_to(
            cx - half_w * 1.12, fy(base_y + top * 0.42),
            cx - half_w * 0.42, fy(base_y + top * 0.82),
            tip_x, fy(top),
        );
        pb.cubic_to(
            cx + half_w * 0.42, fy(base_y + top * 0.82),
            cx + half_w * 1.12, fy(base_y + top * 0.42),
            cx + half_w, fy(base_y),
        );
        pb.quad_to(cx, fy(base_y - height * 0.03), cx - half_w, fy(base_y));
        pb.close();
        pb.finish()
    };

    if let Some(p) = teardrop(0.94, 0.96, 0.06, &mut rng) {
        fill(&mut pixmap, &p, rgba(0.86, 0.20, 0.05, 0.78));
    }
    if let Some(p) = teardrop(0.70, 0.66, 0.05, &mut rng) {
        fill(&mut pixmap, &p, rgba(1.0, 0.56, 0.08, 0.92));
    }
    if let Some(p) = teardrop(0.42, 0.36, 0.04, &mut rng) {
        fill(&mut pixmap, &p, rgba(1.0, 0.92, 0.55, 0.96));
    }

    // Sparks rising out of the flame.
    for _ in 0..rng.int(3, 8) {
        let r = width * rng.range(0.014, 0.032) as f32;
        let x = cx + rng.jitter(width as f64 * 0.28) as f32;
        let y = height - height * rng.range(0.55, 0.99) as f32; // flipped, same rule as above
        let mut pb = PathBuilder::new();
        pb.push_circle(x, y, r);
        if let Some(path) = pb.finish() {
            fill(&mut pixmap, &path, rgba(1.0, 0.80, 0.35, rng.range(0.4, 0.9) as f32));
        }
    }

    pixmap
}

// MARK: - Termite

fn make_termite(frame: i64, w: u32, h: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(w, h).expect("could not allocate the termite pixmap");
    let width = w as f32;
    let height = h as f32;
    let mid_y = height / 2.0;

    let shell = rgba(0.87, 0.78, 0.57, 1.0);
    let chitin = rgba(0.54, 0.33, 0.14, 1.0);
    let dark = rgba(0.30, 0.17, 0.06, 1.0);

    // The legs swap between the two frames — enough to suggest walking. Both branches
    // below draw a line to *both* extremes regardless of `up`, so — a property of the
    // Swift source this was ported from, not introduced here — `frame` doesn't currently
    // change this sprite's pixels. Ported as-is rather than silently "fixed"; every other
    // shape in this sprite is symmetric around `mid_y` and so has no up/down convention to
    // get wrong regardless.
    for (i, &dx) in [0.42f32, 0.56, 0.70].iter().enumerate() {
        let up = (i % 2 == 0) == (frame == 0);
        let x = width * dx;
        for y_end in [if up { height * 0.92 } else { height * 0.08 }, if up { height * 0.08 } else { height * 0.92 }] {
            let mut pb = PathBuilder::new();
            pb.move_to(x, mid_y);
            pb.line_to(x - width * 0.05, y_end);
            if let Some(path) = pb.finish() {
                stroke(&mut pixmap, &path, chitin, height * 0.09, true);
            }
        }
    }

    // The abdomen and the thorax.
    let mut pb = PathBuilder::new();
    push_ellipse(&mut pb, width * 0.60, mid_y, width * 0.26, height * 0.30);
    if let Some(path) = pb.finish() {
        fill(&mut pixmap, &path, shell);
    }
    let mut pb = PathBuilder::new();
    push_ellipse(&mut pb, width * 0.31, mid_y, width * 0.11, height * 0.26);
    if let Some(path) = pb.finish() {
        fill(&mut pixmap, &path, chitin);
    }
    // The head.
    let mut pb = PathBuilder::new();
    push_ellipse(&mut pb, width * 0.16, mid_y, width * 0.12, height * 0.30);
    if let Some(path) = pb.finish() {
        fill(&mut pixmap, &path, chitin);
    }

    // The mandibles.
    for sign in [1.0f32, -1.0] {
        let mut pb = PathBuilder::new();
        pb.move_to(width * 0.08, mid_y + sign * height * 0.14);
        pb.line_to(0.0, mid_y + sign * height * 0.32);
        if let Some(path) = pb.finish() {
            stroke(&mut pixmap, &path, dark, height * 0.11, false);
        }
    }

    // Segments on the abdomen.
    for &dx in &[0.52f32, 0.63, 0.74] {
        let mut pb = PathBuilder::new();
        pb.move_to(width * dx, mid_y - height * 0.24);
        pb.line_to(width * dx, mid_y + height * 0.24);
        if let Some(path) = pb.finish() {
            stroke(&mut pixmap, &path, rgba(0.66, 0.56, 0.36, 0.9), height * 0.05, false);
        }
    }

    pixmap
}

// MARK: - Paint droplet

fn make_droplet(color: (f32, f32, f32), px: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(px, px).expect("could not allocate the droplet pixmap");
    let side = px as f32;
    let c = (side / 2.0, side / 2.0);
    let r = side * 0.36;

    let mut pb = PathBuilder::new();
    pb.push_circle(c.0, c.1, r);
    if let Some(path) = pb.finish() {
        fill(&mut pixmap, &path, rgba(color.0, color.1, color.2, 1.0));
    }
    // A dark rim so the droplet is visible even over paint of the same brightness.
    let mut pb = PathBuilder::new();
    pb.push_circle(c.0, c.1, r);
    if let Some(path) = pb.finish() {
        stroke(&mut pixmap, &path, rgba(0.0, 0.0, 0.0, 0.35), side * 0.05, false);
    }
    // A highlight. Swift's offset is (-0.38r, +0.30r) in a y-up frame — negating the y
    // component keeps it on the same visual (upper) side here.
    let hr = r * 0.34;
    let mut pb = PathBuilder::new();
    pb.push_circle(c.0 - r * 0.38, c.1 - r * 0.30, hr);
    if let Some(path) = pb.finish() {
        fill(&mut pixmap, &path, rgba(1.0, 1.0, 1.0, 0.62));
    }

    pixmap
}

// MARK: - Shell

fn make_shell(w: u32, h: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(w, h).expect("could not allocate the shell pixmap");
    let width = w as f32;
    let height = h as f32;
    let brass = rgba(0.79, 0.62, 0.22, 1.0);

    let mut pb = PathBuilder::new();
    push_rounded_rect(&mut pb, width * 0.06, height * 0.18, width * 0.88, height * 0.64, height * 0.30);
    if let Some(path) = pb.finish() {
        fill(&mut pixmap, &path, brass);
    }
    // The case rim.
    if let Some(r) = SkRect::from_xywh(width * 0.06, height * 0.12, width * 0.13, height * 0.76) {
        let path = PathBuilder::from_rect(r);
        fill(&mut pixmap, &path, rgba(0.62, 0.46, 0.14, 1.0));
    }
    // A highlight along its length. Swift's y = height*0.66 is in a y-up frame (above the
    // shell's centre at height*0.5); `height*(1-0.66)` is the y-down equivalent, keeping
    // the highlight on the same visual (upper) side.
    let hy = height * (1.0 - 0.66);
    let mut pb = PathBuilder::new();
    pb.move_to(width * 0.22, hy);
    pb.line_to(width * 0.86, hy);
    if let Some(path) = pb.finish() {
        stroke(&mut pixmap, &path, rgba(1.0, 1.0, 1.0, 0.55), height * 0.14, false);
    }

    pixmap
}

// MARK: - Flash and beam

fn make_flash(px: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(px, px).expect("could not allocate the flash pixmap");
    let side = px as f32;
    let c = (side / 2.0, side / 2.0);

    // Radially symmetric — no y-flip concern.
    let paint = radial_gradient_paint(
        c,
        side * 0.34,
        vec![
            stop(0.0, rgba(1.0, 1.0, 0.92, 1.0)),
            stop(0.35, rgba(1.0, 0.86, 0.42, 0.75)),
            stop(1.0, rgba(1.0, 0.52, 0.10, 0.0)),
        ],
    );
    let mut pb = PathBuilder::new();
    pb.push_circle(c.0, c.1, side * 0.34);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    // A four-pointed star.
    for i in 0..4 {
        let a = i as f32 * (std::f32::consts::PI / 2.0) + std::f32::consts::FRAC_PI_4;
        let line_width = side * if i % 2 == 0 { 0.05 } else { 0.035 };
        let mut pb = PathBuilder::new();
        pb.move_to(c.0 - a.cos() * side * 0.46, c.1 - a.sin() * side * 0.46);
        pb.line_to(c.0 + a.cos() * side * 0.46, c.1 + a.sin() * side * 0.46);
        if let Some(path) = pb.finish() {
            stroke(&mut pixmap, &path, rgba(1.0, 0.95, 0.72, 0.85), line_width, true);
        }
    }

    pixmap
}

/// A horizontal segment of the phaser beam; the scene stretches it along the shot.
fn make_beam(w: u32, h: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(w, h).expect("could not allocate the beam pixmap");
    let height = h as f32;

    // A vertical gradient: white core, cyan glow, transparent edges. The stop sequence is
    // symmetric top-to-bottom (0/1 transparent, 0.32/0.68 translucent, 0.5 white core), so
    // — unlike the flame or shell — this needs no y-flip correction; ported directly.
    let stops = vec![
        stop(0.0, rgba(0.20, 0.85, 1.0, 0.0)),
        stop(0.32, rgba(0.35, 0.92, 1.0, 0.55)),
        stop(0.5, rgba(1.0, 1.0, 1.0, 1.0)),
        stop(0.68, rgba(0.35, 0.92, 1.0, 0.55)),
        stop(1.0, rgba(0.20, 0.85, 1.0, 0.0)),
    ];
    let mut paint = Paint::default();
    if let Some(shader) = LinearGradient::new(
        Point::from_xy(0.0, 0.0),
        Point::from_xy(0.0, height),
        stops,
        SpreadMode::Pad,
        Transform::identity(),
    ) {
        paint.shader = shader;
    }
    paint.anti_alias = true;

    if let Some(r) = SkRect::from_xywh(0.0, 0.0, w as f32, height) {
        let path = PathBuilder::from_rect(r);
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    pixmap
}

// MARK: - Washer spray

fn make_spray(seed: u64, px: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(px, px).expect("could not allocate the spray pixmap");
    let mut rng = SeededRng::new(seed);
    let side = px as f64;

    // A uniform random scatter — no directional meaning, no y-flip concern.
    for _ in 0..rng.int(12, 22) {
        let r = side * rng.range(0.03, 0.085);
        let x = rng.range(r, side - r);
        let y = rng.range(r, side - r);
        let mut pb = PathBuilder::new();
        pb.push_circle(x as f32, y as f32, r as f32);
        if let Some(path) = pb.finish() {
            fill(&mut pixmap, &path, rgba(0.62, 0.84, 0.96, rng.range(0.25, 0.65) as f32));
        }
    }

    pixmap
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decals::DecalFactory;

    fn is_blank(pixmap: &Pixmap) -> bool {
        !pixmap.data().chunks_exact(4).any(|px| px[3] > 8)
    }

    #[test]
    fn every_sprite_type_is_not_blank() {
        let mut f = SpriteFactory::new();
        assert!(!is_blank(f.flame(0)), "flame is blank");
        assert!(!is_blank(f.standing_flame(0)), "standing_flame is blank");
        assert!(!is_blank(f.termite(0)), "termite is blank");
        let colors = DecalFactory::DEFAULT_PAINT_COLORS.map(|(_, r, g, b)| (r, g, b));
        assert!(!is_blank(f.droplet(0, &colors)), "droplet is blank");
        assert!(!is_blank(f.shell()), "shell is blank");
        assert!(!is_blank(f.flash()), "flash is blank");
        assert!(!is_blank(f.beam()), "beam is blank");
        assert!(!is_blank(f.spray(0)), "spray is blank");
    }

    #[test]
    fn flame_frames_are_deterministic_and_differ() {
        let mut a = SpriteFactory::new();
        let mut b = SpriteFactory::new();
        let mut seen: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
        for f in 0..SpriteFactory::FLAME_FRAMES {
            let da = a.flame(f).data().to_vec();
            let db = b.flame(f).data().to_vec();
            assert_eq!(da, db, "flame frame {f} was not reproducible");
            assert!(seen.insert(da), "flame frame {f} duplicated another frame's pixels");
        }
    }

    #[test]
    fn flame_and_standing_flame_are_different_sizes() {
        let mut f = SpriteFactory::new();
        let flying = f.flame(0);
        assert_eq!((flying.width(), flying.height()), (72, 108));
        let standing = f.standing_flame(0);
        assert_eq!((standing.width(), standing.height()), (84, 148));
    }

    #[test]
    fn eight_droplet_colors_differ() {
        let mut f = SpriteFactory::new();
        let colors = DecalFactory::DEFAULT_PAINT_COLORS.map(|(_, r, g, b)| (r, g, b));
        let mut seen: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
        for c in 0..8 {
            let bytes = f.droplet(c, &colors).data().to_vec();
            assert!(seen.insert(bytes), "droplet colour {c} duplicated another colour's pixels");
        }
    }

    #[test]
    fn droplet_reflects_whatever_colors_its_caller_passes() {
        let mut f = SpriteFactory::new();
        let mut theme = [(0.0, 0.0, 0.0); 8];
        theme[0] = (1.0, 0.0, 1.0); // a colour DEFAULT_PAINT_COLORS never produces
        assert!(!is_blank(f.droplet(0, &theme)));
    }

    #[test]
    fn negative_and_oversized_indices_wrap() {
        let mut f = SpriteFactory::new();
        let colors = DecalFactory::DEFAULT_PAINT_COLORS.map(|(_, r, g, b)| (r, g, b));
        assert_eq!(f.droplet(-1, &colors).data().to_vec(), f.droplet(7, &colors).data().to_vec());
        assert_eq!(f.flame(-1).data().to_vec(), f.flame(SpriteFactory::FLAME_FRAMES - 1).data().to_vec());
        assert_eq!(f.termite(2).data().to_vec(), f.termite(0).data().to_vec());
    }

    #[test]
    fn spray_frames_differ() {
        let mut f = SpriteFactory::new();
        let a = f.spray(0).data().to_vec();
        let b = f.spray(1).data().to_vec();
        assert_ne!(a, b);
    }
}
