//! The pressure solver of the liquids: conjugate gradients, preconditioned by a geometric
//! multigrid (MGPCG, after McAdams, Sifakis and Teran, 2010).
//!
//! The pressure of the liquids is a Poisson problem on the cells of liquid that run: in each,
//! the flows through its four sides must add up to what brings it back to full; the pressure is
//! fixed where the liquid meets a gas (the pocket's pressure), and nothing flows through a wall.
//! Gauss-Seidel sweeps move a pressure one cell a sweep: a sea 500 cells wide asked hundreds of
//! sweeps that the step never had time for, so the pressure was never quite right (the body
//! slowly swelled or shrank, and a `REFILL` term had to make up for it).
//!
//! Here the problem is solved on a hierarchy of grids, each half as fine as the one before: the
//! coarse grids carry a correction across the whole body in a few sweeps, the fine ones polish
//! it. Conjugate gradients round the multigrid make it robust to the uneven coefficients (oil on
//! water, lava), and the warm start from the last sub-step's pressure means a body at rest is
//! done in one iteration.
//!
//! The operator is written in conductances: between two cells of liquid 2 / (ρ_i + ρ_j), between
//! liquid and gas 1 / ρ_i, nothing through a wall; the diagonal is the sum. A coarse edge takes
//! the mean of the two fine edges that cross its face (in two dimensions the conductance of a
//! square does not depend on its size), the residual is summed into the coarse cell and the
//! correction copied back to its four cells: the coarse operator is then the fine one seen from
//! afar. A coarse cell only stands for fine cells that hang together inside it (`Level::parts`):
//! one laid across a thin ridge would join the pools on each side, and a splash in one would
//! throw the other about (the solve stops at a tolerance, well before what leaked has gone).
//!
//! The cost is in the passes over the fine grid, so there are as few as can be: each level packs
//! its fluid cells into two lists by colour, with their coefficients, that a sweep streams
//! through (twice as fast as walking the grid); the residual is summed into the coarse grid in
//! the pass that computes it, and only for the colour the last sweep left inexact; the coarse
//! correction is folded into the first sweep after the visit instead of a pass of its own; the
//! conjugate gradients fuse their vector operations two by two and take the dot product they
//! need from the last sweep of the cycle. All the sums run per chunk then in a fixed order, so
//! the answer is the same whatever the number of cores.

use super::par::Shared;
use rayon::prelude::*;

/// What a cell is to the solver.
pub const NONE: u8 = 0;
/// An unknown pressure.
pub const FLUID: u8 = 1;
/// A fixed pressure (gas): zero in the corrected system, its value having gone into the right side.
pub const FIXED: u8 = 2;

/// Grids this small are solved by sweeping alone.
const COARSEST: usize = 1024;
/// Sweeps on the coarsest grid.
const COARSEST_SWEEPS: usize = 24;
/// Red-black sweeps before and after the visit to the coarser grid.
const SMOOTH: usize = 2;
/// The colour of the last pass before the visit (the sweeps run red then black).
const LAST_COLOUR: usize = 1;
/// Lists of cells this short are swept on one core: handing them out costs more than the cells.
const SMALL: usize = 4096;
/// Cells a core takes at a time.
const CHUNK: usize = 512;
/// Grids this small are built and cleared on one core.
const BIG: usize = 100_000;

/// A pass over the rows of a grid `buf` of width `w`, side by side if `par`.
macro_rules! rows_mut {
    ($buf:expr, $w:expr, $par:expr, $f:expr) => {
        if $par {
            $buf.par_chunks_mut($w).enumerate().for_each($f)
        } else {
            $buf.chunks_mut($w).enumerate().for_each($f)
        }
    };
}

/// A fluid cell as a sweep sees it: p = (b + Σ a_k p_k) · inv over its four neighbours (left,
/// right, up, down; a_k is 0 where there is none).
#[derive(Clone, Copy, Default)]
struct Node {
    /// The cell.
    i: u32,
    /// The coarse cell over it.
    ci: u32,
    a: [f32; 4],
    /// 1 / diag.
    inv: f32,
    /// Bits 0..4: the neighbour is fluid (it takes the coarse correction); 4: x odd; 5: y odd.
    bits: u8,
}

/// One grid of the hierarchy.
pub struct Level {
    pub w: usize,
    pub h: usize,
    /// What each cell is (`NONE`, `FLUID`, `FIXED`).
    pub kind: Vec<u8>,
    /// Conductance of the edge between (x - 1, y) and (x, y): index y * (w + 1) + x.
    pub ax: Vec<f32>,
    /// Conductance of the edge between (x, y - 1) and (x, y): index y * w + x, h + 1 rows.
    pub ay: Vec<f32>,
    /// Sum of the conductances of each fluid cell.
    pub diag: Vec<f32>,
    /// The fluid cells by colour, with what a sweep needs, packed: a sweep streams through them.
    nodes: [Vec<Node>; 2],
    /// The rows that hold fluid, as (first, last) cells; `None` for a row without.
    pub rows: Vec<Option<(usize, usize)>>,
    /// The right side and the solution of the level (the residual and the correction, below the
    /// finest one).
    pub b: Vec<f32>,
    pub x: Vec<f32>,
    /// Below the finest level, the fine cells each cell stands for: bit 2 dy + dx for the fine
    /// fluid cell (2 i + dx, 2 j + dy). The fine fluid cells of a coarse one must hang together
    /// through the edges inside it, or the coarse cell would join what does not touch (two pools
    /// on each side of a thin ridge, cut by the same coarse cell): the largest group that hangs
    /// together is taken, and the others are left out of the coarse grid (their residual is not
    /// passed down, no correction comes back up to them; the sweeps alone see to them).
    parts: Vec<u8>,
    /// Whether some fine fluid cell was left out (see `parts`).
    split: bool,
}

