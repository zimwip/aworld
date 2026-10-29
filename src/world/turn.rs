//! The cells' turn: every cell of matter takes its turn, bottom row first, left to right or right
//! to left by turns.
//!
//! It can also run on every core (`World::parallel_cells`: on with the physics of the liquids,
//! else off unless `AWORLD_CELLS_PAR=1`). The
//! world is then cut into strips `STRIP_W` columns wide (a sleeping zone, two of 16), from the top of the world to the bottom;
//! the cuts move every other step (see `strip_offset`), so that no column of the world stays on
//! the edge of a strip;
//! every third strip runs at once (three rounds, side by side), each in a view of the world of its
//! own, and in a strip the cells take their turn in the usual order. This is about twice as fast
//! on four cores when much is moving, but it is not the same turn: the rules of liquids depend on
//! the order the cells come in, and water spreads and drains differently.
//!
//! A cell's rule only reaches `REACH` columns either side of its strip (debug builds check it, see
//! `guard`), and the strips of a round are two strips apart: they never touch the same cell. The
//! few rules that reach further (a charge of powder going off, a tree breathing out through its
//! crown) are put off to the end of the turn, and run there one after another (see `Later`).
//!
//! On the edge of a strip, the cells on one side take their turn all before or all after those
//! on the other, from the top of the world to the bottom, where the usual order mixes them row
//! by row. Grains feel it most: in the usual order, a grain that leaves a cell is followed by the
//! grain beside and above it, which slides in, then by the next one up, and an avalanche runs
//! across a whole heap in one step. On every core, the grains of the strip that already had its
//! turn could not follow: a hole left by the edge of a strip is offered to them, as the usual
//! order would (see `Edge` and `refill`).
//!
//! Each strip has its own random draws, seeded from the step and its place, and the views hand
//! back what they changed besides their cells (zones woken, events, heat given to the sky) in the
//! order of the strips. So the turn comes out the same whatever the number of cores, in the
//! browser (one core) too.

use super::*;
use rayon::prelude::*;
use std::mem::{take, ManuallyDrop};

/// Columns a rule may reach either side of its strip.
pub const REACH: i32 = 32;
/// Sleeping zones across a strip: enough for the strip to be as wide as the reach.
const STRIP_ZONES: usize = (REACH as usize).div_ceil(sleep::CS);
/// Columns across a strip, and strips across the world.
const STRIP_W: usize = STRIP_ZONES * sleep::CS;
#[allow(non_snake_case)]
fn STRIPS() -> usize {
    sleep::CW() / STRIP_ZONES
}
/// Strips of a round are this many strips from one another. Between two of them lie
/// `ROUNDS - 1` strips, which their reach shares out; the world wraps round at the sides, so the
/// number of strips is a multiple of it.
const ROUNDS: usize = 3;
const _: () = assert!(2 * REACH as usize <= (ROUNDS - 1) * STRIP_W);

/// The strips fill the world's width, in whole rounds (see `GRID_W`).
pub(super) fn check_strips() {
    assert!(sleep::CW() % STRIP_ZONES == 0 && W() % STRIP_W == 0 && STRIPS() % ROUNDS == 0, "a world {} cells wide does not cut into strips", W());
}

/// Where the strips start this step, from the left side of the world: it moves 13 columns every
/// other step (so with either order of the columns), and comes round to every column of a strip.
///
/// Whatever happens on the edges of the strips (see the top) then never stays on the same
/// columns: with the cuts in place, a heap of sand stood in a cliff along each, with steps the
/// width of a strip.
fn strip_offset(frame: u8) -> usize {
    (frame as usize >> 1) * 13 % STRIP_W
}

/// The first column of strip `s` this step; the strip runs `STRIP_W` columns to the right (the
/// last one goes round the right side of the world to the left one, as the world's rules do).
fn strip_start(s: usize, frame: u8) -> usize {
    s * STRIP_W + strip_offset(frame)
}

/// Columns of the strip beside it, over a hole left by the edge of a strip, whose grains may
/// still drop into it: what the rules of a grain reach is then still within `REACH`.
const BESIDE: i32 = REACH - 2;

/// The strip running on this thread, and which of the strips beside it had their turn already
/// and come first in the row (the usual order takes the row from left to right or from right to
/// left by turns): their grains get a hole left by the edge of the strip first.
#[derive(Clone, Copy)]
struct Edge {
    x0: usize,
    left_first: bool,
    right_first: bool,
}

thread_local! {
    static EDGE: std::cell::Cell<Option<Edge>> = const { std::cell::Cell::new(None) };
}

