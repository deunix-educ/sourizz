//! Minimal text rendering on top of fontdue, with a per-size glyph cache.
//!
//! The font is discovered at runtime through fontconfig (`fc-match`) so no
//! font has to be bundled. If none is found, text silently measures as zero
//! width and draws nothing — every other effect keeps working.

use crate::config::Color;
use std::collections::HashMap;
use std::path::PathBuf;
use tiny_skia::Pixmap;

pub struct Fonts {
    font: Option<fontdue::Font>,
    cache: HashMap<(char, u32), (fontdue::Metrics, Vec<u8>)>,
}

fn find_font() -> Option<PathBuf> {
    for pattern in ["sans-serif:bold", "sans-serif", "DejaVu Sans:bold"] {
        if let Ok(out) = std::process::Command::new("fc-match").args(["-f", "%{file}", pattern]).output() {
            let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
            if out.status.success() && p.is_file() {
                return Some(p);
            }
        }
    }
    [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/noto/NotoSans-Bold.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

impl Fonts {
    pub fn load() -> Self {
        let font = find_font().and_then(|p| {
            // fontdue cannot read .ttc collections by index 0 reliably, so just try.
            let data = std::fs::read(&p).ok()?;
            match fontdue::Font::from_bytes(data, fontdue::FontSettings::default()) {
                Ok(f) => {
                    log::debug!("font: {}", p.display());
                    Some(f)
                }
                Err(e) => {
                    log::warn!("cannot parse font {}: {e}", p.display());
                    None
                }
            }
        });
        if font.is_none() {
            log::warn!("no usable font found: keystrokes, text and callout numbers will not be drawn");
        }
        Self { font, cache: HashMap::new() }
    }

    pub fn none() -> Self {
        Self { font: None, cache: HashMap::new() }
    }

    pub fn available(&self) -> bool {
        self.font.is_some()
    }

    fn glyph(&mut self, ch: char, px: f32) -> Option<&(fontdue::Metrics, Vec<u8>)> {
        let font = self.font.as_ref()?;
        let key = (ch, (px * 4.0) as u32);
        Some(self.cache.entry(key).or_insert_with(|| font.rasterize(ch, px)))
    }

    pub fn measure(&mut self, text: &str, px: f32) -> f32 {
        text.chars().map(|c| self.glyph(c, px).map_or(0.0, |g| g.0.advance_width)).sum()
    }

    /// (ascent, descent) in pixels for the given size; descent is positive.
    pub fn vmetrics(&self, px: f32) -> (f32, f32) {
        self.font.as_ref().and_then(|f| f.horizontal_line_metrics(px)).map_or((px * 0.8, px * 0.2), |m| (m.ascent, -m.descent))
    }

    /// Draw `text` with its left edge at `x` and baseline at `baseline`.
    pub fn draw(&mut self, pm: &mut Pixmap, text: &str, x: f32, baseline: f32, px: f32, color: Color, alpha: f32) {
        let (w, h) = (pm.width() as i32, pm.height() as i32);
        let base_a = color.a as f32 / 255.0 * alpha;
        if base_a <= 0.0 {
            return;
        }
        let mut pen = x;
        for ch in text.chars() {
            let Some((m, bitmap)) = self.glyph(ch, px).map(|g| (g.0, g.1.clone())) else { return };
            let gx = (pen + m.xmin as f32).round() as i32;
            let gy = (baseline - m.ymin as f32 - m.height as f32).round() as i32;
            let data = pm.data_mut();
            for row in 0..m.height as i32 {
                let py = gy + row;
                if py < 0 || py >= h {
                    continue;
                }
                for col in 0..m.width as i32 {
                    let pxl = gx + col;
                    if pxl < 0 || pxl >= w {
                        continue;
                    }
                    let cov = bitmap[(row * m.width as i32 + col) as usize];
                    if cov == 0 {
                        continue;
                    }
                    let a = cov as f32 / 255.0 * base_a;
                    let i = ((py * w + pxl) * 4) as usize;
                    let inv = 1.0 - a;
                    // Pixmap data is premultiplied RGBA.
                    data[i] = (color.r as f32 * a + data[i] as f32 * inv).round().min(255.0) as u8;
                    data[i + 1] = (color.g as f32 * a + data[i + 1] as f32 * inv).round().min(255.0) as u8;
                    data[i + 2] = (color.b as f32 * a + data[i + 2] as f32 * inv).round().min(255.0) as u8;
                    data[i + 3] = (255.0 * a + data[i + 3] as f32 * inv).round().min(255.0) as u8;
                }
            }
            pen += m.advance_width;
        }
    }
}
