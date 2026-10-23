//! Statics of the solids: what holds, what tips over, and what breaks when it lands.
//!
//! A solid body is a run of touching cells of the same matter (a whole tree counts as one). It has
//! a mass m = Σ ρ·V, a centre of mass, and a moment of inertia I = Σ ρ·V·r² about that centre:
//! in a flat world a body can only turn about the axis that comes out of the screen, so of its
//! inertia matrix only that one term matters. Earth is not a solid: it is a granular matter and
//! moves grain by grain.
//!
//! - **Anchored** bodies touch something fixed: the base of the world, a tunnel's vault, the ground
//!   a tree's roots grow in. They hold as a whole, but each cross-section has to carry the
//!   moment of everything beyond it. The section through the cells that are `d` cells from the
//!   anchor is cut; the part beyond would turn about the edge fibre of the section, under the
//!   moment of its weight, m·g times the lever arm of its centre of mass. The bonds of the other
//!   fibres resist with their strength σ times their distance to that edge (a third of it: the
//!   stress in a brittle section grows linearly from the edge). Where the weight wins, the section
//!   cracks and the part beyond tips over its edge. A long overhang, a thin stroke of ice standing
//!   out sideways, a top-heavy pillar of glass: they break where they are weakest.
//! - **Free** bodies only rest on what is under them. If the vertical through their centre of mass
//!   falls outside the cells they rest on, they tip over the last one.
//!
//! A tipping body turns about its edge, α = m·g·lever / (I + m·r²), until the edge no longer has to
//! pull to keep it turning (the edge can push, not pull); it then flies on, spinning. When it hits
//! something it breaks according to the energy it brings: a small share of it opens cracks, and
//! opening a crack across a section costs the fracture energy of the matter (J/m²). The faster
//! a part hits, the smaller the pieces: ice and glass shatter from a few metres, rock only after a
//! long fall, wood and metal not at all. What is crushed finer than a cell becomes grains: hail,
//! shards, sand.

use super::blast::strength;
use super::*;
use std::collections::{HashMap, HashSet, VecDeque};

/// Side of a cell, m.
const CELL_M: f32 = 0.05;
/// Gravity, m/s².
const G: f32 = 9.81;
/// The pull of gravity on a body in flight, cells per step² (the same as for debris and clods). It
/// sets the time of a step for the bodies: sqrt(CELL_M · FALL / G), about 25 ms.
const FALL: f32 = 0.12;
/// Share of the energy of an impact that goes into opening cracks; the rest bounces back, shakes
/// the ground and warms it.
const CRACK_SHARE: f32 = 0.0005;
/// Bodies larger than this many cells do not tip as a whole (they are mountains).
const MAX_SLAB: usize = 40_000;
/// At most this many bodies start to tip in one pass.
const MAX_TIPS: usize = 4;

/// Speed in m/s of one cell per step.
fn ms_per_cell_step() -> f32 {
    CELL_M / (CELL_M * FALL / G).sqrt()
}

/// Whether two touching solid cells belong to one body.
pub fn same_body(a: Mat, b: Mat) -> bool {
    a == b || (a.is_vegetation() && b.is_vegetation())
}

/// Fracture energy, J/m²: what it costs to open a crack through the matter. Brittle matters need
/// little (ice, glass, rock), tough ones a great deal (wood, metal: they bend instead).
pub fn toughness(m: Mat) -> f32 {
    match m {
        Mat::Ice => 1.0,
        Mat::Glass => 8.0,
        Mat::Clay => 5.0,
        Mat::Brick | Mat::Terracotta => 30.0,
        Mat::Stone | Mat::Vault => 50.0,
        Mat::Diamond => 20.0,
        Mat::Ember => 100.0,
        Mat::Dynamite => 300.0,
        Mat::Leaf | Mat::Sapling => 500.0,
        Mat::Wood | Mat::Branch => 8_000.0,
        Mat::Trunk => 15_000.0,
        Mat::Iron => 50_000.0,
        Mat::Steel => 100_000.0,
        _ => 1.0e6,
    }
}

/// What a solid crushed finer than a cell becomes.
pub fn rubble(m: Mat) -> Option<Mat> {
    match m {
        Mat::Ice => Some(Mat::Hail),
        Mat::Glass => Some(Mat::Shard),
        Mat::Stone | Mat::Brick | Mat::Terracotta | Mat::Vault => Some(Mat::Sand),
        Mat::Clay => Some(Mat::Dirt),
        Mat::Diamond => Some(Mat::Diamond),
        _ => None,
    }
}

/// Typical size of the pieces, in cells, when matter `m` hits at `v` m/s.
fn piece_size(m: Mat, v: f32) -> f32 {
    if v < 0.3 || rubble(m).is_none() {
        return f32::INFINITY;
    }
    // Breaking a body of area A into squares of side s opens about 2A/s of crack (per metre of
    // depth); its kinetic energy is ½ρAv². The share that cracks it pays for the cracks.
    4.0 * toughness(m) / (CRACK_SHARE * density(m) * 1000.0 * v * v) / CELL_M
}

