//! Commands that steer the running daemon, from the tray, the CLI socket or hotkeys.

use crate::config::Config;
use rustix::fd::{AsFd, OwnedFd};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

pub enum Command {
    Toggle(String),
    Set(String, bool),
    /// Generic `section.key value` setter (used by the tray's radio items and `sourizz set`).
    SetOption(String, String),
    ShowClick,
    AnnotateToggle,
    AnnotateClear,
    Reload,
    /// Reply with a one-line status.
    Status(Sender<String>),
    Settings,
    Quit,
}

/// Names accepted by `toggle`, `on` and `off`.
pub const EFFECTS: &[&str] = &["all", "spotlight", "ring", "trail", "clicks", "sound", "shake", "annotate", "keystrokes", "focus"];

/// Cloneable handle other threads use to talk to the backend loop.
#[derive(Clone)]
pub struct Hub {
    tx: Sender<Command>,
    wake: Arc<OwnedFd>,
}

impl Hub {
    /// Returns the hub, the command receiver and the read end of the wake pipe the
    /// backend must poll (it becomes readable whenever a command is queued).
    pub fn new() -> std::io::Result<(Hub, Receiver<Command>, OwnedFd)> {
        let (tx, rx) = std::sync::mpsc::channel();
        let (r, w) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC | rustix::pipe::PipeFlags::NONBLOCK)?;
        Ok((Hub { tx, wake: Arc::new(w) }, rx, r))
    }

    pub fn send(&self, c: Command) {
        if self.tx.send(c).is_ok() {
            self.poke();
        }
    }

    /// Wake the backend loop without sending a command.
    pub fn poke(&self) {
        let _ = rustix::io::write(self.wake.as_fd(), &[1u8]);
    }

    pub fn wake_fd(&self) -> Arc<OwnedFd> {
        self.wake.clone()
    }
}

/// Drain the wake pipe so poll() stops reporting it as readable.
pub fn drain(fd: &OwnedFd) {
    let mut buf = [0u8; 64];
    while matches!(rustix::io::read(fd, &mut buf), Ok(n) if n > 0) {}
}

pub fn flag<'a>(cfg: &'a mut Config, name: &str) -> Option<&'a mut bool> {
    Some(match name {
        "all" | "enabled" => &mut cfg.general.enabled,
        "spotlight" => &mut cfg.spotlight.enabled,
        "ring" => &mut cfg.ring.enabled,
        "trail" => &mut cfg.trail.enabled,
        "clicks" | "click" => &mut cfg.clicks.enabled,
        "sound" => &mut cfg.sound.enabled,
        "shake" => &mut cfg.shake.enabled,
        "annotate" | "annotations" => &mut cfg.annotate.enabled,
        "keystrokes" | "keys" => &mut cfg.keystrokes.enabled,
        "focus" => &mut cfg.focus.enabled,
        _ => return None,
    })
}

/// Set `section.key` to `value` (parsed as bool, integer, float, then string).
pub fn set_option(cfg: &Config, key: &str, value: &str) -> Result<Config, String> {
    let mut doc = toml::Value::try_from(cfg).map_err(|e| e.to_string())?;
    let (section, field) = key.split_once('.').ok_or_else(|| format!("expected section.key, got {key:?}"))?;
    let table = doc.get_mut(section).and_then(|v| v.as_table_mut()).ok_or_else(|| format!("unknown section {section:?}"))?;
    let old = table.get(field).ok_or_else(|| format!("unknown option {key:?}"))?;
    let new = match old {
        toml::Value::Boolean(_) => toml::Value::Boolean(matches!(value, "true" | "1" | "on" | "yes")),
        toml::Value::Integer(_) => toml::Value::Integer(value.parse().map_err(|_| format!("{value:?} is not an integer"))?),
        toml::Value::Float(_) => toml::Value::Float(value.parse().map_err(|_| format!("{value:?} is not a number"))?),
        _ => toml::Value::String(value.to_string()),
    };
    table.insert(field.to_string(), new);
    let mut out: Config = doc.try_into().map_err(|e: toml::de::Error| format!("invalid value for {key}: {e}"))?;
    out.sanitize();
    Ok(out)
}

