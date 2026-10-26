//! Just for fun: when a big fire breaks out, a fire engine comes in from the side of the world,
//! siren going, drives along the ground up to the fire and sprays it with real water from the
//! monitor on its roof until the fire is out (or its tank is empty), then drives away.
//! The water it throws is the world's own water: it puts flames out, turns to steam on hot things,
//! makes the embers into ash and runs off into the ground like any other water.

use crate::render::View;
use crate::world::{Kind, Mat, World, H, W};
use macroquad::prelude::*;

/// Length and height of the engine, in world cells.
pub const LENGTH: f32 = 36.0;
const HEIGHT: f32 = 15.0;
/// A fire this big (flames and embers within a few zones) calls the firemen.
pub const BIG_FIRE: u32 = 160;
/// It stops this far from the fire to spray it, and can reach this far if the road is cut.
const STOP_AT: f32 = 85.0;
const REACH: f32 = 220.0;
/// Driving speed, cells per step.
const SPEED: f32 = 0.8;
/// Water in the tank, in cells, and drops thrown per step.
pub const TANK: u32 = 3600;
const JET: u32 = 3;
/// Downward pull on the drops, cells per step squared.
const G: f32 = 0.05;
/// Steps between two looks for a big fire, and before another engine may come.
const LOOK_EVERY: u32 = 60;
const REST: u32 = 60 * 25;
/// Side of the zones the fire is counted in.
const ZONE: i32 = 24;
/// Steps of flamethrower on the engine before its fuel tank goes up.
pub const BURN: u32 = 45;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// Driving in towards the fire.
    Coming,
    /// Parked, hosing the fire down.
    Spraying,
    /// Going back the way it came.
    Leaving,
}

#[derive(Clone, Debug)]
pub struct Truck {
    /// Middle of the engine along the road, and the row of the ground under it.
    pub x: f32,
    pub y: f32,
    /// +1 driving right, -1 left.
    pub dir: f32,
    pub state: State,
    pub tank: u32,
    /// The fire it came for, and the flame it is aiming at right now.
    pub fire: Vec2,
    pub aim: Vec2,
    /// Steps since it came.
    pub age: u32,
    /// Tilt of the body on a slope (radians).
    pub tilt: f32,
    /// How hot the flamethrower made it, up to `BURN` (then it blows up); it cools slowly.
    pub heat: u32,
}

impl Truck {
    /// Whether a disc of radius `r` around `p` touches the engine (body, cab or ladder).
    pub fn touches(&self, p: Vec2, r: f32) -> bool {
        let d = Vec2::from_angle(-self.tilt).rotate(p - vec2(self.x, self.y));
        let (lx, ly) = (d.x * self.dir, -d.y);
        lx.abs() <= LENGTH * 0.5 + 2.0 + r && ly >= -r && ly <= HEIGHT + 2.0 + r
    }
}

#[derive(Clone, Debug)]
pub struct Drop {
    pub pos: Vec2,
    pub vel: Vec2,
}

/// What happened this step, for the sound and the messages.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum News {
    /// A fire engine is on its way (and its siren sounds now and then while it drives).
    Siren,
    Arrived,
    PutOut,
    TankEmpty,
    Blocked,
}

#[derive(Clone, Default)]
pub struct FireTrucks {
    pub truck: Option<Truck>,
    pub drops: Vec<Drop>,
    clock: u32,
    rest: u32,
    rng: u32,
}

fn solid(world: &World, x: i32, y: i32) -> bool {
    world.hard_for_car(x.clamp(0, W as i32 - 1), y)
}

/// The ground under column `x` near row `y`: the first solid cell with room above it, looking a
/// few cells up (a gentle climb) and down (a gentle drop). None on water, at a wall or a cliff.
fn ground_near(world: &World, x: i32, y: i32) -> Option<i32> {
    let x = x.clamp(0, W as i32 - 1);
    for gy in (y - 6)..=(y + 12) {
        if world.at(x, gy).kind() == Kind::Liquid && !solid(world, x, gy - 1) {
            return None;
        }
        // It drives through a tree, and over a crown it comes onto from above.
        if world.bears(x, gy, y as f32, 6.0) && !solid(world, x, gy - 1) && !solid(world, x, gy - 2) {
            return Some(gy);
        }
    }
    None
}

