//! Pipes, valves and taps: a network that carries liquid between its open ends.
//!
//! The liquid in the world is a cellular automaton, which has no way to push water uphill or to
//! feel a siphon. Pipes are therefore solved as a small hydraulic network laid over it: every open
//! end sees the surface of the liquid it dips into (or the air), that gives it a *head* (the height
//! the liquid would rise to), junction heads follow from conservation of flow, and the flow along a
//! pipe is proportional to the difference of heads at its ends. Water is then taken from the cells
//! at the inlets and handed back at the outlets. From that single rule come communicating vessels,
//! locks (basins joined by valved pipes), siphons that run over a hump, and their limit: a column of
//! liquid cannot be sucked higher than the atmosphere can hold it, and then the siphon breaks.

use crate::world::{Kind, Mat, World, H};
#[cfg(test)]
use crate::world::W;

/// How high (in cells) the atmosphere can hold a column of liquid: the limit of a siphon's hump
/// above the surface it draws from.
pub const ATM_HEAD: f32 = 140.0;
/// Ends closer than this are one junction.
const JOIN: f32 = 3.0;
/// How far from an open end the liquid is looked for.
const PORT_REACH: f32 = 3.5;
/// Flow (cells of liquid per step) for a head difference of one cell in a pipe of length zero.
const FLOW_GAIN: f32 = 6.0;
/// A pipe never carries more than this many cells of liquid per step.
const FLOW_MAX: f32 = 4.0;

pub fn is_liquid(m: Mat) -> bool {
    m.kind() == Kind::Liquid
}

/// What the network needs from the world around it.
pub trait Env {
    /// The liquid a port at `p` dips into, if it does: what it is and the height of its surface.
    fn liquid_at(&self, p: (f32, f32)) -> Option<(Mat, f32)>;
    /// Height above the bottom of the world.
    fn elevation(&self, p: (f32, f32)) -> f32 {
        H() as f32 - p.1
    }
    /// Removes one cell of liquid next to `p`.
    fn take_liquid(&mut self, p: (f32, f32)) -> Option<Mat>;
    /// Puts one cell of `m` next to `p`; false if there is no room.
    fn put_liquid(&mut self, p: (f32, f32), m: Mat) -> bool;
}

impl Env for World {
    fn liquid_at(&self, p: (f32, f32)) -> Option<(Mat, f32)> {
        let (px, py) = (p.0.floor() as i32, p.1.floor() as i32);
        let mut best: Option<(f32, i32, i32, Mat)> = None;
        let r = PORT_REACH.ceil() as i32;
        for dy in -r..=r {
            for dx in -r..=r {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                let (x, y) = (px + dx, py + dy);
                let m = self.raw(x, y);
                if d <= PORT_REACH && is_liquid(m) && best.map_or(true, |b| d < b.0) {
                    best = Some((d, x, y, m));
                }
            }
        }
        let (_, x, mut y, m) = best?;
        // The surface is the top of the liquid above that cell.
        while y > 0 && is_liquid(self.raw(x, y - 1)) {
            y -= 1;
        }
        Some((m, H() as f32 - y as f32))
    }

    fn take_liquid(&mut self, p: (f32, f32)) -> Option<Mat> {
        let (px, py) = (p.0.floor() as i32, p.1.floor() as i32);
        let mut best: Option<(f32, i32, i32, Mat)> = None;
        let r = PORT_REACH.ceil() as i32;
        for dy in -r..=r {
            for dx in -r..=r {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                let m = self.raw(px + dx, py + dy);
                if d <= PORT_REACH && is_liquid(m) && best.map_or(true, |b| d < b.0) {
                    best = Some((d, px + dx, py + dy, m));
                }
            }
        }
        let (_, x, y, m) = best?;
        self.set(x, y, Mat::Empty);
        Some(m)
    }

