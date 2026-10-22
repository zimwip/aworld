//! Kinematics: gears, axles, connecting rods (bielles) and sliders (coulisseaux), driven by a motor.
//!
//! Each group of meshed gears is one degree of freedom `q` (the angle of its first gear): every
//! gear `i` turns `k_i` times as fast. Rods tie a pin on a gear to a slider on a straight rail,
//! so a slider's position is a function of `q`. The group is integrated as a single body:
//!
//!   I_eq(q) q'' = motor torque + sum(external force on a slider * ds/dq) - friction
//!
//! That one equation gives the behaviour the game needs without simulating teeth: gears that
//! drift apart stop meshing and spin free, a slider reaching its stop stalls the motor, and the
//! force in a rod or on a tooth follows from virtual work (so a rod near dead centre, or a
//! tooth under a stalled motor, can give way).

use crate::blocks::Anchor;
use crate::pipes::Plumbing;
use macroquad::prelude::{vec2, Vec2};
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_MECH_UID: AtomicU32 = AtomicU32::new(1);

/// A fresh id for a gear, rod, rail or axle.
pub fn next_uid() -> u32 {
    NEXT_MECH_UID.fetch_add(1, Ordering::Relaxed)
}

/// After loading a world: new ids must not collide with the ones that came back.
pub fn bump_uids(above: u32) {
    NEXT_MECH_UID.fetch_max(above + 1, Ordering::Relaxed);
}

/// Teeth per cell are the same for every size, so gears mesh when their centres are
/// `r_a + r_b` apart.
const MESH_TOLERANCE: f32 = 1.5;
/// A stop absorbs the motion over this many steps (so it is a hard shove, not infinite).
const STOP_TICKS: f32 = 8.0;
/// Steps of overload before a rod or a tooth gives way.
const FATIGUE: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GearSize {
    Small,
    Medium,
    Large,
}

impl GearSize {
    pub const ALL: [GearSize; 3] = [GearSize::Small, GearSize::Medium, GearSize::Large];

    pub fn radius(self) -> f32 {
        match self {
            GearSize::Small => 6.0,
            GearSize::Medium => 10.0,
            GearSize::Large => 14.0,
        }
    }

    /// Disc of uniform thickness: mass ~ r^2, inertia ~ r^4.
    pub fn inertia(self) -> f32 {
        0.025 * self.radius().powi(4)
    }

