//! Liquids by their physics (`World::liquid_physics`, an option, off unless `AWORLD_LIQUIDS=1`).
//!
//! Every liquid (water, oil, lava, the melts) is moved here by real forces in real units: a
//! velocity in m/s on each edge between two cells (a "MAC" grid: the velocity lives where the
//! liquid passes), a pressure in pascals in each cell, gravity 9.81 m/s², a cell 1/6 m across (60
//! cells of water weigh one atmosphere, as in `pockets.rs`) and a step of 1/60 s. A step:
//! 1. gravity pulls every edge that touches liquid down;
//! 2. the velocities are carried along by the flow itself (semi-Lagrangian): what runs keeps
//!    running, which is what makes waves, jets and the slosh of a U-tube;
//! 3. a thick liquid (lava) smooths its velocities (viscosity, solved implicitly so that it is
//!    stable however thick), thicker as it cools, and stands still under a yield stress;
//! 4. the pressure that keeps every full cell from filling up or emptying is solved (red-black
//!    sweeps, starting from the last step's pressure); at a free surface it is the pressure of
//!    the gas over it (its pocket's, see `pockets.rs`: siphons and boilers keep working);
//! 5. each edge passes velocity × time × the liquid of the cell upstream: the liquid moves, with
//!    its heat, and nothing is created or lost.
//!
//! Only the zones that run take part: a sleeping zone is liquid at rest, its pressure a fixed
//! boundary for the zones around it. Two bodies of liquid that do not touch never share a
//! pressure, so each is in effect solved on its own. A lone cell of water falling through the air
//! is a drop of the cells' turn (rain); a drop that lands gives its speed to the edges under it.

use super::par::Shared;
use super::*;
use rayon::prelude::*;

/// A cell, in metres.
pub const CELL_M: f32 = 1.0 / 6.0;
/// A step, in seconds.
const DT: f32 = 1.0 / 60.0;
const G: f32 = 9.81;
/// One atmosphere, in pascals.
const ATM: f32 = 101_325.0;
/// Sweeps of the pressure a sub-step.
const SWEEPS: usize = 30;
/// The sweeps stop once no pressure moves by more than this, Pa (a tenth of a millimetre of water).
const SETTLED: f32 = 1.0;
/// Over-relaxation of the sweeps.
const SOR: f32 = 1.7;
/// Sweeps of the viscosity a sub-step.
const VISCOUS_SWEEPS: usize = 12;
/// Slower than this (m/s), an edge between two cells of liquid stands still: still water stops,
/// so that its zones can sleep.
const REST: f32 = 0.01;
/// The part of its gap to full that a cell inside a body makes up a sub-step.
const REFILL: f32 = 0.1;
/// A cell holds a pressure from this part of full (1/HOLDS) up.
const HOLDS: u32 = 2;
/// Liquid away from any full cell (drops, spray, a film) goes no faster than this, m/s: the air
/// holds it back (rain falls at about this speed).
const SPRAY: f32 = 9.0;
/// At most this many sub-steps a step (fast flows take smaller ones).
const MAX_SUB: usize = 4;

/// Whether new worlds take their liquids by physics: `AWORLD_LIQUIDS=1`.
pub(super) fn physics_by_default() -> bool {
    std::env::var_os("AWORLD_LIQUIDS").is_some_and(|v| v != "0")
}

/// What a cell is to the liquids.
const SOLID: u8 = 0;
const AIR: u8 = 1;
/// Liquid, less than half full: it moves with the edges around it, but holds no pressure.
const WET: u8 = 2;
/// Liquid, at least half full: it holds a pressure.
const FULLISH: u8 = 3;

/// Density (kg/m³), kinematic viscosity (m²/s) and yield stress (Pa) of liquid `m` at `t` °C.
pub(super) fn props(m: Mat, t: f32) -> (f32, f32, f32) {
    let rho = density(m) * 1000.0;
    match m {
        Mat::Water => (rho, 1e-6, 0.0),
        Mat::Oil => (rho, 1e-4, 0.0),
        // Basalt at 1 150 °C: about 3 000 Pa·s; e times thicker for every 60 °C it cools, and a
        // yield stress that grows as it cools too (a cooling flow stops, its crust holds).
        Mat::Lava => {
            let cool = ((1150.0 - t) / 60.0).clamp(0.0, 8.0);
            (rho, (cool.exp()).min(50.0), 500.0 * ((1150.0 - t) / 150.0).clamp(0.0, 8.0).exp())
        }
        // Molten metal runs about like water.
        _ => (rho, 1e-6, 0.0),
    }
}

/// The liquids' grids: velocities on the edges, pressure in the cells.
#[derive(Default)]
pub struct Liquids {
    /// Rightward velocity on the left edge of each cell, (W + 1) a row, m/s.
    u: Vec<f32>,
    /// Downward velocity on the top edge of each cell, W a row, H + 1 rows, m/s.
    v: Vec<f32>,
    /// Pressure in each cell over the ambient one, Pa (kept from step to step).
    p: Vec<f32>,
    /// What each cell is (`SOLID`, `AIR`, `WET`, `FULLISH`), and whether its zone runs (+ 4).
    class: Vec<u8>,
    /// For each row, the span of cells (first, last) the liquids work on this sub-step: the
    /// liquid that runs and the cells around it. `None`: nothing to do in the row.
    rows: Vec<Option<(usize, usize)>>,
    /// Scratch for the advection and the viscosity.
    u2: Vec<f32>,
    v2: Vec<f32>,
    /// Viscosity of each cell, m²/s (0: not thick), and whether any cell was thick.
    nu: Vec<f32>,
    thick: bool,
    /// Scratch kept from step to step (allocated once, and cleared only where it was used): the
    /// divergence of each cell, what each edge passes, the liquid each empty cell takes and the
    /// cut of each cell's outflows.
    rhs: Vec<f32>,
    fu: Vec<i64>,
    fv: Vec<i64>,
    want: Vec<Mat>,
    scale: Vec<f64>,
}

