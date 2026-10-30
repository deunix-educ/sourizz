//! The effect engine: pure state + drawing, no windowing, no I/O.
//!
//! Backends feed it [`InputEvent`]s and a monotonic clock (seconds as `f64`),
//! ask it what changed ([`Engine::begin_frame`]) and have it paint only the
//! damaged region ([`Engine::paint`]). Keeping it free of platform code makes
//! it deterministic and testable headless.

use crate::annot::Annotator;
use crate::config::{ClickStyle, Color, Config, RingStyle, SpotShape, TrailStyle};
use crate::fx;
use crate::geom::IRect;
use crate::input::{Action, Button, InputEvent, KeyEvent};
use crate::keyviz::KeyViz;
use crate::text::Fonts;
use std::collections::VecDeque;
use tiny_skia::{FillRule, Pixmap, PixmapPaint, Rect, Transform};

/// Outcome of [`Engine::begin_frame`].
pub struct FramePlan {
    /// Regions (global coordinates) that must be repainted. Disjoint-ish and small: a moving
    /// spotlight bar only reports its two soft edges, not the whole screen-wide band.
    pub damage: Vec<IRect>,
    /// Seconds until the next frame is needed; `None` means idle (wait for input).
    pub next_in: Option<f64>,
}

impl FramePlan {
    /// Bounding box of all damage (handy for tests and simple backends).
    pub fn union(&self) -> Option<IRect> {
        self.damage.iter().copied().reduce(IRect::union)
    }
}

/// Damage is tracked in a few fixed slots so a region that *moves* (a spotlight edge) is
/// unioned with its own previous position and nothing in between is missed.
const SLOTS: usize = 3;

struct TrailPt {
    x: f32,
    y: f32,
    t: f64,
}

#[derive(Clone, Copy, PartialEq)]
enum PKind {
    Star,
    Heart,
    Sparkle,
}

struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    gravity: f32,
    born: f64,
    life: f32,
    rot: f32,
    vrot: f32,
    size: f32,
    kind: PKind,
    color: [f32; 3],
}

struct Ripple {
    x: f32,
    y: f32,
    t0: f64,
    dur: f32,
    color: Color,
    style: ClickStyle,
    size: f32,
    thick: f32,
}

pub struct Engine {
    cfg: Config,
    pub fonts: Fonts,
    monitors: Vec<IRect>,
    screen: IRect,
    pointer: Option<(f32, f32)>,

    trail: VecDeque<TrailPt>,
    particles: Vec<Particle>,
    emit_dist: f32,
    last_emit: Option<(f32, f32)>,

    ripples: Vec<Ripple>,

    shake_hist: VecDeque<(f64, f32, f32)>,
    shake_t0: Option<f64>,
    shake_until: f64,

    pub annot: Annotator,
    hold_down: bool,

    keyviz: KeyViz,
    tap_since: Option<f64>,

    focus_rect: Option<IRect>,

    prev_bounds: [IRect; SLOTS],
    last_spot_monitor: Option<usize>,
    force_full: bool,
    scratch: Option<Pixmap>,
    actions: Vec<Action>,
    rng: u64,
}

