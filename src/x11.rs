//! X11 backend.
//!
//! One override-redirect 32-bit ARGB window per monitor, made click-through with an
//! empty XShape *input* region. Global input comes from XInput2 *raw* events (they are
//! delivered no matter which client has the focus or a grab). Hotkeys use passive key
//! grabs. Annotation mode takes a real pointer + keyboard grab while it is active.
//!
//! A compositing manager is required for the transparency to show (KWin, Mutter,
//! Xfwm4, Picom… all provide it); without one the windows would render as opaque.

use crate::canvas::{Layer, Patch};
use crate::config::*;
use crate::control::{self, Command};
use crate::core::Core;
use crate::geom::IRect;
use crate::input::*;
use crate::keysym;
use anyhow::{Context, Result, anyhow};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fd::{AsFd, OwnedFd};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::Event;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

const SHIFT: u16 = 0x01;
const LOCK: u16 = 0x02;
const CONTROL: u16 = 0x04;
const MOD1: u16 = 0x08;
const MOD2: u16 = 0x10; // NumLock
const MOD4: u16 = 0x40;
const MOD5: u16 = 0x80; // AltGr on most layouts
const REAL_MODS: u16 = SHIFT | CONTROL | MOD1 | MOD4;

struct Atoms {
    net_wm_name: xproto::Atom,
    utf8: xproto::Atom,
    active_window: xproto::Atom,
    frame_extents: xproto::Atom,
    cm_owner: xproto::Atom,
}

struct Overlay {
    id: xproto::Window,
    layer: Layer,
}

struct Keymap {
    min: u8,
    per: usize,
    syms: Vec<u32>,
}

impl Keymap {
    fn sym(&self, keycode: u8, col: usize) -> u32 {
        if keycode < self.min || col >= self.per {
            return 0;
        }
        self.syms.get((keycode - self.min) as usize * self.per + col).copied().unwrap_or(0)
    }

    /// First keycode producing `sym` in any of the first four columns.
    fn keycode_for(&self, sym: u32) -> Option<u8> {
        let rows = self.syms.len() / self.per.max(1);
        (0..rows).find(|&r| (0..self.per.min(4)).any(|c| self.syms[r * self.per + c] == sym)).map(|r| self.min + r as u8)
    }
}

struct Binding {
    action: &'static str,
    keycode: u8,
    mods: u16,
}

pub struct X11 {
    conn: RustConnection,
    root: xproto::Window,
    visual: u32,
    colormap: u32,
    gc: Option<u32>,
    atoms: Atoms,
    overlays: Vec<Overlay>,
    keymap: Keymap,
    bindings: Vec<Binding>,
    bound: Hotkeys,
    cursor: u32,
    grabbed: bool,
    moved: bool,
    max_req: usize,
    warned_cm: bool,
}

fn mods_from_state(state: u16) -> u8 {
    let mut m = 0;
    if state & SHIFT != 0 {
        m |= MOD_SHIFT;
    }
    if state & CONTROL != 0 {
        m |= MOD_CTRL;
    }
    if state & MOD1 != 0 {
        m |= MOD_ALT;
    }
    if state & MOD4 != 0 {
        m |= MOD_SUPER;
    }
    m
}

fn chord_mask(mods: u8) -> u16 {
    let mut m = 0;
    if mods & MOD_SHIFT != 0 {
        m |= SHIFT;
    }
    if mods & MOD_CTRL != 0 {
        m |= CONTROL;
    }
    if mods & MOD_ALT != 0 {
        m |= MOD1;
    }
    if mods & MOD_SUPER != 0 {
        m |= MOD4;
    }
    m
}

