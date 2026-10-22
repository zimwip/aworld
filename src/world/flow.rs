//! A simplified flow of the air ("stable fluids" on a coarse grid): winds in layers that blow
//! opposite ways, gusts that curl into vortices, warm air rising, and a flow that goes around
//! solid ground. Clouds, steam and smoke are carried along; the cells themselves stay as they are.
//!
//! Velocities are kept in coarse cells per step (a coarse cell is `CELL` world cells wide).

use super::{Cell, World, H, W};

/// Width of a flow cell, in world cells.
pub const CELL: usize = 8;
pub const FW: usize = W / CELL;
pub const FH: usize = H.div_ceil(CELL);

/// Base wind at full strength, in world cells per step (a gale moves gas about this fast).
pub const GAIN: f32 = 1.2;

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

pub struct Flow {
    vx: Vec<f32>,
    vy: Vec<f32>,
    ux: Vec<f32>,
    uy: Vec<f32>,
    p: Vec<f32>,
    div: Vec<f32>,
    solid: Vec<bool>,
    /// How much warmer than the air each flow cell is (drives the rising of warm air).
    heat: Vec<f32>,
    vortices: Vec<Vortex>,
    rng: u32,
    /// Tests can pin the field.
    pub frozen: bool,
}

#[inline]
fn at(i: i32, j: i32) -> usize {
    (j as usize) * FW + i.rem_euclid(FW as i32) as usize
}

