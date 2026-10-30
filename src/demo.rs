//! Headless renderer: draws each effect over a sample background and writes PNGs.
//! Used for visual checks and for the screenshots in the README.

use crate::annot::Tool;
use crate::config::*;
use crate::engine::Engine;
use crate::geom::IRect;
use crate::input::{Button, InputEvent, KeyEvent};
use crate::text::Fonts;
use anyhow::{Context, Result};
use std::path::Path;
use tiny_skia::{Pixmap, PixmapPaint, Transform};

const W: u32 = 760;
const H: u32 = 300;

fn background() -> Pixmap {
    let mut bg = Pixmap::new(W, H).unwrap();
    let data = bg.data_mut();
    for y in 0..H as usize {
        for x in 0..W as usize {
            let i = (y * W as usize + x) * 4;
            let t = x as f32 / W as f32;
            // Soft slate gradient with a faint grid, vaguely like a document or desktop.
            let grid = if x % 40 == 0 || y % 40 == 0 { 14.0 } else { 0.0 };
            data[i] = (52.0 + 40.0 * t + grid) as u8;
            data[i + 1] = (62.0 + 36.0 * t + grid) as u8;
            data[i + 2] = (84.0 + 50.0 * (1.0 - t) + grid) as u8;
            data[i + 3] = 255;
        }
    }
    bg
}

fn key(name: &str, label: &str, text: Option<char>, mods: u8, pressed: bool) -> KeyEvent {
    KeyEvent { pressed, name: name.into(), label: label.into(), text, mods, is_modifier: modifier_name(name), repeat: false }
}

fn modifier_name(n: &str) -> bool {
    crate::input::modifier_bit(n).is_some()
}

/// Run `script` against a fresh engine and save the resulting frame at `t_end`.
fn shot(dir: &Path, name: &str, cfg: Config, t_end: f64, script: impl FnOnce(&mut Engine)) -> Result<()> {
    let mut e = Engine::new(cfg, Fonts::load());
    e.set_monitors(vec![IRect::new(0, 0, W as i32, H as i32)]);
    script(&mut e);
    let _ = e.begin_frame(t_end);
    let mut layer = Pixmap::new(W, H).unwrap();
    e.paint(&mut layer, IRect::new(0, 0, W as i32, H as i32), t_end);
    let mut out = background();
    out.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    let path = dir.join(format!("{name}.png"));
    out.save_png(&path).with_context(|| format!("writing {}", path.display()))?;
    println!("{}", path.display());
    Ok(())
}

/// A smooth sweep from left to right, used by the trail shots.
fn sweep(e: &mut Engine, t0: f64, n: usize) {
    for i in 0..n {
        let f = i as f32 / (n - 1) as f32;
        let x = 90.0 + f * 520.0;
        let y = 150.0 + (f * 6.0).sin() * 70.0;
        e.handle(t0 + i as f64 * 0.012, InputEvent::Pointer { x, y });
    }
}

