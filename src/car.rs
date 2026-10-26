//! Cars: a rigid chassis on spring/damper suspensions, wheels with their own
//! spin driven by an engine torque curve, and traction limited by grip.
//! Units are grid cells and simulation steps.

use crate::world::{Mat, World, H, W};
use macroquad::prelude::{vec2, Vec2};

/// Everything below is tuned in "design units"; this shrinks the car to fit the finer grid.
pub const S: f32 = 0.67;
const GRAVITY: f32 = 0.12 * S;
const WHEEL_INERTIA: f32 = 8.0 * S * S;
const SLIP_GAIN: f32 = 2.0;
const SAMPLES: usize = 24;
/// Speed (design units) above which the hull breaks through glass.
const GLASS_BREAK_SPEED: f32 = 0.5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModelKind {
    Buggy,
    Race,
    Offroad,
    Truck,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Drive {
    Rear,
    All,
}

pub struct WheelSpec {
    pub x: f32,
    pub r: f32,
}

pub struct Model {
    pub name: &'static str,
    pub blurb: &'static str,
    pub wheels: &'static [WheelSpec],
    pub drive: Drive,
    /// Local y of the suspension anchors (positive is down).
    pub anchor_y: f32,
    pub travel: f32,
    pub stiffness: f32,
    pub damping: f32,
    /// Peak engine torque per driven wheel.
    pub torque: f32,
    /// Speed (cells/step) where the torque curve falls to zero.
    pub top_speed: f32,
    pub grip: f32,
    pub inertia: f32,
    pub crash_speed: f32,
    pub hull_half: f32,
    pub hull_top: f32,
    pub hull_bottom: f32,
    pub color: (u8, u8, u8),
    /// Weight pressed onto whatever it drives over (physics units, kg).
    pub mass: f32,
}

const BUGGY: Model = Model {
    name: "Citadine",
    blurb: "Équilibrée",
    wheels: &[WheelSpec { x: -12.0, r: 7.0 }, WheelSpec { x: 12.0, r: 7.0 }],
    drive: Drive::Rear,
    anchor_y: 5.0,
    travel: 6.0,
    stiffness: 0.020,
    damping: 0.10,
    torque: 0.50,
    top_speed: 1.1,
    grip: 1.1,
    inertia: 70.0,
    crash_speed: 3.2,
    hull_half: 14.0,
    hull_top: -9.0,
    hull_bottom: 3.0,
    color: (240, 190, 40),
    mass: 1.5,
};

const RACE: Model = Model {
    name: "Course",
    blurb: "Rapide, fragile",
    wheels: &[WheelSpec { x: -15.0, r: 6.0 }, WheelSpec { x: 15.0, r: 6.0 }],
    drive: Drive::Rear,
    anchor_y: 3.0,
    travel: 3.5,
    stiffness: 0.040,
    damping: 0.16,
    torque: 0.80,
    top_speed: 2.1,
    grip: 1.4,
    inertia: 90.0,
    crash_speed: 2.6,
    hull_half: 18.0,
    hull_top: -6.0,
    hull_bottom: 2.0,
    color: (220, 50, 60),
    mass: 1.0,
};

const OFFROAD: Model = Model {
    name: "4x4",
    blurb: "Grimpe partout",
    wheels: &[WheelSpec { x: -15.0, r: 10.0 }, WheelSpec { x: 15.0, r: 10.0 }],
    drive: Drive::All,
    anchor_y: 6.0,
    travel: 10.0,
    stiffness: 0.012,
    damping: 0.07,
    torque: 1.20,
    top_speed: 0.9,
    grip: 1.6,
    inertia: 120.0,
    crash_speed: 5.0,
    hull_half: 18.0,
    hull_top: -12.0,
    hull_bottom: 4.0,
    color: (70, 150, 80),
    mass: 2.2,
};

const TRUCK: Model = Model {
    name: "Camion",
    blurb: "Lourd, costaud",
    wheels: &[WheelSpec { x: -22.0, r: 8.0 }, WheelSpec { x: 0.0, r: 8.0 }, WheelSpec { x: 22.0, r: 8.0 }],
    drive: Drive::All,
    anchor_y: 6.0,
    travel: 7.0,
    stiffness: 0.015,
    damping: 0.09,
    torque: 1.20,
    top_speed: 0.75,
    grip: 1.2,
    inertia: 200.0,
    crash_speed: 4.2,
    hull_half: 27.0,
    hull_top: -14.0,
    hull_bottom: 4.0,
    color: (70, 120, 220),
    mass: 3.5,
};

impl ModelKind {
    pub const ALL: [ModelKind; 4] = [ModelKind::Buggy, ModelKind::Race, ModelKind::Offroad, ModelKind::Truck];

    pub fn model(self) -> &'static Model {
        match self {
            ModelKind::Buggy => &BUGGY,
            ModelKind::Race => &RACE,
            ModelKind::Offroad => &OFFROAD,
            ModelKind::Truck => &TRUCK,
        }
    }
}

