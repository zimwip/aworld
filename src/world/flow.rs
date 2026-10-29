//! A simplified flow of the air ("stable fluids" on a coarse grid): winds in layers that blow
//! opposite ways, gusts that curl into vortices, warm air rising, and a flow that goes around
//! solid ground. Clouds, steam and smoke are carried along; the cells themselves stay as they are.
//!
//! Velocities are kept in coarse cells per step (a coarse cell is `CELL()` world cells wide).

use super::num::{floor_f, wrap_i};
use super::{Cell, Kind, World, H, W};
use rayon::prelude::*;

/// Width of a flow cell, in world cells: about 40 cm, 8 cells on the design grid (see
/// `units::flow_cell`).
#[allow(non_snake_case)]
#[inline(always)]
pub fn CELL() -> usize {
    super::units::flow_cell()
}
#[allow(non_snake_case)]
pub fn FW() -> usize {
    W() / CELL()
}
#[allow(non_snake_case)]
pub fn FH() -> usize {
    H().div_ceil(CELL())
}
/// The wind the cells read is tabled by blocks of `BLOCK()` world cells (see `wind_at`): 4, or
/// a flow cell if that is less.
#[allow(non_snake_case)]
#[inline(always)]
fn BLOCK() -> usize {
    CELL().min(4)
}
#[allow(non_snake_case)]
fn BW() -> usize {
    W() / BLOCK()
}
#[allow(non_snake_case)]
fn BH() -> usize {
    H().div_ceil(BLOCK())
}

/// Base wind at full strength, in world cells per step (a gale moves gas about this fast).
pub const GAIN: f32 = 1.2;
/// `GAIN` in world cells per step of this grid: the same wind in m/s on any grid.
pub fn gain() -> f32 {
    GAIN * super::units::finer()
}
/// The jet stream, as a multiple of the base wind, and the flow row it blows along (just under
/// the tropopause, `CEILING_Y`).
const JET: f32 = 0.8;
#[allow(non_snake_case)]
fn JET_ROW() -> f32 {
    super::CEILING_Y() as f32 / CELL() as f32 + 0.5
}

/// How many flow cells up the wind over a solid cell is looked for (`skim`).
const SKIM_REACH: usize = 2;
/// A wind over a pond faster than this (m/s) keeps it awake: it makes waves.
pub(super) const STIR: f32 = 1.5;

/// The wind along x over the liquid at world cell `(x, y)` from the tables `skim` (m/s) and
/// `sky`: none under a roof, else interpolated across between the middles of the flow cells
/// (the same on the graphics card).
pub fn skim_at(skim: &[f32], sky: &[f32], x: usize, y: usize) -> f32 {
    if y as f32 >= sky[x] {
        return 0.0;
    }
    let fx = (x as f32 + 0.5) / CELL() as f32 - 0.5;
    let i0 = floor_f(fx);
    let t = fx - i0;
    let i0 = i0 as i32;
    let row = (y / CELL()).min(FH() - 1) * FW();
    let a = skim[row + wrap_i(i0, FW() as i32) as usize];
    let b = skim[row + wrap_i(i0 + 1, FW() as i32) as usize];
    a + (b - a) * t
}

#[derive(Clone, Copy)]
struct Vortex {
    x: f32,
    y: f32,
    /// Radius in coarse cells.
    r: f32,
    /// Peak swirl speed in coarse cells per step (sign = sense of rotation).
    gamma: f32,
    life: u32,
}

/// The wind, and the player's settings of the weather.
#[derive(Clone, Copy, Debug)]
pub struct Wind {
    /// How it blows now, -1 (left) to 1 (right).
    pub now: f32,
    /// The strongest wind (0..1).
    pub max: f32,
    /// How fast the direction keeps changing (0 = never, 1 = usual, up to 3).
    pub changes: f32,
    /// How windy the weather is (0 calm .. 1 windy).
    pub windiness: f32,
    /// Advances with `changes`; the wind's direction follows it.
    pub clock: f32,
}

#[derive(Clone)]
pub struct Flow {
    vx: Vec<f32>,
    vy: Vec<f32>,
    ux: Vec<f32>,
    uy: Vec<f32>,
    p: Vec<f32>,
    div: Vec<f32>,
    solid: Vec<bool>,
    /// Flow cells that are mostly liquid lying on something (solid to the wind, see `refresh_solids`).
    pond: Vec<bool>,
    /// The wind along x over each flow cell, in m/s: its own, or for a solid cell the wind of
    /// the first open cell over it (`SKIM_REACH` cells up at most). What pushes the surface of
    /// the liquids (`skim`).
    skim: Vec<f32>,
    /// The first row from the top where each world column is hard: liquid under it is
    /// sheltered from the wind (in a cave, a tank, under a bridge).
    sky: Vec<f32>,
    /// How much warmer than the air each flow cell is (drives the rising of warm air).
    heat: Vec<f32>,
    /// Absolute temperature of each flow cell over the ambient one (T / T_ambient, kelvin).
    theta: Vec<f32>,
    /// Air density relative to the ambient air (1 = undisturbed); P V = n R T makes the pressure rho * theta.
    rho: Vec<f32>,
    /// Absolute pressure relative to the ambient one (rho * theta).
    pr: Vec<f32>,
    scratch: Vec<f32>,
    /// Relative humidity of the air, carried by the wind (0..1).
    moist: Vec<f32>,
    /// Fraction of each cell that is water or damp ground (feeds `moist`).
    wet: Vec<f32>,
    /// Compressible air (default): density, pressure and temperature are coupled by the ideal gas
    /// law and the wind is pushed by pressure gradients. Off: the older incompressible solver.
    pub compress: bool,
    vortices: Vec<Vortex>,
    rng: u32,
    /// Tests can pin the field.
    pub frozen: bool,
    /// The velocity at the middle of each block of `BLOCK()` world cells, in world cells per step:
    /// what the cells' rules read (`wind_at`), tabled once a flow step rather than interpolated
    /// for every cell of gas.
    gusts: Vec<(f32, f32)>,
}

