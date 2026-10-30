//! Keyboard layout handling for Wayland, through libxkbcommon loaded at run time
//! (`dlopen`), so no development headers are needed to build and nothing breaks when
//! the library is missing: callers just get `None` and fall back.
//!
//! Wayland does not tell clients what is typed elsewhere, so global key labels come from
//! raw evdev key codes translated with the user's layout (see [`detect_layout`]). While
//! annotation mode owns the keyboard, the compositor's own keymap is used instead.

use crate::config::{MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER};
use crate::input::{KeyEvent, label_for};
use crate::keysym;
use std::ffi::{CString, c_char};
use xkbcommon_dl::{
    XkbCommon, xkb_context, xkb_context_flags, xkb_key_direction, xkb_keymap, xkb_keymap_compile_flags, xkb_keymap_format, xkb_rule_names,
    xkb_state, xkb_state_component,
};

pub struct Xkb {
    lib: &'static XkbCommon,
    ctx: *mut xkb_context,
    keymap: *mut xkb_keymap,
    state: *mut xkb_state,
}

// The raw pointers are only ever used from the thread that owns the `Xkb`.
unsafe impl Send for Xkb {}

/// Read `layout`, `variant` and `options` from a `KEY=value` style file such as
/// `/etc/default/keyboard` or KDE's `kxkbrc`. The layout is required; the rest default to empty.
fn parse_layout_file(text: &str, keys: [&str; 3], sep: char) -> Option<(String, String, String)> {
    let get = |k: &str| {
        text.lines().find_map(|l| {
            let (name, val) = l.split_once(sep)?;
            (name.trim() == k).then(|| val.trim().trim_matches('"').to_string())
        })
    };
    let layout = get(keys[0]).filter(|l| !l.is_empty())?;
    Some((layout, get(keys[1]).unwrap_or_default(), get(keys[2]).unwrap_or_default()))
}