    /// The largest force the teeth carry before they strip.
    pub fn tooth_strength(self) -> f32 {
        match self {
            GearSize::Small => 6.0,
            GearSize::Medium => 9.0,
            GearSize::Large => 13.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motor {
    /// Stall torque.
    pub torque: f32,
    /// Speed (rad/step) at which the torque has fallen to zero.
    pub free_speed: f32,
    /// +1 or -1.
    pub dir: f32,
    pub on: bool,
    /// Turns the other way when the load gets too heavy (or the mechanism hits a stop).
    pub auto_reverse: bool,
    /// Load, as a share of the stall torque, above which it reverses.
    pub reverse_at: f32,
    /// Steps left before it may reverse again.
    pub cooldown: u32,
    overload: u32,
}

impl Motor {
    pub fn new() -> Self {
        Motor { torque: 20.0, free_speed: 0.12, dir: 1.0, on: true, auto_reverse: false, reverse_at: 0.8, cooldown: 0, overload: 0 }
    }

    pub fn reversing() -> Self {
        Motor { auto_reverse: true, ..Motor::new() }
    }
}

#[derive(Clone, Debug)]
pub struct Gear {
    pub uid: u32,
    pub size: GearSize,
    /// Centre, in world cells (follows the support it is mounted on).
    pub pos: Vec2,
    /// What the centre is fixed to: a block, a rod, the ground, or nothing (fixed in the air).
    pub mount: Anchor,
    pub angle: f32,
    pub omega: f32,
    pub motor: Option<Motor>,
    /// Teeth gone: it no longer meshes, but can still turn on its axle.
    pub stripped: bool,
    /// Meshed in an impossible loop (odd ring of gears): locked.
    pub jammed: bool,
    /// A wheel block this gear drives through a shaft (uid of the block).
    pub wheel: Option<u32>,
    /// A winch: the radius of the drum the rope winds on (cells). Turning the gear the positive way
    /// reels the rope in.
    pub reel: Option<f32>,
    /// The angle range the gear may turn through (an end stop with unlimited torque: a winch that
    /// has run out of rope, a valve that is fully open or shut). The motor stalls against it.
    pub limit: Option<(f32, f32)>,
    /// What a driven wheel pushes back with, set by the game every step.
    pub ext_torque: f32,
    /// The motor torque reaching this gear's shaft (output of the last step).
    pub shaft_torque: f32,
    /// Torque going through this gear's shaft, and how loaded its teeth are (1 = about to strip).
    pub torque: f32,
    pub tooth_load: f32,
    /// For a motor: how hard it is pushing against its load, as a share of its stall torque.
    pub motor_load: f32,
    stress: u32,
}

impl Gear {
    pub fn new(uid: u32, size: GearSize, pos: Vec2) -> Self {
        Gear { uid, size, pos, mount: Anchor::Free(pos), angle: 0.0, omega: 0.0, motor: None, stripped: false, jammed: false, wheel: None, reel: None, limit: None, ext_torque: 0.0, shaft_torque: 0.0, torque: 0.0, tooth_load: 0.0, motor_load: 0.0, stress: 0 }
    }
}

/// A shaft joining two gears: they turn together (same angle, same speed).
#[derive(Clone, Copy, Debug)]
pub struct Axle {
    pub uid: u32,
    pub a: u32,
    pub b: u32,
}

/// A rail with a carriage on it.
#[derive(Clone, Debug)]
pub struct Slider {
    pub uid: u32,
    /// What each end of the rail is fixed to.
    pub ends: (Anchor, Anchor),
    pub origin: Vec2,
    /// Unit vector along the rail.
    pub dir: Vec2,
    pub len: f32,
    /// Position along the rail, `margin..=len - margin`.
    pub s: f32,
    pub v: f32,
    pub mass: f32,
    /// What the world pushes on the carriage along `dir` (set by the caller every step).
    pub ext_force: f32,
    /// Force the carriage carries (output of the last step).
    pub load: f32,
    pub margin: f32,
    /// Something solid is in the way in this direction (set by the game every step).
    pub blocked_lo: bool,
    pub blocked_hi: bool,
    /// Half size of the carriage, for drawing and for pushing things.
    pub half: Vec2,
}

impl Slider {
    pub fn new(uid: u32, from: Vec2, to: Vec2) -> Self {
        let d = to - from;
        let len = d.length();
        Slider {
            uid,
            ends: (Anchor::Free(from), Anchor::Free(to)),
            origin: from,
            dir: d / len.max(1e-6),
            len,
            s: len / 2.0,
            v: 0.0,
            mass: 2.0,
            ext_force: 0.0,
            load: 0.0,
            margin: 3.0,
            blocked_lo: false,
            blocked_hi: false,
            half: vec2(5.0, 4.0),
        }
    }

    /// Re-fits the rail to where its two supports are now.
    pub fn set_ends(&mut self, from: Vec2, to: Vec2) {
        let d = to - from;
        let len = d.length();
        if len < 1e-3 {
            return;
        }
        self.origin = from;
        self.dir = d / len;
        self.len = len;
        let (lo, hi) = self.range();
        self.s = self.s.clamp(lo, hi);
    }

    pub fn point(&self) -> Vec2 {
        self.origin + self.dir * self.s
    }

    fn range(&self) -> (f32, f32) {
        let (mut lo, mut hi) = (self.margin, (self.len - self.margin).max(self.margin));
        if self.blocked_lo {
            lo = lo.max(self.s);
        }
        if self.blocked_hi {
            hi = hi.min(self.s);
        }
        (lo, hi.max(lo))
    }
}

/// One end of a connecting rod.
#[derive(Clone, Copy, Debug)]
pub enum End {
    /// A pin on a gear (the point is in the gear's own frame).
    Pin { gear: u32, pin: Vec2 },
    /// The carriage of a rail.
    Carriage(u32),
    /// Anything else (a block, the ground, a rod end, the air): pinned there. These rods are
    /// handled by the physics, not by the mechanism solver.
    Node(Anchor),
}

/// A connecting rod (bielle) between two things that move or hold. It can join a gear pin to a
/// carriage, two gear pins (a linkage), or a pin to anything else.
#[derive(Clone, Debug)]
pub struct Rod {
    pub uid: u32,
    pub a: End,
    pub b: End,
    pub len: f32,
    pub strength: f32,
    pub broken: bool,
    /// Last force carried (for the stress colouring).
    pub force: f32,
    stress: u32,
}

impl Rod {
    /// A rod from a gear pin to a carriage.
    pub fn new(uid: u32, gear: u32, pin: Vec2, slider: u32, len: f32) -> Self {
        Self::between(uid, End::Pin { gear, pin }, End::Carriage(slider), len)
    }

    pub fn between(uid: u32, a: End, b: End, len: f32) -> Self {
        Rod { uid, a, b, len, strength: 6.0, broken: false, force: 0.0, stress: 0 }
    }

    /// Does this rod touch the carriage `uid`?
    pub fn on_carriage(&self, uid: u32) -> bool {
        matches!(self.a, End::Carriage(u) if u == uid) || matches!(self.b, End::Carriage(u) if u == uid)
    }

    pub fn on_gear(&self, uid: u32) -> bool {
        matches!(self.a, End::Pin { gear, .. } if gear == uid) || matches!(self.b, End::Pin { gear, .. } if gear == uid)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    RodBroke(u32),
    TeethStripped(u32),
    Jammed(u32),
    /// A slider hit the end of its rail while driven.
    Stalled(u32),
}

/// A wheel block turning on a fixed axle: a pulley, a winch drum or a drive wheel.
#[derive(Clone, Copy, Debug)]
pub struct Pivot {
    pub wheel: u32,
    /// What the axle is fixed to (a block, the ground, or the air); the anchor is at the wheel's centre.
    pub support: Anchor,
}

/// A pulley (sheave) on a support: ropes bend round it. It needs no wheel block.
#[derive(Clone, Copy, Debug)]
pub struct Pulley {
    pub uid: u32,
    pub mount: Anchor,
    /// Centre in world cells (follows its support).
    pub pos: Vec2,
    pub radius: f32,
}

#[derive(Clone, Default, Debug)]
pub struct Mechanism {
    /// Pipes and valves: part of the mechanisms so that they are saved, undone and reset with them.
    pub plumbing: Plumbing,
    pub pulleys: Vec<Pulley>,
    pub pivots: Vec<Pivot>,
    pub gears: Vec<Gear>,
    pub axles: Vec<Axle>,
    pub sliders: Vec<Slider>,
    pub rods: Vec<Rod>,
}

fn rot(a: f32, v: Vec2) -> Vec2 {
    let (s, c) = a.sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

impl Mechanism {
    pub fn gear_index(&self, uid: u32) -> Option<usize> {
        self.gears.iter().position(|g| g.uid == uid)
    }

    pub fn slider_index(&self, uid: u32) -> Option<usize> {
        self.sliders.iter().position(|s| s.uid == uid)
    }

    /// Moves gears and rails to where the things they are fixed to are now. A support that no
    /// longer exists leaves the piece fixed in the air where it was.
    pub fn follow_mounts(&mut self, resolve: impl Fn(&Anchor) -> Option<Vec2>) {
        for g in &mut self.gears {
            match resolve(&g.mount) {
                Some(p) => g.pos = p,
                None => g.mount = Anchor::Free(g.pos),
            }
        }
        for p in &mut self.pulleys {
            match resolve(&p.mount) {
                Some(q) => p.pos = q,
                None => p.mount = Anchor::Free(p.pos),
            }
        }
        for sl in &mut self.sliders {
            let a = resolve(&sl.ends.0).unwrap_or_else(|| {
                sl.ends.0 = Anchor::Free(sl.origin);
                sl.origin
            });
            let end = sl.origin + sl.dir * sl.len;
            let b = resolve(&sl.ends.1).unwrap_or_else(|| {
                sl.ends.1 = Anchor::Free(end);
                end
            });
            sl.set_ends(a, b);
        }
    }

    /// Drops what depends on a piece that is gone: axles and rods tied to it.
    pub fn prune(&mut self) {
        let gears: Vec<u32> = self.gears.iter().map(|g| g.uid).collect();
        let sliders: Vec<u32> = self.sliders.iter().map(|s| s.uid).collect();
        self.axles.retain(|a| gears.contains(&a.a) && gears.contains(&a.b));
        let ok = |e: &End| match e {
            End::Pin { gear, .. } => gears.contains(gear),
            End::Carriage(u) => sliders.contains(u),
            End::Node(_) => true,
        };
        self.rods.retain(|r| ok(&r.a) && ok(&r.b));
    }

    /// Records the load on a rod the solver does not move (one pinned to a block or the ground, which
    /// the physics holds), and breaks it when overloaded for long enough.
    pub fn load_rod(&mut self, r: usize, force: f32) -> Option<Event> {
        let rod = self.rods.get_mut(r)?;
        rod.force = force;
        if force > rod.strength {
            rod.stress += 1;
            if rod.stress >= FATIGUE && !rod.broken {
                rod.broken = true;
                return Some(Event::RodBroke(rod.uid));
            }
        } else {
            rod.stress = rod.stress.saturating_sub(1);
        }
        None
    }

    /// Where an end of a rod is now (gears and carriages as they are; a node needs the game).
    pub fn end_point(&self, e: &End) -> Option<Vec2> {
        match e {
            End::Pin { gear, pin } => {
                let g = &self.gears[self.gear_index(*gear)?];
                Some(g.pos + rot(g.angle, *pin))
            }
            End::Carriage(u) => Some(self.sliders[self.slider_index(*u)?].point()),
            End::Node(Anchor::Free(p)) | End::Node(Anchor::Ground(p)) => Some(*p),
            End::Node(_) => None,
        }
    }

    /// Slider position that keeps a rod of its length, for a pin at `pin`; picks the solution
    /// nearest `near`. `None` if the rod cannot reach.
    fn solve_s(slider: &Slider, pin: Vec2, len: f32, near: f32) -> Option<f32> {
        let w = pin - slider.origin;
        let a = w.dot(slider.dir);
        let h = w.x * -slider.dir.y + w.y * slider.dir.x;
        let q = len * len - h * h;
        if q < 0.0 {
            return None;
        }
        let r = q.sqrt();
        let (s1, s2) = (a + r, a - r);
        Some(if (s1 - near).abs() <= (s2 - near).abs() { s1 } else { s2 })
    }

    /// The angle a gear (centre `c`, pin at distance `|pin|` and angle `atan2(pin)` in its own
    /// frame) must have for its pin to sit at distance `len` from `target`; the solution nearest
    /// `near`. `None` if the rod cannot reach.
    fn solve_theta(c: Vec2, pin: Vec2, len: f32, target: Vec2, near: f32) -> Option<f32> {
        let rho = pin.length();
        let w = target - c;
        let d = w.length();
        if rho < 1e-3 || d < 1e-3 || d > rho + len || d < (rho - len).abs() {
            return None;
        }
        let cos_d = ((rho * rho + d * d - len * len) / (2.0 * rho * d)).clamp(-1.0, 1.0);
        let delta = cos_d.acos();
        let phi = w.y.atan2(w.x);
        let alpha = pin.y.atan2(pin.x);
        let tau = std::f32::consts::TAU;
        let wrap = |a: f32| (a + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
        let (t1, t2) = (phi + delta - alpha, phi - delta - alpha);
        let (d1, d2) = (wrap(t1 - near).abs(), wrap(t2 - near).abs());
        Some(near + wrap(if d1 <= d2 { t1 } else { t2 } - near))
    }

    /// The two gears are close enough to mesh.
    fn meshes(a: &Gear, b: &Gear) -> bool {
        if a.stripped || b.stripped {
            return false;
        }
        let want = a.size.radius() + b.size.radius();
        ((a.pos - b.pos).length() - want).abs() <= MESH_TOLERANCE
    }

    /// Advances one step. Returns what happened.
    pub fn step(&mut self) -> Vec<Event> {
        let mut events = vec![];
        let n = self.gears.len();

        // The graph of who drives whom: edges carry the speed ratio w_j / w_i.
        let mut adj: Vec<Vec<(usize, f32)>> = vec![vec![]; n];
        for i in 0..n {
            for j in i + 1..n {
                if Self::meshes(&self.gears[i], &self.gears[j]) {
                    let (ri, rj) = (self.gears[i].size.radius(), self.gears[j].size.radius());
                    adj[i].push((j, -ri / rj));
                    adj[j].push((i, -rj / ri));
                }
            }
        }
        for ax in &self.axles {
            if let (Some(a), Some(b)) = (self.gear_index(ax.a), self.gear_index(ax.b)) {
                adj[a].push((b, 1.0));
                adj[b].push((a, 1.0));
            }
        }

        // Groups of linked gears, each with its speed ratios to the first.
        let mut comp = vec![usize::MAX; n];
        let mut k = vec![0.0f32; n];
        let mut groups: Vec<(Vec<usize>, bool)> = vec![];
        for root in 0..n {
            if comp[root] != usize::MAX {
                continue;
            }
            let id = groups.len();
            let mut members = vec![root];
            let mut jam = false;
            comp[root] = id;
            k[root] = 1.0;
            let mut head = 0;
            while head < members.len() {
                let i = members[head];
                head += 1;
                for &(j, ratio) in &adj[i] {
                    let want = k[i] * ratio;
                    if comp[j] == usize::MAX {
                        comp[j] = id;
                        k[j] = want;
                        members.push(j);
                    } else if (k[j] - want).abs() > 1e-3 {
                        jam = true;
                    }
                }
            }
            groups.push((members, jam));
        }
        for (members, jam) in &groups {
            for &m in members {
                if self.gears[m].jammed != *jam {
                    if *jam {
                        events.push(Event::Jammed(self.gears[m].uid));
                    }
                    self.gears[m].jammed = *jam;
                }
            }
        }

        let mut tooth_force = vec![0.0f32; n];
        let mut mesh_edges: Vec<(usize, usize)> = vec![];
        for i in 0..n {
            for &(j, ratio) in &adj[i] {
                if j > i && ratio < 0.0 {
                    mesh_edges.push((i, j));
                }
            }
        }

        // Rods the solver moves (both ends on a pin or a carriage that exist).
        let solver_rods: Vec<usize> = (0..self.rods.len())
            .filter(|&r| {
                let rod = &self.rods[r];
                !rod.broken && self.node_of(&rod.a, &comp).is_some() && self.node_of(&rod.b, &comp).is_some()
            })
            .collect();

        let mut rod_force: Vec<f32> = vec![0.0; self.rods.len()];
        let mut seen_g = vec![false; groups.len()];
        let mut seen_s = vec![false; self.sliders.len()];
        let has_motor = |this: &Mechanism, members: &[usize]| members.iter().any(|&m| this.gears[m].motor.map_or(false, |mo| mo.on));
        // Drivers first (groups with a running motor), then whatever is left stands on its own.
        let mut order: Vec<usize> = (0..groups.len()).filter(|&g| has_motor(self, &groups[g].0)).collect();
        order.extend((0..groups.len()).filter(|&g| !has_motor(self, &groups[g].0)));

        for root_group in order {
            if seen_g[root_group] {
                continue;
            }
            seen_g[root_group] = true;
            let tree = self.build_tree(root_group, &solver_rods, &comp, &mut seen_g, &mut seen_s);
            let ctx = Ctx { k: &k, comp: &comp, groups: &groups };
            self.drive_tree(&tree, &ctx, &mut rod_force, &mut tooth_force, &mut events);
        }

        // Sliders that nothing drives just coast to a stop.
        for (i, sl) in self.sliders.iter_mut().enumerate() {
            if seen_s[i] {
                continue;
            }
            sl.v += (sl.ext_force - 0.1 * sl.v) / sl.mass.max(0.1);
            let (lo, hi) = sl.range();
            sl.s += sl.v;
            if sl.s < lo || sl.s > hi {
                sl.s = sl.s.clamp(lo, hi);
                sl.v = 0.0;
            }
            sl.load = sl.ext_force.abs();
        }

        // Overloaded rods break.
        for (r, f) in rod_force.iter().enumerate() {
            let rod = &mut self.rods[r];
            rod.force = *f;
            if *f > rod.strength {
                rod.stress += 1;
                if rod.stress >= FATIGUE && !rod.broken {
                    rod.broken = true;
                    events.push(Event::RodBroke(rod.uid));
                }
            } else {
                rod.stress = rod.stress.saturating_sub(1);
            }
        }
        // Overloaded teeth strip (the weaker gear of the pair).
        let mut over: Vec<usize> = vec![];
        for &(i, j) in &mesh_edges {
            let (si, sj) = (self.gears[i].size.tooth_strength(), self.gears[j].size.tooth_strength());
            let limit = si.min(sj);
            let force = tooth_force[i].min(tooth_force[j]);
            self.gears[i].tooth_load = self.gears[i].tooth_load.max(force / limit);
            self.gears[j].tooth_load = self.gears[j].tooth_load.max(force / limit);
            if force > limit {
                over.push(if si <= sj { i } else { j });
            }
        }
        for i in 0..n {
            if over.contains(&i) {
                self.gears[i].stress += 1;
                if self.gears[i].stress >= FATIGUE && !self.gears[i].stripped {
                    self.gears[i].stripped = true;
                    events.push(Event::TeethStripped(self.gears[i].uid));
                }
            } else {
                self.gears[i].stress = self.gears[i].stress.saturating_sub(1);
            }
        }
        events
    }

    fn node_of(&self, e: &End, comp: &[usize]) -> Option<Node> {
        match e {
            End::Pin { gear, .. } => Some(Node::Group(comp[self.gear_index(*gear)?])),
            End::Carriage(u) => Some(Node::Slider(self.slider_index(*u)?)),
            End::Node(_) => None,
        }
    }

    /// Who follows whom, starting from a root group: rods lead from a known gear or carriage to one
    /// that has no motor and is not known yet, which then follows it. Rods between things that are
    /// both known already only check that the geometry stays consistent.
    fn build_tree(&self, root: usize, rods: &[usize], comp: &[usize], seen_g: &mut [bool], seen_s: &mut [bool]) -> Tree {
        let mut tree = Tree { root, edges: vec![], checks: vec![], groups: vec![root], sliders: vec![] };
        let mut used = vec![false; self.rods.len()];
        loop {
            let mut grew = false;
            for &r in rods {
                if used[r] {
                    continue;
                }
                let rod = &self.rods[r];
                let (na, nb) = (self.node_of(&rod.a, comp).unwrap(), self.node_of(&rod.b, comp).unwrap());
                let known = |n: Node, seen_g: &[bool], seen_s: &[bool]| match n {
                    Node::Group(g) => seen_g[g],
                    Node::Slider(s) => seen_s[s],
                };
                let (ka, kb) = (known(na, seen_g, seen_s), known(nb, seen_g, seen_s));
                // A group with a motor is never a follower.
                let free = |n: Node| match n {
                    Node::Group(g) => !self.gears.iter().enumerate().any(|(i, gr)| comp[i] == g && gr.motor.map_or(false, |m| m.on)),
                    Node::Slider(_) => true,
                };
                let (parent_is_a, child) = match (ka, kb) {
                    (true, false) if free(nb) => (true, nb),
                    (false, true) if free(na) => (false, na),
                    _ => continue,
                };
                used[r] = true;
                grew = true;
                match child {
                    Node::Group(g) => {
                        seen_g[g] = true;
                        tree.groups.push(g);
                    }
                    Node::Slider(s) => {
                        seen_s[s] = true;
                        tree.sliders.push(s);
                    }
                }
                tree.edges.push(Edge { rod: r, parent_is_a, child });
            }
            if !grew {
                break;
            }
        }
        for &r in rods {
            if !used[r] {
                let rod = &self.rods[r];
                let (na, nb) = (self.node_of(&rod.a, comp).unwrap(), self.node_of(&rod.b, comp).unwrap());
                let in_tree = |n: Node| match n {
                    Node::Group(g) => tree.groups.contains(&g),
                    Node::Slider(s) => tree.sliders.contains(&s),
                };
                if in_tree(na) && in_tree(nb) {
                    tree.checks.push(r);
                }
            }
        }
        tree
    }

    /// The pose of everything in a tree when its root group has turned by `dq` (relative to now):
    /// followers are solved edge by edge. `None` if a rod cannot reach, a carriage would leave its
    /// rail, or a rod that only checks no longer fits.
    fn pose(&self, tree: &Tree, ctx: &Ctx, dq: f32) -> Option<Pose> {
        let mut p = Pose { theta: self.gears.iter().map(|g| g.angle).collect(), s: self.sliders.iter().map(|s| s.s).collect() };
        for &m in &ctx.groups[tree.root].0 {
            p.theta[m] += ctx.k[m] * dq;
        }
        for e in &tree.edges {
            let rod = &self.rods[e.rod];
            let (from, to) = if e.parent_is_a { (&rod.a, &rod.b) } else { (&rod.b, &rod.a) };
            let parent_point = self.end_point_in(from, &p)?;
            match (e.child, to) {
                (Node::Slider(si), _) => {
                    let sl = &self.sliders[si];
                    let s = Self::solve_s(sl, parent_point, rod.len, sl.s)?;
                    let (lo, hi) = sl.range();
                    if s < lo - 1e-3 || s > hi + 1e-3 {
                        return None;
                    }
                    p.s[si] = s;
                }
                (Node::Group(_), End::Pin { gear, pin }) => {
                    let gj = self.gear_index(*gear)?;
                    let g = &self.gears[gj];
                    let th = Self::solve_theta(g.pos, *pin, rod.len, parent_point, g.angle)?;
                    let delta = th - g.angle;
                    for &i in &ctx.groups[ctx.comp[gj]].0 {
                        p.theta[i] = self.gears[i].angle + ctx.k[i] / ctx.k[gj] * delta;
                    }
                }
                _ => return None,
            }
        }
        // End stops on gears (a winch with no rope left, a valve at the end of its travel).
        for &g in &tree.groups {
            for &i in &ctx.groups[g].0 {
                if let Some((lo, hi)) = self.gears[i].limit {
                    if p.theta[i] < lo - 1e-4 || p.theta[i] > hi + 1e-4 {
                        return None;
                    }
                }
            }
        }
        for &r in &tree.checks {
            let rod = &self.rods[r];
            let (a, b) = (self.end_point_in(&rod.a, &p)?, self.end_point_in(&rod.b, &p)?);
            if ((a - b).length() - rod.len).abs() > 0.7 {
                return None;
            }
        }
        Some(p)
    }

    fn end_point_in(&self, e: &End, p: &Pose) -> Option<Vec2> {
        match e {
            End::Pin { gear, pin } => {
                let i = self.gear_index(*gear)?;
                Some(self.gears[i].pos + rot(p.theta[i], *pin))
            }
            End::Carriage(u) => {
                let i = self.slider_index(*u)?;
                Some(self.sliders[i].origin + self.sliders[i].dir * p.s[i])
            }
            End::Node(Anchor::Free(q)) | End::Node(Anchor::Ground(q)) => Some(*q),
            End::Node(_) => None,
        }
    }

    /// Moves one tree: motor and load torques are turned into one equation for the root angle
    /// `q`, with every gear and carriage in the tree following it (`dtheta/dq`, `ds/dq`).
    fn drive_tree(&mut self, tree: &Tree, ctx: &Ctx, rod_force: &mut [f32], tooth_force: &mut [f32], events: &mut Vec<Event>) {
        let root_members = &ctx.groups[tree.root].0;
        let jam = ctx.groups[tree.root].1;
        let qd = self.gears[root_members[0]].omega / ctx.k[root_members[0]];
        let eps = 1e-3;
        let base = self.pose(tree, ctx, 0.0);
        let plus = self.pose(tree, ctx, eps);
        let minus = self.pose(tree, ctx, -eps);

        // All gears and carriages of the tree.
        let tree_gears: Vec<usize> = tree.groups.iter().flat_map(|&g| ctx.groups[g].0.iter().copied()).collect();
        // How each follows q.
        let (mut dth, mut dsl) = (vec![0.0f32; self.gears.len()], vec![0.0f32; self.sliders.len()]);
        for &i in &root_members.clone() {
            dth[i] = ctx.k[i];
        }
        let followers: Vec<usize> = tree_gears.iter().copied().filter(|&i| ctx.comp[i] != tree.root).collect();
        let tree_gears_fd = followers;
        if let (Some(a), Some(b)) = (&plus, &minus) {
            for &i in &tree_gears_fd {
                dth[i] = (a.theta[i] - b.theta[i]) / (2.0 * eps);
            }
            for &si in &tree.sliders {
                dsl[si] = (a.s[si] - b.s[si]) / (2.0 * eps);
            }
        } else if let (Some(a), Some(b)) = (&plus, &base) {
            for &i in &tree_gears_fd {
                dth[i] = (a.theta[i] - b.theta[i]) / eps;
            }
            for &si in &tree.sliders {
                dsl[si] = (a.s[si] - b.s[si]) / eps;
            }
        } else if let (Some(a), Some(b)) = (&base, &minus) {
            for &i in &tree_gears_fd {
                dth[i] = (a.theta[i] - b.theta[i]) / eps;
            }
            for &si in &tree.sliders {
                dsl[si] = (a.s[si] - b.s[si]) / eps;
            }
        }

        let mut inertia = 0.0f32;
        let (mut motor_torque, mut load, mut friction) = (0.0f32, 0.0f32, 0.0f32);
        for &i in &tree_gears {
            let g = &self.gears[i];
            inertia += g.size.inertia() * dth[i] * dth[i];
            friction -= 0.01 * g.size.inertia() * dth[i] * dth[i] * qd;
            load += g.ext_torque * dth[i];
            if tree.groups[0] == tree.root {
                if let Some(mo) = g.motor.filter(|mo| mo.on) {
                    if ctx.comp[i] == tree.root {
                        let w = dth[i] * qd;
                        motor_torque += mo.torque * mo.dir * (1.0 - mo.dir * w / mo.free_speed).clamp(-0.5, 1.0) * dth[i];
                    }
                }
            }
        }
        for &si in &tree.sliders {
            let sl = &self.sliders[si];
            inertia += sl.mass * dsl[si] * dsl[si];
            friction -= 0.05 * sl.mass * dsl[si] * dsl[si] * qd;
            load += sl.ext_force * dsl[si];
        }
        let total = motor_torque + friction + load;
        let (qdd, new_qd) = if jam || base.is_none() {
            (0.0, 0.0)
        } else {
            let a = total / inertia.max(1e-3);
            (a, qd + a)
        };

        // Move as far as the stops and the reach of the rods allow.
        let mut dq = new_qd;
        let mut stalled = false;
        if jam || base.is_none() {
            dq = 0.0;
        } else if self.pose(tree, ctx, new_qd).is_none() {
            stalled = true;
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for _ in 0..14 {
                let mid = (lo + hi) / 2.0;
                if self.pose(tree, ctx, new_qd * mid).is_some() {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            dq = new_qd * lo;
        }
        let final_qd = if stalled || jam || base.is_none() { 0.0 } else { new_qd };

        // A motor set to reverse backs off when it is struggling.
        for &m in root_members {
            let load_at_motor = load.abs() / ctx.k[m].abs().max(1e-3);
            if let Some(mo) = &mut self.gears[m].motor {
                mo.cooldown = mo.cooldown.saturating_sub(1);
                if mo.auto_reverse && mo.on {
                    if stalled || load_at_motor > mo.torque * mo.reverse_at {
                        mo.overload += 1;
                    } else {
                        mo.overload = 0;
                    }
                    if mo.overload >= 3 && mo.cooldown == 0 {
                        mo.dir = -mo.dir;
                        mo.overload = 0;
                        mo.cooldown = 40;
                    }
                }
            }
        }

        let old_s: Vec<f32> = self.sliders.iter().map(|s| s.s).collect();
        if dq != 0.0 {
            if let Some(p) = self.pose(tree, ctx, dq) {
                for &i in &tree_gears {
                    self.gears[i].angle = p.theta[i];
                }
                for &si in &tree.sliders {
                    self.sliders[si].s = p.s[si];
                }
            }
        }
        for &i in &tree_gears {
            self.gears[i].omega = if jam { 0.0 } else { dth[i] * final_qd };
        }
        // Torque that has to go through the teeth: the motor's push or the load pushing back,
        // plus the shove of a flywheel hitting a stop.
        let impact = if stalled { inertia * new_qd.abs() / STOP_TICKS } else { 0.0 };
        let transmitted = motor_torque.abs().max(load.abs()) + impact;
        for &i in &tree_gears {
            let t = transmitted / dth[i].abs().max(1e-3);
            tooth_force[i] = t / self.gears[i].size.radius();
            self.gears[i].shaft_torque = motor_torque.abs() / dth[i].abs().max(1e-3);
            self.gears[i].torque = t;
            self.gears[i].tooth_load = 0.0;
        }
        for &m in root_members {
            if let Some(mo) = self.gears[m].motor {
                self.gears[m].motor_load = (load.abs() / ctx.k[m].abs().max(1e-3) / mo.torque.max(1e-3)).min(2.0);
            }
        }

        // Carriages: speed, and the load they carry.
        let mut blocked: Vec<usize> = vec![];
        for &si in &tree.sliders {
            let sl = &mut self.sliders[si];
            let a = sl.s - old_s[si] - sl.v;
            sl.v = sl.s - old_s[si];
            let (lo, hi) = sl.range();
            if sl.s <= lo + 0.01 || sl.s >= hi - 0.01 {
                blocked.push(si);
            }
            sl.load = (sl.mass * a - sl.ext_force).abs();
        }

        // Forces in the rods: the power that flows through each one, from virtual work.
        let after = if dq != 0.0 { self.pose(tree, ctx, 0.0) } else { base.clone() };
        let _ = after;
        // The load carried by everything below an edge (children's contributions accumulate).
        let mut below_g = vec![0.0f32; ctx.groups.len()];
        let mut below_s = vec![0.0f32; self.sliders.len()];
        for &g in &tree.groups {
            for &i in &ctx.groups[g].0 {
                below_g[g] += (self.gears[i].ext_torque * dth[i]).abs() + self.gears[i].size.inertia() * dth[i] * dth[i] * qdd.abs();
            }
        }
        for &si in &tree.sliders {
            let sl = &self.sliders[si];
            below_s[si] = (sl.ext_force * dsl[si]).abs() + sl.mass * dsl[si] * dsl[si] * qdd.abs();
        }
        let mut carried_g = below_g.clone();
        let mut carried_s = below_s.clone();
        for e in tree.edges.iter().rev() {
            let rod = &self.rods[e.rod];
            let (from, to) = if e.parent_is_a { (&rod.a, &rod.b) } else { (&rod.b, &rod.a) };
            let child_sum = match e.child {
                Node::Group(g) => carried_g[g],
                Node::Slider(s) => carried_s[s],
            };
            // The parent receives what the child carries.
            match self.node_of(from, ctx.comp) {
                Some(Node::Group(g)) => carried_g[g] += child_sum,
                Some(Node::Slider(s)) => carried_s[s] += child_sum,
                None => {}
            }
            let stopped_below = stalled && match e.child {
                Node::Slider(s) => blocked.contains(&s) || blocked.is_empty(),
                Node::Group(_) => true,
            };
            let q_sub = if stopped_below { motor_torque.abs() + impact } else { child_sum };
            // How fast the child end of the rod moves along the rod per unit of q.
            let (pc, mc) = match (plus.as_ref().and_then(|p| self.end_point_in(to, p)), minus.as_ref().and_then(|p| self.end_point_in(to, p))) {
                (Some(a), Some(b)) => (a, b),
                _ => continue,
            };
            let (pa, pb) = match (self.end_point(from), self.end_point(to)) {
                (Some(a), Some(b)) => (a, b),
                _ => continue,
            };
            let dir = (pa - pb).normalize_or_zero();
            let dproj = ((pc - mc) / (2.0 * eps)).dot(dir).abs();
            rod_force[e.rod] = q_sub / dproj.max(1e-3);
        }
        if stalled {
            let uid = blocked.first().map_or(self.gears[root_members[0]].uid, |&s| self.sliders[s].uid);
            events.push(Event::Stalled(uid));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Node {
    Group(usize),
    Slider(usize),
}

struct Edge {
    rod: usize,
    /// Is the known end of the rod its `a` end?
    parent_is_a: bool,
    child: Node,
}

struct Tree {
    root: usize,
    edges: Vec<Edge>,
    /// Rods between two things that are already in the tree: they only have to stay consistent.
    checks: Vec<usize>,
    groups: Vec<usize>,
    sliders: Vec<usize>,
}

struct Ctx<'a> {
    k: &'a [f32],
    comp: &'a [usize],
    groups: &'a [(Vec<usize>, bool)],
}

#[derive(Clone)]
struct Pose {
    theta: Vec<f32>,
    s: Vec<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn motor_gear(uid: u32, size: GearSize, at: Vec2) -> Gear {
        let mut g = Gear::new(uid, size, at);
        g.motor = Some(Motor::new());
        g
    }

    fn run(m: &mut Mechanism, steps: usize) -> Vec<Event> {
        let mut all = vec![];
        for _ in 0..steps {
            all.extend(m.step());
        }
        all
    }

    #[test]
    fn meshed_gears_turn_with_the_exact_ratio_and_opposite_sense() {
        let mut m = Mechanism::default();
        m.gears.push(motor_gear(1, GearSize::Small, vec2(100.0, 100.0)));
        m.gears.push(Gear::new(2, GearSize::Large, vec2(120.0, 100.0))); // 6 + 14
        run(&mut m, 200);
        let (a, b) = (&m.gears[0], &m.gears[1]);
        assert!(a.omega > 0.05, "motor gear did not spin up: {}", a.omega);
        assert!((b.omega / a.omega + 6.0 / 14.0).abs() < 1e-4, "ratio {} vs {}", b.omega, a.omega);
        assert!((b.angle / a.angle + 6.0 / 14.0).abs() < 1e-3);
    }

    #[test]
    fn gears_that_are_apart_do_not_mesh_and_a_released_one_spins_free_then_stops() {
        let mut m = Mechanism::default();
        m.gears.push(motor_gear(1, GearSize::Small, vec2(100.0, 100.0)));
        m.gears.push(Gear::new(2, GearSize::Medium, vec2(120.0, 100.0))); // 6 + 10 = 16: too far
        run(&mut m, 100);
        assert_eq!(m.gears[1].omega, 0.0);
        // Mesh it, spin it up, then pull the support away: it keeps turning, then friction wins.
        m.gears[1].pos = vec2(116.0, 100.0);
        run(&mut m, 200);
        let w = m.gears[1].omega.abs();
        assert!(w > 0.03);
        m.gears[0].motor = None;
        m.gears[1].pos = vec2(140.0, 100.0);
        // The driven gear is now alone: friction slows it.
        run(&mut m, 600);
        assert!(m.gears[1].omega.abs() < w * 0.5, "free gear keeps its speed: {}", m.gears[1].omega);
    }

    #[test]
    fn axle_makes_two_far_gears_turn_together() {
        let mut m = Mechanism::default();
        m.gears.push(motor_gear(1, GearSize::Small, vec2(100.0, 100.0)));
        m.gears.push(Gear::new(2, GearSize::Medium, vec2(300.0, 100.0)));
        m.axles.push(Axle { uid: 0, a: 1, b: 2 });
        run(&mut m, 100);
        assert!((m.gears[0].omega - m.gears[1].omega).abs() < 1e-6);
        assert!(m.gears[1].omega > 0.01);
    }

    #[test]
    fn odd_ring_of_gears_jams() {
        let mut m = Mechanism::default();
        let r = GearSize::Medium.radius() * 2.0;
        m.gears.push(motor_gear(1, GearSize::Medium, vec2(100.0, 100.0)));
        m.gears.push(Gear::new(2, GearSize::Medium, vec2(100.0 + r, 100.0)));
        m.gears.push(Gear::new(3, GearSize::Medium, vec2(100.0 + r / 2.0, 100.0 + r * 0.866)));
        let ev = run(&mut m, 20);
        assert!(ev.iter().any(|e| matches!(e, Event::Jammed(_))));
        assert!(m.gears.iter().all(|g| g.omega == 0.0));
    }

    /// Gear at (100,100) with a pin 8 cells out, rod 30 long to a slider on a horizontal rail.
    fn crank_slider() -> Mechanism {
        let mut m = Mechanism::default();
        m.gears.push(motor_gear(1, GearSize::Medium, vec2(100.0, 100.0)));
        m.sliders.push(Slider::new(1, vec2(100.0, 100.0), vec2(180.0, 100.0)));
        m.sliders[0].s = 38.0;
        m.rods.push(Rod::new(1, 1, vec2(8.0, 0.0), 1, 30.0));
        m
    }

    #[test]
    fn crank_slider_follows_the_exact_geometry() {
        let mut m = crank_slider();
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for _ in 0..600 {
            m.step();
            let g = &m.gears[0];
            let pin = g.pos + rot(g.angle, vec2(8.0, 0.0));
            let s = m.sliders[0].s;
            let d = (pin - m.sliders[0].point()).length();
            assert!((d - 30.0).abs() < 0.05, "rod length drifted: {d}");
            if m.gears[0].angle > 3.0 {
                lo = lo.min(s);
                hi = hi.max(s);
            }
        }
        // Stroke of a crank-slider is twice the crank radius.
        assert!((hi - lo - 16.0).abs() < 0.8, "stroke {}", hi - lo);
        assert!(m.rods[0].broken == false);
    }

    #[test]
    fn slider_at_its_stop_stalls_the_motor() {
        let mut m = crank_slider();
        // The rail ends before the crank's full stroke: the slider cannot follow.
        // The slider reaches 22..38 along the rail; the rail only allows up to 35.
        m.sliders[0].len = 38.0;
        m.sliders[0].s = 22.0;
        m.rods[0].strength = 1e6;
        m.gears[0].angle = std::f32::consts::PI;
        let ev = run(&mut m, 400);
        assert!(ev.iter().any(|e| matches!(e, Event::Stalled(_))), "never stalled");
        assert!(m.gears[0].omega.abs() < 1e-6, "still turning: {}", m.gears[0].omega);
        let s = m.sliders[0].s;
        assert!(s >= m.sliders[0].margin - 1e-3 && s <= m.sliders[0].len - m.sliders[0].margin + 1e-3);
    }

    #[test]
    fn a_weak_rod_tears_under_load_and_a_strong_one_holds() {
        for (strength, should_break) in [(0.5, true), (1e6, false)] {
            let mut m = crank_slider();
            m.rods[0].strength = strength;
            m.sliders[0].ext_force = -3.0; // something heavy in the way
            let ev = run(&mut m, 400);
            let broke = ev.iter().any(|e| matches!(e, Event::RodBroke(_)));
            assert_eq!(broke, should_break, "strength {strength}");
        }
    }

    #[test]
    fn a_broken_rod_frees_the_slider_and_the_gear() {
        let mut m = crank_slider();
        m.rods[0].broken = true;
        run(&mut m, 100);
        assert!(m.gears[0].omega > 0.05, "gear should spin freely");
        assert!(m.sliders[0].v.abs() < 1e-3);
    }

    #[test]
    fn overloaded_teeth_strip_and_the_drive_is_lost() {
        let mut m = Mechanism::default();
        m.gears.push(motor_gear(1, GearSize::Small, vec2(100.0, 100.0)));
        m.gears.push(Gear::new(2, GearSize::Medium, vec2(116.0, 100.0)));
        m.axles.push(Axle { uid: 0, a: 2, b: 3 });
        m.gears.push(Gear::new(3, GearSize::Small, vec2(300.0, 300.0)));
        m.sliders.push(Slider::new(1, vec2(290.0, 300.0), vec2(400.0, 300.0)));
        m.sliders[0].s = 50.0;
        m.rods.push(Rod::new(1, 3, vec2(5.0, 0.0), 1, 40.0));
        m.rods[0].strength = 1e6;
        m.gears[2].angle = std::f32::consts::FRAC_PI_2; // away from dead centre
        m.sliders[0].ext_force = 25.0; // brutal load against the stroke
        let ev = run(&mut m, 500);
        assert!(ev.iter().any(|e| matches!(e, Event::TeethStripped(_))), "teeth never stripped: {ev:?}");
        assert!(m.gears.iter().any(|g| g.stripped));
    }

    #[test]
    fn a_rod_between_two_gears_makes_a_crank_rocker() {
        let mut m = Mechanism::default();
        m.gears.push(motor_gear(1, GearSize::Small, vec2(100.0, 100.0)));
        m.gears.push(Gear::new(2, GearSize::Medium, vec2(140.0, 100.0)));
        // A short crank (4 cells) drives a longer one (8 cells) through a rod: the second gear rocks.
        m.rods.push(Rod::between(1, End::Pin { gear: 1, pin: vec2(0.0, -4.0) }, End::Pin { gear: 2, pin: vec2(0.0, -8.0) }, 36.0));
        m.rods[0].strength = 1e6;
        let (mut worst, mut lo, mut hi) = (0.0f32, f32::MAX, f32::MIN);
        for _ in 0..1000 {
            m.step();
            let (pa, pb) = (m.end_point(&m.rods[0].a).unwrap(), m.end_point(&m.rods[0].b).unwrap());
            worst = worst.max(((pa - pb).length() - 36.0).abs());
            lo = lo.min(m.gears[1].angle);
            hi = hi.max(m.gears[1].angle);
        }
        assert!(worst < 0.05, "the rod keeps its length: {worst}");
        assert!(m.gears[0].angle > 6.0, "the driver keeps turning: {}", m.gears[0].angle);
        assert!(hi - lo > 0.1 && hi - lo < 3.5, "the follower rocks to and fro: {lo}..{hi}");
    }

    #[test]
    fn a_second_rod_on_a_carriage_makes_a_gear_follow_it() {
        let mut m = crank_slider();
        m.rods[0].strength = 1e6;
        m.gears.push(Gear::new(2, GearSize::Small, vec2(160.0, 60.0)));
        let pin = vec2(5.0, 0.0);
        let at_start = m.sliders[0].point();
        let len = (vec2(160.0, 60.0) + pin - at_start).length();
        m.rods.push(Rod::between(2, End::Carriage(1), End::Pin { gear: 2, pin }, len));
        m.rods[1].strength = 1e6;
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut worst = 0.0f32;
        for _ in 0..800 {
            m.step();
            let (pa, pb) = (m.end_point(&m.rods[1].a).unwrap(), m.end_point(&m.rods[1].b).unwrap());
            worst = worst.max(((pa - pb).length() - len).abs());
            lo = lo.min(m.gears[1].angle);
            hi = hi.max(m.gears[1].angle);
        }
        assert!(worst < 0.05, "the second rod keeps its length too: {worst}");
        assert!(hi - lo > 0.05, "the follower gear swings with the carriage: {lo}..{hi}");
        assert!(m.rods.iter().all(|r| !r.broken));
    }

    #[test]
    fn a_gear_with_an_end_stop_stalls_the_motor_after_so_many_turns() {
        let mut m = Mechanism::default();
        m.gears.push(motor_gear(1, GearSize::Medium, vec2(100.0, 100.0)));
        let turns = 3.0;
        m.gears[0].limit = Some((0.0, turns * std::f32::consts::TAU));
        let ev = run(&mut m, 1200);
        let a = m.gears[0].angle;
        assert!((a - turns * std::f32::consts::TAU).abs() < 0.02, "it stops at the end of its travel: {a}");
        assert!(ev.iter().any(|e| matches!(e, Event::Stalled(_))), "and the motor is told it is stuck");
        assert!(m.gears[0].omega.abs() < 1e-5);
        // A reversing motor turns back from the stop and runs to the other end.
        let mut m = Mechanism::default();
        m.gears.push(motor_gear(1, GearSize::Medium, vec2(100.0, 100.0)));
        m.gears[0].motor = Some(Motor::reversing());
        m.gears[0].limit = Some((0.0, turns * std::f32::consts::TAU));
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for _ in 0..3000 {
            m.step();
            lo = lo.min(m.gears[0].angle);
            hi = hi.max(m.gears[0].angle);
        }
        assert!(hi > turns * std::f32::consts::TAU - 0.1 && lo < 0.5, "it goes back and forth between the stops: {lo}..{hi}");
    }

    #[test]
    fn reversing_motor_backs_off_a_stop_and_a_plain_one_stays_stuck() {
        for (auto, should_turn_back) in [(true, true), (false, false)] {
            let mut m = crank_slider();
            m.sliders[0].len = 38.0;
            m.sliders[0].s = 22.0;
            m.gears[0].angle = std::f32::consts::PI;
            m.rods[0].strength = 1e6;
            m.gears[0].motor = Some(if auto { Motor::reversing() } else { Motor::new() });
            let mut reversed = false;
            for _ in 0..800 {
                m.step();
                reversed |= m.gears[0].motor.unwrap().dir < 0.0;
            }
            assert_eq!(reversed, should_turn_back, "auto_reverse={auto}");
        }
    }

    #[test]
    fn reversing_motor_also_backs_off_a_heavy_load() {
        let mut m = crank_slider();
        m.rods[0].strength = 1e6;
        m.sliders[0].ext_force = -4.0;
        m.gears[0].motor = Some(Motor::reversing());
        let mut flips = 0;
        let mut last = 1.0;
        for _ in 0..1500 {
            m.step();
            let d = m.gears[0].motor.unwrap().dir;
            if d != last {
                flips += 1;
                last = d;
            }
        }
        assert!(flips >= 1, "never reversed under load");
    }

    #[test]
    fn a_flywheel_carries_the_crank_through_dead_centre() {
        // Weak motor, heavy load: a small gear stalls at dead centre, a big flywheel on the
        // shaft gets through.
        let mut stalled_at = [0.0f32; 2];
        for (i, with_flywheel) in [false, true].into_iter().enumerate() {
            let mut m = crank_slider();
            m.gears[0].motor = Some(Motor { torque: 6.0, ..Motor::new() });
            if with_flywheel {
                m.gears.push(Gear::new(2, GearSize::Large, vec2(500.0, 500.0)));
                m.axles.push(Axle { uid: 0, a: 1, b: 2 });
            }
            m.rods[0].strength = 1e6;
            m.sliders[0].ext_force = -0.9; // peak torque 7.2 > the 6 the motor can give
            run(&mut m, 1500);
            stalled_at[i] = m.gears[0].angle;
        }
        assert!(stalled_at[0] < 6.3, "plain crank should stick: {stalled_at:?}");
        assert!(stalled_at[1] > stalled_at[0] + 6.3, "flywheel did not carry it through: {stalled_at:?}");
    }
}
