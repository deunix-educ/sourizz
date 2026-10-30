//! A per-monitor pixel layer in the layout windowing systems want
//! (premultiplied BGRA, i.e. `ARGB32` on little-endian machines).
//!
//! The engine paints in RGBA; [`Layer::repaint`] renders only the damaged rectangles and
//! converts them, so a mostly static overlay costs almost nothing per frame.
//!
//! X11 keeps window contents server-side, so it uses a *streaming* layer that retains
//! nothing. Wayland has to hand over a whole buffer every commit, so it uses a
//! *retained* layer that accumulates the full image.

use crate::engine::Engine;
use crate::geom::IRect;
use tiny_skia::Pixmap;

/// Scratch pixmaps above this many pixels are dropped after use instead of cached.
const KEEP_SCRATCH_PIXELS: u32 = 256 * 256;

/// A freshly painted rectangle: `rect` is layer-local, `data` is tightly packed BGRA.
pub struct Patch {
    pub rect: IRect,
    pub data: Vec<u8>,
}

pub struct Layer {
    /// Position of this layer in global (virtual desktop) coordinates.
    pub rect: IRect,
    pub w: usize,
    pub h: usize,
    /// Full image; empty for streaming layers.
    pub bgra: Vec<u8>,
    retain: bool,
    scratch: Option<Pixmap>,
}

impl Layer {
    pub fn new(rect: IRect, retain: bool) -> Self {
        let (w, h) = (rect.width().max(1) as usize, rect.height().max(1) as usize);
        Self { rect, w, h, bgra: if retain { vec![0; w * h * 4] } else { Vec::new() }, retain, scratch: None }
    }

