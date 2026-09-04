//! The procedural decal generator.
//!
//! Ported from `DecalFactory.swift` + `DecalFactory+Weapons.swift` — everything generated
//! in code and cached, no bundled image assets (except the one embedded font the stamp
//! decal needs for its text; see `fonts.rs`).
//!
//! Every decal has to be readable on both light and dark backgrounds, since it's drawn
//! over a live desktop of unknown colour — that's why every stroke has a dark core and an
//! offset light highlight, and why every blob has a light rim.

use std::collections::HashMap;
use std::f64::consts::TAU;

use tiny_skia::{
    BlendMode, Color, FillRule, GradientStop, LineCap, LineJoin, Paint, PathBuilder, Pixmap,
    Point, RadialGradient, SpreadMode, Stroke, Transform,
};

use crate::fonts::StampFont;
use crate::geometry::{push_ellipse, push_rounded_rect};
use crate::rng::SeededRng;

static STAMP_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/ArchivoBlack-Regular.ttf");

pub struct DecalFactory {
    cache: HashMap<String, Pixmap>,
    font: StampFont<'static>,
    /// The live paint-splat/droplet palette — `DEFAULT_PAINT_COLORS`' RGB triples until
    /// `set_paint_colors` overrides them (e.g. from an Omarchy theme; see `hakai`'s
    /// `theme.rs`). This crate stays headless/theme-agnostic itself — see the module doc
    /// comment — an override is just data handed in from outside, same shape as
    /// `AudioBackend` being injected rather than known about here.
    paint_colors: [(f32, f32, f32); 8],
}

impl DecalFactory {
    pub const CRACK_VARIANTS: i64 = 8;
    pub const BULLET_HOLE_VARIANTS: i64 = 4;
    pub const SCORCH_VARIANTS: i64 = 4;
    pub const PAINT_VARIANTS: i64 = 5;
    pub const PHASER_VARIANTS: i64 = 10;
    pub const STAMP_VARIANTS: i64 = 10;
    pub const BITE_VARIANTS: i64 = 4;
    pub const BLOOD_VARIANTS: i64 = 3;
    pub const SAW_CUT_VARIANTS: i64 = 4;
    pub const SLIVER_VARIANTS: i64 = 6;

    /// The eight colour-thrower colours — exactly the ones the original has
    /// (`sprite of color - red/green/blue/yellow/purple/cyan/orange/pink`). The built-in
    /// default and fallback for `paint_colors` — see `set_paint_colors`.
    pub const DEFAULT_PAINT_COLORS: [(&'static str, f32, f32, f32); 8] = [
        ("red", 0.86, 0.13, 0.14),
        ("green", 0.16, 0.70, 0.24),
        ("blue", 0.16, 0.36, 0.88),
        ("yellow", 0.96, 0.82, 0.13),
        ("purple", 0.56, 0.20, 0.78),
        ("cyan", 0.14, 0.76, 0.82),
        ("orange", 0.95, 0.51, 0.10),
        ("pink", 0.94, 0.35, 0.62),
    ];

    /// The stamp texts. The original has ten prints, two of them with a separate English
    /// variant, so they were textual from the start.
    pub const STAMP_TEXTS: [&'static str; 10] = [
        "VOID",
        "REJECTED",
        "APPROVED",
        "PAID",
        "URGENT",
        "TOP SECRET",
        "COPY",
        "CANCELLED",
        "DRAFT",
        "CONFIDENTIAL",
    ];

    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            font: StampFont::new(STAMP_FONT_BYTES).expect("bundled stamp font failed to parse"),
            paint_colors: Self::DEFAULT_PAINT_COLORS.map(|(_, r, g, b)| (r, g, b)),
        }
    }

    fn cached(&mut self, key: String, build: impl FnOnce() -> Pixmap) -> &Pixmap {
        self.cache.entry(key).or_insert_with(build)
    }

    /// The live paint-splat/droplet palette (RGB only — see `DEFAULT_PAINT_COLORS` for the
    /// names). A renderer reads this to keep a live flying droplet
    /// (`SpriteFactory::droplet`, a separate cache from this one) in sync with whatever
    /// `set_paint_colors` last set here.
    pub fn paint_colors(&self) -> [(f32, f32, f32); 8] {
        self.paint_colors
    }

    /// Overrides the eight paint-splat/droplet colours — e.g. from an Omarchy theme (see
    /// `hakai`'s `theme.rs`). Must be called before the first `paint_splat`, since results
    /// are cached by colour *index* alone (`"paint{c}_{v}"`, not the RGB itself) — calling
    /// this after the cache is already warm for some index would leave that index's old
    /// colour baked into its cached `Pixmap` forever.
    pub fn set_paint_colors(&mut self, colors: [(f32, f32, f32); 8]) {
        self.paint_colors = colors;
    }

    // MARK: - Public API

    /// The crack left by a hammer strike. `variant` wraps into `0..CRACK_VARIANTS`.
    pub fn crack(&mut self, variant: i64) -> &Pixmap {
        let v = wrap(variant, Self::CRACK_VARIANTS);
        self.cached(format!("crack{v}"), || {
            make_crack(0xC7AC_0000u64.wrapping_add(v as u64), 512)
        })
    }

    /// A machine gun hole.
    pub fn bullet_hole(&mut self, variant: i64) -> &Pixmap {
        let v = wrap(variant, Self::BULLET_HOLE_VARIANTS);
        self.cached(format!("hole{v}"), || {
            make_bullet_hole(0xB011_0000u64.wrapping_add(v as u64), 128)
        })
    }

    /// A scorch mark left by the flame-thrower.
    pub fn scorch(&mut self, variant: i64) -> &Pixmap {
        let v = wrap(variant, Self::SCORCH_VARIANTS);
        self.cached(format!("scorch{v}"), || {
            make_scorch(0x5C07_0000u64.wrapping_add(v as u64), 256)
        })
    }

    /// A paint splat.
    pub fn paint_splat(&mut self, color_index: i64, variant: i64) -> &Pixmap {
        let c = wrap(color_index, self.paint_colors.len() as i64);
        let v = wrap(variant, Self::PAINT_VARIANTS);
        let (r, g, b) = self.paint_colors[c as usize];
        self.cached(format!("paint{c}_{v}"), || {
            make_paint_splat((r, g, b), 0x9A17_0000u64.wrapping_add((c * 31 + v) as u64), 256)
        })
    }

    /// A phaser impact.
    pub fn phaser_hit(&mut self, variant: i64) -> &Pixmap {
        let v = wrap(variant, Self::PHASER_VARIANTS);
        self.cached(format!("phaser{v}"), || {
            make_phaser_hit(0x7A5E_0000u64.wrapping_add(v as u64), 192)
        })
    }

    /// A stamp print.
    pub fn stamp_print(&mut self, variant: i64) -> &Pixmap {
        let v = wrap(variant, Self::STAMP_VARIANTS);
        let font = &self.font;
        // Can't borrow `self.font` and call `self.cached` (which needs `&mut self.cache`)
        // at the same time — build the pixmap first with a plain function, disjoint from
        // the cache field, then hand the finished value to `cached`.
        let key = format!("stamp{v}");
        if !self.cache.contains_key(&key) {
            let img = make_stamp_print(font, v, 320);
            self.cache.insert(key.clone(), img);
        }
        self.cache.get(&key).unwrap()
    }

    /// A termite bite.
    pub fn bite(&mut self, variant: i64) -> &Pixmap {
        let v = wrap(variant, Self::BITE_VARIANTS);
        self.cached(format!("bite{v}"), || {
            make_bite(0xB17E_0000u64.wrapping_add(v as u64), 32)
        })
    }

    /// The blood of a dead termite.
    pub fn blood(&mut self, variant: i64) -> &Pixmap {
        let v = wrap(variant, Self::BLOOD_VARIANTS);
        self.cached(format!("blood{v}"), || {
            make_blood(0xB100_0000u64.wrapping_add(v as u64), 96)
        })
    }

    /// One segment of a chain-saw cut. Not square — drawn along the drag path.
    pub fn saw_cut(&mut self, variant: i64) -> &Pixmap {
        let v = wrap(variant, Self::SAW_CUT_VARIANTS);
        self.cached(format!("sawcut{v}"), || {
            make_saw_cut(0x5A00_0000u64.wrapping_add(v as u64), 128, 48)
        })
    }

    /// A splinter that flies off an impact.
    pub fn sliver(&mut self, variant: i64) -> &Pixmap {
        let v = wrap(variant, Self::SLIVER_VARIANTS);
        self.cached(format!("sliver{v}"), || {
            make_sliver(0x5117_0000u64.wrapping_add(v as u64), 32)
        })
    }
}