#[inline]
fn at(i: i32, j: i32) -> usize {
    (j as usize) * FW() + wrap_i(i, FW() as i32) as usize
}

impl Flow {
    pub fn new() -> Self {
        let n = FW() * FH();
        Flow {
            vx: vec![0.0; n],
            vy: vec![0.0; n],
            ux: vec![0.0; n],
            uy: vec![0.0; n],
            p: vec![0.0; n],
            div: vec![0.0; n],
            solid: vec![false; n],
            pond: vec![false; n],
            skim: vec![0.0; n],
            sky: vec![H() as f32; W()],
            heat: vec![0.0; n],
            theta: vec![1.0; n],
            rho: vec![1.0; n],
            pr: vec![1.0; n],
            scratch: vec![0.0; n],
            moist: vec![0.2; n],
            wet: vec![0.0; n],
            compress: true,
            vortices: vec![],
            rng: 0x9e37_79b9,
            frozen: false,
            gusts: vec![(0.0, 0.0); BW() * BH()],
        }
    }

    fn rand(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    fn unit(&mut self) -> f32 {
        (self.rand() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Which flow cells are mostly solid ground (cells that `solid` says are hard) or liquid
    /// lying on something (`liquid`): the wind blows over a pond, not through it. Liquid that
    /// falls through the air (rain, a jet) is no wall to it. The flow cells that are mostly
    /// liquid are marked too (`pond`): the wind over them pushes their surface.
    pub fn refresh_solids(&mut self, cells: &[Cell], solid: impl Fn(&Cell) -> bool, liquid: impl Fn(&Cell) -> bool) {
        let was = self.solid.clone();
        // Every other cell of every other row, the columns read from the bottom up so that a
        // cell of liquid knows whether it lies on something.
        let mut hard = vec![0u32; FW() * FH()];
        let mut wet = vec![0u32; FW() * FH()];
        let mut n = vec![0u32; FW() * FH()];
        for (x, sky) in self.sky.iter_mut().enumerate() {
            *sky = (0..H()).find(|&y| solid(&cells[y * W() + x])).unwrap_or(H()) as f32;
        }
        for x in (0..W()).step_by(2) {
            let i = x / CELL();
            let mut lies = true;
            for y in (0..H()).rev() {
                let c = &cells[y * W() + x];
                let (h, l) = (solid(c), liquid(c));
                // Liquid lies on what is under it, if that lies; the rest lies unless it is air.
                if !l {
                    lies = h || !matches!(c.m.kind(), Kind::Empty | Kind::Gas | Kind::Flame);
                }
                if y % 2 == 0 {
                    let k = (y / CELL()) * FW() + i;
                    n[k] += 1;
                    hard[k] += h as u32;
                    wet[k] += (l && lies) as u32;
                }
            }
        }
        for k in 0..FW() * FH() {
            self.solid[k] = (hard[k] + wet[k]) * 5 >= n[k] * 2;
            self.pond[k] = self.solid[k] && wet[k] > hard[k];
        }
        // A cell a solid has just left fills with the air around it, at its density: a body
        // falling through the air must not leave a trail of air at the ambient pressure (the
        // solver holds solid cells there), which would spread as waves of pressure.
        for j in 0..FH() as i32 {
            for i in 0..FW() as i32 {
                let k = at(i, j);
                if self.solid[k] || !was[k] {
                    continue;
                }
                let (mut sum, mut cnt) = (0.0, 0.0);
                for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let nj = j + dj;
                    if nj >= 0 && nj < FH() as i32 && !was[at(i + di, nj)] && !self.solid[at(i + di, nj)] {
                        sum += self.rho[at(i + di, nj)];
                        cnt += 1.0;
                    }
                }
                if cnt > 0.0 {
                    self.rho[k] = sum / cnt;
                }
            }
        }
    }

    /// How much warmer each flow cell is than the air at its height (`ambient[y]`, per world row).
    pub fn refresh_heat(&mut self, temp: &[f32], ambient: &[f32]) {
        for j in 0..FH() {
            for i in 0..FW() {
                let (mut sum, mut n) = (0.0, 0);
                for dy in (0..CELL()).step_by(2) {
                    let y = j * CELL() + dy;
                    if y >= H() {
                        break;
                    }
                    for dx in (0..CELL()).step_by(2) {
                        sum += temp[y * W() + i * CELL() + dx] - ambient[y];
                        n += 1;
                    }
                }
                self.heat[j * FW() + i] = sum / n.max(1) as f32;
                let amb = ambient[(j * CELL() + CELL() / 2).min(H() - 1)] + 273.15;
                let k = j * FW() + i;
                let theta = ((amb + self.heat[k]) / amb.max(1.0)).clamp(0.5, 3.0);
                // Heating in the open air makes the air expand at once (sound crosses a room in no
                // time, here it would take many steps), so most of the expansion happens right away
                // and only a tenth is left as overpressure; a closed room still keeps some.
                if !self.solid[k] {
                    self.rho[k] = (self.rho[k] * (self.theta[k] / theta).powf(0.9)).clamp(0.2, 4.0);
                }
                self.theta[k] = theta;
            }
        }
    }

    /// How much of each flow cell is open water, mud or cloud: the air there is fed with moisture.
    pub fn refresh_wet(&mut self, cells: &[Cell], is_wet: impl Fn(&Cell) -> bool) {
        for j in 0..FH() {
            for i in 0..FW() {
                let (mut n, mut wet) = (0, 0);
                for dy in (0..CELL()).step_by(2) {
                    let y = j * CELL() + dy;
                    if y >= H() {
                        break;
                    }
                    for dx in (0..CELL()).step_by(2) {
                        n += 1;
                        wet += is_wet(&cells[y * W() + i * CELL() + dx]) as u32;
                    }
                }
                self.wet[j * FW() + i] = wet as f32 / n.max(1) as f32;
            }
        }
        // A pond is solid to the wind: the air over it is fed with its moisture.
        for k in (0..FW() * (FH() - 1)).rev() {
            if !self.solid[k] && self.pond[k + FW()] {
                self.wet[k] = self.wet[k].max(self.wet[k + FW()]);
            }
        }
    }

    /// Relative humidity of the air at a world position (0..1), as carried by the wind.
    /// In a pond, it is the air's over it.
    pub fn moisture_at(&self, x: i32, y: i32) -> f32 {
        let (i, j) = ((x.max(0) as usize / CELL()).min(FW() - 1), (y.max(0) as usize / CELL()).min(FH() - 1));
        self.moist[self.over(j * FW() + i)]
    }

    /// Flow cell `k`, or for a solid one the first open cell over it (`SKIM_REACH` up at most).
    fn over(&self, k: usize) -> usize {
        let mut q = k;
        for _ in 0..SKIM_REACH {
            if !self.solid[q] || q < FW() {
                break;
            }
            q -= FW();
        }
        q
    }

    /// Absolute air pressure relative to the ambient one at a world position, when the air is
    /// compressible.
    pub fn air_pressure(&self, x: i32, y: i32) -> Option<f32> {
        if !self.compress {
            return None;
        }
        // Bilinear between the centres of the flow cells, so the picture has no visible blocks.
        let fx = (x as f32 + 0.5) / CELL() as f32 - 0.5;
        let fy = ((y as f32 + 0.5) / CELL() as f32 - 0.5).clamp(0.0, FH() as f32 - 1.001);
        let (i0, j0) = (floor_f(fx), floor_f(fy));
        let (tx, ty) = (fx - i0, fy - j0);
        let (i0, j0) = (i0 as i32, j0 as i32);
        let a = self.pr[at(i0, j0)] * (1.0 - tx) + self.pr[at(i0 + 1, j0)] * tx;
        let b = self.pr[at(i0, j0 + 1)] * (1.0 - tx) + self.pr[at(i0 + 1, j0 + 1)] * tx;
        Some(a * (1.0 - ty) + b * ty)
    }

    /// Excess air pressure of one flow cell over the ambient one, in atmospheres.
    pub fn excess_at_cell(&self, i: usize, j: usize) -> f32 {
        if self.compress { self.pr[j * FW() + i] - 1.0 } else { 0.0 }
    }

    /// A blast: compresses the air around `(x, y)` (world cells) within `r` world cells by up to `amount`.
    pub fn blast(&mut self, x: i32, y: i32, r: i32, amount: f32) {
        let reach = (r / CELL() as i32 + 1).max(1);
        let (ci, cj) = (x / CELL() as i32, y / CELL() as i32);
        for dj in -reach..=reach {
            let j = cj + dj;
            if j < 0 || j >= FH() as i32 {
                continue;
            }
            for di in -reach..=reach {
                let k = at(ci + di, j);
                if self.solid[k] {
                    continue;
                }
                let d = ((di * di + dj * dj) as f32).sqrt() * CELL() as f32 / r.max(1) as f32;
                if d < 1.5 {
                    self.rho[k] = (self.rho[k] + amount * (-d * d * 1.5).exp()).min(3.5);
                }
            }
        }
    }

    /// The wind the layers of air tend towards, in coarse cells per step, at flow row `j`:
    /// bands that blow opposite ways, drifting slowly up and down, and just under the tropopause
    /// the jet stream, the strongest wind of all, always with the prevailing wind. It is what
    /// drags the top of a storm cloud downwind into a long anvil.
    pub fn profile(wind: f32, j: usize, ticks: u32) -> f32 {
        let height = 1.0 - (j as f32 + 0.5) / FH() as f32;
        let phase = ticks as f32 / 11000.0;
        let bands = (std::f32::consts::TAU * (height * 1.7 + phase)).sin();
        let layers = 0.75 * bands + 0.35 + 0.35 * height;
        let jet = (-((j as f32 + 0.5 - JET_ROW()) / 2.5).powi(2)).exp();
        wind * (GAIN / super::units::REF_FLOW_CELL as f32) * super::units::flow_scale() * (layers * (1.0 - jet) + JET * jet)
    }

    /// Adds a swirl: velocity going round `(x, y)` (coarse cell units).
    fn swirl(&mut self, v: Vortex, strength: f32) {
        let reach = (v.r * 2.6).ceil() as i32;
        for dj in -reach..=reach {
            let j = (v.y as i32 + dj).clamp(0, FH() as i32 - 1);
            for di in -reach..=reach {
                let i = v.x as i32 + di;
                let k = at(i, j);
                if self.solid[k] {
                    continue;
                }
                let (dx, dy) = (i as f32 + 0.5 - v.x, j as f32 + 0.5 - v.y);
                let d2 = (dx * dx + dy * dy) / (v.r * v.r);
                // A Lamb-Oseen vortex: solid-body core, decaying tail.
                let mag = v.gamma * strength * (1.0 - (-d2 * 2.0).exp()) / d2.sqrt().max(0.3) * (-d2 * 0.25).exp();
                let d = (dx * dx + dy * dy).sqrt().max(1e-3);
                self.vx[k] += -dy / d * mag;
                self.vy[k] += dx / d * mag;
            }
        }
    }

    /// A gust curls up: a new vortex somewhere in the open air (`gamma` in flow cells a step of
    /// the design grid: the same swirl in m/s on any grid).
    pub fn spawn_vortex(&mut self, x: f32, y: f32, r: f32, gamma: f32) {
        self.vortices.push(Vortex { x, y, r, gamma: gamma * super::units::flow_scale(), life: 40 });
    }

    pub fn step(&mut self, wind: f32, ticks: u32) {
        if self.frozen {
            return;
        }
        // Now and then a gust curls into a vortex, in either sense.
        if self.rand() % 500 == 0 && self.vortices.len() < 4 {
            let (x, y) = (self.unit() * FW() as f32, (0.1 + 0.5 * self.unit()) * FH() as f32);
            if !self.solid[at(x as i32, y as i32)] {
                let r = 3.0 + 3.5 * self.unit();
                let gamma = (0.07 + 0.08 * self.unit()) * if self.rand() % 2 == 0 { 1.0 } else { -1.0 };
                self.spawn_vortex(x, y, r, gamma);
            }
        }
        for v in std::mem::take(&mut self.vortices) {
            self.swirl(v, 0.12);
            if v.life > 1 {
                self.vortices.push(Vortex { life: v.life - 1, ..v });
            }
        }

        // Forces: the layers pull the air towards their wind, warm air rises, cold air sinks.
        let s = super::units::flow_scale();
        for j in 0..FH() {
            let target = Self::profile(wind, j, ticks);
            for i in 0..FW() {
                let k = j * FW() + i;
                if self.solid[k] {
                    continue;
                }
                self.vx[k] += (target - self.vx[k]) * 0.012;
                if !self.compress {
                    self.vy[k] -= (self.heat[k] / 12.0).clamp(-1.0, 3.0) * 0.012 * s;
                }
            }
        }
        self.advect();
        if self.compress {
            self.compress_step();
        } else {
            self.project(24);
        }
        self.carry_moisture();
        let damp = 0.997;
        let most = 0.6 * s;
        for k in 0..FW() * FH() {
            if self.solid[k] {
                self.vx[k] = 0.0;
                self.vy[k] = 0.0;
            } else {
                self.vx[k] *= damp;
                self.vy[k] *= damp;
                // Keep numerical accidents from running away.
                self.vx[k] = self.vx[k].clamp(-most, most);
                self.vy[k] = self.vy[k].clamp(-most, most);
            }
        }
        self.tabulate();
    }

    /// Compressible air: the pressure `rho * theta` (ideal gas, P V = n R T) pushes the wind from high
    /// to low, the wind moves the air and so changes its density, and air lighter than its
    /// surroundings (warmer, so thinner at the same pressure: 1 - 1/theta) floats up. Hot air at first expands, a closed room keeps its pressure for a while.
    fn compress_step(&mut self) {
        // Squared speed of sound, coarse cells² per step², and the lift of warm air, coarse cells
        // per step², on the design grid (`flow_scale`: the same in m/s on any grid).
        let s = super::units::flow_scale();
        let (c2, buoyancy) = (0.12 * s * s, 0.3 * s);
        for k in 0..FW() * FH() {
            self.pr[k] = if self.solid[k] { 1.0 } else { self.rho[k] * self.theta[k] };
        }
        for j in 0..FH() as i32 {
            for i in 0..FW() as i32 {
                let k = at(i, j);
                if self.solid[k] {
                    continue;
                }
                let pc = self.pr[k];
                let p = |di: i32, dj: i32| if self.solid_at(i + di, j + dj) { pc } else { self.pr[at(i + di, j + dj)] };
                let (gx, gy) = (0.5 * (p(1, 0) - p(-1, 0)), 0.5 * (p(0, 1) - p(0, -1)));
                let inv = 1.0 / self.rho[k].max(0.3);
                self.vx[k] -= c2 * gx * inv;
                self.vy[k] -= c2 * gy * inv + buoyancy * (1.0 - 1.0 / self.theta[k]).clamp(-0.1, 0.15);
            }
        }
        // Continuity, in flux form with upwind density: mass is moved, not created.
        self.scratch.copy_from_slice(&self.rho);
        for j in 0..FH() as i32 {
            for i in 0..FW() as i32 {
                let k = at(i, j);
                if self.solid[k] {
                    continue;
                }
                if !self.solid_at(i + 1, j) {
                    let e = at(i + 1, j);
                    let vf = (0.5 * (self.vx[k] + self.vx[e])).clamp(-0.4, 0.4);
                    let fl = vf * if vf > 0.0 { self.rho[k] } else { self.rho[e] };
                    self.scratch[k] -= fl;
                    self.scratch[e] += fl;
                }
                if !self.solid_at(i, j + 1) {
                    let s = at(i, j + 1);
                    let vf = (0.5 * (self.vy[k] + self.vy[s])).clamp(-0.4, 0.4);
                    let fl = vf * if vf > 0.0 { self.rho[k] } else { self.rho[s] };
                    self.scratch[k] -= fl;
                    self.scratch[s] += fl;
                }
            }
        }
        // A little diffusion keeps the waves stable; the open atmosphere slowly relaxes the pressure.
        for j in 0..FH() as i32 {
            for i in 0..FW() as i32 {
                let k = at(i, j);
                if self.solid[k] {
                    self.rho[k] = 1.0;
                    continue;
                }
                let (mut sum, mut n) = (0.0, 0.0);
                for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    if !self.solid_at(i + di, j + dj) {
                        sum += self.scratch[at(i + di, j + dj)];
                        n += 1.0;
                    }
                }
                let mut r = self.scratch[k];
                if n > 0.0 {
                    r += (sum / n - r) * 0.05;
                }
                r += (1.0 / self.theta[k] - r) * 0.003;
                self.rho[k] = r.clamp(0.2, 4.0);
            }
        }
    }

    /// Moisture rides the wind, is fed by water and damp air, and dries out slowly.
    fn carry_moisture(&mut self) {
        self.scratch.copy_from_slice(&self.moist);
        for j in 0..FH() as i32 {
            for i in 0..FW() as i32 {
                let k = at(i, j);
                if self.solid[k] {
                    continue;
                }
                let x = i as f32 - self.vx[k] * 2.0;
                let y = (j as f32 - self.vy[k] * 2.0).clamp(0.0, FH() as f32 - 1.001);
                let (i0, j0) = (floor_f(x), floor_f(y));
                let (fx, fy) = (x - i0, y - j0);
                let (i0, j0) = (i0 as i32, j0 as i32);
                let m = |ii: i32, jj: i32| {
                    let q = at(ii, jj);
                    if self.solid[q] { self.scratch[k] } else { self.scratch[q] }
                };
                let a = m(i0, j0) * (1.0 - fx) + m(i0 + 1, j0) * fx;
                let b = m(i0, j0 + 1) * (1.0 - fx) + m(i0 + 1, j0 + 1) * fx;
                let mut v = a * (1.0 - fy) + b * fy;
                v += (1.0 - v) * (self.wet[k] * 4.0).min(1.0) * 0.05;
                v += (0.18 - v) * 0.004;
                self.moist[k] = v.clamp(0.0, 1.0);
            }
        }
    }

    /// Semi-Lagrangian advection of the velocity by itself (periodic left and right).
    fn advect(&mut self) {
        self.ux.copy_from_slice(&self.vx);
        self.uy.copy_from_slice(&self.vy);
        for j in 0..FH() as i32 {
            for i in 0..FW() as i32 {
                let k = at(i, j);
                if self.solid[k] {
                    continue;
                }
                let x = i as f32 - self.ux[k];
                let y = (j as f32 - self.uy[k]).clamp(0.0, FH() as f32 - 1.001);
                let (i0, j0) = (floor_f(x), floor_f(y));
                let (fx, fy) = (x - i0, y - j0);
                let (i0, j0) = (i0 as i32, j0 as i32);
                let s = |a: &[f32]| {
                    let (k00, k10, k01, k11) = (at(i0, j0), at(i0 + 1, j0), at(i0, j0 + 1), at(i0 + 1, j0 + 1));
                    (a[k00] * (1.0 - fx) + a[k10] * fx) * (1.0 - fy) + (a[k01] * (1.0 - fx) + a[k11] * fx) * fy
                };
                self.vx[k] = s(&self.ux);
                self.vy[k] = s(&self.uy);
            }
        }
    }

    fn solid_at(&self, i: i32, j: i32) -> bool {
        j < 0 || j >= FH() as i32 || self.solid[at(i, j)]
    }

    /// Makes the flow divergence-free (air neither piles up nor appears), going round solids.
    fn project(&mut self, iterations: usize) {
        for j in 0..FH() as i32 {
            for i in 0..FW() as i32 {
                let k = at(i, j);
                self.div[k] = if self.solid[k] {
                    0.0
                } else {
                    let ux1 = if self.solid_at(i + 1, j) { 0.0 } else { self.vx[at(i + 1, j)] };
                    let ux0 = if self.solid_at(i - 1, j) { 0.0 } else { self.vx[at(i - 1, j)] };
                    let vy1 = if self.solid_at(i, j + 1) { 0.0 } else { self.vy[at(i, j + 1)] };
                    let vy0 = if self.solid_at(i, j - 1) { 0.0 } else { self.vy[at(i, j - 1)] };
                    -0.5 * (ux1 - ux0 + vy1 - vy0)
                };
            }
        }
        for _ in 0..iterations {
            for j in 0..FH() as i32 {
                for i in 0..FW() as i32 {
                    let k = at(i, j);
                    if self.solid[k] {
                        self.p[k] = 0.0;
                        continue;
                    }
                    let (mut sum, mut n) = (0.0, 0.0);
                    for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        if !self.solid_at(i + di, j + dj) {
                            sum += self.p[at(i + di, j + dj)];
                            n += 1.0;
                        }
                    }
                    self.p[k] = if n > 0.0 { (sum + self.div[k]) / n } else { 0.0 };
                }
            }
        }
        for j in 0..FH() as i32 {
            for i in 0..FW() as i32 {
                let k = at(i, j);
                if self.solid[k] {
                    continue;
                }
                let pc = self.p[k];
                let p = |di: i32, dj: i32| if self.solid_at(i + di, j + dj) { pc } else { self.p[at(i + di, j + dj)] };
                let (gx, gy) = (0.5 * (p(1, 0) - p(-1, 0)), 0.5 * (p(0, 1) - p(0, -1)));
                self.vx[k] -= gx;
                self.vy[k] -= gy;
            }
        }
    }

