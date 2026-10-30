//! Sleeping zones. The world is cut into blocks of `CS` x `CS` cells. A block where nothing
//! changed during the last step, and where no cell can change on its own (rock, settled sand,
//! dry soil, water at rest under more water...), is asleep: the cells of the next step skip it.
//! Any change to a cell (`set`, `swap`, a falling piece, a rigid block moving) wakes its block, and
//! the neighbouring block too when the cell is on the edge. Heat, wind and pressure are not
//! affected: they keep running everywhere.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// Side of a sleeping zone, in cells: 32, or 16 or 64 when the game is built with
/// `AWORLD_ZONE=16` (or 64) to compare (smaller zones sleep more often but cost more to watch; see
/// the metrics panel, Ctrl+D).
pub const CS: usize = match option_env!("AWORLD_ZONE") {
    Some(v) => zone_side(v.as_bytes()),
    None => 32,
};

const fn zone_side(v: &[u8]) -> usize {
    match v {
        b"16" => 16,
        b"32" => 32,
        b"64" => 64,
        _ => panic!("AWORLD_ZONE : 16, 32 ou 64"),
    }
}
/// Zones across and down (kept, rather than worked out from `W()` and `H()` each time: `wake`
/// asks for them for most cells the turn touches).
static ZONES: [AtomicUsize; 2] = [AtomicUsize::new(960usize.div_ceil(CS)), AtomicUsize::new(540usize.div_ceil(CS))];
pub(super) fn set_zones(w: usize, h: usize) {
    ZONES[0].store(w.div_ceil(CS), Relaxed);
    ZONES[1].store(h.div_ceil(CS), Relaxed);
}
#[allow(non_snake_case)]
#[inline(always)]
pub fn CW() -> usize {
    ZONES[0].load(Relaxed)
}
#[allow(non_snake_case)]
#[inline(always)]
pub fn CH() -> usize {
    ZONES[1].load(Relaxed)
}

/// The state of the zones: which run this step, which were woken for the next one, and which
/// hold a cell that can change on its own.
#[derive(Clone)]
pub struct Zones {
    pub(super) awake: Vec<bool>,
    woke: Vec<bool>,
    restless: Vec<bool>,
    /// When false every zone always runs (to compare, and for the odd test).
    pub enabled: bool,
    /// Counts the steps; `changed[k]` is the count when zone `k` last ran or was touched. The
    /// renderer recolours only the zones that changed since it last drew.
    stamp: u64,
    changed: Vec<u64>,
}

impl Zones {
    pub fn new() -> Self {
        Zones { awake: vec![true; CW() * CH()], woke: vec![true; CW() * CH()], restless: vec![true; CW() * CH()], enabled: true, stamp: 1, changed: vec![1; CW() * CH()] }
    }

    /// Zones with no storage (to put in place of a view's zones once it is done with).
    pub(super) fn none() -> Self {
        Zones { awake: Vec::new(), woke: Vec::new(), restless: Vec::new(), enabled: true, stamp: 0, changed: Vec::new() }
    }

    /// The zones for a strip of the cells' turn (see `turn.rs`): which run this step, and marks of
    /// its own for what it wakes, finds restless or changes.
    pub(super) fn view(&self) -> Self {
        Zones { awake: self.awake.clone(), woke: vec![false; CW() * CH()], restless: vec![false; CW() * CH()], enabled: self.enabled, stamp: self.stamp, changed: vec![0; CW() * CH()] }
    }

    /// Takes in the marks of a strip's zones.
    pub(super) fn absorb(&mut self, v: &Zones) {
        for k in 0..CW() * CH() {
            self.woke[k] |= v.woke[k];
            self.restless[k] |= v.restless[k];
            if v.changed[k] == self.stamp {
                self.changed[k] = self.stamp;
            }
        }
    }

    /// Wakes every zone (after the whole grid was replaced).
    pub fn wake_all(&mut self) {
        self.woke.fill(true);
        self.changed.fill(self.stamp);
    }

