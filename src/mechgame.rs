//! The mechanism tab: placing gears, motors, axles, rails and connecting rods, running them
//! in the simulation, and drawing them. The physics itself lives in `mech.rs`.

use super::*;
use crate::blocks::anchor_at;
use crate::blocks::{Physics, ReelRef, PX_PER_M};
use crate::mech::{self, Axle, End, Event, Gear, GearSize, Motor, Pivot, Pulley, Rod, Slider};

/// A multi-click build in progress.
#[derive(Clone, Copy)]
pub enum MechDraft {
    /// First gear of an axle.
    Axle(u32),
    /// A wheel picked as the end of a shaft, waiting for its gear.
    AxleWheel(u32),
    /// A valve picked, waiting for the gear or rail that turns it; or a rail picked, waiting for the valve.
    AxleValve(u32),
    AxleSlider(u32),
    /// A pump picked, waiting for the gear that turns it.
    AxlePump(u32),
    /// A tige (uid, the end it swings about) picked as an arm, waiting for the gear that swings it.
    AxleArm(u32, u8),
    /// The gear of a lead screw, waiting for its rail; or the rail, waiting for its gear.
    ScrewGear(u32),
    ScrewRail(u32),
    /// First end of a rail.
    Rail(Anchor, Vec2),
    /// The first end of a connecting rod (a pin on a gear, a carriage, or any fixed point).
    Crank { first: End },
}

const IRON_OR_STEEL: &[Mat] = &[Mat::Iron, Mat::Steel];
/// Pressure above the air's (atm) at which a steam motor gives its full torque.
const STEAM_FULL_ATM: f32 = 2.0;
/// Cells of steam a steam motor uses up per radian, at full power.
const STEAM_USE: f32 = 0.8;
const RAIL_MIN: f32 = 24.0;
const RAIL_MAX: f32 = 220.0;
const AXLE_MAX: f32 = 400.0;
/// Radius of a winch drum and of a pulley sheave, in cells; rope a winch can hold beyond what it has.
const WINCH_DRUM: f32 = 7.0;
const PULLEY_R: f32 = 5.0;
const REEL_SPARE: f32 = 80.0;
const REEL_MIN: f32 = 14.0;
/// Wheel torque (physics units) for one unit of mechanism torque, and what a gear can always hold.
const WHEEL_TORQUE_PER_UNIT: f32 = 60.0;
const WHEEL_HOLD: f32 = 120.0;
/// How fast an arm that lags behind its gear catches up: this share of the lag per step.
const ARM_CATCH_UP: f32 = 0.15;

/// A threaded shaft the length of its rail.
fn screw_cost(sl: &Slider) -> u32 {
    8 + (sl.len * 0.1).ceil() as u32
}

/// The end of a rail nearest to `p`: where the screw's shaft comes in.
fn screw_end(sl: &Slider, p: Vec2) -> Vec2 {
    let (a, b) = (sl.origin, sl.origin + sl.dir * sl.len);
    if (a - p).length() <= (b - p).length() {
        a
    } else {
        b
    }
}

/// `a` brought back between -pi and pi.
fn wrap_angle(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

fn gear_cost(size: GearSize) -> u32 {
    (size.radius() * size.radius() * 0.18).ceil() as u32
}

fn teeth(size: GearSize) -> usize {
    match size {
        GearSize::Small => 8,
        GearSize::Medium => 12,
        GearSize::Large => 16,
    }
}

pub(crate) fn gear_name(size: GearSize) -> &'static str {
    match size {
        GearSize::Small => "Petit engrenage",
        GearSize::Medium => "Engrenage moyen",
        GearSize::Large => "Grand engrenage",
    }
}

pub(crate) fn point_to_segment(p: Vec2, a: Vec2, b: Vec2) -> (f32, f32) {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    ((p - (a + ab * t)).length(), t)
}

/// A shaft between `a` and `b` (screen pixels). Bright dashes run along it as it turns, so that a
/// spinning shaft is visibly spinning (the dashes slide with `phase`, in radians).
fn draw_shaft(a: Vec2, b: Vec2, phase: f32, thickness: f32) {
    draw_line(a.x, a.y, b.x, b.y, thickness * 1.15, Color::from_rgba(120, 124, 140, 255));
    draw_line(a.x, a.y, b.x, b.y, thickness * 0.45, Color::from_rgba(176, 180, 196, 255));
    let d = b - a;
    let len = d.length();
    if len < 1.0 {
        return;
    }
    let dir = d / len;
    let side = vec2(-dir.y, dir.x);
    let spacing = (thickness * 2.6).max(7.0);
    let mut t = (phase * 2.2).rem_euclid(spacing);
    while t < len {
        let c = a + dir * t;
        // Slanted ticks: a barber's pole running along the shaft.
        let (p, q) = (c - side * thickness * 0.5 - dir * thickness * 0.3, c + side * thickness * 0.5 + dir * thickness * 0.3);
        draw_line(p.x, p.y, q.x, q.y, (thickness * 0.35).max(1.2), Color::from_rgba(244, 246, 252, 235));
        t += spacing;
    }
}

/// A threaded shaft from `a` to `b` (screen pixels): the thread, `pitch` apart, slides along by
/// `travel` as the screw turns (so it keeps pace with its nut).
fn draw_screw(a: Vec2, b: Vec2, travel: f32, thickness: f32, pitch: f32) {
    draw_line(a.x, a.y, b.x, b.y, thickness, Color::from_rgba(96, 100, 118, 255));
    let d = b - a;
    let len = d.length();
    if len < 1.0 || pitch < 0.5 {
        return;
    }
    let dir = d / len;
    let side = vec2(-dir.y, dir.x);
    let mut t = travel.rem_euclid(pitch);
    while t < len {
        let c = a + dir * t;
        let (p, q) = (c - side * thickness * 0.5 - dir * pitch * 0.25, c + side * thickness * 0.5 + dir * pitch * 0.25);
        draw_line(p.x, p.y, q.x, q.y, (thickness * 0.3).max(1.2), Color::from_rgba(196, 200, 214, 255));
        t += pitch;
    }
    for p in [a, b] {
        draw_circle(p.x, p.y, thickness * 0.7, Color::from_rgba(200, 204, 216, 255));
        draw_circle(p.x, p.y, thickness * 0.3, Color::from_rgba(50, 54, 68, 255));
    }
}

/// A gear with teeth, drawn from its centre in screen pixels.
pub fn draw_gear(c: Vec2, r: f32, angle: f32, size: GearSize, tint: Color) {
    let n = teeth(size);
    let root = r * 0.84;
    let w = std::f32::consts::PI / n as f32;
    let body = Color::new(0.38 * tint.r + 0.1, 0.42 * tint.g + 0.1, 0.55 * tint.b + 0.1, tint.a);
    let edge = Color::new((body.r + 0.2).min(1.0), (body.g + 0.2).min(1.0), (body.b + 0.2).min(1.0), tint.a);
    for k in 0..n {
        let a = angle + k as f32 * 2.0 * w;
        let pts = [(a - w, root), (a - w * 0.45, r), (a + w * 0.45, r), (a + w, root)];
        let p: Vec<Vec2> = pts.iter().map(|(t, rad)| c + vec2(t.cos(), t.sin()) * *rad).collect();
        draw_triangle(c, p[0], p[1], edge);
        draw_triangle(c, p[1], p[2], edge);
        draw_triangle(c, p[2], p[3], edge);
    }
    draw_circle(c.x, c.y, root, body);
    // Three spokes and a hub, so a turning gear is visibly turning.
    for k in 0..3 {
        let a = angle + k as f32 * std::f32::consts::TAU / 3.0;
        let tip = c + vec2(a.cos(), a.sin()) * root * 0.86;
        draw_line(c.x, c.y, tip.x, tip.y, (r * 0.16).max(1.5), Color::new(0.1, 0.12, 0.18, 0.55 * tint.a));
    }
    draw_circle(c.x, c.y, r * 0.2, Color::new(0.14, 0.16, 0.22, tint.a));
    draw_circle(c.x, c.y, r * 0.08, Color::new(0.8, 0.82, 0.9, tint.a));
}

impl Game {
    fn mech_pay(&mut self, units: u32, near: Vec2) -> bool {
        if self.free_now() {
            return true;
        }
        self.census = self.world.census();
        let have = self.stock(IRON_OR_STEEL);
        if have < units {
            self.say(format!("Pas assez de fer ou d'acier : il en faut {units}, il y en a {have}"));
            return false;
        }
        self.world.consume(IRON_OR_STEEL, units, (near.x as i32, near.y as i32));
        true
    }

    /// What the selected mechanism tool would cost right now (for the red "missing" marks).
    pub fn mech_cost(&self, at: Vec2) -> Option<(&'static [Mat], u32)> {
        let units = match self.tool {
            Tool::Gear(s) => gear_cost(s),
            Tool::Motor => gear_cost(GearSize::Small) + 12,
            Tool::Axle => match self.mech_draft {
                Some(MechDraft::Axle(g)) => self.mech.gear_index(g).map_or(6, |i| ((self.mech.gears[i].pos - at).length() * 0.12).ceil().max(4.0) as u32),
                _ => 6,
            },
            Tool::Rail => match self.mech_draft {
                Some(MechDraft::Rail(_, from)) => ((at - from).length().min(RAIL_MAX) * 0.2).ceil() as u32,
                _ => 8,
            },
            Tool::Crank => 6,
            Tool::Pivot => 4,
            Tool::Screw => match self.mech_draft {
                Some(MechDraft::ScrewRail(r)) => self.mech.slider_index(r).map_or(8, |i| screw_cost(&self.mech.sliders[i])),
                _ => 8,
            },
            Tool::Winch => gear_cost(GearSize::Medium) + 10,
            Tool::Pulley => 4,
            _ => return None,
        };
        Some((IRON_OR_STEEL, units))
    }

    /// Selects the motor under `at` for the remote control. Returns whether there was one.
    pub fn select_motor_at(&mut self, at: Vec2) -> bool {
        match self.gear_at(at) {
            Some(i) if self.mech.gears[i].motor.is_some() => {
                self.selected_motor = Some(self.mech.gears[i].uid);
                self.car_remote = false;
                self.remote_menu.show(2.0);
                true
            }
            _ => false,
        }
    }

    pub fn reel_at(&self, p: Vec2) -> Option<usize> {
        self.gear_at(p).filter(|&i| self.mech.gears[i].reel.is_some())
    }

    pub fn pulley_at(&self, p: Vec2) -> Option<usize> {
        self.mech
            .pulleys
            .iter()
            .enumerate()
            .filter(|(_, u)| (u.pos - p).length() <= u.radius + 2.5)
            .min_by(|a, b| (a.1.pos - p).length().total_cmp(&(b.1.pos - p).length()))
            .map(|(i, _)| i)
    }

    /// Where a rope from `a` to `b` touches each pulley it runs over: on the far side from the
    /// bend (a rope hanging from two points runs over the top of a pulley).
    pub fn via_contacts(&self, a: Vec2, via: &[u32], b: Vec2) -> Vec<Vec2> {
        let pulleys: Vec<&Pulley> = via.iter().filter_map(|u| self.mech.pulleys.iter().find(|p| p.uid == *u)).collect();
        let mut out = vec![];
        for (i, u) in pulleys.iter().enumerate() {
            let prev = if i == 0 { a } else { pulleys[i - 1].pos };
            let next = if i + 1 == pulleys.len() { b } else { pulleys[i + 1].pos };
            let bis = (prev - u.pos).normalize_or_zero() + (next - u.pos).normalize_or_zero();
            let off = if bis.length() < 0.25 { vec2(0.0, -1.0) } else { -bis.normalize() };
            out.push(u.pos + off * (u.radius + 1.5));
        }
        out
    }

    fn place_pulley(&mut self, at: Vec2) {
        let c = self.snap_point(at).filter(|_| matches!(node_at(&self.world, &self.blocks, &self.members, at), Some(Anchor::Block { .. }))).unwrap_or(at);
        if self.mech.pulleys.iter().any(|p| (p.pos - c).length() < p.radius + PULLEY_R) {
            self.say("Les poulies se chevauchent");
            return;
        }
        if !self.mech_pay(4, c) {
            return;
        }
        self.checkpoint();
        let mount = self.support_for(c);
        self.mech.pulleys.push(Pulley { uid: mech::next_uid(), mount, pos: c, radius: PULLEY_R });
        self.rebuild_physics_if_running();
        self.say("Poulie posée : clique-la avec l'outil câble, entre deux bouts, pour y passer le câble");
    }

