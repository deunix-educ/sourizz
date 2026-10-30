//! Wayland backend (wlroots layer-shell: KDE Plasma, Hyprland, sway, river, niri, COSMIC…).
//!
//! GNOME's Mutter does not implement layer-shell, so it cannot host the overlay; use an X11
//! session there (see the README).
//!
//! * One full-output layer surface per monitor on the `overlay` layer, with an *empty input
//!   region* so every click passes through.
//! * Software rendering into `wl_shm` buffers; only damaged rectangles are reported.
//! * Global clicks and keystrokes come from evdev, the pointer position from the compositor
//!   when it can tell us (see [`crate::wlsrc`]).
//! * Annotation mode temporarily gives one layer a full input region and exclusive keyboard
//!   focus, and then real Wayland pointer/keyboard events are used.

use crate::canvas::Layer;
use crate::config::Chord;
use crate::control::{self, Command};
use crate::core::Core;
use crate::evdev_in::{self, Devices, Raw};
use crate::geom::IRect;
use crate::input::{Action, InputEvent};
use crate::keysym;
use crate::kwin::KwinFeed;
use crate::wlsrc::{Estimate, Hyprland};
use crate::xkb::{self, Xkb};
use anyhow::{Context, Result, anyhow};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fd::{BorrowedFd, OwnedFd};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData, Region},
    delegate_registry,
    dispatch2::Dispatch2,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer as WlrLayer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, QueueHandle, WEnum,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
};

/// Marks our `wl_keyboard` objects so events are routed to [`Dispatch2`] below.
struct KbData;

enum Source {
    Hyprland(Hyprland),
    /// KDE Plasma: a KWin script pushes the position to us.
    Kwin(KwinFeed),
    /// Position is dead-reckoned from relative evdev motion.
    Estimate,
}

struct OutputLayer {
    output: wl_output::WlOutput,
    layer: LayerSurface,
    canvas: Layer,
    rect: IRect,
    configured: bool,
    frame_pending: bool,
    pending: Vec<IRect>,
}

struct App {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    layers: Vec<OutputLayer>,
    core: Core,

    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// Layer index that currently owns input (annotation mode).
    capture: Option<usize>,
    /// Compositor keymap, used only while annotation mode has keyboard focus.
    cap_xkb: Option<Xkb>,

