//! Just for fun: where the world has a big stretch of deep water, now and then a whale swims up,
//! shows its back, blows a spout of mist through its blowhole, then dives again with a flick of its
//! tail. It needs real water: drain the sea and it goes away. It only draws over the world.

use crate::render::View;
use crate::world::{Kind, Mat, World, H, W};
use macroquad::prelude::*;

/// Length of the whale, in world cells.
pub const LENGTH: f32 = 50.0;
/// Half the height of its body.
const HALF_H: f32 = 7.0;
/// The water it needs: this deep, over this many cells side by side.
pub const MIN_DEPTH: i32 = 24;
pub const MIN_WIDTH: i32 = 150;
/// Seconds between whales, at random.
const GAP: (f32, f32) = (35.0, 100.0);
/// Swimming speed, cells per second.
const SPEED: f32 = 7.0;
/// The timeline of a visit (seconds): rise, stay at the surface (and blow twice), dive.
const RISE: f32 = 5.0;
const SURFACE: f32 = 12.0;
const DIVE: f32 = 15.5;
const BLOWS: [f32; 2] = [5.6, 9.0];
const GRAVITY: f32 = 90.0;

#[derive(Clone, Debug)]
pub struct Whale {
    pub x: f32,
    /// The water surface where it swims (row), followed as the sea rises or falls.
    pub surface: f32,
    /// +1 swimming right, -1 left.
    pub dir: f32,
    pub age: f32,
    /// Spouts blown so far.
    pub blown: usize,
}

impl Whale {
    /// How far the top of its back is below the water (negative: above it).
    pub fn sink(&self) -> f32 {
        let t = self.age;
        if t < RISE {
            16.0 - 18.0 * (t / RISE)
        } else if t < SURFACE {
            -2.5 + 0.6 * (t * 2.0).sin()
        } else {
            -2.5 + 25.0 * ((t - SURFACE) / (DIVE - SURFACE)).powi(2)
        }
    }

    /// Nose-down tilt while diving (radians).
    fn pitch(&self) -> f32 {
        if self.age < SURFACE { 0.0 } else { (((self.age - SURFACE) / (DIVE - SURFACE)) * 3.0).min(1.0) * 0.55 }
    }

    fn blowhole(&self) -> Vec2 {
        vec2(self.x + self.dir * LENGTH * 0.2, self.surface + self.sink())
    }
}

#[derive(Clone, Debug)]
pub struct Drop {
    pub pos: Vec2,
    pub vel: Vec2,
    pub age: f32,
    pub life: f32,
}

#[derive(Default)]
pub struct Whales {
    pub whale: Option<Whale>,
    pub mist: Vec<Drop>,
    cooldown: f32,
    rng: u32,
}

/// The water surface over a column, if the water there is at least `MIN_DEPTH` deep.
pub fn surface_at(world: &World, x: i32) -> Option<i32> {
    if !(0..W as i32).contains(&x) {
        return None;
    }
    let mut y = 0;
    while y < H as i32 && matches!(world.at(x, y).kind(), Kind::Empty | Kind::Gas) {
        y += 1;
    }
    let deep = (y..y + MIN_DEPTH).all(|d| world.at(x, d) == Mat::Water);
    deep.then_some(y)
}

/// The widest stretch of deep open water: (left, right, surface row).
pub fn find_sea(world: &World) -> Option<(i32, i32, i32)> {
    let step = 3;
    let mut best: Option<(i32, i32, i32)> = None;
    let mut run: Option<(i32, i32, i32)> = None;
    for x in (0..W as i32).step_by(step) {
        match (surface_at(world, x), run) {
            (Some(y), Some((x0, _, y0))) if (y - y0).abs() <= 3 => run = Some((x0, x, y0)),
            (Some(y), _) => run = Some((x, x, y)),
            (None, _) => run = None,
        }
        if let Some(r) = run {
            if best.map_or(true, |b| r.1 - r.0 > b.1 - b.0) {
                best = Some(r);
            }
        }
    }
    best.filter(|b| b.1 - b.0 >= MIN_WIDTH)
}

