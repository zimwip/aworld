//! Cars: a rigid chassis on spring/damper suspensions, wheels with their own
//! spin driven by an engine torque curve, and traction limited by grip.
//! Units are grid cells and simulation steps.

use crate::world::{Mat, World, H};
use macroquad::prelude::{vec2, Vec2};

const GRAVITY: f32 = 0.12;
const WHEEL_INERTIA: f32 = 8.0;
const SLIP_GAIN: f32 = 2.0;
const SAMPLES: usize = 24;

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

pub struct Car {
    pub kind: ModelKind,
    pub p: Vec2,
    pub v: Vec2,
    pub angle: f32,
    pub omega: f32,
    pub wheels: Vec<WheelState>,
    pub dir: f32,
    pub fate: Fate,
    damage: u32,
    ticks: u32,
    last_x: f32,
    progress: f32,
    grounded: u32,
    upside_down: u32,
}

fn rot(a: f32, v: Vec2) -> Vec2 {
    let (s, c) = a.sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// Outward normal of the hard cells touching a circle, if any.
fn probe(world: &World, c: Vec2, radius: f32) -> Option<Vec2> {
    let mut sum = Vec2::ZERO;
    for k in 0..SAMPLES {
        let a = k as f32 / SAMPLES as f32 * std::f32::consts::TAU;
        let d = vec2(a.cos(), a.sin());
        let p = c + d * radius;
        if world.hard_for_car(p.x.floor() as i32, p.y.floor() as i32) {
            sum -= d;
        }
    }
    (sum.length() > 1e-3).then(|| sum.normalize())
}

fn overlaps(world: &World, c: Vec2, radius: f32) -> bool {
    for k in 0..SAMPLES {
        let a = k as f32 / SAMPLES as f32 * std::f32::consts::TAU;
        let p = c + vec2(a.cos(), a.sin()) * radius;
        if world.hard_for_car(p.x.floor() as i32, p.y.floor() as i32) {
            return true;
        }
    }
    world.hard_for_car(c.x.floor() as i32, c.y.floor() as i32)
}

impl Car {
    pub fn new(kind: ModelKind, at: (i32, i32)) -> Self {
        let m = kind.model();
        let tallest = m.wheels.iter().fold(0.0f32, |a, w| a.max(w.r));
        let p = vec2(at.0 as f32, at.1 as f32 + 14.0 - m.anchor_y - m.travel - tallest);
        let wheels = m
            .wheels
            .iter()
            .map(|w| WheelState {
                pos: p + vec2(w.x, m.anchor_y + m.travel * 0.5),
                ext: m.travel * 0.5,
                spin: 0.0,
                roll: 0.0,
                contact: false,
            })
            .collect();
        Car {
            kind,
            p,
            v: Vec2::ZERO,
            angle: 0.0,
            omega: 0.0,
            wheels,
            dir: 1.0,
            fate: Fate::Driving,
            damage: 0,
            ticks: 0,
            last_x: p.x,
            progress: 0.0,
            grounded: 0,
            upside_down: 0,
        }
    }

    pub fn center(&self) -> Vec2 {
        self.p
    }

    pub fn anchor(&self, i: usize) -> Vec2 {
        let m = self.kind.model();
        self.p + rot(self.angle, vec2(m.wheels[i].x, m.anchor_y))
    }

    /// Forward speed along the direction of travel, cells per step.
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
        let m = self.kind.model();
        let down = rot(self.angle, vec2(0.0, 1.0));
        let mut acc = vec2(0.0, GRAVITY);
        let mut torque = 0.0f32;
        let mut crashed = false;
        let mut touching = 0;
        let mut wet = false;
        let ground_speed = self.v.x * self.dir;

        for i in 0..self.wheels.len() {
            let r = m.wheels[i].r;
            let anchor = self.anchor(i);
            let at = |e: f32| anchor + down * e;

            // Find how far the wheel can extend before touching the ground.
            let mut ext = m.travel;
            let mut depth = 0.0;
            let mut contact = false;
            if overlaps(world, at(m.travel), r) {
                contact = true;
                let mut e = m.travel;
                while e >= 0.0 && overlaps(world, at(e), r) {
                    e -= 0.5;
                }
                if e < 0.0 {
                    let mut d = 0.0;
                    while d < r * 2.0 && overlaps(world, at(-d), r) {
                        d += 0.5;
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
                let n = probe(world, pos, r + 1.0).unwrap_or(-down);
                let ra = anchor - self.p;
                let va = self.v + vec2(-ra.y, ra.x) * self.omega;
                let vn = va.dot(n);
                if depth > 0.0 && vn < -m.crash_speed {
                    crashed = true;
                }
                let comp = m.travel - ext + depth * 2.0;
                let normal_force = (m.stiffness * comp - m.damping * vn).max(0.0);
                let f = n * normal_force;
                acc += f;
                torque += cross(ra, f);

                let t = vec2(-n.y, n.x);
                // Thrust acts at hub height: keeps wheelies tame.
                let rc = pos - self.p;
                let vhub = self.v + vec2(-(pos - self.p).y, (pos - self.p).x) * self.omega;
                let slip = self.wheels[i].spin * r - vhub.dot(t);
                ft = (SLIP_GAIN * slip).clamp(-m.grip * normal_force, m.grip * normal_force);
                let tf = t * ft;
                acc += tf;
                torque += cross(rc, tf);
            }

            // Engine torque curve, then reaction from the ground.
            let mut te = 0.0;
            if self.is_driven(i) {
                let w_max = m.top_speed / r;
                let forward = (self.dir * self.wheels[i].spin).max(ground_speed / r);
                te = self.dir * m.torque * (1.0 - forward / w_max).clamp(-0.2, 1.5);
            }
            let w = &mut self.wheels[i];
            w.spin += (te - ft * r) / WHEEL_INERTIA;
            w.spin *= 0.9995;
            w.roll += w.spin;
        }

        if wet {
            self.v *= 0.985;
            for w in &mut self.wheels {
                w.spin *= 0.97;
            }
        }

        self.v += acc;
        self.omega += torque / m.inertia;
        self.omega *= 0.995;
        self.v *= 0.9998;
        self.p += self.v;
        self.angle += self.omega;

        // Chassis hull against the terrain.
        let hull = [
            vec2(-m.hull_half, m.hull_top),
            vec2(0.0, m.hull_top),
            vec2(m.hull_half, m.hull_top),
            vec2(-m.hull_half, (m.hull_top + m.hull_bottom) * 0.5),
            vec2(m.hull_half, (m.hull_top + m.hull_bottom) * 0.5),
            vec2(-m.hull_half * 0.55, m.hull_bottom),
            vec2(m.hull_half * 0.55, m.hull_bottom),
            vec2(0.0, m.hull_bottom),
        ];
        for _ in 0..8 {
            let mut sum = Vec2::ZERO;
            for h in hull {
                let q = self.p + rot(self.angle, h);
                if world.hard_for_car(q.x.floor() as i32, q.y.floor() as i32) {
                    sum += (q - self.p).normalize_or_zero();
                }
            }
            if sum.length() < 1e-3 {
                break;
            }
            let n = -sum.normalize();
            self.p += n * 0.6;
            let vn = self.v.dot(n);
            if vn < -m.crash_speed {
                crashed = true;
            }
            if vn < 0.0 {
                self.v -= n * vn;
                self.v *= 0.99;
                self.omega *= 0.9;
            }
        }
        if crashed {
            self.fate = Fate::Exploded;
            return;
        }

        // Upside down for too long counts as a wreck.
        if self.angle.cos() < -0.2 {
            self.upside_down += 1;
            if self.upside_down > 180 {
                self.fate = Fate::Exploded;
                return;
            }
        } else {
            self.upside_down = 0;
        }

        // Turn around when blocked (lemmings style).
        self.progress += (self.p.x - self.last_x) * self.dir;
        self.last_x = self.p.x;
        if touching > 0 {
            self.grounded += 1;
        }
        if self.ticks % 60 == 0 {
            if self.grounded >= 30 && self.progress < 3.0 {
                self.dir = -self.dir;
            }
            self.progress = 0.0;
            self.grounded = 0;
        }

        // Fire hurts.
        let mut burning = false;
        let mut pts = vec![self.p];
        pts.extend(self.wheels.iter().map(|w| w.pos));
        for p in pts {
            for k in 0..8 {
                let a = k as f32 / 8.0 * std::f32::consts::TAU;
                let q = p + vec2(a.cos(), a.sin()) * 10.0;
                if matches!(world.at(q.x.floor() as i32, q.y.floor() as i32), Mat::Fire | Mat::Ember | Mat::Lava) {
                    burning = true;
                }
            }
        }
        if burning {
            self.damage += 1;
        }
        if self.damage > 90 || self.p.y > H as f32 + 24.0 {
            self.fate = Fate::Dead;
        }
        if let Some(e) = exit {
            if (self.p - vec2(e.0 as f32, e.1 as f32)).length() < 18.0 {
                self.fate = Fate::Arrived;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_world() -> World {
        let mut w = World::new();
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

    #[test]
    fn every_model_rides_flat_and_stays_upright() {
        let w = flat_world();
        for kind in ModelKind::ALL {
            let car = drive(kind, &w, (40, 224), 300);
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
            let mut car = Car::new(kind, (20, 224));
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
        let car = drive(ModelKind::Offroad, &w, (40, 224), 900);
        assert_eq!(car.fate, Fate::Driving);
        assert!(car.p.x > 320.0, "stuck at {}", car.p.x);
    }

    #[test]
    fn explodes_when_dropped_from_high() {
        let mut w = World::new();
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
        let mut car = Car::new(ModelKind::Buggy, (20, 224));
        for _ in 0..2000 {
            car.step(&w, Some((500, 225)));
        }
        assert_eq!(car.fate, Fate::Arrived);
    }

    #[test]
    fn turns_around_at_wall() {
        let mut w = flat_world();
        for y in 150..240 {
            for x in 300..310 {
                w.set(x, y, Mat::Stone);
            }
        }
        let mut car = Car::new(ModelKind::Buggy, (40, 224));
        for _ in 0..1500 {
            car.step(&w, None);
        }
        assert!(car.p.x < 300.0);
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
