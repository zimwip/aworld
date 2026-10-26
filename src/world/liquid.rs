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
//! 4. the pressure that keeps every full cell from filling up or emptying is solved to
//!    convergence (conjugate gradients on a multigrid, `mgpcg.rs`, starting from the last
//!    step's pressure: a few iterations whatever the size of the body); at a free surface it is
//!    the pressure of the gas over it (its pocket's, see `pockets.rs`: siphons and boilers keep
//!    working), and a gas lying in the open on the liquid is a wall to it;
//! 5. each edge passes velocity × time × the liquid of the cell upstream: the liquid moves, with
//!    its heat, and nothing is created or lost.
//!
//! Only the zones that run take part: a sleeping zone is liquid at rest, which neither moves nor
//! gives, its pressure a fixed boundary for the zones around it (set again after each solve from
//! the running liquid beside it, so that it follows the weather over the body). Two bodies of
//! liquid that do not touch never share a pressure, so each is in effect solved on its own. A lone cell of water falling through the air
//! is a drop of the cells' turn (rain); a drop that lands gives its speed to the edges under it.

use super::par::Shared;
use super::*;
use rayon::prelude::*;

/// A cell, in metres.
pub const CELL_M: f32 = 1.0 / 6.0;
/// The weight of a cell of water, kN per metre of depth (ρ g h²).
pub const CELL_OF_WATER_KN: f32 = 1000.0 * G * CELL_M * CELL_M / 1000.0;
/// A step, in seconds.
pub(super) const DT: f32 = 1.0 / 60.0;
pub(super) const G: f32 = 9.81;
/// One atmosphere, in pascals.
pub(super) const ATM: f32 = 101_325.0;
/// The pressure solver stops once no pressure would move by more than this, Pa (a tenth of a
/// millimetre of water).
pub(super) const SETTLED: f32 = 1.0;
/// Iterations of the pressure solver a step, at most (each is a multigrid V-cycle and a step
/// of conjugate gradients; a body at rest takes one, a splash three or four).
pub(super) const MAX_ITER: u32 = 12;
/// ... or until it is right to this part of where it started (a body in motion: its pressure
/// moves by thousands of pascals a step, and a part in a thousand of that is well under what
/// the cells carry; a body at rest starts close, and goes to `SETTLED`).
pub(super) const RELATIVE: f32 = 1e-3;
/// Sweeps of the viscosity a sub-step.
pub(super) const VISCOUS_SWEEPS: usize = 12;
/// Slower than this (m/s), an edge between two cells of liquid stands still: still water stops,
/// so that its zones can sleep.
pub(super) const REST: f32 = 0.01;
/// The part of its gap to full that a cell inside a body makes up a sub-step.
pub(super) const REFILL: f32 = 0.1;
/// A cell holds a pressure from this part of full (1/HOLDS) up.
pub(super) const HOLDS: u32 = 2;
/// Liquid away from any full cell (drops, spray, a film) goes no faster than this, m/s: the air
/// holds it back (rain falls at about this speed).
pub(super) const SPRAY: f32 = 9.0;
/// Liquid thrown at the air faster than this (m/s) comes off as drops.
pub(super) const THROW: f32 = 2.0;
/// The most of a cell that comes off as drops in a sub-step, thrown sideways or up.
pub(super) const THROW_SHARE: f32 = 0.5;
/// What the drag of the air leaves of a drop's speed after a step.
pub(super) const DROP_DRAG: f32 = 0.998;
/// At most this many sub-steps a step (fast flows take smaller ones).
pub(super) const MAX_SUB: usize = 4;
/// A cell left with this little liquid or less (units of fill, 1/8192 of a cell) is emptied: so
/// little moves no more (what an edge passes rounds to nothing) and would hang in the air for
/// ever, keeping its zone awake.
pub(super) const CRUMB: u32 = 8;
/// The free surface (an edge between a full cell and the air or a film, or between a film and
/// the air) loses this part of its speed a second: waves and the slosh of a pool die out, where
/// the model alone (no surface tension, no breaking) would keep them for ever.
pub(super) const CALM: f32 = 3.0;

/// Whether new worlds take their liquids by physics: `AWORLD_LIQUIDS=1`.
pub(super) fn physics_by_default() -> bool {
    std::env::var_os("AWORLD_LIQUIDS").is_some_and(|v| v != "0")
}

/// What a cell is to the liquids.
pub const SOLID: u8 = 0;
pub const AIR: u8 = 1;
/// Liquid, less than half full: it moves with the edges around it, but holds no pressure.
pub const WET: u8 = 2;
/// Liquid, at least half full: it holds a pressure.
pub const FULLISH: u8 = 3;

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
    pub(super) u: Vec<f32>,
    /// Downward velocity on the top edge of each cell, W a row, H + 1 rows, m/s.
    pub(super) v: Vec<f32>,
    /// Pressure in each cell over the ambient one, Pa (kept from step to step).
    pub(super) p: Vec<f32>,
    /// What each cell is (`SOLID`, `AIR`, `WET`, `FULLISH`), and whether its zone runs (+ 4).
    pub(super) class: Vec<u8>,
    /// For each row, the span of cells (first, last) the liquids work on this sub-step: the
    /// liquid that runs and the cells around it. `None`: nothing to do in the row.
    pub(super) rows: Vec<Option<(usize, usize)>>,
    /// Scratch for the advection and the viscosity.
    pub(super) u2: Vec<f32>,
    pub(super) v2: Vec<f32>,
    pub(super) u3: Vec<f32>,
    pub(super) v3: Vec<f32>,
    /// Viscosity of each cell, m²/s (0: not thick), and whether any cell was thick.
    pub(super) nu: Vec<f32>,
    pub(super) thick: bool,
    /// Scratch kept from step to step (allocated once, and cleared only where it was used): the
    /// divergence of each cell, what each edge passes, the liquid each empty cell takes and the
    /// cut of each cell's outflows.
    pub(super) rhs: Vec<f32>,
    pub(super) fu: Vec<i64>,
    pub(super) fv: Vec<i64>,
    pub(super) want: Vec<Mat>,
    pub(super) scale: Vec<f64>,
    /// The drops in flight.
    pub(super) drops: Vec<Drop>,
    /// The last pressure solve: when it began, and how many iterations it took (for the profile).
    /// The clock of the last pressure solve and its iterations (for the profile).
    pub(super) solve: (prof::Clock, u32),
    /// The pressure solver, with its grids.
    pub(super) solver: mgpcg::Solver,
    /// The same on the graphics card, for the big systems (`None` until asked for, and where
    /// there is no card).
    pub(super) gpu: Option<Option<mgpcg_gpu::PressureGpu>>,
    /// The whole turn on the card (`None` until asked for, and where there is no card).
    pub(super) card: Option<Option<liquid_gpu::LiquidGpu>>,
    /// The fastest edge at the end of the last step, m/s (the card's turn keeps it to choose
    /// its sub-steps without a look at its grids).
    pub(super) fastest: f32,
}