/// Keyboard layout (`layout`, `variant`, `options`) for the evdev path.
///
/// Order of precedence: `XKB_DEFAULT_*` environment variables, `/etc/default/keyboard`,
/// KDE's `kxkbrc`, then plain `us`.
pub fn detect_layout() -> (String, String, String) {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if let Some(l) = env("XKB_DEFAULT_LAYOUT") {
        return (l, env("XKB_DEFAULT_VARIANT").unwrap_or_default(), env("XKB_DEFAULT_OPTIONS").unwrap_or_default());
    }
    if let Ok(t) = std::fs::read_to_string("/etc/default/keyboard") {
        if let Some(r) = parse_layout_file(&t, ["XKBLAYOUT", "XKBVARIANT", "XKBOPTIONS"], '=') {
            return r;
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        if let Ok(t) = std::fs::read_to_string(std::path::Path::new(&home).join(".config/kxkbrc")) {
            if let Some(r) = parse_layout_file(&t, ["LayoutList", "VariantList", "Options"], '=') {
                return r;
            }
        }
    }
    ("us".into(), String::new(), String::new())
}

impl Xkb {
    pub fn available() -> bool {
        xkbcommon_dl::xkbcommon_option().is_some()
    }

    fn with_keymap(lib: &'static XkbCommon, ctx: *mut xkb_context, keymap: *mut xkb_keymap) -> Option<Xkb> {
        if keymap.is_null() {
            unsafe { (lib.xkb_context_unref)(ctx) };
            return None;
        }
        let state = unsafe { (lib.xkb_state_new)(keymap) };
        if state.is_null() {
            unsafe {
                (lib.xkb_keymap_unref)(keymap);
                (lib.xkb_context_unref)(ctx);
            }
            return None;
        }
        Some(Xkb { lib, ctx, keymap, state })
    }

    pub fn from_names(layout: &str, variant: &str, options: &str) -> Option<Xkb> {
        let lib = xkbcommon_dl::xkbcommon_option()?;
        let ctx = unsafe { (lib.xkb_context_new)(xkb_context_flags::XKB_CONTEXT_NO_FLAGS) };
        if ctx.is_null() {
            return None;
        }
        let c = |s: &str| CString::new(s).ok();
        let (layout, variant, options) = (c(layout)?, c(variant)?, c(options)?);
        let names = xkb_rule_names {
            rules: std::ptr::null(),
            model: std::ptr::null(),
            layout: layout.as_ptr(),
            variant: variant.as_ptr(),
            options: options.as_ptr(),
        };
        let keymap = unsafe { (lib.xkb_keymap_new_from_names)(ctx, &names, xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS) };
        Self::with_keymap(lib, ctx, keymap)
    }

    /// Build from the keymap text a Wayland compositor sends in `wl_keyboard.keymap`.
    pub fn from_keymap_string(text: &str) -> Option<Xkb> {
        let lib = xkbcommon_dl::xkbcommon_option()?;
        let ctx = unsafe { (lib.xkb_context_new)(xkb_context_flags::XKB_CONTEXT_NO_FLAGS) };
        if ctx.is_null() {
            return None;
        }
        let text = CString::new(text.trim_end_matches('\0')).ok()?;
        let keymap = unsafe {
            (lib.xkb_keymap_new_from_string)(
                ctx,
                text.as_ptr(),
                xkb_keymap_format::XKB_KEYMAP_FORMAT_TEXT_V1,
                xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS,
            )
        };
        Self::with_keymap(lib, ctx, keymap)
    }

    /// Forget all pressed keys and modifiers (used when keyboard focus changes).
    pub fn reset(&mut self) {
        unsafe {
            (self.lib.xkb_state_unref)(self.state);
            self.state = (self.lib.xkb_state_new)(self.keymap);
        }
    }

    fn mod_active(&self, name: &[u8]) -> bool {
        debug_assert!(name.ends_with(&[0]));
        unsafe {
            (self.lib.xkb_state_mod_name_is_active)(
                self.state,
                name.as_ptr() as *const c_char,
                xkb_state_component::XKB_STATE_MODS_EFFECTIVE,
            ) > 0
        }
    }

    fn base_sym(&self, kc: u32) -> u32 {
        let mut syms: *const u32 = std::ptr::null();
        let n = unsafe { (self.lib.xkb_keymap_key_get_syms_by_level)(self.keymap, kc, 0, 0, &mut syms) };
        if n > 0 && !syms.is_null() { unsafe { *syms } } else { 0 }
    }

    /// Level-0 keysym of an evdev key code (what a hotkey chord names).
    pub fn base_keysym(&self, evdev_code: u32) -> u32 {
        self.base_sym(evdev_code + 8)
    }

    /// Translate one evdev key event, updating the modifier state.
    ///
    /// `mods` in the result are the modifiers held *before* this key, like on X11.
    pub fn event(&mut self, evdev_code: u32, pressed: bool, repeat: bool) -> KeyEvent {
        let kc = evdev_code + 8;
        let base = self.base_sym(kc);
        let name = keysym::name(base);
        let is_modifier = keysym::is_modifier(&name);
        let shift = self.mod_active(b"Shift\0");
        let mut mods = 0u8;
        if shift {
            mods |= MOD_SHIFT;
        }
        if self.mod_active(b"Control\0") {
            mods |= MOD_CTRL;
        }
        if self.mod_active(b"Mod1\0") {
            mods |= MOD_ALT;
        }
        if self.mod_active(b"Mod4\0") {
            mods |= MOD_SUPER;
        }
        let sym = unsafe { (self.lib.xkb_state_key_get_one_sym)(self.state, kc) };
        let utf32 = unsafe { (self.lib.xkb_state_key_get_utf32)(self.state, kc) };
        let label_sym = if shift && sym != 0 { sym } else { base };
        let text = if mods & (MOD_CTRL | MOD_ALT | MOD_SUPER) == 0 { char::from_u32(utf32).filter(|c| !c.is_control()) } else { None };
        if !repeat {
            let dir = if pressed { xkb_key_direction::XKB_KEY_DOWN } else { xkb_key_direction::XKB_KEY_UP };
            unsafe { (self.lib.xkb_state_update_key)(self.state, kc, dir) };
        }
        KeyEvent { pressed, name, label: label_for(&keysym::name(label_sym)), text, mods, is_modifier, repeat }
    }
}

impl Drop for Xkb {
    fn drop(&mut self) {
        unsafe {
            (self.lib.xkb_state_unref)(self.state);
            (self.lib.xkb_keymap_unref)(self.keymap);
            (self.lib.xkb_context_unref)(self.ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // evdev codes
    const KEY_Q: u32 = 16;
    const KEY_A: u32 = 30;
    const KEY_LEFTSHIFT: u32 = 42;
    const KEY_LEFTCTRL: u32 = 29;
    const KEY_RIGHTCTRL: u32 = 97;
    const KEY_1: u32 = 2;

    fn xkb(layout: &str) -> Option<Xkb> {
        if !Xkb::available() {
            eprintln!("libxkbcommon not installed: skipping");
            return None;
        }
        Some(Xkb::from_names(layout, "", "").expect("layout compiles"))
    }

    #[test]
    fn azerty_and_qwerty_label_the_same_key_differently() {
        let (Some(mut fr), Some(mut us)) = (xkb("fr"), xkb("us")) else { return };
        // The physical key right of Tab: 'a' on AZERTY, 'q' on QWERTY.
        let e = fr.event(KEY_Q, true, false);
        assert_eq!(e.name, "a");
        assert_eq!(e.text, Some('a'));
        let e = us.event(KEY_Q, true, false);
        assert_eq!(e.name, "q");
    }

    #[test]
    fn shift_changes_text_and_digits_on_azerty() {
        let Some(mut fr) = xkb("fr") else { return };
        let plain = fr.event(KEY_1, true, false);
        assert_eq!(plain.text, Some('&'), "AZERTY digit row is symbols without Shift");
        fr.event(KEY_1, false, false);
        let sh = fr.event(KEY_LEFTSHIFT, true, false);
        assert!(sh.is_modifier);
        let e = fr.event(KEY_1, true, false);
        assert_eq!(e.text, Some('1'));
        assert_eq!(e.mods, MOD_SHIFT);
        assert_eq!(e.label, "1");
        fr.event(KEY_1, false, false);
        fr.event(KEY_LEFTSHIFT, false, false);
        assert_eq!(fr.event(KEY_1, true, false).text, Some('&'), "shift released");
    }

    #[test]
    fn ctrl_hides_text_and_reports_modifiers() {
        let Some(mut us) = xkb("us") else { return };
        us.event(KEY_LEFTCTRL, true, false);
        let e = us.event(KEY_A, true, false);
        assert_eq!(e.text, None);
        assert_eq!(e.mods, MOD_CTRL);
        assert_eq!(e.name, "a");
    }

    #[test]
    fn modifier_key_names_match_config_values() {
        let Some(mut us) = xkb("us") else { return };
        assert_eq!(us.event(KEY_RIGHTCTRL, true, false).name, "Control_R");
        assert_eq!(us.base_keysym(KEY_A), 'a' as u32);
    }

    #[test]
    fn layout_files_are_parsed() {
        let debian = "XKBMODEL=\"pc105\"\nXKBLAYOUT=\"fr\"\nXKBVARIANT=\"oss\"\nXKBOPTIONS=\"ctrl:nocaps\"\n";
        assert_eq!(
            parse_layout_file(debian, ["XKBLAYOUT", "XKBVARIANT", "XKBOPTIONS"], '='),
            Some(("fr".into(), "oss".into(), "ctrl:nocaps".into()))
        );
        let kde = "[Layout]\nLayoutList=fr,us\nUse=true\n";
        assert_eq!(
            parse_layout_file(kde, ["LayoutList", "VariantList", "Options"], '='),
            Some(("fr,us".into(), String::new(), String::new()))
        );
        assert_eq!(parse_layout_file("XKBLAYOUT=\"\"\n", ["XKBLAYOUT", "XKBVARIANT", "XKBOPTIONS"], '='), None);
        assert_eq!(parse_layout_file("nothing here", ["XKBLAYOUT", "XKBVARIANT", "XKBOPTIONS"], '='), None);
    }
}