/// The ground seen from the sky over column `x`: where an engine coming in from that side starts.
fn ground_from_top(world: &World, x: i32) -> Option<i32> {
    (0..H as i32).find_map(|y| match world.at(x, y).kind() {
        Kind::Liquid => Some(None),
        _ if solid(world, x, y) => Some(Some(y)),
        _ => None,
    })?
}

/// The biggest fire in the world: where it is and how many burning cells it has.
pub fn biggest_fire(world: &World) -> Option<(Vec2, u32)> {
    let (zw, zh) = ((W as i32 + ZONE - 1) / ZONE, (H as i32 + ZONE - 1) / ZONE);
    let mut count = vec![0u32; (zw * zh) as usize];
    let mut sum = vec![Vec2::ZERO; (zw * zh) as usize];
    for y in (0..H as i32).step_by(2) {
        for x in (0..W as i32).step_by(2) {
            if matches!(world.raw(x, y), Mat::Fire | Mat::Ember) {
                let k = ((y / ZONE) * zw + x / ZONE) as usize;
                // One cell in four is looked at: count it four times.
                count[k] += 4;
                sum[k] += vec2(x as f32, y as f32) * 4.0;
            }
        }
    }
    let mut best: Option<(Vec2, u32)> = None;
    for zy in 0..zh {
        for zx in 0..zw {
            let (mut n, mut s) = (0, Vec2::ZERO);
            for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let (x, y) = (zx + dx, zy + dy);
                if x >= 0 && y >= 0 && x < zw && y < zh {
                    n += count[(y * zw + x) as usize];
                    s += sum[(y * zw + x) as usize];
                }
            }
            if n > 0 && best.map_or(true, |b| n > b.1) {
                best = Some((s / n as f32, n));
            }
        }
    }
    best
}

