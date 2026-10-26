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
#[allow(non_snake_case)]
pub fn QW() -> usize {
    W() / QS
}
#[allow(non_snake_case)]
pub fn QH() -> usize {
    H() / QS
}
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
/// Charge in the ground at which the air above it breaks down: a bolt leaves for it.
pub const GROUND_BREAKDOWN: f32 = 1.0;
/// How strongly a charged cloud draws the opposite charge up into what lies under it.
const INDUCE: f32 = 0.6;
/// A charged square of ground draws at most this share of its charge, the other way, into the
/// matter within `INFLUENCE_R` squares round it.
const INFLUENCE: f32 = 1.0;
const INFLUENCE_R: i32 = 3;

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
        Mat::Charcoal | Mat::Ember => 0.5,
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
        let (qx, qy) = ((x.clamp(0, W() as i32 - 1) as usize) / QS, (y.clamp(0, H() as i32 - 1) as usize) / QS);
        self.charge[qy * QW() + qx]
    }

    pub(super) fn storm_step(&mut self) {
        if self.ticks % STORM_EVERY != 0 {
            return;
        }
        // Cloud in each square, and how cold it is.
        // Also, for the charge in the ground: whether a square holds matter, how well the best of
        // it conducts, and whether it has air in it.
        let mut cloud = vec![0.0f32; QW() * QH()];
        let mut cold = vec![0.0f32; QW() * QH()];
        let mut cond = vec![-1.0f32; QW() * QH()];
        let mut open = vec![false; QW() * QH()];
        // And whether it holds loose grains (which a field can pull), and iron filings (rust).
        let mut grains = vec![false; QW() * QH()];
        let mut rusty = vec![false; QW() * QH()];
        for qy in 0..QH() {
            for qx in 0..QW() {
                let (mut n, mut t) = (0u32, 0.0f32);
                let k = qy * QW() + qx;
                for y in qy * QS..(qy + 1) * QS {
                    for x in qx * QS..(qx + 1) * QS {
                        let i = y * W() + x;
                        let m = self.cells[i].m;
                        if m == Mat::Cloud {
                            n += 1;
                            t += self.temp[i];
                        }
                        match m.kind() {
                            Kind::Empty | Kind::Gas | Kind::Flame => open[k] = true,
                            kind => {
                                cond[k] = cond[k].max(conductivity(m));
                                grains[k] |= kind == Kind::Powder;
                                rusty[k] |= m == Mat::Rust;
                            }
                        }
                    }
                }
                cloud[k] = n as f32 / (QS * QS) as f32;
                cold[k] = if n > 0 && t / (n as f32) < ICE_C { 1.0 } else { 0.0 };
            }
        }
        self.magnet_step(&rusty);
        let any_charge = self.charge.iter().chain(&self.mcharge).any(|q| q.abs() > 1e-4);
        if !any_charge && !cloud.iter().zip(&cold).any(|(c, k)| *c > 0.5 && *k > 0.0) {
            return;
        }
        // 1. Separation: in thick, icy cloud with cloud under it (a deep cloud), driven by the
        //    updraft, the positive charge goes up to the top and the negative stays low.
        let dt = STORM_EVERY as f32;
        let mut q = std::mem::take(&mut self.charge);
        for qy in 3..QH() - 1 {
            for qx in 0..QW() {
                let k = qy * QW() + qx;
                if cloud[k] < 0.5 || cold[k] == 0.0 {
                    continue;
                }
                let deep = (1..=4).filter(|d| qy + d < QH() && cloud[(qy + d) * QW() + qx] > 0.4).count() as f32 / 4.0;
                if deep < 0.5 {
                    continue;
                }
                let (_, vy) = self.flow.sample((qx * QS + QS / 2) as i32, (qy * QS + QS / 2) as i32);
                let updraft = 0.3 + (-vy).clamp(0.0, 2.0);
                let dq = SEPARATION * dt * cloud[k] * deep * updraft;
                // The crystals are lifted a few squares, as far as the cloud goes.
                let mut top = qy;
                while top > qy.saturating_sub(4) && top > 0 && cloud[(top - 1) * QW() + qx] > 0.3 {
                    top -= 1;
                }
                q[top * QW() + qx] += dq;
                q[(qy + 1).min(QH() - 1) * QW() + qx] -= dq;
            }
        }
        // 2. The wind carries the charge along (semi-Lagrangian: each square takes what was upwind
        //    of it, interpolated), and it leaks away, quickly out of the cloud.
        let mut moved = vec![0.0f32; QW() * QH()];
        for qy in 0..QH() {
            for qx in 0..QW() {
                let (cx, cy) = ((qx * QS + QS / 2) as i32, (qy * QS + QS / 2) as i32);
                // Only sideways: up and down, the charge sits where the sorting of the ice put it
                // (the crystals ride the updraft to the top, the graupel falls against it).
                let (vx, _) = self.flow.sample(cx, cy);
                let sx = (qx as f32 - vx * dt / QS as f32).clamp(0.0, QW() as f32 - 1.001);
                let sy = qy as f32;
                let (i, j) = (sx as usize, sy as usize);
                let (tx, ty) = (sx - i as f32, sy - j as f32);
                let at = |a: usize, b: usize| q[b.min(QH() - 1) * QW() + a.min(QW() - 1)];
                let v = (at(i, j) * (1.0 - tx) + at(i + 1, j) * tx) * (1.0 - ty) + (at(i, j + 1) * (1.0 - tx) + at(i + 1, j + 1) * tx) * ty;
                let k = qy * QW() + qx;
                let keep = if cloud[k] > 0.2 { 0.9985 } else { 0.97 };
                moved[k] = v * keep;
            }
        }
        // (Charge can neither be made nor lost by the carrying: what two squares took from the
        // same source is a small error the leak soon hides.)
        q = moved;
        self.charge = q;
        // 2b. The ground under a charged cloud takes the opposite charge, most where it stands
        //     close to the cloud and conducts; the charge spreads along conductors and drains away.
        let exposed = self.ground_charge_step(&cond, &open);
        // Charged grains are pulled toward the opposite charge and pushed from the same.
        self.pull_charged_grains(&grains);
        // The air between the ground and the cloud breaks down where the ground is most charged.
        if self.ground_breakdown(&exposed) {
            return;
        }
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
        let (qx, qy) = (at % QW(), at / QW());
        let (x0, y0) = ((qx * QS + QS / 2) as i32, (qy * QS + QS / 2) as i32);
        let negative = self.charge[at] < 0.0;
        let to_ground = negative && self.storm_rng.next() % 10 < 6;
        if to_ground {
            // From the base of the cloud, down to the best spot under it.
            let mut base = y0;
            while base < H() as i32 - 1 && matches!(self.raw(x0, base + 1), Mat::Cloud | Mat::Snow | Mat::Hail) {
                base += 1;
            }
            let target = self.lightning_target(x0, base, 90);
            self.discharge_around(qx, qy, 5);
            self.lightning_bolt((x0 as f32, base as f32), None, target, best.min(3.0) / 3.0, true);
        } else {
            // Inside the cloud: to the strongest charge of the other sign near it.
            let (mut other, mut ok) = (at, 0.0f32);
            for dy in -12i32..=12 {
                for dx in -16i32..=16 {
                    let (ox, oy) = (qx as i32 + dx, qy as i32 + dy);
                    if ox < 0 || oy < 0 || ox >= QW() as i32 || oy >= QH() as i32 {
                        continue;
                    }
                    let k = oy as usize * QW() + ox as usize;
                    let v = -self.charge[k] * self.charge[at].signum();
                    if v > ok {
                        ok = v;
                        other = k;
                    }
                }
            }
            let (ox, oy) = ((other % QW() * QS + QS / 2) as f32, (other / QW() * QS + QS / 2) as f32);
            self.discharge_around(qx, qy, 4);
            self.discharge_around(other % QW(), other / QW(), 4);
            let (a, b) = if oy < y0 as f32 { ((ox, oy), (x0 as f32, y0 as f32)) } else { ((x0 as f32, y0 as f32), (ox, oy)) };
            self.lightning_bolt(a, None, (b.0 as i32, b.1 as i32), best.min(3.0) / 3.0 * 0.6, false);
        }
    }

    /// The charge around a square drains through the bolt.
    fn discharge_around(&mut self, qx: usize, qy: usize, r: i32) {
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (qx as i32 + dx, qy as i32 + dy);
                if x >= 0 && y >= 0 && x < QW() as i32 && y < QH() as i32 {
                    self.charge[y as usize * QW() + x as usize] *= 0.15;
                }
            }
        }
    }

    /// Where a bolt coming down from (x0, y0) strikes: the point within `reach` columns that
    /// stands highest, conducts best and is nearest. Returns the first solid or liquid cell.
    pub fn lightning_target(&mut self, x0: i32, y0: i32, reach: i32) -> (i32, i32) {
        let mut best = (x0.clamp(0, W() as i32 - 1), H() as i32 - 1);
        let mut score = f32::MIN;
        let mut x = (x0 - reach).max(0);
        while x <= (x0 + reach).min(W() as i32 - 1) {
            if let Some(y) = self.first_hit(x, y0) {
                let m = self.raw(x, y);
                let s = -(y as f32) + 40.0 * conductivity(m) + 40.0 * self.mcharge_at(x, y).abs() - (x - x0).abs() as f32 * 0.35 + self.storm_rng.f() * 6.0;
                if s > score {
                    score = s;
                    best = (x, y);
                }
            }
            x += if reach <= 12 { 1 } else { 2 };
        }
        best
    }

    /// The first cell under (x, y) that lightning can strike (not air, cloud or smoke).
    fn first_hit(&self, x: i32, y: i32) -> Option<i32> {
        (y.max(0)..H() as i32).find(|&yy| {
            let m = self.raw(x, yy);
            match m.kind() {
                Kind::Empty | Kind::Gas | Kind::Flame => false,
                _ => !(matches!(m, Mat::Snow | Mat::Hail) && yy < GROUND_ROW()),
            }
        })
    }

    /// Lightning from the sky where the player clicked: from the cloud above (or the top of the
    /// sky) down to the pointer. It cannot go through the ground: under the surface it strikes the
    /// surface above. Released in the open air, it does not stop there: it jumps on, like a bolt,
    /// to the nearest place where it can reach the ground.
    pub fn strike(&mut self, x: i32, y: i32) {
        let x = x.clamp(0, W() as i32 - 1);
        let y = y.clamp(0, H() as i32 - 1);
        let surface = self.first_hit(x, 0).unwrap_or(H() as i32 - 1);
        let aim = y.min(surface);
        // From the base of the cloud above (the first cloud from the top, down through it), or
        // else from the top of the picture. A wisp of cloud just above the pointer does not count.
        let mut top = (0..aim).find(|&yy| self.raw(x, yy) == Mat::Cloud).unwrap_or(0);
        while top + 1 < aim && self.raw(x, top + 1) == Mat::Cloud {
            top += 1;
        }
        if aim - top < 60 {
            top = 0;
        }
        let start = ((x as f32 + self.storm_rng.f() * 30.0).clamp(0.0, W() as f32 - 1.0), top as f32);
        if y < surface {
            let target = self.nearest_ground(x, y, 60);
            self.lightning_bolt(start, Some((x as f32, y as f32)), target, 1.0, true);
        } else {
            self.lightning_bolt(start, None, (x, surface), 1.0, true);
        }
    }

    /// Where a bolt left hanging in the air at (x, y) goes on to: the nearest thing under it
    /// within `reach` columns, conductors first.
    fn nearest_ground(&mut self, x0: i32, y0: i32, reach: i32) -> (i32, i32) {
        let mut best = (x0, self.first_hit(x0, y0).unwrap_or(H() as i32 - 1));
        let mut score = f32::MIN;
        for x in ((x0 - reach).max(0)..=(x0 + reach).min(W() as i32 - 1)).step_by(2) {
            if let Some(y) = self.first_hit(x, y0) {
                let d = (((x - x0) * (x - x0) + (y - y0) * (y - y0)) as f32).sqrt();
                let s = -d - 0.5 * (x - x0).abs() as f32 + 30.0 * conductivity(self.raw(x, y)) + self.storm_rng.f() * 2.0;
                if s > score {
                    score = s;
                    best = (x, y);
                }
            }
        }
        best
    }

    /// Builds a bolt from `a` down to `b` (by way of `via`), heats its whole channel and, if it
    /// reached the ground, strikes what is there and sends the current into the ground.
    fn lightning_bolt(&mut self, a: (f32, f32), via: Option<(f32, f32)>, b: (i32, i32), power: f32, ground: bool) {
        let mut rng = self.storm_rng.clone();
        let end = (b.0 as f32, b.1 as f32);
        let len = ((end.0 - a.0).powi(2) + (end.1 - a.1).powi(2)).sqrt();
        let depth = (len.max(8.0).log2() as u32).clamp(3, 8);
        let mut path = match via {
            Some(v) => {
                let first = ((v.0 - a.0).powi(2) + (v.1 - a.1).powi(2)).sqrt();
                let second = ((end.0 - v.0).powi(2) + (end.1 - v.1).powi(2)).sqrt();
                let mut p = jagged(a, v, (first.max(8.0).log2() as u32).clamp(3, 8), 0.32, &mut rng);
                p.extend(jagged(v, end, (second.max(8.0).log2() as u32).clamp(2, 7), 0.4, &mut rng).into_iter().skip(1));
                p
            }
            None => jagged(a, end, depth, 0.32, &mut rng),
        };
        path.dedup();
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
        // The whole channel is heated in an instant: the air in it to thousands of degrees (the
        // heat haze and the glow follow), and whatever it goes through is scorched and set alight.
        for w in path.windows(2) {
            let (p, q) = (w[0], w[1]);
            let n = ((q.0 - p.0).abs().max((q.1 - p.1).abs()).ceil() as i32).max(1);
            for s in 0..=n {
                let f = s as f32 / n as f32;
                let (x, y) = ((p.0 + (q.0 - p.0) * f) as i32, (p.1 + (q.1 - p.1) * f) as i32);
                if !Self::inb(x, y) {
                    continue;
                }
                let i = Self::idx(x, y);
                match self.raw(x, y).kind() {
                    Kind::Empty => self.temp[i] = self.temp[i].max(CHANNEL_C * power.max(0.4)),
                    Kind::Gas | Kind::Flame => {}
                    _ if ground => {
                        self.temp[i] += 500.0 * power.max(0.5);
                        self.lightning_burn(x, y, true);
                    }
                    _ => {}
                }
                self.zones.wake(i);
            }
        }
        if ground {
            self.lightning_hit(b.0, b.1, power);
            let current = self.ground_current(b.0, b.1, power);
            branches.extend(current);
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
                self.lightning_burn(cx, cy, d2 <= 2);
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

impl World {
    /// What the heat of lightning does to the cell (x, y): wood, grass and leaves catch fire,
    /// charcoal starts to glow, oil flares up, powder and dynamite are brought to the point of
    /// going off. `sure`: right in the channel, it always catches; a little off it, mostly.
    fn lightning_burn(&mut self, x: i32, y: i32, sure: bool) {
        let m = self.raw(x, y);
        let catches = sure || self.storm_rng.next() % 3 != 0;
        let i = Self::idx(x, y);
        if m.is_wooden() || matches!(m, Mat::Grass | Mat::DeadLeaf | Mat::Sapling) {
            if catches {
                self.set(x, y, if m == Mat::Grass || m == Mat::DeadLeaf { Mat::Fire } else { Mat::Ember });
            }
        } else if m == Mat::Charcoal {
            if catches {
                self.set(x, y, Mat::Ember);
                self.fuel[i] = Fuel { left: 30 + (self.storm_rng.next() % 26) as u8, carbon: true };
            }
        } else if m == Mat::Oil {
            self.ignite_oil(x, y);
        } else if matches!(m, Mat::Gunpowder | Mat::Dynamite) {
            self.temp[i] = self.temp[i].max(1000.0);
        }
    }

    /// The current of a bolt that struck (x, y) goes into the ground. Struck on something that
    /// does not conduct (stone, glass, brick), it first jumps across to the nearest thing that does.
    /// Then it runs down, always into the neighbour that conducts best, heating what it goes
    /// through (the worse the conductor, the hotter): sand fuses into a tube of glass (a
    /// fulgurite), wood and charcoal catch fire. Metal it touches carries it all along, heating
    /// up. Returns the lines it drew, for the picture: the jump, then the path in the ground.
    fn ground_current(&mut self, x: i32, y: i32, power: f32) -> Vec<Vec<(f32, f32)>> {
        let mut lines = vec![];
        if !Self::inb(x, y) {
            return lines;
        }
        let (mut cx, mut cy) = (x, y);
        if conductivity(self.raw(x, y)) == 0.0 {
            let mut best: Option<(i32, i32)> = None;
            let mut bd = i32::MAX;
            for dy in -20i32..=20 {
                for dx in -20i32..=20 {
                    let (nx, ny) = (x + dx, y + dy);
                    let d = dx * dx + dy * dy;
                    if d < bd && d <= 400 && Self::inb(nx, ny) && conductivity(self.raw(nx, ny)) > 0.0 {
                        bd = d;
                        best = Some((nx, ny));
                    }
                }
            }
            if let Some((nx, ny)) = best {
                let mut rng = self.storm_rng.clone();
                lines.push(jagged((x as f32, y as f32), (nx as f32, ny as f32), 3, 0.4, &mut rng));
                self.storm_rng = rng;
                (cx, cy) = (nx, ny);
            }
        }
        let mut seen = std::collections::HashSet::new();
        seen.insert((cx, cy));
        let mut trail = vec![(cx as f32, cy as f32)];
        let mut metal = vec![];
        let deep = 20 + (power * 40.0) as i32;
        // Like the bolt itself, the current zigzags: it leans to one side for a few cells, then
        // to the other.
        let mut side = if self.storm_rng.next() % 2 == 0 { 1 } else { -1 };
        let mut lean = 4 + self.storm_rng.next() % 9;
        for _ in 0..deep {
            if lean == 0 {
                side = -side;
                lean = 4 + self.storm_rng.next() % 9;
            }
            lean -= 1;
            let here = self.raw(cx, cy);
            let k = conductivity(here);
            let i = Self::idx(cx, cy);
            self.temp[i] += 350.0 * power.max(0.5) * (1.2 - k);
            self.zones.wake(i);
            match here {
                Mat::Sand => self.set(cx, cy, Mat::Glass),
                Mat::Iron | Mat::Steel => metal.push((cx, cy)),
                _ => self.lightning_burn(cx, cy, true),
            }
            // Down, leaning to its side: every solid or liquid conducts a little at the voltage of
            // lightning, conductors much better.
            let mut next = None;
            let mut ns = f32::MIN;
            for (dx, dy, bias) in [(side, 1, 0.65), (0, 1, 0.45), (side, 0, 0.3), (-side, 1, 0.15), (-side, 0, -0.2)] {
                let (nx, ny) = (cx + dx, cy + dy);
                if !Self::inb(nx, ny) || seen.contains(&(nx, ny)) {
                    continue;
                }
                let m = self.raw(nx, ny);
                if matches!(m.kind(), Kind::Empty | Kind::Gas | Kind::Flame) {
                    continue;
                }
                let s = conductivity(m) + bias + 0.2 * (self.storm_rng.f() * 0.5 + 0.5);
                if s > ns {
                    ns = s;
                    next = Some((nx, ny));
                }
            }
            let Some(n) = next else { break };
            seen.insert(n);
            (cx, cy) = n;
            trail.push((cx as f32, cy as f32));
        }
        // The bolt leaves its charge (negative, from the base of the cloud) where its current went.
        let mut squares: Vec<usize> = trail.iter().map(|p| (p.1 as usize / QS).min(QH() - 1) * QW() + (p.0 as usize / QS).min(QW() - 1)).collect();
        squares.dedup();
        for k in squares {
            self.mcharge[k] = (self.mcharge[k] - 0.6 * power.max(0.5)).max(-1.5);
        }
        if trail.len() > 1 {
            lines.push(trail);
        }
        // Metal carries the current: everything of it that touches is heated.
        let mut stack = metal;
        let mut done: std::collections::HashSet<(i32, i32)> = stack.iter().copied().collect();
        while let Some((mx, my)) = stack.pop() {
            let i = Self::idx(mx, my);
            self.temp[i] += 250.0 * power.max(0.5);
            // The current through iron magnetises it (lightning makes magnets).
            let q = (my as usize / QS) * QW() + mx as usize / QS;
            self.magnet[q] = self.magnet[q].max(power.max(0.5));
            self.zones.wake(i);
            if done.len() > 1500 {
                continue;
            }
            for (nx, ny) in Self::neighbors(mx, my) {
                if Self::inb(nx, ny) && matches!(self.raw(nx, ny), Mat::Iron | Mat::Steel) && done.insert((nx, ny)) {
                    stack.push((nx, ny));
                }
            }
        }
        lines
    }
}

impl World {
    /// Charge held in the ground at the square of cell (x, y).
    pub fn mcharge_at(&self, x: i32, y: i32) -> f32 {
        let (qx, qy) = ((x.clamp(0, W() as i32 - 1) as usize) / QS, (y.clamp(0, H() as i32 - 1) as usize) / QS);
        self.mcharge[qy * QW() + qx]
    }

    /// One step of the charge in the ground. `cond`: how well the best matter of each square
    /// conducts (-1: no matter); `open`: the square has air in it. Returns which squares are
    /// exposed: matter with air in or just above it (only those feel the cloud).
    fn ground_charge_step(&mut self, cond: &[f32], open: &[bool]) -> Vec<bool> {
        let mut exposed = vec![false; QW() * QH()];
        for qy in 0..QH() {
            for qx in 0..QW() {
                let k = qy * QW() + qx;
                exposed[k] = cond[k] >= 0.0 && (open[k] || (qy > 0 && open[k - QW()]));
            }
        }
        let clouds: Vec<(i32, i32, f32)> = self
            .charge
            .iter()
            .enumerate()
            .filter(|(_, q)| q.abs() > 0.02)
            .map(|(k, q)| ((k % QW()) as i32, (k / QW()) as i32, *q))
            .collect();
        if clouds.is_empty() && self.mcharge.iter().all(|q| q.abs() < 1e-4) {
            self.polar.fill(0.0);
            return exposed;
        }
        // Conductors share their charge with their neighbours, quickly (several passes).
        for _ in 0..4 {
            let m = self.mcharge.clone();
            for qy in 0..QH() {
                for qx in 0..QW() {
                    let k = qy * QW() + qx;
                    if cond[k] <= 0.0 {
                        continue;
                    }
                    let mut flow = 0.0;
                    for (nx, ny) in [(qx as i32 - 1, qy as i32), (qx as i32 + 1, qy as i32), (qx as i32, qy as i32 - 1), (qx as i32, qy as i32 + 1)] {
                        if nx < 0 || ny < 0 || nx >= QW() as i32 || ny >= QH() as i32 {
                            continue;
                        }
                        let n = ny as usize * QW() + nx as usize;
                        if cond[n] > 0.0 {
                            flow += 0.22 * cond[k].min(cond[n]) * (m[n] - m[k]);
                        }
                    }
                    self.mcharge[k] += flow;
                }
            }
        }
        // Influence: charge in the ground draws the opposite charge into the matter round it. Each
        // charged square spreads the pull of its charge over its neighbourhood (normalised, so it
        // gathers at most a share `INFLUENCE` of its own charge, the most close by). It shows in
        // what conducts poorly (along a conductor the charge simply spreads out), at once, and is
        // gone as soon as the charge that draws it is.
        let mut influence = vec![0.0f32; QW() * QH()];
        let sources: Vec<(i32, i32, f32)> = self
            .mcharge
            .iter()
            .enumerate()
            .filter(|(_, q)| q.abs() > 0.03)
            .map(|(k, q)| ((k % QW()) as i32, (k / QW()) as i32, *q))
            .collect();
        for &(sx, sy, q) in &sources {
            let mut total = 0.0;
            for dy in -INFLUENCE_R..=INFLUENCE_R {
                for dx in -INFLUENCE_R..=INFLUENCE_R {
                    let (x, y) = (sx + dx, sy + dy);
                    if (dx, dy) != (0, 0) && x >= 0 && y >= 0 && x < QW() as i32 && y < QH() as i32 && cond[y as usize * QW() + x as usize] >= 0.0 {
                        total += 1.0 / (1.0 + (dx * dx + dy * dy) as f32);
                    }
                }
            }
            if total == 0.0 {
                continue;
            }
            for dy in -INFLUENCE_R..=INFLUENCE_R {
                for dx in -INFLUENCE_R..=INFLUENCE_R {
                    let (x, y) = (sx + dx, sy + dy);
                    if (dx, dy) != (0, 0) && x >= 0 && y >= 0 && x < QW() as i32 && y < QH() as i32 {
                        let k = y as usize * QW() + x as usize;
                        // Only matter that holds no charge of its own takes it.
                        if cond[k] >= 0.0 && self.mcharge[k].abs() < 0.1 {
                            influence[k] -= INFLUENCE * q / (1.0 + (dx * dx + dy * dy) as f32) / total * (1.0 - cond[k]);
                        }
                    }
                }
            }
        }
        self.polar = influence;
        let m = &self.mcharge;
        let mut next = m.clone();
        for qy in 0..QH() {
            for qx in 0..QW() {
                let k = qy * QW() + qx;
                if cond[k] < 0.0 {
                    next[k] = 0.0;
                    continue;
                }
                // The field of the cloud charge over it: nearer charge counts much more.
                let mut target = 0.0;
                if exposed[k] {
                    let mut field = 0.0;
                    for &(cx, cy, q) in &clouds {
                        let (dx, dy) = (cx - qx as i32, cy - qy as i32);
                        if dy >= 0 || dx.abs() > 12 {
                            continue;
                        }
                        let d2 = (dx * dx + dy * dy) as f32;
                        field += q / (1.0 + d2 / 16.0);
                    }
                    target = -INDUCE * field * (0.5 + cond[k]);
                }
                // It follows the cloud's pull (or drains away without one), quicker in a conductor.
                let rate = 0.002 + 0.006 * cond[k];
                next[k] = m[k] + (target - m[k]) * rate;
            }
        }
        self.mcharge = next;
        exposed
    }

    /// Lets the ground take the charge the clouds draw up for `steps` storm steps, without any
    /// bolt (for a demo picture).
    pub fn settle_ground_charge(&mut self, steps: u32) {
        let mut cond = vec![-1.0f32; QW() * QH()];
        let mut open = vec![false; QW() * QH()];
        for y in 0..H() {
            for x in 0..W() {
                let k = (y / QS) * QW() + x / QS;
                let m = self.cells[y * W() + x].m;
                if matches!(m.kind(), Kind::Empty | Kind::Gas | Kind::Flame) {
                    open[k] = true;
                } else {
                    cond[k] = cond[k].max(conductivity(m));
                }
            }
        }
        for _ in 0..steps {
            self.ground_charge_step(&cond, &open);
            for q in &mut self.mcharge {
                *q = q.clamp(-0.95 * GROUND_BREAKDOWN, 0.95 * GROUND_BREAKDOWN);
            }
        }
    }

    /// When the charge somewhere in the ground passes `GROUND_BREAKDOWN` under a cloud charged the
    /// other way, a bolt leaves the cloud for it. True if one did.
    fn ground_breakdown(&mut self, exposed: &[bool]) -> bool {
        let (mut best, mut at) = (0.0f32, 0usize);
        for (k, v) in self.mcharge.iter().enumerate() {
            if exposed[k] && v.abs() > best {
                best = v.abs();
                at = k;
            }
        }
        if best < GROUND_BREAKDOWN {
            return false;
        }
        let (gx, gy) = (at % QW(), at / QW());
        let sign = self.mcharge[at].signum();
        // The cloud square above with the most charge of the other sign.
        let (mut from, mut fq) = (None, 0.3f32);
        for qy in 0..gy {
            for qx in gx.saturating_sub(6)..(gx + 7).min(QW()) {
                let v = -self.charge[qy * QW() + qx] * sign;
                if v > fq {
                    fq = v;
                    from = Some((qx, qy));
                }
            }
        }
        let Some((cx, cy)) = from else { return false };
        let x0 = (cx * QS + QS / 2) as i32;
        let mut base = (cy * QS + QS / 2) as i32;
        while base < H() as i32 - 1 && matches!(self.raw(x0, base + 1), Mat::Cloud | Mat::Snow | Mat::Hail) {
            base += 1;
        }
        let tx = (gx * QS + QS / 2) as i32;
        let target = self.lightning_target(tx, (gy * QS) as i32 - 1, QS as i32);
        self.discharge_around(cx, cy, 5);
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                let (x, y) = (gx as i32 + dx, gy as i32 + dy);
                if x >= 0 && y >= 0 && x < QW() as i32 && y < QH() as i32 {
                    self.mcharge[y as usize * QW() + x as usize] *= 0.1;
                }
            }
        }
        self.lightning_bolt((x0 as f32, base as f32), None, target, (fq.min(3.0) / 3.0).max(0.5), true);
        true
    }
}