/// One cell of a body in flight, as it was when the body broke loose.
#[derive(Clone, Copy)]
struct Part {
    x: i32,
    y: i32,
    cell: Cell,
    temp: f32,
    states: (States, u16),
}

/// A solid body tipping over or tumbling through the air.
#[derive(Clone)]
pub struct Slab {
    parts: Vec<Part>,
    /// Which part was at each cell when it broke loose.
    home: HashMap<(i32, i32), u32>,
    /// Centre of mass when it broke loose and now, cells.
    c0: (f32, f32),
    pub c: (f32, f32),
    /// Cells per step, while it flies.
    pub v: (f32, f32),
    /// Radians (clockwise on the screen) and radians per step.
    pub angle: f32,
    pub spin: f32,
    /// Σ density of the cells.
    mass: f32,
    /// Moment of inertia about the centre of mass, Σ density · r² (cells²).
    inertia: f32,
    /// Farthest a cell is from the centre of mass.
    reach: f32,
    /// The edge it turns about while it still rests on it.
    hinge: Option<(f32, f32)>,
    /// Cells it covers now, and the part in each.
    drawn: Vec<(usize, u32)>,
    /// Parts crushed against the edge it turns on (where they would be now, cells).
    crushed: Vec<(u32, (f32, f32))>,
}

impl Slab {
    pub fn cells(&self) -> impl Iterator<Item = &usize> {
        self.drawn.iter().map(|d| &d.0)
    }

    /// Whether it is still turning on its edge.
    pub fn pivoting(&self) -> bool {
        self.hinge.is_some()
    }

    /// Velocity of the point `p` of the body, cells per step.
    fn velocity_at(&self, p: (f32, f32)) -> (f32, f32) {
        match self.hinge {
            Some(h) => (-self.spin * (p.1 - h.1), self.spin * (p.0 - h.0)),
            None => (self.v.0 - self.spin * (p.1 - self.c.1), self.v.1 + self.spin * (p.0 - self.c.0)),
        }
    }
}

fn free_for_slab(m: Mat) -> bool {
    matches!(m.kind(), Kind::Empty | Kind::Gas | Kind::Flame)
}

/// A crack across a section that could give way: the cells of the section, its edge fibre, the
/// distance from the anchor and how far beyond its strength it is loaded.
struct Crack {
    load: f32,
    level: u32,
    fibres: Vec<usize>,
    pivot: usize,
}

impl World {
    /// The states of cell `i` and the log it belongs to.
    fn take_states(&self, i: usize) -> (States, u16) {
        (self.states(i), self.log[i])
    }

    fn put_states(&mut self, i: usize, (st, log): (States, u16)) {
        self.log[i] = log;
        self.moisture[i] = st.moisture;
        self.fuel[i] = st.fuel;
        self.growth[i] = st.growth;
        self.jet[i] = st.jet;
    }

