//! Closed pockets of gas, and the pressure inside liquids.
//!
//! Every few steps the gas cells (air, steam, smoke, clouds, flames) are grouped into connected
//! pockets. A pocket that reaches the sky, the sides or the bottom of the world, or that is very
//! large, is the open atmosphere: 1 atm. A closed one follows the ideal gas law, P V = n R T: its
//! pressure grows with the amount of gas in it and with its absolute temperature. Water that boils
//! makes far more gas than it took room as a liquid, so each cell of steam counts `STEAM_N` times.
//!
//! What that gives:
//! - a sealed boiler builds pressure, and its weakest wall gives way when the pressure is more
//!   than it can hold (glass first, steel last);
//! - water boils at a higher temperature under pressure (a pressure cooker), and lower in a
//!   partial vacuum;
//! - liquids carry the pressure (Pascal) and their own weight: a hole low in a dam or in a boiler
//!   shoots a jet, faster the deeper it is (Torricelli), and the pressure of a boiler lifts its
//!   water up a pipe (`level_water` balances heads, not just heights);
//! - a steam motor (see `mechgame.rs`) takes its power from the pocket it sits in.

use super::*;

/// A pocket of gas this large is the open atmosphere, even when it is closed all round.
pub const OPEN_POCKET: u32 = 40_000;
/// How many times more gas a cell of steam holds than a cell of air (boiling water swells a lot).
pub const STEAM_N: f32 = 8.0;
/// Pockets and liquid pressure are re-read every this many steps.
pub const POCKETS_EVERY: u32 = 16;
/// Atmospheres per cell of water depth (the same scale as the pressure view).
pub const ATM_PER_CELL: f32 = 1.0 / 60.0;
/// A liquid shoots out sideways once the pressure behind a hole is worth this many cells of head.
const JET_HEAD: f32 = 3.0;
/// Air temperature (K) at which a closed pocket of plain air is at 1 atm.
const T0: f32 = 288.15;

#[derive(Clone, Copy, Default)]
pub struct Pocket {
    /// Gas cells in it (the volume).
    pub cells: u32,
    /// Steam and cloud cells among them.
    pub steam: u32,
    /// Smoke cells among them: air whose oxygen a fire has burnt.
    pub smoke: u32,
    /// Sum of the absolute temperatures of its cells.
    pub kelvin: f32,
    pub open: bool,
    /// Absolute pressure in atmospheres.
    pub pressure: f32,
    /// The wall cell that would give way first, and how much excess pressure it holds.
    weakest: Option<(usize, f32)>,
    /// Cell sides it shares with walls and with liquid.
    wall_edge: u32,
    liquid_edge: u32,
    /// Walls full of cracks (embers, charcoal, ash) round it: the gas seeps out and never
    /// builds up pressure, but no fresh air comes in either.
    leaky: bool,
}

impl Pocket {
    fn settle(&mut self) {
        self.pressure = if self.open || self.leaky || self.cells == 0 {
            1.0
        } else {
            let v = self.cells as f32;
            let moles = (v - self.steam as f32) + STEAM_N * self.steam as f32;
            moles / v * (self.kelvin / v) / T0
        };
    }
}

/// How much excess pressure (atm) a wall of this material holds before it gives way.
/// `None`: it never does (or it is not a wall).
pub fn wall_strength(m: Mat) -> Option<f32> {
    Some(match m {
        Mat::Ice => 0.6,
        Mat::Glass => 0.8,
        Mat::Snow | Mat::Hail | Mat::Ash | Mat::Charcoal | Mat::Sand | Mat::Mud | Mat::Seed | Mat::Shard | Mat::Rust | Mat::Gunpowder | Mat::Saltpeter | Mat::DeadLeaf => 0.6,
        Mat::Dirt | Mat::Grass => 2.0,
        Mat::Clay => 1.5,
        Mat::Wood | Mat::Trunk => 2.5,
        Mat::Brick => 2.0,
        Mat::Terracotta => 3.0,
        Mat::Stone => 5.0,
        Mat::Iron => 8.0,
        Mat::Steel => 14.0,
        _ => return None,
    })
}