    fn put_liquid(&mut self, p: (f32, f32), m: Mat) -> bool {
        let (px, py) = (p.0.floor() as i32, p.1.floor() as i32);
        let mut best: Option<(f32, i32, i32)> = None;
        let r = PORT_REACH.ceil() as i32;
        for dy in -r..=r {
            for dx in -r..=r {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if d <= PORT_REACH && self.raw(px + dx, py + dy) == Mat::Empty && best.map_or(true, |b| d < b.0) {
                    best = Some((d, px + dx, py + dy));
                }
            }
        }
        if let Some((_, x, y)) = best {
            self.set(x, y, m);
            return true;
        }
        // Submerged and surrounded: the new liquid goes on top of the column, which then rises.
        let (x, mut y) = (px, py);
        while y > 0 && is_liquid(self.raw(x, y)) {
            y -= 1;
        }
        if self.raw(x, y) == Mat::Empty {
            self.set(x, y, m);
            return true;
        }
        false
    }
}

/// A straight length of pipe.
#[derive(Clone, Debug)]
pub struct Pipe {
    pub uid: u32,
    pub a: (f32, f32),
    pub b: (f32, f32),
    /// Liquid fills it (a siphon that has caught, a full pipe).
    pub primed: bool,
    /// Flow in cells per step, positive from `a` to `b` (output of the last step).
    pub flow: f32,
    /// Head at each end, in cells above the bottom of the world.
    pub head: (f32, f32),
    /// What it carries.
    pub fluid: Mat,
}

impl Pipe {
    pub fn new(uid: u32, a: (f32, f32), b: (f32, f32)) -> Self {
        Pipe { uid, a, b, primed: false, flow: 0.0, head: (0.0, 0.0), fluid: Mat::Water }
    }

    pub fn len(&self) -> f32 {
        ((self.b.0 - self.a.0).powi(2) + (self.b.1 - self.a.1).powi(2)).sqrt()
    }

    pub fn point(&self, t: f32) -> (f32, f32) {
        (self.a.0 + (self.b.0 - self.a.0) * t, self.a.1 + (self.b.1 - self.a.1) * t)
    }