impl X11 {
    pub fn connect() -> Result<X11> {
        let (conn, screen_num) = x11rb::connect(None).context("cannot connect to the X server (is DISPLAY set?)")?;
        let screen = conn.setup().roots[screen_num].clone();
        let root = screen.root;

        for (name, ext) in
            [("SHAPE", shape::X11_EXTENSION_NAME), ("RANDR", randr::X11_EXTENSION_NAME), ("XInputExtension", xinput::X11_EXTENSION_NAME)]
        {
            conn.extension_information(ext)?.ok_or_else(|| anyhow!("X server lacks the {name} extension"))?;
        }
        let v = conn.xinput_xi_query_version(2, 2)?.reply().context("XInput2 is required for global pointer events")?;
        log::debug!("XInput {}.{}", v.major_version, v.minor_version);

        let visual = screen
            .allowed_depths
            .iter()
            .filter(|d| d.depth == 32)
            .flat_map(|d| d.visuals.iter())
            .find(|v| {
                v.class == xproto::VisualClass::TRUE_COLOR
                    && v.red_mask == 0x00ff_0000
                    && v.green_mask == 0x0000_ff00
                    && v.blue_mask == 0x0000_00ff
            })
            .map(|v| v.visual_id)
            .ok_or_else(|| anyhow!("no 32-bit ARGB visual: transparent overlays are not possible on this X server"))?;
        let colormap = conn.generate_id()?;
        conn.create_colormap(xproto::ColormapAlloc::NONE, colormap, root, visual)?;

        let intern = |n: &str| -> Result<xproto::Atom> { Ok(conn.intern_atom(false, n.as_bytes())?.reply()?.atom) };
        let atoms = Atoms {
            net_wm_name: intern("_NET_WM_NAME")?,
            utf8: intern("UTF8_STRING")?,
            active_window: intern("_NET_ACTIVE_WINDOW")?,
            frame_extents: intern("_NET_FRAME_EXTENTS")?,
            cm_owner: intern(&format!("_NET_WM_CM_S{screen_num}"))?,
        };

        // Global, grab-independent input.
        let mask = xinput::XIEventMask::RAW_MOTION
            | xinput::XIEventMask::RAW_BUTTON_PRESS
            | xinput::XIEventMask::RAW_BUTTON_RELEASE
            | xinput::XIEventMask::RAW_KEY_PRESS
            | xinput::XIEventMask::RAW_KEY_RELEASE;
        conn.xinput_xi_select_events(root, &[xinput::EventMask { deviceid: 1, mask: vec![mask] }])?;
        conn.randr_select_input(root, randr::NotifyMask::SCREEN_CHANGE)?;
        conn.change_window_attributes(root, &xproto::ChangeWindowAttributesAux::new().event_mask(xproto::EventMask::PROPERTY_CHANGE))?;

        // Crosshair cursor for annotation mode.
        let font = conn.generate_id()?;
        conn.open_font(font, b"cursor")?;
        let cursor = conn.generate_id()?;
        conn.create_glyph_cursor(cursor, font, font, 34, 35, 0xffff, 0xffff, 0xffff, 0, 0, 0)?;
        conn.close_font(font)?;

        let max_req = conn.maximum_request_bytes();
        let mut x = X11 {
            conn,
            root,
            visual,
            colormap,
            gc: None,
            atoms,
            overlays: Vec::new(),
            keymap: Keymap { min: 8, per: 1, syms: Vec::new() },
            bindings: Vec::new(),
            bound: Hotkeys { toggle_all: "\0".into(), ..Hotkeys::default() },
            cursor,
            grabbed: false,
            moved: false,
            max_req,
            warned_cm: false,
        };
        x.reload_keymap()?;
        x.conn.flush()?;
        Ok(x)
    }

    fn reload_keymap(&mut self) -> Result<()> {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let r = self.conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        self.keymap = Keymap { min, per: r.keysyms_per_keycode as usize, syms: r.keysyms };
        Ok(())
    }

    // ------------------------------------------------------------------ windows

    fn monitors(&self) -> Vec<IRect> {
        let mut out: Vec<IRect> = Vec::new();
        if let Ok(Ok(r)) = self.conn.randr_get_monitors(self.root, true).map(|c| c.reply()) {
            for m in r.monitors {
                let rect = IRect::from_xywh(m.x as i32, m.y as i32, m.width as i32, m.height as i32);
                if !rect.is_empty() && !out.contains(&rect) {
                    out.push(rect);
                }
            }
        }
        if out.is_empty() {
            if let Ok(Ok(g)) = self.conn.get_geometry(self.root).map(|c| c.reply()) {
                out.push(IRect::from_xywh(0, 0, g.width as i32, g.height as i32));
            }
        }
        out
    }

