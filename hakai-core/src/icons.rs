//! Procedural tool icons.
//!
//! Ported from `IconBuilder.swift` + `ToolIcons.swift` + `ToolIcons+Weapons.swift`. Same
//! reasoning as the decals: everything generated in code, no bundled image assets.
//!
//! **Coordinate convention.** Every icon is authored in a nominal 256×256 space using
//! Swift's exact numeric literals — but a Core Graphics bitmap context is y-up (origin
//! bottom-left) and a `tiny_skia::Pixmap` is y-down (origin top-left, standard raster
//! convention). Rather than hand-reason "does this offset mean up or down" at every one
//! of the many call sites below — which is exactly how the paint splat bug in `decals.rs`
//! happened — every coordinate here is computed with Swift's untouched y-up arithmetic and
//! flipped to screen space in exactly one place, [`IconBuilder::flip`], right before it
//! reaches a `PathBuilder` call. Get that one function right and every icon's orientation
//! follows for free.

use std::collections::HashMap;

use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

use crate::geometry::{push_capsule, push_ellipse, push_rounded_rect_mapped};

// MARK: - Palette

/// Shared colours, so the tools look like one visual set.
pub mod palette {
    use tiny_skia::Color;

    fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
        let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color::from_rgba8(to_u8(r), to_u8(g), to_u8(b), to_u8(a))
    }
    fn rgb(r: f32, g: f32, b: f32) -> Color {
        rgba(r, g, b, 1.0)
    }
    fn gray(g: f32) -> Color {
        rgb(g, g, g)
    }
    pub fn graya(g: f32, a: f32) -> Color {
        rgba(g, g, g, a)
    }
    pub fn rgbaf(r: f32, g: f32, b: f32, a: f32) -> Color {
        rgba(r, g, b, a)
    }

    pub fn steel() -> Color { gray(0.58) }
    pub fn steel_dark() -> Color { gray(0.34) }
    pub fn gunmetal() -> Color { gray(0.24) }
    pub fn plastic_dark() -> Color { gray(0.16) }
    pub fn wood() -> Color { rgb(0.42, 0.27, 0.14) }
    pub fn wood_light() -> Color { rgb(0.62, 0.44, 0.25) }
    pub fn orange() -> Color { rgb(0.90, 0.36, 0.07) }
    pub fn orange_dark() -> Color { rgb(0.66, 0.24, 0.04) }
    pub fn red() -> Color { rgb(0.78, 0.14, 0.12) }
    pub fn blue() -> Color { rgb(0.16, 0.42, 0.78) }
    pub fn cyan_glow() -> Color { rgb(0.35, 0.90, 1.0) }
    pub fn skin() -> Color { rgb(0.92, 0.74, 0.60) }
    pub fn skin_shade() -> Color { rgb(0.76, 0.56, 0.44) }
    pub fn highlight() -> Color { graya(1.0, 0.55) }
}

// MARK: - ToolIcon

/// A tool icon together with its hotspot.
pub struct ToolIcon {
    pub pixmap: Pixmap,
    /// Normalised 0..1, origin **top-left** (unlike the Swift original, which is
    /// normalised y-up to match SpriteKit) — the point that has to sit under the cursor,
    /// because that's where the tool acts.
    pub hotspot: (f32, f32),
    /// The point the sprite rotates around, when that isn't the hotspot. Only the hammer
    /// needs this — see `ToolIcon` in `ToolIcons.swift` for why.
    pub pivot: Option<(f32, f32)>,
    /// The recommended display size in points.
    pub point_size: (f32, f32),
}

// MARK: - IconBuilder

/// Every icon is drawn in a nominal 256×256 space and only then scaled to the actual
/// resolution, which keeps the numbers in the generators readable. Tools float above a
/// desktop of unknown colour, so every icon needs a dark outline around its whole
/// silhouette — the builder assembles that from every filled shape itself.
pub struct IconBuilder {
    pixmap: Pixmap,
    side: f32,
    scale: f32,
    silhouette: Vec<Path>,
}

impl IconBuilder {
    pub fn new(px: u32) -> Self {
        Self {
            pixmap: Pixmap::new(px, px).expect("could not allocate the icon pixmap"),
            side: px as f32,
            scale: px as f32 / 256.0,
            silhouette: Vec::new(),
        }
    }

    /// The one place the y-up→y-down flip happens — see the module doc comment.
    fn flip(&self, p: (f32, f32)) -> (f32, f32) {
        (p.0, self.side - p.1)
    }

    /// A point from the nominal 256×256 space into screen space.
    pub fn at(&self, x: f32, y: f32) -> (f32, f32) {
        self.flip((x * self.scale, y * self.scale))
    }

    pub fn s(&self, value: f32) -> f32 {
        value * self.scale
    }

    // --- Shapes (all already in screen space by the time they build a Path) ---

    pub fn capsule(&self, a: (f32, f32), b: (f32, f32), width: f32) -> Path {
        let mut pb = PathBuilder::new();
        push_capsule(&mut pb, a, b, width);
        pb.finish().expect("capsule path")
    }

