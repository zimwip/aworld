//! Explosions worked out from an energy budget, the same way for every material.
//!
//! A charge of powder goes off all at once: every grain touching the one that caught fire burns
//! in the same instant, so the blast carries the energy of the whole charge. A share of that
//! chemical energy does work on the ground around it; the rest heats the gas, makes light and
//! noise. That share is sent out along rays from the centre. Along a ray, each cell costs what it
//! takes to tear it loose and throw it out of the hole:
//!
//!   cost = V · (σ + ½ ρ (v² + 2 g h))
//!
//! with V the volume of a cell, σ the strength of the material (how much stress it takes to break
//! it, in pascals: joules per cubic metre), ρ its density, v the speed it must be thrown at to clear
//! the crater and h the height it is lifted. So loose sand only costs its weight and the throw,
//! soil a little more, rock many times more and steel nearly holds. The strongest material does not
//! need a special case: it just eats the ray's energy faster.
//!
//! Matter goes where it is easiest to go: a ray that reaches the open air through little ground
//! (the "line of least resistance" of the quarrymen) gets more of the energy than one that would
//! have to dig through bedrock. Rays that break out throw what they tore loose as projectiles, with
//! the energy they had left; rays that run out of energy underground only crush the ground, which
//! stays where it is: the gas of the charge alone opens a hidden cavity, the "camouflet". Projectiles fly, fall back and pile up as a rim: a real crater.
//!
//! The cells can be anything with a density and a strength, so the same rule shapes a crater in a
//! meadow, a hole in a brick wall or a dent in an iron plate.

use super::num::round_i;
use super::{xy_i32, Kind, Mat, MatSpec, World, H, W};
use rayon::prelude::*;

use super::units::{cell_m, G};
/// Speed a torn-off clod must be thrown at to clear the hole, m/s.
const THROW: f32 = 50.0;
/// An explosive, as the blast sees it.
#[derive(Clone, Copy, Debug)]
pub struct Explosive {
    /// Chemical energy, J/kg.
    pub energy: f32,
    /// Density, kg/m³.
    pub rho: f32,
    /// Share of the energy that does work on the ground (the rest heats the gas, light, noise). A
    /// detonation, whose front outruns sound, couples much better than powder that only burns.
    pub coupling: f32,
    /// Gas it gives, m³ per kg at ambient pressure.
    pub gas: f32,
    /// Peak pressure of its shock at the charge, atm.
    pub shock: f32,
    /// Temperature of its hot gas, °C.
    pub flame: f32,
}

/// What makes `m` explode, if it does.
pub fn explosive(m: Mat) -> Option<Explosive> {
    match m {
        // Black powder deflagrates: it burns very fast but below the speed of sound.
        Mat::Gunpowder => Some(Explosive { energy: 3.0e6, rho: 1500.0, coupling: 0.025, gas: 0.28, shock: 3_000.0, flame: 2_000.0 }),
        // Dynamite detonates (7 km/s): a shock of more than 100 000 atm.
        Mat::Dynamite => Some(Explosive { energy: 5.0e6, rho: 1400.0, coupling: 0.06, gas: 0.7, shock: 120_000.0, flame: 3_500.0 }),
        _ => None,
    }
}

/// A charge that goes off: the sum of its cells of explosive.
#[derive(Clone, Copy, Debug, Default)]
pub struct Charge {
    /// Energy that works on the ground, J.
    pub work: f32,
    /// Gas, in cells of air at ambient pressure.
    pub gas: f32,
    /// Peak pressure at the charge, atm, and temperature of the gas, °C.
    pub shock: f32,
    pub flame: f32,
    /// Cells of explosive.
    pub cells: f32,
}

impl Charge {
    fn add(&mut self, m: Mat) {
        let Some(e) = explosive(m) else { return };
        let mass = e.rho * cell_m().powi(3);
        self.work += e.energy * mass * e.coupling;
        self.gas += e.gas * mass / cell_m().powi(3);
        self.shock = self.shock.max(e.shock);
        self.flame = self.flame.max(e.flame);
        self.cells += 1.0;
    }

    /// As much black powder as `grains` cells of it.
    pub fn powder(grains: f32) -> Charge {
        Charge::of(Mat::Gunpowder, grains)
    }

    /// As much of the explosive `m` as `cells` cells of it.
    pub fn of(m: Mat, cells: f32) -> Charge {
        let mut c = Charge::default();
        c.add(m);
        Charge { work: c.work * cells, gas: c.gas * cells, cells: c.cells * cells, ..c }
    }

    /// Both charges going off as one.
    fn join(self, o: Charge) -> Charge {
        Charge {
            work: self.work + o.work,
            gas: self.gas + o.gas,
            shock: self.shock.max(o.shock),
            flame: self.flame.max(o.flame),
            cells: self.cells + o.cells,
        }
    }

    /// How many cells of black powder would do the same work.
    fn grains(&self) -> f32 {
        self.work / grain_energy()
    }
}
/// The share of the speed its energy gives a torn-off clod that it flies off at: most of the work
/// of a blast goes into crushing and heating the ground, and clods thrown at the whole of it
/// would leave the sky.
const THROWN_SHARE: f32 = 0.16;
/// Fastest projectile, m/s: about 17 m up at most, the depth of the design world.
const MAX_SPEED_MS: f32 = 18.0;
/// No more flying clods than this at once; the rest stay where they were, broken.
const MAX_EJECTA: usize = 14_000;
/// Longest ray, cells.
const MAX_REACH: f32 = 220.0;
const ATM_PER_MPA: f32 = 9.87;
/// How many times its tensile strength a brittle solid holds in compression.
const CRUSH: f32 = 10.0;
/// Radial cracks round a blast.
const CRACKS: usize = 11;

