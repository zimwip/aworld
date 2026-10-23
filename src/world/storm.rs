//! Storms: electric charge in the clouds, lightning and thunder.
//!
//! In a thick cloud whose top is below freezing, soft hail (graupel) and ice crystals bump into
//! each other in the updraft: the crystals come away with a positive charge and, being light,
//! are carried up to the top of the cloud, the heavier graupel keeps the negative charge lower
//! down. The charge is kept on a coarse grid (`charge`, one square per `QS`×`QS` cells), carried
//! by the wind, and leaks away slowly. When a centre of charge is strong enough the air breaks
//! down: a bolt of lightning jumps inside the cloud, or, from the negative base, down to the
//! ground, to the highest and best-conducting point near it (a tree, a mast of iron, water). The
//! channel heats the air to thousands of degrees, sets wood on fire, fuses sand into glass
//! (fulgurite), boils water and runs through connected metal. The player can also strike with the
//! lightning tool (`strike`). Bolts and thunder are left in `bolts` for the picture and the sound.

use super::*;

/// Cells per side of a square of the charge grid.
pub const QS: usize = 6;
pub const QW: usize = W / QS;
pub const QH: usize = H / QS;
/// Charge separation per step in a square full of cold cloud with an updraft.
const SEPARATION: f32 = 0.0006;
/// Charge (arbitrary units) at which the air breaks down.
pub const BREAKDOWN: f32 = 1.0;
/// Below this temperature a cloud holds ice: crystals and graupel can trade charge.
const ICE_C: f32 = -2.0;
/// How often (in steps) the charge is updated.
const STORM_EVERY: u32 = 4;
/// Temperature of the lightning channel (the air in it reaches about 30 000 °C for an instant; the
/// game keeps it to something its heat flow can cope with).
const CHANNEL_C: f32 = 2500.0;

/// A bolt of lightning, in cell coordinates: the main channel from top to bottom, and its branches.
#[derive(Clone, Debug)]
pub struct Bolt {
    pub path: Vec<(f32, f32)>,
    pub branches: Vec<Vec<(f32, f32)>>,
    /// True if it reached the ground (or whatever it struck), false inside a cloud.
    pub ground: bool,
    /// 0..1: how strong it was (brightness and thunder).
    pub power: f32,
}

/// A tiny random number generator of the storm's own, so that storms do not change the order of
/// the world's random draws (and every other behaviour with it).
#[derive(Clone, Debug)]
pub struct StormRng(pub u32);

impl StormRng {
    pub fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    /// Uniform in -1..1.
    pub fn f(&mut self) -> f32 {
        (self.next() % 20_001) as f32 / 10_000.0 - 1.0
    }
}

/// How well lightning likes to strike a cell: metal and water conduct, trees stand tall and wet.
pub fn conductivity(m: Mat) -> f32 {
    match m {
        Mat::Iron | Mat::Steel | Mat::Molten | Mat::MoltenSteel => 1.0,
        Mat::Water | Mat::Mud => 0.6,
        Mat::Trunk | Mat::Branch | Mat::Leaf | Mat::Wood => 0.4,
        Mat::Dirt | Mat::Grass | Mat::Clay => 0.2,
        _ => 0.0,
    }
}

/// Midpoint displacement: a jagged line from `a` to `b`, each half kinked sideways by a random
/// fraction of its length. `rough` is that fraction.
pub fn jagged(a: (f32, f32), b: (f32, f32), depth: u32, rough: f32, rng: &mut StormRng) -> Vec<(f32, f32)> {
    let mut pts = vec![a, b];
    for _ in 0..depth {
        let mut next = Vec::with_capacity(pts.len() * 2);
        for w in pts.windows(2) {
            let (p, q) = (w[0], w[1]);
            let (dx, dy) = (q.0 - p.0, q.1 - p.1);
            let len = (dx * dx + dy * dy).sqrt();
            // Kink across the segment.
            let k = rng.f() * rough * len;
            let (nx, ny) = if len > 0.0 { (-dy / len, dx / len) } else { (0.0, 0.0) };
            next.push(p);
            next.push(((p.0 + q.0) * 0.5 + nx * k, (p.1 + q.1) * 0.5 + ny * k));
        }
        next.push(*pts.last().unwrap());
        pts = next;
    }
    pts
}

