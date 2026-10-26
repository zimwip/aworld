//! The cells' turn: every cell of matter takes its turn, bottom row first, left to right or right
//! to left by turns.
//!
//! It can also run on every core (`World::parallel_cells`: on with the physics of the liquids,
//! else off unless `AWORLD_CELLS_PAR=1`). The
//! world is then cut into strips `STRIP_W` columns wide (a sleeping zone, two of 16), from the top of the world to the bottom;
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
    assert!(sleep::CW() % STRIP_ZONES == 0 && STRIPS() % ROUNDS == 0, "a world {} cells wide does not cut into strips", W());
}

/// A rule put off to the end of the turn: it reaches too far to run beside other strips.
#[derive(Clone, Copy, Debug)]
pub(super) enum Later {
    /// A charge of powder or dynamite at this cell goes off (see `set_off_powder`).
    Powder(i32, i32),
    /// The tree whose trunk this is breathes out (see `transpire`).
    Transpire(i32, i32),
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

/// Whether new worlds take the cells' turn on every core: `AWORLD_CELLS_PAR=1`, or the physics
/// of the liquids on (see `parallel_with_liquids`).
pub(super) fn parallel_by_default() -> bool {
    std::env::var_os("AWORLD_CELLS_PAR").is_some_and(|v| v != "0") || (liquid::physics_by_default() && parallel_with_liquids())
}

/// Whether the physics of the liquids turns the cells' turn on every core on (unless
/// `AWORLD_CELLS_PAR=0`): the liquids then no longer take the cells' turn, which was what came
/// out differently on every core.
pub(super) fn parallel_with_liquids() -> bool {
    std::env::var_os("AWORLD_CELLS_PAR").is_none_or(|v| v != "0")
}

#[cfg(debug_assertions)]
thread_local! {
    /// The strip running on this thread, in a view: (first column, last column + 1).
    static STRIP: std::cell::Cell<Option<(i32, i32)>> = const { std::cell::Cell::new(None) };
}

/// Debug builds check every cell a rule reaches: within `REACH` columns of its strip (the world
/// wraps round at the sides).
#[cfg(debug_assertions)]
#[inline]
#[track_caller]
pub(super) fn guard(x: i32) {
    if let Some((x0, x1)) = STRIP.with(|t| t.get()) {
        let d = |a: i32| a.rem_euclid(W() as i32).min((-a).rem_euclid(W() as i32));
        let off = if (x0..x1).contains(&x) { 0 } else { d(x - x0).min(d(x - (x1 - 1))) };
        assert!(off <= REACH, "a rule of the cells' turn reached column {x}, {off} columns off its strip {x0}..{x1}");
    }
}

impl World {
    /// Every cell of matter takes its turn, bottom row first.
    pub(super) fn step_cells(&mut self) {
        self.air_rows = (0..H() as i32).map(|y| self.ambient_c(y)).collect();
        self.air_sky = self.climate.sky_dev;
        self.zones.begin();
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
            let strips: Vec<usize> = (round..STRIPS())
                .step_by(ROUNDS)
                .filter(|&s| (0..sleep::CH()).any(|zy| (s * STRIP_ZONES..(s + 1) * STRIP_ZONES).any(|zx| self.zones.awake[zy * sleep::CW() + zx])))
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
            }
        }
    }

    /// The turn of the cells of strip `s`, bottom row first, in a view of the world of its own.
    fn strip_turn(&self, s: usize, ltr: bool, seed: u32) -> ManuallyDrop<World> {
        let mut v = ManuallyDrop::new(self.view(mix(seed, s as u32)));
        #[cfg(debug_assertions)]
        STRIP.with(|t| t.set(Some(((s * STRIP_W) as i32, ((s + 1) * STRIP_W).min(W()) as i32))));
        for y in (0..H() as i32).rev() {
            let zrow = y as usize / sleep::CS * sleep::CW();
            for j in 0..STRIP_ZONES {
                let zx = s * STRIP_ZONES + if ltr { j } else { STRIP_ZONES - 1 - j };
                let zone = zrow + zx;
                if !v.zones.awake[zone] {
                    continue;
                }
                let (x0, x1) = ((zx * sleep::CS) as i32, ((zx + 1) * sleep::CS).min(W()) as i32);
                for i in 0..x1 - x0 {
                    let x = if ltr { x0 + i } else { x1 - 1 - i };
                    v.cell_turn(x, y, zone);
                }
            }
        }
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
                stack: Vec::new(),
                released: share(&self.released),
                released_list: Vec::new(),
                falling: Vec::new(),
                slabs: Vec::new(),
                level: Vec::new(),
                strain: share(&self.strain),
                pushes: Vec::new(),
                strain_key: self.strain_key,
                stress_cache: Default::default(),
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
                parallel_cells: true,
                liquid_physics: self.liquid_physics,
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
