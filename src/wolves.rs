//! Just for fun: on a full-moon night a small pack of wolves comes up to the highest point of the
//! world, sits facing the moon, raises its heads and howls. The cub joins in, off-key, and the
//! moon, flattered, winks at them. (Real wolves do not howl at the moon: they call one another,
//! heads raised so that the sound carries far, and night is when they are about.) It only draws
//! over the world.

use crate::render::View;
use crate::ui::Ui;
use crate::world::{Kind, Mat, World, H, W};
use macroquad::prelude::*;

/// The timeline of a visit (seconds): the pack comes, howls three times, the moon winks, they go.
const COME: f32 = 2.0;
const HOWLS: [f32; 3] = [2.5, 7.0, 11.5];
/// How long one howl lasts.
const HOWL: f32 = 3.0;
/// The cub tries once, a little late.
const CUB: f32 = 8.6;
const WINK: (f32, f32) = (13.5, 17.5);
const FACT: f32 = 18.0;
const GONE: f32 = 23.0;
/// Cells per unit of the wolf drawing (a sitting wolf is 10 units tall).
const SCALE: f32 = 2.6;

#[derive(Clone, Debug)]
pub struct Wolf {
    /// Where its hind feet are (world cells).
    pub x: f32,
    pub y: f32,
    /// +1 facing right (towards the moon), -1 left.
    pub dir: f32,
    /// 1 for a grown wolf, smaller for the cub.
    pub size: f32,
    /// It starts each howl this much after the others.
    pub delay: f32,
}

impl Wolf {
    fn cub(&self) -> bool {
        self.size < 0.8
    }
}

#[derive(Clone, Debug)]
pub struct Pack {
    pub wolves: Vec<Wolf>,
    pub age: f32,
}

/// What happened this frame, for the sounds and the messages.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum News {
    Howl,
    Cub,
    Wink,
    Fact,
}

#[derive(Default)]
pub struct Wolves {
    pub pack: Option<Pack>,
    /// The night (day number) they last came: once a night is enough.
    night: u32,
}

/// The natural ground under a column (soil, rock, sand, snow), if nothing else lies on it:
/// wolves do not perch on a building, in a fire or under water. Trees are looked through.
fn ground_at(world: &World, x: i32) -> Option<i32> {
    for y in 0..H as i32 {
        let m = world.at(x, y);
        if matches!(m.kind(), Kind::Empty | Kind::Gas) || m.is_vegetation() {
            continue;
        }
        let natural = matches!(m, Mat::Stone | Mat::Dirt | Mat::Grass | Mat::Sand | Mat::Snow | Mat::Ice | Mat::Mud | Mat::Clay);
        return natural.then_some(y);
    }
    None
}

/// Where the pack sits: the highest natural ground, away from the edges, with room for three
/// wolves side by side. Gives the leader's spot and the ground under each wolf (leader, the
/// one behind it, the cub in front), for a pack facing `dir`.
fn find_spot(world: &World, dir: f32) -> Option<Vec<(f32, f32)>> {
    let mut tops: Vec<(i32, i32)> = (40..W as i32 - 40).step_by(2).filter_map(|x| ground_at(world, x).map(|y| (x, y))).collect();
    tops.sort_by_key(|&(x, y)| (y, x));
    tops.into_iter().take(120).filter(|&(x, y)| !near_fire(world, x, y)).find_map(|(px, py)| {
        OFFSETS
            .iter()
            .map(|&(dx, _, _)| {
                let x = px as f32 + dx * dir;
                ground_at(world, x as i32).filter(|y| (y - py).abs() <= 14).map(|y| (x, y as f32))
            })
            .collect()
    })
}

/// The highest bit of ground where a pack fits.
pub fn find_perch(world: &World) -> Option<(i32, i32)> {
    find_spot(world, 1.0).map(|s| (s[0].0 as i32, s[0].1 as i32))
}

/// Leader on the top, one behind it, the cub in front: (offset forward, size, delay).
const OFFSETS: [(f32, f32, f32); 3] = [(0.0, 1.0, 0.0), (-34.0, 0.92, 0.35), (28.0, 0.62, 0.0)];

/// Wolves keep away from fire.
fn near_fire(world: &World, x: i32, y: i32) -> bool {
    (x - 50..=x + 50).step_by(2).any(|fx| (y - 40..=y + 4).step_by(2).any(|fy| matches!(world.at(fx, fy), Mat::Fire | Mat::Ember | Mat::Lava)))
}

/// 0..1 rise, hold, fall over a window.
fn bump(t: f32, from: f32, to: f32, ramp: f32) -> f32 {
    ((t - from) / ramp).clamp(0.0, 1.0).min(((to - t) / ramp).clamp(0.0, 1.0))
}

impl Wolves {
    pub fn new() -> Self {
        Wolves::default()
    }

