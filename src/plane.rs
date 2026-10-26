//! Just for fun: now and then a big, smiling whale-shaped cargo plane (a stylised Beluga XL) drifts
//! across the sky. It simply follows the air: it flies level through calm weather, and only the
//! vortices of the wind field spin it round into loops and tumbles. If it touches anything it explodes.

use crate::render::View;
use crate::world::{Kind, World, H, W};
use macroquad::prelude::*;

/// Length of the plane, in world cells.
const LENGTH: f32 = 70.0;
/// Seconds between planes, at random.
const GAP: (f32, f32) = (110.0, 330.0);
/// The longest a plane stays around.
const MAX_AGE: f32 = 90.0;
/// Radians of turning per unit of air spin, and how much of the air's speed it picks up.
const SPIN: f32 = 0.8;
const COUPLE: f32 = 0.6;

#[derive(Clone, Debug)]
pub struct Plane {
    pub pos: Vec2,
    /// Which way the nose points (radians; 0 is along its flight direction).
    pub heading: f32,
    /// +1 flying to the right, -1 to the left.
    pub dir: f32,
    /// Turned over on its roll axis (for the barrel roll): 0 upright.
    pub roll: f32,
    pub age: f32,
    /// Total turning so far (radians), to see how many loops it has done.
    pub turned: f32,
}

#[derive(Clone, Default)]
pub struct Planes {
    pub plane: Option<Plane>,
    cooldown: f32,
    rng: u32,
}