impl Level {
    fn new(w: usize, h: usize) -> Level {
        Level {
            w,
            h,
            kind: vec![NONE; w * h],
            ax: vec![0.0; (w + 1) * h],
            ay: vec![0.0; w * (h + 1)],
            diag: vec![0.0; w * h],
            nodes: [Vec::new(), Vec::new()],
            rows: vec![None; h],
            b: vec![0.0; w * h],
            x: vec![0.0; w * h],
            parts: vec![0; w * h],
            split: false,
        }
    }

    #[inline]
    fn axi(&self, x: usize, y: usize) -> usize {
        y * (self.w + 1) + x
    }

    /// The span of row `y` widened by a cell on each side, within the grid.
    #[inline]
    fn widen(w: usize, span: Option<(usize, usize)>) -> Option<(usize, usize)> {
        span.map(|(a, b)| (a.saturating_sub(1), (b + 1).min(w - 1)))
    }

    /// The union of two spans.
    #[inline]
    fn union(a: Option<(usize, usize)>, b: Option<(usize, usize)>) -> Option<(usize, usize)> {
        match (a, b) {
            (Some((a, b)), Some((c, d))) => Some((a.min(c), b.max(d))),
            (s, None) | (None, s) => s,
        }
    }

    /// Fills the coarse level from the fine one `f`, over the rows the fine ones fall in (and
    /// a cell around them, as the fine rows hold a cell around the liquid): a cell is fixed if
    /// any of its four is, else fluid if any is, else nothing; an edge takes the mean of the two
    /// fine edges crossing it (where both sides are something and one at least is fluid). A fluid
    /// cell with no edge at all is nothing. The level must be clean (see `clear`).
    fn coarsen_from(&mut self, f: &Level) {
        let (w, h) = (self.w, self.h);
        let fw = f.w;
        // The rows: the fine rows two by two, then a row and a cell around.
        let halves: Vec<Option<(usize, usize)>> = (0..h)
            .map(|j| {
                let s = Self::union(f.rows[2 * j], f.rows.get(2 * j + 1).copied().flatten());
                s.map(|(a, b)| (a / 2, b / 2))
            })
            .collect();
        for j in 0..h {
            let s = Self::union(Self::union(halves[j], if j > 0 { halves[j - 1] } else { None }), halves.get(j + 1).copied().flatten());
            self.rows[j] = Self::widen(w, s);
        }
        let (rows, big) = (&self.rows, self.big());
        rows_mut!(self.kind, w, big, |(j, row): (usize, &mut [u8])| {
            let Some((i0, i1)) = rows[j] else { return };
            for i in i0..=i1 {
                let mut kind = NONE;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let (x, y) = (2 * i + dx, 2 * j + dy);
                        if x < fw && y < f.h {
                            kind = kind.max(f.kind[y * fw + x]);
                        }
                    }
                }
                row[i] = kind;
            }
        });
        // The fine fluid cells of each coarse one that hang together (see `parts`).
        let split = std::sync::atomic::AtomicBool::new(false);
        rows_mut!(self.parts, w, big, |(j, row): (usize, &mut [u8])| {
            let Some((i0, i1)) = rows[j] else { return };
            for i in i0..=i1 {
                let (x, y) = (2 * i, 2 * j);
                let fluid = |dx: usize, dy: usize| x + dx < fw && y + dy < f.h && f.kind[(y + dy) * fw + x + dx] == FLUID;
                let mut cells = 0u8;
                for k in 0..4 {
                    if fluid(k & 1, k >> 1) {
                        cells |= 1 << k;
                    }
                }
                // The four edges inside: top, bottom, left, right.
                let links = [
                    (0, 1, cells & 3 == 3 && f.ax[f.axi(x + 1, y)] > 0.0),
                    (2, 3, cells >> 2 == 3 && f.ax[f.axi(x + 1, y + 1)] > 0.0),
                    (0, 2, cells & 5 == 5 && f.ay[(y + 1) * fw + x] > 0.0),
                    (1, 3, cells & 10 == 10 && f.ay[(y + 1) * fw + x + 1] > 0.0),
                ];
                let (mut best, mut seen) = (0u8, 0u8);
                for k in 0..4 {
                    if cells >> k & 1 == 0 || seen >> k & 1 != 0 {
                        continue;
                    }
                    let mut group = 1u8 << k;
                    loop {
                        let mut more = group;
                        for &(a, b, on) in &links {
                            if on && (group >> a & 1 != 0 || group >> b & 1 != 0) {
                                more |= 1 << a | 1 << b;
                            }
                        }
                        if more == group {
                            break;
                        }
                        group = more;
                    }
                    seen |= group;
                    if group.count_ones() > best.count_ones() {
                        best = group;
                    }
                }
                if best != cells {
                    split.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                row[i] = best;
            }
        });
        self.split = split.into_inner();
        let (kind, parts) = (&self.kind, &self.parts);
        let any_fluid = |a: usize, b: usize| kind[a] != NONE && kind[b] != NONE && (kind[a] == FLUID || kind[b] == FLUID);
        // Whether the fine cell `k` (bit 2 dy + dx) of coarse cell `c` is in it: a fine edge
        // counts in a coarse one only between cells in their coarse cells.
        let within = |c: usize, k: usize| kind[c] != FLUID || parts[c] >> k & 1 != 0;
        rows_mut!(self.ax, w + 1, big, |(j, row): (usize, &mut [f32])| {
            let Some((i0, i1)) = rows[j] else { return };
            for i in i0.max(1)..=(i1 + 1).min(w - 1) {
                let (a, b) = (j * w + i - 1, j * w + i);
                row[i] = if any_fluid(a, b) {
                    let mut s = 0.0;
                    for dy in 0..2 {
                        let y = 2 * j + dy;
                        if y < f.h && 2 * i < fw && within(a, 2 * dy + 1) && within(b, 2 * dy) {
                            s += f.ax[f.axi(2 * i, y)];
                        }
                    }
                    0.5 * s
                } else {
                    0.0
                };
            }
        });
        rows_mut!(self.ay, w, big, |(j, row): (usize, &mut [f32])| {
            if j == 0 || j == h {
                return;
            }
            let Some((i0, i1)) = Self::union(rows[j - 1], rows[j]) else { return };
            for i in i0..=i1 {
                let (a, b) = ((j - 1) * w + i, j * w + i);
                row[i] = if any_fluid(a, b) {
                    let mut s = 0.0;
                    for dx in 0..2 {
                        let x = 2 * i + dx;
                        if x < fw && 2 * j < f.h + 1 && within(a, 2 + dx) && within(b, dx) {
                            s += f.ay[2 * j * fw + x];
                        }
                    }
                    0.5 * s
                } else {
                    0.0
                };
            }
        });
        let (ax, ay) = (&self.ax, &self.ay);
        let kinds = Shared::new(&mut self.kind);
        rows_mut!(self.diag, w, big, |(j, row): (usize, &mut [f32])| {
            let Some((i0, i1)) = rows[j] else { return };
            for i in i0..=i1 {
                // SAFETY: the row is this task's alone.
                let kind = unsafe { kinds.at(j * w + i) };
                if *kind == FLUID {
                    row[i] = ax[j * (w + 1) + i] + ax[j * (w + 1) + i + 1] + ay[j * w + i] + ay[(j + 1) * w + i];
                    if row[i] <= 0.0 {
                        *kind = NONE;
                    }
                }
            }
        });
        self.prepare();
    }

    /// Packs the fluid cells into the two lists of nodes, by colour, in row order (the lists
    /// keep their room from one solve to the next).
    fn prepare(&mut self) {
        let (w, h, rows, kind, diag, ax, ay, big) = (self.w, self.h, &self.rows, &self.kind, &self.diag, &self.ax, &self.ay, self.big());
        let cw = w.div_ceil(2);
        let fluid = |x: usize, y: usize| {
            let c = y * w + x;
            kind[c] == FLUID && diag[c] > 0.0
        };
        let first = |y: usize, colour: usize, x0: usize| if (x0 + y) % 2 == colour { x0 } else { x0 + 1 };
        // How many cells of each colour each row holds, then where each row starts.
        let count = |y: usize| -> [usize; 2] {
            let Some((x0, x1)) = rows[y] else { return [0, 0] };
            [0, 1].map(|colour| (first(y, colour, x0)..=x1).step_by(2).filter(|&x| fluid(x, y)).count())
        };
        let mut starts: Vec<[usize; 2]> = if big { (0..h).into_par_iter().map(count).collect() } else { (0..h).map(count).collect() };
        let mut total = [0, 0];
        for row in &mut starts {
            for colour in 0..2 {
                let n = row[colour];
                row[colour] = total[colour];
                total[colour] += n;
            }
        }
        for colour in 0..2 {
            self.nodes[colour].resize(total[colour], Node::default());
        }
        let lists = [Shared::new(&mut self.nodes[0]), Shared::new(&mut self.nodes[1])];
        let fill = |y: usize| {
            let Some((x0, x1)) = rows[y] else { return };
            for colour in 0..2 {
                let mut at = starts[y][colour];
                for x in (first(y, colour, x0)..=x1).step_by(2) {
                    if !fluid(x, y) {
                        continue;
                    }
                    let c = y * w + x;
                    let mut a = [0.0f32; 4];
                    let mut bits = ((x & 1) << 4 | (y & 1) << 5) as u8;
                    let mut edge = |k: usize, cond: f32, j: usize| {
                        a[k] = cond;
                        if cond != 0.0 && kind[j] == FLUID {
                            bits |= 1 << k;
                        }
                    };
                    if x > 0 {
                        edge(0, ax[y * (w + 1) + x], c - 1);
                    }
                    if x + 1 < w {
                        edge(1, ax[y * (w + 1) + x + 1], c + 1);
                    }
                    if y > 0 {
                        edge(2, ay[c], c - w);
                    }
                    if y + 1 < h {
                        edge(3, ay[c + w], c + w);
                    }
                    // SAFETY: each row writes its own stretch of the list.
                    unsafe {
                        *lists[colour].at(at) = Node { i: c as u32, ci: ((y / 2) * cw + x / 2) as u32, a, inv: 1.0 / diag[c], bits };
                    }
                    at += 1;
                }
            }
        };
        if big {
            (0..h).into_par_iter().for_each(fill);
        } else {
            (0..h).for_each(fill);
        }
    }

    /// Takes the coarse correction away from the fine cells the coarse level below left out
    /// (`parts` of that level, `cw` wide): a sweep that folds the correction in must not read
    /// one into them.
    fn unlink(&mut self, parts: &[u8], cw: usize) {
        let w = self.w;
        for list in &mut self.nodes {
            list.par_chunks_mut(CHUNK).for_each(|chunk| {
                for n in chunk {
                    if n.bits & 15 == 0 {
                        continue;
                    }
                    let (x, y) = (n.i as usize % w, n.i as usize / w);
                    for (k, (jx, jy)) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)].into_iter().enumerate() {
                        if n.bits >> k & 1 != 0 && parts[(jy / 2) * cw + jx / 2] >> ((jy & 1) * 2 + (jx & 1)) & 1 == 0 {
                            n.bits &= !(1 << k);
                        }
                    }
                }
            });
        }
    }

    /// Leaves the level clean over its rows (everything zero, every cell `NONE`), and without
    /// rows: what the next solve does not write must be nothing. One pass over the rows for all
    /// the grids.
    fn clear(&mut self) {
        let (w, h, rows) = (self.w, self.h, &self.rows);
        let (kind, diag, b, x, ax, ay) = (Shared::new(&mut self.kind), Shared::new(&mut self.diag), Shared::new(&mut self.b), Shared::new(&mut self.x), Shared::new(&mut self.ax), Shared::new(&mut self.ay));
        let row_fn = |y: usize| {
            // SAFETY: each task clears its own row (and the row of edges over it, which no other
            // task touches).
            unsafe {
                if let Some((x0, x1)) = rows[y] {
                    for i in y * w + x0..=y * w + x1 {
                        *kind.at(i) = NONE;
                        *diag.at(i) = 0.0;
                        *b.at(i) = 0.0;
                        *x.at(i) = 0.0;
                    }
                    for i in y * (w + 1) + x0..=y * (w + 1) + x1 + 1 {
                        *ax.at(i) = 0.0;
                    }
                }
                let above = if y > 0 { rows[y - 1] } else { None };
                if let Some((x0, x1)) = Self::union(above, rows[y]) {
                    for i in y * w + x0..=y * w + x1 {
                        *ay.at(i) = 0.0;
                    }
                }
            }
        };
        if self.w * self.h > BIG {
            (0..h).into_par_iter().for_each(row_fn);
        } else {
            (0..h).for_each(row_fn);
        }
        // The row of edges under the last row.
        if let Some((x0, x1)) = rows[h - 1] {
            self.ay[h * w + x0..=h * w + x1].fill(0.0);
        }
        self.rows.fill(None);
    }

    /// Row `y` of A x into `row` (the row of the result), over its fluid cells.
    #[inline]
    fn apply_row(&self, x: &[f32], y: usize, row: &mut [f32]) {
        let (w, h, kind, ax, ay, diag) = (self.w, self.h, &self.kind, &self.ax, &self.ay, &self.diag);
        let Some((x0, x1)) = self.rows[y] else { return };
        for i in x0..=x1 {
            let c = y * w + i;
            if kind[c] != FLUID {
                row[i] = 0.0;
                continue;
            }
            let mut s = diag[c] * x[c];
            if i > 0 {
                s -= ax[y * (w + 1) + i] * x[c - 1];
            }
            if i + 1 < w {
                s -= ax[y * (w + 1) + i + 1] * x[c + 1];
            }
            if y > 0 {
                s -= ay[c] * x[c - w];
            }
            if y + 1 < h {
                s -= ay[c + w] * x[c + w];
            }
            row[i] = s;
        }
    }

    /// (b - A x) at the fluid cell `c` = (i, y).
    #[inline]
    fn residual_at(&self, x: &[f32], i: usize, y: usize) -> f32 {
        let (w, h, ax, ay) = (self.w, self.h, &self.ax, &self.ay);
        let c = y * w + i;
        let mut s = self.b[c] - self.diag[c] * x[c];
        if i > 0 {
            s += ax[y * (w + 1) + i] * x[c - 1];
        }
        if i + 1 < w {
            s += ax[y * (w + 1) + i + 1] * x[c + 1];
        }
        if y > 0 {
            s += ay[c] * x[c - w];
        }
        if y + 1 < h {
            s += ay[c + w] * x[c + w];
        }
        s
    }

    /// Whether the passes over the whole grid (building and clearing it) are worth handing out
    /// to the cores.
    #[inline]
    fn big(&self) -> bool {
        self.w * self.h > BIG
    }

    /// Whether the level is worth handing out to the cores.
    #[inline]
    fn par(&self) -> bool {
        self.nodes[0].len() + self.nodes[1].len() > SMALL
    }

    /// `n` red-black Gauss-Seidel sweeps of A x = b over the fluid cells, the colours the other
    /// way round when `reverse`.
    ///
    /// With `from_zero`, the solution starts from zero (the first pass reads nothing; the cells
    /// that are not fluid must already be zero). With `corr`, the coarse correction `cx` is
    /// folded in: the first pass reads the other colour corrected, and the second overwrites
    /// that colour; the same as adding the correction to every cell first, without the pass.
    /// Returns b · x over the fluid cells when `dot_b` (summed in the last two passes, per chunk
    /// then the chunks in order, the same whatever the number of cores).
    fn smooth(&mut self, n: usize, reverse: bool, from_zero: bool, corr: Option<&[f32]>, dot_b: bool) -> f32 {
        let (w, b, nodes) = (self.w, &self.b, &self.nodes);
        let cw = w.div_ceil(2);
        let mut dot = 0.0f64;
        for sweep in 0..2 * n {
            let colour = if reverse { 1 - sweep % 2 } else { sweep % 2 };
            // The last two passes each leave their colour final: b · x is summed there.
            let sum = dot_b && sweep + 2 >= 2 * n;
            let xs = Shared::new(&mut self.x);
            let list = &nodes[colour];
            // SAFETY: a pass writes the cells of one colour and reads the other's.
            let chunk_fn = |chunk: &[Node]| -> f64 {
                let mut d = 0.0f64;
                unsafe {
                    if sweep == 0 && from_zero {
                        for n in chunk {
                            let i = n.i as usize;
                            let v = *b.get_unchecked(i) * n.inv;
                            *xs.at(i) = v;
                            if sum {
                                d += *b.get_unchecked(i) as f64 * v as f64;
                            }
                        }
                        return d;
                    }
                    match (sweep == 0, corr) {
                        (true, Some(cx)) => {
                            for n in chunk {
                                let (i, ci) = (n.i as usize, n.ci as usize);
                                let (xodd, yodd) = ((n.bits >> 4 & 1) as usize, (n.bits >> 5 & 1) as usize);
                                let nb = |k: usize, j: usize, cj: usize| {
                                    if n.a[k] == 0.0 {
                                        return 0.0;
                                    }
                                    let v = *xs.at(j);
                                    n.a[k] * if n.bits & (1 << k) != 0 { v + *cx.get_unchecked(cj) } else { v }
                                };
                                let s = *b.get_unchecked(i)
                                    + nb(0, i.wrapping_sub(1), (ci + xodd).wrapping_sub(1))
                                    + nb(1, i + 1, ci + xodd)
                                    + nb(2, i.wrapping_sub(w), (ci + yodd * cw).wrapping_sub(cw))
                                    + nb(3, i + w, ci + yodd * cw);
                                let v = s * n.inv;
                                *xs.at(i) = v;
                                if sum {
                                    d += *b.get_unchecked(i) as f64 * v as f64;
                                }
                            }
                        }
                        _ => {
                            for n in chunk {
                                let i = n.i as usize;
                                let nb = |k: usize, j: usize| if n.a[k] != 0.0 { n.a[k] * *xs.at(j) } else { 0.0 };
                                let s = *b.get_unchecked(i) + nb(0, i.wrapping_sub(1)) + nb(1, i + 1) + nb(2, i.wrapping_sub(w)) + nb(3, i + w);
                                let v = s * n.inv;
                                *xs.at(i) = v;
                                if sum {
                                    d += *b.get_unchecked(i) as f64 * v as f64;
                                }
                            }
                        }
                    }
                }
                d
            };
            if sum {
                let per_chunk: Vec<f64> = if list.len() <= SMALL { list.chunks(CHUNK).map(chunk_fn).collect() } else { list.par_chunks(CHUNK).map(chunk_fn).collect() };
                dot += per_chunk.iter().sum::<f64>();
            } else if list.len() <= SMALL {
                list.chunks(CHUNK).for_each(|c| {
                    chunk_fn(c);
                });
            } else {
                list.par_chunks(CHUNK).for_each(|c| {
                    chunk_fn(c);
                });
            }
        }
        dot as f32
    }

}