/// A drop of liquid thrown off a body (spray, a splash): it flies off the grid, under gravity
/// and the drag of the air, until it lands back in liquid or on the ground.
#[derive(Clone)]
pub struct Drop {
    /// Where it is, in cells.
    pub x: f32,
    pub y: f32,
    /// Its velocity, m/s (down positive).
    pub(super) vx: f32,
    pub(super) vy: f32,
    pub m: Mat,
    /// Units of liquid (`field::FULL` a cell).
    pub q: u32,
    pub(super) t: f32,
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
            self.u3 = vec![0.0; (W + 1) * H];
            self.v3 = vec![0.0; W * (H + 1)];
            self.nu = vec![0.0; W * H];
            self.rhs = vec![0.0; W * H];
            self.fu = vec![0; (W + 1) * H];
            self.fv = vec![0; W * (H + 1)];
            self.want = vec![Mat::Empty; W * H];
            self.scale = vec![1.0; W * H];
        }
    }

    /// The drops in flight.
    pub fn drops(&self) -> &[Drop] {
        &self.drops
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
pub(super) fn ui(x: usize, y: usize) -> usize {
    y * (W + 1) + x
}
#[inline]
pub(super) fn vi(x: usize, y: usize) -> usize {
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

/// The least and the most of the four nodes of grid `a` around (x, y) (as `sample` reads them).
fn around(a: &[f32], w: usize, h: usize, x: f32, y: f32) -> (f32, f32) {
    let x = x.clamp(0.0, (w - 1) as f32 - 0.001);
    let y = y.clamp(0.0, (h - 1) as f32 - 0.001);
    let (x0, y0) = (x as usize, y as usize);
    let n = [a[y0 * w + x0], a[y0 * w + x0 + 1], a[(y0 + 1) * w + x0], a[(y0 + 1) * w + x0 + 1]];
    (n.iter().copied().fold(f32::MAX, f32::min), n.iter().copied().fold(f32::MIN, f32::max))
}

impl World {
    /// Turns the physics of liquids on or off. Off, a cell of liquid is always full again: the
    /// cells under half full go and the others fill up, about as much as there was.
    pub fn set_liquid_physics(&mut self, on: bool) {
        if on {
            self.set_water_field(false);
            // The liquids no longer take the cells' turn, so that turn can run on every core.
            if turn::parallel_with_liquids() {
                self.parallel_cells = true;
            }
        } else {
            // The drops in flight come down where they are (a cell for at least half of one).
            for d in std::mem::take(&mut self.liq.drops) {
                let (x, y) = (d.x as i32, d.y as i32);
                if d.q >= field::FULL / 2 && Self::inb(x, y) && self.raw(x, y) == Mat::Empty && self.mask_free(x, y) {
                    self.set(x, y, d.m);
                    self.temp[Self::idx(x, y)] = d.t;
                }
            }
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

    /// The drops of liquid in flight.
    pub fn liquid_drops(&self) -> &[Drop] {
        self.liq.drops()
    }

    /// The pressure of the liquid at (x, y) over the ambient one, Pa, while the liquids run by
    /// their physics (None in a cell that holds no pressure).
    pub fn liquid_pa(&self, x: usize, y: usize) -> Option<f32> {
        let i = y * W + x;
        (self.liquid_physics && self.liq.p.len() == W * H && self.liq.class[i] & 3 == FULLISH && self.cells[i].m.kind() == Kind::Liquid).then(|| self.liq.p[i])
    }

    /// The push of the liquids on the solid cell `i`, from the pressure on each of its wet sides,
    /// as a weight (x, y; down positive): in cells of water, the column of water that would press
    /// as hard (none unless the liquids run by their physics).
    pub(super) fn liquid_push(&self, i: usize) -> (f32, f32) {
        if !self.liquid_physics || self.liq.p.len() != W * H {
            return (0.0, 0.0);
        }
        let (x, y) = (i % W, i / W);
        let head = |j: usize| -> f32 {
            if self.liq.class[j] & 3 == FULLISH && self.cells[j].m.kind() == Kind::Liquid {
                (self.liq.p[j] / (1000.0 * G * CELL_M)).max(0.0)
            } else {
                0.0
            }
        };
        let (mut fx, mut fy) = (0.0, 0.0);
        if x > 0 {
            fx += head(i - 1);
        }
        if x + 1 < W {
            fx -= head(i + 1);
        }
        if y > 0 {
            fy += head(i - W);
        }
        if y + 1 < H {
            fy -= head(i + W);
        }
        (fx, fy)
    }

    /// Units of liquid in cell `i` (`field::FULL` for a full cell).
    #[inline]
    pub(super) fn liquid_fill(&self, i: usize) -> u32 {
        self.water_fill(i)
    }

    /// Whether the cell at (x, y) is a lone cell of liquid (nothing liquid beside or over it):
    /// with air under it, it is a drop of the cells' turn.
    pub(super) fn lone_liquid(&self, x: i32, y: i32) -> bool {
        [(x - 1, y), (x + 1, y), (x, y - 1)].iter().all(|&(nx, ny)| self.at(nx, ny).kind() != Kind::Liquid)
    }

    /// The class of cell (x, y) as the liquids last saw it (`SOLID`, `AIR`, `WET`, `FULLISH`,
    /// with bit 4 set where its zone runs), for the simulation view; None before the first step.
    pub fn liquid_class(&self, x: usize, y: usize) -> Option<u8> {
        self.liq.class.get(y * W + x).copied()
    }

    /// Whether the liquids put a drop's velocity (m/s) at the centre of cell (x, y); None before
    /// the first step.
    pub fn liquid_velocity(&self, x: usize, y: usize) -> Option<(f32, f32)> {
        if self.liq.u.len() != (W + 1) * H {
            return None;
        }
        Some((0.5 * (self.liq.u[ui(x, y)] + self.liq.u[ui(x + 1, y)]), 0.5 * (self.liq.v[vi(x, y)] + self.liq.v[vi(x, y + 1)])))
    }

    /// The drops' velocity, m/s (down positive).
    pub fn drop_velocity(d: &Drop) -> (f32, f32) {
        (d.vx, d.vy)
    }

    /// The rows the card works on (the band of the last step given to it), when the liquids
    /// run on a card.
    pub fn liquid_card_rows(&self) -> Option<(usize, usize)> {
        self.liq.card.as_ref().and_then(|c| c.as_ref()).and_then(|c| c.rows())
    }

    /// What the card did with the last step, into the world: before the cells' turn, so that
    /// the rain of the cells and the card never move the same liquid at once (the card works
    /// from the cells as they were after the cells' turn, and what it did is in before the next).
    pub(super) fn take_liquids_from_card(&mut self) {
        let mut liq = std::mem::take(&mut self.liq);
        let mut card = liq.card.take();
        if let Some(Some(c)) = card.as_mut() {
            self.collect_liquids(&mut liq, c);
        }
        liq.card = card;
        self.liq = liq;
    }

    /// The liquids' turn.
    pub(super) fn step_liquids(&mut self) {
        let mut liq = std::mem::take(&mut self.liq);
        liq.ready();
        // The whole turn on the card, where there is one (and it is wanted): the card works on
        // the step while the CPU does the rest of it, and the CPU takes its result at the next.
        if gpu::wanted() != gpu::Use::Off {
            let mut card = liq.card.take().unwrap_or_else(liquid_gpu::LiquidGpu::new);
            if let Some(c) = card.as_mut() {
                self.step_liquids_on_card(&mut liq, c);
            }
            liq.card = Some(card);
            if liq.card.as_ref().is_some_and(Option::is_some) {
                self.liq = liq;
                return;
            }
        }
        self.fly_drops(&mut liq);
        let clock = self.prof.start();
        self.classify(&mut liq, false);
        self.prof.stop(prof::LIQ_CLASSIFY, clock);
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
                let clock = self.prof.start();
                self.classify(&mut liq, true);
                self.prof.stop(prof::LIQ_CLASSIFY, clock);
            }
            let clock = self.prof.start();
            self.forces(&mut liq, dt);
            self.prof.stop(prof::LIQ_FORCES, clock);
            let clock = self.prof.start();
            self.viscosity(&mut liq, dt);
            self.prof.stop(prof::LIQ_VISCOSITY, clock);

            // The pressure once a step, on the last sub-step: the sub-steps are there for the
            // transport (a cell at most each), and the pressure of the step before holds the
            // liquid well enough through the first ones. A solve is nearly half of the step;
            // one a sub-step was three and a half a step on the scene "liquides".
            if k + 1 == sub {
                let clock = self.prof.start();
                self.pressure(&mut liq, dt);
                self.prof.stop(prof::LIQ_PRESSURE, clock);
                let (solve_clock, iterations) = std::mem::take(&mut liq.solve);
                self.prof.stop_n(prof::LIQ_ITERATIONS, solve_clock, iterations);
            }
            let clock = self.prof.start();
            self.throw_drops(&mut liq);
            self.prof.stop(prof::LIQ_DROPS, clock);
            let clock = self.prof.start();
            self.carry_liquids(&mut liq, dt);
            self.prof.stop(prof::LIQ_CARRY, clock);
        }
        self.liq = liq;
    }

    /// Liquid thrown at the air faster than `THROW` comes off as drops: a share of the cell that
    /// grows with the speed, all of it when it falls into the air, and so do the crumbs left
    /// alone in the air (a thin film with air under it and no body beside it).
    fn throw_drops(&mut self, liq: &mut Liquids) {
        let (class, rows, u, v) = (&liq.class, &liq.rows, &liq.u, &liq.v);
        let world: &World = self;
        let throws: Vec<(usize, u32, f32, f32, f32, f32)> = (0..H)
            .into_par_iter()
            .flat_map_iter(|y| {
                let (x0, x1) = rows[y].unwrap_or((1, 0));
                (x0..=x1).filter_map(move |x| {
                    let i = y * W + x;
                    if class[i] & 3 < WET || class[i] & 4 == 0 {
                        return None;
                    }
                    // Air, or the thin film of spray over a surface.
                    let air = |nx: usize, ny: usize| {
                        nx < W && ny < H && {
                            let j = ny * W + nx;
                            match class[j] & 3 {
                                AIR => world.cells[j].m == Mat::Empty,
                                WET => world.cells[j].m == world.cells[i].m && world.liquid_fill(j) < field::FULL / 4,
                                _ => false,
                            }
                        }
                    };
                    // The speed out of the cell into the air on each side: right, left, down, up.
                    let outs = [
                        (if air(x + 1, y) { u[ui(x + 1, y)] } else { 0.0 }, 0.5, 0.0),
                        (if air(x.wrapping_sub(1), y) { -u[ui(x, y)] } else { 0.0 }, -0.5, 0.0),
                        (if air(x, y + 1) { v[vi(x, y + 1)] } else { 0.0 }, 0.0, 0.5),
                        (if air(x, y.wrapping_sub(1)) { -v[vi(x, y)] } else { 0.0 }, 0.0, -0.5),
                    ];
                    let (k, &(speed, ox, oy)): (usize, &(f32, f32, f32)) = outs.iter().enumerate().max_by(|a, b| a.1 .0.total_cmp(&b.1 .0))?;
                    let fill = world.liquid_fill(i).min(field::FULL);
                    let (mut vx, mut vy) = (0.5 * (u[ui(x, y)] + u[ui(x + 1, y)]), 0.5 * (v[vi(x, y)] + v[vi(x, y + 1)]));
                    // It leaves at the speed of the side it goes out by.
                    match k {
                        0 | 1 => vx = speed * ox.signum(),
                        _ => vy = speed * oy.signum(),
                    }
                    let mut share = if speed > THROW { ((speed - THROW) / THROW).min(1.0) * THROW_SHARE } else { 0.0 };
                    if k == 2 {
                        // Falling: a body of liquid holds together (its bottom does not peel
                        // off), but a thin stream or a lip with nothing over it breaks into drops.
                        let held = y > 0 && class[i - W] & 3 == FULLISH;
                        share = if held { 0.0 } else if speed > THROW { 1.0 } else { 0.0 };
                    }
                    let lone = fill < field::FULL / 4
                        && air(x, y + 1)
                        && [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)].iter().all(|&(nx, ny)| nx >= W || ny >= H || class[ny * W + nx] & 3 != FULLISH);
                    if lone && vx.hypot(vy) > 0.5 {
                        share = 1.0;
                    }
                    let q = (fill as f32 * share) as u32;
                    (q > field::FULL / 50).then_some((i, q, x as f32 + 0.5 + ox, y as f32 + 0.5 + oy, vx, vy))
                })
            })
            .collect();
        for (i, q, x, y, vx, vy) in throws {
            let left = self.liquid_fill(i).saturating_sub(q);
            let (m, t) = (self.cells[i].m, self.temp[i]);
            // What the cell is now, for the carrying that follows.
            let run = liq.class[i] & 4;
            if left == 0 {
                self.cells[i] = Cell { m: Mat::Empty, stamp: self.frame };
                self.clear_states(i);
                self.vel[i] = 0;
                liq.class[i] = AIR | run;
            } else {
                self.fill[i] = left;
                if left < field::FULL / HOLDS {
                    liq.class[i] = WET | run;
                }
            }
            // A little scatter, so that a sheet of spray breaks up.
            let jitter = |r: u32| (r % 1000) as f32 / 1000.0 - 0.5;
            let (jx, jy) = (jitter(self.rand()), jitter(self.rand()));
            liq.drops.push(Drop { x, y, vx: vx + jx, vy: vy + jy, m, q, t });
            self.revision += 1;
            self.zones.wake(i);
        }
    }

    /// Moves the drops in flight a step; those that land go back into the liquid (or make a cell
    /// of it on the ground) with their speed and their heat.
    pub(super) fn fly_drops(&mut self, liq: &mut Liquids) {
        if liq.drops.is_empty() {
            return;
        }
        let mut drops = std::mem::take(&mut liq.drops);
        let s = DT / CELL_M;
        drops.retain_mut(|d| {
            d.vy += G * DT;
            d.vx *= DROP_DRAG;
            d.vy *= DROP_DRAG;
            let (dx, dy) = (d.vx * s, d.vy * s);
            let n = dx.abs().max(dy.abs()).ceil().max(1.0) as usize;
            for _ in 0..n {
                let (nx, ny) = (d.x + dx / n as f32, d.y + dy / n as f32);
                let (ix, iy) = (nx.floor() as i32, ny.floor() as i32);
                if ix < 0 || ix >= W as i32 || iy >= H as i32 {
                    // Off the side or the bottom of the world: gone.
                    return false;
                }
                if iy >= 0 {
                    let j = Self::idx(ix, iy);
                    let c = self.cells[j].m;
                    if c == d.m && self.mask[j] == 0 && self.liquid_fill(j) >= field::FULL / 2 {
                        self.land_drop(liq, d, j);
                        return false;
                    }
                    if self.mask[j] != 0 || !matches!(c.kind(), Kind::Empty | Kind::Gas) && c != d.m {
                        // Against the ground, a wall or another liquid: it stays in the last free cell.
                        let (px, py) = (d.x.floor() as i32, d.y.floor() as i32);
                        if Self::inb(px, py) {
                            self.land_drop(liq, d, Self::idx(px, py));
                        }
                        return false;
                    }
                }
                (d.x, d.y) = (nx, ny);
            }
            true
        });
        liq.drops = drops;
    }

    /// Drop `d` lands in cell `j` (liquid of its own, or a free cell).
    fn land_drop(&mut self, liq: &mut Liquids, d: &Drop, j: usize) {
        let had = if self.cells[j].m == d.m { self.liquid_fill(j) } else { 0 };
        if had == 0 {
            if !matches!(self.cells[j].m.kind(), Kind::Empty | Kind::Gas) || self.mask[j] != 0 {
                return;
            }
            self.cells[j] = Cell { m: d.m, stamp: self.frame };
            self.clear_states(j);
            self.latent[j] = 0.0;
            self.vel[j] = 0;
            self.temp[j] = d.t;
        } else {
            self.temp[j] = (self.temp[j] * had as f32 + d.t * d.q as f32) / (had + d.q) as f32;
        }
        let q = had + d.q;
        self.fill[j] = if q == field::FULL { 0 } else { q };
        // Its speed goes to the edges of the cell, in proportion to what it brings.
        let w = d.q as f32 / q as f32;
        let (x, y) = (j % W, j / W);
        if let Some(Some(card)) = liq.card.as_mut() {
            for e in [ui(x, y), ui(x + 1, y)] {
                card.patches_up.push(liquid_gpu::Patch { edge: e as u32, mode: 1, value: d.vx, weight: w });
            }
            for e in [vi(x, y), vi(x, y + 1)] {
                card.patches_up.push(liquid_gpu::Patch { edge: e as u32 | 0x8000_0000, mode: 1, value: d.vy, weight: w });
            }
        } else if liq.u.len() == (W + 1) * H {
            for e in [ui(x, y), ui(x + 1, y)] {
                liq.u[e] += w * (d.vx - liq.u[e]);
            }
            for e in [vi(x, y), vi(x, y + 1)] {
                liq.v[e] += w * (d.vy - liq.v[e]);
            }
        }
        self.revision += 1;
        self.zones.wake(j);
    }

    /// What each cell is to the liquids this sub-step, and the span of each row to work on.
    fn classify(&self, liq: &mut Liquids, again: bool) {
        let awake = &self.zones.awake;
        // Between two sub-steps, only the spans change (the liquid runs there and nowhere else),
        // and a class reads at most the cells beside: the cells a cell further are looked at
        // again, the rest keep their class.
        let old = &liq.rows;
        let range = |y: usize| -> (usize, usize) {
            if !again {
                return (0, W - 1);
            }
            let mut span: Option<(usize, usize)> = None;
            for yy in y.saturating_sub(1)..=(y + 1).min(H - 1) {
                if let Some((a, b)) = old[yy] {
                    span = Some(span.map_or((a, b), |(c, d)| (a.min(c), b.max(d))));
                }
            }
            span.map_or((1, 0), |(a, b)| (a.saturating_sub(1), (b + 1).min(W - 1)))
        };
        // The first and last cell of each row that holds liquid that runs.
        let ends: Vec<(usize, usize)> = liq
            .class
            .par_chunks_mut(W)
            .enumerate()
            .map(|(y, row)| {
                let (mut first, mut last) = (usize::MAX, 0);
                let (a, b) = range(y);
                for (x, c) in row.iter_mut().enumerate().take(b + 1).skip(a) {
                    let i = y * W + x;
                    let run = awake[y / sleep::CS * sleep::CW + x / sleep::CS];
                    let what = if self.mask[i] != 0 {
                        SOLID
                    } else {
                        match self.cells[i].m.kind() {
                            Kind::Empty => AIR,
                            // A gas in the open (a cloud lying on a lake, steam over a bath) is a
                            // wall to the liquid: the carrying never puts liquid into a gas, so a
                            // pressure that counted the gas as open air would push liquid at it
                            // for ever and pile it up in the cell beside. Shut in a pocket, the
                            // gas is open air at the pocket's pressure: that is what pushes the
                            // water of a boiler.
                            Kind::Gas if self.pocket_at(x as i32, y as i32).is_some() => AIR,
                            Kind::Gas => SOLID,
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
    /// the zones that run. The carrying is second order (MacCormack: carried forward, carried
    /// back, and half the gap corrected, kept within what was around): a plain carrying smears the
    /// velocities out, so that waves die and splashes never rise.
    fn forces(&self, liq: &mut Liquids, dt: f32) {
        let class = &liq.class;
        let kind = |x: i32, y: i32| if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 { SOLID } else { class[y as usize * W + x as usize] & 3 };
        let runs = |x: i32, y: i32| (0..W as i32).contains(&x) && (0..H as i32).contains(&y) && class[y as usize * W + x as usize] & 4 != 0;
        let wet = |c: u8| c >= WET;
        let step = dt / CELL_M;
        let rows = &liq.rows;
        let (u, v) = (&liq.u, &liq.v);
        // The velocity (along x, y) of each edge that is carried, or the value an edge that is
        // not carried takes (0 on a wall or in the air, its own speed in liquid that sleeps).
        let u_vel = |x: usize, y: usize| -> Result<(f32, f32), f32> {
            let (xi, yi) = (x as i32, y as i32);
            let (a, b) = (kind(xi - 1, yi), kind(xi, yi));
            let here = u[ui(x, y)];
            if a == SOLID || b == SOLID {
                Err(0.0)
            } else if !(wet(a) || wet(b)) || !(runs(xi - 1, yi) || runs(xi, yi)) {
                Err(if wet(a) || wet(b) { here } else { 0.0 })
            } else {
                // The downward velocity here: the mean of the four edges around.
                Ok((here, 0.25 * (v[vi(x - 1, y)] + v[vi(x, y)] + v[vi(x - 1, y + 1)] + v[vi(x, y + 1)])))
            }
        };
        let v_vel = |x: usize, y: usize| -> Result<(f32, f32), f32> {
            let (xi, yi) = (x as i32, y as i32);
            let (a, b) = (kind(xi, yi - 1), kind(xi, yi));
            let here = v[vi(x, y)];
            if a == SOLID || b == SOLID {
                Err(0.0)
            } else if !(wet(a) || wet(b)) || !(runs(xi, yi - 1) || runs(xi, yi)) {
                Err(if wet(a) || wet(b) { here } else { 0.0 })
            } else {
                Ok((0.25 * (u[ui(x, y - 1)] + u[ui(x + 1, y - 1)] + u[ui(x, y)] + u[ui(x + 1, y)]), here))
            }
        };
        // The grids carried are only touched near the spans: a row's edges are sampled from the
        // rows beside, up to two cells off (a cell of carrying, and the node after).
        let wide = |y: usize, w: usize| -> Option<(usize, usize)> {
            let mut span: Option<(usize, usize)> = None;
            for yy in y.saturating_sub(1)..=(y + 1).min(H - 1) {
                if let Some((a, b)) = rows[yy] {
                    span = Some(span.map_or((a, b), |(c, d)| (a.min(c), b.max(d))));
                }
            }
            span.map(|(a, b)| (a.saturating_sub(3), (b + 4).min(w - 1)))
        };
        // Carried forward (in the index space of each grid: edge (x, y) of `u` sits at x, y + 1/2
        // in cells, its index is (x, y)).
        liq.u2.par_chunks_mut(W + 1).enumerate().for_each(|(y, row)| {
            let Some((a, b)) = wide(y, W + 1) else { return };
            row[a..=b].copy_from_slice(&u[ui(a, y)..=ui(b, y)]);
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0..=(x1 + 1).min(W) {
                row[x] = match u_vel(x, y) {
                    Ok((vx, vy)) => sample(u, W + 1, H, x as f32 - vx * step, y as f32 - vy * step),
                    Err(keep) => keep,
                };
            }
        });
        liq.v2.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
            let Some((a, b)) = wide(y, W) else { return };
            row[a..=b].copy_from_slice(&v[vi(a, y)..=vi(b, y)]);
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0..=x1 {
                row[x] = match v_vel(x, y) {
                    Ok((vx, vy)) => sample(v, W, H + 1, x as f32 - vx * step, y as f32 - vy * step),
                    Err(keep) => keep,
                };
            }
        });
        // Carried back, the gap corrected, then gravity and the drag of the air.
        let (uf, vf) = (&liq.u2, &liq.v2);
        // The free surface is calmed (implicit, before gravity: a surface at rest stays so).
        let calm = 1.0 / (1.0 + CALM * dt);
        liq.u3.par_chunks_mut(W + 1).enumerate().for_each(|(y, row)| {
            let Some((x0, x1)) = rows[y] else { return };
            row[x0..=(x1 + 1).min(W)].copy_from_slice(&uf[ui(x0, y)..=ui((x1 + 1).min(W), y)]);
            for x in x0..=(x1 + 1).min(W) {
                let Ok((vx, vy)) = u_vel(x, y) else { continue };
                let (px, py) = (x as f32 - vx * step, y as f32 - vy * step);
                let back = sample(uf, W + 1, H, x as f32 + vx * step, y as f32 + vy * step);
                let (lo, hi) = around(u, W + 1, H, px, py);
                let to = (uf[ui(x, y)] + 0.5 * (u[ui(x, y)] - back)).clamp(lo, hi);
                // Away from any full cell, liquid in the air is slowed by it (drops and spray, at
                // most as fast as falling rain).
                let (a, b) = (kind(x as i32 - 1, y as i32), kind(x as i32, y as i32));
                let to = if a != b { to * calm } else { to };
                row[x] = if a != FULLISH && b != FULLISH { to.clamp(-SPRAY, SPRAY) } else { to };
            }
        });
        liq.v3.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
            let Some((x0, x1)) = rows[y] else { return };
            row[x0..=x1].copy_from_slice(&vf[vi(x0, y)..=vi(x1, y)]);
            for x in x0..=x1 {
                let Ok((vx, vy)) = v_vel(x, y) else { continue };
                let (px, py) = (x as f32 - vx * step, y as f32 - vy * step);
                let back = sample(vf, W, H + 1, x as f32 + vx * step, y as f32 + vy * step);
                let (lo, hi) = around(v, W, H + 1, px, py);
                let (xi, yi) = (x as i32, y as i32);
                let (a, b) = (kind(xi, yi - 1), kind(xi, yi));
                let was = (vf[vi(x, y)] + 0.5 * (v[vi(x, y)] - back)).clamp(lo, hi);
                let fall = if a != b { was * calm } else { was } + G * dt;
                row[x] = if a != FULLISH && b != FULLISH {
                    // A film lying on liquid or on the ground goes no faster than what is under it
                    // (it has nowhere to fall).
                    let fall = fall.clamp(-SPRAY, SPRAY);
                    if a < WET && b == WET && matches!(kind(xi, yi + 1), SOLID | FULLISH) { fall.min(v[vi(x, y + 1)].max(0.0)) } else { fall }
                } else {
                    fall
                };
            }
        });
        // Back into the grids, in the spans (the rest did not move).
        let (u3, v3) = (&liq.u3, &liq.v3);
        liq.u.par_chunks_mut(W + 1).zip(liq.v.par_chunks_mut(W)).enumerate().for_each(|(y, (u, v))| {
            let Some((x0, x1)) = rows[y] else { return };
            let x2 = (x1 + 1).min(W);
            u[x0..=x2].copy_from_slice(&u3[ui(x0, y)..=ui(x2, y)]);
            v[x0..=x1].copy_from_slice(&v3[vi(x0, y)..=vi(x1, y)]);
        });
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

    /// The pressure that stands on the liquid at (x, y) from a cell that is not full liquid: the
    /// gas there, or, under a film of liquid (spray on a wave, froth on a lake), the gas over the
    /// film. (A film at its own pressure, 0, beside air in a pocket at another one would draw the
    /// liquid under it up for ever.)
    fn gas_pa_over(&self, x: i32, y: i32) -> f32 {
        for k in 0..4 {
            let m = self.at(x, y - k);
            if m.kind() != Kind::Liquid {
                return if k > 0 && !matches!(m.kind(), Kind::Empty | Kind::Gas) { 0.0 } else { self.gas_pa(x, y - k) };
            }
        }
        self.gas_pa(x, y)
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
                // A cell inside the body (full liquid or walls all round) that holds less or
                // more than full is brought back to full a little at a time: else the small
                // errors of the carrying add up and the liquid swells into a light froth that no
                // longer weighs. A film over it does not count as inside: a surface where some
                // cells are drawn full and their neighbours not pumps water up into the films
                // (the drawn cell takes from its neighbours, which take from the air over them,
                // and gives up through its film), and the froth grows without end.
                let inside = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)]
                    .iter()
                    .all(|&(nx, ny)| nx >= W || ny >= H || matches!(class[ny * W + nx] & 3, SOLID | FULLISH));
                let fix = if inside { REFILL * (self.liquid_fill(i) as f32 / field::FULL as f32 - 1.0).clamp(-1.0, 1.0) } else { 0.0 };
                row[x] = (u[ui(x + 1, y)] - u[ui(x, y)] + v[vi(x, y + 1)] - v[vi(x, y)]) * k - fix * CELL_M / dt * k;
            }
        });
        // The system (see `mgpcg.rs`): an unknown pressure in each full cell that runs, fixed
        // pressures around it (the gas over the liquid, over its films too; full liquid that
        // sleeps), nothing through a wall. Conductance 2 / (ρ_i + ρ_j) between two cells of
        // liquid, 1 / ρ between liquid and gas. The pressure of the liquid that sleeps is the
        // one it had when it fell asleep, which the gas over the body may have left since (the
        // weather, a pocket): after the solve it is set again from the cells that run beside it
        // (hydrostatic from them), so that it follows them from one step to the next, and a body
        // hung between the gas and the sleeping liquid comes to rest. (A wall there instead
        // would leave a pool of running liquid shut in by sleeping liquid and walls with no
        // pressure to hang from, a singular system that the conjugate gradients cannot take.)
        let solve_clock = self.prof.start();
        let rhs = &rhs_buf;
        let p_old = &liq.p;
        let solver = &mut liq.solver;
        solver.ready(W, H);
        let f = solver.fine();
        f.rows[..H].copy_from_slice(&rows[..H]);
        // The conductance of the edge between cells a and b (of kinds ka, kb), 0 unless one of
        // them is an unknown.
        let cond = |a: usize, b: usize, ka: u8, kb: u8| -> f32 {
            if ka == mgpcg::NONE || kb == mgpcg::NONE || (ka != mgpcg::FLUID && kb != mgpcg::FLUID) {
                0.0
            } else if class[a] & 3 == FULLISH && class[b] & 3 == FULLISH {
                2.0 / (rho(a) + rho(b))
            } else {
                1.0 / rho(if ka == mgpcg::FLUID { a } else { b })
            }
        };
        // The kind of each cell, and the edges along the row (one pass: an edge reads the kinds
        // of its row alone; the cell before the span is nothing, from the last `clear`).
        f.kind.par_chunks_mut(W).zip(f.ax.par_chunks_mut(W + 1)).enumerate().for_each(|(y, (krow, row))| {
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0..=x1 {
                let c = class[y * W + x];
                krow[x] = if c == FULLISH | 4 {
                    mgpcg::FLUID
                } else if c & 3 == SOLID {
                    mgpcg::NONE
                } else {
                    mgpcg::FIXED
                };
            }
            for x in x0.max(1)..=(x1 + 1).min(W - 1) {
                row[x] = cond(y * W + x - 1, y * W + x, krow[x - 1], krow[x]);
            }
            if x0 == 0 {
                row[0] = 0.0;
            }
            if x1 + 1 == W {
                row[W] = 0.0;
            }
        });
        let kind = &f.kind;
        f.ay.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
            if y == 0 || y == H {
                if let Some((x0, x1)) = rows[y.min(H - 1)] {
                    row[x0..=x1].fill(0.0);
                }
                return;
            }
            let span = match (rows[y - 1], rows[y]) {
                (Some((a, b)), Some((c, d))) => Some((a.min(c), b.max(d))),
                (s, None) | (None, s) => s,
            };
            let Some((x0, x1)) = span else { return };
            for x in x0..=x1 {
                let (a, b) = ((y - 1) * W + x, y * W + x);
                row[x] = cond(a, b, kind[a], kind[b]);
            }
        });
        let (ax, ay) = (&f.ax, &f.ay);
        // The diagonal, the right side (the divergence, and the fixed pressures round the cell),
        // and the start of the solution: the last pressure. A full cell walled in on all sides
        // has no pressure to solve for: nothing, to the solver (it has no edge, so no other cell
        // reads its kind in this pass).
        let kinds = Shared::new(&mut f.kind);
        // SAFETY (the reads below): a row only writes the kind of a cell without edges, and a
        // cell's kind is only read from across an edge.
        let kind = |c: usize| unsafe { kinds.get(c) };
        let (diag, b, x) = (&mut f.diag, &mut f.b, &mut f.x);
        // (How many unknowns there are decides whether the card takes the system.)
        let unknowns: usize = diag.par_chunks_mut(W).zip(b.par_chunks_mut(W)).zip(x.par_chunks_mut(W)).enumerate().map(|(y, ((diag, b), x))| {
            let Some((x0, x1)) = rows[y] else { return 0 };
            let mut n = 0;
            for i in x0..=x1 {
                let c = y * W + i;
                if kind(c) != mgpcg::FLUID {
                    diag[i] = 0.0;
                    b[i] = 0.0;
                    x[i] = 0.0;
                    continue;
                }
                let edges = [(ax[y * (W + 1) + i], c.wrapping_sub(1), i > 0), (ax[y * (W + 1) + i + 1], c + 1, i + 1 < W), (ay[c], c.wrapping_sub(W), y > 0), (ay[c + W], c + W, y + 1 < H)];
                let (mut d, mut rhs_i) = (0.0, -rhs[c]);
                for &(a, j, inside) in &edges {
                    if a == 0.0 || !inside {
                        continue;
                    }
                    d += a;
                    if kind(j) == mgpcg::FIXED {
                        // The fixed pressure beside: the gas over the liquid (or over its film),
                        // or the liquid that sleeps.
                        let pj = if class[j] & 3 == FULLISH { p_old[j] } else { self.gas_pa_over((j % W) as i32, (j / W) as i32) };
                        rhs_i += a * pj;
                    }
                }
                if d <= 0.0 {
                    // SAFETY: the cell is this row's, and no row reads a cell without edges.
                    unsafe {
                        *kinds.at(c) = mgpcg::NONE;
                    }
                    diag[i] = 0.0;
                    b[i] = 0.0;
                    x[i] = 0.0;
                    continue;
                }
                diag[i] = d;
                b[i] = rhs_i;
                x[i] = p_old[c];
                n += 1;
            }
            n
        }).sum();
        let gpu = if unknowns >= mgpcg_gpu::threshold() { liq.gpu.get_or_insert_with(|| mgpcg_gpu::PressureGpu::new(W, H)).as_mut() } else { None };
        let iterations = if let Some(gpu) = gpu {
            gpu.solve(solver.fine(), SETTLED, RELATIVE, MAX_ITER)
        } else {
            solver.coarsen();
            solver.solve(SETTLED, RELATIVE, MAX_ITER)
        };
        // The solution back into the pressures, and the solver's grid left clean.
        {
            let f = solver.fine();
            let (x, kind) = (&f.x, &f.kind);
            liq.p.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
                let Some((x0, x1)) = rows[y] else { return };
                for i in x0..=x1 {
                    if kind[y * W + i] == mgpcg::FLUID {
                        row[i] = x[y * W + i];
                    }
                }
            });
        }
        // The full liquid that sleeps beside liquid that runs takes its pressure from it
        // (hydrostatic: a cell of its own weight more under it, less over it), for the next solve.
        {
            let f = solver.fine();
            let (x, kind) = (&f.x, &f.kind);
            liq.p.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
                let Some((x0, x1)) = rows[y] else { return };
                for i in x0..=x1 {
                    let j = y * W + i;
                    if class[j] != FULLISH {
                        continue;
                    }
                    let weight = rho(j) * G * CELL_M;
                    let (mut sum, mut n) = (0.0, 0);
                    for (c, dp, inside) in [(j.wrapping_sub(1), 0.0, i > 0), (j + 1, 0.0, i + 1 < W), (j.wrapping_sub(W), weight, y > 0), (j + W, -weight, y + 1 < H)] {
                        if inside && kind[c] == mgpcg::FLUID {
                            sum += x[c] + dp;
                            n += 1;
                        }
                    }
                    if n > 0 {
                        row[i] = sum / n as f32;
                    }
                }
            });
        }
        solver.clear();
        liq.solve = (solve_clock, iterations);
        liq.rhs = std::mem::take(&mut rhs_buf);
        // The push of the pressure, on every edge beside a full cell that runs.
        let p = &liq.p;
        let pa = |x: i32, y: i32| {
            let j = y as usize * W + x as usize;
            if class[j] & 3 == FULLISH { p[j] } else { self.gas_pa_over(x, y) }
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
            // Liquid that sleeps does not move (and lies beyond the rows worked on: what it gave
            // would never be taken from it).
            if class[from] & 3 < WET || class[from] & 4 == 0 || self.cells[from].m.kind() != Kind::Liquid {
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
        // The edges of a row of cells: those along it (`fu`, W + 1) and those over it (`fv`, W);
        // the edges under the last row are never written. One pass does both.
        let x_range = |x0: usize, x1: usize| x0.max(1)..=(x1 + 1).min(W - 1);
        fu.par_chunks_mut(W + 1).zip(fv.par_chunks_mut(W)).enumerate().for_each(|(y, (urow, vrow))| {
            let Some((x0, x1)) = rows[y] else { return };
            for x in x_range(x0, x1) {
                urow[x] = flow(liq.u[ui(x, y)], y * W + x - 1, y * W + x);
            }
            if y > 0 {
                for x in x0..=x1 {
                    vrow[x] = flow(liq.v[vi(x, y)], (y - 1) * W + x, y * W + x);
                }
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
        // What an edge passes once the cells that take another liquid have refused theirs (`a`
        // and `b` are its cells, the flow positive from `a` to `b`).
        let want_ref = &want;
        let kept = |q: i64, a: usize, b: usize| -> i64 {
            let (from, to) = if q > 0 { (a, b) } else { (b, a) };
            if q != 0 && self.cells[to].m.kind() == Kind::Empty && want_ref[to] != m(from) {
                0
            } else {
                q
            }
        };
        // No cell gives more than it holds: its outflows are cut down to that.
        let mut scale = std::mem::take(&mut liq.scale);
        scale.par_chunks_mut(W).enumerate().for_each(|(y, row)| {
            let Some((x0, x1)) = rows[y] else { return };
            for x in x0..=x1 {
                let i = y * W + x;
                if class[i] & 3 < WET {
                    continue;
                }
                let out = kept(fu[ui(x + 1, y)], i, i + 1).max(0)
                    + (-kept(fu[ui(x, y)], i.wrapping_sub(1), i)).max(0)
                    + kept(fv[vi(x, y + 1)], i, i + W).max(0)
                    + (-kept(fv[vi(x, y)], i.wrapping_sub(W), i)).max(0);
                let have = self.liquid_fill(i) as i64;
                if out > have {
                    row[x] = have as f64 / out as f64;
                }
            }
        });
        // The flows kept, cut down to what their cell holds.
        let scale_ref = &scale;
        let settle = |q: i64, a: usize, b: usize| -> i64 {
            let q = kept(q, a, b);
            let from = if q > 0 { a } else { b };
            (q as f64 * scale_ref[from]) as i64
        };
        fu.par_chunks_mut(W + 1).zip(fv.par_chunks_mut(W)).enumerate().for_each(|(y, (urow, vrow))| {
            let Some((x0, x1)) = rows[y] else { return };
            for x in x_range(x0, x1) {
                urow[x] = settle(urow[x], y * W + x - 1, y * W + x);
            }
            if y > 0 {
                for x in x0..=x1 {
                    vrow[x] = settle(vrow[x], (y - 1) * W + x, y * W + x);
                }
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
        fu.par_chunks_mut(W + 1).zip(fv.par_chunks_mut(W)).zip(want.par_chunks_mut(W)).zip(scale.par_chunks_mut(W)).enumerate().for_each(|(y, (((urow, vrow), w), sc))| {
            if let Some((x0, x1)) = rows[y] {
                urow[x0..=(x1 + 1).min(W)].fill(0);
                vrow[x0..=x1].fill(0);
                w[x0..=x1].fill(Mat::Empty);
                sc[x0..=x1].fill(1.0);
            }
        });
        (liq.fu, liq.fv, liq.want, liq.scale) = (fu, fv, want, scale);
        for (i, q, t, new) in changes {
            if q <= CRUMB as i64 {
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
        // Liquid lying on a cell of the same liquid that is short of full sinks into it (from
        // the top down, so that a film on the surface sinks through to the first gap): the
        // pressure alone never fills a cell under a film, since the film holds no pressure and
        // the surface edge under it comes to rest, and the film would lie there for ever.
        for y in 0..H - 1 {
            let Some((x0, x1)) = liq.span(y) else { continue };
            for x in x0..=x1 {
                let (i, j) = (y * W + x, (y + 1) * W + x);
                let m = self.cells[i].m;
                if m.kind() != Kind::Liquid || self.cells[j].m != m || self.mask[i] != 0 || self.mask[j] != 0 {
                    continue;
                }
                let (have, gap) = (self.liquid_fill(i), full - self.liquid_fill(j).min(full));
                if gap == 0 || !(self.zones.awake[y / sleep::CS * sleep::CW + x / sleep::CS] || self.zones.awake[(y + 1) / sleep::CS * sleep::CW + x / sleep::CS]) {
                    continue;
                }
                let moved = have.min(gap);
                let below = self.liquid_fill(j);
                self.temp[j] = ((self.temp[j] as f64 * below as f64 + self.temp[i] as f64 * moved as f64) / (below + moved) as f64) as f32;
                let q = below + moved;
                self.fill[j] = if q == full { 0 } else { q };
                if have - moved <= CRUMB {
                    self.cells[i] = Cell { m: Mat::Empty, stamp: self.frame };
                    self.clear_states(i);
                    self.vel[i] = 0;
                } else {
                    self.fill[i] = have - moved;
                }
                self.revision += 1;
                self.zones.wake(i);
                self.zones.wake(j);
            }
        }
    }

    /// Units of liquid in all the world, by liquid (for the tests).
    #[cfg(test)]
    pub(super) fn liquid_units(&self, m: Mat) -> u64 {
        let flying: u64 = self.liq.drops.iter().filter(|d| d.m == m).map(|d| d.q as u64).sum();
        (0..W * H).filter(|&i| self.cells[i].m == m).map(|i| self.liquid_fill(i) as u64).sum::<u64>() + flying
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

    /// A block of water 16 x 16 let go 2.5 m over a pool 15 cells deep, in the same tank.
    fn chute() -> World {
        let mut w = world();
        tank(&mut w, 100, 400, 339, 449);
        rect(&mut w, 100, 435, 339, 449, Mat::Water);
        rect(&mut w, 212, 404, 227, 419, Mat::Water);
        w.temp.fill(15.0);
        w
    }

    /// How a pool whose cells start short of full (froth) comes back to full: every second, how
    /// many cells are short, the mean fill and the fastest edge.
    #[test]
    #[ignore]
    fn probe_froth() {
        let mut w = world();
        tank(&mut w, 100, 400, 339, 449);
        rect(&mut w, 100, 435, 339, 449, Mat::Water);
        for y in 435..450 {
            for x in 100..340 {
                w.fill[World::idx(x, y)] = (field::FULL as f32 * 0.7) as u32;
            }
        }
        for t in 0..=600 {
            if t % 60 == 0 {
                let cells: Vec<u32> = (400..450).flat_map(|y| (100..340).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y) == Mat::Water).map(|(x, y)| w.liquid_fill(World::idx(x, y))).collect();
                let short = cells.iter().filter(|&&f| f < field::FULL).count();
                let mean = cells.iter().map(|&f| f as f64).sum::<f64>() / cells.len() as f64 / field::FULL as f64;
                let top = (400..450).find(|&y| (100..340).any(|x| w.raw(x, y) == Mat::Water)).unwrap_or(450);
                let fastest = if w.liq.u.len() == (W + 1) * H { w.liq.u.iter().chain(w.liq.v.iter()).fold(0.0f32, |a, v| a.max(v.abs())) } else { 0.0 };
                let awake = (12..15).flat_map(|zy| (3..=10).map(move |zx| (zx, zy))).filter(|&(zx, zy)| w.zones.awake[zy * sleep::CW + zx]).count();
                println!("t {:4.1} s: {} cells, {short} short, mean fill {mean:.3}, top y {top}, fastest {fastest:.2} m/s, {awake} zones run", t as f32 / 60.0, cells.len());
            }
            w.step();
        }
    }

    #[test]
    #[ignore]
    fn probe_dam() {
        let mut w = if std::env::var("DAM_SCENE").as_deref() == Ok("chute") { chute() } else { dam(Mat::Water) };
        let before = w.liquid_units(Mat::Water);
        let mut dump = String::new();
        for t in 1..=120 {
            w.step();
            if t % 10 == 0 {
                dump += &format!("t {t}\n");
                for y in 400..=449 {
                    for x in 100..=339 {
                        let q = if w.raw(x, y) == Mat::Water { w.liquid_fill(World::idx(x, y)) as f32 / field::FULL as f32 } else { 0.0 };
                        dump += &format!("{q:.2} ");
                    }
                    dump.push('\n');
                }
                for d in w.liquid_drops() {
                    dump += &format!("p {:.2} {:.2} {:.3}\n", d.x - 100.0, d.y - 400.0, d.q as f32 / field::FULL as f32);
                }
            }
            if t % 10 == 0 {
                println!("{:.2} s: front {:.1} m, drops {}, water {:.4}", t as f32 / 60.0, front(&w, Mat::Water, 100, 400, 339, 449) as f32 * CELL_M, w.liquid_drops().len(), w.liquid_units(Mat::Water) as f64 / before as f64);
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
        // (Its surface dries into the air: about a cell of water in these 4 s, a quarter at a time.)
        let after = w.liquid_units(Mat::Water);
        assert!(after <= before && before - after < before / 1_000, "{before} -> {after}");
        let awake = (0..sleep::CW * sleep::CH).filter(|&z| w.zones.awake[z]).count();
        // Only the zones along the surface (8), where the water dries into the air, stay awake,
        // and a couple over them where the vapour rises; the cells' own water keeps 11 of them
        // awake.
        assert!(awake <= 10, "{awake} zones awake");
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

    #[test]
    fn a_block_of_water_falling_in_a_pool_splashes_and_none_is_lost() {
        let mut w = chute();
        let before = w.liquid_units(Mat::Water);
        // The block lands after about 0.6 s; then drops fly up over the pool (its top is 435).
        let mut highest = f32::MAX;
        for t in 0..90 {
            w.step();
            if t > 40 {
                highest = w.liquid_drops().iter().map(|d| d.y).fold(highest, f32::min);
            }
        }
        assert!(highest < 430.0, "the spray rose to {highest}");
        let after = w.liquid_units(Mat::Water);
        assert!(after <= before && before - after < before / 10_000, "{before} -> {after}");
    }

    /// A wall of `m`, `thick` cells thick and 50 high, standing on a stone floor on the bottom of
    /// the world, with water 50 deep against its left side when `wet`. Its strain at the foot and
    /// whether it still stands after 3 s.
    fn dam_wall(m: Mat, thick: i32, wet: bool) -> (u8, bool, Vec<statics::LiquidPush>) {
        let mut w = world();
        w.structures_fall = true;
        let b = H as i32 - 1;
        rect(&mut w, 100, b - 4, 400, b, Mat::Stone);
        rect(&mut w, 99, b - 60, 99, b - 5, Mat::Stone);
        rect(&mut w, 200, b - 54, 199 + thick, b - 5, m);
        if wet {
            rect(&mut w, 100, b - 54, 199, b - 5, Mat::Water);
        }
        w.temp.fill(15.0);
        let mut strain = 0;
        for t in 0..180 {
            w.step();
            if t == 60 {
                strain = w.strain[World::idx(200, b - 5)];
            }
        }
        let stands = (b - 54..=b - 5).all(|y| w.raw(200, y) == m);
        (strain, stands, w.pushes.clone())
    }

    #[test]
    #[ignore]
    fn probe_dam_wall() {
        for (m, thick) in [(Mat::Stone, 4), (Mat::Stone, 2), (Mat::Brick, 2), (Mat::Glass, 1), (Mat::Wood, 1)] {
            for wet in [false, true] {
                let (strain, stands, pushes) = dam_wall(m, thick, wet);
                println!("{m:?} x{thick} wet {wet}: strain {strain} stands {stands} pushes {pushes:?}");
            }
        }
    }

    #[test]
    fn water_loads_the_wall_of_a_dam_and_pushes_a_thin_one_over() {
        // A thick wall of rock, one with the floor it stands on: the water loads its foot, the
        // stress view shows it, and it holds.
        let (dry, stands, _) = dam_wall(Mat::Stone, 4, false);
        assert!(dry == 0 && stands);
        let (wet, stands, pushes) = dam_wall(Mat::Stone, 4, true);
        assert!(wet > 20 && stands, "strain {wet}");
        // (The water weighs on the floor: 50 cells deep over 100, about 5 000 cells of water; a
        // little of it has run off by the time of the last pass of the structures.)
        assert!(pushes.iter().any(|p| p.fy > 900.0), "{pushes:?}");
        // A thin one gives way, rock or a pane of glass; dry, it stands.
        for (m, thick) in [(Mat::Stone, 2), (Mat::Glass, 1)] {
            assert!(dam_wall(m, thick, false).1, "{m:?} dry");
            assert!(!dam_wall(m, thick, true).1, "{m:?} wet");
        }
    }

    /// How a splash dies down: every half second, the highest water (cell or drop), the drops in
    /// flight, the cells under half full, and the fastest edge.
    #[test]
    #[ignore]
    fn probe_splash() {
        let mut w = chute();
        for t in 0..=600 {
            if t % 30 == 0 {
                let top_cell = (400..450).find(|&y| (100..340).any(|x| w.raw(x, y) == Mat::Water)).unwrap_or(450);
                let top_drop = w.liquid_drops().iter().map(|d| d.y).fold(450.0, f32::min);
                let wet = (400..450).flat_map(|y| (100..340).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y) == Mat::Water && w.liquid_fill(World::idx(x, y)) < field::FULL / 2).count();
                let fastest = if w.liq.u.len() == (W + 1) * H { w.liq.u.iter().chain(w.liq.v.iter()).fold(0.0f32, |a, v| a.max(v.abs())) } else { 0.0 };
                let (mut pool_fast, mut pool_sq, mut n) = (0.0f32, 0.0f64, 0usize);
                if w.liq.u.len() == (W + 1) * H {
                    for y in 435..450usize {
                        for x in 100..340usize {
                            let s = w.liq.u[ui(x, y)].abs().max(w.liq.v[vi(x, y)].abs());
                            pool_fast = pool_fast.max(s);
                            pool_sq += (s * s) as f64;
                            n += 1;
                        }
                    }
                }
                println!("t {:5.2} s: top cell y {top_cell}, top drop y {top_drop:.1}, {} drops, {wet} wet cells, fastest {fastest:.2} m/s, pool fastest {pool_fast:.2} rms {:.3}", t as f32 / 60.0, w.liquid_drops().len(), (pool_sq / n.max(1) as f64).sqrt());
                if t % 120 == 0 && w.liq.u.len() == (W + 1) * H {
                    // Where the fastest edge is, and whether its zone runs.
                    let (mut best, mut at) = (0.0f32, (0usize, 0usize, 'u'));
                    for y in 0..H {
                        for x in 0..=W {
                            if w.liq.u[ui(x, y)].abs() > best { best = w.liq.u[ui(x, y)].abs(); at = (x, y, 'u'); }
                        }
                    }
                    for y in 0..=H {
                        for x in 0..W {
                            if w.liq.v[vi(x, y)].abs() > best { best = w.liq.v[vi(x, y)].abs(); at = (x, y, 'v'); }
                        }
                    }
                    let (x, y, _) = at;
                    let awake = w.zones.awake[(y.min(H - 1)) / sleep::CS * sleep::CW + x.min(W - 1) / sleep::CS];
                    let awake_wet = (400..450).flat_map(|y| (100..340).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y) == Mat::Water && w.liquid_fill(World::idx(x, y)) < field::FULL / 2 && w.zones.awake[y as usize / sleep::CS * sleep::CW + x as usize / sleep::CS]).count();
                    println!("  fastest at {at:?} (zone awake {awake}); wet cells in zones that run: {awake_wet}");
                    let short = |awake: bool| (400..450).flat_map(|y| (100..340).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y) == Mat::Water && (field::FULL / 2..field::FULL).contains(&w.liquid_fill(World::idx(x, y))) && w.zones.awake[y as usize / sleep::CS * sleep::CW + x as usize / sleep::CS] == awake).count();
                    println!("  cells between half and full: {} in zones that run, {} in zones that sleep", short(true), short(false));
                    for y in 425..450 {
                        let fills: Vec<u32> = (100..340).filter(|&x| w.raw(x, y) == Mat::Water).map(|x| w.liquid_fill(World::idx(x, y))).collect();
                        if fills.is_empty() { continue; }
                        let short = fills.iter().filter(|&&f| f < field::FULL).count();
                        let half = fills.iter().filter(|&&f| f < field::FULL / 2).count();
                        let mean = fills.iter().map(|&f| f as f64).sum::<f64>() / fills.len() as f64 / field::FULL as f64;
                        let fast = (100..340).map(|x| w.liq.u[ui(x as usize, y as usize)].abs().max(w.liq.v[vi(x as usize, y as usize)].abs())).fold(0.0f32, f32::max);
                        println!("    row {y}: {} cells, {short} short, {half} under half, mean {mean:.3}, fastest {fast:.2}", fills.len());
                    }
                    for zy in 12..=14 {
                        println!("  zones y {zy}: {}", (3..=10).map(|zx| if w.zones.awake[zy * sleep::CW + zx] { '*' } else { '.' }).collect::<String>());
                    }
                    // The cells around the highest water cell, with their class, fill and edges.
                    let tx = (100..340).find(|&x| w.raw(x, top_cell) == Mat::Water).unwrap_or(0) as usize;
                    for y in (top_cell as usize).saturating_sub(1)..=(top_cell as usize + 2) {
                        println!("  y {y} rows {:?}", w.liq.rows[y]);
                        for x in tx.saturating_sub(2)..=tx + 3 {
                            let i = World::idx(x as i32, y as i32);
                            println!("    ({x},{y}) {:?} fill {} class {} u {:.2}|{:.2} v {:.2}/{:.2}", w.cells[i].m, w.liquid_fill(i), w.liq.class[i], w.liq.u[ui(x, y)], w.liq.u[ui(x + 1, y)], w.liq.v[vi(x, y)], w.liq.v[vi(x, y + 1)]);
                        }
                    }
                    for y in 400..450 {
                        let mut row = String::new();
                        for x in 100..340 {
                            let i = World::idx(x, y);
                            row.push(match w.raw(x, y) {
                                Mat::Empty => if w.liquid_drops().iter().any(|d| d.x as i32 == x && d.y as i32 == y) { 'o' } else { ' ' },
                                Mat::Water => match w.liquid_fill(i) * 64 / field::FULL {
                                    0..=15 => '.',
                                    16..=40 => ':',
                                    41..=63 => '~',
                                    _ => 'W',
                                },
                                Mat::Stone => '#',
                                _ => '?',
                            });
                        }
                        println!("{row}");
                    }
                }
            }
            w.step();
        }
    }
}
