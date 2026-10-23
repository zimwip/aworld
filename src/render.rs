//! Drawing: sky, world texture with a glow shader, blocks, cars, markers.

use crate::blocks::{Block, BlockKind, Member, MemberMat, Spring, SpringKind, STUD};
use crate::car::{Car, ModelKind};
use crate::world::sleep::{CH, CS, CW};
use crate::world::{Mat, World, H, W};
use macroquad::miniquad::{BlendFactor, BlendState, BlendValue, Equation, PipelineParams};
use macroquad::prelude::*;

/// Where the world sits on screen.
#[derive(Clone, Copy)]
pub struct View {
    pub ox: f32,
    pub oy: f32,
    pub s: f32,
    /// The part of the window the world is shown in.
    pub area: Rect,
}

impl View {
    /// The world fills the whole area: a little is cropped (sides centred, the sky rather than the ground)
    /// when the window is not the same shape as the world.
    pub fn fit(area: Rect) -> View {
        let s = (area.w / W as f32).max(area.h / H as f32);
        View { ox: area.x + (area.w - W as f32 * s) / 2.0, oy: area.y + area.h - H as f32 * s, s, area }
    }

    /// What can be seen of the world, in screen pixels.
    pub fn visible(&self) -> Rect {
        self.area
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
// `Light` is the light map (see `light_map`): its green channel is the hot air, which bends the
// light passing through it, so everything seen through it wobbles (heat haze).
// `sky` is (sun x, sun strength, moon x, moon strength), x from 0 to 1 across the world.
const FRAG: &str = "#version 100
precision highp float;
varying vec2 uv;
uniform sampler2D Texture;
uniform sampler2D Light;
uniform vec2 texel;
uniform float time;
uniform vec4 sky;
float hash(vec2 p) {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}
void main() {
    // Heat haze: hot air shifts what is seen through it, in ripples that rise with the warm air.
    float heat = texture2D(Light, uv).g;
    vec2 cellf = uv / texel;
    vec2 p = uv + vec2(sin(cellf.y * 0.45 + time * 9.0 + cellf.x * 0.08), 0.5 * cos(cellf.x * 0.3 - time * 6.0)) * texel * heat * 1.8;
    vec4 c = texture2D(Texture, p);
    vec3 glow = vec3(0.0);
    float wsum = 0.0;
    for (int i = -3; i <= 3; i++) {
        for (int j = -3; j <= 3; j++) {
            vec4 s = texture2D(Texture, p + vec2(float(i), float(j)) * texel * 1.6);
            float e = step(0.93, s.r) * step(s.b, 0.3) * step(0.5, s.a);
            float w = 1.0 / (1.0 + float(i * i + j * j));
            glow += s.rgb * e * w;
            wsum += w;
        }
    }
    glow = glow / wsum;
    vec4 up = texture2D(Texture, p - vec2(0.0, texel.y));
    float edge = c.a * (1.0 - up.a);
    // Water shimmers and embers, lava and flames flicker: a noise per cell that changes twenty
    // times a second, so the texture itself only changes where the cells do.
    vec2 cell = floor(p / texel);
    float n = fract(sin(dot(cell, vec2(12.9898, 78.233)) + floor(time * 20.0) * 3.7) * 43758.5453) - 0.5;
    float water = step(0.78, c.a) * step(c.a, 0.83) * step(c.r, c.b);
    float hot = step(0.93, c.r) * step(c.b, 0.3) * step(0.5, c.a);
    c.g += n * (0.06 * water + 0.36 * hot) * c.a;
    vec3 rgb = c.rgb + vec3(0.18) * edge * c.a + glow * 2.2;
    // Glints: the little waves of the top few rows of water catch the sun (or the moon) and flash,
    // most of all under the sun, the way a lake sparkles.
    float a1 = texture2D(Texture, p - vec2(0.0, texel.y)).a;
    float a2 = texture2D(Texture, p - vec2(0.0, texel.y * 3.0)).a;
    float a3 = texture2D(Texture, p - vec2(0.0, texel.y * 6.0)).a;
    float surf = water * max(max(step(a1, 0.05), 0.75 * step(a2, 0.05)), 0.4 * step(a3, 0.05));
    if (surf > 0.0) {
        float lit = sky.y * (0.35 + 0.65 * exp(-abs(uv.x - sky.x) * 4.0)) + 0.6 * sky.w * exp(-abs(uv.x - sky.z) * 6.0);
        // A glint is a short horizontal streak: a wave face a few cells wide catching the light.
        vec2 facet = vec2(floor(cell.x / 3.0), cell.y);
        float t = time * 3.0 + hash(facet) * 9.0;
        float g = hash(facet + floor(t) * vec2(7.13, 3.71));
        float flash = smoothstep(0.86, 1.0, g) * (1.0 - fract(t));
        float wave = 0.5 + 0.5 * sin(cell.x * 0.5 + time * 3.0 + sin(cell.x * 0.13 - time));
        rgb += vec3(1.0, 0.97, 0.86) * surf * lit * (flash * 2.4 + wave * 0.16);
    }
    float a = max(c.a, clamp(length(glow) * 2.0, 0.0, 1.0));
    // In clear air the haze shows itself as faint bright and dark ripples.
    float rip = sin(cellf.y * 0.5 + time * 10.0 + cellf.x * 0.11) * heat * 0.07 * (1.0 - a);
    gl_FragColor = vec4(rgb + vec3(max(rip, 0.0)), a + max(-rip, 0.0));
}";

// At night a bluish veil covers the picture, except where fire, embers and lava light it: the light
// map (one texel per `LS` cells) is blurred into a wide, flickering warm pool of light.
const NIGHT_FRAG: &str = "#version 100
precision highp float;
varying vec2 uv;
uniform sampler2D Texture;
uniform vec2 texel;
uniform float time;
uniform float night;
void main() {
    float l = 0.0;
    float ws = 0.0;
    for (int i = -3; i <= 3; i++) {
        for (int j = -3; j <= 3; j++) {
            float w = exp(-float(i * i + j * j) / 6.0);
            l += texture2D(Texture, uv + vec2(float(i), float(j)) * texel * 2.2).r * w;
            ws += w;
        }
    }
    l = l / ws * 4.5;
    l *= 0.88 + 0.08 * sin(time * 13.0 + uv.x * 40.0) + 0.06 * sin(time * 23.0 - uv.y * 30.0);
    l = clamp(l, 0.0, 1.0);
    float a = 0.46 * night * (1.0 - 0.9 * l);
    vec3 warm = vec3(1.0, 0.56, 0.22) * l * 0.4 * night;
    gl_FragColor = vec4(vec3(0.02, 0.05, 0.2) * a + warm, a);
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

/// What the world picture shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Normal = 0,
    Thermal = 1,
    Pressure = 2,
    Humidity = 3,
}

pub struct Renderer {
    pub image: Image,
    pub texture: Texture2D,
    material: Material,
    gas_image: Image,
    gas_texture: Texture2D,
    gas_material: Material,
    /// What the textures currently show, so unchanged frames cost nothing.
    /// Trees are a layer of their own so they can sway: (x, y, height above ground, colour).
    veg_image: Image,
    veg_texture: Texture2D,
    /// The tree cells of each sleeping zone, rebuilt only for the zones that changed.
    veg_zones: Vec<Vec<(u16, u16, f32, [u8; 4])>>,
    veg_tick: i64,
    /// The world's step count when the landscape and the clouds were last drawn: only the zones
    /// that changed since are recoloured (see `World::zones`).
    drawn_stamp: u64,
    gas_stamp: u64,
    /// Visible gas pixels of each zone.
    gas_zones: Vec<u32>,
    /// Which row of zones is recoloured this frame even if it did not change (temperature tints,
    /// soil moisture): the whole picture is refreshed every `CH` frames.
    band: usize,
    /// The landscape must be redrawn in full (first frame, or back from another view).
    full: bool,
    rev_world: u64,
    rev_gas: u64,
    last_mode: u8,
    /// The pressure of every cell, recomputed when the picture changes (pressure view only).
    pressure: Vec<f32>,
    has_gas: bool,
    /// Light from fire, embers and lava (red) and hot air (green), one texel per `LS`×`LS` cells.
    light_image: Image,
    light_texture: Texture2D,
    night_material: Material,
    light_tick: i64,
}

/// Cells per texel of the light map.
pub const LS: usize = 6;
const LW: usize = W / LS;
const LH: usize = H / LS;

/// The light map: for each `LS`×`LS` square, how much it glows (fire, embers, lava) in red and how
/// hot its air is (for the heat haze) in green. Both saturate softly, so a big blaze is not
/// blinding and a single ember still shows.
pub fn light_map(world: &World, out: &mut [[u8; 4]]) {
    for ty in 0..LH {
        for tx in 0..LW {
            let (mut glow, mut heat) = (0.0f32, 0.0f32);
            for y in ty * LS..(ty + 1) * LS {
                for x in tx * LS..(tx + 1) * LS {
                    let i = y * W + x;
                    match world.cells[i].m {
                        Mat::Fire => glow += 1.0,
                        Mat::Lava => glow += 1.3,
                        Mat::Ember => glow += 0.5,
                        Mat::Empty | Mat::Steam | Mat::Smoke => heat += ((world.temp[i] - 40.0) / 160.0).clamp(0.0, 1.0),
                        _ => {}
                    }
                }
            }
            let soft = |v: f32, k: f32| (255.0 * (1.0 - (-v / k).exp())) as u8;
            // Above a flame the air is hot too, even before the simulation has warmed it.
            out[ty * LW + tx] = [soft(glow, 5.0), soft(heat + glow * 0.5, 6.0), 0, 255];
        }
    }
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
                uniforms: vec![UniformDesc::new("texel", UniformType::Float2), UniformDesc::new("time", UniformType::Float1), UniformDesc::new("sky", UniformType::Float4)],
                textures: vec!["Light".to_string()],
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
        let night_material = load_material(
            ShaderSource::Glsl { vertex: VERT, fragment: NIGHT_FRAG },
            MaterialParams {
                pipeline_params: PipelineParams { color_blend: Some(blend), alpha_blend: Some(blend), ..Default::default() },
                uniforms: vec![UniformDesc::new("texel", UniformType::Float2), UniformDesc::new("time", UniformType::Float1), UniformDesc::new("night", UniformType::Float1)],
                ..Default::default()
            },
        )
        .expect("night shader");
        let light_image = Image::gen_image_color(LW as u16, LH as u16, Color::new(0.0, 0.0, 0.0, 1.0));
        let light_texture = Texture2D::from_image(&light_image);
        light_texture.set_filter(FilterMode::Linear);
        let veg_image = Image::gen_image_color(W as u16, H as u16, Color::new(0.0, 0.0, 0.0, 0.0));
        let veg_texture = Texture2D::from_image(&veg_image);
        veg_texture.set_filter(FilterMode::Linear);
        Renderer {
            veg_image,
            veg_texture,
            veg_zones: vec![vec![]; CW * CH],
            veg_tick: -1,
            drawn_stamp: 0,
            gas_stamp: 0,
            gas_zones: vec![0; CW * CH],
            band: 0,
            full: true,
            image,
            texture,
            material,
            gas_image,
            gas_texture,
            gas_material,
            rev_world: u64::MAX,
            rev_gas: u64::MAX,
            last_mode: 0,
            pressure: vec![],
            has_gas: false,
            light_image,
            light_texture,
            night_material,
            light_tick: -1,
        }
    }

    /// Fills the texture in parallel from a per-cell colour function.
    /// Returns how many pixels are not fully transparent.
    fn fill(image: &mut Image, color: impl Fn(usize, usize) -> [u8; 4] + Sync) -> usize {
        let px = image.get_image_data_mut();
        // The web build has no threads: fill the rows one after the other.
        #[cfg(target_arch = "wasm32")]
        {
            let mut visible = 0;
            for (idx, out) in px.iter_mut().enumerate() {
                *out = color(idx % W, idx / W);
                visible += (out[3] != 0) as usize;
            }
            visible
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
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
    }

    /// Recolours one sleeping zone of the picture. Returns how many pixels are not fully transparent.
    fn fill_zone(image: &mut Image, k: usize, color: impl Fn(usize, usize) -> [u8; 4]) -> u32 {
        let px = image.get_image_data_mut();
        let (x0, y0) = ((k % CW) * CS, (k / CW) * CS);
        let mut visible = 0;
        for y in y0..(y0 + CS).min(H) {
            for x in x0..(x0 + CS).min(W) {
                let c = color(x, y);
                visible += (c[3] != 0) as u32;
                px[y * W + x] = c;
            }
        }
        visible
    }

    /// The zones whose cells may have changed since step count `since`.
    fn changed_zones(world: &World, since: u64) -> Vec<usize> {
        (0..CW * CH).filter(|&k| world.zones.changed_since(k, since)).collect()
    }

    /// The tree cells of zone `k`: (x, y, height above the ground, colour).
    fn veg_of_zone(world: &World, k: usize) -> Vec<(u16, u16, f32, [u8; 4])> {
        let (x0, y0) = ((k % CW) * CS, (k / CW) * CS);
        let mut out = vec![];
        for y in y0..(y0 + CS).min(H) {
            for x in x0..(x0 + CS).min(W) {
                if world.cell_mat(x, y).is_vegetation() {
                    let mut h = 0usize;
                    while h < 90 && y + h + 1 < H && {
                        let m = world.cell_mat(x, y + h + 1);
                        m.is_vegetation() || !m.is_hard()
                    } {
                        h += 1;
                    }
                    out.push((x as u16, y as u16, h as f32, world.color(x, y)));
                }
            }
        }
        out
    }

    pub fn draw_world(&mut self, world: &World, v: View, mode: RenderMode, time: f32) {
        let size = DrawTextureParams { dest_size: Some(vec2(W as f32 * v.s, H as f32 * v.s)), ..Default::default() };
        let id = mode as u8;
        // Only rebuild the picture when the world (or the view mode) actually changed.
        if self.rev_world != world.revision || self.last_mode != id {
            match mode {
                RenderMode::Thermal => {
                    Self::fill(&mut self.image, |x, y| world.thermal_color(x, y));
                }
                RenderMode::Pressure => {
                    self.pressure = world.pressure_field();
                    let field = &self.pressure;
                    Self::fill(&mut self.image, |x, y| world.pressure_color(field, x, y));
                }
                RenderMode::Humidity => {
                    self.pressure = world.humidity_field();
                    let field = &self.pressure;
                    Self::fill(&mut self.image, |x, y| world.humidity_color(field, x, y));
                }
                RenderMode::Normal => {
                    // Only the zones that changed, plus one row of zones in turn for the slow
                    // changes (temperature tints) that do not wake a zone.
                    // A fresh world (a new game, a loaded one) counts its steps from the start again.
                    if self.full || self.last_mode != id || world.zones.stamp() < self.drawn_stamp {
                        Self::fill(&mut self.image, |x, y| world.color_without_trees(x, y));
                        for k in 0..CW * CH {
                            self.veg_zones[k] = Self::veg_of_zone(world, k);
                        }
                        self.full = false;
                    } else {
                        let mut zones = Self::changed_zones(world, self.drawn_stamp);
                        self.band = (self.band + 1) % CH;
                        zones.extend((0..CW).map(|i| self.band * CW + i));
                        for k in zones {
                            Self::fill_zone(&mut self.image, k, |x, y| world.color_without_trees(x, y));
                            self.veg_zones[k] = Self::veg_of_zone(world, k);
                        }
                    }
                    self.veg_tick = -1;
                    self.drawn_stamp = world.zones.stamp();
                }
            }
            if mode != RenderMode::Normal {
                self.full = true;
            }
            self.texture.update(&self.image);
            self.rev_world = world.revision;
            self.last_mode = id;
        }
        if mode != RenderMode::Normal {
            draw_texture_ex(&self.texture, v.ox, v.oy, WHITE, size);
            return;
        }
        self.update_trees(world, time);
        self.update_light(world, time);
        // The sun and the moon follow the same arcs as in the backdrop.
        let phase = world.phase();
        let (u, sun) = if phase < 0.5 { (phase / 0.5, true) } else { ((phase - 0.5) / 0.5, false) };
        let x = 0.08 + 0.84 * u;
        let height = (std::f32::consts::PI * u).sin();
        let sky = if sun { vec4(x, height.sqrt(), 0.0, 0.0) } else { vec4(0.0, 0.0, x, height.sqrt()) };
        self.material.set_uniform("texel", vec2(1.0 / W as f32, 1.0 / H as f32));
        self.material.set_uniform("time", time);
        self.material.set_uniform("sky", sky);
        self.material.set_texture("Light", self.light_texture.clone());
        gl_use_material(&self.material);
        draw_texture_ex(&self.texture, v.ox, v.oy, WHITE, size.clone());
        draw_texture_ex(&self.veg_texture, v.ox, v.oy, WHITE, size);
        gl_use_default_material();
    }

    /// Rebuilds the light map, fifteen times a second at most.
    fn update_light(&mut self, world: &World, time: f32) {
        let tick = (time * 15.0) as i64;
        if tick == self.light_tick {
            return;
        }
        self.light_tick = tick;
        light_map(world, self.light_image.get_image_data_mut());
        self.light_texture.update(&self.light_image);
    }

    /// The night: a bluish veil over the whole picture, lifted around fires, embers and lava,
    /// which cast a warm, flickering light on everything near them.
    pub fn draw_night(&mut self, v: View, day: f32, time: f32) {
        let night = 1.0 - day;
        if night <= 0.02 {
            return;
        }
        self.night_material.set_uniform("texel", vec2(1.0 / LW as f32, 1.0 / LH as f32));
        self.night_material.set_uniform("time", time);
        self.night_material.set_uniform("night", night);
        gl_use_material(&self.night_material);
        let r = v.rect();
        draw_texture_ex(&self.light_texture, r.x, r.y, WHITE, DrawTextureParams { dest_size: Some(vec2(r.w, r.h)), ..Default::default() });
        gl_use_default_material();
    }

    /// Trees lean with the wind and wave about, more the higher the leaf or branch is
    /// above the ground. Purely visual: the cells themselves do not move.
    fn update_trees(&mut self, world: &World, time: f32) {
        let tick = (time * 12.0) as i64;
        if tick == self.veg_tick {
            return;
        }
        self.veg_tick = tick;
        let px = self.veg_image.get_image_data_mut();
        px.fill([0; 4]);
        let w = world.wind;
        for &(x, y, h, col) in self.veg_zones.iter().flatten() {
            let hf = (h / 40.0).powf(1.4).min(2.5);
            let wave = (time * 1.9 + x as f32 * 0.07 + h * 0.03).sin() + 0.5 * (time * 3.3 + y as f32 * 0.11).sin();
            let dx = (w * 1.4 + wave * (0.35 + 0.9 * w.abs())) * hf;
            let nx = (x as i32 + dx.round() as i32).clamp(0, W as i32 - 1) as usize;
            px[y as usize * W + nx] = col;
        }
        self.veg_texture.update(&self.veg_image);
    }

    /// Clouds and steam as soft blobs, drawn over the landscape.
    pub fn draw_gas(&mut self, world: &World, v: View) {
        if self.rev_gas != world.revision {
            // The gas picture depends only on the cells: recolour the zones that changed.
            let zones = if self.rev_gas == u64::MAX || world.zones.stamp() < self.gas_stamp { (0..CW * CH).collect() } else { Self::changed_zones(world, self.gas_stamp) };
            let was = self.has_gas;
            for k in zones {
                self.gas_zones[k] = Self::fill_zone(&mut self.gas_image, k, |x, y| world.gas_color(x, y));
            }
            self.has_gas = self.gas_zones.iter().any(|&n| n > 0);
            if self.has_gas || was {
                self.gas_texture.update(&self.gas_image);
            }
            self.rev_gas = world.revision;
            self.gas_stamp = world.zones.stamp();
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
        BlockKind::Brick | BlockKind::Long | BlockKind::Post | BlockKind::Cube => {
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

/// A free beam: a bar with a pin at each end. In the effort view it is coloured by its load, with
/// the share of what it can take written on it. In every view a loaded beam shows it: it reddens
/// past half its strength, a floor beam sags (exaggerated, so it can be seen), and close to
/// breaking it trembles and glows red.
pub fn draw_member(m: &Member, v: View, stress: bool, time: f32) {
    let load = m.stress.clamp(0.0, 1.2);
    let (r, g, bl) = stress_rgb(m.stress);
    let (cr, cg, cb) = m.mat.color();
    let base = Color::from_rgba(cr, cg, cb, 255);
    let col = if stress {
        Color::new(r, g, bl, 1.0)
    } else {
        // Past half its strength the beam takes on the effort colours.
        let k = ((load - 0.5) / 0.5).clamp(0.0, 1.0) * 0.85;
        Color::new(base.r + (r - base.r) * k, base.g + (g - base.g) * k, base.b + (bl - base.b) * k, 1.0)
    };
    // About to break: it shakes across its length.
    let danger = ((load - 0.75) / 0.25).clamp(0.0, 1.0);
    let (mut a, mut b) = (v.to_screen(m.p0), v.to_screen(m.p1));
    let d = (b - a).normalize_or_zero();
    let n = vec2(-d.y, d.x);
    let shake = n * (time * 61.0 + m.uid as f32).sin() * danger * 1.2 * v.s;
    a += shake;
    b += shake;
    let th = m.mat.thickness() * v.s;
    let dark = Color::new(col.r * 0.55, col.g * 0.55, col.b * 0.55, 1.0);
    // A floor beam bends down in the middle under the weight it carries.
    let down = if n.y >= 0.0 { n } else { -n };
    let sag = if m.is_deck() { load * load * (b - a).length() * 0.08 } else { 0.0 };
    let pts: Vec<Vec2> = (0..=8)
        .map(|k| {
            let t = k as f32 / 8.0;
            a + (b - a) * t + down * sag * 4.0 * t * (1.0 - t)
        })
        .collect();
    let line = |off: Vec2, w: f32, c: Color| {
        for s in pts.windows(2) {
            draw_line(s[0].x + off.x, s[0].y + off.y, s[1].x + off.x, s[1].y + off.y, w, c);
        }
    };
    if danger > 0.0 {
        let pulse = 0.5 + 0.5 * (time * 14.0).sin();
        line(Vec2::ZERO, th * 2.6, Color::new(1.0, 0.15, 0.1, 0.35 * danger * (0.5 + 0.5 * pulse)));
    }
    line(vec2(1.0, 2.0), th, Color::new(0.0, 0.0, 0.0, 0.25));
    line(Vec2::ZERO, th, dark);
    line(Vec2::ZERO, th * 0.68, col);
    line(n * (-th * 0.18), (th * 0.18).max(1.0), Color::new(1.0, 1.0, 1.0, 0.28));
    if m.is_deck() {
        // The floor a car drives on gets a bright running surface.
        line(n * (-th * 0.5), (th * 0.16).max(1.2), Color::new(1.0, 0.95, 0.6, 0.9));
    }
    let len = (b - a).length();
    match m.mat {
        MemberMat::Wood => {
            let mut t = 12.0;
            while t < len - 6.0 {
                let u = t / len;
                let p = a + d * t + down * sag * 4.0 * u * (1.0 - u);
                draw_line(p.x, p.y, p.x + d.x * 5.0, p.y + d.y * 5.0, 1.0, Color::new(0.35, 0.22, 0.1, 0.6));
                t += 14.0;
            }
        }
        MemberMat::Iron | MemberMat::Steel => {
            let mut t = 8.0;
            while t < len - 4.0 {
                let u = t / len;
                let p = a + d * t + down * sag * 4.0 * u * (1.0 - u);
                draw_circle(p.x, p.y, th * 0.12 + 0.6, Color::new(1.0, 1.0, 1.0, 0.35));
                t += 18.0;
            }
        }
    }
    for p in [a, b] {
        draw_circle(p.x, p.y, th * 0.78, Color::from_rgba(226, 228, 236, 255));
        draw_circle(p.x, p.y, th * 0.34, Color::from_rgba(60, 64, 78, 255));
    }
    if stress && load > 0.2 && len > 24.0 {
        // How much of its strength the beam uses, in the middle of it.
        let label = format!("{}%", (m.stress * 100.0).round() as i32);
        let c = pts[4] - down * (th + 8.0);
        let dim = measure_text(&label, None, 14, 1.0);
        draw_rectangle(c.x - dim.width / 2.0 - 3.0, c.y - dim.height / 2.0 - 3.0, dim.width + 6.0, dim.height + 6.0, Color::new(0.05, 0.06, 0.12, 0.75));
        draw_text(&label, c.x - dim.width / 2.0, c.y + dim.height / 2.0, 14.0, Color::new(r, g, bl, 1.0));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fire_lights_its_square_and_hot_air_makes_haze() {
        let mut w = World::new_static();
        w.climate_fixed = Some(15.0);
        w.reset_temps();
        for y in 300..304 {
            for x in 600..604 {
                w.set(x, y, Mat::Fire);
            }
        }
        // A pocket of hot air far from the fire.
        for y in 120..126 {
            for x in 120..126 {
                w.temp[y * W + x] = 200.0;
            }
        }
        let mut px = vec![[0u8; 4]; LW * LH];
        light_map(&w, &mut px);
        let at = |x: usize, y: usize| px[(y / LS) * LW + x / LS];
        assert!(at(601, 301)[0] > 150, "the fire glows: {:?}", at(601, 301));
        assert!(at(601, 301)[1] > 50, "the air above a flame shimmers");
        assert_eq!(at(300, 300)[0], 0, "no light away from the fire");
        assert!(at(122, 122)[1] > 200, "hot air makes haze: {:?}", at(122, 122));
        assert_eq!(at(122, 122)[0], 0, "hot air does not glow");
        assert_eq!(at(400, 200), [0, 0, 0, 255], "calm cool air: nothing");
    }
}