    /// `x, y, w, h` are nominal, y-up, unscaled — same call convention as the Swift
    /// original (`b.roundRect(140, 76, 96, 84, radius: 16)`), not pre-mapped through `at`.
    pub fn round_rect(&self, x: f32, y: f32, w: f32, h: f32, radius: f32) -> Path {
        let (sx, sy, sw, sh, sr) = (x * self.scale, y * self.scale, w * self.scale, h * self.scale, radius * self.scale);
        // The flipped rect's screen-space top is the flip of its nominal *top* edge
        // (y + h), not its nominal origin — flipping reverses which edge is which.
        let screen_top = self.side - (sy + sh);
        let mut pb = PathBuilder::new();
        push_rounded_rect_mapped(&mut pb, sx, screen_top, sw, sh, sr, |px, py| (px, py));
        pb.finish().expect("round rect path")
    }

    pub fn polygon(&self, points: &[(f32, f32)]) -> Option<Path> {
        if points.is_empty() {
            return None;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(points[0].0, points[0].1);
        for p in &points[1..] {
            pb.line_to(p.0, p.1);
        }
        pb.close();
        pb.finish()
    }

    /// `cx, cy, rx, ry` are nominal, y-up, unscaled — same convention as `round_rect`.
    pub fn ellipse(&self, cx: f32, cy: f32, rx: f32, ry: f32) -> Path {
        let center = self.at(cx, cy);
        let mut pb = PathBuilder::new();
        push_ellipse(&mut pb, center.0, center.1, rx * self.scale, ry * self.scale);
        pb.finish().expect("ellipse path")
    }

    // --- Drawing ---

    /// `in_silhouette: false` for interior detail (highlights, buttons) that shouldn't
    /// contribute to the outer outline.
    pub fn fill(&mut self, path: &Path, color: Color, in_silhouette: bool) {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        self.pixmap.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
        if in_silhouette {
            self.silhouette.push(path.clone());
        }
    }

    pub fn stroke(&mut self, path: &Path, color: Color, width: f32) {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        let stroke = Stroke { width, ..Default::default() };
        self.pixmap.stroke_path(path, &paint, &stroke, Transform::identity(), None);
    }

    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), color: Color, width: f32) {
        let mut pb = PathBuilder::new();
        pb.move_to(a.0, a.1);
        pb.line_to(b.0, b.1);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(color);
            paint.anti_alias = true;
            let stroke = Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
            self.pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }

    /// Outlines the whole accumulated silhouette. Call this last, before `finish`.
    ///
    /// Stroking every contributing shape individually with the same solid, near-opaque
    /// colour is visually identical to Core Graphics' approach of merging them into one
    /// path first and stroking once — tiny-skia has no public "add path to path" — the
    /// only difference would be double-applied alpha at overlaps, which doesn't show
    /// with an opaque stroke colour.
    pub fn outline(&mut self) {
        let color = palette::graya(0.08, 0.9);
        let width = self.s(2.0);
        let paths = std::mem::take(&mut self.silhouette);
        for path in &paths {
            self.stroke(path, color, width);
        }
    }

    pub fn finish(self, hotspot: (f32, f32), pivot: Option<(f32, f32)>, point_size: f32) -> ToolIcon {
        ToolIcon {
            pixmap: self.pixmap,
            hotspot: (hotspot.0 / 256.0, 1.0 - hotspot.1 / 256.0),
            pivot: pivot.map(|p| (p.0 / 256.0, 1.0 - p.1 / 256.0)),
            point_size: (point_size, point_size),
        }
    }
}

// MARK: - ToolIcons

pub struct ToolIcons {
    cache: HashMap<&'static str, ToolIcon>,
}

impl ToolIcons {
    pub fn new() -> Self {
        Self { cache: HashMap::new() }
    }

    fn cached(&mut self, key: &'static str, build: impl FnOnce() -> ToolIcon) -> &ToolIcon {
        self.cache.entry(key).or_insert_with(build)
    }

    pub fn hammer(&mut self) -> &ToolIcon {
        self.cached("hammer", || make_hammer(256))
    }
    pub fn chain_saw(&mut self, cutting: bool) -> &ToolIcon {
        self.cached(if cutting { "saw_cut" } else { "saw_idle" }, move || make_chain_saw(cutting, 256))
    }
    pub fn machine_gun(&mut self) -> &ToolIcon {
        self.cached("machinegun", || make_machine_gun(256))
    }
    pub fn flame_thrower(&mut self) -> &ToolIcon {
        self.cached("flamethrower", || make_flame_thrower(256))
    }
    pub fn color_thrower(&mut self) -> &ToolIcon {
        self.cached("colorthrower", || make_color_thrower(256))
    }
    pub fn phaser(&mut self) -> &ToolIcon {
        self.cached("phaser", || make_phaser(256))
    }
    pub fn stamp(&mut self, pressed: bool) -> &ToolIcon {
        self.cached(if pressed { "stamp_down" } else { "stamp_up" }, move || make_stamp(pressed, 256))
    }
    pub fn termite_hand(&mut self) -> &ToolIcon {
        self.cached("termitehand", || make_termite_hand(256))
    }
    pub fn washer(&mut self) -> &ToolIcon {
        self.cached("washer", || make_washer(256))
    }