impl World {
    /// The electric field at the centre of square (qx, qy), from the charge of the clouds and of
    /// the ground (each charge pushes away from itself, as 1/d²), in charge per square. With a
    /// `skip_d2` (squared squares), the sources that near, and those just under it, are left out.
    fn field_at(&self, qx: usize, qy: usize, sources: &[(i32, i32, f32)], skip_d2: i32) -> (f32, f32) {
        let (mut ex, mut ey) = (0.0, 0.0);
        for &(sx, sy, q) in sources {
            let (ix, iy) = (qx as i32 - sx, qy as i32 - sy);
            // Close by, and the ground right under it, are what the grain lies on: left out.
            if skip_d2 > 0 && (ix * ix + iy * iy <= skip_d2 || (iy < 0 && ix.abs() <= 4)) {
                continue;
            }
            let (dx, dy) = (ix as f32, iy as f32);
            let d2 = dx * dx + dy * dy + 1.0;
            let f = q / (d2 * d2.sqrt());
            ex += f * dx;
            ey += f * dy;
        }
        (ex, ey)
    }

    /// Opposite charges attract, like charges repel: loose grains in a charged square are pulled
    /// along the field (up toward a cloud of the other sign, away from a spot of the same).
    fn pull_charged_grains(&mut self, grains: &[bool]) {
        // The clouds, and the ground's own charge; a grain feels the ground's charge from a little
        // way off, not the layer it lies in. A grain's charge is its square's, or the charge a
        // charged spot nearby draws into it.
        let clouds: Vec<(i32, i32, f32)> = self
            .charge
            .iter()
            .enumerate()
            .filter(|(_, q)| q.abs() > 0.05)
            .map(|(k, q)| ((k % QW()) as i32, (k / QW()) as i32, *q))
            .collect();
        let ground: Vec<(i32, i32, f32)> = (0..QW() * QH())
            .map(|k| (k, self.mcharge[k]))
            .filter(|(_, q)| q.abs() > 0.05)
            .map(|(k, q)| ((k % QW()) as i32, (k / QW()) as i32, q))
            .collect();
        if clouds.is_empty() && ground.is_empty() {
            return;
        }
        for k in 0..QW() * QH() {
            let q = self.mcharge[k] + self.polar[k];
            if !grains[k] || q.abs() < 0.05 {
                continue;
            }
            let (qx, qy) = (k % QW(), k / QW());
            let (cx, cy) = self.field_at(qx, qy, &clouds, 0);
            let (gx, gy) = self.field_at(qx, qy, &ground, 8);
            let (fx, fy) = (q * (cx + gx), q * (cy + gy));
            let f = (fx * fx + fy * fy).sqrt();
            let p = (f * 40.0).min(0.6);
            if p > 0.02 {
                self.nudge_grains(qx, qy, (fx / f, fy / f), p, 1, |m| m.kind() == Kind::Powder);
            }
        }
    }

