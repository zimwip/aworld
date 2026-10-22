//! Drawing: sky, world texture with a glow shader, blocks, cars, markers.

use crate::blocks::{Block, BlockKind, STUD};
use crate::car::{Car, ModelKind};
use crate::world::{Mat, World, H, W};
use macroquad::miniquad::{BlendFactor, BlendState, BlendValue, Equation, PipelineParams};
use macroquad::prelude::*;

/// Where the world sits on screen.
#[derive(Clone, Copy)]
pub struct View {
    pub ox: f32,
    pub oy: f32,
    pub s: f32,
}

impl View {
    pub fn fit(area: Rect) -> View {
        let s = (area.w / W as f32).min(area.h / H as f32);
        View { ox: area.x + (area.w - W as f32 * s) / 2.0, oy: area.y + (area.h - H as f32 * s) / 2.0, s }
    }

    pub fn to_screen(&self, p: Vec2) -> Vec2 {
        vec2(self.ox + p.x * self.s, self.oy + p.y * self.s)
    }

    pub fn to_world(&self, p: Vec2) -> Vec2 {
        vec2((p.x - self.ox) / self.s, (p.y - self.oy) / self.s)
    }

    pub fn rect(&self) -> Rect {
        Rect::new(self.ox, self.oy, W as f32 * self.s, H as f32 * self.s)
    }
}

const VERT: &str = "#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
varying lowp vec2 uv;
uniform mat4 Model;
uniform mat4 Projection;
void main() {
    gl_Position = Projection * Model * vec4(position, 1.0);
    uv = texcoord;
}";

// The texture is premultiplied RGBA. Bright orange cells (fire, lava, embers)
// bloom into their surroundings; every solid gets a light top edge.
const FRAG: &str = "#version 100
precision highp float;
varying vec2 uv;
uniform sampler2D Texture;
uniform vec2 texel;
void main() {
    vec4 c = texture2D(Texture, uv);
    vec3 glow = vec3(0.0);
    float wsum = 0.0;
    for (int i = -3; i <= 3; i++) {
        for (int j = -3; j <= 3; j++) {
            vec4 s = texture2D(Texture, uv + vec2(float(i), float(j)) * texel * 1.6);
            float e = step(0.93, s.r) * step(s.b, 0.3) * step(0.5, s.a);
            float w = 1.0 / (1.0 + float(i * i + j * j));
            glow += s.rgb * e * w;
            wsum += w;
        }
    }
    glow = glow / wsum;
    vec4 up = texture2D(Texture, uv - vec2(0.0, texel.y));
    float edge = c.a * (1.0 - up.a);
    vec3 rgb = c.rgb + vec3(0.18) * edge * c.a + glow * 2.2;
    float a = max(c.a, clamp(length(glow) * 2.0, 0.0, 1.0));
    gl_FragColor = vec4(rgb, a);
}";

pub struct Renderer {
    pub image: Image,
    pub texture: Texture2D,
    material: Material,
}

impl Renderer {
    pub fn new() -> Self {
        let image = Image::gen_image_color(W as u16, H as u16, Color::new(0.0, 0.0, 0.0, 0.0));
        let texture = Texture2D::from_image(&image);
        texture.set_filter(FilterMode::Linear);
        let blend = BlendState::new(Equation::Add, BlendFactor::One, BlendFactor::OneMinusValue(BlendValue::SourceAlpha));
        let material = load_material(
            ShaderSource::Glsl { vertex: VERT, fragment: FRAG },
            MaterialParams {
                pipeline_params: PipelineParams { color_blend: Some(blend), alpha_blend: Some(blend), ..Default::default() },
                uniforms: vec![UniformDesc::new("texel", UniformType::Float2)],
                ..Default::default()
            },
        )
        .expect("shader");
        Renderer { image, texture, material }
    }

    pub fn draw_world(&mut self, world: &World, v: View) {
        {
            let px = self.image.get_image_data_mut();
            for y in 0..H {
                for x in 0..W {
                    px[y * W + x] = world.color(x, y);
                }
            }
        }
        self.texture.update(&self.image);
        self.material.set_uniform("texel", vec2(1.0 / W as f32, 1.0 / H as f32));
        gl_use_material(&self.material);
        draw_texture_ex(
            &self.texture,
            v.ox,
            v.oy,
            WHITE,
            DrawTextureParams { dest_size: Some(vec2(W as f32 * v.s, H as f32 * v.s)), ..Default::default() },
        );
        gl_use_default_material();
    }
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    Color::new(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, 1.0)
}

