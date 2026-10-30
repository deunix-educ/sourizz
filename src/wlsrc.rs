//! Where the Wayland backend gets the pointer's absolute position from.
//!
//! Wayland gives a client no way to ask "where is the pointer?" unless the pointer is over
//! one of its own surfaces. Our overlay must stay click-through, so it never is. The options:
//!
//! * **Hyprland**: ask its IPC socket (exact).
//! * **KDE Plasma** (KWin): a small KWin script feeds us `workspace.cursorPos` (exact).
//! * **Anything else**: integrate the relative motion read from evdev (approximate: the
//!   compositor applies pointer acceleration we cannot see, so it drifts; it is clamped to
//!   the screen and re-synchronised whenever the pointer touches an annotation surface).

use crate::geom::IRect;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

/// Parse the reply of Hyprland's `j/cursorpos` (`{"x": 123, "y": 456}`, numbers may be floats).
pub fn parse_cursorpos(json: &str) -> Option<(f32, f32)> {
    let num = |key: &str| -> Option<f32> {
        let i = json.find(&format!("\"{key}\""))? + key.len() + 2;
        let rest = json[i..].trim_start().strip_prefix(':')?.trim_start();
        let end =
            rest.find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '.' || c == 'e' || c == 'E' || c == '+')).unwrap_or(rest.len());
        rest[..end].parse().ok()
    };
    Some((num("x")?, num("y")?))
}

pub struct Hyprland {
    path: PathBuf,
}

impl Hyprland {
    pub fn detect() -> Option<Hyprland> {
        let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
        let rt = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
        let path = [rt.join("hypr").join(&sig).join(".socket.sock"), PathBuf::from("/tmp/hypr").join(&sig).join(".socket.sock")]
            .into_iter()
            .find(|p| p.exists())?;
        Some(Hyprland { path })
    }

    pub fn cursor_pos(&self) -> Option<(f32, f32)> {
        let mut s = UnixStream::connect(&self.path).ok()?;
        s.set_read_timeout(Some(Duration::from_millis(60))).ok()?;
        s.set_write_timeout(Some(Duration::from_millis(60))).ok()?;
        s.write_all(b"j/cursorpos").ok()?;
        let mut reply = String::new();
        s.read_to_string(&mut reply).ok()?;
        parse_cursorpos(&reply)
    }
}

/// Dead-reckoning pointer position from relative evdev motion.
pub struct Estimate {
    pos: (f32, f32),
    bounds: IRect,
    /// Multiplier from device counts to logical pixels.
    pub gain: f32,
}

impl Estimate {
    pub fn new(bounds: IRect) -> Self {
        Self { pos: ((bounds.x0 + bounds.x1) as f32 / 2.0, (bounds.y0 + bounds.y1) as f32 / 2.0), bounds, gain: 1.0 }
    }

    pub fn set_bounds(&mut self, b: IRect) {
        self.bounds = b;
        self.pos = self.clamp(self.pos);
    }

    fn clamp(&self, p: (f32, f32)) -> (f32, f32) {
        (
            p.0.clamp(self.bounds.x0 as f32, (self.bounds.x1 - 1).max(self.bounds.x0) as f32),
            p.1.clamp(self.bounds.y0 as f32, (self.bounds.y1 - 1).max(self.bounds.y0) as f32),
        )
    }

    pub fn motion(&mut self, dx: f32, dy: f32) -> (f32, f32) {
        self.pos = self.clamp((self.pos.0 + dx * self.gain, self.pos.1 + dy * self.gain));
        self.pos
    }

    /// Replace the estimate with a position known to be right.
    pub fn sync(&mut self, p: (f32, f32)) {
        self.pos = self.clamp(p);
    }

    pub fn pos(&self) -> (f32, f32) {
        self.pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hyprland_replies() {
        assert_eq!(parse_cursorpos("{\"x\": 123, \"y\": 456}"), Some((123.0, 456.0)));
        assert_eq!(parse_cursorpos("{\"x\":12.5,\"y\":-3.25}"), Some((12.5, -3.25)));
        assert_eq!(parse_cursorpos("{\n\t\"x\": 1920,\n\t\"y\": 0\n}"), Some((1920.0, 0.0)));
        assert_eq!(parse_cursorpos("error"), None);
        assert_eq!(parse_cursorpos("{\"x\": 1}"), None);
    }

    #[test]
    fn estimate_integrates_clamps_and_syncs() {
        let mut e = Estimate::new(IRect::new(0, 0, 100, 50));
        assert_eq!(e.pos(), (50.0, 25.0));
        assert_eq!(e.motion(10.0, -5.0), (60.0, 20.0));
        assert_eq!(e.motion(1000.0, -1000.0), (99.0, 0.0), "clamped to the last pixel");
        e.sync((10.0, 10.0));
        assert_eq!(e.pos(), (10.0, 10.0));
        e.gain = 2.0;
        assert_eq!(e.motion(3.0, 0.0), (16.0, 10.0));
        e.set_bounds(IRect::new(0, 0, 8, 8));
        assert_eq!(e.pos(), (7.0, 7.0));
    }
}
