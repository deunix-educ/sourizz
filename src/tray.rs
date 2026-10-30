//! System tray icon (StatusNotifierItem, the KDE/freedesktop standard).
//! Works natively on KDE, XFCE, Cinnamon, MATE…; GNOME needs the AppIndicator extension.

use crate::config::*;
use crate::control::{Command, Hub};
use crate::i18n::{Lang, opt, t};
use ksni::menu::{CheckmarkItem, MenuItem, RadioGroup, RadioItem, StandardItem, SubMenu};
use tiny_skia::{Color as Sk, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

pub struct TrayIcon {
    pub cfg: Config,
    pub lang: Lang,
    pub hub: Hub,
}

/// 32×32 ring icon drawn at runtime so no icon theme or asset file is needed.
fn icon(size: u32) -> ksni::Icon {
    let mut pm = Pixmap::new(size, size).unwrap();
    let s = size as f32;
    let mut bg = Paint::default();
    bg.set_color(Sk::from_rgba8(30, 32, 40, 235));
    bg.anti_alias = true;
    if let Some(p) = PathBuilder::from_circle(s / 2.0, s / 2.0, s * 0.47) {
        pm.fill_path(&p, &bg, FillRule::Winding, Transform::identity(), None);
    }
    let mut ring = Paint::default();
    ring.set_color(Sk::from_rgba8(255, 193, 7, 255));
    ring.anti_alias = true;
    if let Some(p) = PathBuilder::from_circle(s / 2.0, s / 2.0, s * 0.27) {
        let st = Stroke { width: s * 0.11, ..Stroke::default() };
        pm.stroke_path(&p, &ring, &st, Transform::identity(), None);
    }
    // StatusNotifierItem wants straight (non-premultiplied) ARGB32, big-endian.
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for px in pm.pixels() {
        let c = px.demultiply();
        data.extend_from_slice(&[c.alpha(), c.red(), c.green(), c.blue()]);
    }
    ksni::Icon { width: size as i32, height: size as i32, data }
}

impl TrayIcon {
    fn check(&self, key: &'static str, checked: bool, effect: &'static str) -> MenuItem<Self> {
        CheckmarkItem {
            label: t(self.lang, key).into(),
            checked,
            activate: Box::new(move |this: &mut Self| this.hub.send(Command::Toggle(effect.into()))),
            ..Default::default()
        }
        .into()
    }

    fn radio<T: Copy + PartialEq + Send + Sync + 'static>(
        &self,
        title: &'static str,
        all: &'static [T],
        current: T,
        id: fn(T) -> &'static str,
        key: &'static str,
    ) -> MenuItem<Self> {
        let lang = self.lang;
        SubMenu {
            label: t(lang, title).into(),
            submenu: vec![
                RadioGroup {
                    selected: all.iter().position(|v| *v == current).unwrap_or(0),
                    select: Box::new(move |this: &mut Self, i| {
                        if let Some(v) = all.get(i) {
                            this.hub.send(Command::SetOption(key.into(), id(*v).into()));
                        }
                    }),
                    options: all.iter().map(|v| RadioItem { label: opt(lang, id(*v)), ..Default::default() }).collect(),
                }
                .into(),
            ],
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for TrayIcon {
    fn id(&self) -> String {
        "sourizz".into()
    }

    fn title(&self) -> String {
        "Sourizz".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![icon(22), icon(32), icon(48)]
    }

    fn icon_name(&self) -> String {
        String::new()
    }

    /// Left click toggles everything.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.hub.send(Command::Toggle("all".into()));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let c = &self.cfg;
        let l = self.lang;
        vec![
            self.check("enabled", c.general.enabled, "all"),
            MenuItem::Separator,
            self.check("spotlight", c.spotlight.enabled, "spotlight"),
            self.check("ring", c.ring.enabled, "ring"),
            self.check("trail", c.trail.enabled, "trail"),
            self.check("clicks", c.clicks.enabled, "clicks"),
            self.check("sound", c.sound.enabled, "sound"),
            self.check("shake", c.shake.enabled, "shake"),
            self.check("annotate", c.annotate.enabled, "annotate"),
            self.check("keystrokes", c.keystrokes.enabled, "keystrokes"),
            self.check("focus", c.focus.enabled, "focus"),
            MenuItem::Separator,
            SubMenu {
                label: t(l, "styles").into(),
                submenu: vec![
                    self.radio("spot_shape", SpotShape::ALL, c.spotlight.shape, SpotShape::id, "spotlight.shape"),
                    self.radio("ring_style", RingStyle::ALL, c.ring.style, RingStyle::id, "ring.style"),
                    self.radio("trail_style", TrailStyle::ALL, c.trail.style, TrailStyle::id, "trail.style"),
                    self.radio("click_style", ClickStyle::ALL, c.clicks.style, ClickStyle::id, "clicks.style"),
                    self.radio("key_style", KeyStyle::ALL, c.keystrokes.style, KeyStyle::id, "keystrokes.style"),
                ],
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: t(l, "annotation_mode").into(),
                activate: Box::new(|this: &mut Self| this.hub.send(Command::AnnotateToggle)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: t(l, "clear_annotations").into(),
                activate: Box::new(|this: &mut Self| this.hub.send(Command::AnnotateClear)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: t(l, "settings").into(),
                icon_name: "preferences-system".into(),
                activate: Box::new(|this: &mut Self| this.hub.send(Command::Settings)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: t(l, "quit").into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|this: &mut Self| this.hub.send(Command::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}
