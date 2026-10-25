//! Changes of matter from one kind to another, in one place: each change says from what, into
//! what and under which conditions. Two tables hold them all but the reactions between neighbours
//! (rust and charcoal, iron and coals, see `thermal::react`): `FUSIONS`, the changes of state with a
//! latent heat (melting and setting), and `TRANSITIONS`, every other change, by heat, by weight or
//! by shock. The conditions read the fields of the world at the cell (its
//! temperature, the weight of everything above it, the peak of a shock wave that crossed it, what
//! lies on top of it), so one rule covers every place where those conditions are met.
//!
//! The weight of the ground is the lithostatic pressure: one atmosphere of air plus the weight of
//! the column of matter above the cell. Sediments that lie long enough under a load compact:
//! soil soaked under water turns to silt (mud), buried mud loses its water and sets into clay,
//! clay and sand under a great weight become rock (shale, sandstone). In the real world that takes
//! kilometres of ground and millions of years; the world is only 27 m deep, so the loads here are
//! scaled down and the changes come in hours of play.
//!
//! A shock is the same idea for a split second: the detonation of a high explosive squeezes the
//! matter around it far beyond any weight of ground. Above 50 000 atmospheres and 1500 °C carbon
//! turns into diamond, which is how detonations make real nanodiamonds. Black powder only burns
//! (it peaks at a few thousand atmospheres): it neither packs the soil nor makes diamond.

use super::thermal::{cap, heat_of, latent0, GLASS_C, KILN_C, LATENT_FUSION, LATENT_METAL, LATENT_ROCK, MELT_C, METAL_MELT_C, ROCK_MELT_C, WOOD_IGNITE_C};
use super::{density, Kind, Mat, World, H, W};

/// A condition a change needs; all the conditions of a change must hold.
#[derive(Clone, Copy, Debug)]
pub enum Cond {
    /// Temperature of the cell, °C.
    TempAbove(f32),
    TempBelow(f32),
    /// Lasting pressure at the cell (the weight of what is above it), atm.
    PressureAbove(f32),
    /// Peak pressure of a shock wave going through the cell, atm. It lasts a split second.
    ShockAbove(f32),
    /// The cell just above is of this kind.
    Under(Kind),
    /// Air with oxygen in it reaches the cell (see `World::breathes`), or it does not.
    Air,
    NoAir,
}

#[derive(Clone, Copy, Debug)]
pub struct Transition {
    pub from: Mat,
    pub to: Mat,
    /// All must hold.
    pub when: &'static [Cond],
    /// Heat given off by the change (taken in if negative), on top of what the two matters hold.
    pub latent: f32,
    /// 1: at once. N: one chance in N per step while the conditions hold (slow changes).
    pub every: u32,
    /// Whether the matter changes back when the conditions no longer hold.
    pub reversible: bool,
}

use Cond::*;

/// A change by heat alone, with no latent heat, that does not go back.
const fn heat(from: Mat, to: Mat, when: &'static [Cond], every: u32) -> Transition {
    Transition { from, to, when, latent: 0.0, every, reversible: false }
}

