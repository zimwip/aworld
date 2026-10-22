//! Immediate-mode widgets: big friendly buttons with drawn icons.

use macroquad::prelude::*;

pub const BG: Color = Color::new(0.075, 0.085, 0.13, 1.0);
pub const PANEL: Color = Color::new(0.115, 0.13, 0.20, 1.0);
pub const TILE: Color = Color::new(0.17, 0.19, 0.29, 1.0);
pub const TILE_HOT: Color = Color::new(0.24, 0.27, 0.40, 1.0);
pub const TEXT: Color = Color::new(0.93, 0.95, 1.0, 1.0);
pub const DIM: Color = Color::new(0.62, 0.67, 0.80, 1.0);
pub const ACCENT: Color = Color::new(1.0, 0.70, 0.18, 1.0);
pub const GREEN: Color = Color::new(0.25, 0.78, 0.45, 1.0);
pub const BLUE: Color = Color::new(0.30, 0.55, 1.0, 1.0);
pub const RED: Color = Color::new(0.93, 0.30, 0.30, 1.0);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    None,
    Play,
    Pause,
    Reset,
    Car,
    Rotate,
    Save,
    Load,
    World,
    Blank,
    Minus,
    Plus,
    Full,
    Eraser,
    Flag,
    Hammer,
    Cable,
    Loupe,
    Thermo,
    Science,
    Pick,
    Sun,
    Moon,
    Undo,
    Gauge,
    Wind,
    Eye,
}

pub struct Ui {
    pub font: Font,
    pub mouse: Vec2,
    pub pressed: bool,
}

pub fn rrect(r: Rect, rad: f32, c: Color) {
    // Triangle fan from the centre: no overlaps, so translucent colours stay even.
    let rad = rad.min(r.w / 2.0).min(r.h / 2.0);
    let centre = vec2(r.x + r.w / 2.0, r.y + r.h / 2.0);
    let corners = [
        (vec2(r.x + r.w - rad, r.y + rad), -90.0f32),
        (vec2(r.x + r.w - rad, r.y + r.h - rad), 0.0),
        (vec2(r.x + rad, r.y + r.h - rad), 90.0),
        (vec2(r.x + rad, r.y + rad), 180.0),
    ];
    let mut pts: Vec<Vec2> = Vec::with_capacity(32);
    for (cc, a0) in corners {
        for i in 0..=6 {
            let a = (a0 + 90.0 * i as f32 / 6.0).to_radians();
            pts.push(cc + vec2(a.cos(), a.sin()) * rad);
        }
    }
    for i in 0..pts.len() {
        draw_triangle(centre, pts[i], pts[(i + 1) % pts.len()], c);
    }
}

pub fn rrect_lines(r: Rect, rad: f32, th: f32, c: Color) {
    let rad = rad.min(r.w / 2.0).min(r.h / 2.0);
    draw_line(r.x + rad, r.y, r.x + r.w - rad, r.y, th, c);
    draw_line(r.x + rad, r.y + r.h, r.x + r.w - rad, r.y + r.h, th, c);
    draw_line(r.x, r.y + rad, r.x, r.y + r.h - rad, th, c);
    draw_line(r.x + r.w, r.y + rad, r.x + r.w, r.y + r.h - rad, th, c);
    for (cx, cy, a) in [
        (r.x + rad, r.y + rad, 180.0),
        (r.x + r.w - rad, r.y + rad, 270.0),
        (r.x + r.w - rad, r.y + r.h - rad, 0.0),
        (r.x + rad, r.y + r.h - rad, 90.0),
    ] {
        arc(vec2(cx, cy), rad, a, a + 90.0, th, c);
    }
}

/// Polyline arc, angles in degrees.
pub fn arc(c: Vec2, r: f32, a0: f32, a1: f32, th: f32, col: Color) {
    let n = ((a1 - a0).abs() / 12.0).ceil().max(2.0) as i32;
    let mut prev = c + vec2(a0.to_radians().cos(), a0.to_radians().sin()) * r;
    for i in 1..=n {
        let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
        let p = c + vec2(a.cos(), a.sin()) * r;
        draw_line(prev.x, prev.y, p.x, p.y, th, col);
        prev = p;
    }
}