    pub fn gear_at(&self, p: Vec2) -> Option<usize> {
        self.mech
            .gears
            .iter()
            .enumerate()
            .filter(|(_, g)| (g.pos - p).length() <= g.size.radius() + 1.5)
            .min_by(|a, b| (a.1.pos - p).length().total_cmp(&(b.1.pos - p).length()))
            .map(|(i, _)| i)
    }

    pub fn rail_at_pub(&self, p: Vec2) -> Option<(usize, f32)> {
        self.rail_at(p)
    }

    /// The rail under `p` and where along it (cells from its start).
    fn rail_at(&self, p: Vec2) -> Option<(usize, f32)> {
        self.mech
            .sliders
            .iter()
            .enumerate()
            .filter_map(|(i, sl)| {
                let (d, t) = point_to_segment(p, sl.origin, sl.origin + sl.dir * sl.len);
                (d < 9.0).then_some((i, d, t * sl.len))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _, s)| (i, s))
    }

    /// Where a gear of `size` would sit for a cursor at `at`. It helps the builder: it clips onto
    /// the teeth of a neighbour at the exact centre distance (along a straight or diagonal line
    /// when close to one), else onto the centre of a block, else lines up with another gear.
    pub fn gear_spot(&self, size: GearSize, at: Vec2) -> Vec2 {
        let r = size.radius();
        let mut best: Option<(f32, Vec2)> = None;
        for g in &self.mech.gears {
            if g.stripped {
                continue;
            }
            let want = g.size.radius() + r;
            let d = (at - g.pos).length();
            if (d - want).abs() < 7.0 && best.map_or(true, |b| (d - want).abs() < b.0) {
                let dir = if d > 1e-3 { (at - g.pos) / d } else { vec2(1.0, 0.0) };
                best = Some(((d - want).abs(), g.pos + snap_to_axes(dir, 7.0) * want));
            }
        }
        if let Some((_, c)) = best {
            return c;
        }
        // On the centre of a block (a hub on a brick).
        if let Some(b) = self.blocks.iter().find(|b| (b.pos - at).length() < 5.0) {
            return b.pos;
        }
        // Lined up with the centre of another gear.
        let mut c = at;
        for g in &self.mech.gears {
            if (at.y - g.pos.y).abs() < 3.0 {
                c.y = g.pos.y;
            }
            if (at.x - g.pos.x).abs() < 3.0 {
                c.x = g.pos.x;
            }
        }
        c
    }

    fn gear_spot_ok(&self, size: GearSize, c: Vec2) -> bool {
        let r = size.radius();
        self.mech.gears.iter().all(|g| (g.pos - c).length() >= g.size.radius() + r - 1.5)
    }

    pub fn support_for_pub(&self, c: Vec2) -> Anchor {
        self.support_for(c)
    }

    fn support_for(&self, c: Vec2) -> Anchor {
        match node_at(&self.world, &self.blocks, &self.members, c) {
            Some(Anchor::Ground(_)) => Anchor::Ground(c),
            Some(a @ (Anchor::Block { .. } | Anchor::End { .. } | Anchor::Along { .. })) => a,
            _ => Anchor::Free(c),
        }
    }

    fn place_gear(&mut self, size: GearSize, motor: bool, reel: bool, at: Vec2) {
        let c = self.gear_spot(size, at);
        if !self.gear_spot_ok(size, c) {
            self.say("Les engrenages se chevauchent : écarte-les ou fais-les s'engrener");
            return;
        }
        let units = gear_cost(size) + if motor { 12 } else { 0 } + if reel { 10 } else { 0 };
        if !self.mech_pay(units, c) {
            return;
        }
        self.checkpoint();
        let mut g = Gear::new(mech::next_uid(), size, c);
        g.mount = self.support_for(c);
        if reel {
            g.reel = Some(WINCH_DRUM);
        }
        if motor {
            let mut m = Motor::new();
            m.dir = self.motor_dir;
            m.auto_reverse = self.motor_reverse;
            g.motor = Some(m);
        }
        self.mech.gears.push(g);
    }

    fn wheel_at(&self, at: Vec2) -> Option<usize> {
        block_at(&self.blocks, at).filter(|&i| self.blocks[i].kind == BlockKind::Wheel)
    }

    /// Shaft from a gear to a wheel: the gear then drives the wheel (a drive wheel, a drum).
    fn link_wheel(&mut self, gear: u32, wheel: u32) {
        let Some(gi) = self.mech.gear_index(gear) else { return };
        let pos = self.mech.gears[gi].pos;
        if !self.mech_pay(8, pos) {
            return;
        }
        self.checkpoint();
        for g in &mut self.mech.gears {
            if g.wheel == Some(wheel) {
                g.wheel = None;
            }
        }
        self.mech.gears[gi].wheel = Some(wheel);
        self.say("Arbre posé : l'engrenage entraîne la roue");
    }

    /// The tige under `at` and the end it would swing about: the end that is fixed to something
    /// when only one is, else the end clicked, else the one nearest `near` (where the gear is).
    fn arm_end_at(&self, at: Vec2, near: Vec2) -> Option<(u32, u8)> {
        let (uid, clicked) = match node_at(&self.world, &self.blocks, &self.members, at)? {
            Anchor::End { uid, end } => (uid, Some(end)),
            Anchor::Along { uid, .. } => (uid, None),
            _ => return None,
        };
        let m = self.members.iter().find(|m| m.uid == uid)?;
        let fixed = |a: &Anchor| !matches!(a, Anchor::Free(_));
        let end = match (fixed(&m.a), fixed(&m.b), clicked) {
            (true, false, _) => 0,
            (false, true, _) => 1,
            (_, _, Some(e)) => e,
            _ => u8::from((m.p1 - near).length() < (m.p0 - near).length()),
        };
        Some((uid, end))
    }

    /// Shaft from a gear to a tige: the gear swings the tige about one of its ends (an arm). A loose
    /// end gets an axle where it is.
    fn link_arm(&mut self, gear: u32, (member, end): (u32, u8)) {
        let Some(gi) = self.mech.gear_index(gear) else { return };
        let Some(mi) = self.members.iter().position(|m| m.uid == member) else { return };
        let pos = self.mech.gears[gi].pos;
        if !self.mech_pay(8, pos) {
            return;
        }
        self.checkpoint();
        // One gear per arm: two would fight over it.
        for g in &mut self.mech.gears {
            if g.arm.map_or(false, |(u, _)| u == member) {
                g.arm = None;
            }
        }
        let m = &mut self.members[mi];
        let (anchor, p) = if end == 0 { (&mut m.a, m.p0) } else { (&mut m.b, m.p1) };
        if matches!(anchor, Anchor::Free(_)) {
            *anchor = Anchor::Ground(p);
        }
        let g = &mut self.mech.gears[gi];
        g.arm = Some((member, end));
        g.arm_base = None;
        self.rebuild_physics_if_running();
        self.say("Arbre posé : l'engrenage fait pivoter la tige autour de son bout, comme un bras");
    }

    fn place_axle(&mut self, at: Vec2) {
        let gear_hit = self.gear_at(at);
        let wheel_hit = self.wheel_at(at);
        let valve_hit = self.valve_at(at);
        let pump_hit = self.pump_at(at);
        let rail_hit = if gear_hit.is_none() && wheel_hit.is_none() && valve_hit.is_none() { self.rail_at(at).map(|(i, _)| i) } else { None };
        match self.mech_draft.take() {
            Some(MechDraft::AxlePump(pm)) => {
                if let Some(i) = gear_hit {
                    let uid = self.mech.gears[i].uid;
                    self.link_pump(pm, uid);
                } else {
                    self.mech_draft = Some(MechDraft::AxlePump(pm));
                    self.say("Clique l'engrenage qui doit mener cette pompe");
                }
            }
            Some(MechDraft::Axle(first)) if pump_hit.is_some() => {
                let pm = self.mech.plumbing.pumps[pump_hit.unwrap()].uid;
                self.link_pump(pm, first);
            }
            // A valve and the gear or rail that turns it.
            Some(MechDraft::AxleValve(v)) => {
                if let Some(i) = gear_hit {
                    let uid = self.mech.gears[i].uid;
                    self.link_valve(v, crate::pipes::Driver::Gear(uid));
                } else if let Some(i) = rail_hit {
                    let uid = self.mech.sliders[i].uid;
                    self.link_valve(v, crate::pipes::Driver::Slider(uid));
                } else {
                    self.mech_draft = Some(MechDraft::AxleValve(v));
                    self.say("Clique l'engrenage ou le rail qui doit tourner cette vanne");
                }
            }
            Some(MechDraft::AxleSlider(s)) => {
                if let Some(i) = valve_hit {
                    let v = self.mech.plumbing.valves[i].uid;
                    self.link_valve(v, crate::pipes::Driver::Slider(s));
                } else {
                    self.mech_draft = Some(MechDraft::AxleSlider(s));
                    self.say("Clique la vanne que ce rail doit ouvrir");
                }
            }
            Some(MechDraft::Axle(first)) if valve_hit.is_some() => {
                let v = self.mech.plumbing.valves[valve_hit.unwrap()].uid;
                self.link_valve(v, crate::pipes::Driver::Gear(first));
            }
            Some(MechDraft::Axle(first)) => {
                let from = self.mech.gear_index(first).map_or(at, |i| self.mech.gears[i].pos);
                // A gear sitting on the end of a tige: clicking it again picks the tige.
                let on_itself = gear_hit.map_or(false, |i| self.mech.gears[i].uid == first);
                let arm_hit = if gear_hit.is_none() || on_itself { self.arm_end_at(at, from) } else { None };
                if let (Some(arm), None) = (arm_hit, wheel_hit) {
                    self.link_arm(first, arm);
                } else if let Some(i) = gear_hit {
                    let uid = self.mech.gears[i].uid;
                    if first == uid {
                        return;
                    }
                    if self.mech.axles.iter().any(|a| (a.a == first && a.b == uid) || (a.a == uid && a.b == first)) {
                        self.say("Ces deux engrenages ont déjà un axe");
                        return;
                    }
                    let (pa, pb) = (self.mech.gears[self.mech.gear_index(first).unwrap()].pos, self.mech.gears[i].pos);
                    let d = (pa - pb).length();
                    if d > AXLE_MAX {
                        self.say("Axe trop long");
                        self.mech_draft = Some(MechDraft::Axle(first));
                        return;
                    }
                    if !self.mech_pay(((d * 0.12).ceil() as u32).max(4), pb) {
                        self.mech_draft = Some(MechDraft::Axle(first));
                        return;
                    }
                    self.checkpoint();
                    self.mech.axles.push(Axle { uid: mech::next_uid(), a: first, b: uid });
                } else if let Some(w) = wheel_hit {
                    let wuid = self.blocks[w].uid;
                    self.link_wheel(first, wuid);
                } else {
                    self.mech_draft = Some(MechDraft::Axle(first));
                    self.say("Clique un engrenage, une roue ou une tige pour l'autre bout");
                }
            }
            Some(MechDraft::AxleWheel(w)) => {
                if let Some(i) = gear_hit {
                    let guid = self.mech.gears[i].uid;
                    self.link_wheel(guid, w);
                } else {
                    self.mech_draft = Some(MechDraft::AxleWheel(w));
                    self.say("Clique l'engrenage qui entraîne cette roue");
                }
            }
            Some(MechDraft::AxleArm(m, end)) => {
                if let Some(i) = gear_hit {
                    let guid = self.mech.gears[i].uid;
                    self.link_arm(guid, (m, end));
                } else {
                    self.mech_draft = Some(MechDraft::AxleArm(m, end));
                    self.say("Clique l'engrenage qui fait pivoter cette tige");
                }
            }
            _ => {
                if let Some(i) = gear_hit {
                    self.mech_draft = Some(MechDraft::Axle(self.mech.gears[i].uid));
                } else if let Some(w) = wheel_hit {
                    self.mech_draft = Some(MechDraft::AxleWheel(self.blocks[w].uid));
                } else if let Some(i) = pump_hit {
                    self.mech_draft = Some(MechDraft::AxlePump(self.mech.plumbing.pumps[i].uid));
                } else if let Some(i) = valve_hit {
                    self.mech_draft = Some(MechDraft::AxleValve(self.mech.plumbing.valves[i].uid));
                } else if let Some(i) = rail_hit {
                    self.mech_draft = Some(MechDraft::AxleSlider(self.mech.sliders[i].uid));
                } else if let Some((m, end)) = self.arm_end_at(at, at) {
                    self.mech_draft = Some(MechDraft::AxleArm(m, end));
                    self.say("Clique l'engrenage qui fait pivoter cette tige");
                } else {
                    self.say("Clique sur un engrenage (ou une roue, une tige) pour fixer un bout de l'axe");
                }
            }
        }
    }