    /// Checks the sections of the anchored bodies (see the top of the file); `order` lists their
    /// cells from the anchors outwards. The parts beyond a section that gives way break loose and
    /// start to tip.
    pub(super) fn stress_pass(&mut self, order: &[usize], seen: &mut [bool]) {
        // Nothing changed in the anchored bodies since the last check: their loads are the same.
        let key = order.iter().fold(order.len() as u64, |h, &i| (h ^ ((i as u64) << 8 | self.cells[i].m as u64)).wrapping_mul(0x100_0000_01b3));
        if key == self.strain_key {
            return;
        }
        self.strain_key = key;
        self.strain.fill(0);
        let n = order.len();
        if n == 0 {
            return;
        }
        let level = std::mem::take(&mut self.level);
        let mut pos = vec![u32::MAX; W * H];
        for (k, &i) in order.iter().enumerate() {
            pos[i] = k as u32;
        }
        let mut up: Vec<u32> = (0..n as u32).collect();
        let mut mass = vec![0.0f64; n];
        let mut moment = vec![0.0f64; n];
        fn find(up: &mut [u32], mut k: u32) -> u32 {
            while up[k as usize] != k {
                let next = up[up[k as usize] as usize];
                up[k as usize] = next;
                k = next;
            }
            k
        }
        let mut cracks: Vec<Crack> = vec![];
        let mut fibres: Vec<(u32, u32)> = vec![];
        // From the farthest cells inwards: when the cells `d` from the anchor join, each body of
        // cells at `d` or more hangs on its cells at `d`.
        let mut hi = n;
        while hi > 0 {
            let d = level[order[hi - 1]];
            let mut lo = hi;
            while lo > 0 && level[order[lo - 1]] == d {
                lo -= 1;
            }
            for k in lo..hi {
                let i = order[k];
                let rho = density(self.cells[i].m) as f64;
                mass[k] = rho;
                moment[k] = rho * ((i % W) as f64 + 0.5);
            }
            for k in lo..hi {
                let i = order[k];
                let (x, y) = ((i % W) as i32, (i / W) as i32);
                for (nx, ny) in Self::neighbors(x, y) {
                    if !Self::inb(nx, ny) {
                        continue;
                    }
                    let j = Self::idx(nx, ny);
                    if pos[j] == u32::MAX || level[j] < d || !self.same_body(i, j) {
                        continue;
                    }
                    let (a, b) = (find(&mut up, k as u32), find(&mut up, pos[j]));
                    if a != b {
                        up[a as usize] = b;
                        mass[b as usize] += mass[a as usize];
                        moment[b as usize] += moment[a as usize];
                    }
                }
            }
            if d == 0 {
                break;
            }
            fibres.clear();
            for k in lo..hi {
                fibres.push((find(&mut up, k as u32), k as u32));
            }
            fibres.sort_unstable();
            let mut a = 0;
            while a < fibres.len() {
                let root = fibres[a].0;
                let mut b = a;
                while b < fibres.len() && fibres[b].0 == root {
                    b += 1;
                }
                let cells: Vec<usize> = fibres[a..b].iter().map(|&(_, k)| order[k as usize]).collect();
                let (load, pivot) = self.section_load(&cells, mass[root as usize], moment[root as usize]);
                let shown = (load.clamp(0.0, 2.0) * 127.5) as u8;
                for &i in &cells {
                    self.strain[i] = shown;
                }
                if load > 1.0 {
                    cracks.push(Crack { load, level: d, fibres: cells, pivot });
                }
                a = b;
            }
            hi = lo;
        }
        // The worst first; a crack whose part beyond has already broken loose waits for the
        // next pass (its load has changed).
        cracks.sort_by(|a, b| b.load.total_cmp(&a.load));
        // Which crack's part beyond each cell belongs to (0: none yet).
        let mut taken = vec![0u32; if cracks.is_empty() { 0 } else { W * H }];
        let mut tips = 0;
        for (id, crack) in cracks.into_iter().enumerate() {
            if tips >= MAX_TIPS {
                break;
            }
            let id = id as u32 + 1;
            let mut region = vec![];
            let mut stack: Vec<usize> = crack.fibres.clone();
            let mut ok = true;
            for &i in &stack {
                if taken[i] != 0 {
                    ok = false;
                }
                taken[i] = id;
            }
            while let Some(i) = stack.pop() {
                region.push(i);
                if region.len() > MAX_SLAB || self.mask[i] != 0 {
                    ok = false;
                    break;
                }
                let (x, y) = ((i % W) as i32, (i / W) as i32);
                for (nx, ny) in Self::neighbors(x, y) {
                    if !Self::inb(nx, ny) {
                        continue;
                    }
                    let j = Self::idx(nx, ny);
                    if level[j] == u32::MAX || level[j] < crack.level || !self.same_body(i, j) {
                        continue;
                    }
                    if taken[j] != 0 {
                        ok &= taken[j] == id;
                        continue;
                    }
                    taken[j] = id;
                    stack.push(j);
                }
            }
            if !ok {
                continue;
            }
            // It turns about the edge between the pivot fibre and the cell under it that held it.
            let (px, py) = ((crack.pivot % W) as i32, (crack.pivot / W) as i32);
            let held = Self::neighbors(px, py).into_iter().find(|&(nx, ny)| Self::inb(nx, ny) && level[Self::idx(nx, ny)] == crack.level - 1).unwrap_or((px, py + 1));
            let hinge = ((px + held.0) as f32 / 2.0 + 0.5, (py + held.1) as f32 / 2.0 + 0.5);
            for &i in &region {
                seen[i] = true;
            }
            self.loosen(region, Some(hinge), (0.0, 0.0), 0.0);
            tips += 1;
        }
        self.level = level;
    }

    /// How loaded the section through `cells` is, as a share of what it can bear (over 1: it
    /// cracks), given the mass and Σ mass·x of all it carries. Also the fibre it would turn about.
    fn section_load(&self, cells: &[usize], mass: f64, moment: f64) -> (f32, usize) {
        let m = self.cells[cells[0]].m;
        if m.is_vegetation() || mass <= 0.0 {
            return (0.0, cells[0]);
        }
        let sigma = strength(m) as f64 * 1.0e6;
        let xy = |i: usize| ((i % W) as f64 + 0.5, (i / W) as f64 + 0.5);
        let (mut worst, mut pivot) = (0.0f64, cells[0]);
        for side in [-1.0f64, 1.0] {
            // The edge fibre on that side, lowest first: the part beyond would turn about it.
            let p = *cells.iter().max_by(|&&a, &&b| {
                let (ax, ay) = xy(a);
                let (bx, by) = xy(b);
                (side * ax + ay).total_cmp(&(side * bx + by))
            }).unwrap();
            let (px, py) = xy(p);
            // Moment of the weight about the edge (N·m per metre of depth, over h³)...
            let turning = G as f64 * CELL_M as f64 * 1000.0 * side * (moment - mass * px);
            if turning <= 0.0 {
                continue;
            }
            // ... and what the bonds of the section hold against it.
            let lever: f64 = cells.iter().map(|&i| {
                let (x, y) = xy(i);
                ((x - px).powi(2) + (y - py).powi(2)).sqrt() + 0.5
            }).sum();
            let holding = sigma / 3.0 * lever;
            let load = if holding > 0.0 { turning / holding } else { 10.0 };
            if load > worst {
                worst = load;
                pivot = p;
            }
        }
        (worst as f32, pivot)
    }