    /// The air velocity at a world position, in world cells per step (bilinear, periodic in x).
    pub fn sample(&self, x: i32, y: i32) -> (f32, f32) {
        self.sample_at(x as f32 + 0.5, y as f32 + 0.5)
    }

    /// The air velocity at a position in world cells (the middle of cell x is x + 0.5).
    fn sample_at(&self, x: f32, y: f32) -> (f32, f32) {
        let fx = x / CELL() as f32 - 0.5;
        let fy = (y / CELL() as f32 - 0.5).clamp(0.0, FH() as f32 - 1.001);
        let (i0, j0) = (floor_f(fx), floor_f(fy));
        let (tx, ty) = (fx - i0, fy - j0);
        // The four cells round it, wrapped (a plane flying in from off the screen, a drop on
        // its way out of the world, ask for the wind beyond the edges).
        let (i0, j0) = (i0 as i32, j0 as usize);
        let (ia, ib) = (wrap_i(i0, FW() as i32) as usize, wrap_i(i0 + 1, FW() as i32) as usize);
        let (k0, k1) = (j0 * FW(), (j0 + 1) * FW());
        let s = |a: &[f32]| (a[k0 + ia] * (1.0 - tx) + a[k0 + ib] * tx) * (1.0 - ty) + (a[k1 + ia] * (1.0 - tx) + a[k1 + ib] * tx) * ty;
        (s(&self.vx) * CELL() as f32, s(&self.vy) * CELL() as f32)
    }

