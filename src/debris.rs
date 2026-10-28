//! A car that blows up flies apart: its wheels, body panels, bumper and windows are thrown out by
//! the blast, fall, bounce and roll on whatever they land on, smoke while they are hot, then fade.
//! Each piece is a little rigid body of its own (not rapier: a handful of points tested against
//! the cells is plenty and costs nothing).

use crate::car::{Car, S};
use crate::firetruck::{self, Truck};
use crate::render::View;
use crate::world::{ballistic, units, Mat, World, H, W};
use crate::gfx::*;

/// Steps a piece lasts before it is gone (it fades during the last second).
const LIFE: u32 = 60 * 9;
/// The fastest a piece falls through the air, m/s: a thin panel tumbling catches the air.
const TERMINAL_MS: f32 = 15.0;
/// No more pieces than this at once: the oldest go first.
const MAX_PIECES: usize = 90;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shape {
    /// A wheel of radius `r`: it bounces like rubber and rolls.
    Wheel { r: f32 },
    /// A flat piece of `w`×`h` cells: a panel, a bumper, a window.
    Panel { w: f32, h: f32 },
}

#[derive(Clone, Debug)]
pub struct Piece {
    pub p: Vec2,
    /// Cells per step.
    pub v: Vec2,
    pub angle: f32,
    /// Radians per step.
    pub spin: f32,
    pub shape: Shape,
    pub color: (u8, u8, u8),
    /// 1 straight out of the fire, cooling to 0: glows orange and smokes meanwhile.
    pub hot: f32,
    pub age: u32,
    /// Whether it touched something this step.
    pub contact: bool,
    /// Lying still on something solid: it stays put until what holds it goes.
    pub rest: bool,
}

impl Piece {
    /// Points on the outline tested against the ground, relative to the centre.
    fn outline(&self) -> Vec<Vec2> {
        match self.shape {
            Shape::Wheel { r } => (0..10).map(|k| Vec2::from_angle(k as f32 * std::f32::consts::TAU / 10.0) * r).collect(),
            Shape::Panel { w, h } => {
                let (cx, cy) = (w / 2.0, h / 2.0);
                [vec2(-cx, -cy), vec2(cx, -cy), vec2(cx, cy), vec2(-cx, cy), vec2(0.0, -cy), vec2(0.0, cy)].iter().map(|&q| Vec2::from_angle(self.angle).rotate(q)).collect()
            }
        }
    }

    fn bounciness(&self) -> f32 {
        match self.shape {
            Shape::Wheel { .. } => 0.55,
            Shape::Panel { .. } => 0.35,
        }
    }
}

#[derive(Clone, Default)]
pub struct Debris {
    pub pieces: Vec<Piece>,
    seed: u32,
}

fn solid(world: &World, q: Vec2) -> bool {
    let (x, y) = (q.x.floor() as i32, q.y.floor() as i32);
    world.hard_for_car(x, y)
}