    /// Free bodies: those with room under them fall (see `drop_structures`); those resting with
    /// their centre of mass beyond the cells they rest on tip over the last of them.
    pub(super) fn tip_pass(&mut self, free: Vec<Piece>) {
        let mut tips = 0;
        for mut p in free {
            p.cells.sort_unstable_by(|a, b| b.cmp(a));
            if self.piece_can_move(&p, 1) {
                self.falling.push(p);
                continue;
            }
            let in_piece = |i: usize| p.cells.binary_search_by(|q| i.cmp(q)).is_ok();
            let (mut lo, mut hi) = ((i32::MAX, 0), (i32::MIN, 0));
            let (mut loose, mut firm) = (false, false);
            for &i in &p.cells {
                let t = i + W;
                if t >= W * H || in_piece(t) {
                    continue;
                }
                let under = self.cells[t].m;
                if self.mask[t] == 0 {
                    // Firm ground holds it up; a liquid buoys it (it floats, see `drop_structures`),
                    // but only where it dips in: a long arm reaching out beyond tips it over all the same.
                    match under.kind() {
                        Kind::Static | Kind::Powder => firm = true,
                        Kind::Liquid => {}
                        _ => continue,
                    }
                    // Leaves and twigs give way under it (see `crush_foliage`).
                    if matches!(under, Mat::Leaf | Mat::Branch | Mat::Sapling) {
                        loose = true;
                    }
                } else {
                    firm = true;
                }
                let (x, y) = ((i % W) as i32, (i / W) as i32);
                if x < lo.0 || (x == lo.0 && y > lo.1) {
                    lo = (x, y);
                }
                if x > hi.0 || (x == hi.0 && y > hi.1) {
                    hi = (x, y);
                }
            }
            if loose || lo.0 == i32::MAX {
                self.falling.push(p);
                continue;
            }
            if tips >= MAX_TIPS || p.cells.len() > MAX_SLAB {
                if !firm {
                    self.falling.push(p);
                }
                continue;
            }
            let (mass, mx, my) = p.cells.iter().fold((0.0f32, 0.0f32, 0.0f32), |(m, mx, my), &i| {
                let rho = density(self.cells[i].m);
                (m + rho, mx + rho * ((i % W) as f32 + 0.5), my + rho * ((i / W) as f32 + 0.5))
            });
            let (cx, cy) = (mx / mass.max(1e-6), my / mass.max(1e-6));
            let hinge = if cx < lo.0 as f32 {
                (lo.0 as f32, lo.1 as f32 + 1.0)
            } else if cx > hi.0 as f32 + 1.0 {
                (hi.0 as f32 + 1.0, hi.1 as f32 + 1.0)
            } else {
                if !firm {
                    self.falling.push(p);
                }
                continue;
            };
            // Hooked by a part above its centre of mass, it hangs there instead.
            if cy > hinge.1 {
                continue;
            }
            self.loosen(p.cells, Some(hinge), (0.0, 0.0), 0.0);
            tips += 1;
        }
    }

    /// Turns the cells of a body into a slab turning about `hinge`, or flying at `v` (cells per
    /// step) and turning at `spin`.
    fn loosen(&mut self, cells: Vec<usize>, hinge: Option<(f32, f32)>, v: (f32, f32), spin: f32) {
        let mut parts = Vec::with_capacity(cells.len());
        let mut home = HashMap::with_capacity(cells.len());
        let (mut mass, mut mx, mut my) = (0.0f32, 0.0f32, 0.0f32);
        let mut drawn = Vec::with_capacity(cells.len());
        for (k, &i) in cells.iter().enumerate() {
            let (x, y) = ((i % W) as i32, (i / W) as i32);
            let rho = density(self.cells[i].m);
            mass += rho;
            mx += rho * (x as f32 + 0.5);
            my += rho * (y as f32 + 0.5);
            parts.push(Part { x, y, cell: self.cells[i], temp: self.temp[i], states: self.take_states(i) });
            home.insert((x, y), k as u32);
            drawn.push((i, k as u32));
        }
        let c0 = (mx / mass, my / mass);
        let (mut inertia, mut reach) = (0.0f32, 0.0f32);
        for p in &parts {
            let r2 = (p.x as f32 + 0.5 - c0.0).powi(2) + (p.y as f32 + 0.5 - c0.1).powi(2);
            // A square cell about its own centre adds a sixth of its side squared.
            inertia += density(p.cell.m) * (r2 + 1.0 / 6.0);
            reach = reach.max(r2.sqrt());
        }
        self.slabs.push(Slab { parts, home, c0, c: c0, v, angle: 0.0, spin, mass, inertia, reach: reach + 1.0, hinge, drawn, crushed: vec![] });
    }

    /// Moves the bodies in flight; those that hit something come to rest there, broken or not.
    pub(super) fn step_slabs(&mut self) {
        if self.slabs.is_empty() {
            return;
        }
        let mut slabs = std::mem::take(&mut self.slabs);
        slabs.retain_mut(|s| self.step_slab(s));
        slabs.append(&mut self.slabs);
        self.slabs = slabs;
    }

