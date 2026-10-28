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
//! pull to keep it turning (the edge can push, not pull); it then flies on, spinning. A free body
//! with room under it falls the same way, as a rigid body (a large one drops straight down).
//!
//! In flight a body keeps its momentum m·v and its angular momentum I·ω. When it runs into
//! something firm, each point of contact pushes back along the normal of the surface there with
//! an impulse J (J/m changes its velocity, r×J/I its spin), just enough that no point goes on into
//! the surface, plus a bounce of its matter's restitution; friction, at most μ·J, brakes the
//! points along the surface. So a body landing on a corner off its centre of mass is turned over
//! onto a face, one dropped across the edge of a ledge turns off it, a block thrown along the
//! ground slides and stops, a round stone rolls down a slope. It comes to rest when it hardly moves
//! any more, and the statics take it from there.
//!
//! When it hits something it breaks according to the energy it brings: a small share of it opens cracks, and
//! opening a crack across a section costs the fracture energy of the matter (J/m²). The faster
//! a part hits, the smaller the pieces: ice and glass shatter from a few metres, rock only after a
//! long fall, wood and metal not at all. What is crushed finer than a cell becomes grains: hail,
//! shards, sand.

use super::blast::strength;
use super::*;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::f32::consts::FRAC_PI_2;

use super::units::{cell_m, G};
/// The pull of gravity on a body in flight, cells per tick² (the bodies move every tick).
fn fall() -> f32 {
    super::units::g_tick()
}
/// Share of the energy of an impact that goes into opening cracks; the rest bounces back, shakes
/// the ground and warms it.
const CRACK_SHARE: f32 = 0.0005;
/// Bodies larger than this many cells do not tip as a whole (they are mountains).
const MAX_SLAB: usize = 150_000;
/// At most this many bodies start to tip in one pass.
const MAX_TIPS: usize = 4;
/// A falling body of this many cells falls as a rigid body that can turn (see `tip_pass`);
/// a crumb or a large one, or one when this many bodies already fly, drops straight down.
const MIN_TUMBLER: usize = 6;
const MAX_TUMBLER: usize = 6_000;
const MAX_FLYING: usize = 24;
/// Coefficient of friction of a body against what it hits (rock on rock is about 0.6).
const FRICTION: f32 = 0.55;
/// A body in contact whose fastest point moves slower than this (cells per step) for
/// `REST_STEPS` steps in a row comes to rest.
const SETTLE: f32 = 0.05;
const REST_STEPS: u8 = 6;
/// A body still bumping along after this many steps of contact comes to rest all the same.
const MAX_CONTACT: u16 = 900;

/// The sections of one anchored body, as `stress_pass` last worked them out: the load on each of
/// its cells (in `World::strain`'s units) and the sections that cannot carry theirs.
/// The push of the liquids on a solid body: where it bears (cells) and how hard, kN per metre of
/// depth (x, y; down positive).
#[derive(Clone, Copy, Debug)]
pub struct LiquidPush {
    pub x: f32,
    pub y: f32,
    pub fx: f32,
    pub fy: f32,
}

pub(super) struct BodyStress {
    strain: Vec<u8>,
    cracks: Vec<Crack>,
}

/// How far a share of the load (see `section_load`) may be off in the stress view when a body's
/// loads have barely changed since it was last worked out: under it, the old figures stand.
const SHARE_SLACK: f64 = 0.01;

/// In the tests: every run of a plan is checked against the body worked out from scratch.
#[cfg(test)]
pub(super) static CHECK_PLANS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The sections of an anchored body, worked out from its shape alone (see `stress_plan`): run
/// again with the weights and the liquids' push of the moment (see `replay_stress`), it gives
/// what working the body out from scratch gives, to the bit, without looking for its sections
/// again. The liquids lapping at a bank change the push on it every pass, not its shape.
pub(super) struct StressPlan {
    /// Each place's density and centre (x, y), read once.
    rho: Vec<f64>,
    at: Vec<(f64, f64)>,
    steps: Vec<PlanStep>,
    /// The cells of the sections, one after the other, and their places.
    fibre_cells: Vec<usize>,
    fibre_places: Vec<u32>,
    /// The least that the bonds of any section hold (see `section_load`), and the span of the
    /// body (cells across, cells down): how far a change of the loads can move a share.
    weakest: f64,
    span: (f64, f64),
}

enum PlanStep {
    /// The places `lo..hi` (all at one distance from the anchors) take their loads.
    Load(u32, u32),
    /// What place `a` carries goes to place `b`: the parts they head join.
    Join(u32, u32),
    /// A section to check.
    Section(Section),
}

/// A section of a plan: the place that heads what it carries, its distance from the anchors,
/// its fibres (`fibre_cells[a..b]`), and, unless it is vegetation (which never cracks), its two
/// edge fibres: where (x, y), which cell, and what its bonds hold about it.
struct Section {
    root: u32,
    level: u32,
    fibres: (u32, u32),
    veg: bool,
    edges: [(f64, f64, usize, f64); 2],
}

/// A body worked out: its plan, the push of the liquids on each of its places when it was last
/// worked out, the result, and the least margin of any section to cracking or holding again
/// (the gap of its share to 1, times what its bonds hold).
pub(super) struct Worked {
    plan: std::sync::Arc<StressPlan>,
    push: Vec<(f32, f32)>,
    done: std::sync::Arc<BodyStress>,
    margin: f64,
}

/// Speed in m/s of one cell per tick.
fn ms_per_cell_step() -> f32 {
    super::units::metres_per_s(1.0)
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

/// Coefficient of restitution of matter `m`: the share of the speed along the normal a body of
/// it bounces back with.
fn restitution(m: Mat) -> f32 {
    match m {
        Mat::Wood | Mat::Trunk | Mat::Branch => 0.45,
        Mat::Iron | Mat::Steel | Mat::Glass | Mat::Diamond => 0.4,
        Mat::Ice => 0.35,
        Mat::Clay | Mat::Leaf | Mat::Sapling => 0.05,
        _ => 0.3,
    }
}

/// Typical size of the pieces, in cells, when matter `m` hits at `v` m/s.
fn piece_size(m: Mat, v: f32) -> f32 {
    if v < 0.3 || rubble(m).is_none() {
        return f32::INFINITY;
    }
    // Breaking a body of area A into squares of side s opens about 2A/s of crack (per metre of
    // depth); its kinetic energy is ½ρAv². The share that cracks it pays for the cracks.
    4.0 * toughness(m) / (CRACK_SHARE * density(m) * 1000.0 * v * v) / cell_m()
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
    /// Steps in a row it has lain nearly still against something, and steps it has been in
    /// contact at all.
    rest: u8,
    contact: u16,
}

/// At most this many free bodies are shown in the stress view.
const MAX_SHOWN: usize = 64;

/// How a free body resting on something stands, as `tip_pass` last found it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Balance {
    /// Its centre of mass is over what it rests on.
    Rests,
    /// It tips over its edge.
    Tips,
    /// It should tip, but hangs by a part above its centre of mass.
    Hangs,
    /// It should tip, and will once there is room (too many bodies tip at once, or it is a mountain).
    Waits,
}

/// A free body (not held by anything fixed) resting on something, for the stress view: its
/// centre of mass (cells), how many cells, its weight and the push of the liquids on it (kN per
/// metre of depth), the span of what it rests on (x, cells) and the underside of it (row), the
/// edge it turns over if it does, and the moment of its weight about that edge (kN·m per metre:
/// positive, it tips; negative, what holds it about the nearer edge).
#[derive(Clone, Debug)]
pub struct FreeBody {
    pub c: (f32, f32),
    pub cells: usize,
    pub weight: f32,
    pub push: (f32, f32),
    pub span: (f32, f32),
    pub base: f32,
    pub hinge: Option<(f32, f32)>,
    pub moment: f32,
    pub state: Balance,
}

/// What a body in flight ran into.
struct Hit {
    /// Whether all of it was liquid.
    soft: bool,
    /// Where it touched (cells), and the normal of the surface there, out of it (towards the
    /// body), of length 1.
    points: Vec<((f32, f32), (f32, f32))>,
    /// The firmest matter it ran into.
    m: Mat,
}

/// At most this many points of contact are worked out in a hit.
const MAX_POINTS: usize = 16;

impl Slab {
    pub fn cells(&self) -> impl Iterator<Item = &usize> {
        self.drawn.iter().map(|d| &d.0)
    }

    /// Its weight, kN per metre of depth.
    pub fn weight(&self) -> f32 {
        self.mass * G * cell_m() * cell_m()
    }

    /// The edge it still turns on, if it does (cells).
    pub fn edge(&self) -> Option<(f32, f32)> {
        self.hinge
    }

    /// The moment of its weight about the edge it turns on, kN·m per metre (positive: clockwise).
    pub fn moment(&self) -> f32 {
        self.hinge.map_or(0.0, |h| self.weight() * (self.c.0 - h.0) * cell_m())
    }

    /// The pose it has after `dt` of a step: turned about its edge, or moved and turned about
    /// its centre of mass.
    fn pose_after(&self, dt: f32) -> ((f32, f32), f32) {
        let angle = self.angle + self.spin * dt;
        match self.hinge {
            Some(h) => {
                let (sn, cs) = (self.spin * dt).sin_cos();
                let r = (self.c.0 - h.0, self.c.1 - h.1);
                ((h.0 + r.0 * cs - r.1 * sn, h.1 + r.0 * sn + r.1 * cs), angle)
            }
            None => ((self.c.0 + self.v.0 * dt, self.c.1 + self.v.1 * dt), angle),
        }
    }