/// The solver, with its grids kept from one solve to the next.
#[derive(Default)]
pub struct Solver {
    pub levels: Vec<Level>,
    /// How many levels this solve uses: as many as it takes to bring the fluid cells down to a
    /// grid that sweeps alone (a few puddles need no hierarchy, a sea the whole of it).
    used: usize,
    /// Conjugate gradients: the residual, the preconditioned residual and the search direction.
    r: Vec<f32>,
    z: Vec<f32>,
    s: Vec<f32>,
    /// A s.
    q: Vec<f32>,
    /// The right side of the finest level, while a V-cycle borrows its place.
    bsave: Vec<f32>,
    /// The best solution so far, to fall back on if the iterations run away.
    /// Iterations the last solve took.
    pub iterations: u32,
}

impl Solver {
    /// Makes the hierarchy for a finest grid `w` x `h`, if not made yet.
    pub fn ready(&mut self, w: usize, h: usize) {
        if self.levels.first().is_some_and(|l| l.w == w && l.h == h) {
            return;
        }
        self.levels.clear();
        let (mut lw, mut lh) = (w, h);
        loop {
            self.levels.push(Level::new(lw, lh));
            if lw * lh <= COARSEST || lw < 4 || lh < 4 {
                break;
            }
            lw = lw.div_ceil(2);
            lh = lh.div_ceil(2);
        }
        let n = w * h;
        self.r = vec![0.0; n];
        self.z = vec![0.0; n];
        self.s = vec![0.0; n];
        self.q = vec![0.0; n];
        self.bsave = vec![0.0; n];
    }