/// Boiling point of water (°C) under an absolute pressure in atmospheres (Clausius-Clapeyron):
/// 100 °C at 1 atm, about 120 °C at 2 atm, 144 °C at 4 atm.
pub fn boil_c(atm: f32) -> f32 {
    let p = atm.clamp(0.05, 60.0);
    1.0 / (1.0 / 373.15 - p.ln() / 4890.0) - 273.15
}

/// The gas test of the pockets: air, gases and flames.
#[inline]
fn gassy(m: Mat) -> bool {
    matches!(m.kind(), Kind::Empty | Kind::Gas | Kind::Flame)
}

impl World {
    /// Groups the gas into pockets and works out the pressure of every pocket and every liquid
    /// cell; then lets the weakest wall of an over-pressured pocket give way.
    pub(super) fn update_pockets(&mut self) {
        // What each cell is to the pockets: 0 a wall, 1 gas, 2 foliage (air goes through it, but
        // it is not volume), 3 liquid (neither: it is pushed, see `update_liquid_pressure`); and
        // the weight of each liquid cell (atm).
        for i in 0..W * H {
            let m = self.cells[i].m;
            self.cls[i] = if self.mask[i] != 0 {
                0
            } else if gassy(m) {
                1
            } else if m.is_vegetation() {
                2
            } else if m.kind() == Kind::Liquid {
                3
            } else {
                0
            };
            self.liq_w[i] = if self.cls[i] == 3 { density(m) * ATM_PER_CELL } else { 0.0 };
        }
        // Label the connected gas a row span at a time, adding up each pocket's volume, steam and
        // warmth on the way; the edges of the world are the open air.
        let mut stack = std::mem::take(&mut self.stack);
        self.pocket_of.fill(0);
        self.pockets.clear();
        let through = |c: u8| c == 1 || c == 2;
        for start in 0..W * H {
            if self.pocket_of[start] != 0 || !through(self.cls[start]) {
                continue;
            }
            let id = self.pockets.len() as u32 + 1;
            let mut p = Pocket::default();
            stack.push(start);
            while let Some(i) = stack.pop() {
                if self.pocket_of[i] != 0 {
                    continue;
                }
                let row = i - i % W;
                let (mut a, mut b) = (i, i);
                while a > row && self.pocket_of[a - 1] == 0 && through(self.cls[a - 1]) {
                    a -= 1;
                }
                while b + 1 < row + W && self.pocket_of[b + 1] == 0 && through(self.cls[b + 1]) {
                    b += 1;
                }
                if a == row || b == row + W - 1 || row == 0 || row == (H - 1) * W {
                    p.open = true;
                }
                for k in a..=b {
                    self.pocket_of[k] = id;
                    if self.cls[k] == 1 {
                        p.cells += 1;
                        p.kelvin += self.temp[k] + 273.15;
                        p.steam += matches!(self.cells[k].m, Mat::Steam | Mat::Cloud) as u32;
                        p.smoke += (self.cells[k].m == Mat::Smoke) as u32;
                    }
                }
                // One seed per run of free cells in the rows above and below.
                for other in [row.wrapping_sub(W), row + W] {
                    if other >= W * H {
                        continue;
                    }
                    let mut open_run = false;
                    for k in a..=b {
                        let j = other + (k - row);
                        let free = self.pocket_of[j] == 0 && through(self.cls[j]);
                        if free && !open_run {
                            stack.push(j);
                        }
                        open_run = free;
                    }
                }
            }
            self.pockets.push(p);
        }
        self.stack = stack;
        // What each pocket is held in: how much wall and how much liquid round it, and its
        // weakest wall.
        for i in 0..W * H {
            let c = self.cls[i];
            if c != 0 && c != 3 {
                continue;
            }
            let strength = if c == 0 && self.mask[i] == 0 { wall_strength(self.cells[i].m) } else { None };
            let x = i % W;
            for j in [i.wrapping_sub(W), if x + 1 < W { i + 1 } else { usize::MAX }, i + W, if x > 0 { i - 1 } else { usize::MAX }] {
                if j >= W * H {
                    continue;
                }
                let id = self.pocket_of[j];
                if id == 0 {
                    continue;
                }
                let p = &mut self.pockets[id as usize - 1];
                if c == 3 {
                    p.liquid_edge += 1;
                } else {
                    p.wall_edge += 1;
                    p.leaky |= matches!(self.cells[i].m, Mat::Ember | Mat::Charcoal | Mat::Ash);
                }
                if let Some(s) = strength {
                    if p.weakest.map_or(true, |w| s < w.1) {
                        p.weakest = Some((i, s));
                    }
                }
            }
        }
        for p in &mut self.pockets {
            // A bubble (gas held mostly by liquid) just pushes the liquid aside and rises: it is
            // at the pressure of the open air, not shut in.
            let bubble = p.liquid_edge * 10 > (p.liquid_edge + p.wall_edge) * 6;
            p.open |= p.cells >= OPEN_POCKET || bubble;
            p.settle();
        }
        self.update_liquid_pressure();
        self.burst_pockets();
    }