    /// The pack comes up to the highest ground, facing the moon (or the middle of the sky).
    /// Returns whether it did.
    pub fn summon(&mut self, world: &World) -> bool {
        if self.pack.is_some() {
            return false;
        }
        let moon_x = world.moon_position().map_or(W as f32 * 0.5, |p| p.0);
        let Some((px, _)) = find_perch(world).or_else(|| find_spot(world, -1.0).map(|s| (s[0].0 as i32, 0))) else { return false };
        let facing = if moon_x >= px as f32 { 1.0 } else { -1.0 };
        let Some((dir, spot)) = [facing, -facing].into_iter().find_map(|d| find_spot(world, d).map(|s| (d, s))) else { return false };
        let wolves = spot.iter().zip(OFFSETS).map(|(&(x, y), (_, size, delay))| Wolf { x, y, dir, size, delay }).collect();
        self.night = world.day_number();
        self.pack = Some(Pack { wolves, age: 0.0 });
        true
    }

    /// `dt` in seconds. The pack comes on its own on a full-moon night, once the moon is well up.
    pub fn update(&mut self, dt: f32, world: &World) -> Option<News> {
        let Some(p) = &mut self.pack else {
            let high = world.moon_arc().map_or(false, |u| (0.2..0.8).contains(&u));
            if world.full_moon() && world.sun() <= 0.0 && high && self.night != world.day_number() {
                self.summon(world);
            }
            return None;
        };
        // They stand on the ground as it changes; if it goes (dug away, burning), they leave.
        for w in &mut p.wolves {
            match ground_at(world, w.x as i32) {
                Some(y) if (y as f32 - w.y).abs() < 16.0 => w.y += (y as f32 - w.y) * (dt * 4.0).min(1.0),
                _ => p.age = p.age.max(GONE - COME),
            }
        }
        let (was, now) = (p.age, p.age + dt);
        p.age = now;
        let crossed = |t: f32| was < t && now >= t;
        if now >= GONE {
            self.pack = None;
            return None;
        }
        if HOWLS.iter().any(|&t| crossed(t)) {
            Some(News::Howl)
        } else if crossed(CUB) {
            Some(News::Cub)
        } else if crossed(WINK.0) {
            Some(News::Wink)
        } else if crossed(FACT) {
            Some(News::Fact)
        } else {
            None
        }
    }

    /// 0..1: how much the moon is winking right now.
    pub fn wink(&self) -> f32 {
        self.pack.as_ref().map_or(0.0, |p| bump(p.age, WINK.0, WINK.1, 0.3))
    }

    pub fn draw(&self, view: View, ui: &Ui) {
        let Some(p) = &self.pack else { return };
        let fade = bump(p.age, 0.0, GONE, COME);
        for w in &p.wolves {
            let t = p.age - w.delay;
            let howl = if w.cub() {
                bump(t, CUB, CUB + 1.6, 0.25)
            } else {
                HOWLS.iter().map(|&h| bump(t, h, h + HOWL, 0.4)).fold(0.0, f32::max)
            };
            let snout = draw_wolf(w, view, fade, howl);
            // The song, rising and fading from the raised muzzle (the leader's and the cub's).
            if w.delay > 0.0 {
                continue;
            }
            let (text, size, start, len) = if w.cub() {
                ("ouh ?", 13.0, CUB, 1.6)
            } else {
                ("Aouuuuh !", 17.0, HOWLS.iter().copied().filter(|&h| h <= t).last().unwrap_or(-9.0), HOWL)
            };
            let k = (t - start) / len;
            if (0.0..1.0).contains(&k) {
                let at = snout + vec2(w.dir * 10.0 * view.s * k, -(8.0 + 26.0 * k) * view.s.max(0.6));
                let a = fade * (1.0 - k).min(k * 6.0).min(1.0);
                let size = (size * view.s.max(0.7)).round() as u16;
                let tw = ui.text_w(text, size);
                ui.text(text, at.x - tw * 0.5, at.y, size, Color::new(0.88, 0.92, 1.0, a));
            }
        }
    }
}