/// A rule put off to the end of the turn: it reaches too far to run beside other strips.
#[derive(Clone, Copy, Debug)]
pub(super) enum Later {
    /// A charge of powder or dynamite at this cell goes off (see `set_off_powder`).
    Powder(i32, i32),
    /// The tree whose trunk this is breathes out (see `transpire`).
    Transpire(i32, i32),
    /// A grain left this cell by the edge of a strip; the grains over it, in the strip beside,
    /// may have had their turn already (see `refill`).
    Hole(i32, i32),
}

/// The world, for the strips of a round.
struct Ptr(*const World);
// SAFETY: the strips of a round only reach cells no other strip of the round reaches (see the top).
unsafe impl Sync for Ptr {}

impl Ptr {
    /// # Safety
    /// Only for the strips of one round, while nothing else touches the world.
    unsafe fn world(&self) -> &World {
        &*self.0
    }
}

/// A copy of a field that shares its storage with the world's: never dropped, never grown.
unsafe fn share<T>(v: &T) -> T {
    std::ptr::read(v)
}

/// The seed of strip `zx`'s draws this step. The draws are xorshift, which is linear: seeds that
/// only differ by a constant, or follow one another in the world's stream, would give a strip
/// nearly the same draws step after step (a heap of sand creeps to one side). Hence a hash.
fn mix(seed: u32, zx: u32) -> u32 {
    let mut h = seed ^ zx.wrapping_mul(0x9e37_79b9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h | 1
}

/// Whether new worlds take the cells' turn on every core: yes unless `AWORLD_CELLS_PAR=0`. The
/// liquids, moved by their physics, do not take the cells' turn, which was what came out
/// differently on every core.
pub(super) fn parallel_by_default() -> bool {
    std::env::var_os("AWORLD_CELLS_PAR").is_none_or(|v| v != "0")
}

#[cfg(debug_assertions)]
thread_local! {
    /// The strip running on this thread, in a view: (first column, last column + 1).
    static STRIP: std::cell::Cell<Option<(i32, i32)>> = const { std::cell::Cell::new(None) };
}

/// Debug builds check every cell a rule reaches: within `REACH` columns of its strip (the world
/// wraps round at the sides, and so may the strip).
#[cfg(debug_assertions)]
#[inline]
#[track_caller]
pub(super) fn guard(x: i32) {
    if let Some((x0, x1)) = STRIP.with(|t| t.get()) {
        let d = |a: i32| a.rem_euclid(W() as i32).min((-a).rem_euclid(W() as i32));
        let off = if (x - x0).rem_euclid(W() as i32) < x1 - x0 { 0 } else { d(x - x0).min(d(x - (x1 - 1))) };
        assert!(off <= REACH, "a rule of the cells' turn reached column {x}, {off} columns off its strip {x0}..{x1}");
    }
}

impl World {
    /// Every cell of matter takes its turn, bottom row first (`again`: a tick after the first of
    /// the step, see `Zones::begin`).
    pub(super) fn step_cells(&mut self, again: bool) {
        self.air_rows = (0..H() as i32).map(|y| self.ambient_c(y)).collect();
        self.air_sky = self.climate.sky_dev;
        self.zones.begin(again);
        if self.parallel_cells {
            self.cells_on_every_core();
        } else {
            self.cells_in_order();
        }
        self.air_rows.clear();
    }

    fn cells_in_order(&mut self) {
        let ltr = self.frame & 1 == 0;
        for y in (0..H() as i32).rev() {
            let zrow = y as usize / sleep::CS * sleep::CW();
            for zi in 0..sleep::CW() {
                let zx = if ltr { zi } else { sleep::CW() - 1 - zi };
                let zone = zrow + zx;
                if !self.zones.awake[zone] {
                    continue;
                }
                let (x0, x1) = ((zx * sleep::CS) as i32, ((zx + 1) * sleep::CS).min(W()) as i32);
                for i in 0..x1 - x0 {
                    let x = if ltr { x0 + i } else { x1 - 1 - i };
                    self.cell_turn(x, y, zone);
                }
            }
        }
    }

    fn cells_on_every_core(&mut self) {
        let ltr = self.frame & 1 == 0;
        let seed = self.rand();
        let mut later = Vec::new();
        for k in 0..ROUNDS {
            let round = if ltr { k } else { ROUNDS - 1 - k };
            // The zones a strip runs over: one more than it holds when the cuts are not on their
            // edges.
            let zones_of = |s: usize| {
                let x0 = strip_start(s, self.frame);
                (x0 / sleep::CS..(x0 + STRIP_W - 1) / sleep::CS + 1).map(|zx| zx % sleep::CW())
            };
            let strips: Vec<usize> = (round..STRIPS())
                .step_by(ROUNDS)
                .filter(|&s| (0..sleep::CH()).any(|zy| zones_of(s).any(|zx| self.zones.awake[zy * sleep::CW() + zx])))
                .collect();
            let views: Vec<ManuallyDrop<World>> = match strips.len() {
                0 => continue,
                // One strip alone runs here, in the same kind of view.
                1 => vec![self.strip_turn(strips[0], ltr, seed)],
                _ => {
                    let me = Ptr(self);
                    // SAFETY: see `Ptr`; the world itself is left alone until the round is over.
                    strips.par_iter().map(|&zx| unsafe { me.world().strip_turn(zx, ltr, seed) }).collect()
                }
            };
            for mut v in views {
                self.absorb(&mut v, &mut later);
            }
        }
        // What reaches too far, in the order the strips met it.
        for l in later {
            match l {
                Later::Powder(x, y) => {
                    if blast::explosive(self.raw(x, y)).is_some() {
                        self.set_off_powder(x, y);
                    }
                }
                Later::Transpire(x, y) => {
                    if self.raw(x, y) == Mat::Trunk {
                        self.transpire(x, y);
                    }
                }
                Later::Hole(x, y) => self.refill(x, y, None),
            }
        }
    }

    /// The turn of the cells of strip `s`, bottom row first, in a view of the world of its own.
    fn strip_turn(&self, s: usize, ltr: bool, seed: u32) -> ManuallyDrop<World> {
        let mut v = ManuallyDrop::new(self.view(mix(seed, s as u32)));
        let (w, x0) = (W(), strip_start(s, self.frame));
        #[cfg(debug_assertions)]
        STRIP.with(|t| t.set(Some((x0 as i32, (x0 + STRIP_W) as i32))));
        // Which strips beside it had their turn already: those of the rounds before.
        let n = STRIPS();
        let place = |s: usize| if ltr { s % ROUNDS } else { ROUNDS - 1 - s % ROUNDS };
        let edge = Edge { x0: x0 % w, left_first: ltr && place((s + n - 1) % n) < place(s), right_first: !ltr && place((s + 1) % n) < place(s) };
        EDGE.with(|t| t.set(Some(edge)));
        // The strip's columns, cut where the zones are: (first column, last + 1, zone across).
        let mut runs = [(0, 0, 0); STRIP_ZONES + 1];
        let mut k = 0;
        let mut x = x0;
        while x < x0 + STRIP_W {
            let end = ((x / sleep::CS + 1) * sleep::CS).min(x0 + STRIP_W);
            runs[k] = ((x % w) as i32, (end - x + x % w) as i32, x % w / sleep::CS);
            k += 1;
            x = end;
        }
        for y in (0..H() as i32).rev() {
            let zrow = y as usize / sleep::CS * sleep::CW();
            for j in 0..k {
                let (xa, xb, zx) = runs[if ltr { j } else { k - 1 - j }];
                let zone = zrow + zx;
                if !v.zones.awake[zone] {
                    continue;
                }
                for i in 0..xb - xa {
                    let x = if ltr { xa + i } else { xb - 1 - i };
                    v.cell_turn(x, y, zone);
                }
            }
        }
        EDGE.with(|t| t.set(None));
        #[cfg(debug_assertions)]
        STRIP.with(|t| t.set(None));
        v
    }

    /// Takes in what a strip's view changed besides its cells, and lets go of what the view owns.
    /// The rest of the view is shared with the world: it is not dropped.
    fn absorb(&mut self, v: &mut World, later: &mut Vec<Later>) {
        self.revision = self.revision.max(v.revision);
        self.dried += v.dried;
        self.gone += v.gone;
        self.water_flow += v.water_flow;
        self.climate.sky_dev += v.climate.sky_dev - self.air_sky;
        self.touched.append(&mut v.touched);
        self.blasts.append(&mut v.blasts);
        for b in take(&mut v.powder_booms) {
            if self.powder_booms.len() < 16 {
                self.powder_booms.push(b);
            }
        }
        self.ejecta.append(&mut v.ejecta);
        self.thuds.append(&mut v.thuds);
        self.bursts.append(&mut v.bursts);
        self.bolts.append(&mut v.bolts);
        self.falling.append(&mut v.falling);
        self.slabs.append(&mut v.slabs);
        self.liq.drops.append(&mut v.liq.drops);
        later.extend(v.later.take().unwrap_or_default());
        self.zones.absorb(&v.zones);
        drop(std::mem::replace(&mut v.zones, sleep::Zones::none()));
        // The other vectors it owns were emptied above, or never grow in the turn.
    }

    /// A view of the world for one strip: the grids are the world's own (the strip writes its
    /// cells straight into them), everything that grows starts empty, and the random draws are
    /// the strip's own.
    fn view(&self, rng: u32) -> World {
        // SAFETY (every `share`): the field is only read and written in place during the turn,
        // and the view is never dropped (see `absorb`).
        unsafe {
            World {
                cells: share(&self.cells),
                frame: self.frame,
                rng,
                seen: Vec::new(),
                ground: Vec::new(),
                stack: Vec::new(),
                falling: Vec::new(),
                slabs: Vec::new(),
                level: Vec::new(),
                strain: share(&self.strain),
                pushes: Vec::new(),
                bodies: Vec::new(),
                strain_key: self.strain_key,
                stress_cache: Default::default(),
                stress_plans: Default::default(),
                stress_pos: Vec::new(),
                stress_local: Vec::new(),
                ticks: self.ticks,
                log: share(&self.log),
                paint_log: self.paint_log,
                temp: share(&self.temp),
                clock: self.clock,
                dt: self.dt,
                tick_time: self.tick_time,
                heat_stage: self.heat_stage,
                day_cycle: self.day_cycle,
                thermal_flip: self.thermal_flip,
                climate_fixed: self.climate_fixed,
                climate: self.climate.view(),
                revision: self.revision,
                dried: 0,
                gone: 0,
                wind: self.wind,
                wind_max: self.wind_max,
                wind_changes: self.wind_changes,
                windiness: self.windiness,
                wind_clock: self.wind_clock,
                biome: self.biome,
                leaf_rgb: self.leaf_rgb,
                autumn: self.autumn,
                grass_rgb: self.grass_rgb,
                structures_fall: self.structures_fall,
                mask: share(&self.mask),
                mask_prev: share(&self.mask_prev),
                block_mat: share(&self.block_mat),
                mask_pass: share(&self.mask_pass),
                touched: Vec::new(),
                blasts: Vec::new(),
                powder_booms: Vec::new(),
                ejecta: Vec::new(),
                thuds: Vec::new(),
                water_flow: 0,
                flow: share(&self.flow),
                dp: share(&self.dp),
                dp_slow: share(&self.dp_slow),
                dpd: share(&self.dpd),
                pressure_forces: self.pressure_forces,
                lod: self.lod,
                liq_tier: self.liq_tier,
                liq_work: 0,
                parallel_cells: true,
                // (The drops the cells make in the turn, the rain sweeping a cloud, `absorb` takes.)
                liq: Default::default(),
                fill: share(&self.fill),
                pocket_of: share(&self.pocket_of),
                pockets: share(&self.pockets),
                latent: share(&self.latent),
                vel: share(&self.vel),
                fall_at: share(&self.fall_at),
                moisture: share(&self.moisture),
                fuel: share(&self.fuel),
                growth: share(&self.growth),
                jet: share(&self.jet),
                liq_p: share(&self.liq_p),
                cls: share(&self.cls),
                liq_w: share(&self.liq_w),
                zones: self.zones.view(),
                bursts: Vec::new(),
                charge: share(&self.charge),
                mcharge: share(&self.mcharge),
                magnet: share(&self.magnet),
                polar: share(&self.polar),
                bolts: Vec::new(),
                storm_rng: self.storm_rng.clone(),
                pressure_cache: std::sync::Mutex::new(None),
                prof: prof::Profile::default(),
                air_rows: share(&self.air_rows),
                air_sky: self.air_sky,
                heat_carry: (0.0, 0.0),
                temp_old: Vec::new(),
                heat_took: Vec::new(),
                later: Some(Vec::new()),
            }
        }
    }

    /// A grain moved out of (x, y) during a strip's turn. By the edge of the strip, the grains
    /// that could drop into the hole from above may be in the strip beside, and have had their
    /// turn: when that strip comes first in the row, they get the hole now, before the grains of
    /// this strip over it; else at the end of the turn, if it is still open.
    pub(super) fn left_hole(&mut self, x: i32, y: i32) {
        let w = W() as i32;
        let Some(e) = EDGE.with(|t| t.get()) else { return };
        let d = (x - e.x0 as i32).rem_euclid(w);
        if d < 2 && e.left_first {
            self.refill(x, y, Some(e.x0 as i32 - BESIDE));
        } else if (STRIP_W as i32 - 2..STRIP_W as i32).contains(&d) && e.right_first {
            self.refill(x, y, Some(e.x0 as i32 + STRIP_W as i32));
        } else if !(2..STRIP_W as i32 - 2).contains(&(x - strip_offset(self.frame) as i32).rem_euclid(STRIP_W as i32)) {
            self.put_off(Later::Hole(x, y));
        }
    }

    /// The grains over the hole at (x, y) drop into it, then those over the hole each leaves, and
    /// so on up the heap: those of the row above in the order the row takes its turn, the one
    /// right over it, and those beside it, which slide in, or roll in from two cells away. Only
    /// grains that have not moved this step, and, during a strip's turn, only those of the
    /// `BESIDE` columns from `beside` on (the strip beside, which had its turn).
    fn refill(&mut self, x: i32, y: i32, beside: Option<i32>) {
        let w = W() as i32;
        let free = |s: &Self, x: i32, y: i32| matches!(s.at(x, y).kind(), Kind::Empty | Kind::Gas) && s.mask_free(x, y);
        let order: [i32; 5] = if self.frame & 1 == 0 { [-2, -1, 0, 1, 2] } else { [2, 1, 0, -1, -2] };
        let mut holes = vec![(x, y)];
        while let Some((hx, hy)) = holes.pop() {
            for dx in order {
                if !free(self, hx, hy) {
                    break;
                }
                let (gx, gy) = (hx + dx, hy - 1);
                if beside.is_some_and(|b| (gx - b).rem_euclid(w) >= BESIDE) {
                    continue;
                }
                let Some(i) = Self::ix(gx, gy) else { continue };
                let c = self.cells[i];
                let loose = c.m.kind() == Kind::Powder && !matches!(c.m, Mat::Seed | Mat::DeadLeaf | Mat::Shard) && !(c.m == Mat::Rust && self.magnet_holds(gx, gy));
                if !loose || c.stamp == self.frame || self.mask[i] != 0 {
                    continue;
                }
                self.step_powder(gx, gy);
                if self.cells[i].m != c.m {
                    holes.push((gx, gy));
                }
            }
        }
    }

    /// Puts `l` off to the end of the cells' turn when a strip is running in a view; returns
    /// whether it did (out of the turn, the rule just runs).
    pub(super) fn put_off(&mut self, l: Later) -> bool {
        match &mut self.later {
            Some(v) => {
                v.push(l);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world with a floor of rock, and a column of sand 60 cells wide and 300 high standing on
    /// it over the middle (across the cuts between strips), left to fall apart for `steps` steps.
    fn heap(parallel: bool, steps: usize) -> World {
        let mut w = World::new_static();
        w.parallel_cells = parallel;
        let (ww, hh) = (W() as i32, H() as i32);
        for x in 0..ww {
            for y in hh - 40..hh {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in ww / 2 - 30..ww / 2 + 30 {
            for y in hh - 340..hh - 40 {
                w.set(x, y, Mat::Sand);
            }
        }
        for _ in 0..steps {
            w.step();
        }
        w
    }

    /// How far the grains lying on something spread either side of their middle (a standard
    /// deviation, in cells).
    fn spread(w: &World) -> f64 {
        let (mut n, mut sx, mut sxx) = (0.0, 0.0, 0.0);
        for y in 0..H() - 1 {
            for x in 0..W() {
                if w.cells[y * W() + x].m == Mat::Sand && w.cells[(y + 1) * W() + x].m != Mat::Empty {
                    n += 1.0;
                    sx += x as f64;
                    sxx += (x * x) as f64;
                }
            }
        }
        let m = sx / n;
        (sxx / n - m * m).sqrt()
    }

    #[test]
    fn a_column_of_sand_on_every_core_falls_into_a_heap_not_steps() {
        let w = heap(true, 1800);
        let (c, floor) = (W() as i32 / 2, H() as i32 - 40);
        let surface: Vec<i32> = (c - 300..c + 300).map(|x| (0..floor).find(|&y| w.raw(x, y) == Mat::Sand).unwrap_or(floor)).collect();
        let cliff = surface.windows(2).map(|p| (p[0] - p[1]).abs()).max().unwrap();
        // With the cuts in place, the heap still stood in steps along them (a drop of 40 rows).
        assert!(cliff <= 8, "the side of the heap drops {cliff} rows from one column to the next");
    }

    #[test]
    fn sand_spreads_on_every_core_nearer_as_fast_as_in_order() {
        let (one, all) = (spread(&heap(false, 900)), spread(&heap(true, 900)));
        // Avalanches stopping at the edge of every strip spread it barely more than half as far
        // (32.5 cells for 57.6); passing on, about three quarters.
        assert!(all > 0.68 * one, "a heap falling apart on every core spreads {all:.1} cells, in order {one:.1}");
    }
}