    /// Repaint the intersection of each `damage` rectangle with this layer.
    pub fn repaint(&mut self, engine: &mut Engine, damage: &[IRect], now: f64) -> Vec<Patch> {
        let mut out = Vec::new();
        for &d in damage {
            let g = d.intersect(self.rect);
            if g.is_empty() {
                continue;
            }
            let (gw, gh) = (g.width() as u32, g.height() as u32);
            let mut pm = match self.scratch.take() {
                Some(p) if p.width() == gw && p.height() == gh => p,
                _ => match Pixmap::new(gw, gh) {
                    Some(p) => p,
                    None => continue,
                },
            };
            engine.paint(&mut pm, g, now);
            let local = g.translate(-self.rect.x0, -self.rect.y0);
            let mut data = vec![0u8; gw as usize * gh as usize * 4];
            for (dp, sp) in data.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
                dp[0] = sp[2];
                dp[1] = sp[1];
                dp[2] = sp[0];
                dp[3] = sp[3];
            }
            if self.retain {
                let row = gw as usize * 4;
                for y in 0..gh as usize {
                    let o = ((local.y0 as usize + y) * self.w + local.x0 as usize) * 4;
                    self.bgra[o..o + row].copy_from_slice(&data[y * row..(y + 1) * row]);
                }
            }
            if gw * gh <= KEEP_SCRATCH_PIXELS {
                self.scratch = Some(pm);
            }
            out.push(Patch { rect: local, data });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::input::InputEvent;
    use crate::text::Fonts;

    fn setup() -> (Engine, Vec<IRect>) {
        let mut c = Config::default();
        c.ring.pulse_speed = 0.0;
        let mut e = Engine::new(c, Fonts::none());
        e.set_monitors(vec![IRect::new(0, 0, 200, 100), IRect::new(200, 0, 400, 100)]);
        e.handle(0.0, InputEvent::Pointer { x: 100.0, y: 50.0 });
        let d = e.begin_frame(0.0).damage;
        (e, d)
    }

    #[test]
    fn layer_converts_to_bgra_and_only_touches_its_monitor() {
        let (mut e, d) = setup();
        let mut left = Layer::new(IRect::new(0, 0, 200, 100), true);
        let mut right = Layer::new(IRect::new(200, 0, 400, 100), true);
        assert!(!left.repaint(&mut e, &d, 0.0).is_empty());
        assert!(!right.repaint(&mut e, &d, 0.0).is_empty(), "full redraw reaches every monitor");
        assert!(left.bgra.chunks(4).any(|p| p[3] > 0), "ring is on the left monitor");
        assert!(right.bgra.chunks(4).all(|p| p[3] == 0), "right monitor stays empty");
        // Default ring colour is amber #FFC107: in BGRA the blue byte is small, red is large.
        let px = left.bgra.chunks(4).max_by_key(|p| p[3]).unwrap();
        assert!(px[3] > 200 && px[2] > 200 && px[0] < 60, "BGRA order, got {px:?}");
    }

    #[test]
    fn streaming_layer_retains_nothing_but_returns_the_same_pixels() {
        let (mut e, d) = setup();
        let mut retained = Layer::new(IRect::new(0, 0, 200, 100), true);
        let mut streaming = Layer::new(IRect::new(0, 0, 200, 100), false);
        let a = retained.repaint(&mut e, &d, 0.0);
        let b = streaming.repaint(&mut e, &d, 0.0);
        assert!(streaming.bgra.is_empty());
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.rect, y.rect);
            assert_eq!(x.data, y.data);
        }
    }

    #[test]
    fn repaint_outside_layer_yields_nothing() {
        let mut e = Engine::new(Config::default(), Fonts::none());
        let mut l = Layer::new(IRect::new(0, 0, 100, 100), false);
        assert!(l.repaint(&mut e, &[IRect::new(500, 500, 600, 600)], 0.0).is_empty());
    }

    /// The property that matters: painting only the reported damage must give exactly the
    /// same picture as repainting everything. A mismatch means ghost pixels or stale edges.
    fn assert_incremental_matches_full(mut cfg: Config, label: &str) {
        use crate::input::Button;
        cfg.ring.pulse_speed = 0.0;
        let mons = vec![IRect::new(0, 0, 320, 240), IRect::new(320, 0, 640, 240)];
        let mut e = Engine::new(cfg, Fonts::none());
        e.set_monitors(mons.clone());
        let mut layers: Vec<Layer> = mons.iter().map(|m| Layer::new(*m, true)).collect();
        let mut rng = 0x1234_5678u32;
        let mut next = |m: u32| {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng % m
        };
        let (mut x, mut y) = (100.0f32, 100.0f32);
        for step in 0..70 {
            let t = step as f64 * 0.016;
            match next(12) {
                0 => {
                    // Jump, sometimes to the other monitor.
                    x = next(640) as f32;
                    y = next(240) as f32;
                }
                1 if step % 3 == 0 => e.handle(t, InputEvent::Button { button: Button::Left, pressed: true }),
                _ => {
                    x = (x + next(41) as f32 - 20.0).clamp(0.0, 639.0);
                    y = (y + next(41) as f32 - 20.0).clamp(0.0, 239.0);
                }
            }
            e.handle(t, InputEvent::Pointer { x, y });
            let plan = e.begin_frame(t);
            for l in &mut layers {
                l.repaint(&mut e, &plan.damage, t);
            }
            for l in &layers {
                let mut full = Layer::new(l.rect, true);
                full.repaint(&mut e, &[l.rect], t);
                // tiny-skia's anti-aliasing is not perfectly translation-invariant: an isolated
                // edge pixel can differ by up to ~35/255 depending on the clip rectangle. A real
                // damage bug (a ghost or a stale edge) shows up as many pixels, far off.
                let diffs: Vec<(usize, u8)> = l
                    .bgra
                    .chunks_exact(4)
                    .zip(full.bgra.chunks_exact(4))
                    .enumerate()
                    .map(|(i, (a, b))| (i, a.iter().zip(b).map(|(p, q)| p.abs_diff(*q)).max().unwrap()))
                    .filter(|(_, d)| *d > 2)
                    .collect();
                let worst = diffs.iter().map(|(_, d)| *d).max().unwrap_or(0);
                assert!(
                    diffs.len() <= 12 && worst <= 48,
                    "{label}: step {step}: {} pixels differ from a full repaint (worst {worst}), first at ({},{}) on monitor {:?}; pointer=({x},{y}) damage={:?} alpha incremental/full = {:?}",
                    diffs.len(),
                    diffs.first().map_or(0, |(i, _)| i % l.w),
                    diffs.first().map_or(0, |(i, _)| i / l.w),
                    l.rect,
                    plan.damage,
                    diffs.first().map(|(i, _)| (l.bgra[i * 4 + 3], full.bgra[i * 4 + 3]))
                );
            }
        }
    }

    #[test]
    fn incremental_equals_full_for_every_spotlight_shape() {
        use crate::config::SpotShape;
        for shape in SpotShape::ALL {
            let mut c = Config::default();
            c.spotlight.enabled = true;
            c.spotlight.shape = *shape;
            c.spotlight.radius = if *shape == SpotShape::Circle { 70.0 } else { 30.0 };
            assert_incremental_matches_full(c, &format!("spotlight {shape:?}"));
        }
    }

    #[test]
    fn incremental_equals_full_with_trails_clicks_and_ring() {
        use crate::config::TrailStyle;
        for style in TrailStyle::ALL {
            let mut c = Config::default();
            c.trail.enabled = true;
            c.trail.style = *style;
            c.trail.lifetime_ms = 400;
            assert_incremental_matches_full(c, &format!("trail {style:?}"));
        }
    }

    #[test]
    fn incremental_equals_full_with_everything_on() {
        let mut c = Config::default();
        c.spotlight.enabled = true;
        c.spotlight.radius = 50.0;
        c.trail.enabled = true;
        c.ring.style = crate::config::RingStyle::Neon;
        assert_incremental_matches_full(c, "all effects");
    }
}