impl Debris {
    fn rand(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Blows `car` apart: its pieces fly off in every direction, mostly up and away from the blast.
    pub fn burst(&mut self, car: &Car) {
        self.seed ^= car.id.wrapping_mul(2654435761);
        let m = car.kind.model();
        let rgb = car.color_rgb();
        let dark = ((rgb.0 as f32 * 0.6) as u8, (rgb.1 as f32 * 0.6) as u8, (rgb.2 as f32 * 0.6) as u8);
        let mut parts: Vec<(Vec2, Shape, (u8, u8, u8))> = vec![];
        for (i, w) in car.wheels.iter().enumerate() {
            parts.push((w.pos, Shape::Wheel { r: car.wheel_r(i) }, (26, 26, 32)));
        }
        // The body breaks into a few panels, two rows of three, plus the bumper and the windows.
        let (hl, top, bot) = (m.hull_half * S, m.hull_top * S, m.hull_bottom * S);
        let local = |lx: f32, ly: f32| car.p + Vec2::from_angle(car.angle).rotate(vec2(lx * car.dir, ly));
        let (pw, ph) = (2.0 * hl / 3.0, (bot - top) / 2.0);
        for row in 0..2 {
            for col in 0..3 {
                let c = local(-hl + pw * (col as f32 + 0.5), top + ph * (row as f32 + 0.5));
                parts.push((c, Shape::Panel { w: pw * 0.95, h: ph * 0.9 }, if row == 0 { rgb } else { dark }));
            }
        }
        parts.push((local(0.0, bot - 1.0), Shape::Panel { w: hl * 1.4, h: 2.0 }, dark));
        for k in 0..3 {
            parts.push((local(-hl * 0.3 + k as f32 * hl * 0.3, top + 1.5), Shape::Panel { w: 3.0, h: 2.0 }, (170, 225, 250)));
        }
        self.fling(parts, car.p, car.v * 0.5, car.angle);
    }

    /// Blows a fire engine apart: wheels, red panels, the cab's windscreen and its long ladder,
    /// thrown higher than a car's (its fuel and its tank under pressure make a bigger bang).
    pub fn burst_truck(&mut self, t: &Truck) {
        self.seed ^= t.age.wrapping_mul(2654435761) | 1;
        let half = firetruck::LENGTH * 0.5;
        let h = firetruck::BODY_HEIGHT;
        let base = vec2(t.x, t.y);
        let local = |lx: f32, ly: f32| base + Vec2::from_angle(t.tilt).rotate(vec2(lx * t.dir, -ly));
        let (red, dark) = ((214, 36, 36), (120, 16, 20));
        let mut parts: Vec<(Vec2, Shape, (u8, u8, u8))> = vec![];
        for wx in firetruck::WHEELS {
            parts.push((local(wx * half, firetruck::WHEEL_R), Shape::Wheel { r: firetruck::WHEEL_R }, (26, 26, 32)));
        }
        let (pw, ph) = (2.0 * half / 4.0, (h - 5.0) / 2.0);
        for row in 0..2 {
            for col in 0..4 {
                let c = local(-half + pw * (col as f32 + 0.5), 3.0 + ph * (row as f32 + 0.5));
                parts.push((c, Shape::Panel { w: pw * 0.9, h: ph * 0.9 }, if row == 0 { dark } else { red }));
            }
        }
        parts.push((local(half * 0.7, h - 7.0), Shape::Panel { w: 6.0, h: 3.5 }, (170, 220, 250)));
        parts.push((local(-half * 0.3, h + 0.5), Shape::Panel { w: half * 1.1, h: 1.6 }, (200, 204, 214)));
        parts.push((local(-half * 0.2, h + 1.5), Shape::Panel { w: half * 0.6, h: 1.6 }, (200, 204, 214)));
        self.fling(parts, local(0.0, h * 0.4), vec2(t.dir * 0.4, -0.8), t.tilt);
    }

    /// Throws `parts` out of a blast at `centre`, on top of the speed `v` they already had.
    fn fling(&mut self, parts: Vec<(Vec2, Shape, (u8, u8, u8))>, centre: Vec2, v0: Vec2, angle: f32) {
        for (p, shape, color) in parts {
            // Away from the centre of the blast, but the ground under it sends everything up.
            let d = p - centre;
            let away = vec2(d.x, d.y.min(0.0)).normalize_or(vec2(0.0, -1.0));
            let speed = 0.9 + 1.4 * self.rand();
            let lift = 1.0 + 1.6 * self.rand();
            let v = v0 + away * speed + vec2((self.rand() - 0.5) * 1.2, -lift);
            let spin = (self.rand() - 0.5) * 0.7;
            self.pieces.push(Piece { p, v, angle, spin, shape, color, hot: 1.0, age: 0, contact: false, rest: false });
        }
        if self.pieces.len() > MAX_PIECES {
            let extra = self.pieces.len() - MAX_PIECES;
            self.pieces.drain(..extra);
        }
    }

    /// One simulation step. Returns how hard the pieces hit things this step (for the sound) and
    /// where hot pieces give off smoke.
    pub fn step(&mut self, world: &World) -> (f32, Vec<(i32, i32)>) {
        let mut thud = 0.0f32;
        let mut smoke = vec![];
        for pc in &mut self.pieces {
            pc.age += 1;
            pc.hot = (pc.hot - 1.0 / 300.0).max(0.0);
            if pc.rest {
                if pc.outline().iter().any(|&q| solid(world, pc.p + q + vec2(0.0, 1.0))) {
                    if pc.hot > 0.2 && pc.age % 9 == 0 {
                        smoke.push((pc.p.x as i32, pc.p.y as i32 - 2));
                    }
                    continue;
                }
                pc.rest = false;
            }
            // Water and other liquids brake a piece hard; the air barely.
            let here = world.raw(pc.p.x as i32, pc.p.y as i32);
            let wet = matches!(here, Mat::Water | Mat::Oil | Mat::Mud | Mat::Lava);
            ballistic::pull(&mut pc.v.x, &mut pc.v.y, units::g_tick(), units::cells_per_tick(TERMINAL_MS));
            if wet {
                pc.v *= 0.9;
            }
            if wet && here == Mat::Water {
                pc.hot = 0.0;
            }
            pc.p += pc.v;
            pc.angle += pc.spin;
            pc.contact = false;
            // Which points of the outline went into something solid?
            let outline = pc.outline();
            let mut push = Vec2::ZERO;
            let mut inside = 0;
            for &q in &outline {
                if solid(world, pc.p + q) {
                    push -= q;
                    inside += 1;
                }
            }
            if inside == 0 {
                if pc.hot > 0.2 && pc.age % 9 == 0 {
                    smoke.push((pc.p.x as i32, pc.p.y as i32 - 2));
                }
                continue;
            }
            pc.contact = true;
            let n = if inside == outline.len() { vec2(0.0, -1.0) } else { push.normalize_or(vec2(0.0, -1.0)) };
            // Back out of the ground, half a cell at a time.
            for _ in 0..12 {
                if !outline.iter().any(|&q| solid(world, pc.p + q)) {
                    break;
                }
                pc.p += n * 0.5;
            }
            let vn = pc.v.dot(n);
            if vn < 0.0 {
                thud = thud.max(-vn);
                let tangent = pc.v - n * vn;
                match pc.shape {
                    Shape::Wheel { r } => {
                        // A wheel grips and rolls: its spin follows its speed along the ground.
                        let t = vec2(-n.y, n.x);
                        let along = tangent.dot(t) * 0.95;
                        pc.v = t * along - n * vn * pc.bounciness();
                        pc.spin = along / r;
                    }
                    Shape::Panel { .. } => {
                        // A panel scrapes and tumbles, then settles flat.
                        pc.v = tangent * 0.6 - n * vn * pc.bounciness();
                        // Its weight tips it over onto its nearest side.
                        pc.spin = pc.spin * 0.5 + (square(pc.angle) - pc.angle) * 0.15;
                    }
                }
                // Too slow to bounce again: it lies there.
                let settled = match pc.shape {
                    Shape::Wheel { .. } => pc.v.length() < 0.15,
                    // A panel lying (nearly) flat stops scraping.
                    Shape::Panel { .. } => pc.v.length() < 0.3 && (pc.angle - square(pc.angle)).abs() < 0.35,
                };
                if settled {
                    pc.v = Vec2::ZERO;
                    if let Shape::Panel { .. } = pc.shape {
                        pc.angle = square(pc.angle);
                    }
                    pc.spin = 0.0;
                    pc.rest = true;
                }
            }
        }
        let gone = |p: &Piece| p.age >= LIFE || p.p.y > H() as f32 + 30.0 || p.p.x < -30.0 || p.p.x > W() as f32 + 30.0;
        self.pieces.retain(|p| !gone(p));
        (thud, smoke)
    }

    pub fn clear(&mut self) {
        self.pieces.clear();
    }

    pub fn draw(&self, v: View) {
        for pc in &self.pieces {
            let fade = ((LIFE - pc.age) as f32 / 60.0).min(1.0);
            let c = v.to_screen(pc.p);
            let col = |rgb: (u8, u8, u8), k: f32| {
                let (r, g, b) = (rgb.0 as f32 / 255.0 * k, rgb.1 as f32 / 255.0 * k, rgb.2 as f32 / 255.0 * k);
                // Hot metal glows orange.
                let h = pc.hot * pc.hot * 0.5;
                Color::new(r + (1.0 - r) * h, g + (0.45 - g) * h, b * (1.0 - h), fade)
            };
            match pc.shape {
                Shape::Wheel { r } => {
                    let r = r * v.s;
                    draw_circle(c.x, c.y, r, col(pc.color, 1.0));
                    draw_circle(c.x, c.y, r * 0.62, col((160, 164, 178), 1.0));
                    for k in 0..5 {
                        let a = pc.angle + k as f32 * std::f32::consts::TAU / 5.0;
                        let q = c + Vec2::from_angle(a) * r * 0.6;
                        draw_line(c.x, c.y, q.x, q.y, v.s * 0.9, col((90, 94, 108), 1.0));
                    }
                }
                Shape::Panel { w, h } => {
                    let rot = Vec2::from_angle(pc.angle);
                    let corner = |sx: f32, sy: f32| c + rot.rotate(vec2(sx * w / 2.0, sy * h / 2.0)) * v.s;
                    let (a, b, cc, d) = (corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0));
                    draw_triangle(a, b, cc, col(pc.color, 1.0));
                    draw_triangle(a, cc, d, col(pc.color, 1.0));
                    let edge = col(pc.color, 0.55);
                    for (p, q) in [(a, b), (b, cc), (cc, d), (d, a)] {
                        draw_line(p.x, p.y, q.x, q.y, (v.s * 0.6).max(1.0), edge);
                    }
                }
            }
        }
    }
}