    /// The wind at a world cell as the cells' rules read it: the velocity sampled at the middle
    /// of the cell's block of `BLOCK()` × `BLOCK()` cells, in world cells per step. A gas cell reads
    /// the wind once or twice a step, and there can be a hundred thousand of them: the table is
    /// a lookup where `sample` interpolates.
    #[inline]
    pub fn wind_at(&self, x: i32, y: i32) -> (f32, f32) {
        debug_assert!(x >= 0 && x < W() as i32 && y >= 0 && y < H() as i32);
        self.gusts[(y as usize / BLOCK()).min(BH() - 1) * BW() + (x as usize / BLOCK()).min(BW() - 1)]
    }

    /// Tables the wind of every block (`wind_at`), after the field has changed.
    fn tabulate(&mut self) {
        let mut gusts = std::mem::take(&mut self.gusts);
        gusts.par_chunks_mut(BW()).enumerate().for_each(|(by, row)| {
            let y = (by * BLOCK()) as f32 + BLOCK() as f32 / 2.0;
            for (bx, g) in row.iter_mut().enumerate() {
                *g = self.sample_at((bx * BLOCK()) as f32 + BLOCK() as f32 / 2.0, y);
            }
        });
        self.gusts = gusts;
        // The wind over the surface of the liquids, from the first open cell up.
        for k in 0..FW() * FH() {
            let q = self.over(k);
            self.skim[k] = if self.solid[q] { 0.0 } else { self.vx[q] * CELL() as f32 * super::units::cell_m() / super::units::TICK };
        }
    }

