//! Drawing: sky, world texture with a glow shader, blocks, cars, markers.

use crate::blocks::{Block, BlockKind, Member, MemberMat, Spring, SpringKind, STUD};
use crate::car::{Car, ModelKind};
use crate::world::sleep::{CH, CS, CW};
use crate::world::{Kind, Mat, World, H, W};
use crate::gfx::*;
use rayon::prelude::*;

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
        let s = (area.w / W() as f32).max(area.h / H() as f32);
        View { ox: area.x + (area.w - W() as f32 * s) / 2.0, oy: area.y + area.h - H() as f32 * s, s, area }
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
        Rect::new(self.ox, self.oy, W() as f32 * self.s, H() as f32 * self.s)
    }
}

// The texture is premultiplied RGBA. Bright orange cells (fire, lava, embers)
// bloom into their surroundings; every solid gets a light top edge.
// `Light` is the light map (see `light_map`): its green channel is the hot air, which bends the
// light passing through it, so everything seen through it wobbles (heat haze); its blue channel
// is the shadow of the clouds, which dims the sunlit world under them.
// `sky` is (sun x, sun strength, moon x, moon strength), x from 0 to 1 across the world.
const FRAG: &str = "
struct Params { texel: vec2<f32>, time: f32, sky: vec4<f32> }
@group(2) @binding(0) var<uniform> params: Params;
@group(2) @binding(1) var Light: texture_2d<f32>;
@group(2) @binding(2) var Light_sampler: sampler;
fn light(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(Light, Light_sampler, uv, 0.0);
}
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}
// Smooth value noise and two octaves of it: soft blobs with no grid in them.
fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), f.x), mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), f.x), f.y);
}
fn fbm(p: vec2<f32>) -> f32 {
    return 0.65 * vnoise(p) + 0.35 * vnoise(p * 2.3 + vec2<f32>(17.0, 9.0));
}
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let texel = params.texel;
    let time = params.time;
    let sky = params.sky;
    // Heat haze: hot air shifts what is seen through it. The light map is coarse (a texel per six
    // cells), so it is blurred into a soft mask first, a little more from below so the haze sits
    // over the source. The shift itself is turbulence: smooth noise that rises with the warm air
    // and is itself bent by slower noise (domain warping), so it swirls and wavers instead of
    // marching in straight rows.
    let lt = texel * 6.0;
    var heat = 0.3 * light(uv).g
        + 0.2 * light(uv + vec2<f32>(0.0, lt.y)).g
        + 0.15 * (light(uv + vec2<f32>(lt.x, 0.0)).g + light(uv - vec2<f32>(lt.x, 0.0)).g)
        + 0.1 * (light(uv - vec2<f32>(0.0, lt.y)).g + light(uv + vec2<f32>(0.0, 2.0 * lt.y)).g);
    heat = smoothstep(0.05, 0.9, heat);
    let cellf = uv / texel;
    var p = uv;
    var shim = 0.0;
    if (heat > 0.0) {
        let q = cellf * vec2<f32>(0.09, 0.07) + vec2<f32>(0.0, time * 1.4);
        let w = vec2<f32>(fbm(q * 0.45 + vec2<f32>(time * 0.35, 0.0)), fbm(q * 0.45 + vec2<f32>(5.2, 1.3 - time * 0.25)));
        let r = q + (w - 0.5) * 2.6;
        let d = vec2<f32>(fbm(r), fbm(r + vec2<f32>(8.3, 2.8))) - 0.5;
        p = uv + d * vec2<f32>(1.6, 0.9) * texel * heat;
        shim = fbm(r * 1.8 + vec2<f32>(3.1, 0.0)) - 0.5;
    }
    var c = tex(p);
    var glow = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var i: i32 = -3; i <= 3; i++) {
        for (var j: i32 = -3; j <= 3; j++) {
            let s = tex(p + vec2<f32>(f32(i), f32(j)) * texel * 1.6);
            let e = step(0.93, s.r) * step(s.b, 0.3) * step(0.5, s.a);
            let w = 1.0 / (1.0 + f32(i * i + j * j));
            glow += s.rgb * e * w;
            wsum += w;
        }
    }
    glow = glow / wsum;
    let up = tex(p - vec2<f32>(0.0, texel.y));
    let edge = c.a * (1.0 - up.a);
    // Water shimmers and embers, lava and flames flicker: a noise per cell that changes twenty
    // times a second, so the texture itself only changes where the cells do.
    let cell = floor(p / texel);
    let n = fract(sin(dot(cell, vec2<f32>(12.9898, 78.233)) + floor(time * 20.0) * 3.7) * 43758.5453) - 0.5;
    let water = step(0.78, c.a) * step(c.a, 0.83) * step(c.r, c.b);
    let hot = step(0.93, c.r) * step(c.b, 0.3) * step(0.5, c.a);
    c.g += n * (0.06 * water + 0.36 * hot) * c.a;
    var rgb = c.rgb + vec3<f32>(0.18) * edge * c.a + glow * 2.2;
    // Cloud shadow: by day, what lies under a cloud (seen from the sun) is in the shade.
    let shade = light(uv).b * sky.y;
    rgb *= 1.0 - 0.45 * shade;
    // Glints: the little waves of the top few rows of water catch the sun (or the moon) and flash,
    // most of all under the sun, the way a lake sparkles.
    let a1 = tex(p - vec2<f32>(0.0, texel.y)).a;
    let a2 = tex(p - vec2<f32>(0.0, texel.y * 3.0)).a;
    let a3 = tex(p - vec2<f32>(0.0, texel.y * 6.0)).a;
    let surf = water * max(max(step(a1, 0.05), 0.75 * step(a2, 0.05)), 0.4 * step(a3, 0.05));
    if (surf > 0.0) {
        let lit = sky.y * (1.0 - shade) * (0.35 + 0.65 * exp(-abs(uv.x - sky.x) * 4.0)) + 0.6 * sky.w * exp(-abs(uv.x - sky.z) * 6.0);
        // A glint is a short horizontal streak: a wave face a few cells wide catching the light.
        let facet = vec2<f32>(floor(cell.x / 3.0), cell.y);
        let t = time * 3.0 + hash(facet) * 9.0;
        let g = hash(facet + floor(t) * vec2<f32>(7.13, 3.71));
        let flash = smoothstep(0.86, 1.0, g) * (1.0 - fract(t));
        let wave = 0.5 + 0.5 * sin(cell.x * 0.5 + time * 3.0 + sin(cell.x * 0.13 - time));
        rgb += vec3<f32>(1.0, 0.97, 0.86) * surf * lit * (flash * 2.4 + wave * 0.16);
    }
    var a = max(c.a, clamp(length(glow) * 2.0, 0.0, 1.0));
    // Flames: tongues of fire dance above the burning cells. The light map's red is how much fire
    // each square holds; read a little below this point, through a column that sways with slow
    // noise, it fades smoothly upwards from the top of the fire. Turbulent noise rising through it
    // tears it into licks that flare and die, taller in some places than others. Yellow-white at
    // the foot, orange, then red at the tips.
    if (c.a < 0.5) {
        let sway = (fbm(vec2<f32>(cellf.y * 0.1 - time * 2.0, cellf.x * 0.05)) - 0.5) * 7.0 * texel.x;
        let b = uv + vec2<f32>(sway, lt.y * 1.1);
        let fire = 0.5 * light(b).r + 0.25 * (light(b + vec2<f32>(lt.x * 0.5, 0.0)).r + light(b - vec2<f32>(lt.x * 0.5, 0.0)).r);
        if (fire > 0.02) {
            let n = fbm(vec2<f32>(cellf.x * 0.3 + sway / texel.x * 0.1, cellf.y * 0.2 + time * 4.5));
            let tall = fbm(vec2<f32>(cellf.x * 0.12 - time * 0.4, time * 1.3));
            let v = fire * (0.9 + 0.9 * tall) - 0.35 + (n - 0.5) * 1.0;
            // Flames give off light: their colours run past white into the glow, and a soft
            // orange halo spills round each tongue.
            let fa = smoothstep(0.0, 0.2, v) * 0.95;
            var col = mix(vec3<f32>(1.0, 0.42, 0.08), vec3<f32>(1.0, 0.72, 0.18), smoothstep(0.05, 0.35, v));
            col = mix(col, vec3<f32>(1.0, 0.97, 0.7), smoothstep(0.4, 0.8, v)) * (1.15 + 0.5 * smoothstep(0.3, 0.9, v));
            let halo = smoothstep(-0.35, 0.05, v) * (1.0 - fa) * 0.45 * smoothstep(0.1, 0.5, fire);
            rgb = rgb * (1.0 - fa) + col * min(fa * 1.6, 1.0) + vec3<f32>(1.0, 0.55, 0.15) * halo;
            a = a + (fa + halo * 0.6) * (1.0 - a);
        }
    }
    // In clear air the haze shows itself as faint bright and dark swirls.
    let rip = shim * heat * 0.05 * (1.0 - a);
    return vec4<f32>(rgb + vec3<f32>(max(rip, 0.0)), a + max(-rip, 0.0));
}";