    /// The pocket (index into `pockets`) a cell of gas belongs to.
    pub fn pocket_at(&self, x: i32, y: i32) -> Option<usize> {
        if !Self::inb(x, y) {
            return None;
        }
        let i = Self::idx(x, y);
        match self.pocket_of[i] {
            0 => None,
            id if gassy(self.cells[i].m) && self.mask[i] == 0 => Some(id as usize - 1),
            _ => None,
        }
    }

    /// Whether the gas at a cell is shut in a closed pocket (cut off from the open air).
    pub fn sealed(&self, x: i32, y: i32) -> bool {
        self.pocket_at(x, y).and_then(|k| self.pockets.get(k)).is_some_and(|p| !p.open)
    }

    /// For each pocket (by its number in `pocket_of`, 0 being none), the mean temperature (°C) of
    /// its gas if it is shut in for good (no gap, no cracks), NaN otherwise. Shut-in warmth cannot
    /// mix with the open sky: it only stirs round its pocket and passes through the walls.
    pub(super) fn shut_in_means(&self) -> Vec<f32> {
        std::iter::once(f32::NAN)
            .chain(self.pockets.iter().map(|p| if !p.open && !p.leaky && p.cells > 0 { p.kelvin / p.cells as f32 - 273.15 } else { f32::NAN }))
            .collect()
    }

    /// Whether the gas at a cell is shut in with too little oxygen left for a flame: a closed
    /// pocket that is half smoke or more.
    pub fn stale(&self, x: i32, y: i32) -> bool {
        self.pocket_at(x, y).and_then(|k| self.pockets.get(k)).is_some_and(|p| !p.open && p.smoke * 2 >= p.cells)
    }

    /// Absolute pressure (atm) of the gas at a cell: 1 in the open air.
    pub fn gas_pressure(&self, x: i32, y: i32) -> f32 {
        self.pocket_at(x, y).and_then(|k| self.pockets.get(k)).map_or(1.0, |p| p.pressure)
    }

    /// Absolute pressure (atm) inside the liquid at a cell (1 elsewhere).
    pub fn liquid_pressure(&self, x: i32, y: i32) -> f32 {
        if Self::inb(x, y) { self.liq_p[Self::idx(x, y)] } else { 1.0 }
    }

    /// Pressure of each liquid cell: the gas pressing on its surface plus the weight of the
    /// liquid above, shared through the whole connected liquid (a deep arm passes its pressure
    /// to a shallow one, as in a U-tube).
    fn update_liquid_pressure(&mut self) {
        // Which rows hold liquid.
        // (`liq_w` and `cls` are filled by `update_pockets`.)
        let rows: Vec<bool> = (0..H).map(|y| self.cls[y * W..(y + 1) * W].contains(&3)).collect();
        // Down each column from the gas above (row by row, all columns at once). NaN: no gas
        // above, only walls.
        let mut above = vec![f32::NAN; W];
        for y in 0..H {
            for x in 0..W {
                let i = y * W + x;
                let wt = self.liq_w[i];
                if wt > 0.0 {
                    let p = if above[x].is_nan() { 0.0 } else { above[x] + wt };
                    self.liq_p[i] = p;
                    if !above[x].is_nan() {
                        above[x] = p;
                    }
                } else {
                    self.liq_p[i] = 1.0;
                    above[x] = match (self.cls[i], self.pocket_of[i]) {
                        (1, id) if id != 0 => self.pockets[id as usize - 1].pressure,
                        _ => f32::NAN,
                    };
                }
            }
        }
        // Then shared: sideways at the same depth, down and up with the weight of each cell.
        let (p, wt) = (&mut self.liq_p, &self.liq_w);
        for _ in 0..2 {
            for y in (0..H).filter(|&y| rows[y]) {
                let row = y * W;
                for i in row + 1..row + W {
                    if wt[i] > 0.0 && wt[i - 1] > 0.0 && p[i - 1] > p[i] {
                        p[i] = p[i - 1];
                    }
                }
                for i in (row..row + W - 1).rev() {
                    if wt[i] > 0.0 && wt[i + 1] > 0.0 && p[i + 1] > p[i] {
                        p[i] = p[i + 1];
                    }
                }
            }
            for y in (1..H).filter(|&y| rows[y] && rows[y - 1]) {
                for i in y * W..(y + 1) * W {
                    if wt[i] > 0.0 && wt[i - W] > 0.0 {
                        p[i] = p[i].max(p[i - W] + wt[i]);
                    }
                }
            }
            for y in (0..H - 1).rev().filter(|&y| rows[y] && rows[y + 1]) {
                for i in y * W..(y + 1) * W {
                    if wt[i] > 0.0 && wt[i + W] > 0.0 {
                        p[i] = p[i].max(p[i + W] - wt[i + W]);
                    }
                }
            }
        }
        // A liquid sealed off from any gas: as if at 1 atm (it has nowhere to go anyway).
        for i in 0..W * H {
            if p[i] < 1.0 && wt[i] > 0.0 {
                p[i] = 1.0;
            }
        }
    }

