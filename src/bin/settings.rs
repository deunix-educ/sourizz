//! Settings window. It edits `~/.config/sourizz/config.toml`; the running daemon notices the
//! file change and applies it live, so every slider acts as a preview.

use eframe::egui;
use sourizz::config::*;
use sourizz::i18n::{Lang, opt, t};
use sourizz::ipc;
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Effects,
    Clicks,
    Draw,
    Keys,
    Hotkeys,
    General,
}

struct Settings {
    cfg: Config,
    saved: Config,
    tab: Tab,
    lang: Lang,
    last_save: Instant,
    last_poll: Instant,
    file_mtime: Option<SystemTime>,
    daemon_up: bool,
    save_error: Option<String>,
}

impl Settings {
    fn new() -> Self {
        let cfg = Config::load();
        Self {
            lang: Lang::resolve(&cfg.general.language),
            saved: cfg.clone(),
            cfg,
            tab: Tab::Effects,
            last_save: Instant::now(),
            last_poll: Instant::now() - Duration::from_secs(10),
            file_mtime: Config::mtime(),
            daemon_up: ipc::is_running(),
            save_error: None,
        }
    }

    fn save_if_changed(&mut self) {
        if self.cfg == self.saved || self.last_save.elapsed() < Duration::from_millis(120) {
            return;
        }
        self.cfg.sanitize();
        match self.cfg.save() {
            Ok(()) => {
                self.saved = self.cfg.clone();
                self.file_mtime = Config::mtime();
                self.save_error = None;
            }
            Err(e) => self.save_error = Some(e.to_string()),
        }
        self.last_save = Instant::now();
    }

    /// Pick up edits made elsewhere (tray toggles, hotkeys, a text editor) when we have none pending.
    fn poll_external(&mut self) {
        if self.last_poll.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last_poll = Instant::now();
        self.daemon_up = ipc::is_running();
        let m = Config::mtime();
        if m != self.file_mtime && self.cfg == self.saved {
            self.file_mtime = m;
            let fresh = Config::load();
            self.saved = fresh.clone();
            self.cfg = fresh;
        }
        self.lang = Lang::resolve(&self.cfg.general.language);
    }
}

// ------------------------------------------------------------------ small widgets

fn slider(ui: &mut egui::Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>) {
    ui.horizontal(|ui| {
        ui.add(egui::Slider::new(v, range).text(label));
    });
}

fn slider_u32(ui: &mut egui::Ui, label: &str, v: &mut u32, range: std::ops::RangeInclusive<u32>) {
    ui.horizontal(|ui| {
        ui.add(egui::Slider::new(v, range).text(label));
    });
}

fn color(ui: &mut egui::Ui, label: &str, c: &mut Color) {
    ui.horizontal(|ui| {
        let mut rgb = [c.r, c.g, c.b];
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            (c.r, c.g, c.b) = (rgb[0], rgb[1], rgb[2]);
        }
        ui.label(label);
    });
}

/// `id` must be unique on screen: egui treats two combo boxes with the same id as one control
/// (their popups close each other), which made the ring and trail style lists impossible to open
/// when both used the same "Style" label as their id.
fn combo<T: Copy + PartialEq + 'static>(
    ui: &mut egui::Ui,
    lang: Lang,
    id: &str,
    label: &str,
    v: &mut T,
    all: &[T],
    name: fn(T) -> &'static str,
) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(id).selected_text(opt(lang, name(*v))).show_ui(ui, |ui| {
            for &x in all {
                ui.selectable_value(v, x, opt(lang, name(x)));
            }
        });
        ui.label(label);
    });
}

fn text(ui: &mut egui::Ui, label: &str, s: &mut String) {
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(s).desired_width(220.0));
        ui.label(label);
    });
}

fn section<R>(ui: &mut egui::Ui, title: &str, enabled: &mut bool, body: impl FnOnce(&mut egui::Ui) -> R) {
    ui.add_space(6.0);
    ui.group(|ui| {
        ui.checkbox(enabled, egui::RichText::new(title).strong().size(15.0));
        ui.add_enabled_ui(*enabled, |ui| {
            ui.add_space(2.0);
            body(ui);
        });
    });
}

// ------------------------------------------------------------------ tabs