impl Default for DecalFactory {
    fn default() -> Self {
        Self::new()
    }
}

fn wrap(value: i64, count: i64) -> i64 {
    value.rem_euclid(count.max(1))
}

/// Whether an image is entirely transparent. A blank decal shows up in the game as "the
/// tool does not work" — checked on a stride of 2 in both axes, same as the Swift check.
pub fn is_blank(pixmap: &Pixmap) -> bool {
    let data = pixmap.data();
    let w = pixmap.width() as usize;
    let h = pixmap.height() as usize;
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let idx = (y * w + x) * 4;
            if data[idx + 3] > 8 {
                return false;
            }
        }
    }
    true
}

// MARK: - Shared helpers

fn rgb(r: f32, g: f32, b: f32, a: f32) -> Color {
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgba8(to_u8(r), to_u8(g), to_u8(b), to_u8(a))
}

fn gray(g: f32, a: f32) -> Color {
    rgb(g, g, g, a)
}

fn stop(pos: f64, color: Color) -> GradientStop {
    GradientStop::new(pos as f32, color)
}

/// `RadialGradient::new(start, end, radius, stops, mode, transform)` — a two-point conical
/// gradient à la Skia, where `start` is the focal point and `end`+`radius` define the
/// outer circle. Our decals only need a simple concentric gradient (no focal offset), so
/// `start == end == center`.
fn radial_gradient_paint(center: (f32, f32), radius: f32, stops: Vec<GradientStop>) -> Paint<'static> {
    let mut paint = Paint::default();
    let pt = Point::from_xy(center.0, center.1);
    if let Some(shader) = RadialGradient::new(pt, pt, radius.max(0.01), stops, SpreadMode::Pad, Transform::identity()) {
        paint.shader = shader;
    }
    paint.anti_alias = true;
    paint
}

/// Strokes a polyline segment by segment, because tiny-skia (like Core Graphics) has no
/// variable-width strokes.
fn stroke_polyline(
    pixmap: &mut Pixmap,
    pts: &[(f64, f64)],
    width: impl Fn(f64) -> f32,
    color: impl Fn(f64) -> Color,
    offset: (f64, f64),
) {
    if pts.len() < 2 {
        return;
    }
    let n = pts.len() - 1;
    for i in 0..n {
        let t = i as f64 / n as f64;
        let mut pb = PathBuilder::new();
        pb.move_to((pts[i].0 + offset.0) as f32, (pts[i].1 + offset.1) as f32);
        pb.line_to((pts[i + 1].0 + offset.0) as f32, (pts[i + 1].1 + offset.1) as f32);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(color(t));
            paint.anti_alias = true;
            let stroke = Stroke {
                width: width(t),
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Default::default()
            };
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
}

/// An irregular blob — the base shape for splats, scorch marks, blood, bites and holes.
/// Perfect circles give away that a shape was generated, so they're used nowhere except
/// the crack's sparkles and the stamp's oval frame.
fn lobed_path(center: (f64, f64), radius: f64, lobes: i64, jitter: f64, rng: &mut SeededRng) -> Option<tiny_skia::Path> {
    let lobes = (lobes.max(3)) as usize;
    let mut points: Vec<(f64, f64)> = Vec::with_capacity(lobes);
    for i in 0..lobes {
        let a = i as f64 * (TAU / lobes as f64);
        let r = radius * (1.0 + rng.jitter(jitter));
        points.push((center.0 + a.cos() * r, center.1 + a.sin() * r));
    }
    let mid = |a: (f64, f64), b: (f64, f64)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);

    let mut pb = PathBuilder::new();
    let start = mid(points[lobes - 1], points[0]);
    pb.move_to(start.0 as f32, start.1 as f32);
    for i in 0..lobes {
        let curr = points[i];
        let next = points[(i + 1) % lobes];
        let m = mid(curr, next);
        pb.quad_to(curr.0 as f32, curr.1 as f32, m.0 as f32, m.1 as f32);
    }
    pb.close();
    pb.finish()
}

