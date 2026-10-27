//! Shapes, cut into triangles the same way macroquad cut them (the same number of sides to a
//! circle, the same joints), so that the game looks as it did.

use super::batch::{geometry, Vertex};
use super::math::{vec2, vec3, vec4, Color, Mat4, Vec2};
use std::f32::consts::{PI, TAU};

pub fn draw_triangle(v1: Vec2, v2: Vec2, v3: Vec2, color: Color) {
    let v = [Vertex::new(v1.x, v1.y, 0.0, 0.0, 0.0, color), Vertex::new(v2.x, v2.y, 0.0, 0.0, 0.0, color), Vertex::new(v3.x, v3.y, 0.0, 0.0, 0.0, color)];
    geometry(None, &v, &[0, 1, 2]);
}

pub fn draw_rectangle(x: f32, y: f32, w: f32, h: f32, color: Color) {
    let v = [
        Vertex::new(x, y, 0.0, 0.0, 0.0, color),
        Vertex::new(x + w, y, 0.0, 1.0, 0.0, color),
        Vertex::new(x + w, y + h, 0.0, 1.0, 1.0, color),
        Vertex::new(x, y + h, 0.0, 0.0, 1.0, color),
    ];
    geometry(None, &v, &[0, 1, 2, 0, 2, 3]);
}

pub fn draw_rectangle_lines(x: f32, y: f32, w: f32, h: f32, thickness: f32, color: Color) {
    let t = thickness / 2.0;
    let v = [
        Vertex::new(x, y, 0.0, 0.0, 1.0, color),
        Vertex::new(x + w, y, 0.0, 1.0, 0.0, color),
        Vertex::new(x + w, y + h, 0.0, 1.0, 1.0, color),
        Vertex::new(x, y + h, 0.0, 0.0, 0.0, color),
        Vertex::new(x + t, y + t, 0.0, 0.0, 0.0, color),
        Vertex::new(x + w - t, y + t, 0.0, 0.0, 0.0, color),
        Vertex::new(x + w - t, y + h - t, 0.0, 0.0, 0.0, color),
        Vertex::new(x + t, y + h - t, 0.0, 0.0, 0.0, color),
    ];
    geometry(None, &v, &[0, 1, 4, 1, 4, 5, 1, 5, 6, 1, 2, 6, 3, 7, 2, 2, 7, 6, 0, 4, 3, 3, 4, 7]);
}

#[derive(Debug, Clone)]
pub struct DrawRectangleParams {
    /// Where `[x, y]` is on the rectangle, as a fraction of its size ((0.5, 0.5) is its middle).
    pub offset: Vec2,
    /// Radians.
    pub rotation: f32,
    pub color: Color,
}

impl Default for DrawRectangleParams {
    fn default() -> Self {
        DrawRectangleParams { offset: vec2(0.0, 0.0), rotation: 0.0, color: Color::from_rgba(255, 255, 255, 255) }
    }
}

pub fn draw_rectangle_ex(x: f32, y: f32, w: f32, h: f32, p: DrawRectangleParams) {
    let m = Mat4::from_translation(vec3(x, y, 0.0)) * Mat4::from_axis_angle(vec3(0.0, 0.0, 1.0), p.rotation) * Mat4::from_scale(vec3(w, h, 1.0));
    let c = [(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)].map(|(u, v)| m * vec4(u - p.offset.x, v - p.offset.y, 0.0, 1.0));
    let v = [
        Vertex::new(c[0].x, c[0].y, 0.0, 0.0, 0.0, p.color),
        Vertex::new(c[1].x, c[1].y, 0.0, 1.0, 0.0, p.color),
        Vertex::new(c[2].x, c[2].y, 0.0, 1.0, 1.0, p.color),
        Vertex::new(c[3].x, c[3].y, 0.0, 0.0, 1.0, p.color),
    ];
    geometry(None, &v, &[0, 1, 2, 0, 2, 3]);
}

