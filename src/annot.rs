//! Screen annotations.
//!
//! Two ways to draw:
//! * **hold key** (config `annotate.hold_key`): hold the key and move the pointer.
//!   Needs no input capture, so it works on every backend. Strokes fade after release.
//! * **annotation mode** (hotkey): the overlay captures pointer and keyboard and shows a
//!   toolbar with pen, highlighter, text, numbered callouts and an eraser.

use crate::config::{Annotate, Color};
use crate::fx;
use crate::geom::IRect;
use crate::input::{Button, KeyEvent};
use crate::text::Fonts;
use tiny_skia::{LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Highlighter,
    Text,
    Callout,
    Eraser,
}

#[derive(Clone, Debug)]
pub enum Item {
    Stroke {
        pts: Vec<(f32, f32)>,
        color: Color,
        width: f32,
        highlighter: bool,
        /// Set when a fading stroke is released.
        released_at: Option<f64>,
        fade: bool,
        bbox: IRect,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        color: Color,
        size: f32,
    },
    Callout {
        x: f32,
        y: f32,
        n: u32,
        color: Color,
    },
}

const CALLOUT_R: f32 = 16.0;

impl Item {
    fn bbox(&self, fonts: &mut Fonts) -> IRect {
        match self {
            Item::Stroke { bbox, .. } => *bbox,
            Item::Text { x, y, text, size, .. } => {
                let w = fonts.measure(text, *size).max(*size * 0.6) + 8.0;
                IRect::around(*x - 4.0, *y - *size, *x + w, *y + *size * 0.4, 4.0)
            }
            Item::Callout { x, y, .. } => IRect::centered(*x, *y, CALLOUT_R + 4.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Btn {
    Tool(Tool),
    Color(usize),
    Undo,
    Clear,
    Close,
}

pub const PALETTE: [Color; 6] = [
    Color::rgb(0xFF, 0x3B, 0x30),
    Color::rgb(0xFF, 0x95, 0x00),
    Color::rgb(0xFF, 0xEB, 0x3B),
    Color::rgb(0x34, 0xC7, 0x59),
    Color::rgb(0x0A, 0x84, 0xFF),
    Color::rgb(0xFF, 0xFF, 0xFF),
];

pub struct Annotator {
    pub items: Vec<Item>,
    /// Capture mode: overlay owns the pointer and keyboard.
    pub active: bool,
    pub tool: Tool,
    pub color_idx: usize,
    pub width: f32,
    drawing: bool,
    editing: Option<usize>,
    next_callout: u32,
    toolbar: IRect,
    buttons: Vec<(IRect, Btn)>,
    /// Index of the stroke currently being drawn with the hold key.
    hold_stroke: Option<usize>,
    close_requested: bool,
}

impl Default for Annotator {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            active: false,
            tool: Tool::Pen,
            color_idx: 0,
            width: 5.0,
            drawing: false,
            editing: None,
            next_callout: 1,
            toolbar: IRect::new(0, 0, 0, 0),
            buttons: Vec::new(),
            hold_stroke: None,
            close_requested: false,
        }
    }
}

fn stroke_bbox(pts: &[(f32, f32)], width: f32) -> IRect {
    let mut r = IRect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for &(x, y) in pts {
        r = r.union(IRect::centered(x, y, width * 0.5 + 2.0));
    }
    r
}

impl Annotator {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.hold_stroke = None;
        self.editing = None;
        self.next_callout = 1;
    }

    pub fn undo(&mut self) {
        self.editing = None;
        self.hold_stroke = None;
        if let Some(Item::Callout { .. }) = self.items.pop() {
            self.next_callout = self.next_callout.saturating_sub(1).max(1);
        }
    }

    fn color(&self) -> Color {
        PALETTE[self.color_idx % PALETTE.len()]
    }

    // ------------------------------------------------------------- hold-key drawing

    pub fn hold_begin(&mut self, cfg: &Annotate, pos: (f32, f32)) {
        self.hold_end(0.0, cfg);
        self.items.push(Item::Stroke {
            pts: vec![pos],
            color: cfg.color,
            width: cfg.width,
            highlighter: false,
            released_at: None,
            fade: cfg.fade,
            bbox: stroke_bbox(&[pos], cfg.width),
        });
        self.hold_stroke = Some(self.items.len() - 1);
    }

    pub fn hold_move(&mut self, pos: (f32, f32)) {
        if let Some(i) = self.hold_stroke {
            self.push_point(i, pos);
        }
    }

    pub fn hold_end(&mut self, now: f64, _cfg: &Annotate) {
        if let Some(i) = self.hold_stroke.take() {
            if let Some(Item::Stroke { released_at, .. }) = self.items.get_mut(i) {
                *released_at = Some(now);
            }
        }
    }

    /// True while a text box is being typed into (its caret blinks, so frames are needed).
    pub fn editing(&self) -> bool {
        self.active && self.editing.is_some()
    }

    pub fn holding(&self) -> bool {
        self.hold_stroke.is_some()
    }

    fn push_point(&mut self, i: usize, pos: (f32, f32)) {
        if let Some(Item::Stroke { pts, bbox, width, .. }) = self.items.get_mut(i) {
            if let Some(&(lx, ly)) = pts.last() {
                if (lx - pos.0).hypot(ly - pos.1) < 1.5 {
                    return;
                }
            }
            pts.push(pos);
            *bbox = bbox.union(IRect::centered(pos.0, pos.1, *width * 0.5 + 2.0));
        }
    }

    /// Drop strokes that finished fading. Returns true while something is still fading.
    pub fn expire(&mut self, now: f64, cfg: &Annotate) -> bool {
        let fade = cfg.fade_ms as f64 / 1000.0;
        let mut fading = false;
        self.items.retain(|it| match it {
            Item::Stroke { released_at: Some(t), fade: true, .. } => {
                fading = true;
                now - *t < fade
            }
            _ => true,
        });
        // Indices may have shifted; hold_stroke is only set for unreleased strokes, which are never removed.
        if let Some(i) = self.hold_stroke {
            if i >= self.items.len() {
                self.hold_stroke = None;
            }
        }
        if self.editing.is_some_and(|i| i >= self.items.len()) {
            self.editing = None;
        }
        fading
    }

    // ------------------------------------------------------------- capture mode

    pub fn begin_capture(&mut self, monitor: IRect, cfg: &Annotate) {
        self.active = true;
        self.drawing = false;
        self.editing = None;
        self.width = cfg.width;
        self.layout_toolbar(monitor);
    }

    pub fn end_capture(&mut self) {
        self.commit_text();
        self.active = false;
        self.drawing = false;
        self.buttons.clear();
        self.toolbar = IRect::new(0, 0, 0, 0);
    }

    fn layout_toolbar(&mut self, mon: IRect) {
        let (bw, gap, pad) = (38i32, 6i32, 10i32);
        let wide = 62i32;
        let tools = [Tool::Pen, Tool::Highlighter, Tool::Text, Tool::Callout, Tool::Eraser];
        let width = pad * 2 + tools.len() as i32 * (bw + gap) + 10 + PALETTE.len() as i32 * (30 + gap) + 10 + 3 * (wide + gap) - gap;
        let height = bw + pad * 2;
        let x0 = mon.x0 + (mon.width() - width) / 2;
        let y0 = mon.y0 + 18;
        self.toolbar = IRect::from_xywh(x0, y0, width, height);
        self.buttons.clear();
        let mut x = x0 + pad;
        let y = y0 + pad;
        for t in tools {
            self.buttons.push((IRect::from_xywh(x, y, bw, bw), Btn::Tool(t)));
            x += bw + gap;
        }
        x += 10;
        for i in 0..PALETTE.len() {
            self.buttons.push((IRect::from_xywh(x, y + 4, 30, 30), Btn::Color(i)));
            x += 30 + gap;
        }
        x += 10;
        for b in [Btn::Undo, Btn::Clear, Btn::Close] {
            self.buttons.push((IRect::from_xywh(x, y, wide, bw), b));
            x += wide + gap;
        }
    }

    pub fn pointer_down(&mut self, fonts: &mut Fonts, pos: (f32, f32), button: Button) {
        if button != Button::Left {
            return;
        }
        if self.toolbar.contains(pos.0, pos.1) {
            let hit = self.buttons.iter().find(|(r, _)| r.contains(pos.0, pos.1)).map(|(_, b)| *b);
            match hit {
                Some(Btn::Tool(t)) => {
                    self.commit_text();
                    self.tool = t;
                }
                Some(Btn::Color(i)) => self.color_idx = i,
                Some(Btn::Undo) => self.undo(),
                Some(Btn::Clear) => self.clear(),
                Some(Btn::Close) => self.close_requested = true,
                None => {}
            }
            return;
        }
        self.commit_text();
        match self.tool {
            Tool::Pen | Tool::Highlighter => {
                let hl = self.tool == Tool::Highlighter;
                let (mut color, width) = (self.color(), if hl { self.width * 4.5 } else { self.width });
                if hl {
                    color.a = 255;
                }
                self.items.push(Item::Stroke {
                    pts: vec![pos],
                    color,
                    width,
                    highlighter: hl,
                    released_at: None,
                    fade: false,
                    bbox: stroke_bbox(&[pos], width),
                });
                self.drawing = true;
            }
            Tool::Text => {
                self.items.push(Item::Text { x: pos.0, y: pos.1, text: String::new(), color: self.color(), size: 12.0 + self.width * 3.0 });
                self.editing = Some(self.items.len() - 1);
            }
            Tool::Callout => {
                self.items.push(Item::Callout { x: pos.0, y: pos.1, n: self.next_callout, color: self.color() });
                self.next_callout += 1;
            }
            Tool::Eraser => {
                self.drawing = true;
                self.erase_at(fonts, pos);
            }
        }
    }

    pub fn pointer_move(&mut self, fonts: &mut Fonts, pos: (f32, f32)) {
        if !self.drawing {
            return;
        }
        match self.tool {
            Tool::Pen | Tool::Highlighter => {
                if let Some(i) = self.items.len().checked_sub(1) {
                    self.push_point(i, pos);
                }
            }
            Tool::Eraser => self.erase_at(fonts, pos),
            _ => {}
        }
    }

    /// True once if the toolbar's Done button was pressed.
    pub fn take_close_request(&mut self) -> bool {
        std::mem::take(&mut self.close_requested)
    }

    pub fn pointer_up(&mut self, button: Button) {
        if button == Button::Left {
            self.drawing = false;
        }
    }

    fn erase_at(&mut self, fonts: &mut Fonts, pos: (f32, f32)) {
        let r = 14.0;
        let probe = IRect::centered(pos.0, pos.1, r);
        self.items.retain(|it| {
            let hit = match it {
                Item::Stroke { pts, width, .. } => pts.iter().any(|p| (p.0 - pos.0).hypot(p.1 - pos.1) < r + width * 0.5),
                other => !other.bbox(fonts).intersect(probe).is_empty(),
            };
            !hit
        });
        self.editing = None;
        self.hold_stroke = None;
    }

    fn commit_text(&mut self) {
        if let Some(i) = self.editing.take() {
            if matches!(self.items.get(i), Some(Item::Text { text, .. }) if text.is_empty()) {
                self.items.remove(i);
            }
        }
    }

    /// Returns true when the user asked to leave annotation mode.
    pub fn key(&mut self, ev: &KeyEvent) -> bool {
        if !ev.pressed {
            return false;
        }
        if let Some(i) = self.editing {
            match ev.name.as_str() {
                "Escape" => {
                    self.commit_text();
                    return false;
                }
                "Return" | "KP_Enter" => {
                    self.commit_text();
                    return false;
                }
                "BackSpace" => {
                    if let Some(Item::Text { text, .. }) = self.items.get_mut(i) {
                        text.pop();
                    }
                    return false;
                }
                _ => {
                    if let (Some(ch), Some(Item::Text { text, .. })) = (ev.text, self.items.get_mut(i)) {
                        if !ch.is_control() && ev.mods & !crate::config::MOD_SHIFT == 0 {
                            text.push(ch);
                        }
                    }
                    return false;
                }
            }
        }
        if ev.mods & crate::config::MOD_CTRL != 0 && ev.name.eq_ignore_ascii_case("z") {
            self.undo();
            return false;
        }
        match ev.name.as_str() {
            "Escape" => return true,
            "Delete" => self.clear(),
            "BackSpace" => self.undo(),
            "p" | "P" => self.tool = Tool::Pen,
            "h" | "H" => self.tool = Tool::Highlighter,
            "t" | "T" => self.tool = Tool::Text,
            "n" | "N" => self.tool = Tool::Callout,
            "e" | "E" => self.tool = Tool::Eraser,
            "plus" | "KP_Add" => self.width = (self.width + 1.0).min(40.0),
            "minus" | "KP_Subtract" => self.width = (self.width - 1.0).max(1.0),
            n if n.len() == 1 && ('1'..='6').contains(&n.chars().next().unwrap()) => {
                self.color_idx = n.chars().next().unwrap() as usize - '1' as usize;
            }
            _ => {}
        }
        false
    }

    // ------------------------------------------------------------- geometry & painting

    pub fn bounds(&self, fonts: &mut Fonts) -> IRect {
        let mut r = IRect::new(0, 0, 0, 0);
        for it in &self.items {
            r = r.union(it.bbox(fonts));
        }
        if self.active {
            r = r.union(self.toolbar.translate(0, 0));
            r = IRect::new(r.x0 - 8, r.y0 - 8, r.x1 + 8, r.y1 + 8);
        }
        r
    }

    pub fn paint(&mut self, pm: &mut Pixmap, tf: Transform, fonts: &mut Fonts, now: f64, cfg: &Annotate) {
        let fade = cfg.fade_ms as f32 / 1000.0;
        let editing = self.editing;
        for (i, it) in self.items.iter().enumerate() {
            match it {
                Item::Stroke { pts, color, width, highlighter, released_at, fade: fades, .. } => {
                    let mut alpha = 1.0;
                    if let (Some(t), true) = (released_at, *fades) {
                        alpha = (1.0 - (now - *t) as f32 / fade).clamp(0.0, 1.0);
                        alpha *= alpha.sqrt();
                    }
                    let (c, w) = if *highlighter { (color.sk(0.38 * alpha), *width) } else { (color.sk(alpha), *width) };
                    draw_stroke(pm, tf, pts, w, c, *highlighter);
                }
                Item::Text { x, y, text, color, size } => {
                    let shown = if editing == Some(i) {
                        let blink = if (now * 2.0) as i64 % 2 == 0 { "|" } else { " " };
                        format!("{text}{blink}")
                    } else {
                        text.clone()
                    };
                    let (ox, oy) = (tf.tx, tf.ty);
                    // Soft shadow keeps the text readable on any background.
                    fonts.draw(pm, &shown, *x + ox + 1.5, *y + oy + 1.5, *size, Color { r: 0, g: 0, b: 0, a: 140 }, 1.0);
                    fonts.draw(pm, &shown, *x + ox, *y + oy, *size, *color, 1.0);
                }
                Item::Callout { x, y, n, color } => {
                    fx::fill_circle(pm, tf, *x, *y, CALLOUT_R + 2.0, tiny_skia::Color::from_rgba8(255, 255, 255, 230));
                    fx::fill_circle(pm, tf, *x, *y, CALLOUT_R, color.sk(1.0));
                    let label = n.to_string();
                    let px = 18.0;
                    let w = fonts.measure(&label, px);
                    let lum = 0.299 * color.r as f32 + 0.587 * color.g as f32 + 0.114 * color.b as f32;
                    let ink = if lum > 150.0 { Color::rgb(20, 20, 20) } else { Color::rgb(255, 255, 255) };
                    fonts.draw(pm, &label, *x + tf.tx - w / 2.0, *y + tf.ty + px * 0.34, px, ink, 1.0);
                }
            }
        }
        if self.active {
            self.paint_toolbar(pm, tf, fonts);
        }
    }

    fn paint_toolbar(&self, pm: &mut Pixmap, tf: Transform, fonts: &mut Fonts) {
        let t = self.toolbar;
        let bg = fx::solid(tiny_skia::Color::from_rgba8(24, 24, 28, 232));
        fx::fill_rrect(pm, tf, t.x0 as f32, t.y0 as f32, t.width() as f32, t.height() as f32, 16.0, &bg);
        fx::stroke_rrect(
            pm,
            tf,
            t.x0 as f32 + 0.5,
            t.y0 as f32 + 0.5,
            t.width() as f32 - 1.0,
            t.height() as f32 - 1.0,
            16.0,
            1.0,
            tiny_skia::Color::from_rgba8(255, 255, 255, 40),
        );
        let accent = tiny_skia::Color::from_rgba8(59, 130, 246, 255);
        let ink = Color::rgb(235, 235, 240);
        let (ox, oy) = (tf.tx, tf.ty);
        for (r, b) in &self.buttons {
            let (x, y, w, h) = (r.x0 as f32, r.y0 as f32, r.width() as f32, r.height() as f32);
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            match *b {
                Btn::Tool(tool) => {
                    if tool == self.tool {
                        fx::fill_rrect(pm, tf, x, y, w, h, 10.0, &fx::solid(accent));
                    } else {
                        fx::fill_rrect(pm, tf, x, y, w, h, 10.0, &fx::solid(tiny_skia::Color::from_rgba8(255, 255, 255, 18)));
                    }
                    let white = tiny_skia::Color::from_rgba8(255, 255, 255, 255);
                    match tool {
                        Tool::Pen => {
                            fx::stroke_line(
                                pm,
                                tf,
                                (cx - 8.0, cy + 8.0),
                                (cx + 7.0, cy - 7.0),
                                4.0,
                                white,
                                tiny_skia::BlendMode::SourceOver,
                            );
                            fx::fill_circle(pm, tf, cx - 9.0, cy + 9.0, 2.2, white);
                        }
                        Tool::Highlighter => {
                            fx::stroke_line(
                                pm,
                                tf,
                                (cx - 9.0, cy + 6.0),
                                (cx + 9.0, cy - 6.0),
                                11.0,
                                tiny_skia::Color::from_rgba8(255, 235, 59, 190),
                                tiny_skia::BlendMode::SourceOver,
                            );
                        }
                        Tool::Text => {
                            let w = fonts.measure("T", 22.0);
                            fonts.draw(pm, "T", cx + ox - w / 2.0, cy + oy + 8.0, 22.0, Color::rgb(255, 255, 255), 1.0);
                        }
                        Tool::Callout => {
                            fx::fill_circle(pm, tf, cx, cy, 11.0, white);
                            let w = fonts.measure("1", 15.0);
                            fonts.draw(pm, "1", cx + ox - w / 2.0, cy + oy + 5.0, 15.0, Color::rgb(24, 24, 28), 1.0);
                        }
                        Tool::Eraser => {
                            fx::fill_rrect(pm, tf, cx - 10.0, cy - 6.0, 20.0, 12.0, 3.0, &fx::solid(white));
                            fx::fill_rrect(
                                pm,
                                tf,
                                cx - 10.0,
                                cy - 6.0,
                                8.0,
                                12.0,
                                3.0,
                                &fx::solid(tiny_skia::Color::from_rgba8(255, 130, 130, 255)),
                            );
                        }
                    }
                }
                Btn::Color(i) => {
                    let c = PALETTE[i];
                    if i == self.color_idx {
                        fx::fill_circle(pm, tf, cx, cy, 15.5, tiny_skia::Color::from_rgba8(255, 255, 255, 255));
                        fx::fill_circle(pm, tf, cx, cy, 13.0, tiny_skia::Color::from_rgba8(24, 24, 28, 255));
                    }
                    fx::fill_circle(pm, tf, cx, cy, 11.0, c.sk(1.0));
                }
                Btn::Undo | Btn::Clear | Btn::Close => {
                    fx::fill_rrect(pm, tf, x, y, w, h, 10.0, &fx::solid(tiny_skia::Color::from_rgba8(255, 255, 255, 18)));
                    let label = match *b {
                        Btn::Undo => "Undo",
                        Btn::Clear => "Clear",
                        _ => "Done",
                    };
                    let tw = fonts.measure(label, 14.0);
                    fonts.draw(pm, label, cx + ox - tw / 2.0, cy + oy + 5.0, 14.0, ink, 1.0);
                }
            }
        }
    }
}

fn draw_stroke(pm: &mut Pixmap, tf: Transform, pts: &[(f32, f32)], width: f32, c: tiny_skia::Color, flat: bool) {
    let paint: Paint = fx::solid(c);
    if pts.len() == 1 {
        fx::fill_circle(pm, tf, pts[0].0, pts[0].1, width * 0.5, c);
        return;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for w in pts.windows(2) {
        let mid = ((w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0);
        pb.quad_to(w[0].0, w[0].1, mid.0, mid.1);
    }
    let last = pts[pts.len() - 1];
    pb.line_to(last.0, last.1);
    if let Some(path) = pb.finish() {
        let s = Stroke {
            width: width.max(0.5),
            line_cap: if flat { LineCap::Butt } else { LineCap::Round },
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        pm.stroke_path(&path, &paint, &s, tf, None);
    }
}