    /// Leaves every level clean over its rows, ready for the next solve: whatever that one does
    /// not write (beside a smaller body, say) must be nothing.
    pub fn clear(&mut self) {
        // (The finest level is always to clear: it is filled in even when the card solves.)
        for l in &mut self.levels[..self.used.max(1)] {
            l.clear();
        }
    }

    /// The finest level, to fill in over its rows (`kind`, `ax`, `ay`, `diag`, `rows`, `b` the
    /// right side and `x` the start) before `solve`; it is clean, every cell nothing, from the
    /// last `clear`. A fluid cell must have a positive diagonal (an edge at least).
    pub fn fine(&mut self) -> &mut Level {
        &mut self.levels[0]
    }

    /// Builds the coarse levels from the finest one (after it has been filled in).
    pub fn coarsen(&mut self) {
        self.levels[0].prepare();
        let mut n = self.levels[0].nodes[0].len() + self.levels[0].nodes[1].len();
        self.used = 1;
        while n > COARSEST && self.used < self.levels.len() {
            n /= 4;
            self.used += 1;
        }
        // What the V-cycle will read as a fixed or absent cell must be zero: a cell that held
        // fluid last solve may hold gas now.
        let f = &self.levels[0];
        let (w, rows) = (f.w, &f.rows);
        self.z.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            if let Some((x0, x1)) = rows[y] {
                row[x0..=x1].fill(0.0);
            }
        });
        for l in 1..self.used {
            let (fine, coarse) = self.levels.split_at_mut(l);
            coarse[0].coarsen_from(&fine[l - 1]);
            if coarse[0].split {
                fine[l - 1].unlink(&coarse[0].parts, coarse[0].w);
            }
        }
    }

    /// One V-cycle of the multigrid on level `l`, solving A x = b of that level from the x it
    /// holds (zero below the finest). The sweeps after the visit below run the colours the other
    /// way round, so that the cycle is symmetric, as the conjugate gradients want it.
    fn vcycle(&mut self, l: usize, dot_b: bool) -> f32 {
        if l + 1 == self.used {
            return self.levels[l].smooth(COARSEST_SWEEPS, false, true, None, dot_b);
        }
        self.levels[l].smooth(SMOOTH, false, true, None, false);
        // The residual b - A x, summed into the coarse cells, is the coarse right side (one
        // pass: the residual is not kept). The last sweep left the cells of its colour exact,
        // so only the other colour has a residual.
        {
            let (fine, coarse) = self.levels.split_at_mut(l + 1);
            let (f, c) = (&fine[l], &mut coarse[0]);
            let (fw, fh, cw, par) = (f.w, f.h, c.w, f.par());
            let (fx, fkind, crows, cparts) = (&f.x, &f.kind, &c.rows, &c.parts);
            rows_mut!(c.b, cw, par, |(j, row): (usize, &mut [f32])| {
                let Some((i0, i1)) = crows[j] else { return };
                for i in i0..=i1 {
                    let mut s = 0.0;
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let (x, y) = (2 * i + dx, 2 * j + dy);
                            if x < fw && y < fh && (x + y) % 2 != LAST_COLOUR && fkind[y * fw + x] == FLUID && cparts[j * cw + i] >> (2 * dy + dx) & 1 != 0 {
                                s += f.residual_at(fx, x, y);
                            }
                        }
                    }
                    row[i] = s;
                }
            });
        }
        self.vcycle(l + 1, false);
        // The coarse correction (the same for the four cells of a coarse one) is folded into the
        // sweeps after the visit.
        let (fine, coarse) = self.levels.split_at_mut(l + 1);
        let d = fine[l].smooth(SMOOTH, true, false, Some(&coarse[0].x), dot_b);
        d
    }

    /// Solves A x = b on the finest level by conjugate gradients, the multigrid as
    /// preconditioner, from the `x` the level holds (the last solution), until the Jacobi step
    /// of every cell is under `tol`, or under `rel` times what it was at the start (a body in
    /// motion asks a pressure right to a part in a thousand, a body at rest right to the pascal),
    /// or `max_iter` iterations are done. Returns the iterations.
    pub fn solve(&mut self, tol: f32, rel: f32, max_iter: u32) -> u32 {
        let (mut r, mut z, mut s, mut q) = (std::mem::take(&mut self.r), std::mem::take(&mut self.z), std::mem::take(&mut self.s), std::mem::take(&mut self.q));
        // r = b - A x, and how far the pressure is from settled.
        let step0 = {
            let f = &self.levels[0];
            let (x, rows, w, kind, diag) = (&f.x, &f.rows, f.w, &f.kind, &f.diag);
            r.par_chunks_mut(w)
                .enumerate()
                .map(|(y, row)| {
                    let Some((x0, x1)) = rows[y] else { return 0.0 };
                    let mut m = 0.0f32;
                    for i in x0..=x1 {
                        let c = y * w + i;
                        if kind[c] == FLUID {
                            row[i] = f.residual_at(x, i, y);
                            m = m.max((row[i] / diag[c]).abs());
                        } else {
                            row[i] = 0.0;
                        }
                    }
                    m
                })
                .reduce(|| 0.0, f32::max)
        };
        let tol = tol.max(rel * step0);
        let mut iterations = 0;
        let mut best = step0;
        if step0 >= tol {
            // z = M⁻¹ r, s = z, rho = r · z.
            let mut rho = self.precondition(&mut r, &mut z);
            self.levels[0].direction(&z, &mut s, 0.0);
            while iterations < max_iter {
                iterations += 1;
                // q = A s, and s · q.
                let sq = self.levels[0].apply_dot(&s, &mut q);
                if !(sq > 0.0) || !rho.is_finite() {
                    break;
                }
                let alpha = rho / sq;
                // x += alpha s, r -= alpha q, and how far from settled now.
                let step = self.levels[0].advance(alpha, &s, &mut r, &q);
                if step < tol {
                    break;
                }
                // Running away (a pool with no fixed pressure to hang from, say: a singular
                // system): the step is undone and the solve stops there.
                if !(step <= 8.0 * best) {
                    self.levels[0].advance(-alpha, &s, &mut r, &q);
                    break;
                }
                best = best.min(step);
                let rho2 = self.precondition(&mut r, &mut z);
                let beta = rho2 / rho;
                rho = rho2;
                // s = z + beta s.
                self.levels[0].direction(&z, &mut s, beta);
            }
        }
        (self.r, self.z, self.s, self.q) = (r, z, s, q);
        self.iterations = iterations;
        iterations
    }

    /// z = M⁻¹ r: a V-cycle from zero on the finest level, with r as its right side. Returns
    /// r · z.
    fn precondition(&mut self, r: &mut Vec<f32>, z: &mut Vec<f32>) -> f32 {
        // The level's right side becomes r and its solution z (b and x are kept aside); the
        // V-cycle starts z from zero.
        let f = &mut self.levels[0];
        std::mem::swap(&mut f.b, r);
        std::mem::swap(&mut f.x, z);
        let rz = self.vcycle(0, true);
        let f = &mut self.levels[0];
        std::mem::swap(&mut f.x, z);
        std::mem::swap(&mut f.b, r);
        rz
    }
}

