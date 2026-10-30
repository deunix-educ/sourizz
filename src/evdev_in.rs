//! Global pointer and keyboard events on Wayland, read straight from `/dev/input/event*`.
//!
//! Wayland deliberately hides input that happens in other applications, so this is the one
//! compositor-independent way to see clicks and keystrokes. It needs read access to the
//! input devices, i.e. membership of the `input` group (a real security trade-off: any
//! program running as that user could then read keystrokes, which is why this is opt-in
//! and documented in the README).
//!
//! Devices report *relative* motion, never an absolute position; see `wlsrc` for how the
//! pointer position is obtained.

use crate::input::Button;
use rustix::fd::{AsFd, BorrowedFd};
use std::collections::HashSet;
use std::path::PathBuf;

pub const EV_KEY: u16 = 1;
pub const EV_REL: u16 = 2;
pub const EV_ABS: u16 = 3;
pub const REL_X: u16 = 0;
pub const REL_Y: u16 = 1;
pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;

#[derive(Clone, Debug, PartialEq)]
pub enum Raw {
    /// Relative motion in device units.
    Motion(f32, f32),
    /// The pointer moved but this device (a touchpad) cannot say by how much.
    MotionHint,
    Button(Button, bool),
    /// Keyboard key by evdev code (add 8 for the X/xkb keycode).
    Key {
        code: u32,
        pressed: bool,
        repeat: bool,
    },
}

/// Map a Linux button code to ours.
pub fn button_from_code(code: u16) -> Option<Button> {
    match code {
        BTN_LEFT => Some(Button::Left),
        BTN_RIGHT => Some(Button::Right),
        BTN_MIDDLE => Some(Button::Middle),
        0x113..=0x117 => Some(Button::Other),
        _ => None,
    }
}

/// Turn a batch of raw `(type, code, value)` triples into our events, in order, merging
/// consecutive relative motion into one event.
pub fn translate(events: impl IntoIterator<Item = (u16, u16, i32)>) -> Vec<Raw> {
    let mut out = Vec::new();
    let (mut dx, mut dy) = (0f32, 0f32);
    let mut hint = false;
    let flush = |out: &mut Vec<Raw>, dx: &mut f32, dy: &mut f32, hint: &mut bool| {
        if *dx != 0.0 || *dy != 0.0 {
            out.push(Raw::Motion(*dx, *dy));
        } else if *hint {
            out.push(Raw::MotionHint);
        }
        (*dx, *dy, *hint) = (0.0, 0.0, false);
    };
    for (ty, code, value) in events {
        match ty {
            EV_REL if code == REL_X => dx += value as f32,
            EV_REL if code == REL_Y => dy += value as f32,
            EV_ABS if code <= 1 => hint = true,
            EV_KEY => {
                flush(&mut out, &mut dx, &mut dy, &mut hint);
                if let Some(b) = button_from_code(code) {
                    if value != 2 {
                        out.push(Raw::Button(b, value == 1));
                    }
                } else if code < 0x100 {
                    out.push(Raw::Key { code: code as u32, pressed: value != 0, repeat: value == 2 });
                }
            }
            _ => {}
        }
    }
    flush(&mut out, &mut dx, &mut dy, &mut hint);
    out
}

pub struct Devices {
    devs: Vec<(PathBuf, evdev::Device)>,
    /// Nodes already opened and found uninteresting (power button, audio jacks…). Opening a node
    /// costs tens of milliseconds on some machines, so they must not be reopened on every rescan.
    skipped: HashSet<PathBuf>,
    /// Devices that exist but could not be opened (permission denied).
    pub denied: usize,
}

impl Devices {
    pub fn scan() -> Devices {
        let mut d = Devices { devs: Vec::new(), skipped: HashSet::new(), denied: 0 };
        d.rescan();
        d
    }

    fn interesting(dev: &evdev::Device) -> bool {
        use evdev::{KeyCode, RelativeAxisCode};
        let keys = dev.supported_keys();
        let pointer = keys.is_some_and(|k| k.contains(KeyCode::BTN_LEFT))
            && (dev.supported_relative_axes().is_some_and(|a| a.contains(RelativeAxisCode::REL_X))
                || dev.supported_absolute_axes().is_some());
        let keyboard = keys.is_some_and(|k| k.contains(KeyCode::KEY_A) && k.contains(KeyCode::KEY_SPACE));
        pointer || keyboard
    }