/// Player adjustments applied on top of a model (1.0 = stock).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Tuning {
    /// Engine torque.
    pub power: f32,
    /// Top speed.
    pub speed: f32,
    /// Suspension spring stiffness.
    pub stiffness: f32,
    /// Suspension damping.
    pub damping: f32,
    /// Tyre grip.
    pub grip: f32,
    /// Wheel size.
    pub wheel: f32,
    /// How hard a crash it survives.
    pub tough: f32,
    /// Weight pressed on bridges.
    pub weight: f32,
    pub color: Option<(u8, u8, u8)>,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning { power: 1.0, speed: 1.0, stiffness: 1.0, damping: 1.0, grip: 1.0, wheel: 1.0, tough: 1.0, weight: 1.0, color: None }
    }
}

impl Tuning {
    pub const COLORS: [(u8, u8, u8); 8] = [
        (240, 190, 40),
        (220, 50, 60),
        (70, 150, 80),
        (70, 120, 220),
        (170, 90, 210),
        (240, 130, 40),
        (235, 235, 240),
        (40, 44, 54),
    ];
}

static NEXT_CAR_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fate {
    Driving,
    Arrived,
    Dead,
    Exploded,
}

#[derive(Clone, Copy)]
pub struct WheelState {
    pub pos: Vec2,
    pub ext: f32,
    pub spin: f32,
    /// Accumulated rotation, for drawing.
    pub roll: f32,
    pub contact: bool,
}

#[derive(Clone)]
pub struct Car {
    pub id: u32,
    pub tuning: Tuning,
    pub kind: ModelKind,
    pub p: Vec2,
    pub v: Vec2,
    pub angle: f32,
    pub omega: f32,
    pub wheels: Vec<WheelState>,
    pub dir: f32,
    pub fate: Fate,
    /// Where the wheels press on the ground this step: (point, share of the weight).
    pub loads: Vec<(Vec2, f32)>,
    /// Glass cells the hull smashed through this step.
    pub smashed: Vec<(i32, i32)>,
    /// Sand, mud or snow thrown up by the wheels this step: (where, velocity in cells/step).
    pub sprays: Vec<(Vec2, Vec2)>,
    /// 0..1: how deep the wheels have dug themselves into soft ground.
    pub dig: f32,
    smash_grace: u32,
    /// Ticks spent nearly stopped while the engine pushes (a snag).
    stall: u32,
    /// While > 0 the driver floors it: more torque and grip to get over an obstacle.
    boost: u32,
    /// Snags that the boost could not get past in the current window.
    failed_boosts: u32,
    damage: u32,
    ticks: u32,
    last_x: f32,
    progress: f32,
    grounded: u32,
    upside_down: u32,
}

/// How loose a ground material is for a wheel: 0 for rock, 1 for sand or mud.
fn softness(m: Mat) -> f32 {
    match m {
        Mat::Sand | Mat::Mud | Mat::Snow | Mat::Dirt | Mat::Grass => 1.0,
        _ => 0.0,
    }
}

/// How soft the ground touching a wheel is (0..1): the softness of the ground cells
/// within a cell or two of the tyre, averaged.
fn soft_share(world: &World, c: Vec2, r: f32) -> f32 {
    let (mut soft, mut ground) = (0.0f32, 0u32);
    let reach = r + 2.0;
    let ri = reach.ceil() as i32;
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            if (dx * dx + dy * dy) as f32 <= reach * reach {
                let m = world.at(c.x.floor() as i32 + dx, c.y.floor() as i32 + dy);
                if m.is_hard() && !m.one_way() {
                    ground += 1;
                    soft += softness(m);
                }
            }
        }
    }
    if ground == 0 { 0.0 } else { soft / ground as f32 }
}