impl Planes {
    pub fn new() -> Self {
        Planes { plane: None, cooldown: 40.0, rng: 0x5eed_b01a }
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    fn between(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.rand()
    }

    /// A plane comes in from the side the wind blows from.
    pub fn summon(&mut self, world: &World) {
        if self.plane.is_some() {
            return;
        }
        let dir = if world.wind >= 0.0 { 1.0 } else { -1.0 };
        let y = self.between(50.0, 190.0);
        let x = if dir > 0.0 { -LENGTH } else { W() as f32 + LENGTH };
        self.plane = Some(Plane { pos: vec2(x, y), heading: 0.0, dir, roll: 0.0, age: 0.0, turned: 0.0 });
    }

    /// `dt` in seconds. Planes come at random, rarely.
    /// Returns where it crashed, if it touched anything this tick: it explodes on contact.
    pub fn update(&mut self, dt: f32, world: &World) -> Option<Vec2> {
        if self.plane.is_none() {
            self.cooldown -= dt;
            if self.cooldown <= 0.0 {
                self.summon(world);
                self.cooldown = self.between(GAP.0, GAP.1);
            }
            return None;
        }
        let ticks = dt * 60.0;
        let mut plane = self.plane.take().unwrap();
        plane.age += dt;
        let at = |dx: f32, dy: f32| world.flow.sample((plane.pos.x + dx) as i32, (plane.pos.y + dy).clamp(0.0, H() as f32 - 1.0) as i32);
        let (fx, fy) = at(0.0, 0.0);
        // The air's spin where it is (flow units per cell): this alone turns the plane round.
        let d = 12.0;
        let curl = ((at(d, 0.0).1 - at(-d, 0.0).1) - (at(0.0, d).0 - at(0.0, -d).0)) / (2.0 * d);
        let turn = curl * SPIN * plane.dir * ticks;
        // Out of any whirl it slowly levels out again.
        let level = -0.012 * plane.heading.sin() * ticks;
        plane.heading = (plane.heading + turn + level + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        plane.turned += turn.abs();
        // It flies where its nose points and the air carries it along.
        let nose = vec2(plane.heading.cos() * plane.dir, plane.heading.sin());
        let carried = vec2((fx * COUPLE).clamp(-1.2, 1.2), (fy * COUPLE).clamp(-0.9, 0.9));
        let lazy = vec2(0.0, (plane.age * 1.3).sin() * 0.12);
        // It stays in the sky: well above the ground, well below the top of the picture.
        let keep = if plane.pos.y < 30.0 { 0.6 } else if plane.pos.y > 250.0 { -0.6 } else { 0.0 };
        plane.pos += (nose * 0.9 + carried + lazy + vec2(0.0, keep)) * ticks;
        // Touching anything (ground, water, a block, a tree...) sets it off.
        let nose_dir = vec2(plane.heading.cos() * plane.dir, plane.heading.sin());
        let side = vec2(-nose_dir.y, nose_dir.x);
        let hit = [(0.42, 0.0), (0.2, 0.0), (0.0, 0.0), (-0.3, 0.0), (0.0, 0.12), (0.0, -0.18)].iter().any(|&(a, b)| {
            let q = plane.pos + nose_dir * (a * LENGTH) + side * (b * LENGTH);
            let (x, y) = (q.x as i32, q.y as i32);
            World::inb(x, y) && !matches!(world.at(x, y).kind(), Kind::Empty | Kind::Gas)
        });
        if hit {
            return Some(plane.pos);
        }
        // Gone once it has left the world, or has stayed too long.
        let out = plane.pos.x < -LENGTH * 2.0 || plane.pos.x > W() as f32 + LENGTH * 2.0 || plane.pos.y < -LENGTH * 2.0 || plane.pos.y > H() as f32;
        if !(out && plane.age > 3.0 || plane.age > MAX_AGE) {
            self.plane = Some(plane);
        }
        None
    }

    pub fn draw(&self, view: View, time: f32) {
        if let Some(p) = &self.plane {
            draw_beluga(view.to_screen(p.pos), LENGTH * view.s, p.heading * p.dir, p.dir, p.roll, time);
        }
    }
}

/// A stylised Beluga XL, nose to the right when `dir` is 1: white whale-shaped fuselage with its big
/// hump, a blue belly, a smiling face, wing, engine and tail fin. `roll` flips it over (barrel roll).
pub fn draw_beluga(c: Vec2, len: f32, angle: f32, dir: f32, roll: f32, time: f32) {
    let flip = roll.cos();
    let (sa, ca) = angle.sin_cos();
    // Local (-0.5..0.5 along the plane, y down) to screen.
    let tf = |x: f32, y: f32| -> Vec2 {
        let (lx, ly) = (x * dir * len, y * len * flip);
        c + vec2(lx * ca - ly * sa, lx * sa + ly * ca)
    };
    let white = Color::from_rgba(246, 249, 255, 255);
    let blue = Color::from_rgba(38, 98, 190, 255);
    let deep = Color::from_rgba(20, 52, 120, 255);
    let fan = |pts: &[(f32, f32)], centre: (f32, f32), col: Color| {
        let mid = tf(centre.0, centre.1);
        for k in 0..pts.len() {
            let (a, b) = (tf(pts[k].0, pts[k].1), tf(pts[(k + 1) % pts.len()].0, pts[(k + 1) % pts.len()].1));
            draw_triangle(mid, a, b, col);
        }
    };
    let quad = |a: (f32, f32), b: (f32, f32), c2: (f32, f32), d: (f32, f32), col: Color| {
        let (pa, pb, pc, pd) = (tf(a.0, a.1), tf(b.0, b.1), tf(c2.0, c2.1), tf(d.0, d.1));
        draw_triangle(pa, pb, pc, col);
        draw_triangle(pa, pc, pd, col);
    };
    // Tail fin and the far wing, behind the body.
    fan(&[(-0.38, -0.10), (-0.52, -0.37), (-0.31, -0.10)], (-0.40, -0.18), blue);
    quad((0.02, 0.05), (-0.10, 0.05), (-0.22, 0.20), (-0.12, 0.20), Color::from_rgba(120, 150, 205, 255));
    // The body.
    let body: [(f32, f32); 17] = [
        (0.50, 0.04), (0.47, -0.07), (0.40, -0.16), (0.28, -0.23), (0.12, -0.26), (-0.04, -0.22), (-0.16, -0.14), (-0.30, -0.11),
        (-0.44, -0.10), (-0.50, -0.07), (-0.47, 0.02), (-0.38, 0.07), (-0.20, 0.11), (0.0, 0.13), (0.20, 0.13), (0.38, 0.11), (0.47, 0.09),
    ];
    fan(&body, (0.0, -0.05), white);
    // The blue belly: a band along the lower edge.
    for k in 10..16 {
        let (a, b) = (body[k], body[k + 1]);
        quad(a, b, (b.0, b.1 - 0.075), (a.0, a.1 - 0.075), blue);
    }
    // Outline.
    for k in 0..body.len() {
        let (a, b) = (tf(body[k].0, body[k].1), tf(body[(k + 1) % body.len()].0, body[(k + 1) % body.len()].1));
        draw_line(a.x, a.y, b.x, b.y, (len * 0.012).max(1.2), deep);
    }
    // Cockpit windows on the nose.
    for k in 0..3 {
        let p = tf(0.43 - k as f32 * 0.035, -0.12 + k as f32 * 0.012);
        draw_circle(p.x, p.y, len * 0.012, deep);
    }
    // The wing in front, and an engine under it.
    quad((0.10, 0.06), (-0.04, 0.06), (-0.20, 0.27), (-0.07, 0.27), Color::from_rgba(96, 128, 196, 255));
    let eng = tf(-0.02, 0.22);
    draw_circle(eng.x, eng.y, len * 0.034, Color::from_rgba(120, 126, 142, 255));
    draw_circle(eng.x, eng.y, len * 0.016, deep);
    // The face: a big friendly eye, a blush and a wide smile.
    let eye = tf(0.345, -0.065);
    draw_circle(eye.x, eye.y, len * 0.034, Color::from_rgba(18, 22, 40, 255));
    let glint = tf(0.355, -0.075);
    draw_circle(glint.x, glint.y, len * 0.011, WHITE);
    let cheek = tf(0.29, 0.0);
    draw_circle(cheek.x, cheek.y, len * 0.03, Color::new(1.0, 0.55, 0.6, 0.55));
    let mut prev = tf(0.485, 0.025);
    for k in 1..=8 {
        let t = k as f32 / 8.0;
        let p = tf(0.485 - 0.23 * t, 0.025 + 0.055 * (t * std::f32::consts::PI).sin());
        draw_line(prev.x, prev.y, p.x, p.y, (len * 0.014).max(1.4), deep);
        prev = p;
    }
    // A little wobble of the wing tip, for fun.
    let _ = time;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Mat;

    fn run(planes: &mut Planes, world: &World, seconds: f32) {
        let mut t = 0.0;
        while t < seconds {
            planes.update(1.0 / 30.0, world);
            t += 1.0 / 30.0;
        }
    }

    #[test]
    fn planes_are_rare_and_one_at_a_time() {
        let world = World::new_static();
        let mut planes = Planes::new();
        let mut seen = 0;
        let mut was = false;
        let mut at_once = 0;
        for _ in 0..(30 * 60 * 30) {
            planes.update(1.0 / 30.0, &world);
            let now = planes.plane.is_some();
            if now && !was {
                seen += 1;
            }
            was = now;
            at_once = at_once.max(planes.plane.iter().count());
        }
        // Thirty minutes: a handful of planes, never more than one in the sky.
        assert!((4..=20).contains(&seen), "a plane now and then: {seen} in half an hour");
        assert_eq!(at_once, 1);
    }

    #[test]
    fn in_calm_weather_it_crosses_the_screen_flying_level() {
        let mut world = World::new_static();
        world.flow.pin_uniform(0.0, 0.0);
        let mut planes = Planes::new();
        planes.summon(&world);
        let (mut crossed, mut turned) = (false, 0.0f32);
        for _ in 0..(30 * 90) {
            if let Some(p) = &planes.plane {
                crossed |= p.pos.x > 0.0 && p.pos.x < W() as f32;
                turned = turned.max(p.turned);
                assert!(p.heading.abs() < 0.2, "no loops without a vortex: {}", p.heading);
            }
            planes.update(1.0 / 30.0, &world);
        }
        assert!(crossed && turned < 0.3, "it flew over the picture without a loop");
        assert!(planes.plane.is_none(), "and then left");
    }

    #[test]
    fn a_vortex_spins_it_round_and_otherwise_it_flies_straight() {
        let mut best = 0.0f32;
        for gamma in [0.12f32, -0.12] {
            let mut world = World::new_static();
            world.flow.spawn_vortex(40.0, 15.0, 5.0, gamma);
            let mut planes = Planes::new();
            planes.plane = Some(Plane { pos: vec2(250.0, 125.0), heading: 0.0, dir: 1.0, roll: 0.0, age: 0.0, turned: 0.0 });
            for k in 0..(30 * 12) {
                for _ in 0..2 {
                    world.flow.step(0.0, 1);
                }
                planes.update(1.0 / 30.0, &world);
                if let Some(p) = &planes.plane {
                    best = best.max(p.turned);
                }
                let _ = k;
            }
        }
        eprintln!("PLANE turned in a vortex: {best}");
        assert!(best > 5.0, "a vortex turns it a good way round: {best}");
    }

    #[test]
    fn the_wind_carries_it_the_way_it_blows() {
        let fly = |flow: f32| {
            let mut world = World::new_static();
            world.flow.pin_uniform(flow, 0.0);
            let mut planes = Planes::new();
            planes.plane = Some(Plane { pos: vec2(300.0, 100.0), heading: 0.0, dir: 1.0, roll: 0.0, age: 0.0, turned: 0.0 });
            run(&mut planes, &world, 5.0);
            planes.plane.as_ref().unwrap().pos.x
        };
        let (head, calm, tail) = (fly(-0.5), fly(0.0), fly(0.5));
        assert!(head < calm && calm < tail, "wind nudges it: {head} {calm} {tail}");
        assert!(head > 300.0, "but even against the wind it keeps making headway: {head}");
    }

    #[test]
    fn it_explodes_when_it_touches_something() {
        let mut world = World::new_static();
        for y in 60..140 {
            for x in 480..500 {
                world.set(x, y, Mat::Stone);
            }
        }
        let mut planes = Planes::new();
        planes.plane = Some(Plane { pos: vec2(300.0, 100.0), heading: 0.0, dir: 1.0, roll: 0.0, age: 0.0, turned: 0.0 });
        let mut boom = None;
        for _ in 0..(30 * 20) {
            if let Some(at) = planes.update(1.0 / 30.0, &world) {
                boom = Some(at);
                break;
            }
        }
        let at = boom.expect("it hit the wall");
        assert!(at.x > 400.0 && at.x < 500.0, "it blew up at the wall: {at:?}");
        assert!(planes.plane.is_none(), "and is gone");
        let sky = World::new_static();
        planes.summon(&sky);
        for _ in 0..(30 * 60) {
            assert!(planes.update(1.0 / 30.0, &sky).is_none());
        }
    }
}
