//! One record per matter (`MatSpec`): what it weighs, how strong it is, what it crumbles into.
//! Every model that needs one of these reads it here: the falling and the floating (density), the
//! blasts (density, strength, rubble), the structures (toughness, rubble, restitution), the
//! pockets of gas (the pressure a wall holds, whether gas seeps through) and the weight of matter
//! on the ground. These were tables of their own in each model, and did not always agree: crushed
//! ice was hail under a falling block and snow under a blast or a burst boiler, crushed clay was
//! earth or nothing, and a wall of glass, as strong as 7 MPa for a blast, held 0.8 atm in a
//! boiler with nothing to say how both could be.
//!
//! It also says how the cells' turn moves the matter (`Motion`, see `World::cell_turn`): one way
//! for all the grains, one for all the liquids, one for all the gases, the matter only setting
//! how fast it falls (`fall_ms`), how its heap holds (`World::repose`), how it drifts (`drift`).
//!
//! The match in `of` has no catch-all arm: a new matter does not compile until it has its record.

use super::units::RHO_AIR;
use super::{Mat, DROP_TERMINAL_MS, GRAIN_TERMINAL_MS, SNOW_TERMINAL_MS};

/// How the cells' turn moves a matter (`World::move_cell`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Motion {
    /// It stays where it is (solids: when they move, it is as a body, see `statics.rs`).
    Fixed,
    /// A grain: it falls, slides down the side of its heap and sinks or floats in a liquid
    /// (`World::step_powder`).
    Grain,
    /// Earth: grains that hold together, and only fall in a world where things fall apart.
    Clod,
    /// Grass, the living top of the soil: with nothing under it, it drops straight down, through
    /// a liquid too, until it lands on something.
    Turf,
    /// A liquid: the solver moves it (see `liquid.rs`); in the cells' turn a pocket of gas
    /// shoots it out, a lone drop falls through the air, and it rises through a heavier liquid.
    Liquid,
    /// A gas: buoyancy, the wind, and a warm gas climbs (`World::step_gas`, `drift`).
    Gas,
    /// Droplets hanging in the air: carried by the wind, otherwise a still wandering.
    Cloud,
    /// A flame: it licks upwards and leans with the wind.
    Flame,
}

/// Which ways a matter holds back what moves into it (a liquid, the kid, a car).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Holds {
    /// Every way: a wall.
    Every,
    /// Only what comes down on it from above, a "one-way platform": thin matter of a 2-D world
    /// that what moves sideways goes past (in front of it or behind). A tree stands in a plane
    /// of its own; ash, charcoal and dead leaves are light, open flakes that a flow runs through
    /// and under (they float on it, see `World::float_on`). The kid and the cars walk and drive
    /// through it and stand on it (`World::bears`); a liquid runs through it sideways but does
    /// not go down through it, and fills it (`World::open_pores`).
    FromAbove,
}

/// What the models know of a matter.
#[derive(Clone, Copy, Debug)]
pub struct MatSpec {
    /// Density, kg/m³ (air for the gases, the flames and the empty cell).
    pub density: f32,
    /// Tensile (or shear) strength, MPa; cohesion for soils. 0 for loose matter, liquids and
    /// gases: they only weigh.
    pub strength: f32,
    /// Fracture energy, J/m²: what it costs to open a crack through it. Brittle matters need
    /// little (ice, glass, rock), tough ones a great deal (wood, metal: they bend instead).
    pub toughness: f32,
    /// The excess pressure (atm) a wall of it a cell thick holds round a pocket of gas before it
    /// gives way; `None`: it never does, or it is no wall. A flat wall bends under the push, and
    /// gives way long before the matter itself would (a pane of glass 5 cm thick spanning a
    /// metre and a half fails under a third of an atmosphere, for a strength of 7 MPa): never
    /// more than its strength (`holds_no_more_than_its_strength`). Loose matter has none, and
    /// holds a little by its weight.
    pub holds_atm: Option<f32>,
    /// What it becomes crushed finer than a cell (by a landing, a blast, a burst): `None` if it
    /// does not crumble (wood splinters, metal bends).
    pub rubble: Option<Mat>,
    /// Coefficient of restitution: the share of the speed along the normal a body of it bounces
    /// back with.
    pub restitution: f32,
    /// Gas seeps through it: a cave dug in soil or sand, or air trapped among wet grains, does
    /// not hold pressure the way a boiler does.
    pub porous: bool,
    /// How the cells' turn moves it.
    pub motion: Motion,
    /// The fastest a lone cell of it falls through the air in the cells' turn, m/s (its drag
    /// balances gravity); 0: it does not fall there (a lone cell of oil or lava is the solver's).
    pub fall_ms: f32,
    /// Which ways it holds back what moves into it.
    pub holds: Holds,
}

/// No fracture to speak of: what loose matter, liquids and gases take (they never crack).
const NO_CRACK: f32 = 1.0e6;

impl MatSpec {
    /// The record of matter `m`.
    #[inline]
    pub fn of(m: Mat) -> &'static MatSpec {
        static T: std::sync::OnceLock<Vec<MatSpec>> = std::sync::OnceLock::new();
        &T.get_or_init(|| {
            let mut t = vec![record(Mat::Empty); Mat::ALL.len()];
            for m in Mat::ALL {
                t[m as usize] = record(m);
            }
            t
        })[m as usize]
    }
}