fn rot(a: f32, v: Vec2) -> Vec2 {
    let (s, c) = a.sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// How far up (cells) a wheel climbs onto the top of a tree it rolls against.
const TREE_CLIMB: f32 = 3.5;

/// Hard for a wheel whose bottom was at row `feet` before the car moved, except glass it is going
/// fast enough to smash. A tree holds the wheel up only from above (`World::bears`): the car
/// drives through a trunk, and on a crown it comes down on.
fn solid(world: &World, fast: bool, feet: f32, x: i32, y: i32) -> bool {
    if fast && world.at(x, y) == Mat::Glass {
        return false;
    }
    world.bears(x, y, feet, TREE_CLIMB)
}

/// Outward normal of the hard cells touching a circle, if any.
fn probe(world: &World, fast: bool, feet: f32, c: Vec2, radius: f32) -> Option<Vec2> {
    let mut sum = Vec2::ZERO;
    for k in 0..SAMPLES {
        let a = k as f32 / SAMPLES as f32 * std::f32::consts::TAU;
        let d = vec2(a.cos(), a.sin());
        let p = c + d * radius;
        if solid(world, fast, feet, p.x.floor() as i32, p.y.floor() as i32) {
            sum -= d;
        }
    }
    (sum.length() > 1e-3).then(|| sum.normalize())
}

fn overlaps(world: &World, fast: bool, feet: f32, c: Vec2, radius: f32) -> bool {
    for k in 0..SAMPLES {
        let a = k as f32 / SAMPLES as f32 * std::f32::consts::TAU;
        let p = c + vec2(a.cos(), a.sin()) * radius;
        if solid(world, fast, feet, p.x.floor() as i32, p.y.floor() as i32) {
            return true;
        }
    }
    solid(world, fast, feet, c.x.floor() as i32, c.y.floor() as i32)
}

impl Car {
    #[allow(dead_code)]
    pub fn new(kind: ModelKind, at: (i32, i32)) -> Self {
        Self::with_tuning(kind, at, Tuning::default())
    }

    pub fn with_tuning(kind: ModelKind, at: (i32, i32), tuning: Tuning) -> Self {
        let m = kind.model();
        let tallest = m.wheels.iter().fold(0.0f32, |a, w| a.max(w.r)) * tuning.wheel;
        let p = vec2(at.0 as f32, at.1 as f32 + 22.0 - (m.anchor_y + m.travel + tallest) * S);
        let wheels = m
            .wheels
            .iter()
            .map(|w| WheelState {
                pos: p + vec2(w.x * S, (m.anchor_y + m.travel * 0.5) * S),
                ext: m.travel * S * 0.5,
                spin: 0.0,
                roll: 0.0,
                contact: false,
            })
            .collect();
        Car {
            id: NEXT_CAR_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            tuning,
            kind,
            p,
            v: Vec2::ZERO,
            angle: 0.0,
            omega: 0.0,
            wheels,
            dir: 1.0,
            fate: Fate::Driving,
            loads: Vec::new(),
            smashed: Vec::new(),
            sprays: Vec::new(),
            dig: 0.0,
            smash_grace: 0,
            stall: 0,
            boost: 0,
            failed_boosts: 0,
            damage: 0,
            ticks: 0,
            last_x: p.x,
            progress: 0.0,
            grounded: 0,
            upside_down: 0,
        }
    }

    pub fn wheel_r(&self, i: usize) -> f32 {
        self.kind.model().wheels[i].r * S * self.tuning.wheel
    }

    /// Weight pressed on whatever it drives over, in kg.
    fn speed_along_dir(&self) -> f32 {
        self.v.x * self.dir
    }

    pub fn weight_kg(&self) -> f32 {
        self.kind.model().mass * self.tuning.weight
    }

    pub fn color_rgb(&self) -> (u8, u8, u8) {
        self.tuning.color.unwrap_or(self.kind.model().color)
    }

    pub fn anchor(&self, i: usize) -> Vec2 {
        let m = self.kind.model();
        self.p + rot(self.angle, vec2(m.wheels[i].x, m.anchor_y) * S)
    }

    /// Forward speed along the direction of travel, cells per step.
    #[cfg(test)]
    pub fn speed(&self) -> f32 {
        self.v.x * self.dir
    }

    fn is_driven(&self, i: usize) -> bool {
        let m = self.kind.model();
        match m.drive {
            Drive::All => true,
            Drive::Rear => m.wheels[i].x * self.dir <= 0.0,
        }
    }

    pub fn step(&mut self, world: &World, exit: Option<(i32, i32)>) {
        if self.fate != Fate::Driving {
            return;
        }
        self.ticks += 1;
        self.loads.clear();
        self.smashed.clear();
        self.sprays.clear();
        let mut soft_total = 0.0f32;
        let mut max_slip = 0.0f32;
        let mut load_total = 0.0f32;
        let m = self.kind.model();
        // A car that is already ploughing through glass keeps going.
        let fast = self.v.length() > GLASS_BREAK_SPEED * S || self.smash_grace > 0;
        self.smash_grace = self.smash_grace.saturating_sub(1);
        if fast {
            // Glass in the path of a wheel shatters.
            for w in &self.wheels {
                let r = m.wheels[0].r * S * self.tuning.wheel + 2.0;
                for dy in (-(r as i32))..=(r as i32) {
                    for dx in (-(r as i32))..=(r as i32) {
                        let cell = (w.pos.x.floor() as i32 + dx, w.pos.y.floor() as i32 + dy);
                        if (dx * dx + dy * dy) as f32 <= r * r && world.at(cell.0, cell.1) == Mat::Glass && self.smashed.len() < 24 {
                            self.smashed.push(cell);
                        }
                    }
                }
            }
        }
        let travel_len = m.travel * S;
        let crash = m.crash_speed * S * self.tuning.tough;
        let down = rot(self.angle, vec2(0.0, 1.0));
        let mut acc = vec2(0.0, GRAVITY);
        let mut torque = 0.0f32;
        let mut crashed = false;
        let mut touching = 0;
        let mut wet = false;
        let ground_speed = self.v.x * self.dir;

        for i in 0..self.wheels.len() {
            let r = self.wheel_r(i);
            let anchor = self.anchor(i);
            let at = |e: f32| anchor + down * e;
            // Where the bottom of the wheel was: a tree under it holds it up.
            let feet = self.wheels[i].pos.y + r;

            // Find how far the wheel can extend before touching the ground.
            let mut ext = travel_len;
            let mut depth = 0.0;
            let mut contact = false;
            if overlaps(world, fast, feet, at(travel_len), r) {
                contact = true;
                let mut e = travel_len;
                while e >= 0.0 && overlaps(world, fast, feet, at(e), r) {
                    e -= 0.3;
                }
                if e < 0.0 {
                    let mut d = 0.0;
                    while d < r * 2.0 && overlaps(world, fast, feet, at(-d), r) {
                        d += 0.3;
                    }
                    depth = d;
                    ext = 0.0;
                } else {
                    ext = e;
                }
            }
            let pos = at(ext);
            self.wheels[i].pos = pos;
            self.wheels[i].ext = ext;
            self.wheels[i].contact = contact;
            if world.at(pos.x.floor() as i32, pos.y.floor() as i32) == Mat::Water
                || world.at(pos.x.floor() as i32, pos.y.floor() as i32) == Mat::Lava
            {
                wet = true;
            }

            let mut ft = 0.0;
            if contact {
                touching += 1;
                let n = probe(world, fast, feet, pos, r + 1.0).unwrap_or(-down);
                let ra = anchor - self.p;
                let va = self.v + vec2(-ra.y, ra.x) * self.omega;
                let vn = va.dot(n);
                if depth > 0.0 && vn < -crash {
                    crashed = true;
                }
                let comp = travel_len - ext + depth * 2.0;
                let normal_force = (m.stiffness * self.tuning.stiffness * comp - m.damping * self.tuning.damping * vn).max(0.0);
                let f = n * normal_force;
                self.loads.push((pos - n * (r + 0.8), normal_force));
                load_total += normal_force;
                acc += f;
                torque += cross(ra, f);

                let t = vec2(-n.y, n.x);
                // Thrust acts at hub height: keeps wheelies tame.
                let rc = pos - self.p;
                let vhub = self.v + vec2(-(pos - self.p).y, (pos - self.p).x) * self.omega;
                let slip = self.wheels[i].spin * r - vhub.dot(t);
                let soft = (soft_share(world, pos, r)).min(1.0);
                soft_total += soft;
                let grip = m.grip * self.tuning.grip * if self.boost > 0 { 1.35 } else { 1.0 } * (1.0 - 0.2 * soft) * (1.0 - 0.3 * self.dig);
                ft = (SLIP_GAIN * slip).clamp(-grip * normal_force, grip * normal_force);
                if soft > 0.3 {
                    max_slip = max_slip.max(slip.abs());
                }
                // A spinning wheel in loose ground flings some of it up and back.
                if soft > 0.2 && (slip.abs() > 0.8 || self.v.length() > 0.35) && (self.ticks + i as u32 * 7) % 3 == 0 && self.sprays.len() < 6 {
                    let k = (self.ticks.wrapping_mul(2654435761u32) >> 28) as f32 / 16.0;
                    let back = -self.dir * (0.8 + k * 1.6);
                    self.sprays.push((pos + vec2(0.0, r * 0.4), vec2(back, -(1.0 + (1.0 - k) * 1.8))));
                }
                let tf = t * ft;
                acc += tf;
                torque += cross(rc, tf);
            }

            // Engine torque curve, then reaction from the ground.
            let mut te = 0.0;
            if self.is_driven(i) {
                let w_max = m.top_speed * self.tuning.speed * S / r;
                let forward = (self.dir * self.wheels[i].spin).max(ground_speed / r);
                te = self.dir * m.torque * self.tuning.power * S * S * if self.boost > 0 { 2.6 } else { 1.25 } * (1.0 - forward / w_max).clamp(-0.2, 1.5);
            }
            let w = &mut self.wheels[i];
            w.spin += (te - ft * r) / WHEEL_INERTIA;
            w.spin *= 0.9995;
            w.roll += w.spin;
        }

        if load_total > 0.0 {
            for l in &mut self.loads {
                l.1 /= load_total;
            }
        }
        let soft = (soft_total / self.wheels.len().max(1) as f32).min(1.0);
        if soft > 0.15 {
            // Rolling resistance in loose ground; a spinning, slow car digs itself in.
            self.v *= 1.0 - 0.008 * soft - 0.01 * self.dig;
            if (max_slip > 0.6 && self.v.length() < 0.45) || self.v.length() < 0.2 {
                self.dig = (self.dig + 0.003 * soft).min(1.0);
            }
        }
        if soft < 0.05 || self.v.length() > 0.3 {
            self.dig = (self.dig - 0.02).max(0.0);
        }
        if wet {
            self.v *= 0.985;
            for w in &mut self.wheels {
                w.spin *= 0.97;
            }
        }

        self.v += acc;
        self.omega += torque / (m.inertia * S * S);
        self.omega *= 0.995;
        self.v *= 0.9998;
        self.p += self.v;
        self.angle += self.omega;

        // Chassis hull against the terrain.
        let (hh, ht, hb) = (m.hull_half * S, m.hull_top * S, m.hull_bottom * S);
        let hull = [
            vec2(-hh, ht),
            vec2(0.0, ht),
            vec2(hh, ht),
            vec2(-hh, (ht + hb) * 0.5),
            vec2(hh, (ht + hb) * 0.5),
            vec2(-hh * 0.55, hb),
            vec2(hh * 0.55, hb),
            vec2(0.0, hb),
        ];
        for _ in 0..8 {
            let mut sum = Vec2::ZERO;
            for h in hull {
                let q = self.p + rot(self.angle, h);
                let cell = (q.x.floor() as i32, q.y.floor() as i32);
                if world.at(cell.0, cell.1) == Mat::Glass && fast {
                    // Fast enough: the car goes through and the glass shatters.
                    self.smashed.push(cell);
                } else if world.hard_for_car(cell.0, cell.1) {
                    sum += (q - self.p).normalize_or_zero();
                }
            }
            if sum.length() < 1e-3 {
                break;
            }
            let n = -sum.normalize();
            self.p += n * 0.6;
            let vn = self.v.dot(n);
            if vn < -crash {
                crashed = true;
            }
            if vn < 0.0 {
                self.v -= n * vn;
                self.v *= 0.99;
                self.omega *= 0.9;
            }
        }
        if !self.smashed.is_empty() {
            self.smash_grace = 12;
            self.v *= 0.995;
        }
        if crashed {
            self.fate = Fate::Exploded;
            return;
        }

        // Upside down for too long counts as a wreck.
        if self.angle.cos() < -0.2 {
            self.upside_down += 1;
            if self.upside_down > 45 {
                self.fate = Fate::Exploded;
                return;
            }
        } else {
            self.upside_down = 0;
        }

        // Snagged on something? Floor it before giving up and turning around.
        self.progress += (self.p.x - self.last_x) * self.dir;
        self.last_x = self.p.x;
        if touching > 0 {
            self.grounded += 1;
        }
        let creeping = touching > 0 && self.speed_along_dir() < 0.1 * S;
        self.stall = if creeping { self.stall + 1 } else { 0 };
        if self.boost > 0 {
            self.boost -= 1;
        } else if self.stall > 14 {
            self.boost = 55;
            self.stall = 0;
            self.failed_boosts += 1;
            // A little hop forward to get the front wheels up onto a ledge.
            self.v.y -= 0.55 * S * (1.0 + self.tuning.power * 0.3);
            self.v.x += 0.35 * S * self.dir;
            self.omega -= 0.01 * self.dir;
        }
        // Lemmings: only after a couple of boosted attempts fail do we turn around.
        if self.ticks % 90 == 0 {
            if self.grounded >= 45 && self.progress < 3.0 * S && self.failed_boosts >= 2 {
                self.dir = -self.dir;
                self.failed_boosts = 0;
            } else if self.progress >= 3.0 * S {
                self.failed_boosts = 0;
            }
            self.progress = 0.0;
            self.grounded = 0;
        }

        // Fire and lava wreck it fast: it blows up.
        let mut heat = 0u32;
        let mut pts = vec![self.p];
        pts.extend(self.wheels.iter().map(|w| w.pos));
        for p in pts {
            for k in 0..8 {
                let a = k as f32 / 8.0 * std::f32::consts::TAU;
                let q = p + vec2(a.cos(), a.sin()) * (10.0 * S);
                match world.at(q.x.floor() as i32, q.y.floor() as i32) {
                    Mat::Lava | Mat::Molten | Mat::MoltenSteel => heat += 30,
                    Mat::Fire | Mat::Ember => heat += 3,
                    _ => {}
                }
            }
        }
        self.damage += heat;
        let left_the_world = self.p.x < -20.0 || self.p.x > W() as f32 + 20.0 || self.p.y < -400.0;
        if self.damage > 40 {
            self.fate = Fate::Exploded;
        } else if self.p.y > H() as f32 + 16.0 || left_the_world {
            self.fate = Fate::Dead;
        }
        if let Some(e) = exit {
            if (self.p - vec2(e.0 as f32, e.1 as f32)).length() < 14.0 {
                self.fate = Fate::Arrived;
            }
        }
    }
}

/// Two cars touching: a bump at walking pace pushes them apart; anything faster
/// wrecks both at once. Returns true if they exploded.
pub fn collide_cars(a: &mut Car, b: &mut Car) -> bool {
    if a.fate != Fate::Driving || b.fate != Fate::Driving {
        return false;
    }
    let (ra, rb) = (a.kind.model().hull_half * S + 3.0, b.kind.model().hull_half * S + 3.0);
    let d = b.p - a.p;
    let dist = d.length();
    if dist >= ra + rb {
        return false;
    }
    let rel = (a.v - b.v).length();
    if rel > 0.35 * S {
        a.fate = Fate::Exploded;
        b.fate = Fate::Exploded;
        return true;
    }
    // Gentle bump: separate and swap a little momentum.
    let n = if dist > 1e-3 { d / dist } else { vec2(1.0, 0.0) };
    let push = (ra + rb - dist) * 0.5;
    a.p -= n * push;
    b.p += n * push;
    for w in a.wheels.iter_mut() {
        w.pos -= n * push;
    }
    for w in b.wheels.iter_mut() {
        w.pos += n * push;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_world() -> World {
        let mut w = World::new_static();
        for x in 0..600 {
            for y in 240..250 {
                w.set(x, y, Mat::Stone);
            }
        }
        w
    }

    fn ramp_world(slope: f32) -> World {
        let mut w = flat_world();
        for x in 150..300 {
            let h = ((x - 150) as f32 * slope) as i32;
            for y in 240 - h..240 {
                w.set(x, y, Mat::Stone);
            }
        }
        for x in 300..600 {
            let h = (150.0 * slope) as i32;
            for y in 240 - h..240 {
                w.set(x, y, Mat::Stone);
            }
        }
        w
    }

    fn drive(kind: ModelKind, w: &World, start: (i32, i32), steps: usize) -> Car {
        let mut car = Car::new(kind, start);
        for _ in 0..steps {
            car.step(w, None);
        }
        car
    }

    /// A stretch of the road paved with sand instead of rock, at the same level.
    fn sand_world(_depth: i32) -> World {
        let mut w = flat_world();
        for x in 200..400 {
            for y in 240..250 {
                w.set(x, y, Mat::Sand);
            }
        }
        w
    }

    #[test]
    fn cars_drive_through_a_trunk_and_land_on_a_crown() {
        for kind in ModelKind::ALL {
            let mut w = flat_world();
            for y in 160..240 {
                for x in 200..209 {
                    w.set(x, y, Mat::Trunk);
                }
            }
            let mut car = Car::new(kind, (40, 216));
            for _ in 0..600 {
                car.step(&w, None);
                if car.p.x > 260.0 {
                    break;
                }
            }
            assert_eq!(car.fate, Fate::Driving, "{kind:?}");
            assert!(car.p.x > 260.0, "{kind:?} got through the trunk: {}", car.p.x);
            // A wide crown, the car dropped on it from above: it stays on top.
            let mut w = flat_world();
            for y in 150..170 {
                for x in 0..600 {
                    w.set(x, y, Mat::Leaf);
                }
            }
            let mut car = Car::new(kind, (100, 100));
            for _ in 0..300 {
                car.step(&w, None);
            }
            assert!(car.p.y < 150.0, "{kind:?} sank through the crown: {}", car.p.y);
            assert!(car.wheels.iter().any(|w| w.contact), "{kind:?} rests on the leaves");
        }
    }

    #[test]
    fn sand_slows_cars_and_gets_thrown_up() {
        for kind in ModelKind::ALL {
            let flat = flat_world();
            let sand = sand_world(6);
            let (mut a, mut b) = (Car::new(kind, (40, 216)), Car::new(kind, (40, 216)));
            let mut sprays = 0;
            let mut dig = 0.0f32;
            for _ in 0..500 {
                a.step(&flat, None);
                b.step(&sand, None);
                sprays += b.sprays.len();
                dig = dig.max(b.dig);
            }
            eprintln!("SAND {kind:?} flat x={:.0} sand x={:.0} sprays={sprays} dig={dig:.2} fate={:?}", a.p.x, b.p.x, b.fate);
            assert!(b.p.x < a.p.x, "{kind:?} not slowed by sand");
            assert!(sprays > 0, "{kind:?} threw nothing");
        }
    }

    #[test]
    fn weak_car_digs_into_soft_ground_and_recovers_on_rock() {
        let mut w = sand_world(0);
        for x in 400..600 {
            for y in 240..250 {
                w.set(x, y, Mat::Stone);
            }
        }
        let weak = Tuning { power: 0.02, ..Tuning::default() };
        let mut car = Car::with_tuning(ModelKind::Truck, (205, 216), weak);
        let mut dug = 0.0f32;
        for _ in 0..600 {
            car.step(&w, None);
            dug = dug.max(car.dig);
        }
        assert!(dug > 0.1, "never bogged down: {dug}");
        let mut rock = flat_world();
        rock.set(0, 0, Mat::Empty);
        let mut c2 = Car::with_tuning(ModelKind::Truck, (205, 216), weak);
        c2.dig = 0.9;
        for _ in 0..200 {
            c2.step(&rock, None);
        }
        assert!(c2.dig < 0.2, "dig did not recover on rock: {}", c2.dig);
    }

    #[test]
    fn survey_grass_country() {
        for seed in 1..4 {
            for kind in ModelKind::ALL {
                let mut w = World::new_static();
                let (e, x) = w.generate_biome(seed, crate::world::Biome::Countryside);
                let mut car = Car::new(kind, e);
                let mut dug = 0.0f32;
                for t in 0..6000 {
                    car.step(&w, Some(x));
                    dug = dug.max(car.dig);
                    if car.fate != Fate::Driving {
                        eprintln!("GRASS seed {seed} {kind:?} fate {:?} t={t} x={:.0}/{} dig={dug:.2}", car.fate, car.p.x, x.0);
                        break;
                    }
                }
                if car.fate == Fate::Driving {
                    eprintln!("GRASS seed {seed} {kind:?} still driving x={:.0}/{} dig={dug:.2}", car.p.x, x.0);
                }
            }
        }
    }

    #[test]
    fn dirt_and_grass_are_soft_like_sand() {
        let mut w = flat_world();
        for x in 200..400 {
            for y in 240..250 {
                w.set(x, y, Mat::Dirt);
            }
            w.set(x, 239, Mat::Grass);
        }
        let mut car = Car::new(ModelKind::Buggy, (40, 216));
        let mut sprays = 0;
        for _ in 0..500 {
            car.step(&w, None);
            sprays += car.sprays.len();
        }
        assert!(sprays > 0);
    }

    #[test]
    fn every_model_rides_flat_and_stays_upright() {
        let w = flat_world();
        for kind in ModelKind::ALL {
            let car = drive(kind, &w, (40, 216), 300);
            assert_eq!(car.fate, Fate::Driving, "{kind:?}");
            assert!(car.angle.abs() < 0.3, "{kind:?} angle {}", car.angle);
            assert!(car.p.x > 80.0, "{kind:?} did not move: {}", car.p.x);
        }
    }

    #[test]
    fn top_speeds_are_ordered() {
        let w = flat_world();
        let mut speeds = vec![];
        for kind in [ModelKind::Race, ModelKind::Buggy, ModelKind::Offroad, ModelKind::Truck] {
            let mut car = Car::new(kind, (20, 216));
            let mut best = 0.0f32;
            for t in 0..400 {
                car.step(&w, None);
                if t > 150 {
                    best = best.max(car.speed());
                }
                if car.p.x > 560.0 {
                    break;
                }
            }
            speeds.push((kind, best));
        }
        eprintln!("{speeds:?}");
        assert!(speeds[0].1 > speeds[1].1 && speeds[1].1 > speeds[2].1 && speeds[2].1 > speeds[3].1);
    }

    #[test]
    fn four_by_four_climbs_a_ramp() {
        let w = ramp_world(0.45);
        let car = drive(ModelKind::Offroad, &w, (40, 216), 1500);
        assert_eq!(car.fate, Fate::Driving);
        assert!(car.p.x > 320.0, "stuck at {}", car.p.x);
    }

    #[test]
    fn explodes_when_dropped_from_high() {
        let mut w = World::new_static();
        for x in 0..300 {
            for y in 340..350 {
                w.set(x, y, Mat::Stone);
            }
        }
        let mut car = Car::new(ModelKind::Buggy, (100, 20));
        for _ in 0..600 {
            car.step(&w, None);
        }
        assert_eq!(car.fate, Fate::Exploded);
    }

    #[test]
    fn survives_small_drop_thanks_to_suspension() {
        let w = flat_world();
        let mut car = Car::new(ModelKind::Buggy, (100, 200));
        for _ in 0..300 {
            car.step(&w, None);
        }
        assert_eq!(car.fate, Fate::Driving);
    }

    #[test]
    fn reaches_the_exit() {
        let w = flat_world();
        let mut car = Car::new(ModelKind::Buggy, (20, 216));
        for _ in 0..2000 {
            car.step(&w, Some((500, 225)));
        }
        assert_eq!(car.fate, Fate::Arrived);
    }

    #[test]
    fn car_smashes_through_a_glass_wall() {
        let mut w = flat_world();
        for y in 200..240 {
            for x in 300..304 {
                w.set(x, y, Mat::Glass);
            }
        }
        let mut car = Car::new(ModelKind::Buggy, (40, 216));
        let mut smashed = 0;
        for _ in 0..2500 {
            car.step(&w, None);
            for &(x, y) in &car.smashed {
                smashed += 1;
                w.shatter(x, y);
            }
            if car.p.x > 360.0 {
                break;
            }
        }
        assert!(smashed > 0, "never touched the glass");
        assert!(car.p.x > 360.0, "stuck behind the glass at {}", car.p.x);
    }

    #[test]
    fn tuning_changes_how_the_car_behaves() {
        let w = flat_world();
        let run = |t: Tuning| {
            let mut car = Car::with_tuning(ModelKind::Buggy, (20, 216), t);
            let mut best = 0.0f32;
            for k in 0..500 {
                car.step(&w, None);
                if k > 200 {
                    best = best.max(car.speed());
                }
            }
            (best, car.fate)
        };
        let stock = run(Tuning::default());
        let fast = run(Tuning { speed: 1.8, power: 1.8, ..Tuning::default() });
        let slow = run(Tuning { speed: 0.5, ..Tuning::default() });
        assert!(fast.0 > stock.0 * 1.2, "fast {:?} stock {:?}", fast, stock);
        assert!(slow.0 < stock.0 * 0.8, "slow {:?} stock {:?}", slow, stock);
        let big = Car::with_tuning(ModelKind::Buggy, (0, 0), Tuning { wheel: 1.5, ..Tuning::default() });
        assert!(big.wheel_r(0) > Car::new(ModelKind::Buggy, (0, 0)).wheel_r(0) * 1.4);
    }

    #[test]
    fn head_on_cars_explode_at_once() {
        let w = flat_world();
        let mut a = Car::new(ModelKind::Buggy, (100, 216));
        let mut b = Car::new(ModelKind::Truck, (260, 216));
        b.dir = -1.0;
        let mut boom = false;
        for t in 0..1200 {
            a.step(&w, None);
            b.step(&w, None);
            if collide_cars(&mut a, &mut b) {
                boom = true;
                assert!(t < 1100, "took far too long");
                break;
            }
        }
        assert!(boom, "cars passed through each other");
        assert_eq!(a.fate, Fate::Exploded);
        assert_eq!(b.fate, Fate::Exploded);
    }

    #[test]
    fn slow_bump_does_not_explode() {
        let w = flat_world();
        let mut a = Car::new(ModelKind::Buggy, (100, 216));
        let mut b = Car::new(ModelKind::Buggy, (112, 216));
        for _ in 0..30 {
            a.step(&w, None);
            b.step(&w, None);
        }
        a.v = Vec2::ZERO;
        b.v = Vec2::ZERO;
        let exploded = collide_cars(&mut a, &mut b);
        assert!(!exploded);
        assert_eq!(a.fate, Fate::Driving);
    }

    #[test]
    fn turns_around_at_wall() {
        let mut w = flat_world();
        for y in 150..240 {
            for x in 300..310 {
                w.set(x, y, Mat::Stone);
            }
        }
        let mut car = Car::new(ModelKind::Buggy, (40, 216));
        for _ in 0..1500 {
            car.step(&w, None);
        }
        assert!(car.p.x < 300.0);
    }

    #[test]
    fn a_car_in_fire_or_lava_explodes() {
        for (burning, label) in [(Mat::Fire, "fire"), (Mat::Lava, "lava")] {
            let mut w = flat_world();
            let mut car = Car::new(ModelKind::Buggy, (100, 216));
            for _ in 0..40 {
                car.step(&w, None);
            }
            // The ground around the car catches fire (or turns to lava).
            for x in 80..130 {
                for y in 215..240 {
                    if w.raw(x, y) == Mat::Empty {
                        w.set(x, y, burning);
                    }
                }
            }
            let mut steps = 0;
            while car.fate == Fate::Driving && steps < 200 {
                car.step(&w, None);
                steps += 1;
            }
            assert_eq!(car.fate, Fate::Exploded, "a car in {label} should explode");
            assert!(steps < 30, "it should not take long: {steps} steps in {label}");
        }
    }

    /// Furthest x reached on a course with a step of `height` cells or a ramp.
    fn reach(kind: ModelKind, step: f32, slope: f32) -> f32 {
        let mut w = flat_world();
        if step > 0.0 {
            for x in 200..500 {
                for y in (240 - step as i32)..240 {
                    w.set(x, y, Mat::Stone);
                }
            }
        }
        if slope > 0.0 {
            for x in 200..320 {
                let h = ((x - 200) as f32 * slope) as i32;
                for y in 240 - h..240 {
                    w.set(x, y, Mat::Stone);
                }
            }
            for x in 320..500 {
                let h = (120.0 * slope) as i32;
                for y in 240 - h..240 {
                    w.set(x, y, Mat::Stone);
                }
            }
        }
        let mut car = Car::new(kind, (40, 216));
        let mut best = 0.0f32;
        for _ in 0..1800 {
            car.step(&w, None);
            best = best.max(car.p.x);
            if car.fate != Fate::Driving {
                break;
            }
        }
        best
    }

    #[test]
    fn survey() {
        for kind in ModelKind::ALL {
            let steps: Vec<String> = [4.0, 6.0, 8.0, 10.0, 14.0].iter().map(|&h| format!("{:.0}", reach(kind, h, 0.0))).collect();
            let ramps: Vec<String> = [0.3, 0.5, 0.7, 0.9].iter().map(|&sl| format!("{:.0}", reach(kind, 0.0, sl))).collect();
            eprintln!("SURVEY {kind:?} steps(4,6,8,10,14)={steps:?} ramps(.3,.5,.7,.9)={ramps:?}");
        }
    }

    #[test]
    fn leaving_the_world_sideways_loses_the_car() {
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            for y in 240..250 {
                w.set(x, y, Mat::Stone);
            }
        }
        let mut car = Car::new(ModelKind::Buggy, (60, 216));
        car.dir = -1.0;
        for _ in 0..1500 {
            car.step(&w, None);
            if car.fate != Fate::Driving {
                break;
            }
        }
        assert_eq!(car.fate, Fate::Dead, "car should be lost past the left edge, x = {}", car.p.x);
    }

    #[test]
    fn falls_out_of_empty_world() {
        let w = World::new();
        let mut car = Car::new(ModelKind::Buggy, (20, 110));
        for _ in 0..3000 {
            car.step(&w, None);
        }
        assert_eq!(car.fate, Fate::Dead);
    }
}