// At night a bluish veil covers the picture, except where fire, embers and lava light it: the light
// map's alpha (one texel per `LS` cells, already blurred, see `night_pool`) is a wide, flickering
// warm pool of light.
const NIGHT_FRAG: &str = "
struct Params { texel: vec2<f32>, time: f32, night: f32 }
@group(2) @binding(0) var<uniform> params: Params;
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    var l = tex(uv).a;
    // The flames dancing above a fire are lights themselves: no veil over them.
    let flames = max(tex(uv + vec2<f32>(0.0, params.texel.y)).r, tex(uv + vec2<f32>(0.0, 2.0 * params.texel.y)).r * 0.8);
    l = max(l, flames);
    l *= 0.88 + 0.08 * sin(params.time * 13.0 + uv.x * 40.0) + 0.06 * sin(params.time * 23.0 - uv.y * 30.0);
    l = clamp(l, 0.0, 1.0);
    let a = 0.46 * params.night * (1.0 - 0.9 * l);
    let warm = vec3<f32>(1.0, 0.56, 0.22) * l * 0.4 * params.night;
    return vec4<f32>(vec3<f32>(0.02, 0.05, 0.2) * a + warm, a);
}";

// Clouds and steam are drawn from a density layer, blurred once per change into a smooth field
// (`GAS_BLUR`, across then down, every cell read: a blur that skips cells cannot rub out a
// pattern that alternates from one cell to the next, and leaves a fine checkerboard), then
// thresholded here so the cells merge into soft, billowy blobs, lit from above.
// `dusk` is (twilight strength, x of the low sun from 0 to 1): at sunrise and sunset the sun shines
// from below the clouds, so their undersides and the edges facing it glow gold and rose while
// their tops fall into a lavender shade.
const GAS_FRAG: &str = "
struct Params { texel: vec2<f32>, dusk: vec2<f32> }
@group(2) @binding(0) var<uniform> params: Params;
fn dens(p: vec2<f32>) -> f32 {
    return tex(p).a;
}
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let texel = params.texel;
    let dusk = params.dusk;
    let d = dens(uv);
    let a = smoothstep(0.16, 0.52, d);
    if (a <= 0.003) {
        return vec4<f32>(0.0);
    }
    let up = dens(uv - vec2<f32>(0.0, texel.y * 6.0));
    let dn = dens(uv + vec2<f32>(0.0, texel.y * 6.0));
    let lightness = clamp(0.88 + (up - dn) * 0.9, 0.7, 1.08);
    var col = vec3<f32>(0.97, 0.98, 1.0) * lightness;
    if (dusk.x > 0.01) {
        let under = clamp((d - dn) * 4.0, 0.0, 1.0);
        let toward = select(-1.0, 1.0, uv.x < dusk.y);
        let side = dens(uv + vec2<f32>(toward * texel.x * 8.0, 0.0));
        let facing = clamp((d - side) * 2.5, 0.0, 1.0);
        let near = exp(-abs(uv.x - dusk.y) * 2.5);
        let warm = mix(vec3<f32>(0.98, 0.46, 0.5), vec3<f32>(1.0, 0.72, 0.36), near);
        col = mix(col, vec3<f32>(0.56, 0.5, 0.7) * lightness, dusk.x * 0.75);
        col = mix(col, warm * 1.05, clamp(0.35 * near + 1.1 * under + 0.7 * facing, 0.0, 0.9) * dusk.x);
    }
    let alpha = a * 0.86;
    return vec4<f32>(col * alpha, alpha);
}";

// One pass of the gas blur: a Gaussian along `dir` (one texel across, or one down), σ ≈ 3.7
// cells, the two passes together the same bell as the blur the clouds always had. The density
// is the alpha; the result is written as is (no blending) into the field.
const GAS_BLUR: &str = "
struct Params { dir: vec2<f32> }
@group(2) @binding(0) var<uniform> params: Params;
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var d = 0.0;
    var ws = 0.0;
    for (var i: i32 = -9; i <= 9; i++) {
        let w = exp(-f32(i * i) / 28.0);
        d += tex(in.uv + params.dir * f32(i)).a * w;
        ws += w;
    }
    return vec4<f32>(d / ws);
}";