    /// Moves the grains of square (qx, qy) that `which` accepts, each with probability `p`, up to
    /// `reach` cells along `dir`, into empty air.
    fn nudge_grains(&mut self, qx: usize, qy: usize, dir: (f32, f32), p: f32, reach: i32, which: impl Fn(Mat) -> bool) {
        let step = |v: f32, r: f32| if v.abs() > r { v.signum() as i32 } else { 0 };
        for y in qy * QS..(qy + 1) * QS {
            for x in qx * QS..(qx + 1) * QS {
                let (mut cx, mut cy) = (x as i32, y as i32);
                if !which(self.raw(cx, cy)) || (self.storm_rng.next() % 1000) as f32 >= p * 1000.0 {
                    continue;
                }
                for _ in 0..reach {
                    // Mostly along the larger component, now and then the smaller.
                    let r = (self.storm_rng.next() % 1000) as f32 / 1000.0;
                    let (dx, dy) = (step(dir.0, r), step(dir.1, r));
                    let (nx, ny) = (cx + dx, cy + dy);
                    if (dx, dy) == (0, 0) || !Self::inb(nx, ny) || self.raw(nx, ny) != Mat::Empty {
                        break;
                    }
                    self.swap((cx, cy), (nx, ny));
                    (cx, cy) = (nx, ny);
                }
            }
        }
    }