    /// Wakes the zone of cell `i`, and the zone next door when the cell is on its edge.
    #[inline]
    pub fn wake(&mut self, i: usize) {
        let (x, y) = xy(i);
        let (cw, ch) = (CW(), CH());
        let (cx, cy) = (x / CS, y / CS);
        let (lx, ly) = (x % CS, y % CS);
        let k = cy * cw + cx;
        self.changed[k] = self.stamp;
        self.woke[k] = true;
        // The cell on the edge of its zone wakes the zone next door too (and gas wraps round
        // the sides of the world). Most cells are inside their zone: one test, then done.
        if lx == 0 || lx == CS - 1 || ly == 0 || ly == CS - 1 {
            let x0 = if lx == 0 && cx > 0 { cx - 1 } else { cx };
            let x1 = if lx == CS - 1 && cx + 1 < cw { cx + 1 } else { cx };
            let y0 = if ly == 0 && cy > 0 { cy - 1 } else { cy };
            let y1 = if ly == CS - 1 && cy + 1 < ch { cy + 1 } else { cy };
            for zy in y0..=y1 {
                for zx in x0..=x1 {
                    self.woke[zy * cw + zx] = true;
                }
            }
            if x == 0 {
                self.woke[cy * cw + cw - 1] = true;
            } else if x == W() - 1 {
                self.woke[cy * cw] = true;
            }
        }
    }

    /// Starts a tick: the zones woken during the last one, or holding a restless cell, run.
    /// With `again` (a tick after the first of a step of several), the zones that ran at an
    /// earlier tick of the step run too: the liquids take their turn once a step, after the last
    /// tick, and wake the zones they move for the next step only. Without it, a body of water
    /// moving through zones that nothing else wakes (full water under full water is not
    /// restless) found them asleep at the last tick whenever the game took steps of several
    /// ticks (when it falls behind): it stood still there in squares of `CS` cells, a wall to
    /// the water round it.
    pub(super) fn begin(&mut self, again: bool) {
        self.stamp += 1;
        for k in 0..CW() * CH() {
            self.awake[k] = !self.enabled || self.woke[k] || self.restless[k] || (again && self.awake[k]);
            if self.awake[k] {
                self.changed[k] = self.stamp;
            }
        }
        self.woke.fill(false);
        self.restless.fill(false);
    }

    #[inline]
    pub(super) fn mark_restless(&mut self, zone: usize) {
        self.restless[zone] = true;
    }

    /// The current step count (see `changed_since`).
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// Whether zone `k` ran or was touched at or after step count `s`: its cells may look
    /// different.
    pub fn changed_since(&self, k: usize, s: u64) -> bool {
        self.changed[k] >= s
    }

    /// Whether zone `k` runs this step.
    pub fn runs(&self, k: usize) -> bool {
        self.awake.get(k).copied().unwrap_or(true)
    }

    /// How many zones run this step (for the cost measure and the tests).
    pub fn awake_count(&self) -> usize {
        self.awake.iter().filter(|&&a| a).count()
    }
}