    /// Fixes a wheel on an axle (a pulley, a drum, a drive wheel), or takes the axle off again.
    fn place_pivot(&mut self, at: Vec2) {
        let Some(wi) = self.wheel_at(at) else {
            self.pivot_tige_end(at);
            return;
        };
        let (uid, centre) = (self.blocks[wi].uid, self.blocks[wi].pos);
        if let Some(i) = self.mech.pivots.iter().position(|p| p.wheel == uid) {
            self.checkpoint();
            self.mech.pivots.remove(i);
            self.rebuild_physics_if_running();
            self.say("Axe retiré : la roue est libre");
            return;
        }
        if !self.mech_pay(4, centre) {
            return;
        }
        self.checkpoint();
        // What holds the axle: a block under the wheel's centre, else the ground, else the air.
        let others: Vec<Block> = self.blocks.iter().filter(|b| b.uid != uid).cloned().collect();
        let (support, name) = match block_at(&others, centre) {
            Some(i) => (Anchor::Block { uid: others[i].uid, local: rot_vec(-others[i].angle, centre - others[i].pos) }, "un bloc"),
            None => match anchor_at(&self.world, &[], centre) {
                Some(Anchor::Ground(_)) => (Anchor::Ground(centre), "le sol"),
                _ => (Anchor::Free(centre), "l'air"),
            },
        };
        self.mech.pivots.push(Pivot { wheel: uid, support });
        self.rebuild_physics_if_running();
        self.say(format!("Roue fixée sur un axe, tenu par {name}"));
    }

    /// A pivot on the end of a tige: a loose end gets an axle where it is (the tige then turns
    /// about it); clicking an axle in the air again takes it off.
    fn pivot_tige_end(&mut self, at: Vec2) {
        let Some(Anchor::End { uid, end }) = node_at(&self.world, &self.blocks, &self.members, at) else {
            self.say("Clique sur une roue ou sur un bout de tige pour y mettre un pivot");
            return;
        };
        let Some(mi) = self.members.iter().position(|m| m.uid == uid) else { return };
        let m = &self.members[mi];
        let (anchor, p) = if end == 0 { (m.a, m.p0) } else { (m.b, m.p1) };
        match anchor {
            Anchor::Free(_) => {
                if !self.mech_pay(4, p) {
                    return;
                }
                self.checkpoint();
                let m = &mut self.members[mi];
                *(if end == 0 { &mut m.a } else { &mut m.b }) = Anchor::Ground(p);
                self.rebuild_physics_if_running();
                self.say("Pivot posé au bout de la tige : elle tourne autour. Avec l'outil Axe, un engrenage peut la faire pivoter");
            }
            Anchor::Ground(q) if !self.world.raw(q.x as i32, q.y as i32).is_hard() => {
                self.checkpoint();
                let m = &mut self.members[mi];
                *(if end == 0 { &mut m.a } else { &mut m.b }) = Anchor::Free(p);
                for g in &mut self.mech.gears {
                    if g.arm == Some((uid, end)) {
                        g.arm = None;
                    }
                }
                self.rebuild_physics_if_running();
                self.say("Pivot retiré : ce bout de tige est libre");
            }
            _ => self.say("Ce bout de tige pivote déjà sur ce qui le tient. Avec l'outil Axe, un engrenage peut la faire pivoter"),
        }
    }

    /// A lead screw: a gear and a rail, in either order. The rail's carriage becomes the nut.
    fn place_screw(&mut self, at: Vec2) {
        let gear_hit = self.gear_at(at).map(|i| self.mech.gears[i].uid);
        let rail_hit = if gear_hit.is_none() { self.rail_at(at).map(|(i, _)| self.mech.sliders[i].uid) } else { None };
        match (self.mech_draft.take(), gear_hit, rail_hit) {
            (Some(MechDraft::ScrewGear(g)), _, Some(r)) | (Some(MechDraft::ScrewRail(r)), Some(g), _) => self.link_screw(g, r),
            (Some(d @ MechDraft::ScrewGear(_)), _, _) => {
                self.mech_draft = Some(d);
                self.say("Clique le rail que la vis doit faire avancer");
            }
            (Some(d @ MechDraft::ScrewRail(_)), _, _) => {
                self.mech_draft = Some(d);
                self.say("Clique l'engrenage qui doit tourner la vis");
            }
            (_, Some(g), _) => {
                self.mech_draft = Some(MechDraft::ScrewGear(g));
                self.say("Clique maintenant un rail : il devient une vis sans fin");
            }
            (_, _, Some(r)) => {
                self.mech_draft = Some(MechDraft::ScrewRail(r));
                self.say("Clique maintenant l'engrenage qui tourne la vis");
            }
            _ => self.say("Clique un engrenage puis un rail (ou l'inverse) : le rail devient une vis sans fin"),
        }
    }

    fn link_screw(&mut self, gear: u32, rail: u32) {
        let (Some(gi), Some(si)) = (self.mech.gear_index(gear), self.mech.slider_index(rail)) else { return };
        let sl = &self.mech.sliders[si];
        if sl.screw == Some(gear) {
            self.say("Cet engrenage tourne déjà cette vis");
            return;
        }
        let pos = self.mech.gears[gi].pos;
        if (screw_end(sl, pos) - pos).length() > AXLE_MAX {
            self.say("La vis est trop loin de l'engrenage");
            self.mech_draft = Some(MechDraft::ScrewGear(gear));
            return;
        }
        if !self.mech_pay(screw_cost(sl), pos) {
            return;
        }
        self.checkpoint();
        self.mech.sliders[si].screw = Some(gear);
        self.say(format!(
            "Vis sans fin posée : un tour d'engrenage avance le chariot de {:.0}. Elle pousse fort et tient la charge, même moteur arrêté",
            mech::SCREW_LEAD
        ));
    }

    fn rebuild_physics_if_running(&mut self) {
        if self.running {
            self.phys = None;
            self.build_physics();
        }
    }

    /// Where the second end of a rail from `from` lands for a cursor at `at`: on a support if
    /// there is one, otherwise in the air, straightened to horizontal, vertical or diagonal.
    pub fn rail_end(&self, from: Vec2, at: Vec2) -> (Anchor, Vec2) {
        let anchor = self.support_for(at);
        if !matches!(anchor, Anchor::Free(_)) {
            if let Some(p) = anchor_world(&anchor, &self.blocks, &self.members) {
                return (anchor, p);
            }
        }
        let d = at - from;
        let len = d.length();
        if len < 1.0 {
            return (Anchor::Free(at), at);
        }
        let dir = snap_to_axes(d / len, 6.0);
        let len = (len / 5.0).round().max(1.0) * 5.0;
        let p = from + dir * len;
        (Anchor::Free(p), p)
    }

    /// The attach point of a gear nearest to `p`, or `p` itself inside the disc.
    fn pin_spot(&self, gi: usize, p: Vec2) -> Vec2 {
        let g = &self.mech.gears[gi];
        let local = rot_vec(-g.angle, p - g.pos);
        let best = attach_points(g.size).into_iter().min_by(|a, b| (*a - local).length().total_cmp(&(*b - local).length())).unwrap_or(Vec2::ZERO);
        if (best - local).length() <= 4.5 {
            g.pos + rot_vec(g.angle, best)
        } else {
            p
        }
    }

    fn place_rail(&mut self, at: Vec2) {
        let anchor = self.support_for(at);
        match self.mech_draft.take() {
            Some(MechDraft::Rail(first, from)) => {
                let (anchor, to) = self.rail_end(from, at);
                let len = (to - from).length();
                if len < RAIL_MIN {
                    self.say("Le rail est trop court : éloigne le deuxième clic");
                    self.mech_draft = Some(MechDraft::Rail(first, from));
                    return;
                }
                let (anchor, to) = if len > RAIL_MAX {
                    let to = from + (to - from) * (RAIL_MAX / len);
                    (self.support_for(to), to)
                } else {
                    (anchor, to)
                };
                if !self.mech_pay(((to - from).length() * 0.2).ceil() as u32, to) {
                    self.mech_draft = Some(MechDraft::Rail(first, from));
                    return;
                }
                self.checkpoint();
                let mut sl = Slider::new(mech::next_uid(), from, to);
                sl.ends = (first, anchor);
                self.mech.sliders.push(sl);
            }
            _ => {
                let p = anchor_world(&anchor, &self.blocks, &self.members).unwrap_or(at);
                self.mech_draft = Some(MechDraft::Rail(anchor, p));
            }
        }
    }

    /// Where a rod end clicked at `at` attaches: the pin of a gear (at the nearest attach point), the
    /// carriage of a rail, or any other node (a block, a rod end, the ground, the world border).
    fn resolve_end(&mut self, at: Vec2) -> Option<End> {
        if let Some(i) = self.gear_at(at) {
            let spot = self.pin_spot(i, at);
            let g = &self.mech.gears[i];
            return Some(End::Pin { gear: g.uid, pin: rot_vec(-g.angle, spot - g.pos) });
        }
        if let Some((si, s)) = self.rail_at(at) {
            let uid = self.mech.sliders[si].uid;
            // A carriage that nothing holds yet goes where the rod is clicked; otherwise it stays put.
            if !self.mech.rods.iter().any(|r| r.on_carriage(uid)) {
                let sl = &mut self.mech.sliders[si];
                let (lo, hi) = (sl.margin, (sl.len - sl.margin).max(sl.margin));
                sl.s = s.clamp(lo, hi);
            }
            return Some(End::Carriage(uid));
        }
        node_at(&self.world, &self.blocks, &self.members, at).map(End::Node)
    }

    pub fn rod_end_world(&self, e: &End) -> Option<Vec2> {
        match e {
            End::Node(a) => anchor_world(a, &self.blocks, &self.members),
            other => self.mech.end_point(other),
        }
    }

    fn place_crank(&mut self, at: Vec2) {
        match self.mech_draft.take() {
            Some(MechDraft::Crank { first }) => {
                let Some(second) = self.resolve_end(at) else {
                    self.say("Clique un engrenage, un rail, un bloc, une tige ou le sol pour l'autre bout");
                    self.mech_draft = Some(MechDraft::Crank { first });
                    return;
                };
                let same = match (first, second) {
                    (End::Pin { gear: a, .. }, End::Pin { gear: b, .. }) => a == b,
                    (End::Carriage(a), End::Carriage(b)) => a == b,
                    (End::Node(_), End::Node(_)) => true,
                    _ => false,
                };
                if same {
                    self.say("Une bielle relie deux choses différentes (un bloc fixe n'en relie pas un autre)");
                    self.mech_draft = Some(MechDraft::Crank { first });
                    return;
                }
                let (Some(pa), Some(pb)) = (self.rod_end_world(&first), self.rod_end_world(&second)) else { return };
                let len = (pa - pb).length();
                if len < 6.0 {
                    self.say("Bielle trop courte : éloigne les deux bouts");
                    self.mech_draft = Some(MechDraft::Crank { first });
                    return;
                }
                if !self.mech_pay(6 + (len * 0.1).ceil() as u32, pb) {
                    self.mech_draft = Some(MechDraft::Crank { first });
                    return;
                }
                self.checkpoint();
                self.mech.rods.push(Rod::between(mech::next_uid(), first, second, len));
                self.rebuild_physics_if_running();
            }
            _ => match self.resolve_end(at) {
                Some(first) => self.mech_draft = Some(MechDraft::Crank { first }),
                None => self.say("Clique d'abord un engrenage, un rail, un bloc ou le sol pour fixer un bout"),
            },
        }
    }

