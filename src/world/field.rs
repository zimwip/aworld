//! Water as a field (`World::water_field`, a prototype, off unless `AWORLD_WATER_FIELD=1`).
//!
//! Water comes in two modes and passes from one to the other:
//! - **Particles**: a cell of water with free air (or gas) under it is a drop. It falls in the
//!   cells' turn, as it always did (gravity, the wind, foliage, clouds it sweeps up).
//! - **A field**: water that rests on something is a quantity per cell, `fill` (`FULL` for a
//!   full cell), and moves here, every step, by what it weighs. A cell gives to the cell under it
//!   until that one is full, then a little more (water is barely compressible: the extra a cell
//!   holds is the weight of the water over it); what it cannot give down runs to the cells beside
//!   it, faster the more it has been running (the flow along each row is kept from pass to pass);
//!   and what it holds beyond its share it pushes up. A cell under water that is short of full is
//!   made up at once from the top of its column (`settle_columns`).
//!
//! Far across a body of water (two vessels joined at the bottom, a siphon, a boiler pushing water
//! up a pipe), the pressure is still passed on by the levelling of `hydro.rs`, which moves the
//! cells of the field with what they hold.
//!
//! A drop that lands becomes part of the field; field water that runs over an edge, with air
//! under the cell it ran into, becomes drops (a waterfall). Thin films do not run onto dry ground
//! (surface tension): a puddle stays a puddle.
//!
//! Every cell works out what it gives from the state at the start of the pass, then all the
//! gifts are made at once: the order of the cells does not matter, nothing is created or lost,
//! and the pass runs on every core with the same result as on one.

use super::*;
use rayon::prelude::*;

/// A full cell of water, in units of `fill`: finer than `WATER_UNITS`, so that the little extra
/// each cell holds for the weight over it is counted exactly enough to add up down a deep tank.
pub const FULL: u32 = 1 << 16;
/// Units a cell holds beyond full for each full cell of water over it.
const SQUEEZE: u32 = FULL / 1024;
/// Below this a cell of water does not spread onto dry ground beside it.
const FILM: u32 = FULL * 3 / 16;
/// Passes of the field per step.
const PASSES: usize = 4;
/// How much of its flow an edge keeps from one pass to the next.
const DRAG: f32 = 0.97;
/// How fast a difference of level speeds the flow up, a pass.
const PULL: f32 = 0.25;

/// Whether new worlds take water as a field: `AWORLD_WATER_FIELD=1`.
pub(super) fn field_by_default() -> bool {
    std::env::var_os("AWORLD_WATER_FIELD").is_some_and(|v| v != "0")
}

/// What the lower of two cells of water one over the other holds at rest, when they hold `total`
/// between them.
#[inline]
fn lower_share(total: u32) -> u32 {
    let (f, s) = (FULL, SQUEEZE);
    if total <= f {
        total
    } else if total < 2 * f + s {
        ((f as u64 * f as u64 + total as u64 * s as u64) / (f + s) as u64) as u32
    } else {
        (total + s) / 2
    }
}

/// No room for water (a wall, another matter): in `row_gifts`.
const NONE: u32 = u32::MAX;

/// What a cell gives this pass: down, left, right, up.
pub(super) type Gifts = [u32; 4];

impl World {
    /// Units of water in cell `i` (a cell of water; `FULL` unless the field says otherwise).
    #[inline]
    pub fn water_fill(&self, i: usize) -> u32 {
        match self.fill[i] {
            0 => FULL,
            f => f,
        }
    }

    /// Turns the field on or off. Off, a cell of water is always full: the cells under half full
    /// go and the others fill up, about as much water as there was.
    pub fn set_water_field(&mut self, on: bool) {
        if !on {
            for i in 0..W * H {
                if self.cells[i].m == Mat::Water && self.fill[i] != 0 {
                    if self.fill[i] < FULL / 2 {
                        self.cells[i] = Cell { m: Mat::Empty, stamp: self.frame };
                        self.clear_states(i);
                    }
                    self.fill[i] = 0;
                    self.zones.wake(i);
                }
            }
            self.flux.fill(0.0);
            self.revision += 1;
        }
        self.water_field = on;
    }

    /// Water in cell `i` in `WATER_UNITS`, what the ground drinks of it.
    #[inline]
    pub fn water_units_in(&self, i: usize) -> u16 {
        (self.water_fill(i) as u64 * WATER_UNITS as u64 / FULL as u64) as u16
    }