/// A regular polygon centred on `[x, y]`, `rotation` in degrees.
pub fn draw_poly(x: f32, y: f32, sides: u8, radius: f32, rotation: f32, color: Color) {
    let mut v = Vec::with_capacity(sides as usize + 2);
    let mut idx = Vec::with_capacity(sides as usize * 3);
    let rot = rotation.to_radians();
    v.push(Vertex::new(x, y, 0.0, 0.0, 0.0, color));
    for i in 0..=sides {
        let a = i as f32 / sides as f32 * PI * 2.0 + rot;
        let (rx, ry) = (a.cos(), a.sin());
        v.push(Vertex::new(x + radius * rx, y + radius * ry, 0.0, rx, ry, color));
        if i != sides {
            idx.extend_from_slice(&[0, i as u16 + 1, i as u16 + 2]);
        }
    }
    geometry(None, &v, &idx);
}

pub fn draw_circle(x: f32, y: f32, r: f32, color: Color) {
    draw_poly(x, y, 20, r, 0.0, color);
}

pub fn draw_circle_lines(x: f32, y: f32, r: f32, thickness: f32, color: Color) {
    draw_arc(x, y, 30, r, 0.0, thickness, 360.0, color);
}

/// An ellipse of half-axes `w` and `h`, `rotation` in degrees.
pub fn draw_ellipse(x: f32, y: f32, w: f32, h: f32, rotation: f32, color: Color) {
    let sides = 20;
    let mut v = Vec::with_capacity(sides + 2);
    let mut idx = Vec::with_capacity(sides * 3);
    let rot = rotation.to_radians();
    let (sr, cr) = (rot.sin(), rot.cos());
    v.push(Vertex::new(x, y, 0.0, 0.0, 0.0, color));
    for i in 0..=sides {
        let a = i as f32 / sides as f32 * PI * 2.0;
        let (rx, ry) = (a.cos(), a.sin());
        let (px, py) = (w * rx, h * ry);
        v.push(Vertex::new(x + px * cr - py * sr, y + py * cr + px * sr, 0.0, rx, ry, color));
        if i != sides {
            idx.extend_from_slice(&[0, i as u16 + 1, i as u16 + 2]);
        }
    }
    geometry(None, &v, &idx);
}

pub fn draw_line(x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, color: Color) {
    let (nx, ny) = (y1 - y2, x2 - x1);
    let tlen = (nx * nx + ny * ny).sqrt() / (thickness * 0.5);
    if tlen < f32::EPSILON {
        return;
    }
    let (tx, ty) = (nx / tlen, ny / tlen);
    let v = [
        Vertex::new(x1 + tx, y1 + ty, 0.0, 0.0, 0.0, color),
        Vertex::new(x1 - tx, y1 - ty, 0.0, 0.0, 0.0, color),
        Vertex::new(x2 + tx, y2 + ty, 0.0, 0.0, 0.0, color),
        Vertex::new(x2 - tx, y2 - ty, 0.0, 0.0, 0.0, color),
    ];
    geometry(None, &v, &[0, 1, 2, 2, 1, 3]);
}

/// An arc from `rotation` to `rotation + arc` (degrees), `thickness` outwards from `radius`.
#[allow(clippy::too_many_arguments)]
pub fn draw_arc(x: f32, y: f32, sides: u8, radius: f32, rotation: f32, thickness: f32, arc: f32, color: Color) {
    let rot = rotation.to_radians();
    let part = arc.to_radians();
    let n = (sides as f32 * part / TAU).ceil().max(1.0);
    let span = part / n;
    let n = n as usize;
    let mut v = Vec::with_capacity(n * 4);
    let mut idx = Vec::with_capacity(n * 6);
    for i in 0..n {
        let a0 = i as f32 * span + rot;
        let a1 = a0 + span;
        let k = v.len() as u16;
        idx.extend([0, 1, 2, 2, 1, 3].map(|j| j + k));
        for (a, r) in [(a0, radius), (a0, radius + thickness), (a1, radius), (a1, radius + thickness)] {
            let p = vec2(x, y) + r * Vec2::from_angle(a);
            v.push(Vertex::new(p.x, p.y, 0.0, 0.0, 0.0, color));
        }
    }
    geometry(None, &v, &idx);
}

/// Triangles of the caller's own (drawn with the material in use, which reads their `uv` and
/// colour as it likes).
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
}

pub fn draw_mesh(m: &Mesh) {
    geometry(None, &m.vertices, &m.indices);
}