    fn create_overlay(&mut self, rect: IRect) -> Result<Overlay> {
        let id = self.conn.generate_id()?;
        let aux = xproto::CreateWindowAux::new().background_pixel(0).border_pixel(0).override_redirect(1).colormap(self.colormap);
        self.conn.create_window(
            32,
            id,
            self.root,
            rect.x0 as i16,
            rect.y0 as i16,
            rect.width() as u16,
            rect.height() as u16,
            0,
            xproto::WindowClass::INPUT_OUTPUT,
            self.visual,
            &aux,
        )?;
        // Empty input region: every click goes straight through to the windows below.
        self.conn.shape_rectangles(shape::SO::SET, shape::SK::INPUT, xproto::ClipOrdering::UNSORTED, id, 0, 0, &[])?;
        self.conn.change_property8(
            xproto::PropMode::REPLACE,
            id,
            xproto::AtomEnum::WM_NAME,
            xproto::AtomEnum::STRING,
            b"sourizz-overlay",
        )?;
        self.conn.change_property8(xproto::PropMode::REPLACE, id, self.atoms.net_wm_name, self.atoms.utf8, b"sourizz-overlay")?;
        self.conn.change_property8(
            xproto::PropMode::REPLACE,
            id,
            xproto::AtomEnum::WM_CLASS,
            xproto::AtomEnum::STRING,
            b"sourizz\0Sourizz\0",
        )?;
        if self.gc.is_none() {
            let gc = self.conn.generate_id()?;
            self.conn.create_gc(gc, id, &xproto::CreateGCAux::new())?;
            self.gc = Some(gc);
        }
        self.conn.map_window(id)?;
        Ok(Overlay { id, layer: Layer::new(rect, false) })
    }

    fn rebuild_overlays(&mut self, core: &mut Core) -> Result<()> {
        for o in self.overlays.drain(..) {
            let _ = self.conn.destroy_window(o.id);
        }
        let mons = self.monitors();
        for r in &mons {
            let o = self.create_overlay(*r)?;
            self.overlays.push(o);
        }
        core.engine.set_monitors(mons);
        core.engine.request_full_redraw();
        self.conn.flush()?;
        if !self.warned_cm && self.conn.get_selection_owner(self.atoms.cm_owner)?.reply()?.owner == 0 {
            log::warn!("no compositing manager is running: the overlay cannot be transparent. Enable compositing in your window manager.");
            self.warned_cm = true;
        }
        Ok(())
    }

    fn raise_overlays(&mut self) {
        let Ok(Ok(tree)) = self.conn.query_tree(self.root).map(|c| c.reply()) else { return };
        let n = self.overlays.len();
        let top: Vec<u32> = tree.children.iter().rev().take(n).copied().collect();
        if self.overlays.iter().all(|o| top.contains(&o.id)) {
            return;
        }
        for o in &self.overlays {
            let _ = self.conn.configure_window(o.id, &xproto::ConfigureWindowAux::new().stack_mode(xproto::StackMode::ABOVE));
        }
        let _ = self.conn.flush();
    }

    fn upload(&self, idx: usize, patch: &Patch) -> Result<()> {
        let Some(gc) = self.gc else { return Ok(()) };
        let local = patch.rect;
        let w = local.width() as usize;
        let row = w * 4;
        let max_rows = (self.max_req.saturating_sub(64) / row).max(1);
        let id = self.overlays[idx].id;
        for (n, chunk) in patch.data.chunks(max_rows * row).enumerate() {
            let rows = chunk.len() / row;
            let y = local.y0 + (n * max_rows) as i32;
            self.conn.put_image(xproto::ImageFormat::Z_PIXMAP, id, gc, w as u16, rows as u16, local.x0 as i16, y as i16, 0, 32, chunk)?;
        }
        Ok(())
    }

    fn frame(&mut self, core: &mut Core) -> Result<Option<f64>> {
        let now = core.now();
        let plan = core.engine.begin_frame(now);
        if !plan.damage.is_empty() {
            for i in 0..self.overlays.len() {
                for patch in self.overlays[i].layer.repaint(&mut core.engine, &plan.damage, now) {
                    self.upload(i, &patch)?;
                }
            }
            self.conn.flush()?;
        }
        Ok(plan.next_in)
    }

    // ------------------------------------------------------------------ input helpers

    fn pointer(&self) -> Result<(f32, f32, u16)> {
        let r = self.conn.query_pointer(self.root)?.reply()?;
        Ok((r.root_x as f32, r.root_y as f32, u16::from(r.mask)))
    }