    /// Magnetised iron pulls iron filings (rust) toward it; its magnetism fades very slowly.
    fn magnet_step(&mut self, rusty: &[bool]) {
        let magnets: Vec<(i32, i32, f32)> = self
            .magnet
            .iter()
            .enumerate()
            .filter(|(_, m)| **m > 0.05)
            .map(|(k, m)| ((k % QW()) as i32, (k / QW()) as i32, *m))
            .collect();
        if magnets.is_empty() {
            return;
        }
        for v in &mut self.magnet {
            *v *= 0.9998;
        }
        for k in 0..QW() * QH() {
            if !rusty[k] {
                continue;
            }
            let (qx, qy) = (k % QW(), k / QW());
            let (mut fx, mut fy) = (0.0f32, 0.0f32);
            for &(mx, my, s) in &magnets {
                let (dx, dy) = ((mx - qx as i32) as f32, (my - qy as i32) as f32);
                let d2 = dx * dx + dy * dy;
                if d2 > MAGNET_R * MAGNET_R {
                    continue;
                }
                let f = s / ((d2 + 1.0) * (d2 + 1.0).sqrt());
                fx += f * dx;
                fy += f * dy;
            }
            let f = (fx * fx + fy * fy).sqrt();
            if f > 0.002 {
                let p = (f * 6.0).min(0.9);
                self.nudge_grains(qx, qy, (fx / f, fy / f), p, 1 + (f * 8.0).min(3.0) as i32, |m| m == Mat::Rust);
            }
        }
    }

