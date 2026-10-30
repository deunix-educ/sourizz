//! Probe for compositor stalls: a bare layer-shell overlay that redraws a moving ring.
//!
//! It creates the same kind of surface as the Wayland backend (overlay layer, empty input region,
//! `wl_shm` ARGB8888 buffers, a fresh buffer per commit) but with the knobs that matter exposed, so
//! one can find out which of them makes a compositor stall (see "KWin Wayland stalls" in CLAUDE.md).
//! The ring follows its own path, so nothing has to move; move the real pointer to see whether the
//! arrow freezes. Prints, once a second, the commits and the longest commit-to-frame-callback delay.
//!
//! ```sh
//! cargo run --release --example layer_probe -- --size full --fps 60 --damage ring
//! cargo run --release --example layer_probe -- --size 400x400 --fps 60
//! ```

use anyhow::{Context, Result, anyhow};
use clap::{Parser, ValueEnum};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData, Region},
    delegate_registry,
    dispatch2::Dispatch2,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use sourizz::{
    control::{self, Hub},
    evdev_in::Devices,
    kwin::KwinFeed,
};
use std::time::{Duration, Instant};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
};

#[derive(Clone, Copy, ValueEnum, PartialEq)]
enum Damage {
    /// Report the whole surface as damaged on every commit.
    Full,
    /// Report only the box around the ring (what Sourizz does).
    Ring,
}

#[derive(Parser)]
#[command(about = "Layer-shell stall probe")]
struct Args {
    /// `full` (the whole output, like Sourizz) or `WIDTHxHEIGHT` anchored top-left.
    #[arg(long, default_value = "full")]
    size: String,
    /// Commits per second.
    #[arg(long, default_value_t = 60)]
    fps: u32,
    /// What to report as damaged.
    #[arg(long, value_enum, default_value = "ring")]
    damage: Damage,
    /// Commit on a timer without waiting for the compositor's frame callback.
    #[arg(long)]
    free_running: bool,
    /// Follow the real pointer (KDE only, through the same KWin script as Sourizz) and commit only
    /// when it moved, instead of running on a path of its own.
    #[arg(long)]
    follow: bool,
    /// Bind the seat's pointer and keyboard objects, as the Wayland backend does.
    #[arg(long)]
    seat: bool,
    /// Read the input devices in /dev/input (events discarded), as the Wayland backend does.
    #[arg(long)]
    evdev: bool,
    /// With --evdev: scan /dev/input once and never again (the backend rescans every 3 s).
    #[arg(long)]
    no_rescan: bool,
    /// Stop after this many seconds (0 = until interrupted).
    #[arg(long, default_value_t = 0)]
    seconds: u64,
}

const RING_R: f32 = 36.0;
const RING_BOX: i32 = 90;

struct Probe {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: Option<SeatState>,
    wl_pointer_obj: Option<wl_pointer::WlPointer>,
    wl_keyboard_obj: Option<wl_keyboard::WlKeyboard>,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    args: Args,
    fixed: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
    canvas: Vec<u8>,
    /// Box around the ring in the previous frame, to be cleared.
    prev_box: Option<(i32, i32)>,
    frame_pending: bool,
    committed_at: Instant,
    next_due: Instant,
    t0: Instant,
    stat_since: Instant,
    commits: u32,
    cb_max: Duration,
    exit: bool,
    feed: Option<KwinFeed>,
    pointer: Option<(f32, f32)>,
    dirty: bool,
    moves: u32,
}

impl Probe {
    fn ring_center(&self, w: i32, h: i32) -> (f32, f32) {
        if self.args.follow {
            return self.pointer.unwrap_or((w as f32 / 2.0, h as f32 / 2.0));
        }
        let t = self.t0.elapsed().as_secs_f32();
        let (rx, ry) = ((w as f32 / 2.0 - RING_BOX as f32).max(1.0), (h as f32 / 2.0 - RING_BOX as f32).max(1.0));
        (w as f32 / 2.0 + rx * (t * 2.3).sin(), h as f32 / 2.0 + ry * (t * 3.1 + 1.0).sin())
    }