    /// One step of a body: false once it has landed (or left the world).
    fn step_slab(&mut self, s: &mut Slab) -> bool {
        if let Some(h) = s.hinge {
            let r = (s.c.0 - h.0, s.c.1 - h.1);
            let r2 = r.0 * r.0 + r.1 * r.1;
            // Torque of its weight about the edge (clockwise when the centre is to the right).
            s.spin += FALL * s.mass * r.0 / (s.inertia + s.mass * r2);
            // The edge only pushes: once gravity alone no longer bends the path of the centre of
            // mass round it (ω²r > g·cos), the body leaves it and flies.
            let rl = r2.sqrt().max(1e-3);
            if s.spin * s.spin * rl > -FALL * r.1 / rl {
                s.v = s.velocity_at(s.c);
                s.hinge = None;
            }
        } else {
            s.v.1 += FALL;
        }
        let lever = match s.hinge {
            Some(h) => ((s.c.0 - h.0).powi(2) + (s.c.1 - h.1).powi(2)).sqrt() + s.reach,
            None => s.reach,
        };
        let fastest = (s.v.0 * s.v.0 + s.v.1 * s.v.1).sqrt() * s.hinge.is_none() as u8 as f32 + s.spin.abs() * lever;
        let n = fastest.ceil().clamp(1.0, 8.0) as i32;
        for _ in 0..n {
            let (c, angle) = match s.hinge {
                Some(h) => {
                    let (sn, cs) = (s.spin / n as f32).sin_cos();
                    let r = (s.c.0 - h.0, s.c.1 - h.1);
                    ((h.0 + r.0 * cs - r.1 * sn, h.1 + r.0 * sn + r.1 * cs), s.angle + s.spin / n as f32)
                }
                None => ((s.c.0 + s.v.0 / n as f32, s.c.1 + s.v.1 / n as f32), s.angle + s.spin / n as f32),
            };
            match self.place(s, c, angle) {
                Ok(()) => {
                    s.c = c;
                    s.angle = angle;
                }
                Err(soft) => {
                    self.land(s, soft);
                    return false;
                }
            }
        }
        !s.drawn.is_empty()
    }

    /// Draws the body at a new pose, unless that runs into something: then Err, with true if all
    /// it ran into is liquid.
    fn place(&mut self, s: &mut Slab, c: (f32, f32), angle: f32) -> Result<(), bool> {
        let (sn, cs) = angle.sin_cos();
        let mut around: HashSet<usize> = HashSet::with_capacity(s.parts.len() * 2);
        for p in &s.parts {
            let d = (p.x as f32 + 0.5 - s.c0.0, p.y as f32 + 0.5 - s.c0.1);
            let (x, y) = ((c.0 + d.0 * cs - d.1 * sn).floor() as i32, (c.1 + d.0 * sn + d.1 * cs).floor() as i32);
            for (tx, ty) in [(x, y), (x + 1, y), (x, y + 1)] {
                if Self::inb(tx, ty) {
                    around.insert(Self::idx(tx, ty));
                }
            }
        }
        let own: HashSet<usize> = s.drawn.iter().map(|d| d.0).collect();
        let mut drawn = Vec::with_capacity(s.parts.len());
        let mut crushed = vec![];
        let mut soft = true;
        let mut blocked = false;
        for &t in &around {
            // Which part of the body covers the centre of this cell, if any.
            let q = ((t % W) as f32 + 0.5 - c.0, (t / W) as f32 + 0.5 - c.1);
            let home = ((s.c0.0 + q.0 * cs + q.1 * sn).floor() as i32, (s.c0.1 - q.0 * sn + q.1 * cs).floor() as i32);
            let Some(&k) = s.home.get(&home) else { continue };
            // Air gives way; while it turns slowly on its edge, so do water and loose grains
            // (they are pushed aside). In flight they stop it.
            let m = self.cells[t].m;
            let yields = free_for_slab(m) || (s.hinge.is_some() && matches!(m.kind(), Kind::Liquid | Kind::Powder));
            if own.contains(&t) || (self.mask[t] == 0 && yields) {
                drawn.push((t, k));
                continue;
            }
            // Against its own foot while it still turns on it: that corner is crushed, not stopped.
            if let Some(h) = s.hinge {
                let p = ((t % W) as f32 + 0.5, (t / W) as f32 + 0.5);
                if (p.0 - h.0).powi(2) + (p.1 - h.1).powi(2) < 6.25 {
                    crushed.push((k, p));
                    continue;
                }
            }
            blocked = true;
            if self.mask[t] != 0 || self.cells[t].m.kind() != Kind::Liquid {
                soft = false;
            }
        }
        if blocked {
            return Err(soft);
        }
        // What the body leaves is filled with the air (or smoke, or flames) it pushes aside.
        let now: HashSet<usize> = drawn.iter().map(|d| d.0).collect();
        let mut pushed: Vec<(Cell, f32, (States, u16))> = drawn.iter().filter(|d| !own.contains(&d.0) && self.cells[d.0].m != Mat::Empty).map(|d| (self.cells[d.0], self.temp[d.0], self.take_states(d.0))).collect();
        pushed.sort_by_key(|p| matches!(p.0.m.kind(), Kind::Liquid | Kind::Powder | Kind::Static));
        let frame = self.frame;
        for &(i, _) in &s.drawn {
            if now.contains(&i) {
                continue;
            }
            // Pushed aside into the room the body leaves (grains and water first: they matter).
            match pushed.pop() {
                Some((cell, t, st)) => {
                    self.cells[i] = Cell { stamp: frame, ..cell };
                    self.temp[i] = t;
                    self.put_states(i, st);
                }
                None => {
                    self.cells[i] = Cell { stamp: frame, ..EMPTY };
                    self.clear_states(i);
                }
            }
            self.latent[i] = 0.0;
            self.vel[i] = 0;
            self.zones.wake(i);
        }
        // What found no room is thrown up beside it.
        for (cell, _, _) in pushed {
            if cell.m.kind() != Kind::Gas && cell.m.kind() != Kind::Flame && self.ejecta.len() < 14_000 {
                let h = s.hinge.unwrap_or(c);
                let vx = (self.rand() % 100) as f32 / 100.0 - 0.5;
                self.ejecta.push(Ejecta { x: h.0, y: h.1 - 2.0, vx, vy: -0.6, m: cell.m });
            }
        }
        for &(t, k) in &drawn {
            let p = s.parts[k as usize];
            self.cells[t] = Cell { stamp: frame, ..p.cell };
            self.temp[t] = p.temp;
            self.put_states(t, p.states);
            self.latent[t] = 0.0;
            self.vel[t] = 0;
            self.zones.wake(t);
        }
        s.drawn = drawn;
        s.crushed = crushed;
        self.revision += 1;
        Ok(())
    }