    fn key_event(&self, keycode: u8, state: u16, pressed: bool, repeat: bool) -> KeyEvent {
        let base = self.keymap.sym(keycode, 0);
        let name = keysym::name(base);
        let is_modifier = keysym::is_modifier(&name);
        let mut state = state;
        // Raw events can arrive before the server updates the modifier state; never count ourselves.
        if let Some(bit) = modifier_bit(&name) {
            state &= !match bit {
                MOD_SHIFT => SHIFT,
                MOD_CTRL => CONTROL,
                MOD_ALT => MOD1,
                _ => MOD4,
            };
        }
        let mods = mods_from_state(state);
        let shift = state & SHIFT != 0;
        let altgr = state & MOD5 != 0;
        let letter = base < 0x80 && (base as u8 as char).is_ascii_lowercase();
        let eff_shift = shift ^ (state & LOCK != 0 && letter);
        let col = usize::from(eff_shift) + if altgr { 2 } else { 0 };
        let sym = match self.keymap.sym(keycode, col) {
            0 => base,
            s => s,
        };
        // Shortcuts are labelled with the shifted symbol only when Shift is part of them.
        let label_sym = match self.keymap.sym(keycode, usize::from(shift)) {
            0 => base,
            s => s,
        };
        let text = if mods & (MOD_CTRL | MOD_ALT | MOD_SUPER) == 0 { keysym::to_char(sym) } else { None };
        KeyEvent { pressed, name: name.clone(), label: label_for(&keysym::name(label_sym)), text, mods, is_modifier, repeat }
    }