    #[inline]
    fn put_fill(&mut self, i: usize, q: u32) {
        self.fill[i] = if q == FULL { 0 } else { q };
    }

    /// Whether the water at (x, y) rests on something (it is part of the field), or is a drop.
    #[inline]
    pub(super) fn water_rests(&self, x: i32, y: i32) -> bool {
        !(matches!(self.at(x, y + 1).kind(), Kind::Empty | Kind::Gas) && self.mask_free(x, y + 1))
    }

    /// Units of water in all the world (cells of water, and vapour as the water it would make).
    #[cfg(test)]
    pub(super) fn water_units(&self) -> u64 {
        (0..W * H)
            .map(|i| match self.cells[i].m {
                Mat::Water => self.water_fill(i) as u64,
                Mat::Steam | Mat::Cloud => self.vapour_units(i) as u64 * FULL as u64 / VAPOUR_PER_WATER as u64,
                _ => 0,
            })
            .sum()
    }

    /// The field's turn: `PASSES` passes.
    pub(super) fn step_water_field(&mut self) {
        let mut gifts = std::mem::take(&mut self.gifts);
        gifts.resize(W * H, [0; 4]);
        for _ in 0..PASSES {
            self.field_pass(&mut gifts);
        }
        self.gifts = gifts;
        self.settle_columns();
    }

    /// Water under water is never short of full: in each column of resting water, what the cells
    /// lack is made up at once from the top of the column (the surface goes down, the water does
    /// not thin out). The passes alone carry a lack up only one cell a pass, slower than a tank
    /// drains from the bottom.
    fn settle_columns(&mut self) {
        let changed: Vec<Vec<(usize, u32)>> = (0..W)
            .into_par_iter()
            .map(|x| {
                let mut out = vec![];
                // Water of the field, in a zone that runs.
                let field = |y: i32| {
                    let i = Self::idx(x as i32, y);
                    self.cells[i].m == Mat::Water && self.mask[i] == 0 && self.zones.awake[y as usize / sleep::CS * sleep::CW + x / sleep::CS]
                };
                let mut y = H as i32 - 1;
                while y >= 0 {
                    if !field(y) || !self.water_rests(x as i32, y) {
                        y -= 1;
                        continue;
                    }
                    // The run of water from (x, y) up.
                    let bottom = y;
                    while y >= 0 && field(y) {
                        y -= 1;
                    }
                    let run = (y + 1..=bottom).rev().map(|y| Self::idx(x as i32, y));
                    if run.clone().all(|i| self.water_fill(i) >= FULL) || bottom == y + 1 {
                        continue;
                    }
                    let mut have: u64 = run.clone().map(|i| self.water_fill(i) as u64).sum();
                    for i in run {
                        let q = (self.water_fill(i).max(FULL) as u64).min(have) as u32;
                        have -= q as u64;
                        if q != self.water_fill(i) {
                            out.push((i, q));
                        }
                    }
                }
                out
            })
            .collect();
        for (i, q) in changed.into_iter().flatten() {
            if q == 0 {
                self.cells[i] = Cell { m: Mat::Empty, stamp: self.frame };
                self.clear_states(i);
                self.vel[i] = 0;
            } else {
                self.put_fill(i, q);
            }
            self.revision += 1;
            self.zones.wake(i);
        }
    }