    /// The body hit something. In a liquid it goes on as a plain falling piece (it floats or
    /// sinks); on anything firmer it breaks, or not, by how hard each part hits.
    fn land(&mut self, s: &Slab, soft: bool) {
        // What was crushed at its foot crumbles there.
        let drawn: HashSet<u32> = s.drawn.iter().map(|d| d.1).collect();
        for &(k, p) in &s.crushed {
            let m = s.parts[k as usize].cell.m;
            if !drawn.contains(&k) && self.ejecta.len() < 14_000 {
                self.ejecta.push(Ejecta { x: p.0, y: p.1 - 1.5, vx: 0.0, vy: -0.2, m: rubble(m).unwrap_or(m) });
            }
        }
        let cells: Vec<usize> = s.drawn.iter().map(|d| d.0).collect();
        if cells.is_empty() {
            return;
        }
        if soft {
            let density = cells.iter().map(|&i| density(self.cells[i].m)).sum::<f32>() / cells.len() as f32;
            let vy = (s.velocity_at(s.c).1 * 2.0).clamp(0.0, 3.0);
            self.falling.push(Piece { cells, vy, fallen: 0.0, density });
            return;
        }
        let ms = ms_per_cell_step();
        let vel: Vec<(f32, f32)> = cells.iter().map(|&i| {
            let v = s.velocity_at(((i % W) as f32 + 0.5, (i / W) as f32 + 0.5));
            (v.0 * ms, v.1 * ms)
        }).collect();
        let hardest = vel.iter().map(|v| (v.0 * v.0 + v.1 * v.1).sqrt()).fold(0.0f32, f32::max);
        if self.thuds.len() < 8 && hardest > 2.0 {
            let c = (s.c.0 as i32, s.c.1 as i32);
            self.thuds.push((c.0, c.1, (hardest / 3.0).min(4.0)));
        }
        // The pieces fly on with what is left of their speed; those that were the ones to hit
        // hit again at once, softer, and settle or break further.
        let Some(pieces) = self.fracture(&cells, &vel) else { return };
        let at: HashMap<usize, usize> = cells.iter().enumerate().map(|(k, &i)| (i, k)).collect();
        for piece in pieces {
            if piece.len() < 3 {
                continue;
            }
            let v = piece.iter().fold((0.0, 0.0), |a, i| (a.0 + vel[at[i]].0, a.1 + vel[at[i]].1));
            let v = (v.0 / piece.len() as f32 / ms * 0.6, v.1 / piece.len() as f32 / ms * 0.6);
            self.loosen(piece, None, v, s.spin * 0.6);
        }
    }

    /// Whether a falling piece that just touched down on something firm breaks (and breaks it).
    pub(super) fn landing_breaks(&mut self, piece: &Piece) -> bool {
        if piece.fallen < 2.0 {
            return false;
        }
        let in_piece = |i: usize| piece.cells.binary_search_by(|q| i.cmp(q)).is_ok();
        let firm = piece.cells.iter().any(|&i| {
            let t = i + W;
            t < W * H && !in_piece(t) && (self.mask[t] != 0 || matches!(self.cells[t].m.kind(), Kind::Static | Kind::Powder))
        });
        if !firm {
            return false;
        }
        let v = (2.0 * G * piece.fallen * CELL_M).sqrt();
        let cells = piece.cells.clone();
        self.fracture(&cells, &vec![(0.0, v); cells.len()]).is_some()
    }