    /// The icon for a given tool in its resting state. `id` matches `ToolID`'s
    /// `keyDigit` (1..=9) — kept as a plain integer here rather than depending on the
    /// (not yet ported) `ToolID` enum, so this crate doesn't gain a circular dependency
    /// on whatever crate ends up owning tool identity.
    pub fn icon_for(&mut self, key_digit: i32) -> &ToolIcon {
        match key_digit {
            1 => self.hammer(),
            2 => self.chain_saw(false),
            3 => self.machine_gun(),
            4 => self.flame_thrower(),
            5 => self.color_thrower(),
            6 => self.phaser(),
            7 => self.stamp(false),
            8 => self.termite_hand(),
            9 => self.washer(),
            _ => self.hammer(),
        }
    }
}

impl Default for ToolIcons {
    fn default() -> Self {
        Self::new()
    }
}

// MARK: - 0: the hammer (hand-drawn — needs a rotated local frame IconBuilder doesn't offer)

fn make_hammer(px: u32) -> ToolIcon {
    let mut pixmap = Pixmap::new(px, px).expect("could not allocate the hammer pixmap");
    let side = px as f32;
    let scale = side / 256.0;

    // We draw in a local frame: the origin is the centre of the head, the handle runs
    // along +x, the striking face is at −y and the claw at +y (all still y-up — see the
    // module doc comment on why the flip happens only once, at the very end of `l`).
    let head_center = (side * 0.336, side * 0.656);
    let angle: f32 = -0.765; // handle pointing down-right

    let l = |x: f32, y: f32| -> (f32, f32) {
        let (sx, sy) = (x * scale, y * scale);
        let (rx, ry) = (sx * angle.cos() - sy * angle.sin(), sx * angle.sin() + sy * angle.cos());
        let (px, py) = (rx + head_center.0, ry + head_center.1);
        (px, side - py) // the one flip, applied once the y-up math is finished
    };

    let fill_path = |pixmap: &mut Pixmap, path: &Path, color: Color| {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        pixmap.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
    };
    let stroke_line = |pixmap: &mut Pixmap, a: (f32, f32), b: (f32, f32), color: Color, width: f32| {
        let mut pb = PathBuilder::new();
        pb.move_to(a.0, a.1);
        pb.line_to(b.0, b.1);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(color);
            paint.anti_alias = true;
            let stroke = Stroke { width, ..Default::default() };
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    };

    // --- the handle ---
    let mut hpb = PathBuilder::new();
    let p = l(20.0, -15.0);
    hpb.move_to(p.0, p.1);
    let p = l(168.0, -11.0);
    hpb.line_to(p.0, p.1);
    let (c, e) = (l(176.0, -11.0), l(176.0, 0.0));
    hpb.quad_to(c.0, c.1, e.0, e.1);
    let (c, e) = (l(176.0, 11.0), l(168.0, 11.0));
    hpb.quad_to(c.0, c.1, e.0, e.1);
    let p = l(20.0, 15.0);
    hpb.line_to(p.0, p.1);
    hpb.close();
    let handle = hpb.finish().expect("handle path");
    fill_path(&mut pixmap, &handle, palette::rgbaf(0.42, 0.27, 0.14, 1.0));

    // A highlight along the top edge of the handle so the wood isn't flat.
    stroke_line(&mut pixmap, l(26.0, -9.0), l(164.0, -6.0), palette::rgbaf(0.62, 0.44, 0.25, 0.9), 3.2 * scale);
    stroke_line(&mut pixmap, l(26.0, 11.0), l(164.0, 8.0), palette::rgbaf(0.20, 0.12, 0.06, 0.8), 2.0 * scale);

    // --- the claw ---
    // In side view the claw is ONE silhouette with a V notch; two parallel prongs are
    // wrong in this projection, because they're separated in depth.
    let mut cpb = PathBuilder::new();
    let p = l(-22.0, 40.0);
    cpb.move_to(p.0, p.1);
    let (c1, c2, e) = (l(-19.0, 70.0), l(-9.0, 90.0), l(10.0, 101.0));
    cpb.cubic_to(c1.0, c1.1, c2.0, c2.1, e.0, e.1);
    let p = l(21.0, 66.0); // the V notch, downwards
    cpb.line_to(p.0, p.1);
    let p = l(35.0, 95.0); // and back up
    cpb.line_to(p.0, p.1);
    let (c1, c2, e) = (l(30.0, 72.0), l(25.0, 56.0), l(22.0, 40.0)); // inner edge
    cpb.cubic_to(c1.0, c1.1, c2.0, c2.1, e.0, e.1);
    cpb.close();
    let claw = cpb.finish().expect("claw path");
    fill_path(&mut pixmap, &claw, palette::graya(0.34, 1.0));

    // --- the head body ---
    let map = |x: f32, y: f32| l(x / scale, y / scale); // push_rounded_rect_mapped wants already-scaled inputs
    let mut head_pb = PathBuilder::new();
    push_rounded_rect_mapped(&mut head_pb, -23.0 * scale, -52.0 * scale, 46.0 * scale, 96.0 * scale, 6.0 * scale, map);
    let head = head_pb.finish().expect("head path");
    fill_path(&mut pixmap, &head, palette::graya(0.40, 1.0));

    // --- the flared striking face ---
    let mut face_pb = PathBuilder::new();
    push_rounded_rect_mapped(&mut face_pb, -28.0 * scale, -66.0 * scale, 56.0 * scale, 20.0 * scale, 4.0 * scale, map);
    let face = face_pb.finish().expect("face path");
    fill_path(&mut pixmap, &face, palette::graya(0.30, 1.0));

    // A metallic highlight down one side of the head and a shadow down the other.
    stroke_line(&mut pixmap, l(-15.0, -46.0), l(-15.0, 34.0), palette::graya(0.74, 0.8), 6.0 * scale);
    stroke_line(&mut pixmap, l(19.0, -46.0), l(19.0, 30.0), palette::graya(0.12, 0.85), 3.0 * scale);

    // A thin dark outline around the whole head — without it the hammer blends into a
    // light desktop.
    let outline_color = palette::graya(0.08, 0.9);
    let ow = 1.6 * scale;
    for path in [&head, &face, &claw] {
        let mut paint = Paint::default();
        paint.set_color(outline_color);
        paint.anti_alias = true;
        let stroke = Stroke { width: ow, line_join: LineJoin::Round, ..Default::default() };
        pixmap.stroke_path(path, &paint, &stroke, Transform::identity(), None);
    }

    // The hotspot = the centre of the striking face.
    let hot = l(0.0, -60.0);
    // The pivot = the butt of the handle. A real hammer swings around the hand, so this
    // is what makes the handle hold still while the head travels an arc.
    let grip = l(170.0, 0.0);

    ToolIcon {
        pixmap,
        hotspot: (hot.0 / side, hot.1 / side),
        pivot: Some((grip.0 / side, grip.1 / side)),
        point_size: (128.0, 128.0),
    }
}