/// Mechanical energy one grain of powder gives the ground, J.
pub fn grain_energy() -> f32 {
    let e = explosive(Mat::Gunpowder).unwrap();
    e.energy * e.rho * cell_m().powi(3) * e.coupling
}

/// Strength of a material, MPa (see `MatSpec::strength`).
pub fn strength(m: Mat) -> f32 {
    MatSpec::of(m).strength
}

/// Density, kg/m³ (air for gas and empty cells).
fn rho(m: Mat) -> f32 {
    MatSpec::of(m).density
}

/// Energy to tear one cell of `m` loose at `d` cells from the centre and throw it out, J.
pub fn cell_cost(m: Mat, d: f32) -> f32 {
    cell_m().powi(3) * (strength(m) * 1.0e6 + 0.5 * rho(m) * (THROW * THROW + 2.0 * G * d * cell_m()))
}

/// Radius the fire, the pressure wave and the shove of a charge of `grains` reach, cells.
pub fn reach(grains: f32) -> i32 {
    (6.0 + 2.2 * grains.max(0.0).sqrt()).min(70.0) as i32
}

/// What a solid turns into when it is torn loose and thrown.
fn thrown(m: Mat, roll: u32) -> Mat {
    match m {
        // Rock flies as boulders and gravel.
        Mat::Stone => if roll % 3 == 0 { Mat::Stone } else { Mat::Sand },
        Mat::Grass => Mat::Dirt,
        Mat::Glass => Mat::Shard,
        Mat::Ice => if roll % 2 == 0 { Mat::Ice } else { Mat::Hail },
        Mat::Brick | Mat::Terracotta | Mat::Vault => if roll % 2 == 0 { m } else { Mat::Sand },
        Mat::Leaf | Mat::Sapling => Mat::DeadLeaf,
        Mat::Wood | Mat::Trunk | Mat::Branch => if roll % 3 == 0 { Mat::Ember } else { Mat::Wood },
        _ => m,
    }
}

/// What a crushed (or cracked) solid leaves, if anything: its rubble (`MatSpec::rubble`), and
/// the splinters of wood the blast sets alight. Clay is left as it is: the blast packs the soil
/// round the hole into it (`a_detonation_packs_the_soil_left_around_the_hole`), it does not
/// loosen it again.
fn rubble(m: Mat) -> Option<Mat> {
    match m {
        Mat::Clay => None,
        Mat::Wood | Mat::Trunk | Mat::Branch => Some(Mat::Ember),
        _ => MatSpec::of(m).rubble,
    }
}

/// A clod of matter in flight, thrown out of a crater.
#[derive(Clone, Copy, Debug)]
pub struct Ejecta {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub m: Mat,
}

/// What a blast does to one cell.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Fate {
    /// Cracked at the edge of the hole.
    Crack,
    /// Torn loose but with nowhere to go: it stays, crushed into the walls of a hidden cavity.
    Crush,
    /// Thrown out of the crater at this speed (cells per step).
    Throw(f32, f32),
}

/// The fastest a clod thrown by a blast falls, m/s: a lump of earth or rock a few centimetres
/// across (see `ballistic::pull`).
const CLOD_TERMINAL_MS: f32 = 30.0;

fn passable(m: Mat) -> bool {
    matches!(m.kind(), Kind::Empty | Kind::Gas | Kind::Flame)
}

impl World {
    /// Explosive at (x, y) caught fire: the whole charge it belongs to goes off together, at its
    /// centre. A long thin trail of powder goes off in pieces, like a fuse.
    pub(super) fn set_off_powder(&mut self, x: i32, y: i32) {
        // The charge and its blast reach far: they go off after the other cells.
        if self.put_off(super::turn::Later::Powder(x, y)) {
            return;
        }
        let charge = self.charge_from(&[(x, y)]);
        let n = charge.len() as f32;
        let (sx, sy) = charge.iter().fold((0.0, 0.0), |a, &(x, y)| (a.0 + x as f32, a.1 + y as f32));
        let mut sum = Charge::default();
        for &(x, y) in &charge {
            sum.add(self.raw(x, y));
            self.set(x, y, Mat::Empty);
        }
        let (cx, cy) = ((sx / n).round() as i32, (sy / n).round() as i32);
        self.boom(cx, cy, charge.len());
        self.detonate(cx, cy, sum);
    }

    /// Sticks of dynamite go off together: `sticks` cells of dynamite centred on (cx, cy), lying
    /// over the matter of `room` without having taken its place. The blast finds the matter there
    /// and breaks it like the rest; only the gas of the charge opens a room the size of the sticks.
    /// Powder or dynamite touching the sticks goes off with them, as part of the same charge.
    pub fn set_off_sticks(&mut self, cx: f32, cy: f32, sticks: f32, room: &[(i32, i32)]) {
        let seeds: Vec<(i32, i32)> = room
            .iter()
            .flat_map(|&(x, y)| [(x, y), (x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)])
            .filter(|&(x, y)| Self::inb(x, y) && explosive(self.raw(x, y)).is_some() && self.mask_free(x, y))
            .collect();
        let near = if seeds.is_empty() { vec![] } else { self.charge_from(&seeds) };
        let mut sum = Charge::of(Mat::Dynamite, sticks);
        let (mut sx, mut sy) = (cx * sticks, cy * sticks);
        for &(x, y) in &near {
            let mut c = Charge::default();
            c.add(self.raw(x, y));
            sum = sum.join(c);
            sx += x as f32;
            sy += y as f32;
            self.set(x, y, Mat::Empty);
        }
        let n = sticks + near.len() as f32;
        let (cx, cy) = ((sx / n).round() as i32, (sy / n).round() as i32);
        for &(x, y) in room {
            if Self::inb(x, y) && self.mask_free(x, y) && !passable(self.raw(x, y)) && self.raw(x, y).kind() != Kind::Liquid {
                self.set(x, y, Mat::Empty);
            }
        }
        self.boom(cx, cy, n as usize);
        self.detonate(cx, cy, sum);
    }

