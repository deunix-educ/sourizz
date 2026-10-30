//! Backend-independent daemon core: owns the engine and the live config, applies
//! commands, hot-reloads the config file, plays sounds and keeps the tray in sync.

use crate::audio::Audio;
use crate::config::Config;
use crate::control::{self, Command, Hub};
use crate::engine::Engine;
use crate::i18n::Lang;
use crate::input::{Action, InputEvent};
use crate::text::Fonts;
use crate::tray::TrayIcon;
use std::process::Child;
use std::sync::mpsc::Receiver;
use std::time::{Instant, SystemTime};

pub struct Core {
    pub engine: Engine,
    pub cfg: Config,
    pub quit: bool,
    rx: Receiver<Command>,
    hub: Hub,
    start: Instant,
    cfg_mtime: Option<SystemTime>,
    next_cfg_check: f64,
    audio: Audio,
    tray: Option<ksni::blocking::Handle<TrayIcon>>,
    settings: Option<Child>,
    term: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Set whenever a command or config reload may have changed what must be drawn.
    changed: bool,
}

impl Core {
    pub fn new(cfg: Config, hub: Hub, rx: Receiver<Command>) -> Self {
        Self {
            engine: Engine::new(cfg.clone(), Fonts::load()),
            cfg_mtime: Config::mtime(),
            cfg,
            quit: false,
            rx,
            hub,
            start: Instant::now(),
            next_cfg_check: 0.0,
            audio: Audio::new(),
            tray: None,
            settings: None,
            term: None,
            changed: false,
        }
    }

    /// Quit when this flag is set (SIGINT/SIGTERM handlers raise it).
    pub fn set_term_flag(&mut self, f: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        self.term = Some(f);
    }

    /// Seconds on the daemon's monotonic clock.
    pub fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    pub fn hub(&self) -> Hub {
        self.hub.clone()
    }

    pub fn start_tray(&mut self) {
        use ksni::blocking::TrayMethods;
        let tray = TrayIcon { cfg: self.cfg.clone(), lang: Lang::resolve(&self.cfg.general.language), hub: self.hub.clone() };
        match tray.spawn() {
            Ok(h) => self.tray = Some(h),
            Err(e) => log::warn!("no system tray ({e}); control the daemon with the `sourizz` CLI or hotkeys"),
        }
    }

    fn sync_tray(&self) {
        if let Some(h) = &self.tray {
            let (cfg, lang) = (self.cfg.clone(), Lang::resolve(&self.cfg.general.language));
            let _ = h.update(move |t: &mut TrayIcon| {
                t.cfg = cfg;
                t.lang = lang;
            });
        }
    }

    /// Process queued commands and reload the config file if it changed. Call every loop turn.
    /// Returns true when something happened that needs a repaint.
    pub fn pump(&mut self) -> bool {
        if self.term.as_ref().is_some_and(|t| t.load(std::sync::atomic::Ordering::Relaxed)) {
            self.quit = true;
        }
        while let Ok(cmd) = self.rx.try_recv() {
            self.apply(cmd);
        }
        let now = self.now();
        if now >= self.next_cfg_check {
            self.next_cfg_check = now + 0.4;
            let m = Config::mtime();
            if m != self.cfg_mtime {
                self.cfg_mtime = m;
                self.reload();
            }
        }
        std::mem::take(&mut self.changed)
    }

    fn reload(&mut self) {
        let cfg = Config::load();
        if cfg != self.cfg {
            log::info!("config reloaded");
            self.changed = true;
            self.cfg = cfg;
            self.engine.set_config(self.cfg.clone());
            self.sync_tray();
        }
    }

    /// Apply the in-memory config to the engine and persist it.
    fn commit(&mut self) {
        self.cfg.sanitize();
        self.engine.set_config(self.cfg.clone());
        if let Err(e) = self.cfg.save() {
            log::warn!("cannot save config: {e}");
        }
        self.cfg_mtime = Config::mtime();
        self.sync_tray();
    }

    pub fn apply(&mut self, cmd: Command) {
        self.changed = true;
        let now = self.now();
        match cmd {
            Command::Toggle(name) => match control::flag(&mut self.cfg, &name) {
                Some(f) => {
                    *f = !*f;
                    self.commit();
                }
                None => log::warn!("unknown effect {name}"),
            },
            Command::Set(name, v) => match control::flag(&mut self.cfg, &name) {
                Some(f) if *f != v => {
                    *f = v;
                    self.commit();
                }
                Some(_) => {}
                None => log::warn!("unknown effect {name}"),
            },
            Command::SetOption(k, v) => match control::set_option(&self.cfg, &k, &v) {
                Ok(c) => {
                    self.cfg = c;
                    self.commit();
                }
                Err(e) => log::warn!("set {k}: {e}"),
            },
            Command::ShowClick => self.engine.show_click(now),
            Command::AnnotateToggle => self.engine.toggle_annotation_mode(),
            Command::AnnotateClear => self.engine.clear_annotations(),
            Command::Reload => self.reload(),
            Command::Status(tx) => {
                let _ = tx.send(format!("running {} backend-config={}", self.engine.summary(), self.cfg.general.backend.id()));
            }
            Command::Settings => self.open_settings(),
            Command::Quit => self.quit = true,
        }
    }

    pub fn handle_input(&mut self, ev: InputEvent) {
        let now = self.now();
        self.engine.handle(now, ev);
    }

    /// Run the sound side effects and hand everything else (capture changes) to the backend.
    pub fn drain_actions(&mut self) -> Vec<Action> {
        let mut rest = Vec::new();
        for a in self.engine.take_actions() {
            match a {
                Action::PlayClick(b, pressed) => self.audio.play(&self.cfg.sound, b, pressed),
                other => rest.push(other),
            }
        }
        rest
    }

    fn open_settings(&mut self) {
        // A window that is still open may be buried under others, and KWin's focus-stealing prevention
        // only makes the taskbar entry blink when another process asks it to rise: replace it with a
        // fresh one, which always appears on top. The settings are saved as they change.
        if let Some(mut c) = self.settings.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let sibling = std::env::current_exe().ok().map(|p| p.with_file_name("sourizz-settings")).filter(|p| p.is_file());
        let program = sibling.unwrap_or_else(|| "sourizz-settings".into());
        match std::process::Command::new(&program).spawn() {
            Ok(c) => self.settings = Some(c),
            Err(e) => log::warn!("cannot start {}: {e} (is the settings window installed?)", program.display()),
        }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        if let Some(c) = &mut self.settings {
            let _ = c.kill();
        }
    }
}
