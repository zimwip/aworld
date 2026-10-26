//! How much of a cell a liquid fills, when the liquids move by their physics (`liquid.rs`): a
//! cell of liquid may then hold less than a full cell (`fill`, `FULL` for a full one). With the
//! liquids made of cells, every cell of liquid is full.

use super::*;

/// A full cell of liquid, in units of `fill`: finer than `WATER_UNITS`, so that a thin film is
/// counted exactly enough.
pub const FULL: u32 = 1 << 16;

impl World {
    /// Units of water in cell `i` (a cell of water; `FULL` unless the physics says otherwise).
    #[inline]
    pub fn water_fill(&self, i: usize) -> u32 {
        match self.fill[i] {
            0 => FULL,
            f => f,
        }
    }

    /// Water in cell `i` in `WATER_UNITS`, what the ground drinks of it.
    #[inline]
    pub fn water_units_in(&self, i: usize) -> u16 {
        (self.water_fill(i) as u64 * WATER_UNITS as u64 / FULL as u64) as u16
    }

    #[inline]
    pub(super) fn put_fill(&mut self, i: usize, q: u32) {
        self.fill[i] = if q == FULL { 0 } else { q };
    }
}