    pub fn mech_click(&mut self, at: Vec2) {
        match self.tool {
            Tool::Gear(s) => self.place_gear(s, false, false, at),
            Tool::Motor => {
                if !self.select_motor_at(at) {
                    self.place_gear(GearSize::Small, true, false, at);
                    // A motor just placed is the one the remote shows.
                    if let Some(g) = self.mech.gears.last().filter(|g| g.motor.is_some()) {
                        self.selected_motor = Some(g.uid);
                        self.remote_menu.show(2.0);
                    }
                }
            }
            Tool::Axle => self.place_axle(at),
            Tool::Rail => self.place_rail(at),
            Tool::Crank => self.place_crank(at),
            Tool::Pivot => self.place_pivot(at),
            Tool::Screw => self.place_screw(at),
            Tool::Winch => self.place_gear(GearSize::Medium, false, true, at),
            Tool::Pulley => self.place_pulley(at),
            _ => {}
        }
    }

    /// Removes the mechanism piece under `at`, with what hangs on it. Returns whether there was one.
    pub fn mech_erase(&mut self, at: Vec2) -> bool {
        if let Some(i) = self.mech.rods.iter().position(|r| {
            let (Some(a), Some(b)) = (self.rod_end_world(&r.a), self.rod_end_world(&r.b)) else { return false };
            point_to_segment(at, a, b).0 < 3.0
        }) {
            self.checkpoint();
            self.mech.rods.remove(i);
            return true;
        }
        if let Some(i) = self.pulley_at(at) {
            self.checkpoint();
            let uid = self.mech.pulleys.remove(i).uid;
            for c in &mut self.cables {
                c.via.retain(|u| *u != uid);
            }
            self.rebuild_physics_if_running();
            return true;
        }
        // The shaft from a gear to its wheel.
        let shaft = self.mech.gears.iter().position(|g| {
            g.wheel.and_then(|w| self.blocks.iter().find(|b| b.uid == w)).map_or(false, |b| point_to_segment(at, g.pos, b.pos).0 < 3.0 && (at - g.pos).length() > g.size.radius() + 1.5 && (at - b.pos).length() > 13.0)
        });
        if let Some(i) = shaft {
            self.checkpoint();
            self.mech.gears[i].wheel = None;
            return true;
        }
        // The shaft from a gear to the tige it swings.
        let arm = self.mech.gears.iter().position(|g| {
            self.arm_pivot(g).map_or(false, |p| point_to_segment(at, g.pos, p).0 < 3.0 && (at - g.pos).length() > g.size.radius() + 1.5 && (at - p).length() > 4.0)
        });
        if let Some(i) = arm {
            self.checkpoint();
            self.mech.gears[i].arm = None;
            return true;
        }
        // The shaft from a gear to the screw it turns: the rail is a plain rail again.
        let screw = self.mech.sliders.iter().position(|sl| {
            let Some(g) = sl.screw.and_then(|u| self.mech.gear_index(u)).map(|i| &self.mech.gears[i]) else { return false };
            let e = screw_end(sl, g.pos);
            point_to_segment(at, g.pos, e).0 < 3.0 && (at - g.pos).length() > g.size.radius() + 1.5 && (at - e).length() > 4.0
        });
        if let Some(i) = screw {
            self.checkpoint();
            self.mech.sliders[i].screw = None;
            return true;
        }
        if let Some(i) = self.gear_at(at) {
            self.checkpoint();
            let reel = self.mech.gears.remove(i).reel.is_some();
            self.mech.prune();
            if reel {
                self.refresh_reel_anchors();
                self.rebuild_physics_if_running();
            }
            return true;
        }
        if let Some((i, _)) = self.rail_at(at) {
            self.checkpoint();
            self.mech.sliders.remove(i);
            self.mech.prune();
            return true;
        }
        if let Some(i) = self.mech.axles.iter().position(|a| {
            let (Some(x), Some(y)) = (self.mech.gear_index(a.a), self.mech.gear_index(a.b)) else { return false };
            point_to_segment(at, self.mech.gears[x].pos, self.mech.gears[y].pos).0 < 3.0
        }) {
            self.checkpoint();
            self.mech.axles.remove(i);
            return true;
        }
        false
    }

    // ----- simulation -------------------------------------------------------

    /// One step of the mechanisms: follow the supports, feel the world around each carriage,
    /// turn, then push whatever is in the way.
    pub fn mech_step(&mut self) {
        if self.mech.gears.is_empty() && self.mech.sliders.is_empty() {
            return;
        }
        let (blocks, members) = (&self.blocks, &self.members);
        self.mech.follow_mounts(|a| anchor_world(a, blocks, members));
        self.mech_prune_wheels();
        self.mech_feel_world();
        self.mech_drive_wheels();
        let node_loads = self.mech_node_rods();
        self.mech_reels();
        self.pump_loads();
        self.mech_steam();
        let mut events = self.mech.step();
        for (r, f) in node_loads {
            if let Some(e) = self.mech.load_rod(r, f) {
                if let (Event::RodBroke(uid), Some(p)) = (e, self.phys.as_mut()) {
                    p.remove_pin_rod(uid);
                }
                events.push(e);
            }
        }
        for e in events {
            match e {
                Event::RodBroke(uid) => {
                    let spot = self.mech.rods.iter().find(|r| r.uid == uid).and_then(|r| self.rod_end_world(&r.b));
                    if let Some(p) = spot {
                        self.booms.push(Boom { pos: p, t: 0.4, color: Color::new(0.9, 0.7, 0.4, 1.0) });
                    }
                    self.sounds.emit(Ev::Snap);
                    self.say("Une bielle a cassé : trop d'effort");
                }
                Event::TeethStripped(uid) => {
                    if let Some(g) = self.mech.gears.iter().find(|g| g.uid == uid) {
                        self.booms.push(Boom { pos: g.pos, t: 0.4, color: Color::new(0.8, 0.8, 0.9, 1.0) });
                    }
                    self.sounds.emit(Ev::Crack);
                    self.say("Les dents d'un engrenage ont lâché : trop de couple");
                }
                Event::Jammed(_) => self.say("Engrenages coincés : un nombre impair d'engrenages en cercle ne tourne pas"),
                Event::Stalled(uid) => {
                    if self.toast.is_none() {
                        let rail = self.mech.sliders.iter().any(|sl| sl.uid == uid);
                        self.say(if rail { "Un coulisseau est en butée : le moteur force" } else { "Le mécanisme est en butée (vanne, treuil...) : le moteur force" });
                    }
                }
            }
        }
        self.mech_push_world();
    }

    /// Steam motors: each takes its power from the closed pocket of steam it sits in or touches (a
    /// boiler), and uses up that steam as it turns, letting it out of its chimney. No boiler, or a
    /// boiler gone cold: no pressure, no torque.
    fn mech_steam(&mut self) {
        for gi in 0..self.mech.gears.len() {
            let g = &self.mech.gears[gi];
            let Some(mut mo) = g.motor.filter(|m| m.steam) else { continue };
            let (pos, r, omega) = (g.pos, g.size.radius(), g.omega);
            let intake = self.world.steam_intake((pos.x, pos.y), r + 4.0);
            mo.pressure = intake.map_or(1.0, |(_, p)| p);
            mo.power = ((mo.pressure - 1.0) / STEAM_FULL_ATM).clamp(0.0, 1.5);
            match intake {
                Some((k, _)) if mo.on => {
                    mo.draw += mo.power * omega.abs() * STEAM_USE;
                    let exhaust = (pos.x.round() as i32, (pos.y - r - 5.0).round() as i32);
                    while mo.draw >= 1.0 {
                        mo.draw -= 1.0;
                        if !self.world.draw_steam(k, (pos.x, pos.y), exhaust) {
                            mo.draw = 0.0;
                            break;
                        }
                    }
                }
                _ => mo.draw = 0.0,
            }
            self.mech.gears[gi].motor = Some(mo);
        }
    }

    /// Rods from a gear pin or a carriage to a block, a rod or the ground: the physics holds the far end
    /// (a stiff spring), the mechanism feels the pull as a torque or a force. Returns each rod's load.
    fn mech_node_rods(&mut self) -> Vec<(usize, f32)> {
        let mut loads = vec![];
        for ri in 0..self.mech.rods.len() {
            let rod = self.mech.rods[ri].clone();
            if rod.broken {
                continue;
            }
            let (mech_end, node) = match (rod.a, rod.b) {
                (e, End::Node(n)) if !matches!(e, End::Node(_)) => (e, n),
                (End::Node(n), e) if !matches!(e, End::Node(_)) => (e, n),
                _ => continue,
            };
            let (Some(p), Some(q)) = (self.mech.end_point(&mech_end), anchor_world(&node, &self.blocks, &self.members)) else { continue };
            let Some(phys) = self.phys.as_mut() else { continue };
            phys.move_pin(rod.uid, p);
            let pull = blocks::Physics::rod_pull(p, q, rod.len);
            match mech_end {
                End::Pin { gear, .. } => {
                    if let Some(gi) = self.mech.gear_index(gear) {
                        let r = (p - self.mech.gears[gi].pos) / blocks::PX_PER_M;
                        self.mech.gears[gi].ext_torque += (r.x * pull.y - r.y * pull.x) / WHEEL_TORQUE_PER_UNIT;
                    }
                }
                End::Carriage(u) => {
                    if let Some(si) = self.mech.slider_index(u) {
                        let along = pull.dot(self.mech.sliders[si].dir);
                        self.mech.sliders[si].ext_force += along / WHEEL_TORQUE_PER_UNIT;
                    }
                }
                End::Node(_) => {}
            }
            loads.push((ri, pull.length() / WHEEL_TORQUE_PER_UNIT));
        }
        loads
    }

    /// Keeps the positions stored in rope ends on winch drums up to date; a rope whose winch is gone
    /// is left hanging loose.
    pub fn refresh_reel_anchors(&mut self) {
        let gears = &self.mech.gears;
        for c in &mut self.cables {
            for a in [&mut c.a, &mut c.b] {
                if let Anchor::Reel { uid, pos } = a {
                    match gears.iter().find(|g| g.uid == *uid && g.reel.is_some()) {
                        Some(g) => *pos = g.pos,
                        None => *a = Anchor::Free(*pos),
                    }
                }
            }
        }
        // Pulleys that no longer exist drop out of the ropes that ran over them.
        let live: Vec<u32> = self.mech.pulleys.iter().map(|p| p.uid).collect();
        for c in &mut self.cables {
            c.via.retain(|u| live.contains(u));
        }
    }

    /// Before a rope is built: where it touches its pulleys, and how its length follows its winch.
    pub fn mech_prepare_cable(&mut self, i: usize) {
        self.refresh_reel_anchors();
        let (a, b) = (self.cables[i].a, self.cables[i].b);
        if let (Some(pa), Some(pb)) = (anchor_world(&a, &self.blocks, &self.members), anchor_world(&b, &self.blocks, &self.members)) {
            let via = self.cables[i].via.clone();
            self.cables[i].via_pts = self.via_contacts(pa, &via, pb);
        }
        self.cables[i].reel = None;
        for end in [a, b] {
            if let Anchor::Reel { uid, .. } = end {
                if let Some(gi) = self.mech.gear_index(uid) {
                    let r = self.mech.gears[gi].reel.unwrap_or(WINCH_DRUM);
                    let len = self.cables[i].len;
                    let th0 = self.mech.gears[gi].angle;
                    let (min_len, max_len) = (REEL_MIN.min(len), len + REEL_SPARE);
                    self.cables[i].reel = Some(ReelRef { gear: uid, base_len: len, base_angle: th0, radius: r, min_len, max_len });
                    // The drum can turn only as far as there is rope: an end stop with unlimited torque.
                    self.mech.gears[gi].limit = Some((th0 + (len - max_len) / r, th0 + (len - min_len) / r));
                }
            }
        }
    }