/// Every change of matter but the changes of state with a latent heat (`FUSIONS`) and the
/// reactions between neighbours (`thermal::react`). A change whose conditions read only the heat
/// of the cell (and the air around it) is looked at by the heat pass, the others by the slow pass
/// (`settle_matter`) or by the shock of a blast (`shock_cell`).
pub const TRANSITIONS: &[Transition] = &[
    // Heat. Sand fuses into glass (which does not melt back in a fire); clay is fired into
    // terracotta.
    heat(Mat::Sand, Mat::Glass, &[TempAbove(GLASS_C)], 15),
    heat(Mat::Shard, Mat::Glass, &[TempAbove(GLASS_C)], 15),
    heat(Mat::Clay, Mat::Terracotta, &[TempAbove(KILN_C)], 12),
    // With air wood burns; shut away from the air it only chars (pyrolysis): the water and the
    // tars leave and almost pure carbon stays, charcoal.
    heat(Mat::Wood, Mat::Ember, &[TempAbove(WOOD_IGNITE_C), Air], 10),
    heat(Mat::Trunk, Mat::Ember, &[TempAbove(WOOD_IGNITE_C), Air], 10),
    heat(Mat::Leaf, Mat::Ember, &[TempAbove(WOOD_IGNITE_C), Air], 10),
    heat(Mat::Branch, Mat::Ember, &[TempAbove(WOOD_IGNITE_C), Air], 10),
    heat(Mat::DeadLeaf, Mat::Ember, &[TempAbove(WOOD_IGNITE_C), Air], 10),
    heat(Mat::Wood, Mat::Charcoal, &[TempAbove(WOOD_IGNITE_C), NoAir], 40),
    heat(Mat::Trunk, Mat::Charcoal, &[TempAbove(WOOD_IGNITE_C), NoAir], 40),
    heat(Mat::Leaf, Mat::Charcoal, &[TempAbove(WOOD_IGNITE_C), NoAir], 40),
    heat(Mat::Branch, Mat::Charcoal, &[TempAbove(WOOD_IGNITE_C), NoAir], 40),
    heat(Mat::DeadLeaf, Mat::Charcoal, &[TempAbove(WOOD_IGNITE_C), NoAir], 40),
    // Weight.
    // Soil under standing water soaks into silt.
    Transition { from: Mat::Dirt, to: Mat::Mud, when: &[Under(Kind::Liquid)], latent: 0.0, every: 60_000, reversible: false },
    // Buried mud squeezed by the weight above it loses its water and sets into clay.
    Transition { from: Mat::Mud, to: Mat::Clay, when: &[PressureAbove(1.6)], latent: 0.0, every: 80_000, reversible: false },
    // Clay and sand under a great weight become rock: shale and sandstone.
    Transition { from: Mat::Clay, to: Mat::Stone, when: &[PressureAbove(3.0)], latent: 0.0, every: 400_000, reversible: false },
    Transition { from: Mat::Sand, to: Mat::Stone, when: &[PressureAbove(3.2)], latent: 0.0, every: 400_000, reversible: false },
    // Deep snow is pressed into ice by its own weight (how glaciers form).
    Transition { from: Mat::Snow, to: Mat::Ice, when: &[PressureAbove(1.08)], latent: 0.0, every: 20_000, reversible: false },
    // A detonation packs soil tight and snow into ice, and fuses sand into rock.
    Transition { from: Mat::Dirt, to: Mat::Clay, when: &[ShockAbove(5_000.0)], latent: 0.0, every: 1, reversible: false },
    Transition { from: Mat::Grass, to: Mat::Clay, when: &[ShockAbove(5_000.0)], latent: 0.0, every: 1, reversible: false },
    Transition { from: Mat::Snow, to: Mat::Ice, when: &[ShockAbove(200.0)], latent: 0.0, every: 1, reversible: false },
    Transition { from: Mat::Sand, to: Mat::Stone, when: &[ShockAbove(30_000.0)], latent: 0.0, every: 1, reversible: false },
    // Carbon squeezed while white hot: diamond. It stays diamond afterwards (it is metastable).
    Transition { from: Mat::Charcoal, to: Mat::Diamond, when: &[ShockAbove(50_000.0), TempAbove(1_500.0)], latent: 0.0, every: 1, reversible: false },
    Transition { from: Mat::Charcoal, to: Mat::Diamond, when: &[PressureAbove(50_000.0), TempAbove(1_500.0)], latent: 0.0, every: 1, reversible: false },
];

/// The solid and the liquid state of one matter, the temperature where one turns into the other,
/// and the latent heat between them: the cell takes in (or gives up) that heat at that
/// temperature before it changes (see `thermal::latent_heat`). Water boils as well.
#[derive(Clone, Copy, Debug)]
pub struct Fusion {
    pub solid: Mat,
    pub liquid: Mat,
    pub at: f32,
    pub latent: f32,
}

/// Water freezes into ice (the first entry for it); snow and hail melt into water too.
pub const FUSIONS: &[Fusion] = &[
    Fusion { solid: Mat::Ice, liquid: Mat::Water, at: MELT_C, latent: LATENT_FUSION },
    Fusion { solid: Mat::Snow, liquid: Mat::Water, at: MELT_C, latent: LATENT_FUSION },
    Fusion { solid: Mat::Hail, liquid: Mat::Water, at: MELT_C, latent: LATENT_FUSION },
    Fusion { solid: Mat::Stone, liquid: Mat::Lava, at: ROCK_MELT_C, latent: LATENT_ROCK },
    Fusion { solid: Mat::Iron, liquid: Mat::Molten, at: METAL_MELT_C, latent: LATENT_METAL },
    Fusion { solid: Mat::Steel, liquid: Mat::MoltenSteel, at: METAL_MELT_C, latent: LATENT_METAL },
];

