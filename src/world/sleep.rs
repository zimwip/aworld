//! Sleeping zones. The world is cut into blocks of `CS` x `CS` cells. A block where nothing
//! changed during the last step, and where no cell can change on its own (rock, settled sand,
//! dry soil, water at rest under more water...), is asleep: the cells of the next step skip it.
//! Any change to a cell (`set`, `swap`, a falling piece, a rigid block moving) wakes its block, and
//! the neighbouring block too when the cell is on the edge. Heat, wind and pressure are not
//! affected: they keep running everywhere.

use super::*;

/// Side of a sleeping zone, in cells.
pub const CS: usize = 32;
/// Zones across and down.
pub const CW: usize = W.div_ceil(CS);
pub const CH: usize = H.div_ceil(CS);

/// The state of the zones: which run this step, which were woken for the next one, and which
/// hold a cell that can change on its own.
pub struct Zones {
    pub(super) awake: Vec<bool>,
    woke: Vec<bool>,
    restless: Vec<bool>,
    /// When false every zone always runs (to compare, and for the odd test).
    pub enabled: bool,
}

impl Zones {
    pub fn new() -> Self {
        Zones { awake: vec![true; CW * CH], woke: vec![true; CW * CH], restless: vec![true; CW * CH], enabled: true }
    }

    /// Wakes every zone (after the whole grid was replaced).
    pub fn wake_all(&mut self) {
        self.woke.fill(true);
    }

    /// Wakes the zone of cell `i`, and the zone next door when the cell is on its edge.
    #[inline]
    pub fn wake(&mut self, i: usize) {
        let (x, y) = (i % W, i / W);
        let (cx, cy) = (x / CS, y / CS);
        let (lx, ly) = (x % CS, y % CS);
        let x0 = if lx == 0 && cx > 0 { cx - 1 } else { cx };
        let x1 = if lx == CS - 1 && cx + 1 < CW { cx + 1 } else { cx };
        let y0 = if ly == 0 && cy > 0 { cy - 1 } else { cy };
        let y1 = if ly == CS - 1 && cy + 1 < CH { cy + 1 } else { cy };
        for zy in y0..=y1 {
            for zx in x0..=x1 {
                self.woke[zy * CW + zx] = true;
            }
        }
        // Gas wraps round the sides of the world.
        if x == 0 {
            self.woke[cy * CW + CW - 1] = true;
        } else if x == W - 1 {
            self.woke[cy * CW] = true;
        }
    }

    /// Starts a step: the zones woken during the last one, or holding a restless cell, run.
    pub(super) fn begin(&mut self) {
        for k in 0..CW * CH {
            self.awake[k] = !self.enabled || self.woke[k] || self.restless[k];
        }
        self.woke.fill(false);
        self.restless.fill(false);
    }

    #[inline]
    pub(super) fn mark_restless(&mut self, zone: usize) {
        self.restless[zone] = true;
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
            Mat::Dirt => c.life > 0,
            Mat::Grass => c.life > 0 || !self.at(x, y + 1).is_hard(),
            Mat::Sand | Mat::Rust | Mat::Hail | Mat::Snow | Mat::Gunpowder | Mat::Ash => {
                let can_fall = sinks_into(self.at(x, y + 1)) || sinks_into(self.at(x - 1, y + 1)) || sinks_into(self.at(x + 1, y + 1));
                let wet = c.m == Mat::Ash && Self::neighbors(x, y).iter().any(|&(nx, ny)| self.at(nx, ny) == Mat::Water);
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
                c.life >= 2
                    || free(self.at(x, y - 1))
                    || matches!(self.at(x, y + 1), Mat::Dirt | Mat::Grass)
                    || self.at(x, y + 1).is_vegetation()
                    || free(self.at(x, y + 1))
                    || free(self.at(x - 1, y + 1))
                    || free(self.at(x + 1, y + 1))
                    || free(self.at(x - 1, y))
                    || free(self.at(x + 1, y))
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
        for x in 0..W as i32 {
            for y in 400..H as i32 {
                w.set(x, y, Mat::Stone);
            }
            for y in 380..400 {
                w.set(x, y, Mat::Sand);
            }
        }
        for _ in 0..40 {
            w.step();
        }
        w.zones.begin();
        assert_eq!(w.zones.awake_count(), 0, "nothing can move: everything sleeps");
        // A grain dropped from above wakes its zone, which runs again.
        w.set(300, 100, Mat::Sand);
        w.zones.begin();
        assert!(w.zones.awake[(100 / CS) * CW + 300 / CS]);
        assert!(w.zones.awake_count() <= 4);
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
            let water_top = (0..H as i32).find(|&y| (101..399).any(|x| w.raw(x, y) == Mat::Water)).unwrap();
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
