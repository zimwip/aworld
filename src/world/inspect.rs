//! What the simulation holds, read as it is for the simulation view: the velocity each kind of
//! matter has in its own model, the pressure fields on their own grids, and the state of a cell.
//!
//! Nothing here changes the world. Each reading says where it comes from, because the kinds of
//! matter do not share one model:
//! - a liquid moving by its physics has a velocity on the edges of its cells (`liquid.rs`), in m/s;
//!   otherwise (particles), like a grain, it only keeps a falling speed (`vel`, eighths of a cell
//!   a step), or the sideways speed of a jet (`jet`, cells a step) while it flies out of one;
//! - a grain slides down a heap from cell to cell without any speed: only its fall has one;
//! - a gas (and a flame) has no velocity of its own: it rides the air's flow (`flow.rs`), 8 × 8
//!   cells to a flow cell;
//! - a solid stands still, but for the pieces that fall (`drop_structures`, every other step)
//!   and the slabs that tip over (`statics.rs`, a rigid body with a spin).

use super::flow::{CELL as FLOW_CELL, FH, FW};
use super::liquid::{CELL_M, DT};
use super::*;
use std::collections::HashMap;

/// Units of liquid in a full cell.
pub const FULL: u32 = super::field::FULL;

/// A cell, in metres.
pub const CELL_METRES: f32 = CELL_M;

/// A cell a step, in m/s.
pub const CELL_STEP_MS: f32 = CELL_M / DT;

/// Which model a velocity comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// The liquids' grid of velocities (`liquid.rs`).
    LiquidGrid,
    /// The falling speed of a grain or a drop of the cells' turn (`vel`).
    Fall,
    /// The air's flow (`flow.rs`).
    Wind,
    /// A falling piece of a structure.
    Piece,
    /// A slab tipping over or tumbling.
    Slab,
    /// A drop of the cells' liquids shot out of a jet (`jet`): it keeps its sideways speed as it
    /// falls a cell a step.
    Jet,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::LiquidGrid => "grille des liquides (liquid.rs)",
            Source::Fall => "vitesse de chute (vel)",
            Source::Wind => "flux de l'air (flow.rs)",
            Source::Piece => "morceau qui tombe",
            Source::Slab => "dalle, corps rigide (statics.rs)",
            Source::Jet => "jet sous pression (jet, pockets.rs)",
        }
    }
}

/// The velocity of a cell, m/s (down positive), and where it comes from.
#[derive(Clone, Copy, Debug)]
pub struct Motion {
    pub vx: f32,
    pub vy: f32,
    pub source: Source,
}

/// The pressure fields the simulation works with, each on its own grid and in its own unit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    /// The pressure of the liquids moving by their physics, over the ambient one, Pa, in each
    /// cell that holds one (`liquid.rs`, solved by `mgpcg.rs`).
    LiquidPa,
    /// The pressure inside the liquids of the cells' turn, absolute, atm (`pockets.rs`).
    LiquidAtm,
    /// The air's pressure over the ambient one, atm, one value a flow cell (`flow.rs`).
    Air,
}

impl World {
    /// The falling pieces and the tumbling slabs, by the cells they cover, with their velocity
    /// (m/s, down positive): what `cell_motion` reads for a solid. Built once for a picture.
    pub fn solid_motions(&self) -> HashMap<usize, Motion> {
        let mut out = HashMap::new();
        // A piece moves `vy` cells every other step.
        for p in &self.falling {
            let m = Motion { vx: 0.0, vy: p.vy * 0.5 * CELL_STEP_MS, source: Source::Piece };
            for &i in &p.cells {
                out.insert(i, m);
            }
        }
        for s in &self.slabs {
            for &i in s.cells() {
                let (vx, vy) = s.velocity_at({ let (x, y) = xy(i); (x as f32 + 0.5, y as f32 + 0.5) });
                out.insert(i, Motion { vx: vx * CELL_STEP_MS, vy: vy * CELL_STEP_MS, source: Source::Slab });
            }
        }
        out
    }

    /// The velocity of cell (x, y) in the model its matter moves by (see the top of the file);
    /// None for a solid that stands, or a liquid before the liquids' first step.
    /// `solids` is `solid_motions`.
    pub fn cell_motion(&self, x: usize, y: usize, solids: &HashMap<usize, Motion>) -> Option<Motion> {
        let i = y * W() + x;
        let m = self.cells[i].m;
        let fall = |v: u8| (v != 0).then(|| Motion { vx: 0.0, vy: v as f32 / super::FALL_UNIT as f32 * CELL_STEP_MS, source: Source::Fall });
        // A drop shot out of a jet flies a cell down and `v` cells sideways a step (see `fly`).
        let jet = self.jet[i];
        if m.kind() == Kind::Liquid && jet >= 2 {
            let v = (jet >> 1) as f32 * if jet & 1 == 1 { 1.0 } else { -1.0 };
            return Some(Motion { vx: v * CELL_STEP_MS, vy: CELL_STEP_MS, source: Source::Jet });
        }
        match m.kind() {
            Kind::Liquid => {
                let class = self.liq.class.get(i)?;
                if class & 3 < liquid::WET {
                    // A lone drop the liquids leave to the cells' turn.
                    return fall(self.vel[i]);
                }
                let (vx, vy) = self.liquid_velocity(x, y)?;
                Some(Motion { vx, vy, source: Source::LiquidGrid })
            }
            Kind::Powder => fall(self.vel[i]),
            Kind::Empty | Kind::Gas | Kind::Flame => {
                let (vx, vy) = self.flow.sample(x as i32, y as i32);
                Some(Motion { vx: vx * CELL_STEP_MS, vy: vy * CELL_STEP_MS, source: Source::Wind })
            }
            Kind::Static => solids.get(&i).copied(),
        }
    }