    /// Paint the ring into `self.canvas` and return the damaged box (x, y, w, h), clipped.
    fn draw(&mut self, w: i32, h: i32) -> (i32, i32, i32, i32) {
        let (cx, cy) = self.ring_center(w, h);
        let (bx, by) =
            ((cx as i32 - RING_BOX / 2).clamp(0, (w - RING_BOX).max(0)), (cy as i32 - RING_BOX / 2).clamp(0, (h - RING_BOX).max(0)));
        let clip = |x: i32, y: i32| (x.max(0), y.max(0), (x + RING_BOX).min(w) - x.max(0), (y + RING_BOX).min(h) - y.max(0));
        if let Some((px, py)) = self.prev_box {
            let (x, y, bw, bh) = clip(px, py);
            for row in y..y + bh {
                let o = (row * w + x) as usize * 4;
                self.canvas[o..o + bw as usize * 4].fill(0);
            }
        }
        let (x, y, bw, bh) = clip(bx, by);
        for row in y..y + bh {
            for col in x..x + bw {
                let d = ((col as f32 + 0.5 - cx).powi(2) + (row as f32 + 0.5 - cy).powi(2)).sqrt();
                let a = (2.5 + 0.5 - (d - RING_R).abs()).clamp(0.0, 1.0) * 0.95;
                let o = (row * w + col) as usize * 4;
                // premultiplied BGRA of amber (255, 193, 7)
                self.canvas[o..o + 4].copy_from_slice(&[(7.0 * a) as u8, (193.0 * a) as u8, (255.0 * a) as u8, (255.0 * a) as u8]);
            }
        }
        // The damaged box is the union of the cleared and the painted one.
        let (ux, uy, uw, uh) = match self.prev_box {
            Some((px, py)) => {
                let (x0, y0) = (px.min(bx), py.min(by));
                let (x1, y1) = ((px + RING_BOX).max(bx + RING_BOX), (py + RING_BOX).max(by + RING_BOX));
                (x0.max(0), y0.max(0), x1.min(w) - x0.max(0), y1.min(h) - y0.max(0))
            }
            None => (x, y, bw, bh),
        };
        self.prev_box = Some((bx, by));
        (ux, uy, uw, uh)
    }

    fn commit(&mut self, qh: &QueueHandle<Self>) {
        let Some((w, h)) = self.size else { return };
        let dmg = self.draw(w, h);
        let Ok((buffer, slot)) = self.pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else {
            eprintln!("cannot allocate a {w}x{h} buffer");
            return;
        };
        slot.copy_from_slice(&self.canvas);
        let surface = self.layer.wl_surface().clone();
        match self.args.damage {
            Damage::Full => surface.damage_buffer(0, 0, w, h),
            Damage::Ring => surface.damage_buffer(dmg.0, dmg.1, dmg.2, dmg.3),
        }
        surface.frame(qh, FrameCallbackData(surface.clone()));
        if buffer.attach_to(&surface).is_ok() {
            self.layer.commit();
            self.frame_pending = true;
            self.committed_at = Instant::now();
            self.commits += 1;
        }
    }

    fn tick(&mut self, qh: &QueueHandle<Self>) {
        let now = Instant::now();
        if let Some(p) = self.feed.as_ref().and_then(|f| f.take_latest()) {
            self.pointer = Some(p);
            self.dirty = true;
            self.moves += 1;
        }
        let wanted = !self.args.follow || self.dirty;
        if self.size.is_some() && wanted && now >= self.next_due && (!self.frame_pending || self.args.free_running) {
            self.dirty = false;
            self.commit(qh);
            let step = Duration::from_secs_f64(1.0 / self.args.fps.max(1) as f64);
            self.next_due = (self.next_due + step).max(now);
        }
        if now.duration_since(self.stat_since) >= Duration::from_secs(1) {
            println!("positions={:3} commits={:3}  longest callback delay {:4} ms", self.moves, self.commits, self.cb_max.as_millis());
            self.moves = 0;
            self.commits = 0;
            self.cb_max = Duration::ZERO;
            self.stat_since = now;
        }
        if self.args.seconds > 0 && self.t0.elapsed() >= Duration::from_secs(self.args.seconds) {
            self.exit = true;
        }
    }

    fn timeout(&self) -> Duration {
        let wait = self.next_due.saturating_duration_since(Instant::now());
        if self.frame_pending && !self.args.free_running { Duration::from_millis(100) } else { wait.min(Duration::from_millis(100)) }
    }
}

impl CompositorHandler for Probe {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.cb_max = self.cb_max.max(self.committed_at.elapsed());
        self.frame_pending = false;
    }
}

impl OutputHandler for Probe {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Probe {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface, configure: LayerSurfaceConfigure, _serial: u32) {
        let (w, h) = match self.fixed {
            Some(s) => s,
            None => (configure.new_size.0 as i32, configure.new_size.1 as i32),
        };
        if w == 0 || h == 0 || self.size == Some((w, h)) {
            return;
        }
        eprintln!("surface {w}x{h}");
        self.size = Some((w, h));
        self.canvas = vec![0; w as usize * h as usize * 4];
        self.prev_box = None;
    }
}

/// Routes `wl_keyboard` events (ignored) to [`Dispatch2`].
struct KbData;

impl Dispatch2<wl_keyboard::WlKeyboard, Probe> for KbData {
    fn event(&self, _: &mut Probe, _: &wl_keyboard::WlKeyboard, _: wl_keyboard::Event, _: &Connection, _: &QueueHandle<Probe>) {}
}