    fn field_pass(&mut self, gifts: &mut [Gifts]) {
        // 1. What every cell of resting water gives, from the field as it is. A row is worked out
        //    on its own (the flow along it is kept from pass to pass, see `row_gifts`); a row
        //    with no water gives nothing.
        let mut gave = vec![false; H];
        {
            let mut flux = std::mem::take(&mut self.flux);
            flux.resize(W * H, 0.0);
            let me = &*self;
            gifts
                .par_chunks_mut(W)
                .zip(flux.par_chunks_mut(W))
                .zip(gave.par_iter_mut())
                .enumerate()
                .for_each(|(y, ((row, flux), gave))| *gave = me.row_gifts(y as i32, row, flux));
            self.flux = flux;
        }
        // 2. Every cell takes what it is given and keeps what it did not give: only in the rows
        //    that gave, and the rows over and under them.
        let gifts = &*gifts;
        let gave = &gave;
        let changed: Vec<Vec<(usize, u32)>> = self
            .cells
            .par_chunks(W)
            .zip(self.fill.par_chunks(W))
            .enumerate()
            .map(|(y, (cells, fill))| {
                let mut out = vec![];
                if !(gave[y] || (y > 0 && gave[y - 1]) || (y + 1 < H && gave[y + 1])) {
                    return out;
                }
                for x in 0..W {
                    let i = y * W + x;
                    let given: u32 = if gave[y] { gifts[i].iter().map(|&g| g as u32).sum() } else { 0 };
                    let mut got = 0u32;
                    if y > 0 && gave[y - 1] {
                        got += gifts[i - W][0] as u32;
                    }
                    if gave[y] {
                        if x > 0 {
                            got += gifts[i - 1][2] as u32;
                        }
                        if x + 1 < W {
                            got += gifts[i + 1][1] as u32;
                        }
                    }
                    if y + 1 < H && gave[y + 1] {
                        got += gifts[i + W][3] as u32;
                    }
                    if given == 0 && got == 0 {
                        continue;
                    }
                    let had = match cells[x].m {
                        Mat::Water => match fill[x] {
                            0 => FULL as u32,
                            f => f as u32,
                        },
                        _ => 0,
                    };
                    out.push((i, had - given + got));
                }
                out
            })
            .collect();
        for (i, q) in changed.into_iter().flatten() {
            let was_water = self.cells[i].m == Mat::Water;
            if q == 0 {
                if was_water {
                    self.cells[i] = Cell { m: Mat::Empty, stamp: self.frame };
                    self.clear_states(i);
                    self.vel[i] = 0;
                }
            } else {
                if !was_water {
                    // New water takes the warmth of the cell that gave most of it.
                    let (x, y) = (i % W, i / W);
                    let mut best = (0u32, self.temp[i]);
                    for (j, k) in [(y.checked_sub(1).map(|y| y * W + x), 0), (x.checked_sub(1).map(|x| y * W + x), 2), ((x + 1 < W).then(|| y * W + x + 1), 1), ((y + 1 < H).then(|| (y + 1) * W + x), 3)] {
                        if let Some(j) = j {
                            if gifts[j][k] > best.0 {
                                best = (gifts[j][k], self.temp[j]);
                            }
                        }
                    }
                    self.cells[i] = Cell { m: Mat::Water, stamp: self.frame };
                    self.clear_states(i);
                    self.latent[i] = 0.0;
                    self.vel[i] = 0;
                    self.temp[i] = best.1;
                }
                self.put_fill(i, q);
            }
            self.revision += 1;
            self.zones.wake(i);
        }
    }

