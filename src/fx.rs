//! Drawing primitives shared by the effects (all on premultiplied tiny-skia pixmaps).

use tiny_skia::{
    FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, Path, PathBuilder, Pixmap, Point, RadialGradient, Rect, SpreadMode,
    Stroke, Transform,
};

pub fn solid(c: tiny_skia::Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

/// Fill an axis-aligned rect given in global coordinates. The rect is translated by hand and
/// drawn with an identity transform and no anti-aliasing: tiny-skia then takes its fast
/// span-blitting path instead of converting the rect to an anti-aliased path (several times
/// slower, and these fills cover large areas every frame).
fn fill_rect_fast(pm: &mut Pixmap, tf: Transform, rect: Rect, mut paint: Paint) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    paint.anti_alias = false;
    if let Some(local) = rect.transform(Transform::from_translate(tf.tx, tf.ty)) {
        pm.fill_rect(local, &paint, Transform::identity(), None);
    }
}

pub fn circle(cx: f32, cy: f32, r: f32) -> Option<Path> {
    PathBuilder::from_circle(cx, cy, r.max(0.01))
}

pub fn fill_circle(pm: &mut Pixmap, tf: Transform, cx: f32, cy: f32, r: f32, c: tiny_skia::Color) {
    if let Some(p) = circle(cx, cy, r) {
        pm.fill_path(&p, &solid(c), FillRule::Winding, tf, None);
    }
}

pub fn stroke_circle(pm: &mut Pixmap, tf: Transform, cx: f32, cy: f32, r: f32, width: f32, c: tiny_skia::Color) {
    if let Some(p) = circle(cx, cy, r) {
        let s = Stroke { width: width.max(0.1), ..Stroke::default() };
        pm.stroke_path(&p, &solid(c), &s, tf, None);
    }
}

pub fn stroke_line(
    pm: &mut Pixmap,
    tf: Transform,
    a: (f32, f32),
    b: (f32, f32),
    width: f32,
    c: tiny_skia::Color,
    blend: tiny_skia::BlendMode,
) {
    let mut pb = PathBuilder::new();
    pb.move_to(a.0, a.1);
    pb.line_to(b.0, b.1);
    if let Some(p) = pb.finish() {
        let mut paint = solid(c);
        paint.blend_mode = blend;
        let s = Stroke { width: width.max(0.1), line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
        pm.stroke_path(&p, &paint, &s, tf, None);
    }
}

/// Regular star with `points` tips.
pub fn star(cx: f32, cy: f32, r_out: f32, r_in: f32, points: u32, rot: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    for i in 0..points * 2 {
        let a = rot + std::f32::consts::PI * i as f32 / points as f32 - std::f32::consts::FRAC_PI_2;
        let r = if i % 2 == 0 { r_out } else { r_in };
        let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.close();
    pb.finish()
}

/// Concave four-pointed sparkle.
pub fn sparkle(cx: f32, cy: f32, r: f32, rot: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let pt = |ang: f32, rad: f32| (cx + rad * (ang + rot).cos(), cy + rad * (ang + rot).sin());
    let tips = [0.0f32, 90.0, 180.0, 270.0].map(f32::to_radians);
    let (x, y) = pt(tips[0], r);
    pb.move_to(x, y);
    for i in 0..4 {
        let mid = tips[i] + std::f32::consts::FRAC_PI_4;
        let next = tips[(i + 1) % 4];
        let (cx2, cy2) = pt(mid, r * 0.12);
        let (nx, ny) = pt(next, r);
        pb.quad_to(cx2, cy2, nx, ny);
    }
    pb.close();
    pb.finish()
}

pub fn heart(cx: f32, cy: f32, s: f32) -> Option<Path> {
    // Unit heart scaled so its width is roughly 2*s.
    let mut pb = PathBuilder::new();
    pb.move_to(cx, cy + s * 0.95);
    pb.cubic_to(cx - s * 1.6, cy - s * 0.1, cx - s * 0.7, cy - s * 1.15, cx, cy - s * 0.35);
    pb.cubic_to(cx + s * 0.7, cy - s * 1.15, cx + s * 1.6, cy - s * 0.1, cx, cy + s * 0.95);
    pb.close();
    pb.finish()
}

pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let k = r * 0.5523;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

pub fn fill_rrect(pm: &mut Pixmap, tf: Transform, x: f32, y: f32, w: f32, h: f32, r: f32, paint: &Paint) {
    if let Some(p) = rounded_rect(x, y, w, h, r) {
        pm.fill_path(&p, paint, FillRule::Winding, tf, None);
    }
}

pub fn stroke_rrect(pm: &mut Pixmap, tf: Transform, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, c: tiny_skia::Color) {
    if let Some(p) = rounded_rect(x, y, w, h, r) {
        let s = Stroke { width, ..Stroke::default() };
        pm.stroke_path(&p, &solid(c), &s, tf, None);
    }
}

/// HSV (h in degrees, s/v in 0..1) → linear-ish sRGB triple in 0..1.
pub fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

pub fn rgba(rgb: [f32; 3], a: f32) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(rgb[0].clamp(0.0, 1.0), rgb[1].clamp(0.0, 1.0), rgb[2].clamp(0.0, 1.0), a.clamp(0.0, 1.0))
        .unwrap_or(tiny_skia::Color::TRANSPARENT)
}

fn with_alpha(c: tiny_skia::Color, a: f32) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(c.red(), c.green(), c.blue(), a.clamp(0.0, 1.0)).unwrap_or(tiny_skia::Color::TRANSPARENT)
}