fn arrow_head(tip: Vec2, dir: Vec2, s: f32, c: Color) {
    let n = vec2(-dir.y, dir.x);
    draw_triangle(tip + dir * s * 0.5, tip - dir * s * 0.4 + n * s * 0.5, tip - dir * s * 0.4 - n * s * 0.5, c);
}

pub fn draw_icon(icon: Icon, c: Vec2, s: f32, col: Color) {
    let th = (s * 0.14).max(2.0);
    match icon {
        Icon::None => {}
        Icon::Play => draw_triangle(vec2(c.x - s * 0.3, c.y - s * 0.42), vec2(c.x - s * 0.3, c.y + s * 0.42), vec2(c.x + s * 0.5, c.y), col),
        Icon::Pause => {
            draw_rectangle(c.x - s * 0.32, c.y - s * 0.4, s * 0.24, s * 0.8, col);
            draw_rectangle(c.x + s * 0.08, c.y - s * 0.4, s * 0.24, s * 0.8, col);
        }
        Icon::Reset => {
            arc(c, s * 0.38, 30.0, 330.0, th, col);
            let a = 30.0f32.to_radians();
            let tip = c + vec2(a.cos(), a.sin()) * s * 0.38;
            arrow_head(tip, vec2(-a.sin(), a.cos()) * -1.0, s * 0.45, col);
        }
        Icon::Wind => {
            for (k, len) in [(-0.22f32, 0.8f32), (0.0, 0.6), (0.22, 0.75)] {
                draw_line(c.x - s * 0.4, c.y + s * k, c.x - s * 0.4 + s * len, c.y + s * k, th * 0.9, col);
            }
            arrow_head(vec2(c.x + s * 0.42, c.y), vec2(1.0, 0.0), s * 0.4, col);
        }
        Icon::Eye => {
            arc(c + vec2(0.0, s * 0.18), s * 0.36, 215.0, 325.0, th, col);
            arc(c - vec2(0.0, s * 0.18), s * 0.36, 35.0, 145.0, th, col);
            draw_circle(c.x, c.y, s * 0.13, col);
        }
        Icon::Gauge => {
            arc(c + vec2(0.0, s * 0.18), s * 0.42, 180.0, 360.0, th, col);
            draw_line(c.x, c.y + s * 0.18, c.x + s * 0.28, c.y - s * 0.12, th, col);
            draw_circle(c.x, c.y + s * 0.18, s * 0.07, col);
        }
        Icon::Undo => {
            arc(c + vec2(0.0, s * 0.05), s * 0.34, 200.0, 440.0, th, col);
            let a = 200.0f32.to_radians();
            let tip = c + vec2(0.0, s * 0.05) + vec2(a.cos(), a.sin()) * s * 0.34;
            arrow_head(tip, vec2(a.sin(), -a.cos()) * -1.0, s * 0.45, col);
        }
        Icon::Rotate => {
            arc(c, s * 0.38, -60.0, 240.0, th, col);
            let a = (-60.0f32).to_radians();
            let tip = c + vec2(a.cos(), a.sin()) * s * 0.38;
            arrow_head(tip, vec2(a.sin(), -a.cos()), s * 0.45, col);
        }
        Icon::Car => {
            draw_rectangle(c.x - s * 0.5, c.y - s * 0.05, s, s * 0.3, col);
            draw_rectangle(c.x - s * 0.28, c.y - s * 0.32, s * 0.5, s * 0.28, col);
            draw_circle(c.x - s * 0.28, c.y + s * 0.3, s * 0.15, BG);
            draw_circle(c.x + s * 0.28, c.y + s * 0.3, s * 0.15, BG);
            draw_circle_lines(c.x - s * 0.28, c.y + s * 0.3, s * 0.15, th * 0.7, col);
            draw_circle_lines(c.x + s * 0.28, c.y + s * 0.3, s * 0.15, th * 0.7, col);
        }
        Icon::Save => {
            draw_rectangle(c.x - s * 0.4, c.y - s * 0.4, s * 0.8, s * 0.8, col);
            draw_rectangle(c.x - s * 0.22, c.y - s * 0.4, s * 0.44, s * 0.26, BG);
            draw_rectangle(c.x - s * 0.24, c.y + s * 0.04, s * 0.48, s * 0.36, BG);
        }
        Icon::Load => {
            draw_rectangle(c.x - s * 0.45, c.y - s * 0.3, s * 0.4, s * 0.14, col);
            draw_rectangle(c.x - s * 0.45, c.y - s * 0.18, s * 0.9, s * 0.54, col);
            draw_triangle(vec2(c.x, c.y - s * 0.02), vec2(c.x - s * 0.16, c.y + s * 0.2), vec2(c.x + s * 0.16, c.y + s * 0.2), BG);
        }
        Icon::World => {
            draw_circle(c.x + s * 0.2, c.y - s * 0.22, s * 0.16, col);
            draw_triangle(vec2(c.x - s * 0.5, c.y + s * 0.4), vec2(c.x - s * 0.12, c.y - s * 0.18), vec2(c.x + s * 0.26, c.y + s * 0.4), col);
            draw_triangle(vec2(c.x - s * 0.05, c.y + s * 0.4), vec2(c.x + s * 0.26, c.y), vec2(c.x + s * 0.55, c.y + s * 0.4), col);
        }
        Icon::Blank => rrect_lines(Rect::new(c.x - s * 0.4, c.y - s * 0.4, s * 0.8, s * 0.8), 3.0, th, col),
        Icon::Minus => draw_rectangle(c.x - s * 0.35, c.y - th / 2.0, s * 0.7, th, col),
        Icon::Plus => {
            draw_rectangle(c.x - s * 0.35, c.y - th / 2.0, s * 0.7, th, col);
            draw_rectangle(c.x - th / 2.0, c.y - s * 0.35, th, s * 0.7, col);
        }
        Icon::Full => {
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let corner = c + vec2(sx, sy) * s * 0.42;
                draw_line(corner.x, corner.y, corner.x - sx * s * 0.28, corner.y, th, col);
                draw_line(corner.x, corner.y, corner.x, corner.y - sy * s * 0.28, th, col);
            }
        }
        Icon::Eraser => {
            draw_rectangle_ex(c.x - s * 0.1, c.y - s * 0.22, s * 0.6, s * 0.4, DrawRectangleParams { offset: vec2(0.5, 0.5), rotation: -0.6, color: col });
            draw_rectangle_ex(c.x - s * 0.1, c.y - s * 0.22, s * 0.28, s * 0.4, DrawRectangleParams { offset: vec2(-0.6, 0.5), rotation: -0.6, color: Color::new(1.0, 0.55, 0.6, 1.0) });
        }
        Icon::Hammer => {
            draw_line(c.x - s * 0.35, c.y + s * 0.4, c.x + s * 0.2, c.y - s * 0.15, th * 1.3, col);
            draw_rectangle_ex(c.x + s * 0.18, c.y - s * 0.28, s * 0.5, s * 0.3, DrawRectangleParams { offset: vec2(0.5, 0.5), rotation: -0.7, color: col });
        }
        Icon::Cable => {
            draw_circle(c.x - s * 0.38, c.y + s * 0.22, s * 0.16, col);
            draw_circle(c.x - s * 0.38, c.y + s * 0.22, s * 0.06, BG);
            let mut prev = vec2(c.x - s * 0.22, c.y + s * 0.22);
            for i in 1..=8 {
                let t = i as f32 / 8.0;
                let p = vec2(c.x - s * 0.22 + s * 0.62 * t, c.y + s * 0.22 - s * 0.55 * t + (t * std::f32::consts::PI).sin() * s * 0.28);
                draw_line(prev.x, prev.y, p.x, p.y, th, col);
                prev = p;
            }
            draw_circle(prev.x, prev.y, s * 0.09, col);
        }
        Icon::Loupe => {
            draw_circle_lines(c.x - s * 0.1, c.y - s * 0.1, s * 0.3, th, col);
            draw_line(c.x + s * 0.12, c.y + s * 0.12, c.x + s * 0.42, c.y + s * 0.42, th * 1.4, col);
        }
        Icon::Thermo => {
            draw_rectangle(c.x - th * 0.6, c.y - s * 0.42, th * 1.2, s * 0.6, col);
            draw_circle(c.x, c.y + s * 0.26, s * 0.2, col);
            draw_circle(c.x, c.y + s * 0.26, s * 0.1, RED);
        }
        Icon::Science => {
            draw_rectangle(c.x - s * 0.1, c.y - s * 0.42, s * 0.2, s * 0.3, col);
            draw_triangle(vec2(c.x - s * 0.1, c.y - s * 0.12), vec2(c.x + s * 0.1, c.y - s * 0.12), vec2(c.x + s * 0.4, c.y + s * 0.4), col);
            draw_triangle(vec2(c.x - s * 0.1, c.y - s * 0.12), vec2(c.x - s * 0.4, c.y + s * 0.4), vec2(c.x + s * 0.4, c.y + s * 0.4), col);
        }
        Icon::Pick => {
            draw_triangle(vec2(c.x - s * 0.3, c.y - s * 0.42), vec2(c.x - s * 0.3, c.y + s * 0.3), vec2(c.x + s * 0.3, c.y + s * 0.02), col);
        }
        Icon::Sun => {
            draw_circle(c.x, c.y, s * 0.22, col);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                let (d0, d1) = (vec2(a.cos(), a.sin()) * s * 0.32, vec2(a.cos(), a.sin()) * s * 0.46);
                draw_line(c.x + d0.x, c.y + d0.y, c.x + d1.x, c.y + d1.y, th, col);
            }
        }
        Icon::Moon => {
            draw_circle(c.x, c.y, s * 0.36, col);
            draw_circle(c.x + s * 0.18, c.y - s * 0.1, s * 0.3, BG);
        }
        Icon::Flag => {
            draw_rectangle(c.x - s * 0.3, c.y - s * 0.45, th, s * 0.9, col);
            draw_triangle(vec2(c.x - s * 0.3 + th, c.y - s * 0.45), vec2(c.x - s * 0.3 + th, c.y), vec2(c.x + s * 0.4, c.y - s * 0.22), col);
        }
    }
}