    /// Breaks the solid cells `cells` that hit something at `vel` (m/s, one per cell): splits
    /// them into pieces whose size follows the energy each part brings, opens the cracks between
    /// the pieces and throws what is crushed finer than a cell as grains. Returns the pieces, or
    /// None if it holds.
    pub(super) fn fracture(&mut self, cells: &[usize], vel: &[(f32, f32)]) -> Option<Vec<Vec<usize>>> {
        let n = cells.len();
        let at: HashMap<usize, usize> = cells.iter().enumerate().map(|(k, &i)| (i, k)).collect();
        let mut piece = vec![u32::MAX; n];
        let mut crushed = vec![false; n];
        let mut grow = VecDeque::new();
        let mut pieces = 0u32;
        // How many pieces it makes, on average: under one and a half, it holds.
        let sizes: Vec<f32> = (0..n).map(|k| piece_size(self.cells[cells[k]].m, (vel[k].0 * vel[k].0 + vel[k].1 * vel[k].1).sqrt())).collect();
        let expected: f32 = sizes.iter().map(|s| 1.0 / (s * s).max(1.0)).sum();
        if expected < 1.5 {
            return None;
        }
        for k in 0..n {
            let size = sizes[k];
            if size < 1.0 && self.chance_f(1.0 - size) {
                crushed[k] = true;
                continue;
            }
            if self.chance_f(1.0 / (size * size)) {
                piece[k] = pieces;
                pieces += 1;
                grow.push_back(k);
            }
        }
        // Each piece grows from its seed through the cells nearest to it.
        let next = |k: usize| {
            let i = cells[k];
            let (x, y) = ((i % W) as i32, (i / W) as i32);
            Self::neighbors(x, y).into_iter().filter(|&(nx, ny)| Self::inb(nx, ny)).filter_map(|(nx, ny)| at.get(&Self::idx(nx, ny)).copied()).collect::<Vec<usize>>()
        };
        loop {
            while let Some(k) = grow.pop_front() {
                for j in next(k) {
                    if piece[j] == u32::MAX && !crushed[j] {
                        piece[j] = piece[k];
                        grow.push_back(j);
                    }
                }
            }
            // Cells cut off from every seed by crushed ones make pieces of their own.
            match (0..n).find(|&k| piece[k] == u32::MAX && !crushed[k]) {
                Some(k) => {
                    piece[k] = pieces;
                    pieces += 1;
                    grow.push_back(k);
                }
                None => break,
            }
        }
        // The cracks: where two pieces meet, the cells on one side crumble.
        let crack: Vec<usize> = (0..n).filter(|&k| !crushed[k] && next(k).iter().any(|&j| !crushed[j] && piece[j] < piece[k])).collect();
        for k in crack {
            crushed[k] = true;
        }
        let ms = ms_per_cell_step();
        for k in 0..n {
            if !crushed[k] {
                continue;
            }
            let i = cells[k];
            let m = self.cells[i].m;
            let (x, y) = ((i % W) as i32, (i / W) as i32);
            self.set(x, y, Mat::Empty);
            let Some(grain) = rubble(m) else { continue };
            // Thrown back up and aside a little, as the shards of a broken plate.
            let (vx, vy) = (vel[k].0 / ms, vel[k].1 / ms);
            let spread = (vx * vx + vy * vy).sqrt();
            let side = ((self.rand() % 1000) as f32 / 500.0 - 1.0) * 0.35 * spread;
            let up = (self.rand() % 1000) as f32 / 1000.0 * 0.3 + 0.08;
            if self.ejecta.len() < 14_000 {
                self.ejecta.push(Ejecta { x: x as f32 + 0.5, y: y as f32 - 0.5, vx: (vx * 0.4 + side).clamp(-3.0, 3.0), vy: (-vy.abs() * up).clamp(-3.0, 0.0), m: grain });
            } else {
                self.set(x, y, grain);
            }
        }
        let mut out: Vec<Vec<usize>> = vec![vec![]; pieces as usize];
        for k in 0..n {
            if !crushed[k] {
                out[piece[k] as usize].push(cells[k]);
            }
        }
        out.retain(|p| !p.is_empty());
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cold, still, a slab of bedrock from row `ground` down.
    fn world(ground: i32) -> World {
        let mut w = World::new();
        w.day_cycle = false;
        w.climate_fixed = Some(-5.0);
        w.reset_temps();
        for x in 0..W as i32 {
            for y in ground..H as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        w
    }

    fn count(w: &World, m: Mat) -> usize {
        w.cells.iter().filter(|c| c.m == m).count()
    }

    fn run(w: &mut World, steps: usize) {
        for _ in 0..steps {
            w.step();
        }
    }

    #[test]
    fn a_leaning_stroke_of_ice_tips_over_falls_and_shatters() {
        // A thick stroke of ice resting on the rock by its foot and leaning far to the right,
        // as painted in the sky: its centre of mass is well beyond its foot.
        let mut w = world(450);
        for k in 0..160 {
            let (x, y) = (300 + k, 449 - k);
            for dx in 0..6 {
                w.set(x + dx, y, Mat::Ice);
            }
        }
        let n = count(&w, Mat::Ice);
        let mut tipped = false;
        for _ in 0..600 {
            w.step();
            tipped |= w.slabs.iter().any(|s| s.angle > 0.3);
        }
        assert!(tipped, "it turned over its foot");
        assert!((0..W * H).filter(|&i| w.cells[i].m == Mat::Ice).all(|i| i / W > 400), "nothing is left standing in the air");
        let hail = count(&w, Mat::Hail);
        assert!(hail > n / 10, "it shattered on the rock: {hail} hail of {n}");
        assert!(count(&w, Mat::Ice) + hail > n * 8 / 10, "the ice is still there");
    }

    #[test]
    fn a_balanced_block_rests_and_one_over_the_edge_tips() {
        let mut w = world(450);
        // A pedestal of glass (not one body with the rock), and two blocks of ice on it: one
        // centred, one hanging two thirds over the edge.
        for x in 100..140 {
            for y in 430..450 {
                w.set(x, y, Mat::Glass);
            }
        }
        for x in 105..125 {
            for y in 420..430 {
                w.set(x, y, Mat::Ice);
            }
        }
        for x in 300..320 {
            for y in 430..450 {
                w.set(x, y, Mat::Glass);
            }
        }
        for x in 313..343 {
            for y in 420..430 {
                w.set(x, y, Mat::Ice);
            }
        }
        run(&mut w, 400);
        assert!((105..125).all(|x| w.raw(x, 425) == Mat::Ice), "the centred block stays");
        // (It may come to lean against the side of the pedestal, as a plank would.)
        let fallen: Vec<usize> = (0..W * H).filter(|&i| w.cells[i].m == Mat::Ice && i % W > 200).collect();
        let mean_y = fallen.iter().map(|i| i / W).sum::<usize>() / fallen.len();
        assert!(mean_y > 428, "the overhanging block tipped off: its cells are at row {mean_y} on average");
        assert!((0..W * H).any(|i| matches!(w.cells[i].m, Mat::Ice | Mat::Hail) && i % W > 320 && i / W > 440), "it fell beside the pedestal");
    }

    #[test]
    fn a_long_overhang_of_rock_breaks_off_where_a_short_one_holds() {
        let mut w = world(450);
        // A cliff with two ledges 10 cells thick: 30 cells and 300 cells long.
        for x in 200..260 {
            for y in 300..450 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 260..290 {
            for y in 300..310 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 200..260 {
            for y in 300..310 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 600..660 {
            for y in 300..450 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 660..940 {
            for y in 300..310 {
                w.set(x, y, Mat::Stone);
            }
        }
        w.step();
        w.structure_pass();
        assert!(w.strain[World::idx(262, 305)] < 128, "the short ledge is not overloaded");
        run(&mut w, 300);
        assert_eq!(w.raw(285, 305), Mat::Stone, "the short ledge holds");
        assert_ne!(w.raw(900, 305), Mat::Stone, "the long one broke off");
    }

    #[test]
    fn earth_is_granular_it_slumps_but_stands_steep() {
        let mut w = world(450);
        // A wall of earth 40 high and 10 wide.
        for x in 400..410 {
            for y in 410..450 {
                w.set(x, y, Mat::Dirt);
            }
        }
        run(&mut w, 1500);
        let top = |x: i32| (0..H as i32).find(|&y| w.raw(x, y) == Mat::Dirt).unwrap_or(450);
        assert!(top(405) > 420, "the wall slumped: top at {}", top(405));
        assert!(top(395) < 450 && top(415) < 450, "into a heap on both sides");
        assert_eq!(count(&w, Mat::Dirt), 400, "no earth is lost");
        // Steeper than sand: the heap is narrow.
        assert!(top(380) >= 450 && top(430) >= 450, "earth holds a steep slope");
    }

    #[test]
    fn a_body_has_a_mass_a_centre_and_an_inertia() {
        let mut w = world(450);
        for x in 100..110 {
            for y in 100..104 {
                w.set(x, y, Mat::Ice);
            }
        }
        let cells: Vec<usize> = (0..W * H).filter(|&i| w.cells[i].m == Mat::Ice).collect();
        w.loosen(cells, Some((100.0, 104.0)), (0.0, 0.0), 0.0);
        let s = &w.slabs[0];
        assert!((s.mass - 40.0 * 0.92).abs() < 1e-3);
        assert!((s.c.0 - 105.0).abs() < 1e-3 && (s.c.1 - 102.0).abs() < 1e-3, "{:?}", s.c);
        // A 10 x 4 plate: I = m (a² + b²) / 12.
        let plate = s.mass * (100.0 + 16.0) / 12.0;
        assert!((s.inertia - plate).abs() / plate < 1e-3, "{} vs {plate}", s.inertia);
    }
}