/// A sitting wolf in silhouette, its outline lit by the moon. `howl` 0..1 raises its head.
/// Returns where the tip of its muzzle is on the screen.
fn draw_wolf(w: &Wolf, view: View, fade: f32, howl: f32) -> Vec2 {
    let u = w.size * SCALE;
    let pt = |lx: f32, ly: f32| view.to_screen(vec2(w.x + w.dir * lx * u, w.y + ly * u));
    // Head pose: muzzle forward and a bit down at rest, pointing up at the sky to howl.
    let ang = 0.3 + (-1.15 - 0.3) * howl;
    let head = (3.0 - 0.4 * howl, -8.8 - 0.8 * howl);
    let rot = |dx: f32, dy: f32, a: f32| (head.0 + dx * a.cos() - dy * a.sin(), head.1 + dx * a.sin() + dy * a.cos());
    let tip = rot(3.6, 0.0, ang);
    let mut shapes: Vec<[(f32, f32); 3]> = vec![
        // Bushy tail lying behind it.
        [(-3.0, -1.6), (-3.6, -0.2), (-8.0, -0.3)],
        // Back and chest.
        [(-4.2, -0.2), (1.0, -0.2), (2.8, -7.8)],
        [(-4.2, -0.2), (2.8, -7.8), (-0.8, -6.6)],
        // Front legs.
        [(1.0, 0.0), (2.6, 0.0), (2.6, -6.0)],
        [(1.0, 0.0), (2.6, -6.0), (1.2, -6.0)],
        // Neck.
        [(0.6, -6.6), (3.2, -7.4), head],
    ];
    // Muzzle and ears turn with the head.
    shapes.push([rot(0.0, -1.0, ang), rot(0.0, 1.0, ang), tip]);
    let ear_a = ang * 0.5 + 0.15;
    shapes.push([rot(-1.1, -0.9, ear_a), rot(0.2, -1.3, ear_a), rot(-0.9, -3.2, ear_a)]);
    let to = |p: (f32, f32), off: (f32, f32)| {
        let s = pt(p.0, p.1);
        s + vec2(off.0, off.1)
    };
    // The moonlit rim first, a little towards the moon, then the dark body over it.
    let rim = Color::new(0.55, 0.62, 0.85, 0.8 * fade);
    let body = Color::new(0.07, 0.08, 0.14, fade);
    let o = (w.dir * 0.3 * u * view.s.max(0.8), -0.3 * u * view.s.max(0.8));
    for (col, off) in [(rim, o), (body, (0.0, 0.0))] {
        for s in &shapes {
            draw_triangle(to(s[0], off), to(s[1], off), to(s[2], off), col);
        }
        let hp = to(head, off);
        draw_circle(hp.x, hp.y, 1.8 * u * view.s, col);
        let hip = to((-2.0, -2.6), off);
        draw_circle(hip.x, hip.y, 2.6 * u * view.s, col);
    }
    let eye = pt(rot(0.6, -0.5, ang).0, rot(0.6, -0.5, ang).1);
    draw_circle(eye.x, eye.y, (0.35 * u * view.s).max(0.8), Color::new(0.95, 0.9, 0.5, fade * (1.0 - howl)));
    pt(tip.0, tip.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hill() -> World {
        let mut w = World::new_static();
        for x in 0..W as i32 {
            // A hill peaking at x = 600.
            let top = 400 - (60 - (x - 600).abs() / 4).max(0);
            for y in top..H as i32 {
                w.set(x, y, Mat::Stone);
            }
        }
        w
    }

    #[test]
    fn the_pack_sits_on_the_highest_ground_and_faces_the_moon() {
        let mut w = hill();
        let (x, y) = find_perch(&w).unwrap();
        assert!((x - 600).abs() <= 4 && y == 340, "the top of the hill: {x} {y}");
        w.set_day(3);
        w.set_phase(0.7);
        let mut wolves = Wolves::new();
        assert!(wolves.summon(&w));
        let p = wolves.pack.as_ref().unwrap();
        assert_eq!(p.wolves.len(), 3);
        let moon = w.moon_position().unwrap().0;
        for wolf in &p.wolves {
            assert_eq!(wolf.dir, (moon - wolf.x).signum());
            assert!(w.at(wolf.x as i32, wolf.y as i32) == Mat::Stone && w.at(wolf.x as i32, wolf.y as i32 - 1) == Mat::Empty, "standing on the ground");
        }
        assert!(!wolves.summon(&w), "one pack at a time");
    }

    #[test]
    fn they_come_on_full_moon_nights_only_howl_and_leave() {
        let mut w = hill();
        let mut wolves = Wolves::new();
        // A night of last quarter: nobody.
        w.set_day(5);
        w.set_phase(0.8);
        wolves.update(0.1, &w);
        assert!(wolves.pack.is_none());
        // Full moon, high in the sky: here they are.
        w.set_day(3);
        w.set_phase(0.75);
        assert!(wolves.update(0.1, &w).is_none() && wolves.pack.is_some());
        let mut news = vec![];
        let mut winked = false;
        for _ in 0..300 {
            if let Some(n) = wolves.update(0.1, &w) {
                news.push(n);
            }
            winked |= wolves.wink() > 0.9;
        }
        assert_eq!(news, [News::Howl, News::Howl, News::Cub, News::Howl, News::Wink, News::Fact]);
        assert!(winked, "the moon winks");
        assert!(wolves.pack.is_none() && wolves.wink() == 0.0, "gone again");
        // Not twice the same night.
        wolves.update(0.1, &w);
        assert!(wolves.pack.is_none());
    }
}