impl Whales {
    pub fn new() -> Self {
        Whales { cooldown: 20.0, rng: 0x0b1e_5ea5, ..Default::default() }
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

    /// A whale comes up somewhere in the widest sea, if there is one. Returns whether it did.
    pub fn summon(&mut self, world: &World) -> bool {
        if self.whale.is_some() {
            return false;
        }
        let Some((x0, x1, y)) = find_sea(world) else { return false };
        let margin = LENGTH * 0.6;
        let x = self.between(x0 as f32 + margin, (x1 as f32 - margin).max(x0 as f32 + margin));
        let dir = if self.rand() < 0.5 { -1.0 } else { 1.0 };
        self.whale = Some(Whale { x, surface: y as f32, dir, age: 0.0, blown: 0 });
        true
    }

    /// `dt` in seconds. Returns true when the whale blows (for the sound).
    pub fn update(&mut self, dt: f32, world: &World) -> bool {
        for d in &mut self.mist {
            d.age += dt;
            d.vel.y += GRAVITY * dt;
            d.vel.x *= (-1.5 * dt).exp();
            d.pos += d.vel * dt;
        }
        self.mist.retain(|d| d.age < d.life);
        let Some(mut w) = self.whale.take() else {
            self.cooldown -= dt;
            if self.cooldown <= 0.0 {
                let came = self.summon(world);
                self.cooldown = if came { self.between(GAP.0, GAP.1) } else { 10.0 };
            }
            return false;
        };
        w.age += dt;
        // It turns round before the end of its sea.
        let ahead = (w.x + w.dir * (LENGTH * 0.5 + 4.0)) as i32;
        if surface_at(world, ahead).is_none() {
            w.dir = -w.dir;
        }
        w.x += w.dir * SPEED * dt;
        // It follows the water level, and leaves if the water under it has gone.
        match surface_at(world, w.x as i32) {
            Some(y) => w.surface += (y as f32 - w.surface) * (dt * 3.0).min(1.0),
            None => return false,
        }
        let mut blew = false;
        if w.blown < BLOWS.len() && w.age >= BLOWS[w.blown] {
            w.blown += 1;
            blew = true;
            let at = w.blowhole();
            for k in 0..48 {
                let spread = (k as f32 / 47.0 - 0.5) * 2.0;
                let vel = vec2(spread * 14.0 + self.between(-4.0, 4.0), -self.between(55.0, 85.0) * (1.0 - spread.abs() * 0.3));
                let life = self.between(1.1, 1.6);
                self.mist.push(Drop { pos: at, vel, age: 0.0, life });
            }
        }
        if w.age < DIVE {
            self.whale = Some(w);
        }
        blew
    }

    pub fn draw(&self, view: View) {
        if let Some(w) = &self.whale {
            draw_whale(w, view);
        }
        for d in &self.mist {
            let p = view.to_screen(d.pos);
            let a = (1.0 - d.age / d.life) * 0.8;
            draw_circle(p.x, p.y, (1.3 * view.s).max(1.0), Color::new(0.92, 0.96, 1.0, a));
        }
    }
}

/// A cartoon whale: dark blue back, pale grooved belly, a little eye and a smile. What is under
/// the water is drawn faint and blue, what is out of it solid.
fn draw_whale(w: &Whale, view: View) {
    let sink = w.sink();
    let surf = w.surface;
    let centre = vec2(w.x, surf + sink + HALF_H);
    let (sa, ca) = (w.pitch() * w.dir).sin_cos();
    // Local (-0.5..0.5 along the body, y down in half-heights) to world.
    let local = |x: f32, y: f32| -> Vec2 {
        let (lx, ly) = (x * w.dir * LENGTH, y * HALF_H);
        centre + vec2(lx * ca - ly * sa, lx * sa + ly * ca)
    };
    // The outline: a big rounded head, tapering to the tail stock.
    let n = 28;
    let outline: Vec<Vec2> = (0..n)
        .map(|k| {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            let x = 0.5 * a.cos();
            let taper = if x < 0.0 { 1.0 + x * 1.3 } else { 1.0 };
            local(x, a.sin() * taper.max(0.2))
        })
        .collect();
    let back = Color::from_rgba(52, 78, 118, 255);
    let belly = Color::from_rgba(196, 210, 226, 255);
    let deep = Color::from_rgba(20, 34, 60, 255);
    let fan = |pts: &[Vec2], mid: Vec2, col: Color, clip: bool| {
        // `clip` keeps only what is out of the water.
        let c = |p: Vec2| view.to_screen(if clip { vec2(p.x, p.y.min(surf)) } else { p });
        let m = c(mid);
        for k in 0..pts.len() {
            draw_triangle(m, c(pts[k]), c(pts[(k + 1) % pts.len()]), col);
        }
    };
    // Under the water: a faint shape.
    let under = (1.0 - (sink / 24.0)).clamp(0.15, 1.0) * 0.5;
    fan(&outline, centre, Color::new(0.12, 0.2, 0.34, under), false);
    // The tail fluke, flicked up out of the water as it dives.
    let dive = ((w.age - SURFACE) / (DIVE - SURFACE)).clamp(0.0, 1.0);
    let lift = if w.age > SURFACE { (dive * std::f32::consts::PI).sin() * 16.0 } else { 0.0 };
    let stock = local(-0.5, 0.0) - vec2(0.0, lift);
    let tail_col = if stock.y < surf { back } else { Color::new(0.12, 0.2, 0.34, under) };
    let fl = |dx: f32, dy: f32| stock + vec2(-w.dir * dx, dy);
    let (a, b, c2) = (view.to_screen(fl(0.0, 0.0)), view.to_screen(fl(6.0, -6.0)), view.to_screen(fl(3.0, 1.0)));
    draw_triangle(a, b, c2, tail_col);
    let (b2, c3) = (view.to_screen(fl(6.0, 6.0)), view.to_screen(fl(3.0, -1.0)));
    draw_triangle(a, b2, c3, tail_col);
    if sink >= 0.0 {
        return;
    }
    // Out of the water: the back, with a pale belly line, the eye and the smile.
    fan(&outline, centre, back, true);
    let belly_pts: Vec<Vec2> = (0..=10).map(|k| local(0.48 - k as f32 * 0.07, 0.35 + k as f32 * 0.02)).collect();
    for p in belly_pts.windows(2) {
        let (a, b) = (vec2(p[0].x, p[0].y.min(surf)), vec2(p[1].x, p[1].y.min(surf)));
        if a.y < surf - 0.5 || b.y < surf - 0.5 {
            let (sa, sb) = (view.to_screen(a), view.to_screen(b));
            draw_line(sa.x, sa.y, sb.x, sb.y, (1.2 * view.s).max(1.5), belly);
        }
    }
    let eye = local(0.36, 0.05);
    if eye.y < surf {
        let e = view.to_screen(eye);
        draw_circle(e.x, e.y, 1.2 * view.s, deep);
        draw_circle(e.x + 0.3 * view.s, e.y - 0.3 * view.s, 0.45 * view.s, WHITE);
    }
    let mut prev = local(0.5, 0.25);
    for k in 1..=6 {
        let t = k as f32 / 6.0;
        let p = local(0.5 - 0.16 * t, 0.25 + 0.25 * (t * std::f32::consts::PI).sin());
        if p.y < surf && prev.y < surf {
            let (a, b) = (view.to_screen(prev), view.to_screen(p));
            draw_line(a.x, a.y, b.x, b.y, (0.6 * view.s).max(1.2), deep);
        }
        prev = p;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stone basin across the world filled with water to `level`, bottom at row 500.
    fn sea(width: i32, level: i32) -> World {
        let mut w = World::new_static();
        for x in 0..W as i32 {
            for y in 500..H as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        let x0 = 100;
        for x in x0..x0 + width {
            for y in level..500 {
                w.set(x, y, Mat::Water);
            }
        }
        w
    }

    #[test]
    fn whales_need_a_wide_deep_sea() {
        assert!(find_sea(&World::new_static()).is_none(), "no water, no whale");
        assert!(find_sea(&sea(80, 400)).is_none(), "a pond is too narrow");
        assert!(find_sea(&sea(400, 490)).is_none(), "a puddle is too shallow");
        let (x0, x1, y) = find_sea(&sea(400, 400)).expect("a real sea");
        assert!(x0 >= 100 && x1 < 500 && y == 400, "{x0} {x1} {y}");
        let mut whales = Whales::new();
        let mut world = World::new_static();
        for _ in 0..60 * 120 {
            whales.update(1.0 / 60.0, &world);
        }
        assert!(whales.whale.is_none());
        world = sea(500, 380);
        let mut seen = false;
        for _ in 0..60 * 60 {
            whales.update(1.0 / 60.0, &world);
            seen |= whales.whale.is_some();
        }
        assert!(seen, "with a sea, a whale comes within a minute");
    }

    #[test]
    fn it_surfaces_blows_twice_and_dives_staying_in_its_sea() {
        let world = sea(300, 380);
        let mut whales = Whales::new();
        assert!(whales.summon(&world));
        let (mut blows, mut surfaced, mut mist) = (0, false, 0);
        for _ in 0..60 * 20 {
            if whales.update(1.0 / 60.0, &world) {
                blows += 1;
            }
            mist = mist.max(whales.mist.len());
            if let Some(w) = &whales.whale {
                surfaced |= w.sink() < 0.0;
                assert!(w.x > 100.0 && w.x < 400.0, "it stays in the water: {}", w.x);
                assert!((w.surface - 380.0).abs() < 1.0);
            }
        }
        assert!(surfaced, "its back came out of the water");
        assert_eq!(blows, 2, "it blew twice");
        assert!(mist > 40, "a spout of mist");
        assert!(whales.whale.is_none() && whales.mist.is_empty(), "then it dove and the mist fell back");
    }

    #[test]
    fn the_sea_world_has_room_for_a_whale() {
        let mut world = World::new();
        world.generate_biome(1, crate::world::Biome::Sea);
        assert!(find_sea(&world).is_some());
    }

    #[test]
    fn it_leaves_when_the_sea_is_drained() {
        let mut world = sea(400, 380);
        let mut whales = Whales::new();
        assert!(whales.summon(&world));
        whales.update(1.0 / 60.0, &world);
        for x in 0..W as i32 {
            for y in 380..500 {
                world.set(x, y, Mat::Empty);
            }
        }
        whales.update(1.0 / 60.0, &world);
        assert!(whales.whale.is_none());
    }
}