    /// The value of `field` at cell (x, y), in its own unit (see `Field`); None where the field
    /// has no value (a liquid's pressure outside the liquid, the air's in the ground).
    pub fn field_value(&self, field: Field, x: usize, y: usize) -> Option<f32> {
        let i = y * W() + x;
        let m = self.cells[i].m;
        let (fi, fj) = ((x / FLOW_CELL).min(FW() - 1), (y / FLOW_CELL).min(FH() - 1));
        match field {
            Field::LiquidPa => self.liquid_pa(x, y),
            Field::LiquidAtm => (m.kind() == Kind::Liquid).then(|| self.liq_p[i]),
            Field::Air => (matches!(m.kind(), Kind::Empty | Kind::Gas | Kind::Flame) && !self.flow.is_solid(fi, fj) && self.flow.compress).then(|| self.flow.excess_at_cell(fi, fj)),
        }
    }

    /// The pocket of gas at (x, y): its number, how many cells, its pressure (atm), and whether it
    /// is open to the sky.
    pub fn pocket_info(&self, x: usize, y: usize) -> Option<(usize, u32, f32, bool)> {
        let k = self.pocket_at(x as i32, y as i32)?;
        self.pockets.get(k).map(|p| (k, p.cells, p.pressure, p.open))
    }

    /// The units of liquid in cell (x, y) (`field::FULL` a full cell); None if it is no liquid.
    pub fn cell_fill(&self, x: usize, y: usize) -> Option<u32> {
        let i = y * W() + x;
        (self.cells[i].m.kind() == Kind::Liquid).then(|| self.water_fill(i))
    }

    /// The falling speed a grain or a drop keeps (`vel`, in `FALL_UNIT`ths of a cell a tick).
    pub fn cell_fall(&self, x: usize, y: usize) -> u8 {
        self.vel[y * W() + x]
    }

    /// The clods thrown out of craters, in flight: where they are (cells) and their velocity (m/s).
    pub fn ejecta_motion(&self) -> impl Iterator<Item = (f32, f32, f32, f32)> + '_ {
        self.ejecta.iter().map(|e| (e.x, e.y, e.vx * CELL_STEP_MS, e.vy * CELL_STEP_MS))
    }

    /// Steps since the world began (the step counter of the step-by-step mode).
    pub fn steps(&self) -> u32 {
        self.ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_reads_its_own_velocity() {
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            w.set(x, 300, Mat::Stone);
        }
        w.set(100, 100, Mat::Sand);
        w.flow.pin_uniform(0.5, 0.0);
        w.set(400, 150, Mat::Smoke);
        for _ in 0..6 {
            w.step();
        }
        let solids = w.solid_motions();
        let y = (0..H()).find(|&y| w.cell_mat(100, y) == Mat::Sand).expect("the grain");
        let grain = w.cell_motion(100, y, &solids).expect("a falling grain has a speed");
        assert_eq!(grain.source, Source::Fall);
        assert!(grain.vy > 0.0 && grain.vx == 0.0, "{grain:?}");
        // The air and a gas ride the flow: half a cell a step is 5 m/s.
        let air = w.cell_motion(700, 50, &solids).expect("the air");
        assert_eq!(air.source, Source::Wind);
        assert!((air.vx - 0.5 * CELL_STEP_MS).abs() < 0.01 && air.vy.abs() < 0.01, "{air:?}");
        // The ground stands still.
        assert!(w.cell_motion(50, 300, &solids).is_none());
    }

    #[test]
    fn a_drop_out_of_a_jet_keeps_its_sideways_speed() {
        let mut w = World::new_static();
        w.set(300, 100, Mat::Water);
        // Three cells a step to the left.
        w.jet[100 * W() + 300] = 3 << 1;
        let m = w.cell_motion(300, 100, &HashMap::new()).expect("a flying drop");
        assert_eq!(m.source, Source::Jet);
        assert!((m.vx + 3.0 * CELL_STEP_MS).abs() < 1e-3 && (m.vy - CELL_STEP_MS).abs() < 1e-3, "{m:?}");
    }

    #[test]
    fn the_fields_have_values_only_where_they_are_defined() {
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            w.set(x, 300, Mat::Stone);
        }
        for y in 280..300 {
            for x in 200..240 {
                w.set(x, y, Mat::Water);
            }
        }
        w.update_pockets();
        // The weight of the water, as the pockets work it out (what sets its boiling point): one
        // atmosphere at the surface, more under it.
        let (top, deep) = (w.field_value(Field::LiquidAtm, 220, 280).unwrap(), w.field_value(Field::LiquidAtm, 220, 299).unwrap());
        assert!(top >= 1.0 && deep > top, "{top} {deep}");
        assert!(w.field_value(Field::LiquidAtm, 220, 270).is_none());
        assert!(w.field_value(Field::LiquidPa, 220, 290).is_none(), "the liquids' own pressure only once they have taken a step");
        // The open air is a pocket at one atmosphere; the ground is in no pocket.
        assert_eq!(w.pocket_info(220, 100).map(|p| (p.2, p.3)), Some((1.0, true)));
        assert!(w.pocket_info(220, 300).is_none());
        assert!(w.field_value(Field::Air, 220, 300).is_none(), "no air in the ground");
    }
}
