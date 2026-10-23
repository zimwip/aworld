//! Changes of matter from one kind to another, in one table: each change says from what, into
//! what and under which conditions. The conditions read the fields of the world at the cell (its
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

use super::thermal::{cap, heat_of, latent0};
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

pub const TRANSITIONS: &[Transition] = &[
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

/// The world's fields at a cell, as the conditions read them.
#[derive(Clone, Copy, Default)]
pub struct Here {
    pub temp: f32,
    pub pressure: f32,
    pub shock: f32,
    pub above: Option<Kind>,
}

impl Cond {
    fn holds(self, h: &Here) -> bool {
        match self {
            TempAbove(t) => h.temp >= t,
            TempBelow(t) => h.temp < t,
            PressureAbove(p) => h.pressure >= p,
            ShockAbove(p) => h.shock >= p,
            Under(k) => h.above == Some(k),
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

    /// A change of crystal needs heat; a mere packing does not, and it is undone when the matter
    /// is thrown loose.
    fn needs_heat(&self) -> bool {
        self.when.iter().any(|c| matches!(c, TempAbove(_)))
    }
}

/// What a shock of `shock` atm with gas at `temp` °C makes of `m`, if anything. `loose`: the
/// matter is thrown out at once, so only the changes of crystal stay.
pub fn shocked(m: Mat, shock: f32, temp: f32, loose: bool) -> Option<&'static Transition> {
    let here = Here { temp, pressure: 1.0, shock, above: None };
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
        for tr in TRANSITIONS.iter().filter(|t| !t.by_shock()) {
            t[tr.from as usize] = true;
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
    /// new matter holds what the old one did, less the latent heat the change gives off.
    pub(super) fn change_state(&mut self, i: usize, to: Mat, latent: f32) {
        let from = self.cells[i].m;
        let t = self.temp[i];
        let heat = heat_of(from, t) - latent;
        let (x, y) = ((i % W) as i32, (i / W) as i32);
        self.set(x, y, to);
        self.temp[i] = (heat - latent0(to)) / cap(to);
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
            let here = Here { temp: self.temp[i], pressure: load[i], shock: 0.0, above: (i >= W).then(|| self.cells[i - W].m.kind()) };
            for tr in TRANSITIONS.iter().filter(|t| t.from == m && !t.by_shock()) {
                if tr.holds(&here) && (tr.every <= SETTLE_EVERY || self.rand() % tr.every < SETTLE_EVERY) {
                    self.change_state(i, tr.to, tr.latent);
                    break;
                }
            }
        }
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
