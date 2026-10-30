//! Keystroke visualiser: shows recent key presses as chips near a screen edge.
//!
//! Plain typing is merged into one running text chip; shortcuts (`Ctrl+Shift+T`)
//! and special keys each get their own chip.

use crate::config::{Color, KeyPosition, KeyStyle, Keystrokes, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER};
use crate::fx;
use crate::geom::IRect;
use crate::input::KeyEvent;
use crate::text::Fonts;
use tiny_skia::{GradientStop, LinearGradient, Paint, Pixmap, Point, SpreadMode, Transform};

const MAX_CHIPS: usize = 6;
const MAX_RUN: usize = 28;

#[derive(Clone, Debug)]
pub struct Chip {
    pub parts: Vec<String>,
    /// A run of ordinary typed characters, extended by further typing.
    pub run: bool,
    pub born: f64,
    pub last: f64,
}

#[derive(Default)]
pub struct KeyViz {
    pub chips: Vec<Chip>,
}

struct Placed {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    alpha: f32,
    idx: usize,
}

fn mod_parts(mods: u8) -> Vec<String> {
    let mut v = Vec::new();
    if mods & MOD_CTRL != 0 {
        v.push("Ctrl".to_string());
    }
    if mods & MOD_ALT != 0 {
        v.push("Alt".to_string());
    }
    if mods & MOD_SHIFT != 0 {
        v.push("Shift".to_string());
    }
    if mods & MOD_SUPER != 0 {
        v.push("Super".to_string());
    }
    v
}

impl KeyViz {
    pub fn key(&mut self, now: f64, ev: &KeyEvent, cfg: &Keystrokes) {
        if !ev.pressed || ev.is_modifier {
            return;
        }
        let shortcut = ev.mods & (MOD_CTRL | MOD_ALT | MOD_SUPER) != 0;
        if cfg.only_shortcuts && !shortcut {
            return;
        }
        // Ordinary typing: extend the running text chip.
        if !shortcut {
            if let Some(ch) = ev.text.filter(|c| !c.is_control() && *c != ' ') {
                if let Some(last) = self.chips.last_mut() {
                    if last.run && now - last.last < 1.4 && last.parts[0].chars().count() < MAX_RUN {
                        last.parts[0].push(ch);
                        last.last = now;
                        return;
                    }
                }
                self.push(Chip { parts: vec![ch.to_string()], run: true, born: now, last: now });
                return;
            }
        }
        let mut parts = mod_parts(ev.mods);
        parts.push(ev.label.clone());
        // Auto-repeat of the same combo refreshes instead of stacking.
        if let Some(last) = self.chips.last_mut() {
            if !last.run && last.parts == parts && now - last.last < 0.6 {
                last.last = now;
                return;
            }
        }
        self.push(Chip { parts, run: false, born: now, last: now });
    }

    fn push(&mut self, c: Chip) {
        self.chips.push(c);
        if self.chips.len() > MAX_CHIPS {
            self.chips.remove(0);
        }
    }

    /// Drop expired chips; true while any remain (needs animation frames for the fade).
    pub fn expire(&mut self, now: f64, cfg: &Keystrokes) -> bool {
        let life = cfg.duration_ms as f64 / 1000.0;
        self.chips.retain(|c| now - c.last < life);
        !self.chips.is_empty()
    }