/// The change of state `m` takes part in, if any: the one it melts by, or for a liquid the one
/// it sets by.
pub fn fusion(m: Mat) -> Option<&'static Fusion> {
    FUSIONS.iter().find(|f| f.solid == m).or_else(|| FUSIONS.iter().find(|f| f.liquid == m))
}

/// The world's fields at a cell, as the conditions read them.
#[derive(Clone, Copy, Default)]
pub struct Here {
    pub temp: f32,
    pub pressure: f32,
    pub shock: f32,
    pub above: Option<Kind>,
    pub air: bool,
}

impl Cond {
    fn holds(self, h: &Here) -> bool {
        match self {
            TempAbove(t) => h.temp >= t,
            TempBelow(t) => h.temp < t,
            PressureAbove(p) => h.pressure >= p,
            ShockAbove(p) => h.shock >= p,
            Under(k) => h.above == Some(k),
            Air => h.air,
            NoAir => !h.air,
        }
    }
}

impl Transition {
    fn holds(&self, h: &Here) -> bool {
        self.when.iter().all(|c| c.holds(h))
    }

    fn by_shock(&self) -> bool {
        self.when.iter().any(|c| matches!(c, ShockAbove(_)))
    }

    /// Whether the change reads nothing but the heat of the cell and the air around it.
    fn by_heat(&self) -> bool {
        self.when.iter().all(|c| matches!(c, TempAbove(_) | TempBelow(_) | Air | NoAir))
    }

    fn needs_air(&self) -> bool {
        self.when.iter().any(|c| matches!(c, Air | NoAir))
    }

    /// A change of crystal needs heat; a mere packing does not, and it is undone when the matter
    /// is thrown loose.
    fn needs_heat(&self) -> bool {
        self.when.iter().any(|c| matches!(c, TempAbove(_)))
    }
}

/// What a shock of `shock` atm with gas at `temp` °C makes of `m`, if anything. `loose`: the
/// matter is thrown out at once, so only the changes of crystal stay.
pub fn shocked(m: Mat, shock: f32, temp: f32, loose: bool) -> Option<&'static Transition> {
    let here = Here { temp, pressure: 1.0, shock, above: None, air: false };
    TRANSITIONS.iter().find(|t| t.from == m && t.by_shock() && (!loose || t.needs_heat()) && t.holds(&here))
}

/// Side of a cell, m (as in `blast.rs`).
const CELL_M: f32 = 0.05;
const ATM: f32 = 101_325.0;
/// The lasting changes are looked at once every this many steps.
pub const SETTLE_EVERY: u32 = 64;

/// Whether any lasting (not shock) change starts from `m`.
fn settles(m: Mat) -> bool {
    static T: std::sync::OnceLock<[bool; 256]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [false; 256];
        for tr in TRANSITIONS.iter().filter(|t| !t.by_shock() && !t.by_heat()) {
            t[tr.from as usize] = true;
        }
        t
    })[m as usize]
}

/// The changes by heat alone that start from `m`, in the order of `TRANSITIONS`. The heat pass
/// asks for every reactive cell, every pass: a lookup, not a walk through the whole table.
pub(super) fn by_heat(m: Mat) -> &'static [&'static Transition] {
    static T: std::sync::OnceLock<Vec<Vec<&'static Transition>>> = std::sync::OnceLock::new();
    &T.get_or_init(|| {
        let mut t = vec![vec![]; 256];
        for tr in TRANSITIONS.iter().filter(|t| t.by_heat()) {
            t[tr.from as usize].push(tr);
        }
        t
    })[m as usize]
}

/// Weight of one cell of `m`, atm.
fn weight(m: Mat) -> f32 {
    let rho = match m.kind() {
        Kind::Empty | Kind::Gas | Kind::Flame => 1.2,
        _ => density(m) * 1000.0,
    };
    rho * 9.81 * CELL_M / ATM
}