impl Flow {
    pub fn new() -> Self {
        let n = FW * FH;
        Flow {
            vx: vec![0.0; n],
            vy: vec![0.0; n],
            ux: vec![0.0; n],
            uy: vec![0.0; n],
            p: vec![0.0; n],
            div: vec![0.0; n],
            solid: vec![false; n],
            heat: vec![0.0; n],
            vortices: vec![],
            rng: 0x9e37_79b9,
            frozen: false,
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

    /// Which flow cells are mostly solid ground (cells that `solid` says are hard).
    pub fn refresh_solids(&mut self, cells: &[Cell], solid: impl Fn(&Cell) -> bool) {
        for j in 0..FH {
            for i in 0..FW {
                let (mut n, mut hard) = (0, 0);
                for dy in (0..CELL).step_by(2) {
                    let y = j * CELL + dy;
                    if y >= H {
                        break;
                    }
                    for dx in (0..CELL).step_by(2) {
                        n += 1;
                        hard += solid(&cells[y * W + i * CELL + dx]) as u32;
                    }
                }
                self.solid[j * FW + i] = hard * 5 >= n * 2;
            }
        }
    }

    /// How much warmer each flow cell is than the air at its height (`ambient[y]`, per world row).
    pub fn refresh_heat(&mut self, temp: &[f32], ambient: &[f32]) {
        for j in 0..FH {
            for i in 0..FW {
                let (mut sum, mut n) = (0.0, 0);
                for dy in (0..CELL).step_by(2) {
                    let y = j * CELL + dy;
                    if y >= H {
                        break;
                    }
                    for dx in (0..CELL).step_by(2) {
                        sum += temp[y * W + i * CELL + dx] - ambient[y];
                        n += 1;
                    }
                }
                self.heat[j * FW + i] = sum / n.max(1) as f32;
            }
        }
    }

    /// The wind the layers of air tend towards, in coarse cells per step, at flow row `j`:
    /// bands that blow opposite ways, drifting slowly up and down.
    pub fn profile(wind: f32, j: usize, ticks: u32) -> f32 {
        let height = 1.0 - (j as f32 + 0.5) / FH as f32;
        let phase = ticks as f32 / 11000.0;
        let bands = (std::f32::consts::TAU * (height * 1.7 + phase)).sin();
        wind * (GAIN / CELL as f32) * (0.75 * bands + 0.35 + 0.35 * height)
    }

    /// Adds a swirl: velocity going round `(x, y)` (coarse cell units).
    fn swirl(&mut self, v: Vortex, strength: f32) {
        let reach = (v.r * 2.6).ceil() as i32;
        for dj in -reach..=reach {
            let j = (v.y as i32 + dj).clamp(0, FH as i32 - 1);
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

    /// A gust curls up: a new vortex somewhere in the open air.
    pub fn spawn_vortex(&mut self, x: f32, y: f32, r: f32, gamma: f32) {
        self.vortices.push(Vortex { x, y, r, gamma, life: 40 });
    }

    pub fn step(&mut self, wind: f32, ticks: u32) {
        if self.frozen {
            return;
        }
        // Now and then a gust curls into a vortex, in either sense.
        if self.rand() % 500 == 0 && self.vortices.len() < 4 {
            let (x, y) = (self.unit() * FW as f32, (0.1 + 0.5 * self.unit()) * FH as f32);
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
        for j in 0..FH {
            let target = Self::profile(wind, j, ticks);
            for i in 0..FW {
                let k = j * FW + i;
                if self.solid[k] {
                    continue;
                }
                self.vx[k] += (target - self.vx[k]) * 0.012;
                self.vy[k] -= (self.heat[k] / 12.0).clamp(-1.0, 3.0) * 0.012;
            }
        }
        self.advect();
        self.project(24);
        let damp = 0.997;
        for k in 0..FW * FH {
            if self.solid[k] {
                self.vx[k] = 0.0;
                self.vy[k] = 0.0;
            } else {
                self.vx[k] *= damp;
                self.vy[k] *= damp;
                // Keep numerical accidents from running away.
                self.vx[k] = self.vx[k].clamp(-0.6, 0.6);
                self.vy[k] = self.vy[k].clamp(-0.6, 0.6);
            }
        }
    }

    /// Semi-Lagrangian advection of the velocity by itself (periodic left and right).
    fn advect(&mut self) {
        self.ux.copy_from_slice(&self.vx);
        self.uy.copy_from_slice(&self.vy);
        for j in 0..FH as i32 {
            for i in 0..FW as i32 {
                let k = at(i, j);
                if self.solid[k] {
                    continue;
                }
                let x = i as f32 - self.ux[k];
                let y = (j as f32 - self.uy[k]).clamp(0.0, FH as f32 - 1.001);
                let (i0, j0) = (x.floor(), y.floor());
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
        j < 0 || j >= FH as i32 || self.solid[at(i, j)]
    }

    /// Makes the flow divergence-free (air neither piles up nor appears), going round solids.
    fn project(&mut self, iterations: usize) {
        for j in 0..FH as i32 {
            for i in 0..FW as i32 {
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
            for j in 0..FH as i32 {
                for i in 0..FW as i32 {
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
        for j in 0..FH as i32 {
            for i in 0..FW as i32 {
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
        let fx = (x as f32 + 0.5) / CELL as f32 - 0.5;
        let fy = ((y as f32 + 0.5) / CELL as f32 - 0.5).clamp(0.0, FH as f32 - 1.001);
        let (i0, j0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - i0, fy - j0);
        let (i0, j0) = (i0 as i32, j0 as i32);
        let s = |a: &[f32]| {
            (a[at(i0, j0)] * (1.0 - tx) + a[at(i0 + 1, j0)] * tx) * (1.0 - ty) + (a[at(i0, j0 + 1)] * (1.0 - tx) + a[at(i0 + 1, j0 + 1)] * tx) * ty
        };
        (s(&self.vx) * CELL as f32, s(&self.vy) * CELL as f32)
    }

    /// Whole-field access for drawing and tests: velocity of flow cell `(i, j)` in world cells per step.
    pub fn at_cell(&self, i: usize, j: usize) -> (f32, f32) {
        (self.vx[j * FW + i] * CELL as f32, self.vy[j * FW + i] * CELL as f32)
    }

    pub fn is_solid(&self, i: usize, j: usize) -> bool {
        self.solid[j * FW + i]
    }

    /// Largest leftover divergence (for tests).
    #[cfg(test)]
    pub fn max_divergence(&self) -> f32 {
        let mut worst = 0.0f32;
        for j in 1..FH as i32 - 1 {
            for i in 0..FW as i32 {
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
        self.vx[j * FW + i] = vx;
        self.vy[j * FW + i] = vy;
    }

    #[cfg(test)]
    pub fn set_solid(&mut self, i: usize, j: usize, s: bool) {
        self.solid[j * FW + i] = s;
    }

    #[cfg(test)]
    pub fn set_heat(&mut self, i: usize, j: usize, h: f32) {
        self.heat[j * FW + i] = h;
    }

    #[cfg(test)]
    pub fn project_for_test(&mut self) {
        self.project(60);
    }

    /// Fills the whole field with one velocity (world cells per step) and pins it (for tests).
    #[cfg(test)]
    pub fn pin_uniform(&mut self, vx: f32, vy: f32) {
        self.vx.fill(vx / CELL as f32);
        self.vy.fill(vy / CELL as f32);
        self.frozen = true;
    }

    /// Pins the field to `vx` above flow row `split` and `-vx` below it: two opposing winds.
    #[cfg(test)]
    pub fn pin_opposed(&mut self, vx: f32, split: usize) {
        for j in 0..FH {
            for i in 0..FW {
                self.vx[j * FW + i] = if j < split { vx } else { -vx } / CELL as f32;
                self.vy[j * FW + i] = 0.0;
            }
        }
        self.frozen = true;
    }

    #[cfg(test)]
    pub fn inject_vortex_for_test(&mut self, x: f32, y: f32, r: f32, gamma: f32) {
        self.swirl(Vortex { x, y, r, gamma, life: 1 }, 1.0);
    }
}

impl World {
    /// Keeps the air's flow up to date: ground and heat are re-read now and then, the flow
    /// itself is advanced every other step.
    pub(super) fn update_flow(&mut self) {
        let t = self.ticks;
        if t % 16 == 1 {
            self.flow.refresh_solids(&self.cells, |c| c.m.is_hard() && !c.m.is_vegetation());
        }
        if t % 4 == 1 {
            let ambient: Vec<f32> = (0..H as i32).map(|y| self.ambient_c(y)).collect();
            self.flow.refresh_heat(&self.temp, &ambient);
        }
        if t % 2 == 0 {
            let wind = self.wind;
            self.flow.step(wind, t);
        }
    }

    /// Where the air carries a gas cell this step: along the local flow, a cell or two at most.
    /// The fractional part of the speed is decided by chance, so slow air still moves gas now and then.
    pub(super) fn wind_target(&mut self, x: i32, y: i32) -> Option<(i32, i32)> {
        let (vx, vy) = self.flow.sample(x, y);
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
            let (nx, ny) = ((x + tx).rem_euclid(W as i32), y + ty);
            if ny >= 0 && ny < H as i32 && self.at(nx, ny) == super::Mat::Empty {
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
        for j in 0..FH {
            for i in 0..FW {
                let (x, y) = (i as f32 * 0.21, j as f32 * 0.17);
                f.vx[j * FW + i] = 0.05 * (x.sin() + (y * 1.3).cos());
                f.vy[j * FW + i] = 0.05 * ((x * 0.8).cos() + y.sin());
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
    fn flow_goes_around_solid_ground() {
        let mut f = Flow::new();
        // A block of ground in a steady wind: no air inside it, and the stream is bent around it.
        for j in 30..40 {
            for i in 50..58 {
                f.set_solid(i, j, true);
            }
        }
        for k in 0..FW * FH {
            f.vx[k] = 0.1;
        }
        for _ in 0..80 {
            for k in 0..FW * FH {
                if !f.solid[k] {
                    f.vx[k] += (0.1 - f.vx[k]) * 0.05;
                }
            }
            f.advect();
            f.project(30);
            for k in 0..FW * FH {
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
    fn warm_air_rises_above_a_fire() {
        let mut f = Flow::new();
        for j in 50..60 {
            for i in 0..FW {
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
        for j in 0..FH {
            let v: f32 = (0..FW).map(|i| f.at_cell(i, j).0).sum::<f32>() / FW as f32;
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
        let worst = (0..FW * FH).map(|k| f.vx[k].abs().max(f.vy[k].abs())).fold(0.0f32, f32::max);
        assert!(worst.is_finite() && worst <= 0.6 + 1e-3, "speed stays bounded: {worst}");
    }

    #[test]
    fn sampling_is_smooth_and_wraps_sideways() {
        let mut f = Flow::new();
        f.set_cell(0, 10, 0.05, 0.0);
        let a = f.sample(0, 85).0;
        let b = f.sample(W as i32 - 1, 85).0;
        assert!(a > 0.0 && b >= 0.0, "the wrap-around neighbour feels the left edge: {a} {b}");
    }
}