    /// The wind along x over the surface of a liquid at world cell `(x, y)`, in m/s: the wind of
    /// the open air over it, between the middles of the flow cells across.
    pub fn skim(&self, x: usize, y: usize) -> f32 {
        skim_at(&self.skim, &self.sky, x, y)
    }

    /// The wind over the surface of the liquids, every flow cell (for the graphics card).
    pub fn skim_table(&self) -> &[f32] {
        &self.skim
    }

    /// The first hard row of every world column (for the graphics card).
    pub fn sky_table(&self) -> &[f32] {
        &self.sky
    }

    /// Wakes the zones whose liquid the wind over it is strong enough to stir.
    pub(super) fn wake_ponds(&self, zones: &mut super::sleep::Zones) {
        for k in 0..FW() * FH() {
            let (x, y) = ((k % FW()) * CELL() + CELL() / 2, ((k / FW()) * CELL()).min(H() - 1));
            if self.pond[k] && self.skim(x, y).abs() > STIR {
                zones.wake(y * W() + x);
            }
        }
    }

    /// The solver's pressure at a world position (arbitrary units: it is the push that keeps the air
    /// from piling up, large where a stream meets an obstacle).
    pub fn pressure_at(&self, x: i32, y: i32) -> f32 {
        let (i, j) = ((x.max(0) as usize / CELL()).min(FW() - 1), (y.max(0) as usize / CELL()).min(FH() - 1));
        self.p[j * FW() + i]
    }