impl World {
    /// Turns cell `i` into `to`, keeping the heat it holds: the temperature moves so that the
    /// new matter holds what the old one did, less the latent heat the change gives off. The
    /// other states the cell carries stay when `to` carries them too (see `Aspect`).
    pub(super) fn change_state(&mut self, i: usize, to: Mat, latent: f32) {
        let (from, t, kept) = (self.cells[i].m, self.temp[i], self.states(i));
        let heat = heat_of(from, t) - latent;
        let (x, y) = ((i % W) as i32, (i / W) as i32);
        self.set(x, y, to);
        self.temp[i] = (heat - latent0(to)) / cap(to);
        self.keep_states(i, kept, from);
    }

    /// Lithostatic pressure (atm) at every cell: the air plus the weight of the column above it.
    pub fn load(&self) -> Vec<f32> {
        let mut p = vec![0.0; W * H];
        for x in 0..W {
            let mut acc = 1.0;
            for y in 0..H {
                let i = y * W + x;
                p[i] = acc;
                acc += weight(self.cells[i].m);
            }
        }
        p
    }

    /// The slow changes: every cell whose conditions hold may change, with its chance.
    pub(super) fn settle_matter(&mut self) {
        let load = self.load();
        for i in 0..W * H {
            let m = self.cells[i].m;
            if !settles(m) || self.mask[i] != 0 {
                continue;
            }
            let here = Here { temp: self.temp[i], pressure: load[i], shock: 0.0, above: (i >= W).then(|| self.cells[i - W].m.kind()), air: false };
            for tr in TRANSITIONS.iter().filter(|t| t.from == m && !t.by_shock() && !t.by_heat()) {
                if tr.holds(&here) && (tr.every <= SETTLE_EVERY || self.rand() % tr.every < SETTLE_EVERY) {
                    self.change_state(i, tr.to, tr.latent);
                    break;
                }
            }
        }
    }

    /// The changes the heat pass looks at: whether cell `i` (at x, y, made of `m`) changed.
    pub(super) fn heat_change(&mut self, i: usize, x: i32, y: i32, m: Mat) -> bool {
        let mut here = Here { temp: self.temp[i], pressure: 1.0, shock: 0.0, above: None, air: false };
        let mut air_known = false;
        for tr in by_heat(m) {
            if tr.needs_air() && !air_known {
                here.air = self.breathes(x, y);
                air_known = true;
            }
            if tr.holds(&here) && self.chance_t(tr.every) {
                self.change_state(i, tr.to, tr.latent);
                return true;
            }
        }
        false
    }