// MARK: - 1: chain-saw

fn make_chain_saw(cutting: bool, px: u32) -> ToolIcon {
    let mut b = IconBuilder::new(px);
    let tip = b.at(34.0, 206.0);
    let root = b.at(150.0, 128.0);

    // The bar with its chain.
    let p = b.capsule(tip, root, b.s(30.0));
    b.fill(&p, palette::steel_dark(), true);
    let p = b.capsule(tip, root, b.s(19.0));
    b.fill(&p, palette::steel(), true);

    // Teeth along both edges of the bar. Note: `tip`/`root` are already screen-space
    // (from `at`), so the perpendicular here is a screen-space perpendicular — that's
    // fine, teeth just need to point off the bar's own axis, not any absolute direction.
    let (dx, dy) = (root.0 - tip.0, root.1 - tip.1);
    let len = dx.hypot(dy).max(1.0);
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy, ux);
    let teeth = 11;
    for i in 0..teeth {
        let t = (i as f32 + 0.5) / teeth as f32;
        let (px0, py0) = (tip.0 + dx * t, tip.1 + dy * t);
        for sign in [1.0f32, -1.0] {
            let base = (px0 + nx * b.s(15.0) * sign, py0 + ny * b.s(15.0) * sign);
            let out = (base.0 + nx * b.s(6.0) * sign, base.1 + ny * b.s(6.0) * sign);
            let fwd = (base.0 + ux * b.s(7.0), base.1 + uy * b.s(7.0));
            if let Some(p) = b.polygon(&[base, out, fwd]) {
                b.fill(&p, palette::steel_dark(), false);
            }
        }
    }

    // The engine housing.
    let p = b.round_rect(140.0, 76.0, 96.0, 84.0, 16.0);
    b.fill(&p, palette::orange(), true);
    let p = b.round_rect(152.0, 90.0, 52.0, 30.0, 8.0);
    b.fill(&p, palette::orange_dark(), false);
    // The top handle.
    let (a, c) = (b.at(150.0, 168.0), b.at(226.0, 152.0));
    let p = b.capsule(a, c, b.s(15.0));
    b.fill(&p, palette::plastic_dark(), true);
    // The rear handle.
    let (a, c) = (b.at(214.0, 74.0), b.at(240.0, 108.0));
    let p = b.capsule(a, c, b.s(15.0));
    b.fill(&p, palette::plastic_dark(), true);
    // The exhaust.
    let p = b.round_rect(214.0, 96.0, 26.0, 18.0, 5.0);
    b.fill(&p, palette::steel_dark(), false);

    if cutting {
        // While the saw cuts, sawdust flies out of the contact point.
        for i in 0..7 {
            let t = i as f32 / 6.0;
            let r = b.s(3.0 + t * 4.0);
            let center = (
                tip.0 - b.s(10.0) - t * b.s(18.0) + nx * b.s(14.0) * (t - 0.5) * 2.0,
                tip.1 - (b.s(6.0) + t * b.s(20.0)), // `-` here, not `+`: this point is
                // built directly in screen space (from `tip`, already flipped), and the
                // original's "+t*b.s(20)" moves toward the bar — same y-up/y-down
                // reasoning as everywhere else in this file.
            );
            let mut pb = PathBuilder::new();
            pb.push_circle(center.0, center.1, r);
            if let Some(path) = pb.finish() {
                b.fill(&path, palette::rgbaf(0.85, 0.72, 0.48, 0.85), false);
            }
        }
    }

    b.outline();
    b.finish((34.0, 206.0), None, 140.0)
}