    /// Whole-field access for drawing and tests: velocity of flow cell `(i, j)` in world cells per step.
    pub fn at_cell(&self, i: usize, j: usize) -> (f32, f32) {
        (self.vx[j * FW() + i] * CELL() as f32, self.vy[j * FW() + i] * CELL() as f32)
    }

    pub fn is_solid(&self, i: usize, j: usize) -> bool {
        self.solid[j * FW() + i]
    }

    /// Largest leftover divergence (for tests).
    #[cfg(test)]
    pub fn max_divergence(&self) -> f32 {
        let mut worst = 0.0f32;
        for j in 1..FH() as i32 - 1 {
            for i in 0..FW() as i32 {
                if self.solid_at(i, j) || self.solid_at(i + 1, j) || self.solid_at(i - 1, j) || self.solid_at(i, j + 1) || self.solid_at(i, j - 1) {
                    continue;
                }
                let d = 0.5 * (self.vx[at(i + 1, j)] - self.vx[at(i - 1, j)] + self.vy[at(i, j + 1)] - self.vy[at(i, j - 1)]);
                worst = worst.max(d.abs());
            }
        }
        worst
    }

    #[cfg(test)]
    pub fn set_cell(&mut self, i: usize, j: usize, vx: f32, vy: f32) {
        self.vx[j * FW() + i] = vx;
        self.vy[j * FW() + i] = vy;
    }

    #[cfg(test)]
    pub fn set_wet(&mut self, i: usize, j: usize, w: f32) {
        self.wet[j * FW() + i] = w;
    }

    #[cfg(test)]
    pub fn set_solid(&mut self, i: usize, j: usize, s: bool) {
        self.solid[j * FW() + i] = s;
    }

    #[cfg(test)]
    pub fn set_heat(&mut self, i: usize, j: usize, h: f32) {
        self.heat[j * FW() + i] = h;
        self.theta[j * FW() + i] = 1.0 + h / 288.0;
    }

    #[cfg(test)]
    pub fn project_for_test(&mut self) {
        self.project(60);
    }

    /// Fills the whole field with one velocity (world cells per step) and pins it (for tests).
    #[cfg(test)]
    pub fn pin_uniform(&mut self, vx: f32, vy: f32) {
        self.vx.fill(vx / CELL() as f32);
        self.vy.fill(vy / CELL() as f32);
        self.frozen = true;
        self.tabulate();
    }

    /// Pins the field to `vx` above flow row `split` and `-vx` below it: two opposing winds.
    #[cfg(test)]
    pub fn pin_opposed(&mut self, vx: f32, split: usize) {
        for j in 0..FH() {
            for i in 0..FW() {
                self.vx[j * FW() + i] = if j < split { vx } else { -vx } / CELL() as f32;
                self.vy[j * FW() + i] = 0.0;
            }
        }
        self.frozen = true;
        self.tabulate();
    }

    /// Sets the humidity of all the air (for tests; pin the field too, or the wind moves it).
    #[cfg(test)]
    pub fn pin_moisture(&mut self, v: f32) {
        self.moist.fill(v);
    }

    #[cfg(test)]
    pub fn inject_vortex_for_test(&mut self, x: f32, y: f32, r: f32, gamma: f32) {
        self.swirl(Vortex { x, y, r, gamma, life: 1 }, 1.0);
        self.tabulate();
    }
}