impl World {
    /// Charge of the square holding cell (x, y).
    #[cfg(test)]
    pub fn charge_at(&self, x: i32, y: i32) -> f32 {
        let (qx, qy) = ((x.clamp(0, W as i32 - 1) as usize) / QS, (y.clamp(0, H as i32 - 1) as usize) / QS);
        self.charge[qy * QW + qx]
    }

    pub(super) fn storm_step(&mut self) {
        if self.ticks % STORM_EVERY != 0 {
            return;
        }
        // Cloud in each square, and how cold it is.
        let mut cloud = vec![0.0f32; QW * QH];
        let mut cold = vec![0.0f32; QW * QH];
        for qy in 0..QH {
            for qx in 0..QW {
                let (mut n, mut t) = (0u32, 0.0f32);
                for y in qy * QS..(qy + 1) * QS {
                    for x in qx * QS..(qx + 1) * QS {
                        let i = y * W + x;
                        if self.cells[i].m == Mat::Cloud {
                            n += 1;
                            t += self.temp[i];
                        }
                    }
                }
                let k = qy * QW + qx;
                cloud[k] = n as f32 / (QS * QS) as f32;
                cold[k] = if n > 0 && t / (n as f32) < ICE_C { 1.0 } else { 0.0 };
            }
        }
        let any_charge = self.charge.iter().any(|q| q.abs() > 1e-4);
        if !any_charge && !cloud.iter().zip(&cold).any(|(c, k)| *c > 0.5 && *k > 0.0) {
            return;
        }
        // 1. Separation: in thick, icy cloud with cloud under it (a deep cloud), driven by the
        //    updraft, the positive charge goes up to the top and the negative stays low.
        let dt = STORM_EVERY as f32;
        let mut q = std::mem::take(&mut self.charge);
        for qy in 3..QH - 1 {
            for qx in 0..QW {
                let k = qy * QW + qx;
                if cloud[k] < 0.5 || cold[k] == 0.0 {
                    continue;
                }
                let deep = (1..=4).filter(|d| qy + d < QH && cloud[(qy + d) * QW + qx] > 0.4).count() as f32 / 4.0;
                if deep < 0.5 {
                    continue;
                }
                let (_, vy) = self.flow.sample((qx * QS + QS / 2) as i32, (qy * QS + QS / 2) as i32);
                let updraft = 0.3 + (-vy).clamp(0.0, 2.0);
                let dq = SEPARATION * dt * cloud[k] * deep * updraft;
                // The crystals are lifted a few squares, as far as the cloud goes.
                let mut top = qy;
                while top > qy.saturating_sub(4) && top > 0 && cloud[(top - 1) * QW + qx] > 0.3 {
                    top -= 1;
                }
                q[top * QW + qx] += dq;
                q[(qy + 1).min(QH - 1) * QW + qx] -= dq;
            }
        }
        // 2. The wind carries the charge along (semi-Lagrangian: each square takes what was upwind
        //    of it, interpolated), and it leaks away, quickly out of the cloud.
        let mut moved = vec![0.0f32; QW * QH];
        for qy in 0..QH {
            for qx in 0..QW {
                let (cx, cy) = ((qx * QS + QS / 2) as i32, (qy * QS + QS / 2) as i32);
                // Only sideways: up and down, the charge sits where the sorting of the ice put it
                // (the crystals ride the updraft to the top, the graupel falls against it).
                let (vx, _) = self.flow.sample(cx, cy);
                let sx = (qx as f32 - vx * dt / QS as f32).clamp(0.0, QW as f32 - 1.001);
                let sy = qy as f32;
                let (i, j) = (sx as usize, sy as usize);
                let (tx, ty) = (sx - i as f32, sy - j as f32);
                let at = |a: usize, b: usize| q[b.min(QH - 1) * QW + a.min(QW - 1)];
                let v = (at(i, j) * (1.0 - tx) + at(i + 1, j) * tx) * (1.0 - ty) + (at(i, j + 1) * (1.0 - tx) + at(i + 1, j + 1) * tx) * ty;
                let k = qy * QW + qx;
                let keep = if cloud[k] > 0.2 { 0.9985 } else { 0.97 };
                moved[k] = v * keep;
            }
        }
        // (Charge can neither be made nor lost by the carrying: what two squares took from the
        // same source is a small error the leak soon hides.)
        q = moved;
        self.charge = q;
        // 3. Breakdown.
        let (mut best, mut at) = (0.0f32, 0usize);
        for (k, v) in self.charge.iter().enumerate() {
            if v.abs() > best {
                best = v.abs();
                at = k;
            }
        }
        if best < BREAKDOWN {
            return;
        }
        let (qx, qy) = (at % QW, at / QW);
        let (x0, y0) = ((qx * QS + QS / 2) as i32, (qy * QS + QS / 2) as i32);
        let negative = self.charge[at] < 0.0;
        let to_ground = negative && self.storm_rng.next() % 10 < 6;
        if to_ground {
            // From the base of the cloud, down to the best spot under it.
            let mut base = y0;
            while base < H as i32 - 1 && matches!(self.raw(x0, base + 1), Mat::Cloud | Mat::Snow | Mat::Hail) {
                base += 1;
            }
            let target = self.lightning_target(x0, base, 90);
            self.discharge_around(qx, qy, 5);
            self.lightning_bolt((x0 as f32, base as f32), target, best.min(3.0) / 3.0, true);
        } else {
            // Inside the cloud: to the strongest charge of the other sign near it.
            let (mut other, mut ok) = (at, 0.0f32);
            for dy in -12i32..=12 {
                for dx in -16i32..=16 {
                    let (ox, oy) = (qx as i32 + dx, qy as i32 + dy);
                    if ox < 0 || oy < 0 || ox >= QW as i32 || oy >= QH as i32 {
                        continue;
                    }
                    let k = oy as usize * QW + ox as usize;
                    let v = -self.charge[k] * self.charge[at].signum();
                    if v > ok {
                        ok = v;
                        other = k;
                    }
                }
            }
            let (ox, oy) = ((other % QW * QS + QS / 2) as f32, (other / QW * QS + QS / 2) as f32);
            self.discharge_around(qx, qy, 4);
            self.discharge_around(other % QW, other / QW, 4);
            let (a, b) = if oy < y0 as f32 { ((ox, oy), (x0 as f32, y0 as f32)) } else { ((x0 as f32, y0 as f32), (ox, oy)) };
            self.lightning_bolt(a, (b.0 as i32, b.1 as i32), best.min(3.0) / 3.0 * 0.6, false);
        }
    }

