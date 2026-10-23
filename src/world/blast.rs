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
//! the energy they had left; rays that run out of energy underground only crush the ground (a
//! hidden cavity, the "camouflet"). Projectiles fly, fall back and pile up as a rim: a real crater.
//!
//! The cells can be anything with a density and a strength, so the same rule shapes a crater in a
//! meadow, a hole in a brick wall or a dent in an iron plate.

use super::{density, Kind, Mat, World, H, W};

/// Side of a cell, in metres.
const CELL_M: f32 = 0.05;
/// Volume of a cell, m³.
const CELL_V: f32 = CELL_M * CELL_M * CELL_M;
const G: f32 = 9.81;
/// Speed a torn-off clod must be thrown at to clear the hole, m/s.
const THROW: f32 = 50.0;
/// Energy of black powder, J/kg.
const POWDER_J_PER_KG: f32 = 3.0e6;
/// Density of a grain of powder, kg/m³.
const POWDER_RHO: f32 = 1500.0;
/// Share of the chemical energy that does work on the ground (the rest heats the gas, light, noise).
const COUPLING: f32 = 0.01;
/// Gas a cell of powder gives when it burns, in cells of air at ambient pressure (black powder
/// gives about 280 litres per kilogram).
const GAS_PER_GRAIN: f32 = 420.0;
/// Speed in world cells per step for one metre per second (the world's clock is not a real one).
const SPEED: f32 = 0.08;
/// Fastest projectile, cells per step.
const MAX_SPEED: f32 = 9.0;
/// Pull on projectiles, cells per step².
const FALL: f32 = 0.12;
/// No more flying clods than this at once; the rest stay where they were, broken.
const MAX_EJECTA: usize = 14_000;
/// Longest ray, cells.
const MAX_REACH: f32 = 220.0;

/// Mechanical energy one grain of powder gives the ground, J.
pub fn grain_energy() -> f32 {
    POWDER_J_PER_KG * POWDER_RHO * CELL_V * COUPLING
}

/// Strength of a material, MPa (tensile or shear strength for solids, cohesion for soils). Loose
/// matter, liquids and gases have none: they only weigh.
pub fn strength(m: Mat) -> f32 {
    match m {
        Mat::Stone => 10.0,
        Mat::Brick => 3.0,
        Mat::Terracotta => 4.0,
        Mat::Vault => 5.0,
        Mat::Clay => 0.15,
        Mat::Dirt => 0.05,
        Mat::Grass => 0.08,
        Mat::Iron => 250.0,
        Mat::Steel => 400.0,
        Mat::Glass => 7.0,
        Mat::Ice => 1.5,
        Mat::Wood => 5.0,
        Mat::Trunk => 8.0,
        Mat::Branch => 4.0,
        Mat::Leaf | Mat::Sapling => 0.1,
        Mat::Ember => 0.5,
        _ => 0.0,
    }
}

/// Density, kg/m³ (air for gas and empty cells).
fn rho(m: Mat) -> f32 {
    match m.kind() {
        Kind::Empty | Kind::Gas | Kind::Flame => 1.2,
        _ => density(m) * 1000.0,
    }
}

/// Energy to tear one cell of `m` loose at `d` cells from the centre and throw it out, J.
pub fn cell_cost(m: Mat, d: f32) -> f32 {
    CELL_V * (strength(m) * 1.0e6 + 0.5 * rho(m) * (THROW * THROW + 2.0 * G * d * CELL_M))
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
        Mat::Ice => if roll % 2 == 0 { Mat::Ice } else { Mat::Snow },
        Mat::Brick | Mat::Terracotta | Mat::Vault => if roll % 2 == 0 { m } else { Mat::Sand },
        Mat::Leaf | Mat::Sapling => Mat::DeadLeaf,
        Mat::Wood | Mat::Trunk | Mat::Branch => if roll % 3 == 0 { Mat::Ember } else { Mat::Wood },
        _ => m,
    }
}