/// The vector operations of the conjugate gradients, over the fluid cells of a level, each a
/// pass over its rows side by side; every sum is a sum per row, then the rows in order, so that
/// the answer is the same whatever the number of cores.
impl Level {
    /// x += alpha s and r -= alpha q; returns the largest Jacobi step |r_i / diag_i| left, how
    /// far the pressure of a cell would still move.
    fn advance(&mut self, alpha: f32, s: &[f32], r: &mut [f32], q: &[f32]) -> f32 {
        let (w, rows, kind, diag) = (self.w, &self.rows, &self.kind, &self.diag);
        self.x
            .par_chunks_mut(w)
            .zip(r.par_chunks_mut(w))
            .enumerate()
            .map(|(y, (xrow, rrow))| {
                let Some((x0, x1)) = rows[y] else { return 0.0 };
                let mut m = 0.0f32;
                for i in x0..=x1 {
                    let c = y * w + i;
                    xrow[i] += alpha * s[c];
                    rrow[i] -= alpha * q[c];
                    if kind[c] == FLUID {
                        m = m.max((rrow[i] / diag[c]).abs());
                    }
                }
                m
            })
            .reduce(|| 0.0, f32::max)
    }

    /// q = A s; returns s · q.
    fn apply_dot(&self, s: &[f32], q: &mut [f32]) -> f32 {
        let (w, kind) = (self.w, &self.kind);
        let per_row: Vec<f64> = q
            .par_chunks_mut(w)
            .enumerate()
            .map(|(y, row)| {
                let Some((x0, x1)) = self.rows[y] else { return 0.0 };
                self.apply_row(s, y, row);
                let mut d = 0.0f64;
                for i in x0..=x1 {
                    let c = y * w + i;
                    if kind[c] == FLUID {
                        d += s[c] as f64 * row[i] as f64;
                    }
                }
                d
            })
            .collect();
        per_row.iter().sum::<f64>() as f32
    }