// Drops of water: each is a quad along its flight, `uv` in units of the drop's radius: x from the
// middle of the round head back along the tail, y across. The vertex colour's red is the tail's
// length (in radii, over 16), its green the size of a pixel (in radii), its alpha the drop's
// opacity. A cartoon drop: a round head, a tail tapering to a point behind it, a darker rim, a
// glint.
const RAIN_FRAG: &str = "
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let px = in.color.g;
    let len = in.color.r * 16.0;
    let a = uv.x;
    let b = abs(uv.y);
    // Signed distance to the outline, in radii (negative inside).
    var dist: f32;
    if (a <= 0.0) {
        dist = length(uv) - 1.0;
    } else {
        let t = clamp(a / len, 0.0, 1.0);
        let hw = pow(1.0 - t, 1.4);
        dist = (b - hw) * 0.85;
    }
    let r = 1.0 / px;
    let cover = clamp(0.5 - dist * r, 0.0, 1.0);
    if (cover <= 0.0) {
        discard;
    }
    let body = vec3<f32>(0.42, 0.66, 0.98);
    let rim = vec3<f32>(0.16, 0.36, 0.78);
    var c = mix(body, rim, smoothstep(-0.38, -0.12, dist));
    // The glint, on the head, a little to one side.
    let g = 1.0 - smoothstep(0.12, 0.32, length(uv - vec2<f32>(-0.25, -0.35)));
    c = mix(c, vec3<f32>(0.95, 0.98, 1.0), g * 0.9);
    // The tail fades out toward its tip.
    let fade = select(1.0, 1.0 - 0.55 * clamp(a / len, 0.0, 1.0), a > 0.0);
    let alpha = cover * fade * in.color.a;
    return vec4<f32>(c * alpha, alpha);
}";

/// What the world picture shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Normal = 0,
    Thermal = 1,
    Pressure = 2,
    Humidity = 3,
    Electric = 4,
    Stress = 5,
}

// Lightning: each segment of a bolt is a quad whose texcoord y runs from -1 to 1 across it. A thin
// white-hot core, a violet-blue glow round it and a wide faint halo; the vertex colour's alpha is
// the brightness, and the wide halo quads (texcoord x = 1) light up the air and the clouds.
const BOLT_FRAG: &str = "
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let d = abs(in.uv.y);
    var c: vec3<f32>;
    if (in.uv.x > 0.5) {
        // Halo: the air and the clouds round the channel lit up.
        let h = exp(-d * d * 3.5) * (1.0 - d);
        c = vec3<f32>(0.45, 0.5, 0.95) * h * 0.5;
    } else {
        let core = exp(-d * d * 120.0);
        let glow = exp(-d * d * 9.0) * 0.6 + exp(-d * 3.5) * 0.3;
        c = vec3<f32>(1.0, 0.98, 1.0) * core * 1.6 + vec3<f32>(0.62, 0.55, 1.0) * glow;
    }
    c *= in.color.a * in.color.rgb;
    return vec4<f32>(c, 0.0);
}";

/// A bolt of lightning being shown: its shape and how long ago it struck (seconds).
#[derive(Clone)]
pub struct LiveBolt {
    pub bolt: crate::world::Bolt,
    pub age: f32,
    /// Per-bolt random number: when its return strokes come.
    pub seed: f32,
}

/// How long a bolt stays on the screen (seconds).
pub const BOLT_LIFE: f32 = 0.75;

impl LiveBolt {
    /// Brightness at its age: lightning flickers, two to four return strokes down the same
    /// channel, each a bright flash that fades in a few hundredths of a second.
    pub fn brightness(&self) -> f32 {
        let strokes = [0.0, 0.07 + 0.05 * self.seed, 0.19 + 0.1 * self.seed, 0.36 + 0.12 * self.seed];
        let n = 2 + (self.seed * 2.99) as usize;
        let mut b = 0.0f32;
        for (k, t0) in strokes.iter().take(n).enumerate() {
            let t = self.age - t0;
            if t >= 0.0 {
                b += (-t * 22.0).exp() * if k == 0 { 1.0 } else { 0.75 };
            }
        }
        // The fading channel glows a little longer.
        (b + 0.25 * (-self.age * 5.0).exp()).min(1.6) * (1.0 - (self.age / BOLT_LIFE).clamp(0.0, 1.0)).sqrt()
    }

    /// How much the whole scene is lit by it (the flash).
    pub fn flash(&self) -> f32 {
        let s = if self.bolt.ground { 1.0 } else { 0.55 };
        (self.brightness() * self.bolt.power.max(0.4) * s).min(1.0)
    }
}

pub struct Renderer {
    pub image: Image,
    pub texture: Texture2D,
    material: Material,
    gas_image: Image,
    gas_texture: Texture2D,
    gas_material: Material,
    /// The blur of the gas layer: across into the first target, then down into the second, the
    /// smooth density field `GAS_FRAG` reads.
    gas_blur: Material,
    gas_field: [RenderTarget; 2],
    /// The falling drops of rain, drawn as drops rather than cells: (x, y) the middle of the
    /// cell, (dx, dy) the way it falls, the length of its tail in cells, the size of its head
    /// (1 a cell's drop). Gathered once per change of the world.
    drops: Vec<[f32; 6]>,
    rev_rain: u64,
    rain_material: Material,
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
    bolt_material: Material,
}

/// Cells per texel of the light map.
pub const LS: usize = 6;
#[allow(non_snake_case)]
pub fn LW() -> usize {
    W() / LS
}
#[allow(non_snake_case)]
pub fn LH() -> usize {
    H() / LS
}

/// Fully cloudy, one square of the light map lets this fraction of the sunlight through, as the
/// simulation does (it takes 20 % per cell of cloud).
const CLOUD_PASS: f32 = 0.26;

/// The light map: for each `LS`×`LS` square, how many flames it holds (fire only, which the
/// shader turns into dancing tongues) in red, how hot its air is (for the heat haze) in green,
/// how much of the sunlight the clouds, steam and smoke between it and the sun hold back (their
/// shadow) in blue, and in alpha the night's pool of light from everything that glows (fire,
/// embers, lava). Flames, glow and heat saturate softly, so a big blaze is not blinding and a
/// single ember still shows.
pub fn light_map(world: &World, out: &mut [[u8; 4]]) {
    let shadow = cloud_shadow(world);
    let mut glows = vec![0u8; LW() * LH()];
    // (A row of squares per task, side by side.)
    out[..LW() * LH()].par_chunks_mut(LW()).zip(glows.par_chunks_mut(LW())).enumerate().for_each(|(ty, (out, glows))| {
        for tx in 0..LW() {
            let (mut glow, mut heat, mut fire) = (0.0f32, 0.0f32, 0.0f32);
            for y in ty * LS..(ty + 1) * LS {
                for x in tx * LS..(tx + 1) * LS {
                    let i = y * W() + x;
                    match world.cells[i].m {
                        Mat::Fire => {
                            glow += 1.0;
                            fire += 1.0;
                        }
                        Mat::Lava => glow += 1.3,
                        Mat::Ember => glow += 0.5,
                        Mat::Empty | Mat::Steam | Mat::Smoke => heat += ((world.temp[i] - 60.0) / 160.0).clamp(0.0, 1.0),
                        _ => {}
                    }
                }
            }
            let soft = |v: f32, k: f32| (255.0 * (1.0 - (-v / k).exp())) as u8;
            // Above a flame the air is hot too, even before the simulation has warmed it.
            out[tx] = [soft(fire, 4.0), soft(heat + glow * 0.5, 10.0), (shadow[ty * LW() + tx] * 255.0) as u8, 255];
            glows[tx] = soft(glow, 5.0);
        }
    });
    night_pool(&glows, out);
}