impl FireTrucks {
    pub fn new() -> Self {
        FireTrucks { rng: 0x0f1e_7a11, ..Default::default() }
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

    /// Sends an engine to the fire at `fire`, from the nearer side of the world that has a road.
    pub fn call(&mut self, world: &World, fire: Vec2) -> bool {
        if self.truck.is_some() {
            return false;
        }
        let left_first = fire.x < W as f32 / 2.0;
        for left in [left_first, !left_first] {
            let edge = if left { 0 } else { W as i32 - 1 };
            if let Some(y) = ground_from_top(world, edge) {
                let dir = if left { 1.0 } else { -1.0 };
                let x = if left { -LENGTH } else { W as f32 + LENGTH };
                self.truck = Some(Truck { x, y: y as f32, dir, state: State::Coming, tank: TANK, fire, aim: fire, age: 0, tilt: 0.0, heat: 0 });
                return true;
            }
        }
        false
    }

    /// One simulation step. The engine sprays the world's own water, so it needs the world.
    pub fn step(&mut self, world: &mut World) -> Option<News> {
        self.clock = self.clock.wrapping_add(1);
        self.rest = self.rest.saturating_sub(1);
        self.step_drops(world);
        let Some(mut t) = self.truck.take() else {
            if self.rest == 0 && self.clock % LOOK_EVERY == 0 {
                if let Some((at, n)) = biggest_fire(world) {
                    if n >= BIG_FIRE && self.call(world, at) {
                        return Some(News::Siren);
                    }
                }
                self.rest = LOOK_EVERY;
            }
            return None;
        };
        t.age += 1;
        if t.age % 4 == 0 {
            t.heat = t.heat.saturating_sub(1);
        }
        let mut news = None;
        match t.state {
            State::Coming => {
                if t.age % 140 == 0 {
                    news = Some(News::Siren);
                }
                if (t.fire.x - t.x).abs() <= STOP_AT || (t.fire.x - t.x) * t.dir < 0.0 {
                    t.state = State::Spraying;
                    news = Some(News::Arrived);
                } else if !Self::drive(&mut t, world) {
                    if (t.fire.x - t.x).abs() <= REACH {
                        t.state = State::Spraying;
                        news = Some(News::Arrived);
                    } else {
                        t.state = State::Leaving;
                        t.dir = -t.dir;
                        news = Some(News::Blocked);
                    }
                }
            }
            State::Spraying => {
                // The hose sweeps over the flames: a new one to aim at now and then.
                if t.age % 20 == 0 {
                    if let Some(f) = self.pick_flame(world, t.fire) {
                        t.aim = f;
                    }
                }
                if t.age % 60 == 0 {
                    let left = Self::flames_near(world, t.fire, 130.0);
                    if left <= 6 {
                        news = Some(News::PutOut);
                    } else if t.tank == 0 {
                        news = Some(News::TankEmpty);
                    }
                    if news.is_some() {
                        t.state = State::Leaving;
                        t.dir = -t.dir;
                    }
                }
                for _ in 0..JET.min(t.tank) {
                    let nozzle = Self::nozzle(&t);
                    let d = t.aim - nozzle;
                    let steps = (d.x.abs() / 1.8).clamp(18.0, 60.0);
                    let jitter = vec2(self.between(0.94, 1.06), self.between(0.94, 1.06));
                    let vel = vec2(d.x / steps, d.y / steps - 0.5 * G * steps) * jitter;
                    self.drops.push(Drop { pos: nozzle, vel });
                    t.tank -= 1;
                }
            }
            State::Leaving => {
                let gone = t.x < -LENGTH * 1.5 || t.x > W as f32 + LENGTH * 1.5;
                if gone || !Self::drive(&mut t, world) {
                    self.rest = REST;
                    return news;
                }
            }
        }
        self.truck = Some(t);
        news
    }

    /// The flamethrower plays on the engine around `at`: it heats up, and true once it is hot
    /// enough to blow up.
    pub fn scorch(&mut self, at: Vec2, r: f32) -> bool {
        let Some(t) = &mut self.truck else { return false };
        if !t.touches(at, r) {
            return false;
        }
        t.heat += 1;
        t.heat >= BURN
    }

    /// The engine blows up: it is gone, and the water left in its tank bursts out all around
    /// (real water, that puts out the flames it lands on). Returns the engine, for its wreck.
    pub fn blow_up(&mut self) -> Option<Truck> {
        let t = self.truck.take()?;
        let centre = vec2(t.x, t.y - HEIGHT * 0.5);
        for _ in 0..(t.tank / 8).min(450) {
            let a = self.between(-std::f32::consts::PI, 0.0);
            let speed = self.between(0.6, 3.2);
            let pos = centre + vec2(self.between(-LENGTH * 0.4, LENGTH * 0.4), self.between(-4.0, 4.0));
            self.drops.push(Drop { pos, vel: Vec2::from_angle(a) * speed });
        }
        self.rest = REST;
        Some(t)
    }

    /// Moves the engine one step along the road. False if the road is cut (water, wall, cliff).
    fn drive(t: &mut Truck, world: &World) -> bool {
        let nx = t.x + t.dir * SPEED;
        let front = (nx + t.dir * LENGTH * 0.4) as i32;
        let Some(gy) = ground_near(world, front, t.y as i32) else { return false };
        let rear = ground_near(world, (nx - t.dir * LENGTH * 0.4) as i32, t.y as i32).unwrap_or(gy);
        t.x = nx;
        t.y += (((gy + rear) as f32) * 0.5 - t.y) * 0.3;
        let slope = ((gy - rear) as f32 * t.dir).atan2(LENGTH * 0.8);
        t.tilt += (slope - t.tilt) * 0.2;
        true
    }

    fn nozzle(t: &Truck) -> Vec2 {
        vec2(t.x + t.dir * LENGTH * 0.18, t.y - HEIGHT - 3.0)
    }

    fn flames_near(world: &World, at: Vec2, r: f32) -> u32 {
        let mut n = 0;
        let (x0, x1) = ((at.x - r) as i32, (at.x + r) as i32);
        let (y0, y1) = ((at.y - r) as i32, (at.y + r) as i32);
        for y in y0.max(0)..y1.min(H as i32) {
            for x in x0.max(0)..x1.min(W as i32) {
                if matches!(world.raw(x, y), Mat::Fire | Mat::Ember) {
                    n += 1;
                }
            }
        }
        n
    }

    /// A burning cell near the fire, at random.
    fn pick_flame(&mut self, world: &World, at: Vec2) -> Option<Vec2> {
        for _ in 0..400 {
            let p = at + vec2(self.between(-110.0, 110.0), self.between(-110.0, 110.0));
            if matches!(world.raw(p.x as i32, p.y as i32), Mat::Fire | Mat::Ember) {
                return Some(p);
            }
        }
        None
    }

    fn step_drops(&mut self, world: &mut World) {
        let mut keep = Vec::with_capacity(self.drops.len());
        for mut d in std::mem::take(&mut self.drops) {
            d.vel.y += G;
            let n = d.vel.length().ceil().max(1.0) as i32;
            let mut landed = false;
            let mut last = d.pos;
            for k in 1..=n {
                let p = d.pos + d.vel * (k as f32 / n as f32);
                let (x, y) = (p.x as i32, p.y as i32);
                if y >= H as i32 || x < -2 || x > W as i32 + 2 {
                    landed = true;
                    break;
                }
                if !World::inb(x, y) {
                    last = p;
                    continue;
                }
                match world.at(x, y).kind() {
                    Kind::Empty | Kind::Gas => last = p,
                    // Water on a flame: it is out, in a puff of steam.
                    Kind::Flame => {
                        world.set(x, y, Mat::Steam);
                        landed = true;
                        break;
                    }
                    // Anything else: the drop stays where it hit, as water.
                    _ => {
                        let (lx, ly) = (last.x as i32, last.y as i32);
                        if World::inb(lx, ly) && world.block_at_cell(lx, ly).is_none() && matches!(world.raw(lx, ly).kind(), Kind::Empty | Kind::Gas) {
                            world.set(lx, ly, Mat::Water);
                        }
                        landed = true;
                        break;
                    }
                }
            }
            if !landed {
                d.pos += d.vel;
                keep.push(d);
            }
        }
        self.drops = keep;
    }

    pub fn draw(&self, view: View, time: f32) {
        for d in &self.drops {
            let p = view.to_screen(d.pos);
            draw_circle(p.x, p.y, (0.9 * view.s).max(1.0), Color::from_rgba(120, 180, 255, 230));
        }
        if let Some(t) = &self.truck {
            draw_engine(t, view, time);
        }
    }
}

/// A cartoon fire engine: red body, white stripe, cab with its window at the front, a ladder on the
/// roof, flashing blue lights and, while it sprays, the monitor turned towards the fire.
fn draw_engine(t: &Truck, view: View, time: f32) {
    let (sa, ca) = t.tilt.sin_cos();
    let base = vec2(t.x, t.y);
    // Local cells: x along the engine (+ is the front), y up from the ground.
    let p = |x: f32, y: f32| -> Vec2 {
        let (lx, ly) = (x * t.dir, -y);
        view.to_screen(base + vec2(lx * ca - ly * sa, lx * sa + ly * ca))
    };
    let quad = |x0: f32, y0: f32, x1: f32, y1: f32, c: Color| {
        let (a, b, cc, d) = (p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1));
        draw_triangle(a, b, cc, c);
        draw_triangle(a, cc, d, c);
    };
    // The flamethrower blackens the paint as the engine heats up.
    let burnt = t.heat as f32 / BURN as f32;
    let scorched = |c: Color| Color::new(c.r * (1.0 - 0.7 * burnt), c.g * (1.0 - 0.8 * burnt), c.b * (1.0 - 0.8 * burnt), c.a);
    let red = scorched(Color::from_rgba(214, 36, 36, 255));
    let dark = scorched(Color::from_rgba(120, 16, 20, 255));
    let half = LENGTH * 0.5;
    // Body and cab.
    quad(-half, 3.0, half * 0.45, HEIGHT - 2.0, red);
    quad(half * 0.45, 3.0, half, HEIGHT - 4.0, red);
    quad(half * 0.5, HEIGHT - 9.0, half * 0.92, HEIGHT - 5.0, Color::from_rgba(170, 220, 250, 255));
    quad(-half, 6.0, half, 7.5, Color::from_rgba(250, 250, 250, 255));
    // Roller shutters on the side.
    for k in 0..3 {
        let x0 = -half + 2.0 + k as f32 * 7.0;
        quad(x0, 8.5, x0 + 5.5, HEIGHT - 3.5, dark);
    }
    // The ladder on the roof.
    let grey = Color::from_rgba(200, 204, 214, 255);
    quad(-half - 2.0, HEIGHT - 1.0, half * 0.35, HEIGHT - 0.2, grey);
    quad(-half - 2.0, HEIGHT + 1.0, half * 0.35, HEIGHT + 1.8, grey);
    let mut x = -half - 1.0;
    while x < half * 0.35 {
        quad(x, HEIGHT - 0.2, x + 0.6, HEIGHT + 1.0, grey);
        x += 3.0;
    }
    // The water monitor.
    let m = p(LENGTH * 0.18, HEIGHT + 1.0);
    let tip = p(LENGTH * 0.18 + 3.0, HEIGHT + 3.0);
    draw_line(m.x, m.y, tip.x, tip.y, (1.2 * view.s).max(2.0), grey);
    // Flashing blue lights on the cab.
    let on = (time * 6.0) as i32 % 2 == 0;
    let (l1, l2) = (p(half * 0.6, HEIGHT - 3.0), p(half * 0.85, HEIGHT - 3.0));
    let blue = |b: bool| if b { Color::from_rgba(80, 160, 255, 255) } else { Color::from_rgba(30, 50, 110, 255) };
    draw_circle(l1.x, l1.y, 1.2 * view.s, blue(on));
    draw_circle(l2.x, l2.y, 1.2 * view.s, blue(!on));
    if on {
        draw_circle(l1.x, l1.y, 4.0 * view.s, Color::new(0.3, 0.6, 1.0, 0.25));
    } else {
        draw_circle(l2.x, l2.y, 4.0 * view.s, Color::new(0.3, 0.6, 1.0, 0.25));
    }
    // Wheels.
    for wx in WHEELS {
        let c = p(wx * half, 3.2);
        draw_circle(c.x, c.y, 3.2 * view.s, Color::from_rgba(26, 26, 30, 255));
        draw_circle(c.x, c.y, 1.3 * view.s, Color::from_rgba(170, 170, 180, 255));
    }
    // Hot enough, it starts to burn: flames lick the body, the closer to blowing up the taller.
    if burnt > 0.3 {
        for k in 0..5 {
            let flick = (time * 13.0 + k as f32 * 2.1).sin() * 0.5 + 0.5;
            let x = -half + 4.0 + k as f32 * LENGTH * 0.2;
            let h = (2.0 + 6.0 * burnt) * (0.6 + 0.4 * flick);
            let (a, b, c) = (p(x - 2.0, HEIGHT - 3.0), p(x + 2.0, HEIGHT - 3.0), p(x + flick - 0.5, HEIGHT - 3.0 + h));
            draw_triangle(a, b, c, Color::new(1.0, 0.45 + 0.3 * flick, 0.05, 0.85));
        }
    }
}