    /// s = z + beta s.
    fn direction(&self, z: &[f32], s: &mut [f32], beta: f32) {
        let (w, rows) = (self.w, &self.rows);
        rows_mut!(s, w, true, |(y, row): (usize, &mut [f32])| {
            let Some((x0, x1)) = rows[y] else { return };
            for i in x0..=x1 {
                let c = y * w + i;
                row[i] = z[c] + beta * row[i];
            }
        });
    }
}

#[cfg(test)]
pub(in crate::world) mod tests {
    use super::*;

    /// A tank of water w x h with the air over its top row: the pressure comes out hydrostatic,
    /// and in a handful of iterations whatever the width.
    fn tank(w: usize, h: usize) -> (Solver, f32) {
        tank_to(w, h, 1.0)
    }

    /// The same, solved until no pressure would move by more than `tol`.
    fn tank_to(w: usize, h: usize, tol: f32) -> (Solver, f32) {
        let mut s = Solver::default();
        s.ready(w, h);
        let rho = 1000.0f32;
        {
            let f = s.fine();
            for y in 0..h {
                for x in 0..w {
                    f.kind[y * w + x] = if y == 0 { FIXED } else { FLUID };
                }
                f.rows[y] = Some((0, w - 1));
            }
            for y in 0..h {
                for x in 1..w {
                    f.ax[y * (w + 1) + x] = 2.0 / (2.0 * rho);
                }
            }
            for y in 1..h {
                for x in 0..w {
                    f.ay[y * w + x] = if y == 1 { 1.0 / rho } else { 2.0 / (2.0 * rho) };
                }
            }
            for y in 1..h {
                for x in 0..w {
                    let c = y * w + x;
                    f.diag[c] = f.ax[y * (w + 1) + x] + f.ax[y * (w + 1) + x + 1] + f.ay[c] + f.ay[c + w];
                    // Gravity has given every edge of liquid a fall of g dt; the wall under the
                    // bottom row carries none, so that row alone has a divergence, -g dt, and a
                    // right side of +g dx (times dx / dt, over dt... the units of `pressure`).
                    f.b[c] = if y + 1 == h { 9.81 / 6.0 } else { 0.0 };
                }
            }
        }
        s.coarsen();
        let it = s.solve(tol, 0.0, 50) as f32;
        (s, it)
    }