fn ease_out(p: f32) -> f32 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl Engine {
    pub fn new(mut cfg: Config, fonts: Fonts) -> Self {
        cfg.sanitize();
        Self {
            cfg,
            fonts,
            monitors: vec![IRect::new(0, 0, 1920, 1080)],
            screen: IRect::new(0, 0, 1920, 1080),
            pointer: None,
            trail: VecDeque::new(),
            particles: Vec::new(),
            emit_dist: 0.0,
            last_emit: None,
            ripples: Vec::new(),
            shake_hist: VecDeque::new(),
            shake_t0: None,
            shake_until: 0.0,
            annot: Annotator::default(),
            hold_down: false,
            keyviz: KeyViz::default(),
            tap_since: None,
            focus_rect: None,
            prev_bounds: [IRect::new(0, 0, 0, 0); SLOTS],
            last_spot_monitor: None,
            force_full: true,
            scratch: None,
            actions: Vec::new(),
            rng: 0x9E37_79B9_7F4A_7C15,
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn set_config(&mut self, mut cfg: Config) {
        cfg.sanitize();
        if cfg == self.cfg {
            return;
        }
        if !cfg.trail.enabled {
            self.trail.clear();
            self.particles.clear();
        }
        self.cfg = cfg;
        self.force_full = true;
        if self.annot.active && !self.cfg.general.enabled {
            self.end_capture();
        }
    }

    pub fn set_monitors(&mut self, monitors: Vec<IRect>) {
        let monitors: Vec<IRect> = monitors.into_iter().filter(|m| !m.is_empty()).collect();
        if monitors.is_empty() || monitors == self.monitors {
            return;
        }
        self.screen = monitors.iter().fold(IRect::new(0, 0, 0, 0), |a, m| a.union(*m));
        self.monitors = monitors;
        self.force_full = true;
    }

    pub fn monitors(&self) -> &[IRect] {
        &self.monitors
    }

    pub fn pointer(&self) -> Option<(f32, f32)> {
        self.pointer
    }

    pub fn set_focus_rect(&mut self, r: Option<IRect>) {
        if r != self.focus_rect {
            if r.is_none() != self.focus_rect.is_none() {
                self.force_full = true;
            }
            self.focus_rect = r;
        }
    }

    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    pub fn capturing(&self) -> bool {
        self.annot.active
    }

    pub fn request_full_redraw(&mut self) {
        self.force_full = true;
    }

    fn rand(&mut self) -> f32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let v = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((v >> 40) as f32) / (1u64 << 24) as f32
    }

    fn monitor_at(&self, p: Option<(f32, f32)>) -> IRect {
        p.and_then(|(x, y)| self.monitors.iter().find(|m| m.contains(x, y)).copied()).unwrap_or(self.monitors[0])
    }

    // ------------------------------------------------------------------ input

    pub fn handle(&mut self, now: f64, ev: InputEvent) {
        match ev {
            InputEvent::Pointer { x, y } => self.on_pointer(now, x, y),
            InputEvent::Button { button, pressed } => self.on_button(now, button, pressed),
            InputEvent::Key(k) => self.on_key(now, &k),
        }
    }

    fn on_pointer(&mut self, now: f64, x: f32, y: f32) {
        if self.pointer == Some((x, y)) {
            return;
        }
        let prev = self.pointer;
        self.pointer = Some((x, y));
        if !self.cfg.general.enabled {
            return;
        }
        if self.annot.active {
            self.annot.pointer_move(&mut self.fonts, (x, y));
            return;
        }
        if self.hold_down && self.annot.holding() {
            self.annot.hold_move((x, y));
        }
        if self.cfg.trail.enabled {
            self.trail_move(now, prev, (x, y));
        }
        if self.cfg.shake.enabled {
            self.detect_shake(now, x, y);
        }
    }

    fn on_button(&mut self, now: f64, button: Button, pressed: bool) {
        if !self.cfg.general.enabled {
            return;
        }
        if self.annot.active {
            if let Some(p) = self.pointer {
                if pressed {
                    self.annot.pointer_down(&mut self.fonts, p, button);
                    if self.annot.take_close_request() {
                        self.end_capture();
                    }
                } else {
                    self.annot.pointer_up(button);
                }
            }
            return;
        }
        if self.cfg.sound.enabled && (pressed || self.cfg.sound.on_release) {
            self.actions.push(Action::PlayClick(button, pressed));
        }
        if pressed && self.cfg.clicks.enabled && self.cfg.clicks.show_on_clicks {
            self.spawn_ripple(now, button);
        }
    }

    fn on_key(&mut self, now: f64, k: &KeyEvent) {
        if !self.cfg.general.enabled {
            return;
        }
        if self.annot.active {
            if k.pressed && self.annot.key(k) {
                self.end_capture();
            }
            return;
        }
        // On-demand click effect: a clean tap of the configured key.
        let tap = self.cfg.clicks.tap_key.as_str();
        if !tap.is_empty() && self.cfg.clicks.enabled {
            if k.name == tap {
                if k.pressed && !k.repeat {
                    self.tap_since = Some(now);
                } else if !k.pressed {
                    if self.tap_since.take().is_some_and(|t| now - t < 0.35) {
                        self.spawn_ripple(now, Button::Left);
                    }
                }
            } else if k.pressed {
                self.tap_since = None;
            }
        }
        // Hold-to-draw.
        let hold = self.cfg.annotate.hold_key.as_str();
        if self.cfg.annotate.enabled && !hold.is_empty() && k.name == hold && !k.repeat {
            if k.pressed {
                self.hold_down = true;
                if let Some(p) = self.pointer {
                    self.annot.hold_begin(&self.cfg.annotate, p);
                }
            } else {
                self.hold_down = false;
                self.annot.hold_end(now, &self.cfg.annotate);
            }
        }
        if self.cfg.keystrokes.enabled {
            self.keyviz.key(now, k, &self.cfg.keystrokes);
        }
    }

    // ------------------------------------------------------------------ commands

    /// Flip annotation capture mode on or off.
    pub fn toggle_annotation_mode(&mut self) {
        if self.annot.active {
            self.end_capture();
        } else if self.cfg.general.enabled {
            let mon = self.monitor_at(self.pointer);
            self.annot.begin_capture(mon, &self.cfg.annotate);
            self.actions.push(Action::Capture(true));
            self.hold_down = false;
        }
    }

    fn end_capture(&mut self) {
        self.annot.end_capture();
        self.actions.push(Action::Capture(false));
    }

    pub fn clear_annotations(&mut self) {
        self.annot.clear();
    }

    /// Trigger the click effect at the pointer without a real click.
    pub fn show_click(&mut self, now: f64) {
        if self.cfg.general.enabled && self.cfg.clicks.enabled {
            self.spawn_ripple(now, Button::Left);
        }
    }

    // ------------------------------------------------------------------ effects state

    fn spawn_ripple(&mut self, now: f64, button: Button) {
        let Some((x, y)) = self.pointer else { return };
        let c = &self.cfg.clicks;
        let color = match button {
            Button::Left | Button::Other => c.left_color,
            Button::Middle => c.middle_color,
            Button::Right => c.right_color,
        };
        let dur = c.duration_ms as f32 / 1000.0;
        self.ripples.push(Ripple { x, y, t0: now, dur, color, style: c.style, size: c.size, thick: c.thickness });
        if self.ripples.len() > 24 {
            self.ripples.remove(0);
        }
    }

    fn trail_move(&mut self, now: f64, prev: Option<(f32, f32)>, p: (f32, f32)) {
        let moved = self.trail.back().map_or(f32::MAX, |l| (l.x - p.0).hypot(l.y - p.1));
        if moved >= 1.5 {
            self.trail.push_back(TrailPt { x: p.0, y: p.1, t: now });
        }
        let style = self.cfg.trail.style;
        if !matches!(style, TrailStyle::Stars | TrailStyle::Kawaii | TrailStyle::Sparkles) {
            return;
        }
        let from = prev.unwrap_or(p);
        let dist = (p.0 - from.0).hypot(p.1 - from.1);
        if dist > 300.0 {
            // Pointer jumped (monitor switch, warp): don't spray particles across the screen.
            self.last_emit = Some(p);
            return;
        }
        let spacing = (self.cfg.trail.size * 1.3).max(8.0);
        self.emit_dist += dist;
        while self.emit_dist >= spacing {
            self.emit_dist -= spacing;
            let t = if dist > 0.0 { 1.0 - (self.emit_dist / dist).min(1.0) } else { 1.0 };
            let (x, y) = (lerp(from.0, p.0, t), lerp(from.1, p.1, t));
            self.spawn_particle(now, x, y);
        }
    }

    fn spawn_particle(&mut self, now: f64, x: f32, y: f32) {
        let size = self.cfg.trail.size;
        let life = self.cfg.trail.lifetime_ms as f32 / 1000.0 * (0.8 + 0.5 * self.rand());
        let (r1, r2, r3, r4) = (self.rand(), self.rand(), self.rand(), self.rand());
        let base = self.cfg.trail.color.as_f32();
        let pastel = [[1.0, 0.62, 0.80], [0.76, 0.66, 1.0], [0.62, 0.94, 0.80], [0.62, 0.82, 1.0], [1.0, 0.80, 0.62], [1.0, 0.95, 0.62]];
        let mut p = Particle {
            x,
            y,
            vx: (r1 - 0.5) * 50.0,
            vy: (r2 - 0.5) * 50.0,
            gravity: 0.0,
            born: now,
            life,
            rot: r3 * std::f32::consts::TAU,
            vrot: (r4 - 0.5) * 6.0,
            size: size * (0.5 + 0.5 * r1),
            kind: PKind::Star,
            color: base,
        };
        match self.cfg.trail.style {
            TrailStyle::Stars => {}
            TrailStyle::Kawaii => {
                p.color = pastel[(r4 * pastel.len() as f32) as usize % pastel.len()];
                p.vy = -20.0 - 25.0 * r2;
                p.kind = if r3 < 0.55 {
                    PKind::Heart
                } else if r3 < 0.85 {
                    PKind::Sparkle
                } else {
                    PKind::Star
                };
                if p.kind == PKind::Heart {
                    p.vrot *= 0.15;
                }
            }
            TrailStyle::Sparkles => {
                p.kind = PKind::Sparkle;
                p.gravity = 110.0;
                p.size *= 0.7;
                if r4 < 0.4 {
                    p.color = [1.0, 1.0, 1.0];
                }
            }
            _ => return,
        }
        self.particles.push(p);
        if self.particles.len() > 400 {
            self.particles.remove(0);
        }
    }

    fn detect_shake(&mut self, now: f64, x: f32, y: f32) {
        self.shake_hist.push_back((now, x, y));
        while self.shake_hist.front().is_some_and(|s| now - s.0 > 0.7) {
            self.shake_hist.pop_front();
        }
        let sens = self.cfg.shake.sensitivity;
        let need = (6.0 - 4.0 * sens).round().max(2.0) as u32;
        let min_swing = lerp(140.0, 45.0, sens);
        let count = |axis: fn(&(f64, f32, f32)) -> f32| -> u32 {
            let (mut rev, mut dir, mut swing) = (0u32, 0i32, 0.0f32);
            let mut last: Option<f32> = None;
            for s in &self.shake_hist {
                let v = axis(s);
                if let Some(l) = last {
                    let d = v - l;
                    if d.abs() >= 1.0 {
                        let nd = if d > 0.0 { 1 } else { -1 };
                        if dir != 0 && nd != dir {
                            if swing >= min_swing {
                                rev += 1;
                            }
                            swing = 0.0;
                        }
                        swing += d.abs();
                        dir = nd;
                    }
                }
                last = Some(v);
            }
            rev
        };
        let revs = count(|s| s.1).max(count(|s| s.2));
        if revs >= need {
            let dur = self.cfg.shake.duration_ms as f64 / 1000.0;
            if self.shake_active(now) {
                self.shake_until = self.shake_until.max(now + dur * 0.5);
            } else {
                self.shake_t0 = Some(now);
                self.shake_until = now + dur;
            }
            self.shake_hist.clear();
        }
    }

    fn shake_active(&self, now: f64) -> bool {
        self.shake_t0.is_some() && now < self.shake_until
    }

    /// 0 when idle, 1 at full strength; eased in and out.
    fn shake_level(&self, now: f64) -> f32 {
        let Some(t0) = self.shake_t0 else { return 0.0 };
        if now >= self.shake_until {
            return 0.0;
        }
        let up = ease_out(((now - t0) / 0.15) as f32);
        let down = ease_out(((self.shake_until - now) / 0.45) as f32);
        up.min(down)
    }

    // ------------------------------------------------------------------ frame

    /// Advance animations to `now` and work out what must be repainted.
    pub fn begin_frame(&mut self, now: f64) -> FramePlan {
        self.tick(now);
        // A bar or circle that hops to another monitor changes pixels far from either edge.
        let mon = self.pointer.and_then(|(x, y)| self.monitors.iter().position(|m| m.contains(x, y)));
        if mon != self.last_spot_monitor {
            self.last_spot_monitor = mon;
            if self.cfg.spotlight.enabled {
                self.force_full = true;
            }
        }
        let cur = self.bounds(now);
        let mut damage: Vec<IRect> = if self.force_full {
            self.force_full = false;
            vec![self.screen]
        } else {
            (0..SLOTS).map(|i| self.prev_bounds[i].union(cur[i]).intersect(self.screen)).filter(|r| !r.is_empty()).collect()
        };
        self.prev_bounds = cur;
        // Merge overlapping rectangles so nothing is painted twice.
        let mut i = 0;
        while i < damage.len() {
            let mut merged = false;
            for j in (i + 1..damage.len()).rev() {
                if !damage[i].intersect(damage[j]).is_empty() {
                    damage[i] = damage[i].union(damage[j]);
                    damage.remove(j);
                    merged = true;
                }
            }
            if !merged {
                i += 1;
            }
        }
        FramePlan { damage, next_in: self.next_frame_in(now) }
    }

    fn tick(&mut self, now: f64) {
        let life = self.cfg.trail.lifetime_ms as f64 / 1000.0;
        while self.trail.front().is_some_and(|p| now - p.t > life) {
            self.trail.pop_front();
        }
        self.particles.retain(|p| (now - p.born) as f32 <= p.life);
        self.ripples.retain(|r| (now - r.t0) as f32 <= r.dur * 1.05);
        self.annot.expire(now, &self.cfg.annotate);
        if self.cfg.keystrokes.enabled {
            self.keyviz.expire(now, &self.cfg.keystrokes);
        } else {
            self.keyviz.chips.clear();
        }
        if self.shake_t0.is_some() && now >= self.shake_until {
            self.shake_t0 = None;
        }
    }

    fn next_frame_in(&self, now: f64) -> Option<f64> {
        let full = 1.0 / self.cfg.general.fps as f64;
        let busy = !self.ripples.is_empty()
            || !self.particles.is_empty()
            || !self.trail.is_empty()
            || self.shake_active(now)
            || !self.keyviz.chips.is_empty()
            || self.annot.holding()
            || self.annot.items.iter().any(|i| matches!(i, crate::annot::Item::Stroke { released_at: Some(_), fade: true, .. }))
            || self.annot.editing();
        if busy {
            return Some(full);
        }
        // The ring's gentle pulse only needs a low frame rate.
        if self.cfg.general.enabled && self.cfg.ring.enabled && self.cfg.ring.pulse_speed > 0.0 && self.pointer.is_some() {
            return Some(full.max(1.0 / 30.0));
        }
        None
    }

    fn particle_pos(p: &Particle, now: f64) -> (f32, f32) {
        let t = (now - p.born) as f32;
        (p.x + p.vx * t, p.y + p.vy * t + 0.5 * p.gravity * t * t)
    }

    fn ring_radius(&self, now: f64) -> f32 {
        let r = &self.cfg.ring;
        let pulse = if r.pulse_speed > 0.0 { 1.0 + 0.07 * (now as f32 * std::f32::consts::TAU * 0.9 * r.pulse_speed).sin() } else { 1.0 };
        r.size / 2.0 * pulse
    }

    fn shake_scale(&self, now: f64) -> f32 {
        1.0 + (self.cfg.shake.scale - 1.0) * self.shake_level(now)
    }

    /// The monitor the spotlight bar lives on: the one under the pointer.
    fn spot_monitor(&self) -> IRect {
        self.monitor_at(self.pointer)
    }

    /// Conservative boxes of everything drawn right now, in the damage slots.
    fn bounds(&mut self, now: f64) -> [IRect; SLOTS] {
        let empty = IRect::new(0, 0, 0, 0);
        let mut b = [empty; SLOTS];
        if !self.cfg.general.enabled {
            return b;
        }
        let c = self.cfg.clone();
        let mut g = empty;
        if c.focus.enabled {
            if let Some(r) = self.focus_rect {
                g = g.union(IRect::new(r.x0 - 4, r.y0 - 4, r.x1 + 4, r.y1 + 4));
            }
        }
        if let Some((px, py)) = self.pointer {
            if c.spotlight.enabled {
                let feather = c.spotlight.radius * c.spotlight.softness * 0.9;
                let (outer, inner) = (c.spotlight.radius + feather * 0.5 + 3.0, (c.spotlight.radius - feather * 0.5 - 3.0).max(0.0));
                let m = self.spot_monitor();
                match c.spotlight.shape {
                    SpotShape::Circle => b[1] = IRect::centered(px, py, outer),
                    // Only the two soft edges change while a bar slides; the lit middle does not.
                    SpotShape::HBar => {
                        b[1] = IRect::around(m.x0 as f32, py - outer, m.x1 as f32, py - inner, 0.0);
                        b[2] = IRect::around(m.x0 as f32, py + inner, m.x1 as f32, py + outer, 0.0);
                    }
                    SpotShape::VBar => {
                        b[1] = IRect::around(px - outer, m.y0 as f32, px - inner, m.y1 as f32, 0.0);
                        b[2] = IRect::around(px + inner, m.y0 as f32, px + outer, m.y1 as f32, 0.0);
                    }
                }
            }
            let scale = self.shake_scale(now);
            if c.ring.enabled || scale > 1.001 {
                let glow = if c.ring.style == RingStyle::Neon { 20.0 } else { 6.0 };
                let r = (c.ring.size / 2.0 * 1.12 + c.ring.thickness + glow) * scale;
                g = g.union(IRect::centered(px, py, r));
            }
        }
        for t in &self.trail {
            g = g.union(IRect::centered(t.x, t.y, c.trail.size + 4.0));
        }
        for p in &self.particles {
            let (x, y) = Self::particle_pos(p, now);
            g = g.union(IRect::centered(x, y, p.size * 1.8 + 3.0));
        }
        for r in &self.ripples {
            g = g.union(IRect::centered(r.x, r.y, r.size * 1.05 + r.thick + 4.0));
        }
        g = g.union(self.annot.bounds(&mut self.fonts));
        if c.keystrokes.enabled {
            let mon = self.monitor_at(self.pointer);
            g = g.union(self.keyviz.bounds(&mut self.fonts, &c.keystrokes, mon, now));
        }
        b[0] = g;
        b
    }

    // ------------------------------------------------------------------ painting

    /// Paint the whole scene clipped to `region` (global coordinates) into `pm`,
    /// which must be exactly `region`-sized. The pixmap is cleared first.
    pub fn paint(&mut self, pm: &mut Pixmap, region: IRect, now: f64) {
        pm.fill(tiny_skia::Color::TRANSPARENT);
        if !self.cfg.general.enabled || region.is_empty() {
            return;
        }
        let tf = Transform::from_translate(-(region.x0 as f32), -(region.y0 as f32));
        let area = Rect::from_ltrb(region.x0 as f32, region.y0 as f32, region.x1 as f32, region.y1 as f32).unwrap();
        let c = self.cfg.clone();

        if c.focus.enabled {
            if let Some(r) = self.focus_rect {
                if let Some(hole) = Rect::from_ltrb(r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32) {
                    fx::dim_with_rect(pm, tf, area, hole, 8.0, c.focus.color.sk(c.focus.opacity));
                }
            }
        }
        if let (true, Some((px, py))) = (c.spotlight.enabled, self.pointer) {
            let dim = c.spotlight.color.sk(c.spotlight.opacity);
            match c.spotlight.shape {
                SpotShape::Circle => fx::dim_with_circle(pm, tf, area, px, py, c.spotlight.radius, c.spotlight.softness, dim),
                SpotShape::HBar | SpotShape::VBar => {
                    let m = self.spot_monitor();
                    let span = Rect::from_ltrb(m.x0 as f32, m.y0 as f32, m.x1 as f32, m.y1 as f32).unwrap();
                    let horizontal = c.spotlight.shape == SpotShape::HBar;
                    let centre = if horizontal { py } else { px };
                    fx::dim_with_bar(pm, tf, area, span, horizontal, centre, c.spotlight.radius, c.spotlight.softness, dim);
                }
            }
        }

        self.annot.paint(pm, tf, &mut self.fonts, now, &c.annotate);

        if c.trail.enabled {
            self.paint_trail(pm, tf, now);
        }
        for r in &self.ripples {
            Self::paint_ripple(pm, tf, r, now);
        }
        if let Some((px, py)) = self.pointer {
            if !self.annot.active {
                self.paint_ring(pm, tf, now, px, py);
            }
        }
        if c.keystrokes.enabled {
            let mon = self.monitor_at(self.pointer);
            self.keyviz.paint(pm, tf, &mut self.fonts, &c.keystrokes, mon, now);
        }
    }

    fn paint_ring(&self, pm: &mut Pixmap, tf: Transform, now: f64, px: f32, py: f32) {
        let r = &self.cfg.ring;
        let level = self.shake_level(now);
        let scale = self.shake_scale(now);
        if !r.enabled && level <= 0.0 {
            return;
        }
        // When the ring is switched off, shaking still shows a temporary locator ring.
        let alpha = if r.enabled { r.opacity } else { r.opacity * level };
        let radius = self.ring_radius(now) * scale;
        let fill = if r.enabled { r.fill } else { 0.18 * level };
        match r.style {
            RingStyle::Classic => {
                fx::stroke_circle(pm, tf, px, py, radius, r.thickness + 3.0, fx::rgba([0.0, 0.0, 0.0], 0.30 * alpha));
                if fill > 0.0 {
                    fx::fill_circle(pm, tf, px, py, radius, r.color.sk(fill));
                }
                fx::stroke_circle(pm, tf, px, py, radius, r.thickness, r.color.sk(alpha));
            }
            RingStyle::Neon => {
                for i in (1..=6).rev() {
                    let w = r.thickness + i as f32 * 3.2;
                    fx::stroke_circle(pm, tf, px, py, radius, w, r.color.sk(0.075 * alpha));
                }
                if fill > 0.0 {
                    fx::fill_circle(pm, tf, px, py, radius, r.color.sk(fill));
                }
                fx::stroke_circle(pm, tf, px, py, radius, r.thickness, r.color.sk(alpha));
                fx::stroke_circle(pm, tf, px, py, radius, (r.thickness * 0.4).max(1.0), fx::rgba([1.0, 1.0, 1.0], 0.85 * alpha));
            }
        }
    }

    fn take_scratch(&mut self, w: u32, h: u32) -> Option<Pixmap> {
        match self.scratch.take() {
            Some(p) if p.width() == w && p.height() == h => Some(p),
            _ => Pixmap::new(w, h),
        }
    }

    fn paint_trail(&mut self, pm: &mut Pixmap, tf: Transform, now: f64) {
        let c = self.cfg.trail.clone();
        let life = (c.lifetime_ms as f64 / 1000.0) as f32;
        match c.style {
            TrailStyle::Comet | TrailStyle::Ribbon | TrailStyle::Rainbow | TrailStyle::Dots => {
                if self.trail.len() < 2 {
                    return;
                }
                let Some(mut layer) = self.take_scratch(pm.width(), pm.height()) else { return };
                layer.fill(tiny_skia::Color::TRANSPARENT);
                let pts: Vec<(f32, f32, f32)> =
                    self.trail.iter().map(|p| (p.x, p.y, (1.0 - (now - p.t) as f32 / life).clamp(0.0, 1.0))).collect();
                let src = tiny_skia::BlendMode::Source;
                match c.style {
                    TrailStyle::Dots => {
                        let mut last: Option<(f32, f32)> = None;
                        for &(x, y, f) in pts.iter().rev() {
                            if last.is_some_and(|(lx, ly)| (lx - x).hypot(ly - y) < c.size * 1.25) {
                                continue;
                            }
                            last = Some((x, y));
                            fx::fill_circle(&mut layer, tf, x, y, c.size * 0.42 * (0.35 + 0.65 * f), c.color.sk(f.powf(1.1)));
                        }
                    }
                    _ => {
                        for w in pts.windows(2) {
                            let (a, b) = (w[0], w[1]);
                            let f = b.2;
                            let (width, color) = match c.style {
                                TrailStyle::Comet => (c.size * (0.15 + 0.85 * f), c.color.sk(f.powf(1.3))),
                                TrailStyle::Ribbon => (c.size * 1.25 * (0.2 + 0.8 * f.sqrt()), c.color.sk(0.62 * f.powf(0.9))),
                                _ => {
                                    let hue = (now as f32 * 140.0 + (1.0 - f) * 300.0) % 360.0;
                                    (c.size * (0.2 + 0.8 * f), fx::rgba(fx::hsv(hue, 0.85, 1.0), f.powf(1.2)))
                                }
                            };
                            fx::stroke_line(&mut layer, tf, (a.0, a.1), (b.0, b.1), width, color, src);
                        }
                        if c.style == TrailStyle::Ribbon {
                            for w in pts.windows(2) {
                                let (a, b) = (w[0], w[1]);
                                fx::stroke_line(
                                    &mut layer,
                                    tf,
                                    (a.0, a.1),
                                    (b.0, b.1),
                                    (c.size * 0.28 * b.2).max(0.5),
                                    fx::rgba([1.0, 1.0, 1.0], 0.55 * b.2),
                                    tiny_skia::BlendMode::SourceOver,
                                );
                            }
                        }
                    }
                }
                pm.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                self.scratch = Some(layer);
            }
            TrailStyle::Stars | TrailStyle::Kawaii | TrailStyle::Sparkles => {
                for p in &self.particles {
                    let age = (now - p.born) as f32;
                    let f = (age / p.life).clamp(0.0, 1.0);
                    let alpha = (1.0 - f).powf(1.4) * (age / 0.06).min(1.0);
                    let (x, y) = Self::particle_pos(p, now);
                    let size = p.size * (1.0 - 0.55 * f);
                    let rot = p.rot + p.vrot * age;
                    let col = fx::rgba(p.color, alpha);
                    let path = match p.kind {
                        PKind::Star => fx::star(x, y, size, size * 0.45, 5, rot),
                        PKind::Heart => fx::heart(x, y, size * 0.62),
                        PKind::Sparkle => fx::sparkle(x, y, size * 1.15, rot),
                    };
                    if let Some(path) = path {
                        pm.fill_path(&path, &fx::solid(col), FillRule::Winding, tf, None);
                        if p.kind == PKind::Heart {
                            // Little glint so hearts read as glossy.
                            fx::fill_circle(pm, tf, x - size * 0.28, y - size * 0.3, size * 0.12, fx::rgba([1.0, 1.0, 1.0], 0.75 * alpha));
                        }
                    }
                }
            }
        }
    }

    fn paint_ripple(pm: &mut Pixmap, tf: Transform, r: &Ripple, now: f64) {
        let p = ((now - r.t0) as f32 / r.dur).clamp(0.0, 1.0);
        if p >= 1.0 {
            return;
        }
        let half = r.size * 0.5;
        match r.style {
            ClickStyle::SingleRing => {
                let e = ease_out(p);
                let radius = lerp(half * 0.2, half, e);
                let alpha = (1.0 - p).powf(1.3);
                fx::stroke_circle(pm, tf, r.x, r.y, radius, r.thick * (1.0 - 0.5 * p), r.color.sk(alpha));
            }
            ClickStyle::RippleWave => {
                for k in 0..3 {
                    let pk = ((p - k as f32 * 0.14) / 0.72).clamp(0.0, 1.0);
                    if pk <= 0.0 || pk >= 1.0 {
                        continue;
                    }
                    let radius = lerp(half * 0.15, half * (1.0 + 0.4 * k as f32), ease_out(pk));
                    fx::stroke_circle(pm, tf, r.x, r.y, radius, r.thick * (1.0 - 0.6 * pk), r.color.sk((1.0 - pk).powf(1.2)));
                }
            }
            ClickStyle::Pulse => {
                let e = ease_out(p);
                let radius = lerp(half * 0.15, half, e);
                fx::fill_circle(pm, tf, r.x, r.y, radius, r.color.sk(0.42 * (1.0 - p)));
                fx::stroke_circle(pm, tf, r.x, r.y, radius, r.thick * 0.7, r.color.sk(1.0 - p));
            }
            ClickStyle::Burst => {
                let e = ease_out(p);
                let alpha = (1.0 - p).powf(1.2);
                let (inner, outer) = (lerp(half * 0.2, half * 0.65, e), lerp(half * 0.45, half * 1.05, e));
                for i in 0..12 {
                    let a = i as f32 / 12.0 * std::f32::consts::TAU;
                    let (s, c) = a.sin_cos();
                    fx::stroke_line(
                        pm,
                        tf,
                        (r.x + c * inner, r.y + s * inner),
                        (r.x + c * outer, r.y + s * outer),
                        r.thick * (1.0 - 0.5 * p),
                        r.color.sk(alpha),
                        tiny_skia::BlendMode::SourceOver,
                    );
                }
            }
        }
    }

    pub fn summary(&self) -> String {
        let c = &self.cfg;
        let on = |b: bool| if b { "on" } else { "off" };
        format!(
            "enabled={} spotlight={} ring={} trail={} clicks={} sound={} shake={} annotate={} keystrokes={} focus={} capture={}",
            on(c.general.enabled),
            on(c.spotlight.enabled),
            on(c.ring.enabled),
            on(c.trail.enabled),
            on(c.clicks.enabled),
            on(c.sound.enabled),
            on(c.shake.enabled),
            on(c.annotate.enabled),
            on(c.keystrokes.enabled),
            on(c.focus.enabled),
            on(self.annot.active),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ClickStyle, TrailStyle};

    fn engine() -> Engine {
        let mut e = Engine::new(Config::default(), Fonts::none());
        e.set_monitors(vec![IRect::new(0, 0, 800, 600)]);
        e
    }

    fn zigzag(e: &mut Engine, t0: f64) {
        for i in 0..14 {
            let x = if i % 2 == 0 { 100.0 } else { 400.0 };
            e.handle(t0 + i as f64 * 0.04, InputEvent::Pointer { x, y: 300.0 });
        }
    }

    #[test]
    fn shake_triggers_on_zigzag_but_not_on_a_straight_move() {
        let mut e = engine();
        for i in 0..40 {
            e.handle(i as f64 * 0.01, InputEvent::Pointer { x: i as f32 * 10.0, y: 100.0 });
        }
        assert!(!e.shake_active(0.4), "straight motion must not shake");
        let mut e = engine();
        zigzag(&mut e, 0.0);
        assert!(e.shake_active(0.5));
        assert!(e.shake_scale(0.5) > 2.0);
        assert!(e.shake_scale(5.0) - 1.0 < 1e-3, "shake decays");
    }

    #[test]
    fn small_jitter_does_not_shake() {
        let mut e = engine();
        for i in 0..40 {
            let x = 300.0 + if i % 2 == 0 { 0.0 } else { 6.0 };
            e.handle(i as f64 * 0.01, InputEvent::Pointer { x, y: 100.0 });
        }
        assert!(!e.shake_active(0.4));
    }

    #[test]
    fn damage_covers_previous_and_current_ring() {
        let mut e = engine();
        e.handle(0.0, InputEvent::Pointer { x: 100.0, y: 100.0 });
        let _ = e.begin_frame(0.0); // consumes the initial full redraw
        e.handle(0.1, InputEvent::Pointer { x: 400.0, y: 300.0 });
        let d = e.begin_frame(0.1).union().unwrap();
        assert!(d.contains(100.0, 100.0), "old position must be repainted (erased)");
        assert!(d.contains(400.0, 300.0), "new position must be repainted");
        assert!(d.width() < 800, "partial damage, not full screen");
    }

    #[test]
    fn first_frame_and_config_change_are_full_redraws() {
        let mut e = engine();
        assert_eq!(e.begin_frame(0.0).damage, vec![IRect::new(0, 0, 800, 600)]);
        let mut c = e.config().clone();
        c.spotlight.enabled = true;
        e.set_config(c);
        assert_eq!(e.begin_frame(0.1).damage, vec![IRect::new(0, 0, 800, 600)]);
    }

    #[test]
    fn idle_when_nothing_animates() {
        let mut cfg = Config::default();
        cfg.ring.pulse_speed = 0.0;
        let mut e = Engine::new(cfg, Fonts::none());
        e.set_monitors(vec![IRect::new(0, 0, 800, 600)]);
        e.handle(0.0, InputEvent::Pointer { x: 10.0, y: 10.0 });
        assert!(e.begin_frame(0.0).next_in.is_none());
    }

    #[test]
    fn pulsing_ring_runs_at_low_rate() {
        let mut e = engine();
        e.handle(0.0, InputEvent::Pointer { x: 10.0, y: 10.0 });
        let next = e.begin_frame(0.0).next_in.unwrap();
        assert!(next >= 1.0 / 30.0 - 1e-9);
    }

    #[test]
    fn click_spawns_ripple_then_expires() {
        let mut e = engine();
        e.handle(0.0, InputEvent::Pointer { x: 50.0, y: 50.0 });
        e.handle(0.0, InputEvent::Button { button: Button::Left, pressed: true });
        assert_eq!(e.ripples.len(), 1);
        assert!(e.begin_frame(0.1).next_in.is_some());
        let _ = e.begin_frame(5.0);
        assert!(e.ripples.is_empty());
    }

    #[test]
    fn click_effect_respects_switches() {
        let mut c = Config::default();
        c.clicks.show_on_clicks = false;
        let mut e = Engine::new(c, Fonts::none());
        e.handle(0.0, InputEvent::Pointer { x: 50.0, y: 50.0 });
        e.handle(0.0, InputEvent::Button { button: Button::Left, pressed: true });
        assert!(e.ripples.is_empty());
        e.show_click(0.1);
        assert_eq!(e.ripples.len(), 1);
    }

    #[test]
    fn sound_action_emitted_only_when_enabled() {
        let mut e = engine();
        e.handle(0.0, InputEvent::Pointer { x: 5.0, y: 5.0 });
        e.handle(0.0, InputEvent::Button { button: Button::Left, pressed: true });
        assert!(e.take_actions().is_empty());
        let mut c = e.config().clone();
        c.sound.enabled = true;
        e.set_config(c);
        e.handle(0.1, InputEvent::Button { button: Button::Right, pressed: true });
        e.handle(0.1, InputEvent::Button { button: Button::Right, pressed: false });
        assert_eq!(e.take_actions(), vec![Action::PlayClick(Button::Right, true)]);
    }

    #[test]
    fn trail_points_expire() {
        let mut c = Config::default();
        c.trail.enabled = true;
        let mut e = Engine::new(c, Fonts::none());
        e.set_monitors(vec![IRect::new(0, 0, 800, 600)]);
        for i in 0..10 {
            e.handle(i as f64 * 0.01, InputEvent::Pointer { x: i as f32 * 20.0, y: 10.0 });
        }
        assert!(e.trail.len() >= 9);
        let _ = e.begin_frame(10.0);
        assert!(e.trail.is_empty());
    }

    #[test]
    fn master_switch_disables_everything() {
        let mut c = Config::default();
        c.general.enabled = false;
        let mut e = Engine::new(c, Fonts::none());
        e.set_monitors(vec![IRect::new(0, 0, 800, 600)]);
        e.handle(0.0, InputEvent::Pointer { x: 50.0, y: 50.0 });
        e.handle(0.0, InputEvent::Button { button: Button::Left, pressed: true });
        let _ = e.begin_frame(0.0);
        assert!(e.ripples.is_empty());
        let plan = e.begin_frame(0.1);
        assert!(plan.damage.is_empty());
        assert!(plan.next_in.is_none());
    }

    #[test]
    fn hold_key_draws_and_fades() {
        let mut e = engine();
        e.handle(0.0, InputEvent::Pointer { x: 10.0, y: 10.0 });
        let key = |pressed| KeyEvent {
            pressed,
            name: "Control_R".into(),
            label: "Ctrl".into(),
            text: None,
            mods: 0,
            is_modifier: true,
            repeat: false,
        };
        e.handle(0.1, InputEvent::Key(key(true)));
        e.handle(0.2, InputEvent::Pointer { x: 60.0, y: 40.0 });
        e.handle(0.3, InputEvent::Pointer { x: 120.0, y: 90.0 });
        e.handle(0.4, InputEvent::Key(key(false)));
        assert_eq!(e.annot.items.len(), 1);
        e.handle(0.5, InputEvent::Pointer { x: 200.0, y: 200.0 });
        match &e.annot.items[0] {
            crate::annot::Item::Stroke { pts, .. } => assert_eq!(pts.len(), 3, "movement after release must not extend the stroke"),
            _ => panic!("expected a stroke"),
        }
        let _ = e.begin_frame(10.0);
        assert!(e.annot.items.is_empty(), "faded stroke is removed");
    }

    #[test]
    fn annotation_mode_requests_and_releases_capture() {
        let mut e = engine();
        e.handle(0.0, InputEvent::Pointer { x: 400.0, y: 300.0 });
        e.toggle_annotation_mode();
        assert_eq!(e.take_actions(), vec![Action::Capture(true)]);
        assert!(e.capturing());
        let esc =
            KeyEvent { pressed: true, name: "Escape".into(), label: "Esc".into(), text: None, mods: 0, is_modifier: false, repeat: false };
        e.handle(1.0, InputEvent::Key(esc));
        assert_eq!(e.take_actions(), vec![Action::Capture(false)]);
        assert!(!e.capturing());
    }

    #[test]
    fn pen_stroke_in_capture_mode() {
        let mut e = engine();
        e.handle(0.0, InputEvent::Pointer { x: 400.0, y: 300.0 });
        e.toggle_annotation_mode();
        e.handle(0.1, InputEvent::Button { button: Button::Left, pressed: true });
        e.handle(0.2, InputEvent::Pointer { x: 450.0, y: 350.0 });
        e.handle(0.3, InputEvent::Pointer { x: 500.0, y: 380.0 });
        e.handle(0.4, InputEvent::Button { button: Button::Left, pressed: false });
        assert_eq!(e.annot.items.len(), 1);
    }

    /// Every style must paint something visible without panicking.
    #[test]
    fn all_styles_render_pixels() {
        let styles = TrailStyle::ALL;
        for &style in styles {
            let mut c = Config::default();
            c.trail.enabled = true;
            c.trail.style = style;
            let mut e = Engine::new(c, Fonts::none());
            e.set_monitors(vec![IRect::new(0, 0, 600, 300)]);
            for i in 0..60 {
                let t = i as f64 * 0.01;
                e.handle(t, InputEvent::Pointer { x: 50.0 + i as f32 * 8.0, y: 150.0 + (i as f32 * 0.3).sin() * 60.0 });
            }
            let plan = e.begin_frame(0.6);
            let d = plan.union().unwrap();
            let mut pm = Pixmap::new(d.width() as u32, d.height() as u32).unwrap();
            e.paint(&mut pm, d, 0.6);
            assert!(pm.data().chunks(4).any(|p| p[3] > 0), "trail style {style:?} painted nothing");
        }
        for &style in ClickStyle::ALL {
            let mut c = Config::default();
            c.clicks.style = style;
            let mut e = Engine::new(c, Fonts::none());
            e.set_monitors(vec![IRect::new(0, 0, 400, 300)]);
            e.handle(0.0, InputEvent::Pointer { x: 200.0, y: 150.0 });
            e.handle(0.0, InputEvent::Button { button: Button::Left, pressed: true });
            let d = e.begin_frame(0.15).union().unwrap();
            let mut pm = Pixmap::new(d.width() as u32, d.height() as u32).unwrap();
            e.paint(&mut pm, d, 0.15);
            assert!(pm.data().chunks(4).any(|p| p[3] > 0), "click style {style:?} painted nothing");
        }
    }

    #[test]
    fn spotlight_leaves_a_hole_and_dims_the_rest() {
        let mut c = Config::default();
        c.spotlight.enabled = true;
        c.ring.enabled = false;
        c.spotlight.radius = 60.0;
        c.spotlight.opacity = 0.8;
        let mut e = Engine::new(c, Fonts::none());
        e.set_monitors(vec![IRect::new(0, 0, 400, 300)]);
        e.handle(0.0, InputEvent::Pointer { x: 200.0, y: 150.0 });
        let d = e.begin_frame(0.0).union().unwrap();
        assert_eq!(d, IRect::new(0, 0, 400, 300));
        let mut pm = Pixmap::new(400, 300).unwrap();
        e.paint(&mut pm, d, 0.0);
        let alpha = |x: usize, y: usize| pm.data()[(y * 400 + x) * 4 + 3];
        assert_eq!(alpha(200, 150), 0, "centre is lit");
        assert!(alpha(5, 5) > 180, "corner is dimmed (got {})", alpha(5, 5));
    }
}