impl Settings {
    fn tab_effects(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let c = &mut self.cfg;
        section(ui, t(l, "spotlight"), &mut c.spotlight.enabled, |ui| {
            combo(ui, l, "spotlight.shape", t(l, "f.shape"), &mut c.spotlight.shape, SpotShape::ALL, SpotShape::id);
            slider(ui, t(l, "f.radius"), &mut c.spotlight.radius, 20.0..=800.0);
            slider(ui, t(l, "f.opacity"), &mut c.spotlight.opacity, 0.0..=1.0);
            slider(ui, t(l, "f.softness"), &mut c.spotlight.softness, 0.0..=1.0);
            color(ui, t(l, "f.color"), &mut c.spotlight.color);
        });
        section(ui, t(l, "ring"), &mut c.ring.enabled, |ui| {
            combo(ui, l, "ring.style", t(l, "f.style"), &mut c.ring.style, RingStyle::ALL, RingStyle::id);
            slider(ui, t(l, "f.size"), &mut c.ring.size, 16.0..=300.0);
            slider(ui, t(l, "f.thickness"), &mut c.ring.thickness, 1.0..=24.0);
            slider(ui, t(l, "f.opacity"), &mut c.ring.opacity, 0.0..=1.0);
            slider(ui, t(l, "f.fill"), &mut c.ring.fill, 0.0..=1.0);
            slider(ui, t(l, "f.pulse"), &mut c.ring.pulse_speed, 0.0..=4.0);
            color(ui, t(l, "f.color"), &mut c.ring.color);
        });
        section(ui, t(l, "trail"), &mut c.trail.enabled, |ui| {
            combo(ui, l, "trail.style", t(l, "f.style"), &mut c.trail.style, TrailStyle::ALL, TrailStyle::id);
            slider(ui, t(l, "f.size"), &mut c.trail.size, 4.0..=60.0);
            slider_u32(ui, t(l, "f.lifetime"), &mut c.trail.lifetime_ms, 150..=3000);
            color(ui, t(l, "f.color"), &mut c.trail.color);
        });
        section(ui, t(l, "shake"), &mut c.shake.enabled, |ui| {
            slider(ui, t(l, "f.sensitivity"), &mut c.shake.sensitivity, 0.0..=1.0);
            slider(ui, t(l, "f.scale"), &mut c.shake.scale, 1.5..=6.0);
            slider_u32(ui, t(l, "f.duration"), &mut c.shake.duration_ms, 400..=5000);
        });
    }

    fn tab_clicks(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let c = &mut self.cfg;
        section(ui, t(l, "clicks"), &mut c.clicks.enabled, |ui| {
            ui.checkbox(&mut c.clicks.show_on_clicks, t(l, "f.show_on_clicks"));
            combo(ui, l, "clicks.style", t(l, "f.style"), &mut c.clicks.style, ClickStyle::ALL, ClickStyle::id);
            slider(ui, t(l, "f.size"), &mut c.clicks.size, 24.0..=300.0);
            slider(ui, t(l, "f.thickness"), &mut c.clicks.thickness, 1.0..=20.0);
            slider_u32(ui, t(l, "f.duration"), &mut c.clicks.duration_ms, 200..=2500);
            color(ui, t(l, "f.left"), &mut c.clicks.left_color);
            color(ui, t(l, "f.middle"), &mut c.clicks.middle_color);
            color(ui, t(l, "f.right"), &mut c.clicks.right_color);
            text(ui, t(l, "f.tap_key"), &mut c.clicks.tap_key);
        });
        section(ui, t(l, "sound"), &mut c.sound.enabled, |ui| {
            slider(ui, t(l, "f.volume"), &mut c.sound.volume, 0.0..=1.0);
            ui.checkbox(&mut c.sound.on_release, t(l, "f.on_release"));
            text(ui, t(l, "f.sound_file"), &mut c.sound.file);
        });
    }

    fn tab_draw(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let a = &mut self.cfg.annotate;
        section(ui, t(l, "annotate"), &mut a.enabled, |ui| {
            text(ui, t(l, "f.hold_key"), &mut a.hold_key);
            slider(ui, t(l, "f.width"), &mut a.width, 1.0..=30.0);
            color(ui, t(l, "f.color"), &mut a.color);
            ui.checkbox(&mut a.fade, t(l, "f.fade"));
            ui.add_enabled_ui(a.fade, |ui| slider_u32(ui, t(l, "f.fade_ms"), &mut a.fade_ms, 300..=10000));
        });
    }