    /// Winches: the rope pays out or reels in as the drum turns, and pulls back on the drum.
    fn mech_reels(&mut self) {
        let Some(phys) = self.phys.as_mut() else { return };
        for g in &self.mech.gears {
            if g.reel.is_some() {
                phys.move_reel(g.uid, g.pos);
            }
        }
        for c in &mut self.cables {
            let Some(r) = c.reel else { continue };
            let Some(gi) = self.mech.gear_index(r.gear) else { continue };
            let len = (r.base_len - r.radius * (self.mech.gears[gi].angle - r.base_angle)).clamp(r.min_len, r.max_len);
            phys.set_cable_length(c, len);
            c.len = len;
            let at_start = matches!(c.a, Anchor::Reel { .. });
            let pull = Physics::rope_tension(c, at_start);
            self.mech.gears[gi].ext_torque -= pull * (r.radius / PX_PER_M) / WHEEL_TORQUE_PER_UNIT;
        }
    }

    /// Winch drums and pulley sheaves go into the physics before any rope is built.
    pub fn mech_physics_setup(&self, p: &mut Physics) {
        for g in &self.mech.gears {
            if g.reel.is_some() {
                p.add_reel(g.uid, g.pos);
            }
        }
        for pu in &self.mech.pulleys {
            p.add_pulley(&self.blocks, &self.members, pu.uid, &pu.mount, pu.pos, pu.radius);
        }
    }

    /// Forgets axles and shafts of wheels that no longer exist.
    pub fn mech_prune_wheels(&mut self) {
        let live: Vec<u32> = self.blocks.iter().filter(|b| b.kind == BlockKind::Wheel).map(|b| b.uid).collect();
        self.mech.pivots.retain(|p| live.contains(&p.wheel));
        for g in &mut self.mech.gears {
            if g.wheel.map_or(false, |w| !live.contains(&w)) {
                g.wheel = None;
                g.ext_torque = 0.0;
            }
        }
        let tiges: Vec<u32> = self.members.iter().map(|m| m.uid).collect();
        for g in &mut self.mech.gears {
            if g.arm.map_or(false, |(m, _)| !tiges.contains(&m)) {
                g.arm = None;
                g.arm_base = None;
            }
        }
    }

    /// Where the tige a gear swings turns: its pivot end.
    fn arm_pivot(&self, g: &Gear) -> Option<Vec2> {
        let (uid, end) = g.arm?;
        let m = self.members.iter().find(|m| m.uid == uid)?;
        Some(if end == 0 { m.p0 } else { m.p1 })
    }

    /// The angle of what an arm's pivot is fixed to: a block, another tige, or the ground (which
    /// does not turn).
    fn arm_support_angle(&self, support: &Anchor) -> f32 {
        match support {
            Anchor::Block { uid, .. } => self.blocks.iter().find(|b| b.uid == *uid).map_or(0.0, |b| b.angle),
            Anchor::End { uid, .. } | Anchor::Along { uid, .. } => self.members.iter().find(|m| m.uid == *uid).map_or(0.0, |m| m.angle()),
            _ => 0.0,
        }
    }

    /// A gear swinging a tige: the axle at the tige's pivot is driven so that the tige follows the
    /// gear's angle (against its support), with at most the torque the motor gets to that shaft. The
    /// tige pushes back on the gear with the effort it takes. Returns that push.
    fn mech_drive_arm(&mut self, gi: usize) -> f32 {
        let g = &self.mech.gears[gi];
        let Some((uid, end)) = g.arm else { return 0.0 };
        let Some(m) = self.members.iter().find(|m| m.uid == uid) else { return 0.0 };
        let support = if end == 0 { m.a } else { m.b };
        let rel = m.angle() - self.arm_support_angle(&support);
        let g = &mut self.mech.gears[gi];
        let (g0, a0) = *g.arm_base.get_or_insert((g.angle, rel));
        let lag = wrap_angle(a0 + (g.angle - g0) - rel);
        let speed = (g.omega + lag * ARM_CATCH_UP) * 60.0;
        let reach = (g.shaft_torque * WHEEL_TORQUE_PER_UNIT).max(WHEEL_HOLD);
        let Some(p) = self.phys.as_mut() else { return 0.0 };
        p.drive_member_end(m, end, speed, reach).map_or(0.0, |t| -t / WHEEL_TORQUE_PER_UNIT)
    }

    /// A gear on a shaft twists its wheel towards the gear's speed, with at most the torque the
    /// motor gets to that shaft; the wheel pushes back on the gear with the same effort.
    fn mech_drive_wheels(&mut self) {
        for gi in 0..self.mech.gears.len() {
            let arm = self.mech_drive_arm(gi);
            let Some(uid) = self.mech.gears[gi].wheel else {
                self.mech.gears[gi].ext_torque = arm;
                continue;
            };
            let (Some(p), Some(b)) = (self.phys.as_mut(), self.blocks.iter().find(|b| b.uid == uid)) else {
                self.mech.gears[gi].ext_torque = arm;
                continue;
            };
            let g = &mut self.mech.gears[gi];
            let wheel_speed = p.spin_of(b) / 60.0;
            let effort = ((g.omega - wheel_speed) * 20.0).clamp(-1.0, 1.0);
            // The wheel can always be held by the gear (a brake), and driven up to what the motor gives.
            let reach = (g.shaft_torque * WHEEL_TORQUE_PER_UNIT).max(WHEEL_HOLD);
            p.twist(b, effort * reach);
            g.ext_torque = arm - effort * reach / WHEEL_TORQUE_PER_UNIT;
        }
    }

    /// Carriage footprint advanced by `ahead` cells along its rail: (cell, is_block_cell) pairs.
    fn carriage_cells(sl: &Slider, ahead: f32) -> Vec<(i32, i32)> {
        let centre = sl.origin + sl.dir * (sl.s + ahead);
        let perp = vec2(-sl.dir.y, sl.dir.x);
        let mut out = vec![];
        let (hx, hy) = (sl.half.x, sl.half.y);
        let mut u = -hx;
        while u <= hx {
            let mut v = -hy;
            while v <= hy {
                let p = centre + sl.dir * u + perp * v;
                out.push((p.x.floor() as i32, p.y.floor() as i32));
                v += 1.0;
            }
            u += 1.0;
        }
        out
    }

    /// Tells each slider what is in its way: solid rock stops it, loose matter and blocks resist.
    fn mech_feel_world(&mut self) {
        for i in 0..self.mech.sliders.len() {
            let sl = self.mech.sliders[i].clone();
            let step = sl.v.abs().max(0.5) + 1.0;
            let (mut lo, mut hi) = (false, false);
            let mut force = 0.0f32;
            for (sign, dir_blocked) in [(1.0f32, &mut hi), (-1.0, &mut lo)] {
                let mut loose = 0u32;
                let mut blocks_hit: Vec<usize> = vec![];
                for (x, y) in Self::carriage_cells(&sl, sign * step) {
                    let m = self.world.raw(x, y);
                    match self.world.block_at_cell(x, y) {
                        Some(id) => {
                            if !blocks_hit.contains(&id) {
                                blocks_hit.push(id);
                            }
                            continue;
                        }
                        None => {}
                    }
                    if m.is_hard() && !matches!(m.kind(), world::Kind::Powder) {
                        *dir_blocked = true;
                    } else if matches!(m.kind(), world::Kind::Powder | world::Kind::Liquid) {
                        loose += 1;
                    }
                }
                // What it pushes against: a little per cell of sand or water, more per block.
                let moving_this_way = sl.v * sign > -0.05;
                if moving_this_way {
                    force -= sign * (loose as f32 * 0.02).min(1.5);
                    force -= sign * blocks_hit.len() as f32 * 0.8;
                }
                if moving_this_way {
                    if let Some(p) = &mut self.phys {
                        for id in blocks_hit {
                            if let Some(b) = self.blocks.get(id) {
                                p.knock(b, sl.point(), sl.dir * sign * (0.25 + sl.v.abs() * 0.3));
                            }
                        }
                    }
                }
            }
            let slider = &mut self.mech.sliders[i];
            slider.blocked_lo = lo;
            slider.blocked_hi = hi;
            slider.ext_force = force;
        }
    }

    /// The carriage shoves sand, water and gas out of the cells it now covers.
    fn mech_push_world(&mut self) {
        let mut moved = 0u32;
        for i in 0..self.mech.sliders.len() {
            let sl = &self.mech.sliders[i];
            if sl.v.abs() < 0.05 {
                continue;
            }
            let dir = if sl.v > 0.0 { sl.dir } else { -sl.dir };
            let d = (dir.x.round() as i32, if dir.y.abs() > 0.4 { dir.y.signum() as i32 } else { 0 });
            let d = if d == (0, 0) { (1, 0) } else { d };
            for (x, y) in Self::carriage_cells(sl, 0.0) {
                if self.world.shove((x, y), d) {
                    moved += 1;
                }
            }
        }
        if moved > 3 {
            self.sounds.emit(Ev::Spray);
        }
    }

    // ----- panel ------------------------------------------------------------