    /// One report per charge, a few more for a big one (more rockets at night).
    fn boom(&mut self, cx: i32, cy: i32, cells: usize) {
        for _ in 0..1 + (cells / 150).min(3) {
            if self.powder_booms.len() < 16 {
                self.powder_booms.push((cx, cy));
            }
        }
    }

    /// The cells of explosive that burn with those of `seeds`: a lump at once, a trail only so far.
    fn charge_from(&self, seeds: &[(i32, i32)]) -> Vec<(i32, i32)> {
        let mut taken = std::collections::HashSet::new();
        let mut charge: Vec<(i32, i32)> = seeds.iter().copied().filter(|&c| taken.insert(c)).collect();
        let mut front = charge.clone();
        let mut layer = 0;
        while !front.is_empty() && charge.len() < 40_000 {
            layer += 1;
            // A lump burns through at once; along a trail the flame only gets so far at a time.
            if layer as f32 > 2.0 * (charge.len() as f32).sqrt() + 6.0 {
                break;
            }
            let mut next = vec![];
            for (px, py) in front {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let q = (px + dx, py + dy);
                        if explosive(self.raw(q.0, q.1)).is_some() && self.mask_free(q.0, q.1) && taken.insert(q) {
                            next.push(q);
                        }
                    }
                }
            }
            charge.extend_from_slice(&next);
            front = next;
        }
        charge
    }

    /// The charge goes off at (cx, cy): its matter becomes fields. Its gas raises the density of
    /// the air, its shock squeezes the matter around it, its work digs the crater.
    pub fn detonate(&mut self, cx: i32, cy: i32, charge: Charge) {
        self.revision += 1;
        let r = reach(charge.grains());
        // The gas of the charge, spread over the air around it: a jump of density in the wind
        // solver, which then runs out as a pressure wave.
        let rise = charge.gas / (std::f32::consts::PI * (2 * r * r * 2) as f32);
        self.flow.blast(cx, cy, r * 2, rise.clamp(0.15, 3.0));
        let heavy = self.blasts.len() < 8;
        if heavy {
            self.blasts.push((cx, cy, r));
        }
        // A blast under water: how deep it is below the surface (None in the open air).
        let depth = {
            let mut d = 0;
            while d < 3 * r && self.raw(cx, cy - d).kind() == Kind::Liquid {
                d += 1;
            }
            (d > 0 && matches!(self.raw(cx, cy - d).kind(), Kind::Empty | Kind::Gas)).then_some(d)
        };
        let flying = self.ejecta.len();
        self.crater(cx, cy, charge.work, r);
        self.shock_wave(cx, cy, &charge, flying);
        if !heavy {
            return;
        }
        self.shove_out(cx, cy, r);
        // Under water the gas bubble lifts the surface above it into a dome and a plume of spray:
        // the shallower the blast, the higher the water is thrown.
        if let Some(d) = depth {
            let strength = (1.0 - d as f32 / (3.0 * r as f32)).clamp(0.0, 1.0);
            let half = r * 6 / 5;
            for dx in -half..=half {
                let x = cx + dx;
                let Some(top) = (cy - d - 2..=cy).find(|&y| self.raw(x, y).kind() == Kind::Liquid) else { continue };
                let shape = 1.0 - dx.abs() as f32 / half as f32;
                let rise = (strength * r as f32 * 1.6 * shape * (0.5 + (self.rand() % 100) as f32 / 100.0)) as i32;
                // The top few cells of the column go up, the highest first.
                for k in 0..3 {
                    let y = top + k;
                    if self.raw(x, y).kind() != Kind::Liquid {
                        break;
                    }
                    if let Some(up) = (1..=rise).rev().find(|&u| self.at(x, y - u) == Mat::Empty) {
                        self.swap((x, y), (x, y - up));
                    }
                }
            }
        }
        // A billow of smoke around the fireball.
        for dy in -(r * 3 / 2)..=(r * 3 / 2) {
            for dx in -(r * 3 / 2)..=(r * 3 / 2) {
                let d2 = dx * dx + dy * dy;
                if d2 > r * r && d2 <= (r * 3 / 2) * (r * 3 / 2) && self.chance(9) && self.at(cx + dx, cy + dy) == Mat::Empty {
                    self.set(cx + dx, cy + dy, Mat::Smoke);
                }
            }
        }
    }

    /// The shock of the charge: in the ground its peak pressure falls off about as the cube of the
    /// distance (it spreads over a wider front and the ground soaks it up), and every cell it crosses takes the changes that such a
    /// squeeze and such heat bring about (see `matter.rs`): the walls of the hole, and the clods
    /// thrown out of it (from `thrown` on in the list of ejecta).
    fn shock_wave(&mut self, cx: i32, cy: i32, c: &Charge, thrown: usize) {
        // Radius of the charge itself, and of the zone where the shock still changes anything.
        let a = (c.cells / std::f32::consts::PI).sqrt().max(1.0);
        let weakest = super::matter::TRANSITIONS
            .iter()
            .flat_map(|t| t.when.iter())
            .filter_map(|w| if let super::matter::Cond::ShockAbove(p) = w { Some(*p) } else { None })
            .fold(f32::MAX, f32::min);
        // The weakest brittle solid (ice) breaks under a lower shock still.
        let weakest = weakest.min(strength(Mat::Ice) * ATM_PER_MPA);
        let reach = (a * (c.shock / weakest).max(0.0).cbrt() * 1.2).min(80.0) as i32;
        // The cracks that run out from the charge, each at its angle and as far as it gets (in
        // shares of the distance at which the shock still breaks the matter), the same for the same
        // blast.
        let mut seed = (cx as u32).wrapping_mul(0x9e37_79b9) ^ (cy as u32).wrapping_mul(0x85eb_ca6b) ^ 0x2545_f491;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed % 10_000) as f32 / 10_000.0
        };
        let cracks: Vec<(f32, f32)> = (0..CRACKS).map(|k| ((k as f32 + next()) / CRACKS as f32 * std::f32::consts::TAU, 0.5 + 0.5 * next())).collect();
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (x, y) = (cx + dx, cy + dy);
                if !Self::inb(x, y) || self.mask[Self::idx(x, y)] != 0 {
                    continue;
                }
                // The ground is not even: the front is ragged.
                let rough = 0.85 + (self.rand() % 100) as f32 * 0.003;
                let d = (((dx * dx + dy * dy) as f32).sqrt() * rough).max(a);
                if d > reach as f32 || passable(self.raw(x, y)) {
                    continue;
                }
                let k = a / d;
                let i = Self::idx(x, y);
                let p = c.shock * k * k * k;
                self.shock_cell(i, p, c.flame * k);
                self.shock_break(i, p, d / a, (dy as f32).atan2(dx as f32), &cracks);
            }
        }
        // The clods started where they were torn off.
        for p in &mut self.ejecta[thrown..] {
            let d = (p.x - cx as f32).hypot(p.y - cy as f32).max(a);
            let k = a / d;
            if let Some(t) = super::matter::shocked(p.m, c.shock * k * k * k, c.flame * k, true) {
                p.m = t.to;
            }
        }
    }

    /// A brittle solid hit by a shock of `p` atm, at `d` radii of the charge from its centre in the
    /// direction `angle`. Rock holds about ten times more in compression than in tension: past its
    /// crushing strength it is ground to rubble on the spot (the crushed zone round a blast hole),
    /// past its tensile strength only where one of the radial `cracks` runs, and here and there
    /// between them, less and less often towards the end of the fractured zone. That is how a
    /// charge in a blast hole breaks the rock from the inside, without having to throw it anywhere.
    fn shock_break(&mut self, i: usize, p: f32, d: f32, angle: f32, cracks: &[(f32, f32)]) {
        let m = self.cells[i].m;
        let sigma = strength(m) * ATM_PER_MPA;
        let Some(into) = rubble(m) else { return };
        if sigma <= 0.0 || p < sigma {
            return;
        }
        // Wood splinters rather than shattering: only some of it breaks.
        if m.is_wooden() && !self.chance(3) {
            return;
        }
        let (x, y) = xy_i32(i);
        if p >= CRUSH * sigma {
            self.set(x, y, into);
            return;
        }
        // How far into the fractured zone, from 0 at the crushed zone to 1 at its outer edge.
        let depth = (p / sigma).ln() / CRUSH.ln();
        let on_crack = cracks.iter().any(|&(a, len)| {
            let off = (angle - a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            (off * d).abs() < 0.35 && depth > 1.0 - len
        });
        if on_crack || self.chance_f(0.3 * depth * depth) {
            self.set(x, y, into);
        }
    }

    /// Material of a cell as a blast sees it (rigid blocks included), None outside the world.
    fn blast_mat(&self, x: i32, y: i32) -> Option<(Mat, bool)> {
        if !Self::inb(x, y) {
            return None;
        }
        let i = Self::idx(x, y);
        let b = self.mask[i];
        Some(if b != 0 { (self.block_mat[b as usize - 1], true) } else { (self.cells[i].m, false) })
    }

    /// The cells along the ray from (cx, cy) in direction `a` (radians), with their distance.
    fn ray(cx: i32, cy: i32, a: f32, len: f32) -> Vec<(i32, i32, f32)> {
        let (ux, uy) = (a.cos(), a.sin());
        let mut out: Vec<(i32, i32, f32)> = vec![];
        let mut t = 0.0;
        while t <= len {
            let (x, y) = (cx + round_i(ux * t), cy + round_i(uy * t));
            if out.last().map_or(true, |&(px, py, _)| (px, py) != (x, y)) {
                out.push((x, y, t));
            }
            t += 0.5;
        }
        out
    }

    /// Spends `energy` joules on the ground around (cx, cy): digs the hole, throws the ejecta,
    /// fills what opened with fire and smoke. `r` is the reach of the fireball.
    fn crater(&mut self, cx: i32, cy: i32, energy: f32, r: i32) {
        // The farthest a ray could dig through the cheapest ground, with all the energy at once.
        let len = ((energy / cell_cost(Mat::Sand, 0.0)).sqrt() * 1.2 + r as f32).min(MAX_REACH);
        let n = ((std::f32::consts::TAU * len) as usize).clamp(256, 1440);
        let wedge = std::f32::consts::TAU / n as f32;
        // (The rays, and what it costs to break out along each, are worked out side by side.)
        let rays: Vec<Vec<(i32, i32, f32)>> = (0..n).into_par_iter().map(|k| Self::ray(cx, cy, k as f32 * wedge, len)).collect();
        // A ray owns `d · wedge` of each cell it crosses at distance d.
        let share = |d: f32| (d * wedge).clamp(0.08, 1.0);

        // First the cost of breaking out along each ray: what lies between the charge and the air.
        let escape: Vec<f32> = rays
            .par_iter()
            .map(|ray| {
                let (mut cost, mut ground, mut air) = (0.0, false, 0);
                for &(x, y, d) in ray {
                    let Some((m, _)) = self.blast_mat(x, y) else { return if ground { cost } else { 0.0 } };
                    if passable(m) {
                        if ground {
                            air += 1;
                            if air >= 2 {
                                return cost;
                            }
                        }
                    } else {
                        ground = true;
                        air = 0;
                        cost += cell_cost(m, d) * share(d);
                    }
                }
                // Sealed in: dearer than anything that gets out.
                cost * 3.0 + energy
            })
            .collect();
        let floor = cell_cost(Mat::Dirt, 4.0) * 4.0;
        let pull: Vec<f32> = escape.iter().map(|&c| 1.0 / (c + floor)).collect();
        let total: f32 = pull.iter().sum();

        let mut fates: std::collections::HashMap<usize, Fate> = Default::default();
        let mark = |fates: &mut std::collections::HashMap<usize, Fate>, i: usize, f: Fate| {
            let e = fates.entry(i).or_insert(f);
            if f > *e {
                *e = f;
            }
        };
        // Cells of air the fireball fills, and liquid the gas bubble pushes away.
        let mut fire: Vec<usize> = vec![];
        // Each ray digs until it runs out of energy, breaks out or meets something it cannot move.
        // The gas that rushes out of a ray that broke through still presses on the walls on either
        // side: most of what that ray has left goes to its neighbours, which dig on. So the hole
        // widens towards the surface into a cone, as real craters do.
        struct Ray {
            e: f32,
            at: usize,
            ground: bool,
            air: u32,
            out: bool,
            stuck: bool,
            dug: Vec<(usize, f32)>,
        }
        let mut st: Vec<Ray> = (0..n)
            .map(|k| Ray { e: energy * (0.2 / n as f32 + 0.8 * pull[k] / total), at: 0, ground: false, air: 0, out: false, stuck: false, dug: vec![] })
            .collect();
        for _round in 0..24 {
            for k in 0..n {
                let ray = &rays[k];
                let s = &mut st[k];
                if s.out || s.stuck {
                    continue;
                }
                while s.at < ray.len() {
                    let (x, y, d) = ray[s.at];
                    let Some((m, block)) = self.blast_mat(x, y) else {
                        s.out = s.ground;
                        s.stuck = !s.ground;
                        break;
                    };
                    let i = Self::idx(x, y);
                    if passable(m) {
                        if s.ground {
                            s.air += 1;
                            if s.air >= 2 {
                                s.out = true;
                            }
                        }
                        if d <= r as f32 {
                            fire.push(i);
                        } else if s.out || !s.ground {
                            s.stuck = !s.out;
                            break;
                        }
                        s.at += 1;
                        continue;
                    }
                    if s.out {
                        break;
                    }
                    s.ground = true;
                    s.air = 0;
                    let cost = cell_cost(m, d) * share(d);
                    if m.kind() == Kind::Liquid {
                        // Liquids do not break, they are pushed: the bubble opens where the gas is.
                        if d <= r as f32 * 0.8 {
                            fire.push(i);
                        }
                        s.e -= cost;
                        if s.e <= 0.0 {
                            s.stuck = true;
                            break;
                        }
                        s.at += 1;
                        continue;
                    }
                    if block {
                        s.stuck = true;
                        break;
                    }
                    if s.e < cost {
                        // Waits for more energy from a neighbour.
                        break;
                    }
                    s.e -= cost;
                    s.dug.push((i, rho(m) * cell_m().powi(3) * share(d)));
                    s.at += 1;
                }
                if s.at >= ray.len() && !s.out {
                    s.stuck = true;
                }
            }
            // Rays that broke out hand most of what they have left to the nearest rays on each
            // side that are still digging.
            let mut spill = vec![0.0f32; n];
            let mut any = false;
            // The nearest rays still digging, less than a quarter turn away on each side.
            let digs: Vec<bool> = st.iter().map(|s| !s.out && !s.stuck && s.ground).collect();
            let (left_of, right_of) = nearest_on_each_side(&digs, n / 4);
            for k in 0..n {
                if !st[k].out || st[k].e <= 0.0 {
                    continue;
                }
                let give = st[k].e * 0.7;
                let (left, right) = (left_of[k], right_of[k]);
                let to: Vec<usize> = [left, right].into_iter().flatten().collect();
                if to.is_empty() {
                    continue;
                }
                st[k].e -= give;
                for &j in &to {
                    spill[j] += give / to.len() as f32;
                }
                any = true;
            }
            if !any {
                break;
            }
            for k in 0..n {
                st[k].e += spill[k];
            }
        }
        for (k, s) in st.iter().enumerate() {
            let mut a = k as f32 * wedge;
            // The ground flows out of a crater along curved paths and leaves it at about 45°, not
            // straight up: it lands beyond the rim instead of falling back in.
            let up = -std::f32::consts::FRAC_PI_2;
            let off = (a - up + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            if off.abs() < std::f32::consts::FRAC_PI_2 {
                let side = if off.abs() < 0.02 { if k % 2 == 0 { 1.0 } else { -1.0 } } else { off.signum() };
                a = up + side * (0.55 + 0.6 * off.abs()).min(1.35);
            }
            if s.out && !s.dug.is_empty() {
                // Half the energy left goes to the clods, the rest rushes out with the gas.
                let mass: f32 = s.dug.iter().map(|c| c.1).sum();
                let v = super::units::cells_per_tick(((THROW * THROW + s.e.max(0.0) / mass).sqrt() * THROWN_SHARE).min(MAX_SPEED_MS));
                for &(i, _) in &s.dug {
                    mark(&mut fates, i, Fate::Throw(a.cos() * v, a.sin() * v));
                }
            } else {
                for &(i, _) in &s.dug {
                    mark(&mut fates, i, Fate::Crush);
                }
                // The edge of the hole: cracked, the more so the closer it came to breaking.
                if let Some(&(x, y, d)) = rays[k].get(s.at) {
                    if let Some((m, false)) = self.blast_mat(x, y) {
                        let p = (s.e / (cell_cost(m, d) * share(d))).clamp(0.0, 1.0);
                        // The ray goes on past the edge of the world: only the cells inside crack.
                        for &(x, y, _) in rays[k].iter().skip(s.at).take(3).filter(|c| Self::inb(c.0, c.1)) {
                            if self.rand() % 100 < (p * 80.0) as u32 + 10 {
                                mark(&mut fates, Self::idx(x, y), Fate::Crack);
                            }
                        }
                    }
                }
            }
        }

        let mut cells: Vec<(usize, Fate)> = fates.into_iter().collect();
        // A fixed order, so that the same blast always gives the same crater.
        cells.sort_unstable_by_key(|c| c.0);
        for (i, f) in cells {
            let (x, y) = xy_i32(i);
            let m = self.cells[i].m;
            if passable(m) {
                continue;
            }
            match f {
                Fate::Crack => {
                    if let Some(into) = rubble(m) {
                        if !m.is_wooden() || self.chance(3) {
                            self.set(x, y, into);
                        }
                    }
                }
                // Crushed matter that cannot get out stays: only the gas of the charge opens a room.
                // (Its shock breaks the rock around it: see `shock_break`.)
                Fate::Crush => {}
                Fate::Throw(vx, vy) => {
                    let roll = self.rand();
                    if self.ejecta.len() < MAX_EJECTA {
                        self.set(x, y, Mat::Empty);
                        // Clods spread a little around the ray and do not all fly as fast.
                        let k = 0.3 + ((roll >> 8) % 75) as f32 / 100.0;
                        let turn = ((roll >> 16) % 100) as f32 / 100.0 * 0.5 - 0.25;
                        let (s, c) = turn.sin_cos();
                        let (vx, vy) = ((vx * c - vy * s) * k, (vx * s + vy * c) * k);
                        self.ejecta.push(Ejecta { x: x as f32 + 0.5, y: y as f32 + 0.5, vx, vy, m: thrown(m, roll) });
                    } else if let Some(into) = rubble(m) {
                        // Too much in the air already: it stays in the hole, broken.
                        self.set(x, y, into);
                    }
                }
            }
        }
        // The hole and the air around fill with flame and smoke.
        fire.sort_unstable();
        fire.dedup();
        for i in fire {
            let (x, y) = xy_i32(i);
            let m = self.cells[i].m;
            if self.mask[i] != 0 || !(passable(m) || m.kind() == Kind::Liquid) {
                continue;
            }
            match self.rand() % 6 {
                0 => self.set(x, y, Mat::Fire),
                1 | 2 => self.set(x, y, Mat::Smoke),
                _ => self.set(x, y, Mat::Empty),
            }
        }
    }

    /// Moves the clods in flight; those that hit something settle there like a falling grain.
    pub(super) fn step_ejecta(&mut self) {
        if self.ejecta.is_empty() {
            return;
        }
        self.revision += 1;
        let mut flying = std::mem::take(&mut self.ejecta);
        let mut landed: Vec<(i32, i32, Mat)> = vec![];
        let terminal = super::units::cells_per_tick(CLOD_TERMINAL_MS);
        flying.retain_mut(|p| {
            super::ballistic::pull(&mut p.vx, &mut p.vy, super::units::g_tick(), terminal);
            match super::ballistic::sweep(&mut p.x, &mut p.y, p.vx, p.vy, |ix, iy| (!passable(self.at(ix, iy))).then_some(())) {
                super::ballistic::Path::Clear => true,
                super::ballistic::Path::Out => false,
                super::ballistic::Path::Hit(()) => {
                    landed.push((p.x.floor() as i32, p.y.floor() as i32, p.m));
                    false
                }
            }
        });
        self.ejecta = flying;
        for (x, y, m) in landed {
            self.settle(x, y, m);
        }
    }

    /// Lays a landed clod down where it fell, rolling down the slope like sand would.
    fn settle(&mut self, x: i32, mut y: i32, m: Mat) {
        let mut x = x.clamp(0, W() as i32 - 1);
        while y >= 0 && !passable(self.at(x, y)) {
            y -= 1;
        }
        if y < 0 {
            return;
        }
        for _ in 0..12 {
            if passable(self.at(x, y + 1)) && y + 1 < H() as i32 {
                y += 1;
                continue;
            }
            let side = if self.rand() & 1 == 0 { -1 } else { 1 };
            if let Some(s) = [side, -side].into_iter().find(|&s| passable(self.at(x + s, y + 1)) && passable(self.at(x + s, y))) {
                x += s;
                y += 1;
                continue;
            }
            break;
        }
        // A clod of earth or rock that falls in water sinks as mud or sand.
        let m = if m.kind() == Kind::Static && self.raw(x, y + 1).kind() == Kind::Liquid {
            match m {
                Mat::Dirt | Mat::Clay => Mat::Mud,
                Mat::Stone | Mat::Brick | Mat::Terracotta | Mat::Vault => Mat::Sand,
                _ => m,
            }
        } else {
            m
        };
        if Self::inb(x, y) && self.mask_free(x, y) {
            self.set(x, y, m);
        }
    }
}

/// For each place `k` of a ring of `ok.len()` places, the nearest place on the left and the
/// nearest on the right, closer than `reach`, where `ok` holds: what scanning out from `k` one
/// place at a time would find, in two sweeps instead.
fn nearest_on_each_side(ok: &[bool], reach: usize) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let n = ok.len();
    let (mut left, mut right) = (vec![None; n], vec![None; n]);
    // Round the ring twice, so that the places near the start see those near the end.
    let mut last: Option<usize> = None;
    for u in 0..2 * n {
        let k = u % n;
        if u >= n {
            left[k] = last.filter(|&p| u - p < reach).map(|p| p % n);
        }
        if ok[k] {
            last = Some(u);
        }
    }
    let mut next: Option<usize> = None;
    for u in (0..2 * n).rev() {
        let k = u % n;
        if u < n {
            right[k] = next.filter(|&p| p - u < reach).map(|p| p % n);
        }
        if ok[k] {
            next = Some(u);
        }
    }
    (left, right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_rays_on_each_side_are_those_a_scan_finds() {
        let mut r = 0x9e37_79b9u32;
        for n in [256usize, 257, 300, 1440] {
            for density in [2u32, 10, 60, 400] {
                let ok: Vec<bool> = (0..n)
                    .map(|_| {
                        r ^= r << 13;
                        r ^= r >> 17;
                        r ^= r << 5;
                        r % density == 0
                    })
                    .collect();
                let (left, right) = nearest_on_each_side(&ok, n / 4);
                for k in 0..n {
                    assert_eq!(left[k], (1..n / 4).map(|j| (k + n - j) % n).find(|&j| ok[j]), "left of {k} in {n}");
                    assert_eq!(right[k], (1..n / 4).map(|j| (k + j) % n).find(|&j| ok[j]), "right of {k} in {n}");
                }
            }
        }
    }

    /// A meadow over rock: grass on top at y = 300, dirt down to 420, stone below.
    fn ground() -> World {
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            w.set(x, 300, Mat::Grass);
            for y in 301..H() as i32 {
                w.set(x, y, if y < 420 { Mat::Dirt } else { Mat::Stone });
            }
        }
        w
    }

    fn bury(w: &mut World, x0: i32, y0: i32, wd: i32, ht: i32) {
        for x in x0..x0 + wd {
            for y in y0..y0 + ht {
                w.set(x, y, Mat::Gunpowder);
            }
        }
    }

    /// Cells of ground missing between x0..x1 and the surface down to y1.
    fn hole(w: &World, x0: i32, x1: i32, y1: i32) -> usize {
        (x0..x1).map(|x| (301..y1).filter(|&y| passable(w.raw(x, y)) || w.raw(x, y) == Mat::Fire).count()).sum()
    }

    fn run(w: &mut World, steps: usize) {
        for _ in 0..steps {
            w.step();
        }
    }

    #[test]
    fn stronger_materials_cost_more_to_dig() {
        assert!(cell_cost(Mat::Sand, 5.0) < cell_cost(Mat::Dirt, 5.0));
        assert!(cell_cost(Mat::Dirt, 5.0) * 5.0 < cell_cost(Mat::Stone, 5.0));
        assert!(cell_cost(Mat::Stone, 5.0) * 10.0 < cell_cost(Mat::Steel, 5.0));
        // Deeper ground has to be lifted higher.
        assert!(cell_cost(Mat::Dirt, 80.0) > cell_cost(Mat::Dirt, 5.0));
    }

    #[test]
    fn a_whole_charge_goes_off_at_once_at_its_centre() {
        let mut w = ground();
        bury(&mut w, 460, 330, 40, 20);
        w.set_off_powder(460, 330);
        assert_eq!(w.cells.iter().filter(|c| c.m == Mat::Gunpowder).count(), 0, "every grain went off");
        assert_eq!(w.blasts.len(), 1, "one blast, not 800");
        let (x, y, _) = w.blasts[0];
        assert!((x - 480).abs() <= 1 && (y - 340).abs() <= 1, "at the centre of the charge: {x},{y}");
    }

    #[test]
    fn a_trail_of_powder_burns_like_a_fuse_in_pieces() {
        let mut w = ground();
        for x in 100..700 {
            w.set(x, 299, Mat::Gunpowder);
        }
        w.set_off_powder(100, 299);
        let left = w.cells.iter().filter(|c| c.m == Mat::Gunpowder).count();
        assert!(left > 400, "only the start of the trail went off: {left} grains left");
    }

    #[test]
    fn a_bigger_charge_digs_a_bigger_crater() {
        let dig = |side: i32| {
            let mut w = ground();
            bury(&mut w, 480 - side / 2, 315, side, side);
            w.set_off_powder(480, 315);
            run(&mut w, 400);
            hole(&w, 300, 660, 420)
        };
        let (small, big) = (dig(6), dig(30));
        eprintln!("crater of 36 grains: {small} cells, of 900 grains: {big} cells");
        assert!(small > 60, "even a small charge digs: {small}");
        // What lands on the rim slides back down the slope a little: measured after it settled.
        assert!(big > small * 4, "25 times the powder, a far bigger hole: {small} vs {big}");
        assert!(big > 1500, "a big charge makes a real crater: {big}");
    }

    #[test]
    fn a_buried_charge_throws_earth_that_falls_back_as_a_rim() {
        let mut w = ground();
        bury(&mut w, 465, 320, 30, 20);
        w.set_off_powder(465, 320);
        assert!(w.ejecta.len() > 500, "earth flies out of the hole: {}", w.ejecta.len());
        assert!(w.ejecta.iter().filter(|p| p.vy < 0.0).count() * 2 > w.ejecta.len(), "mostly upwards");
        run(&mut w, 600);
        assert!(w.ejecta.is_empty(), "everything has come down");
        // Earth piled above the old ground level on both sides of the hole.
        let rim = |x0: i32, x1: i32| (x0..x1).map(|x| (150..300).filter(|&y| w.raw(x, y) == Mat::Dirt || w.raw(x, y) == Mat::Sand).count()).sum::<usize>();
        let (l, r) = (rim(300, 470), rim(490, 660));
        assert!(l > 100 && r > 100, "a rim on both sides: {l} / {r}");
        // The bottom of the hole is well under where the charge was.
        let lowest = (300..H() as i32).rev().find(|&y| passable(w.raw(480, y))).unwrap_or(0);
        assert!(lowest > 335, "the crater goes below the charge: {lowest}");
    }

    #[test]
    fn rock_resists_better_than_earth() {
        let dig = |m: Mat| {
            let mut w = World::new_static();
            for x in 0..W() as i32 {
                for y in 300..H() as i32 {
                    w.set(x, y, m);
                }
            }
            bury(&mut w, 470, 315, 20, 20);
            w.set_off_powder(470, 315);
            (200..760).map(|x| (300..500).filter(|&y| passable(w.raw(x, y)) || w.raw(x, y) == Mat::Fire).count()).sum::<usize>()
        };
        let (earth, rock, steel) = (dig(Mat::Dirt), dig(Mat::Stone), dig(Mat::Steel));
        eprintln!("hole in earth {earth}, rock {rock}, steel {steel}");
        // The 400 grains of the charge leave their own room in any case.
        assert!(earth - 400 > (rock - 400) * 2, "{earth} vs {rock}");
        // In steel only the powder itself leaves a hole.
        assert!(steel < 500, "{steel}");
    }

    #[test]
    fn a_charge_deep_in_the_ground_leaves_a_hidden_cavity_not_a_crater() {
        let mut w = ground();
        bury(&mut w, 476, 400, 8, 8);
        w.set_off_powder(476, 400);
        assert!(w.ejecta.is_empty(), "nothing gets out: {}", w.ejecta.len());
        assert!(passable(w.raw(480, 404)) || w.raw(480, 404) == Mat::Fire, "a cavity where the charge was");
        assert_eq!(w.raw(480, 301), Mat::Dirt, "the surface is untouched");
    }
    /// Charcoal packed around a charge of `m`, buried in rock; how many cells of diamond it leaves.
    fn diamonds(m: Mat) -> usize {
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            for y in 300..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 460..500 {
            for y in 380..420 {
                let inner = (468..492).contains(&x) && (388..412).contains(&y);
                w.set(x, y, if inner { m } else { Mat::Charcoal });
            }
        }
        w.set_off_powder(480, 400);
        run(&mut w, 300);
        w.cells.iter().filter(|c| c.m == Mat::Diamond).count()
    }

    #[test]
    fn dynamite_squeezes_charcoal_into_diamond_black_powder_does_not() {
        let (powder, dynamite) = (diamonds(Mat::Gunpowder), diamonds(Mat::Dynamite));
        eprintln!("diamond cells: powder {powder}, dynamite {dynamite}");
        assert_eq!(powder, 0, "black powder only burns: a few thousand atmospheres");
        assert!(dynamite > 50, "a detonation goes past 50 000 atm: {dynamite}");
    }

    #[test]
    fn a_detonation_packs_the_soil_left_around_the_hole() {
        let mut w = ground();
        for x in 474..486 {
            for y in 334..346 {
                w.set(x, y, Mat::Dynamite);
            }
        }
        w.set_off_powder(474, 334);
        let clay = w.cells.iter().filter(|c| c.m == Mat::Clay).count();
        assert!(clay > 20, "packed soil lines the hole: {clay}");
        // What flew out was loosened: no clod of packed clay in the air.
        assert!(w.ejecta.iter().all(|p| p.m != Mat::Clay));
    }

    #[test]
    fn a_detonation_shatters_rock_but_not_iron() {
        let shattered = |m: Mat| {
            let mut w = World::new_static();
            for x in 0..W() as i32 {
                for y in 300..H() as i32 {
                    w.set(x, y, m);
                }
            }
            w.detonate(480, 400, Charge::of(Mat::Dynamite, 30.0));
            (380..580).map(|x| (300..500).filter(|&y| w.raw(x, y) != m).count()).sum::<usize>()
        };
        let (stone, iron) = (shattered(Mat::Stone), shattered(Mat::Iron));
        eprintln!("broken by a stick: stone {stone}, iron {iron}");
        assert!(stone > 800, "{stone}");
        assert!(iron < 100, "{iron}");
        // Black powder only deflagrates: its shock barely cracks the rock.
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            for y in 300..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        w.detonate(480, 400, Charge::powder(30.0));
        let powder = (380..580).map(|x| (300..500).filter(|&y| w.raw(x, y) != Mat::Stone).count()).sum::<usize>();
        assert!(powder * 4 < stone, "powder {powder} vs dynamite {stone}");
    }

    #[test]
    fn a_charge_at_the_edges_of_the_world_stays_inside_it() {
        // The cracked edge of the hole used to be looked up past the bottom and sides of the grid.
        for (x, y) in [(480, H() as i32 - 2), (1, 400), (W() as i32 - 2, 400), (1, H() as i32 - 2), (480, 1)] {
            let mut w = World::new_static();
            for xx in 0..W() as i32 {
                for yy in 0..H() as i32 {
                    w.set(xx, yy, Mat::Stone);
                }
            }
            for dx in -4..=4 {
                for dy in -4..=4 {
                    w.set((x + dx).clamp(0, W() as i32 - 1), (y + dy).clamp(0, H() as i32 - 1), Mat::Dynamite);
                }
            }
            w.set_off_powder(x, y);
            w.explode(x, y, 40);
            run(&mut w, 5);
        }
    }
}