pub fn draw_sky(v: View, time: f32) {
    let r = v.rect();
    let top = Color::from_rgba(70, 120, 214, 255);
    let mid = Color::from_rgba(150, 200, 238, 255);
    let low = Color::from_rgba(255, 224, 182, 255);
    let bands = 48;
    for i in 0..bands {
        let t = i as f32 / bands as f32;
        let c = if t < 0.6 { lerp_color(top, mid, t / 0.6) } else { lerp_color(mid, low, (t - 0.6) / 0.4) };
        draw_rectangle(r.x, r.y + r.h * t, r.w, r.h / bands as f32 + 1.0, c);
    }
    // Sun and drifting clouds.
    draw_circle(r.x + r.w * 0.82, r.y + r.h * 0.17, r.h * 0.07, Color::new(1.0, 0.96, 0.78, 0.35));
    draw_circle(r.x + r.w * 0.82, r.y + r.h * 0.17, r.h * 0.045, Color::new(1.0, 0.98, 0.85, 0.95));
    for k in 0..4 {
        let x = (r.x + ((k as f32 * 0.29 + time * 0.004) % 1.2 - 0.1) * r.w) as f32;
        let y = r.y + r.h * (0.12 + 0.09 * k as f32);
        for (dx, dy, rad) in [(0.0, 0.0, 0.03), (0.035, -0.012, 0.038), (0.075, 0.0, 0.03), (0.035, 0.01, 0.034)] {
            if x + dx * r.w - rad * r.h < r.x || x + dx * r.w + rad * r.h > r.x + r.w {
                continue;
            }
            draw_circle(x + dx * r.w, y + dy * r.h, rad * r.h, Color::new(1.0, 1.0, 1.0, 0.7));
        }
    }
    // Far hills.
    for (base, amp, freq, col) in [
        (0.66, 0.05, 6.0, Color::from_rgba(150, 178, 214, 255)),
        (0.74, 0.04, 9.0, Color::from_rgba(118, 152, 190, 255)),
    ] {
        let step = 6.0;
        let mut x = 0.0;
        while x < r.w {
            let h = (base + amp * ((x / r.w) * freq).sin() + amp * 0.5 * ((x / r.w) * freq * 2.3 + 1.0).sin()) * r.h;
            draw_rectangle(r.x + x, r.y + h, step + 1.0, r.h - h, col);
            x += step;
        }
    }
}

fn quad(a: Vec2, b: Vec2, c: Vec2, d: Vec2, col: Color) {
    draw_triangle(a, b, c, col);
    draw_triangle(a, c, d, col);
}

