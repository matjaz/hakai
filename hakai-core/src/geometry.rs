//! Shape-building helpers shared by `decals.rs` and `icons.rs` — nothing here depends on
//! any particular graphics library API beyond `tiny_skia::PathBuilder`'s basic
//! move/line/quad/cubic/close calls, so unlike most of this port these needed no
//! compiler-verification round-trip: the constructions are standard, well-known geometry
//! (the Bézier "kappa" circle/ellipse approximation, a manually-swept stadium shape).

use tiny_skia::PathBuilder;

/// A general ellipse (unlike `PathBuilder::push_circle`, `rx` and `ry` can differ) via the
/// standard four-cubic "kappa" approximation.
pub fn push_ellipse(pb: &mut PathBuilder, cx: f32, cy: f32, rx: f32, ry: f32) {
    const K: f32 = 0.5522847498;
    let (ox, oy) = (rx * K, ry * K);
    pb.move_to(cx + rx, cy);
    pb.cubic_to(cx + rx, cy + oy, cx + ox, cy + ry, cx, cy + ry);
    pb.cubic_to(cx - ox, cy + ry, cx - rx, cy + oy, cx - rx, cy);
    pb.cubic_to(cx - rx, cy - oy, cx - ox, cy - ry, cx, cy - ry);
    pb.cubic_to(cx + ox, cy - ry, cx + rx, cy - oy, cx + rx, cy);
    pb.close();
}

/// A rounded rectangle, corners approximated with quadratic Béziers (visually equivalent
/// to Core Graphics' `CGPath(roundedRect:)` even though the underlying curve math isn't
/// identical).
pub fn push_rounded_rect(pb: &mut PathBuilder, x: f32, y: f32, w: f32, h: f32, r: f32) {
    push_rounded_rect_mapped(pb, x, y, w, h, r, |x, y| (x, y));
}

/// Same shape as `push_rounded_rect`, but every point — corners, straight edges, and the
/// arc control points — is passed through `map` first. Used by the hammer icon, whose
/// head and striking face are rounded rects built in a local frame and then rotated: since
/// rotation is affine, transforming a quadratic Bézier's control point is equivalent to
/// transforming the curve it describes, so this is exact, not an approximation of an
/// approximation.
pub fn push_rounded_rect_mapped(
    pb: &mut PathBuilder,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    map: impl Fn(f32, f32) -> (f32, f32),
) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let m = |px: f32, py: f32| map(px, py);

    let p = m(x + r, y);
    pb.move_to(p.0, p.1);
    let p = m(x + w - r, y);
    pb.line_to(p.0, p.1);
    let (c, e) = (m(x + w, y), m(x + w, y + r));
    pb.quad_to(c.0, c.1, e.0, e.1);
    let p = m(x + w, y + h - r);
    pb.line_to(p.0, p.1);
    let (c, e) = (m(x + w, y + h), m(x + w - r, y + h));
    pb.quad_to(c.0, c.1, e.0, e.1);
    let p = m(x + r, y + h);
    pb.line_to(p.0, p.1);
    let (c, e) = (m(x, y + h), m(x, y + h - r));
    pb.quad_to(c.0, c.1, e.0, e.1);
    let p = m(x, y + r);
    pb.line_to(p.0, p.1);
    let (c, e) = (m(x, y), m(x + r, y));
    pb.quad_to(c.0, c.1, e.0, e.1);
    pb.close();
}

/// A rounded bar between two points — a stadium/capsule shape, the basic building block
/// for icon barrels, handles and mounts (Core Graphics gets this from
/// `CGPath.copy(strokingWithWidth:lineCap:.round:...)`; tiny-skia has no public
/// stroke-to-fill conversion, so it's built directly as a swept polygon instead — 16
/// segments per semicircular cap, plenty at icon resolution).
pub fn push_capsule(pb: &mut PathBuilder, a: (f32, f32), b: (f32, f32), width: f32) {
    let r = width / 2.0;
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt().max(0.0001);
    let base_angle = (dy / len).atan2(dx / len);

    const SEGS: usize = 16;
    const HALF_TURN: f32 = std::f32::consts::PI;
    const QUARTER_TURN: f32 = std::f32::consts::FRAC_PI_2;

    let mut first = true;
    // The cap at `b`, sweeping the semicircle that bulges away from `a`.
    for i in 0..=SEGS {
        let t = i as f32 / SEGS as f32;
        let ang = base_angle - QUARTER_TURN + t * HALF_TURN;
        let p = (b.0 + ang.cos() * r, b.1 + ang.sin() * r);
        if first {
            pb.move_to(p.0, p.1);
            first = false;
        } else {
            pb.line_to(p.0, p.1);
        }
    }
    // The straight side, then the cap at `a`, sweeping the semicircle that bulges away
    // from `b`. `close()` draws the second straight side back to the first point above.
    for i in 0..=SEGS {
        let t = i as f32 / SEGS as f32;
        let ang = base_angle + QUARTER_TURN + t * HALF_TURN;
        let p = (a.0 + ang.cos() * r, a.1 + ang.sin() * r);
        pb.line_to(p.0, p.1);
    }
    pb.close();
}