impl World {
    /// Keeps the air's flow up to date: ground and heat are re-read now and then, the flow
    /// itself is advanced every other step.
    pub(super) fn update_flow(&mut self) {
        let t = self.ticks;
        use super::cadence::{due, Pass};
        if due(Pass::FlowSolids, t) {
            self.refresh_flow_solids();
        }
        if due(Pass::FlowHeat, t) {
            let ambient: Vec<f32> = (0..H() as i32).map(|y| self.ambient_c(y)).collect();
            self.flow.refresh_heat(&self.temp, &ambient);
            self.flow.refresh_wet(&self.cells, |c| matches!(c.m, super::Mat::Water | super::Mat::Mud | super::Mat::Cloud | super::Mat::Steam));
        }
        if due(Pass::FlowStep, t) {
            let wind = self.wind.now;
            self.flow.step(wind, t);
        }
        self.flow.wake_ponds(&mut self.zones);
    }

    /// What the wind goes round: hard ground, and liquid lying on something.
    fn refresh_flow_solids(&mut self) {
        self.flow.refresh_solids(&self.cells, |c| c.m.is_hard() && !c.m.is_vegetation(), |c| c.m.kind() == Kind::Liquid);
    }

    /// Where the air carries a gas cell this step: along the local flow, a cell or two at most.
    /// The fractional part of the speed is decided by chance, so slow air still moves gas now and then.
    pub(super) fn wind_target(&mut self, x: i32, y: i32) -> Option<(i32, i32)> {
        let (vx, vy) = self.flow.wind_at(x, y);
        let mut round = |v: f32| -> i32 {
            let v = v.clamp(-3.0, 3.0);
            let base = v.trunc() as i32;
            let frac = v - v.trunc();
            base + if (self.rand() % 1000) as f32 / 1000.0 < frac.abs() { frac.signum() as i32 } else { 0 }
        };
        let (dx, dy) = (round(vx), round(vy));
        if dx == 0 && dy == 0 {
            return None;
        }
        // The full move if it is free, else the same move shortened, else along one axis only.
        let tries = [(dx, dy), (dx.signum(), dy.signum()), (dx, 0), (0, dy), (dx.signum(), 0), (0, dy.signum())];
        for (tx, ty) in tries {
            if tx == 0 && ty == 0 {
                continue;
            }
            let (nx, ny) = (self.edge_x(x, y, x + tx), y + ty);
            // The wind does not carry gas up past the top of the weather.
            if ty < 0 && !self.gas_may_rise_to(x, y, ny) {
                continue;
            }
            if ny >= 0 && ny < H() as i32 && self.at(nx, ny) == super::Mat::Empty {
                return Some((nx, ny));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: &mut Flow, wind: f32, steps: u32) {
        for t in 0..steps {
            f.step(wind, t);
        }
    }

    #[test]
    fn projection_removes_most_of_the_divergence() {
        let mut f = Flow::new();
        // A smooth but strongly diverging field (air pouring out of one place).
        for j in 0..FH() {
            for i in 0..FW() {
                let (x, y) = (i as f32 * 0.21, j as f32 * 0.17);
                f.vx[j * FW() + i] = 0.05 * (x.sin() + (y * 1.3).cos());
                f.vy[j * FW() + i] = 0.05 * ((x * 0.8).cos() + y.sin());
            }
        }
        let before = f.max_divergence();
        // (The pressure is kept from one step to the next in the game, so it converges over a few steps.)
        for _ in 0..12 {
            f.project_for_test();
        }
        let after = f.max_divergence();
        assert!(before > 0.01, "the test field must diverge: {before}");
        assert!(after < before * 0.25, "divergence {before} -> {after}");
    }

    #[test]
    fn the_wind_blows_over_a_pond_and_through_the_rain() {
        let mut w = World::new_static();
        let set = |w: &mut World, x0: i32, y0: i32, x1: i32, y1: i32, m: super::super::Mat| {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    w.set(x, y, m);
                }
            }
        };
        use super::super::Mat;
        // Ground, a pond on it 4 flow cells deep, and a column of water falling through the air.
        set(&mut w, 0, 480, W() as i32 - 1, H() as i32 - 1, Mat::Brick);
        set(&mut w, 200, 448, 399, 479, Mat::Water);
        set(&mut w, 600, 300, 615, 400, Mat::Water);
        w.refresh_flow_solids();
        let f = &w.flow;
        for j in 56..60 {
            assert!(f.is_solid(30, j) && f.pond[j * FW() + 30], "pond row {j}");
        }
        assert!(f.is_solid(30, 62) && !f.pond[62 * FW() + 30], "the ground is no pond");
        assert!(!f.is_solid(30, 55), "the air over the pond");
        for j in 38..50 {
            assert!(!f.is_solid(75, j), "falling water, row {j}");
        }
    }

    #[test]
    fn flow_goes_around_solid_ground() {
        let mut f = Flow::new();
        // A block of ground in a steady wind: no air inside it, and the stream is bent around it.
        for j in 30..40 {
            for i in 50..58 {
                f.set_solid(i, j, true);
            }
        }
        for k in 0..FW() * FH() {
            f.vx[k] = 0.1;
        }
        for _ in 0..80 {
            for k in 0..FW() * FH() {
                if !f.solid[k] {
                    f.vx[k] += (0.1 - f.vx[k]) * 0.05;
                }
            }
            f.advect();
            f.project(30);
            for k in 0..FW() * FH() {
                if f.solid[k] {
                    f.vx[k] = 0.0;
                    f.vy[k] = 0.0;
                }
            }
        }
        for j in 30..40 {
            for i in 50..58 {
                assert_eq!(f.at_cell(i, j), (0.0, 0.0), "air inside the rock");
            }
        }
        // Over the top of the block the air is pushed up and then down: vertical motion appears.
        let bent = (44..64).map(|i| f.at_cell(i, 28).1.abs()).fold(0.0f32, f32::max);
        assert!(bent > 0.01, "the wind should bend round the rock: {bent}");
    }