fn rot(a: f32, v: Vec2) -> Vec2 {
    let (s, c) = a.sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

fn shade(c: (u8, u8, u8), k: f32) -> Color {
    Color::new((c.0 as f32 / 255.0 * k).min(1.0), (c.1 as f32 / 255.0 * k).min(1.0), (c.2 as f32 / 255.0 * k).min(1.0), 1.0)
}

/// Draws a block shape centred at `c` (screen px); `s` is screen px per cell.
pub fn draw_block_shape(kind: BlockKind, c: Vec2, s: f32, angle: f32, tint: Option<Color>, heat: f32) {
    let spec = kind.spec();
    let p = |lx: f32, ly: f32| c + rot(angle, vec2(lx, ly) * s);
    let rect = |cx: f32, cy: f32, w: f32, h: f32, col: Color| {
        let (a, b, cc, d) = (p(cx - w / 2.0, cy - h / 2.0), p(cx + w / 2.0, cy - h / 2.0), p(cx + w / 2.0, cy + h / 2.0), p(cx - w / 2.0, cy + h / 2.0));
        quad(a, b, cc, d, col);
    };
    let tintify = |col: Color| match tint {
        Some(t) => Color::new(col.r * 0.4 + t.r * 0.6, col.g * 0.4 + t.g * 0.6, col.b * 0.4 + t.b * 0.6, 0.85),
        None => col,
    };
    let (w, h) = (spec.w, spec.h);
    if spec.round {
        let r = w / 2.0 * s;
        draw_circle(c.x + 1.5, c.y + 2.0, r, Color::new(0.0, 0.0, 0.0, 0.25));
        draw_circle(c.x, c.y, r, tintify(shade(spec.color, 0.8)));
        draw_circle(c.x, c.y, r * 0.72, tintify(shade((150, 154, 170), 1.0)));
        draw_circle(c.x, c.y, r * 0.5, tintify(shade(spec.color, 1.2)));
        for k in 0..6 {
            let a = angle + k as f32 * std::f32::consts::TAU / 6.0;
            let q = c + vec2(a.cos(), a.sin()) * r * 0.68;
            draw_line(c.x, c.y, q.x, q.y, (s * 1.4).max(1.5), tintify(shade((190, 194, 208), 1.0)));
        }
        draw_circle(c.x, c.y, r * 0.16, tintify(shade((220, 224, 236), 1.0)));
        return;
    }
    // Shadow, outline, body, bevels.
    rect(0.8, 1.2, w, h, Color::new(0.0, 0.0, 0.0, 0.25));
    rect(0.0, 0.0, w, h, tintify(shade(spec.color, 0.62)));
    let inset = 1.2;
    rect(0.0, 0.0, w - inset * 2.0, h - inset * 2.0, tintify(shade(spec.color, 1.0)));
    rect(0.0, -h / 2.0 + inset + 1.2, w - inset * 2.0, 2.4, tintify(shade(spec.color, 1.28)));
    rect(0.0, h / 2.0 - inset - 1.0, w - inset * 2.0, 2.0, tintify(shade(spec.color, 0.8)));
    match kind {
        BlockKind::Brick | BlockKind::Long | BlockKind::Cube => {
            let studs = (w / STUD).round() as i32;
            for i in 0..studs {
                let x = -w / 2.0 + STUD / 2.0 + i as f32 * STUD;
                rect(x, -h / 2.0 - 1.4, 7.0, 2.8, tintify(shade(spec.color, 0.8)));
                rect(x, -h / 2.0 - 1.6, 7.0, 1.6, tintify(shade(spec.color, 1.2)));
                if kind == BlockKind::Cube {
                    for j in 1..(h / STUD).round() as i32 {
                        let y = -h / 2.0 + j as f32 * STUD;
                        rect(x, y, 5.0, 5.0, tintify(shade(spec.color, 0.9)));
                    }
                }
            }
        }
        BlockKind::Plank => {
            for k in 1..6 {
                let x = -w / 2.0 + w * k as f32 / 6.0;
                let (a, b) = (p(x, -h / 2.0 + 1.0), p(x + 4.0, h / 2.0 - 1.0));
                draw_line(a.x, a.y, b.x, b.y, 1.0, tintify(shade(spec.color, 0.75)));
            }
        }
        BlockKind::Beam => {
            let teeth = (w / 12.0) as i32;
            for k in 0..teeth {
                let x0 = -w / 2.0 + 12.0 * k as f32;
                let (a, b) = if k % 2 == 0 { (p(x0, h / 2.0 - 2.0), p(x0 + 12.0, -h / 2.0 + 2.0)) } else { (p(x0, -h / 2.0 + 2.0), p(x0 + 12.0, h / 2.0 - 2.0)) };
                draw_line(a.x, a.y, b.x, b.y, (s * 1.6).max(1.5), tintify(shade((88, 96, 112), 1.0)));
            }
            for k in 0..=teeth {
                let q = p(-w / 2.0 + 12.0 * k as f32, 0.0);
                draw_circle(q.x, q.y, s * 1.4, tintify(shade((200, 206, 220), 1.0)));
            }
        }
        BlockKind::Wheel => {}
    }
    if heat > 0.0 {
        let t = (heat / 150.0).min(1.0);
        rect(0.0, 0.0, w, h, Color::new(1.0, 0.45, 0.05, 0.45 * t));
    }
}

pub fn draw_block(b: &Block, v: View) {
    let heat = if b.kind.spec().mat == Mat::Wood { b.heat as f32 } else { 0.0 };
    draw_block_shape(b.kind, v.to_screen(b.pos), v.s, b.angle, None, heat);
}

pub fn draw_car(car: &Car, v: View) {
    let m = car.kind.model();
    let d = car.dir;
    let s = v.s;
    let p = |lx: f32, ly: f32| v.to_screen(car.p + rot(car.angle, vec2(lx * d, ly)));
    let col = Color::from_rgba(m.color.0, m.color.1, m.color.2, 255);
    let dark = Color::new(col.r * 0.6, col.g * 0.6, col.b * 0.6, 1.0);
    let glass = Color::from_rgba(170, 225, 250, 255);
    let (hl, top, bot) = (m.hull_half, m.hull_top, m.hull_bottom);

    // Suspension arms and wheels first (behind the body).
    for (i, w) in car.wheels.iter().enumerate() {
        let a = v.to_screen(car.anchor(i));
        let c = v.to_screen(w.pos);
        draw_line(a.x, a.y, c.x, c.y, s * 2.6, Color::from_rgba(70, 74, 86, 255));
        let r = m.wheels[i].r * s;
        draw_circle(c.x, c.y, r, Color::from_rgba(26, 26, 32, 255));
        draw_circle(c.x, c.y, r * 0.62, Color::from_rgba(160, 164, 178, 255));
        for k in 0..5 {
            let ang = w.roll + k as f32 * std::f32::consts::TAU / 5.0;
            let q = c + vec2(ang.cos(), ang.sin()) * r * 0.6;
            draw_line(c.x, c.y, q.x, q.y, s * 1.2, Color::from_rgba(90, 94, 108, 255));
        }
        draw_circle(c.x, c.y, r * 0.2, Color::from_rgba(230, 232, 240, 255));
    }

    // Body: lower slab + cabin, shaped per model.
    let slab_top = top * 0.3;
    quad(p(-hl, slab_top), p(hl, slab_top), p(hl, bot), p(-hl, bot), col);
    quad(p(-hl, bot - 3.0), p(hl, bot - 3.0), p(hl, bot), p(-hl, bot), dark);
    match car.kind {
        ModelKind::Buggy => {
            quad(p(-hl * 0.55, top), p(hl * 0.3, top), p(hl * 0.55, slab_top), p(-hl * 0.7, slab_top), col);
            quad(p(-hl * 0.4, top + 1.5), p(hl * 0.22, top + 1.5), p(hl * 0.42, slab_top), p(-hl * 0.5, slab_top), glass);
        }
        ModelKind::Race => {
            quad(p(-hl * 0.2, top), p(hl * 0.15, top), p(hl * 0.5, slab_top), p(-hl * 0.45, slab_top), col);
            quad(p(-hl * 0.1, top + 1.0), p(hl * 0.1, top + 1.0), p(hl * 0.32, slab_top), p(-hl * 0.3, slab_top), glass);
            quad(p(-hl, top * 1.1), p(-hl * 0.7, top * 1.1), p(-hl * 0.7, top * 0.9), p(-hl, top * 0.9), dark);
            quad(p(-hl * 0.9, top * 1.0), p(-hl * 0.8, top * 1.0), p(-hl * 0.8, slab_top), p(-hl * 0.9, slab_top), dark);
        }
        ModelKind::Offroad => {
            quad(p(-hl * 0.7, top), p(hl * 0.45, top), p(hl * 0.6, slab_top), p(-hl * 0.8, slab_top), col);
            quad(p(-hl * 0.55, top + 2.0), p(hl * 0.35, top + 2.0), p(hl * 0.45, slab_top), p(-hl * 0.65, slab_top), glass);
            quad(p(-hl * 0.8, top - 1.0), p(hl * 0.4, top - 1.0), p(hl * 0.4, top + 0.5), p(-hl * 0.8, top + 0.5), dark);
            quad(p(hl, slab_top + 2.0), p(hl + 3.0, slab_top + 2.0), p(hl + 3.0, bot), p(hl, bot), dark);
        }
        ModelKind::Truck => {
            quad(p(hl * 0.25, top), p(hl * 0.95, top), p(hl, slab_top), p(hl * 0.25, slab_top), col);
            quad(p(hl * 0.45, top + 2.0), p(hl * 0.9, top + 2.0), p(hl * 0.95, slab_top), p(hl * 0.45, slab_top), glass);
            quad(p(-hl, top * 0.75), p(hl * 0.15, top * 0.75), p(hl * 0.15, slab_top), p(-hl, slab_top), dark);
        }
    }
    // Headlight.
    let head = p(hl, slab_top + (bot - slab_top) * 0.35);
    draw_circle(head.x, head.y, s * 1.8, Color::from_rgba(255, 250, 200, 255));
}

pub fn draw_flag(v: View, at: (i32, i32), col: Color, checkered: bool) {
    let base = v.to_screen(vec2(at.0 as f32, at.1 as f32 + 20.0));
    let top = v.to_screen(vec2(at.0 as f32, at.1 as f32 - 14.0));
    draw_line(base.x, base.y, top.x, top.y, (v.s * 1.5).max(2.0), Color::from_rgba(235, 235, 240, 255));
    let (w, h) = (v.s * 18.0, v.s * 12.0);
    draw_rectangle(top.x, top.y, w, h, col);
    if checkered {
        let n = 3;
        for i in 0..n {
            for j in 0..2 {
                if (i + j) % 2 == 0 {
                    draw_rectangle(top.x + w / n as f32 * i as f32, top.y + h / 2.0 * j as f32, w / n as f32, h / 2.0, WHITE);
                }
            }
        }
    }
    draw_circle(top.x, top.y, v.s * 1.6, Color::from_rgba(255, 220, 90, 255));
}

pub struct Boom {
    pub pos: Vec2,
    pub t: f32,
    pub color: Color,
}

pub fn draw_booms(booms: &[Boom], v: View) {
    for b in booms {
        let c = v.to_screen(b.pos);
        let k = b.t;
        let a = (1.0 - k).max(0.0);
        draw_circle(c.x, c.y, (6.0 + 34.0 * k) * v.s, Color::new(b.color.r, b.color.g, b.color.b, 0.45 * a));
        draw_circle(c.x, c.y, (4.0 + 20.0 * k) * v.s, Color::new(1.0, 0.95, 0.7, 0.7 * a));
        draw_circle_lines(c.x, c.y, (8.0 + 46.0 * k) * v.s, 2.5, Color::new(1.0, 1.0, 1.0, 0.8 * a));
    }
}