    fn match_hotkey(&self, keycode: u8, state: u16) -> Option<&'static str> {
        let m = state & REAL_MODS;
        self.bindings.iter().find(|b| b.keycode == keycode && b.mods == m).map(|b| b.action)
    }

    fn grab_hotkeys(&mut self, hk: &Hotkeys) {
        let _ = self.conn.ungrab_key(0, self.root, xproto::ModMask::ANY);
        self.bindings.clear();
        for (action, chord) in control::hotkey_table(hk) {
            let Some(sym) = keysym::from_name(&chord.key) else {
                log::warn!("hotkey {action}: unknown key {:?}", chord.key);
                continue;
            };
            let Some(keycode) = self.keymap.keycode_for(sym) else {
                log::warn!("hotkey {action}: key {:?} is not on this keyboard layout", chord.key);
                continue;
            };
            let mods = chord_mask(chord.mods);
            let mut ok = true;
            for lock in [0, LOCK, MOD2, LOCK | MOD2] {
                let res = self
                    .conn
                    .grab_key(
                        false,
                        self.root,
                        xproto::ModMask::from(mods | lock),
                        keycode,
                        xproto::GrabMode::ASYNC,
                        xproto::GrabMode::ASYNC,
                    )
                    .map_err(anyhow::Error::from)
                    .and_then(|c| c.check().map_err(anyhow::Error::from));
                if res.is_err() {
                    ok = false;
                }
            }
            if ok {
                self.bindings.push(Binding { action, keycode, mods });
            } else {
                log::warn!("hotkey {action} ({}+{}) is already used by another program", chord.mods, chord.key);
            }
        }
        self.bound = hk.clone();
        let _ = self.conn.flush();
    }

    // ------------------------------------------------------------------ capture (annotation mode)

    fn begin_capture(&mut self) -> bool {
        let Some(win) = self.overlays.first().map(|o| o.id) else { return false };
        let ev = xproto::EventMask::BUTTON_PRESS | xproto::EventMask::BUTTON_RELEASE | xproto::EventMask::POINTER_MOTION;
        let p = self
            .conn
            .grab_pointer(false, win, ev, xproto::GrabMode::ASYNC, xproto::GrabMode::ASYNC, 0u32, self.cursor, x11rb::CURRENT_TIME)
            .ok()
            .and_then(|c| c.reply().ok());
        if !matches!(p, Some(r) if r.status == xproto::GrabStatus::SUCCESS) {
            log::warn!("annotation mode: cannot grab the pointer (another program holds it)");
            return false;
        }
        let k = self
            .conn
            .grab_keyboard(false, win, x11rb::CURRENT_TIME, xproto::GrabMode::ASYNC, xproto::GrabMode::ASYNC)
            .ok()
            .and_then(|c| c.reply().ok());
        if !matches!(k, Some(r) if r.status == xproto::GrabStatus::SUCCESS) {
            log::warn!("annotation mode: cannot grab the keyboard");
            let _ = self.conn.ungrab_pointer(x11rb::CURRENT_TIME);
            let _ = self.conn.flush();
            return false;
        }
        self.grabbed = true;
        let _ = self.conn.flush();
        true
    }

    fn end_capture(&mut self) {
        if self.grabbed {
            let _ = self.conn.ungrab_pointer(x11rb::CURRENT_TIME);
            let _ = self.conn.ungrab_keyboard(x11rb::CURRENT_TIME);
            let _ = self.conn.flush();
            self.grabbed = false;
        }
    }

    // ------------------------------------------------------------------ focus mode

    fn focus_rect(&self) -> Option<IRect> {
        let p = self.conn.get_property(false, self.root, self.atoms.active_window, xproto::AtomEnum::WINDOW, 0, 1).ok()?.reply().ok()?;
        let win = p.value32()?.next().filter(|w| *w != 0)?;
        if self.overlays.iter().any(|o| o.id == win) {
            return None;
        }
        let attrs = self.conn.get_window_attributes(win).ok()?.reply().ok()?;
        if attrs.map_state != xproto::MapState::VIEWABLE {
            return None;
        }
        let geo = self.conn.get_geometry(win).ok()?.reply().ok()?;
        let tr = self.conn.translate_coordinates(win, self.root, 0, 0).ok()?.reply().ok()?;
        let (mut x0, mut y0) = (tr.dst_x as i32, tr.dst_y as i32);
        let (mut x1, mut y1) = (x0 + geo.width as i32, y0 + geo.height as i32);
        if let Some(e) = self
            .conn
            .get_property(false, win, self.atoms.frame_extents, xproto::AtomEnum::CARDINAL, 0, 4)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|v| v.collect::<Vec<u32>>()))
            .filter(|v| v.len() == 4)
        {
            x0 -= e[0] as i32;
            x1 += e[1] as i32;
            y0 -= e[2] as i32;
            y1 += e[3] as i32;
        }
        Some(IRect::new(x0, y0, x1, y1))
    }

    // ------------------------------------------------------------------ main loop

    pub fn run(&mut self, core: &mut Core, wake: &OwnedFd) -> Result<()> {
        self.rebuild_overlays(core)?;
        let grab_cfg = core.cfg.hotkeys.clone();
        self.grab_hotkeys(&grab_cfg);
        if let Ok((x, y, _)) = self.pointer() {
            core.handle_input(InputEvent::Pointer { x, y });
        }

        let mut dirty = true;
        let mut last_frame = f64::NEG_INFINITY;
        let mut next_frame: Option<f64> = None;
        let mut last_focus = f64::NEG_INFINITY;
        let mut last_raise = 0.0;

        while !core.quit {
            if core.pump() {
                dirty = true;
            }
            if core.quit {
                break;
            }
            if core.cfg.hotkeys != self.bound {
                let hk = core.cfg.hotkeys.clone();
                self.grab_hotkeys(&hk);
            }

            // Drain every pending X event before drawing.
            while let Some(event) = self.conn.poll_for_event()? {
                self.on_event(core, event, &mut dirty)?;
            }
            if self.moved && !self.grabbed {
                self.moved = false;
                if let Ok((x, y, _)) = self.pointer() {
                    core.handle_input(InputEvent::Pointer { x, y });
                    dirty = true;
                }
            }

            for action in core.drain_actions() {
                match action {
                    Action::Capture(true) => {
                        if !self.begin_capture() {
                            core.apply(Command::AnnotateToggle);
                        }
                    }
                    Action::Capture(false) => self.end_capture(),
                    Action::PlayClick(..) => {}
                }
                dirty = true;
            }

            let now = core.now();
            if core.cfg.focus.enabled && now - last_focus >= 0.25 {
                last_focus = now;
                core.engine.set_focus_rect(self.focus_rect());
                dirty = true;
            } else if !core.cfg.focus.enabled {
                core.engine.set_focus_rect(None);
            }
            if now - last_raise > 2.0 {
                last_raise = now;
                self.raise_overlays();
            }

            let min_interval = 1.0 / core.cfg.general.fps as f64;
            let due = dirty || next_frame.is_some_and(|t| now >= t);
            if due && now - last_frame >= min_interval {
                if next_frame.is_some() && !dirty {
                    // Animation tick: pick up warps / pointer moves the raw events can miss.
                    if let (false, Ok((x, y, _))) = (self.grabbed, self.pointer()) {
                        core.handle_input(InputEvent::Pointer { x, y });
                    }
                }
                last_frame = now;
                dirty = false;
                next_frame = self.frame(core)?.map(|dt| core.now() + dt);
            }

            // Replies read above (query_pointer, ...) may have queued events inside x11rb behind
            // our back; poll() on the socket would never report those, so look before sleeping.
            if let Some(ev) = self.conn.poll_for_event()? {
                self.on_event(core, ev, &mut dirty)?;
                continue;
            }

            // Sleep until the next thing that needs doing.
            let now = core.now();
            let mut timeout = 0.5f64;
            if dirty {
                timeout = timeout.min((last_frame + min_interval - now).max(0.0));
            } else if let Some(t) = next_frame {
                timeout = timeout.min((t - now).max(0.0));
            }
            if core.cfg.focus.enabled {
                timeout = timeout.min((last_focus + 0.25 - now).max(0.0));
            }
            self.wait(wake, timeout);
            control::drain(wake);
        }
        self.end_capture();
        Ok(())
    }

    fn wait(&self, wake: &OwnedFd, timeout: f64) {
        let ts = Timespec { tv_sec: timeout as i64, tv_nsec: (timeout.fract() * 1e9) as _ };
        let x = self.conn.stream().as_fd();
        let mut fds = [PollFd::new(&x, PollFlags::IN), PollFd::new(wake, PollFlags::IN)];
        let _ = poll(&mut fds, Some(&ts));
    }

    fn on_event(&mut self, core: &mut Core, event: Event, dirty: &mut bool) -> Result<()> {
        match event {
            Event::XinputRawMotion(_) => self.moved = true,
            Event::XinputRawButtonPress(e) | Event::XinputRawButtonRelease(e) if !self.grabbed => {
                let pressed = e.event_type == xinput::RAW_BUTTON_PRESS_EVENT;
                let button = match e.detail {
                    1 => Button::Left,
                    2 => Button::Middle,
                    3 => Button::Right,
                    4..=7 => return Ok(()), // scroll wheel
                    _ => Button::Other,
                };
                if let Ok((x, y, _)) = self.pointer() {
                    core.handle_input(InputEvent::Pointer { x, y });
                }
                core.handle_input(InputEvent::Button { button, pressed });
                self.moved = false;
                *dirty = true;
            }
            Event::XinputRawKeyPress(e) | Event::XinputRawKeyRelease(e) if !self.grabbed => {
                let pressed = e.event_type == xinput::RAW_KEY_PRESS_EVENT;
                let repeat = u32::from(e.flags) & (1 << 16) != 0;
                let state = self.pointer().map(|p| p.2).unwrap_or(0);
                let ev = self.key_event(e.detail as u8, state, pressed, repeat);
                core.handle_input(InputEvent::Key(ev));
                *dirty = true;
            }
            Event::KeyPress(e) => {
                if let Some(action) = self.match_hotkey(e.detail, e.state.into()) {
                    if let Some(cmd) = control::hotkey_command(action) {
                        core.apply(cmd);
                        *dirty = true;
                    }
                } else if self.grabbed {
                    let ev = self.key_event(e.detail, e.state.into(), true, false);
                    core.handle_input(InputEvent::Key(ev));
                    *dirty = true;
                }
            }
            Event::KeyRelease(e) if self.grabbed => {
                let ev = self.key_event(e.detail, e.state.into(), false, false);
                core.handle_input(InputEvent::Key(ev));
            }
            Event::MotionNotify(e) if self.grabbed => {
                core.handle_input(InputEvent::Pointer { x: e.root_x as f32, y: e.root_y as f32 });
                *dirty = true;
            }
            Event::ButtonPress(e) | Event::ButtonRelease(e) if self.grabbed => {
                let pressed = e.response_type & 0x7f == xproto::BUTTON_PRESS_EVENT;
                let button = match e.detail {
                    1 => Button::Left,
                    2 => Button::Middle,
                    3 => Button::Right,
                    _ => return Ok(()),
                };
                core.handle_input(InputEvent::Pointer { x: e.root_x as f32, y: e.root_y as f32 });
                core.handle_input(InputEvent::Button { button, pressed });
                *dirty = true;
            }
            Event::MappingNotify(m) if m.request == xproto::Mapping::KEYBOARD => {
                self.reload_keymap()?;
                let hk = core.cfg.hotkeys.clone();
                self.grab_hotkeys(&hk);
            }
            Event::RandrScreenChangeNotify(_) => {
                self.rebuild_overlays(core)?;
                *dirty = true;
            }
            Event::Error(e) => log::debug!("X error: {e:?}"),
            _ => {}
        }
        Ok(())
    }
}

pub fn run(core: &mut Core, wake: &OwnedFd) -> Result<()> {
    let mut x = X11::connect()?;
    log::info!("X11 backend ready ({} monitor(s))", x.monitors().len());
    x.run(core, wake)
}