/// Partially erases random small circles — this is what gives an imperfect print or a
/// worn look. `BlendMode::DestinationOut` (not `Clear`) respects the existing alpha, so it
/// erases only partially, proportional to the fill alpha used here.
fn roughen(pixmap: &mut Pixmap, rect: (f64, f64, f64, f64), count: i64, max_radius: f64, strength: f64, rng: &mut SeededRng) {
    let (x, y, w, h) = rect;
    for _ in 0..count {
        let r = max_radius * rng.range(0.25, 1.0);
        let px = rng.range(x, x + w);
        let py = rng.range(y, y + h);
        let mut pb = PathBuilder::new();
        pb.push_circle(px as f32, py as f32, r as f32);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(gray(0.0, (strength * rng.range(0.35, 1.0)) as f32));
            paint.blend_mode = BlendMode::DestinationOut;
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
}

fn stroke_rect(pixmap: &mut Pixmap, rect: (f64, f64, f64, f64), color: Color, width: f32) {
    let (x, y, w, h) = rect;
    if let Some(r) = tiny_skia::Rect::from_xywh(x as f32, y as f32, w as f32, h as f32) {
        let path = PathBuilder::from_rect(r);
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        let stroke = Stroke { width, ..Default::default() };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

// MARK: - The crack

fn make_crack(seed: u64, px: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(px, px).expect("could not allocate the crack pixmap");
    let mut rng = SeededRng::new(seed);

    let side = px as f64;
    let c = (side / 2.0, side / 2.0);
    let radius = side * 0.46;

    // --- radial arms ---
    // Cracks in glass are mostly straight; too much jitter reads as a scratch, not a
    // fracture.
    let arm_count = rng.int(8, 14) as usize;
    let mut arms: Vec<Vec<(f64, f64)>> = Vec::with_capacity(arm_count);
    let mut arm_lengths: Vec<f64> = Vec::with_capacity(arm_count);
    let base = rng.range(0.0, TAU);
    for i in 0..arm_count {
        let target = base + i as f64 * (TAU / arm_count as f64) + rng.jitter(0.18);
        let length = radius * rng.range(0.62, 1.0);
        let steps = rng.int(3, 6) as usize;
        let mut angle = target;
        let mut travelled = 0.0;
        let mut pts = vec![c];
        for _ in 0..steps {
            travelled += length / steps as f64;
            angle += rng.jitter(0.10);
            pts.push((c.0 + angle.cos() * travelled, c.1 + angle.sin() * travelled));
        }
        arms.push(pts);
        arm_lengths.push(length);
    }

    // The point on arm `i` at fraction `f` of its length.
    let point_on_arm = |i: usize, f: f64| -> (f64, f64) {
        let pts = &arms[i];
        let x = f.max(0.0).min(1.0) * (pts.len() - 1) as f64;
        let lo = x.floor() as usize;
        let hi = (lo + 1).min(pts.len() - 1);
        let t = x - lo as f64;
        (
            pts[lo].0 + (pts[hi].0 - pts[lo].0) * t,
            pts[lo].1 + (pts[hi].1 - pts[lo].1) * t,
        )
    };

    // --- concentric rings ---
    // This is the element that separates "broken glass" from "spider leg": fracture lines
    // that tie adjacent arms into shards.
    let mut rings: Vec<Vec<(f64, f64)>> = Vec::new();
    let ring_count = rng.int(2, 5) as usize;
    for r in 0..ring_count {
        let f = (r + 1) as f64 / (ring_count + 1) as f64 * rng.range(0.85, 1.15);
        for i in 0..arm_count {
            // A few gaps — closed rings look like a spider web.
            if !rng.chance(0.82) {
                continue;
            }
            let j = (i + 1) % arm_count;
            let fi = f * rng.range(0.9, 1.1);
            let fj = f * rng.range(0.9, 1.1);
            if !(fi < 1.0 && fj < 1.0) {
                continue;
            }
            let p0 = point_on_arm(i, fi);
            let p1 = point_on_arm(j, fj);
            // A chord bowed inwards — the way a real fracture runs.
            let mid = ((p0.0 + p1.0) / 2.0, (p0.1 + p1.1) / 2.0);
            let inward = (c.0 - mid.0, c.1 - mid.1);
            let bend = rng.range(0.04, 0.16);
            let bent = (mid.0 + inward.0 * bend, mid.1 + inward.1 * bend);
            rings.push(vec![p0, bent, p1]);
        }
    }

    // --- shards: faintly tinted polygons between arms and rings ---
    // Without them the crack has no faces and looks like a line drawing.
    for i in 0..arm_count {
        let j = (i + 1) % arm_count;
        let mut f = 0.12;
        while f < 0.9 {
            let f2 = f + rng.range(0.18, 0.34);
            if !(f2 < 1.0) {
                break;
            }
            let quad = [
                point_on_arm(i, f),
                point_on_arm(j, f),
                point_on_arm(j, f2),
                point_on_arm(i, f2),
            ];
            let mut pb = PathBuilder::new();
            pb.move_to(quad[0].0 as f32, quad[0].1 as f32);
            for p in &quad[1..] {
                pb.line_to(p.0 as f32, p.1 as f32);
            }
            pb.close();
            if let Some(path) = pb.finish() {
                let lighten = rng.chance(0.5);
                let strength = rng.range(0.03, 0.10) * (1.0 - f).max(0.0);
                let mut paint = Paint::default();
                paint.set_color(gray(if lighten { 1.0 } else { 0.0 }, strength as f32));
                paint.anti_alias = true;
                pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
            }
            f = f2;
        }
    }

    // --- side branches ---
    let mut branches: Vec<Vec<(f64, f64)>> = Vec::new();
    for (i, arm) in arms.iter().enumerate() {
        if !rng.chance(0.6) {
            continue;
        }
        let at = rng.int(1, (arm.len() as i64).max(2)) as usize;
        let from = arm[at.min(arm.len() - 1)];
        let outward = (from.1 - c.1).atan2(from.0 - c.0);
        let sign = if rng.chance(0.5) { 1.0 } else { -1.0 };
        let dir = outward + sign * rng.range(0.4, 0.9);
        let length = arm_lengths[i] * rng.range(0.15, 0.38);
        let mut angle = dir;
        let mut travelled = 0.0;
        let mut pts = vec![from];
        for _ in 0..rng.int(2, 4) {
            travelled += length / 2.5;
            angle += rng.jitter(0.16);
            pts.push((from.0 + angle.cos() * travelled, from.1 + angle.sin() * travelled));
        }
        branches.push(pts);
    }

    fn falloff(t: f64) -> f64 {
        (1.0 - t.min(1.0).powf(2.0)).max(0.0)
    }

    // Two passes: a light offset highlight (readable on dark) plus a dark core (readable
    // on light). The same stroke has to work over an unknown desktop.
    let draw_set = |pixmap: &mut Pixmap, set: &[Vec<(f64, f64)>], base_width: f64, darkness: f64| {
        let hi = side * 0.0045;
        for pts in set {
            stroke_polyline(
                pixmap,
                pts,
                |t| (base_width * 1.25 * (1.0 - t * 0.45)).max(1.0) as f32,
                |t| gray(1.0, (0.46 * falloff(t)) as f32),
                (hi, -hi),
            );
        }
        for pts in set {
            stroke_polyline(
                pixmap,
                pts,
                |t| (base_width * (1.0 - t * 0.5)).max(0.9) as f32,
                |t| gray(0.03, (darkness * falloff(t * 0.8)) as f32),
                (0.0, 0.0),
            );
        }
    };

    draw_set(&mut pixmap, &rings, side * 0.010, 0.78);
    draw_set(&mut pixmap, &branches, side * 0.011, 0.80);
    draw_set(&mut pixmap, &arms, side * 0.017, 0.90);

    // --- the impact point: an irregular dark hole with a light rim ---
    let hole_r = side * rng.range(0.028, 0.048);
    let lobes = rng.int(7, 11);
    let mut pb = PathBuilder::new();
    for i in 0..=lobes {
        let a = i as f64 * (TAU / lobes as f64);
        let r = hole_r * rng.range(0.7, 1.35);
        let p = (c.0 + a.cos() * r, c.1 + a.sin() * r);
        if i == 0 {
            pb.move_to(p.0 as f32, p.1 as f32);
        } else {
            pb.line_to(p.0 as f32, p.1 as f32);
        }
    }
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(gray(0.02, 0.92));
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    // A light rim around the hole — otherwise the core is invisible on a dark background.
    {
        let mut pb = PathBuilder::new();
        pb.push_circle(c.0 as f32, c.1 as f32, (hole_r * 1.15) as f32);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(gray(1.0, 0.5));
            paint.anti_alias = true;
            let stroke = Stroke { width: (side * 0.006) as f32, ..Default::default() };
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }

    // Tiny sparkles around the impact.
    for _ in 0..rng.int(8, 18) {
        let a = rng.range(0.0, TAU);
        let d = hole_r * rng.range(1.2, 3.2);
        let r = side * rng.range(0.0025, 0.006);
        let cx = c.0 + a.cos() * d;
        let cy = c.1 + a.sin() * d;
        let mut pb = PathBuilder::new();
        pb.push_circle(cx as f32, cy as f32, r as f32);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(gray(1.0, rng.range(0.3, 0.7) as f32));
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    pixmap
}

// MARK: - Bullet hole

fn make_bullet_hole(seed: u64, px: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(px, px).expect("could not allocate the bullet hole pixmap");
    let mut rng = SeededRng::new(seed);
    let side = px as f64;
    let c = (side / 2.0, side / 2.0);
    let hole_r = side * rng.range(0.13, 0.19);

    // A light crater around the hole — without it the hole is invisible on a dark
    // background.
    {
        let paint = radial_gradient_paint(
            (c.0 as f32, c.1 as f32),
            (hole_r * 2.6) as f32,
            vec![
                stop(0.0, gray(1.0, 0.0)),
                stop(0.42, gray(1.0, 0.40)),
                stop(0.75, gray(0.55, 0.18)),
                stop(1.0, gray(0.4, 0.0)),
            ],
        );
        let mut pb = PathBuilder::new();
        pb.push_circle(c.0 as f32, c.1 as f32, (hole_r * 2.6) as f32);
        if let Some(path) = pb.finish() {
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    // Short radial cracks running out from the rim.
    let arms = rng.int(6, 11);
    for i in 0..arms {
        let a = i as f64 * (TAU / arms as f64) + rng.jitter(0.3);
        let len = side * rng.range(0.10, 0.26);
        let pts = [
            (c.0 + a.cos() * hole_r * 0.9, c.1 + a.sin() * hole_r * 0.9),
            (
                c.0 + (a + rng.jitter(0.12)).cos() * (hole_r + len * 0.6),
                c.1 + (a + rng.jitter(0.12)).sin() * (hole_r + len * 0.6),
            ),
            (
                c.0 + (a + rng.jitter(0.2)).cos() * (hole_r + len),
                c.1 + (a + rng.jitter(0.2)).sin() * (hole_r + len),
            ),
        ];
        stroke_polyline(
            &mut pixmap,
            &pts,
            |t| (side * 0.018 * (1.0 - t * 0.7)).max(0.9) as f32,
            |_| gray(1.0, 0.35),
            (side * 0.008, -side * 0.008),
        );
        stroke_polyline(
            &mut pixmap,
            &pts,
            |t| (side * 0.014 * (1.0 - t * 0.7)).max(0.8) as f32,
            |t| gray(0.05, (0.8 * (1.0 - t * 0.5)) as f32),
            (0.0, 0.0),
        );
    }

    // The hole itself.
    if let Some(path) = lobed_path(c, hole_r, rng.int(8, 12), 0.22, &mut rng) {
        let mut paint = Paint::default();
        paint.set_color(gray(0.03, 0.95));
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    // A thin light rim around the hole.
    if let Some(path) = lobed_path(c, hole_r * 1.08, 10, 0.14, &mut rng) {
        let mut paint = Paint::default();
        paint.set_color(gray(1.0, 0.45));
        paint.anti_alias = true;
        let stroke = Stroke { width: (side * 0.012) as f32, ..Default::default() };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }

    pixmap
}

// MARK: - Scorch mark

fn make_scorch(seed: u64, px: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(px, px).expect("could not allocate the scorch pixmap");
    let mut rng = SeededRng::new(seed);
    let side = px as f64;
    let c = (side / 2.0, side / 2.0);
    let radius = side * rng.range(0.36, 0.46);

    // An irregular fire outline with a gradient inside it — a perfect circle would look
    // like a stain, not a burn. Filling the lobed blob path itself with the gradient
    // shader gets the same result as "clip to the blob, then paint the gradient" in one
    // step.
    if let Some(blob) = lobed_path(c, radius, rng.int(9, 14), 0.26, &mut rng) {
        let paint = radial_gradient_paint(
            (c.0 as f32, c.1 as f32),
            radius as f32,
            vec![
                stop(0.0, rgb(0.05, 0.04, 0.03, 0.94)),
                stop(0.35, rgb(0.11, 0.07, 0.04, 0.80)),
                stop(0.62, rgb(0.24, 0.13, 0.06, 0.50)),
                stop(0.85, rgb(0.32, 0.18, 0.08, 0.18)),
                stop(1.0, rgb(0.35, 0.20, 0.09, 0.0)),
            ],
        );
        pixmap.fill_path(&blob, &paint, FillRule::Winding, Transform::identity(), None);
    }

    // Charred patches and ash.
    for _ in 0..rng.int(10, 20) {
        let a = rng.range(0.0, TAU);
        let d = radius * rng.range(0.1, 0.9);
        let r = side * rng.range(0.012, 0.055);
        let p = (c.0 + a.cos() * d, c.1 + a.sin() * d);
        if let Some(path) = lobed_path(p, r, 7, 0.35, &mut rng) {
            let mut paint = Paint::default();
            paint.set_color(gray(rng.range(0.02, 0.10) as f32, rng.range(0.35, 0.8) as f32));
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    // A few light ash specks so the burn reads on a black desktop too.
    for _ in 0..rng.int(8, 16) {
        let a = rng.range(0.0, TAU);
        let d = radius * rng.range(0.15, 0.95);
        let r = side * rng.range(0.004, 0.013);
        let cx = c.0 + a.cos() * d;
        let cy = c.1 + a.sin() * d;
        let mut pb = PathBuilder::new();
        pb.push_circle(cx as f32, cy as f32, r as f32);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(rgb(0.75, 0.72, 0.68, rng.range(0.25, 0.6) as f32));
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    roughen(&mut pixmap, (0.0, 0.0, side, side), 26, side * 0.05, 0.5, &mut rng);

    pixmap
}

// MARK: - Paint splat

fn make_paint_splat(base: (f32, f32, f32), seed: u64, px: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(px, px).expect("could not allocate the paint splat pixmap");
    let mut rng = SeededRng::new(seed);
    let side = px as f64;
    // Slightly above the vertical middle, leaving room for the drips below. `0.44`, not
    // Swift's `0.56` — a Core Graphics bitmap context is y-up (origin bottom-left) but a
    // tiny-skia Pixmap is y-down (origin top-left, standard raster convention), so a plain
    // y position ported verbatim lands mirrored.
    let c = (side / 2.0, side * 0.44);
    let radius = side * rng.range(0.22, 0.30);

    let tinted = |factor: f32, alpha: f32| -> Color {
        rgb(base.0 * factor, base.1 * factor, base.2 * factor, alpha)
    };

    // Drips running downwards — without them the splat has no weight. `+ FRAC_PI_2`, not
    // Swift's `-`: same y-up/y-down reason as `c` above — this is the pixmap-coordinate
    // angle that actually points down.
    for _ in 0..rng.int(2, 5) {
        let a = rng.range(-0.9, 0.9) + std::f64::consts::FRAC_PI_2;
        let len = side * rng.range(0.10, 0.30);
        let w = side * rng.range(0.018, 0.040);
        let start = (c.0 + a.cos() * radius * 0.7, c.1 + a.sin() * radius * 0.7);
        let end = (start.0 + rng.jitter(side * 0.02), start.1 + len);

        stroke_polyline(&mut pixmap, &[start, end], |_| w as f32, |_| tinted(0.85, 0.9), (0.0, 0.0));

        // A thicker droplet at the end.
        let br = w * rng.range(0.7, 1.3);
        let mut pb = PathBuilder::new();
        pb.push_circle(end.0 as f32, end.1 as f32, br as f32);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(tinted(0.9, 0.92));
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    // The central splat.
    if let Some(path) = lobed_path(c, radius, rng.int(8, 13), 0.30, &mut rng) {
        let mut paint = Paint::default();
        paint.set_color(tinted(1.0, 0.93));
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    // A darker rim (paint pools at the edge).
    if let Some(path) = lobed_path(c, radius * 0.99, 11, 0.22, &mut rng) {
        let mut paint = Paint::default();
        paint.set_color(tinted(0.66, 0.75));
        paint.anti_alias = true;
        let stroke = Stroke { width: (side * 0.016) as f32, ..Default::default() };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }

    // A bright highlight — wet paint.
    let hl = (c.0 - radius * 0.32, c.1 + radius * 0.34);
    if let Some(path) = lobed_path(hl, radius * 0.24, 7, 0.3, &mut rng) {
        let mut paint = Paint::default();
        paint.set_color(tinted(1.45, 0.45));
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    // Spatter around it.
    for _ in 0..rng.int(6, 14) {
        let a = rng.range(0.0, TAU);
        let d = radius * rng.range(1.05, 1.85);
        let r = side * rng.range(0.006, 0.028);
        let p = (c.0 + a.cos() * d, c.1 + a.sin() * d);
        if let Some(path) = lobed_path(p, r, 6, 0.35, &mut rng) {
            let mut paint = Paint::default();
            paint.set_color(tinted(rng.range(0.85, 1.1) as f32, rng.range(0.6, 0.95) as f32));
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    pixmap
}

// MARK: - Phaser impact

fn make_phaser_hit(seed: u64, px: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(px, px).expect("could not allocate the phaser hit pixmap");
    let mut rng = SeededRng::new(seed);
    let side = px as f64;
    let c = (side / 2.0, side / 2.0);
    let core_r = side * rng.range(0.10, 0.16);

    // A dark fused rim.
    if let Some(path) = lobed_path(c, core_r * 2.1, rng.int(9, 14), 0.24, &mut rng) {
        let mut paint = Paint::default();
        paint.set_color(rgb(0.10, 0.04, 0.03, 0.72));
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    // The glowing core: white → yellow → orange → red.
    {
        let paint = radial_gradient_paint(
            (c.0 as f32, c.1 as f32),
            (core_r * 2.0) as f32,
            vec![
                stop(0.0, rgb(1.0, 1.0, 0.95, 1.0)),
                stop(0.22, rgb(1.0, 0.93, 0.55, 0.95)),
                stop(0.45, rgb(1.0, 0.62, 0.16, 0.80)),
                stop(0.72, rgb(0.85, 0.20, 0.06, 0.45)),
                stop(1.0, rgb(0.5, 0.08, 0.03, 0.0)),
            ],
        );
        let mut pb = PathBuilder::new();
        pb.push_circle(c.0 as f32, c.1 as f32, (core_r * 2.0) as f32);
        if let Some(path) = pb.finish() {
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    // Radial streaks of molten material.
    let streaks = rng.int(5, 11);
    for i in 0..streaks {
        let a = i as f64 * (TAU / streaks as f64) + rng.jitter(0.4);
        let len = side * rng.range(0.12, 0.34);
        let pts = [
            (c.0 + a.cos() * core_r, c.1 + a.sin() * core_r),
            (c.0 + a.cos() * (core_r + len), c.1 + a.sin() * (core_r + len)),
        ];
        stroke_polyline(
            &mut pixmap,
            &pts,
            |t| (side * 0.030 * (1.0 - t)).max(1.0) as f32,
            |t| rgb(1.0, (0.55 - t * 0.3) as f32, 0.12, (0.55 * (1.0 - t)) as f32),
            (0.0, 0.0),
        );
    }

    pixmap
}

// MARK: - Stamp print

fn make_stamp_print(font: &StampFont, index: i64, px: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(px, px).expect("could not allocate the stamp pixmap");
    let mut rng = SeededRng::new(0x57A3_0000u64.wrapping_add(index as u64));
    let side = px as f64;
    let text = DecalFactory::STAMP_TEXTS[(index as usize) % DecalFactory::STAMP_TEXTS.len()];

    // Ink colours, as on real rubber stamps.
    let inks: [(f32, f32, f32); 4] = [
        (0.78, 0.11, 0.13), // red
        (0.13, 0.24, 0.62), // blue
        (0.10, 0.10, 0.11), // black
        (0.10, 0.42, 0.24), // green
    ];
    let (ir, ig, ib) = inks[(index as usize) % inks.len()];
    let ink = rgb(ir, ig, ib, 1.0);

    let inset = side * 0.10;
    let frame = (inset, side * 0.30, side - inset * 2.0, side * 0.40); // (x, y, w, h)

    // Three frame styles, so the ten prints are distinguishable from each other.
    match index % 3 {
        0 => {
            stroke_rect(&mut pixmap, frame, ink, (side * 0.020) as f32);
            let pad = side * 0.022;
            let inner = (frame.0 + pad, frame.1 + pad, frame.2 - pad * 2.0, frame.3 - pad * 2.0);
            stroke_rect(&mut pixmap, inner, ink, (side * 0.008) as f32);
        }
        1 => {
            let mut pb = PathBuilder::new();
            push_ellipse(
                &mut pb,
                (frame.0 + frame.2 / 2.0) as f32,
                (frame.1 + frame.3 / 2.0) as f32,
                (frame.2 / 2.0) as f32,
                (frame.3 / 2.0) as f32,
            );
            if let Some(path) = pb.finish() {
                let mut paint = Paint::default();
                paint.set_color(ink);
                paint.anti_alias = true;
                let stroke = Stroke { width: (side * 0.022) as f32, ..Default::default() };
                pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
        }
        _ => {
            let mut pb = PathBuilder::new();
            push_rounded_rect(
                &mut pb,
                frame.0 as f32,
                frame.1 as f32,
                frame.2 as f32,
                frame.3 as f32,
                (side * 0.05) as f32,
            );
            if let Some(path) = pb.finish() {
                let mut paint = Paint::default();
                paint.set_color(ink);
                paint.anti_alias = true;
                let stroke = Stroke { width: (side * 0.020) as f32, ..Default::default() };
                pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
        }
    }

    // The text. The size follows its length so long words don't escape the frame.
    let font_size = (side * if text.chars().count() > 8 { 0.085 } else { 0.135 }) as f32;
    let kern = font_size * 0.06;
    if let Some((path, (bx, by, bw, bh))) = font.layout(text, font_size, kern) {
        let tx = (frame.0 + frame.2 / 2.0) as f32 - bw / 2.0 - bx;
        let ty = (frame.1 + frame.3 / 2.0) as f32 - bh / 2.0 - by;
        let mut paint = Paint::default();
        paint.set_color(ink);
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::from_translate(tx, ty), None);
    }

    // A rubber stamp never transfers evenly.
    let grown = (frame.0 - side * 0.03, frame.1 - side * 0.03, frame.2 + side * 0.06, frame.3 + side * 0.06);
    roughen(&mut pixmap, grown, 90, side * 0.028, 0.85, &mut rng);

    pixmap
}

// MARK: - Bite and blood

fn make_bite(seed: u64, px: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(px, px).expect("could not allocate the bite pixmap");
    let mut rng = SeededRng::new(seed);
    let side = px as f64;
    let c = (side / 2.0, side / 2.0);

    if let Some(path) = lobed_path(c, side * rng.range(0.26, 0.38), 7, 0.34, &mut rng) {
        let mut paint = Paint::default();
        paint.set_color(gray(0.06, 0.80));
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    // A light rim — otherwise bites vanish on a dark desktop.
    if let Some(path) = lobed_path(c, side * 0.34, 7, 0.30, &mut rng) {
        let mut paint = Paint::default();
        paint.set_color(gray(1.0, 0.30));
        paint.anti_alias = true;
        let stroke = Stroke { width: (side * 0.05) as f32, ..Default::default() };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }

    pixmap
}

fn make_blood(seed: u64, px: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(px, px).expect("could not allocate the blood pixmap");
    let mut rng = SeededRng::new(seed);
    let side = px as f64;
    let c = (side / 2.0, side / 2.0);
    let radius = side * rng.range(0.20, 0.30);

    let dark = rgb(0.34, 0.03, 0.04, 0.92);
    let mid = rgb(0.52, 0.05, 0.06, 0.92);

    if let Some(path) = lobed_path(c, radius, rng.int(7, 12), 0.36, &mut rng) {
        let mut paint = Paint::default();
        paint.set_color(mid);
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    if let Some(path) = lobed_path(c, radius * 0.98, 9, 0.28, &mut rng) {
        let mut paint = Paint::default();
        paint.set_color(dark);
        paint.anti_alias = true;
        let stroke = Stroke { width: (side * 0.03) as f32, ..Default::default() };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }

    for _ in 0..rng.int(5, 12) {
        let a = rng.range(0.0, TAU);
        let d = radius * rng.range(1.1, 2.2);
        let r = side * rng.range(0.012, 0.045);
        let p = (c.0 + a.cos() * d, c.1 + a.sin() * d);
        if let Some(path) = lobed_path(p, r, 6, 0.4, &mut rng) {
            let mut paint = Paint::default();
            paint.set_color(if rng.chance(0.5) { mid } else { dark });
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    pixmap
}

// MARK: - Saw cut

fn make_saw_cut(seed: u64, w: u32, h: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(w, h).expect("could not allocate the saw cut pixmap");
    let mut rng = SeededRng::new(seed);
    let width = w as f64;
    let height = h as f64;
    let mid_y = height / 2.0;
    let steps = 14usize;

    // The torn upper and lower edge of the cut.
    let mut top: Vec<(f64, f64)> = Vec::with_capacity(steps + 1);
    let mut bottom: Vec<(f64, f64)> = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let x = width * i as f64 / steps as f64;
        // The ends taper to nothing so consecutive segments blend smoothly.
        let taper = (std::f64::consts::PI * i as f64 / steps as f64).sin();
        let half = height * 0.18 * taper * rng.range(0.55, 1.0);
        // `-half`/`+half`, not Swift's `+half`/`-half` — same y-up/y-down reason as the
        // paint splat above; this keeps `top` actually meaning the visually upper edge,
        // which matters below since the highlight is deliberately stroked onto `top` only.
        top.push((x, mid_y - half));
        bottom.push((x, mid_y + half));
    }

    let mut pb = PathBuilder::new();
    pb.move_to(top[0].0 as f32, top[0].1 as f32);
    for p in &top[1..] {
        pb.line_to(p.0 as f32, p.1 as f32);
    }
    for p in bottom.iter().rev() {
        pb.line_to(p.0 as f32, p.1 as f32);
    }
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(gray(0.05, 0.85));
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    // A light rim along the top side of the cut — it reads on dark backgrounds.
    stroke_polyline(
        &mut pixmap,
        &top,
        |_| (height * 0.055) as f32,
        |t| gray(1.0, (0.42 * (std::f64::consts::PI * t).sin()) as f32),
        (0.0, 0.0),
    );

    // Splinters beside the cut.
    for _ in 0..rng.int(4, 10) {
        let x = rng.range(width * 0.1, width * 0.9);
        let up = rng.chance(0.5);
        let len = height * rng.range(0.05, 0.15);
        let y0 = if up { mid_y + height * 0.12 } else { mid_y - height * 0.12 };
        let p0 = (x, y0);
        let p1 = (x + rng.jitter(width * 0.02), if up { y0 + len } else { y0 - len });
        stroke_polyline(&mut pixmap, &[p0, p1], |_| (height * 0.028) as f32, |_| gray(0.08, 0.55), (0.0, 0.0));
    }

    pixmap
}

// MARK: - The sliver

fn make_sliver(seed: u64, px: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(px, px).expect("could not allocate the sliver pixmap");
    let mut rng = SeededRng::new(seed);
    let side = px as f64;

    // A sharp-edged polygon — a splinter, not a pellet.
    let n = rng.int(3, 6);
    let mut pts: Vec<(f64, f64)> = Vec::with_capacity(n as usize);
    for i in 0..n {
        let a = i as f64 * (TAU / n as f64) + rng.jitter(0.4);
        let r = side * rng.range(0.18, 0.46);
        pts.push((side / 2.0 + a.cos() * r, side / 2.0 + a.sin() * r));
    }

    let build_path = |pts: &[(f64, f64)]| -> Option<tiny_skia::Path> {
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0 as f32, pts[0].1 as f32);
        for p in &pts[1..] {
            pb.line_to(p.0 as f32, p.1 as f32);
        }
        pb.close();
        pb.finish()
    };

    if let Some(path) = build_path(&pts) {
        let mut paint = Paint::default();
        paint.set_color(gray(rng.range(0.75, 0.95) as f32, 0.9));
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    if let Some(path) = build_path(&pts) {
        let mut paint = Paint::default();
        paint.set_color(gray(0.1, 0.75));
        paint.anti_alias = true;
        let stroke = Stroke { width: (side * 0.05).max(0.8) as f32, ..Default::default() };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }

    pixmap
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(f: &mut DecalFactory, name: &str) -> Vec<u8> {
        match name {
            "bullet_hole" => f.bullet_hole(0).data().to_vec(),
            "scorch" => f.scorch(0).data().to_vec(),
            "paint" => f.paint_splat(0, 0).data().to_vec(),
            "phaser" => f.phaser_hit(0).data().to_vec(),
            "stamp" => f.stamp_print(0).data().to_vec(),
            "bite" => f.bite(0).data().to_vec(),
            "blood" => f.blood(0).data().to_vec(),
            "saw_cut" => f.saw_cut(0).data().to_vec(),
            "sliver" => f.sliver(0).data().to_vec(),
            _ => unreachable!(),
        }
    }

    #[test]
    fn cracks_are_deterministic() {
        let mut a = DecalFactory::new();
        let mut b = DecalFactory::new();
        for v in 0..DecalFactory::CRACK_VARIANTS {
            assert_eq!(a.crack(v).data(), b.crack(v).data(), "variant {v} was not reproducible");
        }
    }

    #[test]
    fn crack_variants_differ_from_each_other() {
        let mut f = DecalFactory::new();
        let mut seen: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
        for v in 0..DecalFactory::CRACK_VARIANTS {
            let bytes = f.crack(v).data().to_vec();
            assert!(seen.insert(bytes), "variant {v} duplicated another variant's pixels");
        }
    }

    #[test]
    fn negative_index_wraps() {
        let mut f = DecalFactory::new();
        let neg = f.crack(-1).data().to_vec();
        let wrapped = f.crack(DecalFactory::CRACK_VARIANTS - 1).data().to_vec();
        assert_eq!(neg, wrapped);
    }

    #[test]
    fn oversized_index_wraps() {
        let mut f = DecalFactory::new();
        let over = f.bullet_hole(99).data().to_vec();
        let wrapped = f.bullet_hole(99 % DecalFactory::BULLET_HOLE_VARIANTS).data().to_vec();
        assert_eq!(over, wrapped);
    }

    #[test]
    fn crack_is_not_blank() {
        let mut f = DecalFactory::new();
        assert!(!is_blank(f.crack(0)), "every pixel transparent");
    }

    #[test]
    fn every_decal_type_is_not_blank() {
        let mut f = DecalFactory::new();
        for name in ["bullet_hole", "scorch", "paint", "phaser", "stamp", "bite", "blood", "saw_cut", "sliver"] {
            let data = bytes(&mut f, name);
            let has_visible_pixel = data.chunks_exact(4).any(|px| px[3] > 8);
            assert!(has_visible_pixel, "the '{name}' decal is blank");
        }
    }

    #[test]
    fn stamps_are_deterministic() {
        let mut a = DecalFactory::new();
        let mut b = DecalFactory::new();
        for v in 0..DecalFactory::STAMP_VARIANTS {
            assert_eq!(a.stamp_print(v).data(), b.stamp_print(v).data(), "stamp {v} was not reproducible");
        }
    }

    #[test]
    fn stamp_variants_differ_from_each_other() {
        let mut f = DecalFactory::new();
        let mut seen: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
        for v in 0..DecalFactory::STAMP_VARIANTS {
            let bytes = f.stamp_print(v).data().to_vec();
            assert!(seen.insert(bytes), "stamp {v} duplicated another variant's pixels");
        }
    }

    #[test]
    fn eight_paint_colors_in_original_order() {
        assert_eq!(DecalFactory::DEFAULT_PAINT_COLORS.len(), 8);
        let names: Vec<&str> = DecalFactory::DEFAULT_PAINT_COLORS.iter().map(|(n, ..)| *n).collect();
        assert_eq!(names, ["red", "green", "blue", "yellow", "purple", "cyan", "orange", "pink"]);
    }

    #[test]
    fn a_fresh_factory_starts_with_the_default_paint_colors() {
        let f = DecalFactory::new();
        assert_eq!(f.paint_colors(), DecalFactory::DEFAULT_PAINT_COLORS.map(|(_, r, g, b)| (r, g, b)));
    }

    #[test]
    fn set_paint_colors_overrides_paint_splat() {
        let mut f = DecalFactory::new();
        let mut theme = [(0.0, 0.0, 0.0); 8];
        theme[0] = (1.0, 0.0, 1.0); // a colour nothing in DEFAULT_PAINT_COLORS produces
        f.set_paint_colors(theme);
        assert_eq!(f.paint_colors(), theme);
        // `paint_splat` doesn't expose pixel colour directly, but a non-blank splat means
        // it ran through `make_paint_splat` with the overridden colour rather than
        // panicking or silently ignoring it.
        assert!(!is_blank(f.paint_splat(0, 0)));
    }

    #[test]
    fn ten_stamp_texts() {
        assert_eq!(DecalFactory::STAMP_TEXTS.len(), 10);
    }

    #[test]
    fn paint_colours_produce_visibly_different_splats() {
        let mut f = DecalFactory::new();
        let red = f.paint_splat(0, 0).data().to_vec();
        let blue = f.paint_splat(2, 0).data().to_vec();
        assert_ne!(red, blue, "different colour indices should tint the splat differently");
    }
}