    pub fn panel_mech(&mut self, ui: &mut Ui, top: f32) {
        struct T {
            tool: Tool,
            label: &'static str,
            text: &'static str,
        }
        let tools = [
            T { tool: Tool::Motor, label: "Moteur", text: "Un petit engrenage motorisé, vu de face. Clique pour le poser (sur un bloc, le sol, ou dans le vide). Clique un moteur posé pour l'ouvrir dans la télécommande (vitesse, couple, seuil de retournement, à vapeur). Un moteur à vapeur tourne avec la pression d'une chaudière fermée qu'il touche, et use sa vapeur." },
            T { tool: Tool::Gear(GearSize::Small), label: "Petit", text: "Petit engrenage (rayon 6). Posé près d'un autre, il s'engrène : les dents collent à la bonne distance." },
            T { tool: Tool::Gear(GearSize::Medium), label: "Moyen", text: "Engrenage moyen (rayon 10). Deux engrenages qui s'engrènent tournent en sens inverse, plus vite pour le plus petit." },
            T { tool: Tool::Gear(GearSize::Large), label: "Grand", text: "Grand engrenage (rayon 14) : lourd, il sert de volant d'inertie et passe les points morts." },
            T { tool: Tool::Axle, label: "Axe", text: "Deux clics sur deux engrenages : l'axe les fait tourner ensemble, à la même vitesse, même s'ils sont loin l'un de l'autre. Un engrenage puis une tige : l'engrenage fait pivoter la tige autour de son bout fixé, comme un bras (une tige accrochée au bout d'une autre fait un bras articulé)." },
            T { tool: Tool::Rail, label: "Coulisseau", text: "Deux clics : un rail (fixé au sol, à un bloc, ou dans le vide) avec un chariot qui glisse dessus. Il pousse le sable, l'eau et les blocs." },
            T { tool: Tool::Screw, label: "Vis", text: "Vis sans fin : clique un engrenage puis un rail (ou l'inverse). Le rail devient une vis et son chariot un écrou : un tour d'engrenage l'avance de 8. Lent mais très fort, et irréversible : la charge ne la fait pas tourner, même moteur arrêté (un vérin, un cric, un ascenseur)." },
            T { tool: Tool::Pivot, label: "Pivot", text: "Clique une roue pour la fixer sur un axe (sur le bloc ou le sol dessous, sinon dans l'air) : elle tourne sur place, comme une poulie ou un tambour de treuil. Clique le bout d'une tige libre : il se fixe là, la tige tourne autour. Re-clique pour l'enlever. Avec l'outil Axe, un engrenage peut entraîner la roue (un treuil) ou faire pivoter la tige (un bras articulé)." },
            T { tool: Tool::Winch, label: "Treuil", text: "Un tambour à enrouleur sur un engrenage. Avec l'outil Câble, accroche un bout de câble sur le tambour : en tournant, il enroule ou déroule le câble. Quand il n'y a plus de câble le tambour bute (le moteur cale ou repart dans l'autre sens)." },
            T { tool: Tool::Pulley, label: "Poulie", text: "Une poulie fixée sur un bloc, le sol ou dans le vide, sans roue. Avec l'outil Câble : un bout, puis la poulie, puis l'autre bout. Le câble glisse dessus sans frottement." },
            T { tool: Tool::Crank, label: "Bielle", text: "Un clic sur un engrenage pose le point d'accroche, un clic sur un rail y relie la bielle. Le chariot va et vient quand l'engrenage tourne." },
        ];
        let free = self.free_now();
        for (i, t) in tools.iter().enumerate() {
            let r = Self::tile_rect(i, 3, top, 54.0);
            if ui.hover(r) {
                let cost = self.mech_cost_hint(t.tool);
                self.hover_info = Some((t.label.to_string(), format!("{} {cost}", t.text)));
            }
            let short = !free && matches!(t.tool, Tool::Gear(_) | Tool::Motor | Tool::Winch) && self.stock(IRON_OR_STEEL) < self.mech_cost_for(t.tool);
            let tool = t.tool;
            if ui.tile(r, t.label, self.tool == tool, |s| {
                let c = vec2(s.x + s.w / 2.0, s.y + s.h / 2.0);
                match tool {
                    Tool::Motor => {
                        draw_rectangle(c.x - 15.0, c.y - 11.0, 30.0, 22.0, Color::from_rgba(70, 74, 92, 255));
                        draw_gear(c + vec2(4.0, 0.0), 9.0, 0.3, GearSize::Small, WHITE);
                        draw_circle_lines(c.x + 4.0, c.y, 13.0, 1.5, ACCENT);
                    }
                    Tool::Gear(size) => {
                        let rad = match size {
                            GearSize::Small => 9.0,
                            GearSize::Medium => 12.0,
                            GearSize::Large => 15.0,
                        };
                        draw_gear(c, rad, 0.2, size, WHITE);
                    }
                    Tool::Axle => {
                        draw_line(c.x - 16.0, c.y, c.x + 16.0, c.y, 4.0, Color::from_rgba(170, 172, 186, 255));
                        draw_circle(c.x - 16.0, c.y, 5.0, Color::from_rgba(120, 130, 160, 255));
                        draw_circle(c.x + 16.0, c.y, 5.0, Color::from_rgba(120, 130, 160, 255));
                    }
                    Tool::Rail => {
                        draw_line(c.x - 18.0, c.y, c.x + 18.0, c.y, 5.0, Color::from_rgba(70, 74, 90, 255));
                        draw_rectangle(c.x - 5.0, c.y - 6.0, 10.0, 12.0, Color::from_rgba(214, 160, 70, 255));
                    }
                    Tool::Screw => {
                        draw_screw(c - vec2(18.0, 0.0), c + vec2(18.0, 0.0), 0.0, 6.0, 4.0);
                        draw_rectangle(c.x + 1.0, c.y - 7.0, 9.0, 14.0, Color::from_rgba(214, 160, 70, 255));
                    }
                    Tool::Winch => {
                        draw_circle(c.x, c.y, 13.0, Color::from_rgba(120, 88, 56, 255));
                        for k in 0..3 {
                            draw_circle_lines(c.x, c.y, 6.0 + 2.5 * k as f32, 1.5, Color::from_rgba(176, 140, 88, 255));
                        }
                        draw_circle_lines(c.x, c.y, 13.0, 2.0, Color::from_rgba(214, 218, 230, 255));
                        draw_circle(c.x, c.y, 2.5, Color::from_rgba(40, 44, 58, 255));
                    }
                    Tool::Pulley => {
                        draw_circle(c.x, c.y, 10.0, Color::from_rgba(150, 156, 176, 255));
                        draw_circle(c.x, c.y, 7.5, Color::from_rgba(96, 102, 124, 255));
                        draw_circle(c.x, c.y, 2.6, Color::from_rgba(214, 218, 230, 255));
                        draw_line(c.x - 16.0, c.y - 9.0, c.x - 2.0, c.y - 10.0, 2.0, Color::from_rgba(176, 140, 88, 255));
                    }
                    Tool::Pivot => {
                        draw_circle(c.x, c.y, 11.0, Color::from_rgba(60, 62, 74, 255));
                        draw_circle_lines(c.x, c.y, 11.0, 2.0, Color::from_rgba(150, 154, 170, 255));
                        draw_circle(c.x, c.y, 4.0, Color::from_rgba(214, 218, 230, 255));
                        draw_circle(c.x, c.y, 1.6, Color::from_rgba(40, 44, 58, 255));
                    }
                    _ => {
                        draw_line(c.x - 16.0, c.y - 4.0, c.x + 14.0, c.y + 5.0, 3.0, Color::from_rgba(200, 170, 120, 255));
                        draw_circle(c.x - 16.0, c.y - 4.0, 4.5, WHITE);
                        draw_circle(c.x + 14.0, c.y + 5.0, 4.5, WHITE);
                    }
                }
            }) {
                self.tool = tool;
            }
            if short {
                Self::mark_missing(r);
            }
        }
        let rows = tools.len().div_ceil(3);
        let y = top + rows as f32 * 60.0 + 4.0;
        let w = PANEL_W - 12.0;
        if ui.checkbox(Rect::new(6.0, y, w, 26.0), "Moteur : inverse si surcharge", self.motor_reverse, true) {
            self.motor_reverse = !self.motor_reverse;
        }
        let label = if self.motor_dir > 0.0 { "Prochain moteur : horaire" } else { "Prochain moteur : antihoraire" };
        if ui.button(Rect::new(6.0, y + 32.0, w, 26.0), label, Icon::Rotate, false, true, BLUE) {
            self.motor_dir = -self.motor_dir;
        }
        ui.wrapped(
            "Clic droit annule un tracé. Deux engrenages proches s'engrènent. Trop d'effort : la bielle casse, les dents lâchent ; un moteur cale en butée.",
            10.0,
            y + 84.0,
            PANEL_W - 20.0,
            12,
            DIM,
        );
    }

    fn mech_cost_for(&self, tool: Tool) -> u32 {
        match tool {
            Tool::Gear(s) => gear_cost(s),
            Tool::Motor => gear_cost(GearSize::Small) + 12,
            Tool::Winch => gear_cost(GearSize::Medium) + 10,
            _ => 6,
        }
    }

    fn mech_cost_hint(&self, tool: Tool) -> String {
        let have = self.stock(IRON_OR_STEEL);
        match tool {
            Tool::Gear(_) | Tool::Motor | Tool::Winch => format!("Coût : {} de fer ou d'acier. En stock : {have}.", self.mech_cost_for(tool)),
            Tool::Axle => format!("Coût : 0,12 par cellule de longueur. En stock : {have}."),
            Tool::Rail => format!("Coût : 0,2 par cellule (max {RAIL_MAX:.0}). En stock : {have}."),
            Tool::Screw => format!("Coût : 8 + 0,1 par cellule de rail. En stock : {have}."),
            _ => format!("Coût : 6 + 0,1 par cellule. En stock : {have}."),
        }
    }

    // ----- drawing ----------------------------------------------------------