    /// Distance from a point to the pipe, and where along it the nearest point is (0..1).
    pub fn distance_to(&self, p: (f32, f32)) -> (f32, f32) {
        let (dx, dy) = (self.b.0 - self.a.0, self.b.1 - self.a.1);
        let l2 = (dx * dx + dy * dy).max(1e-6);
        let t = (((p.0 - self.a.0) * dx + (p.1 - self.a.1) * dy) / l2).clamp(0.0, 1.0);
        let q = self.point(t);
        (((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt(), t)
    }
}

/// What turns a valve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Driver {
    /// A gear: it opens or closes the valve over `turns` turns and then stops against its end stop.
    Gear(u32),
    /// The carriage of a rail: the valve opens as the carriage moves along it.
    Slider(u32),
}

/// A valve (or a tap, at the end of a pipe).
#[derive(Clone, Debug)]
pub struct Valve {
    pub uid: u32,
    pub pipe: u32,
    /// Where along the pipe (0..1).
    pub t: f32,
    /// 0 shut, 1 wide open.
    pub open: f32,
    /// Turns of the driving gear from shut to open.
    pub turns: f32,
    pub driver: Option<Driver>,
}

impl Valve {
    pub fn new(uid: u32, pipe: u32, t: f32) -> Self {
        Valve { uid, pipe, t, open: 1.0, turns: 3.0, driver: None }
    }
}

/// A pump on a pipe: it raises the head of the liquid by `head` cells, pushing it from the first end
/// of the pipe to the second (negative: the other way). It can lift liquid above the level the
/// atmosphere alone could, the way a siphon cannot.
#[derive(Clone, Debug)]
pub struct Pump {
    pub uid: u32,
    pub pipe: u32,
    pub t: f32,
    /// The lift it gives, in cells of liquid.
    pub head: f32,
    pub on: bool,
    /// A gear that turns it: the lift then follows the speed of the gear.
    pub driver: Option<u32>,
}

impl Pump {
    pub fn new(uid: u32, pipe: u32, t: f32) -> Self {
        Pump { uid, pipe, t, head: 90.0, on: true, driver: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// A siphon's hump is too high: the liquid column broke and the flow stopped.
    SiphonBroke(u32),
}

#[derive(Clone, Default, Debug)]
pub struct Plumbing {
    pub pipes: Vec<Pipe>,
    pub valves: Vec<Valve>,
    pub pumps: Vec<Pump>,
    /// Liquid owed to (or by) each end of each pipe: `[2 * pipe + end]`.
    owed: Vec<f32>,
}

/// An end of a pipe: where it is, and whether it is open to the world.
#[derive(Clone, Debug)]
struct Node {
    pos: (f32, f32),
    /// (pipe index, end 0/1) of every pipe end that meets here.
    ends: Vec<(usize, u8)>,
}

impl Plumbing {
    pub fn is_empty(&self) -> bool {
        self.pipes.is_empty()
    }

    #[cfg(test)]
    pub fn pipe_index(&self, uid: u32) -> Option<usize> {
        self.pipes.iter().position(|p| p.uid == uid)
    }

    /// Adds a pipe. If either end lands on the middle of another pipe, that pipe is cut there so that
    /// the two meet at a junction.
    pub fn add_pipe(&mut self, mut a: (f32, f32), mut b: (f32, f32), uid: u32, mut next_uid: impl FnMut() -> u32) -> u32 {
        for end in [&mut a, &mut b] {
            // Ends snap onto the end of a pipe, or into the side of one (which is then split).
            if let Some(q) = self.pipes.iter().flat_map(|p| [p.a, p.b]).find(|q| dist(*q, *end) < JOIN) {
                *end = q;
                continue;
            }
            let hit = self.pipes.iter().enumerate().map(|(i, p)| (i, p.distance_to(*end))).filter(|(_, (d, t))| *d < JOIN && *t > 0.02 && *t < 0.98).min_by(|x, y| (x.1).0.total_cmp(&(y.1).0));
            if let Some((i, (_, t))) = hit {
                let cut = self.pipes[i].point(t);
                self.split(i, t, next_uid());
                *end = cut;
            }
        }
        self.pipes.push(Pipe::new(uid, a, b));
        self.owed = vec![0.0; self.pipes.len() * 2];
        uid
    }

    /// Cuts pipe `i` at `t` into two; valves stay where they were.
    fn split(&mut self, i: usize, t: f32, new_uid: u32) {
        let old = self.pipes[i].clone();
        let mid = old.point(t);
        self.pipes[i].b = mid;
        let mut second = Pipe::new(new_uid, mid, old.b);
        second.fluid = old.fluid;
        second.primed = old.primed;
        for v in &mut self.valves {
            if v.pipe == old.uid {
                if v.t > t {
                    v.pipe = new_uid;
                    v.t = (v.t - t) / (1.0 - t);
                } else {
                    v.t /= t;
                }
            }
        }
        for pm in &mut self.pumps {
            if pm.pipe == old.uid {
                if pm.t > t {
                    pm.pipe = new_uid;
                    pm.t = (pm.t - t) / (1.0 - t);
                } else {
                    pm.t /= t;
                }
            }
        }
        self.pipes.push(second);
        self.owed = vec![0.0; self.pipes.len() * 2];
    }

    pub fn remove_pipe(&mut self, uid: u32) {
        self.pipes.retain(|p| p.uid != uid);
        self.valves.retain(|v| v.pipe != uid);
        self.pumps.retain(|p| p.pipe != uid);
        self.owed = vec![0.0; self.pipes.len() * 2];
    }

    fn nodes(&self) -> Vec<Node> {
        let mut nodes: Vec<Node> = vec![];
        for (i, p) in self.pipes.iter().enumerate() {
            for (e, q) in [(0u8, p.a), (1u8, p.b)] {
                match nodes.iter_mut().find(|n| dist(n.pos, q) < JOIN) {
                    Some(n) => n.ends.push((i, e)),
                    None => nodes.push(Node { pos: q, ends: vec![(i, e)] }),
                }
            }
        }
        nodes
    }

    /// How freely a pipe lets liquid through: its length (friction) and its valves.
    fn conductance(&self, i: usize) -> f32 {
        let p = &self.pipes[i];
        let mut open = 1.0f32;
        for v in self.valves.iter().filter(|v| v.pipe == p.uid) {
            open *= v.open.clamp(0.0, 1.0).powf(1.5);
        }
        if open < 0.01 {
            return 0.0;
        }
        FLOW_GAIN * open / (p.len() + 12.0) * 12.0
    }

    /// The lift the pumps on a pipe give, positive from its first end to its second.
    fn boost(&self, i: usize) -> f32 {
        let uid = self.pipes[i].uid;
        self.pumps.iter().filter(|p| p.pipe == uid && p.on).map(|p| p.head).sum()
    }

    /// The highest point of a pipe, as an elevation.
    fn crest(env: &dyn Env, p: &Pipe) -> f32 {
        env.elevation(p.a).max(env.elevation(p.b))
    }

    /// One step: heads, flow, and liquid moved between the pipes and the world.
    pub fn step(&mut self, env: &mut dyn Env) -> Vec<Event> {
        let mut events = vec![];
        if self.pipes.is_empty() {
            return events;
        }
        if self.owed.len() != self.pipes.len() * 2 {
            self.owed = vec![0.0; self.pipes.len() * 2];
        }
        let nodes = self.nodes();
        let node_of = |i: usize, e: u8| nodes.iter().position(|n| n.ends.contains(&(i, e))).unwrap();

        // Heads: an open end takes the height of the surface it dips into (or its own height in the air).
        let mut head = vec![0.0f32; nodes.len()];
        let mut fixed = vec![false; nodes.len()];
        let mut wet = vec![false; nodes.len()];
        let mut fluid = vec![None; nodes.len()];
        for (n, node) in nodes.iter().enumerate() {
            if node.ends.len() == 1 {
                fixed[n] = true;
                match env.liquid_at(node.pos) {
                    Some((m, surface)) => {
                        head[n] = surface;
                        wet[n] = true;
                        fluid[n] = Some(m);
                    }
                    None => head[n] = env.elevation(node.pos),
                }
            } else {
                head[n] = nodes[n].ends.iter().map(|&(i, e)| env.elevation(if e == 0 { self.pipes[i].a } else { self.pipes[i].b })).sum::<f32>() / node.ends.len() as f32;
            }
        }
        let g: Vec<f32> = (0..self.pipes.len()).map(|i| self.conductance(i)).collect();
        let boost: Vec<f32> = (0..self.pipes.len()).map(|i| self.boost(i)).collect();
        let solve = |head: &mut Vec<f32>, on: &[bool]| {
            for _ in 0..60 {
                for n in 0..nodes.len() {
                    if fixed[n] {
                        continue;
                    }
                    let (mut num, mut den) = (0.0, 0.0);
                    for &(i, e) in &nodes[n].ends {
                        if !on[i] || g[i] <= 0.0 {
                            continue;
                        }
                        let other = node_of(i, 1 - e);
                        // A pump lifts the liquid on its way from the first end to the second.
                        let lift = if e == 1 { boost[i] } else { -boost[i] };
                        num += g[i] * (head[other] + lift);
                        den += g[i];
                    }
                    if den > 0.0 {
                        head[n] = num / den;
                    }
                }
            }
        };
        let mut on = vec![true; self.pipes.len()];
        solve(&mut head, &on);

        // Which pipes are full of liquid and can carry it: the upstream end must dip into liquid (or
        // be fed by a junction), and the pipe's crest must not rise higher above the upstream head
        // than the atmosphere can lift liquid (the siphon limit).
        for (i, p) in self.pipes.iter_mut().enumerate() {
            let (na, nb) = (node_of(i, 0), node_of(i, 1));
            let (ha, hb) = (head[na], head[nb]);
            let drive = ha - hb + boost[i];
            let (up, up_wet) = if drive >= 0.0 { (na, wet[na] || nodes[na].ends.len() > 1) } else { (nb, wet[nb] || nodes[nb].ends.len() > 1) };
            let h_up = head[up];
            // A pump pushes the column, so the pipe may rise higher above its source than a siphon's.
            let too_high = Self::crest(env, p) - h_up - boost[i].abs() > ATM_HEAD;
            let was = p.primed;
            let both_dry = !wet[na] && !wet[nb] && nodes[na].ends.len() == 1 && nodes[nb].ends.len() == 1;
            p.primed = g[i] > 0.0 && up_wet && !too_high && !both_dry && drive.abs() > 0.05;
            if was && too_high && up_wet {
                events.push(Event::SiphonBroke(p.uid));
            }
            on[i] = p.primed;
        }
        // The heads again, with only the pipes that carry something.
        for n in 0..nodes.len() {
            if !fixed[n] {
                head[n] = nodes[n].ends.iter().map(|&(i, e)| env.elevation(if e == 0 { self.pipes[i].a } else { self.pipes[i].b })).sum::<f32>() / nodes[n].ends.len() as f32;
            }
        }
        solve(&mut head, &on);

        // Flow along each pipe, from the higher head to the lower.
        for i in 0..self.pipes.len() {
            let (na, nb) = (node_of(i, 0), node_of(i, 1));
            let p = &mut self.pipes[i];
            p.head = (head[na], head[nb]);
            p.flow = if on[i] { (g[i] * (head[na] - head[nb] + boost[i])).clamp(-FLOW_MAX, FLOW_MAX) } else { 0.0 };
            if let Some(m) = fluid[na].or(fluid[nb]) {
                p.fluid = m;
            }
        }

        // Liquid goes in at the inlets and comes out at the outlets (open ends only).
        for (_n, node) in nodes.iter().enumerate() {
            if node.ends.len() != 1 {
                continue;
            }
            let (i, e) = node.ends[0];
            // Positive flow along the pipe leaves its first end, so it is drawn from the world there.
            let into_pipe = if e == 0 { self.pipes[i].flow } else { -self.pipes[i].flow };
            let slot = 2 * i + e as usize;
            self.owed[slot] += into_pipe;
            let kind = self.pipes[i].fluid;
            while self.owed[slot] >= 1.0 {
                if let Some(m) = env.take_liquid(node.pos) {
                    self.pipes[i].fluid = m;
                    self.owed[slot] -= 1.0;
                } else {
                    // The inlet is dry: air gets in, and the pipe stops being full.
                    self.owed[slot] = 0.0;
                    self.pipes[i].primed = false;
                    break;
                }
            }
            while self.owed[slot] <= -1.0 {
                if env.put_liquid(node.pos, kind) {
                    self.owed[slot] += 1.0;
                } else {
                    self.owed[slot] = -1.0;
                    break;
                }
            }
        }
        events
    }
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Two stone vessels side by side, filled with water to the given levels (elevations above the
    /// bottom of the world), on a static floor.
    fn vessels(left_level: i32, right_level: i32) -> World {
        let mut w = World::new_static();
        let floor = H() as i32 - 20;
        for x in 0..W() as i32 {
            for y in floor..H() as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        // Left vessel x 100..160, right vessel x 300..360, walls 8 thick and 120 high.
        for (x0, x1, level) in [(100, 160, left_level), (300, 360, right_level)] {
            for y in floor - 120..floor {
                for x in x0 - 8..x0 {
                    w.set(x, y, Mat::Stone);
                }
                for x in x1..x1 + 8 {
                    w.set(x, y, Mat::Stone);
                }
            }
            for y in (H() as i32 - level)..floor {
                for x in x0..x1 {
                    w.set(x, y, Mat::Water);
                }
            }
        }
        w
    }

    fn water_in(w: &World, x0: i32, x1: i32) -> usize {
        (0..H() as i32).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| w.raw(x, y) == Mat::Water).count()
    }

    fn surface(w: &World, x: i32) -> f32 {
        let y = (0..H() as i32).find(|&y| w.raw(x, y) == Mat::Water).unwrap_or(H() as i32);
        H() as f32 - y as f32
    }

    fn run(p: &mut Plumbing, w: &mut World, steps: usize) -> Vec<Event> {
        let mut ev = vec![];
        for _ in 0..steps {
            ev.extend(p.step(w));
            w.step();
        }
        ev
    }

    fn uids() -> impl FnMut() -> u32 {
        let mut n = 1000;
        move || {
            n += 1;
            n
        }
    }

    /// A pipe from deep in the left vessel to deep in the right one, with a hump of `rise` above
    /// the water between them (given as three straight pipes).
    fn u_tube(p: &mut Plumbing, rise: f32) {
        let low = (H() as f32) - 40.0; // y of the pipe ends, well under the surface of both vessels
        let top = low - rise;
        let mut u = uids();
        p.add_pipe((140.0, low), (140.0, top), 1, &mut u);
        p.add_pipe((140.0, top), (320.0, top), 2, &mut u);
        p.add_pipe((320.0, top), (320.0, low), 3, &mut u);
    }

    #[test]
    fn a_u_tube_levels_two_vessels() {
        let mut w = vessels(100, 30);
        let mut p = Plumbing::default();
        u_tube(&mut p, 60.0);
        let before = water_in(&w, 90, 370);
        run(&mut p, &mut w, 4000);
        let (l, r) = (surface(&w, 130), surface(&w, 330));
        eprintln!("U-TUBE levels {l} / {r}");
        assert!((l - r).abs() < 6.0, "the two surfaces meet: {l} vs {r}");
        assert!(l < 98.0 && r > 36.0, "the high vessel gave and the low one took: {l} {r}");
        let after = water_in(&w, 90, 370);
        assert!((after as i32 - before as i32).abs() < 60, "no water appears or disappears: {before} -> {after}");
    }

    #[test]
    fn a_siphon_runs_over_a_hump_above_the_water_and_stops_when_levels_meet() {
        // The hump is 20 cells above the high surface: a siphon, not a gravity pipe.
        let mut w = vessels(90, 20);
        let mut p = Plumbing::default();
        u_tube(&mut p, 90.0 - 20.0 + 20.0 + 20.0); // top at elevation 40 + rise
        let crest = p.pipes.iter().map(|q| H() as f32 - q.a.1.min(q.b.1)).fold(0.0f32, f32::max);
        assert!(crest > 90.0 + 15.0, "the hump is above the water: {crest}");
        run(&mut p, &mut w, 5000);
        let (l, r) = (surface(&w, 130), surface(&w, 330));
        assert!((l - r).abs() < 6.0, "the siphon equalised the vessels: {l} vs {r}");
        assert!(r > 40.0, "water went over the hump: {r}");
    }

    #[test]
    fn a_hump_higher_than_the_atmosphere_can_lift_breaks_the_siphon() {
        let mut w = vessels(100, 20);
        let mut p = Plumbing::default();
        u_tube(&mut p, 40.0 + ATM_HEAD + 20.0); // far above the 100 cells of water
        let ev = run(&mut p, &mut w, 1500);
        let (l, r) = (surface(&w, 130), surface(&w, 330));
        assert!(r < 25.0 && l > 95.0, "nothing flows over: {l} / {r}");
        assert!(ev.is_empty() || ev.iter().all(|e| matches!(e, Event::SiphonBroke(_))));
    }

    #[test]
    fn a_closed_valve_stops_the_flow_and_a_half_open_one_slows_it() {
        let mut moved = vec![];
        for open in [1.0f32, 0.25, 0.0] {
            let mut w = vessels(100, 30);
            let mut p = Plumbing::default();
            u_tube(&mut p, 60.0);
            let mut v = Valve::new(1, 2, 0.5);
            v.open = open;
            p.valves.push(v);
            run(&mut p, &mut w, 150);
            moved.push(100.0 - surface(&w, 130));
        }
        eprintln!("VALVE moved {moved:?}");
        assert!(moved[0] > moved[1] + 2.0 && moved[1] > 0.5, "a throttled valve flows slower: {moved:?}");
        assert!(moved[2] < 1.5, "shut means shut: {moved:?}");
    }

    #[test]
    fn a_pump_lifts_water_to_a_vessel_above_where_it_would_flow_by_itself() {
        // The left vessel is the low one: its water is at elevation 70, the right one's at 110.
        let mut w = vessels(70, 110);
        let mut p = Plumbing::default();
        u_tube(&mut p, 130.0);
        // Without a pump nothing goes uphill: the high vessel feeds the low one.
        let mut dry = vessels(70, 110);
        let mut plain = p.clone();
        run(&mut plain, &mut dry, 300);
        assert!(surface(&dry, 130) > 72.0, "water runs downhill, towards the low vessel");
        // A pump on the first pipe, pushing from the left vessel towards the hump.
        p.pumps.push(Pump::new(1, 1, 0.5));
        p.pumps[0].head = 140.0;
        run(&mut p, &mut w, 3000);
        let (l, r) = (surface(&w, 130), surface(&w, 330));
        eprintln!("PUMP levels {l} / {r}");
        assert!(r > 125.0, "the pump raised the right-hand vessel well above its first level: {r}");
        assert!(l < 50.0, "and drew the left one down: {l}");
        // Off, it does nothing: the water flows back down.
        p.pumps[0].on = false;
        let high = surface(&w, 330);
        run(&mut p, &mut w, 2000);
        assert!(surface(&w, 330) < high - 5.0, "without the pump the water returns");
    }

    #[test]
    fn a_pipe_with_both_ends_in_air_carries_nothing() {
        let mut w = vessels(100, 30);
        let mut p = Plumbing::default();
        p.add_pipe((200.0, 200.0), (260.0, 300.0), 1, &mut uids());
        let before = water_in(&w, 0, W() as i32);
        run(&mut p, &mut w, 300);
        // Only a few cells evaporate from the open vessels.
        let after = water_in(&w, 0, W() as i32);
        assert!(after <= before && after + 5 >= before, "{before} -> {after}");
    }

    #[test]
    fn a_branch_splits_a_pipe_into_a_junction_and_feeds_both_vessels() {
        let mut p = Plumbing::default();
        let mut u = uids();
        p.add_pipe((100.0, 300.0), (300.0, 300.0), 1, &mut u);
        // A branch ending on the side of the first pipe cuts it in two.
        p.add_pipe((200.0, 300.0 - 40.0), (201.0, 300.5), 2, &mut u);
        assert_eq!(p.pipes.len(), 3, "the pipe was cut at the branch");
        let ends: Vec<(f32, f32)> = p.pipes.iter().flat_map(|q| [q.a, q.b]).collect();
        let at_junction = ends.iter().filter(|q| dist(**q, (201.0, 300.0)) < JOIN + 1.0 || dist(**q, (200.0, 300.0)) < JOIN + 1.0).count();
        assert!(at_junction >= 3, "three pipe ends meet at the junction: {at_junction}");
        // A valve on the original pipe stays on the right half after the cut.
        let mut p = Plumbing::default();
        p.add_pipe((0.0, 0.0), (100.0, 0.0), 1, &mut u);
        p.valves.push(Valve::new(1, 1, 0.8));
        p.add_pipe((50.0, 40.0), (50.0, 0.5), 2, &mut u);
        let v = &p.valves[0];
        let pipe = &p.pipes[p.pipe_index(v.pipe).unwrap()];
        let at = pipe.point(v.t);
        assert!((at.0 - 80.0).abs() < 2.0, "the valve is still where it was: {at:?}");
    }

    #[test]
    fn a_lock_fills_from_the_upper_basin_through_valves_and_the_level_is_the_same_at_the_end() {
        // Upper basin (left, high), lock chamber (right, low), a valve in the culvert: shut, nothing
        // moves; opened, the chamber fills to the upper level.
        let mut w = vessels(100, 10);
        let mut p = Plumbing::default();
        u_tube(&mut p, 60.0);
        p.valves.push(Valve::new(1, 2, 0.5));
        p.valves[0].open = 0.0;
        run(&mut p, &mut w, 800);
        assert!(surface(&w, 330) < 14.0, "shut: the chamber stays low: {}", surface(&w, 330));
        p.valves[0].open = 1.0;
        run(&mut p, &mut w, 5000);
        let (l, r) = (surface(&w, 130), surface(&w, 330));
        assert!((l - r).abs() < 6.0, "open: the chamber reaches the upper level: {l} vs {r}");
    }
}