    #[test]
    fn a_wide_tank_converges_in_a_few_iterations_and_is_hydrostatic() {
        for (w, h) in [(32, 16), (512, 64), (960, 200)] {
            let (s, it) = tank(w, h);
            let f = &s.levels[0];
            // Hydrostatic: p grows by rho g dx per row (dx = 1/6 m), from 0 at the centre of
            // the air cell (the fixed pressure sits a whole cell away, as in `pressure`).
            let step = 1000.0 * 9.81 / 6.0;
            for y in 1..h {
                let p = f.x[y * w + w / 2];
                let want = step * y as f32;
                assert!((p - want).abs() < 0.02 * want + 20.0, "{w}x{h} row {y}: {p} vs {want}");
            }
            assert!(it <= 8.0, "{w}x{h} took {it} iterations");
        }
    }

    /// Two tanks side by side, the wall between them `wall` cells thick and across the cells of
    /// the coarse grids (it starts at an odd column), the right one solved already and a splash
    /// (a strong divergence in a patch) in the left one: the system, ready to solve, and the
    /// pressure of the right tank before it.
    pub(in crate::world) fn two_tanks(w: usize, h: usize, wall: usize) -> (Solver, Vec<f32>) {
        let (mut s, _) = tank_to(w, h, 0.01);
        let rho = 1000.0f32;
        let still: Vec<f32> = s.levels[0].x.clone();
        s.clear();
        let mid = w / 2;
        let wall_at = |x: usize| x > mid && x <= mid + wall;
        {
            let f = s.fine();
            for y in 0..h {
                for x in 0..w {
                    f.kind[y * w + x] = if wall_at(x) { NONE } else if y == 0 { FIXED } else { FLUID };
                }
                f.rows[y] = Some((0, w - 1));
            }
            for y in 0..h {
                for x in 1..w {
                    f.ax[y * (w + 1) + x] = if wall_at(x - 1) || wall_at(x) || y == 0 { 0.0 } else { 1.0 / rho };
                }
            }
            for y in 1..h {
                for x in 0..w {
                    f.ay[y * w + x] = if wall_at(x) { 0.0 } else { 1.0 / rho };
                }
            }
            for y in 1..h {
                for x in 0..w {
                    let c = y * w + x;
                    if wall_at(x) {
                        continue;
                    }
                    f.diag[c] = f.ax[y * (w + 1) + x] + f.ax[y * (w + 1) + x + 1] + f.ay[c] + f.ay[c + w];
                    f.b[c] = if y + 1 == h { 9.81 / 6.0 } else { 0.0 };
                    if x > mid / 4 && x < mid / 2 && y > h / 4 && y < h / 2 {
                        f.b[c] += 4000.0;
                    }
                    f.x[c] = still[c];
                }
            }
        }
        (s, still)
    }

    /// The most the right tank's pressure in `x` moved from `still`, Pa.
    pub(in crate::world) fn right_tank_moved(x: &[f32], still: &[f32], w: usize, h: usize, wall: usize) -> f32 {
        let mut moved = 0.0f32;
        for y in 1..h {
            for i in w / 2 + wall + 1..w {
                moved = moved.max((x[y * w + i] - still[y * w + i]).abs());
            }
        }
        moved
    }

    /// The splash in the left tank must leave the right one as it was: they share no edge, so
    /// nothing of one may reach the other through the coarse grids (it moved by 3 MPa when a
    /// coarse cell joined them).
    #[test]
    fn a_splash_in_one_tank_leaves_the_tank_beside_it_alone() {
        for (w, h, wall) in [(512, 128, 1), (960, 200, 7), (256, 64, 2)] {
            let (mut s, still) = two_tanks(w, h, wall);
            s.coarsen();
            s.solve(1.0, 1e-3, 12);
            let moved = right_tank_moved(&s.levels[0].x, &still, w, h, wall);
            assert!(moved < 10.0, "{w}x{h}, wall {wall}: the right tank moved by {moved} Pa");
        }
    }
}