/// Spreads the glow into a wide, smooth pool of light in the alpha channel, for the night.
/// A gaussian blur done here, in two passes over the small light map: sampling a few far-apart
/// texels in the shader instead drew the pool as a grid of squares, like a waffle.
fn night_pool(glows: &[u8], out: &mut [[u8; 4]]) {
    const R: i32 = 9;
    let weights: Vec<f32> = (-R..=R).map(|d| (-(d * d) as f32 / 29.0).exp()).collect();
    let total: f32 = weights.iter().sum::<f32>().powi(2);
    let mut rows = vec![0.0f32; LW() * LH()];
    for y in 0..LH() {
        for x in 0..LW() {
            let mut v = 0.0;
            for (k, w) in weights.iter().enumerate() {
                let sx = (x as i32 + k as i32 - R).clamp(0, LW() as i32 - 1) as usize;
                v += glows[y * LW() + sx] as f32 * w;
            }
            rows[y * LW() + x] = v;
        }
    }
    for y in 0..LH() {
        for x in 0..LW() {
            let mut v = 0.0;
            for (k, w) in weights.iter().enumerate() {
                let sy = (y as i32 + k as i32 - R).clamp(0, LH() as i32 - 1) as usize;
                v += rows[sy * LW() + x] * w;
            }
            out[y * LW() + x][3] = (v / total * 4.5).min(255.0) as u8;
        }
    }
}

/// For each square of the light map, the share of the sunlight that clouds, steam and smoke stop
/// before it gets there (0: full sun, 1: deep shade). The light comes along the sun's rays, so a
/// cloud's shadow falls to the side of it in the morning and the evening. Worked out row by row
/// from the top, each square takes the shade of the square the ray came from, plus that square's
/// own cloud.
pub fn cloud_shadow(world: &World) -> Vec<f32> {
    let mut depth = vec![0.0f32; LW() * LH()];
    if world.sun() <= 0.0 {
        return depth;
    }
    // How much of each square is cloud (smoke is darker).
    let mut cloud = vec![0.0f32; LW() * LH()];
    for y in 0..H() {
        for x in 0..W() {
            let k = match world.cells[y * W() + x].m {
                Mat::Cloud | Mat::Steam => 1.0,
                Mat::Smoke => 2.0,
                _ => continue,
            };
            cloud[(y / LS) * LW() + x / LS] += k / (LS * LS) as f32;
        }
    }
    let (dx, dy) = world.sun_ray_dir();
    // Squares the ray moves sideways per row (a very low sun is taken as a slanted one).
    let slope = (dx / dy.max(0.05)).clamp(-3.0, 3.0);
    let k = -CLOUD_PASS.ln();
    for ty in 1..LH() {
        for tx in 0..LW() {
            let from = tx as f32 - slope;
            let (i0, f) = (from.floor(), from - from.floor());
            let at = |i: f32| {
                let i = i as i32;
                if i < 0 || i >= LW() as i32 { 0.0 } else {
                    let j = (ty - 1) * LW() + i as usize;
                    depth[j] + cloud[j] * k
                }
            };
            depth[ty * LW() + tx] = at(i0) * (1.0 - f) + at(i0 + 1.0) * f;
        }
    }
    depth.iter().map(|d| 1.0 - (-d).exp()).collect()
}