impl Ui {
    pub fn begin(&mut self) {
        let (x, y) = mouse_position();
        self.mouse = vec2(x, y);
        self.pressed = is_mouse_button_pressed(MouseButton::Left);
    }

    pub fn text(&self, s: &str, x: f32, y: f32, size: u16, c: Color) {
        draw_text_ex(s, x, y, TextParams { font: Some(&self.font), font_size: size, color: c, ..Default::default() });
    }

    pub fn text_w(&self, s: &str, size: u16) -> f32 {
        measure_text(s, Some(&self.font), size, 1.0).width
    }

    pub fn text_center(&self, s: &str, cx: f32, y: f32, size: u16, c: Color) {
        self.text(s, cx - self.text_w(s, size) / 2.0, y, size, c);
    }

    pub fn hover(&self, r: Rect) -> bool {
        r.contains(self.mouse)
    }

    /// Big rounded button with an icon and a label; returns true on click.
    pub fn button(&mut self, r: Rect, label: &str, icon: Icon, active: bool, enabled: bool, accent: Color) -> bool {
        let hot = enabled && self.hover(r);
        let base = if active { accent } else if hot { TILE_HOT } else { TILE };
        let fg = if !enabled { Color::new(0.4, 0.42, 0.5, 1.0) } else if active { BG } else { TEXT };
        rrect(Rect::new(r.x, r.y + 2.0, r.w, r.h), 10.0, Color::new(0.0, 0.0, 0.0, 0.35));
        rrect(r, 10.0, base);
        if !active && enabled {
            rrect(Rect::new(r.x + 2.0, r.y + 2.0, r.w - 4.0, r.h * 0.45), 8.0, Color::new(1.0, 1.0, 1.0, 0.05));
        }
        let size = 16u16;
        let has_icon = icon != Icon::None;
        let tw = if label.is_empty() { 0.0 } else { self.text_w(label, size) };
        let icon_w = if has_icon { 19.0 } else { 0.0 };
        let gap = if has_icon && !label.is_empty() { 5.0 } else { 0.0 };
        let total = icon_w + gap + tw;
        let x0 = r.x + (r.w - total) / 2.0;
        if has_icon {
            draw_icon(icon, vec2(x0 + icon_w / 2.0, r.y + r.h / 2.0), 18.0, fg);
        }
        if !label.is_empty() {
            self.text(label, x0 + icon_w + gap, r.y + r.h / 2.0 + 5.5, size, fg);
        }
        hot && self.pressed
    }