    /// The weakest wall of a pocket under more pressure than it can hold gives way: with a bang
    /// when the pocket is big, with a crack when it is small.
    fn burst_pockets(&mut self) {
        for k in 0..self.pockets.len() {
            let p = self.pockets[k];
            if p.open || p.cells < 4 {
                continue;
            }
            let Some((wall, strength)) = p.weakest else { continue };
            let excess = p.pressure - 1.0;
            if excess <= strength {
                continue;
            }
            let (x, y) = ((wall % W) as i32, (wall / W) as i32);
            self.burst(x, y, excess, p.cells);
            self.bursts.push((x, y, p.pressure));
            // Opened: the pocket is part of the open air until the next count.
            self.pockets[k].open = true;
            self.pockets[k].settle();
        }
    }

    /// A wall giving way at (x, y) under `excess` atmospheres of a pocket of `volume` cells.
    fn burst(&mut self, cx: i32, cy: i32, excess: f32, volume: u32) {
        self.revision += 1;
        let energy = excess * volume as f32;
        let r = (energy.sqrt() * 0.5).clamp(1.0, 18.0) as i32;
        // The wall breaks round the weak point: whatever is not much stronger than the push.
        let hole = (r / 2).max(1);
        for dy in -hole..=hole {
            for dx in -hole..=hole {
                if dx * dx + dy * dy > hole * hole {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                if !self.mask_free(x, y) {
                    continue;
                }
                let m = self.raw(x, y);
                if wall_strength(m).map_or(true, |s| s > excess * 1.5) {
                    continue;
                }
                let into = match m {
                    Mat::Glass => Mat::Shard,
                    Mat::Stone => Mat::Sand,
                    Mat::Ice => Mat::Snow,
                    Mat::Wood | Mat::Trunk => Mat::Empty,
                    Mat::Dirt | Mat::Grass | Mat::Clay | Mat::Brick | Mat::Terracotta => Mat::Mud,
                    _ => Mat::Empty,
                };
                let into = if self.chance(2) { into } else { Mat::Empty };
                self.set(x, y, into);
            }
        }
        self.flow.blast(cx, cy, r * 2, (0.1 + excess * 0.1).min(1.5));
        if r >= 3 {
            if self.blasts.len() < 8 {
                self.blasts.push((cx, cy, r));
            }
            self.shove_out(cx, cy, r);
        } else if self.thuds.len() < 8 {
            self.thuds.push((cx, cy, 1.5));
        }
    }

    /// A liquid next to an opening with lower pressure shoots through it sideways, faster the
    /// more pressure there is behind it: v = sqrt(2 ΔP / ρ) (Torricelli). Returns whether it moved.
    pub(super) fn jet(&mut self, x: i32, y: i32) -> bool {
        let i = Self::idx(x, y);
        let p = self.liq_p[i];
        if p < 1.0 + JET_HEAD * ATM_PER_CELL {
            return false;
        }
        let m = self.cells[i].m;
        // Lava and molten metal are far too thick to spurt.
        if matches!(m, Mat::Lava | Mat::Molten | Mat::MoltenSteel) {
            return false;
        }
        let rho = density(m).max(0.3);
        // A hole in a wall (something above the opening), not the free edge of a pool; and enough
        // head (in cells of this liquid, less its own height) pushing through it.
        let head = |w: &World, dir: i32| -> Option<f32> {
            let (nx, ny) = (x + dir, y);
            if w.at(nx, ny) != Mat::Empty || matches!(w.at(nx, ny - 1).kind(), Kind::Empty | Kind::Gas | Kind::Flame) {
                return None;
            }
            // Only into gas that was already gas at the last count: a fresh hole (a bubble, a cell a
            // pipe just took) has a pressure nobody knows yet.
            let outside = w.pockets[w.pocket_at(nx, ny)?].pressure;
            Some((p - outside) / (rho * ATM_PER_CELL) - 1.0).filter(|&h| h >= JET_HEAD)
        };
        let (right, left) = (head(self, 1), head(self, -1));
        let (dir, head) = match (right, left) {
            (Some(r), Some(l)) => if self.chance(2) { (1, r) } else { (-1, l) },
            (Some(r), None) => (1, r),
            (None, Some(l)) => (-1, l),
            (None, None) => return false,
        };
        // Speed in cells per step: the drop keeps it while it falls (see `fly`), so it lands
        // further from a deep hole than from a shallow one.
        let v = (0.7 * head.sqrt()).round().clamp(1.0, 10.0) as i32;
        let mut k = 1;
        while k < v && self.at(x + dir * (k + 1), y) == Mat::Empty {
            k += 1;
        }
        let to = Self::idx(x + dir * k, y);
        self.swap((x, y), (x + dir * k, y));
        self.cells[to].life = (v as u8) << 1 | (dir > 0) as u8;
        true
    }

    /// A drop shot out of a jet keeps its sideways speed while it falls: it flies in a slanting
    /// line until it lands. Returns whether it moved.
    pub(super) fn fly(&mut self, x: i32, y: i32) -> bool {
        let i = Self::idx(x, y);
        let l = self.cells[i].life;
        if l < 2 {
            return false;
        }
        if self.at(x, y + 1) != Mat::Empty {
            // Landed: it is ordinary liquid again.
            self.cells[i].life = 0;
            return false;
        }
        let (v, dir) = ((l >> 1) as i32, if l & 1 == 1 { 1 } else { -1 });
        let mut k = 0;
        while k < v && self.at(x + dir * (k + 1), y + 1) == Mat::Empty {
            k += 1;
        }
        let to = (x + dir * k, y + 1);
        self.swap((x, y), to);
        if !Self::inb(to.0, to.1) {
            // Gone out of the world.
            return true;
        }
        // Air drag slows it now and then; blocked sideways, it just drops.
        let v = if k == 0 { 0 } else if self.chance(6) { v - 1 } else { v };
        let j = Self::idx(to.0, to.1);
        self.cells[j].life = if v > 0 { (v as u8) << 1 | (dir > 0) as u8 } else { 0 };
        true
    }

    /// For a steam motor at `pos`: the pressure of the closed pocket it sits in or touches (the
    /// highest one within `reach`), and that pocket.
    pub fn steam_intake(&self, pos: (f32, f32), reach: f32) -> Option<(usize, f32)> {
        let r = reach.ceil() as i32;
        let (cx, cy) = (pos.0.round() as i32, pos.1.round() as i32);
        let mut best: Option<(usize, f32)> = None;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                if let Some(k) = self.pocket_at(cx + dx, cy + dy) {
                    let p = self.pockets[k];
                    if !p.open && best.map_or(true, |b| p.pressure > b.1) {
                        best = Some((k, p.pressure));
                    }
                }
            }
        }
        best
    }

    /// A steam motor uses up a cell of the steam of pocket `k`, the one nearest `pos`, and lets it
    /// out of its chimney at `exhaust`. Returns false when there is no steam left to take.
    pub fn draw_steam(&mut self, k: usize, pos: (f32, f32), exhaust: (i32, i32)) -> bool {
        let (cx, cy) = (pos.0.round() as i32, pos.1.round() as i32);
        let id = k as u32 + 1;
        for r in 1..48 {
            for dy in -r..=r {
                for dx in [-r, r] {
                    for (x, y) in [(cx + dx, cy + dy), (cx + dy, cy + dx)] {
                        if Self::inb(x, y) && self.pocket_of[Self::idx(x, y)] == id && matches!(self.raw(x, y), Mat::Steam | Mat::Cloud) {
                            self.set(x, y, Mat::Empty);
                            if let Some(p) = self.pockets.get_mut(k) {
                                p.steam = p.steam.saturating_sub(1);
                                p.settle();
                            }
                            // Out of the chimney, unless the chimney opens back into the same boiler.
                            let outside = Self::inb(exhaust.0, exhaust.1) && self.pocket_of[Self::idx(exhaust.0, exhaust.1)] != id;
                            if outside && self.at(exhaust.0, exhaust.1) == Mat::Empty {
                                let t = self.temp[Self::idx(x, y)];
                                self.set(exhaust.0, exhaust.1, Mat::Steam);
                                self.temp[Self::idx(exhaust.0, exhaust.1)] = t;
                            }
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box of `wall` from (x0, y0) to (x1, y1) inclusive, hollow inside.
    fn boxed(w: &mut World, x0: i32, y0: i32, x1: i32, y1: i32, wall: Mat) {
        for x in x0..=x1 {
            for y in y0..=y1 {
                if x == x0 || x == x1 || y == y0 || y == y1 {
                    w.set(x, y, wall);
                }
            }
        }
    }

    fn fill(w: &mut World, x0: i32, y0: i32, x1: i32, y1: i32, m: Mat, t: f32) {
        for x in x0..=x1 {
            for y in y0..=y1 {
                w.set(x, y, m);
                w.temp[y as usize * W + x as usize] = t;
            }
        }
    }

    #[test]
    fn boiling_point_rises_with_pressure() {
        assert!((boil_c(1.0) - 100.0).abs() < 0.5);
        assert!((boil_c(2.0) - 120.0).abs() < 3.0, "{}", boil_c(2.0));
        assert!(boil_c(0.5) < 85.0);
    }

    #[test]
    fn open_air_is_one_atmosphere_and_a_closed_box_of_air_too() {
        let mut w = World::new_static();
        boxed(&mut w, 100, 100, 140, 130, Mat::Iron);
        w.update_pockets();
        assert!((w.gas_pressure(300, 300) - 1.0).abs() < 1e-6, "open air");
        let p = w.gas_pressure(120, 115);
        assert!((p - 1.0).abs() < 0.05, "a closed box of air at 15 °C: {p}");
        assert!(!w.pockets[w.pocket_at(120, 115).unwrap()].open);
    }

    #[test]
    fn steam_and_heat_build_pressure_in_a_closed_box() {
        let mut w = World::new_static();
        boxed(&mut w, 100, 100, 140, 130, Mat::Steel);
        w.update_pockets();
        let cold = w.gas_pressure(120, 115);
        fill(&mut w, 101, 101, 139, 129, Mat::Empty, 300.0);
        w.update_pockets();
        let hot = w.gas_pressure(120, 115);
        assert!(hot > cold * 1.8, "hot air presses harder: {cold} -> {hot}");
        fill(&mut w, 101, 101, 139, 114, Mat::Steam, 120.0);
        fill(&mut w, 101, 115, 139, 129, Mat::Empty, 120.0);
        w.update_pockets();
        let steam = w.gas_pressure(120, 120);
        assert!(steam > 4.0, "half full of steam: {steam}");
    }

    #[test]
    fn a_glass_boiler_bursts_but_a_steel_one_holds() {
        for (wall, bursts) in [(Mat::Glass, true), (Mat::Steel, false)] {
            let mut w = World::new_static();
            boxed(&mut w, 100, 100, 130, 120, wall);
            fill(&mut w, 101, 101, 129, 110, Mat::Steam, 130.0);
            w.update_pockets();
            let broken = (100..=130).flat_map(|x| (100..=120).map(move |y| (x, y))).filter(|&(x, y)| (x == 100 || x == 130 || y == 100 || y == 120) && w.raw(x, y) != wall).count();
            assert_eq!(broken > 0, bursts, "{wall:?}: {broken} wall cells gone");
            assert_eq!(!w.bursts.is_empty(), bursts);
        }
    }

    #[test]
    fn water_under_a_pressurised_pocket_boils_later() {
        let mut w = World::new_static();
        boxed(&mut w, 100, 100, 130, 130, Mat::Steel);
        fill(&mut w, 101, 101, 129, 112, Mat::Steam, 125.0);
        fill(&mut w, 101, 113, 129, 129, Mat::Water, 25.0);
        w.update_pockets();
        let p = w.liquid_pressure(115, 120);
        assert!(p > 3.0, "the water feels the steam above it: {p}");
        assert!(boil_c(p) > 135.0);
        // Open water at the same depth: just its own weight.
        let mut o = World::new_static();
        for x in 100..=130 {
            o.set(x, 131, Mat::Stone);
            o.set(100, x, Mat::Stone);
            o.set(130, x, Mat::Stone);
        }
        fill(&mut o, 101, 113, 129, 130, Mat::Water, 25.0);
        o.update_pockets();
        let q = o.liquid_pressure(115, 120);
        assert!(q > 1.05 && q < 1.3, "open water 8 cells down: {q}");
    }

    #[test]
    fn a_hole_low_in_a_dam_shoots_further_than_a_hole_near_the_top() {
        let reach = |hole_y: i32| {
            let mut w = World::new_static();
            for y in 200..=301 {
                w.set(199, y, Mat::Stone);
                w.set(260, y, Mat::Stone);
            }
            for x in 199..=400 {
                w.set(x, 302, Mat::Stone);
            }
            fill(&mut w, 200, 200, 259, 301, Mat::Water, 15.0);
            w.update_pockets();
            w.set(260, hole_y, Mat::Empty);
            let mut far = 0;
            for _ in 0..30 {
                w.step();
                for x in 261..400 {
                    for y in hole_y..302 {
                        if w.raw(x, y) == Mat::Water {
                            far = far.max(x - 260);
                        }
                    }
                }
            }
            far
        };
        let (low, high) = (reach(295), reach(215));
        assert!(low > high + 3, "deep hole {low} vs shallow hole {high}");
    }

    #[test]
    fn steam_pressure_lifts_water_up_a_pipe() {
        let run = |steam: bool| {
            let mut w = World::new_static();
            // A stone boiler with a pipe on its right side: open at the top, joined to the boiler
            // by a hole at the bottom. Water stands at the same level in both.
            boxed(&mut w, 100, 200, 140, 240, Mat::Stone);
            for y in 150..=240 {
                w.set(142, y, Mat::Stone);
            }
            w.set(141, 240, Mat::Stone);
            fill(&mut w, 101, 225, 139, 239, Mat::Water, 20.0);
            fill(&mut w, 140, 238, 141, 239, Mat::Water, 20.0);
            fill(&mut w, 141, 225, 141, 237, Mat::Water, 20.0);
            if steam {
                fill(&mut w, 101, 201, 139, 205, Mat::Steam, 110.0);
            }
            for _ in 0..120 {
                w.step();
            }
            assert!(w.bursts.is_empty(), "a stone boiler holds a few atmospheres");
            (150..241).find(|&y| w.raw(141, y) == Mat::Water).unwrap_or(241)
        };
        let (with, without) = (run(true), run(false));
        assert!(with < without - 10, "the steam pushed the water up the pipe: top at {with}, {without} without steam");
    }

    #[test]
    fn a_bubble_of_steam_under_water_is_not_a_boiler() {
        let mut w = World::new_static();
        // An open pot of water with a pocket of steam caught at the bottom.
        for x in 200..240 {
            w.set(x, 300, Mat::Stone);
            for y in 270..300 {
                w.set(x, y, Mat::Water);
            }
        }
        for y in 265..301 {
            w.set(199, y, Mat::Stone);
            w.set(240, y, Mat::Stone);
        }
        fill(&mut w, 215, 296, 220, 299, Mat::Steam, 110.0);
        w.update_pockets();
        let p = w.gas_pressure(217, 298);
        assert!(p < 1.01, "a bubble is at the pressure of the open air: {p}");
        assert!(w.bursts.is_empty(), "and it never bursts the pot");
        assert!(w.liquid_pressure(217, 295) < 1.6, "the water above it feels only its own weight: {}", w.liquid_pressure(217, 295));
    }
}