impl Renderer {
    pub fn new() -> Self {
        let image = Image::gen_image_color(W() as u16, H() as u16, Color::new(0.0, 0.0, 0.0, 0.0));
        let texture = Texture2D::from_image(&image);
        texture.set_filter(FilterMode::Linear);
        let blend = Blend::Premultiplied;
        let material = load_material(
            FRAG,
            MaterialParams {
                blend,
                uniforms: vec![UniformDesc::new("texel", UniformType::Float2), UniformDesc::new("time", UniformType::Float1), UniformDesc::new("sky", UniformType::Float4)],
                textures: vec!["Light".to_string()],
                ..Default::default()
            },
        )
        .expect("shader");
        let gas_image = Image::gen_image_color(W() as u16, H() as u16, Color::new(0.0, 0.0, 0.0, 0.0));
        let gas_texture = Texture2D::from_image(&gas_image);
        gas_texture.set_filter(FilterMode::Linear);
        let gas_material = load_material(
            GAS_FRAG,
            MaterialParams {
                blend,
                uniforms: vec![UniformDesc::new("texel", UniformType::Float2), UniformDesc::new("dusk", UniformType::Float2)],
                ..Default::default()
            },
        )
        .expect("gas shader");
        let gas_blur = load_material(
            GAS_BLUR,
            MaterialParams { uniforms: vec![UniformDesc::new("dir", UniformType::Float2)], blend: Blend::Replace, ..Default::default() },
        )
        .expect("gas blur shader");
        let gas_field = [(); 2].map(|_| {
            let t = render_target(W() as u32, H() as u32);
            t.texture.set_filter(FilterMode::Linear);
            t
        });
        let rain_material = load_material(RAIN_FRAG, MaterialParams { blend, ..Default::default() })
        .expect("rain shader");
        let night_material = load_material(
            NIGHT_FRAG,
            MaterialParams {
                blend,
                uniforms: vec![UniformDesc::new("texel", UniformType::Float2), UniformDesc::new("time", UniformType::Float1), UniformDesc::new("night", UniformType::Float1)],
                ..Default::default()
            },
        )
        .expect("night shader");
        let light_image = Image::gen_image_color(LW() as u16, LH() as u16, Color::new(0.0, 0.0, 0.0, 1.0));
        let light_texture = Texture2D::from_image(&light_image);
        light_texture.set_filter(FilterMode::Linear);
        // Lightning adds its light to what is under it.
        let bolt_material = load_material(BOLT_FRAG, MaterialParams { blend: Blend::Additive, ..Default::default() })
        .expect("bolt shader");
        let veg_image = Image::gen_image_color(W() as u16, H() as u16, Color::new(0.0, 0.0, 0.0, 0.0));
        let veg_texture = Texture2D::from_image(&veg_image);
        veg_texture.set_filter(FilterMode::Linear);
        Renderer {
            veg_image,
            veg_texture,
            veg_zones: vec![vec![]; CW() * CH()],
            veg_tick: -1,
            drawn_stamp: 0,
            gas_stamp: 0,
            gas_zones: vec![0; CW() * CH()],
            band: 0,
            full: true,
            image,
            texture,
            material,
            gas_image,
            gas_texture,
            gas_material,
            gas_blur,
            gas_field,
            drops: vec![],
            rev_rain: u64::MAX,
            rain_material,
            rev_world: u64::MAX,
            rev_gas: u64::MAX,
            last_mode: 0,
            pressure: vec![],
            has_gas: false,
            light_image,
            light_texture,
            night_material,
            light_tick: -1,
            bolt_material,
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
                *out = color(idx % W(), idx / W());
                visible += (out[3] != 0) as usize;
            }
            visible
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).min(8);
            let rows = H().div_ceil(threads);
            let color = &color;
            std::thread::scope(|scope| {
                let handles: Vec<_> = px
                    .chunks_mut(rows * W())
                    .enumerate()
                    .map(|(i, chunk)| {
                        scope.spawn(move || {
                            let mut visible = 0;
                            for (j, out) in chunk.iter_mut().enumerate() {
                                let idx = i * rows * W() + j;
                                *out = color(idx % W(), idx / W());
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
        let (x0, y0) = ((k % CW()) * CS, (k / CW()) * CS);
        let mut visible = 0;
        for y in y0..(y0 + CS).min(H()) {
            for x in x0..(x0 + CS).min(W()) {
                let c = color(x, y);
                visible += (c[3] != 0) as u32;
                px[y * W() + x] = c;
            }
        }
        visible
    }

    /// The zones whose cells may have changed since step count `since`.
    fn changed_zones(world: &World, since: u64) -> Vec<usize> {
        (0..CW() * CH()).filter(|&k| world.zones.changed_since(k, since)).collect()
    }

    /// The tree cells of zone `k`: (x, y, height above the ground, colour).
    fn veg_of_zone(world: &World, k: usize) -> Vec<(u16, u16, f32, [u8; 4])> {
        let (x0, y0) = ((k % CW()) * CS, (k / CW()) * CS);
        let mut out = vec![];
        for y in y0..(y0 + CS).min(H()) {
            for x in x0..(x0 + CS).min(W()) {
                if world.cell_mat(x, y).is_vegetation() {
                    let mut h = 0usize;
                    while h < 90 && y + h + 1 < H() && {
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
        let size = DrawTextureParams { dest_size: Some(vec2(W() as f32 * v.s, H() as f32 * v.s)), ..Default::default() };
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
                RenderMode::Electric => {
                    Self::fill(&mut self.image, |x, y| world.electric_color(x, y));
                }
                RenderMode::Stress => {
                    Self::fill(&mut self.image, |x, y| stress_color(world, x, y));
                }
                RenderMode::Normal => {
                    // Only the zones that changed, plus one row of zones in turn for the slow
                    // changes (temperature tints) that do not wake a zone.
                    // A fresh world (a new game, a loaded one) counts its steps from the start again.
                    if self.full || self.last_mode != id || world.zones.stamp() < self.drawn_stamp {
                        Self::fill(&mut self.image, |x, y| landscape_color(world, x, y));
                        for k in 0..CW() * CH() {
                            self.veg_zones[k] = Self::veg_of_zone(world, k);
                        }
                        self.full = false;
                    } else {
                        let mut zones = Self::changed_zones(world, self.drawn_stamp);
                        self.band = (self.band + 1) % CH();
                        zones.extend((0..CW()).map(|i| self.band * CW() + i));
                        for k in zones {
                            Self::fill_zone(&mut self.image, k, |x, y| landscape_color(world, x, y));
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
        // The sun and the moon follow the same arcs as in the backdrop. The moon lights the
        // world at night only, and only as much as its face is lit (nothing at new moon).
        let phase = world.phase();
        let at = |u: f32| (0.08 + 0.84 * u, (std::f32::consts::PI * u).sin().sqrt());
        let sky = match (phase < 0.5, world.moon_arc()) {
            (true, _) => {
                let (x, h) = at(phase / 0.5);
                vec4(x, h, 0.0, 0.0)
            }
            (false, Some(u)) => {
                let (x, h) = at(u);
                vec4(0.0, 0.0, x, h * world.moon_light())
            }
            (false, None) => vec4(0.0, 0.0, 0.0, 0.0),
        };
        self.material.set_uniform("texel", vec2(1.0 / W() as f32, 1.0 / H() as f32));
        self.material.set_uniform("time", time);
        self.material.set_uniform("sky", sky);
        self.material.set_texture("Light", self.light_texture.clone());
        gl_use_material(&self.material);
        draw_texture_ex(&self.texture, v.ox, v.oy, WHITE, size.clone());
        draw_texture_ex(&self.veg_texture, v.ox, v.oy, WHITE, size);
        gl_use_default_material();
        self.draw_rain(world, v);
    }

    /// The falling raindrops and the drops of water in flight (spray, splashes), stretched along
    /// their flight like drops in a cartoon: a round head and a tail that tapers behind it,
    /// longer the faster it goes (`landscape_color` leaves the raindrops' cells out of the
    /// landscape).
    fn draw_rain(&mut self, world: &World, v: View) {
        if self.rev_rain != world.revision {
            self.drops.clear();
            for y in 1..H() - 1 {
                for x in 0..W() {
                    if let Some(n) = rain_drop(world, x, y) {
                        // Its fall (eighths of a cell a step), and the wind that slants it.
                        let fall = world.cell_fall(x, y) as f32 / 8.0;
                        let (wx, _) = world.flow.wind_at(x as i32, y as i32);
                        let (dx, dy) = ((wx * 0.6).clamp(-0.7, 0.7), 1.0);
                        let norm = (dx * dx + dy * dy).sqrt();
                        let len = (1.6 + 1.2 * fall).min(4.5) + (n - 1) as f32;
                        self.drops.push([x as f32 + 0.5, y as f32 + 0.5, dx / norm, dy / norm, len, 1.0]);
                    }
                }
            }
            self.rev_rain = world.revision;
        }
        // The drops in flight move every frame: the more water, the bigger.
        let flying = world.liquid_drops().iter().filter(|d| d.m == Mat::Water).map(|d| {
            let (vx, vy) = d.velocity();
            let speed = (vx * vx + vy * vy).sqrt();
            let (dx, dy) = if speed > 1e-3 { (vx / speed, vy / speed) } else { (0.0, 1.0) };
            let size = 0.35 + 0.65 * (d.q as f32 / 65536.0).min(1.0).sqrt();
            [d.x, d.y, dx, dy, (0.8 + 0.3 * speed).min(4.5) * size, size]
        });
        let drops: Vec<[f32; 6]> = self.drops.iter().copied().chain(flying).collect();
        if drops.is_empty() {
            return;
        }
        gl_use_material(&self.rain_material);
        let mut verts: Vec<Vertex> = vec![];
        let mut idx: Vec<u16> = vec![];
        for [x, y, dx, dy, len, size] in drops {
            if idx.len() + 6 > BOLT_MESH_INDICES {
                draw_mesh(&Mesh { vertices: std::mem::take(&mut verts), indices: std::mem::take(&mut idx) });
            }
            // The head's radius on screen: a little more than half a cell for a cell's worth of
            // water, never under a pixel.
            let r = (0.62 * v.s * size).max(1.0);
            let head = vec2(v.ox + x * v.s, v.oy + y * v.s);
            let back = -vec2(dx, dy);
            let side = vec2(-back.y, back.x);
            // In radii: the tail's length, and a margin of a pixel round the drop for its soft edge.
            let tail = (len * v.s / r).max(1.5);
            let m = 1.0 / r;
            let col = Color::new((tail / 16.0).min(1.0), m, 0.0, 0.92);
            let k = verts.len() as u16;
            for (a, b) in [(-1.0 - m, -1.0 - m), (tail + m, -1.0 - m), (tail + m, 1.0 + m), (-1.0 - m, 1.0 + m)] {
                let p = head + (back * a + side * b) * r;
                verts.push(Vertex::new(p.x, p.y, 0.0, a, b, col));
            }
            idx.extend_from_slice(&[k, k + 1, k + 2, k, k + 2, k + 3]);
        }
        draw_mesh(&Mesh { vertices: verts, indices: idx });
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
        self.night_material.set_uniform("texel", vec2(1.0 / LW() as f32, 1.0 / LH() as f32));
        self.night_material.set_uniform("time", time);
        self.night_material.set_uniform("night", night);
        gl_use_material(&self.night_material);
        let r = v.rect();
        draw_texture_ex(&self.light_texture, r.x, r.y, WHITE, DrawTextureParams { dest_size: Some(vec2(r.w, r.h)), ..Default::default() });
        gl_use_default_material();
    }

    /// Bolts of lightning and the flash that lights the whole picture.
    /// Saint Elmo's fire: where the ground or what stands on it is close to breaking down under a
    /// charged cloud, faint violet sparks crackle on its top.
    pub fn draw_corona(&self, world: &World, v: View, time: f32) {
        use crate::world::{GROUND_BREAKDOWN, QH, QS, QW};
        for qy in 1..QH() {
            for qx in 0..QW() {
                let q = world.mcharge[qy * QW() + qx].abs() / GROUND_BREAKDOWN;
                if q < 0.45 {
                    continue;
                }
                let x = (qx * QS + QS / 2) as i32;
                // The top of the matter in this square, with air just above it.
                let Some(y) = ((qy * QS) as i32..((qy + 1) * QS) as i32)
                    .find(|&y| !matches!(world.raw(x, y).kind(), crate::world::Kind::Empty | crate::world::Kind::Gas | crate::world::Kind::Flame))
                    .filter(|&y| matches!(world.raw(x, y - 1).kind(), crate::world::Kind::Empty | crate::world::Kind::Gas))
                else {
                    continue;
                };
                let a = ((q - 0.45) / 0.55).clamp(0.0, 1.0);
                let p = vec2(v.ox + (x as f32 + 0.5) * v.s, v.oy + y as f32 * v.s);
                draw_circle(p.x, p.y, 5.0 * v.s.max(0.6), Color::new(0.6, 0.5, 1.0, 0.12 * a));
                for k in 0..3 {
                    let t = time * 17.0 + k as f32 * 2.1 + qx as f32 * 1.7;
                    if (t.sin() * 43.7).fract().abs() < 0.4 {
                        continue;
                    }
                    let ang = -1.57 + (t * 1.3).sin() * 0.9;
                    let len = (2.0 + 3.0 * a) * v.s.max(0.6);
                    let q2 = p + vec2(ang.cos(), ang.sin()) * len;
                    draw_line(p.x, p.y, q2.x, q2.y, 1.0, Color::new(0.75, 0.7, 1.0, 0.7 * a));
                }
            }
        }
    }

    pub fn draw_bolts(&self, bolts: &[LiveBolt], v: View) {
        if bolts.is_empty() {
            return;
        }
        let flash = bolts.iter().map(|b| b.flash()).fold(0.0f32, f32::max);
        if flash > 0.01 {
            let r = v.rect();
            draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.8, 0.84, 1.0, 0.22 * flash));
        }
        gl_use_material(&self.bolt_material);
        for b in bolts {
            let light = b.brightness() * (0.55 + 0.45 * b.bolt.power);
            if light < 0.01 {
                continue;
            }
            let mut verts: Vec<Vertex> = vec![];
            let mut idx: Vec<u16> = vec![];
            let px = |p: (f32, f32)| vec2(v.ox + p.0 * v.s, v.oy + p.1 * v.s);
            let s = v.s.max(0.6);
            let mut line = |pts: &[(f32, f32)], w: f32, a: f32, halo: bool| {
                for seg in pts.windows(2) {
                    // A mesh numbers its vertices on 16 bits: a long bolt zigzagging through
                    // rock goes out in several meshes.
                    if idx.len() + 6 > BOLT_MESH_INDICES {
                        draw_mesh(&Mesh { vertices: std::mem::take(&mut verts), indices: std::mem::take(&mut idx) });
                    }
                    let (p, q) = (px(seg[0]), px(seg[1]));
                    let d = q - p;
                    let len = d.length().max(1e-3);
                    let dir = d / len;
                    let n = vec2(-dir.y, dir.x) * w;
                    // Stretch each piece a little so the joints have no gaps.
                    let (p, q) = (p - dir * w * 0.3, q + dir * w * 0.3);
                    let col = Color::new(1.0, 1.0, 1.0, a.clamp(0.0, 1.0));
                    let u = if halo { 1.0 } else { 0.0 };
                    let k = verts.len() as u16;
                    verts.push(Vertex::new(p.x + n.x, p.y + n.y, 0.0, u, -1.0, col));
                    verts.push(Vertex::new(p.x - n.x, p.y - n.y, 0.0, u, 1.0, col));
                    verts.push(Vertex::new(q.x - n.x, q.y - n.y, 0.0, u, 1.0, col));
                    verts.push(Vertex::new(q.x + n.x, q.y + n.y, 0.0, u, -1.0, col));
                    idx.extend_from_slice(&[k, k + 1, k + 2, k, k + 2, k + 3]);
                }
            };
            line(&b.bolt.path, 34.0 * s, light * 0.8, true);
            line(&b.bolt.path, (2.6 + 2.4 * b.bolt.power) * s, light, false);
            for br in &b.bolt.branches {
                line(br, 1.8 * s, light * 0.6, false);
            }
            draw_mesh(&Mesh { vertices: verts, indices: idx });
        }
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
            let nx = (x as i32 + dx.round() as i32).clamp(0, W() as i32 - 1) as usize;
            px[y as usize * W() + nx] = col;
        }
        self.veg_texture.update(&self.veg_image);
    }

    /// Clouds and steam as soft blobs, drawn over the landscape.
    pub fn draw_gas(&mut self, world: &World, v: View) {
        if self.rev_gas != world.revision {
            // The gas picture depends only on the cells: recolour the zones that changed.
            let zones = if self.rev_gas == u64::MAX || world.zones.stamp() < self.gas_stamp { (0..CW() * CH()).collect() } else { Self::changed_zones(world, self.gas_stamp) };
            let was = self.has_gas;
            for k in zones {
                self.gas_zones[k] = Self::fill_zone(&mut self.gas_image, k, |x, y| world.gas_color(x, y));
            }
            self.has_gas = self.gas_zones.iter().any(|&n| n > 0);
            if self.has_gas || was {
                self.gas_texture.update(&self.gas_image);
                self.blur_gas();
            }
            self.rev_gas = world.revision;
            self.gas_stamp = world.zones.stamp();
        }
        // No clouds, steam or smoke: skip the expensive blur pass altogether.
        if !self.has_gas {
            return;
        }
        let (dusk, sun_x) = crate::backdrop::twilight(world.phase());
        self.gas_material.set_uniform("texel", vec2(1.0 / W() as f32, 1.0 / H() as f32));
        self.gas_material.set_uniform("dusk", vec2(dusk, sun_x));
        gl_use_material(&self.gas_material);
        draw_texture_ex(
            &self.gas_field[1].texture,
            v.ox,
            v.oy,
            WHITE,
            DrawTextureParams { dest_size: Some(vec2(W() as f32 * v.s, H() as f32 * v.s)), ..Default::default() },
        );
        gl_use_default_material();
    }
}

impl Renderer {
    /// Blurs the gas layer into the smooth density field: across, then down, off screen.
    fn blur_gas(&self) {
        let (w, h) = (W() as f32, H() as f32);
        let pass = |src: &Texture2D, dst: &RenderTarget, dir: Vec2| {
            set_target(Some(dst));
            clear_background(Color::new(0.0, 0.0, 0.0, 0.0));
            self.gas_blur.set_uniform("dir", dir);
            gl_use_material(&self.gas_blur);
            draw_texture_ex(src, 0.0, 0.0, WHITE, DrawTextureParams { dest_size: Some(vec2(w, h)), ..Default::default() });
            gl_use_default_material();
        };
        pass(&self.gas_texture, &self.gas_field[0], vec2(1.0 / w, 0.0));
        pass(&self.gas_field[0].texture, &self.gas_field[1], vec2(0.0, 1.0 / h));
        set_target(None);
    }
}

/// The most cells of water one above the other that still fall as one drop.
const DROP_CELLS: usize = 3;

/// A drop of water falling through the air, whose lowest cell is (x, y): a short column of water
/// (`DROP_CELLS` at most) with air (or gas) under it, and no block, and no liquid above it. Its number of
/// cells; drawn as a drop (`Renderer::draw_rain`) rather than as cells.
fn rain_drop(world: &World, x: usize, y: usize) -> Option<usize> {
    let i = y * W() + x;
    if world.cells[i].m != Mat::Water || y + 1 >= H() || !matches!(world.cells[i + W()].m.kind(), Kind::Empty | Kind::Gas) || !world.mask_free(x as i32, y as i32 + 1) {
        return None;
    }
    let n = (0..=DROP_CELLS).take_while(|&k| k <= y && world.cells[i - k * W()].m == Mat::Water).count();
    (n <= DROP_CELLS && y >= n && world.cells[i - n * W()].m.kind() != Kind::Liquid).then_some(n)
}

/// Whether cell (x, y) is part of a falling drop.
fn in_drop(world: &World, x: usize, y: usize) -> bool {
    (y..(y + DROP_CELLS).min(H())).take_while(|&yy| world.cells[yy * W() + x].m == Mat::Water).any(|yy| rain_drop(world, x, yy).is_some())
}

/// The landscape's colour of cell (x, y): the world's, but none for a falling raindrop.
fn landscape_color(world: &World, x: usize, y: usize) -> [u8; 4] {
    if in_drop(world, x, y) {
        [0, 0, 0, 0]
    } else {
        world.color_without_trees(x, y)
    }
}

/// The most indices one mesh of quads holds: its vertices (four a quad, six indices) must be
/// numbered on 16 bits.
const BOLT_MESH_INDICES: usize = 6 * 16_000;

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

/// The stress view of a cell: solids coloured by how loaded their section is (see
/// `world/statics.rs`), green when it carries nothing to red when it gives way; the rest dimmed.
fn stress_color(world: &World, x: usize, y: usize) -> [u8; 4] {
    let c = world.color(x, y);
    if c[3] == 0 {
        return c;
    }
    let m = world.cell_mat(x, y);
    // A liquid that runs by its physics shows its pressure: pale near the surface, deep violet
    // 10 m down (a full atmosphere more).
    if let Some(pa) = world.liquid_pa(x, y) {
        let t = (pa / 101_325.0).clamp(0.0, 1.0).sqrt();
        let lerp = |a: f32, b: f32| (a + (b - a) * t) as u8;
        return [lerp(150.0, 70.0), lerp(200.0, 20.0), lerp(235.0, 130.0), 230];
    }
    if m.kind() != Kind::Static || matches!(m, Mat::Dirt | Mat::Grass) {
        return [c[0] / 3, c[1] / 3, c[2] / 3, c[3] / 2];
    }
    let (r, g, b) = stress_rgb(world.strain[y * W() + x] as f32 / 127.5);
    let a = c[3] as f32 / 255.0;
    let mix = |k: f32, base: u8| ((k * 255.0 * 0.75 + base as f32 * 0.25) * a) as u8;
    [mix(r, c[0]), mix(g, c[1]), mix(b, c[2]), c[3]]
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

/// Faint dashed lines where a block being placed lines up with its neighbours.
pub fn draw_guides(guides: &[(Vec2, Vec2)], v: View) {
    let col = Color::new(0.55, 0.85, 1.0, 0.55);
    for (a, b) in guides {
        let (sa, sb) = (v.to_screen(*a), v.to_screen(*b));
        let len = (sb - sa).length();
        let dir = (sb - sa) / len.max(1e-3);
        let mut d = 0.0;
        while d < len {
            let (p, q) = (sa + dir * d, sa + dir * (d + 5.0).min(len));
            draw_line(p.x, p.y, q.x, q.y, 1.0, col);
            d += 9.0;
        }
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

#[derive(Clone)]
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
    let th = m.thickness() * v.s;
    // A deck's slab hangs under the line between its pins (its top is the road).
    let (pa, pb) = (a, b);
    let slab = m.slab_offset() * v.s;
    let (a, b) = (a + slab, b + slab);
    let dark = Color::new(col.r * 0.55, col.g * 0.55, col.b * 0.55, 1.0);
    // A floor beam bends down in the middle under the weight it carries.
    let down = if n.y >= 0.0 { n } else { -n };
    let sag = if m.is_deck() { load * load * (b - a).length() * 0.08 } else { 0.0 };
    let along = |t: f32| a + (b - a) * t + down * sag * 4.0 * t * (1.0 - t);
    let pts: Vec<Vec2> = (0..=8).map(|k| along(k as f32 / 8.0)).collect();
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
    let len = (b - a).length();
    if m.is_deck() {
        // The deck: a thick slab, two chords with cross bracing between them, and the bright
        // running surface the car drives on along its top.
        line(Vec2::ZERO, th, dark);
        line(Vec2::ZERO, th * 0.78, Color::new(col.r * 0.8, col.g * 0.8, col.b * 0.8, 1.0));
        let chord = (th * 0.2).max(1.2);
        let edge = th * 0.5 - chord * 0.5;
        line(-down * edge, chord, col);
        line(down * edge, chord, col);
        let brace = Color::new(col.r * 0.4, col.g * 0.4, col.b * 0.4, 1.0);
        let panels = ((len / (th * 1.6)).round() as i32).max(1);
        for k in 0..panels {
            let (t0, t1) = (k as f32 / panels as f32, (k + 1) as f32 / panels as f32);
            let (p0, p1) = (along(t0), along(t1));
            let h = th * 0.36;
            let w = (th * 0.09).max(1.0);
            draw_line(p0.x - down.x * h, p0.y - down.y * h, p1.x + down.x * h, p1.y + down.y * h, w, brace);
            draw_line(p0.x + down.x * h, p0.y + down.y * h, p1.x - down.x * h, p1.y - down.y * h, w, brace);
            if k > 0 {
                draw_line(p0.x - down.x * h, p0.y - down.y * h, p0.x + down.x * h, p0.y + down.y * h, w, brace);
            }
        }
        line(-down * (th * 0.5), (th * 0.14).max(1.2), Color::new(1.0, 0.95, 0.6, 0.9));
    } else {
        line(Vec2::ZERO, th, dark);
        line(Vec2::ZERO, th * 0.68, col);
        line(n * (-th * 0.18), (th * 0.18).max(1.0), Color::new(1.0, 1.0, 1.0, 0.28));
        match m.mat {
            MemberMat::Wood => {
                let mut t = 12.0;
                while t < len - 6.0 {
                    let p = along(t / len);
                    draw_line(p.x, p.y, p.x + d.x * 5.0, p.y + d.y * 5.0, 1.0, Color::new(0.35, 0.22, 0.1, 0.6));
                    t += 14.0;
                }
            }
            MemberMat::Iron | MemberMat::Steel => {
                let mut t = 8.0;
                while t < len - 4.0 {
                    let p = along(t / len);
                    draw_circle(p.x, p.y, th * 0.12 + 0.6, Color::new(1.0, 1.0, 1.0, 0.35));
                    t += 18.0;
                }
            }
        }
    }
    // The pins are the same size on a deck and on a girder.
    let pin = m.mat.thickness() * v.s;
    for p in [pa, pb] {
        draw_circle(p.x, p.y, pin * 0.78, Color::from_rgba(226, 228, 236, 255));
        draw_circle(p.x, p.y, pin * 0.34, Color::from_rgba(60, 64, 78, 255));
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
    fn a_cloud_casts_a_shadow_by_day_along_the_sun_rays_and_none_at_night() {
        let mut w = World::new_static();
        w.day_cycle = false;
        for y in 120..150 {
            for x in 420..540 {
                w.set(x, y, Mat::Cloud);
            }
        }
        let at = |sh: &[f32], x: usize, y: usize| sh[(y / LS) * LW() + x / LS];
        // Noon: the sun is high, the shadow falls straight down.
        w.set_phase(0.25);
        let sh = cloud_shadow(&w);
        assert!(at(&sh, 480, 300) > 0.9, "deep shade under a thick cloud: {}", at(&sh, 480, 300));
        assert!(at(&sh, 480, 100) < 0.01, "above the cloud the sun shines");
        assert!(at(&sh, 150, 300) < 0.01, "far beside it, full sun");
        // Morning: the sun is low in the east (left), the shadow falls to the right of the cloud.
        w.set_phase(0.06);
        let sh = cloud_shadow(&w);
        let (left, right) = (at(&sh, 360, 300), at(&sh, 640, 300));
        assert!(right > left + 0.5, "in the morning the shadow lies to the right: left {left}, right {right}");
        // Night: no sun, no shadow.
        w.set_phase(0.75);
        assert!(cloud_shadow(&w).iter().all(|&v| v == 0.0));
    }

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
                w.temp[y * W() + x] = 200.0;
            }
        }
        let mut px = vec![[0u8; 4]; LW() * LH()];
        light_map(&w, &mut px);
        let at = |x: usize, y: usize| px[(y / LS) * LW() + x / LS];
        assert!(at(601, 301)[0] > 150, "the fire glows: {:?}", at(601, 301));
        assert!(at(601, 301)[1] > 50, "the air above a flame shimmers");
        assert_eq!(at(300, 300)[0], 0, "no light away from the fire");
        assert!(at(122, 122)[1] > 200, "hot air makes haze: {:?}", at(122, 122));
        assert_eq!(at(122, 122)[0], 0, "hot air does not glow");
        assert_eq!(at(400, 200), [0, 0, 0, 0], "calm cool air: nothing");
        // At night the fire lights a smooth pool round it: brightest on the fire, then fading
        // steadily outwards, with no gaps (a waffle) on the way.
        let pool: Vec<u8> = (0..10).map(|k| at(601 + k * LS, 301)[3]).collect();
        assert!(pool[0] > 0, "the fire lights the night: {pool:?}");
        assert!(pool.windows(2).all(|p| p[1] <= p[0]), "the pool fades steadily: {pool:?}");
        assert!(pool[6] > 0, "and reaches a few metres away: {pool:?}");
    }

    #[test]
    fn only_fire_throws_flames_but_lava_still_lights_the_night() {
        let mut w = World::new_static();
        w.climate_fixed = Some(15.0);
        w.reset_temps();
        for y in 300..306 {
            for x in 300..306 {
                w.set(x, y, Mat::Lava);
            }
        }
        let mut px = vec![[0u8; 4]; LW() * LH()];
        light_map(&w, &mut px);
        let lava = px[(300 / LS) * LW() + 300 / LS];
        assert_eq!(lava[0], 0, "no flames on lava: {lava:?}");
        assert!(lava[3] > 0, "but it lights the night: {lava:?}");
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use std::time::Instant;

    /// What the picture costs the processor on the opening world: the light map (15 times a
    /// second), a full recolouring (a new world, a view change), the tree cells (12 times a second).
    /// `cargo test --release --bin aworld bench_render -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_render() {
        let mut w = World::new();
        w.generate_biome(3, crate::world::Biome::Countryside);
        for _ in 0..60 {
            w.step();
        }
        let time = |name: &str, f: &mut dyn FnMut()| {
            let t = Instant::now();
            for _ in 0..20 {
                f();
            }
            eprintln!("RENDER {name}: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
        };
        let mut light = vec![[0u8; 4]; LW() * LH()];
        time("carte de lumière", &mut || light_map(&w, &mut light));
        time("  dont ombre des nuages", &mut || {
            let _ = cloud_shadow(&w);
        });
        let glows = vec![9u8; LW() * LH()];
        time("  dont halo de nuit", &mut || night_pool(&glows, &mut light));
        let mut px = vec![[0u8; 4]; W() * H()];
        time("recoloriage complet (normal)", &mut || {
            for (i, p) in px.iter_mut().enumerate() {
                *p = w.color_without_trees(i % W(), i / W());
            }
        });
        time("recoloriage complet (thermique)", &mut || {
            for (i, p) in px.iter_mut().enumerate() {
                *p = w.thermal_color(i % W(), i / W());
            }
        });
        time("cellules des arbres (toutes les zones)", &mut || {
            for k in 0..CW() * CH() {
                let _ = Renderer::veg_of_zone(&w, k);
            }
        });
    }
}