    /// Filings touching magnetised iron (or a filing stuck to it) cling there.
    pub(super) fn magnet_holds(&self, x: i32, y: i32) -> bool {
        let (qx, qy) = (x as usize / QS, y as usize / QS);
        let mut strong = 0.0f32;
        for j in qy.saturating_sub(1)..(qy + 2).min(QH()) {
            for i in qx.saturating_sub(1)..(qx + 2).min(QW()) {
                strong = strong.max(self.magnet[j * QW() + i]);
            }
        }
        if strong <= 0.1 {
            return false;
        }
        let iron = |w: &World, (nx, ny): (i32, i32)| matches!(w.at(nx, ny), Mat::Iron | Mat::Steel);
        // On the iron itself, or on a filing that is (a short beard of filings).
        Self::neighbors(x, y).iter().any(|&n| iron(self, n) || (self.at(n.0, n.1) == Mat::Rust && Self::neighbors(n.0, n.1).iter().any(|&nn| iron(self, nn))))
    }
}

/// How far (in squares) a magnet reaches.
const MAGNET_R: f32 = 8.0;

/// Lightning goes into snow lying on the ground (low down), not into falling flakes.
#[allow(non_snake_case)]
fn GROUND_ROW() -> i32 {
    (H() as i32) / 2
}