    pub fn draw_mech(&self, view: View, ui: &Ui, inside: bool) {
        let stress_view = self.mode == ViewMode::Stress;
        // Rails first, then axles, then gears on top, then rods and carriages.
        for sl in &self.mech.sliders {
            let (a, b) = (view.to_screen(sl.origin), view.to_screen(sl.origin + sl.dir * sl.len));
            if let Some(g) = sl.screw.and_then(|u| self.mech.gear_index(u)).map(|i| &self.mech.gears[i]) {
                // A threaded shaft, and the shaft that brings the gear's turn to it.
                let (from, to) = (view.to_screen(g.pos), view.to_screen(screw_end(sl, g.pos)));
                draw_shaft(from, to, g.angle, 3.6 * view.s);
                draw_screw(a, b, Mechanism::screw_travel(g.angle) * view.s, 6.0 * view.s, mech::SCREW_LEAD / 2.0 * view.s);
            } else {
                draw_line(a.x, a.y, b.x, b.y, 6.0 * view.s, Color::from_rgba(52, 56, 70, 255));
                draw_line(a.x, a.y, b.x, b.y, 2.4 * view.s, Color::from_rgba(120, 126, 146, 255));
            }
            for p in [a, b] {
                draw_circle(p.x, p.y, 4.0 * view.s, Color::from_rgba(200, 204, 216, 255));
                draw_circle(p.x, p.y, 1.8 * view.s, Color::from_rgba(50, 54, 68, 255));
            }
        }
        for ax in &self.mech.axles {
            if let (Some(x), Some(y)) = (self.mech.gear_index(ax.a), self.mech.gear_index(ax.b)) {
                let (a, b) = (view.to_screen(self.mech.gears[x].pos), view.to_screen(self.mech.gears[y].pos));
                draw_shaft(a, b, self.mech.gears[x].angle, 3.6 * view.s);
            }
        }
        for g in &self.mech.gears {
            if let Some(b) = g.wheel.and_then(|w| self.blocks.iter().find(|b| b.uid == w)) {
                let (a, c) = (view.to_screen(g.pos), view.to_screen(b.pos));
                draw_shaft(a, c, g.angle, 3.6 * view.s);
            }
        }
        // Gears swinging tiges: the shaft to the pivot.
        for g in &self.mech.gears {
            if let Some(p) = self.arm_pivot(g) {
                draw_shaft(view.to_screen(g.pos), view.to_screen(p), g.angle, 3.6 * view.s);
            }
        }
        // Axles at the ends of tiges: in the air, or swung by a gear.
        let swung: Vec<(u32, u8)> = self.mech.gears.iter().filter_map(|g| g.arm).collect();
        let hubs = self.members.iter().flat_map(|m| [(m, 0u8, m.a, m.p0), (m, 1u8, m.b, m.p1)]).filter(|(m, end, a, _)| {
            swung.contains(&(m.uid, *end)) || matches!(a, Anchor::Ground(q) if !self.world.raw(q.x as i32, q.y as i32).is_hard())
        });
        for (_, _, _, p) in hubs {
            let c = view.to_screen(p);
            draw_circle(c.x, c.y, 3.6 * view.s, Color::from_rgba(40, 44, 58, 255));
            draw_circle_lines(c.x, c.y, 3.6 * view.s, 1.6, Color::from_rgba(214, 218, 230, 255));
            draw_circle(c.x, c.y, 1.3 * view.s, Color::from_rgba(214, 218, 230, 255));
        }
        for pv in &self.mech.pivots {
            if let Some(b) = self.blocks.iter().find(|b| b.uid == pv.wheel) {
                let c = view.to_screen(b.pos);
                draw_circle(c.x, c.y, 4.2 * view.s, Color::from_rgba(40, 44, 58, 255));
                draw_circle_lines(c.x, c.y, 4.2 * view.s, 1.6, Color::from_rgba(214, 218, 230, 255));
                draw_circle(c.x, c.y, 1.6 * view.s, Color::from_rgba(214, 218, 230, 255));
            }
        }
        for pu in &self.mech.pulleys {
            let c = view.to_screen(pu.pos);
            let r = pu.radius * view.s;
            // A bracket to its support, the sheave with its groove, the hub.
            draw_rectangle(c.x - r * 0.45, c.y - r * 0.45, r * 0.9, r * 1.9, Color::from_rgba(70, 74, 90, 255));
            draw_circle(c.x, c.y, r, Color::from_rgba(150, 156, 176, 255));
            draw_circle(c.x, c.y, r * 0.8, Color::from_rgba(96, 102, 124, 255));
            draw_circle_lines(c.x, c.y, r * 0.9, 1.4, Color::from_rgba(214, 218, 230, 255));
            draw_circle(c.x, c.y, r * 0.28, Color::from_rgba(214, 218, 230, 255));
            draw_circle(c.x, c.y, r * 0.12, Color::from_rgba(40, 44, 58, 255));
        }
        for g in &self.mech.gears {
            let c = view.to_screen(g.pos);
            let r = g.size.radius() * view.s;
            if let Some(m) = &g.motor {
                // Face-on motor: a housing behind the gear and an arrow for the way it turns. A
                // steam motor is brass, with a chimney on top.
                let half = r + 3.0 * view.s;
                let (body, edge) = if m.steam {
                    (Color::from_rgba(120, 88, 44, 255), Color::from_rgba(214, 170, 90, 255))
                } else {
                    (Color::from_rgba(58, 62, 78, 255), Color::from_rgba(150, 156, 176, 255))
                };
                if m.steam {
                    let cw = 3.0 * view.s;
                    draw_rectangle(c.x - cw / 2.0, c.y - half - 4.0 * view.s, cw, 4.0 * view.s, edge);
                }
                draw_rectangle(c.x - half, c.y - half, half * 2.0, half * 2.0, body);
                draw_rectangle_lines(c.x - half, c.y - half, half * 2.0, half * 2.0, 2.0, edge);
                let a0 = if m.dir > 0.0 { -0.4 } else { std::f32::consts::PI + 0.4 };
                let tip = c + vec2(a0.cos(), a0.sin()) * (r + 1.0 * view.s);
                draw_circle(tip.x, tip.y, 2.4 * view.s, if m.auto_reverse { GREEN } else { ACCENT });
            }
            let tint = if g.jammed { Color::new(1.0, 0.4, 0.4, 1.0) } else { WHITE };
            if g.stripped {
                draw_circle(c.x, c.y, r * 0.86, Color::from_rgba(96, 100, 116, 255));
                draw_circle_lines(c.x, c.y, r * 0.86, 2.0, Color::from_rgba(200, 80, 70, 255));
                draw_circle(c.x, c.y, r * 0.18, Color::from_rgba(40, 44, 58, 255));
            } else {
                draw_gear(c, r, g.angle, g.size, tint);
            }
            if let Some(drum) = g.reel {
                // The drum of a winch: rope wound round it, flanges that turn with the gear.
                let rd = drum * view.s;
                draw_circle(c.x, c.y, rd, Color::from_rgba(120, 88, 56, 255));
                for k in 0..3 {
                    draw_circle_lines(c.x, c.y, rd * (0.55 + 0.2 * k as f32), 1.6, Color::from_rgba(176, 140, 88, 255));
                }
                draw_circle_lines(c.x, c.y, rd, 2.2, Color::from_rgba(214, 218, 230, 255));
                for k in 0..4 {
                    let a = g.angle + k as f32 * std::f32::consts::FRAC_PI_2;
                    let tip = c + vec2(a.cos(), a.sin()) * rd;
                    draw_line(c.x, c.y, tip.x, tip.y, 2.0, Color::from_rgba(214, 218, 230, 255));
                }
                draw_circle(c.x, c.y, rd * 0.18, Color::from_rgba(40, 44, 58, 255));
            }
            if g.mount_is_free() {
                draw_circle(c.x, c.y, 1.6 * view.s, Color::from_rgba(255, 200, 80, 255));
            }
        }
        for r in &self.mech.rods {
            let (Some(pa), Some(pb)) = (self.rod_end_world(&r.a), self.rod_end_world(&r.b)) else { continue };
            let (pin, joint) = (view.to_screen(pa), view.to_screen(pb));
            let load = (r.force / r.strength).clamp(0.0, 1.0);
            let col = if r.broken {
                Color::new(0.6, 0.3, 0.3, 0.5)
            } else if stress_view {
                Color::new(0.3 + 0.7 * load, 1.0 - 0.8 * load, 0.3, 1.0)
            } else {
                Color::from_rgba(212, 176, 120, 255)
            };
            if r.broken {
                // A torn rod hangs from its first end.
                let tip = pin + vec2(0.0, r.len * 0.6 * view.s);
                draw_line(pin.x, pin.y, tip.x, tip.y, 2.4 * view.s, col);
            } else {
                draw_line(pin.x, pin.y, joint.x, joint.y, 3.0 * view.s, col);
                draw_circle(joint.x, joint.y, 3.4 * view.s, WHITE);
                draw_circle(joint.x, joint.y, 1.4 * view.s, Color::from_rgba(60, 64, 78, 255));
            }
            draw_circle(pin.x, pin.y, 3.4 * view.s, WHITE);
            draw_circle(pin.x, pin.y, 1.4 * view.s, Color::from_rgba(60, 64, 78, 255));
        }
        for sl in &self.mech.sliders {
            let c = view.to_screen(sl.point());
            let angle = sl.dir.y.atan2(sl.dir.x);
            draw_rectangle_ex(
                c.x,
                c.y,
                sl.half.x * 2.0 * view.s,
                sl.half.y * 2.0 * view.s,
                DrawRectangleParams { offset: vec2(0.5, 0.5), rotation: angle, color: Color::from_rgba(214, 160, 70, 255) },
            );
            let edge = c + vec2(angle.cos(), angle.sin()) * sl.half.x * view.s;
            draw_circle(edge.x, edge.y, 1.2 * view.s, Color::from_rgba(255, 230, 160, 255));
        }

        // The motor the remote control shows.
        if let Some(g) = self.selected_motor.and_then(|u| self.mech.gears.iter().find(|g| g.uid == u)) {
            let c = view.to_screen(g.pos);
            draw_circle_lines(c.x, c.y, (g.size.radius() + 6.0) * view.s, 2.5, ACCENT);
        }
        if stress_view {
            self.draw_mech_efforts(view, ui);
        }

        // What the cursor is about to build.
        if !inside {
            return;
        }
        let at = view.to_world(ui.mouse);
        match self.tool {
            Tool::Gear(size) => self.draw_gear_ghost(view, size, at),
            Tool::Motor => self.draw_gear_ghost(view, GearSize::Small, at),
            Tool::Winch => self.draw_gear_ghost(view, GearSize::Medium, at),
            Tool::Pulley => {
                let spot = self.snap_point(at).unwrap_or(at);
                let s = view.to_screen(spot);
                let free = !self.mech.pulleys.iter().any(|p| (p.pos - spot).length() < p.radius + PULLEY_R);
                draw_circle(s.x, s.y, PULLEY_R * view.s, if free { Color::new(0.5, 1.0, 0.6, 0.6) } else { Color::new(1.0, 0.3, 0.3, 0.6) });
                draw_circle_lines(s.x, s.y, PULLEY_R * view.s + 2.0, 2.0, GREEN);
            }
            Tool::Axle => {
                if let Some(MechDraft::Axle(g)) = self.mech_draft {
                    if let Some(i) = self.mech.gear_index(g) {
                        let a = view.to_screen(self.mech.gears[i].pos);
                        let hot = self.gear_at(at).is_some();
                        draw_line(a.x, a.y, ui.mouse.x, ui.mouse.y, 3.0, if hot { GREEN } else { Color::new(1.0, 1.0, 1.0, 0.5) });
                    }
                } else if let Some(i) = self.gear_at(at) {
                    let c = view.to_screen(self.mech.gears[i].pos);
                    draw_circle_lines(c.x, c.y, (self.mech.gears[i].size.radius() + 2.0) * view.s, 2.0, GREEN);
                } else if let Some(w) = self.wheel_at(at) {
                    let c = view.to_screen(self.blocks[w].pos);
                    draw_circle_lines(c.x, c.y, 14.0 * view.s, 2.0, GREEN);
                }
                // The pivot a tige would swing about.
                if self.gear_at(at).is_none() || matches!(self.mech_draft, Some(MechDraft::Axle(_))) {
                    let near = match self.mech_draft {
                        Some(MechDraft::Axle(g)) => self.mech.gear_index(g).map_or(at, |i| self.mech.gears[i].pos),
                        _ => at,
                    };
                    if let Some(p) = self.arm_end_at(at, near).and_then(|(u, e)| self.members.iter().find(|m| m.uid == u).map(|m| if e == 0 { m.p0 } else { m.p1 })) {
                        let c = view.to_screen(p);
                        draw_circle_lines(c.x, c.y, 6.0 * view.s, 2.0, GREEN);
                    }
                }
            }
            Tool::Rail => {
                let (spot, snapped) = match self.mech_draft {
                    Some(MechDraft::Rail(_, from)) => {
                        let (a, p) = self.rail_end(from, at);
                        (p, !matches!(a, Anchor::Free(_)) || (p - at).length() > 0.5)
                    }
                    _ => match self.snap_point(at) {
                        Some(p) => (p, true),
                        None => (at, false),
                    },
                };
                let s = view.to_screen(spot);
                if let Some(MechDraft::Rail(_, from)) = self.mech_draft {
                    let a = view.to_screen(from);
                    let len = (spot - from).length();
                    let ok = (RAIL_MIN..=RAIL_MAX).contains(&len);
                    draw_line(a.x, a.y, s.x, s.y, 5.0 * view.s, Color::new(0.5, 0.55, 0.7, 0.6));
                    ui.text(&format!("{len:.0} / {RAIL_MAX:.0}"), s.x + 10.0, s.y - 8.0, 15, if ok { WHITE } else { RED });
                }
                draw_circle(s.x, s.y, 3.5 * view.s, if snapped { GREEN } else { WHITE });
                draw_circle_lines(s.x, s.y, 6.0 * view.s, 2.0, if snapped { GREEN } else { ACCENT });
            }
            Tool::Pivot => {
                if let Some(w) = self.wheel_at(at) {
                    let b = &self.blocks[w];
                    let c = view.to_screen(b.pos);
                    draw_circle_lines(c.x, c.y, 14.0 * view.s, 2.0, GREEN);
                    draw_circle(c.x, c.y, 3.0 * view.s, GREEN);
                } else if let Some(p) = matches!(node_at(&self.world, &self.blocks, &self.members, at), Some(Anchor::End { .. })).then(|| self.snap_point(at)).flatten() {
                    let c = view.to_screen(p);
                    draw_circle_lines(c.x, c.y, 6.0 * view.s, 2.0, GREEN);
                    draw_circle(c.x, c.y, 2.5 * view.s, GREEN);
                }
            }
            Tool::Screw => {
                let from = match self.mech_draft {
                    Some(MechDraft::ScrewGear(g)) => self.mech.gear_index(g).map(|i| self.mech.gears[i].pos),
                    Some(MechDraft::ScrewRail(r)) => self.mech.slider_index(r).map(|i| self.mech.sliders[i].point()),
                    _ => None,
                };
                if let Some(p) = from {
                    let a = view.to_screen(p);
                    let hot = self.gear_at(at).is_some() || self.rail_at(at).is_some();
                    draw_line(a.x, a.y, ui.mouse.x, ui.mouse.y, 3.0, if hot { GREEN } else { Color::new(1.0, 1.0, 1.0, 0.5) });
                }
                if let Some(i) = self.gear_at(at) {
                    let c = view.to_screen(self.mech.gears[i].pos);
                    draw_circle_lines(c.x, c.y, (self.mech.gears[i].size.radius() + 2.0) * view.s, 2.0, GREEN);
                } else if let Some((i, _)) = self.rail_at(at) {
                    let sl = &self.mech.sliders[i];
                    let (a, b) = (view.to_screen(sl.origin), view.to_screen(sl.origin + sl.dir * sl.len));
                    draw_line(a.x, a.y, b.x, b.y, 8.0 * view.s, Color::new(0.5, 1.0, 0.6, 0.45));
                }
            }
            Tool::Crank => {
                if let Some(MechDraft::Crank { first }) = self.mech_draft {
                    if let Some(p) = self.rod_end_world(&first) {
                        let a = view.to_screen(p);
                        // Where the other end would attach: a gear's attach point, a carriage, or a node.
                        let target = if let Some(i) = self.gear_at(at) {
                            Some(self.pin_spot(i, at))
                        } else if let Some((si, s)) = self.rail_at(at) {
                            let sl = &self.mech.sliders[si];
                            let held = self.mech.rods.iter().any(|r| r.on_carriage(sl.uid));
                            Some(if held { sl.point() } else { sl.origin + sl.dir * s })
                        } else {
                            self.snap_point(at)
                        };
                        let end = target.map(|t| view.to_screen(t)).unwrap_or(ui.mouse);
                        draw_line(a.x, a.y, end.x, end.y, 3.0 * view.s, if target.is_some() { GREEN } else { Color::new(1.0, 1.0, 1.0, 0.5) });
                        draw_circle(a.x, a.y, 4.0 * view.s, WHITE);
                        if target.is_some() {
                            draw_circle_lines(end.x, end.y, 6.0 * view.s, 2.0, GREEN);
                        }
                    }
                }
                if let Some(i) = self.gear_at(at) {
                    // The fixed attach points of the gear, the nearest one lit.
                    let g = &self.mech.gears[i];
                    let c = view.to_screen(g.pos);
                    draw_circle_lines(c.x, c.y, (g.size.radius() + 2.0) * view.s, 2.0, GREEN);
                    let spot = self.pin_spot(i, at);
                    for local in attach_points(g.size) {
                        let p = view.to_screen(g.pos + rot_vec(g.angle, local));
                        draw_circle(p.x, p.y, 1.8 * view.s, Color::new(1.0, 1.0, 1.0, 0.75));
                    }
                    let p = view.to_screen(spot);
                    draw_circle(p.x, p.y, 3.6 * view.s, GREEN);
                    draw_circle_lines(p.x, p.y, 6.0 * view.s, 2.0, WHITE);
                } else if self.mech_draft.is_none() {
                    if let Some(p) = self.snap_point(at) {
                        let s = view.to_screen(p);
                        draw_circle(s.x, s.y, 3.5 * view.s, GREEN);
                        draw_circle_lines(s.x, s.y, 6.0 * view.s, 2.0, WHITE);
                    }
                }
            }
            _ => {}
        }
    }