/// Parse one line of the control protocol.
pub fn parse_line(line: &str) -> Result<Command, String> {
    let mut it = line.split_whitespace();
    let verb = it.next().ok_or("empty command")?;
    let arg = it.next();
    let known = |a: Option<&str>| -> Result<String, String> {
        let a = a.ok_or("missing effect name")?;
        if EFFECTS.contains(&a) || matches!(a, "enabled" | "click" | "annotations" | "keys") {
            Ok(a.to_string())
        } else {
            Err(format!("unknown effect {a:?} (try: {})", EFFECTS.join(", ")))
        }
    };
    Ok(match verb {
        "toggle" => Command::Toggle(known(arg)?),
        "on" => Command::Set(known(arg)?, true),
        "off" => Command::Set(known(arg)?, false),
        "set" => {
            let key = arg.ok_or("usage: set <section.key> <value>")?;
            let value: Vec<&str> = it.collect();
            if value.is_empty() {
                return Err("usage: set <section.key> <value>".into());
            }
            Command::SetOption(key.to_string(), value.join(" "))
        }
        "show-click" => Command::ShowClick,
        "annotate" => match arg {
            None | Some("toggle") => Command::AnnotateToggle,
            Some("clear") => Command::AnnotateClear,
            Some(o) => return Err(format!("unknown annotate action {o:?} (toggle, clear)")),
        },
        "reload" => Command::Reload,
        "settings" => Command::Settings,
        "quit" => Command::Quit,
        "status" => return Err("status needs a reply channel".into()),
        other => return Err(format!("unknown command {other:?}")),
    })
}

/// The configured global hotkeys as `(action name, parsed chord)`; invalid or empty ones are skipped.
pub fn hotkey_table(h: &crate::config::Hotkeys) -> Vec<(&'static str, crate::config::Chord)> {
    [
        ("toggle_all", &h.toggle_all),
        ("spotlight", &h.spotlight),
        ("ring", &h.ring),
        ("trail", &h.trail),
        ("clicks", &h.clicks),
        ("keystrokes", &h.keystrokes),
        ("focus", &h.focus),
        ("annotate_mode", &h.annotate_mode),
        ("clear_annotations", &h.clear_annotations),
        ("show_click", &h.show_click),
    ]
    .into_iter()
    .filter_map(|(name, s)| match crate::config::Chord::parse(s) {
        Some(c) => Some((name, c)),
        None => {
            if !s.trim().is_empty() {
                log::warn!("hotkeys.{name}: cannot parse {s:?}");
            }
            None
        }
    })
    .collect()
}

pub fn hotkey_command(name: &str) -> Option<Command> {
    Some(match name {
        "toggle_all" => Command::Toggle("all".into()),
        "annotate_mode" => Command::AnnotateToggle,
        "clear_annotations" => Command::AnnotateClear,
        "show_click" => Command::ShowClick,
        n @ ("spotlight" | "ring" | "trail" | "clicks" | "keystrokes" | "focus") => Command::Toggle(n.into()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        assert!(matches!(parse_line("toggle ring"), Ok(Command::Toggle(n)) if n == "ring"));
        assert!(matches!(parse_line("off spotlight"), Ok(Command::Set(n, false)) if n == "spotlight"));
        assert!(matches!(parse_line("annotate clear"), Ok(Command::AnnotateClear)));
        assert!(matches!(parse_line("annotate"), Ok(Command::AnnotateToggle)));
        assert!(matches!(parse_line("set ring.size 80"), Ok(Command::SetOption(k, v)) if k == "ring.size" && v == "80"));
        assert!(parse_line("toggle nonsense").is_err());
        assert!(parse_line("").is_err());
        assert!(parse_line("frobnicate").is_err());
    }

    #[test]
    fn set_option_types_and_errors() {
        let c = Config::default();
        let c2 = set_option(&c, "ring.size", "90").unwrap();
        assert_eq!(c2.ring.size, 90.0);
        let c3 = set_option(&c, "trail.style", "stars").unwrap();
        assert_eq!(c3.trail.style, crate::config::TrailStyle::Stars);
        let c4 = set_option(&c, "ring.color", "#00ff00").unwrap();
        assert_eq!(c4.ring.color, crate::config::Color::rgb(0, 255, 0));
        let c5 = set_option(&c, "spotlight.enabled", "on").unwrap();
        assert!(c5.spotlight.enabled);
        assert!(set_option(&c, "trail.style", "nope").is_err());
        assert!(set_option(&c, "ring.size", "abc").is_err());
        assert!(set_option(&c, "ring.bogus", "1").is_err());
        assert!(set_option(&c, "bogus.size", "1").is_err());
        // Out-of-range values are clamped rather than rejected.
        assert_eq!(set_option(&c, "ring.size", "99999").unwrap().ring.size, 800.0);
    }

    #[test]
    fn flags_cover_all_effects() {
        let mut c = Config::default();
        for e in EFFECTS {
            assert!(flag(&mut c, e).is_some(), "{e}");
        }
        assert!(flag(&mut c, "nope").is_none());
    }
}
