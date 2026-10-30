//! Persistent configuration (`~/.config/sourizz/config.toml`).
//!
//! Every field has a default, so a partial or empty file is valid. The daemon
//! watches the file's modification time and reloads it live, which is how the
//! settings window and hand edits take effect without a restart.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::PathBuf;
use std::time::SystemTime;

// ---------------------------------------------------------------- Color

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Accepts `#RGB`, `#RRGGBB` and `#RRGGBBAA` (the `#` is optional).
    pub fn parse(s: &str) -> Option<Self> {
        let h = s.trim().trim_start_matches('#');
        let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        match h.len() {
            3 => {
                let n = |i: usize| u8::from_str_radix(h.get(i..i + 1)?, 16).ok().map(|v| v * 17);
                Some(Self::rgb(n(0)?, n(1)?, n(2)?))
            }
            6 => Some(Self::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Self { r: byte(0)?, g: byte(2)?, b: byte(4)?, a: byte(6)? }),
            _ => None,
        }
    }

    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }

    /// Convert to a tiny-skia colour, multiplying the alpha by `alpha`.
    pub fn sk(self, alpha: f32) -> tiny_skia::Color {
        let a = (self.a as f32 / 255.0 * alpha).clamp(0.0, 1.0);
        tiny_skia::Color::from_rgba(self.r as f32 / 255.0, self.g as f32 / 255.0, self.b as f32 / 255.0, a)
            .unwrap_or(tiny_skia::Color::TRANSPARENT)
    }

    pub fn as_f32(self) -> [f32; 3] {
        [self.r as f32 / 255.0, self.g as f32 / 255.0, self.b as f32 / 255.0]
    }

    pub fn from_f32(c: [f32; 3]) -> Self {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self::rgb(q(c[0]), q(c[1]), q(c[2]))
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Color::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid colour {s:?}")))
    }
}

// ---------------------------------------------------------------- Enums

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename = $s)] $variant),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            pub fn id(self) -> &'static str { match self { $($name::$variant => $s),+ } }
        }
    };
}

string_enum!(BackendKind { Auto => "auto", X11 => "x11", Wayland => "wayland" });
string_enum!(SpotShape { Circle => "circle", HBar => "hbar", VBar => "vbar" });
string_enum!(RingStyle { Classic => "classic", Neon => "neon" });
string_enum!(TrailStyle {
    Comet => "comet", Dots => "dots", Ribbon => "ribbon", Rainbow => "rainbow",
    Stars => "stars", Kawaii => "kawaii", Sparkles => "sparkles",
});
string_enum!(ClickStyle {
    SingleRing => "single_ring", RippleWave => "ripple_wave", Burst => "burst", Pulse => "pulse",
});
string_enum!(KeyStyle {
    Classic => "classic", Glass => "glass", Keycaps => "keycaps",
    Neon => "neon", Gradient => "gradient", Minimal => "minimal",
});
string_enum!(KeyPosition {
    BottomCenter => "bottom_center", BottomLeft => "bottom_left", BottomRight => "bottom_right",
    TopCenter => "top_center",
});