    /// The Efforts view for mechanisms: torque on every shaft, tooth load, force in the rods and on
    /// the carriages, and how hard each motor is pushing.
    fn draw_mech_efforts(&self, view: View, ui: &Ui) {
        for g in &self.mech.gears {
            let c = view.to_screen(g.pos);
            let r = (g.size.radius() + 1.5) * view.s;
            // A ring coloured by how close the teeth are to stripping.
            let load = g.tooth_load.clamp(0.0, 1.0);
            let (cr, cg, cb) = render::stress_rgb(load);
            draw_circle_lines(c.x, c.y, r, 3.0, Color::new(cr, cg, cb, 0.95));
            // An arc for the torque, turning the way the shaft turns.
            if g.torque > 0.05 {
                let sweep = (g.torque / 40.0).clamp(0.08, 1.0) * std::f32::consts::TAU * 0.5;
                let dir = if g.omega >= 0.0 { 1.0 } else { -1.0 };
                let start = -std::f32::consts::FRAC_PI_2;
                let mut prev = c + vec2(start.cos(), start.sin()) * (r + 4.0 * view.s);
                for k in 1..=16 {
                    let a = start + dir * sweep * k as f32 / 16.0;
                    let p = c + vec2(a.cos(), a.sin()) * (r + 4.0 * view.s);
                    draw_line(prev.x, prev.y, p.x, p.y, 3.0, Color::new(1.0, 0.85, 0.3, 0.9));
                    prev = p;
                }
                let tip = start + dir * sweep;
                let (t0, t1) = (c + vec2(tip.cos(), tip.sin()) * (r + 4.0 * view.s), vec2(-tip.sin(), tip.cos()) * dir);
                draw_triangle(t0 + t1 * 5.0, t0 - t1 * 1.0 + vec2(tip.cos(), tip.sin()) * 3.0, t0 - t1 * 1.0 - vec2(tip.cos(), tip.sin()) * 3.0, Color::new(1.0, 0.85, 0.3, 0.9));
                ui.text(&format!("C {:.0}", g.torque), c.x - 12.0, c.y + r + 18.0 * view.s.max(0.8), 12, WHITE);
            }
            if g.motor.is_some() {
                ui.text(&format!("{:.0} %", g.motor_load * 100.0), c.x - 12.0, c.y - r - 12.0 * view.s.max(0.8), 12, if g.motor_load > 0.8 { RED } else { WHITE });
            }
        }
        for r in &self.mech.rods {
            if r.broken {
                continue;
            }
            let (Some(a), Some(b)) = (self.rod_end_world(&r.a), self.rod_end_world(&r.b)) else { continue };
            if r.force > 0.05 {
                let m = view.to_screen((a + b) * 0.5);
                ui.text(&format!("F {:.1}", r.force), m.x + 6.0, m.y - 6.0, 12, WHITE);
            }
        }
        for sl in &self.mech.sliders {
            if sl.load < 0.02 {
                continue;
            }
            // The force on the carriage as an arrow along the rail, and a pressure on its face.
            let c = view.to_screen(sl.point());
            let dir = if sl.v >= 0.0 { sl.dir } else { -sl.dir };
            let len = (6.0 + sl.load * 14.0).min(60.0) * view.s;
            let tip = c + dir * (sl.half.x * view.s + len);
            let from = c + dir * sl.half.x * view.s;
            draw_line(from.x, from.y, tip.x, tip.y, 3.0, Color::new(1.0, 0.5, 0.2, 0.95));
            let side = vec2(-dir.y, dir.x);
            draw_triangle(tip + dir * 6.0, tip + side * 4.0, tip - side * 4.0, Color::new(1.0, 0.5, 0.2, 0.95));
            let pressure = sl.load / (sl.half.y * 2.0).max(1.0);
            ui.text(&format!("F {:.1}  p {:.2}", sl.load, pressure), c.x - 14.0, c.y - sl.half.y * view.s - 8.0, 12, WHITE);
        }
    }

    fn draw_gear_ghost(&self, view: View, size: GearSize, at: Vec2) {
        let c = self.gear_spot(size, at);
        let ok = self.gear_spot_ok(size, c);
        let tint = if ok { Color::new(0.5, 1.0, 0.6, 0.55) } else { Color::new(1.0, 0.3, 0.3, 0.55) };
        let s = view.to_screen(c);
        // Guide lines to the gears this one is lined up with.
        for g in &self.mech.gears {
            let o = view.to_screen(g.pos);
            if (g.pos.y - c.y).abs() < 0.01 || (g.pos.x - c.x).abs() < 0.01 {
                draw_line(o.x, o.y, s.x, s.y, 1.2, Color::new(1.0, 0.85, 0.3, 0.7));
            }
        }
        draw_gear(s, size.radius() * view.s, 0.0, size, tint);
        if (c - at).length() > 0.5 {
            draw_circle_lines(s.x, s.y, (size.radius() + 1.5) * view.s, 2.0, GREEN);
            draw_circle(s.x, s.y, 2.5 * view.s, GREEN);
        }
    }
}

/// A unit direction pulled onto the nearest of the eight compass directions when within `tol` degrees.
pub(crate) fn snap_to_axes(dir: Vec2, tol_deg: f32) -> Vec2 {
    let a = dir.y.atan2(dir.x);
    let step = std::f32::consts::FRAC_PI_4;
    let k = (a / step).round();
    if (a - k * step).abs() < tol_deg.to_radians() {
        vec2((k * step).cos(), (k * step).sin())
    } else {
        dir
    }
}

/// The fixed attach points of a gear (in its own frame): two rings of eight, plus the centre.
fn attach_points(size: GearSize) -> Vec<Vec2> {
    let r = size.radius();
    let mut pts = vec![Vec2::ZERO];
    for ring in [0.45, 0.8] {
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            pts.push(vec2(a.cos(), a.sin()) * r * ring);
        }
    }
    pts
}

pub fn rot_vec(a: f32, v: Vec2) -> Vec2 {
    let (s, c) = a.sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

impl Gear {
    fn mount_is_free(&self) -> bool {
        matches!(self.mount, Anchor::Free(_))
    }
}

#[cfg(test)]
impl Game {
    pub fn pin_spot_for_test(&self, gi: usize, p: Vec2) -> Vec2 {
        self.pin_spot(gi, p)
    }

    /// Test helper: an axle between the first two gears.
    pub fn axle_for_test(&mut self) {
        if self.mech.gears.len() >= 2 {
            let (a, b) = (self.mech.gears[0].uid, self.mech.gears[1].uid);
            self.mech.axles.push(Axle { uid: mech::next_uid(), a, b });
        }
    }
}

// ----- the remote control of a motor -------------------------------------------------------

/// Turns per minute for a speed in radians per step (60 steps a second).
fn rpm(rad_per_step: f32) -> f32 {
    rad_per_step * 60.0 * 60.0 / std::f32::consts::TAU
}

impl Game {
    /// Where the remote control goes: just under the views palette, when a motor is selected.
    pub fn remote_rect(&self, views: Rect) -> Option<Rect> {
        let uid = self.selected_motor?;
        self.mech.gear_index(uid).filter(|&i| self.mech.gears[i].motor.is_some())?;
        Some(Rect::new(views.x, views.y + views.h + 8.0, views.w, 316.0))
    }

    pub fn draw_remote(&mut self, ui: &mut Ui, panel: Rect) {
        let Some(gi) = self.selected_motor.and_then(|u| self.mech.gear_index(u)) else { return };
        rrect(panel, 12.0, Color::new(0.09, 0.1, 0.17, 0.95));
        ui.text("Télécommande", panel.x + 12.0, panel.y + 22.0, 16, ACCENT);
        let x = panel.x + 10.0;
        let w = panel.w - 20.0;
        let mut y = panel.y + 34.0;

        let g = &self.mech.gears[gi];
        let Some(mut m) = g.motor else { return };
        let (omega, load, shaft) = (g.omega, g.motor_load, g.shaft_torque);
        let before = m;

        let half = (w - 6.0) / 2.0;
        if ui.button(Rect::new(x, y, half, 26.0), if m.on { "En marche" } else { "Arrêté" }, Icon::None, m.on, true, GREEN) {
            m.on = !m.on;
        }
        if ui.button(Rect::new(x + half + 6.0, y, half, 26.0), if m.dir > 0.0 { "Horaire" } else { "Antihoraire" }, Icon::Rotate, false, true, BLUE) {
            m.dir = -m.dir;
        }
        y += 38.0;

        ui.text("Vitesse à vide", x, y + 9.0, 13, DIM);
        ui.text(&format!("{:.0} tr/min", rpm(m.free_speed)), x + w - 62.0, y + 9.0, 13, TEXT);
        y += 14.0;
        ui.slider(Rect::new(x, y, w, 18.0), &mut m.free_speed, 0.02, 0.25, ACCENT);
        y += 26.0;

        ui.text("Couple", x, y + 9.0, 13, DIM);
        ui.text(&format!("{:.0}", m.torque), x + w - 24.0, y + 9.0, 13, TEXT);
        y += 14.0;
        ui.slider(Rect::new(x, y, w, 18.0), &mut m.torque, 4.0, 40.0, ACCENT);
        y += 26.0;

        if ui.checkbox(Rect::new(x, y, w, 24.0), "Retour si surcharge", m.auto_reverse, true) {
            m.auto_reverse = !m.auto_reverse;
        }
        y += 30.0;
        ui.text("Seuil retour", x, y + 9.0, 13, if m.auto_reverse { DIM } else { Color::new(0.4, 0.42, 0.5, 1.0) });
        ui.text(&format!("{:.0} %", m.reverse_at * 100.0), x + w - 36.0, y + 9.0, 13, TEXT);
        y += 14.0;
        ui.slider(Rect::new(x, y, w, 18.0), &mut m.reverse_at, 0.3, 1.2, if m.auto_reverse { ACCENT } else { DIM });
        y += 28.0;

        if ui.checkbox(Rect::new(x, y, w, 24.0), "À vapeur (chaudière fermée)", m.steam, true) {
            m.steam = !m.steam;
            m.power = 1.0;
        }
        y += 28.0;
        if m.steam {
            let col = if m.pressure > 1.05 { TEXT } else { DIM };
            ui.text(&format!("Vapeur {:.1} atm   puissance {:.0} %", m.pressure, m.power * 100.0), x, y + 8.0, 13, col);
        } else {
            ui.text("Pose-le contre une chaudière", x, y + 8.0, 12, DIM);
        }
        y += 22.0;

        // What it is doing right now.
        ui.text(&format!("Rotation : {:.0} tr/min", rpm(omega.abs())), x, y + 8.0, 13, TEXT);
        y += 16.0;
        let bar = Rect::new(x, y, w, 10.0);
        rrect(bar, 5.0, Color::new(0.0, 0.0, 0.0, 0.4));
        let fill = (load / 1.2).clamp(0.0, 1.0);
        let over = load > m.reverse_at;
        rrect(Rect::new(bar.x, bar.y, (bar.w * fill).max(if load > 0.0 { 5.0 } else { 0.0 }), bar.h), 5.0, if over { RED } else { GREEN });
        // The turn-back threshold as a mark on the load bar.
        let mark = bar.x + bar.w * (m.reverse_at / 1.2).clamp(0.0, 1.0);
        draw_line(mark, bar.y - 3.0, mark, bar.y + bar.h + 3.0, 2.0, if m.auto_reverse { WHITE } else { DIM });
        ui.text(&format!("Charge {:.0} %   couple {:.1}", load * 100.0, shaft), x, y + 24.0, 12, DIM);

        if m != before {
            if let Some(g) = self.mech.gears.get_mut(gi) {
                g.motor = Some(m);
            }
        }
    }
}