pub fn run(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let base = || {
        let mut c = Config::default();
        c.ring.pulse_speed = 0.0;
        c
    };

    // Ring styles.
    for style in RingStyle::ALL {
        let mut c = base();
        c.ring.style = *style;
        shot(dir, &format!("ring_{}", style.id()), c, 0.0, |e| e.handle(0.0, InputEvent::Pointer { x: 380.0, y: 150.0 }))?;
    }

    // Spotlight shapes.
    for shape in SpotShape::ALL {
        let mut c = base();
        c.spotlight.enabled = true;
        c.spotlight.shape = *shape;
        c.spotlight.radius = if *shape == SpotShape::Circle { 110.0 } else { 55.0 };
        shot(dir, &format!("spotlight_{}", shape.id()), c, 0.0, |e| e.handle(0.0, InputEvent::Pointer { x: 380.0, y: 150.0 }))?;
    }

    // Trails.
    for style in TrailStyle::ALL {
        let mut c = base();
        c.trail.enabled = true;
        c.trail.style = *style;
        c.trail.lifetime_ms = 900;
        c.trail.size = if matches!(style, TrailStyle::Stars | TrailStyle::Kawaii | TrailStyle::Sparkles) { 26.0 } else { 20.0 };
        shot(dir, &format!("trail_{}", style.id()), c, 0.62, |e| sweep(e, 0.0, 50))?;
    }

    // Click effects, mid-animation.
    for style in ClickStyle::ALL {
        let mut c = base();
        c.clicks.style = *style;
        c.clicks.size = 150.0;
        c.clicks.thickness = 7.0;
        c.ring.enabled = false;
        shot(dir, &format!("click_{}", style.id()), c, 0.2, |e| {
            e.handle(0.0, InputEvent::Pointer { x: 380.0, y: 150.0 });
            e.handle(0.0, InputEvent::Button { button: Button::Left, pressed: true });
        })?;
    }

    // Shake to locate.
    {
        let c = base();
        shot(dir, "shake", c, 0.7, |e| {
            for i in 0..16 {
                let x = if i % 2 == 0 { 220.0 } else { 520.0 };
                e.handle(0.5 + i as f64 * 0.02, InputEvent::Pointer { x, y: 150.0 });
            }
        })?;
    }

    // Keystroke visualiser styles.
    for style in KeyStyle::ALL {
        let mut c = base();
        c.keystrokes.enabled = true;
        c.keystrokes.style = *style;
        c.keystrokes.size = 24.0;
        c.ring.enabled = false;
        shot(dir, &format!("keys_{}", style.id()), c, 0.5, |e| {
            e.handle(0.0, InputEvent::Pointer { x: 380.0, y: 100.0 });
            for (i, ch) in "hello".chars().enumerate() {
                e.handle(0.05 * i as f64, InputEvent::Key(key(&ch.to_string(), &ch.to_uppercase().to_string(), Some(ch), 0, true)));
            }
            e.handle(0.3, InputEvent::Key(key("t", "T", Some('t'), MOD_CTRL | MOD_SHIFT, true)));
            e.handle(0.4, InputEvent::Key(key("Return", "Enter", None, 0, true)));
        })?;
    }

    // Annotation mode with every tool.
    {
        let mut c = base();
        c.ring.enabled = false;
        shot(dir, "annotate", c, 1.0, |e| {
            e.handle(0.0, InputEvent::Pointer { x: 100.0, y: 200.0 });
            e.toggle_annotation_mode();
            let down = |e: &mut Engine, t: f64| e.handle(t, InputEvent::Button { button: Button::Left, pressed: true });
            let up = |e: &mut Engine, t: f64| e.handle(t, InputEvent::Button { button: Button::Left, pressed: false });
            // Pen scribble.
            down(e, 0.1);
            for i in 0..40 {
                let f = i as f32 / 39.0;
                e.handle(0.1 + f as f64 * 0.2, InputEvent::Pointer { x: 100.0 + f * 260.0, y: 200.0 + (f * 9.0).sin() * 30.0 });
            }
            up(e, 0.4);
            // Highlighter underline.
            e.annot.tool = Tool::Highlighter;
            e.annot.color_idx = 2;
            e.handle(0.5, InputEvent::Pointer { x: 400.0, y: 130.0 });
            down(e, 0.5);
            e.handle(0.6, InputEvent::Pointer { x: 560.0, y: 132.0 });
            e.handle(0.7, InputEvent::Pointer { x: 700.0, y: 130.0 });
            up(e, 0.7);
            // Callouts.
            e.annot.tool = Tool::Callout;
            e.annot.color_idx = 4;
            for (i, (x, y)) in [(420.0, 220.0), (500.0, 240.0), (580.0, 220.0)].iter().enumerate() {
                e.handle(0.8, InputEvent::Pointer { x: *x, y: *y });
                down(e, 0.8 + i as f64 * 0.01);
                up(e, 0.81 + i as f64 * 0.01);
            }
            // Text.
            e.annot.tool = Tool::Text;
            e.annot.color_idx = 0;
            e.handle(0.9, InputEvent::Pointer { x: 130.0, y: 270.0 });
            down(e, 0.9);
            up(e, 0.91);
            for ch in "Look here!".chars() {
                e.handle(0.92, InputEvent::Key(key(&ch.to_string(), &ch.to_string(), Some(ch), 0, true)));
            }
            e.handle(0.93, InputEvent::Key(key("Return", "Enter", None, 0, true)));
        })?;
    }

    // Focus mode.
    {
        let mut c = base();
        c.focus.enabled = true;
        c.ring.enabled = false;
        shot(dir, "focus", c, 0.0, |e| {
            e.handle(0.0, InputEvent::Pointer { x: 380.0, y: 150.0 });
            e.set_focus_rect(Some(IRect::new(180, 60, 580, 240)));
        })?;
    }
    Ok(())
}