/// The nearest angle where a panel lies on one of its sides.
fn square(a: f32) -> f32 {
    let q = std::f32::consts::FRAC_PI_2;
    (a / q).round() * q
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::ModelKind;

    /// A flat stone floor at y = 300.
    fn floor() -> World {
        let mut w = World::new_static();
        for x in 0..W() as i32 {
            for y in 300..330 {
                w.set(x, y, Mat::Stone);
            }
        }
        w
    }

    #[test]
    fn an_exploded_car_flies_apart_in_every_direction_then_lands_and_settles() {
        let world = floor();
        let mut car = Car::new(ModelKind::Offroad, (400, 280));
        car.p = vec2(400.0, 288.0);
        let mut d = Debris::default();
        d.burst(&car);
        let n = d.pieces.len();
        assert!(n >= 10, "wheels, panels, bumper and windows: {n}");
        assert!(d.pieces.iter().any(|p| p.v.x < -0.3) && d.pieces.iter().any(|p| p.v.x > 0.3), "pieces go both ways");
        assert!(d.pieces.iter().all(|p| p.v.y < 0.0), "the blast throws everything up");
        let (mut highest, mut bounced) = (f32::MAX, false);
        let mut was_falling = vec![false; n];
        for _ in 0..400 {
            d.step(&world);
            for (i, p) in d.pieces.iter().enumerate() {
                highest = highest.min(p.p.y);
                // Falling, then going up again after a touch: a bounce.
                if was_falling[i] && p.contact && p.v.y < -0.1 {
                    bounced = true;
                }
                was_falling[i] = p.v.y > 0.5;
            }
        }
        assert!(highest < 270.0, "the pieces fly high: {highest}");
        assert!(bounced, "pieces bounce when they land");
        assert_eq!(d.pieces.len(), n, "nothing fell through the floor");
        for p in &d.pieces {
            assert!(p.p.y < 300.0 && p.p.y > 285.0, "every piece lies on the floor: {:?}", p.p);
            assert!(p.v.length() < 0.2, "and has come to rest: {:?} {:?} {:?} angle {} spin {} rest {}", p.v, p.shape, p.p, p.angle, p.spin, p.rest);
        }
        let spread = d.pieces.iter().map(|p| p.p.x).fold(f32::MIN, f32::max) - d.pieces.iter().map(|p| p.p.x).fold(f32::MAX, f32::min);
        assert!(spread > 60.0, "the wreck is scattered: {spread}");
    }

    #[test]
    fn debris_cools_and_disappears() {
        let world = floor();
        let car = Car::new(ModelKind::Buggy, (400, 280));
        let mut d = Debris::default();
        d.burst(&car);
        let mut smoked = false;
        for _ in 0..200 {
            smoked |= !d.step(&world).1.is_empty();
        }
        assert!(smoked, "hot pieces smoke");
        assert!(d.pieces.iter().all(|p| p.hot < 0.6));
        for _ in 0..LIFE {
            d.step(&world);
        }
        assert!(d.pieces.is_empty());
    }
}
