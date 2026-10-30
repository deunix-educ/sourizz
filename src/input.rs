//! Backend-neutral input events fed to the effect engine.

use crate::config::{MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    Other,
}

#[derive(Clone, Debug)]
pub struct KeyEvent {
    pub pressed: bool,
    /// Key name in X keysym style (`Control_R`, `a`, `F5`, `Return`). Used to match config keys.
    pub name: String,
    /// Human-friendly label (`Ctrl`, `A`, `Enter`).
    pub label: String,
    /// Character the key produces right now, if it is printable.
    pub text: Option<char>,
    /// Modifier mask (`MOD_*`) held when the event happened, *excluding* this key itself.
    pub mods: u8,
    pub is_modifier: bool,
    pub repeat: bool,
}

#[derive(Clone, Debug)]
pub enum InputEvent {
    /// Pointer position in global (virtual desktop) pixels.
    Pointer {
        x: f32,
        y: f32,
    },
    Button {
        button: Button,
        pressed: bool,
    },
    Key(KeyEvent),
}

/// What the engine asks the backend to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Play the click sound for this button (`true` = press, `false` = release).
    PlayClick(Button, bool),
    /// Enter or leave annotation capture mode (the overlay must take pointer + keyboard).
    Capture(bool),
}

/// Modifier bit for a keysym name, if it is a modifier key.
pub fn modifier_bit(name: &str) -> Option<u8> {
    match name {
        "Control_L" | "Control_R" => Some(MOD_CTRL),
        "Shift_L" | "Shift_R" => Some(MOD_SHIFT),
        "Alt_L" | "Alt_R" | "Meta_L" | "Meta_R" => Some(MOD_ALT),
        "Super_L" | "Super_R" | "Hyper_L" | "Hyper_R" => Some(MOD_SUPER),
        _ => None,
    }
}

/// Display label for a keysym name.
pub fn label_for(name: &str) -> String {
    let l = match name {
        "Control_L" | "Control_R" => "Ctrl",
        "Shift_L" | "Shift_R" => "Shift",
        "Alt_L" | "Meta_L" | "Meta_R" => "Alt",
        "Alt_R" | "ISO_Level3_Shift" => "AltGr",
        "Super_L" | "Super_R" | "Hyper_L" | "Hyper_R" => "Super",
        "Return" | "KP_Enter" => "Enter",
        "BackSpace" => "Backspace",
        "Escape" => "Esc",
        "Tab" | "ISO_Left_Tab" => "Tab",
        "space" => "Space",
        "Delete" => "Del",
        "Insert" => "Ins",
        "Prior" => "PgUp",
        "Next" => "PgDn",
        "Home" => "Home",
        "End" => "End",
        "Left" => "←",
        "Right" => "→",
        "Up" => "↑",
        "Down" => "↓",
        "Caps_Lock" => "Caps",
        "Num_Lock" => "Num",
        "Scroll_Lock" => "ScrLk",
        "Print" => "PrtSc",
        "Pause" => "Pause",
        "Menu" => "Menu",
        "minus" => "-",
        "plus" => "+",
        "equal" => "=",
        "comma" => ",",
        "period" => ".",
        "slash" => "/",
        "backslash" => "\\",
        "semicolon" => ";",
        "apostrophe" => "'",
        "grave" => "`",
        "bracketleft" => "[",
        "bracketright" => "]",
        "asterisk" => "*",
        "KP_Add" => "+",
        "KP_Subtract" => "-",
        "KP_Multiply" => "*",
        "KP_Divide" => "/",
        other => {
            if let Some(d) = other.strip_prefix("KP_") {
                return d.to_string();
            }
            if other.chars().count() == 1 {
                return other.to_uppercase();
            }
            return other.to_string();
        }
    };
    l.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(label_for("Control_R"), "Ctrl");
        assert_eq!(label_for("a"), "A");
        assert_eq!(label_for("F5"), "F5");
        assert_eq!(label_for("Return"), "Enter");
        assert_eq!(label_for("KP_5"), "5");
    }

    #[test]
    fn modifiers() {
        assert_eq!(modifier_bit("Shift_L"), Some(MOD_SHIFT));
        assert_eq!(modifier_bit("a"), None);
    }
}