    /// The charge around a square drains through the bolt.
    fn discharge_around(&mut self, qx: usize, qy: usize, r: i32) {
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (qx as i32 + dx, qy as i32 + dy);
                if x >= 0 && y >= 0 && x < QW as i32 && y < QH as i32 {
                    self.charge[y as usize * QW + x as usize] *= 0.15;
                }
            }
        }
    }

    /// Where a bolt coming down from (x0, y0) strikes: the point within `reach` columns that
    /// stands highest, conducts best and is nearest. Returns the first solid or liquid cell.
    pub fn lightning_target(&mut self, x0: i32, y0: i32, reach: i32) -> (i32, i32) {
        let mut best = (x0.clamp(0, W as i32 - 1), H as i32 - 1);
        let mut score = f32::MIN;
        let mut x = (x0 - reach).max(0);
        while x <= (x0 + reach).min(W as i32 - 1) {
            if let Some(y) = self.first_hit(x, y0) {
                let m = self.raw(x, y);
                let s = -(y as f32) + 40.0 * conductivity(m) - (x - x0).abs() as f32 * 0.35 + self.storm_rng.f() * 6.0;
                if s > score {
                    score = s;
                    best = (x, y);
                }
            }
            x += 2;
        }
        best
    }

    /// The first cell under (x, y) that lightning can strike (not air, cloud or smoke).
    fn first_hit(&self, x: i32, y: i32) -> Option<i32> {
        (y.max(0)..H as i32).find(|&yy| {
            let m = self.raw(x, yy);
            match m.kind() {
                Kind::Empty | Kind::Gas | Kind::Flame => false,
                _ => !(matches!(m, Mat::Snow | Mat::Hail) && yy < GROUND_ROW),
            }
        })
    }

    /// Lightning from the sky where the player clicked: from the cloud above (or the top of the
    /// sky) down to the first thing in the column under the pointer. A bolt comes from the sky,
    /// so a click deep in the ground strikes the surface above it, not a point inside the rock.
    pub fn strike(&mut self, x: i32, y: i32) {
        let x = x.clamp(0, W as i32 - 1);
        let _ = y;
        let hit = self.first_hit(x, 0).unwrap_or(H as i32 - 1);
        // From the base of the cloud above (the first cloud from the top, down through it), or
        // else from the top of the picture. A wisp of cloud just above the ground does not count.
        let mut top = (0..hit).find(|&yy| self.raw(x, yy) == Mat::Cloud).unwrap_or(0);
        while top + 1 < hit && self.raw(x, top + 1) == Mat::Cloud {
            top += 1;
        }
        if hit - top < 60 {
            top = 0;
        }
        let start = ((x as f32 + self.storm_rng.f() * 30.0).clamp(0.0, W as f32 - 1.0), top as f32);
        self.lightning_bolt(start, (x, hit), 1.0, true);
    }

    /// Builds a bolt from `a` down to `b`, heats its channel and, if it reached the ground,
    /// strikes what is there.
    fn lightning_bolt(&mut self, a: (f32, f32), b: (i32, i32), power: f32, ground: bool) {
        let mut rng = self.storm_rng.clone();
        let end = (b.0 as f32, b.1 as f32);
        let len = ((end.0 - a.0).powi(2) + (end.1 - a.1).powi(2)).sqrt();
        let depth = (len.max(8.0).log2() as u32).clamp(3, 8);
        let path = jagged(a, end, depth, 0.32, &mut rng);
        // Branches: forks from the upper part of the channel that die out in the air.
        let mut branches = vec![];
        let forks = 2 + rng.next() % 5;
        for _ in 0..forks {
            let k = (rng.next() as usize % (path.len() * 2 / 3).max(1)).min(path.len() - 1);
            let p = path[k];
            let (dx, dy) = (end.0 - a.0, end.1 - a.1);
            let side = if rng.next() % 2 == 0 { 1.0 } else { -1.0 };
            let blen = len * (0.12 + 0.25 * (rng.f() * 0.5 + 0.5));
            let ang = (dy.atan2(dx)) + side * (0.35 + 0.6 * (rng.f() * 0.5 + 0.5));
            let q = (p.0 + ang.cos() * blen, p.1 + ang.sin() * blen);
            branches.push(jagged(p, q, depth.saturating_sub(2).max(2), 0.4, &mut rng));
        }
        self.storm_rng = rng;
        // The channel: the air in it is heated in an instant (the heat haze and the glow follow).
        for w in path.windows(2) {
            let (p, q) = (w[0], w[1]);
            let n = ((q.0 - p.0).abs().max((q.1 - p.1).abs()).ceil() as i32).max(1);
            for s in 0..=n {
                let f = s as f32 / n as f32;
                let (x, y) = ((p.0 + (q.0 - p.0) * f) as i32, (p.1 + (q.1 - p.1) * f) as i32);
                if Self::inb(x, y) && self.raw(x, y) == Mat::Empty {
                    let i = Self::idx(x, y);
                    self.temp[i] = self.temp[i].max(CHANNEL_C * power.max(0.4));
                    self.zones.wake(i);
                }
            }
        }
        if ground {
            self.lightning_hit(b.0, b.1, power);
        }
        self.revision += 1;
        self.bolts.push(Bolt { path, branches, ground, power });
    }

    /// What a bolt does where it strikes.
    fn lightning_hit(&mut self, x: i32, y: i32, power: f32) {
        if !Self::inb(x, y) {
            return;
        }
        let r = 2 + (power * 3.0) as i32;
        let m0 = self.raw(x, y);
        for dy in -r..=r {
            for dx in -r..=r {
                let (cx, cy) = (x + dx, y + dy);
                let d2 = dx * dx + dy * dy;
                if d2 > r * r || !Self::inb(cx, cy) {
                    continue;
                }
                let i = Self::idx(cx, cy);
                let near = 1.0 - (d2 as f32).sqrt() / (r as f32 + 1.0);
                let m = self.cells[i].m;
                self.temp[i] += 900.0 * near * power.max(0.5) * if m == Mat::Empty { 1.0 } else { 0.6 };
                self.zones.wake(i);
                if m.is_wooden() || matches!(m, Mat::Grass | Mat::DeadLeaf | Mat::Sapling) {
                    if self.storm_rng.next() % 3 != 0 {
                        self.set(cx, cy, if m == Mat::Grass || m == Mat::DeadLeaf { Mat::Fire } else { Mat::Ember });
                    }
                } else if m == Mat::Gunpowder {
                    self.temp[i] = self.temp[i].max(1000.0);
                }
            }
        }
        // Sand fuses into a tube of glass along the path of the current into the ground.
        if matches!(m0, Mat::Sand) {
            let deep = 4 + (power * 10.0) as i32;
            let mut xx = x;
            for k in 0..deep {
                if !Self::inb(xx, y + k) || self.raw(xx, y + k) != Mat::Sand {
                    break;
                }
                self.set(xx, y + k, Mat::Glass);
                if self.storm_rng.next() % 3 == 0 {
                    xx += if self.storm_rng.next() % 2 == 0 { 1 } else { -1 };
                }
            }
        }
        // Metal carries the current: everything of it that touches is heated.
        if matches!(m0, Mat::Iron | Mat::Steel) {
            let mut seen = std::collections::HashSet::new();
            let mut stack = vec![(x, y)];
            seen.insert((x, y));
            while let Some((cx, cy)) = stack.pop() {
                let i = Self::idx(cx, cy);
                self.temp[i] += 250.0 * power.max(0.5);
                self.zones.wake(i);
                if seen.len() > 1500 {
                    continue;
                }
                for (nx, ny) in Self::neighbors(cx, cy) {
                    if Self::inb(nx, ny) && matches!(self.raw(nx, ny), Mat::Iron | Mat::Steel) && seen.insert((nx, ny)) {
                        stack.push((nx, ny));
                    }
                }
            }
        }
        // Water: a puff of steam where it was struck.
        if m0 == Mat::Water {
            for k in 0..(1 + (power * 3.0) as i32) {
                let (cx, cy) = (x + k - 1, y);
                if Self::inb(cx, cy) && self.raw(cx, cy) == Mat::Water {
                    self.vaporize(cx, cy);
                }
            }
        }
    }
}