// MARK: - 2: machine gun

fn make_machine_gun(px: u32) -> ToolIcon {
    let mut b = IconBuilder::new(px);

    // The barrel with its muzzle.
    let (a, c) = (b.at(30.0, 196.0), b.at(126.0, 166.0));
    let p = b.capsule(a, c, b.s(15.0));
    b.fill(&p, palette::gunmetal(), true);
    let p = b.round_rect(24.0, 184.0, 22.0, 22.0, 6.0);
    b.fill(&p, palette::steel_dark(), true);
    // Cooling slots along the barrel.
    for i in 0..4 {
        let t = 0.25 + i as f32 * 0.14;
        let p0 = (30.0 + (126.0 - 30.0) * t, 196.0 + (166.0 - 196.0) * t);
        let (a, c) = (b.at(p0.0 - 4.0, p0.1 - 7.0), b.at(p0.0 + 4.0, p0.1 + 7.0));
        b.line(a, c, palette::plastic_dark(), b.s(3.0));
    }

    // The receiver.
    let p = b.round_rect(118.0, 138.0, 104.0, 44.0, 9.0);
    b.fill(&p, palette::gunmetal(), true);
    let p = b.round_rect(128.0, 158.0, 56.0, 12.0, 4.0);
    b.fill(&p, palette::steel_dark(), false);

    // The magazine, tilted forward.
    let pts = [b.at(140.0, 140.0), b.at(178.0, 140.0), b.at(168.0, 82.0), b.at(126.0, 88.0)];
    if let Some(p) = b.polygon(&pts) {
        b.fill(&p, palette::plastic_dark(), true);
    }
    // Suggested rounds inside the magazine.
    for i in 0..3 {
        let y = 96.0 + i as f32 * 13.0;
        let (a, c) = (b.at(134.0 + i as f32 * 2.0, y), b.at(170.0, y + 2.0));
        b.line(a, c, palette::rgbaf(0.72, 0.56, 0.20, 0.9), b.s(4.0));
    }

    // The grip and the stock.
    let (a, c) = (b.at(196.0, 136.0), b.at(214.0, 96.0));
    let p = b.capsule(a, c, b.s(17.0));
    b.fill(&p, palette::plastic_dark(), true);
    let pts = [b.at(216.0, 176.0), b.at(248.0, 168.0), b.at(248.0, 138.0), b.at(214.0, 142.0)];
    if let Some(p) = b.polygon(&pts) {
        b.fill(&p, palette::wood(), true);
    }
    // The trigger.
    let p = b.round_rect(186.0, 122.0, 10.0, 18.0, 4.0);
    b.fill(&p, palette::steel_dark(), false);
    // The sight.
    let p = b.round_rect(150.0, 180.0, 8.0, 14.0, 2.0);
    b.fill(&p, palette::steel_dark(), true);

    b.outline();
    b.finish((28.0, 197.0), None, 132.0)
}

// MARK: - 3: flame-thrower

fn make_flame_thrower(px: u32) -> ToolIcon {
    let mut b = IconBuilder::new(px);

    // The pipe from the tank to the nozzle.
    let (a, c) = (b.at(48.0, 190.0), b.at(168.0, 128.0));
    let p = b.capsule(a, c, b.s(16.0));
    b.fill(&p, palette::steel_dark(), true);
    // The flared nozzle.
    let pts = [b.at(56.0, 206.0), b.at(26.0, 196.0), b.at(34.0, 168.0), b.at(64.0, 178.0)];
    if let Some(p) = b.polygon(&pts) {
        b.fill(&p, palette::steel(), true);
    }
    let p = b.round_rect(30.0, 176.0, 14.0, 22.0, 4.0);
    b.fill(&p, palette::gunmetal(), false);
    // A pilot nozzle above the main one — a flame-thrower has a permanent pilot flame.
    let p = b.round_rect(52.0, 200.0, 16.0, 11.0, 4.0);
    b.fill(&p, palette::steel_dark(), true);
    let p = b.ellipse(48.0, 206.0, 5.0, 7.0);
    b.fill(&p, palette::rgbaf(1.0, 0.62, 0.15, 0.95), false);

    // The fuel tank.
    let p = b.round_rect(152.0, 74.0, 74.0, 84.0, 26.0);
    b.fill(&p, palette::red(), true);
    let p = b.round_rect(164.0, 96.0, 22.0, 44.0, 10.0);
    b.fill(&p, palette::rgbaf(0.92, 0.36, 0.32, 0.8), false);
    // The valve on top of the tank.
    let p = b.round_rect(178.0, 152.0, 22.0, 16.0, 5.0);
    b.fill(&p, palette::steel_dark(), true);
    // The grip with its trigger.
    let (a, c) = (b.at(140.0, 132.0), b.at(126.0, 96.0));
    let p = b.capsule(a, c, b.s(16.0));
    b.fill(&p, palette::plastic_dark(), true);
    let p = b.round_rect(130.0, 108.0, 10.0, 16.0, 4.0);
    b.fill(&p, palette::steel_dark(), false);

    b.outline();
    b.finish((30.0, 190.0), None, 132.0)
}