    fn layout(&self, fonts: &mut Fonts, cfg: &Keystrokes, mon: IRect, now: f64) -> Vec<Placed> {
        if !fonts.available() || self.chips.is_empty() {
            return Vec::new();
        }
        let px = cfg.size;
        let h = (px * 1.75).round();
        let pad = px * 0.65;
        let gap = (px * 0.45).round();
        let life = cfg.duration_ms as f64 / 1000.0;
        let mut items: Vec<(usize, f32)> = Vec::new();
        for (i, c) in self.chips.iter().enumerate() {
            let w = match cfg.style {
                KeyStyle::Keycaps => {
                    let caps: f32 = c.parts.iter().map(|p| fonts.measure(p, px).max(px * 0.6) + pad * 1.1).sum();
                    caps + (c.parts.len().saturating_sub(1)) as f32 * 6.0
                }
                _ => fonts.measure(&c.parts.join(" + "), px) + pad * 2.0,
            };
            items.push((i, w.round()));
        }
        let total: f32 = items.iter().map(|(_, w)| *w).sum::<f32>() + gap * (items.len().saturating_sub(1)) as f32;
        let margin_y = (px * 2.2).round();
        let y = match cfg.position {
            KeyPosition::TopCenter => mon.y0 as f32 + margin_y,
            _ => mon.y1 as f32 - margin_y - h,
        };
        let mut x = match cfg.position {
            KeyPosition::BottomLeft => mon.x0 as f32 + 40.0,
            KeyPosition::BottomRight => mon.x1 as f32 - 40.0 - total,
            _ => mon.x0 as f32 + (mon.width() as f32 - total) / 2.0,
        };
        let mut out = Vec::new();
        for (i, w) in items {
            let c = &self.chips[i];
            let age = (now - c.last) / life;
            let fade = if age > 0.7 { (1.0 - (age - 0.7) / 0.3).clamp(0.0, 1.0) as f32 } else { 1.0 };
            let pop = (((now - c.born) / 0.12) as f32).clamp(0.0, 1.0);
            let dy = (1.0 - pop) * 8.0;
            out.push(Placed { x, y: y + dy, w, h, alpha: fade * pop.max(0.2), idx: i });
            x += w + gap;
        }
        out
    }

    pub fn bounds(&self, fonts: &mut Fonts, cfg: &Keystrokes, mon: IRect, now: f64) -> IRect {
        let mut r = IRect::new(0, 0, 0, 0);
        for p in self.layout(fonts, cfg, mon, now) {
            r = r.union(IRect::around(p.x, p.y - 8.0, p.x + p.w, p.y + p.h + 12.0, 12.0));
        }
        r
    }