    /// Open any readable input device we do not have yet (cheap enough to call every few seconds).
    pub fn rescan(&mut self) {
        let Ok(dir) = std::fs::read_dir("/dev/input") else { return };
        // A node that went away may come back under the same number as another device.
        self.skipped.retain(|p| p.exists());
        let mut denied = 0;
        for entry in dir.flatten() {
            let path = entry.path();
            let is_event = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event"));
            if !is_event || self.skipped.contains(&path) || self.devs.iter().any(|(p, _)| *p == path) {
                continue;
            }
            match evdev::Device::open(&path) {
                Ok(dev) if Self::interesting(&dev) => {
                    if dev.set_nonblocking(true).is_ok() {
                        log::debug!("input device {} ({})", path.display(), dev.name().unwrap_or("?"));
                        self.devs.push((path, dev));
                    }
                }
                Ok(_) => {
                    self.skipped.insert(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => denied += 1,
                Err(_) => {}
            }
        }
        if self.devs.is_empty() {
            self.denied = denied;
        }
    }

    pub fn len(&self) -> usize {
        self.devs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.devs.is_empty()
    }

    pub fn fds(&self) -> Vec<BorrowedFd<'_>> {
        self.devs.iter().map(|(_, d)| d.as_fd()).collect()
    }

    /// Drain device `i`. A device that disappeared (unplugged) is dropped.
    pub fn read(&mut self, i: usize) -> Vec<Raw> {
        let Some((path, dev)) = self.devs.get_mut(i) else { return Vec::new() };
        let result = dev.fetch_events().map(|it| it.map(|e| (e.event_type().0, e.code(), e.value())).collect::<Vec<_>>());
        match result {
            Ok(events) => translate(events),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Vec::new(),
            Err(e) => {
                log::debug!("dropping input device {}: {e}", path.display());
                self.devs.remove(i);
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_motion_is_summed_and_ordered_with_buttons() {
        let evs =
            [(EV_REL, REL_X, 3), (EV_REL, REL_Y, -2), (EV_REL, REL_X, 4), (EV_KEY, BTN_LEFT, 1), (EV_REL, REL_X, 1), (EV_KEY, BTN_LEFT, 0)];
        assert_eq!(
            translate(evs),
            vec![Raw::Motion(7.0, -2.0), Raw::Button(Button::Left, true), Raw::Motion(1.0, 0.0), Raw::Button(Button::Left, false),]
        );
    }

    #[test]
    fn keys_and_repeats() {
        let evs = [(EV_KEY, 30, 1), (EV_KEY, 30, 2), (EV_KEY, 30, 0), (EV_KEY, BTN_RIGHT, 1), (EV_KEY, BTN_RIGHT, 2)];
        assert_eq!(
            translate(evs),
            vec![
                Raw::Key { code: 30, pressed: true, repeat: false },
                Raw::Key { code: 30, pressed: true, repeat: true },
                Raw::Key { code: 30, pressed: false, repeat: false },
                Raw::Button(Button::Right, true),
            ],
            "button auto-repeat (value 2) is ignored, key repeat is flagged"
        );
    }

    #[test]
    fn touchpad_motion_is_only_a_hint() {
        assert_eq!(translate([(EV_ABS, 0, 1200), (EV_ABS, 1, 800)]), vec![Raw::MotionHint]);
        assert_eq!(translate([(EV_ABS, 0, 1200), (EV_REL, REL_X, 2)]), vec![Raw::Motion(2.0, 0.0)]);
    }

    #[test]
    fn unrelated_events_are_dropped() {
        assert!(translate([(0, 0, 0), (EV_REL, 8, 1), (EV_KEY, 0x14a, 1)]).is_empty());
    }

    #[test]
    fn side_buttons_map_to_other() {
        assert_eq!(button_from_code(0x113), Some(Button::Other));
        assert_eq!(button_from_code(0x118), None);
    }
}
