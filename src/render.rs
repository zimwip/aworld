//! Drawing: sky, world texture with a glow shader, blocks, cars, markers.

use crate::blocks::{Block, BlockKind, Member, MemberMat, Spring, SpringKind, STUD};
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

// Clouds and steam are drawn from a density layer: blurred, then thresholded so
// the cells merge into soft, billowy blobs, lit from above.
const GAS_FRAG: &str = "#version 100
precision highp float;
varying vec2 uv;
uniform sampler2D Texture;
uniform vec2 texel;
float dens(vec2 p) {
    float d = 0.0;
    float ws = 0.0;
    for (int i = -3; i <= 3; i++) {
        for (int j = -3; j <= 3; j++) {
            vec4 s = texture2D(Texture, p + vec2(float(i), float(j)) * texel * 2.0);
            float w = exp(-float(i * i + j * j) / 7.0);
            d += s.a * w;
            ws += w;
        }
    }
    return d / ws;
}
void main() {
    float d = dens(uv);
    float a = smoothstep(0.16, 0.52, d);
    if (a <= 0.003) {
        gl_FragColor = vec4(0.0);
        return;
    }
    float up = dens(uv - vec2(0.0, texel.y * 6.0));
    float dn = dens(uv + vec2(0.0, texel.y * 6.0));
    float lightness = clamp(0.88 + (up - dn) * 0.9, 0.7, 1.08);
    vec3 col = vec3(0.97, 0.98, 1.0) * lightness;
    float alpha = a * 0.86;
    gl_FragColor = vec4(col * alpha, alpha);
}";