// MARK: - 4: color-thrower

fn make_color_thrower(px: u32) -> ToolIcon {
    let mut b = IconBuilder::new(px);

    // The nozzle — aligned with the body so the tool doesn't look disassembled.
    let (a, c) = (b.at(52.0, 150.0), b.at(122.0, 138.0));
    let p = b.capsule(a, c, b.s(14.0));
    b.fill(&p, palette::steel_dark(), true);
    let pts = [b.at(58.0, 162.0), b.at(32.0, 154.0), b.at(38.0, 134.0), b.at(64.0, 142.0)];
    if let Some(p) = b.polygon(&pts) {
        b.fill(&p, palette::steel(), true);
    }

    // The gun body.
    let p = b.round_rect(112.0, 116.0, 84.0, 44.0, 10.0);
    b.fill(&p, palette::blue(), true);
    let p = b.round_rect(124.0, 130.0, 44.0, 14.0, 5.0);
    b.fill(&p, palette::rgbaf(0.10, 0.30, 0.62, 1.0), false);
    let (a, c) = (b.at(174.0, 118.0), b.at(190.0, 78.0));
    let p = b.capsule(a, c, b.s(17.0));
    b.fill(&p, palette::plastic_dark(), true);
    let p = b.round_rect(162.0, 102.0, 10.0, 16.0, 4.0);
    b.fill(&p, palette::steel_dark(), false);

    // The paint canister, sitting directly on the body.
    let p = b.round_rect(122.0, 158.0, 58.0, 52.0, 10.0);
    b.fill(&p, palette::graya(0.88, 0.55), true);
    let p = b.round_rect(127.0, 163.0, 48.0, 28.0, 6.0);
    b.fill(&p, palette::rgbaf(0.95, 0.30, 0.55, 0.95), false);
    let p = b.round_rect(133.0, 165.0, 10.0, 34.0, 5.0);
    b.fill(&p, palette::graya(1.0, 0.40), false);

    b.outline();
    b.finish((38.0, 148.0), None, 128.0)
}

// MARK: - 5: phaser

fn make_phaser(px: u32) -> ToolIcon {
    let mut b = IconBuilder::new(px);

    // The sleek body.
    let pts = [
        b.at(46.0, 176.0),
        b.at(150.0, 150.0),
        b.at(196.0, 140.0),
        b.at(204.0, 116.0),
        b.at(160.0, 118.0),
        b.at(120.0, 140.0),
        b.at(48.0, 158.0),
    ];
    if let Some(p) = b.polygon(&pts) {
        b.fill(&p, palette::gunmetal(), true);
    }
    // The grip.
    let (a, c) = (b.at(184.0, 128.0), b.at(206.0, 84.0));
    let p = b.capsule(a, c, b.s(20.0));
    b.fill(&p, palette::plastic_dark(), true);

    // The emitter ring and the glowing tip.
    let p = b.ellipse(46.0, 167.0, 15.0, 15.0);
    b.fill(&p, palette::steel_dark(), true);
    let p = b.ellipse(46.0, 167.0, 9.0, 9.0);
    b.fill(&p, palette::cyan_glow(), false);
    let p = b.ellipse(46.0, 167.0, 4.5, 4.5);
    b.fill(&p, palette::graya(1.0, 0.95), false);

    // A light strip along the body — the energy cell.
    let (a, c) = (b.at(96.0, 154.0), b.at(178.0, 132.0));
    b.line(a, c, palette::cyan_glow(), b.s(5.0));
    b.line(a, c, palette::graya(1.0, 0.5), b.s(2.0));
    // A highlight along the top edge.
    let (a, c) = (b.at(60.0, 172.0), b.at(150.0, 148.0));
    b.line(a, c, palette::highlight(), b.s(3.0));

    b.outline();
    b.finish((40.0, 167.0), None, 128.0)
}

// MARK: - 6: stamp