    #[test]
    fn a_vortex_swirls_and_fades() {
        let mut f = Flow::new();
        f.inject_vortex_for_test(60.0, 30.0, 4.0, 0.1);
        // Going round: air to the right of the centre moves one way vertically, to the left the other way.
        let right = f.at_cell(64, 30).1;
        let left = f.at_cell(56, 30).1;
        assert!(right * left < 0.0 && right.abs() > 0.1, "opposite sides move opposite ways: {left} {right}");
        let start = right.abs();
        run(&mut f, 0.0, 60);
        let (mut peak, mut j_at) = (0.0f32, 0);
        for j in 20..40 {
            for i in 40..80 {
                let v = f.at_cell(i, j);
                let s = (v.0 * v.0 + v.1 * v.1).sqrt();
                if s > peak {
                    peak = s;
                    j_at = j;
                }
            }
        }
        assert!(peak > start * 0.1, "the vortex should not vanish at once: {peak} vs {start}");
        assert!(peak < start * 3.0, "and it must not blow up: {peak} vs {start} (row {j_at})");
    }

    #[test]
    fn a_blast_sends_a_pressure_wave_that_dies_away() {
        let mut f = Flow::new();
        f.blast(480, 270, 28, 0.8);
        let peak_at = |f: &Flow, di: usize| (f.rho[33 * FW() + 60 + di] * f.theta[33 * FW() + 60 + di]).max(f.rho[33 * FW() + 60 - di] * f.theta[33 * FW() + 60 - di]);
        assert!(peak_at(&f, 0) > 1.4, "compressed at the centre: {}", peak_at(&f, 0));
        run(&mut f, 0.0, 14);
        // The front has left the centre and reached cells further out.
        assert!(peak_at(&f, 0) < 1.3, "the centre relaxed: {}", peak_at(&f, 0));
        let out = (6..14).map(|d| peak_at(&f, d)).fold(0.0f32, f32::max);
        assert!(out > 1.01, "a wave runs outward: {out}");
        run(&mut f, 0.0, 600);
        let worst = (0..FW() * FH()).map(|k| (f.pr[k] - 1.0).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.05, "and it settles: {worst}");
    }

    #[test]
    fn a_sealed_room_keeps_its_pressure_when_heated() {
        let mut open = Flow::new();
        let mut shut = Flow::new();
        for j in 30..40 {
            for i in 50..60 {
                if j == 30 || j == 39 || i == 50 || i == 59 {
                    shut.set_solid(i, j, true);
                }
            }
        }
        for f in [&mut open, &mut shut] {
            for j in 32..38 {
                for i in 52..58 {
                    f.set_heat(i, j, 150.0);
                }
            }
        }
        // Mean pressure inside the room over a while (sound rings in it for a bit).
        let mean = |f: &Flow| (31..39).flat_map(|j| (51..59).map(move |i| (i, j))).map(|(i, j)| f.pr[j * FW() + i]).sum::<f32>() / 64.0;
        let (mut o, mut s) = (0.0, 0.0);
        for t in 0..90 {
            open.step(0.0, t);
            shut.step(0.0, t);
            if t >= 60 {
                o += mean(&open) / 30.0;
                s += mean(&shut) / 30.0;
            }
        }
        assert!(s > 1.03 && s > o + 0.04, "a closed room is over-pressured, an open one is not: {s} vs {o}");
    }

    #[test]
    fn the_wind_carries_moisture_downwind() {
        let mut f = Flow::new();
        f.pin_uniform(0.4, 0.0);
        f.frozen = false;
        for t in 0..400 {
            f.vx.fill(0.05);
            f.vy.fill(0.0);
            f.set_wet(30, 20, 1.0);
            f.carry_moisture();
            let _ = t;
        }
        let down = f.moist[20 * FW() + 34];
        let up = f.moist[20 * FW() + 26];
        assert!(down > 0.4 && down > up + 0.2, "damp air is blown downwind: {down} vs {up}");
    }

    #[test]
    fn warm_air_rises_above_a_fire() {
        let mut f = Flow::new();
        for j in 50..60 {
            for i in 0..FW() {
                if j == 59 {
                    continue;
                }
                let _ = (i, j);
            }
        }
        // A hot patch near the ground in still air.
        for j in 55..58 {
            for i in 58..62 {
                f.set_heat(i, j, 300.0);
            }
        }
        run(&mut f, 0.0, 120);
        let above = (46..54).map(|j| f.at_cell(60, j).1).sum::<f32>() / 8.0;
        assert!(above < -0.1, "air above a fire rises (negative y): {above}");
        // And it stays calm far away (some air comes in to replace what rose, a little).
        let far = f.at_cell(10, 20);
        assert!(far.1.abs() < 0.2, "{far:?}");
    }

    #[test]
    fn layers_of_air_blow_opposite_ways() {
        let mut f = Flow::new();
        run(&mut f, 1.0, 1500);
        let (mut right, mut left) = (0.0f32, 0.0f32);
        for j in 0..FH() {
            let v: f32 = (0..FW()).map(|i| f.at_cell(i, j).0).sum::<f32>() / FW() as f32;
            right = right.max(v);
            left = left.min(v);
        }
        assert!(right > 0.2 && left < -0.2, "opposed winds at different heights: {left} .. {right}");
    }

    #[test]
    fn the_solver_stays_stable_for_a_long_time() {
        let mut f = Flow::new();
        for t in 0..3000 {
            f.step(0.8, t);
        }
        let worst = (0..FW() * FH()).map(|k| f.vx[k].abs().max(f.vy[k].abs())).fold(0.0f32, f32::max);
        assert!(worst.is_finite() && worst <= 0.6 + 1e-3, "speed stays bounded: {worst}");
    }

    #[test]
    fn sampling_is_smooth_and_wraps_sideways() {
        let mut f = Flow::new();
        f.set_cell(0, 10, 0.05, 0.0);
        let a = f.sample(0, 85).0;
        let b = f.sample(W() as i32 - 1, 85).0;
        assert!(a > 0.0 && b >= 0.0, "the wrap-around neighbour feels the left edge: {a} {b}");
    }
}
