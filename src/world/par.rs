//! Helpers to run a pass over the grid on every core. Rows are easy to share out (each is its
//! own slice); a pass that runs down the columns (each cell after the one above it) is shared
//! out in bands of columns, through `Shared`.

use super::W;
use rayon::prelude::*;

/// Columns per band.
pub const COLUMN_BAND: usize = 64;

/// A grid that several threads write at once, each in cells no other thread reads or writes.
pub struct Shared<T>(*mut T, usize);

// SAFETY: the callers of `at` promise that no two threads reach the same cell.
unsafe impl<T: Send> Sync for Shared<T> {}

impl<T> Shared<T> {
    pub fn new(grid: &mut [T]) -> Self {
        Shared(grid.as_mut_ptr(), grid.len())
    }

    /// Cell `i`.
    ///
    /// # Safety
    /// No other thread may reach cell `i` while the reference lives.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn at(&self, i: usize) -> &mut T {
        debug_assert!(i < self.1);
        &mut *self.0.add(i)
    }

    /// A copy of cell `i`.
    ///
    /// # Safety
    /// No other thread may write cell `i` at the same time.
    #[inline]
    pub unsafe fn get(&self, i: usize) -> T
    where
        T: Copy,
    {
        debug_assert!(i < self.1);
        *self.0.add(i)
    }
}

/// Runs `f` on every band of columns (`x0..x1`), side by side.
pub fn column_bands(f: impl Fn(std::ops::Range<usize>) + Sync) {
    (0..W.div_ceil(COLUMN_BAND)).into_par_iter().for_each(|b| f(b * COLUMN_BAND..((b + 1) * COLUMN_BAND).min(W)));
}
