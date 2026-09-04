//! The persistent damage layer.
//!
//! Ported from `DamageLayer.swift`. Design unchanged: the screen is cut into square
//! tiles, each with its own `tiny_skia::Pixmap`. Drawing and **erasing** go straight into
//! the tile's pixels, and only the tiles that actually changed are tracked as dirty — in
//! the real app that dirty set drives which tiles get re-uploaded as GPU textures each
//! frame (Phase 4); here it's just tracked and tested, since there's no GPU yet.
//!
//! Why not "bake to texture": the washer has to erase what has already been drawn, which
//! isn't feasible by stacking sprites into a shared texture. `BlendMode::Clear` in a tile's
//! own pixmap, on the other hand, is trivial — that's what `erase` uses.

use std::collections::HashSet;

use tiny_skia::{
    BlendMode, Color, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Transform,
};

/// A plain axis-aligned rectangle, independent of `tiny_skia::Rect` — that type refuses
/// zero/negative width or height, which the geometry below legitimately produces for
/// off-screen queries (see `tile_indices_intersecting`'s tests).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn min_x(&self) -> f32 {
        self.x
    }
    pub fn max_x(&self) -> f32 {
        self.x + self.w
    }
    pub fn min_y(&self) -> f32 {
        self.y
    }
    pub fn max_y(&self) -> f32 {
        self.y + self.h
    }
}

pub struct DamageLayer {
    /// This screen's `backingScaleFactor` equivalent — output scale, not a tile-count
    /// input. Two displays at 1.0 and 2.0 in the same session are normal, so this can't be
    /// hard-coded.
    scale: f32,
    cols: usize,
    rows: usize,
    tiles: Vec<Pixmap>,
    dirty: HashSet<usize>,
}

impl DamageLayer {
    /// The tile side in points (pre-scale). Matches `DamageLayer.tileSide` in the Swift
    /// original.
    pub const TILE_SIDE: f32 = 256.0;

    pub fn new(width: f32, height: f32, scale: f32) -> Self {
        let scale = scale.max(1.0);
        let cols = ((width / Self::TILE_SIDE).ceil() as usize).max(1);
        let rows = ((height / Self::TILE_SIDE).ceil() as usize).max(1);
        let px = ((Self::TILE_SIDE * scale).round() as u32).max(1);
        let tiles = (0..cols * rows)
            .map(|_| Pixmap::new(px, px).expect("could not allocate a tile pixmap"))
            .collect();
        Self {
            scale,
            cols,
            rows,
            tiles,
            dirty: HashSet::new(),
        }
    }

    // MARK: - Tile geometry

    pub fn grid_size(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }
    pub fn dirty_count(&self) -> usize {
        self.dirty.len()
    }
    /// This screen's pixel-density factor — needed by a renderer to know a tile's actual
    /// pixel dimensions (`TILE_SIDE * scale`), since `TILE_SIDE` alone is in points.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// A dirty tile's raw premultiplied-RGBA8 pixel data, ready to upload to a GPU
    /// texture, together with its pixel dimensions (both tiles are square, so
    /// `width == height`, but a renderer building a `wgpu::Extent3d` wants both spelled
    /// out rather than inferred). Doesn't clear the tile's dirty bit — call `commit()`
    /// once every dirty tile has actually been uploaded, not before, so a failed upload
    /// isn't silently forgotten.
    pub fn tile_pixels(&self, index: usize) -> Option<(&[u8], u32, u32)> {
        let tile = self.tiles.get(index)?;
        Some((tile.data(), tile.width(), tile.height()))
    }

