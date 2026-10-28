//! When the passes that do not run every tick run (`World::after_cells`, the air's flow, the
//! storm): every pass has a period, in seconds of the world, and a phase, the tick of its period
//! it falls on. They are set side by side here so that the heavy ones never fall on the same
//! tick, and a test holds them to it (`APART`): a tick costs about the same as the next, instead
//! of one tick in a few carrying everything.

use super::units::TICK;

/// A pass that comes back now and then.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pass {
    /// What holds and what falls (`structure_pass`).
    Structures,
    /// The falling pieces drop a row (`drop_structures`).
    Drops,
    /// The pockets of gas and the pressure of the liquids are counted again (`update_pockets`).
    Pockets,
    /// The lasting changes of matter (`settle_matter`).
    Settle,
    /// The air's pressure (`update_pressure`).
    Pressure,
    /// The colours of the season (`update_season_cache`).
    Season,
    /// The storm's charges and lightning (`storm_step`).
    Storm,
    /// The air's flow learns again where the ground is (`refresh_flow_solids`).
    FlowSolids,
    /// The air's flow learns again how warm and how damp the air is.
    FlowHeat,
    /// The air's flow moves on.
    FlowStep,
}

/// How often a pass comes back, seconds, and on which tick of its period.
struct Cadence {
    every_s: f32,
    phase: u32,
}

fn cadence(p: Pass) -> Cadence {
    let c = |every_s: f32, phase: u32| Cadence { every_s, phase };
    match p {
        // On 9k + 1: always beside space and the sun, the lightest part of the heat cycle.
        Pass::Structures => c(0.15, 1),
        Pass::Drops => c(2.0 * TICK, 0),
        // On 18k + 5: never with the structures, and on an odd tick (with the drops of the
        // pieces, the siphons stopped levelling). A multiple of the heat cycle, so that it always
        // falls on the same part of it.
        Pass::Pockets => c(0.3, 5),
        // (About a second. A second even, 60 ticks, changed how fast soil turns to mud under
        // water enough to undo a test of erosion that sits on its edge: kept as it was.)
        Pass::Settle => c(64.0 * TICK, 17),
        Pass::Pressure => c(4.0 * TICK, 1),
        Pass::Season => c(64.0 * TICK, 0),
        Pass::Storm => c(4.0 * TICK, 0),
        Pass::FlowSolids => c(16.0 * TICK, 1),
        Pass::FlowHeat => c(4.0 * TICK, 1),
        Pass::FlowStep => c(2.0 * TICK, 0),
    }
}

/// The period of pass `p`, ticks.
pub fn every(p: Pass) -> u32 {
    (cadence(p).every_s / TICK).round() as u32
}

/// Whether pass `p` runs on tick `tick`.
#[inline]
pub fn due(p: Pass, tick: u32) -> bool {
    tick % every(p) == cadence(p).phase
}

/// The passes that must never fall on the same tick.
#[cfg(test)]
const APART: [(Pass, Pass); 3] = [(Pass::Structures, Pass::Pockets), (Pass::Pockets, Pass::Drops), (Pass::Settle, Pass::Season)];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::thermal::THERMAL_EVERY;

    #[test]
    fn the_heavy_passes_never_fall_on_the_same_tick() {
        for (a, b) in APART {
            // Over a whole common period of the two, every tick.
            let n = every(a) * every(b);
            assert!((0..n).all(|t| !(due(a, t) && due(b, t))), "{a:?} and {b:?} fall together");
        }
    }

    /// The structures and the pockets always fall on the same part of the heat cycle.
    #[test]
    fn the_structures_and_the_pockets_keep_their_place_in_the_heat_cycle() {
        for p in [Pass::Structures, Pass::Pockets] {
            assert_eq!(every(p) % THERMAL_EVERY, 0, "{p:?}");
        }
    }

    #[test]
    fn every_pass_falls_within_its_period() {
        use Pass::*;
        for p in [Structures, Drops, Pockets, Settle, Pressure, Season, Storm, FlowSolids, FlowHeat, FlowStep] {
            assert!(every(p) >= 1 && cadence(p).phase < every(p), "{p:?}");
        }
    }
}