impl World {
    /// Whether the cell at (x, y) may change on its own next step even if nothing around it
    /// moves: a flame, a gas, a plant, wet soil, a grain that can still fall, water that can
    /// still flow or evaporate...
    pub(super) fn restless(&self, x: i32, y: i32, c: Cell) -> bool {
        let free = |m: Mat| matches!(m.kind(), Kind::Empty | Kind::Gas);
        let sinks_into = |m: Mat| matches!(m.kind(), Kind::Empty | Kind::Gas | Kind::Liquid);
        match c.m {
            Mat::Empty | Mat::Stone | Mat::Brick | Mat::Glass | Mat::Ice | Mat::Clay | Mat::Terracotta => false,
            Mat::Dirt => {
                // A clod with nothing under it, or at the edge of a bank steeper than it holds.
                let drop = |dx: i32| sinks_into(self.at(x + dx, y + 1)) && (dx == 0 || sinks_into(self.at(x + dx, y + 2)));
                (self.structures_fall && (drop(0) || drop(-1) || drop(1))) || self.soil_restless(x, y) || self.seed_waits(x, y)
            }
            // Grass can always change: it grows, dries, wilts or is smothered.
            Mat::Grass => true,
            Mat::Sand | Mat::Rust | Mat::Hail | Mat::Snow | Mat::Gunpowder | Mat::Ash | Mat::Charcoal | Mat::Saltpeter => {
                let can_fall = sinks_into(self.at(x, y + 1)) || sinks_into(self.at(x - 1, y + 1)) || sinks_into(self.at(x + 1, y + 1));
                // Free-running grains roll two cells sideways (see `step_powder`), and a grain
                // lighter than the liquid above it rises.
                let rolls = Self::grain_pick(x, y) < self.repose(x, y, c.m).1
                    && ((sinks_into(self.at(x - 1, y)) && sinks_into(self.at(x - 2, y + 1))) || (sinks_into(self.at(x + 1, y)) && sinks_into(self.at(x + 2, y + 1))));
                let lifts = |nx: i32, ny: i32| self.at(nx, ny).kind() == Kind::Liquid && density(self.at(nx, ny)) > density(c.m);
                let floats = lifts(x, y - 1)
                    // Held down under the surface, it rises aslant; on the side of a heap that
                    // floats, it runs out onto the surface (see `World::float_on`).
                    || ((lifts(x - 1, y - 1) || lifts(x + 1, y - 1) || !free(self.at(x, y - 1))) && (lifts(x - 1, y) || lifts(x + 1, y) || lifts(x, y + 1)))
                    || (!lifts(x, y + 1) && [-1, 1].iter().any(|&d| lifts(x + d, y + 1) && free(self.at(x + d, y))));
                // Deep snow creeps sideways onto free ground (see `World::snow_creeps`).
                let creeps = c.m == Mat::Snow && self.snow_creep_target(x, y).is_some();
                let can_fall = can_fall || rolls || floats || creeps;
                let wet = c.m == Mat::Ash && Self::neighbors(x, y).iter().any(|&(nx, ny)| self.at(nx, ny) == Mat::Water);
                // Saltpetre dissolves in water and turns to powder against charcoal.
                let wet = wet || c.m == Mat::Saltpeter && Self::neighbors(x, y).iter().any(|&(nx, ny)| matches!(self.at(nx, ny), Mat::Water | Mat::Charcoal));
                can_fall || wet
            }
            Mat::Iron | Mat::Steel => Self::neighbors(x, y).iter().any(|&(nx, ny)| self.at(nx, ny) == Mat::Water),
            Mat::Wood | Mat::Trunk => {
                let (mut cover, mut sealed, mut wet) = (0, true, false);
                for (nx, ny) in Self::neighbors(x, y) {
                    match self.at(nx, ny) {
                        Mat::Dirt | Mat::Stone | Mat::Sand | Mat::Mud => cover += 1,
                        Mat::Wood | Mat::Trunk | Mat::Branch | Mat::Grass => {}
                        Mat::Water => {
                            wet = true;
                            sealed = false;
                        }
                        _ => sealed = false,
                    }
                }
                wet || (sealed && cover >= 2)
            }
            Mat::Water => {
                self.jet[Self::idx(x, y)] >= 2
                    || free(self.at(x, y - 1))
                    || self.soil_drinks((x, y + 1))
                    || self.soil_drinks((x - 1, y))
                    || self.soil_drinks((x + 1, y))
                    || free(self.at(x, y + 1))
                    || free(self.at(x - 1, y + 1))
                    || free(self.at(x + 1, y + 1))
                    || free(self.at(x - 1, y))
                    || free(self.at(x + 1, y))
                    // Thin matter beside it with room in it: it runs in (see
                    // `World::open_pores`).
                    || [(x - 1, y), (x + 1, y)].iter().any(|&(nx, ny)| self.thin_has_room(nx, ny))
            }
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rock_and_settled_sand_fall_asleep_and_a_change_wakes_them() {
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            for y in 400..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
            for y in 380..400 {
                w.set(x, y, Mat::Sand);
            }
        }
        // Dry sand runs out at the sides of the world before it settles.
        for _ in 0..150 {
            w.step();
        }
        w.zones.begin(false);
        assert_eq!(w.zones.awake_count(), 0, "nothing can move: everything sleeps");
        // A grain dropped from above wakes its zone, which runs again.
        w.set(300, 100, Mat::Sand);
        w.zones.begin(false);
        assert!(w.zones.awake[(100 / CS) * CW() + 300 / CS]);
        assert!(w.zones.awake_count() <= 4);
    }

    #[test]
    fn the_renderer_learns_which_zones_changed() {
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            for y in 400..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        for _ in 0..40 {
            w.step();
        }
        let drawn = w.zones.stamp() + 1;
        w.step();
        assert!((0..CW() * CH()).all(|k| !w.zones.changed_since(k, drawn)), "nothing moved: nothing to redraw");
        w.set(300, 100, Mat::Sand);
        let k = (100 / CS) * CW() + 300 / CS;
        assert!(w.zones.changed_since(k, drawn), "a painted cell marks its zone");
        assert_eq!((0..CW() * CH()).filter(|&k| w.zones.changed_since(k, drawn)).count(), 1);
    }

    #[test]
    fn a_body_of_water_falling_in_steps_of_several_ticks_keeps_its_zones_running() {
        // The game takes steps of three ticks when it falls behind; the liquids run once a step.
        let mut w = World::new_static();
        for y in 64..224 {
            for x in 192..352 {
                w.set(x, y, Mat::Water);
            }
        }
        for k in 0..8 {
            w.step_dt(MAX_DT);
            let asleep = (0..CW() * CH())
                .filter(|&z| {
                    let (x0, y0) = ((z % CW()) * CS, (z / CW()) * CS);
                    (y0..y0 + CS).all(|y| (x0..x0 + CS).all(|x| w.raw(x as i32, y as i32) == Mat::Water))
                })
                .filter(|&z| !w.zones.runs(z))
                .count();
            assert_eq!(asleep, 0, "step {k}: zones of the falling water asleep (it stands still there in squares)");
        }
    }

    #[test]
    fn sleeping_zones_change_nothing_a_grain_still_falls_and_water_still_levels() {
        let run = |enabled: bool| {
            let mut w = World::new_static();
            w.zones.enabled = enabled;
            for x in 100..400 {
                w.set(x, 400, Mat::Stone);
            }
            for y in 300..400 {
                w.set(100, y, Mat::Stone);
                w.set(399, y, Mat::Stone);
            }
            for _ in 0..50 {
                w.step();
            }
            // Water poured on one side, sand dropped on the other, long after things settled.
            for x in 120..160 {
                for y in 300..340 {
                    w.set(x, y, Mat::Water);
                }
            }
            for x in 300..310 {
                w.set(x, 200, Mat::Sand);
            }
            for _ in 0..600 {
                w.step();
            }
            let water_top = (0..H() as i32).find(|&y| (101..399).any(|x| w.raw(x, y) == Mat::Water)).unwrap();
            let water_right = (101..399).rev().find(|&x| (300..400).any(|y| w.raw(x, y) == Mat::Water)).unwrap();
            let sand_low = (300..310).all(|x| w.raw(x, 200) != Mat::Sand);
            (water_top, water_right, sand_low)
        };
        let (top, right, fell) = run(true);
        let (top0, right0, _) = run(false);
        assert!(fell, "the sand fell");
        assert!(right > 380 && right0 > 380, "the water spread across the basin: {right} / {right0}");
        assert!((top - top0).abs() <= 1, "same level with or without sleeping zones: {top} / {top0}");
    }
}