/// Lightning goes into snow lying on the ground (low down), not into falling flakes.
const GROUND_ROW: i32 = (H as i32) / 2;

impl World {
    /// Charge at a cell, smoothly interpolated between the squares of the grid.
    pub fn charge_smooth(&self, x: usize, y: usize) -> f32 {
        let fx = ((x as f32 + 0.5) / QS as f32 - 0.5).clamp(0.0, QW as f32 - 1.001);
        let fy = ((y as f32 + 0.5) / QS as f32 - 0.5).clamp(0.0, QH as f32 - 1.001);
        let (i, j) = (fx as usize, fy as usize);
        let (tx, ty) = (fx - i as f32, fy - j as f32);
        let q = |a: usize, b: usize| self.charge[b * QW + a];
        (q(i, j) * (1.0 - tx) + q(i + 1, j) * tx) * (1.0 - ty) + (q(i, j + 1) * (1.0 - tx) + q(i + 1, j + 1) * tx) * ty
    }

    /// The electricity view: the world dimmed to grey, what conducts lit up (metal gold, water
    /// and wet things cyan), and the charge of the clouds over it, red where positive and blue
    /// where negative, brightest close to breaking down.
    pub fn electric_color(&self, x: usize, y: usize) -> [u8; 4] {
        let m = self.cells[y * W + x].m;
        let base = self.color(x, y);
        let grey = (base[0] as f32 * 0.3 + base[1] as f32 * 0.59 + base[2] as f32 * 0.11) * 0.4 * base[3] as f32 / 255.0;
        let mut c = [grey + 10.0, grey + 10.0, grey + 16.0];
        let k = conductivity(m);
        if k > 0.0 {
            let tint = if matches!(m, Mat::Iron | Mat::Steel | Mat::Molten | Mat::MoltenSteel) { [255.0, 205.0, 70.0] } else if matches!(m, Mat::Water | Mat::Mud) { [70.0, 210.0, 230.0] } else { [140.0, 170.0, 120.0] };
            for (v, t) in c.iter_mut().zip(tint) {
                *v += (t - *v) * (0.25 + 0.6 * k);
            }
        }
        if m == Mat::Cloud {
            c = [62.0, 66.0, 82.0];
        }
        let q = self.charge_smooth(x, y) / BREAKDOWN;
        let a = q.abs().min(1.0).sqrt();
        if a > 0.02 {
            let tint = if q > 0.0 { [255.0, 70.0, 60.0] } else { [70.0, 130.0, 255.0] };
            for (v, t) in c.iter_mut().zip(tint) {
                *v += (t - *v) * a * 0.85;
            }
            // Close to breaking down the charge glows white-hot.
            let hot = ((q.abs() - 0.75) / 0.25).clamp(0.0, 1.0);
            for v in &mut c {
                *v += (255.0 - *v) * hot * 0.5;
            }
        }
        [c[0].min(255.0) as u8, c[1].min(255.0) as u8, c[2].min(255.0) as u8, 255]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground(w: &mut World, top: i32) {
        for x in 0..W as i32 {
            for y in top..H as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
    }

    #[test]
    fn a_jagged_bolt_keeps_its_ends() {
        let mut rng = StormRng(7);
        let p = jagged((10.0, 0.0), (40.0, 300.0), 6, 0.3, &mut rng);
        assert_eq!(p.len(), 65);
        assert_eq!(p[0], (10.0, 0.0));
        assert_eq!(*p.last().unwrap(), (40.0, 300.0));
        assert!(p.iter().any(|q| (q.0 - (10.0 + 30.0 * q.1 / 300.0)).abs() > 3.0), "it zigzags");
    }

    #[test]
    fn lightning_goes_for_the_iron_mast() {
        let mut w = World::new_static();
        ground(&mut w, 400);
        for y in 330..400 {
            w.set(520, y, Mat::Iron);
        }
        let (x, y) = w.lightning_target(480, 100, 90);
        assert_eq!((x, y), (520, 330), "the mast, not the flat ground right under the cloud");
    }

    #[test]
    fn the_tool_strikes_where_it_is_told_and_does_what_lightning_does() {
        let mut w = World::new_static();
        ground(&mut w, 420);
        // A tree, a patch of sand and an iron rail with a long arm of iron.
        for y in 380..420 {
            w.set(200, y, Mat::Trunk);
        }
        for x in 400..440 {
            for y in 410..420 {
                w.set(x, y, Mat::Sand);
            }
        }
        for x in 600..800 {
            w.set(x, 419, Mat::Iron);
        }
        w.strike(200, 300);
        w.strike(420, 300);
        w.strike(620, 300);
        assert_eq!(w.bolts.len(), 3);
        assert!(w.bolts.iter().all(|b| b.ground && b.path.len() > 8));
        let end = w.bolts[0].path.last().unwrap();
        assert_eq!((end.0 as i32, end.1 as i32), (200, 380), "it hits the top of the tree under the pointer");
        assert!((370..390).any(|y| matches!(w.raw(200, y), Mat::Ember | Mat::Fire)), "the tree catches fire");
        assert!((410..420).any(|y| w.raw(420, y) == Mat::Glass), "sand fuses into a fulgurite");
        assert!(w.temp_c(790, 419) > 100.0, "the current runs along the iron: {}", w.temp_c(790, 419));
        let heated = (0..60).filter(|y| w.temp_c(205, 300 + y) > 500.0 || w.temp_c(195, 300 + y) > 500.0 || w.temp_c(200, 300 + y) > 500.0).count();
        assert!(heated > 0 || w.bolts[0].path.iter().any(|p| w.temp_c(p.0 as i32, p.1 as i32) > 500.0), "the channel heats the air");
    }

    #[test]
    fn a_click_deep_in_the_ground_strikes_the_surface_above_it() {
        let mut w = World::new_static();
        ground(&mut w, 420);
        w.strike(300, 500);
        let end = w.bolts[0].path.last().unwrap();
        assert_eq!((end.0 as i32, end.1 as i32), (300, 420), "lightning comes from the sky: it stops at the ground");
        assert!((440..H as i32).all(|y| w.raw(300, y) == Mat::Stone), "and leaves the rock below whole");
    }

    #[test]
    fn a_deep_icy_cloud_in_an_updraft_charges_up_and_throws_lightning() {
        let mut w = World::new_static();
        w.climate_fixed = Some(-10.0);
        w.reset_temps();
        w.flow.pin_uniform(0.0, -0.6);
        ground(&mut w, 470);
        for x in 300..600 {
            for y in 120..260 {
                w.set(x, y, Mat::Cloud);
            }
        }
        // Keep it a thick cloud: no rain, no drift (only the charge is under test).
        let (cells, temp) = (w.cells.clone(), w.temp.clone());
        let mut sep = None;
        for step in 0..4000 {
            w.cells.copy_from_slice(&cells);
            w.temp.copy_from_slice(&temp);
            w.storm_step();
            w.ticks = w.ticks.wrapping_add(1);
            if sep.is_none() && step == 40 {
                let top = w.charge_at(450, 125);
                let low = w.charge_at(450, 250);
                sep = Some((top, low));
            }
            if !w.bolts.is_empty() {
                eprintln!("first bolt after {step} steps");
                assert!(step > 100, "charge takes a while to build up: {step} steps");
                break;
            }
        }
        let (top, low) = sep.unwrap();
        assert!(top > 0.0 && low < 0.0, "positive at the top, negative at the base: {top} / {low}");
        assert!(!w.bolts.is_empty(), "sooner or later the air breaks down");
        let b = &w.bolts[0];
        let (x0, y0) = b.path[0];
        assert!((290.0..610.0).contains(&x0) && (110.0..270.0).contains(&y0), "it starts in the cloud: {x0}, {y0}");
    }

    #[test]
    fn clear_sky_holds_no_charge() {
        let mut w = World::new_static();
        for _ in 0..200 {
            w.step();
        }
        assert!(w.charge.iter().all(|q| *q == 0.0));
        assert!(w.bolts.is_empty());
    }
}

#[cfg(test)]
mod anvil_lab {
    use super::*;

    fn dump(w: &World, path: &str) {
        let mut out = format!("P6\n{} {}\n255\n", W, H).into_bytes();
        for y in 0..H {
            for x in 0..W {
                let i = y * W + x;
                let c = match w.cells[i].m {
                    Mat::Cloud => [240, 240, 250],
                    Mat::Steam => [150, 160, 190],
                    Mat::Water => [40, 90, 220],
                    Mat::Snow | Mat::Hail | Mat::Ice => [200, 230, 255],
                    Mat::Empty => if y as i32 == CEILING_Y { [90, 40, 40] } else { [20, 24, 40] },
                    _ => [90, 80, 60],
                };
                out.extend_from_slice(&c);
            }
        }
        std::fs::write(path, out).unwrap();
    }

    #[test]
    #[ignore]
    fn anvil_over_a_warm_plain() {
        let dir = std::env::var("LAB").unwrap_or("/tmp".into());
        let mut w = World::new();
        w.clear();
        w.structures_fall = false;
        w.set_phase(std::env::var("PHASE").map(|v| v.parse().unwrap()).unwrap_or(0.2));
        if let Ok(v) = std::env::var("WIND") {
            w.wind_max = v.parse().unwrap();
        }
        let real = std::env::var("REAL").is_ok();
        if real {
            w.generate_biome(2, Biome::Countryside);
            w.set_phase(std::env::var("PHASE").map(|v| v.parse().unwrap()).unwrap_or(0.2));
        }
        for x in 0..W as i32 {
            for y in 420..H as i32 {
                if real {
                    break;
                }
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 300..660 {
            for y in 405..420 {
                if !real {
                    w.set(x, y, Mat::Water);
                }
            }
        }
        for y in (400..420).filter(|_| !real) {
            w.set(298, y, Mat::Stone);
            w.set(299, y, Mat::Stone);
            w.set(660, y, Mat::Stone);
            w.set(661, y, Mat::Stone);
        }
        w.reset_temps();
        for step in 0..3001 {
            // A hot summer lake: the water is kept warm, it steams.
            if step % 8 == 0 {
                for x in 300..660 {
                    for y in 405..420 {
                        let i = y * W + x;
                        if w.cells[i].m == Mat::Water {
                            w.temp[i] = w.temp[i].max(45.0);
                        }
                    }
                }
            }
            // A strong source of damp, warm air rising off the plain.
            if let Ok(rate) = std::env::var("SRC") {
                let rate: u32 = rate.parse().unwrap();
                for k in 0..rate {
                    let x = 600 + ((step as u32 * 7919 + k * 104729) % 120) as i32;
                    let ground = (0..H as i32).find(|&y| !matches!(w.at(x, y).kind(), Kind::Empty | Kind::Gas)).unwrap_or(H as i32);
                    let y = ground - 2 - ((step as u32 + k * 13) % 6) as i32;
                    if w.raw(x, y) == Mat::Empty {
                        w.set(x, y, Mat::Steam);
                        w.temp[y as usize * W + x as usize] = 32.0;
                    }
                }
            }
            w.step();
            if step % 250 == 0 {
                dump(&w, &format!("{dir}/anvil_{step:05}.ppm"));
                let n = w.cells.iter().filter(|c| c.m == Mat::Cloud).count();
                let top = (0..W * H).find(|&i| w.cells[i].m == Mat::Cloud).map(|i| i / W);
                let c = |m: Mat| w.cells.iter().filter(|c| c.m == m).count();
                let v: Vec<String> = [40, 60, 72, 100, 200, 390].iter().map(|&y| format!("{y}:{:.2}", w.flow.sample(480, y).0)).collect();
                eprintln!("  flow vx {}", v.join(" "));
                let anv: Vec<usize> = (0..80 * W).filter(|&i| w.cells[i].m == Mat::Cloud).map(|i| i % W).collect();
                if !anv.is_empty() {
                    let mean = anv.iter().sum::<usize>() as f32 / anv.len() as f32;
                    eprintln!("  anvil n {} mean x {mean:.0} from {} to {}", anv.len(), anv.iter().min().unwrap(), anv.iter().max().unwrap());
                }
                eprintln!("step {step}: wind {:.2}, clouds {n}, top row {top:?}, water {} steam {} snow {} ice {}", w.wind, c(Mat::Water), c(Mat::Steam), c(Mat::Snow), c(Mat::Ice));
            }
        }
        dump(&w, &format!("{dir}/anvil_end.ppm"));
    }
}