    /// What the cells of row `y` give this pass. `flux[x]` is the flow from (x, y) to (x + 1, y),
    /// in units a pass, kept from the last pass: water that runs goes on running (that is what
    /// lets a wave cross a pool in a few steps, where sharing out alone would take thousands).
    fn row_gifts(&self, y: i32, gifts: &mut [Gifts], flux: &mut [f32]) -> bool {
        let row = y as usize * W;
        // (Water in a sleeping zone is at rest: it gives nothing, though it may be given to.)
        let zones = &self.zones.awake[y as usize / sleep::CS * sleep::CW..][..sleep::CW];
        if !zones.contains(&true) || !self.cells[row..row + W].iter().any(|c| c.m == Mat::Water) {
            // (What it gave last pass is left in `gifts`, but no row reads it.)
            flux.fill(0.0);
            return false;
        }
        // Water, or room for it, in this row and the ones under and over it (`NONE`: no room).
        let room_row = |y: i32| {
            let mut r = [NONE; W];
            if (0..H as i32).contains(&y) {
                let row = y as usize * W;
                for (x, r) in r.iter_mut().enumerate() {
                    let i = row + x;
                    if self.mask[i] == 0 {
                        *r = match self.cells[i].m {
                            Mat::Water => self.water_fill(i),
                            Mat::Empty => 0,
                            _ => NONE,
                        };
                    }
                }
            }
            r
        };
        let (here, under) = (room_row(y), room_row(y + 1));
        // First down: what each cell at rest gives the water under it, and what it has left.
        let mut left = [0u32; W];
        let mut rests = [false; W];
        let mut any = false;
        for x in 0..W {
            let i = row + x;
            gifts[x] = [0; 4];
            left[x] = if here[x] == NONE { 0 } else { here[x] };
            if self.cells[i].m != Mat::Water || self.mask[i] != 0 || !zones[x / sleep::CS] || !self.water_rests(x as i32, y) {
                continue;
            }
            let q = left[x];
            rests[x] = true;
            let b = under[x];
            if b != NONE {
                // Room under it is filled at once. Beyond full, half the way there: the cell under
                // it is evening out with the one under it at the same time, and the whole way would
                // overshoot (a column under pressure would ring).
                let room = FULL.saturating_sub(b).min(q);
                let squeeze = lower_share(q + b).saturating_sub(b.max(FULL)).min(q - room);
                let down = room + squeeze.div_ceil(2);
                gifts[x][0] = down;
                left[x] -= down;
                any |= down > 0;
            }
        }
        // Then along the row: the flow on each edge speeds up with the difference of level, and
        // slows down by itself.
        for x in 0..W - 1 {
            let (a, b) = (here[x], here[x + 1]);
            let mut f = 0.0;
            if a != NONE && b != NONE && (rests[x] || rests[x + 1]) {
                let d = left[x] as f32 - left[x + 1] as f32;
                f = flux[x] * DRAG + if d.abs() >= 2.0 { PULL * d } else { 0.0 };
                // Only water at rest gives; a film does not wet dry ground.
                let gives = |from: usize, to: u32| rests[from] && !(to == 0 && left[from] < FILM);
                if (f > 0.0 && !gives(x, b)) || (f < 0.0 && !gives(x + 1, a)) {
                    f = 0.0;
                }
            }
            flux[x] = f;
        }
        flux[W - 1] = 0.0;
        // No cell gives more than it has left: what flows out of it is cut down to that.
        for x in 0..W {
            let out_r = flux[x].max(0.0);
            let out_l = if x > 0 { (-flux[x - 1]).max(0.0) } else { 0.0 };
            let out = out_r + out_l;
            let have = left[x] as f32;
            if out > have && out > 0.0 {
                let k = have / out;
                flux[x] = if flux[x] > 0.0 { flux[x] * k } else { flux[x] };
                if x > 0 && flux[x - 1] < 0.0 {
                    flux[x - 1] *= k;
                }
            }
        }
        // (Only a cell holding more than full pushes up.)
        let over = if rests.iter().zip(&left).any(|(&r, &q)| r && q > FULL) { room_row(y - 1) } else { [NONE; W] };
        for x in 0..W {
            let r = flux[x].max(0.0) as u32;
            let l = if x > 0 { (-flux[x - 1]).max(0.0) as u32 } else { 0 };
            let (r, l) = (r.min(left[x]), l.min(left[x] - r.min(left[x])));
            gifts[x][2] = r;
            gifts[x][1] = l;
            any |= r + l > 0;
            if !rests[x] {
                continue;
            }
            let rest = left[x] - r - l;
            // Up: what it holds beyond its share with the cell over it.
            if rest > FULL && over[x] != NONE {
                let up = rest.saturating_sub(lower_share(rest + over[x])).div_ceil(2);
                gifts[x][3] = up;
                any |= up > 0;
            }
        }
        // (A row that gives nothing is left out of the second half of the pass.)
        any
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        let mut w = World::new_static();
        w.water_field = true;
        w
    }

    fn rect(w: &mut World, x0: i32, y0: i32, x1: i32, y1: i32, m: Mat) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                w.set(x, y, m);
            }
        }
    }

    #[test]
    fn the_share_of_the_lower_cell_is_full_and_a_little_more() {
        assert_eq!(lower_share(10), 10);
        assert_eq!(lower_share(FULL), FULL);
        assert!((FULL + 1..=FULL + SQUEEZE).contains(&lower_share(2 * FULL)));
        assert_eq!(lower_share(2 * FULL + SQUEEZE), FULL + SQUEEZE);
        // Never less than half.
        for t in 0..1000 {
            assert!(2 * lower_share(t) >= t, "{t}");
        }
    }

    #[test]
    fn a_heap_of_water_spreads_into_a_flat_pool_and_none_is_lost() {
        let mut w = world();
        rect(&mut w, 100, 401, 300, 401, Mat::Stone);
        rect(&mut w, 99, 380, 99, 401, Mat::Stone);
        rect(&mut w, 301, 380, 301, 401, Mat::Stone);
        rect(&mut w, 190, 340, 209, 400, Mat::Water);
        w.temp.fill(1.0);
        let before = w.water_units();
        for _ in 0..1500 {
            w.step();
        }
        let after = w.water_units();
        // (No water leaves the walls; the cold keeps it from evaporating much.)
        assert!(before.abs_diff(after) * 100 < before, "{before} units became {after}");
        let depth = |x: i32| (380..401).filter(|&y| w.raw(x, y) == Mat::Water).count();
        let (edge, middle) = (depth(105), depth(200));
        assert!(edge > 0 && middle <= edge + 1, "not level: {} at the side, {} in the middle", edge, middle);
    }

    #[test]
    fn water_in_the_field_comes_out_the_same_on_one_core_and_on_four() {
        let run = |cores: usize| {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(cores).build().unwrap();
            pool.install(|| {
                let mut w = world();
                rect(&mut w, 100, 401, 300, 401, Mat::Stone);
                rect(&mut w, 150, 300, 170, 380, Mat::Water);
                for _ in 0..200 {
                    w.step();
                }
                (0..W * H).map(|i| (w.cells[i].m, w.fill[i])).collect::<Vec<_>>()
            })
        };
        assert!(run(1) == run(4));
    }
}