impl Liquids {
    fn ready(&mut self) {
        if self.u.len() != (W + 1) * H {
            self.u = vec![0.0; (W + 1) * H];
            self.v = vec![0.0; W * (H + 1)];
            self.p = vec![0.0; W * H];
            self.class = vec![SOLID; W * H];
            self.rows = vec![None; H + 1];
            self.u2 = vec![0.0; (W + 1) * H];
            self.v2 = vec![0.0; W * (H + 1)];
            self.nu = vec![0.0; W * H];
            self.rhs = vec![0.0; W * H];
            self.fu = vec![0; (W + 1) * H];
            self.fv = vec![0; W * (H + 1)];
            self.want = vec![Mat::Empty; W * H];
            self.scale = vec![1.0; W * H];
        }
    }

    /// Forgets every flow (a new world, a load).
    pub fn clear(&mut self) {
        *self = Liquids::default();
    }

    /// The span of row `y` (none outside the world).
    #[inline]
    fn span(&self, y: usize) -> Option<(usize, usize)> {
        self.rows.get(y).copied().flatten()
    }
}

#[inline]
fn ui(x: usize, y: usize) -> usize {
    y * (W + 1) + x
}
#[inline]
fn vi(x: usize, y: usize) -> usize {
    y * W + x
}

/// Bilinear sample of a grid `w` wide (`h` rows) at (x, y) in its own index space.
fn sample(a: &[f32], w: usize, h: usize, x: f32, y: f32) -> f32 {
    let x = x.clamp(0.0, (w - 1) as f32 - 0.001);
    let y = y.clamp(0.0, (h - 1) as f32 - 0.001);
    let (x0, y0) = (x as usize, y as usize);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |x: usize, y: usize| a[y * w + x];
    at(x0, y0) * (1.0 - fx) * (1.0 - fy) + at(x0 + 1, y0) * fx * (1.0 - fy) + at(x0, y0 + 1) * (1.0 - fx) * fy + at(x0 + 1, y0 + 1) * fx * fy
}

impl World {
    /// Turns the physics of liquids on or off. Off, a cell of liquid is always full again: the
    /// cells under half full go and the others fill up, about as much as there was.
    pub fn set_liquid_physics(&mut self, on: bool) {
        if on {
            self.set_water_field(false);
        } else {
            for i in 0..W * H {
                if self.cells[i].m.kind() == Kind::Liquid && self.fill[i] != 0 {
                    if self.fill[i] < field::FULL / 2 {
                        self.cells[i] = Cell { m: Mat::Empty, stamp: self.frame };
                        self.clear_states(i);
                    }
                    self.fill[i] = 0;
                    self.zones.wake(i);
                }
            }
            self.revision += 1;
        }
        self.liq.clear();
        self.liquid_physics = on;
    }

    /// Units of liquid in cell `i` (`field::FULL` for a full cell).
    #[inline]
    fn liquid_fill(&self, i: usize) -> u32 {
        self.water_fill(i)
    }

    /// Whether the cell at (x, y) is a lone cell of liquid (nothing liquid beside or over it):
    /// with air under it, it is a drop of the cells' turn.
    pub(super) fn lone_liquid(&self, x: i32, y: i32) -> bool {
        [(x - 1, y), (x + 1, y), (x, y - 1)].iter().all(|&(nx, ny)| self.at(nx, ny).kind() != Kind::Liquid)
    }

    /// The liquids' turn.
    pub(super) fn step_liquids(&mut self) {
        let mut liq = std::mem::take(&mut self.liq);
        liq.ready();
        self.classify(&mut liq);
        if liq.rows.iter().all(Option::is_none) {
            // No liquid runs anywhere.
            self.liq = liq;
            return;
        }
        // A drop that lands gives its speed to the edges under and over it.
        for y in 0..H {
            let Some((x0, x1)) = liq.span(y) else { continue };
            for x in x0..=x1 {
                let i = y * W + x;
                if liq.class[i] & 3 >= WET && self.vel[i] != 0 && !self.lone_liquid(x as i32, y as i32) {
                    let speed = self.vel[i] as f32 / 8.0 * CELL_M / DT;
                    liq.v[vi(x, y + 1)] = liq.v[vi(x, y + 1)].max(speed);
                    liq.v[vi(x, y)] = liq.v[vi(x, y)].max(speed);
                    self.vel[i] = 0;
                }
            }
        }
        // As many sub-steps as it takes for the fastest liquid to cross at most a cell in each.
        let fastest = (0..H)
            .into_par_iter()
            .map(|y| {
                let Some((x0, x1)) = liq.span(y) else { return 0.0 };
                let u = liq.u[ui(x0, y)..=ui(x1 + 1, y)].iter();
                let v = liq.v[vi(x0, y)..=vi(x1, y)].iter().chain(&liq.v[vi(x0, y + 1)..=vi(x1, y + 1)]);
                u.chain(v).fold(0.0f32, |a, v| a.max(v.abs()))
            })
            .reduce(|| 0.0, f32::max)
            + G * DT;
        let sub = ((fastest * DT / CELL_M).ceil() as usize).clamp(1, MAX_SUB);
        let dt = DT / sub as f32;
        for k in 0..sub {
            if k > 0 {
                self.classify(&mut liq);
            }
            self.forces(&mut liq, dt);
            self.viscosity(&mut liq, dt);
            self.pressure(&mut liq, dt);
            self.carry_liquids(&mut liq, dt);
        }
        self.liq = liq;
    }