pub struct Renderer {
    pub image: Image,
    pub texture: Texture2D,
    material: Material,
    gas_image: Image,
    gas_texture: Texture2D,
    gas_material: Material,
    /// What the textures currently show, so unchanged frames cost nothing.
    rev_world: u64,
    rev_gas: u64,
    last_thermal: bool,
    has_gas: bool,
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
        let gas_image = Image::gen_image_color(W as u16, H as u16, Color::new(0.0, 0.0, 0.0, 0.0));
        let gas_texture = Texture2D::from_image(&gas_image);
        gas_texture.set_filter(FilterMode::Linear);
        let gas_material = load_material(
            ShaderSource::Glsl { vertex: VERT, fragment: GAS_FRAG },
            MaterialParams {
                pipeline_params: PipelineParams { color_blend: Some(blend), alpha_blend: Some(blend), ..Default::default() },
                uniforms: vec![UniformDesc::new("texel", UniformType::Float2)],
                ..Default::default()
            },
        )
        .expect("gas shader");
        Renderer { image, texture, material, gas_image, gas_texture, gas_material, rev_world: 0, rev_gas: 0, last_thermal: false, has_gas: false }
    }

    /// Fills the texture in parallel from a per-cell colour function.
    /// Returns how many pixels are not fully transparent.
    fn fill(image: &mut Image, color: impl Fn(usize, usize) -> [u8; 4] + Sync) -> usize {
        let px = image.get_image_data_mut();
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).min(8);
        let rows = H.div_ceil(threads);
        let color = &color;
        std::thread::scope(|scope| {
            let handles: Vec<_> = px
                .chunks_mut(rows * W)
                .enumerate()
                .map(|(i, chunk)| {
                    scope.spawn(move || {
                        let mut visible = 0;
                        for (j, out) in chunk.iter_mut().enumerate() {
                            let idx = i * rows * W + j;
                            *out = color(idx % W, idx / W);
                            visible += (out[3] != 0) as usize;
                        }
                        visible
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or(0)).sum()
        })
    }

    pub fn draw_world(&mut self, world: &World, v: View, thermal: bool) {
        let size = DrawTextureParams { dest_size: Some(vec2(W as f32 * v.s, H as f32 * v.s)), ..Default::default() };
        // Only rebuild the picture when the world (or the view mode) actually changed.
        if self.rev_world != world.revision || self.last_thermal != thermal {
            if thermal {
                Self::fill(&mut self.image, |x, y| world.thermal_color(x, y));
            } else {
                Self::fill(&mut self.image, |x, y| world.color(x, y));
            }
            self.texture.update(&self.image);
            self.rev_world = world.revision;
            self.last_thermal = thermal;
        }
        if thermal {
            draw_texture_ex(&self.texture, v.ox, v.oy, WHITE, size);
            return;
        }
        self.material.set_uniform("texel", vec2(1.0 / W as f32, 1.0 / H as f32));
        gl_use_material(&self.material);
        draw_texture_ex(&self.texture, v.ox, v.oy, WHITE, size);
        gl_use_default_material();
    }

    /// Clouds and steam as soft blobs, drawn over the landscape.
    pub fn draw_gas(&mut self, world: &World, v: View) {
        if self.rev_gas != world.revision {
            self.has_gas = Self::fill(&mut self.gas_image, |x, y| world.gas_color(x, y)) > 0;
            if self.has_gas {
                self.gas_texture.update(&self.gas_image);
            }
            self.rev_gas = world.revision;
        }
        // No clouds, steam or smoke: skip the expensive blur pass altogether.
        if !self.has_gas {
            return;
        }
        self.gas_material.set_uniform("texel", vec2(1.0 / W as f32, 1.0 / H as f32));
        gl_use_material(&self.gas_material);
        draw_texture_ex(
            &self.gas_texture,
            v.ox,
            v.oy,
            WHITE,
            DrawTextureParams { dest_size: Some(vec2(W as f32 * v.s, H as f32 * v.s)), ..Default::default() },
        );
        gl_use_default_material();
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
pub fn draw_block_shape(kind: BlockKind, c: Vec2, s: f32, angle: f32, tint: Option<Color>, heat: f32, rust: f32, stress: Option<f32>) {
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
    if kind.is_glass() {
        let a = |k: f32| Color::new(0.72 * k, 0.9 * k, 0.98 * k, 0.9);
        rect(0.0, 0.0, w, h, Color::new(0.7, 0.88, 0.97, 0.33));
        rect(0.0, -h / 2.0 + 0.6, w, 1.2, a(1.0));
        rect(0.0, h / 2.0 - 0.6, w, 1.2, a(0.75));
        rect(-w / 2.0 + 0.6, 0.0, 1.2, h, a(0.9));
        rect(w / 2.0 - 0.6, 0.0, 1.2, h, a(0.75));
        for k in 0..(w / STUD).round() as i32 {
            let x = -w / 2.0 + STUD / 2.0 + k as f32 * STUD;
            rect(x, -h / 2.0 - 1.1, 5.6, 2.2, Color::new(0.7, 0.88, 0.97, 0.55));
        }
        let (s1, s2) = (p(-w / 2.0 + 3.0, h / 2.0 - 2.0), p(-w / 2.0 + 3.0 + h * 0.6, -h / 2.0 + 2.0));
        draw_line(s1.x, s1.y, s2.x, s2.y, (s * 1.2).max(1.2), Color::new(1.0, 1.0, 1.0, 0.7));
        return;
    }
    // Shadow, outline, body, bevels.
    rect(0.8, 1.2, w, h, Color::new(0.0, 0.0, 0.0, 0.25));
    rect(0.0, 0.0, w, h, tintify(shade(spec.color, 0.62)));
    let inset = 0.9;
    rect(0.0, 0.0, w - inset * 2.0, h - inset * 2.0, tintify(shade(spec.color, 1.0)));
    rect(0.0, -h / 2.0 + inset + 1.0, w - inset * 2.0, 2.0, tintify(shade(spec.color, 1.28)));
    rect(0.0, h / 2.0 - inset - 0.8, w - inset * 2.0, 1.6, tintify(shade(spec.color, 0.8)));
    match kind {
        BlockKind::Brick | BlockKind::Long | BlockKind::Cube => {
            let studs = (w / STUD).round() as i32;
            for i in 0..studs {
                let x = -w / 2.0 + STUD / 2.0 + i as f32 * STUD;
                rect(x, -h / 2.0 - 1.1, 5.6, 2.2, tintify(shade(spec.color, 0.8)));
                rect(x, -h / 2.0 - 1.3, 5.6, 1.3, tintify(shade(spec.color, 1.2)));
                if kind == BlockKind::Cube {
                    for j in 1..(h / STUD).round() as i32 {
                        let y = -h / 2.0 + j as f32 * STUD;
                        rect(x, y, 4.0, 4.0, tintify(shade(spec.color, 0.9)));
                    }
                }
            }
        }
        BlockKind::Plank => {
            for k in 1..6 {
                let x = -w / 2.0 + w * k as f32 / 6.0;
                let (a, b) = (p(x, -h / 2.0 + 0.8), p(x + 3.0, h / 2.0 - 0.8));
                draw_line(a.x, a.y, b.x, b.y, 1.0, tintify(shade(spec.color, 0.75)));
            }
        }
        BlockKind::Beam | BlockKind::SteelBeam => {
            let teeth = (w / STUD) as i32;
            for k in 0..teeth {
                let x0 = -w / 2.0 + STUD * k as f32;
                let (a, b) = if k % 2 == 0 { (p(x0, h / 2.0 - 1.5), p(x0 + STUD, -h / 2.0 + 1.5)) } else { (p(x0, -h / 2.0 + 1.5), p(x0 + STUD, h / 2.0 - 1.5)) };
                draw_line(a.x, a.y, b.x, b.y, (s * 1.2).max(1.2), tintify(shade((88, 96, 112), 1.0)));
            }
            for k in 0..=teeth {
                let q = p(-w / 2.0 + STUD * k as f32, 0.0);
                draw_circle(q.x, q.y, s * 1.0, tintify(shade((200, 206, 220), 1.0)));
            }
        }
        BlockKind::Wheel | BlockKind::Glass | BlockKind::GlassHalf | BlockKind::GlassChip => {}
    }
    if rust > 0.0 {
        rect(0.0, 0.0, w, h, Color::new(0.62, 0.30, 0.10, 0.62 * rust.min(1.0)));
    }
    if let Some(st) = stress {
        let (r, g, b) = stress_rgb(st);
        rect(0.0, 0.0, w, h, Color::new(r, g, b, 0.72));
    }
    if heat > 0.0 {
        let t = (heat / 150.0).min(1.0);
        rect(0.0, 0.0, w, h, Color::new(1.0, 0.45, 0.05, 0.45 * t));
    }
}

/// Green (relaxed) through yellow to red (about to break).
pub fn stress_rgb(t: f32) -> (f32, f32, f32) {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        (0.2 + 1.6 * t * 0.9, 0.85, 0.25)
    } else {
        (0.95, 0.85 - (t - 0.5) * 1.6, 0.2)
    }
}

pub fn draw_block(b: &Block, v: View, stress: Option<f32>) {
    let heat = if b.kind.spec().mat == Mat::Wood { b.heat as f32 } else { 0.0 };
    draw_block_shape(b.kind, v.to_screen(b.pos), v.s, b.angle, None, heat, b.rust, stress);
}

pub fn draw_car(car: &Car, v: View) {
    let m = car.kind.model();
    let d = car.dir;
    let s = v.s;
    let sc = crate::car::S;
    let p = |lx: f32, ly: f32| v.to_screen(car.p + rot(car.angle, vec2(lx * d, ly) * sc));
    let rgb = car.color_rgb();
    let col = Color::from_rgba(rgb.0, rgb.1, rgb.2, 255);
    let dark = Color::new(col.r * 0.6, col.g * 0.6, col.b * 0.6, 1.0);
    let glass = Color::from_rgba(170, 225, 250, 255);
    let (hl, top, bot) = (m.hull_half, m.hull_top, m.hull_bottom);

    // Suspension arms and wheels first (behind the body).
    for (i, w) in car.wheels.iter().enumerate() {
        let a = v.to_screen(car.anchor(i));
        let c = v.to_screen(w.pos);
        draw_line(a.x, a.y, c.x, c.y, s * 1.9, Color::from_rgba(70, 74, 86, 255));
        let r = car.wheel_r(i) * s;
        draw_circle(c.x, c.y, r, Color::from_rgba(26, 26, 32, 255));
        draw_circle(c.x, c.y, r * 0.62, Color::from_rgba(160, 164, 178, 255));
        for k in 0..5 {
            let ang = w.roll + k as f32 * std::f32::consts::TAU / 5.0;
            let q = c + vec2(ang.cos(), ang.sin()) * r * 0.6;
            draw_line(c.x, c.y, q.x, q.y, s * 0.9, Color::from_rgba(90, 94, 108, 255));
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
    draw_circle(head.x, head.y, s * 1.2, Color::from_rgba(255, 250, 200, 255));
}

pub fn draw_flag(v: View, at: (i32, i32), col: Color, checkered: bool) {
    let base = v.to_screen(vec2(at.0 as f32, at.1 as f32 + 26.0));
    let top = v.to_screen(vec2(at.0 as f32, at.1 as f32 - 8.0));
    draw_line(base.x, base.y, top.x, top.y, (v.s * 1.2).max(1.5), Color::from_rgba(235, 235, 240, 255));
    let (w, h) = (v.s * 14.0, v.s * 9.0);
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
    draw_circle(top.x, top.y, v.s * 1.2, Color::from_rgba(255, 220, 90, 255));
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
        draw_circle(c.x, c.y, (4.0 + 24.0 * k) * v.s, Color::new(b.color.r, b.color.g, b.color.b, 0.45 * a));
        draw_circle(c.x, c.y, (3.0 + 14.0 * k) * v.s, Color::new(1.0, 0.95, 0.7, 0.7 * a));
        draw_circle_lines(c.x, c.y, (6.0 + 32.0 * k) * v.s, 2.0, Color::new(1.0, 1.0, 1.0, 0.8 * a));
    }
}

/// A cable as a polyline with a highlight and round fixings.
pub fn draw_cable(pts: &[Vec2], v: View, snapped: bool, valid: bool, rgb: (u8, u8, u8), stress: Option<f32>) {
    if pts.len() < 2 {
        return;
    }
    let (sr, sg, sb) = stress.map(stress_rgb).unwrap_or((0.0, 0.0, 0.0));
    let col = if !valid {
        Color::new(0.95, 0.3, 0.3, 0.9)
    } else if snapped {
        Color::from_rgba(150, 60, 50, 255)
    } else if stress.is_some() {
        Color::new(sr, sg, sb, 1.0)
    } else {
        Color::from_rgba(rgb.0, rgb.1, rgb.2, 255)
    };
    let s = v.s;
    for w in pts.windows(2) {
        let (a, b) = (v.to_screen(w[0]), v.to_screen(w[1]));
        draw_line(a.x, a.y, b.x, b.y, (s * 2.4).max(2.0), col);
        draw_line(a.x, a.y - s * 0.5, b.x, b.y - s * 0.5, (s * 0.8).max(1.0), Color::new(1.0, 1.0, 1.0, 0.25));
    }
    for p in [pts[0], pts[pts.len() - 1]] {
        let c = v.to_screen(p);
        draw_circle(c.x, c.y, s * 2.4, Color::from_rgba(200, 204, 214, 255));
        draw_circle(c.x, c.y, s * 1.1, col);
    }
}

/// The sagging line shown for a cable that has no physics yet.
pub fn cable_preview(a: Vec2, b: Vec2, len: f32) -> Vec<Vec2> {
    let n = 24;
    let sag = ((len * len - (b - a).length_squared()).max(0.0)).sqrt() * 0.55;
    (0..=n)
        .map(|i| {
            let u = i as f32 / n as f32;
            a + (b - a) * u + vec2(0.0, sag * 4.0 * u * (1.0 - u))
        })
        .collect()
}

/// A free beam: a bar with a pin at each end; coloured by load in the effort view.
pub fn draw_member(m: &Member, v: View, stress: bool) {
    let (a, b) = (v.to_screen(m.p0), v.to_screen(m.p1));
    let th = m.mat.thickness() * v.s;
    let (cr, cg, cb) = m.mat.color();
    let base = Color::from_rgba(cr, cg, cb, 255);
    let (r, g, bl) = stress_rgb(m.stress);
    let col = if stress { Color::new(r, g, bl, 1.0) } else { base };
    let dark = Color::new(col.r * 0.55, col.g * 0.55, col.b * 0.55, 1.0);
    let d = (b - a).normalize_or_zero();
    let n = vec2(-d.y, d.x);
    draw_line(a.x + 1.0, a.y + 2.0, b.x + 1.0, b.y + 2.0, th, Color::new(0.0, 0.0, 0.0, 0.25));
    draw_line(a.x, a.y, b.x, b.y, th, dark);
    draw_line(a.x, a.y, b.x, b.y, th * 0.68, col);
    let hi = n * (-th * 0.18);
    draw_line(a.x + hi.x, a.y + hi.y, b.x + hi.x, b.y + hi.y, (th * 0.18).max(1.0), Color::new(1.0, 1.0, 1.0, 0.28));
    if m.is_deck() {
        // The floor a car drives on gets a bright running surface.
        let top = n * (-th * 0.5);
        draw_line(a.x + top.x, a.y + top.y, b.x + top.x, b.y + top.y, (th * 0.16).max(1.2), Color::new(1.0, 0.95, 0.6, 0.9));
    }
    match m.mat {
        MemberMat::Wood => {
            let len = (b - a).length();
            let mut t = 12.0;
            while t < len - 6.0 {
                let p = a + d * t;
                draw_line(p.x, p.y, p.x + d.x * 5.0, p.y + d.y * 5.0, 1.0, Color::new(0.35, 0.22, 0.1, 0.6));
                t += 14.0;
            }
        }
        MemberMat::Iron | MemberMat::Steel => {
            let len = (b - a).length();
            let mut t = 8.0;
            while t < len - 4.0 {
                let p = a + d * t;
                draw_circle(p.x, p.y, th * 0.12 + 0.6, Color::new(1.0, 1.0, 1.0, 0.35));
                t += 18.0;
            }
        }
    }
    for p in [a, b] {
        draw_circle(p.x, p.y, th * 0.78, Color::from_rgba(226, 228, 236, 255));
        draw_circle(p.x, p.y, th * 0.34, Color::from_rgba(60, 64, 78, 255));
    }
}

/// A coil spring or an elastic cord between two points. The number of turns stays
/// the same however far it is pulled, so a stretched one looks stretched.
pub fn draw_spring_between(p0: Vec2, p1: Vec2, rest: f32, kind: SpringKind, v: View, stress: Option<f32>, alpha: f32) {
    let (a, b) = (v.to_screen(p0), v.to_screen(p1));
    let len = (b - a).length();
    if len < 2.0 {
        return;
    }
    let d = (b - a) / len;
    let n = vec2(-d.y, d.x);
    let turns = ((rest / 5.0) as i32).max(5);
    let (amp, thick) = match kind {
        SpringKind::Coil => (4.2 * v.s, 1.7 * v.s),
        SpringKind::Elastic => (2.0 * v.s, 1.2 * v.s),
    };
    let (cr, cg, cb) = kind.color();
    let mut col = Color::from_rgba(cr, cg, cb, (255.0 * alpha) as u8);
    if let Some(st) = stress {
        let (r, g, bl) = stress_rgb(st);
        col = Color::new(r, g, bl, alpha);
    }
    // Straight lead-ins at both ends, then the coil in between.
    let lead = (len * 0.1).min(10.0 * v.s);
    let mut prev = a;
    let segs = turns * 2;
    for i in 1..=segs {
        let t = i as f32 / segs as f32;
        let along = a + d * (lead + (len - 2.0 * lead) * t);
        let side = if i == segs { 0.0 } else if i % 2 == 0 { amp } else { -amp };
        let q = if i == segs { b } else { along + n * side };
        draw_line(prev.x, prev.y, q.x, q.y, thick, col);
        prev = q;
    }
    draw_line(a.x, a.y, a.x + d.x * lead, a.y + d.y * lead, thick, col);
    for p in [a, b] {
        draw_circle(p.x, p.y, 2.6 * v.s, Color::from_rgba(226, 228, 236, (255.0 * alpha) as u8));
    }
}

pub fn draw_spring(sp: &Spring, v: View, stress: bool) {
    draw_spring_between(sp.p0, sp.p1, sp.rest, sp.kind, v, if stress { Some(sp.stress) } else { None }, 1.0);
}