    /// A check box with a label; returns true when clicked.
    pub fn checkbox(&mut self, r: Rect, label: &str, checked: bool, enabled: bool) -> bool {
        let hot = enabled && self.hover(r);
        rrect(r, 8.0, if hot { TILE_HOT } else { TILE });
        let boxr = Rect::new(r.x + 7.0, r.y + (r.h - 16.0) / 2.0, 16.0, 16.0);
        rrect(boxr, 4.0, if checked { GREEN } else { Color::new(0.0, 0.0, 0.0, 0.4) });
        if checked {
            draw_line(boxr.x + 3.5, boxr.y + 8.5, boxr.x + 7.0, boxr.y + 12.0, 2.4, WHITE);
            draw_line(boxr.x + 7.0, boxr.y + 12.0, boxr.x + 13.0, boxr.y + 4.5, 2.4, WHITE);
        }
        let col = if enabled { TEXT } else { Color::new(0.45, 0.47, 0.55, 1.0) };
        self.text(label, r.x + 30.0, r.y + r.h / 2.0 + 5.0, 14, col);
        hot && self.pressed
    }

    /// Horizontal slider; returns true while the value is being changed.
    pub fn slider(&mut self, r: Rect, value: &mut f32, min: f32, max: f32, col: Color) -> bool {
        rrect(Rect::new(r.x, r.y + r.h / 2.0 - 3.0, r.w, 6.0), 3.0, Color::new(0.0, 0.0, 0.0, 0.4));
        let t = ((*value - min) / (max - min)).clamp(0.0, 1.0);
        rrect(Rect::new(r.x, r.y + r.h / 2.0 - 3.0, (r.w * t).max(6.0), 6.0), 3.0, col);
        let hot = r.contains(self.mouse);
        let mut changed = false;
        if hot && is_mouse_button_down(MouseButton::Left) {
            let nt = ((self.mouse.x - r.x) / r.w).clamp(0.0, 1.0);
            *value = min + (max - min) * nt;
            changed = true;
        }
        let knob = vec2(r.x + r.w * t, r.y + r.h / 2.0);
        draw_circle(knob.x, knob.y, if hot { 8.5 } else { 7.0 }, WHITE);
        draw_circle(knob.x, knob.y, 4.0, col);
        changed
    }