    pub fn paint(&self, pm: &mut Pixmap, tf: Transform, fonts: &mut Fonts, cfg: &Keystrokes, mon: IRect, now: f64) {
        let px = cfg.size;
        let (ox, oy) = (tf.tx, tf.ty);
        for p in self.layout(fonts, cfg, mon, now) {
            let chip = &self.chips[p.idx];
            let a = p.alpha;
            let (x, y, w, h) = (p.x, p.y, p.w, p.h);
            let r = h * 0.28;
            let base = y + h / 2.0 + px * 0.34;
            let text = chip.parts.join(" + ");
            let tw = fonts.measure(&text, px);
            let tx = x + (w - tw) / 2.0;
            let accent = cfg.accent;
            match cfg.style {
                KeyStyle::Classic => {
                    fx::fill_rrect(pm, tf, x, y, w, h, r, &fx::solid(tiny_skia::Color::from_rgba8(22, 22, 26, (a * 225.0) as u8)));
                    fonts.draw(pm, &text, tx + ox, base + oy, px, Color::rgb(245, 245, 247), a);
                }
                KeyStyle::Glass => {
                    fx::fill_rrect(pm, tf, x, y, w, h, r, &fx::solid(tiny_skia::Color::from_rgba8(30, 30, 40, (a * 120.0) as u8)));
                    fx::fill_rrect(pm, tf, x, y, w, h * 0.5, r, &fx::solid(tiny_skia::Color::from_rgba8(255, 255, 255, (a * 38.0) as u8)));
                    fx::stroke_rrect(
                        pm,
                        tf,
                        x + 0.5,
                        y + 0.5,
                        w - 1.0,
                        h - 1.0,
                        r,
                        1.2,
                        tiny_skia::Color::from_rgba8(255, 255, 255, (a * 110.0) as u8),
                    );
                    fonts.draw(pm, &text, tx + ox, base + oy, px, Color::rgb(255, 255, 255), a);
                }
                KeyStyle::Neon => {
                    fx::fill_rrect(pm, tf, x, y, w, h, r, &fx::solid(tiny_skia::Color::from_rgba8(10, 10, 16, (a * 235.0) as u8)));
                    for (grow, al) in [(7.0f32, 0.07), (4.0, 0.12), (2.0, 0.22)] {
                        fx::stroke_rrect(
                            pm,
                            tf,
                            x - grow / 2.0,
                            y - grow / 2.0,
                            w + grow,
                            h + grow,
                            r + grow / 2.0,
                            grow,
                            accent.sk(al * a),
                        );
                    }
                    fx::stroke_rrect(pm, tf, x, y, w, h, r, 2.0, accent.sk(a));
                    fonts.draw(pm, &text, tx + ox, base + oy, px, accent, a);
                }
                KeyStyle::Gradient => {
                    let c2 = {
                        let [r0, g0, b0] = accent.as_f32();
                        Color::from_f32([b0 * 0.9 + 0.1, r0 * 0.6, g0 * 0.4 + 0.6])
                    };
                    let stops = vec![GradientStop::new(0.0, accent.sk(a)), GradientStop::new(1.0, c2.sk(a))];
                    if let Some(shader) = LinearGradient::new(
                        Point::from_xy(x, y),
                        Point::from_xy(x + w, y + h),
                        stops,
                        SpreadMode::Pad,
                        Transform::identity(),
                    ) {
                        let paint = Paint { shader, anti_alias: true, ..Paint::default() };
                        fx::fill_rrect(pm, tf, x, y, w, h, r, &paint);
                    }
                    fonts.draw(pm, &text, tx + ox, base + oy, px, Color::rgb(255, 255, 255), a);
                }
                KeyStyle::Minimal => {
                    fonts.draw(pm, &text, tx + ox + 2.0, base + oy + 2.0, px * 1.15, Color { r: 0, g: 0, b: 0, a: 170 }, a);
                    fonts.draw(pm, &text, tx + ox, base + oy, px * 1.15, Color::rgb(255, 255, 255), a);
                }
                KeyStyle::Keycaps => {
                    let mut cx = x;
                    for part in &chip.parts {
                        let cw = fonts.measure(part, px).max(px * 0.6) + px * 0.65 * 1.1;
                        fx::fill_rrect(
                            pm,
                            tf,
                            cx,
                            y + 4.0,
                            cw,
                            h - 2.0,
                            r * 0.8,
                            &fx::solid(tiny_skia::Color::from_rgba8(70, 70, 76, (a * 255.0) as u8)),
                        );
                        fx::fill_rrect(
                            pm,
                            tf,
                            cx,
                            y,
                            cw,
                            h - 5.0,
                            r * 0.8,
                            &fx::solid(tiny_skia::Color::from_rgba8(240, 240, 243, (a * 255.0) as u8)),
                        );
                        let pw = fonts.measure(part, px);
                        fonts.draw(
                            pm,
                            part,
                            cx + (cw - pw) / 2.0 + ox,
                            y + (h - 5.0) / 2.0 + px * 0.34 + oy,
                            px,
                            Color::rgb(30, 30, 34),
                            a,
                        );
                        cx += cw + 6.0;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(name: &str, label: &str, text: Option<char>, mods: u8) -> KeyEvent {
        KeyEvent { pressed: true, name: name.into(), label: label.into(), text, mods, is_modifier: false, repeat: false }
    }

    #[test]
    fn typing_merges_into_one_run() {
        let cfg = Keystrokes::default();
        let mut kv = KeyViz::default();
        for (i, c) in "hello".chars().enumerate() {
            kv.key(i as f64 * 0.1, &ev(&c.to_string(), &c.to_uppercase().to_string(), Some(c), 0), &cfg);
        }
        assert_eq!(kv.chips.len(), 1);
        assert_eq!(kv.chips[0].parts[0], "hello");
    }

    #[test]
    fn shortcut_gets_its_own_chip() {
        let cfg = Keystrokes::default();
        let mut kv = KeyViz::default();
        kv.key(0.0, &ev("t", "T", Some('t'), MOD_CTRL | MOD_SHIFT), &cfg);
        assert_eq!(kv.chips[0].parts, vec!["Ctrl", "Shift", "T"]);
    }

    #[test]
    fn only_shortcuts_hides_plain_typing() {
        let cfg = Keystrokes { only_shortcuts: true, ..Keystrokes::default() };
        let mut kv = KeyViz::default();
        kv.key(0.0, &ev("a", "A", Some('a'), 0), &cfg);
        assert!(kv.chips.is_empty());
        kv.key(0.1, &ev("a", "A", Some('a'), MOD_CTRL), &cfg);
        assert_eq!(kv.chips.len(), 1);
    }

    #[test]
    fn chips_expire() {
        let cfg = Keystrokes::default();
        let mut kv = KeyViz::default();
        kv.key(0.0, &ev("Return", "Enter", None, 0), &cfg);
        assert!(kv.expire(1.0, &cfg));
        assert!(!kv.expire(10.0, &cfg));
    }
}