impl World {
    /// Charge in the ground at a cell, smoothly interpolated between the squares of the grid.
    pub fn mcharge_smooth(&self, x: usize, y: usize) -> f32 {
        smooth(&self.mcharge, x, y) + smooth(&self.polar, x, y)
    }

    /// Charge at a cell, smoothly interpolated between the squares of the grid.
    pub fn charge_smooth(&self, x: usize, y: usize) -> f32 {
        smooth(&self.charge, x, y)
    }
}

/// A value of a `QS` grid at cell (x, y), smoothly interpolated between the squares.
fn smooth(grid: &[f32], x: usize, y: usize) -> f32 {
    let fx = ((x as f32 + 0.5) / QS as f32 - 0.5).clamp(0.0, QW() as f32 - 1.001);
    let fy = ((y as f32 + 0.5) / QS as f32 - 0.5).clamp(0.0, QH() as f32 - 1.001);
    let (i, j) = (fx as usize, fy as usize);
    let (tx, ty) = (fx - i as f32, fy - j as f32);
    let q = |a: usize, b: usize| grid[b * QW() + a];
    (q(i, j) * (1.0 - tx) + q(i + 1, j) * tx) * (1.0 - ty) + (q(i, j + 1) * (1.0 - tx) + q(i + 1, j + 1) * tx) * ty
}