    /// Height `wrapped` would use, without drawing.
    pub fn wrapped_height(&self, text: &str, max_w: f32, size: u16) -> f32 {
        let mut line = String::new();
        let mut h = 0.0;
        for word in text.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if self.text_w(&candidate, size) > max_w && !line.is_empty() {
                h += size as f32 + 3.0;
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        if !line.is_empty() {
            h += size as f32 + 3.0;
        }
        h
    }

    /// Word-wrapped text; returns the height used.
    pub fn wrapped(&self, text: &str, x: f32, y: f32, max_w: f32, size: u16, col: Color) -> f32 {
        let mut line = String::new();
        let mut cy = y;
        for word in text.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if self.text_w(&candidate, size) > max_w && !line.is_empty() {
                self.text(&line, x, cy, size, col);
                cy += size as f32 + 3.0;
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        if !line.is_empty() {
            self.text(&line, x, cy, size, col);
            cy += size as f32 + 3.0;
        }
        cy - y
    }

    /// Palette tile: `draw` paints the swatch inside the given rect.
    pub fn tile(&mut self, r: Rect, label: &str, selected: bool, draw: impl FnOnce(Rect)) -> bool {
        let hot = self.hover(r);
        rrect(Rect::new(r.x, r.y + 2.0, r.w, r.h), 12.0, Color::new(0.0, 0.0, 0.0, 0.35));
        rrect(r, 12.0, if hot { TILE_HOT } else { TILE });
        draw(Rect::new(r.x + 7.0, r.y + 6.0, r.w - 14.0, r.h - 28.0));
        let size = if self.text_w(label, 16) > r.w - 6.0 { 13 } else { 16 };
        self.text_center(label, r.x + r.w / 2.0, r.y + r.h - 7.0, size, if selected { ACCENT } else { TEXT });
        if selected {
            rrect_lines(r, 12.0, 3.5, ACCENT);
        }
        hot && self.pressed
    }
}