    /// What each cell is to the liquids this sub-step, and the span of each row to work on.
    fn classify(&self, liq: &mut Liquids) {
        let awake = &self.zones.awake;
        // The first and last cell of each row that holds liquid that runs.
        let ends: Vec<(usize, usize)> = liq
            .class
            .par_chunks_mut(W)
            .enumerate()
            .map(|(y, row)| {
                let (mut first, mut last) = (usize::MAX, 0);
                for (x, c) in row.iter_mut().enumerate() {
                    let i = y * W + x;
                    let run = awake[y / sleep::CS * sleep::CW + x / sleep::CS];
                    let what = if self.mask[i] != 0 {
                        SOLID
                    } else {
                        match self.cells[i].m.kind() {
                            Kind::Empty | Kind::Gas => AIR,
                            // A lone drop of water in the air is the cells' rain.
                            Kind::Liquid if self.cells[i].m == Mat::Water && y + 1 < H && matches!(self.cells[i + W].m.kind(), Kind::Empty | Kind::Gas) && self.lone_liquid(x as i32, y as i32) => AIR,
                            Kind::Liquid if self.liquid_fill(i) >= field::FULL / HOLDS => FULLISH,
                            Kind::Liquid => WET,
                            _ => SOLID,
                        }
                    };
                    *c = what | if run { 4 } else { 0 };
                    if run && what >= WET {
                        first = first.min(x);
                        last = x;
                    }
                }
                (first, last)
            })
            .collect();
        // The rows around them too, a cell wider on each side.
        for y in 0..=H {
            let mut span: Option<(usize, usize)> = None;
            for yy in y.saturating_sub(1)..=(y + 1).min(H - 1) {
                let (f, l) = ends[yy];
                if f <= l {
                    span = Some(span.map_or((f, l), |(a, b)| (a.min(f), b.max(l))));
                }
            }
            liq.rows[y] = if y < H { span.map(|(a, b)| (a.saturating_sub(1), (b + 1).min(W - 1))) } else { None };
        }
    }