/// Fill `area` with `dim`, leaving a soft-edged round hole at (cx, cy).
pub fn dim_with_circle(pm: &mut Pixmap, tf: Transform, area: Rect, cx: f32, cy: f32, radius: f32, softness: f32, dim: tiny_skia::Color) {
    let feather = (radius * softness * 0.9).max(0.5);
    let r_out = radius + feather * 0.5;
    let r_in = (radius - feather * 0.5).max(0.0);
    let stops = vec![
        GradientStop::new(0.0, with_alpha(dim, 0.0)),
        GradientStop::new((r_in / r_out).clamp(0.0, 0.999), with_alpha(dim, 0.0)),
        GradientStop::new(1.0, dim),
    ];
    let c = Point::from_xy(cx, cy);
    // The shader lives in global coordinates; `tf` maps them into this pixmap.
    if let Some(shader) = RadialGradient::new(c, 0.0, c, r_out, stops, SpreadMode::Pad, tf) {
        fill_rect_fast(pm, tf, area, Paint { shader, ..Paint::default() });
    }
}

/// Fill `area` with `dim`, leaving a soft-edged lit band centred on `centre`.
/// `horizontal` is a band that spans the width (varies with y). The band only exists inside
/// `span` (the pointer's monitor); everything else stays dimmed.
pub fn dim_with_bar(
    pm: &mut Pixmap,
    tf: Transform,
    area: Rect,
    span: Rect,
    horizontal: bool,
    centre: f32,
    half: f32,
    softness: f32,
    dim: tiny_skia::Color,
) {
    fill_rect_fast(pm, tf, area, solid(dim));
    let (l, t) = (area.left().max(span.left()), area.top().max(span.top()));
    let (r, b) = (area.right().min(span.right()), area.bottom().min(span.bottom()));
    // Rect::from_ltrb accepts zero-width rects and tiny-skia then fills a 1px sliver.
    if r <= l || b <= t {
        return;
    }
    let Some(clip) = Rect::from_ltrb(l, t, r, b) else { return };
    let feather = (half * softness * 0.9).max(0.5);
    let h_out = half + feather * 0.5;
    let edge = (feather / (2.0 * h_out)).clamp(0.0, 0.49);
    let stops = vec![
        GradientStop::new(0.0, dim),
        GradientStop::new(edge, with_alpha(dim, 0.0)),
        GradientStop::new(1.0 - edge, with_alpha(dim, 0.0)),
        GradientStop::new(1.0, dim),
    ];
    let (a, c) = if horizontal {
        (Point::from_xy(0.0, centre - h_out), Point::from_xy(0.0, centre + h_out))
    } else {
        (Point::from_xy(centre - h_out, 0.0), Point::from_xy(centre + h_out, 0.0))
    };
    if let Some(shader) = LinearGradient::new(a, c, stops, SpreadMode::Pad, tf) {
        // Source replaces the flat dim underneath, so the lit centre really becomes transparent.
        fill_rect_fast(pm, tf, clip, Paint { shader, blend_mode: tiny_skia::BlendMode::Source, ..Paint::default() });
    }
}

/// Fill `area` with `dim`, leaving a rounded-rect hole (even-odd).
pub fn dim_with_rect(pm: &mut Pixmap, tf: Transform, area: Rect, hole: Rect, corner: f32, dim: tiny_skia::Color) {
    let mut pb = PathBuilder::new();
    pb.push_rect(area);
    if let Some(h) = rounded_rect(hole.x(), hole.y(), hole.width(), hole.height(), corner) {
        pb.push_path(&h);
    }
    if let Some(p) = pb.finish() {
        pm.fill_path(&p, &solid(dim), FillRule::EvenOdd, tf, None);
    }
}
