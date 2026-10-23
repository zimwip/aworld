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
    Rotate,
    Save,
    Load,
    World,
    Blank,
    Minus,
    Plus,
    Full,
    Eraser,
    Tunnel,
    Flag,
    Hammer,
    Cable,
    Loupe,
    Thermo,
    Flame,
    Science,
    Sun,
    Moon,
    Undo,
    Gauge,
    Wind,
    Eye,
    Shovel,
    Select,
    Drop,
    Pin,
    Bolt,
}

pub struct Ui {
    pub font: Font,
    pub mouse: Vec2,
    /// A click for what is being drawn now: false when a window in front of it is under the pointer.
    pub pressed: bool,
    /// When set, buttons use this text size (so that a row of tabs reads evenly).
    pub force_size: Option<u16>,
    /// Under a modal window nothing else answers a click.
    pub muted: bool,
    /// The click of this frame, whoever it goes to, and where the pointer is on the screen.
    click: bool,
    screen_mouse: Vec2,
    layers: Layers,
}

/// The windows (menus, bars, pills) in the order they are drawn, back first, so that a click goes
/// to the frontmost one under the pointer and to nothing beneath it, neither a window nor the world.
#[derive(Default)]
pub struct Layers {
    /// This frame's windows.
    now: Vec<(&'static str, Rect)>,
    /// Last frame's: what was on the screen when the player clicked.
    seen: Vec<(&'static str, Rect)>,
}

impl Layers {
    pub fn next_frame(&mut self) {
        self.seen = std::mem::take(&mut self.now);
    }

    /// `id` covers `r` (a window may claim several rectangles).
    pub fn claim(&mut self, id: &'static str, r: Rect) {
        self.now.push((id, r));
    }

    /// Is `p` on a window drawn in front of `id`? A window that was not there is in front of all.
    pub fn covered(&self, id: &'static str, p: Vec2) -> bool {
        match self.seen.iter().rposition(|(n, _)| *n == id) {
            Some(i) => self.seen[i + 1..].iter().any(|(n, r)| *n != id && r.contains(p)),
            None => false,
        }
    }

    /// Is `p` on any window (so not on the world)?
    pub fn any(&self, p: Vec2) -> bool {
        self.seen.iter().any(|(_, r)| r.contains(p))
    }
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
        Icon::Pin => {
            // A push-pin.
            draw_circle(c.x, c.y - s * 0.2, s * 0.2, col);
            draw_line(c.x - s * 0.3, c.y + s * 0.02, c.x + s * 0.3, c.y + s * 0.02, th, col);
            draw_line(c.x, c.y + s * 0.02, c.x, c.y + s * 0.44, th * 0.8, col);
        }
        Icon::Drop => {
            // A drop: round belly, pointed top.
            draw_circle(c.x, c.y + s * 0.14, s * 0.3, col);
            draw_triangle(vec2(c.x, c.y - s * 0.42), vec2(c.x - s * 0.27, c.y + s * 0.02), vec2(c.x + s * 0.27, c.y + s * 0.02), col);
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
        Icon::Tunnel => {
            // A hill with a vaulted opening: brick arch over a dark hole.
            draw_triangle(vec2(c.x - s * 0.5, c.y + s * 0.38), vec2(c.x + s * 0.5, c.y + s * 0.38), vec2(c.x, c.y - s * 0.45), col);
            draw_rectangle(c.x - s * 0.2, c.y + s * 0.05, s * 0.4, s * 0.33, BG);
            draw_circle(c.x, c.y + s * 0.05, s * 0.2, BG);
            draw_arc(c.x, c.y + s * 0.05, 12, s * 0.24, 180.0, th * 1.2, 180.0, Color::new(0.85, 0.42, 0.3, 1.0));
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
        Icon::Flame => {
            // A flame: an orange drop with a yellow heart, and the nozzle below it.
            let o = Color::new(1.0, 0.45, 0.1, col.a);
            draw_circle(c.x, c.y + s * 0.05, s * 0.26, o);
            draw_triangle(vec2(c.x - s * 0.25, c.y), vec2(c.x + s * 0.25, c.y), vec2(c.x + s * 0.05, c.y - s * 0.5), o);
            draw_circle(c.x, c.y + s * 0.12, s * 0.12, Color::new(1.0, 0.85, 0.3, col.a));
            draw_rectangle(c.x - s * 0.1, c.y + s * 0.3, s * 0.2, s * 0.16, col);
        }
        Icon::Bolt => {
            // A bolt of lightning: two yellow wedges in a zigzag.
            let y = Color::new(1.0, 0.9, 0.3, col.a);
            draw_triangle(vec2(c.x + s * 0.12, c.y - s * 0.48), vec2(c.x - s * 0.3, c.y + s * 0.06), vec2(c.x + s * 0.05, c.y + s * 0.02), y);
            draw_triangle(vec2(c.x - s * 0.08, c.y - s * 0.04), vec2(c.x + s * 0.3, c.y - s * 0.04), vec2(c.x - s * 0.14, c.y + s * 0.5), y);
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
        Icon::Select => {
            // A dashed frame with an arrow pointer.
            for k in 0..4 {
                let t = k as f32 * 0.22 - 0.4;
                draw_line(c.x + s * t, c.y - s * 0.4, c.x + s * (t + 0.12), c.y - s * 0.4, th, col);
                draw_line(c.x - s * 0.4, c.y + s * t, c.x - s * 0.4, c.y + s * (t + 0.12), th, col);
            }
            draw_triangle(vec2(c.x - s * 0.08, c.y - s * 0.12), vec2(c.x + s * 0.4, c.y + s * 0.16), vec2(c.x + s * 0.04, c.y + s * 0.2), col);
            draw_triangle(vec2(c.x + s * 0.04, c.y + s * 0.2), vec2(c.x + s * 0.3, c.y + s * 0.4), vec2(c.x + s * 0.4, c.y + s * 0.16), col);
        }
        Icon::Shovel => {
            // A handle with a cross-grip, and a scoop blade.
            draw_line(c.x - s * 0.1, c.y - s * 0.46, c.x + s * 0.1, c.y + s * 0.05, th * 1.2, col);
            draw_line(c.x - s * 0.22, c.y - s * 0.46, c.x + s * 0.04, c.y - s * 0.46, th * 1.2, col);
            draw_triangle(vec2(c.x - s * 0.18, c.y + s * 0.02), vec2(c.x + s * 0.3, c.y + s * 0.02), vec2(c.x + s * 0.06, c.y + s * 0.46), col);
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
    pub fn new(font: Font) -> Ui {
        Ui { font, mouse: Vec2::ZERO, pressed: false, force_size: None, muted: false, click: false, screen_mouse: Vec2::ZERO, layers: Layers::default() }
    }

    pub fn begin(&mut self) {
        let (x, y) = mouse_position();
        self.mouse = vec2(x, y);
        self.screen_mouse = self.mouse;
        self.click = is_mouse_button_pressed(MouseButton::Left);
        self.pressed = self.click;
        self.muted = false;
        self.layers.next_frame();
    }

    /// No click this frame (and the pointer at `at`), for screenshots.
    pub fn freeze(&mut self, at: Vec2) {
        self.click = false;
        self.pressed = false;
        self.mouse = at;
        self.screen_mouse = at;
    }

    /// What follows is drawn in the window `id`: it gets the click only if no window in front of
    /// it is under the pointer.
    pub fn layer(&mut self, id: &'static str) {
        self.pressed = self.click && !self.muted && !self.layers.covered(id, self.screen_mouse);
    }

    /// The window `id` is at `r` on the screen: clicks there are its own.
    pub fn claim(&mut self, id: &'static str, r: Rect) {
        self.layers.claim(id, r);
    }

    /// Is the pointer on a window rather than on the world?
    pub fn over_ui(&self) -> bool {
        self.layers.any(self.screen_mouse)
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
    /// The largest text size (at most 16) that fits `label` in `width` pixels.
    pub fn fit_size(&self, label: &str, width: f32) -> u16 {
        let mut size = 16u16;
        while size > 10 && self.text_w(label, size) > width {
            size -= 1;
        }
        size
    }

    pub fn button(&mut self, r: Rect, label: &str, icon: Icon, active: bool, enabled: bool, accent: Color) -> bool {
        let hot = enabled && self.hover(r);
        let base = if active { accent } else if hot { TILE_HOT } else { TILE };
        let fg = if !enabled { Color::new(0.4, 0.42, 0.5, 1.0) } else if active { BG } else { TEXT };
        rrect(Rect::new(r.x, r.y + 2.0, r.w, r.h), 10.0, Color::new(0.0, 0.0, 0.0, 0.35));
        rrect(r, 10.0, base);
        if !active && enabled {
            rrect(Rect::new(r.x + 2.0, r.y + 2.0, r.w - 4.0, r.h * 0.45), 8.0, Color::new(1.0, 1.0, 1.0, 0.05));
        }
        let has_icon = icon != Icon::None;
        let icon_w = if has_icon { 19.0 } else { 0.0 };
        let gap = if has_icon && !label.is_empty() { 5.0 } else { 0.0 };
        // A long label shrinks to fit its button rather than spilling over the neighbours.
        let mut size = self.force_size.unwrap_or(16);
        while size > 10 && !label.is_empty() && icon_w + gap + self.text_w(label, size) > r.w - 8.0 {
            size -= 1;
        }
        let tw = if label.is_empty() { 0.0 } else { self.text_w(label, size) };
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

/// A side menu that slides out while the pointer is over it (or over its edge), tucks itself
/// away as soon as the pointer leaves, and stays out when pinned. Every side menu works this way.
#[derive(Clone, Copy, Debug)]
pub struct Drawer {
    /// 0 = tucked away, 1 = fully out.
    pub open: f32,
    pub pin: bool,
    /// Seconds it stays out on its own after `show`, so that something just chosen is seen.
    pub peek: f32,
}

impl Drawer {
    pub fn new(pin: bool) -> Drawer {
        Drawer { open: if pin { 1.0 } else { 0.0 }, pin, peek: 0.0 }
    }

    /// Slides out at once and stays out `secs` seconds even without the pointer, then behaves as usual.
    pub fn show(&mut self, secs: f32) {
        self.open = 1.0;
        self.peek = secs;
    }

    /// Moves the drawer towards out (pointer over it, or pinned) or in, over `dt` seconds.
    pub fn update(&mut self, hovering: bool, dt: f32) {
        // The pointer over it ends the peek: from then on, leaving closes it.
        self.peek = if hovering { 0.0 } else { (self.peek - dt).max(0.0) };
        let target = if hovering || self.pin || self.peek > 0.0 { 1.0 } else { 0.0 };
        self.open += (target - self.open) * (dt * 9.0).min(1.0);
        if (self.open - target).abs() < 0.01 {
            self.open = target;
        }
    }

    /// Whether any of it shows.
    pub fn visible(&self) -> bool {
        self.open > 0.02
    }

    pub fn toggle_pin(&mut self) {
        self.pin = !self.pin;
    }
}

#[cfg(test)]
mod tests {
    use super::{Drawer, Layers};
    use macroquad::prelude::{vec2, Rect};

    /// The palette (at the left) is drawn after the day pill, which it overlaps when it is out.
    fn pill_under_palette() -> Layers {
        let mut l = Layers::default();
        l.claim("hud", Rect::new(0.0, 0.0, 300.0, 26.0));
        l.claim("panel", Rect::new(0.0, 0.0, 200.0, 600.0));
        l.next_frame();
        l
    }

    #[test]
    fn a_click_on_a_window_does_not_reach_the_one_under_it() {
        let l = pill_under_palette();
        let on_both = vec2(100.0, 10.0);
        assert!(l.covered("hud", on_both), "the pill must not open the world's menu under the palette");
        assert!(!l.covered("panel", on_both), "the palette's buttons get the click");
        assert!(!l.covered("hud", vec2(250.0, 10.0)), "the pill still answers beside the palette");
    }

    #[test]
    fn a_click_on_a_window_does_not_reach_the_world() {
        let l = pill_under_palette();
        assert!(l.any(vec2(100.0, 300.0)));
        assert!(!l.any(vec2(400.0, 300.0)));
    }

    #[test]
    fn a_window_that_just_opened_is_in_front() {
        let l = pill_under_palette();
        assert!(!l.covered("climate", vec2(100.0, 10.0)));
    }

    fn run(d: &mut Drawer, hovering: bool, frames: usize) {
        for _ in 0..frames {
            d.update(hovering, 1.0 / 60.0);
        }
    }

    #[test]
    fn a_drawer_opens_on_hover_and_closes_when_the_pointer_leaves() {
        let mut d = Drawer::new(false);
        assert!(!d.visible());
        run(&mut d, true, 60);
        assert_eq!(d.open, 1.0);
        run(&mut d, false, 60);
        assert_eq!(d.open, 0.0, "it tucks away by itself");
    }

    #[test]
    fn a_pinned_drawer_stays_out_until_unpinned() {
        let mut d = Drawer::new(false);
        run(&mut d, true, 60);
        d.toggle_pin();
        run(&mut d, false, 120);
        assert_eq!(d.open, 1.0, "pinned, it stays out");
        d.toggle_pin();
        run(&mut d, false, 60);
        assert_eq!(d.open, 0.0);
    }

    #[test]
    fn a_shown_drawer_peeks_out_then_tucks_away() {
        let mut d = Drawer::new(false);
        d.show(1.0);
        run(&mut d, false, 30);
        assert_eq!(d.open, 1.0, "still out during the peek");
        run(&mut d, false, 90);
        assert_eq!(d.open, 0.0, "then it tucks away by itself");
        d.show(5.0);
        run(&mut d, true, 10);
        run(&mut d, false, 60);
        assert_eq!(d.open, 0.0, "once visited, leaving closes it at once");
    }
}