    /// Gravity and the carrying of the velocities along the flow, on the edges near liquid in
    /// the zones that run.
    fn forces(&self, liq: &mut Liquids, dt: f32) {
        let class = &liq.class;
        let kind = |x: i32, y: i32| if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 { SOLID } else { class[y as usize * W + x as usize] & 3 };
        let runs = |x: i32, y: i32| (0..W as i32).contains(&x) && (0..H as i32).contains(&y) && class[y as usize * W + x as usize] & 4 != 0;
        let wet = |c: u8| c >= WET;
        let step = dt / CELL_M;
        let rows = &liq.rows;
        // Rightward edges: (x, y) between cells (x - 1, y) and (x, y).
        {
            let (u, v) = (&liq.u, &liq.v);
            liq.u2.par_chunks_mut(W + 1).enumerate().for_each(|(y, row)| {
                row.copy_from_slice(&u[ui(0, y)..ui(0, y + 1)]);
                let Some((x0, x1)) = rows[y] else { return };
                for x in x0..=x1 + 1 {
                    let (xi, yi) = (x as i32, y as i32);
                    let (a, b) = (kind(xi - 1, yi), kind(xi, yi));
                    let here = u[ui(x, y)];
                    row[x] = if a == SOLID || b == SOLID {
                        0.0
                    } else if !(wet(a) || wet(b)) || !(runs(xi - 1, yi) || runs(xi, yi)) {
                        if wet(a) || wet(b) { here } else { 0.0 }
                    } else {
                        // The downward velocity here: the mean of the four edges around.
                        let vy = 0.25 * (v[vi(x - 1, y)] + v[vi(x, y)] + v[vi(x - 1, y + 1)] + v[vi(x, y + 1)]);
                        // Where it was a sub-step ago, in the index space of `u` (edge (x, y) sits
                        // at x, y + 1/2 in cells; its index is (x, y)).
                        let to = sample(u, W + 1, H, x as f32 - here * step, y as f32 - vy * step);
                        // Away from any full cell, liquid in the air is slowed by it (drops and
                        // spray, at most as fast as falling rain).
                        if a != FULLISH && b != FULLISH { to.clamp(-SPRAY, SPRAY) } else { to }
                    };
                }
            });
        }
        // Downward edges: (x, y) between cells (x, y - 1) and (x, y).
        {
            let (u, v) = (&liq.u, &liq.v);
            liq.v2.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
                row.copy_from_slice(&v[vi(0, y)..vi(0, y + 1)]);
                let Some((x0, x1)) = rows[y] else { return };
                for x in x0..=x1 {
                    let (xi, yi) = (x as i32, y as i32);
                    let (a, b) = (kind(xi, yi - 1), kind(xi, yi));
                    let here = v[vi(x, y)];
                    row[x] = if a == SOLID || b == SOLID {
                        0.0
                    } else if !(wet(a) || wet(b)) || !(runs(xi, yi - 1) || runs(xi, yi)) {
                        if wet(a) || wet(b) { here } else { 0.0 }
                    } else {
                        let ux = 0.25 * (u[ui(x, y - 1)] + u[ui(x + 1, y - 1)] + u[ui(x, y)] + u[ui(x + 1, y)]);
                        let fall = sample(v, W, H + 1, x as f32 - ux * step, y as f32 - here * step) + G * dt;
                        if a != FULLISH && b != FULLISH {
                            // A film lying on liquid or on the ground goes no faster than what is
                            // under it (it has nowhere to fall).
                            let fall = fall.clamp(-SPRAY, SPRAY);
                            if a < WET && b == WET && matches!(kind(xi, yi + 1), SOLID | FULLISH) { fall.min(v[vi(x, y + 1)].max(0.0)) } else { fall }
                        } else {
                            fall
                        }
                    };
                }
            });
        }
        std::mem::swap(&mut liq.u, &mut liq.u2);
        std::mem::swap(&mut liq.v, &mut liq.v2);
    }

    /// Smooths the velocities of thick liquids (implicit: red-black sweeps of
    /// u - dt ν ∇²u = u_before), no slip on walls, free at the surface.
    fn viscosity(&self, liq: &mut Liquids, dt: f32) {
        if liq.thick {
            liq.nu.fill(0.0);
        }
        // How thick each cell is, from its liquid, its heat and (yield stress) how little it moves.
        let (u, v, class, rows) = (&liq.u, &liq.v, &liq.class, &liq.rows);
        liq.thick = liq
            .nu
            .par_chunks_mut(W)
            .enumerate()
            .map(|(y, row)| {
                let Some((x0, x1)) = rows[y] else { return false };
                let mut any = false;
                for x in x0..=x1 {
                    let i = y * W + x;
                    if class[i] & 3 < WET || class[i] & 4 == 0 {
                        continue;
                    }
                    let (rho, n, tau) = props(self.cells[i].m, self.temp[i]);
                    if n * dt / (CELL_M * CELL_M) < 0.01 {
                        continue;
                    }
                    let mut n = n;
                    if tau > 0.0 {
                        let ux = (u[ui(x + 1, y)] - u[ui(x, y)]) / CELL_M;
                        let vy = (v[vi(x, y + 1)] - v[vi(x, y)]) / CELL_M;
                        let uc = |y: usize| 0.5 * (u[ui(x, y)] + u[ui(x + 1, y)]);
                        let vc = |x: usize| 0.5 * (v[vi(x, y)] + v[vi(x, y + 1)]);
                        let uy = (uc((y + 1).min(H - 1)) - uc(y.saturating_sub(1))) / (2.0 * CELL_M);
                        let vx = (vc((x + 1).min(W - 1)) - vc(x.saturating_sub(1))) / (2.0 * CELL_M);
                        let rate = (2.0 * (ux * ux + vy * vy) + (uy + vx) * (uy + vx)).sqrt();
                        n += tau / (rho * (rate + 1e-3));
                    }
                    row[x] = n.min(50.0);
                    any = true;
                }
                any
            })
            .reduce(|| false, |a, b| a || b);
        if !liq.thick {
            return;
        }
        let c = dt / (CELL_M * CELL_M);
        let nu = &liq.nu;
        let cell_nu = |x: i32, y: i32| if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 { 0.0 } else { nu[y as usize * W + x as usize] };
        // Rightward edges.
        liq.u2.copy_from_slice(&liq.u);
        let before = std::mem::take(&mut liq.u2);
        for sweep in 0..2 * VISCOUS_SWEEPS {
            let colour = sweep % 2;
            let us = Shared::new(&mut liq.u);
            (0..H).into_par_iter().for_each(|y| {
                let Some((x0, x1)) = rows[y] else { return };
                for x in x0.max(1)..=x1.min(W - 1) {
                    if (x + y) % 2 != colour {
                        continue;
                    }
                    let n = 0.5 * (cell_nu(x as i32 - 1, y as i32) + cell_nu(x as i32, y as i32));
                    if n == 0.0 {
                        continue;
                    }
                    // SAFETY: a sweep writes only edges of one colour and reads only the other.
                    let at = |x: usize, y: usize| unsafe { *us.at(ui(x, y)) };
                    let (mut sum, mut diag) = (0.0, 1.0);
                    for (nx, ny, nn) in [(x - 1, y, cell_nu(x as i32 - 1, y as i32)), (x + 1, y, cell_nu(x as i32, y as i32))] {
                        if nx == 0 || nx >= W {
                            diag += c * nn;
                        } else {
                            sum += c * nn * at(nx, ny);
                            diag += c * nn;
                        }
                    }
                    for dy in [-1i32, 1] {
                        let ny = y as i32 + dy;
                        let nn = 0.25 * (cell_nu(x as i32 - 1, y as i32) + cell_nu(x as i32, y as i32) + cell_nu(x as i32 - 1, ny) + cell_nu(x as i32, ny));
                        let wall = ny < 0 || ny >= H as i32 || class[ny as usize * W + x - 1] & 3 == SOLID || class[ny as usize * W + x] & 3 == SOLID;
                        if wall {
                            diag += c * n;
                        } else if nn > 0.0 {
                            sum += c * nn * at(x, ny as usize);
                            diag += c * nn;
                        }
                    }
                    unsafe { *us.at(ui(x, y)) = (before[ui(x, y)] + sum) / diag };
                }
            });
        }
        liq.u2 = before;
        // Downward edges.
        liq.v2.copy_from_slice(&liq.v);
        let before = std::mem::take(&mut liq.v2);
        for sweep in 0..2 * VISCOUS_SWEEPS {
            let colour = sweep % 2;
            let vs = Shared::new(&mut liq.v);
            (1..H).into_par_iter().for_each(|y| {
                let Some((x0, x1)) = rows[y] else { return };
                for x in x0..=x1 {
                    if (x + y) % 2 != colour {
                        continue;
                    }
                    let n = 0.5 * (cell_nu(x as i32, y as i32 - 1) + cell_nu(x as i32, y as i32));
                    if n == 0.0 {
                        continue;
                    }
                    // SAFETY: as for the rightward edges.
                    let at = |x: usize, y: usize| unsafe { *vs.at(vi(x, y)) };
                    let (mut sum, mut diag) = (0.0, 1.0);
                    for (ny, nn) in [(y - 1, cell_nu(x as i32, y as i32 - 1)), (y + 1, cell_nu(x as i32, y as i32))] {
                        if ny == 0 || ny >= H {
                            diag += c * nn;
                        } else {
                            sum += c * nn * at(x, ny);
                            diag += c * nn;
                        }
                    }
                    for dx in [-1i32, 1] {
                        let nx = x as i32 + dx;
                        let nn = 0.25 * (cell_nu(x as i32, y as i32 - 1) + cell_nu(x as i32, y as i32) + cell_nu(nx, y as i32 - 1) + cell_nu(nx, y as i32));
                        let wall = nx < 0 || nx >= W as i32 || class[(y - 1) * W + nx as usize] & 3 == SOLID || class[y * W + nx as usize] & 3 == SOLID;
                        if wall {
                            diag += c * n;
                        } else if nn > 0.0 {
                            sum += c * nn * at(nx as usize, y);
                            diag += c * nn;
                        }
                    }
                    unsafe { *vs.at(vi(x, y)) = (before[vi(x, y)] + sum) / diag };
                }
            });
        }
        liq.v2 = before;
    }

    /// Pressure of the gas over the ambient one at (x, y), Pa (0 outside a closed pocket).
    fn gas_pa(&self, x: i32, y: i32) -> f32 {
        self.pocket_at(x, y).map_or(0.0, |p| (self.pockets[p].pressure - 1.0) * ATM)
    }

    /// Solves the pressure that keeps the full cells of the zones that run from filling or
    /// emptying, then takes its push off the velocities.
    fn pressure(&self, liq: &mut Liquids, dt: f32) {
        let (class, rows) = (&liq.class, &liq.rows);
        let rho = |i: usize| density(self.cells[i].m) * 1000.0;
        // The divergence of each full cell that runs, times dx / dt.
        let k = CELL_M / dt;
        let (u, v) = (&liq.u, &liq.v);
        let mut rhs_buf = std::mem::take(&mut liq.rhs);
        rhs_buf.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0..=x1 {
                let i = y * W + x;
                if class[i] != FULLISH | 4 {
                    row[x] = 0.0;
                    continue;
                }
                // A cell inside the body (no air beside it, a film of liquid at most) that holds
                // less or more than full is brought back to full a little at a time: else the
                // small errors of the carrying add up, the liquid swells into a light froth that
                // no longer weighs, or a film over a cell not quite full can never get into it.
                let inside = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)]
                    .iter()
                    .all(|&(nx, ny)| nx >= W || ny >= H || class[ny * W + nx] & 3 != AIR);
                let fix = if inside { REFILL * (self.liquid_fill(i) as f32 / field::FULL as f32 - 1.0).clamp(-1.0, 1.0) } else { 0.0 };
                row[x] = (u[ui(x + 1, y)] - u[ui(x, y)] + v[vi(x, y + 1)] - v[vi(x, y)]) * k - fix * CELL_M / dt * k;
            }
        });
        // Each full cell that runs, by colour, with what its sweep needs: the weights of its four
        // neighbours (0 for a wall, or for air, whose pressure is fixed and goes into `c`), and
        // p = (Σ b·p_neighbour + c) / den.
        struct Node {
            i: u32,
            b: [f32; 4],
            c: f32,
            den: f32,
        }
        let rhs = &rhs_buf;
        let nodes: Vec<Vec<Node>> = (0..2)
            .map(|colour| {
                (0..H)
                    .into_par_iter()
                    .flat_map_iter(|y| {
                        let (x0, x1) = rows[y].unwrap_or((1, 0));
                        (x0..=x1).filter(move |x| (x + y) % 2 == colour).filter_map(move |x| {
                            let i = y * W + x;
                            if class[i] != FULLISH | 4 {
                                return None;
                            }
                            let r = rho(i);
                            let (mut b, mut c, mut den) = ([0.0f32; 4], -rhs[i], 0.0);
                            for (k, (nx, ny)) in [(x as i32 - 1, y as i32), (x as i32 + 1, y as i32), (x as i32, y as i32 - 1), (x as i32, y as i32 + 1)].into_iter().enumerate() {
                                if nx < 0 || ny < 0 || nx >= W as i32 || ny >= H as i32 {
                                    continue;
                                }
                                let j = ny as usize * W + nx as usize;
                                match class[j] & 3 {
                                    SOLID => {}
                                    FULLISH => {
                                        b[k] = 2.0 / (r + rho(j));
                                        den += b[k];
                                    }
                                    _ => {
                                        c += self.gas_pa(nx, ny) / r;
                                        den += 1.0 / r;
                                    }
                                }
                            }
                            (den > 0.0).then_some(Node { i: i as u32, b, c, den })
                        })
                    })
                    .collect()
            })
            .collect();
        for sweep in 0..2 * SWEEPS {
            let ps = Shared::new(&mut liq.p);
            let moved = nodes[sweep % 2]
                .par_chunks(512)
                .map(|chunk| {
                    let mut moved = 0.0f32;
                    for n in chunk {
                        let i = n.i as usize;
                        // SAFETY: a sweep writes only cells of one colour and reads only the other.
                        unsafe {
                            let at = |j: usize| *ps.at(j);
                            let sum = n.b[0] * if n.b[0] != 0.0 { at(i - 1) } else { 0.0 }
                                + n.b[1] * if n.b[1] != 0.0 { at(i + 1) } else { 0.0 }
                                + n.b[2] * if n.b[2] != 0.0 { at(i - W) } else { 0.0 }
                                + n.b[3] * if n.b[3] != 0.0 { at(i + W) } else { 0.0 };
                            let target = (sum + n.c) / n.den;
                            let p = ps.at(i);
                            let d = SOR * (target - *p);
                            *p += d;
                            moved = moved.max(d.abs());
                        }
                    }
                    moved
                })
                .reduce(|| 0.0, f32::max);
            // Settled (the largest change, whatever the order of the cores, is the same): a body at
            // rest is done in a sweep or two.
            if sweep % 2 == 1 && moved < SETTLED {
                break;
            }
        }
        liq.rhs = std::mem::take(&mut rhs_buf);
        // The push of the pressure, on every edge beside a full cell that runs.
        let p = &liq.p;
        let pa = |x: i32, y: i32| {
            let j = y as usize * W + x as usize;
            if class[j] & 3 == FULLISH { p[j] } else { self.gas_pa(x, y) }
        };
        let s = dt / CELL_M;
        liq.u.par_chunks_mut(W + 1).enumerate().for_each(|(y, row)| {
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0.max(1)..=(x1 + 1).min(W - 1) {
                let (a, b) = (y * W + x - 1, y * W + x);
                let (ca, cb) = (class[a], class[b]);
                if ca & 3 == SOLID || cb & 3 == SOLID || !(ca == FULLISH | 4 || cb == FULLISH | 4) {
                    continue;
                }
                let r = if ca & 3 >= WET && cb & 3 >= WET { 0.5 * (rho(a) + rho(b)) } else if ca & 3 >= WET { rho(a) } else { rho(b) };
                row[x] -= s / r * (pa(x as i32, y as i32) - pa(x as i32 - 1, y as i32));
                if ca & 3 == FULLISH && cb & 3 == FULLISH && row[x].abs() < REST {
                    row[x] = 0.0;
                }
            }
        });
        liq.v.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
            if y == 0 || y == H {
                return;
            }
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0..=x1 {
                let (a, b) = ((y - 1) * W + x, y * W + x);
                let (ca, cb) = (class[a], class[b]);
                if ca & 3 == SOLID || cb & 3 == SOLID || !(ca == FULLISH | 4 || cb == FULLISH | 4) {
                    continue;
                }
                let r = if ca & 3 >= WET && cb & 3 >= WET { 0.5 * (rho(a) + rho(b)) } else if ca & 3 >= WET { rho(a) } else { rho(b) };
                row[x] -= s / r * (pa(x as i32, y as i32) - pa(x as i32, y as i32 - 1));
                if ca & 3 == FULLISH && cb & 3 == FULLISH && row[x].abs() < REST {
                    row[x] = 0.0;
                }
            }
        });
    }

    /// Moves the liquid along the edges: each edge passes velocity × dt × the liquid of the cell
    /// upstream, into air or the same liquid, never more than the cell holds.
    fn carry_liquids(&mut self, liq: &mut Liquids, dt: f32) {
        let (class, rows) = (&liq.class, &liq.rows);
        let s = dt / CELL_M;
        let m = |i: usize| self.cells[i].m;
        // What each edge passes (units of fill, from the upstream cell), before the limits.
        let flow = |vel: f32, a: usize, b: usize| -> i64 {
            let (ca, cb) = (class[a] & 3, class[b] & 3);
            if vel == 0.0 || ca == SOLID || cb == SOLID || (class[a] & 4 == 0 && class[b] & 4 == 0) {
                return 0;
            }
            let (from, to, sign) = if vel > 0.0 { (a, b, 1) } else { (b, a, -1) };
            if class[from] & 3 < WET {
                return 0;
            }
            // Into air (only an empty cell takes liquid: a gas stays where it is) or the same liquid.
            let ok = match self.cells[to].m.kind() {
                Kind::Empty => true,
                Kind::Liquid => m(to) == m(from),
                _ => false,
            };
            if !ok {
                return 0;
            }
            // Into a cell that is full or nearly, the liquid passes as from a full cell (the cut
            // below keeps it to what the giver holds): else a surface that sinks would leave the
            // cells under it short at every step, and the body would swell into froth whose weight
            // the pressure counts as full.
            let q = if class[to] & 3 == FULLISH { field::FULL } else { self.liquid_fill(from).min(field::FULL) };
            sign * ((vel.abs() * s).min(1.0) * q as f32) as i64
        };
        let (mut fu, mut fv) = (std::mem::take(&mut liq.fu), std::mem::take(&mut liq.fv));
        fu.par_chunks_mut(W + 1).enumerate().for_each(|(y, row)| {
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0.max(1)..=(x1 + 1).min(W - 1) {
                row[x] = flow(liq.u[ui(x, y)], y * W + x - 1, y * W + x);
            }
        });
        fv.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
            if y == 0 || y == H {
                return;
            }
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0..=x1 {
                row[x] = flow(liq.v[vi(x, y)], (y - 1) * W + x, y * W + x);
            }
        });
        // An empty cell takes only one liquid: the one that gives it most.
        let mut want = std::mem::take(&mut liq.want);
        want.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0..=x1 {
                let i = y * W + x;
                if self.cells[i].m.kind() != Kind::Empty || class[i] & 3 == SOLID {
                    continue;
                }
                let ins = [(fu[ui(x, y)], x > 0, i.wrapping_sub(1)), (-fu[ui(x + 1, y)], x + 1 < W, i + 1), (fv[vi(x, y)], y > 0, i.wrapping_sub(W)), (-fv[vi(x, y + 1)], y + 1 < H, i + W)];
                if let Some(&(_, _, j)) = ins.iter().filter(|&&(q, inside, _)| inside && q > 0).max_by_key(|&&(q, _, _)| q) {
                    row[x] = m(j);
                }
            }
        });
        let refuse = |q: &mut i64, from: usize, to: usize| {
            if *q != 0 && self.cells[to].m.kind() == Kind::Empty && want[to] != m(from) {
                *q = 0;
            }
        };
        fu.par_chunks_mut(W + 1).enumerate().for_each(|(y, row)| {
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0.max(1)..=(x1 + 1).min(W - 1) {
                let (a, b) = (y * W + x - 1, y * W + x);
                let q = &mut row[x];
                if *q > 0 { refuse(q, a, b) } else if *q < 0 { refuse(q, b, a) }
            }
        });
        fv.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
            if y == 0 || y == H {
                return;
            }
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0..=x1 {
                let (a, b) = ((y - 1) * W + x, y * W + x);
                let q = &mut row[x];
                if *q > 0 { refuse(q, a, b) } else if *q < 0 { refuse(q, b, a) }
            }
        });
        // No cell gives more than it holds: its outflows are cut down to that.
        let mut scale = std::mem::take(&mut liq.scale);
        scale.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0..=x1 {
                let i = y * W + x;
                if class[i] & 3 < WET {
                    continue;
                }
                let out = fu[ui(x + 1, y)].max(0) + (-fu[ui(x, y)]).max(0) + fv[vi(x, y + 1)].max(0) + (-fv[vi(x, y)]).max(0);
                let have = self.liquid_fill(i) as i64;
                if out > have {
                    row[x] = have as f64 / out as f64;
                }
            }
        });
        fu.par_chunks_mut(W + 1).enumerate().for_each(|(y, row)| {
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0.max(1)..=(x1 + 1).min(W - 1) {
                let q = &mut row[x];
                let from = if *q > 0 { y * W + x - 1 } else { y * W + x };
                *q = (*q as f64 * scale[from]) as i64;
            }
        });
        fv.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
            if y == 0 || y == H {
                return;
            }
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0..=x1 {
                let q = &mut row[x];
                let from = if *q > 0 { (y - 1) * W + x } else { y * W + x };
                *q = (*q as f64 * scale[from]) as i64;
            }
        });
        // Each cell takes what flows in and gives what flows out, with the heat it carries.
        let world: &World = self;
        let changes: Vec<(usize, i64, f32, Option<Mat>)> = (0..H)
            .into_par_iter()
            .flat_map_iter(|y| {
                let (x0, x1) = rows[y].unwrap_or((1, 0));
                let (fu, fv) = (&fu, &fv);
                (x0..=x1).filter_map(move |x| {
                    let i = y * W + x;
                    let edges = [(fu[ui(x, y)], i.wrapping_sub(1)), (-fu[ui(x + 1, y)], i + 1), (fv[vi(x, y)], i.wrapping_sub(W)), (-fv[vi(x, y + 1)], i + W)];
                    if edges.iter().all(|&(q, _)| q == 0) {
                        return None;
                    }
                    let had = if world.cells[i].m.kind() == Kind::Liquid { world.liquid_fill(i) as i64 } else { 0 };
                    let (mut q, mut heat) = (had, had as f64 * world.temp[i] as f64);
                    let mut new = None;
                    for &(f, j) in &edges {
                        if f > 0 {
                            q += f;
                            heat += f as f64 * world.temp[j] as f64;
                            new = new.or(Some(world.cells[j].m));
                        } else if f < 0 {
                            q += f;
                            heat += f as f64 * world.temp[i] as f64;
                        }
                    }
                    let t = if q > 0 { (heat / q as f64) as f32 } else { world.temp[i] };
                    Some((i, q, t, if had == 0 { new } else { None }))
                })
            })
            .collect();
        // The scratch back to blank where it was used, for the next sub-step.
        let clear_rows = |y: usize| rows[y];
        fu.par_chunks_mut(W + 1).enumerate().for_each(|(y, row)| {
            if let Some((x0, x1)) = clear_rows(y) {
                row[x0..=(x1 + 1).min(W)].fill(0);
            }
        });
        fv.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
            if let Some((x0, x1)) = rows.get(y).copied().flatten() {
                row[x0..=x1].fill(0);
            }
        });
        want.par_chunks_mut(W).zip(scale.par_chunks_mut(W)).enumerate().for_each(|(y, (w, sc))| {
            if let Some((x0, x1)) = clear_rows(y) {
                w[x0..=x1].fill(Mat::Empty);
                sc[x0..=x1].fill(1.0);
            }
        });
        (liq.fu, liq.fv, liq.want, liq.scale) = (fu, fv, want, scale);
        for (i, q, t, new) in changes {
            if q <= 0 {
                if self.cells[i].m.kind() == Kind::Liquid {
                    self.cells[i] = Cell { m: Mat::Empty, stamp: self.frame };
                    self.clear_states(i);
                    self.vel[i] = 0;
                }
            } else {
                if let Some(nm) = new {
                    self.cells[i] = Cell { m: nm, stamp: self.frame };
                    self.clear_states(i);
                    self.latent[i] = 0.0;
                    self.vel[i] = 0;
                }
                self.temp[i] = t;
                self.fill[i] = if q as u32 == field::FULL { 0 } else { q as u32 };
            }
            self.revision += 1;
            self.zones.wake(i);
        }
        self.overflow(liq);
    }

    /// A cell holding more than full pushes the extra up, into air or the same liquid.
    fn overflow(&mut self, liq: &Liquids) {
        let full = field::FULL;
        for y in (1..H).rev() {
            let Some((x0, x1)) = liq.span(y) else { continue };
            for x in x0..=x1 {
                let i = y * W + x;
                if self.cells[i].m.kind() != Kind::Liquid || self.fill[i] <= full {
                    continue;
                }
                let j = i - W;
                let m = self.cells[i].m;
                let up = match self.cells[j].m {
                    Mat::Empty if self.mask[j] == 0 => 0,
                    n if n == m => self.liquid_fill(j),
                    _ => continue,
                };
                let extra = self.fill[i] - full;
                if up == 0 {
                    self.cells[j] = Cell { m, stamp: self.frame };
                    self.clear_states(j);
                    self.temp[j] = self.temp[i];
                }
                let q = up + extra;
                self.fill[j] = if q == full { 0 } else { q };
                self.fill[i] = 0;
                self.revision += 1;
                self.zones.wake(j);
            }
        }
    }

    /// Units of liquid in all the world, by liquid (for the tests).
    #[cfg(test)]
    pub(super) fn liquid_units(&self, m: Mat) -> u64 {
        (0..W * H).filter(|&i| self.cells[i].m == m).map(|i| self.liquid_fill(i) as u64).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        let mut w = World::new_static();
        w.set_liquid_physics(true);
        w
    }

    fn rect(w: &mut World, x0: i32, y0: i32, x1: i32, y1: i32, m: Mat) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                w.set(x, y, m);
            }
        }
    }

    /// A tank of brick, inside (x0, y0)..=(x1, y1).
    fn tank(w: &mut World, x0: i32, y0: i32, x1: i32, y1: i32) {
        rect(w, x0 - 1, y0 - 1, x1 + 1, y1 + 1, Mat::Brick);
        rect(w, x0, y0, x1, y1, Mat::Empty);
    }

    /// The rightmost column of the tank holding at least a tenth of a cell of `m`.
    fn front(w: &World, m: Mat, x0: i32, y0: i32, x1: i32, y1: i32) -> i32 {
        (x0..=x1).rev().find(|&x| (y0..=y1).any(|y| w.raw(x, y) == m && w.liquid_fill(World::idx(x, y)) > field::FULL / 10)).unwrap_or(x0) - x0 + 1
    }

    /// A dam break: water 40 wide and 30 high at the left end of a tank 240 x 50.
    fn dam(m: Mat) -> World {
        let mut w = world();
        tank(&mut w, 100, 400, 339, 449);
        rect(&mut w, 100, 420, 139, 449, m);
        w.temp.fill(if m == Mat::Lava { 1150.0 } else { 15.0 });
        w
    }

    #[test]
    #[ignore]
    fn probe_dam() {
        let mut w = dam(Mat::Water);
        let before = w.liquid_units(Mat::Water);
        let mut dump = String::new();
        for t in 1..=120 {
            w.step();
            if t % 20 == 0 {
                dump += &format!("t {t}\n");
                for y in 400..=449 {
                    for x in 100..=339 {
                        let q = if w.raw(x, y) == Mat::Water { w.liquid_fill(World::idx(x, y)) as f32 / field::FULL as f32 } else { 0.0 };
                        dump += &format!("{q:.2} ");
                    }
                    dump.push('\n');
                }
            }
            if t % 10 == 0 {
                println!("{:.2} s: front {:.1} m, water {:.4}", t as f32 / 60.0, front(&w, Mat::Water, 100, 400, 339, 449) as f32 * CELL_M, w.liquid_units(Mat::Water) as f64 / before as f64);
            }
        }
        if let Ok(out) = std::env::var("DAM_OUT") {
            std::fs::write(out, dump).unwrap();
        }
    }

    /// Water in a U-tube (two columns 6 wide, joined by a bend 6 high), 20 cells higher on the
    /// left: the height of the left column, every step, for 8 s.
    fn u_tube(m: Mat, steps: usize) -> Vec<f32> {
        let mut w = world();
        // Left column x 200..205, right 230..235, both open at the top; bend y 440..445.
        rect(&mut w, 190, 370, 245, 455, Mat::Brick);
        rect(&mut w, 200, 360, 205, 445, Mat::Empty);
        rect(&mut w, 230, 360, 235, 445, Mat::Empty);
        rect(&mut w, 206, 440, 229, 445, Mat::Empty);
        rect(&mut w, 206, 440, 229, 445, m);
        rect(&mut w, 200, 400, 205, 445, m);
        rect(&mut w, 230, 420, 235, 445, m);
        w.temp.fill(15.0);
        (0..steps)
            .map(|_| {
                w.step();
                let col = |x0: i32| -> f32 {
                    let units: u64 = (380..440).flat_map(|y| (x0..x0 + 6).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y) == m).map(|(x, y)| w.liquid_fill(World::idx(x, y)) as u64).sum();
                    units as f32 / field::FULL as f32 / 6.0
                };
                col(200)
            })
            .collect()
    }

    #[test]
    #[ignore]
    fn probe_u_tube() {
        let h = u_tube(Mat::Water, 960);
        for (t, q) in h.iter().enumerate().step_by(6) {
            println!("{:.1} s {:.1}", t as f32 / 60.0, q);
        }
    }

    /// Liquid `m` at `t` °C, 30 wide and 20 high, let go on a flat floor 240 long: its front after
    /// `secs` seconds, in cells.
    fn spread(m: Mat, t: f32, secs: usize) -> (i32, World) {
        let mut w = world();
        tank(&mut w, 100, 400, 339, 449);
        rect(&mut w, 100, 430, 129, 449, m);
        w.temp.fill(15.0);
        rect_temp(&mut w, 100, 430, 129, 449, t);
        for _ in 0..secs * 60 {
            w.step();
        }
        (front(&w, m, 100, 400, 339, 449), w)
    }

    fn rect_temp(w: &mut World, x0: i32, y0: i32, x1: i32, y1: i32, t: f32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                w.temp[World::idx(x, y)] = t;
            }
        }
    }

    #[test]
    #[ignore]
    fn probe_spread() {
        for (m, t) in [(Mat::Water, 15.0), (Mat::Oil, 15.0), (Mat::Lava, 1150.0), (Mat::Lava, 950.0)] {
            for secs in [1, 3, 6] {
                let (f, w) = spread(m, t, secs);
                println!("{m:?} {t} {secs} s: front {:.1} m, units {:.1} cells, stone {}", f as f32 * CELL_M, w.liquid_units(m) as f32 / field::FULL as f32, (400..450).flat_map(|y| (100..340).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y) != m && w.raw(x, y) != Mat::Empty && w.raw(x, y).kind() != Kind::Gas).count());
            }
        }
    }

    #[test]
    fn a_dam_breaks_at_the_speed_of_real_water_and_none_is_lost() {
        for m in [Mat::Water, Mat::Oil] {
            let mut w = dam(m);
            let before = w.liquid_units(m);
            for _ in 0..60 {
                w.step();
            }
            // 40 cells (6.7 m) at first; after 1 s a real front of water 5 m deep is about 13 m
            // out (the cells' water was at 36).
            let f = front(&w, m, 100, 400, 339, 449) as f32 * CELL_M;
            assert!((9.0..15.0).contains(&f), "{m:?}: {f} m");
            // (Water also dries a little into the air.)
            let after = w.liquid_units(m);
            assert!(after <= before && before - after < before / 10_000, "{m:?}: {before} -> {after}");
        }
    }

    #[test]
    fn water_swings_in_a_u_tube_at_its_period_and_settles() {
        let h = u_tube(Mat::Water, 420);
        // The lowest point of the first swing, and the highest of the second.
        let (t_low, low) = h.iter().enumerate().take(300).fold((0, f32::MAX), |a, (t, &q)| if q < a.1 { (t, q) } else { a });
        let (t_high, high) = h.iter().enumerate().skip(t_low).fold((0, 0.0), |a, (t, &q)| if q > a.1 { (t, q) } else { a });
        // 96 cells of water (16 m) in all: 2π √(L / 2g) = 5.7 s, half of it from low to high.
        let half = (t_high - t_low) as f32 / 60.0;
        assert!((2.3..3.4).contains(&half), "half period {half} s");
        // It swings past the middle (30) and back, a little less each time.
        assert!(low < 26.0 && high > 31.0 && high < 40.0, "{low} {high}");
    }

    #[test]
    fn a_still_pool_stays_still_and_falls_asleep() {
        let mut w = world();
        tank(&mut w, 100, 400, 339, 449);
        rect(&mut w, 100, 430, 339, 449, Mat::Water);
        w.temp.fill(15.0);
        let before = w.liquid_units(Mat::Water);
        for _ in 0..240 {
            w.step();
        }
        let fastest = w.liq.u.iter().chain(w.liq.v.iter()).fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(fastest < 0.05, "{fastest} m/s");
        let after = w.liquid_units(Mat::Water);
        assert!(after <= before && before - after < before / 10_000, "{before} -> {after}");
        let awake = (0..sleep::CW * sleep::CH).filter(|&z| w.zones.awake[z]).count();
        // Only the zones along the surface (8), where the water dries into the air, stay awake;
        // the cells' own water keeps 11 of them awake.
        assert!(awake <= 8, "{awake} zones awake");
    }

    #[test]
    fn lava_creeps_where_water_runs() {
        let (water, _) = spread(Mat::Water, 15.0, 4);
        let (hot, _) = spread(Mat::Lava, 1150.0, 4);
        let (cold, _) = spread(Mat::Lava, 900.0, 4);
        assert!(hot * 2 < water, "{hot} {water}");
        // Cooler, it is thicker and holds more under its yield stress (but a heap 3 m high
        // weighs far more than the few kPa it holds: it still sags).
        assert!(cold < hot, "{cold} {hot}");
    }

    #[test]
    fn the_liquids_take_the_same_turn_on_one_core_and_on_four() {
        let run = |cores: usize| {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(cores).build().unwrap();
            pool.install(|| {
                let mut w = dam(Mat::Water);
                for _ in 0..30 {
                    w.step();
                }
                (0..W * H).map(|i| (w.cells[i].m, w.liquid_fill(i))).collect::<Vec<_>>()
            })
        };
        assert!(run(1) == run(4));
    }
}
