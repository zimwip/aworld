//! One record per matter (`MatSpec`): what it weighs, how strong it is, what it crumbles into.
//! Every model that needs one of these reads it here: the falling and the floating (density), the
//! blasts (density, strength, rubble), the structures (toughness, rubble, restitution), the
//! pockets of gas (the pressure a wall holds, whether gas seeps through) and the weight of matter
//! on the ground. These were tables of their own in each model, and did not always agree: crushed
//! ice was hail under a falling block and snow under a blast or a burst boiler, crushed clay was
//! earth or nothing, and a wall of glass, as strong as 7 MPa for a blast, held 0.8 atm in a
//! boiler with nothing to say how both could be.
//!
//! The match in `of` has no catch-all arm: a new matter does not compile until it has its record.

use super::units::RHO_AIR;
use super::Mat;

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
const AIR: MatSpec = MatSpec { density: RHO_AIR, strength: 0.0, toughness: NO_CRACK, holds_atm: None, rubble: None, restitution: 0.3, porous: false };
/// A liquid of density `rho`.
const fn liquid(rho: f32) -> MatSpec {
    MatSpec { density: rho, ..AIR }
}
/// Loose grains of density `rho`: they hold a little pressure by their weight.
const fn loose(rho: f32, porous: bool) -> MatSpec {
    MatSpec { density: rho, holds_atm: Some(0.6), porous, ..AIR }
}
/// A solid.
const fn solid(density: f32, strength: f32, toughness: f32, holds_atm: Option<f32>, rubble: Option<Mat>, restitution: f32) -> MatSpec {
    MatSpec { density, strength, toughness, holds_atm, rubble, restitution, porous: false }
}

fn record(m: Mat) -> MatSpec {
    use Mat::*;
    match m {
        Empty | Fire | Steam | Cloud | Smoke => AIR,
        Water => liquid(1000.0),
        Oil => liquid(800.0),
        Lava => liquid(3000.0),
        Molten | MoltenSteel => liquid(6500.0),
        Sand | Mud => loose(1600.0, true),
        Snow | DeadLeaf | Seed => loose(300.0, true),
        Hail => loose(920.0, true),
        Shard => loose(2500.0, true),
        Rust | Gunpowder => loose(1500.0, true),
        Saltpeter => loose(2100.0, true),
        Ash => loose(500.0, false),
        Charcoal => loose(300.0, false),
        // Soils: their cohesion is what they hold (and gas seeps through them anyway).
        Dirt => MatSpec { strength: 0.05, holds_atm: Some(0.45), ..loose(1600.0, true) },
        Grass => MatSpec { strength: 0.08, holds_atm: Some(0.75), ..loose(300.0, true) },
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
        Trunk => solid(600.0, 8.0, 15_000.0, Some(2.5), None, 0.45),
        Branch => solid(600.0, 4.0, 8_000.0, None, None, 0.45),
        Leaf | Sapling => solid(300.0, 0.1, 500.0, None, None, 0.05),
        // (A coal of burning wood weighs as a cell of water, as it always has here.)
        Ember => solid(1000.0, 0.5, 100.0, None, None, 0.3),
        Dynamite => solid(1400.0, 1.0, 300.0, None, None, 0.3),
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
