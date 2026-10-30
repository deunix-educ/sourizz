//! X11 keysym <-> name/char helpers, shared by the X11 and Wayland keyboard paths.

const NAMED: &[(u32, &str)] = &[
    (0x0020, "space"),
    (0xff08, "BackSpace"),
    (0xff09, "Tab"),
    (0xfe20, "ISO_Left_Tab"),
    (0xff0d, "Return"),
    (0xff13, "Pause"),
    (0xff14, "Scroll_Lock"),
    (0xff1b, "Escape"),
    (0xff50, "Home"),
    (0xff51, "Left"),
    (0xff52, "Up"),
    (0xff53, "Right"),
    (0xff54, "Down"),
    (0xff55, "Prior"),
    (0xff56, "Next"),
    (0xff57, "End"),
    (0xff61, "Print"),
    (0xff63, "Insert"),
    (0xff67, "Menu"),
    (0xff7f, "Num_Lock"),
    (0xff8d, "KP_Enter"),
    (0xffaa, "KP_Multiply"),
    (0xffab, "KP_Add"),
    (0xffad, "KP_Subtract"),
    (0xffae, "KP_Decimal"),
    (0xffaf, "KP_Divide"),
    (0xffe1, "Shift_L"),
    (0xffe2, "Shift_R"),
    (0xffe3, "Control_L"),
    (0xffe4, "Control_R"),
    (0xffe5, "Caps_Lock"),
    (0xffe7, "Meta_L"),
    (0xffe8, "Meta_R"),
    (0xffe9, "Alt_L"),
    (0xffea, "Alt_R"),
    (0xffeb, "Super_L"),
    (0xffec, "Super_R"),
    (0xffed, "Hyper_L"),
    (0xffee, "Hyper_R"),
    (0xffff, "Delete"),
    (0xfe03, "ISO_Level3_Shift"),
];

/// Canonical name for a keysym (`a`, `F5`, `Control_R`, `KP_3`, `eacute`-style chars as the char).
pub fn name(sym: u32) -> String {
    if let Some((_, n)) = NAMED.iter().find(|(s, _)| *s == sym) {
        return (*n).to_string();
    }
    match sym {
        0xffbe..=0xffd5 => format!("F{}", sym - 0xffbe + 1),
        0xffb0..=0xffb9 => format!("KP_{}", sym - 0xffb0),
        _ => match to_char(sym) {
            Some(c) => c.to_string(),
            None => format!("0x{sym:x}"),
        },
    }
}

/// Printable character for a keysym, if it has one.
pub fn to_char(sym: u32) -> Option<char> {
    match sym {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(sym),
        0x0100_0000..=0x0110_ffff => char::from_u32(sym - 0x0100_0000),
        0xffb0..=0xffb9 => char::from_u32('0' as u32 + sym - 0xffb0),
        0xffaa => Some('*'),
        0xffab => Some('+'),
        0xffad => Some('-'),
        0xffae => Some('.'),
        0xffaf => Some('/'),
        _ => None,
    }
    .filter(|c| !c.is_control())
}

/// Keysym for a config/hotkey key name: `S`, `5`, `F9`, `space`, `Return`, `Control_R`, …
pub fn from_name(n: &str) -> Option<u32> {
    if let Some((s, _)) = NAMED.iter().find(|(_, nm)| nm.eq_ignore_ascii_case(n)) {
        return Some(*s);
    }
    let lower = n.to_ascii_lowercase();
    if let Some(num) = lower.strip_prefix('f').and_then(|d| d.parse::<u32>().ok()) {
        if (1..=24).contains(&num) {
            return Some(0xffbe + num - 1);
        }
    }
    let mut chars = n.chars();
    let (c, rest) = (chars.next()?, chars.next());
    if rest.is_some() {
        return None;
    }
    let c = c.to_lowercase().next()?;
    let cp = c as u32;
    Some(if (0x20..=0x7e).contains(&cp) || (0xa0..=0xff).contains(&cp) { cp } else { 0x0100_0000 + cp })
}

/// True for keys that only change modifier state and should not be shown as keystrokes.
pub fn is_modifier(name: &str) -> bool {
    matches!(
        name,
        "Shift_L"
            | "Shift_R"
            | "Control_L"
            | "Control_R"
            | "Alt_L"
            | "Alt_R"
            | "Meta_L"
            | "Meta_R"
            | "Super_L"
            | "Super_R"
            | "Hyper_L"
            | "Hyper_R"
            | "Caps_Lock"
            | "Num_Lock"
            | "Scroll_Lock"
            | "ISO_Level3_Shift"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_roundtrip() {
        for n in ["Control_R", "Return", "F5", "F12", "space", "a", "Escape", "KP_Enter"] {
            let sym = from_name(n).unwrap_or_else(|| panic!("no keysym for {n}"));
            assert_eq!(name(sym).to_lowercase(), n.to_lowercase());
        }
        assert_eq!(from_name("S"), Some('s' as u32));
        assert_eq!(from_name("é"), Some(0xe9));
        assert_eq!(from_name("nonsense"), None);
        assert_eq!(from_name("F99"), None);
    }

    #[test]
    fn chars() {
        assert_eq!(to_char('a' as u32), Some('a'));
        assert_eq!(to_char(0xe9), Some('é'));
        assert_eq!(to_char(0x0100_20ac), Some('€'));
        assert_eq!(to_char(0xff0d), None);
        assert_eq!(to_char(0xffb5), Some('5'));
    }

    #[test]
    fn modifiers() {
        assert!(is_modifier("Control_R"));
        assert!(!is_modifier("a"));
    }
}