impl World {
    /// The electricity view: the world dimmed to grey, what conducts lit up (metal gold, water
    /// and wet things cyan), and the charge over it, red where positive and blue where negative,
    /// brightest close to breaking down: in the clouds, and in the ground and what stands on it
    /// (drawn up under a charged cloud, or left by a bolt).
    pub fn electric_color(&self, x: usize, y: usize) -> [u8; 4] {
        let m = self.cells[y * W() + x].m;
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
        // Magnetised iron (and the filings it holds) shows violet.
        if matches!(m, Mat::Iron | Mat::Steel | Mat::Rust) {
            let g = self.magnet[(y / QS) * QW() + x / QS].min(1.0);
            if g > 0.05 {
                for (v, t) in c.iter_mut().zip([190.0, 90.0, 255.0]) {
                    *v += (t - *v) * g * 0.8;
                }
            }
        }
        let matter = !matches!(m.kind(), Kind::Empty | Kind::Gas | Kind::Flame);
        let q = if matter { self.mcharge_smooth(x, y) / GROUND_BREAKDOWN } else { self.charge_smooth(x, y) / BREAKDOWN };
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
        for x in 0..W() as i32 {
            for y in top..H() as i32 {
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
    fn lightning_lights_charcoal_and_its_current_goes_into_the_ground() {
        let mut w = World::new_static();
        ground(&mut w, 420);
        for x in 300..340 {
            for y in 400..420 {
                w.set(x, y, Mat::Charcoal);
            }
        }
        w.strike(320, 410);
        assert!((395..425).any(|y| (315..326).any(|x| w.raw(x, y) == Mat::Ember)), "the charcoal glows");
        // The current went on down into the ground and drew its path there.
        let b = &w.bolts[0];
        let deepest = b.branches.iter().flatten().map(|p| p.1).fold(0.0f32, f32::max);
        assert!(deepest > 425.0, "the current runs down into the ground: {deepest}");
    }

    #[test]
    fn clicked_under_ground_it_strikes_the_surface_above() {
        let mut w = World::new_static();
        ground(&mut w, 400);
        for x in 480..500 {
            w.set(x, 400, Mat::Dirt);
        }
        w.strike(490, 470);
        let end = w.bolts[0].path.last().unwrap();
        assert_eq!((end.0 as i32, end.1 as i32), (490, 400), "lightning does not go through rock");
    }

    #[test]
    fn released_in_the_air_it_jumps_on_to_the_nearest_conductor() {
        let mut w = World::new_static();
        ground(&mut w, 420);
        // An iron post a little to the side of the pointer.
        for y in 380..420 {
            w.set(530, y, Mat::Iron);
        }
        w.strike(500, 340);
        let b = &w.bolts[0];
        assert!(b.path.iter().any(|p| (p.0 - 500.0).abs() < 1.0 && (p.1 - 340.0).abs() < 1.0), "it goes through the pointer");
        let end = b.path.last().unwrap();
        assert_eq!((end.0 as i32, end.1 as i32), (530, 380), "then on to the top of the iron post");
        assert!(w.temp_c(530, 418) > 100.0, "and the current runs down the post: {}", w.temp_c(530, 418));
    }

    #[test]
    fn struck_on_bare_rock_the_current_jumps_to_wet_ground_nearby() {
        let mut w = World::new_static();
        ground(&mut w, 420);
        for x in 610..640 {
            for y in 420..440 {
                w.set(x, y, Mat::Mud);
            }
        }
        w.strike(600, 425);
        let b = &w.bolts[0];
        let reached = b.branches.iter().flatten().any(|p| p.0 >= 609.0);
        assert!(reached, "the current looks for the mud");
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
    fn a_charged_cloud_draws_charge_up_the_mast_until_lightning_leaves_for_it() {
        let mut w = World::new_static();
        w.climate_fixed = Some(-10.0);
        w.reset_temps();
        ground(&mut w, 470);
        for y in 380..470 {
            w.set(450, y, Mat::Iron);
        }
        for x in 300..600 {
            for y in 120..260 {
                w.set(x, y, Mat::Cloud);
            }
        }
        // A charged cloud: positive at the top, negative at the base.
        for qy in 0..QH() {
            for qx in 0..QW() {
                let (x, y) = (qx * QS, qy * QS);
                if (300..600).contains(&x) {
                    w.charge[qy * QW() + qx] = if (120..170).contains(&y) { 0.5 } else if (200..260).contains(&y) { -0.6 } else { 0.0 };
                }
            }
        }
        let (cells, temp, charge) = (w.cells.clone(), w.temp.clone(), w.charge.clone());
        let mut seen = None;
        for step in 0..6000 {
            // Keep the cloud and its charge as they are: only the ground is under test.
            w.cells.copy_from_slice(&cells);
            w.temp.copy_from_slice(&temp);
            w.charge.copy_from_slice(&charge);
            w.ticks = 0;
            w.storm_step();
            if step == 40 {
                seen = Some((w.mcharge_at(450, 381), w.mcharge_at(380, 469), w.mcharge_at(50, 469)));
            }
            if !w.bolts.is_empty() {
                assert!(step > 50, "the charge takes a while to build up: {step}");
                assert!(step < 1500, "but not forever: {step}");
                break;
            }
        }
        let (mast, ground_under, far) = seen.unwrap();
        assert!(mast > 0.0 && ground_under > 0.0, "under the negative base the ground turns positive: mast {mast}, ground {ground_under}");
        assert!(mast > ground_under, "most on the tall iron mast: {mast} / {ground_under}");
        assert!(far.abs() < ground_under * 0.2, "far from the cloud, little: {far}");
        assert!(!w.bolts.is_empty(), "the charge builds up until the air breaks down");
        let end = w.bolts[0].path.last().unwrap();
        assert!((end.0 - 450.0).abs() < 4.0 && (end.1 - 380.0).abs() < 6.0, "the bolt leaves for the mast: {end:?}");
        assert!(w.mcharge_at(450, 381).abs() < GROUND_BREAKDOWN * 0.5, "the strike drains the mast");
    }

    #[test]
    fn a_bolt_leaves_its_charge_in_what_it_strikes_and_it_runs_along_the_iron() {
        let mut w = World::new_static();
        ground(&mut w, 420);
        for x in 300..500 {
            w.set(x, 419, Mat::Iron);
        }
        w.strike(310, 418);
        assert!(w.mcharge_at(310, 419) < -0.2, "the bolt leaves negative charge: {}", w.mcharge_at(310, 419));
        for _ in 0..200 {
            w.ticks = 0;
            w.storm_step();
        }
        assert!(w.mcharge_at(400, 419) < -0.01, "the charge spreads along the rail: {}", w.mcharge_at(400, 419));
        let blank = {
            let mut b = World::new_static();
            ground(&mut b, 420);
            for x in 300..500 {
                b.set(x, 419, Mat::Iron);
            }
            b.electric_color(400, 419)
        };
        assert_ne!(w.electric_color(400, 419), blank, "the electricity view shows the charge in the rail");
        for _ in 0..3000 {
            w.ticks = 0;
            w.storm_step();
        }
        assert!(w.mcharge_at(310, 419).abs() < 0.05, "and it drains away into the ground: {}", w.mcharge_at(310, 419));
    }

    #[test]
    fn a_charged_spot_draws_the_opposite_charge_round_it() {
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            for y in 400..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        let k = (420 / QS) * QW() + 480 / QS;
        for _ in 0..200 {
            w.mcharge[k] = -1.2;
            w.ticks = 0;
            w.storm_step();
        }
        let at = |x: usize, y: usize| w.polar[(y / QS) * QW() + x / QS] + w.mcharge[(y / QS) * QW() + x / QS];
        let near = at(480 + QS * 2, 420);
        let far = at(100, 420);
        assert!(near > 0.02, "round a negative spot the rock turns positive: {near}");
        assert!(far.abs() < near * 0.1, "but not far away: {far}");
    }

    #[test]
    fn opposite_charges_attract_charged_sand_jumps_up_toward_the_cloud() {
        let mut w = World::new_static();
        ground(&mut w, 420);
        for x in 450..510 {
            for y in 410..420 {
                w.set(x, y, Mat::Sand);
            }
        }
        let before: Vec<Cell> = w.cells.clone();
        for _ in 0..40 {
            for qx in 450 / QS..510 / QS {
                w.charge[(300 / QS) * QW() + qx] = -1.0;
                w.mcharge[(408 / QS) * QW() + qx] = 0.8;
                w.mcharge[(414 / QS) * QW() + qx] = 0.8;
            }
            w.ticks = 0;
            w.storm_step();
        }
        let lifted = (380..410).flat_map(|y| (440..520).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y) == Mat::Sand).count();
        assert!(lifted > 0, "grains leap up toward the cloud of the other sign");
        // Like charges repel: the same sand under a positive cloud stays down.
        let mut v = World::new_static();
        v.cells.copy_from_slice(&before);
        for _ in 0..40 {
            for qx in 450 / QS..510 / QS {
                v.charge[(300 / QS) * QW() + qx] = 1.0;
                v.mcharge[(408 / QS) * QW() + qx] = 0.8;
                v.mcharge[(414 / QS) * QW() + qx] = 0.8;
            }
            v.ticks = 0;
            v.storm_step();
        }
        let up = (380..410).flat_map(|y| (440..520).map(move |x| (x, y))).filter(|&(x, y)| v.raw(x, y) == Mat::Sand).count();
        assert!(up * 5 < lifted, "a cloud of the same sign hardly lifts it: {up} against {lifted}");
    }

    #[test]
    fn lightning_magnetises_iron_which_pulls_in_iron_filings() {
        let mut w = World::new_static();
        ground(&mut w, 420);
        for y in 360..420 {
            w.set(480, y, Mat::Iron);
        }
        // A heap of filings on the ground a little way off.
        for x in 492..500 {
            for y in 414..420 {
                w.set(x, y, Mat::Rust);
            }
        }
        w.strike(480, 340);
        assert!(w.magnet[(400 / QS) * QW() + 480 / QS] > 0.4, "the current magnetised the post");
        let dist = |w: &World| {
            let (mut n, mut d) = (0, 0i32);
            for y in 300..420 {
                for x in 470..520 {
                    if w.raw(x, y) == Mat::Rust {
                        n += 1;
                        d += (x - 480).abs();
                    }
                }
            }
            d as f32 / n.max(1) as f32
        };
        let start = dist(&w);
        for _ in 0..600 {
            w.step();
        }
        let end = dist(&w);
        assert!(end < start - 3.0, "the filings crept toward the magnet: {start} -> {end}");
        let clinging = (360..420).filter(|&y| w.raw(481, y) == Mat::Rust).count();
        assert!(clinging > 0, "and some reach the post and cling to it");
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
        let mut out = format!("P6\n{} {}\n255\n", W(), H()).into_bytes();
        for y in 0..H() {
            for x in 0..W() {
                let i = y * W() + x;
                let c = match w.cells[i].m {
                    Mat::Cloud => [240, 240, 250],
                    Mat::Steam => [150, 160, 190],
                    Mat::Water => [40, 90, 220],
                    Mat::Snow | Mat::Hail | Mat::Ice => [200, 230, 255],
                    Mat::Empty => if y as i32 == CEILING_Y() { [90, 40, 40] } else { [20, 24, 40] },
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
        for x in 0..W() as i32 {
            for y in 420..H() as i32 {
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
                        let i = y * W() + x;
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
                    let ground = (0..H() as i32).find(|&y| !matches!(w.at(x, y).kind(), Kind::Empty | Kind::Gas)).unwrap_or(H() as i32);
                    let y = ground - 2 - ((step as u32 + k * 13) % 6) as i32;
                    if w.raw(x, y) == Mat::Empty {
                        w.set(x, y, Mat::Steam);
                        w.temp[y as usize * W() + x as usize] = 32.0;
                    }
                }
            }
            w.step();
            if step % 250 == 0 {
                dump(&w, &format!("{dir}/anvil_{step:05}.ppm"));
                let n = w.cells.iter().filter(|c| c.m == Mat::Cloud).count();
                let top = (0..W() * H()).find(|&i| w.cells[i].m == Mat::Cloud).map(|i| i / W());
                let c = |m: Mat| w.cells.iter().filter(|c| c.m == m).count();
                let v: Vec<String> = [40, 60, 72, 100, 200, 390].iter().map(|&y| format!("{y}:{:.2}", w.flow.sample(480, y).0)).collect();
                eprintln!("  flow vx {}", v.join(" "));
                let anv: Vec<usize> = (0..80 * W()).filter(|&i| w.cells[i].m == Mat::Cloud).map(|i| i % W()).collect();
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
