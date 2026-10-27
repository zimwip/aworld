//! Points, rectangles and colours (the same as macroquad's, which the game was written with).

pub use glam::{vec2, vec3, vec4, Mat4, Vec2, Vec4};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub const fn size(&self) -> Vec2 {
        vec2(self.w, self.h)
    }

    pub const fn left(&self) -> f32 {
        self.x
    }

    pub const fn right(&self) -> f32 {
        self.x + self.w
    }

    pub const fn top(&self) -> f32 {
        self.y
    }

    pub const fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub const fn contains(&self, p: Vec2) -> bool {
        p.x >= self.left() && p.x <= self.right() && p.y <= self.bottom() && p.y >= self.top()
    }

    pub const fn overlaps(&self, other: &Rect) -> bool {
        self.left() <= other.right() && self.right() >= other.left() && self.top() <= other.bottom() && self.bottom() >= other.top()
    }

    pub fn combine_with(self, other: Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect { x, y, w: self.right().max(other.right()) - x, h: self.bottom().max(other.bottom()) - y }
    }

}

/// A colour, each channel from 0 to 1, not premultiplied.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Color {
        Color { r, g, b, a }
    }

    pub const fn from_rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0)
    }

    pub const fn with_alpha(&self, a: f32) -> Color {
        Color::new(self.r, self.g, self.b, a)
    }

    pub const fn to_vec(&self) -> Vec4 {
        Vec4::new(self.r, self.g, self.b, self.a)
    }
}

impl From<Color> for [u8; 4] {
    fn from(c: Color) -> Self {
        [(c.r * 255.0) as u8, (c.g * 255.0) as u8, (c.b * 255.0) as u8, (c.a * 255.0) as u8]
    }
}

impl From<[u8; 4]> for Color {
    fn from(c: [u8; 4]) -> Self {
        Color::from_rgba(c[0], c[1], c[2], c[3])
    }
}

pub const WHITE: Color = Color::new(1.0, 1.0, 1.0, 1.0);
pub const BLACK: Color = Color::new(0.0, 0.0, 0.0, 1.0);
pub const BLANK: Color = Color::new(0.0, 0.0, 0.0, 0.0);
pub const ORANGE: Color = Color::new(1.0, 0.63, 0.0, 1.0);