    /// Velocity of the point `p` of the body, cells per step.
    pub(super) fn velocity_at(&self, p: (f32, f32)) -> (f32, f32) {
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
#[derive(Clone)]
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
        // The push of the liquids on each cell (in cells of water, see `liquid_push`), counted in
        // quarters of a cell in the fingerprints so that a body is worked out again when it changes.
        let push: Vec<(f32, f32)> = order.par_iter().map(|&i| self.liquid_push(i)).collect();
        let quarters = |k: usize| push.get(k).map_or(0, |&(fx, fy)| (((fx * 4.0).round() as i32 as u64) << 20) ^ ((fy * 4.0).round() as i32 as u64 & 0xf_ffff));
        let key = order.iter().enumerate().fold(order.len() as u64, |h, (k, &i)| (h ^ ((i as u64) << 8 | self.cells[i].m as u64) ^ quarters(k).rotate_left(29)).wrapping_mul(0x100_0000_01b3));
        if key == self.strain_key {
            return;
        }
        self.strain_key = key;
        self.strain.fill(0);
        let n = order.len();
        if n == 0 {
            self.stress_cache.clear();
            self.stress_plans.clear();
            return;
        }
        let level = std::mem::take(&mut self.level);
        let mut pos = std::mem::take(&mut self.stress_pos);
        pos.resize(W() * H(), u32::MAX);
        for (k, &i) in order.iter().enumerate() {
            pos[i] = k as u32;
        }
        // The anchored bodies, each the list of its places in `order` (rising). The sections of
        // one body never lean on another's, so each is worked out on its own: side by side, and
        // only again once the body itself changed (a tree that burns leaves the rock under it as
        // it was).
        let mut body_of = vec![u32::MAX; n];
        let mut count = 0u32;
        let mut stack: Vec<u32> = vec![];
        for k0 in 0..n as u32 {
            if body_of[k0 as usize] != u32::MAX {
                continue;
            }
            body_of[k0 as usize] = count;
            stack.push(k0);
            while let Some(k) = stack.pop() {
                let i = order[k as usize];
                let (x, y) = xy_i32(i);
                for (nx, ny) in Self::neighbors(x, y) {
                    if !Self::inb(nx, ny) {
                        continue;
                    }
                    let j = Self::idx(nx, ny);
                    let kj = pos[j];
                    if kj != u32::MAX && body_of[kj as usize] == u32::MAX && self.same_body(i, j) {
                        body_of[kj as usize] = count;
                        stack.push(kj);
                    }
                }
            }
            count += 1;
        }
        // Each body's places, rising, a fingerprint of its cells, and where each place comes in
        // its body's list.
        let mut bodies: Vec<Vec<u32>> = vec![vec![]; count as usize];
        let mut keys: Vec<u64> = vec![0; count as usize];
        // (And a fingerprint of the shape alone: the plan of the body, see `StressPlan`.)
        let mut shapes: Vec<u64> = vec![0; count as usize];
        for (k, &b) in body_of.iter().enumerate() {
            let i = order[k];
            let cell = (i as u64) << 8 | self.cells[i].m as u64;
            keys[b as usize] = (keys[b as usize] ^ cell ^ quarters(k).rotate_left(29)).wrapping_mul(0x100_0000_01b3);
            shapes[b as usize] = (shapes[b as usize] ^ cell).wrapping_mul(0x100_0000_01b3);
            bodies[b as usize].push(k as u32);
        }
        for ((key, shape), b) in keys.iter_mut().zip(shapes.iter_mut()).zip(&bodies) {
            *key ^= b.len() as u64;
            *shape ^= b.len() as u64;
        }
        // (Only the bodies that have to be worked out again.)
        let mut local = std::mem::take(&mut self.stress_local);
        local.resize(n, 0);
        for (members, key) in bodies.iter().zip(&keys) {
            if !self.stress_cache.contains_key(key) {
                for (l, &k) in members.iter().enumerate() {
                    local[k as usize] = l as u32;
                }
            }
        }
        // A body seen before keeps its result; one whose shape was seen before is run again on
        // its plan, or not even that while its loads have moved too little to change anything.
        let worked: Vec<(std::sync::Arc<BodyStress>, Option<Worked>)> = {
            let (this, level, pos, local, push) = (&*self, &level, &pos, &local, &push);
            bodies
                .par_iter()
                .zip(keys.par_iter().zip(shapes.par_iter()))
                .map(|(members, (key, shape))| {
                    if let Some(done) = this.stress_cache.get(key) {
                        return (done.clone(), None);
                    }
                    let now: Vec<(f32, f32)> = members.iter().map(|&k| push.get(k as usize).copied().unwrap_or((0.0, 0.0))).collect();
                    let plan = match this.stress_plans.get(shape) {
                        Some(w) => {
                            if this.barely_moved(w, &now) {
                                return (w.done.clone(), None);
                            }
                            w.plan.clone()
                        }
                        None => std::sync::Arc::new(this.stress_plan(order, members, local, level, pos)),
                    };
                    let (done, margin) = Self::replay_stress(&plan, &now);
                    // (In the tests, every run of a plan is checked against the body worked out from scratch.)
                    #[cfg(test)]
                    if CHECK_PLANS.load(std::sync::atomic::Ordering::Relaxed) {
                        let fresh = this.body_stress(order, members, local, level, pos, push);
                        assert_eq!(done.strain, fresh.strain, "a plan gives other strains than the body worked out");
                        let crack = |c: &Crack| (c.load.to_bits(), c.level, c.fibres.clone(), c.pivot);
                        assert_eq!(done.cracks.iter().map(crack).collect::<Vec<_>>(), fresh.cracks.iter().map(crack).collect::<Vec<_>>());
                    }
                    let done = std::sync::Arc::new(done);
                    (done.clone(), Some(Worked { plan, push: now, done, margin }))
                })
                .collect()
        };
        let mut plans = std::mem::take(&mut self.stress_plans);
        let mut results = Vec::with_capacity(worked.len());
        let mut kept = std::collections::HashMap::new();
        for ((done, fresh), shape) in worked.into_iter().zip(&shapes) {
            if let Some(w) = fresh.or_else(|| plans.remove(shape)) {
                kept.insert(*shape, w);
            }
            results.push(done);
        }
        self.stress_plans = kept;
        let mut cracks: Vec<Crack> = vec![];
        self.pushes.clear();
        for (members, done) in bodies.iter().zip(&results) {
            for (&k, &shown) in members.iter().zip(&done.strain) {
                self.strain[order[k as usize]] = shown;
            }
            cracks.extend(done.cracks.iter().cloned());
            // The push of the liquids on each wetted face of the body, at its centre of pressure
            // (for the stress view): the two walls of a basin are pushed apart, which a sum over
            // the body would hide.
            if !push.is_empty() {
                let mut wet: Vec<(bool, usize, usize, f32)> = vec![];
                for &k in members {
                    let (px, py) = push[k as usize];
                    let i = order[k as usize];
                    let (x, y) = xy(i);
                    if px != 0.0 {
                        wet.push((false, x * 2 + (px > 0.0) as usize, y, px));
                    }
                    if py != 0.0 {
                        wet.push((true, y * 2 + (py > 0.0) as usize, x, py));
                    }
                }
                wet.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
                let mut run = 0;
                while run < wet.len() {
                    let (down, line, mut at, _) = wet[run];
                    let (mut f, mut along) = (0.0f32, 0.0f32);
                    let mut end = run;
                    while end < wet.len() && wet[end].0 == down && wet[end].1 == line && wet[end].2 <= at + 1 {
                        at = wet[end].2;
                        f += wet[end].3;
                        along += wet[end].3 * (at as f32 + 0.5);
                        end += 1;
                    }
                    let (across, along) = ((line / 2) as f32 + 0.5, along / f);
                    let f = f * super::units::cell_of_water_kn();
                    self.pushes.push(if down {
                        LiquidPush { x: along, y: across, fx: 0.0, fy: f }
                    } else {
                        LiquidPush { x: across, y: along, fx: f, fy: 0.0 }
                    });
                    run = end;
                }
            }
        }
        self.stress_cache = keys.into_iter().zip(results).collect();
        for &i in order {
            pos[i] = u32::MAX;
        }
        self.stress_pos = pos;
        self.stress_local = local;
        // (The same order whatever the order of the bodies: the outermost section first, then
        // by place, among cracks under the same load.)
        cracks.sort_by(|a, b| b.level.cmp(&a.level).then(a.fibres[0].cmp(&b.fibres[0])));
        // The worst first; a crack whose part beyond has already broken loose waits for the
        // next pass (its load has changed).
        cracks.sort_by(|a, b| b.load.total_cmp(&a.load));
        // Which crack's part beyond each cell belongs to (0: none yet).
        let mut taken = vec![0u32; if cracks.is_empty() { 0 } else { W() * H() }];
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
                let (x, y) = xy_i32(i);
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
            // A part that rests on what is under it, its centre of mass over that base, is held
            // up in compression whatever its bonds (a crust of rock lying on earth, a roof
            // over a short opening): only one that hangs or tips breaks off.
            if !ok || self.rests_on_its_base(&region, |j| taken[j] == id) {
                continue;
            }
            // It turns about the edge between the pivot fibre and the cell under it that held it.
            let (px, py) = ((crack.pivot % W()) as i32, (crack.pivot / W()) as i32);
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

    /// The plan of one anchored body (`members`: its places in `order`, rising; `local`: where
    /// each place of `order` comes in its body's list): what `body_stress` does that hangs on the
    /// shape alone, the joining of the parts from the farthest cells inwards and the edges and
    /// bonds of each section, written down to be run again on any loads (see `replay_stress`).
    fn stress_plan(&self, order: &[usize], members: &[u32], local: &[u32], level: &[u32], pos: &[u32]) -> StressPlan {
        let n = members.len();
        let local = |k: u32| local[k as usize];
        let mut up: Vec<u32> = (0..n as u32).collect();
        fn find(up: &mut [u32], mut k: u32) -> u32 {
            while up[k as usize] != k {
                let next = up[up[k as usize] as usize];
                up[k as usize] = next;
                k = next;
            }
            k
        }
        let cell: Vec<usize> = members.iter().map(|&k| order[k as usize]).collect();
        let (mut steps, mut fibre_cells, mut fibre_places) = (vec![], vec![], vec![]);
        let mut fibres: Vec<(u32, u32)> = vec![];
        let mut weakest = f64::INFINITY;
        let xy = |i: usize| ((i % W()) as f64 + 0.5, (i / W()) as f64 + 0.5);
        let mut hi = n;
        while hi > 0 {
            let d = level[cell[hi - 1]];
            let mut lo = hi;
            while lo > 0 && level[cell[lo - 1]] == d {
                lo -= 1;
            }
            steps.push(PlanStep::Load(lo as u32, hi as u32));
            for k in lo..hi {
                let i = cell[k];
                let (x, y) = xy_i32(i);
                for (nx, ny) in Self::neighbors(x, y) {
                    if !Self::inb(nx, ny) {
                        continue;
                    }
                    let j = Self::idx(nx, ny);
                    if pos[j] == u32::MAX || level[j] < d || !self.same_body(i, j) {
                        continue;
                    }
                    let (a, b) = (find(&mut up, k as u32), find(&mut up, local(pos[j])));
                    if a != b {
                        up[a as usize] = b;
                        steps.push(PlanStep::Join(a, b));
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
                let from = fibre_cells.len() as u32;
                fibre_cells.extend(fibres[a..b].iter().map(|&(_, k)| cell[k as usize]));
                fibre_places.extend(fibres[a..b].iter().map(|&(_, k)| k));
                let cells = &fibre_cells[from as usize..];
                // As `section_load` finds them.
                let m = self.cells[cells[0]].m;
                let sigma = strength(m) as f64 * 1.0e6;
                let mut edges = [(0.0, 0.0, cells[0], 0.0); 2];
                if !m.is_vegetation() {
                    for (e, dir) in [-1.0f64, 1.0].into_iter().enumerate() {
                        let p = *cells.iter().max_by(|&&a, &&b| {
                            let (ax, ay) = xy(a);
                            let (bx, by) = xy(b);
                            (dir * ax + ay).total_cmp(&(dir * bx + by))
                        }).unwrap();
                        let (px, py) = xy(p);
                        let lever: f64 = cells.iter().map(|&i| {
                            let (x, y) = xy(i);
                            ((x - px).powi(2) + (y - py).powi(2)).sqrt() + 0.5
                        }).sum();
                        let holding = sigma / 3.0 * lever;
                        weakest = weakest.min(holding);
                        edges[e] = (px, py, p, holding);
                    }
                }
                steps.push(PlanStep::Section(Section { root, level: d, fibres: (from, fibre_cells.len() as u32), veg: m.is_vegetation(), edges }));
                a = b;
            }
            hi = lo;
        }
        let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
        for &i in &cell {
            let (x, y) = (i % W(), i / W());
            (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
        }
        let span = if cell.is_empty() { (0.0, 0.0) } else { ((x1 - x0 + 1) as f64, (y1 - y0 + 1) as f64) };
        let rho = cell.iter().map(|&i| density(self.cells[i].m) as f64).collect();
        let at = cell.iter().map(|&i| ((i % W()) as f64 + 0.5, (i / W()) as f64 + 0.5)).collect();
        StressPlan { rho, at, steps, fibre_cells, fibre_places, weakest, span }
    }

    /// The load on every section of a body by its plan, with `push` the push of the liquids on
    /// each of its places: what `body_stress` finds, to the bit (the same sums in the same order,
    /// the same shares). Also the least margin of any section, as `Worked` keeps it.
    fn replay_stress(plan: &StressPlan, push: &[(f32, f32)]) -> (BodyStress, f64) {
        let n = plan.rho.len();
        let mut mass = vec![0.0f64; n];
        let mut moment = vec![0.0f64; n];
        let mut side = vec![0.0f64; n];
        let mut side_moment = vec![0.0f64; n];
        let mut strain = vec![0u8; n];
        let mut cracks: Vec<Crack> = vec![];
        let mut margin = f64::INFINITY;
        for step in &plan.steps {
            match *step {
                PlanStep::Load(lo, hi) => {
                    for k in lo as usize..hi as usize {
                        let (fx, fy) = push[k];
                        let (x, y) = plan.at[k];
                        let w = plan.rho[k] + fy as f64;
                        mass[k] = w;
                        moment[k] = w * x;
                        side[k] = fx as f64;
                        side_moment[k] = fx as f64 * y;
                    }
                }
                PlanStep::Join(a, b) => {
                    let (a, b) = (a as usize, b as usize);
                    mass[b] += mass[a];
                    moment[b] += moment[a];
                    side[b] += side[a];
                    side_moment[b] += side_moment[a];
                }
                PlanStep::Section(ref s) => {
                    let r = s.root as usize;
                    let cells = &plan.fibre_cells[s.fibres.0 as usize..s.fibres.1 as usize];
                    let (load, pivot) = Self::section_share(s, cells[0], (mass[r], moment[r]), (side[r], side_moment[r]));
                    if !s.veg {
                        margin = margin.min((1.0 - load as f64).abs() * s.edges[0].3.min(s.edges[1].3));
                    }
                    let shown = (load.clamp(0.0, 2.0) * 127.5) as u8;
                    for &k in &plan.fibre_places[s.fibres.0 as usize..s.fibres.1 as usize] {
                        strain[k as usize] = shown;
                    }
                    if load > 1.0 {
                        cracks.push(Crack { load, level: s.level, fibres: cells.to_vec(), pivot });
                    }
                }
            }
        }
        (BodyStress { strain, cracks }, margin)
    }

    /// `section_load` on a section of a plan, whose edges and bonds are already known.
    fn section_share(s: &Section, first: usize, (mass, moment): (f64, f64), (side, side_moment): (f64, f64)) -> (f32, usize) {
        if s.veg || (mass <= 0.0 && side == 0.0) {
            return (0.0, first);
        }
        let (mut worst, mut pivot) = (0.0f64, first);
        for (e, dir) in [-1.0f64, 1.0].into_iter().enumerate() {
            let (px, py, p, holding) = s.edges[e];
            let turning = G as f64 * cell_m() as f64 * 1000.0 * dir * ((moment - mass * px) - (side_moment - side * py));
            if turning <= 0.0 {
                continue;
            }
            let load = if holding > 0.0 { turning / holding } else { 10.0 };
            if load > worst {
                worst = load;
                pivot = p;
            }
        }
        (worst as f32, pivot)
    }

    /// Whether the push of the liquids on a body (`now`, by place) has moved so little since it
    /// was last worked out that no section can have changed: the turning moment about an edge
    /// moves by at most the change of each push times its lever, which the span of the body
    /// bounds; that cannot carry any section across its breaking point (the least margin) nor
    /// move any share by more than `SHARE_SLACK`. Far from breaking, the sums are not done again.
    fn barely_moved(&self, w: &Worked, now: &[(f32, f32)]) -> bool {
        if w.push.len() != now.len() || !(w.plan.weakest > 0.0) {
            return false;
        }
        let (across, down) = w.plan.span;
        let turn: f64 = w.push.iter().zip(now).map(|(a, b)| (b.1 - a.1).abs() as f64 * across + (b.0 - a.0).abs() as f64 * down).sum::<f64>()
            * G as f64 * cell_m() as f64 * 1000.0;
        turn < w.margin && turn < SHARE_SLACK * w.plan.weakest
    }

    /// The load on every section of one anchored body (`members`: its places in `order`, rising;
    /// `local`: where each place of `order` comes in its body's list), and the sections that
    /// cannot carry it, as `stress_pass` reads them.
    #[cfg(test)]
    fn body_stress(&self, order: &[usize], members: &[u32], local: &[u32], level: &[u32], pos: &[u32], push: &[(f32, f32)]) -> BodyStress {
        let n = members.len();
        let local = |k: u32| local[k as usize];
        let mut up: Vec<u32> = (0..n as u32).collect();
        // What each part carries: its weight (and the liquids' push down) as Σ w and Σ w·x, and
        // the liquids' push sideways as Σ f and Σ f·y.
        let mut mass = vec![0.0f64; n];
        let mut moment = vec![0.0f64; n];
        let mut side = vec![0.0f64; n];
        let mut side_moment = vec![0.0f64; n];
        let mut strain = vec![0u8; n];
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
        let cell = |k: usize| order[members[k] as usize];
        let mut hi = n;
        while hi > 0 {
            let d = level[cell(hi - 1)];
            let mut lo = hi;
            while lo > 0 && level[cell(lo - 1)] == d {
                lo -= 1;
            }
            for k in lo..hi {
                let i = cell(k);
                let (fx, fy) = push.get(members[k] as usize).copied().unwrap_or((0.0, 0.0));
                let w = density(self.cells[i].m) as f64 + fy as f64;
                mass[k] = w;
                moment[k] = w * ((i % W()) as f64 + 0.5);
                side[k] = fx as f64;
                side_moment[k] = fx as f64 * ((i / W()) as f64 + 0.5);
            }
            for k in lo..hi {
                let i = cell(k);
                let (x, y) = xy_i32(i);
                for (nx, ny) in Self::neighbors(x, y) {
                    if !Self::inb(nx, ny) {
                        continue;
                    }
                    let j = Self::idx(nx, ny);
                    if pos[j] == u32::MAX || level[j] < d || !self.same_body(i, j) {
                        continue;
                    }
                    let (a, b) = (find(&mut up, k as u32), find(&mut up, local(pos[j])));
                    if a != b {
                        up[a as usize] = b;
                        mass[b as usize] += mass[a as usize];
                        moment[b as usize] += moment[a as usize];
                        side[b as usize] += side[a as usize];
                        side_moment[b as usize] += side_moment[a as usize];
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
                let cells: Vec<usize> = fibres[a..b].iter().map(|&(_, k)| cell(k as usize)).collect();
                let r = root as usize;
                let (load, pivot) = self.section_load(&cells, (mass[r], moment[r]), (side[r], side_moment[r]));
                let shown = (load.clamp(0.0, 2.0) * 127.5) as u8;
                for &(_, k) in &fibres[a..b] {
                    strain[k as usize] = shown;
                }
                if load > 1.0 {
                    cracks.push(Crack { load, level: d, fibres: cells, pivot });
                }
                a = b;
            }
            hi = lo;
        }
        BodyStress { strain, cracks }
    }

    /// Whether the body `cells` (`own` tells its cells) stands on firm ground under it with its
    /// centre of mass over the span of what it stands on.
    fn rests_on_its_base(&self, cells: &[usize], own: impl Fn(usize) -> bool) -> bool {
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        let mut base = 0;
        let (mut mass, mut mx) = (0.0f32, 0.0f32);
        for &i in cells {
            let rho = density(self.cells[i].m);
            mass += rho;
            mx += rho * ((i % W()) as f32 + 0.5);
            let t = i + W();
            if t >= W() * H() || own(t) {
                continue;
            }
            if self.mask[t] != 0 || matches!(self.cells[t].m.kind(), Kind::Static | Kind::Powder) {
                let x = (i % W()) as i32;
                lo = lo.min(x);
                hi = hi.max(x);
                base = base.max(i / W() + 1);
            }
        }
        let cx = mx / mass.max(1e-6);
        lo <= hi && cx >= lo as f32 && cx <= hi as f32 + 1.0 && self.pushed_over(cells, (lo, hi), base).is_none()
    }

    /// Whether the push of the liquids on `cells`, standing on the span `lo..=hi` of their base
    /// (whose underside is row `base`), turns them over the edge of that span downstream: the
    /// moment of the push about the edge beats the moment of the weight that holds them. The edge
    /// it turns about if so.
    fn pushed_over(&self, cells: &[usize], (lo, hi): (i32, i32), base: usize) -> Option<(f32, f32)> {
        let pushes: Vec<(usize, (f32, f32))> = cells.iter().map(|&i| (i, self.liquid_push(i))).collect();
        let side: f32 = pushes.iter().map(|p| p.1 .0).sum();
        if side == 0.0 {
            return None;
        }
        let edge = if side > 0.0 { hi as f32 + 1.0 } else { lo as f32 };
        let (mut over, mut hold) = (0.0f32, 0.0f32);
        for &(i, (fx, fy)) in &pushes {
            let (x, y) = { let (x, y) = xy(i); (x as f32 + 0.5, y as f32 + 0.5) };
            over += fx.abs() * (base as f32 - y) * if (fx > 0.0) == (side > 0.0) { 1.0 } else { -1.0 };
            hold += (density(self.cells[i].m) + fy) * (edge - x) * side.signum();
        }
        (over > hold).then_some((edge, base as f32))
    }

    /// How loaded the section through `cells` is, as a share of what it can bear (over 1: it
    /// cracks), given what all it carries weighs down, Σ w and Σ w·x (its mass, and the push of
    /// liquids on it), and pushes sideways, Σ f and Σ f·y. Also the fibre it would turn about.
    #[cfg(test)]
    fn section_load(&self, cells: &[usize], (mass, moment): (f64, f64), (side, side_moment): (f64, f64)) -> (f32, usize) {
        let m = self.cells[cells[0]].m;
        if m.is_vegetation() || (mass <= 0.0 && side == 0.0) {
            return (0.0, cells[0]);
        }
        let sigma = strength(m) as f64 * 1.0e6;
        let xy = |i: usize| ((i % W()) as f64 + 0.5, (i / W()) as f64 + 0.5);
        let (mut worst, mut pivot) = (0.0f64, cells[0]);
        for dir in [-1.0f64, 1.0] {
            // The edge fibre on that side, lowest first: the part beyond would turn about it.
            let p = *cells.iter().max_by(|&&a, &&b| {
                let (ax, ay) = xy(a);
                let (bx, by) = xy(b);
                (dir * ax + ay).total_cmp(&(dir * bx + by))
            }).unwrap();
            let (px, py) = xy(p);
            // Moment of the weight (and of the liquids' push) about the edge (N·m per metre of
            // depth, over h³)...
            let turning = G as f64 * cell_m() as f64 * 1000.0 * dir * ((moment - mass * px) - (side_moment - side * py));
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
        self.bodies.clear();
        // From the bottom up: a body holds what rests on it only once it is held itself.
        let mut free = free;
        free.sort_by_cached_key(|p| std::cmp::Reverse(p.cells.iter().copied().max().unwrap_or(0)));
        for mut p in free {
            p.cells.sort_unstable_by(|a, b| b.cmp(a));
            if self.piece_can_move(&p, 1) {
                if self.tumbles(&p) {
                    // Through the air it falls as a rigid body: it lands on a corner, turns, rolls.
                    self.loosen(p.cells, None, (0.0, p.vy / 2.0), 0.0);
                } else {
                    self.falling.push(p);
                }
                continue;
            }
            let in_piece = |i: usize| p.cells.binary_search_by(|q| i.cmp(q)).is_ok();
            let (mut lo, mut hi) = ((i32::MAX, 0), (i32::MIN, 0));
            let mut firm = false;
            for &i in &p.cells {
                let t = i + W();
                if t >= W() * H() || in_piece(t) {
                    continue;
                }
                // Only what a chain of matter joins to the base of the world holds it up.
                if !self.ground[t] {
                    continue;
                }
                let under = self.cells[t].m;
                if self.mask[t] == 0 {
                    // Firm ground holds it up; a liquid buoys it (it floats, see `drop_structures`):
                    // on firm ground, a long arm reaching out over the liquid tips it over all the same.
                    // Leaves and twigs give way under it (see `crush_foliage`): they hold nothing up.
                    if matches!(under, Mat::Leaf | Mat::Branch | Mat::Sapling) {
                        continue;
                    }
                    match under.kind() {
                        Kind::Static | Kind::Powder => firm = true,
                        Kind::Liquid => {}
                        _ => continue,
                    }
                } else {
                    firm = true;
                }
                let (x, y) = xy_i32(i);
                if x < lo.0 || (x == lo.0 && y > lo.1) {
                    lo = (x, y);
                }
                if x > hi.0 || (x == hi.0 && y > hi.1) {
                    hi = (x, y);
                }
            }
            if lo.0 == i32::MAX {
                self.falling.push(p);
                continue;
            }
            let (mass, mx, my) = p.cells.iter().fold((0.0f32, 0.0f32, 0.0f32), |(m, mx, my), &i| {
                let rho = density(self.cells[i].m);
                (m + rho, mx + rho * ((i % W()) as f32 + 0.5), my + rho * ((i / W()) as f32 + 0.5))
            });
            let (cx, cy) = (mx / mass.max(1e-6), my / mass.max(1e-6));
            let base = p.cells.iter().map(|&i| i / W() + 1).max().unwrap_or(0);
            // (Only a body standing on firm ground turns over an edge of it. One that floats has
            // no edge to turn about: the liquid bears it wherever it dips in, and it rights itself
            // as it bobs (see `drop_structures`). A log landing on the waves a rock had raised
            // tipped over the edge of the crest it touched, and sank as a slab.)
            let pushed = if firm { self.pushed_over(&p.cells, (lo.0, hi.0), base) } else { None };
            let hinge = if let Some(edge) = pushed {
                // Pushed over by the liquid against it (a wall of a dam that is too thin).
                Some(edge)
            } else if !firm {
                None
            } else if cx < lo.0 as f32 {
                Some((lo.0 as f32, lo.1 as f32 + 1.0))
            } else if cx > hi.0 as f32 + 1.0 {
                Some((hi.0 as f32 + 1.0, hi.1 as f32 + 1.0))
            } else {
                None
            };
            // For the stress view: what holds it and what turns it.
            let w = mass * G * cell_m() * cell_m();
            let push = p.cells.iter().fold((0.0f32, 0.0f32), |a, &i| {
                let f = self.liquid_push(i);
                (a.0 + f.0, a.1 + f.1)
            });
            let span = (lo.0 as f32, hi.0 as f32 + 1.0);
            // The moment of the weight about the edge it turns over (positive: it tips), or how
            // far it is from tipping (negative: the moment that holds it, about the nearer edge).
            let moment = match hinge {
                Some(h) => w * (cx - h.0).abs() * cell_m(),
                None => -w * (cx - span.0).min(span.1 - cx) * cell_m(),
            };
            let hooked = hinge.is_some_and(|h| cy > h.1);
            let state = if hinge.is_none() {
                Balance::Rests
            } else if hooked {
                Balance::Hangs
            } else if tips >= MAX_TIPS || p.cells.len() > MAX_SLAB {
                Balance::Waits
            } else {
                Balance::Tips
            };
            if self.bodies.len() < MAX_SHOWN {
                self.bodies.push(FreeBody { c: (cx, cy), cells: p.cells.len(), weight: w, push: (push.0 * super::units::cell_of_water_kn(), push.1 * super::units::cell_of_water_kn()), span, base: base as f32, hinge, moment, state });
            }
            if state != Balance::Tips && firm {
                self.spread_ground(false, &p.cells);
            }
            match state {
                Balance::Tips => {
                    self.loosen(p.cells, hinge, (0.0, 0.0), 0.0);
                    tips += 1;
                }
                // Hooked by a part above its centre of mass, it hangs there instead.
                Balance::Hangs => {}
                Balance::Rests | Balance::Waits => {
                    if !firm {
                        self.falling.push(p);
                    }
                }
            }
        }
    }

    /// Whether the falling body `p` falls as a slab that can turn and roll (see `step_slab`)
    /// rather than straight down: through open air, and not too large.
    fn tumbles(&self, p: &Piece) -> bool {
        if !(MIN_TUMBLER..=MAX_TUMBLER).contains(&p.cells.len()) || self.slabs.len() >= MAX_FLYING {
            return false;
        }
        let in_piece = |i: usize| p.cells.binary_search_by(|q| i.cmp(q)).is_ok();
        p.cells.iter().all(|&i| {
            let t = i + W();
            t < W() * H() && (in_piece(t) || (self.mask[t] == 0 && free_for_slab(self.cells[t].m)))
        })
    }

    /// Turns the cells of a body into a slab turning about `hinge`, or flying at `v` (cells per
    /// step) and turning at `spin`.
    fn loosen(&mut self, cells: Vec<usize>, hinge: Option<(f32, f32)>, v: (f32, f32), spin: f32) {
        let mut parts = Vec::with_capacity(cells.len());
        let mut home = HashMap::with_capacity(cells.len());
        let (mut mass, mut mx, mut my) = (0.0f32, 0.0f32, 0.0f32);
        let mut drawn = Vec::with_capacity(cells.len());
        for (k, &i) in cells.iter().enumerate() {
            let (x, y) = xy_i32(i);
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
        self.slabs.push(Slab { parts, home, c0, c: c0, v, angle: 0.0, spin, mass, inertia, reach: reach + 1.0, hinge, drawn, crushed: vec![], rest: 0, contact: 0 });
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
            s.spin += fall() * s.mass * r.0 / (s.inertia + s.mass * r2);
            // The edge only pushes: once gravity alone no longer bends the path of the centre of
            // mass round it (ω²r > g·cos), the body leaves it and flies.
            let rl = r2.sqrt().max(1e-3);
            if s.spin * s.spin * rl > -fall() * r.1 / rl {
                s.v = s.velocity_at(s.c);
                s.hinge = None;
            }
        } else {
            s.v.1 += fall();
        }
        let lever = match s.hinge {
            Some(h) => ((s.c.0 - h.0).powi(2) + (s.c.1 - h.1).powi(2)).sqrt() + s.reach,
            None => s.reach,
        };
        let fastest = (s.v.0 * s.v.0 + s.v.1 * s.v.1).sqrt() * s.hinge.is_none() as u8 as f32 + s.spin.abs() * lever;
        let n = fastest.ceil().clamp(1.0, 8.0) as i32;
        let mut touched = false;
        let mut slipping = false;
        let mut rehinged = 0;
        for _ in 0..n {
            let (c, angle) = s.pose_after(1.0 / n as f32);
            let hit = match self.place(s, c, angle, 0.0) {
                Ok(()) => {
                    s.c = c;
                    s.angle = angle;
                    continue;
                }
                Err(hit) => hit,
            };
            if hit.soft {
                // Into a liquid: it floats or sinks as a plain falling piece.
                self.land(s, true);
                return false;
            }
            // Tipping over onto the ground just beyond its edge, further towards its centre of
            // mass: that is its edge now, and it goes on turning about it (a body sunk in sand or
            // with a rounded foot rolls over it).
            if let Some(h) = s.hinge {
                if rehinged < 4 {
                    if let Some(p) = Self::next_edge(s, &hit, h) {
                        let (a, b) = ((s.c.0 - h.0, s.c.1 - h.1), (s.c.0 - p.0, s.c.1 - p.1));
                        // Its angular momentum about the new edge is kept.
                        s.spin *= (s.inertia + s.mass * (a.0 * b.0 + a.1 * b.1)) / (s.inertia + s.mass * (b.0 * b.0 + b.1 * b.1));
                        s.hinge = Some(p);
                        rehinged += 1;
                        continue;
                    }
                }
            }
            // It hit something firm: from now on it is a free body, and the hit pushes it back
            // at the point of contact, which turns it about its centre of mass.
            if s.hinge.is_some() {
                s.v = s.velocity_at(s.c);
                s.hinge = None;
            }
            touched = true;
            match self.collide(s, &hit) {
                None => return false,
                Some(slip) => slipping |= slip,
            }
            // What is left of the step, with the new velocity: it slides or rolls on along the
            // surface. Should that run into something too, it waits for the next step.
            let (c, angle) = s.pose_after(1.0 / n as f32);
            match self.place(s, c, angle, 0.3) {
                Ok(()) => {
                    s.c = c;
                    s.angle = angle;
                }
                Err(_) => break,
            }
        }
        if s.drawn.is_empty() {
            return false;
        }
        if touched {
            s.contact = s.contact.saturating_add(1);
            let fastest = (s.v.0 * s.v.0 + s.v.1 * s.v.1).sqrt() + s.spin.abs() * s.reach;
            s.rest = if fastest < SETTLE && !slipping { s.rest + 1 } else { 0 };
            if s.rest >= REST_STEPS || s.contact >= MAX_CONTACT {
                // At rest: its cells stay where they are, and the statics take it from there (if
                // its centre of mass is not over what it rests on, it tips over again). Nearly
                // square to the grid, it settles square (a turned body loses its straight edges).
                let square = (s.angle / FRAC_PI_2).round() * FRAC_PI_2;
                if (s.angle - square).abs() < 0.06 && self.place(s, s.c, square, 0.0).is_ok() {
                    s.angle = square;
                }
                self.land(s, false);
                return false;
            }
        } else {
            s.rest = 0;
        }
        true
    }

    /// The edge a body turning about `h` turns about next, having come down on `hit`: the point
    /// it touched furthest over towards its centre of mass, if all it touched lies under it and
    /// its centre of mass is still beyond that point (else it comes to rest on it).
    fn next_edge(s: &Slab, hit: &Hit, h: (f32, f32)) -> Option<(f32, f32)> {
        let side = (s.c.0 - h.0).signum();
        if hit.points.is_empty() || hit.points.iter().any(|&(p, n)| n.1 > -0.3 || p.1 < s.c.1) {
            return None;
        }
        let p = hit.points.iter().map(|&(p, _)| p).max_by(|a, b| ((a.0 - h.0) * side).total_cmp(&((b.0 - h.0) * side)))?;
        let p = (p.0 + 0.5 * side, p.1 - 0.5);
        ((p.0 - h.0) * side > 0.0 && (s.c.0 - p.0) * side > 0.0).then_some(p)
    }

    /// A body hit something firm (`hit`). If the blow is hard enough it breaks (see
    /// `fracture`; None); otherwise the impulses of the contacts change its velocity and its
    /// spin, as for any rigid body: along the normal they push it back, so that no point of
    /// contact goes on into the surface, and bounce it as much as its matter bounces; friction,
    /// at most μ times that, brakes the points of contact along the surface. A body touching on
    /// a corner off its centre of mass is turned by it. Returns whether it slips (friction could
    /// not hold it).
    fn collide(&mut self, s: &mut Slab, hit: &Hit) -> Option<bool> {
        let ms = ms_per_cell_step();
        let cross = |a: (f32, f32), b: (f32, f32)| a.0 * b.1 - a.1 * b.0;
        let dot = |a: (f32, f32), b: (f32, f32)| a.0 * b.0 + a.1 * b.1;
        // The hardest blow, and where.
        let (mut worst, mut at, mut n) = (0.0f32, hit.points[0].0, hit.points[0].1);
        for &(p, pn) in &hit.points {
            let vn = -dot(s.velocity_at(p), pn);
            if vn > worst {
                (worst, at, n) = (vn, p, pn);
            }
        }
        if worst * ms > 2.0 && self.thuds.len() < 8 {
            self.thuds.push((at.0 as i32, at.1 as i32, (worst * ms / 3.0).min(4.0)));
        }
        let before = (s.v, s.spin);
        // A slow touch does not bounce (else a body at rest would hop under its own weight).
        let e = if worst > 3.0 * fall() { (restitution(s.parts[0].cell.m) + restitution(hit.m)) / 2.0 } else { 0.0 };
        struct Contact {
            r: (f32, f32),
            n: (f32, f32),
            t: (f32, f32),
            /// Speed along the normal it must leave with, and the impulses so far.
            goal: f32,
            jn: f32,
            jt: f32,
        }
        let mut contacts: Vec<Contact> = hit.points.iter().map(|&(p, n)| {
            let vn = dot(s.velocity_at(p), n);
            Contact { r: (p.0 - s.c.0, p.1 - s.c.1), n, t: (-n.1, n.0), goal: (-e * vn).max(0.0), jn: 0.0, jt: 0.0 }
        }).collect();
        let push = |s: &mut Slab, r: (f32, f32), j: (f32, f32)| {
            s.v.0 += j.0 / s.mass;
            s.v.1 += j.1 / s.mass;
            s.spin += cross(r, j) / s.inertia;
        };
        let at = |s: &Slab, r: (f32, f32)| (s.v.0 - s.spin * r.1, s.v.1 + s.spin * r.0);
        // Impulses one contact after the other, again and again: each one only ever pushes (its
        // total stays ≥ 0), and its friction stays within the cone.
        for _ in 0..8 {
            for k in &mut contacts {
                let rn = cross(k.r, k.n);
                let dj = (k.goal - dot(at(s, k.r), k.n)) / (1.0 / s.mass + rn * rn / s.inertia);
                let jn = (k.jn + dj).max(0.0);
                let dj = jn - k.jn;
                k.jn = jn;
                push(s, k.r, (dj * k.n.0, dj * k.n.1));
                let rt = cross(k.r, k.t);
                let dj = -dot(at(s, k.r), k.t) / (1.0 / s.mass + rt * rt / s.inertia);
                let limit = FRICTION * k.jn;
                let jt = (k.jt + dj).clamp(-limit, limit);
                let dj = jt - k.jt;
                k.jt = jt;
                push(s, k.r, (dj * k.t.0, dj * k.t.1));
            }
        }
        // How hard each part was hit: the jolt the contacts gave it (the change of its velocity,
        // as the speed it brought into them). The parts near the blow take it all; those far
        // off on a long arm turning about the point of contact hardly feel it.
        if worst * ms > 0.3 && s.parts.iter().any(|p| rubble(p.cell.m).is_some()) {
            let cells: Vec<usize> = s.drawn.iter().map(|d| d.0).collect();
            let (dv, dw) = ((s.v.0 - before.0 .0, s.v.1 - before.0 .1), s.spin - before.1);
            let vel: Vec<(f32, f32)> = cells.iter().map(|&i| {
                let r = ((i % W()) as f32 + 0.5 - s.c.0, (i / W()) as f32 + 0.5 - s.c.1);
                ((dv.0 - dw * r.1) * ms / (1.0 + e), (dv.1 + dw * r.0) * ms / (1.0 + e))
            }).collect();
            if let Some(pieces) = self.fracture(&cells, &vel) {
                let drawn: HashSet<u32> = s.drawn.iter().map(|d| d.1).collect();
                self.fill_in(s, &drawn);
                self.fly_apart(s, pieces, n);
                return None;
            }
        }
        let slipping = contacts.iter().any(|k| {
            let vt = dot(at(s, k.r), k.t);
            k.jn > 0.0 && vt.abs() > 0.01
        });
        Some(slipping)
    }

    /// Draws the body at a new pose, unless that runs into something: then Err, with what it ran
    /// into.
    /// Parts that would press into something while their point barely moves (`slack`, cells per
    /// step: a body rolling over the point it rests on, which the grid cannot quite follow) are
    /// left out for now instead of stopping it; they come back once there is room.
    fn place(&mut self, s: &mut Slab, c: (f32, f32), angle: f32, slack: f32) -> Result<(), Hit> {
        let (sn, cs) = angle.sin_cos();
        // (Sorted, not hashed: the cells are then always looked at in the same order, and the
        // same throw lands the same way every time.)
        let mut around: Vec<usize> = Vec::with_capacity(s.parts.len() * 3);
        let mut walls = vec![];
        for p in &s.parts {
            let d = (p.x as f32 + 0.5 - s.c0.0, p.y as f32 + 0.5 - s.c0.1);
            let (x, y) = ((c.0 + d.0 * cs - d.1 * sn).floor() as i32, (c.1 + d.0 * sn + d.1 * cs).floor() as i32);
            // The base of the world is ground and its sides are walls (the sky is open).
            if y >= H() as i32 {
                walls.push(((x as f32 + 0.5, H() as f32), (0.0, -1.0)));
            } else if x < 0 && y >= 0 {
                walls.push(((0.0, y as f32 + 0.5), (1.0, 0.0)));
            } else if x >= W() as i32 && y >= 0 {
                walls.push(((W() as f32, y as f32 + 0.5), (-1.0, 0.0)));
            }
            for (tx, ty) in [(x, y), (x + 1, y), (x, y + 1)] {
                if Self::inb(tx, ty) {
                    around.push(Self::idx(tx, ty));
                }
            }
        }
        around.sort_unstable();
        around.dedup();
        let own: HashSet<usize> = s.drawn.iter().map(|d| d.0).collect();
        let mut drawn = Vec::with_capacity(s.parts.len());
        let mut crushed = vec![];
        let mut soft = true;
        let mut hits: Vec<usize> = vec![];
        // Which part of the body covers the centre of cell (x, y), if any.
        let covers = |x: i32, y: i32| {
            let q = (x as f32 + 0.5 - c.0, y as f32 + 0.5 - c.1);
            s.home.get(&((s.c0.0 + q.0 * cs + q.1 * sn).floor() as i32, (s.c0.1 - q.0 * sn + q.1 * cs).floor() as i32)).copied()
        };
        for &t in &around {
            let (tx, ty) = ((t % W()) as i32, (t / W()) as i32);
            let Some(k) = covers(tx, ty) else { continue };
            // Air gives way; while it turns slowly on its edge, so do water and loose grains
            // (they are pushed aside). In flight, water stops it, and grains only where the body
            // comes down on them: those beside it are pushed aside (a body sunk in sand is not
            // held fast by the sand around it).
            let m = self.cells[t].m;
            // Leaves and twigs give way under it as well (see `crush_foliage`).
            let yields = free_for_slab(m)
                || matches!(m, Mat::Leaf | Mat::Branch | Mat::Sapling)
                || (s.hinge.is_some() && matches!(m.kind(), Kind::Liquid | Kind::Powder))
                || (m.kind() == Kind::Powder && covers(tx, ty - 1).is_none());
            if own.contains(&t) || (self.mask[t] == 0 && yields) {
                drawn.push((t, k));
                continue;
            }
            // Against its own foot while it still turns on it: that corner is crushed, not stopped.
            if let Some(h) = s.hinge {
                let p = { let (x, y) = xy(t); (x as f32 + 0.5, y as f32 + 0.5) };
                if (p.0 - h.0).powi(2) + (p.1 - h.1).powi(2) < 6.25 {
                    crushed.push((k, p));
                    continue;
                }
            } else if slack > 0.0 {
                let v = s.velocity_at((tx as f32 + 0.5, ty as f32 + 0.5));
                if v.0 * v.0 + v.1 * v.1 < slack * slack {
                    continue;
                }
            }
            hits.push(t);
            if self.mask[t] != 0 || self.cells[t].m.kind() != Kind::Liquid {
                soft = false;
            }
        }
        if !hits.is_empty() || !walls.is_empty() {
            let mut hit = if hits.is_empty() { Hit { soft: false, points: vec![], m: Mat::Stone } } else { self.hit(&hits, &own, soft, s.v) };
            hit.soft &= walls.is_empty();
            let every = walls.len().div_ceil(MAX_POINTS).max(1);
            hit.points.extend(walls.into_iter().step_by(every));
            return Err(hit);
        }
        // What the body leaves is filled with the air (or smoke, or flames) it pushes aside.
        let now: HashSet<usize> = drawn.iter().map(|d| d.0).collect();
        let mut pushed: Vec<(Cell, f32, (States, u16), usize)> = drawn.iter().filter(|d| !own.contains(&d.0) && self.cells[d.0].m != Mat::Empty).map(|d| (self.cells[d.0], self.temp[d.0], self.take_states(d.0), d.0)).collect();
        pushed.sort_by_key(|p| matches!(p.0.m.kind(), Kind::Liquid | Kind::Powder | Kind::Static));
        let frame = self.frame;
        // The liquid moved into the room the body leaves: where it is now, where it was.
        let mut wet = vec![];
        for &(i, _) in &s.drawn {
            if now.contains(&i) {
                continue;
            }
            // Pushed aside into the room the body leaves (grains and water first: they matter).
            match pushed.pop() {
                Some((cell, t, st, from)) => {
                    if cell.m.kind() == Kind::Liquid {
                        wet.push((i, from));
                    }
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
        for (cell, _, _, _) in pushed {
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
        // The room it left is above the liquid where it comes out of it: the liquid it drove
        // aside goes to the surface round it instead (see `wade`).
        if !wet.is_empty() {
            let v = s.velocity_at(c);
            let speed = (v.0 * v.0 + v.1 * v.1).sqrt() * ms_per_cell_step();
            self.wade(&|j| now.contains(&j), &wet, c.0, speed);
        }
        Ok(())
    }

    /// The body's flight is over: at rest, its cells stay where they are; in a liquid
    /// (`soft`) it goes on as a plain falling piece (it floats or sinks).
    fn land(&mut self, s: &Slab, soft: bool) {
        // What was crushed at its foot crumbles there.
        let drawn: HashSet<u32> = s.drawn.iter().map(|d| d.1).collect();
        for &(k, p) in &s.crushed {
            let m = s.parts[k as usize].cell.m;
            if !drawn.contains(&k) && self.ejecta.len() < 14_000 {
                self.ejecta.push(Ejecta { x: p.0, y: p.1 - 1.5, vx: 0.0, vy: -0.2, m: rubble(m).unwrap_or(m) });
            }
        }
        if s.drawn.is_empty() {
            return;
        }
        if !soft {
            self.fill_in(s, &drawn);
            return;
        }
        let cells: Vec<usize> = s.drawn.iter().map(|d| d.0).collect();
        let density = cells.iter().map(|&i| density(self.cells[i].m)).sum::<f32>() / cells.len() as f32;
        let vy = (s.velocity_at(s.c).1 * 2.0).clamp(0.0, 3.0);
        self.falling.push(Piece { cells, vy, fallen: 0.0, density });
    }

    /// A turned body covers the grid with a cell or two fewer than it has (two parts fall on the
    /// same cell): at rest, those parts take the free cell nearest where they would be.
    fn fill_in(&mut self, s: &Slab, drawn: &HashSet<u32>) {
        let crushed: HashSet<u32> = s.crushed.iter().map(|c| c.0).collect();
        let (sn, cs) = s.angle.sin_cos();
        let frame = self.frame;
        for (k, p) in s.parts.iter().enumerate() {
            let k = k as u32;
            if drawn.contains(&k) || crushed.contains(&k) {
                continue;
            }
            let d = (p.x as f32 + 0.5 - s.c0.0, p.y as f32 + 0.5 - s.c0.1);
            let (x, y) = ((s.c.0 + d.0 * cs - d.1 * sn).floor() as i32, (s.c.1 + d.0 * sn + d.1 * cs).floor() as i32);
            let free = |w: &World, (x, y): (i32, i32)| Self::inb(x, y) && w.mask[Self::idx(x, y)] == 0 && free_for_slab(w.cells[Self::idx(x, y)].m);
            let Some(t) = (0..=3).flat_map(|r| (-r..=r).flat_map(move |dy| (-r..=r).map(move |dx| (x + dx, y + dy)))).find(|&c| free(self, c)) else { continue };
            let i = Self::idx(t.0, t.1);
            self.cells[i] = Cell { stamp: frame, ..p.cell };
            self.temp[i] = p.temp;
            self.put_states(i, p.states);
            self.zones.wake(i);
        }
    }

    /// Where the body touches the cells `hits` it ran into (`own`: the cells it covers now),
    /// and which way the surface faces there: away from the firm cells around each of them.
    fn hit(&self, hits: &[usize], own: &HashSet<usize>, soft: bool, v: (f32, f32)) -> Hit {
        let firm = |u: usize| !own.contains(&u) && (self.mask[u] != 0 || !free_for_slab(self.cells[u].m));
        let m = hits.iter().map(|&t| self.cells[t].m).find(|m| m.kind() == Kind::Static).unwrap_or(self.cells[hits[0]].m);
        let every = hits.len().div_ceil(MAX_POINTS);
        let points = hits.iter().step_by(every).map(|&t| {
            let (x, y) = xy_i32(t);
            let (mut nx, mut ny) = (0.0f32, 0.0f32);
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let (ux, uy) = (x + dx, y + dy);
                    // The edges of the world are walls, its floor is the ground.
                    if !Self::inb(ux, uy) || firm(Self::idx(ux, uy)) {
                        nx -= dx as f32;
                        ny -= dy as f32;
                    }
                }
            }
            let l = (nx * nx + ny * ny).sqrt();
            let n = if l > 1e-3 {
                (nx / l, ny / l)
            } else {
                // Nothing to tell which way it faces (in a crack): straight against the motion.
                let l = (v.0 * v.0 + v.1 * v.1).sqrt().max(1e-6);
                (-v.0 / l, -v.1 / l)
            };
            // On the face of the cell it ran into, towards the body.
            ((x as f32 + 0.5 + 0.5 * n.0, y as f32 + 0.5 + 0.5 * n.1), n)
        }).collect();
        Hit { soft, points, m }
    }

    /// The body broke into `pieces` when it hit a surface facing `n`: they fly on with what is
    /// left of their speed, bounced off it, and those that hit again break further or settle.
    fn fly_apart(&mut self, s: &Slab, pieces: Vec<Vec<usize>>, n: (f32, f32)) {
        for piece in pieces {
            if piece.len() < 3 {
                continue;
            }
            let k = piece.len() as f32;
            let c = piece.iter().fold((0.0, 0.0), |a, &i| (a.0 + (i % W()) as f32 + 0.5, a.1 + (i / W()) as f32 + 0.5));
            let v = s.velocity_at((c.0 / k, c.1 / k));
            let vn = (v.0 * n.0 + v.1 * n.1).min(0.0);
            // The part along the surface is braked, the part into it bounces back a little.
            let v = ((v.0 - vn * n.0) * 0.6 - 0.3 * vn * n.0, (v.1 - vn * n.1) * 0.6 - 0.3 * vn * n.1);
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
            let t = i + W();
            t < W() * H() && !in_piece(t) && (self.mask[t] != 0 || matches!(self.cells[t].m.kind(), Kind::Static | Kind::Powder))
        });
        if !firm {
            return false;
        }
        let v = (2.0 * G * piece.fallen * cell_m()).sqrt();
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
            let (x, y) = xy_i32(i);
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
            let (x, y) = xy_i32(i);
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

    /// A plan run on the loads of the moment gives what the body worked out from scratch gives,
    /// to the bit: water lapping at its banks, blasts that change its shape.
    #[test]
    fn a_plan_gives_what_the_body_worked_out_gives() {
        CHECK_PLANS.store(true, std::sync::atomic::Ordering::Relaxed);
        for (name, make, each) in super::super::bench::scenes() {
            if !matches!(name, "liquides" | "explosions") {
                continue;
            }
            let mut w = make();
            for i in 0..100 {
                if let Some(f) = each {
                    f(&mut w, i);
                }
                w.step();
            }
            assert!(!w.stress_plans.is_empty(), "{name}: the anchored bodies have plans");
        }
    }

    /// Cold, still, a slab of bedrock from row `ground` down.
    fn world(ground: i32) -> World {
        let mut w = World::new();
        w.day_cycle = false;
        w.climate_fixed = Some(-5.0);
        w.reset_temps();
        for x in 0..W() as i32 {
            for y in ground..H() as i32 {
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
        assert!((0..W() * H()).filter(|&i| w.cells[i].m == Mat::Ice).all(|i| i / W() > 400), "nothing is left standing in the air");
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
        let fallen: Vec<usize> = (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Ice && i % W() > 200).collect();
        let mean_y = fallen.iter().map(|i| i / W()).sum::<usize>() / fallen.len();
        assert!(mean_y > 428, "the overhanging block tipped off: its cells are at row {mean_y} on average");
        assert!((0..W() * H()).any(|i| matches!(w.cells[i].m, Mat::Ice | Mat::Hail) && i % W() > 320 && i / W() > 440), "it fell beside the pedestal");
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
        let top = |x: i32| (0..H() as i32).find(|&y| w.raw(x, y) == Mat::Dirt).unwrap_or(450);
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
        let cells: Vec<usize> = (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Ice).collect();
        w.loosen(cells, Some((100.0, 104.0)), (0.0, 0.0), 0.0);
        let s = &w.slabs[0];
        assert!((s.mass - 40.0 * 0.92).abs() < 1e-3);
        assert!((s.c.0 - 105.0).abs() < 1e-3 && (s.c.1 - 102.0).abs() < 1e-3, "{:?}", s.c);
        // A 10 x 4 plate: I = m (a² + b²) / 12.
        let plate = s.mass * (100.0 + 16.0) / 12.0;
        assert!((s.inertia - plate).abs() / plate < 1e-3, "{} vs {plate}", s.inertia);
    }

    /// Kinetic energy of a body, Σ ½mv² + ½Iω².
    fn energy(s: &Slab) -> f32 {
        0.5 * s.mass * (s.v.0 * s.v.0 + s.v.1 * s.v.1) + 0.5 * s.inertia * s.spin * s.spin
    }

    /// Cells of matter `m` in the world.
    fn cells_of(w: &World, m: Mat) -> Vec<(i32, i32)> {
        (0..W() * H()).filter(|&i| w.cells[i].m == m).map(|i| xy_i32(i)).collect()
    }

    #[test]
    fn a_hit_off_the_centre_of_mass_turns_a_body_and_makes_no_energy() {
        let mut w = world(450);
        for x in 100..120 {
            for y in 300..306 {
                w.set(x, y, Mat::Wood);
            }
        }
        let cells: Vec<usize> = (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Wood).collect();
        w.loosen(cells, None, (0.0, 2.0), 0.0);
        let mut s = w.slabs.pop().unwrap();
        let before = energy(&s);
        // It comes down on its right-hand corner, onto flat ground.
        let hit = Hit { soft: false, points: vec![((119.5, 306.0), (0.0, -1.0))], m: Mat::Stone };
        assert!(w.collide(&mut s, &hit).is_some(), "wood does not break");
        assert!(s.v.1 < 2.0, "the ground pushed it back: {:?}", s.v);
        assert!(s.spin < -0.01, "pushed up at its right end, it turns anticlockwise: {}", s.spin);
        assert!(energy(&s) <= before, "a hit loses energy, it never makes any: {} > {before}", energy(&s));
        // Straight under its centre, the same blow does not turn it.
        let mut s2 = { let mut w = world(450); for x in 100..120 { for y in 300..306 { w.set(x, y, Mat::Wood); } } let cells: Vec<usize> = (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Wood).collect(); w.loosen(cells, None, (0.0, 2.0), 0.0); w.slabs.pop().unwrap() };
        let mid = Hit { soft: false, points: vec![((s2.c.0, 306.0), (0.0, -1.0))], m: Mat::Stone };
        w.collide(&mut s2, &mid);
        assert!(s2.spin.abs() < 1e-4, "{}", s2.spin);
        // The momentum it gave the ground is what it lost: Δp = J, along the normal.
        assert!(s2.v.0.abs() < 1e-4 && s2.v.1 <= 0.0, "it bounces straight back: {:?}", s2.v);
    }

    #[test]
    fn a_plank_dropped_onto_the_edge_of_a_ledge_turns_off_it() {
        let mut w = world(450);
        // A pedestal of stone (one body with the ground), and a plank of wood in the air above
        // its right-hand edge, three quarters of it beyond the edge.
        for x in 200..260 {
            for y in 400..450 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 250..290 {
            for y in 340..344 {
                w.set(x, y, Mat::Wood);
            }
        }
        let mut turned = 0.0f32;
        for _ in 0..900 {
            w.step();
            for s in &w.slabs {
                turned = turned.max(s.angle.abs());
            }
        }
        assert!(turned > 0.5, "it turned about the edge as it fell: {turned} rad");
        let wood = cells_of(&w, Mat::Wood);
        assert_eq!(wood.len(), 160, "nothing is lost");
        assert!(wood.iter().all(|c| c.0 >= 255 && c.1 > 400), "it fell off beside the ledge: {:?}", &wood[..4]);
        assert!(w.slabs.is_empty(), "and came to rest");
    }

    #[test]
    fn a_block_thrown_sideways_slides_and_stops() {
        let mut w = world(450);
        // A crate of wood on the ground, pushed along it.
        for x in 100..112 {
            for y in 440..450 {
                w.set(x, y, Mat::Wood);
            }
        }
        let cells: Vec<usize> = (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Wood).collect();
        // (At 3 m/s.)
        w.loosen(cells, None, (super::super::units::cells_per_tick(3.0), 0.0), 0.0);
        run(&mut w, 600);
        assert!(w.slabs.is_empty(), "it came to rest");
        let wood = cells_of(&w, Mat::Wood);
        assert_eq!(wood.len(), 120);
        // Friction brakes it at μg: it slides v² / 2μg, some 17 cells.
        let x = wood.iter().map(|c| c.0).sum::<i32>() as f32 / 120.0 + 0.5;
        assert!(x > 116.0, "it carried on along the ground: centre at {x}");
        assert!(x < 140.0, "but friction stopped it: centre at {x}");
        assert!(wood.iter().any(|c| c.1 == 449), "it lies on the ground");
    }

    #[test]
    fn a_block_falling_on_its_corner_comes_to_rest_on_a_face() {
        let mut w = world(450);
        for x in 300..316 {
            for y in 300..310 {
                w.set(x, y, Mat::Wood);
            }
        }
        let cells: Vec<usize> = (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Wood).collect();
        // Spinning as it falls: it lands on a corner.
        w.loosen(cells, None, (0.0, 0.0), 0.02);
        let mut spun = false;
        for _ in 0..900 {
            w.step();
            spun |= w.slabs.iter().any(|s| s.angle.abs() > 0.3);
        }
        assert!(spun);
        assert!(w.slabs.is_empty(), "it came to rest");
        // Flat on the ground: the bottom row is nearly full, and it stands no higher than its
        // long side.
        let wood = cells_of(&w, Mat::Wood);
        let bottom = wood.iter().filter(|c| c.1 == 449).count();
        let top = wood.iter().map(|c| c.1).min().unwrap();
        assert!(bottom >= 8, "it lies on a face: {bottom} cells on the ground");
        assert!(top >= 449 - 17, "it does not stand on a corner: top at {top}");
    }

    /// A ball of stone on a long arm whose foot stands in a pit of sand: its centre of mass is
    /// far beyond its foot. With `leaves`, a tree stands under the ball, its crown just touching.
    fn ball_on_an_arm(leaves: bool) -> World {
        let mut w = world(470);
        for x in 560..640 {
            for y in 470..480 {
                w.set(x, y, Mat::Sand);
            }
        }
        w.paint(250, 200, 40, Mat::Stone);
        w.line((290, 200), (600, 466), 8, Mat::Stone);
        if leaves {
            for x in 230..270 {
                for y in 241..245 {
                    w.set(x, y, Mat::Leaf);
                }
            }
            // (Held by a branch from a trunk off to the side.)
            for x in 150..230 {
                w.set(x, 244, Mat::Branch);
            }
            for x in 148..150 {
                for y in 244..470 {
                    w.set(x, y, Mat::Trunk);
                }
            }
        }
        w
    }

    #[test]
    fn a_ball_on_an_arm_standing_in_sand_tips_over() {
        for leaves in [false, true] {
            let mut w = ball_on_an_arm(leaves);
            // (The statics run every few steps.)
            while w.bodies.is_empty() {
                w.step();
            }
            let b = w.bodies.iter().find(|b| b.cells > 5000).expect("the ball and its arm are one free body");
            assert!(b.c.0 < 400.0 && b.span.0 > 550.0, "its centre of mass is far beyond its foot: {b:?}");
            assert!(b.moment > 0.0 && matches!(b.state, Balance::Tips), "{b:?}");
            run(&mut w, 300);
            assert_ne!(w.raw(250, 200), Mat::Stone, "the ball came down (leaves under it: {leaves})");
        }
    }

    #[test]
    fn a_block_centred_on_a_pedestal_shows_what_holds_it() {
        let mut w = world(450);
        for x in 100..140 {
            for y in 430..450 {
                w.set(x, y, Mat::Glass);
            }
        }
        for x in 110..130 {
            for y in 420..430 {
                w.set(x, y, Mat::Ice);
            }
        }
        while w.bodies.is_empty() {
            w.step();
        }
        let b = w.bodies.iter().find(|b| b.cells == 200).expect("the block");
        assert_eq!(b.state, Balance::Rests);
        assert!((b.c.0 - 120.0).abs() < 0.01 && (b.c.1 - 425.0).abs() < 0.01, "{b:?}");
        // 200 cells of ice of 5 cm: 0.46 t per metre, 4.5 kN/m; its centre of mass is 10 cells
        // (0.5 m) from either edge of what it rests on.
        assert!((b.weight - 200.0 * 0.92 * 0.0025 * 9.81).abs() < 0.1, "{}", b.weight);
        assert!((b.moment + b.weight * 0.5).abs() < 0.05, "{}", b.moment);
    }

    #[test]
    fn a_vault_painted_in_the_sky_holds_nothing_up() {
        let mut w = world(450);
        // One cell of vault, and a block of glass resting on it: nothing joins them to the ground.
        w.set(300, 300, Mat::Vault);
        for x in 290..310 {
            for y in 290..300 {
                w.set(x, y, Mat::Glass);
            }
        }
        // The same in a tunnel's wall: there it holds.
        for y in 380..450 {
            w.set(500, y, Mat::Vault);
        }
        for x in 490..510 {
            for y in 370..380 {
                w.set(x, y, Mat::Glass);
            }
        }
        run(&mut w, 300);
        assert_ne!(w.raw(300, 295), Mat::Glass, "the block fell");
        assert_eq!(w.raw(500, 375), Mat::Glass, "the vault joined to the ground holds its block");
    }

    #[test]
    fn a_round_stone_rolls_down_a_slope() {
        let mut w = world(450);
        // A slope going down to the right, one cell in three.
        for x in 0..600 {
            for y in (250 + x / 3)..450 {
                w.set(x, y, Mat::Glass);
            }
        }
        // A round boulder of rock resting on it.
        let (cx, cy, r) = (100i32, 274i32, 7i32);
        for x in cx - r..=cx + r {
            for y in cy - r..=cy + r {
                if (x - cx).pow(2) + (y - cy).pow(2) <= r * r && w.raw(x, y) == Mat::Empty {
                    w.set(x, y, Mat::Stone);
                }
            }
        }
        let mut spin = 0.0f32;
        for _ in 0..400 {
            w.step();
            for s in &w.slabs {
                spin = spin.max(s.spin);
            }
        }
        let stone: Vec<(i32, i32)> = cells_of(&w, Mat::Stone).into_iter().filter(|c| c.1 < 449).collect();
        let x = stone.iter().map(|c| c.0).sum::<i32>() as f32 / stone.len().max(1) as f32;
        assert!(spin > 0.02, "it turned clockwise, rolling to the right: {spin}");
        assert!(x > 130.0, "it went down the slope: centre at {x}");
    }
}

#[cfg(test)]
mod snapshots {
    use super::*;

    /// Writes frames of three scenes as text (AWORLD_SNAP=dir), for pictures.
    #[test]
    #[ignore]
    fn snapshot_tumbling() {
        let Ok(dir) = std::env::var("AWORLD_SNAP") else { return };
        let mut w = World::new();
        w.day_cycle = false;
        w.climate_fixed = Some(-5.0);
        w.reset_temps();
        for x in 0..W() as i32 {
            for y in 450..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        // A ledge and a plank above its edge.
        for x in 60..120 {
            for y in 400..450 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 110..150 {
            for y in 340..344 {
                w.set(x, y, Mat::Wood);
            }
        }
        // A crate falling on its corner.
        for x in 300..316 {
            for y in 330..340 {
                w.set(x, y, Mat::Wood);
            }
        }
        let cells: Vec<usize> = (0..W() * H()).filter(|&i| w.cells[i].m == Mat::Wood && i % W() >= 300 && i % W() < 316).collect();
        w.loosen(cells, None, (0.4, 0.0), 0.03);
        // A slope and a round boulder.
        for x in 420..700 {
            for y in (330 + (x - 420) / 3)..450 {
                w.set(x, y, Mat::Glass);
            }
        }
        let (cx, cy, r) = (440i32, 318i32, 7i32);
        for x in cx - r..=cx + r {
            for y in cy - r..=cy + r {
                if (x - cx).pow(2) + (y - cy).pow(2) <= r * r && w.raw(x, y) == Mat::Empty {
                    w.set(x, y, Mat::Stone);
                }
            }
        }
        // Ground stone and boulder stone are told apart by height in the picture.
        for k in 0..=300 {
            if k % 10 == 0 {
                let mut out = String::new();
                for y in 280..460 {
                    for x in 40..720 {
                        out.push(match w.raw(x, y) {
                            Mat::Empty => '.',
                            Mat::Wood => 'w',
                            Mat::Glass => 'g',
                            Mat::Stone => 's',
                            _ => 'o',
                        });
                    }
                    out.push('\n');
                }
                std::fs::write(format!("{dir}/f{k:04}.txt"), out).unwrap();
            }
            w.step();
        }
    }
}