/// Air: a gas, a flame, the empty cell.
const AIR: MatSpec = MatSpec { density: RHO_AIR, strength: 0.0, toughness: NO_CRACK, holds_atm: None, rubble: None, restitution: 0.3, porous: false, motion: Motion::Fixed, fall_ms: 0.0, holds: Holds::Every };
/// A gas moving as `motion`.
const fn gas(motion: Motion) -> MatSpec {
    MatSpec { motion, ..AIR }
}
/// A liquid of density `rho`.
const fn liquid(rho: f32) -> MatSpec {
    MatSpec { density: rho, motion: Motion::Liquid, ..AIR }
}
/// Loose grains of density `rho`: they hold a little pressure by their weight.
const fn loose(rho: f32, porous: bool) -> MatSpec {
    MatSpec { density: rho, holds_atm: Some(0.6), porous, motion: Motion::Grain, fall_ms: GRAIN_TERMINAL_MS, ..AIR }
}
/// A solid.
const fn solid(density: f32, strength: f32, toughness: f32, holds_atm: Option<f32>, rubble: Option<Mat>, restitution: f32) -> MatSpec {
    MatSpec { density, strength, toughness, holds_atm, rubble, restitution, porous: false, motion: Motion::Fixed, fall_ms: 0.0, holds: Holds::Every }
}

fn record(m: Mat) -> MatSpec {
    use Mat::*;
    match m {
        Empty => AIR,
        Steam | Smoke => gas(Motion::Gas),
        Cloud => gas(Motion::Cloud),
        Fire => gas(Motion::Flame),
        // A raindrop falls at the speed of a grain.
        Water => MatSpec { fall_ms: DROP_TERMINAL_MS, ..liquid(1000.0) },
        Oil => liquid(800.0),
        Lava => liquid(3000.0),
        MoltenGlass => liquid(2400.0),
        Molten | MoltenSteel => liquid(6500.0),
        Sand | Mud => loose(1600.0, true),
        // Snowflakes catch the air and fall gently.
        Snow => MatSpec { fall_ms: SNOW_TERMINAL_MS, ..loose(300.0, true) },
        // Flakes: a flow runs through them sideways.
        DeadLeaf => MatSpec { holds: Holds::FromAbove, ..loose(300.0, true) },
        Seed => loose(300.0, true),
        Hail => loose(920.0, true),
        Shard => loose(2500.0, true),
        Rust | Gunpowder => loose(1500.0, true),
        Saltpeter => loose(2100.0, true),
        Ash => MatSpec { holds: Holds::FromAbove, ..loose(500.0, false) },
        Charcoal => MatSpec { holds: Holds::FromAbove, ..loose(300.0, false) },
        // Soils: their cohesion is what they hold (and gas seeps through them anyway).
        Dirt => MatSpec { strength: 0.05, holds_atm: Some(0.45), motion: Motion::Clod, ..loose(1600.0, true) },
        Grass => MatSpec { strength: 0.08, holds_atm: Some(0.75), motion: Motion::Turf, ..loose(300.0, true) },
        Clay => solid(1800.0, 0.15, 5.0, Some(1.4), Some(Dirt), 0.05),
        Stone => solid(2700.0, 10.0, 50.0, Some(5.0), Some(Sand), 0.3),
        Brick => solid(1200.0, 3.0, 30.0, Some(2.0), Some(Sand), 0.3),
        Terracotta => solid(2000.0, 4.0, 30.0, Some(3.0), Some(Sand), 0.3),
        Vault => solid(2000.0, 5.0, 50.0, None, Some(Sand), 0.3),
        Glass => solid(2500.0, 7.0, 8.0, Some(0.8), Some(Shard), 0.4),
        // Crushed ice is grains of ice: hail.
        Ice => solid(920.0, 1.5, 1.0, Some(0.6), Some(Hail), 0.35),
        // Diamond is hard but brittle: it breaks at about 2 GPa.
        Diamond => solid(3500.0, 2000.0, 20.0, None, Some(Diamond), 0.4),
        Iron => solid(7800.0, 250.0, 50_000.0, Some(8.0), None, 0.4),
        Steel => solid(7800.0, 400.0, 100_000.0, Some(14.0), None, 0.4),
        Wood => solid(600.0, 5.0, 8_000.0, Some(2.5), None, 0.45),
        // A tree stands in a plane of its own (`Holds::FromAbove`).
        Trunk => MatSpec { holds: Holds::FromAbove, ..solid(600.0, 8.0, 15_000.0, Some(2.5), None, 0.45) },
        Branch => MatSpec { holds: Holds::FromAbove, ..solid(600.0, 4.0, 8_000.0, None, None, 0.45) },
        Leaf | Sapling => MatSpec { holds: Holds::FromAbove, ..solid(300.0, 0.1, 500.0, None, None, 0.05) },
        // (A coal of burning wood weighs as a cell of water, as it always has here.)
        Ember => solid(1000.0, 0.5, 100.0, None, None, 0.3),
        // A stick of dynamite lying loose falls and rolls off a heap as a grain does.
        Dynamite => MatSpec { motion: Motion::Grain, fall_ms: GRAIN_TERMINAL_MS, ..solid(1400.0, 1.0, 300.0, None, None, 0.3) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::units::ATM;

    #[test]
    fn holds_no_more_than_its_strength() {
        for m in Mat::ALL {
            let s = MatSpec::of(m);
            if let (Some(p), true) = (s.holds_atm, s.strength > 0.0) {
                assert!(p <= s.strength * 1.0e6 / ATM, "{m:?}: a wall of it holds {p} atm, more than its strength");
            }
        }
    }

    #[test]
    fn what_crumbles_crumbles_into_something_that_does_not_crumble_further() {
        for m in Mat::ALL {
            if let Some(r) = MatSpec::of(m).rubble {
                assert!(r == m || MatSpec::of(r).rubble.is_none(), "{m:?} -> {r:?} -> {:?}", MatSpec::of(r).rubble);
            }
        }
    }
}