    devices: Devices,
    /// Layout-based translation for global (evdev) key events.
    kb_xkb: Option<Xkb>,
    source: Source,
    estimate: Estimate,
    hotkeys: Vec<(&'static str, Chord)>,
    hotkeys_for: crate::config::Hotkeys,

    last_pos: Option<(f32, f32)>,
    moved: bool,
    dirty: bool,
    next_frame: Option<f64>,
    last_frame: f64,
    next_pos_poll: f64,
    monitors_dirty: bool,
    exit: bool,
}

// ------------------------------------------------------------------ SCTK handlers

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}

    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        if let Some(i) = self.layers.iter().position(|l| l.layer.wl_surface() == surface) {
            log::debug!("frame callback for layer {i}, pending damage: {}", self.layers[i].pending.len());
            self.layers[i].frame_pending = false;
            if !self.layers[i].pending.is_empty() {
                self.commit_layer(i, qh);
            }
        }
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add_layer(qh, output);
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.monitors_dirty = true;
    }

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.layers.retain(|l| l.output != output);
        self.monitors_dirty = true;
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.layers.retain(|l| l.layer.wl_surface() != layer.wl_surface());
        self.monitors_dirty = true;
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, configure: LayerSurfaceConfigure, _serial: u32) {
        let Some(i) = self.layers.iter().position(|l| l.layer.wl_surface() == layer.wl_surface()) else { return };
        let info = self.output_state.info(&self.layers[i].output);
        let (mut w, mut h) = (configure.new_size.0 as i32, configure.new_size.1 as i32);
        if w == 0 || h == 0 {
            if let Some((lw, lh)) = info.as_ref().and_then(|o| o.logical_size) {
                (w, h) = (lw, lh);
            }
        }
        if w == 0 || h == 0 {
            return;
        }
        let (x, y) = info.as_ref().and_then(|o| o.logical_position).or(info.as_ref().map(|o| o.location)).unwrap_or((0, 0));
        let l = &mut self.layers[i];
        let rect = IRect::from_xywh(x, y, w, h);
        if rect != l.rect || !l.configured {
            l.rect = rect;
            l.canvas = Layer::new(rect, true);
            l.pending.clear();
            l.frame_pending = false;
        }
        l.configured = true;
        self.monitors_dirty = true;
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = Some(seat.get_keyboard(qh, KbData));
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
        if cap == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for App {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        let Some(ci) = self.capture else { return };
        for ev in events {
            let Some(l) = self.layers.get(ci) else { return };
            if &ev.surface != l.layer.wl_surface() {
                continue;
            }
            let (x, y) = (l.rect.x0 as f32 + ev.position.0 as f32, l.rect.y0 as f32 + ev.position.1 as f32);
            self.estimate.sync((x, y));
            match ev.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => self.feed(InputEvent::Pointer { x, y }),
                PointerEventKind::Press { button, .. } | PointerEventKind::Release { button, .. } => {
                    let pressed = matches!(ev.kind, PointerEventKind::Press { .. });
                    if let Some(b) = evdev_in::button_from_code(button as u16) {
                        self.feed(InputEvent::Pointer { x, y });
                        self.feed(InputEvent::Button { button: b, pressed });
                    }
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl Dispatch2<wl_keyboard::WlKeyboard, App> for KbData {
    fn event(&self, app: &mut App, _: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _: &Connection, _: &QueueHandle<App>) {
        use wl_keyboard::Event;
        match event {
            Event::Keymap { fd, size, .. } => {
                // The descriptor shares its file offset with the compositor, which has just
                // written the text and left the offset at the end: read from 0 explicitly.
                let mut buf = vec![0u8; size as usize];
                let file = std::fs::File::from(fd);
                if std::os::unix::fs::FileExt::read_exact_at(&file, &mut buf, 0).is_ok() {
                    app.cap_xkb = Xkb::from_keymap_string(&String::from_utf8_lossy(&buf));
                    if app.cap_xkb.is_none() {
                        log::warn!("could not parse the compositor's keyboard layout; annotation text entry is disabled");
                    }
                }
            }
            Event::Enter { keys, .. } => {
                // Replay keys already held (e.g. the hotkey that opened annotation mode) so
                // the modifier state is right.
                if let Some(x) = app.cap_xkb.as_mut() {
                    x.reset();
                    for chunk in keys.chunks_exact(4) {
                        let code = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                        let _ = x.event(code, true, false);
                    }
                }
            }
            Event::Key { key, state, .. } if app.capture.is_some() => {
                let pressed = matches!(state, WEnum::Value(wl_keyboard::KeyState::Pressed));
                if let Some(x) = app.cap_xkb.as_mut() {
                    let ev = x.event(key, pressed, false);
                    if pressed {
                        app.run_hotkey_for(&ev, key);
                    }
                    app.feed(InputEvent::Key(ev));
                }
            }
            _ => {}
        }
    }
}

delegate_registry!(App);
smithay_client_toolkit::delegate_dispatch2!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

// ------------------------------------------------------------------ App logic

impl App {
    fn feed(&mut self, ev: InputEvent) {
        if let InputEvent::Pointer { x, y } = ev {
            self.last_pos = Some((x, y));
        }
        self.core.handle_input(ev);
        self.dirty = true;
    }

    fn add_layer(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if self.layers.iter().any(|l| l.output == output) {
            return;
        }
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(qh, surface, WlrLayer::Overlay, Some("sourizz"), Some(&output));
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0);
        self.make_click_through(&layer);
        layer.commit();
        self.layers.push(OutputLayer {
            output,
            layer,
            canvas: Layer::new(IRect::new(0, 0, 1, 1), true),
            rect: IRect::new(0, 0, 0, 0),
            configured: false,
            frame_pending: false,
            pending: Vec::new(),
        });
    }

    fn make_click_through(&self, layer: &LayerSurface) {
        // An empty (but present) input region means "no part of this surface takes input".
        if let Ok(region) = Region::new(&self.compositor) {
            layer.set_input_region(Some(region.wl_region()));
        }
    }

    fn rebuild_monitors(&mut self) {
        self.monitors_dirty = false;
        let rects: Vec<IRect> = self.layers.iter().filter(|l| l.configured).map(|l| l.rect).collect();
        if rects.is_empty() {
            return;
        }
        let all = rects.iter().fold(IRect::new(0, 0, 0, 0), |a, r| a.union(*r));
        self.estimate.set_bounds(all);
        self.core.engine.set_monitors(rects);
        self.core.engine.request_full_redraw();
        for l in &mut self.layers {
            l.pending.clear();
            if l.configured {
                l.pending.push(IRect::from_xywh(0, 0, l.rect.width(), l.rect.height()));
            }
        }
        self.dirty = true;
    }

    fn commit_layer(&mut self, i: usize, qh: &QueueHandle<Self>) {
        let l = &mut self.layers[i];
        let (w, h) = (l.canvas.w as i32, l.canvas.h as i32);
        log::debug!("commit layer {i}: {w}x{h}, {} damage rect(s)", l.pending.len());
        let Ok((buffer, canvas)) = self.pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else {
            log::warn!("cannot allocate a {w}x{h} buffer");
            return;
        };
        canvas.copy_from_slice(&l.canvas.bgra);
        let surface = l.layer.wl_surface().clone();
        for r in l.pending.drain(..) {
            surface.damage_buffer(r.x0, r.y0, r.width(), r.height());
        }
        surface.frame(qh, FrameCallbackData(surface.clone()));
        if buffer.attach_to(&surface).is_ok() {
            l.layer.commit();
            l.frame_pending = true;
        }
    }

    fn frame(&mut self, qh: &QueueHandle<Self>) {
        let now = self.core.now();
        self.last_frame = now;
        self.dirty = false;
        let plan = self.core.engine.begin_frame(now);
        for i in 0..self.layers.len() {
            if !self.layers[i].configured {
                continue;
            }
            let patches = self.layers[i].canvas.repaint(&mut self.core.engine, &plan.damage, now);
            self.layers[i].pending.extend(patches.iter().map(|p| p.rect));
            log::debug!("frame: layer {i} pending={} frame_pending={}", self.layers[i].pending.len(), self.layers[i].frame_pending);
            if !self.layers[i].pending.is_empty() && !self.layers[i].frame_pending {
                self.commit_layer(i, qh);
            }
        }
        self.next_frame = plan.next_in.map(|dt| now + dt);
    }

    fn sync_pointer(&mut self) {
        if self.capture.is_some() {
            return;
        }
        let pos = match &self.source {
            Source::Hyprland(h) => h.cursor_pos(),
            Source::Kwin(k) => k.take_latest(),
            Source::Estimate => Some(self.estimate.pos()),
        };
        if let Some((x, y)) = pos {
            if self.last_pos != Some((x, y)) {
                self.estimate.sync((x, y));
                self.feed(InputEvent::Pointer { x, y });
            }
        }
    }

    fn handle_raw(&mut self, raws: Vec<Raw>) {
        for raw in raws {
            if self.capture.is_some() {
                continue; // real Wayland events are used while annotating
            }
            match raw {
                Raw::Motion(dx, dy) => {
                    self.estimate.motion(dx, dy);
                    self.moved = true;
                }
                Raw::MotionHint => self.moved = true,
                Raw::Button(button, pressed) => {
                    self.sync_pointer();
                    self.feed(InputEvent::Button { button, pressed });
                }
                Raw::Key { code, pressed, repeat } => {
                    let Some(x) = self.kb_xkb.as_mut() else { continue };
                    let ev = x.event(code, pressed, repeat);
                    if pressed && !repeat {
                        self.run_hotkey_for(&ev, code);
                    }
                    self.feed(InputEvent::Key(ev));
                }
            }
        }
        if self.moved {
            self.moved = false;
            self.sync_pointer();
        }
    }

    /// Fire a configured global hotkey if this key press completes one.
    fn run_hotkey_for(&mut self, ev: &crate::input::KeyEvent, evdev_code: u32) {
        if self.hotkeys_for != self.core.cfg.hotkeys {
            self.hotkeys = control::hotkey_table(&self.core.cfg.hotkeys);
            self.hotkeys_for = self.core.cfg.hotkeys.clone();
        }
        let base = self.kb_xkb.as_ref().map(|x| x.base_keysym(evdev_code)).unwrap_or(0);
        let base = if base == 0 { keysym::from_name(&ev.name).unwrap_or(0) } else { base };
        let hit = self.hotkeys.iter().find(|(_, c)| c.mods == ev.mods && keysym::from_name(&c.key) == Some(base)).map(|(n, _)| *n);
        if let Some(cmd) = hit.and_then(control::hotkey_command) {
            self.core.apply(cmd);
            self.dirty = true;
        }
    }

    fn process_actions(&mut self) {
        for action in self.core.drain_actions() {
            match action {
                Action::Capture(true) => self.begin_capture(),
                Action::Capture(false) => self.end_capture(),
                Action::PlayClick(..) => {}
            }
        }
    }

    fn begin_capture(&mut self) {
        let at = self.last_pos.or(Some(self.estimate.pos()));
        let idx = at
            .and_then(|(x, y)| self.layers.iter().position(|l| l.configured && l.rect.contains(x, y)))
            .or_else(|| self.layers.iter().position(|l| l.configured));
        let Some(i) = idx else {
            self.core.apply(Command::AnnotateToggle);
            return;
        };
        let l = &self.layers[i];
        l.layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        l.layer.set_input_region(None); // the whole surface takes input
        l.layer.commit();
        self.capture = Some(i);
        if let Some(x) = self.cap_xkb.as_mut() {
            x.reset();
        }
    }

    fn end_capture(&mut self) {
        if let Some(i) = self.capture.take() {
            if let Some(l) = self.layers.get(i) {
                l.layer.set_keyboard_interactivity(KeyboardInteractivity::None);
                self.make_click_through(&l.layer);
                l.layer.commit();
            }
        }
        self.dirty = true;
    }

    /// One pass of non-blocking work between waits.
    fn tick(&mut self, qh: &QueueHandle<Self>) {
        if self.core.pump() {
            self.dirty = true;
        }
        if self.monitors_dirty {
            self.rebuild_monitors();
        }
        self.process_actions();
        // KWin pushes positions to us (and wakes the loop); apply the newest one.
        if matches!(self.source, Source::Kwin(_)) {
            self.sync_pointer();
        }
        // Hyprland can be asked for the position; poll it when evdev cannot tell us it moved.
        let now = self.core.now();
        if matches!(self.source, Source::Hyprland(_)) && self.devices.is_empty() && now >= self.next_pos_poll {
            self.next_pos_poll = now + 0.033;
            self.sync_pointer();
        }
        let min_interval = 1.0 / self.core.cfg.general.fps as f64;
        let due = self.dirty || self.next_frame.is_some_and(|t| now >= t);
        if due && now - self.last_frame >= min_interval {
            if !self.dirty && matches!(self.source, Source::Hyprland(_)) {
                self.sync_pointer();
            }
            self.frame(qh);
            self.process_actions();
        }
        if self.core.quit {
            self.exit = true;
        }
    }

    fn wait_timeout(&self) -> f64 {
        let now = self.core.now();
        let mut t = 0.5f64;
        let min_interval = 1.0 / self.core.cfg.general.fps as f64;
        if self.dirty {
            t = t.min((self.last_frame + min_interval - now).max(0.0));
        } else if let Some(n) = self.next_frame {
            t = t.min((n - now).max(0.0));
        }
        if matches!(self.source, Source::Hyprland(_)) && self.devices.is_empty() {
            t = t.min((self.next_pos_poll - now).max(0.0));
        }
        t
    }
}

pub fn run(core: Core, wake: &OwnedFd) -> Result<()> {
    let conn = Connection::connect_to_env().context("cannot connect to the Wayland compositor (is WAYLAND_DISPLAY set?)")?;
    let (globals, mut queue) = registry_queue_init(&conn).context("reading the Wayland registry")?;
    let qh = queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).context("compositor lacks wl_compositor")?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| {
        anyhow!(
            "this compositor does not support wlr-layer-shell, which Sourizz needs to draw its overlay.\n\
             GNOME (Mutter) is the usual case, and its Wayland session is not supported. Log in with a \"GNOME on Xorg\" session if your GNOME \
             still offers one (GNOME 49 and later do not), or use KDE Plasma, Hyprland, sway, …"
        )
    })?;
    let shm = Shm::bind(&globals, &qh).context("compositor lacks wl_shm")?;
    let pool = SlotPool::new(1 << 20, &shm).context("creating the shared-memory pool")?;

    let devices = Devices::scan();
    if devices.is_empty() {
        log::warn!(
            "no readable input devices ({} denied): global clicks and keystrokes are unavailable. \
             Add yourself to the `input` group and log in again — see the README.",
            devices.denied
        );
    }
    let (layout, variant, options) = xkb::detect_layout();
    let kb_xkb = Xkb::from_names(&layout, &variant, &options);
    if kb_xkb.is_none() {
        log::warn!("libxkbcommon not available: keystroke labels and hotkeys from the keyboard are disabled");
    }
    let source = if let Some(h) = Hyprland::detect() {
        log::info!("pointer position: Hyprland IPC");
        Source::Hyprland(h)
    } else if let Some(k) = KwinFeed::start(core.hub()) {
        log::info!("pointer position: KWin script");
        Source::Kwin(k)
    } else {
        log::warn!("pointer position: estimated from relative mouse motion (approximate; exact on KDE Plasma and Hyprland)");
        Source::Estimate
    };

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        shm,
        pool,
        layers: Vec::new(),
        core,
        pointer: None,
        keyboard: None,
        capture: None,
        cap_xkb: None,
        devices,
        kb_xkb,
        source,
        estimate: Estimate::new(IRect::new(0, 0, 1920, 1080)),
        hotkeys: Vec::new(),
        hotkeys_for: crate::config::Hotkeys { toggle_all: "\0".into(), ..Default::default() },
        last_pos: None,
        moved: false,
        dirty: true,
        next_frame: None,
        last_frame: f64::NEG_INFINITY,
        next_pos_poll: 0.0,
        monitors_dirty: false,
        exit: false,
    };

    // Learn the outputs and get the first configure events.
    queue.roundtrip(&mut app)?;
    queue.roundtrip(&mut app)?;
    if app.layers.is_empty() {
        return Err(anyhow!("the compositor reported no outputs"));
    }
    log::info!("Wayland backend ready ({} output(s))", app.layers.len());
    // Diagnostic hook: pretend the pointer is at x,y (useful on a compositor without input devices).
    if let Some((x, y)) = std::env::var("SOURIZZ_FAKE_POINTER").ok().and_then(|v| {
        let (a, b) = v.split_once(',')?;
        Some((a.trim().parse::<f32>().ok()?, b.trim().parse::<f32>().ok()?))
    }) {
        app.rebuild_monitors();
        app.estimate.sync((x, y));
        app.feed(InputEvent::Pointer { x, y });
    }

    let mut last_scan = 0.0;
    while !app.exit {
        queue.dispatch_pending(&mut app)?;
        app.tick(&qh);
        if app.exit {
            break;
        }
        conn.flush()?;
        let Some(guard) = queue.prepare_read() else { continue };

        let timeout = app.wait_timeout();
        let ts = Timespec { tv_sec: timeout as i64, tv_nsec: (timeout.fract() * 1e9) as _ };
        let wl_fd: BorrowedFd = guard.connection_fd();
        let dev_fds = app.devices.fds();
        let mut fds = vec![PollFd::new(&wl_fd, PollFlags::IN), PollFd::new(wake, PollFlags::IN)];
        fds.extend(dev_fds.iter().map(|f| PollFd::new(f, PollFlags::IN)));
        let _ = poll(&mut fds, Some(&ts));
        let ready: Vec<bool> = fds.iter().map(|f| f.revents().intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR)).collect();
        drop(fds);
        drop(dev_fds);

        if ready[0] {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e).context("lost the connection to the compositor"),
            }
        } else {
            drop(guard);
        }
        if ready[1] {
            control::drain(wake);
        }
        // Highest index first: a device that vanished is removed from the list while reading.
        for (n, r) in ready.iter().enumerate().skip(2).rev() {
            if *r {
                let raws = app.devices.read(n - 2);
                app.handle_raw(raws);
            }
        }
        let now = app.core.now();
        if now - last_scan > 3.0 {
            last_scan = now;
            app.devices.rescan();
        }
    }
    app.end_capture();
    Ok(())
}