fn make_stamp(pressed: bool, px: u32) -> ToolIcon {
    let mut b = IconBuilder::new(px);
    // When pressed the handle moves down towards the face.
    let lift: f32 = if pressed { -14.0 } else { 0.0 };

    // The rubber face at the bottom.
    let p = b.round_rect(58.0, 34.0, 140.0, 26.0, 5.0);
    b.fill(&p, palette::rgbaf(0.22, 0.20, 0.22, 1.0), true);
    // The wooden base.
    let p = b.round_rect(52.0, 58.0, 152.0, 34.0, 6.0);
    b.fill(&p, palette::wood(), true);
    let (a, c) = (b.at(60.0, 84.0), b.at(196.0, 84.0));
    b.line(a, c, palette::wood_light(), b.s(5.0));

    // The stem.
    let p = b.round_rect(112.0, 90.0 + lift, 32.0, 78.0, 8.0);
    b.fill(&p, palette::wood_light(), true);
    // The round knob.
    let p = b.ellipse(128.0, 184.0 + lift, 46.0, 32.0);
    b.fill(&p, palette::wood(), true);
    let p = b.ellipse(114.0, 192.0 + lift, 16.0, 10.0);
    b.fill(&p, palette::highlight(), false);

    b.outline();
    // The hotspot is the centre of the rubber face.
    b.finish((128.0, 40.0), None, 122.0)
}

// MARK: - 7: termites

fn make_termite_hand(px: u32) -> ToolIcon {
    let mut b = IconBuilder::new(px);

    // The wrist and the palm. Instead of folded fingers there are just two that meet in
    // a pinch — at cursor size, fewer shapes read better than more.
    let p = b.round_rect(186.0, 66.0, 62.0, 58.0, 18.0);
    b.fill(&p, palette::skin_shade(), true);
    let p = b.round_rect(126.0, 62.0, 84.0, 88.0, 26.0);
    b.fill(&p, palette::skin(), true);

    // The index finger above and the thumb below, converging to the left.
    let (a, c) = (b.at(146.0, 132.0), b.at(88.0, 152.0));
    let p = b.capsule(a, c, b.s(26.0));
    b.fill(&p, palette::skin(), true);
    let (a, c) = (b.at(148.0, 94.0), b.at(90.0, 126.0));
    let p = b.capsule(a, c, b.s(23.0));
    b.fill(&p, palette::skin_shade(), true);

    // The termite dangling from the pinch — big enough to be recognisable.
    let shell = palette::rgbaf(0.86, 0.77, 0.56, 1.0);
    let chitin = palette::rgbaf(0.56, 0.35, 0.15, 1.0);
    let mandible = palette::rgbaf(0.32, 0.18, 0.07, 1.0);

    let p = b.ellipse(58.0, 140.0, 25.0, 14.0);
    b.fill(&p, shell, true);
    // Segments on the abdomen.
    for dx in [-10.0f32, -2.0, 6.0] {
        let (a, c) = (b.at(58.0 + dx, 128.0), b.at(58.0 + dx, 152.0));
        b.line(a, c, chitin, b.s(2.0));
    }
    let p = b.ellipse(30.0, 140.0, 13.0, 11.0);
    b.fill(&p, chitin, true);
    // The mandibles.
    let (a, c) = (b.at(20.0, 146.0), b.at(8.0, 152.0));
    b.line(a, c, mandible, b.s(4.0));
    let (a, c) = (b.at(20.0, 134.0), b.at(8.0, 128.0));
    b.line(a, c, mandible, b.s(4.0));
    // The legs.
    for dx in [-14.0f32, -2.0, 10.0] {
        let (a, c) = (b.at(58.0 + dx, 128.0), b.at(52.0 + dx, 112.0));
        b.line(a, c, chitin, b.s(3.0));
    }

    b.outline();
    b.finish((30.0, 140.0), None, 132.0)
}

// MARK: - 8: washer