    /// The indices of every tile that changed since the last `commit()` — what a renderer
    /// should feed to `tile_pixels` this frame. Order is unspecified (backed by a
    /// `HashSet`).
    pub fn dirty_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.dirty.iter().copied()
    }

    /// The indices of the tiles a rectangle (in scene coordinates, points) overlaps. Kept
    /// separate from drawing so it's testable without touching any pixels.
    pub fn tile_indices_intersecting(&self, rect: Rect) -> Vec<usize> {
        let side = Self::TILE_SIDE;

        let c0 = (rect.min_x() / side).floor().max(0.0) as usize;
        let r0 = (rect.min_y() / side).floor().max(0.0) as usize;

        // c1/r1 have an *upper* clamp (to the last real column/row) but, unlike c0/r0, can
        // legitimately come out negative for a rect entirely off the negative side — that's
        // "no tiles", handled below, not "clamp up to 0".
        let c1f = ((rect.max_x() - 0.0001) / side).floor();
        let r1f = ((rect.max_y() - 0.0001) / side).floor();
        if c1f < 0.0 || r1f < 0.0 {
            return Vec::new();
        }
        let c1 = (c1f as usize).min(self.cols.saturating_sub(1));
        let r1 = (r1f as usize).min(self.rows.saturating_sub(1));

        if c0 > c1 || r0 > r1 {
            return Vec::new();
        }

        let mut out = Vec::with_capacity((c1 - c0 + 1) * (r1 - r0 + 1));
        for r in r0..=r1 {
            for c in c0..=c1 {
                out.push(r * self.cols + c);
            }
        }
        out
    }

    /// A tile's top-left corner, in scene coordinates (points). A renderer needs this to
    /// know where to place the tile's quad on screen.
    pub fn tile_origin(&self, index: usize) -> (f32, f32) {
        let side = Self::TILE_SIDE;
        (
            (index % self.cols) as f32 * side,
            (index / self.cols) as f32 * side,
        )
    }

    // MARK: - Drawing

    /// Draws a decal centred on `center`, at the given size, axis-aligned and at full
    /// opacity — a `stamp_ex(decal, center, size, rotation: 0, alpha: 1)` convenience;
    /// see that method for the rest.
    pub fn stamp(&mut self, decal: &Pixmap, center: (f32, f32), size: (f32, f32)) {
        self.stamp_ex(decal, center, size, 0.0, 1.0);
    }

    /// The full form of `stamp`, with rotation and alpha — every Phase 3 tool call goes
    /// through this one; `stamp` is a `rotation: 0, alpha: 1` convenience for the Phase 1
    /// tests, mirroring the Swift original's default parameter values (Rust has no default
    /// parameters, so the two-method split stands in for that).
    pub fn stamp_ex(&mut self, decal: &Pixmap, center: (f32, f32), size: (f32, f32), rotation: f32, alpha: f32) {
        let reach = (size.0 * size.0 + size.1 * size.1).sqrt() / 2.0;
        let bounds = Rect {
            x: center.0 - reach,
            y: center.1 - reach,
            w: reach * 2.0,
            h: reach * 2.0,
        };

        let dw = decal.width() as f32;
        let dh = decal.height() as f32;
        let scale_x = (size.0 / dw) * self.scale;
        let scale_y = (size.1 / dh) * self.scale;

        for i in self.tile_indices_intersecting(bounds) {
            let (ox, oy) = self.tile_origin(i);
            let dest_center = ((center.0 - ox) * self.scale, (center.1 - oy) * self.scale);

            // RISK: two things here are my best-confidence reconstruction, not verified
            // against a compiler. (1) `Pixmap::draw_pixmap`'s handling of `transform`
            // versus its integer (x, y) — sidestepped, as in Phase 1, by always passing
            // x=0, y=0 and baking placement entirely into `transform`. (2) that
            // `Transform`'s `pre_*` builders compose the same way Core Graphics'
            // `.rotated(by:)` did in the icon port (Phase 2) — "pre_X" happens *before*
            // whatever was already accumulated. Read bottom-to-top, this: moves the
            // decal's own centre to the origin, scales it, rotates it, then moves it to
            // its destination — a decal centred anywhere but the origin would reveal a
            // wrong pre_/post_ order immediately (it'd orbit around (0,0) as it rotates
            // instead of spinning in place), so this is very testable if wrong.
            let transform = Transform::from_translate(dest_center.0, dest_center.1)
                .pre_rotate(rotation.to_degrees())
                .pre_scale(scale_x, scale_y)
                .pre_translate(-dw / 2.0, -dh / 2.0);

            let mut paint = PixmapPaint::default();
            paint.opacity = alpha.clamp(0.0, 1.0);
            self.tiles[i].draw_pixmap(0, 0, decal.as_ref(), &paint, transform, None);
            self.dirty.insert(i);
        }
    }

    /// Fills a circle — used directly by `erase`, and will back termite bites and scorch
    /// marks once those tools exist.
    pub fn fill_circle(&mut self, center: (f32, f32), radius: f32, color: Color, blend: BlendMode) {
        let bounds = Rect {
            x: center.0 - radius,
            y: center.1 - radius,
            w: radius * 2.0,
            h: radius * 2.0,
        };
        for i in self.tile_indices_intersecting(bounds) {
            let (ox, oy) = self.tile_origin(i);
            let mut pb = PathBuilder::new();
            pb.push_circle(
                (center.0 - ox) * self.scale,
                (center.1 - oy) * self.scale,
                radius * self.scale,
            );
            if let Some(path) = pb.finish() {
                let mut paint = Paint::default();
                paint.set_color(color);
                paint.blend_mode = blend;
                paint.anti_alias = true;
                self.tiles[i].fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
            }
            self.dirty.insert(i);
        }
    }

    /// Erases a circle — this is the washer. `BlendMode::Clear` only works because every
    /// tile pixmap has an alpha channel; the fill colour is irrelevant under Clear.
    pub fn erase(&mut self, center: (f32, f32), radius: f32) {
        self.fill_circle(center, radius, Color::from_rgba8(0, 0, 0, 255), BlendMode::Clear);
    }

    pub fn erase_all(&mut self) {
        for (i, tile) in self.tiles.iter_mut().enumerate() {
            tile.fill(Color::from_rgba8(0, 0, 0, 0));
            self.dirty.insert(i);
        }
    }

    /// Marks every tile dirty without touching any pixels. A renderer needs this for a
    /// brand new (or just-resized) `DamageLayer`: every tile's pixels are already correct
    /// (freshly allocated `Pixmap`s are fully transparent), but its GPU texture's memory
    /// is *not* — uninitialized VRAM, not necessarily zeroed — until at least one upload
    /// has actually happened. Without an initial dirty set to upload from, a freshly
    /// created layer would render as garbage instead of a clean blank overlay.
    pub fn mark_all_dirty(&mut self) {
        for i in 0..self.tiles.len() {
            self.dirty.insert(i);
        }
    }

    // MARK: - Texture upload (stubbed until Phase 4)

    /// In the real app this uploads every dirty tile as a GPU texture and returns how many
    /// it touched. Phase 1 has no GPU, so this just clears the dirty set and reports its
    /// size — enough to test the *tracking*, which is the part regression-worthy this
    /// early.
    pub fn commit(&mut self) -> usize {
        let n = self.dirty.len();
        self.dirty.clear();
        n
    }

    // MARK: - Diagnostics

    /// Composites every tile into one image. Used for coverage checks and for PNG dumps —
    /// never called in the (future) render loop.
    pub fn snapshot(&self) -> Pixmap {
        let side_px = ((Self::TILE_SIDE * self.scale).round() as u32).max(1);
        let w = side_px * self.cols as u32;
        let h = side_px * self.rows as u32;
        let mut out = Pixmap::new(w.max(1), h.max(1)).expect("could not allocate the snapshot pixmap");
        let paint = PixmapPaint::default();
        for (i, tile) in self.tiles.iter().enumerate() {
            let x = (i % self.cols) as i32 * side_px as i32;
            let y = (i / self.cols) as i32 * side_px as i32;
            out.draw_pixmap(x, y, tile.as_ref(), &paint, Transform::identity(), None);
        }
        out
    }

    /// The fraction of pixels with alpha above a "clearly drawn on" threshold. Used to
    /// check that drawing adds coverage and erasing removes it — not meant to be exact,
    /// just monotonic in the right direction.
    pub fn coverage(&self) -> f32 {
        let snap = self.snapshot();
        let data = snap.data();
        if data.is_empty() {
            return 0.0;
        }
        let mut covered = 0usize;
        let mut total = 0usize;
        for px in data.chunks_exact(4) {
            total += 1;
            if px[3] > 8 {
                covered += 1;
            }
        }
        covered as f32 / total.max(1) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decals::DecalFactory;
    use std::collections::HashSet as Set;

    // --- geometry --------------------------------------------------------------------

    #[test]
    fn grid_5x3() {
        let side = DamageLayer::TILE_SIDE;
        let layer = DamageLayer::new(side * 5.0, side * 3.0, 1.0);
        assert_eq!(layer.grid_size(), (5, 3));
        assert_eq!(layer.tile_count(), 15);
    }

    #[test]
    fn partial_tile_is_still_allocated() {
        // 300×300 pt with a 256 pt side needs a 2×2 grid, not 1×1.
        let layer = DamageLayer::new(300.0, 300.0, 1.0);
        assert_eq!(layer.grid_size(), (2, 2));
    }

    #[test]
    fn rectangle_inside_one_tile() {
        let side = DamageLayer::TILE_SIDE;
        let grid = DamageLayer::new(side * 4.0, side * 4.0, 1.0);
        let idx = grid.tile_indices_intersecting(Rect { x: 20.0, y: 20.0, w: 40.0, h: 40.0 });
        assert_eq!(idx, vec![0]);
    }

    #[test]
    fn vertical_edge_gives_two_tiles() {
        let side = DamageLayer::TILE_SIDE;
        let grid = DamageLayer::new(side * 4.0, side * 4.0, 1.0);
        let idx: Set<usize> = grid
            .tile_indices_intersecting(Rect { x: side - 10.0, y: 30.0, w: 20.0, h: 20.0 })
            .into_iter()
            .collect();
        assert_eq!(idx, Set::from([0, 1]));
    }

    #[test]
    fn corner_gives_four_tiles() {
        let side = DamageLayer::TILE_SIDE;
        let grid = DamageLayer::new(side * 4.0, side * 4.0, 1.0);
        let idx: Set<usize> = grid
            .tile_indices_intersecting(Rect { x: side - 10.0, y: side - 10.0, w: 20.0, h: 20.0 })
            .into_iter()
            .collect();
        assert_eq!(idx, Set::from([0, 1, 4, 5]));
    }

    #[test]
    fn edge_does_not_spill_into_next_tile() {
        let side = DamageLayer::TILE_SIDE;
        let grid = DamageLayer::new(side * 4.0, side * 4.0, 1.0);
        let idx = grid.tile_indices_intersecting(Rect { x: 0.0, y: 0.0, w: side, h: side });
        assert_eq!(idx, vec![0]);
    }

    #[test]
    fn partly_off_screen_is_clamped() {
        let side = DamageLayer::TILE_SIDE;
        let small = DamageLayer::new(side * 2.0, side * 2.0, 1.0);
        let idx = small.tile_indices_intersecting(Rect { x: -100.0, y: -100.0, w: 150.0, h: 150.0 });
        assert_eq!(idx, vec![0]);
    }

    #[test]
    fn entirely_off_screen_returns_nothing() {
        let side = DamageLayer::TILE_SIDE;
        let small = DamageLayer::new(side * 2.0, side * 2.0, 1.0);
        let idx = small.tile_indices_intersecting(Rect { x: 5_000.0, y: 5_000.0, w: 10.0, h: 10.0 });
        assert!(idx.is_empty());
    }

    // --- drawing and erasing -----------------------------------------------------------

    #[test]
    fn dirty_tracking_and_commit() {
        let side = DamageLayer::TILE_SIDE;
        let mut layer = DamageLayer::new(side * 3.0, side * 2.0, 1.0);
        let mut decals = DecalFactory::new();

        assert_eq!(layer.dirty_count(), 0);
        layer.stamp(decals.crack(0), (300.0, 200.0), (160.0, 160.0));
        assert!(layer.dirty_count() > 0);
        assert!(layer.commit() > 0);
        assert_eq!(layer.dirty_count(), 0);
    }

    #[test]
    fn erasing_reduces_coverage() {
        // Erasing is the only test that `BlendMode::Clear` really works in our pixmaps.
        let side = DamageLayer::TILE_SIDE;
        let mut layer = DamageLayer::new(side * 2.0, side * 2.0, 1.0);
        let mut decals = DecalFactory::new();
        let center = (side, side);

        layer.stamp(decals.crack(3), center, (300.0, 300.0));
        let painted = layer.coverage();
        assert!(painted > 0.001, "a crack should cover something, got {painted}");

        layer.erase(center, 150.0);
        assert!(layer.coverage() < painted, "erasing should reduce coverage");
    }

    #[test]
    fn erase_all_clears_everything() {
        let side = DamageLayer::TILE_SIDE;
        let mut layer = DamageLayer::new(side * 2.0, side * 2.0, 1.0);
        let mut decals = DecalFactory::new();

        layer.stamp(decals.crack(3), (side, side), (300.0, 300.0));
        layer.erase_all();
        assert!(layer.coverage() < 0.0001);
    }

    #[test]
    fn scale_changes_pixel_size_not_tile_count() {
        // Displays in the same session can have different pixel densities.
        let one = DamageLayer::new(512.0, 512.0, 1.0);
        let two = DamageLayer::new(512.0, 512.0, 2.0);
        assert_eq!(one.tile_count(), two.tile_count());
        assert_eq!(one.snapshot().width(), 512);
        assert_eq!(two.snapshot().width(), 1_024);
    }

    #[test]
    fn a_rotated_stamp_lands_near_its_centre_not_near_the_origin() {
        // `stamp_ex`'s rotation composition (see its RISK comment) is the one thing in
        // this file that couldn't be checked against a compiler. This doesn't verify the
        // rotation angle exactly, but it does catch the failure mode that comment warns
        // about: if the pre_rotate/pre_scale/pre_translate order were wrong, a rotated
        // decal would tend to land offset toward (0, 0) instead of centred where it was
        // asked to go. Stamping far from the origin and checking that the tile *at* the
        // origin stays untouched rules that out.
        let side = DamageLayer::TILE_SIDE;
        let mut layer = DamageLayer::new(side * 4.0, side * 4.0, 1.0); // 1024×1024, 4×4 tiles
        let mut decals = DecalFactory::new();

        let far_point = (side * 3.5, side * 3.5); // deep in the bottom-right tile (index 15)
        layer.stamp_ex(decals.crack(0), far_point, (200.0, 200.0), std::f32::consts::FRAC_PI_2, 1.0);

        // `dirty` is a private field, reachable here because `tests` is a child module of
        // `damage.rs` — used directly rather than through `dirty_count()` because a count
        // alone can't tell *which* tiles were touched, which is the whole point of this
        // check.
        assert!(layer.dirty.contains(&15), "the stamp should have marked the tile it was aimed at, got {:?}", layer.dirty);
        assert!(!layer.dirty.contains(&0), "a stamp aimed at the far corner reached the tile at the origin — the rotation transform likely has the wrong composition order: {:?}", layer.dirty);
    }

    #[test]
    fn tile_pixels_matches_the_tile_grid() {
        let side = DamageLayer::TILE_SIDE;
        let layer = DamageLayer::new(side * 2.0, side * 2.0, 2.0); // 2×2 tiles at 2x scale
        let (pixels, w, h) = layer.tile_pixels(0).expect("tile 0 should exist");
        assert_eq!((w, h), (512, 512), "256pt tile at 2x scale should be 512px");
        assert_eq!(pixels.len(), 512 * 512 * 4, "RGBA8 — 4 bytes per pixel");
        assert!(layer.tile_pixels(4).is_none(), "only 4 tiles (0..=3) exist in a 2×2 grid");
    }

    #[test]
    fn dirty_indices_matches_dirty_count_and_clears_on_commit() {
        let side = DamageLayer::TILE_SIDE;
        let mut layer = DamageLayer::new(side * 3.0, side * 3.0, 1.0);
        let mut decals = DecalFactory::new();

        assert_eq!(layer.dirty_indices().count(), 0);
        layer.stamp(decals.crack(0), (side, side), (100.0, 100.0));
        assert_eq!(layer.dirty_indices().count(), layer.dirty_count());
        assert!(layer.dirty_indices().count() > 0);

        layer.commit();
        assert_eq!(layer.dirty_indices().count(), 0);
    }

    #[test]
    fn tile_origin_matches_the_grid_geometry() {
        let side = DamageLayer::TILE_SIDE;
        let layer = DamageLayer::new(side * 3.0, side * 2.0, 1.0);
        assert_eq!(layer.tile_origin(0), (0.0, 0.0));
        assert_eq!(layer.tile_origin(1), (side, 0.0));
        assert_eq!(layer.tile_origin(3), (0.0, side)); // first tile of the second row
    }

    #[test]
    fn scale_accessor_matches_construction() {
        let layer = DamageLayer::new(512.0, 512.0, 2.0);
        assert_eq!(layer.scale(), 2.0);
    }

    #[test]
    fn mark_all_dirty_covers_every_tile_without_changing_pixels() {
        let side = DamageLayer::TILE_SIDE;
        let mut layer = DamageLayer::new(side * 3.0, side * 2.0, 1.0);
        assert_eq!(layer.dirty_count(), 0);
        let before = layer.coverage();

        layer.mark_all_dirty();
        assert_eq!(layer.dirty_count(), layer.tile_count());
        assert_eq!(layer.coverage(), before, "marking dirty shouldn't touch any pixels");

        layer.commit();
        assert_eq!(layer.dirty_count(), 0);
    }
}