// ---------------------------------------------------------------- Sections

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// Master switch: when false nothing is drawn.
    pub enabled: bool,
    pub backend: BackendKind,
    /// `auto`, `en` or `fr`.
    pub language: String,
    /// Maximum animation frame rate.
    pub fps: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Spotlight {
    pub enabled: bool,
    pub shape: SpotShape,
    /// Radius of the lit circle, or half-thickness of the lit bar, in pixels.
    pub radius: f32,
    /// How dark the surroundings are, 0..1.
    pub opacity: f32,
    pub color: Color,
    /// Edge softness, 0 (sharp) .. 1 (very soft).
    pub softness: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ring {
    pub enabled: bool,
    pub style: RingStyle,
    pub size: f32,
    pub thickness: f32,
    pub color: Color,
    pub opacity: f32,
    /// Translucent disc inside the ring, 0..1.
    pub fill: f32,
    /// Pulse animation speed, 0 = static.
    pub pulse_speed: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Trail {
    pub enabled: bool,
    pub style: TrailStyle,
    pub lifetime_ms: u32,
    pub size: f32,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clicks {
    pub enabled: bool,
    /// Show the effect on real mouse clicks (otherwise only on demand, see `tap_key`).
    pub show_on_clicks: bool,
    pub style: ClickStyle,
    pub size: f32,
    pub thickness: f32,
    pub duration_ms: u32,
    pub left_color: Color,
    pub middle_color: Color,
    pub right_color: Color,
    /// Key whose *tap* triggers the effect at the pointer, e.g. `Control_L`. Empty = off.
    pub tap_key: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sound {
    pub enabled: bool,
    pub volume: f32,
    /// Optional custom WAV file. Empty = built-in click.
    pub file: String,
    /// Also play on button release.
    pub on_release: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shake {
    pub enabled: bool,
    /// 0..1, higher triggers more easily.
    pub sensitivity: f32,
    /// How much the pointer marker grows while locating.
    pub scale: f32,
    pub duration_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Annotate {
    pub enabled: bool,
    /// Hold this key and move the pointer to draw. Empty = off.
    pub hold_key: String,
    pub color: Color,
    pub width: f32,
    /// Strokes drawn with the hold key fade away after release.
    pub fade: bool,
    pub fade_ms: u32,
    pub highlighter_color: Color,
    pub highlighter_width: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keystrokes {
    pub enabled: bool,
    pub style: KeyStyle,
    pub position: KeyPosition,
    pub size: f32,
    pub duration_ms: u32,
    /// Only show combinations that include Ctrl/Alt/Super (never plain typing).
    pub only_shortcuts: bool,
    pub accent: Color,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Focus {
    pub enabled: bool,
    pub opacity: f32,
    pub color: Color,
}

/// Global hotkeys written as `Ctrl+Alt+Shift+S`. An empty string disables one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub toggle_all: String,
    pub spotlight: String,
    pub ring: String,
    pub trail: String,
    pub clicks: String,
    pub keystrokes: String,
    pub focus: String,
    pub annotate_mode: String,
    pub clear_annotations: String,
    pub show_click: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub spotlight: Spotlight,
    pub ring: Ring,
    pub trail: Trail,
    pub clicks: Clicks,
    pub sound: Sound,
    pub shake: Shake,
    pub annotate: Annotate,
    pub keystrokes: Keystrokes,
    pub focus: Focus,
    pub hotkeys: Hotkeys,
}

impl Default for General {
    fn default() -> Self {
        Self { enabled: true, backend: BackendKind::Auto, language: "auto".into(), fps: 60 }
    }
}
impl Default for Spotlight {
    fn default() -> Self {
        Self { enabled: false, shape: SpotShape::Circle, radius: 170.0, opacity: 0.65, color: Color::rgb(0, 0, 0), softness: 0.25 }
    }
}
impl Default for Ring {
    fn default() -> Self {
        Self {
            enabled: true,
            style: RingStyle::Classic,
            size: 56.0,
            thickness: 4.0,
            color: Color::rgb(0xFF, 0xC1, 0x07),
            opacity: 0.95,
            fill: 0.0,
            pulse_speed: 1.0,
        }
    }
}
impl Default for Trail {
    fn default() -> Self {
        Self { enabled: false, style: TrailStyle::Comet, lifetime_ms: 650, size: 16.0, color: Color::rgb(0x3D, 0xA5, 0xFF) }
    }
}
impl Default for Clicks {
    fn default() -> Self {
        Self {
            enabled: true,
            show_on_clicks: true,
            style: ClickStyle::SingleRing,
            size: 72.0,
            thickness: 5.0,
            duration_ms: 600,
            left_color: Color::rgb(0x3B, 0x82, 0xF6),
            middle_color: Color::rgb(0x22, 0xC5, 0x5E),
            right_color: Color::rgb(0xF4, 0x3F, 0x5E),
            tap_key: String::new(),
        }
    }
}
impl Default for Sound {
    fn default() -> Self {
        Self { enabled: false, volume: 0.5, file: String::new(), on_release: false }
    }
}
impl Default for Shake {
    fn default() -> Self {
        Self { enabled: true, sensitivity: 0.6, scale: 3.2, duration_ms: 1400 }
    }
}
impl Default for Annotate {
    fn default() -> Self {
        Self {
            enabled: true,
            hold_key: "Control_R".into(),
            color: Color::rgb(0xFF, 0x3B, 0x30),
            width: 5.0,
            fade: true,
            fade_ms: 1800,
            highlighter_color: Color::rgb(0xFF, 0xEB, 0x3B),
            highlighter_width: 24.0,
        }
    }
}
impl Default for Keystrokes {
    fn default() -> Self {
        Self {
            enabled: false,
            style: KeyStyle::Classic,
            position: KeyPosition::BottomCenter,
            size: 28.0,
            duration_ms: 1800,
            only_shortcuts: false,
            accent: Color::rgb(0x3B, 0x82, 0xF6),
        }
    }
}
impl Default for Focus {
    fn default() -> Self {
        Self { enabled: false, opacity: 0.6, color: Color::rgb(0, 0, 0) }
    }
}
impl Default for Hotkeys {
    fn default() -> Self {
        let k = |c: &str| format!("Ctrl+Alt+Shift+{c}");
        Self {
            toggle_all: k("M"),
            spotlight: k("S"),
            // Not "R": GNOME Shell uses Ctrl+Alt+Shift+R for its screen-recording interface.
            ring: k("O"),
            trail: k("T"),
            clicks: k("C"),
            keystrokes: k("K"),
            focus: k("F"),
            annotate_mode: k("A"),
            clear_annotations: k("X"),
            show_click: String::new(),
        }
    }
}

// ---------------------------------------------------------------- Load / save

impl Config {
    /// Clamp values to sane ranges so a hand-edited file can never break rendering.
    pub fn sanitize(&mut self) {
        let c = |v: &mut f32, lo: f32, hi: f32| *v = if v.is_finite() { v.clamp(lo, hi) } else { lo };
        c(&mut self.spotlight.radius, 8.0, 4000.0);
        c(&mut self.spotlight.opacity, 0.0, 1.0);
        c(&mut self.spotlight.softness, 0.0, 1.0);
        c(&mut self.ring.size, 8.0, 800.0);
        c(&mut self.ring.thickness, 0.5, 60.0);
        c(&mut self.ring.opacity, 0.0, 1.0);
        c(&mut self.ring.fill, 0.0, 1.0);
        c(&mut self.ring.pulse_speed, 0.0, 10.0);
        c(&mut self.trail.size, 2.0, 120.0);
        c(&mut self.clicks.size, 8.0, 800.0);
        c(&mut self.clicks.thickness, 0.5, 60.0);
        c(&mut self.sound.volume, 0.0, 1.0);
        c(&mut self.shake.sensitivity, 0.0, 1.0);
        c(&mut self.shake.scale, 1.0, 10.0);
        c(&mut self.annotate.width, 1.0, 80.0);
        c(&mut self.annotate.highlighter_width, 2.0, 200.0);
        c(&mut self.keystrokes.size, 10.0, 120.0);
        c(&mut self.focus.opacity, 0.0, 1.0);
        self.trail.lifetime_ms = self.trail.lifetime_ms.clamp(100, 5000);
        self.clicks.duration_ms = self.clicks.duration_ms.clamp(100, 5000);
        self.shake.duration_ms = self.shake.duration_ms.clamp(300, 8000);
        self.annotate.fade_ms = self.annotate.fade_ms.clamp(200, 30000);
        self.keystrokes.duration_ms = self.keystrokes.duration_ms.clamp(300, 15000);
        self.general.fps = self.general.fps.clamp(15, 240);
    }

    pub fn dir() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("sourizz")
    }

    pub fn path() -> PathBuf {
        Self::dir().join("config.toml")
    }

    /// Load the config; a missing file yields defaults, a broken one is reported and ignored.
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &std::path::Path) -> Self {
        let mut cfg = match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("{}: {e} — using defaults", path.display());
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        };
        cfg.sanitize();
        cfg
    }

    /// Write atomically (temp file + rename) so the daemon never reads a half-written file.
    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn mtime() -> Option<SystemTime> {
        std::fs::metadata(Self::path()).and_then(|m| m.modified()).ok()
    }
}

// ---------------------------------------------------------------- Hotkey chords

pub const MOD_CTRL: u8 = 1;
pub const MOD_SHIFT: u8 = 2;
pub const MOD_ALT: u8 = 4;
pub const MOD_SUPER: u8 = 8;

/// A parsed hotkey: modifier mask plus a key name (`S`, `F5`, `space`, …).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub mods: u8,
    pub key: String,
}

impl Chord {
    pub fn parse(s: &str) -> Option<Chord> {
        let mut mods = 0u8;
        let mut key = None;
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "ctl" => mods |= MOD_CTRL,
                "shift" => mods |= MOD_SHIFT,
                "alt" | "meta" => mods |= MOD_ALT,
                "super" | "win" | "windows" | "logo" | "cmd" => mods |= MOD_SUPER,
                _ => {
                    if key.replace(part.to_string()).is_some() {
                        return None;
                    }
                }
            }
        }
        key.map(|key| Chord { mods, key })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_roundtrip() {
        assert_eq!(Color::parse("#fc0"), Some(Color::rgb(0xff, 0xcc, 0)));
        assert_eq!(Color::parse("FFC107"), Some(Color::rgb(0xFF, 0xC1, 0x07)));
        let c = Color { r: 1, g: 2, b: 3, a: 4 };
        assert_eq!(Color::parse(&c.to_hex()), Some(c));
        assert_eq!(Color::parse("#12345"), None);
        assert_eq!(Color::parse("zzzzzz"), None);
    }

    #[test]
    fn empty_and_partial_files_are_valid() {
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c, Config::default());
        let c: Config = toml::from_str("[ring]\nsize = 90.0\n").unwrap();
        assert_eq!(c.ring.size, 90.0);
        assert_eq!(c.ring.thickness, Ring::default().thickness);
    }

    #[test]
    fn default_roundtrips_through_toml() {
        let c = Config::default();
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn sanitize_clamps_garbage() {
        let mut c = Config::default();
        c.ring.size = f32::NAN;
        c.spotlight.opacity = 7.0;
        c.general.fps = 0;
        c.sanitize();
        assert!(c.ring.size >= 8.0);
        assert_eq!(c.spotlight.opacity, 1.0);
        assert_eq!(c.general.fps, 15);
    }

    #[test]
    fn chords() {
        let c = Chord::parse("Ctrl+Alt+Shift+S").unwrap();
        assert_eq!(c.mods, MOD_CTRL | MOD_ALT | MOD_SHIFT);
        assert_eq!(c.key, "S");
        assert!(Chord::parse("").is_none());
        assert!(Chord::parse("Ctrl+").is_none());
        assert!(Chord::parse("A+B").is_none());
    }
}