    fn tab_keys(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let c = &mut self.cfg;
        section(ui, t(l, "keystrokes"), &mut c.keystrokes.enabled, |ui| {
            combo(ui, l, "keystrokes.style", t(l, "f.style"), &mut c.keystrokes.style, KeyStyle::ALL, KeyStyle::id);
            combo(ui, l, "keystrokes.position", t(l, "f.position"), &mut c.keystrokes.position, KeyPosition::ALL, KeyPosition::id);
            slider(ui, t(l, "f.size"), &mut c.keystrokes.size, 14.0..=72.0);
            slider_u32(ui, t(l, "f.duration"), &mut c.keystrokes.duration_ms, 500..=8000);
            color(ui, t(l, "f.accent"), &mut c.keystrokes.accent);
            ui.checkbox(&mut c.keystrokes.only_shortcuts, t(l, "f.only_shortcuts"));
        });
        section(ui, t(l, "focus"), &mut c.focus.enabled, |ui| {
            slider(ui, t(l, "f.opacity"), &mut c.focus.opacity, 0.0..=1.0);
            color(ui, t(l, "f.color"), &mut c.focus.color);
        });
    }

    fn tab_hotkeys(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let h = &mut self.cfg.hotkeys;
        ui.add_space(6.0);
        ui.label(egui::RichText::new(t(l, "hotkeys.help")).weak());
        ui.add_space(8.0);
        text(ui, t(l, "hk.toggle_all"), &mut h.toggle_all);
        text(ui, t(l, "spotlight"), &mut h.spotlight);
        text(ui, t(l, "ring"), &mut h.ring);
        text(ui, t(l, "trail"), &mut h.trail);
        text(ui, t(l, "clicks"), &mut h.clicks);
        text(ui, t(l, "keystrokes"), &mut h.keystrokes);
        text(ui, t(l, "focus"), &mut h.focus);
        text(ui, t(l, "hk.annotate_mode"), &mut h.annotate_mode);
        text(ui, t(l, "hk.clear_annotations"), &mut h.clear_annotations);
        text(ui, t(l, "hk.show_click"), &mut h.show_click);
    }

    fn tab_general(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let g = &mut self.cfg.general;
        ui.add_space(6.0);
        ui.checkbox(&mut g.enabled, egui::RichText::new(t(l, "enabled")).strong());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("lang").selected_text(g.language.clone()).show_ui(ui, |ui| {
                for v in ["auto", "en", "fr"] {
                    ui.selectable_value(&mut g.language, v.to_string(), v);
                }
            });
            ui.label(t(l, "f.language"));
        });
        combo(ui, l, "general.backend", t(l, "f.backend"), &mut g.backend, BackendKind::ALL, BackendKind::id);
        slider_u32(ui, t(l, "f.fps"), &mut g.fps, 15..=144);
        ui.add_space(12.0);
        if ui.button(t(l, "reset")).clicked() {
            self.cfg = Config::default();
        }
    }
}

impl eframe::App for Settings {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_external();
        let l = self.lang;

        egui::Panel::top("tabs").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (tab, key) in [
                    (Tab::Effects, "tab.effects"),
                    (Tab::Clicks, "tab.clicks"),
                    (Tab::Draw, "tab.draw"),
                    (Tab::Keys, "tab.keys"),
                    (Tab::Hotkeys, "tab.hotkeys"),
                    (Tab::General, "tab.general"),
                ] {
                    ui.selectable_value(&mut self.tab, tab, t(l, key));
                }
            });
            ui.add_space(4.0);
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(4.0);
            if let Some(e) = &self.save_error {
                ui.colored_label(egui::Color32::LIGHT_RED, e);
            } else if !self.daemon_up {
                ui.colored_label(egui::Color32::from_rgb(255, 190, 80), t(l, "daemon_down"));
            } else {
                ui.label(egui::RichText::new(t(l, "saved")).weak());
            }
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.tab {
                Tab::Effects => self.tab_effects(ui),
                Tab::Clicks => self.tab_clicks(ui),
                Tab::Draw => self.tab_draw(ui),
                Tab::Keys => self.tab_keys(ui),
                Tab::Hotkeys => self.tab_hotkeys(ui),
                Tab::General => self.tab_general(ui),
            });
        });

        self.save_if_changed();
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }
}

fn main() -> eframe::Result {
    let lang = Lang::resolve(&Config::load().general.language);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 700.0])
            .with_min_inner_size([420.0, 420.0])
            .with_title(t(lang, "app.title"))
            .with_app_id("sourizz-settings"),
        ..Default::default()
    };
    eframe::run_native("sourizz-settings", options, Box::new(|_cc| Ok(Box::new(Settings::new()))))
}