fn make_washer(px: u32) -> ToolIcon {
    let mut b = IconBuilder::new(px);

    // The spray nozzle.
    let (a, c) = (b.at(52.0, 190.0), b.at(118.0, 166.0));
    let p = b.capsule(a, c, b.s(13.0));
    b.fill(&p, palette::steel_dark(), true);
    let p = b.round_rect(36.0, 178.0, 22.0, 20.0, 5.0);
    b.fill(&p, palette::steel(), true);

    // The bottle.
    let p = b.round_rect(120.0, 54.0, 88.0, 108.0, 12.0);
    b.fill(&p, palette::rgbaf(0.42, 0.72, 0.86, 0.92), true);
    // The liquid level.
    let p = b.round_rect(128.0, 62.0, 72.0, 62.0, 8.0);
    b.fill(&p, palette::rgbaf(0.24, 0.56, 0.76, 0.95), false);
    // A highlight on the bottle.
    let p = b.round_rect(132.0, 74.0, 14.0, 74.0, 7.0);
    b.fill(&p, palette::graya(1.0, 0.42), false);
    // The label.
    let p = b.round_rect(126.0, 92.0, 76.0, 26.0, 4.0);
    b.fill(&p, palette::graya(0.96, 0.9), false);
    let (a, c) = (b.at(132.0, 106.0), b.at(192.0, 106.0));
    b.line(a, c, palette::graya(0.35, 0.8), b.s(3.0));
    let (a, c) = (b.at(132.0, 98.0), b.at(176.0, 98.0));
    b.line(a, c, palette::graya(0.5, 0.7), b.s(2.0));

    // The head and the trigger.
    let p = b.round_rect(114.0, 158.0, 84.0, 26.0, 7.0);
    b.fill(&p, palette::plastic_dark(), true);
    let (a, c) = (b.at(150.0, 158.0), b.at(166.0, 128.0));
    let p = b.capsule(a, c, b.s(15.0));
    b.fill(&p, palette::plastic_dark(), true);
    let p = b.round_rect(136.0, 140.0, 10.0, 16.0, 4.0);
    b.fill(&p, palette::steel_dark(), false);

    b.outline();
    b.finish((40.0, 188.0), None, 128.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_blank(pixmap: &Pixmap) -> bool {
        !pixmap.data().chunks_exact(4).any(|px| px[3] > 8)
    }

    #[test]
    fn every_icon_is_not_blank() {
        let mut icons = ToolIcons::new();
        assert!(!is_blank(&icons.hammer().pixmap), "hammer is blank");
        assert!(!is_blank(&icons.chain_saw(false).pixmap), "chain_saw is blank");
        assert!(!is_blank(&icons.chain_saw(true).pixmap), "chain_saw (cutting) is blank");
        assert!(!is_blank(&icons.machine_gun().pixmap), "machine_gun is blank");
        assert!(!is_blank(&icons.flame_thrower().pixmap), "flame_thrower is blank");
        assert!(!is_blank(&icons.color_thrower().pixmap), "color_thrower is blank");
        assert!(!is_blank(&icons.phaser().pixmap), "phaser is blank");
        assert!(!is_blank(&icons.stamp(false).pixmap), "stamp is blank");
        assert!(!is_blank(&icons.stamp(true).pixmap), "stamp (pressed) is blank");
        assert!(!is_blank(&icons.termite_hand().pixmap), "termite_hand is blank");
        assert!(!is_blank(&icons.washer().pixmap), "washer is blank");
    }

    #[test]
    fn every_hotspot_lies_inside_its_image() {
        let mut icons = ToolIcons::new();
        let hotspots = [
            icons.hammer().hotspot,
            icons.chain_saw(false).hotspot,
            icons.machine_gun().hotspot,
            icons.flame_thrower().hotspot,
            icons.color_thrower().hotspot,
            icons.phaser().hotspot,
            icons.stamp(false).hotspot,
            icons.termite_hand().hotspot,
            icons.washer().hotspot,
        ];
        for (i, h) in hotspots.iter().enumerate() {
            assert!((0.0..=1.0).contains(&h.0) && (0.0..=1.0).contains(&h.1), "icon {i} hotspot {h:?} out of bounds");
        }
    }

    #[test]
    fn hammer_pivot_differs_from_its_hotspot() {
        // The one icon that needs a separate pivot — the handle should hold still while
        // the head travels an arc, which only works if pivot != hotspot.
        let mut icons = ToolIcons::new();
        let hammer = icons.hammer();
        let pivot = hammer.pivot.expect("hammer should report a pivot");
        assert!((pivot.0 - hammer.hotspot.0).abs() > 0.05 || (pivot.1 - hammer.hotspot.1).abs() > 0.05);
    }

    #[test]
    fn only_the_hammer_has_a_pivot() {
        let mut icons = ToolIcons::new();
        assert!(icons.hammer().pivot.is_some());
        assert!(icons.washer().pivot.is_none());
        assert!(icons.phaser().pivot.is_none());
    }

    #[test]
    fn icon_for_matches_the_individual_accessors() {
        let mut a = ToolIcons::new();
        let mut b = ToolIcons::new();
        for digit in 1..=9 {
            let via_lookup = a.icon_for(digit).pixmap.data().to_vec();
            let direct = match digit {
                1 => b.hammer().pixmap.data().to_vec(),
                2 => b.chain_saw(false).pixmap.data().to_vec(),
                3 => b.machine_gun().pixmap.data().to_vec(),
                4 => b.flame_thrower().pixmap.data().to_vec(),
                5 => b.color_thrower().pixmap.data().to_vec(),
                6 => b.phaser().pixmap.data().to_vec(),
                7 => b.stamp(false).pixmap.data().to_vec(),
                8 => b.termite_hand().pixmap.data().to_vec(),
                9 => b.washer().pixmap.data().to_vec(),
                _ => unreachable!(),
            };
            assert_eq!(via_lookup, direct, "icon_for({digit}) disagreed with its direct accessor");
        }
    }

    #[test]
    fn cutting_and_idle_chainsaw_icons_differ() {
        let mut icons = ToolIcons::new();
        let idle = icons.chain_saw(false).pixmap.data().to_vec();
        let cutting = icons.chain_saw(true).pixmap.data().to_vec();
        assert_ne!(idle, cutting, "sawdust should make the cutting icon visibly different");
    }

    #[test]
    fn pressed_and_resting_stamp_icons_differ() {
        let mut icons = ToolIcons::new();
        let up = icons.stamp(false).pixmap.data().to_vec();
        let down = icons.stamp(true).pixmap.data().to_vec();
        assert_ne!(up, down, "the lifted handle should make the pressed icon visibly different");
    }
}