/// What a crushed (or cracked) solid leaves, if anything.
fn rubble(m: Mat) -> Option<Mat> {
    match m {
        Mat::Stone | Mat::Brick | Mat::Terracotta | Mat::Vault => Some(Mat::Sand),
        Mat::Glass => Some(Mat::Shard),
        Mat::Ice => Some(Mat::Snow),
        Mat::Wood | Mat::Trunk | Mat::Branch => Some(Mat::Ember),
        _ => None,
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
    /// Torn loose and pressed into the walls of a hidden cavity.
    Crush,
    /// Thrown out of the crater at this speed (cells per step).
    Throw(f32, f32),
}

fn passable(m: Mat) -> bool {
    matches!(m.kind(), Kind::Empty | Kind::Gas | Kind::Flame)
}

impl World {
    /// Gunpowder at (x, y) caught fire: the whole charge it belongs to goes off together, at its
    /// centre. A long thin trail of powder goes off in pieces, like a fuse.
    pub(super) fn set_off_powder(&mut self, x: i32, y: i32) {
        let mut charge = vec![(x, y)];
        let mut front = vec![(x, y)];
        let mut taken = std::collections::HashSet::from([(x, y)]);
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
                        if self.raw(q.0, q.1) == Mat::Gunpowder && self.mask_free(q.0, q.1) && taken.insert(q) {
                            next.push(q);
                        }
                    }
                }
            }
            charge.extend_from_slice(&next);
            front = next;
        }
        let n = charge.len() as f32;
        let (sx, sy) = charge.iter().fold((0.0, 0.0), |a, &(x, y)| (a.0 + x as f32, a.1 + y as f32));
        for &(x, y) in &charge {
            self.set(x, y, Mat::Empty);
        }
        let (cx, cy) = ((sx / n).round() as i32, (sy / n).round() as i32);
        // One report per charge, a few more for a big one (more rockets at night).
        for _ in 0..1 + (charge.len() / 150).min(3) {
            if self.powder_booms.len() < 16 {
                self.powder_booms.push((cx, cy));
            }
        }
        self.detonate(cx, cy, n);
    }

    /// An explosion as strong as `grains` cells of gunpowder going off at (cx, cy).
    pub fn detonate(&mut self, cx: i32, cy: i32, grains: f32) {
        self.revision += 1;
        let r = reach(grains);
        // The gas of the charge, spread over the air around it: a jump of density in the wind
        // solver, which then runs out as a pressure wave.
        let rise = GAS_PER_GRAIN * grains / (std::f32::consts::PI * (2 * r * r * 2) as f32);
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
        self.crater(cx, cy, grains * grain_energy(), r);
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
            let (x, y) = (cx + (ux * t).round() as i32, cy + (uy * t).round() as i32);
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
        let rays: Vec<Vec<(i32, i32, f32)>> = (0..n).map(|k| Self::ray(cx, cy, k as f32 * wedge, len)).collect();
        // A ray owns `d · wedge` of each cell it crosses at distance d.
        let share = |d: f32| (d * wedge).clamp(0.08, 1.0);

        // First the cost of breaking out along each ray: what lies between the charge and the air.
        let escape: Vec<f32> = rays
            .iter()
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
        for (k, ray) in rays.iter().enumerate() {
            let mut e = energy * (0.2 / n as f32 + 0.8 * pull[k] / total);
            let (mut ground, mut air, mut out) = (false, 0, false);
            let mut dug: Vec<(usize, f32)> = vec![];
            for (j, &(x, y, d)) in ray.iter().enumerate() {
                let Some((m, block)) = self.blast_mat(x, y) else {
                    out = ground;
                    break;
                };
                let i = Self::idx(x, y);
                if passable(m) {
                    if ground {
                        air += 1;
                        if air >= 2 {
                            out = true;
                        }
                    }
                    if d <= r as f32 {
                        fire.push(i);
                    } else if out || !ground {
                        break;
                    }
                    continue;
                }
                if out {
                    break;
                }
                ground = true;
                air = 0;
                let cost = cell_cost(m, d) * share(d);
                if m.kind() == Kind::Liquid {
                    // Liquids do not break, they are pushed: the bubble opens where the gas is.
                    if d <= r as f32 * 0.8 {
                        fire.push(i);
                    }
                    e -= cost;
                    if e <= 0.0 {
                        break;
                    }
                    continue;
                }
                if e < cost || block {
                    if !block {
                        // The edge of the hole: cracked, the more so the closer it came to breaking.
                        let p = (e / cost).clamp(0.0, 1.0);
                        for &(x, y, _) in ray.iter().skip(j).take(3) {
                            if self.rand() % 100 < (p * 80.0) as u32 + 10 {
                                mark(&mut fates, Self::idx(x, y), Fate::Crack);
                            }
                        }
                    }
                    break;
                }
                e -= cost;
                dug.push((i, rho(m) * CELL_V * share(d)));
            }
            let a = k as f32 * wedge;
            if out && !dug.is_empty() {
                // Half the energy left goes to the clods, the rest rushes out with the gas.
                let mass: f32 = dug.iter().map(|c| c.1).sum();
                let v = (THROW * THROW + e.max(0.0) / mass).sqrt() * SPEED;
                let v = v.min(MAX_SPEED);
                for &(i, _) in &dug {
                    mark(&mut fates, i, Fate::Throw(a.cos() * v, a.sin() * v));
                }
            } else {
                for &(i, _) in &dug {
                    mark(&mut fates, i, Fate::Crush);
                }
            }
        }

        let mut cells: Vec<(usize, Fate)> = fates.into_iter().collect();
        // A fixed order, so that the same blast always gives the same crater.
        cells.sort_unstable_by_key(|c| c.0);
        for (i, f) in cells {
            let (x, y) = ((i % W) as i32, (i / W) as i32);
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
                Fate::Crush => {
                    let left = rubble(m).filter(|_| self.chance(3));
                    self.set(x, y, left.unwrap_or(Mat::Empty));
                }
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
            let (x, y) = ((i % W) as i32, (i / W) as i32);
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
        flying.retain_mut(|p| {
            p.vy += FALL;
            p.vx *= 0.995;
            p.vy *= 0.995;
            let steps = p.vx.abs().max(p.vy.abs()).ceil().max(1.0) as i32;
            for _ in 0..steps {
                let (nx, ny) = (p.x + p.vx / steps as f32, p.y + p.vy / steps as f32);
                let (ix, iy) = (nx.floor() as i32, ny.floor() as i32);
                if ix < 0 || ix >= W as i32 || iy >= H as i32 {
                    return false;
                }
                if iy >= 0 && !passable(self.at(ix, iy)) {
                    landed.push((p.x.floor() as i32, p.y.floor() as i32, p.m));
                    return false;
                }
                (p.x, p.y) = (nx, ny);
            }
            true
        });
        self.ejecta = flying;
        for (x, y, m) in landed {
            self.settle(x, y, m);
        }
    }

    /// Lays a landed clod down where it fell, rolling down the slope like sand would.
    fn settle(&mut self, x: i32, mut y: i32, m: Mat) {
        let mut x = x.clamp(0, W as i32 - 1);
        while y >= 0 && !passable(self.at(x, y)) {
            y -= 1;
        }
        if y < 0 {
            return;
        }
        for _ in 0..12 {
            if passable(self.at(x, y + 1)) && y + 1 < H as i32 {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A meadow over rock: grass on top at y = 300, dirt down to 420, stone below.
    fn ground() -> World {
        let mut w = World::new_static();
        for x in 0..W as i32 {
            w.set(x, 300, Mat::Grass);
            for y in 301..H as i32 {
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
        assert!(big > small * 8, "25 times the powder, a far bigger hole: {small} vs {big}");
        assert!(big > 4000, "a big charge makes a real crater: {big}");
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
        let lowest = (300..H as i32).rev().find(|&y| passable(w.raw(480, y))).unwrap_or(0);
        assert!(lowest > 335, "the crater goes below the charge: {lowest}");
    }

    #[test]
    fn rock_resists_better_than_earth() {
        let dig = |m: Mat| {
            let mut w = World::new_static();
            for x in 0..W as i32 {
                for y in 300..H as i32 {
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
        assert!(earth - 400 > (rock - 400) * 3, "{earth} vs {rock}");
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
}