/// Where the wheels are, as a share of half the engine's length from its middle.
pub const WHEELS: [f32; 2] = [-0.62, 0.62];
pub const WHEEL_R: f32 = 3.2;
pub const BODY_HEIGHT: f32 = HEIGHT;

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat stone ground at row 450 across the world, with a woodpile set alight at x = 600.
    fn burning(world: &mut World) {
        for x in 0..W as i32 {
            for y in 450..H as i32 {
                world.set(x, y, Mat::Stone);
            }
        }
        for x in 570..630 {
            for y in 410..450 {
                world.set(x, y, Mat::Wood);
            }
        }
        for x in 565..635 {
            world.set(x, 408, Mat::Fire);
            world.set(x, 409, Mat::Fire);
        }
    }

    fn flames(world: &World) -> u32 {
        FireTrucks::flames_near(world, vec2(600.0, 420.0), 150.0)
    }

    #[test]
    fn a_small_fire_does_not_call_the_firemen_but_a_big_one_does() {
        let mut world = World::new_static();
        for x in 0..W as i32 {
            for y in 450..H as i32 {
                world.set(x, y, Mat::Stone);
            }
        }
        for x in 300..306 {
            world.set(x, 448, Mat::Fire);
            world.set(x, 449, Mat::Fire);
        }
        let mut trucks = FireTrucks::new();
        for _ in 0..200 {
            trucks.step(&mut world);
        }
        assert!(trucks.truck.is_none(), "a camp fire is no business of theirs");
        let (at, n) = biggest_fire(&world).unwrap();
        assert!(n < BIG_FIRE && (at.x - 302.0).abs() < 6.0, "{at:?} {n}");
        let mut world = World::new_static();
        burning(&mut world);
        for _ in 0..120 {
            world.step();
        }
        let (at, n) = biggest_fire(&world).unwrap();
        assert!(n >= BIG_FIRE && (at.x - 600.0).abs() < 40.0, "a woodpile ablaze: {n} at {at:?}");
        let mut called = false;
        for _ in 0..120 {
            called |= trucks.step(&mut world) == Some(News::Siren);
        }
        assert!(called && trucks.truck.is_some(), "the fire engine is on its way");
        assert_eq!(trucks.truck.as_ref().unwrap().dir, -1.0, "from the nearer side");
    }

    #[test]
    fn the_engine_drives_up_hoses_the_fire_down_and_leaves() {
        let mut with = World::new_static();
        burning(&mut with);
        let mut without = World::new_static();
        burning(&mut without);
        let mut trucks = FireTrucks::new();
        assert!(trucks.call(&with, vec2(600.0, 420.0)));
        let mut log = vec![];
        let mut worst = 0u32;
        for k in 0..60 * 50 {
            with.step();
            without.step();
            if let Some(n) = trucks.step(&mut with) {
                log.push(n);
            }
            if let Some(t) = &trucks.truck {
                assert!((t.y - 450.0).abs() < 2.0, "it drives on the ground: {}", t.y);
                if t.state == State::Spraying {
                    worst = worst.max((t.x - 600.0).abs() as u32);
                }
            }
            if k == 60 * 20 {
                let (a, b) = (flames(&with), flames(&without));
                eprintln!("FIRE after 20 s: {a} burning cells with the firemen, {b} without");
                assert!(a * 2 < b, "the firemen beat the fire back: {a} vs {b}");
            }
        }
        assert!(log.contains(&News::Arrived), "{log:?}");
        assert!(worst <= STOP_AT as u32 + 2, "it parked near the fire: {worst}");
        assert!(log.contains(&News::PutOut) || log.contains(&News::TankEmpty), "{log:?}");
        assert!(trucks.truck.is_none(), "then it drove off");
    }

    #[test]
    fn a_lake_on_the_way_stops_the_engine() {
        let mut world = World::new_static();
        burning(&mut world);
        // A lake between the right edge and the fire, too far for the hose.
        for x in 860..900 {
            for y in 430..450 {
                world.set(x, y, Mat::Water);
            }
        }
        let mut trucks = FireTrucks::new();
        assert!(trucks.call(&world, vec2(600.0, 420.0)));
        let mut log = vec![];
        for _ in 0..60 * 10 {
            if let Some(n) = trucks.step(&mut world) {
                log.push(n);
            }
        }
        assert!(log.contains(&News::Blocked), "{log:?}");
    }

    #[test]
    fn the_flamethrower_and_lightning_blow_the_engine_up() {
        let mut world = World::new_static();
        burning(&mut world);
        let mut trucks = FireTrucks::new();
        assert!(trucks.call(&world, vec2(600.0, 420.0)));
        for _ in 0..200 {
            trucks.step(&mut world);
        }
        let t = trucks.truck.clone().unwrap();
        let side = vec2(t.x, t.y - 8.0);
        assert!(t.touches(side, 0.0) && !t.touches(side + vec2(0.0, -60.0), 3.0) && !t.touches(vec2(t.x + 60.0, t.y), 3.0));
        // A brief lick of flame only scorches it.
        for _ in 0..10 {
            assert!(!trucks.scorch(side, 3.0));
        }
        assert!(!trucks.scorch(side + vec2(0.0, -60.0), 3.0), "flames in the sky miss it");
        let mut steps = 10;
        while !trucks.scorch(side, 3.0) {
            steps += 1;
            assert!(steps < 200, "the flamethrower never sets it off");
        }
        let tank = trucks.truck.as_ref().unwrap().tank;
        let drops = trucks.drops.len();
        let wreck = trucks.blow_up().expect("it blows up");
        assert!(trucks.truck.is_none() && wreck.x == t.x);
        assert!(trucks.drops.len() >= drops + (tank / 8).min(450) as usize, "its tank bursts out");
        // It is gone for a while: no new engine straight away.
        for _ in 0..120 {
            trucks.step(&mut world);
        }
        assert!(trucks.truck.is_none());

        // Lightning: the bolt down through the engine passes through its body.
        let mut world = World::new_static();
        burning(&mut world);
        let mut trucks = FireTrucks::new();
        assert!(trucks.call(&world, vec2(600.0, 420.0)));
        for _ in 0..200 {
            trucks.step(&mut world);
        }
        let t = trucks.truck.clone().unwrap();
        world.strike(t.x as i32, (t.y - 5.0) as i32);
        let bolt = world.bolts.last().unwrap();
        assert!(bolt.ground && bolt.path.iter().any(|&(x, y)| t.touches(vec2(x, y), 2.0)), "the bolt goes through the engine");
    }
}