    /// A shock wave of peak `shock` atm, with gas at `temp` °C, went through cell `i`: the
    /// changes it brings about happen at once. Returns whether the cell changed.
    pub(super) fn shock_cell(&mut self, i: usize, shock: f32, temp: f32) -> bool {
        let Some(tr) = shocked(self.cells[i].m, shock, self.temp[i].max(temp), false) else { return false };
        self.change_state(i, tr.to, tr.latent);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_load_grows_with_the_weight_above() {
        let mut w = World::new_static();
        for y in 100..H as i32 {
            for x in 0..10 {
                w.set(x, y, if y < 300 { Mat::Water } else { Mat::Stone });
            }
        }
        let p = w.load();
        // 200 cells of water are 10 m: one more atmosphere.
        let at = |y: usize| p[y * W + 5];
        assert!((at(300) - at(100) - 0.97).abs() < 0.05, "{} atm under 10 m of water", at(300) - at(100));
        assert!(at(500) - at(300) > 2.5 * (at(300) - at(100)), "rock weighs more than water");
        assert!((at(50) - 1.0).abs() < 0.01, "open air");
    }

    #[test]
    fn mud_buried_deep_sets_into_clay_but_not_near_the_surface() {
        let mut w = World::new_static();
        for x in 0..40 {
            for y in 200..H as i32 {
                w.set(x, y, if y < 420 { Mat::Dirt } else { Mat::Mud });
            }
            w.set(x, 230, Mat::Mud);
        }
        for _ in 0..3_000 {
            w.settle_matter();
        }
        let clay = |y0: i32, y1: i32| (0..40).flat_map(|x| (y0..y1).map(move |y| (x, y))).filter(|&(x, y)| w.raw(x, y) == Mat::Clay).count();
        assert!(clay(420, H as i32) > 400, "deep mud became clay: {}", clay(420, H as i32));
        assert_eq!(w.raw(5, 230), Mat::Mud, "shallow mud stays mud");
    }

    #[test]
    fn soil_under_still_water_soaks_into_mud() {
        let mut w = World::new_static();
        for x in 0..40 {
            w.set(x, 300, Mat::Dirt);
            w.set(x, 301, Mat::Dirt);
            w.set(x, 299, if x < 20 { Mat::Water } else { Mat::Empty });
        }
        for _ in 0..4_000 {
            w.settle_matter();
        }
        let mud = |x0: i32, x1: i32| (x0..x1).filter(|&x| w.raw(x, 300) == Mat::Mud).count();
        assert!(mud(0, 20) > 5, "under water: {}", mud(0, 20));
        assert_eq!(mud(20, 40), 0, "in the open air the soil stays soil");
        assert!((0..40).all(|x| w.raw(x, 301) == Mat::Dirt), "only the top of the bed soaks");
    }

    #[test]
    fn a_shock_needs_both_pressure_and_heat_to_make_diamond() {
        let mut w = World::new_static();
        for x in 0..3 {
            w.set(x, 10, Mat::Charcoal);
        }
        w.temp[10 * W] = 20.0;
        assert!(!w.shock_cell(10 * W, 3_000.0, 2_000.0), "black powder: too weak");
        assert!(!w.shock_cell(10 * W + 1, 100_000.0, 600.0), "squeezed but cold");
        assert!(w.shock_cell(10 * W + 2, 100_000.0, 3_000.0));
        assert_eq!(w.raw(2, 10), Mat::Diamond);
    }

    #[test]
    fn soil_soaking_into_mud_keeps_its_water_and_rock_loses_it() {
        let mut w = World::new_static();
        w.set(5, 5, Mat::Dirt);
        let i = 5 * W + 5;
        w.moisture[i] = 150;
        w.change_state(i, Mat::Mud, 0.0);
        assert_eq!(w.moisture[i], 150, "mud carries water: the soil's stays in it");
        w.change_state(i, Mat::Clay, 0.0);
        assert_eq!(w.moisture[i], 0, "clay carries none");
    }

    #[test]
    fn states_move_with_the_matter_and_painting_starts_afresh() {
        let mut w = World::new_static();
        w.set(5, 5, Mat::Mud);
        let (i, j) = (5 * W + 5, 6 * W + 5);
        w.moisture[i] = 99;
        w.swap((5, 5), (5, 6));
        assert_eq!((w.moisture[i], w.moisture[j]), (0, 99), "the water went down with the mud");
        w.set(5, 6, Mat::Mud);
        assert_eq!(w.moisture[j], 0);
        w.set(5, 6, Mat::Ember);
        assert!(w.fuel[j].left > 0 && !w.fuel[j].carbon, "a fresh ember of wood");
    }

    #[test]
    fn water_sets_into_ice_and_snow_melts_into_water() {
        assert_eq!(fusion(Mat::Water).map(|f| f.solid), Some(Mat::Ice));
        assert_eq!(fusion(Mat::Snow).map(|f| f.liquid), Some(Mat::Water));
        assert_eq!(fusion(Mat::Lava).map(|f| f.solid), Some(Mat::Stone));
        assert!(fusion(Mat::Sand).is_none());
    }

    #[test]
    fn hot_sand_turns_to_glass_in_the_heat_pass_keeping_its_heat() {
        let mut w = World::new_static();
        w.set(5, 5, Mat::Sand);
        let i = 5 * W + 5;
        w.temp[i] = 700.0;
        let before = heat_of(Mat::Sand, 700.0);
        while !w.heat_change(i, 5, 5, Mat::Sand) {}
        assert_eq!(w.raw(5, 5), Mat::Glass);
        assert!((heat_of(Mat::Glass, w.temp[i]) - before).abs() < 1e-2);
    }

    #[test]
    fn a_change_of_matter_keeps_the_heat() {
        let mut w = World::new_static();
        w.set(5, 5, Mat::Mud);
        let i = 5 * W + 5;
        w.temp[i] = 40.0;
        let before = heat_of(Mat::Mud, 40.0);
        w.change_state(i, Mat::Clay, 0.0);
        assert!((heat_of(Mat::Clay, w.temp[i]) - before).abs() < 1e-3);
    }
}