impl SeatHandler for Probe {
    fn seat_state(&mut self) -> &mut SeatState {
        self.seat_state.as_mut().expect("seat state is created with --seat")
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Pointer && self.wl_pointer_obj.is_none() {
            self.wl_pointer_obj = self.seat_state.as_mut().and_then(|s| s.get_pointer(qh, &seat).ok());
        }
        if cap == Capability::Keyboard && self.wl_keyboard_obj.is_none() {
            self.wl_keyboard_obj = Some(seat.get_keyboard(qh, KbData));
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, _: Capability) {}
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for Probe {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, _: &[PointerEvent]) {}
}

impl ShmHandler for Probe {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(Probe);
smithay_client_toolkit::delegate_dispatch2!(Probe);

impl ProvidesRegistryState for Probe {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

fn main() -> Result<()> {
    let args = Args::parse();
    let app_args_follow = args.follow;
    let app_args_seat = args.seat;
    let no_rescan = args.no_rescan;
    let mut devices = if args.evdev { Some(Devices::scan()) } else { None };
    let mut last_scan = Instant::now();
    let fixed = match args.size.as_str() {
        "full" => None,
        s => {
            let (w, h) = s.split_once('x').ok_or_else(|| anyhow!("--size is `full` or WIDTHxHEIGHT"))?;
            Some((w.parse::<i32>()?, h.parse::<i32>()?))
        }
    };
    let conn = Connection::connect_to_env().context("cannot connect to the Wayland compositor")?;
    let (globals, mut queue) = registry_queue_init(&conn)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| anyhow!("the compositor lacks wlr-layer-shell"))?;
    let shm = Shm::bind(&globals, &qh)?;
    let pool = SlotPool::new(1 << 20, &shm)?;

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("layer-probe"), None);
    match fixed {
        None => {
            layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
            layer.set_size(0, 0);
        }
        Some((w, h)) => {
            layer.set_anchor(Anchor::TOP | Anchor::LEFT);
            layer.set_size(w as u32, h as u32);
        }
    }
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_input_region(Some(Region::new(&compositor)?.wl_region()));
    layer.commit();

    let (hub, _commands, wake) = Hub::new()?;
    let feed = if app_args_follow { KwinFeed::start(hub) } else { None };
    if app_args_follow && feed.is_none() {
        return Err(anyhow!("--follow needs KDE Plasma (the KWin pointer script could not be loaded)"));
    }
    let now = Instant::now();
    let mut app = Probe {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: if app_args_seat { Some(SeatState::new(&globals, &qh)) } else { None },
        wl_pointer_obj: None,
        wl_keyboard_obj: None,
        shm,
        pool,
        layer,
        args,
        fixed,
        size: None,
        canvas: Vec::new(),
        prev_box: None,
        frame_pending: false,
        committed_at: now,
        next_due: now,
        t0: now,
        stat_since: now,
        commits: 0,
        cb_max: Duration::ZERO,
        exit: false,
        feed,
        pointer: None,
        dirty: false,
        moves: 0,
    };
    println!(
        "size={} fps={} damage={} {}",
        app.args.size,
        app.args.fps,
        if app.args.damage == Damage::Full { "full" } else { "ring" },
        if app.args.free_running { "free-running" } else { "paced by frame callbacks" }
    );

    while !app.exit {
        queue.dispatch_pending(&mut app)?;
        app.tick(&qh);
        conn.flush()?;
        let Some(guard) = queue.prepare_read() else { continue };
        let t = app.timeout();
        let ts = Timespec { tv_sec: t.as_secs() as i64, tv_nsec: t.subsec_nanos() as _ };
        let fd = guard.connection_fd();
        let dev_fds = devices.as_ref().map(|d| d.fds()).unwrap_or_default();
        let mut fds = vec![PollFd::new(&fd, PollFlags::IN), PollFd::new(&wake, PollFlags::IN)];
        fds.extend(dev_fds.iter().map(|f| PollFd::new(f, PollFlags::IN)));
        let _ = poll(&mut fds, Some(&ts));
        let ready: Vec<bool> = fds.iter().map(|f| f.revents().intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR)).collect();
        drop(fds);
        drop(dev_fds);
        if ready[1] {
            control::drain(&wake);
        }
        if let Some(d) = devices.as_mut() {
            for (n, r) in ready.iter().enumerate().skip(2).rev() {
                if *r {
                    let _ = d.read(n - 2);
                }
            }
            if !no_rescan && last_scan.elapsed() > Duration::from_secs(3) {
                last_scan = Instant::now();
                d.rescan();
                eprintln!("rescan of /dev/input took {} ms", last_scan.elapsed().as_millis());
            }
        }
        if ready[0] {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e).context("lost the connection to the compositor"),
            }
        } else {
            drop(guard);
        }
    }
    Ok(())
}