#[cfg(test)]
mod probe {
    use super::*;

    fn rect(w: &mut World, x0: i32, y0: i32, x1: i32, y1: i32, m: Mat) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                w.set(x, y, m);
            }
        }
    }

    fn dump(w: &World, x0: i32, y0: i32, x1: i32, y1: i32) -> String {
        let mut s = String::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = World::idx(x, y);
                s.push(match w.raw(x, y) {
                    Mat::Empty => ' ',
                    Mat::Water if !w.water_field => '~',
                    Mat::Water => match w.water_fill(i) * 64 / FULL {
                        0..=15 => '.',
                        16..=40 => ':',
                        41..=63 => '~',
                        64..=69 => 'w',
                        _ => 'W',
                    },
                    Mat::Stone | Mat::Brick => '#',
                    Mat::Steam => 's',
                    _ => '?',
                });
            }
            s.push('\n');
        }
        s
    }

    #[test]
    #[ignore]
    fn probe_heap() {
        for field in [false, true] {
            let mut w = World::new_static();
            w.water_field = field;
            rect(&mut w, 100, 401, 300, 401, Mat::Stone);
            rect(&mut w, 99, 370, 99, 401, Mat::Stone);
            rect(&mut w, 301, 370, 301, 401, Mat::Stone);
            rect(&mut w, 190, 350, 209, 400, Mat::Water);
            let t0 = std::time::Instant::now();
            for t in 0..400 {
                w.step();
                if [10, 30, 60, 120, 399].contains(&t) {
                    eprintln!("field {field} t {t}\n{}", dump(&w, 99, 380, 301, 401));
                }
            }
            eprintln!("field {field}: {:?} for 400 steps", t0.elapsed());
        }
    }

    #[test]
    #[ignore]
    fn probe_vessels() {
        for field in [false, true] {
            let mut w = World::new_static();
            w.water_field = field;
            rect(&mut w, 100, 300, 160, 460, Mat::Brick);
            rect(&mut w, 101, 300, 116, 450, Mat::Empty);
            rect(&mut w, 144, 300, 159, 450, Mat::Empty);
            rect(&mut w, 117, 447, 143, 450, Mat::Empty);
            rect(&mut w, 101, 371, 116, 450, Mat::Water);
            let surface = |w: &World, x0: i32, x1: i32| (300..=450).find(|&y| (x0..=x1).filter(|&x| w.raw(x, y) == Mat::Water).count() * 2 > (x1 - x0 + 1) as usize).unwrap_or(451);
            let mut line = format!(" start {}", w.water_units());
            for t in 0..3000 {
                w.step();
                if t % 200 == 199 {
                    line += &format!(" {}:{}/{}", t + 1, surface(&w, 101, 116), surface(&w, 144, 159));
                }
            }
            eprintln!("VESSELS field {field}:{line} units {}", w.water_units());
        }
    }

    #[test]
    #[ignore]
    fn probe_fall() {
        for field in [false, true] {
            let mut w = World::new_static();
            w.water_field = field;
            // A tank on a ledge, open on its right side at the bottom, over a basin.
            rect(&mut w, 100, 300, 160, 300, Mat::Stone);
            rect(&mut w, 99, 260, 99, 300, Mat::Stone);
            rect(&mut w, 140, 260, 140, 294, Mat::Stone);
            rect(&mut w, 100, 262, 139, 299, Mat::Water);
            rect(&mut w, 100, 400, 240, 400, Mat::Stone);
            rect(&mut w, 240, 360, 240, 400, Mat::Stone);
            for t in 0..300 {
                w.step();
                if [20, 60, 299].contains(&t) {
                    eprintln!("FALL field {field} t {t}\n{}", dump(&w, 98, 258, 242, 401));
                }
            }
        }
    }
}
